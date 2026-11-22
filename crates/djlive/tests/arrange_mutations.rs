//! `arrange { }` with mutations (mutate.rs, docs/BTW.md "Arrange mutations").

use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic, Severity};
use djlive::mutate::{fix, generations, GenTable};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}\n{}", src, render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn table(src: &str) -> GenTable {
    generations(src, &djlive::env::Env::none()).expect("a generation table")
}

const SONG: &str = "tempo 128
key am
let acid_a = a2 ~ c3 e3 ~ a2 g2 ~
let acid_b = a2 a2 ~ e3
let hoov_a = a3 ~ ~ c4
let hoov_b = e4 ~ g4 ~
let kik = x ~ x ~
section intro 4 bars {
  deck A: kick kik
}
section drop 4 bars {
  deck A: kick kik | bass acid_a
  deck B: melody hoov_a | arp acid_b
}
section peak 4 bars {
  deck B: melody hoov_b | bass acid_a
}
section outro 4 bars {
  deck A: kick kik
}
arrange {
  intro drop
  ~ acid_a rot 2, hoov_a up 3
  drop
  ~ acid_* drift 0.5
  peak
  @ hoov_b rev
  peak outro
} loop 2 then outro end
";


fn one(pattern: &str, role: &str, chain: &str) -> String {
    let src = format!("key am\nlet p = {pattern}\nlet q = x ~ ~ x x ~ x ~\nlet ch = [a3,c4,e4] ~ [g3,b3,d4] ~\nsection s 1 bars {{\n  deck A: {role} p\n  deck B: harmony ch | kick q\n}}\narrange {{\n  ~ p {chain}\n  s\n}}\n");
    let t = table(&src);
    t.patterns.get("p").map_or_else(|| "(unchanged)".to_string(), |g| g.last().unwrap().text.clone())
}

/// One golden per operator: (operator, the pattern after it). The melody is `a3 c4 ~ e4 g4 ~ a4 ~` in A minor; the seeded
/// ones are pinned to this program's CID, so a change in the hash or the draw order shows here.
const GOLDEN: &[(&str, &str)] = &[
    ("rot 2", "a4 ~ a3 c4 ~ e4 g4 ~"),
    ("rot -1", "c4 ~ e4 g4 ~ a4 ~ a3"),
    ("rev", "~ a4 ~ g4 e4 ~ c4 a3"),
    ("thin 0.5", "a3 c4 ~ e4 ~ ~ ~ ~"),
    ("fill 1", "a3 c4 c4 e4 g4 g4 a4 a4"),
    ("euclid 3", "a3 ~ ~ c4 ~ ~ e4 ~"),
    ("swap", "c4 a3 e4 ~ ~ g4 ~ a4"),
    ("mirror", "a3 c4 ~ e4 e4 ~ c4 a3"),
    ("shuffle", "a3 ~ a4 e4 c4 ~ ~ g4"),
    ("half", "a3 c4 ~ e4"),
    ("double", "a3 c4 ~ e4 g4 ~ a4 ~ a3 c4 ~ e4 g4 ~ a4 ~"),
    ("stutter 2", "a3 c4 ~ e4 a3 c4 ~ e4"),
    ("ratchet 0.5", "a3*2 c4*2 ~ e4 g4*2 ~ a4 ~"),
    ("swing 0.5", "[a3 ~ ~ c4] [~ ~ ~ e4] [g4 ~ ~ ~] [a4 ~ ~ ~]"),
    ("up 2", "c4 e4 ~ g4 b4 ~ c5 ~"),
    ("down 1", "g3 b3 ~ d4 f4 ~ g4 ~"),
    ("invert", "a3 f3 ~ d3 b2 ~ a2 ~"),
    ("oct 1", "a4 c5 ~ e5 g5 ~ a5 ~"),
    ("oct -1", "a2 c3 ~ e3 g3 ~ a3 ~"),
    ("drift 0.5", "a3 b3 ~ d4 g4 ~ g4 ~"),
    ("vel 2", "a3! c4 ~ e4 g4! ~ a4 ~"),
    ("accent 0.5", "a3 c4! ~ e4 g4 ~ a4! ~"),
    ("morph q 0.5", "x c4 ~ e4 g4 ~ x ~"),
    ("xfer q", "a3 ~ ~ c4 e4 ~ g4 ~"),
    ("mask q", "a3 ~ ~ e4 g4 ~ a4 ~"),
    ("grow", "a3 c4 ~ e4 g4 g4 a4 ~"),
    ("decay", "~ c4 ~ e4 g4 ~ a4 ~"),
    ("id", "a3 c4 ~ e4 g4 ~ a4 ~"),
];

#[test]
fn golden_per_operator() {
    let mel = "a3 c4 ~ e4 g4 ~ a4 ~";
    for (op, want) in GOLDEN {
        assert_eq!(one(mel, "melody", op), *want, "`{op}`");
    }
    // every operator of the closed set has its golden
    for name in djlive::mutate::operator_names() {
        assert!(GOLDEN.iter().any(|(o, _)| o.split(' ').next() == Some(name)) || name == "voice" || name == "spread" || name == "jux", "no golden for `{name}`");
    }
    assert_eq!(one("x x x x", "kick", "swing 0.5"), "[x ~ ~ ~ ~ ~ x ~] [x ~ ~ ~ ~ ~ x ~]");
    assert_eq!(one("1 3 5 ~", "bass", "up 2"), "3 5 7 ~");
    assert_eq!(one(mel, "melody", "rot 1 | up 1"), "~ b3 d4 ~ f4 a4 ~ b4", "a chain, in order");
}

#[test]
fn golden_chords() {
    let src = "key am\nlet ch = [a3,c4,e4] ~ [g3,b3,d4] ~\nsection s 1 bars {\n  deck B: harmony ch\n}\narrange {\n  ~ ch voice 1\n  s\n  ~ ch spread\n  s\n}\n";
    let g: Vec<String> = table(src).patterns["ch"].iter().map(|g| g.text.clone()).collect();
    assert_eq!(g, ["[a3, c4, e4] ~ [g3, b3, d4] ~", "[c4, e4, a4] ~ [b3, d4, g4] ~", "[c4, a4, e5] ~ [b3, g4, d5] ~"]);
    ok(src);
}

#[test]
fn propagation_and_one_shots() {
    let t = table(SONG);
    let at = |name: &str, pass: u32, pat: &str| t.plays.iter().filter(|p| p.pass == pass && !p.coda).filter(|p| p.section == name).map(|p| p.reads.iter().find(|r| r.pattern == pat).map(|r| (r.generation, r.once))).collect::<Vec<_>>();
    // the first drop reads acid_a as written, the second the rotated one, the peak the drifted one
    assert_eq!(at("drop", 0, "acid_a"), [Some((0, false)), Some((1, false))]);
    assert_eq!(at("peak", 0, "acid_a"), [Some((2, false)), Some((2, false))]);
    // `@ hoov_b rev`: the next peak only, then it reverts
    assert_eq!(at("peak", 0, "hoov_b"), [None, Some((0, true))]);
    let p = t.plays.iter().filter(|p| p.section == "peak").nth(1).unwrap();
    assert_eq!(p.reads.iter().find(|r| r.pattern == "hoov_b").unwrap().text, "~ g4 ~ e4");
    // loop 2: the second pass goes on from the first's last generation (cumulative)
    assert_eq!(at("drop", 1, "acid_a"), [Some((2, false)), Some((3, false))]);
    assert_eq!(t.passes, 2);
    // the coda reads the final generation
    let coda: Vec<_> = t.plays.iter().filter(|p| p.coda).map(|p| p.section.clone()).collect();
    assert_eq!(coda, ["outro"]);
    // the song: 2 passes of 7 plays and the coda, every one laid out (a bounded loop renders every pass)
    let c = ok(SONG);
    let names: Vec<&str> = c.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["intro", "drop", "drop@2", "peak", "peak@2", "outro", "intro", "drop@3", "drop@4", "peak@3", "peak@4", "outro", "outro"]);
    assert_eq!(c.end, Some(13 * 4));
    assert!(!c.looped);
    // a variant's nets differ where its patterns do
    assert_ne!(c.segments[1].decks, c.segments[2].decks);
    assert_eq!(c.segments[0].decks, c.segments[6].decks);
}

#[test]
fn deterministic() {
    let a = ok(SONG);
    let b = ok(SONG);
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.segments, b.segments);
    assert_eq!(table(SONG), table(SONG));
    // spacing and comments are not the program: the same CID, so the same generations
    let spaced = SONG.replace("  ~ acid_a rot 2, hoov_a up 3", "  ~   acid_a rot 2 ,hoov_a   up 3   # turn it");
    let c = ok(&spaced);
    assert_eq!(c.cid, a.cid);
    let hits = |c: &Compiled| c.segments.iter().map(|s| (s.name.clone(), s.decks.values().flat_map(|d| d.parts.iter().map(|p| p.cycles.clone())).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert_eq!(hits(&c), hits(&a));
    // another program (a different CID) seeds the operators differently
    let other = SONG.replace("tempo 128", "tempo 127");
    let o = table(&other);
    let s = table(SONG);
    assert_ne!(ok(&other).cid, a.cid);
    let drifted = |t: &GenTable| t.patterns.values().flat_map(|g| g.iter().map(|x| x.text.clone())).collect::<Vec<_>>();
    assert_ne!(drifted(&o), drifted(&s), "drift is seeded by the CID");
}

#[test]
fn canonical_form() {
    let c = ok(SONG);
    assert!(c.text.contains("arrange {\n  intro drop\n  ~ acid_a rot 2, hoov_a up 3\n  drop\n  ~ acid_* drift 0.5\n  peak\n  @ hoov_b rev\n  peak outro\n} loop 2 then outro end\n"), "{}", c.text);
    // the canonical text is a program, and the same one
    assert_eq!(ok(&c.text).cid, c.cid);
    // `end` is the default ending
    let a = SONG.replace("} loop 2 then outro end", "} end");
    let b = SONG.replace("} loop 2 then outro end", "}");
    assert_eq!(ok(&a).cid, ok(&b).cid);
    // the lowering is not in the text: no `drop@2`, no `acid_a@1`
    assert!(!c.text.contains("@2") && !c.text.contains("acid_a@"));
}

#[test]
fn unbounded_loop_renders_one_pass() {
    let src = SONG.replace("} loop 2 then outro end", "} loop");
    let c = ok(&src);
    assert!(c.looped);
    assert_eq!(c.end, None);
    assert_eq!(c.segments.len(), 6);
    assert!(table(&src).unbounded);
}

#[test]
fn generation_refs() {
    let src = SONG.replace("section outro 4 bars {\n  deck A: kick kik\n}", "section outro 4 bars {\n  deck A: kick kik | bass acid_a@0\n  deck B: bass acid_a@2\n}");
    let c = ok(&src);
    let out = c.segments.iter().find(|s| s.name == "outro").unwrap();
    let drop = &c.segments[1];
    // acid_a@0 is acid_a as written: the first drop's bass
    assert_eq!(out.decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "bass").map(|p| p.cycles.clone()), drop.decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "bass").map(|p| p.cycles.clone()));
    let bad = SONG.replace("deck A: kick kik\n}\narrange", "deck A: kick kik | bass acid_a@99\n}\narrange");
    let e = check(&bad);
    assert!(e.diagnostics.iter().any(|d| d.is_error() && d.message.contains("acid_a@99")), "{:?}", e.diagnostics);
}

#[test]
fn legacy_one_line_arrange_unchanged() {
    let src = LEGACY;
    let c = ok(src);
    assert!(c.text.ends_with("arrange verse chorus verse\n"));
    assert_eq!(c.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["verse", "chorus", "verse"]);
    assert!(generations(src, &djlive::env::Env::none()).is_none());
    // pinned: the CID of this text on main before arrange blocks existed
    assert_eq!(c.cid, LEGACY_CID);
}

const LEGACY: &str = "tempo 128\nlet kik = x ~ x ~\nsection verse 4 bars {\n  deck A: kick kik\n}\nsection chorus 4 bars {\n  deck A: kick x*4\n}\narrange verse chorus verse\n";
const LEGACY_CID: &str = "z4EBG9j4W9kZa4mwxgkprZqfmifaFajuNnLhz4CWjn4UQjHtwXH";

fn diags(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics
}

#[test]
fn checker() {
    let unknown_target = SONG.replace("~ acid_a rot 2", "~ acid_z rot 2");
    let d = diags(&unknown_target);
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`acid_z` is not a `let` pattern") && d.help.as_deref() == Some("did you mean `acid_a`?")), "{d:?}");
    let unknown_op = SONG.replace("rot 2", "rott 2");
    let d = diags(&unknown_op);
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("`rott` is not a mutation operator") && d.help.as_deref().is_some_and(|h| h.contains("`rot`"))), "{d:?}");
    let bad_arg = SONG.replace("drift 0.5", "drift 7");
    assert!(diags(&bad_arg).iter().any(|d| d.is_error() && d.message.contains("share from 0 to 1")));
    let glob = SONG.replace("~ acid_* drift 0.5", "~ zz_* drift 0.5");
    let d = diags(&glob);
    assert!(d.iter().any(|d| d.severity == Severity::Warning && d.message.contains("`zz_*` matches no `let` pattern")), "{d:?}");
    assert!(check(&glob).ok());
    let drum = SONG.replace("~ acid_* drift 0.5", "~ kik up 2");
    let d = diags(&drum);
    assert!(d.iter().any(|d| d.severity == Severity::Warning && d.message.contains("`kik` is a drum pattern")), "{d:?}");
    assert!(table(&drum).patterns["kik"].iter().all(|g| g.text == "x ~ x ~"), "a pitch operator on drums is a no-op");
    let bad_section = SONG.replace("  peak outro\n", "  peak outtro\n");
    assert!(diags(&bad_section).iter().any(|d| d.is_error() && d.message.contains("no section `outtro`")));
}

#[test]
fn fixer_comments_bad_ops_out() {
    let src = SONG.replace("~ acid_a rot 2, hoov_a up 3", "~ acid_a rot 2 | wobble 3 | thin 0.1, hoov_a up 3");
    assert!(!check(&src).ok());
    let fixed = fix(&src).expect("a fix");
    assert!(fixed.contains("~ acid_a rot 2 | id | thin 0.1, hoov_a up 3  # fixed: was `wobble 3`\n"), "{fixed}");
    let c = ok(&fixed);
    assert!(c.warnings.iter().all(|d| !d.is_error()));
    assert!(fix(&fixed).is_none(), "nothing left to fix");
    assert!(fix(SONG).is_none());
}

#[test]
fn parse_errors() {
    let unclosed = "let a = x ~\nsection s 1 bars {\n  deck A: kick a\n}\narrange {\n  s\n";
    assert!(diags(unclosed).iter().any(|d| d.is_error()));
    let no_ops = SONG.replace("~ acid_a rot 2, hoov_a up 3", "~ acid_a");
    assert!(diags(&no_ops).iter().any(|d| d.is_error() && d.message.contains("operator")));
    let two = format!("{SONG}arrange intro\n");
    assert!(diags(&two).iter().any(|d| d.is_error() && d.message.contains("one `arrange`")));
}

/// Named arrangements and `arrange { }` mutations together: `arrange NAME: { ... }` is one named arrangement of several,
/// its mutations run over one pass of it, and the sections the other arrangements play are as written.
#[test]
fn a_named_arrangement_with_mutations() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/named-arrange-mutations.btw")).unwrap();
    let c = ok(&src);
    // the flow: open, set twice (`loop 2`), close; a finite song
    let f = c.flow.as_ref().expect("a flow");
    assert_eq!(f.order, ["open", "set", "set", "close"]);
    assert!(!c.looped && f.loop_from.is_none());
    let names: Vec<&str> = c.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["intro", "drop", "drop", "drop@2", "drop", "drop@2", "drop", "outro"]);
    assert_eq!(c.song_bars(), Some(16));
    // the mutated drop reads the rotated acid; the drop as written (open's, set's first, close's) all read the same
    let bass = |i: usize| c.segments[i].decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "bass").map(|p| p.cycles.clone()).unwrap();
    assert_eq!(bass(1), bass(2));
    assert_eq!(bass(1), bass(6));
    assert_ne!(bass(1), bass(3));
    assert_eq!(bass(3), bass(5));
    // the generation table names the arrangement and has the one mutation
    let t = table(&src);
    assert_eq!(t.arrangement.as_deref(), Some("set"));
    assert_eq!(t.plays.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["drop", "drop@2"]);
    assert_eq!(t.patterns["acid"].len(), 2);
    // the canonical text keeps the block as written, and is the same program
    assert!(c.text.contains("arrange set: {\n  drop\n  ~ acid rot 2\n  drop\n} loop 2\n"), "{}", c.text);
    assert!(!c.text.contains("drop@2"));
    assert_eq!(ok(&c.text).cid, c.cid);
    // every ending a named arrangement has, a named block has too
    for (end, order) in [("then close", vec!["open", "set", "close"]), ("end", vec!["open", "set"]), ("", vec!["open", "set", "close"])] {
        let s = src.replace("} loop 2\n", &format!("}} {end}\n").replace("} \n", "}\n"));
        assert_eq!(ok(&s).flow.as_ref().unwrap().order, order, "{end}");
    }
    let looped = ok(&src.replace("} loop 2\n", "} loop\n"));
    assert!(looped.looped);
}

#[test]
fn an_unnamed_arrange_block_among_named_arrangements_is_refused() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/named-arrange-mutations.btw")).unwrap();
    let e = check(&src.replace("arrange set: {", "arrange {"));
    assert!(e.diagnostics.iter().any(|d| d.is_error() && d.message.starts_with("an `arrange { }` among named arrangements needs a name")), "{:?}", e.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
}

// ---- chord lets (project-datum 6639): `chord(gm7)` and `<chord(..) ..>` steps are pitched, like `[g3, a#3, d4, f4]`

/// The notes `role` of deck B plays in segment `seg`, every cycle: (step, MIDI note).
fn played(c: &Compiled, seg: usize, role: &str) -> Vec<Vec<(u8, u8)>> {
    let part = c.segments[seg].decks[&djlive::ast::Deck::B].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} in segment {seg}"));
    part.cycles.iter().map(|cy| cy.iter().map(|h| (h.step, h.note)).collect()).collect()
}

/// A song that plays `pads` before and after `mutation` (`~ pads voice 1`, `@ pads spread`), the harmony part given as `part`.
fn pads_song(pads: &str, part: &str, mutation: &str) -> String {
    format!("key gm\nlet pads = {pads}\nlet kik = x ~ x ~\nsection s 2 bars {{\n  deck A: kick kik\n  deck B: {part}\n}}\narrange {{\n  s\n  {mutation}\n  s\n}}\n")
}

fn does_nothing(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().filter(|d| d.message.contains("does nothing")).map(|d| d.message.clone()).collect()
}

/// The two spellings of the same chords, voiced the way the part plays them: gm7 and cm7 in octave 3 (harmony's own).
const CHORDS: &str = "chord(gm7) ~ chord(cm7) ~";
const BRACKETS: &str = "[g3,a#3,d4,f4] ~ [c3,d#3,g3,a#3] ~";
const CHORDS_ALT: &str = "<chord(gm7) chord(cm7)>";
const BRACKETS_ALT: &str = "<[g3,a#3,d4,f4] [c3,d#3,g3,a#3]>";

#[test]
fn chord_lets_take_the_pitch_and_chord_operators_as_stacked_chords_do() {
    // every operator the record names, on the one-shot form too, on a let of `chord(..)` steps and on an alternation of them
    for (chords, brackets) in [(CHORDS, BRACKETS), (CHORDS_ALT, BRACKETS_ALT), ("chord(gm7) ~ ~ chord(cm7, 4)", "[g3,a#3,d4,f4] ~ ~ [c4,d#4,g4,a#4]")] {
        for op in ["voice 1", "voice -1", "voice 2", "spread", "invert", "up 2", "down 1", "oct 1", "oct -1", "voice 1 | spread", "rot 1 | up 2 | voice 1"] {
            for lead in ["~", "@"] {
                let m = format!("{lead} pads {op}");
                let (a, b) = (pads_song(chords, "harmony pads", &m), pads_song(brackets, "harmony pads", &m));
                assert_eq!(does_nothing(&a), Vec::<String>::new(), "{chords} `{m}`");
                let (ca, cb) = (ok(&a), ok(&b));
                for seg in 0..2 {
                    assert_eq!(played(&ca, seg, "harmony"), played(&cb, seg, "harmony"), "{chords} `{m}`: segment {seg} plays what its bracket spelling plays");
                }
                assert_ne!(played(&ca, 1, "harmony"), played(&ca, 0, "harmony"), "{chords} `{m}` changes the notes");
            }
        }
    }
}

#[test]
fn chord_lets_are_written_out_as_the_notes_they_play() {
    let gens = |pads: &str, m: &str| -> Vec<String> { table(&pads_song(pads, "harmony pads", m)).patterns["pads"].iter().map(|g| g.text.clone()).collect() };
    assert_eq!(gens(CHORDS, "~ pads voice 1"), [CHORDS, "[a#3, d4, f4, g4] ~ [d#3, g3, a#3, c4] ~"]);
    assert_eq!(gens(BRACKETS, "~ pads voice 1")[1], "[a#3, d4, f4, g4] ~ [d#3, g3, a#3, c4] ~");
    assert_eq!(gens(CHORDS, "~ pads spread")[1], "[g3, d4, a#4, f5] ~ [c3, g3, d#4, a#4] ~");
    // alternations: the chords in them are reached, spelled either way
    assert_eq!(gens(CHORDS_ALT, "~ pads voice 1")[1], "<[a#3, d4, f4, g4] [d#3, g3, a#3, c4]>");
    assert_eq!(gens(BRACKETS_ALT, "~ pads voice 1")[1], "<[a#3, d4, f4, g4] [d#3, g3, a#3, c4]>");
    // an operator that does not move pitches leaves a chord call as it is written
    assert_eq!(gens(CHORDS, "~ pads rot 1")[1], "~ chord(gm7) ~ chord(cm7)");
}

#[test]
fn a_chord_let_is_voiced_in_the_octave_of_the_part_that_reads_it() {
    let m = "~ pads voice 1";
    let notes = |part: &str| -> Vec<Vec<(u8, u8)>> { played(&ok(&pads_song(CHORDS, part, m)), 1, "harmony") };
    // `oct=4` plays `chord(gm7)` from g4; the mutated chord is the one that part played, turned over
    let high = pads_song("chord(gm7) ~ chord(cm7) ~", "harmony pads oct=4", m);
    assert_eq!(table(&high).patterns["pads"][1].text, "[a#4, d5, f5, g5] ~ [d#4, g4, a#4, c5] ~");
    assert_eq!(notes("harmony pads oct=4"), played(&ok(&pads_song("[g4,a#4,d5,f5] ~ [c4,d#4,g4,a#4] ~", "harmony pads oct=4", m)), 1, "harmony"));
    // a chord with an octave of its own keeps it
    assert_eq!(table(&pads_song("chord(gm7, 2) ~ ~ ~", "harmony pads", m)).patterns["pads"][1].text, "[a#2, d3, f3, g3] ~ ~ ~");
    // read in two octaves: voiced in the first, and `check` says so
    let two = "key gm\nlet pads = chord(gm7) ~ chord(cm7) ~\nsection s 2 bars {\n  deck B: harmony pads | arp pads\n}\narrange {\n  s\n  ~ pads voice 1\n  s\n}\n";
    let d = check(two).diagnostics;
    let w = d.iter().find(|d| d.severity == Severity::Warning && d.message.contains("is read in octaves 3 and 4")).unwrap_or_else(|| panic!("{d:?}"));
    assert!(w.message.contains("`voice` voices its `chord(...)` steps in octave 3"), "{}", w.message);
    assert_eq!(table(two).patterns["pads"][1].text, "[a#3, d4, f4, g4] ~ [d#3, g3, a#3, c4] ~");
    // a let read in one octave, or written out, has nothing to say
    assert!(!check(&two.replace("harmony pads | arp pads", "harmony pads")).diagnostics.iter().any(|d| d.message.contains("read in octaves")));
    assert!(!check(&two.replace("chord(gm7) ~ chord(cm7) ~", BRACKETS)).diagnostics.iter().any(|d| d.message.contains("read in octaves")));
}

#[test]
fn a_chord_let_in_a_mixed_pattern_moves_with_its_notes() {
    // a let of a bass line and a chord: the notes were moved before, the chord stayed put
    let src = "key gm\nlet p = g2 ~ chord(gm7, 2) ~\nsection s 2 bars {\n  deck B: harmony p oct=2\n}\narrange {\n  s\n  ~ p up 2\n  s\n}\n";
    assert_eq!(table(src).patterns["p"][1].text, "a#2 ~ [a#2, d3, f3, a3] ~");
}

#[test]
fn drum_lets_still_warn() {
    // no pitch in it
    let d = does_nothing(&pads_song("x ~ x ~", "harmony pads", "~ pads voice 1"));
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].contains("`voice` moves pitches, and `pads` is a drum pattern: it does nothing there"), "{}", d[0]);
    let src = pads_song("x ~ x ~", "harmony pads", "~ pads up 2");
    assert_eq!(does_nothing(&src).len(), 1);
    assert_eq!(table(&src).patterns["pads"].iter().map(|g| g.text.as_str()).collect::<Vec<_>>(), ["x ~ x ~", "x ~ x ~"], "a pitch operator on drums is a no-op");
    // chords, but read only by a drum part: still the drum's
    assert_eq!(does_nothing(&pads_song(CHORDS, "kick pads", "~ pads spread")).len(), 1);
    // a call that is not a chord is no pitch: the generators of hits
    assert_eq!(does_nothing(&pads_song("e(5,8)", "hihat pads", "~ pads up 1")).len(), 1);
    assert_eq!(does_nothing(&pads_song("e(5,8)", "harmony pads", "~ pads up 1")).len(), 1);
    // a chord call that is no chord stays what `check` makes of it: an error where it is played, no mutation behind it
    let bad = pads_song("chord(zz9) ~ ~ ~", "harmony pads", "~ pads voice 1");
    assert!(!check(&bad).ok());
}

#[test]
fn stacked_chords_in_alternations_and_groups_are_reached_by_voice_and_spread() {
    let gens = |pads: &str, m: &str| table(&pads_song(pads, "harmony pads", m)).patterns["pads"].last().unwrap().text.clone();
    assert_eq!(gens("<[a3,c4,e4] [g3,b3,d4]>", "~ pads voice 1"), "<[c4, e4, a4] [b3, d4, g4]>");
    assert_eq!(gens("<[a3,c4,e4] ~> ~", "~ pads spread"), "<[a3, e4, c5] ~> ~");
    assert_eq!(gens("[[a3,c4,e4] ~] [g3,b3,d4] ~", "~ pads voice -1"), "[[e3, a3, c4] ~] [d3, g3, b3] ~");
}

#[test]
fn chord_lets_through_a_one_shot_and_a_loop_keep_their_generations() {
    // the record's shape: pads mutated each pass, a one-shot over the next section
    let src = "key gm\nlet pads = chord(gm7) ~ chord(cm7) ~\nsection s 2 bars {\n  deck B: harmony pads\n}\narrange {\n  s\n  ~ pads voice 1\n  s\n  @ pads spread\n  s s\n} loop 2 then end\n";
    assert_eq!(does_nothing(src), Vec::<String>::new());
    let t = table(src);
    assert!(t.patterns["pads"].len() > 2, "{:?}", t.patterns["pads"]);
    assert!(t.patterns["pads"].iter().skip(1).all(|g| g.text.starts_with('[')), "{:?}", t.patterns["pads"]);
    ok(src);
}
