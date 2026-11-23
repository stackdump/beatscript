//! Regression tests of the fixes of review lens 3 (runtime and hosts) that live in `djlive`'s runtime: each is the repro of a
//! finding (R-xx of the review) on a fake host and a fake clock, and fails without the fix.

use djengine::json::JsonValue;
use djlive::runtime::{Grid, Live, LiveHost};

/// Four beats a bar. The fake deck plays when told and "installs" a net on the next bar line, like `Engine::load_prepared`. Every
/// call is written down with the beat it was made on; the tempo it was last given is kept (`tempos`).
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, net cid, the bar it lands on)
    loads: Vec<(String, String, i64)>,
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
    /// (beat when set, deck, bpm)
    tempos: Vec<(f64, String, f64)>,
    pauses: Vec<String>,
    /// The tempo each deck plays at (what `tempo_of` answers): A at 126.
    deck_tempo: [Option<f64>; 2],
    /// The clip ids `vocal to_row` hands out, in order.
    clips: u32,
    /// A controller that answers `knob(N)` with 1/2.
    set_knob: bool,
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
        // the loop row's clip of a sung line: `vocal to_row` answers its id
        if noun == "vocal" && params.get("action").and_then(JsonValue::as_str) == Some("to_row") {
            self.clips += 1;
            return Ok(JsonValue::Object(djengine::json::JsonObject::from_pairs([("id", JsonValue::str(format!("clip{}", self.clips)))])));
        }
        Ok(JsonValue::Null)
    }

    fn tempo_of(&self, deck: &str) -> Option<f64> {
        self.deck_tempo[ix(deck)]
    }

    fn input(&mut self, kind: &str, _id: &str) -> Option<f64> {
        (self.set_knob && kind == "knob").then_some(0.5)
    }

    fn set_tempo(&mut self, deck: &str, bpm: f64) -> Result<(), String> {
        self.tempos.push((self.beat, deck.into(), bpm));
        Ok(())
    }
}

impl Fake {
    /// Advance `beats` in quarter-beat steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }
}

/// A host with deck A already playing, on a bar line (the host's bar 2): the program's bar 0 is the first pass.
fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    h.deck_tempo[0] = Some(126.0);
    (h, Live::new())
}

/// The clock's bar (a line) a beat is on, counted from the host's bar 2 where `playing()` started: the runtime's own line number.
fn line_of(beat: f64) -> i64 {
    ((beat - 8.0) / 4.0).floor() as i64
}

/// R-03: `fire()` shadowed its own `off` argument with a read of `self.running`, which the callers had taken: the offset was always 0
/// and a `move`'s strip was put back at the phrase line counted from the host's bar 0, a bar early for a song that landed on bar 1.
/// Repro: a move at song bar 2 over 2 bars ends on song bar 4; the strip is back on the next phrase line, song bar 8 (clock bar 9).
#[test]
fn r03_a_move_is_put_back_at_the_songs_phrase_line_not_a_bar_early() {
    let (mut h, mut live) = playing();
    let src = "tempo 120\nsection one 16 bars {\n  deck A: kick x ~ x ~\n  at bar 2: move filter_exit on A over 2 bars\n}\narrange one\n";
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 14.0);
    let arm = h.calls_of("moves");
    assert_eq!(arm.len(), 1, "{:?}", h.calls);
    // the song lands on clock bar 1: its bar 2 is clock bar 3
    assert_eq!(line_of(arm[0].0), 3);
    let unmute = h.calls_of("mixer");
    assert_eq!(unmute.len(), 1, "{unmute:?}");
    assert_eq!(line_of(unmute[0].0), 9, "song bar 8 is clock bar 9 (it was put back on clock bar 8, a bar early)");
}

/// R-02 (decision: the old live count is kept): a program with no sections evaluated while a deck plays has its bar 0 on the bar it was
/// evaluated in, nothing of it sounds on that bar (its net lands on the next one) and `at bar 1` is the first bar you hear. Every timed
/// word counts from there, as every program written before the extensions has always done. With nothing playing the music starts on
/// bar 0 at once and the same words fall on the same bars; only `on next phrase` differs (the first phrase line the program is there
/// for: bar 8 with a deck playing, bar 0 at once). The offline render starts the music on its bar 0 (docs/BEATSCRIPT.md "Time words").
#[test]
fn r02_a_program_with_no_sections_keeps_counting_from_the_bar_it_was_evaluated_in() {
    let src = "deck A: kick x ~ x ~\nat bar 2: master: level 0.5\nevery 3 bars: master: level 0.25\non next phrase: master: level 0.75\nend at bar 10\n";
    let bars = |v: Vec<(f64, String)>, from: f64| v.iter().map(|(b, p)| (((b - from) / 4.0).floor() as i64, p.replace("{\"master\":", "").replace('}', ""))).collect::<Vec<_>>();
    let to = |v: Vec<(i64, &str)>| -> Vec<(i64, String)> { v.into_iter().map(|(b, p)| (b, p.to_string())).collect() };
    // a deck plays (A, from beat 8, the clock's bar 2): the program's bar 0 is that bar (beat 8), its net lands on bar 1 (beat 12)
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 12.0);
    assert_eq!(bars(h.calls_of("mixer"), 8.0), to(vec![(2, "0.5"), (3, "0.25"), (6, "0.25"), (8, "0.75"), (9, "0.25")]), "{:?}", h.calls);
    assert_eq!(h.loads.iter().map(|l| l.2 - 2).collect::<Vec<_>>(), [1], "the music lands on the bar after the evaluation bar");
    // `end at bar 10`: the decks stop on the program's bar 10
    assert!(!live.is_running());
    assert_eq!(live.ended.as_ref().map(|e| e.bar), Some(10), "{:?}", live.ended);
    // nothing plays: the program starts at once, on beat 0, which is its bar 0 and the first bar you hear
    let mut h2 = Fake::default();
    let mut live2 = Live::new();
    assert!(live2.eval(src).ok);
    h2.run(&mut live2, 4.0 * 12.0);
    assert_eq!(bars(h2.calls_of("mixer"), 0.0), to(vec![(0, "0.75"), (2, "0.5"), (3, "0.25"), (6, "0.25"), (9, "0.25")]), "{:?}", h2.calls);
    assert_eq!(h2.loads.iter().map(|l| l.2).collect::<Vec<_>>(), [0]);
}

/// R-02 (the part that is new, not a change of the old count): a tempo curve of a program with no sections starts on the bar its music
/// lands on, the first bar you hear, as in the offline render, not on the silent bar before it. A program that was evaluated while a deck
/// played lands on bar 1 (beat 12 here): the curve `100 -> 140 over 4 bars` is at 100 there and at 140 on bar 5 (beat 28), and the
/// status says the tempo of the same position. Its `at bar` lines are on the old count (the test above) and a song's curve is on its
/// own first bar, which is the landing bar too: the same tempos at the same beats.
#[test]
fn r02_a_flat_programs_tempo_curve_starts_on_the_bar_its_music_lands_on() {
    let series = |src: &str, bars: f64| {
        let (mut h, mut live) = playing();
        assert!(live.eval(src).ok, "{src}");
        h.run(&mut live, 4.0 * bars);
        (h, live)
    };
    let tempo_at = |h: &Fake, beat: f64| h.tempos.iter().filter(|t| t.0 <= beat + 1e-9).map(|t| t.2).last();
    let (h, _) = series("tempo 100 -> 140 over 4 bars\ndeck A: kick x ~ x ~\n", 8.0);
    // before the music lands the first tempo of the curve is already in force, then the ramp: a quarter of the way is bar 2 (beat 16)
    assert_eq!(tempo_at(&h, 12.0), Some(100.0), "{:?}", h.tempos);
    assert!((tempo_at(&h, 16.0).unwrap() - 110.0).abs() < 1e-6, "{:?}", h.tempos);
    assert!((tempo_at(&h, 24.0).unwrap() - 130.0).abs() < 1e-6, "{:?}", h.tempos);
    assert_eq!(h.tempos.iter().find(|t| t.2 == 140.0).map(|t| t.0), Some(28.0), "the curve ends four bars after the landing bar (beat 12 + 16): {:?}", h.tempos);
    // the status says the tempo of the same position (bar 3 is two bars after the landing bar: half the curve)
    let (_, live) = series("tempo 100 -> 140 over 4 bars\ndeck A: kick x ~ x ~\n", 3.0);
    let st = live.status();
    assert_eq!(st.get("bar").and_then(JsonValue::as_f64), Some(3.0));
    assert_eq!(st.get("tempo").and_then(JsonValue::as_f64), Some(120.0), "{st:?}");
    // a song counts its curve from its first section, which is the landing bar too: the same tempos at the same beats
    let (song, _) = series("section one 8 bars {\n  tempo 100 -> 140 over 4 bars\n  deck A: kick x ~ x ~\n}\narrange one\n", 8.0);
    assert_eq!(song.tempos, h.tempos);
}

/// R-02 (found beside it, independent of how the bars are counted): a program too long for one net is chained, and its laps were counted
/// by a clock that did not agree with the lap's own: the lap turned a bar early the first time, and then at every bar line ("the song
/// starts again" in the log, the clock origin moved on by a lap each time). A looped song's first net came a bar late once the lap had
/// turned. Now the lap turns where it ends and the first net is loaded in the bar before it, so it lands on the lap's first bar.
#[test]
fn r02_a_chained_program_and_a_looped_song_turn_their_laps_on_the_bar_line_once() {
    let bar = |k: usize| if k % 2 == 0 { "[x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~]" } else { "[x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~]" };
    let kick: Vec<&str> = (0..80).map(bar).collect();
    let src = format!("deck A: kick <{}>\nat bar 70: master: delay 0.3\n", kick.join(" "));
    let (mut h, mut live) = playing();
    assert!(live.eval(&src).ok);
    h.run(&mut live, 4.0 * 250.0);
    // the nets: 64 bars, 16 bars, and again, each landing on its bar line (the program lands on the clock's bar 1)
    let lines: Vec<i64> = h.loads.iter().map(|l| l.2 - 2).collect();
    assert_eq!(lines[..6], [1, 65, 81, 145, 161, 225], "{lines:?}");
    // the lap turned three times in 250 bars (at 81, 161, 241), each said once, and the origin of the clock did not move on
    let turned = live.words.iter().filter(|w| w.contains("starts again")).count();
    assert_eq!(turned, 3, "{:?}", live.words);
    // `at bar 70` is the same bar each lap: the program's bar 70 of the lap that began on the clock's bar 1, 81, 161: three times
    let delays = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("delay")).count();
    assert_eq!(delays, 2 * 1, "its line fires once (on the first lap: a line that has fired does not fire again)");
    // a song that loops: every section's net is loaded in the bar before it begins, the first section's too when the lap turns
    let song = "section one 2 bars {\n  deck A: kick x ~ x ~\n}\nsection two 2 bars {\n  deck A: kick x x x x\n}\narrange one two\nloop\n";
    let (mut h, mut live) = playing();
    assert!(live.eval(song).ok);
    h.run(&mut live, 4.0 * 14.0);
    let lines: Vec<i64> = h.loads.iter().map(|l| l.2 - 2).collect();
    assert_eq!(lines[..6], [1, 3, 5, 7, 9, 11], "each net lands on its bar line, the first one of a lap too (it was a bar late): {lines:?}");
}

/// R-04: a program that drives the tempo (`tempo 100 -> 140 over 4 bars`) took it from the decks and kept it: `stop` and `end` left the
/// last tempo of the ramp on the deck (129.375 where it was stopped) and never recorded what it was. Now the tempo each deck had when
/// the program was applied is sent back, by `stop` and by `end`, and said in the words.
#[test]
fn r04_stop_and_end_put_the_decks_tempo_back() {
    let src = "tempo 100 -> 140 over 4 bars\ndeck A: kick x ~ x ~\n";
    // stop, in the middle of the ramp
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 3.0);
    let ramped = h.tempos.last().unwrap().2;
    assert!(ramped > 100.0 && ramped < 140.0, "{ramped}");
    let n = h.tempos.len();
    live.stop_with(&mut h);
    assert_eq!(h.tempos.len(), n + 1, "one call puts it back: {:?}", &h.tempos[n..]);
    assert_eq!(h.tempos.last().map(|t| (t.1.as_str(), t.2)), Some(("A", 126.0)));
    assert!(live.words.iter().any(|w| w.contains("tempo put back: deck A is at 126 BPM again")), "{:?}", live.words);
    // and not again: a stopped program has nothing more to put back
    live.stop_with(&mut h);
    assert_eq!(h.tempos.len(), n + 1);
    // end: the ramp is over (140) and the decks stop on the bar line, with the tempo back
    let (mut h, mut live) = playing();
    assert!(live.eval(&format!("{src}end at bar 6\n")).ok);
    h.run(&mut live, 4.0 * 10.0);
    assert!(!live.is_running());
    assert_eq!(h.tempos.iter().map(|t| t.2).fold(0.0, f64::max), 140.0, "the ramp got to its end first");
    assert_eq!(h.tempos.last().map(|t| (t.1.as_str(), t.2)), Some(("A", 126.0)), "{:?}", h.tempos.last());
    // a deck whose tempo is not known (a host with no clock for it) is left where it is: nothing is invented
    let (mut h, mut live) = playing();
    h.deck_tempo = [None, None];
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 3.0);
    let n = h.tempos.len();
    live.stop_with(&mut h);
    assert_eq!(h.tempos.len(), n);
    // a program with a tempo that is one number is not driven, so there is nothing to put back
    let (mut h, mut live) = playing();
    assert!(live.eval("tempo 126\ndeck A: kick x ~ x ~\n").ok);
    h.run(&mut live, 4.0 * 3.0);
    live.stop_with(&mut h);
    assert!(h.tempos.is_empty());
}

/// R-04: a `sing` line puts its voice on the loop row, where it loops until it is stopped, and nothing stopped it: `stop` and `end` left
/// it looping, and an edit of the sung notes (another line) sang a second one over it. Now the clip the line's `vocal to_row` answered
/// is stopped (`loops stop`) when the program stops or ends, when an edit removes the line, and where a lap of a looped song turns.
#[test]
fn r04_a_sung_line_is_stopped_when_the_program_stops_ends_or_the_line_is_edited_away() {
    use djlive::env::{Env, StaticVoices};
    let v = StaticVoices::default();
    let env = Env { voices: Some(&v), ..Env::default() };
    let src = "tempo 120\nkey am\ndeck B: melody c4 e4 g4 e4 | harmony chord(am)\ndeck B: sing \"la la\" on melody voice=alto\n";
    let stops = |h: &Fake| h.calls_of("loops").into_iter().map(|(_, p)| p).collect::<Vec<_>>();
    // stop
    let (mut h, mut live) = playing();
    assert!(live.eval_with(src, &env).ok);
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.clips, 1, "sung once");
    assert!(stops(&h).is_empty());
    live.stop_with(&mut h);
    assert_eq!(stops(&h), vec![r#"{"action":"stop","id":"clip1"}"#.to_string()]);
    // end
    let (mut h, mut live) = playing();
    assert!(live.eval_with(&format!("{src}end at bar 6\n"), &env).ok);
    h.run(&mut live, 4.0 * 10.0);
    assert!(!live.is_running());
    assert_eq!(stops(&h), vec![r#"{"action":"stop","id":"clip1"}"#.to_string()]);
    // the notes it is sung on are edited: another line, so the first is stopped and the second sung
    let (mut h, mut live) = playing();
    assert!(live.eval_with(src, &env).ok);
    h.run(&mut live, 4.0 * 4.0);
    let edited = src.replace("melody c4 e4 g4 e4", "melody c4 e4 g4 a4");
    assert!(live.eval_with(&edited, &env).ok);
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.clips, 2, "the edited line sings its own");
    assert_eq!(stops(&h)[0], r#"{"action":"stop","id":"clip1"}"#, "{:?}", stops(&h));
    // the same text again keeps its voice: it is the same line
    let before = stops(&h).len();
    assert!(live.eval_with(&edited, &env).ok);
    h.run(&mut live, 4.0 * 2.0);
    assert_eq!(stops(&h).len(), before);
    // a song that loops sings again each lap and stops the lap before's voice
    let song = "tempo 120\nkey am\nsection one 2 bars {\n  deck B: melody c4 e4 g4 e4\n  at bar 0: deck B: sing \"la\" on melody voice=alto\n}\narrange one\nloop\n";
    let (mut h, mut live) = playing();
    assert!(live.eval_with(song, &env).ok);
    h.run(&mut live, 4.0 * 7.0);
    assert!(h.clips >= 3, "{}", h.clips);
    assert_eq!(stops(&h).len(), (h.clips - 1) as usize, "each lap but the one that plays stops its voice: {:?}", stops(&h));
}

/// R-05: a hot reload keeps a running line when its key (the section's start and the line's text) is the same, and kept what the line's
/// section's end had given it when it fired: an lfo that stops where its section ends, and a section's `mix` or `duck`, held until it
/// ends. Edit only the section's length and they went on to the old end. Now the end is worked out again from the new length.
#[test]
fn r05_lengthening_a_section_moves_the_end_of_what_a_running_line_holds_and_of_its_lfo() {
    let song = |len: u32| format!("tempo 120\nsection verse {len} bars {{\n  deck A: kick x ~ x ~\n  master: level lfo tri 1 bar center=0.5 depth=0.25\n  at bar 1: mix {{ A level 0.7 }}\n}}\nsection chorus 4 bars {{\n  deck A: kick x x x x\n}}\narrange verse chorus\n");
    let lfo_beats = |h: &Fake| -> Vec<f64> { h.calls_of("mixer").into_iter().filter(|(_, p)| p.contains("\"master\"")).map(|(b, _)| b).collect() };
    let level_a = |h: &Fake| -> Vec<(f64, bool)> { h.calls_of("mixer").into_iter().filter(|(_, p)| p.contains("\"deck\":\"A\"")).map(|(b, p)| (b, p.contains("0.7"))).collect() };
    // the song lands on the clock's bar 1 (beat 12); the verse is its bars 0 to 3 (beats 12 to 28), and its mix, set on its bar 1,
    // is put back where the verse ends: beat 28
    let (mut h, mut live) = playing();
    assert!(live.eval(&song(4)).ok);
    h.run(&mut live, 4.0 * 12.0);
    assert_eq!(lfo_beats(&h).last().copied(), Some(28.0), "the lfo stops where the verse ends");
    let a = level_a(&h);
    assert_eq!(a.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>()[..2], [(16.0, true), (28.0, false)], "{a:?}");
    // lengthen the verse to six bars while it plays: the same lines (the verse starts where it did), a longer verse
    let (mut h, mut live) = playing();
    assert!(live.eval(&song(4)).ok);
    h.run(&mut live, 4.0 * 3.0);
    assert!(live.eval(&song(6)).ok);
    h.run(&mut live, 4.0 * 12.0);
    assert_eq!(lfo_beats(&h).last().copied(), Some(36.0), "the lfo goes on to where the longer verse ends (beat 36), not the old end (28): {:?}", lfo_beats(&h).last());
    let a = level_a(&h);
    assert_eq!(a.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>()[..2], [(16.0, true), (36.0, false)], "the mix is held to the longer verse's end: {a:?}");
    // shortened: both stop sooner (the verse is two bars)
    let (mut h, mut live) = playing();
    assert!(live.eval(&song(4)).ok);
    h.run(&mut live, 4.0 * 3.0);
    assert!(live.eval(&song(2)).ok);
    h.run(&mut live, 4.0 * 12.0);
    assert!(lfo_beats(&h).last().copied().unwrap() <= 20.5, "the lfo ends with the shorter verse: {:?}", lfo_beats(&h).last());
}

/// R-07: `Live::status()` runs every UI frame (the window's position and status lines) and hashed every net of the program on each call
/// (about 1 ms a net, 29 ms for a 32-bar chance song in a debug build), cloned the program text and hashed the whole performance take.
/// Now the net CIDs are worked out once, when the program is evaluated, and `status` has the take's counts only (its CID is the
/// `perform` verb's); the window asks the cheap accessors (`staged_cid`, `staged_lands`, `generation`).
#[test]
fn r07_status_does_not_hash_the_nets_or_the_take_on_every_look() {
    let src = "tempo 124\nkey am\ndeck A: kick x?0.9 ~ x?0.8 ~ | snare ~ x?0.7 ~ x | hihat x*3?0.6 x ~ x | clap ~ x?0.5 ~ ~ | bass 1 ~ 3 5\ndeck B: melody <c4 e4 g4 b4> x?0.5 | arp 1 3 5 | harmony i VI III VII\nevery 1 bars: master: level 0.5 + knob(1) * 0.1\n";
    let (mut h, mut live) = playing();
    let ev = live.eval(src);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    // staged: the window's lines are cheap accessors, and they agree with what the evaluation said
    assert_eq!(live.staged_cid(), ev.cid.as_deref());
    assert_eq!(live.staged_lands(), ev.lands);
    h.run(&mut live, 4.0 * 3.0);
    assert!(live.staged_cid().is_none());
    // the nets in the status are the evaluation's, once
    let st = live.status();
    let nets = st.get("nets").unwrap();
    for (deck, cid) in &ev.nets {
        assert_eq!(nets.get(deck).and_then(JsonValue::as_str), Some(cid.as_str()), "{deck}");
    }
    assert_eq!(live.generation(), st.get("generation").and_then(JsonValue::as_f64).map(|g| g as u32));
    // the status is cheap whatever the program costs to hash: a hundred looks do not hash anything
    let t = std::time::Instant::now();
    for _ in 0..100 {
        let _ = live.status();
    }
    assert!(t.elapsed().as_millis() < 400, "a hundred looks at the status took {:?}", t.elapsed());
    // a take: `btw get` has its counts, the `perform` verb its CID
    let (mut h, mut live) = playing();
    h.set_knob = true;
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 6.0);
    let p = live.status().get("performed").cloned().expect("a take");
    assert!(p.get("take").is_none() && p.get("take_readings").and_then(JsonValue::as_f64).is_some_and(|n| n >= 1.0), "{}", p.canonical_js());
    assert!(live.perform_status().get("take").and_then(JsonValue::as_str).is_some_and(|c| c.starts_with('z')));
}

/// R-19: a UTF-8 byte-order mark at the start of a file gave "unexpected character" with an invisible caret, and a file with CR-only line
/// ends read as one line ("tempo 120deck"). A BOM is white space and a lone CR is a newline; CRLF was fine and stays so. The same
/// program is the same CID however its lines end, and an error is at the right line and column.
#[test]
fn r19_a_byte_order_mark_and_cr_only_line_ends_are_read_as_what_they_are() {
    let lf = "# a riff\ntempo 120\nkey am\ndeck A: kick x ~ x ~ # four\nat bar 2: master: level 0.5\n";
    let cid = djlive::check(lf).compiled.unwrap().cid;
    let cr = lf.replace('\n', "\r");
    let crlf = lf.replace('\n', "\r\n");
    let bom = format!("\u{feff}{lf}");
    let bom_cr = format!("\u{feff}{cr}");
    for (name, text) in [("CR", &cr), ("CRLF", &crlf), ("BOM", &bom), ("BOM and CR", &bom_cr)] {
        let c = djlive::check(text);
        assert!(c.ok(), "{name}: {}", djlive::diag::render_all(&c.diagnostics, text));
        assert_eq!(c.compiled.unwrap().cid, cid, "{name}");
    }
    // the same text is the same canonical form, so the same program fixed up by `fmt` again
    assert_eq!(djlive::check(&cr).compiled.unwrap().text, djlive::check(lf).compiled.unwrap().text);
    // errors are where they are: line 3 column 5 whichever way the lines end (a BOM is no character)
    let bad = "tempo 120\nkey am\ndeck A: kick x ~ x ~ when\n";
    for (name, text) in [("LF", bad.to_string()), ("CR", bad.replace('\n', "\r")), ("CRLF", bad.replace('\n', "\r\n")), ("BOM", format!("\u{feff}{bad}")), ("BOM and CR", format!("\u{feff}{}", bad.replace('\n', "\r")))] {
        let c = djlive::check(&text);
        let e = c.diagnostics.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("{name}: no error"));
        let (line, col) = djlive::diag::line_col(&text, e.span.start);
        assert_eq!(line, 3, "{name}: {e:?}");
        assert!(col >= 1, "{name}");
        // the rendered line is the line, without the mark or the other lines
        let r = e.render(&text);
        assert!(r.contains("3 | deck A: kick x ~ x ~ when") && !r.contains('\u{feff}') && !r.contains('\r'), "{name}: {r}");
    }
    // a BOM alone in front of an error on line 1 does not move its column
    let one = "tempo fast\n";
    let c = djlive::check(&format!("\u{feff}{one}"));
    let e = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(djlive::diag::line_col(&format!("\u{feff}{one}"), e.span.start), djlive::diag::line_col(one, djlive::check(one).diagnostics.iter().find(|d| d.is_error()).unwrap().span.start));
    // a comment ends at a CR: it does not eat the rest
    let c = djlive::check("tempo 120 # fast\rdeck A: kick x\r").compiled.unwrap();
    assert_eq!(c.text, "tempo 120\ndeck A: kick x\n");
    assert_eq!(c.notes, vec!["fast".to_string()]);
}

/// R-21: the position the runtime counts carries a nanobar of slack (`+1e-9`, so a beat a hair short of a bar line is the line), and it
/// reached the tempo sent: `100.00000001` to the deck. The tempo is rounded to six decimals.
#[test]
fn r21_the_tempo_sent_to_a_deck_has_no_nanobar_in_it() {
    let (mut h, mut live) = playing();
    assert!(live.eval("tempo 100 -> 130 over 8 bars\ndeck A: kick x ~ x ~\n").ok);
    h.run(&mut live, 4.0 * 10.0);
    assert!(h.tempos.len() > 20);
    for (beat, _, bpm) in &h.tempos {
        assert_eq!(*bpm, (bpm * 1e6).round() / 1e6, "at beat {beat}: {bpm}");
    }
    assert_eq!(h.tempos.first().map(|t| t.2), Some(100.0));
    assert_eq!(h.tempos.iter().find(|t| t.0 >= 12.0).map(|t| t.2), Some(100.0));
}

/// R-23: the performance log stopped at 262,144 readings with no word, and the saved take was the start of the take with nothing to say
/// so. Now the runtime says so once in its words, `btw get`'s `performed` has `log_full`, and the saved take is marked `truncated`
/// (only a truncated one: every other performance keeps its CID).
#[test]
fn r23_a_full_performance_log_says_so_and_the_saved_take_is_marked() {
    use djlive::perform::{Input, PerfState, Performance, MAX_READINGS};
    let mut h = Fake::default();
    h.set_knob = true;
    let mut st = PerfState::default();
    st.program_applied("zProgram", true);
    let mut n = 0usize;
    'fill: for bar in 0..10_000i64 {
        for k in 1..=128u32 {
            st.read(&mut h, bar, &Input::Knob(k));
            n += 1;
            if n == MAX_READINGS + 50 {
                break 'fill;
            }
        }
    }
    // a take of exactly the cap says nothing; the reading past it does, once
    let said: Vec<&String> = st.notes.iter().filter(|w| w.contains("the log is full")).collect();
    assert_eq!(said.len(), 1, "{:?}", st.notes);
    assert!(said[0].contains("262144") && said[0].contains("truncated"), "{}", said[0]);
    let take = st.performance().expect("a take");
    assert_eq!(take.inputs.len(), MAX_READINGS);
    assert!(take.truncated);
    assert_eq!(st.status(&[], false).get("log_full").and_then(|v| v.as_bool()), Some(true));
    // the saved object says it, and reads back saying it
    let j = take.to_json();
    assert_eq!(j.get("truncated").and_then(|v| v.as_bool()), Some(true));
    let back = Performance::from_json(&j).unwrap();
    assert!(back.truncated && back.cid() == take.cid());
    // a take that fits is as it was: no `truncated`, the same CID as before the field existed
    let small = Performance::new("zProgram", vec![djlive::perform::Reading { bar: 1, at: 0, input: "knob(1)".into(), value: 0.5 }]);
    assert!(!small.truncated && small.to_json().get("truncated").is_none());
    assert_ne!(small.cid(), take.cid());
    // a new take starts clean
    st.start();
    assert!(st.status(&[], false).get("log_full").and_then(|v| v.as_bool()) == Some(false));
}
