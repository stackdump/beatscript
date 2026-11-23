//! BeatScript data lives on beatscript.app only (Matt, 2026-11-22): no page the site serves, no program's linked data and
//! no MCP answer says "bitwrap.io", except the history and attribution `deploy/bitwrap-allowlist.tsv` lists. This runs the
//! live check itself (`deploy/check-data-on-beatscript-app.py`, which `deploy/live-checks.sh` runs against beatscript.app
//! after every deploy) against the real `beatscript-serve` binary on a loopback port, so the check and the test are one
//! mechanism: a page, a document or a tool answer that names bitwrap.io fails here before it can be deployed.
//!
//! `cargo test -p beatscript-serve --test data_on_beatscript_app` (needs python3, as CI has).

use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn nothing_served_names_bitwrap_io_outside_the_allowlist() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), "2a".repeat(32) + "\n").unwrap();
    std::fs::write(d.join("site.json"), r#"{"site_id": "siteSITEsiteSITEsite01", "created": "2026-11-19T00:00:00Z"}"#).unwrap();
    // -docs holds the generated language docs, as the deploy puts them there
    let docs = d.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::copy(repo.join("docs/BEATSCRIPT.md"), docs.join("BEATSCRIPT.md")).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr = format!("127.0.0.1:{port}");
    let s = |p: &Path| p.to_string_lossy().to_string();
    let _server = Server(
        Command::new(env!("CARGO_BIN_EXE_beatscript-serve"))
            .args(["-addr", &addr, "-data", &s(d), "-log-dir", &s(&d.join("log-rs")), "-public-url", "https://beatscript.app", "-legacy-hosts", "btw.bitwrap.io"])
            .args(["-web", &s(&repo.join("site/web")), "-examples", &s(&repo.join("site/examples")), "-docs", &s(&docs), "-docs-src", &s(&repo.join("site/docs-src"))])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let t0 = Instant::now();
    while TcpStream::connect(&addr).is_err() {
        assert!(t0.elapsed() < Duration::from_secs(60), "beatscript-serve did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = Command::new("python3").arg("-I").arg(repo.join("deploy/check-data-on-beatscript-app.py")).arg(format!("http://{addr}")).output().expect("python3 runs the check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "bitwrap.io outside deploy/bitwrap-allowlist.tsv:\n{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("ok: "), "{stdout}");
}

/// The allowlist is history and attribution only: each line names where, the exact text and why, and none is a link a
/// reader would follow to bitwrap.io.
#[test]
fn the_allowlist_holds_no_link() {
    let list = include_str!("../../../deploy/bitwrap-allowlist.tsv");
    let mut n = 0;
    for line in list.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 3, "where, text, why: {line}");
        assert!(f[0].starts_with('/') || f[0].starts_with("mcp:"), "{line}");
        assert!(f[1].contains("bitwrap.io"), "a line that allows nothing: {line}");
        assert!(!f[1].contains("href="), "a link to follow is not history: {line}");
        assert!(["attribution:", "history:", "legacy:"].iter().any(|k| f[2].starts_with(k)), "{line}");
        n += 1;
    }
    assert!((1..=12).contains(&n), "{n} lines: keep the exceptions few");
}
