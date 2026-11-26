//! Android App Links verification, against the real `beatscript-serve` binary: `/.well-known/assetlinks.json` is answered
//! by the server itself (the statement embedded from `src/public/assetlinks.json`), 200, `application/json`, never a
//! redirect, whatever Host the request names (beatscript.app, btw.bitwrap.io, beats.bitwrap.io), with no file configured.
//!
//! `cargo test -p beatscript-serve --test assetlinks_route`.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Child;
use std::time::Duration;

use serde_json::Value;

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
    std::fs::write(d.join("site.json"), r#"{"site_id": "siteSITEsiteSITEsite01", "created": "2026-11-19T00:00:00Z"}"#).unwrap();
    std::fs::create_dir_all(d.join("programs")).unwrap();
    let (child, addr) = common::serve(|_| common::flags(&["-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", "https://beatscript.app", "-legacy-hosts", "btw.bitwrap.io", "-docs", ""]), false);
    Server { child, addr, _dir: dir }
}

/// (status line's code, lower-cased headers joined, body) of a request that names `host`.
fn get(s: &Server, method: &str, host: &str, path: &str) -> (u16, String, Vec<u8>) {
    let mut c = TcpStream::connect(&s.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    c.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = c.read_to_end(&mut raw);
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..at]).to_ascii_lowercase();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head, raw[at + 4..].to_vec())
}

#[test]
fn every_host_gets_the_four_fingerprint_statement_with_no_redirect() {
    let s = start();
    for host in ["beatscript.app", "btw.bitwrap.io", "beats.bitwrap.io", "127.0.0.1"] {
        let (status, head, body) = get(&s, "GET", host, "/.well-known/assetlinks.json");
        assert_eq!(status, 200, "{host}: {head}");
        assert!(!head.contains("\r\nlocation:"), "{host}: a redirect: {head}");
        assert!(head.contains("\r\ncontent-type: application/json"), "{host}: {head}");
        // chunked or not, the JSON is in there
        let text = String::from_utf8_lossy(&body);
        let start = text.find('[').unwrap();
        let end = text.rfind(']').unwrap();
        let v: Value = serde_json::from_str(&text[start..=end]).unwrap_or_else(|e| panic!("{host}: {e}: {text}"));
        assert_eq!(v[0]["target"]["package_name"], "app.beatscript", "{host}");
        assert_eq!(v[0]["target"]["sha256_cert_fingerprints"].as_array().unwrap().len(), 4, "{host}");
        // HEAD answers the same
        let (hs, hh, _) = get(&s, "HEAD", host, "/.well-known/assetlinks.json");
        assert_eq!(hs, 200, "{host}: {hh}");
    }
}
