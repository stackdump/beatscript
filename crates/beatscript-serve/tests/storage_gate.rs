//! The storage gate and the private relay, end to end against the real `beatscript-serve` binary on a temp data directory.
//!
//! - `every_write_site_is_known`: reads `src/` and fails when a file writes to disk that is not on the list below,
//!   or when a program can be written by anything but `gate::write_program`, or a caller of it skips `gate::admit`.
//! - `every_write_route_is_gated`: the routes that take a body are each either a program write path exercised below
//!   or on the short list of routes that store no program.
//! - `write_paths_refuse_what_is_not_cc`: each program write path, over HTTP and MCP, stores CC programs and refuses
//!   the rest with the gate's message.
//! - `private_push_*`: a private push reaches the phone sealed, opens there, and leaves nothing behind (the data
//!   directory and the server's output are searched for it); another account's phone and an unconnected one are
//!   refused.
//! - `*_arrive_formatted`: what eval, load, queue, share and push_private (and the phone push route) hand on is the text
//!   `fmt` prints (header lines and comments kept), with the CID of the text sent; what does not read goes on unchanged.
//! - `write_paths_keep_the_from_lines`: the save round trip of a text with `# from:` lines (an eject's provenance), over
//!   `PUT /api/shares`, `PUT /api/programs` and MCP `share`: the canonical text is stored, the lines are kept as
//!   `meta.based_on` and read back (`GET /api/programs`, its `.beatscript`, MCP `open`, the page's JSON-LD `isBasedOn`), and lineage
//!   counts each save as vendoring what the lines name (`vendoredFrom`).
//!
//! `cargo test -p beatscript-serve --test storage_gate`.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
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
const TOKEN_A: &str = "btw_tokenAtokenAtokenAtokenAtokenAtokenAtokenA";
const TOKEN_B: &str = "btw_tokenBtokenBtokenBtokenBtokenBtokenBtokenB";
const PHONE_A: &str = "devPHONEdevPHONEdevPHO";
const PHONE_B: &str = "devOTHERdevOTHERdevOTH";

const PLAIN: &str = "tempo 120\ndeck A: kick x ~ x ~\n";
const PLAIN2: &str = "tempo 124\ndeck A: kick x ~ ~ x\n";
const PLAIN3: &str = "tempo 126\ndeck A: kick x x ~ x\n";
const PLAIN4: &str = "tempo 132\ndeck A: kick x ~ x x\n";
/// A cover-like private program: a local sample (by its hash) and `use restricted`.
const PRIVATE: &str = "use restricted\nsample harp from \"sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\" license restricted-private else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~ | bass c2 ~ c2 ~ inst=harp\n";
/// The same with an open-tagged sample and no `use restricted`: still a sample, still refused.
const OPEN_SAMPLE: &str = "sample harp from \"sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\" license cc0 else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~ | bass c2 ~ c2 ~ inst=harp\n";

fn cid_of(text: &str) -> String {
    let v: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
    v["cid"].as_str().unwrap().to_string()
}

// ---------- the source tree ----------

/// Every file under src/ that writes to disk (outside its tests), how many write calls it has, and what it writes.
/// Adding a write anywhere fails this test until it is listed here, with what it stores.
const WRITERS: &[(&str, usize, &str)] = &[
    ("gate.rs", 2, "programs/<cid>.json: the only program write (temp file + link(2)), after gate::admit; a BeatScript 1 program's BeatScript 2 twin goes through the same write"),
    ("migrate.rs", 5, "migrate-rename: the backup archive (outside the data dir, create_new, 0600), and the render cache's links and .meta copies for a twin (render/ only); the twins themselves go through gate::write_twin"),
    ("authsvc.rs", 5, "session.secret, tokens.json (sign-in): no programs"),
    ("cards.rs", 3, "the share-card PNG cache, keyed by an admitted stored program's CID: images, no program text"),
    ("compose/limits.rs", 5, "compose spend table and compose-<day>.jsonl (prompt hash and head, CID): no program text"),
    ("hub.rs", 2, "site.json: no programs"),
    ("hub/pair.rs", 3, "devices.json (paired phones' public keys): no programs"),
    ("log.rs", 5, "access-<day>.jsonl (method, path, status, size, hashed address): no bodies"),
    ("playlists/store.rs", 3, "playlists: names and CIDs of admitted programs, never their text"),
    ("public.rs", 2, "site.json: no programs"),
    ("service.rs", 10, "the render cache: audio of stored (admitted) programs and examples"),
    ("export_route.rs", 6, "the export cache: MIDI and stem zips (with manifest.json) of stored (admitted) programs and examples, like the render cache"),
    ("opendata.rs", 4, "dumps/: archives of the public CC0 programs already admitted to programs/ (gate.rs), and latest.json: no new program"),
    ("ontology/query.rs", 1, "the SPARQL worker's stdin (a pipe, not a file): nothing on disk"),
    ("beta.rs", 3, "beta-signups.json: Android beta requests (email, name, status): no programs"),
    ("caps.rs", 5, "cap-signing.key (the grant signing seed) and caps.json (capability grants): no programs"),
    ("oauth.rs", 3, "oauth.json: OAuth clients, codes and tokens (hashed): no programs"),
    ("diagnostics.rs", 2, "diagnostics/lexicon-harvest.jsonl: lyric words, their phonemes and program CIDs, never program text or a caller (docs/beatscript/lexicon-harvest.md)"),
];

const WRITE_CALLS: &[&str] = &["fs::write(", "File::create(", "hard_link(", ".create(true)", ".create_new(true)", "File::options()", "fs::rename(", "write_all("];

fn src_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            src_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// A file's lines outside `#[cfg(test)]` (a test module runs to the end of the file in this crate) and outside
/// `tests.rs` files.
fn live_lines(p: &Path) -> Vec<String> {
    if p.file_name().is_some_and(|n| n == "tests.rs") {
        return Vec::new();
    }
    let s = std::fs::read_to_string(p).unwrap();
    s.lines().take_while(|l| !l.trim_start().starts_with("#[cfg(test)]")).filter(|l| !l.trim_start().starts_with("//")).map(str::to_string).collect()
}

#[test]
fn every_write_site_is_known() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    src_files(&root, &mut files);
    let mut found: Vec<(String, usize)> = Vec::new();
    for p in &files {
        let rel = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let lines = live_lines(p);
        let n: usize = lines.iter().map(|l| WRITE_CALLS.iter().map(|w| l.matches(*w).count()).sum::<usize>()).sum();
        if n > 0 {
            found.push((rel.clone(), n));
        }
        let text = lines.join("\n");
        // a program is written by gate::write_program and nothing else
        if rel != "gate.rs" && rel != "migrate.rs" {
            assert!(!text.contains("hard_link("), "{rel}: link(2) is how a program is stored; only gate.rs may do it");
        }
        // migrate.rs links render files, never programs: its one link is in its render step
        if rel == "migrate.rs" {
            let at = lines.iter().position(|l| l.contains("hard_link(")).expect("migrate.rs links render files");
            assert_eq!(lines.iter().filter(|l| l.contains("hard_link(")).count(), 1);
            let within = lines[..at].iter().rev().find(|l| l.trim_start().starts_with("fn ") || l.trim_start().starts_with("pub fn "));
            assert!(within.is_some_and(|l| l.contains("fn link_renders(")), "migrate.rs: its link is the render step's (link_renders), not a program write");
        }
        if text.contains("write_program(") && rel != "gate.rs" {
            assert!(text.contains("gate::admit(") || text.contains("admit_as("), "{rel} calls gate::write_program without gate::admit");
        }
    }
    found.sort();
    let mut want: Vec<(String, usize)> = WRITERS.iter().map(|(f, n, _)| (f.to_string(), *n)).collect();
    want.sort();
    assert_eq!(found, want, "the files that write to disk changed: list the new write in WRITERS with what it stores (and gate it if it stores a program)");
    // the callers of the one writer: each program write path's module
    let callers: Vec<String> = files
        .iter()
        .filter(|p| live_lines(p).iter().any(|l| l.contains("gate::write_program(")))
        .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned())
        .collect();
    let mut callers = callers;
    callers.sort();
    assert_eq!(callers, ["cards.rs", "compose.rs", "store.rs"], "who stores programs changed");
}

/// The routes that take a body (or, for the render route, a program in the URL), from the source, each either a
/// program write path (tested below) or a route that stores no program.
#[test]
fn every_write_route_is_gated() {
    // program write paths: (route, where it is exercised)
    let gated = [
        "PUT /api/programs/{cid}",       // write_paths_refuse_what_is_not_cc
        "PUT /api/shares/{cid}",         // write_paths_refuse_what_is_not_cc (bearer; a phone signs the same route)
        "GET /api/render/{cid}",         // <cid>.<packed>: write_paths_refuse_what_is_not_cc
        "POST /mcp",                     // share, render, record, queue: write_paths_refuse_what_is_not_cc
        "POST /api/playlists",           // tests/playlists.rs items_must_name_admitted_programs
        "PUT /api/playlists/{id}",       // same
        "POST /api/playlists/{id}/items", // same
        "POST /api/compose",             // compose::put (gated; needs a model, so covered by the source test)
        "POST /api/device/compose",      // same
        "POST /api/generate",            // generate_and_store -> compose::put (gated), as /api/compose
    ];
    // routes with a body that store no program
    let not_programs = [
        "/api/check", "/api/fmt", "/api/share", "/api/open", "/api/qr", "/api/station", // answers only
        "/api/auth/", "/pair", "/api/pair", "/api/devices", "/api/device", "/api/session", // sign-in, pairing, the hub (memory)
        "/api/admin", "/api/playlists/{id}/share", "/api/playlists/{id}/items/{item}", "/api/playlists/p/{token}",
        "/api/beta/signup", "/api/beta/status", "/api/beta/remove", // the Android beta list (beta-signups.json): an email, no program
        "/api/lineage/{cid}", // reads stored programs' lineage, writes nothing
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    src_files(&root, &mut files);
    let mut routes = Vec::new();
    for p in &files {
        for l in live_lines(p) {
            // the route table: `.route("…", …)` and the `for p in ["…", …]` lists the hub and sign-in mount
            let t = l.trim_start();
            if !(t.starts_with(".route(") || t.starts_with("\"/") || t.starts_with("for p in [")) {
                continue;
            }
            for (i, _) in l.match_indices("\"/") {
                let rest = &l[i + 1..];
                let Some(end) = rest.find('"') else { continue };
                let r = &rest[..end];
                if r.starts_with("/api/") || r == "/mcp" || r == "/pair" {
                    routes.push(r.to_string());
                }
            }
        }
    }
    routes.sort();
    routes.dedup();
    assert!(routes.iter().any(|r| r == "/api/shares/{cid}") && routes.iter().any(|r| r == "/api/device/events"), "the route scan found {routes:?}");
    let mut unknown = Vec::new();
    for r in routes {
        let ok = gated.iter().any(|g| g.split(' ').nth(1) == Some(r.as_str()))
            || not_programs.iter().any(|n| r == *n || (n.ends_with('/') && r.starts_with(n)) || r.starts_with(&format!("{n}/")) || r == *n)
            || is_read_only(&r);
        if !ok {
            unknown.push(r);
        }
    }
    assert!(unknown.is_empty(), "routes neither gated program write paths nor known to store no program (add each to one list): {unknown:?}");
}

fn is_read_only(r: &str) -> bool {
    // GET-only routes (no body): the static API
    ["/api/info", "/api/instruments", "/api/examples", "/api/render/{cid}/status", "/api/shares/", "/api/programs/"].iter().any(|p| r == *p || r.starts_with(p))
}

// ---------- a live server ----------

struct Server {
    child: Child,
    addr: String,
    dir: tempfile::TempDir,
    out: Arc<Mutex<String>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn phone_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    std::fs::write(d.join("site.json"), json!({"site_id": SITE, "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    let tok = |t: &str, u: &str| json!({"hash": hex::encode(Sha256::digest(t.as_bytes())), "user": u, "created": "2026-11-19T00:00:00Z"});
    std::fs::write(d.join("tokens.json"), json!([tok(TOKEN_A, A), tok(TOKEN_B, B)]).to_string()).unwrap();
    let dev = |id: &str, owner: &str, k: &SigningKey| json!({"id": id, "owner": owner, "name": "Pixel 9", "key": B64.encode(k.verifying_key().to_bytes()), "paired": "2026-11-19T00:00:00Z"});
    std::fs::write(d.join("devices.json"), json!([dev(PHONE_A, A, &phone_key(1)), dev(PHONE_B, B, &phone_key(2))]).to_string()).unwrap();
    let (mut child, addr) = common::serve(
        |addr| common::flags(&["-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", &format!("http://{addr}"), "-web", "", "-docs", "", "-sse-ping-ms", "200"]),
        true,
    );
    let out = Arc::new(Mutex::new(String::new()));
    for pipe in [Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>, Box::new(child.stderr.take().unwrap())] {
        let out = out.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(pipe).lines().map_while(Result::ok) {
                out.lock().unwrap().push_str(&(l + "\n"));
            }
        });
    }
    Server { child, addr, dir, out }
}

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON ({}): {}", self.status, self.body))
    }
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
    fn call(&self, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> Resp {
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n", self.addr, body.len());
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(body).unwrap();
        let mut raw = String::new();
        let _ = s.read_to_string(&mut raw);
        let (head, b) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(b) } else { b.to_string() };
        Resp { status, body }
    }

    fn bearer(t: &str) -> (&'static str, String) {
        ("Authorization", format!("Bearer {t}"))
    }

    fn mcp(&self, token: &str, tool: &str, args: Value) -> (bool, Value) {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}).to_string();
        let r = self.call("POST", "/mcp", &[Server::bearer(token), ("Content-Type", "application/json".into()), ("Accept", "application/json, text/event-stream".into())], body.as_bytes());
        let mut v = r.json();
        if v.get("result").is_none() {
            // an SSE-framed answer: its data line
            let line = r.body.lines().find_map(|l| l.strip_prefix("data: ")).unwrap_or_else(|| panic!("no MCP answer: {}", r.body)).to_string();
            v = serde_json::from_str(&line).unwrap();
        }
        let res = &v["result"];
        let text = res["content"][0]["text"].as_str().unwrap_or("");
        (res["isError"] == true, serde_json::from_str(text).unwrap_or(json!(text)))
    }

    /// Open the phone's event stream (signed as a paired phone does); events arrive on the receiver.
    fn device_stream(&self, dev: &str, key: &SigningKey) -> (std::sync::mpsc::Receiver<String>, TcpStream) {
        let path = "/api/device/events";
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let msg = format!("btw-pair/2 request\nsite: {SITE}\ndevice: {dev}\nmethod: GET\npath: {path}\nts: {ts}\nbody: {}", hex::encode(Sha256::digest(b"")));
        let sig = B64.encode(key.sign(msg.as_bytes()).to_bytes());
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nAuthorization: BtwDevice {dev}.{ts}.{sig}\r\n\r\n", self.addr).as_bytes()).unwrap();
        let keep = s.try_clone().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(s).lines().map_while(Result::ok) {
                if let Some(d) = l.strip_prefix("data: ") {
                    if tx.send(d.to_string()).is_err() {
                        break;
                    }
                }
            }
        });
        (rx, keep)
    }

    /// Every byte the server wrote under its data directory, and everything it printed.
    fn everything_written(&self) -> String {
        fn walk(p: &Path, out: &mut String) {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                let q = e.path();
                if q.is_dir() {
                    walk(&q, out);
                } else if let Ok(b) = std::fs::read(&q) {
                    out.push_str(&String::from_utf8_lossy(&b));
                }
            }
        }
        let mut out = String::new();
        walk(self.dir.path(), &mut out);
        out.push_str(&self.out.lock().unwrap());
        out
    }
}

fn wait_for(rx: &std::sync::mpsc::Receiver<String>, typ: &str) -> Value {
    let t0 = Instant::now();
    loop {
        let left = Duration::from_secs(15).saturating_sub(t0.elapsed());
        let d = rx.recv_timeout(left).unwrap_or_else(|_| panic!("no {typ} event"));
        let v: Value = serde_json::from_str(&d).unwrap_or(Value::Null);
        if v["type"] == typ {
            return v;
        }
    }
}

fn wait_connected(s: &Server, token: &str, dev: &str) {
    let t0 = Instant::now();
    loop {
        let (_, v) = s.mcp(token, "devices", json!({}));
        if v["devices"].as_array().is_some_and(|l| l.iter().any(|d| d["id"] == dev && d["connected"] == true)) {
            return;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "the phone never showed as connected: {v}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn pack(text: &str) -> String {
    B64.encode(miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 9))
}

// ---------- the program write paths ----------

#[test]
fn write_paths_refuse_what_is_not_cc() {
    let s = start();
    let cc_only = beatscript_serve::gate::CC_ONLY;
    let text_plain = ("Content-Type", "text/plain".to_string());
    let programs = s.dir.path().join("programs");

    // PUT /api/programs/<cid> (anonymous): a plain program is stored CC0; samples and private are refused
    let r = s.call("PUT", &format!("/api/programs/{}", cid_of(PLAIN)), &[text_plain.clone()], PLAIN.as_bytes());
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.json()["meta"]["license"], "CC0-1.0");
    for t in [PRIVATE, OPEN_SAMPLE] {
        let r = s.call("PUT", &format!("/api/programs/{}", cid_of(t)), &[text_plain.clone()], t.as_bytes());
        assert_eq!((r.status, r.json()["error"]["code"].as_str().map(str::to_string)), (422, Some("restricted".into())), "{}", r.body);
    }

    // PUT /api/shares/<cid> (signed in): CC licences are stored as asked; anything else is refused with the gate's message
    let share = |text: &str, rights: Value| {
        let mut b = json!({"text": text});
        b.as_object_mut().unwrap().extend(rights.as_object().unwrap().clone());
        s.call("PUT", &format!("/api/shares/{}", cid_of(text)), &[Server::bearer(TOKEN_A), ("Content-Type", "application/json".into())], b.to_string().as_bytes())
    };
    let r = share(PLAIN2, json!({"license": "cc-by-4.0"}));
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.json()["meta"]["license"], "CC-BY-4.0");
    for lic in ["all-rights-reserved", "MIT", "LicenseRef-cover"] {
        let r = share(PLAIN3, json!({"license": lic}));
        assert_eq!(r.status, 422, "{lic}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], "not_creative_commons", "{lic}");
        assert!(r.json()["error"]["message"].as_str().unwrap().starts_with(cc_only), "{}", r.body);
    }
    assert_eq!(share(PLAIN3, json!({"visibility": "private"})).status, 422);
    assert_eq!(share(PRIVATE, json!({})).status, 422);
    assert!(!programs.join(format!("{}.json", cid_of(PLAIN3))).exists());

    // the render route's <cid>.<packed> store
    let r = s.call("GET", &format!("/api/render/{}.{}/status", cid_of(PRIVATE), pack(PRIVATE)), &[], b"");
    assert!(r.status == 422 || r.status == 404, "{} {}", r.status, r.body);
    let r = s.call("HEAD", &format!("/api/render/{}.{}?format=wav", cid_of(OPEN_SAMPLE), pack(OPEN_SAMPLE)), &[], b"");
    assert_eq!(r.status, 422, "{}", r.body);
    assert!(!programs.join(format!("{}.json", cid_of(OPEN_SAMPLE))).exists());

    // MCP share (signed in): stored only under a CC licence; the inline link is still given
    let (err, v) = s.mcp(TOKEN_A, "share", json!({"text": PLAIN4, "license": "all-rights-reserved"}));
    assert!(!err, "{v}");
    assert_eq!(v["stored"], false, "{v}");
    assert!(v["short_note"].as_str().unwrap().contains(cc_only), "{v}");
    assert!(v["url"].as_str().unwrap().contains('#'));
    assert!(!programs.join(format!("{}.json", cid_of(PLAIN4))).exists());
    let (err, v) = s.mcp(TOKEN_A, "share", json!({"text": PLAIN4, "license": "CC-BY-SA-4.0"}));
    assert!(!err && v["stored"] == true, "{v}");
    assert_eq!(v["meta"]["license"], "CC-BY-SA-4.0");
    // MCP render and record with text: a private program is not stored or rendered
    for tool in ["render", "record"] {
        let (err, v) = s.mcp(TOKEN_A, tool, json!({"text": PRIVATE}));
        assert!(err, "{tool}: {v}");
    }
    // MCP queue: an item must be an example or an admitted program (no session here: the gate answers first)
    std::fs::write(programs.join("zARR.json"), "{}").unwrap();

    // what is stored now, all admitted
    let bad = beatscript_serve::gate::audit_dir(&programs);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!(bad[0].0, "zARR.json");
    let n = std::fs::read_dir(&programs).unwrap().count();
    assert_eq!(n, 7, "PLAIN, PLAIN2, PLAIN4, their BeatScript 2 twins and the planted file");
}

// ---------- the private relay ----------

#[test]
fn private_push_reaches_the_phone_sealed_and_leaves_nothing() {
    let s = start();
    let (rx, _keep) = s.device_stream(PHONE_A, &phone_key(1));
    wait_for(&rx, "hello");
    wait_connected(&s, TOKEN_A, PHONE_A);
    // the devices tool gives the key a client seals for
    let (_, v) = s.mcp(TOKEN_A, "devices", json!({}));
    let key = v["devices"][0]["seal_key"].as_str().unwrap().to_string();
    assert_eq!(key, B64.encode(phone_key(1).verifying_key().to_bytes()));

    // sealed here (text)
    let (err, v) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "text": PRIVATE}));
    assert!(!err, "{v}");
    assert_eq!(v["sent"], true);
    let ev = wait_for(&rx, "private");
    assert!(ev.get("text").is_none() && ev.get("cid").is_none(), "{ev}");
    let envelope = ev["extra"].to_string();
    assert!(!envelope.contains("restricted") && !envelope.contains("2d711642"), "the envelope is ciphertext");
    let mut guard = beatscript_seal::ReplayGuard::default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let p = guard.open(&[1u8; 32], PHONE_A, &envelope, now).unwrap();
    assert_eq!((p.kind.as_str(), p.text.as_str(), p.play), ("eval", PRIVATE, true));
    // replayed: refused on the phone
    assert_eq!(guard.open(&[1u8; 32], PHONE_A, &envelope, now).unwrap_err(), beatscript_seal::Error::Replayed);
    // tampered in transit: refused on the phone
    let mut t: Value = serde_json::from_str(&envelope).unwrap();
    let mut ct = B64.decode(t["ct"].as_str().unwrap()).unwrap();
    ct[5] ^= 1;
    t["ct"] = json!(B64.encode(ct));
    assert_eq!(beatscript_seal::open(&[1u8; 32], PHONE_A, &t.to_string()).unwrap_err(), beatscript_seal::Error::Tampered);

    // sealed by the client (sealed): the server passes the envelope on as it is
    let pk: [u8; 32] = B64.decode(&key).unwrap().try_into().unwrap();
    let mine = beatscript_seal::seal_payload(&pk, PHONE_A, &beatscript_seal::Payload::new("load", PRIVATE, false, now).unwrap()).unwrap();
    let (err, v) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "sealed": mine}));
    assert!(!err, "{v}");
    let ev = wait_for(&rx, "private");
    assert_eq!(guard.open(&[1u8; 32], PHONE_A, &ev["extra"].to_string(), now).unwrap().kind, "load");
    // garbage is not passed on
    let (err, _) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "sealed": "{\"v\":1}"}));
    assert!(err);

    // nothing of it anywhere: data dir (programs, render cache, logs, devices, playlists) and the server's output
    std::thread::sleep(Duration::from_millis(1500)); // the access log flushes every second
    let all = s.everything_written();
    for needle in ["2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881", "use restricted", "bright-pluck", &cid_of(PRIVATE), t["epk"].as_str().unwrap(), &t["ct"].as_str().unwrap()[..40]] {
        assert!(!all.contains(needle), "{needle} was written somewhere");
    }
    assert!(all.contains(&format!("private push: {} bytes to device {PHONE_A}", envelope.len())), "the size line: {}", s.out.lock().unwrap());
    assert!(!s.dir.path().join("programs").exists() || std::fs::read_dir(s.dir.path().join("programs")).unwrap().count() == 0);
}

#[test]
fn private_push_only_to_your_own_connected_phone() {
    let s = start();
    let (rx, keep) = s.device_stream(PHONE_A, &phone_key(1));
    wait_for(&rx, "hello");
    wait_connected(&s, TOKEN_A, PHONE_A);
    // another account's phone: as if it did not exist
    let (err, v) = s.mcp(TOKEN_B, "push_private", json!({"device": PHONE_A, "text": PRIVATE}));
    assert!(err, "{v}");
    assert!(v.to_string().contains("not_found") || v.to_string().contains("no paired phone"), "{v}");
    // no such phone, no sign-in
    assert!(s.mcp(TOKEN_A, "push_private", json!({"device": "devNOPEdevNOPEdevNOPE0", "text": PRIVATE})).0);
    // a token that is not one of ours: since OAuth (oauth.rs) /mcp answers it with a 401 that points at the resource
    // metadata, before any tool runs
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "push_private", "arguments": {"device": PHONE_A, "text": PRIVATE}}}).to_string();
    let r = s.call("POST", "/mcp", &[Server::bearer("btw_notatokenofoursnotatokenofours0000000000"), ("Content-Type", "application/json".into()), ("Accept", "application/json, text/event-stream".into())], body.as_bytes());
    assert!(r.status == 401 && r.body.contains("invalid_token"), "{} {}", r.status, r.body);
    // B's own phone is paired but not connected: nothing sent, nothing kept
    let (err, v) = s.mcp(TOKEN_B, "push_private", json!({"device": PHONE_B, "text": PRIVATE}));
    assert!(err && v.to_string().contains("not connected"), "{v}");
    // A's phone hangs up: the next push is dropped, not queued
    drop(keep);
    drop(rx);
    let t0 = Instant::now();
    loop {
        let (err, v) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "text": PRIVATE}));
        if err {
            assert!(v.to_string().contains("not connected"), "{v}");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "pushes kept being accepted after the phone left");
        std::thread::sleep(Duration::from_millis(100));
    }
    let all = s.everything_written();
    assert!(!all.contains("use restricted"));
}

// ---------- programs arrive formatted ----------

/// A program as an agent might send it: a header, a comment, loose spacing, a trailing comment, a blank run.
const LOOSE: &str = "#! btw 1\n#! name: Loose Beat\n# a beat\ntempo   124   # steady\n\n\ndeck A:   kick  e(4,16) | snare ~ x ~ x\n";
/// What `fmt` prints for it (djlive::format): the same lines, tidy, with the header, the comments and one blank line kept.
const LOOSE_FORMATTED: &str = "#! btw 1\n#! name: Loose Beat\n# a beat\ntempo 124  # steady\n\ndeck A: kick e(4, 16) | snare ~ x ~ x\n";
/// What the CID is of: no comments.
const LOOSE_CANONICAL: &str = "tempo 124\ndeck A: kick e(4, 16) | snare ~ x ~ x\n";

#[test]
fn the_fixture_is_what_fmt_prints() {
    // what the hook sends (the formatter that keeps the version: nothing upgrades silently)
    assert_eq!(beatscript_api::format_text(LOOSE).as_deref(), Some(LOOSE_FORMATTED));
    assert_eq!(cid_of(LOOSE), cid_of(LOOSE_FORMATTED));
    assert_eq!(cid_of(LOOSE), cid_of(LOOSE_CANONICAL));
    // and the fmt tool's answer: the same program in BeatScript 2, naming the CID it came from
    let v: Value = serde_json::from_str(&beatscript_api::fmt_json(LOOSE)).unwrap();
    assert_eq!(v["text"].as_str(), Some(LOOSE_FORMATTED.replacen("#! btw 1", "#! beatscript 2", 1).as_str()));
    assert_eq!(v["canonical"].as_str().map(str::to_string), Some(djlive::version::upgrade_canonical(LOOSE_CANONICAL)));
    assert_eq!(v["from_cid"], cid_of(LOOSE));
}

#[test]
fn eval_load_and_the_phone_push_arrive_formatted() {
    let s = start();
    let (rx, _keep) = s.device_stream(PHONE_A, &phone_key(1));
    wait_for(&rx, "hello");
    wait_connected(&s, TOKEN_A, PHONE_A);

    // eval: the phone gets what fmt prints, with the CID of the text that was sent
    let (err, v) = s.mcp(TOKEN_A, "eval", json!({"device": PHONE_A, "text": LOOSE}));
    assert!(!err, "{v}");
    assert_eq!(v["cid"], cid_of(LOOSE));
    let ev = wait_for(&rx, "eval");
    assert_eq!(ev["text"], LOOSE_FORMATTED);
    assert_eq!(ev["cid"], cid_of(LOOSE));
    assert_eq!(cid_of(ev["text"].as_str().unwrap()), cid_of(LOOSE));

    // a program that does not read is refused as it always was (at the line and column the sender wrote), and sends nothing
    let (err, v) = s.mcp(TOKEN_A, "eval", json!({"device": PHONE_A, "text": "#! btw 1\ntempo 120  # fine\nkey\n"}));
    assert!(err, "{v}");
    assert!(v.to_string().contains("3:") || v.to_string().contains("\"line\":3"), "{v}");

    // load: a link's text (the canonical text, which formatting leaves as it is) is sent as it is
    let link = serde_json::from_str::<Value>(&beatscript_api::share_json(LOOSE, None, false, "")).unwrap()["url"].as_str().unwrap().to_string();
    let (err, v) = s.mcp(TOKEN_A, "load", json!({"device": PHONE_A, "url": link}));
    assert!(!err, "{v}");
    let ev = wait_for(&rx, "load");
    assert_eq!(ev["text"], LOOSE_CANONICAL);
    assert_eq!(ev["cid"], cid_of(LOOSE));

    // the phone push route (the web editor's phone button): eval and load both send the formatted text
    let post = |kind: &str| {
        let body = json!({"type": kind, "text": LOOSE}).to_string();
        s.call("POST", &format!("/api/devices/{PHONE_A}/push"), &[Server::bearer(TOKEN_A), ("Content-Type", "application/json".into())], body.as_bytes())
    };
    for kind in ["eval", "load"] {
        let r = post(kind);
        assert_eq!(r.status, 200, "{kind}: {}", r.body);
        assert_eq!(r.json()["cid"], cid_of(LOOSE));
        let ev = wait_for(&rx, kind);
        assert_eq!(ev["text"], LOOSE_FORMATTED, "{kind}");
    }
}

#[test]
fn push_private_text_is_formatted_before_it_is_sealed_and_sealed_text_is_not_touched() {
    let s = start();
    let (rx, _keep) = s.device_stream(PHONE_A, &phone_key(1));
    wait_for(&rx, "hello");
    wait_connected(&s, TOKEN_A, PHONE_A);
    let loose = "#! visibility: private\nuse restricted\nsample harp from \"sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\" license restricted-private else bright-pluck   # mine\ntempo   120  # slow\n\n\ndeck A: kick  x ~ x ~ | bass c2 ~ c2 ~ inst=harp\n";
    let tidy = "#! visibility: private\nuse restricted\nsample harp from \"sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\" license restricted-private else bright-pluck  # mine\ntempo 120  # slow\n\ndeck A: kick x ~ x ~ | bass c2 ~ c2 ~ inst=harp\n";
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let mut guard = beatscript_seal::ReplayGuard::default();
    let (err, v) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "text": loose}));
    assert!(!err, "{v}");
    let ev = wait_for(&rx, "private");
    let p = guard.open(&[1u8; 32], PHONE_A, &ev["extra"].to_string(), now).unwrap();
    assert_eq!(p.text, tidy, "the sample line stays as it was written, the rest is formatted, the header and comments are kept");
    // what does not read goes on as it is: the phone says why
    let (err, v) = s.mcp(TOKEN_A, "push_private", json!({"device": PHONE_A, "text": "tempo   120\ndeck A: kick [x ~\n", "load": true}));
    assert!(!err, "{v}");
    let p = guard.open(&[1u8; 32], PHONE_A, &wait_for(&rx, "private")["extra"].to_string(), now).unwrap();
    assert_eq!((p.kind.as_str(), p.text.as_str()), ("load", "tempo   120\ndeck A: kick [x ~\n"));
    // nothing of it is stored (the relay's promise stands)
    assert!(!s.dir.path().join("programs").exists() || std::fs::read_dir(s.dir.path().join("programs")).unwrap().count() == 0);
}

#[test]
fn share_stores_the_canonical_text_and_names_it_from_the_header() {
    let s = start();
    let (err, v) = s.mcp(TOKEN_A, "share", json!({"text": LOOSE}));
    assert!(!err, "{v}");
    assert_eq!(v["cid"], cid_of(LOOSE));
    assert_eq!(v["canonical"], LOOSE_CANONICAL);
    assert_eq!(v["stored"], true, "{v}");
    // the store keeps the canonical text (a stored text hashes to its CID); the header's name is what it is called
    assert_eq!(v["meta"]["name"], "Loose Beat", "{v}");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(s.dir.path().join("programs").join(format!("{}.json", cid_of(LOOSE)))).unwrap()).unwrap();
    assert_eq!(file["text"], LOOSE_CANONICAL);
    assert_eq!(file["meta"]["name"], "Loose Beat");
    // a name given as an argument still wins over the header
    let (_, v) = s.mcp(TOKEN_A, "share", json!({"text": "#! name: From Header\ntempo 90\ndeck A: kick x ~ ~ ~\n", "name": "Given"}));
    assert_eq!(v["meta"]["name"], "Given", "{v}");
    // the self-sealing link opens to the same program (the CID is the one of the text sent)
    let (_, o) = s.mcp(TOKEN_A, "open", json!({"url": link_of(&s, LOOSE)}));
    assert_eq!(o["cid"], cid_of(LOOSE));
}

fn link_of(s: &Server, text: &str) -> String {
    let (_, v) = s.mcp(TOKEN_A, "share", json!({"text": text}));
    v["url"].as_str().unwrap().to_string()
}

#[test]
fn a_queued_link_item_is_formatted_text_with_the_cid_of_the_link() {
    let s = start();
    let r = s.call("POST", "/api/session", &[], b"");
    assert_eq!(r.status, 200, "{}", r.body);
    let id = r.json()["id"].as_str().unwrap().to_string();
    let link = serde_json::from_str::<Value>(&beatscript_api::share_json(LOOSE, None, false, "")).unwrap()["url"].as_str().unwrap().to_string();
    let (err, v) = s.mcp(TOKEN_A, "queue", json!({"action": "add", "session": id, "items": [{"url": link}]}));
    assert!(!err, "{v}");
    assert_eq!(v["queue"]["up_next"][0]["cid"], cid_of(LOOSE));
    // the page asks for the next item, listening: the text it is given (held in memory only, never listed)
    let body = json!({"reason": "skip", "presence": {"visible": true, "focused": true, "audio": "running", "idle_ms": 1000}}).to_string();
    let r = s.call("POST", &format!("/api/session/{id}/queue/next"), &[("Content-Type", "application/json".into())], body.as_bytes());
    assert_eq!(r.status, 200, "{}", r.body);
    let item = &r.json()["item"];
    assert_eq!(item["text"], LOOSE_CANONICAL, "{}", r.body);
    assert_eq!(item["cid"], cid_of(LOOSE));
    // it is the canonical text, which is what the formatter leaves as it is
    assert_eq!(beatscript_serve::autoformat::autoformat(item["text"].as_str().unwrap(), 16384), item["text"].as_str().unwrap());
}

/// The save round trip of `# from:` lines (6670 item 4): each write path keeps the lines of the text as sent in
/// `meta.based_on` (the stored text is canonical: it has no comments), every read gives them back, and lineage counts each
/// saved program as vendoring the program the lines name.
#[test]
fn write_paths_keep_the_from_lines() {
    let s = start();
    let text_plain = ("Content-Type", "text/plain".to_string());
    // the source: a program kept here, whose drop three others wrote out (ejected) under `# from:` lines
    let source = "tempo 124\nkey am\nsection drop 4 bars {\n  deck A: kick x ~ x ~ | bass a1 ~ c2 ~\n}\narrange drop\n";
    let src = cid_of(source);
    assert_eq!(s.call("PUT", &format!("/api/programs/{src}"), &[text_plain.clone()], source.as_bytes()).status, 201);
    let line = format!("{src}.drop -> {src}");
    let vendored = |name: &str, notes: &str| format!("#! name: {name}\n# from: {line}\n# from: a comment that pins nothing\ntempo 124\nkey am\nsection drop 4 bars {{\n  # from: {line}\n  deck A: kick x ~ x ~ | bass {notes}\n}}\narrange drop\n");
    let want = json!([line]);

    // PUT /api/shares/<cid> (signed in)
    let by_share = vendored("by share", "a1 ~ c2 e2");
    let cid1 = cid_of(&by_share);
    let r = s.call("PUT", &format!("/api/shares/{cid1}"), &[Server::bearer(TOKEN_A), ("Content-Type", "application/json".into())], json!({"text": by_share}).to_string().as_bytes());
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.json()["meta"]["based_on"], want, "{}", r.body);
    // PUT /api/programs/<cid> (anonymous)
    let by_put = vendored("by put", "a1 ~ e2 c2");
    let cid2 = cid_of(&by_put);
    let r = s.call("PUT", &format!("/api/programs/{cid2}"), &[text_plain.clone()], by_put.as_bytes());
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.json()["meta"]["based_on"], want, "{}", r.body);
    // MCP share (signed in)
    let by_mcp = vendored("by mcp", "c2 ~ a1 e2");
    let (err, v) = s.mcp(TOKEN_A, "share", json!({"text": by_mcp}));
    assert!(!err && v["stored"] == true, "{v}");
    let cid3 = v["cid"].as_str().unwrap().to_string();
    assert_eq!(cid3, cid_of(&by_mcp));
    assert_eq!(v["meta"]["based_on"], want, "{v}");

    for cid in [&cid1, &cid2, &cid3] {
        // the file: the canonical text, and the lines beside it
        let f: Value = serde_json::from_slice(&std::fs::read(s.dir.path().join("programs").join(format!("{cid}.json"))).unwrap()).unwrap();
        assert!(!f["text"].as_str().unwrap().contains('#'), "{f}");
        assert_eq!(f["meta"]["based_on"], want, "{f}");
        // read back: the JSON, the `.beatscript` download (whose save keeps them again), MCP open
        let r = s.call("GET", &format!("/api/programs/{cid}"), &[], b"");
        assert_eq!(r.json()["meta"]["based_on"], want, "{}", r.body);
        let file = s.call("GET", &format!("/api/programs/{cid}.beatscript"), &[], b"").body;
        assert!(file.contains(&format!("\n# from: {line}\n")), "{file}");
        assert_eq!(cid_of(&file), *cid, "the download is the same program");
        let (_, o) = s.mcp(TOKEN_A, "open", json!({"url": cid}));
        assert_eq!(o["meta"]["based_on"], want, "{o}");
    }
    // stored once: the same notes saved again with no lines keep the first meta (the store is append-only)
    let canonical = s.call("GET", &format!("/api/programs/{cid1}"), &[], b"").json()["text"].as_str().unwrap().to_string();
    let r = s.call("PUT", &format!("/api/programs/{cid1}"), &[text_plain.clone()], canonical.as_bytes());
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["meta"]["based_on"], want, "{}", r.body);
    // the page's linked data says what it vendored, from the kept lines (a BeatScript 1 program's page is its twin's: /p/<old CID>
    // redirects there, and the twin keeps the lines too)
    let twin = djlive::compile::renamed_cid(&canonical);
    let page = s.call("GET", &format!("/p/{twin}"), &[("Accept", "text/html".into())], b"").body;
    assert!(page.contains(&format!(r#""name":"{src}.drop","resolves":"{src}","derivation":"vendored""#)), "no vendored isBasedOn in the page");
    // and lineage counts the three as vendoring the source (none of them calls or borrows it in its text)
    let t = s.call("GET", &format!("/api/lineage/{src}"), &[], b"").json();
    assert_eq!(t["programs"][0]["@id"], json!(src), "{t}");
    assert_eq!(t["programs"][0]["vendoredFrom"], 3, "{t}");
    assert!(t["programs"][0].get("referencedBy").is_none(), "{t}");
}
