//! Pairing's state and texts, Go's `server/pair.go` minus the HTTP: open offers (memory only), paired devices
//! (`<data>/devices.json`, written as Go writes it), the signatures seen in the skew window, and the exact texts a
//! phone signs (`btw-pair/1` names a host, `btw-pair/2` the site id). site/PAIRING.md is the protocol.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::engine::{general_purpose::GeneralPurpose, general_purpose::GeneralPurposeConfig, DecodePaddingMode};
use base64::Engine;
use sha2::{Digest, Sha256};

use super::gojson::jstr;
use super::state::NS;

pub const PAIR_PROTO: &str = "btw-pair/1";
pub const PAIR_PROTO2: &str = "btw-pair/2";
pub const PAIR_TTL: i64 = 120 * NS;
pub const MAX_OFFERS_PER_USER: usize = 3;
pub const MAX_DEVICES_PER_USER: usize = 8;
pub const MAX_DEVICES: usize = 10000;
pub const DEVICE_SKEW: i64 = 120 * NS;
pub const MAX_DEVICE_NAME: usize = 64;
pub const MAX_CLAIM_BYTES: usize = 4 << 10;

/// Go's `base64.RawURLEncoding` decoder: no padding, and (not being `Strict`) trailing bits need not be zero; it
/// skips `\r` and `\n`.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    const E: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::URL_SAFE, GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true).with_decode_padding_mode(DecodePaddingMode::RequireNone));
    let clean: String = s.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    E.decode(clean).ok()
}

pub fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// `decodeKey`.
pub fn decode_key(s: &str) -> Result<[u8; 32], &'static str> {
    b64url_decode(s).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()).ok_or("key must be a base64url Ed25519 public key (32 bytes)")
}

/// `KeyFingerprint`: sha256(key)[:4] as "ab12 cd34".
pub fn key_fingerprint(key: &str) -> String {
    let Some(b) = b64url_decode(key) else { return String::new() };
    let x = hex::encode(&Sha256::digest(&b)[..4]);
    format!("{} {}", &x[..4], &x[4..])
}

/// `PairLink`.
pub fn pair_link(public_url: &str, pair_id: &str, token: &str) -> String {
    format!("{}/pair#{pair_id}.{token}", public_url.trim_end_matches('/'))
}

pub fn claim_message(host: &str, pair_id: &str, token: &str, key: &str, name: &str, ts: i64) -> String {
    format!("{PAIR_PROTO} claim\nhost: {host}\npair: {pair_id}\ntoken: {}\nkey: {key}\nname: {name}\nts: {ts}", sha256_hex(token.as_bytes()))
}

pub fn claim_message2(site: &str, pair_id: &str, token: &str, key: &str, name: &str, ts: i64) -> String {
    claim_message2_kind(site, pair_id, token, key, name, KIND_PHONE, ts)
}

/// The v2 claim of a device of `kind`: a booth's has a `kind: booth` line after `name:`; a phone's has none, so a
/// phone's text is what it always was (`vectors-v2.json`; a booth's is `vectors-v2-booth.json`).
pub fn claim_message2_kind(site: &str, pair_id: &str, token: &str, key: &str, name: &str, kind: &str, ts: i64) -> String {
    let kind_line = if kind == KIND_PHONE { String::new() } else { format!("kind: {kind}\n") };
    format!("{PAIR_PROTO2} claim\nsite: {site}\npair: {pair_id}\ntoken: {}\nkey: {key}\nname: {name}\n{kind_line}ts: {ts}", sha256_hex(token.as_bytes()))
}

/// The kinds of paired device (site/PAIRING.md "The booth"): a phone (the default, and every device paired before
/// kinds) and a booth (the desktop DJ app), which also takes `set` pushes.
pub const KIND_PHONE: &str = "phone";
pub const KIND_BOOTH: &str = "booth";

/// A claim's `kind` as stored: absent or empty is a phone; anything but phone or booth is refused.
pub fn device_kind(raw: &str) -> Option<&'static str> {
    match raw {
        "" | KIND_PHONE => Some(KIND_PHONE),
        KIND_BOOTH => Some(KIND_BOOTH),
        _ => None,
    }
}

pub fn request_message(host: &str, dev: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> String {
    format!("{PAIR_PROTO} request\nhost: {host}\ndevice: {dev}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {}", sha256_hex(body))
}

pub fn request_message2(site: &str, dev: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> String {
    format!("{PAIR_PROTO2} request\nsite: {site}\ndevice: {dev}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {}", sha256_hex(body))
}

/// `ed25519.Verify` (cofactorless, RFC 8032 as Go has it; a wrong-length signature is false).
pub fn verify(key: &[u8; 32], msg: &str, sig: &[u8]) -> bool {
    use ed25519_dalek::Verifier;
    let Ok(sig) = <[u8; 64]>::try_from(sig) else { return false };
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(key) else { return false };
    vk.verify(msg.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).is_ok()
}

/// `validDeviceName`.
pub fn valid_device_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= MAX_DEVICE_NAME && !n.chars().any(|r| (r as u32) < 0x20 || r as u32 == 0x7f)
}

/// A paired device (`device`), a phone or a booth: its public key is its only credential, and not a secret.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub key: String,
    pub paired: String,
    /// [`KIND_PHONE`] or [`KIND_BOOTH`].
    pub kind: &'static str,
}

impl Device {
    pub fn is_booth(&self) -> bool {
        self.kind == KIND_BOOTH
    }
}

pub struct Offer {
    pub id: String,
    pub owner: String,
    pub token_hash: [u8; 32],
    pub created: i64,
    pub expires: i64,
    /// waiting | paired | used
    pub state: String,
    pub device: String,
}

pub struct Pairing {
    pub path: PathBuf,
    pub offers: HashMap<String, Offer>,
    pub devices: HashMap<String, Device>,
    pub seen: HashMap<String, i64>,
}

impl Pairing {
    /// `newPairing`: a devices.json that does not parse stops the server, as in Go (saving over it would lose every
    /// pairing).
    pub fn open(data: &Path) -> Result<Pairing, String> {
        let path = data.join("devices.json");
        let mut p = Pairing { path: path.clone(), offers: HashMap::new(), devices: HashMap::new(), seen: HashMap::new() };
        let b = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(p),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let list: Vec<serde_json::Value> = serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))?;
        for d in list {
            let s = |k: &str| d.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            // a record with no kind (every one written before kinds) is a phone; a kind this server does not know is
            // left out rather than guessed
            let Some(kind) = device_kind(&s("kind")) else { continue };
            let dev = Device { id: s("id"), owner: s("owner"), name: s("name"), key: s("key"), paired: s("paired"), kind };
            if decode_key(&dev.key).is_ok() && !dev.id.is_empty() && !dev.owner.is_empty() {
                p.devices.insert(dev.id.clone(), dev);
            }
        }
        Ok(p)
    }

    /// `saveLocked`: json.MarshalIndent (two spaces) of the list sorted by id, 0600, by rename.
    pub fn save(&self) -> std::io::Result<()> {
        let mut list: Vec<&Device> = self.devices.values().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        let b = devices_json(&list);
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(b.as_bytes())?;
        }
        std::fs::rename(&tmp, &self.path)
    }

    pub fn sweep(&mut self, now: i64) {
        self.offers.retain(|_, o| now <= o.expires + PAIR_TTL);
        self.seen.retain(|_, at| now - *at <= 2 * DEVICE_SKEW);
    }

    /// `ownedDevices`: u's devices, most recently paired first.
    pub fn owned(&self, owner: &str) -> Vec<Device> {
        let mut v: Vec<Device> = self.devices.values().filter(|d| d.owner == owner).cloned().collect();
        v.sort_by(|a, b| b.paired.cmp(&a.paired));
        v
    }
}

/// The bytes of devices.json for `list` (already sorted).
pub fn devices_json(list: &[&Device]) -> String {
    if list.is_empty() {
        return "[]".into();
    }
    let mut o = String::from("[\n");
    for (i, d) in list.iter().enumerate() {
        o.push_str("  {\n");
        // a phone's record is written as it always was (no kind); a booth's names its kind last
        let kind = d.kind.to_string();
        let mut fields = vec![("id", &d.id), ("owner", &d.owner), ("name", &d.name), ("key", &d.key), ("paired", &d.paired)];
        if d.is_booth() {
            fields.push(("kind", &kind));
        }
        let last = fields.len() - 1;
        for (j, (k, v)) in fields.iter().enumerate() {
            o.push_str(&format!("    \"{k}\": "));
            jstr(&mut o, v);
            o.push_str(if j < last { ",\n" } else { "\n" });
        }
        o.push_str(if i + 1 < list.len() { "  },\n" } else { "  }\n" });
    }
    o.push(']');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    /// btwsite/testdata/pair/vectors.json (v1) and qr-golden.json, pinned here (the files are outside this repo).
    const VEC1: &str = r#"{
  "claim_message": "btw-pair/1 claim\nhost: btw.bitwrap.io\npair: pairPAIRpairPAIRpair01\ntoken: 449acafbd7a6f31eb4980ebf99d321dcfdb5f2caf2fa322a95860b8e3297a37f\nkey: iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w\nname: Pixel 9\nts: 1795000000000",
  "claim_sig": "yEXq66RBKemYihKtlit3hlaNqvQBC15agAUNWRJEAX4dugwhFykDn2igqzLG2NOHC0gCr9B03YFeAGYZ3F2AAA",
  "key": "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w",
  "request_message": "btw-pair/1 request\nhost: btw.bitwrap.io\ndevice: devDEVdevDEVdevDEVdev1\nmethod: POST\npath: /api/device/report\nts: 1795000000123\nbody: 322b23ab3358fa4126a2ade4d955c51d0cf9556f0d06deda98cc94530e2a4561",
  "request_sig": "xsa0SIF-PsNWEH0B_KtvhGtK1cjjaoqfzn4dbtY5NcHVa23CFU-aSj2Z7_rZQRSA_U2IqXVOePPAGIeVMhskCw"
}"#;
    const QR_GOLDEN: &str = r#"{
  "claim_message": "btw-pair/1 claim\nhost: beatscript.app\npair: AAECAwQFBgcICQoLDA0ODw\ntoken: 53ee3f45e0d6a0002fa2ca51799e594a5760e72bb16506d916a1d84801ce7bf0\nkey: iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w\nname: Pixel 9\nts: 1795000000000",
  "claim_sig": "x9SHBP6Wsy204rE4ThzQwE8nJ-pXXygRDbtsTy7aptvQfiirWTjOiKUHkA2JV0cpAcTQ4FPRMM2wsJccsMOHBg",
  "host": "beatscript.app",
  "key": "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w",
  "name": "Pixel 9",
  "origin": "https://beatscript.app",
  "pair_id": "AAECAwQFBgcICQoLDA0ODw",
  "proto": "btw-pair/1",
  "qr": "https://beatscript.app/pair#AAECAwQFBgcICQoLDA0ODw.8PHy8_T19vf4-fr7_P3-_w",
  "seed_hex": "0101010101010101010101010101010101010101010101010101010101010101",
  "token": "8PHy8_T19vf4-fr7_P3-_w",
  "ts": 1795000000000
}"#;

    fn vectors(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).unwrap()
    }

    /// btwsite/testdata/pair/vectors-v2.json, pinned here as bytes (the file is outside this repo).
    const VEC2: &str = r#"{
  "claim_message": "btw-pair/2 claim\nsite: siteSITEsiteSITEsite01\npair: pairPAIRpairPAIRpair01\ntoken: 449acafbd7a6f31eb4980ebf99d321dcfdb5f2caf2fa322a95860b8e3297a37f\nkey: iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w\nname: Pixel 9\nts: 1795000000000",
  "claim_sig": "JExMxa5mF9J5F8xP4CWaWc5o1HS7IzF2gbaOQLws4jBgVGOYVxj2s1XZ3P5izV4wgV8noeFlfakkomccaKq5Bg",
  "fingerprint": "3475 0f98",
  "key": "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w",
  "site_id": "siteSITEsiteSITEsite01"
}"#;

    #[test]
    fn v2_vectors() {
        let v = vectors(VEC2);
        let key = decode_key(v["key"].as_str().unwrap()).unwrap();
        assert_eq!(key_fingerprint(v["key"].as_str().unwrap()), v["fingerprint"].as_str().unwrap());
        let msg = v["claim_message"].as_str().unwrap();
        // the token whose hash the message carries is not in the file; the message's form is checked by the v1 test
        assert!(verify(&key, msg, &b64url_decode(v["claim_sig"].as_str().unwrap()).unwrap()));
        assert!(!verify(&key, &(msg.to_string() + "x"), &b64url_decode(v["claim_sig"].as_str().unwrap()).unwrap()));
    }

    #[test]
    fn v1_vectors_when_present() {
        let v = vectors(VEC1);
        // whatever the v1 file carries: each *_message has its *_sig under its key
        let key = decode_key(v["key"].as_str().unwrap()).unwrap();
        let obj = v.as_object().unwrap();
        let mut n = 0;
        for (k, m) in obj {
            if let Some(stem) = k.strip_suffix("_message") {
                if let Some(sig) = obj.get(&format!("{stem}_sig")).and_then(|s| s.as_str()) {
                    assert!(verify(&key, m.as_str().unwrap(), &b64url_decode(sig).unwrap()), "{k}");
                    n += 1;
                }
            }
        }
        assert!(n >= 1);
        // qr-golden.json: the link, the claim text and its signature
        let g = vectors(QR_GOLDEN);
        let s = |k: &str| g[k].as_str().unwrap().to_string();
        assert_eq!(pair_link(&s("origin"), &s("pair_id"), &s("token")), s("qr"));
        let msg = claim_message(&s("host"), &s("pair_id"), &s("token"), &s("key"), &s("name"), g["ts"].as_i64().unwrap());
        assert_eq!(msg, s("claim_message"));
        assert!(verify(&decode_key(&s("key")).unwrap(), &msg, &b64url_decode(&s("claim_sig")).unwrap()));
    }

    #[test]
    fn messages_and_links() {
        assert_eq!(pair_link("https://beatscript.app/", "AAECAwQFBgcICQoLDA0ODw", "8PHy"), "https://beatscript.app/pair#AAECAwQFBgcICQoLDA0ODw.8PHy");
        let m = claim_message2("s", "p", "t", "k", "n", 5);
        assert_eq!(m, format!("btw-pair/2 claim\nsite: s\npair: p\ntoken: {}\nkey: k\nname: n\nts: 5", sha256_hex(b"t")));
        assert!(valid_device_name("Pixel 9"));
        assert!(!valid_device_name(""));
        assert!(!valid_device_name("a\nb"));
        assert!(!valid_device_name(&"x".repeat(65)));
    }

    #[test]
    fn devices_file_as_go_writes_it() {
        let d = Device { id: "a".into(), owner: "ed25519:00".into(), name: "<P>".into(), key: "k".into(), paired: "2026-11-20T12:00:00Z".into(), kind: KIND_PHONE };
        assert_eq!(devices_json(&[&d]), "[\n  {\n    \"id\": \"a\",\n    \"owner\": \"ed25519:00\",\n    \"name\": \"\\u003cP\\u003e\",\n    \"key\": \"k\",\n    \"paired\": \"2026-11-20T12:00:00Z\"\n  }\n]");
        assert_eq!(devices_json(&[]), "[]");
        let dir = tempfile::tempdir().unwrap();
        let mut p = Pairing::open(dir.path()).unwrap();
        let k = b64url(&[1u8; 32]);
        p.devices.insert("x".into(), Device { key: k, id: "x".into(), owner: "ed25519:aa".into(), ..d });
        p.save().unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.path().join("devices.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Pairing::open(dir.path()).unwrap().devices.len(), 1);
    }

    /// site/testdata/pair/vectors-v2-booth.json: a booth's v2 claim (the fixed key, seed 32 x 0x01) is the phone's text
    /// with `kind: booth` after `name:`, and its signature verifies; without the line it does not.
    #[test]
    fn v2_booth_vector() {
        let g = vectors(include_str!("../../../../site/testdata/pair/vectors-v2-booth.json"));
        let s = |k: &str| g[k].as_str().unwrap().to_string();
        let msg = claim_message2_kind(&s("site_id"), &s("pair_id"), &s("token"), &s("key"), &s("name"), KIND_BOOTH, g["ts"].as_i64().unwrap());
        assert_eq!(msg, s("claim_message"));
        let key = decode_key(&s("key")).unwrap();
        let sig = b64url_decode(&s("claim_sig")).unwrap();
        assert!(verify(&key, &msg, &sig));
        let as_phone = claim_message2(&s("site_id"), &s("pair_id"), &s("token"), &s("key"), &s("name"), g["ts"].as_i64().unwrap());
        assert!(!as_phone.contains("kind:") && !verify(&key, &as_phone, &sig), "a booth's signature is not a phone's");
        // the phone's text is unchanged by kinds
        assert_eq!(claim_message2_kind("s", "p", "t", "k", "n", KIND_PHONE, 5), claim_message2("s", "p", "t", "k", "n", 5));
        assert_eq!((device_kind(""), device_kind("phone"), device_kind("booth"), device_kind("Booth")), (Some(KIND_PHONE), Some(KIND_PHONE), Some(KIND_BOOTH), None));
    }

    #[test]
    fn booth_record_names_its_kind() {
        let d = Device { id: "b".into(), owner: "ed25519:00".into(), name: "Mac".into(), key: b64url(&[1u8; 32]), paired: "2026-11-27T12:00:00Z".into(), kind: KIND_BOOTH };
        let j = devices_json(&[&d]);
        assert!(j.ends_with("\"paired\": \"2026-11-27T12:00:00Z\",\n    \"kind\": \"booth\"\n  }\n]"), "{j}");
        let dir = tempfile::tempdir().unwrap();
        let mut p = Pairing::open(dir.path()).unwrap();
        p.devices.insert("b".into(), d.clone());
        p.save().unwrap();
        assert_eq!(Pairing::open(dir.path()).unwrap().devices["b"], d);
        // a kind this server does not know is not read as a phone
        std::fs::write(dir.path().join("devices.json"), j.replace("\"booth\"", "\"toaster\"")).unwrap();
        assert!(Pairing::open(dir.path()).unwrap().devices.is_empty());
    }
}
