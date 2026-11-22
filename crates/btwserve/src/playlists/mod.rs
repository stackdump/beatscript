//! Playlists for beatscript.app: `/api/playlists/*`. The contract (routes, auth, sync rules, limits, nginx) is
//! `docs/RUST-SERVER.md` § "Playlists" in beats-bitwrap-io; the browser side is `btwsite/web/js/playlists*.js`.
//!
//! - `auth`: the Go server's account session (cookie, bearer token, paired phone), verified against its files.
//! - `model`: the playlist, its validation and its version hash.
//! - `store`: one JSON file per account, written atomically; the share-token index.
//! - `api`: the axum routes.

#![allow(clippy::result_large_err)] // ApiError can carry the 409 copy; errors are the cold path

pub mod api;
pub mod auth;
pub mod model;
pub mod store;

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use api::router;

/// The peer of the TCP connection, put in the request's extensions by the server loop (as main.rs's `Peer`).
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);

/// An error answered as Go's `api.WriteError`: `{"error":{"code","message","retry_after"?}}` + newline.
#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: u64,
    /// More top-level fields beside "error" (409 carries the server's copy as "playlist").
    pub extra: Option<(String, serde_json::Value)>,
}

impl ApiError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> ApiError {
        ApiError { status, code: code.into(), message: message.into(), retry_after: 0, extra: None }
    }
    pub fn retry(mut self, s: u64) -> ApiError {
        self.retry_after = s;
        self
    }
    pub fn with(mut self, k: &str, v: serde_json::Value) -> ApiError {
        self.extra = Some((k.into(), v));
        self
    }
}

/// Go's JSON encoder escapes <, > and & in strings; so do we, so both servers' bodies look alike.
pub fn go_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

#[derive(Clone, Debug)]
pub struct Config {
    /// The Go server's `-data`.
    pub data: PathBuf,
    /// `BTW_SESSION_SECRET`, when set (trimmed).
    pub secret_env: Option<String>,
    /// `-public-url`, e.g. https://beatscript.app: share links are made under it.
    pub public_url: String,
    /// The site's hosts (Go's `acceptedHosts()`): the public host first, then `-alt-origins`, `-legacy-hosts`.
    pub hosts: Vec<String>,
    pub limits: model::Limits,
    /// The storage gate for items: an item must name an example or a stored program the gate admits.
    pub gate: ItemGate,
}

/// Where a playlist item's program is looked up for [`crate::gate::admit_stored`]: the programs directory and the
/// examples' CIDs.
#[derive(Clone, Debug, Default)]
pub struct ItemGate {
    pub programs: PathBuf,
    pub examples: std::sync::Arc<std::collections::HashSet<String>>,
}

impl ItemGate {
    pub fn new(programs: PathBuf, examples: impl IntoIterator<Item = String>) -> ItemGate {
        ItemGate { programs, examples: std::sync::Arc::new(examples.into_iter().collect()) }
    }

    /// The gate's verdict on the program `cid` names, as the API's 422.
    pub fn admit(&self, cid: &str) -> Result<(), ApiError> {
        crate::gate::admit_stored(cid, self.examples.contains(cid), || std::fs::read(self.programs.join(format!("{cid}.json")))
            .ok())
            .map_err(|r| ApiError::new(422, r.code, format!("item {cid}: {}", r.message)))
    }
}

impl Config {
    /// The host list from the flags, as Go's `acceptedHosts()` builds it.
    pub fn hosts_from(public_url: &str, alt_origins: &str, legacy_hosts: &str) -> Vec<String> {
        let host_of = |u: &str| u.split_once("://").map(|(_, r)| r.split('/').next().unwrap_or("").to_ascii_lowercase()).unwrap_or_default();
        let mut hosts: Vec<String> = vec![];
        let mut add = |h: String| {
            let h = h.trim().to_ascii_lowercase();
            if !h.is_empty() && !hosts.contains(&h) {
                hosts.push(h);
            }
        };
        add(host_of(public_url));
        for o in alt_origins.split(',') {
            add(host_of(o.trim()));
        }
        for h in legacy_hosts.split(',') {
            add(h.to_string());
        }
        hosts
    }
}

/// A sliding-window counter per key (an address, an account).
pub struct Limiter {
    n: usize,
    window: Duration,
    hits: HashMap<String, VecDeque<Instant>>,
    sweep_at: usize,
}

impl Limiter {
    pub fn new(n: usize, window: Duration) -> Limiter {
        Limiter { n, window, hits: HashMap::new(), sweep_at: 4096 }
    }
    /// (allowed, retry-after seconds); an allowed hit is recorded.
    pub fn allow(&mut self, key: &str) -> (bool, u64) {
        let now = Instant::now();
        if self.hits.len() >= self.sweep_at {
            let w = self.window;
            self.hits.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < w));
            self.sweep_at = (self.hits.len() * 2).max(4096);
        }
        let q = self.hits.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            q.pop_front();
        }
        if q.len() >= self.n {
            return (false, self.window.saturating_sub(now.duration_since(*q.front().unwrap())).as_secs() + 1);
        }
        q.push_back(now);
        (true, 0)
    }
}

pub struct Playlists {
    pub cfg: Config,
    pub auth: auth::Auth,
    pub store: store::Store,
    ip: Mutex<Limiter>,
    account: Mutex<Limiter>,
    writes: Mutex<Limiter>,
    resolve: Mutex<Limiter>,
}

impl Playlists {
    pub fn open(cfg: Config) -> std::io::Result<(Arc<Playlists>, Vec<String>)> {
        let (store, mut warnings) = store::Store::open(&cfg.data, cfg.limits.clone())?;
        let auth = auth::Auth::new(auth::AuthConfig { data: cfg.data.clone(), secret_env: cfg.secret_env.clone(), hosts: cfg.hosts.clone() });
        if !auth.ready() {
            warnings.push(format!("no session key (BTW_SESSION_SECRET or {}/session.secret, which the Go server makes): /api/playlists answers 503 until there is one", cfg.data.display()));
        }
        let l = &cfg.limits;
        let min = Duration::from_secs(60);
        let pl = Playlists {
            ip: Mutex::new(Limiter::new(l.requests_per_min * 2, min)),
            account: Mutex::new(Limiter::new(l.requests_per_min, min)),
            writes: Mutex::new(Limiter::new(l.writes_per_min, min)),
            resolve: Mutex::new(Limiter::new(l.resolve_per_min, min)),
            auth,
            store,
            cfg,
        };
        Ok((Arc::new(pl), warnings))
    }

    fn limit(l: &Mutex<Limiter>, key: &str, what: &str) -> Result<(), ApiError> {
        let (ok, retry) = l.lock().unwrap().allow(key);
        if ok {
            Ok(())
        } else {
            Err(ApiError::new(429, "rate_limited", format!("too many {what}; slow down")).retry(retry.max(1)))
        }
    }
}

/// The client's address, as `api.ClientIP`: behind a loopback peer (nginx) `X-Real-IP`, else the last
/// `X-Forwarded-For`; from anyone else the peer itself. IPv6 is bucketed by its /64.
pub fn client_ip(peer: Option<SocketAddr>, h: &axum::http::HeaderMap) -> String {
    use std::net::IpAddr;
    let Some(peer) = peer else { return "unknown".into() };
    let mut ip = peer.ip();
    if ip.is_loopback() {
        let real = h.get("x-real-ip").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<IpAddr>().ok());
        let fwd = h.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|s| s.rsplit(',').next()).and_then(|s| s.trim().parse::<IpAddr>().ok());
        if let Some(r) = real.or(fwd) {
            ip = r;
        }
    }
    match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        IpAddr::V6(v6) => v6.to_ipv4_mapped().unwrap().to_string(),
        v4 => v4.to_string(),
    }
}
