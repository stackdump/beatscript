//! What the MCP tools need of the hub (Go's `api.Hub`, `api.Devices`, `api.Queuer`), as a trait P6's `hub::Hub`
//! implements. Until it does, `EmptyHub` answers exactly as Go's hub does with no browser session and no paired phone:
//! push reaches nobody, a named session does not exist, the device list is empty.
//!
//! Every value crosses as Go's JSON (`api.SessionState`, `api.DeviceInfo`, `api.QueueView`), field order included, so
//! the tools can relay it as Go relays the structs.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::compose::Fail;

/// `api.SessionEvent`: type eval | load | stop | get.
#[derive(Clone, Debug, Default)]
pub struct SessionEvent {
    pub kind: &'static str,
    pub text: String,
    pub cid: String,
    pub play: bool,
    /// A set's members (`hub::set_extra`) when the text is a set: to a booth's session it goes as a `set` event with
    /// them; anywhere else it is dropped and the event is sent as `kind`.
    pub set: Option<String>,
}

/// `api.SessionInfo`, what `get` needs of it.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    /// RFC 3339
    pub last_seen: String,
}

/// `api.QueueItem` (text: a link item's verified canonical text, in memory only).
#[derive(Clone, Debug, Default)]
pub struct QueueItem {
    pub cid: String,
    pub title: String,
    /// store | example | link
    pub source: String,
    pub example: String,
    pub bars: i64,
    pub minutes: f64,
    pub text: String,
}

/// `api.QueueOp`.
#[derive(Clone, Debug, Default)]
pub struct QueueOp {
    /// list | add | remove | move | clear | skip
    pub action: String,
    pub items: Vec<QueueItem>,
    /// the item key (an id or a CID) of remove, move and skip (skip: empty is the next item)
    pub remove: String,
    /// move: the index the item goes to
    pub to: usize,
}

/// A private push (`push_private`): text the server seals for the phone, or an envelope a client sealed itself.
#[derive(Clone, Debug)]
pub enum PrivatePush {
    Text { kind: &'static str, text: String, play: bool },
    Sealed(String),
}

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The hub as MCP drives it. `uid` is the signed-in caller (the write tools need one); `session` "" means "the
/// caller's own sessions".
pub trait HubApi: Send + Sync {
    /// `Hub.Push`: how many live sessions took the event. A named session that does not exist (or is another
    /// account's) is a `no_session` error.
    fn push(&self, uid: &str, session: &str, ev: SessionEvent) -> Result<usize, Fail>;
    /// `Hub.State`: the latest report of one session (the caller's most recent with session ""), asking the page
    /// when it is stale and waiting briefly. Errors as Go's (`no_session` with its words).
    fn state<'a>(&'a self, uid: &'a str, session: &'a str) -> BoxFut<'a, Result<Value, Fail>>;
    /// `Hub.Sessions`: the caller's live sessions.
    fn sessions(&self, uid: &str) -> Vec<SessionInfo>;
    /// `Devices.Devices`: the caller's paired phones (`api.DeviceInfo` JSON), most recently paired first.
    fn devices(&self, uid: &str) -> Vec<Value>;
    /// `Devices.DeviceSession`: the hub session of one of the caller's phones.
    fn device_session(&self, uid: &str, device: &str) -> Result<String, Fail>;
    /// `Queuer.Queue`: the view (`api.QueueView` JSON) and whether a skip was pushed.
    fn queue(&self, uid: &str, session: &str, op: QueueOp) -> Result<(Value, bool), Fail>;
    /// A private push to one of the caller's phones, relayed in memory only (`hub::private`).
    fn push_private(&self, _uid: &str, device: &str, _what: PrivatePush, _max_program_bytes: usize) -> Result<Value, Fail> {
        Err(Fail::new(404, "not_found", format!("no paired phone has id {} on this account; call devices", go_quote(device))))
    }
    /// The phone's Ed25519 public key (base64url), for a client that seals its own pushes.
    fn device_key(&self, _uid: &str, _device: &str) -> Option<String> {
        None
    }
}

/// The hub with nothing connected (until P6's lands).
pub struct EmptyHub;

fn no_session() -> Fail {
    Fail::new(404, "no_session", "no such session (it expired, was closed, or belongs to another account)")
}

impl HubApi for EmptyHub {
    fn push(&self, _uid: &str, session: &str, _ev: SessionEvent) -> Result<usize, Fail> {
        if session.is_empty() {
            Ok(0)
        } else {
            Err(no_session())
        }
    }
    fn state<'a>(&'a self, _uid: &'a str, session: &'a str) -> BoxFut<'a, Result<Value, Fail>> {
        Box::pin(async move {
            if session.is_empty() {
                Err(Fail::new(409, "no_session", "no browser session is connected for you: open this site in a browser tab, signed in, then retry"))
            } else {
                Err(no_session())
            }
        })
    }
    fn sessions(&self, _uid: &str) -> Vec<SessionInfo> {
        Vec::new()
    }
    fn devices(&self, _uid: &str) -> Vec<Value> {
        Vec::new()
    }
    fn device_session(&self, _uid: &str, device: &str) -> Result<String, Fail> {
        Err(Fail::new(404, "not_found", format!("no paired phone has id {} on this account; call devices", go_quote(device))))
    }
    fn queue(&self, _uid: &str, _session: &str, _op: QueueOp) -> Result<(Value, bool), Fail> {
        Err(no_session())
    }
}

/// Go's `%q` of a plain string (the ids here are `[A-Za-z0-9_-]`).
pub fn go_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
