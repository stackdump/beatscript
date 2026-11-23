//! The public-domain phrasebook on a `sing` line (docs/beatscript/phonemes.md, "The public-domain phrasebook"): every line sings
//! on a part with no error and no warning, in the browser's reading of a lyric and in the desktop's; `sing @id` sings what
//! its lyric sings; a wrong id says what it meant among both phrasebooks; the stress is written in BeatScript's own
//! step notation. The notes the lines sing on are the test's: exactly as many as the line has syllables, on whole sixteenths.

use djengine::vocal::phoneme::{pd_phrase, pd_phrases, phrase_ids, PdPhrase};
use djlive::compile::{check_with, Check, Effect};
use djlive::diag::{render_all, Severity};
use djlive::env::{Env, StaticVoices};
use djlive::sing::{layout, syllables};

/// A part of `n` notes: one bar of sixteen sixteenths (the notes first, then rests) or, past sixteen, two. Whole
/// sixteenths, so nothing snaps and the only warnings are the line's own.
fn melody(n: usize) -> String {
    const NOTES: [&str; 8] = ["a3", "c4", "e4", "g4", "a4", "g4", "e4", "c4"];
    assert!((1..=32).contains(&n), "{n} notes");
    let bar = |from: usize, to: usize| -> String {
        let toks: Vec<&str> = (0..16).map(|i| if from + i < to { NOTES[(from + i) % 8] } else { "~" }).collect();
        format!("[{}]", toks.join(" "))
    };
    if n <= 16 {
        bar(0, n)
    } else {
        format!("<{} {}>", bar(0, 16), bar(16, n))
    }
}

/// The program a line is sung in, with `said` after `sing` (a quoted lyric, or `@id`).
fn program(said: &str, n: usize) -> String {
    format!("tempo 100\nkey am\ndeck A: melody {}\ndeck A: sing {said} on melody\n", melody(n))
}

fn quoted(p: &PdPhrase) -> String {
    format!("\"{}\"", p.lyric)
}

/// Check against a host's voices; every diagnostic, rendered.
fn diagnostics(src: &str, v: &StaticVoices) -> (Check, Vec<String>) {
    let env = Env { voices: Some(v), ..Env::default() };
    let c = check_with(src, &env);
    let all = c.diagnostics.iter().map(|d| format!("{}: {}", if d.severity == Severity::Warning { "warning" } else { "error" }, d.message)).collect();
    (c, all)
}

/// The desktop's voices: each plain word's syllables as the voice's own dictionary counts them.
fn desktop_voices(p: &PdPhrase) -> StaticVoices {
    let mut v = StaticVoices::default();
    for w in djengine::vocal::lexicon::words(&p.lyric) {
        if w.escaped.is_none() {
            v.syllables.insert(w.text.clone(), djengine::vocal::fit::syllables(&w.text).len());
        }
    }
    v
}

#[test]
fn every_line_sings_on_a_part_with_no_error_and_no_warning() {
    for p in pd_phrases() {
        let src = program(&quoted(p), p.syllables);
        // the browser's reading counts vowel groups of a plain word; the desktop's asks the voice: both must see the notes
        for (host, v) in [("the browser", StaticVoices::default()), ("the desktop", desktop_voices(p))] {
            let (c, all) = diagnostics(&src, &v);
            assert!(c.ok() && all.is_empty(), "{} on {host}: {all:?}\n{src}\n{}", p.id, render_all(&c.diagnostics, &src));
        }
        // the lyric lies on exactly its notes: once, none dropped
        let words = syllables(&p.lyric, None);
        let lay = layout(&words, p.syllables).expect("a layout");
        assert_eq!((lay.syllables, lay.passes, lay.dropped), (p.syllables, 1, 0), "{}: the check counts {} syllables, the file {}", p.id, lay.syllables, p.syllables);
        // and one note fewer leaves one syllable out, so the check above could have caught a wrong count
        let (_, short) = diagnostics(&program(&quoted(p), p.syllables - 1), &StaticVoices::default());
        assert!(short.iter().any(|m| m.contains("the last 1 is not sung")), "{}: {short:?}", p.id);
    }
}

#[test]
fn sing_at_id_sings_what_its_lyric_sings_and_prints_as_its_id() {
    for p in pd_phrases() {
        let by_id = program(&format!("@{}", p.id), p.syllables);
        let (c, all) = diagnostics(&by_id, &StaticVoices::default());
        assert!(c.ok() && all.is_empty(), "@{}: {all:?}", p.id);
        let compiled = c.compiled.unwrap();
        assert!(compiled.text.contains(&format!("sing @{} on melody", p.id)), "{}: {}", p.id, compiled.text);
        let (c2, _) = diagnostics(&program(&quoted(p), p.syllables), &StaticVoices::default());
        let c2 = c2.compiled.unwrap();
        // the same vocal line: the same words, notes and voice (the controls' keys name the line as written, so differ)
        let calls = |c: &djlive::compile::Compiled| match &c.controls[0].effect {
            Effect::Calls(cs) => cs.iter().map(|c| format!("{}:{}", c.noun, c.params.canonical_js())).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(calls(&compiled), calls(&c2), "{}", p.id);
    }
    // a stock phrase still sings
    let (c, all) = diagnostics(&program("@hands_up", 4), &StaticVoices::default());
    assert!(c.ok() && all.is_empty(), "{all:?}");
}

#[test]
fn a_wrong_id_says_what_it_meant_among_both_phrasebooks_and_where_the_lines_are() {
    let msg = |id: &str| {
        let (c, _) = diagnostics(&program(&format!("@{id}"), 8), &StaticVoices::default());
        assert!(!c.ok(), "@{id} is not a phrase");
        let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
        format!("{} / {}", d.message, d.help.clone().unwrap_or_default())
    };
    assert!(msg("tempest_such_stuf").contains("did you mean `@tempest_such_stuff`"), "{}", msg("tempest_such_stuf"));
    assert!(msg("gettysburg_four_scor").contains("did you mean `@gettysburg_four_score`"));
    // the stock phrases are still found
    assert!(msg("hands_upp").contains("did you mean `@hands_up`"));
    // nothing near: the help says there are lines of verse and speech, and names two
    let far = msg("zzzzzzzzzzzz");
    assert!(far.contains("is not a phrase of the phrasebook") && far.contains("@tempest_such_stuff") && far.contains("before 1929"), "{far}");
}

#[test]
fn the_stress_is_a_step_pattern_in_beatscripts_own_notation() {
    for p in pd_phrases() {
        // `deck A: hihat <stress>` reads (it may say a pattern of this many steps does not divide the bar: that is a warning,
        // the steps are for a template to lay on its own grid)
        let src = format!("deck A: hihat {}\n", p.stress);
        let c = check_with(&src, &Env::default());
        assert!(c.ok(), "{}: {}", p.id, render_all(&c.diagnostics, &src));
        // the same pattern, one bar, when its steps divide the bar: the hits are the stressed syllables
        if [4, 8, 16].contains(&p.syllables) {
            let part = &c.compiled.unwrap().decks[&djlive::ast::Deck::A].parts[0];
            let hits: Vec<u8> = part.cycles[0].iter().map(|h| h.step).collect();
            let want: Vec<u8> = p.accents().iter().enumerate().filter(|(_, a)| **a).map(|(i, _)| (i * 16 / p.syllables) as u8).collect();
            assert_eq!(hits, want, "{}", p.id);
        }
    }
    // a pair of iambs is `~ x ~ x`, a line of Blake's two trochaic lines `x ~ x ~ ...`
    assert_eq!(pd_phrase("sonnet18_summers_day").unwrap().stress, "~ x ~ x ~ x ~ x ~ x");
    assert!(pd_phrase("tyger_burning_bright").unwrap().stress.starts_with("x ~ x ~"));
}

#[test]
fn the_ids_of_both_phrasebooks_are_one_namespace() {
    let ids = phrase_ids();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids.len(), sorted.len(), "an id is in both phrasebooks, or twice");
    assert!(ids.len() >= 28 + 40);
}
