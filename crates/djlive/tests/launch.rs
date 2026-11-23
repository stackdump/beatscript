//! Scenes and launch (docs/BEATSCRIPT.md, "Scenes and launch"): `launch drop on pad(2) next phrase 16`. The statement (parse, print, canonical
//! form, errors in the language's terms), what `check` says of it, each scene compiled on its own, and the runtime on a fake clock with a
//! scripted pad: a hit queues the scene, the last hit before the decision wins, it lands on a bar line as a lap that begins at the scene,
//! the take logs it and the replay brings it in on the same bar line, and a host with no pad plays the song as written.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Severity};
use djlive::perform::Performance;
use djlive::runtime::{Grid, Live, LiveHost};

// ---- the statement

const SONG: &str = "\
tempo 126
key am
deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | hihat x*8
deck B: bass a1 ~ a1 ~
section verse 8 bars {
  deck A: snare ~ ~ ~ ~ x ~ ~ ~
}
section drop 8 bars {
  deck A: snare ~ ~ ~ ~ x ~ ~ ~ | clap ~ ~ x ~
  deck B: melody a4 c5 e5 g5
}
section breakdown 8 bars {
  deck A: hihat x*8
}
arrange verse drop breakdown verse
loop
";

fn with(extra: &str) -> String {
    format!("{SONG}{extra}")
}

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", render_all(&c.diagnostics, src)))
}

fn first_error(src: &str) -> djlive::Diagnostic {
    let c = check(src);
    assert!(c.compiled.is_none(), "should not compile:\n{src}");
    c.diagnostics.into_iter().find(|d| d.severity == Severity::Error).expect("an error")
}

#[test]
fn a_launch_is_read_and_printed_as_one_spelling_whatever_the_default_is_called() {
    for (written, canonical) in [
        ("launch drop on pad(2)", "launch drop on pad(2)"),
        ("launch drop on pad(2) next phrase", "launch drop on pad(2)"),
        ("launch drop on pad(2) next phrase 8", "launch drop on pad(2)"),
        ("launch drop on pad(2) next phrase 16", "launch drop on pad(2) next phrase 16"),
        ("launch drop on pad(2) next phrase 32", "launch drop on pad(2) next phrase 32"),
        ("launch   drop   on   pad ( 2 )   next   bar", "launch drop on pad(2) next bar"),
    ] {
        let c = compiled(&with(&format!("{written}\n")));
        assert!(c.text.ends_with(&format!("{canonical}\n")), "{written} -> {}", c.text);
        // the canonical text is a fixed point, with the same CID
        let again = compiled(&c.text);
        assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()), "{written}");
    }
    // the default says nothing and is the same program; another bar line is another program
    let cid = |t: &str| compiled(&with(&format!("{t}\n"))).cid;
    assert_eq!(cid("launch drop on pad(2)"), cid("launch drop on pad(2) next phrase 8"));
    assert_ne!(cid("launch drop on pad(2)"), cid("launch drop on pad(2) next phrase 16"));
    assert_ne!(cid("launch drop on pad(2)"), cid("launch drop on pad(2) next bar"));
    assert_ne!(cid("launch drop on pad(2)"), cid("launch drop on pad(3)"));
    assert_ne!(cid("launch drop on pad(2)"), compiled(SONG).cid, "a launch is text, so it is in the CID");
}

#[test]
fn a_def_called_launch_is_still_a_call() {
    let src = "def launch(d) {\n  deck $d: kick x ~ x ~\n}\nlaunch(A)\n";
    let c = compiled(src);
    assert_eq!(c.text, src);
    assert!(c.launches.is_empty());
}

#[test]
fn a_launch_names_a_section_or_an_arrangement_and_says_what_is_there_when_it_does_not() {
    let d = first_error(&with("launch drops on pad(2)\n"));
    assert_eq!(d.message, "`launch drops`: there is no section or arrangement called `drops`");
    assert!(d.help.as_deref().is_some_and(|h| h.contains("did you mean `drop`?")), "{:?}", d.help);
    // where it is: the name
    let src = with("launch drops on pad(2)\n");
    assert_eq!(djlive::diag::line_col(&src, d.span.start), (SONG.lines().count() + 1, 8));
    // nothing near it: the list
    let d = first_error(&with("launch zzzz on pad(2)\n"));
    let help = d.help.unwrap();
    assert!(help.contains("`verse`") && help.contains("`drop`") && help.contains("`breakdown`"), "{help}");
    // a composition is not a section
    let d = first_error("section a1x 2 bars {\n  deck A: kick x\n}\nsection b1x 2 bars {\n  deck B: bass c2\n}\nlet both = stack(a1x, b1x)\narrange both\nlaunch both on pad(1)\n");
    assert!(d.help.unwrap().contains("a composition"), "{}", d.message);
}

#[test]
fn one_pad_launches_one_scene() {
    let d = first_error(&with("launch drop on pad(2)\nlaunch breakdown on pad(2)\n"));
    assert!(d.message.starts_with("pad(2) already launches `drop` (line "), "{}", d.message);
    assert_eq!(d.help.as_deref(), Some("one pad launches one scene: use another pad"));
    // two pads may bring in one scene
    assert!(check(&with("launch drop on pad(2)\nlaunch drop on pad(3) next bar\n")).compiled.is_some());
}

#[test]
fn a_launch_needs_a_song_and_belongs_at_the_top() {
    let d = first_error("deck A: kick x ~ x ~\nlaunch drop on pad(1)\n");
    assert_eq!(d.message, "`launch` needs a song to launch into");
    assert!(d.help.unwrap().contains("section drop 8 bars"));
    let d = first_error(&with("repeat 2 {\n  launch drop on pad(2)\n}\n"));
    assert!(d.message.contains("belongs at the top"), "{}", d.message);
}

#[test]
fn a_launch_is_started_by_a_pad_and_a_phrase_is_8_16_or_32() {
    let d = first_error(&with("launch drop on knob(2)\n"));
    assert_eq!(d.message, "a launch is started by a pad: `launch drop on pad(2)`");
    assert!(first_error(&with("launch drop\n")).message.contains("expected `on pad(N)`"));
    assert!(first_error(&with("launch drop on pad(2) next phrase 12\n")).message.contains("a phrase is 8, 16 or 32 bars"));
    assert!(first_error(&with("launch drop on pad(0)\n")).message.contains("numbered from 1"));
    assert!(first_error(&with("launch drop on pad(129)\n")).message.contains("up to 128 pads"));
    assert!(first_error(&with("launch\n")).message.contains("expected a section or an arrangement to launch"));
    assert!(first_error(&with("launch drop on pad(2) next\n")).message.contains("expected `phrase`"));
}

// ---- what check says

#[test]
fn a_launched_scene_is_not_one_that_never_plays_and_check_says_the_pads() {
    // an arrangement no `start` reaches, and a section no `arrange` places: both warned about as they were, until a launch names them
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n}\nsection grind 4 bars {\n  deck A: kick x x x x\n}\nsection spare 4 bars {\n  deck A: kick x ~ ~ ~\n}\narrange main: verse then main\narrange side: grind then main\n";
    let warns = |s: &str| -> Vec<String> { check(s).diagnostics.iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message.clone()).collect() };
    let before = warns(src);
    assert!(before.iter().any(|w| w.contains("arrangement `side` is not reachable")), "{before:?}");
    assert!(before.iter().any(|w| w.contains("section `spare` is not in the `arrange`") || w.contains("`spare`")), "{before:?}");
    let launched = format!("{src}launch side on pad(1)\nlaunch spare on pad(2)\n");
    let after = warns(&launched);
    assert!(!after.iter().any(|w| w.contains("`side`") || w.contains("`spare`")), "{after:?}");
    let c = check(&launched);
    let info = c.diagnostics.iter().find(|d| d.severity == Severity::Info && d.message.starts_with("launch:")).expect("an info line");
    assert!(info.message.contains("pad(1) side, pad(2) spare"), "{}", info.message);
    // the flow no longer lists it as unreachable
    let flow = c.compiled.unwrap().flow.unwrap();
    assert!(!flow.unreachable.contains(&"side".to_string()), "{:?}", flow.unreachable);
}

// ---- each scene is a program of its own

#[test]
fn each_scene_is_the_program_with_the_song_replaced_by_it() {
    let c = compiled(&with("launch drop on pad(2)\nlaunch breakdown on pad(3) next phrase 16\n"));
    assert_eq!(c.launches.table.len(), 2);
    assert_eq!(c.launches.scene_names(), vec!["drop", "breakdown"]);
    let written = compiled(&with("launch drop on pad(2)\nlaunch breakdown on pad(3) next phrase 16\n"));
    let drop = c.launches.scene("drop").expect("the drop scene");
    // the same text and CID as the program (a launch is of the program as written), the timeline is the section, looping
    assert_eq!((drop.text.as_str(), drop.cid.as_str()), (written.text.as_str(), written.cid.as_str()));
    assert!(drop.arranged && drop.looped);
    let names: Vec<&str> = drop.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["drop"]);
    assert_eq!(drop.song_bars(), Some(8));
    assert_eq!(drop.loop_from, 0);
    // the written song is what it was
    assert_eq!(c.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["verse", "drop", "breakdown", "verse"]);
    assert!(c.launches.scene("verse").is_none(), "only what a launch names is compiled");
}

#[test]
fn an_arrangement_is_a_scene_that_ends_as_it_says() {
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n}\nsection grind 4 bars {\n  deck A: kick x x x x\n}\nsection coda 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange main: verse loop\narrange side: grind coda end\nlaunch side on pad(1)\n";
    let c = compiled(src);
    let side = c.launches.scene("side").expect("a scene");
    assert_eq!(side.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["grind", "coda"]);
    assert!(!side.looped, "it ends where `side` says: {:?}", side.end);
    assert_eq!(side.end, Some(6));
    // the program as written loops the verse
    assert!(c.looped);
}

#[test]
fn a_line_that_counts_from_a_section_the_scene_has_not_got_does_not_fire_there_and_check_says_so() {
    // the riser before the drop is a line of the verse: in the scene `verse` there is no drop to count from
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n  before drop 2 beats: master: reverb 0.9\n}\nsection drop 4 bars {\n  deck A: kick x x x x\n}\narrange verse drop\nlaunch verse on pad(1)\nlaunch drop on pad(2)\n";
    let c = check(src);
    let compiled = c.compiled.unwrap_or_else(|| panic!("{}", render_all(&c.diagnostics, src)));
    // said at the name, once, with the scene
    let said: Vec<&djlive::Diagnostic> = c.diagnostics.iter().filter(|d| d.message.contains("is not in the scene")).collect();
    assert_eq!(said.len(), 1, "{:?}", c.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    assert_eq!(said[0].message, "`drop` is not in the scene `verse`: a line that counts from it does not fire there");
    assert_eq!(said[0].severity, Severity::Warning);
    assert_eq!(djlive::diag::line_col(src, said[0].span.start), (4, 10));
    assert!(said[0].help.as_deref().is_some_and(|h| h.contains("the `launch` on line 10")), "{:?}", said[0].help);
    // the written song keeps the line (it fires before the drop as it did), the scene has no such control
    assert!(compiled.controls.iter().any(|k| k.key.contains("reverb")));
    assert!(!compiled.launches.scene("verse").unwrap().controls.iter().any(|k| k.key.contains("reverb")));
    // the drop scene is not touched by it
    assert!(compiled.launches.scene("drop").is_some());
}

#[test]
fn an_error_in_a_scene_is_said_where_it_is_with_the_scene_named() {
    // a def call that is not a def, inside the section the scene plays: the program as written has the same error, so say it once
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n  nope(A)\n}\narrange verse\nlaunch verse on pad(1)\n";
    let d = first_error(src);
    assert!(d.message.contains("`nope` is not a def"), "{}", d.message);
}

// ---- the runtime: a fake clock and a scripted pad

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat, noun, params) of every call.
    calls: Vec<(f64, String, String)>,
    /// (beat, deck, envelope as text) of every net loaded.
    loads: Vec<(f64, String, String)>,
    script: BTreeMap<String, f64>,
    /// Whether the host has controllers at all: with none it answers nothing (the page, the phone).
    answers: bool,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[usize::from(deck == "B")]
    }

    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        self.loads.push((self.beat, deck.into(), env.canonical_js()));
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
        if !self.answers {
            return None;
        }
        Some(self.script.get(&format!("{kind}({id})")).copied().unwrap_or(0.0))
    }
}

impl Fake {
    /// Deck A plays from beat 8: the program lands on the runtime's bar 1 (beat 12), so the song's bar `n` is the runtime's bar `n + 1`.
    fn new() -> Fake {
        Fake { beat: 8.0, playing: [true, false], answers: true, ..Fake::default() }
    }

    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats / 0.25).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    /// Run to the runtime's bar `line` (what `btw get` counts: 0 is the bar it was evaluated in, the song's first bar is 1), `into` of a bar
    /// past its line (0 is the line itself). Deck A plays from beat 8, which is the runtime's bar 0.
    fn to(&mut self, live: &mut Live, line: i64, into: f64) {
        let want = (line + 2) as f64 * 4.0 + into * 4.0;
        while self.beat + 1e-9 < want {
            self.beat += 0.25;
            live.service(self);
        }
    }

    /// Hit a pad: down for one pass, then up.
    fn hit(&mut self, live: &mut Live, pad: u32) {
        self.script.insert(format!("pad({pad})"), 1.0);
        self.run(live, 0.25);
        self.script.insert(format!("pad({pad})"), 0.0);
    }
}

fn started(src: &str) -> (Fake, Live) {
    let mut live = Live::new();
    let ev = live.eval(src);
    assert!(ev.ok, "{}", render_all(&ev.diagnostics, src));
    (Fake::new(), live)
}

fn section(live: &Live) -> Option<(String, i64)> {
    live.section().map(|(n, b, _)| (n, b))
}

fn said(live: &Live, what: &str) -> bool {
    live.words.iter().any(|w| w.contains(what))
}

/// The song of the tests: eight-bar sections, `next bar` and phrase launches.
fn song() -> String {
    with("launch verse on pad(1)\nlaunch drop on pad(2)\nlaunch breakdown on pad(3) next phrase 16\nlaunch drop on pad(4) next bar\n")
}

#[test]
fn a_hit_queues_the_scene_and_it_lands_on_the_phrase_line_as_a_lap_that_begins_at_the_scene() {
    let (mut h, mut live) = started(&song());
    // the song as written: verse for 8 bars (bars 1 to 8)
    h.to(&mut live, 3, 0.1);
    assert_eq!(section(&live), Some(("verse".into(), 2)));
    // a hit of pad 2 in the middle of the third bar: it lands on the next phrase line, the song's bar 8 (bar 9)
    h.hit(&mut live, 2);
    assert!(said(&live, "drop queued, lands bar 9"), "{:?}", live.words);
    assert_eq!(live.status().get("queued").and_then(|q| q.get("scene")).and_then(JsonValue::as_str), Some("drop"));
    h.to(&mut live, 8, 0.9);
    assert_eq!(section(&live).map(|s| s.0), Some("verse".into()), "the song plays on until the line");
    h.to(&mut live, 9, 0.1);
    // the drop, from its bar 0, and it stays: the written song would have gone drop, breakdown, verse
    assert_eq!(section(&live), Some(("drop".into(), 0)));
    assert!(said(&live, "bar 9: the scene drop begins"), "{:?}", live.words);
    assert_eq!(live.status().get("scene").and_then(JsonValue::as_str), Some("drop"));
    assert!(live.status().get("queued").is_none());
    h.to(&mut live, 17, 0.1);
    assert_eq!(section(&live), Some(("drop".into(), 0)), "a scene loops until the next launch: the drop again");
    // the running program is still the program as written
    assert_eq!(live.running_cid(), Some(compiled(&song()).cid.as_str()));
}

#[test]
fn the_nets_of_the_scene_are_loaded_in_the_bar_before_it_lands_and_the_song_plays_on_till_then() {
    let (mut h, mut live) = started(&song());
    let c = compiled(&song());
    let drop = c.launches.scene("drop").unwrap();
    let drop_kick = drop.segments[0].decks.get(&djlive::ast::Deck::B).unwrap().envelope(0).canonical_js();
    h.to(&mut live, 3, 0.1);
    h.hit(&mut live, 2);
    let before = h.loads.len();
    h.to(&mut live, 8, 0.5);
    // loaded in the first three quarters of bar 8 (beats 40 to 43), for the line of bar 9 (beat 44)
    let late: Vec<&(f64, String, String)> = h.loads[before..].iter().filter(|l| l.2 == drop_kick).collect();
    assert!(!late.is_empty(), "the drop's net for deck B is loaded");
    assert!(late.iter().all(|l| (40.0..43.0 + 1e-9).contains(&l.0)), "{:?}", late.iter().map(|l| l.0).collect::<Vec<_>>());
    assert_eq!(late.len(), 1, "once");
}

#[test]
fn a_hit_in_the_last_quarter_of_the_bar_before_the_line_waits_for_the_next_quantum_line() {
    let (mut h, mut live) = started(&song());
    // pad 2 is the phrase launch: the line is bar 9; the engine has until three quarters of bar 8 to load it
    h.to(&mut live, 8, 0.8);
    h.hit(&mut live, 2);
    assert!(said(&live, "drop queued, lands bar 17"), "{:?}", live.words);
    h.to(&mut live, 9, 0.5);
    assert_eq!(section(&live).map(|s| s.0), Some("drop".into()), "the written song's own drop plays at bar 9");
    assert!(live.status().get("scene").is_none(), "no scene has landed");
    h.to(&mut live, 17, 0.5);
    assert_eq!(live.status().get("scene").and_then(JsonValue::as_str), Some("drop"));
    assert_eq!(section(&live), Some(("drop".into(), 0)));
}

#[test]
fn the_last_hit_before_the_decision_wins_and_a_held_pad_is_one_hit() {
    let (mut h, mut live) = started(&song());
    h.to(&mut live, 3, 0.1);
    h.hit(&mut live, 2);
    h.run(&mut live, 2.0);
    h.hit(&mut live, 1);
    assert!(said(&live, "verse queued, lands bar 9"), "{:?}", live.words);
    // a pad held down is not a hit each pass
    h.script.insert("pad(3)".into(), 1.0);
    h.run(&mut live, 3.0);
    assert_eq!(live.words.iter().filter(|w| w.contains("breakdown queued")).count(), 1, "{:?}", live.words);
    h.script.insert("pad(3)".into(), 0.0);
    h.to(&mut live, 9, 0.5);
    // pad 3 wanted a 16-bar line and came last: it is the queue's, and it lands on the next such line (bar 17)
    assert!(said(&live, "breakdown queued, lands bar 17"), "{:?}", live.words);
    h.to(&mut live, 17, 0.5);
    assert_eq!(section(&live), Some(("breakdown".into(), 0)));
}

#[test]
fn a_host_with_no_pad_plays_the_song_as_written() {
    let mut live = Live::new();
    assert!(live.eval(&song()).ok);
    let mut h = Fake { answers: false, ..Fake::new() };
    let mut order = Vec::new();
    for bar in 1..=43 {
        h.to(&mut live, bar, 0.1);
        if let Some((n, _)) = section(&live) {
            if order.last() != Some(&n) {
                order.push(n);
            }
        }
    }
    assert_eq!(order, vec!["verse", "drop", "breakdown", "verse", "drop"], "{order:?}");
    assert!(live.status().get("scene").is_none() && live.status().get("queued").is_none());
}

#[test]
fn what_the_song_held_is_put_back_and_the_scenes_own_lines_count_from_its_bar_0() {
    let src = "tempo 120\nsection verse 8 bars {\n  deck A: kick x ~ x ~\n  at bar 2: deck B: level 0.3\n}\nsection drop 8 bars {\n  deck A: kick x x x x\n  deck B: level 0.9\n  at bar 2: master: reverb 0.4\n}\narrange verse drop\nloop\nlaunch drop on pad(2) next bar\n";
    let (mut h, mut live) = started(src);
    h.to(&mut live, 3, 0.1);
    h.hit(&mut live, 2);
    let at = h.calls.len();
    h.to(&mut live, 6, 0.1);
    // landed on bar 4: the drop's own lines, from its bar 0: the level on the line, the reverb a bar later
    let mixer: Vec<f64> = h.calls[at..].iter().filter(|c| c.1 == "mixer").map(|c| c.0).collect();
    assert_eq!(mixer, vec![24.0], "deck B's level 0.9 on the bar line the scene lands on: {:?}", h.calls[at..].to_vec());
    let reverb: Vec<f64> = h.calls[at..].iter().filter(|c| c.1 == "inserts" && c.2.contains("reverb.mix")).map(|c| c.0).collect();
    assert_eq!(reverb, vec![32.0], "`at bar 2` of the drop is two bars after it lands");
}

#[test]
fn a_launch_after_the_song_ended_starts_its_scene() {
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 4 bars {\n  deck A: kick x x x x\n}\narrange verse\nlaunch drop on pad(2)\n";
    let (mut h, mut live) = started(src);
    h.to(&mut live, 7, 0.0);
    assert!(live.status().get("ended").and_then(JsonValue::as_bool).unwrap_or(false), "{:?}", live.words);
    assert!(live.armed(), "a pad is waiting for a hit");
    h.hit(&mut live, 2);
    h.run(&mut live, 2.0);
    assert!(live.is_running(), "the scene starts: a dead pad would be a stop state: {:?}", live.words);
    assert_eq!(live.status().get("scene").and_then(JsonValue::as_str), Some("drop"));
    assert!(!live.armed());
}

// ---- the take and the replay

#[test]
fn a_take_logs_the_launch_and_the_replay_brings_the_scene_in_on_the_same_bar_line() {
    let src = song();
    let (mut h, mut live) = started(&src);
    h.to(&mut live, 3, 0.1);
    h.hit(&mut live, 2);
    h.to(&mut live, 20, 0.5);
    h.hit(&mut live, 1);
    h.to(&mut live, 40, 0.5);
    let perf = live.performance().expect("a take: it has launches though it has no input");
    let landed: Vec<(i64, &str)> = perf.launches.iter().map(|l| (l.bar, l.scene.as_str())).collect();
    assert_eq!(landed, vec![(9, "drop"), (25, "verse")], "{:?}", live.words);
    // saved as JSON text: `launches` is there, and only because there are some
    let text = perf.to_json().canonical_js();
    assert!(text.contains(r#""launches":[{"bar":9,"scene":"drop"},{"bar":25,"scene":"verse"}]"#), "{text}");
    let back = Performance::from_json(&JsonValue::parse(&text).unwrap()).unwrap();
    assert_eq!(back, perf);
    assert_eq!(back.cid(), perf.cid());
    // replayed with a host whose pads are silent: the same scenes on the same bar lines, call for call
    let run_calls = h.calls.clone();
    let run_loads = h.loads.clone();
    let mut rh = Fake::new();
    let mut r = Live::new();
    assert!(r.eval(&src).ok);
    r.replay(back).unwrap();
    rh.to(&mut r, 40, 0.5);
    assert_eq!(rh.calls, run_calls);
    assert_eq!(rh.loads, run_loads, "the same nets on the same beats");
    assert_eq!(r.performance().unwrap().cid(), perf.cid());
}

#[test]
fn a_performance_without_launches_is_the_same_object_it_was() {
    let p = Performance::new("zProg", vec![djlive::perform::Reading { bar: 1, at: 0, input: "knob(1)".into(), value: 0.25 }]);
    assert!(!p.to_json().canonical_js().contains("launches"));
    let mut q = p.clone();
    q.launches.push(djlive::perform::Launched { bar: 4, scene: "drop".into() });
    assert_ne!(q.cid(), p.cid(), "a launch is part of the take");
    let bad = JsonValue::parse(&q.to_json().canonical_js().replace("\"bar\":4", "\"bar\":-1")).unwrap();
    assert!(Performance::from_json(&bad).unwrap_err().contains("launch 0"));
}

// ---- editing while a scene plays

#[test]
fn an_edit_lands_on_the_scene_that_plays_and_a_scene_that_is_gone_sends_the_song_back_to_its_start() {
    let (mut h, mut live) = started(&song());
    h.to(&mut live, 3, 0.1);
    h.hit(&mut live, 4);
    h.to(&mut live, 6, 0.1);
    assert_eq!(live.status().get("scene").and_then(JsonValue::as_str), Some("drop"));
    // the drop's snare changed: the scene goes on, now with the new snare
    let edited = song().replace("deck A: snare ~ ~ ~ ~ x ~ ~ ~ | clap ~ ~ x ~", "deck A: snare ~ ~ ~ ~ x ~ x ~ | clap ~ ~ x ~");
    assert_ne!(edited, song());
    let ev = live.eval(&edited);
    assert!(ev.ok, "{}", render_all(&ev.diagnostics, &edited));
    h.to(&mut live, 9, 0.1);
    assert_eq!(live.status().get("scene").and_then(JsonValue::as_str), Some("drop"), "{:?}", live.words);
    assert_eq!(live.running_cid(), Some(compiled(&edited).cid.as_str()));
    // the launch of the drop is taken out: the scene is not in the program, and the song starts again
    let without = edited.replace("launch drop on pad(4) next bar\n", "").replace("launch drop on pad(2)\n", "");
    assert!(live.eval(&without).ok);
    h.to(&mut live, 13, 0.1);
    assert!(live.status().get("scene").is_none(), "{:?}", live.words);
    assert!(said(&live, "scene `drop` is not in the program any more"), "{:?}", live.words);
}
