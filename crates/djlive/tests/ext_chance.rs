//! F7, seeded chance (docs/beatscript/ext-chance.md): `x?0.3`, `choose [a b c] weights 3 1 1 seed=4`, `maybe(p, 0.3, 4)`.
//! Decided by an integer hash of (seed, bar, step, note), so the same text is the same hits, and every bar can differ.

mod common;

use common::{reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::diag::Diagnostic;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "should not compile: {src}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn notes(src: &str) -> Vec<String> {
    check(src).diagnostics.into_iter().filter(|d| !d.is_error()).map(|d| d.message).collect()
}

/// (step, note) of every hit of a part, per bar of the net.
fn hits(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<(u8, u8)>> {
    c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role}")).cycles.iter().map(|b| b.iter().map(|h| (h.step, h.note)).collect()).collect()
}

/// A program of `bars` bars of one part on one deck, in a section, so the net is `bars` long.
fn in_section(part: &str, bars: u32) -> String {
    format!("section run {bars} bars {{\n  deck A: {part}\n}}\narrange run\n")
}

fn section_hits(part: &str, bars: u32, role: &str) -> Vec<Vec<(u8, u8)>> {
    let c = ok(&in_section(part, bars));
    c.segments[0].decks[&Deck::A].parts.iter().find(|p| p.role == role).unwrap().cycles.iter().map(|b| b.iter().map(|h| (h.step, h.note)).collect()).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// determinism

#[test]
fn the_same_text_is_the_same_hits_every_time() {
    let src = "deck A: kick x*8?0.7 | hihat x*16?0.5 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n";
    let a = ok(src);
    let b = ok(src);
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    for role in ["kick", "hihat", "bass", "snare"] {
        assert_eq!(hits(&a, Deck::A, role), hits(&b, Deck::A, role), "{role}");
    }
    // another formatting of the same program: the same notes
    let other = "deck A:   kick x*8?0.70\n   | hihat x*16?50% | bass choose [c2 e2 g2] weights 3 1 1 seed=4\n | snare maybe([x ~ x ~], 0.30, 4)   # noise\n";
    let c = ok(other);
    assert_eq!(c.cid, a.cid);
    assert_eq!(c.decks[&Deck::A].net_cid(), a.decks[&Deck::A].net_cid());
}

#[test]
fn every_bar_can_differ() {
    let c = ok("deck A: kick x*16?0.5\n");
    let bars = hits(&c, Deck::A, "kick");
    assert_eq!(bars.len(), 16, "a program with no sections is unrolled over 16 bars");
    let mut distinct = bars.clone();
    distinct.sort();
    distinct.dedup();
    assert!(distinct.len() >= 15, "sixteen bars of sixteen coin tosses are not the same bar: {}", distinct.len());
}

#[test]
fn a_different_seed_is_a_different_pattern() {
    let a = hits(&ok("deck A: kick x*16?0.5 seed=1\n"), Deck::A, "kick");
    let b = hits(&ok("deck A: kick x*16?0.5 seed=2\n"), Deck::A, "kick");
    let a2 = hits(&ok("deck A: kick x*16?0.5 seed=1\n"), Deck::A, "kick");
    assert_ne!(a, b);
    assert_eq!(a, a2);
    // and the part's text, when it has no seed: another text, another notes
    let c = hits(&ok("deck A: kick x*16?0.5 vel=100\n"), Deck::A, "kick");
    assert_ne!(a, c);
    // a deck is part of that text: the same part on the other deck is not the same toss
    let on_b = ok("deck B: kick x*16?0.5\n");
    let on_a = ok("deck A: kick x*16?0.5\n");
    assert_ne!(hits(&on_a, Deck::A, "kick"), hits(&on_b, Deck::B, "kick"));
}

#[test]
fn the_hits_are_pinned_so_a_change_of_the_hash_cannot_pass_unseen() {
    let c = ok("deck A: kick x*16?0.5 seed=1\n");
    let bars = hits(&c, Deck::A, "kick");
    let steps = |b: usize| bars[b].iter().map(|h| h.0).collect::<Vec<u8>>();
    assert_eq!(steps(0), GOLD_BAR_0);
    assert_eq!(steps(1), GOLD_BAR_1);
    assert_eq!(steps(15), GOLD_BAR_15);
}
// worked out by hand from the hash, in Python (hash(1, bar, bar, 60 * step, -1) < 2^31), not read off the run
const GOLD_BAR_0: [u8; 9] = [3, 6, 8, 10, 11, 12, 13, 14, 15];
const GOLD_BAR_1: [u8; 9] = [0, 1, 2, 6, 7, 10, 12, 14, 15];
const GOLD_BAR_15: [u8; 8] = [1, 2, 3, 4, 6, 11, 13, 15];

// ---------------------------------------------------------------------------------------------------------------
// probability

#[test]
fn the_extremes_never_and_always() {
    let never = hits(&ok("deck A: kick x*8?0\n"), Deck::A, "kick");
    assert!(never.iter().all(Vec::is_empty));
    let never = hits(&ok("deck A: kick x*8?0%\n"), Deck::A, "kick");
    assert!(never.iter().all(Vec::is_empty));
    let always = hits(&ok("deck A: kick x*8?1\n"), Deck::A, "kick");
    let plain = hits(&ok("deck A: kick x*8\n"), Deck::A, "kick");
    assert!(always.iter().all(|b| *b == plain[0]));
    assert_eq!(hits(&ok("deck A: kick x*8?100%\n"), Deck::A, "kick")[0], plain[0]);
    // the extremes of a note and of a choice
    assert!(hits(&ok("deck A: bass [c2 e2 g2]?0\n"), Deck::A, "bass").iter().all(Vec::is_empty));
    assert_eq!(hits(&ok("deck A: bass [c2 e2 g2]?1\n"), Deck::A, "bass")[0].len(), 3);
}

#[test]
fn the_empirical_rate_over_4096_bars_is_within_tolerance_of_the_chance() {
    // 64 seeds of a 64-bar section: 4096 bars, one decision a bar
    for (p, text) in [(0.1, "0.1"), (0.3, "0.3"), (0.5, "50%"), (0.7, "0.7"), (0.9, "0.9")] {
        let mut kept = 0usize;
        for seed in 0..64 {
            let bars = section_hits(&format!("kick x?{text} seed={seed}"), 64, "kick");
            assert_eq!(bars.len(), 64);
            kept += bars.iter().filter(|b| !b.is_empty()).count();
        }
        let rate = kept as f64 / 4096.0;
        assert!((rate - p).abs() < 0.03, "p {p}: {rate}");
    }
}

#[test]
fn each_hit_of_a_fast_term_is_its_own_toss() {
    // 16 decisions a bar, 64 bars: 1024 of them
    let bars = section_hits("hihat x*16?0.25 seed=9", 64, "hihat");
    let n: usize = bars.iter().map(Vec::len).sum();
    let rate = n as f64 / 1024.0;
    assert!((rate - 0.25).abs() < 0.05, "{rate}");
    // and the steps are spread over the bar, not always the same ones
    let mut per_step = [0usize; 16];
    for b in &bars {
        for (s, _) in b {
            per_step[*s as usize] += 1;
        }
    }
    assert!(per_step.iter().all(|c| *c > 3), "{per_step:?}");
}

#[test]
fn the_same_term_in_two_places_makes_two_decisions() {
    // four slots, each `x?0.5`: if the decision ignored where the term is, the four would agree in every bar
    let bars = section_hits("kick x?0.5 x?0.5 x?0.5 x?0.5 seed=3", 64, "kick");
    let agree = bars.iter().filter(|b| b.is_empty() || b.len() == 4).count();
    assert!(agree < 24, "the four slots agree in {agree} of 64 bars: that is correlation (expect 8)");
    // inside a fast group: `[x?0.5 ~]*2` has two repeats, two tosses
    let bars = section_hits("kick [x?0.5 ~]*2 seed=3", 64, "kick");
    let same = bars.iter().filter(|b| b.is_empty() || b.len() == 2).count();
    assert!(same < 48, "the two repeats agree in {same} of 64 bars (expect 32)");
    // two alternatives of an alternation are two tosses, and the bar picks which
    let bars = section_hits("kick <x?0.5 x?0.5> seed=3", 64, "kick");
    let even: usize = bars.iter().step_by(2).map(Vec::len).sum();
    let odd: usize = bars.iter().skip(1).step_by(2).map(Vec::len).sum();
    assert!(even > 8 && even < 28 && odd > 8 && odd < 28, "{even} {odd}");
}

#[test]
fn a_chance_after_a_generator_a_group_a_chord_or_a_let_thins_what_it_makes() {
    let full = hits(&ok("deck A: kick e(5,8)\n"), Deck::A, "kick")[0].clone();
    let bars = section_hits("kick e(5,8)?0.5 seed=2", 64, "kick");
    let all: Vec<u8> = bars.iter().flatten().map(|h| h.0).collect();
    assert!(all.iter().all(|s| full.iter().any(|f| f.0 == *s)), "only steps of the rhythm");
    assert!(bars.iter().any(|b| b.len() < full.len()) && bars.iter().any(|b| !b.is_empty()));
    // a chord is thinned note by note: the notes of one chord are tossed apart
    let bars = section_hits("harmony chord(am)?0.5 seed=2", 64, "harmony");
    assert!(bars.iter().any(|b| b.len() == 1 || b.len() == 2), "a chord is its notes, each tossed");
    // a let pattern, a group
    let src = "let riff = x ~ x ~\ndeck A: kick riff?0.5\n";
    let c = ok(src);
    assert!(hits(&c, Deck::A, "kick").iter().any(|b| b.len() < 2));
    assert_eq!(c.text, src);
    // the chance after an accent and a slide mark is after them: c2!^?0.5 reads
    assert!(check("deck A: bass c2!^?0.5 e2\n").ok());
}

// ---------------------------------------------------------------------------------------------------------------
// choose

#[test]
fn choose_picks_one_option_a_bar_by_weight() {
    let bars = section_hits("bass choose [c2 e2 g2] weights 3 1 1 seed=4", 64, "bass");
    for b in &bars {
        assert_eq!(b.len(), 1, "one option a bar, in the whole slot");
        assert_eq!(b[0].0, 0);
    }
    let count = |note: u8| bars.iter().filter(|b| b[0].1 == note).count();
    assert!(count(36) + count(40) + count(43) == 64);
    assert!(count(36) > count(40) && count(36) > count(43), "the weights 3 1 1 favour c2: {} {} {}", count(36), count(40), count(43));
}

#[test]
fn choose_weights_are_respected_over_4096_bars() {
    let mut counts = [0usize; 3];
    for seed in 0..64 {
        for b in section_hits(&format!("bass choose [c2 e2 g2] weights 3 1 1 seed={seed}"), 64, "bass") {
            counts[match b[0].1 {
                36 => 0,
                40 => 1,
                _ => 2,
            }] += 1;
        }
    }
    let share = |i: usize| counts[i] as f64 / 4096.0;
    assert!((share(0) - 0.6).abs() < 0.03, "{counts:?}");
    assert!((share(1) - 0.2).abs() < 0.03, "{counts:?}");
    assert!((share(2) - 0.2).abs() < 0.03, "{counts:?}");
    // a weight of 0 is never chosen, a lone weight always is
    let bars = section_hits("bass choose [c2 e2] weights 0 1 seed=5", 64, "bass");
    assert!(bars.iter().all(|b| b[0].1 == 40));
    let bars = section_hits("bass choose [c2 e2] weights 7 0 seed=5", 64, "bass");
    assert!(bars.iter().all(|b| b[0].1 == 36));
    // fractional weights
    let bars = section_hits("bass choose [c2 e2] weights 0.25 0.75 seed=5", 64, "bass");
    let e = bars.iter().filter(|b| b[0].1 == 40).count();
    assert!(e > 36 && e < 60, "{e}");
}

#[test]
fn a_choose_is_one_slot_and_its_option_fills_it() {
    let c = ok("deck A: snare x choose [x [x x] ~] weights 1 1 1 seed=2 x ~\n");
    let bars = hits(&c, Deck::A, "snare");
    // four slots of four steps: x at 0, the choice at 4 (one hit at 4, two at 4 and 6, or none), x at 8, a rest
    assert!(bars.iter().all(|b| b[0].0 == 0 && b.iter().any(|h| h.0 == 8)));
    let in_slot: Vec<usize> = bars.iter().map(|b| b.iter().filter(|h| (4..8).contains(&h.0)).count()).collect();
    assert!(in_slot.contains(&0) && in_slot.contains(&1) && in_slot.contains(&2), "{in_slot:?}");
    assert!(bars.iter().all(|b| b.iter().all(|h| h.0 <= 8)));
}

#[test]
fn choose_takes_its_seed_from_the_term_then_the_part_then_the_text() {
    let own_a = hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 seed=4\n"), Deck::A, "bass");
    let own_b = hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 seed=5\n"), Deck::A, "bass");
    assert_ne!(own_a, own_b);
    // the term's own seed wins over the part's
    let both = hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 seed=4 seed=9\n"), Deck::A, "bass");
    assert_eq!(both, own_a);
    // with no seed on the term, the part's: `x choose ...` ends with a seed that belongs to the choice, so give it a second term
    let part = hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 ~ seed=7\n"), Deck::A, "bass");
    let part2 = hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 ~ seed=7\n"), Deck::A, "bass");
    assert_eq!(part, part2);
    assert_ne!(part, hits(&ok("deck A: bass choose [c2 e2 g2] weights 1 1 1 ~ seed=8\n"), Deck::A, "bass"));
}

#[test]
fn the_weights_are_read_by_count_so_a_degree_after_them_is_a_step() {
    let c = ok("key am\ndeck A: bass choose [c2 e2] weights 3 1 2\n");
    let bars = hits(&c, Deck::A, "bass");
    // two slots: the choice at step 0 and the degree 2 (B in A minor: 47 at octave 2, `2` is the second degree) at step 8
    assert!(bars.iter().all(|b| b.len() == 2 && b[0].0 == 0 && b[1].0 == 8));
    let n = notes("key am\ndeck A: bass choose [c2 e2] weights 3 1 2\n");
    assert!(n.iter().any(|m| m.contains("this 2 is read as a degree step after the choice: its 2 weights were read already")), "{n:?}");
    // a seed written after the weights ends the choice and the warning
    let n = notes("key am\ndeck A: bass choose [c2 e2] weights 3 1 seed=3 2\n");
    assert!(!n.iter().any(|m| m.contains("degree step")), "{n:?}");
}

#[test]
fn choose_errors_name_both_counts_and_say_what_to_write() {
    let e = errors("deck A: bass choose [c2 e2 g2] weights 3 1\n");
    assert!(e[0].message.contains("this `choose` has 3 options and 2 weights"), "{}", e[0].message);
    assert_eq!(e[0].help.as_deref(), Some("choose [c2 e2 g2] weights 1 1 1"));
    let e = errors("deck A: bass choose [c2 e2 g2] weights\n");
    assert!(e[0].message.contains("3 options and 0 weights"), "{}", e[0].message);
    let e = errors("deck A: bass choose [c2 e2 g2] weights 3 1 x\n");
    assert!(e[0].message.contains("3 options and 2 weights"), "{}", e[0].message);
    assert!(check("deck A: bass choose [c2] weights 1 1\n").ok(), "an extra number is a step after the choice");
    let e = errors("deck A: bass choose [c2 e2]\n");
    assert!(e[0].message.contains("needs its weights after the options"), "{}", e[0].message);
    assert!(e[0].help.as_deref().unwrap().contains("weights 1 1"));
    let e = errors("deck A: bass choose c2 e2\n");
    assert!(e[0].message.contains("takes its options in brackets"), "{}", e[0].message);
    let e = errors("deck A: bass choose [] weights 1\n");
    assert!(e[0].message.contains("no options") || e[0].message.contains("has nothing"), "{}", e[0].message);
    let e = errors("deck A: bass choose [c2, e2] weights 1 1\n");
    assert!(e[0].message.contains("a choice picks one term: a comma would stack"), "{}", e[0].message);
    let e = errors("deck A: bass choose [c2 e2] weights -1 2\n");
    assert!(e[0].message.contains("a weight is not negative"), "{}", e[0].message);
    let e = errors("deck A: bass choose [c2 e2] weights 0 0\n");
    assert!(e[0].message.contains("every weight is 0"), "{}", e[0].message);
    let e = errors("deck A: bass choose [c2 e2] weights 0.1234567 1\n");
    assert!(e[0].message.contains("six decimals"), "{}", e[0].message);
    // where it says it: line and column of the choose
    let src = "deck A: kick x | bass choose [c2 e2 g2] weights 3 1\n";
    let e = errors(src);
    assert_eq!(e[0].position(src).0, 1);
    assert_eq!(e[0].position(src).1, src.find("choose").unwrap() + 1);
}

// ---------------------------------------------------------------------------------------------------------------
// maybe

#[test]
fn maybe_is_the_pattern_with_a_chance_on_each_hit() {
    let a = hits(&ok("deck A: kick maybe([x ~ x ~], 0.3, 4)\n"), Deck::A, "kick");
    let b = hits(&ok("deck A: kick [x?0.3 ~ x?0.3 ~] seed=4\n"), Deck::A, "kick");
    assert_eq!(a, b);
    let a = hits(&ok("deck A: bass maybe([c2 e2 g2 b2], 0.6, 11)\n"), Deck::A, "bass");
    let b = hits(&ok("deck A: bass [c2?0.6 e2?0.6 g2?0.6 b2?0.6] seed=11\n"), Deck::A, "bass");
    assert_eq!(a, b);
    // with no seed it is the part's
    let a = hits(&ok("deck A: kick maybe([x*8], 0.5) seed=6\n"), Deck::A, "kick");
    let b = hits(&ok("deck A: kick x*8?0.5 seed=6\n"), Deck::A, "kick");
    assert_eq!(a, b);
    // and the seed matters
    assert_ne!(hits(&ok("deck A: kick maybe([x*8], 0.5, 1)\n"), Deck::A, "kick"), hits(&ok("deck A: kick maybe([x*8], 0.5, 2)\n"), Deck::A, "kick"));
    // the extremes
    assert!(hits(&ok("deck A: kick maybe([x x], 0, 1)\n"), Deck::A, "kick").iter().all(Vec::is_empty));
    assert_eq!(hits(&ok("deck A: kick maybe([x x], 1, 1)\n"), Deck::A, "kick")[0].len(), 2);
}

#[test]
fn maybe_errors_say_what_to_write() {
    let e = errors("deck A: kick maybe([x x])\n");
    assert!(e[0].message.contains("maybe() takes 2 to 3, and this has 1"), "{}", e[0].message);
    assert!(e[0].help.as_deref().unwrap().contains("maybe([x ~ x ~], 0.3, 4)"));
    let e = errors("deck A: kick maybe([x x], 1.5)\n");
    assert!(e[0].message.contains("a chance is 0 to 1"), "{}", e[0].message);
    let e = errors("deck A: kick maybe([x x], 0.5, 1.5)\n");
    assert!(e[0].message.contains("the seed"), "{}", e[0].message);
    let e = errors("deck A: kick maybe([x x], x)\n");
    assert!(e[0].message.contains("the chance"), "{}", e[0].message);
    let e = errors("deck A: kick maybe [x x]\n");
    assert!(e[0].message.contains("`maybe` takes its pattern and chance in parentheses"), "{}", e[0].message);
}

// ---------------------------------------------------------------------------------------------------------------
// the syntax

#[test]
fn a_chance_is_glued_to_its_term_and_is_a_number_from_zero_to_one() {
    for (src, want) in [
        ("deck A: kick x ?0.3\n", "glued to its term"),
        ("deck A: kick x? 0.3\n", "glued to its `?`"),
        ("deck A: kick x?*\n", "expected the chance after `?`"),
        ("deck A: kick x?hit\n", "expected the chance after `?`"),
        ("deck A: kick x?1.5\n", "a chance is 0 to 1"),
        ("deck A: kick x?150%\n", "a chance is 0 to 1"),
        ("deck A: kick x?30\n", "a chance is 0 to 1"),
        ("deck A: kick x?0.3333333\n", "at most six decimals"),
        ("deck A: kick x?0.3*2\n", "comes before the chance"),
        ("deck A: bass c2?0.3!\n", "comes before the chance"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|d| d.message.contains(want)), "{src}: {:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert!(e.iter().all(|d| d.help.is_some() || !d.expected.is_empty()), "{src}: every error says what to write");
    }
    // where: the number
    let src = "deck A: kick x*8?1.5\n";
    let e = errors(src);
    assert_eq!(e[0].position(src).1, src.find("1.5").unwrap() + 1);
}

#[test]
fn the_canonical_form() {
    assert_eq!(ok("deck A: kick x*8?0.7\n").text, "deck A: kick x*8?0.7\n");
    assert_eq!(ok("deck A: kick x*8?70%\n").text, "deck A: kick x*8?0.7\n");
    assert_eq!(ok("deck A: kick x?30%\n").text, "deck A: kick x?0.3\n");
    // a bare `?` is a half (pattern sugar), and prints so
    assert_eq!(ok("deck A: kick x? [x x?]\n").text, "deck A: kick x?0.5 [x x?0.5]\n");
    assert_eq!(ok("deck A: kick x?0.50\n").text, "deck A: kick x?0.5\n");
    // a chance of 1 is no chance and a chance of 0 is a rest: each is the program written out (L-09)
    assert_eq!(ok("deck A: kick x?1.0\n").text, "deck A: kick x\n");
    assert_eq!(ok("deck A: kick x*8?0 x\n").text, "deck A: kick ~ x\n");
    assert_eq!(ok("deck A: kick x?33.3333%\n").text, "deck A: kick x?0.333333\n");
    assert_eq!(ok("deck A:   bass   choose [ c2   e2 g2 ]  weights  3  1  1   seed = 4 \n").text, "deck A: bass choose [c2 e2 g2] weights 3 1 1 seed=4\n");
    assert_eq!(ok("deck A: bass choose [c2 e2 g2] weights 3 1 1\n").text, "deck A: bass choose [c2 e2 g2] weights 3 1 1\n");
    assert_eq!(ok("deck A: kick maybe([x ~ x ~],0.3,4)\n").text, "deck A: kick maybe([x ~ x ~], 0.3, 4)\n");
    assert_eq!(ok("deck A: kick maybe( [x ~ x ~] , 30% )\n").text, "deck A: kick maybe([x ~ x ~], 0.3)\n");
    // chained chances, a chance after a group and a note, the accent before it
    assert_eq!(ok("deck A: bass [c2 e2]?0.5?0.5 g2!?0.25\n").text, "deck A: bass [c2 e2]?0.5?0.5 g2!?0.25\n");
    // a weight as typed, a fraction too
    assert_eq!(ok("deck A: bass choose [c2 e2] weights 0.50 1.5\n").text, "deck A: bass choose [c2 e2] weights 0.5 1.5\n");
    // a choice with a repeat after it
    assert_eq!(ok("deck A: bass choose [c2 e2] weights 1 1*2\n").text, "deck A: bass choose [c2 e2] weights 1 1*2\n");
}

#[test]
fn the_canonical_text_is_a_fixed_point_and_reformatting_keeps_the_cid() {
    let programs = [
        "tempo 126\nkey am\ndeck A: kick x*8?0.7 | hihat x*16?30% vel=60..90 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: bass choose [c2 e2 g2] weights 3 1 1 seed=4 | kick [x x?0.5]*2 seed=3\n",
        "let riff = x?0.3 ~ x ~\ndef groove(p) {\n  deck A: kick x*8?$p\n}\ngroove(0.4)\n",
        "section verse 8 bars {\n  deck A: hihat x*16?0.5\n}\nsection chorus 4 bars {\n  deck A: hihat x*8?0.25 | bass choose [c2 e2] weights 2 1\n}\narrange verse chorus\n",
        "deck A: harmony chord(am)?0.5 | bass <c2?0.5 e2> | melody choose [c4 [e4 g4] ~] weights 1 1 1 seed=2 c4\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let want = ok(src);
        let again = ok(&want.text);
        assert_eq!(again.text, want.text, "program {n}: a fixed point");
        assert_eq!(again.cid, want.cid);
        let mut rng = Rng(0xc4a7_ce00 + n as u64);
        for round in 0..120 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text))).cid;
            assert_eq!(got, want.cid, "program {n} round {round} forked:\n{text}");
        }
    }
}

#[test]
fn a_semantic_edit_changes_the_cid() {
    let base = ok("deck A: kick x*8?0.7 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n").cid;
    for edit in [
        "deck A: kick x*8?0.6 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: kick x*8?0.7 | bass choose [c2 e2 g2] weights 3 1 2 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: kick x*8?0.7 | bass choose [c2 e2 g2] weights 3 1 1 seed=5 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: kick x*8?0.7 | bass choose [c2 e2 a2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: kick x*8?0.7 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 5)\n",
        "deck A: kick x*8 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n",
        "deck A: kick x*8?0.7 | bass choose [c2 e2 g2] weights 3 1 1 | snare maybe([x ~ x ~], 0.3, 4)\n",
    ] {
        assert_ne!(ok(edit).cid, base, "{edit}");
    }
}

#[test]
fn the_words_are_reserved_and_coloured() {
    for w in ["choose", "maybe"] {
        let e = errors(&format!("let {w} = 3\ndeck A: kick x\n"));
        assert!(e[0].message.contains("is a word of the language"), "{w}: {}", e[0].message);
    }
    // `weights` is syntax right after a choice's group only: it names a let, and the choice still reads it
    ok("let weights = 3\ndeck A: kick x*weights | bass choose [c2 e2] weights 1 1 seed=2\n");
    use djlive::highlight::{highlight, Class};
    let src = "deck A: bass choose [c2 e2] weights 1 1 seed=2 | kick maybe([x], 0.5) x?0.3";
    let c: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    assert!(c.contains(&("choose", Class::Keyword)));
    assert!(c.contains(&("weights", Class::Keyword)));
    assert!(c.contains(&("maybe", Class::Generator)));
    assert!(c.contains(&("?", Class::Operator)));
    assert!(c.contains(&("seed", Class::Param)));
}

// ---------------------------------------------------------------------------------------------------------------
// bars: how long a chance pattern is

#[test]
fn a_program_with_no_sections_unrolls_sixteen_bars_and_a_section_its_length() {
    assert_eq!(ok("deck A: kick x*8?0.5\n").decks[&Deck::A].period, 16);
    let c = ok("section verse 8 bars {\n  deck A: kick x*8?0.5\n}\narrange verse\n");
    assert_eq!(c.segments[0].decks[&Deck::A].period, 8);
    let c = ok("section verse 1 bars {\n  deck A: kick x*8?0.5\n}\narrange verse\n");
    assert_eq!(c.segments[0].decks[&Deck::A].period, 1);
    // capped by a net
    let c = ok("section verse 200 bars {\n  deck A: kick x*8?0.5\n}\narrange verse\n");
    assert_eq!(c.segments.iter().map(|s| s.bars).sum::<u32>(), 200);
    assert!(c.segments.iter().all(|s| s.decks[&Deck::A].period <= 64));
    // each section its own
    let c = ok("section verse 8 bars {\n  deck A: kick x*8?0.5\n}\nsection chorus 24 bars {\n  deck A: kick x*8?0.5\n}\narrange verse chorus\n");
    assert_eq!(c.segments[0].decks[&Deck::A].period, 8);
    assert_eq!(c.segments[1].decks[&Deck::A].period, 24);
    // a program with no chance is not made longer: its period is what it was
    assert_eq!(ok("deck A: kick x*8\n").decks[&Deck::A].period, 1);
    assert_eq!(ok("deck A: kick <x ~ x>\n").decks[&Deck::A].period, 3);
}

#[test]
fn an_alternation_and_chance_come_round_together() {
    // three bars of alternation, repeated to hold sixteen: 18
    assert_eq!(ok("deck A: kick <x x*2 x*4> | hihat x*8?0.5\n").decks[&Deck::A].period, 18);
    // a net is at most 64 bars, so a long alternation beside chance stays long
    let c = ok("section verse 8 bars {\n  deck A: kick <x x*2 x*4> | hihat x*8?0.5\n}\narrange verse\n");
    assert_eq!(c.segments[0].decks[&Deck::A].period, 9);
    // the part with no chance is the same in every copy of the alternation
    let k = hits(&ok("deck A: kick <x x*2 x*4> | hihat x*8?0.5\n"), Deck::A, "kick");
    assert_eq!(k.len(), 18);
    for i in 0..18 {
        assert_eq!(k[i], k[i % 3]);
    }
    // and chance in a part with a cycle of its own: the chance counts the part's cycles (here 16 bars of 12-step cycles)
    let c = ok("deck A: hihat/12 x*4?0.5\n");
    assert_eq!(c.decks[&Deck::A].period, 18);
}

#[test]
fn check_reports_every_chance_term_with_its_probability_seed_and_the_bars() {
    let n = notes("deck A: kick x*8?0.7 seed=12 | bass choose [c2 e2 g2] weights 3 1 1 seed=4 | snare maybe([x ~ x ~], 0.3, 4)\n");
    let chance: Vec<&String> = n.iter().filter(|m| m.starts_with("chance:")).collect();
    assert_eq!(chance.len(), 3, "{n:?}");
    assert!(chance.iter().any(|m| m.as_str() == "chance: `x*8?0.7` keeps each hit with probability 0.7 (seed 12); it differs every bar, unrolled over 16 bars"), "{chance:?}");
    assert!(chance.iter().any(|m| m.as_str() == "chance: `choose [c2 e2 g2] weights 3 1 1 seed=4` picks one of 3 options a bar, weights 3:1:1 (seed 4); it differs every bar, unrolled over 16 bars"), "{chance:?}");
    assert!(chance.iter().any(|m| m.as_str() == "chance: `maybe([x ~ x ~], 0.3, 4)` keeps each hit with probability 0.3 (seed 4); it differs every bar, unrolled over 16 bars"), "{chance:?}");
    // in a section of 8 bars: 8
    let n = notes(&in_section("kick x?0.5 seed=1", 8));
    assert!(n.iter().any(|m| m.contains("unrolled over 8 bars")), "{n:?}");
    // a chance with no seed of its own says the part's, whatever it came to
    let n = notes("deck A: kick x?0.5\n");
    let seed = n[0].split("(seed ").nth(1).unwrap().split(')').next().unwrap().parse::<u32>().unwrap();
    let again = ok("deck A: kick x?0.5\n");
    assert_eq!(hits(&again, Deck::A, "kick"), hits(&ok(&format!("deck A: kick x?0.5 seed={seed}\n")), Deck::A, "kick"));
    // a program with no chance says nothing about it
    assert!(notes("deck A: kick x*8 | snare ~ x ~ x\n").is_empty());
    // a note, not an error
    assert!(check("deck A: kick x?0.5\n").ok());
    // once for each chance term, even when a song builds the part in every section
    let n = notes("deck A: kick x?0.5\nsection verse 4 bars {\n  deck B: bass c2\n}\nsection chorus 6 bars {\n  deck B: bass e2\n}\nsection coda 6 bars {\n  deck B: bass g2\n}\narrange verse chorus coda\n");
    let chance: Vec<&String> = n.iter().filter(|m| m.starts_with("chance:")).collect();
    assert_eq!(chance.len(), 2, "said once for each length it is unrolled over: {n:?}");
    assert!(chance[0].ends_with("unrolled over 4 bars") && chance[1].ends_with("unrolled over 6 bars"), "{chance:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// marks

#[test]
fn a_kept_hit_lights_its_token_and_a_dropped_one_is_not_there() {
    let src = "deck A: kick x*8?0.5 seed=3\n";
    let c = ok(src);
    let part = &c.decks[&Deck::A].parts[0];
    let mut kept = 0;
    for bar in &part.cycles {
        for h in bar {
            let chain = &part.spans[h.sid as usize];
            assert_eq!(chain[0].text(src), "x*8", "the innermost place is the term the hit came from");
            assert!(chain.iter().any(|s| s.text(src) == "x*8?0.5"), "and the term that held it: {:?}", chain.iter().map(|s| s.text(src)).collect::<Vec<_>>());
            kept += 1;
        }
    }
    assert!(kept > 0 && kept < 8 * 16, "some of the 128 hits were dropped: {kept}");
    // the same through a let and a def
    let src = "let riff = x?0.5 x\ndef groove(p) {\n  deck A: kick riff x*4?$p\n}\ngroove(0.5)\n";
    let c = ok(src);
    let part = &c.decks[&Deck::A].parts[0];
    assert!(part.cycles.iter().flatten().all(|h| {
        let chain = &part.spans[h.sid as usize];
        chain.iter().any(|s| s.text(src).contains("riff") || s.text(src).contains("x?0.5") || s.text(src).contains("x*4?$p"))
    }));
}

// ---------------------------------------------------------------------------------------------------------------
// lets, defs and friends

#[test]
fn a_chance_comes_through_defs_lets_and_repeats() {
    let a = ok("def groove(p) {\n  deck A: kick x*8?$p seed=2\n}\ngroove(0.4)\n");
    let b = ok("deck A: kick x*8?0.4 seed=2\n");
    assert_eq!(hits(&a, Deck::A, "kick"), hits(&b, Deck::A, "kick"));
    let a = ok("let p = 0.4\ndeck A: kick x*8?p seed=2\n");
    assert_eq!(hits(&a, Deck::A, "kick"), hits(&b, Deck::A, "kick"));
    let a = ok("repeat 1 {\n  deck A: kick x*8?0.4 seed=2\n}\n");
    assert_eq!(hits(&a, Deck::A, "kick"), hits(&b, Deck::A, "kick"));
    // a let pattern holds a choice
    let a = ok("let pick = choose [c2 e2] weights 1 1\ndeck A: bass pick pick seed=4\n");
    assert_eq!(a.decks[&Deck::A].parts[0].cycles.iter().filter(|b| b.len() == 2).count(), 16);
}

#[test]
fn chance_in_a_cycle_of_its_own_and_with_a_euclidean_term() {
    // 18 bars of 12-step cycles: 24 cycles of five hits is 120 tosses; over many seeds the rate is the chance
    let full = ok("deck A: hihat/12 e(5,12)\n");
    let all: usize = hits(&full, Deck::A, "hihat").iter().map(Vec::len).sum();
    assert_eq!(all, 20, "three bars of the plain rhythm");
    let mut total = 0usize;
    let mut tosses = 0usize;
    for seed in 0..32 {
        let c = ok(&format!("deck A: hihat/12 e(5,12)?0.5 seed={seed}\n"));
        assert_eq!(c.decks[&Deck::A].period, 18);
        assert_eq!(c.decks[&Deck::A].parts[0].cycle_steps, 12);
        total += hits(&c, Deck::A, "hihat").iter().map(Vec::len).sum::<usize>();
        tosses += 120;
    }
    let rate = total as f64 / tosses as f64;
    assert!((rate - 0.5).abs() < 0.03, "{rate}");
    // a kept hit is on a step the plain rhythm has, cycle by cycle: so the thinned net is a subset of the full one over the same bars
    let thin = ok("deck A: hihat/12 e(5,12)?0.5 seed=1\n");
    let t = hits(&thin, Deck::A, "hihat");
    let f = hits(&full, Deck::A, "hihat");
    for (b, bar) in t.iter().enumerate() {
        for h in bar {
            assert!(f[b % 3].contains(h), "bar {b}: {h:?}");
        }
    }
}

#[test]
fn chance_does_not_touch_what_has_none() {
    // a part with no chance is what it was, in a deck whose other part has chance
    let plain = ok("deck A: snare ~ x ~ x\n");
    let mixed = ok("deck A: snare ~ x ~ x | hihat x*8?0.5\n");
    let s = hits(&mixed, Deck::A, "snare");
    assert_eq!(s.len(), 16);
    assert!(s.iter().all(|b| *b == hits(&plain, Deck::A, "snare")[0]));
}

#[test]
fn nothing_reads_a_clock_or_a_random_source() {
    // the same program compiled in another thread, after a sleep, is the same net
    let src = "deck A: kick x*16?0.5 | bass choose [c2 e2 g2] weights 3 1 1 | hihat maybe([x x x x], 0.4)\n";
    let want = ok(src).decks[&Deck::A].net_cid();
    let h = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(5));
        ok(src).decks[&Deck::A].net_cid()
    });
    assert_eq!(h.join().unwrap(), want);
}

// ---------------------------------------------------------------------------------------------------------------
// on a fake clock: the hits that fire are the kept ones

mod clock {
    use djengine::json::JsonValue;
    use djlive::runtime::{Grid, Live, LiveHost};

    #[derive(Default)]
    pub struct Fake {
        pub beat: f64,
        pub playing: [bool; 2],
        pub loads: Vec<(String, String)>,
    }

    impl LiveHost for Fake {
        fn grid(&self) -> Option<Grid> {
            let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
            Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
        }
        fn playing(&self, deck: &str) -> bool {
            self.playing[usize::from(deck == "B")]
        }
        fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
            self.loads.push((deck.into(), djengine::cid::share_cid(env)));
            Ok(())
        }
        fn set_playing(&mut self, deck: &str, on: bool) {
            self.playing[usize::from(deck == "B")] = on;
        }
        fn set_sound(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn call(&mut self, _: &str, _: &JsonValue) -> Result<JsonValue, String> {
            Ok(JsonValue::Null)
        }
    }

    impl Fake {
        pub fn run(&mut self, live: &mut Live, beats: f64) {
            for _ in 0..(beats * 4.0).round() as usize {
                self.beat += 0.25;
                live.service(self);
            }
        }
    }
}

#[test]
fn only_the_kept_hits_fire_and_each_lights_its_token() {
    use djlive::runtime::Live;
    let mut h = clock::Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    let mut live = Live::new();
    let src = "deck A: kick x*8?0.5 seed=3\n";
    assert!(live.eval(src).ok);
    // (bar, step) of every firing, in thirty-two bars of the clock
    let mut fired: Vec<(i64, i64, String)> = Vec::new();
    for _ in 0..(32 * 4 * 4) {
        h.run(&mut live, 0.25);
        for f in live.fires() {
            let bar = f.at.floor() as i64;
            let step = ((f.at - f.at.floor()) * 16.0).round() as i64;
            let tok = f.spans[0].text(src).to_string();
            if !fired.iter().any(|x| x.0 == bar && x.1 == step) {
                fired.push((bar, step, tok));
            }
        }
    }
    let c = ok(src);
    let net = hits(&c, Deck::A, "kick");
    assert_eq!(net.len(), 16);
    // the first bars while the net is landing are not compared; after that every bar fires exactly its kept hits
    let last = fired.iter().map(|f| f.0).max().unwrap() - 1;
    let mut compared = 0;
    for bar in 6..=last {
        let want: Vec<i64> = net[(bar % 16) as usize].iter().map(|h| i64::from(h.0)).collect();
        let mut got: Vec<i64> = fired.iter().filter(|f| f.0 == bar).map(|f| f.1).collect();
        got.sort();
        assert_eq!(got, want, "bar {bar}");
        compared += want.len();
    }
    assert!(compared > 30 && compared < 25 * 8, "some of the eight hits a bar were dropped: {compared} fired");
    // the token that lights is the term the hit came from
    assert!(fired.iter().all(|f| f.2 == "x*8"));
    // the bars repeat after the unrolled sixteen, not before
    let in_bar = |b: i64| -> Vec<i64> {
        let mut v: Vec<i64> = fired.iter().filter(|f| f.0 == b).map(|f| f.1).collect();
        v.sort();
        v
    };
    assert_eq!(in_bar(7), in_bar(23));
    assert_ne!(in_bar(7), in_bar(8));
}

#[test]
fn an_unchanged_chance_program_is_not_reloaded_and_an_edited_seed_is() {
    use djlive::runtime::Live;
    let mut h = clock::Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    let mut live = Live::new();
    assert!(live.eval("deck A: kick x*8?0.5 seed=3\n").ok);
    h.run(&mut live, 8.0);
    let loads = h.loads.len();
    assert!(loads >= 1);
    // the same text, spaced differently, compiles to the same net: nothing is loaded again
    assert!(live.eval("deck A:   kick   x*8?50%   seed=3   # same\n").ok);
    h.run(&mut live, 16.0);
    assert_eq!(h.loads.len(), loads, "a program that did not change is not touched");
    // a new seed is a new net
    assert!(live.eval("deck A: kick x*8?0.5 seed=4\n").ok);
    h.run(&mut live, 16.0);
    assert_eq!(h.loads.len(), loads + 1);
}

// ---------------------------------------------------------------------------------------------------------------
// the laws, on random programs: never a panic, a canonical fixed point, the same net twice, formatting keeps the CID

fn gen_term(rng: &mut Rng, depth: u32) -> String {
    let atom = match rng.below(if depth == 0 { 6 } else { 11 }) {
        0 => "x".to_string(),
        1 => "~".to_string(),
        2 => ["c2", "e2", "g3", "a#1"][rng.below(4) as usize].to_string(),
        3 => (1 + rng.below(7)).to_string(),
        4 => format!("e({},{})", rng.below(6), 5 + rng.below(8)),
        5 => "x".to_string(),
        6 => format!("[{}]", gen_seq(rng, depth - 1)),
        7 => format!("<{}>", gen_seq(rng, depth - 1)),
        8 => {
            let n = 1 + rng.below(3);
            let opts: Vec<String> = (0..n).map(|_| gen_term(rng, depth - 1)).collect();
            let ws: Vec<String> = (0..n).map(|_| ["0", "1", "2", "3", "0.5", "0.25"][rng.below(6) as usize].to_string()).collect();
            let weights = if ws.iter().all(|w| w == "0") { vec!["1".to_string(); n as usize] } else { ws };
            let seed = if rng.below(2) == 0 { format!(" seed={}", rng.below(20)) } else { String::new() };
            format!("choose [{}] weights {}{}", opts.join(" "), weights.join(" "), seed)
        }
        9 => format!("maybe([{}], {}{})", gen_seq(rng, depth - 1), ["0.5", "30%", "0", "1", "0.125"][rng.below(5) as usize], if rng.below(2) == 0 { format!(", {}", rng.below(9)) } else { String::new() }),
        _ => format!("reverse([{}])", gen_seq(rng, depth - 1)),
    };
    let mut t = atom;
    // a `choose` ends with its weights, so nothing that reads as a weight may follow it unseparated: marks and chances come after
    if rng.below(5) == 0 {
        t.push_str(&format!("*{}", 1 + rng.below(4)));
    }
    if rng.below(3) == 0 {
        t.push_str(&format!("?{}", ["0.5", "30%", "0", "1", "0.75", "0.125"][rng.below(6) as usize]));
    }
    t
}

fn gen_seq(rng: &mut Rng, depth: u32) -> String {
    let n = 1 + rng.below(4);
    (0..n).map(|_| gen_term(rng, depth)).collect::<Vec<_>>().join(" ")
}

#[test]
fn random_programs_with_chance_never_panic_and_keep_the_laws() {
    let mut rng = Rng(0x7a11_0c4a_0000_0001);
    let mut compiled = 0;
    for round in 0..400 {
        let role = ["kick", "bass", "hihat", "melody"][rng.below(4) as usize];
        let cycle = if rng.below(3) == 0 { format!("/{}", [4, 7, 8, 12, 16, 24][rng.below(6) as usize]) } else { String::new() };
        let seed = if rng.below(3) == 0 { format!(" seed={}", rng.below(100)) } else { String::new() };
        let src = format!("deck A: {role}{cycle} {}{seed}\n", gen_seq(&mut rng, 2));
        let c = check(&src);
        let Some(first) = c.compiled else { continue };
        compiled += 1;
        // a canonical fixed point with the same CID
        let again = check(&first.text).compiled.unwrap_or_else(|| panic!("round {round}: the canonical text does not compile:\n{src}\n{}", first.text));
        assert_eq!(again.text, first.text, "round {round}: {src}");
        assert_eq!(again.cid, first.cid, "round {round}: {src}");
        // the same text is the same nets
        assert_eq!(again.decks[&Deck::A].net_cid(), first.decks[&Deck::A].net_cid(), "round {round}: {src}");
        // formatting keeps the CID
        let mut r2 = Rng(0xabcd + round);
        for _ in 0..3 {
            let text = reformat(&src, &mut r2);
            let got = check(&text).compiled.unwrap_or_else(|| panic!("round {round}: a reformatting no longer compiles:\n{text}\n(from {src})")).cid;
            assert_eq!(got, first.cid, "round {round}:\n{text}\nfrom {src}");
        }
        // every hit is where a hit can be
        for p in &first.decks[&Deck::A].parts {
            assert!(!p.cycles.is_empty());
            for h in p.cycles.iter().flatten() {
                assert!(h.step < 16 && h.note < 128);
            }
        }
    }
    assert!(compiled > 150, "most of the random programs are valid: {compiled} of 400");
}

#[test]
fn an_evolve_acts_on_the_hits_chance_kept_and_a_cycle_of_its_own_is_one_too() {
    let src = "section drop 8 bars {\n  deck A: hihat x*8?0.5 seed=2 | bass/12 c2 ~ ~ ~\n  evolve every 4 bars {\n    hihat: rotate 1\n    stop when gen == 1\n  }\n}\narrange drop\n";
    let c = ok(src);
    assert_eq!(c.segments.iter().filter(|s| s.name == "drop").count(), 2, "two generations of four bars");
    let g0 = &c.segments[0].decks[&Deck::A].parts.iter().find(|p| p.role == "hihat").unwrap().cycles;
    let g1 = &c.segments[1].decks[&Deck::A].parts.iter().find(|p| p.role == "hihat").unwrap().cycles;
    // generation 1 is generation 0's kept hits rotated a step: no hit the toss dropped comes back
    let count = |c: &Vec<Vec<djlive::compile::Hit>>| c.iter().take(4).map(Vec::len).sum::<usize>();
    assert_eq!(count(g0), count(g1));
}
