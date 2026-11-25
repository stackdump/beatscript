//! Who is calling: the Go server's own account session, verified here against the Go server's own files.
//!
//! Nothing here issues a credential: the Go server (beats-bitwrap-io `internal/btwsite/auth/auth.go`,
//! `internal/btwsite/server/pair.go`, `site.go`), or beatscript-serve's own `authsvc` (P4, the binary's port of auth.go),
//! signs people in and mints tokens, and pairing makes phones. This verifier only checks what a request carries, the
//! same three ways Go does, against the same data directory (`-data`):
//!
//! | credential | Go mints it in | what beatscript-serve reads | how it is checked |
//! |---|---|---|---|
//! | cookie `btw_session` | `auth.Service.setCookie` after a signed challenge | `BEATSCRIPT_SESSION_SECRET` (env), else `<data>/session.secret` (hex, 0600) | `base64url(payload) "." base64url(HMAC-SHA256(secret, "btw-session-v1\n" ‖ payload))`, payload `{"u":uid,"e":unix}`, not expired |
//! | `Authorization: Bearer btw_…` | `POST /api/auth/token` | `<data>/tokens.json` (`[{hash,user,created}]`, 0600) | `hex(sha256(token))` is a record's `hash` |
//! | `Authorization: BtwDevice <dev>.<ts>.<sig>` | `POST /api/pair/claim` (a phone's QR pairing) | `<data>/devices.json` (public keys), `<data>/site.json` (site id) | Ed25519 over the `btw-pair/2 request` text (site id), else `btw-pair/1` under each accepted host; ts within ±2 min; a signature is used once |
//!
//! The secret and the token hashes are read here, never written and never logged. tokens.json and devices.json are
//! re-read when their size, mtime or inode changes (both servers replace them by rename), so a token revoked or a
//! phone unpaired stops working here on the next request. A cookie cannot be revoked in Go either (logout clears it in the browser): it
//! is good until its `e`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Go's `auth.CookieName`.
pub const COOKIE_NAME: &str = "btw_session";
/// Go's `auth.TokenPrefix`.
pub const TOKEN_PREFIX: &str = "btw_";
const DEVICE_SKEW_MS: i64 = 2 * 60 * 1000;

/// The account a request acts as. `uid` is Go's `api.User.ID`: `eip155:0x<40 hex>` or `ed25519:<64 hex>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub uid: String,
    pub via: Via,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    Cookie,
    Token,
    /// A paired phone, acting as the account that paired it (the device id).
    Device(String),
}

/// A credential was presented and is not good. `status` 401, or 410 for a phone that is no longer paired (Go's
/// `unpaired`, which tells the phone to forget the pairing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

fn auth_err(code: &'static str, m: impl Into<String>) -> AuthError {
    AuthError { status: 401, code, message: m.into() }
}

/// Go's `reUID`.
pub fn valid_uid(s: &str) -> bool {
    let hexlow = |t: &str, n: usize| t.len() == n && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if let Some(a) = s.strip_prefix("eip155:0x") {
        return hexlow(a, 40);
    }
    if let Some(a) = s.strip_prefix("ed25519:") {
        return hexlow(a, 64);
    }
    false
}

/// What the verifier needs to know about the site.
#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// The Go server's data directory.
    pub data: PathBuf,
    /// `BEATSCRIPT_SESSION_SECRET` as Go reads it (trimmed), when set. Wins over `<data>/session.secret`, as in Go.
    pub secret_env: Option<String>,
    /// The hosts this site answers for, the public one first: Go's `acceptedHosts()` (the `-public-url` host, the
    /// `-alt-origins` hosts, `-legacy-hosts`). A `btw-pair/1` signature names one of them; a browser's `Origin` must
    /// be one of them (or the request's own `Host`).
    pub hosts: Vec<String>,
}

#[derive(Default)]
struct Cached<T> {
    /// (size, mtime, inode): a rewrite by rename (Go's and beatscript-serve's way) always has a new inode, so a token minted
    /// or revoked within the same second, at the same size, is still noticed.
    stamp: Option<(u64, SystemTime, u64)>,
    value: T,
}

#[derive(Clone)]
struct DeviceRec {
    owner: String,
    key: [u8; 32],
}

pub struct Auth {
    cfg: AuthConfig,
    secret: Mutex<Cached<Option<Vec<u8>>>>,
    tokens: Mutex<Cached<HashMap<String, String>>>,
    devices: Mutex<Cached<HashMap<String, DeviceRec>>>,
    site: Mutex<Cached<Option<String>>>,
    seen: Mutex<HashMap<String, Instant>>,
}

/// Re-read `path` with `parse` when its size or mtime differs from the cached stamp. A file that is gone leaves
/// `T::default()`; one that does not parse keeps the last good value (Go writes by rename, so a half file is not
/// expected, but a bad hand edit must not sign everybody out).
fn refresh<T: Default>(c: &mut Cached<T>, path: &Path, parse: impl FnOnce(&[u8]) -> Option<T>) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let stamp = (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH), std::os::unix::fs::MetadataExt::ino(&m));
            if c.stamp == Some(stamp) {
                return;
            }
            if let Ok(b) = std::fs::read(path) {
                match parse(&b) {
                    Some(v) => c.value = v,
                    None => eprintln!("beatscript-serve: playlists: {} does not parse; keeping what was read before", path.display()),
                }
                c.stamp = Some(stamp);
            }
        }
        Err(_) => {
            c.stamp = None;
            c.value = T::default();
        }
    }
}

impl Auth {
    pub fn new(cfg: AuthConfig) -> Auth {
        Auth {
            cfg,
            secret: Mutex::new(Cached::default()),
            tokens: Mutex::new(Cached::default()),
            devices: Mutex::new(Cached::default()),
            site: Mutex::new(Cached::default()),
            seen: Mutex::new(HashMap::new()),
        }
    }

    pub fn hosts(&self) -> &[String] {
        &self.cfg.hosts
    }

    /// The cookie key, as Go's `auth.LoadSecret`: the env value's bytes, else the hex in `<data>/session.secret`.
    /// None when neither exists: beatscript-serve never makes one (that would sign everybody out of the Go server).
    pub fn secret(&self) -> Option<Vec<u8>> {
        if let Some(s) = &self.cfg.secret_env {
            return (s.len() >= 16).then(|| s.as_bytes().to_vec());
        }
        let mut c = self.secret.lock().unwrap();
        refresh(&mut c, &self.cfg.data.join("session.secret"), |b| {
            let raw = hex::decode(std::str::from_utf8(b).ok()?.trim()).ok()?;
            Some((raw.len() >= 16).then_some(raw))
        });
        c.value.clone()
    }

    /// Is the account side configured (a cookie key exists)?
    pub fn ready(&self) -> bool {
        self.secret().is_some()
    }

    fn sign(secret: &[u8], payload: &[u8]) -> Hmac<Sha256> {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("hmac takes any key length");
        m.update(b"btw-session-v1\n");
        m.update(payload);
        m
    }

    /// Go's `parseCookieValue`. `now` is unix seconds.
    pub fn parse_cookie(&self, v: &str, now: i64) -> Result<String, AuthError> {
        let bad = || auth_err("auth_required", "invalid session cookie: sign in again");
        let secret = self.secret().ok_or_else(bad)?;
        let (a, b) = v.split_once('.').ok_or_else(bad)?;
        let payload = B64.decode(a).map_err(|_| bad())?;
        let sig = B64.decode(b).map_err(|_| bad())?;
        Self::sign(&secret, &payload).verify_slice(&sig).map_err(|_| bad())?;
        let p: serde_json::Value = serde_json::from_slice(&payload).map_err(|_| bad())?;
        let uid = p.get("u").and_then(|u| u.as_str()).ok_or_else(bad)?;
        let exp = p.get("e").and_then(|e| e.as_i64()).ok_or_else(bad)?;
        if now >= exp {
            return Err(auth_err("auth_required", "invalid session cookie: expired: sign in again"));
        }
        if !valid_uid(uid) {
            return Err(bad());
        }
        Ok(uid.to_string())
    }

    /// Go's `makeCookieValue`: the cookie value for `uid` expiring at `exp` (unix seconds). Only the sign-in
    /// (`authsvc`, after a verified signature) and tests call it.
    pub fn make_cookie(secret: &[u8], uid: &str, exp: i64) -> String {
        let p = serde_json::to_vec(&serde_json::json!({"u": uid, "e": exp})).unwrap();
        let sig = Self::sign(secret, &p).finalize().into_bytes();
        format!("{}.{}", B64.encode(&p), B64.encode(sig))
    }

    /// The same, under the name the tests use.
    pub fn make_cookie_for_test(secret: &[u8], uid: &str, exp: i64) -> String {
        Self::make_cookie(secret, uid, exp)
    }

    /// Go's `UserFromToken`.
    pub fn user_from_token(&self, tok: &str) -> Option<String> {
        if !tok.starts_with(TOKEN_PREFIX) || tok.len() > 200 {
            return None;
        }
        let mut c = self.tokens.lock().unwrap();
        refresh(&mut c, &self.cfg.data.join("tokens.json"), |b| {
            let recs: Vec<serde_json::Value> = serde_json::from_slice(b).ok()?;
            Some(
                recs.iter()
                    .filter_map(|r| Some((r.get("hash")?.as_str()?.to_string(), r.get("user")?.as_str()?.to_string())))
                    .filter(|(_, u)| valid_uid(u))
                    .collect(),
            )
        });
        c.value.get(&hex::encode(Sha256::digest(tok.as_bytes()))).cloned()
    }

    fn site_id(&self) -> Option<String> {
        let mut c = self.site.lock().unwrap();
        refresh(&mut c, &self.cfg.data.join("site.json"), |b| {
            let v: serde_json::Value = serde_json::from_slice(b).ok()?;
            Some(v.get("site_id").and_then(|s| s.as_str()).map(str::to_string))
        });
        c.value.clone()
    }

    fn device(&self, id: &str) -> Option<DeviceRec> {
        let mut c = self.devices.lock().unwrap();
        refresh(&mut c, &self.cfg.data.join("devices.json"), |b| {
            let list: Vec<serde_json::Value> = serde_json::from_slice(b).ok()?;
            let mut m = HashMap::new();
            for d in list {
                let (Some(id), Some(owner), Some(key)) = (d.get("id").and_then(|x| x.as_str()), d.get("owner").and_then(|x| x.as_str()), d.get("key").and_then(|x| x.as_str())) else { continue };
                let Ok(k) = B64.decode(key) else { continue };
                let Ok(key) = <[u8; 32]>::try_from(k.as_slice()) else { continue };
                if !id.is_empty() && valid_uid(owner) {
                    m.insert(id.to_string(), DeviceRec { owner: owner.to_string(), key });
                }
            }
            Some(m)
        });
        c.value.get(id).cloned()
    }

    /// Go's `RequestMessage2`.
    pub fn request_message2(site_id: &str, device: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> String {
        format!("btw-pair/2 request\nsite: {site_id}\ndevice: {device}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {}", hex::encode(Sha256::digest(body)))
    }

    /// Go's `RequestMessage` (v1, host-bound).
    pub fn request_message1(host: &str, device: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> String {
        format!("btw-pair/1 request\nhost: {host}\ndevice: {device}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {}", hex::encode(Sha256::digest(body)))
    }

    /// Go's `deviceFrom` for an `Authorization: BtwDevice …` value (the part after the scheme). `now_ms` unix ms.
    pub fn device_from(&self, cred: &str, method: &str, path: &str, body: &[u8], now_ms: i64) -> Result<Caller, AuthError> {
        let fail = |m: &str| auth_err("device_auth", m);
        let parts: Vec<&str> = cred.trim().split('.').collect();
        if parts.len() != 3 {
            return Err(fail("malformed BtwDevice credential"));
        }
        let ts: i64 = parts[1].parse().map_err(|_| fail("malformed BtwDevice credential"))?;
        let sig = B64.decode(parts[2]).ok().and_then(|s| <[u8; 64]>::try_from(s.as_slice()).ok()).ok_or_else(|| fail("malformed BtwDevice credential"))?;
        if (ts - now_ms).abs() > DEVICE_SKEW_MS {
            return Err(fail("the phone's clock is too far from the server's"));
        }
        let Some(d) = self.device(parts[0]) else {
            return Err(AuthError { status: 410, code: "unpaired", message: "this phone is not paired (any more): scan a new code".into() });
        };
        let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&d.key) else {
            return Err(fail("the request's signature does not verify"));
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig);
        let mut ok = false;
        if let Some(site) = self.site_id() {
            ok = vk.verify_strict(Self::request_message2(&site, parts[0], method, path, ts, body).as_bytes(), &sig).is_ok();
        }
        if !ok {
            ok = self.cfg.hosts.iter().any(|h| vk.verify_strict(Self::request_message1(h, parts[0], method, path, ts, body).as_bytes(), &sig).is_ok());
        }
        if !ok {
            return Err(fail("the request's signature does not verify"));
        }
        let mut seen = self.seen.lock().unwrap();
        let now = Instant::now();
        seen.retain(|_, at| now.duration_since(*at).as_millis() as i64 <= 2 * DEVICE_SKEW_MS);
        if seen.contains_key(parts[2]) {
            return Err(fail("this request was already made (replay)"));
        }
        seen.insert(parts[2].to_string(), now);
        Ok(Caller { uid: d.owner, via: Via::Device(parts[0].to_string()) })
    }

    /// Go's `UserFrom` plus the `BtwDevice` scheme (as `PUT /api/shares/<cid>` accepts all three): the cookie first,
    /// then the Authorization header. `Ok(None)` is anonymous. `cookie` is the `btw_session` value, if any.
    pub fn caller(&self, cookie: Option<&str>, authorization: Option<&str>, method: &str, path: &str, body: &[u8]) -> Result<Option<Caller>, AuthError> {
        self.caller_at(SystemTime::now(), cookie, authorization, method, path, body)
    }

    /// `caller` judged at `at` instead of the wall clock: the cookie's expiry and the device's clock skew are measured
    /// against the clock its owner runs on (`AuthSvc`'s injectable one), so a service with a fixed clock does not see a
    /// session expire because the real date moved on.
    pub fn caller_at(&self, at: SystemTime, cookie: Option<&str>, authorization: Option<&str>, method: &str, path: &str, body: &[u8]) -> Result<Option<Caller>, AuthError> {
        let now = at.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let mut cookie_err = None;
        if let Some(c) = cookie.filter(|c| !c.is_empty()) {
            match self.parse_cookie(c, now.as_secs() as i64) {
                Ok(uid) => return Ok(Some(Caller { uid, via: Via::Cookie })),
                Err(e) => cookie_err = Some(e),
            }
        }
        if let Some(h) = authorization.filter(|h| !h.is_empty()) {
            if let Some(cred) = h.strip_prefix("BtwDevice ") {
                return self.device_from(cred, method, path, body, now.as_millis() as i64).map(Some);
            }
            let (scheme, tok) = h.split_once(' ').unwrap_or((h, ""));
            let tok = tok.trim();
            if scheme.eq_ignore_ascii_case("bearer") && tok.starts_with(TOKEN_PREFIX) {
                return match self.user_from_token(tok) {
                    Some(uid) => Ok(Some(Caller { uid, via: Via::Token })),
                    None => Err(auth_err("auth_required", "unknown or revoked token")),
                };
            }
        }
        match cookie_err {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }
}

/// The `btw_session` value from a `Cookie` header (Go's `r.Cookie`: the first one of that name).
pub fn cookie_value<'a>(headers: impl Iterator<Item = &'a str>, name: &str) -> Option<&'a str> {
    for h in headers {
        for part in h.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k == name {
                    return Some(v.trim_matches('"'));
                }
            }
        }
    }
    None
}

/// Go's `crossSite` (auth.go / pair.go): a browser request started from another origin. An `Origin` must name the
/// request's own host or one of the site's hosts ("null" and anything unreadable are cross-site); with no Origin,
/// `Sec-Fetch-Site: cross-site|same-site` is. A client with neither (curl, the phone, the desktop app) passes.
pub fn cross_site(origin: Option<&str>, sec_fetch_site: Option<&str>, host: Option<&str>, hosts: &[String]) -> bool {
    if let Some(o) = origin.filter(|o| !o.is_empty()) {
        let Some(rest) = o.split_once("://").map(|(_, r)| r) else { return true };
        let h = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
        if h.is_empty() || h.contains('@') {
            return true;
        }
        if host.is_some_and(|x| x.eq_ignore_ascii_case(&h)) {
            return false;
        }
        return !hosts.iter().any(|a| a.eq_ignore_ascii_case(&h));
    }
    matches!(sec_fetch_site, Some("cross-site") | Some("same-site"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &Path) -> AuthConfig {
        AuthConfig { data: dir.to_path_buf(), secret_env: None, hosts: vec!["beatscript.app".into(), "btw.bitwrap.io".into()] }
    }

    #[test]
    fn uid_shapes() {
        assert!(valid_uid(&format!("ed25519:{}", "a".repeat(64))));
        assert!(valid_uid(&format!("eip155:0x{}", "0".repeat(40))));
        assert!(!valid_uid(&format!("ed25519:{}", "A".repeat(64))));
        assert!(!valid_uid("eip155:0x12"));
        assert!(!valid_uid(""));
    }

    /// A cookie the Go server made: `auth.Service.makeCookieValue` with Secret "0123456789abcdef0123456789abcdef",
    /// uid ed25519:abcdef0123…789, exp 2000000000 (printed by a throwaway Go test in internal/btwsite/auth). Ours is
    /// the same bytes, and it verifies here.
    const GO_COOKIE: &str = "eyJ1IjoiZWQyNTUxOTphYmNkZWYwMTIzNDU2Nzg5YWJjZGVmMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYwMTIzNDU2Nzg5IiwiZSI6MjAwMDAwMDAwMH0.YWuupXiPTTlMOQPiqLuiMIfP9_TbWHtTdodRaF9X7Q0";
    const GO_UID: &str = "ed25519:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[test]
    fn cookie_matches_go() {
        let secret = b"0123456789abcdef0123456789abcdef";
        assert_eq!(Auth::make_cookie_for_test(secret, GO_UID, 2_000_000_000), GO_COOKIE);
        let a = Auth::new(AuthConfig { secret_env: Some(String::from_utf8(secret.to_vec()).unwrap()), ..cfg(Path::new("/nonexistent")) });
        assert_eq!(a.parse_cookie(GO_COOKIE, 1_795_000_000).unwrap(), GO_UID);
    }

    /// btwsite/testdata/pair/vectors{,-v2}.json: the request texts and signatures Go's pair_test.go pins (the phone's
    /// key is seed 32 x 0x01, the body `{"playing":true}`).
    #[test]
    fn device_messages_match_go_vectors() {
        let body = br#"{"playing":true}"#;
        assert_eq!(
            Auth::request_message2("siteSITEsiteSITEsite01", "devDEVdevDEVdevDEVdev1", "POST", "/api/device/report", 1795000000123, body),
            "btw-pair/2 request\nsite: siteSITEsiteSITEsite01\ndevice: devDEVdevDEVdevDEVdev1\nmethod: POST\npath: /api/device/report\nts: 1795000000123\nbody: 322b23ab3358fa4126a2ade4d955c51d0cf9556f0d06deda98cc94530e2a4561"
        );
        assert_eq!(
            Auth::request_message1("btw.bitwrap.io", "devDEVdevDEVdevDEVdev1", "POST", "/api/device/report", 1795000000123, body),
            "btw-pair/1 request\nhost: btw.bitwrap.io\ndevice: devDEVdevDEVdevDEVdev1\nmethod: POST\npath: /api/device/report\nts: 1795000000123\nbody: 322b23ab3358fa4126a2ade4d955c51d0cf9556f0d06deda98cc94530e2a4561"
        );
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("site.json"), r#"{"site_id":"siteSITEsiteSITEsite01","created":"x"}"#).unwrap();
        let owner = format!("ed25519:{}", "d".repeat(64));
        std::fs::write(d.path().join("devices.json"), format!(r#"[{{"id":"devDEVdevDEVdevDEVdev1","owner":"{owner}","name":"Pixel 9","key":"iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w","paired":"x"}}]"#)).unwrap();
        let a = Auth::new(cfg(d.path()));
        let now = 1795000000123;
        for sig in ["bNVAImIzKyGYP5H1n5YlKRe5e-5E0s3U38_mw784QHBZBkuu2ajCF3D0dIhMDN5ep0MAC5N5757C7rXuBa0oBg", "xsa0SIF-PsNWEH0B_KtvhGtK1cjjaoqfzn4dbtY5NcHVa23CFU-aSj2Z7_rZQRSA_U2IqXVOePPAGIeVMhskCw"] {
            let cred = format!("devDEVdevDEVdevDEVdev1.1795000000123.{sig}");
            let c = a.device_from(&cred, "POST", "/api/device/report", body, now).unwrap();
            assert_eq!(c.uid, owner);
            assert_eq!(c.via, Via::Device("devDEVdevDEVdevDEVdev1".into()));
            assert_eq!(a.device_from(&cred, "POST", "/api/device/report", body, now).unwrap_err().message, "this request was already made (replay)");
        }
        let v2 = "devDEVdevDEVdevDEVdev1.1795000000123.bNVAImIzKyGYP5H1n5YlKRe5e-5E0s3U38_mw784QHBZBkuu2ajCF3D0dIhMDN5ep0MAC5N5757C7rXuBa0oBg";
        let a = Auth::new(cfg(d.path()));
        assert!(a.device_from(v2, "POST", "/api/device/report", b"{}", now).is_err(), "another body");
        assert!(a.device_from(v2, "POST", "/api/device/reports", body, now).is_err(), "another path");
        assert!(a.device_from(v2, "POST", "/api/device/report", body, now + 3 * 60 * 1000).unwrap_err().message.contains("clock"));
        assert_eq!(a.device_from("devOTHER.1795000000123.bNVAImIzKyGYP5H1n5YlKRe5e-5E0s3U38_mw784QHBZBkuu2ajCF3D0dIhMDN5ep0MAC5N5757C7rXuBa0oBg", "POST", "/x", body, now).unwrap_err().status, 410);
    }

    #[test]
    fn cookie_verify_and_refuse() {
        let d = tempfile::tempdir().unwrap();
        let secret = [7u8; 32];
        std::fs::write(d.path().join("session.secret"), hex::encode(secret) + "\n").unwrap();
        let a = Auth::new(cfg(d.path()));
        let uid = format!("ed25519:{}", "b".repeat(64));
        let good = Auth::make_cookie_for_test(&secret, &uid, 2_000_000_000);
        assert_eq!(a.parse_cookie(&good, 1_900_000_000).unwrap(), uid);
        assert!(a.parse_cookie(&good, 2_000_000_000).unwrap_err().message.contains("expired"));
        let forged = Auth::make_cookie_for_test(&[8u8; 32], &uid, 2_000_000_000);
        assert!(a.parse_cookie(&forged, 1).is_err());
        assert!(a.parse_cookie("nodot", 1).is_err());
        // the env secret wins, as in Go
        let a2 = Auth::new(AuthConfig { secret_env: Some("an env secret of 16+".into()), ..cfg(d.path()) });
        assert!(a2.parse_cookie(&good, 1).is_err());
        assert!(a2.parse_cookie(&Auth::make_cookie_for_test(b"an env secret of 16+", &uid, 2_000_000_000), 1).is_ok());
    }

    #[test]
    fn no_secret_no_accounts() {
        let d = tempfile::tempdir().unwrap();
        let a = Auth::new(cfg(d.path()));
        assert!(!a.ready());
        assert!(!d.path().join("session.secret").exists(), "beatscript-serve must never make the Go server's key");
    }

    #[test]
    fn tokens_reload_on_change() {
        let d = tempfile::tempdir().unwrap();
        let a = Auth::new(cfg(d.path()));
        let uid = format!("eip155:0x{}", "c".repeat(40));
        let tok = "btw_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        assert_eq!(a.user_from_token(tok), None);
        let rec = serde_json::json!([{"hash": hex::encode(Sha256::digest(tok.as_bytes())), "user": uid, "created": "2026-11-19T00:00:00Z"}]);
        std::fs::write(d.path().join("tokens.json"), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
        assert_eq!(a.user_from_token(tok), Some(uid));
        std::fs::write(d.path().join("tokens.json"), "[]").unwrap();
        assert_eq!(a.user_from_token(tok), None, "a revoked token stops working at once");
    }

    #[test]
    fn cross_site_rules() {
        let hosts = vec!["beatscript.app".to_string()];
        assert!(!cross_site(Some("https://beatscript.app"), None, Some("127.0.0.1:18098"), &hosts));
        assert!(!cross_site(Some("http://localhost:1234"), None, Some("localhost:1234"), &hosts));
        assert!(cross_site(Some("https://evil.example"), None, Some("beatscript.app"), &hosts));
        assert!(cross_site(Some("null"), None, Some("beatscript.app"), &hosts));
        assert!(cross_site(None, Some("cross-site"), None, &hosts));
        assert!(cross_site(None, Some("same-site"), None, &hosts));
        assert!(!cross_site(None, Some("same-origin"), None, &hosts));
        assert!(!cross_site(None, None, None, &hosts));
    }

    #[test]
    fn cookie_header_parse() {
        assert_eq!(cookie_value(["a=1; btw_session=xyz.abc; c=3"].into_iter(), "btw_session"), Some("xyz.abc"));
        assert_eq!(cookie_value(["a=1"].into_iter(), "btw_session"), None);
    }
}
