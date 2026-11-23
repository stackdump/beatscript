//! The form extensions (roles.rs, docs/beatscript/forms.md "Extensions"), with no generator: themes, notes and forms in a role.
//! (Sets, stored sections and seeds in a role need a store and the generator: crates/beatscript-api/tests/forms_roles.rs.)
//! - a form's section is a source: where a line goes, its part as it first plays; where sections go, that section;
//! - a seed role's parts are bound one at a time (`a.bass=`), after the role in the canonical spelling;
//! - the canon's `delay=` and `interval=` move its chase's copy, and nothing else;
//! - the quodlibet names its roles (`a`, `b`, `ground`, `third`);
//! - every new parameter has a default that keeps a reference that does not say it written out as it was (roles.rs
//!   `bound_forms_are_pinned`, template.rs `templates_v1_are_pinned`); the written-out program plays as the reference;
//! - what is wrong is said at the binding, with what would work.

use djlive::compile::{check_with, Check, Compiled};
use djlive::env::{Env, StaticVoices};
use djlive::template;

fn check(src: &str) -> Check {
    let v = StaticVoices::default();
    check_with(src, &Env { voices: Some(&v), ..Env::default() })
}

fn complaints(c: &Check) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.severity != djlive::Severity::Info).map(|d| d.message.clone()).collect()
}

fn notes(src: &str) -> Vec<String> {
    check(src).diagnostics.into_iter().filter(|d| d.severity == djlive::Severity::Info).map(|d| d.message).collect()
}

fn clean(src: &str) -> Compiled {
    let c = check(src);
    let bad = complaints(&c);
    assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
    c.compiled.unwrap()
}

fn expanded(src: &str) -> String {
    let v = StaticVoices::default();
    match template::expand(src, &Env { voices: Some(&v), ..Env::default() }) {
        Some(Ok(e)) => e.text,
        other => panic!("{src}: {other:?}"),
    }
}

fn errors(src: &str) -> Vec<(String, String)> {
    check(src).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
}

fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>())).collect()
}

/// The reference checks clean, and its written-out program checks clean on its own and plays the same.
fn plays_as_written(src: &str) -> String {
    let c = clean(src);
    let text = expanded(src);
    assert_eq!(nets(&clean(&text)), nets(&c), "{src}: the written-out program plays as the reference:\n{text}");
    text
}

fn lets_of(text: &str, name: &str) -> Vec<String> {
    text.lines().filter(|l| l.starts_with(&format!("let {name}_"))).map(str::to_string).collect()
}

// ---------------------------------------------------------------- a form's section as a source

/// A canon whose subject is the cross's overlap: the arp of its `both` section (Ode to Joy, morphed toward the Badinerie by the
/// plan before `both` first plays), as notes, a let a bar, chased by its copies.
#[test]
fn a_forms_section_is_a_line_where_a_line_goes() {
    let src = "arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both.arp)\n";
    let text = plays_as_written(src);
    assert!(text.contains("# subject, bound: template(cross).both.arp\nlet subject_1 = "), "{text}");
    let bars = lets_of(&text, "subject");
    assert!(!bars.is_empty() && bars.len() <= 16, "{bars:?}");
    // notes a person could type: no reference is left in the line
    assert!(bars.iter().all(|l| !l.contains("template(") && !l.contains("ode_to_joy")), "{bars:?}");
    let k = bars.len();
    assert!(text.contains(&format!("let follow = <subject_{k} ")) || (k == 1 && text.contains("let follow = rotate(subject, 8)")), "{text}");
    assert!(notes(src).iter().any(|n| n.contains("the arp of the both of template(cross) as it first plays") && n.contains("in d")), "{:?}", notes(src));
    // it is the overlap: the arp the plan has morphed, not Ode to Joy as written
    let plain = expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy)\n");
    assert_ne!(lets_of(&plain, "subject"), bars, "the plan before `both` moved the arp");
    // with no part named, the role's own part: a subject's is the melody
    let mel = expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both)\n");
    assert!(mel.contains("# subject, bound: template(cross).both\n"), "{mel}");
    assert_ne!(lets_of(&mel, "subject"), bars);
    clean("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both)\n");
}

/// A section of a form in another key is moved into the key the role plays in (the rule a theme is moved by): the
/// passacaglia's (G minor) in the canon (D major) goes to B minor, D's relative minor, +4.
#[test]
fn a_forms_section_is_moved_into_the_roles_key() {
    let src = "arrange from template(canon, @mgm_pease_porridge_hot, subject=template(passacaglia, upper=[g4 a4 bb4 d5]).variation.arp)\n";
    let text = plays_as_written(src);
    assert!(text.contains("let subject_1 = b4 c#5 d5 f#5"), "{text}");
    assert!(notes(src).iter().any(|n| n.contains("moved +4 semitones from gm into d")), "{:?}", notes(src));
}

/// Where a role takes sections, a form's section is that section as it first plays, its parts written out, standing in for
/// the role's section (the rondo's `episode`) in its own key.
#[test]
fn a_forms_section_is_a_section_where_sections_go() {
    let src = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(cross).both)\n";
    let text = plays_as_written(src);
    assert!(text.contains("# episodes, bound: template(cross).both\n# episode: the both of template(cross), as it first plays (its control lines stay behind)\nsection episode 8 bars {\n  key d\n  deck A: "), "{text}");
    assert!(!text.contains("section ep_") && text.contains("\n  episode\n"), "the places keep their name:\n{text}");
    // with a part named, a role that takes a line or sections takes the line
    let line = expanded("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(cross).both.arp)\n");
    assert!(line.contains("# episodes, bound: template(cross).both.arp\nlet figure_1 = "), "{line}");
}

/// In a seed role, a form's section gives each part of it (the cross's `a` its melody, bass and chords).
#[test]
fn a_forms_section_fills_a_seed_role_part_by_part() {
    let src = "arrange from template(cross, @psalm42_deep_calleth, a=template(canon).round)\n";
    let text = plays_as_written(src);
    for part in ["melody", "bass", "harmony"] {
        assert!(text.contains(&format!("# a's {part}, bound: template(canon).round\nlet a_{part}_1 = ")), "{part}:\n{text}");
    }
}

/// What is wrong with a form's section is said at it.
#[test]
fn a_forms_section_says_what_is_wrong() {
    let cases: [(&str, &str, &str); 5] = [
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).bothh)\n", "template(cross).bothh", "did you mean `.both`"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).sung)\n", "template(cross).sung", "the outer form's phrase"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both.snare)\n", "template(cross).both.snare", "plays no snare"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).arp)\n", "arp", "section comes first"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross))\n", "template(cross)", "one section of the form"),
    ];
    for (src, at, msg) in cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a.starts_with(at) && m.contains(msg)), "{src}: {e:?}");
    }
}

// ---------------------------------------------------------------- sub-roles

/// One part of a seed role, bound alone: the cross's `a.bass` is the bass under a's tune, the rest of a as it was.
#[test]
fn a_seed_roles_part_is_bound_alone() {
    let src = "arrange from template(cross, @psalm42_deep_calleth, a.bass=<[d2 ~ ~ ~ a1 ~ ~ ~] [g1 ~ ~ ~ a1 ~ ~ ~]>)\n";
    let text = plays_as_written(src);
    assert!(text.contains("# a.bass, bound: <[d2 ~ ~ ~ a1 ~ ~ ~] [g1 ~ ~ ~ a1 ~ ~ ~]>\nlet a_bass_1 = d2 ~ ~ ~ a1 ~ ~ ~\nlet a_bass_2 = g1 ~ ~ ~ a1 ~ ~ ~\nlet a_bass = <a_bass_1 a_bass_2>"), "{text}");
    assert!(text.contains("let ode_to_joy = <") && !text.contains("let pachelbel_ground"), "a's tune stays, its bass is the line:\n{text}");
    assert!(text.contains("~ a_bass_* morph b_bass_1 0.2"), "the plan reads the bound part:\n{text}");
    assert!(text.contains("# a: Beethoven's Ode to Joy over Pachelbel's ground, in D, a.bass=<[d2"), "{text}");
    // a whole role and one of its parts: the part bound alone wins over what the whole gives it
    let both = plays_as_written("arrange from template(cross, @psalm42_deep_calleth, a=badinerie, a.bass=[d2 ~ a1 ~])\n");
    assert!(both.contains("# a's melody, bound: badinerie\n") && both.contains("# a.bass, bound: [d2 ~ a1 ~]\n"), "{both}");
}

/// The canonical spelling: a role's parts right after the role, in the order of its parts.
#[test]
fn a_part_is_spelt_after_its_role() {
    let c = clean("arrange from template( cross,@psalm42_deep_calleth,b=badinerie,a.harmony=[[d3,f#3,a3] ~],a.bass = [d2 ~ a1 ~],a=ode_to_joy )\n");
    assert_eq!(c.text, "arrange from template(cross, @psalm42_deep_calleth, a=ode_to_joy, a.bass=[d2 ~ a1 ~], a.harmony=[[d3, f#3, a3] ~], b=badinerie)\n");
    assert_eq!(clean(&c.text).cid, c.cid);
}

/// What is wrong with a part bound alone is said at it.
#[test]
fn a_part_bound_alone_says_what_is_wrong() {
    let cases: [(&str, &str, &str); 5] = [
        ("arrange from template(cross, @psalm42_deep_calleth, a.bas=[d2 ~])\n", "a.bas", "`a.bass`"),
        ("arrange from template(passacaglia, @eccl3_season, ground.bass=[g1 ~])\n", "ground.bass", "is one line"),
        ("arrange from template(cross, @psalm42_deep_calleth, c.bass=[d2 ~])\n", "c.bass", "is no role of the cross"),
        ("arrange from template(cross, @psalm42_deep_calleth, a=[d5 c#5], a.melody=[b4 a4])\n", "a.melody", "bound twice"),
        ("arrange from template(cross, @psalm42_deep_calleth, a.bass=[d2 ~], a.bass=[g1 ~])\n", "a.bass", "bound twice"),
    ];
    for (src, at, msg) in cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a.starts_with(at) && m.contains(msg)), "{src}: {e:?}");
    }
}

// ---------------------------------------------------------------- the canon's delay= and interval=

/// `delay=6` puts the chase's copy a bar and two beats behind (its bars a bar late, each moved two beats along), `interval=7`
/// plays it a fifth up; the comments say so, the round's third voice stays a beat behind in unison.
#[test]
fn the_canons_delay_and_interval_move_its_copy() {
    let src = "arrange from template(canon, @mgm_pease_porridge_hot, delay=6, interval=7)\n";
    let text = plays_as_written(src);
    assert!(text.contains("let follow = <rotate(pachelbel_violin_2, 8) rotate(pachelbel_violin_1, 8)>\n"), "{text}");
    assert!(text.contains("| arp transpose(follow, 7) inst=harpsichord") && !text.contains("octave(follow, -1)"), "{text}");
    assert!(text.contains("# chase: the copy a bar and 2 beats behind, a fifth up") && text.contains("# the copies: a bar and 2 beats behind, and a beat behind"), "{text}");
    assert!(text.contains("let close = rotate(pachelbel_violin, 4)"), "the third voice is as it was:\n{text}");
    assert!(text.contains("# delay: 6; how far behind") && text.contains("# interval: 7; the interval"), "the header says the numbers:\n{text}");
    let n = notes(src);
    assert!(n.iter().any(|m| m.starts_with("delay: the chase's copy a bar and 2 beats behind")) && n.iter().any(|m| m.starts_with("interval: the chase's copy a fifth up")), "{n:?}");
    // a bar behind with a bound subject; half a bar is a one-bar subject's; an octave is octave(), unison the copy itself
    assert!(expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=<[d5 c#5] [b4 a4] [b4 c#5]>, delay=8)\n").contains("let follow = <subject_2 subject_3 subject_1>"));
    assert!(expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4], delay=3)\n").contains("let follow = rotate(subject, 12)"));
    assert!(expanded("arrange from template(canon, @mgm_pease_porridge_hot, interval=-24)\n").contains("arp octave(follow, -2) inst="));
    assert!(expanded("arrange from template(canon, @mgm_pease_porridge_hot, interval=0)\n").contains("| arp follow inst="));
    clean("arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4], delay=1, interval=-5)\n");
}

/// The defaults are the form as it was: `delay=4` (a bar) and `interval=-12` (an octave down) write the same copy and the
/// same parts as a reference that says neither; a reference with neither is its pinned text.
#[test]
fn the_canons_defaults_are_the_canon_as_it_was() {
    let plain = expanded("arrange from template(canon, @mgm_pease_porridge_hot)\n");
    let said = expanded("arrange from template(canon, @mgm_pease_porridge_hot, delay=4, interval=-12)\n");
    let body = |t: &str| t.lines().filter(|l| !l.starts_with('#')).map(str::to_string).collect::<Vec<_>>();
    assert_eq!(body(&plain), body(&said));
}

/// The canonical spelling puts the numbers after the roles, in the form's order.
#[test]
fn the_canons_numbers_are_spelt_after_its_roles() {
    let c = clean("arrange from template(canon,@mgm_pease_porridge_hot,interval = -5,delay=2,subject=[d5 c#5 b4 a4])\n");
    assert_eq!(c.text, "arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4], delay=2, interval=-5)\n");
    assert_eq!(clean(&c.text).cid, c.cid);
}

/// Each number is refused out of its range, a role given a number and a number given a source too, with what works.
#[test]
fn the_canons_numbers_say_what_is_wrong() {
    let cases: [(&str, &str, &str); 7] = [
        ("arrange from template(canon, @mgm_pease_porridge_hot, delay=8)\n", "8", "1 to 7 beats for a subject of 2 bars"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4], delay=4)\n", "4", "1 to 3 beats"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, interval=30)\n", "30", "-24 to 24"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, delay=ode_to_joy)\n", "ode_to_joy", "a whole number"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=3)\n", "3", "is a role"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, delay=2.5)\n", "2.5", "a whole number"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, dealy=2)\n", "dealy", "did you mean `delay`"),
    ];
    for (src, at, msg) in cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a.starts_with(at) && m.contains(msg)), "{src}: {e:?}");
    }
    // a number belongs to the canon
    let e = errors("arrange from template(rondo, @shanty_drunken_sailor, delay=2)\n");
    assert!(e.iter().any(|(a, m)| a == "delay" && m.contains("is no role of the rondo")), "{e:?}");
}

// ---------------------------------------------------------------- the quodlibet's roles

/// The quodlibet takes roles as the other forms do: two tunes, a ground and a third line, each moved into its key; the plan's
/// cross-morphs read them, and the fate motif's lets go with the third it was moved to make.
#[test]
fn the_quodlibet_names_its_roles() {
    let src = "arrange from template(quodlibet, @psalm42_deep_calleth, a=badinerie, b=new_world_largo, ground=<[d2 ~ ~ ~ a1 ~ ~ ~] [b1 ~ ~ ~ f#1 ~ ~ ~]>, third=mountain_king)\n";
    let text = plays_as_written(src);
    for want in ["# a, bound: badinerie\nlet tune_a_1 = ", "# b, bound: new_world_largo\nlet tune_b_1 = ", "# ground, bound: <[d2", "# third, bound: mountain_king\nlet third_1 = ", "~ tune_b_* morph tune_a_1 0.25", "~ tune_a_* morph tune_b_1 0.25", "@ tune_a_* morph third_1 0.3", "| melody third inst=trumpet"] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    assert!(!text.contains("beethoven_fate") && !text.contains("fate_b") && !text.contains("pachelbel_"), "{text}");
    assert!(text.contains("# a: badinerie; the first tune") && text.contains("# third: mountain_king; the third line"), "the header:\n{text}");
    // one role bound, the others the form's own
    let one = plays_as_written("arrange from template(quodlibet, @psalm42_deep_calleth, third=ode_to_joy)\n");
    assert!(one.contains("let pachelbel_violin = <") && one.contains("let ode_to_joy = <") && one.contains("let third = <") && !one.contains("fate_b"), "{one}");
    assert_eq!(djlive::roles::forms_with_roles()[0], "quodlibet");
    let e = errors("arrange from template(quodlibet, @psalm42_deep_calleth, thrid=ode_to_joy)\n");
    assert!(e.iter().any(|(a, m)| a == "thrid" && m.contains("`third`")), "{e:?}");
}

/// The quodlibet in a role of another form: its roles bound there, its names under the role's prefix.
#[test]
fn a_quodlibet_with_roles_fills_a_role() {
    let src = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(quodlibet, third=ode_to_joy))\n";
    let text = plays_as_written(src);
    assert!(text.contains("let ep_third_1 = ") && text.contains("section ep_canon 8 bars {\n  key d\n") && !text.contains("ep_fate_b"), "{text}");
}

// ---------------------------------------------------------------- the reference

/// The extensions read in the grammar's shapes: a form's section after its `)`, a part after a role, a number after `=`; what
/// does not read is said at it.
#[test]
fn the_extensions_read_and_are_said_at_what_does_not() {
    let p = djlive::parse::parse("arrange from template(canon, @x, subject=template(cross, a.bass=[d2 ~]).both.arp, delay=6, interval=-12)\n");
    assert!(p.ok(), "{:?}", p.diagnostics);
    for (src, msg) in [
        ("arrange from template(canon, @x, delay=-)\n", "starts a number"),
        ("arrange from template(canon, @x, subject=template(cross).both.arp.bass)\n", "a section and a part at most"),
        ("arrange from template(canon, @x, subject=template(cross).both.drop)\n", "one of the roles"),
    ] {
        let p = djlive::parse::parse(src);
        assert!(p.diagnostics.iter().any(|d| d.is_error() && d.message.contains(msg)), "{src}: {:?}", p.diagnostics);
    }
}

/// The examples of the extensions check clean, are canonical, say what they are, and are what they say: a canon of the cross's
/// overlap with its copy moved, a quodlibet with every role bound.
#[test]
fn the_examples_of_the_extensions_are_what_they_say() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript");
    for (name, wants, written) in [
        ("canon-of-the-crossing", &["subject=template(cross).both.arp", "delay=6", "interval=7"][..], &["| arp transpose(follow, 7) inst=", "let follow = <rotate(subject_"][..]),
        ("quodlibet-of-four", &["a=badinerie", "b=new_world_largo", "ground=<[d2", "third=mountain_king"][..], &["let tune_a_1 = ", "let third_1 = ", "@ tune_a_* morph third_1 0.3"][..]),
    ] {
        let src = std::fs::read_to_string(dir.join(format!("{name}.beatscript"))).unwrap();
        let c = clean(&src);
        assert_eq!(clean(&c.text).cid, c.cid, "{name}: the canonical text is a fixed point");
        for w in wants {
            assert!(c.text.contains(w), "{name}: {w}");
        }
        let text = plays_as_written(&src);
        for w in written {
            assert!(text.contains(w), "{name}: {w}\n{text}");
        }
        assert!(src.lines().filter(|l| l.starts_with("# ")).count() >= 3, "{name} says what it is and what you hear");
    }
}

// ---------------------------------------------------------------- what the review found (val run 16aa6899)

/// A program and its CID, in a store `check` reads from.
fn stored_env(texts: &[&str]) -> (djlive::env::StaticSnippets, Vec<String>) {
    let mut s = djlive::env::StaticSnippets::default();
    let mut cids = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        let c = clean(t);
        s.items.push((format!("p{i}"), c.cid.clone(), c.text.clone()));
        cids.push(c.cid);
    }
    (s, cids)
}

fn with_store<'a>(s: &'a djlive::env::StaticSnippets, v: &'a StaticVoices) -> Env<'a> {
    Env { voices: Some(v), snippets: Some(s), ..Env::default() }
}

/// After a program that is no set, `.track2` is its section called `track2`, as such a reference always read: written out as it
/// was before sets were sources (`section episode from <cid>.track2`, `let tune_1 = <cid>.track2.melody`). Only a program that
/// has no such section is told it is no set.
#[test]
fn track_n_after_a_program_that_is_no_set_is_its_section_as_it_always_was() {
    let tr = "tempo 120\nkey c\nsection track2 4 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ g1 ~\n  deck B: melody e4 g4 c5 g4\n}\narrange track2\n";
    let (s, cids) = stored_env(&[tr]);
    let cid = &cids[0];
    let v = StaticVoices::default();
    let env = with_store(&s, &v);
    let short = djengine::rights::short_cid(cid);
    let ok = |src: &str| -> String {
        let c = check_with(src, &env);
        let bad: Vec<String> = c.diagnostics.iter().filter(|d| d.severity != djlive::Severity::Info).map(|d| d.message.clone()).collect();
        assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
        match template::expand(src, &env) {
            Some(Ok(e)) => e.text,
            other => panic!("{other:?}"),
        }
    };
    let t = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid}.track2)\n"));
    assert!(t.contains(&format!("# episodes, bound: {cid}.track2\n# episode: the track2 of {short}\nsection episode from {cid}.track2\n")), "{t}");
    let t = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={cid}.track2.melody)\n"));
    assert!(t.contains(&format!("# refrain, bound: {cid}.track2.melody\nlet tune_1 = {cid}.track2.melody\nlet tune = <tune_1>\n")), "{t}");
    let n: Vec<String> = check_with(&format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={cid}.track2.melody)\n"), &env).diagnostics.into_iter().filter(|d| d.severity == djlive::Severity::Info).map(|d| d.message).collect();
    assert!(n.iter().any(|m| m == &format!("refrain: the melody of {short}: one let (`tune_*`)")), "{n:?}");
    // the canonical spelling is the one it always had
    assert_eq!(clean_with(&format!("arrange from template( rondo,@shanty_drunken_sailor,episodes = {cid}.track2 )\n"), &env).text, format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid}.track2)\n"));
    // a section it does not have is still refused
    let e: Vec<String> = check_with(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid}.track3)\n"), &env).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.unwrap_or_default())).collect();
    assert!(e.iter().any(|m| m.contains("is not a set, and has no section `track3`") && m.contains("its sections: track2")), "{e:?}");
}

fn clean_with(src: &str, env: &Env) -> Compiled {
    let c = check_with(src, env);
    let bad: Vec<String> = c.diagnostics.iter().filter(|d| d.severity != djlive::Severity::Info).map(|d| d.message.clone()).collect();
    assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
    c.compiled.unwrap()
}

/// The notes of a form's section are a function of the reference alone: the program's tempo (written or not, 96 or 97) leaves
/// them as they are, and its key moves them by the key rule and nothing else (B minor, D's relative, is D's notes; E is D's
/// moved up a whole tone).
#[test]
fn a_forms_section_is_the_same_notes_at_any_tempo_and_moved_by_key_only() {
    let reference = "arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both.arp)\n";
    let subject = |head: &str| lets_of(&expanded(&format!("{head}{reference}")), "subject");
    let d = subject("");
    assert!(!d.is_empty());
    for head in ["tempo 96\n", "tempo 97\n", "tempo 120\n", "tempo 96\nswing 0.1\n", "tempo 96\nkey bm\n"] {
        assert_eq!(subject(head), d, "{head}");
    }
    let up = |line: &str| -> String {
        line.split(' ').map(|w| {
            let (body, bang) = w.strip_suffix('!').map_or((w, ""), |b| (b, "!"));
            match djlive::music::parse_note(body) {
                Some((pc, Some(oct))) => format!("{}{bang}", djlive::seed::note_name((djlive::music::midi(pc, oct) + 2) as u8)),
                _ => w.to_string(),
            }
        }).collect::<Vec<_>>().join(" ")
    };
    let e = subject("tempo 96\nkey e\n");
    assert_eq!(e, d.iter().map(|l| up(l)).collect::<Vec<_>>(), "E is D moved up 2");
    // what the inner form's own bindings say is its own: no note names a let the program does not have
    let n = notes("arrange from template(canon, @mgm_pease_porridge_hot, subject=template(canon, delay=6, interval=7).chase.arp)\n");
    assert!(n.iter().all(|m| !m.starts_with("delay:") && !m.starts_with("interval:") && !m.contains("pachelbel_violin")), "{n:?}");
}

/// A bar of one step that the plan morphs or crosses toward is written in brackets, as a seed's is, so `morph` and `xfer` take
/// it (bare, it was refused). Anywhere else it stays bare, as it always was: its text, so its CID, so the plan's dice, do not move.
#[test]
fn a_bar_of_one_step_the_plan_morphs_toward_is_written_in_brackets() {
    let t = plays_as_written("arrange from template(cross, @psalm42_deep_calleth, a.bass=[d2], a.melody=[d5])\n");
    assert!(t.contains("let a_bass_1 = [d2]\n") && t.contains("let a_melody_1 = [d5]\n"), "{t}");
    assert!(t.contains("morph a_bass_1 0.25") && t.contains("morph a_melody_1 0.25"), "{t}");
    let t = plays_as_written("arrange from template(canon, @mgm_pease_porridge_hot, ground=[d2])\n");
    assert!(t.contains("let ground_1 = d2\n"), "no plan moves toward the ground: as it was\n{t}");
}

/// A stored subject is one let however many bars it has: the canon's delay is then within the bar, and the refusal says why.
#[test]
fn a_stored_subjects_delay_is_within_the_bar_and_says_why() {
    let song = "tempo 90\nkey d\nsection verse 8 bars {\n  deck A: kick x ~ x ~ | bass d2 ~ a1 ~\n  deck B: melody <[d5 c#5 b4 a4] [b4 a4 g4 f#4]>\n}\narrange verse\n";
    let (s, cids) = stored_env(&[song]);
    let v = StaticVoices::default();
    let env = with_store(&s, &v);
    let cid = &cids[0];
    let e: Vec<String> = check_with(&format!("arrange from template(canon, @mgm_pease_porridge_hot, subject={cid}.verse.melody, delay=4)\n"), &env).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.unwrap_or_default())).collect();
    assert!(e.iter().any(|m| m.contains("the subject is a stored part, one let (`subject_1`)") && !m.contains("subject of 1 bar")), "{e:?}");
    clean_with(&format!("arrange from template(canon, @mgm_pease_porridge_hot, subject={cid}.verse.melody, delay=2)\n"), &env);
}

/// What a template's roles name by CID is what a store credits (stored::cids_of): a bare CID, a section, a set's track, and the
/// stored programs of a form in a role.
#[test]
fn the_cids_a_role_names_are_credited() {
    let a = "z4EBG9j9Z3mPn1vKYrEVc93WXEjYyL7bCGdBM7jX4h8ZWMEuYZx";
    let b = "z4EBG9j7BJKjYPfs6QQBQCdjieviWbSwbJqEPm9idg79wTYNLW1";
    let c = "z4EBG9izUsbisU2JZtPHUo4hcQpYd8FLyZXLqKUmbtftjj3Lirw";
    let src = format!("arrange from template(rondo, @x, refrain={a}.track1.hook.melody, episodes=template(cross, a={b}, b={c}.verse))\n");
    let got = djlive::stored::cids_of(&src);
    assert_eq!(got.len(), 3, "once each: {got:?}");
    for want in [a, b, c] {
        assert!(got.iter().any(|g| g == want), "{want}: {got:?}");
    }
    assert_eq!(djlive::stored::cids_of(&format!("arrange from template(rondo, @x, episodes={a})\n")), [a.to_string()]);
}
