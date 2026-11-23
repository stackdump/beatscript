//! The rename's promise, held (rename-to-beatscript: keep). BeatScript 1 text, the old spelling (`#! btw 1`, or no version
//! line), keeps the CID it had before the language was renamed; its upgrade to BeatScript 2 has one stable new CID; the two
//! are each other's `renamed_cid`; `fmt` upgrades, and formatting the upgrade again changes nothing; and the program plays
//! the same in both (the same nets).
//!
//! `tests/legacy/golden.json` holds every example program as it was before the rename and a sample of the programs stored on
//! beatscript.app, with the CID the pre-rename CLI gave each. `BEATSCRIPT_BLESS=1 cargo test -p beatscript-api --test legacy`
//! writes the BeatScript 2 CIDs; it never rewrites a legacy CID.

use djengine::json::{JsonObject, JsonValue};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/legacy/golden.json");

fn parse(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap()
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k).and_then(JsonValue::as_str).unwrap_or("")
}

#[test]
fn old_text_keeps_its_cid_and_its_upgrade_has_one_new_cid() {
    let golden = parse(&std::fs::read_to_string(GOLDEN).unwrap());
    let programs = golden.get("programs").and_then(JsonValue::as_array).unwrap().clone();
    assert!(programs.len() >= 90, "the golden set has every example and the stored sample: {}", programs.len());
    let bless = std::env::var("BEATSCRIPT_BLESS").is_ok();
    let mut out = Vec::new();
    let mut missing = 0;
    for p in &programs {
        let (source, text, old) = (s(p, "source"), s(p, "text"), s(p, "cid"));
        // the legacy path: the text as it was, read as it always was
        let c = parse(&beatscript_api::check_json(text));
        assert_eq!(s(&c, "cid"), old, "{source}: BeatScript 1 text has the CID it had before the rename");
        assert_eq!(c.get("version").and_then(JsonValue::as_i64), Some(1), "{source}");
        // the upgrade: fmt writes it in BeatScript 2, a new CID, and names the old one
        let f = parse(&beatscript_api::fmt_json(text));
        let new = s(&f, "cid").to_string();
        assert_ne!(new, old, "{source}: BeatScript 2 is a new CID");
        assert_eq!(s(&f, "from_cid"), old, "{source}");
        assert_eq!(s(&c, "renamed_cid"), new, "{source}: check names the CID the upgrade has");
        let up = s(&f, "text");
        assert!(up.starts_with("#! beatscript 2\n"), "{source}: the version line comes first:\n{up}");
        let c2 = parse(&beatscript_api::check_json(up));
        assert_eq!(s(&c2, "cid"), new, "{source}: the upgraded text checks as the program fmt said");
        assert_eq!(c2.get("version").and_then(JsonValue::as_i64), Some(2), "{source}");
        assert_eq!(s(&c2, "renamed_cid"), old, "{source}: and names its legacy CID");
        // the same music: the same nets, the canonical text the same but for the version line
        assert_eq!(c2.get("nets").map(JsonValue::js_stringified), c.get("nets").map(JsonValue::js_stringified), "{source}: plays the same");
        assert_eq!(djlive::version::legacy_canonical(s(&c2, "canonical")), s(&c, "canonical"), "{source}");
        // formatting the upgrade again changes nothing
        let f2 = parse(&beatscript_api::fmt_json(up));
        assert_eq!(s(&f2, "text"), up, "{source}: fmt of the upgraded text is a fixed point");
        assert_eq!(s(&f2, "cid"), new, "{source}");
        assert!(matches!(f2.get("from_cid"), None | Some(JsonValue::Null)), "{source}");
        // and the BeatScript 2 CID is the one pinned
        match p.get("v2_cid").and_then(JsonValue::as_str) {
            Some(pinned) => assert_eq!(new, pinned, "{source}: the BeatScript 2 CID moved"),
            None => missing += 1,
        }
        let mut o = JsonObject::new();
        o.set("source", JsonValue::str(source));
        o.set("cid", JsonValue::str(old));
        o.set("v2_cid", JsonValue::str(new));
        o.set("text", JsonValue::str(text));
        out.push(JsonValue::Object(o));
    }
    if bless {
        // one program a line, so a diff of the file reads
        let about = golden.get("_about").cloned().unwrap_or(JsonValue::Null).js_stringified();
        let lines: Vec<String> = out.iter().map(JsonValue::js_stringified).collect();
        std::fs::write(GOLDEN, format!("{{\n\"_about\": {about},\n\"programs\": [\n{}\n]\n}}\n", lines.join(",\n"))).unwrap();
    } else {
        assert_eq!(missing, 0, "BEATSCRIPT_BLESS=1 writes the BeatScript 2 CIDs");
    }
}

/// The examples in the repo are written in BeatScript 2: each one that was in the golden set is the upgrade of the text it was,
/// so the CID it has now is the golden set's `v2_cid` for it and its legacy CID is the one it had.
#[test]
fn the_examples_are_the_upgrades_of_what_they_were() {
    let golden = parse(&std::fs::read_to_string(GOLDEN).unwrap());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for p in golden.get("programs").and_then(JsonValue::as_array).unwrap() {
        let source = s(p, "source");
        if source.starts_with("stored:") {
            continue;
        }
        // the path it has now (rename-to-beatscript.py's moves)
        let now = source.replace("examples/btw/", "examples/beatscript/").replace("docs/btw/", "docs/beatscript/").replace(".btw", ".beatscript");
        let Ok(text) = std::fs::read_to_string(root.join(&now)) else { panic!("{now}: the example is gone") };
        let c = parse(&beatscript_api::check_json(&text));
        assert_eq!(c.get("version").and_then(JsonValue::as_i64), Some(2), "{now}: examples are BeatScript 2");
        if let Some(v2) = p.get("v2_cid").and_then(JsonValue::as_str) {
            assert_eq!(s(&c, "cid"), v2, "{now}: the example is the upgrade of what it was");
            assert_eq!(s(&c, "renamed_cid"), s(p, "cid"), "{now}: its legacy CID is the one it had");
        }
        seen += 1;
    }
    assert!(seen >= 60, "{seen}");
}
