//! Go's `http.ServeContent`, for a body that is all there (a cached render, a static file, a document): `If-Match`,
//! `If-None-Match` (304), `Range` / `If-Range` (206, multipart for several ranges, 416), `HEAD`, with the headers Go
//! writes in the order the site's header layer then sorts. No `Last-Modified`: every caller passes Go's zero modtime.
//!
//! `serve` takes the handler's own headers (ETag, Cache-Control, Content-Type, ...) and the bytes. The bytes are a
//! `Bytes`, so a memory map of a file (a cache hit) goes out without a copy.

use std::collections::HashMap;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use bytes::Bytes;

/// What the request asked for, as `serve` needs it.
pub struct Req<'a> {
    pub method: &'a Method,
    pub headers: &'a HeaderMap,
}

fn reply(h: HeaderMap, status: StatusCode, body: Body) -> Response<Body> {
    let mut r = Response::new(body);
    *r.status_mut() = status;
    *r.headers_mut() = h;
    r
}

/// An empty body hyper does not know is empty: it then keeps our `Content-Length: 0` where the sorted headers put it
/// (for a body it can see is empty it drops the header and appends its own, last).
pub fn nothing() -> Body {
    Body::from_stream(futures_util::stream::empty::<Result<Bytes, std::io::Error>>())
}

/// `http.ServeContent(w, r, name, time.Time{}, content)`. `h` must carry `ETag` (Go's callers all set one) and
/// `Content-Type`; `data` is `None` for a HEAD whose body was not loaded (only its `size` is needed).
pub fn serve(mut h: HeaderMap, req: &Req, size: u64, data: Option<Bytes>) -> Response<Body> {
    let etag = h.get(header::ETAG).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let hdr = |n: header::HeaderName| req.headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    // checkPreconditions
    if let Some(im) = hdr(header::IF_MATCH) {
        if !etag_list_match(&im, &etag, true) {
            // Go's 412 has no Content-Length in the handler's map: the server appends `Content-Length: 0` last, as
            // hyper does for an empty body
            return reply(h, StatusCode::PRECONDITION_FAILED, Body::empty());
        }
    }
    if let Some(inm) = hdr(header::IF_NONE_MATCH) {
        if etag_list_match(&inm, &etag, false) {
            if req.method == Method::GET || req.method == Method::HEAD {
                // writeNotModified: Content-Type, Content-Length (and Last-Modified) go
                h.remove(header::CONTENT_TYPE);
                h.remove(header::CONTENT_LENGTH);
                return reply(h, StatusCode::NOT_MODIFIED, Body::empty());
            }
            return reply(h, StatusCode::PRECONDITION_FAILED, Body::empty());
        }
    }
    let mut range = hdr(header::RANGE).unwrap_or_default();
    if !range.is_empty() {
        if let Some(ir) = hdr(header::IF_RANGE) {
            // a date never matches (no modtime); an entity tag must match strongly
            if !(ir.trim().starts_with('"') && etag_list_match(ir.trim(), &etag, true)) {
                range.clear();
            }
        }
    }
    let ranges = match parse_ranges(&range, size) {
        Ok(r) => r,
        Err(no_overlap) if no_overlap && size == 0 => Vec::new(),
        Err(no_overlap) => {
            if no_overlap {
                h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{size}")).unwrap());
            }
            // http.Error, after ServeContent's serveError drops the cache validators
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            for n in [header::CACHE_CONTROL, header::ETAG] {
                h.remove(n);
            }
            let msg = if no_overlap { "invalid range: failed to overlap\n" } else { "invalid range\n" };
            let mut r = reply(h, StatusCode::RANGE_NOT_SATISFIABLE, Body::from(msg));
            r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Requested Range Not Satisfiable"));
            return r;
        }
    };
    let ranges = if ranges.iter().map(|r| r.1).sum::<u64>() > size { Vec::new() } else { ranges };
    let head = req.method == Method::HEAD;
    let slice = |a: u64, n: u64| data.as_ref().map_or_else(Bytes::new, |m| m.slice(a as usize..(a + n) as usize));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let (status, len, body) = match ranges.len() {
        0 => (StatusCode::OK, size, slice(0, size).into()),
        1 => {
            let (a, n) = ranges[0];
            h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {}-{}/{size}", a, a + n - 1)).unwrap());
            (StatusCode::PARTIAL_CONTENT, n, slice(a, n).into())
        }
        _ => {
            // mime/multipart as Go writes it: a 60-hex-digit random boundary, parts' headers sorted
            let boundary = random_boundary();
            let ctype = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let mut parts: Vec<Bytes> = Vec::new();
            for (i, (a, n)) in ranges.iter().enumerate() {
                let lead = if i == 0 { "" } else { "\r\n" };
                parts.push(Bytes::from(format!("{lead}--{boundary}\r\nContent-Range: bytes {}-{}/{size}\r\nContent-Type: {ctype}\r\n\r\n", a, a + n - 1)));
                parts.push(slice(*a, *n));
            }
            parts.push(Bytes::from(format!("\r\n--{boundary}--\r\n")));
            // the framing (pieces 0, 2, .. and the closing one) plus the ranges themselves (whether or not they were mapped: HEAD)
            let total = parts.iter().step_by(2).map(|p| p.len() as u64).sum::<u64>() + ranges.iter().map(|r| r.1).sum::<u64>();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&format!("multipart/byteranges; boundary={boundary}")).unwrap());
            (StatusCode::PARTIAL_CONTENT, total, Body::from_stream(futures_util::stream::iter(parts.into_iter().map(Ok::<Bytes, std::io::Error>))))
        }
    };
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    reply(h, status, if head { Body::empty() } else if len == 0 { nothing() } else { body })
}

/// Go's `parseRange`: `Ok(ranges)` as (start, length), `Err(true)` when no range overlaps, `Err(false)` when the
/// header is not a byte range at all.
pub fn parse_ranges(s: &str, size: u64) -> Result<Vec<(u64, u64)>, bool> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let spec = s.strip_prefix("bytes=").ok_or(false)?;
    let trim = |x: &str| x.trim_matches(|c| c == ' ' || c == '\t').to_string();
    let int = |x: &str| -> Result<u64, bool> {
        if x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit() || b == b'+') {
            return Err(false);
        }
        x.parse::<i64>().ok().filter(|v| *v >= 0).map(|v| v as u64).ok_or(false)
    };
    let (mut out, mut no_overlap) = (Vec::new(), false);
    for ra in spec.split(',') {
        let ra = trim(ra);
        if ra.is_empty() {
            continue;
        }
        let (a, b) = ra.split_once('-').ok_or(false)?;
        let (a, b) = (trim(a), trim(b));
        if a.is_empty() {
            if b.is_empty() || b.starts_with('-') {
                return Err(false);
            }
            let n = int(&b)?.min(size);
            out.push((size - n, n));
        } else {
            let a = int(&a)?;
            if a >= size {
                no_overlap = true;
                continue;
            }
            let n = if b.is_empty() {
                size - a
            } else {
                let b = int(&b)?;
                if a > b {
                    return Err(false);
                }
                b.min(size - 1) - a + 1
            };
            out.push((a, n));
        }
    }
    if no_overlap && out.is_empty() {
        return Err(true);
    }
    Ok(out)
}

/// An If-Match / If-None-Match list against our (strong) tag: `*`, or one of the listed tags (weakly: `W/` ignored;
/// strongly: a `W/` tag never matches).
pub fn etag_list_match(list: &str, etag: &str, strong: bool) -> bool {
    list.split(',').map(str::trim).filter(|t| !t.is_empty()).any(|t| {
        if t == "*" {
            return true;
        }
        match t.strip_prefix("W/") {
            Some(w) => !strong && w == etag,
            None => t == etag,
        }
    })
}

fn random_boundary() -> String {
    let mut b = [0u8; 30];
    if let Ok(f) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut &f, &mut b);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `http.Error(w, msg, code)`: text/plain, nosniff, the message and a newline (Go 1.23 also drops Content-Length,
/// which nothing here set).
pub fn text_error(status: u16, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(format!("{msg}\n")))
        .unwrap()
}

/// The hex of the first `n` bytes of sha256(b).
pub fn sha_hex(b: &[u8], n: usize) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(b);
    d[..n].iter().map(|x| format!("{x:02x}")).collect()
}

/// `mime.TypeByExtension` as the Go server sees it: Go's builtin table, then the host's MIME database the way Go's
/// `osInitMime` reads it (the first shared-mime-info `globs2` that exists, where an extension already known keeps
/// its type; else the `mime.types` files, which override), then the Go server's own `mime.AddExtensionType` calls
/// (server/handlers.go `init`). `text/*` without a charset gets `; charset=utf-8` (Go's `setExtensionType`).
/// Host-dependent on purpose: on the same host beatscript-serve answers what Go answers. Unknown: None (ServeContent then
/// sniffs: `sniff`).
pub fn type_by_ext(ext: &str) -> Option<&'static str> {
    static TABLE: std::sync::OnceLock<HashMap<String, &'static str>> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(mime_table);
    t.get(&ext.to_ascii_lowercase()).copied()
}

fn mime_table() -> HashMap<String, &'static str> {
    const BUILTIN: &[(&str, &str)] = &[
        ("ai", "application/postscript"), ("apk", "application/vnd.android.package-archive"), ("apng", "image/apng"), ("avif", "image/avif"),
        ("bin", "application/octet-stream"), ("bmp", "image/bmp"), ("com", "application/octet-stream"), ("css", "text/css; charset=utf-8"),
        ("csv", "text/csv; charset=utf-8"), ("doc", "application/msword"), ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        ("ehtml", "text/html; charset=utf-8"), ("eml", "message/rfc822"), ("eps", "application/postscript"), ("exe", "application/octet-stream"),
        ("flac", "audio/flac"), ("gif", "image/gif"), ("gz", "application/gzip"), ("htm", "text/html; charset=utf-8"), ("html", "text/html; charset=utf-8"),
        ("ico", "image/vnd.microsoft.icon"), ("ics", "text/calendar; charset=utf-8"), ("jfif", "image/jpeg"), ("jpeg", "image/jpeg"), ("jpg", "image/jpeg"),
        ("js", "text/javascript; charset=utf-8"), ("json", "application/json"), ("m4a", "audio/mp4"), ("mjs", "text/javascript; charset=utf-8"),
        ("mp3", "audio/mpeg"), ("mp4", "video/mp4"), ("oga", "audio/ogg"), ("ogg", "audio/ogg"), ("ogv", "video/ogg"), ("opus", "audio/ogg"),
        ("pdf", "application/pdf"), ("pjp", "image/jpeg"), ("pjpeg", "image/jpeg"), ("png", "image/png"), ("ppt", "application/vnd.ms-powerpoint"),
        ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"), ("ps", "application/postscript"), ("rdf", "application/rdf+xml"),
        ("rtf", "application/rtf"), ("shtml", "text/html; charset=utf-8"), ("svg", "image/svg+xml"), ("text", "text/plain; charset=utf-8"),
        ("tif", "image/tiff"), ("tiff", "image/tiff"), ("txt", "text/plain; charset=utf-8"), ("vtt", "text/vtt; charset=utf-8"), ("wasm", "application/wasm"),
        ("wav", "audio/wav"), ("webm", "audio/webm"), ("webp", "image/webp"), ("xbl", "text/xml; charset=utf-8"), ("xbm", "image/x-xbitmap"),
        ("xht", "application/xhtml+xml"), ("xhtml", "application/xhtml+xml"), ("xls", "application/vnd.ms-excel"),
        ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), ("xml", "text/xml; charset=utf-8"), ("xsl", "text/xml; charset=utf-8"),
        ("zip", "application/zip"),
    ];
    const SERVER: &[(&str, &str)] = &[
        ("js", "text/javascript; charset=utf-8"), ("mjs", "text/javascript; charset=utf-8"), ("wasm", "application/wasm"), ("css", "text/css; charset=utf-8"),
        ("json", "application/json; charset=utf-8"), ("svg", "image/svg+xml"), ("btw", "text/plain; charset=utf-8"), ("wav", "audio/wav"),
        ("md", "text/markdown; charset=utf-8"), ("txt", "text/plain; charset=utf-8"), ("html", "text/html; charset=utf-8"), ("webmanifest", "application/manifest+json"),
    ];
    fn with_charset(t: &str) -> &'static str {
        // Go: ParseMediaType + FormatMediaType; the database's types are bare, so only the charset is added
        let t = t.trim();
        let s = if t.starts_with("text/") && !t.contains("charset=") { format!("{t}; charset=utf-8") } else { t.to_string() };
        Box::leak(s.into_boxed_str())
    }
    let mut m: HashMap<String, &'static str> = BUILTIN.iter().map(|(e, t)| (e.to_string(), *t)).collect();
    let globs = ["/usr/local/share/mime/globs2", "/usr/share/mime/globs2"].iter().find_map(|p| std::fs::read_to_string(p).ok());
    if let Some(g) = globs {
        for line in g.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 3 || f[0].is_empty() || f[2].len() < 3 || f[0].starts_with('#') || !f[2].starts_with("*.") {
                continue;
            }
            let ext = &f[2][2..];
            if ext.contains(['?', '*', '[']) || m.contains_key(&ext.to_ascii_lowercase()) {
                continue;
            }
            m.insert(ext.to_ascii_lowercase(), with_charset(f[1]));
        }
    } else {
        for p in ["/etc/mime.types", "/etc/apache2/mime.types", "/etc/apache/mime.types", "/etc/httpd/conf/mime.types"] {
            let Ok(txt) = std::fs::read_to_string(p) else { continue };
            for line in txt.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() <= 1 || f[0].starts_with('#') {
                    continue;
                }
                for e in &f[1..] {
                    if e.starts_with('#') {
                        break;
                    }
                    m.insert(e.to_ascii_lowercase(), with_charset(f[0]));
                }
            }
        }
    }
    for (e, t) in SERVER {
        m.insert(e.to_string(), t);
    }
    m
}

/// The few answers of `http.DetectContentType` a static site can produce (it looks at the first 512 bytes).
pub fn sniff(b: &[u8]) -> &'static str {
    let b = &b[..b.len().min(512)];
    let lead = {
        let i = b.iter().position(|c| !matches!(c, b'\t' | b'\n' | 0x0c | b'\r' | b' ')).unwrap_or(b.len());
        &b[i..]
    };
    let lower: Vec<u8> = lead.iter().take(14).map(|c| c.to_ascii_lowercase()).collect();
    for tag in [&b"<!doctype html"[..], b"<html", b"<head", b"<script", b"<body", b"<!--", b"<p", b"<div"] {
        if lower.starts_with(tag) && lower.get(tag.len()).is_none_or(|c| *c == b' ' || *c == b'>') {
            return "text/html; charset=utf-8";
        }
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE") {
        return "audio/wave";
    }
    if b.starts_with(b"OggS\x00") {
        return "application/ogg";
    }
    if b.starts_with(b"\x00asm") {
        return "application/wasm";
    }
    let binary = b.iter().any(|c| matches!(c, 0x00..=0x08 | 0x0b | 0x0e..=0x1a | 0x1c..=0x1f));
    if binary { "application/octet-stream" } else { "text/plain; charset=utf-8" }
}
