//! The static site (`-web`, the Go server's `handleStatic`) and the public examples (`GET /api/examples`,
//! `GET /examples/<name>.beatscript`, `docs.LoadExampleSet`).
//!
//! Static, as Go serves it: GET and HEAD only (else 405 JSON with `Allow`), `/` and `/p` are `index.html`, a path
//! with a dotted segment or outside the directory is a 404 JSON, a directory is its `index.html`, `Cache-Control:
//! no-cache` and a content ETag (sha256, 12 bytes, cached by size and mtime), `http.ServeContent`'s Range and
//! If-None-Match, no Last-Modified. `/` and `/p` carry the generic share card (`card.InjectGeneric`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::content;
use crate::{go_json, go_num, ApiError};

// ---------- examples ----------

#[derive(Clone, Debug)]
pub struct Example {
    pub name: String,
    pub title: String,
    pub description: String,
    pub cid: String,
    pub bpm: f64,
    pub bars: i64,
    pub license: String,
}

impl Example {
    /// `api.Example` as encoding/json writes it (field order, `omitempty` on bpm and bars).
    pub fn json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("name".into(), json!(self.name));
        m.insert("title".into(), json!(self.title));
        m.insert("description".into(), json!(self.description));
        m.insert("cid".into(), json!(self.cid));
        if self.bpm != 0.0 {
            m.insert("bpm".into(), go_num(self.bpm));
        }
        if self.bars != 0 {
            m.insert("bars".into(), json!(self.bars));
        }
        m.insert("license".into(), json!(self.license));
        Value::Object(m)
    }
}

#[derive(Default)]
pub struct Examples {
    pub list: Vec<Example>,
    pub text: HashMap<String, String>,
}

fn example_name_ok(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty() && b.len() <= 64 && matches!(b[0], b'a'..=b'z' | b'0'..=b'9') && b.iter().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// `docs.LoadExampleSet` + `server.loadExamples`: index.json order, each program checked; a missing directory is a
/// warning (no examples), anything else wrong is an error (Go refuses to start).
pub fn load_examples(dir: Option<&Path>) -> Result<(Examples, Vec<String>), String> {
    let mut set = Examples::default();
    let Some(dir) = dir else { return Ok((set, Vec::new())) };
    let raw = match std::fs::read(dir.join("index.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((set, vec![format!("no examples: examples: {e}")])),
        Err(e) => return Err(format!("examples: {e}")),
    };
    let idx: Vec<Value> = serde_json::from_slice(&raw).map_err(|e| format!("examples: index.json: {e}"))?;
    for ie in idx {
        let s = |k: &str| ie[k].as_str().unwrap_or("").to_string();
        let name = s("name");
        if !example_name_ok(&name) {
            return Err(format!("examples: index.json: {name:?} is not a usable example name"));
        }
        if set.text.contains_key(&name) {
            return Err(format!("examples: index.json: {name:?} is listed twice"));
        }
        let b = std::fs::read(dir.join(format!("{name}.beatscript"))).map_err(|e| format!("examples: {e}"))?;
        let text = String::from_utf8(b).ok().filter(|t| t.len() <= 64 << 10).ok_or(format!("examples: {name}.beatscript is not UTF-8 text under 64 KiB"))?;
        if crate::store::declares_not_public(&text) {
            return Err(format!("example {name} says it is not public (`#! visibility:`): examples are public programs"));
        }
        let rep = crate::store::check(&text);
        if rep["ok"] != true {
            return Err(format!("examples: {name} does not check"));
        }
        if rep["restricted"] == true || rep["uses_assets"] == true {
            return Err(format!("examples: {name} uses samples or `use restricted`: examples are public programs"));
        }
        let mut ex = Example {
            title: s("title"),
            description: s("description"),
            cid: rep["cid"].as_str().unwrap_or("").to_string(),
            bpm: ie["bpm"].as_f64().unwrap_or(0.0),
            bars: ie["bars"].as_i64().unwrap_or(0),
            license: s("license"),
            name: name.clone(),
        };
        if ex.title.is_empty() {
            ex.title = name.clone();
        }
        if ex.license.is_empty() {
            ex.license = "CC0-1.0".into();
        }
        if let Some(t) = rep["tempo"].as_f64().filter(|t| *t > 0.0) {
            ex.bpm = t; // the program is the truth, the index is a hint
        }
        if let Some(b) = rep["song_bars"].as_i64().filter(|b| *b > 0) {
            ex.bars = b;
        }
        set.list.push(ex);
        set.text.insert(name, text);
    }
    Ok((set, Vec::new()))
}

/// `GET /api/examples`.
pub fn examples_json(ex: &Examples) -> Response<Body> {
    let list: Vec<Value> = ex.list.iter().map(Example::json).collect();
    Response::builder()
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(crate::go_body(go_json(&Value::Array(list)) + "\n"))
        .unwrap()
}

/// `GET /examples/{name}`: `<name>.beatscript`, the file as written.
pub fn example_file(ex: &Examples, raw: &str) -> Response<Body> {
    let name = percent_decode(raw);
    match name.strip_suffix(".beatscript").and_then(|n| ex.text.get(n)) {
        Some(t) => Response::builder()
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(crate::go_body(t.clone()))
            .unwrap(),
        None => ApiError::new(404, "not_found", format!("no example named {}", go_quote(&name))).response(),
    }
}

/// Go's `%q` for the strings a URL path can carry (printable text as is, `"` and `\` escaped, the rest `\x..`/`\u....`).
pub fn go_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => o.push_str(&format!("\\x{:02x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A URL path as Go's `URL.Path` holds it: percent-escapes decoded (an invalid escape is left as it is).
pub fn percent_decode(p: &str) -> String {
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------- the generic share card (card.GenericMetaAt / InjectGeneric) ----------

const GENERIC_TITLE: &str = "BeatScript: write a whole track as text";
const GENERIC_DESC: &str = "AI-assisted algorithmic composition";
const CARD_META_MARKER: &str = "<!-- beatscript:card-meta -->";

/// Go's `html.EscapeString`.
pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '\'' => o.push_str("&#39;"),
            '"' => o.push_str("&#34;"),
            c => o.push(c),
        }
    }
    o
}

pub fn generic_meta_at(origin: &str, path: &str) -> String {
    let origin = origin.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let e = html_escape;
    let img = format!("{origin}/card/default.png");
    let url = format!("{origin}{path}");
    let mut b = String::new();
    let mut w = |s: String| {
        b.push_str(&s);
        b.push('\n');
    };
    w(format!(r#"<meta name="description" content="{}">"#, e(GENERIC_DESC)));
    w(format!(r#"<link rel="canonical" href="{}">"#, e(&url)));
    w(r#"<meta property="og:type" content="website">"#.into());
    w(r#"<meta property="og:site_name" content="BeatScript">"#.into());
    w(format!(r#"<meta property="og:title" content="{}">"#, e(GENERIC_TITLE)));
    w(format!(r#"<meta property="og:description" content="{}">"#, e(GENERIC_DESC)));
    w(format!(r#"<meta property="og:url" content="{}">"#, e(&url)));
    w(format!(r#"<meta property="og:image" content="{}">"#, e(&img)));
    w(format!(r#"<meta property="og:image:secure_url" content="{}">"#, e(&img)));
    w(r#"<meta property="og:image:type" content="image/png">"#.into());
    w(r#"<meta property="og:image:width" content="1200">"#.into());
    w(r#"<meta property="og:image:height" content="630">"#.into());
    w(format!(r#"<meta property="og:image:alt" content="{}">"#, e("BeatScript: a few lines of code on a dark stage, over a strip of song sections")));
    w(r#"<meta name="twitter:card" content="summary_large_image">"#.into());
    w(format!(r#"<meta name="twitter:title" content="{}">"#, e(GENERIC_TITLE)));
    w(format!(r#"<meta name="twitter:description" content="{}">"#, e(GENERIC_DESC)));
    w(format!(r#"<meta name="twitter:image" content="{}">"#, e(&img)));
    w(format!(r#"<script type="application/ld+json">{}</script>"#, site_ld(origin)));
    b
}

/// The site as linked data (schema.org WebSite and SoftwareApplication): free to use, the code MIT at
/// [`beatscript_serve::rights::SOURCE_URL`], the programs CC0 (each program's own JSON-LD says so). A data block, never run.
pub fn site_ld(origin: &str) -> String {
    let home = format!("{origin}/");
    let app = serde_json::json!({
        "@type": "SoftwareApplication", "@id": format!("{origin}/#app"), "name": "BeatScript", "url": home,
        "applicationCategory": "MultimediaApplication", "operatingSystem": "Web, Android",
        "isAccessibleForFree": true, "codeRepository": beatscript_serve::rights::SOURCE_URL, "license": "https://opensource.org/licenses/MIT",
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
    });
    let site = serde_json::json!({
        "@type": "WebSite", "@id": format!("{origin}/#site"), "name": "BeatScript", "url": home, "description": GENERIC_DESC,
        "isAccessibleForFree": true, "codeRepository": beatscript_serve::rights::SOURCE_URL, "license": "https://opensource.org/licenses/MIT",
        "about": {"@id": format!("{origin}/#app")}, "usageInfo": beatscript_serve::rights::url(origin),
    });
    // (no field holds markup; a `</` would still be escaped, so nothing can close the script early)
    serde_json::json!({"@context": "https://schema.org", "@graph": [site, app]}).to_string().replace("</", "<\\/")
}

pub fn inject_generic(doc: &[u8], origin: &str, path: &str) -> Vec<u8> {
    let frag = generic_meta_at(origin, path);
    let find = |needle: &[u8]| doc.windows(needle.len()).position(|w| w == needle);
    if let Some(i) = find(CARD_META_MARKER.as_bytes()) {
        return [&doc[..i], frag.as_bytes(), &doc[i + CARD_META_MARKER.len()..]].concat();
    }
    if find(br#"property="og:image""#).is_some() {
        return doc.to_vec();
    }
    if let Some(i) = find(b"</head>") {
        return [&doc[..i], frag.as_bytes(), &doc[i..]].concat();
    }
    doc.to_vec()
}

// ---------- static ----------

pub struct Static {
    root: Option<PathBuf>,
    public_url: String,
    /// The card meta goes into `/` and `/p` (Go does it once the card service is wired, which the deployed server is).
    inject: bool,
    etags: Mutex<HashMap<PathBuf, (u64, SystemTime, String)>>,
}

/// `path.Clean`.
pub fn clean(p: &str) -> String {
    let rooted = p.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let j = parts.join("/");
    if rooted {
        format!("/{j}")
    } else if j.is_empty() {
        ".".into()
    } else {
        j
    }
}

pub fn method_not_allowed(m: &Method) -> Response<Body> {
    let mut r = ApiError::new(405, "method_not_allowed", format!("{m} is not allowed here")).response();
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    r
}

pub fn not_found() -> Response<Body> {
    ApiError::new(404, "not_found", "not found").response()
}

impl Static {
    pub fn new(root: Option<PathBuf>, public_url: &str, inject: bool) -> Static {
        let root = root.map(|r| std::fs::canonicalize(&r).unwrap_or(r));
        Static { root, public_url: public_url.to_string(), inject, etags: Mutex::new(HashMap::new()) }
    }

    fn etag(&self, full: &Path, size: u64, mtime: SystemTime, data: &[u8]) -> String {
        if let Some((s, m, t)) = self.etags.lock().unwrap().get(full) {
            if *s == size && *m == mtime {
                return t.clone();
            }
        }
        let t = format!("\"{}\"", content::sha_hex(data, 12));
        self.etags.lock().unwrap().insert(full.to_path_buf(), (size, mtime, t.clone()));
        t
    }

    /// `handleStatic`. `raw_path` is the request's path as sent (percent-escaped).
    pub fn serve(&self, method: &Method, raw_path: &str, headers: &HeaderMap) -> Response<Body> {
        self.serve_with(method, raw_path, headers, None)
    }

    /// [`serve`](Static::serve), with `view` (a program's `<beatscript-view>` block, djlive `progld`) put at the end of
    /// the body of `/` and `/p`: the editor loads it. The head, and its card meta, are as they always are.
    pub fn serve_with(&self, method: &Method, raw_path: &str, headers: &HeaderMap, view: Option<&str>) -> Response<Body> {
        if method != Method::GET && method != Method::HEAD {
            return method_not_allowed(method);
        }
        let url_path = percent_decode(raw_path);
        let mut p = clean(&format!("/{url_path}"));
        if p == "/" || p == "/p" {
            p = "/index.html".into();
        }
        if p.split('/').any(|s| s.starts_with('.')) {
            return not_found();
        }
        let Some(root) = &self.root else { return not_found() };
        let mut full = root.join(p.trim_start_matches('/'));
        let mut md = std::fs::metadata(&full);
        if md.as_ref().is_ok_and(|m| m.is_dir()) {
            full = full.join("index.html");
            md = std::fs::metadata(&full);
        }
        let Ok(md) = md else { return not_found() };
        // the directory check is on the joined name, as Go's (a symlink inside -web may point anywhere, as there)
        if md.is_dir() || !full.starts_with(root) || full == *root {
            return not_found();
        }
        let Ok(data) = std::fs::read(&full) else { return not_found() };
        let data = Bytes::from(data);
        let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let tag = self.etag(&full, md.len(), mtime, &data);
        let mut h = HeaderMap::new();
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("");
        let ct = content::type_by_ext(ext);
        let req = content::Req { method, headers };
        if self.inject && p == "/index.html" && url_path != "/index.html" {
            let path = if url_path.trim_end_matches('/') == "/p" { "/p" } else { "/" };
            let mut out = inject_generic(&data, &self.public_url, path);
            if let Some(v) = view {
                let at = out.windows(7).rposition(|w| w == b"</body>").unwrap_or(out.len());
                out.splice(at..at, format!("{v}\n").into_bytes());
            }
            let out = Bytes::from(out);
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct.unwrap_or("text/html; charset=utf-8")));
            h.insert(header::ETAG, HeaderValue::from_str(&format!("\"{}\"", content::sha_hex(&out, 12))).unwrap());
            let n = out.len() as u64;
            return content::serve(h, &req, n, Some(out));
        }
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct.unwrap_or_else(|| content::sniff(&data))));
        let ae = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("");
        match precompressed(&full, mtime, ae) {
            Sidecar::Use(enc, sfull, smd, sdata) => {
                // the whole encoded file, always: a byte range of compressed bytes is no use to a browser
                h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
                h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
                let smtime = smd.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let stag = self.etag(&sfull, smd.len(), smtime, &sdata);
                h.insert(header::ETAG, HeaderValue::from_str(&stag).unwrap());
                let mut rh = headers.clone();
                rh.remove(header::RANGE);
                let sreq = content::Req { method, headers: &rh };
                let n = sdata.len() as u64;
                return content::serve(h, &sreq, n, Some(Bytes::from(sdata)));
            }
            Sidecar::Refused => {
                h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
            }
            Sidecar::None => {}
        }
        h.insert(header::ETAG, HeaderValue::from_str(&tag).unwrap());
        let n = data.len() as u64;
        content::serve(h, &req, n, Some(data))
    }
}

enum Sidecar {
    /// encoding, sidecar path, its metadata, its bytes
    Use(&'static str, PathBuf, std::fs::Metadata, Vec<u8>),
    /// a current sidecar exists but the client takes none of them (the response still varies)
    Refused,
    None,
}

/// Does `Accept-Encoding` take `enc`? Listed by name with no `q=0` (Go's `precompressed`: no `*`, no q ordering).
fn accepts(accept_encoding: &str, enc: &str) -> bool {
    for part in accept_encoding.split(',') {
        let (name, params) = part.trim().split_once(';').unwrap_or((part.trim(), ""));
        if name.trim().eq_ignore_ascii_case(enc) {
            let q: String = params.trim().chars().filter(|c| *c != ' ').collect();
            return !matches!(q.as_str(), "q=0" | "q=0.0" | "q=0.00" | "q=0.000");
        }
    }
    false
}

/// Go's `precompressed`: for a `.wasm`, `<file>.br` then `<file>.gz`, no older than the file, that the client
/// accepts. Only .wasm is precompressed (deploy.sh artifacts writes the sidecars); nginx gzips the rest.
fn precompressed(full: &Path, mtime: SystemTime, accept_encoding: &str) -> Sidecar {
    if full.extension().and_then(|e| e.to_str()) != Some("wasm") {
        return Sidecar::None;
    }
    let mut seen = false;
    for (enc, ext) in [("br", "br"), ("gzip", "gz")] {
        let mut s = full.as_os_str().to_owned();
        s.push(".");
        s.push(ext);
        let sfull = PathBuf::from(s);
        let Ok(smd) = std::fs::metadata(&sfull) else { continue };
        if smd.is_dir() || smd.modified().unwrap_or(SystemTime::UNIX_EPOCH) < mtime {
            continue;
        }
        seen = true;
        if !accepts(accept_encoding, enc) {
            continue;
        }
        let Ok(sdata) = std::fs::read(&sfull) else { continue };
        return Sidecar::Use(enc, sfull, smd, sdata);
    }
    if seen { Sidecar::Refused } else { Sidecar::None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_is_go_path_clean() {
        for (a, b) in [("/", "/"), ("//a//b/", "/a/b"), ("/a/../../b", "/b"), ("/a/./b/.", "/a/b"), ("/..", "/"), ("/p/", "/p")] {
            assert_eq!(clean(a), b, "{a}");
        }
    }

    /// Go's TestStaticPrecompressedWasm.
    #[test]
    fn static_precompressed_wasm() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("wasm")).unwrap();
        std::fs::create_dir_all(dir.path().join("js")).unwrap();
        let plain = dir.path().join("wasm/beatscript_bg.wasm");
        let raw = b"\0asm-PLAIN-WASM-BYTES".to_vec();
        std::fs::write(&plain, &raw).unwrap();
        std::fs::write(dir.path().join("wasm/beatscript_bg.wasm.br"), b"BR-BYTES").unwrap();
        std::fs::write(dir.path().join("wasm/beatscript_bg.wasm.gz"), b"GZ-BYTES").unwrap();
        std::fs::write(dir.path().join("js/app.js"), b"console.log(1)").unwrap();
        let st = Static::new(Some(dir.path().to_path_buf()), "https://x.test", false);
        let get = |path: &str, hs: &[(&str, &str)]| {
            let mut h = HeaderMap::new();
            for (k, v) in hs {
                h.insert(header::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
            }
            let r = st.serve(&Method::GET, path, &h);
            let status = r.status().as_u16();
            let hd = r.headers().clone();
            let body = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(axum::body::to_bytes(r.into_body(), usize::MAX)).unwrap();
            (status, hd, body.to_vec())
        };
        let g = |h: &HeaderMap, n: header::HeaderName| h.get(n).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        for (ae, enc, body) in [
            ("br, gzip", "br", b"BR-BYTES".to_vec()),
            ("gzip, deflate, br;q=0", "gzip", b"GZ-BYTES".to_vec()),
            ("gzip;q=0.5", "gzip", b"GZ-BYTES".to_vec()),
            ("identity", "", raw.clone()),
        ] {
            let (s, h, b) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", ae)]);
            assert_eq!(s, 200, "{ae}");
            assert_eq!(g(&h, header::CONTENT_ENCODING), enc, "{ae}");
            assert_eq!(b, body, "{ae}");
            assert_eq!(g(&h, header::CONTENT_TYPE), "application/wasm", "{ae}");
            assert_eq!(g(&h, header::VARY), "Accept-Encoding", "{ae}");
        }
        let (_, hb, _) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", "br")]);
        let (_, hp, _) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", "identity")]);
        let etag = g(&hb, header::ETAG);
        assert!(!etag.is_empty() && etag != g(&hp, header::ETAG));
        assert_eq!(etag, format!("\"{}\"", content::sha_hex(b"BR-BYTES", 12)));
        assert_eq!(g(&hb, header::CONTENT_LENGTH), "8");
        let (s, _, b) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", "br"), ("range", "bytes=0-1")]);
        assert_eq!((s, b), (200, b"BR-BYTES".to_vec()), "range of the encoded file");
        let (s, _, _) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", "br"), ("if-none-match", &etag)]);
        assert_eq!(s, 304, "conditional br");
        // a stale sidecar (older than the wasm) is ignored, and nothing varies
        let old = SystemTime::now() - Duration::from_secs(3600);
        for e in ["br", "gz"] {
            let f = std::fs::File::options().write(true).open(dir.path().join(format!("wasm/beatscript_bg.wasm.{e}"))).unwrap();
            f.set_modified(old).unwrap();
        }
        let (_, h, b) = get("/wasm/beatscript_bg.wasm", &[("accept-encoding", "br, gzip")]);
        assert_eq!(g(&h, header::CONTENT_ENCODING), "");
        assert_eq!(b, raw);
        // js is never looked up for sidecars
        std::fs::write(dir.path().join("js/app.js.gz"), b"GZ").unwrap();
        let (_, h, _) = get("/js/app.js", &[("accept-encoding", "gzip")]);
        assert_eq!(g(&h, header::CONTENT_ENCODING), "");
    }

    #[test]
    fn decode() {
        assert_eq!(percent_decode("/a%20b/%zz%2"), "/a b/%zz%2");
    }

    #[test]
    fn inject() {
        let d = b"<html><head><!-- beatscript:card-meta --></head></html>";
        let o = String::from_utf8(inject_generic(d, "https://x.test/", "/p")).unwrap();
        assert!(o.contains(r#"<link rel="canonical" href="https://x.test/p">"#));
        assert!(!o.contains("beatscript:card-meta"));
        // the site's linked data: free, its code (MIT) at the repository
        let at = o.find(r#"<script type="application/ld+json">"#).expect("no site JSON-LD");
        let json = &o[at + 35..at + o[at..].find("</script>").unwrap()];
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        for n in v["@graph"].as_array().unwrap() {
            assert_eq!(n["codeRepository"], "https://github.com/stackdump/beatscript", "{n}");
            assert_eq!(n["license"], "https://opensource.org/licenses/MIT");
            assert_eq!(n["isAccessibleForFree"], true);
        }
        assert_eq!(v["@graph"][1]["@type"], "SoftwareApplication");
    }

    #[test]
    fn the_editor_page_carries_a_program_block_at_the_end_of_its_body() {
        let dir = tempfile::tempdir().unwrap();
        let index = "<!doctype html><html><head><title>BeatScript</title>\n<!-- beatscript:card-meta -->\n</head><body><main>editor</main></body></html>\n";
        std::fs::write(dir.path().join("index.html"), index).unwrap();
        let st = Static::new(Some(dir.path().to_path_buf()), "https://beatscript.app", true);
        let body = |r: Response<Body>| String::from_utf8(tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(axum::body::to_bytes(r.into_body(), usize::MAX)).unwrap().to_vec()).unwrap();
        let h = HeaderMap::new();
        let plain = body(st.serve(&Method::GET, "/", &h));
        let view = "<beatscript-view data-editor data-ui=\"bar\"><script type=\"application/ld+json\">{}</script></beatscript-view>";
        let with = body(st.serve_with(&Method::GET, "/", &h, Some(view)));
        assert!(with.contains(&format!("<main>editor</main>{view}\n</body>")), "{with}");
        // the head, card meta and all, is the page's as it always is
        let head = |s: &str| s[..s.find("</head>").unwrap()].to_string();
        assert_eq!(head(&with), head(&plain));
        assert!(head(&plain).contains("og:title") && head(&plain).contains("twitter:card"));
        assert_eq!(with.replace(&format!("{view}\n"), ""), plain);
        assert_eq!(body(st.serve_with(&Method::GET, "/", &h, None)), plain);
    }
}
