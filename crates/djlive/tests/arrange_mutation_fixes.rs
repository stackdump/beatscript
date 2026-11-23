//! Regressions from composing over the MCP (2026-11-23): `arrange { }` mutations that should just work where a user
//! writes them (mutate.rs), and the transforms an `evolve` takes (evolve.rs). One test per bug, each program written as
//! it was hit; where a mutation should change the music, the nets of the plays are compared.

use djlive::compile::{check, Compiled};
use djlive::diag::render_all;
use djlive::mutate::{generations, GenTable};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}\n{}", src, render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn messages(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().map(|d| d.message.clone()).collect()
}

fn table(src: &str) -> GenTable {
    generations(src, &djlive::env::Env::none()).expect("a generation table")
}

/// The last generation's text of a pattern.
fn last(src: &str, name: &str) -> String {
    table(src).patterns.get(name).map_or_else(|| "(unchanged)".to_string(), |g| g.last().unwrap().text.clone())
}

/// Each play's nets, deck by deck, as a short fingerprint (a failure prints these, not the nets).
fn nets(c: &Compiled) -> Vec<String> {
    use std::hash::{Hash, Hasher};
    c.segments
        .iter()
        .map(|s| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>().join("|").hash(&mut h);
            format!("{:016x}", h.finish())
        })
        .collect()
}

const HEAD: &str = "#! beatscript 2\ntempo 124\nkey am\nlet k = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n";

// ---- 1. a mutation of a let another let reads

#[test]
fn bug1_a_mutation_of_a_let_that_another_let_reads() {
    let lets = "let acid_a = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nlet acid_b = [1 ~ 1 3 ~ 5 ~ 1 4 ~ 1 ~ 7 ~ 5 1]\nlet line = <acid_a acid_a acid_b acid_a>\n";
    let sec = "section drop 4 bars {\n  deck A: kick k\n  deck B: bass line\n}\n";
    // as reported: it checks (what `drift` draws for this CID may leave the bars as they were: a `1` drifting down stays 1)
    ok(&format!("{HEAD}{lets}{sec}arrange {{\n  drop\n  ~ acid_* drift 0.15\n  drop\n}}\n"));
    // a mutation of the bars reaches the part that plays the let of them
    let src = format!("{HEAD}{lets}{sec}arrange {{\n  drop\n  ~ acid_* up 1\n  drop\n}}\n");
    let n = nets(&ok(&src));
    assert_ne!(n[0], n[1], "`~ acid_* up 1` did not reach `bass line`");
    // with the one-shot the reported name came from (`acid_a@1.once1`): a `@` that leaves a bar's steps all rests
    let src = format!("{HEAD}{lets}{sec}arrange {{\n  drop\n  ~ acid_* drift 0.15\n  drop\n  @ acid_a decay\n  drop\n  drop\n}}\n");
    let n = nets(&ok(&src));
    assert_ne!(n[1], n[2], "`@ acid_a decay` did not reach `bass line`");
    assert_eq!(n[1], n[3], "a one-shot lasts one section");
}

/// The root of 1-3: a generation whose text is one token (`x`, `~`) was a `let` of one value, which is put in place of its
/// name before the text is read, so the lowered name stayed a word.
#[test]
fn bug1_a_generation_of_one_step_is_still_a_pattern() {
    for chain in ["half", "thin 1", "decay", "half | half"] {
        let src = format!("{HEAD}let p = x ~\nsection s 1 bars {{\n  deck A: kick k | hihat p\n}}\narrange {{\n  s\n  ~ p {chain}\n  s\n  @ p {chain}\n  s\n}}\n");
        ok(&src);
    }
}

// ---- 2. a section with a `sing` line

#[test]
fn bug2_a_one_shot_before_a_section_that_sings() {
    let src = format!(
        "{HEAD}let acid = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nlet shout = 5 ~ ~ ~ 5 ~ ~ ~ 3 ~ ~ ~ 1 ~ ~ ~\nsection intro 4 bars {{\n  deck A: kick k\n  deck B: bass acid\n}}\nsection build 4 bars {{\n  deck A: kick k\n  deck B: bass acid inst=sync-lead | melody shout inst=voice-robot\n  sing \"a-cid! a-cid!\" on melody bars=4 voice=voice-robot\n}}\narrange {{\n  intro\n  @ acid thin 0.3\n  build\n  @ acid thin 1\n  build\n}}\n"
    );
    let n = nets(&ok(&src));
    assert_ne!(n[1], n[2], "`@ acid thin 1` did not reach the sung section's bass");
}

#[test]
fn bug2_a_mutation_before_an_outro_that_sings_and_evolves() {
    let src = format!(
        "{HEAD}let acid = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nlet shout = 5 ~ ~ ~ 5 ~ ~ ~ 3 ~ ~ ~ 1 ~ ~ ~\nsection intro 4 bars {{\n  deck A: kick k\n  deck B: bass acid\n}}\nsection outro 4 bars {{\n  deck A: kick k\n  deck B: bass acid | melody shout inst=voice-robot\n  sing \"a-cid!\" on melody bars=4 voice=voice-robot\n  evolve every 1 bars {{ bass: rotate 1 }}\n}}\narrange {{\n  intro\n  ~ acid thin 0.4 | down 2\n  outro\n  ~ acid thin 1 | down 2\n  outro\n}}\n"
    );
    let m = messages(&src);
    assert!(!m.iter().any(|m| m.contains("no `bass` part here")), "{m:?}");
    ok(&src);
}

// ---- 3. `half` and `decay`

#[test]
fn bug3_half_and_decay_keep_the_section() {
    for line in ["[1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]", "1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1"] {
        let src = format!("{HEAD}let line = {line}\nsection drop 4 bars {{\n  deck A: kick k\n  deck B: bass line\n}}\narrange {{\n  drop\n  @ line decay | half\n  drop\n  @ line half\n  drop\n  @ line decay\n  drop\n}}\n");
        let n = nets(&ok(&src));
        assert_ne!(n[0], n[1], "{line}: `decay | half` changed nothing");
        assert_ne!(n[0], n[2], "{line}: `half` changed nothing");
        assert_ne!(n[0], n[3], "{line}: `decay` changed nothing");
    }
    // `decay` takes one hit off a bar's steps, `half` keeps the first half of them: inside a `[...]` as in the plain line
    let src = format!("{HEAD}let line = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nsection drop 4 bars {{\n  deck B: bass line\n}}\narrange {{\n  ~ line half\n  drop\n}}\n");
    assert_eq!(last(&src, "line"), "[1 1 ~ 1 3 ~ 1 5]");
    let src = format!("{HEAD}let line = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nsection drop 4 bars {{\n  deck B: bass line\n}}\narrange {{\n  ~ line decay\n  drop\n}}\n");
    let after = last(&src, "line");
    let hits = |s: &str| s.trim_matches(['[', ']']).split(' ').filter(|w| *w != "~").count();
    assert_eq!(hits(&after), 11, "{after}");
}

// ---- 4. `ratchet` on a bar of sixteenths

#[test]
fn bug4_ratchet_strikes_a_sixteenth_inside_it() {
    let src = format!("{HEAD}let hats = x x x x x x x x x x x x x x x x\nlet acid = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nsection drop 4 bars {{\n  deck A: kick k | hihat hats\n  deck B: bass acid\n}}\narrange {{\n  drop\n  ~ hats ratchet 1\n  ~ acid ratchet 0.15\n  drop\n}}\n");
    let m = messages(&src);
    assert!(!m.iter().any(|m| m.contains("does not divide the bar into sixteenths")), "{m:?}");
    assert_eq!(last(&src, "hats"), "x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2 x:2");
    let acid = last(&src, "acid");
    assert!(!acid.contains("]*"), "the bar was doubled, not its hits: {acid}");
    let c = ok(&src);
    let hat = |seg: usize| c.segments[seg].decks.values().flat_map(|d| d.parts.iter()).find(|p| p.role == "hihat").unwrap().cycles[0].clone();
    assert_eq!(hat(1).len(), 16, "no hit is lost");
    assert!(hat(1).iter().all(|h| h.marks.ratchet == Some(2)));
    assert!(hat(0).iter().all(|h| h.marks.ratchet.is_none()));
}

#[test]
fn bug4_a_coarser_step_still_doubles_on_the_grid() {
    // eight steps: `*2` keeps the hits on sixteenths, as it always did
    let src = format!("{HEAD}let p = x ~ x ~ x ~ x ~\nsection s 1 bars {{\n  deck A: hihat p\n}}\narrange {{\n  ~ p ratchet 1\n  s\n}}\n");
    assert_eq!(last(&src, "p"), "x*2 ~ x*2 ~ x*2 ~ x*2 ~");
    let m = messages(&src);
    assert!(!m.iter().any(|m| m.contains("does not divide")), "{m:?}");
}

// ---- 5. a `<...>` cycle of bars

#[test]
fn bug5_operators_act_on_each_bar_of_a_cycle() {
    let cyc = "<[1 ~ 3 ~ 5 ~ 3 ~ 1 ~ 3 ~ 5 ~ 3 ~] [1 ~ 4 ~ 6 ~ 4 ~ 1 ~ 4 ~ 6 ~ 4 ~]>";
    let src = |op: &str| format!("{HEAD}let cyc = {cyc}\nsection drop 4 bars {{\n  deck A: kick k\n  deck B: bass cyc\n}}\narrange {{\n  drop\n  ~ cyc {op}\n  drop\n}}\n");
    assert_eq!(last(&src("rot 3"), "cyc"), "<[~ 3 ~ 1 ~ 3 ~ 5 ~ 3 ~ 1 ~ 3 ~ 5] [~ 4 ~ 1 ~ 4 ~ 6 ~ 4 ~ 1 ~ 4 ~ 6]>");
    let accented = last(&src("accent 0.3"), "cyc");
    assert!(accented.contains('!') && !accented.ends_with(">!"), "{accented}");
    let ratcheted = last(&src("ratchet 0.3"), "cyc");
    assert!(ratcheted.contains(":2") && !ratcheted.contains(">*") && !ratcheted.contains(">!*"), "{ratcheted}");
    for op in ["rot 3", "accent 0.3", "ratchet 0.3", "rev", "thin 0.5", "up 2"] {
        let n = nets(&ok(&src(op)));
        assert_ne!(n[0], n[1], "`{op}` changed nothing in the cycle");
        let m = messages(&src(op));
        assert!(!m.iter().any(|m| m.contains("does not divide")), "{op}: {m:?}");
    }
}

// ---- 6. numerals and generators are pitched

#[test]
fn bug6_roman_numerals_move_by_scale_degrees() {
    let src = format!("{HEAD}let chords = <i i iv v>\nsection drop 4 bars {{\n  deck A: kick k\n  deck B: harmony chords\n}}\narrange {{\n  drop\n  ~ chords up 2\n  drop\n  ~ chords down 2\n  drop\n}}\n");
    let m = messages(&src);
    assert!(!m.iter().any(|m| m.contains("drum pattern")), "{m:?}");
    let t = table(&src);
    let g = &t.patterns["chords"];
    // A minor: i up two degrees is the key's own triad on its third (C major, III), iv's on the sixth, v's on the seventh
    assert_eq!(g[1].text, "<III III VI VII>");
    assert_eq!(g[2].text, "<i i iv v>");
    let n = nets(&ok(&src));
    assert_ne!(n[0], n[1]);
}

#[test]
fn bug6_a_numeral_the_key_does_not_build_keeps_its_quality() {
    // V7 in A minor is no triad of the key: it moves its degree and keeps what it is
    let src = format!("{HEAD}let chords = i V7\nsection drop 1 bars {{\n  deck B: harmony chords\n}}\narrange {{\n  ~ chords up 1\n  drop\n}}\n");
    assert_eq!(last(&src, "chords"), "ii° VI7");
}

#[test]
fn bug6_a_generator_is_pitched() {
    let gen = "<markov(0.3, 1303) markov(0.3, 909)>";
    let src = |op: &str| format!("{HEAD}let stabs = {gen}\nsection drop 4 bars {{\n  deck A: kick k\n  deck B: melody stabs\n}}\narrange {{\n  drop\n  ~ stabs {op}\n  drop\n}}\n");
    for op in ["up 2", "oct 1"] {
        let m = messages(&src(op));
        assert!(!m.iter().any(|m| m.contains("drum pattern")), "{op}: {m:?}");
    }
    // `oct` moves what the generator plays by whole octaves
    assert_eq!(last(&src("oct 1"), "stabs"), "<octave(markov(0.3, 1303), 1) octave(markov(0.3, 909), 1)>");
    let n = nets(&ok(&src("oct 1")));
    assert_ne!(n[0], n[1]);
    // a step of scale degrees has no meaning for notes the generator picks as it plays: said as that, not as a drum
    let m = messages(&src("up 2"));
    assert!(m.iter().any(|m| m.contains("markov(...)") && m.contains("leaves them")), "{m:?}");
    let c = check(&src("up 2"));
    let d = c.diagnostics.iter().find(|d| d.message.contains("markov(...)")).unwrap();
    assert!(d.help.as_deref().is_some_and(|h| h.contains("oct 1")), "{:?}", d.help);
}

// ---- 7. evolve takes more of the operators

#[test]
fn bug7_evolve_takes_fill_slow_and_fast() {
    use djlive::ast::Deck;
    let src = format!("{HEAD}let acid = 1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1\nsection drop 8 bars {{\n  deck A: kick k | hihat x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n  deck B: bass acid\n  evolve every 1 bars {{\n    hihat: fill 0.2 seed=303\n    bass: slow <1 1 2 2>\n  }}\n}}\narrange drop\n");
    let c = ok(&src);
    let part = |seg: usize, d: Deck, role: &str| c.segments[seg].decks[&d].parts.iter().find(|p| p.role == role).unwrap().cycles[0].iter().map(|h| (h.step, h.note)).collect::<Vec<_>>();
    assert!(c.segments.len() >= 4, "{}", c.segments.len());
    // fill only ever adds hits, on steps that were rests, the note of the hit before
    let hats: Vec<usize> = (0..c.segments.len()).map(|s| part(s, Deck::A, "hihat").len()).collect();
    assert_eq!(hats[0], 8);
    assert!(hats.windows(2).all(|w| w[1] >= w[0]) && *hats.last().unwrap() > 8, "{hats:?}");
    // slow 1 (gens 1, 2) leaves the bass; slow 2 (gen 3) plays the first half of the bar over the whole of it
    assert_eq!(part(0, Deck::B, "bass"), part(2, Deck::B, "bass"));
    let b2 = part(2, Deck::B, "bass");
    let half: Vec<(u8, u8)> = b2.iter().filter(|(s, _)| *s < 8).map(|(s, n)| (s * 2, *n)).collect();
    assert_eq!(part(3, Deck::B, "bass"), half);
    // the same seed is the same fill; another is another
    assert_eq!(ok(&src).lineage, c.lineage);
    assert_ne!(ok(&src.replace("seed=303", "seed=304")).lineage, c.lineage);
    // fast 2: the bar twice in the time of one
    let src2 = format!("{HEAD}section drop 2 bars {{\n  deck A: hihat x ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~\n  evolve every 1 bars {{ hihat: fast 2 }}\n}}\narrange drop\n");
    let c2 = ok(&src2);
    let hat = c2.segments[1].decks[&Deck::A].parts[0].cycles[0].iter().map(|h| h.step).collect::<Vec<_>>();
    assert_eq!(hat, vec![0, 2, 8, 10]);
}

#[test]
fn bug7_an_arrange_only_operator_says_so_in_an_evolve() {
    let src = format!("{HEAD}section drop 4 bars {{\n  deck B: bass 1 3 5 3\n  evolve every 1 bars {{ bass: drift 0.2 }}\n}}\narrange drop\n");
    let c = check(&src);
    assert!(!c.ok());
    let d = c.diagnostics.iter().find(|d| d.message.contains("drift")).expect("a diagnostic about drift");
    assert!(d.message.contains("arrange"), "{}", d.message);
}

/// A sugared cycle is mutated as its longhand is: `iter(p, 2)` writes its second bar as `rotate(p, -2)`, which the
/// operators read as the steps it plays.
#[test]
fn bug5_a_sugared_cycle_is_mutated_as_its_longhand() {
    let arranged = |acid: &str| format!("{HEAD}let acid = {acid}\nsection verse 4 bars {{\n  deck B: bass acid\n}}\narrange {{\n  verse\n  ~ acid rot 1 | rev\n  verse\n  @ acid swap\n  verse\n}}\n");
    assert_eq!(nets(&ok(&arranged("iter([c2 e2 g2 b2], 2)"))), nets(&ok(&arranged("<[c2 e2 g2 b2] [g2 b2 c2 e2]>"))));
    assert_eq!(nets(&ok(&arranged("palindrome([c2 e2 g2 b2])"))), nets(&ok(&arranged("<[c2 e2 g2 b2] [b2 g2 e2 c2]>"))));
}
