//! F5: scale degrees (`deg( )` with accidentals and octave marks), Roman numerals as steps, a key per section and
//! voice leading (docs/btw/ext-scale.md). Pitches are checked against hand-computed notes: harmony is octave 3
//! (C3 = 48), bass 2 (C2 = 36), melody and arp 4 (C4 = 60).

mod common;

use common::{reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic, Span};
use djlive::lex::{lex, Tok};
use djlive::scale::{invert, motion, parse_roman, voice_lead};
use std::collections::BTreeMap;

// ---- helpers

fn compile(src: &str) -> Compiled {
    let c = check(src);
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

/// (step, note) of a part's cycle, in order.
fn cycle(c: &Compiled, deck: Deck, role: &str, n: usize) -> Vec<(u8, u8)> {
    let ir = &c.decks[&deck];
    let p = ir.parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part"));
    p.cycles[n % p.cycles.len()].iter().map(|h| (h.step, h.note)).collect()
}

/// The notes of the first cycle of a part (steps dropped).
fn notes(src: &str, deck: Deck, role: &str) -> Vec<u8> {
    cycle(&compile(src), deck, role, 0).into_iter().map(|(_, n)| n).collect()
}

/// The notes of one harmony chord in a key (one chord in one bar).
fn chord_in(key: &str, step: &str) -> Vec<u8> {
    notes(&format!("key {key}\ndeck B: harmony {step}\n"), Deck::B, "harmony")
}

fn in_c(step: &str) -> Vec<u8> {
    chord_in("c", step)
}

fn seg<'a>(c: &'a Compiled, name: &str) -> &'a djlive::compile::Segment {
    c.segments.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no segment {name}"))
}

fn seg_notes(c: &Compiled, name: &str, deck: Deck, role: &str) -> Vec<u8> {
    let p = seg(c, name).decks[&deck].parts.iter().find(|p| p.role == role).unwrap();
    p.cycles[0].iter().map(|h| h.note).collect()
}

fn toks(src: &str) -> Vec<Tok> {
    let l = lex(src);
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    l.tokens.into_iter().map(|t| t.tok).filter(|t| *t != Tok::Eof).collect()
}

fn word(s: &str) -> Tok {
    Tok::Word(s.to_string())
}

fn int(v: f64) -> Tok {
    Tok::Num { value: v, int: true }
}

/// The notes of each step of a harmony part's first cycle, a chord a step.
fn chords_of(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<u8>> {
    let mut by_step: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    for (s, n) in cycle(c, deck, role, 0) {
        by_step.entry(s).or_default().push(n);
    }
    by_step.into_values().collect()
}

// ---- the lexer

#[test]
fn a_sharp_degree_is_read_inside_parentheses_and_elsewhere_a_hash_is_a_comment() {
    assert_eq!(toks("deg(1 #4)"), vec![word("deg"), Tok::LParen, int(1.0), word("#4"), Tok::RParen]);
    assert_eq!(toks("deg(#4' b3)"), vec![word("deg"), Tok::LParen, word("#4"), Tok::Apostrophe, word("b3"), Tok::RParen]);
    // outside parentheses `#4` starts a comment, as it always did
    let l = lex("kick x ~ #4 beats");
    assert_eq!(l.comments.len(), 1);
    assert_eq!(l.tokens.iter().filter(|t| matches!(t.tok, Tok::Word(_))).count(), 2, "kick and x");
    let l = lex("deck A: kick x ~ #4");
    assert_eq!(l.comments.len(), 1);
}

#[test]
fn comments_after_a_closing_parenthesis_still_work() {
    for src in ["deg(1 #4) # #5 is not a degree here", "kick e(5,8) #4 beats", "markov(am, 0.7)   #7 notes", "bass deg(1 #4)# a comment", "melody deg(1 b3) #2"] {
        let l = lex(src);
        assert!(l.errors.is_empty(), "{src}: {:?}", l.errors);
        assert_eq!(l.comments.len(), 1, "{src}");
        let close = src.rfind(')').unwrap();
        assert!(l.comments[0].start > close, "{src}: the comment starts after the closing parenthesis");
        assert!(!l.tokens.iter().any(|t| matches!(&t.tok, Tok::Word(w) if w.starts_with('#')) && t.span.start > close), "{src}: {:?}", l.tokens);
    }
    // a whole program: the comment after the call is not read as a degree
    let c = compile("key am\ndeck B: melody deg(1 #4) # #4 is the tritone\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0).len(), 2);
}

#[test]
fn the_comma_is_an_octave_mark_when_it_touches_a_degree_and_a_space_or_a_closing_mark_follows() {
    use Tok::{Comma, OctDown};
    assert_eq!(toks("deg(1,)"), vec![word("deg"), Tok::LParen, int(1.0), OctDown, Tok::RParen]);
    assert_eq!(toks("deg(1, 3)"), vec![word("deg"), Tok::LParen, int(1.0), OctDown, int(3.0), Tok::RParen]);
    // (inside a `[ ]` group a comma is the stack comma, with a space after it too; the mark is the comma before `]`)
    assert_eq!(toks("deg([1, 3])")[2..], [Tok::LBrack, int(1.0), Comma, int(3.0), Tok::RBrack, Tok::RParen]);
    assert_eq!(toks("deg([3 1,])")[2..], [Tok::LBrack, int(3.0), int(1.0), OctDown, Tok::RBrack, Tok::RParen]);
    assert_eq!(toks("deg(b3, #4,)")[2..], [word("b3"), OctDown, word("#4"), OctDown, Tok::RParen]);
    assert_eq!(toks("deg(5',)")[2..], [int(5.0), Tok::Apostrophe, OctDown, Tok::RParen]);
    assert_eq!(toks("deg(1,,)")[2..], [int(1.0), OctDown, OctDown, Tok::RParen]);
    // directly followed by another token it is the stack comma, as in any group
    assert_eq!(toks("deg([1,3,5])")[2..], [Tok::LBrack, int(1.0), Comma, int(3.0), Comma, int(5.0), Tok::RBrack, Tok::RParen]);
    // not glued to the degree: a stack comma whatever follows
    assert_eq!(toks("deg([1 , 3])")[3..6], [int(1.0), Comma, int(3.0)]);
    assert_eq!(toks("deg([~, 1])")[3..6], [Tok::Tilde, Comma, int(1.0)]);
    // outside deg( ) a comma is a comma
    assert_eq!(toks("e(5, 8)"), vec![word("e"), Tok::LParen, int(5.0), Comma, int(8.0), Tok::RParen]);
    assert_eq!(toks("[1, 3]"), vec![Tok::LBrack, int(1.0), Comma, int(3.0), Tok::RBrack]);
    // a repeat, an accent, a slide or the end of an alternation after the mark: it is still the mark
    assert_eq!(toks("deg(1,*2 3,! <5,>)")[2..], [int(1.0), OctDown, Tok::Star, int(2.0), int(3.0), OctDown, Tok::Bang, Tok::Lt, int(5.0), OctDown, Tok::Gt, Tok::RParen]);
    // `deg (` with a space is still deg( )
    assert_eq!(toks("deg (1,)")[2..], [int(1.0), OctDown, Tok::RParen]);
}

#[test]
fn numerals_with_signs_are_words_and_a_sharp_numeral_is_a_word_after_a_step() {
    assert_eq!(toks("ii\u{b0} V7 vii\u{f8}7"), vec![word("ii\u{b0}"), word("V7"), word("vii\u{f8}7")]);
    assert_eq!(toks("harmony I #iv V"), vec![word("harmony"), word("I"), word("#iv"), word("V")]);
    assert_eq!(toks("harmony I #iv/3"), vec![word("harmony"), word("I"), word("#iv"), Tok::Slash, int(3.0)]);
    // a comment that begins with a numeral's letters, or a hash with a space, is still a comment
    for src in ["harmony I V #intro", "harmony I V # iv", "harmony I V #iv.", "#iv at the head of a line"] {
        let l = lex(src);
        assert_eq!(l.comments.len(), 1, "{src}");
        assert!(!l.tokens.iter().any(|t| matches!(&t.tok, Tok::Word(w) if w.starts_with('#'))), "{src}");
    }
}

// ---- degrees

#[test]
fn degrees_with_accidentals_in_a_minor_key() {
    // A minor, melody is octave 4: A4 = 69; the scale A B C D E F G = 69 71 72 74 76 77 79
    assert_eq!(notes("key am\ndeck B: melody deg(1 2 3 4)\n", Deck::B, "melody"), vec![69, 71, 72, 74]);
    // a flat degree is a semitone under the scale's own, a sharp one above
    assert_eq!(notes("key am\ndeck B: melody deg(b3 #4 b7 #7)\n", Deck::B, "melody"), vec![71, 75, 78, 80]);
    // in C major the same signs are the textbook ones
    assert_eq!(notes("key c\ndeck B: melody deg(1 b3 5 b7)\n", Deck::B, "melody"), vec![60, 63, 67, 70]);
    assert_eq!(notes("key c\ndeck B: melody deg(#4 #5)\n", Deck::B, "melody"), vec![66, 68]);
    // in a mode, relative to that mode's own scale (D dorian: D E F G A B C)
    assert_eq!(notes("key d:dorian\ndeck B: melody deg(1 3 b3 6 b6)\n", Deck::B, "melody"), vec![62, 65, 64, 71, 70]);
}

#[test]
fn octave_marks_move_a_degree_by_twelve() {
    assert_eq!(notes("key c\ndeck B: melody deg(5 5' 5'' 1,)\n", Deck::B, "melody"), vec![67, 79, 91, 48]);
    assert_eq!(notes("key c\ndeck B: melody deg(1, 1,, b3' #4,)\n", Deck::B, "melody"), vec![48, 36, 75, 54]);
    // the marks cancel
    assert_eq!(notes("key c\ndeck B: melody deg(5' 5',)\n", Deck::B, "melody"), vec![79, 67]);
    // a degree above the seventh climbs the scale as plain degrees do: 8 is the tonic an octave up
    assert_eq!(notes("key c\ndeck B: melody deg(8 9 b10)\n", Deck::B, "melody"), vec![72, 74, 75]);
}

#[test]
fn a_bare_b3_stays_the_note_b_in_octave_three() {
    assert_eq!(notes("key c\ndeck B: melody b3 c4\n", Deck::B, "melody"), vec![59, 60]);
    // inside deg( ) it is the flat third (either case: the note spellings are case-free)
    assert_eq!(notes("key c\ndeck B: melody deg(b3 B3)\n", Deck::B, "melody"), vec![63, 63]);
    // and `#4` outside the parentheses is a comment, so a program that has one means what it did
    assert_eq!(notes("key c\ndeck B: melody 1 3 #4 5\n", Deck::B, "melody"), vec![60, 64]);
}

#[test]
fn deg_is_one_slot_squeezed_like_a_group() {
    // two slots: the deg group and a degree; the group's notes share its half bar
    let c = compile("key c\ndeck B: melody deg(1 3) 5\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0), vec![(0, 60), (4, 64), (8, 67)]);
    // the same as the group written with brackets
    let d = compile("key c\ndeck B: melody [1 3] 5\n");
    assert_eq!(cycle(&d, Deck::B, "melody", 0), cycle(&c, Deck::B, "melody", 0));
    // a rest, a hit (the part's root) and a repeat inside
    let c = compile("key c\ndeck B: bass deg(1 ~ x 5*2)\n");
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 36), (8, 36), (12, 43), (14, 43)]);
}

#[test]
fn deg_stacks_and_alternates() {
    // a stack: [1,3,5] is a chord (the stack comma is glued to what follows)
    let c = compile("key c\ndeck B: harmony deg([1,3,5])\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 48), (0, 52), (0, 55)]);
    // a stack at the top level of deg( ) too
    let d = compile("key c\ndeck B: harmony deg(1,3,5)\n");
    assert_eq!(cycle(&d, Deck::B, "harmony", 0), cycle(&c, Deck::B, "harmony", 0));
    // an alternation plays one degree a bar
    let c = compile("key c\ndeck B: melody deg(<1 3 5>)\n");
    assert_eq!(c.decks[&Deck::B].period, 3);
    assert_eq!(cycle(&c, Deck::B, "melody", 0), vec![(0, 60)]);
    assert_eq!(cycle(&c, Deck::B, "melody", 1), vec![(0, 64)]);
    assert_eq!(cycle(&c, Deck::B, "melody", 2), vec![(0, 67)]);
    // with spaces after the commas they are octave marks, one degree each (three in a bar: they snap)
    let c = compile("key c\ndeck B: harmony deg(1, 3, 5)\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 36), (5, 40), (11, 55)]);
    // inside a `[ ]` group a comma is the stack comma however it is spaced (SYN-04); an octave-down mark there is the comma
    // before the closing bracket, or before another comma
    let c = compile("key c\ndeck B: harmony deg([1, 3, 5])\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 48), (0, 52), (0, 55)]);
    let c = compile("key c\ndeck B: harmony deg([3 1,])\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0).iter().map(|h| h.1).collect::<Vec<_>>(), vec![52, 36]);
}

#[test]
fn deg_works_as_an_argument_and_in_lets_and_defs() {
    let c = compile("key c\ndeck B: melody transpose(deg(1 3 5 b7), 12)\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0).iter().map(|h| h.1).collect::<Vec<_>>(), vec![72, 76, 79, 82]);
    let c = compile("key c\nlet riff = deg(1 b3 5 b7)\ndeck B: melody riff | bass reverse(riff)\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0).iter().map(|h| h.1).collect::<Vec<_>>(), vec![60, 63, 67, 70]);
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 46), (4, 43), (8, 39), (12, 36)]);
    let c = compile("key c\ndef line(d) {\n  deck $d: melody deg(1 #4, 5')\n}\nline(B)\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0).iter().map(|h| h.1).collect::<Vec<_>>(), vec![60, 54, 79]);
}

#[test]
fn degrees_outside_the_scale_and_other_words_in_deg_are_errors_with_a_place() {
    let src = "key c\ndeck B: melody deg(1 29)\n";
    let e = one_error(src);
    assert!(e.message.contains("outside the scale"), "{}", e.message);
    assert_eq!(e.position(src).0, 2);
    let e = one_error("key c\ndeck B: melody deg(1 0)\n");
    assert!(e.message.contains("counts from 1"), "{}", e.message);
    let src = "key c\ndeck B: melody deg(1 c3)\n";
    let e = one_error(src);
    assert!(e.message.contains("`c3` is not a degree"), "{}", e.message);
    assert_eq!(e.position(src).1, 22, "the column of the word");
    let e = one_error("key c\ndeck B: melody deg()\n");
    assert!(e.message.contains("no degrees"), "{}", e.message);
    let e = one_error("key c\ndeck B: melody deg(1 3\n");
    assert!(e.message.contains("close deg("), "{}", e.message);
    // `deg` with no parenthesis is not a step (it is a word of the language)
    one_error("key c\ndeck B: melody deg 1 3\n");
    // 29 is outside the scale even flattened
    let e = one_error("key c\ndeck B: melody deg(b29)\n");
    assert!(e.message.contains("outside the scale"));
    // a chord written in deg( ) is told where it goes
    let e = one_error("key c\ndeck B: harmony deg(I V)\n");
    assert!(e.message.contains("`I` is not a degree"), "{}", e.message);
}

#[test]
fn deg_prints_canonically_and_every_spelling_is_one_program() {
    let want = compile("key am\ndeck B: melody deg(1 b3 5' 1,)\n");
    assert!(want.text.contains("melody deg(1 b3 5' 1,)"), "{}", want.text);
    for spelling in [
        "key am\ndeck B: melody deg( 1   B3 5'  1, )\n",
        "key am\ndeck B: melody deg(1 b3 5' 1,)  # the tune\n",
        "key AM\ndeck B:   melody   deg(1\tb3\t5'\t1,)\n",
        "key am\ndeck B: melody deg(1 (b3) 5' 1,)\n",
    ] {
        assert_eq!(compile(spelling).cid, want.cid, "{spelling}");
    }
    // marks that cancel are one degree: the canonical text says what plays
    let c = compile("key am\ndeck B: melody deg(5',, 5', 3'')\n");
    assert!(c.text.contains("deg(5, 5 3'')"), "{}", c.text);
    // stacks print glued, with a space where a comma mark meets a stack comma
    let c = compile("key am\ndeck B: harmony deg([1 , 3 , 5] [1, , 3])\n");
    assert!(c.text.contains("deg([1,3,5] [1, ,3])"), "{}", c.text);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    // an accent, a slide and a repeat inside
    let c = compile("key am\ndeck B: melody deg(1! 3^ 5*2 b7'!)\n");
    assert!(c.text.contains("deg(1! 3^ 5*2 b7'!)"), "{}", c.text);
    // a semantic edit is another program
    for edit in ["deg(1 b3 5 1,)", "deg(1 b3 5' 1)", "deg(1 3 5' 1,)", "deg(1 b3 5' 2,)", "deg(1 #3 5' 1,)"] {
        let src = format!("key am\ndeck B: melody {edit}\n");
        assert_ne!(compile(&src).cid, want.cid, "{edit}");
    }
}

#[test]
fn a_deg_in_a_let_and_a_def_keeps_its_commas_in_the_canonical_text() {
    let src = "key am\nlet lowline = deg(1, 3, [5,7])\nlet stk = deg([1,3,5] [1, ,3])\ndef tune(d) {\n  deck $d: melody deg(1, b3, 5' [1,5])\n}\ntune(B)\ndeck A: bass lowline\n";
    let c = compile(src);
    assert!(c.text.contains("let lowline = deg(1, 3, [5,7])"), "{}", c.text);
    assert!(c.text.contains("let stk = deg([1,3,5] [1, ,3])"), "{}", c.text);
    assert!(c.text.contains("deck $d: melody deg(1, b3, 5' [1,5])"), "{}", c.text);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    // the notes are what the commas mean: low is 1, (A1 an octave under the bass's A2... A2 = 45, one down is 33),
    // 3, (C3 = 48, down 36) and a stack of 5 and 7 (E3 and G3) in the third slot
    assert_eq!(cycle(&c, Deck::A, "bass", 0), vec![(0, 33), (5, 36), (11, 52), (11, 55)]);
}

// ---- Roman numerals

#[test]
fn every_quality_is_the_chord_it_names() {
    // C major, harmony is octave 3: C3 = 48
    for (step, want) in [
        ("I", vec![48, 52, 55]),
        ("i", vec![48, 51, 55]),
        ("I7", vec![48, 52, 55, 58]),
        ("i7", vec![48, 51, 55, 58]),
        ("Imaj7", vec![48, 52, 55, 59]),
        ("imaj7", vec![48, 51, 55, 59]),
        ("i\u{b0}", vec![48, 51, 54]),
        ("Idim", vec![48, 51, 54]),
        ("I\u{b0}", vec![48, 51, 54]),
        ("i\u{b0}7", vec![48, 51, 54, 57]),
        ("idim7", vec![48, 51, 54, 57]),
        ("i\u{f8}", vec![48, 51, 54, 58]),
        ("i\u{f8}7", vec![48, 51, 54, 58]),
        ("Iaug", vec![48, 52, 56]),
        ("iaug", vec![48, 52, 56]),
        ("Isus2", vec![48, 50, 55]),
        ("Isus4", vec![48, 53, 55]),
        ("I6", vec![48, 52, 55, 57]),
        ("i6", vec![48, 51, 55, 57]),
        ("I9", vec![48, 52, 55, 58, 62]),
        ("i9", vec![48, 51, 55, 58, 62]),
    ] {
        assert_eq!(in_c(step), want, "{step}");
    }
}

#[test]
fn the_seven_degrees_are_the_chords_of_the_scale() {
    // C major: I ii iii IV V vi vii°
    assert_eq!(in_c("I"), vec![48, 52, 55]);
    assert_eq!(in_c("ii"), vec![50, 53, 57]);
    assert_eq!(in_c("iii"), vec![52, 55, 59]);
    assert_eq!(in_c("IV"), vec![53, 57, 60]);
    assert_eq!(in_c("V"), vec![55, 59, 62]);
    assert_eq!(in_c("vi"), vec![57, 60, 64]);
    assert_eq!(in_c("vii\u{b0}"), vec![59, 62, 65]);
    // A minor (the natural minor scale): i ii° III iv v VI VII; A3 = 57
    let am = |s: &str| chord_in("am", s);
    assert_eq!(am("i"), vec![57, 60, 64]);
    assert_eq!(am("ii\u{b0}"), vec![59, 62, 65]);
    assert_eq!(am("III"), vec![60, 64, 67]);
    assert_eq!(am("iv"), vec![62, 65, 69]);
    assert_eq!(am("v"), vec![64, 67, 71]);
    assert_eq!(am("VI"), vec![65, 69, 72]);
    assert_eq!(am("VII"), vec![67, 71, 74]);
    // the case is the chord, not the scale: a major V in A minor is E G# B
    assert_eq!(am("V"), vec![64, 68, 71]);
    assert_eq!(am("V7"), vec![64, 68, 71, 74]);
    // numerals follow the mode of the key
    let dorian = |s: &str| chord_in("d:dorian", s);
    assert_eq!(dorian("i"), vec![50, 53, 57]);
    assert_eq!(dorian("IV"), vec![55, 59, 62]);
    assert_eq!(dorian("bVII"), vec![59, 63, 66], "the dorian seventh is C: the flat one is a semitone under it, B, and the case says major");
    assert_eq!(dorian("VII"), vec![60, 64, 67]);
    // a transposed key moves every chord
    assert_eq!(chord_in("d", "I"), vec![50, 54, 57]);
    assert_eq!(chord_in("f#m", "i"), vec![54, 57, 61]);
}

#[test]
fn accidentals_lower_and_raise_the_root_a_semitone() {
    assert_eq!(in_c("bVII"), vec![58, 62, 65]);
    assert_eq!(in_c("bIII"), vec![51, 55, 58]);
    assert_eq!(in_c("bII"), vec![49, 53, 56]);
    assert_eq!(in_c("#iv"), vec![54, 57, 61]);
    assert_eq!(in_c("#iv\u{b0}"), vec![54, 57, 60]);
    assert_eq!(in_c("bVI7"), vec![56, 60, 63, 66]);
    assert_eq!(chord_in("am", "bVII"), vec![66, 70, 73], "a flat seventh of the minor scale is under its seventh");
    assert_eq!(chord_in("am", "#iv\u{b0}"), vec![63, 66, 69]);
}

#[test]
fn inversions_put_the_named_tone_in_the_bass() {
    assert_eq!(in_c("I/3"), vec![52, 55, 60]);
    assert_eq!(in_c("I/5"), vec![55, 60, 64]);
    assert_eq!(in_c("i/3"), vec![51, 55, 60]);
    assert_eq!(in_c("I7/3"), vec![52, 55, 58, 60]);
    assert_eq!(in_c("I7/5"), vec![55, 58, 60, 64]);
    assert_eq!(in_c("I7/7"), vec![58, 60, 64, 67]);
    assert_eq!(in_c("V7/7"), vec![65, 67, 71, 74]);
    assert_eq!(in_c("Imaj7/7"), vec![59, 60, 64, 67]);
    assert_eq!(in_c("Isus4/5"), vec![55, 60, 65]);
    assert_eq!(in_c("Isus2/5"), vec![55, 60, 62]);
    assert_eq!(in_c("I6/5"), vec![55, 57, 60, 64]);
    assert_eq!(in_c("I9/7"), vec![58, 60, 62, 64, 67]);
    assert_eq!(in_c("vii\u{f8}/5"), vec![65, 69, 71, 74]);
    assert_eq!(in_c("bVII/3"), vec![62, 65, 70]);
    // the pitch set is the chord's in every inversion
    for inv in ["", "/3", "/5"] {
        let mut set: Vec<u8> = in_c(&format!("V{inv}")).iter().map(|n| n % 12).collect();
        set.sort_unstable();
        assert_eq!(set, vec![2, 7, 11], "V{inv}");
    }
    // the lowest note is the named tone
    assert_eq!(in_c("V7/5").iter().min().map(|n| n % 12), Some(2));
    assert_eq!(in_c("V7/7").iter().min().map(|n| n % 12), Some(5));
}

#[test]
fn a_part_plays_a_numeral_by_its_role() {
    // A minor: bass is octave 2 (A2 = 45), melody and arp 4 (A4 = 69), harmony 3
    // single-note roles play the chord's bass note: the root, or with an inversion the tone the slash names
    let c = compile("key am\ndeck B: bass i VI III V7/7\n");
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 45), (4, 53), (8, 48), (12, 62)]);
    let c = compile("key am\ndeck B: melody i VI/3 III/5 V\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0), vec![(0, 69), (4, 81), (8, 79), (12, 76)]);
    // a chord in harmony is its notes together
    let c = compile("key am\ndeck B: harmony i VI\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 57), (0, 60), (0, 64), (8, 65), (8, 69), (8, 72)]);
    // an arp plays the chord one note at a time, four steps over the numeral's share of the bar
    let c = compile("key am\ndeck B: arp i\n");
    assert_eq!(cycle(&c, Deck::B, "arp", 0), vec![(0, 69), (4, 72), (8, 76), (12, 69)]);
    let c = compile("key am\ndeck B: arp i VI\n");
    assert_eq!(cycle(&c, Deck::B, "arp", 0), vec![(0, 69), (2, 72), (4, 76), (6, 69), (8, 77), (10, 81), (12, 84), (14, 77)]);
    // a seventh chord is each of its four tones once, a ninth chord leaves out its fifth
    let c = compile("key c\ndeck B: arp I7\n");
    assert_eq!(cycle(&c, Deck::B, "arp", 0), vec![(0, 60), (4, 64), (8, 67), (12, 70)]);
    let c = compile("key c\ndeck B: arp I9\n");
    assert_eq!(cycle(&c, Deck::B, "arp", 0), vec![(0, 60), (4, 64), (8, 70), (12, 74)]);
    // oct= moves them
    let c = compile("key c\ndeck B: harmony I oct=4\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 60), (0, 64), (0, 67)]);
}

#[test]
fn a_numeral_is_a_step_like_any_other() {
    // in an alternation, with a repeat, a rest and a group
    let c = compile("key c\ndeck B: harmony <I V> ~\n");
    assert_eq!(c.decks[&Deck::B].period, 2);
    assert_eq!(cycle(&c, Deck::B, "harmony", 0), vec![(0, 48), (0, 52), (0, 55)]);
    assert_eq!(cycle(&c, Deck::B, "harmony", 1), vec![(0, 55), (0, 59), (0, 62)]);
    let c = compile("key c\ndeck B: bass I*2 [V vi] ~ ~\n");
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 36), (2, 36), (4, 43), (6, 45)]);
    // a stack of two numerals is both chords at once
    let c = compile("key c\ndeck B: harmony [I, V]\n");
    assert_eq!(cycle(&c, Deck::B, "harmony", 0).len(), 5, "the G of both chords is one note");
    // through the transforms
    let c = compile("key c\ndeck B: bass transpose([I V], 12) reverse([I IV])\n");
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 48), (4, 55), (8, 41), (12, 36)]);
    // with the accent and slide marks
    let c = compile("key c\ndeck B: bass I! V^\n");
    let hits = &c.decks[&Deck::B].parts[0].cycles[0];
    assert!(hits[0].accent && hits[1].slide);
}

#[test]
fn numerals_in_a_drum_part_or_misspelt_are_errors_that_say_what_to_write() {
    let src = "deck B: kick I\n";
    let e = one_error(src);
    assert!(e.message.contains("drum part has no pitch"), "{}", e.message);
    assert_eq!(e.position(src), (1, 14, 15));
    let src = "deck B: harmony I Imaj\n";
    let e = one_error(src);
    assert!(e.message.contains("not a chord quality after the numeral `I`"), "{}", e.message);
    assert!(e.help.as_deref().unwrap().contains("did you mean `Imaj7`"), "{:?}", e.help);
    assert_eq!(e.position(src).1, 19);
    let e = one_error("deck B: harmony Vsus3\n");
    assert!(e.help.as_deref().unwrap().contains("`Vsus2`") || e.help.as_deref().unwrap().contains("`Vsus4`"), "{:?}", e.help);
    let e = one_error("deck B: harmony Iaugg\n");
    assert!(e.help.as_deref().unwrap().contains("`Iaug`"), "{:?}", e.help);
    let e = one_error("deck B: harmony bVIIm7\n");
    assert!(e.message.contains("not a chord quality"), "{}", e.message);
    // upper and lower case in one numeral
    let e = one_error("deck B: harmony Iv\n");
    assert!(e.message.contains("mixes upper and lower case"), "{}", e.message);
    assert!(e.help.as_deref().unwrap().contains("`IV` or `iv`"), "{:?}", e.help);
    // an eighth degree
    let e = one_error("deck B: harmony viii\n");
    assert!(e.message.contains("seven degrees"), "{}", e.message);
    // an inversion tone the chord lacks
    let src = "deck B: harmony I/7\n";
    let e = one_error(src);
    assert!(e.message.contains("`I` has no seventh to put in the bass"), "{}", e.message);
    assert!(e.help.as_deref().unwrap().contains("`I7/7`"), "{:?}", e.help);
    assert_eq!((e.position(src).1, e.position(src).2), (17, 20));
    let e = one_error("deck B: harmony Isus4/3\n");
    assert!(e.message.contains("`Isus4` has no third"), "{}", e.message);
    let e = one_error("deck B: harmony I6/7\n");
    assert!(e.message.contains("no seventh"), "{}", e.message);
    let e = one_error("deck B: harmony I/4\n");
    assert!(e.message.contains("not an inversion"), "{}", e.message);
    // a word that only begins with a numeral's letters is not claimed: the old message stands
    let e = one_error("deck B: harmony inst\n");
    assert!(e.message.contains("`inst` is not a step"), "{}", e.message);
    let e = &errors("deck A: kick x ~ vel 90\n")[0];
    assert!(e.help.as_deref().unwrap_or("").contains("vel=90"), "{e:?}");
}

#[test]
fn numerals_name_a_let_a_def_or_a_section_and_the_let_wins_in_a_pattern() {
    // a numeral is a step where no let has its name; `let v = ...` is what `v` means in the patterns after it (old programs
    // call their lets `i`, `v`, `ii`: fix_syntax.rs has the rest)
    for name in ["I", "i", "ii", "III", "iv", "V", "vi", "VII", "vii", "bVII", "V7", "ii\u{b0}", "Imaj7"] {
        let n = notes(&format!("let {name} = c2 e2\ndeck B: bass {name}\n"), Deck::B, "bass");
        assert_eq!(n, vec![36, 40], "{name}");
        compile(&format!("def {name}(a) {{\n  master: delay 0.1\n}}\n{name}(1)\n"));
        compile(&format!("section {name} 4 bars {{\n  deck A: kick x\n}}\narrange {name}\n"));
        // and the let of a single value, put in place of the name
        let n = notes(&format!("let {name} = 3\ndeck B: bass {name}\n"), Deck::B, "bass");
        assert_eq!(n.len(), 1, "{name}");
    }
    // a numeral that is spelled another way in the canonical text still cannot be a name (`idim` is read back as `i\u{b0}`)
    let e = &errors("let idim = c2 ~\n")[0];
    assert!(e.message.contains("reads as the Roman numeral") && e.help.as_deref().is_some_and(|h| h.contains("rename it")), "{}", e.message);
    // words that merely begin like a numeral are still names
    let c = compile("let ivy = c2 ~\nlet vamp = c2 e2\nlet intro = 1 3\ndeck B: bass ivy vamp intro\n");
    assert_eq!(c.decks[&Deck::B].parts.len(), 1);
}

#[test]
fn a_numeral_prints_with_the_canonical_case_of_its_quality() {
    let c = compile("key c\ndeck B: harmony IMAJ7 VII\u{b0} viiDIM7 Vsus4 isus4 iaug V7/7 bvii #iv IV/3 ii\u{f8}7\n");
    assert!(c.text.contains("harmony Imaj7 vii\u{b0} vii\u{b0}7 Vsus4 Isus4 Iaug V7/7 bvii #iv IV/3 ii\u{f8}"), "{}", c.text);
    // the same in a let, in a def's body and in a call's arguments
    let c = compile("key c\nlet prog = IMAJ7 VII\u{b0} V7/7\ndef p(d) {\n  deck $d: bass ISUS4 vii\u{f8}7\n}\np(B)\ndeck A: bass transpose(IMAJ7, 2)\n");
    assert!(c.text.contains("let prog = Imaj7 vii\u{b0} V7/7"), "{}", c.text);
    assert!(c.text.contains("deck $d: bass Isus4 vii\u{f8}"), "{}", c.text);
    assert!(c.text.contains("transpose(Imaj7, 2)"), "{}", c.text);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    // a semantic edit (the case, the quality, the inversion, the accidental) is another program
    let base = compile("key c\ndeck B: harmony I V7/7 bVII\n").cid;
    for edit in ["i V7/7 bVII", "I V7 bVII", "I V7/5 bVII", "I V7/7 VII", "I V7/7 bvii", "I V7/7 #VII", "I V bVII"] {
        assert_ne!(compile(&format!("key c\ndeck B: harmony {edit}\n")).cid, base, "{edit}");
    }
    // a diminished chord is lower case whatever was typed: one program
    assert_eq!(compile("key c\ndeck B: harmony VII\u{b0}\n").cid, compile("key c\ndeck B: harmony vii\u{b0}\n").cid);
}

#[test]
fn a_numeral_in_a_pentatonic_or_blues_key_counts_in_the_scale_it_comes_from() {
    // A minor pentatonic and the blues take the natural minor's degrees, C pentatonic the major's
    assert_eq!(chord_in("a:minpent", "VII"), chord_in("am", "VII"));
    assert_eq!(chord_in("a:blues", "VI"), chord_in("am", "VI"));
    assert_eq!(chord_in("c:pent", "IV"), chord_in("c", "IV"));
}

#[test]
fn the_numerals_of_a_part_light_their_tokens_through_let_def_and_repeat() {
    let src = "key c\nlet prog = I V\ndef comp(d) {\n  deck $d: harmony prog\n}\ncomp(B)\n";
    let c = compile(src);
    let p = c.decks[&Deck::B].parts.iter().find(|p| p.role == "harmony").unwrap();
    let first = &p.cycles[0][0];
    let chain: Vec<&str> = p.spans[first.sid as usize].iter().map(|s| s.text(src)).collect();
    assert_eq!(chain[0], "I", "the numeral's own token in the let: {chain:?}");
    assert!(chain.contains(&"prog"), "the use: {chain:?}");
    assert_eq!(*chain.last().unwrap(), "comp(B)", "and the call: {chain:?}");
    // every note of the chord lights the same token; the second chord lights V
    let of_first: Vec<_> = p.cycles[0].iter().filter(|h| h.step == 0).map(|h| h.sid).collect();
    assert_eq!(of_first.len(), 3);
    assert!(of_first.iter().all(|s| *s == first.sid));
    let second = p.cycles[0].iter().find(|h| h.step == 8).unwrap();
    assert_eq!(p.spans[second.sid as usize][0].text(src), "V");
    // an inversion lights the numeral and its slash together
    let src = "key c\ndeck B: harmony I V7/7\n";
    let c = compile(src);
    let p = &c.decks[&Deck::B].parts[0];
    let h = p.cycles[0].iter().find(|h| h.step == 8).unwrap();
    assert_eq!(p.spans[h.sid as usize][0].text(src), "V7/7");
    // a degree in deg( ) lights its own token, and the group after it
    let src = "key c\ndeck B: melody deg(1 b3 5')\n";
    let c = compile(src);
    let p = &c.decks[&Deck::B].parts[0];
    let texts: Vec<&str> = p.cycles[0].iter().map(|h| p.spans[h.sid as usize][0].text(src)).collect();
    assert_eq!(texts, vec!["1", "b3", "5'"]);
    assert!(p.spans[p.cycles[0][0].sid as usize].iter().any(|s| s.text(src) == "deg(1 b3 5')"));
    // through a repeat: the numeral's token and the repeat that expanded it
    let src = "key c\nrepeat 1 {\n  deck B: bass I\n}\n";
    let c = compile(src);
    let p = &c.decks[&Deck::B].parts[0];
    let h = &p.cycles[0][0];
    assert_eq!(p.spans[h.sid as usize][0].text(src), "I");
    assert!(p.via.iter().any(|s| s.text(src).starts_with("repeat 1")));
}

// ---- the runtime: a fake host and a fake clock

use djengine::json::JsonValue;
use djlive::runtime::{Grid, Live, LiveHost};

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    loads: Vec<(String, String, i64)>,
}
fn ix(d: &str) -> usize {
    usize::from(d == "B")
}
impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else { "B" };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }
    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), djengine::cid::share_cid(env), land));
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }
    fn set_sound(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn call(&mut self, _: &str, _: &JsonValue) -> Result<JsonValue, String> {
        Ok(JsonValue::Null)
    }
}

impl Fake {
    fn at(beat: f64) -> Fake {
        Fake { beat, playing: [true, false], ..Fake::default() }
    }

    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }
}

// ---- a key per section

const TWO_KEYS: &str = "\
key am
deck A: kick x ~ x ~
deck B: bass 1 ~ 5 ~ | melody deg(1 3 5 b7) | harmony i VI
section verse 4 bars {
  deck B: arp i
}
section lift 4 bars {
  key fm
  deck B: arp i
}
arrange verse lift verse
";

#[test]
fn a_song_in_two_keys_degrees_and_numerals_follow_the_section() {
    let c = compile(TWO_KEYS);
    // the program's key is still am; sections say theirs
    assert_eq!(c.key.unwrap().text(), "am");
    assert_eq!(c.section_key("verse").unwrap().text(), "am");
    assert_eq!(c.section_key("lift").unwrap().text(), "fm");
    assert_eq!(c.section_key("nowhere"), None);
    // global parts (the song's) are read again in each section's key
    assert_eq!(seg_notes(&c, "verse", Deck::B, "bass"), vec![45, 52]);
    assert_eq!(seg_notes(&c, "lift", Deck::B, "bass"), vec![41, 48]);
    assert_eq!(seg_notes(&c, "verse", Deck::B, "melody"), vec![69, 72, 76, 78]);
    assert_eq!(seg_notes(&c, "lift", Deck::B, "melody"), vec![65, 68, 72, 74]);
    assert_eq!(seg_notes(&c, "verse", Deck::B, "harmony"), vec![57, 60, 64, 65, 69, 72]);
    assert_eq!(seg_notes(&c, "lift", Deck::B, "harmony"), vec![53, 56, 60, 61, 65, 68]);
    // a section's own parts too
    assert_eq!(seg_notes(&c, "verse", Deck::B, "arp"), vec![69, 72, 76, 69]);
    assert_eq!(seg_notes(&c, "lift", Deck::B, "arp"), vec![65, 68, 72, 65]);
    // the net of each section carries the key it is in where the section re-keyed it
    assert_eq!(seg(&c, "verse").decks[&Deck::B].key.unwrap().text(), "am");
    assert_eq!(seg(&c, "lift").decks[&Deck::B].key.unwrap().text(), "fm");
    assert_eq!(c.segments.len(), 3);
    assert_eq!(c.key_in(&c.segments[2]).text(), "am", "the second playing of a section is in its key again");
    // the net CIDs differ between the keys and are the same for the same section played twice
    let cids = c.net_cids();
    assert_ne!(cids["verse@0:B"], cids["lift@4:B"]);
    assert_eq!(cids["verse@0:B"], cids["verse@8:B"]);
    // a deck of drums does not care: its net is the same in every key, so it is not reloaded
    assert_eq!(cids["verse@0:A"], cids["lift@4:A"]);
}

#[test]
fn the_markov_default_is_the_sections_key() {
    let src = "key am\nsection first 2 bars {\n  deck B: bass markov(0.9, 3)\n}\nsection second 2 bars {\n  key d:dorian\n  deck B: bass markov(0.9, 3)\n}\narrange first second\n";
    let c = compile(src);
    let in_scale = |name: &str, key: &str| {
        let k = djlive::music::Key::parse(key).unwrap();
        let ok: Vec<i32> = k.intervals().iter().map(|i| *i as i32 % 12).collect();
        seg(&c, name).decks[&Deck::B].parts[0].cycles.iter().flatten().all(|h| ok.contains(&(i32::from(h.note) - i32::from(k.tonic)).rem_euclid(12)))
    };
    assert!(in_scale("first", "am"));
    assert!(in_scale("second", "d:dorian"));
    // the same density and seed over another key is another net
    assert_ne!(seg_notes(&c, "first", Deck::B, "bass"), seg_notes(&c, "second", Deck::B, "bass"));
}

#[test]
fn a_hit_in_a_tonal_part_plays_the_sections_root() {
    let c = compile("key am\nsection first 1 bars {\n  deck B: bass x\n}\nsection second 1 bars {\n  key fm\n  deck B: bass x\n}\narrange first second\n");
    assert_eq!(seg_notes(&c, "first", Deck::B, "bass"), vec![45]);
    assert_eq!(seg_notes(&c, "second", Deck::B, "bass"), vec![41]);
}

#[test]
fn a_sections_key_is_its_programs_when_it_sets_none_and_c_major_when_neither() {
    let c = compile("section x1 2 bars {\n  deck A: bass 1 5\n}\nsection x2 2 bars {\n  key d\n  deck A: bass 1 5\n}\narrange x1 x2\n");
    assert_eq!(c.key, None);
    assert_eq!(c.section_key("x1").unwrap().text(), "c");
    assert_eq!(c.section_key("x2").unwrap().text(), "d");
    assert_eq!(seg_notes(&c, "x1", Deck::A, "bass"), vec![36, 43]);
    assert_eq!(seg_notes(&c, "x2", Deck::A, "bass"), vec![38, 45]);
    // with no key anywhere a net says none, with a section key it says that key
    assert_eq!(seg(&c, "x1").decks[&Deck::A].key, None);
    assert_eq!(seg(&c, "x2").decks[&Deck::A].key.unwrap().text(), "d");
}

#[test]
fn the_key_lines_at_the_head_of_a_section_are_read_and_the_last_wins() {
    let c = compile("key c\nsection s1 2 bars {\n  key d\n  key e\n  deck A: bass 1\n}\narrange s1\n");
    assert_eq!(c.section_key("s1").unwrap().text(), "e");
    assert!(c.warnings.iter().any(|w| w.message.contains("key is set twice in this section")), "{:?}", c.warnings);
    // a key line elsewhere is an error that says where it may be
    let src = "key c\nsection s1 2 bars {\n  deck A: bass 1\n  key d\n}\narrange s1\n";
    let e = one_error(src);
    assert!(e.message.contains("head of a `section`"), "{}", e.message);
    assert_eq!(e.position(src).0, 4);
    // not in a def or a repeat, and not with swing (`tempo` may stand in a section since F3, docs/btw/ext-tempo.md)
    let e = &errors("section s1 2 bars {\n  repeat 2 {\n    key d\n  }\n}\narrange s1\n")[0];
    assert!(e.message.contains("head of a `section`"), "{}", e.message);
    let src = "section s1 2 bars {\n  swing 0.1\n  deck A: bass 1\n}\narrange s1\n";
    let e = one_error(src);
    assert!(e.message.contains("`swing` belongs at the top"), "{}", e.message);
    // a key that is not a key
    let src = "section s1 2 bars {\n  key zz\n}\narrange s1\n";
    let e = one_error(src);
    assert!(e.message.contains("`zz` is not a key"), "{}", e.message);
    assert_eq!(e.position(src).0, 2);
    // once, however many times the section plays
    let e = errors("section s1 2 bars {\n  key zz\n}\narrange s1 s1 s1\n");
    assert_eq!(e.len(), 1, "{e:?}");
}

#[test]
fn a_key_per_section_is_in_the_canonical_text_and_the_cid() {
    let c = compile(TWO_KEYS);
    assert!(c.text.contains("section lift 4 bars {\n  key fm\n  deck B: arp i\n}"), "{}", c.text);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    // the same song in another key is another program; a key line spelt another way is not
    assert_ne!(compile(&TWO_KEYS.replace("key fm", "key gm")).cid, c.cid);
    assert_ne!(compile(&TWO_KEYS.replace("  key fm\n", "")).cid, c.cid);
    assert_eq!(compile(&TWO_KEYS.replace("key fm", "key   FM  # up a fourth-ish")).cid, c.cid);
    assert_eq!(compile(&TWO_KEYS.replace("key fm", "key f:minor")).cid, compile(&TWO_KEYS.replace("key fm", "key F:Minor")).cid);
}

#[test]
fn each_sections_nets_land_with_its_own_notes_on_a_fake_clock() {
    let mut h = Fake::at(8.0);
    let mut live = Live::new();
    let src = "key am\ndeck A: harmony i voice-lead\nsection verse 2 bars {\n  deck B: kick x ~ x ~\n}\nsection lift 2 bars {\n  key fm\n  deck B: kick x ~ x ~\n}\narrange verse lift\n";
    assert!(live.eval(src).ok);
    h.run(&mut live, 16.0);
    let want = compile(src).net_cids();
    let a: Vec<&(String, String, i64)> = h.loads.iter().filter(|l| l.0 == "A").collect();
    assert!(a.iter().any(|l| l.1 == want["verse@0:A"]), "the verse's net for A loaded: {a:?}");
    assert!(a.iter().any(|l| l.1 == want["lift@2:A"]), "the lift's net for A loaded, in its key: {a:?}");
    assert_ne!(want["verse@0:A"], want["lift@2:A"]);
    let verse = a.iter().find(|l| l.1 == want["verse@0:A"]).unwrap();
    let lift = a.iter().find(|l| l.1 == want["lift@2:A"]).unwrap();
    assert_eq!(lift.2 - verse.2, 2, "the lift lands two bars after the verse, on its bar line");
}

#[test]
fn numerals_and_degrees_fire_their_own_tokens_on_the_clock() {
    let mut h = Fake::at(8.0);
    let mut live = Live::new();
    let src = "key c\nlet prog = I V7/7\ndeck A: harmony prog voice-lead | melody deg(1 b3 5') | bass bVII ~ ~ ~\n";
    assert!(live.eval(src).ok);
    let mut seen: Vec<(String, String)> = Vec::new();
    for _ in 0..(4 * 4 * 4) {
        h.run(&mut live, 0.25);
        for f in live.fires() {
            let key = (f.role.clone(), f.spans[0].text(src).to_string());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    }
    let of = |role: &str| -> Vec<&str> { seen.iter().filter(|s| s.0 == role).map(|s| s.1.as_str()).collect() };
    let mut harmony = of("harmony");
    harmony.sort_unstable();
    assert_eq!(harmony, vec!["I", "V7/7"], "{seen:?}");
    let mut melody = of("melody");
    melody.sort_unstable();
    assert_eq!(melody, vec!["1", "5'", "b3"], "{seen:?}");
    assert_eq!(of("bass"), vec!["bVII"], "{seen:?}");
    // the let and the line they came from are in the spans too
    let f = live.fires().into_iter().find(|f| f.role == "harmony").unwrap();
    let chain: Vec<&str> = f.spans.iter().map(|s| s.text(src)).collect();
    assert!(chain.contains(&"prog"), "{chain:?}");
}

// ---- voice leading

fn lead_chords(key: &str, steps: &str, params: &str) -> Vec<Vec<u8>> {
    chords_of(&compile(&format!("key {key}\ndeck B: harmony {steps} {params}\n")), Deck::B, "harmony")
}

#[test]
fn voice_leading_i_vi_iii_vii_in_a_minor_by_hand() {
    // A3 C4 E4 / A3 C4 F4 (the A and C stay, E moves a semitone) / G3 C4 E4 (A to G, F to E: C over G) / G3 B3 D4
    assert_eq!(lead_chords("am", "i VI III VII", "voice-lead"), vec![vec![57, 60, 64], vec![57, 60, 65], vec![55, 60, 64], vec![55, 59, 62]]);
    // without it the chords are the root positions
    assert_eq!(lead_chords("am", "i VI III VII", ""), vec![vec![57, 60, 64], vec![65, 69, 72], vec![60, 64, 67], vec![67, 71, 74]]);
}

#[test]
fn voice_leading_i_v_vi_iv_in_c_by_hand() {
    // I C3 E3 G3; V B2 D3 G3 (G stays, E to D, C to B); vi C3 E3 A3; IV C3 F3 A3
    assert_eq!(lead_chords("c", "I V vi IV", "voice-lead"), vec![vec![48, 52, 55], vec![47, 50, 55], vec![48, 52, 57], vec![48, 53, 57]]);
}

/// The least motion any voicing of `base` (every inversion, three octaves either way) makes from `prev`.
fn best_motion(prev: &[i32], base: &[i32], bass: Option<usize>) -> i64 {
    let mut best = i64::MAX;
    for inv in 0..base.len() {
        if bass.is_some_and(|b| b != inv) {
            continue;
        }
        for oct in -3..=3 {
            let cand: Vec<i32> = invert(base, inv).iter().map(|n| n + 12 * oct).collect();
            if cand.iter().all(|n| (0..=127).contains(n)) {
                best = best.min(motion(prev, &cand));
            }
        }
    }
    best
}

#[test]
fn voice_leading_with_sevenths_by_hand() {
    // ii7 D3 F3 A3 C4; V7 as D3 F3 G3 B3 (the D and F stay, A to G, C to B: motion 3); Imaj7 as C3 E3 G3 B3 (motion 3)
    let got = lead_chords("c", "ii7 V7 Imaj7", "voice-lead");
    assert_eq!(got, vec![vec![50, 53, 57, 60], vec![50, 53, 55, 59], vec![48, 52, 55, 59]]);
    // each step moves the voices as little as any voicing of the chord could (the test's own search)
    let mut prev: Option<Vec<i32>> = None;
    for (chord, name) in got.iter().zip(["ii7", "V7", "Imaj7"]) {
        let base: Vec<i32> = parse_roman(name).unwrap().notes(djlive::music::Key::C_MAJOR, 3);
        let now: Vec<i32> = chord.iter().map(|n| i32::from(*n)).collect();
        if let Some(p) = &prev {
            assert_eq!(motion(p, &now), best_motion(p, &base, None), "{name}");
        }
        prev = Some(now);
    }
}

#[test]
fn ties_go_to_the_lower_inversion_then_the_lower_octave() {
    // the test's own oracle: the first candidate, by (inversion, octave) in rising order, with the least motion
    let oracle = |prev: &[i32], base: &[i32], bass: Option<usize>| -> Vec<i32> {
        let mut best: Option<(i64, Vec<i32>)> = None;
        for inv in 0..base.len() {
            if bass.is_some_and(|b| b != inv) {
                continue;
            }
            for oct in -3..=3 {
                let cand: Vec<i32> = invert(base, inv).iter().map(|n| n + 12 * oct).collect();
                if cand.iter().any(|n| !(0..=127).contains(n)) {
                    continue;
                }
                let m = motion(prev, &cand);
                if best.as_ref().is_none_or(|(b, _)| m < *b) {
                    best = Some((m, cand));
                }
            }
        }
        best.unwrap().1
    };
    let mut rng = Rng(0x00c0_ffee);
    let mut ties = 0;
    for _ in 0..400 {
        let key = djlive::music::Key::parse(["c", "am", "d:dorian", "f#m", "bb"][rng.below(5) as usize]).unwrap();
        let name = ["I", "ii", "V7", "vii\u{b0}", "bVI", "IVmaj7", "Iaug", "Isus4", "i\u{b0}7", "vi9"][rng.below(10) as usize];
        let base = parse_roman(name).unwrap().notes(key, 3);
        let prev_name = ["I", "V", "vi7", "bII", "iv", "Iaug", "i\u{b0}7"][rng.below(7) as usize];
        let prev: Vec<i32> = parse_roman(prev_name).unwrap().notes(key, 2 + rng.below(3) as i32).iter().map(|n| n + 12 * (rng.below(3) as i32 - 1)).collect();
        let bass = (rng.below(4) == 0).then(|| (rng.below(base.len() as u64)) as usize);
        let got = voice_lead(Some(&prev), &base, bass);
        assert_eq!(got, oracle(&prev, &base, bass), "{prev:?} to {name} in {} (bass {bass:?})", key.text());
        // how many candidates tie for the least motion: the rule is exercised
        let m = motion(&prev, &got);
        let mut n = 0;
        for inv in 0..base.len() {
            for oct in -3..=3 {
                let cand: Vec<i32> = invert(&base, inv).iter().map(|x| x + 12 * oct).collect();
                if bass.is_none_or(|b| b == inv) && cand.iter().all(|n| (0..=127).contains(n)) && motion(&prev, &cand) == m {
                    n += 1;
                }
            }
        }
        ties += usize::from(n > 1);
    }
    assert!(ties > 0, "the random cases include ties: {ties}");
    // a hand-made tie: a tritone away, the same motion up and down: the lower octave
    assert_eq!(voice_lead(Some(&[60]), &[66], None), vec![54], "F#3 and F#4 are both six away from C4; the lower wins");
    // an augmented triad is its own inversion: the least motion from C E G# to the same chord is none, where it was
    assert_eq!(voice_lead(Some(&[48, 52, 56]), &[48, 52, 56], None), vec![48, 52, 56]);
    // the same chord from a rotated voicing is that voicing (no inversion is lower than the one that stays put)
    assert_eq!(voice_lead(Some(&[52, 56, 60]), &[48, 52, 56], None), vec![52, 56, 60]);
}

#[test]
fn the_first_chord_is_the_root_position_in_the_parts_octave_and_a_written_inversion_is_kept() {
    assert_eq!(lead_chords("c", "I", "voice-lead"), vec![vec![48, 52, 55]]);
    assert_eq!(lead_chords("c", "I", "voice-lead oct=4"), vec![vec![60, 64, 67]]);
    // a written inversion fixes the bass: only its octave is chosen
    assert_eq!(lead_chords("c", "I V/5", "voice-lead"), vec![vec![48, 52, 55], vec![50, 55, 59]]);
    assert_eq!(lead_chords("c", "I/3 V", "voice-lead"), vec![vec![52, 55, 60], vec![50, 55, 59]]);
    // the bass of a voice-led V/3 is still B
    let got = lead_chords("c", "I V/3 I", "voice-lead");
    assert_eq!(got[1].iter().min().map(|n| n % 12), Some(11));
    assert_eq!(got[2], vec![48, 52, 55]);
}

#[test]
fn voice_lead_switches_on_off_and_prints_one_way() {
    let on = compile("key c\ndeck B: harmony I V voice-lead\n");
    assert!(on.text.contains("harmony I V voice-lead=on"), "{}", on.text);
    assert_eq!(compile("key c\ndeck B: harmony I V voice-lead=on\n").cid, on.cid);
    assert_eq!(compile("key c\ndeck B: harmony I V voice-lead  =  on\n").cid, on.cid);
    // among other params, sorted by name
    let c = compile("key c\ndeck B: harmony I V voice-lead oct=4 vel=80\n");
    assert!(c.text.contains("harmony I V oct=4 vel=80 voice-lead=on"), "{}", c.text);
    assert_eq!(compile("key c\ndeck B: harmony I V vel=80 voice-lead=on oct=4\n").cid, c.cid);
    assert_eq!(compile("key c\ndeck B: harmony I V voice-lead | bass I\n").text.lines().nth(1), Some("deck B: harmony I V voice-lead=on | bass I"));
    // off is the chords as written, and says so in the canonical text
    let off = compile("key c\ndeck B: harmony I V voice-lead=off\n");
    assert!(off.text.contains("voice-lead=off"));
    assert_eq!(cycle(&off, Deck::B, "harmony", 0), cycle(&compile("key c\ndeck B: harmony I V\n"), Deck::B, "harmony", 0));
    assert_ne!(cycle(&on, Deck::B, "harmony", 0), cycle(&off, Deck::B, "harmony", 0));
    // a program is its own fixed point
    let again = compile(&on.text);
    assert_eq!(again.text, on.text);
    assert_eq!(again.cid, on.cid);
    // on a part that plays no chord it is an error that says where it goes
    let src = "key c\ndeck B: bass I V voice-lead\n";
    let e = one_error(src);
    assert!(e.message.contains("shapes the chords of a part"), "{}", e.message);
    assert_eq!(e.position(src), (2, 18, 28));
    one_error("key c\ndeck B: kick x voice-lead\n");
    let e = one_error("key c\ndeck B: harmony I voice-lead=maybe\n");
    assert!(e.message.contains("`on` or `off`"), "{}", e.message);
    let e = one_error("key c\ndeck B: harmony I voice-lead=1\n");
    assert!(e.message.contains("`on` or `off`"), "{}", e.message);
    // `off` on a part that plays no chord is nothing
    compile("key c\ndeck B: bass I voice-lead=off\n");
    // a misspelling says what it means
    let e = one_error("key c\ndeck B: harmony I voice_lead=on\n");
    assert!(e.help.as_deref().unwrap_or("").contains("voice-lead"), "{e:?}");
}

#[test]
fn voice_leading_applies_to_chord_calls_and_arps_and_is_per_part_and_per_section() {
    // chord(...) calls are led like numerals (the same four chords of I V vi IV)
    let c = compile("key c\ndeck B: harmony chord(c) chord(g) chord(am) chord(f) voice-lead\n");
    assert_eq!(chords_of(&c, Deck::B, "harmony"), vec![vec![48, 52, 55], vec![47, 50, 55], vec![48, 52, 57], vec![48, 53, 57]]);
    // a chord with its octave written is voiced as written, and is the chord before for the next one
    let c = compile("key c\ndeck B: harmony chord(c, 4) chord(g) voice-lead\n");
    assert_eq!(chords_of(&c, Deck::B, "harmony"), vec![vec![60, 64, 67], vec![59, 62, 67]], "G from C4 E4 G4: the G stays, the E to D, the C to B");
    // a voice-led arp plays the led chord one note at a time
    let c = compile("key c\ndeck B: arp I V voice-lead\n");
    assert_eq!(cycle(&c, Deck::B, "arp", 0), vec![(0, 60), (2, 64), (4, 67), (6, 60), (8, 59), (10, 62), (12, 67), (14, 59)]);
    // the harmony part does not lead the bass or another part
    let c = compile("key c\ndeck B: harmony I V voice-lead | bass I V\n");
    assert_eq!(cycle(&c, Deck::B, "bass", 0), vec![(0, 36), (8, 43)]);
    // each section starts its parts afresh: root position first
    let c = compile("key c\nsection s1 1 bars {\n  deck B: harmony I V voice-lead\n}\nsection s2 1 bars {\n  deck B: harmony V I voice-lead\n}\narrange s1 s2\n");
    assert_eq!(seg_notes(&c, "s2", Deck::B, "harmony")[..3], [55, 59, 62]);
}

#[test]
fn voice_leading_follows_the_chords_through_the_bars_of_a_longer_part() {
    // an alternation: one chord a bar; the chord before is the one in the bar before
    let c = compile("key c\ndeck B: harmony <I V vi IV> voice-lead\n");
    assert_eq!(c.decks[&Deck::B].period, 4);
    let bars: Vec<Vec<u8>> = (0..4).map(|b| cycle(&c, Deck::B, "harmony", b).into_iter().map(|(_, n)| n).collect()).collect();
    assert_eq!(bars, vec![vec![48, 52, 55], vec![47, 50, 55], vec![48, 52, 57], vec![48, 53, 57]]);
    // two parts of one deck lead separately
    let c = compile("key c\ndeck B: harmony <I V> voice-lead | arp <vi IV> voice-lead\n");
    assert_eq!(c.decks[&Deck::B].period, 2);
    assert_eq!(cycle(&c, Deck::B, "arp", 0)[0].1, 69, "the arp is its own part: its first chord is vi in root position, A4");
}

// ---- properties

const ALL: &str = "\
tempo 100
key am
let tune = deg(1 b3 5' ~ #4, 1, [1,3] <3 5>)
let prog = i VI/3 III VII
deck A: kick x ~ x ~
deck B: harmony prog voice-lead | bass i VI III V7/7 | melody tune | arp ii\u{b0} V7 bVII #iv\u{b0}
section verse 4 bars {
  deck B: arp i Isus4 Iaug vii\u{f8}7 voice-lead=off
}
section lift 4 bars {
  key fm
  deck B: arp bII V7/7 imaj7 VI9
}
arrange verse lift verse
";

#[test]
fn random_reformatting_keeps_the_cid_of_a_program_that_uses_everything() {
    let want = compile(ALL);
    let mut rng = Rng(0xf5f5_f5f5);
    for round in 0..200 {
        let text = reformat(ALL, &mut rng);
        let c = check(&text);
        let got = c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", render_all(&c.diagnostics, &text)));
        assert_eq!(got.cid, want.cid, "round {round} forked:\n{text}");
        assert_eq!(got.text, want.text, "round {round}");
    }
}

#[test]
fn the_canonical_text_is_a_fixed_point_and_deterministic() {
    let c = compile(ALL);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    assert_eq!(compile(ALL).net_cids(), c.net_cids());
    assert_eq!(compile(ALL).cid, c.cid);
    // marks keep their spans pointing into the text that was evaluated
    for s in &c.segments {
        for ir in s.decks.values() {
            for p in &ir.parts {
                for chain in &p.spans {
                    assert!(chain.iter().all(|sp: &Span| sp.end <= ALL.len()));
                }
            }
        }
    }
}

/// A random step for a part that plays chords: a numeral with a random quality, case, accidental and inversion.
fn random_numeral(rng: &mut Rng) -> String {
    let numerals = ["I", "II", "III", "IV", "V", "VI", "VII"];
    let qualities = ["", "", "7", "maj7", "6", "9", "sus2", "sus4", "aug", "\u{b0}", "dim", "\u{f8}", "\u{b0}7", "dim7"];
    let n = numerals[rng.below(7) as usize];
    let q = qualities[rng.below(qualities.len() as u64) as usize];
    let n = if rng.below(2) == 0 { n.to_lowercase() } else { n.to_string() };
    let acc = ["", "", "", "b", "#"][rng.below(5) as usize];
    // an inversion only where the chord has the tone
    let plain = parse_roman(&format!("{acc}{n}{q}")).unwrap();
    let invs: Vec<u8> = [3u8, 5, 7].into_iter().filter(|t| plain.quality.factors().contains(t)).collect();
    let inv = if rng.below(4) == 0 && !invs.is_empty() { format!("/{}", invs[rng.below(invs.len() as u64) as usize]) } else { String::new() };
    format!("{acc}{n}{q}{inv}")
}

fn random_deg(rng: &mut Rng, depth: usize) -> String {
    match rng.below(if depth < 2 { 8 } else { 5 }) {
        0 => "~".to_string(),
        1..=4 => {
            let acc = ["", "", "b", "#"][rng.below(4) as usize];
            let n = 1 + rng.below(9);
            let marks = ["", "", "", "'", "''", ","][rng.below(6) as usize];
            let tail = ["", "", "", "!", "^", "*2"][rng.below(6) as usize];
            format!("{acc}{n}{marks}{tail}")
        }
        5 => {
            let inner: Vec<String> = (0..2 + rng.below(2)).map(|_| random_deg(rng, depth + 1)).collect();
            // a stack: the comma glued to what follows, a space before it where the term before ends in a comma mark
            let mut s = String::from("[");
            for (i, t) in inner.iter().enumerate() {
                if i > 0 {
                    s.push_str(if s.ends_with(',') { " ," } else { "," });
                }
                s.push_str(t);
            }
            s.push(']');
            s
        }
        6 => format!("<{} {}>", random_deg(rng, depth + 1), random_deg(rng, depth + 1)),
        _ => "x".to_string(),
    }
}

fn random_program(rng: &mut Rng) -> String {
    let keys = ["am", "c", "d:dorian", "f#m", "bb", "e:mixolydian", "c:pent"];
    let mut s = format!("key {}\n", keys[rng.below(7) as usize]);
    let line = |rng: &mut Rng| -> String {
        let mut parts = Vec::new();
        parts.push(format!("harmony {}{}", (0..1 + rng.below(4)).map(|_| random_numeral(rng)).collect::<Vec<_>>().join(" "), ["", " voice-lead", " voice-lead=on", " voice-lead=off"][rng.below(4) as usize]));
        parts.push(format!("bass {}", (0..1 + rng.below(4)).map(|_| random_numeral(rng)).collect::<Vec<_>>().join(" ")));
        let k = 1 + rng.below(4);
        parts.push(format!("melody deg({})", (0..k).map(|_| random_deg(rng, 0)).collect::<Vec<_>>().join(" ")));
        parts.push(format!("arp {}", (0..1 + rng.below(3)).map(|_| random_numeral(rng)).collect::<Vec<_>>().join(" ")));
        format!("deck B: {}\n", parts.join(" | "))
    };
    s.push_str(&line(rng));
    if rng.below(2) == 0 {
        s.push_str(&format!("section one 2 bars {{\n  key {}\n  {}}}\nsection two 2 bars {{\n  {}}}\narrange one two one\n", keys[rng.below(7) as usize], line(rng), line(rng)));
    }
    s
}

#[test]
fn random_programs_are_total_deterministic_and_have_one_canonical_form() {
    let mut rng = Rng(0x5ca1_ab1e);
    let mut compiled = 0;
    for round in 0..300 {
        let src = random_program(&mut rng);
        let c = check(&src);
        let Some(first) = c.compiled else {
            // a random program may be out of range (a ninth chord on a high octave); it says so, with a place
            assert!(c.diagnostics.iter().any(Diagnostic::is_error), "round {round}: {src}");
            if std::env::var("SHOW_FAILS").is_ok() {
                println!("{}\n{}", src, render_all(&c.diagnostics, &src));
            }
            continue;
        };
        compiled += 1;
        let again = check(&first.text).compiled.unwrap_or_else(|| panic!("round {round}: the canonical text does not compile:\n{}\nfrom\n{src}", first.text));
        assert_eq!(again.text, first.text, "round {round}: not a fixed point\n{src}");
        assert_eq!(again.cid, first.cid, "round {round}");
        assert_eq!(check(&src).compiled.unwrap().net_cids(), first.net_cids(), "round {round}: deterministic");
        for k in 0..3 {
            let text = reformat(&src, &mut rng);
            let r = check(&text);
            let got = r.compiled.unwrap_or_else(|| panic!("round {round}.{k} no longer compiles:\n{text}\n{}", render_all(&r.diagnostics, &text)));
            assert_eq!(got.cid, first.cid, "round {round}.{k} forked:\n{text}\nfrom\n{src}");
        }
    }
    assert!(compiled > 200, "most random programs compile: {compiled}");
}

#[test]
fn malformed_text_is_an_error_and_never_a_panic() {
    // random strings of the pieces of F5 (and its neighbours) in any order: check answers, it does not panic
    let pieces = [
        "deg(", ")", "(", "[", "]", "<", ">", ",", ", ", "'", "''", "#", "#4", "#iv", "# ", "b3", "5", "1", "0", "99999999999", "x", "~", "I", "V7", "ii\u{b0}", "vii\u{f8}7", "bVII", "/", "/3", "/7", "I/3", " ", "  ", "\n", "\t", "|", "harmony ", "bass ", "kick ", "arp ", "melody ", "deck B: ", "key ", "am", "fm", "section s 2 bars {", "}", "voice-lead", "voice-lead=on", "=", "!", "^", "*2", "*", "\u{b0}", "\u{f8}", "\u{e9}", "Imaj7", "Isus4", "viii", "Iv", "let t = ", "t", "oct=3", ".", "..", "?", "+", "-", "\"", "$i", "%",
    ];
    let mut rng = Rng(0xdead_beef_cafe);
    for round in 0..4000 {
        let n = 1 + rng.below(14);
        let mut src = String::new();
        if rng.below(2) == 0 {
            src.push_str("deck B: harmony ");
        }
        for _ in 0..n {
            src.push_str(pieces[rng.below(pieces.len() as u64) as usize]);
        }
        let c = check(&src);
        // whatever it is: errors have a place inside the text, and an ok program is its own fixed point
        for d in &c.diagnostics {
            assert!(d.span.end <= src.len() && d.span.start <= d.span.end, "round {round}: {src:?}: {d:?}");
        }
        match c.compiled {
            Some(ok) => {
                let again = check(&ok.text).compiled.unwrap_or_else(|| panic!("round {round}: {src:?} compiled but its canonical text {:?} does not", ok.text));
                assert_eq!(again.text, ok.text, "round {round}: {src:?}");
            }
            None => assert!(c.diagnostics.iter().any(Diagnostic::is_error), "round {round}: {src:?}"),
        }
        // the highlighter and the lexer take it too
        let _ = djlive::highlight::highlight(&src);
    }
}

#[test]
fn a_changed_numeral_or_degree_is_another_program() {
    let base = "key c\ndeck B: harmony I V vi IV voice-lead | melody deg(1 b3 5' 1,)\n";
    let want = compile(base).cid;
    for (from, to) in [("I V", "I IV"), ("vi", "VI"), ("IV voice", "IV7 voice"), ("voice-lead", "voice-lead=off"), ("1 b3", "1 3"), ("5'", "5"), ("1,)", "1)"), ("key c", "key d")] {
        let edited = base.replacen(from, to, 1);
        assert_ne!(compile(&edited).cid, want, "{from} -> {to}");
    }
}

#[test]
fn old_programs_keep_their_cids() {
    // the fixtures and examples that were there before F5: their program CIDs, pinned from the base commit
    for (name, src, cid) in [
        ("acid-rave-138", include_str!("fixtures/acid-rave-138.btw"), "z4EBG9jDqdJiCPANzF2EYvMTjnh6k4g66cMwwxtSKAYhaZLMUb2"),
        ("hand-dubstep-140", include_str!("fixtures/hand-dubstep-140.btw"), "z4EBG9jD5z7Kk3J1PeAG7PmE1hTNikEv1rNXWTdpYux1EDAEmMR"),
        ("hand-proghouse-124", include_str!("fixtures/hand-proghouse-124.btw"), "z4EBG9jCfiLnPMeYSVLsEYac76udGyYPwuJ5iy5xgd3GRncvasZ"),
        ("trance-138 longhand", include_str!("fixtures/trance-138.live"), "z4EBG9jE4UEek5HkFX1ier8wannZ8AaokXoyG5o72ZBMkZcHURk"),
        ("hand-dubstep-140-v2", include_str!("../../../docs/btw/hand-dubstep-140-v2.btw"), "z4EBG9j9cjqaYU5uVNFH6YKAMoiyPfNkof87KEXjPNPpAhJzDhi"),
        ("trance-138", include_str!("../../../examples/trance-138.btw"), "z4EBG9jDzsE2UHZfL8A9ouKLU5vUJsgHC8nVaTE7sWjci7PCukZ"),
        ("markers-demo", include_str!("../../../examples/markers-demo.btw"), "z4EBG9jFNsEJ8p6Gr2mmuGvfG3qX58bFUf42LqTYh9Nw4BtrSaE"),
        ("baseline", include_str!("../../../examples/btw/baseline.btw"), "z4EBG9j7F6VjLQ8Q3kqEQXdgxKKcKM3nMN2qDt1ukwv8gwgJ6dv"),
    ] {
        assert_eq!(compile(src).cid, cid, "{name}");
    }
    // a `#` that starts a comment inside a program is still one, and a bare `b3` is still the note
    let c = compile("key c\ndeck B: melody 1 b3 # 4 beats, #2 of the bar\n");
    assert_eq!(cycle(&c, Deck::B, "melody", 0), vec![(0, 60), (8, 59)]);
    // notes, chords and keys that look nothing like a numeral are untouched
    let c = compile("key bbm\ndeck A: bass bb2 db3 eb | harmony chord(bbm7) | arp arp(f#m7b5, down, 4)\n");
    assert!(c.text.contains("bass a#2 c#3 d#") && c.text.contains("chord(a#m7)"), "{}", c.text);
}

#[test]
fn highlight_knows_numerals_degrees_and_voice_lead() {
    use djlive::highlight::{highlight, Class};
    let src = "deck B: harmony ii\u{b0} V7/7 #iv voice-lead | melody deg(1 b3 #4, 5')";
    let classes: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    let get = |t: &str| classes.iter().find(|(s, _)| *s == t).map(|(_, c)| *c);
    assert_eq!(get("ii\u{b0}"), Some(Class::Step));
    assert_eq!(get("V7"), Some(Class::Step));
    assert_eq!(get("#iv"), Some(Class::Step));
    assert_eq!(get("voice-lead"), Some(Class::Param));
    assert_eq!(get("deg"), Some(Class::Generator));
    assert_eq!(get("b3"), Some(Class::Step));
    assert_eq!(get("#4"), Some(Class::Step));
}
