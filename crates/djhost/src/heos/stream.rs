//! The master mix over HTTP, for network players (HEOS `browse/play_stream`): `GET
//! /master.flac` or `/master.wav` answers an endless stream of what the master bus plays from
//! the moment of the request, read from the master's capture tap
//! ([`BusTaps::master`](crate::deck::capture::BusTaps)), which the render thread already writes.
//! Nothing is added to the audio callback: each listener's thread copies new frames out of
//! the tap every [`POLL`] and writes them to its socket.
//!
//! A player buffers several seconds before it starts (HEOS: typically 2-5 s, by firmware and
//! network), so what the zones play is that far behind the booth. They are for the room, not
//! for cueing: the headphones stay the monitor.
//!
//! The server binds every interface (the players are elsewhere on the LAN) on the port asked
//! for; it serves only these two read-only paths and an index, and closes with [`StreamServer`].

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::encode::{wav_frames, wav_header, FlacStream, Format};
use crate::deck::capture::CaptureTap;

/// How often a listener's thread copies new frames.
pub const POLL: Duration = Duration::from_millis(20);
/// A listener that takes no data for this long is let go.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// The default port.
pub const DEFAULT_PORT: u16 = 8790;

/// What the server has done.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamStats {
    pub listeners: usize,
    pub served: u64,
    pub bytes: u64,
    /// The last few clients: (address, path).
    pub recent: Vec<(String, String)>,
}

/// The sender's master beat point for `/master.djl` (docs/NET-PROTOCOL.md §8.3), published by
/// whoever services the engine.
#[cfg(feature = "network")]
pub type ClockCell = Arc<std::sync::Mutex<Option<crate::net::BeatPoint>>>;

#[cfg(feature = "network")]
type ClockArg = Option<ClockCell>;
#[cfg(not(feature = "network"))]
type ClockArg = Option<()>;

struct Shared {
    stop: AtomicBool,
    #[cfg(feature = "network")]
    clock: Option<ClockCell>,
    listeners: AtomicUsize,
    served: AtomicU64,
    bytes: AtomicU64,
    recent: std::sync::Mutex<Vec<(String, String)>>,
}

pub struct StreamServer {
    pub addr: SocketAddr,
    pub sample_rate: u32,
    shared: Arc<Shared>,
}

/// Where the frames come from: the master's tap (anything with `span` / `copy` in frames).
pub trait PcmSource: Send + Sync + 'static {
    fn span(&self) -> Option<(i64, i64)>;
    fn copy(&self, from: i64, to: i64, l: &mut Vec<f32>, r: &mut Vec<f32>) -> i64;
}

impl PcmSource for CaptureTap {
    fn span(&self) -> Option<(i64, i64)> {
        CaptureTap::span(self)
    }
    fn copy(&self, from: i64, to: i64, l: &mut Vec<f32>, r: &mut Vec<f32>) -> i64 {
        CaptureTap::copy(self, from, to, l, r)
    }
}

/// The master tap out of the engine's bus taps, as a source.
pub struct MasterTap(pub Arc<crate::deck::capture::BusTaps>);

impl PcmSource for MasterTap {
    fn span(&self) -> Option<(i64, i64)> {
        self.0.master.span()
    }
    fn copy(&self, from: i64, to: i64, l: &mut Vec<f32>, r: &mut Vec<f32>) -> i64 {
        self.0.master.copy(from, to, l, r)
    }
}

impl StreamServer {
    /// Listen on `bind` (port 0: any free one) and serve `source` at `sample_rate`.
    pub fn start(
        bind: SocketAddr,
        source: Arc<dyn PcmSource>,
        sample_rate: u32,
    ) -> Result<StreamServer, String> {
        Self::start_inner(bind, source, sample_rate, None)
    }

    /// [`start`](Self::start), also serving `/master.djl`: the master with `clock`'s beat grid
    /// in every chunk (docs/NET-PROTOCOL.md §8.3).
    #[cfg(feature = "network")]
    pub fn start_with_clock(
        bind: SocketAddr,
        source: Arc<dyn PcmSource>,
        sample_rate: u32,
        clock: ClockCell,
    ) -> Result<StreamServer, String> {
        Self::start_inner(bind, source, sample_rate, Some(clock))
    }

    fn start_inner(
        bind: SocketAddr,
        source: Arc<dyn PcmSource>,
        sample_rate: u32,
        clock: ClockArg,
    ) -> Result<StreamServer, String> {
        #[cfg(not(feature = "network"))]
        let _ = clock;
        let l = TcpListener::bind(bind).map_err(|e| format!("stream server on {bind}: {e}"))?;
        let addr = l.local_addr().map_err(|e| e.to_string())?;
        let _ = l.set_nonblocking(true);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            #[cfg(feature = "network")]
            clock,
            listeners: AtomicUsize::new(0),
            served: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            recent: std::sync::Mutex::new(Vec::new()),
        });
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("master-stream".into())
            .spawn(move || {
                while !sh.stop.load(Ordering::SeqCst) {
                    match l.accept() {
                        Ok((s, peer)) => {
                            let (sh, src) = (sh.clone(), source.clone());
                            let _ = std::thread::Builder::new()
                                .name("master-stream-client".into())
                                .spawn(move || serve(s, peer, src, sample_rate, sh));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(StreamServer {
            addr,
            sample_rate,
            shared,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The URL a player at `host` fetches `format` from.
    pub fn url(&self, host: IpAddr, format: Format) -> String {
        let h = match host {
            IpAddr::V6(v6) => format!("[{v6}]"),
            v4 => v4.to_string(),
        };
        format!("http://{h}:{}/master.{}", self.port(), format.ext())
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            listeners: self.shared.listeners.load(Ordering::Relaxed),
            served: self.shared.served.load(Ordering::Relaxed),
            bytes: self.shared.bytes.load(Ordering::Relaxed),
            recent: self
                .shared
                .recent
                .lock()
                .map(|r| r.clone())
                .unwrap_or_default(),
        }
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

/// The request line's method and path (headers read and dropped).
fn read_request(s: &TcpStream) -> Option<(String, String)> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut first = String::new();
    r.read_line(&mut first).ok()?;
    let mut parts = first.split_whitespace();
    let (m, p) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut line = String::new();
    for _ in 0..100 {
        line.clear();
        if r.read_line(&mut line).ok()? == 0 || line.trim().is_empty() {
            break;
        }
    }
    Some((m, p))
}

fn serve(
    mut s: TcpStream,
    peer: SocketAddr,
    src: Arc<dyn PcmSource>,
    sample_rate: u32,
    sh: Arc<Shared>,
) {
    let _ = s.set_nonblocking(false);
    let Some((method, path)) = read_request(&s) else {
        return;
    };
    let path_only = path.split('?').next().unwrap_or("").to_string();
    if let Ok(mut r) = sh.recent.lock() {
        r.push((peer.to_string(), format!("{method} {path_only}")));
        let over = r.len().saturating_sub(8);
        r.drain(..over);
    }
    #[cfg(feature = "network")]
    if path_only == "/master.djl" && (method == "GET" || method == "HEAD") {
        return serve_djl(s, src, sample_rate, sh, method == "HEAD");
    }
    let format = match path_only.as_str() {
        "/master.flac" | "/master" => Some(Format::Flac),
        "/master.wav" => Some(Format::Wav),
        _ => None,
    };
    let Some(format) = format else {
        let (status, body) = if path_only == "/" {
            ("200 OK", "djapp master stream: /master.flac, /master.wav\n")
        } else {
            ("404 Not Found", "not here: /master.flac or /master.wav\n")
        };
        let _ = write!(
            s,
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        return;
    };
    if method != "GET" && method != "HEAD" {
        let _ = write!(s, "HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-cache, no-store\r\nPragma: no-cache\r\nAccept-Ranges: none\r\nConnection: close\r\ntransferMode.dlna.org: Streaming\r\nServer: djapp\r\nicy-name: djapp master\r\n\r\n",
        format.mime()
    );
    if s.write_all(head.as_bytes()).is_err() || method == "HEAD" {
        return;
    }
    let _ = s.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = s.set_nodelay(true);
    sh.listeners.fetch_add(1, Ordering::SeqCst);
    sh.served.fetch_add(1, Ordering::SeqCst);
    let mut flac = FlacStream::new(sample_rate);
    let mut out = match format {
        Format::Wav => wav_header(sample_rate),
        Format::Flac => flac.header(),
    };
    // from now: the frame after the newest the tap holds (it may not have started yet)
    let mut cursor: Option<i64> = src.span().map(|(_, end)| end);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let mut last = Instant::now();
    while !sh.stop.load(Ordering::SeqCst) {
        if let Some((oldest, end)) = src.span() {
            let from = cursor.unwrap_or(end).max(oldest);
            l.clear();
            r.clear();
            cursor = Some(src.copy(from, end, &mut l, &mut r));
            match format {
                Format::Wav => wav_frames(&l, &r, &mut out),
                Format::Flac => flac.push(&l, &r, &mut out),
            }
        }
        if !out.is_empty() {
            if s.write_all(&out).is_err() {
                break;
            }
            sh.bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
            out.clear();
            last = Instant::now();
        } else if last.elapsed() > Duration::from_secs(60) {
            // nothing rendered for a minute: the engine is gone
            break;
        }
        std::thread::sleep(POLL);
    }
    sh.listeners.fetch_sub(1, Ordering::SeqCst);
}

/// `/master.djl`: the master from now on, in chunks dated on the engine's frames, with the
/// beat grid of the clock cell (docs/NET-PROTOCOL.md §8.3).
#[cfg(feature = "network")]
fn serve_djl(mut s: TcpStream, src: Arc<dyn PcmSource>, sample_rate: u32, sh: Arc<Shared>, head_only: bool) {
    use crate::net::{clock::mono_ns, djl};
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-cache, no-store\r\nConnection: close\r\nServer: djnet\r\n\r\n",
        djl::MIME
    );
    if s.write_all(head.as_bytes()).is_err() || head_only {
        return;
    }
    let _ = s.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = s.set_nodelay(true);
    sh.listeners.fetch_add(1, Ordering::SeqCst);
    sh.served.fetch_add(1, Ordering::SeqCst);
    let mut cursor: Option<i64> = src.span().map(|(_, end)| end);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let mut out = Vec::new();
    let mut seq = 0u32;
    let mut last = Instant::now();
    while !sh.stop.load(Ordering::SeqCst) {
        if let Some((oldest, end)) = src.span() {
            let mut from = cursor.unwrap_or(end).max(oldest);
            let clock = sh.clock.as_ref().and_then(|c| c.lock().ok().and_then(|p| p.clone()));
            while from < end {
                let to = (from + djl::MAX_FRAMES as i64).min(end);
                l.clear();
                r.clear();
                let next = src.copy(from, to, &mut l, &mut r);
                let n = l.len().min(r.len());
                if n == 0 {
                    break;
                }
                let first = next - n as i64;
                let (flags, beat, bpm, t) = match &clock {
                    Some(p) => (
                        djl::FLAG_CLOCK | if p.playing { djl::FLAG_PLAYING } else { 0 },
                        p.beat_at_frame(first),
                        p.bpm,
                        // when the chunk's end was rendered, on the clock's dated line
                        p.t + ((next - p.frame) as f64 * 1e9 / p.rate.max(1.0)) as i64,
                    ),
                    None => (0, 0.0, 0.0, mono_ns()),
                };
                let c = djl::Chunk { seq, frame: first, n: n as u32, rate: sample_rate, t, flags, beat, bpm, channels: 2 };
                seq = seq.wrapping_add(1);
                djl::encode(&c, &l[..n], &r[..n], &mut out);
                from = next;
            }
            cursor = Some(from);
        }
        if !out.is_empty() {
            if s.write_all(&out).is_err() {
                break;
            }
            sh.bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
            out.clear();
            last = Instant::now();
        } else if last.elapsed() > Duration::from_secs(60) {
            break;
        }
        std::thread::sleep(POLL);
    }
    sh.listeners.fetch_sub(1, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn a_listener_gets_the_header_then_what_the_tap_plays_after_it_asked() {
        let tap = Arc::new(CaptureTap::new());
        tap.write(0, &[0.9; 256], &[0.9; 256]); // before the request: not sent
        let srv = StreamServer::start("127.0.0.1:0".parse().unwrap(), tap.clone(), 48_000).unwrap();
        let mut c = TcpStream::connect(srv.addr).unwrap();
        c.write_all(b"GET /master.wav HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        // wait for the server to take the request before writing what it should send
        let t0 = Instant::now();
        while srv.stats().listeners == 0 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(5));
        }
        tap.write(256, &[0.5; 100], &[-0.5; 100]);
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        let want_body = 44 + 100 * 4;
        while got.len() < 300 + want_body {
            let n = c.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            if let Some(h) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                if got.len() >= h + 4 + want_body {
                    break;
                }
            }
        }
        let text = String::from_utf8_lossy(&got);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
        assert!(text.contains("Content-Type: audio/wav"));
        let h = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let body = &got[h..];
        assert_eq!(&body[..4], b"RIFF");
        assert_eq!(body.len(), want_body, "only the frames after the request");
        let first = i16::from_le_bytes([body[44], body[45]]);
        assert_eq!(first, super::super::encode::to_i16(0.5));
        assert_eq!(srv.stats().served, 1);
    }

    #[test]
    fn other_paths_are_refused() {
        let tap = Arc::new(CaptureTap::new());
        let srv = StreamServer::start("127.0.0.1:0".parse().unwrap(), tap, 44_100).unwrap();
        let mut c = TcpStream::connect(srv.addr).unwrap();
        c.write_all(b"GET /etc/passwd HTTP/1.1\r\n\r\n").unwrap();
        let mut s = String::new();
        c.read_to_string(&mut s).unwrap();
        assert!(s.starts_with("HTTP/1.1 404"), "{s}");
        assert_eq!(
            srv.url("10.0.0.2".parse().unwrap(), Format::Flac),
            format!("http://10.0.0.2:{}/master.flac", srv.port())
        );
    }
}

/// The master bus as a stream source (what [`StreamServer`] serves). A trait because the
/// engine lives in djrust: bring it into scope to call `engine.master_stream_source()`.
pub trait MasterStreamSource {
    fn master_stream_source(&self) -> Arc<dyn PcmSource>;
}

impl MasterStreamSource for crate::deck::Engine {
    fn master_stream_source(&self) -> Arc<dyn PcmSource> {
        Arc::new(MasterTap(self.bus_taps()))
    }
}
