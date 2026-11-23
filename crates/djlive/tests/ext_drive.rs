//! `drive` (docs/BEATSCRIPT.md "Control"): one more word of the control grammar, said after a target and ramped, swung and held like
//! `reverb`. It compiles to the control language's own `fx` noun (the FX unit's drive), is a free name everywhere else (the
//! pinned example `matrix-130` has `section drive`), and a line that is wrong says its range and the nearest value.

use djengine::json::JsonValue;
use djlive::compile::{Call, Compiled, Effect};
use djlive::nets::{self, Event};

fn ok(src: &str) -> Compiled {
    let c = djlive::check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<String> {
    djlive::check(src).diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{} | {}", d.message, d.help.clone().unwrap_or_default())).collect()
}

fn calls(c: &Compiled, i: usize) -> Vec<Call> {
    match &c.controls[i].effect {
        Effect::Calls(cs) => cs.clone(),
        other => panic!("not plain calls: {other:?}"),
    }
}

fn num(v: &JsonValue, k: &str) -> Option<f64> {
    v.get(k).and_then(JsonValue::as_f64)
}

fn text(v: &JsonValue, k: &str) -> Option<String> {
    v.get(k).and_then(JsonValue::as_str).map(str::to_string)
}

#[test]
fn a_deck_drive_is_the_fx_units_drive_on_that_deck_and_the_masters_is_on_both() {
    let c = ok("tempo 128\ndeck A: kick x ~ ~ ~\ndeck A: drive 0.6\nmaster: drive 25%\n");
    let a = calls(&c, 0);
    assert_eq!((a.len(), a[0].noun.as_str()), (1, "fx"));
    assert_eq!((text(&a[0].params, "action").as_deref(), text(&a[0].params, "deck").as_deref(), text(&a[0].params, "param").as_deref(), num(&a[0].params, "value")), (Some("set"), Some("A"), Some("drive"), Some(0.6)));
    // `master:` is the call with no deck, which the control language reads as both decks (`djapp_fx`: "A, B, or both when omitted")
    let m = calls(&c, 1);
    assert_eq!(m[0].noun, "fx");
    assert!(m[0].params.get("deck").is_none(), "{:?}", m[0].params);
    assert_eq!((text(&m[0].params, "param").as_deref(), num(&m[0].params, "value")), (Some("drive"), Some(0.25)));
}

#[test]
fn a_drive_is_ramped_swung_and_held_like_the_other_effects() {
    let c = ok("tempo 128\ndeck A: kick x ~ ~ ~\nmaster: drive 0.1 -> 0.9 over 4 bars exp\ndeck B: drive lfo sine 2 bars center=0.5 depth=0.3\nat bar 2 for 2 bars: master: drive 0.8\n");
    let Effect::Ramp(r) = &c.controls[0].effect else { panic!("a ramp") };
    assert_eq!((r.noun.as_str(), r.field.as_str(), r.from, r.to), ("fx", "value", 0.1, 0.9));
    assert!(matches!(c.controls[1].effect, Effect::Lfo(_)));
    // `for 2 bars` puts it back to 0, where the FX unit does nothing
    let undo = &c.controls[2].undo;
    assert_eq!(undo.len(), 1);
    assert_eq!((undo[0].noun.as_str(), text(&undo[0].params, "param").as_deref(), num(&undo[0].params, "value")), ("fx", Some("drive"), Some(0.0)));
    // a canonical text keeps its words
    assert!(c.text.contains("master: drive 0.1 -> 0.9 over 4 bars exp"), "{}", c.text);
}

#[test]
fn a_drive_out_of_range_says_the_range_and_the_nearest_value() {
    let e = errors("deck A: kick x\ndeck A: drive 2\n");
    assert_eq!(e, ["drive is 0 to 1 (or 0% to 100%) | the nearest is 1, the most"]);
    let e = errors("deck A: kick x\nmaster: drive -0.5\n");
    assert_eq!(e, ["drive is 0 to 1 (or 0% to 100%) | the nearest is 0, which is off"]);
    let e = errors("deck A: kick x\nmaster: drive 0.2 -> 1.5 over 2 bars\n");
    assert_eq!(e.len(), 1);
    assert!(e[0].starts_with("drive is 0 to 1"), "{e:?}");
    // no target says which there are
    let e = errors("deck A: kick x\ndrive 0.3\n");
    assert_eq!(e, ["`drive` needs a target | master: drive 0.6   or   deck A: drive 0.4"]);
    // a ramp with no length is the other effects' error
    assert!(errors("deck A: kick x\nmaster: drive 0.2 -> 0.8\n")[0].contains("a ramp needs a length"));
}

#[test]
fn drive_is_a_free_name_a_section_a_let_or_a_def_may_have() {
    // the pinned example matrix-130 has `section drive 16 bars` and `arrange ... drive ...`: it must keep reading, with its CID
    let c = ok("tempo 120\nsection drive 4 bars {\n  deck A: kick x ~ ~ ~\n  master: drive 0.5\n}\narrange drive\n");
    assert!(c.arranged);
    assert_eq!(calls(&c, 0)[0].noun, "fx");
    // a let called `drive` is its number where it is a number, and the word where the line reads the word:
    // the first `drive` of `master: drive drive` is the word, the second the let's 0.5
    let c = ok("tempo 120\nlet drive = 0.5\ndeck A: kick x ~ ~ ~\nmaster: drive drive\n");
    assert_eq!(num(&calls(&c, 0)[0].params, "value"), Some(0.5));
    // and a def called `drive` is called as before
    let c = ok("tempo 120\ndef drive(d) {\n  deck $d: kick x ~ ~ ~\n}\ndrive(A)\n");
    assert!(!c.decks.is_empty());
}

#[test]
fn a_drive_line_is_in_the_control_net_as_a_knob_of_its_target() {
    let c = ok("tempo 128\nsection main 2 bars {\n  deck A: kick x ~ ~ ~\n  deck A: drive 0.6\n  master: drive 0.2 -> 0.4 over 1 bars\n}\narrange main\n");
    let net = nets::control_net(&c, 2);
    let ev = net.events(None);
    let set = |tg: &str| -> Vec<(i64, f64)> { ev.iter().filter_map(|(t, e)| if let Event::Set { target, param, value } = e { (param == "drive" && target == tg).then_some((*t, *value)) } else { None }).collect() };
    assert_eq!(set("A"), [(0, 0.6)]);
    let m = set("Master");
    assert_eq!((m.first().copied(), m.last().copied()), (Some((0, 0.2)), Some((16, 0.4))));
    assert!(net.residual.iter().all(|(k, _)| !k.contains("drive")), "a drive line is in the net, not beside it: {:?}", net.residual);
    // the label round-trips (the net's CID covers every value of every ramp)
    let e = Event::Set { target: "Master".into(), param: "drive".into(), value: 0.25 };
    assert_eq!(e.text(), "set Master drive 0.25");
    assert_eq!(Event::parse(&e.text()), Ok(e));
}

#[test]
fn the_other_knobs_of_the_fx_unit_stay_beside_the_net_as_they_were() {
    // only the drive was given a place in the net: `do fx set ... param=crush` is the registry call it always was
    let c = ok("tempo 128\ndeck A: kick x ~ ~ ~\ndo fx set deck=A param=crush value=0.5\n");
    let net = nets::control_net(&c, 2);
    assert!(net.events(None).iter().all(|(_, e)| !matches!(e, Event::Set { param, .. } if param == "crush")));
    assert!(!net.residual.is_empty(), "the crush line is beside the net");
    // and a `do` of the drive is the same call as the word, so the same event
    let c = ok("tempo 128\ndeck A: kick x ~ ~ ~\ndo fx set deck=A param=drive value=0.5\n");
    let net = nets::control_net(&c, 2);
    assert!(net.events(None).iter().any(|(_, e)| matches!(e, Event::Set { target, param, value } if target == "A" && param == "drive" && *value == 0.5)));
}

#[test]
fn a_performed_drive_reads_its_input_when_it_plays() {
    let c = ok("tempo 128\ndeck A: kick x ~ ~ ~\nmaster: drive knob(1)\n");
    assert!(matches!(c.controls[0].effect, Effect::Performed(_)), "{:?}", c.controls[0].effect);
}

#[test]
fn a_misspelt_drive_is_met_with_the_word() {
    let d = djlive::check("deck A: kick x\nmaster: drve 0.5\n").diagnostics;
    let e = d.iter().find(|d| d.is_error()).expect("an error");
    assert!(e.help.as_deref().is_some_and(|h| h.contains("did you mean `drive`?")), "{e:?}");
}
