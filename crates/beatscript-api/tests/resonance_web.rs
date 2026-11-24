//! `resonance` on the web: the browser plays the booth's insert filter (engine-calls.js, djrust `live_op("inserts", ..)`), its sweep
//! and its resonance at the booth's own curves, so `deck A: resonance 0.2 -> 0.4 over 8 bars` and `master: resonance 0.7` check
//! clean here. The page's `mapCall` and `snapshot::browser_plays` are the one list in two languages (site/test/wasm-parity.mjs
//! pins the page's half).

use djengine::json::JsonValue;

fn report(text: &str) -> JsonValue {
    let r = JsonValue::parse(&beatscript_api::check_json(text)).expect("check_json is JSON");
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{text}: {}", beatscript_api::check_json(text));
    r
}

fn warnings(text: &str) -> Vec<String> {
    report(text).get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().filter_map(|d| d.get("message").and_then(JsonValue::as_str)).map(str::to_string).collect()
}

#[test]
fn the_browser_plays_resonance_on_a_deck_and_the_master_and_its_sweeps() {
    for line in [
        "deck A: resonance 0.4",
        "master: resonance 0.7",
        "deck A: resonance 0.2 -> 0.4 over 8 bars",
        "deck B: resonance 0.2 -> 0.8 over 4 bars exp",
        "at bar 2 for 2 bars: master: resonance 0.9",
        "deck A: filter lp 800 -> 300 over 4 bars",
    ] {
        let w = warnings(&format!("deck A: kick x ~ x ~\ndeck B: bass 1 ~ 3 ~\n{line}\n"));
        assert!(!w.iter().any(|m| m.contains("filter.res") || m.contains("filter.sweep")), "`{line}` says the browser does not play it: {w:?}");
        assert!(w.is_empty(), "`{line}`: {w:?}");
    }
}

#[test]
fn hand_dubstep_140_v4_has_no_resonance_warning() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../site/examples/hand-dubstep-140-v4.beatscript")).unwrap();
    let r = report(&text);
    assert_eq!(r.get("cid").and_then(JsonValue::as_str), Some("z4EBG9izCf4cqe8shcYtiQcfrHS49zzM4gNW5Bt3B6CuhUC3M1M"), "the test program is v4");
    let w: Vec<String> = r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().filter_map(|d| d.get("message").and_then(JsonValue::as_str)).map(str::to_string).collect();
    assert!(!w.iter().any(|m| m.contains("filter.res") || m.contains("resonance")), "{w:#?}");
}

#[test]
fn the_rest_of_the_filter_and_the_other_inserts_still_say_so() {
    let w = warnings("deck A: kick x ~ x ~\ndo inserts set target=A param=filter.mix value=0.5\n");
    assert!(w.iter().any(|m| m.contains("`filter.mix`")), "{w:?}");
    let w = warnings("deck A: kick x ~ x ~\nmaster: delay 0.3\n");
    assert!(w.iter().any(|m| m.contains("delay")), "{w:?}");
}
