//! The `embed` tool's core: the right snippet or page for a target (a Claude artifact, a web page, an iframe, a README,
//! a notebook, an editor, the Android app ...) and an embed config (site/web/embed-config.schema.json).
//!
//! The config half mirrors site/embed/embed-config.js: [`normalize`] checks a config against the same schema with the
//! same messages and completes it with the kind's defaults, [`to_attributes`] gives the element's attributes and
//! [`artifact_capabilities`] the Claude Artifact tool's `capabilities`. Their output is byte for byte the JavaScript's,
//! held to site/testdata/embed-config/cases.json. The target half, [`TARGETS`] and [`answer`], says for each target
//! whether the config works there, what it degrades to and why, and gives the markup. Pure functions: the tool that
//! calls them checks the program and finds whether the site stores it (the [`Ctx`]).

use base64::Engine as _;
use serde_json::{json, Map, Value};

use super::artifact::{esc, Page, PlayerCdn};
use super::{bad, is_cid, name_ok, needs_fixing, private, q, sprop, tool, Args, Caller, Fail, Rep, Text, Tools, CID_PAT, NAME_PAT};

const SCHEMA_JSON: &str = include_str!("../../../../site/web/embed-config.schema.json");
const STUDIO_LOCK: &str = include_str!("studio-cdn.lock.json");
const JSDELIVR: &str = "https://cdn.jsdelivr.net/npm/";
pub const DEFAULT_SITE: &str = "https://beatscript.app";
pub const KINDS: [&str; 4] = ["player", "studio", "card", "audio"];
pub const FEATURES: [&str; 7] = ["highlight", "editor", "mixer", "knobs", "download", "share", "compose"];
pub const CAPABILITIES: [&str; 7] = ["sample", "mcp", "db", "downloads", "room", "dictation", "tts"];
/// The upstream tools of the viewer's BeatScript connector that the studio calls.
const CONNECTOR_TOOLS: [&str; 3] = ["share", "render", "queue"];
const N_DEFAULT: i64 = 0;
const COUNT_DEFAULT: i64 = 8;

/// The /embed frame, as cards/embed.rs sets it (its constants are private to `cards`; a test holds these to them).
const SANDBOX: &str = "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox";
const SIZE: (u32, u32) = (480, 152);
const SIZE_COMPACT: (u32, u32) = (480, 80);
const STORE_FIRST: &str = "store it first (share, signed in)";
const ONE_PROGRAM: &str = "this target takes one program: give text or cid, not a set or a station";

fn schema() -> &'static Value {
    static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    S.get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("embed-config.schema.json"))
}

// ---------- the config ----------

/// A normalized config: complete with defaults, its keys in embed-config.js's order (`json`), and the warnings
/// normalize gives (settings the schema allows that do nothing here).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub json: Value,
    pub warnings: Vec<String>,
}

impl Config {
    fn s(&self, k: &str) -> &str {
        self.json[k].as_str().unwrap_or("")
    }
    pub fn kind(&self) -> &str {
        self.s("kind")
    }
    pub fn audio(&self) -> &str {
        self.s("audio")
    }
    pub fn theme(&self) -> &str {
        self.s("theme")
    }
    pub fn site(&self) -> &str {
        self.s("site")
    }
    pub fn compact(&self) -> bool {
        self.json["compact"] == true
    }
    pub fn title(&self) -> Option<&str> {
        self.json["title"].as_str()
    }
    pub fn source(&self) -> &Value {
        &self.json["source"]
    }
    fn list(&self, k: &str) -> Vec<&str> {
        self.json[k].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
    }
    pub fn features(&self) -> Vec<&str> {
        self.list("features")
    }
    pub fn capabilities(&self) -> Vec<&str> {
        self.list("capabilities")
    }
    fn size(&self, k: &str) -> Option<u32> {
        self.json[k].as_u64().map(|n| n as u32)
    }
}

/// The audio, features and capabilities a kind has when the config does not say (an unknown kind gives the player's).
pub fn defaults_for(kind: &str) -> (&'static str, &'static [&'static str], &'static [&'static str]) {
    match kind {
        "studio" => ("live", &["highlight", "editor", "download", "share", "compose"], &["sample", "mcp", "db", "downloads"]),
        "card" => ("url", &["share"], &[]),
        "audio" => ("inline", &["download"], &[]),
        _ => ("live", &["share"], &["downloads"]),
    }
}

/// The items of `all` that `list` has, in `all`'s order.
fn in_order(list: &[&str], all: &[&str]) -> Vec<Value> {
    all.iter().filter(|x| list.contains(x)).map(|x| json!(x)).collect()
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// A JSON number as JavaScript reads it: whether it is an integer, and its value.
fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn is_int(v: &Value) -> bool {
    v.is_number() && num(v).is_some_and(|f| f.is_finite() && f.fract() == 0.0)
}

/// An integer of the config as JavaScript would print it (1.0 and 1e2 are 1 and 100).
fn int(v: &Value) -> Value {
    json!(num(v).unwrap_or(0.0) as i64)
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "integer" => is_int(v),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        _ => false,
    }
}

fn type_says(t: &str) -> &'static str {
    match t {
        "object" => "an object",
        "array" => "an array",
        "string" => "a string",
        "integer" => "an integer",
        "boolean" => "true or false",
        _ => "something else",
    }
}

/// What a failed pattern says: the three the schema uses have plain words.
fn pattern_says(p: &str) -> String {
    let props = &schema()["properties"];
    let plain = [
        (&props["source"]["properties"]["cid"]["pattern"], "is not a program CID"),
        (&props["site"]["pattern"], "must start with https://"),
        (&props["pin"]["properties"]["version"]["pattern"], "is not a version such as 1.2.3"),
    ];
    plain.iter().find(|(q, _)| q.as_str() == Some(p)).map_or_else(|| format!("must match {p}"), |(_, s)| s.to_string())
}

fn pattern_ok(p: &str, s: &str) -> bool {
    static RE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, regex::Regex>>> = std::sync::OnceLock::new();
    let mut m = RE.get_or_init(Default::default).lock().unwrap();
    m.entry(p.to_string()).or_insert_with(|| regex::Regex::new(p).expect("a schema pattern")).is_match(s)
}

/// JSON.stringify of a value (numbers as JavaScript prints an integer).
fn js_json(v: &Value) -> String {
    serde_json::to_string(&js_numbers(v)).unwrap()
}

fn js_numbers(v: &Value) -> Value {
    match v {
        Value::Number(_) if is_int(v) && num(v).is_some_and(|f| f.abs() < 9.007_199_254_740_992e15) => int(v),
        Value::Array(a) => Value::Array(a.iter().map(js_numbers).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), js_numbers(x))).collect()),
        x => x.clone(),
    }
}

/// "text, cid, set or station" from the branches' required names.
fn alternatives(branches: &[Value], last: &str) -> String {
    let names: Vec<String> = branches.iter().map(|b| strs(&b["required"]).join(" and ")).collect();
    match names.split_last() {
        Some((l, rest)) if !rest.is_empty() => format!("{} {last} {l}", rest.join(", ")),
        _ => names.join(""),
    }
}

fn matches(s: &Value, v: &Value) -> bool {
    let mut errs = Vec::new();
    check(s, v, "", &mut errs);
    errs.is_empty()
}

/// embed-config.js `check`: appends (path, message) for each way `v` breaks `s`. At one value the first failing scalar
/// keyword is its only error; an object's or array's members are checked one by one.
fn check(s: &Value, v: &Value, path: &str, errs: &mut Vec<(String, String)>) {
    let fail = |errs: &mut Vec<(String, String)>, m: String| errs.push((path.to_string(), m));
    if let Some(t) = s["type"].as_str() {
        if !type_ok(t, v) {
            return fail(errs, format!("must be {}", type_says(t)));
        }
    }
    if let Some(c) = s.get("const") {
        let same = match (num(c), num(v)) {
            (Some(a), Some(b)) if c.is_number() && v.is_number() => a == b,
            _ => c == v,
        };
        if !same {
            return fail(errs, format!("must be {}", js_json(c)));
        }
    }
    if let Some(e) = s["enum"].as_array() {
        if !e.contains(v) {
            return fail(errs, format!("must be one of {}", strs(&s["enum"]).join(", ")));
        }
    }
    if let Some(t) = v.as_str() {
        // JSON Schema counts characters (code points), not UTF-16 units
        let len = t.chars().count() as u64;
        if let Some(n) = s["minLength"].as_u64().filter(|n| len < *n) {
            return fail(errs, if n == 1 { "must not be empty".into() } else { format!("must be at least {n} characters") });
        }
        if let Some(n) = s["maxLength"].as_u64().filter(|n| len > *n) {
            return fail(errs, format!("must be at most {n} characters"));
        }
        if let Some(p) = s["pattern"].as_str().filter(|p| !pattern_ok(p, t)) {
            return fail(errs, pattern_says(p));
        }
    }
    if let Some(f) = v.as_f64().filter(|_| v.is_number()) {
        if let Some(m) = s.get("minimum").filter(|m| num(m).is_some_and(|m| f < m)) {
            return fail(errs, format!("must be at least {m}"));
        }
        if let Some(m) = s.get("maximum").filter(|m| num(m).is_some_and(|m| f > m)) {
            return fail(errs, format!("must be at most {m}"));
        }
    }
    if let Some(a) = v.as_array() {
        let n = a.len() as u64;
        if let Some(m) = s["minItems"].as_u64().filter(|m| n < *m) {
            return fail(errs, format!("must have at least {m} item{}", if m == 1 { "" } else { "s" }));
        }
        if let Some(m) = s["maxItems"].as_u64().filter(|m| n > *m) {
            return fail(errs, format!("must have at most {m} items"));
        }
        if s["uniqueItems"] == true {
            let mut seen = std::collections::HashSet::new();
            for item in a {
                let key = js_json(item);
                if !seen.insert(key.clone()) {
                    return fail(errs, format!("has {key} twice"));
                }
            }
        }
        if let Some(items) = s.get("items") {
            for (i, item) in a.iter().enumerate() {
                check(items, item, &format!("{path}/{i}"), errs);
            }
        }
    }
    if let Some(o) = v.as_object() {
        for (k, val) in o {
            let at = format!("{path}/{}", k.replace('~', "~0").replace('/', "~1"));
            match s["properties"].get(k) {
                Some(sub) => check(sub, val, &at, errs),
                None if s["additionalProperties"] == false => errs.push((at, "is not a known key".into())),
                None => {}
            }
        }
        for k in strs(&s["required"]) {
            if !o.contains_key(k) {
                fail(errs, format!("needs {k}"));
            }
        }
    }
    if let Some(b) = s["anyOf"].as_array() {
        if !b.iter().any(|b| matches(b, v)) {
            fail(errs, format!("needs {}", alternatives(b, "or")));
        }
    }
    if let Some(b) = s["oneOf"].as_array() {
        if b.iter().filter(|b| matches(b, v)).count() != 1 {
            fail(errs, format!("needs exactly one of {}", alternatives(b, "or")));
        }
    }
    if let Some(n) = s.get("not") {
        if matches(n, v) {
            fail(errs, "is not allowed here".into());
        }
    }
}

fn norm_source(s: &Value) -> Value {
    let mut o = Map::new();
    if let Some(set) = s["set"].as_array() {
        let items = set
            .iter()
            .map(|it| Value::Object(["text", "cid", "title"].iter().filter_map(|k| it.get(*k).map(|v| (k.to_string(), v.clone()))).collect()))
            .collect();
        o.insert("set".into(), Value::Array(items));
    } else if let Some(st) = s.get("station") {
        o.insert("station".into(), st.clone());
        o.insert("n".into(), s.get("n").map_or(json!(N_DEFAULT), int));
        o.insert("count".into(), s.get("count").map_or(json!(COUNT_DEFAULT), int));
    } else {
        for k in ["text", "cid"] {
            if let Some(v) = s.get(k) {
                o.insert(k.into(), v.clone());
            }
        }
    }
    Value::Object(o)
}

fn lone_cid(source: &Value) -> bool {
    source.get("cid").is_some() && source.get("text").is_none()
}

/// The raw config an input gives: a JSON string parsed, and null (or the JSON null) as {}.
fn raw_of(input: &Value) -> Result<Value, &'static str> {
    let v = match input {
        Value::String(s) => serde_json::from_str::<Value>(s).map_err(|_| "is not valid JSON")?,
        v => v.clone(),
    };
    Ok(if v.is_null() { json!({}) } else { v })
}

/// embed-config.js `normalize`: the config, checked against the schema and completed with the kind's defaults. input is
/// an object, a JSON string or null (all defaults). Err gives (path, message) for each error, sorted by path (a JSON
/// Pointer, "" for the whole config).
pub fn normalize(input: &Value) -> Result<Config, Vec<(String, String)>> {
    let raw = &raw_of(input).map_err(|e| vec![(String::new(), e.to_string())])?;
    let mut errs = Vec::new();
    check(schema(), raw, "", &mut errs);
    if !errs.is_empty() {
        // by code point, as embed-config.js sorts them (a String's order)
        errs.sort_by(|a, b| a.0.cmp(&b.0));
        return Err(errs);
    }
    let kind = raw["kind"].as_str().unwrap_or("player");
    let (audio, features, caps) = defaults_for(kind);
    let mut c = Map::new();
    c.insert("v".into(), json!(1));
    c.insert("kind".into(), json!(kind));
    if let Some(s) = raw.get("source") {
        c.insert("source".into(), norm_source(s));
    }
    c.insert("audio".into(), raw.get("audio").cloned().unwrap_or(json!(audio)));
    let f = raw.get("features").map_or_else(|| features.to_vec(), strs);
    c.insert("features".into(), Value::Array(in_order(&f, &FEATURES)));
    let k = raw.get("capabilities").map_or_else(|| caps.to_vec(), strs);
    c.insert("capabilities".into(), Value::Array(in_order(&k, &CAPABILITIES)));
    c.insert("theme".into(), raw.get("theme").cloned().unwrap_or(json!("dark")));
    c.insert("compact".into(), raw.get("compact").cloned().unwrap_or(json!(false)));
    for k in ["width", "height"] {
        if let Some(v) = raw.get(k) {
            c.insert(k.into(), int(v));
        }
    }
    if let Some(t) = raw.get("title") {
        c.insert("title".into(), t.clone());
    }
    c.insert("site".into(), raw.get("site").cloned().unwrap_or(json!(DEFAULT_SITE)));
    c.insert("bridge".into(), raw.get("bridge").cloned().unwrap_or(json!(false)));
    if let Some(p) = raw.get("pin") {
        let mut pin = Map::new();
        if let Some(v) = p.get("version") {
            pin.insert("version".into(), v.clone());
        }
        pin.insert("sri".into(), p.get("sri").cloned().unwrap_or(json!(true)));
        c.insert("pin".into(), Value::Object(pin));
    }
    let config = Value::Object(c);

    let mut warnings = Vec::new();
    let s = raw.get("source");
    if s.is_none() && kind != "studio" {
        warnings.push("there is nothing to play: give text, cid, set or station".to_string());
    }
    if let Some(s) = s {
        if s.get("station").is_none() && (s.get("n").is_some() || s.get("count").is_some()) {
            warnings.push("n and count apply only to a station".into());
        }
        if config["audio"] == "inline" && !lone_cid(&config["source"]) {
            warnings.push("inline audio needs a stored program: a cid alone".into());
        }
    }
    if config["bridge"] == true && kind != "studio" {
        warnings.push("bridge applies only to the studio".into());
    }
    Ok(Config { json: config, warnings })
}

/// normalize, with the target's own kind when the config gives none: an audio or rss config is the audio kind, a
/// markdown or preview one a card, without the caller saying so.
pub fn normalize_for(target: &str, input: &Value) -> Result<Config, Vec<(String, String)>> {
    let Ok(mut raw) = raw_of(input) else { return normalize(input) };
    if let (Some(t), Some(o)) = (target_row(target), raw.as_object_mut()) {
        if !o.contains_key("kind") {
            o.insert("kind".into(), json!(t.kinds[0]));
        }
    }
    normalize(&raw)
}

/// embed-config.js `toAttributes`: the element for a normalized config, (tag, [(name, value)], text). Only what differs
/// from the defaults is written, as attributes where there is one (cid, mode or audio, highlight, theme, compact, site)
/// and the rest as the `config` attribute's compact JSON, last. card and audio are not elements: None.
pub fn to_attributes(c: &Config) -> Option<(&'static str, Vec<(String, String)>, Option<String>)> {
    let kind = c.kind();
    let tag = match kind {
        "player" => "beatscript-track",
        "studio" => "beatscript-studio",
        _ => return None,
    };
    let (d_audio, d_features, d_caps) = defaults_for(kind);
    let s = c.source();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut rest = Map::new();
    if let Some(cid) = s["cid"].as_str() {
        attrs.push(("cid".into(), cid.into()));
    }
    if c.audio() != d_audio {
        // player 0.2.0 reads mode="render"; it has no attribute for the server's audio
        let mode = kind == "player" && matches!(c.audio(), "live" | "render");
        attrs.push((if mode { "mode" } else { "audio" }.into(), c.audio().into()));
    }
    let features = c.features();
    let highlight = features.contains(&"highlight") && !d_features.contains(&"highlight");
    if highlight {
        attrs.push(("highlight".into(), String::new()));
    }
    // player 0.2.0 styles only theme="light", and a theme it has as an attribute is never resolved: auto goes in the JSON
    if c.theme() == "light" {
        attrs.push(("theme".into(), c.theme().into()));
    }
    if c.compact() {
        attrs.push(("compact".into(), String::new()));
    }
    if c.site() != DEFAULT_SITE {
        attrs.push(("site".into(), c.site().into()));
    }
    let mut src = Map::new();
    if let Some(set) = s.get("set") {
        src.insert("set".into(), set.clone());
    }
    if let Some(st) = s.get("station") {
        src.insert("station".into(), st.clone());
        if s["n"] != json!(N_DEFAULT) {
            src.insert("n".into(), s["n"].clone());
        }
        if s["count"] != json!(COUNT_DEFAULT) {
            src.insert("count".into(), s["count"].clone());
        }
    }
    if !src.is_empty() {
        rest.insert("source".into(), Value::Object(src));
    }
    // what fromElement makes of the features without a features key: the defaults, plus highlight if its attribute is set
    let mut implied: Vec<&str> = d_features.to_vec();
    if highlight {
        implied.push("highlight");
        implied = FEATURES.iter().copied().filter(|f| implied.contains(f)).collect();
    }
    if features != implied {
        rest.insert("features".into(), c.json["features"].clone());
    }
    if c.capabilities() != d_caps {
        rest.insert("capabilities".into(), c.json["capabilities"].clone());
    }
    if c.theme() == "auto" {
        rest.insert("theme".into(), json!("auto"));
    }
    for k in ["width", "height", "title"] {
        if let Some(v) = c.json.get(k) {
            rest.insert(k.into(), v.clone());
        }
    }
    if c.json["bridge"] == true {
        rest.insert("bridge".into(), json!(true));
    }
    if let Some(p) = c.json.get("pin") {
        rest.insert("pin".into(), p.clone());
    }
    if !rest.is_empty() {
        attrs.push(("config".into(), js_json(&Value::Object(rest))));
    }
    Some((tag, attrs, s["text"].as_str().map(str::to_string)))
}

/// embed-config.js `artifactCapabilities`: the Claude Artifact tool's `capabilities` for a normalized config, in
/// CAPABILITIES order (db brings user: the library is per user).
pub fn artifact_capabilities(c: &Config, server: &str) -> Value {
    let mut o = Map::new();
    for cap in c.capabilities() {
        match cap {
            "mcp" => {
                o.insert("mcp".into(), json!({"servers": [{"server": server, "tools": CONNECTOR_TOOLS}]}));
            }
            "db" => {
                o.insert("db".into(), json!({}));
                o.insert("user".into(), json!({}));
            }
            "downloads" => {
                o.insert("downloads".into(), json!(true));
            }
            // room.d.ts: event topics are admin-only unless the declaration opens them; the jam sends on "program"
            "room" => {
                o.insert("room".into(), json!({"topics": {"program": "interact"}}));
            }
            other => {
                o.insert(other.into(), json!({}));
            }
        }
    }
    Value::Object(o)
}

// ---------- the studio package ----------

/// The pinned `@stackdump/beatscript-studio` the studio page loads (`studio-cdn.lock.json`), the same shape as
/// [`PlayerCdn`]. `placeholder` is true until the package is published and the lock regenerated
/// (deploy/studio-cdn-lock.py): its hashes are then not real, and a page made with them refuses to run.
pub struct StudioCdn {
    pub package: String,
    pub version: String,
    pub entry: String,
    /// (file in the package, SRI integrity), in the lock's order (sorted by file).
    pub files: Vec<(String, String)>,
    pub placeholder: bool,
}

impl StudioCdn {
    pub fn get() -> &'static StudioCdn {
        static S: std::sync::OnceLock<StudioCdn> = std::sync::OnceLock::new();
        S.get_or_init(|| StudioCdn::parse(STUDIO_LOCK))
    }

    fn parse(lock: &str) -> StudioCdn {
        let v: Value = serde_json::from_str(lock).expect("studio-cdn.lock.json");
        let s = |k: &str| v[k].as_str().unwrap_or_else(|| panic!("studio-cdn.lock.json: {k}")).to_string();
        let files = v["files"].as_object().expect("studio-cdn.lock.json: files").iter().map(|(f, x)| (f.clone(), x["integrity"].as_str().expect("integrity").to_string())).collect();
        StudioCdn { package: s("package"), version: s("version"), entry: s("entry"), files, placeholder: v["placeholder"] == true }
    }

    /// The jsDelivr URL of one file of the pinned version.
    pub fn url(&self, file: &str) -> String {
        format!("{JSDELIVR}{}@{}/{file}", self.package, self.version)
    }

    pub fn integrity(&self, file: &str) -> &str {
        self.files.iter().find(|(f, _)| f == file).map(|(_, i)| i.as_str()).unwrap_or("")
    }

    /// The JavaScript modules the entry can import (every `.js` but the entry and a UMD wrapper).
    fn modules(&self) -> impl Iterator<Item = &(String, String)> {
        self.files.iter().filter(|(f, _)| f.ends_with(".js") && *f != self.entry && !f.ends_with(".umd.js"))
    }

    /// The import map and the entry's script tag, in that order, exactly as [`PlayerCdn::tags`] writes them.
    pub fn tags(&self) -> String {
        let mut m = Map::new();
        for (f, i) in self.modules() {
            m.insert(self.url(f), json!(i));
        }
        let map = serde_json::to_string(&json!({"integrity": m})).unwrap();
        format!(
            "<script type=\"importmap\">{}</script>\n<script type=\"module\" src=\"{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>\n",
            map.replace('<', "\\u003c"),
            self.url(&self.entry),
            self.integrity(&self.entry)
        )
    }
}

// ---------- the targets ----------

/// One place a program can be embedded.
pub struct Target {
    pub id: &'static str,
    pub title: &'static str,
    /// The config kinds it takes; the first is the kind a config without one gets ([`normalize_for`]).
    pub kinds: &'static [&'static str],
    /// Whether the BeatScript engine runs there (in the page, a frame or the app), rather than a render playing.
    pub engine: bool,
    /// Whether it needs a program the site stores (its /p/, /embed/, card or render): without one it degrades or is
    /// unsupported, as its answer says.
    pub needs_stored: bool,
    pub note: &'static str,
}

const ALL: &[&str] = &KINDS;

pub const TARGETS: &[Target] = &[
    Target { id: "artifact", title: "A Claude artifact", kinds: &["player", "studio"], engine: true, needs_stored: false, note: "A whole page to publish as an Artifact: the player, or the whole studio with its in-page tools, loaded from cdn.jsdelivr.net at an exact version with Subresource Integrity." },
    Target { id: "web", title: "Any web page", kinds: &["player", "studio"], engine: true, needs_stored: false, note: "The script tags and the element, for a page that allows scripts from cdn.jsdelivr.net and WebAssembly." },
    Target { id: "iframe", title: "An iframe", kinds: &["player"], engine: true, needs_stored: true, note: "The site's /embed/<CID> player in a frame, for a host that takes an iframe but no scripts." },
    Target { id: "oembed", title: "oEmbed", kinds: &["player", "card"], engine: true, needs_stored: true, note: "The /p/<CID> link: WordPress, Discourse, Notion and the like embed the player from it by oEmbed." },
    Target { id: "preview", title: "A link preview", kinds: &["card", "player"], engine: false, needs_stored: true, note: "The /c/<CID> link: X plays it in the post, Discord, Slack and iMessage get its audio, Mastodon shows the card." },
    Target { id: "markdown", title: "Markdown (a README)", kinds: &["card", "player", "audio"], engine: false, needs_stored: true, note: "The program's card image linking to its page, and a link to its audio." },
    Target { id: "jupyter", title: "A Jupyter notebook", kinds: &["player", "studio"], engine: true, needs_stored: false, note: "A cell that shows the web embed through IPython.display.HTML, with nothing to install." },
    Target { id: "vscode", title: "VS Code", kinds: &["player"], engine: true, needs_stored: false, note: "The program as a .beatscript file, played in the BeatScript extension's preview." },
    Target { id: "obsidian", title: "An Obsidian note", kinds: &["player", "card"], engine: true, needs_stored: true, note: "The iframe, pasted into the note; a program the site does not store is a link." },
    Target { id: "audio", title: "An <audio> element", kinds: &["audio"], engine: false, needs_stored: true, note: "The site's Ogg Opus render in a plain <audio controls>, with no script." },
    Target { id: "rss", title: "An RSS enclosure", kinds: &["audio"], engine: false, needs_stored: true, note: "The site's render as a podcast item's enclosure." },
    Target { id: "android", title: "The Android app", kinds: &["player", "studio", "card"], engine: true, needs_stored: false, note: "The /p#<CID>.<packed> link (the program is in the link), which the BeatScript app opens; without the app it opens on the site." },
    Target { id: "json", title: "The config as JSON", kinds: ALL, engine: false, needs_stored: false, note: "The normalized config, complete with defaults, for any surface that reads it." },
];

pub fn target_row(id: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.id == id)
}

/// What [`answer`] needs besides the config: the program, checked, and what the site knows of it.
pub struct Ctx<'a> {
    /// The origin the links point at (the server's public URL).
    pub site: &'a str,
    /// The program's CID; "" when the config names no one program (a set or a station).
    pub cid: &'a str,
    /// The program's canonical text; "" when there is none.
    pub text: &'a str,
    /// Whether the site stores the program, so its /p/, /c/, /embed/, card and render answer.
    pub stored: bool,
    /// The self-sealing share link (the program is in the link); "" when there is none.
    pub sealed: &'a str,
    /// The program's rights, as [`Page`] takes them: {license, source, credits?, forked_from?}.
    pub rights: &'a Value,
    pub player: &'a PlayerCdn,
    pub studio: &'a StudioCdn,
}

#[derive(Default)]
struct Ans {
    works: &'static str,
    engine: bool,
    notes: Vec<String>,
    snippet: String,
    page: Option<String>,
    capabilities: Option<Value>,
    links: Map<String, Value>,
    file: Option<Value>,
}

impl Ans {
    fn no(&mut self, why: impl Into<String>) {
        self.works = "unsupported";
        self.engine = false;
        self.notes.push(why.into());
    }
    fn less(&mut self, why: impl Into<String>) {
        if self.works == "works" {
            self.works = "degraded";
        }
        self.notes.push(why.into());
    }
    fn note(&mut self, s: impl Into<String>) {
        self.notes.push(s.into());
    }
    fn link(&mut self, k: &str, v: impl Into<String>) {
        self.links.insert(k.into(), json!(v.into()));
    }
}

/// `#! <key>: X` of a text.
fn header<'t>(text: &'t str, key: &str) -> Option<&'t str> {
    text.lines().find_map(|l| l.trim().strip_prefix("#!").map(str::trim).and_then(|h| h.strip_prefix(key)).and_then(|h| h.strip_prefix(':')).map(str::trim))
}

/// The name shown: the config's title, else the program's `#! name:`, else "BeatScript".
fn name(c: &Config, ctx: &Ctx) -> String {
    let t = c.title().map(str::trim).filter(|t| !t.is_empty()).or_else(|| header(text_of(c, ctx), "name").filter(|n| !n.is_empty())).unwrap_or("BeatScript");
    t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The program text: the checked canonical text, else the config's own.
fn text_of<'a>(c: &'a Config, ctx: &'a Ctx) -> &'a str {
    if ctx.text.is_empty() {
        c.source()["text"].as_str().unwrap_or("")
    } else {
        ctx.text
    }
}

/// The element with the program: toAttributes' tag and attributes, then the text in the `text` attribute,
/// HTML-escaped, as the artifact page carries it. Not a child <script type="text/beatscript">: nothing in a script's
/// content can be escaped without changing the text the element reads (and so its CID), and the attribute
/// round-trips exactly. Both elements read `text` first.
fn element(c: &Config, ctx: &Ctx) -> Option<String> {
    let (tag, attrs, _) = to_attributes(c)?;
    let text = text_of(c, ctx);
    let mut s = format!("<{tag}");
    for (k, v) in &attrs {
        if v.is_empty() && matches!(k.as_str(), "highlight" | "compact") {
            s.push_str(&format!(" {k}"));
        } else {
            s.push_str(&format!(" {k}=\"{}\"", esc(v)));
        }
    }
    if !text.is_empty() {
        s.push_str(&format!(" text=\"{}\"", esc(text)));
    }
    s.push_str(&format!("></{tag}>"));
    Some(s)
}

/// The script tags and the element, for a page.
fn web(c: &Config, ctx: &Ctx) -> String {
    let tags = if c.kind() == "studio" { ctx.studio.tags() } else { ctx.player.tags() };
    format!("{tags}{}\n", element(c, ctx).unwrap_or_default())
}

/// The studio as a whole page: the pinned studio package, and the element holding the program. No inline style (an
/// artifact's CSP refuses it): the element styles itself.
pub fn studio_page(c: &Config, ctx: &Ctx) -> String {
    let meta = if ctx.cid.is_empty() { String::new() } else { format!("<meta name=\"beatscript:cid\" content=\"{}\">\n", esc(ctx.cid)) };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n{meta}<title>{}</title>\n{}</head>\n<body>\n{}\n</body>\n</html>\n",
        esc(&name(c, ctx)),
        ctx.studio.tags(),
        element(c, ctx).unwrap_or_default()
    )
}

/// The player as a whole page: the artifact tool's [`Page`] with the live player (no inline audio). None without one
/// program to play (a set or a station).
pub fn player_page(c: &Config, ctx: &Ctx) -> Option<String> {
    if ctx.cid.is_empty() || ctx.text.is_empty() {
        return None;
    }
    let title = name(c, ctx);
    // a `#! name:` header line, as the artifact tool adds it: the CID is of the canonical text, so it is unchanged
    let text = if title == "BeatScript" || header(ctx.text, "name").is_some() { ctx.text.to_string() } else { format!("#! name: {title}\n{}", ctx.text) };
    let link = if ctx.stored {
        format!("{}/p/{}", ctx.site, ctx.cid)
    } else if ctx.sealed.is_empty() {
        ctx.site.to_string()
    } else {
        ctx.sealed.to_string()
    };
    Some(Page { title, cid: ctx.cid.into(), text, rights: ctx.rights.clone(), link, site: ctx.site.into(), stored: ctx.stored, opus: None }.html())
}

/// A query component, percent-encoded (everything but the unreserved characters).
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Markdown text with every ASCII punctuation character backslash-escaped, so a name cannot make a link or markup.
fn md_text(s: &str) -> String {
    s.chars().flat_map(|c| if c.is_ascii_punctuation() { vec!['\\', c] } else { vec![c] }).collect()
}

/// A URL as a Markdown link destination: a space, a bracket or a parenthesis would end it.
fn md_url(s: &str) -> String {
    s.chars().map(|c| if matches!(c, ' ' | '(' | ')' | '<' | '>') { format!("%{:02X}", c as u32) } else { c.to_string() }).collect()
}

/// The /embed frame, as cards/embed.rs's iframe_html writes it, sized by the config or the frame's own size.
fn iframe(c: &Config, ctx: &Ctx) -> (String, String) {
    let mut q = Vec::new();
    if c.compact() {
        q.push("compact=1");
    }
    if c.theme() == "light" {
        q.push("theme=light");
    }
    let src = format!("{}/embed/{}{}{}", ctx.site, ctx.cid, if q.is_empty() { "" } else { "?" }, q.join("&"));
    let (w, h) = if c.compact() { SIZE_COMPACT } else { SIZE };
    let (w, h) = (c.size("width").unwrap_or(w), c.size("height").unwrap_or(h));
    let html = format!(
        "<iframe src=\"{}\" width=\"{w}\" height=\"{h}\" title=\"{} · BeatScript player\" sandbox=\"{SANDBOX}\" loading=\"lazy\" referrerpolicy=\"strict-origin-when-cross-origin\" frameborder=\"0\"></iframe>",
        esc(&src),
        esc(&name(c, ctx))
    );
    (html, src)
}

/// The site's Opus render of a stored program.
fn render_url(ctx: &Ctx) -> String {
    beatscript_serve::opendata::urls(ctx.site, ctx.cid).render
}

/// The link the Android app opens, as site/web/js/qr-model.js appLink makes it: `<site>/p#<CID>.<packed>` from the
/// self-sealing link's fragment (the app claims the /p path only, so `<site>/#...` would not open it).
fn app_link(ctx: &Ctx) -> Option<String> {
    let frag = ctx.sealed.split_once('#')?.1.split('~').next().unwrap_or("");
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    let (cid, packed) = frag.split_once('.')?;
    (ok(cid) && ok(packed)).then(|| format!("{}/p#{frag}", ctx.site.trim_end_matches('/')))
}

/// The self-sealing link, else the site.
fn sealed_or_site<'a>(ctx: &'a Ctx) -> &'a str {
    if ctx.sealed.is_empty() {
        ctx.site
    } else {
        ctx.sealed
    }
}

const STUDIO_UNPUBLISHED: &str = "@stackdump/beatscript-studio is not published yet: its lock holds placeholder hashes, so the page cannot load it";
const NEEDS_CDN: &str = "the page must allow scripts from cdn.jsdelivr.net and WebAssembly ('wasm-unsafe-eval')";

fn fill(t: &Target, c: &Config, ctx: &Ctx, a: &mut Ans) {
    let one = !ctx.cid.is_empty();
    let studio = c.kind() == "studio";
    let needs_one = !matches!(t.id, "artifact" | "web" | "jupyter" | "json");
    if needs_one && !one {
        return a.no(ONE_PROGRAM);
    }
    if studio && ctx.studio.placeholder && matches!(t.id, "artifact" | "web" | "jupyter") {
        a.less(STUDIO_UNPUBLISHED);
    }
    let page_link = format!("{}/p/{}", ctx.site, ctx.cid);
    match t.id {
        "artifact" => {
            a.snippet = web(c, ctx);
            if studio {
                a.page = Some(studio_page(c, ctx));
                a.capabilities = Some(artifact_capabilities(c, "Beatscript"));
                a.note("publish page as an Artifact exactly as it is, with capabilities as the Artifact's capabilities");
            } else if let Some(p) = player_page(c, ctx) {
                a.page = Some(p);
                a.note("publish page as an Artifact exactly as it is");
                let mut ignored = Vec::new();
                if c.audio() == "render" {
                    ignored.push("audio render");
                }
                if c.features().contains(&"highlight") {
                    ignored.push("highlight");
                }
                if c.theme() != "dark" {
                    ignored.push("theme");
                }
                if c.compact() {
                    ignored.push("compact");
                }
                if !ignored.is_empty() {
                    a.less(format!("the artifact player page does not take {} yet", ignored.join(", ")));
                }
            } else {
                a.less("a set or a station has no player page: this is the web embed, or use kind studio");
            }
        }
        "web" => {
            a.snippet = web(c, ctx);
            a.note(NEEDS_CDN);
            pinned_player_reads(c, ctx, a);
        }
        "iframe" | "obsidian" if ctx.stored => {
            let (html, src) = iframe(c, ctx);
            a.snippet = html;
            a.link("embed", src);
            if t.id == "obsidian" {
                a.note("paste it into the note: Obsidian shows the frame in reading view and live preview");
            }
        }
        "iframe" | "oembed" | "audio" | "rss" if !ctx.stored => a.no(STORE_FIRST),
        "obsidian" => {
            a.snippet = format!("[{}]({})", md_text(&name(c, ctx)), md_url(sealed_or_site(ctx)));
            a.engine = false;
            a.less(format!("the site does not store it, so the note gets a link: {STORE_FIRST} for the player"));
        }
        "oembed" => {
            a.snippet = page_link.clone();
            a.link("page", page_link.clone());
            a.link("oembed", format!("{}/oembed?url={}&format=json", ctx.site, enc(&page_link)));
            a.note("paste the link where oEmbed is read: the host embeds the player from it");
        }
        "preview" => {
            if ctx.stored {
                a.snippet = format!("{}/c/{}", ctx.site, ctx.cid);
                a.note("X plays it in the post; Discord, Slack and iMessage get its audio (og:audio); Mastodon shows the card");
            } else {
                // the link-preview page of a program the site does not keep: /c/<CID>.<packed>, from the sealed link
                let packed = ctx.sealed.strip_prefix(&format!("{}/#", ctx.site)).filter(|p| p.starts_with(ctx.cid));
                a.snippet = packed.map_or_else(|| sealed_or_site(ctx).to_string(), |p| format!("{}/c/{p}", ctx.site));
                a.less(format!("the site does not store it, so the preview is a card without sound: {STORE_FIRST} for the player and the audio"));
            }
            a.link("preview", a.snippet.clone());
        }
        "markdown" => {
            let n = md_text(&name(c, ctx));
            if ctx.stored {
                let card = format!("{}/card/{}.svg", ctx.site, ctx.cid);
                let listen = render_url(ctx);
                a.snippet = format!("[![{n}]({})]({})\n\n[Listen]({})\n", md_url(&card), md_url(&page_link), md_url(&listen));
                a.link("card", card);
                a.link("page", page_link);
                a.link("listen", listen);
            } else {
                a.snippet = format!("[{n}]({})\n", md_url(sealed_or_site(ctx)));
                a.less(format!("the site does not store it, so it is a link with no card and no audio: {STORE_FIRST}"));
            }
        }
        "jupyter" => {
            let html = serde_json::to_string(&web(c, ctx)).unwrap();
            a.snippet = format!("from IPython.display import HTML, display\n\ndisplay(HTML({html}))\n");
            a.note("a notebook runs the output's scripts only when it is trusted (one you ran yourself); it must reach cdn.jsdelivr.net");
            pinned_player_reads(c, ctx, a);
        }
        "vscode" => {
            let text = text_of(c, ctx);
            if text.is_empty() {
                return a.no("vscode needs the program's text");
            }
            let slug: String = name(c, ctx).to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            let slug = slug.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
            let file = format!("{}.beatscript", if slug.is_empty() { "program" } else { &slug });
            a.snippet = format!("Save the program as {file}, open it in VS Code with the BeatScript extension, and run \"BeatScript: Preview with the player\" from the Command Palette.");
            a.file = Some(json!({"name": file, "text": text}));
        }
        "audio" => {
            let src = render_url(ctx);
            a.snippet = format!("<audio controls preload=\"none\" src=\"{}\"></audio>", esc(&src));
            a.link("render", src);
            a.note("Ogg Opus does not play in older Safari");
            if c.audio() == "inline" {
                a.less("the render is linked by URL for now, not carried in the page");
            }
        }
        "rss" => {
            let src = render_url(ctx);
            // RSS 2.0 requires a length; 0 is what a feed gives when it does not know it
            a.snippet = format!("<enclosure url=\"{}\" length=\"0\" type=\"audio/ogg\"/>", esc(&src));
            a.link("render", src);
        }
        "android" => {
            a.snippet = match app_link(ctx) {
                Some(l) => l,
                None if ctx.stored => page_link,
                None => return a.no("android needs the program's text or a stored cid"),
            };
            a.note("the BeatScript app opens the link when it is installed; otherwise it opens on the site");
            a.less("the released app (0.3.30) claims /p only on the site's old hosts, so until its build for beatscript.app ships the link opens the site's editor");
        }
        "json" => a.snippet = serde_json::to_string(&c.json).unwrap(),
        _ => {}
    }
}

/// The pinned player reads `config` from 0.3.0: an older pin plays only what its own attributes say, so whatever the
/// config attribute carries (a set, a station, a title, features past highlight, theme auto ...) is not done there.
fn pinned_player_reads(c: &Config, ctx: &Ctx, a: &mut Ans) {
    if c.kind() != "player" {
        return;
    }
    let reads_config = ctx.player.version.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>() >= vec![0, 3, 0];
    let carried = to_attributes(c).map(|(_, attrs, _)| attrs.into_iter().any(|(k, _)| k == "config")).unwrap_or(false);
    if carried && !reads_config {
        a.less(format!("the pinned player ({}) does not read config yet, so what only config carries (a set, a station, a title, theme auto, features past highlight) is ignored until a player that reads it is published", ctx.player.version));
    }
}

/// The embed for `target` and a normalized config: {target, kind, works: works|degraded|unsupported, engine, notes,
/// snippet, page?, capabilities?, links?, file?, config}. notes say, one short sentence each, what does not work there
/// and what it does instead; capabilities is what an artifact publisher must declare.
pub fn answer(target: &str, c: &Config, ctx: &Ctx) -> Value {
    let kind = c.kind();
    let mut a = Ans { works: "works", notes: c.warnings.clone(), ..Ans::default() };
    match target_row(target) {
        None => a.no(format!("there is no target {target}: give one of {}", TARGETS.iter().map(|t| t.id).collect::<Vec<_>>().join(", "))),
        Some(t) if !t.kinds.contains(&kind) => a.no(format!("{} takes kind {}, not {kind}", t.id, t.kinds.join(" or "))),
        Some(t) => {
            a.engine = t.engine;
            fill(t, c, ctx, &mut a);
        }
    }
    let mut o = Map::new();
    o.insert("target".into(), json!(target));
    o.insert("kind".into(), json!(kind));
    o.insert("works".into(), json!(a.works));
    o.insert("engine".into(), json!(a.engine));
    o.insert("notes".into(), json!(a.notes));
    o.insert("snippet".into(), json!(a.snippet));
    if let Some(p) = a.page {
        o.insert("page".into(), json!(p));
    }
    if let Some(cap) = a.capabilities {
        o.insert("capabilities".into(), cap);
    }
    if !a.links.is_empty() {
        o.insert("links".into(), Value::Object(a.links));
    }
    if let Some(f) = a.file {
        o.insert("file".into(), f);
    }
    o.insert("config".into(), c.json.clone());
    Value::Object(o)
}

/// tools/list's entry.
pub fn tool_def(p: &str, max_prog: usize) -> Value {
    let ids: Vec<&str> = TARGETS.iter().map(|t| t.id).collect();
    let desc = format!(
        "A program (or a set, or a station) embedded anywhere, from one embed config ({p}/embed-config.schema.json, the same config the <beatscript-track> player and the <beatscript-studio> editor read): give a target and a config, and get the snippet or page for it, whether the config works there, is degraded or is unsupported, and one-sentence notes on what the target cannot do and what it does instead. Use the snippet as it is. \
Targets: {targets}. kind is player, studio (the whole editor as an MCP server inside the page: check, fmt, expand, generate, highlight, share, open, instruments, cheatsheet run offline on the wasm engine), card or audio; source is text, cid, set or station; audio is live, render, inline or url; features and capabilities are lists. \
With target artifact the answer is two text blocks, a JSON summary and the page: publish the page as an HTML page (an Artifact) exactly as it is, and declare the summary's capabilities (sample, mcp with the Beatscript connector's share, render and queue, db and user, downloads, room) when you publish it. Kind player or audio on target artifact is the artifact tool's page: kind audio (or audio inline) carries the site's render of a stored program in the page (a render not cached yet is limited as render's are), kind player plays it with the pinned player; kind studio loads @stackdump/beatscript-studio from cdn.jsdelivr.net at an exact version with Subresource Integrity. \
markdown (a GitHub README) gets an SVG card ({p}/card/<CID>.svg) and an audio link; iframe, oembed, preview, audio, rss and obsidian need a program the site stores (share it signed in, or render it); android is the self-sealing /p# link the app opens. \
Give the program as text, cid or example (or in the config's source). No sign-in, and it stores nothing unless store (artifact, player or audio only); billed as a check. Text at most {max_prog} bytes. See {p}/docs/embed; to build your own interface instead (the engine crates for native apps, the language service outside the studio, what is versioned), {p}/docs/sdk.",
        targets = ids.join(", "),
    );
    tool(
        "embed",
        "A program embedded anywhere",
        desc,
        [false, false, true, false],
        vec![
            ("target", sprop("Where it goes. artifact (the default) gives a whole page.", &[("enum", json!(ids))])),
            ("config", json!({"type": "object", "description": format!("The embed config ({p}/embed-config.schema.json): kind, source, audio, features, capabilities, theme, compact, width, height, title, site, bridge, pin. Defaults by kind.")})),
            ("text", sprop("The program text, instead of the config's source. It must pass check.", &[("maxLength", json!(max_prog))])),
            ("cid", sprop("CID of a program stored here, or of an example.", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))])),
            ("example", sprop("Name of an example, from list_examples.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))])),
            ("title", sprop("The name to show and publish under (default the program's `#! name:`).", &[("maxLength", json!(80))])),
            ("store", json!({"type": "boolean", "description": "Target artifact, kind player or audio: store the text first (signed in), as the artifact tool does."})),
        ],
        &[],
    )
}

/// The one program a call names, checked as the artifact tool checks it.
struct Named {
    text: String,
    cid: String,
    stored: bool,
    rights: Value,
    sealed: String,
}

impl Tools {
    /// text, cid or example: the program, its rights, whether the site stores it and its self-sealing link. The same
    /// gates as artifact: a private text, samples, a stored text under another CID are refused.
    fn embed_program(&self, c: &Caller, text: Option<String>, cid: Option<String>, example: Option<String>) -> Result<Named, Fail> {
        // text with cid: the CID the text must have (the schema's rule), checked below
        let text_cid = if text.is_some() { cid.clone() } else { None };
        let (text, rights, stored, want) = match (text, cid, example) {
            (Some(t), _, _) => {
                if t.len() > self.max_prog {
                    return Err(Fail::new(413, "too_large", format!("the program is {} bytes; the limit is {}", t.len(), self.max_prog)));
                }
                if crate::store::declares_not_public(&t) {
                    return Err(Fail::new(422, "restricted", "the program says `#! visibility:` something but public, so it is not embedded: private programs never leave the machine they were written on."));
                }
                let lic = t
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("#!").and_then(|h| h.trim().strip_prefix("license:")).map(|v| v.trim().to_string()))
                    .filter(|l| super::license_ok(l))
                    .unwrap_or_else(|| "CC0-1.0".into());
                (t, json!({"license": lic, "source": "text"}), false, String::new())
            }
            (None, cid, ex) => {
                let cid = match (cid, ex) {
                    (Some(cid), _) if is_cid(&cid) => cid,
                    (Some(_), _) => return Err(bad("argument \"cid\" is not a CID (it starts with z)")),
                    (None, Some(ex)) if name_ok(&ex) => self.example_cid(&ex).ok_or_else(|| Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&ex))))?,
                    (None, Some(_)) => return Err(bad("argument \"example\" is an example name, as list_examples gives")),
                    (None, None) => return Err(bad("give the program: text, cid or example (or the config's source)")),
                };
                let Some((t, _, meta)) = self.stored(&cid) else {
                    return Err(Fail::new(404, "not_found", format!("no program {cid} is stored here. Pass its text instead, or share it signed in to store it.")));
                };
                super::queue_gate(&t, &meta)?;
                let m = crate::compose::go_meta(&meta);
                let mut r = json!({"license": m["license"].as_str().filter(|l| !l.is_empty()).unwrap_or("CC0-1.0"), "source": if meta["source"] == "example" { "example" } else { "store" }});
                for k in ["credits", "forked_from", "composed_by"] {
                    if !m[k].is_null() {
                        r[k] = m[k].clone();
                    }
                }
                (t, r, true, cid)
            }
        };
        let rep = self.check_program(c, &text)?;
        let rr = Rep(&rep);
        if !rr.ok() || rr.cid().is_empty() {
            return Err(needs_fixing(&rep, "nothing was embedded"));
        }
        if let Some(e) = private(&rep, "it has nothing to play in an embed") {
            return Err(e);
        }
        let cid = rr.cid().to_string();
        if stored && want != cid {
            return Err(Fail::new(422, "cid_mismatch", format!("refused: the text stored under {want} is program {cid}. Do not trust or play it.")));
        }
        if let Some(w) = text_cid.filter(|w| *w != cid) {
            return Err(Fail::new(422, "cid_mismatch", format!("refused: the text is program {cid}, not {w}.")));
        }
        let canonical = rr.canonical().to_string();
        let sealed = match self.eng.share(&c.ip, &canonical) {
            Ok(s) if s["ok"] == true => self.site_links(s["url"].as_str().unwrap_or(""), "").0,
            _ => String::new(),
        };
        // the embed carries the formatted text: its `#!` header lines (the name, the licence) and comments, the same CID
        let formatted = beatscript_serve::autoformat::autoformat(&text, self.max_prog);
        Ok(Named { text: formatted, cid, stored, rights, sealed })
    }

    pub(super) fn embed_tool(&self, c: &Caller, a: Args) -> Result<Text, Fail> {
        a.only(&["target", "config", "text", "cid", "example", "title", "store"])?;
        let target = a.str("target", 16)?.filter(|t| !t.is_empty()).unwrap_or_else(|| "artifact".into());
        if target_row(&target).is_none() {
            return Err(bad(format!("argument \"target\" is one of {}", TARGETS.iter().map(|t| t.id).collect::<Vec<_>>().join(", "))));
        }
        let mut cfg_in = match a.present("config") {
            None => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok().filter(Value::is_object).ok_or_else(|| bad("argument \"config\" is the embed config: a JSON object"))?,
            Some(_) => return Err(bad("argument \"config\" is the embed config: a JSON object")),
        };
        let text_arg = match a.present("text") {
            Some(_) => Some(self.program(&a, "text")?),
            None => None,
        };
        let cid_arg = a.str("cid", 100)?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let example = a.str("example", 64)?.filter(|s| !s.is_empty());
        let title = a.str("title", 80)?.filter(|t| !t.trim().is_empty());
        let store = a.boolean("store")?;
        if [text_arg.is_some(), cid_arg.is_some(), example.is_some()].iter().filter(|b| **b).count() > 1 {
            return Err(bad("give at most one of text, cid, example"));
        }
        // the program named by an argument is the config's source (the config's own source otherwise)
        let named_by_arg = text_arg.is_some() || cid_arg.is_some() || example.is_some();
        if named_by_arg {
            if cfg_in.get("source").is_some() {
                return Err(bad("give the program either as text, cid or example, or as the config's source, not both"));
            }
            cfg_in["source"] = json!({});
        }
        if let Some(t) = &title {
            cfg_in["title"] = json!(t);
        }
        let mut cfg_probe = cfg_in.clone();
        if named_by_arg {
            // a placeholder source so the config checks; the real one is filled in once the program is resolved
            cfg_probe["source"] = json!({"text": "\n"});
        }
        let bad_config = |errs: Vec<(String, String)>| {
            let mut e = Fail::new(422, "bad_config", format!("the embed config does not check: {}", errs.iter().map(|(p, m)| format!("{} {m}", if p.is_empty() { "/" } else { p })).collect::<Vec<_>>().join("; ")));
            e.details = Some(json!({"errors": errs.iter().map(|(p, m)| json!({"path": p, "message": m})).collect::<Vec<_>>(), "schema": format!("{}/embed-config.schema.json", self.public)}));
            e
        };
        let cfg = normalize_for(&target, &cfg_probe).map_err(bad_config)?;
        let kind = cfg.kind().to_string();
        if store && !(target == "artifact" && (kind == "player" || kind == "audio")) {
            return Err(bad("store is for target artifact with kind player or audio (the studio stores through its own store tool, with the viewer's connector)"));
        }
        // kind player or audio on an artifact: the artifact tool's page, with its inline render and store
        if target == "artifact" && (kind == "player" || kind == "audio") {
            let src = cfg.source();
            let mut m = Map::new();
            match (&text_arg, &cid_arg, &example) {
                (Some(t), _, _) => drop(m.insert("text".into(), json!(t))),
                (_, Some(cid), _) => drop(m.insert("cid".into(), json!(cid))),
                (_, _, Some(ex)) => drop(m.insert("example".into(), json!(ex))),
                _ if src.get("text").is_some() => drop(m.insert("text".into(), src["text"].clone())),
                _ if src.get("cid").is_some() => drop(m.insert("cid".into(), src["cid"].clone())),
                _ => return Err(bad("target artifact with kind player or audio plays one program: give text, cid or example, or a text or cid source")),
            }
            if let Some(t) = cfg.title() {
                m.insert("title".into(), json!(t));
            }
            let inline = kind == "audio" || cfg.audio() == "inline";
            m.insert("audio".into(), json!(if inline { "inline" } else { "engine" }));
            if store {
                m.insert("store".into(), json!(true));
            }
            let Text::Page(mut sum, html) = self.artifact_tool(c, Args(m))? else {
                return Err(Fail::new(500, "internal", "the artifact tool gave no page"));
            };
            // the config as it applies: the program the page carries as its source (never the probe's placeholder)
            let mut full = cfg_in.clone();
            let src_text = text_arg.clone().or_else(|| cfg_in.get("source").and_then(|s| s.get("text")).and_then(Value::as_str).map(str::to_string));
            full["source"] = match src_text {
                Some(t) if sum["stored"] != true => json!({"text": t}),
                _ => json!({"cid": sum["cid"]}),
            };
            let cfg = normalize_for(&target, &full).map_err(bad_config)?;
            let mut notes: Vec<String> = cfg.warnings.iter().filter(|w| !(sum["audio"] == "inline" && w.starts_with("inline audio needs"))).cloned().collect();
            let mut works = "works";
            for f in cfg.features() {
                if ["editor", "mixer", "knobs", "compose"].contains(&f) {
                    notes.push(format!("{f} needs kind studio"));
                    works = "degraded";
                }
            }
            if cfg.audio() == "render" || cfg.theme() != "dark" || cfg.compact() || cfg.features().contains(&"highlight") {
                notes.push("the artifact player page plays live with its own look: audio render, highlight, theme and compact are not carried (use target web for those)".into());
                works = "degraded";
            }
            if kind == "audio" && sum["audio"] != "inline" {
                notes.push("there is no inline render for this program, so the page plays it with the engine".into());
                works = "degraded";
            }
            sum["embed"] = json!({"target": "artifact", "kind": kind, "works": works, "engine": sum["audio"] == "engine", "notes": notes, "config": cfg.json});
            return Ok(Text::Page(super::sorted(sum), html));
        }
        // one program (text, cid or example, or a text or cid source), or none (a set or a station)
        let src = cfg_in.get("source").cloned().unwrap_or(Value::Null);
        let (t, ci) = if named_by_arg {
            (text_arg, cid_arg)
        } else {
            (src.get("text").and_then(Value::as_str).map(str::to_string), src.get("cid").and_then(Value::as_str).map(str::to_string))
        };
        let named = if t.is_some() || ci.is_some() || example.is_some() { Some(self.embed_program(c, t, ci, example)?) } else { None };
        // a set: every item passes the same gates as one program (private, samples, a CID not stored or not its text)
        if let Some(items) = src.get("set").and_then(Value::as_array) {
            for (i, it) in items.iter().enumerate() {
                let (it_t, it_c) = (it.get("text").and_then(Value::as_str).map(str::to_string), it.get("cid").and_then(Value::as_str).map(str::to_string));
                self.embed_program(c, it_t, it_c, None).map_err(|mut e| {
                    e.err.message = format!("set item {}: {}", i + 1, e.err.message);
                    e
                })?;
            }
        }
        let cfg = if named_by_arg {
            let n = named.as_ref().expect("an argument names a program");
            let mut full = cfg_in.clone();
            full["source"] = if n.stored { json!({"text": n.text, "cid": n.cid}) } else { json!({"text": n.text}) };
            normalize_for(&target, &full).map_err(bad_config)?
        } else {
            cfg
        };
        let empty = json!({});
        let ctx = Ctx {
            site: &self.public,
            cid: named.as_ref().map_or("", |n| n.cid.as_str()),
            text: named.as_ref().map_or("", |n| n.text.as_str()),
            stored: named.as_ref().is_some_and(|n| n.stored),
            sealed: named.as_ref().map_or("", |n| n.sealed.as_str()),
            rights: named.as_ref().map_or(&empty, |n| &n.rights),
            player: PlayerCdn::get(),
            studio: StudioCdn::get(),
        };
        let mut ans = answer(&target, &cfg, &ctx);
        // audio inline on a page of the caller's own: the site's render as a data: URI, for a stored program
        if target == "audio" && cfg.kind() == "audio" && cfg.audio() == "inline" && ctx.stored && ans["works"] != "unsupported" {
            match self.inline_opus(c, ctx.cid) {
                Ok(b) if b.len().div_ceil(3) * 4 <= super::artifact::MAX_INLINE_B64 => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&b);
                    ans["snippet"] = json!(format!("<audio controls preload=\"metadata\" src=\"data:audio/ogg;base64,{b64}\"></audio>"));
                    ans["works"] = json!("works");
                    if let Some(n) = ans["notes"].as_array_mut() {
                        n.retain(|x| x.as_str().map_or(true, |s| !s.starts_with("the render is linked by URL") && !s.starts_with("inline audio needs")));
                    }
                }
                Ok(_) => push_note(&mut ans, "the render is over the inline limit, so it is linked by URL"),
                Err(why) => push_note(&mut ans, &format!("no inline render ({why}), so it is linked by URL")),
            }
        }
        if let Some(page) = ans.get("page").and_then(Value::as_str).map(str::to_string) {
            if let Some(o) = ans.as_object_mut() {
                o.remove("page");
                o.insert("html_bytes".into(), json!(page.len()));
                o.insert("publish".into(), json!("The second content block is the page: publish it as an HTML page (an Artifact) exactly as given, and declare `capabilities` when you publish it. Do not rewrite it: its scripts are pinned by version and Subresource Integrity."));
            }
            return Ok(Text::Page(super::sorted(ans), page));
        }
        Ok(Text::Json(ans))
    }
}

fn push_note(ans: &mut Value, s: &str) {
    if let Some(n) = ans["notes"].as_array_mut() {
        n.push(json!(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// embed-config.js's own cases (written by its tests): normalize, toAttributes and artifactCapabilities.
    const CASES: &str = include_str!("../../../../site/testdata/embed-config/cases.json");
    const CID: &str = "z4EBG9izEDKtK6ErJfgHcqL5VTiaPVyz3BvCzju4znbxpQjCNDi";
    const TEXT: &str = "#! name: Four on the floor\ntempo 120\ndeck A: kick x ~ x ~ x ~ x ~\n";
    const SITE: &str = "https://beatscript.app";

    fn unesc(s: &str) -> String {
        s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }

    /// The value of `attr="..."` on the first `<tag ` in a page.
    fn attr(page: &str, tag: &str, attr: &str) -> Option<String> {
        let i = page.find(&format!("<{tag} "))?;
        let el = &page[i..i + page[i..].find('>')?];
        let k = el.find(&format!(" {attr}=\""))? + attr.len() + 3;
        Some(unesc(&el[k..k + el[k..].find('"')?]))
    }

    fn sealed() -> String {
        format!("{SITE}/#{CID}.eJxLzs7PzS0pSc0FAA")
    }

    fn ctx<'a>(text: &'a str, stored: bool, rights: &'a Value, sealed: &'a str) -> Ctx<'a> {
        Ctx { site: SITE, cid: CID, text, stored, sealed, rights, player: PlayerCdn::get(), studio: StudioCdn::get() }
    }

    fn cfg(v: Value) -> Config {
        normalize(&v).unwrap_or_else(|e| panic!("{v}: {e:?}"))
    }

    #[test]
    fn the_javascript_cases() {
        let cases: Value = serde_json::from_str(CASES).unwrap();
        let cases = cases.as_array().unwrap();
        assert!(cases.len() >= 15, "{} cases", cases.len());
        let s = |v: &Value| serde_json::to_string(v).unwrap();
        for k in cases {
            let name = k["name"].as_str().unwrap();
            match normalize(&k["input"]) {
                Err(errs) => {
                    let want = k["errors"].as_array().unwrap_or_else(|| panic!("{name}: refused, {errs:?}"));
                    let paths: Vec<Value> = errs.iter().map(|e| json!(e.0)).collect();
                    assert_eq!(&paths, want, "{name}: {errs:?}");
                }
                Ok(c) => {
                    assert!(k["errors"].is_null(), "{name}: accepted, {}", c.json);
                    // byte for byte: the same keys in the same order, the same compact JSON
                    assert_eq!(s(&c.json), s(&k["normalized"]), "{name}");
                    let el = &k["element"];
                    match to_attributes(&c) {
                        None => assert!(el["tag"].is_null() && el["attrs"] == json!([]) && el["text"].is_null(), "{name}"),
                        Some((tag, attrs, text)) => {
                            assert_eq!(el["tag"], tag, "{name}");
                            assert_eq!(s(&json!(attrs.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>())), s(&el["attrs"]), "{name}");
                            assert_eq!(json!(text), el["text"], "{name}");
                        }
                    }
                    assert_eq!(s(&artifact_capabilities(&c, "Beatscript")), s(&k["capabilities"]), "{name}");
                }
            }
        }
    }

    /// The fixture pins error paths; these pin the messages too, as embed-config.js gave them (node 22, run by hand).
    #[test]
    fn the_javascript_messages() {
        let lines = r#"["{nope",[["","is not valid JSON"]],[]]
["null",null,["there is nothing to play: give text, cid, set or station"]]
[null,null,["there is nothing to play: give text, cid, set or station"]]
[5,[["","must be an object"]],[]]
[[],[["","must be an object"]],[]]
[{"v":2},[["/v","must be 1"]],[]]
[{"v":1.0,"kind":"widget"},[["/kind","must be one of player, studio, card, audio"]],[]]
[{"kind":null},[["/kind","must be one of player, studio, card, audio"]],[]]
[{"colour":"red","a/b~c":1},[["/a~1b~0c","is not a known key"],["/colour","is not a known key"]],[]]
[{"source":{}},[["/source","needs exactly one of text, cid, set or station"]],[]]
[{"source":{"text":"x","set":[{"text":"y"}]}},[["/source","needs exactly one of text, cid, set or station"]],[]]
[{"source":{"cid":"Qm"}},[["/source/cid","is not a program CID"]],[]]
[{"source":{"station":""}},[["/source/station","must not be empty"]],[]]
[{"source":{"station":"lofi","n":-1,"count":65}},[["/source/count","must be at most 64"],["/source/n","must be at least 0"]],[]]
[{"source":{"station":"lofi","n":1.5}},[["/source/n","must be an integer"]],[]]
[{"source":{"set":[]}},[["/source/set","must have at least 1 item"]],[]]
[{"source":{"set":[{"title":"x"},{"text":1}]}},[["/source/set/0","needs text or cid"],["/source/set/1/text","must be a string"]],[]]
[{"features":["editor","share","editor"]},[["/features","has \"editor\" twice"]],[]]
[{"features":"share"},[["/features","must be an array"]],[]]
[{"capabilities":["x","mcp"]},[["/capabilities/0","must be one of sample, mcp, db, downloads, room, dictation, tts"]],[]]
[{"theme":"blue","compact":"yes","width":79,"height":4097,"title":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","site":"http://x","bridge":1},[["/bridge","must be true or false"],["/compact","must be true or false"],["/height","must be at most 4096"],["/site","must start with https://"],["/theme","must be one of dark, light, auto"],["/title","must be at most 80 characters"],["/width","must be at least 80"]],[]]
[{"pin":{"version":"latest","sri":"no","x":1}},[["/pin/sri","must be true or false"],["/pin/version","is not a version such as 1.2.3"],["/pin/x","is not a known key"]],[]]
[{"source":{"text":"a","cid":"z4EBG9izEDKtK6ErJfgHcqL5VTiaPVyz3BvCzju4znbxpQjCNDi","n":3},"audio":"inline","bridge":true},null,["n and count apply only to a station","inline audio needs a stored program: a cid alone","bridge applies only to the studio"]]
[{"width":100000000000000000000},[["/width","must be at most 4096"]],[]]
[{"$schema":3},[["/$schema","must be a string"]],[]]
[{"\ud83c\udfb5":1,"\uff01":1,"b":1},[["/b","is not a known key"],["/\uff01","is not a known key"],["/\ud83c\udfb5","is not a known key"]],[]]
[{"kind":"constructor"},[["/kind","must be one of player, studio, card, audio"]],[]]
[{"kind":"__proto__"},[["/kind","must be one of player, studio, card, audio"]],[]]"#;
        let mut cases: Vec<Value> = lines.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        cases.push(json!([{"source": {"set": vec![json!({"cid": CID}); 65]}}, [["/source/set", "must have at most 64 items"]], []]));
        cases.push(json!([{"source": {"text": "x".repeat(65537)}}, [["/source/text", "must be at most 65536 characters"]], []]));
        for k in cases {
            let got = match normalize(&k[0]) {
                Ok(c) => json!([k[0], null, c.warnings]),
                Err(e) => json!([k[0], e.iter().map(|(p, m)| json!([p, m])).collect::<Vec<_>>(), []]),
            };
            assert_eq!(got, k);
        }
    }

    #[test]
    fn the_studio_lock_pins_one_exact_version() {
        let l = StudioCdn::get();
        assert_eq!(l.package, "@stackdump/beatscript-studio");
        assert_eq!(l.entry, "beatscript-studio.js");
        let parts: Vec<&str> = l.version.split('.').collect();
        assert!(parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())), "version {:?} is not an exact x.y.z", l.version);
        assert!(!l.integrity(&l.entry).is_empty(), "the lock has no integrity for its entry");
        for (f, i) in &l.files {
            if l.placeholder {
                // a placeholder lock is all placeholders, never half real
                assert!(i.contains("PLACEHOLDER"), "{f}: {i} in a placeholder lock");
                continue;
            }
            use base64::Engine as _;
            let raw = base64::engine::general_purpose::STANDARD.decode(i.strip_prefix("sha384-").unwrap_or_else(|| panic!("{f}: {i}"))).unwrap();
            assert_eq!(raw.len(), 48, "{f}: a sha384 is 48 bytes");
        }
    }

    /// The studio page: the lock's package at its exact version, the entry with its integrity and crossorigin, an import
    /// map before it with every other module's integrity, and nothing else loaded.
    #[test]
    fn the_studio_page_loads_the_pinned_studio() {
        let l = StudioCdn::get();
        let rights = json!({"license": "CC0-1.0", "source": "text"});
        let c = cfg(json!({"kind": "studio", "source": {"text": TEXT}}));
        let a = answer("artifact", &c, &ctx(TEXT, false, &rights, &sealed()));
        let page = a["page"].as_str().expect("a page");
        let want = format!("<script type=\"module\" src=\"https://cdn.jsdelivr.net/npm/@stackdump/beatscript-studio@{}/{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>", l.version, l.entry, l.integrity(&l.entry));
        let i = page.find(&want).unwrap_or_else(|| panic!("no pinned studio script; want {want}"));
        let m0 = page.find("<script type=\"importmap\">").expect("an import map");
        assert!(m0 < i, "the import map comes before the module script");
        let body = &page[m0 + "<script type=\"importmap\">".len()..];
        let map: Value = serde_json::from_str(&body[..body.find("</script>").unwrap()]).unwrap();
        let integ = map["integrity"].as_object().unwrap();
        let mods: Vec<&(String, String)> = l.modules().collect();
        assert!(!mods.is_empty());
        assert_eq!(integ.len(), mods.len(), "one integrity per module");
        for (f, sri) in mods {
            assert_eq!(integ.get(&l.url(f)).and_then(Value::as_str), Some(sri.as_str()), "{f}");
        }
        assert_eq!(page.matches("<script").count(), 2, "the import map and the studio, nothing else");
        assert_eq!(page.matches(" src=").count(), 1);
        for bad in ["@latest", "@^", "@~", "<style", "style=", "<iframe", "autoplay"] {
            assert!(!page.contains(bad), "page contains {bad:?}");
        }
        assert_eq!(attr(page, "beatscript-studio", "text").as_deref(), Some(TEXT), "the program round-trips");
        assert!(page.contains("<title>Four on the floor</title>"));
        assert_eq!(a["capabilities"], artifact_capabilities(&c, "Beatscript"));
        assert_eq!(a["capabilities"], json!({"sample": {}, "mcp": {"servers": [{"server": "Beatscript", "tools": ["share", "render", "queue"]}]}, "db": {}, "user": {}, "downloads": true}));
        assert_eq!(a["engine"], true);
        assert_eq!(a["works"], if l.placeholder { "degraded" } else { "works" }, "{a}");
        // the web snippet is the same tags and element
        let w = answer("web", &c, &ctx(TEXT, false, &rights, &sealed()));
        let snip = w["snippet"].as_str().unwrap();
        let el = snip.strip_prefix(&l.tags()).expect("the web snippet starts with the tags");
        assert!(page.contains(&l.tags()) && page.contains(el.trim_end()), "the page has the same tags and element");
    }

    #[test]
    fn hostile_titles_and_texts_are_escaped() {
        let text = "#! name: a \"b\" </script><b>x</b>\n# it's & so <!-- </SCRIPT >\ntempo 120\ndeck A: kick x ~ x ~\n";
        let title = "t </title><script>alert(1)</script>";
        let rights = json!({"license": "CC0-1.0", "source": "text"});
        let sealed = sealed();
        let c = cfg(json!({"kind": "studio", "title": title, "source": {"text": text}}));
        let x = ctx(text, false, &rights, &sealed);
        let page = studio_page(&c, &x);
        let low = page.to_ascii_lowercase();
        assert_eq!(low.matches("<script").count(), 2, "{page}");
        assert_eq!(low.matches("</script").count(), 2, "{page}");
        assert!(!page.contains("<!--") && !page.contains("<b>"), "{page}");
        assert!(page.contains("<title>t &lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;</title>"), "{page}");
        // the text and the title come back exactly as given
        assert_eq!(attr(&page, "beatscript-studio", "text").as_deref(), Some(text));
        let conf: Value = serde_json::from_str(&attr(&page, "beatscript-studio", "config").unwrap()).unwrap();
        assert_eq!(conf["title"], title);
        // the player page (the artifact tool's Page) and the web and notebook snippets
        let p = cfg(json!({"source": {"text": text}}));
        let pp = player_page(&p, &x).unwrap();
        assert_eq!(pp.to_ascii_lowercase().matches("</script").count(), 2);
        assert_eq!(attr(&pp, "beatscript-track", "text").as_deref(), Some(text));
        let j = answer("jupyter", &c, &x);
        let snip = j["snippet"].as_str().unwrap();
        let lit = &snip[snip.find("HTML(").unwrap() + 5..snip.rfind("))").unwrap()];
        assert_eq!(serde_json::from_str::<String>(lit).unwrap(), web(&c, &x), "the Python literal is the web embed");
        // Markdown: a name cannot make a link of its own
        let m = cfg(json!({"kind": "card", "title": "x](javascript:alert(1)) [y", "source": {"cid": CID}}));
        let s = answer("markdown", &m, &ctx(TEXT, true, &rights, &sealed))["snippet"].as_str().unwrap().to_string();
        assert!(s.starts_with("[![x\\]\\(javascript\\:alert\\(1\\)\\) \\[y]("), "{s}");
        let i = answer("iframe", &cfg(json!({"title": "\"><script>", "source": {"cid": CID}})), &ctx(TEXT, true, &rights, &sealed));
        assert!(!i["snippet"].as_str().unwrap().contains("<script"), "{i}");
    }

    /// Each target, for a stored CID, for a text the site does not store, and for a kind it cannot take.
    #[test]
    fn every_target_answers() {
        let rights = json!({"license": "CC0-1.0", "source": "store"});
        let sealed = sealed();
        // (target, works for a stored cid, works for an unstored text)
        let want = [
            ("artifact", "works", "works"),
            ("web", "works", "works"),
            ("iframe", "works", "unsupported"),
            ("oembed", "works", "unsupported"),
            ("preview", "works", "degraded"),
            ("markdown", "works", "degraded"),
            ("jupyter", "works", "works"),
            ("vscode", "works", "works"),
            ("obsidian", "works", "degraded"),
            ("audio", "degraded", "unsupported"),
            ("rss", "works", "unsupported"),
            ("android", "degraded", "degraded"),
            ("json", "works", "works"),
        ];
        assert_eq!(want.iter().map(|w| w.0).collect::<Vec<_>>(), TARGETS.iter().map(|t| t.id).collect::<Vec<_>>());
        for (t, (id, stored, unstored)) in TARGETS.iter().zip(want) {
            assert!(!t.title.is_empty() && t.note.ends_with('.'), "{id}");
            assert_eq!(t.needs_stored, matches!(id, "iframe" | "oembed" | "preview" | "markdown" | "obsidian" | "audio" | "rss"), "{id}");
            let kind = t.kinds[0];
            let c = cfg(json!({"kind": kind, "source": {"cid": CID}}));
            let a = answer(id, &c, &ctx(TEXT, true, &rights, &sealed));
            assert_eq!((a["target"].as_str(), a["kind"].as_str()), (Some(id), Some(kind)));
            assert_eq!(a["works"], stored, "{id} stored: {a}");
            assert!(!a["snippet"].as_str().unwrap().is_empty(), "{id} stored: {a}");
            assert_eq!(a["config"], c.json);
            if a["works"] == "works" {
                assert_eq!(a["engine"], t.engine, "{id}");
            }
            let c = cfg(json!({"kind": kind, "source": {"text": TEXT}}));
            let a = answer(id, &c, &ctx(TEXT, false, &rights, &sealed));
            assert_eq!(a["works"], unstored, "{id} unstored: {a}");
            let notes: Vec<&str> = a["notes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            if unstored == "unsupported" {
                assert!(notes.contains(&STORE_FIRST), "{id}: {notes:?}");
                assert_eq!(a["engine"], false);
            } else {
                assert!(!a["snippet"].as_str().unwrap().is_empty(), "{id} unstored: {a}");
                assert!(!a["snippet"].as_str().unwrap().contains(&format!("/p/{CID}")), "{id}: an unstored program never links /p/<CID>: {a}");
            }
            if unstored == "degraded" && id != "android" {
                assert!(notes.iter().any(|n| n.contains(STORE_FIRST)), "{id}: {notes:?}");
            }
            for bad in KINDS.iter().filter(|k| !t.kinds.contains(k)) {
                let c = cfg(json!({"kind": bad, "source": {"cid": CID}}));
                let a = answer(id, &c, &ctx(TEXT, true, &rights, &sealed));
                assert_eq!((a["works"].as_str(), a["engine"].as_bool()), (Some("unsupported"), Some(false)), "{id} {bad}: {a}");
                assert_eq!(a["notes"][0], format!("{id} takes kind {}, not {bad}", t.kinds.join(" or ")));
            }
        }
        // a set has no one program: the CID targets say so
        let set = cfg(json!({"source": {"set": [{"cid": CID}]}}));
        let a = answer("iframe", &set, &Ctx { cid: "", text: "", ..ctx(TEXT, true, &rights, &sealed) });
        assert_eq!((a["works"].as_str(), a["notes"][0].as_str()), (Some("unsupported"), Some(ONE_PROGRAM)));
        assert_eq!(answer("nope", &set, &ctx(TEXT, true, &rights, &sealed))["works"], "unsupported");
    }

    #[test]
    fn the_snippets() {
        let rights = json!({"license": "CC0-1.0", "source": "store"});
        let sealed = sealed();
        let at = |t: &str, c: Value, stored: bool| answer(t, &cfg(c), &ctx(TEXT, stored, &rights, &sealed));
        let cid = json!({"cid": CID});
        assert_eq!(
            at("iframe", json!({"source": cid, "compact": true, "theme": "light"}), true)["snippet"],
            format!("<iframe src=\"{SITE}/embed/{CID}?compact=1&amp;theme=light\" width=\"480\" height=\"80\" title=\"Four on the floor · BeatScript player\" sandbox=\"{SANDBOX}\" loading=\"lazy\" referrerpolicy=\"strict-origin-when-cross-origin\" frameborder=\"0\"></iframe>")
        );
        assert!(at("iframe", json!({"source": cid, "width": 600, "height": 200}), true)["snippet"].as_str().unwrap().contains(" width=\"600\" height=\"200\" "));
        let o = at("oembed", json!({"source": cid}), true);
        assert_eq!(o["snippet"], format!("{SITE}/p/{CID}"));
        assert_eq!(o["links"]["oembed"], format!("{SITE}/oembed?url=https%3A%2F%2Fbeatscript.app%2Fp%2F{CID}&format=json"));
        assert_eq!(at("preview", json!({"kind": "card", "source": cid}), true)["snippet"], format!("{SITE}/c/{CID}"));
        assert_eq!(at("preview", json!({"kind": "card", "source": {"text": TEXT}}), false)["snippet"], format!("{SITE}/c/{CID}.eJxLzs7PzS0pSc0FAA"));
        assert_eq!(
            at("markdown", json!({"kind": "card", "source": cid}), true)["snippet"],
            format!("[![Four on the floor]({SITE}/card/{CID}.svg)]({SITE}/p/{CID})\n\n[Listen]({SITE}/api/render/{CID}?format=opus)\n")
        );
        assert_eq!(at("markdown", json!({"kind": "card", "source": {"text": TEXT}}), false)["snippet"], format!("[Four on the floor]({sealed})\n"));
        assert_eq!(at("audio", json!({"kind": "audio", "audio": "url", "source": cid}), true)["snippet"], format!("<audio controls preload=\"none\" src=\"{SITE}/api/render/{CID}?format=opus\"></audio>"));
        assert_eq!(at("audio", json!({"kind": "audio", "audio": "url", "source": cid}), true)["works"], "works");
        assert_eq!(at("rss", json!({"kind": "audio", "source": cid}), true)["snippet"], format!("<enclosure url=\"{SITE}/api/render/{CID}?format=opus\" length=\"0\" type=\"audio/ogg\"/>"));
        assert_eq!(at("android", json!({"source": cid}), true)["snippet"], format!("{SITE}/p#{CID}.eJxLzs7PzS0pSc0FAA"));
        let no_seal = answer("android", &cfg(json!({"source": cid})), &Ctx { sealed: "", ..ctx(TEXT, true, &rights, &sealed) });
        assert_eq!(no_seal["snippet"], format!("{SITE}/p/{CID}"));
        let v = at("vscode", json!({"source": {"text": TEXT}}), false);
        assert_eq!(v["file"], json!({"name": "four-on-the-floor.beatscript", "text": TEXT}));
        let j = at("json", json!({"source": {"text": TEXT}}), false);
        assert_eq!(serde_json::from_str::<Value>(j["snippet"].as_str().unwrap()).unwrap(), j["config"]);
        // web: the player's pinned tags, then the element with the program in its text attribute
        let w = at("web", json!({"source": {"text": TEXT}, "audio": "render"}), false);
        assert_eq!(w["snippet"], format!("{}<beatscript-track mode=\"render\" text=\"{}\"></beatscript-track>\n", PlayerCdn::get().tags(), esc(TEXT)));
    }

    /// The player artifact is the artifact tool's own page, with the live player.
    #[test]
    fn the_player_page_is_the_artifact_page() {
        let rights = json!({"license": "CC0-1.0", "source": "text"});
        let sealed = sealed();
        let x = ctx(TEXT, false, &rights, &sealed);
        let a = answer("artifact", &cfg(json!({"source": {"text": TEXT}})), &x);
        let page = a["page"].as_str().unwrap();
        assert_eq!((a["works"].as_str(), a["engine"].as_bool()), (Some("works"), Some(true)));
        assert!(page.contains(&PlayerCdn::get().tags()), "the pinned player");
        assert_eq!(attr(page, "beatscript-track", "cid").as_deref(), Some(CID));
        assert_eq!(attr(page, "beatscript-track", "text").as_deref(), Some(TEXT));
        assert!(page.contains(&format!("href=\"{}\"", esc(&sealed))) && !page.contains(&format!("/p/{CID}")), "an unstored program links the sealed link");
        assert!(a["capabilities"].is_null());
        // a title the text does not have goes in as its `#! name:` header line, as the artifact tool does it
        let t = "tempo 120\ndeck A: kick x ~ x ~\n";
        let p = player_page(&cfg(json!({"title": "Kick test", "source": {"text": t}})), &ctx(t, true, &rights, &sealed)).unwrap();
        assert_eq!(attr(&p, "beatscript-track", "text").as_deref(), Some("#! name: Kick test\ntempo 120\ndeck A: kick x ~ x ~\n"));
        assert!(p.contains(&format!("href=\"{SITE}/p/{CID}\"")), "a stored program links /p/<CID>");
        // what the page cannot do yet, it says
        let r = answer("artifact", &cfg(json!({"source": {"text": TEXT}, "audio": "render", "theme": "light"})), &x);
        assert_eq!(r["works"], "degraded");
        assert!(r["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("audio render, theme")), "{r}");
        // a set has no player page: the web embed instead
        let s = answer("artifact", &cfg(json!({"source": {"set": [{"cid": CID}]}})), &Ctx { cid: "", text: "", ..ctx(TEXT, true, &rights, &sealed) });
        assert!(s["page"].is_null() && s["works"] == "degraded" && s["engine"] == true, "{s}");
        assert!(s["snippet"].as_str().unwrap().contains("<beatscript-track config="));
    }

    #[test]
    fn a_config_without_a_kind_takes_the_targets() {
        assert_eq!(normalize_for("audio", &json!({"source": {"cid": CID}})).unwrap().kind(), "audio");
        assert_eq!(normalize_for("markdown", &json!("{\"source\":{\"cid\":\"z4EBG9izEDKtK6ErJfgHcqL5VTiaPVyz3BvCzju4znbxpQjCNDi\"}}")).unwrap().kind(), "card");
        assert_eq!(normalize_for("audio", &json!({"kind": "player", "source": {"cid": CID}})).unwrap().kind(), "player");
        assert_eq!(normalize_for("web", &Value::Null).unwrap().kind(), "player");
        assert!(normalize_for("web", &json!("{nope")).is_err());
    }

    /// The frame is the one cards/embed.rs serves (its constants are private to `cards`).
    #[test]
    fn the_frame_is_the_sites() {
        let src = include_str!("../cards/embed.rs");
        assert!(src.contains(&format!("const SANDBOX: &str = \"{SANDBOX}\";")));
        assert!(src.contains(&format!("const SIZE: (u32, u32) = ({}, {});", SIZE.0, SIZE.1)));
        assert!(src.contains(&format!("const SIZE_COMPACT: (u32, u32) = ({}, {});", SIZE_COMPACT.0, SIZE_COMPACT.1)));
        assert!(src.contains("\"compact\" =>") && src.contains("\"theme\" => l.light = v == \"light\""), "the frame reads compact and theme=light");
    }
}
