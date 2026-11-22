//! Go's auth_test.go and sec1_test.go, on the transport-free handlers (the wire is the contract test's).

use super::sig::tests::{eth_key, personal_sign};
use super::*;

use std::sync::atomic::{AtomicI64, Ordering};

struct H {
    svc: AuthSvc,
    dir: tempfile::TempDir,
    clock: Arc<AtomicI64>, // unix ms
}

/// 2026-11-17T12:00:00Z, the Go harness's clock.
const T0: i64 = 1_794_916_800_000;

fn harness_with(public_url: &str, alt: &str) -> H {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let svc = AuthSvc::with(Options {
        data: dir.path().to_path_buf(),
        secret_env: "0123456789abcdef0123456789abcdef".into(),
        public_url: public_url.into(),
        alt_origins: alt.into(),
        legacy_hosts: "btw.bitwrap.io".into(),
        now: Box::new(move || UNIX_EPOCH + Duration::from_millis(c.load(Ordering::SeqCst) as u64)),
    })
    .unwrap();
    H { svc, dir, clock }
}

fn harness() -> H {
    harness_with("https://btw.bitwrap.io", "")
}

impl H {
    fn advance(&self, d: Duration) {
        self.clock.fetch_add(d.as_millis() as i64, Ordering::SeqCst);
    }
    fn challenge(&self) -> (String, String, String) {
        let r = self.svc.challenge("1.2.3.4");
        let Reply::Json(v, _) = r else { panic!("challenge refused") };
        (v["nonce"].as_str().unwrap().into(), v["message"].as_str().unwrap().into(), v["expires"].as_str().unwrap().into())
    }
}

fn hdrs(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.append(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    h
}

fn login_body(typ: &str, addr: &str, sig: &str, nonce: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"type": typ, "address": addr, "signature": sig, "nonce": nonce})).unwrap()
}

fn ed_key(seed: u8) -> (ed25519_dalek::SigningKey, String) {
    let k = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    let p = hex::encode(k.verifying_key().as_bytes());
    (k, p)
}

fn ed_sign(k: &ed25519_dalek::SigningKey, m: &str) -> String {
    hex::encode(ed25519_dalek::Signer::sign(k, m.as_bytes()).to_bytes())
}

/// (status, error code or "", cookies set, json)
fn parts(r: Reply) -> (u16, String, Vec<String>, Value) {
    match r {
        Reply::Json(v, c) => (200, String::new(), c.iter().map(|c| c.to_str().unwrap().to_string()).collect(), v),
        Reply::Raw(resp) => {
            let st = resp.status().as_u16();
            let cookies: Vec<String> = resp.headers().get_all(header::SET_COOKIE).iter().map(|c| c.to_str().unwrap().to_string()).collect();
            let body = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(axum::body::to_bytes(resp.into_body(), 1 << 20)).unwrap();
            let v: Value = serde_json::from_slice(&body).unwrap();
            (st, v["error"]["code"].as_str().unwrap_or("").into(), cookies, v)
        }
    }
}

fn session(cookies: &[String]) -> Option<String> {
    cookies.iter().find_map(|c| c.strip_prefix("btw_session=")).map(|c| c.split(';').next().unwrap().to_string())
}

fn with_cookie(v: &str) -> HeaderMap {
    hdrs(&[("cookie", &format!("btw_session={v}"))])
}

fn login_eth(h: &H, seed: u8) -> (String, String) {
    let (sk, a) = eth_key(seed);
    let (n, m, _) = h.challenge();
    let (st, _, c, _) = parts(h.svc.login(&HeaderMap::new(), "1.2.3.4", &login_body("eth", &a, &personal_sign(&sk, &m), &n)));
    assert_eq!(st, 200);
    (a, session(&c).unwrap())
}

#[test]
fn challenge_shape() {
    let h = harness();
    let (n, m, e) = h.challenge();
    assert_eq!(m, format!("btw.bitwrap.io sign-in\nnonce: {n}\nissued: 2026-11-17T12:00:00Z\nexpires: 2026-11-17T12:05:00Z"));
    assert_eq!(e, "2026-11-17T12:05:00Z");
    assert_eq!(n.len(), 32);
    assert_ne!(h.challenge().0, n, "nonces repeat");
    let r = h.svc.challenge("1.2.3.4").response();
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    // the domain follows the configured origin, port and all
    let h2 = harness_with("http://127.0.0.1:18097", "");
    assert!(h2.challenge().1.starts_with("127.0.0.1:18097 sign-in\n"));
    assert!(harness_with("", "").challenge().1.starts_with("localhost sign-in\n"));
}

#[test]
fn login_eth_and_cookie() {
    let h = harness();
    let (sk, a) = eth_key(3);
    let (n, m, _) = h.challenge();
    let (st, _, c, v) = parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("eth", &a, &personal_sign(&sk, &m), &n)));
    assert_eq!(st, 200);
    assert_eq!(v, json!({"user": {"id": format!("eip155:{a}"), "kind": "eth", "address": a, "via": "cookie"}}));
    let ck = session(&c).unwrap();
    // Go's http.Cookie.String(): Expires is now + 7 days
    assert_eq!(c[0], format!("btw_session={ck}; Path=/; Expires=Tue, 24 Nov 2026 12:00:00 GMT; Max-Age=604800; HttpOnly; Secure; SameSite=Lax"));
    let (_, _, _, me) = parts(h.svc.me(&with_cookie(&ck)));
    assert_eq!(me["user"]["via"], "cookie");
    assert_eq!(me["user"]["id"], format!("eip155:{a}"));
    assert_eq!(crate::go_json(&parts(h.svc.me(&HeaderMap::new())).3), r#"{"user":null}"#);
    // mixed-case address: the same user
    let (n, m, _) = h.challenge();
    let up = format!("0x{}", a[2..].to_uppercase());
    let (st, _, _, v) = parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("eth", &up, &personal_sign(&sk, &m), &n)));
    assert_eq!((st, v["user"]["id"].as_str().unwrap()), (200, format!("eip155:{a}").as_str()));
    // not Secure over plain http
    let h3 = harness_with("http://localhost:8080", "");
    let (n, m, _) = h3.challenge();
    let (_, _, c, _) = parts(h3.svc.login(&HeaderMap::new(), "ip", &login_body("eth", &a, &personal_sign(&sk, &m), &n)));
    assert!(!c[0].contains("Secure"));
}

#[test]
fn login_ed25519() {
    let h = harness();
    let (k, p) = ed_key(5);
    let (n, m, _) = h.challenge();
    let (st, _, c, v) = parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("ed25519", &p, &ed_sign(&k, &m), &n)));
    assert!(st == 200 && session(&c).is_some());
    assert_eq!(v["user"], json!({"id": format!("ed25519:{p}"), "kind": "ed25519", "address": p, "via": "cookie"}));
    let (n, m, _) = h.challenge();
    let (st, ..) = parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("ed25519", &format!("0x{}", p.to_uppercase()), &format!("0x{}", ed_sign(&k, &m)), &n)));
    assert_eq!(st, 200, "0x forms");
}

#[test]
fn login_failures_set_no_cookie() {
    let h = harness();
    let (vk, victim) = eth_key(10);
    let (ak, _) = eth_key(11);
    let (ed, edp) = ed_key(12);
    let (ed2, _) = ed_key(13);
    let none = HeaderMap::new();
    let try_ = |b: Vec<u8>| parts(h.svc.login(&none, "ip", &b));
    let cases: Vec<(&str, Box<dyn Fn() -> Vec<u8>>, u16, &str)> = vec![
        ("someone else's signature", Box::new(|| { let (n, m, _) = h.challenge(); login_body("eth", &victim, &personal_sign(&ak, &m), &n) }), 401, "bad_signature"),
        ("another message", Box::new(|| { let (n, m, _) = h.challenge(); login_body("eth", &victim, &personal_sign(&vk, &(m + "x")), &n) }), 401, "bad_signature"),
        ("another challenge", Box::new(|| { let (_, m1, _) = h.challenge(); let (n2, _, _) = h.challenge(); login_body("eth", &victim, &personal_sign(&vk, &m1), &n2) }), 401, "bad_signature"),
        ("unknown nonce", Box::new(|| login_body("eth", &victim, &personal_sign(&vk, "x"), "AAAAAAAAAAAAAAAAAAAAAAAA")), 401, "challenge_stale"),
        ("ed25519 sig as eth", Box::new(|| { let (n, m, _) = h.challenge(); login_body("eth", &victim, &ed_sign(&ed, &m), &n) }), 401, "bad_signature"),
        ("ed25519 wrong key", Box::new(|| { let (n, m, _) = h.challenge(); login_body("ed25519", &edp, &ed_sign(&ed2, &m), &n) }), 401, "bad_signature"),
        ("eth sig as ed25519", Box::new(|| { let (n, m, _) = h.challenge(); login_body("ed25519", &edp, &personal_sign(&vk, &m), &n) }), 401, "bad_signature"),
        ("unknown type", Box::new(|| { let (n, m, _) = h.challenge(); login_body("rsa", &victim, &personal_sign(&vk, &m), &n) }), 400, "bad_request"),
        ("short address", Box::new(|| { let (n, m, _) = h.challenge(); login_body("eth", "0x1234", &personal_sign(&vk, &m), &n) }), 400, "bad_request"),
        ("non-hex signature", Box::new(|| { let (n, _, _) = h.challenge(); login_body("eth", &victim, &format!("0x{}", "zz".repeat(65)), &n) }), 400, "bad_request"),
        ("empty body", Box::new(Vec::new), 400, "bad_request"),
        ("not json", Box::new(|| b"nonce=1".to_vec()), 400, "bad_request"),
        ("huge body", Box::new(|| format!("{{\"nonce\":\"{}\"}}", "a".repeat(10000)).into_bytes()), 413, "too_large"),
    ];
    for (name, body, code, err) in cases {
        let (st, e, c, _) = try_(body());
        assert_eq!((st, e.as_str()), (code, err), "{name}");
        assert!(c.is_empty(), "{name}: a failed login set a cookie");
    }
    // replay
    let (n, m, _) = h.challenge();
    let b = login_body("eth", &victim, &personal_sign(&vk, &m), &n);
    assert_eq!(try_(b.clone()).0, 200);
    let (st, e, c, _) = try_(b);
    assert_eq!((st, e.as_str(), c.len()), (401, "challenge_stale", 0));
    // a failed attempt burns the nonce too
    let (n, m, _) = h.challenge();
    try_(login_body("eth", &victim, &personal_sign(&ak, &m), &n));
    assert_eq!(try_(login_body("eth", &victim, &personal_sign(&vk, &m), &n)).1, "challenge_stale");
}

#[test]
fn challenge_expires() {
    let h = harness();
    let (sk, a) = eth_key(20);
    let (n, m, _) = h.challenge();
    h.advance(Duration::from_secs(5 * 60 + 1));
    let (st, e, c, _) = parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("eth", &a, &personal_sign(&sk, &m), &n)));
    assert_eq!((st, e.as_str(), c.len()), (401, "challenge_stale", 0));
    let (n, m, _) = h.challenge();
    h.advance(Duration::from_secs(4 * 60 + 59));
    assert_eq!(parts(h.svc.login(&HeaderMap::new(), "ip", &login_body("eth", &a, &personal_sign(&sk, &m), &n))).0, 200);
}

#[test]
fn bad_cookie_is_cleared_by_me() {
    let h = harness();
    let (_, ck) = login_eth(&h, 30);
    let (p, s) = ck.split_once('.').unwrap();
    let mut sb = s.as_bytes().to_vec();
    let mid = sb.len() / 2;
    sb[mid] = if sb[mid] == b'A' { b'B' } else { b'A' };
    let forged = format!("{p}.{}", String::from_utf8(sb).unwrap());
    let (st, _, c, v) = parts(h.svc.me(&with_cookie(&forged)));
    assert_eq!((st, v), (200, json!({"user": null})));
    assert_eq!(c, vec!["btw_session=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; HttpOnly; Secure; SameSite=Lax".to_string()]);
    // the error UserFrom gives is a 401 for every tampering
    for bad in ["nodot", &format!("{p}."), "!!!.???", &format!("{s}.{p}")] {
        let e = h.svc.user(&with_cookie(bad)).unwrap_err();
        assert_eq!(e.status, 401, "{bad}");
    }
    // an empty cookie is no credential at all
    assert_eq!(h.svc.user(&with_cookie("")).unwrap(), None);
    let (_, _, c, _) = parts(h.svc.me(&with_cookie("")));
    assert!(c.is_empty());
}

#[test]
fn logout() {
    let h = harness();
    let (st, _, c, v) = parts(h.svc.logout(&HeaderMap::new()));
    assert_eq!((st, v), (200, json!({"ok": true})));
    assert!(c[0].starts_with("btw_session=; ") && c[0].contains("Max-Age=0"));
}

#[test]
fn tokens() {
    let h = harness();
    let (a, ck) = login_eth(&h, 40);
    let (_, ck2) = login_eth(&h, 41);
    assert_eq!(parts(h.svc.token_create(&HeaderMap::new(), "ip")).1, "auth_required");
    let mint = |c: &str| {
        let (st, _, _, v) = parts(h.svc.token_create(&with_cookie(c), "ip"));
        assert_eq!(st, 200);
        assert_eq!(v["max_per_user"], 5);
        v["token"].as_str().unwrap().to_string()
    };
    let tok = mint(&ck);
    assert!(tok.starts_with("btw_") && tok.len() == 47);
    let bearer = |t: &str| hdrs(&[("authorization", &format!("Bearer {t}"))]);
    assert_eq!(parts(h.svc.token_create(&bearer(&tok), "ip")).0, 401, "a token minted a token");
    // accepted wherever the cookie is: by this service's verifier and by any other Auth on the same files
    let u = h.svc.user(&bearer(&tok)).unwrap().unwrap();
    assert_eq!((u.id.as_str(), u.via.as_str()), (format!("eip155:{a}").as_str(), "token"));
    let other = Auth::new(AuthConfig { data: h.dir.path().to_path_buf(), secret_env: None, hosts: vec![] });
    assert_eq!(other.user_from_token(&tok), Some(format!("eip155:{a}")));
    let (_, _, _, me) = parts(h.svc.me(&bearer(&tok)));
    assert_eq!(me["user"]["via"], "token");
    for bad in [format!("btw_{}", "A".repeat(43)), "btw_".into(), format!("{tok}x"), tok[..tok.len() - 1].to_string()] {
        assert!(h.svc.user(&bearer(&bad)).is_err(), "{bad}");
        assert!(h.svc.user_from_token(&bad).is_none());
    }
    assert_eq!(h.svc.user(&bearer("some-admin-token")).unwrap(), None, "another scheme's bearer");
    let mixed = hdrs(&[("cookie", "btw_session=x.y"), ("authorization", &format!("Bearer {tok}"))]);
    assert_eq!(h.svc.user(&mixed).unwrap().unwrap().via, "token", "the bearer wins over a bad cookie");
    // only the hash, 0600, Go's layout
    let p = h.dir.path().join("tokens.json");
    let b = std::fs::read_to_string(&p).unwrap();
    assert!(!b.contains(&tok) && !b.contains(&tok[4..]) && b.contains(&hash_token(&tok)));
    assert!(b.starts_with("[\n {\n  \"hash\": \"") && b.ends_with("\n }\n]"), "{b}");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    // at most 5: the sixth mint drops the oldest
    let mut toks = vec![tok.clone()];
    for _ in 0..5 {
        h.advance(Duration::from_secs(1));
        toks.push(mint(&ck));
    }
    assert_eq!(h.svc.token_count(&format!("eip155:{a}")), 5);
    assert!(h.svc.user_from_token(&toks[0]).is_none(), "the oldest token should have been dropped");
    for t in &toks[1..] {
        assert!(h.svc.user_from_token(t).is_some());
    }
    // revoke-all leaves the other user's token
    let other_tok = mint(&ck2);
    let (st, _, _, v) = parts(h.svc.token_revoke(&bearer(&toks[1])));
    assert_eq!((st, v), (200, json!({"revoked": 5})));
    for t in &toks[1..] {
        assert!(h.svc.user_from_token(t).is_none(), "a revoked token still works");
        assert!(other.user_from_token(t).is_none(), "another Auth still takes a revoked token");
    }
    assert!(h.svc.user_from_token(&other_tok).is_some());
    assert_eq!(parts(h.svc.token_revoke(&HeaderMap::new())).0, 401);
}

/// A token minted and revoked inside one second, at the same file size, is still seen by a verifier that read the
/// file before (the inode changes with every rename).
#[test]
fn same_second_rewrite_is_noticed() {
    let h = harness();
    let (_, ck) = login_eth(&h, 50);
    let reader = Auth::new(AuthConfig { data: h.dir.path().to_path_buf(), secret_env: None, hosts: vec![] });
    for _ in 0..5 {
        let (_, _, _, v) = parts(h.svc.token_create(&with_cookie(&ck), "ip"));
        let t = v["token"].as_str().unwrap().to_string();
        assert!(reader.user_from_token(&t).is_some());
        parts(h.svc.token_revoke(&with_cookie(&ck)));
        assert!(reader.user_from_token(&t).is_none());
    }
}

#[test]
fn rate_limits() {
    let h = harness();
    for _ in 0..30 {
        assert_eq!(h.svc.challenge("9.9.9.9").status(), 200);
    }
    let r = h.svc.challenge("9.9.9.9").response();
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()[header::RETRY_AFTER], "60");
    assert_eq!(h.svc.challenge("9.9.9.8").status(), 200, "per address");
    for _ in 0..20 {
        h.svc.login(&HeaderMap::new(), "7.7.7.7", b"{}");
    }
    let (st, e, _, v) = parts(h.svc.login(&HeaderMap::new(), "7.7.7.7", b"{}"));
    assert_eq!((st, e.as_str()), (429, "rate_limited"));
    assert_eq!(v["error"]["message"], "too many login requests; slow down");
}

#[test]
fn challenge_flood_is_bounded() {
    let h = harness();
    {
        let mut c = h.svc.challenges.lock().unwrap();
        let exp = h.svc.now() + Duration::from_secs(3600);
        for i in 0..MAX_CHALLENGES {
            c.insert(format!("n{i}"), (String::new(), exp));
        }
    }
    assert_eq!(h.svc.challenge("a").status(), 503);
    h.advance(Duration::from_secs(7200));
    assert_eq!(h.svc.challenge("b").status(), 200);
}

/// sec1_test.go TestCrossSiteRequestsToAuthRoutesAreRefused.
#[test]
fn cross_site_requests_are_refused() {
    let h = harness();
    let (k, p) = ed_key(60);
    let (n, m, _) = h.challenge();
    let evil = hdrs(&[("origin", "https://evil.example"), ("content-type", "text/plain")]);
    let (st, _, c, _) = parts(h.svc.login(&evil, "ip", &login_body("ed25519", &p, &ed_sign(&k, &m), &n)));
    assert_eq!((st, c.len()), (403, 0));
    // the refusal comes first: the nonce is not burned
    let same = hdrs(&[("origin", "https://btw.bitwrap.io")]);
    let (st, _, c, _) = parts(h.svc.login(&same, "ip", &login_body("ed25519", &p, &ed_sign(&k, &m), &n)));
    assert_eq!(st, 200);
    let ck = session(&c).unwrap();
    for mods in [
        vec![("origin", "null")],
        vec![("origin", "https://beats.bitwrap.io")],
        vec![("origin", "https://btw.bitwrap.io:8443")],
        vec![("sec-fetch-site", "cross-site")],
        vec![("sec-fetch-site", "same-site")],
        vec![("origin", "::::")],
    ] {
        let mut hm = with_cookie(&ck);
        hm.insert("host", HeaderValue::from_static("example.com"));
        for (k2, v) in &mods {
            hm.insert(*k2, HeaderValue::from_str(v).unwrap());
        }
        let (n, m, _) = h.challenge();
        for (name, r) in [
            ("login", h.svc.login(&hm, "ip", &login_body("ed25519", &p, &ed_sign(&k, &m), &n))),
            ("logout", h.svc.logout(&hm)),
            ("token", h.svc.token_create(&hm, "ip")),
            ("revoke", h.svc.token_revoke(&hm)),
        ] {
            let (st, _, c, _) = parts(r);
            assert_eq!(st, 403, "{mods:?} {name}");
            assert!(c.is_empty(), "{mods:?} {name} set a cookie");
        }
    }
    assert_eq!(h.svc.token_count(&format!("ed25519:{p}")), 0, "a cross-site request minted a token");
    for mods in [
        vec![("origin", "https://btw.bitwrap.io")],
        vec![("origin", "http://example.com")],
        vec![("sec-fetch-site", "same-origin")],
        vec![],
        vec![("sec-fetch-site", "none")],
    ] {
        let mut hm = with_cookie(&ck);
        hm.insert("host", HeaderValue::from_static("example.com"));
        for (k2, v) in &mods {
            hm.insert(*k2, HeaderValue::from_str(v).unwrap());
        }
        assert_eq!(h.svc.token_create(&hm, &format!("ip{}", mods.len())).status(), 200, "{mods:?}");
    }
    assert_eq!(h.svc.token_revoke(&with_cookie(&ck)).status(), 200);
}

/// An -alt-origins host is this site; a -legacy-hosts one is not (Go's AltHosts are -alt-origins only).
#[test]
fn alt_origins_pass_legacy_hosts_do_not() {
    let h = harness_with("https://beatscript.app", "https://btw.example/, junk");
    assert!(!h.svc.cross_site(&hdrs(&[("origin", "https://BTW.example")])));
    assert!(!h.svc.cross_site(&hdrs(&[("origin", "https://beatscript.app")])));
    assert!(h.svc.cross_site(&hdrs(&[("origin", "https://btw.bitwrap.io")])));
}

/// sec1_test.go TestTokenStoreFailureDoesNotEchoThePath.
#[test]
fn token_store_failure_does_not_echo_the_path() {
    let h = harness();
    let (_, ck) = login_eth(&h, 70);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(h.dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let (st, _, _, v) = parts(h.svc.token_create(&with_cookie(&ck), "ip"));
    std::fs::set_permissions(h.dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    if st != 500 {
        return; // running as root: the directory stayed writable
    }
    let s = v.to_string();
    assert!(!s.contains(&h.dir.path().display().to_string()) && !s.contains("ermission"), "{s}");
}

#[test]
fn load_secret_as_go() {
    let d = tempfile::tempdir().unwrap();
    let a = load_secret(d.path(), "").unwrap();
    assert_eq!(a.len(), 32);
    assert_eq!(load_secret(d.path(), "").unwrap(), a, "the generated secret persists");
    let f = d.path().join("session.secret");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), hex::encode(&a) + "\n");
    assert_eq!(load_secret(d.path(), "an-env-secret-of-length").unwrap(), b"an-env-secret-of-length");
    assert!(load_secret(d.path(), "short").is_err());
    std::fs::write(&f, "not hex").unwrap();
    assert!(load_secret(d.path(), "").is_err(), "a corrupt secret file is an error, not replaced");
    // the verifier reads the key made here
    let d2 = tempfile::tempdir().unwrap();
    let k = load_secret(d2.path(), "").unwrap();
    let v = Auth::new(AuthConfig { data: d2.path().to_path_buf(), secret_env: None, hosts: vec![] });
    assert_eq!(v.secret(), Some(k));
}

#[test]
fn times_as_go() {
    let t = UNIX_EPOCH + Duration::new(1_794_916_800, 120_000_000);
    assert_eq!(rfc3339(t), "2026-11-17T12:00:00Z");
    assert_eq!(rfc3339_nano(t), "2026-11-17T12:00:00.12Z");
    assert_eq!(rfc3339_nano(UNIX_EPOCH + Duration::from_secs(1_794_916_800)), "2026-11-17T12:00:00Z");
    assert_eq!(http_date(UNIX_EPOCH), "Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(http_date(t), "Tue, 17 Nov 2026 12:00:00 GMT");
    assert_eq!(parse_rfc3339("2026-11-17T12:00:00.12Z"), Some((1_794_916_800, 120_000_000)));
    assert_eq!(parse_rfc3339("2026-11-17T13:00:00+01:00"), Some((1_794_916_800, 0)));
}

#[test]
fn origin_hosts_as_url_parse() {
    assert_eq!(origin_host("https://Beatscript.app").as_deref(), Some("Beatscript.app"));
    assert_eq!(origin_host("http://localhost:1234/x?y#z").as_deref(), Some("localhost:1234"));
    assert_eq!(origin_host("https://u@h.example").as_deref(), Some("h.example"));
    assert_eq!(origin_host("//h.example").as_deref(), Some("h.example"));
    assert_eq!(origin_host("https://[::1]:8080").as_deref(), Some("[::1]:8080"));
    for bad in ["null", "::::", "https://", "https://h:x", "a b:c", "https://h e", "beatscript.app"] {
        assert_eq!(origin_host(bad), None, "{bad}");
    }
}

#[test]
fn require_user_answers_go_401() {
    let h = harness();
    let r = h.svc.require_user(&HeaderMap::new()).unwrap_err();
    let (st, e, _, v) = parts(Reply::Raw(r));
    assert_eq!((st, e.as_str()), (401, "auth_required"));
    assert_eq!(v["error"]["message"], "authentication required: sign in at https://btw.bitwrap.io (wallet or browser key), press \"MCP token\", and send it as `Authorization: Bearer btw_…`");
    let (a, ck) = login_eth(&h, 80);
    assert_eq!(h.svc.require_user(&with_cookie(&ck)).unwrap().id, format!("eip155:{a}"));
    assert_eq!(h.svc.require_caller(&with_cookie(&ck), "PUT", "/x", b"").unwrap().uid, format!("eip155:{a}"));
}
