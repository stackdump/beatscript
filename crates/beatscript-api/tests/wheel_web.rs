//! `wheel` on the web: the browser plays the Wheel (djweb src/wheel.rs, windwheel's tonewheel, the booth's own
//! instrument; engine-calls.js `wheel`), so a program's wheel lines check clean here. The page's `mapCall` and
//! `snapshot::browser_plays` are the one list in two languages (site/test/unit.mjs has the page's half).

use djengine::json::JsonValue;

fn warnings(text: &str) -> Vec<String> {
    let r = JsonValue::parse(&beatscript_api::check_json(text)).expect("check_json is JSON");
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{text}: {}", beatscript_api::check_json(text));
    r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().filter_map(|d| d.get("message").and_then(JsonValue::as_str)).map(str::to_string).collect()
}

/// hand-dubstep 140, v4 (z4EBG9izCf4cqe8shcYtiQcfrHS49zzM4gNW5Bt3B6CuhUC3M1M as BeatScript 2): its wheel lines say
/// nothing about the browser.
#[test]
fn hand_dubstep_v4_has_no_wheel_warnings() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/hand-dubstep-140-v4.beatscript")).unwrap();
    let r = JsonValue::parse(&beatscript_api::check_json(&src)).unwrap();
    assert_eq!(r.get("renamed_cid").and_then(JsonValue::as_str), Some("z4EBG9izCf4cqe8shcYtiQcfrHS49zzM4gNW5Bt3B6CuhUC3M1M"), "the program is the one asked about");
    let w = warnings(&src);
    assert!(!w.iter().any(|m| m.contains("wheel")), "{w:?}");
}

#[test]
fn every_wheel_action_plays_here() {
    for line in [
        "wheel load preset=cmaj7-hoop",
        "wheel key root=F",
        "wheel key transpose=-2",
        "wheel sequence notes=\"C5 Ab4 F4 G4\"",
        "wheel brightness value=0.4",
        "wheel play",
        "wheel pause",
        "wheel toggle",
        "wheel motor on=true speed=1.05",
        "wheel level db=10",
        "wheel interval beats=1/2 deck=A",
        "wheel interval seconds=0.25",
        "wheel sync deck=A ratio=1/4",
        "wheel step deck=B up=off",
        "wheel follow deck=B",
        "wheel pluck tube=1",
        "wheel trigger on=true pull=0.3",
        "at bar 4: wheel play",
        "repeat 2 every 2 bars {\n  wheel play\n  at bar 1: wheel pause\n}",
    ] {
        let w = warnings(&format!("deck A: kick x ~ x ~\n{line}\n"));
        assert!(w.is_empty(), "`{line}` says the browser does not play it: {w:?}");
    }
    for action in ["status", "presets", "play", "load", "motor", "sync"] {
        let p = JsonValue::parse(&format!(r#"{{"action":"{action}"}}"#)).unwrap();
        assert_eq!(beatscript_api::snapshot::browser_plays("wheel", &p), Ok(()), "{action}");
    }
}

#[test]
fn the_wheels_strip_inserts_are_still_not_played_here() {
    // the booth's Windwheel input strip has an insert chain; the browser's wheel has none of its own
    let p = JsonValue::parse(r#"{"action":"set","target":"Wheel","param":"filter.sweep","value":0.2}"#).unwrap();
    assert!(beatscript_api::snapshot::browser_plays("inserts", &p).is_err());
}
