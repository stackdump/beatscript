//! Roles of the form templates (roles.rs, docs/beatscript/ext-roles.md), with no generator: themes, notes and forms in a role.
//! (A seed in a role needs the seeded generator: crates/beatscript-api/tests/forms_roles.rs.)
//! - a role nobody binds is the form's own: an unbound reference is the text it always was (templates_v1_are_pinned);
//! - a theme in a role is its bars moved into the key the role plays in, read by the sections and the plan by its new name;
//! - notes are a bar each; a nested form is its sections, its names given the role's prefix, its plan over the role's places;
//! - the reference is spelt one way, and what is wrong is said at the binding.

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

/// A theme in a role: its bars, moved into the form's key (to the relative key when its mode is the other one), named for the
/// role, and the plan's mutations reach them. Ode to Joy (D major) in the passacaglia (G minor) goes to B-flat major, -4.
#[test]
fn a_theme_fills_a_role_in_the_forms_key() {
    let src = "arrange from template(passacaglia, @eccl3_season, upper=ode_to_joy)\n";
    let c = clean(src);
    let text = expanded(src);
    assert!(text.contains("# upper, bound: ode_to_joy\nlet upper_1 = d4 ~ ~ ~ d4 ~ ~ ~ d#4 ~ ~ ~ f4 ~ ~ ~\n"), "{text}");
    assert!(text.contains("let upper = <upper_1 upper_2 upper_3 upper_4>") && !text.contains("let upper_1 = g4"), "{text}");
    assert!(text.contains("~ upper_* rot 2 | accent 0.25"));
    // the header says what fills each role
    assert!(text.contains("# upper: ode_to_joy; the line above the ground") && text.contains("# ground: Holst's five-beat ostinato from Mars, mars_ostinato; the bass line"), "{text}");
    // the written-out program plays as the reference
    assert_eq!(nets(&clean(&text)), nets(&c));
    // in a form in the theme's own key it is not moved; the program's own key moves it
    assert!(expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy)\n").contains("let subject_1 = f#4 ~ ~ ~ f#4"));
    assert!(expanded("key e\narrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy)\n").contains("let subject_1 = g#4 ~ ~ ~ g#4"));
    // a 5/4 theme keeps its cycle where only parts play the role
    let t = expanded("arrange from template(canon, @mgm_pease_porridge_hot, ground=mars_ostinato)\n");
    assert!(t.contains("bass/20 ground inst=bass"), "{t}");
    clean("arrange from template(canon, @mgm_pease_porridge_hot, ground=mars_ostinato)\n");
}

/// Notes in a role: a bar each, a let a bar; the parts that played the default play them, with their own cycle.
#[test]
fn notes_fill_a_role_a_bar_each() {
    let src = "arrange from template(passacaglia, @eccl3_season, ground=<[g1 ~ ~ ~ d2 ~ ~ ~] [bb1 ~ ~ ~ c2 ~ ~ ~]>)\n";
    clean(src);
    let text = expanded(src);
    assert!(text.contains("let ostinato_1 = g1 ~ ~ ~ d2 ~ ~ ~\nlet ostinato_2 = a#1 ~ ~ ~ c2 ~ ~ ~\nlet ostinato = <ostinato_1 ostinato_2>"), "{text}");
    assert!(text.contains("| bass ostinato inst=bass") && !text.contains("mars_ostinato") && !text.contains("five beats against"), "{text}");
    // one bar
    let one = expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4])\n");
    assert!(one.contains("let subject_1 = d5 c#5 b4 a4\nlet subject = <subject_1>"), "{one}");
    // the canon's copies are made of the subject's bars: half a bar behind for a one-bar subject, the last bar first for more
    assert!(one.contains("let follow = rotate(subject, 8)") && one.contains("# chase: the copy half a bar behind"), "{one}");
    let three = expanded("arrange from template(canon, @mgm_pease_porridge_hot, subject=<[d5 c#5] [b4 a4] [b4 c#5]>)\n");
    assert!(three.contains("let follow = <subject_3 subject_1 subject_2>"), "{three}");
    clean("arrange from template(canon, @mgm_pease_porridge_hot, subject=<[d5 c#5] [b4 a4] [b4 c#5]>)\n");
}

/// A form inside another: its sections and lets under the role's prefix (`ep_`), in its own key when that is not the
/// program's, its vocal left out (the phrase is the outer form's), and its plan spread over the places the role plays in.
#[test]
fn a_form_in_a_role_is_its_sections_and_its_plan() {
    let src = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, upper=ode_to_joy))\n";
    let c = clean(src);
    let text = expanded(src);
    for want in ["# episodes, bound: template(passacaglia, upper=ode_to_joy)", "section ep_ground 8 bars {\n  key gm\n", "section ep_variation 8 bars {\n  key gm\n", "let ep_upper_1 = d4 ~ ~ ~", "let ep_mars_ostinato = <ep_mars_ostinato_1>", "  ~ ep_upper_* rot 2 | accent 0.25"] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    assert_eq!(text.matches("sing \"").count(), 1, "one sung section, the rondo's");
    assert!(!text.contains("section episode ") && !text.contains("section ep_sung"), "{text}");
    let arrange: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("arrange {")).map(str::trim).collect();
    let first = arrange.iter().position(|l| *l == "ep_ground").unwrap();
    assert_eq!(arrange[first - 1], "refrain", "the first episode's place");
    assert_eq!(nets(&clean(&text)), nets(&c));
    // three deep is the most
    let deep = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(rondo, episodes=template(rondo, episodes=template(canon))))\n";
    let e = errors(deep);
    assert!(e.iter().any(|(at, m)| at == "template(canon)" && m.contains("nest 3 deep")), "{e:?}");
    let three = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(rondo, episodes=template(canon, subject=ode_to_joy)))\n";
    let t = expanded(three);
    assert!(t.contains("section ep_ep_lead 8 bars {\n  key d\n") && t.contains("let ep_ep_follow = <ep_ep_subject_4 ep_ep_subject_1 ep_ep_subject_2 ep_ep_subject_3>"), "{t}");
    clean(three);
}

/// One spelling: the roles in the form's order, `role=source` glued, sources canonical. An unbound reference is unchanged.
#[test]
fn a_reference_with_roles_is_spelt_one_way() {
    let a = clean("arrange from template(passacaglia, @eccl3_season, upper=ode_to_joy, ground=[ g1 ~ bb1 ~ ])\n");
    let b = clean("arrange from template( passacaglia,@eccl3_season,ground = [g1 ~ a#1 ~],upper= ode_to_joy )\n");
    assert_eq!(a.text, "arrange from template(passacaglia, @eccl3_season, ground=[g1 ~ a#1 ~], upper=ode_to_joy)\n");
    assert_eq!((a.cid.clone(), a.text.clone()), (b.cid, b.text));
    let nested = clean("arrange from template(rondo,@shanty_drunken_sailor,episodes=template( canon ,ground=pachelbel_ground))\n");
    assert_eq!(nested.text, "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(canon, ground=pachelbel_ground))\n");
    assert_eq!(clean("arrange from template(canon, @mgm_pease_porridge_hot)\n").text, "arrange from template(canon, @mgm_pease_porridge_hot)\n");
}

/// A role bound to a seed where no generator is installed: the call is the usual error, at it.
#[test]
fn a_seed_with_no_generator_is_said_at_it() {
    let src = "arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass)\n";
    let e = errors(src);
    assert!(e.iter().any(|(at, m)| at.starts_with("gen@1") && m.contains("is not here")), "{e:?}");
}

/// What is wrong with a binding is said at it, with what would fix it.
#[test]
fn check_names_what_is_wrong_at_the_binding() {
    let cases: [(&str, &str, &str); 9] = [
        ("arrange from template(passacaglia, @eccl3_season, uper=ode_to_joy)\n", "uper", "`upper`"),
        ("arrange from template(passacaglia, @eccl3_season, upper=ode_to_joy, upper=badinerie)\n", "upper", "bound twice"),
        ("arrange from template(passacaglia, @eccl3_season, upper=ode_to_jy)\n", "ode_to_jy", "`ode_to_joy`"),
        ("arrange from template(deep-house, @tempest_such_stuff, upper=ode_to_joy)\n", "upper", "takes no roles yet"),
        ("arrange from template(canon, @mgm_pease_porridge_hot, subject=gymnopedie_melody)\n", "gymnopedie_melody", "bars of 16"),
        ("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(deep-house))\n", "deep-house", "not a form"),
        ("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(rondoo))\n", "rondoo", "`rondo`"),
        ("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, @eccl3_season))\n", "@eccl3_season", "takes no phrase"),
        ("arrange from template(passacaglia, @eccl3_season, ground=[g1 ~ x*99 ~])\n", "[g1", "a repeat count is 1 to 64"),
    ];
    for (src, at, msg) in cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a.starts_with(at) && m.contains(msg)), "{src}: {e:?}");
    }
    let bars = vec!["[g1 ~ d2 ~]"; 17].join(" ");
    let e = errors(&format!("arrange from template(passacaglia, @eccl3_season, ground=<{bars}>)\n"));
    assert!(e.iter().any(|(_, m)| m.contains("a line is at most 16")), "{e:?}");
    // a binding needs its phrase first
    assert!(!errors("arrange from template(passacaglia, upper=ode_to_joy)\n").is_empty());
}

/// A source bound to a role is the template's: `refs::sites` lists the template alone, and eject writes it out with the
/// sources in it (a theme is notes in the text, structure all the way down).
#[test]
fn a_bound_source_is_the_templates_reference() {
    let src = "arrange from template(passacaglia, @eccl3_season, upper=ode_to_joy, ground=[g1 ~ d2 ~])\n";
    let ss = djlive::refs::sites(src, &Env::none()).unwrap();
    assert_eq!(ss.len(), 1, "{ss:?}");
    assert!(ss[0].call.contains("upper=ode_to_joy"));
    let e = djlive::refs::eject(src, &Env::none(), ss[0].span.start).unwrap().unwrap();
    assert!(e.starts_with("# from: template(passacaglia, @eccl3_season, ground=[g1 ~ d2 ~], upper=ode_to_joy) -> z"), "{e}");
    assert!(djlive::refs::sites(&e, &Env::none()).unwrap().is_empty());
    assert_eq!(nets(&clean(&e)), nets(&clean(src)));
}

/// Every role of every form is said in the table the docs read (name, what it is, its default, the let it is written as),
/// and the default it names is in the form's text.
#[test]
fn every_role_is_in_its_forms_text() {
    for name in djlive::roles::forms_with_roles() {
        let t = template::find(name).unwrap();
        for r in djlive::roles::specs(name) {
            assert!(!r.what.is_empty() && !r.default.is_empty(), "{name}.{}", r.name);
            let ids: Vec<&str> = if r.parts.is_empty() { vec![r.id] } else { r.parts.iter().map(|p| p.1).collect() };
            for id in ids {
                let text = expanded(&format!("{}\n", t.example));
                assert!(text.contains(&format!("let {id} = <")), "{name}.{}: no `{id}` in the form's text", r.name);
            }
            if !r.above.is_empty() {
                assert!(t.text().contains(r.above), "{name}.{}: the comment above its default", r.name);
            }
        }
    }
}
