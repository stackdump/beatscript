//! F6, Euclidean rhythms and polymeter (docs/btw/ext-meter.md): `e(k, n, rot)` is checked and named by the ontology,
//! `role/N` gives a part a cycle of its own that drifts against the bar, and the program's period is reported.

mod common;

use common::{reformat, Rng};
use djengine::gen::euclidean::{bjorklund, rotated};
use djlive::ast::Deck;
use djlive::compile::{check, Compiled, MAX_BARS};
use djlive::diag::{Diagnostic, Severity};
use djlive::meter;

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

/// The steps (0..16) hit in each bar of a net's part.
fn bars_of(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<u8>> {
    let ir = &c.decks[&deck];
    ir.parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role}")).cycles.iter().map(|b| b.iter().map(|h| h.step).collect()).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// e(k, n, rot)

#[test]
fn a_euclidean_term_is_a_pattern_in_its_own_right_three_in_eight_is_the_tresillo() {
    let c = ok("deck A: kick e(3,8)\n");
    // eight steps over the bar: x . . x . . x .  is sixteenths 0, 6, 12
    assert_eq!(bars_of(&c, Deck::A, "kick"), vec![vec![0, 6, 12]]);
    // the same, rotated by two steps, is the same three hits turned
    let r = ok("deck A: kick e(3,8,2)\n");
    let want: Vec<u8> = {
        let mut v: Vec<u8> = rotated(&bjorklund(3, 8), 2).iter().enumerate().filter(|(_, x)| **x == 1).map(|(i, _)| (i * 2) as u8).collect();
        v.sort();
        v
    };
    assert_eq!(bars_of(&r, Deck::A, "kick"), vec![want]);
}

#[test]
fn every_k_n_and_any_rotation_is_the_engines_bjorklund_rotated() {
    for n in 1..=16i64 {
        for k in 0..=n {
            for rot in [-40, -17, -3, -1, 0, 1, 2, 5, 15, 16, 33, 1000] {
                let src = format!("deck A: kick e({k},{n},{rot})\n");
                let c = ok(&src);
                // n steps over the bar, at sixteenth `i * 16 / n` (n not a divisor of 16 snaps; compare on the grid it lands on)
                let pat = rotated(&bjorklund(k, n), rot);
                let mut want: Vec<u8> = pat.iter().enumerate().filter(|(_, v)| **v == 1).map(|(i, _)| (((i as f64 / n as f64) * 16.0 + 0.5 + 1e-9).floor() as i64 % 16) as u8).collect();
                want.sort();
                want.dedup();
                assert_eq!(bars_of(&c, Deck::A, "kick")[0], want, "{src}");
            }
        }
    }
}

#[test]
fn a_rotation_is_any_whole_number_and_turns_round() {
    let a = ok("deck A: kick e(3,8,2)\n");
    for rot in ["10", "-6", "18", "-14", "1002"] {
        let b = ok(&format!("deck A: kick e(3,8,{rot})\n"));
        assert_eq!(bars_of(&a, Deck::A, "kick"), bars_of(&b, Deck::A, "kick"), "rot {rot}");
    }
    // the texts differ (so the programs do): a rotation is kept as it was typed
    assert_ne!(a.cid, ok("deck A: kick e(3,8,10)\n").cid);
}

#[test]
fn zero_hits_and_all_hits_are_fine() {
    assert_eq!(bars_of(&ok("deck A: kick e(0,8)\n"), Deck::A, "kick"), vec![Vec::<u8>::new()]);
    assert_eq!(bars_of(&ok("deck A: kick e(8,8)\n"), Deck::A, "kick"), vec![(0..8).map(|i| i * 2).collect::<Vec<u8>>()]);
    assert_eq!(bars_of(&ok("deck A: kick e(1,1)\n"), Deck::A, "kick"), vec![vec![0u8]]);
}

#[test]
fn a_euclidean_term_that_cannot_be_is_an_error_at_its_place_with_the_way_out() {
    let e = errors("deck A: kick e(9,8)\n");
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("9 hits do not fit in 8 steps"), "{}", e[0].message);
    assert!(e[0].help.as_deref().unwrap().contains("e(8,8)"), "{:?}", e[0].help);
    let src = "deck A: snare ~ x ~ x | kick e(9,8)\n";
    let e = errors(src);
    let (line, col, _) = e[0].position(src);
    assert_eq!((line, col), (1, src.find("e(9,8)").unwrap() + 1), "the error is at the call e(9,8)");

    let e = errors("deck A: kick e(3,0)\n");
    assert!(e[0].message.contains("the step count is a whole number from 1 to 64"), "{}", e[0].message);
    let e = errors("deck A: kick e(-1,8)\n");
    assert!(e[0].message.contains("the hit count is a whole number from 0 to 64"), "{}", e[0].message);
    let e = errors("deck A: kick e(3,65)\n");
    assert!(e[0].message.contains("the step count"), "{}", e[0].message);
    let e = errors("deck A: kick e(2.5,8)\n");
    assert!(e[0].message.contains("the hit count"), "{}", e[0].message);
    let e = errors("deck A: kick e(3,8,1.5)\n");
    assert!(e[0].message.contains("the rotation is a whole number"), "{}", e[0].message);
    let e = errors("deck A: kick e(3)\n");
    assert!(e[0].message.contains("e() takes 2 to 3"), "{}", e[0].message);
    assert!(e[0].help.as_deref().unwrap().contains("e(5,8)"), "{:?}", e[0].help);
    let e = errors("deck A: kick e(3,8,1,1)\n");
    assert!(e[0].message.contains("e() takes 2 to 3, and this has 4"), "{}", e[0].message);
    let e = errors("deck A: kick e(a,8)\n");
    assert!(e[0].message.contains("expected the hit count"), "{}", e[0].message);
}

#[test]
fn euclid_is_another_spelling_of_e_and_prints_as_e() {
    let a = ok("deck A: kick euclid(3,8)\n");
    let b = ok("deck A: kick e(3,8)\n");
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.text, "deck A: kick e(3, 8)\n");
}

// ---------------------------------------------------------------------------------------------------------------
// the ontology

#[test]
fn the_table_is_the_ontologys_fifty_seven_rhythms_and_well_formed() {
    assert_eq!(meter::EUCLID_TABLE.len(), 57);
    for t in meter::EUCLID_TABLE.iter() {
        assert!(t.iri.starts_with(meter::RHYTHM_NS), "{}", t.iri);
        assert!(t.iri.ends_with(&format!("{}-{}", t.label.split(' ').next().unwrap(), t.role)), "{} / {}", t.iri, t.label);
        assert!(t.pulses <= t.steps && t.steps >= 1 && t.rotation < t.steps, "{t:?}");
        assert!(matches!(t.role, "kick" | "snare" | "hihat"), "{t:?}");
        assert!(t.label.ends_with(t.role), "{t:?}");
        // the term is a rhythm the engine can make
        assert_eq!(bjorklund(i64::from(t.pulses), i64::from(t.steps)).iter().filter(|v| **v == 1).count(), t.pulses as usize);
    }
    // no IRI twice
    let mut iris: Vec<&str> = meter::EUCLID_TABLE.iter().map(|t| t.iri).collect();
    iris.sort();
    iris.dedup();
    assert_eq!(iris.len(), 57);
    assert_eq!(meter::euclid_class_iri(), "https://ontology.bitwrap.io/ns#EuclideanRhythm");
    assert_eq!(meter::ONTOLOGY_NS, "https://ontology.bitwrap.io/ns#");
}

#[test]
fn a_rhythm_is_named_by_its_pulses_steps_and_rotation() {
    let t = meter::euclid_name(3, 8, 0).unwrap();
    assert_eq!((t.label, t.role, t.iri), ("house snare", "snare", "https://ontology.bitwrap.io/rhythm/house-snare"));
    assert_eq!((t.pulses, t.steps, t.rotation, t.midi_note), (3, 8, 0, 38));
    assert_eq!(t.parameters(), "E(3,8) rotation 0");
    // the rotation counts modulo the steps, as the engine's own does
    assert_eq!(meter::euclid_name(2, 16, 4 + 16), meter::euclid_name(2, 16, 4));
    assert_eq!(meter::euclid_name(2, 16, 4 - 16), meter::euclid_name(2, 16, 4));
    // several genres share a rhythm: all of them, in the ontology's order
    let all = meter::euclid_names(4, 16, 0);
    let labels: Vec<&str> = all.iter().map(|t| t.label).collect();
    assert_eq!(labels, vec!["country kick", "edm kick", "house kick", "synthwave kick", "techno kick", "trance kick"]);
    // by role
    assert_eq!(meter::euclid_names_for_role("kick", 4, 16, 0).len(), 6);
    assert!(meter::euclid_names_for_role("snare", 4, 16, 0).is_empty());
    assert_eq!(meter::euclid_names_for_role("snare", 4, 16, 4).iter().map(|t| t.label).collect::<Vec<_>>(), vec!["country snare", "funk snare", "metal snare"]);
    // not named
    assert!(meter::euclid_name(7, 5, 0).is_none());
    assert!(meter::euclid_name(3, 0, 0).is_none());
    assert!(meter::euclid_name(-1, 8, 0).is_none());
    assert!(meter::euclid_name(11, 16, 3).is_none());
}

#[test]
fn check_says_the_ontology_name_of_a_euclidean_term_for_its_role() {
    let n = notes("#! genre: techno\ndeck A: hihat e(5,8)\n");
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(n[0].starts_with("e(5,8): the ontology's hihat rhythm of lofi and techno"), "{}", n[0]);
    assert!(n[0].contains("https://ontology.bitwrap.io/rhythm/lofi-hihat"), "{}", n[0]);
    let n = notes("#! genre: techno\ndeck A: snare e(3,8)\n");
    assert_eq!(n.len(), 1);
    assert!(n[0].starts_with("e(3,8): the ontology's house snare rhythm (https://ontology.bitwrap.io/rhythm/house-snare)"), "{}", n[0]);
    // with a rotation, written as it is
    let n = notes("#! tags: acid\ndeck A: snare e(2,16,4)\n");
    assert!(n[0].starts_with("e(2,16,4): the ontology's snare rhythm of blues, dubstep, edm"), "{}", n[0]);
    // the same term for another role is not the ontology's rhythm for that role, and a tonal part has none
    assert!(notes("#! genre: techno\ndeck A: kick e(5,8)\n").is_empty());
    assert!(notes("#! genre: techno\ndeck A: bass e(3,8) 1 3 5\n").iter().all(|m| !m.starts_with("e(")));
    assert!(notes("#! genre: techno\ndeck A: clap e(3,8)\n").is_empty());
    // nested in a transform it is still found, once
    let n = notes("#! genre: techno\ndeck A: hihat reverse(e(5,8))\n");
    assert_eq!(n.len(), 1, "{n:?}");
    // a note, not an error: the program compiles
    assert!(check("#! genre: techno\ndeck A: hihat e(5,8)\n").ok());
    // a program that is not linked to the ontology (no `#! genre:` or `#! tags:` line) is not told: these were warnings on every
    // `e(4,16)` of every old program (R-20)
    assert!(notes("deck A: hihat e(5,8)\ndeck B: snare e(3,8)\n").is_empty());
    assert!(notes("# only a comment\n#! license: CC0-1.0\ndeck A: kick e(4,16)\n").is_empty());
}

// ---------------------------------------------------------------------------------------------------------------
// role/N

#[test]
fn a_part_with_a_cycle_of_twelve_drifts_across_the_barlines() {
    let c = ok("deck A: hihat/12 x x ~ x\n");
    // four terms share twelve steps: hits at cycle steps 0, 3, 9; the cycle starts again at step 12, 24, 36 ...
    // over lcm(12, 16) = 48 steps = 3 bars
    assert_eq!(c.decks[&Deck::A].period, 3);
    assert_eq!(bars_of(&c, Deck::A, "hihat"), vec![vec![0, 3, 9, 12, 15], vec![5, 8, 11], vec![1, 4, 7, 13]]);
    assert_eq!(c.decks[&Deck::A].parts[0].cycle_steps, 12);
}

#[test]
fn terms_share_the_cycle_equally_x_times_three_in_twelve_is_a_hit_every_four_steps() {
    let c = ok("deck A: hihat/12 x*3\n");
    assert_eq!(bars_of(&c, Deck::A, "hihat"), vec![vec![0, 4, 8, 12], vec![0, 4, 8, 12], vec![0, 4, 8, 12]]);
}

#[test]
fn a_euclidean_term_fills_a_cycle_of_n_steps_exactly() {
    let c = ok("deck A: bass/12 e(5,12)\n");
    let pat = bjorklund(5, 12);
    let steps: Vec<usize> = pat.iter().enumerate().filter(|(_, v)| **v == 1).map(|(i, _)| i).collect();
    assert_eq!(steps.len(), 5);
    let bars = bars_of(&c, Deck::A, "bass");
    assert_eq!(bars.len(), 3);
    // each hit of the 12-step ring is at its own step, each cycle 12 steps along
    let mut want = vec![Vec::<u8>::new(); 3];
    for k in 0..4 {
        for s in &steps {
            let abs = k * 12 + s;
            want[abs / 16].push((abs % 16) as u8);
        }
    }
    for w in &mut want {
        w.sort();
    }
    assert_eq!(bars, want);
}

#[test]
fn a_cycle_of_sixteen_is_the_bar_and_is_not_printed() {
    let a = ok("deck A: hihat/16 x ~ x ~\n");
    let b = ok("deck A: hihat x ~ x ~\n");
    assert_eq!(a.text, "deck A: hihat x ~ x ~\n");
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    assert_eq!(a.decks[&Deck::A].parts[0].cycle_steps, 16);
}

#[test]
fn a_cycle_of_its_own_is_part_of_the_program() {
    let a = ok("deck A: hihat/12 x x ~ x\n");
    let b = ok("deck A: hihat/9 x x ~ x\n");
    assert_ne!(a.cid, b.cid);
    assert_ne!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    assert_eq!(a.text, "deck A: hihat/12 x x ~ x\n");
}

#[test]
fn cycles_that_divide_the_bar_repeat_inside_it() {
    let c = ok("deck A: hihat/4 x ~\n");
    assert_eq!(c.decks[&Deck::A].period, 1);
    assert_eq!(bars_of(&c, Deck::A, "hihat"), vec![vec![0, 4, 8, 12]]);
    let c = ok("deck A: hihat/8 x x\n");
    assert_eq!(bars_of(&c, Deck::A, "hihat"), vec![vec![0, 4, 8, 12]]);
    let c = ok("deck A: hihat/1 x\n");
    assert_eq!(bars_of(&c, Deck::A, "hihat")[0].len(), 16);
    // a longer cycle than the bar: 32 steps is two bars
    let c = ok("deck A: bass/32 c2 ~ ~ c2\n");
    assert_eq!(c.decks[&Deck::A].period, 2);
    assert_eq!(bars_of(&c, Deck::A, "bass"), vec![vec![0], vec![8]]);
}

#[test]
fn a_cycle_is_unrolled_over_lcm_of_n_and_sixteen_steps_whatever_n_and_the_pattern() {
    let mut rng = Rng(0x00c0_ffee_1234_5678);
    for round in 0..300 {
        let n = 1 + rng.below(64) as usize;
        // a term count that divides n, so nothing snaps
        let divisors: Vec<usize> = (1..=n).filter(|d| n % d == 0 && *d <= 16).collect();
        let m = divisors[rng.below(divisors.len() as u64) as usize];
        let terms: Vec<bool> = (0..m).map(|_| rng.below(2) == 0).collect();
        let text: Vec<&str> = terms.iter().map(|t| if *t { "x" } else { "~" }).collect();
        let src = format!("deck A: hihat/{n} {}\n", text.join(" "));
        let c = ok(&src);
        let l = lcm(n, 16);
        let bars = l / 16;
        assert_eq!(c.decks[&Deck::A].period, bars, "{src}");
        let mut want = vec![Vec::<u8>::new(); bars];
        for k in 0..l / n {
            for (j, on) in terms.iter().enumerate() {
                if *on {
                    let abs = k * n + j * (n / m);
                    want[abs / 16].push((abs % 16) as u8);
                }
            }
        }
        for w in &mut want {
            w.sort();
            w.dedup();
        }
        assert_eq!(bars_of(&c, Deck::A, "hihat"), want, "round {round}: {src}");
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}
fn lcm(a: usize, b: usize) -> usize {
    a / gcd(a, b) * b
}

#[test]
fn chords_generators_and_notes_work_inside_a_cycle_of_their_own() {
    // chord() puts its notes together at the cycle's step
    let c = ok("key am\ndeck A: harmony/12 chord(am) ~\n");
    let h = &c.decks[&Deck::A].parts[0].cycles;
    // two terms in twelve steps: a chord at 0, 12, 24, ...; each time three notes at once
    assert_eq!(h.len(), 3);
    let at = |bar: usize, step: u8| h[bar].iter().filter(|x| x.step == step).count();
    assert_eq!(at(0, 0), 3);
    assert_eq!(at(0, 12), 3);
    assert_eq!(at(1, 8), 3, "step 24 is bar 1 step 8");
    assert_eq!(at(2, 4), 3, "step 36 is bar 2 step 4");
    assert_eq!(h.iter().map(Vec::len).sum::<usize>(), 12);
    // an arp and a degree are read the same way
    let c = ok("key am\ndeck A: melody/12 arp(am7, up, 4)\n");
    // four steps a cycle, four cycles in 48 steps
    assert_eq!(c.decks[&Deck::A].parts[0].cycles.iter().map(Vec::len).sum::<usize>(), 16);
    // an alternation counts the part's own cycles: <a b> over 12 steps is 24 steps, 48 with the bar: 3 bars
    let c = ok("deck A: bass/12 <c2 e2>\n");
    assert_eq!(c.decks[&Deck::A].period, 3);
    let notes: Vec<(usize, u8, u8)> = c.decks[&Deck::A].parts[0].cycles.iter().enumerate().flat_map(|(b, v)| v.iter().map(move |h| (b, h.step, h.note))).collect();
    // cycle 0 plays c2 (36) at step 0, cycle 1 e2 (40) at step 12, cycle 2 c2 at 24 = bar 1 step 8, cycle 3 e2 at 36 = bar 2 step 4
    assert_eq!(notes, vec![(0, 0, 36), (0, 12, 40), (1, 8, 36), (2, 4, 40)]);
    // and an alternation of three against twelve: lcm(36, 16) = 144 steps = 9 bars
    assert_eq!(ok("deck A: bass/12 <c2 e2 g2>\n").decks[&Deck::A].period, 9);
}

#[test]
fn a_part_with_a_cycle_and_a_bar_part_share_one_net() {
    let c = ok("deck A: kick x ~ x ~ | hihat/12 x x ~ x | bass/7 c2\n");
    let ir = &c.decks[&Deck::A];
    // lcm(3, 1, 7) = 21 bars
    assert_eq!(ir.period, 21);
    let kick = bars_of(&c, Deck::A, "kick");
    assert_eq!(kick.len(), 21);
    assert!(kick.iter().all(|b| *b == vec![0, 8]));
    // the bass: every seventh step across 21 bars = 48 hits
    let bass: usize = bars_of(&c, Deck::A, "bass").iter().map(Vec::len).sum();
    assert_eq!(bass, 21 * 16 / 7);
}

#[test]
fn the_same_part_text_is_the_same_net_whatever_the_order_and_spacing() {
    let a = ok("deck A: kick x ~ x ~ | hihat/12 x x ~ x\n");
    let b = ok("deck A:  hihat/12   x x ~ x\ndeck A: kick x ~ x ~\n");
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
}

#[test]
fn the_cycle_must_be_glued_to_its_role_and_a_number_of_steps() {
    for (src, want) in [
        ("deck A: hihat /12 x x\n", "glued to its role"),
        ("deck A: hihat/ 12 x x\n", "glued to its role"),
        ("deck A: hihat/0 x x\n", "a cycle is a whole number of sixteenth steps from 1 to 64"),
        ("deck A: hihat/65 x x\n", "from 1 to 64"),
        ("deck A: hihat/12.5 x x\n", "a whole number"),
        ("deck A: hihat/x x x\n", "the cycle's length in steps"),
        ("deck A: hihat/ x x\n", "the cycle's length in steps"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|d| d.message.contains(want)), "{src}: {:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert!(e.iter().all(|d| d.help.is_some() || !d.expected.is_empty() || d.message.contains("pattern")), "{src}: every error says what to write");
    }
    // the error is at the number
    let src = "deck A: hihat/99 x x\n";
    let e = errors(src);
    let (line, col, _) = e[0].position(src);
    assert_eq!((line, col), (1, 15));
}

#[test]
fn a_role_may_have_only_one_part_on_a_deck_whatever_its_cycle() {
    let e = errors("deck A: hihat/12 x x\ndeck A: hihat x ~\n");
    assert!(e.iter().any(|d| d.message.contains("already has a `hihat` part")), "{e:?}");
}

#[test]
fn a_cycle_comes_through_lets_defs_repeats_and_sections() {
    let src = "let pat = x x ~ x\ndef groove(n) {\n  deck A: hihat/$n pat\n}\ngroove(12)\n";
    let c = ok(src);
    assert_eq!(c.decks[&Deck::A].parts[0].cycle_steps, 12);
    assert_eq!(c.text, src);
    let plain = ok("deck A: hihat/12 [x x ~ x]\n");
    assert_eq!(c.decks[&Deck::A].parts[0].cycles, plain.decks[&Deck::A].parts[0].cycles);
    // a scalar let is put in place of the number
    let c = ok("let n = 12\ndeck A: hihat/n x x ~ x\n");
    assert_eq!(c.decks[&Deck::A].parts[0].cycle_steps, 12);
    // in a section: its nets are three bars, and the section plays them for its length
    let c = ok("section intro 6 bars {\n  deck A: hihat/12 x x ~ x\n}\narrange intro\n");
    assert_eq!(c.segments[0].decks[&Deck::A].period, 3);
    // a repeat's `$n` is the count: one copy, a cycle of one step
    let c = ok("repeat 1 {\n  deck A: hihat/$n x\n}\n");
    assert_eq!(c.decks[&Deck::A].parts[0].cycle_steps, 1);
}

#[test]
fn a_hit_in_a_cycle_of_its_own_still_knows_its_token() {
    let src = "deck A: hihat/12 x x ~ x\n";
    let c = ok(src);
    let part = &c.decks[&Deck::A].parts[0];
    let mut seen = 0;
    for bar in &part.cycles {
        for h in bar {
            let chain = &part.spans[h.sid as usize];
            assert!(!chain.is_empty());
            assert_eq!(chain[0].text(src), "x", "the innermost place is the token");
            seen += 1;
        }
    }
    assert_eq!(seen, 12);
}

#[test]
fn velocity_ranges_and_accents_work_across_the_barline() {
    let c = ok("deck A: hihat/12 x! x ~ x vel=60..90\n");
    let part = &c.decks[&Deck::A].parts[0];
    assert!(part.cycles.iter().flatten().all(|h| (60..=127).contains(&h.vel)));
    assert!(part.cycles.iter().flatten().any(|h| h.accent));
    // the same text is the same velocities
    assert_eq!(ok("deck A: hihat/12 x! x ~ x vel=60..90\n").decks[&Deck::A].net_cid(), c.decks[&Deck::A].net_cid());
}

// ---------------------------------------------------------------------------------------------------------------
// the program's period

#[test]
fn check_reports_the_period_in_bars_and_steps_as_a_note() {
    let n = notes("deck A: hihat/12 x x ~ x\n");
    assert_eq!(n, vec!["period: 3 bars = 48 steps (hihat/12 against the bar)".to_string()]);
    let n = notes("deck A: hihat/12 x | bass/7 c2\n");
    assert_eq!(n, vec!["period: 21 bars = 336 steps (hihat/12 and bass/7 against the bar and each other)".to_string()]);
    // a cycle that divides the bar is one bar
    let n = notes("deck A: hihat/8 x\n");
    assert_eq!(n, vec!["period: 1 bar = 16 steps (hihat/8 against the bar)".to_string()]);
    // none without a cycle of its own: existing programs say nothing new
    assert!(notes("deck A: hihat x*8 | kick x ~ x ~\n").is_empty());
    // a note is `info` in severity and does not stop the program
    let c = check("deck A: hihat/12 x x ~ x\n");
    assert!(c.ok());
    assert!(c.diagnostics.iter().all(|d| d.severity == Severity::Info));
    assert!(c.compiled.unwrap().warnings.iter().any(|d| d.message.starts_with("period:")));
    // and it points at the part with a cycle
    let src = "deck A: kick x ~ x ~ | hihat/12 x x ~ x\n";
    let d = check(src).diagnostics.into_iter().find(|d| d.message.starts_with("period:")).unwrap();
    assert_eq!(d.position(src).1, 24);
}

#[test]
fn the_period_is_the_least_common_multiple_across_decks_and_is_a_function_for_later_workers() {
    let c = ok("deck A: hihat/12 x x ~ x\ndeck B: bass/10 c2\n");
    // deck A: 3 bars; deck B: lcm(10, 16) / 16 = 80 / 16 = 5 bars; together 15
    let p = meter::period_of(&c.decks);
    assert_eq!(p.bars, 15);
    assert_eq!(p.steps, 240);
    assert_eq!(p.polymeter, vec!["hihat/12".to_string(), "bass/10".to_string()]);
    assert_eq!(p.text(), "15 bars = 240 steps");
    assert_eq!(meter::program_period(&c), vec![(String::new(), p)]);
    let n = notes("deck A: hihat/12 x x ~ x\ndeck B: bass/10 c2\n");
    assert_eq!(n, vec!["period: 15 bars = 240 steps (hihat/12 and bass/10 against the bar and each other)".to_string()]);
    // a program with no cycle of its own has the period of its alternations, as before
    let c = ok("deck A: bass <c2 e2 g2>\n");
    assert_eq!(meter::period_of(&c.decks).bars, 3);
    assert!(meter::period_of(&c.decks).polymeter.is_empty());
}

#[test]
fn a_song_has_a_period_for_each_section() {
    let src = "section intro 6 bars {\n  deck A: hihat/12 x x ~ x\n}\nsection drop 4 bars {\n  deck A: hihat/8 x\n}\narrange intro drop\n";
    let c = ok(src);
    let n = notes(src);
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n[0].starts_with("period: 3 bars = 48 steps (hihat/12 against the bar), in section intro"), "{}", n[0]);
    assert!(n[1].starts_with("period: 1 bar = 16 steps (hihat/8 against the bar), in section drop"), "{}", n[1]);
    let per = meter::program_period(&c);
    assert_eq!(per.iter().map(|(n, p)| (n.as_str(), p.bars)).collect::<Vec<_>>(), vec![("intro", 3), ("drop", 1)]);
}

#[test]
fn a_period_over_a_net_is_an_error_naming_the_parts_and_the_way_out() {
    let e = errors("deck A: hihat/63 x | bass/62 c2 | kick/61 x\n");
    assert_eq!(e.len(), 1);
    let m = &e[0].message;
    assert!(m.contains("deck A's parts take 119133 bars to come round"), "{m}");
    assert!(m.contains("hihat/63 (63 bars)") && m.contains("bass/62 (31 bars)") && m.contains("kick/61 (61 bars)"), "{m}");
    assert!(m.contains(&format!("a net is at most {MAX_BARS}")), "{m}");
    assert!(e[0].help.as_deref().unwrap().contains("share a factor"), "{:?}", e[0].help);
    // exactly 64 bars is fine: /1 .. /63 alone are at most 63; two coprime ones are not
    assert!(check("deck A: hihat/63 x\n").ok());
    assert!(check("deck A: hihat/9 x | bass/7 c2\n").ok(), "63 bars");
    let e = errors("deck A: hihat/9 x | bass/11 c2\n");
    assert!(e[0].message.contains("99 bars"), "{}", e[0].message);
    // the same two on two decks are two nets of their own
    assert!(check("deck A: hihat/9 x\ndeck B: bass/11 c2\n").ok());
    // a program with no cycle of its own is not held to this: a long alternation chains, as before
    let long = format!("deck A: bass <{}>\n", (0..80).map(|_| "c2").collect::<Vec<_>>().join(" "));
    assert!(check(&long).ok());
}

#[test]
fn a_snapped_polymeter_term_says_which_cycle_it_could_not_divide() {
    let n = notes("deck A: hihat/7 x x x\n");
    assert!(n.iter().any(|m| m.contains("does not divide its 7-step cycle into whole steps")), "{n:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// the canonical form

const PROGRAMS: [&str; 5] = [
    "tempo 124\nkey am\ndeck A: kick e(4,16) | hihat/12 x x ~ x vel=60..90 | bass/7 c2 ~ e2 g2 oct=2\n",
    "deck A: snare e(3,8,2) | kick e(5,16,-3)\ndeck B: harmony/24 chord(am7) ~ ~ ~\n",
    "let pat = x x ~ x\ndef groove(n) {\n  deck A: hihat/$n pat\n}\ngroove(12)\n",
    "section intro 6 bars {\n  deck A: hihat/12 x x ~ x | kick e(3,8)\n}\nsection drop 4 bars {\n  deck A: hihat/8 x\n}\narrange intro drop\n",
    "deck A: bass/12 <c2 e2> | hihat/16 x*8\n",
];

#[test]
fn random_reformatting_keeps_the_cid_and_a_semantic_edit_changes_it() {
    for (n, src) in PROGRAMS.iter().enumerate() {
        let want = ok(src);
        let again = ok(&want.text);
        assert_eq!(again.text, want.text, "program {n}: the canonical text is a fixed point");
        assert_eq!(again.cid, want.cid);
        let mut rng = Rng(0x5eed_f006 + n as u64);
        for round in 0..120 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text))).cid;
            assert_eq!(got, want.cid, "program {n} round {round} forked:\n{text}");
        }
    }
    // a different cycle, a different rhythm, a different rotation are different programs
    let base = ok("deck A: hihat/12 x x ~ x | kick e(3,8)\n").cid;
    for edit in ["deck A: hihat/11 x x ~ x | kick e(3,8)\n", "deck A: hihat/12 x ~ x x | kick e(3,8)\n", "deck A: hihat/12 x x ~ x | kick e(3,8,1)\n", "deck A: hihat/12 x x ~ x | kick e(4,8)\n"] {
        assert_ne!(ok(edit).cid, base, "{edit}");
    }
}

#[test]
fn the_editor_colours_a_cycle_and_a_generator() {
    use djlive::highlight::{highlight, Class};
    let src = "deck A: hihat/12 x e(3,8)";
    let c: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    assert!(c.contains(&("hihat", Class::Role)));
    assert!(c.contains(&("12", Class::Number)));
    assert!(c.contains(&("e", Class::Generator)));
}
