//! A Roman numeral where `chord()` and `arp()` take a chord symbol (docs/btw/ext-scale.md, "Numerals in calls"): the
//! numeral is read in the key in force, so a section's `key` moves it, and it plays exactly the notes the same numeral
//! plays as a step of a part. Pitches: harmony is octave 3 (C3 = 48), bass 2, melody and arp 4 (C4 = 60).

mod common;

use common::{reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic};
use djlive::music::Chord;
use djlive::scale::parse_roman;

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

/// (step, note) of the first cycle of a part, in the order the part plays them.
fn cycle(c: &Compiled, deck: Deck, role: &str) -> Vec<(u8, u8)> {
    let p = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part"));
    p.cycles[0].iter().map(|h| (h.step, h.note)).collect()
}

/// The notes of the first cycle of a part, steps dropped.
fn notes(src: &str, deck: Deck, role: &str) -> Vec<u8> {
    cycle(&compile(src), deck, role).into_iter().map(|(_, n)| n).collect()
}

fn sorted(mut v: Vec<u8>) -> Vec<u8> {
    v.sort_unstable();
    v
}

fn seg_notes(c: &Compiled, name: &str, role: &str) -> Vec<(u8, u8)> {
    let seg = c.segments.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no segment {name}"));
    let p = seg.decks[&Deck::B].parts.iter().find(|p| p.role == role).unwrap();
    p.cycles[0].iter().map(|h| (h.step, h.note)).collect()
}

/// Every spelling a numeral has: an accidental, a degree in either case, a quality, an inversion the chord has.
fn numerals() -> Vec<String> {
    let mut out = Vec::new();
    for acc in ["", "b", "#"] {
        for n in ["I", "II", "III", "IV", "V", "VI", "VII", "i", "ii", "iii", "iv", "v", "vi", "vii"] {
            for q in ["", "7", "maj7", "\u{b0}", "\u{b0}7", "\u{f8}", "aug", "sus2", "sus4", "6", "9"] {
                let w = format!("{acc}{n}{q}");
                if parse_roman(&w).is_ok() {
                    out.push(w);
                }
            }
        }
    }
    out.extend(["I/3", "I/5", "V7/7", "V7/3", "ii\u{b0}7/5", "bVII/3", "i6/5"].map(String::from));
    out
}

// ---- the rule: where a chord symbol goes, a numeral goes

#[test]
fn chord_of_a_numeral_plays_the_notes_the_numeral_plays_as_a_step() {
    let keys = ["c", "am", "f#m", "bb", "d:dorian", "e:phrygian", "g:mixolydian", "a:harmonic", "c:pent", "a:minpent", "a:blues"];
    let all = numerals();
    assert!(all.len() > 200, "{}", all.len());
    for key in keys {
        for n in &all {
            for oct in [2, 3, 4] {
                let step = notes(&format!("key {key}\ndeck B: harmony {n} oct={oct}\n"), Deck::B, "harmony");
                let call = notes(&format!("key {key}\ndeck B: harmony chord({n}, {oct})\n"), Deck::B, "harmony");
                assert_eq!(sorted(call.clone()), sorted(step.clone()), "chord({n}, {oct}) in {key}");
                // (a written inversion has its tone lowest, as the step does)
                assert_eq!(call, step, "chord({n}, {oct}) in {key}: order");
            }
        }
    }
}

#[test]
fn arp_of_a_numeral_runs_through_the_notes_of_the_numeral() {
    // i in A minor: A4 C5 E5, round the eight steps
    let a = cycle(&compile("key am\ndeck B: arp arp(i, up, 8)\n"), Deck::B, "arp");
    assert_eq!(a, [(0, 69), (2, 72), (4, 76), (6, 69), (8, 72), (10, 76), (12, 69), (14, 72)]);
    // the same chord named as a symbol: the same arp
    assert_eq!(a, cycle(&compile("key am\ndeck B: arp arp(am, up, 8)\n"), Deck::B, "arp"));
    // down and updown, 16 steps
    let d = notes("key am\ndeck B: arp arp(i, down, 4)\n", Deck::B, "arp");
    assert_eq!(d, [76, 72, 69, 76]);
    let u = notes("key am\ndeck B: arp arp(V7, updown, 8)\n", Deck::B, "arp");
    // V7 in A minor is E G# B D, one octave above the tonic's own: E5 G#5 B5 D6 and back down
    assert_eq!(u, [76, 80, 83, 86, 83, 80, 76, 80]);
    assert_eq!(notes("key am\ndeck B: arp arp(VI, up, 16)\n", Deck::B, "arp").len(), 16);
    // the default mode and step count
    assert_eq!(notes("key c\ndeck B: arp arp(V)\n", Deck::B, "arp"), [67, 71, 74, 67, 71, 74, 67, 71]);
}

#[test]
fn a_numeral_is_read_in_the_key_of_the_section_that_plays_it() {
    let src = "key am\nlet arps = <arp(i, up, 8) arp(VI, updown, 16)>\nlet pad = <chord(i, 3) chord(VI, 3)>\nsection verse 2 bars {\n  deck B: arp arps | harmony pad\n}\nsection lift 2 bars {\n  key cm\n  deck B: arp arps | harmony pad\n}\narrange verse lift\n";
    let c = compile(src);
    let (verse, lift) = (seg_notes(&c, "verse", "arp"), seg_notes(&c, "lift", "arp"));
    assert_eq!(verse.iter().map(|x| x.1).collect::<Vec<_>>()[..3], [69, 72, 76]);
    // C minor: C4 Eb4 G4, a minor third above A minor's A4 C5 E5 ... below it: 60 63 67
    assert_eq!(lift.iter().map(|x| x.1).collect::<Vec<_>>()[..3], [60, 63, 67]);
    let (pv, pl) = (seg_notes(&c, "verse", "harmony"), seg_notes(&c, "lift", "harmony"));
    assert_eq!(pv.iter().map(|x| x.1).collect::<Vec<_>>(), [57, 60, 64]);
    assert_eq!(pl.iter().map(|x| x.1).collect::<Vec<_>>(), [48, 51, 55]);
    // the old way, a symbol: the key moves nothing
    let old = "key am\nlet arps = <arp(am, up, 8)>\nsection verse 1 bars {\n  deck B: arp arps\n}\nsection lift 1 bars {\n  key cm\n  deck B: arp arps\n}\narrange verse lift\n";
    let c = compile(old);
    assert_eq!(seg_notes(&c, "verse", "arp"), seg_notes(&c, "lift", "arp"));
}

#[test]
fn the_key_of_the_program_is_the_default_and_the_numeral_keeps_its_written_colour() {
    // no key: C
    assert_eq!(notes("deck B: harmony chord(V7, 3)\n", Deck::B, "harmony"), [55, 59, 62, 65]);
    // am to c: `i` stays minor
    assert_eq!(notes("key c\ndeck B: harmony chord(i, 3)\n", Deck::B, "harmony"), [48, 51, 55]);
}

// ---- voice leading and the chord before

#[test]
fn a_numeral_in_a_call_is_led_like_a_symbol_and_without_an_octave() {
    // `harmony <I V vi IV> voice-lead` and the same four as calls play the same notes
    let steps = notes("key c\ndeck B: harmony <I V vi IV> voice-lead\n", Deck::B, "harmony");
    for bar in 0..4 {
        let src = |chord: &str| format!("key c\nlet p = <{chord}>\ndeck B: harmony p voice-lead\n");
        let calls = compile(&src("chord(I) chord(V) chord(vi) chord(IV)"));
        let bare = compile(&src("I V vi IV"));
        let get = |c: &Compiled| c.decks[&Deck::B].parts[0].cycles[bar].iter().map(|h| h.note).collect::<Vec<_>>();
        assert_eq!(get(&calls), get(&bare), "bar {bar}");
    }
    assert_eq!(steps.len(), 3);
    // with an octave it is voiced as written, and is the chord before for the one after
    let w = compile("key c\nlet p = <chord(I, 3) chord(V)>\ndeck B: harmony p voice-lead\n");
    let bar = |n: usize| w.decks[&Deck::B].parts[0].cycles[n].iter().map(|h| h.note).collect::<Vec<_>>();
    assert_eq!(bar(0), [48, 52, 55]);
    assert_eq!(bar(1), [47, 50, 55], "V led from C3 E3 G3: B2 D3 G3");
}

#[test]
fn a_rest_keeps_the_chord_before_for_the_next_one() {
    let a = compile("key c\nlet p = <chord(I) ~ chord(V)>\ndeck B: harmony p voice-lead\n");
    let b = compile("key c\nlet p = <chord(I) chord(V)>\ndeck B: harmony p voice-lead\n");
    let at = |c: &Compiled, n: usize| c.decks[&Deck::B].parts[0].cycles[n].iter().map(|h| h.note).collect::<Vec<_>>();
    assert_eq!(at(&a, 2), at(&b, 1));
}

#[test]
fn an_inversion_goes_with_the_numeral() {
    // G7 with its seventh in the bass: F lowest, the other three raised an octave
    assert_eq!(notes("key c\ndeck B: harmony chord(V7/7, 3)\n", Deck::B, "harmony"), [65, 67, 71, 74]);
    assert_eq!(notes("key c\ndeck B: arp arp(I/3, up, 4)\n", Deck::B, "arp"), [64, 67, 72, 64]);
}

// ---- no program changes meaning

#[test]
fn no_numeral_spelling_is_a_chord_symbol() {
    // so a call that took a symbol means what it did, and the order of the two readings cannot matter
    for n in numerals() {
        assert!(Chord::parse(&n).is_none(), "{n} is both a chord symbol and a numeral");
    }
}

#[test]
fn symbols_play_as_before() {
    assert_eq!(notes("deck B: harmony chord(am7, 3)\n", Deck::B, "harmony"), [57, 60, 64, 67]);
    assert_eq!(notes("deck B: arp arp(f#m7b5, down, 8)\n", Deck::B, "arp").len(), 8);
}

// ---- misuse, in the language's terms

#[test]
fn a_numeral_that_does_not_read_says_why() {
    let src = "key am\ndeck B: arp arp(VIII, up, 8)\n";
    let e = one_error(src);
    assert!(e.message.contains("`VIII` is not a numeral: a scale has seven degrees"), "{}", e.message);
    assert_eq!(djlive::diag::line_col(src, e.span.start), (2, 17), "at the numeral");
    let e = one_error("key am\ndeck B: harmony chord(Imaj, 3)\n");
    assert!(e.message.contains("`maj` is not a chord quality after the numeral `I`"), "{}", e.message);
    assert_eq!(e.help.as_deref(), Some("did you mean `Imaj7`?"));
    let e = one_error("key am\ndeck B: harmony chord(Iv)\n");
    assert!(e.message.contains("mixes upper and lower case"), "{}", e.message);
    let e = one_error("key am\ndeck B: harmony chord(I/7)\n");
    assert!(e.message.contains("has no seventh to put in the bass"), "{}", e.message);
}

#[test]
fn a_word_that_is_neither_says_what_a_chord_is() {
    for w in ["foo", "vel", "x", "am9x", "k"] {
        let e = one_error(&format!("key am\ndeck B: harmony chord({w})\n"));
        assert_eq!(e.message, format!("`{w}` is not a chord"));
        assert_eq!(e.found, format!("`{w}`"));
        assert!(e.expected.iter().any(|x| x.contains("a Roman numeral like V7")), "{:?}", e.expected);
        assert!(e.expected.iter().any(|x| x.contains("a chord like am7")), "{:?}", e.expected);
    }
    // a number is no chord either
    let e = one_error("key am\ndeck B: harmony chord(3)\n");
    assert!(e.message.contains("expected a chord like am7 or a Roman numeral like V7"), "{}", e.message);
}

// ---- def parameters and typed ports carry numerals into calls

#[test]
fn a_def_takes_a_numeral_for_a_call() {
    let src = "key am\ndef band(c) {\n  deck B: arp arp($c, up, 8) | harmony chord($c, 3)\n}\nband(VI)\n";
    let c = compile(src);
    // F in A minor above the tonic: F5 A5 C6 ... the numeral counts from the tonic A
    assert_eq!(cycle(&c, Deck::B, "arp").iter().map(|x| x.1).collect::<Vec<_>>()[..3], [77, 81, 84]);
    assert_eq!(cycle(&c, Deck::B, "harmony").iter().map(|x| x.1).collect::<Vec<_>>(), [65, 69, 72]);
}

#[test]
fn a_lone_numeral_flows_through_a_chords_port_into_a_call() {
    let src = "key am\ndef one() out: chords {\n  emit V7\n}\ndef two() in: chords {\n  deck B: arp arp($in, up, 8)\n}\nwire one.out -> two.in\ntwo()\n";
    let c = compile(src);
    assert_eq!(cycle(&c, Deck::B, "arp").iter().map(|x| x.1).collect::<Vec<_>>()[..4], [76, 80, 83, 86]);
    // and a chord(numeral) in a progression port plays in the harmony part that takes it
    let src = "key am\ndef one() out: chords {\n  emit <chord(i) chord(VI)>\n}\ndef two() in: chords {\n  deck B: harmony $in voice-lead\n}\nwire one.out -> two.in\ntwo()\n";
    compile(src);
}

// ---- it is still one canonical text

#[test]
fn a_program_with_numeral_calls_is_canonical_and_stable_under_reformatting() {
    let src = "tempo 120\nkey am\nlet arps = <arp(i, up, 8) arp(VI, updown, 16) arp(V7/7, down, 8) arp(bVII, up)>\nlet pad = <chord(ii\u{b0}) chord(#iv, 3) chord(i)>\nsection verse 2 bars {\n  deck B: arp arps oct=3 | harmony pad voice-lead\n}\nsection lift 2 bars {\n  key cm\n  deck B: arp arps oct=3 | harmony pad voice-lead\n}\narrange verse lift\n";
    let c = compile(src);
    let again = compile(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(0x5eed_6596);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        assert_eq!(compile(&text).cid, c.cid, "round {round}:\n{text}");
    }
}
