//! btwserve: the beatscript.app server in Rust, phase 1 (render, the static site, the docs). The plan is
//! `docs/RUST-SERVER.md` in this repo.
//!
//! Routes (the Go server's, same status codes, error JSON and headers):
//!   GET|HEAD /api/render/<cid>[.<packed>]?format=opus|wav&sr=44100|48000
//!   GET|HEAD /api/render/<cid>[.<packed>]?format=midi|stems|stems-opus   for a DAW (export_route.rs)
//!   GET      /api/render/<cid>/status[?sr=]
//!   GET      /healthz
//!   GET      /api/examples, /examples/<name>.btw
//!   GET      /llms.txt, /llms-full.txt, /docs/BTW.md, /docs/cheatsheet.md, /docs/api.md       (docs.rs)
//!   GET      /docs/, /docs/cheatsheet, /docs/language, /docs/api, /docs/assets/docs.{css,js}  (docs.rs, HTML)
//!   GET      everything else no Go route takes: the static site `-web`                         (site.rs)
//! A Go route (`proxy::go_owned`) is passed to `-fallback` when it is set (the strangler: one front for nginx), and
//! is a JSON 404 when it is not (nginx then sends only the moved locations here).
//!
//! Flags (Go's names; `-flag value`, `-flag=value` or `--flag`; each also `BTW_<NAME>` in the environment):
//!   -addr 127.0.0.1:18098 -data ./data -examples btwsite/examples -public-url https://beatscript.app
//!   -render-concurrency 2 -render-ip-hour 30 -render-max-bars 2400 -render-max-seconds 3600 -render-cache-mb 2048
//!   -put-ip-hour 60 -max-program-bytes 16384
//! and btwserve's own:
//!   -programs <data>/programs   the programs store (the Go server's `-data`/programs, shared)
//!   -programs-readonly          never write the store: a `<cid>.<packed>` program is held in memory instead
//!   -dump-dir <data>/dumps      the open-data archives `btw-dump` writes, served at /dumps/
//!   -render-dir <data>/render   the render cache (its own, or the Go server's: either serves the other's files)
//!   -render-workers <cpus>      engine threads computing at once (the CPU pool, earliest deadline first)
//!   -render-admit-lag-ms 0      refuse new renders (429 busy) while a running one is this far behind real time
//!   -log-dir <data>/log         access log (JSONL, redacted); `off` for none
//!   -session-secret-file <data>/session.secret   the IP-hash key's source, read only (BTW_SESSION_SECRET first)
//!   -web btwsite/web            the static site (Go's -web); empty: no static site
//!   -docs btwsite/web/docs      the generated BTW.md and cheatsheet.md (Go's -docs)
//!   -docs-src <dir>             the Go docs package (internal/btwsite/docs): api.md, assets/docs.css, assets/docs.js
//!   -fallback <url>             the Go server, for every route not served here (e.g. http://127.0.0.1:18097)
//!   -card-meta true             put the generic share card into / and /p (Go does once its card service is wired)

#[path = "../../btwd/src/render.rs"]
#[allow(dead_code)]
mod render;
#[path = "../../btwd/src/export.rs"]
#[allow(dead_code)]
mod export;
mod export_route; // ?format=midi|stems|stems-opus on the render route (docs/EXPORT.md)
#[path = "../../btwd/src/listen.rs"]
#[allow(dead_code)] // its CLI half is btwd's
mod listen;

mod content;
mod docs;
mod log;
mod proxy;
mod site;
mod opus;
mod sched;
mod service;
mod store;
// the phases of docs/RUST-SERVER.md §7, one module each; each has `routes(&AppState) -> Router` merged in main
mod beta; // the Android beta sign-up (off unless -android-beta-url)
mod public; // P2 + P3: info, site doc, assetlinks, instruments, context, programs, check, fmt, share, open, qr
mod authsvc; // P4: /api/auth/*
mod caps; // capabilities: caps.json, the signed grant (/.well-known/btw-capabilities.json)
mod oauth; // MCP authorization: OAuth 2.1 (/.well-known/oauth-*, /oauth/*)
mod cards; // P5: PUT /api/shares, /p/<cid>, /c/*, /card/*
mod hub; // P6: pairing, devices, sessions (SSE), the up-next queue
mod compose; // P7: POST /api/compose, /api/admin/*
mod mcp; // P7: /mcp
mod opensite; // open data: sitemap, feeds, dumps (btwserve::opendata)
mod ontology; // /ontology/: the BeatScript ontology (derived from bitwrap 0.2.0), its release, SPARQL and MCP
mod music_tools; // the MCP `lineage` and `genre_template` tools (btwserve::lineage, btwserve::genres)

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use axum::middleware::{self, Next};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};

use service::{Acquired, Fmt, Job, Key, Limiter, Limits, Service};
use store::Store;

// ---------- errors ----------

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: u64,
}

impl ApiError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> ApiError {
        ApiError { status, code: code.into(), message: message.into(), retry_after: 0 }
    }
    pub fn internal(m: String) -> ApiError {
        ApiError::new(500, "internal", m)
    }
    pub fn retry(mut self, s: u64) -> ApiError {
        self.retry_after = s;
        self
    }
    /// `api.WriteError`: `{"error":{...}}` + newline, json.Encoder's HTML escaping, Retry-After.
    fn response(&self) -> Response<Body> {
        let mut e = serde_json::Map::new();
        e.insert("code".into(), json!(self.code));
        e.insert("message".into(), json!(self.message));
        if self.retry_after > 0 {
            e.insert("retry_after".into(), json!(self.retry_after));
        }
        let body = go_json(&json!({"error": Value::Object(e)})) + "\n";
        let mut r = Response::builder().status(self.status).header(header::CONTENT_TYPE, "application/json; charset=utf-8");
        if self.retry_after > 0 {
            r = r.header(header::RETRY_AFTER, self.retry_after.to_string());
        }
        let mut r = r.body(go_body(body)).unwrap();
        if self.status == 413 {
            // net/http's reason phrase (hyper's is "Payload Too Large")
            r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Request Entity Too Large"));
        }
        r
    }
}

/// A body a Go handler writes without declaring its length: net/http sends `Content-Length` when the whole body fits
/// its 2048-byte buffer before the handler returns, else `Transfer-Encoding: chunked` (and on HEAD no length at all).
/// A body of unknown size makes hyper do the same.
pub fn go_body(b: impl Into<Bytes>) -> Body {
    let b: Bytes = b.into();
    if b.len() > 2048 {
        Body::from_stream(futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(b) }))
    } else {
        Body::from(b)
    }
}

/// A float as encoding/json writes a float64: a whole number without a fraction (`96`, where serde writes `96.0`).
pub fn go_num(x: f64) -> Value {
    if x.is_finite() && x == x.trunc() && x.abs() < 1e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// encoding/json's `Encoder` (HTML-safe, U+2028/2029, `\\b`/`\\f`, Go's float format): `public::go_encode`.
pub fn go_json(v: &Value) -> String {
    public::go_encode(v)
}

// ---------- config ----------

#[derive(Clone)]
struct Config {
    addr: String,
    examples: Option<PathBuf>,
    public_url: String,
    render_concurrency: usize,
    render_ip_hour: usize,
    render_max_bars: u32,
    render_max_seconds: u32,
    render_cache_mb: u64,
    put_ip_hour: usize,
    max_program_bytes: usize,
    programs: PathBuf,
    programs_readonly: bool,
    render_dir: PathBuf,
    /// Where `btw-dump` writes the open-data archives (served at /dumps/).
    dump_dir: PathBuf,
    render_workers: usize,
    admit_lag_ms: u64,
    log_dir: Option<PathBuf>,
    secret_file: PathBuf,
    version: String,
    web: Option<PathBuf>,
    docs: Option<PathBuf>,
    docs_src: Option<PathBuf>,
    fallback: Option<String>,
    card_meta: bool,
    data: PathBuf,
    alt_origins: String,
    legacy_hosts: String,
    playlist_limits: btwserve::playlists::model::Limits,
    /// every flag as given (`-name value`), for the modules that read their own: use `Config::flag`
    raw: HashMap<String, String>,
}

impl Config {
    /// A flag by Go's name, else `BTW_<NAME>` from the environment, else the default.
    pub fn flag(&self, k: &str, d: &str) -> String {
        self.raw.get(k).cloned().or_else(|| std::env::var(format!("BTW_{}", k.to_uppercase().replace('-', "_"))).ok()).unwrap_or_else(|| d.to_string())
    }
    pub fn flag_num(&self, k: &str, d: i64) -> i64 {
        self.flag(k, &d.to_string()).trim().parse().unwrap_or(d)
    }
    pub fn flag_f64(&self, k: &str, d: f64) -> f64 {
        self.flag(k, &d.to_string()).trim().parse().unwrap_or(d)
    }
    pub fn flag_bool(&self, k: &str, d: bool) -> bool {
        matches!(self.flag(k, if d { "true" } else { "false" }).trim(), "true" | "1" | "yes" | "TRUE" | "True")
    }
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data
    }
}

const BOOL_FLAGS: &[&str] = &["programs-readonly"];

fn parse_config() -> Result<Config, String> {
    config_from(std::env::args().skip(1).collect())
}

/// The configuration these flags (`-name value`, as on the command line) give, with `BTW_*` and the defaults.
fn config_from(args: Vec<String>) -> Result<Config, String> {
    let env = |k: &str| std::env::var(format!("BTW_{}", k.to_uppercase().replace('-', "_"))).ok();
    let mut flags: HashMap<String, String> = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].trim_start_matches('-').to_string();
        if a == args[i] {
            return Err(format!("unexpected argument {a}"));
        }
        let (k, v) = match a.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None if BOOL_FLAGS.contains(&a.as_str()) => (a, "true".to_string()),
            None => {
                i += 1;
                (a, args.get(i).cloned().ok_or("a flag needs a value")?)
            }
        };
        flags.insert(k, v);
        i += 1;
    }
    let get = |k: &str, d: &str| flags.get(k).cloned().or_else(|| env(k)).unwrap_or_else(|| d.to_string());
    let num = |k: &str, d: &str| get(k, d).parse::<u64>().map_err(|_| format!("-{k}: a whole number"));
    let ex = get("examples", "");
    let data: PathBuf = get("data", "./data").into();
    let cpus = std::thread::available_parallelism().map_or(2, |n| n.get());
    let log_dir = get("log-dir", &data.join("log").to_string_lossy());
    let truthy = |k: &str| matches!(get(k, "false").as_str(), "true" | "1" | "yes");
    Ok(Config {
        addr: get("addr", "127.0.0.1:18098"),
        programs: get("programs", &data.join("programs").to_string_lossy()).into(),
        programs_readonly: truthy("programs-readonly"),
        render_dir: get("render-dir", &data.join("render").to_string_lossy()).into(),
        dump_dir: get("dump-dir", &data.join("dumps").to_string_lossy()).into(),
        render_workers: num("render-workers", &cpus.to_string())?.max(1) as usize,
        admit_lag_ms: num("render-admit-lag-ms", "0")?,
        log_dir: if log_dir == "off" || log_dir.is_empty() { None } else { Some(log_dir.into()) },
        secret_file: get("session-secret-file", &data.join("session.secret").to_string_lossy()).into(),
        version: get("version", "dev"),
        web: Some(get("web", "btwsite/web")).filter(|s| !s.is_empty()).map(PathBuf::from),
        docs: Some(get("docs", "btwsite/web/docs")).filter(|s| !s.is_empty()).map(PathBuf::from),
        docs_src: Some(get("docs-src", "")).filter(|s| !s.is_empty()).map(PathBuf::from),
        fallback: Some(get("fallback", "")).filter(|s| !s.is_empty()),
        card_meta: !matches!(get("card-meta", "true").as_str(), "false" | "0" | "no"),
        examples: if ex.is_empty() { None } else { Some(ex.into()) },
        public_url: get("public-url", "https://beatscript.app"),
        render_concurrency: num("render-concurrency", "2")? as usize,
        render_ip_hour: num("render-ip-hour", "30")? as usize,
        // the 60-minute caps (2400 bars, an hour), as the Go server's are being raised to
        render_max_bars: num("render-max-bars", "2400")? as u32,
        render_max_seconds: num("render-max-seconds", "3600")? as u32,
        render_cache_mb: num("render-cache-mb", "2048")?,
        put_ip_hour: num("put-ip-hour", "60")? as usize,
        max_program_bytes: num("max-program-bytes", "16384")? as usize,
        data: data.clone(),
        alt_origins: get("alt-origins", ""),
        legacy_hosts: get("legacy-hosts", "btw.bitwrap.io"),
        playlist_limits: btwserve::playlists::model::Limits {
            max_playlists: num("playlists-max", "100")? as usize,
            max_items: num("playlists-max-items", "500")? as usize,
            max_account_bytes: (num("playlists-account-kb", "512")? as usize) << 10,
            max_total_bytes: num("playlists-total-mb", "256")? << 20,
            writes_per_min: num("playlists-writes-min", "60")? as usize,
            requests_per_min: num("playlists-requests-min", "300")? as usize,
            resolve_per_min: num("playlists-resolve-min", "120")? as usize,
        },
        raw: flags.clone(),
    })
}

// ---------- state ----------

struct App {
    cfg: Config,
    store: Store,
    rs: Arc<Service>,
    put_ip: Mutex<Limiter>,
    /// CheckReports by CID (a program's report never changes: the CID names its text).
    reports: Mutex<HashMap<String, Value>>,
    /// Checker work (open + check of a link's text, the first check of a stored program) runs on at most this many
    /// blocking threads at once. Go bounded it with btwd subprocess slots, a 10 s timeout and a per-address budget;
    /// a thread cannot be killed, so the plan's P1 hardening item adds the budget and a compile-time cap.
    check_slots: Arc<tokio::sync::Semaphore>,
    site: site::Static,
    docs: docs::Docs,
    examples: site::Examples,
    fallback: Option<proxy::Fallback>,
    /// What the phase modules share with each other, set once at start (main): the checker (P3), the auth service
    /// (P4), the hub of sessions, devices and the queue (P6).
    checker: std::sync::OnceLock<Arc<public::Checker>>,
    authsvc: std::sync::OnceLock<Arc<authsvc::AuthSvc>>,
    hub: std::sync::OnceLock<Arc<hub::Hub>>,
    /// The public CC0 programs (open data: sitemap, feeds).
    catalog: btwserve::opendata::Catalog,
    beta: std::sync::OnceLock<Arc<beta::Beta>>,
}

type AppState = Arc<App>;

/// The client's address, as `api.ClientIP`: behind a loopback peer (nginx) `X-Real-IP`, else the last
/// `X-Forwarded-For`; from anyone else the peer itself. IPv6 is bucketed by its /64 (`api.IPKey`).
#[derive(Clone, Copy)]
struct Peer(SocketAddr);

pub fn client_ip(peer: SocketAddr, h: &HeaderMap) -> String {
    let mut ip = peer.ip();
    if ip.is_loopback() {
        let real = h.get("x-real-ip").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<IpAddr>().ok());
        let fwd = h.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|s| s.rsplit(',').next()).and_then(|s| s.trim().parse::<IpAddr>().ok());
        if let Some(r) = real.or(fwd) {
            ip = r;
        }
    }
    match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        IpAddr::V6(v6) => v6.to_ipv4_mapped().unwrap().to_string(),
        v4 => v4.to_string(),
    }
}

/// The programs store as what an `import` reads: a whole CID, the text it names (btwapi::known re-hashes it).
struct StoreSnippets(Arc<App>);

impl djlive::env::Snippets for StoreSnippets {
    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        match self.0.store.get(query) {
            Some(p) => Ok((p.cid, p.text)),
            None => Err(djlive::env::not_here(query)),
        }
    }
}

impl App {
    fn report(&self, p: &store::Program) -> Value {
        if let Some(r) = self.reports.lock().unwrap().get(&p.cid) {
            return r.clone();
        }
        let r = store::check(&p.text);
        self.reports.lock().unwrap().insert(p.cid.clone(), r.clone());
        r
    }

    /// `Server.renderable`.
    fn renderable(&self, cid: &str) -> Result<(store::Program, Value), ApiError> {
        let Some(p) = self.store.get(cid) else {
            return Err(ApiError::new(404, "not_found", format!("no program {cid} here; PUT it to /api/programs/{cid} first")));
        };
        let rep = self.report(&p);
        if rep["ok"] != true {
            return Err(ApiError::new(422, "bad_request", "that program does not check"));
        }
        if rep["restricted"] == true || rep["uses_assets"] == true {
            return Err(ApiError::new(422, "restricted", "programs that use restricted sounds or sample/loop assets are not rendered on this site"));
        }
        let bars = rep["song_bars"].as_i64().unwrap_or(0);
        if bars > i64::from(self.cfg.render_max_bars) {
            return Err(ApiError::new(422, "too_large", format!("that program is {bars} bars; this site renders at most {}", self.cfg.render_max_bars)));
        }
        if let Some(s) = rep["seconds"].as_f64() {
            if s > f64::from(self.cfg.render_max_seconds) {
                return Err(ApiError::new(422, "too_large", format!("that program is {s:.0} s long; this site renders at most {} s", self.cfg.render_max_seconds)));
            }
        }
        Ok((p, rep))
    }

    /// `Server.fromLink`: the program carried in the URL, opened, checked against the CID and stored.
    fn from_link(&self, ip: &str, cid: &str, packed: &str) -> Result<(), ApiError> {
        if self.store.get(cid).is_some() {
            return Ok(());
        }
        if packed.is_empty() || packed.len() > 3 * self.cfg.max_program_bytes {
            return Err(ApiError::new(400, "bad_request", "after <cid>. the URL carries the packed program, as a share link does"));
        }
        let (ok, retry) = self.put_ip.lock().unwrap().allow(ip);
        if !ok {
            return Err(ApiError::new(429, "rate_limited", "too many new programs from this address; try again later").retry(retry));
        }
        let rep: Value = serde_json::from_str(&btwapi::open_json(&format!("{cid}.{packed}"))).unwrap_or(Value::Null);
        let canonical = rep["canonical"].as_str().unwrap_or("");
        let verified = rep["verified"] == true;
        let got = rep["cid"].as_str().unwrap_or("");
        if !canonical.is_empty() && (!verified || got != cid) {
            return Err(ApiError::new(422, "cid_mismatch", format!("the program in the URL is not {cid}")));
        }
        if rep["ok"] != true || canonical.is_empty() {
            let msg = rep["error"].as_str().filter(|s| !s.is_empty()).unwrap_or("the program in the URL does not open").to_string();
            return Err(ApiError::new(422, "bad_request", msg));
        }
        // programStore.put's refusals
        if canonical.len() > self.cfg.max_program_bytes {
            return Err(ApiError::new(413, "too_large", format!("program is {} bytes; the limit is {}", canonical.len(), self.cfg.max_program_bytes)));
        }
        if store::declares_not_public(canonical) {
            return Err(ApiError::new(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site"));
        }
        let chk = store::check(canonical);
        if chk["ok"] != true {
            return Err(ApiError::new(422, "bad_request", format!("the program has {} error(s)", chk["errors"])));
        }
        if chk["restricted"] == true || chk["uses_assets"] == true {
            return Err(ApiError::new(422, "restricted", "programs that use restricted sounds or sample/loop assets are not stored on this site"));
        }
        if chk["cid"].as_str() != Some(cid) || djlive::compile::program_cid(canonical) != cid {
            return Err(ApiError::new(422, "cid_mismatch", format!("that text is program {}, not {cid}", chk["cid"].as_str().unwrap_or("?"))));
        }
        self.store.put_canonical(cid, canonical).map_err(|e| ApiError::internal(e.to_string()))
    }
}

// ---------- the render route ----------

fn parse_params(raw: &str, q: &HashMap<String, String>) -> Result<Key, ApiError> {
    let cid = raw.split_once('.').map_or(raw, |(c, _)| c);
    if !store::is_cid(cid) {
        return Err(ApiError::new(404, "not_found", "no such program"));
    }
    let format = match q.get("format").map(String::as_str) {
        Some("wav") => Fmt::Wav,
        Some("opus") | Some("") | None => Fmt::Opus,
        Some(_) => return Err(ApiError::new(400, "bad_request", "format must be opus, wav, midi, stems or stems-opus")),
    };
    let sr = match q.get("sr").map(String::as_str) {
        None | Some("") => 44_100,
        Some(v) => match v.parse::<u32>() {
            Ok(n @ (44_100 | 48_000)) => n,
            _ => return Err(ApiError::new(400, "bad_request", "sr must be 44100 or 48000")),
        },
    };
    Ok(Key { cid: cid.to_string(), format, sr })
}

fn secs(f: f64) -> String {
    format!("{f:.2}")
}

async fn handle_render(State(app): State<AppState>, Path(raw): Path<String>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response<Body> {
    match render(app, raw, q, req).await {
        Ok(r) => r,
        Err(e) => e.response(),
    }
}

async fn render(app: AppState, raw: String, q: HashMap<String, String>, req: Request) -> Result<Response<Body>, ApiError> {
    if let Some(f) = q.get("format").filter(|f| export_route::FORMATS.contains(&f.as_str())) {
        return export_route::serve(app, raw, f.clone(), req).await;
    }
    let key = parse_params(&raw, &q)?;
    let peer = req.extensions().get::<Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    let ip = client_ip(peer, req.headers());
    let _slot = app.check_slots.clone().acquire_owned().await.map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some((_, packed)) = raw.split_once('.') {
        let (app2, ip2, cid, packed) = (app.clone(), ip.clone(), key.cid.clone(), packed.to_string());
        tokio::task::spawn_blocking(move || app2.from_link(&ip2, &cid, &packed)).await.map_err(|e| ApiError::internal(e.to_string()))??;
    }
    let (p, rep) = {
        let (app2, cid) = (app.clone(), key.cid.clone());
        tokio::task::spawn_blocking(move || app2.renderable(&cid)).await.map_err(|e| ApiError::internal(e.to_string()))??
    };
    drop(_slot);
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(key.content_type()));
    h.insert("x-btw-cid", HeaderValue::from_str(&key.cid).unwrap());
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("inline; filename=\"{}.{}\"", key.cid, key.format.name())).unwrap());
    let rep_seconds = rep["seconds"].as_f64();

    if req.method() == Method::HEAD {
        let (hit, job) = app.rs.peek(&key);
        if let Some((path, seconds)) = hit {
            return Ok(serve_complete(h, &req, &key, path, seconds.or(rep_seconds)));
        }
        h.insert("x-btw-cache", HeaderValue::from_static(if job.is_some() { "joined" } else { "miss" }));
        if let Some(s) = rep_seconds {
            h.insert("x-btw-seconds", HeaderValue::from_str(&secs(s)).unwrap());
        }
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        // a body of unknown length, so no Content-Length: 0 (Go sends none on this HEAD)
        let mut r = Response::new(Body::from_stream(futures_util::stream::empty::<Result<Bytes, std::io::Error>>()));
        *r.headers_mut() = h;
        return Ok(r);
    }

    // past this point Go has already set Content-Type, X-Btw-Cid and Content-Disposition when it answers an
    // error: WriteError replaces only the type
    let with_h = |e: ApiError, h: &HeaderMap| {
        let mut r = e.response();
        for (n, v) in h.iter() {
            if n != header::CONTENT_TYPE {
                r.headers_mut().insert(n.clone(), v.clone());
            }
        }
        r
    };
    let acq = match app.rs.acquire(&key, &ip, &p.text) {
        Ok(a) => a,
        Err(e) => return Ok(with_h(e, &h)),
    };
    match acq {
        Acquired::Hit(path, seconds) => Ok(serve_complete(h, &req, &key, path, seconds.or(rep_seconds))),
        Acquired::Job(job, joined) => {
            let guard = Leave { rs: app.rs.clone(), job: job.clone() };
            let mut ready = job.ready.clone();
            let start = loop {
                let now = ready.borrow_and_update().clone();
                if let Some(r) = now {
                    break r;
                }
                if ready.changed().await.is_err() {
                    break Err(ApiError::internal("render ended before it started".into()));
                }
            };
            let start = match start {
                Ok(s) => s,
                Err(e) => return Ok(with_h(e, &h)),
            };
            h.insert("x-btw-seconds", HeaderValue::from_str(&secs(start.seconds)).unwrap());
            h.insert("x-btw-cache", HeaderValue::from_static(if joined { "joined" } else { "miss" }));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            h.insert("x-accel-buffering", HeaderValue::from_static("no"));
            let mut r = Response::new(Body::from_stream(follow(job, guard)));
            *r.headers_mut() = h;
            Ok(r)
        }
    }
}

/// The last follower to leave stops the render (dropped with the response body).
struct Leave {
    rs: Arc<Service>,
    job: Arc<Job>,
}

impl Drop for Leave {
    fn drop(&mut self) {
        self.rs.leave(&self.job);
    }
}

/// The growing file, from byte 0, as it grows. A failed render, or a minute with nothing new, aborts the
/// connection (the client must see a broken stream, never a short one that looks whole).
fn follow(job: Arc<Job>, guard: Leave) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
    struct St {
        job: Arc<Job>,
        f: Option<std::fs::File>,
        pos: u64,
        rx: tokio::sync::watch::Receiver<u64>,
        _guard: Leave,
    }
    let rx = job.subscribe();
    let st = St { f: None, pos: 0, rx, job, _guard: guard };
    futures_util::stream::unfold(Some(st), |st| async move {
        let mut st = st?;
        loop {
            let (written, done, err) = st.job.snapshot();
            if st.pos < written {
                if st.f.is_none() {
                    match st.job.open() {
                        Ok(f) => st.f = Some(f),
                        Err(e) => return Some((Err(e), None)),
                    }
                }
                let n = (written - st.pos).min(256 << 10) as usize;
                let mut buf = vec![0u8; n];
                let r = st.f.as_ref().unwrap().read_at(&mut buf, st.pos);
                match r {
                    Ok(0) => {}
                    Ok(k) => {
                        buf.truncate(k);
                        st.pos += k as u64;
                        return Some((Ok(Bytes::from(buf)), Some(st)));
                    }
                    Err(e) => return Some((Err(e), None)),
                }
            }
            if done && st.pos >= written {
                return match err {
                    Some(m) => Some((Err(std::io::Error::other(m)), None)),
                    None => None,
                };
            }
            match tokio::time::timeout(Duration::from_secs(60), st.rx.changed()).await {
                Ok(_) => continue,
                Err(_) => return Some((Err(std::io::Error::other("render stalled")), None)),
            }
        }
    })
}

/// `serveComplete`: a finished render as a file, answered as Go's `http.ServeContent` answers it with no modtime:
/// If-Match (412), If-None-Match (304), If-Range, one range (206), several (206 multipart/byteranges), a bad or
/// unsatisfiable range (416), HEAD. The bytes go out of a memory map of the file: no copy into the process, and the
/// kernel sends straight from the page cache (an evicted file stays valid while it is mapped: it is unlinked, never
/// truncated).
fn serve_complete(mut h: HeaderMap, req: &Request, key: &Key, path: PathBuf, seconds: Option<f64>) -> Response<Body> {
    let Ok(f) = std::fs::File::open(&path) else {
        return ApiError::new(404, "not_found", "that render was just evicted; ask again").response();
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if let Some(s) = seconds.filter(|s| *s > 0.0) {
        h.insert("x-btw-seconds", HeaderValue::from_str(&secs(s)).unwrap());
    }
    h.insert("x-btw-cache", HeaderValue::from_static("hit"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    let etag = format!("\"{}-{}-{}-{}\"", key.cid, key.sr, key.format.version(), key.format.name());
    h.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    let head = req.method() == Method::HEAD;
    let map = if size > 0 && !head { unsafe { memmap2::Mmap::map(&f) }.ok().map(Bytes::from_owner) } else { None };
    content::serve(h, &content::Req { method: req.method(), headers: req.headers() }, size, map)
}

async fn handle_status(State(app): State<AppState>, Path(cid): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response<Body> {
    if !store::is_cid(&cid) {
        return ApiError::new(404, "not_found", "no such program").response();
    }
    let sr: u32 = q.get("sr").and_then(|v| v.parse().ok()).filter(|v| *v == 44_100 || *v == 48_000).unwrap_or(44_100);
    let app2 = app.clone();
    let cid2 = cid.clone();
    let found = tokio::task::spawn_blocking(move || app2.store.get(&cid2).map(|p| app2.report(&p))).await.ok().flatten();
    let Some(rep) = found else {
        return ApiError::new(404, "not_found", format!("no program {cid} here")).response();
    };
    let mut formats = serde_json::Map::new();
    for f in [Fmt::Opus, Fmt::Wav] {
        let key = Key { cid: cid.clone(), format: f, sr };
        let mut st = serde_json::Map::new();
        st.insert("state".into(), json!("none"));
        let (hit, job) = app.rs.peek(&key);
        if let Some((path, seconds)) = hit {
            st.insert("state".into(), json!("ready"));
            st.insert("bytes".into(), json!(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)));
            if let Some(s) = seconds.filter(|s| *s != 0.0) {
                st.insert("seconds".into(), go_num(s));
            }
        } else if let Some(j) = job {
            st.insert("state".into(), json!("rendering"));
            let (written, _, _) = j.snapshot();
            if written > 0 {
                st.insert("bytes".into(), json!(written));
            }
            if let Some(s) = j.start() {
                if j.progress() != 0.0 {
                    st.insert("seconds_rendered".into(), go_num(j.progress()));
                }
                if s.seconds != 0.0 {
                    st.insert("seconds".into(), go_num(s.seconds));
                }
            }
        }
        st.insert("url".into(), json!(format!("/api/render/{cid}?format={}&sr={sr}&v={}", f.name(), f.version())));
        st.insert("available".into(), json!(true));
        formats.insert(f.name().into(), Value::Object(st));
    }
    // encoding/json writes map keys sorted, struct fields in order; this is the map Go builds
    let mut out = serde_json::Map::new();
    // CheckReport.SongBars is an int in Go: 0 when the program has no song length (btwd says null)
    out.insert("bars".into(), json!(rep["song_bars"].as_i64().unwrap_or(0)));
    out.insert("cid".into(), json!(cid));
    out.insert("formats".into(), Value::Object(formats));
    out.insert("max_bars".into(), json!(app.cfg.render_max_bars));
    out.insert("max_seconds".into(), json!(app.cfg.render_max_seconds));
    out.insert("restricted".into(), json!(rep["restricted"] == true || rep["uses_assets"] == true));
    if let Some(s) = rep["seconds"].as_f64() {
        out.insert("seconds".into(), go_num(s));
    }
    Response::builder()
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(go_body(go_json(&Value::Object(out)) + "\n"))
        .unwrap()
}

// ---------- the headers every response carries (securityHeaders, licenseHeaders, corsReadOnly) ----------

const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; style-src 'self'; img-src 'self' data:; media-src 'self'; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

async fn site_headers(State(app): State<AppState>, req: Request, next: Next) -> Response<Body> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let has_origin = req.headers().contains_key(header::ORIGIN);
    let cors = has_origin && public_read(&path);
    if cors && method == Method::OPTIONS {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NO_CONTENT;
        let h = r.headers_mut();
        h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
        h.insert("access-control-allow-methods", HeaderValue::from_static("GET, HEAD, OPTIONS"));
        h.insert("access-control-allow-headers", HeaderValue::from_static("Range, If-None-Match"));
        h.insert("access-control-max-age", HeaderValue::from_static("86400"));
        add_site(h, &app, &path);
        return sorted(r);
    }
    let mut r = next.run(req).await;
    if r.extensions().get::<proxy::Proxied>().is_some() {
        return r;
    }
    let h = r.headers_mut();
    if cors && (method == Method::GET || method == Method::HEAD) {
        h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
        h.insert("access-control-expose-headers", HeaderValue::from_static("X-Btw-Cid, X-Btw-Seconds, X-Btw-Cache, Content-Length, Content-Range, Accept-Ranges, ETag, Retry-After"));
    }
    add_site(h, &app, &path);
    sorted(r)
}

/// `publicRead`: the read-only GETs that may be fetched cross-origin.
fn public_read(path: &str) -> bool {
    matches!(
        path,
        "/healthz" | "/api/info" | "/api/examples" | "/api/instruments" | "/llms.txt" | "/llms-full.txt" | "/sitemap.xml" | "/feed.json" | "/feed.atom" | "/robots.txt" | "/schema" | "/schema.jsonld" | "/schema.md" | "/grammar" | "/grammar.ebnf"
            | "/authorship" | "/authorship.md" | "/phrasebook" | "/phrasebook.txt" | "/phrasebook.jsonld" | "/context" | "/context.jsonld" | "/context/v1.jsonld" | "/.well-known/btw-site"
            | "/embed.js" | "/oembed"
    ) || ["/examples/", "/docs/", "/card/", "/api/programs/", "/api/render/", "/p/", "/c/", "/dumps/", "/phrasebook/"].iter().any(|p| path.starts_with(p))
}

fn add_site(h: &mut HeaderMap, app: &App, path: &str) {
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.entry("content-security-policy").or_insert(HeaderValue::from_static(CSP));
    // the embed player is the one page other sites may frame (its CSP says `frame-ancestors *`: cards/embed.rs)
    if !path.starts_with(cards::EMBED_PREFIX) {
        h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    }
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    // a handler may set its own (GET /pair: no-referrer), as Go's securityHeaders runs before the handler
    h.entry("referrer-policy").or_insert(HeaderValue::from_static("strict-origin-when-cross-origin"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"));
    if app.cfg.public_url.starts_with("https://") {
        h.insert("strict-transport-security", HeaderValue::from_static("max-age=31536000"));
    }
    // licenseHeaders: what the site stores and serves publicly (CC0)
    if path.starts_with("/api/render/") || path.starts_with("/api/programs/") || path == "/api/compose" || path == "/api/device/compose" || path.starts_with("/c/") {
        h.insert("x-content-license", HeaderValue::from_static("CC0-1.0"));
    }
    // Go's middleware sets these Links before the handler runs, so a handler's own `Link` (h.Set) replaces them
    if (path.starts_with("/api/render/") || path.starts_with("/api/programs/") || path == "/api/compose" || path == "/api/device/compose" || path.starts_with("/c/")) && !h.contains_key(header::LINK) {
        h.append(header::LINK, HeaderValue::from_str(&format!("<{}/privacy>; rel=\"privacy-policy\"", app.cfg.public_url)).unwrap());
        h.append(header::LINK, HeaderValue::from_static("<https://creativecommons.org/publicdomain/zero/1.0/>; rel=\"license\""));
    }
}

/// Go writes headers sorted by name; so do we (and title-cased, by the connection builder).
fn sorted(mut r: Response<Body>) -> Response<Body> {
    let old = std::mem::take(r.headers_mut());
    let mut names: Vec<HeaderName> = old.keys().cloned().collect();
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let h = r.headers_mut();
    for n in names {
        for v in old.get_all(&n) {
            h.append(n.clone(), v.clone());
        }
    }
    r
}

/// `handleHealthz`: `{"name":"btw","ok":true,"version":…}` (keys sorted, as encoding/json writes a map).
async fn healthz(State(app): State<AppState>) -> Response<Body> {
    let body = go_json(&json!({"name": "btw", "ok": true, "version": app.cfg.version})) + "\n";
    Response::builder().header(header::CACHE_CONTROL, "no-store").header(header::CONTENT_TYPE, "application/json; charset=utf-8").body(Body::from(body)).unwrap()
}

/// Everything that is not a render route: the docs, the examples, a Go route (to `-fallback`), else the static site.
async fn dispatch(State(app): State<AppState>, req: Request) -> Response<Body> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    if let Some(r) = mux_redirect(&method, &path, req.uri().query()) {
        return r;
    }
    if let Some(r) = app.docs.respond(&method, req.headers(), &path, &app.examples) {
        return r;
    }
    if let Some(r) = opensite::respond(&app, &method, req.headers(), &path) {
        return r;
    }
    let get = method == Method::GET || method == Method::HEAD;
    if path == "/api/examples" && get {
        return site::examples_json(&app.examples);
    }
    if let Some(name) = path.strip_prefix("/examples/").filter(|n| !n.is_empty() && !n.contains('/')) {
        if get {
            return site::example_file(&app.examples, name);
        }
    }
    if proxy::go_owned(&path) && !(get && (path == "/healthz" || path.starts_with("/api/render/") || path == "/api/examples")) {
        if let Some(f) = &app.fallback {
            let peer = req.extensions().get::<Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
            return f.forward(req, peer).await;
        }
        if path.starts_with("/api/") && !get_owned(&path) {
            return ApiError::new(404, "not_found", "this route is still the Go server's (btwserve has no -fallback)").response();
        }
    }
    // `/?cid=<CID>`: the editor's page with the stored public program in it, as a `<beatscript-view>` block it loads
    if get && path == "/" {
        let cid = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("cid="))).unwrap_or("");
        if store::is_cid(cid) {
            let view = cards::stored_view(&app.cfg.programs, cid);
            return app.site.serve_with(&method, &path, req.headers(), view.as_deref());
        }
    }
    app.site.serve(&method, &path, req.headers())
}

/// The redirects Go's ServeMux makes before any handler runs: a path that is not clean (`//x`, `/a/../b`, `/./x`) goes
/// to its clean form, and a subtree root asked without its slash (`/docs`) to the slash (`/docs/{$}`); 307, the query
/// kept, the body `http.Redirect` writes. `/c` and `/card` are Go's subtrees and go to -fallback with the rest.
fn mux_redirect(method: &Method, raw: &str, query: Option<&str>) -> Option<Response<Body>> {
    let mut clean = site::clean(raw);
    if raw.ends_with('/') && clean != "/" {
        clean.push('/');
    }
    let target = if clean != raw {
        clean
    } else if raw == "/docs" {
        "/docs/".to_string()
    } else {
        return None;
    };
    let loc = match query {
        Some(q) => format!("{target}?{q}"),
        None => target,
    };
    // http.Redirect: the type for GET and HEAD, the little body for GET only (a HEAD then has no length, as in Go)
    let mut r = Response::builder().status(307);
    if method == Method::GET || method == Method::HEAD {
        r = r.header(header::CONTENT_TYPE, "text/html; charset=utf-8");
    }
    r = r.header(header::LOCATION, &loc);
    let body = match *method {
        Method::GET => Body::from(format!("<a href=\"{}\">Temporary Redirect</a>.\n\n", site::html_escape(&loc))),
        Method::HEAD => content::nothing(),
        _ => Body::empty(),
    };
    Some(r.body(body).unwrap())
}

/// Paths under /api/ whose GET is btwserve's (a wrong method there is the static site's 405, as in Go).
fn get_owned(path: &str) -> bool {
    path == "/api/examples" || path.starts_with("/api/render/")
}

/// The response body, counted, with the access-log line written when it is done (or dropped: a client gone, an
/// aborted stream), as the Go middleware logs after the handler returns.
struct Logged {
    inner: Body,
    bytes: u64,
    done: Option<(Arc<log::AccessLog>, SystemTime, Instant, String, String, u16, String)>,
}

impl http_body::Body for Logged {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        let r = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(f))) = &r {
            if let Some(d) = f.data_ref() {
                self.bytes += d.len() as u64;
            }
        }
        r
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Logged {
    fn drop(&mut self) {
        if let Some((l, at, t0, method, path, status, ip)) = self.done.take() {
            l.write(at, &method, &path, status, t0.elapsed().as_millis() as u64, self.bytes, &ip);
        }
    }
}

/// The lineage index of the programs store and the examples: one per process, shared by `/p/<cid>/tree` and the
/// MCP `lineage` tool.
fn lineage(app: &App) -> Arc<btwserve::lineage::Lineage> {
    btwserve::lineage::shared(&app.cfg.programs, || {
        app.examples
            .list
            .iter()
            .filter_map(|e| Some(btwserve::lineage::Example { cid: e.cid.clone(), name: e.name.clone(), license: e.license.clone(), text: app.examples.text.get(&e.name)?.clone() }))
            .collect()
    })
}

/// An App on a fresh data dir, with no web, docs, log or examples, as `main` builds one: for the tests of the routes'
/// and tools' answers.
#[cfg(test)]
fn test_app(data: &std::path::Path) -> AppState {
    let d = data.to_string_lossy().to_string();
    let cfg = config_from(["-data", &d, "-log-dir", "off", "-web", "", "-docs", ""].iter().map(|s| s.to_string()).collect()).expect("test config");
    let (store, _) = Store::open(&cfg.programs, None, false).expect("programs store");
    let limits = Limits { cache_mb: 16, concurrency: 1, ip_hour: 30, max_bars: 64, max_seconds: 60, workers: 1, admit_lag: Duration::ZERO };
    let rs = Arc::new(Service::new(&cfg.render_dir, limits).expect("render dir"));
    let (examples, _) = site::load_examples(None).expect("examples");
    Arc::new(App {
        put_ip: Mutex::new(Limiter::new(cfg.put_ip_hour, Duration::from_secs(3600))),
        cfg: cfg.clone(),
        store,
        rs,
        reports: Mutex::new(HashMap::new()),
        check_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        site: site::Static::new(cfg.web.clone(), &cfg.public_url, cfg.card_meta),
        docs: docs::Docs::new(cfg.docs.clone(), cfg.docs_src.clone(), &cfg.public_url),
        examples,
        fallback: None,
        checker: Default::default(),
        authsvc: Default::default(),
        hub: Default::default(),
        catalog: btwserve::opendata::Catalog::new(cfg.programs.clone()),
        beta: Default::default(),
    })
}

#[tokio::main]
async fn main() {
    if std::env::var_os(ontology::query::WORKER_ENV).is_some() {
        return ontology::worker_main(); // an /ontology/ query worker, started by this server
    }
    let cfg = match parse_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("btwserve: {e}");
            std::process::exit(2);
        }
    };
    let (store, warnings) = Store::open(&cfg.programs, cfg.examples.as_deref(), cfg.programs_readonly).expect("programs store");
    for w in warnings {
        eprintln!("btwserve: warning: {w}");
    }
    let limits = Limits {
        cache_mb: cfg.render_cache_mb,
        concurrency: cfg.render_concurrency,
        ip_hour: cfg.render_ip_hour,
        max_bars: cfg.render_max_bars,
        max_seconds: cfg.render_max_seconds,
        workers: cfg.render_workers,
        admit_lag: Duration::from_millis(cfg.admit_lag_ms),
    };
    let rs = Arc::new(Service::new(&cfg.render_dir, limits).expect("render dir"));
    // Go's auth.LoadSecret runs before the log is opened: a fresh data dir gets its session.secret first, so the
    // access log's address key is the lasting one (and a corrupt secret stops the start, as in Go)
    let secret_env = std::env::var("BTW_SESSION_SECRET").ok().map(|s| s.trim().to_string()).unwrap_or_default();
    if let Err(e) = authsvc::load_secret(&cfg.data, &secret_env) {
        eprintln!("btwserve: {e}");
        std::process::exit(1);
    }
    let log = cfg.log_dir.as_ref().map(|d| log::AccessLog::new(d, &log::load_secret(&cfg.secret_file)).expect("log dir"));
    let (examples, ex_warn) = match site::load_examples(cfg.examples.as_deref()) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("btwserve: {e}");
            std::process::exit(1);
        }
    };
    for w in ex_warn {
        eprintln!("btwserve: warning: {w}");
    }
    let app = Arc::new(App {
        put_ip: Mutex::new(Limiter::new(cfg.put_ip_hour, Duration::from_secs(3600))),
        cfg: cfg.clone(),
        store,
        rs: rs.clone(),
        reports: Mutex::new(HashMap::new()),
        check_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        site: site::Static::new(cfg.web.clone(), &cfg.public_url, cfg.card_meta),
        docs: docs::Docs::new(cfg.docs.clone(), cfg.docs_src.clone(), &cfg.public_url),
        examples,
        fallback: cfg.fallback.as_deref().map(proxy::Fallback::new),
        checker: Default::default(),
        authsvc: Default::default(),
        hub: Default::default(),
        catalog: btwserve::opendata::Catalog::new(cfg.programs.clone()),
        beta: Default::default(),
    });
    // an `import` (and a borrowed part) of a CID is resolved from the programs this site keeps, in every check it makes: the
    // check, share, open, render and the storage gate. Only public Creative Commons programs and the examples are in the store.
    btwapi::known::set_source(Arc::new(StoreSnippets(app.clone())));
    match caps::Caps::open(&cfg.data, &cfg.public_url) {
        Ok(c) => caps::init(Arc::new(c)),
        Err(e) => {
            eprintln!("btwserve: caps: {e}");
            std::process::exit(1);
        }
    }
    // the phase modules, in dependency order: each may read what the earlier ones put on App
    let phases: Router = Router::new()
        .merge(public::routes(&app))
        .merge(authsvc::routes(&app))
        .merge(cards::routes(&app))
        .merge(hub::routes(&app))
        .merge(compose::routes(&app))
        .merge(beta::routes(&app))
        .merge(mcp::routes(&app))
        .merge(oauth::routes(&app))
        .merge(ontology::routes(&app));
    {
        // orphaned .part files (a crash, the other server's), every five minutes
        let rs = rs.clone();
        std::thread::Builder::new()
            .name("part sweep".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(300));
                let n = rs.sweep_parts();
                if n > 0 {
                    eprintln!("btwserve: removed {n} orphaned .part file(s)");
                }
            })
            .expect("sweep thread");
    }
    let secret_env = std::env::var("BTW_SESSION_SECRET").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let pl_cfg = btwserve::playlists::Config {
        data: cfg.data.clone(),
        secret_env,
        public_url: cfg.public_url.clone(),
        hosts: btwserve::playlists::Config::hosts_from(&cfg.public_url, &cfg.alt_origins, &cfg.legacy_hosts),
        limits: cfg.playlist_limits.clone(),
        gate: btwserve::playlists::ItemGate::new(cfg.programs.clone(), app.examples.list.iter().map(|e| e.cid.clone())),
    };
    let (playlists, pl_warn) = btwserve::playlists::Playlists::open(pl_cfg).expect("playlists dir");
    for w in pl_warn {
        eprintln!("btwserve: playlists: warning: {w}");
    }
    let router = Router::new()
        .route("/healthz", get(healthz).fallback(dispatch))
        .route("/api/render/{cid}", get(handle_render).fallback(dispatch))
        .route("/api/render/{cid}/status", get(handle_status).fallback(dispatch))
        .fallback(dispatch)
        .with_state(app.clone())
        .merge(btwserve::playlists::router(playlists))
        .merge(btwserve::lineage::router(lineage(&app), &cfg.public_url))
        .merge(phases)
        .layer(middleware::from_fn_with_state(app.clone(), site_headers));

    let ln = tokio::net::TcpListener::bind(&cfg.addr).await.expect("listen");
    let (files, used, cap) = rs.cache_usage();
    eprintln!(
        "btwserve: listening on {} (public {}, programs {}{}, render {} [{files} files, {} of {} MB], {} CPU workers, at most {} renders, {} bars / {} s)",
        ln.local_addr().unwrap(),
        cfg.public_url,
        cfg.programs.display(),
        if cfg.programs_readonly { " read-only" } else { "" },
        cfg.render_dir.display(),
        used >> 20,
        cap >> 20,
        rs.sched.workers(),
        cfg.render_concurrency,
        cfg.render_max_bars,
        cfg.render_max_seconds
    );
    loop {
        let (sock, peer) = tokio::select! {
            r = ln.accept() => match r { Ok(x) => x, Err(_) => continue },
            _ = tokio::signal::ctrl_c() => break,
        };
        let _ = sock.set_nodelay(true);
        let router = router.clone();
        let log = log.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(Peer(peer));
                req.extensions_mut().insert(btwserve::playlists::PeerAddr(peer));
                let router = router.clone();
                let log = log.clone();
                async move {
                    let (at, t0) = (SystemTime::now(), Instant::now());
                    let line = log.as_ref().map(|_| (req.method().to_string(), req.uri().path().to_string(), client_ip(peer, req.headers())));
                    let r = tower::ServiceExt::oneshot(router, req.map(Body::new)).await.unwrap();
                    let status = r.status().as_u16();
                    // the Go server logs what it answered
                    let line = if r.extensions().get::<proxy::Proxied>().is_some() { None } else { line };
                    let r = r.map(|inner| {
                        let done = log.zip(line).map(|(l, (m, p, ip))| (l, at, t0, m, p, status, ip));
                        Body::new(Logged { inner, bytes: 0, done })
                    });
                    Ok::<_, Infallible>(r)
                }
            });
            let _ = hyper::server::conn::http1::Builder::new().timer(hyper_util::rt::TokioTimer::new()).title_case_headers(true).header_read_timeout(Duration::from_secs(10)).serve_connection(TokioIo::new(sock), svc).await;
        });
    }
    eprintln!("btwserve: shutting down");
}
