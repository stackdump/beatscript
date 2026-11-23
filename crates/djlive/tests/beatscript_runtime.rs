//! BeatScript on a fake host and a fake clock: an arranged song lands each section's nets on its bar line,
//! `for N bars` and moves put things back, per-role sounds are set once a net has landed.
//! (The hot reload on a fake host and a fake clock: a change lands on the bar and not before, an
//! error keeps the last good program, what did not change is not touched, `every` and ramps keep
//! their time through an edit.)

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
    /// (beat when called, deck, role, option)
    sounds: Vec<(f64, String, String, String)>,
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

    fn set_sound(&mut self, deck: &str, role: &str, option: &str) -> Result<(), String> {
        self.sounds.push((self.beat, deck.into(), role.into(), option.into()));
        Ok(())
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


fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

fn bar_of(beat: f64) -> i64 {
    (beat / 4.0).floor() as i64
}

#[test]
fn a_move_from_a_program_puts_its_strip_back_at_the_next_phrase() {
    let (mut h, mut live) = playing();
    assert!(live.eval("every 16 bars: move filter_exit on B over 4 bars\n").ok);
    h.run(&mut live, 4.0 * 45.0);
    let moves = h.calls_of("moves");
    let unmutes: Vec<(i64, String)> = h
        .calls_of("mixer")
        .into_iter()
        .map(|(b, p)| (bar_of(b), p))
        .collect();
    assert!(moves.len() >= 2, "{moves:?}");
    // the first move fires on program bar 16, ends on 20, and the strip is back on the phrase line 24
    let first = bar_of(moves[0].0);
    assert!(unmutes.iter().all(|(_, p)| p.contains("\"mute\":false") && p.contains("\"deck\":\"B\"")), "{unmutes:?}");
    let back = unmutes[0].0;
    assert!(back > first && back - first <= 8, "move on {first}, strip back on {back}");
    // never past the next phrase after the move ends: and again for the next one
    assert!(unmutes.len() >= 2, "{unmutes:?}");
}

#[test]
fn stopping_a_program_puts_back_what_is_still_held() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2: move echo_out on B over 4 bars\n").ok);
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.calls_of("moves").len(), 1);
    assert!(h.calls_of("mixer").is_empty());
    live.stop_with(&mut h);
    assert_eq!(h.calls_of("mixer").len(), 1, "the strip is put back when the program stops");
}

#[test]
fn for_n_bars_undoes_it_on_the_bar() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 4 for 2 bars: master: filter lp 800\n").ok);
    h.run(&mut live, 4.0 * 10.0);
    let ons: Vec<(i64, String)> = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("\"action\":\"on\"")).map(|(b, p)| (bar_of(b), p)).collect();
    assert_eq!(ons.len(), 2, "{ons:?}");
    assert!(ons[0].1.contains("\"on\":true") && ons[1].1.contains("\"on\":false"), "{ons:?}");
    assert_eq!(ons[1].0 - ons[0].0, 2);
}

const SONG: &str = "\
section intro 4 bars {
  deck A: kick x ~ x ~
  at bar 2: master: reverb 0.4
}
section drop 4 bars {
  deck A: kick x x x x | bass c2 sound=synth:wobble-lead
  at bar 1: master: delay 0.3
}
arrange intro drop
";

#[test]
fn an_arranged_song_lands_each_sections_nets_on_its_bar_line() {
    let (mut h, mut live) = playing();
    let ev = live.eval(SONG);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    assert_eq!(ev.nets.len(), 2);
    h.run(&mut live, 4.0 * 7.0);
    let loads: Vec<_> = h.loads.iter().filter(|l| l.0 == "A").collect();
    assert_eq!(loads.len(), 2, "{:?}", h.loads);
    // the intro lands on the song's first bar, the drop four bars on
    assert_eq!(loads[1].2 - loads[0].2, 4);
    assert_ne!(loads[0].1, loads[1].1);
    // the controls count from the section's start
    let rev = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("reverb")).map(|(b, _)| bar_of(b)).next().unwrap();
    let del = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("delay")).map(|(b, _)| bar_of(b)).next().unwrap();
    assert_eq!(del - rev, 3, "bar 2 of the intro, then bar 1 of the drop");
    assert_eq!(live.section().map(|s| s.0).as_deref(), Some("drop"));
}

#[test]
fn a_roles_sound_is_set_once_its_net_has_landed() {
    let (mut h, mut live) = playing();
    live.eval(SONG);
    h.run(&mut live, 4.0 * 7.0);
    assert_eq!(h.sounds.len(), 1, "{:?}", h.sounds);
    let (beat, deck, role, opt) = &h.sounds[0];
    assert_eq!((deck.as_str(), role.as_str(), opt.as_str()), ("A", "bass", "synth:wobble-lead"));
    let landed = h.loads.iter().filter(|l| l.0 == "A").nth(1).unwrap().2;
    assert!(bar_of(*beat) >= landed, "set at bar {} before the net landed on {landed}", bar_of(*beat));
}

#[test]
fn the_status_names_the_section_and_the_bar_in_it() {
    let (mut h, mut live) = playing();
    live.eval(SONG);
    h.run(&mut live, 4.0 * 3.0);
    let st = live.status();
    assert_eq!(st.get("section").and_then(|v| v.as_str()), Some("intro"));
    assert_eq!(st.get("song_bars").and_then(|v| v.as_f64()), Some(8.0));
    assert_eq!(st.get("arranged").and_then(|v| v.as_bool()), Some(true));
    h.run(&mut live, 4.0 * 3.0);
    assert_eq!(live.status().get("section").and_then(|v| v.as_str()), Some("drop"));
}

#[test]
fn an_edit_in_a_section_changes_only_that_sections_net() {
    let (mut h, mut live) = playing();
    live.eval(SONG);
    h.run(&mut live, 4.0 * 2.0);
    let n = h.loads.len();
    let edited = SONG.replace("bass c2", "bass e2");
    let ev = live.eval(&edited);
    let lines = ev.plan.as_ref().unwrap().lines();
    assert!(lines.iter().any(|l| l.contains("drop@4") && l.contains("bass changed")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("intro")), "{lines:?}");
    h.run(&mut live, 4.0 * 1.0);
    assert_eq!(h.loads.len(), n, "the intro's net is not reloaded, the drop's waits for its bar");
}

// ---- the end

#[test]
fn a_song_ends_after_its_last_section_and_the_decks_stop() {
    let (mut h, mut live) = playing();
    assert!(live.eval(SONG).ok);
    // intro 4 + drop 4 = eight bars from where it lands
    h.run(&mut live, 4.0 * 6.0);
    assert!(live.is_running() && !h.pauses.contains(&"A".to_string()), "still playing in the last section");
    h.run(&mut live, 4.0 * 6.0);
    assert!(!live.is_running(), "the program has ended");
    assert!(h.pauses.contains(&"A".to_string()), "the deck stopped: {:?}", h.pauses);
    assert!(!h.playing[0]);
    let st = live.status();
    assert_eq!(st.get("ended").and_then(|v| v.as_bool()), Some(true));
    let at = st.get("ended_at_bar").and_then(|v| v.as_f64()).unwrap();
    // eight bars after the song's first bar (program bar 1: it landed the bar after it was evaluated)
    assert_eq!(at, 9.0);
    // no deck is loaded again after the end, nothing wraps to bar 1
    let loads = h.loads.len();
    h.beat += 0.0;
    h.playing[0] = true; // something else plays the clock
    h.run(&mut live, 4.0 * 20.0);
    assert_eq!(h.loads.len(), loads, "a program that ended does not load nets again");
    assert!(live.status().get("ended").and_then(|v| v.as_bool()) == Some(true));
}

#[test]
fn a_flat_program_ends_at_the_bar_it_says_and_puts_back_what_it_holds() {
    let (mut h, mut live) = playing();
    assert!(live.eval("deck A: kick x ~ x ~\nat bar 2: move echo_out on B over 4 bars\nend at bar 6\n").ok);
    h.run(&mut live, 4.0 * 3.0);
    assert!(live.is_running());
    h.run(&mut live, 4.0 * 6.0);
    assert!(!live.is_running());
    assert_eq!(live.status().get("ended_at_bar").and_then(|v| v.as_f64()), Some(live.ended.as_ref().unwrap().bar as f64));
    assert!(live.ended.as_ref().unwrap().bar >= 6);
    // the strip the move muted is back, and the deck stopped
    assert_eq!(h.calls_of("mixer").len(), 1, "{:?}", h.calls);
    assert!(h.pauses.contains(&"A".to_string()));
}

#[test]
fn end_after_a_section_and_loop() {
    // end after the first section: the second never plays
    let (mut h, mut live) = playing();
    let src = SONG.replace("arrange intro drop\n", "arrange intro drop\nend after intro\n");
    assert!(live.eval(&src).ok);
    h.run(&mut live, 4.0 * 8.0);
    assert!(!live.is_running());
    assert_eq!(h.loads.iter().filter(|l| l.0 == "A").count(), 1, "only the intro was ever loaded: {:?}", h.loads);
    // loop: the song starts again, loading the intro's net a second time
    let (mut h, mut live) = playing();
    let src = format!("{SONG}loop\n");
    assert!(live.eval(&src).ok);
    h.run(&mut live, 4.0 * 14.0);
    assert!(live.is_running(), "a looping song does not end");
    assert!(live.ended.is_none());
    let intro_loads = h.loads.iter().filter(|l| l.0 == "A").count();
    assert!(intro_loads >= 3, "intro, drop, intro again: {:?}", h.loads);
    // the control lines fire again each lap
    let rev = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("reverb")).count();
    assert!(rev >= 2, "{rev}");
}

#[test]
fn a_new_program_after_the_end_starts_fresh() {
    let (mut h, mut live) = playing();
    assert!(live.eval("deck A: kick x\nend at bar 3\n").ok);
    h.run(&mut live, 4.0 * 8.0);
    assert!(live.ended.is_some());
    h.playing[0] = true;
    assert!(live.eval("deck A: snare x\n").ok);
    assert!(live.ended.is_none());
    h.run(&mut live, 4.0 * 2.0);
    assert!(live.is_running());
}

#[test]
fn an_evolving_section_swaps_in_each_generations_net_on_its_bar_line() {
    let (mut h, mut live) = playing();
    let src = "section drop 12 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 4 bars {\n    bass: transpose 2\n    stop when gen == 2\n  }\n}\narrange drop\n";
    let ev = live.eval(src);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    assert_eq!(ev.lineage.len(), 3);
    h.run(&mut live, 4.0 * 6.0);
    let loads: Vec<_> = h.loads.iter().filter(|l| l.0 == "A").collect();
    assert_eq!(loads.len(), 2, "gen 0 and gen 1 so far: {loads:?}");
    assert_eq!(loads[1].2 - loads[0].2, 4, "a generation is four bars");
    assert_eq!(live.status().get("generation").and_then(|v| v.as_f64()), Some(1.0));
    assert_eq!(live.status().get("section").and_then(|v| v.as_str()), Some("drop"));
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.loads.iter().filter(|l| l.0 == "A").count(), 3, "gen 2");
    // the net of each generation is the lineage's: three different nets
    let mut cids: Vec<&String> = h.loads.iter().filter(|l| l.0 == "A").map(|l| &l.1).collect();
    cids.dedup();
    assert_eq!(cids.len(), 3);
    h.run(&mut live, 4.0 * 8.0);
    assert!(!live.is_running(), "the song ends where the evolution does");
}

#[test]
fn a_program_too_long_for_one_net_swaps_nets_on_the_bar_line_keeps_its_clock_and_goes_round() {
    // 80 bars of drums: a net of 64 bars, then one of 16, then round again. Its `at bar` lines keep the
    // program's own count (the first music bar is bar 1), as a program that fits in a net does.
    let bar = |k: usize| if k % 2 == 0 { "[x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~]" } else { "[x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~]" };
    let kick: Vec<&str> = (0..80).map(bar).collect();
    let src = format!("deck A: kick <{}>\nat bar 70: master: delay 0.3\n", kick.join(" "));
    let (mut h, mut live) = playing();
    let ev = live.eval(&src);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    h.run(&mut live, 4.0 * 66.0);
    let loads: Vec<_> = h.loads.iter().filter(|l| l.0 == "A").collect();
    assert_eq!(loads.len(), 2, "the first net and the second, at bar 64 of the chain: {loads:?}");
    assert_eq!(loads[1].2 - loads[0].2, 64);
    h.run(&mut live, 4.0 * 20.0);
    // `at bar 70` fired once, on program bar 70
    let at = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("delay")).map(|(b, _)| (b / 4.0).floor() as i64).collect::<Vec<_>>();
    assert_eq!(at.len(), 2, "the delay is turned on and set: {at:?}");
    // and it went round again: the first net is loaded a second time, after the 80 bars
    h.run(&mut live, 4.0 * 20.0);
    assert!(live.is_running(), "a chain loops");
    assert!(h.loads.iter().filter(|l| l.0 == "A").count() >= 3, "{:?}", h.loads);
    assert_eq!(h.loads.iter().filter(|l| l.0 == "A").next().unwrap().1, h.loads.iter().filter(|l| l.0 == "A").nth(2).unwrap().1, "round again: the first net");
}

/// A section that starts again, by `loop` or by being named twice in `arrange`, starts from its first bar: its nets are
/// loaded again and land on its bar line, even though they are the same nets as the ones playing. (They used to run on: a
/// 2-bar section with a `<p1 p2 p3>` alternation played p1 p2 p3 p1 ... on the engine where the render, which loads each
/// section afresh, played p1 p2 p1 p2. site/test/parity/loop.mjs heard it.)
#[test]
fn a_section_that_starts_again_reloads_its_nets_on_its_bar_line() {
    let lets = "let p1 = x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~\nlet p2 = x ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~\nlet p3 = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~\n";
    let section = "section groove 2 bars {\n  deck A: kick <p1 p2 p3>\n}\n";
    for (what, tail) in [("a single-section loop", "arrange groove\nloop\n"), ("a section named three times", "arrange groove groove groove\n")] {
        let (mut h, mut live) = playing();
        let ev = live.eval(&format!("tempo 120\n{lets}{section}{tail}"));
        assert!(ev.ok, "{what}: {:?}", ev.diagnostics);
        h.run(&mut live, 4.0 * 7.0);
        let lands: Vec<i64> = h.loads.iter().filter(|l| l.0 == "A").map(|l| l.2).collect();
        assert!(lands.len() >= 3, "{what}: the section's net is loaded at each of its starts: {:?}", h.loads);
        // every 2 bars, on the section's bar line, and once each
        assert!(lands.windows(2).all(|w| w[1] - w[0] == 2), "{what}: {lands:?}");
        let first = &h.loads.iter().find(|l| l.0 == "A").unwrap().1;
        assert!(h.loads.iter().filter(|l| l.0 == "A").all(|l| &l.1 == first), "{what}: the same net (phase 0) each time: {:?}", h.loads);
    }
}

/// Named arrangements on a Live: `open` once, then `set` for ever. The lap goes back to the start of `set`, not to `open`.
#[test]
fn named_arrangements_play_in_flow_order_and_the_lap_goes_back_to_the_looping_one() {
    let (mut h, mut live) = playing();
    let src = "section intro 1 bars { deck A: kick x ~ ~ ~ }\nsection build 1 bars { deck A: kick x ~ x ~ }\nsection drop 1 bars { deck A: kick x x x x }\nsection outro 1 bars { deck A: snare x ~ ~ ~ }\narrange open: intro build then set\narrange set: drop outro loop\n";
    let ev = live.eval(src);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..12 {
        h.run(&mut live, 4.0);
        if let Some(s) = live.status().get("section").and_then(|v| v.as_str()) {
            if seen.last().map(String::as_str) != Some(s) {
                seen.push(s.to_string());
            }
        }
    }
    assert!(live.is_running(), "a looping flow does not end");
    assert_eq!(seen[..4], ["intro", "build", "drop", "outro"], "{seen:?}");
    assert!(seen.len() >= 8, "{seen:?}");
    assert!(seen[4..].iter().enumerate().all(|(i, s)| s == if i % 2 == 0 { "drop" } else { "outro" }), "the lap starts at `set`: {seen:?}");
    // a finite flow plays through and ends
    let (mut h, mut live) = playing();
    let src = "section intro 1 bars { deck A: kick x ~ ~ ~ }\nsection drop 1 bars { deck A: kick x x x x }\narrange open: intro then set\narrange set: drop loop 2 then close\narrange close: intro end\n";
    let src = src.replace(" then close", "");
    assert!(live.eval(&src).ok);
    h.run(&mut live, 4.0 * 12.0);
    assert!(!live.is_running(), "loop N ends the song");
    assert!(live.ended.as_ref().is_some_and(|e| e.bar >= 5), "intro + 2 drops + intro = 4 bars");
}
