//! The routes. All JSON, all `Cache-Control: no-store`, all for the signed-in account except the share link.
//!
//! ```text
//! GET    /api/playlists                         mine: {account, playlists:[…], limits}
//! POST   /api/playlists                         create {name, items?, visibility?}            -> 201
//! GET    /api/playlists/{id}                    one (ETag "<version>", If-None-Match -> 304)
//! PUT    /api/playlists/{id}                    replace {name, items, visibility?} (sync)
//! PATCH  /api/playlists/{id}                    rename / visibility {name?, visibility?}
//! DELETE /api/playlists/{id}                    delete
//! POST   /api/playlists/{id}/items              add {cid, title?, note?, id?, position?}
//! DELETE /api/playlists/{id}/items/{item}       remove one item (by item id: a CID may be there twice)
//! POST   /api/playlists/{id}/items/{item}/move  move {to: index}
//! POST   /api/playlists/{id}/share              make (or keep; {rotate:true} replaces) the link token
//! DELETE /api/playlists/{id}/share              revoke it (visibility private)
//! GET    /api/playlists/p/{token}               the shared list, read-only, no account: {name, count, items:[{cid,title?}]}
//! ```
//!
//! Every write takes an optional `If-Match: "<version>"` (or `*`); a stale one is 409 `conflict` with the server's
//! copy as `playlist`. Every answer to a playlist carries its `ETag`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Map, Value};

use super::auth::{cookie_value, cross_site, Caller, COOKIE_NAME};
use super::model::{self, clean_line, now_rfc3339, rand_b64, valid_cid, valid_item_id, valid_playlist_id, valid_share_token, valid_time, Item, Playlist, MAX_NAME, MAX_NOTE, MAX_TITLE, VISIBILITIES};
use super::{client_ip, go_json, ApiError, PeerAddr, Playlists};

type St = Arc<Playlists>;

pub fn router(pl: St) -> Router {
    Router::new()
        .route("/api/playlists", get(list).post(create))
        .route("/api/playlists/p/{token}", get(resolve).options(resolve_options))
        .route("/api/playlists/{id}", get(get_one).put(replace).patch(patch).delete(delete))
        .route("/api/playlists/{id}/items", post(add_item))
        .route("/api/playlists/{id}/items/{item}", axum::routing::delete(remove_item))
        .route("/api/playlists/{id}/items/{item}/move", post(move_item))
        .route("/api/playlists/{id}/share", post(share).delete(unshare))
        .with_state(pl)
}

// ---------- responses ----------

fn finish(mut r: axum::http::response::Builder) -> axum::http::response::Builder {
    r = r.header(header::CACHE_CONTROL, "no-store").header(header::VARY, "Cookie, Authorization");
    r
}

fn json_resp(status: u16, v: &Value, etag: Option<&str>) -> Response<Body> {
    let mut r = finish(Response::builder().status(status)).header(header::CONTENT_TYPE, "application/json; charset=utf-8");
    if let Some(e) = etag {
        r = r.header(header::ETAG, format!("\"{e}\""));
    }
    r.body(Body::from(go_json(v) + "\n")).unwrap()
}

fn err_resp(e: ApiError) -> Response<Body> {
    let mut o = Map::new();
    o.insert("code".into(), json!(e.code));
    o.insert("message".into(), json!(e.message));
    if e.retry_after > 0 {
        o.insert("retry_after".into(), json!(e.retry_after));
    }
    let mut top = Map::new();
    top.insert("error".into(), Value::Object(o));
    let mut etag = None;
    if let Some((k, v)) = e.extra {
        etag = v.get("version").and_then(|x| x.as_str()).map(str::to_string);
        top.insert(k, v);
    }
    let mut r = json_resp(e.status, &Value::Object(top), etag.as_deref());
    if e.retry_after > 0 {
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_str(&e.retry_after.to_string()).unwrap());
    }
    r
}

fn reply(r: Result<Response<Body>, ApiError>) -> Response<Body> {
    r.unwrap_or_else(err_resp)
}

/// A playlist as its owner sees it: the stored record plus `count` and, when shared, `share_url`.
fn view(pl: &Playlists, p: &Playlist) -> Value {
    let mut v = serde_json::to_value(p).unwrap();
    v["count"] = json!(p.items.len());
    if let (true, Some(t)) = (p.shared(), &p.share_token) {
        v["share_url"] = json!(format!("{}/#playlist={t}", pl.cfg.public_url.trim_end_matches('/')));
        v["share_api"] = json!(format!("{}/api/playlists/p/{t}", pl.cfg.public_url.trim_end_matches('/')));
    }
    v
}

fn one(pl: &Playlists, status: u16, p: &Playlist) -> Response<Body> {
    json_resp(status, &view(pl, p), Some(&p.version))
}

// ---------- the request ----------

struct Ctx {
    caller: Caller,
    body: bytes::Bytes,
    headers: HeaderMap,
}

/// Everything a playlist route does first, in this order: the per-address limit, the cross-site refusal (Go's
/// `crossSite`), the body (read before auth: a phone's signature covers it), the account side being configured, the
/// caller, and the per-account limits.
async fn begin(pl: &Playlists, req: Request, write: bool) -> Result<Ctx, ApiError> {
    let (parts, body) = req.into_parts();
    let h = parts.headers;
    let peer = parts.extensions.get::<PeerAddr>().map(|p| p.0);
    let ip = client_ip(peer, &h);
    Playlists::limit(&pl.ip, &ip, "requests from this address")?;
    let hv = |n: header::HeaderName| h.get(n).and_then(|v| v.to_str().ok());
    if cross_site(hv(header::ORIGIN), h.get("sec-fetch-site").and_then(|v| v.to_str().ok()), hv(header::HOST), pl.auth.hosts()) {
        return Err(ApiError::new(403, "cross_site", "this request came from another site; playlists are only for this site's own page and the apps"));
    }
    let body = to_bytes(body, pl.cfg.limits.max_account_bytes).await.map_err(|_| ApiError::new(413, "too_large", format!("request body over {} bytes", pl.cfg.limits.max_account_bytes)))?;
    if write && !body.is_empty() {
        let ct = hv(header::CONTENT_TYPE).unwrap_or("");
        if !ct.trim().to_ascii_lowercase().starts_with("application/json") {
            return Err(ApiError::new(415, "bad_request", "send the body as Content-Type: application/json"));
        }
    }
    if !pl.auth.ready() {
        return Err(ApiError::new(503, "unavailable", "accounts are not configured on this server yet").retry(60));
    }
    let cookies = h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok());
    let cookie = cookie_value(cookies, COOKIE_NAME);
    let caller = pl
        .auth
        .caller(cookie, hv(header::AUTHORIZATION), parts.method.as_str(), parts.uri.path(), &body)
        .map_err(|e| ApiError::new(e.status, e.code, e.message))?
        .ok_or_else(|| ApiError::new(401, "auth_required", "sign in to keep playlists on your account (they work in this browser without it)"))?;
    Playlists::limit(&pl.account, &caller.uid, "requests for this account")?;
    if write {
        Playlists::limit(&pl.writes, &caller.uid, "changes to playlists")?;
    }
    Ok(Ctx { caller, body, headers: h })
}

fn body_obj(b: &[u8], allowed: &[&str]) -> Result<Map<String, Value>, ApiError> {
    if b.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_slice(b).map_err(|e| ApiError::new(400, "bad_request", format!("body must be JSON: {e}")))?;
    let Value::Object(m) = v else { return Err(ApiError::new(400, "bad_request", "body must be a JSON object")) };
    for k in m.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(ApiError::new(400, "bad_request", format!("unknown field {k:?} (allowed: {})", allowed.join(", "))));
        }
    }
    Ok(m)
}

fn opt_str<'a>(m: &'a Map<String, Value>, k: &str) -> Result<Option<&'a str>, ApiError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ApiError::new(400, "bad_request", format!("{k} must be a string"))),
    }
}

fn name_of(m: &Map<String, Value>, required: bool) -> Result<Option<String>, ApiError> {
    match opt_str(m, "name")? {
        None if required => Err(ApiError::new(400, "bad_request", "name is required")),
        None => Ok(None),
        Some(s) => clean_line(s, MAX_NAME).map(Some).ok_or_else(|| ApiError::new(400, "bad_request", format!("name is one line of 1 to {MAX_NAME} characters"))),
    }
}

fn visibility_of(m: &Map<String, Value>) -> Result<Option<String>, ApiError> {
    match opt_str(m, "visibility")? {
        None => Ok(None),
        Some(v) if VISIBILITIES.contains(&v) => Ok(Some(v.to_string())),
        Some(_) => Err(ApiError::new(400, "bad_request", "visibility is private, link or public")),
    }
}

fn optional_line(m: &Map<String, Value>, k: &str, max: usize) -> Result<Option<String>, ApiError> {
    match opt_str(m, k)? {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => clean_line(s, max).map(Some).ok_or_else(|| ApiError::new(400, "bad_request", format!("{k} is one line of at most {max} characters"))),
    }
}

/// An item from a client: `{cid, title?, note?, id?, added_at?}`. A missing id is made; a missing or malformed
/// `added_at` is now.
fn item_of(v: &Value, now: &str, extra: &[&str]) -> Result<Item, ApiError> {
    let Value::Object(m) = v else { return Err(ApiError::new(400, "bad_request", "an item is a JSON object {cid, title?, note?}")) };
    // `renamed_from` is the server's (migrate-rename): a client that sends an item back as it got it keeps it
    let mut allowed = vec!["cid", "title", "note", "id", "added_at", "renamed_from"];
    allowed.extend_from_slice(extra);
    for k in m.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(ApiError::new(400, "bad_request", format!("unknown item field {k:?}")));
        }
    }
    let cid = opt_str(m, "cid")?.ok_or_else(|| ApiError::new(400, "bad_request", "an item needs a cid"))?;
    if !valid_cid(cid) {
        return Err(ApiError::new(400, "bad_request", "cid must be a program CID (z and base58)"));
    }
    let id = match opt_str(m, "id")? {
        Some(i) if valid_item_id(i) => i.to_string(),
        Some(_) => return Err(ApiError::new(400, "bad_request", "an item id is 8 to 40 of A-Z a-z 0-9 _ -")),
        None => format!("it_{}", rand_b64(9)),
    };
    let added_at = opt_str(m, "added_at")?.filter(|t| valid_time(t)).map(str::to_string).unwrap_or_else(|| now.to_string());
    Ok(Item { id, cid: cid.to_string(), title: optional_line(m, "title", MAX_TITLE)?, added_at, note: optional_line(m, "note", MAX_NOTE)?, renamed_from: opt_str(m, "renamed_from")?.filter(|c| valid_cid(c)).map(str::to_string) })
}

fn items_of(m: &Map<String, Value>, max: usize, now: &str) -> Result<Option<Vec<Item>>, ApiError> {
    let Some(v) = m.get("items") else { return Ok(None) };
    let Value::Array(a) = v else { return Err(ApiError::new(400, "bad_request", "items must be an array")) };
    if a.len() > max {
        return Err(ApiError::new(422, "limit", format!("at most {max} items in a playlist")));
    }
    let items = a.iter().map(|x| item_of(x, now, &[])).collect::<Result<Vec<_>, _>>()?;
    let mut ids = std::collections::HashSet::new();
    for it in &items {
        if !ids.insert(it.id.as_str()) {
            return Err(ApiError::new(400, "bad_request", format!("item id {} is there twice", it.id)));
        }
    }
    Ok(Some(items))
}

/// `If-Match`: absent is no condition; `*` any existing copy; else one of the listed (weak or strong) tags.
fn if_match_ok(h: &HeaderMap, version: &str) -> bool {
    let Some(v) = h.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) else { return true };
    v.split(',').map(str::trim).any(|t| t == "*" || t.trim_start_matches("W/").trim_matches('"') == version)
}

fn conflict(pl: &Playlists, p: &Playlist) -> ApiError {
    ApiError::new(409, "conflict", "this playlist changed since your copy (another device?): merge with the server's copy below and send again").with("playlist", view(pl, p))
}

fn not_found() -> ApiError {
    ApiError::new(404, "not_found", "no such playlist on your account")
}

fn path_id(id: &str) -> Result<(), ApiError> {
    if valid_playlist_id(id) {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// Load, check If-Match, change one playlist with `f`, store, answer it.
async fn change(pl: St, req: Request, id: String, allowed: &'static [&'static str], f: impl FnOnce(&Playlists, &mut Playlist, &Map<String, Value>, &str) -> Result<(), ApiError> + Send + 'static) -> Result<Response<Body>, ApiError> {
    path_id(&id)?;
    let ctx = begin(&pl, req, true).await?;
    let m = body_obj(&ctx.body, allowed)?;
    let pl2 = pl.clone();
    let p = tokio::task::spawn_blocking(move || {
        pl2.store.update(&ctx.caller.uid, |file| {
            let p = file.playlists.iter_mut().find(|p| p.id == id).ok_or_else(not_found)?;
            if !if_match_ok(&ctx.headers, &p.version) {
                return Err(conflict(&pl2, p));
            }
            let before = p.compute_version();
            let now = now_rfc3339();
            f(&pl2, p, &m, &now)?;
            if p.compute_version() != before {
                p.updated = now;
            }
            Ok(p.clone())
        })
    })
    .await
    .map_err(|_| ApiError::new(500, "internal", "the change failed"))??;
    let mut p = p;
    p.seal();
    Ok(one(&pl, 200, &p))
}

fn set_visibility(p: &mut Playlist, v: &str) {
    p.visibility = v.to_string();
    if v == "private" {
        p.share_token = None;
    } else if p.share_token.is_none() {
        p.share_token = Some(rand_b64(16));
    }
}

// ---------- handlers ----------

async fn list(State(pl): State<St>, req: Request) -> Response<Body> {
    reply(async {
        let ctx = begin(&pl, req, false).await?;
        let pl2 = pl.clone();
        let uid = ctx.caller.uid.clone();
        let f = tokio::task::spawn_blocking(move || pl2.store.read(&uid)).await.map_err(|_| ApiError::new(500, "internal", "read failed"))??;
        let l = pl.store.limits();
        Ok(json_resp(200, &json!({
            "account": ctx.caller.uid,
            "playlists": f.playlists.iter().map(|p| view(&pl, p)).collect::<Vec<_>>(),
            "limits": {"max_playlists": l.max_playlists, "max_items": l.max_items, "max_name": MAX_NAME, "max_title": MAX_TITLE, "max_note": MAX_NOTE},
        }), None))
    }.await)
}

async fn create(State(pl): State<St>, req: Request) -> Response<Body> {
    reply(async {
        let ctx = begin(&pl, req, true).await?;
        let m = body_obj(&ctx.body, &["name", "items", "visibility"])?;
        let now = now_rfc3339();
        let name = name_of(&m, true)?.unwrap();
        let items = items_of(&m, pl.cfg.limits.max_items, &now)?.unwrap_or_default();
        for it in &items {
            pl.cfg.gate.admit(&it.cid)?;
        }
        let mut p = Playlist { id: format!("pl_{}", rand_b64(12)), owner: ctx.caller.uid.clone(), name, items, created: now.clone(), updated: now, visibility: "private".into(), share_token: None, version: String::new() };
        set_visibility(&mut p, visibility_of(&m)?.as_deref().unwrap_or("private"));
        p.seal();
        let pl2 = pl.clone();
        let p2 = p.clone();
        tokio::task::spawn_blocking(move || pl2.store.update(&ctx.caller.uid, |f| {
            f.playlists.push(p2);
            Ok(())
        }))
        .await
        .map_err(|_| ApiError::new(500, "internal", "the change failed"))??;
        let mut r = one(&pl, 201, &p);
        r.headers_mut().insert(header::LOCATION, HeaderValue::from_str(&format!("/api/playlists/{}", p.id)).unwrap());
        Ok(r)
    }.await)
}

async fn get_one(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    reply(async {
        path_id(&id)?;
        let inm = req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
        let ctx = begin(&pl, req, false).await?;
        let pl2 = pl.clone();
        let f = tokio::task::spawn_blocking(move || pl2.store.read(&ctx.caller.uid)).await.map_err(|_| ApiError::new(500, "internal", "read failed"))??;
        let p = f.playlists.iter().find(|p| p.id == id).ok_or_else(not_found)?;
        if inm.is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/").trim_matches('"') == p.version)) {
            return Ok(finish(Response::builder().status(304)).header(header::ETAG, format!("\"{}\"", p.version)).body(Body::empty()).unwrap());
        }
        Ok(one(&pl, 200, p))
    }.await)
}

async fn replace(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let max = pl.cfg.limits.max_items;
    reply(change(pl, req, id, &["name", "items", "visibility", "version"], move |pl, p, m, now| {
        p.name = name_of(m, true)?.unwrap();
        p.items = items_of(m, max, now)?.ok_or_else(|| ApiError::new(400, "bad_request", "items is required (send [] for none)"))?;
        for it in &p.items {
            pl.cfg.gate.admit(&it.cid)?;
        }
        if let Some(v) = visibility_of(m)? {
            set_visibility(p, &v);
        }
        Ok(())
    }).await)
}

async fn patch(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    reply(change(pl, req, id, &["name", "visibility"], |_, p, m, _| {
        if let Some(n) = name_of(m, false)? {
            p.name = n;
        }
        if let Some(v) = visibility_of(m)? {
            set_visibility(p, &v);
        }
        Ok(())
    }).await)
}

async fn delete(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    reply(async {
        path_id(&id)?;
        let ctx = begin(&pl, req, true).await?;
        let pl2 = pl.clone();
        let id2 = id.clone();
        tokio::task::spawn_blocking(move || pl2.store.update(&ctx.caller.uid, |f| {
            let i = f.playlists.iter().position(|p| p.id == id2).ok_or_else(not_found)?;
            if !if_match_ok(&ctx.headers, &f.playlists[i].version) {
                return Err(conflict(&pl2, &f.playlists[i]));
            }
            f.playlists.remove(i);
            Ok(())
        }))
        .await
        .map_err(|_| ApiError::new(500, "internal", "the change failed"))??;
        Ok(json_resp(200, &json!({"deleted": id}), None))
    }.await)
}

async fn add_item(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let max = pl.cfg.limits.max_items;
    reply(change(pl, req, id, &["cid", "title", "note", "id", "added_at", "position"], move |pl, p, m, now| {
        if p.items.len() >= max {
            return Err(ApiError::new(422, "limit", format!("at most {max} items in a playlist")));
        }
        let it = item_of(&Value::Object(m.clone()), now, &["position"])?;
        pl.cfg.gate.admit(&it.cid)?;
        if p.items.iter().any(|x| x.id == it.id) {
            return Err(ApiError::new(409, "duplicate_item", format!("item id {} is already in this playlist", it.id)));
        }
        let at = match m.get("position") {
            None | Some(Value::Null) => p.items.len(),
            Some(v) => v.as_u64().ok_or_else(|| ApiError::new(400, "bad_request", "position is a whole number"))?.min(p.items.len() as u64) as usize,
        };
        p.items.insert(at, it);
        Ok(())
    }).await)
}

async fn remove_item(State(pl): State<St>, Path((id, item)): Path<(String, String)>, req: Request) -> Response<Body> {
    reply(change(pl, req, id, &[], move |_, p, _, _| {
        let i = p.items.iter().position(|x| x.id == item).ok_or_else(|| ApiError::new(404, "not_found", "no such item in this playlist"))?;
        p.items.remove(i);
        Ok(())
    }).await)
}

async fn move_item(State(pl): State<St>, Path((id, item)): Path<(String, String)>, req: Request) -> Response<Body> {
    reply(change(pl, req, id, &["to"], move |_, p, m, _| {
        let to = m.get("to").and_then(|v| v.as_u64()).ok_or_else(|| ApiError::new(400, "bad_request", "to is the item's new index (a whole number)"))?;
        let i = p.items.iter().position(|x| x.id == item).ok_or_else(|| ApiError::new(404, "not_found", "no such item in this playlist"))?;
        let it = p.items.remove(i);
        let to = (to as usize).min(p.items.len());
        p.items.insert(to, it);
        Ok(())
    }).await)
}

async fn share(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    reply(change(pl, req, id, &["rotate"], |_, p, m, _| {
        if m.get("rotate").and_then(|v| v.as_bool()) == Some(true) {
            p.share_token = None;
        }
        let v = if p.visibility == "private" { "link".to_string() } else { p.visibility.clone() };
        set_visibility(p, &v);
        Ok(())
    }).await)
}

async fn unshare(State(pl): State<St>, Path(id): Path<String>, req: Request) -> Response<Body> {
    reply(change(pl, req, id, &[], |_, p, _, _| {
        set_visibility(p, "private");
        Ok(())
    }).await)
}

fn cors(mut r: Response<Body>) -> Response<Body> {
    let h = r.headers_mut();
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    r
}

/// The shared list: name and the items' CIDs and titles. Nothing about the owner, their other playlists, the item
/// ids or notes, or the playlist's id. No credentials are read (CORS `*`), so another site's player may open it.
async fn resolve(State(pl): State<St>, Path(token): Path<String>, req: Request) -> Response<Body> {
    let peer = req.extensions().get::<PeerAddr>().map(|p| p.0);
    let ip = client_ip(peer, req.headers());
    let method = req.method().clone();
    drop(req);
    let r = async {
        Playlists::limit(&pl.resolve, &ip, "playlist link lookups from this address")?;
        if method != Method::GET && method != Method::HEAD {
            return Err(ApiError::new(405, "bad_request", "GET only"));
        }
        let gone = || ApiError::new(404, "not_found", "this playlist link is not shared (any more)");
        if !valid_share_token(&token) {
            return Err(gone());
        }
        let pl2 = pl.clone();
        let p = tokio::task::spawn_blocking(move || pl2.store.resolve(&token)).await.map_err(|_| ApiError::new(500, "internal", "read failed"))?.ok_or_else(gone)?;
        let items: Vec<Value> = p.items.iter().map(|it| match &it.title {
            Some(t) => json!({"cid": it.cid, "title": t}),
            None => json!({"cid": it.cid}),
        }).collect();
        Ok(json_resp(200, &json!({"name": p.name, "count": items.len(), "items": items, "license_note": "each item is a public program (CC0-1.0) addressed by its CID: GET /api/programs/<cid>"}), Some(&p.version)))
    }.await;
    cors(reply(r))
}

async fn resolve_options() -> Response<Body> {
    let mut r = Response::builder().status(204).body(Body::empty()).unwrap();
    let h = r.headers_mut();
    h.insert("access-control-allow-methods", HeaderValue::from_static("GET, HEAD, OPTIONS"));
    h.insert("access-control-max-age", HeaderValue::from_static("86400"));
    cors(r)
}

/// For main.rs: the limit model in one place (also what the tests assert against).
pub fn default_limits() -> model::Limits {
    model::Limits::default()
}
