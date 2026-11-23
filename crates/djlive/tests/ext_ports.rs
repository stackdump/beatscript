//! F11 typed ports on defs, `wire`, imports glued along ports (docs/beatscript/ext-ports.md): the header and the wire in
//! canonical form, the checks (types, one wire to an input, no loop, an `emit` of the right type), what a call
//! expands to, imports that carry their ports, and the laws (wiring is associative, order of wires does not
//! matter, the identity module is invisible, a def without ports is the macro it was).

mod common;

use common::{reformat, Rng};
use djlive::ast::StmtKind;
use djlive::compile::check_with;
use djlive::env::*;
use djlive::{check, print};

// ---- helpers

fn ok(src: &str) -> djlive::compile::Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String)> {
    errs_with(src, &Env::none())
}

fn errs_with(src: &str, env: &Env) -> Vec<(usize, usize, String)> {
    let c = check_with(src, env);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| {
            let (l, col, _) = d.position(src);
            (l, col, d.message.clone())
        })
        .collect()
}

fn one_err(src: &str) -> (usize, usize, String) {
    let e = errs(src);
    assert_eq!(e.len(), 1, "{e:?}");
    e.into_iter().next().unwrap()
}

fn said(e: &[(usize, usize, String)], what: &str) -> bool {
    e.iter().any(|(_, _, m)| m.contains(what))
}

/// The help line of the first error.
fn help_of(src: &str) -> String {
    help_of_with(src, &Env::none())
}

fn help_of_with(src: &str, env: &Env) -> String {
    let c = check_with(src, env);
    c.diagnostics.iter().find(|d| d.is_error()).and_then(|d| d.help.clone()).unwrap_or_default()
}

/// Every flat line the program expands to, printed, in order (the globals, then each segment's), with its timing.
fn lines(src: &str, env: &Env) -> Vec<String> {
    let parsed = djlive::parse::parse_with(src, env);
    assert!(parsed.ok(), "{}", djlive::diag::render_all(&parsed.diagnostics, src));
    let mut diags = Vec::new();
    let lw = djlive::expand::lower(&parsed, src, &mut diags);
    assert!(!diags.iter().any(|d| d.is_error()), "{}", djlive::diag::render_all(&diags, src));
    let mut out: Vec<String> = lw.globals.iter().map(|f| format!("{:?} {}", f.timing, print::line(&f.line))).collect();
    for s in &lw.segments {
        out.extend(s.lines.iter().map(|f| format!("[{}] {:?} {}", s.name, f.timing, print::line(&f.line))));
    }
    out
}

fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    // each snippet may import the ones before it
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let canon = check_with(text, &env).compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}"));
        let (cid, text) = (canon.cid.clone(), canon.text.clone());
        snips.items.push((name.to_string(), cid, text));
    }
    snips
}

/// Thirteen lines: four modules with ports.
const PIECES: &str = "\
def changes() out: chords {
  emit <chord(am7) chord(fmaj7) chord(cmaj7) chord(g7)>
}
def home() out: key {
  emit am
}
def walker(oct) in: chords, key out: bass {
  deck B: harmony $chords | melody markov($key, 0.5) oct=$oct
  emit [a2 ~ a2 ~ f2 ~ g2 ~]
}
def thump() in: bass {
  deck A: kick x ~ x ~ | bass $in
}
";

fn program(extra: &str) -> String {
    format!("{PIECES}{extra}")
}

const WIRED: &str = "wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\nwalker(4)\nthump()\n";

// ---- the header and the wire, in canonical form

#[test]
fn a_def_header_with_ports_is_read_and_printed_in_one_form() {
    let c = ok(&program(WIRED));
    assert!(c.text.contains("def changes() out: chords {\n"), "{}", c.text);
    assert!(c.text.contains("def walker(oct) in: chords, key out: bass {\n"), "{}", c.text);
    assert!(c.text.contains("def thump() in: bass {\n"), "{}", c.text);
    assert!(c.text.contains("wire changes.out -> walker.chords\n"), "{}", c.text);
    assert!(c.text.contains("wire walker.out -> thump.in\n"), "{}", c.text);
    // a fixed point, with the same CID
    let again = ok(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
}

#[test]
fn ports_are_sorted_by_type_so_their_order_is_not_a_fork() {
    let a = ok("def foo() in: key, chords, bars out: tempo, bass {\n  emit tempo 120\n  emit bass c2\n}\n");
    let b = ok("def foo() in: bars, chords, key out: bass, tempo {\n  emit tempo 120\n  emit bass c2\n}\n");
    assert!(a.text.starts_with("def foo() in: chords, key, bars out: bass, tempo {\n"), "{}", a.text);
    assert_eq!(a.cid, b.cid);
}

#[test]
fn the_two_spellings_of_a_sole_port_are_one_program() {
    let by_role = "def one() out: chords {\n  emit am7\n}\ndef two() in: chords {\n  deck B: arp arp($in, up, 8)\n}\nwire one.out -> two.in\ntwo()\n";
    let by_type = by_role.replace("wire one.out -> two.in", "wire one.chords -> two.chords");
    let x = ok(by_role);
    let y = ok(&by_type);
    assert_eq!(x.cid, y.cid);
    assert!(x.text.contains("wire one.out -> two.in\n"), "{}", x.text);
    // `$chords` and `$in` are the same value, inside the body
    let z = ok(&by_role.replace("$in", "$chords"));
    assert_eq!(z.decks[&djlive::ast::Deck::B].net_cid(), x.decks[&djlive::ast::Deck::B].net_cid());
}

#[test]
fn reformatting_keeps_the_cid_and_an_edit_changes_it() {
    let src = program(WIRED);
    let base = ok(&src).cid;
    let mut rng = Rng(0xf11);
    for round in 0..40 {
        let text = reformat(&src, &mut rng);
        let c = check(&text);
        let got = c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text))).cid;
        assert_eq!(got, base, "round {round} forked:\n{text}");
    }
    // each edit is another program or an error, never the same CID
    for (from, to) in [("emit am\n", "emit cm\n"), ("thump()\n", "thump()\nthump()\n"), ("oct=$oct", "oct=2"), ("wire home.out -> walker.key\n", ""), ("walker.chords", "walker.key")] {
        let edited = src.replace(from, to);
        assert_ne!(edited, src, "{from}");
        if let Some(c) = check(&edited).compiled {
            assert_ne!(c.cid, base, "{from} -> {to}");
        }
    }
}

#[test]
fn a_def_with_no_ports_is_the_macro_it_was() {
    let src = "def drop(deck, bars) {\n  deck $deck: kick x ~ x ~\n  at bar 1: master: level 0.9\n}\ndrop(A, 8)\n";
    let c = ok(src);
    assert_eq!(c.text, "def drop(deck, bars) {\n  deck $deck: kick x ~ x ~\n  at bar 1: master: level 0.9\n}\ndrop(A, 8)\n");
    let StmtKind::Def(d) = &djlive::parse::parse(src).program.stmts[0].kind else { panic!() };
    assert!(d.ports.is_empty());
    // and `emit` in one is said, not run
    let e = errs("def foo(a) {\n  emit x\n}\nfoo(1)\n");
    assert!(said(&e, "`emit` is for a def with `out:` ports"), "{e:?}");
}

// ---- what a call expands to

#[test]
fn a_call_expands_with_the_wired_values_in_place_of_its_inputs() {
    let l = lines(&program(WIRED), &Env::none());
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[0], "Now deck B: harmony <chord(am7) chord(fmaj7) chord(cmaj7) chord(g7)> | melody markov(am, 0.5) oct=4");
    assert_eq!(l[1], "Now deck A: kick x ~ x ~ | bass [a2 ~ a2 ~ f2 ~ g2 ~]");
}

#[test]
fn an_input_takes_its_value_as_one_group_wherever_it_stands() {
    // `$in` is a unit: in a call's argument, in an alternation, beside other terms
    let src = "def riff() out: bass {\n  emit c2 ~ e2 g2\n}\ndef play_it() in: bass {\n  deck A: bass transpose($in, 12) | melody <$in c4>\n  deck B: bass $in c3\n}\nwire riff.out -> play_it.in\nplay_it()\n";
    let l = lines(src, &Env::none());
    assert_eq!(l[0], "Now deck A: bass transpose([c2 ~ e2 g2], 12) | melody <[c2 ~ e2 g2] c4>");
    assert_eq!(l[1], "Now deck B: bass [c2 ~ e2 g2] c3");
    ok(src);
    // a single term, a group, a call and an alternation go in as they are
    for (value, shown) in [("c2", "c2"), ("[c2 e2]", "[c2 e2]"), ("<c2 e2>", "<c2 e2>"), ("reverse([c2 e2])", "reverse([c2 e2])")] {
        let s = format!("def src() out: bass {{\n  emit {value}\n}}\ndef sink() in: bass {{\n  deck A: bass $in\n}}\nwire src.out -> sink.in\nsink()\n");
        let l = lines(&s, &Env::none());
        assert_eq!(l[0], format!("Now deck A: bass {shown}"), "{value}");
    }
    // a key and a number are never wrapped
    let s = "def kk() out: key {\n  emit d:dorian\n}\ndef nn() out: bars {\n  emit 4\n}\ndef sink() in: key, bars {\n  deck A: bass markov($key, 0.5)\n  at bar $bars: master: filter off\n}\nwire kk.out -> sink.key\nwire nn.out -> sink.bars\nsink()\n";
    let l = lines(s, &Env::none());
    assert_eq!(l[0], "Now deck A: bass markov(d:dorian, 0.5)");
    assert_eq!(l[1], "At(4) at bar 4: master: filter off");
}

#[test]
fn what_an_out_is_follows_its_inputs_and_not_the_call() {
    // one wired value, two calls with different params: both read the same input
    let src = program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwalker(3)\nwalker(5)\n");
    let l = lines(&src, &Env::none());
    assert!(l[0].ends_with("oct=3") && l[1].ends_with("oct=5"), "{l:?}");
    assert_eq!(l[0].replace("oct=3", ""), l[1].replace("oct=5", ""));
}

#[test]
fn an_output_may_feed_several_inputs_and_a_def_may_sit_in_a_section() {
    let src = "def riff() out: bass {\n  emit [c2 ~ e2 ~]\n}\ndef left() in: bass {\n  deck A: bass $in\n}\ndef right() in: bass {\n  deck B: bass transpose($in, 12)\n}\nwire riff.out -> left.in\nwire riff.out -> right.in\nsection one 4 bars {\n  left()\n}\nsection two 4 bars {\n  right()\n}\narrange one two\n";
    let l = lines(src, &Env::none());
    assert_eq!(l, vec!["[one] At(0) deck A: bass [c2 ~ e2 ~]".to_string(), "[two] At(4) deck B: bass transpose([c2 ~ e2 ~], 12)".to_string()]);
    ok(src);
}

#[test]
fn the_value_of_an_out_is_made_from_the_chain_it_hangs_from() {
    // alpha -> beta -> gamma: each hop transforms what it was given
    let src = "def alpha() out: bass {\n  emit [c2 e2]\n}\ndef beta() in: bass out: bass {\n  emit transpose($in, 5)\n}\ndef gamma() in: bass {\n  deck A: bass transpose($in, 7)\n}\nwire alpha.out -> beta.in\nwire beta.out -> gamma.in\ngamma()\n";
    assert_eq!(lines(src, &Env::none()), vec!["Now deck A: bass transpose(transpose([c2 e2], 5), 7)".to_string()]);
}

#[test]
fn a_module_a_call_never_reaches_still_makes_its_value() {
    // `walker` is not called, `thump` is: walker's lines are not in the program, its out still flows
    let src = program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\nthump()\n");
    let c = check(&src);
    assert!(c.ok());
    let l = lines(&src, &Env::none());
    assert_eq!(l, vec!["Now deck A: kick x ~ x ~ | bass [a2 ~ a2 ~ f2 ~ g2 ~]".to_string()]);
}

#[test]
fn a_bars_input_is_a_count_a_repeat_can_take_and_a_def_may_call_a_def_with_ports() {
    let src = "def howlong() out: bars {\n  emit 4\n}\ndef riser() in: bars {\n  repeat $bars every 1 bars {\n    master: filter lp 300 -> 2800 over 1 bars\n  }\n}\ndef intro() {\n  riser()\n  deck A: kick x ~ x ~\n}\nwire howlong.out -> riser.in\nintro()\n";
    let l = lines(src, &Env::none());
    assert_eq!(l.len(), 5, "{l:?}");
    assert!(l[0].starts_with("Now master: filter lp 300 -> 2800"), "{l:?}");
    assert!(l[3].starts_with("At(3) master: filter lp"), "{l:?}");
    assert_eq!(l[4], "Now deck A: kick x ~ x ~");
    ok(src);
}

#[test]
fn imports_that_import_keep_their_ports_and_wires() {
    let inner = HARMONY.to_string();
    let outer = format!("import \"inner\"\n{WALKER}wire harmony_changes.out -> walker.chords\nwire harmony_key.out -> walker.key\n");
    let snips = snippets(&[("inner", &inner), ("outer", &outer)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let l = lines("import \"outer\"\nwalker(1)\n", &env);
    assert_eq!(l.len(), 1);
    assert!(l[0].contains("harmony <chord(dm7)") && l[0].ends_with("oct=1"), "{l:?}");
}

// ---- types

#[test]
fn a_type_error_says_what_each_end_is_and_where() {
    // `drums` is the type's word in a header only: a def may be named `drums` too
    let c = check("def drums() out: drums {\n  emit x ~ x ~\n}\n");
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, ""));
    let src = "def groove() out: drums {\n  emit x ~ x ~\n}\ndef walker() in: chords {\n  deck B: harmony $in\n}\nwire groove.out -> walker.chords\n";
    let (line, col, msg) = one_err(src);
    assert_eq!((line, col), (7, 6));
    assert_eq!(msg, "type error: `groove.out` is a drums pattern, `walker.chords` wants chords");
    // the contract's own example, with the names it uses
    let src = "def drums_out() out: drums {\n  emit x ~ x ~\n}\ndef walker() in: chords {\n  deck B: harmony $in\n}\nwire drums_out.out -> walker.chords\n";
    assert_eq!(one_err(src).2, "type error: `drums_out.out` is a drums pattern, `walker.chords` wants chords");
}

#[test]
fn the_types_that_fit_and_the_ones_that_do_not() {
    let wire = |from: &str, emit: &str, to: &str| -> Result<(), String> {
        let src = format!("def p() out: {from} {{\n  emit {emit}\n}}\ndef q() in: {to} {{\n  deck A: kick x\n}}\nwire p.out -> q.in\n");
        let c = check(&src);
        if c.ok() {
            Ok(())
        } else {
            Err(c.diagnostics.iter().find(|d| d.is_error()).unwrap().message.clone())
        }
    };
    // every pattern kind into `pattern`, and into its own
    for (t, v) in [("bass", "c2"), ("melody", "c4"), ("drums", "x ~"), ("rhythm", "x ~"), ("pattern", "c2 ~"), ("chords", "am7"), ("key", "am"), ("bars", "4"), ("tempo", "120")] {
        assert!(wire(t, v, t).is_ok(), "{t}");
        assert_eq!(wire(t, v, "pattern").is_ok(), !matches!(t, "chords" | "key" | "bars" | "tempo"), "{t} -> pattern");
    }
    // a rhythm into any part's input
    for t in ["bass", "melody", "drums"] {
        assert!(wire("rhythm", "x ~ x", t).is_ok(), "rhythm -> {t}");
    }
    // not the other way, not across kinds
    for (a, v, b) in [("bass", "c2", "drums"), ("drums", "x", "bass"), ("bass", "c2", "melody"), ("pattern", "c2", "bass"), ("bass", "c2", "rhythm")] {
        let e = wire(a, v, b).unwrap_err();
        assert!(e.starts_with("type error: `p.out` is "), "{a} -> {b}: {e}");
    }
    for (a, v, b) in [("chords", "am7", "key"), ("key", "am", "chords"), ("bars", "4", "tempo"), ("tempo", "120", "bars"), ("bass", "c2", "chords")] {
        assert!(wire(a, v, b).is_err(), "{a} -> {b}");
    }
    assert_eq!(wire("key", "am", "pattern").unwrap_err(), "type error: `p.out` is a key, `q.in` wants any pattern");
}

#[test]
fn an_emit_is_checked_against_its_ports_type() {
    let e = |ty: &str, value: &str| -> String {
        let src = format!("def foo() out: {ty} {{\n  emit {value}\n}}\n");
        let c = check(&src);
        assert!(!c.ok(), "{ty}: {value}");
        c.diagnostics.iter().find(|d| d.is_error()).unwrap().message.clone()
    };
    assert!(e("chords", "x ~ x").contains("`x` is not a chord"));
    assert!(e("chords", "c3 e3").contains("not a chord"));
    assert!(e("chords", "<am7 foo>").contains("`foo` is not a chord"));
    assert!(e("chords", "arp(am7, up, 8)").contains("not a chord"));
    assert!(e("key", "x").contains("is not a key"));
    assert!(e("key", "am dm").contains("one word"));
    assert!(e("bars", "0").contains("whole number"));
    assert!(e("bars", "2.5").contains("whole number"));
    assert!(e("tempo", "5").contains("tempo"));
    assert!(e("drums", "c3 ~ x").contains("is a pitch"));
    assert!(e("rhythm", "markov(am, 0.5)").contains("makes pitches"));
    assert!(e("drums", "1 ~ 3").contains("scale degree"));
    assert!(e("bass", "[c2 ~").contains("expected"));
    // and the ones that are fine
    for (ty, v) in [
        ("chords", "<chord(am7) chord(g7)>"),
        ("chords", "am7"),
        ("chords", "<I V vi IV>"),
        ("chords", "ii\u{b0} V7 I/3 bVII"),
        ("chords", "<am7 ~ g7*2>"),
        ("key", "am"),
        ("key", "d:dorian"),
        ("bars", "16"),
        ("tempo", "128"),
        ("bass", "c2 ~ <e2 g2> e(3,8)"),
        ("melody", "1 3 5 markov(am, 0.5)"),
        ("drums", "x ~ [x x] e(5,8)"),
        ("rhythm", "x*4 reverse([x ~ x ~])"),
        ("pattern", "x c2 1"),
    ] {
        let src = format!("def foo() out: {ty} {{\n  emit {v}\n}}\n");
        let c = check(&src);
        assert!(c.ok(), "{ty}: {v}\n{}", djlive::diag::render_all(&c.diagnostics, &src));
    }
}

#[test]
fn a_let_pattern_is_a_rhythm_when_it_is_hits_and_rests() {
    ok("let hats = x ~ x x\ndef foo() out: drums {\n  emit hats\n}\n");
    let e = errs("let riff = c2 e2\ndef foo() out: drums {\n  emit riff\n}\n");
    assert!(said(&e, "is a pitch"), "{e:?}");
}

#[test]
fn a_plain_forward_of_an_input_is_typed_where_it_is_written() {
    let e = errs("def foo() in: bass out: chords {\n  emit $in\n}\n");
    assert!(said(&e, "type error: `$in` is a bass pattern"), "{e:?}");
    // the same type, or a rhythm into a bass out, is fine
    ok("def foo() in: bass out: bass {\n  emit $in\n}\n");
    ok("def foo() in: rhythm out: bass {\n  emit $in\n}\n");
    ok("def foo() in: bass out: pattern {\n  emit $in\n}\n");
}

// ---- the wiring

#[test]
fn a_wire_to_something_that_is_not_there_says_what_to_write() {
    let src = program("wire chnages.out -> walker.chords\n");
    let c = check(&src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(d.message, "`chnages` is not a def");
    assert_eq!(d.help.as_deref(), Some("did you mean `changes`?"));
    assert_eq!(d.position(&src).0, 14);
    // a def with no ports
    let src = "def plain() {\n  deck A: kick x\n}\ndef sink() in: bass {\n  deck A: bass $in\n}\nwire plain.out -> sink.in\n";
    let e = errs(src);
    assert_eq!(e[0].2, "`plain` has no ports");
    assert!(help_of(src).contains("declare them in its header"));
    // a port that is not there
    let src = program("wire changes.out -> walker.chord\n");
    let c = check(&src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(d.message, "`walker` has no input port `chord`");
    assert_eq!(d.help.as_deref(), Some("did you mean `chords`?"));
    assert_eq!(d.expected, vec!["chords".to_string(), "key".to_string()]);
}

#[test]
fn a_wire_goes_from_an_output_to_an_input() {
    // an input as the source
    let src = program("wire walker.chords -> thump.in\n");
    let c = check(&src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.message.contains("has no output port `chords`"), "{}", d.message);
    assert!(d.help.as_deref().unwrap().contains("is an input"), "{:?}", d.help);
    // `in` where there are two inputs
    let src = program("wire changes.out -> walker.in\n");
    let c = check(&src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.help.as_deref().unwrap().contains("walker.chords or walker.key"), "{:?}", d.help);
    // a def with nothing on that side
    let src = program("wire thump.out -> walker.chords\n");
    let d = check(&src).diagnostics.into_iter().find(|d| d.is_error()).unwrap();
    assert!(d.message.contains("`thump` has no output port `out`"), "{}", d.message);
}

#[test]
fn an_input_is_wired_once() {
    let src = program("def other() out: chords {\n  emit g7\n}\nwire changes.out -> walker.chords\nwire other.out -> walker.chords\n");
    let e = errs(&src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].2, "`walker.chords` is wired twice: from `changes.out` (line 17) and from `other.out`");
    assert_eq!((e[0].0, e[0].1), (18, 6));
}

#[test]
fn an_unwired_input_is_an_error_naming_the_def_and_the_port() {
    let src = program("wire changes.out -> walker.chords\nwalker(4)\n");
    let e = errs(&src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].2, "`walker.key` is not wired: `walker` needs a key and nothing feeds it");
    assert_eq!((e[0].0, e[0].1), (15, 1));
    assert_eq!(help_of(&src), "wire home.out -> walker.key");
    // and the one that needs a bass names the defs that make one
    let src = program("thump()\n");
    assert_eq!(one_err(&src).2, "`thump.in` is not wired: `thump` needs a bass pattern and nothing feeds it");
    assert_eq!(help_of(&src), "wire walker.out -> thump.in");
    // said once, however many times it is called
    assert_eq!(errs(&program("thump()\nthump()\n")).len(), 1);
    // a def that is never called does not need its inputs
    ok(&program(""));
}

#[test]
fn an_input_unwired_upstream_is_said_at_the_call_that_needs_it() {
    // thump is called, its bass comes from walker, and walker has no key
    let src = program("wire changes.out -> walker.chords\nwire walker.out -> thump.in\nthump()\n");
    let e = errs(&src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].2, "`walker.key` is not wired: `walker` needs a key and nothing feeds it");
    assert_eq!((e[0].0, e[0].1), (16, 1), "at the call");
    assert_eq!(help_of(&src), "wire home.out -> walker.key");
}

#[test]
fn a_loop_in_the_wiring_is_an_error_naming_it() {
    let src = "def alpha() in: bass out: bass {\n  emit $in\n}\ndef beta() in: bass out: bass {\n  emit $in\n}\ndef gamma() in: bass out: bass {\n  emit $in\n}\nwire alpha.out -> beta.in\nwire beta.out -> gamma.in\nwire gamma.out -> alpha.in\ngamma()\n";
    let e = errs(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].2, "the wires make a loop: alpha -> beta -> gamma -> alpha");
    assert_eq!(e[0].0, 12, "the wire that closes it");
    // a def feeding itself
    let src = "def alpha() in: bass out: bass {\n  emit $in\n}\nwire alpha.out -> alpha.in\nalpha()\n";
    assert_eq!(one_err(src).2, "the wires make a loop: alpha -> alpha");
}

#[test]
fn a_wire_never_followed_is_a_warning() {
    // `thump` is called, `walker` is wired and is not
    let src = program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\nthump()\n");
    let c = check(&src);
    assert!(c.ok());
    let w = c.diagnostics.iter().find(|d| !d.is_error() && d.message.contains("never called")).expect("a warning");
    assert!(w.message.contains("`walker` is wired but never called"), "{}", w.message);
    assert_eq!(c.diagnostics.iter().filter(|d| d.message.contains("never called")).count(), 1, "said once for the def");
    // both called: no warning
    let c = check(&program(WIRED));
    assert!(!c.diagnostics.iter().any(|d| d.message.contains("never called")));
    // a library, called by nothing, wires its modules without a word
    let c = check(&program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\n"));
    assert!(c.ok() && c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn a_long_chain_is_fine_and_a_very_long_one_is_said() {
    let chain = |n: usize| -> String {
        let mut s = String::from("def m0() out: bass {\n  emit [c2 e2]\n}\n");
        for i in 1..n {
            s.push_str(&format!("def m{i}() in: bass out: bass {{\n  emit $in\n}}\n"));
        }
        s.push_str("def sink() in: bass {\n  deck A: bass $in\n}\n");
        for i in 1..n {
            s.push_str(&format!("wire m{}.out -> m{i}.in\n", i - 1));
        }
        s.push_str(&format!("wire m{}.out -> sink.in\nsink()\n", n - 1));
        s
    };
    let src = chain(40);
    let c = check(&src);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    let e = errs(&chain(80));
    assert!(said(&e, "wires away"), "{e:?}");
}

// ---- emits

#[test]
fn an_emit_is_made_once_for_every_out_port_and_only_for_those() {
    // never emitted
    let e = errs("def foo() out: chords {\n  deck A: kick x\n}\n");
    assert_eq!(e[0].2, "`foo` declares `out: chords` and never emits it");
    assert_eq!((e[0].0, e[0].1), (1, 5));
    // twice
    let e = errs("def foo() out: chords {\n  emit am7\n  emit g7\n}\n");
    assert!(said(&e, "emits its `chords` out twice (line 2)"), "{e:?}");
    // an emit with no out ports
    let e = errs("def foo() in: chords {\n  emit am7\n}\n");
    assert!(said(&e, "has no `out:` ports"), "{e:?}");
    // nested in a block
    let e = errs("def foo() out: bass {\n  emit c2\n  repeat 2 {\n    emit c2\n  }\n}\n");
    assert!(said(&e, "directly in the def's body"), "{e:?}");
    // several out ports need their name
    let e = errs("def foo() out: bass, chords {\n  emit c2\n  emit am7\n}\n");
    assert!(said(&e, "several out ports"), "{e:?}");
    ok("def foo() out: bass, chords {\n  emit bass c2\n  emit chords am7\n}\n");
    // a port it does not have
    let e = errs("def foo() out: bass {\n  emit melody c4\n}\n");
    assert!(said(&e, "`foo` has no out port `melody`"), "{e:?}");
    // an emit with nothing after it
    assert!(said(&errs("def foo() out: bass {\n  emit\n}\n"), "has no value"));
    assert!(said(&errs("def foo() out: bass {\n  emit bass\n}\n"), "has no value"));
}

#[test]
fn the_out_port_may_be_named_or_not() {
    let a = ok("def foo() out: bass {\n  emit c2 e2\n}\n");
    let b = ok("def foo() out: bass {\n  emit bass c2 e2\n}\n");
    let c = ok("def foo() out: bass {\n  emit out c2 e2\n}\n");
    // three spellings in the text are three programs (a body is kept as written), each fine
    assert_ne!(a.cid, b.cid);
    assert_ne!(b.cid, c.cid);
}

#[test]
fn an_emit_is_fixed_by_the_inputs_not_by_the_parameters() {
    let src = "def foo(k) in: bass out: bass {\n  emit transpose($in, $k)\n}\n";
    let e = errs(src);
    assert!(said(&e, "an `emit` cannot use the parameter `$k`"), "{e:?}");
    assert!(help_of(src).contains("make it an input"));
    // an unknown name is said with the nearest
    let src = "def foo() in: bass out: bass {\n  emit transpose($inn, 1)\n}\n";
    let e = errs(src);
    assert!(said(&e, "`$inn` is not an in port of `foo`"), "{e:?}");
    assert_eq!(help_of(src), "did you mean `$in`?");
}

#[test]
fn a_parameter_cannot_share_a_name_with_a_port() {
    let e = errs("def foo(key) in: key {\n  deck A: bass markov($key, 0.5)\n}\n");
    assert!(said(&e, "both a parameter and an in port"), "{e:?}");
    let e = errs("def foo(in) in: bass {\n  deck A: bass $in\n}\n");
    assert!(said(&e, "both a parameter and an in port"), "{e:?}");
}

#[test]
fn a_value_that_does_not_fit_when_it_flows_is_said_at_its_emit() {
    // the forward is typed where it is written
    let src = "def alpha() out: bass {\n  emit [c2 e2]\n}\ndef beta() in: bass out: drums {\n  emit $in\n}\ndef gamma() in: drums {\n  deck A: kick $in\n}\nwire alpha.out -> beta.in\nwire beta.out -> gamma.in\ngamma()\n";
    let e = errs(src);
    assert!(said(&e, "type error: `$in` is a bass pattern, and `beta` emits it as its drums out"), "{e:?}");
    // through a call, only the made value shows it
    let src = "def alpha() out: bass {\n  emit [c2 e2]\n}\ndef beta() in: bass out: drums {\n  emit reverse($in)\n}\ndef gamma() in: drums {\n  deck A: kick $in\n}\nwire alpha.out -> beta.in\nwire beta.out -> gamma.in\ngamma()\n";
    let e = errs(src);
    assert!(said(&e, "is a pitch"), "{e:?}");
    assert_eq!((e[0].0, e[0].1), (5, 16), "at the `$in` of the emit");
    assert!(help_of(src).contains("used by the call on line 12"), "{}", help_of(src));
}

#[test]
fn a_progression_of_chord_symbols_plays_as_chords_and_a_lone_chord_is_a_name() {
    // `emit <am7 fmaj7>` is `emit <chord(am7) chord(fmaj7)>` once it flows, so a harmony part plays it
    let sym = "def prog() out: chords {\n  emit <am7 fmaj7 cmaj7 g7>\n}\ndef sink() in: chords {\n  deck B: harmony $in\n}\nwire prog.out -> sink.in\nsink()\n";
    let call = sym.replace("<am7 fmaj7 cmaj7 g7>", "<chord(am7) chord(fmaj7) chord(cmaj7) chord(g7)>");
    assert_eq!(lines(sym, &Env::none()), lines(&call, &Env::none()));
    assert_eq!(ok(sym).decks[&djlive::ast::Deck::B].net_cid(), ok(&call).decks[&djlive::ast::Deck::B].net_cid());
    // a Roman numeral is left for the part to read, and what is inside a call is left alone
    let l = lines(&sym.replace("<am7 fmaj7 cmaj7 g7>", "<chord(am7) V7 ~ chord(g7)>"), &Env::none());
    assert_eq!(l[0], "Now deck B: harmony <chord(am7) V7 ~ chord(g7)>");
    // a lone chord word stays a word: `arp($in, up, 8)` and `chord($in)` take it
    let lone = "def one() out: chords {\n  emit am7\n}\ndef sink() in: chords {\n  deck B: arp arp($in, up, 8) | harmony chord($in)\n}\nwire one.out -> sink.in\nsink()\n";
    assert_eq!(lines(lone, &Env::none()), vec!["Now deck B: arp arp(am7, up, 8) | harmony chord(am7)".to_string()]);
    ok(lone);
}

// ---- imports

const HARMONY: &str = "def harmony_changes() out: chords {\n  emit <chord(dm7) chord(g7) chord(cmaj7) chord(fmaj7)>\n}\ndef harmony_key() out: key {\n  emit am\n}\n";
const WALKER: &str = "def walker(oct) in: chords, key out: bass {\n  deck B: harmony $chords | melody markov($key, 0.5) oct=$oct\n  emit [a2 ~ a2 ~ e2 ~ g2 ~]\n}\n";

#[test]
fn an_import_brings_its_defs_with_their_ports_and_is_pinned_by_its_cid() {
    let snips = snippets(&[("harmony", HARMONY)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = format!("import \"harmony\"\n{WALKER}wire harmony_changes.out -> walker.chords\nwire harmony_key.out -> walker.key\nwalker(3)\n");
    let c = check_with(&src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    let comp = c.compiled.unwrap();
    let cid = check(HARMONY).compiled.unwrap().cid;
    assert!(comp.text.starts_with(&format!("import {cid}\n")), "{}", comp.text);
    // the same program by the CID, the same program
    let by_cid = check_with(&src.replace("import \"harmony\"", &format!("import {cid}")), &env).compiled.unwrap();
    assert_eq!(by_cid.cid, comp.cid);
    assert_eq!(lines(&src, &env), vec!["Now deck B: harmony <chord(dm7) chord(g7) chord(cmaj7) chord(fmaj7)> | melody markov(am, 0.5) oct=3".to_string()]);
    // a different library is a different program
    let other = snippets(&[("harmony", &HARMONY.replace("dm7", "dm9"))]);
    let env2 = Env { snippets: Some(&other), ..Env::default() };
    let c2 = check_with(&src, &env2).compiled.unwrap();
    assert_ne!(c2.cid, comp.cid);
}

#[test]
fn a_wire_to_a_port_an_import_does_not_have_is_said_at_the_wire() {
    let snips = snippets(&[("harmony", HARMONY)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = format!("import \"harmony\"\n{WALKER}wire harmony_changes.out -> walker.key\nwalker(3)\n");
    let e = errs_with(&src, &env);
    assert!(e.iter().any(|e| e.2 == "type error: `harmony_changes.out` is chords, `walker.key` wants a key"), "{e:?}");
    // the typo of an imported name still gets its did-you-mean
    let src = format!("import \"harmony\"\n{WALKER}wire harmony_change.out -> walker.chords\n");
    let c = check_with(&src, &env);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(d.help.as_deref(), Some("did you mean `harmony_changes`?"));
}

#[test]
fn an_import_brings_its_wires_and_a_second_wire_to_the_same_input_is_an_error() {
    let lib = format!("{HARMONY}{WALKER}wire harmony_changes.out -> walker.chords\nwire harmony_key.out -> walker.key\n");
    let snips = snippets(&[("band", &lib)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let l = lines("import \"band\"\nwalker(2)\n", &env);
    assert_eq!(l.len(), 1);
    assert!(l[0].contains("markov(am, 0.5) oct=2"), "{l:?}");
    // wiring what the import already wired
    let src = "import \"band\"\ndef mine() out: chords {\n  emit am7\n}\nwire mine.out -> walker.chords\nwalker(2)\n";
    let e = errs_with(src, &env);
    assert!(said(&e, "`walker.chords` is wired twice: from `harmony_changes.out` (line 1) and from `mine.out`"), "{e:?}");
}

#[test]
fn the_import_example_compiles_against_the_snippet_it_describes() {
    let lib = "def harmony_changes() out: chords {\n  emit <dm7 g7 cmaj7 fmaj7>\n}\ndef harmony_key() out: key {\n  emit am\n}\n";
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/ports-import.beatscript")).unwrap();
    assert!(src.lines().take_while(|l| l.starts_with("#!")).any(|l| l.starts_with("#! skip-test:")), "the header block says skip-test");
    // the snippet is in the example's own comment, as saved
    for l in lib.lines() {
        assert!(src.contains(&format!("#   {l}")) || src.contains(&format!("#{l}")) || l.is_empty(), "{l}");
    }
    let snips = snippets(&[("harmony", lib)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let c = check_with(&src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    let comp = c.compiled.unwrap();
    assert!(comp.warnings.is_empty(), "{:?}", comp.warnings);
    assert!(djlive::version::legacy_canonical(&comp.text).starts_with(&format!("import {}\n", check(lib).compiled.unwrap().cid)));
    // reformatted, the same program
    let mut rng = Rng(0x1117);
    for _ in 0..20 {
        let text = reformat(&src, &mut rng);
        assert_eq!(check_with(&text, &env).compiled.expect("compiles").cid, comp.cid, "{text}");
    }
    let l = lines(&src, &env);
    assert_eq!(l, vec!["Now deck B: harmony <chord(dm7) chord(g7) chord(cmaj7) chord(fmaj7)> | melody markov(am, 0.45) oct=4".to_string()]);
}

#[test]
fn an_unwired_input_suggests_what_an_import_brought() {
    let snips = snippets(&[("harmony", HARMONY)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"harmony\"\ndef bad() in: chords {\n  deck B: harmony $in\n}\nbad()\n";
    let e = errs_with(src, &env);
    assert_eq!(e[0].2, "`bad.in` is not wired: `bad` needs chords and nothing feeds it");
    assert_eq!(help_of_with(src, &env), "wire harmony_changes.out -> bad.in");
}

#[test]
fn an_import_holds_ports_of_every_kind_and_the_text_prints_them_back() {
    let lib = "def tempo_src() out: tempo {\n  emit 128\n}\ndef hats() out: drums {\n  emit x*8\n}\n";
    let snips = snippets(&[("kit", lib)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"kit\"\ndef consumer() in: drums, tempo {\n  deck A: hihat $drums\n  master: delay 0.2\n}\nwire hats.out -> consumer.drums\nwire tempo_src.out -> consumer.tempo\nconsumer()\n";
    let comp = check_with(src, &env).compiled.unwrap();
    // the canonical text is a fixed point under the same store
    let again = check_with(&comp.text, &env).compiled.unwrap();
    assert_eq!((again.text.clone(), again.cid.clone()), (comp.text.clone(), comp.cid.clone()));
    assert!(comp.text.contains("def consumer() in: drums, tempo {\n"), "{}", comp.text);
}

// ---- the laws

/// A chain of modules `m0 -> m1 -> ... -> last`: the first makes a pattern, each other adds a transposition, the
/// last plays what it is given. What its line must say is worked out on the side.
struct Chain {
    base: String,
    shifts: Vec<i32>,
}

impl Chain {
    fn random(rng: &mut Rng) -> Chain {
        let bases = ["[c2 e2 g2]", "c2 ~ e2 g2", "<c2 e2> g2", "[c2 ~ ~ e2]", "x ~ x ~", "c2"];
        let n = 1 + rng.below(5) as usize;
        Chain { base: bases[rng.below(bases.len() as u64) as usize].to_string(), shifts: (0..n).map(|_| 1 + rng.below(11) as i32).collect() }
    }

    /// Modules `from..to` of the chain (0 makes, 1..=n shift, n + 1 plays).
    fn defs(&self, from: usize, to: usize) -> String {
        let n = self.shifts.len();
        let mut s = String::new();
        for i in from..to {
            if i == 0 {
                s.push_str(&format!("def m0() out: bass {{\n  emit {}\n}}\n", self.base));
            } else if i <= n {
                s.push_str(&format!("def m{i}() in: bass out: bass {{\n  emit transpose($in, {})\n}}\n", self.shifts[i - 1]));
            } else {
                s.push_str("def last() in: bass {\n  deck A: bass $in\n}\n");
            }
        }
        s
    }

    fn name(&self, i: usize) -> String {
        if i == self.shifts.len() + 1 {
            "last".to_string()
        } else {
            format!("m{i}")
        }
    }

    /// Module `i` feeds module `i + 1`.
    fn wire(&self, i: usize) -> String {
        format!("wire {}.out -> {}.in\n", self.name(i), self.name(i + 1))
    }

    /// The line the call of the last module expands to.
    fn expected(&self) -> String {
        let mut v = if one_term(&self.base) { self.base.clone() } else { format!("[{}]", self.base) };
        for s in &self.shifts {
            v = format!("transpose({v}, {s})");
        }
        format!("Now deck A: bass {v}")
    }

    fn total(&self) -> usize {
        self.shifts.len() + 2
    }
}

/// The bases above: a group, or a single word.
fn one_term(s: &str) -> bool {
    (s.starts_with('[') && s.ends_with(']') && !s[1..s.len() - 1].contains('[')) || s.split_whitespace().count() == 1
}

#[test]
fn a_chain_expands_to_what_the_hops_make() {
    let mut rng = Rng(0xc4a1);
    for _ in 0..40 {
        let ch = Chain::random(&mut rng);
        let mut src = ch.defs(0, ch.total());
        for i in 0..ch.total() - 1 {
            src.push_str(&ch.wire(i));
        }
        src.push_str("last()\n");
        assert_eq!(lines(&src, &Env::none()), vec![ch.expected()], "{src}");
    }
}

#[test]
fn reformatting_a_random_wiring_keeps_its_cid_and_its_expansion() {
    let mut rng = Rng(0x7e57);
    for _ in 0..25 {
        let ch = Chain::random(&mut rng);
        let mut src = ch.defs(0, ch.total());
        for i in 0..ch.total() - 1 {
            src.push_str(&ch.wire(i));
        }
        src.push_str("last()\n");
        let base = ok(&src);
        // the canonical text is a fixed point
        let again = ok(&base.text);
        assert_eq!((&again.text, &again.cid), (&base.text, &base.cid));
        for _ in 0..3 {
            let text = reformat(&src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text)));
            assert_eq!(got.cid, base.cid, "forked:\n{text}");
            assert_eq!(lines(&text, &Env::none()), vec![ch.expected()]);
        }
    }
}

#[test]
fn the_order_the_wires_are_written_in_does_not_change_what_flows() {
    let mut rng = Rng(0x0dd);
    for _ in 0..40 {
        let ch = Chain::random(&mut rng);
        let mut order: Vec<usize> = (0..ch.total() - 1).collect();
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        let wires: String = order.iter().map(|i| ch.wire(*i)).collect();
        let src = format!("{}{wires}last()\n", ch.defs(0, ch.total()));
        assert_eq!(lines(&src, &Env::none()), vec![ch.expected()], "{src}");
        // the wires may come before the defs they name, and the call before both
        let src2 = format!("{wires}{}last()\n", ch.defs(0, ch.total()));
        assert_eq!(lines(&src2, &Env::none()), vec![ch.expected()], "{src2}");
    }
}

#[test]
fn any_bracketing_of_the_wiring_expands_to_the_same_lines() {
    // (m0 ; ... ; mk) saved as a snippet with its wires, then the rest wired onto it: every cut of the chain,
    // either side in the snippet, and three snippets wired together
    let mut rng = Rng(0xb4ac);
    let mut cuts_tried = 0;
    for _ in 0..30 {
        let ch = Chain::random(&mut rng);
        let total = ch.total();
        let mut flat = ch.defs(0, total);
        for i in 0..total - 1 {
            flat.push_str(&ch.wire(i));
        }
        flat.push_str("last()\n");
        let want = vec![ch.expected()];
        assert_eq!(lines(&flat, &Env::none()), want);
        for cut in 0..total - 1 {
            // the left part is a snippet (its defs and the wires among them) ...
            let mut left = ch.defs(0, cut + 1);
            for i in 0..cut {
                left.push_str(&ch.wire(i));
            }
            let snips = snippets(&[("left", &left)]);
            let env = Env { snippets: Some(&snips), ..Env::default() };
            // ... and the right part is written in the program, with the wire that crosses the cut
            let mut right = String::from("import \"left\"\n");
            right.push_str(&ch.defs(cut + 1, total));
            for i in cut..total - 1 {
                right.push_str(&ch.wire(i));
            }
            right.push_str("last()\n");
            assert_eq!(lines(&right, &env), want, "cut {cut}\n{right}\nleft:\n{left}");
            // and the other way round: the right part is the snippet
            let mut rsnip = ch.defs(cut + 1, total);
            for i in cut + 1..total - 1 {
                rsnip.push_str(&ch.wire(i));
            }
            let snips2 = snippets(&[("right", &rsnip)]);
            let env2 = Env { snippets: Some(&snips2), ..Env::default() };
            let mut main = String::from("import \"right\"\n");
            main.push_str(&ch.defs(0, cut + 1));
            for i in 0..=cut {
                main.push_str(&ch.wire(i));
            }
            main.push_str("last()\n");
            assert_eq!(lines(&main, &env2), want, "cut {cut} (right in the snippet)\n{main}\nright:\n{rsnip}");
            cuts_tried += 1;
        }
        // three snippets, each a bracket, and the program only wires them together
        if total >= 4 {
            let (a, b) = (1, total - 2);
            let mut s1 = ch.defs(0, a + 1);
            for i in 0..a {
                s1.push_str(&ch.wire(i));
            }
            let mut s2 = ch.defs(a + 1, b + 1);
            for i in a + 1..b {
                s2.push_str(&ch.wire(i));
            }
            let s3 = ch.defs(b + 1, total);
            let snips = snippets(&[("one", &s1), ("two", &s2), ("three", &s3)]);
            let env = Env { snippets: Some(&snips), ..Env::default() };
            let src = format!("import \"one\"\nimport \"two\"\nimport \"three\"\n{}{}last()\n", ch.wire(a), ch.wire(b));
            assert_eq!(lines(&src, &env), want, "{src}");
        }
    }
    assert!(cuts_tried > 30);
}

#[test]
fn the_identity_module_is_invisible() {
    let mut rng = Rng(0x1d);
    for _ in 0..30 {
        let ch = Chain::random(&mut rng);
        let total = ch.total();
        // `id` sits between module `at` and the one after it
        let at = rng.below(total as u64 - 1) as usize;
        let mut src = ch.defs(0, total);
        src.push_str("def id() in: bass out: bass {\n  emit $in\n}\n");
        for i in 0..total - 1 {
            if i == at {
                src.push_str(&format!("wire {}.out -> id.in\nwire id.out -> {}.in\n", ch.name(i), ch.name(i + 1)));
            } else {
                src.push_str(&ch.wire(i));
            }
        }
        src.push_str("last()\n");
        assert_eq!(lines(&src, &Env::none()), vec![ch.expected()], "{src}");
    }
}

#[test]
fn independent_modules_may_be_called_in_either_order() {
    let a = program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\nwalker(4)\nthump()\n");
    let b = program("wire changes.out -> walker.chords\nwire home.out -> walker.key\nwire walker.out -> thump.in\nthump()\nwalker(4)\n");
    let mut la = lines(&a, &Env::none());
    let mut lb = lines(&b, &Env::none());
    la.sort();
    lb.sort();
    assert_eq!(la, lb);
    // and the nets are the same: the order of lines is not in them
    assert_eq!(ok(&a).net_cids(), ok(&b).net_cids());
}

#[test]
fn a_program_is_the_same_expansion_every_time() {
    let src = program(WIRED);
    let first = (ok(&src).cid, lines(&src, &Env::none()));
    for _ in 0..5 {
        assert_eq!(first, (ok(&src).cid, lines(&src, &Env::none())));
    }
}

// ---- random wirings: loops are found exactly when there is one

#[test]
fn the_loop_finder_agrees_with_a_topological_sort() {
    let mut rng = Rng(0x1009);
    for round in 0..300 {
        let n = 2 + rng.below(7) as usize;
        let m = rng.below(2 * n as u64) as usize;
        let edges: Vec<(String, String)> = (0..m).map(|_| (format!("d{}", rng.below(n as u64)), format!("d{}", rng.below(n as u64)))).collect();
        let found = djlive::ports::wiring_cycles(&edges);
        // the oracle: peel nodes with no incoming edge until none can be
        let mut left: Vec<(String, String)> = edges.clone();
        loop {
            let nodes: std::collections::BTreeSet<String> = left.iter().flat_map(|(a, b)| [a.clone(), b.clone()]).collect();
            let free: Vec<String> = nodes.iter().filter(|n| !left.iter().any(|(_, b)| b == *n)).cloned().collect();
            if free.is_empty() {
                break;
            }
            left.retain(|(a, _)| !free.contains(a));
        }
        assert_eq!(found.is_empty(), left.is_empty(), "round {round}: {edges:?} -> {found:?}");
        // every loop reported is a loop: each step is an edge, and it closes
        for cyc in &found {
            for (i, a) in cyc.iter().enumerate() {
                let b = &cyc[(i + 1) % cyc.len()];
                assert!(edges.contains(&(a.clone(), b.clone())), "round {round}: {cyc:?} is not a loop of {edges:?}");
            }
        }
    }
}

#[test]
fn a_random_wiring_of_modules_is_a_loop_error_exactly_when_it_has_a_loop() {
    let mut rng = Rng(0xfeed);
    for round in 0..60 {
        let n = 2 + rng.below(4) as usize;
        let mut src = String::new();
        for i in 0..n {
            src.push_str(&format!("def q{i}() in: bass out: bass {{\n  emit $in\n}}\n"));
        }
        let mut edges = Vec::new();
        let mut fed = std::collections::BTreeSet::new();
        for _ in 0..rng.below(2 * n as u64 + 1) {
            let (a, b) = (rng.below(n as u64) as usize, rng.below(n as u64) as usize);
            // each input is wired once
            if fed.insert(b) {
                src.push_str(&format!("wire q{a}.out -> q{b}.in\n"));
                edges.push((format!("q{a}"), format!("q{b}")));
            }
        }
        let has_loop = !djlive::ports::wiring_cycles(&edges).is_empty();
        let c = check(&src);
        let loop_said = c.diagnostics.iter().any(|d| d.is_error() && d.message.starts_with("the wires make a loop"));
        assert_eq!(loop_said, has_loop, "round {round}\n{src}\n{}", djlive::diag::render_all(&c.diagnostics, &src));
        assert_eq!(c.ok(), !has_loop, "round {round}");
    }
}

// ---- the editor's marks, and the words

#[test]
fn spans_and_via_point_at_the_def_and_the_call() {
    let src = program(WIRED);
    let comp = ok(&src);
    let parsed = djlive::parse::parse(&src);
    let mut diags = Vec::new();
    let lw = djlive::expand::lower(&parsed, &src, &mut diags);
    let call = lw.globals.iter().find(|f| print::line(&f.line).contains("kick")).unwrap();
    assert_eq!(call.via.len(), 1);
    assert_eq!(call.via[0].text(&src), "thump()");
    assert!(call.span.text(&src).starts_with("deck A: kick"), "{}", call.span.text(&src));
    assert!(call.span.start < src.find("wire changes").unwrap(), "the line's span is inside the def's own text");
    // the hits of the bass part trace to the `$in` of the def
    let a = comp.decks.get(&djlive::ast::Deck::A).expect("deck A");
    let bass = a.parts.iter().find(|p| p.role == "bass").expect("a bass part");
    let texts: Vec<&str> = bass.spans.iter().flat_map(|c| c.iter().map(|s| s.text(&src))).collect();
    assert!(texts.contains(&"$in"), "{texts:?}");
    assert_eq!(bass.via.iter().map(|s| s.text(&src)).collect::<Vec<_>>(), vec!["thump()"]);
}

#[test]
fn the_new_words_are_reserved_and_highlighted() {
    // `emit` starts a statement of a def's body: it names nothing; the port types are syntax in a def's header only
    let e = errs("let emit = 4 4\n");
    assert!(said(&e, "cannot name a let"), "emit: {e:?}");
    for w in ["chords", "drums", "rhythm", "pattern"] {
        let c = check(&format!("let {w} = x ~ x\ndeck A: kick {w}\n"));
        assert!(c.ok(), "{w}: {}", djlive::diag::render_all(&c.diagnostics, ""));
    }
    let src = "def foo() in: chords out: bass {\n  emit c2\n}\nwire alpha.out -> foo.in\n";
    let classes: Vec<(String, djlive::highlight::Class)> = djlive::highlight::highlight(src).into_iter().map(|(s, c)| (s.text(src).to_string(), c)).collect();
    for w in ["in", "out", "chords", "emit", "wire"] {
        assert!(classes.iter().any(|(t, c)| t == w && *c == djlive::highlight::Class::Keyword), "{w}: {classes:?}");
    }
}

#[test]
fn a_wire_belongs_at_the_top() {
    let src = format!("{PIECES}section s 4 bars {{\n  wire changes.out -> walker.chords\n}}\n");
    let e = errs(&src);
    assert!(said(&e, "a `wire` belongs at the top"), "{e:?}");
}

#[test]
fn a_header_that_is_wrong_is_said_with_what_to_write() {
    let src = "def foo() out: bass in: chords {\n  emit c2\n}\n";
    let e = errs(src);
    assert!(e[0].2.contains("`in:` is in the wrong place"), "{e:?}");
    assert_eq!(help_of(src), "write `in:` before `out:`");
    let src = "def foo() in: chord {\n  deck A: kick x\n}\n";
    let c = check(src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(d.message, "`chord` is not a port type");
    assert_eq!(d.help.as_deref(), Some("did you mean `chords`?"));
    assert_eq!(d.expected.len(), 9);
    assert!(said(&errs("def foo() in: bass, bass {\n  deck A: kick x\n}\n"), "`bass` is already a port of `in:` (line 1)"));
    assert!(said(&errs("def foo() in {\n  deck A: kick x\n}\n"), "followed by a colon"));
    assert!(said(&errs("def foo() in: {\n  deck A: kick x\n}\n"), "expected a port type"));
    // a wire that forgets its port, or its arrow
    assert!(said(&errs("wire changes -> walker.chords\n"), "needs a port"));
    assert!(said(&errs("wire changes.out walker.chords\n"), "expected `->`"));
}

#[test]
fn the_example_of_the_docs_compiles() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/ports.beatscript")).unwrap();
    let c = ok(&src);
    assert_eq!(c.decks.len(), 2);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}
