//! Features that meet, second file: arithmetic (F2), guards and sub-bar timing (F1) against ports (F11), euclid and
//! polymeter (F6), chance (F7) and imports. The tests the integrator writes when two branches that were built apart land
//! on one; each names the features it joins. (`ext_cross.rs` is the first file; each feature's own tests are in
//! `ext_<feature>.rs`.)

mod common;

use common::{reformat, Rng};
use djlive::compile::{check_with, Timing};
use djlive::dur::Dur;
use djlive::env::{Env, StaticSnippets};
use djlive::{check, print};

fn ok(src: &str) -> djlive::compile::Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String)> {
    let c = check_with(src, &Env::none());
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone())).collect()
}

fn said(e: &[(usize, usize, String)], what: &str) -> bool {
    e.iter().any(|(_, _, m)| m.contains(what))
}

fn lines_of(src: &str, env: &Env) -> Vec<String> {
    let parsed = djlive::parse::parse_with(src, env);
    assert!(parsed.ok(), "{}", djlive::diag::render_all(&parsed.diagnostics, src));
    let mut diags = Vec::new();
    let lw = djlive::expand::lower(&parsed, src, &mut diags);
    assert!(!diags.iter().any(|d| d.is_error()), "{}", djlive::diag::render_all(&diags, src));
    let mut out: Vec<String> = lw.globals.iter().map(|f| print::line(&f.line)).collect();
    for s in &lw.segments {
        out.extend(s.lines.iter().map(|f| print::line(&f.line)));
    }
    out
}

/// Every flat line the program expands to, printed, in order.
fn lines(src: &str) -> Vec<String> {
    lines_of(src, &Env::none())
}

/// A store of saved snippets (each may import the ones before it).
fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let c = check_with(text, &env);
        let canon = c.compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, text)));
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    snips
}

fn stable(src: &str, seed: u64) {
    let c = ok(src);
    let again = ok(&c.text);
    assert_eq!(again.text, c.text, "the canonical text is not a fixed point");
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
}

// ---- F2 arithmetic and guards through F11 ports, defs, repeats

/// A def with three wired inputs (a rhythm, a number of bars and a tempo, the last two arithmetic), called from a repeat; its
/// lines hold arithmetic, `$i` and guards.
const WIRED: &str = "\
tempo 120
key am
def kicks() out: rhythm {
  emit e(3, 8)
}
def span_of() out: bars {
  emit 2 * 4
}
def cutoff() out: tempo {
  emit 100 + 20
}
def lay(i) in: rhythm, bars, tempo {
  deck A: kick $rhythm vel=(60 + $i * 4)..100 when $i == 0
  at bar $bars + $i * 2 + 1: master: filter lp $tempo * ($i + 1) when $i % 4 == 3
  at bar $bars + 1: master: reverb 1 / 4 + $i / 8 when $i > 1 and $i != 2
}
wire kicks.out -> lay.rhythm
wire span_of.out -> lay.bars
wire cutoff.out -> lay.tempo
repeat 4 { lay($i) }
";

#[test]
fn arithmetic_and_guards_in_a_def_that_takes_wired_ports_called_from_a_repeat() {
    // F2 x F11 x repeat: the arithmetic wired in (`2 * 4`, `100 + 20`) keeps its grouping where it is put in place (so it comes to
    // the right number, folded in the canonical text as everywhere), the guard of each copy sees its own `$i`, and the copies
    // that fail it are not there
    let l = lines(WIRED);
    assert_eq!(l, vec!["deck A: kick e(3, 8) vel=60..100", "at bar 15: master: filter lp 480", "at bar 9: master: reverb 0.625"], "{l:#?}");
    let c = ok(WIRED);
    assert_eq!(c.controls.len(), 2);
    let timings: Vec<_> = c.controls.iter().map(|k| k.timing.clone()).collect();
    assert_eq!(timings, vec![Timing::At(15), Timing::At(9)]);
    stable(WIRED, 0xf2_f11);
}

#[test]
fn a_wired_value_that_is_arithmetic_reads_back_the_same_in_two_places() {
    // the same wired number in a count and in a position: `2 * 4` is eight bars in both, never `2 * (4 + 1)`
    let src = "\
tempo 120
def span_of() out: bars {
  emit 2 * 4
}
def late() in: bars {
  at bar $bars * 2 + 1: master: reverb 0.2
  at bar 1 + $bars: master: delay 0.1
}
wire span_of.out -> late.bars
late()
";
    assert_eq!(lines(src), vec!["at bar 17: master: reverb 0.2", "at bar 9: master: delay 0.1"]);
    stable(src, 0xf2_f11_2);
}

#[test]
fn a_guard_that_is_false_drops_the_call_of_a_def_with_wired_ports() {
    // the copy of the call that fails its guard is not there, the one that holds expands with its wired value
    let src = "\
tempo 120
def pulse() out: rhythm {
  emit x ~ x ~
}
def lay() in: rhythm {
  deck A: kick $in
}
wire pulse.out -> lay.in
repeat 2 { lay() when $i == 1 }
";
    assert_eq!(lines(src), vec!["deck A: kick [x ~ x ~]"]);
}

// ---- F2 guards in a def called from a repeat

#[test]
fn a_guard_on_the_index_inside_a_def_called_from_a_repeat() {
    let src = "\
tempo 124
def tick(i) {
  at bar $i + 1: master: filter lp 200 * ($i + 1) when $i % 4 == 3
  every 1/2 bar from $i + 1: deck B: level 0.5 when $i == 0
}
repeat 8 { tick($i) }
";
    let l = lines(src);
    assert_eq!(l, vec!["every 2 beats from 1: deck B: level 0.5", "at bar 4: master: filter lp 800", "at bar 8: master: filter lp 1600"], "{l:#?}");
    let c = ok(src);
    let at: Vec<_> = c.controls.iter().map(|k| k.timing.clone()).collect();
    assert_eq!(at.len(), 3);
    assert!(matches!(at[0], Timing::EveryPos { .. }) && at[1] == Timing::At(4) && at[2] == Timing::At(8), "{at:?}");
    stable(src, 0xf2_f1_a);
}

#[test]
fn a_guard_that_compares_a_def_parameter_with_a_deck_through_a_call_and_a_repeat() {
    let src = "\
tempo 124
def lay(deck, i) {
  deck $deck: hihat x*8 vel=(40 + $i * 10)..100 when $deck == A and $i >= 1
}
repeat 3 { lay(A, $i) }
lay(B, 2)
";
    // only the copy `$i == 1` of deck A would fail the `one part per role` rule if both copies of A held, so the guard is
    // what keeps this program valid: it keeps i = 1 and 2 of A, which is two hihats, hence the error below
    let e = errs(src);
    assert!(said(&e, "already has a `hihat` part"), "{e:?}");
    let src = src.replace("repeat 3 { lay(A, $i) }", "repeat 3 { lay(A, $i) when $i == 2 }");
    assert_eq!(lines(&src), vec!["deck A: hihat x*8 vel=60..100"]);
    stable(&src, 0xf2_dec);
}

// ---- tokens that must touch (F2 `==` `!=` `<=` `>=`, F1 `1/2`, `3m30s`-style units) through a macro and an import

#[test]
fn the_marks_of_a_comparison_in_a_def_argument_still_touch() {
    // a def argument put where `when $cond` stands: `==`, `!=`, `<=`, `>=` are two glued tokens and must stay one
    let src = "\
tempo 120
def gate(c) {
  deck A: kick x ~ x ~ when $c
}
gate(2 == 2)
";
    assert_eq!(lines(src), vec!["deck A: kick x ~ x ~"]);
    for (cond, holds) in [("2 == 2", true), ("2 == 3", false), ("2 != 3", true), ("2 != 2", false), ("2 <= 2", true), ("3 <= 2", false), ("2 >= 2", true), ("1 >= 2", false), ("1 < 2 and 2 > 1", true)] {
        let src = format!("tempo 120\ndef gate(c) {{\n  deck A: kick x ~ x ~ when $c\n}}\ngate({cond})\n");
        assert_eq!(lines(&src).len(), usize::from(holds), "{cond}");
    }
}

#[test]
fn a_guard_in_an_imported_def_still_reads_its_comparison() {
    let snips = snippets(&[(
        "pulses",
        "def pulse(deck, i) {\n  deck $deck: hihat x*8 when $deck == A and $i != 1 and $i <= 2 and $i >= 0\n  deck $deck: snare ~ x ~ x when $deck != A\n}\n",
    )]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"pulses\"\npulse(A, 0)\npulse(B, 0)\n";
    let l = lines_of(src, &env);
    assert_eq!(l, vec!["deck A: hihat x*8", "deck B: snare ~ x ~ x"], "{l:?}");
    let src = "import \"pulses\"\nrepeat 4 {\n  pulse(A, $i) when $i != 0\n}\n";
    let l = lines_of(src, &env);
    assert_eq!(l, vec!["deck A: hihat x*8"], "{l:?}");
}

// ---- F1 sub-bar timing words in an imported def, a repeat and a def with a wired value

const TIMED: &str = "\
def swell(bars_) {
  every 1/2 bar from 3: deck B: level 0.8
  at bar 2 beat 3: master: delay 0.3
  at bar 5 for 3 beats: deck B: mute
  master: filter lp 400 -> 3k over 2 beats
  before drop 1/2 bar: master: reverb 0.1 -> 0.7 over (1 + 1) beats
}
";

#[test]
fn sub_bar_timing_words_in_an_imported_def() {
    let snips = snippets(&[("swell", TIMED)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"swell\"\ntempo 128\nsection intro 4 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 4 bars {\n  deck A: kick x x x x\n  swell(2)\n}\narrange intro drop\n";
    let parsed = djlive::parse::parse_with(src, &env);
    assert!(parsed.ok(), "{}", djlive::diag::render_all(&parsed.diagnostics, src));
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    let comp = c.compiled.unwrap();
    let times: Vec<_> = comp.controls.iter().map(|k| (k.timing.clone(), k.hold)).collect();
    // an exact half bar every time from bar 3 of the section, a beat inside a bar, a hold of three beats, a ramp that is a half bar
    assert!(times.iter().any(|(t, _)| matches!(t, Timing::EveryPos { .. })), "{times:?}");
    assert!(times.iter().any(|(t, _)| matches!(t, Timing::AtPos(_))), "{times:?}");
    assert!(times.iter().any(|(_, h)| *h == Dur::from_beats(djlive::rat::Rat::new(3, 1).unwrap())), "{times:?}");
    // the canonical text of the importer keeps the snippet's CID, and reading it back is the same program
    let again = check_with(&comp.text, &env);
    assert!(again.ok(), "{}\n{}", comp.text, djlive::diag::render_all(&again.diagnostics, &comp.text));
    assert_eq!(again.compiled.unwrap().cid, comp.cid);
}

#[test]
fn an_imported_def_with_timing_words_plays_the_same_as_the_def_written_in_place() {
    let snips = snippets(&[("swell", TIMED)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let body = "tempo 128\nsection drop 4 bars {\n  deck A: kick x x x x\n  swell(2)\n}\nsection intro 2 bars {\n  deck A: kick x ~ x ~\n}\narrange intro drop\n";
    let imported = check_with(&format!("import \"swell\"\n{body}"), &env);
    assert!(imported.ok(), "{}", djlive::diag::render_all(&imported.diagnostics, ""));
    let inline = ok(&format!("{TIMED}{body}"));
    let (imported, inline) = (imported.compiled.unwrap(), inline);
    let shape = |c: &djlive::compile::Compiled| c.controls.iter().map(|k| (k.timing.clone(), k.hold, k.effect.clone())).collect::<Vec<_>>();
    assert_eq!(shape(&imported), shape(&inline));
}

// ---- the slots that took a bare number (F6 `role/N`, F7 `?p` and a choice's weights) take arithmetic (F2), also through a def

#[test]
fn a_def_argument_that_is_arithmetic_goes_into_a_cycle_a_chance_and_a_weight() {
    // a def puts an arithmetic argument in place in parentheses (`$n` with `n = 4 + 8` is `(4 + 8)`): the slots that are written
    // glued to what comes before them (`hihat/12`, `x?0.3`) read that, as `x*(1 + 1)` always did
    let src = "\
tempo 120
def cyc(n) {
  deck A: hihat/$n x*8
}
def odds(p, w, s) {
  deck B: snare x*4?$p choose [x ~ [x x]] weights $w 1 1 seed=$s
}
cyc(4 + 8)
odds(1 / 4 + 1 / 4, 1 + 1, 1 + 1)
";
    assert_eq!(lines(src), vec!["deck A: hihat/12 x*8", "deck B: snare x*4?0.5 choose [x ~ [x x]] weights 2 1 1 seed=2"]);
    stable(src, 0xf6_f7_f2);
    // and written in place: the same program, so the same net
    let net = |src: &str, d: djlive::ast::Deck| ok(src).decks[&d].net_cid();
    let placed = "tempo 120\ndeck A: hihat/(4 + 8) x*8\ndeck B: snare x*4?(1 / 4 + 1 / 4) choose [x ~ [x x]] weights (1 + 1) 1 1 seed=1 + 1\n";
    assert_eq!(net(placed, djlive::ast::Deck::A), net(src, djlive::ast::Deck::A));
    assert_eq!(net(placed, djlive::ast::Deck::B), net(src, djlive::ast::Deck::B));
}

#[test]
fn arithmetic_in_a_cycle_a_chance_and_a_weight_folds_to_the_number_it_is_and_says_what_is_wrong() {
    // canonical: a cycle is steps, a chance and a weight are one number each, so they fold (`hihat/12`, `?0.25`, `weights 2 0`)
    let c = ok("tempo 120\ndeck A: hihat/(4 + 8) x*8?(1 / 4) choose [x ~] weights (1 + 1) (1 - 1) seed=1 + 1\n");
    assert!(c.text.contains("deck A: hihat/12 x*8?0.25 choose [x ~] weights 2 0 seed=2\n"), "{}", c.text);
    // each wrong one is an error at its own place
    let e = errs("tempo 120\ndeck A: hihat/(3 / 2) x*8\n");
    assert!(said(&e, "a cycle is a whole number"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat/(100) x*8\n");
    assert!(said(&e, "a cycle is a whole number of sixteenth steps from 1 to 64"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat/ (4) x*8\n");
    assert!(said(&e, "glued to its role"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat x?(5 / 4)\n");
    assert!(said(&e, "a chance is 0 to 1"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat x?(1 / 3)\n");
    assert!(said(&e, "at most six decimals"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat x? (1 / 4)\n");
    assert!(said(&e, "glued to its `?`"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat choose [x ~] weights (1 - 2) 1\n");
    assert!(said(&e, "not negative"), "{e:?}");
    let e = errs("tempo 120\ndeck A: hihat choose [x ~] weights (1 / 3) 1\n");
    assert!(said(&e, "at most six decimals"), "{e:?}");
}

#[test]
fn a_repeat_index_drives_a_cycle_an_euclid_and_a_chance_under_a_guard() {
    let src = "\
tempo 120
repeat 4 {
  deck A: hihat/($i + 12) e($i % 4 + 1, 8, $i)?(($i + 1) / 8) when $i == 2
}
";
    assert_eq!(lines(src), vec!["deck A: hihat/14 e(3, 8, 2)?0.375"]);
    stable(src, 0xf6_f7_f2_2);
}

// ---- a comparison in the call of a def (F2 `<=`, `>=` against the `<a b>` the call reader counts)

#[test]
fn a_comparison_with_an_equals_sign_in_a_call_is_not_the_bracket_of_an_alternation() {
    let src = "tempo 120\ndef gate(c) {\n  deck A: kick x ~ x ~ when $c\n}\ngate(2 <= 3)\n";
    assert_eq!(lines(src), vec!["deck A: kick x ~ x ~"]);
    let src = "tempo 120\ndef gate(c) {\n  deck A: kick x ~ x ~ when $c\n}\ngate(2 >= 3)\n";
    assert!(lines(src).is_empty());
    // a lone `<` is still the bracket of `<a b>`; the error says what to write instead
    let e = errs("tempo 120\ndef gate(c) {\n  deck A: kick x ~ x ~ when $c\n}\ngate(2 < 3)\n");
    assert!(e.iter().any(|(_, _, m)| m.contains("never closed")), "{e:?}");
    let c = check_with("tempo 120\ndef gate(c) {\n  deck A: kick x ~ x ~ when $c\n}\ngate(2 < 3)\n", &Env::none());
    assert!(c.diagnostics.iter().any(|d| d.help.as_deref().is_some_and(|h| h.contains("alternation"))), "{:?}", c.diagnostics);
}

// ---- the editor's marks (spans) of lines that carry arithmetic, a guard or a time between bars, through a def, a wire and an import

/// Every control's span and the spans of the calls that expanded it are real, non-empty pieces of the text; every hit's too.
fn spans_are_real(src: &str, c: &djlive::compile::Compiled) -> (usize, usize) {
    for k in &c.controls {
        assert!(!k.span.is_empty() && k.span.end <= src.len(), "control {:?}: an empty or outside span {:?} in {src:?}", k.key, k.span);
        for v in &k.via {
            assert!(!v.is_empty() && v.end <= src.len(), "control {:?}: an empty or outside via {v:?}", k.key);
        }
    }
    let mut hits = 0;
    for deck in c.decks.values() {
        for part in &deck.parts {
            for chain in &part.spans {
                for s in chain {
                    assert!(!s.is_empty() && s.end <= src.len(), "an empty or outside span {s:?} in {src:?}");
                }
            }
            hits += part.cycles.iter().map(Vec::len).sum::<usize>();
        }
    }
    (c.controls.len(), hits)
}

#[test]
fn the_marks_of_guarded_timed_arithmetic_lines_are_real_through_a_wire_a_repeat_and_an_import() {
    // through a wire and a repeat
    let c = ok(WIRED);
    let (controls, hits) = spans_are_real(WIRED, &c);
    assert!(controls == 2 && hits > 0, "{controls} {hits}");
    // through an import: the lines of the def are marked where the importing program calls them
    let snips = snippets(&[("swell", TIMED), ("ditty", "def ditty(i) {\n  deck A: hihat/12 e($i % 4 + 1, 8)?(1 / 4) when $i == 0\n  at bar 2 beat 1 + $i: master: delay 0.3 when $i != 1\n}\n")]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"swell\"\nimport \"ditty\"\ntempo 128\nsection intro 2 bars {\n  deck B: kick x ~ x ~\n}\nsection drop 4 bars {\n  swell(2)\n  repeat 2 { ditty($i) }\n}\narrange intro drop\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    let comp = c.compiled.unwrap();
    let (controls, hits) = spans_are_real(src, &comp);
    assert!(controls >= 6 && hits > 0, "{controls} {hits}");
    // the call that expanded a line is in its `via`
    assert!(comp.controls.iter().all(|k| !k.via.is_empty()), "{:?}", comp.controls.iter().map(|k| (&k.key, &k.via)).collect::<Vec<_>>());
}

// ---- words

#[test]
fn the_keywords_of_arithmetic_timing_ports_and_chance_are_highlighted_and_reserved() {
    use djlive::highlight::{highlight, Class};
    let src = "def f(i) in: rhythm out: bass {\n  deck A: kick $in vel=min(60, 80) when $i == 0 and not $i > 3 or 1 < 2\n  every 2 beats from 1: master: level 0.5\n  at bar 2 beat 2: master: level 0.5\n  emit bass choose [c2 e2] weights 3 1\n}\nwire a.out -> f.in\n";
    let marks = highlight(src);
    // every mark of that word in the text has the class
    let classes_of = |word: &str| marks.iter().filter(|(s, _)| s.text(src) == word).map(|(_, c)| *c).collect::<Vec<_>>();
    for w in ["when", "and", "not", "or", "beats", "from", "beat", "emit", "choose", "weights", "wire", "in", "out"] {
        let cs = classes_of(w);
        assert!(!cs.is_empty() && cs.iter().all(|c| *c == Class::Keyword), "{w}: {cs:?}");
    }
    assert_eq!(classes_of("min"), vec![Class::Generator]);
    for word in ["when", "and", "or", "not", "from", "emit", "choose", "maybe"] {
        let e = errs(&format!("let {word} = x ~ x\n"));
        assert!(said(&e, "word of the language") || said(&e, "generator or transform"), "{word}: {e:?}");
        let e = errs(&format!("def {word}() {{\n  deck A: kick x\n}}\n"));
        assert!(said(&e, "word of the language") || said(&e, "generator or transform"), "{word}: {e:?}");
    }
    // `beat`, `beats` and `weights` are read as syntax after a number or a group only: they name a let as before
    for word in ["beat", "beats", "weights"] {
        ok(&format!("let {word} = x ~ x\ndeck A: kick {word}\n"));
    }
}
