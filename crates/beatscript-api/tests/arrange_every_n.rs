//! `check`'s report of an `arrange { }` with a counted mutation (`~3`, `@4`): which passes each counted line fired on and
//! which plays first read each firing. A program with no count has the report it always had: no `fires`, no `every`.

mod support;

use djengine::json::JsonValue;
use support::*;

fn check(text: &str) -> JsonValue {
    let r = parse(&beatscript_api::check_json(text));
    assert_report_invariants(text, &r);
    r
}

fn ints(v: &JsonValue) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as i64).collect()
}

#[test]
fn check_says_which_passes_each_counted_line_fired_on() {
    let r = check(include_str!("../../../examples/beatscript/arrange-every-n.beatscript"));
    assert_eq!(get(&r, "ok").as_bool(), Some(true));
    let arrange = get(&r, "arrange");
    assert_eq!(get(arrange, "passes").as_f64(), Some(6.0));
    let fires = get(arrange, "fires").as_array().unwrap();
    assert_eq!(fires.len(), 3);
    // `~3 riff rev`, `~2 bass_* drift 0.2`, `@4 kik double`, in the order the lines are written
    let line = |i: usize| (get(&fires[i], "mutation").as_f64().unwrap() as i64, get(&fires[i], "every").as_f64().unwrap() as i64, get(&fires[i], "once").as_bool().unwrap(), ints(get(&fires[i], "passes")));
    assert_eq!(line(0), (0, 3, false, vec![2, 5]));
    assert_eq!(line(1), (1, 2, false, vec![1, 3, 5]));
    assert_eq!(line(2), (2, 4, true, vec![3]));
    // `plays` are indices into the report's own `plays`: the first play that reads each firing
    let plays = get(arrange, "plays").as_array().unwrap();
    for f in fires {
        let (passes, at) = (ints(get(f, "passes")), ints(get(f, "plays")));
        assert_eq!(passes.len(), at.len());
        for (p, i) in passes.iter().zip(&at) {
            assert_eq!(get(&plays[*i as usize], "pass").as_f64(), Some(*p as f64));
        }
    }
    let name = |i: i64| get(&plays[i as usize], "name").as_str().unwrap().to_string();
    assert_eq!(ints(get(&fires[2], "plays")).into_iter().map(name).collect::<Vec<_>>(), ["lift@4"]);
    // a generation says how often its line fires; generation 0 and a line with no count say nothing
    let riff = get(arrange, "patterns.riff").as_array().unwrap();
    assert!(riff[0].as_object().unwrap().get("every").is_none());
    assert_eq!(riff[1..].iter().map(|g| get(g, "every").as_f64().unwrap() as i64).collect::<Vec<_>>(), [3, 3]);
    assert_eq!(riff[1..].iter().map(|g| get(g, "pass").as_f64().unwrap() as i64).collect::<Vec<_>>(), [2, 5]);
}

/// A firing at the very end (the last pass, no coda) is read by no play: null, not an index past the end.
#[test]
fn a_firing_nothing_plays_after_is_null() {
    let src = "tempo 120\nkey am\nlet syn_arp = a3 c4 e4 g4\nlet kik = x ~ x ~\nsection jam 2 bars {\n  deck A: kick kik\n  deck B: arp syn_arp\n}\narrange {\n  jam\n  ~3 syn_arp rev\n} loop 6 then end\n";
    let r = check(src);
    assert_eq!(get(&r, "ok").as_bool(), Some(true));
    let fires = get(get(&r, "arrange"), "fires").as_array().unwrap();
    assert_eq!(ints(get(&fires[0], "passes")), [2, 5]);
    let plays = get(&fires[0], "plays").as_array().unwrap();
    assert_eq!(plays[0].as_f64(), Some(3.0), "pass 2's first play after the line");
    assert_eq!(plays[1], JsonValue::Null, "pass 5's firing is at the end: no play reads it");
}

#[test]
fn a_program_with_no_count_has_the_report_it_always_had() {
    let src = include_str!("../../../examples/beatscript/arrange-mutations.beatscript");
    let r = check(src);
    let arrange = get(&r, "arrange");
    assert!(arrange.as_object().unwrap().get("fires").is_none());
    for (_, gens) in get(arrange, "patterns").as_object().unwrap().iter() {
        for g in gens.as_array().unwrap() {
            assert!(g.as_object().unwrap().get("every").is_none(), "{}", g.js_stringified());
        }
    }
    // a `~1` is no count
    let one = check(&src.replace("~ hats thin", "~1 hats thin"));
    assert!(get(&one, "arrange").as_object().unwrap().get("fires").is_none());
    assert_eq!(get(&one, "cid").as_str(), get(&r, "cid").as_str());
}

#[test]
fn a_line_that_never_fires_is_a_warning_in_the_report() {
    let src = include_str!("../../../examples/beatscript/arrange-every-n.beatscript").replace("} loop 6 then outro end", "} loop 2 then outro end");
    let r = check(&src);
    assert_eq!(get(&r, "ok").as_bool(), Some(true));
    let warns: Vec<String> = get(&r, "diagnostics").as_array().unwrap().iter().filter(|d| s(d, "severity") == "warning").map(|d| s(d, "message").to_string()).collect();
    assert!(warns.iter().any(|w| w == "`loop 2` plays 2 passes, so `~3` never fires"), "{warns:?}");
    assert!(warns.iter().any(|w| w == "`loop 2` plays 2 passes, so `@4` never fires"), "{warns:?}");
    // the line that does fire is still reported
    let fires = get(get(&r, "arrange"), "fires").as_array().unwrap();
    assert_eq!(fires.len(), 1);
    assert_eq!(ints(get(&fires[0], "passes")), [1]);
}

/// A switch of the control deck (`~ control off`, docs/beatscript/ext-control.md "Per pass: the switch"): each play the switches hold
/// lines of its control deck back in says which calls, as `control_off`, and the play is the section's copy (`groove@2`); a play
/// that fires them all has no `control_off`, so every other program's report is as it was. A counted switch is in `fires`.
#[test]
fn check_says_which_calls_a_switch_holds_back_in_each_play() {
    let r = check(include_str!("../../../examples/beatscript/control-per-pass.beatscript"));
    assert_eq!(get(&r, "ok").as_bool(), Some(true));
    let arrange = get(&r, "arrange");
    let plays = arrange.get("plays").and_then(|p| p.as_array()).expect("plays");
    let off = |p: &JsonValue| p.get("control_off").map(|v| v.canonical_js());
    let said: Vec<(String, Option<String>)> = plays.iter().map(|p| (get(p, "name").as_str().unwrap().to_string(), off(p))).collect();
    // pass 1 fires every line; pass 2 holds all but the duck back after its first two sections; pass 3's first lift holds its
    // freeze back (a one-shot); pass 4 is pass 2; the coda fires its crossfader
    let s = |n: &str, o: Option<&str>| (n.to_string(), o.map(str::to_string));
    let pass2 = [s("groove", None), s("lift", None), s("groove@2", Some(r#"["mix"]"#)), s("lift@2", Some(r#"["crossfader","freeze"]"#))];
    let mut want = vec![s("groove", None), s("lift", None), s("groove", None), s("lift", None)];
    want.extend(pass2.iter().cloned());
    want.extend([s("groove", None), s("lift@3", Some(r#"["freeze"]"#)), s("groove", None), s("lift", None)]);
    want.extend(pass2.iter().cloned());
    want.push(s("outro", None));
    assert_eq!(said, want);
    // the counted switches, by their lines among the block's mutation lines
    let fires = get(arrange, "fires").as_array().unwrap();
    let line = |i: usize| (get(&fires[i], "mutation").as_f64().unwrap() as i64, get(&fires[i], "every").as_f64().unwrap() as i64, get(&fires[i], "once").as_bool().unwrap(), ints(get(&fires[i], "passes")));
    assert_eq!(fires.len(), 3);
    assert_eq!(line(0), (0, 3, true, vec![2]));
    assert_eq!(line(1), (1, 2, false, vec![1, 3]));
    assert_eq!(line(2), (2, 2, false, vec![1, 3]));
    // a program with no switch: no play says anything of its control deck
    let r = check(include_str!("../../../examples/beatscript/arrange-every-n.beatscript"));
    assert!(get(&r, "arrange").get("plays").and_then(|p| p.as_array()).unwrap().iter().all(|p| p.get("control_off").is_none()));
}
