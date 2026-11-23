//! Go `internal/btwsite/limits`: what stands between compose and a surprise bill. Per-user hour and day windows, a
//! per-address day window, the daily spend cap in USD (held in reserve before each model call, settled after it,
//! persisted in `<data>/spend/YYYY-MM-DD.json` exactly as Go writes it), the kill switch (`BEATSCRIPT_COMPOSE_ENABLED`,
//! `<data>/compose.off`), the salted address hash (`<data>/ip-salt`, shared with Go) and the JSONL log
//! `<data>/log/compose-YYYYMMDD.jsonl`.
//!
//! The clock is injected (unix seconds + nanoseconds), so the tests walk windows and days without sleeping.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::gojson::{self, obj};
use crate::ApiError;

pub type Clock = Arc<dyn Fn() -> (i64, u32) + Send + Sync>;

/// Time as nanoseconds since the epoch (what the windows compare).
fn ns(t: (i64, u32)) -> i128 {
    i128::from(t.0) * 1_000_000_000 + i128::from(t.1)
}

const HOUR: i128 = 3600 * 1_000_000_000;
const DAY: i128 = 24 * HOUR;

#[derive(Clone, Debug)]
pub struct Config {
    pub user_hour: i64,
    pub user_day: i64,
    pub ip_day: i64,
    pub daily_spend_usd: f64,
    pub price_in_per_m: f64,
    pub price_out_per_m: f64,
    /// "" keeps everything in memory (no persistence, no log).
    pub data_dir: Option<PathBuf>,
}

impl Config {
    /// `WithDefaults`: a zero is the default, a negative "no limit".
    pub fn with_defaults(mut self) -> Config {
        if self.user_hour == 0 {
            self.user_hour = 10;
        }
        if self.user_day == 0 {
            self.user_day = 50;
        }
        if self.ip_day == 0 {
            self.ip_day = 40;
        }
        if self.daily_spend_usd == 0.0 {
            self.daily_spend_usd = 2.0;
        }
        if self.price_in_per_m == 0.0 {
            self.price_in_per_m = 1.5;
        }
        if self.price_out_per_m == 0.0 {
            self.price_out_per_m = 7.5;
        }
        self
    }
}

pub const ENV_ENABLED: &str = "BEATSCRIPT_COMPOSE_ENABLED";
pub const FLAG_FILE: &str = "compose.off";

#[derive(Clone, Debug, Default)]
struct DayRecord {
    date: String,
    usd: f64,
    requests: i64,
    tokens_in: i64,
    tokens_out: i64,
    /// Go's map: written sorted.
    outcomes: std::collections::BTreeMap<String, i64>,
}

impl DayRecord {
    fn json(&self) -> Value {
        let mut v = obj([
            ("date", json!(self.date)),
            ("usd", gojson::num(self.usd)),
            ("requests", json!(self.requests)),
            ("tokens_in", json!(self.tokens_in)),
            ("tokens_out", json!(self.tokens_out)),
        ]);
        if !self.outcomes.is_empty() {
            v["outcomes"] = Value::Object(self.outcomes.iter().map(|(k, n)| (k.clone(), json!(n))).collect());
        }
        v
    }

    fn parse(b: &[u8]) -> Option<DayRecord> {
        let v: Value = serde_json::from_slice(b).ok()?;
        let o = v.as_object()?;
        let int = |k: &str| -> Option<i64> {
            match o.get(k) {
                None | Some(Value::Null) => Some(0),
                Some(x) => x.as_i64(),
            }
        };
        let usd = match o.get("usd") {
            None | Some(Value::Null) => 0.0,
            Some(x) => x.as_f64()?,
        };
        let mut outcomes = std::collections::BTreeMap::new();
        match o.get("outcomes") {
            None | Some(Value::Null) => {}
            Some(Value::Object(m)) => {
                for (k, n) in m {
                    outcomes.insert(k.clone(), n.as_i64()?);
                }
            }
            Some(_) => return None,
        }
        if !matches!(o.get("date"), None | Some(Value::Null) | Some(Value::String(_))) {
            return None;
        }
        Some(DayRecord { date: String::new(), usd, requests: int("requests")?, tokens_in: int("tokens_in")?, tokens_out: int("tokens_out")?, outcomes })
    }
}

/// Whether compose is switched off and by what.
#[derive(Clone, Copy, Debug, Default)]
pub struct KillState {
    pub disabled: bool,
    pub env: bool,
    pub file: bool,
}

struct State {
    users: HashMap<String, Vec<i128>>,
    ips: HashMap<String, Vec<i128>>,
    calls: u64,
    day: String,
    rec: DayRecord,
    reserved: f64,
    spend_broke: String,
    persist_err: String,
    mem_off: bool,
    log: Option<(std::fs::File, String)>,
}

pub struct Limiter {
    cfg: Config,
    now: Clock,
    /// `BEATSCRIPT_COMPOSE_ENABLED` as the process sees it (the flag merged with the environment at start).
    env_enabled: Mutex<Box<dyn Fn() -> Option<String> + Send + Sync>>,
    salt: Vec<u8>,
    st: Mutex<State>,
}

/// One admission's worth of the windows, or the spend cap.
pub struct Reservation {
    l: Arc<Limiter>,
    est: f64,
    done: bool,
}

fn env_off(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "off" | "no")
}

/// `writeAtomic`: a temp file in the same directory, chmod, fsync, rename.
pub fn write_atomic(path: &Path, b: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let mut rnd = [0u8; 6];
    let _ = getrandom::fill(&mut rnd);
    let tmp = dir.join(format!(".{base}.tmp-{}", u64::from_le_bytes([rnd[0], rnd[1], rnd[2], rnd[3], rnd[4], rnd[5], 0, 0]) % 10_000_000_000));
    let r = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(b)?;
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn load_salt(dir: Option<&Path>) -> Result<Vec<u8>, String> {
    if let Some(d) = dir {
        if let Ok(b) = std::fs::read_to_string(d.join("ip-salt")) {
            if let Ok(s) = hex::decode(b.trim()) {
                if s.len() >= 16 {
                    return Ok(s);
                }
            }
        }
    }
    let mut s = vec![0u8; 32];
    getrandom::fill(&mut s).map_err(|e| format!("limits: salt: {e}"))?;
    if let Some(d) = dir {
        write_atomic(&d.join("ip-salt"), format!("{}\n", hex::encode(&s)).as_bytes(), 0o600).map_err(|e| format!("limits: salt: {e}"))?;
    }
    Ok(s)
}

/// `api.IPKey` of what `crate::client_ip` returns: IPv4 as it is, IPv6 as its /64 in canonical form.
pub fn ip_key(ip: &str) -> String {
    if let Some(p) = ip.strip_suffix("/64") {
        if let Ok(a) = p.parse::<std::net::Ipv6Addr>() {
            return format!("{a}/64");
        }
    }
    if let Ok(a) = ip.parse::<std::net::IpAddr>() {
        return match a {
            std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => v4.to_string(),
                None => {
                    let s = v6.segments();
                    format!("{}/64", std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
                }
            },
            v4 => v4.to_string(),
        };
    }
    ip.to_string()
}

fn prune(ts: &mut Vec<i128>, now: i128) {
    let cut = now - DAY;
    let i = ts.iter().position(|t| *t > cut).unwrap_or(ts.len());
    ts.drain(..i);
}

/// The window of length `w` already holds `limit` events: how long until one leaves.
fn window_full(ts: &[i128], now: i128, w: i128, limit: i64) -> Option<i128> {
    if limit < 0 {
        return None;
    }
    let cut = now - w;
    let i = ts.partition_point(|t| *t <= cut);
    let inw = &ts[i..];
    if (inw.len() as i64) < limit {
        return None;
    }
    let oldest = inw[inw.len() - limit as usize];
    Some(oldest + w - now)
}

fn sweep(m: &mut HashMap<String, Vec<i128>>, now: i128) {
    m.retain(|_, ts| {
        prune(ts, now);
        !ts.is_empty()
    });
}

fn human_wait(secs: i64) -> String {
    if secs >= 7200 {
        format!("about {} hours", (secs + 1800) / 3600)
    } else if secs >= 120 {
        format!("about {} minutes", (secs + 30) / 60)
    } else {
        format!("{secs} seconds")
    }
}

fn rate_limited(which: &str, n: i64, wait: i128) -> ApiError {
    let secs = ((wait + 999_999_999) / 1_000_000_000).max(1) as i64;
    ApiError::new(429, "rate_limited", format!("compose {which} limit reached ({n}); try again in {}", human_wait(secs))).retry(secs as u64)
}

/// One line of the compose log (no key, no prompt, no program).
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub user: String,
    pub ip_hash: String,
    pub prompt_chars: i64,
    pub prompt_sha: String,
    pub prompt_head: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
    pub attempts: i64,
    pub outcome: String,
    pub cid: String,
    pub forked_from: String,
    pub detail: String,
    pub ms: i64,
}

/// `PromptHash`: the SHA-256 hex and the first 80 characters (tabs and newlines as spaces, controls dropped).
pub fn prompt_hash(prompt: &str) -> (String, String) {
    let sum = hex::encode(Sha256::digest(prompt.as_bytes()));
    let mut head = String::new();
    let mut n = 0;
    for mut r in prompt.chars() {
        if n >= 80 {
            break;
        }
        if r == '\n' || r == '\t' {
            r = ' ';
        }
        if (r as u32) < 0x20 || r == '\u{7f}' {
            continue;
        }
        head.push(r);
        n += 1;
    }
    (sum, head)
}

impl Limiter {
    pub fn new(cfg: Config, now: Clock) -> Result<Arc<Limiter>, String> {
        let cfg = cfg.with_defaults();
        if let Some(d) = &cfg.data_dir {
            for sub in ["", "spend", "log"] {
                let p = if sub.is_empty() { d.clone() } else { d.join(sub) };
                if !p.exists() {
                    std::fs::create_dir_all(&p).map_err(|e| format!("limits: {e}"))?;
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700));
                }
            }
        }
        let salt = load_salt(cfg.data_dir.as_deref())?;
        let st = State {
            users: HashMap::new(),
            ips: HashMap::new(),
            calls: 0,
            day: String::new(),
            rec: DayRecord::default(),
            reserved: 0.0,
            spend_broke: String::new(),
            persist_err: String::new(),
            mem_off: false,
            log: None,
        };
        let l = Arc::new(Limiter { cfg, now, env_enabled: Mutex::new(Box::new(|| std::env::var(ENV_ENABLED).ok())), salt, st: Mutex::new(st) });
        {
            let mut st = l.st.lock().unwrap();
            if let Err(e) = l.roll(&mut st, true) {
                // Go refuses to start; beatscript-serve starts closed (spend_cap) and says why
                eprintln!("beatscript-serve: compose: {e}");
            }
        }
        Ok(l)
    }

    /// Replace the kill switch's environment lookup (main: the `-compose-enabled` flag merged with the env).
    pub fn set_env_enabled(&self, f: Box<dyn Fn() -> Option<String> + Send + Sync>) {
        *self.env_enabled.lock().unwrap() = f;
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The USD price of a call.
    pub fn cost(&self, tokens_in: i64, tokens_out: i64) -> f64 {
        tokens_in as f64 * self.cfg.price_in_per_m / 1e6 + tokens_out as f64 * self.cfg.price_out_per_m / 1e6
    }

    /// `HashIP`: salted SHA-256 of the address (the /64 for IPv6).
    pub fn hash_ip(&self, ip: &str) -> String {
        if ip.is_empty() {
            return String::new();
        }
        let mut h = Sha256::new();
        h.update(&self.salt);
        h.update([0u8]);
        h.update(ip_key(ip).as_bytes());
        hex::encode(h.finalize())
    }

    // ---------- kill switch ----------

    pub fn kill(&self) -> KillState {
        let env = (self.env_enabled.lock().unwrap())().map(|v| env_off(&v)).unwrap_or(false);
        let file = match &self.cfg.data_dir {
            Some(d) => d.join(FLAG_FILE).exists(),
            None => self.st.lock().unwrap().mem_off,
        };
        KillState { disabled: env || file, env, file }
    }

    pub fn set_enabled(&self, enabled: bool) -> std::io::Result<()> {
        let Some(d) = &self.cfg.data_dir else {
            self.st.lock().unwrap().mem_off = !enabled;
            return Ok(());
        };
        let p = d.join(FLAG_FILE);
        if enabled {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            write_atomic(&p, format!("compose switched off at {}\n", gojson::rfc3339((self.now)().0)).as_bytes(), 0o600)
        }
    }

    // ---------- windows + spend ----------

    /// `Begin`: user hour, user day, address day, then the cap, holding `est` USD. Nothing is recorded on a refusal.
    pub fn begin(self: &Arc<Self>, user: &str, ip: &str, est: f64) -> Result<Reservation, ApiError> {
        let mut st = self.st.lock().unwrap();
        let now = ns((self.now)());
        let ip_key = if ip.is_empty() { String::new() } else { self.hash_ip(ip) };
        st.calls += 1;
        if st.calls % 256 == 0 {
            sweep(&mut st.users, now);
            sweep(&mut st.ips, now);
        }
        let mut u = st.users.get(user).cloned().unwrap_or_default();
        prune(&mut u, now);
        if let Some(w) = window_full(&u, now, HOUR, self.cfg.user_hour) {
            return Err(rate_limited("hourly", self.cfg.user_hour, w));
        }
        if let Some(w) = window_full(&u, now, DAY, self.cfg.user_day) {
            return Err(rate_limited("daily", self.cfg.user_day, w));
        }
        let mut p = Vec::new();
        if !ip_key.is_empty() {
            p = st.ips.get(&ip_key).cloned().unwrap_or_default();
            prune(&mut p, now);
            if let Some(w) = window_full(&p, now, DAY, self.cfg.ip_day) {
                return Err(rate_limited("per-address daily", self.cfg.ip_day, w));
            }
        }
        self.reserve_locked(&mut st, est)?;
        u.push(now);
        st.users.insert(user.to_string(), u);
        if !ip_key.is_empty() {
            p.push(now);
            st.ips.insert(ip_key, p);
        }
        Ok(Reservation { l: self.clone(), est, done: false })
    }

    pub fn reserve(self: &Arc<Self>, est: f64) -> Result<Reservation, ApiError> {
        let mut st = self.st.lock().unwrap();
        self.reserve_locked(&mut st, est)?;
        Ok(Reservation { l: self.clone(), est, done: false })
    }

    fn reserve_locked(&self, st: &mut State, est: f64) -> Result<(), ApiError> {
        let _ = self.roll(st, false);
        if !st.spend_broke.is_empty() {
            return Err(ApiError::new(503, "spend_cap", "compose is closed: its spend record is unreadable").retry(300));
        }
        if self.cfg.daily_spend_usd >= 0.0 && st.rec.usd + st.reserved + est > self.cfg.daily_spend_usd {
            return Err(ApiError::new(503, "spend_cap", "today's compose budget is used up; it resets at 00:00 UTC").retry(self.seconds_to_reset() as u64));
        }
        st.reserved += est;
        Ok(())
    }

    fn seconds_to_reset(&self) -> i64 {
        let (s, n) = (self.now)();
        let next = i128::from(gojson::next_midnight(s)) * 1_000_000_000;
        let d = next - ns((s, n));
        ((d + 999_999_999) / 1_000_000_000).max(1) as i64
    }

    fn spend_path(&self, day: &str) -> Option<PathBuf> {
        self.cfg.data_dir.as_ref().map(|d| d.join("spend").join(format!("{day}.json")))
    }

    /// `rollLocked`: the record is today's (UTC), loaded from disk; an unreadable one closes compose.
    fn roll(&self, st: &mut State, first: bool) -> Result<(), String> {
        let day = gojson::day((self.now)().0);
        if !first && day == st.day {
            return Ok(());
        }
        st.day = day.clone();
        st.rec = DayRecord { date: day.clone(), ..Default::default() };
        st.spend_broke.clear();
        let Some(p) = self.spend_path(&day) else { return Ok(()) };
        let err = match std::fs::read(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => e.to_string(),
            Ok(b) => match DayRecord::parse(&b) {
                Some(mut r) => {
                    r.date = day;
                    st.rec = r;
                    return Ok(());
                }
                None => "invalid JSON".to_string(),
            },
        };
        let msg = format!("the spend file {} is unreadable ({err}); compose stays closed until it is fixed or removed", p.display());
        st.spend_broke = msg.clone();
        Err(format!("limits: {msg}"))
    }

    fn persist(&self, st: &mut State) {
        let Some(p) = self.spend_path(&st.day) else { return };
        let mut b = gojson::indent(&st.rec.json(), " ").into_bytes();
        b.push(b'\n');
        match write_atomic(&p, &b, 0o600) {
            Ok(()) => st.persist_err.clear(),
            Err(e) => st.persist_err = e.to_string(),
        }
    }

    fn settle(&self, est: f64, cost: f64, tin: i64, tout: i64) {
        let mut st = self.st.lock().unwrap();
        st.reserved -= est;
        if st.reserved < 1e-12 {
            st.reserved = 0.0;
        }
        let _ = self.roll(&mut st, false);
        st.rec.usd += cost;
        st.rec.tokens_in += tin;
        st.rec.tokens_out += tout;
        self.persist(&mut st);
    }

    /// Today's recorded spend.
    pub fn spent(&self) -> f64 {
        let mut st = self.st.lock().unwrap();
        let _ = self.roll(&mut st, false);
        st.rec.usd
    }

    // ---------- log ----------

    /// `Record`: the outcome counted in the day's table, the line appended to the day's log.
    pub fn record(&self, e: &Entry) {
        let mut st = self.st.lock().unwrap();
        let (s, n) = (self.now)();
        let _ = self.roll(&mut st, false);
        st.rec.requests += 1;
        if !e.outcome.is_empty() {
            *st.rec.outcomes.entry(e.outcome.clone()).or_insert(0) += 1;
        }
        self.persist(&mut st);
        let Some(dir) = &self.cfg.data_dir else { return };
        let day = gojson::day_compact(s);
        if st.log.as_ref().map(|(_, d)| d != &day).unwrap_or(true) {
            st.log = None;
            match std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(dir.join("log").join(format!("compose-{day}.jsonl"))) {
                Ok(f) => st.log = Some((f, day)),
                Err(err) => {
                    st.persist_err = err.to_string();
                    return;
                }
            }
        }
        let mut v = obj([
            ("ts", json!(gojson::rfc3339_nano(s, n))),
            ("user", json!(e.user)),
            ("ip_hash", json!(e.ip_hash)),
            ("prompt_chars", json!(e.prompt_chars)),
            ("prompt_sha256", json!(e.prompt_sha)),
            ("prompt_head", json!(e.prompt_head)),
            ("model", json!(e.model)),
            ("tokens_in", json!(e.tokens_in)),
            ("tokens_out", json!(e.tokens_out)),
            ("cost_usd", gojson::num(e.cost_usd)),
            ("attempts", json!(e.attempts)),
            ("outcome", json!(e.outcome)),
        ]);
        let m = v.as_object_mut().unwrap();
        for (k, x) in [("cid", &e.cid), ("forked_from", &e.forked_from), ("detail", &e.detail)] {
            if !x.is_empty() {
                m.insert(k.into(), json!(x));
            }
        }
        m.insert("ms".into(), json!(e.ms));
        let line = gojson::line(&v);
        if let Some((f, _)) = st.log.as_mut() {
            if let Err(err) = f.write_all(line.as_bytes()) {
                st.persist_err = err.to_string();
            }
        }
    }

    // ---------- stats ----------

    /// `Stats` as encoding/json writes the struct.
    pub fn stats(&self) -> Value {
        let mut st = self.st.lock().unwrap();
        let _ = self.roll(&mut st, false);
        let now = ns((self.now)());
        sweep(&mut st.users, now);
        sweep(&mut st.ips, now);
        let outcomes: serde_json::Map<String, Value> = st.rec.outcomes.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
        let mut v = obj([
            ("date", json!(st.day)),
            ("spent_usd", gojson::num(st.rec.usd)),
            ("reserved_usd", gojson::num(st.reserved)),
            ("cap_usd", gojson::num(self.cfg.daily_spend_usd)),
            ("requests", json!(st.rec.requests)),
            ("tokens_in", json!(st.rec.tokens_in)),
            ("tokens_out", json!(st.rec.tokens_out)),
            ("outcomes", Value::Object(outcomes)),
            ("users_tracked", json!(st.users.len())),
            ("ips_tracked", json!(st.ips.len())),
            ("reset_in_seconds", json!(self.seconds_to_reset())),
            ("user_per_hour", json!(self.cfg.user_hour)),
            ("user_per_day", json!(self.cfg.user_day)),
            ("ip_per_day", json!(self.cfg.ip_day)),
        ]);
        let m = v.as_object_mut().unwrap();
        if !st.spend_broke.is_empty() {
            m.insert("spend_closed".into(), json!(st.spend_broke));
        }
        if !st.persist_err.is_empty() {
            m.insert("persist_error".into(), json!(st.persist_err));
        }
        v
    }

    /// (spent, spend_closed) for Status.
    pub fn spend_state(&self) -> (f64, String) {
        let mut st = self.st.lock().unwrap();
        let _ = self.roll(&mut st, false);
        (st.rec.usd, st.spend_broke.clone())
    }
}

impl Reservation {
    /// Replace the reservation with the real cost and count the tokens.
    pub fn settle(mut self, cost: f64, tin: i64, tout: i64) {
        self.done = true;
        self.l.settle(self.est, cost, tin, tout);
    }
    pub fn release(self) {
        self.settle(0.0, 0, 0);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.done {
            self.l.settle(self.est, 0.0, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn clock() -> (Arc<AtomicI64>, Clock) {
        let t = Arc::new(AtomicI64::new(1_795_214_474));
        let c = t.clone();
        (t, Arc::new(move || (c.load(Ordering::SeqCst), 0)))
    }

    fn cfg(dir: Option<PathBuf>) -> Config {
        Config { user_hour: 2, user_day: 3, ip_day: 0, daily_spend_usd: 0.0, price_in_per_m: 0.0, price_out_per_m: 0.0, data_dir: dir }
    }

    #[test]
    fn windows_refuse_without_recording() {
        let (t, c) = clock();
        let l = Limiter::new(cfg(None), c).unwrap();
        l.begin("u", "1.2.3.4", 0.01).unwrap().release();
        l.begin("u", "1.2.3.4", 0.01).unwrap().release();
        let e = l.begin("u", "1.2.3.4", 0.01).err().unwrap();
        assert_eq!((e.status, e.code.as_str(), e.retry_after), (429, "rate_limited", 3600));
        assert_eq!(e.message, "compose hourly limit reached (2); try again in about 60 minutes");
        t.fetch_add(3601, Ordering::SeqCst);
        l.begin("u", "1.2.3.4", 0.01).unwrap().release();
        let e = l.begin("u", "1.2.3.4", 0.01).err().unwrap();
        assert!(e.message.starts_with("compose daily limit reached (3)"), "{}", e.message);
    }

    #[test]
    fn spend_cap_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_t, c) = clock();
        let l = Limiter::new(cfg(Some(dir.path().to_path_buf())), c.clone()).unwrap();
        let r = l.begin("u", "", 1.5).unwrap();
        let e = l.reserve(0.6).err().unwrap();
        assert_eq!((e.status, e.code.as_str()), (503, "spend_cap"));
        r.settle(0.008265, 2310, 640);
        l.record(&Entry { outcome: "ok".into(), ..Default::default() });
        let f = std::fs::read_to_string(dir.path().join("spend/2026-11-20.json")).unwrap();
        assert_eq!(f, "{\n \"date\": \"2026-11-20\",\n \"usd\": 0.008265,\n \"requests\": 1,\n \"tokens_in\": 2310,\n \"tokens_out\": 640,\n \"outcomes\": {\n  \"ok\": 1\n }\n}\n");
        // a second limiter reads it back
        let l2 = Limiter::new(cfg(Some(dir.path().to_path_buf())), c).unwrap();
        assert_eq!(l2.spent(), 0.008265);
        assert!(std::fs::read_to_string(dir.path().join("log/compose-20261120.jsonl")).unwrap().contains("\"outcome\":\"ok\""));
    }

    #[test]
    fn broken_spend_file_closes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("spend")).unwrap();
        std::fs::write(dir.path().join("spend/2026-11-20.json"), "{nope").unwrap();
        let (_t, c) = clock();
        let l = Limiter::new(cfg(Some(dir.path().to_path_buf())), c).unwrap();
        let e = l.begin("u", "", 0.01).err().unwrap();
        assert_eq!((e.code.as_str(), e.retry_after), ("spend_cap", 300));
    }

    #[test]
    fn kill_switch_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_t, c) = clock();
        let l = Limiter::new(cfg(Some(dir.path().to_path_buf())), c).unwrap();
        l.set_env_enabled(Box::new(|| None));
        assert!(!l.kill().disabled);
        l.set_enabled(false).unwrap();
        assert!(l.kill().file && l.kill().disabled);
        l.set_enabled(true).unwrap();
        assert!(!l.kill().disabled);
        l.set_env_enabled(Box::new(|| Some("0".into())));
        assert!(l.kill().env);
    }

    #[test]
    fn ipv6_key_is_canonical() {
        assert_eq!(ip_key("2001:db8:0:0::/64"), "2001:db8::/64");
        assert_eq!(ip_key("127.0.0.1"), "127.0.0.1");
    }
}
