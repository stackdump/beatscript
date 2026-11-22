//! The djnet peer (docs/NET-PROTOCOL.md §4-§6, §10): one connection to a host with hello,
//! auth / pair and requests ([`Conn`]), and the managed link a phone keeps ([`Link`]): it
//! reconnects, pings the clock every second, and keeps the host's master on the peer's clock.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djengine::json::{JsonObject, JsonValue};

use super::clock::{mono_ns, BeatFit, BeatPoint, Exchange, OffsetEstimator};
use super::proto::{self, NetError, DEFAULT_PORT, MIN_VERSION, PROTO, VERSION};
use super::store::Store;

/// Who this peer is.
#[derive(Clone, Debug)]
pub struct Me {
    pub name: String,
    /// `djandroid/0.3.23`.
    pub app: String,
}

/// What `hello` said about the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hello {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub app: String,
    pub caps: Vec<String>,
    pub paired: bool,
    pub trust: String,
    /// The desktop's input strip the stream plays on.
    pub strip: String,
}

/// `host[:port]` (the default port when none), resolved.
pub fn resolve(host: &str) -> Result<SocketAddr, NetError> {
    let h = host.trim();
    let with_port = if h.parse::<SocketAddr>().is_ok() || h.rsplit_once(':').is_some_and(|(a, p)| !a.contains(':') && p.parse::<u16>().is_ok()) {
        h.to_string()
    } else {
        format!("{h}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| NetError::unavailable(format!("{h}: {e}")))?
        .next()
        .ok_or_else(|| NetError::unavailable(format!("{h}: no address")))
}

/// One connection: requests and their replies, in order.
pub struct Conn {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next: i64,
    pub addr: SocketAddr,
}

impl Conn {
    pub fn connect(addr: SocketAddr, timeout: Duration) -> Result<Conn, NetError> {
        let s = TcpStream::connect_timeout(&addr, timeout)
            .map_err(|e| NetError::unavailable(format!("{addr}: {e}")))?;
        let _ = s.set_nodelay(true);
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        let writer = s.try_clone().map_err(|e| NetError::unavailable(e.to_string()))?;
        Ok(Conn {
            reader: BufReader::new(s),
            writer,
            next: 1,
            addr,
        })
    }

    /// Send a request and wait for its reply. An I/O error means the connection is gone.
    pub fn call(&mut self, method: &str, params: JsonValue) -> Result<JsonValue, NetError> {
        let id = self.next;
        self.next += 1;
        let req = JsonValue::Object(JsonObject::from_pairs([
            ("id", JsonValue::int(id)),
            ("method", JsonValue::str(method)),
            ("params", params),
        ]));
        let mut line = req.js_stringified();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| NetError::unavailable(format!("send: {e}")))?;
        loop {
            let mut got = String::new();
            match self.reader.read_line(&mut got) {
                Ok(0) => return Err(NetError::unavailable("the desktop closed the connection")),
                Err(e) => return Err(NetError::unavailable(format!("read: {e}"))),
                Ok(_) => {}
            }
            let Ok(v) = JsonValue::parse(got.trim()) else { continue };
            if v.get("id").and_then(JsonValue::as_f64) != Some(id as f64) {
                continue; // an older request's reply after a timeout
            }
            if let Some(e) = v.get("error") {
                let code = v.get("code").and_then(JsonValue::as_str).unwrap_or("app");
                let code: &'static str = match code {
                    "version" => "version",
                    "unpaired" => "unpaired",
                    "bad_code" => "bad_code",
                    "locked_out" => "locked_out",
                    "forbidden" => "forbidden",
                    "unknown_cid" => "unknown_cid",
                    "cid_mismatch" => "cid_mismatch",
                    "busy" => "busy",
                    "unavailable" => "unavailable",
                    _ => "app",
                };
                return Err(NetError::new(code, e.as_str().unwrap_or("error").to_string()));
            }
            return Ok(v.get("result").cloned().unwrap_or(JsonValue::Null));
        }
    }

    /// `hello`, then `auth` with a stored token or `pair` with `code`. A new token is stored.
    pub fn open(
        addr: SocketAddr,
        me: &Me,
        store: &Mutex<Store>,
        code: Option<&str>,
    ) -> Result<(Conn, Hello), NetError> {
        let mut c = Conn::connect(addr, Duration::from_secs(3))?;
        let my_id = store.lock().map(|s| s.id.clone()).unwrap_or_default();
        let h = c.call(
            "hello",
            JsonValue::Object(JsonObject::from_pairs([
                ("proto", PROTO.into()),
                ("version", JsonValue::int(VERSION as i64)),
                ("min_version", JsonValue::int(MIN_VERSION as i64)),
                ("id", JsonValue::str(my_id)),
                ("name", JsonValue::str(me.name.clone())),
                ("app", JsonValue::str(me.app.clone())),
            ])),
        )?;
        let s = |k: &str| h.get(k).and_then(JsonValue::as_str).unwrap_or("").to_string();
        let mut hello = Hello {
            version: h.get("version").and_then(JsonValue::as_f64).unwrap_or(1.0) as u32,
            id: s("id"),
            name: s("name"),
            app: s("app"),
            caps: h
                .get("caps")
                .and_then(JsonValue::as_array)
                .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            paired: h.get("paired").and_then(JsonValue::as_bool).unwrap_or(false),
            trust: s("trust"),
            strip: s("strip"),
        };
        if hello.paired {
            return Ok((c, hello));
        }
        let token = store.lock().ok().and_then(|st| st.token_of(&hello.id).map(String::from));
        if let Some(token) = token {
            let r = c.call(
                "auth",
                JsonValue::Object(JsonObject::from_pairs([("proof", JsonValue::str(proto::proof(&token, &s("nonce"))))])),
            );
            match r {
                Ok(_) => {
                    hello.paired = true;
                    return Ok((c, hello));
                }
                // the desktop forgot us: pair again if a code was given
                Err(e) if e.code == "unpaired" && code.is_some() => {}
                Err(e) => return Err(e),
            }
        }
        let Some(code) = code.filter(|c| !c.trim().is_empty()) else {
            return Err(NetError::new(
                "unpaired",
                format!("enter the pairing code {} shows (Net window)", hello.name),
            ));
        };
        let r = c.call(
            "pair",
            JsonValue::Object(JsonObject::from_pairs([("code", JsonValue::str(code.trim()))])),
        )?;
        let token = r
            .get("token")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| NetError::new("unpaired", "the desktop sent no token"))?;
        if let Ok(mut st) = store.lock() {
            st.put(&hello.id, &hello.name, token);
        }
        hello.paired = true;
        Ok((c, hello))
    }

    /// One clock exchange. `mine`: this side's master (sent with the current estimate).
    pub fn ping(
        &mut self,
        est: Option<(i64, i64)>,
        mine: Option<&BeatPoint>,
    ) -> Result<(Exchange, Option<BeatPoint>), NetError> {
        let t1 = mono_ns();
        let mut p = JsonObject::new();
        p.set("t1", JsonValue::int(t1));
        if let Some((o, d)) = est {
            p.set("offset_ns", JsonValue::int(o));
            p.set("delay_ns", JsonValue::int(d));
        }
        if let Some(b) = mine {
            p.set("beat", b.to_json());
        }
        let r = self.call("clock", JsonValue::Object(p))?;
        let t4 = mono_ns();
        let n = |k: &str| r.get(k).and_then(JsonValue::as_f64).map(|v| v as i64);
        let ex = Exchange {
            t1,
            t2: n("t2").ok_or_else(|| NetError::new("app", "clock: no t2"))?,
            t3: n("t3").ok_or_else(|| NetError::new("app", "clock: no t3"))?,
            t4,
        };
        Ok((ex, r.get("beat").and_then(BeatPoint::from_json)))
    }
}

/// The managed link's state, for the UI.
#[derive(Clone, Debug, Default)]
pub struct LinkStatus {
    /// idle, connecting, connected, reconnecting, unpaired, refused.
    pub state: String,
    pub addr: Option<SocketAddr>,
    pub hello: Option<Hello>,
    pub error: Option<String>,
    /// host clock − this clock, ns; the round trip of the best exchange; their spread.
    pub offset_ns: Option<i64>,
    pub delay_ns: Option<i64>,
    pub spread_ns: Option<i64>,
    /// The host's master on this side's clock (smoothed).
    pub host_beat: Option<BeatPoint>,
    /// When the last clock answer came (this clock).
    pub clock_at: Option<i64>,
    pub log: Vec<String>,
}

type ClockFn = dyn Fn() -> Option<BeatPoint> + Send + Sync;

struct LinkShared {
    stop: AtomicBool,
    me: Me,
    store: Mutex<Store>,
    target: Mutex<Option<(SocketAddr, Option<String>)>>,
    conn: Mutex<Option<Conn>>,
    status: Mutex<LinkStatus>,
    est: Mutex<OffsetEstimator>,
    fit: Mutex<BeatFit>,
    mine: Box<ClockFn>,
    /// Bumped by `connect`: wakes the thread and resets the backoff.
    kick: AtomicBool,
}

/// A peer's managed connection to one host.
pub struct Link {
    shared: Arc<LinkShared>,
}

impl Link {
    /// `mine`: this side's master now (None: no engine).
    pub fn start(me: Me, store: Store, mine: Box<ClockFn>) -> Link {
        let shared = Arc::new(LinkShared {
            stop: AtomicBool::new(false),
            me,
            store: Mutex::new(store),
            target: Mutex::new(None),
            conn: Mutex::new(None),
            status: Mutex::new(LinkStatus {
                state: "idle".into(),
                ..Default::default()
            }),
            est: Mutex::new(OffsetEstimator::default()),
            fit: Mutex::new(BeatFit::default()),
            mine,
            kick: AtomicBool::new(false),
        });
        let sh = shared.clone();
        let _ = std::thread::Builder::new()
            .name("djnet-link".into())
            .spawn(move || run(sh));
        Link { shared }
    }

    /// Connect to `addr` (pairing with `code` if it is new); replaces any other host.
    pub fn connect(&self, addr: SocketAddr, code: Option<String>) {
        if let Ok(mut t) = self.shared.target.lock() {
            *t = Some((addr, code));
        }
        if let Ok(mut c) = self.shared.conn.lock() {
            *c = None;
        }
        self.reset_clock();
        self.set(|s| {
            s.state = "connecting".into();
            s.addr = Some(addr);
            s.error = None;
        });
        self.shared.kick.store(true, Ordering::SeqCst);
    }

    pub fn disconnect(&self) {
        if let Ok(mut t) = self.shared.target.lock() {
            *t = None;
        }
        if let Ok(mut c) = self.shared.conn.lock() {
            *c = None;
        }
        self.reset_clock();
        self.set(|s| {
            *s = LinkStatus {
                state: "idle".into(),
                log: std::mem::take(&mut s.log),
                ..Default::default()
            }
        });
    }

    fn reset_clock(&self) {
        if let Ok(mut e) = self.shared.est.lock() {
            e.clear();
        }
        if let Ok(mut f) = self.shared.fit.lock() {
            f.clear();
        }
    }

    fn set(&self, f: impl FnOnce(&mut LinkStatus)) {
        if let Ok(mut s) = self.shared.status.lock() {
            f(&mut s);
        }
    }

    pub fn status(&self) -> LinkStatus {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// A request on the open connection (`unavailable` while there is none).
    pub fn call(&self, method: &str, params: JsonValue) -> Result<JsonValue, NetError> {
        let mut g = self.shared.conn.lock().map_err(|_| NetError::unavailable("link closed"))?;
        let Some(c) = g.as_mut() else {
            let st = self.status();
            return Err(NetError::unavailable(match st.state.as_str() {
                "idle" => "not connected to a desktop".to_string(),
                s => format!("the desktop link is {s}{}", st.error.map(|e| format!(": {e}")).unwrap_or_default()),
            }));
        };
        let r = c.call(method, params);
        if let Err(e) = &r {
            if e.code == "unavailable" {
                *g = None;
                drop(g);
                self.set(|s| {
                    s.state = "reconnecting".into();
                    s.error = Some(e.message.clone());
                });
            }
        }
        r
    }

    /// The host's master beat at this side's time `t` (its own `mono_ns`).
    pub fn host_beat_at(&self, t: i64) -> Option<(f64, f64)> {
        let b = self.status().host_beat?;
        Some((b.beat_at(t), b.bpm))
    }

    /// This side's sides it has paired with: (id, name).
    pub fn paired(&self) -> Vec<(String, String)> {
        self.shared
            .store
            .lock()
            .map(|s| s.paired.iter().map(|p| (p.id.clone(), p.name.clone())).collect())
            .unwrap_or_default()
    }

    pub fn forget(&self, id: &str) -> usize {
        self.shared.store.lock().map(|mut s| s.forget(id)).unwrap_or(0)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

fn note(sh: &LinkShared, line: String) {
    if let Ok(mut s) = sh.status.lock() {
        s.log.push(line);
        let over = s.log.len().saturating_sub(30);
        s.log.drain(..over);
    }
}

const BACKOFF: [u64; 4] = [500, 1000, 2000, 4000];

fn run(sh: Arc<LinkShared>) {
    let mut fails = 0usize;
    let mut next_try = Instant::now();
    let mut next_ping = Instant::now();
    while !sh.stop.load(Ordering::SeqCst) {
        if sh.kick.swap(false, Ordering::SeqCst) {
            fails = 0;
            next_try = Instant::now();
        }
        let target = sh.target.lock().ok().and_then(|t| t.clone());
        let Some((addr, code)) = target else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let connected = sh.conn.lock().map(|c| c.is_some()).unwrap_or(false);
        if !connected {
            let blocked = sh.status.lock().map(|s| matches!(s.state.as_str(), "unpaired" | "refused")).unwrap_or(false);
            if blocked || Instant::now() < next_try {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            match Conn::open(addr, &sh.me, &sh.store, code.as_deref()) {
                Ok((c, hello)) => {
                    fails = 0;
                    note(&sh, format!("connected to {} ({addr}), djnet v{}", hello.name, hello.version));
                    if let Ok(mut t) = sh.target.lock() {
                        // the code was used: a reconnect authenticates by token
                        if let Some(t) = t.as_mut() {
                            t.1 = None;
                        }
                    }
                    if let Ok(mut g) = sh.conn.lock() {
                        *g = Some(c);
                    }
                    if let Ok(mut s) = sh.status.lock() {
                        s.state = "connected".into();
                        s.hello = Some(hello);
                        s.error = None;
                    }
                    // the burst: four exchanges back to back
                    for _ in 0..4 {
                        ping(&sh);
                    }
                    next_ping = Instant::now() + Duration::from_secs(1);
                }
                Err(e) => {
                    let state = match e.code {
                        "unpaired" | "bad_code" | "locked_out" => "unpaired",
                        "version" | "forbidden" => "refused",
                        _ => "reconnecting",
                    };
                    note(&sh, format!("✗ {addr}: {}", e.message));
                    if let Ok(mut s) = sh.status.lock() {
                        s.state = state.into();
                        s.error = Some(e.message);
                    }
                    next_try = Instant::now() + Duration::from_millis(BACKOFF[fails.min(BACKOFF.len() - 1)]);
                    fails += 1;
                }
            }
            continue;
        }
        if Instant::now() >= next_ping {
            ping(&sh);
            next_ping = Instant::now() + Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One exchange on the open connection; the estimate and the host's master updated.
fn ping(sh: &LinkShared) {
    let est = sh.est.lock().ok().and_then(|e| e.best());
    let mine = (sh.mine)();
    let r = {
        let Ok(mut g) = sh.conn.lock() else { return };
        let Some(c) = g.as_mut() else { return };
        let r = c.ping(est, mine.as_ref());
        if r.is_err() {
            *g = None;
        }
        r
    };
    match r {
        Ok((ex, host)) => {
            let (best, spread) = {
                let Ok(mut e) = sh.est.lock() else { return };
                e.push(ex);
                (e.best(), e.spread())
            };
            let Some((offset, delay)) = best else { return };
            let smoothed = host.and_then(|h| {
                let mut f = sh.fit.lock().ok()?;
                f.push(h.shifted(-offset));
                f.point()
            });
            if let Ok(mut s) = sh.status.lock() {
                s.offset_ns = Some(offset);
                s.delay_ns = Some(delay);
                s.spread_ns = Some(spread);
                s.clock_at = Some(ex.t4);
                if smoothed.is_some() {
                    s.host_beat = smoothed;
                }
            }
        }
        Err(e) => {
            note(sh, format!("✗ clock: {}", e.message));
            if let Ok(mut s) = sh.status.lock() {
                s.state = "reconnecting".into();
                s.error = Some(e.message);
            }
        }
    }
}
