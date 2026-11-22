//! project-datum 6685: an info is a note, not a warning.
//!
//! The repro is generated trap (seed 38953, long, 140 BPM, D minor): its three diagnostics are the ontology's rhythm notes,
//! severity info, and its evolving sections say `stop when gen == 5` in 24 bars. It used to check as `warnings: 3` and its
//! evolve summary claimed a tail of 26 generations (104 bars) and a cycle of 4 the section never reaches.

use beatscript_api::generate::{generate, GenRequest};
use djengine::json::JsonValue;

const REPRO_CID: &str = "z4EBG9jGM2ysxZvwdkidt3rSHzoUuqoEXtHNZB9grVV3c6G8C1T";

fn n(r: &JsonValue, k: &str) -> i64 {
    r.get(k).and_then(JsonValue::as_i64).unwrap_or_else(|| panic!("{k} is not a number: {}", r.js_stringified()))
}

fn severities(r: &JsonValue) -> Vec<String> {
    r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().map(|d| d.get("severity").and_then(JsonValue::as_str).unwrap().to_string()).collect()
}

fn repro() -> String {
    let g = generate(&GenRequest { genre: "trap".into(), bpm: Some(140.0), key: "dm".into(), length: "long".into(), seed: "38953".into() }).unwrap();
    g.text
}

#[test]
fn infos_are_counted_apart_from_warnings() {
    let r = beatscript_api::check_report(&repro());
    assert_eq!(r.get("cid").and_then(JsonValue::as_str), Some(REPRO_CID), "the generator still makes the repro");
    assert_eq!(severities(&r), ["info", "info", "info"], "{}", r.js_stringified());
    assert_eq!((n(&r, "errors"), n(&r, "warnings"), n(&r, "infos")), (0, 0, 3));
}

#[test]
fn a_real_warning_is_still_a_warning() {
    // a family word is accepted, with a warning that says what plays
    let r = beatscript_api::check_report("deck A: bass c2 ~ c2 ~ inst=reese\n");
    assert_eq!(severities(&r), ["warning"], "{}", r.js_stringified());
    assert_eq!((n(&r, "errors"), n(&r, "warnings"), n(&r, "infos")), (0, 1, 0));
    // and an error that does not compile still counts its warnings and infos (none here)
    let r = beatscript_api::check_report("deck A: kick x ~ x ~ kit=drums-909\n");
    assert!(n(&r, "errors") >= 1);
    assert_eq!(n(&r, "errors") + n(&r, "warnings") + n(&r, "infos"), severities(&r).len() as i64);
}
