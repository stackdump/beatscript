//! Capabilities: what an identity may do beyond being signed in (today only `compose`, the LLM), and the signed grant
//! a client holds to know it. CAPABILITIES.md in beats-bitwrap-io `btwsite/` is the client-facing spec.
//!
//! - The allowlist: `<data>/caps.json`, `{"enforce":false,"identities":{"ed25519:<hex>":["compose"]}}`, managed
//!   through `GET`/`POST /api/admin/caps` (the admin token). An identity not on it has no capabilities (`[]`).
//!   Re-read when it changes on disk, so a hand edit is seen too.
//! - Enforcement is here, on the server, whatever grant a client holds: `require(uid, "compose")` is called by
//!   compose (web and phone). With `enforce: false` (the default, and what a missing file means) every signed-in
//!   account may compose, as before, under the existing limits and spend policy; with `enforce: true` only an
//!   identity whose list has `compose`.
//! - The grant: `base64url(header).base64url(payload).base64url(sig)`, header `{"alg":"EdDSA","kid","typ":"btw-cap+jws"}`,
//!   payload `{"iss","sub","caps","iat","exp","jti"}`, Ed25519 over the ASCII `<header>.<payload>`, 7 days. It tells
//!   a client (the Android app, the page) what to show; it is never what the server checks.
//! - The signing key: `<data>/cap-signing.key` (hex seed, 0600, made at start when missing), separate from
//!   session.secret. `<data>/cap-signing.prev.key`, when present, is published too (not used to sign), so a rotated
//!   key's grants verify until they expire. Published at `GET /.well-known/btw-capabilities.json`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const GRANT_TTL: i64 = 7 * 24 * 3600;
pub const TYP: &str = "btw-cap+jws";
/// The capabilities the server knows. The admin endpoint refuses any other name (a typo would silently grant nothing).
pub const KNOWN: &[&str] = &["compose"];

pub struct Key {
    pub kid: String,
    pub vk: VerifyingKey,
    sk: Option<SigningKey>,
}

impl Key {
    pub fn from_seed(seed: [u8; 32], signing: bool) -> Key {
        let sk = SigningKey::from_bytes(&seed);
        let vk = sk.verifying_key();
        Key { kid: kid_of(&vk), vk, sk: signing.then_some(sk) }
    }
}

/// The key id: the first 16 hex digits of SHA-256 of the public key.
pub fn kid_of(vk: &VerifyingKey) -> String {
    hex::encode(&Sha256::digest(vk.as_bytes())[..8])
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Allow {
    pub enforce: bool,
    pub identities: BTreeMap<String, Vec<String>>,
}

impl Allow {
    fn parse(b: &[u8]) -> Option<Allow> {
        let v: Value = serde_json::from_slice(b).ok()?;
        let o = v.as_object()?;
        let enforce = o.get("enforce").and_then(Value::as_bool).unwrap_or(false);
        let mut identities = BTreeMap::new();
        if let Some(m) = o.get("identities").and_then(Value::as_object) {
            for (k, list) in m {
                let caps: Vec<String> = list.as_array()?.iter().filter_map(|c| c.as_str().map(str::to_string)).collect();
                identities.insert(k.clone(), caps);
            }
        }
        Some(Allow { enforce, identities })
    }
    pub fn json(&self) -> Value {
        json!({"enforce": self.enforce, "identities": self.identities})
    }
}

pub struct Caps {
    data: PathBuf,
    issuer: String,
    keys: Vec<Key>,
    allow: Mutex<(Option<(u64, SystemTime, u64)>, Allow)>,
    pub now: Box<dyn Fn() -> i64 + Send + Sync>,
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn read_seed(p: &Path) -> Result<Option<[u8; 32]>, String> {
    match std::fs::read(p) {
        Ok(b) => {
            let raw = std::str::from_utf8(&b).ok().and_then(|s| hex::decode(s.trim()).ok());
            match raw.and_then(|r| <[u8; 32]>::try_from(r).ok()) {
                Some(s) => Ok(Some(s)),
                None => Err(format!("{} is not a hex Ed25519 seed (32 bytes); delete it to generate a new one", p.display())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// The signing key from `<data>/cap-signing.key`, made (32 random bytes, hex + newline, 0600) when missing.
pub fn load_key(data: &Path) -> Result<[u8; 32], String> {
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let p = data.join("cap-signing.key");
    if let Some(s) = read_seed(&p)? {
        return Ok(s);
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    f.write_all((hex::encode(seed) + "\n").as_bytes()).map_err(|e| e.to_string())?;
    Ok(seed)
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)] // the client-side check: tests, and the spec clients implement
pub struct Claims {
    pub sub: String,
    pub caps: Vec<String>,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

impl Caps {
    /// From the data dir: the current key (made when missing) and the previous one, if any.
    pub fn open(data: &Path, public_url: &str) -> Result<Caps, String> {
        let mut keys = vec![Key::from_seed(load_key(data)?, true)];
        if let Some(prev) = read_seed(&data.join("cap-signing.prev.key"))? {
            keys.push(Key::from_seed(prev, false));
        }
        Ok(Caps::with_keys(data, public_url, keys))
    }

    pub fn with_keys(data: &Path, public_url: &str, keys: Vec<Key>) -> Caps {
        let issuer = public_url.trim_end_matches('/').to_string();
        Caps { data: data.to_path_buf(), issuer, keys, allow: Mutex::new((None, Allow::default())), now: Box::new(unix_now) }
    }

    #[allow(dead_code)]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// `/.well-known/btw-capabilities.json`.
    pub fn jwks(&self) -> Value {
        let keys: Vec<Value> = self.keys.iter().map(|k| json!({"kid": k.kid, "alg": "Ed25519", "pub": B64.encode(k.vk.as_bytes())})).collect();
        json!({"issuer": self.issuer, "typ": TYP, "keys": keys})
    }

    /// caps.json as it is now (re-read when its size, mtime or inode changed; a file that does not parse keeps the
    /// last good one, so a bad hand edit grants nothing new and takes nothing away).
    pub fn allow(&self) -> Allow {
        let p = self.data.join("caps.json");
        let mut g = self.allow.lock().unwrap();
        match std::fs::metadata(&p) {
            Ok(m) => {
                let stamp = (m.len(), m.modified().unwrap_or(UNIX_EPOCH), std::os::unix::fs::MetadataExt::ino(&m));
                if g.0 != Some(stamp) {
                    match std::fs::read(&p).ok().as_deref().and_then(Allow::parse) {
                        Some(a) => g.1 = a,
                        None => eprintln!("btwserve: caps: {} does not parse; keeping what was read before", p.display()),
                    }
                    g.0 = Some(stamp);
                }
            }
            Err(_) => *g = (None, Allow::default()),
        }
        g.1.clone()
    }

    fn save(&self, a: &Allow) -> std::io::Result<()> {
        let tmp = self.data.join(format!(".caps-{}", unix_now()));
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all((serde_json::to_string_pretty(&a.json()).unwrap() + "\n").as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, self.data.join("caps.json"))
    }

    /// The capabilities of an identity: its allowlist entry, `[]` when it has none.
    pub fn caps_for(&self, uid: &str) -> Vec<String> {
        let a = self.allow();
        let mut c = a.identities.get(uid).cloned().unwrap_or_default();
        c.retain(|x| KNOWN.contains(&x.as_str()));
        c.sort();
        c.dedup();
        c
    }

    /// Whether `uid` may use `cap` now: the server's check, whatever a client holds.
    pub fn allowed(&self, uid: &str, cap: &str) -> bool {
        !self.allow().enforce || self.caps_for(uid).iter().any(|c| c == cap)
    }

    /// A grant for `uid`, signed with the current key, issued now: (grant, caps, exp).
    pub fn grant(&self, uid: &str) -> (String, Vec<String>, i64) {
        let mut jti = [0u8; 16];
        getrandom::fill(&mut jti).expect("the OS has no entropy");
        let iat = (self.now)();
        let caps = self.caps_for(uid);
        (self.sign(uid, &caps, iat, &B64.encode(jti)), caps, iat + GRANT_TTL)
    }

    /// The grant for exactly these claims (tests and vectors pin iat and jti).
    pub fn sign(&self, uid: &str, caps: &[String], iat: i64, jti: &str) -> String {
        let k = &self.keys[0];
        let sk = k.sk.as_ref().expect("the first key signs");
        let header = json!({"alg": "EdDSA", "kid": k.kid, "typ": TYP});
        let payload = json!({"iss": self.issuer, "sub": uid, "caps": caps, "iat": iat, "exp": iat + GRANT_TTL, "jti": jti});
        let input = format!("{}.{}", B64.encode(header.to_string()), B64.encode(payload.to_string()));
        let sig = sk.sign(input.as_bytes());
        format!("{input}.{}", B64.encode(sig.to_bytes()))
    }

    /// Check a grant as a client does: a published key (by kid), the signature, the issuer, not expired.
    #[allow(dead_code)]
    pub fn verify(&self, grant: &str) -> Result<Claims, String> {
        verify_with(grant, &self.jwks(), &self.issuer, (self.now)())
    }

    // ---------- the admin endpoint ----------

    /// `GET /api/admin/caps`: the allowlist. `POST` with `{"identity":"…","caps":["compose"]}` (an empty list removes
    /// the identity) and/or `{"enforce":true|false}`: the allowlist after the change.
    pub fn admin(&self, post: Option<&[u8]>) -> Result<Value, (u16, String)> {
        let Some(body) = post else { return Ok(self.allow().json()) };
        let v: Value = serde_json::from_slice(body).map_err(|e| (400, format!("body must be JSON: {e}")))?;
        let o = v.as_object().ok_or((400, "body must be a JSON object".to_string()))?;
        if o.keys().any(|k| !matches!(k.as_str(), "identity" | "caps" | "enforce")) {
            return Err((400, r#"send {"identity":"ed25519:…","caps":["compose"]} and/or {"enforce":true|false}"#.into()));
        }
        let mut a = self.allow();
        if let Some(e) = o.get("enforce") {
            a.enforce = e.as_bool().ok_or((400, "enforce must be true or false".to_string()))?;
        }
        match (o.get("identity"), o.get("caps")) {
            (None, None) => {}
            (Some(Value::String(id)), Some(Value::Array(list))) => {
                if !btwserve::playlists::auth::valid_uid(id) {
                    return Err((400, "identity must be ed25519:<64 hex> or eip155:0x<40 hex>, lowercase".into()));
                }
                let mut caps = vec![];
                for c in list {
                    match c.as_str() {
                        Some(c) if KNOWN.contains(&c) => caps.push(c.to_string()),
                        _ => return Err((400, format!("caps must be a list of {KNOWN:?}"))),
                    }
                }
                caps.sort();
                caps.dedup();
                if caps.is_empty() {
                    a.identities.remove(id);
                } else {
                    a.identities.insert(id.clone(), caps);
                }
            }
            _ => return Err((400, "identity (a string) and caps (a list) go together".into())),
        }
        self.save(&a).map_err(|e| (500, format!("could not write caps.json: {e}")))?;
        Ok(self.allow().json())
    }
}

/// Verify a grant against a published key set (`{"keys":[{kid,alg,pub}]}`), the expected issuer and the time.
/// This is what a client does; the Android app and the page do the same steps.
#[allow(dead_code)]
pub fn verify_with(grant: &str, jwks: &Value, issuer: &str, now: i64) -> Result<Claims, String> {
    let parts: Vec<&str> = grant.split('.').collect();
    if parts.len() != 3 {
        return Err("not three dot-separated parts".into());
    }
    let dec = |s: &str| B64.decode(s).map_err(|_| "not base64url".to_string());
    let header: Value = serde_json::from_slice(&dec(parts[0])?).map_err(|_| "header is not JSON")?;
    if header["alg"] != "EdDSA" || header["typ"] != TYP {
        return Err("header alg/typ is not EdDSA/btw-cap+jws".into());
    }
    let kid = header["kid"].as_str().ok_or("no kid")?;
    let key = jwks["keys"].as_array().into_iter().flatten().find(|k| k["kid"] == kid && k["alg"] == "Ed25519").ok_or("unknown kid")?;
    let pk: [u8; 32] = key["pub"].as_str().and_then(|p| B64.decode(p).ok()).and_then(|b| b.try_into().ok()).ok_or("bad published key")?;
    let vk = VerifyingKey::from_bytes(&pk).map_err(|_| "bad published key")?;
    let sig: [u8; 64] = dec(parts[2])?.try_into().map_err(|_| "signature is not 64 bytes")?;
    vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).map_err(|_| "bad signature")?;
    let p: Value = serde_json::from_slice(&dec(parts[1])?).map_err(|_| "payload is not JSON")?;
    if p["iss"] != issuer {
        return Err("wrong issuer".into());
    }
    let (iat, exp) = (p["iat"].as_i64().ok_or("no iat")?, p["exp"].as_i64().ok_or("no exp")?);
    if now >= exp {
        return Err("expired".into());
    }
    if iat > now + 300 {
        return Err("issued in the future".into());
    }
    let caps = p["caps"].as_array().ok_or("no caps")?.iter().map(|c| c.as_str().map(str::to_string).ok_or("caps are not strings")).collect::<Result<_, _>>()?;
    Ok(Claims { sub: p["sub"].as_str().ok_or("no sub")?.to_string(), caps, iat, exp, jti: p["jti"].as_str().unwrap_or("").to_string() })
}

static CAPS: OnceLock<Arc<Caps>> = OnceLock::new();

/// Set once at start (main). Unset (unit tests of other modules) means nothing is enforced and no grant is issued.
pub fn init(c: Arc<Caps>) {
    let _ = CAPS.set(c);
}

pub fn get() -> Option<Arc<Caps>> {
    CAPS.get().cloned()
}

/// The server-side check for an LLM route: Err(message) when caps.json enforces `cap` and `uid` does not have it.
pub fn require(uid: &str, cap: &str) -> Result<(), String> {
    match get() {
        Some(c) if !c.allowed(uid, cap) => Err(format!("this account does not have the {cap} capability on this server (ask the site's admin)")),
        _ => Ok(()),
    }
}

/// `{"caps":[…],"exp":…,"grant":"…"}` for an identity (empty object pieces when the service is not set up).
pub fn grant_json(uid: &str) -> Option<Value> {
    let c = get()?;
    let (g, caps, exp) = c.grant(uid);
    Some(json!({"caps": caps, "exp": exp, "grant": g}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7u8; 32];
    const UID: &str = "ed25519:8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";

    fn caps(dir: &Path) -> Caps {
        let mut c = Caps::with_keys(dir, "https://beatscript.app", vec![Key::from_seed(SEED, true)]);
        c.now = Box::new(|| 1_800_000_000);
        c
    }

    #[test]
    fn grant_verifies_against_published_key() {
        let d = tempfile::tempdir().unwrap();
        let c = caps(d.path());
        c.admin(Some(format!(r#"{{"identity":"{UID}","caps":["compose"]}}"#).as_bytes())).unwrap();
        let (g, list, exp) = c.grant(UID);
        assert_eq!(list, vec!["compose"]);
        assert_eq!(exp, 1_800_000_000 + GRANT_TTL);
        let cl = c.verify(&g).unwrap();
        assert_eq!((cl.sub.as_str(), cl.caps.clone(), cl.iat), (UID, vec!["compose".to_string()], 1_800_000_000));
        // the header, byte for byte as the spec writes it
        let h = B64.decode(g.split('.').next().unwrap()).unwrap();
        assert_eq!(String::from_utf8(h).unwrap(), format!(r#"{{"alg":"EdDSA","kid":"{}","typ":"btw-cap+jws"}}"#, c.keys[0].kid));
        // and with only the published JSON
        let jwks: Value = serde_json::from_str(&c.jwks().to_string()).unwrap();
        assert!(verify_with(&g, &jwks, "https://beatscript.app", 1_800_000_001).is_ok());
    }

    #[test]
    fn refused_grants() {
        let d = tempfile::tempdir().unwrap();
        let c = caps(d.path());
        let g = c.sign(UID, &["compose".into()], 1_800_000_000, "j");
        let jwks = c.jwks();
        // expired
        assert_eq!(verify_with(&g, &jwks, "https://beatscript.app", 1_800_000_000 + GRANT_TTL).unwrap_err(), "expired");
        // wrong issuer
        assert!(verify_with(&g, &jwks, "https://evil.example", 1_800_000_001).is_err());
        // a kid that is not published
        let other = Caps::with_keys(d.path(), "https://beatscript.app", vec![Key::from_seed([9u8; 32], true)]);
        let g2 = other.sign(UID, &[], 1_800_000_000, "j");
        assert_eq!(verify_with(&g2, &jwks, "https://beatscript.app", 1_800_000_001).unwrap_err(), "unknown kid");
        // the right kid with another key's signature
        let parts: Vec<&str> = g.split('.').collect();
        let forged = format!("{}.{}.{}", parts[0], parts[1], g2.split('.').nth(2).unwrap());
        assert_eq!(verify_with(&forged, &jwks, "https://beatscript.app", 1_800_000_001).unwrap_err(), "bad signature");
        // a tampered payload (caps added)
        let p = B64.encode(format!(r#"{{"iss":"https://beatscript.app","sub":"{UID}","caps":["compose","admin"],"iat":1800000000,"exp":1800604800,"jti":"j"}}"#));
        let tampered = format!("{}.{p}.{}", parts[0], parts[2]);
        assert_eq!(verify_with(&tampered, &jwks, "https://beatscript.app", 1_800_000_001).unwrap_err(), "bad signature");
        // a tampered header (alg none)
        let h = B64.encode(r#"{"alg":"none","kid":"x","typ":"btw-cap+jws"}"#);
        assert!(verify_with(&format!("{h}.{}.{}", parts[1], parts[2]), &jwks, "https://beatscript.app", 1_800_000_001).is_err());
    }

    #[test]
    fn not_on_the_allowlist_is_empty_and_enforcement() {
        let d = tempfile::tempdir().unwrap();
        let c = caps(d.path());
        let (g, list, _) = c.grant(UID);
        assert!(list.is_empty());
        assert!(c.verify(&g).unwrap().caps.is_empty());
        // not enforced (the default): everyone signed in may compose, as before
        assert!(c.allowed(UID, "compose"));
        c.admin(Some(br#"{"enforce":true}"#)).unwrap();
        assert!(!c.allowed(UID, "compose"));
        c.admin(Some(format!(r#"{{"identity":"{UID}","caps":["compose"]}}"#).as_bytes())).unwrap();
        assert!(c.allowed(UID, "compose"));
        assert_eq!(c.admin(None).unwrap(), json!({"enforce": true, "identities": {UID: ["compose"]}}));
        // removing
        c.admin(Some(format!(r#"{{"identity":"{UID}","caps":[]}}"#).as_bytes())).unwrap();
        assert!(!c.allowed(UID, "compose"));
        // unknown capability names and bad identities are refused
        assert_eq!(c.admin(Some(format!(r#"{{"identity":"{UID}","caps":["admin"]}}"#).as_bytes())).unwrap_err().0, 400);
        assert_eq!(c.admin(Some(br#"{"identity":"bob","caps":["compose"]}"#)).unwrap_err().0, 400);
    }

    #[test]
    fn key_file_made_once_and_rotation_publishes_both() {
        let d = tempfile::tempdir().unwrap();
        let a = Caps::open(d.path(), "https://beatscript.app").unwrap();
        let b = Caps::open(d.path(), "https://beatscript.app").unwrap();
        assert_eq!(a.keys[0].kid, b.keys[0].kid);
        let g = a.grant(UID).0;
        // rotate: the old key becomes prev, a new one is made
        std::fs::rename(d.path().join("cap-signing.key"), d.path().join("cap-signing.prev.key")).unwrap();
        let r = Caps::open(d.path(), "https://beatscript.app").unwrap();
        assert_eq!(r.jwks()["keys"].as_array().unwrap().len(), 2);
        assert_ne!(r.keys[0].kid, a.keys[0].kid);
        assert!(r.verify(&g).is_ok(), "a grant of the previous key still verifies");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(d.path().join("cap-signing.key")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// `vectors.json`: a fixed seed, its published key set and a grant, for clients' tests (the Android app).
    /// `BTW_REGEN_VECTORS=1 cargo test -p btwserve vectors` rewrites it.
    #[test]
    fn vectors() {
        let d = tempfile::tempdir().unwrap();
        let c = Caps::with_keys(d.path(), "https://beatscript.app", vec![Key::from_seed(SEED, true)]);
        let grant = c.sign(UID, &["compose".into()], 1_800_000_000, "AAECAwQFBgcICQoLDA0ODw");
        let empty = c.sign(UID, &[], 1_800_000_000, "AAECAwQFBgcICQoLDA0ODw");
        let v = json!({
            "about": "btw-cap+jws test vectors (btwserve src/caps.rs). The seed is a test key only; never use it.",
            "seed_hex": hex::encode(SEED),
            "issuer": "https://beatscript.app",
            "jwks": c.jwks(),
            "grant": grant,
            "payload": {"iss": "https://beatscript.app", "sub": UID, "caps": ["compose"], "iat": 1_800_000_000i64, "exp": 1_800_000_000i64 + GRANT_TTL, "jti": "AAECAwQFBgcICQoLDA0ODw"},
            "grant_no_caps": empty,
            "verify_at": 1_800_000_001i64,
            "expired_at": 1_800_000_000i64 + GRANT_TTL,
        });
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/caps/vectors.json");
        let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
        if std::env::var("BTW_REGEN_VECTORS").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(verify_with(&grant, &v["jwks"], "https://beatscript.app", 1_800_000_001).is_ok());
    }
}
