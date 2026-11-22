//! Features that meet, seventh file: tempo curves (F3) and ramp shapes and lfo (F4) against the composition (F10, a stack of
//! sections), sub-bar timing (F1), arithmetic (F2), defs, repeats and ports (F11), mix moves (F8), the section keys (F5), the
//! period check (F6) and the glued tokens of a macro. The tests the integrator writes when F3 and F4 landed on the branches that
//! were built apart. (`ext_cross.rs` to `ext_cross6.rs` are the first six files; each feature's own tests are in
//! `ext_<feature>.rs`.)

mod common;

use std::collections::BTreeMap;

use common::{reformat, Rng};
use djengine::json::{JsonObject, JsonValue};
use djlive::ast::Deck;
use djlive::compile::{Compiled, Effect, Timing};
use djlive::dur::Dur;
use djlive::highlight::{highlight, Class};
use djlive::music::Key;
use djlive::rat::Rat;
use djlive::runtime::{Grid, Live, LiveHost};
use djlive::shape::{Lfo, Shape};
use djlive::tempo::SEND_EPS;
use djlive::{check, diag};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message and help) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String)> {
    let c = check(src);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, format!("{} {}", d.message, d.help.clone().unwrap_or_default()))).collect()
}

fn err_text(src: &str) -> String {
    errs(src).into_iter().map(|e| e.2).collect::<Vec<_>>().join("\n")
}

/// The canonical text is a fixed point, and 40 random reformattings keep the CID.
fn stable(src: &str, seed: u64) -> Compiled {
    let c = ok(src);
    let again = ok(&c.text);
    assert_eq!(again.text, c.text, "the canonical text is not a fixed point:\n{}", c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
    c
}

fn lfos(c: &Compiled) -> Vec<Lfo> {
    c.controls.iter().filter_map(|x| if let Effect::Lfo(l) = &x.effect { Some(l.clone()) } else { None }).collect()
}

fn ramps(c: &Compiled) -> Vec<djlive::compile::Ramp> {
    c.controls.iter().filter_map(|x| if let Effect::Ramp(r) = &x.effect { Some(r.clone()) } else { None }).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn half_bar() -> Dur {
    Dur::from_bars(Rat::new(1, 2).unwrap()).unwrap()
}

/// The column (1-based) of `pat` on the `line`th line of `src`.
fn col_of(src: &str, line: usize, pat: &str) -> usize {
    src.lines().nth(line - 1).and_then(|l| l.find(pat)).map(|c| c + 1).unwrap_or_else(|| panic!("no {pat:?} on line {line} of {src}"))
}

// ---- a fake host with a mixer you can read, a tempo you can set, and nets that land on the next bar line

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    loads: Vec<(String, i64)>,
    /// (beat when called, noun, params): every call but a read of the mixer
    calls: Vec<(f64, String, String)>,
    /// (beat when called, deck, bpm)
    tempos: Vec<(f64, String, f64)>,
    mixer: BTreeMap<String, f64>,
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

    fn load_net(&mut self, deck: &str, _env: &JsonValue) -> Result<(), String> {
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        if noun == "mixer" {
            let o = params.as_object().cloned().unwrap_or_default();
            if o.iter().next().is_none() {
                return Ok(self.state());
            }
            let deck = o.get("deck").and_then(JsonValue::as_str).unwrap_or("").to_string();
            for (k, v) in o.iter() {
                let Some(v) = v.as_f64() else { continue };
                match k {
                    "volume" | "low" | "mid" | "high" => {
                        self.mixer.insert(format!("{deck}.{k}"), v);
                    }
                    "master" => {
                        self.mixer.insert("master".into(), v);
                    }
                    _ => {}
                }
            }
        }
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }

    fn set_tempo(&mut self, deck: &str, bpm: f64) -> Result<(), String> {
        self.tempos.push((self.beat, deck.into(), bpm));
        Ok(())
    }
}

/// The deck beat a program with no sections has its bar 0 on (deck A plays from beat 8): the bar it was evaluated in, which its `at bar` lines
/// count from (the music lands on the next bar line).
const FLAT: f64 = 8.0;
/// The deck beat its tempo curve has its bar 0 on: the bar the music lands on, the first you hear (R-02, docs/BTW.md "Time words").
const LANDED: f64 = 12.0;

impl Fake {
    fn state(&self) -> JsonValue {
        let get = |k: &str, d: f64| JsonValue::Number(self.mixer.get(k).copied().unwrap_or(d));
        let strip = |d: &str| {
            JsonValue::Object(JsonObject::from_pairs([
                ("deck", JsonValue::str(d)),
                ("volume", get(&format!("{d}.volume"), 0.8)),
                ("eq", JsonValue::Object(JsonObject::from_pairs([("high", get(&format!("{d}.high"), 0.5)), ("mid", get(&format!("{d}.mid"), 0.5)), ("low", get(&format!("{d}.low"), 0.5)), ("filter", JsonValue::Number(0.5))]))),
            ]))
        };
        JsonValue::Object(JsonObject::from_pairs([
            ("set", JsonValue::Array(Vec::new())),
            ("mixer", JsonValue::Object(JsonObject::from_pairs([("strips", JsonValue::Array(vec![strip("A"), strip("B")])), ("master", get("master", 1.0))]))),
        ]))
    }

    /// Advance `beats` in quarter-beat steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    /// The deck beat the song's bar 0 is on (an arranged program's nets land there).
    fn song(&self) -> f64 {
        self.loads.iter().find(|l| l.0 == "A").map_or(0, |l| l.1) as f64 * 4.0
    }

    /// Run to a beat of the song (from its first bar line).
    fn run_to_beat(&mut self, live: &mut Live, beat: f64) {
        if self.loads.is_empty() {
            self.run(live, 0.25);
        }
        let want = self.song() + beat;
        if want > self.beat {
            self.run(live, want - self.beat);
        }
    }

    /// (beat from `off`, params) of the calls of one noun.
    fn calls_of(&self, noun: &str, off: f64) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0 - off, c.2.clone())).collect()
    }

    /// (beat from `off`, value) of one field of the calls of one noun.
    fn field(&self, noun: &str, field: &str, off: f64) -> Vec<(f64, f64)> {
        self.calls.iter().filter(|c| c.1 == noun).filter_map(|c| JsonValue::parse(&c.2).ok().and_then(|p| p.get(field).and_then(JsonValue::as_f64)).map(|v| (c.0 - off, v))).collect()
    }

    /// The tempos sent to deck A (beat from `off`, bpm), one a change (the once-a-bar resend left out).
    fn sent(&self, off: f64) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (b, d, t) in &self.tempos {
            if d == "A" && out.last().is_none_or(|l| (l.1 - t).abs() > 1e-9) {
                out.push((*b - off, *t));
            }
        }
        out
    }
}

/// A deck A that plays at beat 8, with the mixer as someone left it.
fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = FLAT;
    h.mixer.insert("A.volume".into(), 0.65);
    h.mixer.insert("master".into(), 0.75);
    (h, Live::new())
}

// ---- (a) F3 x F10: a tempo in a stacked section

const GROOVE: &str = "section groove 8 bars {\n  tempo 100 -> 130 over 4 bars\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  deck B: bass c2\n}\narrange stack(groove, wash)\n";

#[test]
fn a_tempo_in_one_stacked_section_is_the_tempo_of_the_stretch_and_the_length_is_the_longer() {
    let c = stable(GROOVE, 0xf3_0001);
    // one event: the carried copy of the longer section in the second stretch is not a second tempo
    let ev = c.tempo_map().events();
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!((ev[0].at, ev[0].from, ev[0].to, ev[0].over), (Rat::ZERO, 100.0, 130.0, Rat::int(4)));
    assert_eq!(c.segments.len(), 2, "groove+wash, then groove alone");
    // each stretch's nets are made at the tempo the curve has where the stretch begins
    assert_eq!(c.segments[0].decks[&Deck::B].tempo, Some(100.0));
    assert_eq!(c.segments[0].decks[&Deck::A].tempo, Some(100.0));
    assert_eq!(c.segments[1].decks[&Deck::A].tempo, Some(130.0));
    // the song is as long as the longer section, and the seconds are the integral over those bars
    assert_eq!(c.length_bars(), Some(8));
    let secs = c.length_seconds().unwrap();
    assert!(close(secs, c.tempo_map().total_seconds(8u32)));
    // (4 bars of ramp: 16 beats from 100 to 130 is 60 * 16 * ln(1.3) / 30, then 16 beats at 130)
    assert!((secs - (60.0 * 16.0 * 1.3f64.ln() / 30.0 + 16.0 * 60.0 / 130.0)).abs() < 1e-9, "{secs}");
    assert_eq!(c.length_report().unwrap(), format!("length: {} with the tempo curve, 8 bars", djlive::evolve::human(secs)));
    // the shorter section can be the one that says it (the stretch is in that tempo all the same)
    let c = ok("section groove 8 bars {\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  tempo 90\n  deck B: bass c2\n}\narrange stack(groove, wash)\n");
    assert_eq!(c.tempo_map().constant_bpm(), Some(90.0));
    assert_eq!(c.segments[0].decks[&Deck::A].tempo, Some(90.0));
    // and two that say the same thing are one tempo
    let c = ok("section groove 8 bars {\n  tempo 90\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  tempo 90\n  deck B: bass c2\n}\narrange stack(groove, wash)\n");
    assert_eq!(c.tempo_map().constant_bpm(), Some(90.0));
    assert_eq!(c.length_report().unwrap().split(", ").nth(1), Some("8 bars"));
}

#[test]
fn two_stacked_sections_that_set_different_tempos_are_an_error_naming_both() {
    let two = |a: &str, b: &str| format!("section groove 8 bars {{\n  {a}\n  deck A: kick x ~ x ~\n}}\nsection wash 4 bars {{\n  {b}\n  deck B: bass c2\n}}\narrange stack(groove, wash)\n");
    for (a, b) in [("tempo 100", "tempo 120"), ("tempo 100 -> 130 over 4 bars", "tempo 100"), ("tempo 100 -> 130 over 4 bars", "tempo 100 -> 130 over 4 bars exp"), ("tempo 100 -> 130 over 4 bars", "tempo 100 -> 130 over 2 bars")] {
        let e = errs(&two(a, b));
        assert_eq!(e.len(), 1, "{a} / {b}: {e:?}");
        // at the second one's line, naming both sections and the line of the first
        assert_eq!(e[0].0, 6, "{a} / {b}");
        assert!(e[0].2.contains("sections `groove` and `wash` play at the same time and both set the tempo on bar 0") && e[0].2.contains("line 2"), "{}", e[0].2);
        assert!(e[0].2.contains("then(...)"), "the way out is said: {}", e[0].2);
    }
    // the same on a bar that is not the first: `at bar 2` of each
    let e = err_text(&two("at bar 2: tempo 100", "at bar 2: tempo 120"));
    assert!(e.contains("on bar 2"), "{e}");
    // no clash: one after the other (then), two different bars, a section's tempo over the program's own (the default it
    // overrides), a stack of three where two agree
    let then = "section groove 4 bars {\n  tempo 100\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  tempo 120\n  deck B: bass c2\n}\narrange then(groove, wash)\n";
    let c = ok(then);
    assert_eq!((c.tempo_map().declared_at(0u32), c.tempo_map().declared_at(4u32)), (Some(100.0), Some(120.0)));
    ok(&two("at bar 1: tempo 100", "at bar 2: tempo 120"));
    ok(&format!("tempo 110\n{}", two("tempo 100", "tempo 100")));
    let three = "section groove 8 bars {\n  tempo 100\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  tempo 100\n  deck B: bass c2\n}\nsection top 2 bars {\n  deck A: hihat x*8\n}\narrange stack(groove, wash, top)\n";
    ok(three);
    // the same tempo at the same bar from two sections that follow each other: no clash, no change
    let c = ok("section one 4 bars {\n  tempo 100\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  tempo 100\n  deck A: kick x x x x\n}\narrange one two\n");
    assert_eq!(c.tempo_map().constant_bpm(), Some(100.0));
}

// ---- (a2) F4 x F10: an lfo that has no `over` ends where its own section ends

#[test]
fn an_lfo_with_no_over_runs_to_the_end_of_its_own_section_not_of_the_stretch_it_is_in() {
    let lfo = "master: filter lp lfo sine 2 bars center=3k depth=1k";
    let stack = |long_first: bool| {
        let (a, b) = if long_first { (8, 4) } else { (4, 8) };
        format!("section groove {a} bars {{\n  {lfo}\n  deck A: kick x ~ x ~\n}}\nsection wash {b} bars {{\n  deck B: bass c2\n}}\narrange stack(groove, wash)\n")
    };
    // the lfo's section is the longer: it goes on after the stretch it was said in (bars 0 to 4) ends
    assert_eq!(lfos(&ok(&stack(true)))[0].until, Some(8));
    // the shorter: it stops where its section does
    assert_eq!(lfos(&ok(&stack(false)))[0].until, Some(4));
    // a `do` line is read the same way
    let c = ok("section groove 8 bars {\n  do vocoder set mix=lfo sine 2 bars depth=0.5 center=0.5\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  deck B: bass c2\n}\narrange stack(groove, wash)\n");
    assert_eq!(lfos(&c)[0].until, Some(8));
    // a section that is three in a row (then) stacked with a longer one: each copy ends with its own placement
    let c = ok("section riff 4 bars {\n  deck B: melody <a4 c5>\n  master: level lfo sine 1 bar center=0.5 depth=0.3\n}\nsection verse 12 bars {\n  deck A: kick x\n  master: delay lfo tri 2 bars center=0.5 depth=0.3\n}\narrange stack(then(riff, riff, riff), verse)\n");
    let mut untils: Vec<Option<u32>> = lfos(&c).iter().map(|l| l.until).collect();
    untils.sort();
    assert_eq!(untils, [Some(4), Some(8), Some(12), Some(12)]);
    // one section on its own, and a program with no sections, are what they were
    assert_eq!(lfos(&ok("section s 6 bars {\n  deck A: kick x\n  master: level lfo sine 2 bars center=0.5 depth=0.3\n}\narrange s\n"))[0].until, Some(6));
    assert_eq!(lfos(&ok("master: level lfo sine 2 bars center=0.5 depth=0.3\n"))[0].until, None);
}

#[test]
fn the_lfo_of_the_longer_stacked_section_goes_on_past_the_end_of_the_first_stretch_on_the_clock() {
    let src = "section groove 8 bars {\n  master: level lfo sine 2 bars center=0.5 depth=0.4\n  deck A: kick x ~ x ~\n}\nsection wash 4 bars {\n  deck B: bass c2\n}\narrange stack(groove, wash)\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to_beat(&mut live, 40.0);
    let sends = h.field("mixer", "master", h.song());
    assert!(!sends.is_empty());
    // it is still moving after bar 4 (beat 16), where the first stretch ends, and stops at bar 8 (beat 32), its section's end
    assert!(sends.iter().any(|(b, _)| *b > 18.0 && *b < 32.0), "{sends:?}");
    assert!(sends.iter().all(|(b, _)| *b <= 32.25), "nothing after the section: {sends:?}");
    assert!(sends.iter().any(|(b, _)| *b > 28.0), "{sends:?}");
}

// ---- (b) F3 x F1: a tempo on a time between bar lines

#[test]
fn a_timed_tempo_line_takes_a_beat_inside_a_bar_and_never_moves_another_time_word() {
    let src = "tempo 100\ndeck A: kick x ~ x ~\nat bar 2 beat 2: tempo 130 -> 140 over 2 beats exp\n";
    let c = stable(src, 0xf3_0010);
    assert!(c.text.contains("at bar 2 beat 2: tempo 130 -> 140 over 2 beats exp\n"), "{}", c.text);
    let ev = c.tempo_map().events();
    assert_eq!(ev.len(), 2);
    assert_eq!((ev[1].at, ev[1].over, ev[1].shape, ev[1].from, ev[1].to), (Rat::new(5, 2).unwrap(), Rat::new(1, 2).unwrap(), Shape::Exp, 130.0, 140.0));
    // two spellings of one time are one program
    assert_eq!(ok("tempo 100\ndeck A: kick x ~ x ~\nat bar 2 beat 2: tempo 130 -> 140 over 1/2 bar exp\n").cid, c.cid);
    // 10 beats at 100 BPM take 6 seconds, exactly: the line is at beat 10
    assert!(close(c.tempo_map().seconds_at(Rat::new(5, 2).unwrap()), 6.0));
    // another line at the same time compiles to the same time with and without the curve, and fires on the same beat
    let plain = "tempo 100\ndeck A: kick x ~ x ~\nat bar 2 beat 2: master: level 0.5\n";
    let both = "tempo 100\ndeck A: kick x ~ x ~\nat bar 2 beat 2: tempo 130 -> 140 over 2 beats exp\nat bar 2 beat 2: master: level 0.5\n";
    let (a, b) = (ok(plain), ok(both));
    assert_eq!(a.controls.len(), 1);
    assert_eq!(b.controls.len(), 1, "a tempo line is no control");
    assert_eq!(a.controls[0].timing, Timing::AtPos(Rat::new(5, 2).unwrap()));
    assert_eq!(b.controls[0].timing, a.controls[0].timing);
    let fire = |src: &str| {
        let (mut h, mut live) = playing();
        assert!(live.eval(src).ok);
        h.run(&mut live, 16.0);
        (h.calls.iter().filter(|c| c.1 == "mixer").map(|c| c.0).collect::<Vec<_>>(), h)
    };
    let (beats_plain, _) = fire(plain);
    let (beats_both, h) = fire(both);
    assert_eq!(beats_plain, beats_both, "the curve moves no bar and no beat");
    assert_eq!(beats_both.len(), 1);
    assert!((18.0..=18.25).contains(&beats_both[0]), "bar 2 beat 2 of a program that began on beat 8: {beats_both:?}");
    // and the tempo goes where the map says, from that beat (counted from the bar the music lands on): 100 until it, then 130 climbing to 140
    // over two beats
    let sent = h.sent(LANDED);
    assert_eq!(sent.first().map(|s| s.1), Some(100.0));
    let at = |p: f64| c.tempo_map().declared_at_f(p).unwrap();
    for (b, t) in &sent {
        let rel = b / 4.0;
        assert!((t - at(rel.max(0.0))).abs() < SEND_EPS + 1e-9, "beat {b}: sent {t}, the map says {}", at(rel));
    }
    let first_move = sent.iter().find(|s| s.1 > 100.0).unwrap();
    assert!((10.0..=10.25).contains(&first_move.0), "{first_move:?}");
    assert_eq!(sent.last().map(|s| s.1), Some(140.0));
}

// ---- (c) F4 x F1: shapes over a fractional duration, and the progress bar

#[test]
fn a_shaped_ramp_over_a_fraction_of_a_bar_follows_its_shape_and_its_progress_bar_is_its_time() {
    // `over 1/2 bar` is `over 2 beats`; exp at a quarter, a half and three quarters is 1/15, 1/5 and 7/15 of the way
    let src = "at bar 1: master: level 0 -> 1 over 1/2 bar exp\n";
    let c = stable(src, 0xf4_0001);
    assert!(c.text.contains("over 2 beats exp"), "{}", c.text);
    assert_eq!(ramps(&c)[0].bars, half_bar());
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 + 1.0);
    // one beat in (the ramp began on beat 12): half the time, a fifth of the way; the editor's bar says half
    let m = live.marks();
    let mark = m.controls.iter().find(|x| x.key.contains("master: level")).expect("the ramp is lit");
    assert!((mark.ramp.unwrap() - 0.5).abs() < 1e-6, "{:?}", mark.ramp);
    h.run(&mut live, 1.5);
    let sends = h.field("mixer", "master", FLAT + 4.0);
    let want = |t: f64| (2f64.powf(4.0 * t) - 1.0) / 15.0;
    for (b, v) in &sends {
        let t = b / 2.0;
        assert!((v - want(t.clamp(0.0, 1.0))).abs() < 1e-6, "beat {b}: {v} vs {}", want(t));
    }
    assert!(sends.len() >= 4, "{sends:?}");
    assert!(sends.iter().any(|(b, v)| (*b - 1.0).abs() < 1e-9 && (v - 0.2).abs() < 1e-6), "a fifth of the way at half the time: {sends:?}");
    assert_eq!(sends.last().map(|s| s.1), Some(1.0));
    // 1/3 of a bar, an s-curve: 4/3 beats, and the sends are on the curve of their own time
    let src = "at bar 1: master: level 0 -> 1 over 1/3 bar s-curve\n";
    assert!(ok(src).text.contains("over 1/3 bars s-curve"), "{}", ok(src).text);
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 + 3.0);
    let sends = h.field("mixer", "master", FLAT + 4.0);
    assert!(sends.len() >= 3, "{sends:?}");
    for (b, v) in &sends {
        let t = (b / (4.0 / 3.0)).clamp(0.0, 1.0);
        assert!((v - (3.0 * t * t - 2.0 * t * t * t)).abs() < 1e-6, "beat {b}: {v}");
    }
    assert_eq!(sends.last().map(|s| s.1), Some(1.0));
}

#[test]
fn a_tempo_ramp_with_a_shape_over_a_fraction_of_a_bar_is_the_same_curve_a_control_ramp_has() {
    // the tempo's `exp` and a control's `exp` are one function of the ramp's own time
    let c = ok("tempo 100 -> 140 over 2 beats exp\ndeck A: kick x\nmaster: level 0 -> 1 over 2 beats exp\n");
    let ev = &c.tempo_map().events()[0];
    assert_eq!((ev.over, ev.shape), (half_bar().as_bars(), Shape::Exp));
    let r = &ramps(&c)[0];
    assert_eq!((r.bars, r.shape), (half_bar(), Shape::Exp));
    for k in 0..=8 {
        let t = f64::from(k) / 8.0;
        // the tempo a fraction t of the way along (bar t / 2), from the map; the value of the ramp at t
        let bpm = c.tempo_map().bpm_at(Rat::new(i64::from(k), 16).unwrap());
        assert!((bpm - (100.0 + 40.0 * r.shape.apply(t))).abs() < 1e-9, "t {t}: {bpm}");
        assert!((r.value(t) - r.shape.apply(t)).abs() < 1e-12);
    }
}

// ---- (d) F4 x F8: a mix item is one number

#[test]
fn a_mix_item_takes_no_ramp_and_no_lfo_and_the_message_says_where_they_go() {
    let src = "section groove 4 bars {\n  deck A: kick x ~ x ~\n  mix { A level 0 -> 1 over 2 bars exp }\n}\narrange groove\n";
    let e = errs(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!((e[0].0, e[0].1), (3, col_of(src, 3, "->")), "at the arrow");
    assert!(e[0].2.contains("a mix item sets one number, once: it takes no ramp") && e[0].2.contains("deck A: level 0 -> 1 over 4 bars exp"), "{}", e[0].2);
    let src = "section groove 4 bars {\n  deck A: kick x ~ x ~\n  mix { A level lfo sine 2 bars }\n}\narrange groove\n";
    let e = errs(src);
    assert_eq!((e[0].0, e[0].1), (3, col_of(src, 3, "lfo")), "at the lfo");
    assert!(e[0].2.contains("a mix item sets one number, once: it takes no lfo") && e[0].2.contains("deck A: level lfo sine 2 bars"), "{}", e[0].2);
    // the same for an eq item and the master, and later in a block
    assert!(err_text("mix {\n  A level 0.5\n  A eq low -3 -> 3 over 1 bar\n}\n").contains("takes no ramp"));
    assert!(err_text("mix { master level lfo tri 1 bar }\n").contains("takes no lfo"));
    // what the message points at works: the control lines, next to a mix, in one section
    let c = ok("section groove 4 bars {\n  deck A: kick x ~ x ~\n  mix { A level 0.7 }\n  deck A: level 0.2 -> 0.9 over 4 bars exp\n  master: level lfo sine 2 bars center=0.5 depth=0.3\n}\narrange groove\n");
    assert_eq!((ramps(&c).len(), lfos(&c).len()), (1, 1));
    assert_eq!(c.mix_report().len(), 1);
    // and `exp` after a duck is no shape of anything
    assert!(!check("deck A: kick x\ndeck B: bass c2\nduck B under A.kick 6db exp\n").ok());
}

// ---- (e) F4 x F2 x F11: defs, repeats, arithmetic, ports

#[test]
fn an_lfo_in_a_def_called_from_a_repeat_takes_dollar_i_arithmetic_in_its_centre_and_depth() {
    let src = "def wob(num) {\n  master: delay lfo sine 2 bars center=0.2 + $num / 10 depth=$num / 20\n}\nrepeat 3 every 4 bars {\n  wob($i + 1)\n}\n";
    let c = stable(src, 0xf4_0020);
    let l = lfos(&c);
    assert_eq!(l.len(), 3);
    for (i, l) in l.iter().enumerate() {
        let n = (i + 1) as f64;
        assert!(close(l.center, 0.2 + n / 10.0), "copy {i}: centre {}", l.center);
        assert!(close(l.depth, n / 20.0), "copy {i}: depth {}", l.depth);
        assert_eq!(l.period, Dur::bars(2));
    }
    assert_eq!(c.controls.iter().map(|x| x.timing).collect::<Vec<_>>(), [Timing::Now, Timing::At(4), Timing::At(8)]);
    // a shaped ramp in the same def, with a step count from the copy
    let src = "def climb(num) {\n  master: level 0 -> 1 over 2 bars step $num\n}\nrepeat 3 every 4 bars {\n  climb($i + 2)\n}\n";
    let c = stable(src, 0xf4_0021);
    assert_eq!(ramps(&c).iter().map(|r| r.shape).collect::<Vec<_>>(), [Shape::Step(2), Shape::Step(3), Shape::Step(4)]);
    // an lfo in a section, said by a def inside a repeat: it ends with the section, and its copy's span points at the text that is there
    let src = "def wob(num) {\n  master: delay lfo sine 2 bars center=0.5 depth=$num / 20\n}\nsection s 8 bars {\n  deck A: kick x\n  repeat 2 every 4 bars {\n    wob($i + 1)\n  }\n}\narrange s\n";
    let c = stable(src, 0xf4_0022);
    assert_eq!(c.controls.len(), 2);
    for ctl in &c.controls {
        assert!(ctl.span.end <= src.len() && ctl.via.iter().any(|v| v.text(src).starts_with("wob(")), "{:?}", ctl.via);
        let Effect::Lfo(l) = &ctl.effect else { panic!("{:?}", ctl.effect) };
        assert_eq!(l.until, Some(8));
    }
}

#[test]
fn a_wired_bars_port_gives_a_shaped_ramp_its_length_and_an_lfo_its_period_and_a_tempo_port_a_curve_its_ends() {
    let src = "def howlong() out: bars {\n  emit 4\n}\ndef riser() in: bars {\n  master: filter lp 300 -> 2800 over $bars bars exp\n  master: reverb lfo sine $bars bars center=0.5 depth=0.3\n}\nwire howlong.out -> riser.in\nriser()\n";
    let c = stable(src, 0xf4_0030);
    assert_eq!((ramps(&c)[0].bars, ramps(&c)[0].shape), (Dur::bars(4), Shape::Exp));
    assert_eq!(lfos(&c)[0].period, Dur::bars(4));
    let src = "def howfast() out: tempo {\n  emit 128\n}\ndef push() in: tempo {\n  tempo $tempo -> $tempo + 12 over 4 bars exp\n}\nwire howfast.out -> push.in\npush()\ndeck A: kick x ~ x ~\n";
    let c = stable(src, 0xf3_0031);
    let ev = c.tempo_map().events();
    assert_eq!((ev[0].at, ev[0].from, ev[0].to, ev[0].over, ev[0].shape), (Rat::ZERO, 128.0, 140.0, Rat::int(4), Shape::Exp));
    // a repeat that moves the tempo by its copy, each from where its copy starts
    let src = "repeat 3 every 8 bars {\n  tempo 100 + $i * 10 -> 110 + $i * 10 over 2 bars exp\n}\ndeck A: kick x ~ x ~\n";
    let c = stable(src, 0xf3_0032);
    let ev: Vec<(Rat, f64, f64)> = c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to)).collect();
    assert_eq!(ev, [(Rat::ZERO, 100.0, 110.0), (Rat::int(8), 110.0, 120.0), (Rat::int(16), 120.0, 130.0)]);
    // and an arithmetic tempo out of range is an error at the number, in whichever copy it is in
    let e = err_text("repeat 3 every 8 bars {\n  tempo 250 + $i * 30\n}\ndeck A: kick x\n");
    assert!(e.contains("a tempo is 40 to 300 BPM"), "{e}");
}

// ---- (f) glued tokens after a macro puts a value in place

#[test]
fn tokens_that_touch_still_touch_after_a_macro_puts_a_value_in_place() {
    // `1k` and `3k` are a number with its suffix, `1/2` arithmetic; a count after `step`, a period after `lfo sine`
    let src = "def w(dep, per, cnt, cen) {\n  master: filter lp lfo sine $per bars center=$cen depth=$dep\n  master: level 0.1 -> 0.9 over $per bars step $cnt\n}\nw(1k, 4, 4, 3k)\nw(1k, 1/2, 2 + 2, 3k)\n";
    let c = stable(src, 0xf4_0040);
    let l = lfos(&c);
    assert_eq!((l[0].period, l[1].period), (Dur::bars(4), half_bar()));
    assert!(close(l[0].center, l[1].center) && close(l[0].depth, l[1].depth));
    assert_eq!(ramps(&c).iter().map(|r| r.shape).collect::<Vec<_>>(), [Shape::Step(4), Shape::Step(4)]);
    assert_eq!(ramps(&c).iter().map(|r| r.bars).collect::<Vec<_>>(), [Dur::bars(4), half_bar()]);
    // a tempo's ends and its length from a def's arguments
    let src = "def push(lo, hi, nth) {\n  at bar $nth: tempo $lo -> $hi over 1/2 bar s-curve\n}\npush(100, 100 + 20, 4)\ndeck A: kick x\n";
    let c = stable(src, 0xf3_0041);
    let ev = &c.tempo_map().events()[0];
    assert_eq!((ev.at, ev.from, ev.to, ev.over, ev.shape), (Rat::int(4), 100.0, 120.0, Rat::new(1, 2).unwrap(), Shape::SCurve));
    // a `do` line whose lfo params come from arguments
    let src = "def sweep(mid1, wide) {\n  do vocoder set mix=lfo tri 1 bar depth=$wide center=$mid1\n}\nsweep(0.5, 0.25)\n";
    let c = stable(src, 0xf4_0042);
    assert_eq!((lfos(&c)[0].center, lfos(&c)[0].depth), (0.5, 0.25));
}

#[test]
fn the_highlight_tables_of_both_sides_are_there_a_mix_line_beside_an_lfo_and_a_shape() {
    let src = "duck B under A.kick 6db release=120ms\nmix { A level 0.7 }\nmaster: filter lp lfo sine 2 bars center=3k depth=1k\nmaster: reverb 0.1 -> 0.9 over 4 bars s-curve\ndeck A: level 0 -> 1 over 4 bars step 4\ntempo 100 -> 120 over 2 bars exp\ndo vocoder set mix=0 -> 1 over 4 bars log\n";
    let classes: Vec<(String, Class)> = highlight(src).into_iter().map(|(sp, c)| (sp.text(src).to_string(), c)).collect();
    let class_of = |w: &str| classes.iter().find(|(t, _)| t == w).map(|x| x.1);
    for w in ["duck", "mix", "under", "lfo", "sine", "exp", "log", "step", "tempo"] {
        assert_eq!(class_of(w), Some(Class::Keyword), "{w}: {classes:?}");
    }
    for w in ["release", "center", "depth"] {
        assert_eq!(class_of(w), Some(Class::Param), "{w}");
    }
}

// ---- (g) the length, the period and the stack

#[test]
fn the_length_says_the_longer_of_a_stack_and_the_period_of_the_nets_does_not_move_with_the_tempo() {
    let body = "section groove 8 bars {\n  deck A: kick x ~ x ~ | hihat/12 x*8\n}\nsection wash 4 bars {\n  deck B: bass c2\n}\n";
    let build = |tempo: &str, arrange: &str| format!("{tempo}{body}arrange {arrange}\n");
    for (arrange, bars) in [("stack(groove, wash)", 8u32), ("stack(wash, groove)", 8), ("groove wash", 12), ("stack(then(wash, wash), groove)", 8), ("then(stack(groove, wash), groove)", 16)] {
        let constant = ok(&build("tempo 120\n", arrange));
        let curve = ok(&build("tempo 100 -> 140 over 6 bars\n", arrange));
        for c in [&constant, &curve] {
            assert_eq!(c.length_bars(), Some(bars), "{arrange}");
            assert_eq!(c.song_bars(), Some(bars), "{arrange}");
            assert!(close(c.length_seconds().unwrap(), c.tempo_map().total_seconds(bars)), "{arrange}");
        }
        assert!(constant.length_report().unwrap().ends_with(&format!("at 120, {bars} bars")), "{:?}", constant.length_report());
        assert!(curve.length_report().unwrap().ends_with(&format!("with the tempo curve, {bars} bars")), "{:?}", curve.length_report());
        // 8 bars at 120 BPM take 16 seconds, exactly
        assert!(close(constant.length_seconds().unwrap(), f64::from(bars) * 2.0), "{arrange}");
        // the period of each section's nets is what it was: the tempo is in the envelope, not in how the parts come round
        let (pc, pv) = (djlive::meter::program_period(&constant), djlive::meter::program_period(&curve));
        assert_eq!(pc, pv, "{arrange}");
        assert!(pv.iter().any(|(_, p)| p.polymeter.iter().any(|t| t == "hihat/12")), "{pv:?}");
    }
    // `end at bar N` and a loop: the report says so
    let c = ok("tempo 100 -> 140 over 6 bars\ndeck A: kick x\nend at bar 6\n");
    assert!(c.length_report().unwrap().ends_with("with the tempo curve, 6 bars"), "{:?}", c.length_report());
    let c = ok(&format!("tempo 100 -> 140 over 6 bars\n{body}arrange stack(groove, wash)\nloop\n"));
    assert!(c.length_report().unwrap().ends_with("8 bars, then it starts again"), "{:?}", c.length_report());
}

// ---- (h) F5 x F3: a net's identity

#[test]
fn a_key_change_and_a_tempo_curve_keep_each_sections_nets_apart_and_a_constant_tempo_keeps_its_nets() {
    let song = |tempo: &str| format!("{tempo}\nkey am\nsection one 4 bars {{\n  deck B: melody <a4 c5 e5 c5>\n  deck A: kick x ~ x ~\n}}\nsection two 4 bars {{\n  key cm\n  deck B: melody <a4 c5 e5 c5>\n  deck A: kick x ~ x ~\n}}\narrange one two\n");
    // constant: the pitched deck's nets differ by key alone, the drum deck's are one net, every net says the one tempo
    let c = stable(&song("tempo 100"), 0xf5_0001);
    let (b0, b1) = (&c.segments[0].decks[&Deck::B], &c.segments[1].decks[&Deck::B]);
    assert_ne!(b0.net_cid(), b1.net_cid());
    assert_eq!(c.segments[0].decks[&Deck::A].net_cid(), c.segments[1].decks[&Deck::A].net_cid());
    assert!([b0.tempo, b1.tempo, c.segments[0].decks[&Deck::A].tempo, c.segments[1].decks[&Deck::A].tempo].iter().all(|t| *t == Some(100.0)));
    // a tempo that is one number said by a ramp from and to the same is no curve: nothing is driven
    assert!(!ok(&song("tempo 100 -> 100 over 4 bars")).tempo_map().drives());
    // with a curve: each section's nets are made at the tempo it begins at, so the drum deck's two differ now (by tempo alone)
    let c = stable(&song("tempo 100 -> 128 over 8 bars"), 0xf5_0002);
    let (a0, a1) = (&c.segments[0].decks[&Deck::A], &c.segments[1].decks[&Deck::A]);
    assert_eq!((a0.tempo, a1.tempo), (Some(100.0), Some(114.0)));
    assert_ne!(a0.net_cid(), a1.net_cid());
    // the pitched deck's two differ by tempo and key, and its second says its own key
    let (b0, b1) = (&c.segments[0].decks[&Deck::B], &c.segments[1].decks[&Deck::B]);
    assert_ne!(b0.net_cid(), b1.net_cid());
    assert_eq!((b0.tempo, b1.tempo), (Some(100.0), Some(114.0)));
    assert_eq!(b1.key, Key::parse("cm"), "C minor");
    assert_eq!(c.section_key("one"), Key::parse("am"));
    assert_eq!(c.section_key("two"), Key::parse("cm"));
    // every net of the song is its own: four nets, four CIDs
    let cids = c.net_cids();
    assert_eq!(cids.len(), 4);
    assert_eq!(cids.values().collect::<std::collections::BTreeSet<_>>().len(), 4, "{cids:?}");
    // the key alone moves nothing about tempo and the tempo alone moves nothing about key: the second section's pitched net of
    // a constant 114 is the curve's second section's net, to the bit
    let by_hand = ok(&song("tempo 114"));
    assert_eq!(by_hand.segments[1].decks[&Deck::B].net_cid(), b1.net_cid());
    assert_eq!(by_hand.segments[1].decks[&Deck::A].net_cid(), a1.net_cid());
}

// ---- (i) F3 x F8 on the runtime's clock, one pass at a time

const BOTH: &str = "tempo 100 -> 140 over 8 bars\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  mix { A level 0.5 }\n  duck B under A.kick 6db\n}\nsection two 4 bars {\n  deck A: kick x x x x\n  at bar 1 beat 2 for 2 beats: mix { master level 0.2 }\n}\narrange one two\n";

#[test]
fn a_tempo_curve_and_a_mix_run_on_one_clock_the_mix_on_its_beats_and_the_tempo_on_the_map() {
    let c = ok(BOTH);
    assert!(c.tempo_map().drives());
    // the mix and the duck are controls (they fire at bars and beats); the curve is not (it is the map)
    assert_eq!(c.controls.iter().filter(|x| matches!(x.effect, Effect::Mix(_) | Effect::Duck(_))).count(), 3);
    assert!(c.controls.iter().all(|x| !x.key.contains("tempo")));
    let (mut h, mut live) = playing();
    let ev = live.check(BOTH);
    // both answers are in the evaluation: what the mixes come to, and how long the song is through the curve
    assert!(ev.ok && !ev.mix.is_empty() && ev.length.as_deref().is_some_and(|l| l.contains("with the tempo curve, 8 bars")), "{:?} {:?}", ev.mix, ev.length);
    assert!(live.eval(BOTH).ok);
    h.run_to_beat(&mut live, 36.0);
    let song = h.song();
    // the mix: set on the section's first bar, put back where it ends (the duck's pump off with it), the timed one between bar lines
    let mixer = h.calls_of("mixer", song);
    let vol: Vec<f64> = mixer.iter().filter(|m| m.1.contains(r#""volume""#)).map(|m| m.0).collect();
    assert_eq!(vol, [0.0, 16.0], "A's fader set at bar 0 and put back at bar 4: {mixer:?}");
    let master: Vec<f64> = mixer.iter().filter(|m| m.1.contains(r#""master""#)).map(|m| m.0).collect();
    assert_eq!(master, [22.0, 24.0], "bar 1 beat 2 of section two, for two beats: {mixer:?}");
    assert_eq!(h.calls_of("trick", song).iter().map(|t| t.0).collect::<Vec<_>>(), [0.0, 16.0]);
    // the tempo: what the map says at the song's position each time it moved
    let map = c.tempo_map();
    let sent = h.sent(song);
    assert!(sent.len() > 20, "a curve is a sequence of values: {}", sent.len());
    for (b, t) in &sent {
        let want = map.declared_at_f((b / 4.0).max(0.0)).unwrap();
        assert!((t - want).abs() < SEND_EPS + 1e-9, "song beat {b}: sent {t}, the map says {want}");
    }
    // the curve is not disturbed by a put back or a net that lands: half way through it is 120, and it goes on to 140
    assert!(sent.iter().any(|(b, t)| (*b - 16.0).abs() <= 0.5 && (t - 120.0).abs() < 0.5), "{sent:?}");
    assert!(sent.last().is_some_and(|s| s.1 > 139.0), "{:?}", sent.last());
}

#[test]
fn a_song_that_loops_goes_back_to_its_first_tempo_and_puts_the_mix_back_each_lap() {
    let src = "tempo 100 -> 140 over 8 bars\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  mix { master level 0.4 }\n}\nsection two 4 bars {\n  deck A: kick x x x x\n}\narrange one two\nloop\n";
    let c = ok(src);
    assert!(c.looped);
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    // two laps and a bit: 8 bars a lap
    h.run_to_beat(&mut live, 8.0 * 4.0 * 2.0 + 8.0);
    let song = h.song();
    let master = h.calls_of("mixer", song).iter().filter(|m| m.1.contains(r#""master""#)).map(|m| m.0).collect::<Vec<_>>();
    // set at bar 0, put back at bar 4, again at bar 8 (the second lap's first), and put back at bar 12
    assert_eq!(&master[..4], &[0.0, 16.0, 32.0, 48.0], "{master:?}");
    let sent = h.sent(song);
    // the tempo comes back to where the curve began at each lap's first bar
    let lap2: Vec<&(f64, f64)> = sent.iter().filter(|(b, _)| *b >= 31.5 && *b <= 34.0).collect();
    assert!(lap2.iter().any(|(_, t)| (t - 100.0).abs() < 0.5), "{lap2:?}");
    assert!(sent.iter().any(|(b, t)| *b > 20.0 && *b < 32.0 && *t > 135.0), "{sent:?}");
}

#[test]
fn a_duck_is_chosen_at_the_tempo_a_section_sets_not_only_at_the_top_level_one() {
    // the program says its tempo in its first section only: the pump's release note and the report are worked out at that tempo
    let duck_bpm = |c: &Compiled| c.controls.iter().find_map(|x| if let Effect::Duck(d) = &x.effect { Some(d.bpm) } else { None }).unwrap();
    let in_section = ok("section one 4 bars {\n  tempo 140\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  duck B under A.kick 6db\n}\narrange one\n");
    assert_eq!(duck_bpm(&in_section), Some(140.0));
    // as at the top: the same program, and the same pump
    let at_top = ok("tempo 140\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  duck B under A.kick 6db\n}\narrange one\n");
    assert_eq!(duck_bpm(&at_top), Some(140.0));
    assert_eq!(in_section.mix_report(), at_top.mix_report());
    // and a program with a curve from its first bar is at the tempo it starts at
    let curve = ok("tempo 100 -> 140 over 4 bars\ndeck A: kick x ~ x ~\ndeck B: bass c2\nduck B under A.kick 6db\n");
    assert_eq!(duck_bpm(&curve), Some(100.0));
    // a program that says no tempo has none to choose by: the deck's own, when it fires
    assert_eq!(duck_bpm(&ok("deck A: kick x ~ x ~\ndeck B: bass c2\nduck B under A.kick 6db\n")), None);
}
