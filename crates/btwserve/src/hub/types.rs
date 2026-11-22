//! Go's `api` types the hub speaks (`api/api.go`), each with the JSON `encoding/json` writes for it (struct fields
//! in declaration order, `omitempty` honoured, HTML escaped) and, where a request carries one, Go's decoding of it.
//! MCP (P7) uses these as they are: `to_json` is what Go's tools marshal.

use super::gojson::{compact, jfloat, jstr, Bind, Node};

/// `api.User`: who a request is from.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct User {
    pub id: String,
    pub kind: String,
    pub address: String,
    /// "cookie" | "token" | "device"
    pub via: String,
}

impl User {
    /// `userFromID` (and `ownerUser`): kind and address from the id.
    pub fn of(id: &str, via: &str) -> User {
        let (kind, address) = if let Some(a) = id.strip_prefix("eip155:") {
            ("eth", a)
        } else if let Some(a) = id.strip_prefix("ed25519:") {
            ("ed25519", a)
        } else {
            ("", "")
        };
        User { id: id.into(), kind: kind.into(), address: address.into(), via: via.into() }
    }
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"id\":");
        jstr(&mut o, &self.id);
        o.push_str(",\"kind\":");
        jstr(&mut o, &self.kind);
        o.push_str(",\"address\":");
        jstr(&mut o, &self.address);
        o.push_str(",\"via\":");
        jstr(&mut o, &self.via);
        o.push('}');
        o
    }
}

/// `api.SessionEvent`, pushed to a page over SSE. `extra` is compact JSON (a `json.RawMessage`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionEvent {
    pub typ: String,
    pub request_id: String,
    pub text: String,
    pub cid: String,
    pub play: bool,
    pub extra: Option<String>,
}

impl SessionEvent {
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"type\":");
        jstr(&mut o, &self.typ);
        if !self.request_id.is_empty() {
            o.push_str(",\"request_id\":");
            jstr(&mut o, &self.request_id);
        }
        if !self.text.is_empty() {
            o.push_str(",\"text\":");
            jstr(&mut o, &self.text);
        }
        if !self.cid.is_empty() {
            o.push_str(",\"cid\":");
            jstr(&mut o, &self.cid);
        }
        if self.play {
            o.push_str(",\"play\":true");
        }
        if let Some(x) = self.extra.as_ref().filter(|x| !x.is_empty()) {
            o.push_str(",\"extra\":");
            o.push_str(&compact(x));
        }
        o.push('}');
        o
    }
}

/// `api.Presence`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    pub visible: bool,
    pub focused: bool,
    pub audio: String,
    pub idle_ms: i64,
}

pub const PRESENCE_TYPE: &str = "api.Presence";

impl Presence {
    pub fn to_json(&self) -> String {
        let mut o = format!("{{\"visible\":{},\"focused\":{},\"audio\":", self.visible, self.focused);
        jstr(&mut o, &self.audio);
        o.push_str(&format!(",\"idle_ms\":{}}}", self.idle_ms));
        o
    }
    /// a `*api.Presence` field: null makes it nil, an object allocates (keeping nothing of what was there? Go keeps
    /// the old value's fields when the pointer was set; a fresh decode always starts from nil)
    pub fn bind(b: &mut Bind, n: &Node, out: &mut Option<Presence>) {
        if matches!(n.kind, super::gojson::Kind::Null) {
            *out = None;
            return;
        }
        let mut p = out.take().unwrap_or_default();
        if !matches!(n.kind, super::gojson::Kind::Obj(_)) {
            b.object(n, "Presence", PRESENCE_TYPE, &[], |_, _, _| {});
            // a type error leaves the allocated zero value in place, as Go's indirect does
            *out = Some(p);
            return;
        }
        b.object(n, "Presence", PRESENCE_TYPE, &["visible", "focused", "audio", "idle_ms"], |b, i, v| match i {
            0 => b.boolean(v, &mut p.visible),
            1 => b.boolean(v, &mut p.focused),
            2 => b.string(v, &mut p.audio),
            _ => b.int(v, &mut p.idle_ms, "int64"),
        });
        *out = Some(p);
    }
}

/// `api.SessionState`: what the page last reported. `marks` and `diagnostics` are raw JSON as received.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    pub id: String,
    pub text: String,
    pub cid: String,
    pub playing: bool,
    pub source: String,
    pub section: String,
    pub section_num: i64,
    pub sections_total: i64,
    pub bar: i64,
    pub bars_in_section: i64,
    pub phase: i64,
    pub marks: Option<String>,
    pub diagnostics: Option<String>,
    pub presence: Option<Presence>,
    pub ts: String,
    /// What the listener hears, as the page or the phone reports it (djrust `host::deck::audible`: the crossfader, the
    /// master and each deck's level, mute, EQ kills, filters, vocoder). Raw JSON; None from a client too old to send it.
    pub mixer: Option<String>,
}

pub const SESSION_STATE_TYPE: &str = "api.SessionState";

impl SessionState {
    /// The struct's fields, without the braces: MCP's `get` embeds them in a larger object (Go embeds the struct).
    pub fn fields_json(&self) -> String {
        let mut o = String::from("\"id\":");
        jstr(&mut o, &self.id);
        o.push_str(",\"text\":");
        jstr(&mut o, &self.text);
        o.push_str(",\"cid\":");
        jstr(&mut o, &self.cid);
        o.push_str(&format!(",\"playing\":{},\"source\":", self.playing));
        jstr(&mut o, &self.source);
        o.push_str(",\"section\":");
        jstr(&mut o, &self.section);
        o.push_str(&format!(
            ",\"section_num\":{},\"sections_total\":{},\"bar\":{},\"bars_in_section\":{},\"phase\":{}",
            self.section_num, self.sections_total, self.bar, self.bars_in_section, self.phase
        ));
        if let Some(m) = self.marks.as_ref().filter(|m| !m.is_empty()) {
            o.push_str(",\"marks\":");
            o.push_str(&compact(m));
        }
        if let Some(m) = self.diagnostics.as_ref().filter(|m| !m.is_empty()) {
            o.push_str(",\"diagnostics\":");
            o.push_str(&compact(m));
        }
        if let Some(p) = &self.presence {
            o.push_str(",\"presence\":");
            o.push_str(&p.to_json());
        }
        o.push_str(",\"ts\":");
        jstr(&mut o, &self.ts);
        if let Some(m) = self.mixer.as_ref().filter(|m| m.trim_start().starts_with('{')) {
            o.push_str(",\"mixer\":");
            o.push_str(&compact(m));
        }
        o
    }
    pub fn to_json(&self) -> String {
        format!("{{{}}}", self.fields_json())
    }
    pub fn bind(b: &mut Bind, n: &Node, st: &mut SessionState) {
        const F: &[&str] = &["id", "text", "cid", "playing", "source", "section", "section_num", "sections_total", "bar", "bars_in_section", "phase", "marks", "diagnostics", "presence", "ts", "mixer"];
        b.object(n, "SessionState", SESSION_STATE_TYPE, F, |b, i, v| match i {
            0 => b.string(v, &mut st.id),
            1 => b.string(v, &mut st.text),
            2 => b.string(v, &mut st.cid),
            3 => b.boolean(v, &mut st.playing),
            4 => b.string(v, &mut st.source),
            5 => b.string(v, &mut st.section),
            6 => b.int(v, &mut st.section_num, "int"),
            7 => b.int(v, &mut st.sections_total, "int"),
            8 => b.int(v, &mut st.bar, "int"),
            9 => b.int(v, &mut st.bars_in_section, "int"),
            10 => b.int(v, &mut st.phase, "int"),
            11 => b.rawmsg(v, &mut st.marks),
            12 => b.rawmsg(v, &mut st.diagnostics),
            13 => Presence::bind(b, v, &mut st.presence),
            14 => b.string(v, &mut st.ts),
            _ => b.rawmsg(v, &mut st.mixer),
        });
    }
}

/// `api.QueueItem`. `text` is a link item's verified canonical text: never written in a listing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueItem {
    pub id: String,
    pub cid: String,
    pub title: String,
    /// store | example | link
    pub source: String,
    pub example: String,
    pub bars: i64,
    pub minutes: f64,
    pub text: String,
}

impl QueueItem {
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"id\":");
        jstr(&mut o, &self.id);
        o.push_str(",\"cid\":");
        jstr(&mut o, &self.cid);
        if !self.title.is_empty() {
            o.push_str(",\"title\":");
            jstr(&mut o, &self.title);
        }
        o.push_str(",\"source\":");
        jstr(&mut o, &self.source);
        if !self.example.is_empty() {
            o.push_str(",\"example\":");
            jstr(&mut o, &self.example);
        }
        if self.bars != 0 {
            o.push_str(&format!(",\"bars\":{}", self.bars));
        }
        if self.minutes != 0.0 {
            o.push_str(",\"minutes\":");
            o.push_str(&jfloat(self.minutes));
        }
        o.push('}');
        o
    }
}

/// `api.QueueView`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueView {
    pub session: String,
    pub up_next: Vec<QueueItem>,
    pub now_playing: Option<QueueItem>,
    pub position: i64,
    pub paused_reason: String,
    pub listener: String,
    pub auto_advanced: i64,
    pub max_auto: i64,
    pub idle_minutes: i64,
    pub max_items: i64,
}

impl QueueView {
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"session\":");
        jstr(&mut o, &self.session);
        o.push_str(",\"up_next\":[");
        for (i, it) in self.up_next.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            o.push_str(&it.to_json());
        }
        o.push(']');
        if let Some(np) = &self.now_playing {
            o.push_str(",\"now_playing\":");
            o.push_str(&np.to_json());
        }
        o.push_str(&format!(",\"position\":{}", self.position));
        if !self.paused_reason.is_empty() {
            o.push_str(",\"paused_reason\":");
            jstr(&mut o, &self.paused_reason);
        }
        o.push_str(",\"listener\":");
        jstr(&mut o, &self.listener);
        o.push_str(&format!(",\"auto_advanced\":{},\"max_auto_advance\":{},\"idle_minutes\":{},\"max_items\":{}}}", self.auto_advanced, self.max_auto, self.idle_minutes, self.max_items));
        o
    }
}

/// `api.QueueOp`: list | add | remove | clear | skip.
#[derive(Clone, Debug, Default)]
pub struct QueueOp {
    pub action: String,
    pub items: Vec<QueueItem>,
    pub remove: String,
}

/// `api.SessionInfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub owner: String,
    pub device: String,
    pub last_seen: String,
    pub cid: String,
    pub playing: bool,
}

impl SessionInfo {
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"id\":");
        jstr(&mut o, &self.id);
        if !self.owner.is_empty() {
            o.push_str(",\"owner\":");
            jstr(&mut o, &self.owner);
        }
        if !self.device.is_empty() {
            o.push_str(",\"device\":");
            jstr(&mut o, &self.device);
        }
        o.push_str(",\"last_seen\":");
        jstr(&mut o, &self.last_seen);
        if !self.cid.is_empty() {
            o.push_str(",\"cid\":");
            jstr(&mut o, &self.cid);
        }
        o.push_str(&format!(",\"playing\":{}}}", self.playing));
        o
    }
}

/// `api.DeviceInfo`: one paired phone, as MCP's devices tool lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub paired: String,
    pub connected: bool,
    pub last_seen: String,
    pub session: String,
    pub state: Option<SessionState>,
}

impl DeviceInfo {
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\"id\":");
        jstr(&mut o, &self.id);
        o.push_str(",\"name\":");
        jstr(&mut o, &self.name);
        o.push_str(",\"fingerprint\":");
        jstr(&mut o, &self.fingerprint);
        o.push_str(",\"paired\":");
        jstr(&mut o, &self.paired);
        o.push_str(&format!(",\"connected\":{}", self.connected));
        if !self.last_seen.is_empty() {
            o.push_str(",\"last_seen\":");
            jstr(&mut o, &self.last_seen);
        }
        if !self.session.is_empty() {
            o.push_str(",\"session\":");
            jstr(&mut o, &self.session);
        }
        if let Some(st) = &self.state {
            o.push_str(",\"state\":");
            o.push_str(&st.to_json());
        }
        o.push('}');
        o
    }
}

/// `api.ComposeRequest`, as the phone's `POST /api/device/compose` hands it to compose (P7).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComposeRequest {
    pub prompt: String,
    pub genre: String,
    pub bpm: f64,
    pub key: String,
    /// short | medium | long | "<bars>"
    pub length: String,
    pub base_cid: String,
    pub current: String,
}
