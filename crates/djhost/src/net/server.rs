//! The djnet host (docs/NET-PROTOCOL.md §3-§6, §9): a TCP listener speaking the control
//! socket's framing. `hello`, `auth`, `pair` and `clock` are answered on the connection's own
//! thread; everything else, once paired and allowed, goes to the same handler the Unix socket
//! uses (the app's window-thread queue).

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djengine::json::{JsonObject, JsonValue};

use super::clock::{mono_ns, BeatPoint};
use super::proto::{self, NetError, MIN_VERSION, PROTO, VERSION};
use super::store::Store;
use crate::control_socket::Handler;

/// Who may connect (`net.trust`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// A 6-digit code shown on the host, once per peer.
    Code,
    /// Any peer that asks is paired (a closed network, a test rig).
    Open,
    /// Every peer is refused.
    Off,
}

impl Trust {
    pub fn parse(s: &str) -> Option<Trust> {
        Some(match s {
            "code" => Trust::Code,
            "open" => Trust::Open,
            "off" => Trust::Off,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Trust::Code => "code",
            Trust::Open => "open",
            Trust::Off => "off",
        }
    }
}

pub struct HostConfig {
    pub bind: SocketAddr,
    pub name: String,
    /// `djapp/<version>`.
    pub app: String,
    pub trust: Trust,
    pub store: Store,
    /// The modes offered (a subset of [`CAPS`]).
    pub caps: Vec<String>,
    /// The input strip a peer may touch (the stream's).
    pub strip: String,
}

/// A connected (or recently connected) peer, as the host sees it.
#[derive(Clone, Debug)]
pub struct PeerLive {
    pub addr: SocketAddr,
    pub id: String,
    pub name: String,
    pub app: String,
    pub version: u32,
    pub paired: bool,
    pub connected: bool,
    /// The peer's clock offset estimate (host − peer, ns) and its round trip.
    pub offset_ns: Option<i64>,
    pub delay_ns: Option<i64>,
    /// The peer's master, on the host's clock.
    pub beat: Option<BeatPoint>,
    pub requests: u64,
    pub last_seen: Instant,
}

struct Shared {
    stop: AtomicBool,
    cfg: Mutex<(String, Trust, Vec<String>, String, String)>,
    store: Mutex<Store>,
    code: Mutex<String>,
    /// Wrong codes in a row, and when pairing unlocks.
    fails: Mutex<(u32, Option<Instant>)>,
    clock: Mutex<Option<BeatPoint>>,
    peers: Mutex<Vec<PeerLive>>,
    log: Mutex<Vec<String>>,
    handler: Arc<Handler>,
    /// Open connections, shut down when the host stops.
    conns: Mutex<Vec<TcpStream>>,
}

impl Shared {
    fn note(&self, line: String) {
        if let Ok(mut l) = self.log.lock() {
            l.push(line);
            let over = l.len().saturating_sub(40);
            l.drain(..over);
        }
    }
}

/// The running host. Dropping it closes the listener; open connections end at their next line.
pub struct Host {
    pub addr: SocketAddr,
    pub id: String,
    shared: Arc<Shared>,
    accept: Option<std::thread::JoinHandle<()>>,
}

const LOCKOUT_AFTER: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(30);

impl Host {
    pub fn start(cfg: HostConfig, handler: Arc<Handler>) -> Result<Host, String> {
        let l = TcpListener::bind(cfg.bind).map_err(|e| format!("djnet on {}: {e}", cfg.bind))?;
        let addr = l.local_addr().map_err(|e| e.to_string())?;
        let _ = l.set_nonblocking(true);
        let id = cfg.store.id.clone();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            cfg: Mutex::new((cfg.name, cfg.trust, cfg.caps, cfg.strip, cfg.app)),
            store: Mutex::new(cfg.store),
            code: Mutex::new(proto::pair_code()),
            fails: Mutex::new((0, None)),
            clock: Mutex::new(None),
            peers: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
            handler,
            conns: Mutex::new(Vec::new()),
        });
        let sh = shared.clone();
        let accept = std::thread::Builder::new()
            .name("djnet-accept".into())
            .spawn(move || {
                while !sh.stop.load(Ordering::SeqCst) {
                    match l.accept() {
                        Ok((s, peer)) => {
                            if let (Ok(mut c), Ok(k)) = (sh.conns.lock(), s.try_clone()) {
                                c.retain(|x| x.peer_addr().is_ok());
                                c.push(k);
                            }
                            let sh = sh.clone();
                            let _ = std::thread::Builder::new()
                                .name("djnet-conn".into())
                                .spawn(move || serve(s, peer, sh));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25))
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Host {
            addr,
            id,
            shared,
            accept: Some(accept),
        })
    }

    /// The master's beat point now (the app publishes it every pass).
    pub fn publish_clock(&self, p: BeatPoint) {
        if let Ok(mut c) = self.shared.clock.lock() {
            *c = Some(p);
        }
    }

    pub fn pair_code(&self) -> String {
        self.shared.code.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// A new pairing code (and the lockout lifted).
    pub fn new_code(&self) -> String {
        let c = proto::pair_code();
        if let Ok(mut k) = self.shared.code.lock() {
            *k = c.clone();
        }
        if let Ok(mut f) = self.shared.fails.lock() {
            *f = (0, None);
        }
        c
    }

    pub fn trust(&self) -> Trust {
        self.shared.cfg.lock().map(|c| c.1).unwrap_or(Trust::Off)
    }
    pub fn set_trust(&self, t: Trust) {
        if let Ok(mut c) = self.shared.cfg.lock() {
            c.1 = t;
        }
        self.shared.note(format!("trust {}", t.name()));
    }
    pub fn name(&self) -> String {
        self.shared.cfg.lock().map(|c| c.0.clone()).unwrap_or_default()
    }

    /// Peers seen since the host started, connected first.
    pub fn peers(&self) -> Vec<PeerLive> {
        let mut p = self.shared.peers.lock().map(|p| p.clone()).unwrap_or_default();
        p.sort_by_key(|x| (!x.connected, std::cmp::Reverse(x.last_seen)));
        p
    }

    /// The paired sides on disk: (id, name).
    pub fn paired(&self) -> Vec<(String, String)> {
        self.shared
            .store
            .lock()
            .map(|s| s.paired.iter().map(|p| (p.id.clone(), p.name.clone())).collect())
            .unwrap_or_default()
    }

    pub fn forget(&self, id: &str) -> usize {
        let n = self.shared.store.lock().map(|mut s| s.forget(id)).unwrap_or(0);
        self.shared.note(format!("forgot {id} ({n})"));
        n
    }

    pub fn log(&self) -> Vec<String> {
        self.shared.log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// The best-placed connected peer's master on this host's clock (the newest heard).
    pub fn peer_beat(&self) -> Option<(String, BeatPoint)> {
        self.peers()
            .into_iter()
            .filter(|p| p.connected)
            .find_map(|p| p.beat.map(|b| (p.name, b)))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Ok(mut c) = self.shared.conns.lock() {
            for s in c.drain(..) {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
        // the listener closes with its thread
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

/// One connection's state.
struct Session {
    hello: bool,
    version: u32,
    id: String,
    name: String,
    paired: bool,
    nonce: String,
}

fn serve(stream: TcpStream, addr: SocketAddr, sh: Arc<Shared>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_nonblocking(false);
    // a peer that goes quiet is let go: it pings every second
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(mut out) = stream.try_clone() else { return };
    let mut s = Session {
        hello: false,
        version: 0,
        id: String::new(),
        name: String::new(),
        paired: false,
        nonce: proto::random_hex(16),
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if sh.stop.load(Ordering::SeqCst) {
            break;
        }
        let t2 = mono_ns();
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let mut is_clock = false;
        let reply = match JsonValue::parse(text) {
            Err(_) => proto::reply_err(&JsonValue::Null, &NetError::new("app", "not JSON")),
            Ok(req) => {
                let id = req.get("id").cloned().unwrap_or(JsonValue::Null);
                let method = req.get("method").and_then(JsonValue::as_str).unwrap_or("");
                is_clock = method == "clock";
                let params = req
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| JsonValue::Object(JsonObject::new()));
                match dispatch(&mut s, addr, &sh, method, &params, t2) {
                    Ok(v) => proto::reply_ok(&id, v),
                    Err(e) => proto::reply_err(&id, &e),
                }
            }
        };
        // `clock`'s t3 is stamped as late as possible: patch it into the reply here
        let reply = if is_clock {
            reply.replacen("\"t3\":-1", &format!("\"t3\":{}", mono_ns()), 1)
        } else {
            reply
        };
        if out.write_all(reply.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
            break;
        }
        touch(&sh, addr, |p| p.requests += 1);
    }
    touch(&sh, addr, |p| p.connected = false);
    if s.hello {
        sh.note(format!("{} ({addr}) left", s.name));
    }
}

/// Update the live record of `addr`'s current session.
fn touch(sh: &Shared, addr: SocketAddr, f: impl FnOnce(&mut PeerLive)) {
    if let Ok(mut peers) = sh.peers.lock() {
        if let Some(p) = peers.iter_mut().find(|p| p.addr == addr) {
            p.last_seen = Instant::now();
            f(p);
        }
    }
}

fn s_of<'a>(p: &'a JsonValue, k: &str) -> &'a str {
    p.get(k).and_then(JsonValue::as_str).unwrap_or("")
}

fn dispatch(
    s: &mut Session,
    addr: SocketAddr,
    sh: &Shared,
    method: &str,
    params: &JsonValue,
    t2: i64,
) -> Result<JsonValue, NetError> {
    let (name, trust, caps, strip, app) = sh.cfg.lock().map(|c| c.clone()).map_err(|_| NetError::unavailable("host stopped"))?;
    if method == "hello" {
        if s_of(params, "proto") != PROTO {
            return Err(NetError::new("version", "not a djnet peer"));
        }
        let n = |k: &str, d: u32| params.get(k).and_then(JsonValue::as_f64).map_or(d, |v| v as u32);
        s.version = proto::negotiate((VERSION, MIN_VERSION), (n("version", 1), n("min_version", 1)))?;
        s.hello = true;
        s.id = s_of(params, "id").to_string();
        s.name = s_of(params, "name").to_string();
        if trust == Trust::Off {
            return Err(NetError::new("forbidden", format!("{name} takes no network peers now (net.trust off)")));
        }
        s.paired = trust == Trust::Open;
        if let Ok(mut peers) = sh.peers.lock() {
            peers.retain(|p| p.addr != addr && (p.connected || p.id != s.id));
            peers.push(PeerLive {
                addr,
                id: s.id.clone(),
                name: s.name.clone(),
                app: s_of(params, "app").to_string(),
                version: s.version,
                paired: s.paired,
                connected: true,
                offset_ns: None,
                delay_ns: None,
                beat: None,
                requests: 0,
                last_seen: Instant::now(),
            });
        }
        sh.note(format!("{} ({addr}) hello v{}", s.name, s.version));
        let store_id = sh.store.lock().map(|st| st.id.clone()).unwrap_or_default();
        return Ok(JsonValue::Object(JsonObject::from_pairs([
            ("proto", PROTO.into()),
            ("version", JsonValue::int(s.version as i64)),
            ("id", JsonValue::str(store_id)),
            ("name", JsonValue::str(name)),
            ("app", JsonValue::str(app)),
            ("caps", JsonValue::Array(caps.iter().map(|c| JsonValue::str(c.clone())).collect())),
            ("paired", s.paired.into()),
            ("nonce", JsonValue::str(s.nonce.clone())),
            ("trust", trust.name().into()),
            ("strip", JsonValue::str(strip)),
        ])));
    }
    if !s.hello {
        return Err(NetError::new("unpaired", "say hello first"));
    }
    match method {
        "auth" => {
            let token = sh.store.lock().ok().and_then(|st| st.token_of(&s.id).map(String::from));
            let Some(token) = token else {
                return Err(NetError::new("unpaired", "not paired with this desktop: pair with its code"));
            };
            if !proto::same(&proto::proof(&token, &s.nonce), s_of(params, "proof")) {
                sh.note(format!("{} ({addr}) auth refused", s.name));
                return Err(NetError::new("unpaired", "the pairing token doesn't match: pair again"));
            }
            s.paired = true;
            s.nonce = proto::random_hex(16); // one proof per nonce
            touch(sh, addr, |p| p.paired = true);
            sh.note(format!("{} ({addr}) paired (token)", s.name));
            Ok(JsonValue::Object(JsonObject::from_pairs([("paired", true.into())])))
        }
        "pair" => {
            if let Ok(mut f) = sh.fails.lock() {
                if let Some(until) = f.1 {
                    if Instant::now() < until {
                        return Err(NetError::new("locked_out", "too many wrong codes: wait 30 s"));
                    }
                    *f = (0, None);
                }
            }
            let code = sh.code.lock().map(|c| c.clone()).unwrap_or_default();
            let given: String = s_of(params, "code").chars().filter(char::is_ascii_digit).collect();
            if trust != Trust::Open && !proto::same(&given, &code) {
                if let Ok(mut f) = sh.fails.lock() {
                    f.0 += 1;
                    if f.0 >= LOCKOUT_AFTER {
                        f.1 = Some(Instant::now() + LOCKOUT);
                    }
                }
                sh.note(format!("{} ({addr}) wrong code", s.name));
                return Err(NetError::new("bad_code", "wrong pairing code"));
            }
            if s.id.len() < 8 {
                return Err(NetError::new("unpaired", "the peer has no id"));
            }
            let token = proto::random_hex(32);
            if let Ok(mut st) = sh.store.lock() {
                st.put(&s.id, &s.name, &token);
            }
            if let Ok(mut f) = sh.fails.lock() {
                *f = (0, None);
            }
            s.paired = true;
            touch(sh, addr, |p| p.paired = true);
            sh.note(format!("{} ({addr}) paired (code)", s.name));
            Ok(JsonValue::Object(JsonObject::from_pairs([
                ("paired", true.into()),
                ("token", JsonValue::str(token)),
            ])))
        }
        _ if !s.paired => Err(NetError::new("unpaired", "pair with the desktop's code first")),
        "clock" => {
            let n = |k: &str| params.get(k).and_then(JsonValue::as_f64).map(|v| v as i64);
            let offset = n("offset_ns");
            let delay = n("delay_ns");
            let beat = params
                .get("beat")
                .and_then(BeatPoint::from_json)
                .zip(offset)
                .map(|(b, o)| b.shifted(o));
            touch(sh, addr, |p| {
                p.offset_ns = offset;
                p.delay_ns = delay;
                if beat.is_some() {
                    p.beat = beat;
                }
            });
            let mine = sh.clock.lock().ok().and_then(|c| c.clone());
            Ok(JsonValue::Object(JsonObject::from_pairs([
                ("t1", JsonValue::int(n("t1").unwrap_or(0))),
                ("t2", JsonValue::int(t2)),
                // stamped as the line goes out (see `serve`)
                ("t3", JsonValue::int(-1)),
                ("beat", mine.map_or(JsonValue::Null, |b| b.to_json())),
            ])))
        }
        _ => {
            proto::allowed(method, params, &strip)?;
            // the app sees who asked: `_via` (it answers a peer less than the desk) and `_peer`
            let mut o = params.as_object().cloned().unwrap_or_default();
            o.set("_via", "djnet".into());
            o.set("_peer", JsonValue::str(s.id.clone()));
            o.set("_peer_name", JsonValue::str(s.name.clone()));
            o.set("_peer_ip", JsonValue::str(addr.ip().to_string()));
            (sh.handler)(method, &JsonValue::Object(o)).map_err(app_error)
        }
    }
}

/// The app's error text as a djnet error: `[code] text` names one of the codes (§3).
fn app_error(e: String) -> NetError {
    const CODES: [&str; 4] = ["unknown_cid", "cid_mismatch", "busy", "forbidden"];
    if let Some(rest) = e.strip_prefix('[') {
        if let Some((code, text)) = rest.split_once("] ") {
            if let Some(c) = CODES.iter().find(|c| **c == code) {
                return NetError::new(c, text.to_string());
            }
        }
    }
    NetError::new("app", e)
}
