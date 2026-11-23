//! A `beatscript-serve` of the test's own, for the tests that run the real binary (`mod common;`).
//!
//! A test picks a port by binding `127.0.0.1:0` and letting it go, then starts the server on it. The tests of a file run in
//! parallel, so two can draw one port: the second server exits ("Address already in use"), a bare connect reaches the first,
//! and the first goes when its own test ends, so the second test's requests answer nothing (a status 0, an SSE stream with
//! no hello). Here each server is started with a `-version` of its own and is ready when its `/healthz` names that version;
//! one that exits first, or a port that answers with another version, is tried again on another port.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Start `beatscript-serve` with the flags `args` gives for an address (`127.0.0.1:PORT`: every flag but `-addr` and
/// `-version`, which this adds), its stdout and stderr piped when `piped` (else discarded), and wait until it answers as
/// itself. Gives the child and its address.
pub fn serve(args: impl Fn(&str) -> Vec<String>, piped: bool) -> (Child, String) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let version = format!("test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst));
    let stdio = || if piped { Stdio::piped() } else { Stdio::null() };
    for _ in 0..5 {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let addr = format!("127.0.0.1:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_beatscript-serve"))
            .args(args(&addr))
            .args(["-addr", addr.as_str(), "-version", version.as_str()])
            .stdout(stdio())
            .stderr(stdio())
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
            assert!(t0.elapsed() < Duration::from_secs(60), "beatscript-serve did not start on {addr}");
            std::thread::sleep(Duration::from_millis(50));
        };
        if ours {
            return (child, addr);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!("beatscript-serve did not start on a free port (5 tries)");
}

/// `GET /healthz` on `addr`: the whole response when it is a 200.
pub fn healthz(addr: &str) -> Option<String> {
    let mut s = TcpStream::connect(addr).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(format!("GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    Some(raw).filter(|r| r.starts_with("HTTP/1.1 200"))
}

/// A flag list as owned strings.
pub fn flags(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}
