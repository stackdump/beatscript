//! Roles: a form template's parts bound to sources (djlive::roles, docs/beatscript/ext-roles.md). Here, where the seeded generator is
//! installed, so a role can be bound to a seed (`gen@1 dubstep seed=54876`):
//! - every form with roles checks clean with a seed bound to each of its roles, and its written-out text checks clean alone;
//! - two seeds in different native keys cross cleanly in `cross`: both fitted to the program's key, every note in it;
//! - the same inputs are the same CID and the same nets, however the reference is spelt;
//! - forms nest three deep, a line is at most 16 bars and a role of sections at most 128 a pass, each an error at the reference;
//! - a nested form (a rondo of passacaglias from one seed) ejects level by level, playing the same at every level, its
//!   provenance a chain;
//! - a stored program's section, and a whole stored program, fill a role that takes sections.

use std::collections::BTreeSet;

use djlive::compile::Compiled;
use djlive::diag::Severity;
use djlive::env::{Env, StaticSnippets};
use djlive::refs;

fn env() -> Env<'static> {
    beatscript_api::web_env().env()
}

fn check(src: &str) -> djlive::compile::Check {
    beatscript_api::generate::install();
    djlive::compile::check_with(src, &env())
}

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    let errs: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("`{}` {} {}", d.span.text(src), d.message, d.help.clone().unwrap_or_default())).collect();
    c.compiled.unwrap_or_else(|| panic!("{src}\n{errs:#?}"))
}

/// The warnings and errors of a check (not its notes).
fn complaints(c: &djlive::compile::Check) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.severity != Severity::Info).map(|d| format!("{:?}: {}", d.severity, d.message)).collect()
}

fn clean(src: &str) -> Compiled {
    let c = check(src);
    let bad = complaints(&c);
    assert!(bad.is_empty(), "{src}\n{bad:#?}");
    compiled(src)
}

/// Each segment's name, place, length and the nets its decks play.
fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect())).collect()
}

fn errors(src: &str) -> Vec<(String, String)> {
    check(src).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
}

/// The program a reference is written out as (the template stage alone).
fn written(src: &str) -> String {
    beatscript_api::generate::install();
    match djlive::template::expand(src, &env()) {
        Some(Ok(e)) => e.text,
        other => panic!("{src}: {other:?}"),
    }
}

const A: &str = "gen@1 dubstep seed=54876";
const B: &str = "gen@1 house seed=7";

/// Every form with roles takes a seed in each role, all at once: it checks with no warning, the arrangement's mutations reach
/// what the seeds wrote, and the written-out program checks clean on its own and plays the same.
#[test]
fn every_form_checks_clean_with_a_seed_in_each_role() {
    let cases = [
        format!("arrange from template(quodlibet, @psalm42_deep_calleth, a={A}, b={B}, ground={A} part=bass, third={B} part=bass)\n"),
        format!("arrange from template(passacaglia, @eccl3_season, ground={A} section=drop part=bass, upper={B})\n"),
        format!("arrange from template(canon, @mgm_pease_porridge_hot, subject={A}, ground={B} part=bass)\n"),
        format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={A}, episodes={B} part=melody)\n"),
        format!("arrange from template(fugue, @tyger_burning_bright, subject={A}, counter={B})\n"),
        format!("arrange from template(theme-and-variations, @dickinson_chariot_death, theme={A})\n"),
        format!("arrange from template(cross, @psalm42_deep_calleth, a={A}, b={B})\n"),
    ];
    let mut forms = BTreeSet::new();
    for src in &cases {
        let c = clean(src);
        let name = src.split(['(', ',']).nth(1).unwrap().to_string();
        forms.insert(name.clone());
        // every role bound says so, in a note at its source
        let notes = check(src).diagnostics.iter().filter(|d| d.severity == Severity::Info && d.message.contains(": gen@1") || d.message.contains("its melody gen@1")).count();
        assert!(notes >= 1, "{src}");
        // the written-out program is plain structure with the calls in it, checks clean, and plays the same
        let text = written(src);
        assert!(text.contains("= gen@1 ") && !text.lines().any(|l| !l.starts_with('#') && l.contains("template(")), "{text}");
        let alone = clean(&text);
        assert_eq!(nets(&alone), nets(&c), "{name}: the written-out program plays as the reference");
        // the mutation plan reads the bound lines: a `~` or `@` of the arrangement is aimed at one
        let plan: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("arrange {")).map(str::trim).filter(|l| l.starts_with('~') || l.starts_with('@')).collect();
        let bound = ["upper_*", "subject_*", "tune_*", "figure_*", "dux_*", "countersubject_*", "air_*", "a_melody_*", "b_bass_*", "tune_a_*", "tune_b_*"];
        assert!(plan.iter().any(|l| bound.iter().any(|b| l.contains(b))), "{name}: {plan:?}");
        // and a pattern a seed wrote has generations in check's table
        let g = djlive::mutate::generations(src, &env()).expect("an arrange table");
        let fired: Vec<&String> = g.patterns.keys().filter(|k| ["upper_", "subject_", "tune_", "figure_", "dux_", "countersubject_", "a_melody_", "b_bass_", "drm_", "third_"].iter().any(|p| k.starts_with(p))).collect();
        assert!(!fired.is_empty(), "{name}: {:?}", g.patterns.keys().collect::<Vec<_>>());
    }
    let with: BTreeSet<String> = djlive::roles::forms_with_roles().into_iter().map(String::from).collect();
    assert_eq!(forms, with, "every form with roles is tried");
}

/// Two seeds of different genres, in different native keys, cross in `cross` without a clash: each call is fitted to the
/// program's key (check says so), and every note the two seeds play there is in it.
#[test]
fn two_seeds_in_different_native_keys_cross_cleanly() {
    let native = |call: &str| -> String {
        let r = djlive::gen::resolve_word(call, &djlive::gen::Host::default()).unwrap();
        r.text.lines().find_map(|l| l.strip_prefix("key ")).unwrap().to_string()
    };
    let (ka, kb) = (native(A), native(B));
    assert_ne!(ka, kb, "the two seeds are in different keys of their own");
    for key in ["d", "am"] {
        let src = format!("key {key}\narrange from template(cross, @psalm42_deep_calleth, a={A}, b={B})\n");
        let c = clean(&src);
        let fitted: Vec<String> = check(&src).diagnostics.iter().filter(|d| d.severity == Severity::Info && d.message.contains("generated in")).map(|d| d.message.clone()).collect();
        assert_eq!(fitted.len(), 6, "a bass, a melody and the chords of each: {fitted:?}");
        assert!(fitted.iter().all(|n| n.contains(&format!("the program's key {key}"))), "{fitted:?}");
        // every tonal note of the crossing is in the key
        let k = djlive::music::Key::parse(key).unwrap();
        let scale: BTreeSet<i32> = (1..=7).map(|d| k.degree(d, 4).rem_euclid(12)).collect();
        let mut off = BTreeSet::new();
        for seg in c.segments.iter().filter(|s| ["a_leads", "b_leads", "both"].contains(&s.name.as_str())) {
            for deck in seg.decks.values() {
                for p in deck.parts.iter().filter(|p| ["bass", "melody", "harmony", "arp"].contains(&p.role.as_str())) {
                    for bar in &p.cycles {
                        for h in bar {
                            if !scale.contains(&(i32::from(h.note).rem_euclid(12))) {
                                off.insert((seg.name.clone(), p.role.clone(), h.note));
                            }
                        }
                    }
                }
            }
        }
        assert!(off.is_empty(), "in {key}, notes out of the key: {off:?}");
    }
}

/// The same inputs are the same text, CID and nets, however the reference is spelt (roles in any order, spaces, `gen` for
/// `gen@2`); another seed is another CID.
#[test]
fn the_same_inputs_give_the_same_cid() {
    let src = format!("arrange from template(passacaglia, @eccl3_season, ground={A} part=bass, upper=ode_to_joy)\n");
    let (x, y) = (compiled(&src), compiled(&src));
    assert_eq!((x.cid.clone(), nets(&x)), (y.cid.clone(), nets(&y)));
    let respelt = "arrange from template( passacaglia , @eccl3_season , upper = ode_to_joy , ground = gen@1  dubstep part=bass seed=54876 )\n";
    let z = compiled(respelt);
    assert_eq!((z.cid.clone(), z.text.clone()), (x.cid.clone(), x.text.clone()));
    assert_eq!(x.text, format!("arrange from template(passacaglia, @eccl3_season, ground={A} part=bass, upper=ode_to_joy)\n"), "the canonical spelling: the form's order");
    assert_ne!(compiled(&src.replace("54876", "54877")).cid, x.cid);
    assert_eq!(written(&src), written(respelt));
}

/// Composition stays total: a fourth form inside forms, a line of more than 16 bars, and more than 128 bars of sections are
/// each an error at the reference that crosses the cap.
#[test]
fn the_caps_are_errors_at_the_reference() {
    let deep = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(rondo, episodes=template(rondo, episodes=template(passacaglia))))\n";
    let e = errors(deep);
    assert!(e.iter().any(|(at, m)| at == "template(passacaglia)" && m.contains("nest 3 deep")), "{e:?}");
    // three deep is fine
    clean("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(rondo, episodes=template(passacaglia)))\n");
    let long = format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={A} length=long)\n");
    let e = errors(&long);
    assert!(e.iter().any(|(at, m)| *at == format!("{A} length=long") && m.contains("at most 128")), "{e:?}");
    let bars = vec!["[g1 ~ d2 ~]"; 17].join(" ");
    let e = errors(&format!("arrange from template(passacaglia, @eccl3_season, ground=<{bars}>)\n"));
    assert!(e.iter().any(|(at, m)| at.starts_with("<[g1") && m.contains("at most 16")), "{e:?}");
}

/// A rondo of passacaglias from one seed: the refrain is its melody, and every episode a passacaglia over its bass. Eject
/// writes the rondo out under a `# from:` line that is the whole chain (`template(rondo, ..., episodes=template(passacaglia,
/// ground=gen@1 ...))`); each call left in it ejects in turn; every level plays the same.
#[test]
fn a_nested_form_ejects_level_by_level_and_plays_the_same() {
    let src = format!("tempo 140\narrange from template(rondo, @shanty_drunken_sailor, refrain={A} part=melody, episodes=template(passacaglia, ground={A} part=bass, upper={A} part=melody))\n");
    let c = clean(&src);
    let ss = refs::sites(&src, &env()).unwrap();
    assert_eq!(ss.len(), 1, "a bound source is the template's: {ss:?}");
    assert_eq!((ss[0].kind, ss[0].half), ("template", refs::STRUCTURE));
    let mut text = refs::eject(&src, &env(), ss[0].span.start).unwrap().unwrap();
    let line = text.lines().find(|l| l.starts_with("# from: ")).unwrap();
    let (call, cid) = refs::read_from_line(line).unwrap();
    assert!(call.contains(&format!("episodes=template(passacaglia, ground={A} part=bass, upper={A} part=melody)")), "{call}");
    assert!(cid.is_some());
    assert_eq!(nets(&clean(&text)), nets(&c), "the rondo written out:\n{text}");
    // the passacaglia's sections are there, in G minor, its calls saying so
    assert!(text.contains("section ep_ground 8 bars {\n  key gm") && text.contains(&format!("let ep_ostinato = {A} key=gm section=drop part=bass")), "{text}");
    // then every call, one at a time
    let mut n = 0;
    loop {
        let sites = refs::sites(&text, &env()).unwrap();
        let Some(s) = sites.first() else { break };
        assert_eq!(s.kind, "gen", "{s:?}");
        text = refs::eject(&text, &env(), s.span.start).unwrap().unwrap();
        assert_eq!(nets(&compiled(&text)), nets(&c), "after {} ejects:\n{text}", n + 1);
        n += 1;
    }
    assert_eq!(n, 3);
    assert!(text.lines().filter(|l| !l.trim_start().starts_with('#')).all(|l| !l.contains("gen@1")), "{text}");
}

/// The sections a program plays, in order, one pass (`take`), their generation marks off.
fn played_sections(c: &Compiled, prefix: &str, take: usize) -> Vec<String> {
    c.segments.iter().filter(|s| !s.chain && s.name.starts_with(prefix)).map(|s| s.name.split('@').next().unwrap().to_string()).take(take).collect()
}

/// A stored program fills a role: its section stands in for the rondo's episode, the whole of it (its sections, in its order)
/// for every episode, and its part for the refrain. Each checks clean (the source has no control lines to leave behind).
#[test]
fn a_stored_program_fills_a_role() {
    let source = "tempo 120\nkey c\nsection verse 4 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ g1 ~\n  deck B: melody e4 g4 c5 g4\n}\nsection hook 4 bars {\n  deck A: kick x x x x | bass f1 ~ c2 ~\n  deck B: melody a4 c5 f5 c5\n}\narrange verse hook verse\n";
    let comp = compiled(source);
    let mut s = StaticSnippets::default();
    s.items.push(("song".into(), comp.cid.clone(), comp.text.clone()));
    let with = Env { snippets: Some(&s), ..env() };
    let ok = |src: &str| {
        let c = djlive::compile::check_with(src, &with);
        let bad = complaints(&c);
        assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
        c.compiled.unwrap()
    };
    let cid = &comp.cid;
    let one = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid}.hook)\n"));
    assert!(one.segments.iter().any(|s| s.name.starts_with("episode") && s.bars == 4));
    let whole = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid})\n"));
    assert_eq!(played_sections(&whole, "ep_", 3), ["ep_verse", "ep_hook", "ep_verse"], "its sections, in its order, over the episodes");
    ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={cid}.hook.melody)\n"));
    ok(&format!("arrange from template(cross, @psalm42_deep_calleth, a={A}, b={cid}.verse.melody)\n"));
    // one no host has is the usual error, at the reference
    let e: Vec<String> = check(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid}.hook)\n")).diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(e.iter().any(|m| m.contains("is not here")), "{e:?}");
}

/// A whole seed in a role that takes sections is its sections, in its order; what each leaves behind (its filters) is said.
#[test]
fn a_whole_seed_is_its_sections() {
    let src = format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={A})\n");
    let c = check(&src);
    assert!(c.compiled.is_some(), "{:?}", complaints(&c));
    let w = complaints(&c);
    assert!(!w.is_empty() && w.iter().all(|m| m.contains("control lines") && m.contains("a borrowed section does not bring")), "{w:#?}");
    assert_eq!(played_sections(&c.compiled.unwrap(), "ep_", 6), ["ep_intro", "ep_buildup", "ep_drop", "ep_verse", "ep_drop", "ep_outro"]);
}

/// The examples that bind seeds (`#! skip-test` in tests/examples_beatscript.rs, which has no generator) check clean here, are
/// canonical, and are what they say: a crossing of two seeds, a rondo of passacaglias from one.
#[test]
fn the_examples_with_seeds_check_clean() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript");
    for (name, wants) in [("cross-two-seeds", &["template(cross, ", "a=gen@1 ", "b=gen@1 "][..]), ("rondo-of-passacaglias", &["template(rondo, ", "episodes=template(passacaglia, "][..])] {
        let src = std::fs::read_to_string(dir.join(format!("{name}.beatscript"))).unwrap();
        assert!(djlive::linked::header_lines(&src).iter().any(|h| h.key == "skip-test"), "{name}");
        let c = clean(&src);
        assert_eq!(compiled(&c.text).cid, c.cid, "{name}: the canonical text is a fixed point");
        for w in wants {
            assert!(c.text.contains(w), "{name}: {w}");
        }
        assert!(src.lines().filter(|l| l.starts_with("# ")).count() >= 3, "{name} says what it is and what you hear");
    }
}

/// What is wrong with a binding is said at it.
#[test]
fn check_names_what_is_wrong_at_the_binding() {
    let cases: [(String, &str, &str); 6] = [
        (format!("arrange from template(passacaglia, @eccl3_season, grund={A})\n"), "grund", "`ground`"),
        (format!("arrange from template(passacaglia, @eccl3_season, ground={A}, ground=ode_to_joy)\n"), "ground", "bound twice"),
        ("arrange from template(passacaglia, @eccl3_season, upper=ode_to_jy)\n".into(), "ode_to_jy", "`ode_to_joy`"),
        (format!("arrange from template(deep-house, @eccl3_season, a={A})\n"), "a", "takes no roles yet"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=gymnopedie_melody)\n".into(), "gymnopedie_melody", "bars of 16"),
        (format!("arrange from template(rondo, @shanty_drunken_sailor, refrain=template(passacaglia, ground={A}))\n"), "template(passacaglia", "takes a line"),
    ];
    for (src, at, msg) in &cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a.starts_with(at) && m.contains(msg)), "{src}: {e:?}");
    }
    let e = errors("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, @eccl3_season))\n");
    assert!(e.iter().any(|(_, m)| m.contains("takes no phrase")), "{e:?}");
}

/// Two stored tracks and a set that plays them, with a mix between: what the set tests below bind. (The tracks have no control
/// lines, so a borrowed section leaves nothing behind.)
fn a_set() -> (StaticSnippets, String, String, String) {
    let one = "tempo 120\nkey c\nsection verse 4 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ g1 ~\n  deck B: melody e4 g4 c5 g4\n}\nsection hook 4 bars {\n  deck A: kick x x x x | bass f1 ~ c2 ~\n  deck B: melody a4 c5 f5 c5\n}\narrange verse hook verse\n";
    let two = "tempo 124\nkey am\nsection intro 4 bars {\n  deck A: kick x ~ ~ ~ | bass a1 ~ e2 ~\n}\nsection lift 8 bars {\n  deck A: kick x ~ x ~ | bass a1 ~ c2 ~\n  deck B: melody a4 e5 c5 e5 | harmony [a3, c4, e4] ~ ~ ~\n}\narrange intro lift\n";
    let (c1, c2) = (compiled(one), compiled(two));
    let mut s = StaticSnippets::default();
    s.items.push(("one".into(), c1.cid.clone(), c1.text.clone()));
    s.items.push(("two".into(), c2.cid.clone(), c2.text.clone()));
    let set = format!("set {{\n  play {}\n  mix 4 bars with crossfade\n  play {}\n}}\nend\n", c1.cid, c2.cid);
    let with = Env { snippets: Some(&s), ..env() };
    let cs = djlive::compile::check_with(&set, &with).compiled.unwrap_or_else(|| panic!("the set compiles"));
    drop(with);
    s.items.push(("the set".into(), cs.cid.clone(), cs.text.clone()));
    (s, cs.cid, c1.cid, c2.cid)
}

/// A stored set fills a role of sections: its tracks in order, each its sections in its order, one track an episode; its mixes
/// stay behind (a note says so), and the set itself goes with the program for a link to carry. Its track (`.track2`) is that
/// track's program: its sections, a section, a part.
#[test]
fn a_stored_set_fills_a_role_of_sections() {
    let (s, set, t1, t2) = a_set();
    let with = Env { snippets: Some(&s), ..env() };
    let ok = |src: &str| {
        let c = djlive::compile::check_with(src, &with);
        let bad = complaints(&c);
        assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
        (c.compiled.unwrap(), c.diagnostics)
    };
    let src = format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set})\n");
    let (whole, diags) = ok(&src);
    assert_eq!(played_sections(&whole, "ep_", 5), ["ep_t1_verse", "ep_t1_hook", "ep_t1_verse", "ep_t2_intro", "ep_t2_lift"], "track 1 in the first episode, track 2 in the second");
    assert!(diags.iter().any(|d| d.severity == Severity::Info && d.message.contains("the set") && d.message.contains("2 tracks") && d.message.contains("its mixes and cuts stay behind")), "{diags:#?}");
    assert!(whole.plays.iter().any(|(c, _)| *c == set), "the set goes with the program: {:?}", whole.plays.iter().map(|p| &p.0).collect::<Vec<_>>());
    assert!(whole.plays.iter().any(|(c, _)| *c == t1) && whole.plays.iter().any(|(c, _)| *c == t2));
    let text = match djlive::template::expand(&src, &with) {
        Some(Ok(e)) => e.text,
        other => panic!("{other:?}"),
    };
    assert!(text.contains(&format!("# ep_t2_lift: the lift of track 2 of the set {}\nsection ep_t2_lift from {t2}.lift", djengine::rights::short_cid(&set))), "{text}");
    assert_eq!(ok(&whole.text).0.cid, whole.cid, "the canonical text is a fixed point");
    // a track: its sections; its section; its part; a section where a line goes is the role's part of it
    let (t, _) = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.track2)\n"));
    assert_eq!(played_sections(&t, "ep_", 2), ["ep_intro", "ep_lift"]);
    let (t, _) = ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.track2.lift)\n"));
    assert!(t.segments.iter().any(|g| g.name.starts_with("episode") && g.bars == 8));
    ok(&format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={set}.track1.hook.melody)\n"));
    let line = format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={set}.track1.hook)\n");
    ok(&line);
    let (_, d) = ok(&line);
    assert!(d.iter().any(|x| x.message.contains("refrain: the melody of the hook of track 1 of the set")), "{d:#?}");
    // spelt one way
    let (c, _) = ok(&format!("arrange from template( rondo,@shanty_drunken_sailor,episodes = {set}.track2 )\n"));
    assert_eq!(c.text, format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.track2)\n"));
}

/// What is wrong with a set in a role is said at it.
#[test]
fn a_stored_set_says_what_is_wrong() {
    let (s, set, t1, _) = a_set();
    let with = Env { snippets: Some(&s), ..env() };
    let errs = |src: &str| -> Vec<(String, String)> {
        djlive::compile::check_with(src, &with).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
    };
    let cases: Vec<(String, &str)> = vec![
        (format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.track3)\n"), "has 2 tracks"),
        (format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={t1}.track1)\n"), "is not a set"),
        (format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={set})\n"), "this is a set"),
        (format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.verse)\n"), "it has tracks, not sections"),
        (format!("arrange from template(rondo, @shanty_drunken_sailor, refrain={set}.track2.intro)\n"), "plays no melody"),
    ];
    for (src, msg) in &cases {
        let e = errs(src);
        assert!(e.iter().any(|(at, m)| at.starts_with('z') && m.contains(msg)), "{src}: {e:?}");
    }
}

/// A stored program's section where a line goes is the role's part of it, and in a seed role each of its parts; one part of a
/// seed role is bound alone beside a seed (`a=gen@1 ..., a.bass=[...]`), the seed giving the rest.
#[test]
fn a_section_is_a_line_and_a_part_is_bound_alone() {
    let source = "tempo 120\nkey d\nsection verse 4 bars {\n  deck A: kick x ~ x ~ | bass d2 ~ a1 ~\n  deck B: melody f#4 a4 d5 a4 | harmony [d3, f#3, a3] ~ ~ ~\n}\narrange verse\n";
    let comp = compiled(source);
    let mut s = StaticSnippets::default();
    s.items.push(("song".into(), comp.cid.clone(), comp.text.clone()));
    let with = Env { snippets: Some(&s), ..env() };
    let ok = |src: &str| {
        let c = djlive::compile::check_with(src, &with);
        let bad = complaints(&c);
        assert!(bad.is_empty() && c.compiled.is_some(), "{src}\n{bad:#?}");
        match djlive::template::expand(src, &with) {
            Some(Ok(e)) => e.text,
            other => panic!("{other:?}"),
        }
    };
    let cid = &comp.cid;
    let t = ok(&format!("arrange from template(canon, @mgm_pease_porridge_hot, subject={cid}.verse)\n"));
    assert!(t.contains(&format!("let subject_1 = {cid}.verse.melody")), "{t}");
    let t = ok(&format!("arrange from template(cross, @psalm42_deep_calleth, a={cid}.verse)\n"));
    for part in ["melody", "bass", "harmony"] {
        assert!(t.contains(&format!("let a_{part}_1 = {cid}.verse.{part}")), "{part}: {t}");
    }
    let t = ok(&format!("arrange from template(cross, @psalm42_deep_calleth, a={A}, a.bass=<[d2 ~ ~ ~ a1 ~ ~ ~] [g1 ~ ~ ~ a1 ~ ~ ~]>, b={B})\n"));
    assert!(t.contains(&format!("let a_melody = {A} section=")) && t.contains(&format!("let a_harmony = {A} section=")) && !t.contains(&format!("let a_bass = {A}")), "{t}");
    assert!(t.contains("# a.bass, bound: <[d2"), "{t}");
    // the seed is generated in the program's key, and checks with the bound bass under it
    let e: Vec<String> = djlive::compile::check_with(&format!("arrange from template(cross, @psalm42_deep_calleth, a={A} part=bass, a.bass=[d2 ~])\n"), &with).diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(e.iter().any(|m| m.contains("bass is bound twice")), "{e:?}");
}

/// A set in a set: its tracks are named by their place (`ep_t2_1_...`: track 1 of track 2), as `.trackN` counts them; `.track0`
/// and its kin are told that tracks count from 1; the 128-bar cap's way out names the set's real track only for a set.
#[test]
fn a_set_in_a_set_and_what_a_track_is_not() {
    let (mut s, set, t1, t2) = a_set();
    let three = "tempo 122\nkey am\nsection drop 4 bars {\n  deck A: kick x x x x | bass a1 ~ ~ ~\n}\narrange drop\n";
    let c3 = compiled(three);
    s.items.push(("three".into(), c3.cid.clone(), c3.text.clone()));
    let with = Env { snippets: Some(&s), ..env() };
    let outer_text = format!("set {{\n  play {t1}\n  play {set}\n}}\nend\n");
    let outer = djlive::compile::check_with(&outer_text, &with).compiled.expect("the outer set compiles");
    drop(with);
    s.items.push(("outer".into(), outer.cid.clone(), outer.text.clone()));
    let with = Env { snippets: Some(&s), ..env() };
    let src = format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={})\n", outer.cid);
    let c = djlive::compile::check_with(&src, &with);
    assert!(c.compiled.is_some(), "{:?}", complaints(&c));
    let text = match djlive::template::expand(&src, &with) {
        Some(Ok(e)) => e.text,
        other => panic!("{other:?}"),
    };
    let short = djengine::rights::short_cid(&outer.cid);
    assert!(text.contains(&format!("# ep_t2_2_lift: the lift of track 2 of track 2 of the set {short}\nsection ep_t2_2_lift from {t2}.lift")), "{text}");
    assert!(text.contains("section ep_t1_verse from ") && !text.contains("ep_t2_1_verse"), "track 1 of the inner set is the outer's track 1, written once:\n{text}");
    let errs = |src: &str| -> Vec<String> { djlive::compile::check_with(src, &with).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.unwrap_or_default())).collect() };
    for bad in ["track0", "track01", "trackx"] {
        let e = errs(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={set}.{bad})\n"));
        assert!(e.iter().any(|m| m.contains("counted from 1") && m.contains(&format!("episodes={set}.track1"))), "{bad}: {e:?}");
    }
    // the cap's way out: a set's real track for a set, nothing of tracks for a program that is no set
    let long: String = (0..9).map(|i| format!("section s{i} 16 bars {{\n  deck A: kick x ~ x ~\n}}\n")).collect::<String>() + &format!("arrange {}\n", (0..9).map(|i| format!("s{i}")).collect::<Vec<_>>().join(" "));
    let lc = compiled(&long);
    drop(with);
    s.items.push(("long".into(), lc.cid.clone(), lc.text.clone()));
    let big_set = format!("set {{\n  play {}\n}}\nend\n", lc.cid);
    let with = Env { snippets: Some(&s), ..env() };
    let bs = djlive::compile::check_with(&big_set, &with).compiled.expect("the big set compiles");
    drop(with);
    s.items.push(("big".into(), bs.cid.clone(), bs.text.clone()));
    let with = Env { snippets: Some(&s), ..env() };
    let errs = |src: &str| -> Vec<String> { djlive::compile::check_with(src, &with).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.unwrap_or_default())).collect() };
    let e = errs(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={})\n", lc.cid));
    assert!(e.iter().any(|m| m.contains("at most 128") && !m.contains("track")), "{e:?}");
    let e = errs(&format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={})\n", bs.cid));
    assert!(e.iter().any(|m| m.contains("at most 128") && m.contains(&format!("episodes={}.track1", bs.cid))), "{e:?}");
}
