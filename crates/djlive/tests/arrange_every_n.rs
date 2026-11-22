//! `~3` / `@4` (mutate.rs, docs/BTW.md "Arrange mutations"): a mutation line that fires only on every Nth pass of its
//! `arrange { }`, as the occurrences N, 2N, 3N ... It stays total and deterministic, and the generation table says which
//! plays each firing was read by.

use djlive::ast::StmtKind;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic, Severity};
use djlive::mutate::{fix, generations, ArrItem, GenTable, Mutation};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}\n{}", src, render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn table(src: &str) -> GenTable {
    generations(src, &djlive::env::Env::none()).expect("a generation table")
}

fn diags(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics
}

fn warnings(src: &str) -> Vec<String> {
    diags(src).into_iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message).collect()
}

/// A song whose three sections read the patterns: `block` is its `arrange`.
fn song(block: &str) -> String {
    format!(
        "tempo 120\nkey am\nlet acid = a2 c3 e3 g3 a3 g3 e3 c3\nlet hoov = e4 ~ g4 ~\nlet kik = x ~ x ~\n\
         section intro 2 bars {{\n  deck A: kick kik\n}}\n\
         section drop 2 bars {{\n  deck A: kick kik\n  deck B: bass acid | melody hoov\n}}\n\
         section outro 2 bars {{\n  deck A: kick kik\n  deck B: bass acid\n}}\n{block}\n"
    )
}

/// The block's mutation lines, as the parser reads them.
fn mutations(src: &str) -> Vec<Mutation> {
    let p = djlive::parse::parse(src);
    assert!(p.ok(), "{}", render_all(&p.diagnostics, src));
    p.program
        .stmts
        .iter()
        .filter_map(|s| if let StmtKind::ArrangeBlock(b) = &s.kind { Some(b.items.clone()) } else { None })
        .flatten()
        .filter_map(|i| if let ArrItem::Mutation(m) = i { Some(m) } else { None })
        .collect()
}

/// The loop the tests turn: the arp-like `acid` turned round on every 3rd pass, `hoov` up on every pass (so a line after a
/// line that does not fire is seen to roll its own dice), six passes and a coda.
const SIX: &str = "arrange {\n  intro drop\n  ~3 acid rev\n  ~ hoov up 1\n  drop\n} loop 6 then outro end";

#[test]
fn the_count_follows_the_mark_spaced_or_glued_and_one_is_no_count() {
    let m = |line: &str| mutations(&song(&format!("arrange {{\n  intro drop\n  {line}\n  drop\n}} loop 6 then end")));
    let glued = &m("~3 acid rev")[0];
    let spaced = &m("~ 3 acid rev")[0];
    assert_eq!((glued.every, glued.once), (3, false));
    assert_eq!((spaced.every, spaced.once), (3, false));
    assert_eq!(glued.every_span.unwrap().text(&song("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} loop 6 then end")), "3");
    let once = &m("@4 kik double")[0];
    assert_eq!((once.every, once.once), (4, true));
    // no count and `1` are the same: every pass
    assert_eq!(m("~ acid rev")[0].every, 1);
    assert_eq!(m("~1 acid rev")[0].every, 1);
    assert_eq!(m("~ acid rev")[0].every_span, None);
    // a count is for the whole line: both targets
    assert_eq!(m("~2 acid rev, hoov up 1")[0].targets.len(), 2);
    // the largest a loop can reach
    assert_eq!(m("~256 acid rev")[0].every, 256);
}

#[test]
fn the_canonical_text_glues_the_count_and_leaves_the_default_out() {
    let block = |line: &str| format!("arrange {{\n  intro drop\n  {line}\n  drop\n}} loop 6 then end");
    let a = ok(&song(&block("~3 acid rev")));
    let b = ok(&song(&block("~ 3 acid rev")));
    assert_eq!(a.cid, b.cid, "spaced or glued is one program");
    assert!(a.text.contains("\n  ~3 acid rev\n"), "{}", a.text);
    assert_eq!(b.text, a.text);
    // the canonical text is a program, and the same one
    assert_eq!(ok(&a.text).cid, a.cid);
    let once = ok(&song(&block("@ 4 kik double")));
    assert!(once.text.contains("\n  @4 kik double\n"), "{}", once.text);
    assert_eq!(ok(&once.text).cid, once.cid);
    // the count is the program: another count is another program; `~1` is no count, the program it always was
    assert_ne!(a.cid, ok(&song(&block("~2 acid rev"))).cid);
    let one = ok(&song(&block("~1 acid rev")));
    let none = ok(&song(&block("~ acid rev")));
    assert_eq!(one.cid, none.cid);
    assert!(one.text.contains("\n  ~ acid rev\n"), "{}", one.text);
    assert_ne!(one.cid, a.cid);
    // the lowering is not in the text
    assert!(!a.text.contains("acid@") && !a.text.contains("drop@"));
}

/// The song of `arrange_mutations.rs` (`~`, `@`, a glob, a coda; no count), the CID of which was pinned before `~N` existed.
const OLD_SONG: &str = "tempo 128\nkey am\nlet acid_a = a2 ~ c3 e3 ~ a2 g2 ~\nlet acid_b = a2 a2 ~ e3\nlet hoov_a = a3 ~ ~ c4\nlet hoov_b = e4 ~ g4 ~\nlet kik = x ~ x ~\n\
section intro 4 bars {\n  deck A: kick kik\n}\nsection drop 4 bars {\n  deck A: kick kik | bass acid_a\n  deck B: melody hoov_a | arp acid_b\n}\n\
section peak 4 bars {\n  deck B: melody hoov_b | bass acid_a\n}\nsection outro 4 bars {\n  deck A: kick kik\n}\n\
arrange {\n  intro drop\n  ~ acid_a rot 2, hoov_a up 3\n  drop\n  ~ acid_* drift 0.5\n  peak\n  @ hoov_b rev\n  peak outro\n} loop 2 then outro end\n";

/// Pinned before `~N` existed: the program of `arrange_mutations.rs` has the CID it had, and so do the two example
/// programs with mutation lines. (Every program in the repo keeps its CIDs: `btwapi`'s `cid_pins`.)
#[test]
fn programs_without_a_count_keep_their_cid() {
    assert_eq!(ok(OLD_SONG).cid, "z4EBG9j48vaqydR4nYH2fQYmfWqa2GXrJ9dbXGzwsJDzSey5Rey");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/");
    let read = |n: &str| std::fs::read_to_string(format!("{dir}{n}")).unwrap();
    assert_eq!(ok(&read("arrange-mutations.btw")).cid, "z4EBG9jCtKY4ioQ3TEJCQQmTTJZjugJRihCNbBxNJrBaWr1adMi");
    assert_eq!(ok(&read("named-arrange-mutations.btw")).cid, "z4EBG9jBgVnH5y3CBC2U72LiZ6FVxEuPVMtaGpoa2QcZesBzWFr");
}

#[test]
fn a_count_that_cannot_be_one_is_an_error_at_the_count() {
    // (the line, the count as written, what the message says)
    for (line, at, says) in [
        ("~0 acid rev", "0", "`~0` never fires"),
        ("@0 kik double", "0", "`@0` never fires"),
        ("~1.5 acid rev", "1.5", "counts whole passes"),
        ("~3k acid rev", "3k", "counts whole passes"),
        ("~257 acid rev", "257", "1 to 256"),
        ("~99999999999999999999 acid rev", "99999999999999999999", "1 to 256"),
    ] {
        let src = song(&format!("arrange {{\n  intro drop\n  {line}\n  drop\n}} loop 6 then end"));
        let d = diags(&src);
        let e = d.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("`{line}` was accepted"));
        assert!(e.message.contains(says), "`{line}`: {}", e.message);
        assert_eq!(e.span.text(&src), at, "`{line}` points at the count");
        assert!(e.help.is_some(), "`{line}`: says what would fix it");
    }
    // a count is not a target, and it comes before the first target only
    for line in ["~3", "~3 ", "~ acid rev, 3 hoov up 1", "~-3 acid rev", "~3s acid rev"] {
        let d = diags(&song(&format!("arrange {{\n  intro drop\n  {line}\n  drop\n}} loop 6 then end")));
        assert!(d.iter().any(|d| d.is_error() && d.message.contains("names the `let` pattern")), "`{line}`: {d:?}");
    }
}

#[test]
fn a_line_fires_on_the_passes_that_are_a_multiple_of_its_count() {
    let src = song(SIX);
    let t = table(&src);
    assert_eq!(t.passes, 6);
    // `~3`: the 3rd and 6th time round (passes 2 and 5, counted from 0)
    let acid = &t.patterns["acid"];
    assert_eq!(acid.iter().map(|g| g.pass).collect::<Vec<_>>(), [None, Some(2), Some(5)]);
    assert_eq!(acid.iter().map(|g| g.every).collect::<Vec<_>>(), [1, 3, 3]);
    assert!(acid[1..].iter().all(|g| g.mutation == Some(0) && g.ops == "rev"));
    assert_eq!(acid[0].text, "a2 c3 e3 g3 a3 g3 e3 c3");
    assert_eq!(acid[1].text, "c3 e3 g3 a3 g3 e3 c3 a2", "rev, once");
    assert_eq!(acid[2].text, "a2 c3 e3 g3 a3 g3 e3 c3", "rev again: back to the start");
    // a line after one that did not fire keeps its own index, so every generation of `hoov` names the line it came from
    let hoov = &t.patterns["hoov"];
    assert_eq!(hoov.iter().map(|g| g.pass).collect::<Vec<_>>(), [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]);
    assert!(hoov[1..].iter().all(|g| g.mutation == Some(1) && g.every == 1));
    // what the plays read: three plays a pass (intro, drop, drop), then the outro as the coda
    assert_eq!(t.plays.len(), 6 * 3 + 1);
    let acid_gen = |i: usize| t.plays[i].reads.iter().find(|r| r.pattern == "acid").map(|r| r.generation);
    assert_eq!(acid_gen(1), Some(0));
    assert_eq!(acid_gen(7), Some(0), "pass 2's first drop is before the line");
    assert_eq!(acid_gen(8), Some(1), "its second drop is the first to read it");
    assert_eq!(acid_gen(10), Some(1));
    assert_eq!(acid_gen(14), Some(1));
    assert_eq!(acid_gen(16), Some(1), "pass 5's first drop");
    assert_eq!(acid_gen(17), Some(2), "and the second, after the 6th occurrence");
    assert_eq!(acid_gen(18), Some(2), "the coda reads the last generation");
    // check's table: the passes it fired on and the first play that read each; only the counted line is listed
    assert_eq!(t.fires.len(), 1);
    let f = &t.fires[0];
    assert_eq!((f.mutation, f.every, f.once), (0, 3, false));
    assert_eq!(f.passes, [2, 5]);
    assert_eq!(f.plays, [Some(8), Some(17)]);
    for (&p, i) in f.passes.iter().zip(f.plays.iter().flatten()) {
        assert_eq!((t.plays[*i].pass, t.plays[*i].section.as_str()), (p, "drop"));
    }
    // and the song lays out every pass: 19 sections of two bars
    let c = ok(&src);
    assert_eq!(c.segments.len(), 19);
    assert_eq!(c.end, Some(19 * 2));
}

#[test]
fn it_is_deterministic_and_the_pass_is_in_the_dice() {
    let src = song(SIX);
    assert_eq!(ok(&src).cid, ok(&src).cid);
    assert_eq!(table(&src), table(&src));
    // `@2` fires on passes 1 and 3: the same base, another pass, another permutation
    let shuffle = song("arrange {\n  intro drop\n  @2 acid shuffle\n  drop\n} loop 4 then end");
    let t = table(&shuffle);
    let once: Vec<String> = t.plays.iter().flat_map(|p| p.reads.iter().filter(|r| r.once).map(|r| r.text.clone())).collect();
    assert_eq!(once.len(), 2);
    assert_ne!(once[0], once[1], "the pass, not the count, is what the dice are rolled on");
    assert_eq!(table(&shuffle), t);
}

#[test]
fn a_one_shot_count_reads_once_and_carries_into_the_next_pass() {
    // `@4`: on passes 3 and 7 the next section (the second drop) reads the doubled kick, then it reverts
    let src = song("arrange {\n  intro drop\n  @4 kik double\n  drop\n} loop 8 then outro end");
    let t = table(&src);
    let once: Vec<usize> = t.plays.iter().enumerate().filter(|(_, p)| p.reads.iter().any(|r| r.once)).map(|(i, _)| i).collect();
    assert_eq!(once, [11, 23]);
    assert_eq!(t.plays[11].reads.iter().find(|r| r.once).unwrap().text, "x ~ x ~ x ~ x ~");
    assert!(!t.patterns.contains_key("kik"), "a one-shot makes no generation");
    assert_eq!(t.fires.len(), 1);
    assert_eq!((t.fires[0].once, t.fires[0].passes.clone(), t.fires[0].plays.clone()), (true, vec![3, 7], vec![Some(11), Some(23)]));
    ok(&src);
    // a `@2` at the end of the block rides across the pass to the next pass's first section, as a `@` always has; on the
    // last pass the coda reads it
    let end = song("arrange {\n  intro drop\n  @2 kik double\n} loop 4 then outro end");
    let t = table(&end);
    let once: Vec<(usize, u32, bool)> = t.plays.iter().enumerate().filter(|(_, p)| p.reads.iter().any(|r| r.once)).map(|(i, p)| (i, p.pass, p.coda)).collect();
    assert_eq!(once, [(4, 2, false), (8, 3, true)]);
    assert_eq!(t.fires[0].plays, [Some(4), Some(8)]);
    // with no coda the last firing has nothing to play in: the warning `@` always had
    let none = song("arrange {\n  intro drop\n  @2 kik double\n} loop 4 then end");
    assert!(warnings(&none).iter().any(|w| w.contains("no next section")), "{:?}", warnings(&none));
    // (and the table says so: the last firing is read by no play)
    assert_eq!(table(&none).fires[0].passes, [1, 3]);
    assert_eq!(table(&none).fires[0].plays, [Some(4), None]);
}

#[test]
fn a_glob_and_comma_targets_share_the_line_count() {
    let src = "tempo 120\nkey am\nlet pad_a = a3 c4 e4 g4\nlet pad_b = e4 g4 b4 d5\nlet lead = a4 ~ c5 ~\nlet kik = x ~ x ~\n\
               section s 2 bars {\n  deck A: kick kik\n  deck B: harmony pad_a | arp pad_b | melody lead\n}\n\
               arrange {\n  s\n  ~2 pad_* drift 0.5, lead up 1\n} loop 6 then end\n";
    assert!(check(src).ok(), "{}", render_all(&diags(src), src));
    let t = table(src);
    for name in ["pad_a", "pad_b", "lead"] {
        assert_eq!(t.patterns[name].iter().map(|g| g.pass).collect::<Vec<_>>(), [None, Some(1), Some(3), Some(5)], "{name}");
        assert!(t.patterns[name][1..].iter().all(|g| g.mutation == Some(0) && g.every == 2), "{name}");
    }
    assert_eq!(t.fires.len(), 1, "one line, one entry");
    assert_eq!(t.fires[0].passes, [1, 3, 5]);
    // a glob that names nothing fires on nothing, and says so as it always did
    let none = src.replace("pad_*", "zz_*").replace(", lead up 1", "");
    assert!(warnings(&none).iter().any(|w| w.contains("`zz_*` matches no `let` pattern")));
    assert!(table(&none).fires.is_empty());
}

#[test]
fn a_line_that_can_never_fire_says_so_at_its_count_and_still_checks() {
    let at = |block: &str| {
        let src = song(block);
        assert!(check(&src).ok(), "{}", render_all(&diags(&src), &src));
        let d = diags(&src);
        let w = d.iter().find(|d| d.severity == Severity::Warning && d.message.contains("never fires")).unwrap_or_else(|| panic!("no warning for {block}"));
        assert!(w.help.is_some());
        assert!(!table(&src).patterns.contains_key("acid"), "a line that never fires makes no generation");
        assert!(table(&src).fires.is_empty());
        (w.message.clone(), w.span.text(&src).to_string())
    };
    // no `loop`: the block plays once
    assert_eq!(at("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n}"), ("this `arrange` plays once, so `~3` never fires".to_string(), "3".to_string()));
    assert_eq!(at("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} end").0, "this `arrange` plays once, so `~3` never fires");
    // fewer passes than the count
    assert_eq!(at("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} loop 2 then outro end").0, "`loop 2` plays 2 passes, so `~3` never fires");
    assert_eq!(at("arrange {\n  intro drop\n  @2 acid rev\n  drop\n} loop 1 then end").0, "`loop 1` plays 1 pass, so `@2` never fires");
    // a `loop` with no count lays out one pass
    assert_eq!(at("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} loop").0, "a `loop` with no count is laid out as one pass, so `~3` never fires");
    // a named arrangement runs its mutations over one pass
    let named = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/named-arrange-mutations.btw")).unwrap().replace("~ acid rot 2", "~2 acid rot 2");
    assert!(check(&named).ok());
    assert!(warnings(&named).iter().any(|w| w.starts_with("a named `arrange` runs its mutations over one pass") && w.ends_with("so `~2` never fires")), "{:?}", warnings(&named));
    assert!(!table(&named).patterns.contains_key("acid"));
    // no warning where it does fire: `loop 3` has the 3rd pass, and a count of 1 is every pass
    for block in ["arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} loop 3 then outro end", "arrange {\n  intro drop\n  ~1 acid rev\n  drop\n}", "arrange {\n  intro drop\n  ~ acid rev\n  drop\n} loop"] {
        assert!(warnings(&song(block)).iter().all(|w| !w.contains("never fires")), "{block}");
    }
    // `~N` equal to the loop's count fires once, on the last pass, and the coda reads it
    let t = table(&song("arrange {\n  intro drop\n  ~3 acid rev\n  drop\n} loop 3 then outro end"));
    assert_eq!(t.patterns["acid"].iter().map(|g| g.pass).collect::<Vec<_>>(), [None, Some(2)]);
    assert_eq!(t.plays.last().unwrap().reads.iter().find(|r| r.pattern == "acid").map(|r| r.generation), Some(1));
}

#[test]
fn the_fixer_keeps_the_count() {
    let src = song("arrange {\n  intro drop\n  ~3 acid rot 2 | wobble 3\n  drop\n} loop 6 then end");
    assert!(!check(&src).ok());
    let fixed = fix(&src).expect("a fix");
    assert!(fixed.contains("~3 acid rot 2 | id  # fixed: was `wobble 3`\n"), "{fixed}");
    assert!(check(&fixed).ok());
}

/// The example a person plays: the generations it comes to are pinned, so a change in the walk, the dice or the lowering
/// shows here.
#[test]
fn the_example_comes_to_the_generations_it_teaches() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/arrange-every-n.btw")).unwrap();
    let c = ok(&src);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let t = table(&src);
    assert_eq!(t.passes, 6);
    let passes = |name: &str| t.patterns[name].iter().map(|g| g.pass).collect::<Vec<_>>();
    // the arp reversed on every 3rd pass; the basses drift on every 2nd
    assert_eq!(passes("riff"), [None, Some(2), Some(5)]);
    assert_eq!(passes("bass_a"), [None, Some(1), Some(3), Some(5)]);
    assert_eq!(passes("bass_b"), [None, Some(1), Some(3), Some(5)]);
    assert!(!t.patterns.contains_key("kik"), "`@4` is a one-shot: no generation");
    let fires: Vec<(usize, u32, bool, Vec<u32>)> = t.fires.iter().map(|f| (f.mutation, f.every, f.once, f.passes.clone())).collect();
    assert_eq!(fires, [(0, 3, false, vec![2, 5]), (1, 2, false, vec![1, 3, 5]), (2, 4, true, vec![3])]);
    // the kick is doubled in one section only: pass 3's last
    let once: Vec<(u32, &str)> = t.plays.iter().filter(|p| p.reads.iter().any(|r| r.once)).map(|p| (p.pass, p.section.as_str())).collect();
    assert_eq!(once, [(3, "lift")]);
    // the plays each firing was first read by, in the order the lines fire
    let names = |i: usize| t.fires[i].plays.iter().map(|&p| t.plays[p.unwrap()].name.clone()).collect::<Vec<_>>();
    assert_eq!(names(0), ["groove@3", "groove@5"]);
    assert_eq!(names(1), ["lift@2", "lift@4", "lift@6"]);
    assert_eq!(names(2), ["lift@4"]);
    // four sections a pass, six passes, and the outro
    assert_eq!(t.plays.len(), 6 * 4 + 1);
}
