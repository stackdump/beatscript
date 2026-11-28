//! The booth as a paired device (site/PAIRING.md "The booth", R1a), against the real `beatscript-serve` binary: a booth
//! pairs with a signed `kind: booth` claim, its stream's hello names its kind and the resolver, a set pushed to it
//! arrives as a `set` event with its members' CIDs, a set never goes to a phone, the resolver answers a stored public
//! program (immutable, its JSON-LD linked) and 404 for anything else, and no push path puts a private, restricted or
//! asset-using program into the site's store.
//!
//! `cargo test -p beatscript-serve --test booth_pairing`.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SITE: &str = "siteSITEsiteSITEsite01";
const ORIGIN: &str = "https://beatscript.app";
const SECRET: [u8; 32] = [42u8; 32];
const OWNER: &str = "ed25519:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

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

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode(SECRET) + "\n").unwrap();
    std::fs::write(d.join("site.json"), format!(r#"{{"site_id": "{SITE}", "created": "2026-11-19T00:00:00Z"}}"#)).unwrap();
    std::fs::create_dir_all(d.join("programs")).unwrap();
    let (child, addr) = common::serve(|_| common::flags(&["-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", ORIGIN, "-legacy-hosts", "btw.bitwrap.io", "-docs", ""]), false);
    Server { child, addr, dir }
}

fn b64u(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn cookie() -> String {
    let p = format!(r#"{{"u":"{OWNER}","e":2000000000}}"#);
    let mut m = Hmac::<Sha256>::new_from_slice(&SECRET).unwrap();
    m.update(format!("btw-session-v1\n{p}").as_bytes());
    format!("btw_session={}.{}", b64u(p.as_bytes()), b64u(&m.finalize().into_bytes()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// (status, lower-cased head, body).
fn http(s: &Server, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, String, Vec<u8>) {
    let mut c = TcpStream::connect(&s.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: beatscript.app\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.write_all(req.as_bytes()).unwrap();
    c.write_all(body).unwrap();
    let mut raw = Vec::new();
    let _ = c.read_to_end(&mut raw);
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..at]).to_ascii_lowercase();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let mut body = raw[at + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    (status, head, body)
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..i]).trim(), 16).unwrap_or(0);
        if n == 0 || b.len() < i + 2 + n {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[(i + 4 + n).min(b.len())..];
    }
    out
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(b)))
}

fn owner_post(s: &Server, path: &str, body: &Value) -> (u16, Value) {
    let ck = cookie();
    let b = body.to_string();
    let (st, _, rb) = http(s, "POST", path, &[("Cookie", &ck), ("Origin", ORIGIN), ("Content-Type", "application/json")], b.as_bytes());
    (st, if rb.is_empty() { Value::Null } else { json_of(&rb) })
}

/// A paired device: its key, and its id once paired.
struct Dev {
    k: SigningKey,
    id: String,
}

impl Dev {
    fn new(seed: u8) -> Dev {
        Dev { k: SigningKey::from_bytes(&[seed; 32]), id: String::new() }
    }
    fn key(&self) -> String {
        b64u(self.k.verifying_key().as_bytes())
    }
    /// A v2 claim; `kind` None sends none (a phone as phones always did), `signed_kind` is the line signed.
    fn claim(&self, pair_id: &str, token: &str, name: &str, kind: Option<&str>, signed_kind: Option<&str>, proto: &str) -> Vec<u8> {
        let ts = now_ms();
        let th = hex::encode(Sha256::digest(token.as_bytes()));
        let line = signed_kind.map(|k| format!("kind: {k}\n")).unwrap_or_default();
        let msg = if proto == "btw-pair/2" {
            format!("btw-pair/2 claim\nsite: {SITE}\npair: {pair_id}\ntoken: {th}\nkey: {}\nname: {name}\n{line}ts: {ts}", self.key())
        } else {
            format!("btw-pair/1 claim\nhost: beatscript.app\npair: {pair_id}\ntoken: {th}\nkey: {}\nname: {name}\nts: {ts}", self.key())
        };
        let mut v = json!({"proto": proto, "pair_id": pair_id, "token": token, "key": self.key(), "name": name, "ts": ts, "sig": b64u(&self.k.sign(msg.as_bytes()).to_bytes())});
        if let Some(k) = kind {
            v["kind"] = json!(k);
        }
        v.to_string().into_bytes()
    }
    fn auth(&self, method: &str, path: &str) -> String {
        let ts = now_ms();
        let bh = hex::encode(Sha256::digest(b""));
        let msg = format!("btw-pair/2 request\nsite: {SITE}\ndevice: {}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {bh}", self.id);
        format!("BtwDevice {}.{ts}.{}", self.id, b64u(&self.k.sign(msg.as_bytes()).to_bytes()))
    }
}

fn offer(s: &Server) -> (String, String) {
    let (st, o) = owner_post(s, "/api/pair", &json!({}));
    assert_eq!(st, 200, "{o}");
    let url = o["url"].as_str().unwrap();
    let frag = url.split('#').nth(1).unwrap();
    let (id, tok) = frag.split_once('.').unwrap();
    (id.to_string(), tok.to_string())
}

fn claim(s: &Server, body: &[u8]) -> (u16, Value) {
    let (st, _, b) = http(s, "POST", "/api/pair/claim", &[("Content-Type", "application/json")], body);
    (st, json_of(&b))
}

fn pair(s: &Server, d: &mut Dev, name: &str, kind: Option<&str>) -> Value {
    let (id, tok) = offer(s);
    let (st, v) = claim(s, &d.claim(&id, &tok, name, kind, kind.filter(|k| *k != "phone"), "btw-pair/2"));
    assert_eq!(st, 200, "{v}");
    d.id = v["device_id"].as_str().unwrap().to_string();
    v
}

/// An open event stream: its frames' JSON, as they arrive.
struct Stream {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Stream {
    fn open(s: &Server, d: &Dev) -> Stream {
        let mut c = TcpStream::connect(&s.addr).unwrap();
        let a = d.auth("GET", "/api/device/events");
        c.write_all(format!("GET /api/device/events HTTP/1.1\r\nHost: beatscript.app\r\nAuthorization: {a}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let buf = Arc::new(Mutex::new(Vec::new()));
        let b2 = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 65536];
            while let Ok(n) = c.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                b2.lock().unwrap().extend_from_slice(&chunk[..n]);
            }
        });
        Stream { buf }
    }
    /// The `data:` JSON of every event so far.
    fn events(&self) -> Vec<Value> {
        let raw = String::from_utf8_lossy(&self.buf.lock().unwrap()).to_string();
        raw.lines().filter_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:"))).filter_map(|j| serde_json::from_str(j.trim()).ok()).collect()
    }
    /// Wait for the first event whose type is `typ`.
    fn wait(&self, typ: &str) -> Value {
        let t0 = Instant::now();
        loop {
            if let Some(e) = self.events().into_iter().find(|e| e["type"] == typ) {
                return e;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no {typ} event; got {:?}", self.events());
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

/// The CID of `text` as the site's checker gives it, and the text stored under it (PUT /api/programs, anonymous).
fn store(s: &Server, text: &str) -> String {
    let (st, rep) = http(s, "POST", "/api/check", &[("Content-Type", "application/json")], json!({"text": text}).to_string().as_bytes()).pipe(|(a, _, b)| (a, json_of(&b)));
    assert_eq!(st, 200, "{rep}");
    let cid = rep["cid"].as_str().unwrap().to_string();
    let (st, _, b) = http(s, "PUT", &format!("/api/programs/{cid}"), &[("Content-Type", "text/plain")], text.as_bytes());
    assert!(st == 200 || st == 201, "PUT {cid}: {st} {}", String::from_utf8_lossy(&b));
    cid
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn program_files(s: &Server) -> usize {
    std::fs::read_dir(s.dir.path().join("programs")).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn a_booth_pairs_receives_a_set_and_resolves_its_members() {
    let s = start();

    // pairing: the signed kind line, refusals before any offer is spent
    let mut booth = Dev::new(7);
    let (id, tok) = offer(&s);
    let (st, v) = claim(&s, &booth.claim(&id, &tok, "Booth Mac", Some("toaster"), Some("toaster"), "btw-pair/2"));
    assert_eq!((st, v["error"]["code"].as_str()), (400, Some("bad_request")), "{v}");
    let (st, v) = claim(&s, &booth.claim(&id, &tok, "Booth Mac", Some("booth"), None, "btw-pair/1"));
    assert_eq!(st, 400, "a v1 claim cannot name a kind: {v}");
    let (st, v) = claim(&s, &booth.claim(&id, &tok, "Booth Mac", Some("booth"), None, "btw-pair/2"));
    assert_eq!((st, v["error"]["code"].as_str()), (401, Some("bad_signature")), "the kind is signed: {v}");
    let (st, v) = claim(&s, &booth.claim(&id, &tok, "Booth Mac", Some("booth"), Some("booth"), "btw-pair/2"));
    assert_eq!(st, 200, "{v}");
    assert_eq!((v["kind"].as_str(), v["programs"].as_str()), (Some("booth"), Some("https://beatscript.app/api/programs/{cid}")), "{v}");
    booth.id = v["device_id"].as_str().unwrap().to_string();
    let devices = std::fs::read_to_string(s.dir.path().join("devices.json")).unwrap();
    assert!(devices.contains("\"kind\": \"booth\""), "{devices}");

    // a phone pairs as it always did: no kind sent, none written, kind phone in every view
    let mut phone = Dev::new(9);
    let pv = pair(&s, &mut phone, "Pixel 9", None);
    assert_eq!(pv["kind"], "phone");
    let devices = std::fs::read_to_string(s.dir.path().join("devices.json")).unwrap();
    assert_eq!(devices.matches("\"kind\"").count(), 1, "only the booth's record names a kind: {devices}");
    let ck = cookie();
    let (_, _, b) = http(&s, "GET", "/api/devices", &[("Cookie", &ck)], b"");
    let list = json_of(&b);
    let kinds: Vec<(String, String)> = list["devices"].as_array().unwrap().iter().map(|d| (d["name"].as_str().unwrap().into(), d["kind"].as_str().unwrap().into())).collect();
    assert!(kinds.contains(&("Booth Mac".into(), "booth".into())) && kinds.contains(&("Pixel 9".into(), "phone".into())), "{kinds:?}");

    // the stream: hello names the kind and the resolver
    let bs = Stream::open(&s, &booth);
    let hello = bs.wait("hello");
    let h = if hello.get("extra").is_some() && hello["extra"].is_object() { hello["extra"].clone() } else { hello.clone() };
    let hs = h.to_string();
    assert!(hs.contains("\"kind\":\"booth\"") && hs.contains("/api/programs/{cid}"), "{hello}");
    let ps = Stream::open(&s, &phone);
    ps.wait("hello");

    // two public members stored on the site, and a set of them
    let a = store(&s, "#! beatscript 2\ntempo 124\ndeck A: kick x ~ x ~\n");
    let b = store(&s, "#! beatscript 2\ntempo 126\ndeck A: kick x x ~ x\n");
    let set = format!("#! beatscript 2\nset {{\n  play {a} until bar 16\n  mix 8 bars with crossfade\n  play {b} until bar 16\n  play {a} until bar 8\n}}\n");
    let before = program_files(&s);
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", booth.id), &json!({"type": "set", "text": set, "play": true}));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["members"], json!([a, b]), "each member once, in order: {v}");
    let ev = bs.wait("set");
    assert_eq!((ev["cid"].as_str(), ev["play"].as_bool()), (v["cid"].as_str(), Some(true)), "{ev}");
    assert_eq!(ev["extra"]["members"], json!([{"cid": a, "n": 1, "set": false}, {"cid": b, "n": 2, "set": false}]), "{ev}");
    assert!(ev["text"].as_str().unwrap().contains("set {"), "{ev}");
    assert_eq!(program_files(&s), before, "a push stores nothing");

    // eval of a set to a booth arrives as a set; to a phone a set push is refused, and an eval of it stays an eval
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", booth.id), &json!({"type": "eval", "text": set}));
    assert_eq!(st, 200, "{v}");
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", phone.id), &json!({"type": "set", "text": set}));
    assert_eq!((st, v["error"]["code"].as_str()), (422, Some("not_a_booth")), "{v}");
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", booth.id), &json!({"type": "set", "text": "#! beatscript 2\ntempo 120\ndeck A: kick x ~ x ~\n"}));
    assert_eq!((st, v["error"]["code"].as_str()), (422, Some("not_a_set")), "{v}");
    let (st, _) = owner_post(&s, &format!("/api/devices/{}/push", phone.id), &json!({"type": "eval", "text": set}));
    assert_eq!(st, 200);
    let pe = ps.wait("eval");
    assert!(pe.get("extra").is_none() || pe["extra"].is_null(), "a phone gets a plain eval: {pe}");

    // a member this site does not hold: the set does not check here, nothing is sent (send it sealed)
    let absent = "z".to_string() + &"Q".repeat(40);
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", booth.id), &json!({"type": "set", "text": format!("#! beatscript 2\nset {{\n  play {absent} until bar 8\n}}\n")}));
    assert_eq!(st, 422, "{v}");

    // the gate: a restricted set is refused, and nothing private reaches the store
    let restricted = format!("#! beatscript 2\nuse restricted\nset {{\n  play {a} until bar 8\n}}\n");
    let (st, v) = owner_post(&s, &format!("/api/devices/{}/push", booth.id), &json!({"type": "set", "text": restricted}));
    assert_eq!(st, 422, "{v}");
    assert_eq!(program_files(&s), before, "a refused set is not stored");

    // the resolver: a stored public program, immutable, with its linked data; 404 for anything else
    let (st, head, body) = http(&s, "GET", &format!("/api/programs/{a}"), &[], b"");
    assert_eq!(st, 200, "{head}");
    let p = json_of(&body);
    assert_eq!((p["cid"].as_str(), p["meta"]["visibility"].as_str()), (Some(a.as_str()), Some("public")));
    assert!(head.contains("cache-control: public, max-age=31536000, immutable"), "{head}");
    assert!(head.contains(&format!("<{ORIGIN}/p/{a}.jsonld>; rel=\"describedby\"; type=\"application/ld+json\"").to_ascii_lowercase()), "{head}");
    assert!(head.contains("rel=\"privacy-policy\"") && head.contains("rel=\"license\""), "the middleware's links are kept: {head}");
    let (st, _, _) = http(&s, "GET", &format!("/api/programs/{a}"), &[("If-None-Match", &format!("\"{a}\""))], b"");
    assert_eq!(st, 304);
    let (st, head, _) = http(&s, "GET", &format!("/api/programs/{absent}"), &[], b"");
    assert_eq!(st, 404, "{head}");
    assert!(!head.contains("immutable"), "a miss is not cached as immutable: {head}");
    let (st, _, _) = http(&s, "GET", "/api/programs/notacid", &[], b"");
    assert_eq!(st, 404);
    let (_, _, wk) = http(&s, "GET", "/.well-known/btw-site", &[], b"");
    assert_eq!(json_of(&wk)["programs"], "https://beatscript.app/api/programs/{cid}");
}
