//! The public-domain phrasebook over HTTP, against the real `beatscript-serve` binary: `/phrasebook` (the page),
//! `/phrasebook.txt` and `/phrasebook.jsonld`, and each of the two by CID (`/phrasebook/<CID>.txt|.jsonld`), which
//! answer only for the CID the server ships and are `404` for any other. The corpus is `djengine`'s file; every view
//! is built from it, so the tests hold the views to it.
//!
//! `cargo test -p beatscript-serve --test phrasebook_routes`.

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Child;
use std::time::Duration;

use serde_json::{json, Value};

struct Server {
    child: Child,
    addr: String,
    _dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    std::fs::write(d.join("site.json"), json!({"site_id": "siteSITEsiteSITEsite01", "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    // a server of its own (tests/common: a -version per server, /healthz names it, a dead child retried on another port)
    let (child, addr) = common::serve(|_| common::flags(&["-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", "https://beatscript.app", "-web", "", "-docs", ""]), false);
    Server { child, addr, _dir: dir }
}

struct Resp {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

fn dechunk(b: &str) -> String {
    let mut out = String::new();
    let mut rest = b;
    while let Some((len, tail)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(len.trim(), 16) else { return b.to_string() };
        if n == 0 || tail.len() < n {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].trim_start_matches("\r\n");
    }
    out
}

impl Server {
    fn get(&self, method: &str, path: &str) -> Resp {
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: https://reader.example\r\nConnection: close\r\n\r\n", self.addr).as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let raw = String::from_utf8_lossy(&raw).to_string();
        let (head, b) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let headers: BTreeMap<String, String> = head.lines().skip(1).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        let body = if headers.get("transfer-encoding").is_some_and(|t| t.contains("chunked")) { dechunk(b) } else { b.to_string() };
        Resp { status, headers, body }
    }
}

fn cid() -> String {
    beatscript_serve::phrasebook::cid()
}

#[test]
fn the_page_is_html_with_every_line_and_no_inline_style() {
    let s = start();
    let r = s.get("GET", "/phrasebook");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.headers["content-type"].starts_with("text/html"), "{:?}", r.headers);
    assert_eq!(r.headers["access-control-allow-origin"], "*");
    assert!(r.headers["content-security-policy"].contains("style-src 'self'"), "{:?}", r.headers);
    for p in djengine::vocal::phoneme::pd_phrases() {
        assert!(r.body.contains(&format!("<tr id=\"{}\">", p.id)), "{}", p.id);
        assert!(r.body.contains(&format!("sing @{} on melody", p.id)), "{}", p.id);
    }
    assert!(r.body.contains(&cid()) && r.body.contains("Public-domain phrasebook"));
    assert!(!r.body.contains("style=") && !r.body.contains("<style"), "inline style");
    // a page, so HEAD answers too, with no body
    let h = s.get("HEAD", "/phrasebook");
    assert_eq!(h.status, 200);
    assert!(h.body.is_empty());
}

#[test]
fn the_file_and_the_linked_data_are_the_corpus() {
    let s = start();
    let t = s.get("GET", "/phrasebook.txt");
    assert_eq!(t.status, 200);
    assert!(t.headers["content-type"].starts_with("text/plain"), "{:?}", t.headers);
    assert_eq!(t.headers["access-control-allow-origin"], "*");
    assert_eq!(t.body, djengine::vocal::phoneme::pd_phrasebook_text(), "the file as shipped");
    // its CID is the program-CID function over exactly that text
    assert_eq!(djlive::compile::program_cid(&t.body), cid());
    let j = s.get("GET", "/phrasebook.jsonld");
    assert_eq!(j.status, 200);
    assert!(j.headers["content-type"].starts_with("application/ld+json"), "{:?}", j.headers);
    assert_eq!(j.headers["access-control-allow-origin"], "*");
    let v: Value = serde_json::from_str(&j.body).unwrap();
    assert_eq!(v["@id"], cid());
    assert_eq!(v["hasPart"].as_array().unwrap().len(), djengine::vocal::phoneme::pd_phrases().len());
    assert_eq!(v["distribution"][0]["contentUrl"], format!("https://beatscript.app/phrasebook/{}.txt", cid()));
}

#[test]
fn a_cid_route_answers_for_the_cid_it_ships_and_is_404_for_any_other() {
    let s = start();
    let c = cid();
    let t = s.get("GET", &format!("/phrasebook/{c}.txt"));
    assert_eq!(t.status, 200);
    assert_eq!(t.body, djengine::vocal::phoneme::pd_phrasebook_text());
    assert!(t.headers["cache-control"].contains("immutable"), "a path named by the bytes' CID never changes: {:?}", t.headers);
    assert_eq!(t.headers["access-control-allow-origin"], "*");
    let j = s.get("GET", &format!("/phrasebook/{c}.jsonld"));
    assert_eq!(j.status, 200);
    assert_eq!(serde_json::from_str::<Value>(&j.body).unwrap()["@id"], c);
    assert_eq!(j.body, s.get("GET", "/phrasebook.jsonld").body);
    // a new version of the corpus is a new CID, and an old or invented one is not served
    let other = djlive::compile::program_cid(&format!("{}# an older version\n", djengine::vocal::phoneme::pd_phrasebook_text()));
    for bad in [format!("/phrasebook/{other}.txt"), format!("/phrasebook/{other}.jsonld"), format!("/phrasebook/{c}.json"), format!("/phrasebook/{c}"), "/phrasebook/zNotACid.txt".to_string(), "/phrasebook/".to_string()] {
        let r = s.get("GET", &bad);
        assert_eq!(r.status, 404, "{bad}: {}", r.body);
    }
}

#[test]
fn the_sitemap_and_llms_txt_list_it() {
    let s = start();
    let sm = s.get("GET", "/sitemap.xml");
    assert!(sm.body.contains("<loc>https://beatscript.app/phrasebook</loc>"), "{}", sm.body);
    let l = s.get("GET", "/llms.txt");
    for u in ["/phrasebook", "/phrasebook.txt", "/phrasebook.jsonld", "/phrasebook/<CID>.txt", "/phrasebook/<CID>.jsonld"] {
        assert!(l.body.contains(&format!("https://beatscript.app{u}")), "llms.txt lacks {u}");
    }
    assert!(l.body.contains("sing @tempest_such_stuff on melody"));
}
