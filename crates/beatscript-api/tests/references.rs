//! What a recipe makes (beatscript_api::resolution, project-datum 6668, 6680): the check report's `refs`, `expanded_cid` and
//! `resolution`, and the seeded generator's calls and parts as generate answers them.
//!
//! - `refs`: each reference with its kind, half, call, place, name, line and col, resolved CID, generator, fitted and track;
//!   `expansion` only when asked;
//! - `expanded_cid`: the CID of the written-out program, which is `expand`'s CID and plays what the recipe plays; the same
//!   recipe always gives it; a program with no references has its own CID;
//! - `resolution`: each distinct reference once, pinned, with its generator, and a gen call's key and tempo;
//! - generate's `gen` call is the program; a part's call writes the let generate gives for it.

use djengine::json::JsonValue;

fn env() -> djlive::env::Env<'static> {
    beatscript_api::web_env().env()
}

fn report(text: &str, expand: bool) -> JsonValue {
    JsonValue::parse(&beatscript_api::check_json_with(text, expand)).unwrap()
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k).and_then(JsonValue::as_str).unwrap_or_else(|| panic!("no {k} in {}", v.js_stringified()))
}

fn arr<'a>(v: &'a JsonValue, k: &str) -> &'a Vec<JsonValue> {
    v.get(k).and_then(JsonValue::as_array).unwrap_or_else(|| panic!("no {k} in {}", v.js_stringified()))
}

/// Each segment's name, place, length and the nets its decks play: what is heard.
fn nets(text: &str) -> Vec<(String, u32, u32, Vec<String>)> {
    let c = djlive::compile::check_with(text, &env()).compiled.unwrap();
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect())).collect()
}

const HOST: &str = "tempo 124\nkey am\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection drop 4 bars {\n  deck A: kick x ~ x ~ | bass bed\n  deck B: arp gen@2 house seed=7 section=drop\n}\nsection hook from gen@1 techno seed=3 section=drop\narrange drop hook\n";

#[test]
fn a_program_with_no_references_is_its_own_expansion() {
    for text in ["tempo 120\ndeck A: kick x ~ x ~\n", "tempo 126\nsection one 2 bars {\n  deck A: kick x x x x\n}\narrange one\n"] {
        let r = report(text, false);
        assert_eq!(s(&r, "expanded_cid"), s(&r, "cid"), "{text}");
        assert!(r.get("refs").is_none() && r.get("resolution").is_none(), "{}", r.js_stringified());
    }
    // a program that does not check has no CID and no expansion
    let bad = report("deck A: kick x ~ q ~\n", false);
    assert!(bad.get("expanded_cid").is_none());
}

#[test]
fn refs_say_what_each_reference_is_where_it_stands_and_what_it_resolved_to() {
    let r = report(HOST, false);
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", r.js_stringified());
    let refs = arr(&r, "refs");
    let by = |place: &str| refs.iter().find(|x| s(x, "place") == place).unwrap_or_else(|| panic!("no {place} ref in {refs:?}"));
    let l = by("let");
    assert_eq!((s(l, "kind"), s(l, "half"), s(l, "call"), s(l, "name")), ("gen", "engine", "gen@2 techno seed=42 section=drop part=bass", "bed"));
    assert_eq!(l.get("line").and_then(JsonValue::as_i64), Some(3));
    assert_eq!(s(l, "generator"), "seeded@2");
    assert!(s(l, "resolved").starts_with('z'));
    assert_eq!(l.get("fitted").unwrap().js_stringified(), r#"["key","bpm"]"#);
    let p = by("part");
    assert_eq!((s(p, "call"), s(p, "name")), ("gen@2 house seed=7 section=drop", "arp"));
    let sec = by("section");
    assert_eq!((s(sec, "call"), s(sec, "name"), s(sec, "generator")), ("gen@1 techno seed=3 section=drop", "hook", "seeded@1"));
    assert!(refs.iter().all(|x| x.get("expansion").is_none()), "no expansion unless asked");
    // the same call resolves to the same CID wherever it stands, and is the call's own program's
    let whole = report("arrange from gen@2 techno seed=42 key=am bpm=124\n", false);
    assert_eq!(s(&arr(&whole, "refs")[0], "resolved"), s(l, "resolved"), "a fitted call is the call that says the key and tempo");
    // asked, each one says what it is written out as
    let x = report(HOST, true);
    let lx = arr(&x, "refs").iter().find(|x| s(x, "place") == "let").unwrap().clone();
    assert!(s(&lx, "expansion").starts_with("let bed_1 = "), "{}", lx.js_stringified());
    // and asking changes nothing else
    assert_eq!(s(&x, "expanded_cid"), s(&r, "expanded_cid"));
}

#[test]
fn expanded_cid_is_the_written_out_program_and_plays_what_the_recipe_plays() {
    let r = report(HOST, false);
    let e = JsonValue::parse(&beatscript_api::expand_json(HOST, false)).unwrap();
    assert_eq!(s(&r, "expanded_cid"), s(&e, "cid"), "check's expanded_cid is expand's cid");
    assert_ne!(s(&r, "expanded_cid"), s(&r, "cid"));
    assert_eq!(nets(HOST), nets(s(&e, "text")), "the recipe and its expansion play the same nets");
    // the same recipe, the same result, every time; spacing and comments change neither CID
    let again = report(&format!("# a comment\n{}", HOST.replace("seed=42 section", "seed=42  section")), false);
    assert_eq!((s(&again, "cid"), s(&again, "expanded_cid")), (s(&r, "cid"), s(&r, "expanded_cid")));
}

#[test]
fn resolution_is_each_reference_once_with_its_pin_and_its_generator() {
    let text = "tempo 124\nkey am\nsection one 4 bars {\n  deck B: bass gen@2 techno seed=42 section=drop\n}\nsection two 4 bars {\n  deck B: bass gen@2 techno seed=42 section=drop\n}\narrange one two\n";
    let r = report(text, false);
    assert_eq!(arr(&r, "refs").len(), 2, "two places");
    let res = arr(&r, "resolution");
    assert_eq!(res.len(), 1, "one call: {res:?}");
    let g = &res[0];
    assert_eq!((s(g, "kind"), s(g, "generator"), s(g, "key")), ("gen", "seeded@2", "am"));
    assert_eq!(g.get("bpm").and_then(JsonValue::as_f64), Some(124.0));
    assert_eq!(s(g, "cid"), s(&arr(&r, "refs")[0], "resolved"));
    assert_eq!(g.get("nested").and_then(JsonValue::as_bool), Some(false));
    // a seed(...) is the beats generator's seeds, with its model
    let sd = report("arrange from seed(techno, 42)\n", false);
    let e = &arr(&sd, "resolution")[0];
    assert_eq!((s(e, "kind"), s(e, "generator")), ("seed", "seeds@v1"));
    assert!(s(e, "cid").starts_with('z') && !s(e, "model").is_empty());
}

#[test]
fn generate_s_call_is_its_program_and_a_part_s_call_writes_its_let() {
    for (v, genre, seed) in [(2, "techno", "42"), (1, "techno", "42"), (2, "trap", "54877"), (2, "house", "sunrise")] {
        let req = beatscript_api::GenRequest { genre: genre.into(), seed: seed.into(), ..Default::default() };
        let g = beatscript_api::generate::generate_version(v, &req).unwrap();
        let cid = s(&report(&g.text, false), "cid").to_string();
        let call = beatscript_api::generate::gen_call(v, &req, &g, &cid).unwrap_or_else(|| panic!("no call for {genre} {seed} v{v}"));
        assert_eq!(call, format!("gen@{v} {genre} seed={seed}"));
        let parts = beatscript_api::generate::parts_of(&g.text, &call, v == 2, None, None).unwrap();
        assert!(parts.len() >= 4, "{parts:?}");
        for p in &parts {
            assert_eq!(p.gen, format!("{call} section={} part={}", p.section, p.role));
            assert!(p.sections.contains(&p.section));
            if v == 2 && p.let_error.is_none() {
                // the let is what the call writes where a let stands
                let with_call = report(&format!("let {} = {}\ndeck B: {} {}\n", p.name, p.gen, p.role, p.name), false);
                let with_let = report(&format!("{}\ndeck B: {} {}\n", p.let_text.as_deref().unwrap(), p.role, p.name), false);
                assert_eq!(s(&with_call, "expanded_cid"), s(&with_let, "cid"), "{}", p.gen);
            }
        }
    }
    // the request's own key and tempo are in the call; what it left to the genre is left out
    let req = beatscript_api::GenRequest { genre: "techno".into(), seed: "9".into(), bpm: Some(128.0), key: "f#m".into(), length: "short".into() };
    let g = beatscript_api::generate::generate(&req).unwrap();
    let cid = s(&report(&g.text, false), "cid").to_string();
    assert_eq!(beatscript_api::generate::gen_call(3, &req, &g, &cid).as_deref(), Some("gen@3 techno seed=9 bpm=128 key=f#m length=short"));
    let g2 = beatscript_api::generate::generate_version(2, &req).unwrap();
    let cid2 = s(&report(&g2.text, false), "cid").to_string();
    assert_eq!(beatscript_api::generate::gen_call(2, &req, &g2, &cid2).as_deref(), Some("gen@2 techno seed=9 bpm=128 key=f#m length=short"));
    // a seed that is not one word cannot be a call
    let odd = beatscript_api::GenRequest { genre: "techno".into(), seed: "two words".into(), ..Default::default() };
    let g = beatscript_api::generate::generate(&odd).unwrap();
    let cid = s(&report(&g.text, false), "cid").to_string();
    assert_eq!(beatscript_api::generate::gen_call(2, &odd, &g, &cid), None);
}

#[test]
fn one_part_and_its_section_are_asked_for_by_name() {
    let req = beatscript_api::GenRequest { genre: "techno".into(), seed: "42".into(), ..Default::default() };
    let g = beatscript_api::generate::generate(&req).unwrap();
    let call = "gen@2 techno seed=42";
    let bass = beatscript_api::generate::parts_of(&g.text, call, true, Some("bass"), None).unwrap();
    assert_eq!(bass.len(), 1);
    assert_eq!((bass[0].section.as_str(), bass[0].name.as_str()), ("drop", "drop_bass"));
    assert!(bass[0].let_text.as_deref().is_some_and(|l| l.contains("let drop_bass = <")), "{bass:?}");
    let other = bass[0].sections.iter().find(|x| *x != "drop").cloned().unwrap();
    let there = beatscript_api::generate::parts_of(&g.text, call, false, Some("bass"), Some(&other)).unwrap();
    assert_eq!(there[0].section, other);
    assert!(beatscript_api::generate::parts_of(&g.text, call, false, Some("bass"), Some("nowhere")).is_err());
}
