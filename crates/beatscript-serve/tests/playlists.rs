//! /api/playlists end to end through the router, against a data directory laid out as the Go server lays it out
//! (session.secret, tokens.json, devices.json, site.json). `cargo test -p beatscript-serve --test playlists`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use beatscript_serve::playlists::auth::Auth;
use beatscript_serve::playlists::model::Limits;
use beatscript_serve::playlists::{router, Config, PeerAddr, Playlists};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const SECRET: [u8; 32] = [42; 32];
const SITE: &str = "siteSITEsiteSITEsite01";
const A: &str = "ed25519:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "eip155:0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN_A: &str = "btw_tokenAtokenAtokenAtokenAtokenAtokenAtokenA";
// two stored CC0 programs (the storage gate: an item names an example or a stored program it admits)
const CID1: &str = "z4EBG9j6QXgY1YzgWMp8tdV3cWznt6reFvfEE5WVF5pA5Nm9G3R";
const CID2: &str = "z4EBG9j2S47J8ducGMzvAUXbt7cL8JNj8r5WiCi1DxupYtRtvK4";
const PROGRAMS: [(&str, &str); 2] = [(CID1, "tempo 120\ndeck A: kick x ~ x ~\n"), (CID2, "tempo 128\ndeck A: kick x x x x\n")];
/// A stored file the gate refuses today (an all-rights-reserved licence): an item may not name it.
const CID_ARR: &str = "z4EBG9jGQynCTG74PKpp7FVKPg6rKBuA6JzPSYrD7ShfuFnbQ6G";

struct T {
    _dir: tempfile::TempDir,
    dir: std::path::PathBuf,
    app: axum::Router,
    phone: SigningKey,
}

fn go_data(dir: &Path, with_secret: bool) -> SigningKey {
    if with_secret {
        std::fs::write(dir.join("session.secret"), hex::encode(SECRET) + "\n").unwrap();
    }
    std::fs::write(dir.join("site.json"), json!({"site_id": SITE, "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    std::fs::write(dir.join("tokens.json"), json!([{"hash": hex::encode(Sha256::digest(TOKEN_A.as_bytes())), "user": A, "created": "2026-11-19T00:00:00Z"}]).to_string()).unwrap();
    let programs = dir.join("programs");
    std::fs::create_dir_all(&programs).unwrap();
    for (cid, text) in PROGRAMS {
        std::fs::write(programs.join(format!("{cid}.json")), json!({"cid": cid, "text": text, "meta": {"license": "CC0-1.0", "visibility": "public"}}).to_string()).unwrap();
    }
    let phone = SigningKey::from_bytes(&[1u8; 32]);
    let key = B64.encode(phone.verifying_key().to_bytes());
    std::fs::write(dir.join("devices.json"), json!([{"id": "devPHONEdevPHONEdevPHO", "owner": A, "name": "Pixel 9", "key": key, "paired": "2026-11-19T00:00:00Z"}]).to_string()).unwrap();
    phone
}

fn setup_with(limits: Limits, with_secret: bool) -> T {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_path_buf();
    let phone = go_data(&dir, with_secret);
    let app = make_app(&dir, limits);
    T { _dir: d, dir, app, phone }
}

fn make_app(dir: &Path, limits: Limits) -> axum::Router {
    let cfg = Config { data: dir.to_path_buf(), secret_env: None, public_url: "https://beatscript.app".into(), hosts: Config::hosts_from("https://beatscript.app", "", "btw.bitwrap.io"), limits, gate: beatscript_serve::playlists::ItemGate::new(dir.join("programs"), []) };
    let (pl, _w) = Playlists::open(cfg).unwrap();
    router(pl)
}

fn setup() -> T {
    setup_with(Limits::default(), true)
}

#[derive(Clone)]
enum Cred {
    None,
    Cookie(String),
    Bearer(&'static str),
    Phone,
}

fn cookie(uid: &str) -> Cred {
    Cred::Cookie(Auth::make_cookie_for_test(&SECRET, uid, 4_000_000_000))
}

struct Resp {
    status: StatusCode,
    etag: Option<String>,
    headers: axum::http::HeaderMap,
    json: Value,
}

impl T {
    async fn call(&self, method: &str, path: &str, cred: Cred, body: Option<Value>, extra: &[(&str, &str)]) -> Resp {
        let body_bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        let mut rb = Request::builder().method(method).uri(path).header("host", "127.0.0.1:18098");
        if !body_bytes.is_empty() {
            rb = rb.header("content-type", "application/json");
        }
        match &cred {
            Cred::None => {}
            Cred::Cookie(c) => rb = rb.header("cookie", format!("other=1; btw_session={c}")),
            Cred::Bearer(t) => rb = rb.header("authorization", format!("Bearer {t}")),
            Cred::Phone => {
                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
                let msg = Auth::request_message2(SITE, "devPHONEdevPHONEdevPHO", method, path.split('?').next().unwrap(), ts, &body_bytes);
                let sig = B64.encode(self.phone.sign(msg.as_bytes()).to_bytes());
                rb = rb.header("authorization", format!("BtwDevice devPHONEdevPHONEdevPHO.{ts}.{sig}"));
            }
        }
        for (k, v) in extra {
            rb = rb.header(*k, *v);
        }
        let mut req = rb.body(Body::from(body_bytes)).unwrap();
        req.extensions_mut().insert(PeerAddr("127.0.0.1:5555".parse::<SocketAddr>().unwrap()));
        let r = self.app.clone().oneshot(req).await.unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let etag = headers.get("etag").map(|v| v.to_str().unwrap().to_string());
        let b = to_bytes(r.into_body(), 1 << 22).await.unwrap();
        let json = if b.is_empty() { Value::Null } else { serde_json::from_slice(&b).unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&b))) };
        Resp { status, etag, headers, json }
    }

    async fn create(&self, cred: Cred, name: &str) -> Value {
        let r = self.call("POST", "/api/playlists", cred, Some(json!({"name": name})), &[]).await;
        assert_eq!(r.status, 201, "{}", r.json);
        r.json
    }
}

// ---------------- auth ----------------

#[tokio::test]
async fn auth_refused() {
    let t = setup();
    let r = t.call("GET", "/api/playlists", Cred::None, None, &[]).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.json["error"]["code"], "auth_required");
    let forged = Cred::Cookie(Auth::make_cookie_for_test(&[9u8; 32], A, 4_000_000_000));
    assert_eq!(t.call("GET", "/api/playlists", forged, None, &[]).await.status, 401);
    let expired = Cred::Cookie(Auth::make_cookie_for_test(&SECRET, A, 1_000_000_000));
    let r = t.call("GET", "/api/playlists", expired, None, &[]).await;
    assert_eq!(r.status, 401);
    assert!(r.json["error"]["message"].as_str().unwrap().contains("expired"));
    assert_eq!(t.call("GET", "/api/playlists", Cred::Bearer("btw_notatokenofoursnotatokenofours0000000000"), None, &[]).await.status, 401);
    // a phone whose record the Go server removed (unpaired) is told so: 410
    std::fs::write(t.dir.join("devices.json"), "[]").unwrap();
    let r = t.call("GET", "/api/playlists", Cred::Phone, None, &[]).await;
    assert_eq!(r.status, 410);
    assert_eq!(r.json["error"]["code"], "unpaired");
}

#[tokio::test]
async fn no_session_key_is_503_and_none_is_made() {
    let t = setup_with(Limits::default(), false);
    let r = t.call("GET", "/api/playlists", cookie(A), None, &[]).await;
    assert_eq!(r.status, 503);
    assert!(!t.dir.join("session.secret").exists());
}

#[tokio::test]
async fn cookie_token_and_phone_are_one_account() {
    let t = setup();
    let p = t.create(cookie(A), "from the web").await;
    let id = p["id"].as_str().unwrap();
    for cred in [cookie(A), Cred::Bearer(TOKEN_A), Cred::Phone] {
        let r = t.call("GET", "/api/playlists", cred.clone(), None, &[]).await;
        assert_eq!(r.status, 200, "{}", r.json);
        assert_eq!(r.json["account"], A);
        assert_eq!(r.json["playlists"][0]["id"], id);
    }
    // the phone adds a track it composed: signed over the body
    let r = t.call("POST", &format!("/api/playlists/{id}/items"), Cred::Phone, Some(json!({"cid": CID1, "title": "phone take"})), &[]).await;
    assert_eq!(r.status, 200, "{}", r.json);
    assert_eq!(r.json["items"][0]["title"], "phone take");
    let r = t.call("GET", &format!("/api/playlists/{id}"), Cred::Bearer(TOKEN_A), None, &[]).await;
    assert_eq!(r.json["count"], 1);
}

#[tokio::test]
async fn phone_signature_must_cover_this_request() {
    let t = setup();
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let msg = Auth::request_message2(SITE, "devPHONEdevPHONEdevPHO", "GET", "/api/playlists/other", ts, b"");
    let sig = B64.encode(t.phone.sign(msg.as_bytes()).to_bytes());
    let r = t.call("GET", "/api/playlists", Cred::None, None, &[("authorization", &format!("BtwDevice devPHONEdevPHONEdevPHO.{ts}.{sig}"))]).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.json["error"]["code"], "device_auth");
}

#[tokio::test]
async fn cross_site_and_content_type() {
    let t = setup();
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "x"})), &[("origin", "https://evil.example")]).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json["error"]["code"], "cross_site");
    let r = t.call("GET", "/api/playlists", cookie(A), None, &[("sec-fetch-site", "cross-site")]).await;
    assert_eq!(r.status, 403);
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "x"})), &[("origin", "https://beatscript.app")]).await;
    assert_eq!(r.status, 201);
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "x"})), &[("origin", "https://btw.bitwrap.io")]).await;
    assert_eq!(r.status, 201, "a legacy host is the same site");
    // a text/plain body (what a cross-site form could send without a preflight) is refused
    let mut req = Request::builder().method("POST").uri("/api/playlists").header("content-type", "text/plain");
    if let Cred::Cookie(c) = cookie(A) {
        req = req.header("cookie", format!("btw_session={c}"));
    }
    let r = t.app.clone().oneshot(req.body(Body::from(r#"{"name":"x"}"#)).unwrap()).await.unwrap();
    assert_eq!(r.status(), 415);
}

// ---------------- ownership ----------------

#[tokio::test]
async fn ownership_isolation() {
    let t = setup();
    let p = t.create(cookie(A), "A's private list").await;
    let id = p["id"].as_str().unwrap().to_string();
    assert_eq!(p["visibility"], "private");
    let b = cookie(B);
    assert_eq!(t.call("GET", &format!("/api/playlists/{id}"), b.clone(), None, &[]).await.status, 404);
    assert_eq!(t.call("GET", "/api/playlists", b.clone(), None, &[]).await.json["playlists"], json!([]));
    assert_eq!(t.call("PUT", &format!("/api/playlists/{id}"), b.clone(), Some(json!({"name": "mine now", "items": []})), &[]).await.status, 404);
    assert_eq!(t.call("PATCH", &format!("/api/playlists/{id}"), b.clone(), Some(json!({"name": "mine now"})), &[]).await.status, 404);
    assert_eq!(t.call("POST", &format!("/api/playlists/{id}/items"), b.clone(), Some(json!({"cid": CID1})), &[]).await.status, 404);
    assert_eq!(t.call("POST", &format!("/api/playlists/{id}/share"), b.clone(), None, &[]).await.status, 404);
    assert_eq!(t.call("DELETE", &format!("/api/playlists/{id}"), b.clone(), None, &[]).await.status, 404);
    let r = t.call("GET", &format!("/api/playlists/{id}"), cookie(A), None, &[]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json["name"], "A's private list");
    // the same 404 for an id that never existed: nothing says the other one does
    let none = t.call("GET", "/api/playlists/pl_AAAAAAAAAAAAAAAA", b, None, &[]).await;
    assert_eq!(none.json["error"], t.call("GET", &format!("/api/playlists/{id}"), cookie(B), None, &[]).await.json["error"]);
}

// ---------------- items, order, duplicates ----------------

#[tokio::test]
async fn items_order_duplicates_move_remove() {
    let t = setup();
    let id = t.create(cookie(A), "set").await["id"].as_str().unwrap().to_string();
    let items_path = format!("/api/playlists/{id}/items");
    let add = |cid: &'static str, title: &'static str| t.call("POST", &items_path, cookie(A), Some(json!({"cid": cid, "title": title})), &[]);
    add(CID1, "one").await;
    add(CID2, "two").await;
    let r = add(CID1, "one again").await;
    let items = r.json["items"].as_array().unwrap().clone();
    assert_eq!(items.iter().map(|i| i["cid"].as_str().unwrap()).collect::<Vec<_>>(), [CID1, CID2, CID1], "the same CID may be there twice");
    let last = items[2]["id"].as_str().unwrap();
    let r = t.call("POST", &format!("/api/playlists/{id}/items/{last}/move"), cookie(A), Some(json!({"to": 0})), &[]).await;
    assert_eq!(r.json["items"][0]["title"], "one again");
    let first_one = items[0]["id"].as_str().unwrap();
    let r = t.call("DELETE", &format!("/api/playlists/{id}/items/{first_one}"), cookie(A), None, &[]).await;
    let titles: Vec<_> = r.json["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(titles, ["one again", "two"]);
    // bad input
    assert_eq!(t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": "not-a-cid"})), &[]).await.status, 400);
    let r = t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": CID1, "text": "program text"})), &[]).await;
    assert_eq!(r.status, 400, "an item never carries program text");
    assert_eq!(t.call("PATCH", &format!("/api/playlists/{id}"), cookie(A), Some(json!({"name": "  "})), &[]).await.status, 400);
    let r = t.call("PATCH", &format!("/api/playlists/{id}"), cookie(A), Some(json!({"name": "renamed"})), &[]).await;
    assert_eq!(r.json["name"], "renamed");
}

// ---------------- share links ----------------

#[tokio::test]
async fn share_link_is_read_only_and_narrow() {
    let t = setup();
    let id = t.create(cookie(A), "shared set").await["id"].as_str().unwrap().to_string();
    let other = t.create(cookie(A), "A's other secret list").await;
    t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": CID1, "title": "t1", "note": "private note"})), &[]).await;
    let s = t.call("POST", &format!("/api/playlists/{id}/share"), cookie(A), None, &[]).await;
    assert_eq!(s.status, 200);
    assert_eq!(s.json["visibility"], "link");
    let token = s.json["share_token"].as_str().unwrap().to_string();
    assert_eq!(s.json["share_url"], format!("https://beatscript.app/#playlist={token}"));
    let r = t.call("GET", &format!("/api/playlists/p/{token}"), Cred::None, None, &[]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers.get("access-control-allow-origin").unwrap(), "*");
    assert_eq!(r.json["name"], "shared set");
    assert_eq!(r.json["items"], json!([{"cid": CID1, "title": "t1"}]));
    let text = r.json.to_string();
    for leak in [A, "private note", other["id"].as_str().unwrap(), "secret", &id, "owner", "share_token"] {
        assert!(!text.contains(leak), "the share link leaks {leak:?}: {text}");
    }
    // read-only: the token is no credential for anything else
    assert_eq!(t.call("POST", &format!("/api/playlists/p/{token}"), Cred::None, Some(json!({"cid": CID2})), &[]).await.status, 405);
    assert_eq!(t.call("GET", &format!("/api/playlists/{id}"), Cred::Bearer("beatscript_x"), None, &[]).await.status, 401);
    // sharing again keeps the link; rotate replaces it; unshare ends it
    let again = t.call("POST", &format!("/api/playlists/{id}/share"), cookie(A), None, &[]).await;
    assert_eq!(again.json["share_token"], token.as_str());
    let rot = t.call("POST", &format!("/api/playlists/{id}/share"), cookie(A), Some(json!({"rotate": true})), &[]).await;
    let token2 = rot.json["share_token"].as_str().unwrap().to_string();
    assert_ne!(token2, token);
    assert_eq!(t.call("GET", &format!("/api/playlists/p/{token}"), Cred::None, None, &[]).await.status, 404);
    assert_eq!(t.call("GET", &format!("/api/playlists/p/{token2}"), Cred::None, None, &[]).await.status, 200);
    t.call("DELETE", &format!("/api/playlists/{id}/share"), cookie(A), None, &[]).await;
    assert_eq!(t.call("GET", &format!("/api/playlists/p/{token2}"), Cred::None, None, &[]).await.status, 404);
    // a private playlist has no link at all
    assert_eq!(t.call("GET", "/api/playlists/p/AAAAAAAAAAAAAAAAAAAAAA", Cred::None, None, &[]).await.status, 404);
}

#[tokio::test]
async fn storage_survives_a_restart() {
    let t = setup();
    let id = t.create(cookie(A), "kept").await["id"].as_str().unwrap().to_string();
    let token = t.call("POST", &format!("/api/playlists/{id}/share"), cookie(A), None, &[]).await.json["share_token"].as_str().unwrap().to_string();
    let files: Vec<_> = std::fs::read_dir(t.dir.join("playlists")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files.len(), 1, "one file per account and no temp files left: {files:?}");
    assert!(!files[0].contains(':') && !files[0].contains("aaaa"), "the file name does not name the account");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(t.dir.join("playlists").join(&files[0])).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let t2 = T { app: make_app(&t.dir, Limits::default()), dir: t.dir.clone(), phone: t.phone.clone(), _dir: tempfile::tempdir().unwrap() };
    assert_eq!(t2.call("GET", &format!("/api/playlists/{id}"), cookie(A), None, &[]).await.json["name"], "kept");
    assert_eq!(t2.call("GET", &format!("/api/playlists/p/{token}"), Cred::None, None, &[]).await.status, 200, "the share index is rebuilt from the files");
}

// ---------------- optimistic concurrency ----------------

#[tokio::test]
async fn if_match_conflict_409_with_server_copy() {
    let t = setup();
    let p = t.create(cookie(A), "shared between devices").await;
    let id = p["id"].as_str().unwrap().to_string();
    let v0 = format!("\"{}\"", p["version"].as_str().unwrap());
    // device 1 adds a track with the version it has
    let r = t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": CID1})), &[("if-match", &v0)]).await;
    assert_eq!(r.status, 200);
    let v1 = r.etag.clone().unwrap();
    assert_ne!(v1, v0);
    // device 2 still holds v0: 409, and the server's copy to merge with
    let r = t.call("PUT", &format!("/api/playlists/{id}"), Cred::Phone, Some(json!({"name": "renamed on the phone", "items": []})), &[("if-match", &v0)]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json["error"]["code"], "conflict");
    assert_eq!(r.json["playlist"]["items"][0]["cid"], CID1);
    assert_eq!(r.etag.as_deref(), Some(v1.as_str()));
    assert_eq!(format!("\"{}\"", r.json["playlist"]["version"].as_str().unwrap()), v1);
    // after merging it sends again with the version it was given
    let merged = json!({"name": "renamed on the phone", "items": r.json["playlist"]["items"]});
    let r = t.call("PUT", &format!("/api/playlists/{id}"), Cred::Phone, Some(merged), &[("if-match", &v1)]).await;
    assert_eq!(r.status, 200, "{}", r.json);
    assert_eq!(r.json["name"], "renamed on the phone");
    assert_eq!(r.json["count"], 1);
    // delete with a stale version is refused too
    assert_eq!(t.call("DELETE", &format!("/api/playlists/{id}"), cookie(A), None, &[("if-match", &v1)]).await.status, 409);
    // If-None-Match: a poll that changes nothing costs a 304
    let cur = r.etag.unwrap();
    assert_eq!(t.call("GET", &format!("/api/playlists/{id}"), cookie(A), None, &[("if-none-match", &cur)]).await.status, 304);
    assert_eq!(t.call("DELETE", &format!("/api/playlists/{id}"), cookie(A), None, &[("if-match", &cur)]).await.status, 200);
}

// ---------------- limits ----------------

#[tokio::test]
async fn limits_playlists_items_size_rate() {
    let t = setup_with(Limits { max_playlists: 2, max_items: 3, max_account_bytes: 4 << 10, writes_per_min: 1000, ..Limits::default() }, true);
    let id = t.create(cookie(A), "one").await["id"].as_str().unwrap().to_string();
    t.create(cookie(A), "two").await;
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "three"})), &[]).await;
    assert_eq!(r.status, 422);
    assert_eq!(r.json["error"]["code"], "limit");
    assert_eq!(t.call("POST", "/api/playlists", cookie(B), Some(json!({"name": "B's first"})), &[]).await.status, 201, "limits are per account");
    for _ in 0..3 {
        assert_eq!(t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": CID1})), &[]).await.status, 200);
    }
    assert_eq!(t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(json!({"cid": CID1})), &[]).await.status, 422);
    let many = json!({"name": "x", "items": [{"cid": CID1}, {"cid": CID1}, {"cid": CID1}, {"cid": CID1}]});
    assert_eq!(t.call("PUT", &format!("/api/playlists/{id}"), cookie(A), Some(many), &[]).await.status, 422);
    // the account's file size: long titles and notes until it is over 4 KiB
    let big = json!({"name": "x", "items": [{"cid": CID1, "title": "t".repeat(120), "note": "n".repeat(500)}, {"cid": CID2, "title": "t".repeat(120), "note": "n".repeat(500)}, {"cid": CID2, "title": "t".repeat(120), "note": "n".repeat(500)}]});
    let r = t.call("PUT", &format!("/api/playlists/{id}"), cookie(A), Some(big.clone()), &[]).await;
    assert_eq!(r.status, 200, "{}", r.json);
    let other = t.call("GET", "/api/playlists", cookie(A), None, &[]).await.json["playlists"][1]["id"].as_str().unwrap().to_string();
    let r = t.call("PUT", &format!("/api/playlists/{other}"), cookie(A), Some(big.clone()), &[]).await;
    assert_eq!(r.status, 413, "{}", r.json);
    assert_eq!(r.json["error"]["code"], "too_large");
    // nothing was written by the refused change
    assert_eq!(t.call("GET", &format!("/api/playlists/{other}"), cookie(A), None, &[]).await.json["count"], 0);
}

#[tokio::test]
async fn write_rate_per_account() {
    let t = setup_with(Limits { writes_per_min: 3, ..Limits::default() }, true);
    for i in 0..3 {
        assert_eq!(t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": format!("p{i}")})), &[]).await.status, 201);
    }
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "p4"})), &[]).await;
    assert_eq!(r.status, 429);
    assert!(r.headers.get("retry-after").is_some());
    assert_eq!(t.call("GET", "/api/playlists", cookie(A), None, &[]).await.status, 200, "reads have their own budget");
    assert_eq!(t.call("POST", "/api/playlists", cookie(B), Some(json!({"name": "b"})), &[]).await.status, 201, "another account is not slowed");
}

#[tokio::test]
async fn total_storage_cap() {
    let t = setup_with(Limits { max_total_bytes: 600, ..Limits::default() }, true);
    assert_eq!(t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "a"})), &[]).await.status, 201);
    let r = t.call("POST", "/api/playlists", cookie(B), Some(json!({"name": "b", "items": [{"cid": CID1, "title": "x".repeat(100)}, {"cid": CID2, "title": "y".repeat(100)}]})), &[]).await;
    assert_eq!(r.status, 507, "{}", r.json);
    assert_eq!(r.json["error"]["code"], "storage_full");
}

#[tokio::test]
async fn create_with_items_for_the_first_sign_in_merge() {
    let t = setup();
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "made offline", "items": [{"id": "it_localAAAA", "cid": CID1, "title": "a", "added_at": "2026-11-18T10:00:00Z"}, {"cid": CID2}]})), &[]).await;
    assert_eq!(r.status, 201, "{}", r.json);
    assert_eq!(r.json["items"][0]["id"], "it_localAAAA", "a client's item id is kept: it is what the merge matches on");
    assert_eq!(r.json["items"][0]["added_at"], "2026-11-18T10:00:00Z");
    assert!(r.headers.get("location").unwrap().to_str().unwrap().starts_with("/api/playlists/pl_"));
    let dup = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "x", "items": [{"id": "it_sameSAME", "cid": CID1}, {"id": "it_sameSAME", "cid": CID2}]})), &[]).await;
    assert_eq!(dup.status, 400);
    let _ = Arc::new(());
}

// ---------------- the storage gate ----------------

#[tokio::test]
async fn items_must_name_admitted_programs() {
    let t = setup();
    let id = t.create(cookie(A), "gated").await["id"].as_str().unwrap().to_string();
    let add = |cid: &str| json!({"cid": cid});
    assert_eq!(t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(add(CID1)), &[]).await.status, 200);
    // not stored: refused
    let r = t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(add("z4EBG9j9RzFGWj5WxHbDN8BWr3wBCJpbHqRY1XWpYKgC1aUgVwU")), &[]).await;
    assert_eq!((r.status.as_u16(), r.json["error"]["code"].as_str()), (422, Some("not_stored")), "{}", r.json);
    // stored, but under a licence the gate refuses: refused, and the message says why
    std::fs::write(t.dir.join(format!("programs/{CID_ARR}.json")), json!({"cid": CID_ARR, "text": "", "meta": {"license": "all-rights-reserved", "visibility": "public"}}).to_string()).unwrap();
    let r = t.call("POST", &format!("/api/playlists/{id}/items"), cookie(A), Some(add(CID_ARR)), &[]).await;
    assert_eq!((r.status.as_u16(), r.json["error"]["code"].as_str()), (422, Some("not_creative_commons")), "{}", r.json);
    assert!(r.json["error"]["message"].as_str().unwrap().contains(beatscript_serve::gate::CC_ONLY));
    // create and replace with items are gated too
    let r = t.call("POST", "/api/playlists", cookie(A), Some(json!({"name": "x", "items": [add(CID_ARR)]})), &[]).await;
    assert_eq!(r.status.as_u16(), 422, "{}", r.json);
    let r = t.call("PUT", &format!("/api/playlists/{id}"), cookie(A), Some(json!({"name": "x", "items": [add(CID2), add(CID_ARR)]})), &[]).await;
    assert_eq!(r.status.as_u16(), 422, "{}", r.json);
}
