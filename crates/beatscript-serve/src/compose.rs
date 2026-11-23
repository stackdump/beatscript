//! P7: `POST /api/compose` and `/api/admin/*` (Go `internal/btwsite/compose`, `limits`, `server/wire.go`).
//!
//! A signed-in user's prompt goes to a text model (Gemini, or `-model fake`: Go's recorded fixtures), the answer goes
//! through the checker, and while it has errors the diagnostics go back to the model, a few rounds at most. What
//! comes out is a checked program stored content-addressed with its provenance, or a clean 422 with every attempt.
//! Around it: validation, the kill switch, per-user and per-address windows, the daily spend cap held in reserve
//! before each call and settled after it, and a log that never holds the key, a prompt or a program.
//!
//! The service is reachable from other modules (P6's `POST /api/device/compose`, P2's `/api/info`) through
//! `compose::get()`: `Composer::compose` (blocking) / `compose_async`, `Composer::status`.

pub mod decode;
pub mod engine;
pub mod gemini;
pub mod gojson;
pub mod limits;
pub mod model;
pub mod prompt;

use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::any;
use axum::Router;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use self::engine::{Engine, Rep};
use self::gojson::obj;
use self::limits::{Entry, Limiter};
use self::model::{Message, Model, ModelError, Redactor};
use self::prompt::{ExampleCand, Request as CReq};
use crate::{ApiError, AppState};

pub const PUBLIC_NOTICE: &str = "Everything generated here is published: the program, its CID, share link and rendered audio are stored publicly and released under CC0-1.0 (public domain), and anyone with the link can play, copy and reuse them. The prompt goes to a third-party model provider. Do not put anything private in a prompt.";

pub const NOT_CONFIGURED_HELP: &str = "compose is not configured: set the GEMINI_API_KEY environment variable of the btw-bitwrap service and restart it (BEATSCRIPT_MODEL=fake runs the recorded fixtures instead of a real model)";

/// The render version a URL handed out for a format names (docs/RUST-SERVER.md §5.1: beatscript-serve's Opus is `r2rs`, its
/// WAV the same bytes as Go's `r2`).
pub fn render_version(format: &str) -> &'static str {
    if format == "opus" {
        "r2rs"
    } else {
        "r2"
    }
}

// ---------- errors ----------

/// `api.Error` with `details`: `{"error":{"code","message","retry_after","details"}}`.
#[derive(Clone, Debug)]
pub struct Fail {
    pub err: ApiError,
    pub details: Option<Value>,
}

impl From<ApiError> for Fail {
    fn from(err: ApiError) -> Fail {
        Fail { err, details: None }
    }
}

impl Fail {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Fail {
        ApiError::new(status, code, message).into()
    }

    /// The error object as encoding/json writes `api.Error`.
    pub fn json(&self) -> Value {
        let mut e = Map::new();
        e.insert("code".into(), json!(self.err.code));
        e.insert("message".into(), json!(self.err.message));
        if self.err.retry_after > 0 {
            e.insert("retry_after".into(), json!(self.err.retry_after));
        }
        if let Some(d) = &self.details {
            e.insert("details".into(), d.clone());
        }
        Value::Object(e)
    }

    /// `api.WriteError` (`no_store`: compose's `writeErr`, which adds Cache-Control first).
    pub fn response(&self, no_store: bool) -> Response<Body> {
        let mut r = Response::builder().status(self.err.status).header(header::CONTENT_TYPE, "application/json; charset=utf-8");
        if no_store {
            r = r.header(header::CACHE_CONTROL, "no-store");
        }
        if self.err.retry_after > 0 {
            r = r.header(header::RETRY_AFTER, self.err.retry_after.to_string());
        }
        go_reason(r.body(crate::go_body(gojson::line(&json!({"error": self.json()})))).unwrap())
    }
}

/// net/http's reason phrases where hyper's differ (413 is "Request Entity Too Large" in Go).
pub fn go_reason(mut r: Response<Body>) -> Response<Body> {
    if r.status().as_u16() == 413 {
        if let Ok(p) = hyper::ext::ReasonPhrase::try_from(&b"Request Entity Too Large"[..]) {
            r.extensions_mut().insert(p);
        }
    }
    r
}

fn fail(status: u16, code: &str, msg: impl Into<String>) -> Fail {
    Fail::new(status, code, msg)
}

// ---------- the stored programs, as compose and MCP read and write them ----------

/// `api.Meta` as encoding/json writes it, then `based_on` (the text's `# from:` lines its save kept, gate::write_program).
pub fn go_meta(m: &Value) -> Value {
    let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
    let mut o = Map::new();
    o.insert("license".into(), json!(s("license")));
    o.insert("visibility".into(), json!(s("visibility")));
    for k in ["name", "composed_by", "forked_from"] {
        if !s(k).is_empty() {
            o.insert(k.into(), json!(s(k)));
        }
    }
    if let Some(c) = m["credits"].as_array().filter(|c| !c.is_empty()) {
        o.insert("credits".into(), Value::Array(c.iter().map(|x| json!(x.as_str().unwrap_or(""))).collect()));
    }
    for k in ["prompt_sha256", "author", "created", "source"] {
        if !s(k).is_empty() {
            o.insert(k.into(), json!(s(k)));
        }
    }
    // a recipe and its stored expansion name each other, and the expansion keeps what it was written out from (roadmap 6680)
    for k in ["expands_to", "expanded_from"] {
        if !s(k).is_empty() {
            o.insert(k.into(), json!(s(k)));
        }
    }
    if let Some(b) = m["based_on"].as_array().filter(|b| !b.is_empty()) {
        o.insert("based_on".into(), Value::Array(b.iter().filter_map(Value::as_str).map(|l| json!(l)).collect()));
    }
    Value::Object(o)
}

/// Store the expansion of the recipe `cid` (its canonical text, its meta, its check report `rep`) beside it (store.rs
/// `put_expansion`): the one path every program write takes (compose::put for the MCP and compose, `PUT /api/programs`, `PUT
/// /api/shares`). Nothing for a program with no references. One stored already (first write wins) is answered at once; else
/// it is written out under the checker's budget and timeout, charged to `owner`, and stored when its rights, its size and the
/// store's room allow. A panic is caught: the recipe stays stored, and the outcome says the expansion is not.
pub fn expansion_beside(app: &crate::App, eng: &Engine, owner: &str, cid: &str, canonical: &str, meta: &Value, rep: &Value) -> Option<crate::store::Outcome> {
    let x = rep["expanded_cid"].as_str().filter(|x| *x != cid && crate::store::is_cid(x))?.to_string();
    if let Some(o) = app.store.stored_expansion(&x) {
        return Some(o);
    }
    let (c, t, lic) = (cid.to_string(), canonical.to_string(), meta["license"].as_str().unwrap_or("").to_string());
    let plan = match eng.budgeted(owner, "expand", move || crate::store::plan_expansion(&c, &t, &lic)) {
        Ok(Some(p)) => p,
        Ok(None) => return None,
        Err(e) => return Some(crate::store::Outcome::skipped(&x, format!("it was not written out: {}", e.message))),
    };
    let cap = (app.cfg.flag_num("store-max-mb", 256).max(0) as u64) << 20;
    let room = cap.saturating_sub(app.store.bytes());
    let put = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.store.put_expansion(plan, cid, meta, app.cfg.max_program_bytes, Some(room))));
    Some(match put {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            eprintln!("beatscript-serve: the expansion of {cid} was not stored: {e}");
            crate::store::Outcome::skipped(&x, format!("it could not be stored: {e}"))
        }
        Err(_) => {
            eprintln!("beatscript-serve: storing the expansion of {cid} panicked");
            crate::store::Outcome::skipped(&x, "it could not be stored")
        }
    })
}

/// A stored program's answer (`{cid, text, meta}`) with what became of its expansion: `expanded_cid` and `expansion_license` when
/// it is stored (the licence is its first sharer's when it was stored already), `expanded_cid` and `expansion_note` (why) when not.
pub fn with_expansion(mut prog: Value, o: Option<&crate::store::Outcome>) -> Value {
    if let Some(o) = o {
        prog["expanded_cid"] = json!(o.cid);
        if o.stored {
            prog["expansion_license"] = json!(o.license);
        } else if let Some(w) = &o.why {
            prog["expansion_note"] = json!(format!("its expansion is not stored: {w}"));
        }
    }
    prog
}

/// [`expansion_beside`] for a program [`put`] stored.
fn beside(app: &crate::App, eng: &Engine, owner: &str, prog: Value, rep: &Value) -> Value {
    let (Some(cid), Some(text)) = (prog["cid"].as_str().map(str::to_string), prog["text"].as_str().map(str::to_string)) else { return prog };
    let o = expansion_beside(app, eng, owner, &cid, &text, &prog["meta"], rep);
    with_expansion(prog, o.as_ref())
}

/// A program the site has: (canonical text, meta, "store"). Examples first, as `Server.Get`.
pub fn stored(app: &crate::App, cid: &str) -> Option<(String, Value)> {
    if !crate::store::is_cid(cid) {
        return None;
    }
    if let Some(e) = app.examples.list.iter().find(|e| e.cid == cid) {
        let p = app.store.get(cid)?;
        return Some((p.text, app.examples.meta(e)));
    }
    let b = std::fs::read(app.cfg.programs.join(format!("{cid}.json"))).ok();
    if let Some(b) = b {
        let v: Value = serde_json::from_slice(&b).ok()?;
        let text = v["text"].as_str()?;
        if v["cid"].as_str() != Some(cid) || text.is_empty() || djlive::compile::program_cid(text) != cid {
            return None;
        }
        return Some((text.to_string(), v["meta"].clone()));
    }
    // a program held in memory (read-only store)
    app.store.get(cid).map(|p| (p.text, json!({"license": "CC0-1.0", "visibility": "public", "source": "put"})))
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

/// `programStore.Put`: checks `text`, refuses what the store refuses, stores the canonical text under its CID with
/// `meta` and the text's `# from:` lines as `meta.based_on` (first meta wins: an existing program is returned as it is).
/// Answers `{cid, text, meta}` and the report.
pub fn put(app: &crate::App, eng: &Engine, owner: &str, text: &str, meta: Value) -> Result<(Value, Value), Fail> {
    let max = app.cfg.max_program_bytes;
    if text.len() > max {
        return Err(fail(413, "too_large", format!("program is {} bytes; the limit is {max}", text.len())));
    }
    if text.trim().is_empty() {
        return Err(fail(400, "bad_request", "program text is empty"));
    }
    if crate::store::declares_not_public(text) {
        return Err(fail(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site"));
    }
    let rep = eng.check(owner, text)?;
    let r = Rep(&rep);
    if !r.ok() {
        let first = r.diagnostics().iter().find(|d| d["severity"] == "error").map(|d| format!("{}:{} {}", d["line"], d["col"], d["message"].as_str().unwrap_or(""))).unwrap_or_else(|| "see the diagnostics".into());
        return Err(Fail { err: ApiError::new(422, "bad_request", format!("the program has {} error(s): {first}", r.errors())), details: Some(json!({"check": rep})) });
    }
    if r.restricted() || r.uses_assets() {
        return Err(fail(422, "restricted", "programs that use restricted sounds or sample/loop assets are not stored on this site"));
    }
    let cid = r.cid().to_string();
    let canonical = r.canonical().to_string();
    if !crate::store::is_cid(&cid) || djlive::compile::program_cid(&canonical) != cid {
        return Err(fail(500, "internal", "the checker answered no usable CID"));
    }
    let path = app.cfg.programs.join(format!("{cid}.json"));
    if let Ok(b) = std::fs::read(&path) {
        if let Ok(v) = serde_json::from_slice::<Value>(&b) {
            if v["cid"].as_str() == Some(&cid) && v["text"].as_str() == Some(&canonical) {
                // (a recipe stored before its expansion was kept gets it now)
                let prog = beside(app, eng, owner, json!({"cid": cid, "text": canonical, "meta": go_meta(&v["meta"])}), &rep);
                return Ok((prog, rep));
            }
        }
    }
    let mut meta = meta;
    let set_default = |m: &mut Value, k: &str, v: String| {
        if m[k].as_str().unwrap_or("").is_empty() {
            m[k] = json!(v);
        }
    };
    set_default(&mut meta, "license", "CC0-1.0".into());
    set_default(&mut meta, "visibility", "public".into());
    set_default(&mut meta, "created", gojson::rfc3339(gojson::now().0));
    set_default(&mut meta, "source", "put".into());
    set_default(&mut meta, "name", header_name(text));
    // the storage gate: only Creative Commons, public, sample-free programs are stored. It reads the text as sent, so what it
    // admits carries the text's `# from:` lines (the canonical text has no comments): kept as meta.based_on
    let adm = beatscript_serve::gate::admit(text, meta["license"].as_str().unwrap_or(""), meta["visibility"].as_str().unwrap_or("")).map_err(|r| fail(r.status, r.code, r.message))?;
    if adm.cid() != cid {
        return Err(fail(500, "internal", "the checker answered no usable CID"));
    }
    meta["license"] = json!(adm.license());
    meta["visibility"] = json!("public");
    // the text is the only source of based_on (as gate::write_program has it): a caller's own never reaches the answer
    if let Some(o) = meta.as_object_mut() {
        o.remove("based_on");
    }
    if !adm.based_on().is_empty() {
        meta["based_on"] = json!(adm.based_on());
    }
    // its expansion, when it has references, is stored first, and the recipe names it (`expands_to`) only once it is stored
    let x = expansion_beside(app, eng, owner, &cid, &canonical, &go_meta(&meta), &rep);
    if let Some(o) = x.as_ref().filter(|o| o.stored) {
        meta["expands_to"] = json!(o.cid);
    }
    let prog = json!({"cid": cid, "text": canonical, "meta": go_meta(&meta)});
    if app.cfg.programs_readonly {
        let _ = app.store.put_canonical(&cid, &canonical);
        return Ok((with_expansion(prog, x.as_ref()), rep));
    }
    match beatscript_serve::gate::write_program(&app.cfg.programs, &adm, prog["meta"].clone(), gojson::to_string) {
        Ok(Some(_)) => Ok((with_expansion(prog, x.as_ref()), rep)),
        Ok(None) => match stored(app, &cid) {
            Some((t, m)) => Ok((with_expansion(json!({"cid": cid, "text": t, "meta": go_meta(&m)}), x.as_ref()), rep)),
            None => Err(fail(500, "internal", "the program could not be stored")),
        },
        Err(_) => Err(fail(500, "internal", "the program could not be stored")),
    }
}

// ---------- the service ----------

/// One finished compose (`api.ComposeResult`).
#[derive(Clone, Debug)]
pub struct Composed {
    pub program: String,
    pub text: String,
    pub cid: String,
    pub check: Value,
    pub attempts: Vec<Value>,
    pub model: String,
    pub forked_from: String,
    pub composed_by: String,
    pub header: Vec<String>,
    pub render_url: String,
    pub share_url: String,
    pub cost_usd: f64,
    pub usage: (i64, i64),
}

impl Composed {
    fn fields(&self, forked: bool) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("program".into(), json!(self.program));
        m.insert("text".into(), json!(self.text));
        m.insert("cid".into(), json!(self.cid));
        m.insert("check".into(), self.check.clone());
        m.insert("attempts".into(), Value::Array(self.attempts.clone()));
        m.insert("model".into(), json!(self.model));
        if forked {
            m.insert("forkedFrom".into(), json!(self.forked_from));
        }
        m.insert("license".into(), json!("CC0-1.0"));
        m.insert("notice".into(), json!(PUBLIC_NOTICE));
        m.insert("composedBy".into(), json!(self.composed_by));
        m.insert("header".into(), json!(self.header));
        m.insert("render_url".into(), json!(self.render_url));
        m.insert("share_url".into(), json!(self.share_url));
        m.insert("cost_usd".into(), gojson::num(self.cost_usd));
        m.insert("usage".into(), json!({"in": self.usage.0, "out": self.usage.1}));
        m
    }

    /// `POST /api/compose`'s body (`composeResponse`: forkedFrom null when there is none, last).
    pub fn web_json(&self) -> Value {
        let mut m = self.fields(false);
        m.insert("forkedFrom".into(), if self.forked_from.is_empty() { Value::Null } else { json!(self.forked_from) });
        Value::Object(m)
    }

    /// `POST /api/device/compose`'s body (`api.ComposeResult` as it is: forkedFrom a string, in place).
    pub fn device_json(&self) -> Value {
        Value::Object(self.fields(true))
    }
}

pub struct Options {
    pub public_url: String,
    pub model: String,
    pub api_key: String,
    pub endpoint: String,
    pub limits: limits::Config,
    pub max_prompt_chars: usize,
    pub max_program_bytes: usize,
    /// rounds after the first attempt; negative: none
    pub max_repair: i64,
    /// rounds that send a passing program's warnings back to the model; zero or negative: none
    pub max_refine: i64,
    pub max_output_tokens: i64,
    pub thinking: String,
}

pub struct Composer {
    app: std::sync::Weak<crate::App>,
    opt: Options,
    pub lim: Arc<Limiter>,
    red: Arc<Redactor>,
    model: Option<Box<dyn Model>>,
    model_name: String,
    by: String,
    think: String,
    eng: Engine,
    /// the built system prompt and the examples in it
    prompt: Mutex<Option<(String, Vec<String>)>>,
    cap_day: Mutex<String>,
    base_seen: Mutex<std::collections::HashMap<String, String>>,
}

struct Trace {
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    attempts: Vec<Value>,
    cid: String,
    forked_from: String,
    detail: String,
}

fn attempt(n: usize, ok: bool, errors: i64, diags: &[Value], tin: i64, tout: i64) -> Value {
    attempt_as(n, ok, false, errors, diags, tin, tout)
}

/// An attempt; `refine: true` (Go's `omitempty`: absent otherwise) marks a round that asked the model to clear the
/// warnings of a program that already passed.
fn attempt_as(n: usize, ok: bool, refine: bool, errors: i64, diags: &[Value], tin: i64, tout: i64) -> Value {
    let mut m = if refine {
        obj([("n", json!(n)), ("ok", json!(ok)), ("refine", json!(true)), ("errors", json!(errors))])
    } else {
        obj([("n", json!(n)), ("ok", json!(ok)), ("errors", json!(errors))])
    };
    if !diags.is_empty() {
        m["diagnostics"] = Value::Array(diags.to_vec());
    }
    let o = m.as_object_mut().unwrap();
    o.insert("tokens_in".into(), json!(tin));
    o.insert("tokens_out".into(), json!(tout));
    m
}

fn is_genre(g: &str) -> bool {
    let n = g.chars().count();
    (1..=40).contains(&n) && g.chars().all(|c| c.is_alphabetic() || c.is_numeric() || " +&/'.-".contains(c))
}

fn is_key(k: &str) -> bool {
    // (?i)^[a-g][#b]?(m|:[a-z]{3,12})?$ on the lower-cased key
    let b = k.as_bytes();
    if b.is_empty() || !(b'a'..=b'g').contains(&b[0]) {
        return false;
    }
    let i = 1;
    if matches!(b.get(i), Some(b'#') | Some(b'b')) {
        // "b" may be the flat or (as in "bm") nothing: try both readings
        if is_key_rest(&b[i + 1..]) {
            return true;
        }
    }
    is_key_rest(&b[i..])
}

fn is_key_rest(r: &[u8]) -> bool {
    r.is_empty() || r == b"m" || (r.first() == Some(&b':') && (4..=13).contains(&r.len()) && r[1..].iter().all(|c| c.is_ascii_lowercase()))
}

impl Composer {
    pub fn new(app: &AppState, opt: Options) -> Result<Composer, String> {
        let think = gemini::normalize_thinking(&opt.thinking).map_err(|e| format!("compose: {e}"))?;
        let clock: limits::Clock = Arc::new(gojson::now);
        let lim = Limiter::new(opt.limits.clone(), clock)?;
        let red = Arc::new(Redactor::new(&[&opt.api_key]));
        let mut name = opt.model.trim().to_string();
        let model: Option<Box<dyn Model>> = if name.eq_ignore_ascii_case("fake") {
            name = "fake".into();
            Some(Box::new(model::FakeModel::embedded()))
        } else if !opt.api_key.trim().is_empty() {
            let g = gemini::Gemini::new(&opt.api_key, &name, &opt.endpoint, opt.max_output_tokens, &think, red.clone())?;
            name = g.model_name().to_string();
            Some(Box::new(g))
        } else {
            if name.is_empty() {
                name = gemini::DEFAULT_MODEL.into();
            }
            None
        };
        let by = if name.starts_with("gemini") {
            name.clone()
        } else if name == "fake" {
            "fake-model".into()
        } else {
            format!("gemini-{name}")
        };
        Ok(Composer {
            app: Arc::downgrade(app),
            opt,
            lim,
            red,
            model,
            model_name: name,
            by,
            think,
            eng: Engine::over(app.checker.get().cloned()),
            prompt: Mutex::new(None),
            cap_day: Mutex::new(String::new()),
            base_seen: Mutex::new(Default::default()),
        })
    }

    fn app(&self) -> Arc<crate::App> {
        self.app.upgrade().expect("app")
    }

    pub fn configured(&self) -> bool {
        self.model.is_some()
    }

    fn logf(&self, m: &str) {
        eprintln!("beatscript-serve: {}", self.red.redact(m));
    }

    /// Whether a request was turned away by the spend cap today (UTC): `Status` and `/api/info` report it.
    pub fn cap_hit_today(&self) -> bool {
        *self.cap_day.lock().unwrap() == gojson::day(gojson::now().0)
    }

    fn note_spend(&self, e: Fail) -> Fail {
        if e.err.code == "spend_cap" {
            *self.cap_day.lock().unwrap() = gojson::day(gojson::now().0);
        }
        e
    }

    fn kill_reason(&self, k: limits::KillState) -> String {
        let mut parts = Vec::new();
        if k.env {
            parts.push(format!("{}=0 (or {}=0) is set in the service's environment (unset it and restart)", limits::ENV_ENABLED, limits::ENV_ENABLED_OLD));
        }
        if k.file {
            let where_ = match &self.opt.limits.data_dir {
                Some(d) => format!("{}/{}", d.display(), limits::FLAG_FILE),
                None => limits::FLAG_FILE.into(),
            };
            parts.push(format!("the flag file {where_} exists (delete it, or POST {{\"enabled\":true}} to /api/admin/compose)"));
        }
        format!("compose is switched off: {}", parts.join("; "))
    }

    /// `Status` (`api.ComposeStatus`): what `/api/info` reports (with `reason` dropped there; `public_reason` stays).
    pub fn status(&self) -> Value {
        let k = self.lim.kill();
        let (spent, closed) = self.lim.spend_state();
        let c = self.lim.config();
        let mut v = obj([
            ("enabled", json!(!k.disabled)),
            ("configured", json!(self.model.is_some())),
            ("model", json!(self.model_name)),
            ("user_per_hour", json!(c.user_hour)),
            ("user_per_day", json!(c.user_day)),
            ("ip_per_day", json!(c.ip_day)),
            ("spent_today_usd", gojson::num(spent)),
            ("daily_cap_usd", gojson::num(c.daily_spend_usd)),
        ]);
        let (reason, public) = if self.model.is_none() {
            (NOT_CONFIGURED_HELP.to_string(), "compose is not available on this site right now")
        } else if k.disabled {
            (self.kill_reason(k), "compose is switched off for now")
        } else if !closed.is_empty() {
            (closed, "compose is closed for now")
        } else if c.daily_spend_usd >= 0.0 && (spent >= c.daily_spend_usd || self.cap_hit_today()) {
            (format!("today's budget of ${:.2} is used up (or too little is left for another request); it resets at 00:00 UTC", c.daily_spend_usd), "today's compose budget is used up; it resets at 00:00 UTC")
        } else {
            (String::new(), "")
        };
        if !reason.is_empty() {
            v["reason"] = json!(reason);
            v["public_reason"] = json!(public);
        }
        v
    }

    // ---------- validation ----------

    fn clean(&self, mut r: CReq) -> Result<CReq, Fail> {
        r.prompt = r.prompt.replace('\0', "").trim().to_string();
        if r.prompt.is_empty() {
            return Err(fail(400, "bad_request", "prompt is required: say what you want to hear"));
        }
        let n = r.prompt.chars().count();
        if n > self.opt.max_prompt_chars {
            return Err(fail(413, "prompt_too_long", format!("the prompt is {n} characters; the limit is {}", self.opt.max_prompt_chars)));
        }
        r.genre = r.genre.trim().to_string();
        if !r.genre.is_empty() && !is_genre(&r.genre) {
            return Err(fail(400, "bad_request", "genre must be a short plain word or two"));
        }
        if r.bpm != 0.0 && !(r.bpm >= 40.0 && r.bpm <= 300.0) {
            return Err(fail(400, "bad_request", "bpm must be between 40 and 300"));
        }
        r.key = r.key.trim().to_lowercase();
        if !r.key.is_empty() && !is_key(&r.key) {
            return Err(fail(400, "bad_request", "key must look like am, c, f#m or d:dorian"));
        }
        r.length = r.length.trim().to_lowercase();
        if !r.length.is_empty() && prompt::length_words(&r.length).is_none() {
            let digits = (1..=3).contains(&r.length.len()) && r.length.bytes().all(|c| c.is_ascii_digit());
            if !digits {
                return Err(fail(400, "bad_request", "length must be \"short\", \"medium\", \"long\" or a number of bars"));
            }
            let n: i64 = r.length.parse().unwrap_or(0);
            if !(4..=256).contains(&n) {
                return Err(fail(400, "bad_request", "length in bars must be between 4 and 256"));
            }
        }
        if r.current.trim().is_empty() {
            r.current.clear();
        }
        if r.current.len() > self.opt.max_program_bytes {
            return Err(fail(413, "too_large", format!("the current program is {} bytes; the limit is {}", r.current.len(), self.opt.max_program_bytes)));
        }
        r.base_cid = r.base_cid.trim().to_string();
        if !r.base_cid.is_empty() && !crate::store::is_cid(&r.base_cid) {
            return Err(fail(400, "bad_request", "base_cid is not a program CID"));
        }
        Ok(r)
    }

    fn current_program(&self, ip: &str, text: &str) -> Result<String, Fail> {
        let rep = self.eng.check(ip, text)?;
        let why = prompt::vet_public_program(text, Some(&rep), false);
        if !why.is_empty() {
            return Err(fail(400, "bad_request", format!("the current program cannot be edited here ({why}): compose without it, or fix it first")));
        }
        Ok(prompt::strip_header(Rep(&rep).canonical()))
    }

    fn base_program(&self, ip: &str, cid: &str) -> Result<String, Fail> {
        let refuse = || fail(400, "bad_request", format!("base_cid {cid} is not a public program this site can edit"));
        let app = self.app();
        let Some((text, meta)) = stored(&app, cid) else { return Err(refuse()) };
        let vis = meta["visibility"].as_str().unwrap_or("");
        let lic = meta["license"].as_str().unwrap_or("");
        if (!vis.is_empty() && vis != "public") || (!lic.is_empty() && lic != "CC0-1.0") {
            return Err(refuse());
        }
        let key = format!("{cid}:{}", hex::encode(&Sha256::digest(text.as_bytes())[..8]));
        if let Some(hit) = self.base_seen.lock().unwrap().get(&key) {
            return if hit.is_empty() { Err(refuse()) } else { Ok(hit.clone()) };
        }
        let rep = self.eng.check(ip, &text)?;
        let base = if prompt::vet_public_program(&text, Some(&rep), false).is_empty() && Rep(&rep).cid() == cid { prompt::strip_header(Rep(&rep).canonical()) } else { String::new() };
        let mut seen = self.base_seen.lock().unwrap();
        if seen.len() >= 512 {
            seen.clear();
        }
        seen.insert(key, base.clone());
        if base.is_empty() {
            Err(refuse())
        } else {
            Ok(base)
        }
    }

    /// The system prompt (built once) and the names of the examples in it.
    fn system_prompt(&self) -> (String, Vec<String>) {
        if let Some(p) = self.prompt.lock().unwrap().clone() {
            return p;
        }
        let app = self.app();
        let mut cands = Vec::new();
        for e in &app.examples.list {
            if !e.license.eq_ignore_ascii_case("CC0-1.0") {
                continue;
            }
            let Some(text) = app.examples.text.get(&e.name) else { continue };
            if text.len() > self.opt.max_program_bytes {
                continue;
            }
            // a checker failure leaves the example out, as in Go
            let Ok(rep) = self.eng.check("", text) else { continue };
            let why = prompt::vet_public_program(text, Some(&rep), true);
            if !why.is_empty() {
                self.logf(&format!("compose: example {:?} left out of the prompt: {why}", e.name));
                continue;
            }
            cands.push(ExampleCand::new(&e.name, prompt::strip_header(text), Rep(&rep).sections()));
        }
        let chosen = prompt::pick_examples(cands, 9000);
        let sys = prompt::system_prompt(&self.eng.cheatsheet(), &self.eng.instruments(), &chosen);
        let p = (sys, chosen.into_iter().map(|c| c.name).collect());
        *self.prompt.lock().unwrap() = Some(p.clone());
        p
    }

    fn worst_case(&self, system: &str, msgs: &[Message]) -> f64 {
        self.lim.cost(prompt::estimate_tokens(system, msgs), self.opt.max_output_tokens)
    }

    fn with_attempts(&self, mut e: Fail, tr: &Trace, diags: Option<&[Value]>, last: &str) -> Fail {
        if e.details.is_none() && !tr.attempts.is_empty() {
            let d = diags.map_or(Value::Null, |d| Value::Array(d.to_vec()));
            e.details = Some(json!({"attempts": tr.attempts, "cost_usd": gojson::num(tr.cost), "diagnostics": d, "last_text": last}));
        }
        e
    }

    fn model_api_error(me: &ModelError) -> Fail {
        let msg = match me.kind.as_str() {
            model::KIND_RATE_LIMITED => return fail(502, "model_error", "the model service is busy right now; try again in a minute").err.retry(me.retry_after).into(),
            model::KIND_BLOCKED => "the model declined this request; try rewording the prompt".to_string(),
            model::KIND_AUTH => "the model service refused this site's credentials; the operator has been logged an error".to_string(),
            model::KIND_TRANSPORT => "the model service did not answer in time; try again".to_string(),
            _ => {
                let mut m = "the model service returned an error".to_string();
                if me.status != 0 {
                    m += &format!(" (HTTP {})", me.status);
                }
                format!("{m}; try again")
            }
        };
        fail(502, "model_error", msg)
    }

    /// `Compose`: blocking (model calls and the checker), so call it from a blocking thread (`compose_async` does).
    /// `uid` is the signed-in user (Go's `api.User.ID`), `ip` what `crate::client_ip` gives.
    pub fn compose(&self, uid: &str, ip: &str, req: CReq) -> Result<Composed, Fail> {
        if uid.is_empty() {
            return Err(fail(401, "auth_required", "sign in to compose"));
        }
        // the capability check (caps.rs): the server's, whatever grant the client holds; a no-op unless caps.json enforces
        if let Err(m) = crate::caps::require(uid, "compose") {
            return Err(fail(403, "capability_required", m));
        }
        let started = std::time::Instant::now();
        let (sum, head) = limits::prompt_hash(&req.prompt);
        let mut ent = Entry {
            user: uid.into(),
            ip_hash: self.lim.hash_ip(ip),
            prompt_chars: req.prompt.chars().count() as i64,
            prompt_sha: sum,
            prompt_head: self.red.redact(&head),
            model: self.model_name.clone(),
            ..Default::default()
        };
        let mut tr = Trace { tokens_in: 0, tokens_out: 0, cost: 0.0, attempts: Vec::new(), cid: String::new(), forked_from: String::new(), detail: String::new() };
        let res = self.run(uid, ip, req, &mut tr);
        ent.tokens_in = tr.tokens_in;
        ent.tokens_out = tr.tokens_out;
        ent.cost_usd = tr.cost;
        ent.attempts = tr.attempts.len() as i64;
        ent.cid = tr.cid.clone();
        ent.forked_from = tr.forked_from.clone();
        ent.detail = model::clip(&self.red.redact(&tr.detail), 200);
        ent.ms = started.elapsed().as_millis() as i64;
        ent.outcome = match &res {
            Ok(_) => "ok".into(),
            Err(e) => e.err.code.clone(),
        };
        self.lim.record(&ent);
        res
    }

    /// `compose` on a blocking thread. It runs to the end even if the caller goes away (what it spent is recorded).
    pub async fn compose_async(self: &Arc<Self>, uid: String, ip: String, req: CReq) -> Result<Composed, Fail> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.compose(&uid, &ip, req)).await.unwrap_or_else(|_| Err(fail(500, "internal", "compose failed")))
    }

    fn run(&self, uid: &str, ip: &str, req: CReq, tr: &mut Trace) -> Result<Composed, Fail> {
        let req = self.clean(req)?;
        let mut base = String::new();
        if !req.base_cid.is_empty() {
            base = self.base_program(ip, &req.base_cid)?;
            tr.forked_from = req.base_cid.clone();
        } else if !req.current.is_empty() {
            base = self.current_program(ip, &req.current)?;
        }
        if self.lim.kill().disabled {
            return Err(fail(503, "disabled", "compose is switched off for now"));
        }
        let Some(model) = &self.model else {
            return Err(fail(503, "not_configured", "compose is not available: the model is not configured on this site"));
        };
        let (system, _) = self.system_prompt();
        let mut msgs = vec![Message { role: "user", text: prompt::user_message(&req, &base, self.opt.max_program_bytes) }];
        let mut rsv = Some(self.lim.begin(uid, ip, self.worst_case(&system, &msgs)).map_err(|e| self.note_spend(e.into()))?);
        let rounds = 1 + self.opt.max_repair.max(0) as usize;
        let mut last_diags: Option<Vec<Value>> = None;
        let mut last_text = String::new();
        for n in 1..=rounds {
            if rsv.is_none() {
                match self.lim.reserve(self.worst_case(&system, &msgs)) {
                    Ok(r) => rsv = Some(r),
                    Err(e) => return Err(self.with_attempts(self.note_spend(e.into()), tr, last_diags.as_deref(), &last_text)),
                }
            }
            if self.lim.kill().disabled {
                rsv.take().unwrap().release();
                return Err(self.with_attempts(fail(503, "disabled", "compose is switched off for now"), tr, last_diags.as_deref(), &last_text));
            }
            let (text, mut use_, me) = model.generate(&system, &msgs);
            let mut cost = self.lim.cost(use_.tin, use_.tout);
            if me.as_ref().is_some_and(|m| m.kind == model::KIND_TRANSPORT) && use_.tin == 0 && use_.tout == 0 {
                use_.tin = prompt::estimate_tokens(&system, &msgs);
                if me.as_ref().unwrap().aborted {
                    use_.tout = self.opt.max_output_tokens;
                }
                cost = self.lim.cost(use_.tin, use_.tout);
            }
            rsv.take().unwrap().settle(cost, use_.tin, use_.tout);
            tr.tokens_in += use_.tin;
            tr.tokens_out += use_.tout;
            tr.cost += cost;

            let mut prog = String::new();
            let mut report: Option<Value> = None;
            let mut diags: Vec<Value> = Vec::new();
            let mut model_tx = String::new();
            match &me {
                Some(m) if m.kind == model::KIND_MAX_TOKENS => {
                    diags = vec![prompt::synthetic_diag("your reply was cut off because it ran out of output space before the program ended", "write a shorter program: fewer bars, fewer sections, fewer parts")];
                    model_tx = "[the reply was cut off before the program ended]".into();
                }
                Some(m) => {
                    tr.detail = m.to_string();
                    tr.attempts.push(attempt(n, false, 1, &[], use_.tin, use_.tout));
                    return Err(self.with_attempts(Self::model_api_error(m), tr, None, ""));
                }
                None => {
                    prog = prompt::extract_program(&text);
                    let max = self.opt.max_program_bytes;
                    if prog.is_empty() {
                        diags = vec![prompt::synthetic_diag("your reply held no program", "reply with the BeatScript program only, starting with tempo and key")];
                        model_tx = "[an empty reply]".into();
                    } else if prog.len() > max {
                        diags = vec![prompt::synthetic_diag(&format!("the program is {} bytes; the limit is {max}", prog.len()), "write a shorter program: fewer bars, sections and parts, fewer comments")];
                        model_tx = format!("[a program of {} bytes, over the limit of {max}]", prog.len());
                    } else {
                        let rep = self.eng.check(ip, &prog)?;
                        model_tx = prog.clone();
                        let r = Rep(&rep);
                        if !r.ok() {
                            diags = r.diagnostics();
                        } else if r.restricted() || r.uses_assets() {
                            diags = vec![prompt::synthetic_diag(
                                "the program uses restricted sounds or asset statements (use restricted / sample / loop), which are not allowed here",
                                "use only the instruments and kits in the INSTRUMENTS list",
                            )];
                        } else {
                            let why = prompt::header_claim(&prog);
                            if !why.is_empty() {
                                diags = vec![prompt::synthetic_diag(&format!("the program carries a #! header line that says it is {why}; everything here is public CC0"), "write no #! lines at all")];
                            }
                        }
                        report = Some(rep);
                    }
                }
            }
            if diags.is_empty() {
                if let Some(rep) = report.filter(|r| Rep(r).ok()) {
                    let warn = Rep(&rep).diagnostics();
                    tr.attempts.push(attempt(n, true, 0, &warn, use_.tin, use_.tout));
                    msgs.push(Message { role: "model", text: model_tx });
                    let (prog, rep) = self.refine(ip, &system, msgs, prog, rep, tr);
                    return self.finish(ip, &req, &prog, rep, tr);
                }
            }
            let errs = diags.iter().filter(|d| matches!(d["severity"].as_str().unwrap_or(""), "" | "error")).count() as i64;
            tr.attempts.push(attempt(n, false, errs, &diags, use_.tin, use_.tout));
            if n < rounds {
                msgs.push(Message { role: "model", text: model_tx });
                msgs.push(Message { role: "user", text: prompt::repair_message(&diags) });
            }
            last_diags = Some(diags);
            last_text = prog;
        }
        let mut e = fail(422, "unrepairable", format!("the model could not write a program that passes the checker in {} attempts", tr.attempts.len()));
        e.details = Some(json!({
            "attempts": tr.attempts,
            "cost_usd": gojson::num(tr.cost),
            "diagnostics": last_diags.map_or(Value::Null, Value::Array),
            "last_text": last_text,
        }));
        Err(e)
    }

    /// Send the warnings of a program that already passed back to the model, up to `max_refine` rounds. It never
    /// makes the result worse: a reply is kept only when it passes everything the first program passed (checker OK,
    /// no restricted sounds or assets, no `#!` header claim, within the size cap) and has fewer warnings. Anything
    /// else (a model error, a refused reservation, the kill switch, a worse reply) ends the refining and the best
    /// program so far is the result. Every call is reserved, paid for and recorded as an attempt with `refine: true`.
    fn refine(&self, ip: &str, system: &str, mut msgs: Vec<Message>, mut prog: String, mut rep: Value, tr: &mut Trace) -> (String, Value) {
        let Some(model) = &self.model else { return (prog, rep) };
        for _ in 0..self.opt.max_refine.max(0) {
            if Rep(&rep).warnings() <= 0 || self.lim.kill().disabled {
                break;
            }
            let mut ask = msgs.clone();
            ask.push(Message { role: "user", text: prompt::refine_message(&Rep(&rep).diagnostics()) });
            let Ok(rsv) = self.lim.reserve(self.worst_case(system, &ask)) else { break };
            let (text, mut use_, me) = model.generate(system, &ask);
            if me.as_ref().is_some_and(|m| m.kind == model::KIND_TRANSPORT) && use_.tin == 0 && use_.tout == 0 {
                use_.tin = prompt::estimate_tokens(system, &ask);
                if me.as_ref().unwrap().aborted {
                    use_.tout = self.opt.max_output_tokens;
                }
            }
            let cost = self.lim.cost(use_.tin, use_.tout);
            rsv.settle(cost, use_.tin, use_.tout);
            tr.tokens_in += use_.tin;
            tr.tokens_out += use_.tout;
            tr.cost += cost;
            let n = tr.attempts.len() + 1;
            if let Some(m) = &me {
                tr.attempts.push(attempt_as(n, false, true, 1, &[], use_.tin, use_.tout));
                self.logf(&format!("compose: refine round ended by the model: {}", self.red.redact(&m.to_string())));
                break;
            }
            let cand = prompt::extract_program(&text);
            if cand.is_empty() || cand.len() > self.opt.max_program_bytes {
                tr.attempts.push(attempt_as(n, false, true, 1, &[], use_.tin, use_.tout));
                break;
            }
            let Ok(r2) = self.eng.check(ip, &cand) else {
                tr.attempts.push(attempt_as(n, false, true, 1, &[], use_.tin, use_.tout));
                break;
            };
            let r = Rep(&r2);
            let diags = r.diagnostics();
            let errs = diags.iter().filter(|d| matches!(d["severity"].as_str().unwrap_or(""), "" | "error")).count() as i64;
            let better = r.ok() && !r.restricted() && !r.uses_assets() && prompt::header_claim(&cand).is_empty() && r.warnings() < Rep(&rep).warnings();
            tr.attempts.push(attempt_as(n, better, true, errs, &diags, use_.tin, use_.tout));
            if !better {
                break;
            }
            msgs = ask;
            msgs.push(Message { role: "model", text: cand.clone() });
            prog = cand;
            rep = r2;
        }
        (prog, rep)
    }

    fn finish(&self, ip: &str, req: &CReq, prog: &str, rep: Value, tr: &mut Trace) -> Result<Composed, Fail> {
        let app = self.app();
        let r = Rep(&rep);
        let cid = r.cid().to_string();
        let meta = json!({
            "license": "CC0-1.0", "visibility": "public", "composed_by": self.by, "forked_from": req.base_cid,
            "prompt_sha256": hex::encode(Sha256::digest(req.prompt.as_bytes())), "created": gojson::rfc3339(gojson::now().0), "source": "compose",
        });
        let (p, _) = put(&app, &self.eng, ip, prog, meta)?;
        if p["cid"].as_str() != Some(cid.as_str()) {
            self.logf(&format!("compose: store CID {} differs from checked CID {cid}", p["cid"]));
        }
        tr.cid = cid.clone();
        let mut header = vec!["#! license: CC0-1.0".to_string(), format!("#! composed-by: {}", self.by)];
        if !req.base_cid.is_empty() {
            header.push(format!("#! forked-from: {}", req.base_cid));
        }
        Ok(Composed {
            program: r.canonical().to_string(),
            text: prog.to_string(),
            cid: cid.clone(),
            check: rep.clone(),
            attempts: tr.attempts.clone(),
            model: self.model_name.clone(),
            forked_from: req.base_cid.clone(),
            composed_by: self.by.clone(),
            header,
            render_url: format!("/api/render/{cid}?format=opus&v={}", render_version("opus")),
            share_url: self.share_url(ip, &rep),
            cost_usd: tr.cost,
            usage: (tr.tokens_in, tr.tokens_out),
        })
    }

    fn share_url(&self, ip: &str, rep: &Value) -> String {
        let origin = self.opt.public_url.trim_end_matches('/');
        let sr = self.eng.share(ip, Rep(rep).canonical()).unwrap_or(Value::Null);
        if sr["ok"] == true {
            if let Some(u) = sr["url"].as_str().filter(|u| !u.is_empty()) {
                return match u.find('#') {
                    Some(i) if !origin.is_empty() => format!("{origin}/{}", &u[i..]),
                    _ => u.to_string(),
                };
            }
        }
        if !origin.is_empty() {
            return format!("{origin}/#{}", Rep(rep).cid());
        }
        String::new()
    }

    fn kill_json(&self) -> Map<String, Value> {
        let k = self.lim.kill();
        let mut m = Map::new();
        m.insert("disabled_by_env".into(), json!(k.env));
        m.insert("disabled_by_file".into(), json!(k.file));
        m.insert("enabled".into(), json!(!k.disabled));
        m
    }

    /// `GET /api/admin/stats`'s body (a Go map: keys sorted).
    pub fn admin_stats(&self) -> Value {
        let (_, names) = if self.prompt.lock().unwrap().is_some() { self.system_prompt() } else { (String::new(), Vec::new()) };
        let mut m = self.kill_json();
        m.insert("status".into(), self.status());
        m.insert("limits".into(), self.lim.stats());
        m.insert("model".into(), json!(self.model_name));
        m.insert("thinking".into(), json!(self.think));
        m.insert("max_output_tokens".into(), json!(self.opt.max_output_tokens));
        m.insert("max_repair".into(), json!(self.opt.max_repair.max(0)));
        m.insert("max_refine".into(), json!(self.opt.max_refine.max(0)));
        m.insert("prompt_examples".into(), if names.is_empty() { Value::Null } else { json!(names) });
        gojson::sorted(Value::Object(m))
    }
}

static COMPOSER: OnceLock<Arc<Composer>> = OnceLock::new();

/// The compose service, once `routes` has built it (P6's device compose, P2's `/api/info`).
pub fn get() -> Option<Arc<Composer>> {
    COMPOSER.get().cloned()
}

// ---------- HTTP ----------

/// `isJSON`: `mime.ParseMediaType(Content-Type)` is `application/json`.
pub fn is_json(h: &HeaderMap) -> bool {
    let Some(ct) = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else { return false };
    let mut parts = ct.split(';');
    let mt = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if mt != "application/json" {
        return false;
    }
    let tok = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?=".contains(&c));
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let Some((k, v)) = p.split_once('=') else { return false };
        let v = v.trim();
        if !tok(k.trim()) || !(tok(v) || (v.len() >= 2 && v.starts_with('"') && v.ends_with('"'))) {
            return false;
        }
    }
    true
}

fn json_ok(v: &Value) -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(crate::go_body(gojson::line(v)))
        .unwrap()
}

/// Read the body, at most `limit + 1` bytes.
async fn read_body(body: Body, limit: usize) -> Vec<u8> {
    use http_body_util_lite::collect_limited;
    collect_limited(body, limit + 1).await
}

mod http_body_util_lite {
    use axum::body::Body;
    use futures_util::StreamExt;

    pub async fn collect_limited(body: Body, max: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = body.into_data_stream();
        while let Some(c) = s.next().await {
            let Ok(c) = c else { break };
            out.extend_from_slice(&c);
            if out.len() >= max {
                out.truncate(max);
                break;
            }
        }
        out
    }
}

/// Go's `jsonErrWords` for the compose body, from what the decoder says about it.
fn decode_compose_body(b: &[u8], max_body: usize) -> Result<CReq, Fail> {
    let too_large = || fail(413, "too_large", format!("the request body is over {max_body} bytes"));
    let bad = |w: String| fail(400, "bad_request", format!("the body is not the JSON compose expects: {w}"));
    let (s, e) = match decode::body(b, max_body) {
        decode::Body::Ok(s, e) => (s, e),
        decode::Body::Empty => return Err(fail(400, "bad_request", r#"send a JSON body like {"prompt":"dark techno 128"}"#)),
        decode::Body::Syntax(o) => return Err(bad(format!("syntax error at byte {o}"))),
        decode::Body::UnexpectedEof => return Err(bad("invalid JSON".into())),
        decode::Body::TooLarge => return Err(too_large()),
        decode::Body::Trailing => {
            // the value decodes first: its errors come before the trailing data's
            return match decode_value(&b[..b.len().min(max_body)], true) {
                Err(f) => Err(f),
                Ok(_) => Err(fail(400, "bad_request", "the body must be one JSON object")),
            };
        }
    };
    decode_value(&b[s..e], false)
}

fn decode_value(b: &[u8], prefix: bool) -> Result<CReq, Fail> {
    let bad = |w: String| fail(400, "bad_request", format!("the body is not the JSON compose expects: {w}"));
    let v: Value = if prefix {
        let mut de = serde_json::Deserializer::from_slice(b).into_iter::<Value>();
        match de.next() {
            Some(Ok(v)) => v,
            _ => return Err(bad("invalid JSON".into())),
        }
    } else {
        serde_json::from_slice(b).map_err(|_| bad("invalid JSON".into()))?
    };
    let mut r = CReq::default();
    let m = match v {
        Value::Null => return Ok(r),
        Value::Object(m) => m,
        _ => return Err(bad("field \"\" has the wrong type".into())),
    };
    const FIELDS: [&str; 7] = ["prompt", "genre", "bpm", "key", "length", "base_cid", "current"];
    let mut saved: Option<Fail> = None;
    for (k, x) in m {
        let Some(f) = FIELDS.iter().find(|f| f.eq_ignore_ascii_case(&k)) else {
            saved.get_or_insert_with(|| bad(format!("unknown field {}", serde_json::to_string(&k).unwrap())));
            continue;
        };
        let wrong = || bad(format!("field \"{f}\" has the wrong type"));
        match *f {
            "bpm" => match x {
                Value::Null => {}
                Value::Number(n) => r.bpm = n.as_f64().unwrap_or(0.0),
                _ => {
                    saved.get_or_insert_with(wrong);
                }
            },
            "length" => match x {
                Value::Null => {}
                Value::String(s) => r.length = s,
                Value::Number(n) => r.length = n.to_string(),
                _ => return Err(bad("invalid JSON".into())),
            },
            _ => match x {
                Value::Null => {}
                Value::String(s) => match *f {
                    "prompt" => r.prompt = s,
                    "genre" => r.genre = s,
                    "key" => r.key = s,
                    "base_cid" => r.base_cid = s,
                    _ => r.current = s,
                },
                _ => {
                    saved.get_or_insert_with(wrong);
                }
            },
        }
    }
    match saved {
        Some(e) => Err(e),
        None => Ok(r),
    }
}

fn peer_ip(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

/// `requireUser` for compose: Go's `UserFrom` (cookie, then bearer; a `BtwDevice` header is not a user here).
pub fn web_user(app: &crate::App, h: &HeaderMap) -> Result<Option<String>, Fail> {
    let Some(a) = app.authsvc.get() else { return Ok(None) };
    let cookie = beatscript_serve::playlists::auth::cookie_value(h.get_all("cookie").iter().filter_map(|v| v.to_str().ok()), "btw_session");
    let authz = h.get("authorization").and_then(|v| v.to_str().ok()).filter(|v| !v.starts_with("BtwDevice "));
    match a.verify.caller(cookie, authz, "", "", b"") {
        Ok(c) => Ok(c.map(|c| c.uid)),
        Err(e) => Err(fail(e.status, e.code, e.message)),
    }
}

pub fn auth_help(public: &str) -> String {
    format!("sign in at {public} (wallet or browser key), press \"MCP token\", and send it as `Authorization: Bearer btw_…`")
}

async fn handle_compose(State(st): State<Arc<Http>>, req: Request) -> Response<Body> {
    let app = &st.app;
    // the gate: no user, a 401 before anything else about the service is said
    let uid = match web_user(app, req.headers()) {
        Err(e) => return e.response(false),
        Ok(None) => return fail(401, "auth_required", format!("authentication required: {}", auth_help(&app.cfg.public_url))).response(false),
        Ok(Some(u)) => u,
    };
    let c = &st.composer;
    if !is_json(req.headers()) {
        return fail(415, "bad_request", "send the body as JSON with Content-Type: application/json").response(true);
    }
    let max_body = c.opt.max_prompt_chars * 6 + c.opt.max_program_bytes * 6 + 4096;
    let ip = peer_ip(&req);
    let b = read_body(req.into_body(), max_body).await;
    let body = match decode_compose_body(&b, max_body) {
        Ok(b) => b,
        Err(e) => return e.response(true),
    };
    match c.compose_async(uid, ip, body).await {
        Ok(r) => json_ok(&r.web_json()),
        Err(e) => e.response(true),
    }
}

async fn handle_admin(State(st): State<Arc<Http>>, req: Request) -> Response<Body> {
    let c = &st.composer;
    if st.admin_token.is_empty() {
        return fail(503, "not_configured", "the admin endpoints are closed: BEATSCRIPT_ADMIN_TOKEN is not set").response(true);
    }
    let authz = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
    let tok = authz.strip_prefix("Bearer ").unwrap_or(authz).trim();
    let want = Sha256::digest(st.admin_token.as_bytes());
    let got = Sha256::digest(tok.as_bytes());
    if !authz.starts_with("Bearer ") || tok.is_empty() || !bool::from(subtle::ConstantTimeEq::ct_eq(got.as_slice(), want.as_slice())) {
        let mut r = fail(401, "auth_required", "admin token required").response(true);
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"btw admin\""));
        return r;
    }
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    match path.strip_suffix('/').unwrap_or(&path) {
        "/api/admin/compose" => {
            if method != Method::POST {
                let mut r = fail(405, "bad_request", "use POST").response(true);
                r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
                return r;
            }
            if !is_json(req.headers()) {
                return fail(415, "bad_request", "send the body as JSON with Content-Type: application/json").response(true);
            }
            let b = read_body(req.into_body(), 1024).await;
            let want = || fail(400, "bad_request", r#"send {"enabled":true} or {"enabled":false}"#);
            let enabled = match decode::body(&b, 1024) {
                decode::Body::Ok(s, e) => match serde_json::from_slice::<Value>(&b[s..e]) {
                    Ok(Value::Object(m)) => {
                        let mut en = None;
                        let mut bad = false;
                        for (k, v) in m {
                            if !k.eq_ignore_ascii_case("enabled") {
                                bad = true;
                                break;
                            }
                            match v {
                                Value::Bool(x) => en = Some(x),
                                Value::Null => {}
                                _ => bad = true,
                            }
                        }
                        if bad {
                            None
                        } else {
                            en
                        }
                    }
                    _ => None,
                },
                decode::Body::Trailing => {
                    let mut de = serde_json::Deserializer::from_slice(&b).into_iter::<Value>();
                    match de.next() {
                        Some(Ok(Value::Object(m))) if m.len() == 1 && m.iter().all(|(k, v)| k.eq_ignore_ascii_case("enabled") && v.is_boolean()) => {
                            return fail(400, "bad_request", "the body must be one JSON object").response(true);
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some(enabled) = enabled else { return want().response(true) };
            if let Err(e) = c.lim.set_enabled(enabled) {
                return fail(500, "internal", format!("could not change the switch: {}", c.red.redact(&e.to_string()))).response(true);
            }
            c.logf(&format!("compose: switched {} through the admin endpoint", if enabled { "on" } else { "off" }));
            let mut m = c.kill_json();
            m.insert("status".into(), c.status());
            json_ok(&gojson::sorted(Value::Object(m)))
        }
        "/api/admin/beta" => {
            let b = crate::beta::get(&st.app);
            let q = req.uri().query().unwrap_or("").to_string();
            let qv = |k: &str| q.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
            if method == Method::GET || method == Method::HEAD {
                if qv("format") == "csv" {
                    return Response::builder().header(header::CONTENT_TYPE, "text/csv; charset=utf-8").header(header::CACHE_CONTROL, "no-store").body(Body::from(b.csv(&qv("status")))).unwrap();
                }
                return match b.act("list", "", &qv("status")) { Ok(v) => json_ok(&v), Err(e) => fail(e.status, &e.code, e.message).response(true) };
            }
            if method != Method::POST {
                let mut r = fail(405, "bad_request", "use GET or POST").response(true);
                r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD, POST"));
                return r;
            }
            let body = read_body(req.into_body(), 2048).await;
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            match b.act(v["action"].as_str().unwrap_or(""), v["email"].as_str().unwrap_or(""), "") {
                Ok(v) => json_ok(&v),
                Err(e) => fail(e.status, &e.code, e.message).response(true),
            }
        }
        "/api/admin/caps" => {
            let Some(caps) = crate::caps::get() else { return fail(503, "not_configured", "capabilities are not set up").response(true) };
            let post = method == Method::POST;
            if !post && method != Method::GET && method != Method::HEAD {
                let mut r = fail(405, "bad_request", "use GET or POST").response(true);
                r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD, POST"));
                return r;
            }
            let b = if post { Some(read_body(req.into_body(), 4096).await) } else { None };
            match caps.admin(b.as_deref()) {
                Ok(v) => json_ok(&v),
                Err((st, m)) => fail(st, if st == 500 { "internal" } else { "bad_request" }, m).response(true),
            }
        }
        "/api/admin/stats" => {
            if method != Method::GET && method != Method::HEAD {
                let mut r = fail(405, "bad_request", "use GET").response(true);
                r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
                return r;
            }
            json_ok(&c.admin_stats())
        }
        _ => fail(404, "not_found", "no such admin endpoint").response(true),
    }
}

/// `/api/admin` (no slash): ServeMux's redirect to the subtree, as `http.Redirect` writes it.
async fn admin_redirect(req: Request) -> Response<Body> {
    let loc = match req.uri().query() {
        Some(q) => format!("/api/admin/?{q}"),
        None => "/api/admin/".to_string(),
    };
    let m = req.method();
    let mut r = Response::builder().status(307);
    if m == Method::GET || m == Method::HEAD {
        r = r.header(header::CONTENT_TYPE, "text/html; charset=utf-8");
    }
    r = r.header(header::LOCATION, &loc);
    let body = match *m {
        Method::GET => Body::from(format!("<a href=\"{}\">Temporary Redirect</a>.\n\n", crate::site::html_escape(&loc))),
        Method::HEAD => crate::content::nothing(),
        _ => Body::empty(),
    };
    r.body(body).unwrap()
}

/// `POST /api/compose`; any other method is Go's catch-all's (the static site) to answer.
async fn compose_or_site(st: State<Arc<Http>>, req: Request) -> Response<Body> {
    if req.method() == Method::POST {
        return handle_compose(st, req).await;
    }
    st.app.site.serve(req.method(), req.uri().path(), req.headers())
}

struct Http {
    app: AppState,
    composer: Arc<Composer>,
    admin_token: String,
}

fn parse_bool01(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "t" => Some(true),
        "0" | "false" | "no" | "off" | "f" => Some(false),
        _ => None,
    }
}

/// The options from Go's flags (`server/config.go`): `-model`, `-thinking`, `-compose-user-hour`, `-compose-user-day`,
/// `-compose-ip-day`, `-daily-spend-usd`, `-price-in-per-m`, `-price-out-per-m`, `-max-prompt-chars`,
/// `-max-program-bytes`, `-max-repair`, `-max-refine`, `-compose-enabled`; `GEMINI_API_KEY` and `BEATSCRIPT_ADMIN_TOKEN` from the env only.
fn options(app: &AppState) -> Options {
    let c = &app.cfg;
    let repair = c.flag_num("max-repair", 3);
    let refine = c.flag_num("max-refine", 1);
    Options {
        public_url: c.public_url.trim_end_matches('/').to_string(),
        model: c.flag("model", gemini::DEFAULT_MODEL),
        api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default().trim().to_string(),
        endpoint: c.flag("gemini-endpoint", ""),
        limits: limits::Config {
            user_hour: c.flag_num("compose-user-hour", 10),
            user_day: c.flag_num("compose-user-day", 50),
            ip_day: c.flag_num("compose-ip-day", 40),
            daily_spend_usd: c.flag_f64("daily-spend-usd", 2.0),
            price_in_per_m: c.flag_f64("price-in-per-m", 1.5),
            price_out_per_m: c.flag_f64("price-out-per-m", 7.5),
            data_dir: Some(c.data.clone()),
        },
        max_prompt_chars: c.flag_num("max-prompt-chars", 800).max(1) as usize,
        max_program_bytes: c.max_program_bytes,
        // server.repairRounds: zero or less means none
        max_repair: if repair <= 0 { -1 } else { repair },
        max_refine: refine.max(0),
        max_output_tokens: gemini::DEFAULT_MAX_OUTPUT_TOKENS,
        thinking: c.flag("thinking", ""),
    }
}

pub fn routes(app: &AppState) -> Router {
    let composer = match Composer::new(app, options(app)) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("beatscript-serve: compose: {e}");
            std::process::exit(1);
        }
    };
    // the kill switch's starting position: -compose-enabled / BEATSCRIPT_COMPOSE_ENABLED, read once (as Go merges them)
    let enabled = parse_bool01(&app.cfg.flag("compose-enabled", "true")).unwrap_or(true);
    composer.lim.set_env_enabled(Box::new(move || Some(if enabled { "1" } else { "0" }.to_string())));
    let _ = COMPOSER.set(composer.clone());
    if composer.configured() {
        let c = composer.clone();
        std::thread::Builder::new().name("compose warm".into()).spawn(move || drop(c.system_prompt())).ok();
    }
    let st = Arc::new(Http { app: app.clone(), composer, admin_token: beatscript_serve::env_var("ADMIN_TOKEN").unwrap_or_default().trim().to_string() });
    Router::new()
        .route("/api/compose", any(compose_or_site))
        .route("/api/admin", any(admin_redirect))
        .route("/api/admin/", any(handle_admin))
        .route("/api/admin/{*rest}", any(handle_admin))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_genres() {
        for k in ["am", "c", "f#m", "d:dorian", "bb", "bbm", "b", "bm", "gb:lydian"] {
            assert!(is_key(k), "{k}");
        }
        for k in ["h", "amm", "c:ab", "c#b", "c:abcdefghijklm"] {
            assert!(!is_key(k), "{k}");
        }
        assert!(is_genre("drum & bass"));
        assert!(!is_genre("a\"b"));
    }

    #[test]
    fn body_errors_as_go() {
        let m = 1000;
        let words = |b: &str| decode_compose_body(b.as_bytes(), m).err().map(|e| e.err.message);
        assert_eq!(words("").unwrap(), r#"send a JSON body like {"prompt":"dark techno 128"}"#);
        assert_eq!(words(r#"{"prompt":1}"#).unwrap(), "the body is not the JSON compose expects: field \"prompt\" has the wrong type");
        assert_eq!(words(r#"{"nope":1,"prompt":2}"#).unwrap(), "the body is not the JSON compose expects: unknown field \"nope\"");
        assert_eq!(words(r#"{"prompt":"x"} {}"#).unwrap(), "the body must be one JSON object");
        assert_eq!(words(r#"{"prompt":"#).unwrap(), "the body is not the JSON compose expects: invalid JSON");
        assert_eq!(words(r#"{"length":true}"#).unwrap(), "the body is not the JSON compose expects: invalid JSON");
        let r = decode_compose_body(br#"{"PROMPT":"x","length":64}"#, m).unwrap();
        assert_eq!((r.prompt.as_str(), r.length.as_str()), ("x", "64"));
    }
}
