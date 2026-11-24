//! project-datum 6685: an info is a note, not a warning, and an evolve's summary says what the section plays.
//!
//! The repro is generated trap (seed 38953, long, 140 BPM, D minor): its three diagnostics are the ontology's rhythm notes,
//! severity info, and its evolving sections say `stop when gen == 5` in 24 bars. It used to check as `warnings: 3` and its
//! evolve summary claimed a tail of 26 generations (104 bars) and a cycle of 4 the section never reaches.

use beatscript_api::generate::{generate, GenRequest};
use djengine::json::JsonValue;

const REPRO_CID: &str = "z4EBG9j1GNdChBUSszyidKRiLZEL5vECeFGppDFeu2eZXEDqUUT";

fn n(r: &JsonValue, k: &str) -> i64 {
    r.get(k).and_then(JsonValue::as_i64).unwrap_or_else(|| panic!("{k} is not a number: {}", r.js_stringified()))
}

fn severities(r: &JsonValue) -> Vec<String> {
    r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().map(|d| d.get("severity").and_then(JsonValue::as_str).unwrap().to_string()).collect()
}

fn evolve(r: &JsonValue) -> Vec<String> {
    r.get("evolve").and_then(JsonValue::as_array).unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect()
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

#[test]
fn the_evolve_summary_names_the_stop_within_the_section() {
    let r = beatscript_api::check_report(&repro());
    let ev = evolve(&r);
    assert!(!ev.is_empty(), "{}", r.js_stringified());
    for scope in ["drop", "chorus"] {
        let line = ev.iter().find(|l| l.starts_with(&format!("evolve in {scope}:"))).unwrap_or_else(|| panic!("no evolve in {scope}: {ev:?}"));
        assert!(line.contains("generations 0 to 5 in 24 bars, then it stops at gen 5"), "{line}");
        assert!(line.contains("does not cycle") && !line.contains("tail") && !line.contains("period"), "{line}");
    }
}

#[test]
fn the_evolve_summary_says_a_cycle_only_when_one_plays() {
    // rotate 1 (a sixteenth along) comes back every 16 generations: 40 bars of 2 play generations 0 to 19, so it cycles
    let cycles = beatscript_api::check_report("tempo 120\nsection drop 40 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 2 bars {\n    bass: rotate 1\n  }\n}\narrange drop\n");
    let ev = evolve(&cycles);
    assert!(ev[0].starts_with("evolve in drop: tail 0 generations (0 bars), then it cycles: period 16 generations = period 32 bars"), "{ev:?}");
    // 6 bars of 2 play generations 0 to 2: the section ends long before generation 16 comes back to generation 0
    let short = beatscript_api::check_report("tempo 120\nsection drop 6 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 2 bars {\n    bass: rotate 1\n  }\n}\narrange drop\n");
    assert_eq!(evolve(&short), ["evolve in drop: generations 0 to 2 in 6 bars, then the section ends; no generation repeats, so it does not cycle"]);
    // a stop past the section's end: what plays is what fits
    let past = beatscript_api::check_report("tempo 120\nsection drop 6 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 2 bars {\n    bass: rotate 1\n    stop when gen == 3\n  }\n}\narrange drop\n");
    assert_eq!(evolve(&past), ["evolve in drop: generations 0 to 2 in 6 bars, then the section ends; no generation repeats, so it does not cycle"]);
    // a program of its own with no stop plays its orbit through, and says so
    let own = beatscript_api::check_report("tempo 120\ndeck A: bass [a2 e2 c2 g2]\nevolve every 2 bars {\n  bass: rotate 1\n}\n");
    assert!(evolve(&own)[0].starts_with("evolve: tail 0 generations (0 bars), then it cycles: period 16 generations"), "{:?}", evolve(&own));
}
