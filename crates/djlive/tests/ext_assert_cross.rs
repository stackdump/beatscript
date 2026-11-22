//! F12 meets F3, F10, F5, F6, F7 and F2: `assert` through the tempo map (a curve, a section's own tempo, a stack), through a
//! stack's section names (`in short` when `short` plays inside `long+short`), through the key a stack settles, through the
//! periods of polymeter and chance, and through the lets a program or an import brings. The numbers of the length are worked out
//! by hand from the closed form of F3 (`60 L ln(b / a) / (b - a)` for a linear ramp of `L` beats from `a` to `b`).

mod common;

use common::{reformat, Rng};
use djlive::assertion::Status;
use djlive::compile::{check, check_report, check_with, Compiled};
use djlive::diag::{render_all, Diagnostic};
use djlive::env::{Env, StaticSnippets};

fn report(src: &str) -> Compiled {
    let c = check_report(src, &Env::none());
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

/// Seconds of `beats` beats at a constant `bpm`.
fn hold(bpm: f64, beats: f64) -> f64 {
    60.0 * beats / bpm
}

/// Seconds of a linear ramp `a -> b` over `beats` beats: the closed form of F3.
fn ramp(a: f64, b: f64, beats: f64) -> f64 {
    60.0 * beats * (b / a).ln() / (b - a)
}

/// The status of the statics in `src` followed by `asserts`, each with its message.
fn judged(src: &str) -> Vec<(Status, String)> {
    report(src).assertions.iter().map(|a| (a.status, a.line())).collect()
}

/// `assert length == <secs> ± 1ms` for exactly this many seconds (to the tenth of a millisecond), and the same a hair away.
fn length_is(secs: f64) -> String {
    format!("assert length == {secs:.4}s ± 1ms\nassert length == {:.4}s ± 1ms\n", secs + 0.01)
}

// ---- the length through the tempo map

const TWO_SECTIONS: &str = "section verse 4 bars {\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  deck A: kick x x x x\n}\narrange verse chorus\n";

#[test]
fn the_length_of_a_constant_tempo_is_what_it_was() {
    // 8 bars at 126 BPM: 32 beats of 60 / 126 s
    let secs = hold(126.0, 32.0);
    let r = judged(&format!("tempo 126\n{TWO_SECTIONS}{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    assert_eq!(r[1].0, Status::Fail, "{}", r[1].1);
    // no word of a curve in what it says
    assert!(!r[0].1.contains("curve"), "{}", r[0].1);
    // a program that says no tempo is 120
    let r = judged(&format!("{TWO_SECTIONS}{}", length_is(hold(120.0, 32.0))));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
}

#[test]
fn the_length_follows_a_tempo_ramp_by_the_closed_form() {
    // `tempo 100 -> 140 over 6 bars`: 24 beats of ramp, then the other 8 beats of the 8 bars at 140
    let secs = ramp(100.0, 140.0, 24.0) + hold(140.0, 8.0);
    assert!((secs - 15.5416).abs() < 1e-3, "worked out by hand: {secs}");
    let src = format!("tempo 100 -> 140 over 6 bars\n{TWO_SECTIONS}{}", length_is(secs));
    let r = judged(&src);
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    assert_eq!(r[0].1, format!("assert length == {secs:.4}s ± 0.001s: the program is 15.542s (through the tempo curve)"));
    assert_eq!(r[1].0, Status::Fail, "{}", r[1].1);
    // what it was before F3 (the program's first tempo for every bar) is not the length any more
    let old = hold(100.0, 32.0);
    assert!(judged(&format!("tempo 100 -> 140 over 6 bars\n{TWO_SECTIONS}assert length == {old:.3}s ± 1s\n"))[0].0 == Status::Fail);
    // and `check`'s own line agrees with the assertion to the second it prints
    let c = report(&src);
    assert!((c.length_seconds().unwrap() - secs).abs() < 1e-9);
    assert_eq!(c.length_report().as_deref(), Some("length: 16s with the tempo curve, 8 bars"));
    // a program that is refused says the number a person would put there
    let e = errors(&format!("tempo 100 -> 140 over 6 bars\n{TWO_SECTIONS}assert length == 3m\n"));
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("the program is 15.542s (through the tempo curve)"), "{}", e[0].message);
}

#[test]
fn a_timed_ramp_that_falls_and_a_hold_before_it_are_added_piece_by_piece() {
    // 2 bars at 90, the ramp 90 -> 150 over 4 bars from bar 2, then 150 to bar 10: 8 + 16 + 16 beats
    let secs = hold(90.0, 8.0) + ramp(90.0, 150.0, 16.0) + hold(150.0, 16.0);
    let src = format!("tempo 90\nat bar 2: tempo 90 -> 150 over 4 bars\ndeck A: kick x\nend at bar 10\n{}", length_is(secs));
    let r = judged(&src);
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    assert_eq!(r[1].0, Status::Fail);
    // the same song brought down again: the ramp's closed form is the same with a above b
    let secs = hold(150.0, 8.0) + ramp(150.0, 90.0, 16.0) + hold(90.0, 16.0);
    let r = judged(&format!("tempo 150\nat bar 2: tempo 150 -> 90 over 4 bars\ndeck A: kick x\nend at bar 10\n{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    // `end at bar N` decides how many bars of the curve count, a bar inside a ramp too
    // (the tempo is linear in beats, so 4 beats into the ramp it is 90 + 60 * 4 / 16 = 105, and those 4 beats are a ramp 90 -> 105)
    let secs = hold(90.0, 8.0) + ramp(90.0, 105.0, 4.0);
    let r = judged(&format!("tempo 90\nat bar 2: tempo 90 -> 150 over 4 bars\ndeck A: kick x\nend at bar 3\n{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
}

#[test]
fn a_sections_own_tempo_and_a_sections_own_ramp_set_the_length() {
    // verse at 120 for 4 bars (16 beats), chorus at 150 for 4 bars
    let secs = hold(120.0, 16.0) + hold(150.0, 16.0);
    let src = "section verse 4 bars {\n  tempo 120\n  deck A: kick x x x x\n}\nsection chorus 4 bars {\n  tempo 150\n  deck A: kick x x x x\n}\narrange verse chorus\n";
    let r = judged(&format!("{src}{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    assert!(r[0].1.contains("14.4s"), "{}", r[0].1);
    // a ramp in a section: from where the section begins
    let secs = hold(120.0, 16.0) + ramp(120.0, 150.0, 16.0);
    let src = src.replace("tempo 150", "tempo 120 -> 150 over 4 bars");
    let r = judged(&format!("{src}{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    // a section that sets none keeps the tempo before it
    let src = "section verse 4 bars {\n  tempo 100 -> 140 over 2 bars\n  deck A: kick x\n}\nsection chorus 4 bars {\n  deck A: kick x\n}\narrange verse chorus\n";
    let secs = ramp(100.0, 140.0, 8.0) + hold(140.0, 8.0) + hold(140.0, 16.0);
    let r = judged(&format!("{src}{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
}

#[test]
fn a_stack_is_as_long_as_the_longer_section_through_the_curve() {
    // F10: stack(short, long) is 8 bars, the longer; the ramp spans the first 6 of them
    let secs = ramp(100.0, 140.0, 24.0) + hold(140.0, 8.0);
    let stack = "tempo 100 -> 140 over 6 bars\nsection short 4 bars {\n  deck A: kick x\n}\nsection long 8 bars {\n  deck B: kick x\n}\n";
    let r = judged(&format!("{stack}arrange stack(short, long)\n{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    // it is the length of the longer alone, and `then` is the sum
    assert_eq!(judged(&format!("{stack}arrange long\n{}", length_is(secs)))[0].0, Status::Pass);
    let secs12 = ramp(100.0, 140.0, 24.0) + hold(140.0, 24.0);
    let r = judged(&format!("{stack}arrange then(short, long)\n{}", length_is(secs12)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    // a stack inside a longer song: intro, then the stack, then the coda
    let secs = ramp(100.0, 140.0, 24.0) + hold(140.0, 4.0 * 4.0 + 8.0 * 4.0 - 24.0 + 16.0);
    let r = judged(&format!("{stack}section intro 4 bars {{\n  deck A: kick x\n}}\narrange intro stack(short, long) intro\n{}", length_is(secs)));
    assert_eq!(r[0].0, Status::Pass, "{}", r[0].1);
    assert!(r[0].1.contains("(through the tempo curve)"), "{}", r[0].1);
}

#[test]
fn a_looping_song_has_no_length_but_the_lap_is_said_through_the_curve() {
    let secs = ramp(100.0, 140.0, 24.0) + hold(140.0, 8.0);
    let r = judged(&format!("tempo 100 -> 140 over 6 bars\n{TWO_SECTIONS}loop\nassert length == 3m\n"));
    assert_eq!(r[0].0, Status::Fail);
    assert_eq!(r[0].1, "assert length == 3m: the program loops, so it has no length (one lap is 15.542s)");
    assert!((secs - 15.5416).abs() < 1e-3);
    // `check` still says a lap's length, and `assert ends` is false
    let c = report(&format!("tempo 100 -> 140 over 6 bars\n{TWO_SECTIONS}loop\nassert ends\n"));
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert!((c.length_seconds().unwrap() - secs).abs() < 1e-9);
}

// ---- section names and keys through a stack

#[test]
fn a_section_in_a_stack_is_named_by_in_and_its_windows_are_the_stretches_it_plays_in() {
    let src = "section short 4 bars {\n  deck A: kick x\n}\nsection long 8 bars {\n  deck B: kick x\n}\narrange stack(short, long)\n";
    let c = report(&format!("{src}assert level A > 0 in short\nassert level B > 0 in long\n"));
    // the stack is two stretches: `long+short` for 4 bars, then `long` alone for 4
    assert_eq!(c.segments.iter().map(|s| (s.name.as_str(), s.start, s.bars)).collect::<Vec<_>>(), [("long+short", 0, 4), ("long", 4, 4)]);
    let windows: Vec<Vec<(u32, u32)>> = c.assertions.iter().map(|a| a.level.as_ref().unwrap().windows.clone()).collect();
    assert_eq!(windows[0], [(0, 4)], "`short` plays in the first stretch only");
    assert_eq!(windows[1], [(0, 4), (4, 8)], "`long` plays in both");
    // (`long+short` names the stretch, not a section: the sections are what `in` takes)
    assert!(check(&format!("{src}assert level A < B in long+short\n")).compiled.is_none(), "a stretch's name is not a section name: the `+` is not part of one");
    // a name that is no section lists the sections, one at a time, and says which was meant
    let e = errors(&format!("{src}assert level A > 0 in shrt\n"));
    assert_eq!(e.len(), 1);
    assert!(e[0].help.as_deref().unwrap().contains("did you mean `short`"), "{:?}", e[0].help);
    assert!(e[0].help.as_deref().unwrap().contains("the sections are long, short"), "{:?}", e[0].help);
    // a section that is more than once on the timeline has every stretch
    let song = "section short 4 bars {\n  deck A: kick x\n}\nsection long 8 bars {\n  deck B: kick x\n}\nsection coda 2 bars {\n  deck A: kick x\n}\narrange then(coda, stack(short, long), coda)\n";
    let c = report(&format!("{song}assert level A > 0 in coda\n"));
    assert_eq!(c.assertions[0].level.as_ref().unwrap().windows, [(0, 2), (10, 12)]);
    assert_eq!(c.assertions[0].status, Status::Pending);
}

#[test]
fn a_stacked_sections_key_is_the_stretchs_and_an_unkeyed_ones_is_the_programs() {
    let src = "key am\nsection groove 4 bars {\n  key dm\n  deck A: kick x\n}\nsection wash 8 bars {\n  deck B: bass a2\n}\nsection solo 4 bars {\n  deck A: kick x\n}\n";
    // groove sets dm; wash is stacked with it for 4 bars and follows the stretch, then plays alone: it answers the first stretch's key
    let r = judged(&format!("{src}arrange stack(groove, wash) solo\nassert key == dm in groove\nassert key == dm in wash\nassert key == am in wash\nassert key == am in solo\nassert key != dm in solo\nassert key == am\n"));
    let want = [Status::Pass, Status::Pass, Status::Fail, Status::Pass, Status::Pass, Status::Pass];
    for ((got, msg), want) in r.iter().zip(want) {
        assert_eq!(*got, want, "{msg}");
    }
    assert_eq!(r[2].1, "assert key == am in wash: the key in wash is dm");
    // a section with no key of its own, on its own timeline, has the program's
    let r = judged(&format!("{src}arrange wash solo\nassert key == am in wash\nassert key == am in solo\n"));
    assert!(r.iter().all(|(s, _)| *s == Status::Pass), "{r:?}");
    // a program with no key at all is C major, and says so
    let r = judged("section drop 4 bars {\n  deck A: kick x\n}\narrange drop\nassert key == c in drop\n");
    assert_eq!(r[0], (Status::Pass, "assert key == c in drop: the key in drop is c".to_string()));
}

#[test]
fn a_key_clash_of_a_stack_is_the_error_and_the_assertion_is_not_judged() {
    let src = "section one 4 bars {\n  key dm\n  deck A: kick x\n}\nsection two 4 bars {\n  key em\n  deck B: kick x\n}\narrange stack(one, two)\n";
    // with a false assertion, with a true one, and with none: one error, the clash
    for asserts in ["", "assert key == dm in one\n", "assert key == am in one\n", "assert key == dm in two\nassert ends\n"] {
        let e = errors(&format!("{src}{asserts}"));
        assert_eq!(e.len(), 1, "{asserts:?}: {e:?}");
        assert!(e[0].message.contains("play at the same time, in different keys"), "{asserts:?}: {}", e[0].message);
    }
    // the report entry (`btw test`) is refused in the same words
    let c = check_report(&format!("{src}assert key == dm in one\n"), &Env::none());
    assert!(c.compiled.is_none());
    assert_eq!(c.diagnostics.iter().filter(|d| d.is_error()).count(), 1);
    // the same sections one after the other have no clash: each has its own key
    let c = report(&format!("{}assert key == dm in one\nassert key == em in two\n", src.replace("stack(one, two)", "then(one, two)")));
    assert!(c.assertions.iter().all(|a| a.status == Status::Pass));
}

// ---- periods through polymeter and chance

#[test]
fn the_period_agrees_with_the_period_note_of_polymeter() {
    let polymeter = "deck A: kick x ~ x ~ | hihat/12 x x ~ x\n";
    let c = report(&format!("{polymeter}assert period == 3\nassert period == 4\n"));
    // `check` says `period: 3 bars = 48 steps (hihat/12 against the bar)`; the assertion says the same number
    let note = c.warnings.iter().map(|w| w.message.as_str()).find(|m| m.starts_with("period:")).expect("the period note");
    assert_eq!(note, "period: 3 bars = 48 steps (hihat/12 against the bar)");
    assert_eq!(c.assertions[0].status, Status::Pass);
    assert_eq!(c.assertions[0].line(), "assert period == 3: the program's period is 3 bars (hihat/12 against the bar)");
    assert_eq!(c.assertions[1].status, Status::Fail);
    // meter's own report is the source of both
    let periods = djlive::meter::program_period(&c);
    assert_eq!(periods.len(), 1);
    assert_eq!((periods[0].1.bars, periods[0].1.steps), (3, 48));
    // two parts of their own cycle come round together after their least common multiple: 12 and 20 steps -> 15 bars? (60 / 16 -> 15)
    let c = report("deck A: hihat/12 x x ~ x | snare/20 x ~ x ~\nassert period == 15\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    let note = c.warnings.iter().map(|w| w.message.as_str()).find(|m| m.starts_with("period:")).unwrap();
    assert!(note.starts_with("period: 15 bars = 240 steps"), "{note}");
}

#[test]
fn the_period_of_a_song_is_the_lcm_of_its_sections_and_chance_unrolls_over_the_sections_bars() {
    let src = "section poly 4 bars {\n  deck A: hihat/12 x x ~ x\n}\nsection dice 4 bars {\n  deck A: kick x?0.5 ~ x ~\n}\nsection plain 4 bars {\n  deck A: kick x ~ x ~\n}\narrange poly dice plain\n";
    let c = report(&format!("{src}assert period == 12\nassert period == 3\n"));
    // meter's report by section: 3 bars of polymeter, the dice of the section (no period: what is left of it is 1 bar, whatever the
    // 4 bars it is placed in), 1 plain; the program's period is of the rest, and it says it has none because of the chance (L-10)
    let by: Vec<(String, u64)> = djlive::meter::program_period(&c).into_iter().map(|(n, p)| (n, p.bars)).collect();
    assert_eq!(by, [("poly".to_string(), 3), ("dice".to_string(), 1), ("plain".to_string(), 1)]);
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert_eq!(c.assertions[1].status, Status::Fail);
    assert_eq!(c.assertions[0].line(), "assert period == 12: the program has no period: kick on deck A has chance, which differs every bar (the rest comes round in 3 bars (hihat/12 against the bar))");
    // without the dice it is the lcm of the sections' periods
    let c = report(&format!("{}assert period == 3\n", src.replace("x?0.5", "x")));
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    assert_eq!(c.assertions[0].line(), "assert period == 3: the program's period is 3 bars (hihat/12 against the bar)");
    // the period note is only about the parts with a cycle of their own, section by section
    let notes: Vec<&str> = c.warnings.iter().map(|w| w.message.as_str()).filter(|m| m.starts_with("period:")).collect();
    assert_eq!(notes, ["period: 3 bars = 48 steps (hihat/12 against the bar), in section poly"]);
    // chance on a program with no sections unrolls over 16 bars (`chance::DEFAULT_BARS`), and over a section's own bars in a
    // section: that is the net's length, not a period, so the program has none wherever it is placed (L-10, L-21)
    for src in ["deck A: kick x?0.5 ~ x ~\n", "section dice 4 bars {\n  deck A: kick x?0.5 ~ x ~\n}\narrange dice\n", "section dice 200 bars {\n  deck A: kick x?0.5 ~ x ~\n}\narrange dice\n"] {
        for n in [16, 4, 64, 1] {
            let c = report(&format!("{src}assert period == {n}\n"));
            assert_eq!(c.assertions[0].status, Status::Fail, "{src}");
            assert!(c.assertions[0].message.starts_with("the program has no period: kick on deck A has chance"), "{}", c.assertions[0].message);
        }
    }
    // a stack's period is of the stretch the two sections play in
    let c = report("section poly 4 bars {\n  deck A: hihat/12 x x ~ x\n}\nsection dice 4 bars {\n  deck B: kick x ~ x ~\n}\narrange stack(poly, dice)\nassert period == 3\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    // (and with the dice in the stack it has none, the rest still coming round in 3)
    let c = report("section poly 4 bars {\n  deck A: hihat/12 x x ~ x\n}\nsection dice 4 bars {\n  deck B: kick x?0.5 ~ x ~\n}\narrange stack(poly, dice)\nassert period == 3\n");
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert!(c.assertions[0].message.ends_with("(the rest comes round in 3 bars (hihat/12 against the bar))"), "{}", c.assertions[0].message);
}

// ---- lets in an assertion, a program's and an import's

#[test]
fn a_let_is_folded_into_an_assertion_arithmetic_and_all() {
    let c = report("let loud = 2\nlet n = 2 + 1\ndeck A: kick x ~ x ~ | hihat/12 x x ~ x\nassert level A < loud * 3 * B\nassert period == n\nassert period == n * 2 - 3\nassert period == n + 1\n");
    let texts: Vec<&str> = c.assertions.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["assert level A < 6 * B", "assert period == 3", "assert period == 3", "assert period == 4"]);
    let want = [Status::Pending, Status::Pass, Status::Pass, Status::Fail];
    for (a, w) in c.assertions.iter().zip(want) {
        assert_eq!(a.status, w, "{}", a.line());
    }
    // the folded text is the canonical one: another way of writing the same let is the same program
    let d = report("let loud = 6\nlet n = 3\ndeck A: kick x ~ x ~ | hihat/12 x x ~ x\nassert level A < loud * B\nassert period == n\nassert period == n\nassert period == n + 1\n");
    assert_eq!(c.text.lines().filter(|l| l.starts_with("assert")).collect::<Vec<_>>(), d.text.lines().filter(|l| l.starts_with("assert")).collect::<Vec<_>>());
}

#[test]
fn a_let_an_import_brings_is_a_value_in_an_assertion() {
    let lib = "let loud = 2\nlet n = 3\n";
    let libc = check_with(lib, &Env::none()).compiled.expect("a snippet of lets");
    let mut snips = StaticSnippets::default();
    snips.items.push(("levels".into(), libc.cid.clone(), libc.text.clone()));
    let env = Env { snippets: Some(&snips), ..Env::none() };
    let src = "import \"levels\"\ndeck A: kick x ~ x ~ | hihat/12 x x ~ x\nassert level A < loud * B\nassert period == n\n";
    let c = check_with(src, &env);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", render_all(&c.diagnostics, src)));
    let texts: Vec<&str> = comp.assertions.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["assert level A < 2 * B", "assert period == 3"]);
    assert_eq!(comp.assertions[1].status, Status::Pass, "{}", comp.assertions[1].line());
    // the text keeps the import and the assertion in its folded form: reading it back gives the same program
    let again = check_with(&comp.text, &env).compiled.expect("the canonical text reads back");
    assert_eq!(again.cid, comp.cid);
}

// ---- everything at once

/// A curve, a stack of unequal sections, polymeter, chance, a duck and a mix, a section key, a let, and every kind of assertion.
const ALL: &str = "tempo 100 -> 140 over 6 bars
key am
let two = 1 + 1
section groove 4 bars {
  deck A: kick x?0.5 ~ x ~ | hihat/12 x x ~ x
  deck B: bass <a1 c2> ~ a1 ~
  duck B under A.kick 6db
}
section wash 8 bars {
  key dm
  deck B: melody <d4 f4> ~ d4 ~
  mix { B level 0.6 }
}
arrange stack(groove, wash)
assert ends
assert length < 1m
assert length == 15.5416s ± 2ms
assert key == dm in wash
assert key == dm in groove
assert level A < two * B in groove
assert level master >= 0.5 * (A + B) in wash
assert level B > 0
";

#[test]
fn a_program_with_every_extension_and_every_assertion_is_a_fixed_point_and_survives_a_reformat() {
    let c = report(ALL);
    // statics hold (their numbers are the other tests' work), the levels wait for `btw test`
    let st: Vec<(Status, String)> = c.assertions.iter().map(|a| (a.status, a.line())).collect();
    for (status, line) in &st[..5] {
        assert_eq!(*status, Status::Pass, "{line}");
    }
    assert!(st[5..].iter().all(|(s, _)| *s == Status::Pending), "{st:?}");
    // the period is meter's: the 3 bars of the polymeter against the 2 of the alternation (the chance is no period at all: it
    // differs every bar, so `assert period` of this program says so, and the 6 bars are what the rest comes round in)
    let by: Vec<(String, u64)> = djlive::meter::program_period(&c).into_iter().map(|(n, p)| (n, p.bars)).collect();
    assert_eq!(by, [("groove+wash".to_string(), 6)], "{:?}", c.segments.iter().map(|s| (s.name.as_str(), s.start, s.bars, s.chain)).collect::<Vec<_>>());
    // the canonical text reads back as itself, with the assertions in their folded form and in order
    let again = report(&c.text);
    assert_eq!(again.text, c.text, "not a fixed point:\n{}", c.text);
    assert_eq!(again.cid, c.cid);
    let tail: Vec<&str> = c.text.lines().filter(|l| l.starts_with("assert")).collect();
    assert_eq!(tail[0], "assert ends");
    assert_eq!(tail[2], "assert length == 15.5416s ± 0.002s");
    assert_eq!(tail[5], "assert level A < 2 * B in groove");
    let mut rng = Rng(0xa55e_47f1_0f12_0001);
    for round in 0..40 {
        let text = reformat(ALL, &mut rng);
        let r = check_report(&text, &Env::none());
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", render_all(&r.diagnostics, &text)));
        assert_eq!(got.cid, c.cid, "round {round} forked:\n{text}");
    }
    // an edit of an assertion is another program; an edit that makes a static false is refused with the number
    let edited = ALL.replace("assert length < 1m", "assert key == cm in groove");
    assert_ne!(report(&ALL.replace("assert level B > 0", "assert level B > 1")).cid, c.cid);
    let e = errors(&edited);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].message, "assert key == cm in groove: the key in groove is dm");
    // the same program without its dice has a period, 6 bars: and with them it says it has none (L-10)
    let plain = ALL.replace("x?0.5", "x");
    let p = report(&format!("{plain}assert period == 6\n"));
    assert_eq!(p.assertions.last().unwrap().status, Status::Pass, "{}", p.assertions.last().unwrap().line());
    let e = errors(&format!("{plain}assert period == 13\n"));
    assert_eq!(e[0].message, "assert period == 13: the program's period is 6 bars (hihat/12 against the bar)");
    let e = errors(&format!("{ALL}assert period == 6\n"));
    assert!(e[0].message.starts_with("assert period == 6: the program has no period: kick on deck A has chance, which differs every bar (the rest comes round in 6 bars"), "{}", e[0].message);
}
