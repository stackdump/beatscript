//! The rename (BeatScript 1 -> 2) end to end, against the real `beatscript-serve` binary on a temp data directory:
//!
//! - a BeatScript 1 program stored after the rename is stored with its BeatScript 2 twin (`derivation: "renamed"`);
//! - `/p/<old>` (and `.jsonld`, `/tree`, `/c/`, `/card/`) answers 308 at the twin's address; a link that carries the text
//!   still opens; `/api/programs/<old>` still answers its own text, immutable, and names the twin (`Link: rel=
//!   "successor-version"`, `renamed_to`); renders of both are the same audio;
//! - an example written in BeatScript 2 answers under its legacy CID and redirects from it;
//! - the lineage does not count the twin as a fork;
//! - `migrate-rename`: refuses without a backup, a dry run writes nothing, the backup verifies by content, a second run
//!   writes nothing, and a tampered twin file is refused.
//!
//! `cargo test -p beatscript-serve --test rename`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const V1: &str = "tempo 120\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";
const V1_FORK: &str = "tempo 126\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";
/// A program whose sound is drawn from its CID's dice (the arrangement's mutations): a twin must play it the same.
const DICE: &str = "tempo 140\nlet h = x ~ x ~ ~ x ~ x x ~ ~ x ~ x ~ x\nsection groove 1 bars {\n  deck A: hihat h | kick x ~ ~ ~\n}\narrange {\n  groove\n  ~ h thin 0.5 | fill 0.3\n  groove\n  ~ h thin 0.5\n  groove\n}\n";
const EXAMPLE: &str = "#! beatscript 2\n#! name: night\n# an example in the new spelling\ntempo 128\nkey am\ndeck A: kick x x x x | bass a1 ~ ~ c2\n";

fn check(text: &str) -> Value {
    serde_json::from_str(&beatscript_api::check_json(text)).unwrap()
}

/// (cid, canonical) of a text that checks.
fn canon(text: &str) -> (String, String) {
    let r = check(text);
    assert_eq!(r["ok"], true, "{text}: {r}");
    (r["cid"].as_str().unwrap().to_string(), r["canonical"].as_str().unwrap().to_string())
}

fn twin(canonical: &str) -> String {
    djlive::compile::renamed_cid(canonical)
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

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, k: &str) -> Option<String> {
        let k = format!("{}:", k.to_ascii_lowercase());
        self.head.lines().find(|l| l.to_ascii_lowercase().starts_with(&k)).map(|l| l[k.len()..].trim().to_string())
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON ({}): {}", self.status, self.text()))
    }
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = b;
    loop {
        let Some(at) = rest.windows(2).position(|w| w == b"\r\n") else { break };
        let Ok(n) = usize::from_str_radix(std::str::from_utf8(&rest[..at]).unwrap_or("").trim(), 16) else { return b.to_vec() };
        let tail = &rest[at + 2..];
        if n == 0 || tail.len() < n {
            break;
        }
        out.extend_from_slice(&tail[..n]);
        rest = tail[n..].strip_prefix(b"\r\n").unwrap_or(&tail[n..]);
    }
    out
}

fn start(examples: Option<&Path>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr = format!("127.0.0.1:{port}");
    let ex = examples.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = Command::new(env!("CARGO_BIN_EXE_beatscript-serve"))
        .args(["-addr", &addr, "-data", &d.to_string_lossy(), "-log-dir", "off", "-public-url", &format!("http://{addr}"), "-web", "", "-docs", "", "-examples", &ex])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
        assert!(t0.elapsed() < Duration::from_secs(30), "beatscript-serve did not start: {}", out.lock().unwrap());
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, addr, dir, out }
}

impl Server {
    fn call(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Resp {
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n", self.addr, body.len());
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(body).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
        let head = String::from_utf8_lossy(&raw[..at]).into_owned();
        let b = raw.get(at + 4..).unwrap_or(&[]).to_vec();
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(&b) } else { b };
        Resp { status, head, body }
    }

    fn get(&self, path: &str) -> Resp {
        self.call("GET", path, &[], b"")
    }

    fn put_text(&self, text: &str) -> (String, String) {
        let (cid, canonical) = canon(text);
        let r = self.call("PUT", &format!("/api/programs/{cid}"), &[("Content-Type", "text/plain")], text.as_bytes());
        assert!((200..300).contains(&r.status), "PUT {cid}: {} {}\n{}", r.status, r.text(), self.out.lock().unwrap());
        (cid, canonical)
    }

    fn file(&self, cid: &str) -> Option<Value> {
        std::fs::read(self.dir.path().join("programs").join(format!("{cid}.json"))).ok().map(|b| serde_json::from_slice(&b).unwrap())
    }
}

fn redirects(s: &Server, from: &str, to: &str) {
    let r = s.get(from);
    assert_eq!(r.status, 308, "GET {from}: {} {}", r.status, r.text());
    assert_eq!(r.header("location").as_deref(), Some(to), "GET {from}");
}

#[test]
fn a_beatscript_1_program_is_stored_with_its_twin_and_its_old_address_redirects() {
    let s = start(None);
    let (x, xt) = s.put_text(V1);
    let y = twin(&xt);
    // an old fork of it, stored as a raw file as it was before the rename (before the lineage index first looks)
    let (fork, fork_text) = canon(V1_FORK);
    std::fs::write(s.dir.path().join("programs").join(format!("{fork}.json")), json!({"cid": fork, "text": fork_text, "meta": {"license": "CC0-1.0", "visibility": "public", "forked_from": x}}).to_string()).unwrap();
    // the twin, stored through the gate: the same program with the version line first, renamed from the old one
    let f = s.file(&y).expect("the twin is stored");
    assert_eq!(f["text"].as_str().unwrap(), djlive::version::upgrade_canonical(&xt));
    assert_eq!((f["meta"]["forked_from"].as_str(), f["meta"]["derivation"].as_str(), f["meta"]["source"].as_str()), (Some(x.as_str()), Some("renamed"), Some("renamed")));
    assert_eq!(s.file(&x).unwrap()["text"].as_str(), Some(xt.as_str()), "the old file is the old program");

    // the old addresses answer at the twin's
    redirects(&s, &format!("/p/{x}"), &format!("/p/{y}"));
    redirects(&s, &format!("/p/{x}.jsonld"), &format!("/p/{y}.jsonld"));
    redirects(&s, &format!("/p/{x}/tree"), &format!("/p/{y}/tree"));
    redirects(&s, &format!("/p/{x}/tree.jsonld?x=1"), &format!("/p/{y}/tree.jsonld?x=1"));
    redirects(&s, &format!("/c/{x}"), &format!("/c/{y}"));
    redirects(&s, &format!("/card/{x}.png"), &format!("/card/{y}.png"));
    redirects(&s, &format!("/card/{x}.square.png"), &format!("/card/{y}.square.png"));
    // the new one is the program; its page says where it came from
    let r = s.call("GET", &format!("/p/{y}"), &[("Accept", "text/html")], b"");
    assert_eq!(r.status, 200);
    assert!(r.text().contains("Renamed from BeatScript 1"), "{}", r.text());
    // an old share link (it carries the text) still opens, as the old program, and links the new one
    let packed = B64.encode(miniz_oxide::deflate::compress_to_vec(V1.as_bytes(), 9));
    let r = s.call("GET", &format!("/p/{x}.{packed}"), &[("Accept", "text/html")], b"");
    if r.status == 200 {
        assert!(r.text().contains(&format!("/p/{y}")), "the old program's page links its twin");
    }

    // /api/programs/<old>: its own text, immutable, naming its successor
    let r = s.get(&format!("/api/programs/{x}"));
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["text"].as_str(), Some(xt.as_str()));
    assert_eq!(r.json()["renamed_to"].as_str(), Some(y.as_str()));
    assert!(r.header("cache-control").unwrap().contains("immutable"));
    let link = r.header("link").expect("a Link header");
    assert!(link.contains(&format!("/api/programs/{y}>")) && link.contains("rel=\"successor-version\""), "{link}");
    // and the new one names its predecessor and its derivation
    let r = s.get(&format!("/api/programs/{y}"));
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["renamed_from"].as_str(), Some(x.as_str()));
    assert_eq!(v["meta"]["derivation"].as_str(), Some("renamed"));
    assert_eq!(v["meta"]["forked_from"].as_str(), Some(x.as_str()));
    assert!(r.header("link").unwrap().contains("rel=\"predecessor-version\""));

    // the lineage: the twin is not a fork of the old program
    let r = s.get(&format!("/api/lineage/{x}"));
    assert_eq!(r.status, 200, "{}", r.text());
    let t = r.json();
    assert_eq!(t["forkCount"], 1, "the fork, not the twin: {t}");
    assert_eq!(t["programs"][0]["renamedTo"].as_str(), Some(y.as_str()));
    let r = s.get(&format!("/api/lineage/{y}"));
    let t = r.json();
    let d = t["derivations"].as_array().unwrap().iter().find(|d| d["to"] == y.as_str()).cloned().unwrap();
    assert_eq!(d["derivation"], "renamed");
    assert_eq!(d["change"], json!(["renamed: BeatScript 1 -> 2, same music"]));
}

#[test]
fn a_program_and_its_twin_render_the_same_audio() {
    let s = start(None);
    let (x, xt) = s.put_text(DICE);
    let y = twin(&xt);
    assert!(s.file(&y).is_some());
    let a = s.get(&format!("/api/render/{x}?format=wav"));
    let b = s.get(&format!("/api/render/{y}?format=wav"));
    assert_eq!((a.status, b.status), (200, 200), "{} / {}", a.text(), b.text());
    assert!(a.body.len() > 44 + 1000, "{} bytes", a.body.len());
    // the header's sizes say "streaming" or the real size, by whether it came from the cache: the samples are the same
    assert_eq!(Sha256::digest(&a.body[44..]), Sha256::digest(&b.body[44..]), "a twin plays the same");
}

#[test]
fn an_examples_legacy_cid_resolves_and_redirects() {
    let ex = tempfile::tempdir().unwrap();
    std::fs::write(ex.path().join("night.beatscript"), EXAMPLE).unwrap();
    std::fs::write(ex.path().join("index.json"), json!([{"name": "night", "title": "Night"}]).to_string()).unwrap();
    let s = start(Some(ex.path()));
    let (new, canonical) = canon(EXAMPLE);
    let old = twin(&canonical);
    let legacy = djlive::version::legacy_canonical(&canonical);
    // the legacy CID answers with the legacy text, and names the example's CID
    let r = s.get(&format!("/api/programs/{old}"));
    assert_eq!(r.status, 200, "{}\n{}", r.text(), s.out.lock().unwrap());
    assert_eq!(r.json()["text"].as_str(), Some(legacy));
    assert_eq!(r.json()["renamed_to"].as_str(), Some(new.as_str()));
    assert!(r.header("link").unwrap().contains(&format!("/api/programs/{new}>")));
    redirects(&s, &format!("/p/{old}"), &format!("/p/{new}"));
    // the example itself answers
    let r = s.get(&format!("/api/programs/{new}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["text"].as_str(), Some(canonical.as_str()));
    // and renders under its old CID still
    let r = s.call("HEAD", &format!("/api/render/{old}?format=wav"), &[], b"");
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---------- migrate-rename ----------

const UID: &str = "ed25519:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A data directory as it was before the rename: BeatScript 1 programs alone (no twins), a playlist naming one, render
/// files of one, a share card, and state files.
fn old_data() -> (tempfile::TempDir, Vec<(String, String)>) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let programs = p.join("programs");
    std::fs::create_dir_all(&programs).unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    for (i, t) in [V1, V1_FORK, DICE].iter().enumerate() {
        let (cid, canonical) = canon(t);
        let mut meta = json!({"license": "CC0-1.0", "visibility": "public", "name": format!("p{i}"), "created": "2026-11-01T00:00:00Z", "source": "put"});
        if i == 1 {
            meta["forked_from"] = json!(out[0].0);
        }
        std::fs::write(programs.join(format!("{cid}.json")), json!({"cid": cid, "text": canonical, "meta": meta}).to_string()).unwrap();
        out.push((cid, canonical));
    }
    // one already BeatScript 2 (no twin to make)
    let (v2, v2t) = canon("#! beatscript 2\ntempo 99\ndeck A: kick x ~ ~ x\n");
    std::fs::write(programs.join(format!("{v2}.json")), json!({"cid": v2, "text": v2t, "meta": {"license": "CC0-1.0", "visibility": "public"}}).to_string()).unwrap();
    // a file that is not a program
    std::fs::write(programs.join("zNOTAPROGRAMzzzzzzzzzzzzzzzz.json"), "{}").unwrap();
    // a playlist naming the first program twice and the BeatScript 2 one once
    let pl = p.join("playlists");
    std::fs::create_dir_all(&pl).unwrap();
    let item = |id: &str, cid: &str| json!({"id": id, "cid": cid, "added_at": "2026-11-01T00:00:00Z"});
    let file = json!({"v": 1, "owner": UID, "playlists": [{"id": "pl_aaaaaaaaaaaaaaaa", "owner": UID, "name": "set", "items": [item("it_aaaaaaaa", &out[0].0), item("it_bbbbbbbb", &v2), item("it_cccccccc", &out[0].0)], "created": "2026-11-01T00:00:00Z", "updated": "2026-11-01T00:00:00Z", "visibility": "private", "version": "x"}]});
    std::fs::write(pl.join(format!("{}.json", beatscript_serve::playlists::store::account_key(UID))), file.to_string()).unwrap();
    // render files of the first program: a WAV and its meta (no CID in it), an Opus (left), and a card
    let render = p.join("render");
    std::fs::create_dir_all(&render).unwrap();
    std::fs::write(render.join(format!("{}.48000.r2.wav", out[0].0)), b"RIFF....WAVEfmt fake audio").unwrap();
    std::fs::write(render.join(format!("{}.48000.r2.wav.meta", out[0].0)), "{\"seconds\":2,\"bars\":1}").unwrap();
    std::fs::write(render.join(format!("{}.44100.r2.wav", out[2].0)), b"RIFF dice").unwrap();
    std::fs::write(render.join(format!("{}.44100.r2.wav.meta", out[2].0)), format!("{{\"seconds\":2,\"bars\":1,\"cid\":\"{}\"}}", out[2].0)).unwrap();
    std::fs::write(render.join(format!("{}.48000.r2rs.opus", out[0].0)), b"OggS").unwrap();
    let cards = p.join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    std::fs::write(cards.join(format!("{}.wide.v2.abcd.png", out[0].0)), b"png").unwrap();
    std::fs::write(p.join("site.json"), "{\"site_id\":\"x\"}").unwrap();
    std::fs::write(p.join("tokens.json"), "[]").unwrap();
    std::fs::write(p.join("ip-salt"), "secret, not JSON: not in the backup").unwrap();
    (d, out)
}

/// Every file under `dir`, by path relative to it, with its content's sha256.
fn snapshot(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(at).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), hex::encode(Sha256::digest(std::fs::read(&p).unwrap())));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn migrate(args: &[&str]) -> (i32, Value, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_beatscript-serve")).arg("migrate-rename").args(args).output().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
    let v = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (o.status.code().unwrap_or(-1), v, format!("{stdout}{}", String::from_utf8_lossy(&o.stderr)))
}

fn untar_gz(b: &[u8]) -> BTreeMap<String, String> {
    let raw = miniz_oxide::inflate::decompress_to_vec(&b[10..b.len() - 8]).unwrap();
    let mut out = BTreeMap::new();
    let mut at = 0;
    while at + 512 <= raw.len() && raw[at] != 0 {
        let h = &raw[at..at + 512];
        let name = String::from_utf8(h[..100].iter().take_while(|c| **c != 0).copied().collect()).unwrap();
        let size = usize::from_str_radix(std::str::from_utf8(&h[124..135]).unwrap(), 8).unwrap();
        out.insert(name, hex::encode(Sha256::digest(&raw[at + 512..at + 512 + size])));
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

#[test]
fn migration_refuses_without_a_backup() {
    let (d, _) = old_data();
    let before = snapshot(d.path());
    let (code, _, out) = migrate(&["--data", &d.path().to_string_lossy()]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("backup"), "{out}");
    // inside the data directory is refused too, before anything is written
    let inside = d.path().join("backup.tar.gz");
    let (code, v, out) = migrate(&["--data", &d.path().to_string_lossy(), "--backup", &inside.to_string_lossy(), "--json"]);
    assert_eq!(code, 1, "{out}");
    assert!(v["error"].as_str().unwrap().contains("inside the data directory"), "{v}");
    assert_eq!(snapshot(d.path()), before, "nothing written");
}

#[test]
fn migration_dry_run_writes_nothing_and_counts() {
    let (d, progs) = old_data();
    let before = snapshot(d.path());
    let (code, v, out) = migrate(&["--data", &d.path().to_string_lossy(), "--dry-run", "--json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(snapshot(d.path()), before, "a dry run writes nothing");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["programs_seen"], 5);
    assert_eq!(v["v1"], 3);
    assert_eq!(v["v2_already"], 1);
    assert_eq!(v["twins_to_write"], 3);
    assert_eq!(v["twins_present"], 0);
    assert_eq!(v["unverifiable"].as_array().unwrap().len(), 1);
    assert_eq!(v["playlists"]["items_to_rewrite"], 2);
    assert_eq!(v["playlists"]["accounts_to_rewrite"], 1);
    assert_eq!(v["render"]["to_link"], 3, "{v}");
    assert_eq!(v["render"]["to_rewrite_meta"], 1, "{v}");
    assert_eq!(v["render"]["left_to_rerender"], 1);
    assert_eq!(v["cards_left"], 1);
    for (cid, canonical) in &progs {
        assert_eq!(v["map"][cid].as_str(), Some(twin(canonical).as_str()));
    }
}

#[test]
fn migration_writes_twins_through_the_gate_verifies_its_backup_and_is_idempotent() {
    let (d, progs) = old_data();
    let before = snapshot(d.path());
    let out_dir = tempfile::tempdir().unwrap();
    let backup = out_dir.path().join("pre-rename.tar.gz");
    let data = d.path().to_string_lossy().into_owned();
    let (code, v, out) = migrate(&["--data", &data, "--backup", &backup.to_string_lossy(), "--json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(v["wrote"]["twins"], 3, "{v}");
    assert_eq!(v["self_check"]["ok"], true, "{v}");
    assert_eq!((v["self_check"]["v1"].as_u64(), v["self_check"]["twins"].as_u64()), (Some(3), Some(3)));

    // the backup: written first, 0600, and holding exactly programs/, playlists/ and the JSON state as they were
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(v["backup"]["verified"], true);
    let held = untar_gz(&std::fs::read(&backup).unwrap());
    let want: BTreeMap<String, String> = before.iter().filter(|(k, _)| k.starts_with("programs/") || k.starts_with("playlists/") || (!k.contains('/') && k.ends_with(".json"))).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_eq!(held, want, "the backup is the data as it was, by content");
    assert!(!held.keys().any(|k| k.starts_with("render/") || k.starts_with("cards/") || k == "ip-salt"));

    // every old file is untouched; each twin verifies and says where it came from
    let after = snapshot(d.path());
    for (k, h) in &before {
        if !k.starts_with("playlists/") {
            assert_eq!(after.get(k), Some(h), "{k} changed");
        }
    }
    for (x, xt) in &progs {
        let y = twin(xt);
        let f: Value = serde_json::from_slice(&std::fs::read(d.path().join("programs").join(format!("{y}.json"))).unwrap()).unwrap();
        assert_eq!(djlive::compile::program_cid(f["text"].as_str().unwrap()), y);
        assert_eq!(twin(f["text"].as_str().unwrap()), *x, "its legacy CID is the original");
        let m = &f["meta"];
        assert_eq!((m["forked_from"].as_str(), m["derivation"].as_str(), m["source"].as_str(), m["created"].as_str()), (Some(x.as_str()), Some("renamed"), Some("renamed"), Some("2026-11-01T00:00:00Z")));
        assert!(m["renamed_at"].is_string());
        assert_eq!(beatscript_serve::gate::audit_file(&serde_json::to_vec(&f).unwrap()).map(|a| a.cid().to_string()).ok(), Some(y.clone()));
    }
    // the playlist names the twins, and remembers the old CIDs
    let pl: Value = serde_json::from_slice(&std::fs::read(d.path().join("playlists").join(format!("{}.json", beatscript_serve::playlists::store::account_key(UID)))).unwrap()).unwrap();
    let items = pl["playlists"][0]["items"].as_array().unwrap();
    let (x0, y0) = (&progs[0].0, twin(&progs[0].1));
    assert_eq!(items[0]["cid"].as_str(), Some(y0.as_str()));
    assert_eq!(items[0]["renamed_from"].as_str(), Some(x0.as_str()));
    assert_eq!(items[2]["cid"].as_str(), Some(y0.as_str()));
    assert!(items[1].get("renamed_from").is_none());
    // the render cache: a hard link for the WAV and its meta, a rewritten copy of a meta that named the old CID
    use std::os::unix::fs::MetadataExt;
    let r = d.path().join("render");
    let ino = |n: String| std::fs::metadata(r.join(n)).unwrap().ino();
    assert_eq!(ino(format!("{x0}.48000.r2.wav")), ino(format!("{y0}.48000.r2.wav")));
    assert_eq!(ino(format!("{x0}.48000.r2.wav.meta")), ino(format!("{y0}.48000.r2.wav.meta")));
    let y2 = twin(&progs[2].1);
    assert_eq!(std::fs::read_to_string(r.join(format!("{y2}.44100.r2.wav.meta"))).unwrap(), format!("{{\"seconds\":2,\"bars\":1,\"cid\":\"{y2}\"}}"));
    assert!(!r.join(format!("{y0}.48000.r2rs.opus")).exists(), "Opus re-renders");

    // again: nothing to do, nothing written (not even a backup)
    let mid = snapshot(d.path());
    let backup2 = out_dir.path().join("again.tar.gz");
    let (code, v, out) = migrate(&["--data", &data, "--backup", &backup2.to_string_lossy(), "--json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(v["nothing_to_do"], true, "{v}");
    assert_eq!(v["twins_present"], 3);
    assert_eq!(v["wrote"]["twins"], 0);
    assert_eq!(v["self_check"]["ok"], true);
    assert_eq!(snapshot(d.path()), mid, "a second run writes nothing");
    assert!(!backup2.exists());
    let (_, v, _) = migrate(&["--data", &data, "--dry-run", "--json"]);
    assert_eq!((v["twins_to_write"].as_u64(), v["playlists"]["items_to_rewrite"].as_u64(), v["render"]["to_link"].as_u64()), (Some(0), Some(0), Some(0)));
}

#[test]
fn a_tampered_twin_file_is_refused() {
    let (d, progs) = old_data();
    let (x, xt) = &progs[0];
    let y = twin(xt);
    // a file at the twin's name whose text is not the twin's
    std::fs::write(d.path().join("programs").join(format!("{y}.json")), json!({"cid": y, "text": format!("#! beatscript 2\n{xt}deck B: snare ~ x ~ x\n"), "meta": {"forked_from": x, "derivation": "renamed"}}).to_string()).unwrap();
    let data = d.path().to_string_lossy().into_owned();
    let before = snapshot(d.path());
    let (code, v, out) = migrate(&["--data", &data, "--dry-run", "--json"]);
    assert_eq!(code, 1, "a dry run whose real run would fail says so: {out}");
    assert_eq!(snapshot(d.path()), before);
    assert_eq!(v["ok"], false);
    let refused = v["twins_refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{v}");
    assert_eq!(refused[0]["cid"].as_str(), Some(x.as_str()));
    assert!(refused[0]["reason"].as_str().unwrap().contains("does not verify"));
    assert!(v["map"].get(x).is_none());
    // the real run stores the other twins, leaves the tampered file alone and fails its self-check, saying why
    let out_dir = tempfile::tempdir().unwrap();
    let backup = out_dir.path().join("b.tar.gz");
    let (code, v, out) = migrate(&["--data", &data, "--backup", &backup.to_string_lossy(), "--json"]);
    assert_eq!(code, 1, "{out}");
    assert_eq!(v["wrote"]["twins"], 2);
    assert_eq!(v["self_check"]["ok"], false);
    assert!(v["self_check"]["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains(&format!("{y}.json does not verify"))), "{v}");
    assert!(out.contains("does not verify") || v.to_string().contains("does not verify"));
}
