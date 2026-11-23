//! Private pushes: a BeatScript program sealed for one paired phone.
//!
//! The phone's identity is the Ed25519 key it made when it paired (beatscript.app keeps only the public half, in
//! `devices.json`). A push is sealed to that key's X25519 form, the same conversion `age` uses for `ssh-ed25519`
//! recipients and libsodium calls `crypto_sign_ed25519_pk_to_curve25519`:
//!
//! ```text
//! epk, esk   = a fresh X25519 key pair, one per push
//! rpk        = X25519 form of the phone's Ed25519 public key
//! shared     = X25519(esk, rpk)                  (refused if not contributory: a low-order rpk)
//! key        = HKDF-SHA256(ikm = shared, salt = epk || rpk, info = "beatscript-private-push/v1")
//! ct         = ChaCha20-Poly1305(key, nonce = 12 random bytes, aad = "beatscript-private-push/v1" 0 device_id 0 epk)
//! envelope   = {"v":1,"alg":ALG,"epk":b64url,"nonce":b64url,"ct":b64url}
//! ```
//!
//! The plaintext is a [`Payload`] (JSON): what to do (`eval` or `load`), the program text, whether to play, a
//! random id and a timestamp. The phone refuses a payload older than [`MAX_AGE_MS`] or one it has seen
//! ([`ReplayGuard`]). The device id is bound in the AAD, so an envelope sealed for one phone does not open as a push to
//! another even if both had the same key.
//!
//! Who can read what: the sealed envelope travels over beatscript.app's device stream, which relays it in memory
//! and never stores it. When the server itself seals (the MCP `push_private` tool given `text`), the program is in
//! the server's memory for that one request; a client that seals locally ([`seal`] with the phone's public key from
//! the `devices` tool, then `push_private` with `sealed`) sends the server only the envelope.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload as AeadPayload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use serde_json::{json, Value};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

pub const ALG: &str = "x25519-hkdf-sha256-chacha20poly1305";
const LABEL: &[u8] = b"beatscript-private-push/v1";
/// A payload older (or newer) than this is refused: ten minutes.
pub const MAX_AGE_MS: i64 = 10 * 60 * 1000;
/// The longest envelope accepted (a 16 KB program sealed and base64'd is about 22 KB).
pub const MAX_ENVELOPE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The phone's public key is not an Ed25519 point (or converts to a low-order one).
    BadKey,
    /// Not an envelope of this version.
    BadEnvelope(&'static str),
    /// The ciphertext, nonce, epk or device id was changed, or it was sealed for another key.
    Tampered,
    /// The payload is not ours.
    BadPayload(&'static str),
    Stale,
    Replayed,
    Random,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadKey => f.write_str("the device key is not a usable Ed25519 public key"),
            Error::BadEnvelope(w) => write!(f, "not a private-push envelope: {w}"),
            Error::Tampered => f.write_str("the private push does not open: it was changed in transit or sealed for another device"),
            Error::BadPayload(w) => write!(f, "the private push is not a program: {w}"),
            Error::Stale => f.write_str("the private push is too old (or from the future)"),
            Error::Replayed => f.write_str("the private push was already received"),
            Error::Random => f.write_str("no randomness"),
        }
    }
}

impl std::error::Error for Error {}

/// What a push asks the phone to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// `eval` (play this text) or `load` (load it, leave it stopped unless `play`).
    pub kind: String,
    pub text: String,
    pub play: bool,
    /// 16 random bytes, hex: the replay guard's key.
    pub id: String,
    /// Milliseconds since the epoch when it was sealed.
    pub ts_ms: i64,
}

impl Payload {
    pub fn new(kind: &str, text: &str, play: bool, ts_ms: i64) -> Result<Payload, Error> {
        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).map_err(|_| Error::Random)?;
        Ok(Payload { kind: kind.into(), text: text.into(), play, id: raw.iter().map(|b| format!("{b:02x}")).collect(), ts_ms })
    }

    pub fn to_json(&self) -> String {
        json!({"v": 1, "kind": self.kind, "text": self.text, "play": self.play, "id": self.id, "ts": self.ts_ms}).to_string()
    }

    pub fn from_json(b: &[u8]) -> Result<Payload, Error> {
        let v: Value = serde_json::from_slice(b).map_err(|_| Error::BadPayload("not JSON"))?;
        if v["v"] != 1 {
            return Err(Error::BadPayload("unknown version"));
        }
        let kind = v["kind"].as_str().ok_or(Error::BadPayload("no kind"))?;
        if !matches!(kind, "eval" | "load") {
            return Err(Error::BadPayload("kind is eval or load"));
        }
        let text = v["text"].as_str().ok_or(Error::BadPayload("no text"))?;
        let id = v["id"].as_str().filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit())).ok_or(Error::BadPayload("no id"))?;
        let ts = v["ts"].as_i64().ok_or(Error::BadPayload("no ts"))?;
        Ok(Payload { kind: kind.into(), text: text.into(), play: v["play"] == true, id: id.into(), ts_ms: ts })
    }
}

/// The X25519 public key of an Ed25519 public key (32 bytes).
pub fn recipient(ed25519_public: &[u8; 32]) -> Result<[u8; 32], Error> {
    let vk = ed25519_dalek::VerifyingKey::from_bytes(ed25519_public).map_err(|_| Error::BadKey)?;
    if vk.is_weak() {
        return Err(Error::BadKey);
    }
    Ok(vk.to_montgomery().to_bytes())
}

/// The X25519 secret of an Ed25519 seed (the phone's 32-byte pairing seed).
fn secret_of_seed(seed: &[u8; 32]) -> StaticSecret {
    let sk = ed25519_dalek::SigningKey::from_bytes(seed);
    let mut b = sk.to_scalar_bytes();
    let s = StaticSecret::from(b);
    b.zeroize();
    s
}

fn aad(device_id: &str, epk: &[u8; 32]) -> Vec<u8> {
    let mut a = Vec::with_capacity(LABEL.len() + device_id.len() + 34);
    a.extend_from_slice(LABEL);
    a.push(0);
    a.extend_from_slice(device_id.as_bytes());
    a.push(0);
    a.extend_from_slice(epk);
    a
}

fn derive(shared: &[u8; 32], epk: &[u8; 32], rpk: &[u8; 32]) -> [u8; 32] {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(epk);
    salt[32..].copy_from_slice(rpk);
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&salt), shared).expand(LABEL, &mut key).expect("32 bytes is a valid HKDF length");
    key
}

/// Seal `plaintext` for the phone `device_id` whose Ed25519 public key is `ed25519_public`: the envelope JSON.
pub fn seal(ed25519_public: &[u8; 32], device_id: &str, plaintext: &[u8]) -> Result<String, Error> {
    let rpk = recipient(ed25519_public)?;
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|_| Error::Random)?;
    let esk = StaticSecret::from(seed);
    seed.zeroize();
    let epk = PublicKey::from(&esk).to_bytes();
    let shared = esk.diffie_hellman(&PublicKey::from(rpk));
    if !shared.was_contributory() {
        return Err(Error::BadKey);
    }
    let mut key = derive(shared.as_bytes(), &epk, &rpk);
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| Error::Random)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    key.zeroize();
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), AeadPayload { msg: plaintext, aad: &aad(device_id, &epk) }).map_err(|_| Error::Random)?;
    Ok(json!({"v": 1, "alg": ALG, "epk": B64.encode(epk), "nonce": B64.encode(nonce), "ct": B64.encode(ct)}).to_string())
}

/// Seal a [`Payload`].
pub fn seal_payload(ed25519_public: &[u8; 32], device_id: &str, p: &Payload) -> Result<String, Error> {
    seal(ed25519_public, device_id, p.to_json().as_bytes())
}

/// Whether `s` has the shape of an envelope (what the relay checks before passing a client-sealed one on: it cannot
/// open it).
pub fn envelope_shape_ok(s: &str) -> Result<(), Error> {
    parse(s).map(|_| ())
}

fn b32(v: &Value, k: &'static str) -> Result<[u8; 32], Error> {
    let b = B64.decode(v[k].as_str().ok_or(Error::BadEnvelope(k))?).map_err(|_| Error::BadEnvelope(k))?;
    b.try_into().map_err(|_| Error::BadEnvelope(k))
}

fn parse(s: &str) -> Result<([u8; 32], [u8; 12], Vec<u8>), Error> {
    if s.len() > MAX_ENVELOPE {
        return Err(Error::BadEnvelope("too long"));
    }
    let v: Value = serde_json::from_str(s).map_err(|_| Error::BadEnvelope("not JSON"))?;
    if v["v"] != 1 || v["alg"] != ALG {
        return Err(Error::BadEnvelope("unknown version or algorithm"));
    }
    let epk = b32(&v, "epk")?;
    let nonce: [u8; 12] = B64.decode(v["nonce"].as_str().ok_or(Error::BadEnvelope("nonce"))?).map_err(|_| Error::BadEnvelope("nonce"))?.try_into().map_err(|_| Error::BadEnvelope("nonce"))?;
    let ct = B64.decode(v["ct"].as_str().ok_or(Error::BadEnvelope("ct"))?).map_err(|_| Error::BadEnvelope("ct"))?;
    if ct.len() < 16 {
        return Err(Error::BadEnvelope("ct"));
    }
    Ok((epk, nonce, ct))
}

/// Open an envelope on the phone: its 32-byte pairing seed and its device id. The plaintext.
pub fn open(seed: &[u8; 32], device_id: &str, envelope: &str) -> Result<Vec<u8>, Error> {
    let (epk, nonce, ct) = parse(envelope)?;
    let sk = secret_of_seed(seed);
    let rpk = PublicKey::from(&sk).to_bytes();
    let shared = sk.diffie_hellman(&PublicKey::from(epk));
    if !shared.was_contributory() {
        return Err(Error::Tampered);
    }
    let mut key = derive(shared.as_bytes(), &epk, &rpk);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    key.zeroize();
    cipher.decrypt(Nonce::from_slice(&nonce), AeadPayload { msg: &ct, aad: &aad(device_id, &epk) }).map_err(|_| Error::Tampered)
}

/// Remembers the ids of the pushes it has let through (for [`MAX_AGE_MS`]; older ones are refused anyway).
#[derive(Default, Debug)]
pub struct ReplayGuard {
    seen: std::collections::VecDeque<(i64, String)>,
}

impl ReplayGuard {
    /// Open, parse and check one push: fresh and not seen before.
    pub fn open(&mut self, seed: &[u8; 32], device_id: &str, envelope: &str, now_ms: i64) -> Result<Payload, Error> {
        let pt = open(seed, device_id, envelope)?;
        let p = Payload::from_json(&pt)?;
        if (now_ms - p.ts_ms).abs() > MAX_AGE_MS {
            return Err(Error::Stale);
        }
        while self.seen.front().is_some_and(|(t, _)| now_ms - t > 2 * MAX_AGE_MS) {
            self.seen.pop_front();
        }
        if self.seen.iter().any(|(_, id)| *id == p.id) {
            return Err(Error::Replayed);
        }
        self.seen.push_back((now_ms, p.id.clone()));
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> ([u8; 32], [u8; 32]) {
        let seed = [7u8; 32];
        let pk = ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        (seed, pk)
    }

    #[test]
    fn round_trip() {
        let (seed, pk) = phone();
        let p = Payload::new("eval", "use restricted\ntempo 120\n", true, 1_000_000).unwrap();
        let env = seal_payload(&pk, "devA", &p).unwrap();
        assert!(!env.contains("restricted"), "the envelope carries no plaintext");
        let mut g = ReplayGuard::default();
        assert_eq!(g.open(&seed, "devA", &env, 1_000_500).unwrap(), p);
        assert_eq!(g.open(&seed, "devA", &env, 1_000_600).unwrap_err(), Error::Replayed);
    }

    #[test]
    fn the_server_side_conversion_matches_the_phone_side() {
        let (seed, pk) = phone();
        assert_eq!(recipient(&pk).unwrap(), PublicKey::from(&secret_of_seed(&seed)).to_bytes());
    }

    #[test]
    fn tampering_is_refused() {
        let (seed, pk) = phone();
        let env = seal(&pk, "devA", b"hello").unwrap();
        let mut v: Value = serde_json::from_str(&env).unwrap();
        let mut ct = B64.decode(v["ct"].as_str().unwrap()).unwrap();
        ct[0] ^= 1;
        v["ct"] = json!(B64.encode(&ct));
        assert_eq!(open(&seed, "devA", &v.to_string()).unwrap_err(), Error::Tampered);
        let mut v: Value = serde_json::from_str(&env).unwrap();
        let mut n = B64.decode(v["nonce"].as_str().unwrap()).unwrap();
        n[3] ^= 0x80;
        v["nonce"] = json!(B64.encode(&n));
        assert_eq!(open(&seed, "devA", &v.to_string()).unwrap_err(), Error::Tampered);
        // another device id (the AAD), another key
        assert_eq!(open(&seed, "devB", &env).unwrap_err(), Error::Tampered);
        assert_eq!(open(&[8u8; 32], "devA", &env).unwrap_err(), Error::Tampered);
        assert!(matches!(open(&seed, "devA", "{}").unwrap_err(), Error::BadEnvelope(_)));
    }

    #[test]
    fn stale_is_refused() {
        let (seed, pk) = phone();
        let p = Payload::new("load", "tempo 120\n", false, 0).unwrap();
        let env = seal_payload(&pk, "d", &p).unwrap();
        assert_eq!(ReplayGuard::default().open(&seed, "d", &env, MAX_AGE_MS + 1).unwrap_err(), Error::Stale);
    }

    #[test]
    fn a_weak_key_is_refused() {
        // the identity point: low order
        let mut id = [0u8; 32];
        id[0] = 1;
        assert_eq!(seal(&id, "d", b"x").unwrap_err(), Error::BadKey);
    }
}
