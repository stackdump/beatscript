//! `use SOURCE as NAME` where the seeded generator is installed and a store has programs (djlive::uses, docs/beatscript/ext-roles.md
//! "Name a source"): a stored program's section, part and whole; a seed's section, part and whole; the examples that name them;
//! and what check and expand answer of the names (`imports`, a reference of kind `use`).
//! - a stored or generated section named with `use` plays as `section NAME from ...` typed, a part as `let NAME = ...` typed;
//! - in a role the name is its source: a rondo whose episodes are a named form of a named seed is the rondo written inline;
//! - a CID no host has is the usual error, at the line; a section the source lacks says the ones it has;
//! - eject writes a seed's line out as its notes, under its `# from:` line, and the result plays the same;
//! - the program's provenance credits the stored program, and the call.

use djlive::compile::{Check, Compiled};
use djlive::diag::Severity;
use djlive::env::{Env, StaticSnippets};
use djlive::refs;

fn env() -> Env<'static> {
    beatscript_api::generate::install();
    beatscript_api::web_env().env()
}

fn complaints(c: &Check) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.severity != Severity::Info).map(|d| format!("{:?}: {}", d.severity, d.message)).collect()
}

fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect())).collect()
}

const SONG: &str = "tempo 120\nkey c\nsection verse 4 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ g1 ~\n  deck B: melody e4 g4 c5 g4\n}\nsection hook 4 bars {\n  deck A: kick x x x x | bass f1 ~ c2 ~\n  deck B: melody a4 c5 f5 c5\n}\narrange verse hook verse\n";

/// A store with SONG in it, and its CID.
fn with_song<T>(f: impl FnOnce(&Env, &str) -> T) -> T {
    let comp = djlive::compile::check_with(SONG, &env()).compiled.expect("the song checks");
    let mut s = StaticSnippets::default();
    s.items.push(("song".into(), comp.cid.clone(), comp.text.clone()));
    let e = Env { snippets: Some(&s), ..env() };
    f(&e, &comp.cid)
}

fn ok(src: &str, e: &Env) -> Compiled {
    let c = djlive::compile::check_with(src, e);
    let bad = complaints(&c);
    assert!(c.compiled.is_some() && !c.diagnostics.iter().any(|d| d.is_error()), "{src}\n{bad:#?}");
    c.compiled.unwrap()
}

fn errors(src: &str, e: &Env) -> Vec<String> {
    djlive::compile::check_with(src, e).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.unwrap_or_default())).collect()
}

#[test]
fn a_stored_programs_section_part_and_whole() {
    with_song(|e, cid| {
        let named = format!("tempo 120\nuse {cid}.hook as hook\nuse {cid}.verse.bass as walk\nsection intro 4 bars {{\n  deck A: kick x ~ ~ ~ | bass walk\n}}\narrange intro hook\n");
        let typed = format!("tempo 120\nsection hook from {cid}.hook\nlet walk = {cid}.verse.bass\nsection intro 4 bars {{\n  deck A: kick x ~ ~ ~ | bass walk\n}}\narrange intro hook\n");
        let a = ok(&named, e);
        assert_eq!(nets(&a), nets(&ok(&typed, e)));
        // refs: two lines of kind `use`, each pinned to the CID
        let ss = refs::sites(&named, e).unwrap();
        let uses: Vec<(&str, &str, Option<&str>)> = ss.iter().filter(|s| s.kind == "use").map(|s| (s.place, s.name.as_deref().unwrap_or(""), s.resolved.as_deref())).collect();
        assert_eq!(uses, [("section", "hook", Some(cid)), ("let", "walk", Some(cid))]);
        // the store credits it as any borrow, and the program says it is based on it
        assert_eq!(djlive::stored::cids_of(&a.text), [cid.to_string()]);
        assert!(refs::based_on(&a.text, &a.notes).iter().any(|b| b.cid.as_deref() == Some(cid) && b.how == "borrowed"));
        // a whole program is a source for a role: its sections, in its order, over the rondo's episodes
        let inline = format!("arrange from template(rondo, @shanty_drunken_sailor, episodes={cid})\n");
        let by_name = format!("use {cid} as song\narrange from template(rondo, @shanty_drunken_sailor, episodes=song)\n");
        assert_eq!(nets(&ok(&by_name, e)), nets(&ok(&inline, e)));
        // a section it has not got, with the ones it has
        let bad = errors(&format!("use {cid}.hoook as hk\narrange hk\n"), e);
        assert!(bad.iter().any(|m| m.contains("has no section `hoook`") && m.contains("hook")), "{bad:?}");
    });
    // a CID no host has: the usual words, at the line
    let comp = djlive::compile::check_with(SONG, &env()).compiled.unwrap();
    let bad = errors(&format!("use {}.hook as hook\narrange hook\n", comp.cid), &env());
    assert!(bad.iter().any(|m| m.contains("is not here")), "{bad:?}");
}

#[test]
fn a_seeds_section_part_and_whole() {
    let e = env();
    let src = "tempo 124\nkey am\nuse gen@1 dubstep seed=54876 section=drop part=bass as walk\nsection verse 4 bars {\n  deck A: kick x ~ x ~ | bass walk\n}\narrange {\n  verse\n  ~ walk_* rot 2\n  verse\n}\n";
    let c = ok(src, &e);
    let ss = refs::sites(src, &e).unwrap();
    let s = ss.iter().find(|s| s.kind == "use").expect("the use");
    assert_eq!((s.half, s.place, s.generator.as_deref()), (refs::ENGINE, "let", Some("seeded@1")));
    assert!(s.resolved.as_deref().is_some_and(djlive::stored::is_cid) && s.fitted == ["key", "bpm"], "{s:?}");
    // eject writes the line out as its notes, and plays the same
    let ej = refs::eject(src, &e, s.span.start).unwrap().unwrap();
    assert!(ej.contains("# from: gen@1 dubstep seed=54876 section=drop part=bass -> z") && ej.contains("let walk_1 = ") && !ej.contains("use gen@1"), "{ej}");
    assert_eq!(nets(&ok(&ej, &e)), nets(&c));
    // provenance: the call
    assert!(refs::based_on(&c.text, &c.notes).iter().any(|b| b.call == "gen@1 dubstep seed=54876 section=drop part=bass" && b.how == "called"));
    // the design's example: a long nested reference read in lines is the reference written inline
    let lines = "use gen@1 dubstep seed=54876 section=drop part=bass as walk\nuse template(passacaglia, ground=walk) as episode_form\narrange from template(rondo, @shanty_drunken_sailor, episodes=episode_form)\n";
    let inline = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876 section=drop part=bass))\n";
    assert_eq!(nets(&ok(lines, &e)), nets(&ok(inline, &e)));
    // a seed's section is a section
    let sec = ok("tempo 124\nuse gen@2 techno seed=42 section=drop as drop\narrange drop drop\n", &e);
    assert_eq!(sec.segments.iter().filter(|s| !s.chain).map(|s| s.name.as_str()).collect::<Vec<_>>(), ["drop", "drop"]);
}

/// The two example programs check clean here (the second with the saved program it names as a snippet), are canonical, and the
/// written-out text plays as the program.
#[test]
fn the_examples_that_name_sources() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus = std::fs::read_to_string(root.join("crates/beatscript-api/tests/corpus/hand-dubstep-140-v2.beatscript")).unwrap();
    let source = djlive::compile::check_with(&corpus, &env()).compiled.unwrap();
    assert_eq!(source.cid, "z4EBG9j9cjqaYU5uVNFH6YKAMoiyPfNkof87KEXjPNPpAhJzDhi");
    let mut s = StaticSnippets::default();
    s.items.push(("hand-dubstep-140 v2".into(), source.cid.clone(), source.text.clone()));
    let e = Env { snippets: Some(&s), ..env() };
    for name in ["named-sources", "borrowed-by-name"] {
        let src = std::fs::read_to_string(root.join(format!("examples/beatscript/{name}.beatscript"))).unwrap();
        let c = djlive::compile::check_with(&src, &e);
        let errs: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
        let c = c.compiled.unwrap_or_else(|| panic!("{name}: {errs:?}"));
        let again = djlive::compile::check_with(&c.text, &e).compiled.unwrap();
        assert_eq!((again.text.clone(), again.cid.clone()), (c.text.clone(), c.cid.clone()), "{name}: the canonical text is a fixed point");
        let (written, _) = refs::expanded(&src, &e).unwrap().unwrap();
        assert_eq!(nets(&ok(&written, &e)), nets(&c), "{name}: written out, it plays the same");
        let ims = djlive::uses::imports(&src, &e).unwrap();
        assert!(ims.len() >= 2, "{name}: {ims:?}");
    }
}

/// check and expand answer the names: `imports`, and refs of kind `use`.
#[test]
fn check_and_expand_answer_the_imports() {
    env();
    let src = "key am\nuse theme ode_to_joy as tune\nuse template(passacaglia).ground as ground\nsection verse 4 bars {\n  deck B: melody tune inst=piano\n}\narrange ground verse\n";
    let rep = djengine::json::JsonValue::parse(&beatscript_api::check_json(src)).unwrap();
    let txt = rep.js_stringified();
    assert!(txt.contains("\"imports\":[") && txt.contains("\"kind\":\"use\"") && txt.contains("\"from\":\"template(passacaglia).ground\""), "{txt}");
    let ex = beatscript_api::expand_json(src, false);
    assert!(ex.contains("\"imports\":[") && ex.contains("let tune_1 = e4") && !ex.contains("use theme"), "{ex}");
    // a program with no `use` answers as it did: no imports
    assert!(!beatscript_api::check_json("tempo 120\ndeck A: kick x ~ x ~\n").contains("\"imports\""));
}
