//! P5: the share cards and the CID-only share links (Go `internal/btwsite/card`, `server/shares.go`).
//!
//!   GET  /c/<cid>[.<packed>][.jsonld]   the link-preview page (og:/twitter: tags) of a program, or its JSON-LD
//!   GET  /p/<cid>[.jsonld]              the short link's page: the same, and a person is sent to /#<cid>
//!   GET  /card/<cid>[.<packed>][.square].png, /card/default[.square].png, /card/go.js, /card/card.css
//!   GET  /card/<cid>.svg, /card/default.svg   the card as an SVG, for a README (cards/svg.rs)
//!   PUT  /api/shares/<cid>              a signed-in (cookie, bearer, BtwDevice) store of a public program
//!
//! Pages, JSON-LD, headers and errors are Go's to the byte (the page's packed text is Go's own flate output:
//! `goflate`). The PNG is drawn by `draw` and matches Go's perceptually, not bitwise. PNGs are cached in
//! `<data>/cards/` under Go's file names (`<cid>.<variant>.v2[.s<site>][.<name+licence hash>].png`), LRU-capped at
//! 256 MiB, written beside and renamed, so either server can use the other's files.

mod draw;
pub(crate) mod goflate; // also the hub's pairing QR PNG (hub::qr)
mod lex;
mod region;
mod svg;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::any;
use axum::Router;
use base64::Engine;
use bytes::Bytes;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{content, go_body, go_num, AppState};
use draw::{Content, Section, Variant};
use lex::Span;

/// Part of every cached file's name (Go `version`).
const VERSION: &str = "2";
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_PAGE: &str = "public, max-age=3600";
const CACHE_SHORT: &str = "public, max-age=300";
const GENERIC_TITLE: &str = "BeatScript: write a whole track as text";
const GENERIC_DESC: &str = "AI-assisted algorithmic composition";
const PRIVATE_TITLE: &str = "a private BeatScript program";
const PRIVATE_DESC: &str = "This BeatScript program is private. AI-assisted algorithmic composition.";
const GENERIC_ALT: &str = "BeatScript: a few lines of code on a dark stage, over a strip of song sections";
const MAX_TEXT: usize = 32 << 10;
const MAX_LINK_TEXT: usize = 1 << 20;
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// The share page's styles, served at `/card/card.css` (the site's CSP is `style-src 'self'`: no inline `<style>`).
const CARD_CSS: &str = include_str!("cards/card.css");
const GO_JS: &str = "(function () {\n  var a = document.getElementById('open');\n  var h = a && a.getAttribute('href');\n  if (h) { location.replace(h); }\n})();\n";

// ---------- errors as Go's card service and server answer them ----------

/// An `*api.Error` (status, code, message, retry), or a checker failure.
#[derive(Clone, Debug)]
struct CardErr {
    status: u16,
    code: &'static str,
    message: String,
    retry: u64,
}

fn errf(status: u16, code: &'static str, message: impl Into<String>) -> CardErr {
    CardErr { status, code, message: message.into(), retry: 0 }
}

/// `http.Error`: text, nosniff, the message and a newline.
fn http_error(mut h: HeaderMap, status: u16, msg: &str) -> Response<Body> {
    h.remove(header::CONTENT_LENGTH);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    let mut r = Response::new(Body::from(format!("{msg}\n")));
    *r.status_mut() = axum::http::StatusCode::from_u16(status).unwrap();
    *r.headers_mut() = h;
    r
}

fn not_found(h: HeaderMap) -> Response<Body> {
    http_error(h, 404, "404 page not found")
}

/// `api.WriteError`, with `details` when set (the share's check report).
fn write_error(status: u16, code: &str, message: &str, retry: u64, details: Option<Value>) -> Response<Body> {
    let mut e = Map::new();
    e.insert("code".into(), json!(code));
    e.insert("message".into(), json!(message));
    if retry > 0 {
        e.insert("retry_after".into(), json!(retry));
    }
    if let Some(d) = details {
        e.insert("details".into(), d);
    }
    let body = go_json(&json!({ "error": Value::Object(e) })) + "\n";
    let mut r = Response::builder().status(status).header(header::CONTENT_TYPE, "application/json; charset=utf-8");
    if retry > 0 {
        r = r.header(header::RETRY_AFTER, retry.to_string());
    }
    let mut r = r.body(go_body(body)).unwrap();
    if status == 413 {
        // net/http's reason phrase (hyper says "Payload Too Large")
        r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Request Entity Too Large"));
    }
    r
}

/// encoding/json's string escaping: serde's, plus `<`, `>`, `&`, U+2028 and U+2029.
fn go_json(v: &Value) -> String {
    go_escape(serde_json::to_string(v).unwrap())
}

fn go_escape(s: String) -> String {
    s.replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// A body a Go handler wrote with `w.Write`: `go_body` for GET; for HEAD no body, and the length only when the whole
/// body fit net/http's 2048-byte buffer.
fn written(method: &Method, h: HeaderMap, body: Vec<u8>) -> Response<Body> {
    let b = if *method == Method::HEAD {
        // a sized body: hyper writes its length last, as net/http does, and sends no body on HEAD
        if body.len() <= 2048 {
            Body::from(body)
        } else {
            content::nothing()
        }
    } else {
        go_body(body)
    };
    let mut r = Response::new(b);
    *r.headers_mut() = h;
    r
}

// ---------- the program store's meta (api.Meta) ----------

#[derive(Clone, Debug, Default)]
struct Meta {
    license: String,
    visibility: String,
    name: String,
    composed_by: String,
    forked_from: String,
    credits: Vec<String>,
    prompt_sha: String,
    author: String,
    created: String,
    source: String,
    /// The doc comments of the text as it was shared (djlive::doc: `[{kind, name, doc}]`), read by the server from that
    /// text, as `name` is from its `#! name:`: the canonical text the store keeps has no comments, so they are kept
    /// beside it. The program's JSON-LD gives each documented section its `description` from them.
    docs: Vec<(String, String, String)>,
    /// What the text as shared said it vendored: its `# from: <call> -> <cid>` lines as `<call> -> <cid>` (gate::Admitted's
    /// `based_on`, which gate::write_program keeps). The program's JSON-LD lists each in `isBasedOn` as `vendored`, and
    /// lineage counts them (`vendoredFrom`).
    based_on: Vec<String>,
}

impl Meta {
    fn from_json(v: &Value) -> Meta {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Meta {
            license: s("license"),
            visibility: s("visibility"),
            name: s("name"),
            composed_by: s("composed_by"),
            forked_from: s("forked_from"),
            credits: v.get("credits").and_then(Value::as_array).map(|a| a.iter().map(|c| c.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default(),
            prompt_sha: s("prompt_sha256"),
            author: s("author"),
            created: s("created"),
            source: s("source"),
            docs: v
                .get("docs")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|d| {
                            let f = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_string);
                            Some((f("kind")?, f("name")?, f("doc")?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            based_on: v.get("based_on").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        }
    }
    /// The doc comments of the text as it was shared (`meta.docs`), as djlive's (they have no place in the canonical text:
    /// their spans are empty).
    fn doc_comments(&self) -> Vec<djlive::doc::DocComment> {
        self.docs
            .iter()
            .filter_map(|(k, n, d)| {
                let kind = ["section", "let", "def", "arrange"].into_iter().find(|w| w == k)?;
                Some(djlive::doc::DocComment { kind, name: n.clone(), doc: d.clone(), item: djlive::Span::new(0, 0), comment: djlive::Span::new(0, 0) })
            })
            .collect()
    }
    /// The doc of section `name`, if the shared text gave it one.
    fn section_doc(&self, name: &str) -> Option<&str> {
        self.docs.iter().find(|(k, n, _)| k == "section" && n == name).map(|(_, _, d)| d.as_str())
    }
    /// As encoding/json writes api.Meta (field order, omitempty).
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("license".into(), json!(self.license));
        m.insert("visibility".into(), json!(self.visibility));
        let mut opt = |k: &str, v: &str| {
            if !v.is_empty() {
                m.insert(k.into(), json!(v));
            }
        };
        opt("name", &self.name);
        opt("composed_by", &self.composed_by);
        opt("forked_from", &self.forked_from);
        if !self.credits.is_empty() {
            m.insert("credits".into(), json!(self.credits));
        }
        let mut opt = |k: &str, v: &str| {
            if !v.is_empty() {
                m.insert(k.into(), json!(v));
            }
        };
        opt("prompt_sha256", &self.prompt_sha);
        opt("author", &self.author);
        opt("created", &self.created);
        opt("source", &self.source);
        if !self.docs.is_empty() {
            m.insert("docs".into(), Value::Array(self.docs.iter().map(|(k, n, d)| json!({"kind": k, "name": n, "doc": d})).collect()));
        }
        if !self.based_on.is_empty() {
            m.insert("based_on".into(), json!(self.based_on));
        }
        Value::Object(m)
    }
}

// ---------- the check report a card is drawn from ----------

#[derive(Clone, Debug, Default)]
struct Report {
    ok: bool,
    cid: String,
    canonical: String,
    restricted: bool,
    uses_assets: bool,
    key: String,
    tempo: Option<f64>,
    seconds: Option<f64>,
    song_bars: i64,
    sections: Vec<Section>,
}

impl Report {
    fn from_json(v: &Value) -> Report {
        Report {
            ok: v["ok"] == true,
            cid: v["cid"].as_str().unwrap_or("").into(),
            canonical: v["canonical"].as_str().unwrap_or("").into(),
            restricted: v["restricted"] == true,
            uses_assets: v["uses_assets"] == true,
            key: v["key"].as_str().unwrap_or("").into(),
            tempo: v["tempo"].as_f64(),
            seconds: v["seconds"].as_f64(),
            song_bars: v["song_bars"].as_i64().unwrap_or(0),
            sections: v["sections"]
                .as_array()
                .map(|a| a.iter().map(|s| Section { name: s["name"].as_str().unwrap_or("").into(), start: s["start"].as_i64().unwrap_or(0), bars: s["bars"].as_i64().unwrap_or(0) }).collect())
                .unwrap_or_default(),
        }
    }
}

// ---------- programs ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Ok,
    Unknown,
    Mismatch,
    Restricted,
    Private,
    Invalid,
}

fn state_name(s: St) -> &'static str {
    match s {
        St::Unknown => "unknown",
        St::Mismatch => "mismatch",
        St::Restricted => "restricted",
        St::Private => "private",
        St::Invalid => "invalid",
        St::Ok => "ok",
    }
}

/// noindex for these (an unknown CID may be stored tomorrow, so it does not get one).
fn no_index(s: St) -> bool {
    s != St::Ok && s != St::Unknown
}

#[derive(Clone, Debug)]
struct Prog {
    cid: String,
    canonical: String,
    report: Option<Arc<Report>>,
    meta: Meta,
    desc: String,
    ex_name: String,
    ex_desc: String,
    st: St,
    known: bool,
    packed: String,
}

impl Prog {
    fn bare(cid: &str, st: St) -> Prog {
        Prog { cid: cid.into(), canonical: String::new(), report: None, meta: Meta::default(), desc: String::new(), ex_name: String::new(), ex_desc: String::new(), st, known: false, packed: String::new() }
    }
    fn state(&self) -> St {
        if self.st != St::Ok {
            return self.st;
        }
        let Some(r) = &self.report else { return St::Invalid };
        if !r.ok || r.cid.is_empty() {
            return St::Invalid;
        }
        if r.restricted || r.uses_assets {
            return St::Restricted;
        }
        if matches!(self.meta.visibility.trim().to_lowercase().as_str(), "private" | "restricted" | "unlisted-private") {
            return St::Private;
        }
        St::Ok
    }
    fn label(&self) -> String {
        let n = lex::clean_text(&self.meta.name, 60);
        if !n.is_empty() {
            return n;
        }
        let n = lex::clean_text(&self.ex_name, 60);
        if !n.is_empty() {
            return n;
        }
        format!("BeatScript {}", lex::short_cid(&self.cid))
    }
}

fn name_hash(p: &Prog) -> String {
    let mut n = lex::clean_text(&p.meta.name, 60);
    if n.is_empty() {
        n = lex::clean_text(&p.ex_name, 60);
    }
    if n.is_empty() {
        return String::new();
    }
    hex::encode(&Sha256::digest(n.as_bytes())[..4])
}

fn license_hash(p: &Prog) -> String {
    let l = lex::license_badge(&p.meta.license);
    if l == "CC0" {
        return String::new();
    }
    hex::encode(&Sha256::digest(l.as_bytes())[..2])
}

/// `^z[1-9A-HJ-NP-Za-km-z]{20,120}$`.
fn valid_cid(c: &str) -> bool {
    let b = c.as_bytes();
    b.first() == Some(&b'z') && (21..=121).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// Go `btw.Parse` of `<cid>.<packed>[.<sig>][~<notes>]`: the CID and the text.
fn parse_link(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let frag = s.find('#').map_or(s, |i| &s[i + 1..]);
    let (sealed, notes) = match frag.split_once('~') {
        Some((a, b)) => (a, Some(b)),
        None => (frag, None),
    };
    let parts: Vec<&str> = sealed.split('.').collect();
    let cid = parts[0];
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if cid.is_empty() || !cid.chars().all(|c| B58.contains(c)) {
        return None;
    }
    let mut text = String::new();
    if parts.len() > 1 && !parts[1].is_empty() {
        text = unpack(parts[1])?;
    }
    if let Some(n) = notes {
        unpack(n)?;
    }
    Some((cid.to_string(), text))
}

fn unpack(s: &str) -> Option<String> {
    let s: String = s.trim_end_matches('=').chars().filter(|c| *c != '\r' && *c != '\n').collect();
    let cfg = base64::engine::GeneralPurposeConfig::new().with_encode_padding(false).with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone).with_decode_allow_trailing_bits(true);
    let raw = base64::engine::GeneralPurpose::new(&base64::alphabet::URL_SAFE, cfg).decode(s).ok()?;
    let out = miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, MAX_LINK_TEXT + 1).ok()?;
    if out.len() > MAX_LINK_TEXT {
        return None;
    }
    // Go keeps the bytes as they are; text that is not UTF-8 never checks
    String::from_utf8(out).ok()
}

/// Go `btw.Pack`: base64url(flate BestCompression), Go's own bytes.
fn pack(text: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(goflate::deflate_best(text.as_bytes()))
}

// ---------- the work limiter (work.go) ----------

struct WorkLimiter {
    n: i64,
    hits: HashMap<String, (Instant, i64)>,
}

impl WorkLimiter {
    fn allow(&mut self, addr: &str) -> (bool, u64) {
        if self.n < 0 {
            return (true, 0);
        }
        let now = Instant::now();
        let min = Duration::from_secs(60);
        if self.hits.len() > 100_000 {
            self.hits.retain(|_, w| now.duration_since(w.0) < min);
            if self.hits.len() > 100_000 {
                self.hits.clear();
            }
        }
        let w = self.hits.entry(addr.to_string()).or_insert((now, 0));
        if now.duration_since(w.0) >= min {
            *w = (now, 0);
        }
        if w.1 >= self.n {
            let left = (w.0 + min).saturating_duration_since(now).as_secs_f64();
            return (false, ((left as u64) + 1).max(1));
        }
        w.1 += 1;
        (true, 0)
    }
}

// ---------- the service ----------

type Flight = Arc<tokio::sync::OnceCell<Result<Bytes, CardErr>>>;

pub struct Cards {
    app: AppState,
    origin: String,
    site: String,
    cache_dir: PathBuf,
    cache_max: u64,
    render_sem: Arc<tokio::sync::Semaphore>,
    check_sem: Arc<tokio::sync::Semaphore>,
    checks: Mutex<HashMap<String, Arc<Report>>>,
    spans: Mutex<HashMap<String, Arc<Vec<Span>>>>,
    flights: Mutex<HashMap<String, Flight>>,
    work: Mutex<WorkLimiter>,
    books: Mutex<Books>,
    generic: [OnceLock<Bytes>; 2],
    share_acct: Mutex<crate::service::Limiter>,
    store_total: Mutex<Option<u64>>,
}

#[derive(Default)]
struct Books {
    indexed: bool,
    files: HashMap<String, (u64, SystemTime)>,
    total: u64,
}

fn site_of(origin: &str) -> String {
    // url.Parse(origin).Host, else "BeatScript"
    let rest = match origin.split_once("://") {
        Some((_, r)) => r,
        None => return "btw".into(),
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if host.is_empty() {
        "btw".into()
    } else {
        host.to_string()
    }
}

fn text_key(t: &str) -> String {
    hex::encode(Sha256::digest(t.as_bytes()))
}

impl Cards {
    fn new(app: &AppState) -> Cards {
        let origin = app.cfg.public_url.trim_end_matches('/').to_string();
        let cache_dir = app.cfg.data_dir().join("cards");
        let _ = std::fs::create_dir_all(&cache_dir);
        Cards {
            site: site_of(&origin),
            origin,
            cache_dir,
            cache_max: 256 << 20,
            render_sem: Arc::new(tokio::sync::Semaphore::new(2)),
            check_sem: Arc::new(tokio::sync::Semaphore::new(2)),
            checks: Mutex::new(HashMap::new()),
            spans: Mutex::new(HashMap::new()),
            flights: Mutex::new(HashMap::new()),
            work: Mutex::new(WorkLimiter { n: 120, hits: HashMap::new() }),
            books: Mutex::new(Books::default()),
            generic: [OnceLock::new(), OnceLock::new()],
            share_acct: Mutex::new(crate::service::Limiter::new(app.cfg.flag_num("share-hour", 60).max(0) as usize, Duration::from_secs(3600))),
            store_total: Mutex::new(None),
            app: app.clone(),
        }
    }

    /// The 308 to the BeatScript 2 twin's address, for an address (`/p/<cid>[.jsonld|.beatscript]`, `/c/<cid>`,
    /// `/card/<cid>[.square].png`) that names a BeatScript 1 program by its CID alone whose twin is here.
    fn renamed(&self, method: &axum::http::Method, path: &str, query: Option<&str>, cid: &str, packed: &str) -> Option<Response<Body>> {
        if !packed.is_empty() || !valid_cid(cid) {
            return None;
        }
        let to = self.app.store.renamed_to(cid)?;
        Some(beatscript_serve::lineage::routes::renamed_redirect(method, path, query, cid, &to))
    }

    fn allow_work(&self, addr: Option<&str>) -> Result<(), CardErr> {
        let Some(addr) = addr else { return Ok(()) };
        let (ok, retry) = self.work.lock().unwrap().allow(addr);
        if ok {
            return Ok(());
        }
        Err(CardErr { status: 429, code: "rate_limited", message: format!("too many link previews from your address; try again in {retry}s"), retry })
    }

    /// The checker on text, memoised (256), charged as work, on one of two checker slots.
    async fn check(&self, addr: Option<&str>, text: &str) -> Result<Arc<Report>, CardErr> {
        let k = text_key(text);
        if let Some(r) = self.checks.lock().unwrap().get(&k) {
            return Ok(r.clone());
        }
        self.allow_work(addr)?;
        let _slot = tokio::time::timeout(CHECK_TIMEOUT, self.check_sem.clone().acquire_owned()).await.map_err(|_| errf(503, "busy", "the checker is busy; try again in a moment"))?.unwrap();
        let t = text.to_string();
        let v = tokio::task::spawn_blocking(move || crate::store::check(&t)).await.map_err(|_| errf(502, "checker_error", "the checker failed; try again"))?;
        let r = Arc::new(Report::from_json(&v));
        let mut m = self.checks.lock().unwrap();
        if m.len() >= 256 {
            m.clear();
        }
        m.insert(k, r.clone());
        Ok(r)
    }

    async fn highlight(&self, text: &str) -> Arc<Vec<Span>> {
        let k = text_key(text);
        if let Some(s) = self.spans.lock().unwrap().get(&k) {
            return s.clone();
        }
        let got = match tokio::time::timeout(CHECK_TIMEOUT, self.check_sem.clone().acquire_owned()).await {
            Ok(Ok(_slot)) => {
                let t = text.to_string();
                tokio::task::spawn_blocking(move || beatscript_api::highlight::spans(&t).into_iter().map(|(s, c)| Span { start: s.start, end: s.end, class: c.to_string() }).collect::<Vec<_>>()).await.unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let sp = if got.is_empty() || got.iter().all(|s| s.class == "comment") { lex::basic_spans(text) } else { got };
        let sp = Arc::new(sp);
        let mut m = self.spans.lock().unwrap();
        if m.len() >= 64 {
            m.clear();
        }
        m.insert(k, sp.clone());
        sp
    }

    async fn program(&self, addr: Option<&str>, text: &str, meta: Meta) -> Result<Prog, CardErr> {
        if text.len() > MAX_TEXT {
            return Err(errf(413, "too_large", format!("program text is over {MAX_TEXT} bytes: no card")));
        }
        let r = self.check(addr, text).await?;
        let mut p = Prog::bare(&r.cid, St::Ok);
        p.canonical = r.canonical.clone();
        p.report = Some(r);
        p.meta = meta;
        p.desc = lex::first_comment(text);
        Ok(p)
    }

    /// `known`: an example (its original text) or a stored program, with its meta.
    fn known(&self, cid: &str) -> Option<(String, Meta, Option<crate::site::Example>)> {
        let ex = &self.app.examples;
        for e in ex.list.iter().filter(|e| e.cid == cid) {
            if let Some(t) = ex.text.get(&e.name) {
                let h = lex::header_meta(t);
                // (its `# from:` lines as a save would keep them: the page's isBasedOn agrees with lineage, which counts them)
                let mut m = Meta { name: h.name, license: h.license, visibility: h.visibility, based_on: djlive::refs::from_lines(t), ..Meta::default() };
                if m.name.is_empty() {
                    m.name = e.name.clone();
                }
                if m.license.is_empty() {
                    m.license = e.license.clone();
                }
                return Some((t.clone(), m, Some(e.clone())));
            }
        }
        let (text, meta) = stored(&self.app.cfg.programs, cid)?;
        Some((text, meta, None))
    }

    /// `resolve`: a request (a CID, and the link's text when it carries one) as a program or the reason for the
    /// generic card. Err only for the service failing (busy, rate limited).
    async fn resolve(&self, addr: Option<&str>, cid: &str, packed: &str) -> Result<Prog, CardErr> {
        if !valid_cid(cid) {
            return Ok(Prog::bare(cid, St::Invalid));
        }
        let known = self.known(cid);
        let bad = |e: &CardErr| e.status == 413 || e.status == 422;
        let mut p = if !packed.is_empty() {
            if packed.len() > MAX_TEXT {
                return Ok(Prog::bare(cid, St::Invalid));
            }
            let Some((lcid, ltext)) = parse_link(&format!("{cid}.{packed}")) else { return Ok(Prog::bare(cid, St::Invalid)) };
            if ltext.is_empty() || lcid != cid || ltext.len() > MAX_TEXT {
                return Ok(Prog::bare(cid, St::Invalid));
            }
            let meta = known.as_ref().map(|k| k.1.clone()).unwrap_or_default();
            match self.program(addr, &ltext, meta).await {
                Ok(p) => p,
                Err(e) if bad(&e) => return Ok(Prog::bare(cid, St::Invalid)),
                Err(e) => return Err(e),
            }
        } else if let Some((t, m, _)) = &known {
            match self.program(addr, t, m.clone()).await {
                Ok(p) => p,
                Err(e) if bad(&e) => return Ok(Prog::bare(cid, St::Invalid)),
                Err(e) => return Err(e),
            }
        } else {
            return Ok(Prog::bare(cid, St::Unknown));
        };
        // the page's "open" link keeps the link's own packed text when it is the program the CID names: the canonical text
        // (an older link) or the formatted one, whose `#!` headers and comments then open with it
        if !packed.is_empty() && !p.canonical.is_empty() {
            if let Some((_, t)) = parse_link(&format!("{cid}.{packed}")) {
                if t == p.canonical || p.report.as_ref().is_some_and(|r| r.ok && r.cid == cid) {
                    p.packed = packed.to_string();
                }
            }
        }
        p.known = known.is_some();
        if let Some((t, m, ex)) = known {
            p.meta = m;
            if let Some(e) = ex {
                p.ex_name = e.name;
                p.ex_desc = e.description;
            }
            let d = lex::first_comment(&t);
            if !d.is_empty() {
                p.desc = d;
            }
        }
        let r = p.report.clone().unwrap();
        if !r.ok {
            p.st = St::Invalid;
        } else if r.cid != cid {
            p.st = St::Mismatch;
        }
        Ok(p)
    }

    fn cache_name(&self, p: &Prog, v: Variant) -> String {
        let mut n = format!("{}.{}.v{VERSION}", p.cid, v.name());
        if self.site != "btw" {
            n += &format!(".s{}", hex::encode(&Sha256::digest(self.site.as_bytes())[..3]));
        }
        let h = name_hash(p) + &license_hash(p);
        if !h.is_empty() {
            n += &format!(".{h}");
        }
        n + ".png"
    }

    fn etag(p: &Prog, v: Variant) -> String {
        let mut n = format!("{}-{}", p.cid, v.name());
        let h = name_hash(p) + &license_hash(p);
        if !h.is_empty() {
            n += &format!("-{h}");
        }
        format!("\"{n}\"")
    }

    fn cache_get(&self, key: &str) -> Option<Bytes> {
        let b = std::fs::read(self.cache_dir.join(key)).ok().filter(|b| !b.is_empty())?;
        if let Some(f) = self.books.lock().unwrap().files.get_mut(key) {
            f.1 = SystemTime::now();
        }
        Some(Bytes::from(b))
    }

    fn index_cache(&self, b: &mut Books) {
        if b.indexed {
            return;
        }
        b.indexed = true;
        let Ok(rd) = std::fs::read_dir(&self.cache_dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(".card-") {
                let _ = std::fs::remove_file(e.path());
                continue;
            }
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    b.files.insert(name, (md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
                    b.total += md.len();
                }
            }
        }
    }

    fn cache_put(&self, key: &str, data: &[u8]) {
        let mut b = self.books.lock().unwrap();
        self.index_cache(&mut b);
        let tmp = self.cache_dir.join(format!(".card-{}-{:x}.tmp", std::process::id(), rand_u64()));
        let ok = (|| -> std::io::Result<()> {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            f.write_all(data)?;
            std::fs::rename(&tmp, self.cache_dir.join(key))
        })();
        if ok.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        if let Some(old) = b.files.insert(key.to_string(), (data.len() as u64, SystemTime::now())) {
            b.total -= old.0;
        }
        b.total += data.len() as u64;
        if b.total > self.cache_max {
            let mut names: Vec<(SystemTime, String)> = b.files.iter().filter(|(n, _)| n.as_str() != key).map(|(n, f)| (f.1, n.clone())).collect();
            names.sort();
            for (_, n) in names {
                if b.total <= self.cache_max {
                    break;
                }
                let _ = std::fs::remove_file(self.cache_dir.join(&n));
                if let Some(f) = b.files.remove(&n) {
                    b.total -= f.0;
                }
            }
        }
    }

    /// The card of p: the cache, else one render per key at a time (followers wait for it), charged as work.
    async fn draw(&self, addr: Option<&str>, p: &Prog, v: Variant) -> Result<Bytes, CardErr> {
        let key = self.cache_name(p, v);
        if let Some(b) = self.cache_get(&key) {
            return Ok(b);
        }
        let (cell, leader) = {
            let mut f = self.flights.lock().unwrap();
            match f.get(&key) {
                Some(c) => (c.clone(), false),
                None => {
                    let c: Flight = Arc::new(tokio::sync::OnceCell::new());
                    f.insert(key.clone(), c.clone());
                    (c, true)
                }
            }
        };
        if !leader {
            return cell.get_or_init(|| async { Err(errf(503, "busy", "card unavailable, try again shortly")) }).await.clone();
        }
        let res = match self.allow_work(addr) {
            Err(e) => Err(e),
            Ok(()) => self.render_png(p, v).await,
        };
        if let Ok(b) = &res {
            self.cache_put(&key, b);
        }
        let _ = cell.set(res.clone());
        self.flights.lock().unwrap().remove(&key);
        res
    }

    async fn render_png(&self, p: &Prog, v: Variant) -> Result<Bytes, CardErr> {
        let _slot = self.render_sem.clone().acquire_owned().await.unwrap();
        let spans = self.highlight(&p.canonical).await;
        let rep = p.report.clone().unwrap();
        let seconds = match (rep.seconds, rep.tempo) {
            (Some(s), _) => s,
            (None, Some(t)) if t > 0.0 && rep.song_bars > 0 => rep.song_bars as f64 * 4.0 * 60.0 / t,
            _ => 0.0,
        };
        let c = Content {
            seed: p.cid.clone(),
            cid: p.cid.clone(),
            name: p.label(),
            license: p.meta.license.clone(),
            canonical: p.canonical.clone(),
            spans: (*spans).clone(),
            tempo: rep.tempo.unwrap_or(0.0),
            key: rep.key.clone(),
            bars: rep.song_bars,
            seconds,
            sections: rep.sections.clone(),
            site: self.site.clone(),
            ..Content::default()
        };
        let png = tokio::task::spawn_blocking(move || draw::draw_card(&c, v)).await.map_err(|_| errf(500, "internal", "card unavailable"))?;
        Ok(Bytes::from(png))
    }

    fn generic_png(&self, v: Variant) -> Bytes {
        let i = usize::from(v == Variant::Square);
        self.generic[i].get_or_init(|| Bytes::from(draw::draw_card(&generic_content(&self.site), v))).clone()
    }
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

/// A stored program (`programStore.Get`): text and meta, only when the file hashes to its name.
fn stored(dir: &std::path::Path, cid: &str) -> Option<(String, Meta)> {
    if !crate::store::is_cid(cid) {
        return None;
    }
    let b = std::fs::read(dir.join(format!("{cid}.json"))).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let text = v["text"].as_str()?;
    if v["cid"].as_str() != Some(cid) || text.is_empty() || djlive::compile::program_cid(text) != cid {
        return None;
    }
    Some((text.to_string(), Meta::from_json(&v["meta"])))
}

/// `genericContent`: the fixed sample on the generic card (coloured by the built-in lexer).
const GENERIC_PROGRAM: &str = "tempo 128\nkey am\nlet kick = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nlet bass = a1! ~ a1 ~ ~ a1 ~ ~ c2 ~ a1 ~ ~ e2 ~ ~\nsection drop 8 bars {\n  deck A: kick kick | bass bass inst=reese\n  deck B: harmony chord(am) | melody <a4 c5 e5 g5> inst=pwm-lead\n  master: filter lp 300 -> 2800 over 8 bars\n}\narrange drop\n";

fn generic_content(site: &str) -> Content {
    let lines = GENERIC_PROGRAM.trim_end_matches('\n').split('\n').enumerate().map(|(i, l)| region::RLine { idx: i as i64, text: l.to_string(), gap_top: false }).collect();
    let sec = |n: &str, s: i64, b: i64| Section { name: n.into(), start: s, bars: b };
    Content {
        seed: "btw-generic".into(),
        name: "write a whole track as text".into(),
        license: "CC0-1.0".into(),
        canonical: GENERIC_PROGRAM.into(),
        spans: lex::basic_spans(GENERIC_PROGRAM),
        lines: Some(lines),
        section: "drop".into(),
        generic: true,
        site: site.into(),
        sections: vec![sec("intro", 0, 4), sec("build", 4, 4), sec("drop", 8, 8), sec("outro", 16, 4)],
        ..Content::default()
    }
}

// ---------- JSON-LD (ld.Document) ----------

/// Every program's document, BeatScript 1 or 2, names the BeatScript context (`/context/beatscript-v1.jsonld`, which has
/// the rights terms too), its types in that vocabulary, its IRI `https://beatscript.app/p/<cid>`, and its parent by CID
/// (`forkedFrom`, as the program block says it) and by IRI (`isBasedOn`). BeatScript data lives on beatscript.app only
/// (2026-11-22): until then a BeatScript 1 program's document named the first context, the djrust: types and an IRI
/// under beats.bitwrap.io. The CID is the text's, so no CID moved with it; `cid` says which text the document is of.
fn ld_document(origin: &str, p: &Prog) -> Value {
    let r = p.report.clone().unwrap();
    // keys in sorted order: encoding/json writes a map sorted
    let mut d: std::collections::BTreeMap<&str, Value> = std::collections::BTreeMap::new();
    d.insert("@context", json!(format!("{origin}/context/beatscript-v1.jsonld")));
    d.insert("@id", json!(program_iri(&p.cid)));
    d.insert("@type", json!("Program"));
    d.insert("cid", json!(p.cid));
    d.insert("name", json!(p.label()));
    if !r.key.is_empty() {
        d.insert("key", json!(r.key));
    }
    if let Some(t) = r.tempo {
        d.insert("tempo", go_num(t));
    }
    if !r.sections.is_empty() {
        // a section the shared text documented (a `#` comment line above it) has that doc as its schema.org description
        d.insert(
            "sections",
            Value::Array(
                r.sections
                    .iter()
                    .map(|s| {
                        let mut o = json!({"@type": "Section", "bars": s.bars, "name": s.name, "start": s.start});
                        if let Some(doc) = p.meta.section_doc(&s.name) {
                            o["description"] = json!(doc);
                        }
                        o
                    })
                    .collect(),
            ),
        );
    }
    if !p.meta.license.is_empty() {
        d.insert("licenseId", json!(p.meta.license));
        // the licence as an IRI: the CC deeds, and rightsstatements.org's "In Copyright" for all rights reserved
        if let Some(u) = djlive::progld::license_iri(&p.meta.license) {
            d.insert("license", json!(u));
        }
    }
    if !p.meta.author.is_empty() {
        d.insert("author", json!(p.meta.author));
    }
    // the rights statement: the licence above, and only what the store keeps besides it (its credits, the program it
    // was forked from), never a guess at who wrote it; then the page that says plainly what that means
    let credits: Vec<&String> = p.meta.credits.iter().filter(|c| !c.trim().is_empty()).collect();
    if !credits.is_empty() {
        d.insert("credit", json!(credits));
    }
    if !p.meta.forked_from.is_empty() {
        d.insert("forkedFrom", json!(p.meta.forked_from));
        d.insert("isBasedOn", json!(program_iri(&p.meta.forked_from)));
    }
    d.insert("usageInfo", json!(beatscript_serve::rights::url(origin)));
    Value::Object(d.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A program's IRI in its JSON-LD document (`@id`, and `isBasedOn` of its forks): its page, the same name the program
/// block gives it (djlive progld).
fn program_iri(cid: &str) -> String {
    format!("{}{cid}", djlive::progld::IRI_PREFIX)
}

/// The program as a `<beatscript-view data-editor data-ui="bar">` block (djlive `progld`): its canonical text, whose CID
/// is the block's `@id`, and the rights and provenance its meta keeps. The editor loads it, the phone and the MCP read
/// it. None when the text does not compile to its CID (then there is nothing to vouch for).
#[allow(clippy::too_many_arguments)]
pub(crate) fn program_view_of(cid: &str, canonical: &str, name: &str, license: &str, author: &str, forked_from: &str, sig: Option<&str>, docs: &[djlive::doc::DocComment], based_on: &[String]) -> Option<String> {
    let c = djlive::compile::check(canonical).compiled?;
    if c.cid != cid {
        return None;
    }
    let some = |s: &str| Some(s.to_string()).filter(|s| !s.is_empty());
    let (name, license, author, forked) = (some(name), some(license), some(author), some(forked_from));
    // the key's scale and the instruments linked to the bitwrap ontology (the host's snapshot of it)
    let onto: &dyn djlive::linked::Ontology = djhost::ontology::vocabulary();
    let f = djlive::progld::Facts { name: name.as_deref(), license: license.as_deref(), author: author.as_deref(), forked_from: forked.as_deref(), signature: sig, ontology: Some(onto), docs, based_on, ..Default::default() };
    Some(djlive::progld::view_html(&djlive::progld::document(&c, &f), true, "bar"))
}

fn program_view(p: &Prog, sig: Option<&str>) -> Option<String> {
    program_view_of(&p.cid, &p.canonical, &p.label(), &p.meta.license, &p.meta.author, &p.meta.forked_from, sig, &p.meta.doc_comments(), &p.meta.based_on)
}

/// The `<beatscript-view>` block of a stored public program, for the editor's own page (`/?cid=`). None: not stored,
/// not public, or not a program.
pub(crate) fn stored_view(dir: &std::path::Path, cid: &str) -> Option<String> {
    let (text, meta) = stored(dir, cid)?;
    if !meta.visibility.is_empty() && meta.visibility != "public" {
        return None;
    }
    program_view_of(cid, &text, &meta.name, &meta.license, &meta.author, &meta.forked_from, None, &meta.doc_comments(), &meta.based_on)
}

/// What `Accept` asks `/p/<cid>` for first.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Want {
    Html,
    Text,
    Ld,
    Audio,
}

/// The media type `Accept` names first (by q, then by order; `*/*` and anything unknown is the page).
fn negotiate(h: &HeaderMap) -> Want {
    let a = h.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let mut best: Option<(f32, usize, Want)> = None;
    for (i, part) in a.split(',').enumerate() {
        let mut it = part.split(';');
        let ty = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it.filter_map(|p| p.trim().strip_prefix("q=")).find_map(|q| q.trim().parse::<f32>().ok()).unwrap_or(1.0);
        let w = match ty.as_str() {
            "text/html" | "application/xhtml+xml" => Want::Html,
            "text/plain" | "text/beatscript" | "text/x-beatscript" | "text/btw" => Want::Text,
            "application/ld+json" | "application/json" => Want::Ld,
            t if t.starts_with("audio/") => Want::Audio,
            _ => continue,
        };
        if q <= 0.0 {
            continue;
        }
        if best.is_none_or(|(bq, bi, _)| q > bq || (q == bq && i < bi)) {
            best = Some((q, i, w));
        }
    }
    best.map_or(Want::Html, |b| b.2)
}

/// The program's formats as `<link rel="alternate">` tags (the page's head).
fn alternates_html(origin: &str, cid: &str, license: &str) -> String {
    let u = beatscript_serve::opendata::urls(origin, cid);
    let mut s = format!(
        "<link rel=\"alternate\" type=\"text/beatscript\" href=\"{}\" title=\"BeatScript program (.beatscript)\">\n<link rel=\"alternate\" type=\"application/ld+json\" href=\"{}\">\n<link rel=\"alternate\" type=\"audio/ogg\" href=\"{}\">\n",
        esc_url(&u.beatscript),
        esc_url(&u.jsonld),
        esc_url(&u.render)
    );
    // the licence the program has (an empty one is the language's default, CC0)
    let id = if license.trim().is_empty() { "CC0-1.0" } else { license.trim() };
    if let Some(l) = djlive::progld::license_iri(id) {
        s.push_str(&format!("<link rel=\"license\" href=\"{}\">\n", esc_url(l)));
    }
    s
}

/// The same as a `Link` header value.
fn alternates_link(origin: &str, cid: &str) -> String {
    let u = beatscript_serve::opendata::urls(origin, cid);
    format!("<{}>; rel=\"alternate\"; type=\"text/beatscript\", <{}>; rel=\"alternate\"; type=\"application/ld+json\", <{}>; rel=\"alternate\"; type=\"audio/ogg\"", u.beatscript, u.jsonld, u.render)
}

/// json.MarshalIndent(d, "", "  ").
fn ld_indent(d: &Value) -> String {
    go_escape(serde_json::to_string_pretty(d).unwrap())
}

// ---------- the page (html/template, escaped as it escapes) ----------

/// html/template's escaper for text and quoted attributes.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\0' => o.push('\u{FFFD}'),
            '"' => o.push_str("&#34;"),
            '&' => o.push_str("&amp;"),
            '\'' => o.push_str("&#39;"),
            '+' => o.push_str("&#43;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            c => o.push(c),
        }
    }
    o
}

/// A URL attribute: urlFilter, urlNormalizer, then the attribute escaper.
fn esc_url(s: &str) -> String {
    if let Some((proto, _)) = s.split_once(':') {
        if !proto.contains('/') && !["http", "https", "mailto"].iter().any(|p| proto.eq_ignore_ascii_case(p)) {
            return "#ZgotmplZ".into();
        }
    }
    let b = s.as_bytes();
    let mut o = String::with_capacity(s.len());
    for (i, &c) in b.iter().enumerate() {
        let keep = c.is_ascii_alphanumeric()
            || b"!#$&*+,/:;=?@[]-._~".contains(&c)
            || (c == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit());
        if keep {
            o.push(c as char);
        } else {
            o.push_str(&format!("%{c:02x}"));
        }
    }
    esc(&o)
}

struct PageData {
    title: String,
    desc: String,
    url: String,
    image: String,
    alt: String,
    open: String,
    og_type: &'static str,
    no_index: bool,
    ld: Option<String>,
    /// The program as a `<beatscript-view>` block in the body (djlive `progld`): additive, the head is as it was.
    view: Option<String>,
    /// `<link rel="alternate">` tags for the program's other formats (`.beatscript`, `.jsonld`, the render): additive.
    alternates: Option<String>,
    /// A playable card (`/c/` of an embeddable program, cards/embed.rs): twitter:card=player and og:audio. Additive.
    player: Option<embed::Player>,
    width: u32,
    height: u32,
}

fn page_html(d: &PageData) -> String {
    let (t, ds, u, im, alt, open) = (esc(&d.title), esc(&d.desc), esc_url(&d.url), esc(&d.image), esc(&d.alt), esc_url(&d.open));
    let (u_meta, im_url) = (esc(&d.url), esc_url(&d.image));
    let mut s = String::with_capacity(4096);
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n");
    s.push_str(&format!("<title>{t}</title>\n<meta name=\"description\" content=\"{ds}\">\n"));
    if d.no_index {
        s.push_str("<meta name=\"robots\" content=\"noindex\">\n");
    }
    s.push_str(&format!("<link rel=\"canonical\" href=\"{u}\">\n<meta property=\"og:type\" content=\"{}\">\n<meta property=\"og:site_name\" content=\"BeatScript\">\n", d.og_type));
    s.push_str(&format!("<meta property=\"og:title\" content=\"{t}\">\n<meta property=\"og:description\" content=\"{ds}\">\n<meta property=\"og:url\" content=\"{u_meta}\">\n"));
    s.push_str(&format!("<meta property=\"og:image\" content=\"{im}\">\n<meta property=\"og:image:secure_url\" content=\"{im}\">\n<meta property=\"og:image:type\" content=\"image/png\">\n"));
    s.push_str(&format!("<meta property=\"og:image:width\" content=\"{}\">\n<meta property=\"og:image:height\" content=\"{}\">\n<meta property=\"og:image:alt\" content=\"{alt}\">\n", d.width, d.height));
    let card = if d.player.is_some() { "player" } else { "summary_large_image" };
    s.push_str(&format!("<meta name=\"twitter:card\" content=\"{card}\">\n<meta name=\"twitter:title\" content=\"{t}\">\n<meta name=\"twitter:description\" content=\"{ds}\">\n"));
    s.push_str(&format!("<meta name=\"twitter:image\" content=\"{im}\">\n<meta name=\"twitter:image:alt\" content=\"{alt}\">\n"));
    if let Some(p) = &d.player {
        s.push_str(&embed::player_meta(p));
    }
    if let Some(a) = &d.alternates {
        s.push_str(a);
    }
    if let Some(ld) = &d.ld {
        s.push_str(&format!("<script type=\"application/ld+json\">{ld}</script>\n"));
    }
    s.push_str("<link rel=\"stylesheet\" href=\"/card/card.css\">\n</head>\n<body>\n<main>\n");
    s.push_str(&format!("<a href=\"{open}\"><img src=\"{im_url}\" width=\"{}\" height=\"{}\" alt=\"{alt}\"></a>\n<h1>{t}</h1>\n<p>{ds}</p>\n", d.width, d.height));
    s.push_str(&format!("<p><a id=\"open\" class=\"open\" href=\"{open}\">Open in BeatScript &rarr;</a></p>\n<noscript><p>JavaScript is off: <a href=\"{open}\">open this in BeatScript</a>.</p></noscript>\n{}</main>\n{}<script src=\"/card/go.js\" defer></script>\n</body>\n</html>\n", d.view.as_ref().map_or(String::new(), |v| format!("{v}\n")), beatscript_serve::rights::FOOTER));
    s
}

fn program_page(d: &mut PageData, p: &Prog, origin: &str, req_packed: &str) {
    let r = p.report.clone().unwrap();
    let name = p.label();
    let mut parts = vec![name.clone()];
    if !r.key.is_empty() {
        parts.push(lex::clean_text(&r.key, 24));
    }
    if let Some(t) = r.tempo {
        parts.push(format!("{} BPM", lex::fmt_bpm(t)));
    }
    if r.song_bars > 0 {
        parts.push(format!("{} bars", r.song_bars));
    }
    d.title = parts.join(" · ");
    d.desc = if !p.desc.is_empty() {
        p.desc.clone()
    } else if !lex::clean_text(&p.ex_desc, 200).is_empty() {
        lex::clean_text(&p.ex_desc, 200)
    } else {
        format!("a track in {} lines of BeatScript", p.canonical.trim_end_matches('\n').matches('\n').count() + 1)
    };
    let mut img = format!("{origin}/card/{}", p.cid);
    if !p.known && !req_packed.is_empty() {
        img += &format!(".{req_packed}");
    }
    d.image = img + ".png";
    d.alt = lex::clean_text(&format!("Share card for {name}: code from the program over its section timeline; {}", parts[1..].join(", ")), 240);
    let pk = if p.packed.is_empty() { pack(&p.canonical) } else { p.packed.clone() };
    d.url = format!("{origin}/#{}.{pk}", p.cid);
    d.open = format!("/#{}.{pk}", p.cid);
}

// ---------- handlers ----------

fn addr_of(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

fn unavailable(e: &CardErr) -> Response<Body> {
    let mut h = HeaderMap::new();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let retry = if e.retry > 0 { e.retry } else { 30 };
    h.insert(header::RETRY_AFTER, HeaderValue::from(retry));
    if e.status != 0 {
        return http_error(h, e.status, &e.message);
    }
    http_error(h, 503, "card unavailable, try again shortly")
}

/// The static site's answer (Go's catch-all), for a wrong method on a card route.
async fn site_fallback(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let path = req.uri().path().to_string();
    if let Some(r) = crate::mux_redirect(req.method(), &path, req.uri().query()) {
        return r;
    }
    c.app.site.serve(req.method(), &path, req.headers())
}

/// `/c` and `/card`: ServeMux's redirect to the subtree.
async fn to_subtree(req: Request) -> Response<Body> {
    let target = format!("{}/", req.uri().path());
    let loc = match req.uri().query() {
        Some(q) => format!("{target}?{q}"),
        None => target,
    };
    let r = Response::builder().status(307).header(header::CONTENT_TYPE, "text/html; charset=utf-8").header(header::LOCATION, &loc);
    let body = if *req.method() == Method::HEAD { content::nothing() } else { Body::from(format!("<a href=\"{}\">Temporary Redirect</a>.\n\n", crate::site::html_escape(&loc))) };
    r.body(body).unwrap()
}

async fn serve_card(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let raw = req.uri().path().to_string();
    if let Some(r) = crate::mux_redirect(req.method(), &raw, req.uri().query()) {
        return r;
    }
    let path = crate::site::percent_decode(&raw);
    let name = path.strip_prefix("/card/").unwrap_or("");
    if name == "card.css" {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
        return written(req.method(), h, CARD_CSS.as_bytes().to_vec());
    }
    if name == "go.js" {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/javascript; charset=utf-8"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
        return written(req.method(), h, GO_JS.as_bytes().to_vec());
    }
    if let Some(name) = name.strip_suffix(".svg") {
        let (creq, addr) = (content::Req { method: req.method(), headers: req.headers() }, addr_of(&req));
        return svg::serve(&c, &creq, &addr, name).await;
    }
    let Some(name) = name.strip_suffix(".png") else { return not_found(HeaderMap::new()) };
    let (v, name) = match name.strip_suffix(".square") {
        Some(n) => (Variant::Square, n),
        None => (Variant::Wide, name),
    };
    let creq = content::Req { method: req.method(), headers: req.headers() };
    if name == "default" {
        return generic_png(&c, &creq, v, "default", "public, max-age=86400", false);
    }
    let (cid, packed) = name.split_once('.').unwrap_or((name, ""));
    if let Some(r) = c.renamed(req.method(), &path, req.uri().query(), cid, packed) {
        return r;
    }
    let addr = addr_of(&req);
    let p = match c.resolve(Some(&addr), cid, packed).await {
        Ok(p) => p,
        Err(e) => return unavailable(&e),
    };
    let st = p.state();
    if st != St::Ok {
        return generic_png(&c, &creq, v, state_name(st), CACHE_SHORT, no_index(st));
    }
    let data = match c.draw(Some(&addr), &p, v).await {
        Ok(d) => d,
        Err(e) => return unavailable(&e),
    };
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_IMMUTABLE));
    h.insert(header::ETAG, HeaderValue::from_str(&Cards::etag(&p, v)).unwrap());
    h.insert("x-btw-card", HeaderValue::from_static("program"));
    content::serve(h, &creq, data.len() as u64, Some(data))
}

fn generic_png(c: &Cards, req: &content::Req, v: Variant, reason: &str, cache: &'static str, noidx: bool) -> Response<Body> {
    let data = c.generic_png(v);
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::ETAG, HeaderValue::from_str(&format!("\"default-{}-v{VERSION}\"", v.name())).unwrap());
    h.insert("x-btw-card", HeaderValue::from_str(&format!("generic; reason={reason}")).unwrap());
    if noidx {
        h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    }
    content::serve(h, req, data.len() as u64, Some(data))
}

async fn serve_page(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let raw = req.uri().path().to_string();
    if let Some(r) = crate::mux_redirect(req.method(), &raw, req.uri().query()) {
        return r;
    }
    let path = crate::site::percent_decode(&raw);
    let short = path.starts_with("/p/");
    let name = path.strip_prefix("/c/").unwrap_or(&path);
    let name = name.strip_prefix("/p/").unwrap_or(name);
    let name = name.strip_suffix('/').unwrap_or(name);
    let (name, mut want_ld) = match name.strip_suffix(".jsonld") {
        Some(n) => (n, true),
        None => (name, false),
    };
    // `/p/<cid>.beatscript`: the canonical text itself (`/p/<cid>.btw`, its name before the rename, the same)
    let (name, mut want_beatscript) = match crate::content::strip_program_suffix(name) {
        Some(n) if short => (n, true),
        _ => (name, false),
    };
    // `/p/<cid>` with no suffix: the format the Accept header asks for first (the page for a browser)
    let negotiated = short && !want_ld && !want_beatscript && !name.contains('.');
    let mut want_audio = false;
    if negotiated {
        match negotiate(req.headers()) {
            Want::Text => want_beatscript = true,
            Want::Ld => want_ld = true,
            Want::Audio => want_audio = true,
            Want::Html => {}
        }
    }
    let (cid, packed) = name.split_once('.').unwrap_or((name, ""));
    if short && (!packed.is_empty() || !valid_cid(cid)) {
        return not_found(HeaderMap::new());
    }
    // the rename: a BeatScript 1 program whose BeatScript 2 twin is here is answered at the twin's address (a link that
    // carries its text, `<cid>.<packed>`, opens as it is)
    if let Some(r) = c.renamed(req.method(), &path, req.uri().query(), cid, packed) {
        return r;
    }
    let origin = c.origin.clone();
    let addr = addr_of(&req);
    let p = match c.resolve(Some(&addr), cid, packed).await {
        Ok(p) => p,
        Err(e) => return unavailable(&e),
    };
    let mut d = PageData {
        title: String::new(),
        desc: String::new(),
        url: format!("{origin}/"),
        image: format!("{origin}/card/default.png"),
        alt: GENERIC_ALT.into(),
        open: "/".into(),
        og_type: "website",
        no_index: false,
        ld: None,
        view: None,
        alternates: None,
        player: None,
        width: 1200,
        height: 630,
    };
    let mut cache = CACHE_PAGE;
    let mut doc: Option<Value> = None;
    let st = p.state();
    match st {
        St::Ok => {
            d.og_type = "music.song";
            program_page(&mut d, &p, &origin, packed);
            if short {
                d.url = format!("{origin}/p/{}", p.cid);
                d.open = format!("/#{}", p.cid);
            }
            doc = Some(ld_document(&origin, &p));
            d.alternates = Some(alternates_html(&origin, &p.cid, &p.meta.license));
            // a program that embeds says so (oEmbed discovery: cards/embed.rs)
            if embed::embeddable(&p) {
                let page = format!("{origin}/{}/{}", if short { "p" } else { "c" }, p.cid);
                d.alternates.get_or_insert_with(String::new).push_str(&embed::discovery(&origin, &p, &page));
                // the social page's card plays where the platform lets it (X, Discord, Slack, iMessage via og:audio)
                if !short {
                    d.player = Some(embed::Player::of(&origin, &p));
                }
            }
            // a signed share's link carries the author's signature after the packed text: `<cid>.<packed>.<sig>`
            let sig = packed.split('~').next().and_then(|s| s.split('.').nth(1)).filter(|s| !s.is_empty());
            d.view = program_view(&p, sig);
        }
        St::Private | St::Restricted => {
            d.title = PRIVATE_TITLE.into();
            d.desc = PRIVATE_DESC.into();
        }
        St::Unknown => {
            d.title = GENERIC_TITLE.into();
            d.desc = GENERIC_DESC.into();
            if valid_cid(cid) {
                d.url = format!("{origin}/c/{cid}");
                d.open = format!("/#{cid}");
                if short {
                    d.url = format!("{origin}/p/{cid}");
                }
            }
            cache = CACHE_SHORT;
        }
        _ => {
            d.title = GENERIC_TITLE.into();
            d.desc = GENERIC_DESC.into();
        }
    }
    let mut h = HeaderMap::new();
    if no_index(st) {
        d.no_index = true;
        h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    }
    if negotiated {
        h.insert(header::VARY, HeaderValue::from_static("Accept"));
    }
    if want_audio {
        if doc.is_none() {
            return not_found(h);
        }
        h.insert(header::LOCATION, HeaderValue::from_str(&beatscript_serve::opendata::urls(&origin, &p.cid).render).unwrap());
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
        let mut r = written(req.method(), h, Vec::new());
        *r.status_mut() = axum::http::StatusCode::SEE_OTHER;
        return r;
    }
    if want_beatscript {
        if doc.is_none() {
            return not_found(h);
        }
        // text/plain, so a browser shows it, unless Accept asks for text/beatscript (so the type varies by Accept)
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(crate::content::program_text_type(req.headers())));
        h.insert(header::VARY, HeaderValue::from_static("Accept"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
        h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
        h.insert("x-btw-cid", HeaderValue::from_str(&p.cid).unwrap());
        h.insert(header::LINK, HeaderValue::from_str(&alternates_link(&origin, &p.cid)).unwrap());
        return written(req.method(), h, p.canonical.clone().into_bytes());
    }
    if want_ld {
        let Some(doc) = doc else { return not_found(h) };
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/ld+json; charset=utf-8"));
        h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
        h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
        return written(req.method(), h, ld_indent(&doc).into_bytes());
    }
    d.ld = doc.as_ref().map(go_json);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert("x-btw-card", HeaderValue::from_static(state_name(st)));
    if doc.is_some() {
        h.insert(header::LINK, HeaderValue::from_str(&format!("<{origin}/c/{}.jsonld>; rel=\"describedby\"; type=\"application/ld+json\", {}", p.cid, alternates_link(&origin, &p.cid))).unwrap());
    }
    let mut html = page_html(&d);
    // a public program's page names its forks and links its family tree (beatscript_serve::lineage)
    if let (Some(_), Some(l)) = (&doc, beatscript_serve::lineage::installed()) {
        let cid = p.cid.clone();
        if let Ok(n) = tokio::task::spawn_blocking(move || l.fork_count(&cid)).await {
            html = beatscript_serve::lineage::page::with_fork_line(&html, &p.cid, n);
        }
    }
    // and its twin: a BeatScript 1 program (opened by a link that carries its text) links the same program in BeatScript 2,
    // a BeatScript 2 one names the legacy CID it was renamed from
    if doc.is_some() {
        if let Some(y) = c.app.store.renamed_to(&p.cid) {
            html = beatscript_serve::lineage::page::with_renamed_line(&html, &p.cid, &y, true);
        } else if let Some(x) = c.app.store.renamed_from(&p.cid) {
            html = beatscript_serve::lineage::page::with_renamed_line(&html, &p.cid, &x, false);
        }
    }
    written(req.method(), h, html.into_bytes())
}

// ---------- PUT /api/shares/<cid> (server/shares.go) ----------

fn clean_line_ok(s: &str, max: usize) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > max || s.chars().any(|r| (r as u32) < 0x20 || r == '\u{7f}') {
        return None;
    }
    Some(s.to_string())
}

struct ShareBody {
    text: String,
    license: String,
    visibility: String,
    forked_from: String,
    credits: Vec<String>,
    name: String,
}

/// json.Decoder with DisallowUnknownFields into shareBody: the first JSON value, field names matched without case.
fn decode_share(b: &[u8]) -> Option<ShareBody> {
    let v: Value = serde_json::Deserializer::from_slice(b).into_iter::<Value>().next()?.ok()?;
    let mut out = ShareBody { text: String::new(), license: String::new(), visibility: String::new(), forked_from: String::new(), credits: Vec::new(), name: String::new() };
    let obj = match v {
        Value::Null => return Some(out),
        Value::Object(o) => o,
        _ => return None,
    };
    let s = |v: &Value| -> Option<String> {
        match v {
            Value::Null => Some(String::new()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        }
    };
    for (k, v) in obj {
        match k.to_lowercase().as_str() {
            "text" => out.text = s(&v)?,
            "license" => out.license = s(&v)?,
            "visibility" => out.visibility = s(&v)?,
            "forked_from" => out.forked_from = s(&v)?,
            "name" => out.name = s(&v)?,
            "credits" => {
                out.credits = match v {
                    Value::Null => Vec::new(),
                    Value::Array(a) => a.iter().map(s).collect::<Option<Vec<_>>>()?,
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn license_ok(l: &str) -> bool {
    let b = l.as_bytes();
    !b.is_empty() && b.len() <= 40 && b[0].is_ascii_alphanumeric() && b[1..].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
}

fn rights(b: &ShareBody) -> Result<Meta, CardErr> {
    let mut m = Meta { license: "CC0-1.0".into(), visibility: "public".into(), source: "share".into(), ..Meta::default() };
    let v = b.visibility.trim();
    if !v.is_empty() && v != "public" {
        return Err(errf(422, "restricted", "only public programs are stored; a private or restricted program stays in its inline link"));
    }
    let l = b.license.trim();
    if !l.is_empty() {
        if !license_ok(l) {
            return Err(errf(400, "bad_request", "license is a short SPDX-style id such as CC0-1.0 or CC-BY-4.0"));
        }
        m.license = l.into();
    }
    let f = b.forked_from.trim();
    if !f.is_empty() {
        if !crate::store::is_cid(f) {
            return Err(errf(400, "bad_request", "forked_from must be a CID (it starts with z)"));
        }
        m.forked_from = f.into();
    }
    if b.credits.len() > 8 {
        return Err(errf(400, "bad_request", "at most 8 credits"));
    }
    for c in &b.credits {
        m.credits.push(clean_line_ok(c, 80).ok_or_else(|| errf(400, "bad_request", "each credit is one line of 1 to 80 characters"))?);
    }
    let n = b.name.trim();
    if !n.is_empty() {
        m.name = clean_line_ok(n, 80).ok_or_else(|| errf(400, "bad_request", "name is one line of at most 80 characters"))?;
    }
    Ok(m)
}

/// The check report as encoding/json writes `api.CheckReport` (field order, omitempty), for an error's details.
fn go_check_report(v: &Value) -> Value {
    let mut m = Map::new();
    let num = |x: &Value| if x.is_null() { Value::Null } else { x.as_f64().map_or(Value::Null, go_num) };
    m.insert("ok".into(), json!(v["ok"] == true));
    m.insert("errors".into(), json!(v["errors"].as_i64().unwrap_or(0)));
    m.insert("warnings".into(), json!(v["warnings"].as_i64().unwrap_or(0)));
    let diags = match v["diagnostics"].as_array() {
        None => Value::Null,
        Some(a) => Value::Array(
            a.iter()
                .map(|d| {
                    let mut o = Map::new();
                    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
                    o.insert("severity".into(), json!(s("severity")));
                    for k in ["line", "col", "end_col", "start", "end"] {
                        o.insert(k.into(), json!(d[k].as_i64().unwrap_or(0)));
                    }
                    o.insert("message".into(), json!(s("message")));
                    if let Some(e) = d["expected"].as_array().filter(|e| !e.is_empty()) {
                        o.insert("expected".into(), json!(e.iter().map(|x| x.as_str().unwrap_or("")).collect::<Vec<_>>()));
                    }
                    for k in ["found", "help", "render"] {
                        if !s(k).is_empty() {
                            o.insert(k.into(), json!(s(k)));
                        }
                    }
                    Value::Object(o)
                })
                .collect(),
        ),
    };
    m.insert("diagnostics".into(), diags);
    m.insert("cid".into(), json!(v["cid"].as_str().unwrap_or("")));
    m.insert("canonical".into(), json!(v["canonical"].as_str().unwrap_or("")));
    for k in ["nets", "song_nets"] {
        if let Some(o) = v[k].as_object().filter(|o| !o.is_empty()) {
            let sorted: std::collections::BTreeMap<_, _> = o.iter().map(|(k, v)| (k.clone(), json!(v.as_str().unwrap_or("")))).collect();
            m.insert(k.into(), Value::Object(sorted.into_iter().collect()));
        }
    }
    m.insert("tempo".into(), num(&v["tempo"]));
    if let Some(k) = v["key"].as_str().filter(|k| !k.is_empty()) {
        m.insert("key".into(), json!(k));
    }
    m.insert("swing".into(), num(&v["swing"]));
    m.insert("arranged".into(), json!(v["arranged"] == true));
    m.insert("song_bars".into(), json!(v["song_bars"].as_i64().unwrap_or(0)));
    m.insert("ends".into(), json!(v["ends"] == true));
    m.insert("loops".into(), json!(v["loops"] == true));
    if let Some(a) = v["sections"].as_array().filter(|a| !a.is_empty()) {
        m.insert("sections".into(), Value::Array(a.iter().map(|s| json!({"name": s["name"].as_str().unwrap_or(""), "start": s["start"].as_i64().unwrap_or(0), "bars": s["bars"].as_i64().unwrap_or(0)})).collect()));
    }
    for k in ["decks", "evolve"] {
        if let Some(a) = v[k].as_array().filter(|a| !a.is_empty()) {
            m.insert(k.into(), json!(a.iter().map(|x| x.as_str().unwrap_or("")).collect::<Vec<_>>()));
        }
    }
    if let Some(a) = v["lineage"].as_array().filter(|a| !a.is_empty()) {
        m.insert("lineage".into(), Value::Array(a.clone()));
    }
    // `arrange { }` mutations (djlive mutate.rs): the generation table, and on an error the fixer's text; absent otherwise
    for k in ["arrange", "fixed"] {
        if let Some(x) = v.get(k).filter(|x| !x.is_null()) {
            m.insert(k.into(), x.clone());
        }
    }
    m.insert("uses_assets".into(), json!(v["uses_assets"] == true));
    m.insert("restricted".into(), json!(v["restricted"] == true));
    m.insert("seconds".into(), num(&v["seconds"]));
    Value::Object(m)
}

fn rfc3339_now() -> String {
    let s = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `headerName`: `#! name: X` from the text as submitted.
fn header_name(text: &str) -> String {
    for l in text.split('\n') {
        if let Some(rest) = l.trim().strip_prefix("#!") {
            if let Some(v) = rest.trim().strip_prefix("name:") {
                let v = v.trim();
                let v = if v.len() > 80 { String::from_utf8_lossy(&v.as_bytes()[..80]).into_owned() } else { v.to_string() };
                return v.chars().filter(|r| (*r as u32) >= 0x20 && *r != '\u{7f}').collect();
            }
        }
    }
    String::new()
}

enum PutErr {
    Api(CardErr, Option<Value>),
}

impl Cards {
    fn store_total(&self, dir: &std::path::Path) -> u64 {
        let mut t = self.store_total.lock().unwrap();
        *t.get_or_insert_with(|| std::fs::read_dir(dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0))
    }

    /// `programStore.put` with an expected CID: (stored meta, check report, created).
    async fn store_put(&self, text: &str, mut meta: Meta, expect: &str) -> Result<(Meta, Value, bool, String), PutErr> {
        let max = self.app.cfg.max_program_bytes;
        let api = |s, c, m: String| Err(PutErr::Api(errf(s, c, m), None));
        if text.len() > max {
            return api(413, "too_large", format!("program is {} bytes; the limit is {max}", text.len()));
        }
        if text.trim().is_empty() {
            return api(400, "bad_request", "program text is empty".into());
        }
        if crate::store::declares_not_public(text) {
            return api(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site".into());
        }
        let t = text.to_string();
        let rep = tokio::task::spawn_blocking(move || crate::store::check(&t)).await.unwrap_or(Value::Null);
        if rep["ok"] != true {
            let first = rep["diagnostics"]
                .as_array()
                .and_then(|a| a.iter().find(|d| d["severity"] == "error"))
                .map_or("see the diagnostics".to_string(), |d| format!("{}:{} {}", d["line"].as_i64().unwrap_or(0), d["col"].as_i64().unwrap_or(0), d["message"].as_str().unwrap_or("")));
            let msg = format!("the program has {} error(s): {first}", rep["errors"].as_i64().unwrap_or(0));
            return Err(PutErr::Api(errf(422, "bad_request", msg), Some(json!({"check": go_check_report(&rep)}))));
        }
        if rep["restricted"] == true || rep["uses_assets"] == true {
            return api(422, "restricted", "programs that use restricted sounds or sample/loop assets are not stored on this site".into());
        }
        let cid = rep["cid"].as_str().unwrap_or("").to_string();
        let canonical = rep["canonical"].as_str().unwrap_or("").to_string();
        if cid.is_empty() || canonical.is_empty() || !crate::store::is_cid(&cid) || djlive::compile::program_cid(&canonical) != cid {
            return api(502, "checker_error", "the checker failed; try again".into());
        }
        if !expect.is_empty() && cid != expect {
            return api(422, "cid_mismatch", format!("that text is program {cid}, not {expect}"));
        }
        let dir = self.app.cfg.programs.clone();
        if let Some((_, m)) = stored(&dir, &cid) {
            return Ok((m, rep, false, cid));
        }
        if meta.created.is_empty() {
            meta.created = rfc3339_now();
        }
        if meta.name.is_empty() {
            meta.name = header_name(text);
        }
        // the doc comments of the text as shared: the canonical text the store keeps has none
        meta.docs = djlive::doc::doc_comments(text).into_iter().map(|d| (d.kind.to_string(), d.name, d.doc)).collect();
        // the storage gate: only Creative Commons, public, sample-free programs are stored. It reads the text as shared, so
        // what it admits carries the text's `# from:` lines, which the write keeps as meta.based_on
        let adm = match beatscript_serve::gate::admit(text, &meta.license, &meta.visibility) {
            Ok(a) if a.cid() == cid => a,
            Ok(_) => return api(502, "checker_error", "the checker failed; try again".into()),
            Err(r) => return api(r.status, r.code, r.message),
        };
        meta.based_on = adm.based_on().to_vec();
        meta.license = adm.license().to_string();
        meta.visibility = "public".into();
        let body = go_json(&json!({"cid": cid, "text": canonical, "meta": meta.to_json()}));
        let cap = (self.app.cfg.flag_num("store-max-mb", 256).max(0) as u64) << 20;
        if self.store_total(&dir) + body.len() as u64 > cap {
            return Err(PutErr::Api(CardErr { status: 507, code: "store_full", message: "the programs store is full".into(), retry: 3600 }, None));
        }
        let res = beatscript_serve::gate::write_program(&dir, &adm, meta.to_json(), go_json).map(|o| o.is_some());
        match res {
            Ok(true) => {
                if let Some(t) = self.store_total.lock().unwrap().as_mut() {
                    *t += body.len() as u64;
                }
                Ok((meta, rep, true, cid))
            }
            Ok(false) => match stored(&dir, &cid) {
                Some((_, m)) => Ok((m, rep, false, cid)),
                None => api(500, "internal", "the stored program does not read back".into()),
            },
            Err(e) => api(500, "internal", e.to_string()),
        }
    }
}

async fn put_share(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let raw = req.uri().path().to_string();
    if let Some(r) = crate::mux_redirect(req.method(), &raw, req.uri().query()) {
        return r;
    }
    let cid = crate::site::percent_decode(raw.strip_prefix("/api/shares/").unwrap_or(""));
    if !crate::store::is_cid(&cid) {
        return write_error(400, "bad_request", &format!("{} is not a CID", crate::site::go_quote(&cid)), 0, None);
    }
    let ip = addr_of(&req);
    let max = c.app.cfg.max_program_bytes * 6 + 8192;
    let (parts, body) = req.into_parts();
    let h = &parts.headers;
    let authz = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
    let too_large = || write_error(413, "too_large", &format!("request body is over {max} bytes"), 0, None);
    let auth = c.app.authsvc.get().expect("authsvc");
    let (owner, body) = if authz.starts_with("BtwDevice ") {
        // a paired phone: the body is read first (its hash is signed), then the signature checked
        let Ok(b) = axum::body::to_bytes(body, max).await else { return too_large() };
        match auth.verify.caller(None, Some(authz), parts.method.as_str(), parts.uri.path(), &b) {
            Ok(Some(who)) => (who.uid, b),
            Ok(None) => return write_error(401, "device_auth", "this route is for a paired phone (Authorization: BtwDevice …)", 0, None),
            Err(e) => return write_error(e.status, e.code, &e.message, 0, None),
        }
    } else {
        match auth.caller(h, parts.method.as_str(), parts.uri.path(), b"") {
            Ok(Some(who)) => {
                let Ok(b) = axum::body::to_bytes(body, max).await else { return too_large() };
                (who.uid, b)
            }
            Ok(None) => {
                let help = format!("sign in at {} (wallet or browser key), press \"MCP token\", and send it as `Authorization: Bearer btw_…`", c.app.cfg.public_url);
                return write_error(401, "auth_required", &format!("authentication required: {help}"), 0, None);
            }
            Err(e) => return write_error(e.status, e.code, &e.message, 0, None),
        }
    };
    let (ok, retry) = c.share_acct.lock().unwrap().allow(&owner);
    if !ok {
        return write_error(429, "rate_limited", &format!("this account has shared too many programs this hour; try again in {retry}s"), retry, None);
    }
    let (ok, retry) = c.app.put_ip.lock().unwrap().allow(&ip);
    if !ok {
        return write_error(429, "rate_limited", &format!("too many upload requests from your address; try again in {retry}s"), retry, None);
    }
    let sb = match decode_share(&body) {
        Some(b) if !b.text.trim().is_empty() => b,
        _ => return write_error(400, "bad_request", r#"send JSON like {"text": "<the program>", "license": "CC0-1.0", "forked_from": "<CID>", "credits": ["…"]}"#, 0, None),
    };
    let meta = match rights(&sb) {
        Ok(m) => m,
        Err(e) => return write_error(e.status, e.code, &e.message, 0, None),
    };
    match c.store_put(&sb.text, meta, &cid).await {
        Err(PutErr::Api(e, details)) => write_error(e.status, e.code, &e.message, e.retry, details),
        Ok((meta, rep, created, cid)) => {
            let site = c.app.cfg.public_url.trim_end_matches('/');
            let mut m = Map::new();
            m.insert("cid".into(), json!(cid));
            m.insert("created".into(), json!(created));
            m.insert("fetch_url".into(), json!(format!("{}/api/programs/{cid}", c.app.cfg.public_url)));
            m.insert("meta".into(), meta.to_json());
            m.insert("short_social_url".into(), json!(format!("{site}/c/{cid}")));
            m.insert("short_url".into(), json!(format!("{site}/p/{cid}")));
            m.insert("warnings".into(), json!(rep["warnings"].as_i64().unwrap_or(0)));
            Response::builder()
                .status(if created { 201 } else { 200 })
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .body(go_body(go_json(&Value::Object(m)) + "\n"))
                .unwrap()
        }
    }
}

/// Only these methods reach the handler; the rest get the static site's answer, as from Go's catch-all. (`any`, not a
/// method router: axum's would add an `Allow` header Go does not send.)
macro_rules! only {
    ($h:ident, $($m:ident),+) => {
        any(|st: State<Arc<Cards>>, req: Request| async move {
            if $(*req.method() == Method::$m)||+ { $h(st, req).await } else { site_fallback(st, req).await }
        })
    };
}

mod embed; // after `only!`, which it uses
pub(crate) use embed::PREFIX as EMBED_PREFIX;

async fn subtree(_: State<Arc<Cards>>, req: Request) -> Response<Body> {
    to_subtree(req).await
}

pub fn routes(app: &AppState) -> Router {
    let c = Arc::new(Cards::new(app));
    Router::new()
        .route("/c", only!(subtree, GET, HEAD))
        .route("/card", only!(subtree, GET, HEAD))
        .route("/c/", only!(serve_page, GET, HEAD))
        .route("/c/{*rest}", only!(serve_page, GET, HEAD))
        .route("/p/{cid}", only!(serve_page, GET, HEAD))
        .route("/card/", only!(serve_card, GET, HEAD))
        .route("/card/{*rest}", only!(serve_card, GET, HEAD))
        .route("/api/shares/", any(site_fallback))
        .route("/api/shares/{cid}", only!(put_share, PUT))
        .merge(embed::routes())
        .with_state(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_round_trip() {
        let t = "tempo 120\ndeck A: kick x ~ x ~\n";
        let p = pack(t);
        let (cid, text) = parse_link(&format!("zabc.{p}")).unwrap();
        assert_eq!((cid.as_str(), text.as_str()), ("zabc", t));
        assert!(parse_link("zabc.!!!").is_none());
        assert!(parse_link("0abc").is_none());
    }

    #[test]
    fn escaping() {
        assert_eq!(esc(r#"a<b>&"'+"#), "a&lt;b&gt;&amp;&#34;&#39;&#43;");
        assert_eq!(esc_url("javascript:alert(1)"), "#ZgotmplZ");
        assert_eq!(esc_url("/#z1.a b"), "/#z1.a%20b");
        assert_eq!(site_of("https://beatscript.app"), "beatscript.app");
        assert_eq!(site_of(""), "btw");
    }

    #[test]
    fn work_limit() {
        let mut w = WorkLimiter { n: 2, hits: HashMap::new() };
        assert!(w.allow("a").0 && w.allow("a").0);
        let (ok, retry) = w.allow("a");
        assert!(!ok && (1..=61).contains(&retry));
        assert!(w.allow("b").0);
    }

    #[test]
    fn share_body() {
        assert!(decode_share(br#"{"text":"x","TEXT":"y"}"#).is_some());
        assert!(decode_share(br#"{"text":"x","nope":1}"#).is_none());
        assert!(decode_share(br#"{"text":"x"} trailing"#).is_some());
        assert!(decode_share(br#"[1]"#).is_none());
        assert!(decode_share(b"").is_none());
        let b = decode_share(br#"{"text":"x","credits":["a"," "]}"#).unwrap();
        assert!(rights(&b).is_err());
        let b = decode_share(br#"{"text":"x","license":"CC-BY-4.0","visibility":"public"}"#).unwrap();
        assert_eq!(rights(&b).unwrap().license, "CC-BY-4.0");
    }

    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut d = png::Decoder::new(std::io::Cursor::new(png));
        d.set_transformations(png::Transformations::EXPAND);
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        let ch = info.color_type.samples();
        let rgb = buf[..info.buffer_size()].chunks(ch).flat_map(|p| [p[0], p[1], p[2]]).collect();
        (info.width, info.height, rgb)
    }

    /// The perceptual parity rule of bench/contract.py (PNG_MEAN 0.15, PNG_BIG 0.0005), against Go's own generic card
    /// (btwsite/testdata/cards/samples/, drawn by the Go card tests under the site name "btw.bitwrap.io").
    #[test]
    fn generic_card_matches_go_perceptually() {
        for (v, go) in [(Variant::Wide, &include_bytes!("../testdata/cards/default.png")[..]), (Variant::Square, &include_bytes!("../testdata/cards/default.square.png")[..])] {
            let (gw, gh, g) = decode(go);
            let (rw, rh, r) = decode(&draw::draw_card(&generic_content("btw.bitwrap.io"), v));
            assert_eq!((gw, gh), (rw, rh));
            let n = (gw * gh) as f64;
            let mean = g.iter().zip(&r).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / (3.0 * n);
            let big = g.chunks(3).zip(r.chunks(3)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 64)).count() as f64 / n;
            assert!(mean <= 0.15 && big <= 0.0005, "{v:?}: mean {mean:.4}, big {big:.5}");
        }
    }

    #[test]
    fn generic_cards_draw() {
        for v in [Variant::Wide, Variant::Square] {
            let png = draw::draw_card(&generic_content("beatscript.app"), v);
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }

    fn store_program(dir: &std::path::Path, text: &str, meta: Value) -> String {
        let cid = djlive::compile::program_cid(text);
        std::fs::write(dir.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": text, "meta": meta})).unwrap()).unwrap();
        cid
    }

    #[test]
    fn a_stored_public_program_is_a_block_that_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let text = "tempo 128\nkey am\ndeck A: kick x ~ x ~\n";
        let cid = store_program(dir.path(), text, json!({"license": "CC0-1.0", "name": "night bus", "visibility": "public", "forked_from": "z4EBG9parent"}));
        let v = stored_view(dir.path(), &cid).expect("a block");
        assert!(v.starts_with("<beatscript-view data-editor data-ui=\"bar\"><script type=\"application/ld+json\">"), "{v}");
        let r = djlive::progld::read_view(&v, true).unwrap();
        assert_eq!((r.cid.as_str(), r.text.as_str()), (cid.as_str(), text));
        assert_eq!(r.doc.get("name").and_then(djengine::json::JsonValue::as_str), Some("night bus"));
        assert_eq!(r.doc.get("forkedFrom").and_then(djengine::json::JsonValue::as_str), Some("z4EBG9parent"));
        // private, unknown, or a file that is not its CID's: no block
        let private = store_program(dir.path(), "tempo 90\ndeck A: kick x\n", json!({"visibility": "private"}));
        assert!(stored_view(dir.path(), &private).is_none());
        assert!(stored_view(dir.path(), "z4EBG9jCtKY4ioQ3TEJCQQmTTJZjugJRihCNbBxNJrBaWr1adMi").is_none());
        let other = djlive::compile::program_cid("tempo 100\n");
        std::fs::write(dir.path().join(format!("{other}.json")), serde_json::to_vec(&json!({"cid": other, "text": text})).unwrap()).unwrap();
        assert!(stored_view(dir.path(), &other).is_none());
    }

    /// A stored program's JSON-LD (`/p/<cid>.jsonld`, `/c/<cid>.jsonld`) carries its rights statement: the licence, the
    /// credits and the parent the store keeps, and `usageInfo` naming /authorship. Nothing the store does not keep.
    #[test]
    fn a_stored_programs_json_ld_says_its_rights_and_links_authorship() {
        let text = "tempo 128\nkey am\ndeck A: kick x ~ x ~\n";
        let cid = djlive::compile::program_cid(text);
        let prog = |meta: Value| Prog { canonical: text.into(), report: Some(Arc::new(Report { ok: true, cid: cid.clone(), ..Report::default() })), meta: Meta::from_json(&meta), ..Prog::bare(&cid, St::Ok) };
        let d = ld_document("https://beatscript.app", &prog(json!({"license": "CC0-1.0", "visibility": "public", "name": "night bus", "forked_from": "z4EBG9parent", "credits": ["ann", " "]})));
        assert_eq!(d["usageInfo"], "https://beatscript.app/authorship");
        assert_eq!(d["@context"], "https://beatscript.app/context/beatscript-v1.jsonld", "the BeatScript context, which has the rights terms");
        assert_eq!((d["licenseId"].as_str(), d["license"].as_str()), (Some("CC0-1.0"), Some("https://creativecommons.org/publicdomain/zero/1.0/")));
        assert_eq!(d["credit"], json!(["ann"]));
        assert_eq!(d["forkedFrom"], "z4EBG9parent");
        assert_eq!(d["isBasedOn"], "https://beatscript.app/p/z4EBG9parent");
        assert!(d.get("author").is_none(), "no author is stored, so none is said");
        // a program with nothing but its licence: still the link, and no parent or credits invented
        let bare = ld_document("https://beatscript.app", &prog(json!({"license": "CC0-1.0", "visibility": "public"})));
        assert_eq!(bare["usageInfo"], "https://beatscript.app/authorship");
        for k in ["credit", "forkedFrom", "isBasedOn", "author"] {
            assert!(bare.get(k).is_none(), "{k} on a program that stores none");
        }
        // the published v1 context is untouched: the new terms are only in the local one
        let v1: Value = serde_json::from_str(include_str!("public/btw-context-v1.jsonld")).unwrap();
        assert!(v1["@context"].get("usageInfo").is_none() && v1["@context"].get("isBasedOn").is_none());
    }

    /// A program's JSON-LD, BeatScript 1 or 2, names the BeatScript context, its IRI under beatscript.app/p/, its types in the
    /// BeatScript vocabulary, and its parent by CID and by IRI: nothing in it names bitwrap.io. Only `cid` differs.
    #[test]
    fn a_beatscript_2_programs_json_ld_names_the_beatscript_context() {
        let doc = |text: &str, meta: Value| {
            let cid = djlive::compile::program_cid(text);
            let secs = vec![Section { name: "drop".into(), start: 0, bars: 4 }];
            let p = Prog { canonical: text.into(), report: Some(Arc::new(Report { ok: true, cid: cid.clone(), sections: secs, ..Report::default() })), meta: Meta::from_json(&meta), ..Prog::bare(&cid, St::Ok) };
            (cid, ld_document("https://beatscript.app", &p))
        };
        let meta = json!({"license": "CC0-1.0", "visibility": "public", "name": "night bus", "forked_from": "z4EBG9parent"});
        let (c1, v1) = doc("tempo 128\nkey am\ndeck A: kick x ~ x ~\n", meta.clone());
        let shape = |cid: &str| {
            json!({
                "@context": "https://beatscript.app/context/beatscript-v1.jsonld",
                "@id": format!("https://beatscript.app/p/{cid}"), "@type": "Program", "cid": cid, "forkedFrom": "z4EBG9parent", "isBasedOn": "https://beatscript.app/p/z4EBG9parent",
                "license": "https://creativecommons.org/publicdomain/zero/1.0/", "licenseId": "CC0-1.0", "name": "night bus",
                "sections": [{"@type": "Section", "bars": 4, "name": "drop", "start": 0}], "usageInfo": "https://beatscript.app/authorship",
            })
        };
        assert_eq!(v1, shape(&c1));
        assert!(!v1.to_string().contains("bitwrap.io"));
        let (c2, v2) = doc("#! beatscript 2\ntempo 128\nkey am\ndeck A: kick x ~ x ~\n", meta);
        assert_ne!(c1, c2);
        assert_eq!(v2, shape(&c2));
        // every term the BeatScript 2 document uses is the BeatScript context's (it needs no local context)
        let ctx: Value = serde_json::from_slice(crate::public::CONTEXT_BEATSCRIPT_V1).unwrap();
        for k in v2.as_object().unwrap().keys().chain(v2["sections"][0].as_object().unwrap().keys()).filter(|k| !k.starts_with('@')) {
            assert!(ctx["@context"].get(k).is_some(), "{k} is not in the BeatScript context");
        }
        for t in ["Program", "Section"] {
            assert_eq!(ctx["@context"][t], format!("bs:{t}"));
        }
        // the block says the same IRI
        let v = program_view_of(&c2, "#! beatscript 2\ntempo 128\nkey am\ndeck A: kick x ~ x ~\n", "night bus", "CC0-1.0", "", "", None, &[]).expect("a block");
        let r = djlive::progld::read_view(&v, true).unwrap();
        assert_eq!(r.cid, c2);
        assert_eq!(r.doc.get("@id").and_then(djengine::json::JsonValue::as_str), Some(v2["@id"].as_str().unwrap()));
    }

    /// Doc comments (project-datum 6662): the share keeps the doc comments of the text as sent beside its canonical text
    /// (`meta.docs`), and the program's JSON-LD gives each documented section its `description`; the CID is the text's.
    #[test]
    fn a_shared_programs_section_docs_are_its_json_ld_descriptions() {
        let written = "tempo 128\n# drop: the kick and the bass\nsection drop 4 bars {\n  deck A: kick x ~ x ~\n}\nsection out 4 bars {\n  deck A: kick x ~ ~ ~\n}\narrange drop out\n";
        let rep = crate::store::check(written);
        let canonical = rep["canonical"].as_str().unwrap().to_string();
        let cid = rep["cid"].as_str().unwrap().to_string();
        assert_eq!(cid, djlive::compile::program_cid(&canonical));
        assert_eq!(crate::store::check(&canonical)["cid"], rep["cid"], "the same CID without the comments");
        assert_eq!(rep["sections"][0]["doc"], "the kick and the bass", "check says it as `doc`");
        // what store_put keeps in the meta
        let mut meta = Meta { license: "CC0-1.0".into(), visibility: "public".into(), ..Meta::default() };
        meta.docs = djlive::doc::doc_comments(written).into_iter().map(|d| (d.kind.to_string(), d.name, d.doc)).collect();
        let mj = meta.to_json();
        assert_eq!(mj["docs"], json!([{"kind": "section", "name": "drop", "doc": "the kick and the bass"}]));
        let meta = Meta::from_json(&mj);
        let secs = vec![Section { name: "drop".into(), start: 0, bars: 4 }, Section { name: "out".into(), start: 4, bars: 4 }];
        let p = Prog { canonical: canonical.clone(), report: Some(Arc::new(Report { ok: true, cid: cid.clone(), sections: secs, ..Report::default() })), meta, ..Prog::bare(&cid, St::Ok) };
        let d = ld_document("https://beatscript.app", &p);
        assert_eq!(d["sections"][0]["description"], "the kick and the bass");
        assert!(d["sections"][1].get("description").is_none(), "an undocumented section has none");
        let ctx: Value = serde_json::from_slice(crate::public::CONTEXT_BEATSCRIPT_V1).unwrap();
        assert_eq!((d["@context"].as_str(), &ctx["@context"]["description"]), (Some("https://beatscript.app/context/beatscript-v1.jsonld"), &json!("schema:description")));
        // the block (the editor's, the MCP's) says it too, and still reads back as its CID
        let v = program_view(&p, None).expect("a block");
        let r = djlive::progld::read_view(&v, true).unwrap();
        assert_eq!(r.cid, cid);
        assert!(r.doc.js_stringified().contains(r#""section":[{"name":"drop","description":"the kick and the bass"}]"#), "{}", r.doc.js_stringified());
        // a meta with no docs writes no `docs` key: a program shared without comments is stored as before
        assert!(Meta::default().to_json().get("docs").is_none());
    }

    #[test]
    fn the_card_page_head_is_unchanged_by_the_block() {
        let mk = |view: Option<String>| PageData {
            title: "night bus · am · 128 BPM".into(),
            desc: "a track".into(),
            url: "https://beatscript.app/p/zX".into(),
            image: "https://beatscript.app/card/zX.png".into(),
            alt: "card".into(),
            open: "/#zX".into(),
            og_type: "music.song",
            no_index: false,
            ld: Some("{\"@id\":\"x\"}".into()),
            view,
            alternates: Some(alternates_html("https://beatscript.app", "zX", "")),
            player: None,
            width: 1200,
            height: 630,
        };
        let plain = page_html(&mk(None));
        let block = program_view_of(&djlive::compile::program_cid("tempo 128\n"), "tempo 128\n", "night bus", "", "", "", Some("0xsig"), &[], &[]).unwrap();
        let with = page_html(&mk(Some(block.clone())));
        let head = |s: &str| s[..s.find("</head>").unwrap()].to_string();
        assert_eq!(head(&with), head(&plain));
        assert_eq!(with.replace(&format!("{block}\n"), ""), plain);
        assert!(with.contains(&format!("{block}\n</main>")));
        let r = djlive::progld::read_view(&with, true).unwrap();
        assert_eq!(r.doc.get("signature").and_then(djengine::json::JsonValue::as_str), Some("0xsig"));
    }

    #[test]
    fn accept_picks_the_format() {
        let h = |a: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::ACCEPT, HeaderValue::from_str(a).unwrap());
            negotiate(&m)
        };
        assert_eq!(negotiate(&HeaderMap::new()), Want::Html);
        assert_eq!(h("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"), Want::Html);
        assert_eq!(h("text/plain"), Want::Text);
        assert_eq!(h("text/x-beatscript, text/html;q=0.5"), Want::Text);
        assert_eq!(h("application/ld+json"), Want::Ld);
        assert_eq!(h("text/html;q=0.4, application/ld+json;q=0.9"), Want::Ld);
        assert_eq!(h("audio/ogg"), Want::Audio);
        assert_eq!(h("audio/*;q=0, text/plain"), Want::Text);
        assert_eq!(h("*/*"), Want::Html);
        // BeatScript's own type, and the two older names of it
        for t in ["text/beatscript", "text/x-beatscript", "text/btw", "text/beatscript;q=0.9, text/html;q=0.5"] {
            assert_eq!(h(t), Want::Text, "{t}");
        }
    }

    #[test]
    fn alternates_name_every_format_and_the_programs_own_licence() {
        let a = alternates_html("https://beatscript.app", "zX", "CC-BY-4.0");
        assert!(a.contains(r#"<link rel="alternate" type="text/beatscript" href="https://beatscript.app/p/zX.beatscript""#), "{a}");
        assert!(alternates_link("https://beatscript.app", "zX").starts_with(r#"<https://beatscript.app/p/zX.beatscript>; rel="alternate"; type="text/beatscript", "#));
        assert!(a.contains(r#"<link rel="alternate" type="application/ld+json" href="https://beatscript.app/p/zX.jsonld">"#));
        assert!(a.contains(r#"<link rel="alternate" type="audio/ogg" href="https://beatscript.app/api/render/zX?format=opus">"#));
        assert!(a.contains(r#"<link rel="license" href="https://creativecommons.org/licenses/by/4.0/">"#));
        assert!(alternates_html("https://beatscript.app", "zX", "").contains("publicdomain/zero/1.0/"));
        assert!(alternates_html("https://beatscript.app", "zX", "all-rights-reserved").contains("rightsstatements.org/vocab/InC/1.0/"));
    }
}

/// A share page as [`page_html`] writes it, for the docs' CSP test.
#[cfg(test)]
pub(crate) fn test_page_html() -> String {
    page_html(&PageData {
        title: "t".into(),
        desc: "d".into(),
        url: "https://beatscript.app/p/z".into(),
        image: "https://beatscript.app/card/z.png".into(),
        alt: "a".into(),
        open: "https://beatscript.app/?cid=z".into(),
        og_type: "music.song",
        no_index: false,
        ld: Some("{}".into()),
        view: None,
        alternates: None,
        player: None,
        width: 1200,
        height: 630,
    })
}
