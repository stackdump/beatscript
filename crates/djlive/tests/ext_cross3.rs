//! Features that meet, third file: scale degrees, Roman numerals, a key per section and voice-lead (F5) against
//! arithmetic (F2), ports (F11), euclid / polymeter (F6), chance (F7), timing (F1), defs, repeats and imports. The tests
//! the integrator writes when two branches that were built apart land on one. (`ext_cross.rs` and `ext_cross2.rs` are
//! the first two files; each feature's own tests are in `ext_<feature>.rs`.)

mod common;

use common::{glued_pair, reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check_with, Compiled};
use djlive::env::{Env, StaticSnippets};
use djlive::lex::{lex, put_at, touches, Tok, Token};
use djlive::{check, diag};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message, help) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String, String)> {
    let c = check_with(src, &Env::none());
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone().unwrap_or_default())).collect()
}

fn said(e: &[(usize, usize, String, String)], what: &str) -> bool {
    e.iter().any(|(_, _, m, h)| m.contains(what) || h.contains(what))
}

/// A store of saved snippets (each may import the ones before it).
fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let c = check_with(text, &env);
        let canon = c.compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}\n{}", diag::render_all(&c.diagnostics, text)));
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    snips
}

/// (step, note) of the first cycle of a part.
fn played(c: &Compiled, deck: Deck, role: &str) -> Vec<(u8, u8)> {
    let p = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part"));
    p.cycles[0].iter().map(|h| (h.step, h.note)).collect()
}

/// (step, note) of cycle `n` of a part.
fn played_cycle(c: &Compiled, deck: Deck, role: &str, n: usize) -> Vec<(u8, u8)> {
    let p = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part"));
    p.cycles[n % p.cycles.len()].iter().map(|h| (h.step, h.note)).collect()
}

/// `a` and `b` are two ways to write one part: the same notes in the same places.
fn same_notes(a: &str, b: &str, deck: Deck, role: &str) -> Vec<(u8, u8)> {
    let (ca, cb) = (ok(a), ok(b));
    let (pa, pb) = (played(&ca, deck, role), played(&cb, deck, role));
    assert!(!pa.is_empty(), "{a}\nplays nothing");
    assert_eq!(pa, pb, "\n{a}\nplays other notes than\n{b}");
    pa
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
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
}

fn toks(src: &str) -> Vec<Token> {
    let l = lex(src);
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    l.tokens.into_iter().filter(|t| t.tok != Tok::Eof).collect()
}

// ---- the tokens of F5 that must touch (`5'`, `1,` in deg( )), through lex::touches and the test helper

#[test]
fn every_pair_the_test_helper_keeps_glued_is_one_the_lexer_says_touches() {
    // `tests/common::glued_pair` and `lex::touches` are one list kept twice: whatever the first keeps together when it
    // reformats a program, the second must keep together when a macro or an import puts tokens in place
    let samples = [
        Tok::Word("a".into()),
        Tok::Num { value: 1.0, int: true },
        Tok::Unit("db".into()),
        Tok::Dot,
        Tok::Question,
        Tok::Apostrophe,
        Tok::Slash,
        Tok::PlusMinus,
        Tok::OctDown,
        Tok::Eq,
        Tok::Lt,
        Tok::Gt,
        Tok::Bang,
        Tok::Caret,
        Tok::Comma,
        Tok::LParen,
        // `c2~3` (pattern sugar) and the expression marks before it (`x.~2`, `x&~2`). (Not `:`: the helper keeps a
        // colon glued to whatever is before it, which is how the text writes one; the ratchet's pair is below.)
        Tok::Tilde,
        Tok::RBrack,
        Tok::Ghost,
        Tok::Amp,
    ];
    for p in &samples {
        for t in &samples {
            if glued_pair(p, t) {
                assert!(touches(p, t), "`{p:?}` `{t:?}` are kept glued by the test helper but not by lex::touches");
            }
        }
    }
    // a ratchet `x:3` (and `[x x]:3`) and an elongate `c2~3`, by name: a macro or an import keeps them one step
    for step in [Tok::Word("x".into()), Tok::RBrack, Tok::Ghost, Tok::Amp] {
        assert!(touches(&step, &Tok::Colon) && touches(&step, &Tok::Tilde), "{step:?}");
    }
    assert!(touches(&Tok::Colon, &Tok::Num { value: 3.0, int: true }) && touches(&Tok::Tilde, &Tok::Num { value: 3.0, int: true }));
    // the octave marks of F5, by name
    assert!(touches(&Tok::Num { value: 5.0, int: true }, &Tok::Apostrophe));
    assert!(touches(&Tok::Num { value: 1.0, int: true }, &Tok::OctDown));
    assert!(touches(&Tok::Word("b3".into()), &Tok::OctDown));
    assert!(touches(&Tok::Dollar("a".into()), &Tok::OctDown));
}

#[test]
fn a_macro_or_an_import_that_puts_degree_marks_in_place_keeps_them_touching() {
    // `deg(1, 3 5')` re-spanned at one place (what `fill` and an import do to the tokens they carry): the octave marks
    // stay glued to their degrees, whatever the text there was
    let src = "deg(1, 3 5' #4,)";
    let original = toks(src);
    let at = djlive::diag::Span::new(40, 52);
    let moved = put_at(&original, at);
    assert_eq!(moved.len(), original.len());
    for (i, t) in moved.iter().enumerate() {
        if matches!(t.tok, Tok::OctDown | Tok::Apostrophe) {
            assert_eq!(moved[i - 1].span.end, t.span.start, "{:?} after {:?} no longer touches", t.tok, moved[i - 1].tok);
        }
    }
    // and again (an import of an import): the same
    let again = put_at(&moved, at);
    for (i, t) in again.iter().enumerate() {
        if matches!(t.tok, Tok::OctDown | Tok::Apostrophe) {
            assert_eq!(again[i - 1].span.end, t.span.start);
        }
    }
}

// ---- F5 degrees through defs, wires, imports and repeats

/// A def with a degree for an argument, played in a melody part.
fn riff(arg: &str) -> String {
    format!("key am\ndef riff(p) {{\n  deck B: melody deg(1 $p 5)\n}}\nriff({arg})\n")
}

fn riff_literal(deg: &str) -> String {
    format!("key am\ndeck B: melody deg(1 {deg} 5)\n")
}

#[test]
fn a_sharp_a_flat_and_an_octave_mark_go_through_a_def_argument() {
    // `#4`, `b3`, `5'`, `3''` as arguments: the same program as with the degree written where `$a` is
    for d in ["#4", "b3", "5'", "3''", "#2'", "b7"] {
        let got = same_notes(&riff(d), &riff_literal(d), Deck::B, "melody");
        assert_eq!(got.len(), 3, "{d}");
    }
    // the one that differs from its plain degree
    let plain = played(&ok(&riff("4")), Deck::B, "melody");
    let sharp = played(&ok(&riff("#4")), Deck::B, "melody");
    assert_eq!(sharp[1].1, plain[1].1 + 1);
    let up = played(&ok(&riff("5'")), Deck::B, "melody");
    assert_eq!(up[1].1, played(&ok(&riff("5")), Deck::B, "melody")[1].1 + 12);
    // the canonical text keeps the call as it was written
    assert!(ok(&riff("#4")).text.contains("riff(#4)"), "{}", ok(&riff("#4")).text);
    assert!(ok(&riff("5'")).text.contains("riff(5')"), "{}", ok(&riff("5'")).text);
    stable(&riff("#4"), 0xf5_01);
    stable(&riff("5'"), 0xf5_02);
}

#[test]
fn an_octave_down_goes_through_a_def_as_a_whole_deg_and_as_the_degree_before_a_comma() {
    // a `deg( )` as the argument carries its commas with it
    let by_arg = "key am\ndef tune(p) {\n  deck B: melody $p\n}\ntune(deg(1, 3 b3' #4,))\n";
    let literal = "key am\ndeck B: melody deg(1, 3 b3' #4,)\n";
    let got = same_notes(by_arg, literal, Deck::B, "melody");
    assert_eq!(got.len(), 4);
    // and a degree that a macro puts before a comma (`deg($a, 3)`) is an octave down, as `deg(1, 3)` is
    let by_dollar = "key am\ndef lower(p) {\n  deck B: melody deg($p, 3)\n}\nlower(2)\n";
    let got = same_notes(by_dollar, "key am\ndeck B: melody deg(2, 3)\n", Deck::B, "melody");
    assert_eq!(got.len(), 2, "{got:?}");
    let plain = played(&ok("key am\ndeck B: melody deg(2 3)\n"), Deck::B, "melody");
    assert_eq!(got[0].1 + 12, plain[0].1, "the degree before the comma is an octave down");
    // a stack comma has no space after it, whatever the macro puts there
    let stack = same_notes("key am\ndef stk(p) {\n  deck B: melody deg($p,3)\n}\nstk(1)\n", "key am\ndeck B: melody deg(1,3)\n", Deck::B, "melody");
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[0].0, stack[1].0, "stacked: both on one step");
    stable(by_dollar, 0xf5_03);
    stable(by_arg, 0xf5_06);
    stable("key am\ndef stk(p) {\n  deck B: melody deg($p,3)\n}\nstk(1)\n", 0xf5_07);
}

#[test]
fn a_sharp_numeral_and_a_chord_with_an_inversion_go_through_a_def_argument() {
    for prog in ["i VI III #iv", "I bVII V7/7 vi", "ii\u{b0} V7 Imaj7/3 #iv", "i VI/3 III VII"] {
        let by_arg = format!("key am\ndef comp(p) {{\n  deck B: harmony $p voice-lead\n}}\ncomp({prog})\n");
        let literal = format!("key am\ndeck B: harmony {prog} voice-lead\n");
        let got = same_notes(&by_arg, &literal, Deck::B, "harmony");
        assert!(got.len() >= 12, "{prog}: {got:?}");
    }
    stable("key am\ndef comp(p) {\n  deck B: harmony $p voice-lead\n}\ncomp(i VI III #iv V7/7)\n", 0xf5_04);
}

#[test]
fn a_degree_computed_from_the_repeat_is_read_inside_deg() {
    // `($i + 1)` is a degree as in any pattern (F2); only the copy that passes its guard plays
    let lit = "key am\ndeck B: melody deg(1 3 5' 5)\n";
    let src = "key am\nrepeat 4 {\n  deck B: melody deg(1 ($i + 1) ($i * 2 + 1)' 5) when $i == 2\n}\n";
    same_notes(src, lit, Deck::B, "melody");
    // an octave mark after the parenthesis, a nested call with a comma of its own, an accidental degree beside them
    let src = "key am\nrepeat 3 {\n  deck B: melody deg((min($i + 2, 5)) b3 (1 + 1)' #4,) when $i == 1\n}\n";
    same_notes(src, "key am\ndeck B: melody deg(3 b3 2' #4,)\n", Deck::B, "melody");
    // `(b3)` and `(5')` are still one degree in parentheses
    same_notes("key am\ndeck B: melody deg((b3) (5') 1)\n", "key am\ndeck B: melody deg(b3 5' 1)\n", Deck::B, "melody");
    // through a def: the argument is arithmetic, put in place in parentheses
    let src = "key am\ndef pick(p) {\n  deck B: melody deg(1 $p 5)\n}\npick(1 + 2)\n";
    same_notes(src, &riff_literal("3"), Deck::B, "melody");
    stable("key am\nrepeat 4 {\n  deck B: melody deg(1 ($i + 1) ($i * 2 + 1)' 5) when $i == 2\n}\n", 0xf5_05);
}

#[test]
fn a_computed_degree_that_is_wrong_says_so_with_its_place() {
    let e = errs("key am\ndeck B: melody deg(1 (7 / 2) 5)\n");
    assert!(said(&e, "whole number"), "{e:?}");
    let e = errs("key am\ndeck B: melody deg(1 (2 - 2) 5)\n");
    assert!(said(&e, "a degree counts from 1"), "{e:?}");
    assert_eq!(e[0].0, 2);
    // a repeat's `$i` of zero is no degree either, and the message points at the place
    let e = errs("key am\nrepeat 2 {\n  deck B: melody deg(1 $i 5) when $i == 0\n}\n");
    assert!(said(&e, "a degree counts from 1"), "{e:?}");
    // an arithmetic that does not parse is the arithmetic's error, not "is not a degree"
    let e = errs("key am\ndeck B: melody deg(1 (2 +) 5)\n");
    assert!(!said(&e, "is not a degree"), "{e:?}");
}

#[test]
fn degrees_numerals_and_voice_lead_come_through_an_import() {
    let snips = snippets(&[
        ("lines", "def lead() {\n  deck B: melody deg(1, 3 b3' #4 5'' 2,)\n}\ndef chords_of(n) {\n  deck A: harmony i VI III #iv V7/7 voice-lead\n}\n"),
        ("more", "import \"lines\"\ndef both() {\n  lead()\n  chords_of(1)\n}\n"),
    ]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let imported = check_with("import \"more\"\nkey am\nboth()\n", &env);
    assert!(imported.ok(), "{}", diag::render_all(&imported.diagnostics, "import \"more\"\nkey am\nboth()\n"));
    let by_hand = ok("key am\ndeck B: melody deg(1, 3 b3' #4 5'' 2,)\ndeck A: harmony i VI III #iv V7/7 voice-lead\n");
    let imported = imported.compiled.unwrap();
    assert_eq!(played(&imported, Deck::B, "melody"), played(&by_hand, Deck::B, "melody"));
    assert_eq!(played(&imported, Deck::A, "harmony"), played(&by_hand, Deck::A, "harmony"));
    assert!(!played(&imported, Deck::B, "melody").is_empty());
    // the marks of an imported hit point at the import's line, never at a position that is not in the text
    let src = "import \"more\"\nkey am\nboth()\n";
    for ir in imported.decks.values() {
        for p in &ir.parts {
            for chain in &p.spans {
                assert!(chain.iter().all(|s| s.end <= src.len()), "{chain:?}");
            }
        }
    }
}

// ---- F5 numerals through F11 ports

const CHANGES: &str = "\
def changes() out: chords {
  emit <i VI III VII>
}
def comp(oct) in: chords {
  deck B: harmony $in voice-lead oct=$oct
}
wire changes.out -> comp.in
comp(3)
";

#[test]
fn a_wired_progression_of_numerals_reaches_a_harmony_part_with_voice_lead() {
    let c = ok(CHANGES);
    let by_hand = ok("key c\ndeck B: harmony <i VI III VII> voice-lead oct=3\n");
    let mut chords = Vec::new();
    for bar in 0..4 {
        let wired = played_cycle(&c, Deck::B, "harmony", bar);
        assert_eq!(wired, played_cycle(&by_hand, Deck::B, "harmony", bar), "bar {bar}");
        chords.push(wired.into_iter().map(|(_, n)| i32::from(n)).collect::<Vec<_>>());
    }
    // the chords lead: the first is the root position (c minor from c3), the second is voiced near it (not VI's root position)
    assert_eq!(chords[0], vec![48, 51, 55], "{chords:?}");
    let spread = |a: &[i32], b: &[i32]| -> i32 { a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum() };
    assert!(spread(&chords[0], &chords[1]) < spread(&chords[0], &[56, 60, 63]), "the second chord is voiced for the least motion: {chords:?}");
    stable(CHANGES, 0xf5_11);
}

#[test]
fn every_numeral_the_part_reads_a_chords_port_takes_and_the_other_way() {
    // the words a `harmony` step accepts and the words a `chords` value accepts are one list
    let words = [
        "I", "ii", "iii", "IV", "V", "vi", "vii", "VII", "i", "II", "bVII", "bIII", "#iv", "#IV", "bII", "V7", "ii7", "Imaj7", "vii\u{b0}", "vii\u{f8}", "vii\u{f8}7", "ii\u{b0}7", "Vsus4", "Isus2", "Iaug", "vi6", "I9", "idim", "Vdim7",
        // not numerals, or numerals that are wrong
        "VIII", "IIII", "iVi", "Vfoo", "IVx", "b", "#v1", "viiii", "Imaj", "I7maj",
    ];
    for w in words {
        let step = check(&format!("key c\ndeck B: harmony {w}\n"));
        let wire = check(&format!("def foo() out: chords {{\n  emit {w}\n}}\n"));
        assert_eq!(step.ok(), wire.ok(), "`{w}`: a harmony part says {} and a chords value says {}", step.ok(), wire.ok());
    }
    // with an inversion after a slash
    for w in ["I/3", "V7/7", "ii/5", "Imaj7/3", "I/4", "V/6", "I/2"] {
        let step = check(&format!("key c\ndeck B: harmony {w}\n"));
        let wire = check(&format!("def foo() out: chords {{\n  emit {w}\n}}\n"));
        assert_eq!(step.ok(), wire.ok(), "`{w}`: a harmony part says {} and a chords value says {}", step.ok(), wire.ok());
    }
    // and the message of a wire says what to write
    let e = errs("def foo() out: chords {\n  emit I/4\n}\n");
    assert!(said(&e, "inversion"), "{e:?}");
}

#[test]
fn degrees_through_a_wire_are_pitches_and_a_drum_port_refuses_them() {
    let src = "def tune() out: melody {\n  emit deg(1 b3 5' 1,)\n}\ndef tuned() in: melody {\n  deck B: melody $in\n}\nwire tune.out -> tuned.in\ntuned()\nkey am\n";
    let c = ok(src);
    assert_eq!(played(&c, Deck::B, "melody").len(), 4);
    same_notes(src, "key am\ndeck B: melody deg(1 b3 5' 1,)\n", Deck::B, "melody");
    let e = errs("def hits() out: drums {\n  emit deg(1 3)\n}\n");
    assert!(said(&e, "scale degree is a pitch"), "{e:?}");
    // a numeral is a chord, not a rhythm
    let e = errs("def hits() out: rhythm {\n  emit I V\n}\n");
    assert!(said(&e, "pitch"), "{e:?}");
    stable(src, 0xf5_12);
}

#[test]
fn the_key_of_a_section_re_keys_the_numerals_a_def_plays_in_it() {
    let src = "\
tempo 120
key am
def comp() {
  deck B: harmony i VI III VII voice-lead
}
section verse 4 bars {
  comp()
}
section chorus 4 bars {
  key dm
  comp()
}
arrange verse chorus
";
    let c = ok(src);
    let a = c.segments.iter().find(|s| s.name == "verse").unwrap();
    let b = c.segments.iter().find(|s| s.name == "chorus").unwrap();
    let na: Vec<u8> = a.decks[&Deck::B].parts[0].cycles[0].iter().map(|h| h.note).collect();
    let nb: Vec<u8> = b.decks[&Deck::B].parts[0].cycles[0].iter().map(|h| h.note).collect();
    assert_eq!(na.len(), nb.len());
    assert!(na.iter().zip(&nb).all(|(x, y)| x != y), "another key, other notes: {na:?} {nb:?}");
    // a fifth apart (A minor to D minor) is five semitones up, modulo the voicing
    assert_eq!(na.iter().map(|n| u32::from(*n) % 12).collect::<std::collections::BTreeSet<_>>().len(), nb.iter().map(|n| u32::from(*n) % 12).collect::<std::collections::BTreeSet<_>>().len());
    stable(src, 0xf5_13);
}

// ---- F5 with F6 (polymeter), F7 (chance), F1 (timing), F2 (guards)

#[test]
fn numerals_and_degrees_take_a_cycle_of_their_own_chance_and_repeats() {
    // a cycle in steps: `harmony/12`, a numeral with an inversion beside the slash of the cycle
    let src = "key am\ndeck B: harmony/12 i VI/3 III VII voice-lead | melody/8 deg(1 b3 5' 1,)*2\n";
    let c = ok(src);
    assert!(!played(&c, Deck::B, "harmony").is_empty());
    assert!(!played(&c, Deck::B, "melody").is_empty());
    assert!(c.text.contains("harmony/12 i VI/3 III VII voice-lead=on"), "{}", c.text);
    assert!(c.text.contains("melody/8 deg(1 b3 5' 1,)*2"), "{}", c.text);
    stable(src, 0xf5_21);
    // chance on a deg( ) and on a numeral: the same text is the same notes; a chance of 1 keeps all
    let src = "key am\ndeck B: melody deg(1 3 5)?(1 / 2) | harmony i?(1 / 2) V\n";
    let a = ok(src);
    assert_eq!(played(&a, Deck::B, "melody"), played(&ok(src), Deck::B, "melody"));
    stable(src, 0xf5_22);
    same_notes("key am\ndeck B: melody deg(1 3 5)?1\n", "key am\ndeck B: melody deg(1 3 5)\n", Deck::B, "melody");
}

#[test]
fn a_program_that_uses_everything_is_one_canonical_text() {
    let src = "\
tempo 124
key am
def changes() out: chords {
  emit <i VI III #iv>
}
def comp(n) in: chords {
  deck B: harmony/12 $in voice-lead oct=3 | melody deg(1 b3 5' 1,)*2?(1 / 2) vel=(60 + $n * 4)..100 when $n % 4 == 1
  at bar 2 beat $n % 4: master: reverb 0.2 when $n != 1
}
wire changes.out -> comp.in
section intro 4 bars {
  deck A: kick e(3, 8)
  comp(1)
}
section drop 8 bars {
  key fm
  deck A: kick x*(1 + 1) ~ x ~
  repeat 3 {
    comp($i + 4)
  }
}
arrange intro drop
";
    let c = ok(src);
    assert!(c.text.contains("key fm"), "{}", c.text);
    assert!(c.text.contains("deg(1 b3 5' 1,)*2?(1/2)"), "{}", c.text);
    stable(src, 0xf5_23);
    // the section's key is in the nets, and the intro's is the program's
    let intro = c.segments.iter().find(|s| s.name == "intro").unwrap();
    let drop = c.segments.iter().find(|s| s.name == "drop").unwrap();
    assert_ne!(intro.decks[&Deck::B].parts[0].cycles[0].iter().map(|h| h.note).collect::<Vec<_>>(), drop.decks[&Deck::B].parts[0].cycles[0].iter().map(|h| h.note).collect::<Vec<_>>());
}

#[test]
fn a_timed_line_in_a_keyed_section_and_the_lines_a_repeat_makes_there() {
    let src = "\
tempo 120
key am
section verse 4 bars {
  key dm
  deck B: bass deg(1 5) | harmony i VI
  at bar 2 beat 2: master: filter lp 1200
  repeat 2 every 1 bars {
    at bar $i + 1: master: reverb 0.1 * ($i + 1)
  }
}
arrange verse
";
    let c = ok(src);
    assert_eq!(c.controls.len(), 3);
    stable(src, 0xf5_24);
    // `swing` is still refused at the head of a section, `tempo` is allowed there since F3 (docs/btw/ext-tempo.md), and
    // `key` is refused further down it
    let e = errs("key am\nsection verse 4 bars {\n  swing 0.1\n  deck B: bass 1\n}\narrange verse\n");
    assert!(!e.is_empty());
    ok("key am\nsection verse 4 bars {\n  tempo 100\n  deck B: bass 1\n}\narrange verse\n");
    let e = errs("key am\nsection verse 4 bars {\n  deck B: bass 1\n  key dm\n}\narrange verse\n");
    assert!(said(&e, "key"), "{e:?}");
}

// ---- the comment that looks like a sharp numeral

#[test]
fn a_comment_that_starts_with_a_numeral_is_a_comment_and_a_sharp_numeral_says_what_a_comment_is() {
    // `#I like it`: a hash glued to a numeral's letters and then words that are no steps is a comment (fix_syntax.rs, SYN-12:
    // it was a sharp numeral and two words, and an error)
    for src in ["key am\ndeck B: harmony I #I like it\n", "key am\ndeck B: bass 1 #V is the dominant\n", "key am\ndeck B: harmony I #iv chord\n"] {
        let c = ok(src);
        let without = format!("{}\n", &src[..src.find(" #").unwrap()]);
        assert_eq!(c.cid, ok(&without).cid, "{src}");
    }
    // a sharp numeral that is followed by a step, a mark or the end of the line is one: the error that follows says how to write a comment
    let e = errs("key am\ndeck B: harmony I #iv V bogus\n");
    assert!(said(&e, "if `#iv` was meant as a comment"), "{e:?}");
    // the same words with a space after the hash are a comment, and a real sharp numeral is a chord
    ok("key am\ndeck B: harmony I # I like it\n");
    ok("key am\ndeck B: harmony I # iv chord\n");
    let sharp = ok("key am\ndeck B: harmony I #iv V\n");
    assert_eq!(played(&sharp, Deck::B, "harmony").len(), 9);
    // a comment that does not begin with a numeral's letters was never in doubt, and neither is one after a parenthesis
    ok("key am\ndeck B: harmony I V #nice progression\n");
    ok("key am\ndeck B: melody deg(1 #4) #4 is the tritone\n");
    ok("key am\ndeck B: melody deg(1 #4) #I like it\n");
    // an error that has nothing to do with a comment does not talk about one
    let e = errs("key am\ndeck B: harmony I V bogus\n");
    assert!(!said(&e, "a comment starts"), "{e:?}");
}

#[test]
fn a_trailing_comment_after_a_def_call_or_a_let_does_not_become_a_numeral() {
    // `#` after a `)` or at the end of a line of lets, defs and calls is a comment whatever follows it
    let src = "key am # the key\nlet prog = i VI III VII # the changes\ndef comp(p) { # a def\n  deck B: harmony $p voice-lead # the pad\n}\ncomp(prog) #4 bars of it\n";
    let c = ok(src);
    assert_eq!(played(&c, Deck::B, "harmony").len(), 12);
}

// ---- reformatting: every spelling of a program of F5 is one program

#[test]
fn random_reformatting_keeps_the_cid_through_a_def_a_repeat_and_a_wire() {
    for (i, src) in [riff("#4"), riff("5'"), CHANGES.to_string(), "key am\nrepeat 3 {\n  deck B: melody deg(b3 ($i + 1)' 1,) when $i == 1\n}\n".to_string()].iter().enumerate() {
        stable(src, 0xf5_30 + i as u64);
    }
}
