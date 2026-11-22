//! F2 arithmetic (docs/btw/ext-arith.md): every place that takes a number takes arithmetic, a `let` of arithmetic
//! folds to a scalar, a `when` guard keeps or drops a line where it is expanded, and the canonical form keeps
//! single literals as they were and prints any other expression the same way every time.

mod common;

use common::{reformat, Rng};
use djlive::compile::{check, Compiled, Effect, Timing};
use djlive::expr::{eval_str, parse_expr, Cursor, Val};
use djlive::lex::lex;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn text(src: &str) -> String {
    ok(src).text
}

/// The errors of a program that does not compile, as (line, column, message).
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

fn timings(c: &Compiled) -> Vec<Timing> {
    c.controls.iter().map(|c| c.timing).collect()
}

/// What a program does, without where it was written: the nets and, for each control, when it fires and what it
/// sends.
fn behaviour(c: &Compiled) -> (std::collections::BTreeMap<String, String>, Vec<(Timing, Effect, Option<djlive::dur::Dur>)>) {
    (c.net_cids(), c.controls.iter().map(|k| (k.timing, k.effect.clone(), k.hold)).collect())
}

// ---- let

#[test]
fn a_let_of_arithmetic_is_a_scalar_put_in_place() {
    let c = ok("let n = 2 * 4\nat bar n: master: level 0.5\nat bar n + 1: master: level 0.25\n");
    assert_eq!(c.text, "let n = 8\nat bar 8: master: level 0.5\nat bar 9: master: level 0.25\n");
    assert_eq!(timings(&c), vec![Timing::At(8), Timing::At(9)]);
    // lets feed lets, min and max count
    let c = ok("let lo = 4\nlet hi = lo * 2 + 1\nlet top = max(lo, hi) - min(2, 3)\nat bar top: master: level 0.5\n");
    assert_eq!(c.text, "let lo = 4\nlet hi = 9\nlet top = 7\nat bar 7: master: level 0.5\n");
    // a result that is not whole is a decimal the language reads back exactly
    let c = ok("let q = 1 / 4\nmaster: level q\n");
    assert_eq!(c.text, "let q = 0.25\nmaster: level 0.25\n");
    let c = ok("let t = 1 / 3\nlet u = t * 3\ntempo 100 + u\n");
    assert_eq!(c.tempo, Some(101.0), "{}", c.text);
}

#[test]
fn let_r_is_3_times_2_is_arithmetic_now_and_the_old_meanings_are_spelled_out() {
    // the one deliberate change of meaning: this was the degree 3 played twice
    assert_eq!(text("let r = 3*2\ndeck A: bass r\n"), "let r = 6\ndeck A: bass 6\n");
    // the pattern it used to mean
    assert_eq!(text("let r = 3 3\ndeck A: bass r\n"), "let r = 3 3\ndeck A: bass r\n");
    assert_eq!(text("let r = [3 3]\ndeck A: bass r\n"), "let r = [3 3]\ndeck A: bass r\n");
    assert_eq!(text("let r = 3*2 4\ndeck A: bass r\n"), "let r = 3*2 4\ndeck A: bass r\n", "arithmetic that is not the whole value is a pattern");
    // a lone number, a word, a note: as before
    assert_eq!(text("let n = 8\nlet w = c2\nat bar n: master: level 0.5\ndeck A: bass w\n"), "let n = 8\nlet w = c2\nat bar 8: master: level 0.5\ndeck A: bass c2\n");
}

#[test]
fn a_let_that_is_arithmetic_and_wrong_says_where() {
    let e = errors("let n = 1 / 0\ntempo 120\n");
    assert_eq!((e[0].0, e[0].1), (1, 13), "{e:?}");
    assert!(e[0].2.contains("division by zero"), "{e:?}");
    let e = errors("let n = (1 + 2\ntempo 120\n");
    assert_eq!(e[0].0, 1, "{e:?}");
    assert!(e[0].2.contains("`)`"), "{e:?}");
    // too precise to be a plain number put in place of a name
    let e = errors("let n = 1 / 7000000\ntempo 120\n");
    assert!(e[0].2.contains("cannot be written"), "{e:?}");
    // a name of the language is no name for it
    let e = errors("let kick = 1 + 2\ntempo 120\n");
    assert!(e[0].2.contains("cannot be a name"), "{e:?}");
}

// ---- numbers wherever a number goes

#[test]
fn every_place_that_takes_a_number_takes_arithmetic() {
    let c = ok("tempo 60 + 6 * 2\nswing 1 / 8 + 1 / 8\ndeck A: kick x ~ x ~\n");
    assert_eq!(c.tempo, Some(72.0));
    assert_eq!(c.swing, Some(0.25));
    // arithmetic is the number it comes to, as a literal of that value is spelled: `0.25` is one program however it is reached (L-08)
    assert_eq!(c.text, "tempo 72\nswing 0.25\ndeck A: kick x ~ x ~\n");
    assert_eq!(c.cid, ok("tempo 72\nswing 0.25\ndeck A: kick x ~ x ~\n").cid);

    // params: ranges, plain values, calls
    let c = ok("deck A: bass c2 e2 vel=(60 + 12)..100 oct=min(3, 4) len=1 + 1\n");
    assert_eq!(c.text, "deck A: bass c2 e2 len=2 oct=3 vel=72..100\n");
    let part = &c.decks.values().next().unwrap().parts[0];
    assert_eq!(part.vel_range, Some((72, 100)));

    // time words
    let c = ok("at bar 2 + 2: master: level 0.5\nevery 2 * 2 bars from 1 + 1: master: level 0.5\nat bar 2 for 2 * 2 bars: master: filter lp 800\nmaster: filter lp 100 * 3 -> 4k / 2 over 2 * 4 bars\n");
    assert_eq!(c.text, "at bar 4: master: level 0.5\nevery 4 bars from 2: master: level 0.5\nat bar 2 for 4 bars: master: filter lp 800\nmaster: filter lp 300 -> 2000 over 8 bars\n");
    assert_eq!(timings(&c)[..2], [Timing::At(4), Timing::EveryFrom(4, 2)]);

    // repeat counts, every, section lengths, end
    let c = ok("repeat 2 + 1 every 2 * 2 bars {\n  master: delay 0.5\n}\n");
    assert_eq!(timings(&c), vec![Timing::Now, Timing::At(4), Timing::At(8)]);
    let c = ok("section drop 4 * 4 bars {\n  deck A: kick x\n}\nsection outro 2 + 2 bars {\n  deck A: kick x x\n}\narrange drop outro\nend at bar 10 + 10\n");
    assert_eq!(c.text, "section drop 16 bars {\n  deck A: kick x\n}\nsection outro 4 bars {\n  deck A: kick x x\n}\narrange drop outro\nend at bar 20\n");

    // `do` values, ramps of them, and a ratio that stays a ratio
    let c = ok("do vocoder set mix=0.25 * 2 bands=2 * 8\ndo vocoder set mix=0 + 0 -> 1 over 2 + 2 bars\nwheel sync deck=A ratio=1/4\n");
    assert_eq!(c.text, "do vocoder set bands=16 mix=0.5\ndo vocoder set mix=0 -> 1 over 4 bars\nwheel sync deck=A ratio=1/4\n");

    // generator arguments, a computed degree, a computed repeat count
    let a = ok("deck A: kick e(2 + 1, 4 * 2) | bass 1 (1 + 2) (2 * 2) c2 (3 - 1) | hihat x*(1 + 1)\n");
    let b = ok("deck A: kick e(3, 8) | bass 1 3 4 c2 2 | hihat x*2\n");
    assert_eq!(a.net_cids(), b.net_cids());
    assert_eq!(a.text, "deck A: kick e(3, 8) | bass 1 3 4 c2 2 | hihat x*2\n");
    assert_eq!(a.cid, b.cid);
}

#[test]
fn a_single_literal_keeps_its_spelling_and_its_exact_value() {
    // `2k` and `50%` are 2000 and 0.5; a long decimal is not squeezed through a rational
    assert_eq!(text("master: filter lp 2k\nmaster: level 50%\nmaster: delay 0.123456789\n"), "master: filter lp 2000\nmaster: level 0.5\nmaster: delay 0.123456789\n");
    // parentheses round a lone number are no expression
    assert_eq!(text("master: level (0.5)\nat bar (4): master: level 0.5\n"), "master: level 0.5\nat bar 4: master: level 0.5\n");
    // a ratio in a `do` is still a word, and still a word after canonical form and reading it again
    let c = ok("wheel interval beats=1/2\nwheel sync deck=A ratio=3 / 2\n");
    assert_eq!(c.text, "wheel interval beats=1/2\nwheel sync deck=A ratio=3/2\n");
    // arithmetic that happens to be a short decimal is that number (`0.25`); one that is no decimal of at most six places keeps its
    // parentheses so that it stays a number and not the registry's ratio word
    let c = ok("do vocoder set mix=(1/4)\n");
    assert_eq!(c.text, "do vocoder set mix=0.25\n");
    assert_eq!(text(&c.text), c.text);
    let c = ok("do vocoder set mix=(1/3)\n");
    assert_eq!(c.text, "do vocoder set mix=(1 / 3)\n");
    assert_eq!(text(&c.text), c.text);
}

#[test]
fn the_things_that_were_not_numbers_are_still_not_swallowed() {
    // `0 -5` side by side in an evolve is two numbers, not a subtraction
    let src = "section drop 8 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 4 bars {\n    bass: transpose <0 -5 7> | rotate 1\n    stop when gen == 1\n  }\n}\narrange drop\n";
    assert!(check(src).ok(), "{:?}", check(src).diagnostics);
    // `x*8`, `3*2` and a row of numbers stay what they were
    assert_eq!(text("deck A: hihat x*8 vel=60..90 | bass 1 3*2 5\n"), "deck A: hihat x*8 vel=60..90 | bass 1 3*2 5\n");
    // a step word before a `(` is a step, not a call
    assert_eq!(text("deck A: bass c2 (1 + 1) x (3)\n"), "deck A: bass c2 2 x 3\n");
    // a generator call is a call
    assert!(check("deck A: kick e(3, 8)\n").ok());
}

#[test]
fn a_def_argument_that_is_arithmetic_keeps_its_grouping() {
    let src = "def cuts(a) {\n  at bar $a * 2: master: level 0.5\n}\ncuts(1 + 2)\ncuts(4)\n";
    let c = ok(src);
    assert_eq!(timings(&c), vec![Timing::At(6), Timing::At(8)], "(1 + 2) * 2, not 1 + 2 * 2");
    let src = "def fades(a, b) {\n  repeat $a * $b {\n    master: delay 0.5\n  }\n}\nfades(1 + 1, 2 + 1)\n";
    assert_eq!(ok(src).controls.len(), 6);
    // a ratio argument is still a ratio word
    let c = ok("def locks(r) {\n  wheel sync deck=A ratio=$r\n}\nlocks(1/4)\n");
    assert!(c.text.contains("locks(1/4)"), "{}", c.text);
}

#[test]
fn repeat_with_arithmetic_is_the_same_program_as_the_lines_written_out() {
    let by_hand: String = (0..5).map(|i| format!("at bar {}: master: level {}\n", (i + 1) * 3 - 1, f64::from(i + 1) / 10.0)).collect();
    let src = "repeat 5 {\n  at bar ($i + 1) * 3 - 1: master: level ($i + 1) / 10\n}\n";
    assert_eq!(behaviour(&ok(src)), behaviour(&ok(&by_hand)), "\n{by_hand}");
    // $n too, and a nested expression with min / max and a remainder; `every 2 bars` shifts copy i by 2 i bars
    let src = "repeat 4 every 2 bars {\n  at bar min($i + 1, $n - 2) + $i % 2: master: delay 1 / $n\n}\n";
    let by_hand: String = (0..4i64).map(|i| format!("at bar {}: master: delay 0.25\n", (i + 1).min(4 - 2) + i % 2 + 2 * i)).collect();
    assert_eq!(behaviour(&ok(src)), behaviour(&ok(&by_hand)), "\n{by_hand}");
}

// ---- guards

#[test]
fn a_guard_keeps_or_drops_a_line_where_it_is_expanded() {
    let c = ok("repeat 8 {\n  at bar $i + 1: master: level 0.5 when $i % 4 == 3\n}\n");
    assert_eq!(timings(&c), vec![Timing::At(4), Timing::At(8)]);
    // a body prints as the tokens it was written in, and is a fixed point
    assert_eq!(c.text, "repeat 8 {\n  at bar $i + 1: master: level 0.5 when $i % 4==3\n}\n");
    assert_eq!(ok(&c.text).text, c.text);
    // after a part line, a `do` line, a call; `!=` `<=` `>=` `and` `or` `not`
    let c = ok("def picks(deck, k) {\n  deck $deck: kick x ~ x ~ when $deck == A and $k >= 2\n  at bar 2: do vocoder on when not ($k < 2) and $deck != A\n}\npicks(A, 3)\npicks(B, 1)\npicks(B, 2)\n");
    let decks: Vec<_> = c.decks.keys().map(|d| d.name()).collect();
    assert_eq!(decks, vec!["A"]);
    assert_eq!(c.controls.len(), 1, "only picks(B, 2) keeps its `do`: {:?}", c.controls.iter().map(|c| &c.key).collect::<Vec<_>>());
    // words compare with == and !=
    let c = ok("def wipes(deck) {\n  at bar 1: master: level 0.5 when $deck == B\n  at bar 2: master: level 0.5 when $deck != B\n}\nwipes(A)\nwipes(B)\n");
    assert_eq!(timings(&c), vec![Timing::At(2), Timing::At(1)]);
    // a guard after a def's call drops the whole call
    let c = ok("def hush() {\n  master: level 0.5\n}\nrepeat 4 {\n  hush() when $i == 2\n}\n");
    assert_eq!(c.controls.len(), 1);
}

#[test]
fn a_false_guard_drops_the_line_before_it_is_checked_or_compiled() {
    // a second kick would be an error, a bar 0 and an unknown section too: none is looked at
    let c = ok("deck A: kick x\ndeck A: kick x ~ when 1 == 2\nat bar 0: master: level 0.5 when 1 == 2\nbefore ghost 2 bars: master: level 0.5 when 1 > 2\nswap pairs when 2 < 1\n");
    assert!(c.controls.is_empty());
    // ... and kept, the same line is the error it always was
    let e = errors("deck A: kick x\ndeck A: kick x ~ when 1 == 1\n");
    assert!(e[0].2.contains("already has a `kick`"), "{e:?}");
    let e = errors("at bar 0: master: level 0.5 when 1 == 1\n");
    assert!(e[0].2.contains("bars are counted from 1"), "{e:?}");
    // in a repeat, a line that names a section that is not there in this copy (here, in none) is only an error when kept
    let c = ok("repeat 3 {\n  before nosuch 2 bars: master: level 0.5 when $i == 99\n}\n");
    assert!(c.controls.is_empty());
    let e = errors("repeat 3 {\n  before nosuch 2 bars: master: level 0.5 when $i == 1\n}\n");
    assert!(e[0].2.contains("no section `nosuch`"), "{e:?}");
}

#[test]
fn a_guard_at_the_top_is_a_constant_kept_as_written() {
    let c = ok("deck A: kick x ~ x ~ when 1 == 1\nat bar 2: master: level 0.5 when 2 + 2 == 4\n");
    // (its arithmetic is the number it comes to, as everywhere: the comparison stays)
    assert_eq!(c.text, "deck A: kick x ~ x ~ when 1 == 1\nat bar 2: master: level 0.5 when 4 == 4\n");
    assert_eq!(c.controls.len(), 1);
    // the guard is not part of what identifies a control, so editing a guard that still holds keeps its state
    let a = ok("at bar 2: master: level 0.5 when 1 == 1\n");
    let b = ok("at bar 2: master: level 0.5 when 2 == 2\n");
    assert_eq!(a.controls[0].key, b.controls[0].key);
    // `when` is not a step: a pattern before a guard ends there
    let c = ok("deck A: kick x ~ x ~ vel=90 when 1 == 1\n");
    assert_eq!(c.decks.values().next().unwrap().parts[0].cycles[0].len(), 2);
}

#[test]
fn guard_errors_say_where() {
    let e = errors("deck A: kick x ~ x ~ when 1\n");
    assert_eq!((e[0].0, e[0].1), (1, 27), "{e:?}");
    assert!(e[0].2.contains("not a condition"), "{e:?}");
    let e = errors("at bar 2: master: level 0.5 when $i == 1\n");
    assert!(e[0].2.contains("`$i` has no value here"), "{e:?}");
    let e = errors("repeat 2 {\n  at bar 1: master: level 0.5 when A < B\n}\n");
    assert!(e[0].2.contains("words can be compared with `==` and `!=`"), "{e:?}");
    let e = errors("at bar 1: master: level 0.5 when 1 / 0 == 1\n");
    assert!(e[0].2.contains("division by zero"), "{e:?}");
    let e = errors("at bar 1: master: level 0.5 when\n");
    assert!(e[0].2.contains("expected a number or a name"), "{e:?}");
    // a guard belongs to a line, not a setting
    let e = errors("tempo 120 when 1 == 1\n");
    assert!(e[0].2.contains("expected the end of the line"), "{e:?}");
    // an evolve's own `when gen` is what it was
    assert!(check("section drop 8 bars {\n  deck A: bass [a2 e2]\n  evolve every 4 bars {\n    bass: invert when gen % 2 == 1\n    stop when gen == 1\n  }\n}\narrange drop\n").ok());
}

#[test]
fn unbound_names_and_bad_arithmetic_are_errors_with_help() {
    let c = check("tempo $i\n");
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.message.contains("`$i` has no value here"), "{d:?}");
    assert!(d.help.as_deref().unwrap_or("").contains("repeat"), "{d:?}");
    assert_eq!(d.position("tempo $i\n").0, 1);
    for (src, want) in [
        ("tempo 200 / 0\n", "division by zero"),
        ("tempo 200 % 0\n", "modulo by zero"),
        ("tempo 1 +\n", "expected a number or a name"),
        ("tempo (1 + 2\n", "expected `)`"),
        ("tempo foo + 1\n", "expected a tempo"),
        ("tempo 9223372036854775807 * 9223372036854775807\n", "too large"),
        ("tempo knob(3)\n", "expected a tempo"),
        ("deck A: kick x*(1 / 2)\n", "a whole number"),
        ("deck A: bass (1 - 2)\n", "scale degree"),
        ("deck A: kick x*(3 * 100)\n", "1 to 64"),
        ("master: level 1 / 0\n", "division by zero"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|(_, _, m)| m.contains(want)), "{src:?}: wanted `{want}`, got {e:?}");
    }
    // an error at the end of the text points at the last token, with a line
    let e = errors("tempo 1 +\n");
    assert_eq!((e[0].0, e[0].1), (1, 9), "{e:?}");
}

// ---- marks

#[test]
fn spans_survive_arithmetic() {
    let src = "deck A: kick x ~ x ~ vel=(60 + 12)..100\nat bar 2 * 2: master: level 0.5 when 1 == 1\n";
    let c = ok(src);
    let part = &c.decks.values().next().unwrap().parts[0];
    // each hit of the part knows its token and its line
    for h in &part.cycles[0] {
        let spans = &part.spans[h.sid as usize];
        assert_eq!(spans[0].text(src), "x");
        assert!(spans.iter().any(|s| s.text(src).starts_with("kick")));
    }
    // a control's span is its whole statement, guard and all
    assert_eq!(c.controls[0].span.text(src), "at bar 2 * 2: master: level 0.5 when 1 == 1");
}

// ---- the canonical form

#[test]
fn canonical_expressions_are_a_fixed_point_and_a_reformatting_keeps_the_cid() {
    let programs = [
        "tempo 60 + 6 * 2\nswing 1 / 8 + 1 / 8\nkey am\ndeck A: kick x ~ x ~ vel=(60 + 12)..(100 + 20) | bass c2 (1 + 2) x*(1 + 1) oct=min(3, 4)\n",
        "let n = 2 * 4\nat bar n + (4 - 2) * 3: master: level 1 / 2 * (1 / 2)\nevery n / 2 bars: master: filter lp 100 * (2 + 1) -> 8k - 1 over 2 * 4 bars when 1 + 1 == 2 and not 3 < 2\n",
        "def cuts(lo, hi) {\n  at bar $lo * $hi + 1: master: level ($lo + 1) / 10 when $lo % 2 == 0 and $hi != 3\n}\nrepeat 3 every 1 + 1 bars {\n  cuts($i, $n - 1)\n}\n",
        "section drop 4 * 4 bars {\n  deck A: kick x*(2 + 2)\n  at bar 1 + 1 for 2 * 1 bars: master: filter lp 800\n}\narrange drop\nend at bar 4 * 4\n",
        "do vocoder set mix=0.25 * 2 -> 1 over 2 * 2 bars\nwheel sync deck=A ratio=1/4\nwheel interval beats=(1/2)\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let c = ok(src);
        let again = ok(&c.text);
        assert_eq!(again.text, c.text, "program {n}: the canonical text is not a fixed point");
        assert_eq!(again.cid, c.cid, "program {n}");
        let mut rng = Rng(0xA217_0000 + n as u64);
        for round in 0..120 {
            let t = reformat(src, &mut rng);
            let got = check(&t);
            let got = got.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{t}\n{}", djlive::diag::render_all(&got.diagnostics, &t)));
            assert_eq!(got.cid, c.cid, "program {n} round {round} forked:\n{t}");
        }
    }
}

#[test]
fn a_semantic_edit_changes_the_cid() {
    let a = ok("tempo 60 + 6\n").cid;
    assert_ne!(a, ok("tempo 60 + 7\n").cid);
    assert_eq!(a, ok("tempo 60 + 6 + 0\n").cid, "the same number is the same program, however it is reached (L-08)");
    assert_eq!(ok("tempo 66\n").tempo, ok("tempo 60 + 6\n").tempo);
    assert_eq!(ok("tempo 66\n").cid, a, "`tempo 66` and `tempo 60 + 6` are one program");
    let g = ok("repeat 4 {\n  at bar $i + 1: master: level 0.5 when $i % 4 == 3\n}\n").cid;
    assert_ne!(g, ok("repeat 4 {\n  at bar $i + 1: master: level 0.5 when $i % 4 == 2\n}\n").cid);
}

// ---- properties

/// A reference evaluator, written independently of `rat.rs`: fractions over i128.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fr(i128, i128);

fn gcd(a: i128, b: i128) -> i128 {
    if b == 0 {
        a.abs()
    } else {
        gcd(b, a % b)
    }
}

impl Fr {
    fn new(n: i128, d: i128) -> Fr {
        let g = gcd(n, d).max(1);
        let (n, d) = (n / g, d / g);
        if d < 0 {
            Fr(-n, -d)
        } else {
            Fr(n, d)
        }
    }
    fn add(self, o: Fr) -> Fr {
        Fr::new(self.0 * o.1 + o.0 * self.1, self.1 * o.1)
    }
    fn sub(self, o: Fr) -> Fr {
        Fr::new(self.0 * o.1 - o.0 * self.1, self.1 * o.1)
    }
    fn mul(self, o: Fr) -> Fr {
        Fr::new(self.0 * o.0, self.1 * o.1)
    }
    fn div(self, o: Fr) -> Option<Fr> {
        (o.0 != 0).then(|| Fr::new(self.0 * o.1, self.1 * o.0))
    }
    fn floor(self) -> i128 {
        self.0.div_euclid(self.1)
    }
    fn rem(self, o: Fr) -> Option<Fr> {
        let q = self.div(o)?;
        Some(self.sub(o.mul(Fr::new(q.floor(), 1))))
    }
    fn lt(self, o: Fr) -> bool {
        self.0 * o.1 < o.0 * self.1
    }
    fn show(self) -> String {
        if self.1 == 1 {
            format!("{}", self.0)
        } else {
            format!("{}/{}", self.0, self.1)
        }
    }
    fn to_f64(self) -> f64 {
        self.0 as f64 / self.1 as f64
    }
}

#[derive(Clone, Debug)]
enum E {
    N(i64),
    Neg(Box<E>),
    Bin(char, Box<E>, Box<E>),
    Min(Vec<E>),
    Max(Vec<E>),
}

fn gen(rng: &mut Rng, depth: u32) -> E {
    if depth == 0 || rng.below(4) == 0 {
        return E::N(rng.below(10) as i64);
    }
    match rng.below(9) {
        0 => E::Neg(Box::new(gen(rng, depth - 1))),
        1 => E::Min((0..1 + rng.below(3)).map(|_| gen(rng, depth - 1)).collect()),
        2 => E::Max((0..1 + rng.below(3)).map(|_| gen(rng, depth - 1)).collect()),
        k => E::Bin(['+', '-', '*', '/', '%', '+', '*'][(k - 3) as usize], Box::new(gen(rng, depth - 1)), Box::new(gen(rng, depth - 1))),
    }
}

fn reference(e: &E) -> Option<Fr> {
    Some(match e {
        E::N(n) => Fr::new(i128::from(*n), 1),
        E::Neg(x) => Fr::new(0, 1).sub(reference(x)?),
        E::Bin(op, a, b) => {
            let (x, y) = (reference(a)?, reference(b)?);
            match op {
                '+' => x.add(y),
                '-' => x.sub(y),
                '*' => x.mul(y),
                '/' => x.div(y)?,
                _ => x.rem(y)?,
            }
        }
        E::Min(v) | E::Max(v) => {
            let vals = v.iter().map(reference).collect::<Option<Vec<_>>>()?;
            let is_min = matches!(e, E::Min(_));
            let mut best = vals[0];
            for x in &vals[1..] {
                if (is_min && x.lt(best)) || (!is_min && best.lt(*x)) {
                    best = *x;
                }
            }
            best
        }
    })
}

fn prec(op: char) -> u8 {
    if matches!(op, '+' | '-') {
        1
    } else {
        2
    }
}

/// Text for `e` with random spacing and, now and then, parentheses nobody needed.
fn render(e: &E, outer: u8, rng: &mut Rng) -> String {
    let sp = |rng: &mut Rng| " ".repeat(1 + rng.below(2) as usize);
    match e {
        E::N(n) => n.to_string(),
        E::Neg(x) => match **x {
            E::N(n) => format!("-{n}"),
            _ => format!("-({})", render(x, 0, rng)),
        },
        E::Min(v) | E::Max(v) => {
            let args: Vec<String> = v.iter().map(|x| render(x, 0, rng)).collect();
            format!("{}({})", if matches!(e, E::Min(_)) { "min" } else { "max" }, args.join(&format!(",{}", if rng.below(2) == 0 { " " } else { "" })))
        }
        E::Bin(op, a, b) => {
            let p = prec(*op);
            let s = format!("{}{}{}{}{}", render(a, p, rng), sp(rng), op, sp(rng), render(b, p + 1, rng));
            if p < outer || rng.below(5) == 0 {
                format!("({s})")
            } else {
                s
            }
        }
    }
}

#[test]
fn expressions_equal_the_reference_evaluator_and_their_canonical_text_is_a_fixed_point() {
    let mut rng = Rng(0xE5C0_DE00_0000_0001);
    let mut checked = 0;
    for _ in 0..1500 {
        let e = gen(&mut rng, 4);
        let Some(want) = reference(&e) else { continue };
        let src = render(&e, 0, &mut rng);
        match eval_str(&src) {
            Ok(Val::Num(got)) => assert_eq!(got.to_string(), want.show(), "{src}"),
            other => panic!("{src}: {other:?}, wanted {}", want.show()),
        }
        // canonical text: reading it back gives the same text and the same value
        let read = |s: &str| {
            let lexed = lex(s);
            let mut c = Cursor { toks: &lexed.tokens, at: 0 };
            parse_expr(&mut c).unwrap_or_else(|d| panic!("{s}: {}", d.message))
        };
        let canon = read(&src).canonical();
        let again = read(&canon);
        assert_eq!(again.canonical(), canon, "{src}");
        match eval_str(&canon) {
            Ok(Val::Num(got)) => assert_eq!(got.to_string(), want.show(), "{src} -> {canon}"),
            other => panic!("{canon}: {other:?}"),
        }
        checked += 1;
    }
    assert!(checked > 700, "{checked} expressions checked");
}

#[test]
fn a_tempo_that_is_an_expression_is_the_reference_value_through_the_whole_language() {
    let mut rng = Rng(0x7E3_0000_0000_0042);
    let mut checked = 0;
    for _ in 0..600 {
        let e = gen(&mut rng, 3);
        let Some(v) = reference(&e) else { continue };
        let Some(r) = v.rem(Fr::new(50, 1)) else { continue };
        let want = Fr::new(100, 1).add(r);
        let src = format!("tempo 100 + ({}) % 50\ndeck A: kick x ~ x ~\n", render(&e, 0, &mut rng));
        let c = ok(&src);
        assert_eq!(c.tempo, Some(want.to_f64()), "{src}");
        // and in `let`: the same value through a scalar
        let via_let = format!("let t = {}\ntempo 100 + t % 50\ndeck A: kick x ~ x ~\n", render(&e, 0, &mut rng));
        if let Some(c2) = check(&via_let).compiled {
            assert_eq!(c2.tempo, Some(want.to_f64()), "{via_let}");
        }
        // the canonical text says the same and is the same program
        let again = ok(&c.text);
        assert_eq!((again.cid.as_str(), again.tempo), (c.cid.as_str(), c.tempo), "{src}\n{}", c.text);
        let t = reformat(&src, &mut rng);
        assert_eq!(ok(&t).cid, c.cid, "{t}");
        checked += 1;
    }
    assert!(checked > 250, "{checked}");
}

#[test]
fn the_example_does_what_its_comments_say() {
    let c = ok(include_str!("../../../examples/btw/arith.btw"));
    assert_eq!((c.tempo, c.swing), (Some(128.0), Some(0.125)));
    // the filter opens on copies 0 and 2 only, the reverb on every copy; `pulse(B, 3)` is dropped by its guard
    assert_eq!(c.controls.len(), 6, "{:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    let seg = &c.segments[0];
    assert_eq!(seg.bars, 16);
    let hat = seg.decks.values().flat_map(|d| d.parts.iter()).find(|p| p.role == "hihat").unwrap();
    assert_eq!(hat.vel_range, Some((84, 100)));
    assert_eq!(hat.cycles[0].len(), 4, "x*(4 + (1 + 1) % 2 * 4) is x*4");
    assert!(c.text.starts_with("tempo 128\nkey am\nswing 0.125\nlet total = 16\nlet base = 72\n"), "{}", c.text);
}

#[test]
fn counts_of_an_evolve_and_a_queue_take_arithmetic_too() {
    let c = ok("section drop 16 bars {\n  deck A: bass [a2 e2]\n  evolve every 2 * 2 bars {\n    bass: transpose 2\n    stop when gen == 1 + 2\n  }\n}\narrange drop\nqueue jump 1 + 1\nload pair dnb:2602 group 1 + 1\n");
    assert!(c.text.contains("evolve every 4 bars {") && c.text.contains("stop when gen == 3"), "{}", c.text);
    assert!(c.text.contains("queue jump 2") && c.text.contains("group 2"), "{}", c.text);
    assert_eq!(c.lineage.len(), 4);
    // a guard that calls a function there is none of says what it is (`knob(3)` is an input now, F13: tests/ext_perform.rs)
    let e = errors("at bar 2: master: level 0.5 when foo(3) > 0\n");
    assert!(e[0].2.contains("not a function of arithmetic"), "{e:?}");
}

#[test]
fn a_def_with_a_part_line_takes_arithmetic_in_its_params_and_generators() {
    let src = "def riffs(k) {\n  deck A: bass 1 ($k + 1) 5 e($k % 4 + 1, 8) vel=(60 + $k * 4)..100 oct=$k % 2 + 2\n}\nriffs(3)\n";
    let a = ok(src);
    let b = ok("deck A: bass 1 4 5 e(4, 8) vel=72..100 oct=3\n");
    assert_eq!(a.net_cids(), b.net_cids());
    // the same def called with an arithmetic argument
    let a = ok(&src.replace("riffs(3)", "riffs(1 + 2)"));
    assert_eq!(a.net_cids(), b.net_cids());
    // a bad guard in a repeat is said once
    let e = errors("repeat 4 {\n  master: level 0.5 when 1\n}\n");
    assert_eq!(e.len(), 1, "{e:?}");
}
