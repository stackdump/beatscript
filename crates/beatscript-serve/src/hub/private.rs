//! The private relay: a program for one of the caller's paired phones, sealed for that phone's key
//! (`beatscript-seal`), handed to the phone's live event stream and forgotten.
//!
//! What this server keeps of a private push: nothing. The envelope (and, when the server seals, the text for the
//! length of the call) lives only in memory; it is never written to the programs store, the render cache, a log, a
//! metric or a trace, no CID is computed or registered, and a phone that is not connected gets nothing (no queue, no
//! retry). The one line logged is the envelope's size and the device id.

use serde_json::{json, Value};

use super::types::SessionEvent;
use super::{gojson, Hub};
use crate::mcp::hubapi::PrivatePush;
use crate::ApiError;

/// The event type a phone receives: `{"type":"private","extra":<envelope>}`.
pub const EVENT: &str = "private";

impl Hub {
    /// A phone's Ed25519 public key (base64url, as paired) when `uid` owns it.
    pub fn device_key(&self, uid: &str, device: &str) -> Option<String> {
        let p = self.pair.lock().unwrap();
        p.devices.get(device).filter(|d| !uid.is_empty() && d.owner == uid).map(|d| d.key.clone())
    }

    /// Seal (or pass on a client-sealed envelope) and deliver to the phone's live stream. `{sent, bytes, device}`.
    pub fn push_private(&self, uid: &str, device: &str, what: PrivatePush, max_program_bytes: usize) -> Result<Value, ApiError> {
        if uid.is_empty() {
            return Err(ApiError::new(401, "auth_required", "a private push needs a signed-in caller: it goes only to your own paired phones"));
        }
        let d = self.pair.lock().unwrap().devices.get(device).filter(|d| d.owner == uid).map(|d| (d.name.clone(), d.key.clone(), d.is_booth()));
        let Some((name, key, booth)) = d else {
            return Err(ApiError::new(404, "not_found", format!("no paired phone has id {} on this account; call devices", gojson::go_quote(device))));
        };
        // a sealed set is for a booth (the server cannot see inside a client-sealed envelope, so only its own sealing is held to this)
        if matches!(what, PrivatePush::Text { kind: "set", .. }) && !booth {
            return Err(ApiError::new(422, "not_a_booth", format!("{name} is a phone; a set goes to a booth")));
        }
        let gone = || ApiError::new(409, "not_connected", format!("{name} is paired but not connected, so nothing was sent and nothing was kept: open BeatScript on the phone and push again"));
        if !self.core.device_status(device).0 {
            return Err(gone());
        }
        let envelope = match what {
            PrivatePush::Sealed(env) => {
                beatscript_seal::envelope_shape_ok(&env).map_err(|e| ApiError::new(400, "bad_request", e.to_string()))?;
                env
            }
            PrivatePush::Text { kind, text, play } => {
                if text.trim().is_empty() {
                    return Err(ApiError::new(400, "bad_request", "argument \"text\" is empty"));
                }
                if text.len() > max_program_bytes {
                    return Err(ApiError::new(413, "too_large", format!("program is {} bytes; the limit is {max_program_bytes}", text.len())));
                }
                let raw = super::pair::b64url_decode(&key).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
                let Some(pk) = raw else { return Err(ApiError::new(409, "bad_device_key", format!("{name}'s pairing key cannot be sealed to: pair it again"))) };
                let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
                let p = beatscript_seal::Payload::new(kind, &text, play, now_ms).map_err(|e| ApiError::internal(e.to_string()))?;
                beatscript_seal::seal_payload(&pk, device, &p).map_err(|e| ApiError::new(409, "bad_device_key", format!("{name}: {e}")))?
            }
        };
        let bytes = envelope.len();
        let ev = SessionEvent { typ: EVENT.into(), extra: Some(envelope), ..Default::default() };
        if !self.core.push_device(device, &ev) {
            return Err(gone());
        }
        // size and device id only
        eprintln!("beatscript-serve: private push: {bytes} bytes to device {device}");
        Ok(json!({"sent": true, "bytes": bytes, "device": device}))
    }
}
