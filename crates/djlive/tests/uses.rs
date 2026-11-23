//! `use SOURCE as NAME` (uses.rs, docs/beatscript/ext-roles.md "Name a source"), with no generator and no store: the sources that are
//! in the repo (a theme, a phrasebook line, a canned section of a template, a genre template's section, a form for a role).
//! (A stored program's and a seed's: crates/beatscript-api/tests/uses.rs.)
//! - the canonical text keeps the line as written, one way, and the name where it is used;
//! - the line is written out as the definition its source makes, and plays as that definition typed by hand;
//! - in a role the name is its source, in a phrase's place its `@id`;
//! - what is wrong is said at the line or at the use: an unknown source, a name bound twice, a name used before its `use`, the
//!   wrong kind for its place;
//! - refs lists the line as a reference of kind `use`, eject writes it out, and the program says what it is based on.

use djlive::compile::{check_with, Check, Compiled};
use djlive::env::{Env, StaticVoices};
use djlive::refs;
use djlive::uses::{self, Makes};

fn env_with<T>(f: impl FnOnce(&Env) -> T) -> T {
    let v = StaticVoices::default();
    f(&Env { voices: Some(&v), ..Env::default() })
}

fn check(src: &str) -> Check {
    env_with(|e| check_with(src, e))
}

fn complaints(c: &Check) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.severity != djlive::Severity::Info).map(|d| d.message.clone()).collect()
}

fn clean(src: &str) -> Compiled {
    let c = check(src);
    let bad = complaints(&c);
    assert!(c.compiled.is_some() && !c.diagnostics.iter().any(|d| d.is_error()), "{src}\n{bad:#?}");
    c.compiled.unwrap()
}

fn errors(src: &str) -> Vec<(String, String)> {
    check(src).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
}

fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>())).collect()
}

fn expanded(src: &str) -> String {
    env_with(|e| match djlive::stored::expand_text(src, e) {
        Some(Ok((t, _, _))) => t,
        other => panic!("{src}: {other:?}"),
    })
}

const THEME: &str = "tempo 110\nkey am\nuse theme ode_to_joy as tune\nsection verse 4 bars {\n  deck B: melody tune inst=piano\n}\narrange {\n  verse\n  ~ tune_* rot 2\n  verse\n}\n";

/// A theme is a line: its bars as lets named for the name, moved into the program's key (Ode to Joy, D major, in A minor goes
/// to its relative C major, -2), the parts and the mutations reading them by name; the program plays as those lets typed.
#[test]
fn a_theme_is_a_line_in_the_programs_key() {
    let c = clean(THEME);
    assert!(c.text.contains("use theme ode_to_joy as tune\n"), "{}", c.text);
    let text = expanded(THEME);
    assert!(text.contains("let tune_1 = e4 ~ ~ ~ e4 ~ ~ ~ f4 ~ ~ ~ g4 ~ ~ ~\n"), "{text}");
    assert!(text.contains("let tune = <tune_1 tune_2 tune_3 tune_4>") && !text.contains("use "), "{text}");
    // typed by hand, it is the same program
    assert_eq!(nets(&clean(&text)), nets(&c));
    // with no key of its own the program takes the theme as the library has it
    let plain = expanded("use theme ode_to_joy as tune\ndeck B: melody tune\n");
    assert!(plain.contains("let tune_1 = f#4 ~ ~ ~ f#4"), "{plain}");
    // check says what the name is, at the line
    let note = check(THEME).diagnostics.into_iter().find(|d| d.severity == djlive::Severity::Info && d.message.starts_with("`tune` is a line")).expect("a note");
    assert!(note.message.contains("moved -2 semitones from d into am") && note.message.contains("play it as `melody tune`"), "{}", note.message);
}

/// The canonical text writes the line one way: spacing is no part of it, and `genre` is `genre@1`.
#[test]
fn the_line_is_spelt_one_way() {
    let a = clean(THEME);
    let b = clean(&THEME.replace("use theme ode_to_joy as tune", "use   theme  ode_to_joy   as  tune"));
    assert_eq!((a.text.clone(), a.cid.clone()), (b.text, b.cid));
    // a program with a `use` is a program of its own: not the one with the lets typed
    assert_ne!(a.cid, clean(&expanded(THEME)).cid);
    // the same text, the same written-out program, every time
    assert_eq!(expanded(THEME), expanded(THEME));
    // `use restricted` is what it was
    assert!(djlive::parse::parse("use restricted\ndeck A: kick x\n").ok());
    assert_eq!(djlive::print::canonical(&djlive::parse::parse("use restricted\ndeck A: kick x\n").program), "use restricted\ndeck A: kick x\n");
}

/// A phrasebook line is a phrase: `sing NAME on ROLE` sings it, and the canonical text keeps the name.
#[test]
fn a_phrase_is_sung_by_its_name() {
    let src = "tempo 128\nuse phrase @tyger_burning_bright as tyger\nsection chant 4 bars {\n  deck B: melody c4 e4 g4 e4\n  deck B: sing tyger on melody\n}\narrange chant\n";
    let c = clean(src);
    assert!(c.text.contains("sing tyger on melody"), "{}", c.text);
    let by_id = src.replace("use phrase @tyger_burning_bright as tyger\n", "").replace("sing tyger", "sing @tyger_burning_bright");
    assert_eq!(nets(&c), nets(&clean(&by_id)));
    assert!(expanded(src).contains("deck B: sing @tyger_burning_bright on melody"));
    // a template's phrase too
    let t = "use phrase @tempest_such_stuff as line\narrange from template(deep-house, line)\n";
    assert_eq!(nets(&clean(t)), nets(&clean("arrange from template(deep-house, @tempest_such_stuff)\n")));
    assert!(clean(t).text.contains("template(deep-house, line)"));
    // a name no `use phrase` line gave is no phrase
    assert!(errors("deck B: melody c4\ndeck B: sing tyger on melody\n")[0].1.starts_with("no `use phrase` line names `tyger`"));
    assert!(errors("arrange from template(deep-house, line)\n")[0].1.starts_with("no `use phrase` line names `line`"));
}

/// A canned section: a section of a template's written-out program (the form's own example phrase when it names none), placed
/// by `arrange` like a section of the program's own; it plays as that section borrowed by its CID.
#[test]
fn a_canned_section_is_a_section_of_a_template() {
    let src = "use template(passacaglia).ground as ground\nuse template(deep-house, @tempest_such_stuff).night as night\narrange ground night ground\n";
    let c = clean(src);
    assert!(c.text.starts_with("use template(passacaglia).ground as ground\nuse template(deep-house, @tempest_such_stuff).night as night\n"), "{}", c.text);
    assert_eq!(c.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["ground", "night", "ground"]);
    let text = expanded(src);
    assert!(text.contains("section ground 8 bars {") && text.contains("section night "), "{text}");
    assert_eq!(nets(&clean(&text)), nets(&c));
    let ims = env_with(|e| uses::imports(src, e)).unwrap();
    assert_eq!(ims.len(), 2);
    assert_eq!((ims[0].name.as_str(), ims[0].makes, ims[0].source.kind()), ("ground", Makes::Section, "template"));
    assert!(ims[0].what.contains("the ground of template(passacaglia, @eccl3_season)"), "{}", ims[0].what);
    assert!(ims[0].resolved.as_deref().is_some_and(djlive::stored::is_cid));
    // a section it has not got, with the nearest
    let e = errors("use template(passacaglia).grund as grd\narrange grd\n");
    assert!(e[0].1.contains("the passacaglia template has no section `grund`") && e[0].1.contains("did you mean `ground`?"), "{e:?}");
    // a template with a phrase is a whole track: `use` takes a section of it
    assert!(errors("use template(deep-house, @tempest_such_stuff) as dh\n")[0].1.contains("makes a whole track"));
}

/// A form is a source for a role: the name in `episodes=` is the form, nested as if written there; a theme's name in a role is
/// the theme, which the role moves into its own key, and its lets are not written where no part plays them.
#[test]
fn in_a_role_the_name_is_its_source() {
    let inline = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=ode_to_joy))\n";
    let named = "use theme ode_to_joy as tune\nuse template(passacaglia, ground=tune) as episode_form\narrange from template(rondo, @shanty_drunken_sailor, episodes=episode_form)\n";
    assert_eq!(nets(&clean(named)), nets(&clean(inline)));
    let text = expanded(named);
    assert!(!text.contains("let tune_1") && !text.contains("use "), "{text}");
    let canon = "arrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy)\n";
    assert_eq!(nets(&clean("use theme ode_to_joy as tune\narrange from template(canon, @mgm_pease_porridge_hot, subject=tune)\n")), nets(&clean(canon)));
}

/// What is wrong is said where it is, with what would work.
#[test]
fn refusals_say_what_would_work() {
    let first = |src: &str| errors(src).into_iter().next().unwrap_or_else(|| panic!("no error: {src}"));
    // an unknown source; a theme without its word
    assert!(first("use foo as tnx\n").1.starts_with("`foo` is no source for `use`"));
    assert!(first("use ode_to_joy as tnx\n").1.contains("did you mean `use theme ode_to_joy as NAME`?"));
    assert!(first("use thme ode_to_joy as tnx\n").1.contains("did you mean `theme`?"));
    assert!(first("use theme ode_to_jo as tnx\ndeck B: melody tnx\n").1.contains("did you mean `ode_to_joy`?"));
    assert!(first("use phrase tyger as tnx\n").1.starts_with("a phrase is `@` and its id"));
    assert!(first("use [d2 ~ a1 ~] as riff\n").1.contains("let riff = d2 ~ a1 ~"));
    assert!(first("use seed(techno, 42) as s\n").1.contains("use gen@2 techno seed=42 section=drop as drop"));
    assert!(first("use genre@7 techno.drop as dr\narrange dr\n").1.starts_with("this BeatScript has genre@1, not `genre@7`"));
    assert!(first("use theme ode_to_joy\n").1.contains("use theme ode_to_joy as NAME"));
    // a name bound twice, or bound already
    let e = first("use theme ode_to_joy as tune\nuse theme badinerie as tune\ndeck B: melody tune\n");
    assert_eq!(e.0, "tune");
    assert!(e.1.starts_with("`tune` is named already, by the `use` on line 1"), "{e:?}");
    assert!(first("let tune = c4 e4\nuse theme ode_to_joy as tune\ndeck B: melody tune\n").1.starts_with("`tune` is bound already: a let `tune` on line 1"));
    assert!(first("let tune_2 = c4\nuse theme ode_to_joy as tune\ndeck B: melody tune\n").1.starts_with("`tune` is bound already: a let `tune_2`"));
    // used before its `use`
    let e = first("deck B: melody tune\nuse theme ode_to_joy as tune\n");
    assert!(e.1.starts_with("`tune` is used before its `use` (line 2)") && e.1.contains("above line 1"), "{e:?}");
    // the wrong kind for the place
    assert!(first("use template(passacaglia).ground as grd\ndeck A: bass grd\n").1.starts_with("`grd` is a section, and a part plays a line"));
    assert!(first("use theme ode_to_joy as tune\narrange tune\n").1.starts_with("`tune` is a line (a let), and this place takes a section"));
    assert!(first("use phrase @tyger_burning_bright as tg\narrange from template(canon, @hands_up, subject=tg)\n").1.starts_with("`tg` is a phrase, and a role takes a source"));
    assert!(first("use theme ode_to_joy as tune\ndeck B: melody c4\ndeck B: sing tune on melody\n").1.starts_with("no `use phrase` line names `tune`"));
    assert!(first("use template(passacaglia) as frm\ndeck A: bass frm\n").1.starts_with("`frm` is a source for a role"));
    // a `use` is at the top
    assert!(first("section s 4 bars {\n  use theme ode_to_joy as tune\n  deck B: melody tune\n}\narrange s\n").1.contains("a `use` goes at the top"));
    // a name never used is said, not refused
    let c = check("use theme ode_to_joy as tune\ndeck A: kick x\n");
    assert!(c.compiled.is_some() && c.diagnostics.iter().any(|d| d.message == "`tune` is never used"));
}

/// refs lists the line (kind `use`, the name, what it is, its pin); eject writes it out under its `# from:` line, and the ejected
/// program plays the same with no `use` in it; the program's provenance says the theme library.
#[test]
fn eject_writes_a_use_out_and_provenance_says_it() {
    let ss = env_with(|e| refs::sites(THEME, e)).unwrap();
    assert_eq!(ss.len(), 1);
    let s = &ss[0];
    assert_eq!((s.kind, s.half, s.call.as_str(), s.place, s.name.as_deref()), ("use", refs::STRUCTURE, "theme ode_to_joy", "let", Some("tune")));
    let lib = djlive::compile::program_cid(djengine::themes::themes_text());
    assert_eq!(s.resolved.as_deref(), Some(lib.as_str()));
    let ej = env_with(|e| refs::eject(THEME, e, s.span.start)).unwrap().unwrap();
    assert!(ej.contains(&format!("# from: theme ode_to_joy -> {lib}\nlet tune_1 = e4")) && !ej.contains("use theme"), "{ej}");
    assert_eq!(nets(&clean(&ej)), nets(&clean(THEME)));
    assert!(env_with(|e| refs::sites(&ej, e)).unwrap().is_empty());
    // a name only a role reads: its line goes, and the role says the source
    let named = "use template(passacaglia, ground=ode_to_joy) as ep\narrange from template(rondo, @shanty_drunken_sailor, episodes=ep)\n";
    let ss = env_with(|e| refs::sites(named, e)).unwrap();
    assert_eq!(ss.iter().map(|s| s.kind).collect::<Vec<_>>(), ["use", "template"]);
    let ej = env_with(|e| refs::eject(named, e, ss[0].span.start)).unwrap().unwrap();
    assert_eq!(ej, "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=ode_to_joy))\n");
    // provenance: the library a theme comes from, as a borrow
    let c = clean(THEME);
    let b = refs::based_on(&c.text, &c.notes);
    assert!(b.iter().any(|x| x.call == "theme ode_to_joy" && x.cid.as_deref() == Some(lib.as_str()) && x.how == "borrowed"), "{b:?}");
}

/// A genre template's section: the genre's starting program as `genre_template` wrote it, frozen (genre templates v1), its
/// section borrowed by its CID; a bare `genre` is `genre@1`. Every section of every genre template can be named.
#[test]
fn a_genre_templates_section() {
    let src = "use genre techno.drop as drop\nuse genre@1 techno.intro as intro\narrange intro drop\n";
    let c = clean(src);
    assert!(c.text.starts_with("use genre@1 techno.drop as drop\nuse genre@1 techno.intro as intro\n"), "{}", c.text);
    assert_eq!(nets(&clean(&expanded(src))), nets(&c));
    let ims = env_with(|e| uses::imports(src, e)).unwrap();
    assert!(ims[0].resolved.as_deref().is_some_and(djlive::stored::is_cid) && ims[0].what.contains("the drop of the techno template (genre@1)"), "{:?}", ims[0]);
    assert!(errors("use genre@1 tecno.drop as dr\narrange dr\n")[0].1.contains("did you mean `techno`?"));
    assert!(errors("use genre@1 techno.drp as dr\narrange dr\n")[0].1.contains("did you mean `drop`?"));
    let all = djengine::genre_templates::templates(1);
    assert!(all.len() >= 5, "{} genre templates", all.len());
    for t in all {
        for s in uses::genre_sections(1, &t.genre).unwrap() {
            clean(&format!("use genre@1 {}.{s} as sec\narrange sec\n", t.genre));
        }
    }
}
