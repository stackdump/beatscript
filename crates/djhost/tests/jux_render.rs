//! `jux` is heard: the part plays on the left and its right copy on the right, in the offline render, and the live
//! runtime hands the engine the same envelope the render loads (docs/BTW.md "Stereo").

use djengine::json::JsonValue;
use djhost::btw_render;
use djlive::runtime::{Grid, Live, LiveHost};

const SR: f64 = 22_050.0;

fn compile(src: &str) -> djlive::compile::Compiled {
    let c = djlive::check(src);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics);
    c.compiled.unwrap()
}

/// One melody note on beat 1 of the left copy; `jux rot 2` plays the same note on beat 3 of the right one. The first section is
/// plain (one copy, in the middle), the second has the pair; `CHAIN` is what the line says.
fn song(chain: &str) -> String {
    format!("tempo 120\nkey am\nlet lead = c4 ~ ~ ~\nsection s 2 bars {{\n  deck A: melody lead\n}}\narrange {{\n  s\n  ~ lead {chain}\n  s\n}}\n")
}

/// The energy of `x[from..to]` (seconds), in dB.
fn db(x: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * SR) as usize, (to * SR) as usize);
    let e: f64 = x[a..b].iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / (b - a) as f64;
    10.0 * e.max(1e-20).log10()
}

fn correlation(l: &[f32], r: &[f32]) -> f64 {
    let (mut lr, mut ll, mut rr) = (0.0, 0.0, 0.0);
    for (a, b) in l.iter().zip(r) {
        let (a, b) = (f64::from(*a), f64::from(*b));
        lr += a * b;
        ll += a * a;
        rr += b * b;
    }
    lr / (ll * rr).sqrt().max(1e-30)
}

/// (left, right) of the render of `src`.
fn render(src: &str) -> (Vec<f32>, Vec<f32>) {
    let c = compile(src);
    let a = btw_render::render(&c, SR, false).expect("render");
    assert!(a.refused.is_empty(), "the engine refused: {:?}", a.refused);
    (a.audio.left, a.audio.right)
}

// bar = 2 s at 120 BPM: section 1 is 0-4 s, section 2 4-8 s; beat 1 of its first bar is 4.0 s, beat 3 is 5.0 s. The first 45 ms of
// a note are its own sound: after that the room (the reverb is stereo and takes a send from both copies) is on both sides.
const PLAIN: (f64, f64) = (0.0, 0.045);
const BEAT1: (f64, f64) = (4.0, 4.045);
const BEAT3: (f64, f64) = (5.0, 5.045);

#[test]
fn the_right_channel_differs_from_the_left_and_each_ear_hears_its_own_copy() {
    let (l, r) = render(&song("jux rot 2"));
    let (l1, r1) = (db(&l, BEAT1.0, BEAT1.1), db(&r, BEAT1.0, BEAT1.1));
    let (l3, r3) = (db(&l, BEAT3.0, BEAT3.1), db(&r, BEAT3.0, BEAT3.1));
    eprintln!("beat 1: L {l1:.1} dB R {r1:.1} dB   beat 3: L {l3:.1} dB R {r3:.1} dB");
    // the left copy plays beat 1 on the left only, the right copy plays beat 3 on the right only
    assert!(l1 - r1 >= 30.0, "beat 1 is on the left: L {l1:.1} R {r1:.1}");
    assert!(r3 - l3 >= 30.0, "beat 3 is on the right: L {l3:.1} R {r3:.1}");
    // over the pair's section the channels are different signals (the room gives each a little of the other's)
    let (a, b) = ((4.0 * SR) as usize, (8.0 * SR) as usize);
    let c = correlation(&l[a..b], &r[a..b]);
    eprintln!("correlation of the pair's section: {c:.3}");
    assert!(c < 0.5, "the channels are not one signal: correlation {c:.2}");
    // the room is a tail, not the note: 100 ms on, the far channel is more than 15 dB under the near one
    let (l1t, r1t) = (db(&l, 4.1, 4.4), db(&r, 4.1, 4.4));
    assert!(l1t - r1t >= 15.0, "the tail of beat 1: L {l1t:.1} R {r1t:.1}");
    // before the jux the part is one copy in the middle: the channels are the same
    let (l0, r0) = (db(&l, PLAIN.0, PLAIN.1), db(&r, PLAIN.0, PLAIN.1));
    assert!((l0 - r0).abs() < 1.0, "L {l0:.1} R {r0:.1}");
    // and each ear hears its copy at the level the part had: the loud channel is the part's, not louder
    assert!((l1 - l0).abs() < 1.5, "beat 1 on the left {l1:.1} dB against the plain part's {l0:.1}");
    assert!((r3 - l0).abs() < 1.5, "beat 3 on the right {r3:.1} dB against the plain part's {l0:.1}");
}

#[test]
fn a_pair_of_equal_copies_is_the_part_in_the_middle() {
    // `jux id`: both copies are the pattern, one on each side: the same as the part unpanned, in each channel
    let (l, r) = render(&song("jux id"));
    let (l0, r0) = (db(&l, PLAIN.0, PLAIN.1), db(&r, PLAIN.0, PLAIN.1));
    let (l1, r1) = (db(&l, BEAT1.0, BEAT1.1), db(&r, BEAT1.0, BEAT1.1));
    assert!((l1 - l0).abs() < 0.5 && (r1 - r0).abs() < 0.5, "section 1: {l0:.1} {r0:.1}; section 2: {l1:.1} {r1:.1}");
    let (a, b) = ((4.0 * SR) as usize, (8.0 * SR) as usize);
    // the control: the same program with no pair at all (`rot 2` alone). The room (a stereo reverb) decorrelates a part's channels a
    // little whatever is done to it; the pair of equal copies has the same channels as the part, to within a hair
    let (pl, pr) = render(&song("rot 2"));
    let plain = correlation(&pl[a..b], &pr[a..b]);
    let pair = correlation(&l[a..b], &r[a..b]);
    eprintln!("correlation of the channels: the part {plain:.4}, a pair of equal copies {pair:.4}");
    assert!(plain > 0.9, "no jux: one signal in both channels, but for the room ({plain:.3})");
    assert!((pair - plain).abs() < 0.04, "a pair of equal copies is the part: {pair:.3} against {plain:.3}");
    assert!((db(&pl, BEAT3.0, BEAT3.1) - db(&pr, BEAT3.0, BEAT3.1)).abs() < 1.0);
}

/// A fake host that keeps every envelope it is handed.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: bool,
    loads: Vec<(String, JsonValue)>,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        self.playing.then(|| Grid { deck: "A".into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, _deck: &str) -> bool {
        self.playing
    }
    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        self.loads.push((deck.into(), env.clone()));
        Ok(())
    }
    fn set_playing(&mut self, _deck: &str, on: bool) {
        self.playing = on;
    }
    fn call(&mut self, _noun: &str, _params: &JsonValue) -> Result<JsonValue, String> {
        Ok(JsonValue::Null)
    }
}

#[test]
fn live_and_the_offline_render_load_the_same_envelope() {
    let src = song("jux rot 2");
    let c = compile(&src);
    let mut h = Fake { playing: true, beat: 8.0, ..Fake::default() };
    let mut live = Live::new();
    assert!(live.eval(&src).ok);
    for _ in 0..(4 * 4 * 4) {
        h.beat += 0.25;
        live.service(&mut h);
    }
    // the render loads `btw_tempo::envelope(ir, tempo)` of each section's net; the live runtime loads `ir.envelope(phase)`
    let offline: Vec<String> = c.segments.iter().map(|s| djhost::btw_tempo::envelope(&s.decks[&djlive::ast::Deck::A], None).canonical_js()).collect();
    let live_loaded: Vec<String> = h.loads.iter().map(|(_, e)| e.canonical_js()).collect();
    assert!(live_loaded.len() >= 2, "{} loads", live_loaded.len());
    // the plain section's net, then the pair's: each is among what the render loads, byte for byte
    for e in &live_loaded {
        assert!(offline.contains(e), "live sent an envelope the render does not load: {e}");
    }
    assert!(live_loaded.iter().any(|e| e.contains("melody_r") && e.contains("\"pan\":1")), "the pair's envelope reached the host");
    assert!(live_loaded.iter().any(|e| !e.contains("melody_r") && !e.contains("\"pan\"")), "and the plain one before it has none");
}
