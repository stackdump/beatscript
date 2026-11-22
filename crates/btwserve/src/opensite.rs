//! Open data on the site (docs/OPEN-DATA.md): `/sitemap.xml`, `/feed.json`, `/feed.atom`, `/robots.txt` (when the
//! static site has none), the dumps `btw-dump` writes (`/dumps/`, `/dumps/latest.json`, `/dumps/<name>.tar.gz`) and the
//! public-domain phrasebook's and theme library's files (`/phrasebook.txt`, `/phrasebook.jsonld`, `/phrasebook/<CID>.txt|.jsonld`,
//! and the same under `/themes`).
//! Every one is a public read: GET and HEAD, CORS open (main.rs `public_read`), no cookies read. What they list is
//! `btwserve::opendata`'s: public CC0 programs only.

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use btwserve::opendata;

use crate::{content, App};

pub fn is_route(path: &str) -> bool {
    matches!(path, "/sitemap.xml" | "/feed.json" | "/feed.atom" | "/robots.txt" | "/dumps" | "/dumps/") || path.starts_with("/dumps/") || btwserve::phrasebook::is_data_route(path) || btwserve::themes::is_data_route(path)
}

fn body(method: &Method, headers: &HeaderMap, ct: &'static str, cache: &'static str, b: Bytes) -> Response<Body> {
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::ETAG, HeaderValue::from_str(&format!("\"{}\"", content::sha_hex(&b, 12))).unwrap());
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let n = b.len() as u64;
    content::serve(h, &content::Req { method, headers }, n, Some(b))
}

/// The answer for an open-data route; None when `path` is not one (or `/robots.txt` is the static site's own).
pub fn respond(app: &App, method: &Method, headers: &HeaderMap, path: &str) -> Option<Response<Body>> {
    if !is_route(path) {
        return None;
    }
    if path == "/robots.txt" && app.cfg.web.as_ref().is_some_and(|w| w.join("robots.txt").exists()) {
        return None;
    }
    if method != Method::GET && method != Method::HEAD {
        let mut r = content::text_error(405, "method not allowed");
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return Some(r);
    }
    let o = app.cfg.public_url.trim_end_matches('/');
    const SHORT: &str = "public, max-age=300";
    // the public-domain phrasebook's files: the corpus and its linked data, and each by the one CID this server ships
    if let Some(answer) = btwserve::phrasebook::data(path, o).or_else(|| btwserve::themes::data(path, o)) {
        return Some(match answer {
            Some(d) => body(method, headers, d.content_type, if d.immutable { "public, max-age=31536000, immutable" } else { SHORT }, Bytes::from(d.body)),
            None => content::text_error(404, "not found"),
        });
    }
    Some(match path {
        "/sitemap.xml" => body(method, headers, "application/xml; charset=utf-8", SHORT, Bytes::from(opendata::sitemap(o, &app.catalog.entries()))),
        "/feed.json" => body(method, headers, "application/feed+json; charset=utf-8", SHORT, Bytes::from(serde_json::to_vec_pretty(&opendata::json_feed(o, &app.catalog.entries())).unwrap())),
        "/feed.atom" => body(method, headers, "application/atom+xml; charset=utf-8", SHORT, Bytes::from(opendata::atom_feed(o, &app.catalog.entries()))),
        "/robots.txt" => body(method, headers, "text/plain; charset=utf-8", SHORT, Bytes::from(format!("User-agent: *\nAllow: /\nSitemap: {o}/sitemap.xml\n"))),
        "/dumps" | "/dumps/" => body(method, headers, "application/json; charset=utf-8", SHORT, Bytes::from(serde_json::to_vec_pretty(&dump_index(app, o)).unwrap())),
        "/dumps/latest.json" => match std::fs::read(app.cfg.dump_dir.join("latest.json")) {
            Ok(b) => body(method, headers, "application/json; charset=utf-8", SHORT, Bytes::from(b)),
            Err(_) => content::text_error(404, "no dump has been made yet"),
        },
        _ => {
            let name = path.strip_prefix("/dumps/").unwrap_or("");
            if !dump_name_ok(name) {
                return Some(content::text_error(404, "not found"));
            }
            match std::fs::read(app.cfg.dump_dir.join(name)) {
                // its name is its content: never changes
                Ok(b) => body(method, headers, "application/gzip", "public, max-age=31536000, immutable", Bytes::from(b)),
                Err(_) => content::text_error(404, "not found"),
            }
        }
    })
}

/// `beatscript-cc0-<16 hex>.tar.gz` and nothing else (no path, no dot file).
fn dump_name_ok(n: &str) -> bool {
    n.strip_prefix("beatscript-cc0-").and_then(|r| r.strip_suffix(".tar.gz")).is_some_and(|h| h.len() == 16 && h.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// `/dumps/`: the archives on disk, the newest pointer, and what they are.
fn dump_index(app: &App, origin: &str) -> Value {
    let mut files: Vec<Value> = std::fs::read_dir(&app.cfg.dump_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            dump_name_ok(&n).then(|| json!({"name": n, "url": format!("{origin}/dumps/{n}"), "bytes": e.metadata().map(|m| m.len()).unwrap_or(0)}))
        })
        .collect();
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let latest: Value = std::fs::read(app.cfg.dump_dir.join("latest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    json!({
        "description": "Content-addressed archives of every public CC0 BeatScript program: programs/<cid>.btw (canonical text), programs/<cid>.jsonld and manifest.jsonld (the CIDs). The same programs are the same bytes and the same name.",
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
        "latest": latest,
        "dumps": files,
        "docs": format!("{origin}/docs/open-data"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_names() {
        assert!(dump_name_ok("beatscript-cc0-0123456789abcdef.tar.gz"));
        for bad in ["../x", "beatscript-cc0-0123.tar.gz", ".latest.json.tmp", "latest.json", "beatscript-cc0-0123456789abcdeg.tar.gz", "beatscript-cc0-0123456789abcdef.tar.gz/x"] {
            assert!(!dump_name_ok(bad), "{bad}");
        }
    }
}
