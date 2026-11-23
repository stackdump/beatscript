//! F3 tempo curves in the offline render (docs/beatscript/ext-tempo.md): `djhost::beatscript_render::arrangement` writes a program whose
//! tempo moves as a staircase of tempo-fader moves, every event carries the tempo it falls in, and the engine follows it:
//! the render is as long as the curve's integral says and the kicks of a beat-by-beat pattern fall on the seconds the
//! tempo map gives their beats. A program whose tempo is one number is rendered exactly as it was.

use djengine::session::arrangement::{ArrangeOp, Arrangement};
use djhost::beatscript_render::arrangement;
use djhost::beatscript_tempo;
use djhost::deck::offline::{render_arrangement, ArrangeRenderOptions};
use djlive::compile::Compiled;
use djlive::rat::Rat;

const SR: f64 = 8000.0;

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

/// The seconds each onset (a hit after a quiet of at least 80 ms) falls at.
fn onsets(left: &[f32]) -> Vec<f64> {
    let peak = left.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let thr = peak * 0.2;
    let (mut out, mut quiet) = (Vec::new(), 0usize);
    for (i, x) in left.iter().enumerate() {
        if x.abs() > thr {
            if quiet as f64 > 0.08 * SR || out.is_empty() {
                out.push(i as f64 / SR);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

struct Rendered {
    seconds: f64,
    onsets: Vec<f64>,
    refused: Vec<(f64, String)>,
    beats: f64,
}

fn render(c: &Compiled) -> Rendered {
    let (arr, beats) = arr_of(c);
    let r = render_arrangement(&arr, ArrangeRenderOptions { sample_rate: SR, beats: Some(beats), ..Default::default() });
    let seconds = r.audio.left.len() as f64 / SR;
    // (a hit on the very last frame is the next bar's, not this render's)
    let onsets = onsets(&r.audio.left).into_iter().filter(|t| *t < seconds - 0.02).collect();
    Rendered { seconds, onsets, refused: r.refused, beats }
}

/// Where the map says every beat of the program is, in seconds.
fn expected_beats(c: &Compiled, beats: f64) -> Vec<f64> {
    (0..beats as i64).map(|b| c.tempo_map().seconds_at(Rat::new(b, 4).unwrap())).collect()
}

fn near(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() < tol, "{what}: beat {i}: heard at {g:.4} s, the tempo map says {w:.4} s");
    }
}

const TWO: &str = "section verse 4 bars {\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  deck A: kick x x x x\n}\narrange verse chorus\n";

#[test]
fn a_tempo_that_is_one_number_is_rendered_as_it_was() {
    let c = compile(&format!("tempo 120\n{TWO}"));
    let (a, beats) = arr_of(&c);
    assert_eq!(beats, 32.0);
    assert_eq!(a.bpm, 120.0);
    // no fader moves, and every event at the one tempo
    assert!(a.events.iter().all(|e| !matches!(&e.op, ArrangeOp::Live { method, .. } if method == "pitch_range")));
    assert!(a.events.iter().all(|e| e.op.op() != "param"));
    assert!(a.events.iter().all(|e| e.bpm == 120.0));
    assert!(beatscript_tempo::notes(&c).is_empty());
    let r = render(&c);
    assert!((r.seconds - 16.0).abs() < 1.0 / SR, "{}", r.seconds);
    near(&r.onsets, &expected_beats(&c, r.beats), 0.012, "constant 120");
    assert!(r.refused.is_empty(), "{:?}", r.refused);
}

#[test]
fn a_curve_is_a_staircase_of_fader_moves_and_every_event_says_its_tempo() {
    let c = compile(&format!("tempo 100 -> 140 over 4 bars\n{TWO}"));
    let (a, beats) = arr_of(&c);
    let map = c.tempo_map();
    // the arrangement's own clock is the curve's integral, exactly
    assert!((a.seconds_at(beats) - map.total_seconds(8u32)).abs() < 1e-9 * map.total_seconds(8u32), "{} vs {}", a.seconds_at(beats), map.total_seconds(8u32));
    assert!((a.bpm - a.events.iter().find(|e| e.at == 0.0).unwrap().bpm).abs() < 1e-12);
    // the fader's travel is set before it is moved, and the moves are in order of beat
    let first = a.events.iter().position(|e| matches!(&e.op, ArrangeOp::Live { method, .. } if method == "pitch_range")).unwrap();
    let first_move = a.events.iter().position(|e| e.op.op() == "param").unwrap();
    assert!(first < first_move);
    assert!(a.events.windows(2).all(|w| w[0].at <= w[1].at));
    // a ramp over 16 beats at a quarter beat a step: 64 steps, then the held tempo of the chorus (and the load of its nets)
    let moves = a.events.iter().filter(|e| e.op.op() == "param" && e.at < 16.0).count();
    assert!((60..=70).contains(&moves), "{moves}");
    // every step is the constant tempo that takes exactly the seconds the curve takes over it
    for e in a.events.iter().filter(|e| e.op.op() == "param") {
        assert!(e.bpm > 99.0 && e.bpm <= 140.0 + 1e-9, "{} {}", e.at, e.bpm);
    }
    // the chorus's nets are made at the tempo the curve ends on, and its moves are relative to that
    let chorus = c.segments[1].decks[&djlive::ast::Deck::A].tempo;
    assert_eq!(chorus, Some(140.0));
}

#[test]
fn the_engine_follows_a_ramp_across_two_sections_to_the_millisecond() {
    for (name, src) in [
        ("up, linear", format!("tempo 100 -> 140 over 4 bars\n{TWO}")),
        ("up, s-curve over the whole song", format!("tempo 90 -> 150 over 8 bars s-curve\n{TWO}")),
        ("down, exp", format!("tempo 150 -> 100 over 6 bars exp\n{TWO}")),
        ("up, log, a step in the middle", format!("tempo 100 -> 130 over 3 bars log\nat bar 5: tempo 110\n{TWO}")),
        ("a tempo for each section", "section verse 4 bars {\n  tempo 100\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  tempo 130 -> 150 over 2 bars\n  deck A: kick x x x x\n}\narrange verse chorus\n".to_string()),
        ("step 4", format!("tempo 100 -> 130 over 4 bars step 4\n{TWO}")),
    ] {
        let c = compile(&src);
        let r = render(&c);
        assert!(r.refused.is_empty(), "{name}: {:?}", r.refused);
        let total = c.tempo_map().total_seconds(8u32);
        assert!((r.seconds - total).abs() < 2.0 / SR, "{name}: the render is {} s, the curve says {total}", r.seconds);
        near(&r.onsets, &expected_beats(&c, r.beats), 0.012, name);
        assert!(beatscript_tempo::notes(&c).is_empty(), "{name}");
    }
}

#[test]
fn a_curve_in_one_net_with_no_sections_is_followed_too() {
    // no sections: a 16-bar net at the tempo the curve begins at, the fader does the rest
    let c = compile("tempo 100 -> 140 over 8 bars\ndeck A: kick x x x x\n");
    let r = render(&c);
    assert_eq!(r.beats, 64.0);
    let total = c.tempo_map().total_seconds(16u32);
    assert!((r.seconds - total).abs() < 2.0 / SR, "{} vs {total}", r.seconds);
    near(&r.onsets, &expected_beats(&c, r.beats), 0.012, "one net");
    // and the curve makes the render longer or shorter than the constant tempo it starts at by the integral
    let flat = compile("tempo 100\ndeck A: kick x x x x\n");
    let longer = render(&flat).seconds;
    // 16 bars at 100 BPM: 64 beats of 0.6 s
    assert!((longer - 38.4).abs() < 2.0 / SR, "{longer}");
    assert!(r.seconds < longer, "a curve that speeds up is shorter: {} vs {longer}", r.seconds);
    assert!(((longer - r.seconds) - (38.4 - total)).abs() < 4.0 / SR, "shorter by the integral: {} vs {}", longer - r.seconds, 38.4 - total);
    // a curve that slows down is longer
    let slow = compile("tempo 100 -> 70 over 8 bars\ndeck A: kick x x x x\n");
    assert!(render(&slow).seconds > longer);
}

#[test]
fn a_deck_that_comes_in_with_a_section_follows_the_curve_from_its_own_net() {
    let src = "tempo 100 -> 140 over 6 bars\nsection verse 4 bars {\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  deck A: kick x x x x\n  deck B: snare x x x x\n}\narrange verse chorus\n";
    let c = compile(src);
    let (a, _) = arr_of(&c);
    // deck B's moves are in the arrangement, relative to its own net
    assert!(a.events.iter().any(|e| e.op.deck().as_deref() == Some("B") && e.op.op() == "param"));
    let r = render(&c);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    let total = c.tempo_map().total_seconds(8u32);
    assert!((r.seconds - total).abs() < 2.0 / SR);
}

#[test]
fn a_curve_beyond_the_faders_travel_is_held_at_the_edge_and_says_so() {
    // one net made at 80 cannot be taken to 200 by a fader that reaches 120
    let c = compile("tempo 80 -> 200 over 8 bars\ndeck A: kick x x x x\n");
    let notes = beatscript_tempo::notes(&c);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("deck A") && notes[0].contains("tempo fader reaches 50%") && notes[0].contains("sections"), "{notes:?}");
    // it renders, held at the edge of the travel
    let r = render(&c);
    assert!(r.refused.is_empty());
    // the same curve in sections, each net made at the tempo it begins at, is within the travel
    let src = "tempo 80 -> 200 over 8 bars\nsection a1 2 bars {\n  deck A: kick x x x x\n}\nsection a2 2 bars {\n  deck A: kick x x x x\n}\nsection a3 2 bars {\n  deck A: kick x x x x\n}\nsection a4 2 bars {\n  deck A: kick x x x x\n}\narrange a1 a2 a3 a4\n"
        .replace("a1", "verse").replace("a2", "build").replace("a3", "lift").replace("a4", "peak");
    let c = compile(&src);
    assert!(beatscript_tempo::notes(&c).is_empty(), "{:?}", beatscript_tempo::notes(&c));
    let r = render(&c);
    let total = c.tempo_map().total_seconds(8u32);
    assert!((r.seconds - total).abs() < 2.0 / SR);
    near(&r.onsets, &expected_beats(&c, r.beats), 0.012, "in sections");
}

// ---- the same words in a `do` line, against the real registry

#[test]
fn do_lines_with_shapes_and_lfos_check_against_the_real_registry() {
    let be = djhost::beatscript_env::BeatscriptEnv::new(None);
    let env = be.env();
    for src in [
        "do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3\n",
        "do vocoder set mix=0 -> 1 over 8 bars exp\n",
        "do vocoder set mix=lfo sine 2 bars depth=0.4 center=0.5 over 8 bars\n",
        "at bar 2: do inserts set target=Master param=filter.sweep value=0.2 -> 0.8 over 4 bars s-curve\n",
    ] {
        let c = djlive::compile::check_with(src, &env);
        assert!(c.ok(), "{src}{}", djlive::diag::render_all(&c.diagnostics, src));
        let comp = c.compiled.unwrap();
        assert_eq!(djlive::compile::check_with(&comp.text, &env).compiled.unwrap().cid, comp.cid, "{src}");
    }
    // an lfo moves a number, not a name
    let src = "do mixer deck=lfo sine 1 bar\n";
    let c = djlive::compile::check_with(src, &env);
    assert!(!c.ok());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("`deck` is not a number, so an lfo cannot move it")), "{:?}", c.diagnostics);
}

// ---- F3 x F8: a tempo curve and a mix or a duck in one render

const MIXED: &str = "tempo 100 -> 140 over 6 bars\nsection verse 4 bars {\n  deck A: kick x x x x\n  deck B: bass c2 len=16\n}\nsection chorus 4 bars {\n  deck A: kick x x x x\n  deck B: bass c2 len=16\n  duck B under A.kick 12db\n  mix { A level 0.7, master level 0.9 }\n  at bar 1 beat 2 for 2 beats: mix { B level 0.5 }\n}\narrange verse chorus\n";

/// The mix and duck moves of a render: (beat, what the op says).
fn mix_moves(a: &Arrangement) -> Vec<(f64, String)> {
    a.events
        .iter()
        .filter(|e| e.via == "btw")
        .filter(|e| e.op.op() == "trick" || e.op.op() == "master" || (e.op.op() == "param" && !e.op.json().canonical_js().contains(r#""name":"pitch""#)))
        .map(|e| (e.at, e.op.json().canonical_js()))
        .collect()
}

/// A song with a tempo curve and a duck and a mix renders: the moves are on the beats the program says (bars and beats, whatever
/// the tempo), every one carries the tempo of the step it falls in so the renderer's clock puts it on the second the curve gives
/// that beat, and the engine takes every one.
#[test]
fn a_curve_and_a_duck_and_a_mix_land_their_moves_on_the_beats_and_the_curves_seconds() {
    let c = compile(MIXED);
    let (a, beats) = arr_of(&c);
    assert_eq!(beats, 32.0);
    let m = mix_moves(&a);
    let at = |t: f64, frag: &str| m.iter().any(|(b, j)| (*b - t).abs() < 1e-9 && j.contains(frag));
    // the chorus begins on bar 4 (beat 16): the pump, A's fader and the master; the timed mix is on bar 5 beat 2 (beat 22), and
    // put back two beats later; what the chorus did is put back where it ends (beat 32, the render's end: left out)
    assert!(at(16.0, r#""action":"on""#) && at(16.0, r#""kind":"pump""#), "{m:?}");
    assert!(at(16.0, r#""name":"volume""#) && at(16.0, r#""value":0.7"#), "{m:?}");
    assert!(at(16.0, r#""op":"master""#) && at(16.0, "0.9"), "{m:?}");
    assert!(at(22.0, r#""deck":"B""#) && at(22.0, r#""value":0.5"#), "{m:?}");
    assert!(at(24.0, r#""deck":"B""#) && at(24.0, r#""value":0.8"#), "{m:?}");
    assert!(m.iter().all(|(b, _)| *b < 32.0), "{m:?}");
    // each of them says the tempo it is in, and the arrangement's own clock agrees with the curve's integral at its beat (the
    // steps of a ramp are a quarter of a beat: the clock is the curve's to within a millisecond)
    let map = c.tempo_map();
    for (beat, what) in &m {
        let want = map.seconds_at(Rat::new((*beat * 4.0).round() as i64, 16).unwrap());
        let got = a.seconds_at(*beat);
        assert!((got - want).abs() < 0.002, "beat {beat} ({what}): the clock says {got} s, the curve {want} s");
    }
    for e in a.events.iter().filter(|e| e.via == "btw") {
        assert!(e.bpm > 0.0, "{e:?}");
    }
    // the moves are in order, and the render takes every one of them
    assert!(a.events.windows(2).all(|w| w[0].at <= w[1].at));
    let r = render(&c);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    let total = map.total_seconds(8u32);
    assert!((r.seconds - total).abs() < 2.0 / SR, "{} vs {total}", r.seconds);
}

/// The sound of it: the pump of the duck dips deck B after the kicks of the chorus, at the seconds the curve gives their beats,
/// and not after those of the verse (it is on from beat 16, not from the second the same beat would be at a constant tempo).
#[test]
fn the_pump_of_a_duck_is_on_in_the_chorus_and_not_the_verse_under_a_curve() {
    use djhost::deck::offline::{render_arrangement, ArrangeRenderOptions};
    let render_with = |src: &str| {
        let c = compile(src);
        let (arr, beats) = arr_of(&c);
        let r = render_arrangement(&arr, ArrangeRenderOptions { stems: true, beats: Some(beats), sample_rate: 22_050.0, ..ArrangeRenderOptions::default() });
        (c, r)
    };
    let (c, ducked) = render_with(MIXED);
    let (_, plain) = render_with(&MIXED.replace("  duck B under A.kick 12db\n", "").replace("  mix { A level 0.7, master level 0.9 }\n", "").replace("  at bar 1 beat 2 for 2 beats: mix { B level 0.5 }\n", ""));
    assert!(ducked.refused.is_empty(), "the engine took every move: {:?}", ducked.refused);
    let stem = |r: &djhost::deck::offline::ArrangeRender| r.stems.iter().find(|(n, _)| n == "B").map(|(_, a)| a.clone()).expect("B's stem");
    let (p, d) = (stem(&plain), stem(&ducked));
    let rms = |a: &djengine::session::wav::RenderedAudio, from: f64, to: f64| {
        let (i, j) = ((from * a.sample_rate) as usize, (to * a.sample_rate) as usize);
        (a.left[i..j].iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / (j - i) as f64).sqrt()
    };
    let map = c.tempo_map();
    let kick = |beat: i64| map.seconds_at(Rat::new(beat, 4).unwrap());
    // the verse's kick on beat 9 (bar 2): nothing ducked, the two renders are one sound there
    let (vp, vd) = (rms(&p, kick(9) + 0.005, kick(9) + 0.030), rms(&d, kick(9) + 0.005, kick(9) + 0.030));
    assert!(vp > 0.001, "deck B plays: {vp}");
    assert!((vd / vp - 1.0).abs() < 0.05, "no pump in the verse: {vd} of {vp}");
    // the chorus's kick on beat 18 (bar 4 beat 2): B is down just after it, and back before the next
    let (cp, cd) = (rms(&p, kick(18) + 0.005, kick(18) + 0.030), rms(&d, kick(18) + 0.005, kick(18) + 0.030));
    assert!(cd / cp < 0.7, "B is down just after the kick: {cd} of {cp}");
}
