//! The browser's Wheel (src/wheel.rs) against the booth's (djhost::wheel, the `wheel` feature): the same program
//! lines, rendered by both, give the same samples, bit for bit, at the voice's output (what the booth's feeder writes
//! into the Windwheel strip's ring; the strip after it is the same arithmetic on both: djrust's DC blocker, trim and
//! fader, which src/wheel.rs copies).
//!
//! The booth renders on a feeder thread into a ring. While the wheel is paused the feeder writes silence without
//! clocking the voice, so every line given before the first play lands on the same voice frame on both, whatever
//! the thread's timing: the lines are given once the ring is full of that silence, and then the wheel is played.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use djhost::wheel::Wheel;
use djweb::wheel::{Args, WheelCore, Winder};

const SR: f64 = 48_000.0;
const START_AT: usize = 2048;
/// What the booth's feeder fills the ring to with no strip mirror: the threshold and one cushion (djhost::wheel CUSHION).
const HELD: usize = START_AT + 512;

/// The booth's wheel, started and holding: its ring full of the paused silence.
struct Booth {
    wheel: Wheel,
    rx: rtrb::Consumer<f32>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Booth {
    fn new() -> Booth {
        let (tx, rx) = rtrb::RingBuffer::<f32>::new(1 << 16);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(SR, tx, START_AT, None, stop.clone()).unwrap();
        let b = Booth { wheel, rx, stop, thread: Some(thread) };
        for _ in 0..2000 {
            if b.rx.slots() >= HELD {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(b.rx.slots(), HELD, "the feeder holds the threshold and a cushion");
        b
    }

    /// The next `n` frames the feeder writes, after the `HELD` frames of silence it wrote before.
    fn take(&mut self, n: usize) -> Vec<f32> {
        let mut got = Vec::with_capacity(HELD + n);
        for _ in 0..20_000 {
            let k = self.rx.slots().min(HELD + n - got.len());
            if k > 0 {
                let c = self.rx.read_chunk(k).unwrap();
                let (a, b) = c.as_slices();
                got.extend_from_slice(a);
                got.extend_from_slice(b);
                c.commit_all();
            }
            if got.len() == HELD + n {
                break;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(got.len(), HELD + n, "the feeder kept up");
        assert!(got[..HELD].iter().all(|x| *x == 0.0), "held: silence");
        got.split_off(HELD)
    }
}

impl Drop for Booth {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The browser's wheel: the two halves, as the page wires them (a wind goes to the voice as bytes).
struct Web {
    winder: Winder,
    core: WheelCore,
}

impl Web {
    fn new() -> Web {
        Web { winder: Winder::new(), core: WheelCore::new(SR) }
    }
    fn line(&mut self, json: &str) {
        let a = Args::parse(json).unwrap();
        if let Some(w) = self.winder.call(&a).unwrap() {
            let w = djweb::wheel::Wound::decode(&w.encode()).unwrap();
            self.core.install(&w);
        }
        self.core.call(&a, |_| 140.0).unwrap();
    }
    fn take(&mut self, n: usize, block: usize) -> Vec<f32> {
        let mut out = vec![0f32; n];
        for c in out.chunks_mut(block) {
            self.core.feed(c);
        }
        out
    }
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<(usize, f32, f32)> {
    a.iter().zip(b).enumerate().find(|(_, (x, y))| x.to_bits() != y.to_bits()).map(|(i, (x, y))| (i, *x, *y))
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0f32, |m, x| m.max(x.abs()))
}

/// hand-dubstep 140 v4's wheel lines, as the intro gives them, then the breakdown's `wheel play`.
#[test]
fn hand_dubstep_v4s_wheel_plays_as_the_booths_does() {
    let mut booth = Booth::new();
    let mut web = Web::new();

    // at bar 1: wheel load preset=cmaj7-hoop
    let l = booth.wheel.load("cmaj7-hoop").unwrap();
    web.line(r#"{"action":"load","preset":"cmaj7-hoop"}"#);
    assert_eq!(web.winder.key(), Some((l.root.clone(), l.transpose)));
    assert_eq!(web.winder.volume_db(), Some(l.volume_db));

    // at bar 3: wheel key root=F (no octave: the current one's)
    let octave: String = l.root.chars().filter(|c| c.is_ascii_digit()).collect();
    let l = booth.wheel.set_key(&format!("F{octave}"), l.transpose).unwrap();
    web.line(r#"{"action":"key","root":"F"}"#);
    assert_eq!(web.winder.key(), Some((l.root.clone(), l.transpose)));

    // at bar 4: wheel sequence notes="C5 Ab4 F4 G4 Ab4 G4 F4 C5"
    let l = booth.wheel.set_sequence("C5 Ab4 F4 G4 Ab4 G4 F4 C5").unwrap();
    web.line(r#"{"action":"sequence","notes":"C5 Ab4 F4 G4 Ab4 G4 F4 C5"}"#);
    assert_eq!(l.sequence, "C5 Ab4 F4 G4 Ab4 G4 F4 C5");

    // at bar 5: wheel motor on=true speed=1.05
    booth.wheel.set_motor(true, 1.05);
    web.line(r#"{"action":"motor","on":true,"speed":1.05}"#);

    // at bar 6: wheel level db=10
    booth.wheel.set_volume(10.0);
    web.line(r#"{"action":"level","db":10}"#);
    let l = booth.wheel.layout().unwrap();
    assert_eq!((l.motor, l.speed_mps, l.volume_db), (true, 1.05, 10.0));
    assert_eq!(web.winder.volume_db(), Some(10.0));

    // breakdown: wheel play (the first play: the motor is already on)
    assert!(booth.wheel.play_pause());
    web.line(r#"{"action":"play"}"#);

    // two seconds: the cord spinning up on its motor and the sequence going round
    let n = 2 * SR as usize;
    let a = booth.take(n);
    let b = web.take(n, 128);
    assert!(peak(&a) > 0.05, "the booth's wheel sounds (peak {})", peak(&a));
    assert_eq!(first_difference(&a, &b), None, "the browser's wheel is the booth's, sample for sample");
}

/// Every instrument of the library: loaded and played (the first play kicks the motor or plucks), the same samples.
#[test]
fn every_instrument_plays_as_the_booths_does() {
    for p in djhost::wheel::presets() {
        let mut booth = Booth::new();
        let mut web = Web::new();
        booth.wheel.load(&p.id).unwrap();
        web.line(&format!(r#"{{"action":"load","preset":"{}"}}"#, p.id));
        booth.wheel.play_pause();
        web.line(r#"{"action":"toggle"}"#);
        let n = SR as usize / 2;
        let a = booth.take(n);
        // an odd block size: the voice is the same however it is asked for
        let b = web.take(n, 97);
        assert_eq!(first_difference(&a, &b), None, "{}", p.id);
    }
}

/// A key or a sequence re-wound while held keeps the motor and the level: the booth's keep-the-trigger rule.
#[test]
fn a_rewind_keeps_the_motor_and_the_level_as_the_booths_does() {
    let mut booth = Booth::new();
    let mut web = Web::new();
    booth.wheel.load("music-box").unwrap();
    web.line(r#"{"action":"load","preset":"music-box"}"#);
    booth.wheel.set_motor(true, 2.5);
    web.line(r#"{"action":"motor","speed":2.5}"#);
    booth.wheel.set_volume(-4.0);
    web.line(r#"{"action":"level","db":-4}"#);
    let (root, t) = web.winder.key().unwrap();
    booth.wheel.set_key(&root, t + 1).unwrap();
    web.line(r#"{"action":"key","transpose":1}"#);
    assert_eq!(booth.wheel.layout().unwrap().transpose, t + 1);
    booth.wheel.play_pause();
    web.line(r#"{"action":"play"}"#);
    let n = SR as usize;
    let a = booth.take(n);
    let b = web.take(n, 128);
    assert!(peak(&a) > 0.01);
    assert_eq!(first_difference(&a, &b), None);
}

/// What the strip adds to the master: the feeder's output through djrust's own DC blocker, trim and fader.
#[test]
fn the_strip_is_djrusts() {
    let mut web = Web::new();
    web.line(r#"{"action":"load","preset":"hoop-looper"}"#);
    web.line(r#"{"action":"play"}"#);
    let mut twin = Web::new();
    twin.line(r#"{"action":"load","preset":"hoop-looper"}"#);
    twin.line(r#"{"action":"play"}"#);
    let n = 4800;
    let mut fed = twin.take(n, 128);
    let fresh = djrust::input::InputSettings::new("Windwheel");
    let mut dc = djrust::input::DcBlock::new(SR, 20.0);
    let trim = djrust::input::gain_of_db(fresh.trim_db) as f32;
    let fader = fresh.volume as f32;
    let mut want = Vec::with_capacity(n);
    for c in fed.chunks_mut(128) {
        dc.process(c);
        want.extend(c.iter().map(|x| fader * (x * trim)));
    }
    let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        web.core.render_add(cl, cr);
    }
    assert_eq!(first_difference(&l, &want), None);
    assert_eq!(l, r, "mono on both sides");
}

/// Paused, the voice is not clocked and nothing is added; played, it fades in over 10 ms.
#[test]
fn pause_holds_and_play_fades_in() {
    let mut web = Web::new();
    web.line(r#"{"action":"load","preset":"hoop-looper"}"#);
    assert!(!web.core.sounding());
    let (mut l, mut r) = (vec![0f32; 128], vec![0f32; 128]);
    web.core.render_add(&mut l, &mut r);
    assert!(l.iter().all(|x| *x == 0.0));
    web.line(r#"{"action":"play"}"#);
    let a = web.take(480, 128);
    assert!(a[0].abs() <= peak(&a[..479]));
    web.line(r#"{"action":"pause"}"#);
    let _ = web.take(1024, 128); // the block in flight, and the fade out
    let held = web.take(480, 128);
    assert!(held.iter().all(|x| *x == 0.0), "held after the fade");
    assert!(web.core.status().contains("\"paused\":true"));
}

/// sync, a rung and a step set the motor as the booth's do (the same target, the same motor setting, the same refusals
/// at the motor's ends), and interval beats reads a deck's tempo in hundredths.
#[test]
fn sync_rung_step_and_interval_set_what_the_booths_do() {
    for id in ["hoop-looper", "cmaj7-hoop", "music-box", "bowed-rope"] {
        let booth = Booth::new();
        let mut web = Web::new();
        booth.wheel.load(id).unwrap();
        web.line(&format!(r#"{{"action":"load","preset":"{id}"}}"#));
        let motor = |b: &Booth| {
            let l = b.wheel.layout().unwrap();
            (l.motor, l.speed_mps)
        };
        let ours = |w: &Web| {
            let s = w.core.status();
            (s.contains("\"motor\":true"), s)
        };
        // the nearest rung
        let b = booth.wheel.sync("A", 140.0);
        let w = web.core.call(&Args::parse(r#"{"action":"sync","deck":"A"}"#).unwrap(), |_| 140.0);
        assert_eq!(b.is_ok(), w.is_ok(), "{id}: sync {b:?} vs {w:?}");
        if let (Ok(b), Ok(w)) = (&b, &w) {
            assert!(w.contains(&format!("\"target_bpm\":{}", b.target_bpm)), "{id}: {w} vs {b:?}");
            assert!(w.contains(&format!("\"motor_mps\":{}", b.motor_mps)), "{id}: {w} vs {b:?}");
        }
        assert_eq!(motor(&booth).0, ours(&web).0, "{id}");
        // ratio=1/4, ratio=2: rungs -3 and 1
        for (ratio, index) in [("1/4", -3), ("2", 1), ("1/2", -1)] {
            let b = booth.wheel.rung("A", 140.0, index);
            let w = web.core.call(&Args::parse(&format!(r#"{{"action":"sync","deck":"A","ratio":"{ratio}"}}"#)).unwrap(), |_| 140.0);
            assert_eq!(b.as_ref().err(), w.as_ref().err(), "{id}: ratio {ratio}");
            if let (Ok(b), Ok(w)) = (&b, &w) {
                assert!(w.contains(&format!("\"motor_mps\":{}", b.motor_mps)), "{id} {ratio}: {w} vs {b:?}");
            }
        }
        // a step up and down
        for up in [true, false] {
            let b = booth.wheel.step("A", 140.0, up);
            let w = web.core.call(&Args::parse(&format!(r#"{{"action":"step","deck":"A","up":{up}}}"#)).unwrap(), |_| 140.0);
            match (&b, &w) {
                (Ok(Some(b)), Ok(w)) => assert!(w.contains(&format!("\"motor_mps\":{}", b.motor_mps)), "{id} step {up}: {w} vs {b:?}"),
                (Ok(None), Ok(w)) => assert_eq!(w, "null"),
                (Err(b), Err(w)) => assert_eq!(b, w),
                _ => panic!("{id} step {up}: {b:?} vs {w:?}"),
            }
            let (on, speed) = motor(&booth);
            let s = web.core.status();
            assert!(s.contains(&format!("\"motor\":{on}")) && s.contains(&format!("\"motor_mps\":{speed}")), "{id}: {s} vs {on} {speed}");
        }
    }
    let mut core = WheelCore::new(SR);
    let mut winder = Winder::new();
    core.install(&winder.call(&Args::parse(r#"{"action":"load","preset":"drop-chimes"}"#).unwrap()).unwrap().unwrap());
    assert!(core.call(&Args::parse(r#"{"action":"sync","deck":"A","ratio":"1/3.5"}"#).unwrap(), |_| 140.0).is_err());
    let r = core.call(&Args::parse(r#"{"action":"interval","beats":"1/2","deck":"A"}"#).unwrap(), |_| 120.0).unwrap();
    assert_eq!(r, "{\"seconds\":0.25}");
    assert!(core.call(&Args::parse(r#"{"action":"interval","beats":"1/2","deck":"B"}"#).unwrap(), |_| 0.0).is_err());
    assert!(core.call(&Args::parse(r#"{"action":"spin"}"#).unwrap(), |_| 0.0).is_err());
}

/// `master: filter lp ...` and `master: reverb ...` reach the wheel as they reach the decks: the decks' FX unit and
/// strip reverb, set alike (at rest both leave it exactly as the strip made it: the_strip_is_djrusts).
#[test]
fn the_masters_filter_and_reverb_reach_the_wheel() {
    let run = |lines: &[(&str, f64)]| {
        let mut web = Web::new();
        web.line(r#"{"action":"load","preset":"hoop-looper"}"#);
        for (k, v) in lines {
            web.core.master(k, *v).unwrap();
        }
        web.line(r#"{"action":"play"}"#);
        let n = SR as usize;
        let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
        for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            web.core.render_add(cl, cr);
        }
        (l, r)
    };
    let energy = |v: &[f32]| v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>();
    // the high half of the spectrum, roughly: first differences
    let hf = |v: &[f32]| v.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>();
    let (open, _) = run(&[]);
    // lp-freq at the bottom of its travel: the low-pass at 100 Hz (MasterParams::lp_freq)
    let (dark, _) = run(&[("lp-freq", 0.0)]);
    assert!(hf(&dark) < 0.5 * hf(&open), "the master's low-pass darkens the wheel: {} vs {}", hf(&dark), hf(&open));
    let (wet_l, wet_r) = run(&[("reverbWet", 0.45)]);
    assert_ne!(wet_l, wet_r, "the reverb is stereo");
    assert!(energy(&wet_l) > 0.0 && wet_l != open);
    assert!(web_rejects_unknown());
}

fn web_rejects_unknown() -> bool {
    let mut c = WheelCore::new(SR);
    c.master("no-such", 0.5).is_err() && c.master("lp-freq", f64::NAN).is_err() && c.master("lp-freq", 1.0).is_ok()
}

