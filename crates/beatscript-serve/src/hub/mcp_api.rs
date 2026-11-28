//! `mcp::hubapi::HubApi` (P7's view of the hub) over `hub::Hub`: Go's `api.Hub`, `api.Devices` and `api.Queuer` as
//! the MCP tools call them. Values cross as Go's JSON (field order included), parsed from the same writers the HTTP
//! routes use. `uid` "" is an anonymous caller (Go's nil `*api.User`).

use serde_json::Value;

use super::types::{QueueItem, QueueOp, SessionEvent, User};
use super::Hub;
use crate::compose::Fail;
use crate::mcp::hubapi::{self, BoxFut, HubApi};

fn user(uid: &str) -> Option<User> {
    (!uid.is_empty()).then(|| User::of(uid, "token"))
}

fn val(s: String) -> Value {
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

impl HubApi for Hub {
    fn push(&self, uid: &str, session: &str, ev: hubapi::SessionEvent) -> Result<usize, Fail> {
        // a set to a booth's session goes as `set`, with its members (site/PAIRING.md "The booth")
        let booth = ev.set.is_some() && !uid.is_empty() && self.booth_session(uid, session);
        let (typ, extra) = if booth { ("set", ev.set) } else { (ev.kind, None) };
        let ev = SessionEvent { typ: typ.to_string(), text: ev.text, cid: ev.cid, play: ev.play, extra, ..Default::default() };
        Hub::push(self, user(uid).as_ref(), session, &ev).map_err(Fail::from)
    }

    fn state<'a>(&'a self, uid: &'a str, session: &'a str) -> BoxFut<'a, Result<Value, Fail>> {
        Box::pin(async move {
            let u = user(uid);
            Hub::state(self, u.as_ref(), session).await.map(|st| val(st.to_json())).map_err(Fail::from)
        })
    }

    fn sessions(&self, uid: &str) -> Vec<hubapi::SessionInfo> {
        Hub::sessions(self, user(uid).as_ref()).into_iter().map(|s| hubapi::SessionInfo { id: s.id, last_seen: s.last_seen }).collect()
    }

    fn devices(&self, uid: &str) -> Vec<Value> {
        Hub::devices(self, user(uid).as_ref()).into_iter().map(|d| val(d.to_json())).collect()
    }

    fn device_session(&self, uid: &str, device: &str) -> Result<String, Fail> {
        Hub::device_session(self, user(uid).as_ref(), device).map_err(Fail::from)
    }

    fn queue(&self, uid: &str, session: &str, op: hubapi::QueueOp) -> Result<(Value, bool), Fail> {
        let items = op
            .items
            .into_iter()
            .map(|i| QueueItem { cid: i.cid, title: i.title, source: i.source, example: i.example, bars: i.bars, minutes: i.minutes, text: i.text, ..Default::default() })
            .collect();
        Hub::queue(self, user(uid).as_ref(), session, QueueOp { action: op.action, items, remove: op.remove, to: op.to }).map(|(v, sent)| (val(v.to_json()), sent)).map_err(Fail::from)
    }

    fn push_private(&self, uid: &str, device: &str, what: hubapi::PrivatePush, max_program_bytes: usize) -> Result<Value, Fail> {
        Hub::push_private(self, uid, device, what, max_program_bytes).map_err(Fail::from)
    }

    fn device_key(&self, uid: &str, device: &str) -> Option<String> {
        Hub::device_key(self, uid, device)
    }
}
