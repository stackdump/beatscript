//! MCP authorization (the MCP spec's OAuth 2.1 profile), so a client (claude.ai, Claude Code) connects to `/mcp`
//! without a pasted token.
//!
//! | route | what it is |
//! |---|---|
//! | `GET /.well-known/oauth-protected-resource[/mcp]` | RFC 9728: the resource `<public>/mcp` and its authorization server |
//! | `GET /.well-known/oauth-authorization-server` | RFC 8414 metadata: code + PKCE S256 only, public clients |
//! | `POST /oauth/register` | RFC 7591 dynamic client registration (public clients, `token_endpoint_auth_method: none`) |
//! | `GET /oauth/authorize` | the consent page: the site's own sign-in (`/js/auth.js`), then the client's name and redirect URI |
//! | `POST /oauth/authorize` | the page's decision (same-site, cookie, a consent MAC): `{"redirect": …}` |
//! | `POST /oauth/token` | `authorization_code` (with `code_verifier`) and `refresh_token` (rotated) |
//! | `POST /oauth/revoke` | RFC 7009 |
//! | `GET /api/auth/clients`, `DELETE /api/auth/clients/<id>` | the MCP panel: the caller's connected clients and tokens |
//!
//! There is no implicit grant and no `plain` PKCE. A redirect URI must equal a registered one byte for byte. Access
//! tokens (`bto_…`, 1 h) are bound to the resource `<public>/mcp`: only `/mcp` takes them (every other route reads
//! `btw_` tokens only), and a token or authorize request naming another `resource` is refused (`invalid_target`).
//! Refresh tokens (`btr_…`, 30 days) rotate on use; a refresh token used twice revokes its grant (a stolen copy).
//!
//! Store: `<data>/oauth.json` (clients and grants; token hashes only, SHA-256, 0600, written by rename). Codes are
//! in memory (10 minutes, single use).

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::any;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use btwserve::playlists::Limiter;

use crate::AppState;

pub const ACCESS_PREFIX: &str = "bto_";
pub const REFRESH_PREFIX: &str = "btr_";
const ACCESS_TTL: i64 = 3600;
const REFRESH_TTL: i64 = 30 * 24 * 3600;
const CODE_TTL: i64 = 600;
const REQ_TTL: i64 = 1800;
const MAX_CLIENTS: usize = 5000;
const MAX_GRANTS_PER_USER: usize = 20;
const MAX_REDIRECTS: usize = 10;
const SCOPE: &str = "mcp";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub client_id: String,
    pub user: String,
    pub resource: String,
    pub redirect_uri: String,
    pub created: i64,
    pub last_used: i64,
    pub access_hash: String,
    pub access_exp: i64,
    pub refresh_hash: String,
    pub refresh_exp: i64,
    /// The refresh token this one replaced: presented again, it means a copy was stolen (the grant is revoked).
    #[serde(default)]
    pub prev_refresh_hash: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    clients: Vec<Client>,
    grants: Vec<Grant>,
}

struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    user: String,
    resource: String,
    exp: i64,
}

/// A validated authorization request: what the consent page shows and what the decision acts on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthzReq {
    pub client_id: String,
    pub redirect_uri: String,
    pub challenge: String,
    pub state: String,
    pub resource: String,
    pub exp: i64,
}

/// An OAuth error answer (RFC 6749 §5.2 shape).
#[derive(Debug, PartialEq)]
pub struct OErr {
    pub status: u16,
    pub error: &'static str,
    pub desc: String,
}

fn oerr(status: u16, error: &'static str, desc: impl Into<String>) -> OErr {
    OErr { status, error, desc: desc.into() }
}

/// What a bad authorize request gets: a page (the redirect URI is not trusted yet: never redirect), or a redirect
/// back to the client with `error=`.
#[derive(Debug, PartialEq)]
pub enum AuthzErr {
    Page(String),
    Redirect(String),
}

pub struct OAuth {
    data: PathBuf,
    pub issuer: String,
    pub resource: String,
    secret: Vec<u8>,
    store: Mutex<Store>,
    codes: Mutex<HashMap<String, Code>>,
    limit_register: Mutex<Limiter>,
    limit_token: Mutex<Limiter>,
    pub now: Box<dyn Fn() -> i64 + Send + Sync>,
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn rand_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the OS has no entropy");
    B64.encode(b)
}

fn sha_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(subtle::ConstantTimeEq::ct_eq(a, b))
}

/// `https://…` anywhere, `http://` only to a loopback host (RFC 8252 §7.3), or a private-use scheme of a native app
/// (`com.example.app:/cb`, RFC 8252 §7.1). No fragment, no userinfo, no script-ish schemes.
pub fn redirect_ok(u: &str) -> bool {
    if u.is_empty() || u.len() > 2000 || u.contains('#') || u.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return false;
    }
    let Some((scheme, rest)) = u.split_once(':') else { return false };
    let s = scheme.to_ascii_lowercase();
    if s.is_empty() || !s.bytes().next().unwrap().is_ascii_alphabetic() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) {
        return false;
    }
    match s.as_str() {
        "https" | "http" => {
            let Some(auth) = rest.strip_prefix("//") else { return false };
            let auth = auth.split(['/', '?']).next().unwrap_or("");
            if auth.contains('@') || auth.is_empty() {
                return false;
            }
            let host = if auth.starts_with('[') { auth.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { auth.rsplit_once(':').map_or(auth, |(h, _)| h).to_string() };
            if host.is_empty() {
                return false;
            }
            s == "https" || matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")
        }
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" | "ftp" | "ws" | "wss" => false,
        // a native app's own scheme: must look like a reversed domain (has a dot), per RFC 8252
        _ => s.contains('.'),
    }
}

/// Append query parameters to a redirect URI (keeping any query it has).
fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut u = base.to_string();
    let mut sep = if base.contains('?') { '&' } else { '?' };
    for (k, v) in params {
        if v.is_empty() {
            continue;
        }
        u.push(sep);
        sep = '&';
        u.push_str(k);
        u.push('=');
        u.push_str(&pct(v));
    }
    u
}

fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// `application/x-www-form-urlencoded` (and query strings) into a map; a repeated name is an error (RFC 6749 §3.1).
pub fn parse_form(s: &str) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    for pair in s.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let dec = |x: &str| -> Result<String, String> {
            let x = x.replace('+', " ");
            let b = x.as_bytes();
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%' {
                    let h = std::str::from_utf8(b.get(i + 1..i + 3).ok_or("bad percent-encoding")?).map_err(|_| "bad percent-encoding")?;
                    out.push(u8::from_str_radix(h, 16).map_err(|_| "bad percent-encoding")?);
                    i += 3;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).map_err(|_| "not UTF-8".to_string())
        };
        let (k, v) = (dec(k)?, dec(v)?);
        if m.insert(k.clone(), v).is_some() {
            return Err(format!("{k} is repeated"));
        }
    }
    Ok(m)
}

/// The verifier's form (RFC 7636 §4.1) and its S256 check against the challenge.
pub fn pkce_ok(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len()) && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
    well_formed && ct_eq(B64.encode(Sha256::digest(verifier.as_bytes())).as_bytes(), challenge.as_bytes())
}

impl OAuth {
    pub fn new(data: PathBuf, public_url: &str, secret: Vec<u8>) -> OAuth {
        let issuer = public_url.trim_end_matches('/').to_string();
        let store = match std::fs::read(data.join("oauth.json")) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                eprintln!("btwserve: oauth: oauth.json does not parse ({e}); starting with no clients (the file is kept until the next write)");
                Store::default()
            }),
            Err(_) => Store::default(),
        };
        let min = Duration::from_secs(60);
        OAuth {
            data,
            resource: format!("{issuer}/mcp"),
            issuer,
            secret,
            store: Mutex::new(store),
            codes: Mutex::new(HashMap::new()),
            limit_register: Mutex::new(Limiter::new(10, min)),
            limit_token: Mutex::new(Limiter::new(60, min)),
            now: Box::new(unix_now),
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn mac(&self, label: &str, data: &str) -> String {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.secret).expect("hmac key");
        m.update(label.as_bytes());
        m.update(b"\n");
        m.update(data.as_bytes());
        B64.encode(m.finalize().into_bytes())
    }

    fn save(&self, s: &Store) {
        let tmp = self.data.join(format!(".oauth-{}", rand_b64(9).replace(['-', '_'], "0")));
        let res = (|| {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            f.write_all(serde_json::to_string(s).unwrap().as_bytes())?;
            drop(f);
            std::fs::rename(&tmp, self.data.join("oauth.json"))
        })();
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("btwserve: oauth: could not write oauth.json: {e}");
        }
    }

    // ---------- metadata ----------

    pub fn resource_metadata_url(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource/mcp", self.issuer)
    }

    pub fn protected_resource(&self) -> Value {
        json!({
            "resource": self.resource,
            "authorization_servers": [self.issuer],
            "bearer_methods_supported": ["header"],
            "scopes_supported": [SCOPE],
            "resource_name": "BeatScript",
            "resource_documentation": format!("{}/llms.txt", self.issuer),
        })
    }

    pub fn server_metadata(&self) -> Value {
        let i = &self.issuer;
        json!({
            "issuer": i,
            "authorization_endpoint": format!("{i}/oauth/authorize"),
            "token_endpoint": format!("{i}/oauth/token"),
            "registration_endpoint": format!("{i}/oauth/register"),
            "revocation_endpoint": format!("{i}/oauth/revoke"),
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "scopes_supported": [SCOPE],
            "authorization_response_iss_parameter_supported": true,
            "service_documentation": format!("{i}/llms.txt"),
        })
    }

    // ---------- registration (RFC 7591) ----------

    pub fn register(&self, ip: &str, body: &[u8]) -> Result<Value, OErr> {
        let (ok, _) = self.limit_register.lock().unwrap().allow(ip);
        if !ok {
            return Err(oerr(429, "invalid_request", "too many registrations from this address; slow down"));
        }
        let v: Value = serde_json::from_slice(body).map_err(|_| oerr(400, "invalid_client_metadata", "the body must be a JSON object"))?;
        let o = v.as_object().ok_or_else(|| oerr(400, "invalid_client_metadata", "the body must be a JSON object"))?;
        let uris: Vec<String> = match o.get("redirect_uris") {
            Some(Value::Array(a)) if !a.is_empty() && a.len() <= MAX_REDIRECTS => a.iter().map(|u| u.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(|| oerr(400, "invalid_redirect_uri", "redirect_uris must be strings"))?,
            _ => return Err(oerr(400, "invalid_redirect_uri", format!("redirect_uris must be a list of 1..{MAX_REDIRECTS} URIs"))),
        };
        if let Some(bad) = uris.iter().find(|u| !redirect_ok(u)) {
            return Err(oerr(400, "invalid_redirect_uri", format!("{bad} is not allowed: use https, http to localhost/127.0.0.1/[::1], or an app's reversed-domain scheme; no fragment")));
        }
        if let Some(g) = o.get("grant_types").and_then(Value::as_array) {
            if g.iter().any(|t| !matches!(t.as_str(), Some("authorization_code") | Some("refresh_token"))) {
                return Err(oerr(400, "invalid_client_metadata", "grant_types may be authorization_code and refresh_token only"));
            }
        }
        if let Some(r) = o.get("response_types").and_then(Value::as_array) {
            if r.iter().any(|t| t.as_str() != Some("code")) {
                return Err(oerr(400, "invalid_client_metadata", "response_types may be code only (there is no implicit grant)"));
            }
        }
        let name: String = o.get("client_name").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect();
        let name = if name.trim().is_empty() { "an unnamed MCP client".to_string() } else { name.trim().to_string() };
        let now = self.now();
        let c = Client { client_id: format!("btwc_{}", rand_b64(18)), client_name: name, redirect_uris: uris, created: now };
        {
            let mut s = self.store.lock().unwrap();
            if s.clients.len() >= MAX_CLIENTS {
                // drop the oldest clients nobody granted anything to
                let used: std::collections::HashSet<String> = s.grants.iter().map(|g| g.client_id.clone()).collect();
                let mut idle: Vec<(i64, String)> = s.clients.iter().filter(|c| !used.contains(&c.client_id)).map(|c| (c.created, c.client_id.clone())).collect();
                idle.sort();
                let drop: std::collections::HashSet<String> = idle.into_iter().take(MAX_CLIENTS / 10).map(|x| x.1).collect();
                s.clients.retain(|c| !drop.contains(&c.client_id));
                if s.clients.len() >= MAX_CLIENTS {
                    return Err(oerr(503, "temporarily_unavailable", "too many registered clients"));
                }
            }
            s.clients.push(c.clone());
            self.save(&s);
        }
        Ok(json!({
            "client_id": c.client_id,
            "client_id_issued_at": now,
            "client_name": c.client_name,
            "redirect_uris": c.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": SCOPE,
        }))
    }

    pub fn client(&self, id: &str) -> Option<Client> {
        self.store.lock().unwrap().clients.iter().find(|c| c.client_id == id).cloned()
    }

    /// The resource a request names, canonical: absent is ours; anything but ours (trailing slash aside) is refused.
    fn resource_of(&self, r: Option<&String>) -> Result<String, ()> {
        match r.map(|s| s.trim_end_matches('/')) {
            None | Some("") => Ok(self.resource.clone()),
            Some(x) if x == self.resource => Ok(self.resource.clone()),
            _ => Err(()),
        }
    }

    // ---------- authorize ----------

    /// Validate `GET /oauth/authorize`'s parameters. The client and its redirect URI come first: until both are
    /// known good, an error is a page, never a redirect.
    pub fn authorize_check(&self, q: &HashMap<String, String>) -> Result<(AuthzReq, Client), AuthzErr> {
        let g = |k: &str| q.get(k).map(String::as_str).unwrap_or("");
        let Some(client) = self.client(g("client_id")) else {
            return Err(AuthzErr::Page("This app is not registered here (unknown client_id). Start the connection again from the app.".into()));
        };
        let redirect = g("redirect_uri");
        if !client.redirect_uris.iter().any(|u| u == redirect) {
            return Err(AuthzErr::Page("The redirect_uri is not one this app registered (it must match exactly). Start the connection again from the app.".into()));
        }
        let state = g("state");
        let back = |error: &str, desc: &str| AuthzErr::Redirect(with_query(redirect, &[("error", error), ("error_description", desc), ("state", state), ("iss", &self.issuer)]));
        if g("response_type") != "code" {
            return Err(back("unsupported_response_type", "response_type must be code"));
        }
        let ch = g("code_challenge");
        if ch.is_empty() {
            return Err(back("invalid_request", "code_challenge is required (PKCE)"));
        }
        if g("code_challenge_method") != "S256" {
            return Err(back("invalid_request", "code_challenge_method must be S256"));
        }
        if ch.len() != 43 || !ch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(back("invalid_request", "code_challenge must be base64url(SHA-256(verifier)), 43 characters"));
        }
        if !g("scope").split(' ').all(|s| s.is_empty() || s == SCOPE) {
            return Err(back("invalid_scope", "the only scope is mcp"));
        }
        let Ok(resource) = self.resource_of(q.get("resource")) else {
            return Err(back("invalid_target", &format!("the only resource here is {}", self.resource)));
        };
        if state.len() > 1024 {
            return Err(back("invalid_request", "state is too long"));
        }
        let r = AuthzReq { client_id: client.client_id.clone(), redirect_uri: redirect.to_string(), challenge: ch.to_string(), state: state.to_string(), resource, exp: self.now() + REQ_TTL };
        Ok((r, client))
    }

    /// The request, sealed for the consent page to send back unchanged.
    pub fn seal(&self, r: &AuthzReq) -> String {
        let j = B64.encode(serde_json::to_vec(r).unwrap());
        format!("{j}.{}", self.mac("btw-oauth-req-v1", &j))
    }

    pub fn unseal(&self, s: &str) -> Option<AuthzReq> {
        let (j, m) = s.split_once('.')?;
        if !ct_eq(self.mac("btw-oauth-req-v1", j).as_bytes(), m.as_bytes()) {
            return None;
        }
        let r: AuthzReq = serde_json::from_slice(&B64.decode(j).ok()?).ok()?;
        (self.now() < r.exp).then_some(r)
    }

    /// The consent MAC: this request, approved by this signed-in account (the page can only get it from the server).
    pub fn consent_mac(&self, uid: &str, sealed: &str) -> String {
        self.mac("btw-oauth-consent-v1", &format!("{uid}\n{sealed}"))
    }

    /// The decision: the URL to send the browser to (with a code, or `error=access_denied`).
    pub fn decide(&self, uid: &str, sealed: &str, consent: &str, approve: bool) -> Result<String, OErr> {
        let r = self.unseal(sealed).ok_or_else(|| oerr(400, "invalid_request", "this consent page is stale; start again from the app"))?;
        if !ct_eq(self.consent_mac(uid, sealed).as_bytes(), consent.as_bytes()) {
            return Err(oerr(403, "access_denied", "this consent was not shown to the signed-in account; reload the page"));
        }
        // the client may have been removed since
        let c = self.client(&r.client_id).ok_or_else(|| oerr(400, "invalid_client", "this app is no longer registered"))?;
        if !c.redirect_uris.contains(&r.redirect_uri) {
            return Err(oerr(400, "invalid_request", "the redirect_uri is no longer registered"));
        }
        if !approve {
            return Ok(with_query(&r.redirect_uri, &[("error", "access_denied"), ("error_description", "the user declined"), ("state", &r.state), ("iss", &self.issuer)]));
        }
        let code = rand_b64(32);
        let now = self.now();
        let mut codes = self.codes.lock().unwrap();
        codes.retain(|_, c| c.exp > now);
        if codes.len() > 10_000 {
            return Err(oerr(503, "temporarily_unavailable", "too many sign-ins in flight"));
        }
        codes.insert(sha_hex(&code), Code { client_id: r.client_id, redirect_uri: r.redirect_uri.clone(), challenge: r.challenge, user: uid.to_string(), resource: r.resource, exp: now + CODE_TTL });
        Ok(with_query(&r.redirect_uri, &[("code", &code), ("state", &r.state), ("iss", &self.issuer)]))
    }

    // ---------- token ----------

    fn issue(&self, s: &mut Store, g: Grant, uid: &str) -> Value {
        let access = format!("{ACCESS_PREFIX}{}", rand_b64(32));
        let refresh = format!("{REFRESH_PREFIX}{}", rand_b64(32));
        let now = self.now();
        let mut g = g;
        g.access_hash = sha_hex(&access);
        g.access_exp = now + ACCESS_TTL;
        if !g.refresh_hash.is_empty() {
            g.prev_refresh_hash = std::mem::take(&mut g.refresh_hash);
        }
        g.refresh_hash = sha_hex(&refresh);
        g.refresh_exp = now + REFRESH_TTL;
        g.last_used = now;
        s.grants.retain(|x| x.id != g.id);
        s.grants.push(g.clone());
        // at most MAX_GRANTS_PER_USER: the least recently used go
        let mut mine: Vec<(i64, String)> = s.grants.iter().filter(|x| x.user == uid).map(|x| (x.last_used, x.id.clone())).collect();
        if mine.len() > MAX_GRANTS_PER_USER {
            mine.sort();
            let drop: Vec<String> = mine[..mine.len() - MAX_GRANTS_PER_USER].iter().map(|x| x.1.clone()).collect();
            s.grants.retain(|x| !drop.contains(&x.id));
        }
        self.save(s);
        let mut v = json!({"access_token": access, "token_type": "Bearer", "expires_in": ACCESS_TTL, "refresh_token": refresh, "scope": SCOPE, "resource": g.resource});
        // the identity's capabilities (caps.rs), and the signed grant for a client that wants to show them
        if let Some(cg) = crate::caps::grant_json(uid) {
            v["caps"] = cg["caps"].clone();
            v["cap_grant"] = cg["grant"].clone();
        }
        v
    }

    pub fn token(&self, ip: &str, f: &HashMap<String, String>) -> Result<Value, OErr> {
        let (ok, _) = self.limit_token.lock().unwrap().allow(ip);
        if !ok {
            return Err(oerr(429, "invalid_request", "too many token requests; slow down"));
        }
        let g = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
        match g("grant_type") {
            "authorization_code" => {
                // single use: the code is gone whatever happens next
                let code = self.codes.lock().unwrap().remove(&sha_hex(g("code")));
                let Some(c) = code.filter(|c| c.exp > self.now()) else {
                    return Err(oerr(400, "invalid_grant", "the code is unknown, used or expired"));
                };
                if c.client_id != g("client_id") {
                    return Err(oerr(400, "invalid_grant", "the code was issued to another client"));
                }
                if c.redirect_uri != g("redirect_uri") {
                    return Err(oerr(400, "invalid_grant", "redirect_uri does not match the authorization request"));
                }
                if !pkce_ok(g("code_verifier"), &c.challenge) {
                    return Err(oerr(400, "invalid_grant", "the code_verifier does not match the code_challenge"));
                }
                match self.resource_of(f.get("resource")) {
                    Ok(r) if r == c.resource => {}
                    _ => return Err(oerr(400, "invalid_target", format!("the only resource here is {}", self.resource))),
                }
                let mut s = self.store.lock().unwrap();
                if !s.clients.iter().any(|x| x.client_id == c.client_id) {
                    return Err(oerr(400, "invalid_client", "this client is no longer registered"));
                }
                let now = self.now();
                let gr = Grant {
                    id: format!("g_{}", rand_b64(12)),
                    client_id: c.client_id,
                    user: c.user.clone(),
                    resource: c.resource,
                    redirect_uri: c.redirect_uri,
                    created: now,
                    last_used: now,
                    access_hash: String::new(),
                    access_exp: 0,
                    refresh_hash: String::new(),
                    refresh_exp: 0,
                    prev_refresh_hash: String::new(),
                };
                Ok(self.issue(&mut s, gr, &c.user))
            }
            "refresh_token" => {
                let h = sha_hex(g("refresh_token"));
                let mut s = self.store.lock().unwrap();
                if let Some(i) = s.grants.iter().position(|x| !x.prev_refresh_hash.is_empty() && x.prev_refresh_hash == h) {
                    // a replaced refresh token came back: whoever holds the newer one may not be the user
                    let gone = s.grants.remove(i);
                    self.save(&s);
                    eprintln!("btwserve: oauth: a used refresh token was presented again; revoked grant {}", gone.id);
                    return Err(oerr(400, "invalid_grant", "this refresh token was already used; the connection is revoked, connect again"));
                }
                let Some(gr) = s.grants.iter().find(|x| x.refresh_hash == h).cloned() else {
                    return Err(oerr(400, "invalid_grant", "the refresh token is unknown or revoked"));
                };
                if gr.client_id != g("client_id") {
                    return Err(oerr(400, "invalid_grant", "the refresh token was issued to another client"));
                }
                if gr.refresh_exp <= self.now() {
                    return Err(oerr(400, "invalid_grant", "the refresh token expired; connect again"));
                }
                match self.resource_of(f.get("resource")) {
                    Ok(r) if r == gr.resource => {}
                    _ => return Err(oerr(400, "invalid_target", format!("the only resource here is {}", self.resource))),
                }
                if !g("scope").split(' ').all(|x| x.is_empty() || x == SCOPE) {
                    return Err(oerr(400, "invalid_scope", "the only scope is mcp"));
                }
                let uid = gr.user.clone();
                Ok(self.issue(&mut s, gr, &uid))
            }
            "" => Err(oerr(400, "invalid_request", "grant_type is required")),
            _ => Err(oerr(400, "unsupported_grant_type", "grant_type must be authorization_code or refresh_token")),
        }
    }

    /// RFC 7009: an access or refresh token's whole grant. Unknown tokens are not an error.
    pub fn revoke_token(&self, tok: &str) {
        let h = sha_hex(tok);
        let mut s = self.store.lock().unwrap();
        let before = s.grants.len();
        s.grants.retain(|g| g.access_hash != h && g.refresh_hash != h);
        if s.grants.len() != before {
            self.save(&s);
        }
    }

    /// The account an access token acts as, at `/mcp`: known, not expired, and bound to this resource.
    pub fn access_user(&self, tok: &str) -> Option<String> {
        if !tok.starts_with(ACCESS_PREFIX) {
            return None;
        }
        let h = sha_hex(tok);
        let now = self.now();
        let mut s = self.store.lock().unwrap();
        let g = s.grants.iter_mut().find(|g| ct_eq(g.access_hash.as_bytes(), h.as_bytes()))?;
        if g.access_exp <= now || g.resource != self.resource {
            return None;
        }
        // last_used is for the panel: kept in memory, written with the next change
        g.last_used = now;
        Some(g.user.clone())
    }

    // ---------- the panel ----------

    pub fn list(&self, uid: &str) -> Vec<Value> {
        let s = self.store.lock().unwrap();
        let mut out: Vec<&Grant> = s.grants.iter().filter(|g| g.user == uid).collect();
        out.sort_by_key(|g| std::cmp::Reverse(g.last_used));
        out.iter()
            .map(|g| {
                let name = s.clients.iter().find(|c| c.client_id == g.client_id).map_or("a removed client", |c| c.client_name.as_str());
                json!({"id": g.id, "client_name": name, "client_id": g.client_id, "redirect_uri": g.redirect_uri, "created": g.created, "last_used": g.last_used})
            })
            .collect()
    }

    pub fn revoke_grant(&self, uid: &str, id: &str) -> bool {
        let mut s = self.store.lock().unwrap();
        let before = s.grants.len();
        s.grants.retain(|g| !(g.user == uid && g.id == id));
        let hit = s.grants.len() != before;
        if hit {
            self.save(&s);
        }
        hit
    }

    pub fn revoke_user(&self, uid: &str) -> usize {
        let mut s = self.store.lock().unwrap();
        let before = s.grants.len();
        s.grants.retain(|g| g.user != uid);
        let n = before - s.grants.len();
        if n > 0 {
            self.save(&s);
        }
        n
    }
}

// ---------- HTTP ----------

static OAUTH: std::sync::OnceLock<Arc<OAuth>> = std::sync::OnceLock::new();

pub fn get() -> Option<Arc<OAuth>> {
    OAUTH.get().cloned()
}

/// `WWW-Authenticate` for a 401 from `/mcp` (RFC 9728 §5.1).
pub fn www_authenticate(error: Option<&str>) -> String {
    let meta = get().map(|o| o.resource_metadata_url()).unwrap_or_default();
    match error {
        Some(e) => format!("Bearer resource_metadata=\"{meta}\", scope=\"{SCOPE}\", error=\"{e}\""),
        None => format!("Bearer resource_metadata=\"{meta}\", scope=\"{SCOPE}\""),
    }
}

fn cors(h: &mut HeaderMap) {
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, OPTIONS"));
    h.insert("access-control-allow-headers", HeaderValue::from_static("Content-Type, Authorization, MCP-Protocol-Version"));
}

fn json_resp(status: u16, v: &Value, with_cors: bool) -> Response<Body> {
    let mut r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(v.to_string() + "\n"))
        .unwrap();
    if with_cors {
        cors(r.headers_mut());
    }
    r
}

fn oerr_resp(e: OErr) -> Response<Body> {
    json_resp(e.status, &json!({"error": e.error, "error_description": e.desc}), true)
}

fn ip_of(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

async fn body_of(req: Request, max: usize) -> Option<Vec<u8>> {
    use futures_util::StreamExt;
    let mut s = req.into_body().into_data_stream();
    let mut out = vec![];
    while let Some(c) = s.next().await {
        out.extend_from_slice(&c.ok()?);
        if out.len() > max {
            return None;
        }
    }
    Some(out)
}

const PAGE_JS: &str = include_str!("oauth/authorize.js");
const PAGE_CSS: &str = include_str!("oauth/authorize.css");

fn page(title: &str, inner: &str) -> Response<Body> {
    let esc = crate::site::html_escape;
    let html = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{} · BeatScript</title><link rel=\"stylesheet\" href=\"/oauth/authorize.css\"><meta name=\"robots\" content=\"noindex\"></head>\
<body><main>{inner}</main>{}<script type=\"module\" src=\"/oauth/authorize.js\"></script></body></html>\n",
        esc(title),
        btwserve::rights::FOOTER
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("referrer-policy", "no-referrer")
        .body(Body::from(html))
        .unwrap()
}

fn short_id(uid: &str) -> String {
    let (kind, a) = uid.split_once(':').unwrap_or(("", uid));
    let a = a.trim_start_matches("0x");
    format!("{} {}…{}", if kind == "eip155" { "wallet" } else { "browser key" }, &a[..6.min(a.len())], &a[a.len().saturating_sub(4)..])
}

async fn authorize_get(app: &AppState, o: &OAuth, req: Request) -> Response<Body> {
    let esc = crate::site::html_escape;
    let q = match parse_form(req.uri().query().unwrap_or("")) {
        Ok(q) => q,
        Err(e) => return page("Cannot connect", &format!("<h1>Cannot connect</h1><p class=\"err\">The request is malformed: {}.</p>", esc(&e))),
    };
    let (r, client) = match o.authorize_check(&q) {
        Ok(x) => x,
        Err(AuthzErr::Page(m)) => return page("Cannot connect", &format!("<h1>Cannot connect</h1><p class=\"err\">{}</p>", esc(&m))),
        Err(AuthzErr::Redirect(u)) => return Response::builder().status(302).header(header::LOCATION, u).body(Body::empty()).unwrap(),
    };
    let sealed = o.seal(&r);
    let user = app.authsvc.get().and_then(|a| a.user(req.headers()).ok().flatten()).filter(|u| u.via == "cookie");
    let who = match &user {
        Some(u) => format!(
            "<p>Signed in as <b>{}</b>.</p><div class=\"row\"><button id=\"approve\" class=\"primary\" data-req=\"{}\" data-consent=\"{}\">Allow</button><button id=\"deny\" data-req=\"{}\" data-consent=\"{}\">Deny</button></div>",
            esc(&short_id(&u.id)),
            esc(&sealed),
            esc(&o.consent_mac(&u.id, &sealed)),
            esc(&sealed),
            esc(&o.consent_mac(&u.id, &sealed))
        ),
        None => "<p>Sign in first, with the key this browser keeps for beatscript.app (or a wallet).</p><div class=\"row\"><button id=\"signin-key\" class=\"primary\">Sign in with this browser's key</button><button id=\"signin-eth\" hidden>Sign in with a wallet</button></div>".to_string(),
    };
    let inner = format!(
        "<h1>Connect {name} to BeatScript</h1>\
<p><b>{name}</b> is asking to use BeatScript's MCP tools as you: write and check programs, store them, and drive your open tabs and paired phone (eval, load, get, stop, devices, queue).</p>\
<p>After you allow it, you are sent back to:</p><pre>{uri}</pre>\
<p class=\"warn\">Only allow this if you just started connecting from that app. You can disconnect it at any time in the MCP panel.</p>\
{who}<p id=\"status\" role=\"status\"></p>",
        name = esc(&client.client_name),
        uri = esc(&r.redirect_uri),
    );
    page(&format!("Connect {}", client.client_name), &inner)
}

async fn authorize_post(app: &AppState, o: &OAuth, req: Request) -> Response<Body> {
    let Some(a) = app.authsvc.get() else { return oerr_resp(oerr(503, "temporarily_unavailable", "sign-in is not set up")) };
    if a.cross_site(req.headers()) {
        return oerr_resp(oerr(403, "access_denied", "this request came from another site"));
    }
    // the session cookie only: a bearer token cannot approve a connection
    let Some(u) = a.user(req.headers()).ok().flatten().filter(|u| u.via == "cookie") else {
        return oerr_resp(oerr(401, "access_denied", "sign in on this page first"));
    };
    let Some(b) = body_of(req, 16 << 10).await else { return oerr_resp(oerr(413, "invalid_request", "body too large")) };
    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    match o.decide(&u.id, s("req"), s("consent"), v.get("approve").and_then(Value::as_bool).unwrap_or(false)) {
        Ok(url) => json_resp(200, &json!({"redirect": url}), false),
        Err(e) => oerr_resp(e),
    }
}

async fn handle(State((app, o)): State<(AppState, Arc<OAuth>)>, req: Request) -> Response<Body> {
    let m = req.method().clone();
    let path = req.uri().path().to_string();
    let get = m == Method::GET || m == Method::HEAD;
    if m == Method::OPTIONS && (path.starts_with("/.well-known/") || matches!(path.as_str(), "/oauth/token" | "/oauth/register" | "/oauth/revoke")) {
        let mut r = Response::builder().status(204).body(Body::empty()).unwrap();
        cors(r.headers_mut());
        r.headers_mut().insert("access-control-max-age", HeaderValue::from_static("86400"));
        return r;
    }
    match path.as_str() {
        "/.well-known/oauth-protected-resource" | "/.well-known/oauth-protected-resource/mcp" if get => json_resp(200, &o.protected_resource(), true),
        "/.well-known/oauth-authorization-server" if get => json_resp(200, &o.server_metadata(), true),
        "/.well-known/btw-capabilities.json" if get => match crate::caps::get() {
            Some(c) => {
                let mut r = json_resp(200, &c.jwks(), true);
                r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
                r
            }
            None => oerr_resp(oerr(503, "temporarily_unavailable", "capabilities are not set up")),
        },
        "/oauth/authorize.js" if get => Response::builder().header(header::CONTENT_TYPE, "text/javascript; charset=utf-8").header(header::CACHE_CONTROL, "no-cache").body(Body::from(PAGE_JS)).unwrap(),
        "/oauth/authorize.css" if get => Response::builder().header(header::CONTENT_TYPE, "text/css; charset=utf-8").header(header::CACHE_CONTROL, "no-cache").body(Body::from(PAGE_CSS)).unwrap(),
        "/oauth/authorize" if get => authorize_get(&app, &o, req).await,
        "/oauth/authorize" if m == Method::POST => authorize_post(&app, &o, req).await,
        "/oauth/register" if m == Method::POST => {
            let ip = ip_of(&req);
            let Some(b) = body_of(req, 16 << 10).await else { return oerr_resp(oerr(413, "invalid_client_metadata", "body too large")) };
            match o.register(&ip, &b) {
                Ok(v) => json_resp(201, &v, true),
                Err(e) => oerr_resp(e),
            }
        }
        "/oauth/token" | "/oauth/revoke" if m == Method::POST => {
            let ip = ip_of(&req);
            let ct = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
            if !ct.starts_with("application/x-www-form-urlencoded") {
                return oerr_resp(oerr(400, "invalid_request", "send the body as application/x-www-form-urlencoded"));
            }
            let Some(b) = body_of(req, 16 << 10).await else { return oerr_resp(oerr(413, "invalid_request", "body too large")) };
            let f = match parse_form(&String::from_utf8_lossy(&b)) {
                Ok(f) => f,
                Err(e) => return oerr_resp(oerr(400, "invalid_request", e)),
            };
            if path == "/oauth/revoke" {
                o.revoke_token(f.get("token").map(String::as_str).unwrap_or(""));
                return json_resp(200, &json!({}), true);
            }
            match o.token(&ip, &f) {
                Ok(v) => json_resp(200, &v, true),
                Err(e) => oerr_resp(e),
            }
        }
        p if p == "/api/auth/clients" || p.starts_with("/api/auth/clients/") => clients(&app, &o, req).await,
        _ => app.site.serve(req.method(), req.uri().path(), req.headers()),
    }
}

/// The MCP panel: `GET /api/auth/clients` lists the caller's connections and pasted tokens; `DELETE
/// /api/auth/clients/<id>` disconnects one.
async fn clients(app: &AppState, o: &OAuth, req: Request) -> Response<Body> {
    let Some(a) = app.authsvc.get() else { return oerr_resp(oerr(503, "temporarily_unavailable", "sign-in is not set up")) };
    let u = match a.require_user(req.headers()) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let path = req.uri().path();
    if req.method() == Method::GET && path == "/api/auth/clients" {
        return json_resp(200, &json!({"clients": o.list(&u.id), "tokens": a.tokens_of(&u.id)}), false);
    }
    if req.method() == Method::DELETE {
        if a.cross_site(req.headers()) {
            return json_resp(403, &json!({"error": {"code": "cross_site", "message": "this request came from another site"}}), false);
        }
        if let Some(id) = path.strip_prefix("/api/auth/clients/") {
            return if o.revoke_grant(&u.id, id) { json_resp(200, &json!({"revoked": 1}), false) } else { json_resp(404, &json!({"error": {"code": "not_found", "message": "no such connection on this account"}}), false) };
        }
    }
    json_resp(405, &json!({"error": {"code": "bad_request", "message": "GET /api/auth/clients or DELETE /api/auth/clients/<id>"}}), false)
}

pub fn routes(app: &AppState) -> Router {
    let env = std::env::var("BTW_SESSION_SECRET").map(|s| s.trim().to_string()).unwrap_or_default();
    let secret = crate::authsvc::load_secret(&app.cfg.data, &env).unwrap_or_else(|e| {
        eprintln!("btwserve: oauth: {e}");
        std::process::exit(1);
    });
    let o = Arc::new(OAuth::new(app.cfg.data.clone(), &app.cfg.public_url, secret));
    let _ = OAUTH.set(o.clone());
    let mut r = Router::new();
    for p in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-authorization-server",
        "/.well-known/btw-capabilities.json",
        "/oauth/authorize",
        "/oauth/authorize.js",
        "/oauth/authorize.css",
        "/oauth/register",
        "/oauth/token",
        "/oauth/revoke",
        "/api/auth/clients",
        "/api/auth/clients/{id}",
    ] {
        r = r.route(p, any(handle));
    }
    r.with_state((app.clone(), o))
}

#[cfg(test)]
mod tests;
