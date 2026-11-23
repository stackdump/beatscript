//! `drive` on the web (docs/BEATSCRIPT.md "Control"): the browser plays the FX unit's drive (engine-calls.js, `perform {op: fx}`), so a
//! program that drives a deck or the master checks clean here, while the rest of the unit and the loop channel still say they are
//! not played. The page's `mapCall` and `snapshot::browser_plays` are the one list in two languages.

use djengine::json::JsonValue;

fn warnings(text: &str) -> Vec<String> {
    let r = JsonValue::parse(&beatscript_api::check_json(text)).expect("check_json is JSON");
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{text}: {}", beatscript_api::check_json(text));
    r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().filter_map(|d| d.get("message").and_then(JsonValue::as_str)).map(str::to_string).collect()
}

#[test]
fn the_browser_plays_a_decks_drive_and_the_masters_and_its_ramps() {
    for line in ["master: drive 0.6", "deck A: drive 0.4", "deck B: drive 0.2 -> 0.9 over 4 bars exp", "master: drive lfo sine 2 bars center=0.5 depth=0.3", "at bar 2 for 2 bars: master: drive 0.8"] {
        let w = warnings(&format!("deck A: kick x ~ x ~\n{line}\n"));
        assert!(w.is_empty(), "`{line}` says the browser does not play it: {w:?}");
    }
}

#[test]
fn the_rest_of_the_fx_unit_and_the_loop_channel_are_still_not_played_here() {
    let w = warnings("deck A: kick x ~ x ~\ndo fx set deck=A param=crush value=0.3\n");
    assert!(w.iter().any(|m| m.contains("the FX unit's `crush`")), "{w:?}");
    let w = warnings("deck A: kick x ~ x ~\ndo fx set deck=L param=drive value=0.3\n");
    assert!(w.iter().any(|m| m.contains("deck `L`")), "{w:?}");
    let w = warnings("deck A: kick x ~ x ~\ndo fx kill deck=A\n");
    assert!(w.iter().any(|m| m.contains("`fx kill`")), "{w:?}");
}

#[test]
fn a_program_with_a_drive_line_is_another_program_with_its_own_cid() {
    // (the programs without the word keep their CIDs: tests/cid_pins.rs, matrix-130 with its `section drive` among them)
    let a = JsonValue::parse(&beatscript_api::check_json("tempo 120\ndeck A: kick x ~ x ~\n")).unwrap();
    let b = JsonValue::parse(&beatscript_api::check_json("tempo 120\ndeck A: kick x ~ x ~\nmaster: drive 0\n")).unwrap();
    assert_ne!(a.get("cid"), b.get("cid"), "a program with a line is another program");
    assert!(b.get("canonical").and_then(JsonValue::as_str).unwrap().contains("master: drive 0"));
}
