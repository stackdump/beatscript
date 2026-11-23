//! The lineage routes over HTTP: /p/<cid>/tree (page and JSON-LD), /p/<a>/compare/<b>, /api/lineage/<cid>, and the
//! same 404 for a private program as for an unknown one.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use beatscript_serve::lineage::{router, Lineage};
use serde_json::{json, Value};
use tower::ServiceExt;

const ORIGIN: &str = "https://beatscript.app";

fn put(dir: &std::path::Path, text: &str, meta: Value) -> String {
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
    let (cid, canon) = (rep["cid"].as_str().unwrap().to_string(), rep["canonical"].as_str().unwrap().to_string());
    std::fs::write(dir.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": canon, "meta": meta})).unwrap()).unwrap();
    cid
}

async fn get(app: &axum::Router, path: &str, accept: Option<&str>) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut r = Request::get(path);
    if let Some(a) = accept {
        r = r.header(header::ACCEPT, a);
    }
    let res = app.clone().oneshot(r.body(Body::empty()).unwrap()).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let b = axum::body::to_bytes(res.into_body(), 1 << 22).await.unwrap();
    (st, h, String::from_utf8(b.to_vec()).unwrap())
}

#[tokio::test]
async fn tree_compare_and_api() {
    let d = tempfile::tempdir().unwrap();
    let root = put(d.path(), "tempo 124\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n", json!({"visibility": "public", "name": "night bus"}));
    let fork = put(d.path(), "tempo 126\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n", json!({"visibility": "public", "forked_from": root}));
    let private = put(d.path(), "tempo 100\nkey am\ndeck A: kick x ~ x ~\n", json!({"visibility": "private", "forked_from": root}));
    let app = router(Arc::new(Lineage::new(d.path(), Vec::new())), ORIGIN);

    let (st, h, body) = get(&app, &format!("/p/{root}/tree"), Some("text/html")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert_eq!(h[header::VARY], "Accept");
    assert!(body.contains("tempo 124 -&gt; 126"), "{body}");
    assert!(body.contains("night bus"));
    assert!(!body.contains(&private));

    let (st, h, body) = get(&app, &format!("/p/{root}/tree"), Some("application/ld+json")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("application/ld+json"));
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let doc: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(doc["@type"], "Lineage");
    assert_eq!(doc["forks"], json!([fork]));
    assert_eq!(doc["derivations"][0]["change"], json!(["tempo 124 -> 126"]));

    let (st, _, body2) = get(&app, &format!("/p/{root}/tree.jsonld"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body2, body);
    let (st, h, body3) = get(&app, &format!("/api/lineage/{root}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("application/json"));
    assert_eq!(body3, body);

    let (st, _, body) = get(&app, &format!("/p/{fork}/compare/{root}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(body.contains("tempo 126 -&gt; 124"), "{body}");
    let (st, _, body) = get(&app, &format!("/p/{fork}/compare/{root}.jsonld"), None).await;
    assert_eq!(st, StatusCode::OK);
    let c: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(c["change"], json!(["tempo 126 -> 124"]));

    // private and unknown: the same 404, as a page and as JSON
    let unknown = "z4EBG9jCtKY4ioQ3TEJCQQmTTJZjugJRihCNbBxNJrBaWr1adMi";
    let (s1, _, b1) = get(&app, &format!("/p/{private}/tree"), None).await;
    let (s2, _, b2) = get(&app, &format!("/p/{unknown}/tree"), None).await;
    assert_eq!((s1, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    assert_eq!(b1, b2);
    let (s1, _, b1) = get(&app, &format!("/api/lineage/{private}"), None).await;
    let (s2, _, b2) = get(&app, &format!("/api/lineage/{unknown}"), None).await;
    assert_eq!((s1, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    assert_eq!(b1, b2);
    let (s, _, _) = get(&app, &format!("/p/{root}/compare/{private}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, "/p/not-a-cid/tree", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
