//! F4 ramp shapes and lfo (docs/beatscript/ext-shape.md): `... over 8 bars exp`, `lfo sine 2 bars center=3k depth=1k over 16 bars`.
//! The canonical forms, the exact values of every shape and every wave against hand formulas, the errors (with a place and a
//! did-you-mean), the same words in a `do` line, and the runtime on a fake clock: the values sent follow the shape, an lfo
//! stops where its section ends, the editor's progress bar moves for both.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, check_with, Compiled, Effect};
use djlive::dur::Dur;
use djlive::env::{Env, StaticSnippets};
use djlive::highlight::{highlight, Class};
use djlive::runtime::{Grid, Live, LiveHost};
use djlive::shape::{Lfo, Shape, Wave};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

/// (line, column, message, help) of every error.
fn errors(src: &str) -> Vec<(usize, usize, String, String)> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone().unwrap_or_default())).collect()
}

fn ramp(c: &Compiled, i: usize) -> djlive::compile::Ramp {
    match &c.controls[i].effect {
        Effect::Ramp(r) => r.clone(),
        other => panic!("not a ramp: {other:?}"),
    }
}

fn lfo(c: &Compiled, i: usize) -> Lfo {
    match &c.controls[i].effect {
        Effect::Lfo(l) => l.clone(),
        other => panic!("not an lfo: {other:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---- the hand formulas (written out here, independent of shape.rs)

fn hand_shape(name: &str, t: f64) -> f64 {
    let exp = |t: f64| (2f64.powf(4.0 * t) - 1.0) / 15.0;
    match name {
        "linear" => t,
        "exp" => exp(t),
        "log" => 1.0 - exp(1.0 - t),
        "s-curve" => 3.0 * t * t - 2.0 * t * t * t,
        n if n.starts_with("step ") => {
            let n: f64 = n[5..].parse().unwrap();
            ((t * n + 1e-9).floor() / (n - 1.0)).min(1.0)
        }
        _ => unreachable!(),
    }
}

fn hand_wave(name: &str, p: f64) -> f64 {
    let p = p - p.floor();
    match name {
        "sine" => (2.0 * std::f64::consts::PI * p).sin(),
        "tri" => (2.0 / std::f64::consts::PI) * (2.0 * std::f64::consts::PI * p).sin().asin(),
        "square" => {
            if (2.0 * std::f64::consts::PI * p).sin() >= 0.0 {
                1.0
            } else {
                -1.0
            }
        }
        "saw" => 2.0 * (p - (p + 0.5).floor()),
        _ => unreachable!(),
    }
}

// ---- canonical forms

#[test]
fn shapes_and_lfos_print_the_way_the_contract_says() {
    for (src, want) in [
        // a shape after the length; `linear` is the default and is not printed
        ("master: filter lp 300 -> 2k over 8 bars exp\n", "master: filter lp 300 -> 2000 over 8 bars exp\n"),
        ("master: filter lp 300 -> 2k over 8 bars log\n", "master: filter lp 300 -> 2000 over 8 bars log\n"),
        ("master: filter lp 300 -> 2k over 8 bars s-curve\n", "master: filter lp 300 -> 2000 over 8 bars s-curve\n"),
        ("master: filter lp 300 -> 2k over 8 bars step 4\n", "master: filter lp 300 -> 2000 over 8 bars step 4\n"),
        ("master: filter lp 300 -> 2k over 8 bars linear\n", "master: filter lp 300 -> 2000 over 8 bars\n"),
        ("master: filter lp 300 -> 2k over 8 bars\n", "master: filter lp 300 -> 2000 over 8 bars\n"),
        ("deck A: level 0.2 -> 0.9 over 1/2 bar exp\n", "deck A: level 0.2 -> 0.9 over 2 beats exp\n"),
        ("master: delay 0.1 -> 0.9 over 4 bars exp when 1 == 1\n", "master: delay 0.1 -> 0.9 over 4 bars exp when 1 == 1\n"),
        // an lfo: params sorted by name, a default left out
        ("master: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars\n", "master: filter lp lfo sine 2 bars center=3000 depth=1000 over 16 bars\n"),
        ("master: filter lp lfo sine 2 bars depth=1k center=3k over 16 bars\n", "master: filter lp lfo sine 2 bars center=3000 depth=1000 over 16 bars\n"),
        ("master: delay lfo tri 1 bar center=0.5 depth=1 phase=0\n", "master: delay lfo tri 1 bars center=0.5\n"),
        ("deck A: level lfo saw 4 beats depth=0.2 center=0.7 phase=1/4\n", "deck A: level lfo saw 1 bars center=0.7 depth=0.2 phase=0.25\n"),
        ("deck B: low lfo square 2 beats center=0.5 depth=0.5\n", "deck B: low lfo square 2 beats center=0.5 depth=0.5\n"),
        ("master: reverb lfo sine 1/2 bar center=0.5 depth=(1 / 4)\n", "master: reverb lfo sine 2 beats center=0.5 depth=0.25\n"),
        ("master: filter hp lfo tri 8 bars center=400 depth=200\n", "master: filter hp lfo tri 8 bars center=400 depth=200\n"),
        ("crossfader lfo sine 4 bars center=0.5 depth=0.5 over 8 bars\n", "crossfader lfo sine 4 bars center=0.5 depth=0.5 over 8 bars\n"),
        // in a `do` line, on a ramp and as a value
        ("do vocoder set mix=0 -> 1 over 8 bars exp\n", "do vocoder set mix=0 -> 1 over 8 bars exp\n"),
        ("do vocoder set mix=0 -> 1 over 8 bars step 3\n", "do vocoder set mix=0 -> 1 over 8 bars step 3\n"),
        ("do vocoder set mix=0 -> 1 over 8 bars linear\n", "do vocoder set mix=0 -> 1 over 8 bars\n"),
        ("do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3\n", "do inserts set param=delay.mix target=Master value=lfo tri 1 bars center=0.3 depth=0.2\n"),
        ("do vocoder set mix=lfo sine 2 bars depth=0.5 center=0.5 over 4 bars\n", "do vocoder set mix=lfo sine 2 bars center=0.5 depth=0.5 over 4 bars\n"),
        ("vocoder set mix=0.2 -> 0.9 over 2 bars s-curve\n", "vocoder set mix=0.2 -> 0.9 over 2 bars s-curve\n"),
    ] {
        let c = ok(src);
        assert_eq!(c.text, want, "{src}");
        // and the canonical text is a fixed point
        assert_eq!(ok(&c.text).text, c.text, "{src}");
    }
}

#[test]
fn two_spellings_of_one_shape_or_lfo_are_one_program_and_another_value_is_another() {
    let same = [
        ("master: delay 0.1 -> 0.9 over 4 bars", "master: delay 0.1 -> 0.9 over 4 bars linear"),
        ("master: delay 0.1 -> 0.9 over 4 bars exp", "master: delay 0.1 -> 0.9 over 16 beats exp"),
        ("master: delay lfo sine 2 bars", "master: delay lfo sine 8 beats depth=1"),
        ("master: delay lfo sine 2 bars center=0.5", "master: delay lfo sine 2 bars phase=0 center=0.5 depth=1"),
        ("master: delay lfo sine 2 bars center=0.5 depth=0.25", "master: delay lfo sine 2 bars depth=0.25 center=0.50"),
    ];
    for (a, b) in same {
        assert_eq!(ok(&format!("{a}\n")).cid, ok(&format!("{b}\n")).cid, "{a} / {b}");
    }
    let base = ok("master: delay 0.1 -> 0.9 over 4 bars exp\n").cid;
    for other in [
        "master: delay 0.1 -> 0.9 over 4 bars log",
        "master: delay 0.1 -> 0.9 over 4 bars s-curve",
        "master: delay 0.1 -> 0.9 over 4 bars step 4",
        "master: delay 0.1 -> 0.9 over 4 bars",
        "master: delay 0.1 -> 0.8 over 4 bars exp",
        "master: delay 0.1 -> 0.9 over 5 bars exp",
    ] {
        assert_ne!(ok(&format!("{other}\n")).cid, base, "{other}");
    }
    assert_ne!(ok("master: delay 0.1 -> 0.9 over 4 bars step 4\n").cid, ok("master: delay 0.1 -> 0.9 over 4 bars step 5\n").cid);
    let lfo_base = ok("master: delay lfo sine 2 bars center=0.5 depth=0.25\n").cid;
    for other in [
        "master: delay lfo tri 2 bars center=0.5 depth=0.25",
        "master: delay lfo sine 3 bars center=0.5 depth=0.25",
        "master: delay lfo sine 2 bars center=0.5 depth=0.3",
        "master: delay lfo sine 2 bars center=0.4 depth=0.25",
        "master: delay lfo sine 2 bars center=0.5 depth=0.25 phase=0.5",
        "master: delay lfo sine 2 bars center=0.5 depth=0.25 over 8 bars",
        "master: reverb lfo sine 2 bars center=0.5 depth=0.25",
    ] {
        assert_ne!(ok(&format!("{other}\n")).cid, lfo_base, "{other}");
    }
}

#[test]
fn a_plain_ramp_and_an_unshaped_program_keep_the_tree_they_had() {
    // `linear` and no shape are the same tree as before F4: Action::Change, a Ramp with the linear shape
    let c = ok("master: filter lp 300 -> 2k over 8 bars linear\nmaster: delay 0.4\n");
    assert_eq!(ramp(&c, 0).shape, Shape::Linear);
    assert_eq!(c.text, "master: filter lp 300 -> 2000 over 8 bars\nmaster: delay 0.4\n");
}

// ---- the values: shapes

#[test]
fn every_shape_is_its_hand_formula_at_the_quarters() {
    for name in ["linear", "exp", "log", "s-curve", "step 4", "step 5", "step 2", "step 16"] {
        let src = format!("deck A: level 0.2 -> 0.8 over 4 bars {name}\n");
        let c = ok(&src);
        let r = ramp(&c, 0);
        assert_eq!((r.from, r.to, r.bars), (0.2, 0.8, Dur::bars(4)));
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let want = 0.2 + 0.6 * hand_shape(name, t);
            assert!(close(r.value(t), want), "{name} at {t}: {} vs {want}", r.value(t));
            // `at` is the call with the same value, rounded to a millionth
            let sent = r.at(t).params.get("volume").and_then(JsonValue::as_f64).unwrap();
            assert!((sent - want).abs() < 1e-6, "{name} at {t}");
        }
        // the ends are exact
        assert_eq!((r.value(0.0), r.value(1.0)), (0.2, 0.8_f64.min(0.2 + 0.6)), "{name}");
    }
    // by hand: exp at 1/4, 1/2, 3/4 is 1/15, 1/5, 7/15 of the way; log is its mirror; the s-curve 5/32, 1/2, 27/32
    let r = ramp(&ok("deck A: level 0 -> 1 over 4 bars exp\n"), 0);
    assert!(close(r.value(0.25), 1.0 / 15.0) && close(r.value(0.5), 0.2) && close(r.value(0.75), 7.0 / 15.0));
    let r = ramp(&ok("deck A: level 0 -> 1 over 4 bars log\n"), 0);
    assert!(close(r.value(0.25), 8.0 / 15.0) && close(r.value(0.5), 0.8) && close(r.value(0.75), 14.0 / 15.0));
    let r = ramp(&ok("deck A: level 0 -> 1 over 4 bars s-curve\n"), 0);
    assert!(close(r.value(0.25), 5.0 / 32.0) && close(r.value(0.5), 0.5) && close(r.value(0.75), 27.0 / 32.0));
}

#[test]
fn step_n_is_n_values_from_the_start_to_the_end() {
    let r = ramp(&ok("deck A: level 0 -> 0.9 over 4 bars step 4\n"), 0);
    let at = |t: f64| r.value(t);
    // four values: 0, 1/3, 2/3 and 1 of the way (0.9), each held for a quarter
    assert!(close(at(0.0), 0.0) && close(at(0.24), 0.0));
    assert!(close(at(0.25), 0.3) && close(at(0.49), 0.3));
    assert!(close(at(0.5), 0.6) && close(at(0.74), 0.6));
    assert!(close(at(0.75), 0.9) && close(at(1.0), 0.9));
    // two steps: the start, then the end
    let r = ramp(&ok("deck A: level 0.1 -> 0.5 over 2 bars step 2\n"), 0);
    assert!(close(r.value(0.49), 0.1) && close(r.value(0.5), 0.5));
    // a shape goes the way down too
    let r = ramp(&ok("deck A: level 1 -> 0.4 over 4 bars exp\n"), 0);
    assert!(close(r.value(0.5), 1.0 - 0.6 * 0.2));
    // a filter's ends are knobs: the shape is of the knob, as a linear ramp is
    let r = ramp(&ok("master: filter lp 300 -> 2k over 8 bars s-curve\n"), 0);
    assert!(close(r.value(0.5), (r.from + r.to) / 2.0));
}

// ---- the values: lfos

#[test]
fn every_wave_is_its_hand_formula_at_known_phases() {
    for wave in ["sine", "tri", "square", "saw"] {
        for (extra, phase) in [("", 0.0), (" phase=0.25", 0.25), (" phase=0.5", 0.5), (" phase=1", 1.0)] {
            let src = format!("master: level lfo {wave} 4 bars center=0.5 depth=0.25{extra}\n");
            let c = ok(&src);
            let l = lfo(&c, 0);
            assert_eq!((l.period, l.center, l.depth, l.phase, l.over), (Dur::bars(4), 0.5, 0.25, phase, None), "{wave}{extra}");
            // the quarters of the period, 4 bars: 0, 1, 2, 3, 4 bars in, and a few between
            for p in [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 1.25, 2.75] {
                let x = p * 4.0;
                // (a point on a jump of a square or a saw is a coin toss for a float: not asked)
                if matches!(wave, "square" | "saw") && ((p + phase) * 2.0 - ((p + phase) * 2.0).round()).abs() < 1e-9 {
                    continue;
                }
                let want = 0.5 + 0.25 * hand_wave(wave, p + phase);
                assert!(close(l.value_at(x), want), "{wave}{extra} at {p} periods: {} vs {want}", l.value_at(x));
            }
        }
    }
    // the plain facts, by hand (centre 0.5, depth 0.25, a period 4 bars)
    let val = |wave: &str, p: f64| lfo(&ok(&format!("master: level lfo {wave} 4 bars center=0.5 depth=0.25\n")), 0).value_at(p * 4.0);
    assert!(close(val("sine", 0.0), 0.5) && close(val("sine", 0.25), 0.75) && close(val("sine", 0.5), 0.5) && close(val("sine", 0.75), 0.25));
    assert!(close(val("tri", 0.125), 0.625) && close(val("tri", 0.25), 0.75) && close(val("tri", 0.75), 0.25));
    assert!(close(val("square", 0.25), 0.75) && close(val("square", 0.75), 0.25));
    // `saw` rises: 0 at the start, up to the top at the half, where it falls to the bottom and rises again
    assert!(close(val("saw", 0.0), 0.5) && close(val("saw", 0.25), 0.625) && close(val("saw", 0.5), 0.25) && close(val("saw", 0.75), 0.375));
    assert!(val("saw", 0.49) > 0.74);
}

#[test]
fn the_rate_is_a_duration_in_bars_or_beats() {
    for (rate, bars) in [("2 bars", 2.0), ("1/2 bar", 0.5), ("4 beats", 1.0), ("3 beats", 0.75), ("(1 + 1) bars", 2.0), ("1/4 beat", 1.0 / 16.0)] {
        let l = lfo(&ok(&format!("master: delay lfo sine {rate} center=0.5 depth=0.5\n")), 0);
        assert!(close(l.period.to_f64(), bars), "{rate}");
        // a period later it is where it began
        assert!(close(l.value_at(bars), l.value_at(0.0)), "{rate}");
    }
}

#[test]
fn a_filter_lfo_swings_between_the_knobs_of_its_ends() {
    // center 3k, depth 1k: between 2k and 4k, linear in the knob as a ramp's ends are
    let l = lfo(&ok("master: filter lp lfo sine 2 bars center=3k depth=1k\n"), 0);
    let (lo, hi) = (djlive::compile::sweep_knob(2000.0, false), djlive::compile::sweep_knob(4000.0, false));
    assert!(close(l.center, (lo + hi) / 2.0) && close(l.depth, (hi - lo) / 2.0), "{l:?}");
    // a quarter period in it is at 4 kHz, three quarters at 2 kHz
    assert!(close(l.value_at(0.5), hi) && close(l.value_at(1.5), lo));
    // the prelude turns the filter on, and the field is the sweep
    assert_eq!(l.prelude.len(), 1);
    assert_eq!((l.noun.as_str(), l.field.as_str()), ("inserts", "value"));
    // a high-pass is the same with its own knob
    let l = lfo(&ok("deck A: filter hp lfo tri 4 bars center=400 depth=300\n"), 0);
    let (lo, hi) = (djlive::compile::sweep_knob(100.0, true), djlive::compile::sweep_knob(700.0, true));
    assert!(close(l.value_at(1.0), hi) && close(l.value_at(3.0), lo), "{l:?}");
    // level on a deck and the master, and the others, are their own fields
    assert_eq!(lfo(&ok("deck A: level lfo sine 2 bars center=0.5 depth=0.25\n"), 0).field, "volume");
    assert_eq!(lfo(&ok("master: level lfo sine 2 bars center=0.5 depth=0.25\n"), 0).field, "master");
    assert_eq!(lfo(&ok("deck B: mid lfo sine 2 bars center=0.5 depth=0.25\n"), 0).field, "mid");
    assert_eq!(lfo(&ok("crossfader lfo sine 2 bars center=0.5 depth=0.25\n"), 0).field, "crossfader");
}

#[test]
fn an_lfo_that_leaves_the_range_is_held_at_the_edge_and_says_so() {
    // the defaults are center 0 and depth 1: -1 to 1 on a 0 to 1 param
    let c = check("master: delay lfo sine 2 bars\n");
    let comp = c.compiled.unwrap();
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("outside the range of `delay` (0 to 1)")), "{:?}", c.diagnostics);
    let l = lfo(&comp, 0);
    assert!(l.value_at(0.0) >= 0.0 && l.value_at(0.5) <= 1.0 && l.value_at(1.5) >= 0.0, "{l:?}");
    // a filter outside its cutoffs
    let c = check("master: filter lp lfo sine 2 bars center=100 depth=1k\n");
    assert!(c.compiled.is_some());
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("low-pass cutoff (60 to 20000)")), "{:?}", c.diagnostics);
    // one that stays in range says nothing
    let c = check("master: delay lfo sine 2 bars center=0.5 depth=0.5\n");
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

// ---- errors

#[test]
fn errors_say_where_and_what_would_do() {
    // a near miss is named
    let e = errors("master: filter lp lfo sin 2 bars center=3k depth=1k\n");
    assert_eq!((e[0].0, e[0].1), (1, 23), "{e:?}");
    assert!(e[0].2.contains("`sin` is not a wave") && e[0].3.contains("did you mean `sine`?"), "{e:?}");
    let e = errors("master: delay lfo triangle 2 bars\n");
    assert!(e[0].3.contains("did you mean `tri`?"), "{e:?}");
    let e = errors("master: delay 0.1 -> 0.9 over 4 bars expo\n");
    assert_eq!((e[0].0, e[0].1), (1, 38), "{e:?}");
    assert!(e[0].2.contains("`expo` is not a shape") && e[0].3.contains("did you mean `exp`?"), "{e:?}");
    let e = errors("master: delay 0.1 -> 0.9 over 4 bars scurve\n");
    assert!(e[0].3.contains("did you mean `s-curve`?"), "{e:?}");
    let e = errors("master: delay lfo sine 2 bars deph=0.5\n");
    assert!(e[0].2.contains("`deph` is not a parameter of an lfo") && e[0].3.contains("did you mean `depth`?"), "{e:?}");
    // a shape needs a ramp, and a ramp needs its length
    let e = errors("master: delay 0.4 exp\n");
    assert!(e[0].2.contains("`exp` needs a ramp: `0.4 -> 1 over 4 bars exp`"), "{e:?}");
    let e = errors("master: delay 0.1 -> 0.9 exp\n");
    assert!(e[0].2.contains("`exp` needs the ramp's length") && e[0].3.contains("over 4 bars exp"), "{e:?}");
    let e = errors("master: delay 0.1 -> 0.9 over 4 bars step 1\n");
    assert!(e[0].2.contains("`step N` takes N from 2"), "{e:?}");
    let e = errors("master: delay 0.1 -> 0.9 over 4 bars step 2000\n");
    assert!(e[0].2.contains("`step N` takes N from 2 to 1024"), "{e:?}");
    // what an lfo takes
    for (src, want) in [
        ("master: delay lfo 2 bars\n", "an lfo needs a wave"),
        ("master: delay lfo sine\n", "expected the lfo's period"),
        ("master: delay lfo sine 0 bars\n", "an lfo's period takes some time"),
        ("master: delay lfo sine 1/8 beat\n", "faster than the clock"),
        ("master: delay lfo sine 2 bars depth=-1\n", "an lfo's depth is not negative"),
        ("master: delay lfo sine 2 bars phase=1.5\n", "an lfo's phase is 0 to 1"),
        ("master: delay lfo sine 2 bars depth=1 depth=2\n", "`depth` is given twice"),
        ("master: delay lfo sine 2 bars over 0 bars\n", "runs for no time"),
        ("master: delay lfo sine 2 bars center=2 depth=0.1\n", "outside the range"),
        ("master: delay lfo sine 2 bars depth=5db\n", "`depth` is a plain number in the param's own unit, not `5db`"),
        ("master: filter lp lfo sine 2 bars center=440hz depth=100\n", "`center` is a plain number in the param's own unit, not `440hz`"),
        ("master: delay lfo sine 2 bars phase=2s\n", "`phase` is a plain number"),
    ] {
        let c = check(src);
        let msgs: Vec<String> = c.diagnostics.iter().map(|d| d.message.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains(want)), "{src:?}: wanted `{want}`, got {msgs:?}");
    }
    // a `do` line: a shape needs a ramp, an lfo and a ramp are two moving params
    let e = errors("do vocoder set mix=0.5 over 8 bars exp\n");
    assert!(e.iter().any(|x| x.2.contains("`exp` needs a ramp")), "{e:?}");
    let e = errors("do vocoder set mix=lfo sine 2 bars depth=0.5 center=0.5 level=0 -> 1 over 4 bars\n");
    assert!(e.iter().any(|x| x.2.contains("one param a line can move")), "{e:?}");
    let e = errors("do vocoder set mix=lfo sine 2 bars exp\n");
    assert!(e.iter().any(|x| x.2.contains("`exp` needs a ramp")), "{e:?}");
}

// ---- the same in a `do` line

#[test]
fn a_do_line_takes_a_shape_and_an_lfo() {
    let c = ok("do vocoder set mix=0 -> 1 over 8 bars exp\n");
    let r = ramp(&c, 0);
    assert_eq!((r.shape, r.from, r.to, r.field.as_str(), r.noun.as_str()), (Shape::Exp, 0.0, 1.0, "mix", "vocoder"));
    assert!(close(r.value(0.5), 0.2));
    let c = ok("do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3\n");
    let l = lfo(&c, 0);
    assert_eq!((l.noun.as_str(), l.field.as_str(), l.wave, l.center, l.depth, l.over, l.until), ("inserts", "value", Wave::Tri, 0.3, 0.2, None, None));
    // the other params of the call are in its base
    let base = l.base.canonical_js();
    assert!(base.contains("\"param\":\"delay.mix\"") && base.contains("\"target\":\"Master\"") && !base.contains("\"value\""), "{base}");
    let sent = l.call_at(0.25).params;
    assert_eq!(sent.get("value").and_then(JsonValue::as_f64), Some(0.5));
    // `over` after the lfo is the lfo's own
    let l = lfo(&ok("do vocoder set mix=lfo sine 2 bars depth=0.5 center=0.5 over 4 bars\n"), 0);
    assert_eq!(l.over, Some(Dur::bars(4)));
    // sugar and a param that is not last
    let c = ok("vocoder set mix=lfo sine 2 bars depth=0.5 center=0.5 level=0.7\n");
    assert!(lfo(&c, 0).base.canonical_js().contains("\"level\":0.7"));
}

#[test]
fn an_lfo_in_a_section_knows_where_the_section_ends() {
    let c = ok("section verse 4 bars {\n  deck A: kick x\n  master: level lfo tri 1 bar center=0.5 depth=0.25\n}\nsection chorus 6 bars {\n  deck A: kick x x\n  master: level lfo tri 1 bar center=0.5 depth=0.25 over 2 bars\n  master: delay lfo sine 2 bars center=0.5 depth=0.25\n}\narrange verse chorus\nmaster: reverb lfo sine 4 bars center=0.5 depth=0.25\n");
    let until: Vec<Option<u32>> = c.controls.iter().map(|k| if let Effect::Lfo(l) = &k.effect { l.until } else { panic!() }).collect();
    // (the song's lines are in the order they are compiled: the outside lines first)
    assert_eq!(until.len(), 4);
    assert!(until.contains(&None) && until.contains(&Some(4)) && until.iter().filter(|u| **u == Some(10)).count() == 2, "{until:?}");
}

// ---- the runtime on a fake clock

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }
    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn field(&self, noun: &str, field: &str) -> Vec<(f64, f64)> {
        self.calls
            .iter()
            .filter(|c| c.1 == noun)
            .filter_map(|c| JsonValue::parse(&c.2).ok().and_then(|p| p.get(field).and_then(JsonValue::as_f64)).map(|v| (c.0, v)))
            .collect()
    }

    fn master_levels(&self) -> Vec<(f64, f64)> {
        self.field("mixer", "master")
    }
}

/// A deck A that plays at beat 8: program bar 0 is the deck's bar 2.
fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

/// The beat of program position `p` bars.
fn beat_at(p: f64) -> f64 {
    8.0 + 4.0 * p
}

#[test]
fn the_values_sent_follow_the_shape() {
    for name in ["linear", "exp", "log", "s-curve", "step 4", "step 7"] {
        let (mut h, mut live) = playing();
        assert!(live.eval(&format!("at bar 2: master: level 0.1 -> 0.9 over 4 bars {name}\n")).ok);
        h.run(&mut live, 4.0 * 9.0);
        let sends = h.master_levels();
        let start = beat_at(2.0);
        assert_eq!(sends.first().copied(), Some((start, 0.1)), "{name}: {sends:?}");
        // the last send is the end value, exactly four bars on, and nothing after
        assert_eq!(sends.last().copied(), Some((start + 16.0, 0.9)), "{name}: {sends:?}");
        for (b, v) in &sends {
            let t = (b - start) / 16.0;
            let want = 0.1 + 0.8 * hand_shape(name, t);
            assert!((v - want).abs() < 1e-6, "{name} at beat {b} (t {t}): sent {v}, the shape says {want}");
        }
        // it never goes back, and it does not repeat itself
        assert!(sends.windows(2).all(|w| w[0].1 <= w[1].1), "{name}: {sends:?}");
        match name {
            // a linear ramp sends a lot; a slow-start one fewer at the start; a step sends once per value
            "step 4" => {
                let mut vs: Vec<f64> = sends.iter().map(|s| s.1).collect();
                vs.dedup();
                assert_eq!(vs, vec![0.1, 0.1 + 0.8 / 3.0, 0.1 + 1.6 / 3.0, 0.9].iter().map(|x| (x * 1e6_f64).round() / 1e6).collect::<Vec<_>>(), "{sends:?}");
                // each at the beat the step begins: a quarter of the ramp is four beats
                let at: Vec<f64> = sends.iter().map(|s| s.0 - start).collect();
                assert_eq!(at, vec![0.0, 4.0, 8.0, 12.0, 16.0], "{sends:?}");
            }
            "linear" => assert!(sends.len() > 20, "{}", sends.len()),
            _ => {}
        }
    }
}

#[test]
fn a_shaped_ramp_of_a_filter_follows_it_in_the_knob() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2: master: filter lp 300 -> 2k over 4 bars exp\n").ok);
    h.run(&mut live, 4.0 * 8.0);
    let sends = h.field("inserts", "value");
    let (a, b) = (djlive::compile::sweep_knob(300.0, false), djlive::compile::sweep_knob(2000.0, false));
    let start = beat_at(2.0);
    for (beat, v) in &sends {
        let t = (beat - start) / 16.0;
        assert!((v - (a + (b - a) * hand_shape("exp", t))).abs() < 1e-6, "{beat}: {v}");
    }
    assert_eq!(sends.last().map(|s| s.1), Some((b * 1e6_f64).round() / 1e6));
}

#[test]
fn an_lfo_sends_center_plus_depth_times_the_wave_and_ends_on_its_span() {
    for wave in ["sine", "tri", "saw", "square"] {
        let (mut h, mut live) = playing();
        // a period of 2 bars, for 4: two whole periods
        assert!(live.eval(&format!("at bar 2: master: level lfo {wave} 2 bars center=0.5 depth=0.25 over 4 bars\n")).ok);
        h.run(&mut live, 4.0 * 9.0);
        let sends = h.master_levels();
        let start = beat_at(2.0);
        assert_eq!(sends.first().map(|s| s.0), Some(start), "{wave}");
        // the last send is at the end of the span: two whole periods in, the same as the start (a square's first value)
        let last = sends.last().unwrap();
        assert_eq!(last.0, start + 16.0, "{wave}: {sends:?}");
        for (b, v) in &sends {
            let x_bars = (b - start) / 4.0;
            // (a square or a saw on its jump: the pass that lands there takes the value of its own float)
            let want = 0.5 + 0.25 * hand_wave(wave, x_bars / 2.0);
            let on_jump = matches!(wave, "square" | "saw") && ((x_bars / 2.0) * 2.0 - ((x_bars / 2.0) * 2.0).round()).abs() < 1e-9;
            if !on_jump {
                assert!((v - want).abs() < 1e-6, "{wave} at beat {b}: sent {v}, the wave says {want}");
            }
        }
        // it was sent often enough to draw the wave, and nothing after its end
        // (a square changes twice a period)
        assert!(sends.len() >= if wave == "square" { 4 } else { 8 }, "{wave}: {}", sends.len());
        assert!(sends.iter().all(|s| s.0 <= start + 16.0), "{wave}");
    }
}

#[test]
fn an_lfo_with_no_over_stops_where_its_section_ends() {
    let src = "section verse 4 bars {\n  deck A: kick x\n  master: level lfo tri 1 bar center=0.5 depth=0.25\n}\nsection chorus 4 bars {\n  deck A: kick x x\n}\narrange verse chorus\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 12.0);
    let sends = h.master_levels();
    let start = sends.first().unwrap().0;
    // the song's first bar is the first bar line after the evaluation
    assert_eq!(start, beat_at(1.0), "{sends:?}");
    // the last send is the section's end (4 bars on): four whole periods in, at the centre again, and no more after it
    let last = sends.last().unwrap();
    assert_eq!(last.0, start + 16.0, "{sends:?}");
    assert!((last.1 - 0.5).abs() < 1e-9);
    for (b, v) in &sends {
        let x = (b - start) / 4.0;
        assert!((v - (0.5 + 0.25 * hand_wave("tri", x))).abs() < 1e-6, "{b}: {v}");
    }
    // and an lfo of the second section does not start until the section does
    let src = "section verse 4 bars {\n  deck A: kick x\n}\nsection chorus 4 bars {\n  deck A: kick x x\n  master: level lfo sine 1 bar center=0.5 depth=0.25\n}\narrange verse chorus\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 12.0);
    let sends = h.master_levels();
    assert_eq!(sends.first().unwrap().0, beat_at(1.0) + 16.0, "{sends:?}");
    // (the last section's lfo is stopped with the program: the pass before the end is its last)
    assert_eq!(sends.last().unwrap().0, beat_at(1.0) + 32.0 - 0.25, "{sends:?}");
}

#[test]
fn an_lfo_that_is_held_stops_when_what_it_did_is_put_back() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2 for 2 bars: master: filter lp lfo sine 1 bar center=2k depth=1k\n").ok);
    h.run(&mut live, 4.0 * 8.0);
    let sweeps = h.field("inserts", "value");
    let start = beat_at(2.0);
    assert!(sweeps.len() > 6, "{sweeps:?}");
    assert_eq!(sweeps.last().unwrap().0, start + 8.0, "{sweeps:?}");
    let offs: Vec<f64> = h.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("\"on\":false")).map(|c| c.0).collect();
    assert_eq!(offs, vec![start + 8.0]);
}

#[test]
fn a_do_lfo_sends_its_call_with_the_wave_in_the_value() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2: do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3 over 2 bars\n").ok);
    h.run(&mut live, 4.0 * 6.0);
    let sends = h.field("inserts", "value");
    let start = beat_at(2.0);
    assert!(sends.len() > 8, "{sends:?}");
    for (b, v) in &sends {
        let x = (b - start) / 4.0;
        assert!((v - (0.3 + 0.2 * hand_wave("tri", x))).abs() < 1e-6, "{b}: {v}");
    }
    assert_eq!(sends.last().unwrap().0, start + 8.0);
    assert!(h.calls.iter().all(|c| c.2.contains("\"param\":\"delay.mix\"") && c.2.contains("\"target\":\"Master\"")));
}

#[test]
fn the_editors_progress_bar_moves_for_a_shaped_ramp_and_an_lfo() {
    let (mut h, mut live) = playing();
    assert!(live
        .eval("at bar 2: master: level 0.1 -> 0.9 over 4 bars exp\nat bar 2: master: delay lfo sine 2 bars center=0.5 depth=0.25 over 4 bars\nat bar 2: master: reverb lfo sine 2 bars center=0.5 depth=0.25\n")
        .ok);
    // half way through the span (a ramp, an lfo with an `over`) and a quarter into the period (an lfo with none)
    h.run(&mut live, beat_at(2.0) - 8.0 + 8.0);
    let m = live.marks();
    let progress = |needle: &str| m.controls.iter().find(|c| c.key.contains(needle)).and_then(|c| c.ramp);
    assert!((progress("level 0.1").unwrap() - 0.5).abs() < 0.03, "{:?}", m.controls);
    assert!((progress("delay lfo").unwrap() - 0.5).abs() < 0.03, "{:?}", m.controls);
    // the reverb lfo has no end: its progress is how far into its 2 bars it is (8 beats in: a whole period 0 and 1)
    let p = progress("reverb lfo").unwrap();
    assert!((0.0..=1.0).contains(&p));
    // the JSON of the marks has it
    let j = m.to_json();
    let controls = j.get("controls").and_then(JsonValue::as_array).unwrap();
    assert!(controls.iter().any(|c| c.get("progress").and_then(JsonValue::as_f64).is_some()));
    // and after the end there is no progress left
    h.run(&mut live, 4.0 * 4.0);
    let m = live.marks();
    assert!(m.controls.iter().filter(|c| c.key.contains("level 0.1") || c.key.contains("delay lfo")).all(|c| c.ramp.is_none()), "{:?}", m.controls);
    // `get` calls a running lfo ramping
    let st = live.status();
    let states: Vec<String> = st.get("controls").and_then(JsonValue::as_array).unwrap().iter().filter_map(|c| c.get("state").and_then(JsonValue::as_str).map(str::to_string)).collect();
    assert!(states.contains(&"ramping".to_string()), "{states:?}");
}

// ---- what edits do to a running one

#[test]
fn editing_a_shape_replaces_the_ramp_and_leaving_it_alone_keeps_it() {
    let a = "at bar 2: master: level 0.1 -> 0.9 over 8 bars exp\ndeck A: kick x\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(a).ok);
    h.run(&mut live, 4.0 * 3.0);
    let before = h.master_levels().len();
    // a change elsewhere: the ramp keeps its state (no restart)
    assert!(live.eval(&a.replace("kick x", "kick x ~ x ~")).ok);
    h.run(&mut live, 4.0 * 1.0);
    let sends = h.master_levels();
    assert!(sends.len() > before && sends.iter().filter(|s| s.1 == 0.1).count() == 1, "{sends:?}");
    // a changed shape is another line, which starts again
    let ev = live.eval(&a.replace("exp", "log"));
    assert!(ev.ok);
    let plan = ev.plan.unwrap();
    assert_eq!((plan.controls_added.len(), plan.controls_removed.len()), (1, 1), "{plan:?}");
}

// ---- through defs, repeats and imports; reformatting; the editor's colours

#[test]
fn shapes_and_lfos_through_a_def_a_repeat_and_an_import() {
    let src = "def sweep(deck, n, c) {\n  deck $deck: level lfo sine $n bars center=$c depth=0.25\n  deck $deck: delay 0.1 -> 0.9 over $n bars exp\n}\nrepeat 3 every 2 bars {\n  sweep(A, 1 + $i, 0.4 + $i * 0.1)\n}\n";
    let c = ok(src);
    assert_eq!(c.controls.len(), 6);
    let l: Vec<Lfo> = c.controls.iter().filter_map(|k| if let Effect::Lfo(l) = &k.effect { Some(l.clone()) } else { None }).collect();
    assert_eq!(l.iter().map(|x| x.period.whole_bars().unwrap()).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(l.iter().enumerate().all(|(i, x)| close(x.center, 0.4 + i as f64 * 0.1)));
    assert!(c.controls.iter().filter(|k| matches!(&k.effect, Effect::Ramp(r) if r.shape == Shape::Exp)).count() == 3);
    // an imported def
    let mut snips = StaticSnippets::default();
    let base = "def swell(n) {\n  master: reverb 0.1 -> 0.7 over $n bars s-curve\n  master: delay lfo tri $n bars center=0.5 depth=0.25 over 8 bars\n}\n";
    let canon = check(base).compiled.unwrap();
    snips.items.push(("swells".to_string(), canon.cid.clone(), canon.text.clone()));
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"swells\"\nsection intro 4 bars {\n  deck A: kick x\n  swell(2)\n}\nsection drop 4 bars {\n  deck A: kick x x\n  swell(4)\n}\narrange intro drop\n";
    let c = check_with(src, &env).compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&check_with(src, &env).diagnostics, src)));
    assert_eq!(c.controls.len(), 4);
    assert!(matches!(&c.controls[0].effect, Effect::Ramp(r) if r.shape == Shape::SCurve));
    // reading the canonical text back is the same program
    let again = check_with(&c.text, &env).compiled.unwrap();
    assert_eq!(again.cid, c.cid);
}

#[test]
fn reformatting_a_program_with_shapes_and_lfos_keeps_its_cid() {
    let programs = [
        "tempo 124\nmaster: filter lp 300 -> 2k over 8 bars exp\nmaster: delay 0.1 -> 0.9 over 1/2 bar s-curve\ndeck A: level 0.2 -> 0.8 over 4 bars step 4\ndeck B: kick x ~ x ~\n",
        "master: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars\ndeck A: level lfo tri 1/2 bar center=0.5 depth=0.25 phase=0.25\nmaster: reverb lfo saw 4 beats center=0.5 depth=1/4\n",
        "do vocoder set mix=0 -> 1 over 8 bars exp\ndo inserts set target=Master param=delay.mix value=lfo square 1 bar depth=0.2 center=0.3 over 2 bars\n",
        "section intro 4 bars {\n  deck A: kick x\n  master: filter hp 100 -> 5000 over 4 bars log\n  master: level lfo sine 1 bar center=0.5 depth=0.25\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n",
        "def sweep(deck, n) {\n  deck $deck: level lfo sine $n bars center=0.5 depth=0.25\n  at bar $n: deck $deck: delay 0.1 -> 0.9 over $n bars exp\n}\nrepeat 2 every 4 bars {\n  sweep(A, 2)\n}\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let c = ok(src);
        let again = ok(&c.text);
        assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()), "program {n}");
        let mut rng = Rng(0x5a4e_0000 + n as u64);
        for round in 0..120 {
            let t = reformat(src, &mut rng);
            let got = check(&t);
            let got = got.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{t}\n{}", djlive::diag::render_all(&got.diagnostics, &t)));
            assert_eq!(got.cid, c.cid, "program {n} round {round} forked:\n{t}");
        }
    }
}

#[test]
fn random_shapes_and_lfos_are_their_formulas_and_their_canonical_text_reads_back() {
    let mut rng = Rng(0x4f4_4000_77);
    let shapes = ["", " linear", " exp", " log", " s-curve", " step 3", " step 9", " step 2"];
    let waves = ["sine", "tri", "square", "saw"];
    let rates = ["1 bar", "2 bars", "1/2 bar", "3 beats", "5 beats", "4 bars", "1/4 bar"];
    for round in 0..300 {
        let a = rng.below(90) as f64 / 100.0;
        let b = rng.below(90) as f64 / 100.0 + 0.05;
        let shape = shapes[rng.below(shapes.len() as u64) as usize];
        let bars = 1 + rng.below(8);
        let src = format!("deck A: level {a} -> {b} over {bars} bars{shape}\n");
        let c = ok(&src);
        let r = ramp(&c, 0);
        let name = shape.trim();
        let name = if name.is_empty() { "linear" } else { name };
        for t in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            let want = a + (b - a) * hand_shape(name, t);
            assert!((r.value(t) - want).abs() < 1e-9, "{src} t={t}");
        }
        assert_eq!(ok(&c.text).cid, c.cid, "{src}");
        // an lfo
        let wave = waves[rng.below(4) as usize];
        let rate = rates[rng.below(rates.len() as u64) as usize];
        let (center, depth) = (0.3 + rng.below(40) as f64 / 100.0, rng.below(30) as f64 / 100.0);
        let phase = rng.below(101) as f64 / 100.0;
        let src = format!("master: level lfo {wave} {rate} center={center} depth={depth} phase={phase}\n");
        let c = ok(&src);
        let l = lfo(&c, 0);
        for x in [0.0, 0.3, 0.77, 1.0, 2.5] {
            let want = center + depth * hand_wave(wave, x / l.period.to_f64() + phase);
            // (not asked on the jump of a square or a saw)
            let q = x / l.period.to_f64() + phase;
            if matches!(wave, "square" | "saw") && ((q * 2.0) - (q * 2.0).round()).abs() < 1e-6 {
                continue;
            }
            assert!((l.value_at(x) - want).abs() < 1e-9, "round {round}: {src} x={x}");
        }
        let again = ok(&c.text);
        assert_eq!(again.cid, c.cid, "{src}");
    }
}

#[test]
fn the_editor_draws_the_new_words() {
    let src = "master: filter lp lfo sine 2 bars center=3k depth=1k phase=0\nmaster: delay 0.1 -> 0.9 over 4 bars s-curve\ndeck A: level 0 -> 1 over 4 bars step 4\n";
    let classes: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    let class_of = |w: &str| classes.iter().find(|(t, _)| *t == w).map(|(_, c)| *c);
    for w in ["lfo", "sine", "s-curve", "step"] {
        assert_eq!(class_of(w), Some(Class::Keyword), "{w}");
    }
    for w in ["center", "depth", "phase"] {
        assert_eq!(class_of(w), Some(Class::Param), "{w}");
    }
}

#[test]
fn the_new_words_are_reserved() {
    // `lfo` starts a value, `s-curve` is one word: they name nothing. The waves, the shapes and the params are read as syntax
    // after `lfo`, after a duration and before `=` only, so they name a let as before the extensions
    for w in ["lfo", "s-curve"] {
        let e = errors(&format!("let {w} = 4\n"));
        assert!(e[0].2.contains("cannot be a name") || e[0].2.contains("cannot name"), "{w}: {e:?}");
    }
    for w in ["sine", "tri", "square", "saw", "exp", "log", "step", "linear", "depth", "center", "phase"] {
        let src = format!("let {w} = 4\ndeck A: kick x\n");
        ok(&src);
    }
}

// ---- no panic, whatever is written

#[test]
fn mutated_programs_are_an_error_with_a_place_or_a_program_that_reads_back() {
    let vocab = [
        "tempo", "lfo", "sine", "tri", "square", "saw", "sin", "exp", "log", "s-curve", "step", "linear", "depth=", "center=", "phase=", "over", "bars", "bar", "beats", "beat", "->", "at", "every", "for", "master:", "deck", "A:", "B:",
        "filter", "lp", "hp", "level", "delay", "0.5", "2", "4", "1k", "300", "0", "-1", "(", ")", "+", "/", "*", "do", "vocoder", "set", "mix=", "section", "verse", "8", "{", "}", "arrange", "5db", "120ms", "when", "$i", "repeat", "def", "kick", "x",
    ];
    let bases = [
        "tempo 100 -> 130 over 8 bars exp\ndeck A: kick x ~ x ~\n",
        "section verse 4 bars {\n  tempo 120\n  deck A: kick x\n  master: filter lp lfo sine 2 bars center=3k depth=1k\n}\narrange verse\n",
        "at bar 4 beat 2: tempo 128 -> 140 over 2 beats s-curve\nmaster: delay 0.1 -> 0.9 over 4 bars step 4\n",
        "do vocoder set mix=lfo tri 1 bar depth=0.2 center=0.3 over 4 bars\nmaster: level lfo saw 4 beats center=0.5 depth=0.25 phase=0.5\n",
        "def sweep(n) {\n  master: reverb 0.1 -> 0.7 over $n bars log\n  tempo 90 -> 120 over $n bars\n}\nrepeat 2 every 4 bars {\n  sweep(2)\n}\n",
    ];
    let mut rng = Rng(0xf022_5001);
    let (mut oks, mut errs) = (0, 0);
    for _ in 0..6000 {
        let base = bases[rng.below(bases.len() as u64) as usize];
        let mut toks: Vec<String> = base.split(' ').map(str::to_string).collect();
        for _ in 0..(1 + rng.below(3)) {
            let at = rng.below(toks.len() as u64) as usize;
            let word = vocab[rng.below(vocab.len() as u64) as usize].to_string();
            match rng.below(4) {
                0 => {
                    toks.remove(at);
                }
                1 => toks.insert(at, word),
                2 => toks[at] = word,
                _ => {
                    let other = rng.below(toks.len() as u64) as usize;
                    toks.swap(at, other)
                }
            }
            if toks.is_empty() {
                toks.push("x".into());
            }
        }
        let src = toks.join(" ");
        let c = check(&src);
        match c.compiled {
            Some(comp) => {
                oks += 1;
                // what reads reads back as itself
                let again = check(&comp.text).compiled.unwrap_or_else(|| panic!("the canonical text of {src:?} does not read:\n{}", comp.text));
                assert_eq!(again.text, comp.text, "{src:?}");
                assert_eq!(again.cid, comp.cid, "{src:?}");
            }
            None => {
                errs += 1;
                assert!(c.diagnostics.iter().any(|d| d.is_error()), "{src:?}");
                // every error has a place inside the text
                for d in c.diagnostics.iter().filter(|d| d.is_error()) {
                    assert!(d.span.start <= src.len() && d.span.end <= src.len(), "{src:?}: {d:?}");
                }
            }
        }
    }
    assert!(errs > 1000 && oks > 100, "{oks} programs, {errs} errors");
}

#[test]
fn a_wired_port_gives_a_shape_its_length_and_an_lfo_its_period() {
    // F11: a `bars` port carries a number, and the line that takes it is a ramp or an lfo of that length
    let src = "def howlong() out: bars {\n  emit 4\n}\ndef sweep() in: bars {\n  master: delay 0.1 -> 0.9 over $bars bars s-curve\n  master: reverb lfo sine $bars bars center=0.5 depth=0.25\n}\nwire howlong.out -> sweep.bars\nsweep()\n";
    let c = ok(src);
    assert_eq!(ramp(&c, 0).bars, Dur::bars(4));
    assert_eq!(ramp(&c, 0).shape, Shape::SCurve);
    assert_eq!(lfo(&c, 1).period, Dur::bars(4));
    assert_eq!(ok(&c.text).cid, c.cid);
}
