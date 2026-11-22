//! The MIDI clock master's thread (docs/HUB.md, M2): runs a [`ClockEngine`] against real ports.
//!
//! The thread waits on its command channel until [`WAKE_S`] before the next deadline, so a
//! command (a tempo change, Start, a live note) is taken at once. Then it sleeps on the OS's
//! precise timer ([`sleep_until`]: `mach_wait_until` on macOS, `clock_nanosleep` with an
//! absolute deadline on Linux) to a [`SpinMargin`] before the deadline, and spins only that
//! last sliver before it sends, so a tick goes out within a few tens of microseconds of its
//! deadline without burning a core: spinning the whole [`WAKE_S`] cost 7% of a core at 120 BPM
//! and 11% at 174 (`examples/clock_bench.rs`), the sleep brings that to under 1%. The margin
//! follows how late the thread's own sleeps wake (~0.1 ms on macOS with the time-constraint
//! policy; more on a loaded Linux box without `SCHED_FIFO`), so a sloppier timer spins longer
//! instead of sending late. Each output has
//! its own midir connection, opened by port name and retried while the port is missing (hot-
//! plug). Live notes for a clocked device go through the same connection as its clock, in order.
//!
//! Portable: midir on CoreMIDI (macOS) and the ALSA sequencer (Linux, PipeWire's ALSA bridge
//! included). The thread asks for real-time scheduling ([`promote_thread`]): on macOS the
//! time-constraint policy, without which a process macOS treats as background (one started from
//! tmux or launchd) wakes from an 18 ms wait 60-70 ms late (measured on the booth Mac, 2026-11-03); on
//! Linux `SCHED_FIFO`, which needs `rtprio` in limits.conf or rtkit and is skipped when refused
//! (Linux hrtimers wake within ~0.1 ms without it). `tests/clock_loop.rs` measures the result
//! through a virtual port.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use djengine::clock::{ClockEngine, OutputConfig, StepPattern};
use midir::{MidiOutput, MidiOutputConnection};

/// How long before a deadline the thread stops waiting on its command channel (whose timed
/// wait wakes less precisely) and sleeps on the precise timer instead.
pub const WAKE_S: f64 = 0.0015;
/// The shortest spin after the precise sleep, seconds ([`SpinMargin`]).
pub const SPIN_MIN_S: f64 = 0.000_05;

/// How long before a deadline the precise sleep ends and the thread spins: a decaying peak of
/// how late recent sleeps woke, times 1.5, plus 30 us, between [`SPIN_MIN_S`] and [`WAKE_S`]
/// (the old fixed spin, so it never spins longer than before). A late wake raises it at once;
/// it relaxes by ~40% a second at 120 BPM.
#[derive(Clone, Copy, Debug)]
pub struct SpinMargin {
    peak: f64,
}

impl Default for SpinMargin {
    fn default() -> Self {
        // start a little cautious (a 0.33 ms margin) until the thread has seen its own wakes;
        // a late first wake raises it at once
        SpinMargin { peak: 0.000_2 }
    }
}

impl SpinMargin {
    /// Seconds before the deadline to stop sleeping.
    pub fn get(&self) -> f64 {
        (self.peak * 1.5 + 0.000_03).clamp(SPIN_MIN_S, WAKE_S)
    }
    /// Record how late a sleep woke (seconds past the time it was asked for); 0 when there was
    /// no sleep (the margin covered the whole wait), so a margin at its cap still relaxes.
    pub fn record(&mut self, oversleep: f64) {
        let o = if oversleep.is_finite() {
            oversleep.max(0.0)
        } else {
            WAKE_S
        };
        self.peak = o.max(self.peak * 0.99);
    }
}
/// The longest the thread waits for a command when nothing is due.
const IDLE_S: f64 = 0.050;
/// How often a missing output is looked for again.
const RETRY_S: f64 = 1.0;

enum Cmd {
    Bpm(f64),
    Start(f64),
    Stop,
    Outputs(Vec<OutputConfig>),
    Pattern(StepPattern),
    Align(f64),
    ClockAlways(bool),
    Send(String, Vec<u8>),
    /// Send at host time `.2`, tagged `.3` (loop routing: notes ahead of time).
    SendAt(String, Vec<u8>, f64, u64),
    /// Drop every message still queued with this tag.
    Cancel(u64),
    Release(String),
    Quit,
}

/// What the clock thread reports.
#[derive(Clone, Debug, Default)]
pub struct ClockStatus {
    pub running: bool,
    pub bpm: f64,
    /// Messages sent since the thread started.
    pub sent: u64,
    /// Ticks skipped because the thread woke too late.
    pub skipped: u64,
    /// The worst lateness of a send, seconds, since the last [`Clock::reset_stats`].
    pub max_late: f64,
    /// Ports connected now.
    pub connected: Vec<String>,
    /// Ports wanted but not connected (missing, or released for a pattern write).
    pub missing: Vec<String>,
    pub last_error: Option<String>,
    /// The thread got real-time scheduling ([`promote_thread`]).
    pub realtime: bool,
    /// How long before a deadline the thread stops sleeping and spins now, seconds
    /// ([`SpinMargin`]).
    pub spin_margin: f64,
    /// The engine's timing, to read the clock's beat without asking the thread.
    pub timing: Option<Timing>,
}

/// A copy of the engine's tick grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub tick0: f64,
    pub tick0_time: f64,
    pub period: f64,
    pub start_tick: Option<u64>,
}

impl Timing {
    /// Beats since Start at host time `t`.
    pub fn beat_at(&self, t: f64) -> Option<f64> {
        let s = self.start_tick? as f64;
        Some((self.tick0 + (t - self.tick0_time) / self.period - s) / djengine::clock::PPQN as f64)
    }
}

/// Messages to send at a host time (`Clock::send_at`): in time order, same-time ones in the
/// order they came. Kept by the clock thread beside its engine's own schedule.
#[derive(Clone, Debug, Default)]
pub struct TimedQueue {
    /// (time, sequence, port, bytes, tag)
    items: Vec<(f64, u64, String, Vec<u8>, u64)>,
    seq: u64,
}

impl TimedQueue {
    pub fn push(&mut self, port: String, bytes: Vec<u8>, at: f64, tag: u64) {
        self.seq += 1;
        let item = (at, self.seq, port, bytes, tag);
        let k = self
            .items
            .partition_point(|x| (x.0, x.1) <= (item.0, item.1));
        self.items.insert(k, item);
    }
    /// The earliest time queued.
    pub fn next_time(&self) -> Option<f64> {
        self.items.first().map(|x| x.0)
    }
    /// Everything due at `now`, in order: (port, bytes, its time).
    pub fn pop_due(&mut self, now: f64) -> Vec<(String, Vec<u8>, f64)> {
        let k = self.items.partition_point(|x| x.0 <= now);
        self.items.drain(..k).map(|x| (x.2, x.3, x.0)).collect()
    }
    /// Drop what is queued with `tag`.
    pub fn cancel(&mut self, tag: u64) {
        self.items.retain(|x| x.4 != tag);
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The handle the app keeps.
pub struct Clock {
    tx: Sender<Cmd>,
    base: Instant,
    status: Arc<Mutex<ClockStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl Clock {
    /// Start the clock thread at `bpm`. `client` names the midir client.
    pub fn spawn(client: &str, bpm: f64) -> Clock {
        let (tx, rx) = channel::<Cmd>();
        let base = Instant::now();
        let status = Arc::new(Mutex::new(ClockStatus {
            bpm,
            ..Default::default()
        }));
        let st = status.clone();
        let client = client.to_string();
        let thread = std::thread::Builder::new()
            .name("djrust-clock".into())
            .spawn(move || {
                let promoted = promote_thread();
                if let Ok(mut s) = st.lock() {
                    s.realtime = promoted;
                }
                let now = || base.elapsed().as_secs_f64();
                let mut eng = ClockEngine::new(bpm, now());
                let mut conns: BTreeMap<String, MidiOutputConnection> = BTreeMap::new();
                let mut released: Vec<String> = Vec::new();
                let mut last_retry = f64::NEG_INFINITY;
                let mut sent = 0u64;
                let mut max_late = 0f64;
                let mut last_error: Option<String> = None;
                let mut timed = TimedQueue::default();
                let mut margin = SpinMargin::default();
                loop {
                    // (re)connect what is wanted and missing, at most once a second
                    if now() - last_retry > RETRY_S {
                        last_retry = now();
                        for o in eng.outputs() {
                            if o.enabled
                                && !conns.contains_key(&o.port)
                                && !released.contains(&o.port)
                            {
                                match connect(&client, &o.port) {
                                    Ok(c) => {
                                        conns.insert(o.port.clone(), c);
                                    }
                                    Err(e) => last_error = Some(e),
                                }
                            }
                        }
                    }
                    let next = match (eng.next_time(), timed.next_time()) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    let wait = next.map_or(IDLE_S, |t| (t - now() - WAKE_S).clamp(0.0, IDLE_S));
                    match rx.recv_timeout(Duration::from_secs_f64(wait)) {
                        Ok(cmd) => {
                            let t = now();
                            match cmd {
                                Cmd::Bpm(b) => eng.set_bpm(b, t),
                                Cmd::Start(at) => {
                                    eng.start(at);
                                }
                                Cmd::Stop => eng.stop(),
                                Cmd::Outputs(o) => {
                                    let keep: Vec<String> = o
                                        .iter()
                                        .filter(|c| c.enabled)
                                        .map(|c| c.port.clone())
                                        .collect();
                                    conns.retain(|p, _| keep.contains(p));
                                    // setting the outputs again takes back released ports
                                    released.retain(|p| !keep.contains(p));
                                    eng.set_outputs(o, t);
                                    last_retry = f64::NEG_INFINITY;
                                }
                                Cmd::Pattern(p) => eng.set_pattern(p),
                                Cmd::Align(k) => eng.align(k),
                                Cmd::ClockAlways(on) => eng.clock_always = on,
                                Cmd::Send(port, bytes) => {
                                    if !conns.contains_key(&port) && !released.contains(&port) {
                                        if let Ok(c) = connect(&client, &port) {
                                            conns.insert(port.clone(), c);
                                        }
                                    }
                                    match conns.get_mut(&port) {
                                        Some(c) => {
                                            if let Err(e) = c.send(&bytes) {
                                                last_error = Some(format!("{port}: {e}"));
                                                conns.remove(&port);
                                            } else {
                                                sent += 1;
                                            }
                                        }
                                        None => last_error = Some(format!("{port}: not connected")),
                                    }
                                }
                                Cmd::SendAt(port, bytes, at, tag) => {
                                    if !conns.contains_key(&port) && !released.contains(&port) {
                                        if let Ok(c) = connect(&client, &port) {
                                            conns.insert(port.clone(), c);
                                        }
                                    }
                                    timed.push(port, bytes, at, tag);
                                }
                                Cmd::Cancel(tag) => timed.cancel(tag),
                                Cmd::Release(port) => {
                                    conns.remove(&port);
                                    if !released.contains(&port) {
                                        released.push(port);
                                    }
                                }
                                Cmd::Quit => break,
                            }
                            publish(&st, &eng, &conns, &released, sent, max_late, &last_error);
                            set_margin(&st, &margin);
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    let Some(t) = next else { continue };
                    if t - now() > WAKE_S * 2.0 {
                        continue; // woke early (the idle cap): wait again
                    }
                    // the absolute deadline, slept to precisely, then the last sliver spun
                    let wake = t - margin.get();
                    if wake > now() {
                        sleep_until(wake, now);
                        margin.record(now() - wake);
                    } else {
                        margin.record(0.0); // no sleep to judge by: relax towards the floor
                    }
                    while now() < t {
                        std::hint::spin_loop();
                    }
                    let n = now();
                    let outs: Vec<String> = eng.outputs().map(|o| o.port.clone()).collect();
                    for ev in eng.pop_due(n) {
                        let port = &outs[ev.output];
                        if let Some(c) = conns.get_mut(port) {
                            match c.send(&ev.bytes) {
                                Ok(()) => {
                                    sent += 1;
                                    max_late = max_late.max(now() - ev.time);
                                }
                                Err(e) => {
                                    last_error = Some(format!("{port}: {e}"));
                                    conns.remove(port);
                                }
                            }
                        }
                    }
                    for (port, bytes, at) in timed.pop_due(n) {
                        if let Some(c) = conns.get_mut(&port) {
                            match c.send(&bytes) {
                                Ok(()) => {
                                    sent += 1;
                                    max_late = max_late.max(now() - at);
                                }
                                Err(e) => {
                                    last_error = Some(format!("{port}: {e}"));
                                    conns.remove(&port);
                                }
                            }
                        }
                    }
                    publish(&st, &eng, &conns, &released, sent, max_late, &last_error);
                    set_margin(&st, &margin);
                }
                // let go of anything held and stop the receivers on the way out
                eng.stop();
                let outs: Vec<String> = eng.outputs().map(|o| o.port.clone()).collect();
                for ev in eng.pop_due(f64::INFINITY.min(now() + 0.001)) {
                    if let Some(c) = conns.get_mut(&outs[ev.output]) {
                        let _ = c.send(&ev.bytes);
                    }
                }
            })
            .expect("spawn the clock thread");
        Clock {
            tx,
            base,
            status,
            thread: Some(thread),
        }
    }

    /// Host time, seconds, on the clock's own base (what `start_at` and `beat_at` take).
    pub fn now(&self) -> f64 {
        self.base.elapsed().as_secs_f64()
    }

    /// An `Instant` on the clock's time line.
    pub fn time_of(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.base).as_secs_f64()
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    pub fn set_bpm(&self, bpm: f64) {
        self.send(Cmd::Bpm(bpm));
    }
    /// Start on the first tick at or after host time `at`.
    pub fn start_at(&self, at: f64) {
        self.send(Cmd::Start(at));
    }
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }
    pub fn set_outputs(&self, outputs: Vec<OutputConfig>) {
        self.send(Cmd::Outputs(outputs));
    }
    pub fn set_pattern(&self, p: StepPattern) {
        self.send(Cmd::Pattern(p));
    }
    /// Move the clock by `ticks` (bounded per call): the phase lock's correction.
    pub fn align(&self, ticks: f64) {
        self.send(Cmd::Align(ticks));
    }
    pub fn set_clock_always(&self, on: bool) {
        self.send(Cmd::ClockAlways(on));
    }
    /// Send one message to `port` now, through the clock's connection (live notes).
    pub fn send_now(&self, port: &str, bytes: &[u8]) {
        self.send(Cmd::Send(port.to_string(), bytes.to_vec()));
    }
    /// Send one message to `port` at host time `at` ([`now`](Self::now)'s base), on time like
    /// a clock tick, tagged `tag` so [`cancel`](Self::cancel) can take back what has not gone.
    /// A time already past goes at once.
    pub fn send_at(&self, port: &str, bytes: &[u8], at: f64, tag: u64) {
        self.send(Cmd::SendAt(port.to_string(), bytes.to_vec(), at, tag));
    }
    /// Drop every message [`send_at`](Self::send_at) queued with `tag` that has not gone yet.
    pub fn cancel(&self, tag: u64) {
        self.send(Cmd::Cancel(tag));
    }
    /// Close `port` and keep it closed (another process needs it: a pattern write) until the
    /// outputs are set again.
    pub fn release(&self, port: &str) {
        self.send(Cmd::Release(port.to_string()));
    }

    pub fn status(&self) -> ClockStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Beats since Start at host time `t` (None while stopped).
    pub fn beat_at(&self, t: f64) -> Option<f64> {
        let s = self.status();
        if !s.running {
            return None;
        }
        s.timing?.beat_at(t)
    }

    pub fn reset_stats(&self) {
        if let Ok(mut s) = self.status.lock() {
            s.max_late = 0.0;
        }
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn connect(client: &str, port: &str) -> Result<MidiOutputConnection, String> {
    let out = MidiOutput::new(client).map_err(|e| e.to_string())?;
    let p = out
        .ports()
        .into_iter()
        .find(|q| out.port_name(q).ok().as_deref() == Some(port))
        .ok_or_else(|| format!("no MIDI output named {port:?}"))?;
    out.connect(&p, "clock").map_err(|e| e.to_string())
}

fn set_margin(st: &Arc<Mutex<ClockStatus>>, m: &SpinMargin) {
    if let Ok(mut s) = st.lock() {
        s.spin_margin = m.get();
    }
}

fn publish(
    st: &Arc<Mutex<ClockStatus>>,
    eng: &ClockEngine,
    conns: &BTreeMap<String, MidiOutputConnection>,
    released: &[String],
    sent: u64,
    max_late: f64,
    last_error: &Option<String>,
) {
    let Ok(mut s) = st.lock() else { return };
    s.running = eng.is_running();
    s.bpm = eng.bpm();
    s.sent = sent;
    s.skipped = eng.skipped;
    s.max_late = s.max_late.max(max_late);
    s.connected = conns.keys().cloned().collect();
    s.missing = eng
        .outputs()
        .filter(|o| o.enabled && !conns.contains_key(&o.port))
        .map(|o| o.port.clone())
        .chain(released.iter().cloned())
        .collect();
    s.last_error = last_error.clone();
    let t = eng.time_of(0);
    s.timing = Some(Timing {
        tick0: 0.0,
        tick0_time: t,
        period: eng.time_of(1) - t,
        start_tick: eng.start_tick(),
    });
}

/// Sleep until host time `t` on `now`'s time line, on the OS's most precise timer
/// ([`crate::platform::sleep_for`]: an absolute deadline, so a sleep cut short by a signal
/// resumes towards the same instant). A time already past returns at once.
pub fn sleep_until(t: f64, now: impl Fn() -> f64) {
    crate::platform::sleep_for(t - now());
}

/// Real-time scheduling for the calling thread ([`crate::platform::promote_thread`]: the
/// time-constraint policy on macOS, `SCHED_FIFO` on Linux). True if the OS granted it.
pub fn promote_thread() -> bool {
    crate::platform::promote_thread()
}

#[cfg(test)]
mod tests {
    use super::{SpinMargin, TimedQueue, SPIN_MIN_S, WAKE_S};

    #[test]
    fn the_spin_margin_follows_late_wakes_and_relaxes() {
        let mut m = SpinMargin::default();
        assert!(
            (m.get() - 0.000_33).abs() < 1e-9,
            "cautious until it has seen a wake"
        );
        for _ in 0..2000 {
            m.record(0.000_01);
        }
        assert!(m.get() < 0.000_06, "{}", m.get());
        assert!(m.get() >= SPIN_MIN_S);
        m.record(0.000_4); // one sloppy wake: spin longer at once
        assert!((m.get() - 0.000_63).abs() < 1e-9, "{}", m.get());
        m.record(0.5); // a stall: capped at the old full spin
        assert_eq!(m.get(), WAKE_S);
        m.record(f64::NAN);
        assert_eq!(m.get(), WAKE_S);
        // at the cap no sleep happens, so it relaxes on the zeros it is given then
        for _ in 0..1500 {
            m.record(0.0);
        }
        assert_eq!(m.get(), SPIN_MIN_S);
    }

    #[test]
    fn the_timed_queue_sends_in_time_order_and_cancels_by_tag() {
        let mut q = TimedQueue::default();
        q.push("td3".into(), vec![0x90, 60, 100], 1.0, 7);
        q.push("td3".into(), vec![0x80, 60, 0], 1.5, 7);
        q.push("pd".into(), vec![0x99, 36, 100], 1.0, 8);
        q.push("td3".into(), vec![0x90, 62, 100], 0.5, 7);
        assert_eq!(q.next_time(), Some(0.5));
        let due = q.pop_due(1.0);
        let notes: Vec<u8> = due.iter().map(|d| d.1[1]).collect();
        // same time: in the order they came
        assert_eq!(notes, [62, 60, 36]);
        q.push("td3".into(), vec![0x90, 64, 100], 2.0, 7);
        q.cancel(7);
        assert!(q.is_empty());
        assert!(q.pop_due(10.0).is_empty());
    }
}
