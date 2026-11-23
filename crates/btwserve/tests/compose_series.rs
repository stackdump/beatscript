//! Compose end to end against the real `btwserve` binary with the recorded-fixture model (`-model fake`).
//!
//! - `several_prompts_in_a_row_from_a_paired_phone`: the phone's Code tab sends the program in its editor
//!   (`current`) with every prompt after the first. Five prompts in a row from one paired phone must all compose
//!   (the bug: only the first did, every later one was a 400 "send a JSON body").
//! - `several_prompts_in_a_row_on_the_web_route`: the same series on `POST /api/compose`.
//! - `a_current_that_cannot_go_to_the_model_is_refused_clearly`: a private current is a 400, an oversize one a 413,
//!   and an unknown field names itself (a client can drop an optional field this site does not know).
//! - `refine_clears_the_warnings` / `refine_never_makes_it_worse` / `max_refine_0_turns_it_off`: a program that passes
//!   with warnings gets a refine round; its reply is kept only when it has fewer warnings.
//!
//! `cargo test -p btwserve --test compose_series`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SITE: &str = "siteSITEsiteSITEsite01";
const A: &str = "ed25519:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "ed25519:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN_B: &str = "btw_tokenBtokenBtokenBtokenBtokenBtokenBtokenB";
const PHONE_A: &str = "devPHONEdevPHONEdevPHO";

struct Server {
    child: Child,
    addr: String,
    _dir: tempfile::TempDir,
    out: Arc<Mutex<String>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn phone_key() -> SigningKey {
    SigningKey::from_bytes(&[1; 32])
}

fn start(extra: &[&str]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    std::fs::write(d.join("site.json"), json!({"site_id": SITE, "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    let tok = json!({"hash": hex::encode(Sha256::digest(TOKEN_B.as_bytes())), "user": B, "created": "2026-11-19T00:00:00Z"});
    std::fs::write(d.join("tokens.json"), json!([tok]).to_string()).unwrap();
    let dev = json!({"id": PHONE_A, "owner": A, "name": "Pixel 9", "key": B64.encode(phone_key().verifying_key().to_bytes()), "paired": "2026-11-19T00:00:00Z"});
    std::fs::write(d.join("devices.json"), json!([dev]).to_string()).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr = format!("127.0.0.1:{port}");
    let mut args: Vec<String> = ["-addr", &addr, "-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", &format!("http://{addr}"), "-web", "", "-docs", ""]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.extend(["-model", "fake", "-compose-user-hour", "50", "-compose-user-day", "50", "-compose-ip-day", "50"].map(String::from));
    args.extend(extra.iter().map(|s| s.to_string()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_btwserve")).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let out = Arc::new(Mutex::new(String::new()));
    for pipe in [Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>, Box::new(child.stderr.take().unwrap())] {
        let out = out.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(pipe).lines().map_while(Result::ok) {
                out.lock().unwrap().push_str(&(l + "\n"));
            }
        });
    }
    let t0 = Instant::now();
    while TcpStream::connect(&addr).is_err() {
        assert!(t0.elapsed() < Duration::from_secs(30), "btwserve did not start: {}", out.lock().unwrap());
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, addr, _dir: dir, out }
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
    fn call(&self, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> (u16, Value) {
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", self.addr, body.len());
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(body).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let raw = String::from_utf8_lossy(&raw).into_owned();
        let (head, b) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(b) } else { b.to_string() };
        let v = serde_json::from_str(&body).unwrap_or_else(|_| panic!("not JSON ({status}): {body}\nserver: {}", self.out.lock().unwrap()));
        (status, v)
    }

    /// `POST /api/device/compose`, signed as the paired phone signs it.
    fn phone_compose(&self, req: &Value) -> (u16, Value) {
        let path = "/api/device/compose";
        let body = req.to_string();
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let msg = format!("btw-pair/2 request\nsite: {SITE}\ndevice: {PHONE_A}\nmethod: POST\npath: {path}\nts: {ts}\nbody: {}", hex::encode(Sha256::digest(body.as_bytes())));
        let sig = B64.encode(phone_key().sign(msg.as_bytes()).to_bytes());
        self.call("POST", path, &[("Authorization", format!("BtwDevice {PHONE_A}.{ts}.{sig}"))], body.as_bytes())
    }

    /// `POST /api/compose` with a bearer token.
    fn web_compose(&self, req: &Value) -> (u16, Value) {
        self.call("POST", "/api/compose", &[("Authorization", format!("Bearer {TOKEN_B}"))], req.to_string().as_bytes())
    }
}

/// The prompts of a session: quotes, a newline, non-ASCII and an emoji, so a body that is not JSON-encoded shows.
const PROMPTS: [&str; 5] = [
    "dark techno 128 with an acid line",
    "make it faster",
    "add a \"hi-hat\" roll\nand a break",
    "ünïcödé 🎛 bass, slower",
    "keep it, but end after 32 bars",
];

fn series(s: &Server, compose: impl Fn(&Server, &Value) -> (u16, Value), route: &str) {
    let mut current = String::new();
    for (i, prompt) in PROMPTS.iter().enumerate() {
        let mut req = json!({"prompt": prompt});
        if !current.is_empty() {
            req["current"] = json!(current);
        }
        let (st, out) = compose(s, &req);
        assert_eq!(st, 200, "{route} prompt {} ({prompt:?}, current {} bytes): {out}", i + 1, current.len());
        let program = out["program"].as_str().unwrap_or("");
        assert!(!program.is_empty() && out["check"]["ok"] == true && !out["cid"].as_str().unwrap_or("").is_empty(), "{route} prompt {}: {out}", i + 1);
        current = program.to_string(); // what the editor holds now
    }
}

#[test]
fn several_prompts_in_a_row_from_a_paired_phone() {
    let s = start(&[]);
    series(&s, Server::phone_compose, "phone");
}

#[test]
fn several_prompts_in_a_row_on_the_web_route() {
    let s = start(&[]);
    series(&s, Server::web_compose, "web");
}

#[test]
fn a_current_that_cannot_go_to_the_model_is_refused_clearly() {
    let s = start(&[]);
    let private = "#! visibility: private\ntempo 120\ndeck A: kick x ~ x ~\n";
    for compose in [Server::phone_compose, Server::web_compose] {
        let (st, out) = compose(&s, &json!({"prompt": "x", "current": private}));
        assert_eq!(st, 400, "private current: {out}");
        assert!(out["error"]["message"].as_str().unwrap_or("").contains("current program"), "{out}");
        let (st, out) = compose(&s, &json!({"prompt": "x", "current": "# long\n".repeat(4000)}));
        assert_eq!(st, 413, "oversize current: {out}");
        let (st, out) = compose(&s, &json!({"prompt": "x", "model": "pro"}));
        assert!(st == 400 && out["error"]["message"].as_str().unwrap_or("").contains("unknown field"), "unknown field: {st} {out}");
    }
}

fn attempts(out: &Value) -> Vec<Value> {
    out["attempts"].as_array().cloned().unwrap_or_default()
}

#[test]
fn refine_clears_the_warnings() {
    let s = start(&[]);
    let (st, out) = s.web_compose(&json!({"prompt": "fake-refine-clears: house"}));
    assert_eq!(st, 200, "{out}");
    let a = attempts(&out);
    assert_eq!(a.len(), 2, "one attempt and one refine round: {out}");
    assert!(a[0]["ok"] == true && a[0].get("refine").is_none(), "the first attempt passed, with warnings: {}", a[0]);
    assert!(a[0]["diagnostics"].as_array().is_some_and(|d| d.iter().any(|d| d["severity"] == "warning")), "the fixture's first answer must warn: {}", a[0]);
    assert!(a[1]["refine"] == true && a[1]["ok"] == true, "the refine round was kept: {}", a[1]);
    assert_eq!(out["check"]["warnings"], 0, "the result is the refined program: {out}");
    assert!(out["program"].as_str().unwrap().contains("let hh = ~ ~ x ~"), "{out}");
}

#[test]
fn refine_never_makes_it_worse() {
    let s = start(&[]);
    let (st, out) = s.web_compose(&json!({"prompt": "fake-refine-worse: house"}));
    assert_eq!(st, 200, "{out}");
    let a = attempts(&out);
    assert_eq!(a.len(), 2, "{out}");
    assert!(a[1]["refine"] == true && a[1]["ok"] == false, "the worse reply was not kept: {}", a[1]);
    assert!(out["check"]["warnings"].as_i64().unwrap_or(0) > 0, "the first program is the result: {out}");
    assert!(!out["program"].as_str().unwrap().contains("snare"), "{out}");
}

#[test]
fn max_refine_0_turns_it_off() {
    let s = start(&["-max-refine", "0"]);
    let (st, out) = s.web_compose(&json!({"prompt": "fake-refine-clears: house"}));
    assert_eq!(st, 200, "{out}");
    assert_eq!(attempts(&out).len(), 1, "{out}");
    assert!(out["check"]["warnings"].as_i64().unwrap_or(0) > 0, "{out}");
}

#[test]
fn a_program_with_no_warnings_gets_no_refine_round() {
    let s = start(&[]);
    let (st, out) = s.web_compose(&json!({"prompt": "dark techno"}));
    assert_eq!(st, 200, "{out}");
    assert_eq!(out["check"]["warnings"], 0, "the default fixture is clean: {out}");
    assert_eq!(attempts(&out).len(), 1, "{out}");
}
