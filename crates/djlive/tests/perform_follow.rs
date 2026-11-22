//! Hardware controls (docs/btw/ext-perform.md, "Follow"): a control line that names an input follows it. Four rules on the
//! performed tier: **R1** an untimed value line follows its input on every bar, **R2** an input written alone covers a filter's
//! range, **R3** an input is read when a line is due (not once a bar), **R4** a knob that nothing answers for rests at 1/2. Each
//! test here fails on the code before them.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djlive::compile::check;
use djlive::perform::{Input, Performance, Reading};
use djlive::runtime::{Grid, Live, LiveHost};

/// A host with a clock and scripted controllers: the fake the other performed tests use, kept here so that this file stands alone.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat, noun, params) of every call the runtime made.
    calls: Vec<(f64, String, String)>,
    /// What each input says now, by id (`knob(3)`): a script changes these between passes.
    script: BTreeMap<String, f64>,
    /// How many times the runtime asked for each input.
    asked: BTreeMap<String, usize>,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[usize::from(deck == "B")]
    }

    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[usize::from(deck == "B")] = on;
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

    /// The `volume` of every `mixer` call, with the beat it was made on.
    fn volumes(&self) -> Vec<(f64, f64)> {
        self.calls
            .iter()
            .filter(|c| c.1 == "mixer")
            .filter_map(|c| JsonValue::parse(&c.2).ok().and_then(|j| j.get("volume").and_then(JsonValue::as_f64)).map(|v| (c.0, v)))
            .collect()
    }
}

fn started(src: &str) -> (Fake, Live) {
    let mut live = Live::new();
    let ev = live.eval(src);
    assert!(ev.ok, "{}", djlive::diag::render_all(&ev.diagnostics, src));
    (Fake::new(), live)
}

// ---- R3: an input is read when a line is due

#[test]
fn a_line_that_repeats_inside_the_bar_reads_its_knob_each_time_it_is_due() {
    // `every 1 beats` fires four times a bar; the knob moves between them
    let (mut h, mut live) = started("every 1 beats: deck A: level knob(1)\n");
    // (the program lands on the clock's bar 1, beat 12: the first firing is at the end of the first step of the loop)
    h.run(&mut live, 3.0);
    for (i, v) in [0.25, 0.5, 0.75, 1.0, 0.0, 0.5, 0.25, 0.75].into_iter().enumerate() {
        h.set("knob(1)", v);
        h.run(&mut live, 1.0);
        let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
        assert_eq!(sent.len(), i + 1, "one call a firing: {sent:?}");
        assert_eq!(*sent.last().unwrap(), v, "firing {i} reads the knob where it is then: {sent:?}");
    }
}

#[test]
fn a_reading_says_where_in_the_bar_it_was_made_and_the_replay_answers_there() {
    let (mut h, mut live) = started("every 2 beats: deck A: level knob(1)\n");
    for v in [0.25, 0.75, 0.5, 1.0] {
        h.set("knob(1)", v);
        h.run(&mut live, 2.0);
    }
    let perf = live.performance().expect("a take");
    let got: Vec<(i64, u8, f64)> = perf.inputs.iter().map(|r| (r.bar, r.at, r.value)).collect();
    // (the line is due at the half bars: program bars 1, 1 1/2, 2, 2 1/2 ...)
    assert!(got.iter().any(|g| g.1 == 32) && got.iter().any(|g| g.1 == 0), "readings at the bar line and half way in: {got:?}");
    // saved as JSON text and read back: `at` is there when it is not the bar line, and the same performance comes back
    let text = perf.to_json().canonical_js();
    assert!(text.contains("\"at\":32") && text.matches("\"at\"").count() == got.iter().filter(|g| g.1 != 0).count(), "{text}");
    let back = Performance::from_json(&JsonValue::parse(&text).unwrap()).unwrap();
    assert_eq!(back, perf);
    assert_eq!(back.cid(), perf.cid());
    // the replay answers each firing from the place it was read, whatever the controllers say now
    let run_calls = h.calls.clone();
    let mut rh = Fake::new();
    rh.set("knob(1)", 0.999);
    let mut r = Live::new();
    assert!(r.eval("every 2 beats: deck A: level knob(1)\n").ok);
    r.replay(back).unwrap();
    rh.run(&mut r, 8.0);
    assert_eq!(rh.calls, run_calls);
    assert!(rh.asked.is_empty());
}

#[test]
fn a_performance_made_before_a_bar_had_places_keeps_its_cid_and_replays_as_it_did() {
    // the readings of a take that has no `at`: one for the bar, read at the bar line
    let readings: Vec<Reading> = (1..=4).map(|bar| Reading { bar, at: 0, input: "knob(1)".into(), value: f64::from(bar as i32) / 8.0 }).collect();
    let old = Performance::new("zProg", readings);
    let text = old.to_json().canonical_js();
    assert!(!text.contains("\"at\""), "a reading of the bar line is saved as it always was: {text}");
    assert_eq!(text, r#"{"format":1,"inputs":[{"bar":1,"input":"knob(1)","value":0.125},{"bar":2,"input":"knob(1)","value":0.25},{"bar":3,"input":"knob(1)","value":0.375},{"bar":4,"input":"knob(1)","value":0.5}],"program":"zProg","type":"djrust.performance"}"#);
    assert_eq!(Performance::from_json(&JsonValue::parse(&text).unwrap()).unwrap(), old);

    // replayed against a line that fires four times a bar, it answers every firing of a bar with the bar's one value: the old rule
    let src = "every 1 beats: deck A: level knob(1)\n";
    let program = {
        let c = check(src).compiled.expect("compiles");
        c.cid
    };
    let perf = Performance::new(program, (1..=4).map(|bar| Reading { bar, at: 0, input: "knob(1)".into(), value: f64::from(bar as i32) / 8.0 }).collect());
    let mut live = Live::new();
    assert!(live.eval(src).ok);
    live.replay(perf).unwrap();
    let mut h = Fake::new();
    h.run(&mut live, 12.0);
    let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
    // (program bar 1 is clock bar 1: the beats 12 to 16 of the clock are its four firings)
    assert!(sent.windows(2).all(|w| w[0] <= w[1]), "{sent:?}");
    assert!(sent.iter().filter(|v| **v == 0.125).count() >= 3, "the bar's value, every firing of the bar: {sent:?}");
}

#[test]
fn a_bad_place_in_a_saved_performance_is_refused() {
    let ok = Performance::new("zProg", vec![Reading { bar: 1, at: 32, input: "knob(1)".into(), value: 0.25 }]).to_json();
    assert!(Performance::from_json(&ok).is_ok());
    let bad = |at: &str| JsonValue::parse(&ok.canonical_js().replace("\"at\":32", &format!("\"at\":{at}"))).unwrap();
    for at in ["64", "-1", "1.5", "\"x\""] {
        let e = Performance::from_json(&bad(at)).unwrap_err();
        assert!(e.contains("`at` is a whole number from 0 to 63"), "{at}: {e}");
    }
    assert_eq!(Input::parse_id("knob(1)"), Some(Input::Knob(1)));
}

// ---- R4: a knob that nothing answers for rests at 1/2

#[test]
fn a_knob_nothing_answers_for_reads_one_half_and_a_pad_reads_zero() {
    // (no controller: the host answers None. Before, the cutoff of this line started at 200 Hz, a dull start; now it is the middle)
    let (mut h, mut live) = started("every 1 bars: master: filter lp 200 + knob(3) * 7800\nevery 1 bars: deck B: level 0.5 + pad(2) * 0.5\n");
    h.run(&mut live, 8.0);
    let filter: Vec<String> = h.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("filter.sweep")).map(|c| c.2.clone()).collect();
    let want = {
        let mut l = Live::new();
        assert!(l.eval("master: filter lp 4100\n").ok);
        let mut r = Fake::new();
        r.run(&mut l, 8.0);
        r.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("filter.sweep")).map(|c| c.2.clone()).collect::<Vec<_>>()
    };
    assert_eq!(filter.first(), want.first(), "knob(3) unanswered is 1/2: 200 + 0.5 * 7800 = 4100 Hz");
    assert_eq!(h.volumes().first().map(|v| v.1), Some(0.5), "pad(2) unanswered is 0");
    assert!(live.words.iter().any(|w| w.contains("knob(3): no controller is connected for it: it reads 1/2")), "{:?}", live.words);
    assert!(live.words.iter().any(|w| w.contains("pad(2): no controller is connected for it: it reads 0")), "{:?}", live.words);
    // nothing was answered, so nothing is logged: a replay reads the same home
    assert!(live.performance().is_none());
}

#[test]
fn a_replay_that_has_no_value_for_a_knob_reads_its_home() {
    let src = "every 1 bars: deck A: level knob(1)\n";
    let program = check(src).compiled.expect("compiles").cid;
    // a take that read the knob at bar 1 only
    let perf = Performance::new(program, vec![Reading { bar: 1, at: 0, input: "knob(1)".into(), value: 0.25 }]);
    let mut live = Live::new();
    assert!(live.eval(src).ok);
    live.replay(perf).unwrap();
    let mut h = Fake::new();
    h.run(&mut live, 12.0);
    let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
    assert_eq!(sent, vec![0.25, 0.5, 0.5], "bar 1 as read, then the home of a knob the take has no value for");
    assert!(live.words.iter().any(|w| w.contains("knob(1): the performance has no value for it at bar 2: it reads 1/2")), "{:?}", live.words);
}

// ---- R2: an input written alone covers a filter's range

/// The `value` of the filter sweeps the calls say, in order.
fn sweeps(h: &Fake) -> Vec<f64> {
    h.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("filter.sweep")).filter_map(|c| JsonValue::parse(&c.2).ok().and_then(|j| j.get("value").and_then(JsonValue::as_f64))).collect()
}

#[test]
fn a_bare_input_as_a_cutoff_covers_the_filters_range_on_its_own_curve() {
    // the record's line: before, "this value is always outside the range of `filter`: it is 0 to 1 and the range is 60 to 20000"
    let c = check("master: filter lp knob(1)\nmaster: filter hp fader(A)\n");
    assert!(c.compiled.is_some(), "{}", djlive::diag::render_all(&c.diagnostics, ""));
    // low-pass: knob at 0 is the lowest cutoff (60 Hz), at 1 the highest (20 kHz); up is a higher cutoff, on the log curve
    let (mut h, mut live) = started("every 1 bars: master: filter lp knob(1)\n");
    for v in [0.0, 0.5, 1.0] {
        h.set("knob(1)", v);
        h.run(&mut live, 4.0);
    }
    let want = |hz: f64, hp: bool| djlive::compile::sweep_knob(hz, hp);
    let got = sweeps(&h);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0], want(60.0, false), "knob 0: 60 Hz");
    assert_eq!(got[2], want(20_000.0, false), "knob 1: 20 kHz");
    let middle = want((60.0_f64 * 20_000.0).sqrt(), false);
    assert!((got[1] - middle).abs() < 1e-5, "half way is the geometric middle of the range, {} vs {middle}", got[1]);
    assert!(got[0] < got[1] && got[1] < got[2], "up is a higher cutoff: {got:?}");
    // high-pass: 20 Hz to 8 kHz
    let (mut h, mut live) = started("every 1 bars: master: filter hp knob(2)\n");
    for v in [0.0, 1.0] {
        h.set("knob(2)", v);
        h.run(&mut live, 4.0);
    }
    let got = sweeps(&h);
    assert_eq!(got, vec![want(20.0, true), want(8000.0, true)]);
}

#[test]
fn arithmetic_on_a_cutoff_keeps_its_error_and_the_help_says_how_to_write_it() {
    let c = check("master: filter lp knob(1) * 2\n");
    let d = c.diagnostics.iter().find(|d| d.is_error()).expect("an error");
    assert!(d.message.contains("always outside the range of `filter`"), "{}", d.message);
    let help = d.help.clone().unwrap_or_default();
    assert!(help.contains("or write the input alone to cover the range"), "{help}");
    // a level has no such rule: the same error with the old help
    let d = check("master: level knob(1) + 2\n").diagnostics.into_iter().find(|d| d.is_error()).expect("an error");
    assert!(!d.help.clone().unwrap_or_default().contains("alone"), "{:?}", d.help);
}

#[test]
fn a_bare_input_ends_a_filter_ramp_too_and_prints_as_written() {
    let src = "master: filter lp 20000 -> knob(1) over 4 bars\n";
    let c = check(src).compiled.expect("compiles");
    assert_eq!(c.text, src);
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.0);
    h.run(&mut live, 24.0);
    // the ramp ends at the lowest cutoff the line may say: 60 Hz
    let last = sweeps(&h).last().copied().expect("the ramp sent");
    assert!((last - djlive::compile::sweep_knob(60.0, false)).abs() < 1e-5, "{last}");
}

// ---- R1: a value line with no time word follows its input

/// Run up to the clock's bar 1 (the first bar line of the program), then a bar for each value, with `input` set to it before the bar line.
fn follow(h: &mut Fake, live: &mut Live, input: &str, values: &[f64]) {
    h.run(live, 3.0);
    bars(h, live, input, values);
}

/// A bar for each value, with `input` set to it before the bar line.
fn bars(h: &mut Fake, live: &mut Live, input: &str, values: &[f64]) {
    for v in values {
        h.set(input, *v);
        h.run(live, 4.0);
    }
}

#[test]
fn an_untimed_value_line_follows_its_knob_and_sends_only_when_it_moved() {
    let (mut h, mut live) = started("deck A: level knob(1)\n");
    // (the line is due on bar 1, 2, 3 ...: the clock's bar lines after the program landed)
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.25, 0.75, 0.75, 0.75, 0.0]);
    let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
    assert_eq!(sent, vec![0.25, 0.75, 0.0], "one call for each time the value moved, none for a bar it did not: {sent:?}");
    // the words say once that it ran, not once for each turn of the knob
    assert_eq!(live.words.iter().filter(|w| w.starts_with("bar ") && w.contains("deck A: level knob(1)")).count(), 1, "{:?}", live.words);
}

#[test]
fn a_line_with_a_time_word_keeps_its_meaning() {
    // once: `at bar 2` fires on bar 2 and no more, whatever the knob does after
    let (mut h, mut live) = started("at bar 2: deck A: level knob(1)\n");
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.5, 0.75, 1.0]);
    assert_eq!(h.volumes().len(), 1, "{:?}", h.volumes());
    // every N bars: on its bars, and each time, moved or not
    let (mut h, mut live) = started("every 1 bars: deck A: level knob(1)\n");
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.25, 0.25, 0.25]);
    assert_eq!(h.volumes().len(), 4, "{:?}", h.volumes());
    // a line whose only input is a `when` has no value to follow: it is due once, as it always was
    let (mut h, mut live) = started("deck A: level 0.5 when knob(1) > 0.1\n");
    follow(&mut h, &mut live, "knob(1)", &[0.0, 0.5, 0.5, 0.5]);
    assert!(h.volumes().is_empty(), "due once, on the first bar, when the knob was 0");
}

#[test]
fn a_for_hold_is_one_shot_and_a_ramp_keeps_its_own_rule() {
    let (mut h, mut live) = started("for 3 bars: master: filter lp knob(1)\n");
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.5, 0.75, 1.0]);
    let sets = h.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("filter.sweep")).count();
    assert_eq!(sets, 1, "a `for N bars` line is put back after N bars, not followed: {:?}", h.calls);
    // a ramp re-reads its ends at every bar line, as it always did: one ramp, not a line that fires every bar
    let (mut h, mut live) = started("master: level fader(A) -> 0 over 4 bars\n");
    h.set("fader(A)", 1.0);
    h.run(&mut live, 16.0);
    assert!(h.volumes().is_empty() && h.calls.iter().any(|c| c.1 == "mixer"), "{:?}", h.calls);
}

#[test]
fn in_a_section_a_value_line_follows_each_bar_of_it_and_stops_with_it() {
    let src = "section verse 3 bars {\n  deck A: kick x ~ x ~\n  deck B: level knob(1)\n}\nsection drop 3 bars {\n  deck A: kick x x x x\n}\narrange verse drop\n";
    let (mut h, mut live) = started(src);
    // bar 0 of `verse` is clock bar 1: the knob moves on each of its three bars, and then on each of `drop`'s
    follow(&mut h, &mut live, "knob(1)", &[0.125, 0.25, 0.375, 0.5, 0.625, 0.75]);
    let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
    assert_eq!(sent, vec![0.125, 0.25, 0.375], "every bar of section verse, none of section drop: {sent:?}");
}

#[test]
fn a_def_body_follows_when_it_is_called_with_no_time_word_and_not_otherwise() {
    let def = "def wob(d) {\n  deck $d: level knob(1)\n}\n";
    // called with no time word: its lines have none either, and follow
    let (mut h, mut live) = started(&format!("{def}wob(A)\n"));
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.5, 0.75]);
    assert_eq!(h.volumes().len(), 3, "{:?}", h.volumes());
    // called `at bar 2`, the call's time word is the lines': once
    let (mut h, mut live) = started(&format!("{def}at bar 2: wob(A)\n"));
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.5, 0.75, 1.0]);
    assert_eq!(h.volumes().len(), 1, "{:?}", h.volumes());
    // a repeat's copies are timed by the repeat: each fires once, where it is
    let (mut h, mut live) = started("repeat 2 every 2 bars {\n  deck A: level knob(1)\n}\n");
    follow(&mut h, &mut live, "knob(1)", &[0.25, 0.5, 0.75, 1.0]);
    assert_eq!(h.volumes().len(), 2, "{:?}", h.volumes());
}

#[test]
fn a_guard_pauses_a_line_that_follows_and_the_words_say_so_once() {
    let (mut h, mut live) = started("deck A: level knob(1) when held(key)\n");
    h.run(&mut live, 3.0);
    // the key is down for two bars, up for two, down for one
    for (key, knob) in [(0.5, 0.25), (0.5, 0.5), (0.0, 0.75), (0.0, 1.0), (0.5, 0.125)] {
        h.set("held(key)", key);
        h.set("knob(1)", knob);
        h.run(&mut live, 4.0);
    }
    let sent: Vec<f64> = h.volumes().into_iter().map(|x| x.1).collect();
    assert_eq!(sent, vec![0.25, 0.5, 0.125], "while the key is up the line waits, and goes on where the knob is when it comes down: {sent:?}");
    assert_eq!(live.words.iter().filter(|w| w.contains("not run, its `when` is false")).count(), 1, "said when it stopped, not on each bar: {:?}", live.words);
}

#[test]
fn a_lap_of_a_looped_song_sends_a_line_that_follows_again() {
    let src = "section verse 2 bars {\n  deck A: kick x ~ x ~\n}\narrange verse\nloop\ndeck B: level knob(1)\n";
    let (mut h, mut live) = started(src);
    h.set("knob(1)", 0.5);
    h.run(&mut live, 3.0 + 4.0 * 6.0);
    let sent = h.volumes().len();
    assert!(sent >= 3, "the song went round three times with the knob still: it is sent again at the start of each lap: {sent}");
}

#[test]
fn a_run_that_follows_and_its_replay_make_the_same_calls() {
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n  deck B: level knob(2)\n}\nsection drop 4 bars {\n  deck A: kick x x x x\n}\narrange verse drop\nloop\nmaster: filter lp knob(1)\nevery 1/4 bar: master: reverb knob(3)\n";
    let (mut h, mut live) = started(src);
    let mut rng = 0x6600_f011u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % 129) as f64 / 128.0
    };
    for _ in 0..24 {
        h.set("knob(1)", next());
        h.set("knob(2)", next());
        h.set("knob(3)", next());
        h.run(&mut live, 2.0);
        h.set("knob(3)", next());
        h.run(&mut live, 2.0);
    }
    let perf = live.performance().expect("a take");
    let run_calls = h.calls.clone();
    assert!(run_calls.len() > 30, "{}", run_calls.len());
    let mut rh = Fake::new();
    rh.set("knob(1)", 0.999);
    let mut r = Live::new();
    assert!(r.eval(src).ok);
    r.replay(Performance::from_json(&JsonValue::parse(&perf.to_json().canonical_js()).unwrap()).unwrap()).unwrap();
    rh.run(&mut r, 24.0 * 4.0);
    assert_eq!(rh.calls, run_calls);
    assert!(rh.asked.is_empty());
    assert_eq!(r.performance().unwrap().cid(), perf.cid());
}

#[test]
fn check_says_a_value_is_read_when_its_line_is_due() {
    let c = check("master: filter lp knob(1)\n");
    let notes: Vec<&str> = c.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(notes.contains(&"performed: knob(1) (read when its line is due)"), "{notes:?}");
}
