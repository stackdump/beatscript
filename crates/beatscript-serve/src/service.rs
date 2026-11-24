//! Renders as jobs, mirroring `internal/btwsite/server/render.go` one for one:
//!
//! - a complete render is `<data>/render/<cid>.<sr>.<opus|wav>` (+ `.meta`), the Go server's names, so either
//!   server serves the other's cache;
//! - a render in progress is a growing `.part` file; the first request starts it, later ones join and read it
//!   from byte 0, following it (singleflight);
//! - the render dies when its last follower goes, the server stops, or it passes the wall limit;
//! - limits: `render-concurrency` renders at once (429 busy), `render-ip-hour` uncached renders per address,
//!   LRU eviction of the cache past `render-cache-mb`.
//!
//! The difference from Go is the inside of a job: no `beatscript` child and no `ffmpeg` child. The engine runs on a
//! thread of this process (`beatscript`'s own `render_to`, linked in), and its PCM goes straight into the WAV file or
//! the in-process Ogg Opus encoder.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::opus::OggOpus;
use crate::render::{self as cli_render, Events, Format, Options};
use crate::sched::{Permit, Sched};
use crate::ApiError;

/// The least wall time a render may take; `render_wall` gives a long one more (as the Go server's `renderWall`).
pub const RENDER_WALL_LIMIT: Duration = Duration::from_secs(600);

/// How long a render may run: at least 10 minutes, and a third of the longest audio the site renders (an hour of
/// audio may take 20 minutes on a slow, busy host). Go's `renderWall(maxSeconds)`.
pub fn render_wall(max_seconds: u32) -> Duration {
    (Duration::from_secs(u64::from(max_seconds)) / 3).max(RENDER_WALL_LIMIT)
}

/// A `.part` file nobody has written to for this long is an orphan (a live render writes several times a second).
pub const PART_ORPHAN_AFTER: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fmt {
    Opus,
    Wav,
}

/// What the engine makes of a program (the Go server's `api.RenderVersion` was `r2`). Bump it when the render starts producing
/// different audio for the same CID (cached files are served immutable); the page's `RENDER_VERSION` (site/web/js/api.js)
/// says the same. `r3` (2026-11-24): the render plays a program's control lines as the browser's engine plays them (the Wheel,
/// reverb and master crush as the page has them, `kit=` / `sound=`, every move on the booth's scheduler: beatscript-cli
/// src/browser.rs). Files of an older version are no longer cache names: never served, counted or evicted.
pub const RENDER_VERSION: &str = "r3";

/// beatscript-serve's Opus is not the Go server's: the same PCM through another encoder (libopus 1.3.1 in-process at 48 kHz,
/// where Go pipes 44.1 kHz through ffmpeg's resampler and the system libopus). Different bytes are a different render,
/// so they get their own version and never share a file name or an ETag with Go's `r2` Opus. The name is chosen so the
/// Go server's start-up sweep (`(?:r\d+\.)?`) neither counts nor deletes these files. WAV was byte-identical to Go's
/// (the contract test checked it) and shared `r2`; both moved on together (`r3`, `r3rs`).
pub const OPUS_VERSION: &str = "r3rs";

impl Fmt {
    pub fn version(self) -> &'static str {
        match self {
            Fmt::Opus => OPUS_VERSION,
            Fmt::Wav => RENDER_VERSION,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Fmt::Opus => "opus",
            Fmt::Wav => "wav",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub cid: String,
    pub format: Fmt,
    pub sr: u32,
}

impl Key {
    pub fn name(&self) -> String {
        format!("{}.{}.{}.{}", self.cid, self.sr, self.format.version(), self.format.name())
    }
    pub fn content_type(&self) -> &'static str {
        match self.format {
            Fmt::Wav => "audio/wav",
            Fmt::Opus => "audio/ogg; codecs=opus",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Start {
    pub seconds: f64,
    pub bars: i64,
}

#[derive(Default)]
struct JobState {
    written: u64,
    done: bool,
    err: Option<String>,
}

pub struct Job {
    pub key: Key,
    part: PathBuf,
    final_: PathBuf,
    cancel: AtomicBool,
    pub ready: watch::Receiver<Option<Result<Start, ApiError>>>,
    ready_tx: watch::Sender<Option<Result<Start, ApiError>>>,
    progress: AtomicU64,
    state: Mutex<JobState>,
    notify_tx: watch::Sender<u64>,
    started: Instant,
}

impl Job {
    pub fn progress(&self) -> f64 {
        f64::from_bits(self.progress.load(Ordering::Relaxed))
    }
    pub fn start(&self) -> Option<Start> {
        match &*self.ready.borrow() {
            Some(Ok(s)) => Some(*s),
            _ => None,
        }
    }
    /// (written, done, err)
    pub fn snapshot(&self) -> (u64, bool, Option<String>) {
        let s = self.state.lock().unwrap();
        (s.written, s.done, s.err.clone())
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.notify_tx.subscribe()
    }
    /// The file to read: the final name once it is complete, else the part (the rename happens under the same
    /// lock, so a reader that opened the part keeps the same inode).
    pub fn open(&self) -> io::Result<File> {
        let s = self.state.lock().unwrap();
        if s.done && s.err.is_none() {
            File::open(&self.final_)
        } else {
            File::open(&self.part)
        }
    }
    fn publish(&self, written: u64) {
        self.state.lock().unwrap().written = written;
        self.notify_tx.send_modify(|v| *v += 1);
    }
    fn finish(&self, err: Option<String>, swap: Option<&dyn Fn() -> io::Result<()>>) {
        let mut s = self.state.lock().unwrap();
        let mut err = err;
        if err.is_none() {
            if let Some(f) = swap {
                if let Err(e) = f() {
                    err = Some(e.to_string());
                }
            }
        }
        s.done = true;
        s.err = err;
        drop(s);
        self.notify_tx.send_modify(|v| *v += 1);
    }
}

pub struct CacheEntry {
    pub size: u64,
    used: SystemTime,
    chtimes: SystemTime,
    seconds: Option<f64>,
}

pub struct Limiter {
    n: usize,
    window: Duration,
    hits: HashMap<String, VecDeque<Instant>>,
}

impl Limiter {
    pub fn new(n: usize, window: Duration) -> Limiter {
        Limiter { n, window, hits: HashMap::new() }
    }
    /// (allowed, retry-after seconds), without recording.
    pub fn peek(&mut self, key: &str) -> (bool, u64) {
        let now = Instant::now();
        let q = self.hits.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            q.pop_front();
        }
        if q.len() < self.n {
            return (true, 0);
        }
        let retry = self.window.saturating_sub(now.duration_since(*q.front().unwrap())).as_secs() + 1;
        (false, retry)
    }
    pub fn record(&mut self, key: &str) {
        self.hits.entry(key.to_string()).or_default().push_back(Instant::now());
    }
    pub fn allow(&mut self, key: &str) -> (bool, u64) {
        let r = self.peek(key);
        if r.0 {
            self.record(key);
        }
        r
    }
}

struct Inner {
    jobs: HashMap<Key, Arc<Job>>,
    followers: HashMap<Key, usize>,
    cache: HashMap<String, CacheEntry>,
    total: u64,
    ip: Limiter,
}

pub struct Service {
    pub dir: PathBuf,
    cap: u64,
    slots: Arc<Semaphore>,
    concurrency: usize,
    ip_hour: usize,
    max_bars: u32,
    wall: Duration,
    /// The CPU pool every render's engine thread takes turns on.
    pub sched: Arc<Sched>,
    /// Refuse a new render (429 busy) while the most overdue render is this far behind its listener (zero: never).
    admit_lag: Duration,
    inner: Mutex<Inner>,
}

pub struct Limits {
    pub cache_mb: u64,
    pub concurrency: usize,
    pub ip_hour: usize,
    pub max_bars: u32,
    pub max_seconds: u32,
    pub workers: usize,
    pub admit_lag: Duration,
}

pub enum Acquired {
    Hit(PathBuf, Option<f64>),
    Job(Arc<Job>, bool),
}

impl Service {
    pub fn new(dir: &Path, l: Limits) -> io::Result<Service> {
        let dir = dir.to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let mut cache = HashMap::new();
        let mut total = 0;
        for e in std::fs::read_dir(&dir)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".part") {
                // ours are orphans at start-up; another server's only once nobody has written them for a while (the
                // Go server may be rendering into the same directory right now)
                if name.ends_with(".rs.part") || is_stale(&e.path()) {
                    let _ = std::fs::remove_file(e.path());
                }
                continue;
            }
            if is_cache_name(&name) {
                if let Ok(m) = e.metadata() {
                    let t = m.modified().unwrap_or(SystemTime::now());
                    cache.insert(name, CacheEntry { size: m.len(), used: t, chtimes: t, seconds: None });
                    total += m.len();
                }
            }
        }
        Ok(Service {
            dir,
            cap: l.cache_mb << 20,
            slots: Arc::new(Semaphore::new(l.concurrency)),
            concurrency: l.concurrency,
            ip_hour: l.ip_hour,
            max_bars: l.max_bars,
            wall: render_wall(l.max_seconds),
            sched: Arc::new(Sched::new(l.workers)),
            admit_lag: l.admit_lag,
            inner: Mutex::new(Inner { jobs: HashMap::new(), followers: HashMap::new(), cache, total, ip: Limiter::new(l.ip_hour, Duration::from_secs(3600)) }),
        })
    }

    /// Remove orphaned `.part` files (ours not in the job table, anyone's not written for `PART_ORPHAN_AFTER`). Run
    /// every few minutes; returns how many went.
    pub fn sweep_parts(&self) -> usize {
        let live: Vec<PathBuf> = self.inner.lock().unwrap().jobs.values().map(|j| j.part.clone()).collect();
        let mut n = 0;
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return 0 };
        for e in rd.flatten() {
            let p = e.path();
            if !e.file_name().to_string_lossy().ends_with(".part") || live.contains(&p) {
                continue;
            }
            if is_stale(&p) && std::fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// (cache files, bytes, cap) for the health line.
    pub fn cache_usage(&self) -> (usize, u64, u64) {
        let i = self.inner.lock().unwrap();
        (i.cache.len(), i.total, self.cap)
    }


    fn lookup_locked(&self, inner: &mut Inner, key: &Key) -> Option<(PathBuf, Option<f64>)> {
        let name = key.name();
        let p = self.dir.join(&name);
        if !inner.cache.contains_key(&name) {
            // the Go server, beside us, may have completed it: adopt what is on disk
            let m = std::fs::metadata(&p).ok()?;
            inner.cache.insert(name.clone(), CacheEntry { size: m.len(), used: SystemTime::now(), chtimes: SystemTime::now(), seconds: None });
            inner.total += m.len();
        }
        if std::fs::metadata(&p).is_err() {
            let e = inner.cache.remove(&name).unwrap();
            inner.total -= e.size;
            return None;
        }
        let now = SystemTime::now();
        let e = inner.cache.get_mut(&name).unwrap();
        e.used = now;
        if now.duration_since(e.chtimes).unwrap_or_default() > Duration::from_secs(600) {
            e.chtimes = now;
            let _ = File::options().write(true).open(&p).and_then(|f| f.set_modified(now));
        }
        if e.seconds.is_none() {
            e.seconds = std::fs::read(self.dir.join(format!("{name}.meta"))).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| v["seconds"].as_f64());
        }
        Some((p, e.seconds))
    }

    pub fn peek(&self, key: &Key) -> (Option<(PathBuf, Option<f64>)>, Option<Arc<Job>>) {
        let mut inner = self.inner.lock().unwrap();
        let hit = self.lookup_locked(&mut inner, key);
        let job = inner.jobs.get(key).cloned();
        (hit, job)
    }

    pub fn acquire(self: &Arc<Self>, key: &Key, ip: &str, text: &str) -> Result<Acquired, ApiError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some((p, s)) = self.lookup_locked(&mut inner, key) {
            return Ok(Acquired::Hit(p, s));
        }
        if let Some(j) = inner.jobs.get(key).cloned() {
            *inner.followers.entry(key.clone()).or_default() += 1;
            return Ok(Acquired::Job(j, true));
        }
        let (ok, retry) = inner.ip.peek(ip);
        if !ok {
            return Err(ApiError::new(429, "rate_limited", format!("at most {} new renders per hour per address; this one is cached by the time you retry, or try again in {}s", self.ip_hour, retry)).retry(retry));
        }
        let lagging = !self.admit_lag.is_zero() && self.sched.lag() > self.admit_lag;
        let permit = match self.slots.clone().try_acquire_owned() {
            Ok(p) if !lagging => p,
            _ => {
                let retry = self.busy_retry_locked(&inner);
                return Err(ApiError::new(429, "busy", format!("all {} render slots are busy; try again shortly", self.concurrency)).retry(retry));
            }
        };
        inner.ip.record(ip);
        let (ready_tx, ready) = watch::channel(None);
        let (notify_tx, _) = watch::channel(0u64);
        let j = Arc::new(Job {
            key: key.clone(),
            part: self.dir.join(format!("{}.rs.part", key.name())),
            final_: self.dir.join(key.name()),
            cancel: AtomicBool::new(false),
            ready,
            ready_tx,
            progress: AtomicU64::new(0),
            state: Mutex::new(JobState::default()),
            notify_tx,
            started: Instant::now(),
        });
        inner.jobs.insert(key.clone(), j.clone());
        inner.followers.insert(key.clone(), 1);
        let (svc, job, text) = (self.clone(), j.clone(), text.to_string());
        std::thread::Builder::new()
            .name(format!("render {}", key.name()))
            .spawn(move || {
                // yield to the other services on the host, as Go renices its children to 10
                renice_thread();
                svc.run(job, text, permit)
            })
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(Acquired::Job(j, false))
    }

    fn busy_retry_locked(&self, inner: &Inner) -> u64 {
        let mut best = 10.0f64;
        for j in inner.jobs.values() {
            if let Some(s) = j.start() {
                let left = s.seconds - j.progress();
                if left > 0.0 && left < best {
                    best = left;
                }
            }
        }
        (best / 4.0 + 1.0).clamp(2.0, 30.0) as u64
    }

    /// One follower gone; the last one out stops a render that is still going.
    pub fn leave(&self, job: &Arc<Job>) {
        let mut inner = self.inner.lock().unwrap();
        let n = inner.followers.entry(job.key.clone()).or_default();
        *n = n.saturating_sub(1);
        if *n == 0 {
            let (_, done, _) = job.snapshot();
            if !done {
                job.cancel.store(true, Ordering::Relaxed);
                if inner.jobs.get(&job.key).is_some_and(|j| Arc::ptr_eq(j, job)) {
                    inner.jobs.remove(&job.key);
                    inner.followers.remove(&job.key);
                }
            }
        }
    }

    fn remove_job(&self, job: &Arc<Job>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.jobs.get(&job.key).is_some_and(|j| Arc::ptr_eq(j, job)) {
            inner.jobs.remove(&job.key);
            inner.followers.remove(&job.key);
        }
    }

    fn add_cache(&self, key: &Key, size: u64, seconds: f64) {
        let mut inner = self.inner.lock().unwrap();
        let name = key.name();
        if let Some(old) = inner.cache.remove(&name) {
            inner.total -= old.size;
        }
        let now = SystemTime::now();
        inner.cache.insert(name.clone(), CacheEntry { size, used: now, chtimes: now, seconds: Some(seconds) });
        inner.total += size;
        while inner.total > self.cap && inner.cache.len() > 1 {
            let Some(oldest) = inner.cache.iter().filter(|(n, _)| **n != name).min_by_key(|(_, e)| e.used).map(|(n, _)| n.clone()) else { break };
            let e = inner.cache.remove(&oldest).unwrap();
            inner.total -= e.size;
            let _ = std::fs::remove_file(self.dir.join(&oldest));
            let _ = std::fs::remove_file(self.dir.join(format!("{oldest}.meta")));
        }
    }

    /// One render, from the engine's start to the finished file (or its removal). Runs on its own thread.
    fn run(&self, job: Arc<Job>, text: String, permit: OwnedSemaphorePermit) {
        let _permit = permit;
        let fail = |job: &Arc<Job>, msg: String| {
            let _ = std::fs::remove_file(&job.part);
            self.remove_job(job);
            if job.ready.borrow().is_none() {
                let _ = job.ready_tx.send(Some(Err(ApiError::new(422, "render_failed", msg.clone()))));
            }
            job.finish(Some(msg), None);
        };
        let f = match File::options().create(true).truncate(true).read(true).write(true).open(&job.part) {
            Ok(f) => f,
            Err(e) => return fail(&job, e.to_string()),
        };
        let wall = self.wall;
        let out = Published { f, written: 0, job: job.clone(), wall };
        // a CPU slot before the check and the engine's start: deadline now (a listener is waiting for the first byte)
        let Some(cpu) = self.sched.acquire(job.started, &job.cancel) else {
            return fail(&job, "render cancelled".into());
        };
        let pace = |frame_bytes: usize, sr: u32| Pace { sched: self.sched.clone(), job: job.clone(), permit: Some(cpu), bytes: 0, frame_bytes: frame_bytes as u64, sr: f64::from(sr) };
        let key = job.key.clone();
        // the engine's rate: Opus is 48 kHz only, so the engine renders at 48 kHz (nothing to resample); WAV at the
        // asked rate, the same PCM beatscript hands the Go server
        let (engine_sr, format) = match key.format {
            Fmt::Opus => (48_000, Format::F32le),
            Fmt::Wav => (key.sr, Format::S16le),
        };
        let opts = Options { file: None, sr: engine_sr, format, chunk_ms: 250, max_bars: self.max_bars, from_bar: 0 };
        let mut ev_sink = EventSink { job: job.clone(), line: Vec::new() };
        let result: Result<Published, String> = match key.format {
            Fmt::Wav => {
                let mut out = out;
                let r = {
                    let mut sink = WavSink { out: &mut out, header: Some(wav_header(key.sr, 2)), pace: pace(4, key.sr) };
                    let mut ev = Events::new(&mut ev_sink);
                    caught(|| cli_render::render_to(&text, &opts, &mut sink, &mut ev, Instant::now()))
                };
                r.map(|_| out).map_err(|e| e.message)
            }
            Fmt::Opus => {
                // the encoder on its own thread, so the engine and libopus run side by side (as beatscript | ffmpeg did, in
                // two processes); a bounded channel of 250 ms chunks between them keeps memory flat
                let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
                let (sr, serial) = (key.sr, serial_of(&key.cid));
                let enc = std::thread::Builder::new()
                    .name(format!("opus {}", key.name()))
                    .spawn(move || -> Result<Published, String> {
                        renice_thread();
                        let mut out = out;
                        {
                            let mut enc = OggOpus::new(&mut out, sr, 96_000, serial).map_err(|e| e.to_string())?;
                            for chunk in rx {
                                enc.push(&chunk).map_err(|e| e.to_string())?;
                            }
                            enc.finish().map_err(|e| e.to_string())?;
                        }
                        Ok(out)
                    })
                    .expect("encoder thread");
                let r = {
                    let mut sink = ChanSink { tx, carry: Vec::new(), pace: pace(8, 48_000) };
                    let mut ev = Events::new(&mut ev_sink);
                    caught(|| cli_render::render_to(&text, &opts, &mut sink, &mut ev, Instant::now()))
                }; // the sink is dropped: the encoder sees the end of the stream
                let encoded = enc.join().unwrap_or_else(|_| Err("the encoder panicked".into()));
                match r {
                    Err(e) => Err(e.message),
                    Ok(_) => encoded,
                }
            }
        };
        let out = match result {
            Ok(o) => o,
            Err(m) => {
                let m = if job.cancel.load(Ordering::Relaxed) { "render cancelled".to_string() } else { m };
                return fail(&job, m);
            }
        };
        let written = out.written;
        let mut f = out.f;
        if key.format == Fmt::Wav && written >= 44 && written - 44 < 0xFFFF_FFFF - 36 {
            // the cached copy gets its real sizes (the streamed one says 0xFFFFFFFF)
            let _ = f.seek(SeekFrom::Start(4)).and_then(|_| f.write_all(&((written - 8) as u32).to_le_bytes()));
            let _ = f.seek(SeekFrom::Start(40)).and_then(|_| f.write_all(&((written - 44) as u32).to_le_bytes()));
        }
        if let Err(e) = f.sync_data() {
            return fail(&job, e.to_string());
        }
        drop(f);
        let start = job.start().unwrap_or_default();
        // renderMeta as Go marshals it
        let meta = format!("{{\"seconds\":{},\"bars\":{}}}", go_float(start.seconds), start.bars);
        let _ = std::fs::write(format!("{}.meta", job.final_.display()), meta);
        let (part, fin) = (job.part.clone(), job.final_.clone());
        job.finish(None, Some(&move || std::fs::rename(&part, &fin)));
        if let (_, true, None) = job.snapshot() {
            self.add_cache(&key, written, start.seconds);
        } else {
            let _ = std::fs::remove_file(&job.part);
        }
        self.remove_job(&job);
    }
}

/// `render_to`, with a panic turned into the error beatscript's CLI prints for one (`internal`).
fn caught<T>(f: impl FnOnce() -> Result<T, cli_render::RenderError>) -> Result<T, cli_render::RenderError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let m = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
        Err(cli_render::RenderError { code: "internal", message: format!("internal error: {m}") })
    })
}

/// Go's strconv 'g'-shortest float, as encoding/json writes it (enough for seconds like 49.87).
fn go_float(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e21 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

fn serial_of(cid: &str) -> u32 {
    // a stream serial that is the same for the same program (a render is reproducible byte for byte)
    cid.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619))
}

fn is_stale(p: &Path) -> bool {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > PART_ORPHAN_AFTER)
}

/// `<cid>.<sr>.<version>.<fmt>` of this server's versions (Go's `reCacheName`, per format: WAV [`RENDER_VERSION`], Opus [`OPUS_VERSION`]).
pub fn is_cache_name(n: &str) -> bool {
    let mut it = n.rsplitn(4, '.');
    let (Some(ext), Some(ver), Some(sr), Some(cid)) = (it.next(), it.next(), it.next(), it.next()) else { return false };
    let want = match ext {
        "wav" => Fmt::Wav.version(),
        "opus" => Fmt::Opus.version(),
        _ => return false,
    };
    ver == want && !sr.is_empty() && sr.bytes().all(|b| b.is_ascii_digit()) && crate::store::is_cid(cid)
}

pub fn wav_header(sr: u32, ch: u16) -> Vec<u8> {
    let mut b = Vec::with_capacity(44);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * u32::from(ch) * 2).to_le_bytes());
    b.extend_from_slice(&(ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    b
}

/// The job's file: every write is published to the followers at once. A cancelled job (its last follower
/// left, or the wall limit passed) refuses the write, which ends `render_to` with `closed`.
struct Published {
    f: File,
    written: u64,
    job: Arc<Job>,
    wall: Duration,
}

impl Write for Published {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.job.cancel.load(Ordering::Relaxed) || self.job.started.elapsed() > self.wall {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "render cancelled"));
        }
        self.f.write_all(b)?;
        self.written += b.len() as u64;
        self.job.publish(self.written);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The engine thread's turn on the CPU pool: after each PCM chunk it hands its slot back and queues for the next one
/// with that chunk's deadline (`sched.rs`).
struct Pace {
    sched: Arc<Sched>,
    job: Arc<Job>,
    permit: Option<Permit>,
    bytes: u64,
    frame_bytes: u64,
    sr: f64,
}

impl Pace {
    /// One chunk of `n` PCM bytes is out of the engine: hand the CPU slot back, do `then` (which may block: the
    /// encoder's channel is full, the disk is slow) without holding it, and queue for the next chunk's deadline.
    fn chunk_done(&mut self, n: usize, then: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        self.bytes += n as u64;
        self.permit = None;
        then()?;
        let ahead = Duration::from_secs_f64(self.bytes as f64 / self.frame_bytes as f64 / self.sr);
        self.permit = self.sched.acquire(self.job.started + ahead, &self.job.cancel);
        if self.permit.is_none() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "render cancelled"));
        }
        Ok(())
    }
}

struct WavSink<'a> {
    out: &'a mut Published,
    header: Option<Vec<u8>>,
    pace: Pace,
}

impl Write for WavSink<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if let Some(h) = self.header.take() {
            self.out.write_all(&h)?;
        }
        let out = &mut *self.out;
        self.pace.chunk_done(b.len(), || out.write_all(b))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// f32le interleaved stereo bytes in, sample chunks to the encoder thread out. When the encoder has gone (its
/// file write failed: cancelled, disk full) the send fails and so does the render.
struct ChanSink {
    tx: std::sync::mpsc::SyncSender<Vec<f32>>,
    carry: Vec<u8>,
    pace: Pace,
}

impl Write for ChanSink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.carry.extend_from_slice(b);
        let whole = self.carry.len() / 8 * 8;
        let chunk: Vec<f32> = self.carry[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        self.carry.drain(..whole);
        let tx = &self.tx;
        self.pace.chunk_done(b.len(), || tx.send(chunk).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the encoder stopped")))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `render_to`'s JSON lines: the start event makes the job ready, progress lines move its progress.
struct EventSink {
    job: Arc<Job>,
    line: Vec<u8>,
}

impl Write for EventSink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(b);
        while let Some(i) = self.line.iter().position(|c| *c == b'\n') {
            let l: Vec<u8> = self.line.drain(..=i).collect();
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&l) else { continue };
            match v["event"].as_str() {
                Some("start") => {
                    let s = Start { seconds: v["seconds"].as_f64().unwrap_or(0.0), bars: v["bars"].as_i64().unwrap_or(0) };
                    let _ = self.job.ready_tx.send(Some(Ok(s)));
                }
                Some("progress") => {
                    if let Some(x) = v["seconds_rendered"].as_f64() {
                        self.job.progress.store(x.to_bits(), Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Lower the calling thread's priority (Go reniced its children to 10). Linux only:
/// `gettid` does not exist elsewhere, so other platforms keep the default priority.
fn renice_thread() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
}
