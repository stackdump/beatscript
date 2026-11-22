//! F13 meets the other thirteen (docs/btw/ext-perform.md "Meeting the others"): the performed tier against F4's lfo and
//! shaped ramps, F8's mix and duck, F9's sing, F10's stacks, F11's ports, F1's sub-bar times, F3's tempo, F12's assertions
//! and F14's links. Each test is a live run on a fake clock with scripted controllers, or a program that must be refused
//! with a reason.

mod common;

use std::collections::BTreeMap;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, check_with, Compiled, Effect};
use djlive::diag::Severity;
use djlive::env::{ActionSpec, Env, NounSpec, ParamSpec, StaticRegistry, StaticSnippets};
use djlive::perform::{Input, Performance};
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


/// The `value` or `master` number of each call of a noun, in order.
fn numbers(h: &Fake, noun: &str, field: &str) -> Vec<(f64, f64)> {
    h.calls
        .iter()
        .filter(|c| c.1 == noun)
        .filter_map(|c| JsonValue::parse(&c.2).ok().and_then(|j| j.get(field).and_then(JsonValue::as_f64)).map(|v| (c.0, v)))
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

// ---- F4: shapes and lfo

#[test]
fn a_shaped_ramp_whose_end_is_a_knob_ends_where_the_knob_was_read() {
    let src = "master: level 0 -> knob(1) over 4 bars exp\n";
    let end_with = |k: f64| {
        let (mut h, mut live) = started(src);
        h.set("knob(1)", k);
        h.run(&mut live, 20.0);
        let n = numbers(&h, "mixer", "master");
        assert!(n.len() > 8, "{n:?}");
        n.last().unwrap().1
    };
    // the end is the knob, not the 1/2 the compiler checked with (the ramp used to be frozen at the placeholder)
    assert!(near(end_with(1.0), 1.0), "{}", end_with(1.0));
    assert!(near(end_with(0.25), 0.25), "{}", end_with(0.25));
    // an exponential shape is still an exponential shape: below the straight line half way
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 1.0);
    h.run(&mut live, 16.0);
    let at_half = numbers(&h, "mixer", "master").into_iter().find(|(beat, _)| near(*beat, 8.0 + 4.0 + 8.0)).map(|x| x.1).expect("the call two bars in");
    assert!(at_half < 0.3, "{at_half}");
    // and the line is performed (check says so)
    assert!(check(src).diagnostics.iter().any(|d| d.message.contains("performed: knob(1)")));
}

const LFO: &str = "master: filter lp lfo sine 2 bars center=2000 + knob(3) * 4000 depth=1000\n";

fn knob_of(hz: f64) -> f64 {
    djlive::compile::sweep_knob(hz, false)
}

#[test]
fn an_lfo_whose_centre_names_a_knob_swings_round_the_knob_and_follows_it_at_each_bar_line() {
    let c = compiled(LFO);
    assert_eq!(c.text, LFO);
    assert!(check(LFO).diagnostics.iter().any(|d| d.message.contains("performed: knob(3)")));
    let (mut h, mut live) = started(LFO);
    // bars 0 and 1: the knob is down (centre 2000, 1000..3000); then it goes all the way up (centre 6000, 5000..7000)
    h.set("knob(3)", 0.0);
    h.run(&mut live, 16.0);
    h.set("knob(3)", 1.0);
    h.run(&mut live, 32.0);
    let v = numbers(&h, "inserts", "value");
    let (first, later): (Vec<_>, Vec<_>) = v.into_iter().partition(|(beat, _)| *beat < 28.0);
    let (lo1, hi1) = (first.iter().map(|x| x.1).fold(f64::INFINITY, f64::min), first.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max));
    assert!(lo1 >= knob_of(1000.0) - 1e-3 && hi1 <= knob_of(3000.0) + 1e-3, "bars 0-1 swing 1000..3000 Hz: {lo1} {hi1}");
    assert!(hi1 - lo1 > 0.5 * (knob_of(3000.0) - knob_of(1000.0)), "and it does swing: {lo1} {hi1}");
    // from the bar the knob moved, the wave is round the new centre (a whole period later, to leave the bar it was read at)
    let after: Vec<f64> = later.iter().filter(|(beat, _)| *beat > 28.0).map(|x| x.1).collect();
    let (lo2, hi2) = (after.iter().copied().fold(f64::INFINITY, f64::min), after.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    assert!(lo2 >= knob_of(5000.0) - 1e-3 && hi2 <= knob_of(7000.0) + 1e-3, "then 5000..7000 Hz: {lo2} {hi2}");
    assert!(lo2 > hi1, "the swing moved up with the knob: {hi1} < {lo2}");
}

#[test]
fn a_performed_lfo_is_read_once_a_bar_and_replays_call_for_call() {
    let mut rng = Rng(0x1f0_0001);
    let (mut h, mut live) = started(LFO);
    for _ in 0..12 {
        h.set("knob(3)", (rng.below(129) as f64) / 128.0);
        h.run(&mut live, 1.0);
        // a wiggle inside the bar changes nothing until the next bar line
        h.set("knob(3)", (rng.below(129) as f64) / 128.0);
        h.run(&mut live, 3.0);
    }
    let live_calls = h.all();
    assert!(live_calls.len() > 100, "{}", live_calls.len());
    let perf = live.performance().expect("a performance");
    assert!(perf.inputs.iter().all(|r| r.input == "knob(3)") && perf.inputs.len() >= 11, "one reading a bar: {}", perf.inputs.len());
    let mut rh = Fake::new();
    rh.set("knob(3)", 0.123);
    let mut replay = Live::new();
    assert!(replay.eval(LFO).ok);
    replay.replay(perf.clone()).unwrap();
    rh.run(&mut replay, 12.0 * 4.0);
    assert_eq!(rh.all(), live_calls, "the replay of a performed lfo is the run, call for call");
    assert!(rh.asked.is_empty(), "{:?}", rh.asked);
    assert_eq!(replay.performance().unwrap().cid(), perf.cid());
}

#[test]
fn a_literal_lfo_is_what_it_was_and_a_guarded_lfo_starts_when_the_guard_is_true() {
    // no input: no performed line, no reading
    let lit = "master: filter lp lfo sine 2 bars center=3000 depth=1000\n";
    assert!(!matches!(compiled(lit).controls[0].effect, Effect::Performed(_)));
    let (mut h, mut live) = started(lit);
    h.run(&mut live, 8.0);
    assert!(h.asked.is_empty());
    // a guard only: the lfo is the literal one, started when due
    let g = "at bar 1: master: filter lp lfo sine 2 bars center=3000 depth=1000 when held(key)\n";
    let (mut h, mut live) = started(g);
    h.set("held(key)", 0.0);
    h.run(&mut live, 16.0);
    assert!(numbers(&h, "inserts", "value").is_empty(), "the guard was false at the bar the line was due");
    let (mut h, mut live) = started(g);
    h.set("held(key)", 0.5);
    h.run(&mut live, 16.0);
    assert!(numbers(&h, "inserts", "value").len() > 10);
}

#[test]
fn an_lfo_takes_a_performed_centre_and_depth_but_a_literal_phase_and_says_what_is_out_of_range() {
    let e = err_of("master: filter lp lfo sine 2 bars center=3000 phase=knob(1)\n");
    assert!(e.message.contains("literal phase"), "{}", e.message);
    // the swing can never be in the range of a level
    let e = err_of("master: level lfo sine 2 bars center=2 + knob(1) depth=0.1\n");
    assert!(e.message.contains("always outside the range of `level`"), "{}", e.message);
    // one that sometimes is: held to the range, said
    let c = check("master: level lfo sine 2 bars center=knob(1) depth=0.5\n");
    assert!(c.compiled.is_some());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("can swing from -0.5 to 1.5") && d.message.contains("held to 0 to 1")), "{:?}", c.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    // a performed depth is never negative: the wave is the same as its absolute value
    let (mut h, mut live) = started("master: level lfo sine 2 bars center=0.5 depth=knob(1) - 0.5\n");
    h.set("knob(1)", 0.0);
    h.run(&mut live, 8.0);
    assert!(numbers(&h, "mixer", "master").iter().all(|x| near(x.1, 0.5)), "depth held at 0: the level stays at its centre");
}

#[test]
fn a_do_lfo_takes_a_performed_centre_and_depth_too() {
    let src = "do vocoder set mix=lfo sine 2 bars center=0.5 depth=knob(2) * 0.4\n";
    let c = check_with(src, &Env { registry: Some(&registry()), ..Env::none() });
    assert!(c.compiled.is_some(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    let run = |k: f64| {
        let mut live = Live::new();
        assert!(live.eval_with(src, &Env { registry: Some(&registry()), ..Env::none() }).ok);
        let mut h = Fake::new();
        h.set("knob(2)", k);
        h.run(&mut live, 12.0);
        let v = numbers(&h, "vocoder", "mix");
        (v.iter().map(|x| x.1).fold(f64::INFINITY, f64::min), v.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max))
    };
    let (lo, hi) = run(1.0);
    assert!(lo >= 0.1 - 1e-3 && hi <= 0.9 + 1e-3 && hi - lo > 0.6, "{lo} {hi}");
    let (lo, hi) = run(0.0);
    assert!(near(lo, 0.5) && near(hi, 0.5), "no depth: the centre, {lo} {hi}");
}


// ---- F8: mix and duck

#[test]
fn a_mix_item_takes_no_input_and_the_error_names_the_control_line_to_write() {
    for (src, line) in [
        ("deck A: kick x\nmix { A level knob(1) }\n", "deck A: level knob(1)"),
        ("mix { master level fader(A) }\n", "master: level fader(A)"),
        ("mix { B eq low pad(2) }\n", "deck B: low pad(2)"),
        ("deck A: kick x\nmix { A level 0.5 + knob(1) / 2 }\n", "deck A: level knob(1)"),
        ("deck A: kick x\nmix { A level 0.5, B level held(key) }\n", "deck B: level held(key)"),
    ] {
        let e = err_of(src);
        assert!(e.message.starts_with("a mix item sets one number, once: it takes no performed input"), "{src}: {}", e.message);
        assert!(e.help.as_deref().unwrap_or("").contains(line), "{src}: {:?}", e.help);
        // the place is the call, not the line
        let at = &src[e.span.start..e.span.end];
        assert!(at.starts_with("knob") || at.starts_with("fader") || at.starts_with("pad") || at.starts_with("held"), "{src}: {at:?}");
    }
    // and what it says to write is a program
    let c = compiled("deck A: kick x\ndeck A: level knob(1)\n");
    assert!(matches!(c.controls[0].effect, Effect::Performed(_)));
}

const MIX_GUARDED: &str = "deck A: kick x ~ x ~\ndeck B: bass c2\nat bar 2: mix { A level 0.2, master level 0.6 } when held(key)\nat bar 3: duck B under A.kick 6db when held(c4)\nevery 4 bars for 2 bars: mix { B level 0.1 } when knob(1) > 0.5\n";

#[test]
fn a_when_after_a_mix_or_a_duck_is_read_while_the_program_plays() {
    // the guard used to be dropped without a word: a guarded mix fired whatever the keys did
    let c = compiled(MIX_GUARDED);
    assert_eq!(c.controls.len(), 3);
    assert!(c.controls.iter().all(|k| matches!(&k.effect, Effect::Performed(p) if p.guard.is_some())), "{:?}", c.controls.iter().map(|k| &k.effect).collect::<Vec<_>>());
    assert!(matches!(c.controls[0].effect.base(), Effect::Mix(_)) && matches!(c.controls[1].effect.base(), Effect::Duck(_)));
    // (its canonical text: a fixed point, as for any program)
    assert_eq!(compiled(&c.text).text, c.text);
    assert_eq!(compiled(&c.text).cid, c.cid);
    assert!(c.text.contains("} when held(key)") && c.text.contains("6db attack=5ms release=120ms when held(c4)") && c.text.contains("} when knob(1) > 0.5"), "{}", c.text);
    assert!(check(MIX_GUARDED).diagnostics.iter().any(|d| d.message.starts_with("performed: ") && ["held(c4)", "held(key)", "knob(1)"].iter().all(|i| d.message.contains(i))), "check says what it reads: {:?}", check(MIX_GUARDED).diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    let report = c.mix_report().join("\n");
    assert!(report.contains("only when `held(key)` is true at the bar it is due") && report.contains("only when `held(c4)` is true"), "{report}");

    // keys up, knob down: nothing fires, and the bars are said
    let (mut h, mut live) = started(MIX_GUARDED);
    h.set("knob(1)", 0.2);
    h.run(&mut live, 16.0);
    assert!(h.calls_of("mixer").is_empty() && h.calls_of("trick").is_empty(), "{:?}", h.all());
    assert!(live.words.iter().any(|w| w.contains("not run, its `when` is false") && w.contains("mix { A level 0.2")), "{:?}", live.words);

    // keys down, knob up: all three fire, the mixes at their bars
    let (mut h, mut live) = started(MIX_GUARDED);
    h.set("held(key)", 0.5);
    h.set("held(c4)", 1.0);
    h.set("knob(1)", 0.9);
    h.run(&mut live, 20.0);
    let mixer = h.calls_of("mixer");
    assert!(mixer.iter().any(|(_, p)| p.contains("\"volume\":0.2")), "{mixer:?}");
    assert!(mixer.iter().any(|(_, p)| p.contains("\"master\":0.6")), "{mixer:?}");
    assert!(mixer.iter().any(|(_, p)| p.contains("\"volume\":0.1")), "{mixer:?}");
    let trick = h.calls_of("trick");
    assert!(trick.iter().any(|(_, p)| p.contains("\"action\":\"on\"") && p.contains("\"kind\":\"pump\"")), "{trick:?}");

    // the same through a replay: the guards are logged and answered by bar
    let perf = live.performance().unwrap();
    let mut rh = Fake::new();
    let mut r = Live::new();
    assert!(r.eval(MIX_GUARDED).ok);
    r.replay(perf).unwrap();
    rh.run(&mut r, 20.0);
    assert_eq!(rh.all(), h.all());
}

#[test]
fn a_for_hold_on_a_guarded_mix_puts_back_only_what_it_set() {
    let src = "deck A: kick x ~ x ~\nat bar 1 for 2 bars: mix { A level 0.2 } when held(key)\n";
    // fired: the snapshot is read and put back two bars later
    let (mut h, mut live) = started(src);
    h.set("held(key)", 0.5);
    h.run(&mut live, 20.0);
    let levels: Vec<String> = h.calls_of("mixer").into_iter().map(|c| c.1).collect();
    assert!(levels.iter().any(|p| p.contains("0.2")), "{levels:?}");
    // not fired: nothing to put back, nothing said but the skipped line
    let (mut h, mut live) = started(src);
    h.run(&mut live, 20.0);
    assert!(h.calls_of("mixer").is_empty(), "{:?}", h.all());
    assert!(!live.words.iter().any(|w| w.contains("put back")), "{:?}", live.words);
}

// ---- F9: sing

#[test]
fn a_when_after_a_sing_line_decides_while_the_program_plays() {
    let src = "deck B: melody c4 e4 g4 e4\nat bar 1: deck B: sing \"la la\" on melody voice=robot when held(key)\n";
    let c = compiled(src);
    assert!(matches!(&c.controls[0].effect, Effect::Performed(p) if p.guard.is_some() && matches!(p.base, Effect::Calls(_))), "{:?}", c.controls[0].effect);
    let vocal = |h: &Fake| h.calls_of("vocal").into_iter().map(|c| c.1).collect::<Vec<_>>();
    let (mut h, mut live) = started(src);
    h.run(&mut live, 16.0);
    assert!(vocal(&h).is_empty(), "no key down: nobody sings: {:?}", h.all());
    let (mut h, mut live) = started(src);
    h.set("held(key)", 0.5);
    h.run(&mut live, 16.0);
    let v = vocal(&h);
    assert_eq!(v.len(), 2, "the line's two calls (create, to_row): {v:?}");
    assert!(v[0].contains("\"action\":\"create\"") && v[0].contains("la"), "{}", v[0]);
}

// ---- F11 and F2: ports, defs, repeats, imports; the glue of tokens that touch

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

/// `<=` and `>=` are two glued tokens: they have to stay glued where a def puts its argument in place and where an import gives
/// the def's tokens new places, and a guard with them is read like one with `<` and `>`.
const GATES: &str = "def gate(th, lvl) {\n  every 1 bars: deck B: level $lvl when held(key) and fader(A) <= $th or knob(2) >= 1 - $th\n}\ngate(0.5, knob(1))\nrepeat 2 every 1 bars {\n  at bar $i + 2: master: delay 0.2 when pad($i + 1) >= 0.5\n}\n";

#[test]
fn a_guard_with_less_or_equal_and_greater_or_equal_survives_a_def_a_repeat_and_reformatting() {
    let c = compiled(GATES);
    let keys: Vec<&str> = c.controls.iter().map(|k| k.key.as_str()).collect();
    // (a short decimal prints as itself and arithmetic that holds no input is the number it comes to: L-08)
    assert!(keys[0].contains("when held(key) and fader(A) <= 0.5 or knob(2) >= 0.5"), "{keys:?}");
    // (`$i` is put in place as written: the argument is read as arithmetic, so the logged input is `pad(1)` all the same)
    assert!(keys.iter().any(|k| k.contains("when pad(1) >= 0.5")) && keys.iter().any(|k| k.contains("when pad(2) >= 0.5")), "{keys:?}");
    stable(GATES, 0xf13_9a7e);

    // fader(A) <= 0.5 holds at exactly one half (the comparison is within a nano-unit), knob(2) >= 0.5 too
    let run = |held: f64, fader: f64, knob2: f64, pad1: f64| {
        let (mut h, mut live) = started(GATES);
        h.set("held(key)", held);
        h.set("fader(A)", fader);
        h.set("knob(2)", knob2);
        h.set("knob(1)", 0.75);
        h.set("pad(1)", pad1);
        h.run(&mut live, 12.0);
        (h.calls_of("mixer").len(), h.calls_of("mixer").iter().filter(|c| c.1.contains("volume")).count())
    };
    // the inputs read are the folded ones
    let (mut h, mut live) = started(GATES);
    h.run(&mut live, 12.0);
    let ids: Vec<String> = live.performed_inputs().iter().map(Input::id).collect();
    assert_eq!(ids, ["knob(1)", "knob(2)", "pad(1)", "pad(2)", "fader(A)", "held(key)"].map(String::from).to_vec().into_iter().filter(|i| ids.contains(i)).collect::<Vec<_>>());
    assert!(["pad(1)", "pad(2)", "fader(A)", "held(key)", "knob(1)", "knob(2)"].iter().all(|i| ids.iter().any(|x| x == i)), "{ids:?}");
    // key down and the fader at its half: the `and` side is true, every bar
    let (_, levels) = run(0.5, 0.5, 0.0, 0.0);
    assert!(levels >= 2, "{levels}");
    // key down but the fader above its half and knob 2 below its half: neither side
    let (_, levels) = run(0.5, 0.75, 0.25, 0.0);
    assert_eq!(levels, 0);
    // knob 2 at exactly its half: `>= 1 - 1/2` holds
    let (_, levels) = run(0.0, 0.0, 0.5, 0.0);
    assert!(levels >= 2, "{levels}");
}

#[test]
fn equal_and_not_equal_in_a_performed_guard_survive_a_def_and_reformatting() {
    let src = "def pick(th, lvl) {\n  every 1 bars: deck B: level $lvl when knob(1) == $th or pad(2) != held(c4) and knob(3) >= $th\n}\npick(0.5, fader(A))\npick(1 / 4, 0.9)\n";
    let c = compiled(src);
    assert_eq!(c.controls.len(), 2);
    stable(src, 0xf13_e9);
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.5);
    h.set("fader(A)", 0.75);
    h.run(&mut live, 8.0);
    // the first copy's `knob(1) == 1/2` holds; the second's `knob(1) == 1/4` does not, and pad(2) = held(c4) = 0 so `!=` fails too
    let levels: Vec<String> = h.calls_of("mixer").into_iter().map(|c| c.1).collect();
    assert!(!levels.is_empty() && levels.iter().all(|p| p.contains("\"volume\":0.75")), "{levels:?}");
}

#[test]
fn a_def_with_a_less_or_equal_guard_imported_from_a_saved_program_is_the_same_program() {
    let lib = "def gate(th, lvl) {\n  every 1 bars: deck B: level $lvl when fader(A) <= $th and knob(2) >= $th\n}\n";
    let snips = snippets(&[("gates", lib)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"gates\"\ngate(0.5, knob(1))\n";
    let c = check_with(src, &env);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&c.diagnostics, src)));
    assert!(comp.controls[0].key.contains("fader(A) <= 0.5 and knob(2) >= 0.5"), "{}", comp.controls[0].key);
    let again = check_with(&comp.text, &env).compiled.unwrap();
    assert_eq!(again.cid, comp.cid);
    // it runs: both sides true at one half
    let mut live = Live::new();
    assert!(live.eval_with(src, &env).ok);
    let mut h = Fake::new();
    h.set("fader(A)", 0.5);
    h.set("knob(2)", 0.5);
    h.set("knob(1)", 0.25);
    h.run(&mut live, 8.0);
    assert!(h.calls_of("mixer").iter().any(|c| c.1.contains("\"volume\":0.25")), "{:?}", h.all());
}

#[test]
fn a_port_carries_a_number_known_when_the_program_is_read_and_an_input_is_not() {
    for (src, port) in [
        ("def t() out: tempo {\n  emit 100 + knob(1) * 40\n}\ndef w() in: tempo {\n  deck A: kick x\n}\nwire t.out -> w.in\nw()\n", "tempo"),
        ("def t() out: bars {\n  emit 2 + fader(A) * 4\n}\ndef w() in: bars {\n  deck A: kick x\n}\nwire t.out -> w.bars\nw()\n", "bars"),
        ("def t() out: tempo {\n  emit min(knob(1), 1)\n}\ndef w() in: tempo {\n  deck A: kick x\n}\nwire t.out -> w.in\nw()\n", "tempo"),
    ] {
        let e = err_of(src);
        assert!(e.message.contains("is read while the program plays") && e.message.contains("fixed when the program is read"), "{port}: {}", e.message);
        assert!(e.help.as_deref().unwrap_or("").contains(port), "{port}: {:?}", e.help);
    }
    // a wired number and an input side by side in one control line is the way to have both
    let src = "def t() out: tempo {\n  emit 100\n}\ndef w() in: tempo {\n  deck A: kick x\n  every 1 bars: master: filter lp $in * 20 + knob(1) * 4000\n}\nwire t.out -> w.in\nw()\n";
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.5);
    h.run(&mut live, 8.0);
    let want = literal_calls("deck A: kick x\nevery 1 bars: master: filter lp 4000\n", 2.0);
    assert_eq!(h.calls.iter().map(|c| (c.1.clone(), c.2.clone())).collect::<Vec<_>>(), want);
}

// ---- F1: sub-bar times

const SUBBAR: &str = "deck A: kick x ~ x ~\nat bar 2 beat 2: master: level knob(1)\nevery 2 beats from bar 1: deck B: level fader(A) when held(key)\nat bar 3 beat 3: deck A: low knob(2) * 1 / 2\n";

fn play_sub_bar(h: &mut Fake, live: &mut Live, bars: usize, seed: u64) {
    let mut rng = Rng(seed);
    for bar in 0..bars {
        // the performer moves things at the top of a bar and again at the third beat, which is after some of the lines
        h.set("knob(1)", (rng.below(129) as f64) / 128.0);
        h.set("knob(2)", (rng.below(129) as f64) / 128.0);
        h.set("fader(A)", (rng.below(129) as f64) / 128.0);
        h.set("held(key)", if bar % 3 != 1 { 0.5 } else { 0.0 });
        h.run(live, 1.5);
        h.set("knob(1)", (rng.below(129) as f64) / 128.0);
        h.set("fader(A)", (rng.below(129) as f64) / 128.0);
        h.run(live, 2.5);
    }
}

#[test]
fn a_control_that_fires_between_bar_lines_reads_its_input_when_it_is_due_and_replays() {
    let (mut h, mut live) = started(SUBBAR);
    play_sub_bar(&mut h, &mut live, 8, 0xf13_5ab);
    let run_calls = h.all();
    assert!(run_calls.iter().any(|c| c.1 == "mixer" && c.2.contains("\"master\"")), "at bar 2 beat 2 fired: {run_calls:?}");
    let perf = live.performance().expect("a performance");
    // the value `at bar 2 beat 2` made is the one logged where it was read: bar 2, half way in (slot 32 of 64), quantised to 1/128
    let logged = perf.inputs.iter().find(|r| r.input == "knob(1)" && r.bar == 2).expect("knob(1) at bar 2");
    assert_eq!(logged.at, 32, "{logged:?}");
    let sent: Vec<f64> = numbers(&h, "mixer", "master").into_iter().map(|x| x.1).collect();
    assert_eq!(sent.len(), 1, "once");
    assert!(near(sent[0], logged.value), "{} vs logged {}", sent[0], logged.value);
    // a line that repeats inside the bar (`every 2 beats`) reads when each firing is due: the performer moved the fader at 1.5 beats,
    // so the firing at beat 2 reads what the firing at beat 0 did not (the old rule read the bar's first value both times)
    let faders: Vec<f64> = perf.inputs.iter().filter(|r| r.input == "fader(A)" && r.bar == 3).map(|r| r.value).collect();
    let places: Vec<u8> = perf.inputs.iter().filter(|r| r.input == "fader(A)" && r.bar == 3).map(|r| r.at).collect();
    assert_eq!(places, vec![0, 32], "two firings of the bar, each read where it was due: {faders:?}");
    // replayed by a fresh runtime on a host with other controllers: the same calls, in order, with the same numbers
    let mut rh = Fake::new();
    rh.set("knob(1)", 0.999);
    let mut r = Live::new();
    assert!(r.eval(SUBBAR).ok);
    r.replay(perf.clone()).unwrap();
    rh.run(&mut r, 8.0 * 4.0);
    assert_eq!(rh.all(), run_calls);
    assert!(rh.asked.is_empty());
    assert_eq!(r.performance().unwrap().cid(), perf.cid());
}

#[test]
fn the_replay_of_sub_bar_controls_equals_the_run_for_any_script() {
    let mut seeds = Rng(0x5eed_f13);
    for _ in 0..12 {
        let seed = seeds.below(1 << 30) + 1;
        let (mut h, mut live) = started(SUBBAR);
        play_sub_bar(&mut h, &mut live, 6, seed);
        let perf = live.performance().unwrap();
        let mut rh = Fake::new();
        let mut r = Live::new();
        assert!(r.eval(SUBBAR).ok);
        r.replay(Performance::from_json(&perf.to_json()).unwrap()).unwrap();
        rh.run(&mut r, 6.0 * 4.0);
        assert_eq!(rh.all(), h.all(), "seed {seed}");
    }
}

// ---- F10: stacks and sections

#[test]
fn performed_lines_in_stacked_sections_fire_once_each_where_their_section_is_and_replay() {
    let src = "tempo 120\nsection groove 4 bars {\n  deck A: kick x ~ x ~\n  every 1 bars: master: level knob(1) when held(key)\n}\nsection wash 2 bars {\n  deck B: bass c3 ~ c3 ~\n  every 1 bars: deck B: level fader(A)\n  at bar 1: master: filter lp 200 + knob(2) * 3000\n}\nsection drop 2 bars {\n  deck A: kick x x x x\n  master: reverb knob(3) -> 1 over 2 bars\n}\narrange then(stack(groove, wash), drop)\nloop\n";
    let c = compiled(src);
    // stacked, the groove's and the wash's lines are each one control; the drop's is its own
    assert_eq!(c.controls.iter().filter(|k| matches!(k.effect, Effect::Performed(_))).count(), 4, "{:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    let (mut h, mut live) = started(src);
    let mut rng = Rng(0xf10_f13);
    for bar in 0..14 {
        h.set("knob(1)", rng.below(129) as f64 / 128.0);
        h.set("knob(2)", rng.below(129) as f64 / 128.0);
        h.set("knob(3)", rng.below(129) as f64 / 128.0);
        h.set("fader(A)", rng.below(129) as f64 / 128.0);
        h.set("held(key)", if bar % 3 == 0 { 0.0 } else { 0.5 });
        h.run(&mut live, 4.0);
    }
    assert!(h.calls_of("mixer").len() > 6 && !h.calls_of("inserts").is_empty(), "{:?}", h.all());
    let perf = live.performance().unwrap();
    let mut rh = Fake::new();
    let mut r = Live::new();
    assert!(r.eval(src).ok);
    r.replay(perf.clone()).unwrap();
    rh.run(&mut r, 14.0 * 4.0);
    assert_eq!(rh.all(), h.all());
    assert_eq!(r.performance().unwrap().cid(), perf.cid());
    stable(src, 0xf13_f10);
}

#[test]
fn a_performed_line_in_a_section_with_a_key_and_a_tempo_is_still_one_program_and_marks_its_line() {
    let src = "section verse 2 bars {\n  key am\n  tempo 100\n  deck A: bass a2 ~ c3 ~\n  every 1 bars: master: filter hp 100 + knob(1) * 700\n}\nsection chorus 2 bars {\n  key dm\n  tempo 130\n  deck A: bass d2 ~ f2 ~\n  every 1 bars: master: filter hp 100 + knob(1) * 700 when pad(1)\n}\narrange verse chorus\nloop\n";
    let c = compiled(src);
    assert_eq!(c.controls.iter().filter(|k| matches!(k.effect, Effect::Performed(_))).count(), 2);
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.5);
    h.set("pad(1)", 1.0);
    h.run(&mut live, 20.0);
    assert!(!h.calls_of("inserts").is_empty());
    let marks = live.marks();
    assert!(marks.controls.iter().any(|m| m.span.text(&marks.source).starts_with("every 1 bars: master: filter hp")), "the lines that fired are marked");
    stable(src, 0xf13_5ec);
}

// ---- F3: tempo; F1: times; F12: assertions

#[test]
fn a_tempo_a_swing_and_every_time_word_refuse_an_input_and_say_what_was_wanted_there() {
    for (src, wanted) in [
        ("tempo knob(3)\n", "expected a tempo in BPM"),
        ("deck A: kick x\nat bar 4: tempo 120 + knob(1) * 20\n", "expected a tempo in BPM"),
        ("deck A: kick x\nat bar 2: tempo 120 -> 100 + knob(1) * 40 over 4 bars\n", "expected the tempo to move to"),
        ("deck A: kick x\nat bar 2: tempo 120 -> 140 over knob(1) bars\n", "expected a length"),
        ("deck A: kick x\nswing knob(1)\n", "expected a swing amount"),
        ("deck A: kick x\nat bar knob(1): tempo 120\n", "expected a bar number"),
        ("deck A: kick x\nevery knob(1) bars: master: delay 0.5\n", "expected a length"),
        ("deck A: kick x\nend at bar knob(1)\n", "expected a bar number"),
        ("deck A: kick x\nat bar 2 beat knob(1): master: delay 0.5\n", "expected"),
    ] {
        let e = err_of(src);
        assert!(e.message.starts_with(wanted) && e.message.contains("is a performed input: it is read while the program plays, so it cannot stand here"), "{src}: {}", e.message);
        assert!(e.help.as_deref().unwrap_or("").contains("drive a control line from the input instead"), "{src}: {:?}", e.help);
        // the place is the call
        let at = &src[e.span.start..e.span.end];
        assert!(at.starts_with("knob"), "{src}: {at:?}");
    }
}

#[test]
fn an_assertion_cannot_read_an_input_and_says_the_render_has_no_controllers() {
    for src in ["deck A: kick x\nassert level A < knob(1)\n", "deck A: kick x\nassert level A < 0.5 + fader(B) * 0.1\n", "deck A: kick x\nassert level master > min(held(key), 0.5)\n"] {
        let e = err_of(src);
        assert!(e.message.contains("is a performed input: an assertion is judged on an offline render, which has no controllers"), "{src}: {}", e.message);
        assert!(e.help.as_deref().unwrap_or("").contains("with every input at 1/2"), "{:?}", e.help);
    }
    // the program with a performed line and a literal assertion is fine, and check lists what it reads
    let src = "tempo 120\nsection verse 2 bars {\n  deck A: kick x ~ x ~\n}\narrange verse\nevery 1 bars: deck A: level knob(1)\nassert level A < 0.5\nassert ends\n";
    let c = check(src);
    assert!(c.compiled.is_some(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    assert!(c.diagnostics.iter().any(|d| d.message.starts_with("performed: knob(1)")));
    let canon = compiled(src);
    assert_eq!(canon.assertions.len(), 2);
    stable(src, 0xf13_f12);
}

#[test]
fn a_tempo_curve_and_a_performed_ramp_run_together_and_replay_alike() {
    let src = "tempo 100 -> 140 over 8 bars\ndeck A: kick x ~ x ~\nevery 1 bars: master: filter lp 200 + knob(1) * 3000\nat bar 2: master: level fader(A) -> 0 over 4 bars\n";
    let c = compiled(src);
    assert!(c.tempo_map().drives(), "the tempo curve drives the decks");
    let (mut h, mut live) = started(src);
    let mut rng = Rng(0x7e90);
    for _ in 0..10 {
        h.set("knob(1)", rng.below(129) as f64 / 128.0);
        h.set("fader(A)", rng.below(129) as f64 / 128.0);
        h.run(&mut live, 4.0);
    }
    assert!(h.calls_of("deck").iter().any(|c| c.1.contains("\"action\":\"tempo\"")), "the curve is sent: {:?}", h.calls_of("deck").len());
    let perf = live.performance().unwrap();
    let mut rh = Fake::new();
    let mut r = Live::new();
    assert!(r.eval(src).ok);
    r.replay(perf).unwrap();
    rh.run(&mut r, 10.0 * 4.0);
    assert_eq!(rh.all(), h.all(), "the tempo calls, the ramp's steps and the filter's values are the same in the replay");
}
