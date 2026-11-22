use super::*;
use crate::autodj::LitPair;
use crate::perform::MoveHost;
use crate::setlist::{recipe::Bank, InsertAuto, InsertKind, Tag, TransitionKind};

/// 1/32 of a beat per pass: exact in binary, so the fake's lines fall exactly on the bar.
const STEP: f64 = 0.03125;

fn ix(d: &str) -> usize {
    usize::from(d != "A")
}

/// Two decks on a fake clock (120 bpm), a log of everything the queue did, and the Master's
/// knobs.
struct Fake {
    beat: [f64; 2],
    playing: [bool; 2],
    mute: [bool; 2],
    xf: f64,
    low: [f64; 2],
    master: Vec<(String, f64)>,
    master_on: Vec<(String, bool)>,
    fail_load: bool,
    /// A pair requested and not yet in: it installs at the next bar, as `Engine::load_prepared`.
    pending_pair: Option<(String, u32)>,
    log: Vec<String>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            beat: [0.0; 2],
            playing: [true; 2],
            mute: [false; 2],
            xf: 0.0,
            low: [0.5; 2],
            master: Vec::new(),
            master_on: Vec::new(),
            fail_load: false,
            pending_pair: None,
            log: Vec::new(),
        }
    }
    fn at(&self) -> String {
        format!("@{:.2}", self.beat[0].max(self.beat[1]) / 4.0)
    }
    fn tick(&mut self) {
        let was = (self.beat[0] / 4.0).floor();
        for i in 0..2 {
            if self.playing[i] {
                self.beat[i] += STEP;
            }
        }
        if (self.beat[0] / 4.0).floor() > was {
            if let Some((spec, group)) = self.pending_pair.take() {
                let at = self.at();
                self.log.push(format!("pair in {spec} {group} {at}"));
                // the new loops start from their top, both playing
                self.beat = [0.0; 2];
                self.playing = [true; 2];
            }
        }
    }
    fn said(&self, what: &str) -> Vec<&String> {
        self.log.iter().filter(|l| l.contains(what)).collect()
    }
}

impl MoveHost for Fake {
    fn move_decks(&self) -> Vec<String> {
        vec!["A".into(), "B".into()]
    }
    fn grid(&self, d: &str) -> Option<Grid> {
        Some(Grid { beat: self.beat[ix(d)], bpm: 120.0, playing: self.playing[ix(d)] })
    }
    fn eq_low(&self, d: &str) -> Option<f64> {
        Some(self.low[ix(d)])
    }
    fn set_eq_low(&mut self, d: &str, v: f64) -> bool {
        self.low[ix(d)] = v;
        true
    }
    fn set_mute(&mut self, d: &str, on: bool) -> bool {
        self.mute[ix(d)] = on;
        let at = self.at();
        self.log.push(format!("{d} mute {on} {at}"));
        true
    }
    fn fire_trick(&mut self, d: &str, k: TrickKind, on: bool, a: Option<TrickArgs>) -> Result<(), String> {
        let bars = a.map_or(String::new(), |a| format!(" bars {}", a.bars));
        let at = self.at();
        self.log.push(format!("{d} {} {on}{bars} {at}", k.name()));
        Ok(())
    }
    fn can_loop(&self, _: &str) -> bool {
        false
    }
    fn loop_beats(&mut self, d: &str, b: f64) {
        self.log.push(format!("{d} loop {b}"));
    }
    fn loop_halve(&mut self, d: &str) {
        self.log.push(format!("{d} halve"));
    }
    fn exit_loop(&mut self, d: &str) {
        self.log.push(format!("{d} exit loop"));
    }
    fn step_seed(&mut self, d: &str) -> bool {
        self.log.push(format!("{d} seed"));
        true
    }
    fn crossfade_to(&mut self, d: &str) {
        self.xf = if d == "A" { 0.0 } else { 1.0 };
        let at = self.at();
        self.log.push(format!("xfade {d} {at}"));
    }
    fn vocoder_swap(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn key(&self, _: &str) -> Option<crate::perform::Key> {
        None
    }
}

impl AutoDjHost for Fake {
    fn crossfader(&self) -> f64 {
        self.xf
    }
    fn muted(&self, d: &str) -> bool {
        self.mute[ix(d)]
    }
    fn can_step(&self, _: &str) -> bool {
        true
    }
    fn lit_pair(&self) -> Option<LitPair> {
        None
    }
    fn load_pair(&mut self, spec: &str, group: u32) -> Result<(), String> {
        let at = self.at();
        self.log.push(format!("load_pair {spec} {group} {at}"));
        if self.playing[0] || self.playing[1] {
            // a playing deck takes it at its next bar
            self.pending_pair = Some((spec.to_string(), group));
        } else {
            self.beat = [0.0; 2];
            self.playing = [true; 2];
        }
        Ok(())
    }
}

impl QueueHost for Fake {
    fn load_deck(&mut self, d: &str, src: &Source, solo: bool) -> Result<(), String> {
        let at = self.at();
        self.log.push(format!("load {d} {} solo={solo} {at}", src.text()));
        if self.fail_load {
            return Err("nothing to compose".into());
        }
        self.beat[ix(d)] = 0.0;
        self.playing[ix(d)] = true;
        self.mute[ix(d)] = !solo;
        if solo {
            self.xf = if d == "A" { 0.0 } else { 1.0 };
        }
        Ok(())
    }
    fn sync_deck(&mut self, d: &str) {
        // locked on the bar: it takes the other deck's place in the bar
        let other = self.beat[1 - ix(d)];
        self.beat[ix(d)] = other;
        self.log.push(format!("sync {d}"));
    }
    fn master_on(&mut self, e: &str, on: bool) {
        self.master_on.push((e.into(), on));
    }
    fn master_set(&mut self, p: &str, v: f64) {
        self.master.push((p.into(), v));
    }
    fn play_all(&mut self, on: bool) {
        self.playing = [on; 2];
        self.log.push(format!("play_all {on}"));
    }
}

struct Rig {
    q: Queue,
    moves: Moves,
    h: Fake,
    now: f64,
}

impl Rig {
    fn new(h: Fake) -> Rig {
        Rig { q: Queue::new(), moves: Moves::new(), h, now: 0.0 }
    }
    /// `bars` bars on, a pass every 1/32 beat.
    fn run(&mut self, bars: f64) {
        let steps = (bars * 4.0 / STEP).round() as usize;
        for _ in 0..steps {
            self.h.tick();
            self.now += STEP * 0.5;
            self.q.service(&mut self.h, &mut self.moves);
            self.moves.service(&mut self.h, self.now);
        }
    }
    fn play(&mut self) {
        self.q.play(&mut self.h, None).unwrap();
    }
}

fn pair(spec: &str, group: u32, bars: u32, kind: TransitionKind) -> Segment {
    let mut s = Segment::new(Source::Pair { spec: spec.into(), group }, bars);
    s.transition = Transition::new(kind);
    s
}

fn set_of(name: &str, segs: Vec<Segment>) -> SetList {
    let mut s = SetList::new(name);
    s.segments = segs;
    s
}

fn mv(k: MoveKind) -> TransitionKind {
    TransitionKind::Move(k)
}

/// Three pairs of 16 bars, each handed over with a bare phrase sync.
fn bare() -> SetList {
    set_of("bare", (1..=3).map(|g| pair("dnb:1", g, 16, mv(MoveKind::PhraseSync))).collect())
}

fn bar(l: &str) -> f64 {
    l.rsplit('@').next().unwrap().parse().unwrap()
}

#[test]
fn nothing_happens_until_it_is_played() {
    let mut r = Rig::new(Fake::new());
    r.q.add(bare()).unwrap();
    r.run(40.0);
    assert!(r.h.log.is_empty(), "{:?}", r.h.log);
    assert_eq!(r.q.state(), State::Stopped);
    assert!(r.q.play(&mut r.h, Some(9)).is_err(), "no segment 10");
    assert!(Queue::new().play(&mut r.h, None).unwrap_err().contains("empty"));
}

#[test]
fn the_first_segment_starts_on_the_next_phrase_line_and_each_one_on_its_line() {
    let mut h = Fake::new();
    h.beat = [10.0, 10.0]; // bar 2.5: the next 8-bar phrase is bar 8
    let mut r = Rig::new(h);
    r.q.add(bare()).unwrap();
    r.play();
    r.run(3.0);
    assert!(r.h.said("load_pair").is_empty(), "not before the phrase line: {:?}", r.h.log);
    assert_eq!(r.q.text(), "starting at segment 1");
    r.run(3.0);
    let loads = r.h.said("load_pair");
    assert_eq!(loads.len(), 1, "{:?}", r.h.log);
    // requested a bar early (a playing deck takes a project at its next bar), in on the line
    assert!(loads[0].contains("dnb:1 1") && bar(loads[0]) >= 6.99 && bar(loads[0]) < 7.1, "{loads:?}");
    let ins = r.h.said("pair in dnb:1 1");
    assert_eq!(ins.len(), 1, "{:?}", r.h.log);
    assert!(bar(ins[0]) >= 7.99 && bar(ins[0]) < 8.1, "{ins:?}");
    assert_eq!(r.q.state(), State::Playing);
    assert_eq!(r.q.current(), Some(0));
    // 16 bars later the second, and the third: every 16 lines, within one pass of the line
    r.run(16.0);
    r.run(16.0);
    let loads = r.h.said("load_pair");
    assert_eq!(loads.len(), 3, "{:?}", r.h.log);
    assert!(loads[1].contains("dnb:1 2") && loads[2].contains("dnb:1 3"));
    assert_eq!(r.q.current(), Some(2));
    // (the fake restarts the loops from their top: a load is on the line it counts)
    assert_eq!(r.q.transitions, 3);
}

#[test]
fn a_filter_exit_builds_a_riser_and_a_sweep_on_both_decks_that_end_on_the_line() {
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("f", vec![pair("dnb:1", 1, 16, mv(MoveKind::FilterExit)), pair("dnb:1", 2, 16, mv(MoveKind::FilterExit))])).unwrap();
    r.play();
    r.run(8.0); // the phrase line is bar 8
    assert_eq!(r.h.said("load_pair").len(), 1);
    r.h.log.clear();
    // 16 bars on: the fade is 4 (the filter exit's default): the sweep from bar 12 of the segment
    r.run(11.0);
    assert!(r.h.said("riser").is_empty());
    r.run(1.0);
    assert_eq!(r.h.said("riser true bars 4").len(), 2, "both decks: {:?}", r.h.log);
    assert_eq!(r.h.said("filter true bars 4").len(), 2);
    r.run(4.5);
    assert_eq!(r.h.said("load_pair dnb:1 2").len(), 1);
    let off = r.h.said("filter false");
    assert_eq!(off.len(), 2, "{:?}", r.h.log);
    assert!(off.iter().all(|l| bar(l) < 0.2), "released on the line, as the next pair is in (the fake's loops are at their top): {off:?}");
    let ins = r.h.said("pair in dnb:1 2");
    assert_eq!(ins.len(), 1, "{:?}", r.h.log);
}

#[test]
fn a_net_segment_is_loaded_muted_on_the_idle_deck_locked_and_cut_to_on_the_line() {
    let mut h = Fake::new();
    h.playing[1] = false; // deck B idle
    let mut r = Rig::new(h);
    let mut s2 = Segment::new(Source::Net { spec: "house:7".into() }, 16);
    s2.transition = Transition::new(mv(MoveKind::PhraseSync));
    r.q.add(set_of("m", vec![pair("dnb:1", 1, 16, mv(MoveKind::BassSwap)), s2])).unwrap();
    // a pair plays on both decks; then one plays A alone for the mix to be a mix
    r.play();
    r.run(8.0);
    // (the pair is on: both decks) a bass swap hand-over into a deck source
    assert_eq!(r.h.said("load_pair").len(), 1);
    r.h.log.clear();
    r.run(16.5);
    let load = r.h.said("load B house:7");
    assert_eq!(load.len(), 1, "{:?}", r.h.log);
    assert!(load[0].contains("solo=false"));
    assert!(r.h.said("sync B").len() == 1);
    // the bass swap and the cut land on the line, B is unmuted
    assert!(!r.h.mute[1] || r.h.said("B mute false").len() == 1, "{:?}", r.h.log);
    assert!(r.h.said("xfade B").len() == 1, "{:?}", r.h.log);
    assert_eq!(r.q.current(), Some(1));
    assert!(r.q.to_json().get("now").and_then(|n| n.get("text")).and_then(|t| t.as_str()).unwrap().contains("house:7"));
}

#[test]
fn a_deck_source_with_nothing_playing_starts_at_once_and_solo() {
    let mut h = Fake::new();
    h.playing = [false; 2];
    let mut r = Rig::new(h);
    r.q.add(set_of("s", vec![Segment::new(Source::Net { spec: "house:7".into() }, 8)])).unwrap();
    r.play();
    r.run(0.5);
    assert_eq!(r.h.said("load A house:7 solo=true").len(), 1, "{:?}", r.h.log);
    assert_eq!(r.q.state(), State::Playing);
    assert_eq!(r.q.current(), Some(0));
}

#[test]
fn pause_freezes_the_count_and_play_goes_on_where_it_left_off() {
    let mut r = Rig::new(Fake::new());
    r.q.add(bare()).unwrap();
    r.play();
    r.run(8.0);
    r.run(6.0); // 6 bars into segment 1 (10 left)
    assert_eq!(r.q.bars_left(), Some(10));
    r.q.pause(&mut r.h).unwrap();
    assert_eq!(r.q.state(), State::Paused);
    assert!(!r.h.playing[0] && !r.h.playing[1]);
    r.run(40.0);
    assert_eq!(r.q.bars_left(), Some(10), "no time passes while paused");
    assert_eq!(r.h.said("load_pair").len(), 1);
    r.q.play(&mut r.h, None).unwrap();
    assert!(r.h.playing[0]);
    r.run(8.5);
    assert_eq!(r.h.said("load_pair").len(), 1);
    r.run(1.0);
    assert_eq!(r.h.said("load_pair").len(), 2, "requested a bar before the line, 10 bars after the pause point: {:?}", r.h.log);
    r.run(1.0);
    assert_eq!(r.h.said("pair in").len(), 2, "and in on the line");
    assert!(r.q.pause(&mut r.h).is_ok());
    assert!(Queue::new().pause(&mut r.h).is_err());
}

#[test]
fn stop_echoes_out_on_the_next_line_and_then_stops_the_decks() {
    let mut r = Rig::new(Fake::new());
    r.q.add(bare()).unwrap();
    r.play();
    r.run(8.0);
    r.run(3.5);
    r.h.log.clear();
    r.q.stop(&mut r.h, &mut r.moves);
    assert_eq!(r.q.state(), State::Playing, "still playing while the room rings");
    r.run(1.0);
    r.run(0.5);
    // an echo out on both decks, armed on the line
    assert!(r.moves.find(MoveKind::EchoOut, "A").is_some() || !r.h.said("wash").is_empty(), "{:?}", r.h.log);
    r.run(6.0);
    assert_eq!(r.q.state(), State::Stopped, "{:?}", r.h.log);
    assert!(!r.h.playing[0] && !r.h.playing[1], "the decks stopped");
    assert!(!r.h.mute[0] && !r.h.mute[1], "and unmuted for the next time");
    assert!(r.moves.is_idle());
    assert_eq!(r.h.said("load_pair").len(), 0, "no further segment");
    // stop while paused: at once
    r.q.add(bare()).ok();
    r.h.playing = [true; 2];
    r.play();
    r.run(9.0);
    r.q.pause(&mut r.h).unwrap();
    r.q.stop(&mut r.h, &mut r.moves);
    assert_eq!(r.q.state(), State::Stopped);
}

#[test]
fn next_skips_ahead_and_jump_goes_to_a_segment() {
    let mut r = Rig::new(Fake::new());
    r.q.add(bare()).unwrap();
    r.play();
    r.run(8.0);
    r.run(2.0);
    assert_eq!(r.q.current(), Some(0));
    r.q.next().unwrap();
    r.run(3.0);
    assert_eq!(r.q.current(), Some(1), "{:?}", r.h.log);
    assert!(r.h.said("load_pair dnb:1 2").len() == 1);
    // the new segment runs its own 16 bars from where it landed
    assert!(r.q.bars_left().unwrap() >= 14);
    // jump back to the first
    r.q.jump(0).unwrap();
    r.run(3.0);
    assert_eq!(r.q.current(), Some(0));
    assert!(r.q.jump(9).is_err());
    // jump to the last, then next past the end: the queue ends
    r.q.jump(2).unwrap();
    r.run(3.0);
    assert_eq!(r.q.current(), Some(2));
    r.q.next().unwrap();
    r.run(6.0);
    assert_eq!(r.q.state(), State::Stopped, "next on the last segment ends the set: {:?}", r.h.log);
    assert!(Queue::new().next().is_err());
}

#[test]
fn a_set_ends_with_an_echo_out_and_a_looping_queue_wraps() {
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("two", vec![pair("dnb:1", 1, 8, TransitionKind::Cut), pair("dnb:1", 2, 8, TransitionKind::Cut)])).unwrap();
    r.play();
    r.run(8.0);
    r.run(8.0 + 8.0 + 1.0);
    // the end: the last segment's 8 bars, then the echo out, then stopped
    r.run(4.0);
    assert_eq!(r.q.state(), State::Stopped, "{:?}", r.h.log);
    assert_eq!(r.h.said("load_pair").len(), 2);
    // looping: it goes back to the top
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("two", vec![pair("dnb:1", 1, 8, TransitionKind::Cut), pair("dnb:1", 2, 8, TransitionKind::Cut)])).unwrap();
    r.q.set_looping(true);
    r.play();
    r.run(8.0 + 8.0 * 5.0 + 1.0);
    let loads = r.h.said("load_pair");
    assert_eq!(loads.len(), 6, "{:?}", r.h.log);
    assert!(loads[2].contains("dnb:1 1") && loads[3].contains("dnb:1 2"), "wrapped: {loads:?}");
    assert_eq!(r.q.state(), State::Playing);
}

#[test]
fn several_sets_play_back_to_back() {
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("one", vec![pair("dnb:1", 1, 8, TransitionKind::Cut), pair("dnb:1", 2, 8, TransitionKind::Cut)])).unwrap();
    r.q.add(set_of("two", vec![pair("jungle:2", 1, 8, TransitionKind::Cut)])).unwrap();
    assert_eq!(r.q.len(), 3);
    assert_eq!(r.q.locate(2), Some((1, 0)));
    r.play();
    r.run(8.0);
    r.run(8.0 * 2.0 + 0.5);
    let loads = r.h.said("load_pair");
    assert_eq!(loads.len(), 3, "{:?}", r.h.log);
    assert!(loads[2].contains("jungle:2 1"));
    let j = r.q.to_json();
    let now = j.get("now").unwrap();
    assert_eq!(now.get("set").and_then(|v| v.as_str()), Some("two"));
    assert_eq!(now.get("number").and_then(|v| v.as_f64()), Some(3.0));
    assert!(j.get("next").unwrap().get("text").is_none(), "nothing after the last: {j:?}");
    assert_eq!(j.get("queue").and_then(|q| q.as_array()).map(|a| a.len()), Some(2));
    // empty sets are refused
    assert!(r.q.add(SetList::new("none")).is_err());
}

#[test]
fn the_master_inserts_ride_the_last_bars_and_go_home_after_the_line() {
    let mut r = Rig::new(Fake::new());
    let mut a = pair("dnb:1", 1, 16, mv(MoveKind::PhraseSync));
    a.inserts = vec![InsertAuto::new(InsertKind::FilterSweep, 4, 0.14), InsertAuto::new(InsertKind::DelayThrow, 2, 0.5)];
    r.q.add(set_of("i", vec![a, pair("dnb:1", 2, 16, mv(MoveKind::PhraseSync))])).unwrap();
    r.play();
    r.run(8.0);
    r.run(11.0);
    assert!(r.h.master.is_empty(), "nothing before the last four bars: {:?}", r.h.master);
    r.run(1.5);
    let sweep: Vec<f64> = r.h.master.iter().filter(|(p, _)| p == "filter.sweep").map(|(_, v)| *v).collect();
    assert!(sweep.len() > 4 && sweep[0] > 0.4 && sweep.windows(2).all(|w| w[1] <= w[0] + 1e-9), "a sweep down to 0.14: {sweep:?}");
    assert!(r.h.master_on.contains(&("filter".into(), true)));
    r.run(3.4);
    let last = r.h.master.iter().rev().find(|(p, _)| p == "filter.sweep").unwrap().1;
    assert!((last - 0.14).abs() < 0.03, "{last}");
    // after the line: home, and the effects off
    r.run(2.0);
    let home = r.h.master.iter().rev().find(|(p, _)| p == "filter.sweep").unwrap().1;
    assert_eq!(home, FILTER_OPEN);
    let mix = r.h.master.iter().rev().find(|(p, _)| p == "delay.mix").unwrap().1;
    assert_eq!(mix, DELAY_MIX);
    assert!(r.h.master_on.contains(&("filter".into(), false)) && r.h.master_on.contains(&("delay".into(), false)));
    assert!(r.h.master.iter().filter(|(p, _)| p == "delay.mix").map(|(_, v)| *v).fold(0.0, f64::max) > 0.45, "the throw reached its peak");
}

#[test]
fn a_failed_load_is_said_and_the_queue_carries_on() {
    let mut h = Fake::new();
    h.playing[1] = false;
    h.fail_load = true;
    let mut r = Rig::new(h);
    r.q.add(set_of("f", vec![pair("dnb:1", 1, 8, TransitionKind::Cut), Segment::new(Source::Net { spec: "x:1".into() }, 8)])).unwrap();
    r.play();
    r.run(8.0);
    r.run(8.5);
    assert_eq!(r.q.current(), Some(1));
    assert!(r.q.word.contains("nothing to compose") || r.h.said("load B x:1").len() == 1, "{}", r.q.word);
}

#[test]
fn clear_lets_go_and_the_decks_are_left_as_they_are() {
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("f", vec![pair("dnb:1", 1, 16, mv(MoveKind::FilterExit)), pair("dnb:1", 2, 16, mv(MoveKind::FilterExit))])).unwrap();
    r.play();
    r.run(8.0);
    r.run(12.5); // inside the filter exit
    assert!(r.q.running());
    r.q.clear_items(&mut r.h, &mut r.moves);
    assert_eq!(r.q.state(), State::Stopped);
    assert_eq!(r.q.len(), 0);
    assert!(r.h.playing[0] && r.h.playing[1], "playing on");
    assert!(r.h.said("filter false").len() == 2, "what it switched on was let go: {:?}", r.h.log);
}

#[test]
fn the_plan_is_pure_and_reads_whole() {
    let ctx = Ctx { playing: vec!["A".into(), "B".into()], inn: "B".into(), out: "A".into() };
    let a = pair("dnb:1", 1, 16, mv(MoveKind::EchoOut));
    let b = pair("dnb:1", 2, 16, TransitionKind::Cut);
    let p = build(Some(&a), Some(&b), &ctx);
    assert_eq!(p.lead, 2);
    let offs: Vec<i64> = p.cues.iter().map(|(o, _)| *o).collect();
    assert_eq!(offs, vec![-1, -1, -1, 0, 0, 0], "a delay throw on both decks and the load a bar ahead, off on the line, the enter: {:?}", p.cues);
    assert!(matches!(p.cues.last().unwrap().1, QOp::Enter));
    // a deck source loads two bars ahead of the lead
    let n = Segment::new(Source::Net { spec: "house:1".into() }, 16);
    let p = build(Some(&b), Some(&n), &ctx);
    assert_eq!(p.cues[0].0, -(p.lead + LOAD_AHEAD));
    assert!(matches!(&p.cues[0].1, QOp::Load { deck, solo: false, .. } if deck == "B"));
    // the fade is held under half the segment
    assert_eq!(fade_of(&Transition { kind: mv(MoveKind::FilterExit), bars: 8 }, 8), 3);
    // the end: an echo out and then the stop
    let e = build(Some(&a), None, &ctx);
    assert!(matches!(e.cues.last().unwrap().1, QOp::Halt));
    let _ = (Bank { groups: 0, bpm: 0.0, key: String::new() }, Tag::default());
}

/// The dropout of /tmp/djset.log (docs/SETLIST.md): a playing deck takes a loaded project at its
/// NEXT bar, so a pair requested on the line (or a socket round trip too late) is in a whole bar
/// late, with the exit move's mute already down. The queue asks a bar ahead: every pair is in
/// exactly one segment after the one before, never a bar late.
#[test]
fn every_pair_is_in_on_its_line_never_a_bar_late() {
    let mut r = Rig::new(Fake::new());
    r.q.add(set_of("p", (1..=4).map(|g| pair("dnb:1", g, 16, mv(MoveKind::FilterExit))).collect())).unwrap();
    r.play();
    r.run(8.0 + 16.0 * 3.0 + 2.0);
    let ins = r.h.said("pair in");
    assert_eq!(ins.len(), 4, "{:?}", r.h.log);
    for l in &ins[1..] {
        assert!((bar(l) - 16.0).abs() < 0.05, "16 bars after the one before, not 17: {l}");
    }
    // a request made on the line itself slips a bar: the fake models it, the queue never does it
    let mut h = Fake::new();
    h.beat = [15.99, 15.99];
    let before = h.beat[0] / 4.0;
    h.load_pair("x:1", 1).unwrap();
    for _ in 0..(1.0 * 4.0 / STEP) as usize {
        h.tick();
    }
    assert!(before < 4.0 && h.said("pair in").len() == 1, "requested at the end of a bar: in at the next one");
}
