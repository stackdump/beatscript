//! The hub's core, Go's `server/sessions.go` and `server/queue.go`: sessions with their SSE subscribers, the
//! reports and "get" round trips, ownership, and the up-next queue with its presence rules. No HTTP here; the
//! routes (hub.rs) and MCP (through `hub::Hub`) call these. Every clock read goes through `Clock`, so the tests
//! move time by hand as Go's do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::gojson::{jstr, q};
use super::types::*;
use crate::ApiError;

pub const MAX_SESSIONS: usize = 5000;
pub const MAX_SESSIONS_PER_USER: usize = 20;
pub const MAX_STREAMS_PER_SESSION: usize = 4;
pub const SESSION_TTL: i64 = 10 * 60 * NS;
pub const STATE_FRESH: i64 = 1500 * MS;
pub const GET_WAIT: Duration = Duration::from_secs(3);
pub const MAX_REPORT_BYTES: usize = 256 << 10;
pub const MAX_STATE_BYTES: usize = 128 << 20;
pub const MAX_STREAMS_PER_IP: usize = 40;
pub const MAX_QUEUE_TEXT_BYTES: usize = 256 << 10;
pub const PRESENCE_STALE: i64 = 2 * 60 * NS;

pub const PAUSE_IDLE_HIDDEN: &str = "idle_hidden";
pub const PAUSE_AUDIO_SUSPENDED: &str = "audio_suspended";
pub const PAUSE_MAX_AUTO: &str = "max_auto_advance";
pub const PAUSE_NO_REPORT: &str = "no_listener_report";
pub const LISTENER_PRESENT: &str = "present";

/// nanoseconds in a second / a millisecond
pub const NS: i64 = 1_000_000_000;
pub const MS: i64 = 1_000_000;

/// Unix nanoseconds, from the wall clock or a test's hand.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    pub fn system() -> Clock {
        Clock(Arc::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64)))
    }
    pub fn manual(at: Arc<std::sync::atomic::AtomicI64>) -> Clock {
        Clock(Arc::new(move || at.load(std::sync::atomic::Ordering::SeqCst)))
    }
    pub fn now(&self) -> i64 {
        (self.0)()
    }
}

/// `newID`: 16 random bytes, base64url (22 characters).
pub fn new_id() -> String {
    use base64::Engine;
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("random");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

pub fn err_no_session() -> ApiError {
    ApiError::new(404, "no_session", "no such session (or it is not yours to drive)")
}

/// The up-next queue's limits (`QueueConfig`, defaults applied).
#[derive(Clone, Copy, Debug)]
pub struct QueueConfig {
    pub idle_minutes: i64,
    pub max_auto: i64,
    pub max_items: usize,
    pub max_per_add: usize,
}

impl QueueConfig {
    pub fn new(idle_minutes: i64, max_auto: i64, max_items: i64, max_per_add: i64) -> QueueConfig {
        let d = |v: i64, d: i64| if v <= 0 { d } else { v };
        QueueConfig { idle_minutes: d(idle_minutes, 30), max_auto: d(max_auto, 8), max_items: d(max_items, 50) as usize, max_per_add: d(max_per_add, 20) as usize }
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig::new(0, 0, 0, 0)
    }
}

#[derive(Default)]
pub struct QueueState {
    pub items: Vec<QueueItem>,
    pub now_playing: Option<QueueItem>,
    pub position: i64,
    pub seq: i64,
    pub text_bytes: usize,
    pub paused: String,
    pub paused_at: i64,
    pub auto: i64,
    pub last_advance: i64,
    pub last_interact: i64,
    pub presence: Option<Presence>,
    pub presence_at: i64,
}

/// Go's zero `time.Time` sits before every real one: 0 here would be 1970, so "never" is the minimum.
const NEVER: i64 = i64::MIN;

impl QueueState {
    fn fresh() -> QueueState {
        QueueState { paused_at: NEVER, last_advance: NEVER, last_interact: NEVER, presence_at: NEVER, ..Default::default() }
    }
    pub fn bytes(q: &Option<QueueState>) -> usize {
        q.as_ref().map_or(0, |q| q.text_bytes + 128 * q.items.len())
    }
    pub fn note_presence(&mut self, p: Option<&Presence>, now: i64) {
        let Some(p) = p else { return };
        let mut cp = p.clone();
        if cp.idle_ms < 0 {
            cp.idle_ms = 0;
        }
        let at = now.saturating_sub(cp.idle_ms.saturating_mul(MS));
        self.presence = Some(cp);
        self.presence_at = now;
        if at > self.last_interact {
            self.last_interact = at;
        }
    }
    pub fn verdict(&self, cfg: &QueueConfig, now: i64) -> &'static str {
        let Some(p) = &self.presence else { return PAUSE_NO_REPORT };
        if now.saturating_sub(self.presence_at) > PRESENCE_STALE {
            return PAUSE_NO_REPORT;
        }
        if p.audio != "running" {
            return PAUSE_AUDIO_SUSPENDED;
        }
        let idle = now.saturating_sub(self.last_interact);
        if !p.visible && idle >= cfg.idle_minutes.saturating_mul(60 * NS) {
            return PAUSE_IDLE_HIDDEN;
        }
        if self.auto >= cfg.max_auto && self.last_interact <= self.last_advance {
            return PAUSE_MAX_AUTO;
        }
        LISTENER_PRESENT
    }
    pub fn settle(&mut self) {
        if self.last_interact > self.last_advance {
            self.auto = 0;
        }
        if !self.paused.is_empty() && self.last_interact > self.paused_at {
            self.paused.clear();
        }
    }
}

pub fn queue_view(q: Option<&mut QueueState>, id: &str, cfg: &QueueConfig, now: i64) -> QueueView {
    let mut v = QueueView { session: id.to_string(), max_auto: cfg.max_auto, idle_minutes: cfg.idle_minutes, max_items: cfg.max_items as i64, listener: PAUSE_NO_REPORT.into(), ..Default::default() };
    let Some(q) = q else { return v };
    q.settle();
    v.up_next = q.items.clone();
    v.now_playing = q.now_playing.clone();
    v.position = q.position;
    v.paused_reason = q.paused.clone();
    v.auto_advanced = q.auto;
    v.listener = q.verdict(cfg, now).into();
    v
}

/// `queueEvent`: `{"queue": view, ...extra}` (a map: keys sorted).
pub fn queue_event(v: &QueueView, action: Option<&str>) -> SessionEvent {
    queue_event_item(v, action, None)
}

/// A queue event that names an item: `{"action": "skip", "item": "q3", "queue": view}` (skip to that item).
pub fn queue_event_item(v: &QueueView, action: Option<&str>, item: Option<&str>) -> SessionEvent {
    let mut o = String::from("{");
    if let Some(a) = action {
        o.push_str("\"action\":");
        jstr(&mut o, a);
        o.push(',');
    }
    if let Some(i) = item {
        o.push_str("\"item\":");
        jstr(&mut o, i);
        o.push(',');
    }
    o.push_str("\"queue\":");
    o.push_str(&v.to_json());
    o.push('}');
    SessionEvent { typ: "queue".into(), extra: Some(o), ..Default::default() }
}

/// One queue operation on session `sid` (found and allowed by the caller): (view, sent). `op.remove` is the item key
/// (an id from the view, q1 ..., or a CID: the first queued item with it) for remove, move and skip.
fn queue_op(h: &mut Inner, sid: &str, op: QueueOp, cfg: &QueueConfig, max_state: usize, now: i64) -> Result<(QueueView, bool), ApiError> {
    let hbytes = h.bytes;
    let Some(s) = h.sessions.get_mut(sid) else { return Err(err_no_session()) };
    let q = s.queue.get_or_insert_with(QueueState::fresh);
    q.settle();
    let delta: isize;
    let qb = |q: &QueueState| (q.text_bytes + 128 * q.items.len()) as isize;
    let find = |q: &QueueState, key: &str| -> Result<usize, ApiError> {
        let key = key.trim();
        q.items.iter().position(|it| it.id == key || it.cid == key).ok_or_else(|| ApiError::new(404, "not_found", format!("no queued item has id or cid {}; call queue with action list", super::gojson::go_quote(key))))
    };
    match op.action.as_str() {
        "list" => {
            let v = queue_view(Some(q), sid, cfg, now);
            return Ok((v, false));
        }
        "add" => {
            if op.items.is_empty() {
                return Err(ApiError::new(400, "bad_request", "add needs at least one item"));
            }
            if op.items.len() > cfg.max_per_add {
                return Err(ApiError::new(413, "too_large", format!("one add takes at most {} items; this one has {}", cfg.max_per_add, op.items.len())));
            }
            if q.items.len() + op.items.len() > cfg.max_items {
                return Err(ApiError::new(413, "too_large", format!("the queue holds at most {} items; it has {} and this add brings {}. Remove or clear some first.", cfg.max_items, q.items.len(), op.items.len())));
            }
            let add: usize = op.items.iter().map(|i| i.text.len()).sum();
            if q.text_bytes + add > MAX_QUEUE_TEXT_BYTES {
                return Err(ApiError::new(413, "too_large", format!("the queue's link items may hold at most {} KiB of program text; store the programs (share or render) and queue their CIDs instead", MAX_QUEUE_TEXT_BYTES >> 10)));
            }
            if hbytes + add + 128 * op.items.len() > max_state {
                return Err(ApiError::new(503, "busy", "the server is holding too much session state; try again later").retry(60));
            }
            let before = qb(q);
            for mut it in op.items {
                q.seq += 1;
                it.id = format!("q{}", q.seq);
                q.text_bytes += it.text.len();
                q.items.push(it);
            }
            delta = qb(q) - before;
        }
        "remove" => {
            let idx = find(q, &op.remove)?;
            let before = qb(q);
            q.text_bytes -= q.items[idx].text.len();
            q.items.remove(idx);
            delta = qb(q) - before;
        }
        "move" => {
            // reorder: the item goes to index `to` of the queue as it is after taking it out (past the end: last)
            let idx = find(q, &op.remove)?;
            let it = q.items.remove(idx);
            let to = op.to.min(q.items.len());
            q.items.insert(to, it);
            delta = 0;
        }
        "clear" => {
            delta = -qb(q);
            q.items.clear();
            q.text_bytes = 0;
        }
        "skip" => {
            // skip: the next item now, or with an item key that item (the page plays it, taking it out of the queue)
            let key = op.remove.trim().to_string();
            let pick = if key.is_empty() { None } else { Some(q.items[find(q, &key)?].id.clone()) };
            let v = queue_view(Some(q), sid, cfg, now);
            if q.items.is_empty() || !v.paused_reason.is_empty() || v.listener != LISTENER_PRESENT {
                return Ok((v, false));
            }
            let sent = h.deliver(sid, &queue_event_item(&v, Some("skip"), pick.as_deref()));
            return Ok((v, sent));
        }
        a => return Err(ApiError::new(400, "bad_request", format!("unknown queue action {} (add, list, remove, move, clear, skip)", super::gojson::go_quote(a)))),
    }
    let v = queue_view(Some(q), sid, cfg, now);
    h.bytes = (h.bytes as isize + delta).max(0) as usize;
    h.deliver(sid, &queue_event(&v, None));
    Ok((v, false))
}

pub struct Sub {
    pub id: u64,
    pub seq: u64,
    pub tx: mpsc::Sender<SessionEvent>,
}

pub struct Session {
    pub id: String,
    pub owner: String,
    pub device: String,
    pub created: i64,
    pub last_seen: i64,
    pub subs: Vec<Sub>,
    pub state: Option<SessionState>,
    pub state_at: i64,
    pub pending: HashMap<String, oneshot::Sender<SessionState>>,
    pub queue: Option<QueueState>,
    pub tokens: f64,
    pub tokens_at: i64,
}

#[derive(Default)]
pub struct Inner {
    pub sessions: HashMap<String, Session>,
    pub closed: bool,
    pub seq: u64,
    pub sub_ids: u64,
    pub bytes: usize,
}

/// The subscriber handle a stream holds: its events, and the ids to unsubscribe with.
pub struct Subscription {
    pub session: String,
    pub sub: u64,
    pub rx: mpsc::Receiver<SessionEvent>,
}

pub struct Core {
    pub clock: Clock,
    pub queue_cfg: QueueConfig,
    pub max_state_bytes: usize,
    pub inner: Mutex<Inner>,
    pub streams: Mutex<HashMap<String, usize>>,
}

fn state_bytes(st: &Option<SessionState>) -> usize {
    st.as_ref().map_or(0, |st| st.text.len() + st.marks.as_ref().map_or(0, |m| m.len()) + st.diagnostics.as_ref().map_or(0, |m| m.len()) + st.cid.len() + st.source.len() + st.section.len() + 512)
}

impl Inner {
    pub fn drop_session(&mut self, id: &str) {
        if let Some(s) = self.sessions.remove(id) {
            self.bytes = self.bytes.saturating_sub(state_bytes(&s.state) + QueueState::bytes(&s.queue));
            // the subscribers' senders go with it: their streams end
        }
    }

    /// `deliverLocked`: to every subscriber; one whose buffer is full is cut off. True if one took it.
    pub fn deliver(&mut self, id: &str, ev: &SessionEvent) -> bool {
        let Some(s) = self.sessions.get_mut(id) else { return false };
        let mut ok = false;
        s.subs.retain(|sub| match sub.tx.try_send(ev.clone()) {
            Ok(()) => {
                ok = true;
                true
            }
            Err(_) => false,
        });
        ok
    }

    fn set_state(&mut self, id: &str, st: SessionState, now: i64, max: usize) {
        let Some(s) = self.sessions.get_mut(id) else { return };
        let new = Some(st);
        self.bytes = (self.bytes + state_bytes(&new)).saturating_sub(state_bytes(&s.state));
        s.state = new;
        s.state_at = now;
        s.last_seen = now;
        if self.bytes <= max {
            return;
        }
        let mut held: Vec<(i64, String)> = self.sessions.values().filter(|o| o.id != id && o.state.is_some()).map(|o| (o.state_at, o.id.clone())).collect();
        held.sort();
        for (_, oid) in held {
            if self.bytes <= max {
                break;
            }
            let o = self.sessions.get_mut(&oid).unwrap();
            self.bytes = self.bytes.saturating_sub(state_bytes(&o.state));
            o.state = None;
        }
    }

    pub fn sweep(&mut self, now: i64) {
        let cut = now - SESSION_TTL;
        let dead: Vec<String> = self.sessions.values().filter(|s| s.subs.is_empty() && s.last_seen < cut).map(|s| s.id.clone()).collect();
        for id in dead {
            self.drop_session(&id);
        }
    }

    /// `pageAccess`: the id is the capability, but a signed-in user who is not the owner is refused.
    fn page_access(&self, id: &str, u: Option<&User>) -> bool {
        match self.sessions.get(id) {
            None => false,
            Some(s) => !(!s.owner.is_empty() && u.is_some_and(|u| u.id != s.owner)),
        }
    }

    fn claim(&mut self, id: &str, u: Option<&User>) {
        if let (Some(s), Some(u)) = (self.sessions.get_mut(id), u) {
            if s.owner.is_empty() {
                s.owner = u.id.clone();
            }
        }
    }

    /// `targetLocked`: the named session (owned by u or anonymous) or all of u's, most recently seen first.
    pub fn targets(&self, u: Option<&User>, id: &str) -> Result<Vec<String>, ApiError> {
        let Some(u) = u else { return Err(ApiError::new(401, "auth_required", "driving a browser session needs a signed-in caller")) };
        if !id.is_empty() {
            return match self.sessions.get(id) {
                Some(s) if s.owner.is_empty() || s.owner == u.id => Ok(vec![id.to_string()]),
                _ => Err(err_no_session()),
            };
        }
        let mut out: Vec<(i64, String)> = self.sessions.values().filter(|s| s.owner == u.id).map(|s| (s.last_seen, s.id.clone())).collect();
        out.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(out.into_iter().map(|x| x.1).collect())
    }

    pub fn device_session(&self, dev: &str) -> Option<&Session> {
        self.sessions.values().find(|s| s.device == dev)
    }
}

impl Core {
    pub fn new(clock: Clock, queue_cfg: QueueConfig) -> Core {
        Core { clock, queue_cfg, max_state_bytes: MAX_STATE_BYTES, inner: Mutex::new(Inner::default()), streams: Mutex::new(HashMap::new()) }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// `create`: a session owned by owner ("" = anonymous).
    pub fn create(&self, owner: &str) -> Result<String, ApiError> {
        let now = self.now();
        let mut h = self.inner.lock().unwrap();
        h.sweep(now);
        if h.sessions.len() >= MAX_SESSIONS {
            return Err(ApiError::new(503, "busy", "too many open sessions; try again later").retry(60));
        }
        if !owner.is_empty() {
            let mut n = 0;
            let mut oldest: Option<(i64, String)> = None;
            for s in h.sessions.values().filter(|s| s.owner == owner) {
                n += 1;
                if !s.device.is_empty() {
                    continue;
                }
                if oldest.as_ref().is_none_or(|o| s.last_seen < o.0) {
                    oldest = Some((s.last_seen, s.id.clone()));
                }
            }
            if n >= MAX_SESSIONS_PER_USER {
                if let Some((_, id)) = oldest {
                    h.drop_session(&id);
                }
            }
        }
        let id = new_id();
        h.sessions.insert(
            id.clone(),
            Session { id: id.clone(), owner: owner.to_string(), device: String::new(), created: now, last_seen: now, subs: Vec::new(), state: None, state_at: NEVER, pending: HashMap::new(), queue: None, tokens: 20.0, tokens_at: now },
        );
        Ok(id)
    }

    /// `subscribe`: attach an SSE stream (the oldest of more than four goes).
    pub fn subscribe(&self, id: &str, u: Option<&User>) -> Result<Subscription, ApiError> {
        let now = self.now();
        let mut h = self.inner.lock().unwrap();
        if h.closed {
            return Err(ApiError::new(503, "busy", "the server is shutting down"));
        }
        if !h.page_access(id, u) {
            return Err(err_no_session());
        }
        h.seq += 1;
        h.sub_ids += 1;
        let (seq, sid) = (h.seq, h.sub_ids);
        h.claim(id, u);
        let (tx, rx) = mpsc::channel(16);
        let s = h.sessions.get_mut(id).unwrap();
        while s.subs.len() >= MAX_STREAMS_PER_SESSION {
            let i = s.subs.iter().enumerate().min_by_key(|(_, x)| x.seq).map(|(i, _)| i).unwrap();
            s.subs.remove(i);
        }
        s.subs.push(Sub { id: sid, seq, tx });
        s.last_seen = now;
        Ok(Subscription { session: id.to_string(), sub: sid, rx })
    }

    pub fn unsubscribe(&self, id: &str, sub: u64) {
        let now = self.now();
        let mut h = self.inner.lock().unwrap();
        if let Some(s) = h.sessions.get_mut(id) {
            s.subs.retain(|x| x.id != sub);
            s.last_seen = now;
        }
    }

    /// `report`: (found, limited).
    pub fn report(&self, id: &str, u: Option<&User>, mut st: SessionState) -> (bool, bool) {
        let now = self.now();
        let mut h = self.inner.lock().unwrap();
        if !h.page_access(id, u) {
            return (false, false);
        }
        h.claim(id, u);
        let s = h.sessions.get_mut(id).unwrap();
        s.tokens = (s.tokens + (now - s.tokens_at) as f64 / 1e9 * 10.0).min(20.0);
        s.tokens_at = now;
        if s.tokens < 1.0 {
            return (true, true);
        }
        s.tokens -= 1.0;
        st.id = id.to_string();
        st.ts = rfc3339_nano(now);
        if let Some(p) = &st.presence {
            s.queue.get_or_insert_with(QueueState::fresh).note_presence(Some(p), now);
        }
        let max = self.max_state_bytes;
        h.set_state(id, st, now, max);
        (true, false)
    }

    /// `reply`: answers a pending "get"; an unknown request id changes nothing.
    pub fn reply(&self, id: &str, u: Option<&User>, request_id: &str, mut st: SessionState) -> bool {
        let now = self.now();
        let mut h = self.inner.lock().unwrap();
        if !h.page_access(id, u) {
            return false;
        }
        let Some(ch) = h.sessions.get_mut(id).unwrap().pending.remove(request_id) else { return true };
        st.id = id.to_string();
        st.ts = rfc3339_nano(now);
        let max = self.max_state_bytes;
        h.set_state(id, st.clone(), now, max);
        let _ = ch.send(st);
        true
    }

    /// `Push`.
    pub fn push(&self, u: Option<&User>, session: &str, ev: &SessionEvent) -> Result<usize, ApiError> {
        if !matches!(ev.typ.as_str(), "eval" | "load" | "stop" | "get") {
            return Err(ApiError::new(400, "bad_request", format!("unknown session event type {} (eval, load, stop, get)", super::gojson::go_quote(&ev.typ))));
        }
        let mut h = self.inner.lock().unwrap();
        let targets = h.targets(u, session)?;
        Ok(targets.iter().filter(|id| h.deliver(id, ev)).count())
    }

    /// `State`: the latest report; a stale one makes the hub ask the page ("get") and wait up to three seconds.
    pub async fn state(&self, u: Option<&User>, session: &str) -> Result<SessionState, ApiError> {
        let (wait, req, sid, last) = {
            let now = self.now();
            let mut h = self.inner.lock().unwrap();
            let targets = h.targets(u, session)?;
            let Some(sid) = targets.first().cloned() else {
                return Err(ApiError::new(409, "no_session", "no browser session is connected for you: open this site in a browser tab, signed in, then retry"));
            };
            let s = h.sessions.get_mut(&sid).unwrap();
            if let Some(st) = &s.state {
                if now - s.state_at < STATE_FRESH {
                    return Ok(st.clone());
                }
            }
            let last = s.state.as_ref().filter(|_| now - s.state_at < SESSION_TTL).cloned();
            let mut wait = None;
            let mut req = String::new();
            if !s.subs.is_empty() {
                req = new_id();
                let (tx, rx) = oneshot::channel();
                s.pending.insert(req.clone(), tx);
                wait = Some(rx);
                h.deliver(&sid, &SessionEvent { typ: "get".into(), request_id: req.clone(), ..Default::default() });
            }
            (wait, req, sid, last)
        };
        if let Some(rx) = wait {
            if let Ok(Ok(st)) = tokio::time::timeout(GET_WAIT, rx).await {
                return Ok(st);
            }
            let mut h = self.inner.lock().unwrap();
            if let Some(s) = h.sessions.get_mut(&sid) {
                s.pending.remove(&req);
            }
        }
        last.ok_or_else(|| ApiError::new(409, "no_session", "the browser did not answer (is the page still open?)"))
    }

    /// `Sessions`: u's sessions, most recently seen first.
    pub fn sessions(&self, u: Option<&User>) -> Vec<SessionInfo> {
        let Some(u) = u else { return Vec::new() };
        let h = self.inner.lock().unwrap();
        let mut out: Vec<SessionInfo> = h
            .sessions
            .values()
            .filter(|s| s.owner == u.id)
            .map(|s| SessionInfo {
                id: s.id.clone(),
                owner: s.owner.clone(),
                device: s.device.clone(),
                last_seen: rfc3339(s.last_seen),
                cid: s.state.as_ref().map(|x| x.cid.clone()).unwrap_or_default(),
                playing: s.state.as_ref().is_some_and(|x| x.playing),
            })
            .collect();
        out.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        out
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().sessions.len()
    }

    pub fn sweep(&self) {
        let now = self.now();
        self.inner.lock().unwrap().sweep(now);
    }

    // ---------- the queue ----------

    /// `Queue` (api.Queuer): (view, sent). MCP's side: the caller's named session, else its most recently seen one.
    pub fn queue(&self, u: Option<&User>, session: &str, op: QueueOp) -> Result<(QueueView, bool), ApiError> {
        let now = self.now();
        let (cfg, max_state) = (self.queue_cfg, self.max_state_bytes);
        let mut h = self.inner.lock().unwrap();
        let targets = h.targets(u, session)?;
        let Some(sid) = targets.first().cloned() else { return Err(err_no_session()) };
        queue_op(&mut h, &sid, op, &cfg, max_state, now)
    }

    /// The page's own writes to its queue (`POST /api/session/<id>/queue`): add, remove, move, clear. The id is the
    /// capability, as for the page's other routes (a signed-in other user than the owner gets None: 404).
    pub fn page_queue_write(&self, id: &str, u: Option<&User>, op: QueueOp) -> Option<Result<QueueView, ApiError>> {
        let now = self.now();
        let (cfg, max_state) = (self.queue_cfg, self.max_state_bytes);
        let mut h = self.inner.lock().unwrap();
        if !h.page_access(id, u) {
            return None;
        }
        h.claim(id, u);
        if let Some(s) = h.sessions.get_mut(id) {
            s.last_seen = now;
        }
        if !matches!(op.action.as_str(), "add" | "remove" | "move" | "clear") {
            return Some(Err(ApiError::new(400, "bad_request", "action must be add, remove, move or clear")));
        }
        Some(queue_op(&mut h, id, op, &cfg, max_state, now).map(|(v, _)| v))
    }

    /// `next`: the page asks for the item to play now. None: unknown session (or not this user's).
    pub fn next(&self, id: &str, u: Option<&User>, p: Option<&Presence>) -> Option<(Option<QueueItem>, QueueView)> {
        self.next_pick(id, u, p, None)
    }

    /// `next`, or with `pick` (an item id or CID) that item rather than the first: a row the person chose to play now.
    /// The same presence rules decide it; a pick the queue no longer has answers no item.
    pub fn next_pick(&self, id: &str, u: Option<&User>, p: Option<&Presence>, pick: Option<&str>) -> Option<(Option<QueueItem>, QueueView)> {
        let now = self.now();
        let cfg = self.queue_cfg;
        let mut h = self.inner.lock().unwrap();
        if !h.page_access(id, u) {
            return None;
        }
        h.claim(id, u);
        let s = h.sessions.get_mut(id).unwrap();
        s.last_seen = now;
        let q = s.queue.get_or_insert_with(QueueState::fresh);
        q.note_presence(p, now);
        q.settle();
        if !q.paused.is_empty() || q.items.is_empty() {
            return Some((None, queue_view(Some(q), id, &cfg, now)));
        }
        let why = q.verdict(&cfg, now);
        if why != LISTENER_PRESENT {
            q.paused = why.into();
            q.paused_at = now;
            let v = queue_view(Some(q), id, &cfg, now);
            h.deliver(id, &queue_event(&v, None));
            return Some((None, v));
        }
        let idx = match pick.map(str::trim).filter(|k| !k.is_empty()) {
            None => 0,
            Some(k) => match q.items.iter().position(|it| it.id == k || it.cid == k) {
                Some(i) => i,
                None => return Some((None, queue_view(Some(q), id, &cfg, now))),
            },
        };
        let before = q.text_bytes + 128 * q.items.len();
        let it = q.items.remove(idx);
        q.text_bytes -= it.text.len();
        let after = q.text_bytes + 128 * q.items.len();
        let mut np = it.clone();
        np.text.clear();
        q.now_playing = Some(np);
        q.position += 1;
        q.auto += 1;
        q.last_advance = now;
        let v = queue_view(Some(q), id, &cfg, now);
        h.bytes = (h.bytes + after).saturating_sub(before);
        h.deliver(id, &queue_event(&v, None));
        Some((Some(it), v))
    }

    /// `pageQueue`.
    pub fn page_queue(&self, id: &str, u: Option<&User>) -> Option<QueueView> {
        let now = self.now();
        let cfg = self.queue_cfg;
        let mut h = self.inner.lock().unwrap();
        if !h.page_access(id, u) {
            return None;
        }
        let s = h.sessions.get_mut(id).unwrap();
        Some(queue_view(s.queue.as_mut(), id, &cfg, now))
    }

    // ---------- devices (pair.go's hub side) ----------

    pub fn device_session_id(&self, owner: &str, dev: &str) -> String {
        let h = self.inner.lock().unwrap();
        h.device_session(dev).filter(|s| s.owner == owner).map(|s| s.id.clone()).unwrap_or_default()
    }

    /// `subscribeDevice`: find or make the device's session and attach a stream.
    pub fn subscribe_device(&self, owner: &str, dev: &str) -> Result<Subscription, ApiError> {
        let existing = {
            let mut h = self.inner.lock().unwrap();
            match h.device_session(dev).map(|s| (s.id.clone(), s.owner.clone())) {
                Some((id, o)) if o != owner => {
                    h.drop_session(&id);
                    None
                }
                x => x.map(|x| x.0),
            }
        };
        let id = match existing {
            Some(id) => id,
            None => {
                let id = self.create(owner)?;
                if let Some(s) = self.inner.lock().unwrap().sessions.get_mut(&id) {
                    s.device = dev.to_string();
                }
                id
            }
        };
        self.subscribe(&id, Some(&User::of(owner, "")))
    }

    pub fn push_device(&self, dev: &str, ev: &SessionEvent) -> bool {
        let mut h = self.inner.lock().unwrap();
        let Some(id) = h.device_session(dev).map(|s| s.id.clone()) else { return false };
        h.deliver(&id, ev)
    }

    /// `dropDevice`: tell the stream it is unpaired, end the session.
    pub fn drop_device(&self, dev: &str, why: &str) {
        let mut h = self.inner.lock().unwrap();
        let Some(id) = h.device_session(dev).map(|s| s.id.clone()) else { return };
        let extra = format!("{{\"reason\":{}}}", q(why));
        h.deliver(&id, &SessionEvent { typ: "unpaired".into(), extra: Some(extra), ..Default::default() });
        h.drop_session(&id);
    }

    /// `deviceStatus`: (connected, last seen, last report).
    pub fn device_status(&self, dev: &str) -> (bool, Option<i64>, Option<SessionState>) {
        let h = self.inner.lock().unwrap();
        match h.device_session(dev) {
            None => (false, None, None),
            Some(s) => (!s.subs.is_empty(), Some(s.last_seen), s.state.clone()),
        }
    }

    // ---------- open streams per address ----------

    pub fn acquire_stream(&self, key: &str) -> bool {
        let mut m = self.streams.lock().unwrap();
        let n = m.entry(key.to_string()).or_insert(0);
        if *n >= MAX_STREAMS_PER_IP {
            if *n == 0 {
                m.remove(key);
            }
            return false;
        }
        *n += 1;
        true
    }

    pub fn release_stream(&self, key: &str) {
        let mut m = self.streams.lock().unwrap();
        match m.get_mut(key) {
            Some(n) if *n > 1 => *n -= 1,
            _ => {
                m.remove(key);
            }
        }
    }

    pub fn close_all(&self) {
        let mut h = self.inner.lock().unwrap();
        h.closed = true;
        for s in h.sessions.values_mut() {
            s.subs.clear();
        }
    }
}

// ---------- time as Go formats it ----------

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn split(ns: i64) -> (String, i64) {
    let secs = ns.div_euclid(NS);
    let frac = ns.rem_euclid(NS);
    let (y, mo, d) = civil(secs.div_euclid(86400));
    let t = secs.rem_euclid(86400);
    (format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60), frac)
}

/// `t.UTC().Format(time.RFC3339)`
pub fn rfc3339(ns: i64) -> String {
    split(ns).0 + "Z"
}

/// `t.UTC().Format(time.RFC3339Nano)`: the fraction without trailing zeros
pub fn rfc3339_nano(ns: i64) -> String {
    let (s, frac) = split(ns);
    if frac == 0 {
        return s + "Z";
    }
    let f = format!("{frac:09}");
    format!("{s}.{}Z", f.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Go's queue_test.go: a hub on a hand-moved clock, with one session owned by alice.
    struct QEnv {
        at: Arc<AtomicI64>,
        h: Core,
        id: String,
        u: User,
    }

    fn t0() -> i64 {
        // 2026-11-20T12:00:00Z
        1_795_176_000 * NS
    }

    fn qenv(cfg: QueueConfig) -> QEnv {
        let at = Arc::new(AtomicI64::new(t0()));
        let h = Core::new(Clock::manual(at.clone()), cfg);
        let id = h.create("alice").unwrap();
        QEnv { at, h, id, u: User::of("alice", "") }
    }

    impl QEnv {
        fn adv(&self, d: i64) {
            self.at.fetch_add(d, Ordering::SeqCst);
        }
        fn now(&self) -> i64 {
            self.at.load(Ordering::SeqCst)
        }
        fn add(&self, n: usize) -> QueueView {
            let items = (0..n).map(|i| QueueItem { cid: format!("zCID{}{}", "x".repeat(20), (b'a' + (i % 26) as u8) as char), source: "store".into(), ..Default::default() }).collect();
            self.h.queue(Some(&self.u), &self.id, QueueOp { action: "add".into(), items, ..Default::default() }).unwrap().0
        }
        fn op(&self, a: &str) -> Result<(QueueView, bool), ApiError> {
            self.h.queue(Some(&self.u), &self.id, QueueOp { action: a.into(), ..Default::default() })
        }
        fn next(&self, p: Option<Presence>) -> (Option<QueueItem>, QueueView) {
            self.h.next(&self.id, Some(&self.u), p.as_ref()).expect("next: no session")
        }
    }

    fn pres(visible: bool, audio: &str, idle_ms: i64) -> Option<Presence> {
        Some(Presence { visible, focused: visible, audio: audio.into(), idle_ms })
    }
    fn listening() -> Option<Presence> {
        pres(true, "running", 1000)
    }

    #[test]
    fn rfc3339_formats() {
        assert_eq!(rfc3339(t0()), "2026-11-20T12:00:00Z");
        assert_eq!(rfc3339_nano(t0() + 120_000_000), "2026-11-20T12:00:00.12Z");
        assert_eq!(rfc3339_nano(t0()), "2026-11-20T12:00:00Z");
    }

    #[test]
    fn queue_add_list_remove_clear_and_caps() {
        let e = qenv(QueueConfig::new(0, 0, 5, 3));
        let v = e.add(3);
        assert!(v.up_next.len() == 3 && v.up_next[0].id == "q1" && v.up_next[2].id == "q3" && v.max_items == 5);
        let err = e.h.queue(Some(&e.u), &e.id, QueueOp { action: "add".into(), items: vec![QueueItem::default(); 4], ..Default::default() }).unwrap_err();
        assert!(err.message.contains("at most 3"), "{}", err.message);
        e.add(2);
        let err = e.h.queue(Some(&e.u), &e.id, QueueOp { action: "add".into(), items: vec![QueueItem::default(); 1], ..Default::default() }).unwrap_err();
        assert!(err.message.contains("at most 5"));
        let (v, _) = e.h.queue(Some(&e.u), &e.id, QueueOp { action: "remove".into(), remove: "q2".into(), ..Default::default() }).unwrap();
        assert!(v.up_next.len() == 4 && v.up_next[1].id == "q3");
        assert!(e.h.queue(Some(&e.u), &e.id, QueueOp { action: "remove".into(), remove: "q99".into(), ..Default::default() }).is_err());
        let cid = v.up_next[0].cid.clone();
        let (v, _) = e.h.queue(Some(&e.u), &e.id, QueueOp { action: "remove".into(), remove: cid, ..Default::default() }).unwrap();
        assert_eq!(v.up_next.len(), 3);
        assert_eq!(e.op("clear").unwrap().0.up_next.len(), 0);
        assert!(e.op("bogus").is_err());
    }

    #[test]
    fn queue_auth() {
        let e = qenv(QueueConfig::default());
        assert!(e.h.queue(None, &e.id, QueueOp { action: "list".into(), ..Default::default() }).is_err());
        let m = User::of("mallory", "");
        assert!(e.h.queue(Some(&m), &e.id, QueueOp { action: "list".into(), ..Default::default() }).is_err());
        assert!(e.h.queue(Some(&m), "", QueueOp { action: "list".into(), ..Default::default() }).is_err());
        assert!(e.h.next(&e.id, Some(&m), listening().as_ref()).is_none());
    }

    fn ids(v: &QueueView) -> Vec<String> {
        v.up_next.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn queue_move_reorders() {
        let e = qenv(QueueConfig::default());
        e.add(4);
        let mv = |key: &str, to: usize| e.h.queue(Some(&e.u), &e.id, QueueOp { action: "move".into(), remove: key.into(), to, ..Default::default() });
        assert_eq!(ids(&mv("q4", 0).unwrap().0), ["q4", "q1", "q2", "q3"]);
        // past the end is last; by CID too
        assert_eq!(ids(&mv("q4", 99).unwrap().0), ["q1", "q2", "q3", "q4"]);
        let cid = e.op("list").unwrap().0.up_next[2].cid.clone();
        assert_eq!(ids(&mv(&cid, 1).unwrap().0), ["q1", "q3", "q2", "q4"]);
        assert_eq!(mv("q99", 0).unwrap_err().code, "not_found");
        // a move keeps the bytes the queue counts
        assert_eq!(e.op("list").unwrap().0.up_next.len(), 4);
    }

    #[test]
    fn queue_next_pick_plays_the_row_chosen() {
        let e = qenv(QueueConfig::default());
        e.add(3);
        let (it, v) = e.h.next_pick(&e.id, Some(&e.u), listening().as_ref(), Some("q2")).unwrap();
        assert_eq!(it.unwrap().id, "q2");
        assert_eq!(ids(&v), ["q1", "q3"]);
        assert_eq!(v.now_playing.unwrap().id, "q2");
        // a row that is gone answers no item and leaves the queue as it was
        let (it, v) = e.h.next_pick(&e.id, Some(&e.u), listening().as_ref(), Some("q2")).unwrap();
        assert!(it.is_none());
        assert_eq!(ids(&v), ["q1", "q3"]);
        // the presence rules decide a pick as they decide every advance
        let (it, v) = e.h.next_pick(&e.id, Some(&e.u), pres(true, "suspended", 0).as_ref(), Some("q3")).unwrap();
        assert!(it.is_none());
        assert_eq!(v.paused_reason, PAUSE_AUDIO_SUSPENDED);
    }

    #[test]
    fn queue_skip_to_an_item_tells_the_page_which() {
        let e = qenv(QueueConfig::default());
        let mut sub = e.h.subscribe(&e.id, Some(&e.u)).unwrap();
        e.add(3);
        e.next(listening()); // a listener reported: q1 plays
        while sub.rx.try_recv().is_ok() {}
        let skip = |key: &str| e.h.queue(Some(&e.u), &e.id, QueueOp { action: "skip".into(), remove: key.into(), ..Default::default() });
        let (_, sent) = skip("q3").unwrap();
        assert!(sent);
        let ev = sub.rx.try_recv().unwrap();
        let extra = ev.extra.unwrap();
        assert!(extra.starts_with("{\"action\":\"skip\",\"item\":\"q3\",\"queue\":"), "{extra}");
        // a plain skip names no item; an unknown item is refused
        let (_, sent) = skip("").unwrap();
        assert!(sent);
        assert!(!sub.rx.try_recv().unwrap().extra.unwrap().contains("\"item\""));
        assert_eq!(skip("q9").unwrap_err().code, "not_found");
    }

    #[test]
    fn page_queue_writes() {
        let e = qenv(QueueConfig::new(0, 0, 0, 2));
        // the page of an anonymous tab writes its own queue by the id alone
        let anon = e.h.create("").unwrap();
        let item = |c: char| QueueItem { cid: format!("zCID{}{c}", "y".repeat(20)), source: "store".into(), ..Default::default() };
        let w = |id: &str, u: Option<&User>, op: QueueOp| e.h.page_queue_write(id, u, op);
        let v = w(&anon, None, QueueOp { action: "add".into(), items: vec![item('a'), item('b')], ..Default::default() }).unwrap().unwrap();
        assert_eq!(ids(&v), ["q1", "q2"]);
        let v = w(&anon, None, QueueOp { action: "move".into(), remove: "q2".into(), to: 0, ..Default::default() }).unwrap().unwrap();
        assert_eq!(ids(&v), ["q2", "q1"]);
        let v = w(&anon, None, QueueOp { action: "remove".into(), remove: "q1".into(), ..Default::default() }).unwrap().unwrap();
        assert_eq!(ids(&v), ["q2"]);
        // the per-add cap holds for the page as for MCP
        assert_eq!(w(&anon, None, QueueOp { action: "add".into(), items: vec![item('c'); 3], ..Default::default() }).unwrap().unwrap_err().status, 413);
        // the page has no list or skip here (GET …/queue and …/queue/next are those)
        for a in ["list", "skip", "bogus"] {
            assert_eq!(w(&anon, None, QueueOp { action: a.into(), ..Default::default() }).unwrap().unwrap_err().status, 400);
        }
        let v = w(&anon, None, QueueOp { action: "clear".into(), ..Default::default() }).unwrap().unwrap();
        assert!(v.up_next.is_empty());
        // alice's tab: alice's page may write it, mallory's may not; an unknown id is no session
        let m = User::of("mallory", "");
        assert!(w(&e.id, Some(&e.u), QueueOp { action: "clear".into(), ..Default::default() }).is_some());
        assert!(w(&e.id, Some(&m), QueueOp { action: "clear".into(), ..Default::default() }).is_none());
        assert!(w("nope-nope-nope", None, QueueOp { action: "clear".into(), ..Default::default() }).is_none());
        // a signed-in page claims an anonymous tab by writing to it, as by reporting
        w(&anon, Some(&e.u), QueueOp { action: "clear".into(), ..Default::default() }).unwrap().unwrap();
        assert!(w(&anon, Some(&m), QueueOp { action: "clear".into(), ..Default::default() }).is_none());
    }

    #[test]
    fn queue_advances_while_listening() {
        let e = qenv(QueueConfig::default());
        e.add(2);
        let (it, v) = e.next(listening());
        assert!(it.unwrap().id == "q1" && v.position == 1 && v.now_playing.as_ref().unwrap().id == "q1" && v.up_next.len() == 1);
        let (it, v) = e.next(listening());
        assert!(it.unwrap().id == "q2" && v.position == 2);
        let (it, v) = e.next(listening());
        assert!(it.is_none() && v.paused_reason.is_empty());
    }

    #[test]
    fn queue_pauses_without_a_listener() {
        for (p, want) in [
            (None, PAUSE_NO_REPORT),
            (pres(true, "suspended", 0), PAUSE_AUDIO_SUSPENDED),
            (pres(true, "none", 0), PAUSE_AUDIO_SUSPENDED),
            (pres(false, "running", 30 * 60 * 1000), PAUSE_IDLE_HIDDEN),
        ] {
            let e = qenv(QueueConfig::default());
            e.add(2);
            let (it, v) = e.next(p);
            assert!(it.is_none() && v.paused_reason == want, "{want}: {:?}", v.paused_reason);
            let lv = e.op("list").unwrap().0;
            assert!(lv.paused_reason == want && lv.up_next.len() == 2);
        }
        let e = qenv(QueueConfig::default());
        e.add(2);
        assert!(e.next(pres(false, "running", 29 * 60 * 1000)).0.is_some(), "hidden 29 minutes is still present");
        assert!(e.next(pres(true, "running", 5 * 3600 * 1000)).0.is_some(), "visible is present");
    }

    #[test]
    fn queue_idle_minutes_is_configurable() {
        let e = qenv(QueueConfig::new(5, 0, 0, 0));
        e.add(1);
        let (it, v) = e.next(pres(false, "running", 5 * 60 * 1000));
        assert!(it.is_none() && v.paused_reason == PAUSE_IDLE_HIDDEN && v.idle_minutes == 5);
    }

    #[test]
    fn queue_max_auto_advance_and_resume() {
        let e = qenv(QueueConfig::new(0, 3, 0, 0));
        e.add(6);
        let mut touched = e.now();
        let idle = |touched: i64| pres(true, "running", (e.now() - touched) / MS);
        for i in 1..=3 {
            e.adv(3 * 60 * NS);
            let (it, v) = e.next(idle(touched));
            assert!(it.is_some() && v.auto_advanced == i, "advance {i}");
        }
        e.adv(3 * 60 * NS);
        let (it, v) = e.next(idle(touched));
        assert!(it.is_none() && v.paused_reason == PAUSE_MAX_AUTO);
        let (sv, sent) = e.op("skip").unwrap();
        assert!(!sent && sv.paused_reason == PAUSE_MAX_AUTO);
        e.adv(60 * NS);
        let (it, v) = e.next(idle(touched));
        assert!(it.is_none() && v.paused_reason == PAUSE_MAX_AUTO, "no fresh interaction");
        touched = e.now();
        let (it, v) = e.next(idle(touched));
        assert!(it.is_some() && v.paused_reason.is_empty() && v.auto_advanced == 1, "resume");
    }

    #[test]
    fn queue_interaction_resets_the_count() {
        let e = qenv(QueueConfig::new(0, 2, 0, 0));
        e.add(5);
        for _ in 0..4 {
            e.adv(60 * NS);
            let (it, v) = e.next(listening());
            assert!(it.is_some() && v.auto_advanced == 1);
        }
    }

    #[test]
    fn queue_presence_from_reports() {
        let e = qenv(QueueConfig::default());
        e.add(1);
        assert_eq!(e.op("list").unwrap().0.listener, PAUSE_NO_REPORT);
        let st = SessionState { playing: true, presence: pres(true, "running", 0), ..Default::default() };
        assert!(e.h.report(&e.id, Some(&e.u), st).0);
        assert_eq!(e.op("list").unwrap().0.listener, "present");
        e.adv(3 * 60 * NS);
        assert_eq!(e.op("list").unwrap().0.listener, PAUSE_NO_REPORT);
        assert!(!e.op("skip").unwrap().1);
    }

    #[test]
    fn queue_goes_with_the_session_and_counts_bytes() {
        let e = qenv(QueueConfig::default());
        let before = e.h.inner.lock().unwrap().bytes;
        let link = |cid: &str, t: String| QueueItem { cid: cid.into(), source: "link".into(), text: t, ..Default::default() };
        let (v, _) = e.h.queue(Some(&e.u), &e.id, QueueOp { action: "add".into(), items: vec![link("zLink", "a".repeat(1000))], ..Default::default() }).unwrap();
        assert_eq!(v.up_next.len(), 1);
        assert!(e.h.inner.lock().unwrap().bytes > before + 1000);
        assert!(e.h.queue(Some(&e.u), &e.id, QueueOp { action: "add".into(), items: vec![link("zBig", "b".repeat(MAX_QUEUE_TEXT_BYTES))], ..Default::default() }).is_err());
        let (it, _) = e.next(listening());
        assert_eq!(it.unwrap().text.len(), 1000);
        e.h.queue(Some(&e.u), &e.id, QueueOp { action: "add".into(), items: vec![link("zLink", "x".into())], ..Default::default() }).unwrap();
        e.h.inner.lock().unwrap().drop_session(&e.id);
        assert_eq!(e.h.inner.lock().unwrap().bytes, before);
        assert!(e.op("list").is_err());
    }

    #[test]
    fn session_ttl_and_per_user_cap() {
        let e = qenv(QueueConfig::default());
        for _ in 0..5 {
            e.h.create("").unwrap();
        }
        e.adv(11 * 60 * NS);
        e.h.sweep();
        assert_eq!(e.h.count(), 0, "untouched sessions expire");
        let s = e.h.create("").unwrap();
        let _sub = e.h.subscribe(&s, None).unwrap();
        e.adv(30 * 60 * NS);
        e.h.sweep();
        assert_eq!(e.h.count(), 1, "a session with a stream does not expire");
        let u = User::of("bob", "");
        let first = e.h.create("bob").unwrap();
        for _ in 0..MAX_SESSIONS_PER_USER + 2 {
            e.adv(NS);
            e.h.create("bob").unwrap();
        }
        assert_eq!(e.h.sessions(Some(&u)).len(), MAX_SESSIONS_PER_USER);
        assert!(!e.h.inner.lock().unwrap().sessions.contains_key(&first));
    }

    #[test]
    fn slow_subscriber_is_cut_off() {
        let e = qenv(QueueConfig::default());
        let mut sub = e.h.subscribe(&e.id, Some(&e.u)).unwrap();
        for _ in 0..40 {
            let _ = e.h.push(Some(&e.u), &e.id, &SessionEvent { typ: "stop".into(), ..Default::default() });
        }
        let mut n = 0;
        while let Ok(_) = sub.rx.try_recv() {
            n += 1;
        }
        assert_eq!(n, 16);
        assert!(matches!(sub.rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)), "closed");
    }

    #[test]
    fn streams_per_session_are_bounded() {
        let e = qenv(QueueConfig::default());
        let s = e.h.create("").unwrap();
        let mut subs: Vec<Subscription> = (0..MAX_STREAMS_PER_SESSION + 2).map(|_| e.h.subscribe(&s, None).unwrap()).collect();
        for sub in subs.iter_mut().take(2) {
            assert!(matches!(sub.rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)));
        }
        assert_eq!(e.h.push(Some(&User::of("x", "")), &s, &SessionEvent { typ: "stop".into(), ..Default::default() }).unwrap(), 1);
        assert_eq!(subs.last_mut().unwrap().rx.try_recv().unwrap().typ, "stop");
    }

    #[test]
    fn anonymous_session_is_claimed_by_sign_in_report() {
        let e = qenv(QueueConfig::default());
        let s = e.h.create("").unwrap();
        let (alice, bob) = (User::of("alice2", ""), User::of("bob2", ""));
        let mut sub = e.h.subscribe(&s, None).unwrap();
        assert!(e.h.sessions(Some(&alice)).is_empty());
        assert_eq!(e.h.push(Some(&alice), "", &SessionEvent { typ: "stop".into(), ..Default::default() }).unwrap(), 0);
        e.h.report(&s, None, SessionState::default());
        assert!(e.h.sessions(Some(&alice)).is_empty());
        assert_eq!(e.h.report(&s, Some(&alice), SessionState::default()), (true, false));
        assert_eq!(e.h.sessions(Some(&alice))[0].id, s);
        assert_eq!(e.h.push(Some(&alice), "", &SessionEvent { typ: "eval".into(), text: "t".into(), ..Default::default() }).unwrap(), 1);
        assert_eq!(sub.rx.try_recv().unwrap().typ, "eval");
        assert!(e.h.push(Some(&bob), &s, &SessionEvent { typ: "stop".into(), ..Default::default() }).is_err());
        assert_eq!(e.h.report(&s, Some(&bob), SessionState::default()), (false, false));
    }

    #[test]
    fn report_bucket_limits() {
        let e = qenv(QueueConfig::default());
        let limited = (0..60).filter(|_| e.h.report(&e.id, Some(&e.u), SessionState::default()).1).count();
        assert!(limited > 0);
    }

    #[tokio::test]
    async fn state_round_trip() {
        let e = Arc::new(qenv(QueueConfig::default()));
        assert_eq!(e.h.state(Some(&e.u), "").await.unwrap_err().code, "no_session");
        let mut sub = e.h.subscribe(&e.id, Some(&e.u)).unwrap();
        let e2 = e.clone();
        let t = tokio::spawn(async move { e2.h.state(Some(&e2.u), "").await });
        let ev = sub.rx.recv().await.unwrap();
        assert_eq!(ev.typ, "get");
        assert!(e.h.reply(&e.id, Some(&e.u), &ev.request_id, SessionState { cid: "zX".into(), ..Default::default() }));
        let st = t.await.unwrap().unwrap();
        assert_eq!(st.cid, "zX");
        assert_eq!(st.id, e.id);
        // fresh: answered at once
        assert_eq!(e.h.state(Some(&e.u), &e.id).await.unwrap().cid, "zX");
    }
}
