//! P4: `/api/auth/*`, Go's `internal/btwsite/auth` (auth.go) and its rate-limit hook (server.go `authAllow`).
//!
//! | route | what it does |
//! |---|---|
//! | `GET /api/auth/challenge` | a single-use nonce and the text to sign (`<public host> sign-in\nnonce: …\nissued: …\nexpires: …`), 5 min |
//! | `POST /api/auth/login` | `{type: eth\|ed25519, address, signature, nonce}`: EIP-191 personal_sign (secp256k1 recovery) or raw Ed25519 over that text; sets `btw_session` |
//! | `POST /api/auth/logout` | clears the cookie |
//! | `GET /api/auth/me` | `{"user":…}` or `{"user":null}` (a bad cookie is cleared) |
//! | `POST /api/auth/token` | mints `btw_<43>` for the cookie's user (a token cannot mint one); at most 5 per user, the oldest dropped |
//! | `DELETE /api/auth/token` | revokes every token of the caller (cookie or bearer) |
//!
//! Files (shared with the Go server, same shapes): `<data>/session.secret` (hex, 0600, made at start when missing,
//! as `auth.LoadSecret`; `BEATSCRIPT_SESSION_SECRET` wins) and `<data>/tokens.json` (`[{hash,user,created}]`, SHA-256 of
//! the token only, 0600, written by rename). The verifier (`beatscript_serve::playlists::auth::Auth`, `AuthSvc::verify`)
//! re-reads tokens.json when its size, mtime or inode changes, so a mint or revoke here is seen by the next request,
//! by every `Auth` in the process.
//!
//! Cross-site refusal (Go's `crossSite`): an `Origin` must be the request's `Host`, the `-public-url` host or an
//! `-alt-origins` host (not `-legacy-hosts`: those are for phones, as in Go); with no Origin, `Sec-Fetch-Site:
//! cross-site|same-site` is refused. Per-address limits: challenge 30/min, login 20/min, token 10/min.
//!
//! For the other phases: `caller` (cookie, bearer, `BtwDevice`), `user` (Go's `UserFrom`: cookie, bearer),
//! `require_user` / `require_caller` (Go's `requireUser`: the 401 answer, ready to return), `user_from_token` (MCP).

// the verifier API (caller, user, require_*, user_from_token, auth_help) is for the other phases' routes
#![allow(dead_code)]

mod gojson;
pub mod sig;

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use axum::routing::any;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use beatscript_serve::playlists::auth::{cookie_value, Auth, AuthConfig, AuthError, Caller, COOKIE_NAME, TOKEN_PREFIX};
use beatscript_serve::playlists::Limiter;

pub use sig::User;

use crate::{ApiError, AppState};

const COOKIE_TTL: u64 = 7 * 24 * 3600;
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_TOKENS_PER_USER: usize = 5;
const MAX_CHALLENGES: usize = 20000;
const LOGIN_MAX_BODY: usize = 4096;

/// What `AuthSvc` is built from (main reads it from the flags; tests set it directly).
pub struct Options {
    pub data: PathBuf,
    /// `BEATSCRIPT_SESSION_SECRET`, trimmed (Go's `cfg.SessionSecret`); empty = the file.
    pub secret_env: String,
    pub public_url: String,
    pub alt_origins: String,
    pub legacy_hosts: String,
    /// The clock (tests move it).
    pub now: Box<dyn Fn() -> SystemTime + Send + Sync>,
}

/// The auth service: challenge nonces, login, logout, tokens (writes session.secret, tokens.json), and the shared
/// verifier every phase asks who is calling.
pub struct AuthSvc {
    pub verify: Auth,
    secret: Vec<u8>,
    data: PathBuf,
    /// Go's `s.domain`: the `-public-url` host as written (port included), "localhost" without one. The challenge
    /// names it.
    domain: String,
    alt_hosts: Vec<String>,
    secure: bool,
    public_url: String,
    now: Box<dyn Fn() -> SystemTime + Send + Sync>,
    challenges: Mutex<HashMap<String, (String, SystemTime)>>,
    tokens: Mutex<()>,
    limit_challenge: Mutex<Limiter>,
    limit_login: Mutex<Limiter>,
    limit_token: Mutex<Limiter>,
}

// ---------- time, as Go formats it ----------

fn civil(s: i64) -> (i64, i64, i64, i64) {
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d, rem)
}

fn unix(t: SystemTime) -> (i64, u32) {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(_) => (0, 0),
    }
}

/// `t.UTC().Format(time.RFC3339)`.
pub fn rfc3339(t: SystemTime) -> String {
    let (y, m, d, r) = civil(unix(t).0);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", r / 3600, r % 3600 / 60, r % 60)
}

/// `t.UTC().Format(time.RFC3339Nano)` (how encoding/json writes a `time.Time`): the fraction without trailing zeros.
pub fn rfc3339_nano(t: SystemTime) -> String {
    let (_, n) = unix(t);
    let base = rfc3339(t);
    if n == 0 {
        return base;
    }
    let frac = format!("{n:09}");
    format!("{}.{}Z", &base[..base.len() - 1], frac.trim_end_matches('0'))
}

/// `http.TimeFormat` (a cookie's Expires).
fn http_date(t: SystemTime) -> String {
    let s = unix(t).0;
    let (y, m, d, r) = civil(s);
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let wd = DAYS[s.div_euclid(86_400).rem_euclid(7) as usize];
    format!("{wd}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT", MONTHS[(m - 1) as usize], r / 3600, r % 3600 / 60, r % 60)
}

/// An RFC 3339 time as (unix seconds, nanoseconds), for ordering tokens by `created`. None when it is not one.
fn parse_rfc3339(s: &str) -> Option<(i64, u32)> {
    let b = s.as_bytes();
    let num = |a: usize, n: usize| -> Option<i64> { std::str::from_utf8(b.get(a..a + n)?).ok()?.parse().ok() };
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut i = 19;
    let mut nanos = 0u32;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let f = &s[start..i];
        nanos = format!("{:0<9}", &f[..f.len().min(9)]).parse().ok()?;
    }
    let off = match b.get(i)? {
        b'Z' | b'z' => 0,
        b'+' | b'-' => {
            let o = num(i + 1, 2)? * 3600 + num(i + 4, 2)? * 60;
            if b[i] == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    // days from civil
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + se - off, nanos))
}

// ---------- the secret ----------

/// Go's `auth.LoadSecret(dataDir, fromEnv)`: the env value when set (16+ characters), else the hex key in
/// `<data>/session.secret`, made (32 random bytes, hex + newline, 0600) when the file does not exist. A file that
/// is not a hex key of 16+ bytes is an error, never replaced.
pub fn load_secret(data: &Path, from_env: &str) -> Result<Vec<u8>, String> {
    if !from_env.is_empty() {
        if from_env.len() < 16 {
            return Err("BEATSCRIPT_SESSION_SECRET must be at least 16 characters".into());
        }
        return Ok(from_env.as_bytes().to_vec());
    }
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let p = data.join("session.secret");
    match std::fs::read(&p) {
        Ok(b) => {
            let raw = std::str::from_utf8(&b).ok().and_then(|s| hex::decode(s.trim()).ok());
            match raw {
                Some(r) if r.len() >= 16 => Ok(r),
                _ => Err(format!("{} is not a hex key of 16+ bytes; delete it to generate a new one", p.display())),
            }
        }
        Err(_) => {
            let mut raw = vec![0u8; 32];
            getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p).map_err(|e| e.to_string())?;
            f.write_all((hex::encode(&raw) + "\n").as_bytes()).map_err(|e| e.to_string())?;
            Ok(raw)
        }
    }
}

fn rand_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the OS has no entropy");
    B64.encode(b)
}

fn hash_token(tok: &str) -> String {
    hex::encode(Sha256::digest(tok.as_bytes()))
}

/// Go's `hostOf` / `url.Parse(o).Host` for an origin: the host (port included), None when it does not parse or has
/// none. Covers what `url.Parse` refuses that an Origin can carry: a control byte, a leading colon, a colon in a
/// scheme-less first segment, a bad port, a byte a host cannot hold.
pub fn origin_host(o: &str) -> Option<String> {
    let o = o.split('#').next().unwrap_or("");
    if o.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }
    // the scheme
    let mut rest = o;
    let mut has_scheme = false;
    for (i, c) in o.char_indices() {
        match c {
            'a'..='z' | 'A'..='Z' => {}
            '0'..='9' | '+' | '-' | '.' if i > 0 => {}
            ':' => {
                if i == 0 {
                    return None; // "missing protocol scheme"
                }
                rest = &o[i + 1..];
                has_scheme = true;
                break;
            }
            _ => break,
        }
    }
    let rest = rest.split('?').next().unwrap_or("");
    let Some(auth) = rest.strip_prefix("//") else {
        if !has_scheme && rest.split('/').next().unwrap_or("").contains(':') {
            return None;
        }
        return None; // no authority: no host
    };
    let auth = auth.split('/').next().unwrap_or("");
    let host = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    let port_ok = |p: &str| p.is_empty() || p.bytes().all(|b| b.is_ascii_digit());
    if let Some(inner) = host.strip_prefix('[') {
        let (_, after) = inner.split_once(']')?;
        if !after.is_empty() && !after.strip_prefix(':').is_some_and(port_ok) {
            return None;
        }
    } else {
        if let Some((_, p)) = host.rsplit_once(':') {
            if !port_ok(p) {
                return None;
            }
        }
        let bad = |b: u8| b < 0x80 && !(b.is_ascii_alphanumeric() || b"-_.~!$&'()*+,;=:[]<>\"%".contains(&b));
        if host.bytes().any(bad) {
            return None;
        }
    }
    (!host.is_empty()).then(|| host.to_string())
}

fn alt_hosts(list: &str) -> Vec<String> {
    list.split(',')
        .map(|p| p.trim().trim_end_matches('/'))
        .filter(|p| !p.is_empty() && (p.starts_with("https:") || p.starts_with("http:")))
        .filter_map(origin_host)
        .map(|h| h.to_ascii_lowercase())
        .collect()
}

// ---------- tokens.json ----------

#[derive(Clone, Debug)]
struct TokenRec {
    hash: String,
    user: String,
    created: String,
}

impl AuthSvc {
    pub fn new(app: &AppState) -> AuthSvc {
        let cfg = &app.cfg;
        let o = Options {
            data: cfg.data.clone(),
            secret_env: beatscript_serve::env_var("SESSION_SECRET").map(|s| s.trim().to_string()).unwrap_or_default(),
            public_url: cfg.public_url.clone(),
            alt_origins: cfg.alt_origins.clone(),
            legacy_hosts: cfg.legacy_hosts.clone(),
            now: Box::new(SystemTime::now),
        };
        match AuthSvc::with(o) {
            Ok(s) => s,
            Err(e) => {
                // as the Go server: no key, no start
                eprintln!("beatscript-serve: auth: {e}");
                std::process::exit(1);
            }
        }
    }

    pub fn with(o: Options) -> Result<AuthSvc, String> {
        let secret = load_secret(&o.data, &o.secret_env)?;
        let secret_env = (!o.secret_env.is_empty()).then(|| o.secret_env.clone());
        let hosts = beatscript_serve::playlists::Config::hosts_from(&o.public_url, &o.alt_origins, &o.legacy_hosts);
        let domain = o.public_url.split_once("://").and_then(|_| origin_host(&o.public_url)).unwrap_or_else(|| "localhost".into());
        let min = Duration::from_secs(60);
        Ok(AuthSvc {
            verify: Auth::new(AuthConfig { data: o.data.clone(), secret_env, hosts }),
            secret,
            data: o.data,
            domain,
            alt_hosts: alt_hosts(&o.alt_origins),
            secure: o.public_url.starts_with("https://"),
            public_url: o.public_url,
            now: o.now,
            challenges: Mutex::new(HashMap::new()),
            tokens: Mutex::new(()),
            limit_challenge: Mutex::new(Limiter::new(30, min)),
            limit_login: Mutex::new(Limiter::new(20, min)),
            limit_token: Mutex::new(Limiter::new(10, min)),
        })
    }

    // ---------- the verifier, for every phase ----------

    /// Cookie, bearer token or `BtwDevice` signature. `Ok(None)`: anonymous. `body` is the request body (a device
    /// signature covers it; pass `b""` for a route without one).
    pub fn caller(&self, h: &HeaderMap, method: &str, path: &str, body: &[u8]) -> Result<Option<Caller>, AuthError> {
        let cookie = cookie_value(h.get_all("cookie").iter().filter_map(|v| v.to_str().ok()), "btw_session");
        let authz = h.get("authorization").and_then(|v| v.to_str().ok());
        self.verify.caller(cookie, authz, method, path, body)
    }

    /// Go's `UserFrom`: the session cookie, then `Authorization: Bearer btw_…` (no phones). `Ok(None)`: anonymous;
    /// an error only for a credential of ours that is not good.
    pub fn user(&self, h: &HeaderMap) -> Result<Option<User>, AuthError> {
        let cookie = cookie_value(h.get_all("cookie").iter().filter_map(|v| v.to_str().ok()), COOKIE_NAME);
        let authz = h.get("authorization").and_then(|v| v.to_str().ok()).filter(|a| !a.starts_with("BtwDevice "));
        let c = self.verify.caller(cookie, authz, "", "", b"")?;
        Ok(c.and_then(|c| User::from_id(&c.uid, if c.via == beatscript_serve::playlists::auth::Via::Token { "token" } else { "cookie" })))
    }

    /// Go's `UserFromToken` (MCP): the user of a bearer token, None when it is not one.
    pub fn user_from_token(&self, tok: &str) -> Option<User> {
        self.verify.user_from_token(tok).and_then(|uid| User::from_id(&uid, "token"))
    }

    /// Go's `AuthHelp`.
    pub fn auth_help(&self) -> String {
        format!("sign in at {} (wallet or browser key), press \"MCP token\", and send it as `Authorization: Bearer btw_…`", self.public_url)
    }

    fn refusal(&self, e: Option<AuthError>) -> Response<Body> {
        match e {
            Some(e) => ApiError::new(e.status, e.code, e.message).response(),
            None => ApiError::new(401, "auth_required", format!("authentication required: {}", self.auth_help())).response(),
        }
    }

    /// Go's `requireUser`: the user, or the 401 answer to send as it is (the credential's error, else
    /// `authentication required: <AuthHelp>`).
    pub fn require_user(&self, h: &HeaderMap) -> Result<User, Response<Body>> {
        match self.user(h) {
            Ok(Some(u)) => Ok(u),
            Ok(None) => Err(self.refusal(None)),
            Err(e) => Err(self.refusal(Some(e))),
        }
    }

    /// `require_user` that also takes a paired phone (`BtwDevice`; a phone no longer paired is a 410).
    pub fn require_caller(&self, h: &HeaderMap, method: &str, path: &str, body: &[u8]) -> Result<Caller, Response<Body>> {
        match self.caller(h, method, path, body) {
            Ok(Some(c)) => Ok(c),
            Ok(None) => Err(self.refusal(None)),
            Err(e) => Err(self.refusal(Some(e))),
        }
    }

    // ---------- the pieces of the handlers ----------

    fn now(&self) -> SystemTime {
        (self.now)()
    }

    /// Go's `crossSite`.
    pub fn cross_site(&self, h: &HeaderMap) -> bool {
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok());
        if let Some(o) = h.get("origin").filter(|v| !v.is_empty()) {
            let Some(host) = o.to_str().ok().and_then(origin_host) else { return true };
            let host = host.to_ascii_lowercase();
            if get("host").is_some_and(|x| x.to_ascii_lowercase() == host) || host == self.domain.to_ascii_lowercase() {
                return false;
            }
            return !self.alt_hosts.contains(&host);
        }
        matches!(get("sec-fetch-site"), Some("cross-site") | Some("same-site"))
    }

    fn refuse_cross_site(&self, h: &HeaderMap) -> Option<Response<Body>> {
        self.cross_site(h).then(|| ApiError::new(403, "cross_site", "this request came from another site; sign-in and tokens are only for this site's own page").response())
    }

    fn allow(&self, bucket: &str, ip: &str) -> Option<Response<Body>> {
        let l = match bucket {
            "challenge" => &self.limit_challenge,
            "login" => &self.limit_login,
            _ => &self.limit_token,
        };
        let (ok, retry) = l.lock().unwrap().allow(ip);
        (!ok).then(|| ApiError::new(429, "rate_limited", format!("too many {bucket} requests; slow down")).retry(retry.max(1)).response())
    }

    fn cookie_header(&self, value: &str, exp: SystemTime, max_age: i64) -> HeaderValue {
        let ma = if max_age > 0 { format!("; Max-Age={max_age}") } else { "; Max-Age=0".into() };
        let s = format!("{COOKIE_NAME}={value}; Path=/; Expires={}{ma}; HttpOnly{}; SameSite=Lax", http_date(exp), if self.secure { "; Secure" } else { "" });
        HeaderValue::from_str(&s).unwrap()
    }

    fn set_cookie(&self, uid: &str) -> HeaderValue {
        let exp = self.now() + Duration::from_secs(COOKIE_TTL);
        self.cookie_header(&Auth::make_cookie(&self.secret, uid, unix(exp).0), exp, COOKIE_TTL as i64)
    }

    fn clear_cookie(&self) -> HeaderValue {
        self.cookie_header("", UNIX_EPOCH, -1)
    }

    /// The `btw_session` cookie is present (Go's `r.Cookie(CookieName)` without an error), and its value.
    fn session_cookie<'a>(&self, h: &'a HeaderMap) -> Option<&'a str> {
        cookie_value(h.get_all("cookie").iter().filter_map(|v| v.to_str().ok()), COOKIE_NAME)
    }

    fn cookie_user(&self, h: &HeaderMap) -> Option<User> {
        let v = self.session_cookie(h)?;
        let uid = self.verify.parse_cookie(v, unix(self.now()).0).ok()?;
        User::from_id(&uid, "cookie")
    }

    fn load_tokens(&self) -> Result<Vec<TokenRec>, String> {
        let p = self.data.join("tokens.json");
        let b = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let v: Vec<Value> = serde_json::from_slice(&b).map_err(|e| format!("{} is not valid JSON: {e}", p.display()))?;
        Ok(v.iter()
            .map(|r| {
                let s = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                TokenRec { hash: s("hash"), user: s("user"), created: s("created") }
            })
            .collect())
    }

    /// Go's `saveLocked`: `json.MarshalIndent(recs, "", " ")` to a `.tokens-*` file in the same directory, 0600,
    /// renamed over tokens.json.
    fn save_tokens(&self, recs: &[TokenRec]) -> std::io::Result<()> {
        let mut out = String::new();
        if recs.is_empty() {
            out.push_str("[]");
        } else {
            out.push_str("[\n");
            for (i, r) in recs.iter().enumerate() {
                let q = |s: &str| crate::go_json(&json!(s));
                out.push_str(&format!(" {{\n  \"hash\": {},\n  \"user\": {},\n  \"created\": {}\n }}", q(&r.hash), q(&r.user), q(&r.created)));
                out.push_str(if i + 1 < recs.len() { ",\n" } else { "\n" });
            }
            out.push(']');
        }
        let tmp = self.data.join(format!(".tokens-{}", rand_b64(9).replace(['-', '_'], "0")));
        let res = (|| {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            f.write_all(out.as_bytes())?;
            drop(f);
            std::fs::rename(&tmp, self.data.join("tokens.json"))
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    /// The pasted tokens of a user, for the MCP panel: when each was made (the token itself is never kept).
    pub fn tokens_of(&self, uid: &str) -> Vec<Value> {
        self.load_tokens().unwrap_or_default().iter().filter(|t| t.user == uid).map(|t| json!({"created": t.created})).collect()
    }

    /// Tokens of a user in tokens.json (Go's `TokenCount`; tests).
    pub fn token_count(&self, uid: &str) -> usize {
        self.load_tokens().unwrap_or_default().iter().filter(|t| t.user == uid).count()
    }

    // ---------- the handlers, transport-free ----------

    pub fn challenge(&self, ip: &str) -> Reply {
        if let Some(r) = self.allow("challenge", ip) {
            return Reply::Raw(r);
        }
        let now = self.now();
        let exp = now + CHALLENGE_TTL;
        let nonce = rand_b64(24);
        let msg = format!("{} sign-in\nnonce: {nonce}\nissued: {}\nexpires: {}", self.domain, rfc3339(now), rfc3339(exp));
        {
            let mut c = self.challenges.lock().unwrap();
            if c.len() >= MAX_CHALLENGES {
                c.retain(|_, (_, e)| now <= *e);
                if c.len() >= MAX_CHALLENGES {
                    return Reply::Raw(ApiError::new(503, "busy", "too many sign-ins in flight; try again shortly").retry(30).response());
                }
            }
            c.insert(nonce.clone(), (msg.clone(), exp));
        }
        Reply::json(json!({"expires": rfc3339(exp), "message": msg, "nonce": nonce}))
    }

    pub fn login(&self, h: &HeaderMap, ip: &str, body: &[u8]) -> Reply {
        if let Some(r) = self.refuse_cross_site(h) {
            return Reply::Raw(r);
        }
        if let Some(r) = self.allow("login", ip) {
            return Reply::Raw(r);
        }
        let f = match gojson::decode_strings(body, LOGIN_MAX_BODY, "auth.loginReq", &["type", "address", "signature", "nonce"]) {
            Ok(f) => f,
            Err(gojson::DecodeError::TooLarge) => {
                let mut r = ApiError::new(413, "too_large", format!("request body over {LOGIN_MAX_BODY} bytes")).response();
                // Go's status text for 413 (hyper's is RFC 9110's "Content Too Large"/"Payload Too Large")
                r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Request Entity Too Large"));
                return Reply::Raw(r);
            }
            Err(gojson::DecodeError::Bad(m)) => return Reply::err(400, "bad_request", format!("body must be JSON: {m}")),
        };
        let [typ, address, signature, nonce] = [0, 1, 2, 3].map(|i| f[i].clone().unwrap_or_default());
        let mut user = match sig::identity(&typ, &address) {
            Ok(u) => u,
            Err(e) => return Reply::err(400, "bad_request", e),
        };
        let nonce_ok = (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !nonce_ok {
            return Reply::err(400, "bad_request", "nonce is missing or malformed");
        }
        let sig_hex = signature.strip_prefix("0x").unwrap_or(&signature);
        if signature.len() < 100 || signature.len() > 140 || sig_hex.is_empty() || !sig_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Reply::err(400, "bad_request", "signature must be hex (eth: 65 bytes, ed25519: 64)");
        }
        // single use: the nonce is gone whether or not the signature turns out good
        let ch = self.challenges.lock().unwrap().remove(&nonce);
        let Some((msg, exp)) = ch.filter(|(_, e)| self.now() <= *e) else {
            return Reply::err(401, "challenge_stale", "that challenge is unknown, used or expired; ask for a new one");
        };
        if !sig::verify_personal(msg.as_bytes(), &typ, &user.address, &signature) {
            return Reply::err(401, "bad_signature", "the signature does not verify for that address");
        }
        let _ = exp;
        user.via = "cookie".into();
        Reply::Json(json!({"user": user.json()}), vec![self.set_cookie(&user.id)])
    }

    pub fn logout(&self, h: &HeaderMap) -> Reply {
        if let Some(r) = self.refuse_cross_site(h) {
            return Reply::Raw(r);
        }
        Reply::Json(json!({"ok": true}), vec![self.clear_cookie()])
    }

    pub fn me(&self, h: &HeaderMap) -> Reply {
        match self.user(h) {
            Ok(Some(u)) => Reply::json(json!({"user": u.json()})),
            Ok(None) => Reply::json(json!({"user": null})),
            // a stale or forged credential is just "not signed in" here
            Err(_) => Reply::Json(json!({"user": null}), self.session_cookie(h).map(|_| self.clear_cookie()).into_iter().collect()),
        }
    }

    pub fn token_create(&self, h: &HeaderMap, ip: &str) -> Reply {
        if let Some(r) = self.refuse_cross_site(h) {
            return Reply::Raw(r);
        }
        if let Some(r) = self.allow("token", ip) {
            return Reply::Raw(r);
        }
        // the cookie is required: a token cannot be used to mint more tokens
        let Some(u) = self.cookie_user(h) else {
            return Reply::err(401, "auth_required", "sign in (with the session cookie) to create a token");
        };
        let tok = format!("{TOKEN_PREFIX}{}", rand_b64(32));
        let rec = TokenRec { hash: hash_token(&tok), user: u.id.clone(), created: rfc3339_nano(self.now()) };
        let _g = self.tokens.lock().unwrap();
        let res = self.load_tokens().map_err(std::io::Error::other).and_then(|mut recs| {
            recs.retain(|t| t.hash != rec.hash);
            recs.push(rec);
            loop {
                let mine: Vec<usize> = (0..recs.len()).filter(|&i| recs[i].user == u.id).collect();
                if mine.len() <= MAX_TOKENS_PER_USER {
                    break;
                }
                let key = |i: usize| parse_rfc3339(&recs[i].created).unwrap_or((i64::MIN, 0));
                let oldest = mine.iter().copied().fold(mine[0], |o, i| if key(i) < key(o) { i } else { o });
                recs.remove(oldest);
            }
            self.save_tokens(&recs)
        });
        if let Err(e) = res {
            eprintln!("beatscript-serve: auth: could not store the token: {e}");
            return Reply::err(500, "internal", "could not store the token; try again later");
        }
        Reply::json(json!({"max_per_user": MAX_TOKENS_PER_USER, "token": tok}))
    }

    pub fn token_revoke(&self, h: &HeaderMap) -> Reply {
        if let Some(r) = self.refuse_cross_site(h) {
            return Reply::Raw(r);
        }
        let Ok(Some(u)) = self.user(h) else {
            return Reply::err(401, "auth_required", "sign in to revoke your tokens");
        };
        let _g = self.tokens.lock().unwrap();
        let mut n = 0;
        let res = self.load_tokens().map_err(std::io::Error::other).and_then(|mut recs| {
            let before = recs.len();
            recs.retain(|t| t.user != u.id);
            n = before - recs.len();
            self.save_tokens(&recs)
        });
        if let Err(e) = res {
            eprintln!("beatscript-serve: auth: could not store the change: {e}");
            return Reply::err(500, "internal", "could not store the change; try again later");
        }
        Reply::json(json!({"revoked": n}))
    }
}

/// What a handler answers: Go's `writeJSON` (no-store, the JSON, any cookies), or an answer made already (an error).
pub enum Reply {
    Json(Value, Vec<HeaderValue>),
    Raw(Response<Body>),
}

impl Reply {
    fn json(v: Value) -> Reply {
        Reply::Json(v, vec![])
    }
    fn err(status: u16, code: &str, m: impl Into<String>) -> Reply {
        Reply::Raw(ApiError::new(status, code, m).response())
    }
    pub fn status(&self) -> u16 {
        match self {
            Reply::Json(..) => 200,
            Reply::Raw(r) => r.status().as_u16(),
        }
    }
    pub fn response(self) -> Response<Body> {
        match self {
            Reply::Raw(r) => r,
            Reply::Json(v, cookies) => {
                let mut r = Response::builder().header(header::CACHE_CONTROL, "no-store").header(header::CONTENT_TYPE, "application/json; charset=utf-8");
                for c in cookies {
                    r = r.header(header::SET_COOKIE, c);
                }
                let body = crate::go_json(&v) + "\n";
                // HEAD is answered the same: net/http counts what the handler wrote and appends Content-Length
                // after the handler's (sorted) headers, as axum does for a body of known size, which it then drops
                r.body(crate::go_body(body)).unwrap()
            }
        }
    }
}

// ---------- the routes ----------

type St = (Arc<AuthSvc>, AppState);

fn ip_of(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

/// A body read as Go's MaxBytesReader lets the decoder see it: at most `max + 1` bytes (one more says "too long").
async fn read_upto(body: Body, max: usize) -> Vec<u8> {
    use futures_util::StreamExt;
    let mut s = body.into_data_stream();
    let mut out = vec![];
    while let Some(Ok(chunk)) = s.next().await {
        out.extend_from_slice(&chunk);
        if out.len() > max {
            out.truncate(max + 1);
            break;
        }
    }
    out
}

async fn h_challenge(State((a, _)): State<St>, req: Request) -> Response<Body> {
    a.challenge(&ip_of(&req)).response()
}

async fn h_login(State((a, _)): State<St>, req: Request) -> Response<Body> {
    let ip = ip_of(&req);
    let (parts, body) = req.into_parts();
    // the refusals that come before the body is read do not need it
    if a.cross_site(&parts.headers) {
        return a.login(&parts.headers, &ip, b"").response();
    }
    let b = read_upto(body, LOGIN_MAX_BODY).await;
    a.login(&parts.headers, &ip, &b).response()
}

async fn h_logout(State((a, _)): State<St>, req: Request) -> Response<Body> {
    a.logout(req.headers()).response()
}

async fn h_me(State((a, _)): State<St>, req: Request) -> Response<Body> {
    let mut r = a.me(req.headers());
    // the identity's capabilities (caps.rs): what the page may show; the server still checks on every LLM route
    if let Reply::Json(v, _) = &mut r {
        if let Some(id) = v["user"]["id"].as_str().map(str::to_string) {
            if let Some(c) = crate::caps::get() {
                v["caps"] = json!(c.caps_for(&id));
            }
        }
    }
    r.response()
}

/// `GET /api/auth/grant`: the signed capability grant (caps.rs) of the caller: the cookie, a `btw_` token, or a
/// paired phone (`BtwDevice`, signed over `GET /api/auth/grant` and an empty body).
async fn h_grant(State((a, _)): State<St>, req: Request) -> Response<Body> {
    let c = match a.require_caller(req.headers(), "GET", "/api/auth/grant", b"") {
        Ok(c) => c,
        Err(r) => return r,
    };
    match crate::caps::grant_json(&c.uid) {
        Some(v) => Reply::json(v).response(),
        None => Reply::err(503, "not_configured", "capabilities are not set up on this server").response(),
    }
}

async fn h_token_create(State((a, _)): State<St>, req: Request) -> Response<Body> {
    let ip = ip_of(&req);
    a.token_create(req.headers(), &ip).response()
}

async fn h_token_revoke(State((a, _)): State<St>, req: Request) -> Response<Body> {
    let uid = a.user(req.headers()).ok().flatten().map(|u| u.id);
    let mut r = a.token_revoke(req.headers());
    // "Revoke all my tokens" disconnects the OAuth clients too
    if let (Reply::Json(v, _), Some(uid), Some(o)) = (&mut r, uid, crate::oauth::get()) {
        v["oauth_revoked"] = json!(o.revoke_user(&uid));
    }
    r.response()
}

/// A method Go's mux has no pattern for here: the catch-all (the static site) answers, as in Go.
fn other(app: &AppState, req: &Request) -> Response<Body> {
    app.site.serve(req.method(), req.uri().path(), req.headers())
}

/// One handler per path, choosing by method itself (an axum method router would add its own `Allow` to the static
/// site's answer, which Go's catch-all does not have).
async fn h_route(State(st): State<St>, req: Request) -> Response<Body> {
    let get = req.method() == Method::GET || req.method() == Method::HEAD;
    let post = req.method() == Method::POST;
    match req.uri().path() {
        "/api/auth/challenge" if get => h_challenge(State(st), req).await,
        "/api/auth/login" if post => h_login(State(st), req).await,
        "/api/auth/logout" if post => h_logout(State(st), req).await,
        "/api/auth/me" if get => h_me(State(st), req).await,
        "/api/auth/grant" if get => h_grant(State(st), req).await,
        "/api/auth/token" if post => h_token_create(State(st), req).await,
        "/api/auth/token" if req.method() == Method::DELETE => h_token_revoke(State(st), req).await,
        _ => other(&st.1, &req),
    }
}

pub fn routes(app: &AppState) -> Router {
    let svc = Arc::new(AuthSvc::new(app));
    let _ = app.authsvc.set(svc.clone());
    let mut r = Router::new();
    for p in ["/api/auth/challenge", "/api/auth/login", "/api/auth/logout", "/api/auth/me", "/api/auth/grant", "/api/auth/token"] {
        r = r.route(p, any(h_route));
    }
    r.with_state((svc, app.clone()))
}

#[cfg(test)]
mod tests;
