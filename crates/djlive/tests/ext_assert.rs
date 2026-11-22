//! F12: assertions (`assert ends | period | length | key | level`), docs/btw/ext-assert.md. The static ones are proved by
//! `check` (a false one refuses the program and says the actual value), the level ones are pending until `btw test`
//! (djhost::btw_test measures them; its tests and djctl's are there). Here: canonical forms, each statement passing and
//! failing with the right message, errors with their place and a did-you-mean, the laws (canonical fixed point,
//! reformatting keeps the CID, an edit changes it, a length is what the tempo and the bars make it) and the judging of a
//! level assertion from measured numbers.

mod common;

use common::{reformat, Rng};
use djlive::assertion::{clock_text, Status};
use djlive::ast::StmtKind;
use djlive::compile::{check, check_report, check_with, Compiled};
use djlive::diag::{line_col, render_all, Diagnostic, Severity};
use djlive::env::{Env, StaticSnippets};
use djlive::rat::Rat;

// ---- helpers

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", render_all(&c.diagnostics, src)))
}

fn report(src: &str) -> Compiled {
    let c = check_report(src, &Env::none());
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn one_error(src: &str) -> Diagnostic {
    let mut e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    e.remove(0)
}

/// (line, column) of an error.
fn at(src: &str, d: &Diagnostic) -> (usize, usize) {
    line_col(src, d.span.start)
}

/// The canonical text of one `assert` statement (read, not checked: what it names need not be there).
fn canon(assert: &str) -> String {
    let p = djlive::parse::parse(&format!("{assert}\n"));
    assert!(p.ok(), "`{assert}` does not read:\n{}", render_all(&p.diagnostics, assert));
    djlive::print::canonical(&p.program).trim_end().to_string()
}

/// Two bars of intro, four of climax in D minor, at 120 BPM: 24 s, 12 bars.
const SONG: &str = "tempo 120
key am
section intro 8 bars {
  deck A: kick x ~ x ~
}
section climax 16 bars {
  key dm
  deck A: kick x x x x | hihat x*8
  deck B: bass d2 ~ ~ ~
}
arrange intro climax
";

fn song(asserts: &str) -> String {
    format!("{SONG}{asserts}")
}

/// What the statement `assert` says of `SONG`: its status and its message.
fn judged(asserts: &str) -> Vec<(Status, String)> {
    report(&song(asserts)).assertions.iter().map(|a| (a.status, a.line())).collect()
}

// ---- the canonical form

#[test]
fn every_assertion_prints_in_its_canonical_form() {
    for (written, want) in [
        ("assert ends", "assert ends"),
        ("assert   ends   # a comment", "assert ends"),
        ("assert period==3", "assert period == 3"),
        ("assert period ( drop ) >= 8", "assert period(drop) >= 8"),
        ("assert period(evolve) != 4 * 2", "assert period(evolve) != 8"),
        // clocks print as minutes and seconds
        ("assert length == 90s", "assert length == 1m30s"),
        ("assert length==3m30s±5s", "assert length == 3m30s ± 5s"),
        ("assert length == 3m 30s ± 5s", "assert length == 3m30s ± 5s"),
        ("assert length == 210s ± 0.5s", "assert length == 3m30s ± 0.5s"),
        ("assert length == 3m ± 500ms", "assert length == 3m ± 0.5s"),
        ("assert length < 4m", "assert length < 4m"),
        ("assert length >= 90s", "assert length >= 1m30s"),
        ("assert length <= 240s", "assert length <= 4m"),
        ("assert length > 0.5s", "assert length > 0.5s"),
        ("assert length != 0s", "assert length != 0s"),
        ("assert length == 3m30.5s", "assert length == 3m30.5s"),
        ("assert length == 3600s", "assert length == 60m"),
        // levels: one space round an operator, constants folded, decimals kept
        ("assert level A<2*B in drop", "assert level A < 2 * B in drop"),
        ("assert level A < B", "assert level A < B"),
        ("assert level   master>=A+B", "assert level master >= A + B"),
        ("assert level (1 + 1) * A <= B", "assert level 2 * A <= B"),
        ("assert level A > 0.05", "assert level A > 0.05"),
        ("assert level A > 5%", "assert level A > 0.05"),
        ("assert level A > 1 / 20", "assert level A > 0.05"),
        ("assert level A > 1 / 3", "assert level A > 1 / 3"),
        ("assert level A * (B + 1) < 3", "assert level A * (B + 1) < 3"),
        ("assert level (A + B) * 2 < master", "assert level (A + B) * 2 < master"),
        ("assert level A - (B - master) < 0", "assert level A - (B - master) < 0"),
        ("assert level min(A, B) < max(A, master)", "assert level min(A, B) < max(A, master)"),
        ("assert level min(2, 3) * A == B", "assert level 2 * A == B"),
        ("assert level -A < 0", "assert level -A < 0"),
        ("assert level A == 0", "assert level A == 0"),
        ("assert level A != B", "assert level A != B"),
        // keys: lower case, flats as sharps
        ("assert key == Dm", "assert key == dm"),
        ("assert key == Bbm", "assert key == a#m"),
        ("assert key == D:Dorian", "assert key == d:dorian"),
        ("assert key!=c", "assert key != c"),
    ] {
        assert_eq!(canon(written), want, "`{written}`");
    }
}

#[test]
fn the_canonical_text_is_a_fixed_point_and_the_cid_is_of_it() {
    let src = song("assert ends\nassert period == 1\nassert length == 24s\nassert key == dm in climax\nassert level A < 2 * B in climax\nassert level master > 0.01\n");
    let c = report(&src);
    let again = report(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    assert_eq!(c.cid, djlive::program_cid(&c.text));
    // the statements print in order, after what was before them
    let tail: Vec<&str> = c.text.lines().skip_while(|l| !l.starts_with("assert")).collect();
    assert_eq!(tail, ["assert ends", "assert period == 1", "assert length == 24s", "assert key == dm in climax", "assert level A < 2 * B in climax", "assert level master > 0.01"]);
}

#[test]
fn another_spelling_is_the_same_program_and_another_assertion_is_another() {
    let base = "deck A: kick x\nend at bar 4\n";
    let cid = |a: &str| report(&format!("{base}{a}\n")).cid;
    assert_eq!(cid("assert length == 3m30s ± 5s"), cid("assert length==210s±5s"));
    assert_eq!(cid("assert length == 90s"), cid("assert length == 1m30s"));
    assert_eq!(cid("assert level A < (1 + 1) * B"), cid("assert level A < 2 * B"));
    assert_eq!(cid("assert key == Bbm"), cid("assert key == a#m"));
    let c = cid("assert length == 3m30s ± 5s");
    for other in ["assert length == 3m30s ± 6s", "assert length == 3m31s ± 5s", "assert length != 3m30s ± 5s", "assert length == 3m30s", "assert ends", ""] {
        assert_ne!(c, cid(other), "{other}");
    }
    assert_ne!(cid("assert level A < 2 * B"), cid("assert level A <= 2 * B"));
    assert_ne!(cid("assert level A < 2 * B"), cid("assert level B < 2 * A"));
    // the order of statements is meaning
    assert_ne!(cid("assert ends\nassert key == c"), cid("assert key == c\nassert ends"));
    // a comment is not part of it
    assert_eq!(cid("assert ends"), cid("assert ends # and so it does"));
    // a program with no assertion keeps the CID it had
    assert_eq!(check(base).compiled.unwrap().text, base);
}

// ---- the static assertions

#[test]
fn ends_holds_for_a_song_an_end_and_a_bounded_evolve_and_fails_for_a_program_that_plays_on() {
    let r = judged("assert ends\n");
    assert_eq!(r, [(Status::Pass, "assert ends: the program ends at bar 24".to_string())]);
    let c = report("deck A: kick x\nend at bar 8\nassert ends\n");
    assert_eq!(c.assertions[0].status, Status::Pass);
    assert_eq!(c.assertions[0].message, "the program ends at bar 8");
    // a program with no sections and an `evolve` with a `stop` is a timeline that ends
    let c = report("deck A: kick x ~ x ~ | hihat x*8\nevolve every 4 bars {\n  hihat: rotate 1\n  stop when gen == 3\n}\nassert ends\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    // an evolve with no `stop` runs the orbit of its parts and ends there
    let c = report("deck A: hihat x ~ ~ ~ ~ ~ ~ ~\nevolve every 4 bars {\n  hihat: rotate 2\n}\nassert ends\nassert length == 1m12s\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    // (the orbit of 8 generations, played through and round to the first again: 9 generations of 4 bars, 36 bars at 120 BPM: 72 s)
    assert_eq!(c.assertions[1].status, Status::Pass, "{}", c.assertions[1].line());
    // plays on
    let c = report("deck A: kick x\nassert ends\n");
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert_eq!(c.assertions[0].line(), "assert ends: the program does not end: nothing stops it");
    assert!(c.assertions[0].help.as_deref().unwrap().contains("end at bar N"));
    // loops
    let c = report("deck A: kick x\nsection one 4 bars {\n  deck A: kick x\n}\narrange one\nloop\nassert ends\n");
    assert_eq!(c.assertions[0].line(), "assert ends: the program loops: `loop` starts the song again at its end");
}

#[test]
fn length_is_the_bars_through_the_tempo() {
    // 24 bars at 120 BPM
    let r = judged("assert length == 48s\nassert length == 48s ± 0s\nassert length == 3m\nassert length == 3m30s ± 5s\nassert length < 1m\nassert length >= 48s\nassert length > 48s\n");
    let want = [
        (Status::Pass, "assert length == 48s: the program is 48s"),
        (Status::Pass, "assert length == 48s ± 0s: the program is 48s"),
        (Status::Fail, "assert length == 3m: the program is 48s"),
        (Status::Fail, "assert length == 3m30s ± 5s: the program is 48s"),
        (Status::Pass, "assert length < 1m: the program is 48s"),
        (Status::Pass, "assert length >= 48s: the program is 48s"),
        (Status::Fail, "assert length > 48s: the program is 48s"),
    ];
    assert_eq!(r.len(), want.len());
    for (got, w) in r.iter().zip(want) {
        assert_eq!((got.0, got.1.as_str()), w);
    }
    // the default tempo is 120; another tempo is another length: 24 bars at 96 BPM are exactly 60 s
    let c = report("tempo 96\ndeck A: kick x\nend at bar 24\nassert length == 1m\n");
    assert_eq!(c.assertions[0].status, Status::Pass);
    let c = report("deck A: kick x\nend at bar 24\nassert length == 48s\n");
    assert_eq!(c.assertions[0].status, Status::Pass);
    // a length that is not whole seconds says so to the millisecond
    let c = report("tempo 126\ndeck A: kick x\nend at bar 100\nassert length == 3m10s\nassert length == 3m10.476s ± 1ms\n");
    assert_eq!(c.assertions[0].line(), "assert length == 3m10s: the program is 3m10.476s");
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert_eq!(c.assertions[1].status, Status::Pass);
    // `end after` and `end at` decide where it ends
    let c = report(&format!("{SONG}end after intro\nassert length == 16s\n").replace("arrange intro climax\n", "arrange intro climax\n"));
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
}

#[test]
fn a_tolerance_widens_equal_and_not_equal_and_nothing_else() {
    // 24 bars at 120: 48 s
    let r = |a: &str| judged(a)[0].0;
    assert_eq!(r("assert length == 50s ± 2s\n"), Status::Pass);
    assert_eq!(r("assert length == 50s ± 1.999s\n"), Status::Fail);
    assert_eq!(r("assert length == 46s ± 2s\n"), Status::Pass);
    assert_eq!(r("assert length != 50s ± 1s\n"), Status::Pass);
    assert_eq!(r("assert length != 50s ± 3s\n"), Status::Fail);
    // a plain `==` has tolerance 0
    assert_eq!(r("assert length == 48s\n"), Status::Pass);
    assert_eq!(r("assert length == 48.001s\n"), Status::Fail);
    let e = one_error(&song("assert length < 4m ± 5s\n"));
    assert!(e.message.contains("a tolerance `±` goes with `==` or `!=`, not `<`"), "{}", e.message);
    assert!(e.help.as_deref().unwrap().contains("length == 4m ± 5s"));
}

#[test]
fn a_program_that_never_ends_has_no_length() {
    let c = report("deck A: kick x\nassert length < 1h\n".replace("1h", "99m").as_str());
    assert_eq!(c.assertions[0].status, Status::Fail);
    assert_eq!(c.assertions[0].line(), "assert length < 99m: the program plays on, so it has no length");
    // a loop has a lap, which is said
    let c = report(&format!("{SONG}loop\nassert length == 48s\n"));
    assert_eq!(c.assertions[0].line(), "assert length == 48s: the program loops, so it has no length (one lap is 48s)");
}

#[test]
fn key_is_the_programs_or_a_sections_own() {
    let r = judged("assert key == am\nassert key == dm in climax\nassert key == am in intro\nassert key == am in climax\nassert key != dm in climax\nassert key == dm\n");
    let want = [
        (Status::Pass, "assert key == am: the program's key is am"),
        (Status::Pass, "assert key == dm in climax: the key in climax is dm"),
        (Status::Pass, "assert key == am in intro: the key in intro is am"),
        (Status::Fail, "assert key == am in climax: the key in climax is dm"),
        (Status::Fail, "assert key != dm in climax: the key in climax is dm"),
        (Status::Fail, "assert key == dm: the program's key is am"),
    ];
    for (got, w) in r.iter().zip(want) {
        assert_eq!((got.0, got.1.as_str()), w);
    }
    // two spellings of a key are one key; a mode is part of it
    let c = report(&song("assert key == D:Minor in climax\nassert key == d:dorian in climax\n"));
    assert_eq!((c.assertions[0].status, c.assertions[1].status), (Status::Pass, Status::Fail));
    // no key is C major, and the message says so
    let c = report("deck A: kick x\nassert key == c\nassert key == am\n");
    assert_eq!(c.assertions[0].line(), "assert key == c: the program's key is c (no `key` is set: C major)");
    assert_eq!(c.assertions[1].status, Status::Fail);
    // a section with no key of its own has the program's
    let c = report(&song("assert key == am in intro\n"));
    assert_eq!(c.assertions[0].status, Status::Pass);
}

#[test]
fn period_is_the_programs_as_meter_reports_it_or_an_evolves() {
    // one bar of everything
    assert_eq!(judged("assert period == 1\n")[0], (Status::Pass, "assert period == 1: the program's period is 1 bar".to_string()));
    // hihat/12 against the bar: lcm(12, 16) = 48 steps = 3 bars
    let c = report("deck A: kick x ~ x ~ | hihat/12 x x ~ x\nassert period == 3\nassert period < 3\nassert period >= 2\n");
    assert_eq!(c.assertions[0].line(), "assert period == 3: the program's period is 3 bars (hihat/12 against the bar)");
    assert_eq!((c.assertions[1].status, c.assertions[2].status), (Status::Fail, Status::Pass));
    // an alternation counts too: <a b c> takes three bars
    let c = report("deck A: bass <c2 d2 e2> ~ ~ ~\nassert period == 3\n");
    assert_eq!(c.assertions[0].status, Status::Pass);
    // sections with their own periods: the least common multiple of them
    let c = report("section one 4 bars {\n  deck A: bass <c2 d2> ~ ~ ~\n}\nsection two 6 bars {\n  deck A: bass <c2 d2 e2> ~ ~ ~\n}\narrange one two\nassert period == 6\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
}

/// The docs' example with its `stop` taken out: the melody climbs, the thinned hi-hat runs out of hits, and what is left
/// comes round every four generations: period 4 generations of 8 bars = 32 bars.
const EVOLVING: &str = "tempo 124
section drop 32 bars {
  deck B: melody [a4 c5 e5 g5] | bass [a2 ~ e2 ~]
  deck A: kick x ~ x ~ | hihat x*8
  evolve every 8 bars {
    melody: rotate 1 | transpose <0 5 7 0>
    hihat: thin 0.1 seed=3
    bass: invert when gen % 2 == 1
  }
}
section tail 8 bars {
  deck A: kick x ~ x ~
}
arrange drop tail
";

#[test]
fn period_of_an_evolve_is_its_cycle_in_bars() {
    let c = report(&format!("{EVOLVING}assert period(drop) == 32\nassert period(drop) == 16\nassert period(drop) >= 32\nassert period(drop) < 32\n"));
    let a = &c.assertions;
    assert_eq!(a[0].status, Status::Pass, "{}", a[0].line());
    assert_eq!(a[0].message, "the period of the evolve in drop is 32 bars (4 generations of 8 bars)");
    assert_eq!(a[1].line(), "assert period(drop) == 16: the period of the evolve in drop is 32 bars (4 generations of 8 bars)");
    assert_eq!((a[1].status, a[2].status, a[3].status), (Status::Fail, Status::Pass, Status::Fail));
    // it agrees with what `check` reports for the evolve
    assert!(c.evolve_report()[0].contains("period 32 bars"), "{:?}", c.evolve_report());
    // a program's own evolve is `evolve`
    let c = report("deck A: kick x ~ x ~ | hihat x ~ ~ ~ ~ ~ ~ ~\nevolve every 4 bars {\n  hihat: rotate 2\n}\nassert period(evolve) == 32\nassert period(evolve) == 4\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
    assert_eq!(c.assertions[1].line(), "assert period(evolve) == 4: the period of the program's evolve is 32 bars (8 generations of 4 bars)");
    assert_eq!(c.assertions[1].status, Status::Fail);
    // a rule that changes nothing comes round at once
    let c = report("deck A: bass c2 ~ ~ ~\nevolve every 4 bars {\n  bass: transpose 0\n}\nassert period(evolve) == 4\n");
    assert_eq!(c.assertions[0].status, Status::Pass, "{}", c.assertions[0].line());
}

#[test]
fn a_false_static_is_an_error_at_its_statement_with_the_actual_value() {
    let src = song("assert ends\nassert length == 3m30s ± 5s\n");
    let e = one_error(&src);
    assert_eq!(e.message, "assert length == 3m30s ± 5s: the program is 48s");
    assert_eq!(at(&src, &e), (13, 1));
    assert_eq!(e.span.text(&src), "assert length == 3m30s ± 5s");
    assert!(e.is_error());
    // each kind says what is
    for (a, msg) in [
        ("assert key == fm in climax", "assert key == fm in climax: the key in climax is dm"),
        ("assert period == 7", "assert period == 7: the program's period is 1 bar"),
        ("assert length > 1m", "assert length > 1m: the program is 48s"),
    ] {
        let src = song(&format!("{a}\n"));
        assert_eq!(one_error(&src).message, msg);
    }
    assert_eq!(one_error("deck A: kick x\nassert ends\n").message, "assert ends: the program does not end: nothing stops it");
    // every false one is an error of its own, in the order of the text
    let src = song("assert key == c\nassert period == 2\nassert length == 1s\n");
    let e = errors(&src);
    assert_eq!(e.len(), 3);
    assert!(e.windows(2).all(|w| w[0].span.start < w[1].span.start));
}

#[test]
fn eval_refuses_a_program_with_a_false_static_and_keeps_the_last_good_one() {
    let mut live = djlive::Live::new();
    let good = live.eval(&song("assert length == 48s\n"));
    assert!(good.ok, "{:?}", good.diagnostics);
    let bad = live.eval(&song("assert length == 49s\n"));
    assert!(!bad.ok);
    assert!(bad.diagnostics.iter().any(|d| d.is_error() && d.message == "assert length == 49s: the program is 48s"), "{:?}", bad.diagnostics);
    assert!(bad.cid.is_none() && bad.canonical.is_none());
    // `check` too: nothing changes
    assert!(!live.check(&song("assert ends\nassert period == 9\n")).ok);
    // a pending assertion does not block it
    let pending = live.eval(&song("assert level A < 2 * B in climax\n"));
    assert!(pending.ok, "{:?}", pending.diagnostics);
}

#[test]
fn check_report_keeps_a_false_static_as_a_failure_and_still_refuses_what_names_nothing() {
    let src = song("assert length == 1m\nassert ends\nassert level A < B in climax\n");
    assert!(check(&src).compiled.is_none());
    let c = check_report(&src, &Env::none());
    let comp = c.compiled.expect("a false assertion is reported, not refused");
    let st: Vec<Status> = comp.assertions.iter().map(|a| a.status).collect();
    assert_eq!(st, [Status::Fail, Status::Pass, Status::Pending]);
    assert!(c.diagnostics.iter().all(|d| !d.is_error()));
    // a name that is nowhere is an error either way
    let src = song("assert key == c in nowhere\n");
    assert!(check_report(&src, &Env::none()).compiled.is_none());
}

// ---- the level assertions: pending at check

#[test]
fn level_assertions_are_notes_at_check_and_do_not_block() {
    let src = song("assert level A < 2 * B in climax\nassert level master > 0.01\n");
    let c = check(&src);
    let comp = c.compiled.expect("a pending assertion does not block");
    assert_eq!(comp.assertions.len(), 2);
    assert!(comp.assertions.iter().all(|a| a.status == Status::Pending));
    // one note each, a warning with the statement's place, never an error
    let notes: Vec<&Diagnostic> = c.diagnostics.iter().filter(|d| d.message.contains("pending")).collect();
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().all(|d| d.severity == Severity::Info));
    assert_eq!(notes[0].message, "assert level A < 2 * B in climax: pending: measured on an offline render of section climax (`btw test`)");
    assert_eq!(notes[1].message, "assert level master > 0.01: pending: measured on an offline render of the whole program (`btw test`)");
    assert_eq!(at(&src, notes[0]), (12, 1));
    // and they are in the compiled program's warnings too
    assert!(comp.warnings.iter().filter(|d| d.message.contains("pending")).count() == 2);
    // statics that pass say nothing
    let c = check(&song("assert ends\nassert length == 48s\n"));
    assert!(c.diagnostics.iter().all(|d| !d.message.contains("assert")));
}

#[test]
fn a_level_assertion_knows_which_bars_it_is_measured_over() {
    let c = report("section verse 4 bars {\n  deck A: kick x\n}\nsection drop 8 bars {\n  deck A: kick x\n}\narrange verse drop verse\nassert level A < B in verse\nassert level A < B in drop\nassert level A < B\n");
    let w: Vec<(Option<&str>, Vec<(u32, u32)>)> = c.assertions.iter().map(|a| a.level.as_ref().map(|l| (l.section.as_deref(), l.windows.clone())).unwrap()).collect();
    // a section named twice on the timeline is measured in both places
    assert_eq!(w[0], (Some("verse"), vec![(0, 4), (12, 16)]));
    assert_eq!(w[1], (Some("drop"), vec![(4, 12)]));
    assert_eq!(w[2], (None, vec![]));
    let l = c.assertions[0].level.as_ref().unwrap();
    assert_eq!(l.leaves(), ["A", "B"]);
}

#[test]
fn a_level_is_judged_from_what_was_measured() {
    let c = report(&song("assert level A < 2 * B in climax\nassert level A + B >= master\nassert level min(A, B) == 0\nassert level A / B > 1\nassert level -A < 0\nassert level A != B\n"));
    let judge = |i: usize, a: f64, b: f64, m: f64| {
        let v = c.assertions[i].level.as_ref().unwrap().judge(&|w| match w {
            "A" => a,
            "B" => b,
            _ => m,
        });
        (v.pass, v.text)
    };
    // the contract's example
    assert_eq!(judge(0, 0.083, 0.051, 0.0), (true, "A 0.083, B 0.051: 0.083 < 0.102".to_string()));
    assert_eq!(judge(0, 0.2, 0.051, 0.0), (false, "A 0.200, B 0.051: 0.200 < 0.102".to_string()));
    // the leaves are listed once, in the order met, whichever side they are on
    assert_eq!(judge(1, 0.5, 0.25, 0.7), (true, "A 0.500, B 0.250, master 0.700: 0.750 >= 0.700".to_string()));
    // silence is exactly 0, and `==` is exact
    assert!(judge(2, 0.3, 0.0, 0.0).0 && !judge(2, 0.3, 0.1, 0.0).0);
    assert_eq!(judge(2, 0.3, 0.0, 0.0).1, "A 0.300, B 0: 0 == 0");
    // a division by zero is not a number and fails, and says so
    let (pass, text) = judge(3, 0.5, 0.0, 0.0);
    assert!(pass, "{text}: infinity is greater");
    let (pass, text) = judge(3, 0.0, 0.0, 0.0);
    assert!(!pass && text.ends_with("(not a number: a division by zero)"), "{text}");
    assert!(judge(4, 0.1, 0.0, 0.0).0);
    assert!(judge(5, 0.1, 0.2, 0.0).0 && !judge(5, 0.2, 0.2, 0.0).0);
    // tiny levels are shown in scientific notation, not as 0.000
    assert_eq!(judge(5, 4e-5, 0.0, 0.0).1, "A 4.0e-5, B 0: 4.0e-5 != 0");
}

// ---- errors

#[test]
fn errors_say_where_and_what_would_do() {
    struct Case {
        src: &'static str,
        at: (usize, usize),
        message: &'static str,
        help: &'static str,
    }
    let cases = [
        Case { src: "deck A: kick x\nassert lenght == 3m\n", at: (2, 8), message: "`assert` is followed by what to assert, found `lenght`", help: "did you mean `length`?" },
        Case { src: "deck A: kick x\nassert levl A < B\n", at: (2, 8), message: "`assert` is followed by what to assert, found `levl`", help: "did you mean `level`?" },
        Case { src: "deck A: kick x\nassert\n", at: (2, 7), message: "`assert` is followed by what to assert, found the end of the line", help: "assert ends" },
        Case { src: "deck A: kick x\nassert 3\n", at: (2, 8), message: "`assert` is followed by what to assert, found a number", help: "assert ends" },
        Case { src: "deck A: kick x\nassert ends soon\n", at: (2, 13), message: "expected the end of the line, found `soon`", help: "" },
        Case { src: "deck A: kick x\nassert length 3m\n", at: (2, 15), message: "expected a comparison, found a number", help: "assert length == 3m30s ± 5s" },
        Case { src: "deck A: kick x\nassert length = 3m\n", at: (2, 15), message: "expected a comparison, found `=`", help: "assert length == 3m30s ± 5s" },
        Case { src: "deck A: kick x\nassert length == 3\n", at: (2, 18), message: "a length needs a unit glued to its number, found `3`", help: "3m30s" },
        Case { src: "deck A: kick x\nassert length == three\n", at: (2, 18), message: "expected a length like `3m30s`, `90s` or `4m`, found `three`", help: "3m30s" },
        Case { src: "deck A: kick x\nassert length == 6db\n", at: (2, 19), message: "`db` is not a unit of time: a clock is minutes and seconds", help: "3m30s   90s   4m   500ms" },
        Case { src: "deck A: kick x\nassert length == -3m\n", at: (2, 18), message: "a length is not negative", help: "3m30s" },
        Case { src: "deck A: kick x\nassert length == 3m ±\n", at: (2, 22), message: "expected a tolerance like `5s`, found the end of the line", help: "3m30s" },
        Case { src: "deck A: kick x\nassert length < 3m ± 5s\n", at: (2, 20), message: "a tolerance `±` goes with `==` or `!=`, not `<`", help: "length == 4m ± 5s" },
        Case { src: "deck A: kick x\nassert level C < B\n", at: (2, 14), message: "`C` is not something with a level", help: "a level is read from deck `A`, deck `B` or `master`" },
        Case { src: "deck A: kick x\nassert level a < B\n", at: (2, 14), message: "`a` is not something with a level", help: "did you mean `A`?" },
        Case { src: "deck A: kick x\nassert level mastr < B\n", at: (2, 14), message: "`mastr` is not something with a level", help: "did you mean `master`?" },
        Case { src: "deck A: kick x\nassert level A < 2 * decks\n", at: (2, 22), message: "`decks` is not something with a level", help: "a level is read from deck `A`" },
        Case { src: "deck A: kick x\nassert level A\n", at: (2, 15), message: "expected a comparison, found the end of the line", help: "compare two levels" },
        Case { src: "deck A: kick x\nassert level < B\n", at: (2, 14), message: "expected a number or a name, found `<`", help: "a level is `A`, `B` or `master`" },
        Case { src: "deck A: kick x\nassert level 1 < 2\n", at: (2, 14), message: "this reads no level: it names no deck", help: "name `A`, `B` or `master`" },
        Case { src: "deck A: kick x\nassert level A / 0 < 1\n", at: (2, 18), message: "division by zero", help: "" },
        Case { src: "deck A: kick x\nassert level A % 0 < 1\n", at: (2, 18), message: "modulo by zero", help: "" },
        Case { src: "deck A: kick x\nassert level A < 1 / 0\n", at: (2, 22), message: "division by zero", help: "" },
        Case { src: "deck A: kick x\nassert level A < (B < 1)\n", at: (2, 19), message: "a level is a number: a comparison goes between two levels, not inside one", help: "level A < 2 * B" },
        Case { src: "deck A: kick x\nassert level A < sqrt(B)\n", at: (2, 18), message: "`sqrt(...)` is not a function of levels", help: "`min(A, B)` and `max(A, B)` are" },
        Case { src: "deck A: kick x\nassert level A < B in\n", at: (2, 22), message: "expected the name of a section, found the end of the line", help: "" },
        Case { src: "deck A: kick x\nassert key == zz\n", at: (2, 15), message: "`zz` is not a key", help: "modes: major" },
        Case { src: "deck A: kick x\nassert key < c\n", at: (2, 12), message: "keys are compared with `==` and `!=`, not `<`", help: "assert key == dm in climax" },
        Case { src: "deck A: kick x\nassert key c\n", at: (2, 12), message: "expected a comparison, found `c`", help: "assert key == dm in climax" },
        Case { src: "deck A: kick x\nassert period(drop)\n", at: (2, 20), message: "expected a comparison, found the end of the line", help: "assert period == 3" },
        Case { src: "deck A: kick x\nassert period == x\n", at: (2, 18), message: "expected a period in bars, found `x`", help: "" },
        Case { src: "deck A: kick x\nassert period == 1.5\n", at: (2, 18), message: "a period in bars is a whole number", help: "" },
        Case { src: "deck A: kick x\nassert period(\n", at: (2, 15), message: "expected a section with an evolve, or `evolve`, found the end of the line", help: "" },
    ];
    for c in cases {
        // (not the last line of the text, so that "the end of the line" is what is found there)
        let src = format!("{}deck B: kick x\n", c.src);
        let c = Case { src: Box::leak(src.into_boxed_str()), ..c };
        let Some(d) = errors(c.src).into_iter().next() else { unreachable!() };
        if c.message.is_empty() {
            assert!(d.is_error());
            continue;
        }
        assert_eq!((at(c.src, &d), d.message.as_str()), (c.at, c.message), "{}", c.src);
        let all = format!("{} {}", d.help.clone().unwrap_or_default(), d.expected.join(" "));
        assert!(all.contains(c.help), "help of {:?} is {:?}, wanted {:?}", c.src, d.help, c.help);
    }
}

#[test]
fn a_name_that_is_nowhere_lists_what_is() {
    let e = one_error(&song("assert level A < B in clmax\n"));
    assert_eq!(at(&song("assert level A < B in clmax\n"), &e).0, 12);
    assert_eq!(e.message, "there is no section `clmax`");
    assert_eq!(e.expected, ["intro", "climax"]);
    assert!(e.help.as_deref().unwrap().contains("did you mean `climax`?"), "{:?}", e.help);
    let e = one_error(&song("assert key == c in intr\n"));
    assert!(e.help.as_deref().unwrap().contains("did you mean `intro`?"));
    // a program with no sections has nothing to name
    let e = one_error("deck A: kick x\nassert level A < B in drop\n");
    assert_eq!(e.message, "there is no section `drop`: this program has no sections");
    assert!(e.help.as_deref().unwrap().contains("leave out `in`"));
    // a section that is declared but not on the timeline is not one
    let e = one_error("section one 2 bars {\n  deck A: kick x\n}\nsection two 2 bars {\n  deck A: kick x\n}\narrange one\nassert key == c in two\n");
    assert_eq!(e.expected, ["one"]);
    // the evolves
    let e = one_error(&format!("{EVOLVING}assert period(tail) == 1\n"));
    assert_eq!(e.message, "section `tail` has no `evolve`");
    assert!(e.help.as_deref().unwrap().contains("the evolves are in drop"));
    let e = one_error(&format!("{EVOLVING}assert period(dorp) == 1\n"));
    assert_eq!(e.message, "there is no evolve in `dorp`");
    assert!(e.help.as_deref().unwrap().contains("did you mean `drop`?"));
    let e = one_error("deck A: kick x\nassert period(drop) == 1\n");
    assert!(e.message.contains("this program has no `evolve`"), "{}", e.message);
}

#[test]
fn an_assert_belongs_at_the_top_of_the_program() {
    let e = one_error("section one 4 bars {\n  deck A: kick x\n  assert ends\n}\narrange one\n");
    assert_eq!((at("section one 4 bars {\n  deck A: kick x\n  assert ends\n}\n", &e), e.message.as_str()), ((3, 3), "an `assert` belongs at the top of the program, outside blocks"));
    assert!(e.help.as_deref().unwrap().contains("in"));
    let e = one_error("def groove() {\n  assert ends\n}\ngroove()\n");
    assert!(e.message.contains("belongs at the top"), "{}", e.message);
    let e = one_error("repeat 2 {\n  assert ends\n}\n");
    assert!(e.message.contains("belongs at the top"), "{}", e.message);
    // `$i` is no value in one
    let e = one_error("deck A: kick x\nassert level A < $i\n");
    assert!(e.message.contains("has no value here"), "{}", e.message);
}

#[test]
fn the_words_are_reserved_and_a_comment_is_a_comment() {
    // `assert` starts a statement: it names nothing. The other words are syntax in one position only (after `assert`), so
    // they name a let as they did before the extensions, and the assertion still reads them (fix_syntax.rs)
    for name in ["assert"] {
        let e = one_error(&format!("let {name} = 3\ndeck A: kick x\n"));
        assert!(e.message.contains("cannot be a name") || e.message.contains("cannot name"), "{name}: {}", e.message);
        assert!(e.help.as_deref().is_some_and(|h| h.contains("rename it")), "{name}: {:?}", e.help);
    }
    for name in ["ends", "period", "length", "in", "test"] {
        ok(&format!("let {name} = 3\ndeck A: kick x\nend at bar 4\nassert ends\n"));
    }
    let c = ok("deck A: kick x\n# assert length == 3m\n");
    assert!(c.assertions.is_empty());
    assert_eq!(c.text, "deck A: kick x\n");
}

#[test]
fn an_import_does_not_bring_a_snippets_assertions() {
    let snippet = "let riff = c2 ~ [e2 g2] ~\nassert ends\nassert length == 1s\n";
    let canon = check_with(snippet, &Env::none()).compiled;
    // (the snippet on its own is a program whose assertion is false: it is saved as text all the same)
    assert!(canon.is_none());
    let text = "let riff = c2 ~ [e2 g2] ~\nassert level A < B\n";
    let comp = check_with(text, &Env::none()).compiled.unwrap();
    let mut snips = StaticSnippets::default();
    snips.items.push(("riffs".into(), comp.cid.clone(), comp.text.clone()));
    let env = Env { snippets: Some(&snips), ..Env::none() };
    let c = check_with("import \"riffs\"\ndeck A: bass riff\n", &env).compiled.unwrap();
    assert!(c.assertions.is_empty(), "{:?}", c.assertions);
    assert!(c.text.starts_with("import "));
}

#[test]
fn the_statements_span_is_the_statement_and_the_editor_colours_the_words() {
    let src = song("assert length == 48s\nassert level A < B in climax\n");
    let c = report(&src);
    assert_eq!(c.assertions[0].span.text(&src), "assert length == 48s");
    assert_eq!(c.assertions[1].span.text(&src), "assert level A < B in climax");
    let classes = djlive::highlight::highlight("assert length == 3m30s ± 5s\nassert ends\nassert period(drop) == 4\nassert level A < B in drop\n");
    let class_of = |w: &str| classes.iter().find(|(s, _)| s.text("assert length == 3m30s ± 5s\nassert ends\nassert period(drop) == 4\nassert level A < B in drop\n") == w).map(|(_, c)| *c);
    use djlive::highlight::Class;
    assert_eq!(class_of("assert"), Some(Class::Keyword));
    assert_eq!(class_of("length"), Some(Class::Keyword));
    assert_eq!(class_of("ends"), Some(Class::Keyword));
    assert_eq!(class_of("in"), Some(Class::Keyword));
    // (`assert period(drop)`: the assertion's own word, not a generator: SYN-18)
    assert_eq!(class_of("period"), Some(Class::Keyword));
    assert_eq!(class_of("A"), Some(Class::Deck));
    assert_eq!(class_of("level"), Some(Class::Action));
}

// ---- laws

fn the_program_with_every_assertion() -> String {
    song("assert ends\nassert period == 1\nassert length == 48s ± 3s\nassert length < 5m\nassert key == dm in climax\nassert key != c\nassert level A < 2 * B in climax\nassert level master >= 0.5 * (A + B)\nassert level min(A, B) > 0.01 in intro\n")
}

#[test]
fn reformatting_keeps_the_cid_and_an_edit_changes_it() {
    let src = the_program_with_every_assertion();
    let cid = report(&src).cid;
    let mut rng = Rng(0xa55e_47f1_2024_0001);
    for round in 0..40 {
        let text = reformat(&src, &mut rng);
        let c = check_report(&text, &Env::none());
        let got = c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", render_all(&c.diagnostics, &text)));
        assert_eq!(got.cid, cid, "round {round} forked:\n{text}");
    }
    // each edit is another program
    for (from, to) in [
        ("assert length == 48s ± 3s", "assert length == 48s ± 4s"),
        ("assert length < 5m", "assert length < 6m"),
        ("assert period == 1", "assert period == 2"),
        ("assert key == dm in climax", "assert key == fm in climax"),
        ("assert key != c", "assert key == c"),
        ("assert level A < 2 * B in climax", "assert level A < 3 * B in climax"),
        ("assert level A < 2 * B in climax", "assert level A < 2 * B in intro"),
        ("assert level master >= 0.5 * (A + B)", "assert level master >= 0.5 * (A - B)"),
        ("assert level min(A, B) > 0.01 in intro", "assert level max(A, B) > 0.01 in intro"),
        ("assert ends\n", ""),
    ] {
        let edited = src.replacen(from, to, 1);
        assert_ne!(edited, src);
        assert_ne!(report(&edited).cid, cid, "{from} -> {to}");
    }
}

#[test]
fn the_plus_minus_sign_and_the_units_stay_glued_through_a_reformat() {
    // `3m30s±5s` glued and `3m30s ± 5s` spaced are one program; a unit never comes apart from its number
    let a = report("deck A: kick x\nend at bar 4\nassert length == 3m30s±5s\n").cid;
    let b = report("deck A: kick x\nend at bar 4\nassert length == 3m30s ± 5s\n").cid;
    let c = report("deck A: kick x\nend at bar 4\nassert length == 3m30s   ±   5s\n").cid;
    assert_eq!((a.as_str(), b.as_str()), (c.as_str(), c.as_str()));
    // the lexer keeps `3m30s` as four tokens (3 m 30 s) and the shared helper knows the pairs
    let toks: Vec<_> = djlive::lex::lex("3m30s ± 5s").tokens.into_iter().map(|t| t.tok).collect();
    use djlive::lex::Tok;
    for w in toks.windows(2) {
        if matches!(w[1], Tok::Unit(_) | Tok::PlusMinus) {
            assert!(common::glued_pair(&w[0], &w[1]), "{w:?}");
        }
    }
}

/// A random clock (what a person might write), and its seconds.
fn random_clock(rng: &mut Rng) -> (String, Rat) {
    let secs = |n: i64, d: i64| Rat::new(n, d).unwrap();
    match rng.below(6) {
        0 => {
            let (m, s) = (rng.below(20) as i64, rng.below(60) as i64);
            (format!("{m}m{s}s"), secs(m * 60 + s, 1))
        }
        1 => {
            let s = rng.below(5000) as i64;
            (format!("{s}s"), secs(s, 1))
        }
        2 => {
            let m = rng.below(90) as i64;
            (format!("{m}m"), secs(m * 60, 1))
        }
        3 => {
            let ms = rng.below(100_000) as i64;
            (format!("{ms}ms"), secs(ms, 1000))
        }
        4 => {
            let (s, d) = (rng.below(900) as i64, rng.below(10) as i64);
            (format!("{s}.{d}s"), secs(s * 10 + d, 10))
        }
        _ => {
            let (m, s, d) = (rng.below(9) as i64, rng.below(59) as i64, 1 + rng.below(9) as i64);
            (format!("{m}m{s}.{d}s"), secs(m * 600 + s * 10 + d, 10))
        }
    }
}

#[test]
fn a_clock_prints_so_that_it_reads_back_as_the_same_seconds() {
    let mut rng = Rng(0xc10c_c10c_0000_0001);
    for _ in 0..400 {
        let (written, secs) = random_clock(&mut rng);
        let printed = clock_text(secs);
        // read it back through a statement
        let c = canon(&format!("assert length >= {written}"));
        assert_eq!(c, format!("assert length >= {printed}"), "{written}");
        let back = canon(&c);
        assert_eq!(back, c, "{written} -> {printed}");
        // and it is the seconds that were meant
        let comp = report(&format!("deck A: kick x\nassert length >= {printed}\n"));
        let djlive::ast::StmtKind::Assert(a) = &djlive::parse::parse(&format!("assert length >= {printed}\n")).program.stmts[0].kind else { panic!() };
        let djlive::assertion::AssertKind::Length { target, .. } = &a.kind else { panic!() };
        assert_eq!(target.secs, secs, "{written} -> {printed}");
        let _ = comp;
    }
}

fn random_level(rng: &mut Rng, depth: u32) -> String {
    let leaves = ["A", "B", "master"];
    let nums = ["0", "1", "2", "3", "0.5", "0.25", "0.05", "10", "1 / 3", "5%", "(1 + 2)"];
    if depth == 0 || rng.below(3) == 0 {
        return if rng.below(3) == 0 { nums[rng.below(nums.len() as u64) as usize].to_string() } else { leaves[rng.below(3) as usize].to_string() };
    }
    let (a, b) = (random_level(rng, depth - 1), random_level(rng, depth - 1));
    match rng.below(8) {
        0 => format!("{a} + {b}"),
        1 => format!("{a} - {b}"),
        2 => format!("({a}) * ({b})"),
        3 => format!("{a} * {b}"),
        4 => format!("({a}) / ({b} + master)"),
        5 => format!("min({a}, {b})"),
        6 => format!("max({a}, {b}, {})", leaves[rng.below(3) as usize]),
        _ => format!("-({a}) + {b}"),
    }
}

fn random_assertion(rng: &mut Rng) -> String {
    let ops = ["==", "!=", "<", "<=", ">", ">="];
    let op = ops[rng.below(6) as usize];
    let section = |rng: &mut Rng| if rng.below(2) == 0 { String::new() } else { format!(" in {}", ["intro", "climax"][rng.below(2) as usize]) };
    match rng.below(5) {
        0 => "assert ends".to_string(),
        1 => format!("assert period{} {op} {}", if rng.below(2) == 0 { String::new() } else { format!("({})", ["evolve", "drop"][rng.below(2) as usize]) }, rng.below(100)),
        2 => {
            let (c, _) = random_clock(rng);
            let tol = if (op == "==" || op == "!=") && rng.below(2) == 0 { format!(" ± {}", random_clock(rng).0) } else { String::new() };
            format!("assert length {op} {c}{tol}")
        }
        3 => {
            let (l, r) = (random_level(rng, 2), random_level(rng, 2));
            // at least one deck must be read
            format!("assert level A + {l} {op} {r}{}", section(rng))
        }
        _ => {
            let k = ["am", "dm", "c", "f#m", "bb", "d:dorian", "e:blues", "Gm"][rng.below(8) as usize];
            format!("assert key {} {k}{}", if rng.below(2) == 0 { "==" } else { "!=" }, section(rng))
        }
    }
}

#[test]
fn random_assertions_print_a_fixed_point_and_reformat_to_the_same_text() {
    let mut rng = Rng(0xa55e_47f1_2024_0002);
    for n in 0..300 {
        let a = random_assertion(&mut rng);
        let parsed = djlive::parse::parse(&format!("{a}\n"));
        assert!(parsed.ok(), "#{n} `{a}`: {}", render_all(&parsed.diagnostics, &a));
        let canonical = djlive::print::canonical(&parsed.program);
        let again = djlive::parse::parse(&canonical);
        assert!(again.ok(), "#{n} `{a}` -> `{canonical}`: {}", render_all(&again.diagnostics, &canonical));
        assert_eq!(djlive::print::canonical(&again.program), canonical, "#{n} `{a}`");
        // reformatted, it is the same statement
        let text = reformat(&format!("{a}\n"), &mut rng);
        let re = djlive::parse::parse(&text);
        assert!(re.ok(), "#{n} reformat of `{a}`:\n{text}\n{}", render_all(&re.diagnostics, &text));
        assert_eq!(djlive::print::canonical(&re.program), canonical, "#{n} `{a}` as\n{text}");
    }
}

#[test]
fn the_length_is_what_the_bars_and_the_tempo_make_it() {
    // a law: for any tempo and end bar, `length == the exact seconds ± t` holds exactly when |actual - wanted| <= t. The
    // exact seconds are bars * 240 / bpm (integer BPM here), kept as a fraction.
    let mut rng = Rng(0x1e46_7a11_0000_0003);
    for n in 0..120 {
        let bpm = 60 + rng.below(140) as i64;
        let bars = 1 + rng.below(200) as i64;
        let actual = Rat::new(bars * 240, bpm).unwrap();
        // a wanted length and a tolerance, in milliseconds: the answer follows from the fractions
        let want_ms = (actual.to_f64() * 1000.0) as i64 + rng.below(4001) as i64 - 2000;
        let tol_ms = rng.below(3001) as i64;
        let diff = Rat::new(want_ms, 1000).unwrap().sub(actual).unwrap();
        let expect = diff.to_f64().abs() <= tol_ms as f64 / 1000.0;
        // stay clear of the boundary, where a float and a fraction may differ by their rounding
        if (diff.to_f64().abs() - tol_ms as f64 / 1000.0).abs() < 1e-6 {
            continue;
        }
        let src = format!("tempo {bpm}\ndeck A: kick x\nend at bar {bars}\nassert length == {want_ms}ms ± {tol_ms}ms\n");
        let c = report(&src);
        assert_eq!(c.assertions[0].status == Status::Pass, expect, "#{n} {src}{}", c.assertions[0].line());
        // the message says the actual length to the millisecond
        let said = &c.assertions[0].message;
        let ms = (actual.to_f64() * 1000.0).round() as i64;
        assert_eq!(said, &format!("the program is {}", djlive::assertion::seconds_text(ms as f64 / 1000.0)), "#{n}");
    }
    // exact: a length that is whole seconds is matched to the nanosecond, at the tempos that make whole bars of it
    for bpm in [60, 80, 96, 120, 240] {
        let bars = 30;
        let secs = bars * 240 / bpm;
        let c = report(&format!("tempo {bpm}\ndeck A: kick x\nend at bar {bars}\nassert length == {secs}s\n"));
        assert_eq!(c.assertions[0].status, Status::Pass, "{bpm}");
    }
}

#[test]
fn programs_without_assertions_are_untouched() {
    // the fixtures pin the canonical text and the CIDs of everything that was before: an `assert` word changes none of it
    let c = ok(SONG);
    assert!(c.assertions.is_empty());
    assert_eq!(c.text, SONG.replace("\n  ", "\n  "));
    let stmts = djlive::parse::parse(SONG).program.stmts;
    assert!(stmts.iter().all(|s| !matches!(s.kind, StmtKind::Assert(_))));
}

#[test]
fn a_let_puts_its_value_in_an_assertion_like_anywhere_else() {
    let c = report(&format!("let loud = 2\nlet half = 1 / 2\nlet n = 1\n{SONG}assert level A < loud * B in climax\nassert level master > half * A\nassert period == n\n"));
    let texts: Vec<&str> = c.assertions.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["assert level A < 2 * B in climax", "assert level master > 0.5 * A", "assert period == 1"]);
    assert_eq!(c.assertions[2].status, Status::Pass);
}

#[test]
fn malformed_assertions_are_errors_with_a_place_never_a_panic() {
    let base = the_program_with_every_assertion();
    let mut rng = Rng(0x0f0f_5e47_00a5_5e47);
    let alphabet: Vec<char> = "abcdeAB mnsdb0123456789±=<>!()+-*/%.,:#\"{}|~?'$ \n".chars().collect();
    for n in 0..1500 {
        let mut chars: Vec<char> = base.chars().collect();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(chars.len() as u64) as usize;
            match rng.below(3) {
                0 => {
                    chars.remove(at);
                }
                1 => chars.insert(at, alphabet[rng.below(alphabet.len() as u64) as usize]),
                _ => chars[at] = alphabet[rng.below(alphabet.len() as u64) as usize],
            }
        }
        let text: String = chars.into_iter().collect();
        for c in [check(&text), check_report(&text, &Env::none())] {
            match c.compiled {
                Some(comp) => {
                    // whatever compiles has a canonical text that compiles to itself
                    let again = check_report(&comp.text, &Env::none()).compiled.unwrap_or_else(|| panic!("#{n}: the canonical text does not compile:\n{}\nfrom\n{text}", comp.text));
                    assert_eq!(again.text, comp.text, "#{n}");
                }
                None => {
                    let errs: Vec<&Diagnostic> = c.diagnostics.iter().filter(|d| d.is_error()).collect();
                    assert!(!errs.is_empty(), "#{n}: refused with no error:\n{text}");
                    assert!(errs.iter().all(|d| d.span.start <= text.len() && d.span.end <= text.len() && !d.message.is_empty()), "#{n}:\n{text}");
                }
            }
        }
    }
}

#[test]
fn a_share_link_carries_the_assertions_and_opening_it_checks_them_again() {
    let comp = ok(&song("assert ends\nassert length == 48s ± 2s\nassert level A < 2 * B in climax\n"));
    let url = djlive::share::encode(&comp, None, false);
    let opened = djlive::share::open(&url, &Env::none()).expect("the link opens");
    assert_eq!(opened.cid, comp.cid);
    let again = opened.check.compiled.expect("it compiles again");
    assert_eq!(again.text, comp.text);
    assert_eq!(again.assertions.len(), 3);
    assert!(again.text.contains("assert length == 48s ± 2s"));
    // the same assertion over another tempo is another program that does not hold
    let other = comp.text.replace("tempo 120", "tempo 100");
    assert!(check(&other).compiled.is_none());
}
