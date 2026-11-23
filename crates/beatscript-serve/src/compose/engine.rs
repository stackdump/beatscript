//! The checker as compose and MCP see it: Go's `api.Checker` over beatscript, here beatscript-api in-process. Each answer is what
//! the Go server relays after decoding beatscript's JSON into its own structs (`api.CheckReport`, `FmtReport`,
//! `ShareReport`, `OpenReport`) and encoding them again: Go's field order, `omitempty`, unknown fields dropped, the
//! diagnostics clipped as `beatscript.clipDiagnostics` clips them.
//!
//! In the server every call goes through P3's `public::Checker` (the same shapes, plus Go's per-address budget,
//! slots and timeout); the beatscript-api path is for unit tests that run without one.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::ApiError;

use super::gojson::obj;

/// The engine calls compose and MCP make (Go's `Checker` + `SameAser`): P3's `public::Checker` when it is there
/// (always, in the server), else beatscript-api directly (unit tests).
#[derive(Default)]
pub struct Engine {
    checker: Option<Arc<crate::public::Checker>>,
    rt: Option<tokio::runtime::Handle>,
}

const MAX_DIAGNOSTICS: usize = 200;

/// `beatscript.clip`: at most n bytes on a char boundary, marked.
fn clip_bytes(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut cut = n;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn i(v: &Value) -> Value {
    json!(v.as_i64().unwrap_or(0))
}

fn b(v: &Value) -> Value {
    json!(v == &Value::Bool(true))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// `api.Diagnostic`, clipped.
pub fn diagnostic(d: &Value) -> Value {
    let mut m = Map::new();
    m.insert("severity".into(), json!(s(&d["severity"])));
    for k in ["line", "col", "end_col", "start", "end"] {
        m.insert(k.into(), i(&d[k]));
    }
    m.insert("message".into(), json!(clip_bytes(&s(&d["message"]), 1000)));
    let ex: Vec<String> = strings(&d["expected"]).into_iter().take(20).map(|e| clip_bytes(&e, 200)).collect();
    if !ex.is_empty() {
        m.insert("expected".into(), json!(ex));
    }
    for (k, n) in [("found", 200), ("help", 500), ("render", 2000)] {
        let v = clip_bytes(&s(&d[k]), n);
        if !v.is_empty() {
            m.insert(k.into(), json!(v));
        }
    }
    Value::Object(m)
}

fn diagnostics(v: &Value) -> Value {
    Value::Array(v.as_array().map(|a| a.iter().take(MAX_DIAGNOSTICS).map(diagnostic).collect()).unwrap_or_default())
}

fn str_map(v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    if m.is_empty() {
        return None;
    }
    let mut e: Vec<(String, Value)> = m.iter().map(|(k, x)| (k.clone(), json!(s(x)))).collect();
    e.sort_by(|a, b| a.0.cmp(&b.0));
    Some(Value::Object(e.into_iter().collect()))
}

fn opt_float(v: &Value) -> Value {
    v.as_f64().map_or(Value::Null, super::gojson::num)
}

/// `api.CheckReport` of beatscript-api's check JSON.
pub fn go_check_report(r: &Value) -> Value {
    let mut m = Map::new();
    m.insert("ok".into(), b(&r["ok"]));
    m.insert("errors".into(), i(&r["errors"]));
    m.insert("warnings".into(), i(&r["warnings"]));
    m.insert("diagnostics".into(), diagnostics(&r["diagnostics"]));
    m.insert("cid".into(), json!(s(&r["cid"])));
    m.insert("canonical".into(), json!(s(&r["canonical"])));
    // the language version and the other version's CID, as /api/check keeps them
    crate::public::versions(&mut m, r);
    if let Some(n) = str_map(&r["nets"]) {
        m.insert("nets".into(), n);
    }
    if let Some(n) = str_map(&r["song_nets"]) {
        m.insert("song_nets".into(), n);
    }
    m.insert("tempo".into(), opt_float(&r["tempo"]));
    if !s(&r["key"]).is_empty() {
        m.insert("key".into(), json!(s(&r["key"])));
    }
    m.insert("swing".into(), opt_float(&r["swing"]));
    m.insert("arranged".into(), b(&r["arranged"]));
    m.insert("song_bars".into(), i(&r["song_bars"]));
    m.insert("ends".into(), b(&r["ends"]));
    m.insert("loops".into(), b(&r["loops"]));
    if let Some(a) = r["sections"].as_array().filter(|a| !a.is_empty()) {
        let sec = |x: &Value| {
            let mut e = obj([("name", json!(s(&x["name"]))), ("start", i(&x["start"])), ("bars", i(&x["bars"]))]);
            // a documented section's doc comment (djlive::doc): present only when it has one
            if let Some(d) = x["doc"].as_str() {
                e["doc"] = json!(d);
            }
            e
        };
        m.insert("sections".into(), Value::Array(a.iter().map(sec).collect()));
    }
    for k in ["decks", "evolve"] {
        let v = strings(&r[k]);
        if !v.is_empty() {
            m.insert(k.into(), json!(v));
        }
    }
    if let Some(a) = r["lineage"].as_array().filter(|a| !a.is_empty()) {
        m.insert("lineage".into(), Value::Array(a.clone()));
    }
    // `arrange { }` mutations (djlive mutate.rs): the generation table, and on an error the fixer's text; absent otherwise
    for k in ["arrange", "fixed"] {
        if let Some(x) = r.get(k).filter(|x| !x.is_null()) {
            m.insert(k.into(), x.clone());
        }
    }
    // every doc comment ([{kind, name, doc}], djlive::doc): present only when there are some
    if let Some(a) = r["docs"].as_array().filter(|a| !a.is_empty()) {
        m.insert("docs".into(), Value::Array(a.clone()));
    }
    m.insert("uses_assets".into(), b(&r["uses_assets"]));
    m.insert("restricted".into(), b(&r["restricted"]));
    m.insert("seconds".into(), opt_float(&r["seconds"]));
    // a station program's report (radio), as /api/check keeps it
    crate::public::station(&mut m, r);
    // what the recipe makes: expanded_cid, refs, resolution (beatscript_api::resolution), as /api/check keeps them
    crate::public::references(&mut m, r);
    Value::Object(m)
}

impl Engine {
    /// Over P3's `public::Checker` (the per-address budget and slots of Go's beatscript.Client). Its calls are async; the
    /// engine's are blocking (compose and the MCP tools run on blocking threads), bridged with the runtime's handle.
    pub fn over(checker: Option<Arc<crate::public::Checker>>) -> Engine {
        Engine { checker, rt: tokio::runtime::Handle::try_current().ok() }
    }

    fn block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.rt.as_ref().expect("a tokio runtime").block_on(f)
    }

    /// `Check`, charged to `owner` (the caller's address; "" for unbilled work).
    pub fn check(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        self.check_with(owner, text, false)
    }

    /// [`Engine::check`], and with `expand` each reference's `expansion` in `refs` (the MCP check's `expand: true`).
    pub fn check_with(&self, owner: &str, text: &str, expand: bool) -> Result<Value, ApiError> {
        match &self.checker {
            Some(c) => self.block(c.check_with(owner, text, expand)),
            None => Ok(go_check_report(&serde_json::from_str(&beatscript_api::check_json_with(text, expand)).unwrap_or(Value::Null))),
        }
    }

    /// A station request as JSON (beatscript_api::station_json, docs/beatscript/radio.md): on the checker's budget when there is one, else in
    /// process. The MCP `station` tool; `POST /api/station` calls the checker itself.
    pub fn station(&self, owner: &str, request: &str) -> Result<Value, ApiError> {
        match &self.checker {
            Some(c) => self.block(c.station(owner, request)),
            None => Ok(serde_json::from_str(&beatscript_api::station_json(request)).unwrap_or(Value::Null)),
        }
    }

    /// `expand` (beatscript_api::expand_json): the program with its seed(...) references written out. In process: it needs no
    /// checker child (a seed is composed once and cached).
    pub fn expand(&self, text: &str, lineage: bool) -> Result<Value, ApiError> {
        Ok(serde_json::from_str(&beatscript_api::expand_json(text, lineage)).unwrap_or(Value::Null))
    }

    /// `Fmt` (`api.FmtReport`).
    pub fn fmt(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        if let Some(c) = &self.checker {
            let mut v = self.block(c.fmt(owner, text))?;
            // (the formatter is the language's, in process, whoever answered the rest)
            if v["ok"] == true && v.get("text").is_none() {
                if let Some(t) = beatscript_api::format_text(text) {
                    v["text"] = json!(t);
                }
            }
            return Ok(v);
        }
        let r: Value = serde_json::from_str(&beatscript_api::fmt_json(text)).unwrap_or(Value::Null);
        let mut m = Map::new();
        m.insert("ok".into(), b(&r["ok"]));
        m.insert("canonical".into(), json!(s(&r["canonical"])));
        m.insert("cid".into(), json!(s(&r["cid"])));
        crate::public::versions(&mut m, &r);
        // the program formatted, its header lines and comments kept (what eval, load, share, queue and push_private send)
        if let Some(t) = r["text"].as_str() {
            m.insert("text".into(), json!(t));
        }
        let c = strings(&r["comments"]);
        if !c.is_empty() {
            m.insert("comments".into(), json!(c));
        }
        let d = diagnostics(&r["diagnostics"]);
        if d.as_array().is_some_and(|a| !a.is_empty()) {
            m.insert("diagnostics".into(), d);
        }
        Ok(Value::Object(m))
    }

    /// `Share` (`api.ShareReport`), on beatscript's default origin as Go's beatscript child is.
    pub fn share(&self, owner: &str, text: &str) -> Result<Value, ApiError> {
        if let Some(c) = &self.checker {
            return self.block(c.share(owner, text));
        }
        let r: Value = serde_json::from_str(&beatscript_api::share_json(text, None, false, "")).unwrap_or(Value::Null);
        let mut v = obj([
            ("ok", b(&r["ok"])),
            ("url", json!(s(&r["url"]))),
            ("social_url", json!(s(&r["social_url"]))),
            ("cid", json!(s(&r["cid"]))),
            ("chars", i(&r["chars"])),
            ("canonical", json!(s(&r["canonical"]))),
        ]);
        let w = strings(&r["warnings"]);
        if !w.is_empty() {
            v["warnings"] = json!(w);
        }
        Ok(v)
    }

    /// `Open` (`api.OpenReport`), with beatscript.Client's argument checks.
    pub fn open(&self, owner: &str, u: &str) -> Result<Value, ApiError> {
        if let Some(c) = &self.checker {
            return self.block(c.open(owner, u));
        }
        let u = u.trim();
        if u.is_empty() {
            return Err(ApiError::new(400, "bad_request", "nothing to open: give a link, a fragment or a CID"));
        }
        if u.len() > 100 << 10 {
            return Err(ApiError::new(413, "too_large", format!("link is {} bytes; the limit is {}", u.len(), 100 << 10)));
        }
        if u.starts_with('-') || u.contains('\0') {
            return Err(ApiError::new(400, "bad_request", "not a BeatScript link or CID"));
        }
        let r: Value = serde_json::from_str(&beatscript_api::open_json(u)).unwrap_or(Value::Null);
        let mut m = Map::new();
        m.insert("ok".into(), b(&r["ok"]));
        m.insert("cid".into(), json!(s(&r["cid"])));
        m.insert("canonical".into(), json!(s(&r["canonical"])));
        if !s(&r["text"]).is_empty() {
            // the program formatted, with the `#!` headers and comments the link carried: what an editor shows
            m.insert("text".into(), json!(s(&r["text"])));
        }
        m.insert("verified".into(), b(&r["verified"]));
        for k in ["warnings", "notes_unverified"] {
            let v = strings(&r[k]);
            if !v.is_empty() {
                m.insert(k.into(), json!(v));
            }
        }
        if !s(&r["error"]).is_empty() {
            m.insert("error".into(), json!(s(&r["error"])));
        }
        Ok(Value::Object(m))
    }

    /// `SameAs`: beatscript's same-as JSON, passed through as it came (key order kept).
    pub fn same_as(&self, owner: &str, a: &str, b: &str) -> Result<String, ApiError> {
        match &self.checker {
            Some(c) => self.block(c.same_as(owner, a, b)),
            None => Ok(beatscript_api::same_as_json(a, b).trim().to_string()),
        }
    }

    /// `Instruments`: the raw JSON, trimmed.
    pub fn instruments(&self) -> String {
        match &self.checker {
            Some(c) => c.instruments().to_string(),
            None => beatscript_api::instruments_json().trim().to_string(),
        }
    }

    /// `Cheatsheet`.
    pub fn cheatsheet(&self) -> String {
        let s = match &self.checker {
            Some(c) => c.cheatsheet(),
            None => beatscript_api::cheatsheet(),
        };
        if s.ends_with('\n') {
            s
        } else {
            s + "\n"
        }
    }
}

/// Field readers over a Go-shaped CheckReport.
pub struct Rep<'a>(pub &'a Value);

impl Rep<'_> {
    pub fn ok(&self) -> bool {
        self.0["ok"] == true
    }
    pub fn cid(&self) -> &str {
        self.0["cid"].as_str().unwrap_or("")
    }
    pub fn canonical(&self) -> &str {
        self.0["canonical"].as_str().unwrap_or("")
    }
    pub fn restricted(&self) -> bool {
        self.0["restricted"] == true
    }
    pub fn uses_assets(&self) -> bool {
        self.0["uses_assets"] == true
    }
    pub fn errors(&self) -> i64 {
        self.0["errors"].as_i64().unwrap_or(0)
    }
    pub fn warnings(&self) -> i64 {
        self.0["warnings"].as_i64().unwrap_or(0)
    }
    pub fn sections(&self) -> usize {
        self.0["sections"].as_array().map_or(0, Vec::len)
    }
    pub fn diagnostics(&self) -> Vec<Value> {
        self.0["diagnostics"].as_array().cloned().unwrap_or_default()
    }
    pub fn seconds(&self) -> Option<f64> {
        self.0["seconds"].as_f64()
    }
    pub fn ends(&self) -> bool {
        self.0["ends"] == true
    }
}
