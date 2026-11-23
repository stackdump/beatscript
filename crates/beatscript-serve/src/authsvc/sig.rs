//! Who signed the challenge: Go's `auth.Identity` (the canonical user for a key) and `share.VerifyPersonal`
//! (EIP-191 personal_sign recovered with secp256k1, or a raw Ed25519 signature).

use sha3::{Digest, Keccak256};

/// Go's `api.User`, serialized in its field order: `{"id","kind","address","via"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub kind: &'static str,
    pub address: String,
    pub via: String,
}

impl User {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"id": self.id, "kind": self.kind, "address": self.address, "via": self.via})
    }

    /// Go's `userFromID`: the user a validated uid names.
    pub fn from_id(id: &str, via: &str) -> Option<User> {
        if !beatscript_serve::playlists::auth::valid_uid(id) {
            return None;
        }
        Some(match id.strip_prefix("eip155:") {
            Some(a) => User { id: id.into(), kind: "eth", address: a.into(), via: via.into() },
            None => User { id: id.into(), kind: "ed25519", address: id["ed25519:".len()..].into(), via: via.into() },
        })
    }
}

fn hex_n(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// libsodium's blocklist of small-order Ed25519 encodings (sign bit cleared), as Go's `smallOrderKeys`.
pub const SMALL_ORDER_KEYS: [&str; 7] = [
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0100000000000000000000000000000000000000000000000000000000000000",
    "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
    "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
    "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
];

fn small_order(hex_key: &str) -> bool {
    let Ok(mut raw) = hex::decode(hex_key) else { return true };
    if raw.len() != 32 {
        return true;
    }
    raw[31] &= 0x7f;
    SMALL_ORDER_KEYS.contains(&hex::encode(raw).as_str())
}

/// Go's `Identity`: the error text is Go's (`bad address: …`).
pub fn identity(typ: &str, address: &str) -> Result<User, String> {
    match typ {
        "eth" => {
            let ok = address.strip_prefix("0x").is_some_and(|a| hex_n(a, 40));
            if !ok {
                return Err("bad address: an eth address is 0x and 40 hex digits".into());
            }
            let a = address.to_ascii_lowercase();
            Ok(User { id: format!("eip155:{a}"), kind: "eth", address: a, via: String::new() })
        }
        "ed25519" => {
            let a = address.strip_prefix("0x").unwrap_or(address);
            let a = a.strip_prefix("0X").unwrap_or(a);
            if !hex_n(a, 64) {
                return Err("bad address: an ed25519 public key is 64 hex digits".into());
            }
            let a = a.to_ascii_lowercase();
            if small_order(&a) {
                return Err("bad address: that is not a usable ed25519 public key".into());
            }
            Ok(User { id: format!("ed25519:{a}"), kind: "ed25519", address: a, via: String::new() })
        }
        _ => Err("bad address: type must be eth or ed25519".into()),
    }
}

pub fn keccak(b: &[u8]) -> [u8; 32] {
    Keccak256::digest(b).into()
}

/// The EIP-191 personal_sign digest of `msg`.
pub fn personal_hash(msg: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(format!("\x19Ethereum Signed Message:\n{}", msg.len()).as_bytes());
    h.update(msg);
    h.finalize().into()
}

/// The address (`0x` + 40 lower-case hex) of an uncompressed secp256k1 public key.
pub fn eth_address(vk: &k256::ecdsa::VerifyingKey) -> String {
    let p = vk.to_encoded_point(false);
    format!("0x{}", hex::encode(&keccak(&p.as_bytes()[1..])[12..]))
}

/// Go's `ethRecover`: R‖S‖v (v 0/1 or 27/28) over the personal_sign digest; the recovered address. A high S is
/// accepted, as decred's `RecoverCompact` accepts it (k256 wants it low: it is normalized, the recovery id flipped).
pub fn eth_recover(msg: &[u8], sig: &[u8]) -> Option<String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let mut v = sig[64];
    if v >= 27 {
        v -= 27;
    }
    if v > 1 {
        return None;
    }
    let mut s = Signature::from_slice(&sig[..64]).ok()?;
    if let Some(low) = s.normalize_s() {
        s = low;
        v ^= 1;
    }
    let vk = VerifyingKey::recover_from_prehash(&personal_hash(msg), &s, RecoveryId::from_byte(v)?).ok()?;
    Some(eth_address(&vk))
}

/// Go's `share.VerifyPersonal(msg, typ, address, sigHex)`: `address` is the canonical one `identity` returned.
pub fn verify_personal(msg: &[u8], typ: &str, address: &str, sig_hex: &str) -> bool {
    let Ok(sig) = hex::decode(sig_hex.strip_prefix("0x").unwrap_or(sig_hex)) else { return false };
    match typ {
        "eth" => sig.len() == 65 && eth_recover(msg, &sig).is_some_and(|a| a.eq_ignore_ascii_case(address)),
        "ed25519" => {
            let (Ok(sig), Some(pk)) = (<[u8; 64]>::try_from(sig.as_slice()), hex::decode(address.strip_prefix("0x").unwrap_or(address)).ok().and_then(|p| <[u8; 32]>::try_from(p.as_slice()).ok())) else {
                return false;
            };
            // ed25519.Verify: cofactorless, S canonical, the key as given (dalek's `verify`, not `verify_strict`)
            let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&pk) else { return false };
            ed25519_dalek::Verifier::verify(&vk, msg, &ed25519_dalek::Signature::from_bytes(&sig)).is_ok()
        }
        _ => false,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// MetaMask's personal_sign, as Go's test wallet makes it: R‖S‖(27+recid), 0x-hex.
    pub fn personal_sign(sk: &k256::ecdsa::SigningKey, msg: &str) -> String {
        let (sig, rid) = sk.sign_prehash_recoverable(&personal_hash(msg.as_bytes())).unwrap();
        let mut b = sig.to_bytes().to_vec();
        b.push(27 + rid.to_byte());
        format!("0x{}", hex::encode(b))
    }

    pub fn eth_key(seed: u8) -> (k256::ecdsa::SigningKey, String) {
        let sk = k256::ecdsa::SigningKey::from_slice(&[seed; 32]).unwrap();
        let a = eth_address(sk.verifying_key());
        (sk, a)
    }

    #[test]
    fn eth_known_vector() {
        // the private key 0x01…01's address (well known: 0x1a642f0e3c3af545e7acbd38b07251b3990914f1)
        let (sk, a) = eth_key(1);
        assert_eq!(a, "0x1a642f0e3c3af545e7acbd38b07251b3990914f1");
        let sig = personal_sign(&sk, "hello");
        assert!(verify_personal(b"hello", "eth", &a, &sig));
        assert!(verify_personal(b"hello", "eth", &a, &sig[2..]), "without 0x");
        assert!(!verify_personal(b"hellO", "eth", &a, &sig));
        // v as 0/1 is accepted too
        let mut raw = hex::decode(&sig[2..]).unwrap();
        raw[64] -= 27;
        assert!(verify_personal(b"hello", "eth", &a, &hex::encode(&raw)));
        raw[64] = 2;
        assert!(!verify_personal(b"hello", "eth", &a, &hex::encode(&raw)));
    }

    #[test]
    fn eth_high_s_accepted_as_go() {
        use k256::elliptic_curve::ops::Neg;
        let (sk, a) = eth_key(9);
        let (sig, rid) = sk.sign_prehash_recoverable(&personal_hash(b"m")).unwrap();
        let (r, s) = sig.split_scalars();
        let high = k256::ecdsa::Signature::from_scalars(r, s.as_ref().neg()).unwrap();
        let mut b = high.to_bytes().to_vec();
        b.push(27 + (rid.to_byte() ^ 1));
        assert!(verify_personal(b"m", "eth", &a, &hex::encode(b)));
    }

    #[test]
    fn identities() {
        let u = identity("eth", "0xABCDEF0123456789abcdef0123456789ABCDEF01").unwrap();
        assert_eq!(u.id, "eip155:0xabcdef0123456789abcdef0123456789abcdef01");
        assert!(identity("eth", "ABCDEF0123456789abcdef0123456789ABCDEF0100").is_err());
        assert!(identity("eth", &format!("0X{}", "1".repeat(40))).is_err(), "eth takes 0x only");
        let k = "a".repeat(64);
        assert_eq!(identity("ed25519", &format!("0X{}", k.to_uppercase())).unwrap().address, k);
        assert_eq!(identity("rsa", "x").unwrap_err(), "bad address: type must be eth or ed25519");
    }

    /// sec1_test.go TestSmallOrderEd25519KeysAreNotIdentities: every listed encoding, with and without the sign
    /// bit, with and without 0x, is refused; ordinary keys are not.
    #[test]
    fn small_order_keys_are_not_identities() {
        for k in SMALL_ORDER_KEYS {
            let raw = hex::decode(k).unwrap();
            for top in [0u8, 0x80] {
                let mut b = raw.clone();
                b[31] |= top;
                for form in [hex::encode(&b), format!("0x{}", hex::encode(&b)), hex::encode(&b).to_uppercase()] {
                    assert!(identity("ed25519", &form).is_err(), "{form} accepted");
                }
            }
        }
        for i in 0..50u8 {
            let sk = ed25519_dalek::SigningKey::from_bytes(&[i; 32]);
            assert!(identity("ed25519", &hex::encode(sk.verifying_key().as_bytes())).is_ok());
        }
        // the premise: the identity key's universal signature (R = identity, S = 0) verifies under a
        // cofactorless verifier, which is why the key must be refused before any signature is looked at
        let ident = SMALL_ORDER_KEYS[1];
        let zero_sig = format!("01{}", "0".repeat(126));
        assert!(verify_personal(b"any message at all", "ed25519", ident, &zero_sig));
    }
}
