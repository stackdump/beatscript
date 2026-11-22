//! Fixes of the second review (fixer b): the findings of the laws and runtime reviewers, each with the repro that failed.
//! L-05 chance seeds, L-09 chance extremes, L-10/L-21 the period of chance, L-07 time inside bar 0, L-08 folding and printing.

mod common;

use djengine::json::JsonValue;
use djlive::assertion::Status;
use djlive::ast::Deck;
use djlive::compile::{check, Compiled, Timing};
use djlive::diag::{Diagnostic, Severity};
use djlive::rat::Rat;
use djlive::runtime::{Grid, Live, LiveHost};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "should not compile: {src}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

/// What the check says that is no error: warnings and notes (a chance, a period and an ontology name are `info` since SYN-20).
fn warnings(src: &str) -> Vec<String> {
    check(src).diagnostics.into_iter().filter(|d| d.severity != Severity::Error).map(|d| d.message).collect()
}

/// The steps of every hit of a part, per bar of the net (the notes left out: an `oct=` moves them, not which hits drop).
fn steps(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<u8>> {
    c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role}")).cycles.iter().map(|b| b.iter().map(|h| h.step).collect()).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// L-05: a chance part's implicit seed is of the deck, the role and the pattern, not of its params

#[test]
fn l05_a_velocity_an_instrument_or_an_octave_does_not_reroll_which_hits_drop() {
    let base = steps(&ok("deck A: kick x?0.5 x x?0.5 x\n"), Deck::A, "kick");
    for params in ["vel=90", "vel=91", "vel=60..100", "len=2", "vel=100 len=3"] {
        let c = ok(&format!("deck A: kick x?0.5 x x?0.5 x {params}\n"));
        assert_eq!(steps(&c, Deck::A, "kick"), base, "{params}");
    }
    let bass = steps(&ok("deck A: bass c2?0.5 e2?0.5 g2?0.5 b2?0.5\n"), Deck::A, "bass");
    for params in ["inst=sub-bass", "oct=3", "oct=1 vel=80", "inst=sub-bass oct=3 vel=90"] {
        let c = ok(&format!("deck A: bass c2?0.5 e2?0.5 g2?0.5 b2?0.5 {params}\n"));
        assert_eq!(steps(&c, Deck::A, "bass"), bass, "{params}");
    }
    // and the same through a section, where the net is the section's length
    let sec = |params: &str| format!("section run 32 bars {{\n  deck A: hihat x*16?0.5 {params}\n}}\narrange run\n");
    let seg_steps = |c: &Compiled| -> Vec<Vec<u8>> { c.segments[0].decks[&Deck::A].parts[0].cycles.iter().map(|b| b.iter().map(|h| h.step).collect()).collect() };
    assert_eq!(seg_steps(&ok(&sec(""))), seg_steps(&ok(&sec("vel=40..80"))));
}

#[test]
fn l05_the_deck_the_role_and_the_pattern_still_make_the_toss() {
    let base = steps(&ok("deck A: kick x*16?0.5\n"), Deck::A, "kick");
    // another deck, another pattern: another toss (an explicit `seed=` is the way to keep one across them)
    assert_ne!(steps(&ok("deck B: kick x*16?0.5\n"), Deck::B, "kick"), base);
    assert_ne!(steps(&ok("deck A: kick x*16?0.51\n"), Deck::A, "kick"), base);
    assert_ne!(steps(&ok("deck A: snare x*16?0.5\n"), Deck::A, "snare"), base);
    assert_eq!(steps(&ok("deck A: kick x*16?0.5 seed=7\n"), Deck::A, "kick"), steps(&ok("deck A: kick x*16?0.5 seed=7 vel=50\n"), Deck::A, "kick"));
}

#[test]
fn l05_a_part_with_no_chance_keeps_the_seed_it_always_had() {
    // a markov part's default seed is still of its whole text, so the notes (and the CIDs) of old programs do not move
    let a = ok("key am\ndeck A: melody markov(am, 0.5) vel=90\n");
    let b = ok("key am\ndeck A: melody markov(am, 0.5) vel=91\n");
    let notes = |c: &Compiled| c.decks[&Deck::A].parts[0].cycles.iter().map(|b| b.iter().map(|h| (h.step, h.note)).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_ne!(notes(&a), notes(&b));
}

// ---------------------------------------------------------------------------------------------------------------
// L-09: a chance of 0 is a rest and a chance of 1 is no chance

#[test]
fn l09_a_chance_of_nothing_or_everything_is_the_program_written_out() {
    let rest = ok("deck A: kick ~ ~ x x\n");
    for never in ["~ ~ x x", "x?0 ~ x x", "x?0% ~ x x", "x?0.0 ~ x x", "x?(1 - 1) ~ x x"] {
        let c = ok(&format!("deck A: kick {never}\n"));
        assert_eq!(c.cid, rest.cid, "{never}");
        assert_eq!(c.text, rest.text, "{never}");
    }
    let bare = ok("deck A: kick x x x x\n");
    for always in ["x?1 x x x", "x?100% x x x", "x?1.0 x x x", "x?(1 / 1) x x x"] {
        let c = ok(&format!("deck A: kick {always}\n"));
        assert_eq!(c.cid, bare.cid, "{always}");
        assert_eq!(c.text, bare.text, "{always}");
    }
    // the term in front keeps its repeat, its accent and its group; chained chances fold one after the other
    assert_eq!(ok("deck A: kick x*8?1 ~\n").cid, ok("deck A: kick x*8 ~\n").cid);
    assert_eq!(ok("deck A: kick x*8?0 x\n").cid, ok("deck A: kick ~ x\n").cid);
    assert_eq!(ok("deck A: bass [c2 e2]?1 g2!?1\n").cid, ok("deck A: bass [c2 e2] g2!\n").cid);
    assert_eq!(ok("deck A: kick x?0?0.5 x\n").cid, ok("deck A: kick ~ x\n").cid);
    assert_eq!(ok("deck A: kick x?0.5?1 x\n").cid, ok("deck A: kick x?0.5 x\n").cid);
    // so the net is one bar and `check` does not say the program differs every bar
    let c = ok("deck A: kick x?0 x?1 x x\n");
    assert_eq!(c.decks[&Deck::A].period, 1);
    assert!(warnings("deck A: kick x?0 x?1 x x\n").iter().all(|w| !w.contains("differs every bar")));
    // a real chance still does
    assert!(warnings("deck A: kick x?0.5 x x x\n").iter().any(|w| w.contains("differs every bar")));
    // a chance that is not 0 or 1 is a different program from either
    assert_ne!(ok("deck A: kick x?0.999999 x x x\n").cid, bare.cid);
    assert_ne!(ok("deck A: kick x?0.000001 x x x\n").cid, ok("deck A: kick ~ x x x\n").cid);
    // the mark after a folded chance is still an error
    let e = errors("deck A: kick x?1*2\n");
    assert!(e[0].message.contains("comes before the chance"), "{}", e[0].message);
}

// ---------------------------------------------------------------------------------------------------------------
// L-10, L-21: chance has no period, and the rest of a program's period does not move with the dice

#[test]
fn l10_assert_period_of_a_chance_program_says_it_has_none_wherever_it_is_placed() {
    let flat = "deck A: kick x?0.5\n";
    let in4 = "section dice 4 bars {\n  deck A: kick x?0.5\n}\narrange dice\n";
    let in200 = "section dice 200 bars {\n  deck A: kick x?0.5\n}\narrange dice\n";
    let mut said = Vec::new();
    for src in [flat, in4, in200] {
        let c = djlive::compile::check_report(&format!("{src}assert period == 1\n"), &djlive::env::Env::none());
        // (the assertion is judged and false: `eval` refuses the program, `check_report` still hands it back with the verdict)
        let comp = c.compiled.expect("compiles");
        assert_eq!(comp.assertions[0].status, Status::Fail, "a period that is not there is not 1: {src}");
        said.push(comp.assertions[0].line());
        // the period note of meter does not move either
        let p = djlive::meter::program_period(&ok(src));
        assert!(p.iter().all(|(_, p)| p.bars == 1 && p.chance == ["kick on deck A"]), "{p:?}");
    }
    assert_eq!(said[0], said[1]);
    assert_eq!(said[1], said[2]);
    assert!(said[0].starts_with("assert period == 1: the program has no period: kick on deck A has chance, which differs every bar"), "{}", said[0]);
    // every comparison says so: there is nothing to compare
    for op in ["!=", "<", ">=", "=="] {
        let c = djlive::compile::check_report(&format!("{flat}assert period {op} 3\n"), &djlive::env::Env::none());
        assert_eq!(c.compiled.unwrap().assertions[0].status, Status::Fail, "{op}");
        assert!(check(&format!("{flat}assert period {op} 3\n")).compiled.is_none(), "eval refuses it: {op}");
    }
}

#[test]
fn l10_the_polymeter_period_stays_exact_beside_a_chance_part() {
    // hihat/12 against the bar is 3 bars; the dice on the other part are no period, and do not make it 18
    let src = "deck A: hihat/12 x x ~ x | kick x?0.5\n";
    let c = ok(src);
    let p = djlive::meter::period_of(&c.decks);
    assert_eq!((p.bars, p.steps), (3, 48));
    assert_eq!(p.polymeter, ["hihat/12"]);
    assert_eq!(p.chance, ["kick on deck A"]);
    let e = errors(&format!("{src}assert period == 3\n"));
    assert!(e[0].message.ends_with("(the rest comes round in 3 bars (hihat/12 against the bar))"), "{}", e[0].message);
    // the same in a section of any length
    for bars in [4, 12, 40, 200] {
        let src = format!("section s{bars} {bars} bars {{\n  deck A: hihat/12 x x ~ x | kick x?0.5\n}}\narrange s{bars}\n");
        let p = djlive::meter::program_period(&ok(&src));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].1.bars, 3, "{bars}");
    }
    // a program with no chance is exactly what it was
    let c = ok("deck A: hihat/12 x x ~ x | kick x ~ x ~\n");
    assert_eq!(djlive::meter::period_of(&c.decks).bars, 3);
    assert!(djlive::meter::period_of(&c.decks).chance.is_empty());
}

// ---------------------------------------------------------------------------------------------------------------
// L-07: a time inside bar 0 of a program with no sections

/// A deck A that plays, at beat 8: program bar 0 is the deck's bar 2, so a position of P bars is the beat `8 + 4 P`.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
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
    fn beats_of(&self, noun: &str) -> Vec<f64> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| c.0).collect()
    }
}

fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

#[test]
fn l07_a_time_inside_bar_0_is_legal_and_fires_once_between_the_start_and_bar_1() {
    for src in ["at bar 0 beat 2: master: level 0.5\n", "at bar 1/2: master: level 0.5\n", "at bar 0.5: master: level 0.5\n", "at bar 0 beat 1 + 1: master: level 0.5\n"] {
        let c = ok(src);
        assert_eq!(c.controls[0].timing, Timing::AtPos(Rat::new(1, 2).unwrap()), "{src}");
        assert_eq!(c.text, "at bar 0 beat 2: master: level 0.5\n", "{src}");
        let (mut h, mut live) = playing();
        assert!(live.eval(src).ok, "{src}");
        h.run(&mut live, 4.0 * 5.0);
        // program bar 0 is the deck's bar 2 (beat 8): half a bar into it is beat 10, before the program lands on bar 1 (beat 12)
        assert_eq!(h.beats_of("mixer"), [10.0], "{src}");
    }
    // a quarter: at bar 1/4 is the beat after the start
    let (mut h, mut live) = playing();
    live.eval("at bar 1/4: master: level 0.5\nat bar 3/4: master: level 0.6\nat bar 1 beat 2: master: level 0.7\n");
    h.run(&mut live, 4.0 * 3.0);
    assert_eq!(h.beats_of("mixer"), [9.0, 11.0, 14.0]);
}

#[test]
fn l07_the_whole_bar_0_is_still_nothing_to_be_at() {
    for src in ["at bar 0: master: level 0.5\n", "at bar 0 beat 0: master: level 0.5\n", "at bar 1 - 1: master: level 0.5\n"] {
        let e = errors(src);
        assert!(e[0].message.contains("bars are counted from 1"), "{src}: {}", e[0].message);
    }
    // and a negative one is an error too
    assert!(!check("at bar 0 beat -1: master: level 0.5\n").ok());
    // `every 1/2 bar from 1/4` was always fine at the top
    assert!(check("every 1/2 bar from 1/4: master: level 0.5\n").ok());
}

// ---------------------------------------------------------------------------------------------------------------
// L-08: arithmetic folds to the number it comes to, a short decimal prints as itself, any other ratio as the fraction

fn text(src: &str) -> String {
    ok(src).text
}

#[test]
fn l08_arithmetic_with_no_input_is_one_program_with_its_value() {
    // control values, tempo, `e()` arguments, params, a swing, a `do` value: all fold, as `at bar 1 + 1` and `over 1 + 1 beats` did
    for (arith, lit) in [
        ("master: level 0.25 + 0.25\n", "master: level 0.5\n"),
        ("master: filter lp 100 * 4 + 100\n", "master: filter lp 500\n"),
        ("master: level 1 / 4\n", "master: level 0.25\n"),
        ("master: level 0.1 + 0.2\n", "master: level 0.3\n"),
        ("master: level 0.5 -> 0.25 + 0.25 over 2 bars\n", "master: level 0.5 -> 0.5 over 2 bars\n"),
        ("tempo 120 + 8\n", "tempo 128\n"),
        ("tempo 100 -> 120 + 20 over 4 bars\n", "tempo 100 -> 140 over 4 bars\n"),
        ("swing 1 / 8 + 1 / 8\n", "swing 0.25\n"),
        ("deck A: kick e(1 + 2, 4 * 2)\n", "deck A: kick e(3, 8)\n"),
        ("deck A: bass c2 vel=(60 + 12)..100\n", "deck A: bass c2 vel=72..100\n"),
        ("deck A: bass c2 oct=min(3, 4)\n", "deck A: bass c2 oct=3\n"),
        ("do vocoder set mix=0.25 * 2\n", "do vocoder set mix=0.5\n"),
        ("master: level max(0.25, 1 / 8) * 2\n", "master: level 0.5\n"),
    ] {
        let (a, b) = (ok(arith), ok(lit));
        assert_eq!(a.text, lit, "{arith}");
        assert_eq!(a.cid, b.cid, "{arith} is {lit}");
        // the canonical text is a fixed point
        assert_eq!(ok(&a.text).text, a.text, "{arith}");
    }
}

#[test]
fn l08_a_short_decimal_prints_as_itself_and_any_other_ratio_as_the_fraction() {
    // inside what names an input, constants are folded and a short decimal is a decimal
    assert_eq!(text("master: level 0.5 when knob(1) > 0.5\n"), "master: level 0.5 when knob(1) > 0.5\n");
    assert_eq!(text("master: level knob(1) * 0.7\n"), "master: level knob(1) * 0.7\n");
    assert_eq!(text("master: level knob(1) * (0.5 + 0.2)\n"), "master: level knob(1) * 0.7\n");
    assert_eq!(text("master: level knob(1) * 1 / 4\n"), "master: level knob(1) * 1 / 4\n");
    assert_eq!(text("master: level knob(1) * (1 / 3)\n"), "master: level knob(1) * (1 / 3)\n");
    assert_eq!(text("master: level knob(1) * -0.5\n"), "master: level knob(1) * -0.5\n");
    assert_eq!(text("master: filter lp 200 + knob(3) * 7800\n"), "master: filter lp 200 + knob(3) * 7800\n");
    assert_eq!(text("master: level 0.5 when knob(1) > 0.25 + 0.25 and knob(2) <= 3 / 8\n"), "master: level 0.5 when knob(1) > 0.5 and knob(2) <= 0.375\n");
    // a value that is not a decimal of six places stays the exact fraction
    assert_eq!(text("master: level 0.5 when knob(1) > 1 / 3\n"), "master: level 0.5 when knob(1) > 1 / 3\n");
    assert_eq!(text("master: level 0.5 when knob(1) > 1 / 64 + 1 / 64\n"), "master: level 0.5 when knob(1) > 0.03125\n");
    assert_eq!(text("master: level 0.5 when knob(1) > 1 / 128\n"), "master: level 0.5 when knob(1) > 1 / 128\n");
    // each is a fixed point and keeps its CID through the reformatting
    let mut rng = common::Rng(0x1808);
    for src in [
        "master: level knob(1) * 0.7 when knob(2) > 0.5\n",
        "master: level knob(1) * (1 / 3) when knob(2) <= 1 / 64\n",
        "master: level 0.999999 * knob(1)\n",
        "master: level 0.000001 + knob(1)\n",
    ] {
        let c = ok(src);
        assert_eq!(ok(&c.text).text, c.text, "{src}");
        assert_eq!(ok(&c.text).cid, c.cid, "{src}");
        for _ in 0..20 {
            assert_eq!(ok(&common::reformat(src, &mut rng)).cid, c.cid, "{src}");
        }
    }
}

#[test]
fn l08_every_decimal_of_six_places_prints_as_that_decimal_and_reads_back_the_same() {
    let mut rng = common::Rng(0x0808_0001);
    for _ in 0..300 {
        let places = (rng.next() % 7) as u32;
        let den = 10u64.pow(places);
        let k = rng.next() % (3 * den + 1);
        let text = if places == 0 { format!("{k}") } else { format!("{}.{:0w$}", k / den, k % den, w = places as usize) };
        let src = format!("master: level knob(1) * {text}\n");
        let c = ok(&src);
        let shown = c.text.trim_start_matches("master: level knob(1) * ").trim_end().to_string();
        // the shortest spelling of the same number: no trailing zeros
        let want = if places == 0 { format!("{k}") } else { format!("{}.{:0w$}", k / den, k % den, w = places as usize).trim_end_matches('0').trim_end_matches('.').to_string() };
        assert_eq!(shown, want, "{text}");
        assert_eq!(ok(&c.text).cid, c.cid, "{text}");
    }
}

#[test]
fn l08_a_let_of_a_ratio_is_the_fraction_and_not_a_rounded_float() {
    let c = ok("let third = 1/3\nmaster: level third\n");
    assert!(c.text.starts_with("let third = 1 / 3\n"), "{}", c.text);
    assert_eq!(ok(&c.text).text, c.text);
    assert_eq!(ok(&c.text).cid, c.cid);
    // the same program however the ratio is reached
    assert_eq!(ok("let third = 2/6\nmaster: level third\n").cid, c.cid);
    assert_eq!(ok("let third = 3 / 9 + 0\nmaster: level third\n").cid, c.cid);
    // a large denominator stays the fraction too (the decimal would be a rounded 0.000001000001000001)
    assert!(text("let tiny = 1/999999\nmaster: level tiny\n").starts_with("let tiny = 1 / 999999\n"));
    // a short decimal is a decimal, an integer an integer
    assert!(text("let half = 1/2\nmaster: level half\n").starts_with("let half = 0.5\n"));
    assert!(text("let n = 2*4\nmaster: delay 0.5\n").starts_with("let n = 8\n"));
    // a literal that is not exactly a ratio is not turned into one
    assert!(text("let y = 0.333333333333\nmaster: level y\n").starts_with("let y = 0.333333333333\n"));
    assert!(text("let y = 0.1234567\nmaster: level y\n").starts_with("let y = 0.1234567\n"));
    // and a value put in a def's body by the name reads back as the same number
    let d = ok("let third = 1/3\ndef lay() {\n  master: level third\n}\nlay()\n");
    assert_eq!(ok(&d.text).cid, d.cid);
    assert_eq!(ok(&d.text).controls[0].key, d.controls[0].key);
}

#[test]
fn l08_a_mix_value_is_exactly_what_was_written_or_an_error() {
    for src in ["mix { A level 1 / 3 }\n", "mix { A level 0.3333333 }\n", "mix { master level 2 / 3 }\n", "mix { B pan 1 / 7 - 1 }\n"] {
        let e = errors(src);
        assert!(e.iter().any(|d| d.message.contains("a mix value has at most six decimals")), "{src}: {:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert!(e[0].help.is_some(), "{src}");
    }
    // `1/3` and `0.333333` are no longer one program: one is an error
    assert!(check("mix { A level 0.333333 }\n").ok());
    // what has a decimal of six places is fine, and exactly itself
    for (src, want) in [("mix { A level 1 / 4 }\n", "mix {\n  A level 0.25\n}\n"), ("mix { A level 0.000001 }\n", "mix {\n  A level 0.000001\n}\n"), ("mix { A level 999999 / 1000000 }\n", "mix {\n  A level 0.999999\n}\n")] {
        assert_eq!(text(src), want, "{src}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// L-15: a line whose time lies beyond its section's end

fn said(src: &str, what: &str) -> bool {
    warnings(src).iter().any(|m| m.contains(what))
}

#[test]
fn l15_a_time_beyond_its_sections_end_is_a_warning_with_the_bar_and_the_length() {
    let sec = |line: &str| format!("section intro 4 bars {{\n  deck A: kick x ~ x ~\n  {line}\n}}\narrange intro\n");
    // the bar and the section's length are named
    for (line, what) in [
        ("at bar 9: master: level 0.5", "is due at bar 9 of section intro, which is only 4 bars long (bars 0 to 3)"),
        ("at bar 4: master: level 0.5", "is due at bar 4 of section intro, which is only 4 bars long"),
        ("at bar 4 beat 2: master: level 0.5", "is due at bar 4 beat 2 of section intro"),
        ("every 8 bars: master: delay 0.5", "first fires at bar 8 of section intro, which is only 4 bars long"),
        ("every 2 bars from 6: master: delay 0.5", "first fires at bar 6 of section intro"),
        ("master: level 0 -> 1 over 9 bars", "this ramp takes 9 bars and starts at bar 0 of section intro, which is only 4 bars long"),
        ("at bar 2: master: level 0 -> 1 over 3 bars", "this ramp takes 3 bars and starts at bar 2 of section intro"),
    ] {
        assert!(said(&sec(line), what), "{line}: {:?}", warnings(&sec(line)));
        // and it is a warning: the program still compiles
        assert!(check(&sec(line)).ok(), "{line}");
    }
    // inside the section there is nothing to say
    for line in ["at bar 3: master: level 0.5", "at bar 3 beat 3: master: level 0.5", "every 2 bars: master: delay 0.5", "every 1/2 bar: master: delay 0.5", "master: level 0 -> 1 over 4 bars", "at bar 1: master: level 0 -> 1 over 3 bars", "at bar 0 beat 2: master: level 0.5"] {
        assert!(!warnings(&sec(line)).iter().any(|m| m.contains("section intro")), "{line}: {:?}", warnings(&sec(line)));
    }
    // a later section counts from its own first bar: bar 3 of the second section is song bar 7
    let two = "section verse 4 bars {\n  deck A: kick x\n}\nsection chorus 4 bars {\n  deck A: kick x\n  at bar 3: master: level 0.5\n  at bar 5: master: level 0.6\n}\narrange verse chorus\n";
    assert!(!said(two, "at bar 3 of section chorus"));
    assert!(said(two, "is due at bar 5 of section chorus, which is only 4 bars long"), "{:?}", warnings(two));
    // the warning points at the time word
    let src = sec("at bar 9: master: level 0.5");
    let d = check(&src).diagnostics.into_iter().find(|d| d.message.contains("section intro")).unwrap();
    assert_eq!(&src[d.span.start..d.span.end], "at bar 9");
    // a word that names a section is not a number of bars: it is left alone, as the song's own lines outside every section are
    let named = "section drop 4 bars {\n  deck A: kick x\n}\nsection intro 4 bars {\n  deck A: kick x\n  after drop: master: level 0.5\n}\narrange intro drop\nat bar 20: master: level 0.5\n";
    assert!(!warnings(named).iter().any(|m| m.contains("which is only")), "{:?}", warnings(named));
    // in a stack, the section's own length is the one that counts
    let stack = "section long 8 bars {\n  deck A: kick x\n  at bar 6: master: level 0.5\n}\nsection short 4 bars {\n  deck B: bass c2\n  at bar 6: master: level 0.6\n}\narrange stack(long, short)\n";
    let w = warnings(stack);
    assert!(w.iter().any(|m| m.contains("section short, which is only 4 bars long")), "{w:?}");
    assert!(!w.iter().any(|m| m.contains("section long")), "{w:?}");
}

#[test]
fn l15_no_example_and_no_fixture_trips_it() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for dir in ["examples/btw", "examples", "crates/djlive/tests/fixtures", "docs/btw"] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "btw") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let c = check(&src);
            if c.compiled.is_none() {
                continue; // (an example that needs a host: it has its own test)
            }
            seen += 1;
            let w: Vec<String> = c.diagnostics.iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message.clone()).collect();
            assert!(!w.iter().any(|m| m.contains("which is only") || m.contains("it is still moving when the section ends")), "{}: {w:?}", p.display());
        }
    }
    assert!(seen >= 10, "{seen}");
}

// ---------------------------------------------------------------------------------------------------------------
// L-17: an lfo with no depth on a target in hertz

#[test]
fn l17_an_lfo_with_no_depth_on_a_cutoff_is_a_warning() {
    for line in ["master: filter lp lfo sine 1 bar center=3k over 4 bars", "master: filter hp lfo tri 2 bars center=400", "deck A: filter lp lfo sine 1 bar center=3k"] {
        let src = format!("deck A: kick x\n{line}\n");
        let w = warnings(&src);
        assert!(w.iter().any(|m| m.contains("this lfo has no `depth=`, so it swings 1 either side of its centre")), "{line}: {w:?}");
        assert!(check(&src).ok(), "it is a warning, not an error: {line}");
    }
    // a depth written is no warning, and neither is a 0..1 knob (a depth of 1 is its whole swing, and says so if it is too much)
    for line in ["master: filter lp lfo sine 1 bar center=3k depth=1k", "master: level lfo sine 2 bars center=0.5 depth=0.25", "master: reverb lfo sine 2 bars center=0.5 depth=0.5", "master: level lfo sine 1 bar center=0.5 depth=1k"] {
        let src = format!("deck A: kick x\n{line}\n");
        assert!(!warnings(&src).iter().any(|m| m.contains("no `depth=`")), "{line}: {:?}", warnings(&src));
    }
    assert!(!warnings("deck A: kick x\nmaster: level lfo sine 2 bars center=0.5\n").iter().any(|m| m.contains("no `depth=`")));
    // the warning is on the lfo, and the help says what to write
    let src = "deck A: kick x\nmaster: filter lp lfo sine 1 bar center=3k\n";
    let d = check(src).diagnostics.into_iter().find(|d| d.message.contains("no `depth=`")).unwrap();
    assert_eq!(&src[d.span.start..d.span.end], "lfo sine 1 bar center=3k");
    assert!(d.help.unwrap().contains("depth=1k"));
}

// ---------------------------------------------------------------------------------------------------------------
// L-18: the JSON-LD does not count the `e()` calls of a part every section shadows

fn euclids(src: &str) -> Vec<(u32, u32, u32, u32)> {
    ok(src).linked.euclids.iter().map(|(s, n)| (s.pulses, s.steps, s.rotation, *n)).collect()
}

#[test]
fn l18_a_part_that_every_section_shadows_plays_nowhere_and_is_not_counted() {
    // the song's kick is e(3,8); the only section has a kick of its own, so e(3,8) never plays
    let shadowed = "deck A: kick e(3,8) | snare e(2,8)\nsection s 4 bars {\n  deck A: kick x ~ x ~\n}\narrange s\n";
    assert_eq!(euclids(shadowed), [(2, 8, 0, 1)], "the snare plays, the kick is shadowed");
    // a section that does not shadow it lets it play (once: it is one part of the song)
    let plays = "deck A: kick e(3,8)\nsection s 4 bars {\n  deck A: kick x ~ x ~\n}\nsection t 4 bars {\n  deck A: snare x\n}\narrange s t\n";
    assert_eq!(euclids(plays), [(3, 8, 0, 1)]);
    // the same role on the other deck is not a shadow
    let other = "deck A: kick e(3,8)\nsection s 4 bars {\n  deck B: kick x ~ x ~\n}\narrange s\n";
    assert_eq!(euclids(other), [(3, 8, 0, 1)]);
    // the section's own part is counted for each time it is placed, and the song's part when no section shadows it
    let own = "section s 4 bars {\n  deck A: kick e(5,8)\n}\narrange s s\n";
    assert_eq!(euclids(own), [(5, 8, 0, 2)]);
    // a flat program has no section to shadow anything
    assert_eq!(euclids("deck A: kick e(3,8) | snare e(2,8)\n"), [(2, 8, 0, 1), (3, 8, 0, 1)]);
    // a shadowed part through a def and a repeat: still not counted
    let via = "def k() {\n  deck A: kick e(3,8)\n}\nk()\nsection s 4 bars {\n  deck A: kick x ~ x ~\n}\narrange s\n";
    assert!(euclids(via).is_empty());
}

// ---------------------------------------------------------------------------------------------------------------
// L-24: `then` over parts is a sequence, so a nested `then` flattens; the law is on the text AND the music follows the flat form

/// The hits of a one-part deck, bar by bar, as the step of each hit (`x` on the first step is 0, `~` is nothing): `x` `~` per bar.
fn bars_of(src: &str, bars: usize) -> String {
    let c = ok(src);
    let part = &c.decks[&Deck::A].parts[0];
    let n = part.cycles.len().max(1);
    (0..bars).map(|b| if part.cycles[b % n].is_empty() { '~' } else { 'x' }).collect()
}

#[test]
fn l24_then_over_parts_is_a_sequence_and_a_nested_then_is_the_flat_one() {
    // the finding's program: an operand that alternates
    let nested = "deck A: then(then(kick <x ~>, kick x), kick ~)\n";
    let flat = "deck A: then(kick <x ~>, kick x, kick ~)\n";
    let right = "deck A: then(kick <x ~>, then(kick x, kick ~))\n";
    // one text, one CID, one music: the law is on the text and the music is the music of that text
    assert_eq!(ok(nested).text, "deck A: kick <<x ~> x ~>\n");
    assert_eq!(ok(nested).cid, ok(flat).cid);
    assert_eq!(ok(right).cid, ok(flat).cid);
    // the sequence reading: bar 0 the first operand (its own first cycle), bar 1 the second, bar 2 the third, bar 3 the first again,
    // now at its second cycle: `x x ~` then `~ x ~`, round
    assert_eq!(bars_of(nested, 12), "xx~~x~xx~~x~");
    // an alternation WITHIN an alternation is another thing and is written as one: `<<<x ~> x> ~>` advances the inner ones once each
    // time their turn comes, which `then(then(..))` never means
    let literal = "deck A: kick <<<x ~> x> ~>\n";
    assert_ne!(ok(literal).cid, ok(nested).cid);
    assert_ne!(bars_of(literal, 12), bars_of(nested, 12));
    // an operand that is one `then` of its own is a sequence in the sequence, not a nested alternation
    let inner = "deck A: then(kick x, then(kick <x ~>, kick ~))\n";
    assert_eq!(ok(inner).text, "deck A: kick <x <x ~> ~>\n");
    assert_eq!(bars_of(inner, 12), "xx~x~~xx~x~~");
}

// ---------------------------------------------------------------------------------------------------------------
// R-24: the number of tags is capped

#[test]
fn r24_a_program_has_at_most_sixty_four_tags_and_the_rest_are_not_looked_up() {
    use djlive::compile::check_with;
    use djlive::env::Env;
    use djlive::linked::{self, Ontology, StaticOntology};
    let onto = StaticOntology::from_json(r#"{"version":"1","verified":true,"terms":[{"iri":"https://ontology.bitwrap.io/genre/techno","kind":"genre","local":"techno","label":"techno"},{"iri":"https://ontology.bitwrap.io/instrument/acid","kind":"instrument","local":"acid","label":"acid"}]}"#).unwrap();
    let many = |n: usize| (0..n).map(|i| format!("acid{i}")).collect::<Vec<_>>().join(", ");
    // 64 is fine (the unknown ones are said, one each: the vocabulary has no `acid0`); 65 is said once, at the first one over
    let (_, ds) = linked::check_headers(&format!("#! tags: {}\n", many(64)), &onto, Severity::Error);
    assert_eq!(ds.len(), 64);
    assert!(ds.iter().all(|d| d.message.starts_with("unknown tag")), "{:?}", ds.iter().map(|d| &d.message).collect::<Vec<_>>());
    let src = format!("#! tags: {}\n", many(65));
    let (_, ds) = linked::check_headers(&src, &onto, Severity::Error);
    assert_eq!(ds.len(), 65, "64 lookups and one word on the 65th");
    let d = ds.last().unwrap();
    assert!(d.message.starts_with("a program has at most 64 tags: this is the 65th"), "{}", d.message);
    assert_eq!(&src[d.span.start..d.span.end], "acid64");
    assert!(d.is_error());
    // several lines are one set
    let src = format!("#! tags: {}\n#! tags: {}\n", many(40), many(40));
    let (_, ds) = linked::check_headers(&src, &onto, Severity::Error);
    assert!(ds.iter().any(|d| d.message.contains("at most 64 tags: this is the 65th")), "{:?}", ds.len());
    // twenty thousand are not looked up: it was fifteen seconds of `check` (debug) and is now a few lookups
    let t = std::time::Instant::now();
    let src = format!("#! tags: {}\ndeck A: kick x\n", many(20_000));
    let c = check_with(&src, &Env { ontology: Some(&onto), ..Env::default() });
    assert!(t.elapsed().as_secs_f64() < 2.0, "{:?}", t.elapsed());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("at most 64 tags")));
    assert!(c.ok(), "a warning from check, as every header problem is");
    // the other door: links made of a list
    let tags: Vec<String> = (0..65).map(|i| format!("acid{i}")).collect();
    let e = linked::canonical_links(&onto, None, &tags).unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("at most 64 tags"), "{}", e[0].message);
    assert!(linked::canonical_links(&onto, None, &vec!["acid".to_string(); 64]).is_ok());
    let _ = Ontology::terms(&onto);
}

// ---------------------------------------------------------------------------------------------------------------
// R-20: the ontology's names for a rhythm are said only to a program linked to the ontology

#[test]
fn r20_an_old_program_with_e_4_16_has_no_ontology_warning() {
    for src in ["deck A: kick e(4,16)\n", "deck A: snare e(3,8)\n", "tempo 128\ndeck A: kick e(4,16) | hihat e(5,8)\n#! genre: techno\n"] {
        // (the last has its header line after the program has begun: a comment, so not linked)
        assert!(!warnings(src).iter().any(|m| m.contains("the ontology's")), "{src}: {:?}", warnings(src));
    }
    // linked: a note, as it was
    assert!(warnings("#! genre: techno\ndeck A: kick e(4,16)\n").iter().any(|m| m.contains("the ontology's kick rhythm of")));
    assert!(warnings("#! tags: acid\ndeck A: snare e(3,8)\n").iter().any(|m| m.contains("the ontology's house snare rhythm")));
    // and the program is the same program, so its CID does not move
    assert_eq!(ok("deck A: kick e(4,16)\n").cid, ok("#! genre: techno\ndeck A: kick e(4,16)\n").cid);
}

#[test]
fn l15_the_copies_of_a_repeat_and_a_section_placed_twice_say_it_once() {
    let src = "section s 4 bars {\n  deck A: kick x\n  repeat 8 {\n    at bar $i: master: level 0.5\n  }\n  at bar 9: master: level 0.6\n}\narrange s s\n";
    let w: Vec<String> = warnings(src).into_iter().filter(|m| m.contains("which is only")).collect();
    // `at bar $i` is one word in the text (copies 4 to 7 lie past the end); `at bar 9` is another
    assert_eq!(w.len(), 2, "{w:?}");
}
