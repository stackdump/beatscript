//! F12 x F3 x F8 x F10: `btw test` on a program whose tempo moves. The level windows of a section are the section's seconds
//! *from the tempo map* (`Compiled::tempo_map().seconds_at(start)..seconds_at(end)`), not the bars at the program's first tempo:
//! the render of a curve (`btw_render` with `btw_tempo`) tags every event with the tempo of the step it falls in, and the window
//! is read off that clock. Here the windows the report claims are compared with the map, for a curve with a duck and a mix (F8), a
//! section's own tempo and ramp, and a stack of sections of unequal lengths (F10); the measure is compared with an independent
//! RMS of the stems over the map's seconds; and the duck and the mix move the levels the assertions read.

use djengine::session::arrangement::Arrangement;
use djengine::session::wav::RenderedAudio;
use djhost::btw_render::arrangement;
use djhost::btw_test::{run_text, Options, Report};
use djhost::deck::offline::{render_arrangement, ArrangeRenderOptions};
use djlive::compile::Compiled;
use djlive::env::Env;
use djlive::rat::Rat;

const SR: f64 = 22_050.0;

fn compile(src: &str) -> Compiled {
    let c = djlive::compile::check_report(src, &Env::none());
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn test(src: &str) -> Report {
    run_text(src, &Env::none(), &Options { sample_rate: SR }).unwrap_or_else(|d| panic!("{src}\n{}", djlive::diag::render_all(&d, src)))
}

/// A curve under a duck and a mix: the verse is 4 bars of the ramp, the chorus is the rest of it and the hold, with the pump on
/// deck B under A's kick, A's fader at 0.7 and the master at 0.9.
const MIXED: &str = "tempo 100 -> 140 over 6 bars\nsection verse 4 bars {\n  deck A: kick x x x x\n  deck B: bass c2 len=16\n}\nsection chorus 4 bars {\n  deck A: kick x x x x\n  deck B: bass c2 len=16\n  duck B under A.kick 12db\n  mix { A level 0.7, master level 0.9 }\n}\narrange verse chorus\n";

fn plain() -> String {
    MIXED.replace("  duck B under A.kick 12db\n", "").replace("  mix { A level 0.7, master level 0.9 }\n", "")
}

/// The seconds `bar` is at, by the map.
fn at(c: &Compiled, bar: u32) -> f64 {
    c.tempo_map().seconds_at(Rat::int(i64::from(bar)))
}

/// The windows an outcome claims, against the section's bars through the map.
fn windows_are(c: &Compiled, r: &Report, i: usize, bars: &[(u32, u32)]) {
    let o = &r.outcomes[i];
    assert_eq!(o.windows.len(), bars.len(), "{}: {:?}", o.text, o.windows);
    for ((from, to), (s, e)) in o.windows.iter().zip(bars) {
        let (want_from, want_to) = (at(c, *s), at(c, *e));
        assert!((from - want_from).abs() < 1e-3 && (to - want_to).abs() < 1e-3, "{}: the window is {from:.4}..{to:.4} s, the map says {want_from:.4}..{want_to:.4} s (bars {s}..{e})", o.text);
    }
}

#[test]
fn the_windows_of_a_curve_with_a_duck_and_a_mix_are_the_maps_seconds() {
    let src = format!("{MIXED}assert level A > 0 in verse\nassert level B > 0 in chorus\nassert level master > 0\nassert level A > 0 in chorus\n");
    let (c, r) = (compile(&src), test(&src));
    assert!(r.ok(), "{}", r.text());
    windows_are(&c, &r, 0, &[(0, 4)]);
    windows_are(&c, &r, 1, &[(4, 8)]);
    windows_are(&c, &r, 2, &[(0, 8)]);
    windows_are(&c, &r, 3, &[(4, 8)]);
    // the verse ends where the ramp's 16 beats do: 60 * 16 * ln(126.667 / 100) / 26.667 s, not the 9.6 s of four bars at 100
    let (vs, ve) = r.outcomes[0].windows[0];
    let by_hand = 60.0 * 16.0 * (126.666_666_666_666_67f64 / 100.0).ln() / (126.666_666_666_666_67 - 100.0);
    assert_eq!(vs, 0.0);
    assert!((ve - by_hand).abs() < 1e-3 && (ve - 9.6).abs() > 1.0, "{ve} s; by hand {by_hand} s");
    // the render is as long as the curve says, and the report says what it rendered
    let rd = r.rendered.as_ref().expect("a level assertion renders");
    assert_eq!(rd.bars, 8);
    assert!((rd.seconds - c.tempo_map().total_seconds(8u32)).abs() < 2.0 / SR + 1e-9, "{} vs {}", rd.seconds, c.tempo_map().total_seconds(8u32));
    // the answer of the verb carries them
    let j = r.to_json();
    let w = j.get("results").and_then(|v| v.as_array()).unwrap()[1].get("windows").and_then(|v| v.as_array()).unwrap().clone();
    assert_eq!(w.len(), 1);
    let pair = w[0].as_array().unwrap();
    assert!((pair[0].as_f64().unwrap() - at(&c, 4)).abs() < 1e-3 && (pair[1].as_f64().unwrap() - at(&c, 8)).abs() < 1e-3, "{w:?}");
    // a static carries no window
    let r = test(&format!("{MIXED}assert length == 15.54s ± 10ms\nassert level A > 0 in verse\n"));
    assert!(r.outcomes[0].windows.is_empty() && r.outcomes[0].pass, "{}", r.text());
    assert_eq!(r.outcomes[0].detail, "the program is 15.542s (through the tempo curve)");
}

#[test]
fn a_sections_own_tempo_and_ramp_and_a_stack_have_the_maps_windows_too() {
    // a section's own tempos: the verse at 120, the chorus ramping 90 -> 150 over its 4 bars
    let src = "section verse 4 bars {\n  tempo 120\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  tempo 90 -> 150 over 4 bars\n  deck A: kick x x x x\n}\narrange verse chorus\nassert level A > 0 in verse\nassert level A > 0 in chorus\nassert level A > 0\n";
    let (c, r) = (compile(src), test(src));
    assert!(r.ok(), "{}", r.text());
    windows_are(&c, &r, 0, &[(0, 4)]);
    windows_are(&c, &r, 1, &[(4, 8)]);
    windows_are(&c, &r, 2, &[(0, 8)]);
    // 4 bars at 120 are 8 s; the ramp is 60 * 16 * ln(150 / 90) / 60 s
    assert!((r.outcomes[0].windows[0].1 - 8.0).abs() < 1e-3);
    assert!((r.outcomes[1].windows[0].1 - (8.0 + 16.0 * (150.0f64 / 90.0).ln())).abs() < 1e-3);
    // a stack of sections of unequal lengths under a curve: `short` plays in the first stretch, `long` in both
    let src = "tempo 100 -> 140 over 6 bars\nsection short 4 bars {\n  deck A: kick x x x x\n}\nsection long 8 bars {\n  deck B: bass c2 len=16\n}\narrange stack(short, long)\nassert level A > 0 in short\nassert level B > 0 in long\nassert level A > 0.001 in long\n";
    let (c, r) = (compile(src), test(src));
    windows_are(&c, &r, 0, &[(0, 4)]);
    windows_are(&c, &r, 1, &[(0, 4), (4, 8)]);
    assert!(r.outcomes[0].pass && r.outcomes[1].pass, "{}", r.text());
    // `short` stops at its bar 4, so deck A is silent in the second stretch of `long`: pooled over both stretches (the seconds of each
    // stretch from the map) its energy is the first stretch's over the whole, `x * sqrt(8.51 / 15.54)`
    let (x, pooled) = (r.outcomes[0].measured[0].1, r.outcomes[2].measured[0].1);
    let share = ((at(&c, 4) - at(&c, 0)) / (at(&c, 8) - at(&c, 0))).sqrt();
    assert!((pooled / (x * share) - 1.0).abs() < 0.03, "A over `long` is {pooled}; over `short` {x}, the share by the map {share}");
    // the same stack with a program that loops is measured over one lap
    let looped = test(&format!("{src}loop\n"));
    assert!((looped.rendered.as_ref().unwrap().seconds - c.tempo_map().total_seconds(8u32)).abs() < 2.0 / SR + 1e-9);
    windows_are(&c, &looped, 1, &[(0, 4), (4, 8)]);
}

/// The RMS of both channels of `audio` over `[from, to)` seconds, independent of `btw_test`'s own (from the map).
fn rms(audio: &RenderedAudio, from: f64, to: f64) -> f64 {
    let (i, j) = ((from * audio.sample_rate).round() as usize, ((to * audio.sample_rate).round() as usize).min(audio.left.len()));
    let sum: f64 = audio.left[i..j].iter().chain(&audio.right[i..j]).map(|x| f64::from(*x) * f64::from(*x)).sum();
    (sum / (2 * (j - i)) as f64).sqrt()
}

/// What the report measured is what an independent reading of the stems over the map's seconds gives.
#[test]
fn what_is_measured_is_the_sound_between_the_maps_seconds() {
    let src = format!("{MIXED}assert level B > 0 in verse\nassert level B > 0 in chorus\nassert level A > 0 in chorus\nassert level master > 0\n");
    let (c, r) = (compile(&src), test(&src));
    let (j, beats) = arrangement(&c);
    let (arr, bad) = Arrangement::from_json(&j).expect("an arrangement");
    assert_eq!(bad, 0);
    let rr = render_arrangement(&arr, ArrangeRenderOptions { sample_rate: SR, beats: Some(beats), stems: true, ..Default::default() });
    let stem = |n: &str| rr.stems.iter().find(|(k, _)| k == n).map(|(_, a)| a.clone()).unwrap_or_else(|| panic!("no stem {n}"));
    let (a, b) = (stem("A"), stem("B"));
    let want = [rms(&b, at(&c, 0), at(&c, 4)), rms(&b, at(&c, 4), at(&c, 8)), rms(&a, at(&c, 4), at(&c, 8)), rms(&rr.audio, at(&c, 0), at(&c, 8))];
    for (o, w) in r.outcomes.iter().zip(want) {
        let got = o.measured[0].1;
        assert!((got / w - 1.0).abs() < 0.005, "{}: the report measured {got}, the stem over the map's seconds is {w}", o.text);
    }
}

/// The same, where the sound changes at the section line (deck A in the verse only, deck B in the chorus only): the clock a
/// curve-blind reading would use (four bars at the first tempo, 9.6 s) puts 1.1 s of the chorus into the verse, so the number it
/// gives for B in the verse is far from the one the map gives, and the report's is the map's. The agreement is not by luck.
#[test]
fn a_window_that_ignored_the_curve_would_hear_the_next_section() {
    let src = "tempo 100 -> 140 over 6 bars\nsection verse 4 bars {\n  deck A: bass c2 len=16\n}\nsection chorus 4 bars {\n  deck B: bass c2 len=16\n}\narrange verse chorus\nassert level B > 0 in verse\nassert level A > 0 in verse\nassert level B > 0 in chorus\n";
    let (c, r) = (compile(src), test(src));
    let (j, beats) = arrangement(&c);
    let (arr, _) = Arrangement::from_json(&j).expect("an arrangement");
    let rr = render_arrangement(&arr, ArrangeRenderOptions { sample_rate: SR, beats: Some(beats), stems: true, ..Default::default() });
    let stem = |n: &str| rr.stems.iter().find(|(k, _)| k == n).map(|(_, a)| a.clone()).unwrap();
    let (a, b) = (stem("A"), stem("B"));
    let (by_map, naive) = (rms(&b, at(&c, 0), at(&c, 4)), rms(&b, 0.0, 4.0 * 4.0 * 60.0 / 100.0));
    let got = r.outcomes[0].measured[0].1;
    assert!((got / by_map - 1.0).abs() < 0.005, "the report measured {got}, the stem over the map's seconds is {by_map}");
    assert!(naive > 1.15 * by_map, "a window at the first tempo hears the chorus: {naive} against {by_map}");
    // and the rest of it: A is the verse's, B the chorus's
    assert!((r.outcomes[1].measured[0].1 / rms(&a, at(&c, 0), at(&c, 4)) - 1.0).abs() < 0.005);
    assert!((r.outcomes[2].measured[0].1 / rms(&b, at(&c, 4), at(&c, 8)) - 1.0).abs() < 0.005);
    assert!(r.outcomes[1].measured[0].1 > 2.0 * got, "deck A is the verse's sound, deck B only comes in at its end: {} against {got}", r.outcomes[1].measured[0].1);
}

/// The duck and the mix are in the render, so a level assertion reads them under a curve: deck B is down in the chorus after A's
/// kicks and A's fader is at 0.7, and the verse (before either) is exactly what it is without them.
#[test]
fn a_duck_and_a_mix_move_the_levels_a_level_assertion_reads_under_a_curve() {
    let asserts = "assert level A > 0 in verse\nassert level A > 0 in chorus\nassert level B > 0 in verse\nassert level B > 0 in chorus\nassert level master > 0 in chorus\n";
    let (ducked, flat) = (test(&format!("{MIXED}{asserts}")), test(&format!("{}{asserts}", plain())));
    assert!(ducked.ok() && flat.ok(), "{}\n{}", ducked.text(), flat.text());
    let m = |r: &Report, i: usize| r.outcomes[i].measured[0].1;
    // the verse comes before the moves: the same render, to the bit
    assert_eq!(m(&ducked, 0), m(&flat, 0));
    assert_eq!(m(&ducked, 2), m(&flat, 2));
    // the chorus: A at 0.7 of its fader, B pumped under A's kick, the master at 0.9 of what it was
    assert!(m(&ducked, 1) < 0.9 * m(&flat, 1), "A: {} of {}", m(&ducked, 1), m(&flat, 1));
    assert!(m(&ducked, 3) < 0.95 * m(&flat, 3), "B: {} of {}", m(&ducked, 3), m(&flat, 3));
    assert!(m(&ducked, 4) < 0.9 * m(&flat, 4), "master: {} of {}", m(&ducked, 4), m(&flat, 4));
    // and so an assertion written with a bound between the two numbers is the one that tells them apart
    let bound = (m(&ducked, 3) + m(&flat, 3)) / 2.0;
    let src = |base: &str| format!("{base}assert level B < {bound:.5} in chorus\nassert level B > {bound:.5} in verse\n");
    assert!(test(&src(MIXED)).ok(), "{}", test(&src(MIXED)).text());
    let r = test(&src(&plain()));
    assert!(!r.outcomes[0].pass && r.outcomes[1].pass, "{}", r.text());
    assert!(!r.ok());
}
