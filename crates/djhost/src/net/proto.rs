//! djnet's constants, version rule, error codes, pairing proof and the host's remote
//! allowlist (docs/NET-PROTOCOL.md §3-§5, §9). Pure.

use djengine::json::{JsonObject, JsonValue};

pub const PROTO: &str = "djnet";
/// The highest version this build speaks.
pub const VERSION: u32 = 1;
/// The lowest it accepts.
pub const MIN_VERSION: u32 = 1;
/// The host's control port.
pub const DEFAULT_PORT: u16 = 8795;
/// The peer's mix stream port.
pub const STREAM_PORT: u16 = 8796;
/// SSDP search target.
pub const SSDP_ST: &str = "urn:vevr-se:service:djnet:1";
/// The modes a host can offer.
pub const CAPS: [&str; 4] = ["clock", "track", "remote", "stream"];

/// A djnet error: its code (§3) and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetError {
    pub code: &'static str,
    pub message: String,
}

impl NetError {
    pub fn new(code: &'static str, message: impl Into<String>) -> NetError {
        NetError {
            code,
            message: message.into(),
        }
    }
    pub fn unavailable(message: impl Into<String>) -> NetError {
        NetError::new("unavailable", message)
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for NetError {}

/// The version two sides use, or the `version` error (§4).
pub fn negotiate(host: (u32, u32), peer: (u32, u32)) -> Result<u32, NetError> {
    let v = host.0.min(peer.0);
    if v < host.1 || v < peer.1 {
        return Err(NetError::new(
            "version",
            format!(
                "djnet versions don't meet: host speaks {}-{}, peer {}-{}; update the {}",
                host.1,
                host.0,
                peer.1,
                peer.0,
                if host.0 < peer.1 { "desktop" } else { "phone" }
            ),
        ));
    }
    Ok(v)
}

/// `hex(sha256(token ‖ nonce))`: proves the token without sending it.
pub fn proof(token: &str, nonce: &str) -> String {
    let mut b = token.as_bytes().to_vec();
    b.extend_from_slice(nonce.as_bytes());
    hex(&djengine::cid::sha256(&b))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `n` random bytes as hex: /dev/urandom, else a hash of the time and the pid.
pub fn random_hex(n: usize) -> String {
    use std::io::Read;
    let mut b = vec![0u8; n];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let mut seed = format!(
            "{:?}{}{}",
            std::time::SystemTime::now(),
            std::process::id(),
            super::clock::mono_ns()
        )
        .into_bytes();
        let mut out = Vec::new();
        while out.len() < n {
            let h = djengine::cid::sha256(&seed);
            out.extend_from_slice(&h);
            seed = h.to_vec();
        }
        b.copy_from_slice(&out[..n]);
    }
    hex(&b)
}

/// A 6-digit pairing code.
pub fn pair_code() -> String {
    let h = random_hex(4);
    let n = u32::from_str_radix(&h, 16).unwrap_or(0) % 1_000_000;
    format!("{n:06}")
}

/// Constant-time-ish equality for secrets.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// May a paired remote peer call `method` with `params` (§9)? `strip`: the label of the strip
/// the stream plays on, the one input a peer may touch.
pub fn allowed(method: &str, params: &JsonValue, strip: &str) -> Result<(), NetError> {
    let action = params.get("action").and_then(JsonValue::as_str).unwrap_or("");
    let no = |why: String| Err(NetError::new("forbidden", why));
    match method {
        "ping" | "methods" | "state" | "status" | "mixer" | "transport" | "fx" | "aux" | "trick"
        | "autonext" | "clock" => Ok(()),
        "deck" if action == "file" => no("deck file: a local path, not from the network".into()),
        "deck" => Ok(()),
        "net" if matches!(action, "forget" | "trust" | "code" | "peers") => {
            no(format!("net {action}: only on the desktop itself"))
        }
        "net" => Ok(()),
        "input" => {
            let which = params.get("input").and_then(JsonValue::as_str).unwrap_or("");
            if !strip.is_empty() && which.eq_ignore_ascii_case(strip) {
                Ok(())
            } else {
                no(format!("input: only the {strip:?} strip from the network"))
            }
        }
        m => no(format!("{m}: not available over the network")),
    }
}

/// `{"id", "result"}`.
pub fn reply_ok(id: &JsonValue, result: JsonValue) -> String {
    let mut o = JsonObject::new();
    o.set("id", id.clone());
    o.set("result", result);
    JsonValue::Object(o).js_stringified()
}

/// `{"id", "error", "code"}`.
pub fn reply_err(id: &JsonValue, e: &NetError) -> String {
    let mut o = JsonObject::new();
    o.set("id", id.clone());
    o.set("error", JsonValue::str(e.message.clone()));
    o.set("code", JsonValue::str(e.code));
    JsonValue::Object(o).js_stringified()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_meet_at_the_lower_unless_a_side_refuses_it() {
        assert_eq!(negotiate((1, 1), (3, 1)).unwrap(), 1);
        assert_eq!(negotiate((2, 1), (1, 1)).unwrap(), 1);
        let e = negotiate((1, 1), (3, 2)).unwrap_err();
        assert_eq!(e.code, "version");
        assert!(e.message.contains("desktop"), "{}", e.message);
    }

    #[test]
    fn the_proof_hides_the_token_and_depends_on_the_nonce() {
        let p = proof("secret", "n1");
        assert_eq!(p.len(), 64);
        assert!(!p.contains("secret"));
        assert_ne!(p, proof("secret", "n2"));
        assert!(same(&p, &proof("secret", "n1")));
        assert_eq!(pair_code().len(), 6);
        assert_ne!(random_hex(16), random_hex(16));
    }

    #[test]
    fn the_allowlist_keeps_files_gear_and_config_local() {
        let p = |s: &str| JsonValue::parse(s).unwrap();
        assert!(allowed("deck", &p(r#"{"action":"play"}"#), "Phone").is_ok());
        assert!(allowed("deck", &p(r#"{"action":"file","path":"/etc"}"#), "Phone").is_err());
        assert!(allowed("config", &p("{}"), "Phone").is_err());
        assert!(allowed("perform", &p("{}"), "Phone").is_err());
        assert!(allowed("input", &p(r#"{"input":"phone","volume":1}"#), "Phone").is_ok());
        assert!(allowed("input", &p(r#"{"input":"TD-3"}"#), "Phone").is_err());
        assert!(allowed("net", &p(r#"{"action":"forget"}"#), "Phone").is_err());
        assert!(allowed("net", &p(r#"{"action":"track"}"#), "Phone").is_ok());
    }
}
