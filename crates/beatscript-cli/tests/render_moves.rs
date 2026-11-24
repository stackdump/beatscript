//! The DJ moves in the streaming render (`djhost::offline_moves`): a `move` line the control net does not play is armed on
//! the booth's own scheduler (`djhost::perform::Moves`) and serviced on the render's clock, so `beatscript render` and `beatscript listen`
//! hear it. `filter_exit` and `echo_out` stay on the control net, which played them before: a program with one renders
//! byte for byte as it did, and so does a program with no move at all (the pins below were taken before the change).
//!
//! The render and listen are included by path, as in `render_inproc.rs`.

#![allow(dead_code)]

#[path = "../src/render.rs"]
mod render;

#[path = "../src/listen.rs"]
mod listen;

use std::time::Instant;

use djengine::json::JsonValue;
use render::{Events, Format, Options};

const SR: u32 = 22_050;

struct Out {
    pcm: Vec<u8>,
    notes: String,
}

fn render(text: &str) -> Out {
    let o = Options { sr: SR, format: Format::S16le, ..Options::default() };
    let mut pcm: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    {
        let mut ev = Events::new(&mut err);
        render::render_to(text, &o, &mut pcm, &mut ev, Instant::now()).unwrap_or_else(|e| panic!("{text}: {}", e.message));
    }
    let start = String::from_utf8(err).unwrap().lines().map(|l| JsonValue::parse(l).unwrap()).find(|l| l.get("event").and_then(JsonValue::as_str) == Some("start")).expect("a start event");
    Out { pcm, notes: start.get("notes").map(|n| n.js_stringified()).unwrap_or_default() }
}

fn hex(b: &[u8]) -> String {
    djengine::cid::sha256(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// The s16le stream as mono floats (the channels' mean).
fn mono(pcm: &[u8]) -> Vec<f64> {
    pcm.chunks_exact(4).map(|c| (f64::from(i16::from_le_bytes([c[0], c[1]])) + f64::from(i16::from_le_bytes([c[2], c[3]]))) / 65536.0).collect()
}

/// Frames of `beats` beats at `bpm`.
fn at(beats: f64, bpm: f64) -> usize {
    (beats * 60.0 / bpm * f64::from(SR)).round() as usize
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// RMS of bar `bar` (its middle half, clear of the bar lines), at `bpm`.
fn bar_rms(x: &[f64], bpm: f64, bar: f64) -> f64 {
    let (a, b) = (at(bar * 4.0 + 1.0, bpm), at(bar * 4.0 + 3.0, bpm));
    rms(&x[a..b.min(x.len())])
}

/// The magnitude of `hz` in `x[a..b]` (Goertzel), over the length.
fn tone(x: &[f64], a: usize, b: usize, hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / f64::from(SR);
    let (c, mut s1, mut s2) = (2.0 * w.cos(), 0.0, 0.0);
    for v in &x[a..b] {
        let s0 = v + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / (b - a) as f64
}

// ---- pins: taken before the moves were played, so the change is proved to leave these renders alone

/// `examples/beatscript/render-controls.beatscript` (no move: a fade, an lfo, a crossfader move, a mute hold, a filter sweep), `beatscript render`
/// at 22.05 kHz s16le.
const PIN_NO_MOVE: &str = "899ffd161733aa5d79d58691c75c96c6b8ca22c69af269cdc4ef3fabc76b2eec";
/// A program whose `filter_exit` the control net plays (it played it before the change, and still does).
const PIN_NET_FILTER_EXIT: &str = "fcbb6f097ef253fad42dfc561c7091b90ab14d776e8cbd544ddf0f4be74ab131";
const NET_FILTER_EXIT: &str = "tempo 120\nsection main 10 bars {\n  deck A: hihat x*16 | bass c2 c2 c2 c2 | kick x ~ x ~\n  at bar 2: control: move filter_exit on A over 2 bars\n}\narrange main\n";

fn example(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript").join(name)).unwrap()
}

#[test]
fn pin_a_program_with_no_move_renders_as_it_did() {
    let r = render(&example("render-controls.beatscript"));
    assert_eq!(hex(&r.pcm), PIN_NO_MOVE, "render-controls.beatscript renders differently");
}

#[test]
fn pin_a_filter_exit_the_net_plays_renders_as_it_did() {
    let r = render(NET_FILTER_EXIT);
    assert_eq!(hex(&r.pcm), PIN_NET_FILTER_EXIT, "the net's filter exit renders differently");
    // and it is the move: cut on bar 2, back on the phrase line (bar 8)
    let x = mono(&r.pcm);
    let (before, cut, back) = (bar_rms(&x, 120.0, 1.0), bar_rms(&x, 120.0, 5.0), bar_rms(&x, 120.0, 9.0));
    assert!(before > 0.01 && cut < before * 0.05 && back > before * 0.5, "{before} {cut} {back}");
}

// ---- the moves the scheduler plays

/// Deck A plays a different note on each beat (c4 e4 g4 c5, a bar), so a loop shows in which notes each beat plays.
const ROLL: &str = "tempo 120\nsection main 6 bars {\n  deck A: melody c4 e4 g4 c5 len=4\n  control: move loop_roll on A over 1 bars\n}\narrange main\n";

/// The note of the bar's melody (0 c4, 1 e4, 2 g4, 3 c5) whose fundamental is loudest in `x[a..b]`.
fn note_in(x: &[f64], a: usize, b: usize) -> usize {
    let hz = [261.63, 329.63, 392.0, 523.25];
    (0..4).max_by(|&i, &j| tone(x, a, b, hz[i]).total_cmp(&tone(x, a, b, hz[j]))).unwrap()
}

/// The notes of bar `bar`'s beats (or of its half beats), clear of the onsets.
fn notes_of(x: &[f64], bar: usize, per_beat: usize) -> Vec<usize> {
    let len = at(1.0, 120.0) / per_beat;
    (0..4 * per_beat).map(|k| at(bar as f64 * 4.0, 120.0) + k * len).map(|s| note_in(x, s + len / 8, s + len - len / 8)).collect()
}

/// Deck A is a net deck: the roll is its own loop (the engine's net loop), 4 → 2 → 1 → ½ beat from the bar after the line
/// that armed it, a bar a stage, so bar 1 plays as written, bar 2 its first two beats twice, bar 3 its first beat four times,
/// bar 4 the first half of that beat eight times, and the drop (bar 5) lets go. (Before the engine had a net loop this was
/// the stutter's beat repeat, 1 → ½ → ¼ → ⅛ beat, which made bar 1 its first beat four times.)
#[test]
fn a_loop_roll_on_a_net_deck_loops_four_two_one_half_and_lets_go_on_the_drop() {
    let plain_text = ROLL.replace("  control: move loop_roll on A over 1 bars\n", "");
    let (a, b, plain) = (render(ROLL), render(ROLL), render(&plain_text));
    assert!(a.pcm == b.pcm, "two renders of one program differ");
    assert_eq!(a.pcm.len(), plain.pcm.len());
    assert!(!a.notes.contains("not in this render"), "the roll is in the render: {}", a.notes);
    let (x, y) = (mono(&a.pcm), mono(&plain.pcm));
    // nothing moves before the roll lands (armed on bar 0, it lands on bar 1)
    let bar1 = at(4.0, 120.0);
    assert!(x[..bar1 - 64] == y[..bar1 - 64], "bar 0 is the program's own");
    let roll: Vec<Vec<usize>> = (0..6).map(|bar| notes_of(&x, bar, 1)).collect();
    let as_written: Vec<Vec<usize>> = (0..6).map(|bar| notes_of(&y, bar, 1)).collect();
    eprintln!("roll notes by bar {roll:?}\n  plain           {as_written:?}\n  bar 4 by half beat {:?}", notes_of(&x, 4, 2));
    for bar in &as_written {
        assert_eq!(bar, &[0, 1, 2, 3], "the plain bars play c4 e4 g4 c5");
    }
    assert_eq!(roll[1], [0, 1, 2, 3], "the 4-beat stage's first pass is the bar as written");
    assert_eq!(roll[2], [0, 1, 0, 1], "2 beats, twice: the loop starts on the bar line");
    assert_eq!(roll[3], [0, 0, 0, 0], "1 beat, four times");
    assert_eq!(notes_of(&x, 4, 2), [0; 8], "half a beat, eight times");
    // (the windows themselves, 16, 8, 4 and 2 steps, are djhost's tests/perform_roll.rs; a net's notes are played again on
    // each pass, not its audio, so the passes match note for note rather than sample for sample)
    assert_eq!(roll[5], [0, 1, 2, 3], "released on the drop: the deck plays on from the bar line, in phase");
    assert!(bar_rms(&x, 120.0, 5.0) > 0.5 * bar_rms(&y, 120.0, 5.0), "released on the drop");
}

/// A macro and a move in one program: the macro (the control net's `effects fire`, on B) and the roll (the moves' scheduler,
/// on A) share the render's timeline, and the step it takes is limited by both.
const MACRO_AND_ROLL: &str = "tempo 120\nsection main 6 bars {\n  deck A: melody c4 e4 g4 c5 len=4\n  deck B: kick x ~ x ~ | bass c2 ~ c2 ~\n  at bar 1: control: macro filter on B over 2 bars\n  control: move loop_roll on A over 1 bars\n}\narrange main\n";

#[test]
fn a_program_with_a_macro_and_a_move_renders_the_same_twice_and_plays_both() {
    let run = |text: &str| {
        let o = Options { sr: SR, format: Format::S16le, ..Options::default() };
        let (mut pcm, mut err) = (Vec::new(), Vec::new());
        let s = render::render_to(text, &o, &mut pcm, &mut Events::new(&mut err), Instant::now()).unwrap_or_else(|e| panic!("{}", e.message));
        (pcm, s)
    };
    let ((a, s), (b, _)) = (run(MACRO_AND_ROLL), run(MACRO_AND_ROLL));
    assert_eq!(hex(&a), hex(&b), "two renders of a macro and a move differ");
    assert_eq!(s.macros.len(), 1, "{:?}", s.macros);
    assert!(s.macros[0].refused.is_none(), "{:?}", s.macros[0]);
    let (no_roll, _) = run(&MACRO_AND_ROLL.replace("  control: move loop_roll on A over 1 bars\n", ""));
    let (no_macro, _) = run(&MACRO_AND_ROLL.replace("  at bar 1: control: macro filter on B over 2 bars\n", ""));
    assert!(a != no_roll && a != no_macro, "both lines are heard");
    // the roll is the roll beside the macro: its 1-beat stage (bar 3) plays the bar's first beat four times
    assert_eq!(notes_of(&mono(&a), 3, 1), [0, 0, 0, 0]);
}

/// Two decks with a low note each (A: a1, 55 Hz; B: e2, 82.4 Hz): the bass swap cuts B's lows as it is armed and swaps the
/// lows on the line (a 2-bar "phrase": bar 2), A's cut and B's back.
const SWAP: &str = "tempo 120\nsection main 4 bars {\n  deck A: bass a1 a1 a1 a1 len=4\n  deck B: bass e2 e2 e2 e2 len=4\n  control: move bass_swap on B over 2 bars\n}\narrange main\n";

#[test]
fn a_bass_swap_swaps_the_lows_of_two_decks_on_its_line() {
    let plain_text = SWAP.replace("  control: move bass_swap on B over 2 bars\n", "");
    let (a, b, plain) = (render(SWAP), render(SWAP), render(&plain_text));
    assert!(a.pcm == b.pcm);
    let (x, y) = (mono(&a.pcm), mono(&plain.pcm));
    let bar = |v: &[f64], k: f64, hz: f64| tone(v, at(k * 4.0 + 0.5, 120.0), at(k * 4.0 + 3.5, 120.0), hz);
    let (a_lo, b_lo) = (55.0, 82.41);
    eprintln!(
        "bar 1: A {:.5} B {:.5} (plain A {:.5} B {:.5}); bar 3: A {:.5} B {:.5} (plain A {:.5} B {:.5})",
        bar(&x, 1.0, a_lo),
        bar(&x, 1.0, b_lo),
        bar(&y, 1.0, a_lo),
        bar(&y, 1.0, b_lo),
        bar(&x, 3.0, a_lo),
        bar(&x, 3.0, b_lo),
        bar(&y, 3.0, a_lo),
        bar(&y, 3.0, b_lo)
    );
    // before the line: A's lows as they were, B's cut
    assert!((bar(&x, 1.0, a_lo) / bar(&y, 1.0, a_lo) - 1.0).abs() < 0.2, "A's lows untouched before the line");
    assert!(bar(&x, 1.0, b_lo) < 0.4 * bar(&y, 1.0, b_lo), "B's lows cut from the arm");
    // after it: swapped
    assert!(bar(&x, 3.0, a_lo) < 0.4 * bar(&y, 3.0, a_lo), "A's lows cut on the line");
    assert!((bar(&x, 3.0, b_lo) / bar(&y, 3.0, b_lo) - 1.0).abs() < 0.2, "B's lows back on the line");
}

/// A filter exit with `for N bars` (the control net takes no hold on a move, so the scheduler plays it): armed on bar 0, the
/// sweep on bar 1, the cut on bar 2, and the hold's end on bar 5 unmutes the strip.
const HELD_EXIT: &str = "tempo 120\nsection main 7 bars {\n  deck A: hihat x*16 | bass c2 c2 c2 c2 | kick x ~ x ~\n  at bar 0 for 5 bars: control: move filter_exit on A over 1 bars\n}\narrange main\n";

#[test]
fn a_held_filter_exit_cuts_and_its_hold_puts_the_strip_back() {
    let a = render(HELD_EXIT);
    assert!(a.pcm == render(HELD_EXIT).pcm);
    assert!(!a.notes.contains("not in this render"), "{}", a.notes);
    let x = mono(&a.pcm);
    let (open, cut, cut2, back) = (bar_rms(&x, 120.0, 0.0), bar_rms(&x, 120.0, 2.0), bar_rms(&x, 120.0, 4.0), bar_rms(&x, 120.0, 6.0));
    eprintln!("held filter exit: bar 0 {open:.4}, bars 2 and 4 {cut:.4} {cut2:.4}, bar 6 {back:.4}");
    assert!(open > 0.01 && cut < open * 0.05 && cut2 < open * 0.05, "cut on bar 2 until the hold ends");
    assert!(back > open * 0.5, "the hold put it back on bar 5");
}

/// An echo out whose line has a performed guard (beside the net; read at 1/2, where `held(key)` holds): the wash on bar 1, the
/// strip cut a bar later, and put back on the first phrase line after the move ended (bar 8), as the runtime does.
const GUARDED_ECHO: &str = "tempo 120\nsection main 10 bars {\n  deck A: hihat x*16 | bass c2 c2 c2 c2 | kick x ~ x ~\n  control: move echo_out on A over 1 bars when held(key)\n}\narrange main\n";

#[test]
fn an_echo_out_is_put_back_on_the_phrase_line_after_it() {
    let a = render(GUARDED_ECHO);
    let x = mono(&a.pcm);
    let r: Vec<f64> = (0..10).map(|b| bar_rms(&x, 120.0, b as f64)).collect();
    eprintln!("echo out, rms by bar: {r:.4?}");
    assert!(r[0] > 0.01);
    for (b, v) in r.iter().enumerate().take(8).skip(3) {
        assert!(*v < r[0] * 0.05, "bar {b} is cut until the phrase line: {r:?}");
    }
    assert!(r[9] > r[0] * 0.5, "back on bar 8: {r:?}");
}

/// The moves that cannot act offline are not armed, and the render's notes say which and why.
#[test]
fn a_move_that_cannot_act_offline_is_named_with_why() {
    let text = "tempo 120\nsection main 2 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  control: move vocoder_handoff on A\n  control: move phrase_sync on B\n}\narrange main\n";
    let r = render(text);
    assert!(r.notes.contains("not in this render: 2 control lines"), "{}", r.notes);
    assert!(r.notes.contains("no vocoder") && r.notes.contains("not a seed"), "{}", r.notes);
    let plain = render("tempo 120\nsection main 2 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n}\narrange main\n");
    assert!(r.pcm == plain.pcm, "nothing is faked: the render is the program without them");
}

// ---- listen hears them

fn listen_of(text: &str) -> JsonValue {
    let lo = listen::ListenOptions { budget_seconds: 0.0, ..listen::ListenOptions::default() };
    let mut v = listen::listen(text, &lo).unwrap_or_else(|e| panic!("{}", e.message));
    if let JsonValue::Object(o) = &mut v {
        o.remove("ms");
    }
    v
}

fn curve(v: &JsonValue) -> Vec<Option<f64>> {
    v.get("bar_lufs").and_then(JsonValue::as_array).unwrap().iter().map(JsonValue::as_f64).collect()
}

/// `examples/beatscript/render-controls.beatscript` (no move): listen's answer (less its timings), pinned before the change.
/// Re-pinned after the rename for the program's CID alone: with the old CID put back, the answer is the old pin's
/// (2e42b7cdcd8cf5ea4a35c9ccef307279a5bbbf295f8e7780119b8cd3675ff74f).
const PIN_LISTEN_NO_MOVE: &str = "bb45e0083745e3d0c3446c4331ba4b80672b47c47321e69625a420c35601a73f";

#[test]
fn pin_listen_of_a_program_with_no_move_is_as_it_was() {
    let v = listen_of(&example("render-controls.beatscript"));
    assert_eq!(hex(v.canonical_js().as_bytes()), PIN_LISTEN_NO_MOVE, "{}", v.canonical_js());
}

#[test]
fn listen_hears_the_moves_in_the_bars_they_act_on() {
    // the held filter exit: deck A is heard in bar 1, muted from bar 3 to bar 5 (listen says so, and why), back after
    let v = listen_of(HELD_EXIT);
    let plain = listen_of(&HELD_EXIT.replace("  at bar 0 for 5 bars: control: move filter_exit on A over 1 bars\n", ""));
    let (c, p) = (curve(&v), curve(&plain));
    eprintln!("held exit bar_lufs {c:?}\n  plain           {p:?}\n  silent {}", v.get("silent").unwrap().js_stringified());
    assert_eq!(c[0], p[0], "bar 1 (before the move lands) is the plain program's");
    assert!(c[2].is_none_or(|l| l < p[2].unwrap() - 20.0), "bar 3 is cut: {c:?} against {p:?}");
    let silent = v.get("silent").and_then(JsonValue::as_array).unwrap();
    assert!(silent.iter().any(|s| s.get("deck").and_then(JsonValue::as_str) == Some("A") && s.get("reason").and_then(JsonValue::as_str) == Some("mute")), "{}", v.get("silent").unwrap().js_stringified());
    assert!(plain.get("silent").and_then(JsonValue::as_array).unwrap().is_empty());
    // the bass swap: both decks' numbers move (each loses its lows for half the program). The RMS shows it; the loudness
    // (K-weighted, which counts a sub bass for little) barely does
    let v = listen_of(SWAP);
    let plain = listen_of(&SWAP.replace("  control: move bass_swap on B over 2 bars\n", ""));
    let num = |v: &JsonValue, d: &str, k: &str| v.get("decks").and_then(|x| x.get(d)).and_then(|x| x.get(k)).and_then(JsonValue::as_f64).unwrap();
    eprintln!("bass swap rms_db: A {} / {}, B {} / {}; lufs: A {} / {}, B {} / {}", num(&v, "A", "rms_db"), num(&plain, "A", "rms_db"), num(&v, "B", "rms_db"), num(&plain, "B", "rms_db"), num(&v, "A", "lufs"), num(&plain, "A", "lufs"), num(&v, "B", "lufs"), num(&plain, "B", "lufs"));
    for d in ["A", "B"] {
        assert!(num(&v, d, "rms_db") < num(&plain, d, "rms_db") - 1.0, "deck {d} loses its lows for half the song");
        assert!(num(&v, d, "lufs") <= num(&plain, d, "lufs"));
    }
    // the roll: the bars it acts on move, the bar before it does not
    let v = listen_of(ROLL);
    let plain = listen_of(&ROLL.replace("  control: move loop_roll on A over 1 bars\n", ""));
    let (c, p) = (curve(&v), curve(&plain));
    eprintln!("roll bar_lufs {c:?}\n  plain      {p:?}");
    assert_eq!(c[0], p[0]);
    assert!((1..5).any(|b| c[b] != p[b]), "the roll's bars move: {c:?} against {p:?}");
}
