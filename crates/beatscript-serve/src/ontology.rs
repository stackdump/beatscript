//! `/ontology/`: the BeatScript ontology, served from beatscript.app (the query side is a port of ontology-bitwrap-io's).
//!
//! The BeatScript ontology is a fork of the bitwrap ontology 0.2.0 that evolves on its own: its terms are under
//! `https://beatscript.app/ontology/` (vocabulary `…/ns#`), each one that came from bitwrap linked back to its bitwrap
//! IRI by `prov:wasDerivedFrom`, and the BeatScript vocabulary (grammar, voices, control registry, MCP tools) is
//! generated from this server's own code. `ontology/release/` is built by `ontology/build.sh` (`src/ontology/fork.rs`)
//! from `ontology/upstream/bitwrap-0.2.0/ns.nt`, `ontology/fork.ttl` and `ontology/generated/beatscript.ttl`, pinned by
//! `ontology/release.lock` and embedded here; nothing is fetched from bitwrap, at build or at serve time. `tests`
//! holds the served release to the vendored files, the files to what their inputs build, and the vocabulary to the code.
//!
//! - `GET /ontology/` the docs page; `/ontology/ns` (and the version IRI, `/ontology/<version>`) content-negotiated
//!   (Turtle, JSON-LD, N-Triples, RDF/XML; a browser is sent to the page); `/ontology/ns.{ttl,jsonld,nt,rdf}`.
//! - `GET /ontology/<kind>/<name>` a term's own IRI (`/ontology/genre/techno`): what the release asserts of it, as
//!   Turtle (N-Triples on request).
//! - `GET /ontology/release` the release as get_release answers it; `/ontology/release/<file>` and
//!   `/ontology/cert/<file>` the release files.
//! - `/ontology/sparql` the SPARQL 1.1 protocol, read-only; `/ontology/mcp` the ontology's MCP server (`mcp.rs`).
//!   Both run over the materialized graph (the pack's), capped at 1000 rows and 5 seconds, rate limited per client.
//!   Queries run in two worker processes (this binary, `query::WORKER_ENV`): one past its deadline is killed.

#[cfg(test)]
pub mod fork;
pub mod graph;
pub mod mcp;
pub mod query;
#[cfg(test)]
pub mod vocab;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::any;
use axum::Router;
use bytes::Bytes;
use serde_json::{json, Value};

use crate::compose::gojson;
use crate::content;
use crate::site::html_escape;
use graph::{parse_term, Index, NS};
use query::{Answer, Service};

/// The release files, embedded. `release.lock` pins their bytes.
pub const FILES: [(&str, &[u8]); 10] = [
    ("ns.ttl", include_bytes!("../ontology/release/ns.ttl")),
    ("ns.nt", include_bytes!("../ontology/release/ns.nt")),
    ("ns.jsonld", include_bytes!("../ontology/release/ns.jsonld")),
    ("ns.rdf", include_bytes!("../ontology/release/ns.rdf")),
    ("manifest.json", include_bytes!("../ontology/release/manifest.json")),
    ("pack.json", include_bytes!("../ontology/release/pack.json")),
    ("cert/asserted.sha256", include_bytes!("../ontology/release/cert/asserted.sha256")),
    ("cert/asserted.tsv", include_bytes!("../ontology/release/cert/asserted.tsv")),
    ("cert/derivations.tsv", include_bytes!("../ontology/release/cert/derivations.tsv")),
    ("cert/scope.tsv", include_bytes!("../ontology/release/cert/scope.tsv")),
];
#[cfg_attr(not(test), allow(dead_code))]
pub const LOCK: &str = include_str!("../ontology/release.lock");
const CSS: &str = include_str!("../ontology/ontology.css");

/// The bitwrap ontology's own site: the upstream the fork derives from (named as attribution text, never linked or fetched).
pub const UPSTREAM: &str = "https://ontology.bitwrap.io";

pub struct Ontology {
    pub version: String,
    pub manifest: Value,
    pub sp: Arc<Service>,
    pub graph: Index,
    public: String,
}

/// A row value as the Go server plains it (`parseTerm(raw).Value`).
pub fn parse_value(raw: &str) -> Option<String> {
    parse_term(raw).map(|t| t.value)
}

/// The materialized graph: pack.json's `graph` (encoding/json matches the Go struct field `Graph` case-insensitively;
/// the pack writes `graph`). Empty when the pack has none, which the engine then refuses.
pub fn pack_graph() -> String {
    let pack: Value = serde_json::from_slice(file("pack.json").unwrap()).unwrap_or(Value::Null);
    pack.get("graph").or_else(|| pack.get("Graph")).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A query worker's main (`query::WORKER_ENV` set): main() calls it first and exits when it returns.
pub fn worker_main() {
    query::worker_loop(pack_graph().as_bytes());
}

fn file(name: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

impl Ontology {
    /// Loads the embedded release: the manifest, the asserted index (ns.nt) and the engine over the materialized graph
    /// (pack.json's `graph`), as ontology-bitwrap-io runs it with `-dataset materialized`. Queries run in this process.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn load(public_url: &str) -> Result<Ontology, String> {
        Ontology::load_with(public_url, None)
    }

    /// As [`Ontology::load`], with queries in worker processes started by `workers` (production).
    pub fn load_with(public_url: &str, workers: Option<query::WorkerCmd>) -> Result<Ontology, String> {
        let manifest: Value = serde_json::from_slice(file("manifest.json").unwrap()).map_err(|e| format!("manifest.json: {e}"))?;
        let version = manifest["version"].as_str().unwrap_or("").to_string();
        let g = pack_graph();
        let sp = Arc::new(match workers {
            Some(cmd) => Service::with_workers(g.as_bytes(), cmd)?,
            None => Service::new(g.as_bytes())?,
        });
        Ok(Ontology { version, manifest, sp, graph: Index::from_nt(file("ns.nt").unwrap()), public: public_url.trim_end_matches('/').to_string() })
    }

    /// get_release: `json.MarshalIndent(manifest, "", "  ")` (a Go map: keys sorted at every level).
    pub fn release_json(&self) -> String {
        gojson::indent(&gojson::sorted_deep(self.manifest.clone()), "  ")
    }
}

static SHARED: std::sync::OnceLock<Option<Arc<Ontology>>> = std::sync::OnceLock::new();

/// The embedded release, loaded once per process, for any module that reads the ontology: `graph` (the asserted
/// index: `first`, `all`, `num`, `of_type`) or `sp.run(query, "")` (a guarded SPARQL query over the materialized
/// graph, capped and timed out as the public endpoint's; an empty ip never earns a strike). None: it did not load.
pub fn shared(public_url: &str) -> Option<Arc<Ontology>> {
    SHARED
        .get_or_init(|| match Ontology::load_with(public_url, Some(query::WorkerCmd::this_binary())) {
            Ok(o) => Some(Arc::new(o)),
            Err(e) => {
                eprintln!("beatscript-serve: warning: ontology not served: {e}");
                None
            }
        })
        .clone()
}

/// The genre conventions' graph (`beatscript_serve::genres`: check's `style`, the MCP `genre_template`): the fork's own
/// release (`ns.nt`, served at /ontology/ns.nt), which carries the bitwrap 0.2.0 terms and bitwrap's conventions module
/// re-homed under https://beatscript.app/ontology/, so every IRI `genres` answers with is this site's.
struct GenreGraph(graph::Index);

impl beatscript_serve::genres::Graph for GenreGraph {
    fn all(&self, s: &str, p: &str) -> Vec<String> {
        self.0.all(s, p)
    }
    fn of_type(&self, class: &str) -> Vec<String> {
        self.0.of_type(class)
    }
}

/// The genre conventions' graph: the release's asserted graph, as N-Triples.
pub fn genre_graph() -> graph::Index {
    graph::Index::from_nt(FILES[1].1)
}

/// Give the process its genre conventions, once (genres::install ignores a second call). Until this ran, check's style and
/// genre_template answered that the conventions were not available.
pub fn install_genres() {
    beatscript_serve::genres::install(Arc::new(GenreGraph(genre_graph())));
}

pub fn routes(app: &crate::AppState) -> Router {
    install_genres();
    match shared(&app.cfg.public_url) {
        Some(o) => {
            eprintln!("beatscript-serve: ontology {} ({} triples materialized) at /ontology/", o.version, o.sp.len());
            router(o)
        }
        None => Router::new(),
    }
}

pub fn router(o: Arc<Ontology>) -> Router {
    let version_iri = format!("/ontology/{}", o.version);
    Router::new()
        .route("/ontology", any(to_slash))
        .route("/ontology/", any(page))
        .route("/ontology/assets/ontology.css", any(css))
        .route("/ontology/health", any(health))
        .route("/ontology/ns", any(ns))
        .route("/ontology/ns.{ext}", any(ns_file))
        .route(&version_iri, any(ns))
        .route("/ontology/{kind}/{name}", any(term))
        .route("/ontology/release", any(release))
        .route("/ontology/release/{*file}", any(release_file))
        .route("/ontology/cert/{file}", any(cert_file))
        .route("/ontology/sparql", any(sparql))
        .route("/ontology/mcp", any(mcp_route))
        .with_state(o)
}

type St = State<Arc<Ontology>>;

fn ip_of(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

fn get_only(m: &Method) -> Option<Response<Body>> {
    if m == Method::GET || m == Method::HEAD {
        return None;
    }
    let mut r = content::text_error(405, "method not allowed");
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    Some(r)
}

/// `serveFile` / `serveBytes`: the bytes, CORS open, five minutes' cache, an ETag that follows the bytes (the same
/// tag ontology.bitwrap.io sends for the same file).
fn serve_bytes(req: &Request, b: Bytes, ct: &str, extra: &[(&str, &str)]) -> Response<Body> {
    if let Some(r) = get_only(req.method()) {
        return r;
    }
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(ct).unwrap());
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
    h.insert(header::ETAG, HeaderValue::from_str(&format!("\"{}\"", content::sha_hex(&b, 8))).unwrap());
    for (k, v) in extra {
        h.insert(header::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    let n = b.len() as u64;
    content::serve(h, &content::Req { method: req.method(), headers: req.headers() }, n, Some(b))
}

fn serve_file(req: &Request, name: &str) -> Response<Body> {
    let Some(b) = file(name) else { return content::text_error(404, "404 page not found") };
    let ct = match name.rsplit('.').next().unwrap_or("") {
        "ttl" => "text/turtle; charset=utf-8",
        "nt" => "application/n-triples; charset=utf-8",
        "jsonld" => "application/ld+json; charset=utf-8",
        "rdf" => "application/rdf+xml; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "tsv" => "text/tab-separated-values; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    };
    serve_bytes(req, Bytes::from_static(b), ct, &[("vary", "Accept")])
}

async fn to_slash(req: Request) -> Response<Body> {
    let loc = match req.uri().query() {
        Some(q) => format!("/ontology/?{q}"),
        None => "/ontology/".into(),
    };
    Response::builder().status(308).header(header::LOCATION, loc).body(Body::empty()).unwrap()
}

async fn css(req: Request) -> Response<Body> {
    serve_bytes(&req, Bytes::from_static(CSS.as_bytes()), "text/css; charset=utf-8", &[])
}

async fn health(State(o): St) -> Response<Body> {
    Response::builder().header(header::CONTENT_TYPE, "text/plain; charset=utf-8").header(header::CACHE_CONTROL, "no-store").body(Body::from(format!("ok {}\n", o.version))).unwrap()
}

// ---------- content negotiation (site.go `negotiate`) ----------

/// The offer the Accept header ranks highest; earlier offers win ties, so a bare `*/*` gets the first. None: nothing
/// acceptable.
pub fn negotiate<'a>(accept: &str, offers: &[&'a str]) -> Option<&'a str> {
    if accept.trim().is_empty() {
        return Some(offers[0]);
    }
    let ranges: Vec<(String, f64)> = accept
        .split(',')
        .map(|part| {
            let mut fields = part.trim().split(';');
            let mime = fields.next().unwrap_or("").trim().to_lowercase();
            let mut q = 1.0;
            for p in fields {
                if let Some((k, v)) = p.trim().split_once('=') {
                    if k == "q" {
                        if let Ok(f) = v.parse::<f64>() {
                            q = f;
                        }
                    }
                }
            }
            (mime, q)
        })
        .collect();
    let (mut best, mut best_q) = (None, 0.0);
    for o in offers {
        for (r, q) in &ranges {
            let m = r == o || r == "*/*" || (r.ends_with("/*") && o.starts_with(&r[..r.len() - 1]));
            let score = if m && r == o { q + 0.001 } else { *q };
            if m && *q > 0.0 && score > best_q {
                best = Some(*o);
                best_q = score;
            }
        }
    }
    best
}

const NS_OFFERS: [&str; 5] = ["text/turtle", "application/ld+json", "application/n-triples", "application/rdf+xml", "text/html"];

async fn ns(req: Request) -> Response<Body> {
    let accept = req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut r = match negotiate(&accept, &NS_OFFERS) {
        None => content::text_error(406, "no acceptable representation; try text/turtle, application/ld+json, application/n-triples or application/rdf+xml"),
        Some("text/html") => Response::builder().status(303).header(header::LOCATION, "/ontology/").body(Body::empty()).unwrap(),
        Some(m) => {
            let f = match m {
                "text/turtle" => "ns.ttl",
                "application/ld+json" => "ns.jsonld",
                "application/n-triples" => "ns.nt",
                _ => "ns.rdf",
            };
            serve_file(&req, f)
        }
    };
    r.headers_mut().insert(header::VARY, HeaderValue::from_static("Accept"));
    r
}

async fn ns_file(Path(ext): Path<String>, req: Request) -> Response<Body> {
    match ext.as_str() {
        "ttl" | "nt" | "jsonld" | "rdf" => serve_file(&req, &format!("ns.{ext}")),
        _ => content::text_error(404, "404 page not found"),
    }
}

/// A term's IRI (`{SITE_BASE}/genre/techno`, `…/slot/edm-drop-2-3`, `…/concept/grammar-term`): the asserted triples
/// with it as subject, sorted (N-Triples, which is also Turtle). 404 for a name the release does not describe. The
/// static routes (`release`, `cert`, `assets`) are matched first; no term lives under those segments.
async fn term(Path((kind, name)): Path<(String, String)>, req: Request) -> Response<Body> {
    if !graph::is_slug(&kind) || !graph::is_slug(&name) {
        return content::text_error(404, "404 page not found");
    }
    let head = format!("<{}/{kind}/{name}> ", graph::SITE_BASE);
    let nt = String::from_utf8_lossy(file("ns.nt").unwrap());
    let body: String = nt.lines().filter(|l| l.starts_with(&head)).map(|l| format!("{l}\n")).collect();
    if body.is_empty() {
        return content::text_error(404, "no such term in this release");
    }
    let accept = req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let ct = match negotiate(&accept, &["text/turtle", "application/n-triples", "text/plain", "text/html"]) {
        Some("application/n-triples") => "application/n-triples; charset=utf-8",
        Some("text/plain") | Some("text/html") => "text/plain; charset=utf-8",
        _ => "text/turtle; charset=utf-8",
    };
    serve_bytes(&req, Bytes::from(body), ct, &[("vary", "Accept")])
}

async fn release(State(o): St, req: Request) -> Response<Body> {
    serve_bytes(&req, Bytes::from(o.release_json()), "application/json; charset=utf-8", &[])
}

async fn release_file(Path(f): Path<String>, req: Request) -> Response<Body> {
    serve_file(&req, &f)
}

async fn cert_file(Path(f): Path<String>, req: Request) -> Response<Body> {
    serve_file(&req, &format!("cert/{f}"))
}

// ---------- /ontology/sparql (sparql.go) ----------

fn deny(wait: std::time::Duration) -> Response<Body> {
    let mut r = content::text_error(429, "rate limit exceeded or client paused after repeated timeouts");
    r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_str(&(wait.as_secs() + 1).to_string()).unwrap());
    r
}

async fn query_from_request(req: Request) -> Result<String, String> {
    match *req.method() {
        Method::GET => Ok(req.uri().query().map(|q| form_get(q, "query")).unwrap_or_default()),
        Method::POST => {
            let ct = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_lowercase();
            let body = axum::body::to_bytes(req.into_body(), query::MAX_QUERY_BYTES + 1).await.map_err(|e| e.to_string())?;
            match ct.as_str() {
                "application/sparql-query" => Ok(String::from_utf8_lossy(&body).into_owned()),
                "application/x-www-form-urlencoded" => {
                    let s = String::from_utf8_lossy(&body);
                    if !form_get(&s, "update").is_empty() {
                        return Err("updates are not accepted".into());
                    }
                    Ok(form_get(&s, "query"))
                }
                "application/sparql-update" => Err("updates are not accepted".into()),
                _ => Err("unsupported Content-Type".into()),
            }
        }
        _ => Err("use GET or POST".into()),
    }
}

/// The first value of `key` in a urlencoded string.
fn form_get(q: &str, key: &str) -> String {
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if url_decode(k) == key {
            return url_decode(v);
        }
    }
    String::new()
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push(hex(b[i + 1]).unwrap() * 16 + hex(b[i + 2]).unwrap());
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn sparql(State(o): St, req: Request) -> Response<Body> {
    let cors = |mut r: Response<Body>| {
        r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        r.headers_mut().insert(header::VARY, HeaderValue::from_static("Accept"));
        r
    };
    if req.method() == Method::OPTIONS {
        let r = Response::builder().header("access-control-allow-methods", "GET, POST, OPTIONS").header("access-control-allow-headers", "Content-Type, Accept").body(Body::empty()).unwrap();
        return cors(r);
    }
    let ip = ip_of(&req);
    let accept = req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let q = match query_from_request(req).await {
        Ok(q) => q,
        Err(e) => return cors(content::text_error(400, &e)),
    };
    if q.is_empty() {
        return cors(content::text_error(400, "no query: send ?query=… or POST application/sparql-query"));
    }
    if let Err(wait) = o.sp.gate(&ip) {
        return cors(deny(wait));
    }
    let res = match o.sp.run(&q, &ip).await {
        Ok(r) => r,
        Err(e) => return cors(content::text_error(e.status(), &e.to_string())),
    };
    let mut b = Response::builder();
    if res.truncated {
        b = b.header("x-truncated", "rows capped at 1000");
    }
    let (ct, body) = match res.answer {
        Answer::Ask(v) => ("application/sparql-results+json", gojson::line(&json!({"boolean": v, "head": {}}))),
        Answer::Graph(ts) => (if accept.contains("text/turtle") { "text/turtle" } else { "application/n-triples" }, query::triples_to_nt(&ts)),
        Answer::Select { vars, rows } if accept.contains("text/csv") && !accept.contains("json") => {
            let mut s = csv_line(&vars);
            for row in &rows {
                let rec: Vec<String> = vars.iter().map(|v| row.iter().find(|(k, _)| k == v).and_then(|(_, raw)| parse_value(raw)).unwrap_or_default()).collect();
                s.push_str(&csv_line(&rec));
            }
            ("text/csv", s)
        }
        Answer::Select { vars, rows } => {
            let bindings: Vec<Value> = rows
                .iter()
                .map(|row| gojson::sorted(Value::Object(row.iter().filter_map(|(k, raw)| parse_term(raw).map(|t| (k.clone(), t.to_json()))).collect())))
                .collect();
            ("application/sparql-results+json", gojson::line(&json!({"head": {"vars": vars}, "results": {"bindings": bindings}})))
        }
    };
    cors(b.header(header::CONTENT_TYPE, ct).body(crate::go_body(body)).unwrap())
}

/// encoding/csv's Writer: quote a field holding `"`, `,`, CR/LF or a leading space; CRLF never, `\n` line ends.
fn csv_line(fields: &[String]) -> String {
    let mut s = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        if f.contains(['"', ',', '\n', '\r']) || f.starts_with(' ') || f.starts_with('\t') {
            s.push('"');
            s.push_str(&f.replace('"', "\"\""));
            s.push('"');
        } else {
            s.push_str(f);
        }
    }
    s.push('\n');
    s
}

// ---------- /ontology/mcp ----------

async fn mcp_route(State(o): St, req: Request) -> Response<Body> {
    let cors = |mut r: Response<Body>| {
        r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        r
    };
    match *req.method() {
        Method::OPTIONS => {
            return cors(Response::builder().header("access-control-allow-methods", "POST, OPTIONS").header("access-control-allow-headers", "Content-Type, Accept, Mcp-Session-Id, MCP-Protocol-Version").body(Body::empty()).unwrap());
        }
        Method::POST => {}
        _ => {
            let mut r = content::text_error(405, "this server answers POST only (no SSE stream)");
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
            return cors(r);
        }
    }
    let ip = ip_of(&req);
    let body = match axum::body::to_bytes(req.into_body(), 64 << 10).await {
        Ok(b) => b,
        Err(_) => return cors(content::text_error(400, "bad body")),
    };
    match mcp::handle(&o, &body, &ip).await {
        None => cors(Response::builder().status(202).body(Body::empty()).unwrap()),
        Some(line) => cors(Response::builder().header(header::CONTENT_TYPE, "application/json").body(crate::go_body(line)).unwrap()),
    }
}

// ---------- the docs page ----------

async fn page(State(o): St, req: Request) -> Response<Body> {
    if let Some(r) = get_only(req.method()) {
        return r;
    }
    let html = render_page(&o);
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::ETAG, HeaderValue::from_str(&format!("\"{}\"", content::sha_hex(html.as_bytes(), 8))).unwrap());
    let b = Bytes::from(html);
    let n = b.len() as u64;
    content::serve(h, &content::Req { method: req.method(), headers: req.headers() }, n, Some(b))
}

pub fn render_page(o: &Ontology) -> String {
    let e = html_escape;
    let m = &o.manifest;
    let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
    let p = &o.public;
    let mut files = String::new();
    if let Some(f) = m["files"].as_object() {
        let mut names: Vec<&String> = f.keys().collect();
        names.sort();
        for n in names {
            let href = if n.starts_with("cert/") { format!("/ontology/{n}") } else { format!("/ontology/release/{n}") };
            files.push_str(&format!("<tr><td><a href=\"{}\"><code>{}</code></a></td><td><code class=\"hash\">{}</code></td></tr>\n", e(&href), e(n), e(f[n].as_str().unwrap_or(""))));
        }
    }
    let tools: String = mcp::tool_defs()
        .iter()
        .map(|t| {
            let d = t["description"].as_str().unwrap_or("");
            let first = d.split(". ").next().unwrap_or(d).trim_end_matches('.');
            format!("<li><code>{}</code>: {}.</li>\n", e(t["name"].as_str().unwrap_or("")), e(first))
        })
        .collect();
    let body = format!(
        r##"<h1>BeatScript ontology</h1>
<p>The vocabulary behind BeatScript: the genres the generator knows (tempo, scale, groove, Euclidean drum rhythms, instruments, chord progressions), the song structures and sections it arranges them into, the Petri-net engine concepts underneath, and the language itself: every production and keyword of its grammar, its voices and kits, the control nouns and verbs, its operators and the tools of both MCP servers, generated from the code that implements them.</p>
<ul class="facts">
<li>Version <strong>{ver}</strong>, released {rel}: <code>{viri}</code></li>
<li>Derived from the bitwrap ontology {upver} (ontology.bitwrap.io), and evolving on its own</li>
<li>{triples} triples in the materialized graph</li>
<li>Pack digest <code class="hash">{digest}</code></li>
<li>License <a href="https://creativecommons.org/publicdomain/zero/1.0/">CC0 1.0</a>: use it for anything, no attribution required</li>
</ul>

<h2 id="iris"><a class="anchor" href="#iris" aria-label="Link to this section">#</a>Names</h2>
<p>Every term has its IRI here, in the namespace <code>{ns}</code>: classes and properties such as <a href="/ontology/ns"><code>ns#Genre</code></a>, and individuals such as <a href="/ontology/genre/techno"><code>{p}/ontology/genre/techno</code></a>, whose IRI answers with what the release states about it. A term that came from the bitwrap ontology says so: <code>prov:wasDerivedFrom</code> links it to its bitwrap IRI (<code>{up}/genre/techno</code>), and the ontology itself is <code>prov:wasDerivedFrom</code> the bitwrap {upver} release. Nothing else points back, and nothing is fetched from bitwrap: from here the two evolve apart. A program's <code>#! genre:</code> line still names bitwrap's genre IRIs, which stay where they are.</p>

<h2 id="documents"><a class="anchor" href="#documents" aria-label="Link to this section">#</a>Documents</h2>
<ul>
<li><a href="/ontology/ns"><code>/ontology/ns</code></a>: content-negotiated. <code>Accept: text/turtle</code>, <code>application/ld+json</code>, <code>application/n-triples</code> or <code>application/rdf+xml</code>; a browser comes back to this page.</li>
<li><a href="/ontology/ns.ttl">ns.ttl</a> · <a href="/ontology/ns.jsonld">ns.jsonld</a> · <a href="/ontology/ns.nt">ns.nt</a> · <a href="/ontology/ns.rdf">ns.rdf</a>: the asserted ontology</li>
<li><a href="/ontology/release"><code>/ontology/release</code></a>: version, pack digest and the hash of every file (what the <code>get_release</code> tool returns)</li>
<li><a href="/ontology/release/pack.json">pack.json</a>: the portable pack, the materialized graph and its certificate</li>
</ul>
<pre><code>curl -H 'Accept: text/turtle' {p}/ontology/ns</code></pre>

<h2 id="query"><a class="anchor" href="#query" aria-label="Link to this section">#</a>Query</h2>
<p>Queries run over the materialized graph: the asserted ontology plus its RDFS consequences, so the subclass closure is already there. Read-only, at most 1000 rows and 5 seconds a query, rate limited per client.</p>
<ul>
<li><code>/ontology/sparql</code>: the SPARQL 1.1 protocol. <code>SELECT</code>, <code>ASK</code>, <code>CONSTRUCT</code> and <code>DESCRIBE</code>; GET <code>?query=</code> or POST.</li>
<li><code>/ontology/mcp</code>: the same data as an MCP server (Streamable HTTP, stateless, no sign-in). It is the ontology's own server, separate from the BeatScript MCP at <code>/mcp</code>.</li>
</ul>
<pre><code>curl -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=SELECT ?g ?bpm WHERE {{ ?g a &lt;{ns}Genre&gt; ; &lt;{ns}bpm&gt; ?bpm }} ORDER BY ?bpm' \
  {p}/ontology/sparql</code></pre>
<p>The MCP tools:</p>
<ul>
{tools}</ul>
<pre><code>claude mcp add --transport http beatscript-ontology {p}/ontology/mcp</code></pre>

<h2 id="release"><a class="anchor" href="#release" aria-label="Link to this section">#</a>Release files</h2>
<p>SHA-256 of every file, from the release's own <a href="/ontology/release/manifest.json">manifest.json</a>. <code>sha256sum</code> of a downloaded file reproduces its line.</p>
<table>
<thead><tr><th>File</th><th>SHA-256</th></tr></thead>
<tbody>
{files}</tbody>
</table>
<p>The certificate in <a href="/ontology/cert/derivations.tsv"><code>cert/</code></a> covers: {scope}. It is in open-ontologies' <code>oo-cert/1</code> format, one derivation step per inferred triple, and the pack binds it to the graph with the <code>oo-pack-content/1</code> digest. Matching digests prove the files are intact and belong together, not that the ontology is correct. The release is built reproducibly from four inputs: the bitwrap {upver} release, bitwrap's genre conventions module, the fork's own module, and the vocabulary generated from BeatScript's code.</p>
"##,
        up = UPSTREAM,
        ns = NS,
        p = e(p),
        ver = e(&s("version")),
        rel = e(&s("released")),
        viri = e(m["version_iri"].as_str().unwrap_or("")),
        upver = e(m["derived_from"]["version"].as_str().unwrap_or("")),
        triples = m["pack_triples"],
        digest = e(&s("pack_content_sha256")),
        scope = e(&s("certificate_scope")),
        tools = tools,
        files = files,
    );
    shell(o, &body)
}

/// The docs pages' shell (docs.rs `shell`), with this page's alternates.
fn shell(o: &Ontology, body: &str) -> String {
    let p = html_escape(&o.public);
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="theme-color" content="#16213e">
<meta name="color-scheme" content="dark">
<title>Ontology · BeatScript docs</title>
<meta name="description" content="The BeatScript ontology: genres, grooves, Euclidean rhythms, song structures, the language's terms and the engine concepts, as Turtle, JSON-LD, SPARQL and MCP. Derived from the bitwrap ontology. CC0.">
<link rel="canonical" href="{p}/ontology/">
<link rel="alternate" type="text/turtle" href="/ontology/ns.ttl">
<link rel="alternate" type="application/ld+json" href="/ontology/ns.jsonld">
<link rel="alternate" type="application/n-triples" href="/ontology/ns.nt">
<link rel="alternate" type="application/rdf+xml" href="/ontology/ns.rdf">
<link rel="icon" type="image/svg+xml" href="/favicon.svg">
<link rel="stylesheet" href="/docs/assets/docs.css">
<link rel="stylesheet" href="/ontology/assets/ontology.css">
</head>
<body class="ontology">
<a class="skip" href="#main">Skip to the content</a>
<header class="top">
  <a class="brand" href="/" aria-label="BeatScript editor"><span class="brand-mark" aria-hidden="true"></span><span class="brand-name">BeatScript</span></a>
  <nav class="crumbs" aria-label="Docs">
    <a href="/docs/">Docs</a>
    <a href="/docs/cheatsheet">Cheat sheet</a>
    <a href="/docs/language">Language reference</a>
    <a href="/docs/api">API reference</a>
    <a href="/ontology/" aria-current="page">Ontology</a>
  </nav>
  <span class="spacer"></span>
  <a class="tb" href="/" title="Back to the editor"><span class="gl-ic" aria-hidden="true">←</span><span class="lbl">Editor</span></a>
</header>
<div class="layout">
<main id="main" class="doc">
{body}
</main>
</div>
{footer}</body>
</html>
"##,
        footer = beatscript_serve::rights::FOOTER,
    )
}

#[cfg(test)]
mod tests;
