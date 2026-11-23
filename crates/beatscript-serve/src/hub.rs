//! P6: the hub (Go `internal/btwsite/server` pair.go, sessions.go, queue.go, device_compose.go, limiter.go).
//!
//! How MCP drives a browser or a paired phone. A page registers a session (`POST /api/session`), listens on its SSE
//! stream and reports its state; a phone pairs by scanning a QR code (`POST /api/pair` → `POST /api/pair/claim`)
//! and from then on signs every request (`Authorization: BtwDevice …`); MCP's write tools push events to the
//! sessions their caller may drive. The up-next queue lives on a session. Everything is in memory except the paired
//! devices (`<data>/devices.json`, written as Go writes it, so either server reads the other's).
//!
//! Routes (Go's patterns; a wrong method falls through to the static site's answer, as Go's catch-all gives):
//!   GET /pair · POST /api/pair · GET /api/pair/{id} · POST /api/pair/claim
//!   GET /api/devices · DELETE /api/devices/{id} · POST /api/devices/{id}/push
//!   GET|DELETE /api/device · GET /api/device/events (SSE) · POST /api/device/report|reply|compose
//!   POST /api/session · GET /api/session/{id}/events (SSE) · POST /api/session/{id}/report|reply
//!   GET /api/session/{id}/queue · POST /api/session/{id}/queue (the page's add, remove, move, clear) · POST /api/session/{id}/queue/next
//!
//! Flags: -queue-idle-minutes 30, -queue-max-auto 8, -queue-max-items 50, -queue-max-per-add 20, -moved-to,
//! -site-id, -max-prompt-chars 800 (device compose's body cap), and beatscript-serve's own -sse-ping-ms 20000.
//!
//! The MCP side (P7) uses `Hub`'s methods: `push`, `state`, `sessions`, `queue`, `devices`, `device_session`
//! (Go's `api.Hub`, `api.Queuer`, `api.Devices`), with the types in `hub::types`. Compose from a phone goes through
//! P7's compose service (`compose::get()`, resolved per request); without one it answers as Go's unwired compose.

#![allow(dead_code)] // the MCP-facing API (P7) is not called yet

pub mod gojson;
pub mod mcp_api;
pub mod pair;
pub mod private;
pub mod qr;
pub mod state;
pub mod types;

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, Method, Response};
use axum::routing::any;
use axum::Router;
use base64::Engine;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use crate::{go_body, ApiError, AppState};
use gojson::{jstr, q, Bind, Kind, Node};
use pair::*;
use state::{err_no_session, rfc3339, Clock, Core, QueueConfig, Subscription, MAX_REPORT_BYTES, MAX_STREAMS_PER_IP, NS};
pub use types::*;

/// `ipLimiter`: at most n events per key in any window (sliding), on the hub's clock.
pub struct IpLimiter {
    n: usize,
    window: i64,
    hits: Mutex<HashMap<String, VecDeque<i64>>>,
}

impl IpLimiter {
    pub fn new(n: usize, window: Duration) -> IpLimiter {
        IpLimiter { n, window: window.as_nanos() as i64, hits: Mutex::new(HashMap::new()) }
    }
    /// (ok, retry-after seconds)
    pub fn allow(&self, key: &str, now: i64) -> (bool, u64) {
        if self.n == 0 {
            return (true, 0);
        }
        let mut m = self.hits.lock().unwrap();
        if m.len() > 4096 {
            let cut = now - self.window;
            m.retain(|_, q| q.back().is_some_and(|t| *t > cut));
        }
        let q = m.entry(key.to_string()).or_default();
        let cut = now - self.window;
        while q.front().is_some_and(|t| *t <= cut) {
            q.pop_front();
        }
        if q.len() >= self.n {
            let wait = (q[0] + self.window - now) / NS + 1;
            return (false, wait.max(1) as u64);
        }
        q.push_back(now);
        (true, 0)
    }
}

/// The hub: sessions, paired devices, the queue, and the limits of the routes that make them.
pub struct Hub {
    pub core: Core,
    pair: Mutex<Pairing>,
    pub site_id: String,
    /// Go's `acceptedHosts()`: the public host first
    pub hosts: Vec<String>,
    pub public_url: String,
    pub moved_to: String,
    max_program_bytes: usize,
    max_prompt_chars: usize,
    sse_ping: Duration,
    session_ip: IpLimiter,
    pair_ip: IpLimiter,
    claim_ip: IpLimiter,
    /// the page's own queue writes (add, remove, move, clear) per address
    queue_ip: IpLimiter,
}

/// `errorf` for `limited`: 429 for a refused limiter.
fn limited(retry: u64, what: &str) -> ApiError {
    ApiError::new(429, "rate_limited", format!("too many {what} requests from your address; try again in {retry}s")).retry(retry)
}

impl Hub {
    pub fn new(clock: Clock, queue_cfg: QueueConfig, pairing: Pairing, site_id: String, hosts: Vec<String>, public_url: &str, moved_to: &str, max_program_bytes: usize, max_prompt_chars: usize) -> Hub {
        Hub {
            core: Core::new(clock, queue_cfg),
            pair: Mutex::new(pairing),
            site_id,
            hosts,
            public_url: public_url.trim_end_matches('/').to_string(),
            moved_to: moved_to.trim_end_matches('/').to_string(),
            max_program_bytes,
            max_prompt_chars,
            sse_ping: Duration::from_secs(20),
            session_ip: IpLimiter::new(120, Duration::from_secs(3600)),
            pair_ip: IpLimiter::new(20, Duration::from_secs(60)),
            claim_ip: IpLimiter::new(20, Duration::from_secs(60)),
            queue_ip: IpLimiter::new(60, Duration::from_secs(60)),
        }
    }

    pub fn queue_config(&self) -> QueueConfig {
        self.core.queue_cfg
    }

    fn now(&self) -> i64 {
        self.core.now()
    }

    // ---------- api.Hub / api.Queuer / api.Devices: what MCP calls ----------

    /// `Push`: deliver ev to the sessions u may drive (the named one, else all of u's); how many had a live page.
    pub fn push(&self, u: Option<&User>, session: &str, ev: &SessionEvent) -> Result<usize, ApiError> {
        self.core.push(u, session, ev)
    }

    /// `State`: the latest report of one session (asking the page with a "get" when stale). MCP's `get` bounds
    /// the whole call itself (Go: 8 s context); the hub waits at most 3 s for the page.
    pub async fn state(&self, u: Option<&User>, session: &str) -> Result<SessionState, ApiError> {
        self.core.state(u, session).await
    }

    /// `Sessions`: u's sessions, most recently seen first.
    pub fn sessions(&self, u: Option<&User>) -> Vec<SessionInfo> {
        self.core.sessions(u)
    }

    /// `Queue`: (view, sent). sent is true only for a skip that was pushed to the page.
    pub fn queue(&self, u: Option<&User>, session: &str, op: QueueOp) -> Result<(QueueView, bool), ApiError> {
        self.core.queue(u, session, op)
    }

    /// `Devices`: u's paired phones, most recently paired first (empty for no caller).
    pub fn devices(&self, u: Option<&User>) -> Vec<DeviceInfo> {
        let Some(u) = u else { return Vec::new() };
        let list = self.pair.lock().unwrap().owned(&u.id);
        list.into_iter()
            .map(|d| {
                let (connected, last, st) = self.core.device_status(&d.id);
                DeviceInfo {
                    fingerprint: key_fingerprint(&d.key),
                    connected,
                    last_seen: last.map(rfc3339).unwrap_or_default(),
                    session: if connected { self.core.device_session_id(&d.owner, &d.id) } else { String::new() },
                    state: st,
                    id: d.id,
                    name: d.name,
                    paired: d.paired,
                }
            })
            .collect()
    }

    /// `DeviceSession`: the hub session of u's device.
    pub fn device_session(&self, u: Option<&User>, device: &str) -> Result<String, ApiError> {
        let Some(u) = u else { return Err(ApiError::new(401, "auth_required", "driving a paired phone needs a signed-in caller")) };
        let d = self.pair.lock().unwrap().devices.get(device).map(|d| (d.name.clone(), d.owner.clone()));
        let Some((name, owner)) = d.filter(|(_, o)| *o == u.id) else {
            return Err(ApiError::new(404, "not_found", format!("no paired phone has id {} on this account; call devices", gojson::go_quote(device))));
        };
        let busy = || ApiError::new(409, "no_session", format!("{name} is paired but not connected: open djrust on the phone"));
        if !self.core.device_status(device).0 {
            return Err(busy());
        }
        let sid = self.core.device_session_id(&owner, device);
        if sid.is_empty() {
            return Err(busy());
        }
        Ok(sid)
    }

    // ---------- pairing helpers ----------

    fn public_host(&self) -> &str {
        self.hosts.first().map_or("localhost", |h| h.as_str())
    }

    /// `deviceView`: what the page sees of a device (a map: keys sorted).
    fn device_view(&self, d: &Device) -> String {
        let (connected, last, st) = self.core.device_status(&d.id);
        let mut o = format!("{{\"connected\":{connected},\"fingerprint\":{},\"id\":{}", q(&key_fingerprint(&d.key)), q(&d.id));
        if let Some(l) = last {
            o.push_str(&format!(",\"last_seen\":{}", q(&rfc3339(l))));
        }
        o.push_str(&format!(",\"name\":{},\"paired\":{}", q(&d.name), q(&d.paired)));
        if let Some(st) = st {
            o.push_str(&format!(",\"state\":{{\"bar\":{},\"cid\":{},\"playing\":{},\"section\":{},\"ts\":{}}}", st.bar, q(&st.cid), st.playing, q(&st.section), q(&st.ts)));
        }
        o.push('}');
        o
    }

    /// `deviceHello`: what the phone learns of its pairing (a map: keys sorted).
    fn device_hello(&self, d: &Device) -> String {
        let addr = d.owner.split_once(':').map_or(d.owner.as_str(), |(_, a)| a);
        // the account's capabilities and their signed grant (caps.rs), for the phone to decide what to show
        let g = crate::caps::grant_json(&d.owner);
        let caps = g.as_ref().map(|g| format!("\"caps\":{},", g["caps"])).unwrap_or_default();
        let grant = g.as_ref().map(|g| format!(",\"grant\":{}", g["grant"])).unwrap_or_default();
        let mut o = format!("{{\"address\":{},{caps}\"device_id\":{},\"events\":\"/api/device/events\",\"fingerprint\":{}{grant}", q(addr), q(&d.id), q(&key_fingerprint(&d.key)));
        if !self.moved_to.is_empty() {
            o.push_str(&format!(",\"moved_to\":{}", q(&self.moved_to)));
        }
        o.push_str(&format!(
            ",\"name\":{},\"origin\":{},\"proto\":\"{PAIR_PROTO2}\",\"protos\":[\"{PAIR_PROTO2}\",\"{PAIR_PROTO}\"],\"server\":{},\"site_id\":{},\"user\":{}}}",
            q(&d.name),
            q(&self.public_url),
            q(&self.public_url),
            q(&self.site_id),
            q(&d.owner)
        ));
        o
    }

    fn moved_event(&self) -> Option<SessionEvent> {
        if self.moved_to.is_empty() {
            return None;
        }
        let extra = format!("{{\"from\":{},\"moved_to\":{},\"site_id\":{}}}", q(&self.public_url), q(&self.moved_to), q(&self.site_id));
        Some(SessionEvent { typ: "moved".into(), extra: Some(extra), ..Default::default() })
    }

    /// `unpair`: forget a device (owner "" = any) and tell its stream.
    fn unpair(&self, id: &str, owner: &str, why: &str) -> bool {
        {
            let mut p = self.pair.lock().unwrap();
            match p.devices.get(id) {
                Some(d) if owner.is_empty() || d.owner == owner => {}
                _ => return false,
            }
            p.devices.remove(id);
            if let Err(e) = p.save() {
                eprintln!("beatscript-serve: pair: saving devices: {e}");
            }
        }
        self.core.drop_device(id, why);
        true
    }

    fn verify_claim(&self, key: &[u8; 32], c: &Claim, sig: &[u8]) -> bool {
        if c.proto == PAIR_PROTO2 {
            return verify(key, &claim_message2(&self.site_id, &c.pair_id, &c.token, &c.key, &c.name, c.ts), sig);
        }
        self.hosts.iter().any(|h| verify(key, &claim_message(h, &c.pair_id, &c.token, &c.key, &c.name, c.ts), sig))
    }

    fn verify_request(&self, key: &[u8; 32], dev: &str, method: &str, path: &str, ts: i64, body: &[u8], sig: &[u8]) -> bool {
        verify(key, &request_message2(&self.site_id, dev, method, path, ts, body), sig) || self.hosts.iter().any(|h| verify(key, &request_message(h, dev, method, path, ts, body), sig))
    }
}

fn skewed(ts_ms: i64, now: i64) -> bool {
    let d = ts_ms as i128 * 1_000_000 - now as i128;
    d > DEVICE_SKEW as i128 || d < -(DEVICE_SKEW as i128)
}

#[derive(Default)]
struct Claim {
    proto: String,
    pair_id: String,
    token: String,
    key: String,
    name: String,
    ts: i64,
    sig: String,
}

// ---------- HTTP ----------

#[derive(Clone)]
struct HS {
    app: AppState,
    hub: Arc<Hub>,
}

fn json_answer(status: u16, body: String) -> Response<Body> {
    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json; charset=utf-8").body(go_body(body + "\n")).unwrap()
}

fn no_content() -> Response<Body> {
    Response::builder().status(204).body(Body::empty()).unwrap()
}

fn err(e: ApiError) -> Response<Body> {
    e.response()
}

fn peer_ip(req: &Request) -> String {
    let peer: SocketAddr = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

/// The body, at most `max + 1` bytes of it (enough to tell "over the cap").
async fn read_body(body: Body, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut s = body.into_data_stream();
    while let Some(Ok(chunk)) = s.next().await {
        out.extend_from_slice(&chunk);
        if out.len() > max {
            out.truncate(max + 1);
            break;
        }
    }
    out
}

/// `decodeBody`: a JSON body of at most max bytes, the error answered as Go does.
fn decode_body(b: &[u8], max: usize) -> Result<Node, ApiError> {
    gojson::decode_capped(b, max).map_err(|e| match e {
        None => ApiError::new(413, "too_large", format!("request body is over {max} bytes")),
        Some(m) => ApiError::new(400, "bad_request", format!("the body must be JSON: {m}")),
    })
}

/// A binding's first type error as decodeBody answers it.
fn bind_err(b: Bind) -> Result<(), ApiError> {
    match b.err {
        Some(m) => Err(ApiError::new(400, "bad_request", format!("the body must be JSON: {m}"))),
        None => Ok(()),
    }
}

fn caller_user(c: beatscript_serve::playlists::auth::Caller) -> User {
    use beatscript_serve::playlists::auth::Via;
    let via = match c.via {
        Via::Cookie => "cookie",
        Via::Token => "token",
        Via::Device(_) => "device",
    };
    User::of(&c.uid, via)
}

impl HS {
    /// Go's `authn.UserFrom`: the cookie, then a bearer token. A `BtwDevice` header is not a user here (Go's
    /// authenticator does not read it), so it is taken off before the lookup.
    fn user_from(&self, h: &HeaderMap) -> Result<Option<User>, ApiError> {
        let Some(auth) = self.app.authsvc.get() else { return Ok(None) };
        let mut h2;
        let h = if h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("BtwDevice ")) {
            h2 = h.clone();
            h2.remove(header::AUTHORIZATION);
            &h2
        } else {
            h
        };
        auth.caller(h, "", "", b"").map(|c| c.map(caller_user)).map_err(|e| ApiError::new(e.status, e.code, e.message))
    }

    fn user_or_anon(&self, h: &HeaderMap) -> Option<User> {
        self.user_from(h).ok().flatten()
    }

    fn require_user(&self, h: &HeaderMap) -> Result<User, ApiError> {
        match self.user_from(h)? {
            Some(u) => Ok(u),
            None => Err(ApiError::new(401, "auth_required", format!("authentication required: {}", self.auth_help()))),
        }
    }

    fn auth_help(&self) -> String {
        format!("sign in at {} (wallet or browser key), press \"MCP token\", and send it as `Authorization: Bearer btw_…`", self.hub.public_url)
    }

    /// `refuseCrossSite`.
    fn cross_site(&self, h: &HeaderMap) -> Result<(), ApiError> {
        let g = |k: &str| h.get(k).and_then(|v| v.to_str().ok());
        if beatscript_serve::playlists::auth::cross_site(g("origin"), g("sec-fetch-site"), g("host"), &self.hub.hosts) {
            return Err(ApiError::new(403, "cross_site", "this request came from another site; pairing is only for this site's own page"));
        }
        Ok(())
    }

    /// `deviceFrom`: a phone's signed request. The body (when max > 0) is read, hashed and given back.
    async fn device_from(&self, req: Request, max: usize) -> Result<(Device, Vec<u8>, Request<()>), ApiError> {
        let fail = |m: &str| ApiError::new(401, "device_auth", m);
        let (parts, body) = req.into_parts();
        let h = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        let Some(cred) = h.strip_prefix("BtwDevice ") else { return Err(fail("this route is for a paired phone (Authorization: BtwDevice …)")) };
        let f: Vec<&str> = cred.trim().split('.').collect();
        if f.len() != 3 {
            return Err(fail("malformed BtwDevice credential"));
        }
        let Ok(ts) = go_parse_i64(f[1]) else { return Err(fail("malformed BtwDevice credential")) };
        let sig = match b64url_decode(f[2]) {
            Some(s) if s.len() == 64 => s,
            _ => return Err(fail("malformed BtwDevice credential")),
        };
        let mut b = Vec::new();
        if max > 0 {
            b = read_body(body, max).await;
            if b.len() > max {
                return Err(ApiError::new(413, "too_large", format!("request body is over {max} bytes")));
            }
        }
        let now = self.hub.now();
        if skewed(ts, now) {
            return Err(fail("the phone's clock is too far from the server's"));
        }
        let mut p = self.hub.pair.lock().unwrap();
        let Some(d) = p.devices.get(f[0]).cloned() else {
            return Err(ApiError::new(410, "unpaired", "this phone is not paired (any more): scan a new code"));
        };
        let key = decode_key(&d.key).unwrap_or([0; 32]);
        if !self.hub.verify_request(&key, &d.id, parts.method.as_str(), parts.uri.path(), ts, &b, &sig) {
            return Err(fail("the request's signature does not verify"));
        }
        p.sweep(now);
        if p.seen.contains_key(f[2]) {
            return Err(fail("this request was already made (replay)"));
        }
        p.seen.insert(f[2].to_string(), now);
        drop(p);
        Ok((d, b, Request::from_parts(parts, ())))
    }
}

/// `strconv.ParseInt(s, 10, 64)`: an optional sign and decimal digits only.
fn go_parse_i64(s: &str) -> Result<i64, ()> {
    let d = s.strip_prefix(['+', '-']).unwrap_or(s);
    if d.is_empty() || !d.bytes().all(|c| c.is_ascii_digit()) {
        return Err(());
    }
    s.parse().map_err(|_| ())
}

// ---------- SSE ----------

struct StreamGuard {
    hub: Arc<Hub>,
    session: String,
    sub: u64,
    ip: String,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.hub.core.unsubscribe(&self.session, self.sub);
        self.hub.core.release_stream(&self.ip);
    }
}

fn frame(ev: &SessionEvent) -> String {
    format!("event: {}\ndata: {}\n\n", ev.typ, ev.to_json())
}

/// `streamEvents`: `retry` and a comment, then hello and the after-events, then the subscriber's events and a ping
/// every `-sse-ping-ms`, until the subscriber is closed or the client goes.
fn stream_events(hub: Arc<Hub>, sub: Subscription, ip: String, first: Vec<SessionEvent>) -> Response<Body> {
    let mut pre = String::from("retry: 3000\n: connected\n\n");
    for ev in &first {
        pre.push_str(&frame(ev));
    }
    let ping = hub.sse_ping;
    let guard = StreamGuard { hub: hub.clone(), session: sub.session.clone(), sub: sub.sub, ip };
    struct St {
        rx: tokio::sync::mpsc::Receiver<SessionEvent>,
        tick: tokio::time::Interval,
        pre: Option<String>,
        _g: StreamGuard,
        hub: Arc<Hub>,
    }
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + ping, ping);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let st = St { rx: sub.rx, tick, pre: Some(pre), _g: guard, hub };
    let s = futures_util::stream::unfold(st, |mut st| async move {
        if let Some(p) = st.pre.take() {
            return Some((Ok::<_, std::io::Error>(bytes::Bytes::from(p)), st));
        }
        tokio::select! {
            ev = st.rx.recv() => match ev {
                Some(ev) => { let f = frame(&ev); Some((Ok(bytes::Bytes::from(f)), st)) }
                None => None,
            },
            _ = st.tick.tick() => {
                let f = format!("event: ping\ndata: {{\"type\":\"ping\",\"ts\":\"{}\"}}\n\n", rfc3339(st.hub.now()));
                Some((Ok(bytes::Bytes::from(f)), st))
            }
        }
    });
    Response::builder()
        .status(200)
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(s))
        .unwrap()
}

// ---------- the page: pairing ----------

pub(crate) const PAIR_PAGE: &str = "<!doctype html>
<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<meta name=\"robots\" content=\"noindex\"><title>Pair your phone with BeatScript</title><link rel=\"stylesheet\" href=\"/css/beatscript.css\"></head>
<body><main class=\"pair-page\">
<h1>This is a pairing code for the BeatScript app</h1>
<p>Open <b>BeatScript</b> on your Android phone, tap the menu, then <b>beatscript.app</b> and <b>Scan to connect</b>, and scan the code again from there.</p>
<p>It is not an invite code: scan it with <b>Scan to connect</b>, not from Identity. It works once, for two minutes; if it has run out, press <b>Show a pairing code</b> under <b>mobile</b> on the page again.</p>
<p><a href=\"/\">Back to the site</a></p>
</main>
<footer class=\"rights\"><a href=\"/authorship\">Authorship and rights</a> &middot; <a href=\"https://github.com/stackdump/beatscript\">source</a> &middot; Code MIT &middot; programs CC0</footer>
</body></html>
";

async fn pair_page() -> Response<Body> {
    Response::builder()
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(go_body(PAIR_PAGE))
        .unwrap()
}

async fn pair_new(State(s): State<HS>, req: Request) -> Response<Body> {
    let r = async move {
        s.cross_site(req.headers())?;
        let u = s.require_user(req.headers())?;
        let hub = &s.hub;
        let now = hub.now();
        let (ok, retry) = hub.pair_ip.allow(&peer_ip(&req), now);
        if !ok {
            return Err(limited(retry, "pair"));
        }
        let mut tb = [0u8; 16];
        getrandom::fill(&mut tb).expect("random");
        let token = b64url(&tb);
        let id = state::new_id();
        let expires = now + PAIR_TTL;
        {
            let mut p = hub.pair.lock().unwrap();
            p.sweep(now);
            let mut open: Vec<(i64, String)> = p.offers.values().filter(|o| o.owner == u.id && o.state == "waiting" && now < o.expires).map(|o| (o.created, o.id.clone())).collect();
            open.sort();
            while open.len() >= MAX_OFFERS_PER_USER {
                let (_, oid) = open.remove(0);
                p.offers.get_mut(&oid).unwrap().state = "used".into();
            }
            p.offers.insert(id.clone(), Offer { id: id.clone(), owner: u.id.clone(), token_hash: Sha256::digest(token.as_bytes()).into(), created: now, expires, state: "waiting".into(), device: String::new() });
        }
        let link = pair_link(&hub.public_url, &id, &token);
        let png = qr::encode_png(&link, qr::Level::Medium, 320).map_err(|e| ApiError::new(500, "internal", format!("could not draw the QR code: {e}")))?;
        let qr = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
        Ok(json_answer(
            200,
            format!(
                "{{\"expires\":{},\"expires_in\":{},\"origin\":{},\"pair_id\":{},\"protocol\":\"{PAIR_PROTO2}\",\"qr\":{},\"site_id\":{},\"url\":{}}}",
                q(&rfc3339(expires)),
                PAIR_TTL / NS,
                q(&hub.public_url),
                q(&id),
                q(&qr),
                q(&hub.site_id),
                q(&link)
            ),
        ))
    };
    r.await.unwrap_or_else(err)
}

async fn pair_state(State(s): State<HS>, Path(id): Path<String>, h: HeaderMap) -> Response<Body> {
    let u = match s.require_user(&h) {
        Ok(u) => u,
        Err(e) => return err(e),
    };
    let hub = &s.hub;
    let now = hub.now();
    let p = hub.pair.lock().unwrap();
    let Some(o) = p.offers.get(&id).filter(|o| o.owner == u.id) else {
        return err(ApiError::new(404, "not_found", "no such pairing code (or it is not yours)"));
    };
    let mut state = o.state.clone();
    if state == "waiting" && now >= o.expires {
        state = "expired".into();
    }
    let mut out = String::from("{");
    if state == "paired" {
        if let Some(d) = p.devices.get(&o.device) {
            out.push_str(&format!("\"device\":{},", hub.device_view(d)));
        }
    }
    out.push_str(&format!("\"expires\":{},\"pair_id\":{},\"state\":{}}}", q(&rfc3339(o.expires)), q(&o.id), q(&state)));
    drop(p);
    json_answer(200, out)
}

async fn devices_list(State(s): State<HS>, h: HeaderMap) -> Response<Body> {
    let u = match s.require_user(&h) {
        Ok(u) => u,
        Err(e) => return err(e),
    };
    let list = s.hub.pair.lock().unwrap().owned(&u.id);
    let views: Vec<String> = list.iter().map(|d| s.hub.device_view(d)).collect();
    json_answer(200, format!("{{\"devices\":[{}]}}", views.join(",")))
}

async fn device_unpair(State(s): State<HS>, Path(id): Path<String>, h: HeaderMap) -> Response<Body> {
    if let Err(e) = s.cross_site(&h) {
        return err(e);
    }
    let u = match s.require_user(&h) {
        Ok(u) => u,
        Err(e) => return err(e),
    };
    if !s.hub.unpair(&id, &u.id, &format!("unpaired on {}", s.hub.public_host())) {
        return err(ApiError::new(404, "not_found", "no such device (or it is not yours)"));
    }
    no_content()
}

async fn device_push(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let r = async {
        s.cross_site(req.headers())?;
        let _u = s.require_user(req.headers())?;
        let u = _u;
        let max = s.hub.max_program_bytes + 4096;
        let ip = peer_ip(&req);
        let b = read_body(req.into_body(), max).await;
        let n = decode_body(&b, max)?;
        let (mut typ, mut text, mut play) = (String::new(), String::new(), false);
        let mut bd = Bind::new(&b);
        bd.object(&n, "", "struct { Type string \"json:\\\"type\\\"\"; Text string \"json:\\\"text\\\"\"; Play bool \"json:\\\"play\\\"\" }", &["type", "text", "play"], |bd, i, v| match i {
            0 => bd.string(v, &mut typ),
            1 => bd.string(v, &mut text),
            _ => bd.boolean(v, &mut play),
        });
        bind_err(bd)?;
        let d = s.hub.pair.lock().unwrap().devices.get(&id).cloned();
        let Some(d) = d.filter(|d| d.owner == u.id) else { return Err(ApiError::new(404, "not_found", "no such device (or it is not yours)")) };
        let mut ev = SessionEvent { typ: typ.clone(), play, ..Default::default() };
        match typ.as_str() {
            "stop" => {}
            "eval" | "load" => {
                if text.trim().is_empty() {
                    return Err(ApiError::new(400, "bad_request", "text is required"));
                }
                let maxp = s.hub.max_program_bytes;
                if text.len() > maxp {
                    return Err(ApiError::new(413, "too_large", format!("the program is over {maxp} bytes")));
                }
                let rep = check(&s.app, &ip, text.clone()).await?;
                let flag = |k: &str| rep.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
                if !flag("ok") {
                    return Err(ApiError::new(422, "bad_request", "the program has errors; nothing was sent to the phone"));
                }
                if flag("restricted") || flag("uses_assets") {
                    return Err(ApiError::new(422, "restricted", "a private or asset-using program is not sent to a phone"));
                }
                ev.cid = rep.get("cid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                // (checked as it was sent; the phone gets it formatted, a load as an eval, with the header lines and comments)
                ev.text = beatscript_serve::autoformat::autoformat(&text, maxp);
            }
            _ => return Err(ApiError::new(400, "bad_request", "type must be eval, load or stop")),
        }
        if !s.hub.core.push_device(&d.id, &ev) {
            return Err(ApiError::new(409, "no_session", format!("{} is paired but not connected: open djrust on the phone", d.name)));
        }
        Ok(json_answer(200, format!("{{\"cid\":{},\"delivered\":1}}", q(&ev.cid))))
    };
    r.await.unwrap_or_else(err)
}

/// The checker: P3's (`public::Checker`, charged to the caller's address as Go's checkerOwner does), else beatscript-api
/// in-process on the blocking pool (at most `check_slots` at once). The CheckReport JSON.
async fn check(app: &AppState, owner: &str, text: String) -> Result<serde_json::Value, ApiError> {
    if let Some(c) = app.checker.get() {
        return c.check(owner, &text).await;
    }
    let _slot = app.check_slots.clone().acquire_owned().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let rep = tokio::task::spawn_blocking(move || crate::store::check(&text)).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if rep.is_null() {
        return Err(ApiError::new(502, "checker_error", "the checker failed; try again"));
    }
    Ok(rep)
}

// ---------- the phone ----------

async fn pair_claim(State(s): State<HS>, req: Request) -> Response<Body> {
    let r = async {
        let hub = &s.hub;
        let (ok, retry) = hub.claim_ip.allow(&peer_ip(&req), hub.now());
        if !ok {
            return Err(limited(retry, "pair claim"));
        }
        let b = read_body(req.into_body(), MAX_CLAIM_BYTES).await;
        let n = decode_body(&b, MAX_CLAIM_BYTES)?;
        let mut c = Claim::default();
        let mut bd = Bind::new(&b);
        bd.object(&n, "claimBody", "server.claimBody", &["proto", "pair_id", "token", "key", "name", "ts", "sig"], |bd, i, v| match i {
            0 => bd.string(v, &mut c.proto),
            1 => bd.string(v, &mut c.pair_id),
            2 => bd.string(v, &mut c.token),
            3 => bd.string(v, &mut c.key),
            4 => bd.string(v, &mut c.name),
            5 => bd.int(v, &mut c.ts, "int64"),
            _ => bd.string(v, &mut c.sig),
        });
        bind_err(bd)?;
        let bad = |st: u16, code: &str, m: &str| Err(ApiError::new(st, code, m));
        if c.proto != PAIR_PROTO && c.proto != PAIR_PROTO2 {
            return bad(400, "bad_request", &format!("proto must be {PAIR_PROTO2} (or {PAIR_PROTO})"));
        }
        let key = match decode_key(&c.key) {
            Ok(k) => k,
            Err(m) => return bad(400, "bad_request", m),
        };
        if !valid_device_name(&c.name) {
            return bad(400, "bad_request", &format!("name must be 1..{MAX_DEVICE_NAME} printable characters"));
        }
        let now = hub.now();
        if skewed(c.ts, now) {
            return bad(401, "challenge_stale", "the phone's clock is too far from the server's (set it automatically and scan again)");
        }
        match b64url_decode(&c.sig) {
            Some(sig) if hub.verify_claim(&key, &c, &sig) => {}
            _ => return bad(401, "bad_signature", "the claim's signature does not verify"),
        }
        let (d, replaced) = {
            let mut p = hub.pair.lock().unwrap();
            p.sweep(now);
            let Some(o) = p.offers.get_mut(&c.pair_id) else {
                return bad(404, "not_found", "this QR code is unknown or too old: show a new one and scan again");
            };
            let th: [u8; 32] = Sha256::digest(c.token.as_bytes()).into();
            if o.state != "waiting" {
                return bad(410, "pair_used", "this QR code was already used: show a new one and scan again");
            }
            if now >= o.expires {
                return bad(410, "pair_expired", "this QR code expired: show a new one and scan again");
            }
            if !bool::from(subtle::ConstantTimeEq::ct_eq(&th[..], &o.token_hash[..])) {
                o.state = "used".into();
                return bad(401, "bad_signature", "the QR code's token does not match: show a new one and scan again");
            }
            let owner = o.owner.clone();
            let mut replaced: Vec<String> = p.devices.values().filter(|d| d.key == c.key).map(|d| d.id.clone()).collect();
            for id in &replaced {
                p.devices.remove(id);
            }
            let mine: Vec<&Device> = p.devices.values().filter(|d| d.owner == owner).collect();
            if mine.len() >= MAX_DEVICES_PER_USER {
                let oldest = mine.iter().min_by(|a, b| a.paired.cmp(&b.paired)).unwrap().id.clone();
                p.devices.remove(&oldest);
                replaced.push(oldest);
            }
            if p.devices.len() >= MAX_DEVICES {
                return bad(503, "busy", "too many paired devices on this server");
            }
            let d = Device { id: state::new_id(), owner, name: c.name.clone(), key: c.key.clone(), paired: rfc3339(now) };
            p.devices.insert(d.id.clone(), d.clone());
            let o = p.offers.get_mut(&c.pair_id).unwrap();
            o.state = "paired".into();
            o.device = d.id.clone();
            if let Err(e) = p.save() {
                eprintln!("beatscript-serve: pair: saving devices: {e}");
            }
            (d, replaced)
        };
        for id in replaced {
            hub.core.drop_device(&id, "paired again");
        }
        Ok(json_answer(200, hub.device_hello(&d)))
    };
    r.await.unwrap_or_else(err)
}

async fn device_me(State(s): State<HS>, req: Request) -> Response<Body> {
    match s.device_from(req, 0).await {
        Ok((d, _, _)) => json_answer(200, s.hub.device_hello(&d)),
        Err(e) => err(e),
    }
}

async fn device_forget(State(s): State<HS>, req: Request) -> Response<Body> {
    match s.device_from(req, 0).await {
        Ok((d, _, _)) => {
            s.hub.unpair(&d.id, "", "unpaired on the phone");
            no_content()
        }
        Err(e) => err(e),
    }
}

async fn device_events(State(s): State<HS>, req: Request) -> Response<Body> {
    let ip = peer_ip(&req);
    let d = match s.device_from(req, 0).await {
        Ok((d, _, _)) => d,
        Err(e) => return err(e),
    };
    let hub = s.hub.clone();
    if !hub.core.acquire_stream(&ip) {
        return err(ApiError::new(429, "rate_limited", "too many open streams from your address").retry(30));
    }
    let sub = match hub.core.subscribe_device(&d.owner, &d.id) {
        Ok(x) => x,
        Err(e) => {
            hub.core.release_stream(&ip);
            return err(e);
        }
    };
    let mut first = vec![SessionEvent { typ: "hello".into(), extra: Some(hub.device_hello(&d)), ..Default::default() }];
    first.extend(hub.moved_event());
    stream_events(hub, sub, ip, first)
}

async fn device_report(State(s): State<HS>, req: Request) -> Response<Body> {
    let (d, b, _) = match s.device_from(req, MAX_REPORT_BYTES).await {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    let st = match gojson::unmarshal(&b) {
        Err(m) => Err(m),
        Ok(n) => {
            let mut st = SessionState::default();
            let mut bd = Bind::new(&b);
            SessionState::bind(&mut bd, &n, &mut st);
            match bd.err {
                Some(m) => Err(m),
                None => Ok(st),
            }
        }
    };
    let st = match st {
        Ok(st) => st,
        Err(m) => return err(ApiError::new(400, "bad_request", format!("the body must be JSON: {m}"))),
    };
    let mut st = st;
    if st.source == "private" {
        // a private program's report keeps no text, CID or marks here (its diagnostics say what is missing)
        st.text.clear();
        st.cid.clear();
        st.marks = None;
    }
    let sid = s.hub.core.device_session_id(&d.owner, &d.id);
    if sid.is_empty() {
        return err(ApiError::new(409, "no_session", "open the event stream first"));
    }
    report_answer(s.hub.core.report(&sid, Some(&User::of(&d.owner, "")), st))
}

fn report_answer(r: (bool, bool)) -> Response<Body> {
    match r {
        (false, _) => err(err_no_session()),
        (_, true) => err(ApiError::new(429, "rate_limited", "reporting too fast").retry(1)),
        _ => no_content(),
    }
}

const REPLY_TYPE: &str = "struct { RequestID string \"json:\\\"request_id\\\"\"; State api.SessionState \"json:\\\"state\\\"\" }";

fn bind_reply(b: &[u8], n: &Node) -> (Option<String>, String, SessionState) {
    let (mut rid, mut st) = (String::new(), SessionState::default());
    let mut bd = Bind::new(b);
    bd.object(n, "", REPLY_TYPE, &["request_id", "state"], |bd, i, v| match i {
        0 => bd.string(v, &mut rid),
        _ => SessionState::bind(bd, v, &mut st),
    });
    (bd.err, rid, st)
}

async fn device_reply(State(s): State<HS>, req: Request) -> Response<Body> {
    let (d, b, _) = match s.device_from(req, MAX_REPORT_BYTES).await {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    let parsed = gojson::unmarshal(&b).ok().map(|n| bind_reply(&b, &n));
    let Some((None, rid, st)) = parsed.filter(|(e, rid, _)| e.is_none() && !rid.is_empty()) else {
        return err(ApiError::new(400, "bad_request", "body must be {request_id, state}"));
    };
    let sid = s.hub.core.device_session_id(&d.owner, &d.id);
    if sid.is_empty() || !s.hub.core.reply(&sid, Some(&User::of(&d.owner, "")), &rid, st) {
        return err(err_no_session());
    }
    no_content()
}

/// `jsonWords`/`clipWords`.
fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    String::from_utf8_lossy(&s.as_bytes()[..n]).into_owned()
}

async fn device_compose(State(s): State<HS>, req: Request) -> Response<Body> {
    let hub = s.hub.clone();
    let prompt = if hub.max_prompt_chars == 0 { 800 } else { hub.max_prompt_chars };
    let program = if hub.max_program_bytes == 0 { 16384 } else { hub.max_program_bytes };
    let max = prompt * 6 + program * 6 + 4096;
    let ip = peer_ip(&req);
    let (d, b, _) = match s.device_from(req, max).await {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    let ask = "send a JSON body like {\"prompt\":\"dark techno 128\"}";
    let mut r = ComposeRequest::default();
    let mut length: Option<String> = None;
    let words = match gojson::decode_first(&b) {
        Err(_) => Some("the body is not valid JSON".to_string()),
        Ok(n) => {
            let mut bd = Bind::new(&b);
            bd.disallow_unknown = true;
            let mut field = String::new();
            bd.object(&n, "deviceComposeBody", "server.deviceComposeBody", &["prompt", "genre", "key", "length", "bpm", "base_cid", "current"], |bd, i, v| {
                let before = bd.err.is_some();
                match i {
                    0 => bd.string(v, &mut r.prompt),
                    1 => bd.string(v, &mut r.genre),
                    2 => bd.string(v, &mut r.key),
                    3 => bd.rawmsg(v, &mut length),
                    4 => bd.float(v, &mut r.bpm),
                    5 => bd.string(v, &mut r.base_cid),
                    _ => bd.string(v, &mut r.current),
                }
                if !before && bd.err.is_some() {
                    field = ["prompt", "genre", "key", "length", "bpm", "base_cid", "current"][i].to_string();
                }
            });
            match bd.err {
                None => None,
                Some(m) if m.starts_with("json: unknown field") => Some(clip(&m[m.find("unknown field").unwrap()..], 80)),
                Some(m) if m.starts_with("json: cannot unmarshal") => Some(format!("a field has the wrong type: {}", clip(&field, 40))),
                Some(_) => Some("the body is not valid JSON".to_string()),
            }
        }
    };
    if let Some(w) = words {
        return err(ApiError::new(400, "bad_request", format!("{ask}: {w}")));
    }
    if r.prompt.trim().is_empty() {
        return err(ApiError::new(400, "bad_request", ask));
    }
    // lengthArg: absent or null, a string, or a number (as written)
    r.length = match length.as_deref().map(|raw| gojson::unmarshal(raw.as_bytes()).map(|n| n.kind)) {
        None => String::new(),
        Some(Ok(Kind::Null)) => String::new(),
        Some(Ok(Kind::Str(s))) => s,
        Some(Ok(Kind::Num(n))) => n,
        _ => return err(ApiError::new(400, "bad_request", "length is short, medium, long or a number of bars")),
    };
    // compose (P7) runs as the pairing owner: the web route's limits, kill switch and spend cap, the same account
    let Some(c) = crate::compose::get() else {
        return err(ApiError::new(503, "not_configured", "compose is not configured on this server"));
    };
    let req = crate::compose::prompt::Request { prompt: r.prompt, genre: r.genre, bpm: r.bpm, key: r.key, length: r.length, base_cid: r.base_cid, current: r.current };
    match c.compose_async(d.owner.clone(), ip, req).await {
        Ok(res) => {
            // compose's own writer (encoding/json's floats and escaping); Go's writeJSON, no Cache-Control
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .body(go_body(crate::compose::gojson::line(&res.device_json())))
                .unwrap()
        }
        Err(f) => f.response(false),
    }
}

// ---------- the page: sessions and the queue ----------

async fn session_create(State(s): State<HS>, req: Request) -> Response<Body> {
    let hub = &s.hub;
    let (ok, retry) = hub.session_ip.allow(&peer_ip(&req), hub.now());
    if !ok {
        return err(limited(retry, "session"));
    }
    let u = s.user_or_anon(req.headers());
    match hub.core.create(u.as_ref().map_or("", |u| u.id.as_str())) {
        Ok(id) => json_answer(200, format!("{{\"id\":{},\"user\":{}}}", q(&id), u.map_or("null".into(), |u| u.to_json()))),
        Err(e) => err(e),
    }
}

async fn session_events(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let u = s.user_or_anon(req.headers());
    let ip = peer_ip(&req);
    let hub = s.hub.clone();
    if !hub.core.acquire_stream(&ip) {
        return err(ApiError::new(429, "rate_limited", format!("too many open streams from your address ({MAX_STREAMS_PER_IP}); close a tab or wait")).retry(30));
    }
    match hub.core.subscribe(&id, u.as_ref()) {
        Ok(sub) => stream_events(hub, sub, ip, Vec::new()),
        Err(e) => {
            hub.core.release_stream(&ip);
            err(e)
        }
    }
}

async fn session_report(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let b = read_body(body, MAX_REPORT_BYTES).await;
    let n = match decode_body(&b, MAX_REPORT_BYTES) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let mut st = SessionState::default();
    let mut bd = Bind::new(&b);
    SessionState::bind(&mut bd, &n, &mut st);
    if let Err(e) = bind_err(bd) {
        return err(e);
    }
    let u = s.user_or_anon(&parts.headers);
    report_answer(s.hub.core.report(&id, u.as_ref(), st))
}

async fn session_reply(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let b = read_body(body, MAX_REPORT_BYTES).await;
    let n = match decode_body(&b, MAX_REPORT_BYTES) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let (e, rid, st) = bind_reply(&b, &n);
    if let Some(m) = e {
        return err(ApiError::new(400, "bad_request", format!("the body must be JSON: {m}")));
    }
    if rid.is_empty() {
        return err(ApiError::new(400, "bad_request", "request_id is required"));
    }
    let u = s.user_or_anon(&parts.headers);
    if !s.hub.core.reply(&id, u.as_ref(), &rid, st) {
        return err(err_no_session());
    }
    no_content()
}

async fn queue_next(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let max = 4 << 10;
    let b = read_body(body, max).await;
    let n = match decode_body(&b, max) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let (mut reason, mut presence, mut pick) = (String::new(), None, String::new());
    let mut bd = Bind::new(&b);
    bd.object(&n, "", "struct { Reason string \"json:\\\"reason\\\"\"; Presence *api.Presence \"json:\\\"presence\\\"\"; Item string \"json:\\\"item\\\"\" }", &["reason", "presence", "item"], |bd, i, v| match i {
        0 => bd.string(v, &mut reason),
        1 => Presence::bind(bd, v, &mut presence),
        _ => bd.string(v, &mut pick),
    });
    if let Err(e) = bind_err(bd) {
        return err(e);
    }
    if !matches!(reason.as_str(), "ended" | "length" | "skip" | "resume" | "pick") {
        return err(ApiError::new(400, "bad_request", "reason must be ended, length, skip, resume or pick"));
    }
    // pick: a row the person chose (its id, or a CID); skip may name one too (an MCP skip to an item)
    if reason == "pick" && pick.trim().is_empty() {
        return err(ApiError::new(400, "bad_request", "pick needs item: the id of the row to play"));
    }
    let u = s.user_or_anon(&parts.headers);
    let pick = Some(pick.as_str()).filter(|p| !p.trim().is_empty());
    let Some((it, v)) = s.hub.core.next_pick(&id, u.as_ref(), presence.as_ref(), pick) else { return err(err_no_session()) };
    let item = match it {
        None => "null".to_string(),
        Some(it) => {
            let mut o = format!("{{\"bars\":{},\"cid\":{}", it.bars, q(&it.cid));
            if !it.example.is_empty() {
                o.push_str(&format!(",\"example\":{}", q(&it.example)));
            }
            o.push_str(&format!(",\"id\":{},\"minutes\":{},\"source\":{}", q(&it.id), gojson::jfloat(it.minutes), q(&it.source)));
            if !it.text.is_empty() {
                o.push_str(&format!(",\"text\":{}", q(&it.text)));
            }
            o.push_str(&format!(",\"title\":{}}}", q(&it.title)));
            o
        }
    };
    json_answer(200, format!("{{\"item\":{item},\"queue\":{}}}", v.to_json()))
}

/// `POST /api/session/<id>/queue` `{action: add|remove|move|clear, items?: [{cid} | {example}], item?, to?}`: the page's
/// own playlist edits (the radio page). The id is the capability, as for the page's other routes. An added item is a
/// program that already exists here (a stored program's CID or an example's, gated as MCP's add gates it); nothing is
/// composed, rendered or stored. Answers `{queue}`.
async fn queue_write(State(s): State<HS>, Path(id): Path<String>, req: Request) -> Response<Body> {
    let (ok, retry) = s.hub.queue_ip.allow(&peer_ip(&req), s.hub.now());
    if !ok {
        return err(limited(retry, "queue"));
    }
    let (parts, body) = req.into_parts();
    let max = 16 << 10;
    let b = read_body(body, max).await;
    if b.len() > max {
        return err(ApiError::new(413, "too_large", format!("request body is over {max} bytes")));
    }
    let v: serde_json::Value = match serde_json::from_slice(&b) {
        Ok(v) => v,
        Err(e) => return err(ApiError::new(400, "bad_request", format!("the body must be JSON: {e}"))),
    };
    let op = match page_queue_op(&s.app, &v, s.hub.core.queue_cfg.max_per_add) {
        Ok(op) => op,
        Err(e) => return err(e),
    };
    let u = s.user_or_anon(&parts.headers);
    match s.hub.core.page_queue_write(&id, u.as_ref(), op) {
        None => err(err_no_session()),
        Some(Err(e)) => err(e),
        Some(Ok(v)) => json_answer(200, format!("{{\"queue\":{}}}", v.to_json())),
    }
}

/// The page's queue write as a [`QueueOp`], its items resolved against the store and the examples.
fn page_queue_op(app: &crate::App, v: &serde_json::Value, max_per_add: usize) -> Result<types::QueueOp, ApiError> {
    let bad = |m: &str| ApiError::new(400, "bad_request", m.to_string());
    let Some(o) = v.as_object() else { return Err(bad("the body must be a JSON object")) };
    if let Some(k) = o.keys().find(|k| !["action", "items", "item", "to"].contains(&k.as_str())) {
        return Err(bad(&format!("{} is no field of a queue write: action, items, item, to", gojson::go_quote(k))));
    }
    let action = o.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
    if !matches!(action.as_str(), "add" | "remove" | "move" | "clear") {
        return Err(bad("action must be add, remove, move or clear"));
    }
    let item = o.get("item").and_then(|i| i.as_str()).unwrap_or("").trim().to_string();
    if item.len() > 100 {
        return Err(bad("item is an id from the queue (q1 ...) or a CID"));
    }
    if matches!(action.as_str(), "remove" | "move") && item.is_empty() {
        return Err(bad(&format!("{action} needs item: an id from the queue (q1 ...) or a CID")));
    }
    let mut op = types::QueueOp { action: action.clone(), remove: item, ..Default::default() };
    if action == "move" {
        let Some(to) = o.get("to").and_then(|t| t.as_u64()).filter(|t| *t <= 1023) else { return Err(bad("move needs to: the index the item goes to, 0 to 1023")) };
        op.to = to as usize;
    }
    if action == "add" {
        let Some(raw) = o.get("items").and_then(|i| i.as_array()).filter(|r| !r.is_empty()) else { return Err(bad("add needs items: a list of {cid} or {example}")) };
        if raw.len() > max_per_add {
            return Err(ApiError::new(413, "too_large", format!("one add takes at most {max_per_add} items; this one has {}", raw.len())));
        }
        for (i, r) in raw.iter().enumerate() {
            op.items.push(page_queue_item(app, r).map_err(|mut e| {
                e.message = format!("items[{i}]: {}", e.message);
                e
            })?);
        }
    }
    Ok(op)
}

/// One `{cid}` (a stored program or an example's) or `{example}` (by name) for the page's add.
fn page_queue_item(app: &crate::App, r: &serde_json::Value) -> Result<types::QueueItem, ApiError> {
    let bad = |m: &str| ApiError::new(400, "bad_request", m.to_string());
    let Some(o) = r.as_object() else { return Err(bad("an item is {cid} or {example}")) };
    let s = |k: &str| o.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let (cid, example) = (s("cid"), s("example"));
    if o.keys().any(|k| k != "cid" && k != "example") || cid.is_empty() == example.is_empty() {
        return Err(bad("an item is {cid} or {example}, one of them"));
    }
    if !example.is_empty() {
        let Some(e) = app.examples.list.iter().find(|e| e.name == example) else { return Err(ApiError::new(404, "not_found", format!("no example is named {}", gojson::go_quote(&example)))) };
        return Ok(types::QueueItem { cid: e.cid.clone(), title: e.title.clone(), source: "example".into(), example: e.name.clone(), ..Default::default() });
    }
    if !crate::store::is_cid(&cid) {
        return Err(bad("cid is not a CID (it starts with z)"));
    }
    if let Some(e) = app.examples.list.iter().find(|e| e.cid == cid) {
        return Ok(types::QueueItem { cid, title: e.title.clone(), source: "example".into(), example: e.name.clone(), ..Default::default() });
    }
    let Some((text, meta)) = crate::compose::stored(app, &cid) else {
        return Err(ApiError::new(404, "not_found", format!("no stored program has CID {cid}: the queue plays only programs that exist here (share one first)")));
    };
    beatscript_serve::gate::admit(&text, meta["license"].as_str().unwrap_or(""), meta["visibility"].as_str().unwrap_or("")).map_err(|r| ApiError::new(r.status, r.code, r.message))?;
    Ok(types::QueueItem { cid, title: meta["name"].as_str().unwrap_or("").to_string(), source: "store".into(), ..Default::default() })
}

async fn queue_get(State(s): State<HS>, Path(id): Path<String>, h: HeaderMap) -> Response<Body> {
    match s.hub.core.page_queue(&id, s.user_or_anon(&h).as_ref()) {
        Some(v) => json_answer(200, format!("{{\"queue\":{}}}", v.to_json())),
        None => err(err_no_session()),
    }
}

/// A method no pattern of this path takes: Go's catch-all (the static site) answers it.
async fn other_method(State(s): State<HS>, req: Request) -> Response<Body> {
    s.app.site.serve(req.method(), req.uri().path(), req.headers())
}

/// What every hub answer gets on its way out: the reason phrase net/http writes for 413 ("Request Entity Too
/// Large", where hyper says "Payload Too Large").
async fn finish(mut r: Response<Body>) -> Response<Body> {
    if r.status().as_u16() == 413 {
        r.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Request Entity Too Large"));
    }
    r
}

/// `loadSite`: `<data>/site.json`'s id, made the first time (16 random bytes, base64url) as Go makes it; `-site-id`
/// must agree with the file.
fn load_site(data: &std::path::Path, over: &str) -> Result<String, String> {
    let path = data.join("site.json");
    let valid = |s: &str| (16..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');
    if !over.is_empty() && !valid(over) {
        return Err("-site-id must be 16..64 characters of A-Z a-z 0-9 - _".into());
    }
    match std::fs::read(&path) {
        Ok(b) => {
            let id = serde_json::from_slice::<serde_json::Value>(&b).ok().and_then(|v| v.get("site_id").and_then(|s| s.as_str()).map(str::to_string)).filter(|s| valid(s));
            let Some(id) = id else { return Err(format!("{} is not a valid site file", path.display())) };
            if !over.is_empty() && over != id {
                return Err(format!("-site-id {} differs from {} ({}): a site keeps one id for good", gojson::go_quote(over), path.display(), gojson::go_quote(&id)));
            }
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = if over.is_empty() {
                let mut r = [0u8; 16];
                getrandom::fill(&mut r).map_err(|e| e.to_string())?;
                b64url(&r)
            } else {
                over.to_string()
            };
            std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
            let mut o = String::from("{\n  \"site_id\": ");
            jstr(&mut o, &id);
            o.push_str(",\n  \"created\": ");
            jstr(&mut o, &rfc3339(Clock::system().now()));
            o.push_str("\n}");
            let tmp = data.join("site.json.tmp");
            std::fs::write(&tmp, o).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn routes(app: &AppState) -> Router {
    let cfg = &app.cfg;
    let data = cfg.data_dir().to_path_buf();
    let pairing = Pairing::open(&data).unwrap_or_else(|e| {
        eprintln!("beatscript-serve: {e}");
        std::process::exit(1);
    });
    let site_id = load_site(&data, &cfg.flag("site-id", "")).unwrap_or_else(|e| {
        eprintln!("beatscript-serve: {e}");
        std::process::exit(1);
    });
    let hosts = beatscript_serve::playlists::Config::hosts_from(&cfg.public_url, &cfg.alt_origins, &cfg.legacy_hosts);
    let qc = QueueConfig::new(cfg.flag_num("queue-idle-minutes", 30), cfg.flag_num("queue-max-auto", 8), cfg.flag_num("queue-max-items", 50), cfg.flag_num("queue-max-per-add", 20));
    let mut hub = Hub::new(Clock::system(), qc, pairing, site_id, hosts, &cfg.public_url, &cfg.flag("moved-to", ""), cfg.max_program_bytes, cfg.flag_num("max-prompt-chars", 800).max(0) as usize);
    hub.sse_ping = Duration::from_millis(cfg.flag_num("sse-ping-ms", 20000).max(1) as u64);
    let hub = Arc::new(hub);
    let _ = app.hub.set(hub.clone());
    {
        // the sweeper: sessions with no stream, unseen for ten minutes
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(30));
            t.tick().await;
            loop {
                t.tick().await;
                hub.core.sweep();
            }
        });
    }
    let st = HS { app: app.clone(), hub };
    let mut r = Router::new();
    for p in [
        "/pair", "/api/pair", "/api/pair/{id}", "/api/devices", "/api/devices/{id}", "/api/devices/{id}/push", "/api/device", "/api/device/events",
        "/api/device/report", "/api/device/reply", "/api/device/compose", "/api/session", "/api/session/{id}/events", "/api/session/{id}/report",
        "/api/session/{id}/reply", "/api/session/{id}/queue", "/api/session/{id}/queue/next",
    ] {
        r = r.route(p, any(route));
    }
    r.with_state(st).layer(axum::middleware::map_response(finish))
}

/// Go's mux for the hub's patterns: method and path to a handler; any other method is the catch-all's (the static
/// site). `any` rather than axum's method routers, which would add an `Allow` header Go does not send.
async fn route(State(s): State<HS>, Path(params): Path<HashMap<String, String>>, req: Request) -> Response<Body> {
    let id = params.get("id").cloned().unwrap_or_default();
    let get = req.method() == Method::GET || req.method() == Method::HEAD;
    let post = req.method() == Method::POST;
    let del = req.method() == Method::DELETE;
    let path = req.uri().path().to_string();
    let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let (st, h) = (State(s.clone()), req.headers().clone());
    match (seg.as_slice(), get, post, del) {
        (["pair"], true, _, _) => pair_page().await,
        (["api", "pair"], _, true, _) => pair_new(st, req).await,
        (["api", "pair", "claim"], _, true, _) => pair_claim(st, req).await,
        (["api", "pair", _], true, _, _) => pair_state(st, Path(id), h).await,
        (["api", "devices"], true, _, _) => devices_list(st, h).await,
        (["api", "devices", _], _, _, true) => device_unpair(st, Path(id), h).await,
        (["api", "devices", _, "push"], _, true, _) => device_push(st, Path(id), req).await,
        (["api", "device"], true, _, _) => device_me(st, req).await,
        (["api", "device"], _, _, true) => device_forget(st, req).await,
        (["api", "device", "events"], true, _, _) => device_events(st, req).await,
        (["api", "device", "report"], _, true, _) => device_report(st, req).await,
        (["api", "device", "reply"], _, true, _) => device_reply(st, req).await,
        (["api", "device", "compose"], _, true, _) => device_compose(st, req).await,
        (["api", "session"], _, true, _) => session_create(st, req).await,
        (["api", "session", _, "events"], true, _, _) => session_events(st, Path(id), req).await,
        (["api", "session", _, "report"], _, true, _) => session_report(st, Path(id), req).await,
        (["api", "session", _, "reply"], _, true, _) => session_reply(st, Path(id), req).await,
        (["api", "session", _, "queue"], true, _, _) => queue_get(st, Path(id), h).await,
        (["api", "session", _, "queue"], _, true, _) => queue_write(st, Path(id), req).await,
        (["api", "session", _, "queue", "next"], _, true, _) => queue_next(st, Path(id), req).await,
        _ => other_method(st, req).await,
    }
}

