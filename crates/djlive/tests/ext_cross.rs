//! Features that meet: tests the integrator writes when two branches that were built apart land on one. Each test names
//! the features it joins. (Each feature's own tests are in `ext_<feature>.rs`.)

mod common;

use common::{reformat, Rng};
use djlive::compile::check_with;
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

/// Every flat line the program expands to, printed, in order.
fn lines(src: &str) -> Vec<String> {
    let parsed = djlive::parse::parse(src);
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

/// A store of saved snippets (each may import the ones before it) and a program's expansion against it.
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

fn lines_with(src: &str, env: &Env) -> Vec<String> {
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

// ---- F11 ports with F6 euclid / polymeter and F7 chance

const GROOVE: &str = "\
tempo 120
key am
def groove() out: rhythm {
  emit x*8?0.7 ~ e(3,8,2)
}
def hats() out: drums {
  emit maybe([x ~ x ~], 0.8, 4) choose [x ~ [x x]] weights 3 1 1 seed=2
}
def lay_kick() in: rhythm {
  deck A: kick $in
}
def lay_hats() in: drums {
  deck A: hihat/12 $in
}
wire groove.out -> lay_kick.in
wire hats.out -> lay_hats.in
lay_kick()
lay_hats()
";

#[test]
fn a_rhythm_port_carries_chance_choose_maybe_and_euclid() {
    let c = ok(GROOVE);
    assert!(c.text.contains("  emit x*8?0.7 ~ e(3, 8, 2)\n"), "{}", c.text);
    let l = lines(GROOVE);
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[0], "deck A: kick [x*8?0.7 ~ e(3, 8, 2)]");
    assert!(l[1].starts_with("deck A: hihat/12 [maybe([x ~ x ~], 0.8, 4) choose [x ~ [x x]] weights 3 1 1 seed=2]"), "{}", l[1]);
    // the chance flows into the net: another chance is another net, the same text the same net
    let net = |src: &str| ok(src).decks[&djlive::ast::Deck::A].net_cid();
    assert_eq!(net(GROOVE), net(GROOVE));
    assert_ne!(net(GROOVE), net(&GROOVE.replace("?0.7", "?0.2")));
}

#[test]
fn a_program_with_ports_chance_and_polymeter_is_canonical_and_keeps_its_cid_under_reformatting() {
    stable(GROOVE, 0xf11_f6_f7);
}

#[test]
fn a_chance_or_choose_that_holds_a_pitch_is_not_a_rhythm() {
    for emit in ["c3?0.5 x", "x*4?0.5 [c3 x]", "choose [x c3] weights 1 1", "maybe([c3 x], 0.5)"] {
        let src = format!("def foo() out: drums {{\n  emit {emit}\n}}\n");
        let e = errs(&src);
        assert!(said(&e, "pitch"), "{emit}: {e:?}");
    }
    // the same things holding hits are fine, in a drums port and in a rhythm port and in a bass
    for emit in ["x?0.5 ~", "x*4?0.5 [x x]", "choose [x ~ [x x]] weights 1 1 1", "maybe([x ~ x], 0.5)", "e(5,8,1)"] {
        for ty in ["drums", "rhythm"] {
            ok(&format!("def foo() out: {ty} {{\n  emit {emit}\n}}\n"));
        }
        ok(&format!("def foo() out: bass {{\n  emit {emit}\n}}\n"));
    }
    // a `let` is looked through: a chance of hits is a rhythm, a chance of a pitch is not
    ok("let r = x?0.5 ~ x\ndef foo() out: rhythm {\n  emit r\n}\n");
    let e = errs("let r = c3?0.5 ~ x\ndef foo() out: drums {\n  emit r\n}\n");
    assert!(said(&e, "pitch"), "{e:?}");
    // and pitches are fine in a melody port
    ok("def foo() out: melody {\n  emit c3?0.5 choose [e3 g3] weights 3 1 maybe([c4 d4], 0.5)\n}\n");
}

#[test]
fn a_wired_value_goes_into_chance_and_polymeter_as_a_unit() {
    let src = "\
def riff() out: bass {
  emit c2 ~ e2 g2
}
def lay_kick() in: bass {
  deck A: bass/12 $in?0.5 | melody maybe($in, 0.25, 3)
}
wire riff.out -> lay_kick.in
lay_kick()
";
    let l = lines(src);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].contains("bass/12 [c2 ~ e2 g2]?0.5"), "{}", l[0]);
    assert!(l[0].contains("maybe([c2 ~ e2 g2], 0.25, 3)"), "{}", l[0]);
    stable(src, 0xf11_f7);
}

#[test]
fn an_euclid_with_a_rotation_is_a_rhythm_input_too() {
    let src = "\
def pulse() out: rhythm {
  emit e(5,12,3)
}
def lay_kick() in: rhythm {
  deck B: bass/12 $in oct=2 | hihat/16 x
}
wire pulse.out -> lay_kick.in
lay_kick()
";
    let c = ok(src);
    let l = lines(src);
    assert_eq!(l[0], "deck B: bass/12 e(5, 12, 3) oct=2 | hihat x");
    // the polymeter is still reported: 12 and 16 sixteenths come round together after 48
    assert!(c.warnings.iter().any(|d| d.message.starts_with("period:")), "{:?}", c.warnings.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    stable(src, 0xf11_f6);
}

// ---- the words and the marks

#[test]
fn the_keywords_of_both_features_are_highlighted() {
    use djlive::highlight::{highlight, Class};
    let src = "def f() in: rhythm out: bass {\n  emit bass choose [c2 e2] weights 3 1\n}\nwire a.out -> f.in\n";
    let marks = highlight(src);
    let class_of = |word: &str| {
        let at = src.find(word).unwrap();
        marks.iter().find(|(s, _)| s.start == at).map(|(_, c)| *c).unwrap()
    };
    for w in ["in:", "out:", "emit", "choose", "weights", "wire"] {
        let w = w.trim_end_matches(':');
        assert_eq!(class_of(w), Class::Keyword, "{w}");
    }
}

#[test]
fn a_reserved_word_of_either_feature_cannot_name_a_def_a_let_or_a_section() {
    for word in ["emit", "choose", "maybe"] {
        let e = errs(&format!("let {word} = x ~ x\n"));
        assert!(said(&e, "word of the language") || said(&e, "generator or transform"), "{word}: {e:?}");
        let e = errs(&format!("def {word}() {{\n  deck A: kick x\n}}\n"));
        assert!(said(&e, "word of the language") || said(&e, "generator or transform"), "{word}: {e:?}");
    }
    // the words of a def's ports and a choice's weights are syntax in one place only: they name a let and a def
    for word in ["chords", "drums", "rhythm", "pattern", "weights"] {
        ok(&format!("let {word} = x ~ x\ndeck A: kick {word}\n"));
        ok(&format!("def {word}() {{\n  deck A: kick x\n}}\n{word}()\n"));
    }
}

// ---- tokens that must touch (F7 `?`, F6 `/N`, F2 `==`, the accent) through a macro's substitution and an import

#[test]
fn a_chance_in_a_def_argument_still_touches_its_term() {
    let src = "def line(pat) {\n  deck A: hihat $pat\n}\nline(x*8?0.7 ~ [x x]?50%)\n";
    let l = lines(src);
    assert_eq!(l[0], "deck A: hihat x*8?0.7 ~ [x x]?0.5");
    // and an accent and a slide, which have to touch their step too
    let l = lines("def line(pat) {\n  deck A: bass $pat\n}\nline(c2! e2^ g2)\n");
    assert!(l[0].contains("c2!"), "{}", l[0]);
}

#[test]
fn a_chance_in_an_imported_def_still_touches_its_term() {
    let snips = snippets(&[("grooves", "def hats() {\n  deck A: hihat x*8?0.7 | clap maybe([~ x], 0.5, 2)\n}\ndef roles() {\n  deck B: bass/12 e(5,12) oct=2\n}\n")]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"grooves\"\nhats()\nroles()\n";
    let l = lines_with(src, &env);
    assert!(l[0].contains("x*8?0.7"), "{l:?}");
    assert!(l[1].contains("bass/12"), "{l:?}");
}

// ---- the editor's marks through a wired value, a def argument and an import (F11, F7, F6)

/// Every span a deck's hits carry (and the line's own and its `via`) is a real, non-empty piece of the text.
fn marks_are_real(src: &str, c: &djlive::compile::Compiled) -> usize {
    let mut n = 0;
    for deck in c.decks.values() {
        for part in &deck.parts {
            for chain in &part.spans {
                for s in chain {
                    assert!(!s.is_empty() && s.end <= src.len(), "an empty or outside span {s:?} in {src:?}");
                }
            }
            for s in &part.via {
                assert!(!s.is_empty() && s.end <= src.len(), "{s:?}");
            }
            n += part.cycles.iter().map(Vec::len).sum::<usize>();
        }
    }
    n
}

#[test]
fn the_hits_of_a_chance_that_came_through_a_wire_light_the_place_it_was_wired_into() {
    let c = ok(GROOVE);
    let hits = marks_are_real(GROOVE, &c);
    assert!(hits > 0);
    let kick = c.decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "kick").unwrap();
    let texts: Vec<&str> = kick.spans.iter().flat_map(|ch| ch.iter().map(|s| s.text(GROOVE))).collect();
    assert!(texts.contains(&"$in"), "{texts:?}");
    assert_eq!(kick.via.iter().map(|s| s.text(GROOVE)).collect::<Vec<_>>(), vec!["lay_kick()"]);
}

#[test]
fn the_hits_of_a_chance_that_came_through_an_argument_or_an_import_have_real_spans() {
    let src = "def line(pat) {\n  deck A: hihat $pat seed=2\n}\nline(x*8?0.7 ~ [x x]?50%)\n";
    let c = ok(src);
    assert!(marks_are_real(src, &c) > 0);
    let snips = snippets(&[("grooves", "def hats() {\n  deck A: hihat x*8?0.7 seed=2\n}\n")]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"grooves\"\nhats()\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    let comp = c.compiled.unwrap();
    assert!(marks_are_real(src, &comp) > 0);
}

// ---- a law across F11, F6 and F7: a value that goes through wires is the value written where it is used

/// A random rhythm term of the language: hits, rests, groups, euclid with and without a rotation, a chance, a choose, a maybe.
fn random_term(rng: &mut Rng, depth: u32) -> String {
    let leaf = |rng: &mut Rng| ["x", "~", "x", "x!"][rng.below(4) as usize].to_string();
    if depth == 0 {
        return leaf(rng);
    }
    match rng.below(9) {
        0 | 1 => leaf(rng),
        2 => format!("[{} {}]", random_term(rng, depth - 1), random_term(rng, depth - 1)),
        3 => {
            let n = 4 + rng.below(9);
            let k = 1 + rng.below(n);
            if rng.below(2) == 0 {
                format!("e({k},{n})")
            } else {
                format!("e({k},{n},{})", rng.below(n))
            }
        }
        4 => format!("x*{}?0.{}", 2 + rng.below(4), 1 + rng.below(9)),
        5 => format!("[{} x]?{}%", random_term(rng, depth - 1), 10 * (1 + rng.below(9))),
        6 => format!("choose [x ~ [x x]] weights {} {} 1 seed={}", 1 + rng.below(4), 1 + rng.below(4), rng.below(9)),
        7 => format!("maybe([x ~ x ~], 0.{}, {})", 1 + rng.below(9), rng.below(9)),
        _ => format!("<{} {}>", random_term(rng, depth - 1), random_term(rng, depth - 1)),
    }
}

#[test]
fn a_rhythm_that_goes_through_wires_plays_the_same_net_as_the_rhythm_written_in_place() {
    let mut rng = Rng(0xc4055_f11);
    for round in 0..40 {
        let n = 2 + rng.below(3);
        let seq: Vec<String> = (0..n).map(|_| random_term(&mut rng, 2)).collect();
        let seq = seq.join(" ");
        let wired = format!(
            "def src() out: rhythm {{\n  emit {seq}\n}}\ndef through() in: rhythm out: rhythm {{\n  emit $in\n}}\ndef sink() in: rhythm {{\n  deck A: kick $in vel=60..90\n}}\nwire src.out -> through.in\nwire through.out -> sink.in\nsink()\n"
        );
        let direct = format!("deck A: kick [{seq}] vel=60..90\n");
        let (w, d) = (check(&wired), check(&direct));
        assert!(w.ok(), "round {round}\n{wired}\n{}", djlive::diag::render_all(&w.diagnostics, &wired));
        assert!(d.ok(), "round {round}\n{direct}\n{}", djlive::diag::render_all(&d.diagnostics, &direct));
        let (w, d) = (w.compiled.unwrap(), d.compiled.unwrap());
        let (wa, da) = (&w.decks[&djlive::ast::Deck::A], &d.decks[&djlive::ast::Deck::A]);
        assert_eq!(wa.net_cid(), da.net_cid(), "round {round}: {seq}");
        // the same plays, bar for bar
        let (wp, dp) = (&wa.parts[0], &da.parts[0]);
        assert_eq!(wp.cycles.len(), dp.cycles.len(), "round {round}: {seq}");
        // and reformatting the wired program is the same program
        let again = reformat(&wired, &mut rng);
        assert_eq!(ok(&again).cid, w.cid, "round {round}\n{again}");
    }
}

// ---- a repeat's copies come to different numbers (F2): an error only a later copy has is said (found joining F9 with F2)

#[test]
fn an_error_that_only_a_later_copy_of_a_repeat_has_is_said_and_not_dropped_with_its_line() {
    // copy 1 sings over 1 bar, copy 2 over 3/2 of a bar: the line is not silently left out of the program
    let src = "deck B: melody c4 e4 g4 e4\nrepeat 2 every 4 bars {\n  deck B: sing \"la\" on melody voice=robot bars=$i / 2 + 1\n}\n";
    let e = errs(src);
    assert!(e.iter().any(|(l, _, m)| *l == 3 && m.contains("whole number")), "{e:?}");
    // an error every copy has is said once, not once a copy
    let e = errs("deck B: melody c4 e4 g4 e4\nrepeat 3 every 4 bars {\n  deck B: sing \"la\" on melody voice=robot bars=0 * $i\n}\n");
    assert_eq!(e.iter().filter(|(_, _, m)| m.contains("1 to 32")).count(), 1, "{e:?}");
}
