//! A `macro` line on the web (docs/beatscript/ext-control.md): it checks, and the browser plays none of the DJ macro catalogue, so `check`
//! names the line as the program says it (`macro riser`) and says where it does play: render and listen. The page's `mapCall`
//! (engine-calls.js, site/test/unit.mjs) and `snapshot::browser_plays` say the same.

use djengine::json::JsonValue;

#[test]
fn check_says_the_browser_does_not_play_a_macro_and_render_and_listen_do() {
    let text = "deck A: kick x ~ x ~\nat bar 2: macro riser on A over 4 bars\n";
    let r = JsonValue::parse(&beatscript_api::check_json(text)).unwrap();
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true));
    let d = r.get("diagnostics").and_then(JsonValue::as_array).unwrap();
    let w = d.iter().find(|d| d.get("message").and_then(JsonValue::as_str).is_some_and(|m| m.contains("does not play"))).expect("a warning");
    assert!(w.get("message").and_then(JsonValue::as_str).unwrap().contains("`macro riser` (the browser plays none of the DJ macro catalogue; render and listen do)"), "{}", w.js_stringified());
    assert!(w.get("help").and_then(JsonValue::as_str).unwrap().contains("`render` and `listen` play it"), "{}", w.js_stringified());
    let e = |p: &str| beatscript_api::snapshot::browser_plays("effects", &JsonValue::parse(p).unwrap());
    assert_eq!(e(r#"{"action":"release","id":"tape_stop","target":"A"}"#).unwrap_err(), "`macro tape_stop` (the browser plays none of the DJ macro catalogue; render and listen do)");
    assert!(e(r#"{"action":"tap","id":"riser"}"#).unwrap_err().contains("`effects tap`"));
}
