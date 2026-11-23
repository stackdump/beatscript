//! The embeddable player over HTTP (project-datum 6664), against the real `beatscript-serve` binary: `/embed/<CID>` is a page
//! any site may frame, for a stored Creative Commons program only; every other route still refuses to be framed.
//!
//! `cargo test -p beatscript-serve --test embed_routes`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

struct Server {
    child: Child,
    addr: String,
    dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Programs that check (a tiny one renders in well under a second).
const PUBLIC: &str = "#! name: embed test\ntempo 128\nkey am\nsection groove 2 bars {\n  deck A: kick x ~ x ~\n}\narrange groove\n";
const PRIVATE: &str = "tempo 90\nkey cm\nsection groove 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange groove\n";
const UNSTORED: &str = "tempo 100\nkey dm\nsection groove 2 bars {\n  deck A: kick x x ~ ~\n}\narrange groove\n";

fn canonical(text: &str) -> (String, String) {
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
    assert_eq!(rep["ok"], true, "{rep}");
    (rep["cid"].as_str().unwrap().to_string(), rep["canonical"].as_str().unwrap().to_string())
}

fn store(programs: &Path, text: &str, meta: Value) -> String {
    let (cid, canon) = canonical(text);
    std::fs::write(programs.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": canon, "meta": meta})).unwrap()).unwrap();
    cid
}

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    std::fs::write(d.join("site.json"), json!({"site_id": "siteSITEsiteSITEsite01", "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    std::fs::create_dir_all(d.join("programs")).unwrap();
    // the site's static files: only the element's script matters here (beats site/web/embed.js)
    std::fs::create_dir_all(d.join("web")).unwrap();
    std::fs::write(d.join("web/embed.js"), "/* <beatscript-player> */\n").unwrap();
    // a port picked here can be taken by another test's server before this one binds it: each server says its own
    // version, and one that is not ours, or exits, is retried on another port
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let version = format!("embed-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    for _ in 0..5 {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let addr = format!("127.0.0.1:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_beatscript-serve"))
            .args(["-addr", &addr, "-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", "https://beatscript.app", "-web", &d.join("web").to_string_lossy(), "-docs", "", "-version", &version])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let t0 = Instant::now();
        let ours = loop {
            if child.try_wait().ok().flatten().is_some() {
                break false;
            }
            if let Some(body) = healthz(&addr) {
                break body.contains(&format!("\"version\":\"{version}\""));
            }
            assert!(t0.elapsed() < Duration::from_secs(60), "beatscript-serve did not start");
            std::thread::sleep(Duration::from_millis(50));
        };
        if ours {
            return Server { child, addr, dir };
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!("beatscript-serve did not start on a free port");
}

fn healthz(addr: &str) -> Option<String> {
    let mut s = TcpStream::connect(addr).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(format!("GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    Some(raw).filter(|r| r.starts_with("HTTP/1.1 200"))
}

struct Resp {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Resp {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = b;
    loop {
        let Some(i) = rest.windows(2).position(|w| w == b"\r\n") else { break };
        let Ok(n) = usize::from_str_radix(String::from_utf8_lossy(&rest[..i]).trim(), 16) else { return b.to_vec() };
        let tail = &rest[i + 2..];
        if n == 0 || tail.len() < n {
            break;
        }
        out.extend_from_slice(&tail[..n]);
        rest = &tail[n..];
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
    }
    out
}

impl Server {
    fn programs(&self) -> std::path::PathBuf {
        self.dir.path().join("programs")
    }

    fn get(&self, method: &str, path: &str) -> Resp {
        self.get_with(method, path, "")
    }

    /// `extra`: more header lines, each ending in CRLF.
    fn get_with(&self, method: &str, path: &str, extra: &str) -> Resp {
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n{extra}Connection: close\r\n\r\n", self.addr).as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
        let head = String::from_utf8_lossy(&raw[..at]).to_string();
        let b = if at + 4 <= raw.len() { &raw[at + 4..] } else { &[][..] };
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let headers: BTreeMap<String, String> = head.lines().skip(1).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        let body = if headers.get("transfer-encoding").is_some_and(|t| t.contains("chunked")) { dechunk(b) } else { b.to_vec() };
        Resp { status, headers, body }
    }
}

#[test]
fn a_stored_program_embeds_and_may_be_framed_by_any_site() {
    let s = start();
    let cid = store(&s.programs(), PUBLIC, json!({"license": "CC-BY-4.0", "visibility": "public", "name": "embed test", "author": "ann"}));
    let r = s.get("GET", &format!("/embed/{cid}"));
    assert_eq!(r.status, 200, "{}", r.text());
    let html = r.text();
    assert!(r.headers["content-type"].starts_with("text/html"), "{:?}", r.headers);
    let csp = &r.headers["content-security-policy"];
    assert!(csp.ends_with("frame-ancestors *") && csp.contains("style-src 'self'") && csp.contains("script-src 'self' 'wasm-unsafe-eval'"), "{csp}");
    assert!(!r.headers.contains_key("x-frame-options"), "{:?}", r.headers);
    // CSP-clean: no inline style, no inline script (the module is a file, the program block is JSON-LD data)
    assert!(!html.contains("style=") && !html.contains("<style"), "{html}");
    for (i, _) in html.match_indices("<script") {
        let tag = &html[i..i + html[i..].find('>').unwrap()];
        assert!(tag.contains(" src=\"/js/embed-player.js\"") || tag.contains("type=\"application/ld+json\""), "inline script: {tag}");
    }
    assert!(html.contains("embed test") && html.contains("by ann") && html.contains("creativecommons.org/licenses/by/4.0/"), "{html}");
    assert!(html.contains(&format!("https://beatscript.app/?cid={cid}")), "the editor link");
    assert!(html.contains("<beatscript-view data-editor"), "the program block");
    // HEAD answers too
    let h = s.get("HEAD", &format!("/embed/{cid}"));
    assert_eq!(h.status, 200);
    assert!(h.body.is_empty());
    // nothing else became frameable
    for p in ["/", &format!("/p/{cid}"), &format!("/c/{cid}"), "/embed"] {
        let o = s.get("GET", p);
        assert_eq!(o.headers.get("x-frame-options").map(String::as_str), Some("DENY"), "{p}: {:?}", o.headers);
        assert!(o.headers["content-security-policy"].ends_with("frame-ancestors 'none'"), "{p}");
    }
}

#[test]
fn a_private_unknown_or_unstored_program_does_not_embed() {
    let s = start();
    let private = store(&s.programs(), PRIVATE, json!({"license": "CC0-1.0", "visibility": "private"}));
    let (unstored, _) = canonical(UNSTORED);
    let rights = store(&s.programs(), UNSTORED.replace("tempo 100", "tempo 101").as_str(), json!({"license": "all-rights-reserved", "visibility": "public"}));
    for p in [format!("/embed/{private}"), format!("/embed/{unstored}"), format!("/embed/{rights}"), "/embed/zabc".into(), "/embed/".into(), "/embed/nope".into(), format!("/embed/{private}/x")] {
        let r = s.get("GET", &p);
        assert_eq!(r.status, 404, "{p}: {}", r.text());
    }
}

#[test]
fn the_elements_script_is_served_and_readable_from_any_origin() {
    let s = start();
    let r = s.get_with("GET", "/embed.js", "Origin: https://blog.example\r\n");
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.headers["content-type"].contains("javascript"), "{:?}", r.headers);
    assert_eq!(r.headers.get("access-control-allow-origin").map(String::as_str), Some("*"));
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[test]
fn oembed_answers_for_a_p_or_c_link_and_pages_say_where_it_is() {
    let s = start();
    let cid = store(&s.programs(), PUBLIC, json!({"license": "CC0-1.0", "visibility": "public", "name": "embed test", "author": "ann"}));
    for page in [format!("https://beatscript.app/p/{cid}"), format!("https://beatscript.app/c/{cid}")] {
        let r = s.get("GET", &format!("/oembed?url={}&format=json", enc(&page)));
        assert_eq!(r.status, 200, "{page}: {}", r.text());
        assert!(r.headers["content-type"].starts_with("application/json"), "{:?}", r.headers);
        let o: Value = serde_json::from_slice(&r.body).expect("valid JSON");
        assert_eq!((o["version"].as_str(), o["type"].as_str()), (Some("1.0"), Some("rich")));
        assert_eq!((o["title"].as_str(), o["author_name"].as_str(), o["provider_name"].as_str()), (Some("embed test"), Some("ann"), Some("BeatScript")));
        assert_eq!((o["width"].as_u64(), o["height"].as_u64()), (Some(480), Some(152)));
        assert!(o["html"].as_str().unwrap().contains(&format!("src=\"https://beatscript.app/embed/{cid}\"")), "{o}");
        assert_eq!(o["provider_url"], "https://beatscript.app/");
        // the page links it
        let html = s.get("GET", &page.replace("https://beatscript.app", "")).text();
        let tag = format!("<link rel=\"alternate\" type=\"application/json+oembed\" href=\"https://beatscript.app/oembed?url={}&amp;format=json\"", enc(&page));
        assert!(html.contains(&tag), "{page} lacks {tag}:\n{html}");
    }
    // without format= it is json too; maxheight makes it the compact player
    let r = s.get("GET", &format!("/oembed?url={}&maxheight=90", enc(&format!("https://beatscript.app/p/{cid}"))));
    let o: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(o["height"].as_u64(), Some(80), "{o}");
    // xml is not served
    assert_eq!(s.get("GET", &format!("/oembed?url={}&format=xml", enc(&format!("https://beatscript.app/p/{cid}")))).status, 501);
    assert_eq!(s.get("GET", "/oembed").status, 400);
}

#[test]
fn oembed_is_404_for_anything_but_an_embeddable_programs_page() {
    let s = start();
    let cid = store(&s.programs(), PUBLIC, json!({"license": "CC0-1.0", "visibility": "public"}));
    let private = store(&s.programs(), PRIVATE, json!({"visibility": "private"}));
    let (unstored, _) = canonical(UNSTORED);
    for url in [
        format!("https://beatscript.app/p/{private}"),
        format!("https://beatscript.app/c/{unstored}"),
        format!("https://elsewhere.example/p/{cid}"),
        format!("https://beatscript.app/embed/{cid}"),
        "https://beatscript.app/docs/".into(),
        "https://beatscript.app/".into(),
    ] {
        let r = s.get("GET", &format!("/oembed?url={}&format=json", enc(&url)));
        assert_eq!(r.status, 404, "{url}: {}", r.text());
        let _: Value = serde_json::from_slice(&r.body).expect("the error is JSON too");
    }
    // a private program's page names no oEmbed
    let html = s.get("GET", &format!("/p/{private}")).text();
    assert!(!html.contains("json+oembed"), "{html}");
}

#[test]
fn the_c_page_card_plays_and_the_p_page_and_others_stay_as_they_were() {
    let s = start();
    let cid = store(&s.programs(), PUBLIC, json!({"license": "CC0-1.0", "visibility": "public"}));
    let html = s.get("GET", &format!("/c/{cid}")).text();
    for tag in [
        "<meta name=\"twitter:card\" content=\"player\">".to_string(),
        format!("<meta name=\"twitter:player\" content=\"https://beatscript.app/embed/{cid}\">"),
        "<meta name=\"twitter:player:width\" content=\"480\">".into(),
        "<meta name=\"twitter:player:height\" content=\"152\">".into(),
        format!("<meta property=\"og:audio\" content=\"https://beatscript.app/api/render/{cid}?format=opus\">"),
        "<meta property=\"og:audio:type\" content=\"audio/ogg\">".into(),
        format!("<meta name=\"twitter:image\" content=\"https://beatscript.app/card/{cid}.png\">"),
    ] {
        assert!(html.contains(&tag), "/c/ lacks {tag}:\n{html}");
    }
    // the short page and a program that does not embed keep the large-image card, no player
    let p = s.get("GET", &format!("/p/{cid}")).text();
    assert!(p.contains("content=\"summary_large_image\"") && !p.contains("twitter:player") && !p.contains("og:audio\""), "{p}");
    let private = store(&s.programs(), PRIVATE, json!({"visibility": "private"}));
    let q = s.get("GET", &format!("/c/{private}")).text();
    assert!(q.contains("content=\"summary_large_image\"") && !q.contains("twitter:player"), "{q}");
}

/// The file og:audio names: the existing render route, its bytes the same on every fetch (a miss, then the cache),
/// and only for a program the site stores.
#[test]
fn the_cards_audio_is_rendered_once_by_cid_and_only_for_a_stored_program() {
    use sha2::{Digest, Sha256};
    let s = start();
    let cid = store(&s.programs(), PUBLIC, json!({"license": "CC0-1.0", "visibility": "public"}));
    let a = s.get("GET", &format!("/api/render/{cid}?format=opus"));
    assert_eq!(a.status, 200, "{}", a.text());
    assert!(a.headers["content-type"].starts_with("audio/ogg"), "{:?}", a.headers);
    assert!(a.body.starts_with(b"OggS") && a.body.len() > 1000, "{} bytes", a.body.len());
    let b = s.get("GET", &format!("/api/render/{cid}?format=opus"));
    assert_eq!(b.status, 200);
    assert_eq!(b.headers.get("x-btw-cache").map(String::as_str), Some("hit"));
    assert!(b.headers["cache-control"].contains("immutable"), "{:?}", b.headers);
    let (ha, hb) = (hex::encode(Sha256::digest(&a.body)), hex::encode(Sha256::digest(&b.body)));
    assert_eq!(ha, hb, "the same bytes from the render and from the cache");
    // a second server renders the same bytes for the same CID: the hash is the CID's, not the run's
    let t = start();
    store(&t.programs(), PUBLIC, json!({"license": "CC0-1.0", "visibility": "public"}));
    let c = t.get("GET", &format!("/api/render/{cid}?format=opus"));
    assert_eq!(hex::encode(Sha256::digest(&c.body)), ha, "a fresh render of {cid}");
    let (unstored, _) = canonical(UNSTORED);
    assert_eq!(s.get("GET", &format!("/api/render/{unstored}?format=opus")).status, 404);
}
