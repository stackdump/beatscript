//! P2 + P3 of docs/RUST-SERVER.md: the site's public, stateless routes and the checker routes, as the Go server
//! answers them (internal/btwsite/server/handlers.go, site.go, assetlinks.go, qr.go, store.go, ld/):
//!
//!   GET      /api/info, /.well-known/btw-site (= /.well-known/beatscript-site), /.well-known/assetlinks.json, /api/instruments
//!   GET      /context/beatscript-v1.jsonld, /context/v1.jsonld, /context.jsonld, /context   (+ OPTIONS on all but the last)
//!   GET      /api/programs/{cid}[.beatscript|.btw]                   (examples first, ETag, 304)
//!   PUT      /api/programs/{cid}                              (put-ip-hour, store-max-mb, first meta wins, link(2))
//!   POST     /api/check, /api/fmt, /api/share, /api/open, /api/qr   (check-ip-min, 128 KiB / 4 KiB bodies)
//!
//! A wrong method on any of them is the static site's answer (Go's catch-all): 405 JSON with `Allow`, or its 404.
//!
//! `Checker` is Go's `beatscript.Client` without the subprocess: the same beatscript-api calls beatscript makes, the answers decoded into
//! Go's structs and written back as encoding/json writes them (`api.CheckReport` and friends: field order, omitempty,
//! diagnostics clipped), with beatscript.Client's limits: 4 calls at once (shared with the render route's checks), per
//! caller address 2 in flight and a bank of 30 s of checker time refilled at 0.5 s/s, 10 s per call, 256 KiB of
//! program text, 4 MiB of answer. MCP and compose (P7) use it through `app.checker`.

mod gojson;
pub mod qr;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, Method, Response};
use axum::routing::any;
use axum::Router;
use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;

#[allow(unused_imports)]
pub use gojson::{encode as go_encode, float as go_float};

use crate::service::Limiter;
use crate::{client_ip, go_body, store, ApiError, AppState};

/// `maxCheckBody`: the JSON body of check, fmt, share and open.
pub const MAX_CHECK_BODY: usize = 128 << 10;
const MAX_QR_BODY: usize = 4 << 10;
const MAX_QR_LINK: usize = 2000;
/// beatscript.Client's `MaxOpenArg`.
pub const MAX_OPEN_ARG: usize = 100 << 10;
/// beatscript.Client's defaults.
const MAX_STDIN: usize = 256 << 10;
const MAX_STDOUT: usize = 4 << 20;
const MAX_DIAGNOSTICS: usize = 200;

const PUBLIC_NOTICE: &str = "Everything generated here is published: the program, its CID, share link and rendered audio are stored publicly and released under CC0-1.0 (public domain), and anyone with the link can play, copy and reuse them. The prompt goes to a third-party model provider. Do not put anything private in a prompt.";
const CONTENT_LICENSE: &str = "CC0-1.0";

// ---------- the checker ----------

struct OwnerState {
    sem: Arc<Semaphore>,
    bal: f64,
    at: Instant,
    refs: usize,
}

/// The in-process checker (check, fmt, same_as, share, open, instruments), shared with MCP and compose (P7).
/// Every call names its owner: the caller's address (`client_ip`), or "" for work no one is charged for.
pub struct Checker {
    slots: Arc<Semaphore>,
    owners: Arc<Mutex<HashMap<String, OwnerState>>>,
    timeout: Duration,
    owner_concurrent: usize,
    owner_seconds: f64,
    owner_refill: f64,
    instruments: OnceLock<String>,
}

const MAX_OWNERS: usize = 50_000;

impl Default for Checker {
    fn default() -> Checker {
        Checker::new(Arc::new(Semaphore::new(4)))
    }
}

fn busy() -> ApiError {
    ApiError::new(503, "busy", "the checker is busy; try again in a moment").retry(2)
}

#[allow(dead_code)] // same_as, cheatsheet: for MCP and compose (P7)
impl Checker {
    /// `slots` is the process's checker concurrency (beatscript's MaxConcurrent, 4): `App::check_slots`.
    pub fn new(slots: Arc<Semaphore>) -> Checker {
        Checker {
            slots,
            owners: Arc::new(Mutex::new(HashMap::new())),
            timeout: Duration::from_secs(10),
            owner_concurrent: 2,
            owner_seconds: 30.0,
            owner_refill: 0.5,
            instruments: OnceLock::new(),
        }
    }

    /// beatscript.Client.run: the owner's budget and slot, a process slot, the work on a blocking thread, its time charged.
    async fn run<F>(&self, owner: &str, what: &str, stdin_len: usize, f: F) -> Result<String, ApiError>
    where
        F: FnOnce() -> String + Send + 'static,
    {
        if stdin_len > MAX_STDIN {
            return Err(ApiError::new(413, "too_large", format!("program text is {stdin_len} bytes; the limit is {MAX_STDIN}")));
        }
        let deadline = tokio::time::Instant::now() + self.timeout;
        let mut owner_permit = None;
        if !owner.is_empty() {
            let sem = {
                let now = Instant::now();
                let mut m = self.owners.lock().unwrap();
                if !m.contains_key(owner) && m.len() >= MAX_OWNERS {
                    m.retain(|_, o| o.refs > 0);
                }
                let st = m.entry(owner.to_string()).or_insert_with(|| OwnerState { sem: Arc::new(Semaphore::new(self.owner_concurrent)), bal: self.owner_seconds, at: now, refs: 0 });
                st.bal = self.owner_seconds.min(st.bal + now.duration_since(st.at).as_secs_f64() * self.owner_refill);
                st.at = now;
                if st.bal <= 0.0 {
                    let wait = ((-st.bal) / self.owner_refill) as u64 + 1;
                    return Err(ApiError::new(429, "rate_limited", format!("your address has used its share of the checker's time (programs that take seconds to check add up); try again in {wait}s")).retry(wait));
                }
                st.refs += 1;
                st.sem.clone()
            };
            match tokio::time::timeout_at(deadline, sem.acquire_owned()).await {
                Ok(Ok(p)) => owner_permit = Some(p),
                _ => {
                    self.owners.lock().unwrap().get_mut(owner).map(|o| o.refs -= 1);
                    return Err(busy());
                }
            }
        }
        let release = |owners: &Mutex<HashMap<String, OwnerState>>, owner: &str, took: f64| {
            if let Some(o) = owners.lock().unwrap().get_mut(owner) {
                o.refs -= 1;
                o.bal -= took;
            }
        };
        let slot = match tokio::time::timeout_at(deadline, self.slots.clone().acquire_owned()).await {
            Ok(Ok(p)) => p,
            _ => {
                if owner_permit.is_some() {
                    release(&self.owners, owner, 0.0);
                }
                return Err(busy());
            }
        };
        let (owners, owner_s) = (self.owners.clone(), owner.to_string());
        let task = tokio::task::spawn_blocking(move || {
            let began = Instant::now();
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            drop(slot);
            if let Some(p) = owner_permit {
                drop(p);
                if let Some(o) = owners.lock().unwrap().get_mut(&owner_s) {
                    o.refs -= 1;
                    o.bal -= began.elapsed().as_secs_f64();
                }
            }
            out
        });
        let out = match tokio::time::timeout_at(deadline, task).await {
            Err(_) => return Err(ApiError::new(504, "checker_timeout", format!("beatscript {what} timed out after {}s", self.timeout.as_secs()))),
            Ok(Ok(Ok(s))) => s,
            Ok(_) => {
                eprintln!("beatscript-serve: checker {what} panicked");
                return Err(ApiError::new(502, "checker_error", "the checker failed; try again"));
            }
        };
        // beatscript prints the answer and a newline; more than MaxStdout of it is refused
        if out.len() + usize::from(!out.ends_with('\n')) > MAX_STDOUT {
            return Err(ApiError::new(422, "bad_request", format!("beatscript {what}: the answer is over {MAX_STDOUT} bytes: a program or link with that many problems cannot be reported; fix the first ones and try again")));
        }
        Ok(out)
    }

    fn parse(out: &str, what: &str) -> Result<Value, ApiError> {
        serde_json::from_str(out).map_err(|e| {
            eprintln!("beatscript-serve: beatscript {what}: answer is not the expected JSON: {e}");
            ApiError::new(502, "checker_error", "the checker failed; try again")
        })
    }

    /// `Checker.Check`: the `api.CheckReport` as Go writes it.
    pub async fn check(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        self.check_with(owner, text, false).await
    }

    /// [`Checker::check`], and with `expand` each reference's `expansion` in `refs` (`/api/check` and the MCP check with
    /// `expand: true`).
    pub async fn check_with(&self, owner: &str, text: &str, expand: bool) -> Result<Value, ApiError> {
        let t = text.to_string();
        let out = self.run(owner, "check", text.len(), move || beatscript_api::check_json_with(&t, expand)).await?;
        Ok(shape_check(&Self::parse(&out, "check")?))
    }

    /// `Checker.Fmt`: the `api.FmtReport`.
    pub async fn fmt(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        let t = text.to_string();
        let out = self.run(owner, "fmt", text.len(), move || beatscript_api::fmt_json(&t)).await?;
        Ok(shape_fmt(&Self::parse(&out, "fmt")?))
    }

    /// `Checker.Open`: the `api.OpenReport` of a link, a fragment or a CID.
    pub async fn open(&self, owner: &str, url_or_cid: &str) -> Result<Value, ApiError> {
        let a = url_or_cid.trim();
        if a.is_empty() {
            return Err(ApiError::new(400, "bad_request", "nothing to open: give a link, a fragment or a CID"));
        }
        if a.len() > MAX_OPEN_ARG {
            return Err(ApiError::new(413, "too_large", format!("link is {} bytes; the limit is {MAX_OPEN_ARG}", a.len())));
        }
        if a.starts_with('-') || a.contains('\0') {
            return Err(ApiError::new(400, "bad_request", "not a BeatScript link or CID"));
        }
        let t = a.to_string();
        let out = self.run(owner, "open", 0, move || beatscript_api::open_json(&t)).await?;
        Ok(shape_open(&Self::parse(&out, "open")?))
    }

    /// `Checker.Share`: the `api.ShareReport` with beatscript's default origin (https://beatscript.app); `fix_share` makes
    /// the links a site's own.
    pub async fn share(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        let t = text.to_string();
        let out = self.run(owner, "share", text.len(), move || beatscript_api::share_json(&t, None, false, "")).await?;
        Ok(shape_share(&Self::parse(&out, "share")?))
    }

    /// `Client.SameAs`: beatscript's `same-as` JSON for two program texts, as is (Go relays it as a json.RawMessage).
    pub async fn same_as(&self, owner: &str, a: &str, b: &str) -> Result<String, ApiError> {
        let stdin_len = serde_json::to_string(&json!({"a": a, "b": b})).map_or(0, |s| s.len());
        let (a, b) = (a.to_string(), b.to_string());
        let out = self.run(owner, "same-as", stdin_len, move || beatscript_api::same_as_json(&a, &b)).await?;
        let v = Self::parse(&out, "same-as")?;
        if !v.is_object() {
            return Err(ApiError::new(502, "checker_error", "the checker failed; try again"));
        }
        Ok(out.trim().to_string())
    }

    /// A station request (radio, docs/beatscript/radio.md): beatscript-api's answer to the JSON `request` (station, n, count, segue, check,
    /// retune, locks, neighbours), on the checker's budget and slots like a check (its tracks and segues are checked when it
    /// asks). `POST /api/station` and the MCP `station` tool; stores nothing.
    pub async fn station(&self, owner: &str, request: &str) -> Result<Value, ApiError> {
        let r = request.to_string();
        let out = self.run(owner, "station", request.len(), move || beatscript_api::station_json(&r)).await?;
        Self::parse(&out, "station")
    }

    /// `Checker.Instruments`: beatscript's answer, trimmed, kept (it never changes for one binary).
    pub fn instruments(&self) -> &str {
        self.instruments.get_or_init(|| beatscript_api::instruments_json().trim().to_string())
    }

    /// `Checker.Cheatsheet`.
    pub fn cheatsheet(&self) -> String {
        beatscript_api::cheatsheet()
    }
}

// ---------- Go's structs, as encoding/json writes them ----------

fn s_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn i_of(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}
fn b_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn strs_of(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}
fn fptr(v: &Value, k: &str) -> Value {
    match v.get(k).and_then(Value::as_f64) {
        Some(f) if v[k].is_f64() => json!(f),
        Some(_) => v[k].clone(),
        None => Value::Null,
    }
}
fn put_s(m: &mut Map<String, Value>, k: &str, s: String, omitempty: bool) {
    if !(omitempty && s.is_empty()) {
        m.insert(k.into(), json!(s));
    }
}
fn put_strs(m: &mut Map<String, Value>, k: &str, a: Vec<String>) {
    if !a.is_empty() {
        m.insert(k.into(), json!(a));
    }
}
/// a Go map[string]string: sorted keys, omitempty
fn put_map(m: &mut Map<String, Value>, k: &str, v: Option<&Value>) {
    if let Some(Value::Object(o)) = v {
        if !o.is_empty() {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), json!(o[key].as_str().unwrap_or("")));
            }
            m.insert(k.into(), Value::Object(out));
        }
    }
}

/// beatscript.clip: at most n bytes, on a character boundary, marked.
fn clip(s: String, n: usize) -> String {
    if s.len() <= n {
        return s;
    }
    let mut cut = n;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// `api.Diagnostic`, clipped (`clipDiagnostics`).
fn shape_diag(d: &Value) -> Value {
    let mut m = Map::new();
    m.insert("severity".into(), json!(s_of(d, "severity")));
    for k in ["line", "col", "end_col", "start", "end"] {
        m.insert(k.into(), json!(i_of(d, k)));
    }
    m.insert("message".into(), json!(clip(s_of(d, "message"), 1000)));
    let ex: Vec<String> = strs_of(d, "expected").into_iter().take(20).map(|e| clip(e, 200)).collect();
    put_strs(&mut m, "expected", ex);
    put_s(&mut m, "found", clip(s_of(d, "found"), 200), true);
    put_s(&mut m, "help", clip(s_of(d, "help"), 500), true);
    put_s(&mut m, "render", clip(s_of(d, "render"), 2000), true);
    Value::Object(m)
}

fn diags(v: &Value) -> Vec<Value> {
    v.get("diagnostics").and_then(Value::as_array).map(|a| a.iter().take(MAX_DIAGNOSTICS).map(shape_diag).collect()).unwrap_or_default()
}

/// `api.CheckReport` (what `Client.Check` returns: diagnostics never null, clipped).
pub fn shape_check(v: &Value) -> Value {
    let mut m = Map::new();
    m.insert("ok".into(), json!(b_of(v, "ok")));
    m.insert("errors".into(), json!(i_of(v, "errors")));
    m.insert("warnings".into(), json!(i_of(v, "warnings")));
    m.insert("diagnostics".into(), Value::Array(diags(v)));
    m.insert("cid".into(), json!(s_of(v, "cid")));
    m.insert("canonical".into(), json!(s_of(v, "canonical")));
    versions(&mut m, v);
    put_map(&mut m, "nets", v.get("nets"));
    put_map(&mut m, "song_nets", v.get("song_nets"));
    m.insert("tempo".into(), fptr(v, "tempo"));
    put_s(&mut m, "key", s_of(v, "key"), true);
    m.insert("swing".into(), fptr(v, "swing"));
    m.insert("arranged".into(), json!(b_of(v, "arranged")));
    m.insert("song_bars".into(), json!(i_of(v, "song_bars")));
    m.insert("ends".into(), json!(b_of(v, "ends")));
    m.insert("loops".into(), json!(b_of(v, "loops")));
    if let Some(a) = v.get("sections").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        let secs: Vec<Value> = a
            .iter()
            .map(|s| {
                let mut e = json!({"name": s_of(s, "name"), "start": i_of(s, "start"), "bars": i_of(s, "bars")});
                // a documented section's doc comment (djlive::doc): present only when it has one
                if let Some(d) = s.get("doc").and_then(Value::as_str) {
                    e["doc"] = json!(d);
                }
                e
            })
            .collect();
        m.insert("sections".into(), Value::Array(secs));
    }
    put_strs(&mut m, "decks", strs_of(v, "decks"));
    put_strs(&mut m, "evolve", strs_of(v, "evolve"));
    if let Some(a) = v.get("lineage").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        m.insert("lineage".into(), Value::Array(a.clone())); // json.RawMessage: as beatscript wrote it
    }
    // `arrange { }` mutations (djlive mutate.rs): the generation table, and on an error the fixer's text; absent otherwise
    for k in ["arrange", "fixed"] {
        if let Some(x) = v.get(k).filter(|x| !x.is_null()) {
            m.insert(k.into(), x.clone());
        }
    }
    m.insert("uses_assets".into(), json!(b_of(v, "uses_assets")));
    m.insert("restricted".into(), json!(b_of(v, "restricted")));
    m.insert("seconds".into(), fptr(v, "seconds"));
    // the programs it draws on (an `import`, a set's tracks), leaves first: present only when there are some
    if let Some(a) = v.get("uses").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        m.insert("uses".into(), Value::Array(a.clone()));
    }
    // every doc comment ([{kind, name, doc}], djlive::doc): present only when there are some
    if let Some(a) = v.get("docs").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        m.insert("docs".into(), Value::Array(a.clone()));
    }
    station(&mut m, v);
    references(&mut m, v);
    Value::Object(m)
}

/// A station program's report (beatscript-api's `station`: its knobs, where its walk starts, the first chapter's tracks and the first
/// segue), as beatscript-api wrote it; absent for any other program. The MCP check's shape keeps it too.
pub fn station(m: &mut Map<String, Value>, v: &Value) {
    if let Some(s) = v.get("station").filter(|s| s.is_object()) {
        m.insert("station".into(), s.clone());
    }
}

/// What the recipe makes (beatscript_api::resolution), as beatscript-api wrote it: `expanded_cid` (a program that checks has one), and `refs`
/// and `resolution` (a program with references has them). The MCP check's shape (compose::engine::go_check_report) keeps the
/// same three.
pub fn references(m: &mut Map<String, Value>, v: &Value) {
    if let Some(e) = v.get("expanded_cid").and_then(Value::as_str) {
        m.insert("expanded_cid".into(), json!(e));
    }
    for k in ["refs", "resolution"] {
        if let Some(a) = v.get(k).and_then(Value::as_array).filter(|a| !a.is_empty()) {
            m.insert(k.into(), Value::Array(a.clone()));
        }
    }
}

/// The language version a check or fmt answer says (beatscript-api's report): `version` (1 or 2: `#! beatscript 2` is 2, a
/// text with `#! btw 1` or no version line is 1), `renamed_cid` (check: the CID of the same program in the other version)
/// and `from_cid` (fmt: the CID of the text as given, when fmt upgraded it to a new one). Each only when the answer has it.
/// The MCP check's shape (compose::engine::go_check_report) keeps the same.
pub fn versions(m: &mut Map<String, Value>, v: &Value) {
    if let Some(n) = v.get("version").and_then(Value::as_u64) {
        m.insert("version".into(), json!(n));
    }
    for k in ["renamed_cid", "from_cid"] {
        if let Some(c) = v.get(k).and_then(Value::as_str).filter(|c| !c.is_empty()) {
            m.insert(k.into(), json!(c));
        }
    }
}

/// `api.FmtReport`.
pub fn shape_fmt(v: &Value) -> Value {
    let mut m = Map::new();
    m.insert("ok".into(), json!(b_of(v, "ok")));
    m.insert("canonical".into(), json!(s_of(v, "canonical")));
    m.insert("cid".into(), json!(s_of(v, "cid")));
    versions(&mut m, v);
    // the program formatted, its header lines and comments kept: the editor's fmt button and what arrives from a client
    if let Some(t) = v["text"].as_str() {
        m.insert("text".into(), json!(t));
    }
    put_strs(&mut m, "comments", strs_of(v, "comments"));
    let d = diags(v);
    if !d.is_empty() {
        m.insert("diagnostics".into(), Value::Array(d));
    }
    Value::Object(m)
}

/// `api.OpenReport`.
pub fn shape_open(v: &Value) -> Value {
    let mut m = Map::new();
    m.insert("ok".into(), json!(b_of(v, "ok")));
    m.insert("cid".into(), json!(s_of(v, "cid")));
    m.insert("canonical".into(), json!(s_of(v, "canonical")));
    put_s(&mut m, "text", s_of(v, "text"), true);
    m.insert("verified".into(), json!(b_of(v, "verified")));
    put_strs(&mut m, "warnings", strs_of(v, "warnings"));
    put_strs(&mut m, "notes_unverified", strs_of(v, "notes_unverified"));
    put_s(&mut m, "error", s_of(v, "error"), true);
    Value::Object(m)
}

/// `api.ShareReport`.
pub fn shape_share(v: &Value) -> Value {
    let mut m = Map::new();
    m.insert("ok".into(), json!(b_of(v, "ok")));
    for k in ["url", "social_url", "cid"] {
        m.insert(k.into(), json!(s_of(v, k)));
    }
    m.insert("chars".into(), json!(i_of(v, "chars")));
    m.insert("canonical".into(), json!(s_of(v, "canonical")));
    put_strs(&mut m, "warnings", strs_of(v, "warnings"));
    Value::Object(m)
}

/// `Server.fixShare`: url = <origin>/#<CID>.<packed>, social_url = <origin>/c/<CID>.<packed> (no ~notes).
pub fn fix_share(public_url: &str, rep: &Value) -> Value {
    let mut out = rep.clone();
    let url = s_of(rep, "url");
    if let Some((_, frag)) = url.split_once('#') {
        let sealed = frag.split_once('~').map_or(frag, |(s, _)| s);
        out["url"] = json!(format!("{public_url}/#{frag}"));
        out["social_url"] = json!(format!("{public_url}/c/{sealed}"));
    }
    out
}

/// `api.Meta` as read back from a stored file.
pub fn shape_meta(v: &Value) -> Value {
    let mut m = Map::new();
    m.insert("license".into(), json!(s_of(v, "license")));
    m.insert("visibility".into(), json!(s_of(v, "visibility")));
    // `derivation: "renamed"` (and when): the BeatScript 2 twin of the program `forked_from` names
    for k in ["name", "composed_by", "forked_from", "derivation"] {
        put_s(&mut m, k, s_of(v, k), true);
    }
    put_strs(&mut m, "credits", strs_of(v, "credits"));
    for k in ["prompt_sha256", "author", "created", "source", "renamed_at"] {
        put_s(&mut m, k, s_of(v, k), true);
    }
    // what the text as saved said it vendored (its `# from:` lines), the stored text being canonical (gate::write_program)
    put_strs(&mut m, "based_on", strs_of(v, "based_on"));
    Value::Object(m)
}

/// A Go map[string]any: every object's keys sorted, all the way down.
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut kv: Vec<(String, Value)> = o.into_iter().collect();
            kv.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(kv.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        x => x,
    }
}

// ---------- answers ----------

/// `writeJSON`.
fn json_resp(status: u16, v: &Value) -> Response<Body> {
    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json; charset=utf-8").body(go_body(go_encode(v) + "\n")).unwrap()
}

/// `api.WriteError` of an `*api.Error` with `Details`.
fn error_details(status: u16, code: &str, message: &str, details: Value) -> Response<Body> {
    let v = json!({"error": {"code": code, "message": message, "details": details}});
    json_resp(status, &v)
}

fn not_found() -> Response<Body> {
    ApiError::new(404, "not_found", "not found").response()
}

/// `limited`.
fn limited(ok: bool, retry: u64, what: &str) -> Result<(), Response<Body>> {
    if ok {
        return Ok(());
    }
    Err(ApiError::new(429, "rate_limited", format!("too many {what} requests from your address; try again in {retry}s")).retry(retry).response())
}

// ---------- state ----------

pub struct Pub {
    app: AppState,
    checker: Arc<Checker>,
    check_ip: Option<Mutex<Limiter>>,
    site_id: String,
    public_url: String,
    store_total: AtomicI64,
    store_cap: i64,
}

impl Pub {
    fn ip(&self, req: &Request) -> String {
        let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
        client_ip(peer, req.headers())
    }

    fn allow_check(&self, ip: &str) -> (bool, u64) {
        match &self.check_ip {
            Some(l) => l.lock().unwrap().allow(ip),
            None => (true, 0),
        }
    }

    fn cfg(&self) -> &crate::Config {
        &self.app.cfg
    }
}

type St = State<Arc<Pub>>;

/// The body, at most `limit + 1` bytes of it (enough to know it is over).
async fn read_body(body: Body, limit: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut s = body.into_data_stream();
    while let Some(Ok(chunk)) = s.next().await {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            out.truncate(limit + 1);
            break;
        }
    }
    out
}

/// `decodeBody` into a struct of string fields.
fn decode_body(body: &[u8], max: usize, fields: &[&str], go_type: &str, struct_name: &str) -> Result<Vec<String>, Response<Body>> {
    gojson::decode_strings(body, max, fields, go_type, struct_name).map_err(|e| match e {
        gojson::DecodeError::TooLarge => ApiError::new(413, "too_large", format!("request body is over {max} bytes")).response(),
        gojson::DecodeError::Bad(m) => ApiError::new(400, "bad_request", format!("the body must be JSON: {m}")).response(),
    })
}

/// `checkBody`: the per-address limit, then `{"text", "url"}`.
async fn check_body(st: &Pub, req: Request) -> Result<(String, String, String), Response<Body>> {
    let ip = st.ip(&req);
    let (ok, retry) = st.allow_check(&ip);
    limited(ok, retry, "check")?;
    let b = read_body(req.into_body(), MAX_CHECK_BODY).await;
    let v = decode_body(&b, MAX_CHECK_BODY, &["text", "url"], "server.textBody", "textBody")?;
    Ok((ip, v[0].clone(), v[1].clone()))
}

fn answer(r: Result<Response<Body>, Response<Body>>) -> Response<Body> {
    r.unwrap_or_else(|e| e)
}

// ---------- P2 ----------

/// What /api/info says of compose: `compose.Service.Status`, from the same flags, files and environment (P7 owns
/// the service; this reads its switch and its day's spend record as Go does on every request).
fn compose_status(cfg: &crate::Config) -> (bool, bool, String, String) {
    let name = cfg.flag("model", "").trim().to_string();
    let key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    let (configured, model) = if name.eq_ignore_ascii_case("fake") {
        (true, "fake".to_string())
    } else if !key.trim().is_empty() {
        (true, if name.is_empty() { "gemini-3.8-flash".into() } else { name })
    } else {
        (false, if name.is_empty() { "gemini-3.8-flash".into() } else { name })
    };
    let enabled_flag = !matches!(cfg.flag("compose-enabled", "true").trim().to_lowercase().as_str(), "0" | "false" | "no" | "off" | "f");
    let file_off = cfg.data.join("compose.off").exists();
    let enabled = enabled_flag && !file_off;
    let cap = cfg.flag_f64("daily-spend-usd", 2.0);
    let day = &store::rfc3339_now()[..10];
    let (spent, broke) = match std::fs::read(cfg.data.join("spend").join(format!("{day}.json"))) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0.0, false),
        Err(_) => (0.0, true),
        Ok(b) => match serde_json::from_slice::<Value>(&b) {
            Ok(v) if v.is_object() => (v["usd"].as_f64().unwrap_or(0.0), false),
            _ => (0.0, true),
        },
    };
    let reason = if !configured {
        "compose is not available on this site right now"
    } else if !enabled {
        "compose is switched off for now"
    } else if broke {
        "compose is closed for now"
    } else if cap >= 0.0 && spent >= cap {
        "today's compose budget is used up; it resets at 00:00 UTC"
    } else {
        ""
    };
    (enabled, configured, model, reason.to_string())
}

fn site_doc(st: &Pub) -> Value {
    let cfg = st.cfg();
    let mut origins = vec![json!(st.public_url)];
    origins.extend(parse_origins(&cfg.alt_origins).into_iter().map(Value::from));
    let mut d = json!({
        "proto": "btw-site/1", "site_id": st.site_id, "origin": st.public_url, "origins": origins,
        "pair": ["btw-pair/2", "btw-pair/1"], "license": CONTENT_LICENSE, "privacy": format!("{}/privacy", st.public_url),
    });
    let moved = cfg.flag("moved-to", "").trim_end_matches('/').to_string();
    if !moved.is_empty() {
        d["moved_to"] = json!(moved);
    }
    sorted(d)
}

fn parse_origins(list: &str) -> Vec<String> {
    list.split(',').map(|p| p.trim().trim_end_matches('/').to_string()).filter(|p| !p.is_empty()).collect()
}

async fn info(State(st): St) -> Response<Body> {
    let cfg = st.cfg();
    // Go: `s.composer.Status()` (the live service: its kill switch, spend record and in-memory "cap hit today"),
    // with only the public reason shown; the file-based reading is for a start where compose is not up
    let (enabled, configured, model, reason, limits) = match crate::compose::get() {
        Some(c) => {
            let cs = c.status();
            let mut reason = cs["public_reason"].as_str().unwrap_or("").to_string();
            if reason.is_empty() && cs["reason"].as_str().is_some_and(|r| !r.is_empty()) {
                reason = "compose is not available right now".into();
            }
            let lim = (cs["user_per_hour"].clone(), cs["user_per_day"].clone(), cs["ip_per_day"].clone(), cs["daily_cap_usd"].clone());
            (cs["enabled"].as_bool().unwrap_or(false), cs["configured"].as_bool().unwrap_or(false), cs["model"].as_str().unwrap_or("").to_string(), reason, Some(lim))
        }
        None => {
            let (e, c, m, r) = compose_status(cfg);
            (e, c, m, r, None)
        }
    };
    let (uh, ud, ipd, cap) = limits.unwrap_or_else(|| {
        (json!(cfg.flag_num("compose-user-hour", 10)), json!(cfg.flag_num("compose-user-day", 50)), json!(cfg.flag_num("compose-ip-day", 40)), json!(cfg.flag_f64("daily-spend-usd", 2.0)))
    });
    let max_prog = cfg.max_program_bytes;
    let v = json!({
        "name": "beatscript", "version": cfg.version, "public_url": st.public_url,
        "android_beta": crate::beta::get(&st.app).enabled(), "site": site_doc(&st), "license": CONTENT_LICENSE,
        "notice": PUBLIC_NOTICE, "model": model,
        "compose": {"enabled": enabled, "configured": configured, "reason": reason, "limits": {
            "user_per_hour": uh, "user_per_day": ud, "ip_per_day": ipd, "daily_cap_usd": cap,
            "max_prompt_chars": cfg.flag_num("max-prompt-chars", 800), "max_program_bytes": max_prog}},
        "render": {"formats": ["opus", "wav"], "default_format": "opus", "sample_rates": [44100, 48000],
            "max_bars": cfg.render_max_bars, "concurrency": cfg.render_concurrency, "ip_per_hour": cfg.render_ip_hour},
        "auth": {"kinds": ["eth", "ed25519"], "cookie": "btw_session", "token_prefix": "btw_"},
        "limits": {"max_check_bytes": MAX_CHECK_BODY, "max_program_bytes": max_prog, "check_per_minute": cfg.flag_num("check-ip-min", 120), "put_per_hour": cfg.put_ip_hour},
        "examples": st.app.examples.list.len(), "mcp": "/mcp",
    });
    let mut r = json_resp(200, &sorted(v));
    r.headers_mut().insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    r
}

async fn site_route(State(st): St) -> Response<Body> {
    let mut r = json_resp(200, &site_doc(&st));
    r.headers_mut().insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    r.headers_mut().insert("access-control-allow-origin", "*".parse().unwrap());
    r
}

fn json_valid(b: &[u8]) -> bool {
    serde_json::from_slice::<serde::de::IgnoredAny>(b).is_ok()
}

async fn assetlinks(State(st): St) -> Response<Body> {
    let f = st.cfg().flag("assetlinks-file", "");
    if f.is_empty() {
        return not_found();
    }
    match std::fs::read(&f) {
        Ok(b) if json_valid(&b) => Response::builder()
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(go_body(b))
            .unwrap(),
        r => {
            eprintln!("assetlinks: {f} unreadable or not JSON: {:?}", r.err());
            ApiError::new(500, "internal", "the asset links file is not available").response()
        }
    }
}

async fn instruments(State(st): St, req: Request) -> Response<Body> {
    let ip = st.ip(&req);
    let (ok, retry) = st.allow_check(&ip);
    if let Err(r) = limited(ok, retry, "check") {
        return r;
    }
    let raw = st.checker.instruments().to_string();
    Response::builder()
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(go_body(raw))
        .unwrap()
}

// the linked-data contexts (ld/)
/// The first context (`/context/v1.jsonld`, the djrust: terms), published immutable: served byte for byte, with its ETag,
/// for the documents made before 2026-11-23 that name it. No document names it now: every program's names the BeatScript
/// context, whatever the version of its text.
const CONTEXT_V1: &[u8] = include_bytes!("public/btw-context-v1.jsonld");
const CONTEXT_V1_ETAG: &str = "\"btw-context-v1\"";
/// The BeatScript context (`/context/beatscript-v1.jsonld`): the v1 terms and the program block's in the BeatScript
/// vocabulary (`https://beatscript.app/schema#`, the one `/schema` defines), named by every program's
/// documents (its `bw:` is the BeatScript ontology, beatscript.app/ontology/ns#). Immutable too: a change to its terms is a new file and URL. `/context.jsonld` and `/context` answer it.
pub const CONTEXT_BEATSCRIPT_V1: &[u8] = include_bytes!("public/beatscript-context-v1.jsonld");
const CONTEXT_BEATSCRIPT_V1_ETAG: &str = "\"beatscript-context-v1\"";
/// The vocabulary a context's terms are described by: the BeatScript ontology, on this site.
const ONTOLOGY_LINK: &str = "<https://beatscript.app/ontology/ns.ttl>; rel=\"describedby\"; type=\"text/turtle\"";

fn ld_serve(req: &Request, cache: &'static str, body: &'static [u8], etag: &'static str) -> Response<Body> {
    let mut b = Response::builder()
        .header("access-control-allow-origin", "*")
        .header(header::CACHE_CONTROL, cache)
        .header(header::LINK, ONTOLOGY_LINK)
        .header("x-content-type-options", "nosniff")
        .header(header::ETAG, etag);
    if req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag) {
        // a 304 drops Content-Type (net/http's suppressed headers)
        return b.status(304).body(crate::content::nothing()).unwrap();
    }
    b = b.header(header::CONTENT_TYPE, "application/ld+json; charset=utf-8").header(header::CONTENT_LENGTH, body.len());
    b.body(Body::from(body)).unwrap()
}

/// `/context/v1.jsonld`: the first context, as it always was.
async fn ld_v1(req: Request) -> Response<Body> {
    ld_serve(&req, "public, max-age=31536000, immutable", CONTEXT_V1, CONTEXT_V1_ETAG)
}
/// `/context/beatscript-v1.jsonld`: the BeatScript context, immutable.
async fn ld_beatscript_v1(req: Request) -> Response<Body> {
    ld_serve(&req, "public, max-age=31536000, immutable", CONTEXT_BEATSCRIPT_V1, CONTEXT_BEATSCRIPT_V1_ETAG)
}
/// `/context.jsonld`, `/context`: the newest context (the BeatScript one), cached briefly since it moves with a version.
async fn ld_latest(req: Request) -> Response<Body> {
    ld_serve(&req, "public, max-age=300", CONTEXT_BEATSCRIPT_V1, CONTEXT_BEATSCRIPT_V1_ETAG)
}
async fn ld_preflight() -> Response<Body> {
    Response::builder()
        .status(204)
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, OPTIONS")
        .header("access-control-allow-headers", "Content-Type")
        .body(Body::empty())
        .unwrap()
}

// programs

/// `beatscriptFile`: the `.beatscript` download, the `#!` header lines (rights, provenance) in front of the text, then the `# from:`
/// lines its save kept (`meta.based_on`), so the file says what it vendored and a save of it keeps them again.
fn beatscript_file(cid: &str, text: &str, meta: &Value) -> String {
    let mut b = String::new();
    let one = |s: String| s.replace(['\n', '\r'], " ");
    for (k, v) in [("name", s_of(meta, "name")), ("license", s_of(meta, "license")), ("composed-by", s_of(meta, "composed_by")), ("forked-from", s_of(meta, "forked_from")), ("cid", cid.to_string())] {
        if !v.is_empty() {
            b.push_str(&format!("#! {k}: {}\n", one(v)));
        }
    }
    for l in strs_of(meta, "based_on") {
        b.push_str(&format!("# from: {}\n", one(l)));
    }
    b.push_str(text);
    if !text.ends_with('\n') {
        b.push('\n');
    }
    b
}

impl Pub {
    /// `Server.Get`: the examples first (their own meta), then the store.
    fn program(&self, cid: &str) -> Option<(String, Value)> {
        if let Some(ex) = self.app.examples.list.iter().find(|e| e.cid == cid) {
            let text = match self.app.store.get(cid) {
                Some(p) => p.text,
                None => s_of(&store::check(self.app.examples.text.get(&ex.name)?), "canonical"),
            };
            let meta = json!({"license": ex.license, "visibility": "public", "name": ex.name, "source": "example"});
            return Some((text, meta));
        }
        let p = self.app.store.get(cid)?;
        let meta = match self.app.store.file_meta(cid) {
            Some(m) => shape_meta(&m),
            None if self.app.store.is_example(cid) => json!({"license": "CC0-1.0", "visibility": "public", "source": "example"}),
            None => json!({"license": "CC0-1.0", "visibility": "public", "source": "put"}),
        };
        Some((p.text, meta))
    }
}

async fn get_program(State(st): St, req: Request) -> Response<Body> {
    let raw = cid_of(&req);
    // `.beatscript`, or `.btw` (its name before the rename, still answered: the same response)
    let (cid, as_text) = match crate::content::strip_program_suffix(&raw) {
        Some(c) => (c.to_string(), true),
        None => (raw.clone(), false),
    };
    if !store::is_cid(&cid) {
        return ApiError::new(404, "not_found", "no such program").response();
    }
    let (st2, cid2) = (st.clone(), cid.clone());
    // the rename: an old program still answers with its own text (it is immutable), and names its BeatScript 2 twin
    let found = tokio::task::spawn_blocking(move || st2.program(&cid2).map(|p| (p, st2.app.store.renamed_to(&cid2), st2.app.store.renamed_from(&cid2)))).await.ok().flatten();
    let Some(((text, meta), renamed_to, renamed_from)) = found else {
        return ApiError::new(404, "not_found", format!("no program {cid} here; PUT it first")).response();
    };
    let etag = format!("\"{cid}\"");
    let mut b = Response::builder().header(header::CACHE_CONTROL, "public, max-age=31536000, immutable").header(header::ETAG, &etag);
    let link = renamed_link(&st.public_url, renamed_to.as_deref(), renamed_from.as_deref());
    if let Some(l) = &link {
        b = b.header(header::LINK, l);
    }
    if req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return b.status(304).body(crate::content::nothing()).unwrap();
    }
    if as_text {
        // the API's program text is BeatScript's own type (a person's URL, /p/<cid>.beatscript, stays text/plain)
        return b
            .header(header::CONTENT_TYPE, crate::content::BEATSCRIPT_TYPE)
            .header(header::CONTENT_DISPOSITION, format!("inline; filename=\"{cid}.beatscript\""))
            .body(go_body(beatscript_file(&cid, &text, &meta)))
            .unwrap();
    }
    let mut doc = json!({"cid": cid, "text": text, "meta": meta});
    if let Some(y) = &renamed_to {
        doc["renamed_to"] = json!(y);
    }
    if let Some(x) = &renamed_from {
        doc["renamed_from"] = json!(x);
    }
    let mut r = json_resp(200, &doc);
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, "public, max-age=31536000, immutable".parse().unwrap());
    h.insert(header::ETAG, etag.parse().unwrap());
    if let Some(l) = link.and_then(|l| l.parse().ok()) {
        h.insert(header::LINK, l);
    }
    r
}

/// The `Link` of a program with a twin: `rel="successor-version"` to the BeatScript 2 program an old one is renamed to,
/// `rel="predecessor-version"` to the legacy program a BeatScript 2 one was renamed from (RFC 5829).
fn renamed_link(origin: &str, to: Option<&str>, from: Option<&str>) -> Option<String> {
    let origin = origin.trim_end_matches('/');
    match (to, from) {
        (Some(y), _) => Some(format!("<{origin}/api/programs/{y}>; rel=\"successor-version\"")),
        (None, Some(x)) => Some(format!("<{origin}/api/programs/{x}>; rel=\"predecessor-version\"")),
        _ => None,
    }
}

/// The `{cid}` segment of /api/programs/{cid}, percent-decoded as Go's PathValue is.
fn cid_of(req: &Request) -> String {
    crate::site::percent_decode(req.uri().path().strip_prefix("/api/programs/").unwrap_or(""))
}

// ---------- P3 ----------

/// Go's `strings.TrimSpace(s) == ""` on bytes that may not be UTF-8 (an invalid byte is not space).
fn blank(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok_and(|s| s.trim().is_empty())
}

/// `headerName`: `#! name: X` of the text as submitted.
fn header_name(text: &str) -> String {
    for l in text.split('\n') {
        if let Some(rest) = l.trim().strip_prefix("#!") {
            if let Some(v) = rest.trim().strip_prefix("name:") {
                let v = v.trim();
                let v = if v.len() > 80 { &v[..v.floor_char_boundary(80)] } else { v };
                return v.chars().filter(|c| (*c as u32) >= 0x20 && *c != '\u{7f}').collect();
            }
        }
    }
    String::new()
}

async fn put_program(State(st): St, req: Request) -> Response<Body> {
    let cid = cid_of(&req);
    if !store::is_cid(&cid) {
        return ApiError::new(400, "bad_request", format!("{} is not a CID", crate::site::go_quote(&cid))).response();
    }
    let ip = st.ip(&req);
    // the limit is on distinct work: refused before the body is even read
    let (ok, retry) = st.app.put_ip.lock().unwrap().allow(&ip);
    if let Err(r) = limited(ok, retry, "upload") {
        return r;
    }
    let max = st.cfg().max_program_bytes;
    let ct = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let body = read_body(req.into_body(), max + 1).await;
    if body.len() > max + 1 {
        return ApiError::new(413, "too_large", format!("program is over {max} bytes")).response();
    }
    if ct.contains("json") {
        return ApiError::new(415, "bad_request", "send the program text itself as the body (Content-Type: text/beatscript or text/plain), not JSON").response();
    }
    // programStore.put
    if body.len() > max {
        return ApiError::new(413, "too_large", format!("program is {} bytes; the limit is {max}", body.len())).response();
    }
    if blank(&body) {
        return ApiError::new(400, "bad_request", "program text is empty").response();
    }
    let Ok(text) = String::from_utf8(body) else {
        return ApiError::new(400, "bad_request", "program text must be UTF-8").response();
    };
    if store::declares_not_public(&text) {
        return ApiError::new(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site").response();
    }
    let rep = match st.checker.check(&ip, &text).await {
        Ok(r) => r,
        Err(e) => return e.response(),
    };
    if rep["ok"] != true {
        let first = rep["diagnostics"]
            .as_array()
            .and_then(|a| a.iter().find(|d| d["severity"] == "error"))
            .map_or("see the diagnostics".to_string(), |d| format!("{}:{} {}", i_of(d, "line"), i_of(d, "col"), s_of(d, "message")));
        let msg = format!("the program has {} error(s): {first}", i_of(&rep, "errors"));
        return error_details(422, "bad_request", &msg, json!({"check": rep}));
    }
    if rep["restricted"] == true || rep["uses_assets"] == true {
        return ApiError::new(422, "restricted", "programs that use restricted sounds or sample/loop assets are not stored on this site").response();
    }
    let (got, canonical) = (s_of(&rep, "cid"), s_of(&rep, "canonical"));
    if got.is_empty() || canonical.is_empty() || !store::is_cid(&got) || djlive::compile::program_cid(&canonical) != got {
        eprintln!("beatscript-serve: internal error: the checker answered no usable CID for a PUT");
        return ApiError::new(502, "checker_error", "the checker failed; try again").response();
    }
    if got != cid {
        return ApiError::new(422, "cid_mismatch", format!("that text is program {got}, not {cid}")).response();
    }
    let warnings = i_of(&rep, "warnings");
    let reply = |status: u16, created: bool, meta: Value| json_resp(status, &json!({"cid": got, "created": created, "meta": meta, "warnings": warnings}));
    if let Some(m) = st.app.store.file_meta(&cid) {
        if st.app.store.get(&cid).is_some() {
            return reply(200, false, shape_meta(&m));
        }
    }
    let mut meta = Map::new();
    meta.insert("license".into(), json!("CC0-1.0"));
    meta.insert("visibility".into(), json!("public"));
    let name = header_name(&text);
    if !name.is_empty() {
        meta.insert("name".into(), json!(name));
    }
    meta.insert("created".into(), json!(store::rfc3339_now()));
    meta.insert("source".into(), json!("put"));
    // the text's `# from:` lines (an eject's provenance): the canonical text has no comments, so they are kept beside it, as
    // gate::write_program keeps them from the text sent
    let based_on = djlive::refs::from_lines(&text);
    if !based_on.is_empty() {
        meta.insert("based_on".into(), json!(based_on));
    }
    let meta = Value::Object(meta);
    let size = go_encode(&json!({"cid": cid, "text": canonical, "meta": meta})).len() as i64;
    if st.store_total.load(Ordering::Relaxed) + size > st.store_cap {
        return ApiError::new(507, "store_full", "the programs store is full").retry(3600).response();
    }
    match st.app.store.put_new(&cid, &text, meta.clone(), go_encode) {
        Ok(Some(n)) => {
            st.store_total.fetch_add(n as i64, Ordering::Relaxed);
            reply(201, true, meta)
        }
        Ok(None) => match st.program(&cid) {
            Some((_, m)) => reply(200, false, m),
            None => ApiError::internal("the stored program cannot be read back".into()).response(),
        },
        Err(e) => ApiError::internal(e.to_string()).response(),
    }
}

async fn check(State(st): St, req: Request) -> Response<Body> {
    answer(async {
        // `{"text", "url", "genre"}`: genre (else the text's `#! genre:` line) adds the genre's style warnings
        let ip = st.ip(&req);
        let (ok, retry) = st.allow_check(&ip);
        limited(ok, retry, "check")?;
        let b = read_body(req.into_body(), MAX_CHECK_BODY).await;
        let v = decode_body(&b, MAX_CHECK_BODY, &["text", "url", "genre"], "server.textBody", "textBody")?;
        let (text, genre) = (v[0].clone(), v[2].clone());
        // `"expand": true`: each reference in refs carries its expansion (the body's one field that is not a string)
        let expand = serde_json::from_slice::<Value>(&b).ok().and_then(|v| v.get("expand").and_then(Value::as_bool)).unwrap_or(false);
        let mut rep = st.checker.check_with(&ip, &text, expand).await.map_err(|e| e.response())?;
        if let Some(g) = beatscript_serve::genres::wanted(&text, Some(&genre)) {
            let t = text.clone();
            let style = tokio::task::spawn_blocking(move || beatscript_serve::genres::installed_style(&t, &g)).await.unwrap_or(Value::Null);
            if let Some(m) = rep.as_object_mut() {
                m.insert("style".into(), style);
            }
        }
        Ok(json_resp(200, &rep))
    }
    .await)
}

/// `POST /api/generate`: the seeded generator (no sign-in, no model). JSON `{genre, bpm, key, length, seed}`, every
/// field optional. Rate-limited as an upload (it stores the program, public CC0, source "generate").
async fn generate(State(st): St, req: Request) -> Response<Body> {
    let ip = st.ip(&req);
    let (ok, retry) = st.app.put_ip.lock().unwrap().allow(&ip);
    if let Err(r) = limited(ok, retry, "generate") {
        return r;
    }
    let b = read_body(req.into_body(), 4096).await;
    if b.len() > 4096 {
        return ApiError::new(413, "too_large", "the request is over 4096 bytes").response();
    }
    let v: Value = if b.iter().all(u8::is_ascii_whitespace) { json!({}) } else {
        match serde_json::from_slice(&b) {
            Ok(v @ Value::Object(_)) => v,
            _ => return ApiError::new(400, "bad_request", "send a JSON object: {genre, bpm, key, length, seed}").response(),
        }
    };
    let s = |k: &str| match &v[k] {
        Value::String(s) => s.chars().take(64).collect(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let bpm = match &v["bpm"] {
        Value::Number(n) => n.as_f64(),
        Value::String(t) if !t.trim().is_empty() => t.trim().parse().ok(),
        _ => None,
    };
    let greq = beatscript_api::GenRequest { genre: s("genre"), bpm, key: s("key"), length: s("length"), seed: s("seed") };
    // the MCP tool's `generator` (1 or 2), `part` and `section` (one seed: `seeds` is the MCP tool's alone)
    let mut ask = crate::mcp::tools::GenAsk::new(greq);
    ask.version = match crate::mcp::tools::gen_version(v.get("generator").filter(|g| !g.is_null())) {
        Ok(n) => n,
        Err(e) => return e.response(false),
    };
    ask.part = Some(s("part")).filter(|p| !p.trim().is_empty());
    ask.section = Some(s("section")).filter(|p| !p.trim().is_empty());
    if let Some(p) = &ask.part {
        if !djlive::parse::ROLES.contains(&p.as_str()) {
            return ApiError::new(400, "bad_request", format!("part is a role: one of {}", djlive::parse::ROLES.join(", "))).response();
        }
    }
    let (app, public) = (st.app.clone(), st.public_url.clone());
    let eng = crate::compose::engine::Engine::over(app.checker.get().cloned());
    let r = tokio::task::spawn_blocking(move || crate::mcp::tools::generate_and_store(&app, &eng, &public, &ip, &ask)).await;
    match r {
        Ok(Ok(out)) => json_resp(200, &out),
        Ok(Err(e)) => e.response(false),
        Err(_) => ApiError::internal("the generator failed".into()).response(),
    }
}

/// `POST /api/station`: a station (radio, docs/beatscript/radio.md), as the MCP `station` tool and the page's `beatscript_station` answer
/// it: JSON `{station, n, count, segue, check, retune, locks, neighbours}` in, beatscript-api's answer out (`ok: false` with the error
/// is an answer, as check's is). Rate-limited like check and run on the checker's budget; it stores nothing.
async fn station_route(State(st): St, req: Request) -> Response<Body> {
    answer(async {
        let ip = st.ip(&req);
        let (ok, retry) = st.allow_check(&ip);
        limited(ok, retry, "station")?;
        let b = read_body(req.into_body(), MAX_CHECK_BODY).await;
        if b.len() > MAX_CHECK_BODY {
            return Err(ApiError::new(413, "too_large", format!("request body is over {MAX_CHECK_BODY} bytes")).response());
        }
        let (body, v) = match std::str::from_utf8(&b).ok().and_then(|s| serde_json::from_str::<Value>(s).ok().filter(Value::is_object).map(|v| (s.to_string(), v))) {
            Some(x) => x,
            None => return Err(ApiError::new(400, "bad_request", "send a JSON object: {station, n, count, segue, check, retune, locks, neighbours}").response()),
        };
        // (as the MCP tool charges: with check, one more call of the checker for each program checked)
        if v.get("check").and_then(Value::as_bool) == Some(true) {
            let count = v.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, beatscript_api::station::MAX_CHECKED);
            let segue = v.get("segue").and_then(Value::as_bool) != Some(false);
            for _ in 1..count * if segue { 2 } else { 1 } {
                let (ok, retry) = st.allow_check(&ip);
                limited(ok, retry, "station")?;
            }
        }
        let rep = st.checker.station(&ip, &body).await.map_err(|e| e.response())?;
        Ok(json_resp(200, &rep))
    }
    .await)
}

async fn fmt(State(st): St, req: Request) -> Response<Body> {
    answer(async {
        let (ip, text, _) = check_body(&st, req).await?;
        let rep = st.checker.fmt(&ip, &text).await.map_err(|e| e.response())?;
        Ok(json_resp(200, &rep))
    }
    .await)
}

async fn share(State(st): St, req: Request) -> Response<Body> {
    answer(async {
        let (ip, text, _) = check_body(&st, req).await?;
        let rep = st.checker.share(&ip, &text).await.map_err(|e| e.response())?;
        let mut out = fix_share(&st.public_url, &rep);
        if rep["ok"] != true {
            if let Ok(cr) = st.checker.check(&ip, &text).await {
                if cr["diagnostics"].as_array().is_some_and(|a| !a.is_empty()) {
                    out["diagnostics"] = cr["diagnostics"].clone();
                }
            }
            out["error"] = json!("the program does not check; fix its errors first");
        }
        Ok(json_resp(200, &out))
    }
    .await)
}

async fn open(State(st): St, req: Request) -> Response<Body> {
    answer(async {
        let (ip, text, url) = check_body(&st, req).await?;
        let arg = if url.is_empty() { text } else { url };
        if arg.trim().is_empty() {
            return Err(ApiError::new(400, "bad_request", "give {\"url\": \"<link, fragment or CID>\"}").response());
        }
        let rep = st.checker.open(&ip, &arg).await.map_err(|e| e.response())?;
        Ok(json_resp(200, &rep))
    }
    .await)
}

fn frag_ok(s: &str) -> bool {
    let Some((a, b)) = s.split_once('.') else { return false };
    let url64 = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    (8..=128).contains(&a.len()) && a.chars().all(url64) && !b.is_empty() && b.chars().all(url64)
}

/// `qrLinkOK`: only this site's own share forms.
fn qr_link_ok(public: &str, link: &str) -> bool {
    if link.len() > MAX_QR_LINK {
        return false;
    }
    if let Some(rest) = link.strip_prefix(&format!("{public}/p#")) {
        return frag_ok(rest);
    }
    if let Some(rest) = link.strip_prefix(&format!("{public}/p/")) {
        return (8..=128).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_alphanumeric());
    }
    false
}

async fn qr_route(State(st): St, req: Request) -> Response<Body> {
    let ip = st.ip(&req);
    let (ok, retry) = st.allow_check(&ip);
    if let Err(r) = limited(ok, retry, "qr") {
        return r;
    }
    let b = read_body(req.into_body(), MAX_QR_BODY).await;
    let text = match decode_body(&b, MAX_QR_BODY, &["text"], "struct { Text string \"json:\\\"text\\\"\" }", "") {
        Ok(v) => v[0].clone(),
        Err(r) => return r,
    };
    let p = &st.public_url;
    if !qr_link_ok(p, &text) {
        return ApiError::new(400, "bad_request", format!("a QR code is drawn only for this site's own share links ({p}/p#<CID>.<packed> or {p}/p/<CID>), at most {MAX_QR_LINK} characters")).response();
    }
    let bm = match qr::bitmap(&text) {
        Ok(b) => b,
        Err(e) => return ApiError::new(400, "bad_request", format!("could not draw the QR code: {e}")).response(),
    };
    let svg = qr::svg(&bm);
    let v = json!({
        "modules": bm.len() - 8,
        "qr": format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg)),
        "url": text,
    });
    let mut r = json_resp(200, &v);
    r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}

/// A method no Go pattern of the path takes: the catch-all (the static site) answers, 405 or 404.
async fn site_fallback(State(st): St, req: Request) -> Response<Body> {
    st.app.site.serve(req.method(), req.uri().path(), req.headers())
}

/// net/http's reason phrases where they are not hyper's (`StatusText`).
async fn go_reason(mut r: Response<Body>) -> Response<Body> {
    if r.status().as_u16() == 413 {
        r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Request Entity Too Large"));
    }
    r
}

// ---------- start ----------

fn valid_site_id(s: &str) -> bool {
    (16..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// `loadSite`: <data>/site.json, made the first time with a random id (or -site-id, which must agree).
fn load_site(data: &std::path::Path, over: &str) -> Result<String, String> {
    let path = data.join("site.json");
    if !over.is_empty() && !valid_site_id(over) {
        return Err("-site-id must be 16..64 characters of A-Z a-z 0-9 - _".into());
    }
    match std::fs::read(&path) {
        Ok(b) => {
            let id = serde_json::from_slice::<Value>(&b).ok().and_then(|v| v["site_id"].as_str().map(str::to_string)).filter(|s| valid_site_id(s));
            let Some(id) = id else { return Err(format!("{} is not a valid site file", path.display())) };
            if !over.is_empty() && over != id {
                return Err(format!("-site-id {over:?} differs from {} ({id:?}): a site keeps one id for good", path.display()));
            }
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = if over.is_empty() {
                let mut raw = [0u8; 16];
                getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
            } else {
                over.to_string()
            };
            std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
            let out = format!("{{\n  \"site_id\": {},\n  \"created\": {}\n}}", go_encode(&json!(id)), go_encode(&json!(store::rfc3339_now())));
            let tmp = data.join("site.json.tmp");
            std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn store_size(dir: &std::path::Path) -> i64 {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with(".tmp-")).filter_map(|e| e.metadata().ok()).map(|m| m.len() as i64).sum())
        .unwrap_or(0)
}

pub fn routes(app: &AppState) -> Router {
    let checker = Arc::new(Checker::new(app.check_slots.clone()));
    let _ = app.checker.set(checker.clone());
    let cfg = &app.cfg;
    let site_id = match load_site(&cfg.data, &cfg.flag("site-id", "")) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("beatscript-serve: {e}");
            std::process::exit(1);
        }
    };
    let al = cfg.flag("assetlinks-file", "");
    if !al.is_empty() {
        match std::fs::read(&al) {
            Ok(b) if json_valid(&b) => {}
            Ok(_) => {
                eprintln!("beatscript-serve: -assetlinks-file {al} is not valid JSON");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("beatscript-serve: -assetlinks-file: {e}");
                std::process::exit(2);
            }
        }
    }
    let n = cfg.flag_num("check-ip-min", 120);
    let st = Arc::new(Pub {
        app: app.clone(),
        checker,
        check_ip: (n > 0).then(|| Mutex::new(Limiter::new(n as usize, Duration::from_secs(60)))),
        site_id,
        public_url: cfg.public_url.trim_end_matches('/').to_string(),
        store_total: AtomicI64::new(store_size(app.store.dir())),
        store_cap: cfg.flag_num("store-max-mb", 256) << 20,
    });
    // one handler per path, the method decided here: a method no Go pattern takes is the static site's answer
    // (a MethodRouter fallback would add axum's own `Allow` to it)
    macro_rules! by_method {
        ($($m:ident => $h:expr),+) => {
            any(|State(st): St, req: Request| async move {
                $(if req.method() == Method::$m || (Method::$m == Method::GET && req.method() == Method::HEAD) {
                    return ($h)(State(st), req).await;
                })+
                site_fallback(State(st), req).await
            })
        };
    }
    Router::new()
        .route("/api/info", by_method!(GET => |s, _r| info(s)))
        .route("/.well-known/btw-site", by_method!(GET => |s, _r| site_route(s)))
        .route("/.well-known/beatscript-site", by_method!(GET => |s, _r| site_route(s)))
        .route("/.well-known/assetlinks.json", by_method!(GET => |s, _r| assetlinks(s)))
        .route("/api/instruments", by_method!(GET => instruments))
        .route("/context/v1.jsonld", by_method!(GET => |_s, r| ld_v1(r), OPTIONS => |_s, _r| ld_preflight()))
        .route("/context/beatscript-v1.jsonld", by_method!(GET => |_s, r| ld_beatscript_v1(r), OPTIONS => |_s, _r| ld_preflight()))
        .route("/context.jsonld", by_method!(GET => |_s, r| ld_latest(r), OPTIONS => |_s, _r| ld_preflight()))
        .route("/context", by_method!(GET => |_s, r| ld_latest(r)))
        .route("/api/programs/{cid}", by_method!(GET => get_program, PUT => put_program))
        .route("/api/check", by_method!(POST => check))
        .route("/api/generate", by_method!(POST => generate))
        .route("/api/station", by_method!(POST => station_route))
        .route("/api/fmt", by_method!(POST => fmt))
        .route("/api/share", by_method!(POST => share))
        .route("/api/open", by_method!(POST => open))
        .route("/api/qr", by_method!(POST => qr_route))
        .layer(axum::middleware::map_response(go_reason))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_allow_list() {
        let p = "https://beatscript.test";
        let frag = "zdpuAyGqNP8rmqxhRGzVeEyNKqjFbCpaXaR3yD6nQvZ5z5wdN.eJwrKUotyc9LUShKTUtMTVRIy8xJLVYoSMwrSUtNU0jLTE4tVkjKT0nNTE_RK0gtysvMT1WwVQAAcPoQ_g";
        assert!(qr_link_ok(p, &format!("{p}/p#{frag}")));
        assert!(qr_link_ok(p, &format!("{p}/p/zdpuAyGqNP8rmqxhRGzVeEyNKqjFbCpaXaR3yD6nQvZ5z5wdN")));
        for bad in [
            String::new(),
            "hello".into(),
            format!("https://evil.example/p#{frag}"),
            format!("http://beatscript.test/p#{frag}"),
            format!("{p}/p#{frag}~notes"),
            format!("{p}/#{frag}"),
            format!("{p}/p?x=1#{frag}"),
            format!("{p}/p/"),
            format!("{p}/p/abc"),
            format!("{p}/p/{}/x", "a".repeat(20)),
            format!("{p}/c/{frag}"),
            format!("{p}/pair#abcdefgh.ijklmnop"),
            format!("https://beatscript.test.evil.example/p#{frag}"),
            format!("{p}/p#{frag} "),
            "WIFI:S:x;T:WPA;P:y;;".into(),
        ] {
            assert!(!qr_link_ok(p, &bad), "{bad}");
        }
        let long = format!("{p}/p#{}.{}", &frag[..20], "A".repeat(MAX_QR_LINK - format!("{p}/p#").len() - 21));
        assert_eq!(long.len(), MAX_QR_LINK);
        assert!(qr_link_ok(p, &long));
        let bm = qr::bitmap(&long).unwrap();
        assert!(bm.len() - 8 >= 21);
    }

    #[test]
    fn shapes() {
        let rep = shape_check(&serde_json::from_str(&beatscript_api::check_json("tempo 120\ndeck A: kick x ~ x ~\n")).unwrap());
        let keys: Vec<&String> = rep.as_object().unwrap().keys().collect();
        assert_eq!(keys[..6], ["ok", "errors", "warnings", "diagnostics", "cid", "canonical"]);
        assert_eq!(clip("ab€".into(), 3), "ab…");
    }

    /// `/api/open` and the MCP `open` answer the text the link carried, formatted with its `#!` header and comments, beside
    /// the canonical text; `/api/share` makes the link on beatscript.app with that text in it.
    #[test]
    fn open_and_share_keep_the_header_and_comments() {
        let text = "#! name: Two Kicks\n# the only comment\ntempo 120\ndeck A: kick x ~ x ~\n";
        let sh = shape_share(&serde_json::from_str(&beatscript_api::share_json(text, None, false, "")).unwrap());
        assert!(sh["url"].as_str().unwrap().starts_with("https://beatscript.app/#z"), "{sh}");
        let o = shape_open(&serde_json::from_str(&beatscript_api::open_json(sh["url"].as_str().unwrap())).unwrap());
        assert_eq!(o["ok"], true);
        assert_eq!(o["text"], text);
        assert_eq!(o["canonical"], "tempo 120\ndeck A: kick x ~ x ~\n");
        assert_eq!(o["cid"], sh["cid"]);
        // a refused link has no text
        let bad = shape_open(&serde_json::from_str(&beatscript_api::open_json("zabc.AAAA")).unwrap());
        assert!(bad.get("text").is_none(), "{bad}");
    }

    /// Doc comments (project-datum 6662) reach a client: `/api/check` and the MCP `check` answer through these shapes,
    /// which keep a section's `doc` and the `docs` list (and add neither to a program without them).
    #[test]
    fn the_shapes_keep_doc_comments() {
        let text = "tempo 120\n# drop: the kick and the bass\nsection drop 1 bars {\n  deck A: kick x ~ x ~\n}\narrange drop\n";
        let raw: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
        for rep in [shape_check(&raw), crate::compose::engine::go_check_report(&raw)] {
            assert_eq!(rep["sections"][0]["doc"], "the kick and the bass", "{rep}");
            assert_eq!(rep["docs"], json!([{"kind": "section", "name": "drop", "doc": "the kick and the bass"}]));
        }
        let bare: Value = serde_json::from_str(&beatscript_api::check_json("tempo 120\nsection drop 1 bars {\n  deck A: kick x ~ x ~\n}\narrange drop\n")).unwrap();
        for rep in [shape_check(&bare), crate::compose::engine::go_check_report(&bare)] {
            assert!(rep.get("docs").is_none() && rep["sections"][0].get("doc").is_none(), "{rep}");
            assert_eq!(rep["cid"], raw["cid"], "one CID with and without the comment");
        }
    }

    /// A program's references (project-datum 6668, 6680) reach a client: `/api/check` and the MCP `check` keep `refs`,
    /// `expanded_cid` and `resolution` (with `expand`, each ref's `expansion`); a program with none has `expanded_cid` equal
    /// to its `cid` and neither list.
    #[test]
    fn the_shapes_keep_references() {
        let text = "tempo 124\nkey am\nsection drop 4 bars {\n  deck B: bass gen@2 techno seed=42 section=drop\n}\narrange drop\n";
        let raw: Value = serde_json::from_str(&beatscript_api::check_json_with(text, true)).unwrap();
        assert_eq!(raw["ok"], true, "{raw}");
        for rep in [shape_check(&raw), crate::compose::engine::go_check_report(&raw)] {
            assert_eq!(rep["refs"], raw["refs"], "{rep}");
            assert_eq!(rep["resolution"], raw["resolution"]);
            assert_eq!(rep["expanded_cid"], raw["expanded_cid"]);
            assert_ne!(rep["expanded_cid"], rep["cid"]);
            assert_eq!(rep["refs"][0]["kind"], "gen");
            assert!(rep["refs"][0]["expansion"].as_str().is_some_and(|e| !e.is_empty()), "{rep}");
            assert_eq!(rep["resolution"][0]["generator"], "seeded@2");
        }
        let bare: Value = serde_json::from_str(&beatscript_api::check_json("tempo 120\ndeck A: kick x ~ x ~\n")).unwrap();
        for rep in [shape_check(&bare), crate::compose::engine::go_check_report(&bare)] {
            assert!(rep.get("refs").is_none() && rep.get("resolution").is_none(), "{rep}");
            assert_eq!(rep["expanded_cid"], rep["cid"]);
        }
    }
}
