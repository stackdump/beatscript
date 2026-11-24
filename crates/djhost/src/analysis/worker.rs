//! Analysis on a thread of its own: jobs in, events out, nothing ever waits on it.
//!
//! The window and the control socket [`submit`](Worker::submit) a job (mono audio and what to
//! find) and later [`poll`](Worker::poll) its events. A grid job answers twice: the DSP grid at
//! once ([`Stage::Dsp`]), then the final one ([`Stage::Final`]) when Beat This! is done (or at
//! once again when it cannot run). The models are loaded on the worker's first job that wants
//! them: fetched and checked if they are missing. If that fails the worker stops trying (the DSP
//! grid stands, and the status says why) until [`retry`](Worker::retry) or
//! [`set_neural`](Worker::set_neural) turns it on again, so an offline machine is not asked to
//! download on every capture.
//!
//! The cache answers a grid job only with a grid of the analyzer that would make it now
//! ([`super::analyzers`]): Beat This! when it is wanted (its pinned id, or the loaded backend's), the
//! DSP detector when it is off or cannot run. A changed detector or model finds nothing and analyses.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use djengine::beatgrid::{estimate_latency, LatencyEstimate};

use super::cache::Cache;
use super::{analyze, analyzers, content_key, Analysis, Backend, Options};

/// What a job finds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A beat grid.
    Grid,
    /// How late the audio's beats are against a clock: the clock's beat period and the time of
    /// one of its beats, both in seconds on the audio's own clock.
    Latency { ref_period: f64, ref_origin: f64 },
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub kind: Kind,
    /// Whatever the caller wants back with the answer (a clip id, an input label, a path).
    pub tag: String,
    pub audio: Vec<f32>,
    pub sample_rate: f64,
    /// Read and write the cache (a grid job only).
    pub use_cache: bool,
    /// The audio's own tempo, when it has one: the grid is that, nothing is detected, and no
    /// model is loaded for it.
    pub known_bpm: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The DSP grid; Beat This! is still working.
    Dsp,
    Final,
}

#[derive(Clone, Debug)]
pub enum Event {
    Grid { id: u64, tag: String, key: String, stage: Stage, cached: bool, analysis: Box<Analysis> },
    Latency {
        id: u64,
        tag: String,
        estimate: Option<LatencyEstimate>,
        method: &'static str,
        note: String,
        /// The tempo of the source's own grid, when it has one: a source that is not on the clock
        /// has a tempo of its own, and no latency to speak of.
        source_bpm: Option<f64>,
    },
    Failed { id: u64, tag: String, error: String },
}

/// What the worker is doing, for `analysis status`.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// `off`, `idle` (on, nothing loaded yet), `loading`, `ready` or `failed`.
    pub neural: String,
    /// The backend's name once loaded.
    pub backend: String,
    /// Why Beat This! is not running, if it is not.
    pub last_error: String,
    pub done: usize,
}

type Factory = Box<dyn FnMut() -> Result<Box<dyn Backend>, String> + Send>;

enum Msg {
    Job(Job),
    Retry,
}

struct Shared {
    neural: AtomicBool,
    queued: AtomicUsize,
    status: Mutex<Status>,
}

pub struct Worker {
    tx: Option<Sender<Msg>>,
    rx: Receiver<Event>,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
    next_id: u64,
}

impl Worker {
    /// A worker whose backend comes from `factory`, called on the worker's thread the first time
    /// a job wants it. `cache`: where grids are kept (None: not kept). The backend's analyzer is
    /// known once it is loaded, so before that the cache answers no grid of it.
    pub fn spawn(factory: Factory, cache: Option<Cache>, opts: Options, neural: bool) -> Worker {
        Worker::spawn_inner(factory, None, cache, opts, neural)
    }

    /// [`spawn`](Self::spawn), with the analyzer id `factory`'s backend will have when that is known
    /// before it is loaded (Beat This!'s is pinned): a cached grid of it answers without loading
    /// the model.
    fn spawn_inner(factory: Factory, neural_analyzer: Option<String>, cache: Option<Cache>, opts: Options, neural: bool) -> Worker {
        let (tx, jobs) = channel::<Msg>();
        let (events, rx) = channel::<Event>();
        let shared = Arc::new(Shared {
            neural: AtomicBool::new(neural),
            queued: AtomicUsize::new(0),
            status: Mutex::new(Status { neural: if neural { "idle" } else { "off" }.into(), ..Status::default() }),
        });
        let s = shared.clone();
        let handle = std::thread::Builder::new()
            .name("djapp-analysis".into())
            .spawn(move || run(jobs, events, s, factory, neural_analyzer, cache, opts))
            .expect("spawn the analysis thread");
        Worker { tx: Some(tx), rx, shared, handle: Some(handle), next_id: 1 }
    }

    /// The real one: Beat This! with its models in `models_dir` (fetched on first use), or the DSP
    /// detector alone when this build has no `neural` feature.
    pub fn spawn_default(models_dir: std::path::PathBuf, cache: Option<Cache>, neural: bool) -> Worker {
        #[cfg(feature = "neural")]
        let (factory, pinned): (Factory, Option<String>) = (
            Box::new(move || {
                super::neural::NeuralBackend::fetch_and_load(&models_dir).map(|b| Box::new(b) as Box<dyn Backend>)
            }),
            Some(super::models::analyzer()),
        );
        #[cfg(not(feature = "neural"))]
        let (factory, pinned): (Factory, Option<String>) = {
            let _ = models_dir;
            (Box::new(|| Err("this build has no neural beat detection (build with --features neural)".into())), None)
        };
        Worker::spawn_inner(factory, pinned, cache, Options::default(), neural)
    }

    /// Queue a job; returns its id.
    pub fn submit(&mut self, kind: Kind, tag: &str, audio: Vec<f32>, sample_rate: f64, use_cache: bool) -> u64 {
        self.submit_known(kind, tag, audio, sample_rate, use_cache, None)
    }

    /// [`submit`](Self::submit) for audio that carries its own tempo (`known_bpm`).
    pub fn submit_known(&mut self, kind: Kind, tag: &str, audio: Vec<f32>, sample_rate: f64, use_cache: bool, known_bpm: Option<f64>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.shared.queued.fetch_add(1, Ordering::Relaxed);
        let job = Job { id, kind, tag: tag.to_string(), audio, sample_rate, use_cache, known_bpm };
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Job(job));
        }
        id
    }

    /// The next event, if one is ready.
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Wait up to `ms` for the next event (tests, the CLI).
    pub fn wait(&self, ms: u64) -> Option<Event> {
        self.rx.recv_timeout(std::time::Duration::from_millis(ms)).ok()
    }

    /// Jobs not yet finished.
    pub fn queued(&self) -> usize {
        self.shared.queued.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn neural_enabled(&self) -> bool {
        self.shared.neural.load(Ordering::Relaxed)
    }

    /// Turn Beat This! on or off. Turning it on after a failure tries again.
    pub fn set_neural(&self, on: bool) {
        let was = self.shared.neural.swap(on, Ordering::Relaxed);
        if on && !was {
            if let Some(tx) = &self.tx {
                let _ = tx.send(Msg::Retry);
            }
        }
        if !on {
            if let Ok(mut s) = self.shared.status.lock() {
                s.neural = "off".into();
            }
        }
    }

    /// Try loading the backend again after a failure.
    pub fn retry(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Retry);
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.tx = None; // the thread ends when its channel does
        if let Some(h) = self.handle.take() {
            // a job in flight is let finish rather than killed; shutdown is not the audio's problem
            let _ = h.join();
        }
    }
}

enum Backing {
    NotTried,
    Ready(Box<dyn Backend>),
    Failed(String),
}

fn set_status(shared: &Shared, f: impl FnOnce(&mut Status)) {
    if let Ok(mut s) = shared.status.lock() {
        f(&mut s);
    }
}

fn run(jobs: Receiver<Msg>, events: Sender<Event>, shared: Arc<Shared>, mut factory: Factory, neural_analyzer: Option<String>, cache: Option<Cache>, opts: Options) {
    let mut backing = Backing::NotTried;
    while let Ok(msg) = jobs.recv() {
        let job = match msg {
            Msg::Retry => {
                if matches!(backing, Backing::Failed(_)) {
                    backing = Backing::NotTried;
                }
                continue;
            }
            Msg::Job(j) => j,
        };
        let want_neural = shared.neural.load(Ordering::Relaxed);
        // a clip that knows its tempo, or one with nothing to find: answered at once, with no
        // model loaded and nothing downloaded for it
        if job.kind == Kind::Grid {
            let o = Options { known_bpm: job.known_bpm, ..opts };
            if let Some(a) = super::pre_analysis(&job.audio, job.sample_rate, &o) {
                let key = content_key(&job.audio, job.sample_rate);
                let _ = events.send(Event::Grid { id: job.id, tag: job.tag, key, stage: Stage::Final, cached: false, analysis: Box::new(a) });
                shared.queued.fetch_sub(1, Ordering::Relaxed);
                set_status(&shared, |s| s.done += 1);
                continue;
            }
        }
        // the cache: a grid of the analyzer that would answer now. Beat This! when it is wanted (the
        // loaded backend's id, else the one it is pinned to), then the DSP detector when Beat This! is
        // off or cannot run. Never a grid of another analyzer: a changed one finds nothing here.
        let key = content_key(&job.audio, job.sample_rate);
        if job.kind == Kind::Grid && job.use_cache && cache.is_some() {
            let mut wanted: Vec<String> = Vec::new();
            if want_neural {
                let neural_id = match &backing {
                    Backing::Ready(b) => Some(b.analyzer()),
                    _ => neural_analyzer.clone(),
                };
                wanted.extend(neural_id);
            }
            if !want_neural || matches!(backing, Backing::Failed(_)) {
                wanted.push(analyzers::BEATGRID_DSP.to_string());
            }
            if let Some(hit) = cache.as_ref().and_then(|c| wanted.iter().find_map(|id| c.get(&key, id))) {
                let mut a = Analysis {
                    method: if hit.method == "neural" { "neural" } else { "dsp" },
                    analyzer: hit.analyzer,
                    dsp: None,
                    detections: Default::default(),
                    note: "cached".into(),
                    seconds: 0.0,
                    duration: hit.duration,
                    grid: Some(hit.grid),
                };
                a.dsp = a.grid.clone().filter(|g| g.source == djengine::beatgrid::GridSource::Dsp);
                let _ = events.send(Event::Grid {
                    id: job.id,
                    tag: job.tag,
                    key,
                    stage: Stage::Final,
                    cached: true,
                    analysis: Box::new(a),
                });
                shared.queued.fetch_sub(1, Ordering::Relaxed);
                set_status(&shared, |s| s.done += 1);
                continue;
            }
        }
        // the DSP grid first, so there is something to show while the models load and run
        let mut why_none = String::new();
        if want_neural {
            if matches!(backing, Backing::NotTried) {
                set_status(&shared, |s| s.neural = "loading".into());
                match factory() {
                    Ok(b) => {
                        let name = b.name().to_string();
                        backing = Backing::Ready(b);
                        set_status(&shared, |s| {
                            s.neural = "ready".into();
                            s.backend = name;
                            s.last_error.clear();
                        });
                    }
                    Err(e) => {
                        set_status(&shared, |s| {
                            s.neural = "failed".into();
                            s.last_error = e.clone();
                        });
                        backing = Backing::Failed(e);
                    }
                }
            }
            if let Backing::Failed(e) = &backing {
                why_none = format!("Beat This! unavailable: {e}");
            }
        } else {
            why_none = "neural detection is off".into();
        }
        if job.kind == Kind::Grid && matches!(backing, Backing::Ready(_)) && want_neural {
            let quick = analyze(&job.audio, job.sample_rate, None, "", opts);
            let _ = events.send(Event::Grid {
                id: job.id,
                tag: job.tag.clone(),
                key: key.clone(),
                stage: Stage::Dsp,
                cached: false,
                analysis: Box::new(quick),
            });
        }
        let backend: Option<&mut dyn Backend> = match (&mut backing, want_neural) {
            (Backing::Ready(b), true) => Some(b.as_mut()),
            _ => None,
        };
        let a = analyze(&job.audio, job.sample_rate, backend, &why_none, opts);
        match job.kind {
            Kind::Grid => {
                if job.use_cache {
                    if let Some(c) = &cache {
                        c.put(&key, &a);
                    }
                }
                let _ = events.send(Event::Grid {
                    id: job.id,
                    tag: job.tag,
                    key,
                    stage: Stage::Final,
                    cached: false,
                    analysis: Box::new(a),
                });
            }
            Kind::Latency { ref_period, ref_origin } => {
                let det = &a.detections;
                let period = ref_period / det.fold.max(1) as f64;
                let est = estimate_latency(&det.times, &det.weights, period, ref_origin);
                let _ = events.send(Event::Latency {
                    id: job.id,
                    tag: job.tag,
                    estimate: est,
                    method: if det.fold == 1 { "neural" } else { "dsp" },
                    source_bpm: a.grid.as_ref().map(|g| g.bpm()),
                    note: a.note,
                });
            }
        }
        shared.queued.fetch_sub(1, Ordering::Relaxed);
        set_status(&shared, |s| s.done += 1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::super::RawBeats;
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const SR: f64 = 44_100.0;

    fn oracle_factory(calls: Arc<AtomicUsize>, bpm: f64, first: f64) -> Factory {
        struct Counted(Oracle, Arc<AtomicUsize>);
        impl Backend for Counted {
            fn name(&self) -> &'static str {
                "oracle"
            }
            fn analyzer(&self) -> String {
                "oracle@1".into()
            }
            fn detect(&mut self, m: &[f32], sr: f64) -> Result<RawBeats, String> {
                self.1.fetch_add(1, Ordering::SeqCst);
                self.0.detect(m, sr)
            }
        }
        Box::new(move || Ok(Box::new(Counted(Oracle { bpm, first, jitter: 0.004, calls: 0 }, calls.clone()))))
    }

    /// A backend like the oracle but with analyzer id `id` (another detector, or another model).
    fn oracle_as(id: &'static str, calls: Arc<AtomicUsize>, bpm: f64, first: f64) -> Factory {
        struct As(&'static str, Oracle, Arc<AtomicUsize>);
        impl Backend for As {
            fn name(&self) -> &'static str {
                "oracle"
            }
            fn analyzer(&self) -> String {
                self.0.into()
            }
            fn detect(&mut self, m: &[f32], sr: f64) -> Result<RawBeats, String> {
                self.2.fetch_add(1, Ordering::SeqCst);
                self.1.detect(m, sr)
            }
        }
        Box::new(move || Ok(Box::new(As(id, Oracle { bpm, first, jitter: 0.004, calls: 0 }, calls.clone()))))
    }

    /// The final grid event of the next job, skipping the DSP stage.
    fn final_grid(w: &Worker) -> (bool, Box<Analysis>) {
        while let Some(e) = w.wait(20_000) {
            if let Event::Grid { stage: Stage::Final, cached, analysis, .. } = e {
                return (cached, analysis);
            }
        }
        panic!("no final grid");
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("djanalysis-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_grid_job_answers_dsp_first_then_the_final_grid() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn(oracle_factory(calls.clone(), 124.0, 0.3), None, Options::default(), true);
        let audio = click_track(124.0, 0.3, 16.0, SR);
        let id = w.submit(Kind::Grid, "clip-1", audio, SR, false);
        let Some(Event::Grid { stage: Stage::Dsp, id: i1, analysis, .. }) = w.wait(20_000) else { panic!("no dsp event") };
        assert_eq!(i1, id);
        assert_eq!(analysis.method, "dsp");
        let Some(Event::Grid { stage: Stage::Final, tag, analysis, cached, .. }) = w.wait(20_000) else { panic!("no final") };
        assert_eq!(tag, "clip-1");
        assert!(!cached);
        assert_eq!(analysis.method, "neural");
        assert!((analysis.grid.unwrap().bpm() - 124.0).abs() < 0.05);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(w.status().neural, "ready");
    }

    #[test]
    fn the_same_audio_again_is_read_from_the_cache() {
        let dir = tmp("cache");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn(oracle_factory(calls.clone(), 100.0, 0.2), Some(Cache::new(&dir)), Options::default(), true);
        let audio = click_track(100.0, 0.2, 16.0, SR);
        w.submit(Kind::Grid, "a", audio.clone(), SR, true);
        let mut finals = 0;
        while let Some(e) = w.wait(20_000) {
            if matches!(e, Event::Grid { stage: Stage::Final, .. }) {
                finals += 1;
                break;
            }
        }
        assert_eq!(finals, 1);
        w.submit(Kind::Grid, "b", audio, SR, true);
        let Some(Event::Grid { stage, cached, tag, analysis, .. }) = w.wait(20_000) else { panic!() };
        assert_eq!((stage, cached, tag.as_str()), (Stage::Final, true, "b"));
        assert_eq!(analysis.method, "neural");
        assert!((analysis.grid.unwrap().bpm() - 100.0).abs() < 0.05);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the second was not analysed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_analyzer_is_never_answered_from_the_old_ones_cache() {
        let dir = tmp("analyzer");
        let audio = click_track(118.0, 0.25, 16.0, SR);
        let old = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn(oracle_as("oracle@1", old.clone(), 118.0, 0.25), Some(Cache::new(&dir)), Options::default(), true);
        w.submit(Kind::Grid, "a", audio.clone(), SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str()), (false, "oracle@1"));
        drop(w);
        // the detector changes (a new version, another model): the same audio is analysed again
        let new = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn(oracle_as("oracle@2", new.clone(), 118.0, 0.25), Some(Cache::new(&dir)), Options::default(), true);
        w.submit(Kind::Grid, "b", audio.clone(), SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str(), a.method), (false, "oracle@2", "neural"));
        assert_eq!(new.load(Ordering::SeqCst), 1, "the new analyzer ran");
        // and then its own grid is the one kept for it
        w.submit(Kind::Grid, "c", audio.clone(), SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str()), (true, "oracle@2"));
        assert_eq!(new.load(Ordering::SeqCst), 1);
        drop(w);
        // Beat This! off: the DSP grid kept beside them answers, named as the DSP detector's
        let mut w = Worker::spawn(Box::new(|| Err("should not be asked".into())), Some(Cache::new(&dir)), Options::default(), false);
        w.submit(Kind::Grid, "d", audio, SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str(), a.method), (true, analyzers::BEATGRID_DSP, "dsp"));
        assert_eq!(a.grid.unwrap().source, djengine::beatgrid::GridSource::Dsp);
        assert_eq!(old.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pinned_analyzer_is_answered_from_the_cache_without_loading_the_model() {
        let dir = tmp("pinned");
        let audio = click_track(126.0, 0.1, 16.0, SR);
        let mut w = Worker::spawn(oracle_as("pinned@1", Arc::new(AtomicUsize::new(0)), 126.0, 0.1), Some(Cache::new(&dir)), Options::default(), true);
        w.submit(Kind::Grid, "a", audio.clone(), SR, true);
        let _ = final_grid(&w);
        drop(w);
        let counting = |tries: Arc<AtomicUsize>| -> Factory {
            Box::new(move || {
                tries.fetch_add(1, Ordering::SeqCst);
                Err("offline".into())
            })
        };
        // the same pin: answered, nothing loaded
        let tries = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn_inner(counting(tries.clone()), Some("pinned@1".into()), Some(Cache::new(&dir)), Options::default(), true);
        w.submit(Kind::Grid, "b", audio.clone(), SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str(), a.method), (true, "pinned@1", "neural"));
        assert_eq!(tries.load(Ordering::SeqCst), 0);
        drop(w);
        // another pin finds nothing of its own; with the model unavailable the DSP grid stands
        let tries = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn_inner(counting(tries.clone()), Some("pinned@2".into()), Some(Cache::new(&dir)), Options::default(), true);
        w.submit(Kind::Grid, "c", audio.clone(), SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str()), (false, analyzers::BEATGRID_DSP));
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        w.submit(Kind::Grid, "d", audio, SR, true);
        let (cached, a) = final_grid(&w);
        assert_eq!((cached, a.analyzer.as_str()), (true, analyzers::BEATGRID_DSP), "the model failed: the DSP detector is what answers");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_models_leave_the_dsp_grid_and_are_not_retried_per_job() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let factory: Factory = Box::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
            Err("no network".into())
        });
        let mut w = Worker::spawn(factory, None, Options::default(), true);
        for n in 0..2 {
            w.submit(Kind::Grid, &format!("c{n}"), click_track(120.0, 0.1, 14.0, SR), SR, false);
            let Some(Event::Grid { stage: Stage::Final, analysis, .. }) = w.wait(20_000) else { panic!() };
            assert_eq!(analysis.method, "dsp");
            assert!(analysis.note.contains("no network"), "{}", analysis.note);
            assert!((analysis.grid.unwrap().bpm() - 120.0).abs() < 0.6);
        }
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        assert_eq!(w.status().neural, "failed");
        // turning it off and on again tries once more
        w.set_neural(false);
        w.set_neural(true);
        w.submit(Kind::Grid, "c2", click_track(120.0, 0.1, 14.0, SR), SR, false);
        let _ = w.wait(20_000);
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn neural_off_never_loads_a_model() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let factory: Factory = Box::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
            Err("should not be asked".into())
        });
        let mut w = Worker::spawn(factory, None, Options::default(), false);
        w.submit(Kind::Grid, "x", click_track(120.0, 0.1, 14.0, SR), SR, false);
        let Some(Event::Grid { analysis, .. }) = w.wait(20_000) else { panic!() };
        assert_eq!(analysis.method, "dsp");
        assert_eq!(tries.load(Ordering::SeqCst), 0);
        assert_eq!(w.status().neural, "off");
    }

    #[test]
    fn a_latency_job_finds_a_delayed_source() {
        // a source on a 128 bpm clock whose beat 0 is at 0.5 s, arriving 35 ms late
        let bpm = 128.0;
        let audio = click_track(bpm, 0.5 + 0.035, 16.0, SR);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut w = Worker::spawn(oracle_factory(calls, bpm, 0.5 + 0.035), None, Options::default(), true);
        w.submit(Kind::Latency { ref_period: 60.0 / bpm, ref_origin: 0.5 }, "TD-3", audio.clone(), SR, false);
        let Some(Event::Latency { estimate, method, .. }) = w.wait(20_000) else { panic!() };
        let e = estimate.unwrap();
        assert_eq!(method, "neural");
        assert!((e.ms - 35.0).abs() < 4.0, "{e:?}");
        assert!(e.confidence > 0.8, "{e:?}");

        // the same with no models: onsets folded on sixteenths
        let mut w = Worker::spawn(Box::new(|| Err("none".into())), None, Options::default(), true);
        w.submit(Kind::Latency { ref_period: 60.0 / bpm, ref_origin: 0.5 }, "TD-3", audio, SR, false);
        let Some(Event::Latency { estimate, method, .. }) = w.wait(20_000) else { panic!() };
        let e = estimate.unwrap();
        assert_eq!(method, "dsp");
        assert!((e.ms - 35.0).abs() < 8.0, "{e:?}");
    }
}
