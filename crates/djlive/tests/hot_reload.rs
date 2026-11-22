//! The hot reload on a fake host and a fake clock: a change lands on the bar and not before, an
//! error keeps the last good program, what did not change is not touched, `every` and ramps keep
//! their time through an edit.

use djengine::json::JsonValue;
use djlive::runtime::{Grid, Live, LiveHost};

/// Four beats a bar. The fake deck plays when told, and "installs" a net on the next bar line like
/// `Engine::load_prepared`.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, net cid, the bar it lands on)
    loads: Vec<(String, String, i64)>,
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
    fail_load: bool,
    pauses: Vec<String>,
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

    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        if self.fail_load {
            return Err("no such luck".into());
        }
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), djengine::cid::share_cid(env), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
        if !on {
            self.pauses.push(deck.into());
        }
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    /// Advance `beats` in sixteenth-bar steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        let steps = (beats * 4.0).round() as usize;
        for _ in 0..steps {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }
}

const BACKBEAT: &str = "deck A: kick x ~ x ~ | snare ~ x ~ x\n";

/// A host with deck A already playing something, on a bar line.
fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

#[test]
fn nothing_plays_so_the_program_starts_at_once() {
    let mut h = Fake::default();
    let mut live = Live::new();
    let ev = live.eval(BACKBEAT);
    assert!(ev.ok && ev.lands.is_none());
    h.beat = 0.0;
    live.service(&mut h);
    assert_eq!(h.loads.len(), 1);
    assert_eq!(h.loads[0].0, "A");
    assert!(h.playing[0], "the deck is started");
    assert!(live.is_running());
}

#[test]
fn an_evaluation_lands_on_the_next_bar_not_before() {
    let (mut h, mut live) = playing();
    // evaluate half way into bar 2 (beat 10)
    h.run(&mut live, 2.0);
    assert_eq!(h.beat, 10.0);
    let ev = live.eval(BACKBEAT);
    assert!(ev.ok);
    // the program's bars: the first service anchors at bar 0, so this one lands on bar 1
    live.service(&mut h);
    assert_eq!(h.loads.len(), 1, "applied in the first part of a bar");
    // the fake engine installs it on the next bar line, beat 12
    assert_eq!(h.loads[0].2, 3);
    assert_eq!(h.beat, 10.0);
}

#[test]
fn an_evaluation_in_the_last_beat_waits_for_the_next_bar_and_says_so() {
    let (mut h, mut live) = playing();
    h.run(&mut live, 3.5); // beat 11.5: frac 0.875
    let ev = live.eval(BACKBEAT);
    assert_eq!(ev.lands, Some(2), "the answer says which program bar: not the next, the one after");
    h.run(&mut live, 0.25);
    assert!(h.loads.is_empty(), "not in the last beat");
    h.run(&mut live, 0.5); // into bar 3
    assert_eq!(h.loads.len(), 1);
    assert_eq!(h.loads[0].2, 4, "installed on the bar after that: a bar later than asked");
    // and a program that runs: an edit in the last beat lands a bar later too
    h.run(&mut live, 11.25); // beat 23.5: the last beat of a bar
    let line = live.line();
    let ev = live.eval("deck A: kick x x x x\n");
    assert_eq!(ev.lands, Some(line + 2));
}

#[test]
fn a_bad_evaluation_keeps_the_last_good_program_and_shows_the_errors() {
    let (mut h, mut live) = playing();
    live.eval(BACKBEAT);
    h.run(&mut live, 1.0);
    assert_eq!(h.loads.len(), 1);
    let running = live.running_cid().unwrap().to_string();
    // a typo
    let ev = live.eval("deck A: kick x ~ q ~ | snare ~ x ~ x\n");
    assert!(!ev.ok);
    assert!(ev.diagnostics.iter().any(|d| d.is_error() && d.message.contains("`q`")));
    h.run(&mut live, 8.0);
    assert_eq!(h.loads.len(), 1, "nothing was applied");
    assert_eq!(live.running_cid(), Some(running.as_str()), "the last good program runs on");
    assert!(!live.has_staged());
    assert_eq!(live.errors.iter().filter(|d| d.is_error()).count(), 1);
    assert!(live.words.last().unwrap().contains("last good program plays on"));
    // a good one after it clears the errors
    live.eval("deck A: kick x ~ x ~ | snare ~ x ~ x vel=90\n");
    assert!(live.errors.iter().all(|d| !d.is_error()));
}

#[test]
fn a_bad_evaluation_does_not_unstage_a_good_one() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    h.run(&mut live, 3.5);
    live.eval(BACKBEAT); // staged, waiting for the next bar
    assert!(live.has_staged());
    live.eval("deck A: kick\n");
    assert!(live.has_staged(), "the good program is still going to land");
    h.run(&mut live, 1.0);
    assert_eq!(h.loads.len(), 1);
}

#[test]
fn what_did_not_change_is_not_touched() {
    let (mut h, mut live) = playing();
    live.eval("deck A: kick x ~ x ~\ndeck B: bass 1 ~ 3 ~\n");
    h.run(&mut live, 2.0);
    assert_eq!(h.loads.len(), 2);
    // edit only deck B
    let ev = live.eval("deck A: kick x ~ x ~\ndeck B: bass 1 ~ 5 ~\n");
    let plan = ev.plan.unwrap();
    assert_eq!(plan.lines(), vec!["deck B: bass changed".to_string()]);
    h.run(&mut live, 4.0);
    assert_eq!(h.loads.len(), 3, "one more load");
    assert_eq!(h.loads[2].0, "B");
    // the same text again is nothing
    let ev = live.eval("deck A: kick x ~ x ~\ndeck B: bass 1 ~ 5 ~  # same\n");
    assert!(ev.unchanged && ev.plan.unwrap().is_empty());
    h.run(&mut live, 4.0);
    assert_eq!(h.loads.len(), 3);
}

#[test]
fn removing_a_deck_line_pauses_that_deck_and_removing_a_part_reloads_the_net() {
    let (mut h, mut live) = playing();
    live.eval("deck A: kick x ~ x ~ | snare ~ x ~ x\ndeck B: bass 1 ~ 3 ~\n");
    h.run(&mut live, 2.0);
    assert!(h.playing[1]);
    live.eval("deck A: kick x ~ x ~\n");
    h.run(&mut live, 4.0);
    assert_eq!(h.pauses, vec!["B".to_string()]);
    assert!(!h.playing[1]);
    let last = h.loads.last().unwrap();
    assert_eq!(last.0, "A", "deck A got a net without the snare");
}

#[test]
fn an_alternation_goes_on_from_where_it_was() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    // three bars of melody, then an edit that keeps the alternation: the net starts at the phase
    live.eval("deck A: melody <c4 e4 g4>\n");
    h.run(&mut live, 1.0);
    let first = h.loads[0].1.clone();
    h.run(&mut live, 4.0 * 3.0);
    live.eval("deck A: melody <c4 e4 g4> | kick x ~ x ~\n");
    h.run(&mut live, 1.0);
    let second = h.loads[1].1.clone();
    assert_ne!(first, second);
    // phase = (land bar) mod 3: both loads were rotated to the bar they landed on
    let env0 = djlive::check("deck A: melody <c4 e4 g4>\n").compiled.unwrap().decks[&djlive::ast::Deck::A].envelope(1);
    assert_eq!(first, djengine::cid::share_cid(&env0), "landed on program bar 1: phase 1");
}

#[test]
fn every_n_bars_fires_on_program_bars_and_survives_an_edit() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    live.eval("deck A: kick x ~ x ~\nevery 4 bars: swap pairs with echo_out\n");
    h.run(&mut live, 4.0 * 9.0 + 0.5);
    // program bars 4 and 8 (the program started on bar 0 = the bar of the first service)
    let fires: Vec<f64> = h.calls_of("autodj").iter().filter(|c| c.1.contains("\"next\"")).map(|c| c.0).collect();
    assert_eq!(fires.len(), 2, "{fires:?}");
    assert_eq!(fires[1] - fires[0], 16.0, "four bars of four beats apart");
    // an edit elsewhere does not restart the schedule: the next fire is bar 12
    live.eval("deck A: kick x x x x\nevery 4 bars: swap pairs with echo_out\n");
    h.run(&mut live, 4.0 * 3.0);
    let n = h.calls_of("autodj").iter().filter(|c| c.1.contains("\"next\"")).count();
    assert_eq!(n, 3, "bar 12 fired once");
}

#[test]
fn a_plain_line_runs_once_on_the_first_bar_and_not_again_when_the_text_is_reevaluated() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    live.eval("deck A: kick x ~ x ~\nqueue play\n");
    h.run(&mut live, 1.0);
    assert!(h.calls_of("queue").is_empty(), "not until the bar line");
    h.run(&mut live, 4.0);
    assert_eq!(h.calls_of("queue").len(), 1);
    live.eval("deck A: kick x ~ x ~\nqueue play\n");
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("queue").len(), 1, "an unchanged line does not run again");
    // edited: it is a new line, it runs
    live.eval("deck A: kick x ~ x ~\nqueue play\nqueue next\n");
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("queue").len(), 2);
}

#[test]
fn at_bar_fires_once_on_that_bar() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    live.eval("deck A: kick x ~ x ~\nat bar 3: queue stop\n");
    h.run(&mut live, 4.0 * 6.0);
    let q = h.calls_of("queue");
    assert_eq!(q.len(), 1);
    // bar 3 begins 12 beats after bar 0 (beat 8): beat 20
    assert!((q[0].0 - 20.0).abs() <= 0.5, "{q:?}");
}

#[test]
fn a_ramp_moves_smoothly_inside_the_bars_and_ends_where_it_says() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    live.eval("master: filter lp 2k -> 8k over 4 bars\n");
    h.run(&mut live, 4.0 * 7.0);
    let sets: Vec<(f64, f64)> = h
        .calls_of("inserts")
        .iter()
        .filter(|c| c.1.contains("filter.sweep"))
        .map(|c| (c.0, JsonValue::parse(&c.1).unwrap().get("value").and_then(JsonValue::as_f64).unwrap()))
        .collect();
    assert!(sets.len() > 8, "many small steps inside four bars, not one per bar: {}", sets.len());
    assert!(sets.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-9), "low-pass 2k to 8k opens: the knob climbs to 0.5");
    let first = sets.first().unwrap();
    let last = sets.last().unwrap();
    assert!((first.1 - djlive::compile::sweep_knob(2000.0, false)).abs() < 0.01, "{first:?}");
    assert!((last.1 - djlive::compile::sweep_knob(8000.0, false)).abs() < 1e-6, "{last:?}");
    // it started on a bar line and took four bars
    assert!((last.0 - first.0 - 16.0).abs() <= 1.0, "{first:?} {last:?}");
    // and it did not go on
    let n = sets.len();
    h.run(&mut live, 8.0);
    assert_eq!(h.calls_of("inserts").iter().filter(|c| c.1.contains("filter.sweep")).count(), n);
    // the filter was switched on first
    assert!(h.calls_of("inserts")[0].1.contains("\"on\":true"));
}

#[test]
fn a_failed_load_is_said_and_the_rest_goes_on() {
    let (mut h, mut live) = playing();
    h.fail_load = true;
    live.eval("deck A: kick x ~ x ~\nqueue play\n");
    h.run(&mut live, 6.0);
    assert!(h.loads.is_empty());
    assert!(live.words.iter().any(|w| w.contains("deck A: no such luck")));
    assert_eq!(h.calls_of("queue").len(), 1);
}

#[test]
fn stop_ends_the_program_and_the_next_one_counts_from_bar_zero() {
    let (mut h, mut live) = playing();
    live.eval("deck A: kick x ~ x ~\nevery 2 bars: queue next\n");
    h.run(&mut live, 4.0 * 5.0);
    assert!(!h.calls_of("queue").is_empty());
    live.stop();
    assert!(!live.is_running());
    let n = h.calls_of("queue").len();
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.calls_of("queue").len(), n, "stopped lines do not fire");
    live.eval("deck A: snare ~ x ~ x\n");
    h.run(&mut live, 4.0);
    assert_eq!(live.line() <= 4, true, "a new program counts from its own bar 0: {}", live.line());
}

#[test]
fn check_says_what_would_change_without_staging() {
    let (mut h, mut live) = playing();
    live.eval(BACKBEAT);
    h.run(&mut live, 2.0);
    let ev = live.check("deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8\n");
    assert!(ev.ok);
    assert_eq!(ev.plan.unwrap().lines(), vec!["deck A: hihat added".to_string()]);
    assert!(!live.has_staged());
}

#[test]
fn status_reports_the_bar_the_phrase_and_the_staged_program() {
    let (mut h, mut live) = playing();
    live.service(&mut h);
    live.eval(BACKBEAT);
    h.run(&mut live, 4.0 * 9.0 + 1.0);
    let s = live.status();
    assert_eq!(s.get("running").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(s.get("bar").and_then(JsonValue::as_f64), Some(9.0));
    assert_eq!(s.get("phrase").and_then(JsonValue::as_f64), Some(2.0));
    assert_eq!(s.get("bar_in_phrase").and_then(JsonValue::as_f64), Some(2.0));
    assert!(s.get("cid").is_some() && s.get("nets").is_some());
}
