//! The nested parts of a template's expansion (nested.rs, roadmap 6680 item 2), with no generator: themes, notes and forms in
//! roles. (A seed in a role: crates/beatscript-api/tests/references.rs.)
//! - each role a reference binds is a part, at any depth, by path (`episodes/upper`), in the order the form names its roles;
//! - a part's text is a program of its own (the key and tempo it plays in, the lines its binding wrote), canonical, and its CID
//!   is that text's; what it wrote is what the whole expansion has;
//! - the same recipe gives the same parts, every time; the recipe's own CID is what it was;
//! - the program's JSON-LD carries them as `hasPart`, by CID, and the schema's context defines the term.

use djlive::compile::{check_with, program_cid};
use djlive::env::{Env, StaticVoices};
use djlive::nested::{self, Part};

fn parts(src: &str) -> Vec<Part> {
    let v = StaticVoices::default();
    nested::parts(src, &Env { voices: Some(&v), ..Env::default() })
}

fn expanded(src: &str) -> String {
    let v = StaticVoices::default();
    let env = Env { voices: Some(&v), ..Env::default() };
    djlive::refs::expanded(src, &env).unwrap().unwrap().0
}

const NESTED: &str = "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, upper=ode_to_joy, ground=[g1 ~ d2 ~]))\n";

#[test]
fn every_role_is_a_part_by_path_with_its_own_parts() {
    let ps = parts(NESTED);
    assert_eq!(ps.len(), 1, "{ps:#?}");
    let ep = &ps[0];
    assert_eq!((ep.path.as_str(), ep.role.as_str(), ep.kind), ("episodes", "episodes", "template"));
    assert_eq!(ep.call, "template(passacaglia, ground=[g1 ~ d2 ~], upper=ode_to_joy)");
    assert_eq!((ep.resolves.as_deref(), ep.generator.as_deref()), (None, None));
    // the inner form's roles, in the order the passacaglia names them (ground, then upper)
    let paths: Vec<(&str, &str)> = ep.parts.iter().map(|p| (p.path.as_str(), p.kind)).collect();
    assert_eq!(paths, [("episodes/ground", "notes"), ("episodes/upper", "theme")]);
    assert!(ep.parts.iter().all(|p| p.parts.is_empty()));
    assert_eq!(nested::flatten(&ps).len(), 3);
}

#[test]
fn a_parts_text_is_a_program_of_its_own_and_its_cid_is_that_texts() {
    let ps = parts(NESTED);
    let whole = expanded(NESTED);
    for p in nested::flatten(&ps) {
        assert_eq!(p.cid, program_cid(&p.text), "{}", p.path);
        // canonical: it prints as itself
        assert_eq!(djlive::print::canonical(&djlive::parse::parse(&p.text).program), p.text, "{}", p.path);
        // under the key and tempo of the program it was written into: the rondo's
        assert!(p.text.starts_with("tempo 116\nkey c\n"), "{}: {}", p.path, p.text);
        // and what its binding wrote is what the whole expansion has, line for line
        for l in p.text.lines().skip(2) {
            assert!(whole.contains(l), "{}: `{l}` is not in the expansion", p.path);
        }
    }
    let upper = &ps[0].parts[1];
    assert!(upper.text.contains("let ep_upper_1 = d4 ~ ~ ~"), "{}", upper.text);
    // the form in the role is its sections, in its own key
    assert!(ps[0].text.contains("section ep_ground 8 bars {\n  key gm\n"), "{}", ps[0].text);
    // each part is another program
    let cids: Vec<&str> = nested::flatten(&ps).iter().map(|p| p.cid.as_str()).collect();
    assert!(cids.iter().all(|c| c.starts_with('z')) && cids[0] != cids[1] && cids[1] != cids[2]);
}

#[test]
fn the_same_recipe_gives_the_same_parts_and_the_recipe_keeps_its_cid() {
    let a = parts(NESTED);
    assert_eq!(a, parts(NESTED));
    // spacing and the order the roles are written in are not part of the reference
    assert_eq!(a, parts("arrange from template( rondo,@shanty_drunken_sailor, episodes=template(passacaglia,ground=[g1 ~ d2 ~],upper=ode_to_joy ))\n"));
    // the parts are beside the CID: the recipe's is of its text as written, as it always was
    let v = StaticVoices::default();
    let c = check_with(NESTED, &Env { voices: Some(&v), ..Env::default() }).compiled.unwrap();
    assert_eq!(c.text, "arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=[g1 ~ d2 ~], upper=ode_to_joy))\n");
    // a pin: these parts are what this reference means in templates v1
    assert_eq!(a[0].parts[1].cid, "z4EBG9jGZ7xxriri7DZArC2QxePgw2rzyBPxPL5FQVb8vmRT7oA", "{}", a[0].parts[1].text);
}

#[test]
fn a_reference_with_no_roles_or_no_template_has_no_parts() {
    assert!(parts("arrange from template(passacaglia, @eccl3_season)\n").is_empty());
    assert!(parts("arrange from template(deep-house, @tempest_such_stuff)\n").is_empty());
    assert!(parts("tempo 120\ndeck A: kick x ~ x ~\n").is_empty());
    // one that does not expand (a role the form does not have) has none either: check says why
    assert!(parts("arrange from template(passacaglia, @eccl3_season, uper=ode_to_joy)\n").is_empty());
    // notes in a role at the top
    let n = parts("arrange from template(canon, @mgm_pease_porridge_hot, subject=[d5 c#5 b4 a4])\n");
    assert_eq!((n.len(), n[0].path.as_str(), n[0].kind), (1, "subject", "notes"));
    assert!(n[0].text.contains("let subject_1 = d5 c#5 b4 a4\nlet subject = <subject_1>"), "{}", n[0].text);
}

#[test]
fn the_json_ld_has_each_part_by_its_cid() {
    let v = StaticVoices::default();
    let env = Env { voices: Some(&v), ..Env::default() };
    let c = check_with(NESTED, &env).compiled.unwrap();
    let d = djlive::progld::document(&c, &djlive::progld::Facts { env: Some(&env), ..Default::default() });
    let ps = parts(NESTED);
    let hp = d.get("hasPart").and_then(|h| h.as_array()).expect("hasPart");
    let ep = &hp[0];
    let s = |v: &djengine::json::JsonValue, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    assert_eq!((s(ep, "@type"), s(ep, "@id"), s(ep, "name")), ("CreativeWork".to_string(), ps[0].cid.clone(), "episodes".to_string()));
    let src = &ep.get("isBasedOn").and_then(|b| b.as_array()).unwrap()[0];
    assert_eq!((s(src, "name"), s(src, "derivation")), (ps[0].call.clone(), "borrowed".to_string()));
    let inner = ep.get("hasPart").and_then(|h| h.as_array()).unwrap();
    assert_eq!(s(&inner[1], "@id"), ps[0].parts[1].cid);
    assert_eq!(s(&inner[1], "name"), "episodes/upper");
    // the recipe expands to its written-out program, as before
    assert_eq!(s(&d, "expandsTo"), program_cid(&expanded(NESTED)));
    // the context names the term (schema.org's hasPart)
    let ctx = djlive::progld::schema_context().js_stringified();
    assert!(ctx.contains(r#""hasPart":{"@id":"schema:hasPart","@container":"@set"}"#), "{ctx}");
    // a program with no template has no hasPart
    let plain = check_with("tempo 120\ndeck A: kick x ~ x ~\n", &env).compiled.unwrap();
    assert!(djlive::progld::document(&plain, &djlive::progld::Facts::default()).get("hasPart").is_none());
}

#[test]
fn a_stored_expansion_says_its_recipe_and_what_it_was_written_out_from() {
    let v = StaticVoices::default();
    let env = Env { voices: Some(&v), ..Env::default() };
    let x = check_with(&expanded(NESTED), &env).compiled.unwrap();
    let recipe = program_cid("arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=[g1 ~ d2 ~], upper=ode_to_joy))\n");
    // (the last line resolves to the expansion itself, as a template with nothing after it would: a program is never based on
    // itself, so it says its call alone)
    let based = vec![format!("template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=[g1 ~ d2 ~], upper=ode_to_joy)) -> {recipe}"), "ode_to_joy".to_string(), format!("template(canon, @mgm_pease_porridge_hot) -> {}", x.cid)];
    let d = djlive::progld::document(&x, &djlive::progld::Facts { expanded_from: Some(&recipe), based_on: &based, ..Default::default() }).js_stringified();
    assert!(d.contains(&format!(r#""expandedFrom":"{recipe}""#)), "{d}");
    assert!(d.contains(&format!(r#""resolves":"{recipe}","derivation":"expanded""#)), "{d}");
    assert!(d.contains(r#"{"@type":"CreativeWork","name":"ode_to_joy","derivation":"expanded"}"#), "{d}");
    assert!(d.contains(r#"{"@type":"CreativeWork","name":"template(canon, @mgm_pease_porridge_hot)","derivation":"expanded"}"#), "{d}");
    assert!(!d.contains(&format!(r#""resolves":"{}""#, x.cid)), "based on itself: {d}");
    assert!(!d.contains("expandsTo") && !d.contains("hasPart"), "an expansion has no references of its own: {d}");
}
