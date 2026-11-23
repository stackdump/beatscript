//! F1 sub-bar timing (docs/beatscript/ext-timing.md): `over 1/2 bar`, `for 2 beats`, `every 2 beats`, `at bar 16 beat 3`,
//! `before drop 2 beats`. The canonical forms (a duration is `N bars` when whole, else `N beats`, else `P/Q bars`; a
//! position `bar W` or `bar W beat B`), the way they resolve to times, and the runtime on a fake clock: a line with a
//! time between bar lines fires when the program's position crosses it, once, whatever the pass rate; a ramp over half
//! a bar ends exactly; a hold of two beats is put back two beats on.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, Compiled, Effect, Timing};
use djlive::dur::{Dur, Pos};
use djlive::rat::Rat;
use djlive::runtime::{Grid, Live, LiveHost};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<(usize, usize, String)> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| {
            let (l, col, _) = d.position(src);
            (l, col, d.message.clone())
        })
        .collect()
}

fn r(n: i64, d: i64) -> Rat {
    Rat::new(n, d).unwrap()
}

// ---- canonical forms

#[test]
fn durations_and_positions_print_the_way_the_contract_says() {
    for (src, want) in [
        // whole bars print exactly as they always did
        ("master: filter lp 300 -> 2k over 1 bar\n", "master: filter lp 300 -> 2000 over 1 bars\n"),
        ("master: filter lp 300 -> 2k over 8 bars\n", "master: filter lp 300 -> 2000 over 8 bars\n"),
        ("every 4 bars from 8: master: level 0.5\n", "every 4 bars from 8: master: level 0.5\n"),
        ("at bar 16: master: level 0.5\n", "at bar 16: master: level 0.5\n"),
        ("at bar 16 beat 0: master: level 0.5\n", "at bar 16: master: level 0.5\n"),
        ("at bar 4 for 2 bars: master: filter lp 800\n", "at bar 4 for 2 bars: master: filter lp 800\n"),
        // a half bar is two beats; a quarter is a beat; three quarters three beats
        ("master: filter lp 300 -> 2k over 1/2 bar\n", "master: filter lp 300 -> 2000 over 2 beats\n"),
        ("master: filter lp 300 -> 2k over 2 beats\n", "master: filter lp 300 -> 2000 over 2 beats\n"),
        ("master: filter lp 300 -> 2k over 1/4 bars\n", "master: filter lp 300 -> 2000 over 1 beat\n"),
        ("master: filter lp 300 -> 2k over 3/4 bar\n", "master: filter lp 300 -> 2000 over 3 beats\n"),
        ("master: filter lp 300 -> 2k over 5 beats\n", "master: filter lp 300 -> 2000 over 5 beats\n"),
        ("master: filter lp 300 -> 2k over 4 beats\n", "master: filter lp 300 -> 2000 over 1 bars\n"),
        ("master: filter lp 300 -> 2k over 1/3 bar\n", "master: filter lp 300 -> 2000 over 1/3 bars\n"),
        ("master: filter lp 300 -> 2k over 1/8 bar\n", "master: filter lp 300 -> 2000 over 1/8 bars\n"),
        ("master: filter lp 300 -> 2k over 1.5 beats\n", "master: filter lp 300 -> 2000 over 3/8 bars\n"),
        ("master: filter lp 300 -> 2k over (1 + 1) beats\n", "master: filter lp 300 -> 2000 over 2 beats\n"),
        ("for 1/2 bar: master: filter lp 800\n", "for 2 beats: master: filter lp 800\n"),
        ("at bar 2 for 3 beats: master: filter lp 800\n", "at bar 2 for 3 beats: master: filter lp 800\n"),
        ("every 2 beats: master: level 0.5\n", "every 2 beats: master: level 0.5\n"),
        ("every 1/2 bar: master: level 0.5\n", "every 2 beats: master: level 0.5\n"),
        ("every 3/4 bar: master: level 0.5\n", "every 3 beats: master: level 0.5\n"),
        ("every 1/2 bar from 3: master: level 0.5\n", "every 2 beats from 3: master: level 0.5\n"),
        ("every 2 beats from bar 3 beat 2: master: level 0.5\n", "every 2 beats from bar 3 beat 2: master: level 0.5\n"),
        ("every 2 bars from bar 1 beat 2: master: level 0.5\n", "every 2 bars from bar 1 beat 2: master: level 0.5\n"),
        ("every 4 bars from bar 8: master: level 0.5\n", "every 4 bars from 8: master: level 0.5\n"),
        ("at bar 16 beat 3: master: level 0.5\n", "at bar 16 beat 3: master: level 0.5\n"),
        ("at bar 16 beat 5/2: master: level 0.5\n", "at bar 16 beat 5/2: master: level 0.5\n"),
        ("at bar 16 beat 2.5: master: level 0.5\n", "at bar 16 beat 5/2: master: level 0.5\n"),
        ("at bar 3/2: master: level 0.5\n", "at bar 1 beat 2: master: level 0.5\n"),
        ("at bar 2 + 1 beat 1 + 1: master: level 0.5\n", "at bar 3 beat 2: master: level 0.5\n"),
        ("do vocoder set mix=0 -> 1 over 3 beats\n", "do vocoder set mix=0 -> 1 over 3 beats\n"),
    ] {
        let c = ok(src);
        assert_eq!(c.text, want, "{src}");
        // and the canonical text is a fixed point
        assert_eq!(ok(&c.text).text, c.text, "{src}");
    }
    // a section's `before` with a part of a bar
    let c = ok("section intro 4 bars {\n  deck A: kick x\n  before drop 2 beats: master: level 0.5\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n");
    assert!(c.text.contains("  before drop 2 beats: master: level 0.5\n"), "{}", c.text);
    let c = ok("section intro 4 bars {\n  deck A: kick x\n  before drop 1/2 bar: master: level 0.5\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n");
    assert!(c.text.contains("before drop 2 beats"), "{}", c.text);
}

#[test]
fn two_spellings_of_one_time_are_one_program_and_another_time_is_another() {
    let same = [("over 1/2 bar", "over 2 beats"), ("over 1/2 bar", "over 4 / 8 bars"), ("over 1/2 bar", "over 0.5 bars"), ("over 3/4 bars", "over 3 beats"), ("over 1 bar", "over 4 beats")];
    for (a, b) in same {
        let a = ok(&format!("master: filter lp 300 -> 2k {a}\n"));
        let b = ok(&format!("master: filter lp 300 -> 2k {b}\n"));
        assert_eq!(a.cid, b.cid);
    }
    let one = ok("master: filter lp 300 -> 2k over 2 beats\n").cid;
    assert_ne!(one, ok("master: filter lp 300 -> 2k over 3 beats\n").cid);
    assert_ne!(one, ok("master: filter lp 300 -> 2k over 1 bars\n").cid);
    assert_eq!(ok("at bar 2 beat 2: master: level 0.5\n").cid, ok("at bar 5/2: master: level 0.5\n").cid);
    assert_ne!(ok("at bar 2 beat 2: master: level 0.5\n").cid, ok("at bar 2 beat 3: master: level 0.5\n").cid);
}

#[test]
fn a_whole_bar_program_prints_as_it_did() {
    // the fixtures pin the old CIDs elsewhere (tests/canonical.rs); here the whole-bar words one by one
    let src = "every 16 bars: swap pairs with echo_out\non phrase: deck A: filter lp 12k -> 400 over 4 bars\nat bar 8 for 4 bars: master: filter lp 800\nevery 4 bars from 2: master: delay 0.2\nat bar 3: master: level 0.5\nsection drop 8 bars {\n  deck A: kick x\n  master: level 0.5\n}\n";
    assert_eq!(ok(src).text, "every 16 bars: swap pairs with echo_out\non phrase: deck A: filter lp 12000 -> 400 over 4 bars\nat bar 8 for 4 bars: master: filter lp 800\nevery 4 bars from 2: master: delay 0.2\nat bar 3: master: level 0.5\nsection drop 8 bars {\n  deck A: kick x\n  master: level 0.5\n}\n");
    let c = ok("section intro 4 bars {\n  deck A: kick x\n  before drop 2 bars: master: level 0.5\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n");
    assert!(c.text.contains("  before drop 2 bars: master: level 0.5\n"), "{}", c.text);
    assert_eq!(c.controls[0].timing, Timing::At(2));
}

#[test]
fn the_time_words_resolve_to_exact_times() {
    let c = ok("at bar 2 beat 3: master: level 0.5\nevery 2 beats: master: level 0.5\nevery 1/2 bar from 3: master: level 0.5\nat bar 7/2: master: level 0.5\n");
    let t: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(t, vec![Timing::AtPos(r(11, 4)), Timing::EveryPos { step: r(1, 2), from: r(1, 2) }, Timing::EveryPos { step: r(1, 2), from: r(3, 1) }, Timing::AtPos(r(7, 2))]);
    // a whole bar spelled with a part of a bar is the whole bar
    let c = ok("at bar 2 beat 0: master: level 0.5\nevery 8 / 2 bars: master: level 0.5\nat bar 4 / 2: master: level 0.5\n");
    let t: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(t, vec![Timing::At(2), Timing::Every(4), Timing::At(2)]);
    // inside a section the times count from its first bar; `before` counts back from the section
    let src = "section intro 4 bars {\n  deck A: kick x\n  at bar 1 beat 2: master: level 0.5\n  every 2 beats: master: level 0.5\n  before drop 2 beats: master: level 0.4\n}\nsection drop 4 bars {\n  deck A: kick x x\n  at bar 1 beat 2: master: level 0.3\n}\narrange intro drop\n";
    let c = ok(src);
    let t: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(
        t,
        vec![Timing::AtPos(r(3, 2)), Timing::EveryPos { step: r(1, 2), from: r(1, 2) }, Timing::AtPos(r(7, 2)), Timing::AtPos(r(11, 2))],
        "{:?}",
        c.controls.iter().map(|k| &k.key).collect::<Vec<_>>()
    );
}

#[test]
fn the_hold_and_the_ramp_keep_their_exact_length() {
    let c = ok("at bar 4 for 2 beats: master: filter lp 800\nmaster: filter lp 300 -> 2k over 1/2 bar\n");
    assert_eq!(c.controls[0].hold, Some(Dur::from_bars(r(1, 2)).unwrap()));
    assert_eq!(c.controls[0].hold.unwrap().canonical(), "2 beats");
    let Effect::Ramp(ramp) = &c.controls[1].effect else { panic!() };
    assert_eq!(ramp.bars.as_bars(), r(1, 2));
    assert_eq!(c.controls[0].undo.len(), 1);
}

#[test]
fn errors_say_where_and_what_would_do() {
    let e = errors("at bar 16 beat 4: master: level 0.5\n");
    assert_eq!((e[0].0, e[0].1), (1, 16), "{e:?}");
    assert!(e[0].2.contains("a beat is 0 to 4"), "{e:?}");
    let e = errors("at bar 16 beat 7/2 + 1: master: level 0.5\n");
    assert!(e[0].2.contains("a beat is 0 to 4"), "{e:?}");
    let e = errors("at bar 16 beat -1: master: level 0.5\n");
    assert!(e[0].2.contains("a beat is 0 to 4"), "{e:?}");
    for (src, want) in [
        ("every 0 beats: master: level 0.5\n", "every 0 beats would never be"),
        ("every 0 bars: master: level 0.5\n", "every 0 bars would never be"),
        ("every 0 bar: master: level 0.5\n", "every 0 bar would never be"),
        ("for 0 beats: master: filter lp 800\n", "`for 0 beats` would be over before it began"),
        ("for 0 bars: master: filter lp 800\n", "`for 0 bars` would be over before it began"),
        ("master: filter lp 300 -> 2k over 0 beats\n", "a ramp takes some time"),
        ("master: filter lp 300 -> 2k over 2\n", "expected `bars` or `beats`"),
        ("at bar 2: master: filter lp 300 -> 2k over 2 secs\n", "expected `bars` or `beats`"),
        ("every 1 - 2 bars: master: level 0.5\n", "a length is not negative"),
        ("at bar 2 - 3: master: level 0.5\n", "a bar number is not negative"),
        ("at bar 2: move bass_swap on B over 2 beats\n", "a move takes whole bars"),
        ("every 2 beats from 2 - 3: master: level 0.5\n", "a bar number is not negative"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|(_, _, m)| m.contains(want)), "{src:?}: wanted `{want}`, got {e:?}");
    }
    // a section is whole bars, and a distance back that goes before the song is an error that says so
    assert!(!errors("section intro 2 beats {\n  deck A: kick x\n}\n").is_empty());
    let e = errors("section drop 4 bars {\n  deck A: kick x\n  before drop 2 beats: master: level 0.5\n}\narrange drop\n");
    assert!(e[0].2.contains("2 beats before it is before the song"), "{e:?}");
    // a call takes whole bars
    let e = errors("def noisy() {\n  master: level 0.5\n}\nat bar 2 beat 2: noisy()\n");
    assert!(e[0].2.contains("a call of `noisy` takes"), "{e:?}");
}

#[test]
fn a_flat_program_has_no_whole_bar_0_but_a_time_inside_it() {
    // the whole bar 0 is nothing to be `at` (`at bar 1` is the first bar after the program starts) ...
    let e = errors("at bar 0: master: level 0.5\n");
    assert!(e[0].2.contains("bars are counted from 1"), "{e:?}");
    let e = errors("at bar 0 beat 0: master: level 0.5\n");
    assert!(e[0].2.contains("bars are counted from 1"), "{e:?}");
    // ... but a time inside it is fine: beats are counted from 0 (L-07)
    let c = ok("at bar 0 beat 2: master: level 0.5\n");
    assert_eq!(c.controls[0].timing, Timing::AtPos(r(1, 2)));
    // in a song bar 0 is the first bar, and a position in it is fine
    let c = ok("section drop 4 bars {\n  deck A: kick x\n  at bar 0 beat 2: master: level 0.5\n}\narrange drop\n");
    assert_eq!(c.controls[0].timing, Timing::AtPos(r(1, 2)));
}

// ---- the property: a duration or a position is its exact value, printed the one way

fn show_dur(bars: Rat) -> String {
    // the contract, written out independently of dur.rs
    if bars.is_int() {
        format!("{bars} bars")
    } else {
        let beats = bars.mul(Rat::int(4)).unwrap();
        if beats.is_int() {
            if beats == Rat::ONE {
                "1 beat".to_string()
            } else {
                format!("{beats} beats")
            }
        } else {
            format!("{bars} bars")
        }
    }
}

fn show_pos(bars: Rat) -> String {
    if bars.is_int() {
        format!("bar {bars}")
    } else {
        let w = bars.floor();
        let b = bars.sub(Rat::int(w)).unwrap().mul(Rat::int(4)).unwrap();
        format!("bar {w} beat {b}")
    }
}

#[test]
fn random_durations_and_positions_print_as_the_contract_says_and_read_back() {
    let mut rng = Rng(0xD0A7_1111_2222_3333);
    for _ in 0..400 {
        let den = [1, 2, 3, 4, 8, 16, 5][rng.below(7) as usize];
        let num = 1 + rng.below(40) as i64;
        let bars = r(num, den);
        // spelled in bars, in beats, or as a division
        let beats = bars.mul(Rat::int(4)).unwrap();
        let spelling = match rng.below(3) {
            0 => format!("{} bars", if den == 1 { num.to_string() } else { format!("{num}/{den}") }),
            1 => format!("{} beats", if beats.is_int() { beats.to_string() } else { format!("{}/{}", beats.numer(), beats.denom()) }),
            _ => format!("({num} / {den}) bar"),
        };
        let line = format!("master: filter lp 300 -> 2k over {spelling}\n");
        let c = ok(&line);
        assert_eq!(c.text, format!("master: filter lp 300 -> 2000 over {}\n", show_dur(bars)), "{line}");
        let Effect::Ramp(ramp) = &c.controls[0].effect else { panic!() };
        assert_eq!(ramp.bars.as_bars(), bars, "{line}");
        assert_eq!(ok(&c.text).cid, c.cid);
        // the same in `for` and `every`
        let hold = ok(&format!("at bar 3 for {spelling}: master: filter lp 800\n"));
        assert_eq!(hold.text, format!("at bar 3 for {}: master: filter lp 800\n", show_dur(bars)));
        let every = ok(&format!("every {spelling}: master: level 0.5\n"));
        assert_eq!(every.text, format!("every {}: master: level 0.5\n", show_dur(bars)));
        // a position: a bar and a beat of it (a beat 0 to 4, whole or not)
        let w = 1 + rng.below(40) as i64;
        let b = r(rng.below(7) as i64 * 2 + 1, 1 + rng.below(4) as i64);
        let b = if b >= Rat::int(4) { r(b.numer() % (4 * b.denom()), b.denom()) } else { b };
        let total = Rat::int(w).add(b.div(Rat::int(4)).unwrap()).unwrap();
        let beat_spelling = if b.is_int() { b.to_string() } else { format!("{}/{}", b.numer(), b.denom()) };
        let at = ok(&format!("at bar {w} beat {beat_spelling}: master: level 0.5\n"));
        assert_eq!(at.text, format!("at {}: master: level 0.5\n", show_pos(total)));
        assert_eq!(ok(&at.text).cid, at.cid);
    }
}

#[test]
fn dur_and_pos_are_the_exact_value_whatever_the_unit() {
    assert_eq!(Dur::from_beats(r(6, 1)), Dur::from_bars(r(3, 2)));
    assert_eq!(Dur::from_beats(r(6, 1)).unwrap().canonical(), "6 beats");
    assert_eq!(Pos::from_bars(r(131, 8)).unwrap().canonical(), "bar 16 beat 3/2");
    assert_eq!(Pos::from_bars(r(5, 1)).unwrap().canonical_from(), "5");
    assert!(Dur::from_bars(r(-1, 4)).is_none() && Pos::from_bars(r(-1, 4)).is_none());
}

#[test]
fn reformatting_a_program_with_sub_bar_times_keeps_its_cid() {
    let programs = [
        "tempo 124\nat bar 2 beat 3: master: filter lp 300 -> 2k over 1/2 bar\nevery 2 beats: master: level 0.5\nevery 1/2 bar from 3: master: delay 0.3\nat bar 4 for 2 beats: master: filter lp 800\ndeck A: kick x ~ x ~\n",
        "section intro 4 bars {\n  deck A: kick x\n  at bar 1 beat 5/2: master: reverb 0.2\n  before drop 2 beats: do vocoder set mix=0 -> 1 over 3 beats\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n",
        "def roll(deck) {\n  at bar 1: deck $deck: delay 0.2 -> 0.8 over 1/4 bar\n}\nroll(A)\nat bar 3 beat 1 + 1: master: level 0.5\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let c = ok(src);
        let again = ok(&c.text);
        assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()), "program {n}");
        let mut rng = Rng(0x7131_0000 + n as u64);
        for round in 0..120 {
            let t = reformat(src, &mut rng);
            let got = check(&t);
            let got = got.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{t}\n{}", djlive::diag::render_all(&got.diagnostics, &t)));
            assert_eq!(got.cid, c.cid, "program {n} round {round} forked:\n{t}");
        }
    }
}

// ---- the runtime on a fake clock

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
    pauses: Vec<String>,
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
        if !on {
            self.pauses.push(deck.into());
        }
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    /// Advance `beats` in steps of `step` beats, servicing each.
    fn run_by(&mut self, live: &mut Live, beats: f64, step: f64) {
        let steps = (beats / step).round() as usize;
        for _ in 0..steps {
            self.beat += step;
            live.service(self);
        }
    }

    fn run(&mut self, live: &mut Live, beats: f64) {
        self.run_by(live, beats, 0.25);
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }

    /// (beat, the master level sent) of every `mixer` call.
    fn master_levels(&self) -> Vec<(f64, f64)> {
        self.calls_of("mixer").iter().map(|(b, p)| (*b, JsonValue::parse(p).unwrap().get("master").and_then(JsonValue::as_f64).unwrap())).collect()
    }
}

/// A deck A that plays, at beat 8: program bar 0 is the deck's bar 2, so a position of P bars is the beat `8 + 4 P`.
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
fn at_bar_2_beat_3_fires_when_the_position_reaches_it_and_once() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2 beat 3: master: level 0.5\n").ok);
    h.run(&mut live, 4.0 * 6.0);
    let fired = h.calls_of("mixer");
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0].0, beat_at(2.75));
    // nothing a pass before it
    let (mut h, mut live) = playing();
    live.eval("at bar 2 beat 3: master: level 0.5\n");
    h.run(&mut live, beat_at(2.75) - 8.0 - 0.25);
    assert!(h.calls_of("mixer").is_empty());
    h.run(&mut live, 0.25);
    assert_eq!(h.calls_of("mixer").len(), 1);
}

#[test]
fn every_2_beats_fires_every_half_bar_never_at_zero_and_only_from_the_landing_bar() {
    let (mut h, mut live) = playing();
    live.eval("every 2 beats: master: level 0.5\n");
    h.run(&mut live, 4.0 * 4.0);
    let beats: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    // the program lands on bar 1 (beat 12): then every two beats
    assert_eq!(beats, vec![12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0], "{beats:?}");
}

#[test]
fn every_half_bar_from_3_fires_at_3_3_and_a_half_4() {
    let (mut h, mut live) = playing();
    live.eval("every 1/2 bar from 3: master: level 0.5\n");
    h.run(&mut live, 4.0 * 6.0);
    let beats: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    assert_eq!(beats[..4], [beat_at(3.0), beat_at(3.5), beat_at(4.0), beat_at(4.5)], "{beats:?}");
    // a whole-bar `every` is what it was
    let (mut h, mut live) = playing();
    live.eval("every 2 bars from 2: master: level 0.5\n");
    h.run(&mut live, 4.0 * 7.0);
    let beats: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    assert_eq!(beats[..2], [beat_at(2.0), beat_at(4.0)]);
}

#[test]
fn over_half_a_bar_ends_exactly_on_its_value() {
    let (mut h, mut live) = playing();
    live.eval("at bar 2: master: level 0.2 -> 0.8 over 1/2 bar\n");
    h.run(&mut live, 4.0 * 5.0);
    let sends = h.master_levels();
    assert_eq!(sends.first().unwrap(), &(beat_at(2.0), 0.2));
    // the last send is the end value, exactly two beats after the first, and nothing after it
    assert_eq!(sends.last().unwrap(), &(beat_at(2.0) + 2.0, 0.8), "{sends:?}");
    // it moved smoothly in between (never going back)
    assert!(sends.windows(2).all(|w| w[0].1 < w[1].1 && w[1].0 > w[0].0), "{sends:?}");
    assert!(sends.len() >= 4 && sends.len() <= 9, "{sends:?}");
    // an odd length: a third of a bar
    let (mut h, mut live) = playing();
    live.eval("at bar 2: master: level 0 -> 1 over 1/3 bar\n");
    h.run(&mut live, 4.0 * 5.0);
    let sends = h.master_levels();
    let last = sends.last().unwrap();
    assert_eq!(last.1, 1.0);
    // the first pass at or after a third of a bar (1 1/3 beats) from the start
    assert!(last.0 >= beat_at(2.0) + 4.0 / 3.0 && last.0 < beat_at(2.0) + 4.0 / 3.0 + 0.25, "{sends:?}");
}

#[test]
fn a_ramp_that_starts_between_bar_lines_starts_at_its_time() {
    let (mut h, mut live) = playing();
    live.eval("at bar 2 beat 2: master: level 0 -> 1 over 2 beats\n");
    h.run(&mut live, 4.0 * 5.0);
    let sends = h.master_levels();
    assert_eq!(sends.first().unwrap(), &(beat_at(2.5), 0.0), "{sends:?}");
    assert_eq!(sends.last().unwrap(), &(beat_at(2.5) + 2.0, 1.0), "{sends:?}");
    // one beat in it is at a half
    let mid = sends.iter().find(|s| s.0 == beat_at(2.5) + 1.0).unwrap();
    assert!((mid.1 - 0.5).abs() < 1e-9, "{mid:?}");
}

#[test]
fn for_2_beats_is_put_back_two_beats_on() {
    fn ons(h: &Fake) -> Vec<(f64, bool)> {
        h.calls_of("inserts").iter().filter(|c| c.1.contains("\"action\":\"on\"")).map(|(b, p)| (*b, p.contains("\"on\":true"))).collect()
    }
    let (mut h, mut live) = playing();
    live.eval("at bar 2 for 2 beats: master: filter lp 800\n");
    h.run(&mut live, 4.0 * 5.0);
    assert_eq!(ons(&h), vec![(beat_at(2.0), true), (beat_at(2.0) + 2.0, false)]);
    // after a time between bar lines
    let (mut h, mut live) = playing();
    live.eval("at bar 2 beat 1 for 1 beat: master: filter lp 800\n");
    h.run(&mut live, 4.0 * 5.0);
    assert_eq!(ons(&h), vec![(beat_at(2.25), true), (beat_at(2.25) + 1.0, false)]);
    // a length that is not a whole number of beats is put back at the first pass that has reached it
    let (mut h, mut live) = playing();
    live.eval("at bar 2 for 3/8 bars: master: filter lp 800\n");
    h.run(&mut live, 4.0 * 5.0);
    let o = ons(&h);
    assert_eq!(o.len(), 2);
    assert_eq!(o[1].0, beat_at(2.0) + 1.5);
    // whole bars are put back on the bar line, as they were
    let (mut h, mut live) = playing();
    live.eval("at bar 2 for 2 bars: master: filter lp 800\n");
    h.run(&mut live, 4.0 * 6.0);
    assert_eq!(ons(&h), vec![(beat_at(2.0), true), (beat_at(4.0), false)]);
}

#[test]
fn before_drop_2_beats_is_a_half_bar_before_the_section() {
    let src = "section intro 4 bars {\n  deck A: kick x\n  before drop 2 beats: master: reverb 0.4\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 7.0);
    let rev: Vec<f64> = h.calls_of("inserts").iter().filter(|c| c.1.contains("reverb.mix")).map(|c| c.0).collect();
    // the song lands on program bar 1, the drop is its bar 4 (program bar 5): half a bar earlier is 4.5
    assert_eq!(rev, vec![beat_at(4.5)], "{rev:?}");
    // the same line in whole bars is a bar earlier than that
    let whole = src.replace("before drop 2 beats", "before drop 1 bar");
    let (mut h, mut live) = playing();
    assert!(live.eval(&whole).ok);
    h.run(&mut live, 4.0 * 7.0);
    let rev: Vec<f64> = h.calls_of("inserts").iter().filter(|c| c.1.contains("reverb.mix")).map(|c| c.0).collect();
    assert_eq!(rev, vec![beat_at(4.0)], "{rev:?}");
}

#[test]
fn a_section_counts_its_sub_bar_times_from_its_own_first_bar() {
    let src = "section intro 2 bars {\n  deck A: kick x\n}\nsection drop 4 bars {\n  deck A: kick x x\n  at bar 1 beat 2: master: delay 0.3\n  every 2 beats: master: reverb 0.1\n}\narrange intro drop\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 7.0);
    // the song's first bar is program bar 1: the drop starts on 3, `bar 1 beat 2` is 4.5
    let delay: Vec<f64> = h.calls_of("inserts").iter().filter(|c| c.1.contains("delay.mix")).map(|c| c.0).collect();
    assert_eq!(delay, vec![beat_at(4.5)]);
    // `every 2 beats` in a section starts one step into it: 3.5, 4, 4.5 ...
    let rev: Vec<f64> = h.calls_of("inserts").iter().filter(|c| c.1.contains("reverb.mix")).map(|c| c.0).collect();
    assert_eq!(rev[..3], [beat_at(3.5), beat_at(4.0), beat_at(4.5)], "{rev:?}");
}

#[test]
fn a_time_fires_once_whatever_the_pass_rate() {
    let src = "at bar 2 beat 3: master: level 0.5\nevery 2 beats: master: delay 0.1\nevery 1/2 bar from 3: master: reverb 0.1\nat bar 3 for 1/2 bar: master: filter lp 800\n";
    let mut reference: Option<Vec<(String, String)>> = None;
    for step in [0.0625, 0.125, 0.25, 0.5, 1.0] {
        let (mut h, mut live) = playing();
        assert!(live.eval(src).ok);
        h.run_by(&mut live, 4.0 * 6.0, step);
        // what was called up to a point well inside the run (the lines are the same at every rate; the order of two
        // lines due in one pass may differ with the pass, so compare as sets of calls)
        let mut seen: Vec<(String, String)> = h.calls.iter().filter(|c| c.0 <= beat_at(5.0)).map(|c| (c.1.clone(), c.2.clone())).collect();
        seen.sort();
        match &reference {
            None => reference = Some(seen),
            Some(r0) => assert_eq!(r0, &seen, "step {step}"),
        }
    }
    assert!(reference.unwrap().len() > 8);
}

#[test]
fn the_calls_are_at_the_same_beats_at_any_pass_rate_that_hits_the_time() {
    // every pass of a quarter beat or finer meets a time that is a multiple of a quarter beat
    let src = "at bar 2 beat 3: master: level 0.5\nevery 2 beats: master: delay 0.1\n";
    let mut got = Vec::new();
    for step in [0.0625, 0.125, 0.25] {
        let (mut h, mut live) = playing();
        live.eval(src);
        h.run_by(&mut live, 4.0 * 5.0, step);
        got.push(h.calls.iter().map(|c| (c.0, c.1.clone())).collect::<Vec<_>>());
    }
    assert_eq!(got[0], got[1]);
    assert_eq!(got[1], got[2]);
}

#[test]
fn an_edit_that_leaves_a_sub_bar_line_alone_keeps_its_schedule() {
    let (mut h, mut live) = playing();
    live.eval("every 2 beats: master: level 0.5\n");
    h.run(&mut live, 4.0 * 2.0);
    let n = h.calls_of("mixer").len();
    // another line is added: the one that ran goes on at its own pace, it is not fired again for the edit
    assert!(live.eval("every 2 beats: master: level 0.5\nat bar 20: master: delay 0.1\n").ok);
    h.run(&mut live, 4.0 * 2.0);
    let beats: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    assert!(beats.len() > n);
    assert!(beats.windows(2).all(|w| w[1] - w[0] == 2.0), "{beats:?}");
    // a line that is changed starts over from the bar it lands on
    assert!(live.eval("every 1/2 bar from 1: master: level 0.5\n").ok);
    h.run(&mut live, 4.0 * 2.0);
    assert!(h.calls_of("mixer").len() > beats.len());
}

#[test]
fn a_program_that_ends_does_not_fire_what_is_after_its_end_and_a_catch_up_is_bounded() {
    let (mut h, mut live) = playing();
    live.eval("every 2 beats: master: level 0.5\nat bar 3 beat 2: master: delay 0.1\nend at bar 3\n");
    h.run(&mut live, 4.0 * 6.0);
    assert!(!live.is_running());
    let beats: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    assert!(beats.iter().all(|b| *b < beat_at(3.0)), "{beats:?}");
    assert!(h.calls_of("inserts").is_empty(), "bar 3 beat 2 is after the end");
    // a long stall: the clock jumps 100 bars at once, and a line that repeats twice a bar does not fire 200 times
    let (mut h, mut live) = playing();
    live.eval("every 2 beats: master: level 0.5\n");
    h.run(&mut live, 4.0 * 2.0);
    let n = h.calls_of("mixer").len();
    h.beat += 4.0 * 100.0;
    live.service(&mut h);
    let fired = h.calls_of("mixer").len() - n;
    assert!(fired > 0 && fired <= 40, "{fired}");
}

#[test]
fn the_editor_marks_see_a_sub_bar_line_fire_and_a_ramp_move() {
    let (mut h, mut live) = playing();
    live.eval("at bar 2 beat 2: master: filter lp 300 -> 2k over 2 beats\n");
    h.run(&mut live, beat_at(2.5) - 8.0 + 0.5);
    let m = live.marks();
    let c = m.controls.iter().find(|c| c.key.contains("filter lp")).expect("the line fired");
    assert!(c.age_beats <= 1.0, "{}", c.age_beats);
    assert!(c.ramp.is_some_and(|p| p > 0.0 && p < 1.0), "{:?}", c.ramp);
    assert!(c.span.len() > 0);
    h.run(&mut live, 2.0);
    let m = live.marks();
    assert!(m.controls.iter().find(|c| c.key.contains("filter lp")).is_none_or(|c| c.ramp.is_none()), "the ramp is over");
}

#[test]
fn the_status_names_what_waits_and_what_ran() {
    let (mut h, mut live) = playing();
    live.eval("at bar 3 beat 2: master: level 0.5\n");
    h.run(&mut live, 4.0 * 2.0);
    let state = |st: &JsonValue| st.get("controls").and_then(|c| c.as_array().and_then(|a| a[0].get("state").and_then(JsonValue::as_str).map(str::to_string)));
    assert_eq!(state(&live.status()).as_deref(), Some("waiting"));
    h.run(&mut live, 4.0 * 2.0);
    assert_eq!(state(&live.status()).as_deref(), Some("ran"));
}

#[test]
fn random_positions_fire_exactly_at_their_beat_once() {
    let mut rng = Rng(0xFEED_5EED_0000_0007);
    for _ in 0..40 {
        let bar = 2 + rng.below(4) as i64;
        let beat = rng.below(16) as i64; // quarter beats of the bar: 0 to 3 3/4
        let beat_n = Rat::new(beat, 4).unwrap();
        let spelled = if beat_n.is_int() { beat_n.to_string() } else { format!("{}/{}", beat_n.numer(), beat_n.denom()) };
        let src = format!("at bar {bar} beat {spelled}: master: level 0.5\n");
        let (mut h, mut live) = playing();
        assert!(live.eval(&src).ok, "{src}");
        h.run(&mut live, 4.0 * 10.0);
        // `beat` here counts quarter beats: bar + (beat / 4) / 4 bars
        let at = beat_at(bar as f64 + (beat as f64 / 4.0) / 4.0);
        let fired = h.calls_of("mixer");
        assert_eq!(fired.len(), 1, "{src}: {fired:?}");
        assert_eq!(fired[0].0, at, "{src}");
    }
}

#[test]
fn a_looping_song_fires_its_sub_bar_lines_again_each_lap() {
    let src = "section loopy 2 bars {\n  deck A: kick x\n  at bar 0 beat 2: master: level 0.5\n  every 2 beats: master: delay 0.1\n}\narrange loopy\nloop\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 5.0);
    // the song lands on program bar 1: `bar 0 beat 2` is 1.5, then again a lap (2 bars) later
    let lvl: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
    assert_eq!(lvl[..2], [beat_at(1.5), beat_at(3.5)], "{lvl:?}");
    // `every 2 beats` in the section starts a step in and never at the section's own first bar line, so each lap has
    // 1.5, 2, 2.5 and then 3.5, 4, 4.5: the lap line (3) is the next lap's first bar line
    let d: Vec<f64> = h.calls_of("inserts").iter().filter(|c| c.1.contains("delay.mix")).map(|c| c.0).collect();
    assert_eq!(d, vec![beat_at(1.5), beat_at(2.0), beat_at(2.5), beat_at(3.5), beat_at(4.0), beat_at(4.5)], "{d:?}");
}

#[test]
fn a_hold_on_a_line_with_no_time_word_starts_on_the_first_bar_line() {
    let (mut h, mut live) = playing();
    live.eval("for 2 beats: master: filter lp 800\n");
    h.run(&mut live, 4.0 * 3.0);
    let ons: Vec<(f64, bool)> = h.calls_of("inserts").iter().filter(|c| c.1.contains("\"action\":\"on\"")).map(|(b, p)| (*b, p.contains("\"on\":true"))).collect();
    assert_eq!(ons, vec![(beat_at(1.0), true), (beat_at(1.5), false)]);
}

#[test]
fn stopping_puts_back_a_hold_that_has_not_run_out() {
    let (mut h, mut live) = playing();
    live.eval("at bar 2 for 2 beats: master: filter lp 800\n");
    h.run(&mut live, beat_at(2.25) - 8.0);
    let off = |h: &Fake| h.calls_of("inserts").iter().filter(|c| c.1.contains("\"on\":false")).count();
    assert_eq!(off(&h), 0);
    live.stop_with(&mut h);
    assert_eq!(off(&h), 1, "the filter is put back when the program stops");
}

#[test]
fn random_repeating_lines_fire_on_exactly_their_times() {
    let mut rng = Rng(0xAB12_CD34_0000_0099);
    for _ in 0..30 {
        let step_beats = 1 + rng.below(7) as i64; // every 1 to 7 beats
        let from_beats = rng.below(9) as i64 * 2; // from bar 0 to 4, in beats
        let pass = [0.25, 0.5, 1.0][rng.below(3) as usize];
        let from_bar = Rat::new(from_beats, 4).unwrap();
        let from = if from_bar.is_int() { from_bar.to_string() } else { format!("{}/{}", from_bar.numer(), from_bar.denom()) };
        let src = format!("every {step_beats} beats from {from}: master: level 0.5\n");
        let (mut h, mut live) = playing();
        assert!(live.eval(&src).ok, "{src}");
        h.run_by(&mut live, 4.0 * 8.0, pass);
        let got: Vec<f64> = h.calls_of("mixer").iter().map(|c| c.0).collect();
        // the times are `from + k step` bars... in beats of the clock: 8 + 4 * from_bar + k * step; from the landing bar line
        // (beat 12) on, up to the last pass
        let end = 8.0 + 4.0 * 8.0;
        let mut want = Vec::new();
        let mut k = 0;
        loop {
            let beat = 8.0 + from_beats as f64 + (k * step_beats) as f64;
            if beat > end {
                break;
            }
            if beat >= 12.0 {
                want.push(beat);
            }
            k += 1;
        }
        // a pass sees a time at the first pass at or after it
        let seen: Vec<f64> = want.iter().map(|b| (b / pass).ceil() * pass).collect();
        assert_eq!(got.len(), want.len(), "{src} pass {pass}: {got:?} {want:?}");
        assert_eq!(got, seen, "{src} pass {pass}");
    }
}

#[test]
fn existing_programs_keep_their_cids() {
    // computed on the commit before the extensions and on this one: the same
    for (name, src, cid) in [
        ("hand-dubstep-140", include_str!("fixtures/hand-dubstep-140.beatscript"), "z4EBG9jD5z7Kk3J1PeAG7PmE1hTNikEv1rNXWTdpYux1EDAEmMR"),
        ("hand-proghouse-124", include_str!("fixtures/hand-proghouse-124.beatscript"), "z4EBG9jCfiLnPMeYSVLsEYac76udGyYPwuJ5iy5xgd3GRncvasZ"),
        ("acid-rave-138", include_str!("fixtures/acid-rave-138.beatscript"), "z4EBG9jDqdJiCPANzF2EYvMTjnh6k4g66cMwwxtSKAYhaZLMUb2"),
        ("hand-dubstep-140-v2", include_str!("../../../docs/beatscript/hand-dubstep-140-v2.beatscript"), "z4EBG9jEj3ephVYiwBA3bmUnp5GmMQ8hjtdgFSbrfgKHp3cJfef"),
        ("trance-138", include_str!("../../../examples/trance-138.beatscript"), "z4EBG9j5MMozvDkT1gTck6ZKTsqXctuNA25bzD86dkhc6buYuuB"),
        ("markers-demo", include_str!("../../../examples/markers-demo.beatscript"), "z4EBG9jETvW85s1uQCB4t3z6xFfsgiAPmpJm6Gg9R8DuTd3rZZy"),
    ] {
        assert_eq!(ok(src).cid, cid, "{name}");
    }
}

#[test]
fn an_every_finer_than_the_clock_is_an_error() {
    let e = errors("every 1/100 bars: master: level 0.5\n");
    assert!(e[0].2.contains("faster than the clock"), "{e:?}");
    assert!(check("every 1/64 bars: master: level 0.5\n").ok(), "1/64 of a bar is the finest");
    assert!(check("every 1/4 beat: master: level 0.5\n").ok());
    assert!(!errors("every 1/1000000000000 bars: master: level 0.5\n").is_empty());
}
