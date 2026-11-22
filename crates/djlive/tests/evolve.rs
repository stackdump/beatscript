//! `evolve` (docs/BTW.md, "Evolving"): a program that applies transforms to itself over time. Generation n is
//! the rule applied n times to generation 0; every generation has a CID of its nets and records the one it
//! came from; the bound is in the text; the same text is the same generations every time.

use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::share;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errors(src: &str) -> Vec<String> {
    let c = check(src);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

const SONG: &str = "\
tempo 124
key am
section drop 32 bars {
  deck A: kick x ~ x ~ | hihat x*8
  deck B: melody [a4 c5 e5 g5] | bass [a2 ~ e2 ~]
  evolve every 8 bars {
    melody: rotate 1 | transpose <0 5 7 0>
    hihat: thin 0.1 seed=3
    bass: invert when gen % 2 == 1
    stop when gen == 3
  }
}
arrange drop
";

fn notes(c: &Compiled, seg: usize, deck: Deck, role: &str) -> Vec<(u8, u8)> {
    c.segments[seg].decks[&deck].parts.iter().find(|p| p.role == role).unwrap().cycles[0].iter().map(|h| (h.step, h.note)).collect()
}

#[test]
fn a_section_that_evolves_is_one_segment_a_generation() {
    let c = ok(&SONG);
    assert_eq!(c.segments.len(), 4, "gen 0 to 3, eight bars each");
    assert_eq!(c.segments.iter().map(|s| (s.start, s.bars, s.gen)).collect::<Vec<_>>(), vec![(0, 8, Some(0)), (8, 8, Some(1)), (16, 8, Some(2)), (24, 8, Some(3))]);
    assert_eq!(c.song_bars(), Some(32));
    // an evolving section is still one section to the window: its bar counts through the generations
    assert_eq!(c.section_at(17), Some(("drop", 17, 32)));
    assert_eq!(c.generation_at(17), Some(2));
    assert_eq!(c.lineage.len(), 4);
}

#[test]
fn the_rules_are_applied_in_order_a_generation_at_a_time() {
    let c = ok(&SONG);
    // gen 0: a4 c5 e5 g5 on steps 0 4 8 12
    assert_eq!(notes(&c, 0, Deck::B, "melody"), vec![(0, 69), (4, 72), (8, 76), (12, 79)]);
    // gen 1: rotated one step, transposed by the first of <0 5 7 0> (0)
    assert_eq!(notes(&c, 1, Deck::B, "melody"), vec![(1, 69), (5, 72), (9, 76), (13, 79)]);
    // gen 2: rotated again, up 5
    assert_eq!(notes(&c, 2, Deck::B, "melody"), vec![(2, 74), (6, 77), (10, 81), (14, 84)]);
    // gen 3: rotated again, up 7
    assert_eq!(notes(&c, 3, Deck::B, "melody"), vec![(3, 81), (7, 84), (11, 88), (15, 91)]);
    // bass is inverted about its first note, but only in odd generations: gen 1 turned over, gen 2 back, gen 3 over
    let b0 = notes(&c, 0, Deck::B, "bass");
    let b1 = notes(&c, 1, Deck::B, "bass");
    let b2 = notes(&c, 2, Deck::B, "bass");
    assert_eq!(b0, vec![(0, 45), (8, 40)]);
    assert_eq!(b1, vec![(0, 45), (8, 50)]);
    assert_eq!(b2, b1, "the guard `gen % 2 == 1` is false in gen 2: the rule does not run");
    assert_eq!(notes(&c, 3, Deck::B, "bass"), vec![(0, 45), (8, 40)]);
}

#[test]
fn thinning_is_seeded_and_only_ever_takes_hits_away() {
    let c = ok(&SONG);
    let hits = |seg: usize| c.segments[seg].decks[&Deck::A].parts.iter().find(|p| p.role == "hihat").unwrap().cycles.iter().map(Vec::len).sum::<usize>();
    let counts: Vec<usize> = (0..4).map(hits).collect();
    assert_eq!(counts[0], 8);
    assert!(counts.windows(2).all(|w| w[1] <= w[0]) && counts[3] < counts[0], "{counts:?}");
    // another seed thins other hits
    let other = ok(&SONG.replace("seed=3", "seed=4"));
    assert_ne!(c.lineage.iter().map(|g| g.cid.clone()).collect::<Vec<_>>(), other.lineage.iter().map(|g| g.cid.clone()).collect::<Vec<_>>());
    // the kick is no one's business
    assert_eq!(notes(&c, 0, Deck::A, "kick"), notes(&c, 3, Deck::A, "kick"));
}

#[test]
fn the_cid_of_a_generation_is_stable_across_runs_and_formatting() {
    let a = ok(&SONG);
    let b = ok(&SONG);
    assert_eq!(a.lineage, b.lineage);
    let spaced = SONG.replace("rotate 1 | transpose <0 5 7 0>", "rotate   1|transpose <0   5 7 0>   # a comment").replace("thin 0.1", "thin 0.10");
    let c = ok(&spaced);
    assert_eq!(c.cid, a.cid, "the program is the same program");
    assert_eq!(c.lineage, a.lineage);
    // each generation's CID is its own
    let mut cids: Vec<&String> = a.lineage.iter().map(|g| &g.cid).collect();
    cids.sort();
    cids.dedup();
    assert_eq!(cids.len(), 4);
    assert!(a.lineage.iter().all(|g| g.cid.starts_with('z')));
}

#[test]
fn each_generation_records_the_one_it_came_from() {
    let c = ok(&SONG);
    assert_eq!(c.lineage[0].forked_from, None);
    for w in c.lineage.windows(2) {
        assert_eq!(w[1].forked_from.as_deref(), Some(w[0].cid.as_str()), "gen {} forks gen {}", w[1].gen, w[0].gen);
    }
    assert_eq!(c.lineage.iter().map(|g| g.gen).collect::<Vec<_>>(), [0, 1, 2, 3]);
}

#[test]
fn an_identity_rule_is_a_fixed_point_with_the_same_cid_every_generation() {
    let src = "section loopy 24 bars {\n  deck A: bass [a2 e2 c2 g2] | hihat x*8\n  evolve every 4 bars {\n    bass: transpose 0 | rotate 0\n    hihat: reverse | reverse\n    stop when gen == 5\n  }\n}\narrange loopy\n".replace("\\\n", "\n");
    let c = ok(&src.replace("\\\n", "\n"));
    assert_eq!(c.lineage.len(), 6);
    let first = &c.lineage[0].cid;
    assert!(c.lineage.iter().all(|g| &g.cid == first), "{:?}", c.lineage.iter().map(|g| g.cid.clone()).collect::<Vec<_>>());
    // the nets are the same net, so the segments are too
    let nets: Vec<_> = c.segments.iter().map(|s| s.decks[&Deck::A].net_cid()).collect();
    assert!(nets.windows(2).all(|w| w[0] == w[1]));
    // and a generation forks itself: the chain is a loop on one CID
    assert!(c.lineage[1..].iter().all(|g| g.forked_from.as_ref() == Some(first)));
}

#[test]
fn a_program_with_no_sections_evolves_on_its_own_timeline_and_ends() {
    let src = "deck A: bass [a2 e2 c2 g2]\nevolve every 4 bars {\n  bass: transpose 2\n  for 3 gens\n}\n".replace("\\\n", "\n");
    let c = ok(&src.replace("\\\n", "\n"));
    assert!(c.arranged);
    assert_eq!(c.lineage.len(), 4);
    assert_eq!(c.song_bars(), Some(16), "gen 0 and three more, four bars each");
    assert_eq!(c.end, Some(16), "it ends where the evolution does");
    assert_eq!(notes(&c, 3, Deck::A, "bass").iter().map(|n| n.1).collect::<Vec<_>>(), vec![45 + 6, 40 + 6, 36 + 6, 43 + 6]);
}

#[test]
fn the_bound_is_required_and_the_rest_is_checked() {
    // `stop` is optional: a section is bounded by its own length, a program by the orbit of its parts. An orbit too
    // long to play through is refused with the way out
    let e = errors("deck A: bass [a2 e2 c2 g2] | hihat x*8\nevolve every 1 bars {\n  bass: rotate 1 | transpose <0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40>\n  hihat: rotate 3 when gen % 7 != 0\n}\n");
    assert!(e.iter().any(|m| m.contains("no `stop`") && m.contains("too long")), "{e:?}");
    let e = errors("section sd 8 bars {\n  deck A: bass x\n  evolve every 2 bars {\n    bass: rotat 1\n    stop when gen == 2\n  }\n}\n");
    assert!(e.iter().any(|m| m.contains("not a transform")), "{e:?}");
    assert!(errors("section sd 8 bars {\n  deck A: bass x\n  evolve every 2 bars {\n    melody: rotate 1\n    stop when gen == 2\n  }\n}\n").iter().any(|m| m.contains("no `melody` part here")));
    assert!(errors("section sd 8 bars {\n  deck A: kick x\n  evolve every 2 bars {\n    kick: transpose 5\n    stop when gen == 2\n  }\n}\n").iter().any(|m| m.contains("pitches")));
    assert!(errors("section sd 8 bars {\n  deck A: bass x\n  evolve every 2 bars {\n    bass: rotate 1\n    stop when gen == 500\n  }\n}\n").iter().any(|m| m.contains("generations")));
    // an evolve in a song belongs in its section
    assert!(errors("section sd 8 bars {\n  deck A: bass x\n}\nevolve every 2 bars {\n  bass: rotate 1\n  stop when gen == 2\n}\n").iter().any(|m| m.contains("inside the section")));
    // `stop when gen < 3` says what it can mean
    assert!(!errors("deck A: bass x\nevolve every 2 bars {\n  bass: rotate 1\n  stop when gen < 3\n}\n").is_empty());
}

#[test]
fn the_canonical_text_keeps_the_rule_and_the_link_replays_every_generation() {
    let c = ok(&SONG);
    assert!(c.text.contains("evolve every 8 bars {\n    melody: rotate 1 | transpose <0 5 7 0>\n    hihat: thin 0.1 seed=3\n    bass: invert when gen % 2 == 1\n    stop when gen == 3\n  }"), "{}", c.text);
    let again = ok(&c.text);
    assert_eq!(again.cid, c.cid, "the canonical text is a fixed point");
    // the link carries gen 0 and the rule: opening it reproduces every generation
    let url = share::encode(&c, None, false);
    let opened = share::open(&url, &djlive::env::Env::none()).unwrap();
    let replay = opened.check.compiled.unwrap();
    assert_eq!(replay.lineage, c.lineage);
    assert!(url.len() < 2000, "{}", url.len());
}

#[test]
fn stop_is_optional_a_section_is_bounded_by_its_length_and_a_program_by_its_orbit() {
    // a section: as many generations as fit
    let c = ok("section sx 20 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 4 bars {\n    bass: transpose 1\n  }\n}\narrange sx\n");
    assert_eq!(c.lineage.len(), 5, "generations 0 to 4: five stretches of four bars");
    assert_eq!(c.song_bars(), Some(20));
    // a program of its own: the orbit. Rotating a bar of sixteen steps comes round in 16 generations
    let c = ok("deck A: bass [a2 e2 c2 g2]\nevolve every 2 bars {\n  bass: rotate 1\n}\n");
    let run = &c.evolves[0];
    assert_eq!((run.analysis.tail, run.analysis.period), (0, Some(16)));
    assert_eq!(run.last, 16, "it plays the orbit through, one generation more to show it come back");
    assert_eq!(c.song_bars(), Some(17 * 2));
    assert_eq!(c.end, Some(34));
    // it came back: generation 16 is generation 0
    assert_eq!(c.lineage[16].cid, c.lineage[0].cid);
}

#[test]
fn the_report_gives_the_tail_and_the_period_in_bars_and_time() {
    let c = ok("deck A: bass [a2 e2 c2 g2] | hihat x*8\nevolve every 2 bars {\n  bass: rotate 1\n  hihat: thin 0.5 seed=1\n  stop when gen == 40\n}\n");
    let r = c.evolve_report();
    assert_eq!(r.len(), 1);
    // rotate's period is 16 generations, thin settles into a fixed point: the program's is 16 generations = 32 bars
    assert!(r[0].contains("period 16 generations = period 32 bars"), "{r:?}");
    assert!(r[0].contains("≈ 1m04s at 120"), "{r:?}");
    // the same sentence for a long period, as in the sketch: 16,016 bars at 124 BPM
    assert_eq!(djlive::evolve::group(16016), "16,016");
    assert_eq!(djlive::evolve::human(16016.0 * 4.0 * 60.0 / 124.0), "8h36m");
    assert_eq!(djlive::evolve::group(7), "7");
    assert_eq!(djlive::evolve::human(124.0), "2m04s");
}

#[test]
fn the_parts_orbits_combine_by_least_common_multiple() {
    // bass comes round in 16 generations (a rotation over sixteen steps), melody in 4 (an alternation <0 12 0 -12>
    // brings the notes home), together 16; add a part that comes round in 3 and it is 48
    let c = ok("deck A: bass [a2 e2 c2 g2] | hihat x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\ndeck B: melody [a4 c5]\nevolve every 1 bars {\n  bass: rotate 1\n  melody: transpose <0 12 0 -12>\n  hihat: rotate 3 when gen % 3 != 3\n  stop when gen == 20\n}\n");
    let a = &c.evolves[0].analysis;
    let by_role = |r: &str| a.parts.iter().find(|o| o.role == r).unwrap();
    assert_eq!(by_role("bass").period, 16);
    assert_eq!(by_role("melody").period, 4);
    // a pattern of hits on every other step comes back after every second rotation by 3 (6 is an even shift),
    // and the guard `gen % 3` is part of the rule's clock: the part and its clock come round together in 6
    assert_eq!(by_role("hihat").period, 6);
    assert_eq!(a.period, Some(48), "lcm(16, 4, 6)");
}

#[test]
fn any_generations_cid_comes_from_generation_0_and_the_rule_alone() {
    let c = ok(&SONG);
    // the ones on the timeline
    for g in 0..4 {
        assert_eq!(c.generation_cid("drop", g).as_deref(), Some(c.lineage[g as usize].cid.as_str()), "gen {g}");
    }
    // far past the end, against walking every step by hand
    for rule_src in [SONG.to_string(), SONG.replace("rotate 1 | transpose <0 5 7 0>", "rotate 3").replace("thin 0.1 seed=3", "thin 0.02 seed=9")] {
        let c = ok(&rule_src);
        let run = &c.evolves[0];
        for n in [4u32, 17, 100, 1000] {
            let mut cur = run.gen0.clone();
            for g in 1..=n {
                cur = djlive::evolve::step(&run.block, &cur, g);
            }
            assert_eq!(c.generation_cid("drop", n), Some(djlive::evolve::generation_cid(&cur)), "generation {n} by walking and by the orbit");
        }
        // a million costs no more than its orbit
        assert!(c.generation_cid("drop", 1_000_000).is_some());
    }
    assert_eq!(c.generation_cid("nowhere", 3), None);
}
