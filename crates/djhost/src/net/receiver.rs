//! The receiver (docs/NET-PROTOCOL.md §8.4-§8.5): a peer's `/master.djl` into an input strip's
//! ring.
//!
//! ```text
//!  socket ─▶ parse ─▶ jitter buffer (by sender frame) ─▶ varispeed read ─▶ ring ─▶ strip
//!                         ▲ newest frame                   │ read position (a sender frame)
//!                         └──── delay = newest − read ─────┘ → the beat of what plays now
//! ```
//!
//! Two threads. The **network** thread fetches, reconnects and appends chunks. The **feeder**
//! keeps the strip's ring at its cushion, so it runs on this side's output clock, never the
//! sender's. It reads the buffer at `rate_sender / rate_here × (1 + c)`, where `c` (±0.2 %, a
//! few cents at most) holds the buffer at its target, absorbing clock drift and a 44.1 / 48 kHz
//! mismatch. A target moved by more than 20 ms (Sync, a nudge, a reconnect) jumps the read
//! position with a 10 ms crossfade. Running dry fades out, refilling fades in: a gap is heard as
//! a gap, never a click.
//!
//! Every sample keeps its sender frame, and every chunk dates its frames on the sender's beat
//! grid, so the beat of the sample the strip plays now is known whatever the network did
//! ([`Receiver::beat`]).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::clock::{mono_ns, FrameClock};
use super::djl::{self, Chunk};
use crate::deck::input::InputMirror;

/// The strip's ring the receiver writes (kept by the app between receivers).
pub type Ring = rtrb::Producer<f32>;

/// The receiver's state, for the window and `net status`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RxStats {
    /// connecting, buffering, playing, reconnecting, lost, stopped.
    pub state: String,
    pub url: String,
    pub sender_rate: u32,
    pub chunks: u64,
    pub bytes: u64,
    pub reconnects: u32,
    /// Times the buffer ran dry while playing.
    pub underruns: u32,
    /// Times the stream jumped (sender restarted, a long gap) and the buffer started again.
    pub resets: u32,
    /// Read-position jumps (Sync, nudges, a target moved more than 20 ms).
    pub jumps: u32,
    /// The jitter buffer now and its target, ms.
    pub buffer_ms: f64,
    pub target_ms: f64,
    /// The varispeed's correction, parts per million.
    pub correction_ppm: f64,
    /// Network: the sender's send to here, ms (needs the sender's clock offset).
    pub transit_ms: Option<f64>,
    /// The sender's render to this side's render of the same sample, ms (needs the offset);
    /// add this side's output latency for the acoustic figure.
    pub e2e_ms: Option<f64>,
    pub bpm: Option<f64>,
    pub playing: bool,
    pub last_error: Option<String>,
    /// Seconds since the last chunk.
    pub idle_s: f64,
    pub latency_ms: f64,
    pub trim_ms: f64,
    /// The phase offset Sync / nudges set, ms.
    pub offset_ms: f64,
    /// How far the buffer still is from its target, smoothed (+: more buffered), ms: the
    /// varispeed is still removing this, so the phase will move by it.
    pub pending_ms: f64,
}

/// The beat of the sample the strip plays, dated on this side's render clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamBeat {
    pub beat: f64,
    pub bpm: f64,
    /// This process's `mono_ns` when that sample was rendered here.
    pub t: i64,
    /// The sender is playing (its beat means something).
    pub playing: bool,
}

impl StreamBeat {
    pub fn beat_at(&self, t: i64) -> f64 {
        self.beat + (t - self.t) as f64 * self.bpm / 60e9
    }
}

/// The part both threads touch.
struct State {
    // the jitter buffer: mono samples from sender frame `start`
    buf: VecDeque<f32>,
    start: i64,
    rate: f64,
    clocks: VecDeque<Chunk>,
    have: bool,
    last_chunk_at: i64,
    // playback
    read: f64,
    playing: bool,
    gain: f32,
    xf_from: f64,
    xf_left: usize,
    err_ema: f64,
    correction: f64,
    last_target: f64,
    // control
    latency_ms: f64,
    trim_ms: f64,
    offset_frames: f64,
    // the strip's position → the read position, to find what plays now
    out_total: u64,
    marks: VecDeque<(u64, f64)>,
    frames: FrameClock,
    beat: Option<StreamBeat>,
    peer_offset: Option<i64>,
    transit: VecDeque<i64>,
    stats: RxStats,
}

impl State {
    fn end(&self) -> i64 {
        self.start + self.buf.len() as i64
    }

    fn target_frames(&self) -> f64 {
        ((self.latency_ms + self.trim_ms) / 1000.0 * self.rate + self.offset_frames).max(0.01 * self.rate)
    }

    fn sample(&self, pos: f64) -> f32 {
        let i = pos.floor();
        let k = (i as i64 - self.start) as isize;
        if k < 0 || k as usize + 1 >= self.buf.len() {
            return 0.0;
        }
        let f = (pos - i) as f32;
        let (a, b) = (self.buf[k as usize], self.buf[k as usize + 1]);
        a + (b - a) * f
    }

    fn push_chunk(&mut self, c: Chunk, body: &[u8], now: i64) {
        let rate = c.rate as f64;
        let end = self.end();
        let restart = !self.have
            || (rate - self.rate).abs() > 0.5
            || c.frame < end
            || c.frame - end > (2.0 * rate) as i64;
        // a gap after the buffer ran dry (a reconnect): start the buffer again at the new
        // frame, so the fade-in lands on the music, not on the silence a fill would put first
        let resume = !restart && !self.playing && c.frame > end;
        if restart || resume {
            if self.have && restart {
                self.stats.resets += 1;
            }
            self.buf.clear();
            self.clocks.clear();
            self.start = c.frame;
            self.rate = rate;
            self.read = c.frame as f64;
            self.playing = false;
            self.have = true;
        } else if c.frame > end {
            // a gap (a reconnect starts at the sender's "now"): silence keeps the timeline
            self.buf.extend(std::iter::repeat_n(0.0, (c.frame - end) as usize));
        }
        djl::decode_mono(&c, body, &mut self.buf);
        self.clocks.push_back(c);
        while self.clocks.len() > 256 {
            self.clocks.pop_front();
        }
        // keep two seconds behind the read position (Sync may move it back), ten at most
        let keep_from = (self.read as i64 - (2.0 * rate) as i64).max(self.end() - (10.0 * rate) as i64);
        while self.start < keep_from && !self.buf.is_empty() {
            self.buf.pop_front();
            self.start += 1;
        }
        self.last_chunk_at = now;
        self.stats.chunks += 1;
        self.stats.bytes += (djl::HEADER + body.len()) as u64;
        self.stats.sender_rate = c.rate;
        if let Some(off) = self.peer_offset {
            self.transit.push_back(now - (c.t + off));
            while self.transit.len() > 50 {
                self.transit.pop_front();
            }
        }
    }

    /// The chunk that dates sender frame `f`.
    fn clock_for(&self, f: i64) -> Option<&Chunk> {
        self.clocks.iter().rev().find(|c| c.frame <= f).or(self.clocks.front())
    }

    /// `need` samples at `out_rate` into `out`.
    fn render(&mut self, need: usize, out_rate: f64, out: &mut Vec<f32>) {
        out.clear();
        if !self.have {
            out.resize(need, 0.0);
            self.out_total += need as u64;
            return;
        }
        let rate = self.rate;
        let fade = (0.010 * out_rate) as usize;
        let newest = self.end() as f64;
        let target = self.target_frames();
        // (re)start once the buffer holds the target ahead of where it stopped
        if !self.playing && newest - self.read.max(self.start as f64) >= target {
            self.read = newest - target;
            self.playing = true;
            self.gain = 0.0;
            self.err_ema = 0.0;
            self.last_target = target;
        }
        if self.playing {
            let err = (newest - self.read) - target;
            // jump when the target moved (Sync, a nudge, the latency) or far too much is
            // buffered (a reconnect's gap); running low only slows the read, then runs dry
            let moved = (target - self.last_target).abs() > 0.020 * rate;
            // chunks land in bursts (the sender's 20 ms poll, the network), so the buffer swings
            // by tens of ms: only a gap's worth too much (a reconnect) is a jump
            let flooded = err > (0.25 * rate).max(target);
            if moved || flooded {
                // a big move: jump, crossfading from where it was
                self.xf_from = self.read;
                self.xf_left = fade;
                self.read = newest - target;
                self.err_ema = 0.0;
                self.stats.jumps += 1;
            } else {
                // smooth over ~1 s (this runs every ~2 ms)
                self.err_ema += (err - self.err_ema) * 0.004;
                self.correction = (self.err_ema / rate * 0.1).clamp(-0.002, 0.002);
            }
        }
        self.last_target = target;
        let step = rate / out_rate * (1.0 + self.correction);
        for _ in 0..need {
            let mut s = 0f32;
            if self.playing {
                let left = newest - self.read;
                if left < 2.0 {
                    self.playing = false;
                    self.stats.underruns += 1;
                } else {
                    s = self.sample(self.read);
                    if self.xf_left > 0 {
                        let w = self.xf_left as f32 / fade.max(1) as f32;
                        s = s * (1.0 - w) + self.sample(self.xf_from) * w;
                        self.xf_from += step;
                        self.xf_left -= 1;
                    }
                    // fade out ahead of running dry, back in once there is room
                    let want = if left > 2.0 * fade as f64 * step { 1.0 } else { 0.0 };
                    let d = 1.0 / fade.max(1) as f32;
                    self.gain = if want > self.gain { (self.gain + d).min(1.0) } else { (self.gain - d).max(0.0) };
                    self.read += step;
                }
            }
            out.push(s * self.gain);
        }
        self.out_total += need as u64;
        self.marks.push_back((self.out_total, self.read));
        while self.marks.len() > 512 {
            self.marks.pop_front();
        }
    }

    /// The read position when the strip had taken `played` samples.
    fn read_at(&self, played: u64) -> Option<f64> {
        let mut prev: Option<(u64, f64)> = None;
        for &(n, r) in &self.marks {
            if n >= played {
                return Some(match prev {
                    Some((n0, r0)) if n > n0 => r0 + (r - r0) * (played - n0) as f64 / (n - n0) as f64,
                    _ => r,
                });
            }
            prev = Some((n, r));
        }
        prev.map(|(_, r)| r)
    }
}

struct Shared {
    stop: AtomicBool,
    url: Mutex<String>,
    /// Bumped to make the network thread reconnect (a new URL).
    reconnect: AtomicBool,
    out_rate: f64,
    st: Mutex<State>,
}

/// A running receiver. [`Receiver::stop`] hands the strip's ring back.
pub struct Receiver {
    sh: Arc<Shared>,
    net: Option<JoinHandle<()>>,
    feed: Option<JoinHandle<rtrb::Producer<f32>>>,
}

impl Receiver {
    /// Fetch `url` into the ring `ring` of the strip `mirror` describes, at this side's
    /// `out_rate`, buffering `latency_ms`.
    pub fn start(
        url: &str,
        ring: rtrb::Producer<f32>,
        mirror: Arc<InputMirror>,
        out_rate: f64,
        latency_ms: f64,
    ) -> Receiver {
        let sh = Arc::new(Shared {
            stop: AtomicBool::new(false),
            url: Mutex::new(url.to_string()),
            reconnect: AtomicBool::new(false),
            out_rate,
            st: Mutex::new(State {
                buf: VecDeque::new(),
                start: 0,
                rate: out_rate,
                clocks: VecDeque::new(),
                have: false,
                last_chunk_at: mono_ns(),
                read: 0.0,
                playing: false,
                gain: 0.0,
                xf_from: 0.0,
                xf_left: 0,
                err_ema: 0.0,
                correction: 0.0,
                last_target: 0.0,
                latency_ms: latency_ms.clamp(10.0, 2000.0),
                trim_ms: 0.0,
                offset_frames: 0.0,
                out_total: 0,
                marks: VecDeque::new(),
                frames: FrameClock::default(),
                beat: None,
                peer_offset: None,
                transit: VecDeque::new(),
                stats: RxStats {
                    state: "connecting".into(),
                    url: url.to_string(),
                    ..Default::default()
                },
            }),
        });
        let (a, b) = (sh.clone(), sh.clone());
        let net = std::thread::Builder::new()
            .name("djnet-rx-net".into())
            .spawn(move || run_net(a))
            .ok();
        let feed = std::thread::Builder::new()
            .name("djnet-rx-feed".into())
            .spawn(move || run_feed(b, ring, mirror))
            .ok();
        Receiver { sh, net, feed }
    }

    /// Stop both threads; the ring, for the next receiver.
    pub fn stop(mut self) -> Option<rtrb::Producer<f32>> {
        self.sh.stop.store(true, Ordering::SeqCst);
        if let Some(n) = self.net.take() {
            let _ = n.join();
        }
        self.feed.take().and_then(|f| f.join().ok())
    }

    pub fn url(&self) -> String {
        self.sh.url.lock().map(|u| u.clone()).unwrap_or_default()
    }

    /// Fetch another URL (the same sender restarted its server elsewhere).
    pub fn set_url(&self, url: &str) {
        if let Ok(mut u) = self.sh.url.lock() {
            *u = url.to_string();
        }
        self.sh.reconnect.store(true, Ordering::SeqCst);
    }

    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.sh.st.lock().ok().map(|mut s| f(&mut s))
    }

    pub fn stats(&self) -> RxStats {
        self.with(|s| {
            let mut st = s.stats.clone();
            st.buffer_ms = if s.have { (s.end() as f64 - s.read) / s.rate * 1000.0 } else { 0.0 };
            st.target_ms = s.target_frames() / s.rate * 1000.0;
            st.correction_ppm = s.correction * 1e6;
            st.playing = s.playing;
            // between chunks the text can lag the flag
            if matches!(st.state.as_str(), "playing" | "buffering") {
                st.state = if s.playing { "playing".into() } else { "buffering".into() };
            }
            st.bpm = s.beat.map(|b| b.bpm);
            st.idle_s = (mono_ns() - s.last_chunk_at) as f64 / 1e9;
            st.latency_ms = s.latency_ms;
            st.trim_ms = s.trim_ms;
            st.offset_ms = s.offset_frames / s.rate * 1000.0;
            st.pending_ms = if s.playing { s.err_ema / s.rate * 1000.0 } else { 0.0 };
            if !s.transit.is_empty() {
                let mut v: Vec<i64> = s.transit.iter().copied().collect();
                v.sort_unstable();
                st.transit_ms = Some(v[v.len() / 2] as f64 / 1e6);
            }
            st
        })
        .unwrap_or_default()
    }

    /// The beat of the sample the strip plays (on this process's clock), once known.
    pub fn beat(&self) -> Option<StreamBeat> {
        self.with(|s| s.beat).flatten()
    }

    /// The jitter buffer's target, ms (30-1000 in the app; 10-2000 here).
    pub fn set_latency_ms(&self, ms: f64) {
        self.with(|s| s.latency_ms = ms.clamp(10.0, 2000.0));
    }

    /// A fine offset, ms (−50..50 in the app): for what the clock can't see.
    pub fn set_trim_ms(&self, ms: f64) {
        self.with(|s| s.trim_ms = ms.clamp(-200.0, 200.0));
    }

    /// Play the stream `beats` later (negative: earlier), at its tempo: Sync and the nudges.
    /// Kept within half a beat of none, and never below 10 ms of buffer.
    pub fn shift_beats(&self, beats: f64) {
        self.with(|s| {
            let bpm = s.beat.map_or(120.0, |b| b.bpm).max(1.0);
            let beat = 60.0 / bpm * s.rate;
            let mut off = s.offset_frames + beats * beat;
            off -= (off / beat).round() * beat;
            while (s.latency_ms + s.trim_ms) / 1000.0 * s.rate + off < 0.01 * s.rate {
                off += beat;
            }
            s.offset_frames = off;
        });
    }

    /// Back to no phase offset.
    pub fn reset_offset(&self) {
        self.with(|s| s.offset_frames = 0.0);
    }

    /// The sender's clock offset (this clock − the sender's, ns), for the latency figures.
    pub fn set_peer_offset(&self, ns: Option<i64>) {
        self.with(|s| s.peer_offset = ns);
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.sh.stop.store(true, Ordering::SeqCst);
    }
}

/// `http://host:port/path` → (host:port, path).
fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    Some((hostport.to_string(), path.to_string()))
}

const BACKOFF_MS: [u64; 4] = [250, 500, 1000, 2000];

fn run_net(sh: Arc<Shared>) {
    let mut fails = 0usize;
    let mut lost_since: Option<Instant> = None;
    let set = |f: &dyn Fn(&mut State)| {
        if let Ok(mut s) = sh.st.lock() {
            f(&mut s);
        }
    };
    while !sh.stop.load(Ordering::SeqCst) {
        sh.reconnect.store(false, Ordering::SeqCst);
        let url = sh.url.lock().map(|u| u.clone()).unwrap_or_default();
        let r = fetch(&sh, &url);
        if sh.stop.load(Ordering::SeqCst) {
            break;
        }
        let err = match r {
            Ok(()) => "the sender closed the stream".to_string(),
            Err(e) => e,
        };
        let since = *lost_since.get_or_insert_with(Instant::now);
        let state = if since.elapsed() > Duration::from_secs(60) { "lost" } else { "reconnecting" };
        set(&|s: &mut State| {
            s.stats.state = state.into();
            s.stats.reconnects += 1;
            s.stats.last_error = Some(err.clone());
            s.stats.url = url.clone();
        });
        let wait = Duration::from_millis(BACKOFF_MS[fails.min(BACKOFF_MS.len() - 1)]);
        fails += 1;
        let t0 = Instant::now();
        while t0.elapsed() < wait && !sh.stop.load(Ordering::SeqCst) && !sh.reconnect.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        // a fetch that ran a while resets the backoff (see `fetch`)
        if let Ok(s) = sh.st.lock() {
            if mono_ns() - s.last_chunk_at < 3_000_000_000 {
                fails = fails.min(1);
                lost_since = None;
            }
        }
    }
    set(&|s: &mut State| s.stats.state = "stopped".into());
}

/// One connection: until it ends, the receiver stops, or another URL is asked for.
fn fetch(sh: &Shared, url: &str) -> Result<(), String> {
    let (hostport, path) = split_url(url).ok_or_else(|| format!("not an http:// URL: {url}"))?;
    let addr = hostport
        .to_socket_addrs()
        .map_err(|e| format!("{hostport}: {e}"))?
        .next()
        .ok_or_else(|| format!("{hostport}: no address"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| format!("{addr}: {e}"))?;
    let _ = s.set_nodelay(true);
    s.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: {hostport}\r\nUser-Agent: djnet-rx/1\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut r = BufReader::with_capacity(1 << 16, s);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    if !line.starts_with("HTTP/1.1 200") && !line.starts_with("HTTP/1.0 200") {
        return Err(format!("{url}: {}", line.trim()));
    }
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line.trim().is_empty() {
            break;
        }
    }
    if let Ok(mut st) = sh.st.lock() {
        st.stats.state = if st.playing { "playing".into() } else { "buffering".into() };
        st.stats.url = url.to_string();
        st.stats.last_error = None;
    }
    let mut head = [0u8; djl::HEADER];
    let mut body = Vec::new();
    while !sh.stop.load(Ordering::SeqCst) && !sh.reconnect.load(Ordering::SeqCst) {
        // no data for 2 s (the read timeout) ends the connection
        r.read_exact(&mut head).map_err(|e| format!("no data: {e}"))?;
        let c = Chunk::parse(&head).ok_or("a bad chunk header")?;
        body.resize(c.body_len(), 0);
        r.read_exact(&mut body).map_err(|e| format!("cut off: {e}"))?;
        let now = mono_ns();
        if let Ok(mut st) = sh.st.lock() {
            st.push_chunk(c, &body, now);
            st.stats.state = if st.playing { "playing".into() } else { "buffering".into() };
        }
    }
    Ok(())
}

fn run_feed(sh: Arc<Shared>, mut ring: rtrb::Producer<f32>, mirror: Arc<InputMirror>) -> rtrb::Producer<f32> {
    let cap = ring.buffer().capacity();
    let mut out = Vec::with_capacity(4096);
    while !sh.stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(2));
        let block = mirror.block.load(Ordering::Relaxed) as usize;
        let start_at = ((sh.out_rate * 0.02) as usize).max(1024);
        // the strip plays once it holds start_at and skips past 3× it: sit ~1.5 ms × 1024 above,
        // so a feeder woken late (a loaded box) still finds room before the strip runs dry
        let desired = (start_at.max(block) + 1536).min(cap).min(3 * start_at.max(block) - 256);
        let fill = cap - ring.slots();
        let need = desired.saturating_sub(fill);
        let now = mono_ns();
        let Ok(mut s) = sh.st.lock() else { continue };
        if need > 0 {
            s.render(need, sh.out_rate, &mut out);
            if let Ok(chunk) = ring.write_chunk_uninit(out.len()) {
                chunk.fill_from_iter(out.iter().copied());
            }
        }
        // what the strip plays now: the samples it has taken, dated on this side's render clock
        let fill_now = cap - ring.slots();
        let played = s.out_total.saturating_sub(fill_now as u64);
        let rate_out = sh.out_rate;
        let t = s.frames.push(played as i64, now, rate_out);
        let read = s.read_at(played);
        let beat = read.and_then(|f| {
            let fi = f.floor() as i64;
            let c = *s.clock_for(fi)?;
            c.has_clock().then(|| {
                let b = StreamBeat { beat: c.beat_at(fi) + (f - fi as f64) * c.bpm / 60.0 / c.rate as f64, bpm: c.bpm, t, playing: c.playing() && s.playing };
                (b, c, fi)
            })
        });
        if let Some((b, c, fi)) = beat {
            s.beat = Some(b);
            if let Some(off) = s.peer_offset {
                // the sender rendered `fi` at its time_of(fi); here it renders at `t`
                s.stats.e2e_ms = Some((t - (c.time_of(fi) + off)) as f64 / 1e6);
            }
        }
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(frame: i64, n: u32, beat: f64) -> (Chunk, Vec<u8>) {
        let c = Chunk {
            seq: 0,
            frame,
            n,
            rate: 48_000,
            t: frame * 1_000_000_000 / 48_000,
            flags: djl::FLAG_CLOCK | djl::FLAG_PLAYING,
            beat,
            bpm: 120.0,
            channels: 2,
        };
        let mut v = Vec::new();
        let l: Vec<f32> = (0..n).map(|k| ((frame + k as i64) % 100) as f32 / 200.0).collect();
        djl::encode(&c, &l, &l, &mut v);
        (c, v[djl::HEADER..].to_vec())
    }

    fn state() -> State {
        State {
            buf: VecDeque::new(),
            start: 0,
            rate: 48_000.0,
            clocks: VecDeque::new(),
            have: false,
            last_chunk_at: 0,
            read: 0.0,
            playing: false,
            gain: 0.0,
            xf_from: 0.0,
            xf_left: 0,
            err_ema: 0.0,
            correction: 0.0,
            last_target: 0.0,
            latency_ms: 50.0,
            trim_ms: 0.0,
            offset_frames: 0.0,
            out_total: 0,
            marks: VecDeque::new(),
            frames: FrameClock::default(),
            beat: None,
            peer_offset: None,
            transit: VecDeque::new(),
            stats: RxStats::default(),
        }
    }

    #[test]
    fn it_waits_for_the_target_then_plays_at_it_and_fades_at_a_gap() {
        let mut s = state();
        let mut out = Vec::new();
        let (c, b) = chunk(1000, 960, 0.0);
        s.push_chunk(c, &b, 0);
        s.render(480, 48_000.0, &mut out);
        assert!(!s.playing, "960 frames < the 2400 the target wants");
        assert!(out.iter().all(|v| *v == 0.0));
        let (c, b) = chunk(1960, 2400, 0.5);
        s.push_chunk(c, &b, 0);
        s.render(480, 48_000.0, &mut out);
        assert!(s.playing);
        assert!(((s.end() as f64 - s.read) - (2400.0 - 480.0)).abs() < 2.0);
        // drain it: it fades and stops, counting one underrun
        for _ in 0..10 {
            s.render(480, 48_000.0, &mut out);
        }
        assert!(!s.playing);
        assert_eq!(s.stats.underruns, 1);
        // a reconnect after a second's gap: silence fills the timeline, no reset
        let (c, b) = chunk(4360 + 48_000, 4800, 0.0);
        s.push_chunk(c, &b, 0);
        assert_eq!(s.stats.resets, 0);
        // the sender restarted (its frame went back): a reset
        let (c, b) = chunk(10, 480, 0.0);
        s.push_chunk(c, &b, 0);
        assert_eq!(s.stats.resets, 1);
        assert_eq!(s.start, 10);
    }

    #[test]
    fn the_read_position_maps_back_to_a_beat() {
        let mut s = state();
        let mut out = Vec::new();
        for k in 0..20 {
            let f = k * 480;
            let (c, b) = chunk(f, 480, f as f64 * 2.0 / 48_000.0);
            s.push_chunk(c, &b, 0);
        }
        s.render(960, 48_000.0, &mut out);
        let r = s.read_at(s.out_total).unwrap();
        let c = s.clock_for(r as i64).unwrap();
        assert!((c.beat_at(r as i64) - r.floor() * 2.0 / 48_000.0).abs() < 1e-6);
        assert_eq!(split_url("http://1.2.3.4:8796/master.djl"), Some(("1.2.3.4:8796".into(), "/master.djl".into())));
    }
}

