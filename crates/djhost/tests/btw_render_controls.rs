//! The offline render carries the control lines the arrangement format has an op for (docs/btw/ext-render.md): plain values, ramps
//! of every shape, lfos and holds on the mixer's knobs, at the beats the program says (under a tempo curve too), equal to what the
//! runtime sends at the same beats, and it says which lines it has no op for.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeOp, Arrangement};
use djhost::btw_render::{arrangement, arrangement_report, runtime_controls, SAMPLE_BEATS};
use djhost::deck::offline::{render_arrangement, ArrangeRenderOptions};
use djlive::compile::Compiled;
use djlive::env::Env;
use djlive::rat::Rat;

const RAMP_EPS: f64 = djlive::runtime::RAMP_EPS;

fn compile(src: &str) -> Compiled {
    let c = djlive::check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn arr_of(c: &Compiled) -> (Arrangement, f64) {
    let (j, beats) = arrangement(c);
    let (a, bad) = Arrangement::from_json(&j).expect("an arrangement");
    assert_eq!(bad, 0, "every event reads back");
    (a, beats)
}

/// One mixer action of the render: the beat, what it moves (`A volume`, `A eqLow`, `B mute`, `master`, `crossfader`), the value.
#[derive(Clone, Debug, PartialEq)]
struct Move {
    at: f64,
    knob: String,
    value: f64,
}

fn moves_of(a: &Arrangement) -> Vec<Move> {
    a.events
        .iter()
        .filter(|e| e.via == "btw")
        .filter_map(|e| match &e.op {
            ArrangeOp::Deck(DeckAction::Param { deck, name, value }) if name != "pitch" => Some(Move { at: e.at, knob: format!("{deck} {name}"), value: *value }),
            ArrangeOp::Deck(DeckAction::Master(v)) => Some(Move { at: e.at, knob: "master".into(), value: *v }),
            ArrangeOp::Deck(DeckAction::Crossfader(v)) => Some(Move { at: e.at, knob: "crossfader".into(), value: *v }),
            _ => None,
        })
        .collect()
}

/// The mixer moves of a program's render.
fn moves(src: &str) -> Vec<Move> {
    moves_of(&arr_of(&compile(src)).0)
}

/// The moves of one knob, as (beat, value).
fn knob(m: &[Move], name: &str) -> Vec<(f64, f64)> {
    m.iter().filter(|x| x.knob == name).map(|x| (x.at, x.value)).collect()
}

/// What a knob holds at `beat`: its last move at or before it.
fn held(m: &[(f64, f64)], beat: f64) -> Option<f64> {
    m.iter().rev().find(|(at, _)| *at <= beat + 1e-9).map(|(_, v)| *v)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

/// One section of `bars` bars on deck A (and B), with `lines` inside it.
fn song(bars: u32, lines: &str) -> String {
    format!("tempo 120\nsection one {bars} bars {{\n  deck A: kick x ~ x ~\n  deck B: bass c2\n{lines}}}\narrange one\n")
}

// ---------------------------------------------------------------------------------------------------------------------
// plain values, mute, play and stop
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_plain_value_is_an_event_at_its_beat_for_every_knob_the_arrangement_has() {
    let m = moves(&song(
        4,
        "  at bar 1: deck A: level 0.6\n  at bar 1: deck B: low 0.3\n  at bar 1: deck B: mid 0.4\n  at bar 1: deck B: high 0.9\n  at bar 2: master: level 0.7\n  at bar 2 beat 2: crossfader 0.25\n  at bar 3: deck B: mute\n  at bar 3 beat 2: deck B: unmute\n",
    ));
    let want = [(4.0, "A volume", 0.6), (4.0, "B eqLow", 0.3), (4.0, "B eqMid", 0.4), (4.0, "B eqHigh", 0.9), (8.0, "master", 0.7), (10.0, "crossfader", 0.25), (12.0, "B mute", 1.0), (14.0, "B mute", 0.0)];
    assert_eq!(m.len(), want.len(), "{m:?}");
    for (got, (at, knob, v)) in m.iter().zip(want) {
        assert!(near(got.at, at) && got.knob == knob && near(got.value, v), "{got:?} is not {at} {knob} {v}");
    }
}

#[test]
fn play_and_stop_are_the_decks_transport() {
    let c = compile(&song(4, "  at bar 1: deck A: stop\n  at bar 2: deck A: play\n"));
    let (a, _) = arr_of(&c);
    let plays: Vec<(f64, bool)> = a
        .events
        .iter()
        .filter(|e| e.text.contains("deck A: stop") || e.text.contains("deck A: play"))
        .filter_map(|e| if let ArrangeOp::Deck(DeckAction::Play { deck, on }) = &e.op { (deck == "A").then_some((e.at, *on)) } else { None })
        .collect();
    assert_eq!(plays, vec![(4.0, false), (8.0, true)], "{:?}", a.events.iter().map(|e| (e.at, e.text.clone())).collect::<Vec<_>>());
    // (`sync` has no op in the format: it is listed as not in the render, below)
}

#[test]
fn a_hold_is_put_back_where_it_ends_a_mute_is_unmuted_a_started_deck_stopped() {
    let m = moves(&song(4, "  at bar 1 for 2 beats: deck B: mute\n  at bar 2 beat 1 for 1 bars: deck A: mute\n"));
    assert_eq!(
        m.iter().map(|x| (x.at, x.knob.as_str(), x.value)).collect::<Vec<_>>(),
        vec![(4.0, "B mute", 1.0), (6.0, "B mute", 0.0), (9.0, "A mute", 1.0), (13.0, "A mute", 0.0)],
    );
    // `every` puts back each time
    let m = moves(&song(8, "  every 2 bars for 1 beats: deck B: mute\n"));
    let b = knob(&m, "B mute");
    assert_eq!(b, vec![(8.0, 1.0), (9.0, 0.0), (16.0, 1.0), (17.0, 0.0), (24.0, 1.0), (25.0, 0.0)]);
    // a deck started for a while is stopped after it
    let c = compile(&song(4, "  at bar 1 for 2 beats: deck B: play\n"));
    let (a, _) = arr_of(&c);
    let plays: Vec<(f64, bool)> = a.events.iter().filter(|e| e.text.contains("deck B: play")).filter_map(|e| if let ArrangeOp::Deck(DeckAction::Play { on, .. }) = &e.op { Some((e.at, *on)) } else { None }).collect();
    assert_eq!(plays, vec![(4.0, true), (6.0, false)]);
}

#[test]
fn a_value_that_is_already_there_is_not_sent_again_by_a_moving_line_and_a_plain_one_is_always_sent() {
    // two lines holding the same value in a row: plain lines say what they say; a ramp whose first sample is the value the knob has is not repeated
    let m = moves(&song(4, "  at bar 1: deck A: level 0.5\n  at bar 2: deck A: level 0.5 -> 0.9 over 1 bars\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a[0], (4.0, 0.5));
    assert!(a[1].0 > 8.0 + 1e-9 && a[1].1 > 0.5, "the ramp's first sample is the knob's own value: not sent twice: {:?}", &a[..3]);
}

// ---------------------------------------------------------------------------------------------------------------------
// ramps
// ---------------------------------------------------------------------------------------------------------------------

fn grid(from: f64, to: f64) -> impl Iterator<Item = f64> {
    let n = ((to - from) / SAMPLE_BEATS).round() as i64;
    (0..=n).map(move |k| from + k as f64 * SAMPLE_BEATS)
}

#[test]
fn a_linear_ramp_is_a_sample_every_sixty_fourth_of_a_bar_from_its_first_value_to_its_exact_last() {
    let m = moves(&song(4, "  at bar 1: deck A: level 0.2 -> 0.8 over 2 bars\n"));
    let a = knob(&m, "A volume");
    // beats 4 to 12, a sixteenth of a beat apart: 129 values, each of them moving by more than the runtime's threshold
    assert_eq!(a.len(), 129);
    assert_eq!(a[0], (4.0, 0.2));
    assert_eq!(a[128], (12.0, 0.8));
    for (at, v) in &a {
        assert!(near(*v, 0.2 + 0.6 * (at - 4.0) / 8.0), "{at}: {v}");
    }
    // the grid is the documented one
    for ((at, _), g) in a.iter().zip(grid(4.0, 12.0)) {
        assert!((at - g).abs() < 1e-9, "{at} vs {g}");
    }
    // nothing else of the program moved a knob
    assert_eq!(m.len(), 129);
}

#[test]
fn every_shape_is_what_the_formula_says_at_every_beat_of_the_grid_and_exact_at_its_ends() {
    // by hand, as docs/btw/ext-shape.md has them: t is the fraction of the ramp gone
    type Shape = (&'static str, fn(f64) -> f64);
    let shapes: [Shape; 6] = [
        ("", |t| t),
        ("exp", |t| ((4.0 * t).exp2() - 1.0) / 15.0),
        ("log", |t| 1.0 - (((4.0 * (1.0 - t)).exp2() - 1.0) / 15.0)),
        ("s-curve", |t| 3.0 * t * t - 2.0 * t * t * t),
        ("step 4", |t| ((t * 4.0 + 1e-9).floor() / 3.0).min(1.0)),
        ("step 5", |t| ((t * 5.0 + 1e-9).floor() / 4.0).min(1.0)),
    ];
    for (word, f) in shapes {
        let m = moves(&song(4, &format!("  at bar 1: deck A: level 0 -> 1 over 2 bars {word}\n")));
        let a = knob(&m, "A volume");
        assert_eq!(a.first(), Some(&(4.0, 0.0)), "{word}");
        // (a step ramp holds its last value from its last leap: the value it ends on is not sent again at the end)
        assert_eq!(a.last().map(|x| x.1), Some(1.0), "{word}");
        assert!(word.starts_with("step") || a.last().unwrap().0 == 12.0, "{word}: {:?}", a.last());
        for g in grid(4.0, 12.0) {
            let t = (g - 4.0) / 8.0;
            let got = held(&a, g).unwrap();
            // a value is held until it moved by the runtime's threshold, exactly as the runtime holds it
            assert!((got - f(t)).abs() <= RAMP_EPS + 1e-6, "{word}: at beat {g}: the render holds {got}, the formula says {}", f(t));
        }
    }
}

#[test]
fn the_leaps_of_a_step_ramp_are_on_their_exact_beats() {
    // 4 bars of 16 beats from beat 4: leaps at 4 + 4 k, whatever the grid
    let m = moves(&song(8, "  at bar 1: deck A: level 0 -> 1 over 4 bars step 4\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a.len(), 4, "{a:?}");
    for (k, (at, v)) in a.iter().enumerate() {
        assert!((at - (4.0 + 4.0 * k as f64)).abs() < 1e-9 && near(*v, k as f64 / 3.0), "{a:?}");
    }
    // a ramp whose length is no multiple of the grid (a third of a bar): the end is exact all the same
    let m = moves(&song(4, "  at bar 1: deck A: level 0 -> 1 over 1/3 bars\n"));
    let a = knob(&m, "A volume");
    assert!(near(a.last().unwrap().0, 4.0 + 4.0 / 3.0) && a.last().unwrap().1 == 1.0, "{:?}", a.last());
    // and EQ, master and crossfader ramps are the same ramps on their own knobs
    let m = moves(&song(4, "  deck B: low 0 -> 1 over 1 bars\n  master: level 1 -> 0.5 over 1 bars\n  crossfader 0 -> 1 over 1 bars exp\n"));
    for (name, from, to) in [("B eqLow", 0.0, 1.0), ("master", 1.0, 0.5), ("crossfader", 0.0, 1.0)] {
        let k = knob(&m, name);
        assert!(k.len() > 30 && near(k[0].1, from) && near(k.last().unwrap().1, to) && near(k.last().unwrap().0, 4.0), "{name}: {:?}", k.len());
    }
}

#[test]
fn a_ramp_stops_where_its_line_fires_again() {
    // every bar a ramp of 4 bars starts afresh: it never gets past a quarter of the way
    let m = moves(&song(4, "  every 1 bars: deck A: level 0 -> 1 over 4 bars\n"));
    let a = knob(&m, "A volume");
    assert!(a.iter().all(|(_, v)| *v < 0.25 + 1e-6), "{:?}", a.iter().map(|x| x.1).fold(0.0, f64::max));
    // it started at beats 4, 8 and 12 (an `every 1 bars` line never fires on bar 0)
    for at in [4.0, 8.0, 12.0] {
        assert!(a.contains(&(at, 0.0)), "{at}");
    }
    // the value of the bar line before it restarts is just short of a quarter (its last sample is a grid step before)
    let before: Vec<&(f64, f64)> = a.iter().filter(|(at, _)| *at < 8.0 && *at > 7.9).collect();
    assert_eq!(before.len(), 1);
    assert!(near(before[0].1, (8.0 - SAMPLE_BEATS - 4.0) / 16.0), "{before:?}");
}

#[test]
fn a_ramp_runs_on_past_its_section_as_the_runtime_lets_it() {
    // the ramp is 8 bars; its section 4: the render goes on to the end of the song, as the runtime does
    let m = moves("tempo 120\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck A: level 0 -> 1 over 8 bars\n}\nsection two 8 bars {\n  deck A: kick x ~ x ~\n}\narrange one two\n");
    let a = knob(&m, "A volume");
    assert!(near(a.last().unwrap().0, 32.0) && a.last().unwrap().1 == 1.0, "{:?}", a.last());
}

// ---------------------------------------------------------------------------------------------------------------------
// lfos
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn an_lfo_is_its_formula_to_the_end_of_its_section() {
    let m = moves(&song(4, "  master: level lfo sine 1 bar center=0.8 depth=0.1\n"));
    let a = knob(&m, "master");
    assert!(a.len() > 100, "{}", a.len());
    for (at, v) in &a {
        let x = at / 4.0;
        assert!(near(*v, 0.8 + 0.1 * (std::f64::consts::TAU * x).sin()), "beat {at}: {v}");
    }
    // it rises from the middle to 0.9 a quarter period in and is at 0.7 three quarters in
    assert_eq!(held(&a, 1.0), Some(0.9));
    assert_eq!(held(&a, 3.0), Some(0.7));
    // it stops with its section: the render ends there (16 beats)
    assert!(a.last().unwrap().0 < 16.0);
}

#[test]
fn an_lfo_with_an_over_ends_there_and_sends_its_last_value_once_phase_and_waves() {
    let m = moves(&song(4, "  at bar 1: master: level lfo sine 1 bar center=0.5 depth=0.25 phase=0.25 over 2 bars\n"));
    let a = knob(&m, "master");
    // from bar 1 (beat 4) for two bars: the value at its end (beat 12) is the one of a whole number of periods
    assert_eq!(a.first().unwrap().0, 4.0);
    assert!(near(a.first().unwrap().1, 0.75), "phase a quarter in: the top: {:?}", a.first());
    assert_eq!(a.last().unwrap().0, 12.0);
    assert!(near(a.last().unwrap().1, 0.75), "{:?}", a.last());
    // the triangle and the saw, by their formulas
    let tri = |c: f64| {
        let p = c - c.floor();
        if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        }
    };
    let saw = |c: f64| {
        let p = c - c.floor();
        2.0 * (p - (p + 0.5).floor())
    };
    for (word, f) in [("tri", tri as fn(f64) -> f64), ("saw", saw as fn(f64) -> f64)] {
        let m = moves(&song(4, &format!("  deck A: level lfo {word} 2 bars center=0.5 depth=0.4\n")));
        let a = knob(&m, "A volume");
        assert!(a.len() > 40, "{word}: {}", a.len());
        for (at, v) in &a {
            assert!(near(*v, 0.5 + 0.4 * f(at / 8.0)), "{word}: beat {at}: {v}");
        }
    }
}

#[test]
fn a_square_lfo_changes_on_the_beat_of_its_edge() {
    // a bar a period: high for two beats, low for two
    let m = moves(&song(2, "  deck A: level lfo square 1 bar center=0.5 depth=0.25\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a, vec![(0.0, 0.75), (2.0, 0.25), (4.0, 0.75), (6.0, 0.25)]);
    // with a phase the edges move: an eighth of a period in at the start, so the first edge is at 7/16 of a bar
    let m = moves(&song(2, "  deck A: level lfo square 1 bar center=0.5 depth=0.25 phase=0.125\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a[0], (0.0, 0.75));
    assert!(near(a[1].0, 1.5) && near(a[1].1, 0.25), "{a:?}");
    assert!(near(a[2].0, 3.5) && near(a[2].1, 0.75), "{a:?}");
}

#[test]
fn a_square_lfo_edge_that_is_between_two_grid_points_is_on_its_own_beat() {
    // phase 0.1 of a bar: the first edge is at (1/2 - 1/10) bar = 1.6 beats, off the grid of sixteenths of a beat
    let m = moves(&song(2, "  deck A: level lfo square 1 bar center=0.5 depth=0.25 phase=0.1\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a[0], (0.0, 0.75));
    for (got, (at, v)) in a.iter().skip(1).zip([(1.6, 0.25), (3.6, 0.75), (5.6, 0.25), (7.6, 0.75)]) {
        assert!((got.0 - at).abs() < 1e-9 && near(got.1, v), "{a:?}");
    }
    // the same for a step ramp whose leaps are off the grid: 3 bars in 7 steps is a leap every 12/7 beats
    let m = moves(&song(4, "  at bar 1: deck A: level 0 -> 1 over 3 bars step 7\n"));
    let a = knob(&m, "A volume");
    assert_eq!(a.len(), 7, "{a:?}");
    for (k, (at, v)) in a.iter().enumerate() {
        assert!((at - (4.0 + 12.0 * k as f64 / 7.0)).abs() < 1e-9 && near(*v, k as f64 / 6.0), "{a:?}");
    }
}

#[test]
fn the_fastest_lfo_the_language_allows_stays_inside_its_range() {
    // the shortest period the language allows is a sixteenth of a bar: four samples a period at the grid's fineness
    let m = moves(&song(2, "  master: level lfo sine 1/16 bars center=0.5 depth=0.3\n"));
    let a = knob(&m, "master");
    assert!(a.iter().all(|(_, v)| (0.2 - 1e-6..=0.8 + 1e-6).contains(v)), "{a:?}");
    assert!(a.len() > 8);
}

// ---------------------------------------------------------------------------------------------------------------------
// sub-bar lines, holds with moving values, the render's own end
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_line_between_bar_lines_starts_on_its_beat_and_a_repeating_one_on_each_of_its_beats() {
    let m = moves(&song(4, "  at bar 1 beat 2: deck A: level 0.5 -> 0.9 over 2 beats\n"));
    let a = knob(&m, "A volume");
    assert_eq!((a.first().unwrap().0, a.first().unwrap().1), (6.0, 0.5));
    assert_eq!((a.last().unwrap().0, a.last().unwrap().1), (8.0, 0.9));
    assert_eq!(a.len(), 33);
    let m = moves(&song(2, "  every 2 beats from bar 1: master: level 0.4 -> 0.6 over 1 beats\n"));
    let a = knob(&m, "master");
    // one beat of ramp, then the next starts two beats on; every run is 17 samples (0.4, 0.4125 ... 0.6)
    let firsts: Vec<f64> = a.iter().filter(|(_, v)| near(*v, 0.4)).map(|x| x.0).collect();
    assert_eq!(firsts, vec![4.0, 6.0]);
}

#[test]
fn nothing_is_sent_at_or_after_the_end_of_the_render() {
    let (arr, beats) = arr_of(&compile(&song(2, "  deck A: level 0 -> 1 over 8 bars\n  master: level lfo sine 1 bar center=0.5 depth=0.2\n")));
    assert_eq!(beats, 8.0);
    assert!(moves_of(&arr).iter().all(|m| m.at < beats), "{:?}", moves_of(&arr).iter().map(|m| m.at).fold(0.0, f64::max));
}

#[test]
fn a_program_with_no_sections_counts_its_bars_from_the_first_beat() {
    let m = moves("tempo 120\ndeck A: kick x ~ x ~\ndeck A: level 0.2 -> 0.4 over 1 bars\nat bar 2: master: level 0.5\n");
    assert_eq!(knob(&m, "A volume")[0], (0.0, 0.2));
    assert_eq!(knob(&m, "master"), vec![(8.0, 0.5)]);
}

#[test]
fn a_render_with_no_control_lines_is_as_it_was_and_a_mix_goes_back_to_what_the_lines_left() {
    let (a, _) = arr_of(&compile(&song(4, "")));
    assert!(moves_of(&a).is_empty());
    // a section's mix is put back to what the render held: the line's 0.5, no longer the engine's starting 0.8
    let src = "tempo 120\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck A: level 0.5\n  at bar 1: mix { A level 0.9 }\n}\nsection two 4 bars {\n  deck A: kick x ~ x ~\n}\narrange one two\n";
    let a = knob(&moves(src), "A volume");
    assert_eq!(a, vec![(0.0, 0.5), (4.0, 0.9), (16.0, 0.5)]);
}

// ---------------------------------------------------------------------------------------------------------------------
// the runtime's own values
// ---------------------------------------------------------------------------------------------------------------------

/// The knobs a runtime call moves, by the render's names.
fn call_knobs(noun: &str, p: &JsonValue) -> Vec<(String, f64)> {
    if noun != "mixer" {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(d) = p.get("deck").and_then(JsonValue::as_str) {
        for (key, name) in [("volume", "volume"), ("low", "eqLow"), ("mid", "eqMid"), ("high", "eqHigh")] {
            if let Some(v) = p.get(key).and_then(JsonValue::as_f64) {
                out.push((format!("{d} {name}"), v));
            }
        }
        if let Some(m) = p.get("mute").and_then(JsonValue::as_bool) {
            out.push((format!("{d} mute"), if m { 1.0 } else { 0.0 }));
        }
    }
    for k in ["master", "crossfader"] {
        if let Some(v) = p.get(k).and_then(JsonValue::as_f64) {
            out.push((k.to_string(), v));
        }
    }
    out
}

/// What the runtime sends for `src` on a fake clock, passes `step` beats apart, by knob: (the song's beat, value). The song lands on
/// the clock's bar 1, so its beat 0 is the clock's beat 4.
fn runtime_moves(src: &str, bars: u32, step: f64) -> BTreeMap<String, Vec<(f64, f64)>> {
    let calls = runtime_controls(src, &Env::none(), bars + 2, step).expect("the program runs");
    let mut m: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    for c in calls {
        for (k, v) in call_knobs(&c.noun, &c.params) {
            m.entry(k).or_default().push((c.beat - 4.0, v));
        }
    }
    m
}

/// The render's moves and the runtime's agree at every pass of the runtime: the render holds what the runtime holds (to the
/// runtime's own threshold), and neither moves a knob the other does not. Passes `step` beats apart up to `end`.
fn agrees_with_the_runtime(src: &str, bars: u32, step: f64, end: f64) {
    let render = moves(src);
    let runtime = runtime_moves(src, bars, step);
    let names: std::collections::BTreeSet<String> = render.iter().map(|m| m.knob.clone()).chain(runtime.keys().cloned()).collect();
    assert!(!names.is_empty(), "no knob moved:\n{src}");
    for name in names {
        let (r, u) = (knob(&render, &name), runtime.get(&name).cloned().unwrap_or_default());
        let mut g = 0.0;
        while g < end - 1e-9 {
            // (the runtime's pass at `g` has made its calls by then; so has the render's event at `g`)
            let (rv, uv) = (held(&r, g), held(&u, g));
            match (rv, uv) {
                (None, None) => {}
                (Some(a), Some(b)) => assert!((a - b).abs() <= RAMP_EPS + 1e-6, "{name} at beat {g}: the render holds {a}, the runtime {b}\n{src}"),
                (a, b) => panic!("{name} at beat {g}: the render holds {a:?}, the runtime {b:?}\n{src}"),
            }
            g += step;
        }
    }
}

#[test]
fn the_render_holds_what_the_runtime_sends_at_the_same_beats() {
    // every shape of ramp, an lfo of each wave, a plain value, a mute with a hold: the whole of one mixer
    let shapes = ["", "exp", "log", "s-curve", "step 4"];
    for shape in shapes {
        let src = song(4, &format!("  at bar 1: deck A: level 0 -> 1 over 2 bars {shape}\n"));
        agrees_with_the_runtime(&src, 4, SAMPLE_BEATS, 16.0);
    }
    // (a square lfo whose edges are off the grid: the runtime's pass after the edge and the render's own beat agree on the pass)
    agrees_with_the_runtime(&song(4, "  deck A: level lfo square 1 bar center=0.5 depth=0.25 phase=0.1\n"), 4, SAMPLE_BEATS, 15.5);
    for wave in ["sine", "tri", "saw", "square"] {
        let src = song(4, &format!("  master: level lfo {wave} 1 bar center=0.7 depth=0.2\n  deck B: level lfo {wave} 2 bars center=0.5 depth=0.3 phase=0.25 over 3 bars\n"));
        agrees_with_the_runtime(&src, 4, SAMPLE_BEATS, 15.5);
    }
    let src = song(4, "  at bar 1: deck B: low 0.3\n  at bar 1 for 2 beats: deck A: mute\n  at bar 2 beat 2: crossfader 0.25 -> 0.75 over 1 bars s-curve\n  at bar 2: master: level 0.9 -> 0.3 over 2 bars exp\n");
    agrees_with_the_runtime(&src, 4, SAMPLE_BEATS, 16.0);
}

#[test]
fn a_ramp_that_restarts_and_two_lines_on_one_knob_agree_with_the_runtime_too() {
    let src = song(8, "  every 2 bars: deck A: level 0.2 -> 0.9 over 3 bars\n  at bar 5: deck A: level 0.5\n");
    agrees_with_the_runtime(&src, 8, SAMPLE_BEATS, 31.5);
    let src = song(4, "  deck A: level 0.8 -> 0.4 over 3 bars\n  at bar 1 beat 2: deck A: level lfo sine 1 bar center=0.6 depth=0.1 over 1 bars\n");
    agrees_with_the_runtime(&src, 4, SAMPLE_BEATS, 15.5);
}

#[test]
fn the_runtime_comparison_is_made_of_real_moves() {
    // a guard against a comparison that compares nothing: the runtime sends dozens of values for a ramp
    let r = runtime_moves(&song(4, "  at bar 1: deck A: level 0 -> 1 over 2 bars\n"), 4, SAMPLE_BEATS);
    let a = &r["A volume"];
    assert!(a.len() > 50, "{}", a.len());
    assert_eq!(a[0], (4.0, 0.0));
    assert_eq!(a.last().unwrap().1, 1.0);
}

// ---------------------------------------------------------------------------------------------------------------------
// the tempo curve
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn under_a_tempo_curve_a_ramp_lands_on_the_seconds_the_map_gives_its_beats() {
    let src = "tempo 100 -> 140 over 6 bars\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck A: level 0 -> 1 over 4 bars\n}\nsection two 4 bars {\n  deck A: kick x ~ x ~\n  master: level lfo sine 1 bar center=0.7 depth=0.2\n  at bar 1 for 2 beats: deck B: mute\n}\narrange one two\n";
    let c = compile(src);
    let (a, beats) = arr_of(&c);
    let map = c.tempo_map();
    let m = moves_of(&a);
    assert!(m.len() > 150, "{}", m.len());
    for x in &m {
        let bars = Rat::new((x.at * 64.0).round() as i64, 256).unwrap();
        let (got, want) = (a.seconds_at(x.at), map.seconds_at(bars));
        assert!((got - want).abs() < 1e-3, "beat {}: the render's clock says {got:.5} s, the tempo map {want:.5} s", x.at);
    }
    // the ramp's last value is at the beat the map puts 4 bars in
    let end = knob(&m, "A volume").last().copied().unwrap();
    assert_eq!(end, (16.0, 1.0));
    assert!((a.seconds_at(16.0) - map.seconds_at(4u32)).abs() < 1e-6);
    // and the render is as long as the map's integral says
    assert!((a.seconds_at(beats) - map.seconds_at(8u32)).abs() < 1e-6);
    // every one of the program's moves is tagged with the tempo of the step it falls in: not all the same
    let bpms: std::collections::BTreeSet<u64> = a.events.iter().filter(|e| e.via == "btw" && e.op.op() == "param").map(|e| e.bpm.to_bits()).collect();
    assert!(bpms.len() > 5, "{}", bpms.len());
}

// ---------------------------------------------------------------------------------------------------------------------
// what it sounds like
// ---------------------------------------------------------------------------------------------------------------------

const SR: f64 = 8000.0;

fn rms(a: &djengine::session::wav::RenderedAudio, from: f64, to: f64) -> f64 {
    let (i, j) = ((from * a.sample_rate) as usize, (to * a.sample_rate) as usize);
    let s: f64 = a.left[i..j].iter().chain(&a.right[i..j]).map(|x| f64::from(*x) * f64::from(*x)).sum();
    (s / (2 * (j - i)) as f64).sqrt()
}

fn stem(c: &Compiled, name: &str) -> djengine::session::wav::RenderedAudio {
    let (a, beats) = arr_of(c);
    let r = render_arrangement(&a, ArrangeRenderOptions { sample_rate: SR, beats: Some(beats), stems: true, ..Default::default() });
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    r.stems.into_iter().find(|(n, _)| n == name).map(|(_, s)| s).unwrap_or_else(|| panic!("no stem {name}"))
}

const HELD: &str = "tempo 120\ndeck A: bass c2 len=16\n";

#[test]
fn a_faded_deck_is_quieter_at_its_start_than_at_its_end_by_the_fade() {
    // 8 bars at 120 BPM are 16 seconds; a steady bass note under a fade of the whole length
    let fade = compile(&format!("{HELD}deck A: level 0 -> 1 over 8 bars\n"));
    let flat = compile(&format!("{HELD}deck A: level 1\n"));
    let (f, p) = (stem(&fade, "A"), stem(&flat, "A"));
    // the same windows of both: the fade's level over the window is the mean of its line there
    let win = |a: f64, b: f64| rms(&f, a, b) / rms(&p, a, b);
    // (a window of a fifth of a second round 2 s, 8 s and 14 s, each a quarter, a half and seven eighths of the way)
    for (from, to, level) in [(1.9, 2.1, 2.0 / 16.0), (7.9, 8.1, 8.0 / 16.0), (13.9, 14.1, 14.0 / 16.0)] {
        let got = win(from, to);
        assert!((got - level).abs() < 0.04, "{from}..{to}: the fade is at {got:.3} of the flat one, the line says {level:.3}");
    }
    // the start is much quieter than the end: by the ratio of the line
    let (start, end) = (win(0.5, 1.5), win(14.5, 15.5));
    assert!(start < 0.12 && end > 0.85, "{start} {end}");
    assert!((end / start - (15.0 / 1.0)).abs() / 15.0 < 0.25, "the end is {} times the start; the line says 15", end / start);
}

#[test]
fn a_render_with_an_lfo_tremolo_is_modulated_as_deep_as_it_says() {
    // a tremolo of a bar a period, 0.5 +- 0.4 on the deck: the quietest window against the loudest
    let t = compile(&format!("{HELD}deck A: level lfo sine 2 bars center=0.5 depth=0.4\n"));
    let flat = compile(&format!("{HELD}deck A: level 1\n"));
    let (m, p) = (stem(&t, "A"), stem(&flat, "A"));
    // a period is 4 s: the top is at 1 s (0.9), the bottom at 3 s (0.1)
    let top = rms(&m, 0.95, 1.05) / rms(&p, 0.95, 1.05);
    let bottom = rms(&m, 2.95, 3.05) / rms(&p, 2.95, 3.05);
    assert!((top - 0.9).abs() < 0.05 && (bottom - 0.1).abs() < 0.05, "{top} {bottom}");
}

#[test]
fn a_crossfader_move_takes_a_deck_out_of_the_mix() {
    // with the crossfader at 1 deck A is out of the mix (the engine's own law)
    let src = "tempo 120\nsection one 4 bars {\n  deck A: bass c2 len=16\n  at bar 2: crossfader 1\n}\narrange one\n";
    let a = stem(&compile(src), "A");
    let (before, after) = (rms(&a, 0.5, 1.5), rms(&a, 4.5, 5.5));
    assert!(before > 0.01 && after < before * 0.02, "{before} {after}");
}

#[test]
fn the_same_text_is_the_same_events() {
    let src = song(4, "  at bar 1: deck A: level 0 -> 1 over 2 bars exp\n  master: level lfo tri 1 bar center=0.6 depth=0.3\n  at bar 2 beat 1 for 2 beats: deck B: mute\n");
    let (a, b) = (arrangement(&compile(&src)).0.js_stringified(), arrangement(&compile(&src)).0.js_stringified());
    assert_eq!(a, b);
    // reformatting the text (spacing, a comment) does not move an event
    let spaced = src.replace("  ", "\t").replace("0 -> 1", "0   ->   1").replace("exp\n", "exp # a fade\n");
    assert_eq!(arrangement(&compile(&spaced)).0.js_stringified(), a);
}

// ---------------------------------------------------------------------------------------------------------------------
// what it cannot carry
// ---------------------------------------------------------------------------------------------------------------------

fn left_out(src: &str) -> djhost::btw_render::NotInRender {
    arrangement_report(&compile(src)).2
}

#[test]
fn the_lines_with_no_op_are_counted_and_named() {
    let src = song(
        8,
        "  master: filter lp 300 -> 2k over 4 bars\n  deck A: filter hp 400\n  master: reverb 0.2\n  deck B: delay 0.3 -> 0.5 over 2 bars\n  master: crush 0.1\n  at bar 2: move bass_swap on B\n  do vocoder set mix=0.5\n  deck A: sync\n  deck A: level 0.5\n  master: level lfo sine 1 bar center=0.7 depth=0.1\n",
    );
    let l = left_out(&src);
    assert_eq!(l.count(), 8, "{l:?}");
    let k = l.kinds();
    assert_eq!(k.get("filter"), Some(&2), "{k:?}");
    assert_eq!((k.get("reverb"), k.get("delay"), k.get("crush"), k.get("move")), (Some(&1), Some(&1), Some(&1), Some(&1)), "{k:?}");
    assert_eq!(k.get("vocoder"), Some(&1), "{k:?}");
    assert_eq!(k.get("deck sync"), Some(&1), "{k:?}");
    // the lines carried are not in the list; the others are named by line and text
    assert!(l.lines.iter().all(|x| !x.text.contains("level")), "{l:?}");
    assert_eq!((l.lines[0].line, l.lines[0].text.as_str()), (5, "master: filter lp 300 -> 2k over 4 bars"));
    let note = l.note();
    assert!(note.starts_with("not in this render: 8 control lines (") && note.contains("filter x2") && note.contains("line 5 `master: filter lp 300 -> 2k over 4 bars`") && note.ends_with("5 more"), "{note}");
    // the JSON says the same
    let j = l.to_json();
    assert_eq!(j.get("count").and_then(JsonValue::as_f64), Some(8.0));
    assert_eq!(j.get("kinds").and_then(|k| k.get("filter")).and_then(JsonValue::as_f64), Some(2.0));
    assert_eq!(j.get("lines").and_then(JsonValue::as_array).map(Vec::len), Some(8));
}

#[test]
fn a_program_the_render_carries_whole_says_nothing_and_a_line_a_def_copies_counts_once() {
    assert!(left_out(&song(4, "  deck A: level 0 -> 1 over 2 bars\n  master: level 0.9\n  at bar 1 for 2 beats: deck B: mute\n")).is_empty());
    let src = "tempo 120\ndef sweep() {\n  master: filter lp 300\n}\nsection one 8 bars {\n  deck A: kick x ~ x ~\n  at bar 1: sweep()\n  at bar 3: sweep()\n  at bar 5: sweep()\n}\narrange one\n";
    let l = left_out(src);
    assert_eq!(l.count(), 1, "{l:?}");
    assert_eq!(l.lines[0].text, "master: filter lp 300");
}

#[test]
fn a_performed_line_is_read_at_one_half_and_a_false_guard_is_no_line_left_out() {
    // the render has no controllers: `knob(1)` reads 1/2, so a level of it is 0.5 and in the render; a filter of it is not
    let src = song(4, "  at bar 1: deck A: level knob(1)\n  at bar 2: deck A: level 0 -> knob(1) over 1 bars\n  at bar 3: master: filter lp 200 + knob(2) * 7800\n  at bar 3: deck B: level 0.3 when knob(1) < 0.5\n");
    let c = compile(&src);
    let (a, _) = arr_of(&c);
    let m = moves_of(&a);
    let v = knob(&m, "A volume");
    assert_eq!(v[0], (4.0, 0.5));
    assert!(near(v.last().unwrap().1, 0.5) && v.last().unwrap().0 == 12.0, "{:?}", v.last());
    assert!(knob(&m, "B volume").is_empty(), "knob(1) < 1/2 is false at 1/2");
    let l = arrangement_report(&c).2;
    assert_eq!(l.count(), 1, "{l:?}");
    assert_eq!(l.kinds().get("filter"), Some(&1));
}

#[test]
fn a_ramp_in_a_def_called_from_a_repeat_is_in_the_render_at_each_copys_beat() {
    let src = "tempo 120\ndef swell(a, b) {\n  deck A: level $a -> $b over 1 bars\n}\nsection one 8 bars {\n  deck A: kick x ~ x ~\n  repeat 2 every 4 bars {\n    swell(0, 1)\n  }\n}\narrange one\n";
    let a = knob(&moves(src), "A volume");
    // two runs: from bar 4 and bar 8 of the section... the repeat's copies are placed by the compiler; both are in
    let starts: Vec<f64> = a.iter().filter(|(_, v)| *v == 0.0).map(|x| x.0).collect();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!((starts[1] - starts[0] - 16.0).abs() < 1e-9, "{starts:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// the example
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_example_renders_its_fade_its_tremolo_its_crossfader_and_its_hold_and_names_its_filter() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw/render-controls.btw");
    let src = std::fs::read_to_string(&path).unwrap();
    let c = compile(&src);
    let (a, beats) = arr_of(&c);
    assert_eq!(beats, 64.0);
    let m = moves_of(&a);
    // intro is beats 0 to 32: the bass fades in along an exp curve, to exactly 1 on the bar line the groove begins on
    let fade = knob(&m, "B volume");
    assert_eq!((fade.first().copied(), fade.last().copied()), (Some((0.0, 0.0)), Some((32.0, 1.0))));
    // (halfway, `exp` is at (2^2 - 1) / 15 = 0.2)
    assert!((held(&fade, 16.0).unwrap() - 0.2).abs() <= RAMP_EPS + 1e-6, "{:?}", held(&fade, 16.0));
    // the groove: a tremolo of the master from its first bar (beat 32), 0.85 +- 0.1, to the end of the render
    let trem = knob(&m, "master");
    assert_eq!(trem[0], (32.0, 0.85));
    assert_eq!(held(&trem, 33.0), Some(0.95));
    assert_eq!(held(&trem, 35.0), Some(0.75));
    // the crossfader moves from bar 3 of the groove (beat 44) to bar 5 (beat 52), gently at both ends
    let xf = knob(&m, "crossfader");
    assert_eq!((xf.first().copied(), xf.last().copied()), (Some((44.0, 0.5)), Some((52.0, 0.7))));
    // the mute is on at bar 6 of the groove (beat 56) and off two beats later
    assert_eq!(knob(&m, "A mute"), vec![(56.0, 1.0), (58.0, 0.0)]);
    // the filter is an insert: the render says it is not there
    let left = arrangement_report(&c).2;
    assert_eq!(left.count(), 1, "{left:?}");
    assert_eq!((left.lines[0].kind.as_str(), left.lines[0].text.as_str()), ("filter", "master: filter lp 400 -> 8k over 4 bars"));
    // the engine takes every event of it
    let r = render_arrangement(&a, ArrangeRenderOptions { sample_rate: 8000.0, beats: Some(beats), ..Default::default() });
    assert!(r.refused.is_empty(), "{:?}", r.refused);
}

#[test]
fn a_do_line_to_the_mixer_or_a_deck_is_carried_like_the_line_that_says_the_same_and_a_sing_line_is_not() {
    // `do` calls are registry calls: the ones the arrangement has an op for are in the render, ramps and holds too
    let m = moves(&song(8, "  at bar 1: do mixer volume=0.4 deck=A\n  at bar 2: do mixer master=0.3\n  at bar 3: do mixer deck=B low=0.2\n  at bar 3: do mixer deck=A volume=0 -> 1 over 1 bars\n"));
    assert_eq!(m[0], Move { at: 4.0, knob: "A volume".into(), value: 0.4 });
    assert_eq!(m[1], Move { at: 8.0, knob: "master".into(), value: 0.3 });
    assert_eq!(knob(&m, "B eqLow"), vec![(12.0, 0.2)]);
    let ramp = knob(&m, "A volume");
    assert!(ramp.len() > 60 && ramp.last() == Some(&(16.0, 1.0)), "{:?} {}", ramp.last(), ramp.len());
    let (a, _) = arr_of(&compile(&song(4, "  at bar 1: do deck play deck=B\n  at bar 2: do deck stop deck=B\n")));
    let plays: Vec<(f64, bool)> = a.events.iter().filter(|e| e.text.contains("do deck")).filter_map(|e| if let ArrangeOp::Deck(DeckAction::Play { on, .. }) = &e.op { Some((e.at, *on)) } else { None }).collect();
    assert_eq!(plays, vec![(4.0, true), (8.0, false)]);
    // a vocal line is a registry call the arrangement has no op for
    let c = compile(&song(4, "  deck B: melody <c4 e4 g4 e4>\n  at bar 1: deck B: sing \"la la la\" on melody voice=alto\n"));
    let left = arrangement_report(&c).2;
    assert_eq!(left.count(), 1, "{left:?}");
    assert_eq!(left.lines[0].kind, "vocal", "{left:?}");
}

#[test]
fn a_ramp_and_a_mute_from_an_imported_def_are_in_the_render_and_a_filter_from_one_is_listed_at_its_calls() {
    let dir = std::env::temp_dir().join(format!("btw-render-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = djhost::live_store::LiveStore::new(&dir);
    store.save("fades", "def swell(a, b) {\n  deck A: level $a -> $b over 2 bars\n}\ndef blip() {\n  for 2 beats: deck B: mute\n}\ndef sweep() {\n  master: filter lp 300\n}\n").unwrap();
    let be = djhost::btw_env::BtwEnv::new(Some(&store));
    let src = "import \"fades\"\nsection one 8 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  at bar 1: swell(0.2, 0.8)\n  at bar 4: blip()\n  at bar 5: sweep()\n  at bar 6: sweep()\n}\narrange one\n";
    let check = djlive::compile::check_with(src, &be.env());
    let c = check.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&check.diagnostics, src)));
    let (a, _) = arr_of(&c);
    let m = moves_of(&a);
    let fade = knob(&m, "A volume");
    assert_eq!((fade.first().copied(), fade.last().copied()), (Some((4.0, 0.2)), Some((12.0, 0.8))));
    assert_eq!(knob(&m, "B mute"), vec![(16.0, 1.0), (18.0, 0.0)]);
    // the filter is in an imported def that is called twice: the import has no line of its own here, so each call is the line, named by the call
    let left = arrangement_report(&c).2;
    assert_eq!(left.count(), 2, "{left:?}");
    assert_eq!((left.lines[0].line, left.lines[0].kind.as_str(), left.lines[0].text.as_str()), (7, "filter", "at bar 5: sweep(): master: filter lp 300"));
    assert_eq!((left.lines[1].line, left.lines[1].text.as_str()), (8, "at bar 6: sweep(): master: filter lp 300"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sections_stacked_and_one_after_another_agree_with_the_runtime_too() {
    // F10: a stack plays two sections at once (the longer decides the length), and an lfo ends where its own section ends, not the stretch
    let src = "tempo 120\nsection groove 8 bars {\n  deck A: kick x ~ x ~\n  deck A: level 0.2 -> 0.9 over 4 bars exp\n  at bar 5: master: level 0.9 -> 0.6 over 2 bars\n}\nsection wash 4 bars {\n  deck B: bass c2 len=16\n  deck B: level lfo sine 1 bar center=0.5 depth=0.3\n  at bar 2: crossfader 0.4\n}\nsection coda 4 bars {\n  deck A: kick x ~ x ~\n  deck A: level 1 -> 0 over 2 bars s-curve\n}\narrange then(stack(groove, wash), coda)\n";
    let (a, beats) = arr_of(&compile(src));
    assert_eq!(beats, 48.0);
    let m = moves_of(&a);
    // the wash's lfo is over at bar 4 (beat 16), the groove goes on to bar 8
    let b = knob(&m, "B volume");
    assert!(b.last().unwrap().0 <= 16.0 && b.last().unwrap().0 > 15.0, "{:?}", b.last());
    let (ga, ca) = (knob(&m, "A volume"), 32.0);
    assert_eq!(held(&ga, ca - 0.1), Some(0.9), "the groove's fade ended at its 4 bars");
    assert!(ga.iter().any(|(at, _)| *at == ca), "the coda's own fade starts at its bar");
    agrees_with_the_runtime(src, 12, SAMPLE_BEATS, 47.5);
}

#[test]
fn a_long_song_with_many_moving_lines_renders_in_reasonable_time_and_the_events_are_bounded() {
    // 64 bars, four lfos and two ramps: the worst a program the net bound allows (a sample a sixty-fourth of a bar a run, at most)
    let src = song(
        64,
        "  master: level lfo sine 1 bar center=0.8 depth=0.1\n  deck A: level lfo tri 1/2 bars center=0.7 depth=0.2\n  deck B: low lfo saw 3 beats center=0.5 depth=0.3\n  crossfader lfo sine 2 bars center=0.5 depth=0.4\n  deck A: high 0 -> 1 over 64 bars exp\n  deck B: level 1 -> 0 over 60 bars s-curve\n",
    );
    let c = compile(&src);
    let start = std::time::Instant::now();
    let (a, beats) = arr_of(&c);
    let n = moves_of(&a).len();
    // at most a sample per grid point per moving line
    assert!(n <= 6 * (beats / SAMPLE_BEATS) as usize, "{n} moves");
    let r = render_arrangement(&a, ArrangeRenderOptions { sample_rate: 22_050.0, beats: Some(beats), ..Default::default() });
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    eprintln!("{n} moves, built and rendered in {:?}", start.elapsed());
    assert!(start.elapsed().as_secs() < 60, "{:?}", start.elapsed());
}
