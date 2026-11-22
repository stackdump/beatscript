//! `btwapi::run::Run`: the Live runtime behind a command queue, driven on a fake clock.
//! A `Sim` stands in for the browser's engine (it reads the commands, keeps the decks' play flags and a
//! clock that runs while a deck plays, and tells the runtime where it is with the grid JSON the page sends);
//! a `Desk` is the same runtime on the desktop's kind of host (direct calls). The two must give the same
//! loads and calls: the queue loses nothing and reorders nothing.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djlive::runtime::{Grid, Live, LiveHost};
use btwapi::run::{GridIn, Run};

fn corpus(name: &str) -> String {
    let p = format!("{}/tests/corpus/{name}.btw", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn jget<'a>(v: &'a JsonValue, k: &str) -> &'a JsonValue {
    v.get(k).unwrap_or_else(|| panic!("no {k} in {}", v.js_stringified()))
}

fn jstr<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    jget(v, k).as_str().unwrap_or_else(|| panic!("{k} is not a string in {}", v.js_stringified()))
}

/// A command as the test reads it.
#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Load { deck: String, cid: String },
    Play { deck: String, on: bool },
    Sound { deck: String, role: String, option: String },
    Call { noun: String, params: String },
}

fn read(c: &JsonValue) -> Cmd {
    match jstr(c, "op") {
        "load_net" => Cmd::Load { deck: jstr(c, "deck").into(), cid: djengine::cid::share_cid(jget(c, "envelope")) },
        "play" => Cmd::Play { deck: jstr(c, "deck").into(), on: jget(c, "on").as_bool().unwrap() },
        "sound" => Cmd::Sound { deck: jstr(c, "deck").into(), role: jstr(c, "role").into(), option: jstr(c, "option").into() },
        "call" => Cmd::Call { noun: jstr(c, "noun").into(), params: jget(c, "params").canonical_js() },
        other => panic!("unknown command {other}"),
    }
}

/// The browser engine, as far as the runtime can tell: decks that play, one clock.
struct Sim {
    run: Run,
    /// The grid deck's beat (the transport starts at 0 when the first deck starts).
    beat: f64,
    playing: BTreeMap<String, bool>,
    /// (beat the command was issued at, command)
    log: Vec<(f64, Cmd)>,
    passes: usize,
}

impl Sim {
    fn new() -> Sim {
        Sim { run: Run::new(), beat: 0.0, playing: BTreeMap::new(), log: Vec::new(), passes: 0 }
    }

    fn any_playing(&self) -> bool {
        self.playing.values().any(|p| *p)
    }

    fn grid_json(&self) -> String {
        let Some(d) = ["A", "B"].into_iter().find(|d| self.playing.get(*d).copied().unwrap_or(false)) else { return "null".into() };
        let flags: Vec<String> = ["A", "B"].iter().map(|k| format!("\"{k}\":{}", self.playing.get(*k).copied().unwrap_or(false))).collect();
        format!("{{\"deck\":\"{d}\",\"beat\":{},\"bpm\":140,\"playing\":{{{}}}}}", self.beat, flags.join(","))
    }

    /// One service pass.
    fn pass(&mut self) -> Vec<Cmd> {
        let g = self.grid_json();
        let out = self.run.service_str(&g).expect("grid json");
        self.passes += 1;
        let cmds: Vec<Cmd> = JsonValue::parse(&out).unwrap().as_array().unwrap().iter().map(read).collect();
        for c in &cmds {
            if let Cmd::Play { deck, on } = c {
                if *on && !self.any_playing() {
                    self.beat = 0.0;
                }
                self.playing.insert(deck.clone(), *on);
            }
            self.log.push((self.beat, c.clone()));
        }
        cmds
    }

    /// Advance `beats`, a pass every sixteenth of a bar.
    fn run_beats(&mut self, beats: f64) {
        let steps = (beats * 4.0).round() as usize;
        for _ in 0..steps {
            if self.any_playing() {
                self.beat += 0.25;
            }
            self.pass();
        }
    }

    fn loads(&self) -> Vec<(f64, String, String)> {
        self.log.iter().filter_map(|(b, c)| if let Cmd::Load { deck, cid } = c { Some((*b, deck.clone(), cid.clone())) } else { None }).collect()
    }

    fn calls(&self, noun: &str) -> Vec<(f64, JsonValue)> {
        self.log
            .iter()
            .filter_map(|(b, c)| match c {
                Cmd::Call { noun: n, params } if n == noun => Some((*b, JsonValue::parse(params).unwrap())),
                _ => None,
            })
            .collect()
    }
}

/// The same runtime on the desktop's kind of host: calls are direct, the deck's play flags are its own.
#[derive(Default)]
struct Desk {
    beat: f64,
    playing: BTreeMap<String, bool>,
    log: Vec<(f64, Cmd)>,
}

impl LiveHost for Desk {
    fn grid(&self) -> Option<Grid> {
        ["A", "B"].into_iter().find(|d| self.playing.get(*d).copied().unwrap_or(false)).map(|d| Grid { deck: d.into(), beat: self.beat, bpm: 140.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing.get(deck).copied().unwrap_or(false)
    }
    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        self.log.push((self.beat, Cmd::Load { deck: deck.into(), cid: djengine::cid::share_cid(env) }));
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        if on && !self.playing.values().any(|p| *p) {
            self.beat = 0.0;
        }
        self.playing.insert(deck.into(), on);
        self.log.push((self.beat, Cmd::Play { deck: deck.into(), on }));
    }
    fn set_sound(&mut self, deck: &str, role: &str, option: &str) -> Result<(), String> {
        self.log.push((self.beat, Cmd::Sound { deck: deck.into(), role: role.into(), option: option.into() }));
        Ok(())
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.log.push((self.beat, Cmd::Call { noun: noun.into(), params: params.canonical_js() }));
        Ok(JsonValue::Null)
    }
}

fn bar_of(beat: f64) -> i64 {
    (beat / 4.0).floor() as i64
}

#[test]
fn a_song_starts_at_once_and_each_sections_nets_load_a_bar_ahead() {
    let src = corpus("hand-dubstep-140-v2");
    let mut s = Sim::new();
    let ev = s.run.eval(&src);
    assert_eq!(jget(&ev, "ok").as_bool(), Some(true), "{}", ev.js_stringified());
    assert_eq!(jget(&ev, "lands").as_f64(), None, "nothing plays: it starts at once");
    assert_eq!(jstr(&ev, "lands_at"), "once (nothing plays: the decks start)");
    // nothing is issued until a pass
    let first = s.pass();
    // intro: deck A (kick, hihat), deck B (harmony): the loads come with the plays, in order
    let kinds: Vec<String> = first.iter().map(|c| match c { Cmd::Load { deck, .. } => format!("load {deck}"), Cmd::Play { deck, on } => format!("play {deck} {on}"), Cmd::Sound { deck, role, .. } => format!("sound {deck} {role}"), Cmd::Call { noun, .. } => format!("call {noun}") }).collect();
    assert_eq!(&kinds[..2], ["load A", "play A true"], "{kinds:?}");
    assert!(kinds.contains(&"load B".to_string()) && kinds.contains(&"play B true".to_string()), "{kinds:?}");
    // the nets that were loaded are the nets eval named
    let nets = jget(&ev, "nets").as_object().unwrap();
    let loaded: Vec<String> = s.loads().into_iter().map(|l| l.2).collect();
    let named: Vec<&str> = nets.iter().map(|(_, v)| v.as_str().unwrap()).collect();
    for cid in &loaded {
        assert!(named.contains(&cid.as_str()), "{cid} is not one of {named:?}");
    }
    // run the intro and a bit: the build section (bar 8 of the program) loads in the bar before it
    s.run_beats(4.0 * 7.0 + 2.0);
    let build_loads: Vec<_> = s.loads().into_iter().filter(|(b, _, _)| *b > 0.0).collect();
    assert!(!build_loads.is_empty(), "the next section's nets were loaded");
    for (b, deck, cid) in &build_loads {
        assert_eq!(bar_of(*b), 7, "{deck} {cid}: the build's nets load in bar 7 (a bar ahead of bar 8), not at {b}");
        assert!(named.contains(&cid.as_str()), "{cid} is a net of the program");
    }
    // a section later: the drop (bar 16) loads in bar 15 and not before
    s.run_beats(4.0 * 8.0);
    let drop_loads: Vec<_> = s.loads().into_iter().filter(|(b, _, _)| bar_of(*b) >= 8).collect();
    assert!(!drop_loads.is_empty());
    assert!(drop_loads.iter().all(|(b, _, _)| bar_of(*b) == 15), "{drop_loads:?}");
}

#[test]
fn the_queue_gives_the_same_loads_and_calls_as_the_desktop_host() {
    for name in ["hand-dubstep-140-v2", "hand-dubstep-140-v4", "matrix-130", "trance-138", "markers-demo"] {
        let src = corpus(name);
        let mut s = Sim::new();
        let ev = s.run.eval(&src);
        assert_eq!(jget(&ev, "ok").as_bool(), Some(true), "{name}: {}", ev.js_stringified());
        let mut live = Live::new();
        let mut d = Desk::default();
        assert!(live.eval_with(&src, &btwapi::web_env().env()).ok);
        // 40 bars is enough to cross a few sections of each
        for _ in 0..(40 * 16) {
            if s.any_playing() {
                s.beat += 0.25;
            }
            s.pass();
            if d.playing.values().any(|p| *p) {
                d.beat += 0.25;
            }
            live.service(&mut d);
        }
        assert!(s.log.len() > 20, "{name}: only {} commands", s.log.len());
        let (a, b): (Vec<_>, Vec<_>) = (s.log.iter().map(|(b, c)| (*b, c.clone())).collect(), d.log.clone());
        assert_eq!(a.len(), b.len(), "{name}: {} vs {} commands", a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(x, y, "{name}: command {i}");
        }
    }
}

#[test]
fn a_ramp_reports_its_progress_and_moves_the_filter() {
    let src = corpus("hand-dubstep-140-v2");
    let mut s = Sim::new();
    assert!(jget(&s.run.eval(&src), "ok").as_bool().unwrap());
    // the intro: master: filter lp 400 -> 6000 over 8 bars; run to the middle of it
    s.run_beats(4.0 * 4.0);
    let marks = JsonValue::parse(&s.run.marks_str()).unwrap();
    let ramp = jget(&marks, "controls")
        .as_array()
        .unwrap()
        .iter()
        .find(|c| jstr(c, "line").contains("filter lp 400"))
        .unwrap_or_else(|| panic!("the ramp is in the marks: {}", marks.js_stringified()));
    let p = jget(ramp, "progress").as_f64().expect("a ramp's progress");
    assert!((p - 0.5).abs() < 0.08, "half way through the intro's ramp: {p}");
    // the sweep goes out as inserts calls; an opening low-pass moves the knob up toward 0.5 (open)
    let sweeps: Vec<f64> = s
        .calls("inserts")
        .iter()
        .filter(|(_, j)| j.get("param").and_then(JsonValue::as_str) == Some("filter.sweep") && j.get("target").and_then(JsonValue::as_str) == Some("Master"))
        .map(|(_, j)| jget(j, "value").as_f64().unwrap())
        .collect();
    assert!(sweeps.len() > 10, "the ramp sends its value as it moves: {} calls", sweeps.len());
    assert!(sweeps.windows(2).all(|w| w[1] >= w[0] - 1e-9), "an opening low-pass moves the knob one way: {sweeps:?}");
    assert!(sweeps[0] < 0.2 && *sweeps.last().unwrap() > sweeps[0] + 0.05, "{sweeps:?}");
    // and the filter was switched on before the first value
    let on = s.calls("inserts");
    assert_eq!(jget(&on[0].1, "action").as_str(), Some("on"));
    assert_eq!(jget(&on[0].1, "effect").as_str(), Some("filter"));
}

#[test]
fn fires_show_in_the_marks_with_spans_that_slice_the_source() {
    let src = corpus("markers-demo");
    let mut s = Sim::new();
    assert!(jget(&s.run.eval(&src), "ok").as_bool().unwrap());
    s.run_beats(4.0 * 2.0);
    let marks = JsonValue::parse(&s.run.marks_str()).unwrap();
    assert_eq!(jget(&marks, "running").as_bool(), Some(true));
    let fires = jget(&marks, "fires").as_array().unwrap();
    assert!(!fires.is_empty(), "something fired in two bars: {}", marks.js_stringified());
    let units: Vec<u16> = src.encode_utf16().collect();
    for f in fires {
        let sp = &jget(f, "spans").as_array().unwrap()[0];
        let (a, b) = (jget(sp, "start16").as_f64().unwrap() as usize, jget(sp, "end16").as_f64().unwrap() as usize);
        let tok = String::from_utf16(&units[a..b]).unwrap();
        // the token that sounded: a hit `x` or a note of a part
        assert!(tok == "x" || tok.chars().next().is_some_and(|c| ('a'..='g').contains(&c)), "{tok:?} ({})", f.js_stringified());
        // utf-8 and utf-16 agree on an ASCII text, and the line is the line it is on
        assert_eq!(jget(sp, "start").as_f64(), jget(sp, "start16").as_f64());
        let line = jget(sp, "line").as_f64().unwrap() as usize;
        assert!(src.lines().nth(line - 1).unwrap().contains(&tok));
    }
    // the playing lines and the header are lit as spans too
    assert!(!jget(&marks, "active_lines").as_array().unwrap().is_empty());
    assert!(jget(&marks, "header").get("span").is_some());
}

#[test]
fn spans_are_in_utf16_when_the_text_has_wide_characters() {
    // a comment with an astral character and an accent before the part: byte offsets are not UTF-16 offsets
    let src = "# caf\u{e9} \u{1F3B5} first\ndeck A: kick x ~ x ~ | snare ~ x ~ x\n";
    let mut s = Sim::new();
    let ev = s.run.eval(src);
    assert!(jget(&ev, "ok").as_bool().unwrap(), "{}", ev.js_stringified());
    s.run_beats(4.0 * 2.0);
    let marks = JsonValue::parse(&s.run.marks_str()).unwrap();
    let fires = jget(&marks, "fires").as_array().unwrap();
    assert!(!fires.is_empty());
    let units: Vec<u16> = src.encode_utf16().collect();
    for f in fires {
        let sp = &jget(f, "spans").as_array().unwrap()[0];
        let (b0, b1) = (jget(sp, "start").as_f64().unwrap() as usize, jget(sp, "end").as_f64().unwrap() as usize);
        let (a, b) = (jget(sp, "start16").as_f64().unwrap() as usize, jget(sp, "end16").as_f64().unwrap() as usize);
        assert!(a < b0, "the wide characters make the byte offset bigger ({a} vs {b0})");
        assert_eq!(&src[b0..b1], "x");
        assert_eq!(String::from_utf16(&units[a..b]).unwrap(), "x");
    }
}

#[test]
fn an_evolving_section_flashes_when_a_generation_begins() {
    let src = corpus("markers-demo");
    let mut s = Sim::new();
    assert!(jget(&s.run.eval(&src), "ok").as_bool().unwrap());
    // build 8 bars, then the drop: evolve every 4 bars. Run to a little after generation 1 begins (program bar 12)
    s.run_beats(4.0 * 8.0 + 1.0);
    let at_drop = JsonValue::parse(&s.run.marks_str()).unwrap();
    assert_eq!(jget(&at_drop, "generation").as_f64(), Some(0.0), "{}", at_drop.js_stringified());
    let mut flashed = None;
    for _ in 0..(4 * 4 * 14) {
        s.run_beats(0.25);
        let m = JsonValue::parse(&s.run.marks_str()).unwrap();
        if jget(&m, "generation").as_f64() == Some(1.0) && jget(&m, "generation_flash_beats").as_f64().is_some() && flashed.is_none() {
            flashed = Some(m);
        }
    }
    let m = flashed.expect("generation 1 flashed");
    assert!(jget(&m, "generation_flash_beats").as_f64().unwrap() < 2.0);
    let spans = jget(&m, "evolve_spans").as_array().unwrap();
    assert!(spans.iter().any(|sp| {
        let (a, b) = (jget(sp, "start16").as_f64().unwrap() as usize, jget(sp, "end16").as_f64().unwrap() as usize);
        let units: Vec<u16> = src.encode_utf16().collect();
        String::from_utf16(&units[a..b]).unwrap().starts_with("evolve every 4 bars")
    }));
    // each generation of the drop is its own net, loaded on its bar line
    let loads = s.loads();
    let distinct: std::collections::BTreeSet<&String> = loads.iter().filter(|(_, d, _)| d == "B").map(|(_, _, c)| c).collect();
    assert!(distinct.len() >= 3, "deck B's melody changes with each generation: {} nets", distinct.len());
}

#[test]
fn stop_puts_back_what_a_line_still_holds_and_nothing_plays_on_from_the_queue() {
    let mut s = Sim::new();
    let src = "deck A: kick x ~ x ~\nat bar 2: move echo_out on B over 4 bars\n";
    let ev = s.run.eval(src);
    assert!(jget(&ev, "ok").as_bool().unwrap(), "{}", ev.js_stringified());
    s.run_beats(4.0 * 4.0);
    assert_eq!(s.calls("moves").len(), 1);
    assert!(s.calls("mixer").is_empty());
    let cmds: Vec<Cmd> = JsonValue::parse(&s.run.stop_str()).unwrap().as_array().unwrap().iter().map(read).collect();
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    match &cmds[0] {
        Cmd::Call { noun, params } => {
            assert_eq!(noun, "mixer");
            assert!(params.contains("\"mute\":false") && params.contains("\"deck\":\"B\""), "{params}");
        }
        c => panic!("{c:?}"),
    }
    // stopped: no more commands
    s.run_beats(4.0 * 4.0);
    assert_eq!(s.calls("moves").len(), 1);
    assert!(jget(&JsonValue::parse(&s.run.status_str()).unwrap(), "running").as_bool() == Some(false));
}

#[test]
fn a_program_with_an_error_changes_nothing_and_says_where() {
    let mut s = Sim::new();
    assert!(jget(&s.run.eval("deck A: kick x ~ x ~\n"), "ok").as_bool().unwrap());
    s.run_beats(8.0);
    let cid = s.run.running_cid().unwrap();
    let bad = "# caf\u{e9}\ndeck A: kick x ~ x ~ | snare ~ y ~ x\n";
    let ev = s.run.eval(bad);
    assert_eq!(jget(&ev, "ok").as_bool(), Some(false));
    assert!(jget(&ev, "errors").as_f64().unwrap() >= 1.0);
    let d = &jget(&ev, "diagnostics").as_array().unwrap()[0];
    assert_eq!(jget(d, "line").as_f64(), Some(2.0));
    let units: Vec<u16> = bad.encode_utf16().collect();
    let (a, b) = (jget(d, "start16").as_f64().unwrap() as usize, jget(d, "end16").as_f64().unwrap() as usize);
    assert!(b > a && b <= units.len());
    assert!(jstr(d, "render").contains("-->"), "the caret form is under `render`");
    assert!(jstr(&ev, "kept").contains(&cid), "{}", jstr(&ev, "kept"));
    assert_eq!(s.run.running_cid().as_deref(), Some(cid.as_str()));
    // and the runtime goes on
    s.run_beats(4.0);
    assert_eq!(s.run.running_cid().as_deref(), Some(cid.as_str()));
}

#[test]
fn an_edit_lands_on_the_next_bar_and_what_did_not_change_is_not_loaded_again() {
    let mut s = Sim::new();
    assert!(jget(&s.run.eval("deck A: kick x ~ x ~\ndeck B: hihat x*8\n"), "ok").as_bool().unwrap());
    s.run_beats(4.0 * 2.0 + 1.0);
    let before = s.loads().len();
    assert_eq!(before, 2);
    // change deck A only, a beat into bar 2: it lands on bar 3
    let ev = s.run.eval("deck A: kick x ~ x x\ndeck B: hihat x*8\n");
    assert_eq!(jget(&ev, "lands").as_f64(), Some(3.0), "{}", ev.js_stringified());
    assert_eq!(jstr(&ev, "lands_at"), "the next bar");
    let changes: Vec<&str> = jget(&ev, "changes").as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert!(changes.iter().any(|c| c.contains("deck A")), "{changes:?}");
    s.run_beats(1.0);
    let after: Vec<_> = s.loads().into_iter().skip(before).collect();
    assert_eq!(after.len(), 1, "only the deck that changed is loaded: {after:?}");
    assert_eq!(after[0].1, "A");
    // sent in the first three quarters of the bar it was evaluated in, so the engine has the rest of it to take the net in
    assert_eq!(bar_of(after[0].0), 2);
}

#[test]
fn the_page_gives_a_grid_as_json_and_a_bad_one_is_an_error_not_a_panic() {
    let mut r = Run::new();
    assert_eq!(r.service_str("null").unwrap(), "[]");
    assert_eq!(r.service_str("").unwrap(), "[]");
    assert_eq!(r.service_str("undefined").unwrap(), "[]");
    assert!(r.service_str("{nope").is_err());
    // a grid with no beat is no clock, not an error
    assert_eq!(r.service_str("{\"deck\":\"A\"}").unwrap(), "[]");
    let g = GridIn::parse("{\"deck\":\"B\",\"beat\":12.5,\"bpm\":140,\"playing\":{\"A\":false,\"B\":true}}").unwrap().unwrap();
    assert_eq!((g.deck.as_str(), g.beat, g.bpm), ("B", 12.5, 140.0));
    assert_eq!(g.playing.get("B"), Some(&true));
    // nothing evaluated: the marks and the status still answer
    assert_eq!(JsonValue::parse(&r.marks_str()).unwrap().get("running").and_then(JsonValue::as_bool), Some(false));
    assert!(JsonValue::parse(&r.status_str()).is_ok());
    assert_eq!(r.running_cid(), None);
}

#[test]
fn virtual_mode_follows_a_clock_the_page_already_has_and_discards_the_commands() {
    // a streamed render: the page has a beat for the stream and no deck of its own; it echoes `play` into its flags
    let src = corpus("hand-dubstep-140-v2");
    let mut r = Run::new();
    assert!(jget(&r.eval(&src), "ok").as_bool().unwrap());
    let mut flags: BTreeMap<String, bool> = BTreeMap::new();
    let mut beat = 0.0;
    let mut sections = Vec::new();
    for i in 0..(4 * 4 * 20) {
        let playing = if flags.values().any(|p| *p) { "true" } else { "false" };
        if i > 0 {
            beat += 0.25;
        }
        // the stream plays from the first beat: deck A "plays" as soon as the page says so
        let g = format!("{{\"deck\":\"A\",\"beat\":{beat},\"bpm\":140,\"playing\":{{\"A\":{playing},\"B\":{playing}}}}}");
        let cmds = JsonValue::parse(&r.service_str(&g).unwrap()).unwrap();
        for c in cmds.as_array().unwrap() {
            if jstr(c, "op") == "play" {
                flags.insert(jstr(c, "deck").to_string(), jget(c, "on").as_bool().unwrap());
            }
        }
        let st = JsonValue::parse(&r.status_str()).unwrap();
        if let Some(n) = st.get("section").and_then(JsonValue::as_str) {
            if sections.last().map(String::as_str) != Some(n) {
                sections.push(n.to_string());
            }
        }
    }
    // twenty bars: intro (8), build (8), into the drop
    assert_eq!(&sections[..2], ["intro", "build"], "{sections:?}");
}

/// The riser of happy-wub-172 and rave-duel-138: a high-pass and a reverb ramp that end on the very bar line the next section
/// starts on, where that section says `filter off` and a reverb of its own. The section's lines are the last word: after its
/// first bar line nothing of the ramps may follow them (the page dedupes values, so a ramp's last value sent after `filter off`
/// held the whole section high-passed and wet, and it played very quietly).
#[test]
fn a_sections_filter_off_and_reverb_win_over_the_ramps_that_end_on_its_first_bar() {
    let src = "tempo 140\n\
        section build 4 bars {\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n  master: filter hp 100 -> 4000 over 4 bars\n  master: reverb 0.1 -> 0.45 over 4 bars\n  before drop 2 bars: move loop_roll on A over 2 bars\n}\n\
        section drop 4 bars {\n  master: filter off\n  master: reverb 0.15\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n}\n\
        arrange build drop\n";
    let mut s = Sim::new();
    let ev = s.run.eval(src);
    assert!(jget(&ev, "ok").as_bool().unwrap(), "{}", ev.js_stringified());
    s.run_beats(4.0 * 7.0);
    let master: Vec<(f64, JsonValue)> = s.calls("inserts").into_iter().filter(|(_, j)| j.get("target").and_then(JsonValue::as_str) == Some("Master")).collect();
    let off = master
        .iter()
        .position(|(_, j)| jget(j, "action").as_str() == Some("on") && jget(j, "effect").as_str() == Some("filter") && jget(j, "on").as_bool() == Some(false))
        .unwrap_or_else(|| panic!("the drop switches the filter off: {master:?}"));
    let after: Vec<&(f64, JsonValue)> = master[off..].iter().collect();
    assert!(
        !after.iter().any(|(_, j)| j.get("param").and_then(JsonValue::as_str) == Some("filter.sweep")),
        "no sweep after the drop's `filter off`: {after:?}"
    );
    let last_mix = master.iter().rev().find(|(_, j)| j.get("param").and_then(JsonValue::as_str) == Some("reverb.mix")).map(|(_, j)| jget(j, "value").as_f64().unwrap());
    assert_eq!(last_mix, Some(0.15), "the drop's reverb is the last one sent: {master:?}");
    // and the ramps did run up to the line (the riser is heard)
    let sweeps = master[..off].iter().filter(|(_, j)| j.get("param").and_then(JsonValue::as_str) == Some("filter.sweep")).count();
    assert!(sweeps > 10, "the riser's high-pass moved: {sweeps}");
}
