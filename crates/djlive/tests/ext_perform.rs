//! F13, the performed tier (docs/btw/ext-perform.md): `knob(3)`, `pad(5)`, `fader(A)`, `held(key)` and `held(c4)` in control
//! values and control guards, read at bar boundaries from the host, logged, and replayed from a performance. The heart is a
//! live run on a fake clock with scripted inputs against the replay of its log: the same calls, in the same order, with the
//! same numbers.

mod common;

use std::collections::BTreeMap;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, check_with, Compiled, Effect};
use djlive::diag::Severity;
use djlive::env::{ActionSpec, Env, NounSpec, ParamSpec, StaticRegistry, StaticSnippets};
use djlive::perform::{Input, InputBank, Performance};
use djlive::runtime::{Grid, Live, LiveHost};

// ---- a host with a clock and scripted controllers

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat, noun, params) of every call the runtime made
    calls: Vec<(f64, String, String)>,
    /// What each input says now, by id (`knob(3)`): a script changes these between passes.
    script: BTreeMap<String, f64>,
    /// How many times the runtime asked for each input.
    asked: BTreeMap<String, usize>,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }

    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }

    fn input(&mut self, kind: &str, id: &str) -> Option<f64> {
        let key = format!("{kind}({id})");
        *self.asked.entry(key.clone()).or_default() += 1;
        self.script.get(&key).copied()
    }
}

impl Fake {
    /// Deck A plays from beat 8: program bar 0 is the deck's bar 2.
    fn new() -> Fake {
        Fake { beat: 8.0, playing: [true, false], ..Fake::default() }
    }

    fn set(&mut self, input: &str, v: f64) {
        self.script.insert(input.to_string(), v);
    }

    /// Advance `beats` in steps of 1/4 beat, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats / 0.25).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }

    fn all(&self) -> Vec<(f64, String, String)> {
        self.calls.clone()
    }
}

fn p(name: &str, types: &[&str]) -> ParamSpec {
    ParamSpec { name: name.into(), types: types.iter().map(|s| s.to_string()).collect(), options: Vec::new(), required: false, doc: String::new(), ..Default::default() }
}

fn noun(name: &str, actions: &[&str], params: Vec<ParamSpec>) -> NounSpec {
    NounSpec { name: name.into(), default_action: actions.first().map_or(String::new(), |a| a.to_string()), actions: actions.iter().map(|a| ActionSpec { name: a.to_string(), effect: "live".into(), params: Vec::new() }).collect(), params }
}

fn registry() -> StaticRegistry {
    let mut r = StaticRegistry::default();
    r.nouns.insert("inserts".into(), noun("inserts", &["status", "on", "set"], vec![p("target", &["string"]), p("param", &["string"]), p("value", &["number"]), p("effect", &["string"]), p("on", &["boolean"])]));
    r.nouns.insert("vocoder".into(), noun("vocoder", &["status", "on", "set"], vec![p("mix", &["number"]), p("bands", &["integer"]), p("levels", &["array"])]));
    r
}

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn err_of(src: &str) -> djlive::Diagnostic {
    let c = check(src);
    assert!(c.compiled.is_none(), "should not compile: {src}");
    c.diagnostics.into_iter().find(|d| d.severity == Severity::Error).expect("an error")
}

/// The calls a program makes in `bars` bars at the start, as (noun, params) in order, with no inputs.
fn literal_calls(src: &str, bars: f64) -> Vec<(String, String)> {
    let mut h = Fake::new();
    let mut live = Live::new();
    assert!(live.eval(src).ok, "{src}");
    h.run(&mut live, bars * 4.0);
    h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect()
}

fn started(src: &str) -> (Fake, Live) {
    let mut live = Live::new();
    let ev = live.eval(src);
    assert!(ev.ok, "{}", djlive::diag::render_all(&ev.diagnostics, src));
    (Fake::new(), live)
}

// ---- reading and compiling

#[test]
fn an_input_in_a_control_value_is_a_performed_line_and_prints_as_written() {
    let c = compiled("master: filter lp 200 + knob(3) * 7800\n");
    assert_eq!(c.text, "master: filter lp 200 + knob(3) * 7800\n");
    let Effect::Performed(p) = &c.controls[0].effect else { panic!("{:?}", c.controls[0].effect) };
    assert_eq!(p.inputs, vec![Input::Knob(3)]);
    assert_eq!(p.patches.len(), 1);
    // the canonical text is a fixed point, so the CID does not move when it is read again
    assert_eq!(compiled(&c.text).cid, c.cid);
    // every input spelling prints as it was written
    for src in ["deck A: level fader(B)\n", "master: level pad(5)\n", "master: filter hp 20 + held(key) * 7000\n", "deck B: low held(c4)\n", "master: reverb knob(1) -> 1 - knob(2) over 4 bars\n"] {
        let c = compiled(src);
        assert_eq!(c.text, src, "{src}");
        assert_eq!(compiled(&c.text).cid, c.cid);
    }
}

#[test]
fn check_says_what_is_performed_and_when_it_is_read() {
    let c = check("master: filter lp 200 + knob(3) * 7800\ndeck A: level fader(A)\nevery 2 bars: master: reverb 0.1 when held(key)\n");
    let notes: Vec<&str> = c.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(notes.contains(&"performed: knob(3), fader(A), held(key) (read when its line is due)"), "{notes:?}");
    // a program with no inputs says nothing of the kind
    assert!(check("master: filter lp 800\n").diagnostics.iter().all(|d| !d.message.starts_with("performed")));
}

#[test]
fn a_control_value_with_an_input_reads_at_the_bar_it_is_due() {
    let (mut h, mut live) = started("master: filter lp 200 + knob(3) * 7800\n");
    h.set("knob(3)", 0.5);
    h.run(&mut live, 16.0);
    let calls = h.calls_of("inserts");
    // (the filter's insert is switched on, then its sweep set: both are `inserts`)
    assert_eq!(calls.len(), 2, "{calls:?}");
    // the same call the literal line makes with the value the input came to (4100 Hz)
    let literal = literal_calls("master: filter lp 4100\n", 4.0);
    assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), literal);
}

#[test]
fn a_level_a_cutoff_and_a_do_param_each_take_the_value_in_their_own_unit() {
    let src = "deck A: level knob(1)\nmaster: filter hp 20 + knob(2) * 7980\nmaster: resonance fader(A)\n";
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.25);
    h.set("knob(2)", 0.5);
    h.set("fader(A)", 1.0);
    h.run(&mut live, 12.0);
    let literal = literal_calls("deck A: level 0.25\nmaster: filter hp 4010\nmaster: resonance 1\n", 3.0);
    assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), literal);
}

#[test]
fn a_do_line_takes_a_performed_param_and_the_registry_is_still_checked() {
    let reg = registry();
    let env = Env { registry: Some(&reg), ..Env::none() };
    let src = "do inserts set param=delay.mix target=Master value=knob(2)\n";
    let checked = check_with(src, &env);
    let c = checked.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&checked.diagnostics, src)));
    assert_eq!(c.text, src);
    let mut live = Live::new();
    assert!(live.eval_with(src, &env).ok);
    let mut h = Fake::new();
    h.set("knob(2)", 0.75);
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("inserts"), vec![(12.0, r#"{"action":"set","param":"delay.mix","target":"Master","value":0.75}"#.to_string())]);
    // a param that is not a number cannot take one, and says so where it is written
    let bad = check_with("do inserts set target=knob(1) param=delay.mix value=1\n", &env);
    assert!(bad.compiled.is_none());
    // a whole-number param is rounded when it is read
    let bands = check_with("do vocoder set bands=2 + knob(1) * 14\n", &env);
    assert!(bands.compiled.is_some(), "{}", djlive::diag::render_all(&bands.diagnostics, ""));
    let mut live = Live::new();
    live.eval_with("do vocoder set bands=2 + knob(1) * 14\n", &env);
    let mut h = Fake::new();
    h.set("knob(1)", 0.5);
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("vocoder")[0].1, r#"{"action":"set","bands":9}"#);
}

#[test]
fn values_are_quantised_to_a_hundred_and_twenty_eighth_when_read() {
    let (mut h, mut live) = started("deck A: level knob(1)\n");
    h.set("knob(1)", 0.3);
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("mixer")[0].1, format!(r#"{{"deck":"A","volume":{}}}"#, 38.0 / 128.0));
}

#[test]
fn a_ramp_that_names_an_input_reads_it_at_every_bar_boundary_and_keeps_the_value_inside_the_bar() {
    let (mut h, mut live) = started("master: level fader(A) -> 0 over 4 bars\n");
    h.set("fader(A)", 1.0);
    // the line lands on bar 1 (beat 12): the ramp runs from the fader to 0 over four bars
    h.run(&mut live, 4.0);
    let at_start = h.calls_of("mixer");
    assert_eq!(at_start[0].0, 12.0);
    assert_eq!(at_start[0].1, r#"{"master":1}"#);
    // the fader moves inside the bar: the ramp keeps the value it read at the bar line
    h.set("fader(A)", 0.5);
    h.run(&mut live, 2.0);
    let before = h.calls_of("mixer").len();
    let last_in_bar: Vec<f64> = h.calls_of("mixer").iter().map(|c| JsonValue::parse(&c.1).unwrap().get("master").unwrap().as_f64().unwrap()).collect();
    assert!(last_in_bar.iter().all(|v| *v <= 1.0 && *v > 0.7), "{last_in_bar:?}");
    // the next bar line reads it again: the ramp's start is now 0.5
    h.run(&mut live, 2.5);
    let after: Vec<f64> = h.calls_of("mixer").iter().skip(before).map(|c| JsonValue::parse(&c.1).unwrap().get("master").unwrap().as_f64().unwrap()).collect();
    assert!(after.iter().any(|v| *v < 0.55), "{after:?}");
    assert!(*h.asked.get("fader(A)").unwrap() <= 4, "once a bar, not every pass: {:?}", h.asked);
}

#[test]
fn two_lines_that_name_an_input_read_it_once_each_time_they_are_due() {
    // (R1: an untimed value line follows its input, so it is due on every bar; the two lines are due together and read the knob once)
    let (mut h, mut live) = started("deck A: level knob(1)\ndeck B: level 1 - knob(1)\n");
    h.set("knob(1)", 0.25);
    h.run(&mut live, 8.0);
    assert_eq!(h.asked.get("knob(1)"), Some(&2), "once for each of the two bars they were due on, not once for each line: {:?}", h.asked);
    // and the knob did not move, so nothing was sent again
    let calls = h.calls_of("mixer");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, r#"{"deck":"A","volume":0.25}"#);
    assert_eq!(calls[1].1, r#"{"deck":"B","volume":0.75}"#);
}

// ---- guards

#[test]
fn a_when_on_a_control_line_fires_only_if_it_is_true_when_the_line_is_due() {
    let (mut h, mut live) = started("every 1 bars: master: level 0.5 when held(key)\n");
    // bar 1 (beat 12): not held; bar 2: held; bar 3: not; bar 4: held
    h.run(&mut live, 8.0);
    assert!(h.calls_of("mixer").is_empty(), "{:?}", h.calls);
    h.set("held(key)", 60.0 / 127.0);
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("mixer").len(), 1);
    h.set("held(key)", 0.0);
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("mixer").len(), 1);
    h.set("held(key)", 0.5);
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("mixer").len(), 2);
    assert!(live.words.iter().any(|w| w.contains("not run, its `when` is false")), "{:?}", live.words);
}

#[test]
fn a_guard_may_compare_combine_and_use_the_held_note() {
    let src = "every 1 bars: master: level 0.5 when held(c4) and knob(2) >= 0.5 or fader(B) < 0.1\n";
    let (mut h, mut live) = started(src);
    // held(c4) is 0, knob(2) is 0.9, fader(B) is 0.5: false
    h.set("knob(2)", 0.9);
    h.set("fader(B)", 0.5);
    h.run(&mut live, 4.0);
    assert!(h.calls_of("mixer").is_empty());
    // c4 held and the knob up: true
    h.set("held(c4)", 1.0);
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("mixer").len(), 1);
    // the fader down makes it true by the other side
    h.set("held(c4)", 0.0);
    h.set("fader(B)", 0.0);
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("mixer").len(), 2);
}

#[test]
fn a_one_shot_line_whose_guard_is_false_when_due_does_not_wait_for_it_to_come_true() {
    let (mut h, mut live) = started("master: level 0.5 when held(key)\n");
    h.run(&mut live, 8.0);
    h.set("held(key)", 0.5);
    h.run(&mut live, 16.0);
    assert!(h.calls_of("mixer").is_empty(), "due once, on the first bar line; the key came later");
}

#[test]
fn a_guard_that_is_not_about_an_input_is_decided_when_the_program_is_read_as_before() {
    let c = compiled("master: level 0.5 when 1 == 1\nmaster: level 0.7 when 1 == 2\n");
    assert_eq!(c.controls.len(), 1);
    assert!(matches!(c.controls[0].effect, Effect::Calls(_)));
}

#[test]
fn a_for_hold_puts_a_performed_line_back() {
    let (mut h, mut live) = started("at bar 1 for 1 bars: master: filter lp 200 + knob(3) * 7800\n");
    h.set("knob(3)", 0.5);
    h.run(&mut live, 20.0);
    let on: Vec<String> = h.calls_of("inserts").iter().map(|c| c.1.clone()).collect();
    assert!(on.iter().any(|c| c.contains(r#""on":true"#)), "{on:?}");
    assert!(on.iter().any(|c| c.contains(r#""on":false"#)), "put back: {on:?}");
}

// ---- no controller

#[test]
fn with_no_controller_a_knob_reads_its_home_and_a_pad_zero_and_btw_get_says_so() {
    // a knob nothing answers for rests in the middle (R4: the dull start of a performed program is gone), a pad at 0
    let (mut h, mut live) = started("deck A: level 0.5 + knob(4) * 0.5\ndeck B: level 0.5 + pad(2) * 0.5\n");
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("mixer")[0].1, r#"{"deck":"A","volume":0.75}"#);
    assert_eq!(h.calls_of("mixer")[1].1, r#"{"deck":"B","volume":0.5}"#);
    assert!(live.words.iter().any(|w| w.contains("knob(4): no controller is connected for it: it reads 1/2")), "{:?}", live.words);
    assert!(live.words.iter().any(|w| w.contains("pad(2): no controller is connected for it: it reads 0")), "{:?}", live.words);
    // said once
    assert_eq!(live.words.iter().filter(|w| w.contains("knob(4): no controller")).count(), 1);
    // and nothing was read, so nothing is logged
    assert!(live.performance().is_none());
}

// ---- the log

#[test]
fn every_value_read_is_logged_by_program_bar() {
    let (mut h, mut live) = started("every 1 bars: deck A: level knob(1)\n");
    h.set("knob(1)", 0.5);
    h.run(&mut live, 4.0);
    h.set("knob(1)", 0.25);
    h.run(&mut live, 4.0);
    h.set("knob(1)", 1.0);
    h.run(&mut live, 4.0);
    let perf = live.performance().expect("a take");
    assert_eq!(perf.program, live.running_cid().unwrap());
    let got: Vec<(i64, &str, f64)> = perf.inputs.iter().map(|r| (r.bar, r.input.as_str(), r.value)).collect();
    assert_eq!(got, vec![(1, "knob(1)", 0.5), (2, "knob(1)", 0.25), (3, "knob(1)", 1.0)]);
    let status = live.status();
    let p = status.get("performed").expect("performed in btw get");
    assert_eq!(p.get("inputs").unwrap().as_array().unwrap().len(), 1);
    // `btw get` (the window's look every frame) says the counts; the take's CID, which hashes the whole take, is the `perform` verb's (R-07)
    assert!(p.get("take").is_none());
    assert_eq!(p.get("take_readings").and_then(JsonValue::as_f64), Some(perf.inputs.len() as f64));
    assert_eq!(live.perform_status().get("take").unwrap().as_str().unwrap(), perf.cid());
}

#[test]
fn stopping_the_log_keeps_what_was_read_and_a_new_take_starts_clean() {
    let (mut h, mut live) = started("every 1 bars: deck A: level knob(1)\n");
    h.set("knob(1)", 0.5);
    h.run(&mut live, 8.0);
    live.perform_stop();
    h.set("knob(1)", 0.75);
    h.run(&mut live, 8.0);
    let kept = live.performance().unwrap();
    assert!(kept.inputs.iter().all(|r| r.value == 0.5), "{:?}", kept.inputs);
    live.perform_start();
    h.run(&mut live, 8.0);
    let fresh = live.performance().unwrap();
    assert!(fresh.inputs.iter().all(|r| r.value == 0.75 && r.bar >= kept.inputs.last().unwrap().bar), "{:?}", fresh.inputs);
}

// ---- the heart: a live run and its replay make the same calls

const SHOW: &str = "\
tempo 120
every 1 bars: master: filter lp 200 + knob(3) * 7800
every 1 bars: deck A: level fader(A)
every 2 bars: deck B: level 1 - knob(1) when held(key)
at bar 2: master: resonance knob(2) -> 1 over 4 bars
every 3 bars: master: reverb pad(5) * 0.6 when held(c4) or knob(2) > 0.5
";

/// What the performer does, bar by bar: a script of the controllers' values.
fn perform(h: &mut Fake, live: &mut Live, bars: usize) {
    let mut rng = Rng(0xfeed_beef_0001);
    for bar in 0..bars {
        // the performer moves things at the top of some bars, and wiggles a knob inside others
        h.set("knob(3)", (rng.below(129) as f64) / 128.0);
        h.set("fader(A)", (rng.below(129) as f64) / 128.0);
        h.set("knob(1)", (rng.below(129) as f64) / 128.0);
        h.set("knob(2)", (rng.below(129) as f64) / 128.0);
        h.set("pad(5)", if bar % 3 == 0 { 1.0 } else { 0.0 });
        h.set("held(key)", if bar % 4 < 2 { (36 + rng.below(30)) as f64 / 127.0 } else { 0.0 });
        h.set("held(c4)", if bar % 5 == 0 { 1.0 } else { 0.0 });
        h.run(live, 1.0);
        // a wiggle inside the bar: it must not change what the bar read
        h.set("knob(3)", (rng.below(129) as f64) / 128.0);
        h.run(live, 3.0);
    }
}

#[test]
fn a_live_run_and_the_replay_of_its_log_make_identical_calls() {
    // ---- live, with scripted controllers
    let (mut live_host, mut live) = started(SHOW);
    perform(&mut live_host, &mut live, 24);
    let live_calls = live_host.all();
    assert!(live_calls.len() > 40, "the show does something: {}", live_calls.len());
    let perf = live.performance().expect("a performance");
    assert_eq!(perf.program, live.running_cid().unwrap());
    assert!(perf.inputs.len() > 60, "{}", perf.inputs.len());

    // ---- saved and read back as JSON text, as the store does
    let text = perf.to_json().canonical_js();
    let back = Performance::from_json(&JsonValue::parse(&text).unwrap()).unwrap();
    assert_eq!(back.cid(), perf.cid());

    // ---- replayed by a fresh Live on a host whose controllers say nothing at all (or something else)
    let mut replay_host = Fake::new();
    replay_host.set("knob(3)", 0.999);
    replay_host.set("held(key)", 1.0);
    let mut replay = Live::new();
    assert!(replay.eval(SHOW).ok);
    replay.replay(back).expect("the same program");
    assert!(replay.replaying());
    // the replayed performer does not move: the log answers the reads
    let mut rng = Rng(1);
    for _ in 0..24 {
        replay_host.set("knob(3)", (rng.below(129) as f64) / 128.0);
        replay_host.run(&mut replay, 4.0);
    }
    assert_eq!(replay_host.all(), live_calls, "the replay makes the calls of the live run, in order, with the same numbers");
    // and it logs the same performance (it read the same things)
    assert_eq!(replay.performance().unwrap().cid(), perf.cid());
    assert!(replay_host.asked.is_empty(), "the replay asks the controllers for nothing: {:?}", replay_host.asked);
}

#[test]
fn the_replay_of_a_changed_log_makes_other_calls_and_is_another_performance() {
    let (mut h, mut live) = started(SHOW);
    perform(&mut h, &mut live, 12);
    let perf = live.performance().unwrap();
    let mut t = perf.clone();
    let k = t.inputs.iter().position(|r| r.input == "knob(3)" && r.bar == 4).expect("a read");
    t.inputs[k].value = if t.inputs[k].value > 0.5 { 0.0 } else { 1.0 };
    assert_ne!(t.cid(), perf.cid(), "one value changed: another performance");
    let mut rh = Fake::new();
    let mut r = Live::new();
    r.eval(SHOW);
    r.replay(t).unwrap();
    rh.run(&mut r, 12.0 * 4.0);
    assert_ne!(rh.all(), h.all()[..rh.all().len().min(h.all().len())].to_vec());
}

#[test]
fn a_log_of_another_program_is_refused() {
    let (mut h, mut live) = started(SHOW);
    perform(&mut h, &mut live, 4);
    let perf = live.performance().unwrap();
    // another program is evaluated: the log is not its log
    let mut other = Live::new();
    other.eval("master: level knob(1)\n");
    let e = other.replay(perf.clone()).unwrap_err();
    assert!(e.starts_with("refused:") && e.contains(&perf.program), "{e}");
    // nothing running at all
    let e = Live::new().replay(perf.clone()).unwrap_err();
    assert!(e.contains("no program is running"), "{e}");
    // the program the log is of: accepted, even before it has landed
    let mut same = Live::new();
    same.eval(SHOW);
    assert!(same.replay(perf).is_ok());
}

#[test]
fn replaying_a_performance_ends_when_another_program_is_applied() {
    let (mut h, mut live) = started(SHOW);
    perform(&mut h, &mut live, 4);
    let perf = live.performance().unwrap();
    let mut r = Live::new();
    r.eval(SHOW);
    r.replay(perf).unwrap();
    let mut rh = Fake::new();
    rh.run(&mut r, 8.0);
    assert!(r.replaying());
    r.eval("master: level 0.5\n");
    rh.run(&mut r, 8.0);
    assert!(!r.replaying(), "another program: the replay ended");
    assert!(r.words.iter().any(|w| w.contains("replay ended")), "{:?}", r.words);
}

// ---- errors: where an input may not stand

#[test]
fn an_input_in_a_pattern_is_an_error_that_says_what_to_do_instead() {
    for src in ["deck A: kick x*knob(2)\n", "deck A: kick e(knob(1), 8)\n", "deck A: kick x vel=knob(1)\n", "deck A: bass markov(am, knob(1))\n", "deck A: kick x?knob(1)\n"] {
        let d = err_of(src);
        assert!(d.message.contains("performed input") || d.message.contains("knob"), "{src}: {}", d.message);
    }
    let d = err_of("deck A: kick x*(1 + knob(2))\n");
    assert!(d.message.contains("`knob(...)` is a performed input"), "{}", d.message);
    assert!(d.help.as_deref().unwrap_or("").contains("control value"), "{:?}", d.help);
    assert_eq!(djlive::diag::line_col("deck A: kick x*(1 + knob(2))\n", d.span.start), (1, 21), "at the call");
    // a part cannot be guarded by one either
    let d = err_of("deck A: kick x ~ x ~ when held(key)\n");
    assert!(d.message.contains("an input cannot decide whether a part plays"), "{}", d.message);
    assert!(d.help.unwrap().contains("control line"));
}

#[test]
fn an_input_where_a_number_is_needed_when_the_program_is_read_is_an_error() {
    for src in ["tempo knob(1)\n", "repeat knob(1) {\n  master: level 0.5\n}\n", "at bar knob(1): master: level 0.5\n", "master: level 0.5 -> 1 over knob(1) bars\n", "swing knob(1)\n", "every knob(1) bars: master: level 0.5\n"] {
        let d = err_of(src);
        assert!(d.message.contains("performed input") || d.message.contains("expected"), "{src}: {}", d.message);
    }
    let d = err_of("master: level 0.5 -> 1 over knob(1) bars\n");
    assert!(d.message.contains("`knob(...)` is a performed input"), "{}", d.message);
}

#[test]
fn an_evolve_rule_does_not_take_an_input_and_says_why() {
    let d = err_of("deck A: bass c2 e2\nevolve every 2 bars {\n  bass: transpose knob(1)\n  stop when gen == 3\n}\n");
    assert!(d.message.contains("cannot stand in an `evolve` rule"), "{}", d.message);
    assert!(d.message.contains("btw generation"), "{}", d.message);
    assert_eq!(djlive::diag::line_col("deck A: bass c2 e2\nevolve every 2 bars {\n  bass: transpose knob(1)\n", d.span.start).0, 3);
}

#[test]
fn a_let_cannot_hold_an_input() {
    let d = err_of("let cut = 200 + knob(3) * 7800\nmaster: filter lp 800\n");
    assert!(d.message.contains("cannot hold an input"), "{}", d.message);
    assert!(d.help.unwrap().contains("def"));
}

#[test]
fn an_input_that_names_what_does_not_exist_is_an_error_with_a_place() {
    let src = "master: level fader(C)\n";
    let d = err_of(src);
    assert!(d.message.contains("a fader belongs to deck A or deck B"), "{}", d.message);
    assert_eq!(d.expected, ["A", "B"]);
    assert_eq!(djlive::diag::line_col(src, d.span.start), (1, 15));
    let d = err_of("master: level fader(a)\n");
    assert!(d.help.unwrap().contains("fader(A)"));
    let d = err_of("master: level knob(0)\n");
    assert!(d.message.contains("numbered from 1"), "{}", d.message);
    let d = err_of("master: level knob(129)\n");
    assert!(d.message.contains("up to 128 knobs"), "{}", d.message);
    let d = err_of("master: level pad(129)\n");
    assert!(d.message.contains("up to 128 pads"), "{}", d.message);
    let d = err_of("master: level held(c)\n");
    assert!(d.help.unwrap().contains("octave"));
    let d = err_of("master: level knob()\n");
    assert!(d.message.contains("takes one argument"), "{}", d.message);
    let d = err_of("master: level knob(1.5)\n");
    assert!(d.message.contains("whole number"), "{}", d.message);
    // and in a guard, at the call
    let src = "every 1 bars: master: level 0.5 when knob(199) > 0.5\n";
    let d = err_of(src);
    assert!(d.message.contains("there is no `knob(199)`"), "{}", d.message);
    assert_eq!(djlive::diag::line_col(src, d.span.start), (1, 38));
}

#[test]
fn the_connected_bank_does_not_change_what_a_program_checks_and_reads_home_where_it_has_no_control() {
    // (R-06) acceptance is the same on every machine: the connected controller's bank is host data, not a compile-time matter
    let small = Env { bank: Some(InputBank { knobs: 8, pads: 9 }), ..Env::none() };
    let big = Env { bank: Some(InputBank { knobs: 32, pads: 128 }), ..Env::none() };
    for src in ["master: level knob(8)\n", "master: level knob(12)\n", "master: level pad(100)\n", "every 1 bars: master: level 0.5 when knob(9) > 0\n", "master: level knob(128)\n"] {
        let (a, b, c) = (check_with(src, &small), check_with(src, &big), check(src));
        assert!(a.compiled.is_some() && b.compiled.is_some() && c.compiled.is_some(), "{src}");
        // the same program, the same CID and the same warnings, whatever is plugged in
        assert_eq!(a.compiled.as_ref().unwrap().cid, c.compiled.as_ref().unwrap().cid);
        assert_eq!(a.diagnostics.len(), c.diagnostics.len());
    }
    // there is a limit, the same everywhere
    for env in [&small, &big, &Env::none()] {
        assert!(check_with("master: level knob(129)\n", env).compiled.is_none());
        assert!(check_with("master: level pad(129)\n", env).compiled.is_none());
    }
    // and while it plays, a knob the connected bank does not have reads 1/2 and says so (the host answers None for it)
    let (mut h, mut live) = started("master: level knob(12)\n");
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("mixer")[0].1, r#"{"master":0.5}"#);
    assert!(live.words.iter().any(|w| w.contains("knob(12): no controller is connected for it: it reads 1/2")), "{:?}", live.words);
}

#[test]
fn a_value_that_can_never_be_in_range_is_an_error_and_one_that_sometimes_is_not_is_held() {
    let d = err_of("master: level knob(1) + 2\n");
    assert!(d.message.contains("always outside the range of `level`"), "{}", d.message);
    let c = check("master: level knob(1) * 2\n");
    assert!(c.compiled.is_some());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("can be 0 to 2: it is held to 0 to 1")), "{:?}", c.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    // held at runtime
    let (mut h, mut live) = started("master: level knob(1) * 2\n");
    h.set("knob(1)", 0.75);
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("mixer")[0].1, r#"{"master":1}"#);
    // a division that can divide by zero says so, and reads 0 when it does
    let c = check("master: level 0.5 + 0.1 / knob(1)\n");
    assert!(c.diagnostics.iter().any(|d| d.message.contains("can divide by zero")));
}

// ---- through defs, repeats, wires and imports

#[test]
fn an_input_goes_through_a_def_an_argument_and_a_repeat() {
    let src = "def sweep(cut, lvl) {\n  master: filter lp $cut * 2\n  deck A: level $lvl\n}\nsweep(100 + knob(3) * 3900, fader(B))\nrepeat 2 every 1 bars {\n  at bar $i + 1: deck B: level knob($i + 1)\n}\n";
    let c = compiled(src);
    // the argument keeps its grouping where it is put in place of `$cut * 2`
    assert!(c.controls.iter().any(|k| k.key.contains("filter lp (100 + knob(3) * 3900) * 2")), "{:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    let (mut h, mut live) = started(src);
    h.set("knob(3)", 0.5);
    h.set("knob(1)", 0.25);
    h.set("knob(2)", 0.5);
    h.set("fader(B)", 0.75);
    h.run(&mut live, 16.0);
    let literal = literal_calls("master: filter lp (100 + 1950) * 2\ndeck A: level 0.75\nat bar 1: deck B: level 0.25\nat bar 2: deck B: level 0.5\n", 4.0);
    assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), literal);
    stable(src, 0xf13_de5);
}

#[test]
fn an_input_in_a_def_called_through_a_wire_or_imported_keeps_its_marks() {
    let src = "def duckit(x) {\n  every 1 bars: deck A: level $x when held(key)\n}\nduckit(knob(1))\n";
    let c = compiled(src);
    let ctl = &c.controls[0];
    // the control line carries the line's own span and the call that made it
    assert_eq!(ctl.via.len(), 1);
    assert_eq!(ctl.via[0].text(&c.source), "duckit(knob(1))");
    assert!(ctl.span.text(&c.source).starts_with("every 1 bars"));
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.5);
    h.set("held(key)", 0.5);
    h.run(&mut live, 8.0);
    assert!(!h.calls_of("mixer").is_empty());
    // the marks name the line when it fires
    let marks = live.marks();
    assert!(!marks.controls.is_empty());
    assert!(marks.controls[0].span.text(&marks.source).starts_with("every 1 bars"));
}

// ---- canonical form

#[test]
fn performed_programs_have_one_canonical_form_and_a_semantic_edit_changes_the_cid() {
    let src = "tempo 120\nevery 1 bars: master: filter lp 200 + knob(3) * 7800 when held(key)\ndeck A: level fader(B) -> 1 over 2 bars\nmaster: reverb min(knob(1), 0.5)\n";
    let base = compiled(src);
    assert_eq!(compiled(&base.text).text, base.text);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..60 {
        let text = reformat(src, &mut rng);
        let c = check(&text);
        let got = c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text)));
        assert_eq!(got.cid, base.cid, "round {round} forked:\n{text}");
        assert_eq!(got.text, base.text);
    }
    for (from, to) in [("knob(3)", "knob(4)"), ("held(key)", "held(c4)"), ("fader(B)", "fader(A)"), ("7800", "7000"), ("min(knob(1), 0.5)", "max(knob(1), 0.5)")] {
        let other = compiled(&src.replace(from, to));
        assert_ne!(other.cid, base.cid, "{from} -> {to}");
    }
}

#[test]
fn spacing_inside_an_input_call_is_not_part_of_the_program() {
    let a = compiled("master: level knob(1)\n");
    let b = compiled("master:   level   knob ( 1 )\n");
    assert_eq!(a.cid, b.cid);
    let a = compiled("every 1 bars: master: level 0.5 when held(c4)\n");
    let b = compiled("every 1 bars: master: level 0.5 when held ( C4 )\n");
    assert_eq!(a.cid, b.cid, "a note is one spelling");
}

#[test]
fn an_input_word_still_names_nothing_else() {
    // an input is `knob(` with its parenthesis: the words name a let, a def or a section as they did before the controllers
    // (fix_syntax.rs has the programs), and `knob(3)` is still the input beside a let of that name
    for src in ["let knob = 3\nmaster: level 0.5\n", "let pad = 3\nmaster: filter lp knob(3) * 5000\n", "def held() {\n  master: level 0.5\n}\nheld()\n"] {
        let c = check(src);
        assert!(c.ok(), "{src}: {}", djlive::diag::render_all(&c.diagnostics, src));
    }
}

// ---- through wired ports and from an imported def

/// A def with a wired input, called from a repeat: its control lines hold an input, arithmetic over the wired number, `$i` and a
/// guard that names an input as well as `$i`.
const WIRED: &str = "\
def cutoff() out: tempo {
  emit 100 + 20
}
def lay(i) in: tempo {
  every 1 bars: master: filter lp $tempo * ($i + 1) + knob(3) * 1000 when held(key) or $i == 0
}
wire cutoff.out -> lay.in
repeat 2 {
  lay($i)
}
";

#[test]
fn an_input_through_a_wired_port_in_a_def_called_from_a_repeat() {
    let c = compiled(WIRED);
    assert_eq!(c.controls.len(), 2);
    let keys: Vec<&str> = c.controls.iter().map(|k| k.key.as_str()).collect();
    // the wired number keeps its grouping where it is put in place of `$tempo * (...)` (it is folded into the number it comes to
    // in the canonical text: only what names an input stays), and `$i` is each copy's own
    assert!(keys[0].ends_with("filter lp 120 + knob(3) * 1000 when held(key) or 0 == 0"), "{keys:?}");
    assert!(keys[1].ends_with("filter lp 240 + knob(3) * 1000 when held(key) or 1 == 0"), "{keys:?}");
    // the inputs are the same in both copies: knob(3) and held(key), each read once a bar
    let (mut h, mut live) = started(WIRED);
    h.set("knob(3)", 0.5);
    h.set("held(key)", 0.0);
    h.run(&mut live, 12.0);
    // copy 0's guard is `held(key) or 0 == 0`: true; copy 1's is `held(key) or 1 == 0`: false while nothing is held
    let sweeps: Vec<String> = h.calls_of("inserts").iter().filter(|c| c.1.contains("filter.sweep")).map(|c| c.1.clone()).collect();
    assert_eq!(sweeps.len(), 3, "copy 0 ran at bars 1, 2 and 3: {sweeps:?}");
    assert!(h.asked.get("knob(3)").is_some_and(|n| *n <= 3), "read once a bar: {:?}", h.asked);
    let want = literal_calls("every 1 bars: master: filter lp (100 + 20) * (0 + 1) + 500\n", 3.0);
    let got: Vec<(String, String)> = h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect();
    assert_eq!(got, want);
    // the key goes down: both copies run
    h.set("held(key)", 0.5);
    let before = h.calls.len();
    h.run(&mut live, 4.0);
    let new: Vec<String> = h.calls[before..].iter().filter(|c| c.2.contains("filter.sweep")).map(|c| c.2.clone()).collect();
    assert_eq!(new.len(), 2, "{new:?}");
    stable(WIRED, 0xf13_f11);
}

fn stable(src: &str, seed: u64) {
    let c = compiled(src);
    assert_eq!(compiled(&c.text).text, c.text, "the canonical text is not a fixed point");
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
}

/// A store of saved snippets (each may import the ones before it).
fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let c = check_with(text, &env);
        let canon = c.compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, text)));
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    snips
}

#[test]
fn a_def_with_an_input_imported_from_a_saved_program_runs_and_its_marks_point_at_the_import() {
    let lib = "def wob(a) {\n  every 1 bars: master: filter lp 200 + knob(3) * $a when held(key)\n}\ndef sidechain(x) {\n  every 2 bars: deck B: level 1 - $x\n}\n";
    let snips = snippets(&[("wobs", lib)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"wobs\"\nwob(7800)\nsidechain(fader(A))\n";
    let c = check_with(src, &env);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&c.diagnostics, src)));
    assert_eq!(comp.controls.len(), 2);
    // the line is the def's own line, found through the import: its marks point at the call (and the import line holds the def)
    assert!(comp.controls[0].via[0].text(&comp.source).starts_with("wob(7800)"), "{:?}", comp.controls[0].via);
    // it runs
    let mut live = Live::new();
    assert!(live.eval_with(src, &env).ok);
    let mut h = Fake::new();
    h.set("knob(3)", 0.5);
    h.set("held(key)", 0.25);
    h.set("fader(A)", 0.25);
    h.run(&mut live, 16.0);
    let want = literal_calls("every 1 bars: master: filter lp 4100\nevery 2 bars: deck B: level 0.75\n", 4.0);
    assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), want);
    // the canonical text pins the import by CID and is stable under reformatting
    assert!(comp.text.starts_with("import z"), "{}", comp.text);
    let again = check_with(&comp.text, &env).compiled.unwrap();
    assert_eq!(again.cid, comp.cid);
}

#[test]
fn the_calls_of_an_endless_program_stay_equal_between_a_run_and_its_replay_across_a_loop_of_sections() {
    let src = "tempo 120\nsection verse 2 bars {\n  deck A: kick x ~ x ~\n  every 1 bars: master: level fader(A)\n}\nsection chorus 2 bars {\n  deck A: kick x x x x\n  master: filter lp 200 + knob(1) * 3000\n}\narrange verse chorus\nloop\n";
    let (mut h, mut live) = started(src);
    let mut rng = Rng(77);
    for _ in 0..12 {
        h.set("fader(A)", rng.below(129) as f64 / 128.0);
        h.set("knob(1)", rng.below(129) as f64 / 128.0);
        h.run(&mut live, 4.0);
    }
    let perf = live.performance().unwrap();
    let mut rh = Fake::new();
    let mut r = Live::new();
    r.eval(src);
    r.replay(perf.clone()).unwrap();
    rh.run(&mut r, 12.0 * 4.0);
    assert_eq!(rh.all(), h.all());
    assert_eq!(r.performance().unwrap().cid(), perf.cid());
}

// ---- laws

#[test]
fn a_performed_line_is_the_literal_line_with_the_value_it_read() {
    // for every value a controller can say, `deck A: level knob(1)` makes the call `deck A: level v` makes
    for k in 0..=128 {
        let v = k as f64 / 128.0;
        let (mut h, mut live) = started("deck A: level knob(1)\nmaster: delay 1 - knob(1)\n");
        h.set("knob(1)", v);
        h.run(&mut live, 8.0);
        let literal = literal_calls(&format!("deck A: level {k} / 128\nmaster: delay 1 - {k} / 128\n"), 2.0);
        assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), literal, "knob(1) = {k}/128");
    }
}

#[test]
fn a_replay_equals_the_run_for_any_script_of_the_controllers() {
    let programs = [
        SHOW,
        "every 1 bars: deck A: level fader(A) -> fader(B) over 3 bars\nevery 4 bars: master: reverb knob(1) when pad(2) > 0\n",
        "at bar 2 beat 3: crossfader knob(4)\nevery 3 bars for 1 bars: master: filter hp 20 + held(key) * 7000\n",
        "section verse 3 bars {\n  every 1 bars: deck A: low knob(1)\n}\nsection chorus 2 bars {\n  master: filter lp 300 + knob(2) * 3000 -> 200 + knob(3) * 5000 over 2 bars\n}\narrange verse chorus verse\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        for seed in 1..=12u64 {
            let mut rng = Rng(seed * 0x9e37_79b9 + n as u64);
            let (mut h, mut live) = started(src);
            let inputs = ["knob(1)", "knob(2)", "knob(3)", "knob(4)", "pad(2)", "pad(5)", "fader(A)", "fader(B)", "held(key)", "held(c4)"];
            for _ in 0..10 {
                for i in inputs {
                    // some inputs are not connected some of the time
                    if rng.below(5) == 0 {
                        h.script.remove(i);
                    } else {
                        h.set(i, rng.below(129) as f64 / 128.0);
                    }
                }
                h.run(&mut live, 4.0 * (1 + rng.below(2)) as f64);
            }
            let Some(perf) = live.performance() else { continue };
            let mut rh = Fake::new();
            let mut r = Live::new();
            r.eval(src);
            r.replay(perf.clone()).unwrap();
            // the replay's host says something else: it must not matter
            rh.set("knob(1)", 0.77);
            rh.run(&mut r, h.beat - 8.0);
            assert_eq!(rh.all().iter().map(|c| (c.0, c.1.clone(), c.2.clone())).collect::<Vec<_>>(), h.all(), "program {n} seed {seed}");
            assert_eq!(r.performance().map(|p| p.cid()), Some(perf.cid()), "program {n} seed {seed}");
        }
    }
}

#[test]
fn a_performance_survives_its_json_text_and_keeps_its_cid() {
    let (mut h, mut live) = started(SHOW);
    perform(&mut h, &mut live, 10);
    let perf = live.performance().unwrap();
    for text in [perf.to_json().canonical_js()] {
        let back = Performance::from_json(&JsonValue::parse(&text).unwrap()).unwrap();
        assert_eq!(back, perf);
        assert_eq!(back.cid(), perf.cid());
    }
}

// ---- the rest of the language around it

#[test]
fn a_call_of_a_def_takes_no_performed_guard_and_a_part_takes_no_input_step() {
    let d = err_of("def wobble() {\n  master: level 0.5\n}\nwobble() when held(key)\n");
    assert!(d.message.contains("a call of a `def` takes no performed guard"), "{}", d.message);
    assert!(d.help.unwrap().contains("inside the def"));
    let d = err_of("deck A: bass pad(3)\n");
    assert!(d.message.contains("performed input"), "{}", d.message);
    let d = err_of("deck A: bass [c2 knob(1)]\n");
    assert!(d.message.contains("performed input"), "{}", d.message);
}

#[test]
fn the_words_of_an_input_are_highlighted_like_the_calls_and_words_round_them() {
    use djlive::highlight::{highlight, Class};
    let src = "master: filter lp 200 + knob(3) * 7800 when held(c4) and fader(A) > 0.5\n";
    let c: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
    assert_eq!(class_of("knob"), Some(Class::Generator));
    assert_eq!(class_of("held"), Some(Class::Generator));
    assert_eq!(class_of("fader"), Some(Class::Generator));
    assert_eq!(class_of("when"), Some(Class::Keyword));
    assert_eq!(class_of("A"), Some(Class::Deck));
    assert_eq!(class_of("c4"), Some(Class::Step));
    assert_eq!(class_of("and"), Some(Class::Keyword));
}

#[test]
fn a_ramp_that_names_an_input_shows_its_progress_and_the_line_that_fired_in_the_marks() {
    let (mut h, mut live) = started("master: level knob(1) -> 0 over 4 bars\n");
    h.set("knob(1)", 1.0);
    h.run(&mut live, 4.0 + 8.0);
    let m = live.marks();
    let c = &m.controls[0];
    assert!(c.span.text(&m.source).starts_with("master: level knob(1) -> 0 over 4 bars"), "{:?}", c.span.text(&m.source));
    let p = c.ramp.expect("a ramp in progress");
    assert!((p - 0.5).abs() < 0.1, "{p}");
    assert_eq!(live.status().get("controls").and_then(JsonValue::as_array).unwrap()[0].get("state").and_then(JsonValue::as_str), Some("ramping"));
}

#[test]
fn editing_a_performed_line_while_it_runs_keeps_the_others_and_starts_the_changed_one() {
    let (mut h, mut live) = started("every 1 bars: deck A: level knob(1)\nevery 1 bars: deck B: level knob(2)\n");
    h.set("knob(1)", 0.5);
    h.set("knob(2)", 0.25);
    h.run(&mut live, 8.0);
    let ev = live.eval("every 1 bars: deck A: level knob(1)\nevery 1 bars: deck B: level 1 - knob(2)\n");
    assert!(ev.ok);
    assert_eq!(ev.plan.as_ref().unwrap().controls_kept, 1);
    assert_eq!(ev.plan.as_ref().unwrap().controls_added.len(), 1, "{:?}", ev.plan);
    let before = h.calls.len();
    h.run(&mut live, 8.0);
    let new: Vec<String> = h.calls[before..].iter().filter(|c| c.2.contains(r#""deck":"B""#)).map(|c| c.2.clone()).collect();
    assert!(new.iter().all(|c| c == r#"{"deck":"B","volume":0.75}"#), "{new:?}");
    assert!(!new.is_empty());
    // the take is of the program that plays now: the old take is the last one, still there to save
    let perf = live.performance().unwrap();
    assert_eq!(perf.program, live.running_cid().unwrap());
}

#[test]
fn a_program_with_inputs_and_an_evolve_keeps_the_generations_it_had() {
    // the controls are not part of a generation: an input on a control line leaves every generation's CID alone
    let plain = "section drop 8 bars {\n  deck A: bass c2 e2 g2\n  evolve every 2 bars {\n    bass: transpose 2\n    stop when gen == 3\n  }\n}\narrange drop\n";
    let performed = format!("{plain}every 1 bars: master: level knob(1)\n");
    let (a, b) = (compiled(plain), compiled(&performed));
    for g in 0..=3 {
        assert_eq!(a.generation_cid("drop", g), b.generation_cid("drop", g), "generation {g}");
    }
    assert_eq!(a.lineage, b.lineage);
}

#[test]
fn a_note_is_spelled_once_in_an_input_and_the_example_plays() {
    assert_eq!(compiled("every 1 bars: master: level 0.5 when held(C#4)\n").text, "every 1 bars: master: level 0.5 when held(c#4)\n");
    assert_eq!(compiled("every 1 bars: master: level 0.5 when held(Db4)\n").text, "every 1 bars: master: level 0.5 when held(c#4)\n");
    // examples/btw/perform.btw, played: the gate opens the delay while a key is down, pad 5 takes deck A's low end for its bar
    let src = include_str!("../../../examples/btw/perform.btw");
    let (mut h, mut live) = started(src);
    h.set("knob(3)", 0.5);
    h.set("fader(A)", 0.25);
    h.set("knob(2)", 0.5);
    h.run(&mut live, 4.0 * 4.0);
    assert!(h.calls_of("mixer").iter().any(|c| c.1 == r#"{"deck":"B","volume":0.75}"#), "deck B rides deck A's fader: {:?}", h.calls_of("mixer"));
    assert!(h.calls_of("inserts").iter().all(|c| !c.1.contains("delay")), "no key is down: no delay");
    h.set("held(key)", 60.0 / 127.0);
    h.set("pad(5)", 1.0);
    h.run(&mut live, 4.0 * 4.0);
    assert!(h.calls_of("inserts").iter().any(|c| c.1.contains(r#""param":"delay.mix""#)), "a key is down: the delay opens");
    assert!(h.calls_of("mixer").iter().any(|c| c.1 == r#"{"deck":"A","low":0}"#), "pad 5: deck A's low end goes");
    assert!(live.performance().is_some());
}

#[test]
fn a_do_ramp_and_another_performed_param_on_one_line_and_a_list_item_that_cannot_be() {
    let reg = registry();
    let env = Env { registry: Some(&reg), ..Env::none() };
    let src = "do vocoder set bands=2 + knob(2) * 14 mix=knob(1) -> 1 over 4 bars\n";
    let checked = check_with(src, &env);
    let c = checked.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&checked.diagnostics, src)));
    assert_eq!(c.text, "do vocoder set bands=2 + knob(2) * 14 mix=knob(1) -> 1 over 4 bars\n");
    let mut live = Live::new();
    assert!(live.eval_with(src, &env).ok);
    let mut h = Fake::new();
    h.set("knob(1)", 0.5);
    h.set("knob(2)", 0.5);
    h.run(&mut live, 4.0 + 8.0);
    let sent: Vec<JsonValue> = h.calls_of("vocoder").iter().map(|c| JsonValue::parse(&c.1).unwrap()).collect();
    assert!(sent.len() > 3, "{sent:?}");
    // the ramp's start is the knob, its end 1, and the other performed param is in every send, rounded to a whole number
    assert!(sent.iter().all(|c| c.get("bands").and_then(JsonValue::as_f64) == Some(9.0)), "{sent:?}");
    let mixes: Vec<f64> = sent.iter().map(|c| c.get("mix").and_then(JsonValue::as_f64).unwrap()).collect();
    assert!((mixes[0] - 0.5).abs() < 0.01 && mixes.windows(2).all(|w| w[1] >= w[0] - 1e-9), "{mixes:?}");
    // an item of a list cannot be performed
    let bad = check_with("do vocoder set levels=[knob(1), 2]\n", &env);
    assert!(bad.compiled.is_none());
    assert!(bad.diagnostics.iter().any(|d| d.message.contains("a performed value cannot be one of its items")), "{:?}", bad.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
}
