//! OAuth: discovery, registration, PKCE, exact redirect match, audience, refresh (rotation, reuse), revoke.

use super::*;

const UID: &str = "ed25519:8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";
const REDIRECT: &str = "http://localhost:33418/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn oauth(dir: &std::path::Path) -> OAuth {
    OAuth::new(dir.to_path_buf(), "https://beatscript.app", vec![1u8; 32])
}

fn challenge(v: &str) -> String {
    B64.encode(Sha256::digest(v.as_bytes()))
}

fn register(o: &OAuth) -> String {
    let v = o.register("1.2.3.4", format!(r#"{{"client_name":"Claude Code","redirect_uris":["{REDIRECT}"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"}}"#).as_bytes()).unwrap();
    assert_eq!(v["token_endpoint_auth_method"], "none");
    v["client_id"].as_str().unwrap().to_string()
}

fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn authz(cid: &str) -> HashMap<String, String> {
    q(&[("response_type", "code"), ("client_id", cid), ("redirect_uri", REDIRECT), ("code_challenge", &challenge(VERIFIER)), ("code_challenge_method", "S256"), ("state", "xyz"), ("resource", "https://beatscript.app/mcp"), ("scope", "mcp")])
}

/// Through consent to a code: the URL's `code`.
fn code_for(o: &OAuth, cid: &str) -> String {
    let (r, _) = o.authorize_check(&authz(cid)).unwrap();
    let sealed = o.seal(&r);
    let url = o.decide(UID, &sealed, &o.consent_mac(UID, &sealed), true).unwrap();
    assert!(url.starts_with(&format!("{REDIRECT}?code=")), "{url}");
    assert!(url.contains("&state=xyz&iss=https%3A%2F%2Fbeatscript.app"), "{url}");
    let f = parse_form(url.split_once('?').unwrap().1).unwrap();
    f["code"].clone()
}

fn exchange(o: &OAuth, cid: &str, code: &str, verifier: &str) -> Result<Value, OErr> {
    o.token("1.2.3.4", &q(&[("grant_type", "authorization_code"), ("code", code), ("client_id", cid), ("redirect_uri", REDIRECT), ("code_verifier", verifier), ("resource", "https://beatscript.app/mcp")]))
}

#[test]
fn discovery() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let pr = o.protected_resource();
    assert_eq!(pr["resource"], "https://beatscript.app/mcp");
    assert_eq!(pr["authorization_servers"], json!(["https://beatscript.app"]));
    let m = o.server_metadata();
    assert_eq!(m["issuer"], "https://beatscript.app");
    assert_eq!(m["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(m["response_types_supported"], json!(["code"]));
    assert_eq!(m["grant_types_supported"], json!(["authorization_code", "refresh_token"]));
    assert_eq!(m["registration_endpoint"], "https://beatscript.app/oauth/register");
    assert_eq!(o.resource_metadata_url(), "https://beatscript.app/.well-known/oauth-protected-resource/mcp");
}

#[test]
fn registration_rules() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    for bad in ["http://evil.example/cb", "javascript:alert(1)", "https://x.example/cb#frag", "https://user@x.example/cb", "data:text/html,x", "noscheme"] {
        let e = o.register("9.9.9.9", format!(r#"{{"redirect_uris":["{bad}"]}}"#).as_bytes()).unwrap_err();
        assert_eq!(e.error, "invalid_redirect_uri", "{bad}");
    }
    for good in ["https://claude.ai/api/mcp/auth_callback", "http://127.0.0.1:5000/cb", "http://[::1]:8/x", "app.beatscript:/oauth"] {
        assert!(redirect_ok(good), "{good}");
    }
    let e = o.register("9.9.9.8", br#"{"redirect_uris":["https://a.example/cb"],"response_types":["token"]}"#).unwrap_err();
    assert_eq!(e.error, "invalid_client_metadata", "no implicit grant");
    assert_eq!(o.register("9.9.9.7", br#"{}"#).unwrap_err().error, "invalid_redirect_uri");
}

#[test]
fn code_pkce_and_tokens() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    // a wrong verifier is refused, and the code is spent by the attempt
    let code = code_for(&o, &cid);
    let e = exchange(&o, &cid, &code, "wrong-verifier-wrong-verifier-wrong-verifier-00").unwrap_err();
    assert_eq!((e.error, e.desc.as_str()), ("invalid_grant", "the code_verifier does not match the code_challenge"));
    assert_eq!(exchange(&o, &cid, &code, VERIFIER).unwrap_err().error, "invalid_grant", "single use");
    // no verifier at all
    let code = code_for(&o, &cid);
    assert_eq!(exchange(&o, &cid, &code, "").unwrap_err().error, "invalid_grant");
    // the right one
    let code = code_for(&o, &cid);
    let t = exchange(&o, &cid, &code, VERIFIER).unwrap();
    assert_eq!(t["token_type"], "Bearer");
    assert_eq!(t["resource"], "https://beatscript.app/mcp");
    let at = t["access_token"].as_str().unwrap();
    assert!(at.starts_with(ACCESS_PREFIX));
    assert_eq!(o.access_user(at).as_deref(), Some(UID));
    // the code cannot be used twice
    assert_eq!(exchange(&o, &cid, &code, VERIFIER).unwrap_err().error, "invalid_grant");
    // the store holds hashes only
    let raw = std::fs::read_to_string(d.path().join("oauth.json")).unwrap();
    assert!(!raw.contains(at) && !raw.contains(t["refresh_token"].as_str().unwrap()));
}

#[test]
fn pkce_required_and_s256_only() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    let mut a = authz(&cid);
    a.insert("code_challenge_method".into(), "plain".into());
    assert!(matches!(o.authorize_check(&a), Err(AuthzErr::Redirect(u)) if u.contains("error=invalid_request")));
    let mut a = authz(&cid);
    a.remove("code_challenge");
    assert!(matches!(o.authorize_check(&a), Err(AuthzErr::Redirect(u)) if u.contains("error=invalid_request")));
    let mut a = authz(&cid);
    a.insert("response_type".into(), "token".into());
    assert!(matches!(o.authorize_check(&a), Err(AuthzErr::Redirect(u)) if u.contains("error=unsupported_response_type")));
}

#[test]
fn redirect_uri_exact_match() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    for other in ["http://localhost:33419/callback", "http://localhost:33418/callback/", "http://localhost:33418/callback?x=1", "http://LOCALHOST:33418/callback", "https://evil.example/cb", ""] {
        let mut a = authz(&cid);
        a.insert("redirect_uri".into(), other.into());
        assert!(matches!(o.authorize_check(&a), Err(AuthzErr::Page(_))), "{other}: never redirected to");
    }
    // unknown client: a page too
    assert!(matches!(o.authorize_check(&authz("btwc_nope")), Err(AuthzErr::Page(_))));
    // and at the token endpoint the redirect_uri must be the authorize request's
    let code = code_for(&o, &cid);
    let e = o.token("1.2.3.4", &q(&[("grant_type", "authorization_code"), ("code", &code), ("client_id", &cid), ("redirect_uri", "http://localhost:1/x"), ("code_verifier", VERIFIER)])).unwrap_err();
    assert_eq!(e.error, "invalid_grant");
    // another client cannot redeem it
    let cid2 = register(&o);
    let code = code_for(&o, &cid);
    assert_eq!(exchange(&o, &cid2, &code, VERIFIER).unwrap_err().error, "invalid_grant");
}

#[test]
fn audience() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    // authorize for another resource: refused
    let mut a = authz(&cid);
    a.insert("resource".into(), "https://beatscript.app/api".into());
    assert!(matches!(o.authorize_check(&a), Err(AuthzErr::Redirect(u)) if u.contains("error=invalid_target")));
    // a token request naming another resource: refused
    let code = code_for(&o, &cid);
    let e = o.token("1.2.3.4", &q(&[("grant_type", "authorization_code"), ("code", &code), ("client_id", &cid), ("redirect_uri", REDIRECT), ("code_verifier", VERIFIER), ("resource", "https://other.example/mcp")])).unwrap_err();
    assert_eq!(e.error, "invalid_target");
    // a token for this server bound to another resource (a store from a server with another public URL) is not taken
    let code = code_for(&o, &cid);
    let at = exchange(&o, &cid, &code, VERIFIER).unwrap()["access_token"].as_str().unwrap().to_string();
    let other = OAuth::new(d.path().to_path_buf(), "https://staging.beatscript.app", vec![1u8; 32]);
    assert_eq!(other.access_user(&at), None, "audience is https://beatscript.app/mcp");
    assert_eq!(o.access_user(&at).as_deref(), Some(UID));
    // expired access token
    let mut o2 = oauth(d.path());
    o2.now = Box::new(|| unix_now() + ACCESS_TTL + 1);
    assert_eq!(o2.access_user(&at), None);
}

#[test]
fn refresh_rotates_and_reuse_revokes() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    let code = code_for(&o, &cid);
    let t1 = exchange(&o, &cid, &code, VERIFIER).unwrap();
    let r1 = t1["refresh_token"].as_str().unwrap();
    let refresh = |r: &str, c: &str| o.token("1.2.3.4", &q(&[("grant_type", "refresh_token"), ("refresh_token", r), ("client_id", c)]));
    assert_eq!(refresh(r1, "btwc_other").unwrap_err().error, "invalid_grant", "another client");
    let t2 = refresh(r1, &cid).unwrap();
    let (a1, a2) = (t1["access_token"].as_str().unwrap(), t2["access_token"].as_str().unwrap());
    assert_eq!(o.access_user(a1), None, "the old access token is replaced");
    assert_eq!(o.access_user(a2).as_deref(), Some(UID));
    // r1 again: a stolen copy; the whole grant goes
    assert_eq!(refresh(r1, &cid).unwrap_err().error, "invalid_grant");
    assert_eq!(o.access_user(a2), None);
    assert_eq!(refresh(t2["refresh_token"].as_str().unwrap(), &cid).unwrap_err().error, "invalid_grant");
}

#[test]
fn revoke_and_panel() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    let code = code_for(&o, &cid);
    let t = exchange(&o, &cid, &code, VERIFIER).unwrap();
    let at = t["access_token"].as_str().unwrap();
    let l = o.list(UID);
    assert_eq!(l.len(), 1);
    assert_eq!((l[0]["client_name"].as_str(), l[0]["redirect_uri"].as_str()), (Some("Claude Code"), Some(REDIRECT)));
    // RFC 7009 by refresh token
    o.revoke_token(t["refresh_token"].as_str().unwrap());
    assert_eq!(o.access_user(at), None);
    assert!(o.list(UID).is_empty());
    // the panel's disconnect, and "revoke all"
    for _ in 0..2 {
        let code = code_for(&o, &cid);
        exchange(&o, &cid, &code, VERIFIER).unwrap();
    }
    let id = o.list(UID)[0]["id"].as_str().unwrap().to_string();
    assert!(!o.revoke_grant("ed25519:0000000000000000000000000000000000000000000000000000000000000000", &id), "only the owner");
    assert!(o.revoke_grant(UID, &id));
    assert_eq!(o.revoke_user(UID), 1);
    assert!(o.list(UID).is_empty());
    // a store survives a restart
    let code = code_for(&o, &cid);
    let at = exchange(&o, &cid, &code, VERIFIER).unwrap()["access_token"].as_str().unwrap().to_string();
    assert_eq!(oauth(d.path()).access_user(&at).as_deref(), Some(UID));
}

#[test]
fn consent_is_bound_to_the_account_and_unaltered() {
    let d = tempfile::tempdir().unwrap();
    let o = oauth(d.path());
    let cid = register(&o);
    let (r, _) = o.authorize_check(&authz(&cid)).unwrap();
    let sealed = o.seal(&r);
    let other = "ed25519:0000000000000000000000000000000000000000000000000000000000000000";
    // a consent MAC of another account
    assert_eq!(o.decide(UID, &sealed, &o.consent_mac(other, &sealed), true).unwrap_err().status, 403);
    // an altered request (another redirect)
    let mut r2 = r.clone();
    r2.redirect_uri = "https://evil.example/cb".into();
    let forged = format!("{}.{}", B64.encode(serde_json::to_vec(&r2).unwrap()), sealed.split_once('.').unwrap().1);
    assert_eq!(o.decide(UID, &forged, &o.consent_mac(UID, &forged), true).unwrap_err().status, 400);
    // deny goes back with access_denied
    let u = o.decide(UID, &sealed, &o.consent_mac(UID, &sealed), false).unwrap();
    assert!(u.starts_with(REDIRECT) && u.contains("error=access_denied") && u.contains("state=xyz"), "{u}");
}
