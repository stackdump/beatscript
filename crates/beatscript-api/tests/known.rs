//! The programs a web host knows by CID (known.rs): an `import` resolves from them, a link carries them, and a program
//! that is not here says so. They live in one process-wide table, so the whole story is one test: the cases do not race.

mod support;

use djengine::json::JsonValue;
use support::*;

const RIFFS: &str = "let riff = c2 e2 g2 b2\ndef hit(d) {\n  deck $d: kick x ~ x ~\n}\n";

fn cid_of(text: &str) -> String {
    let c = djlive::check(text).compiled.expect("it compiles");
    c.cid
}

fn first_message(r: &JsonValue) -> String {
    get(r, "diagnostics").as_array().unwrap().iter().find(|d| s(d, "severity") == "error").map(|d| s(d, "message").to_string()).unwrap_or_default()
}

#[test]
fn an_import_by_cid_resolves_from_the_programs_the_host_knows() {
    let canonical = djlive::check(RIFFS).compiled.unwrap().text;
    let cid = cid_of(RIFFS);
    let text = format!("import {cid}\ndeck A: bass riff\nhit(B)\n");
    beatscript_api::known::forget_all();
    // not here: said on the import's line, in the words every host uses, with what would fix it
    let r = parse(&beatscript_api::check_json(&text));
    assert!(!b(&r, "ok"));
    let m = first_message(&r);
    assert!(djlive::env::is_not_here(&m), "{m}");
    assert!(m.starts_with(&format!("`{}` is not here", djengine::rights::short_cid(&cid))), "{m}");
    assert_eq!(s(r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().find(|d| s(d, "severity") == "error").unwrap(), "help"), djlive::env::NOT_HERE_HELP);
    // a short CID is a CID too: not here, not "by name"
    assert!(djlive::env::is_not_here(&first_message(&parse(&beatscript_api::check_json("import zQmNothing\ndeck A: kick x\n")))));

    // told the program, it checks: the same CID, nets and music as where the program is saved
    assert!(beatscript_api::known::know(&cid, &canonical).is_ok());
    let r = parse(&beatscript_api::check_json(&text));
    assert!(b(&r, "ok"), "{}", r.js_stringified());
    assert_eq!(get(&r, "uses").as_array().unwrap().iter().filter_map(JsonValue::as_str).collect::<Vec<_>>(), [cid.as_str()]);
    let saved = {
        let mut st = djlive::env::StaticSnippets::default();
        st.items.push(("riffs".into(), cid.clone(), canonical.clone()));
        djlive::compile::check_with(&text, &djlive::env::Env { snippets: Some(&st), ..Default::default() }).compiled.unwrap()
    };
    assert_eq!(s(&r, "cid"), saved.cid);
    assert_eq!(parse(&beatscript_api::nets_json(&text, false)).get("ok").and_then(JsonValue::as_bool), Some(true));

    // a text that is not the program the CID names is refused, and nothing is kept
    let other = cid_of("deck A: kick x x ~ ~\n");
    assert!(beatscript_api::known::know(&other, &canonical).unwrap_err().contains("not the program it names"));
    assert!(!beatscript_api::known::knows(&other));
    assert!(beatscript_api::known::know("not a cid", "x").is_err());

    // its share link carries the program, and opens on a host that was told nothing
    let sh = parse(&beatscript_api::share_json(&text, None, false, ""));
    assert!(b(&sh, "ok"), "{}", sh.js_stringified());
    let url = s(&sh, "url").to_string();
    assert!(url.contains('*'), "the bundle rides the link");
    assert!(!s(&sh, "social_url").contains('*'), "the social form is the sealed part only");
    assert!(get(&sh, "warnings").as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|w| w.contains("carries the 1 program"))));
    beatscript_api::known::forget_all();
    assert!(!beatscript_api::known::knows(&cid));
    let o = parse(&beatscript_api::open_json(&url));
    assert!(b(&o, "ok") && b(&o, "verified"), "{}", o.js_stringified());
    assert_eq!(s(&o, "cid"), s(&r, "cid"));
    // the opened link hands the bundle back, so the host keeps it (and edits check again with nothing fetched)
    let bundle = get(&o, "bundle");
    assert_eq!(bundle.get(&cid).and_then(JsonValue::as_str), Some(canonical.as_str()));
    // ... and a host that does not keep it cannot check the text on its own
    assert!(!b(&parse(&beatscript_api::check_json(&text)), "ok"));
    for (c, t) in bundle.as_object().unwrap().iter() {
        beatscript_api::known::know(c, t.as_str().unwrap()).unwrap();
    }
    assert!(b(&parse(&beatscript_api::check_json(&text)), "ok"));

    // a program the link does not carry and the host was not told: refused in words, never played
    beatscript_api::known::forget_all();
    let frag = url.split_once('#').unwrap().1;
    let without = frag.split('*').next().unwrap();
    let o2 = parse(&beatscript_api::open_json(without));
    assert!(!b(&o2, "ok"));
    assert!(djlive::env::is_not_here(s(&o2, "error")), "{}", s(&o2, "error"));
    beatscript_api::known::forget_all();

    // a section borrowed from a stored program: not here until the host knows it, then it checks, `expand` writes it out, and
    // a link carries it
    let song = "tempo 140\nkey fm\nsection drop 4 bars {\n  deck A: kick x ~ x ~ | bass f1 ~ ~ f1 vel=90\n}\narrange drop\n";
    let song_cid = cid_of(song);
    let song_text = djlive::check(song).compiled.unwrap().text;
    let borrower = format!("tempo 140\nkey fm\nsection hook from {song_cid}.drop\narrange hook\n");
    let missing = parse(&beatscript_api::check_json(&borrower));
    assert!(!b(&missing, "ok") && djlive::env::is_not_here(&first_message(&missing)), "{}", missing.js_stringified());
    let e = parse(&beatscript_api::expand_json(&borrower, false));
    assert!(!b(&e, "ok"), "{}", e.js_stringified());
    beatscript_api::known::know(&song_cid, &song_text).unwrap();
    let got = parse(&beatscript_api::check_json(&borrower));
    assert!(b(&got, "ok"), "{}", got.js_stringified());
    assert_eq!(get(&got, "uses").as_array().unwrap().iter().filter_map(JsonValue::as_str).collect::<Vec<_>>(), [song_cid.as_str()]);
    let e = parse(&beatscript_api::expand_json(&borrower, false));
    assert!(b(&e, "ok"), "{}", e.js_stringified());
    assert!(s(&e, "text").contains("# from ") && s(&e, "text").contains("section hook 4 bars {"), "{}", s(&e, "text"));
    assert_eq!(get(&e, "refs").as_array().unwrap().len(), 1);
    let sh = parse(&beatscript_api::share_json(&borrower, None, false, ""));
    assert!(b(&sh, "ok") && s(&sh, "url").contains('*'), "{}", sh.js_stringified());
    beatscript_api::known::forget_all();
    let opened = parse(&beatscript_api::open_json(s(&sh, "url")));
    assert!(b(&opened, "ok") && b(&opened, "verified"), "{}", opened.js_stringified());
    assert_eq!(s(&opened, "cid"), s(&got, "cid"));
    beatscript_api::known::forget_all();
}
