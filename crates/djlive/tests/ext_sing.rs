//! F9 `sing "lyrics" on melody voice=alto` (docs/btw/ext-sing.md): the syllables and their notes, the part's own timing, the
//! errors (line, column, did-you-mean), the canonical form and its CID under random reformatting, the neural voice's
//! fallback at check and when the line fires, and the control line's marks. Lyrics here are original lines of our own.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, check_with, Compiled, Effect, Timing};
use djlive::diag::Severity;
use djlive::env::*;
use djlive::runtime::{Grid, Live, LiveHost};
use djlive::sing::FALLBACK;

// ---- environments

fn absent() -> StaticVoices {
    StaticVoices::default()
}

fn installed() -> StaticVoices {
    StaticVoices { installed: true, ..StaticVoices::default() }
}

fn compile_with(src: &str, v: &StaticVoices) -> Compiled {
    let env = Env { voices: Some(v), ..Env::default() };
    let c = check_with(src, &env);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errors_with(src: &str, v: &StaticVoices) -> Vec<(usize, usize, String, Option<String>)> {
    let env = Env { voices: Some(v), ..Env::default() };
    let c = check_with(src, &env);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone())).collect()
}

/// The warning about words the BeatScript lexicon lacks: its own tests are in ext_phonemes.rs; here it is set aside.
const LEXICON_NOTE: &str = "not in the BeatScript lexicon";

fn warnings_with(src: &str, v: &StaticVoices) -> Vec<(usize, usize, String)> {
    let env = Env { voices: Some(v), ..Env::default() };
    let c = check_with(src, &env);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.diagnostics.iter().filter(|d| d.severity == Severity::Warning && !d.message.starts_with(LEXICON_NOTE)).map(|d| (d.position(src).0, d.position(src).1, d.message.clone())).collect()
}

/// (line, column) of the last place `needle` is in `src`.
fn at(src: &str, needle: &str) -> (usize, usize) {
    djlive::diag::line_col(src, src.rfind(needle).unwrap_or_else(|| panic!("no {needle:?} in {src:?}")))
}

fn calls(c: &Compiled, i: usize) -> Vec<(String, JsonValue)> {
    let Effect::Calls(cs) = &c.controls[i].effect else { panic!("not calls") };
    cs.iter().map(|c| (c.noun.clone(), c.params.clone())).collect()
}

fn text(v: &JsonValue, k: &str) -> String {
    v.get(k).and_then(JsonValue::as_str).unwrap_or_else(|| panic!("no {k} in {}", v.canonical_js())).to_string()
}

fn notes(v: &JsonValue) -> Vec<(f64, f64, i64)> {
    v.get("notes")
        .and_then(JsonValue::as_array)
        .expect("notes")
        .iter()
        .map(|n| (n.get("start").and_then(JsonValue::as_f64).unwrap(), n.get("dur").and_then(JsonValue::as_f64).unwrap(), n.get("midi").and_then(JsonValue::as_f64).unwrap() as i64))
        .collect()
}

/// A part of four quarter notes and a deck B to put it on.
const SONG: &str = "tempo 120\nkey am\ndeck B: melody c4 e4 g4 e4 | harmony chord(am)\n";

// ---- the line, compiled

#[test]
fn a_sing_line_is_a_control_that_creates_a_vocal_on_the_parts_notes_and_puts_it_on_the_loop_row() {
    let src = format!("{SONG}deck B: sing \"robots on the road\" on melody bars=2 voice=robot\n");
    let c = compile_with(&src, &absent());
    assert_eq!(c.controls.len(), 1);
    assert_eq!(c.controls[0].timing, Timing::Now);
    let cs = calls(&c, 0);
    assert_eq!(cs.len(), 2);
    let (noun, create) = &cs[0];
    assert_eq!(noun, "vocal");
    assert_eq!(text(create, "action"), "create");
    // five syllables on eight notes: it wraps, up to the word that reaches the last note
    assert_eq!(text(create, "lyrics"), "robots on the road robots on");
    assert_eq!(create.get("bars").and_then(JsonValue::as_f64), Some(2.0));
    assert_eq!(text(create, "preset"), "robot");
    assert_eq!(text(create, "key"), "A minor");
    assert_eq!(create.get("bpm").and_then(JsonValue::as_f64), Some(120.0));
    assert_eq!(text(create, "name"), "robots on the road");
    // timed to the part: its four quarters, twice, c4 e4 g4 e4, each until the next with a sixteenth of air
    let n = notes(create);
    assert_eq!(n.iter().map(|x| x.0).collect::<Vec<_>>(), [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    assert_eq!(n.iter().map(|x| x.2).collect::<Vec<_>>(), [60, 64, 67, 64, 60, 64, 67, 64]);
    assert_eq!(n.iter().map(|x| x.1).collect::<Vec<_>>(), [0.75, 0.75, 0.75, 0.75, 0.75, 0.75, 0.75, 1.0]);
    let (noun, to_row) = &cs[1];
    assert_eq!((noun.as_str(), text(to_row, "action"), to_row.get("play").and_then(JsonValue::as_bool)), ("vocal", "to_row".into(), Some(true)));
}

#[test]
fn a_part_with_rests_sings_only_where_it_sounds_and_the_cover_is_the_parts_own_cycle() {
    let src = "deck B: bass c2 ~ [e2 g2] ~ <a2 b2>\ndeck B: sing \"one two three\" on bass voice=robot\n";
    let c = compile_with(src, &absent());
    let Some((_, create)) = calls(&c, 0).into_iter().next() else { panic!() };
    // the alternation <a2 b2> makes the part two bars long: the line covers both
    assert_eq!(create.get("bars").and_then(JsonValue::as_f64), Some(2.0));
    // bar one: c2 at 0, e2 and g2 in the third quarter, a2 in the fifth slot: five terms, 16 steps: snaps; just check the starts are the part's
    let part = &c.decks[&djlive::ast::Deck::B].parts[0];
    let want: Vec<f64> = (0..2usize)
        .flat_map(|bar| part.cycles[bar].iter().map(move |h| (bar * 16 + usize::from(h.step)) as f64 / 4.0))
        .collect();
    let mut want = want;
    want.dedup();
    assert_eq!(notes(&create).iter().map(|x| x.0).collect::<Vec<_>>(), want);
}

#[test]
fn a_shorter_lyric_wraps_and_a_longer_one_stops_with_a_warning_that_says_how_many() {
    let wrap = format!("{SONG}deck B: sing \"la la\" on melody voice=robot bars=2\n");
    let c = compile_with(&wrap, &absent());
    let (_, create) = calls(&c, 0).remove(0);
    assert_eq!(text(&create, "lyrics"), "la la la la la la la la");
    assert!(c.warnings.iter().all(|w| !w.message.contains("not sung")), "{:?}", c.warnings);
    // four notes, five syllables: the last is not sung
    let stop = format!("{SONG}deck B: sing \"robots on the road\" on melody voice=robot\n");
    let w = warnings_with(&stop, &absent());
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!((w[0].0, w[0].2.as_str()), (4, "the lyric has 5 syllables and the part 4 notes in 1 bar: the last 1 is not sung"));
    let c = compile_with(&stop, &absent());
    let (_, create) = calls(&c, 0).remove(0);
    assert_eq!(text(&create, "lyrics"), "robots on the");
    assert_eq!(notes(&create).len(), 4);
}

#[test]
fn a_section_is_the_bars_a_line_in_it_covers_and_it_fires_where_the_section_starts() {
    let src = "section verse 8 bars {\n  deck B: melody c4 e4 g4 e4\n  deck B: sing \"robots on the road\" on melody voice=robot\n}\nsection chorus 4 bars {\n  deck B: melody a4 ~ c5 ~\n  deck B: sing \"la\" on melody voice=robot\n}\narrange verse chorus\n";
    let c = compile_with(src, &absent());
    assert_eq!(c.controls.len(), 2);
    assert_eq!(c.controls[0].timing, Timing::At(0));
    assert_eq!(c.controls[1].timing, Timing::At(8));
    let (_, verse) = calls(&c, 0).remove(0);
    assert_eq!(verse.get("bars").and_then(JsonValue::as_f64), Some(8.0));
    assert_eq!(notes(&verse).len(), 32);
    let (_, chorus) = calls(&c, 1).remove(0);
    assert_eq!(chorus.get("bars").and_then(JsonValue::as_f64), Some(4.0));
    assert_eq!(notes(&chorus).len(), 8);
    assert_eq!(text(&chorus, "lyrics"), "la la la la la la la la");
    assert!(c.controls[0].key.starts_with("[verse@0] ") && c.controls[1].key.starts_with("[chorus@8] "), "{:?}", c.controls.iter().map(|c| &c.key).collect::<Vec<_>>());
}

#[test]
fn a_line_outside_every_section_is_sung_on_the_first_section_that_has_its_part() {
    let src = "section intro 4 bars {\n  deck A: kick x ~ x ~\n}\nsection verse 8 bars {\n  deck B: melody c4 e4 g4 e4\n}\narrange intro verse\nat bar 4: sing \"la la\" on melody voice=robot\n";
    let c = compile_with(src, &absent());
    let (_, create) = calls(&c, 0).remove(0);
    assert_eq!(create.get("bars").and_then(JsonValue::as_f64), Some(8.0));
    assert_eq!(c.controls[0].timing, Timing::At(4));
}

#[test]
fn the_only_deck_with_the_part_is_the_target_and_two_decks_need_saying() {
    let one = "deck A: kick x ~ x ~\ndeck B: melody c4 e4 g4 e4\nsing \"la\" on melody voice=robot\n";
    let c = compile_with(one, &absent());
    assert!(calls(&c, 0)[0].1.get("notes").is_some());
    let both = "deck A: melody c4 e4 g4 e4\ndeck B: melody g3 e3 c3 e3\nsing \"la\" on melody voice=robot\n";
    let e = errors_with(both, &absent());
    assert_eq!((e[0].0, e[0].1), (3, 14));
    assert!(e[0].2.contains("both decks have a `melody` part"), "{e:?}");
    // with its deck named it is that deck's
    let c = compile_with("deck A: melody c4 e4 g4 e4\ndeck B: melody g3 e3 c3 e3\ndeck B: sing \"la\" on melody voice=robot\n", &absent());
    assert_eq!(notes(&calls(&c, 0)[0].1)[0].2, 55);
}

#[test]
fn the_line_may_come_before_the_part_it_is_sung_on() {
    let a = compile_with("deck B: sing \"la la\" on melody voice=robot\ndeck B: melody c4 e4 g4 e4\n", &absent());
    let b = compile_with("deck B: melody c4 e4 g4 e4\ndeck B: sing \"la la\" on melody voice=robot\n", &absent());
    assert_eq!(calls(&a, 0), calls(&b, 0));
}

// ---- errors

#[test]
fn errors_say_where_and_what_would_do() {
    let e = |src: &str| {
        let c = check_with(src, &Env { voices: Some(&absent()), ..Env::default() });
        assert!(!c.ok(), "expected an error in {src:?}");
        let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
        (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone())
    };
    // an empty lyric
    let src = "deck B: melody c4 e4\ndeck B: sing \"  \" on melody\n";
    let r = e(src);
    assert_eq!((r.0, r.1), at(src, "\"  \""));
    assert_eq!(r.2, "there is nothing to sing: the lyric is empty");
    // no words (only marks)
    let src = "deck B: melody c4 e4\ndeck B: sing \"... !\" on melody voice=robot\n";
    let r = e(src);
    assert_eq!((r.0, r.1, r.2.as_str()), (at(src, "\"... !\"").0, at(src, "\"... !\"").1, "there is nothing to sing: the lyric has no words"));
    // an unknown role, with the nearest
    let src = "deck B: melody c4 e4\ndeck B: sing \"la\" on melodi\n";
    let r = e(src);
    assert_eq!((r.0, r.1), (2, 22));
    assert_eq!((r.0, r.1), at(src, "melodi"));
    assert_eq!(r.2, "`melodi` is not a part to sing on");
    assert_eq!(r.3.as_deref(), Some("did you mean `melody`?"));
    // a part the deck does not have, with what it has
    let src = "deck B: harmony chord(am)\ndeck B: sing \"la\" on melody voice=robot\n";
    let r = e(src);
    assert_eq!((r.0, r.1), at(src, "melody"));
    assert_eq!(r.2, "deck B has no `melody` part to sing on");
    assert_eq!(r.3.as_deref(), Some("deck B has: harmony"));
    let r = e("deck B: bass c2 e2\ndeck B: sing \"la\" on bas voice=robot\n");
    assert_eq!(r.2, "`bas` is not a part to sing on");
    // a part on another deck only
    let r = e("deck A: melody c4 e4\ndeck B: kick x\ndeck B: sing \"la\" on melody voice=robot\n");
    assert_eq!((r.0, r.2.as_str()), (3, "deck B has no `melody` part to sing on"));
    // a drum has no pitch
    let src = "deck A: kick x ~ x ~\ndeck A: sing \"la\" on kick voice=robot\n";
    let r = e(src);
    assert_eq!((r.0, r.1), at(src, "kick voice"));
    assert_eq!(r.2, "`kick` is a drum: it has no pitch to sing on");
    // the master has no parts
    let r = e("deck A: melody c4\nmaster: sing \"la\" on melody voice=robot\n");
    assert_eq!((r.0, r.1), (2, 1));
    assert!(r.2.contains("the master has none"), "{r:?}");
    // a part with no notes
    let r = e("deck B: melody ~ ~ ~ ~\ndeck B: sing \"la\" on melody voice=robot\n");
    assert!(r.2.contains("has no notes"), "{r:?}");
    // a voice that is not one, with the nearest names
    let src = "deck B: melody c4 e4\ndeck B: sing \"la\" on melody voice=altoo\n";
    let r = e(src);
    assert_eq!((r.0, r.1), at(src, "altoo"));
    assert_eq!(r.2, "`altoo` is not a voice");
    assert_eq!(r.3.as_deref(), Some("did you mean `alto`?"));
    // a key that is not one
    let r = e("deck B: melody c4 e4\ndeck B: sing \"la\" on melody key=h voice=robot\n");
    assert_eq!((r.0, r.2.as_str()), (2, "`h` is not a key"));
    // only rests
    let src = "deck B: melody c4 e4\ndeck B: sing \"_ _\" on melody voice=robot\n";
    let r = e(src);
    assert_eq!((r.0, r.1, r.2.as_str()), (at(src, "\"_ _\"").0, at(src, "\"_ _\"").1, "there is nothing to sing: the lyric is all rests"));
    // the shape of the line
    assert_eq!(e("sing la on melody\n").2, "`sing` needs the words in quotes or a phrase, found `la`");
    assert!(e("sing \"la\" melody\n").2.contains("expected `on`"));
    assert!(e("sing \"la\" on\n").2.contains("expected the part to sing on"));
    assert_eq!(e("sing \"la\" on melody voic=alto\n").3.as_deref(), Some("did you mean `voice`?"));
    assert_eq!(e("sing \"la\" on melody voice=alto voice=tenor\n").2, "`voice` is given twice");
    assert_eq!(e("sing \"la\" on melody bars=40\n").2, "a sung line is 1 to 32 bars");
    assert_eq!(e("sing \"la\" on melody bars=0\n").2, "a sung line is 1 to 32 bars");
    assert!(e("sing \"la\" on melody voice alto\n").2.contains("`voice` takes a value"));
    // a sung line is not put back
    let r = e("deck B: melody c4 e4\nat bar 2 for 4 bars: deck B: sing \"la\" on melody voice=robot\n");
    assert!(r.2.contains("`for N bars` needs something that can be put back"), "{r:?}");
    // sing is a word of the language
    let r = e("let sing = 4\n");
    assert!(r.2.contains("cannot be a name"), "{r:?}");
}

#[test]
fn a_part_that_did_not_compile_is_not_blamed_twice() {
    let src = "deck B: melody c4 q9 e4\ndeck B: sing \"la\" on melody voice=robot\n";
    let c = check_with(src, &Env { voices: Some(&absent()), ..Env::default() });
    let errs: Vec<_> = c.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errs.len(), 1, "{}", djlive::diag::render_all(&c.diagnostics, src));
    assert_eq!(errs[0].position(src).0, 1);
}

// ---- the canonical form

const PROGRAM: &str = "tempo 124\nkey am\ndeck B: melody c4 e4 g4 e4 | harmony chord(am)\ndeck B: sing \"robots on the road\" on melody voice=alto bars=2\nat bar 4: deck B: sing \"la la\" on harmony voice=robot\n";

#[test]
fn the_canonical_form_is_a_fixed_point_with_parameters_sorted() {
    let c = check(PROGRAM).compiled.unwrap();
    assert!(c.text.contains("deck B: sing \"robots on the road\" on melody bars=2 voice=alto\n"), "{}", c.text);
    assert!(c.text.contains("at bar 4: deck B: sing \"la la\" on harmony voice=robot\n"), "{}", c.text);
    let again = check(&c.text).compiled.unwrap();
    assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()));
    // the same program written another way is the same program
    let other = "tempo 124\nkey am\ndeck B: melody c4 e4 g4 e4 | harmony chord(am)\n  deck B:  sing   \"robots on the road\"   on melody   bars = 2   voice = alto   # words\nat   bar 4: deck B: sing \"la la\" on harmony voice=robot\n";
    assert_eq!(check(other).compiled.unwrap().cid, c.cid);
    // `key=` is spelled one way
    let a = check("deck B: melody c4\ndeck B: sing \"la\" on melody key=Bbm voice=robot\n").compiled.unwrap();
    let b = check("deck B: melody c4\ndeck B: sing \"la\" on melody key=a#m voice=robot\n").compiled.unwrap();
    assert_eq!(a.cid, b.cid);
}

#[test]
fn random_reformatting_keeps_the_cid_and_an_edit_changes_it() {
    let want = check(PROGRAM).compiled.unwrap().cid;
    let mut rng = Rng(0x51_46_00_dd_ee_ff_12_34);
    for round in 0..150 {
        let text = reformat(PROGRAM, &mut rng);
        let c = check(&text);
        let got = c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{:?}", c.diagnostics)).cid;
        assert_eq!(got, want, "round {round} forked:\n{text}");
    }
    // a semantic edit is another program
    for edit in [
        PROGRAM.replace("robots on the road", "robots on the roads"),
        PROGRAM.replace("on melody", "on harmony"),
        PROGRAM.replace("voice=alto", "voice=tenor"),
        PROGRAM.replace("bars=2", "bars=3"),
        PROGRAM.replace("c4 e4 g4 e4", "c4 e4 g4 f4"),
        PROGRAM.replace("at bar 4", "at bar 5"),
        PROGRAM.replace("sing \"la la\"", "sing \"la lo\""),
    ] {
        let c = check(&edit);
        let got = c.compiled.unwrap_or_else(|| panic!("{edit}\n{:?}", c.diagnostics)).cid;
        assert_ne!(got, want, "{edit}");
    }
}

#[test]
fn a_def_and_a_repeat_expand_sing_lines_with_their_spans_and_calls() {
    let src = "def verse(d, words) {\n  deck $d: melody c4 e4 g4 e4\n  deck $d: sing \"la la\" on melody voice=robot\n}\nverse(B, 1)\nrepeat 2 every 4 bars {\n  deck A: sing \"hey\" on melody voice=robot\n}\ndeck A: melody g3 b3\n";
    let c = check(src);
    // the repeat runs two copies, each its own control; the def's line points at its text and its call
    let c = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&check(src).diagnostics, src)));
    assert_eq!(c.controls.len(), 3);
    let verse = c.controls.iter().find(|x| x.key.contains("deck B: sing")).unwrap();
    // a line out of a def is the def's own text, with the call site in `via`
    assert_eq!(verse.span.text(src), "deck $d: sing \"la la\" on melody voice=robot");
    assert_eq!(verse.via.iter().map(|s| s.text(src)).collect::<Vec<_>>(), ["verse(B, 1)"]);
    let reps: Vec<&djlive::compile::Control> = c.controls.iter().filter(|x| x.key.contains("deck A: sing")).collect();
    assert_eq!(reps.len(), 2);
    assert_eq!(reps[0].span.text(src), "deck A: sing \"hey\" on melody voice=robot");
    assert_eq!(reps[0].via[0].text(src).trim_end(), "repeat 2 every 4 bars {\n  deck A: sing \"hey\" on melody voice=robot\n}");
    // the two copies fire four bars apart
    assert_eq!((reps[0].timing, reps[1].timing), (Timing::Now, Timing::At(4)));
    // and the program, def and all, is its own canonical text
    let again = check(&c.text).compiled.unwrap();
    assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()));
    assert!(c.text.contains("deck $d: sing \"la la\" on melody voice=robot"), "{}", c.text);
}

#[test]
fn the_same_line_over_another_melody_is_another_control() {
    let a = check("deck B: melody c4 e4 g4 e4\ndeck B: sing \"la\" on melody voice=robot\n").compiled.unwrap();
    let b = check("deck B: melody c4 e4 g4 g4\ndeck B: sing \"la\" on melody voice=robot\n").compiled.unwrap();
    assert!(a.controls[0].key.starts_with("deck B: sing \"la\" on melody voice=robot [notes "), "{}", a.controls[0].key);
    assert_ne!(a.controls[0].key, b.controls[0].key);
    // the same program twice is the same control
    let a2 = check("deck B: melody c4 e4 g4 e4\ndeck B: sing \"la\" on melody voice=robot\n").compiled.unwrap();
    assert_eq!(a.controls[0].key, a2.controls[0].key);
    // two identical lines are numbered as any two are
    let two = check("deck B: melody c4 e4\ndeck B: sing \"la\" on melody voice=robot\ndeck B: sing \"la\" on melody voice=robot\n").compiled.unwrap();
    assert!(two.controls[1].key.ends_with("#2"), "{}", two.controls[1].key);
}

// ---- the neural voice

#[test]
fn a_neural_voice_that_is_not_installed_warns_and_plays_on_the_formant_voice() {
    let src = format!("{SONG}deck B: sing \"robots on the\" on melody voice=alto\n");
    let w = warnings_with(&src, &absent());
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!((w[0].0, w[0].1), at(&src, "alto"), "at the voice it named");
    assert_eq!(w[0].2.as_str(), "the neural voice is not installed: falling back to the formant voice");
    assert_eq!(w[0].2, FALLBACK);
    let c = compile_with(&src, &absent());
    let (_, create) = calls(&c, 0).remove(0);
    assert_eq!(text(&create, "preset"), "robot", "the line plays on the formant voice");
    assert_eq!(c.controls[0].notice.as_deref(), Some(FALLBACK));
    // the warning is in the compiled program's warnings too, with the way out
    let help = c.warnings.iter().find(|d| d.message == FALLBACK).and_then(|d| d.help.clone()).unwrap();
    assert!(help.contains("voice=robot"), "{help}");
}

#[test]
fn with_the_neural_voice_installed_the_line_names_it_and_says_nothing() {
    let src = format!("{SONG}deck B: sing \"robots on the\" on melody voice=alto\n");
    assert!(warnings_with(&src, &installed()).is_empty());
    let c = compile_with(&src, &installed());
    assert_eq!(text(&calls(&c, 0)[0].1, "preset"), "alto");
    assert!(c.controls[0].notice.is_none());
    // a line that names no voice sings in the host's default: the neural one when it is installed, the formant one when not
    let none = format!("{SONG}deck B: sing \"robots on the\" on melody\n");
    let c = compile_with(&none, &installed());
    assert_eq!(text(&calls(&c, 0)[0].1, "preset"), "alto");
    let c = compile_with(&none, &absent());
    assert_eq!(text(&calls(&c, 0)[0].1, "preset"), "robot");
    assert!(warnings_with(&none, &absent()).is_empty());
    // unless the host's default is the neural one and it is not there
    let v = StaticVoices { default: Some("alto".into()), ..absent() };
    let w = warnings_with(&none, &v);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].2, FALLBACK);
    assert_eq!((w[0].0, w[0].1), at(&none, "sing"), "at the line's `sing`");
}

#[test]
fn a_formant_voice_is_never_a_fallback() {
    for v in ["robot", "robot-choir", "android", "choir-aah", "choir-ooh", "soprano", "bass-choir", "talkbox", "voice-robot"] {
        let src = format!("{SONG}deck B: sing \"robots on the\" on melody voice={v}\n");
        assert!(warnings_with(&src, &absent()).is_empty(), "{v}");
        let c = compile_with(&src, &absent());
        assert_eq!(text(&calls(&c, 0)[0].1, "preset"), v.strip_prefix("voice-").unwrap_or(v));
    }
}

#[test]
fn with_no_voices_to_ask_the_line_compiles_and_says_nothing_can_be_checked() {
    let src = format!("{SONG}deck B: sing \"robots on the\" on melody voice=alto\n");
    let c = check(&src);
    assert!(c.ok());
    let w: Vec<_> = c.diagnostics.iter().filter(|d| d.severity == Severity::Warning && !d.message.starts_with(LEXICON_NOTE)).collect();
    assert_eq!(w.len(), 1);
    assert!(w[0].message.starts_with("cannot check the voice"), "{}", w[0].message);
    // an unknown name cannot be told from a neural one: it is passed on as written
    let comp = c.compiled.unwrap();
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "alto");
    assert!(comp.controls[0].notice.is_none());
}

#[test]
fn the_voice_has_the_last_word_on_a_syllable_count() {
    // `fire` is one syllable by vowel groups and two to the voice: the line gives it two notes, so nothing drifts
    let src = format!("{SONG}deck B: sing \"fire on the road\" on melody voice=robot bars=2\n");
    let plain = compile_with(&src, &absent());
    assert_eq!(text(&calls(&plain, 0)[0].1, "lyrics"), "fire on the road fire on the road");
    let mut v = absent();
    v.syllables.insert("fire".into(), 2);
    let told = compile_with(&src, &v);
    // five syllables a pass, eight notes
    assert_eq!(text(&calls(&told, 0)[0].1, "lyrics"), "fire on the road fire on");
}

// ---- the `vocal` sugar and the neural default

fn vocal_registry() -> StaticRegistry {
    let p = |name: &str, types: &[&str], required: bool| ParamSpec { name: name.into(), types: types.iter().map(|s| s.to_string()).collect(), options: vec![], required, doc: String::new(), ..Default::default() };
    let mut r = StaticRegistry::default();
    r.nouns.insert(
        "vocal".into(),
        NounSpec {
            name: "vocal".into(),
            default_action: "status".into(),
            actions: ["status", "create", "hook", "to_row"]
                .iter()
                .map(|a| ActionSpec { name: a.to_string(), effect: "write".into(), params: if *a == "create" || *a == "hook" { vec![p("lyrics", &["string"], true)] } else { vec![] } })
                .collect(),
            params: vec![p("lyrics", &["string"], false), p("key", &["string"], false), p("bars", &["integer"], false), p("preset", &["string"], false), p("play", &["boolean"], false)],
        },
    );
    r
}

#[test]
fn the_vocal_sugar_names_a_neural_voice_with_the_same_fallback() {
    let reg = vocal_registry();
    let line = "vocal create \"robots on the road\" key=am bars=4 preset=alto\n";
    let v = absent();
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let c = check_with(line, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, line));
    let w: Vec<_> = c.diagnostics.iter().filter(|d| d.severity == Severity::Warning && !d.message.starts_with(LEXICON_NOTE)).collect();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].message, FALLBACK);
    assert_eq!(w[0].span.text(line), "alto", "at the voice it named");
    let comp = c.compiled.unwrap();
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "robot");
    assert_eq!(comp.controls[0].notice.as_deref(), Some(FALLBACK));
    // the line as written is the program: the canonical text and CID do not know what the host had
    assert!(comp.text.contains("preset=alto"), "{}", comp.text);
    // installed: untouched
    let v = installed();
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let comp = check_with(line, &env).compiled.unwrap();
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "alto");
    assert!(comp.warnings.is_empty());
    // no voice named: the host's default, which says nothing unless it is neural and missing
    let plain = "vocal create \"robots on the road\" key=am bars=4\n";
    let v = absent();
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let comp = check_with(plain, &env).compiled.unwrap();
    assert!(comp.warnings.is_empty());
    assert!(calls(&comp, 0)[0].1.get("preset").is_none());
    let v = StaticVoices { default: Some("tenor".into()), ..absent() };
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let comp = check_with(plain, &env).compiled.unwrap();
    assert_eq!(comp.warnings.len(), 1);
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "robot");
    // on a host whose default is the neural voice and has it, a line that names none sings in it
    let v = installed();
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let comp = check_with(plain, &env).compiled.unwrap();
    assert!(comp.warnings.is_empty());
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "alto");
    // a voice that is none is an error with the nearest
    let v = StaticVoices { default: Some("tenor".into()), ..absent() };
    let bad = "vocal create \"x\" preset=altto\n";
    let env = Env { registry: Some(&reg), voices: Some(&v), ..Env::default() };
    let c = check_with(bad, &env);
    assert!(!c.ok());
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(d.message, "`altto` is not a voice");
    assert_eq!(d.help.as_deref(), Some("did you mean `alto`?"));
    assert_eq!((d.position(bad).0, d.position(bad).1), at(bad, "altto"));
    // with no voices to ask, the sugar is as it was
    let env = Env { registry: Some(&reg), ..Env::default() };
    let comp = check_with(line, &env).compiled.unwrap();
    assert!(comp.warnings.is_empty());
    assert_eq!(text(&calls(&comp, 0)[0].1, "preset"), "alto");
}

// ---- when it fires

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat, noun, params)
    calls: Vec<(f64, String, JsonValue)>,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[usize::from(deck == "B")]
    }
    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[usize::from(deck == "B")] = on;
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.clone()));
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
}

fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

#[test]
fn a_sing_line_fires_once_at_its_bar_and_says_the_fallback_when_it_does() {
    let src = format!("{SONG}at bar 3: deck B: sing \"robots on the\" on melody voice=alto\n");
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let (mut h, mut live) = playing();
    let ev = live.eval_with(&src, &env);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    h.run(&mut live, 4.0 * 8.0);
    let vocal: Vec<&(f64, String, JsonValue)> = h.calls.iter().filter(|c| c.1 == "vocal").collect();
    // create and to_row, once, on program bar 3
    assert_eq!(vocal.len(), 2, "{:?}", h.calls.iter().map(|c| (&c.1, c.2.canonical_js())).collect::<Vec<_>>());
    assert_eq!(text(&vocal[0].2, "action"), "create");
    assert_eq!(text(&vocal[1].2, "action"), "to_row");
    assert_eq!(text(&vocal[0].2, "preset"), "robot");
    let bar = |b: f64| (b / 4.0).floor() as i64;
    assert_eq!(bar(vocal[0].0) - bar(8.0), 3 + 0, "fires on program bar 3 (the program began on the bar it landed on)");
    // the log says so, once, when the line fires
    let said: Vec<&String> = live.words.iter().filter(|w| w.contains(FALLBACK)).collect();
    assert_eq!(said.len(), 1, "{:?}", live.words);
    assert!(said[0].starts_with("bar "), "{}", said[0]);
}

#[test]
fn an_installed_neural_voice_is_sung_in_and_the_log_says_nothing_of_it() {
    let src = format!("{SONG}at bar 2: deck B: sing \"robots on the\" on melody voice=alto\n");
    let v = installed();
    let env = Env { voices: Some(&v), ..Env::default() };
    let (mut h, mut live) = playing();
    assert!(live.eval_with(&src, &env).ok);
    h.run(&mut live, 4.0 * 6.0);
    let create = h.calls.iter().find(|c| c.1 == "vocal").unwrap();
    assert_eq!(text(&create.2, "preset"), "alto");
    assert!(live.words.iter().all(|w| !w.contains("neural voice")), "{:?}", live.words);
}

#[test]
fn editing_the_melody_sings_the_line_again_and_editing_nothing_does_not() {
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let (mut h, mut live) = playing();
    let a = "deck B: melody c4 e4 g4 e4\nsing \"la la\" on melody voice=robot\n";
    assert!(live.eval_with(a, &env).ok);
    h.run(&mut live, 4.0 * 4.0);
    let count = |h: &Fake| h.calls.iter().filter(|c| c.1 == "vocal" && text(&c.2, "action") == "create").count();
    assert_eq!(count(&h), 1);
    // the same text: nothing happens
    assert!(live.eval_with(a, &env).unchanged);
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(count(&h), 1);
    // a note of the melody changes: the line is another control, so it is sung again on the new notes
    let b = "deck B: melody c4 e4 g4 d4\nsing \"la la\" on melody voice=robot\n";
    assert!(live.eval_with(b, &env).ok);
    h.run(&mut live, 4.0 * 8.0);
    assert_eq!(count(&h), 2);
    let last = h.calls.iter().rev().find(|c| c.1 == "vocal" && text(&c.2, "action") == "create").unwrap();
    assert_eq!(notes(&last.2).last().unwrap().2, 62);
}

#[test]
fn the_line_is_lit_while_it_fires_through_its_span() {
    let src = "deck B: melody c4 e4 g4 e4\nat bar 1: deck B: sing \"la la\" on melody voice=robot\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    let mut seen = None;
    for _ in 0..40 {
        h.run(&mut live, 1.0);
        let m = live.marks();
        if let Some(c) = m.controls.iter().find(|c| c.key.contains("sing")) {
            seen = Some((c.span.text(&m.source).to_string(), c.age_beats));
            break;
        }
    }
    let (span, age) = seen.expect("it fired and is marked");
    assert_eq!(span, "at bar 1: deck B: sing \"la la\" on melody voice=robot");
    assert!(age < 4.0, "lit as it fires: {age}");
}

// ---- the editor

#[test]
fn the_editor_knows_sing_for_a_keyword_and_voice_for_a_parameter() {
    use djlive::highlight::{highlight, Class};
    let src = "deck B: sing \"la\" on melody voice=alto\n";
    let c: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    assert!(c.contains(&("sing", Class::Keyword)), "{c:?}");
    assert!(c.contains(&("voice", Class::Param)), "{c:?}");
    assert!(c.contains(&("melody", Class::Role)), "{c:?}");
}

// ---- laws

#[test]
fn the_layout_laws_hold_for_every_lyric_and_every_number_of_notes() {
    use djlive::sing::{assign, layout, syllables};
    let mut rng = Rng(0x5eed_5106_0000_0001);
    let vocab = ["la", "robots", "on", "the", "road", "ro-bo-tic", "_", "harbor", "glowing", "fire", "hmm", "beautiful", "don't"];
    for _ in 0..400 {
        let words: Vec<&str> = (0..1 + rng.below(8)).map(|_| vocab[rng.below(vocab.len() as u64) as usize]).collect();
        let lyric = words.join(" ");
        let lw = syllables(&lyric, None);
        let per: usize = lw.iter().map(|w| w.syllables()).sum();
        let notes = 1 + rng.below(70) as usize;
        let l = layout(&lw, notes).unwrap();
        assert_eq!(l, layout(&lw, notes).unwrap(), "deterministic");
        // it wraps as often as the notes need, and stops when they run out
        assert_eq!((l.syllables, l.passes, l.dropped), (per, notes.div_ceil(per), per.saturating_sub(notes)));
        // the text is whole words: enough for every note, and not a word more than the note that is last needs
        let got = syllables(&l.text, None);
        let have: usize = got.iter().map(|w| w.syllables()).sum();
        assert!(have >= notes, "{lyric:?} on {notes}: {have}");
        let without_last = have - got.last().unwrap().syllables();
        assert!(without_last < notes, "{lyric:?} on {notes}: {} would do", l.text);
        // each note sings the next syllable, round
        let a = assign(per, notes);
        assert_eq!(a.len(), notes);
        assert!(a.iter().enumerate().all(|(i, s)| *s == i % per));
        assert_eq!(a[..per.min(notes)], (0..per.min(notes)).collect::<Vec<_>>());
    }
}

#[test]
fn the_notes_of_any_part_never_overlap_and_stay_inside_the_line() {
    use std::collections::BTreeSet;
    let mut rng = Rng(0x5eed_5106_0000_0002);
    let atoms = ["c4", "e4", "g4", "~", "[c4 e4]", "[c4,e4,g4]", "a3", "~", "x", "g4!", "c5^"];
    for round in 0..200 {
        let terms: Vec<String> = (0..1 + rng.below(9)).map(|_| atoms[rng.below(atoms.len() as u64) as usize].to_string()).collect();
        let pattern = if rng.below(3) == 0 { format!("<{}> {}", terms.join(" "), terms.join(" ")) } else { terms.join(" ") };
        let src = format!("deck B: melody {pattern}\n");
        let Some(c) = check(&src).compiled else { continue };
        let part = &c.decks[&djlive::ast::Deck::B].parts[0];
        let cycles = part.cycles.len().max(1);
        for cover in [1u32, 2, 3, 5, 8] {
            let n = djlive::sing::sung_notes(part, cover);
            assert!(n.windows(2).all(|w| w[0].start < w[1].start && w[0].start + w[0].len <= w[1].start), "round {round} {src:?} cover {cover}: {n:?}");
            assert!(n.iter().all(|x| x.len >= 1 && x.start < cover * 16 && x.start + x.len <= cover * 16), "round {round} {src:?}: {n:?}");
            // the onsets are the part's, bar by bar, the cycles going round
            let want: BTreeSet<u32> = (0..cover).flat_map(|b| part.cycles[b as usize % cycles].iter().map(move |h| b * 16 + u32::from(h.step))).collect();
            assert_eq!(n.iter().map(|x| x.start).collect::<BTreeSet<_>>(), want, "round {round} {src:?} cover {cover}");
            // a chord sings its top note
            for x in &n {
                let top = (0..cover).flat_map(|b| part.cycles[b as usize % cycles].iter().filter(move |h| b * 16 + u32::from(h.step) == x.start).map(|h| h.note)).max().unwrap();
                assert_eq!(x.midi, top, "round {round} {src:?}");
            }
        }
    }
}

#[test]
fn the_same_text_and_voices_compile_to_the_same_calls_every_time() {
    let src = format!("{SONG}at bar 2: deck B: sing \"robots on the road\" on melody voice=alto\nsection verse 4 bars {{\n  deck B: sing \"la\" on harmony voice=robot\n}}\n");
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let a = check_with(&src, &env);
    let b = check_with(&src, &env);
    assert_eq!(a.compiled, b.compiled);
    assert!(a.compiled.is_some(), "{}", djlive::diag::render_all(&a.diagnostics, &src));
}

// ---- phrase breaks beside escapes (6634)

/// What a `sing` line over `SONG` (four notes) sends the voice: the lyric as laid on the notes, and the notes.
fn sung(lyric: &str, extra: &str) -> (String, Vec<(f64, f64, i64)>) {
    let src = format!("{SONG}deck B: sing \"{lyric}\" on melody{extra}\n");
    let c = compile_with(&src, &absent());
    let (_, create) = calls(&c, 0).remove(0);
    (text(&create, "lyrics"), notes(&create))
}

#[test]
fn an_annotated_escape_sings_the_escape_keeps_the_word_and_is_not_a_lexicon_miss() {
    // 6665: `word=/phonemes/` is the escape, labelled: the line sings exactly what the plain escape sings, and check says
    // nothing about the word (it is not sounded out by the rules: it is not sounded at all)
    let lexicon_notes = |lyric: &str| {
        let src = format!("{SONG}deck B: sing \"{lyric}\" on melody\n");
        let c = check(&src);
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
        c.diagnostics.iter().filter(|d| d.message.starts_with(LEXICON_NOTE)).map(|d| d.message.clone()).collect::<Vec<_>>()
    };
    assert_eq!(lexicon_notes("zorblax on the road").len(), 1, "the plain word is a miss");
    assert!(lexicon_notes("zorblax=/z oh r b l ae k s/ on the road").is_empty());
    assert_eq!(sung("zorblax=/z oh r b l ae k s/ on the road", ""), sung("/z oh r b l ae k s/ on the road", ""));
    assert_eq!(sung("hey=/h ay:/ we=/w ee/ go", ""), sung("/h ay:/ /w ee/ go", ""));
    // the plain forms are as they were: a word and a space before `=`, or `=` with no escape after it
    assert_eq!(sung("road =/r oh d/", ""), sung("road /r oh d/", ""));
    // a bad symbol in an annotated escape is the escape's error, at its own column
    let src = format!("{SONG}deck B: sing \"road=/zz oh d/\" on melody\n");
    let e = errors_with(&src, &absent());
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!((e[0].0, e[0].1), at(&src, "zz oh d/"));
}

#[test]
fn a_phrase_break_beside_an_escape_checks_and_sings_what_the_spelling_without_it_sings() {
    // the record: `/h ii/ ! / /h ii/ !` was read as an escape (`/ /`) with nothing in it, at the second and third slash
    let src = format!("{SONG}deck B: sing \"/h ii/ ! / /h ii/ !\" on melody\n");
    let c = check(&src);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    // each form with a lone `/` is sung as the same words with a comma where the break is (a comma never was ambiguous)
    for (slash, comma) in [
        ("/h ii/ ! / /h ii/ !", "/h ii/ !, /h ii/ !"),
        ("/h ii/ / /h aa/", "/h ii/, /h aa/"),
        ("/ /h ii/ /h aa/", "/h ii/ /h aa/"),
        ("/h ii/ /h aa/ /", "/h ii/ /h aa/,"),
        ("/h ii/!/ /h aa/", "/h ii/!, /h aa/"),
        ("/h ii/ / / /h aa/", "/h ii/,, /h aa/"),
        ("we /dh uh/ rave / and /dh uh/ rave", "we /dh uh/ rave, and /dh uh/ rave"),
        ("/h ii/ / hello", "/h ii/, hello"),
        ("hello / /h ii/", "hello, /h ii/"),
    ] {
        assert_eq!(sung(slash, ""), sung(comma, ""), "{slash:?} against {comma:?}");
    }
    // and what that is: two escaped words, a note each
    assert_eq!(sung("/h ii/ ! / /h ii/ !", ""), sung("/h ii/ /h ii/", ""));
    assert_eq!(sung("/h ii/ ! / /h ii/ !", "").0, "/h ii/ /h ii/ /h ii/ /h ii/");
}

#[test]
fn a_lone_slash_between_plain_words_is_a_phrase_break_and_never_an_escape() {
    // each `/` here has a space after it: the first opened an escape that the second closed, and `hold`, `me` and `close`
    // were then phonemes that are not in the set
    let lyric = "deep er in to night / hold me close / we move slow";
    let src = format!("{SONG}deck B: sing \"{lyric}\" on melody bars=4\n");
    let c = check_with(&src, &Env { voices: Some(&absent()), ..Env::default() });
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    assert_eq!(sung(lyric, " bars=4"), sung("deep er in to night, hold me close, we move slow", " bars=4"));
    assert!(sung(lyric, " bars=4").0.contains("hold me close"), "{:?}", sung(lyric, " bars=4").0);
    // several breaks, at the edges, in a row, and on their own line
    for l in ["la / la / la / la", "/ la la /", "la / / la", "la /\\nla /\\nla", "/ la / la / la /"] {
        let src = format!("{SONG}deck B: sing \"{l}\" on melody\n");
        let c = check(&src);
        assert!(c.ok(), "{l:?}: {}", djlive::diag::render_all(&c.diagnostics, &src));
    }
    // a lyric of breaks alone has nothing to sing, and says that
    let src = "deck B: melody c4 e4\ndeck B: sing \"/ /\" on melody voice=robot\n";
    let r = errors_with(src, &absent());
    assert_eq!(r[0].2, "there is nothing to sing: the lyric has no words");
}

#[test]
fn the_forms_that_are_still_refused_stay_refused_at_their_own_line_and_column() {
    let e = |lyric: &str| {
        let src = format!("deck B: melody c4 e4 g4 e4\ndeck B: sing \"{lyric}\" on melody voice=robot\n");
        (src.clone(), errors_with(&src, &absent()))
    };
    // `//`: nothing between the slashes (a break is a `/` with a space after it); at the two slashes, saying how to fix it
    let (src, r) = e("hello // world");
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].0, r[0].1), at(&src, "//"));
    assert!(r[0].2.starts_with("an escape with nothing in it: write phonemes between the slashes (`/h ay/`)"), "{}", r[0].2);
    assert!(r[0].2.contains("a phrase break a `/` with a space after it"), "{}", r[0].2);
    // after an escape, and with a break before it (the pairing starts again after each escape)
    let (src, r) = e("/h ii/ / //");
    assert_eq!((r.len(), (r[0].0, r[0].1)), (1, at(&src, "//")));
    // a symbol that is not in the set, in an escape that follows a break, is reported where it is
    let (src, r) = e("/h ii/ ! / /zz aa/");
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].0, r[0].1), at(&src, "zz aa"));
    assert!(r[0].2.starts_with("`zz` is not a BeatScript phoneme"), "{}", r[0].2);
    // an escape with only a hyphen in it
    let (src, r) = e("a /-/ b");
    assert_eq!((r.len(), (r[0].0, r[0].1)), (1, at(&src, "/-/")));
    assert!(r[0].2.starts_with("an escape with nothing in it"), "{}", r[0].2);
}

#[test]
fn a_slash_that_opens_and_is_never_closed_is_a_phrase_break_as_it_always_was() {
    // `/h ii` has a phoneme after its slash, so it opens an escape; with no closing slash it is read as a break, and
    // `h` and `ii` are plain words
    assert_eq!(sung("/h ii", ""), sung("h ii", ""));
    assert_eq!(sung("go /h ii", ""), sung("go, h ii", ""));
    // a slash that only ends the lyric
    assert_eq!(sung("go on /", ""), sung("go on", ""));
    // a spaced pair is not an escape: both slashes are breaks and `w ee` are plain words (an escape opens on a phoneme)
    assert_eq!(sung("/ w ee /", ""), sung("w ee", ""));
}

#[test]
fn a_lone_slash_with_whitespace_on_both_sides_is_always_a_phrase_break() {
    // the holdout `lyric-lone-slash-breaks`: the lyric as it was written, in a whole program, with no voice named
    let lyric = "deep er in to night / hold me close / we move slow";
    let src = format!("{SONG}deck B: sing \"{lyric}\" on melody\n");
    let c = check(&src);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    // `hold`, `me` and `close` were once "not a BeatScript phoneme": nothing says that, or anything about an escape. What
    // is said is the three notes of a bare `check`: no voice to ask, the lexicon lacking `er` and `close`, and eleven
    // syllables (every word read as a word) over a part of four notes
    assert!(c.diagnostics.iter().all(|d| !d.message.contains("phoneme") && !d.message.contains("escape")), "{:?}", c.diagnostics);
    let notes: Vec<&str> = c.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes.iter().any(|m| m.starts_with(LEXICON_NOTE) && m.contains("`er`") && m.contains("`close`")), "{notes:?}");
    assert!(notes.iter().any(|m| m.starts_with("the lyric has 11 syllables and the part 4 notes")), "{notes:?}");
    // three phrases of plain words: a phrase ends after `night` and after `close`, and the last one ends with the lyric
    let w = djengine::vocal::lexicon::words(lyric);
    let ends: Vec<&str> = w.iter().filter(|w| w.phrase_end).map(|w| w.text.as_str()).collect();
    assert_eq!(ends, ["night", "close"]);
    assert_eq!(w.len(), 11);
    assert!(w.iter().all(|w| w.escaped.is_none()));
    // and the line sings what the same words with commas sing
    assert_eq!(sung(lyric, ""), sung("deep er in to night, hold me close, we move slow", ""));
    // the neighbours: between plain words, at either end, in a row, on tabs, round an escape, and as a lyric's only
    // content; a `/` with no whitespace after it still opens an escape (and with no partner is a break)
    for (slash, comma) in [
        ("a / b", "a, b"),
        ("/ a b", ", a b"),
        ("a b /", "a b,"),
        ("a / / b", "a,, b"),
        ("a\t/\tb", "a, b"),
        ("a \t / \t b", "a, b"),
        ("/ /h ii/ !  /", ", /h ii/ !,"),
        ("/h ii/ ! / /h ii/ !", "/h ii/ !, /h ii/ !"),
        ("a /b c", "a, b c"),
    ] {
        assert_eq!(sung(slash, ""), sung(comma, ""), "{slash:?} against {comma:?}");
    }
    // `a/b` has the slash glued on both sides: it opens an escape on `b`, which is never closed
    assert_eq!(sung("a/b c", ""), sung("a, b c", ""));
    // a lyric of one lone slash has no words to sing, and says that
    let src = "deck B: melody c4 e4\ndeck B: sing \"/\" on melody voice=robot\n";
    assert_eq!(errors_with(src, &absent())[0].2, "there is nothing to sing: the lyric has no words");
}
