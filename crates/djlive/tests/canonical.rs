//! The canonical form (docs/BEATSCRIPT.md, "Canonical form"): formatting never forks a program. Tonight's
//! three programs, reformatted at random (spacing, tabs, blank lines, comments, indentation, `|`
//! continuation lines, number and note spelling), keep their CID; one note changed does not. The
//! lets/defs/sections examples too. And a link opens only if its CID is its text's.

use djlive::compile::check;
mod common;

use common::{reformat, Rng};
use djlive::lex::{lex, Tok};
use djlive::music;
use djlive::share;

const FIXTURES: [(&str, &str); 3] = [
    ("hand-dubstep-140", include_str!("fixtures/hand-dubstep-140.beatscript")),
    ("hand-proghouse-124", include_str!("fixtures/hand-proghouse-124.beatscript")),
    ("acid-rave-138", include_str!("fixtures/acid-rave-138.beatscript")),
];

const EXAMPLES: [&str; 6] = [
    include_str!("../../../docs/beatscript/hand-dubstep-140-v2.beatscript"),
    "let late = 8\nlet riff = c2 ~ [e2 g2] ~\nat bar late: master: level 0.5\ndeck A: bass riff | kick x ~ x ~\n",
    "def drop(deck, bars) {\n  deck $deck: kick x ~ x ~ | bass transpose([c2 e2 g2 b2], 5) vel=70..100 oct=2\n  at bar 1: master: filter lp 2k -> 8k over $bars bars\n}\ndrop(A, 8)\n",
    "repeat 3 every 2 bars {\n  master: delay 0.5\n}\n",
    "section intro 8 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 8 bars {\n  deck A: kick x x x x | bass reverse([c2 e2 g2 b2]) inst=wobble-lead\n  at bar 4 for 2 bars: master: filter lp 800\n}\narrange intro drop\n",
    "tempo 140\nkey Bbm\ndeck A: bass c2! e2^ g2 ~ route=td3 vel=70..100\ndo vocoder set mix=0 -> 1 over 8 bars\n",
];

#[test]
fn tonights_programs_keep_their_cid_through_random_reformatting() {
    for (name, src) in FIXTURES {
        let want = check(src).compiled.unwrap_or_else(|| panic!("{name} does not compile")).cid;
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for round in 0..150 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("{name} round {round} no longer compiles:\n{text}\n{:?}", c.diagnostics)).cid;
            assert_eq!(got, want, "{name} round {round} forked:\n{text}");
        }
    }
}

#[test]
fn the_lets_defs_and_sections_examples_keep_their_cid_too() {
    for (n, src) in EXAMPLES.iter().enumerate() {
        let first = check(src);
        let want = first.compiled.unwrap_or_else(|| panic!("example {n} does not compile: {:?}", first.diagnostics)).cid;
        let mut rng = Rng(0x1234_5678_9abc_def1 + n as u64);
        for round in 0..150 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("example {n} round {round} no longer compiles:\n{text}\n{:?}", c.diagnostics)).cid;
            assert_eq!(got, want, "example {n} round {round} forked:\n{text}");
        }
    }
}

#[test]
fn the_canonical_text_is_a_fixed_point_and_has_one_spelling_for_everything() {
    for (name, src) in FIXTURES {
        let c = check(src).compiled.unwrap();
        let again = check(&c.text).compiled.unwrap();
        assert_eq!(again.text, c.text, "{name}: canonical text is not a fixed point");
        assert_eq!(again.cid, c.cid);
        // flats are sharps, notes lower case
        assert!(!c.text.contains("ab1") || name != "hand-dubstep-140", "{}", c.text);
    }
    let a = check("key Bbm\ndeck A: bass Bb2 18k\n").compiled;
    let _ = a;
    let a = check("tempo 120.0\nkey Bbm\ndeck A: bass Bb2 eb2 | hihat x*8 vel=100..70\nmaster: filter lp 18k\n");
    // (a descending range is an error: what matters below is the spelling)
    assert!(!a.ok());
    let one = check("key Bbm\ndeck A: bass Bb2 Eb2 oct=2 vel=70..100\nmaster: filter lp 18k -> 0.50k over 4 bars\n");
    let two = check("key a#m\ndeck A: bass a#2 d#2 vel=70..100 oct=2\nmaster: filter lp 18000 -> 500 over 4 bars\n");
    let (one, two) = (one.compiled.expect("one"), two.compiled.expect("two"));
    assert_eq!(one.text, two.text);
    assert_eq!(one.cid, two.cid);
    assert!(one.text.contains("key a#m") && one.text.contains("bass a#2 d#2 oct=2 vel=70..100"), "{}", one.text);
    // param order inside a do line
    let a = check("do vocoder set mix=0.5 bands=16\n").compiled.unwrap();
    let b = check("do vocoder set bands=16 mix=0.50\n").compiled.unwrap();
    assert_eq!(a.cid, b.cid);
}

#[test]
fn comments_are_not_in_the_cid_but_travel_with_the_text() {
    let plain = check("deck A: kick x ~ x ~\n").compiled.unwrap();
    let said = check("# four on the floor\ndeck A: kick x ~ x ~   # kick\n").compiled.unwrap();
    assert_eq!(plain.cid, said.cid);
    assert_eq!(said.notes, vec!["four on the floor".to_string(), "kick".to_string()]);
    assert!(plain.notes.is_empty());
}

#[test]
fn one_note_changed_is_another_program() {
    for (name, src) in FIXTURES {
        let want = check(src).compiled.unwrap().cid;
        let lexed = lex(src);
        let notes: Vec<_> = lexed.tokens.iter().filter(|t| matches!(&t.tok, Tok::Word(w) if music::parse_note(w).is_some_and(|(_, o)| o.is_some()))).collect();
        assert!(notes.len() > 8, "{name}");
        let mut rng = Rng(7);
        for _ in 0..40 {
            let t = notes[rng.below(notes.len() as u64) as usize];
            let w = t.span.text(src);
            let (pc, oct) = music::parse_note(w).unwrap();
            let changed = format!("{}{}", ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"][((pc + 1) % 12) as usize], oct.unwrap());
            let mut text = src.to_string();
            text.replace_range(t.span.start..t.span.end, &changed);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("{name}: {:?}", c.diagnostics)).cid;
            assert_ne!(got, want, "{name}: changing {w} to {changed} left the CID alone");
        }
    }
}

#[test]
fn a_link_seals_its_text_and_refuses_a_forgery() {
    for (name, src) in FIXTURES {
        let c = check(src).compiled.unwrap();
        let url = share::encode(&c, None, false);
        let opened = share::open(&url, &djlive::env::Env::none()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(opened.cid, c.cid);
        assert!(opened.check.ok());
        assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
        // the same CID over a different text is refused
        let other = check("deck A: kick x x x x\n").compiled.unwrap();
        let other_url = share::encode(&other, None, false);
        let other_payload = other_url.split_once('#').unwrap().1.split_once('.').unwrap().1;
        let forged = format!("{}#{}.{}", share::BASE, c.cid, other_payload);
        let e = share::open(&forged, &djlive::env::Env::none()).err().expect("a forged link opens");
        assert!(e.contains("refused"), "{e}");
    }
}

#[test]
fn a_link_carries_comments_unsealed_and_a_signature_opaquely() {
    let c = check("# the drop\ndeck A: kick x ~ x ~\n").compiled.unwrap();
    let url = share::encode(&c, Some("sigABC"), true);
    assert!(url.contains(".sigABC~"), "{url}");
    let sealed_only = share::encode(&c, Some("sigABC"), false);
    let a = share::open(&url, &djlive::env::Env::none()).unwrap();
    let b = share::open(&sealed_only, &djlive::env::Env::none()).unwrap();
    assert_eq!(a.cid, b.cid, "notes are not covered by the CID");
    assert_eq!(a.link.notes, vec!["the drop".to_string()]);
    assert_eq!(a.link.sig.as_deref(), Some("sigABC"));
    assert!(a.warnings.iter().any(|w| w.contains("unsealed")));
    // a bare CID has no text
    let e = share::open(&c.cid, &djlive::env::Env::none()).err().unwrap();
    assert!(e.contains("storage"), "{e}");
}

#[test]
fn how_long_the_links_are() {
    for (name, src) in FIXTURES {
        let c = check(src).compiled.unwrap();
        let url = share::encode(&c, None, false);
        eprintln!("LINK {name}: canonical {} bytes, link {} chars", c.text.len(), url.len());
    }
}
