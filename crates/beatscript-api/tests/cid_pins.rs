// rename-to-beatscript: keep (reads the legacy .btw extension too)
//! Every BeatScript program in the repo keeps its CIDs: the program's, each net's and each song net's, and a CID of
//! every envelope `nets` hands the player. `tests/cid-pins.json` was written before the expression marks
//! (ghost `.`, flam `&`, ratchet `:N`, `glide=`, `decay=`, `pitch-drop=`) existed, so a program that does not use
//! them is proved to compile to exactly what it did. `BEATSCRIPT_BLESS=1 cargo test -p beatscript-api --test cid_pins` rewrites
//! the file (only ever for a program that is new, or one whose change is meant).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use djengine::cid;
use djengine::json::{JsonObject, JsonValue};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every `.beatscript` and `.live` file under the repo's examples, docs and test fixtures.
fn programs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["examples", "docs", "crates/beatscript-api/tests", "crates/djlive/tests", "crates/djlive/examples"] {
        walk(&root().join(dir), &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("beatscript" | "btw" | "live")) {
            out.push(p);
        }
    }
}

fn pin(text: &str) -> JsonValue {
    let r = beatscript_api::check_report(text);
    let check = r.get("check").cloned().unwrap_or(r.clone());
    let mut o = JsonObject::new();
    for k in ["ok", "cid", "nets", "song_nets"] {
        o.set(k, check.get(k).cloned().unwrap_or(JsonValue::Null));
    }
    let nets = JsonValue::parse(&beatscript_api::nets_json(text, true)).expect("nets answers JSON");
    o.set("envelopes", JsonValue::str(cid::share_cid(&nets)));
    JsonValue::Object(o)
}

#[test]
fn every_program_in_the_repo_keeps_its_cids() {
    let path = root().join("crates/beatscript-api/tests/cid-pins.json");
    let mut have: BTreeMap<String, JsonValue> = BTreeMap::new();
    for p in programs() {
        let rel = p.strip_prefix(root()).unwrap().to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(&p).unwrap();
        have.insert(rel, pin(&text));
    }
    assert!(have.len() >= 40, "found only {} programs", have.len());
    if std::env::var("BEATSCRIPT_BLESS").is_ok() {
        // one program a line, so a diff of the file names the program that moved
        let lines: Vec<String> = have.iter().map(|(k, v)| format!("  {}: {}", JsonValue::str(k.as_str()).js_stringified(), v.js_stringified())).collect();
        std::fs::write(&path, format!("{{\n{}\n}}\n", lines.join(",\n"))).unwrap();
        return;
    }
    let want = JsonValue::parse(&std::fs::read_to_string(&path).expect("tests/cid-pins.json")).unwrap();
    let want = want.as_object().expect("an object");
    let mut moved = Vec::new();
    for (k, v) in want.iter() {
        match have.get(k) {
            Some(h) if h == v => {}
            Some(h) => moved.push(format!("{k}: was {} now {}", v.js_stringified(), h.js_stringified())),
            None => moved.push(format!("{k}: pinned but no longer in the repo")),
        }
    }
    assert!(moved.is_empty(), "programs whose CIDs moved:\n{}", moved.join("\n"));
}
