//! `jux`, a mutation operator that keeps the pattern as the left copy and plays a mutated copy as the right one (mutate.rs,
//! docs/BEATSCRIPT.md "Arrange mutations", "Stereo"). `~ lead jux rev`, `~ lead rot 1 | jux rev | up 2`.

use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic, Severity};
use djlive::mutate::{fix, generations, GenTable};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}\n{}", src, render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn diags(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics
}

fn table(src: &str) -> GenTable {
    generations(src, &djlive::env::Env::none()).expect("a generation table")
}

/// The pattern `p` after one line `~ p CHAIN`: (the left copy, the right copy if it has one).
fn one(pattern: &str, role: &str, chain: &str) -> (String, Option<String>) {
    let src = format!("key am\nlet p = {pattern}\nlet q = x ~ ~ x x ~ x ~\nsection s 1 bars {{\n  deck A: {role} p\n  deck B: harmony p | kick q\n}}\narrange {{\n  ~ p {chain}\n  s\n}}\n");
    let g = table(&src).patterns["p"].last().unwrap().clone();
    (g.text, g.right)
}

const MEL: &str = "a3 c4 ~ e4 g4 ~ a4 ~";

/// A one-section program: the melody `p` on deck A, other patterns and a drum part for the operators that read them.
fn prog(role: &str, chain: &str) -> String {
    format!("key am\nlet p = a3 c4 ~ e4 g4 ~ a4 ~\nlet q = x ~ ~ x x ~ x ~\nsection s 1 bars {{\n  deck A: {role} p\n  deck B: kick q\n}}\narrange {{\n  ~ p {chain}\n  s\n}}\n")
}

#[test]
fn jux_is_an_operator_that_takes_an_operator() {
    for chain in ["jux rev", "jux rot 3", "jux thin 0.4", "rot 1 | jux rev | up 2", "jux morph q 0.5"] {
        let src = prog("melody", chain);
        let c = ok(&src);
        // the canonical text says it as written, and is a fixed point with the same CID
        assert!(c.text.contains(&format!("  ~ p {chain}\n")), "{chain}: {}", c.text);
        assert_eq!(ok(&c.text).cid, c.cid, "{chain}");
    }
    assert_eq!(djlive::mutate::operator_args("jux"), Some(vec!["operator"]));
    assert!(djlive::mutate::operator_names().contains(&"jux"));
}

#[test]
fn jux_errors_say_what_it_takes() {
    // bare, or followed by nothing an operator can be
    let d = diags(&prog("melody", "jux"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`jux` takes the operator the right copy gets")), "{d:?}");
    let d = diags(&prog("melody", "jux 3"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`jux` takes the operator the right copy gets")), "{d:?}");
    // its operator is any operator but another `jux`
    let d = diags(&prog("melody", "jux jux rev"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("cannot be another `jux`")), "{d:?}");
    // an unknown operator, and the arguments, are checked as they are anywhere
    let d = diags(&prog("melody", "jux wobble"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`wobble` is not a mutation operator")), "{d:?}");
    let d = diags(&prog("melody", "jux rot"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`rot` takes 1 argument")), "{d:?}");
    // one `jux` to a chain
    let d = diags(&prog("melody", "jux rev | up 2 | jux rot 1"));
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("a chain has one `jux`")), "{d:?}");
}

#[test]
fn the_fixer_puts_a_bad_inner_operator_out_of_action_as_id() {
    let src = prog("melody", "jux wobble 3");
    assert!(!check(&src).ok());
    let fixed = fix(&src).expect("a fix");
    assert!(fixed.contains("~ p jux id  # fixed: was `wobble 3`\n"), "{fixed}");
    ok(&fixed);
    assert!(fix(&fixed).is_none());
    // a second `jux` in a chain goes whole
    let two = prog("melody", "jux rev | jux rot 1");
    let fixed = fix(&two).expect("a fix");
    assert!(fixed.contains("~ p jux rev | id  # fixed: was `jux rot 1`\n"), "{fixed}");
    ok(&fixed);
}

#[test]
fn a_pitch_operator_under_jux_on_a_drum_pattern_gives_it_no_right_copy() {
    let src = "let k = x ~ x ~\nsection s 1 bars {\n  deck A: kick k\n}\narrange {\n  ~ k jux up 2\n  s\n}\n";
    let d = diags(src);
    assert!(d.iter().any(|d| d.severity == Severity::Warning && d.message.contains("`up` moves pitches, and `k` is a drum pattern: `jux` gives it no right copy")), "{d:?}");
    assert!(check(src).ok());
}

#[test]
fn the_left_copy_stays_and_the_right_one_is_the_operator_applied() {
    // `jux OP`: the pattern is the left copy; the right copy is the pattern with OP done to it
    for (op, right) in [
        ("rev", "~ a4 ~ g4 e4 ~ c4 a3"),
        ("rot 3", "~ a4 ~ a3 c4 ~ e4 g4"),
        ("up 2", "c4 e4 ~ g4 b4 ~ c5 ~"),
        ("half", "a3 c4 ~ e4"),
    ] {
        let (left, r) = one(MEL, "melody", &format!("jux {op}"));
        assert_eq!(left, MEL, "jux {op}: the left copy is the pattern as it was");
        assert_eq!(r.as_deref(), Some(right), "jux {op}");
        // and the right copy is what the operator alone makes of the pattern
        let (alone, none) = one(MEL, "melody", op);
        assert_eq!((alone.as_str(), none), (right, None), "{op} alone");
    }
    // an operator that reads another pattern reads it the same way: the right copy is some of the pattern and some of `q`
    let (left, right) = one(MEL, "melody", "jux morph q 0.5");
    assert_eq!(left, MEL);
    let (mel, q): (Vec<&str>, Vec<&str>) = (MEL.split(' ').collect(), "x ~ ~ x x ~ x ~".split(' ').collect());
    let right = right.unwrap();
    let right: Vec<&str> = right.split(' ').collect();
    assert!(right.iter().enumerate().all(|(i, t)| *t == mel[i] || *t == q[i]), "{right:?}");
    assert!(right.iter().enumerate().any(|(i, t)| *t == q[i] && *t != mel[i]), "{right:?}");
}

#[test]
fn operators_before_jux_work_on_the_pattern_and_after_it_on_the_right_copy() {
    // before: both copies; after: the right copy only
    assert_eq!(one(MEL, "melody", "rot 1 | jux rev"), ("~ a3 c4 ~ e4 g4 ~ a4".into(), Some("a4 ~ g4 e4 ~ c4 a3 ~".into())));
    assert_eq!(one(MEL, "melody", "jux rev | up 2"), (MEL.into(), Some("~ c5 ~ b4 g4 ~ e4 c4".into())));
    assert_eq!(one(MEL, "melody", "rot 1 | jux rev | up 2"), ("~ a3 c4 ~ e4 g4 ~ a4".into(), Some("c5 ~ b4 g4 ~ e4 c4 ~".into())));
    // the left copy is what the chain without its `jux` and what follows makes of the pattern
    let (alone, _) = one(MEL, "melody", "rot 1");
    assert_eq!(one(MEL, "melody", "rot 1 | jux rev | up 2").0, alone);
}

#[test]
fn the_right_copy_has_dice_of_its_own() {
    // `thin` is seeded (by the program's CID, so by the text, `jux` and all): the left copy of
    // `thin 0.5 | jux rev | thin 0.5` is some of the pattern's hits and the right copy some of those, reversed
    let (left, right) = one(MEL, "melody", "thin 0.5 | jux rev | thin 0.5");
    let right = right.unwrap();
    let terms = |t: &str| t.split(' ').map(str::to_string).collect::<Vec<_>>();
    for (l, m) in terms(&left).iter().zip(terms(MEL)) {
        assert!(l == "~" || *l == m, "{left} against {MEL}");
    }
    let rev: Vec<String> = terms(&left).into_iter().rev().collect();
    for (r, l) in terms(&right).iter().zip(&rev) {
        assert!(r == "~" || r == l, "{right} against {rev:?}");
    }
    // pinned: the dice of the right copy, so a change in how they are drawn shows here
    assert_eq!(one(MEL, "melody", "jux thin 0.5").1.as_deref(), Some("~ c4 ~ e4 ~ ~ ~ ~"));
    assert_eq!(one(MEL, "melody", "jux shuffle").1.as_deref(), Some("a3 a4 ~ ~ g4 ~ e4 c4"));
}

#[test]
fn a_later_line_works_on_both_copies_and_a_later_jux_forks_again() {
    let src = "key am\nlet p = a3 c4 ~ e4 g4 ~ a4 ~\nsection s 1 bars {\n  deck A: melody p\n}\narrange {\n  s\n  ~ p jux rev\n  s\n  ~ p up 2\n  s\n  ~ p rot 1 | jux half\n  s\n}\n";
    let g = &table(src).patterns["p"];
    let text = |i: usize| (g[i].text.as_str(), g[i].right.as_deref());
    assert_eq!(text(0), (MEL, None));
    assert_eq!(text(1), (MEL, Some("~ a4 ~ g4 e4 ~ c4 a3")));
    // a plain line after: both copies move
    assert_eq!(text(2), ("c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")));
    // a line with its own `jux`: the right copy is made again, from the new left copy
    assert_eq!(text(3), ("~ c4 e4 ~ g4 b4 ~ c5", Some("~ c4 e4 ~")));
}

#[test]
fn a_one_shot_jux_is_a_pair_for_the_next_section_only() {
    let src = "key am\nlet p = a3 c4 ~ e4 g4 ~ a4 ~\nsection s 1 bars {\n  deck A: melody p\n}\narrange {\n  s\n  @ p jux rev\n  s\n  s\n}\n";
    let t = table(src);
    let read = |i: usize| t.plays[i].reads.iter().find(|r| r.pattern == "p").map(|r| (r.once, r.text.clone(), r.right.clone()));
    assert_eq!(read(0), None);
    assert_eq!(read(1), Some((true, MEL.to_string(), Some("~ a4 ~ g4 e4 ~ c4 a3".to_string()))));
    assert_eq!(read(2), None, "it reverts");
    let c = ok(src);
    let twin = |i: usize| c.segments[i].decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "melody").unwrap().twin.is_some();
    assert_eq!((twin(0), twin(1), twin(2)), (false, true, false));
}

#[test]
fn a_pitch_operator_under_jux_leaves_a_drum_pattern_one_copy() {
    let src = "let k = x ~ x ~\nsection s 1 bars {\n  deck A: kick k\n}\narrange {\n  ~ k jux up 2 | rot 1\n  s\n}\n";
    let g = table(src).patterns["k"].last().unwrap().clone();
    // no right copy; what follows the skipped `jux` is the pattern's own (the left copy turns)
    assert_eq!((g.text.as_str(), g.right), ("~ x ~ x", None));
    let c = ok(src);
    assert!(c.segments.iter().all(|s| s.decks[&djlive::ast::Deck::A].parts.iter().all(|p| p.twin.is_none())), "no twin, no doubled voices");
}

// ---- the pair through the lowering, the compiler and the envelope ---------------------------------------------------

use djlive::ast::Deck;

const JUX_SONG: &str = "key am
let p = a3 c4 ~ e4 g4 ~ a4 ~
let kik = x ~ x ~
section s 2 bars {
  deck A: kick kik | melody p
}
arrange {
  s
  ~ p jux rev
  s
  ~ p up 2
  s
} loop 3 then end
";

fn part<'a>(c: &'a Compiled, seg: usize, role: &str) -> &'a djlive::compile::PartIr {
    c.segments[seg].decks[&Deck::A].parts.iter().find(|p| p.role == role).unwrap()
}

#[test]
fn both_copies_generations_are_pinned_over_a_loop() {
    let t = table(JUX_SONG);
    // the pattern's generations, both copies: jux on the first pass, `up 2` on both copies, jux again from the new left copy
    let gens: Vec<(&str, &str, Option<&str>)> = t.patterns["p"].iter().map(|g| (g.ops.as_str(), g.text.as_str(), g.right.as_deref())).collect();
    assert_eq!(
        gens,
        [
            ("", "a3 c4 ~ e4 g4 ~ a4 ~", None),
            ("jux rev", "a3 c4 ~ e4 g4 ~ a4 ~", Some("~ a4 ~ g4 e4 ~ c4 a3")),
            ("up 2", "c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")),
            ("jux rev", "c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")),
            ("up 2", "e4 g4 ~ b4 d5 ~ e5 ~", Some("~ e5 ~ d5 b4 ~ g4 e4")),
            ("jux rev", "e4 g4 ~ b4 d5 ~ e5 ~", Some("~ e5 ~ d5 b4 ~ g4 e4")),
            ("up 2", "g4 b4 ~ d5 f5 ~ g5 ~", Some("~ g5 ~ f5 d5 ~ b4 g4")),
        ]
    );
    // what each play read: the generation, and both copies' text
    let plays: Vec<(String, u32, u32, String, Option<String>)> = t.plays.iter().map(|p| (p.name.clone(), p.pass, p.reads[0].generation, p.reads[0].text.clone(), p.reads[0].right.clone())).collect();
    let want = |n: &str, pass: u32, g: u32, l: &str, r: Option<&str>| (n.to_string(), pass, g, l.to_string(), r.map(str::to_string));
    assert_eq!(
        plays,
        [
            want("s", 0, 0, "a3 c4 ~ e4 g4 ~ a4 ~", None),
            want("s@2", 0, 1, "a3 c4 ~ e4 g4 ~ a4 ~", Some("~ a4 ~ g4 e4 ~ c4 a3")),
            want("s@3", 0, 2, "c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")),
            want("s@3", 1, 2, "c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")),
            want("s@4", 1, 3, "c4 e4 ~ g4 b4 ~ c5 ~", Some("~ c5 ~ b4 g4 ~ e4 c4")),
            want("s@5", 1, 4, "e4 g4 ~ b4 d5 ~ e5 ~", Some("~ e5 ~ d5 b4 ~ g4 e4")),
            want("s@5", 2, 4, "e4 g4 ~ b4 d5 ~ e5 ~", Some("~ e5 ~ d5 b4 ~ g4 e4")),
            want("s@6", 2, 5, "e4 g4 ~ b4 d5 ~ e5 ~", Some("~ e5 ~ d5 b4 ~ g4 e4")),
            want("s@7", 2, 6, "g4 b4 ~ d5 f5 ~ g5 ~", Some("~ g5 ~ f5 d5 ~ b4 g4")),
        ]
    );
    // the lowered program: a twin on the melody of every play that reads a pair, and none before the first jux
    let c = ok(JUX_SONG);
    let twins: Vec<bool> = (0..c.segments.len()).map(|i| part(&c, i, "melody").twin.is_some()).collect();
    assert_eq!(twins, [false, true, true, true, true, true, true, true, true]);
    // the kick has none, ever: it does not read the pair
    assert!((0..c.segments.len()).all(|i| part(&c, i, "kick").twin.is_none()));
    // the twin plays the right copy: the notes of its text, on the same part
    let notes = |p: &djlive::compile::PartIr| p.cycles[0].iter().map(|h| (h.step, h.note)).collect::<Vec<_>>();
    let m = part(&c, 1, "melody");
    assert_eq!(notes(m), [(0, 57), (2, 60), (6, 64), (8, 67), (12, 69)]);
    assert_eq!(notes(m.twin.as_ref().unwrap()), [(2, 69), (6, 67), (8, 64), (12, 60), (14, 57)]);
    // and the lowering is not in the text: no `_r`, no `.R`, no `@`
    assert!(!c.text.contains("_r") && !c.text.contains(".R") && !c.text.contains('@'), "{}", c.text);
    assert!(c.text.contains("  ~ p jux rev\n") && c.text.contains("  ~ p up 2\n"), "{}", c.text);
    assert_eq!(ok(&c.text).cid, c.cid);
}

#[test]
fn the_envelope_has_the_part_hard_left_and_its_twin_hard_right() {
    let c = ok(JUX_SONG);
    let env = |i: usize| c.segments[i].decks[&Deck::A].envelope(0);
    let nets = |e: &djengine::json::JsonValue| e.get("nets").unwrap().as_object().unwrap().keys().map(str::to_string).collect::<Vec<_>>();
    // before the jux: the nets it always had; after: the part's net with a pan, and the twin's
    assert_eq!(nets(&env(0)), ["kick", "melody"]);
    assert_eq!(nets(&env(1)), ["kick", "melody", "melody_r"]);
    let e = env(1);
    let track = |id: &str| e.get("nets").unwrap().get(id).unwrap().get("track").unwrap().clone();
    let pan = |id: &str| track(id).get("pan").and_then(|v| v.as_f64());
    assert_eq!((pan("melody"), pan("melody_r"), pan("kick")), (Some(-1.0), Some(1.0), None));
    for id in ["melody", "melody_r"] {
        assert_eq!(track(id).get("channel").and_then(|v| v.as_i64()), Some(4), "{id}: the melody channel");
        assert_eq!(track(id).get("instrument").and_then(|v| v.as_str()), Some("square-lead"), "{id}");
    }
    // the twin's net plays the right copy's notes
    let first_notes = |id: &str| -> Vec<i64> {
        let n = e.get("nets").unwrap().get(id).unwrap();
        n.get("transitions").unwrap().as_object().unwrap().iter().filter_map(|(_, t)| t.get("midi")?.get("note")?.as_i64()).collect()
    };
    assert_eq!(first_notes("melody"), [57, 60, 64, 67, 69]);
    assert_eq!(first_notes("melody_r"), [69, 67, 64, 60, 57]);
    // the song as one net (the longhand a hot reload and `check`'s nets read): the twin is silent where the part has none
    let song = c.song_nets().unwrap();
    let flat = song[&Deck::A].envelope(0);
    assert!(nets(&flat).contains(&"melody_r".to_string()));
    // the share format takes the twin's net id
    assert!("melody_r".chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'));
}

#[test]
fn a_deck_without_a_jux_has_the_envelope_it_always_had() {
    // the same program with the jux put out of action as `id`: every section's envelope is the first section's, with
    // no pan and no twin anywhere in it; the jux program's first section is the same bytes
    let plain = ok(&JUX_SONG.replace("jux rev", "id"));
    let jux = ok(JUX_SONG);
    let bytes = |c: &Compiled, i: usize| c.segments[i].decks[&Deck::A].envelope(0).canonical_js();
    assert!(!bytes(&plain, 0).contains("pan") && !bytes(&plain, 0).contains("_r"));
    assert_eq!(bytes(&plain, 1), bytes(&plain, 0), "`id` changes nothing");
    assert_eq!(bytes(&jux, 0), bytes(&plain, 0), "before the jux: the same bytes");
    assert_ne!(bytes(&jux, 1), bytes(&plain, 1), "after it: the twin");
    assert_ne!(jux.segments[1].decks[&Deck::A].net_cid(), plain.segments[1].decks[&Deck::A].net_cid());
    // and the part of a deck that reads no pair is a part as it was, in a deck that has a pair
    assert_eq!(part(&jux, 1, "kick").fingerprint(), part(&plain, 1, "kick").fingerprint());
    assert_ne!(part(&jux, 1, "melody").fingerprint(), part(&plain, 1, "melody").fingerprint(), "a hot reload that only adds the twin must reload");
}

#[test]
fn a_chord_part_gets_a_chord_twin() {
    // a chord spills into sibling nets (`harmony-b`, ...): the twin's spill is `harmony_r-b`, panned with it
    let src = "key am\nlet ch = [a3,c4,e4] ~ [g3,b3,d4] ~\nsection s 1 bars {\n  deck A: harmony ch\n}\narrange {\n  ~ ch jux voice 1\n  s\n}\n";
    let c = ok(src);
    let e = c.segments[0].decks[&Deck::A].envelope(0);
    let ids: Vec<String> = e.get("nets").unwrap().as_object().unwrap().keys().map(str::to_string).collect();
    assert_eq!(ids, ["harmony", "harmony-b", "harmony-c", "harmony_r", "harmony_r-b", "harmony_r-c"]);
    let pan = |id: &str| e.get("nets").unwrap().get(id).unwrap().get("track").unwrap().get("pan").and_then(|v| v.as_f64());
    assert_eq!([pan("harmony"), pan("harmony-c"), pan("harmony_r"), pan("harmony_r-c")], [Some(-1.0), Some(-1.0), Some(1.0), Some(1.0)]);
}

#[test]
fn a_part_that_reads_two_patterns_has_a_twin_that_reads_both_right_copies() {
    let src = "key am\nlet ma = a3 c4 ~ e4\nlet mb = g3 ~ b3 d4\nsection s 1 bars {\n  deck A: melody <ma mb>\n  deck B: bass ma\n}\narrange {\n  ~ ma jux rev\n  s\n}\n";
    let c = ok(src);
    let m = c.segments[0].decks[&Deck::A].parts.iter().find(|p| p.role == "melody").unwrap();
    assert!(m.twin.is_some());
    let bass = c.segments[0].decks[&Deck::B].parts.iter().find(|p| p.role == "bass").unwrap();
    assert!(bass.twin.is_some(), "every part that reads the pair gets a twin, on either deck");
    // the alternation: bar 0 plays `ma` (the twin: its reverse), bar 1 plays `mb` (the twin: `mb` as it is, not a pair)
    let twin = m.twin.as_ref().unwrap();
    assert_eq!(twin.cycles.len(), m.cycles.len());
    assert_ne!(twin.cycles[0], m.cycles[0]);
    assert_eq!(twin.cycles[1], m.cycles[1]);
}

#[test]
fn an_evolve_moves_both_copies() {
    let src = "key am\nlet m = a4 ~ ~ ~ c5 ~ ~ ~\nsection drop 8 bars {\n  deck B: melody m\n  evolve every 4 bars {\n    melody: rotate 1\n    stop when gen == 1\n  }\n}\narrange {\n  ~ m jux rev\n  drop\n}\n";
    let c = ok(src);
    assert_eq!(c.segments.len(), 2, "two generations");
    let both = |i: usize| {
        let p = c.segments[i].decks[&Deck::B].parts.iter().find(|p| p.role == "melody").unwrap();
        let notes = |q: &djlive::compile::PartIr| q.cycles[0].iter().map(|h| (h.step, h.note)).collect::<Vec<_>>();
        (notes(p), notes(p.twin.as_ref().unwrap()))
    };
    let (l0, r0) = both(0);
    let (l1, r1) = both(1);
    assert_eq!((l0.clone(), r0.clone()), (vec![(0, 69), (8, 72)], vec![(6, 72), (14, 69)]));
    // generation 1: every hit of both copies one step on
    assert_eq!(l1, l0.iter().map(|(s, n)| (s + 1, *n)).collect::<Vec<_>>());
    assert_eq!(r1, r0.iter().map(|(s, n)| (s + 1, *n)).collect::<Vec<_>>());
}

#[test]
fn check_says_where_jux_cannot_split_or_leaves_the_middle() {
    let warns = |src: &str| -> Vec<String> { diags(src).into_iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message).collect() };
    // a kick and a bass keep the middle
    let k = "let k = x ~ x ~\nsection s 1 bars {\n  deck A: kick k\n}\narrange {\n  ~ k jux rot 1\n  s\n}\n";
    assert!(warns(k).iter().any(|w| w.contains("`jux` on `k`, which a kick plays: the low end leaves the middle")), "{:?}", warns(k));
    let b = "key am\nlet b = a2 ~ a2 ~\nsection s 1 bars {\n  deck A: bass b\n}\narrange {\n  ~ b jux rev\n  s\n}\n";
    assert!(warns(b).iter().any(|w| w.contains("which a bass plays")), "{:?}", warns(b));
    // a melody does not
    assert!(!warns(&prog("melody", "jux rev")).iter().any(|w| w.contains("low end")));
    // a reader outside a section's lines plays the left copy only: a line of the song, a repeat, a def
    let outside = "key am\nlet p = a3 c4 ~ e4\ndeck A: melody p\nsection s 1 bars {\n  deck B: arp p\n}\narrange {\n  ~ p jux rev\n  s\n}\n";
    assert!(warns(outside).iter().any(|w| w.contains("`p` is also read in a line outside every section, where `jux` cannot split it")), "{:?}", warns(outside));
    let repeated = "key am\nlet p = a3 c4 ~ e4\nsection s 4 bars {\n  deck B: arp p\n  repeat 2 {\n    deck A: melody p\n  }\n}\narrange {\n  ~ p jux rev\n  s\n}\n";
    assert!(warns(repeated).iter().any(|w| w.contains("`p` is also read in a `repeat`")), "{:?}", warns(repeated));
    // nothing to say about a plain one
    assert!(warns(&prog("melody", "jux rev")).is_empty(), "{:?}", warns(&prog("melody", "jux rev")));
}

#[test]
fn check_says_what_a_sampled_voice_and_a_crowded_deck_and_a_sung_line_do() {
    let warns = |src: &str| -> Vec<String> { diags(src).into_iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message).collect() };
    // a sampled sound plays in the middle: both copies are heard there
    let sampled = "key am\nlet p = a3 c4 ~ e4\nsection s 1 bars {\n  deck A: melody p sound=cat:piano\n}\narrange {\n  ~ p jux rev\n  s\n}\n";
    assert!(warns(sampled).iter().any(|w| w.contains("`jux` on `melody` with `sound=cat:piano`: a sampled voice plays in the middle")), "{:?}", warns(sampled));
    let synth = sampled.replace("sound=cat:piano", "sound=synth");
    assert!(!warns(&synth).iter().any(|w| w.contains("sampled")), "{:?}", warns(&synth));
    // more than four twinned parts in a deck
    let crowded = "key am\nlet p = a3 c4 ~ e4\nsection s 1 bars {\n  deck A: bass p | melody p | harmony p | arp p | kick p | snare p\n}\narrange {\n  ~ p jux rev\n  s\n}\n";
    assert!(warns(crowded).iter().any(|w| w.contains("deck A has 6 parts with a right copy (`jux`): each plays a second set of voices")), "{:?}", warns(crowded));
    let four = crowded.replace(" | kick p | snare p", "");
    assert!(!warns(&four).iter().any(|w| w.contains("right copy")), "{:?}", warns(&four));
}

#[test]
fn a_sung_line_follows_the_left_copy_and_says_so() {
    use djlive::compile::check_with;
    use djlive::env::{Env, StaticVoices};
    let src = "key am\nlet p = a3 c4 ~ e4 g4 ~ a4 ~\nsection s 2 bars {\n  deck A: melody p\n  sing \"hush little lantern\" on melody\n}\narrange {\n  s\n  ~ p jux rev\n  s\n}\n";
    let v = StaticVoices::default();
    let c = check_with(src, &Env { voices: Some(&v), ..Env::default() });
    assert!(c.ok(), "{}", render_all(&c.diagnostics, src));
    let says = |d: &Diagnostic| d.severity == Severity::Warning && d.message.contains("`melody` has a right copy (`jux`): the lyric is sung on the left copy's notes only");
    assert_eq!(c.diagnostics.iter().filter(|d| says(d)).count(), 1, "once, for the play that has the copy: {:?}", c.diagnostics);
    // a program with no jux does not say it
    let plain = check_with(&src.replace("~ p jux rev", "~ p rot 1"), &Env { voices: Some(&v), ..Env::default() });
    assert!(!plain.diagnostics.iter().any(says));
}

/// `~3 syn_arp jux rev` (6641 with 6640): a counted `jux` makes the right copy only on the passes it fires on, and the
/// twin plays from the play that first reads the firing.
#[test]
fn a_counted_jux_forks_on_its_passes_only() {
    let src = "tempo 120\nkey am\nlet syn_arp = a3 c4 e4 g4 a4 g4 e4 c4\nlet drm_kick = x ~ x ~\nsection jam 2 bars {\n  deck A: kick drm_kick\n  deck B: arp syn_arp\n}\narrange {\n  jam\n  ~3 syn_arp jux rev\n} loop 6 then end\n";
    let c = ok(src);
    assert!(diags(src).iter().all(|d| d.severity != Severity::Warning), "{:?}", diags(src));
    assert!(c.text.contains("  ~3 syn_arp jux rev\n"), "{}", c.text);
    assert_eq!(ok(&c.text).cid, c.cid);
    let t = table(src);
    // generation 0, then a firing on the 3rd and on the 6th pass (0-based 2 and 5): the left copy is what it was, the right one is its reverse
    let gens: Vec<(Option<u32>, &str, Option<&str>, u32)> = t.patterns["syn_arp"].iter().map(|g| (g.pass, g.text.as_str(), g.right.as_deref(), g.every)).collect();
    assert_eq!(
        gens,
        [
            (None, "a3 c4 e4 g4 a4 g4 e4 c4", None, 1),
            (Some(2), "a3 c4 e4 g4 a4 g4 e4 c4", Some("c4 e4 g4 a4 g4 e4 c4 a3"), 3),
            (Some(5), "a3 c4 e4 g4 a4 g4 e4 c4", Some("c4 e4 g4 a4 g4 e4 c4 a3"), 3),
        ]
    );
    assert_eq!(t.fires.len(), 1);
    assert_eq!((t.fires[0].every, t.fires[0].passes.clone()), (3, vec![2, 5]));
    // the plays before the first firing read one copy; the first play after it reads the pair
    let reads: Vec<(u32, Option<String>)> = t.plays.iter().map(|p| (p.pass, p.reads.first().and_then(|r| r.right.clone()))).collect();
    assert!(reads.iter().filter(|(pass, _)| *pass <= 2).all(|(_, r)| r.is_none()), "{reads:?}");
    assert!(reads.iter().any(|(pass, r)| *pass == 3 && r.is_some()), "{reads:?}");
    // lowered: a twin on the arp of the plays that read the pair, none on the kick
    let on = |i: usize, d: Deck, role: &str| c.segments[i].decks[&d].parts.iter().find(|p| p.role == role).unwrap().twin.is_some();
    let twins: Vec<bool> = (0..c.segments.len()).map(|i| on(i, Deck::B, "arp")).collect();
    assert!(twins.iter().any(|t| *t) && !twins[0], "{twins:?}");
    assert!((0..c.segments.len()).all(|i| !on(i, Deck::A, "kick")));
}
