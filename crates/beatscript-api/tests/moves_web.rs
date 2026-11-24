//! The moves on the web (docs/PERFORM.md): the browser plays every move the booth plays (engine-calls.js `MoveScheduler`, a port of
//! djhost perform.rs `Moves`, and vocoder_handoff's phrased swap), so a program with a `move` line checks clean here. A phrase sync
//! that steps a seed is the one left: a program's deck plays its own nets, not a seed (the booth's offline render does not play it
//! either). The page's `mapCall` and `snapshot::browser_plays` are the one list in two languages.

use djengine::json::JsonValue;

fn warnings(text: &str) -> Vec<String> {
    let r = JsonValue::parse(&beatscript_api::check_json(text)).expect("check_json is JSON");
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{text}: {}", beatscript_api::check_json(text));
    r.get("diagnostics").and_then(JsonValue::as_array).unwrap().iter().filter_map(|d| d.get("message").and_then(JsonValue::as_str)).map(str::to_string).collect()
}

#[test]
fn the_browser_plays_every_move_the_booth_plays() {
    for line in [
        "at bar 2: move loop_roll on A over 4 bars",
        "at bar 2: move loop_roll on B",
        "at bar 2: move filter_exit on B over 4 bars",
        "at bar 2: move echo_out on A over 1 bars",
        "at bar 2: move bass_swap on B over 8 bars",
        "at bar 2: move vocoder_handoff on A",
        "at bar 2 for 2 bars: move echo_out on A",
        "at bar 2: do moves arm kind=phrase_sync deck=B do=cut",
        "at bar 4: do moves cancel kind=loop_roll deck=A",
        "at bar 4: do moves cancel_all",
    ] {
        let w = warnings(&format!("tempo 120\ndeck A: kick x ~ x ~\ndeck B: bass c2 ~ c2 ~\n{line}\n"));
        assert!(w.is_empty(), "`{line}` says the browser does not play it: {w:?}");
    }
}

#[test]
fn a_phrase_sync_that_steps_a_seed_is_still_not_played_here() {
    let w = warnings("tempo 120\ndeck A: kick x ~ x ~\ndeck B: bass c2 ~ c2 ~\nat bar 2: move phrase_sync on B\n");
    assert!(w.iter().any(|m| m.contains("`phrase_sync` that steps a seed")), "{w:?}");
}

/// The program the bug was found on: "hand-dubstep 140, v4" (site/examples), whose `before drop 4 bars: move loop_roll on A over
/// 4 bars` the page said it did not play.
#[test]
fn hand_dubstep_140_v4_has_no_move_warning() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../site/examples/hand-dubstep-140-v4.beatscript")).unwrap();
    assert!(text.contains("move loop_roll on A over 4 bars"), "the example still has its roll");
    let r = JsonValue::parse(&beatscript_api::check_json(&text)).unwrap();
    assert_eq!(r.get("cid").and_then(JsonValue::as_str), Some("z4EBG9izCf4cqe8shcYtiQcfrHS49zzM4gNW5Bt3B6CuhUC3M1M"), "the program's CID");
    let w = warnings(&text);
    assert!(!w.iter().any(|m| m.contains("loop_roll") || m.contains(" move ")), "{w:?}");
}
