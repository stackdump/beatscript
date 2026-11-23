//! The lineage routes:
//!
//! - `GET /p/<cid>/tree`: the family tree, an HTML page (a browser) or the JSON-LD document (`Accept:
//!   application/ld+json` or `application/json`); `/p/<cid>/tree.jsonld` is always the document.
//! - `GET /p/<a>/compare/<b>`: the two programs and what changed from a to b; `.jsonld` on `<b>` for the document.
//! - `GET /api/lineage/<cid>`: the tree's document as JSON (what the MCP `lineage` tool returns).
//!
//! A CID that is unknown and one that is private get the same 404, so the answer does not tell a private program
//! exists.
//!
//! The rename: the tree of a BeatScript 1 program whose BeatScript 2 twin may be shown answers 308 at the twin's tree
//! ([`renamed_redirect`]), and a compare of two such programs at their twins' (not a compare of a program with its own twin).
//! `/api/lineage/<cid>` answers for the CID it names (its root says `renamedTo`).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::Value;
use tokio::sync::Semaphore;

use super::{page, Lineage};

pub struct Routes {
    pub lineage: Arc<Lineage>,
    /// The site's origin, no trailing slash (`https://beatscript.app`).
    pub origin: String,
    /// Tree builds at once (each compiles programs on a blocking thread).
    slots: Semaphore,
}

type St = State<Arc<Routes>>;

const CACHE: &str = "public, max-age=60";

pub fn router(lineage: Arc<Lineage>, origin: &str) -> Router {
    let st = Arc::new(Routes { lineage, origin: origin.trim_end_matches('/').to_string(), slots: Semaphore::new(2) });
    Router::new()
        .route("/p/{cid}/tree", get(tree))
        .route("/p/{cid}/tree.jsonld", get(tree_ld))
        .route("/p/{a}/compare/{b}", get(compare))
        .route("/api/lineage/{cid}", get(api_tree))
        .with_state(st)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Want {
    Html,
    Ld,
    Json,
}

/// The format `Accept` names first (by q, then order); anything else is the page.
pub fn negotiate(h: &HeaderMap) -> Want {
    let a = h.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let mut best: Option<(f32, usize, Want)> = None;
    for (i, part) in a.split(',').enumerate() {
        let mut it = part.split(';');
        let ty = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it.filter_map(|p| p.trim().strip_prefix("q=")).find_map(|q| q.trim().parse::<f32>().ok()).unwrap_or(1.0);
        let w = match ty.as_str() {
            "text/html" | "application/xhtml+xml" => Want::Html,
            "application/ld+json" => Want::Ld,
            "application/json" => Want::Json,
            _ => continue,
        };
        if q > 0.0 && best.is_none_or(|(bq, bi, _)| q > bq || (q == bq && i < bi)) {
            best = Some((q, i, w));
        }
    }
    best.map_or(Want::Html, |b| b.2)
}

fn respond(method: &Method, status: StatusCode, ctype: &'static str, body: String, vary: bool) -> Response<Body> {
    let n = body.len();
    let mut r = Response::new(if method == Method::HEAD { Body::empty() } else { Body::from(body) });
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(n));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if status == StatusCode::OK { CACHE } else { "no-cache" }));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    if ctype.starts_with("application/") {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    }
    if vary {
        h.insert(header::VARY, HeaderValue::from_static("Accept"));
    }
    r
}

fn ld_body(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

fn not_found(st: &Routes, method: &Method, want: Want) -> Response<Body> {
    match want {
        Want::Html => respond(method, StatusCode::NOT_FOUND, "text/html; charset=utf-8", page::missing_page(&st.origin), false),
        _ => respond(method, StatusCode::NOT_FOUND, "application/json", "{\"error\":{\"code\":\"not_found\",\"message\":\"no public program with that CID is stored here\"}}\n".into(), false),
    }
}

fn busy(method: &Method) -> Response<Body> {
    let mut r = respond(method, StatusCode::SERVICE_UNAVAILABLE, "application/json", "{\"error\":{\"code\":\"busy\",\"message\":\"busy; try again in a moment\",\"retry_after\":2}}\n".into(), false);
    r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    r
}

/// Run `f` on a blocking thread, at most `slots` at once (waiting up to 10 s for one).
async fn blocking<T: Send + 'static>(st: &Arc<Routes>, f: impl FnOnce(&Lineage) -> T + Send + 'static) -> Option<T> {
    let _p = tokio::time::timeout(std::time::Duration::from_secs(10), st.slots.acquire()).await.ok()?.ok()?;
    let l = st.lineage.clone();
    tokio::task::spawn_blocking(move || f(&l)).await.ok()
}

async fn serve_tree(st: Arc<Routes>, method: Method, cid: String, want: Want, vary: bool) -> Response<Body> {
    if !super::is_cid(&cid) {
        return not_found(&st, &method, want);
    }
    let Some(t) = blocking(&st, move |l| l.tree(&cid)).await else { return busy(&method) };
    let Some(t) = t else { return not_found(&st, &method, want) };
    let ld = t.json_ld(&st.origin);
    match want {
        Want::Html => respond(&method, StatusCode::OK, "text/html; charset=utf-8", page::tree_page(&st.origin, &t, &ld), vary),
        Want::Ld => respond(&method, StatusCode::OK, "application/ld+json; charset=utf-8", ld_body(&ld), vary),
        Want::Json => respond(&method, StatusCode::OK, "application/json; charset=utf-8", ld_body(&ld), vary),
    }
}

/// A `308 Permanent Redirect` from the address of a BeatScript 1 program to the same address with its BeatScript 2 twin's CID
/// (`path` with `from` replaced by `to`, the query kept): the store is append-only, so a twin once stored stays.
pub fn renamed_redirect(method: &Method, path: &str, query: Option<&str>, from: &str, to: &str) -> Response<Body> {
    permanent_redirect(method, &path.replacen(from, to, 1), query)
}

/// `308 Permanent Redirect` to `path` (the query kept), with Go's `http.Redirect` body.
pub fn permanent_redirect(method: &Method, path: &str, query: Option<&str>) -> Response<Body> {
    let mut loc = path.to_string();
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        loc = format!("{loc}?{q}");
    }
    let body = if method == Method::GET { format!("<a href=\"{}\">Permanent Redirect</a>.\n\n", super::page::esc(&loc)) } else { String::new() };
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = StatusCode::PERMANENT_REDIRECT;
    let h = r.headers_mut();
    h.insert(header::LOCATION, HeaderValue::from_str(&loc).unwrap_or(HeaderValue::from_static("/")));
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
    r
}

/// The twin a BeatScript 1 program's address redirects to, asked off the async threads.
async fn twin_of(st: &Arc<Routes>, cid: &str) -> Option<String> {
    if !super::is_cid(cid) {
        return None;
    }
    let c = cid.to_string();
    blocking(st, move |l| l.renamed_to(&c)).await.flatten()
}

async fn tree(State(st): St, Path(cid): Path<String>, method: Method, h: HeaderMap, uri: axum::http::Uri) -> Response<Body> {
    if let Some(to) = twin_of(&st, &cid).await {
        return renamed_redirect(&method, uri.path(), uri.query(), &cid, &to);
    }
    serve_tree(st, method, cid, negotiate(&h), true).await
}

async fn tree_ld(State(st): St, Path(cid): Path<String>, method: Method, uri: axum::http::Uri) -> Response<Body> {
    if let Some(to) = twin_of(&st, &cid).await {
        return renamed_redirect(&method, uri.path(), uri.query(), &cid, &to);
    }
    serve_tree(st, method, cid, Want::Ld, false).await
}

async fn api_tree(State(st): St, Path(cid): Path<String>, method: Method) -> Response<Body> {
    serve_tree(st, method, cid, Want::Json, false).await
}

async fn compare(State(st): St, Path((a, b)): Path<(String, String)>, method: Method, h: HeaderMap, uri: axum::http::Uri) -> Response<Body> {
    let (b, want, vary) = match b.strip_suffix(".jsonld") {
        Some(b) => (b.to_string(), Want::Ld, false),
        None => (b, negotiate(&h), true),
    };
    if !super::is_cid(&a) || !super::is_cid(&b) {
        return not_found(&st, &method, want);
    }
    // two old programs compare as their twins; a program and its own twin (the renamed edge's compare) stay as they are
    let (ta, tb) = (twin_of(&st, &a).await, twin_of(&st, &b).await);
    if ta.as_deref() != Some(b.as_str()) && tb.as_deref() != Some(a.as_str()) && (ta.is_some() || tb.is_some()) {
        let (na, nb) = (ta.unwrap_or_else(|| a.clone()), tb.unwrap_or_else(|| b.clone()));
        let suffix = if want == Want::Ld && !vary { ".jsonld" } else { "" };
        let path = format!("/p/{na}/compare/{nb}{suffix}");
        return permanent_redirect(&method, &path, uri.query());
    }
    let Some(r) = blocking(&st, move |l| l.compare(&a, &b)).await else { return busy(&method) };
    let Some((na, nb, c)) = r else { return not_found(&st, &method, want) };
    let ld = super::compare_json_ld(&st.origin, &na, &nb, c.as_ref());
    match want {
        Want::Html => respond(&method, StatusCode::OK, "text/html; charset=utf-8", page::compare_page(&st.origin, &na, &nb, c.as_ref(), &ld), vary),
        Want::Ld => respond(&method, StatusCode::OK, "application/ld+json; charset=utf-8", ld_body(&ld), vary),
        Want::Json => respond(&method, StatusCode::OK, "application/json; charset=utf-8", ld_body(&ld), vary),
    }
}
