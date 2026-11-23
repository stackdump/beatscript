//! A program's `move` lines in an offline render (docs/PERFORM.md; docs/BEATSCRIPT.md "The offline render").
//!
//! `move bass_swap on B over 8 bars` (on the control deck or with no head) compiles to the registry call `moves {action: arm,
//! kind, deck, bars}`. Live, the booth hands that call to [`crate::perform::Moves`], which arms the move now and lands it on
//! the bar or phrase line its deck keeps. Offline the same scheduler runs on the render's own engine: [`ProgramMoves`] arms a
//! `Moves` on the frame the line fires and services it on every block of the render, with the render's clock (its frames
//! over the sample rate, never the wall clock). So the move acts on the render as it does in the booth, on the next line
//! after the bar line that armed it, and the same text is the same PCM.
//!
//! **One path a line.** The program's control net (`djlive::nets`) already plays `filter_exit` and `echo_out` as subnets
//! (the trick, the cut and the put-back on the phrase pulse), and the render applies its events. Those lines stay there:
//! this plays only the move lines the net leaves beside it (its `residual`): `loop_roll`, `bass_swap`, `phrase_sync`, a
//! move with `for N bars`, a performed move, and a `do moves cancel`. (The two readings differ by a bar: the net starts a
//! move on the bar line that fires it, the booth on the line after; NETS-ALL-THE-WAY-DOWN.md, open question 1.)
//!
//! **The clock.** The moves read the song's clock: where the render's frame is, in beats from the song's first bar line, and
//! the tempo there ([`Clock`]). Not a deck's own beat: in the render a deck has no clock before its first block, reads one
//! frame behind, and starts its beat again at every section's load (and reads 0 for up to a beat before the section's bar
//! line, while the new net waits), so a move armed near a section would wait for a line the deck's new clock reaches bars
//! later. On the song's clock a bar line is a program bar line and a phrase line is a program phrase line, the lines the
//! runtime counts its put-backs on. A deck plays or not as the engine says.
//!
//! What a move holds is put back as the runtime puts it back ("Moves put their strip back"): the strip is unmuted at the
//! first phrase line after the move has ended. `for N bars` on the line puts it back N bars after it fired instead, and
//! cancels the move if it is still in flight.
//!
//! A move that cannot act offline is not armed and is named with the reason ([`ProgramMoves::unplayed`]): `vocoder_handoff`
//! (the render has no vocoder) and `phrase_sync` with `do=seed` (a program's deck plays its own nets, not a seed).
//! `phrase_sync` with `do=cut` (the crossfader) acts, and `loop_roll` on a net deck is the stutter's beat repeat, as live.

use std::collections::{BTreeMap, BTreeSet};

use djengine::fx::tricks::{TrickArgs, TrickKind};
use djengine::json::JsonValue;
use djlive::compile::{Call, Compiled, Effect};
use djlive::diag::line_col;

use djengine::session::arrangement::Arrangement;

use crate::deck::engine::Engine;
use crate::perform::{Grid, Key, MoveHost, MoveKind, MoveSpec, Moves, PhraseAction, BEATS_PER_BAR};

/// Beats in a phrase: a move's strip is put back on the first of these lines after the move (the runtime's `PHRASE` bars).
pub const PHRASE_BEATS: f64 = djlive::runtime::PHRASE as f64 * BEATS_PER_BAR;

/// The render's clock: the song's beat at a frame (beats from its first bar line) and the tempo there. Pieces of steady tempo,
/// `(from beat, at seconds, bpm)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    sr: f64,
    pieces: Vec<(f64, f64, f64)>,
}

impl Clock {
    /// One tempo all through (the streaming render's).
    pub fn steady(bpm: f64, sr: f64) -> Clock {
        Clock { sr, pieces: vec![(0.0, 0.0, if bpm > 0.0 { bpm } else { 120.0 })] }
    }

    /// The clock of an arrangement: [`Arrangement::seconds_at`], turned round (each event's tempo holds from its beat until the
    /// next one that has one, as there).
    pub fn of_arrangement(a: &Arrangement, sr: f64) -> Clock {
        let base = if a.bpm > 0.0 { a.bpm } else { 120.0 };
        let mut c = Clock::steady(base, sr);
        let (mut secs, mut t, mut bpm) = (0.0, 0.0, base);
        for e in &a.events {
            if e.at > t {
                secs += (e.at - t) * 60.0 / bpm;
                t = e.at;
            }
            if e.bpm > 0.0 && e.bpm != bpm {
                bpm = e.bpm;
                match c.pieces.last_mut() {
                    Some(last) if last.0 == t => *last = (t, secs, bpm),
                    _ => c.pieces.push((t, secs, bpm)),
                }
            }
        }
        c
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    fn piece(&self, frame: i64) -> (f64, f64, f64) {
        let secs = frame as f64 / self.sr;
        let i = self.pieces.partition_point(|p| p.1 <= secs).max(1);
        self.pieces[i - 1]
    }

    /// The song's beat at `frame`.
    pub fn beat_at(&self, frame: i64) -> f64 {
        let (t, s, bpm) = self.piece(frame);
        t + (frame as f64 / self.sr - s) * bpm / 60.0
    }

    /// The tempo at `frame`.
    pub fn bpm_at(&self, frame: i64) -> f64 {
        self.piece(frame).2
    }
}

/// Why a move cannot act in an offline render, or None when it can.
pub fn why_not_offline(spec: &MoveSpec) -> Option<&'static str> {
    match spec.kind {
        MoveKind::VocoderHandoff => Some("the render has no vocoder (a `do vocoder` line is not in it), so there is no carrier and modulator to swap"),
        MoveKind::PhraseSync if spec.action == PhraseAction::Seed => Some("a program's deck plays its own nets, not a seed, so there is no next seed to step to (`do=cut` acts)"),
        _ => None,
    }
}

/// The move a `moves arm` call's params ask for. None when the kind is not a move.
pub fn spec_of(p: &JsonValue) -> Option<MoveSpec> {
    let kind = MoveKind::parse(p.get("kind").and_then(JsonValue::as_str)?)?;
    let mut spec = MoveSpec::new(kind, p.get("deck").and_then(JsonValue::as_str).unwrap_or("A"));
    // no `over`: 0, which `Moves::arm` reads as the move's own default
    spec.bars = p.get("bars").and_then(JsonValue::as_f64).map_or(0, |b| b.max(0.0) as u32);
    if let Some(a) = p.get("do").and_then(JsonValue::as_str).and_then(PhraseAction::parse) {
        spec.action = a;
    }
    Some(spec)
}

/// A move line the render does not play, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Unplayed {
    /// The control's key (`move vocoder_handoff on A`).
    pub key: String,
    /// 1-based, in the text that was compiled.
    pub line: usize,
    pub col: usize,
    /// The line as written.
    pub text: String,
    pub why: String,
}

/// What a line asks of the moves.
#[derive(Clone, Debug)]
enum Op {
    Arm(MoveSpec),
    /// `do moves toggle`: cancel the move in flight, else arm it.
    Toggle(MoveSpec),
    Cancel(MoveKind, String),
    CancelAll,
}

/// One line the render plays.
struct Line {
    key: String,
    op: Op,
    /// What puts it back: the strip unmuted, and with `for N bars` the move cancelled (`Control::undo`).
    undo: Vec<Call>,
    /// `for N bars`: put back where the hold ends, not on a phrase line.
    held: bool,
}

/// A line's put-back, waiting: for its move to end (then the phrase line after it), or for its beat.
#[derive(Clone, Copy, Debug)]
enum Back {
    Moving(u64),
    At(f64),
}

/// The program's move lines on an offline render's clock: see the module.
pub struct ProgramMoves {
    lines: Vec<Line>,
    /// (beat, 0 = a hold ends | 1 = the line fires, line), in the order the render meets them.
    due: Vec<(f64, u8, usize)>,
    next: usize,
    moves: Moves,
    back: BTreeMap<usize, Back>,
    clock: Clock,
    /// The keys of the lines played here (all beside the control net, so nothing plays twice).
    pub played: BTreeSet<String>,
    /// The move lines that cannot act offline, in the order of the text.
    pub unplayed: Vec<Unplayed>,
    /// What the lines did, in order: the render frame and the words (`armed move filter_exit on A: sweep from bar 5`).
    pub log: Vec<(i64, String)>,
}

impl ProgramMoves {
    /// The move lines of `c` in a render of `beats` beats (from the song's first bar line) that `net`, the program's control
    /// net, does not play (a line the net holds is left to it), on the render's `clock`.
    pub fn new(c: &Compiled, beats: f64, net: &djlive::nets::ControlNet, clock: Clock) -> ProgramMoves {
        let total = (beats / 4.0).ceil().max(0.0) as u32;
        let beside: BTreeSet<&str> = net.residual.iter().map(|(k, _)| k.as_str()).collect();
        let mut lines = Vec::new();
        let mut due = Vec::new();
        let mut played = BTreeSet::new();
        let mut unplayed = Vec::new();
        let mut said = BTreeSet::new();
        for ctl in &c.controls {
            if !beside.contains(ctl.key.as_str()) {
                continue;
            }
            // a performed line is read with every input at 1/2, as everywhere in a render
            let effect = match &ctl.effect {
                Effect::Performed(p) => match p.at_half() {
                    Some(e) => e,
                    None => continue,
                },
                e => e.clone(),
            };
            let Effect::Calls(cs) = &effect else { continue };
            let Some(call) = cs.iter().find(|c| c.noun == "moves") else { continue };
            let p = &call.params;
            let deck = || p.get("deck").and_then(JsonValue::as_str).unwrap_or("A").to_ascii_uppercase();
            let op = match p.get("action").and_then(JsonValue::as_str).unwrap_or("status") {
                "arm" => spec_of(p).map(Op::Arm),
                "toggle" => spec_of(p).map(Op::Toggle),
                "cancel" => p.get("kind").and_then(JsonValue::as_str).and_then(MoveKind::parse).map(|k| Op::Cancel(k, deck())),
                "cancel_all" => Some(Op::CancelAll),
                // `status` reads: nothing to hear
                _ => None,
            };
            let Some(op) = op else { continue };
            if let Op::Arm(s) | Op::Toggle(s) = &op {
                if let Some(why) = why_not_offline(s) {
                    let (anchor, text) = crate::beatscript_render::place_of(c, ctl);
                    if said.insert(anchor) {
                        let (line, col) = line_col(&c.source, anchor);
                        unplayed.push(Unplayed { key: ctl.key.clone(), line, col, text, why: why.to_string() });
                    }
                    continue;
                }
            }
            let li = lines.len();
            lines.push(Line { key: ctl.key.clone(), op, undo: ctl.undo.clone(), held: ctl.hold.is_some() });
            played.insert(ctl.key.clone());
            for at in crate::beatscript_render::fire_pos(ctl.timing, total) {
                let b = at.to_f64() * 4.0;
                if b >= beats {
                    continue;
                }
                due.push((b, 1, li));
                if let Some(h) = ctl.hold {
                    if let Ok(back) = at.add(h.as_bars()) {
                        due.push((back.to_f64() * 4.0, 0, li));
                    }
                }
            }
        }
        unplayed.sort_by_key(|u| (u.line, u.col));
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        ProgramMoves { lines, due, next: 0, moves: Moves::new(), back: BTreeMap::new(), clock, played, unplayed, log: Vec::new() }
    }

    /// No line acts: the render has nothing to do here (a program with no move renders as it did).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Whether the line with this control key is played here.
    pub fn plays(&self, key: &str) -> bool {
        self.played.contains(key)
    }

    /// The words for an unplayed line, by its control key (for a residual list that names it with another reason).
    pub fn why_unplayed(&self, key: &str) -> Option<&str> {
        self.unplayed.iter().find(|u| u.key == key).map(|u| u.why.as_str())
    }

    /// One pass on the render's frame `frame` (call it on every block, before the block is rendered): the put-backs whose
    /// phrase line has come, the holds that end and the lines that fire on or before this frame, then the scheduler's own
    /// service with the render's clock (`frame / sample rate` seconds). `frame_of` is the render's own beat to frame, by which
    /// it places every event.
    pub fn run(&mut self, e: &mut Engine, frame: i64, frame_of: &dyn Fn(f64) -> i64) {
        if self.lines.is_empty() {
            return;
        }
        // a bar line's put-backs, then its lines (the runtime's order)
        let backs: Vec<usize> = self.back.iter().filter(|(_, b)| matches!(b, Back::At(p) if frame_of(*p) <= frame)).map(|(i, _)| *i).collect();
        for i in backs {
            self.back.remove(&i);
            self.put_back(e, i, frame);
        }
        while let Some(&(beat, phase, i)) = self.due.get(self.next) {
            if frame_of(beat) > frame {
                break;
            }
            self.next += 1;
            if phase == 0 {
                self.put_back(e, i, frame);
            } else {
                self.fire(e, i, frame, frame_of);
            }
        }
        let now = frame as f64 / self.clock.sr;
        self.moves.service(&mut OnRender::new(e, &self.clock, frame), now);
        // a move that is over: its strip back on the first phrase line after
        let ended: Vec<usize> = self.back.iter().filter(|(_, b)| matches!(b, Back::Moving(id) if !self.moves.in_flight(*id))).map(|(i, _)| *i).collect();
        for i in ended {
            self.back.insert(i, Back::At(phrase_after(frame, frame_of)));
        }
    }

    /// The first frame after `frame` something is due: a line, a put-back, or the next action of a move in flight, so the
    /// render can end its block there and the move acts on its frame, not up to a block late.
    pub fn next_frame(&self, e: &Engine, frame: i64, frame_of: &dyn Fn(f64) -> i64) -> Option<i64> {
        if self.lines.is_empty() {
            return None;
        }
        let mut best: Option<i64> = None;
        let mut take = |f: i64| {
            if f > frame {
                best = Some(best.map_or(f, |b: i64| b.min(f)));
            }
        };
        if let Some(&(b, _, _)) = self.due.get(self.next) {
            take(frame_of(b));
        }
        for b in self.back.values() {
            if let Back::At(p) = b {
                take(frame_of(*p));
            }
        }
        let fpb = 60.0 / self.clock.bpm_at(frame) * self.clock.sr;
        for st in self.moves.state(&Reading { e, beat: OnRender::beat_seen(&self.clock, frame), bpm: self.clock.bpm_at(frame) }) {
            if let Some(left) = st.beats_left {
                take(frame + (left * fpb).ceil().max(1.0) as i64);
            }
        }
        best
    }

    fn fire(&mut self, e: &mut Engine, i: usize, frame: i64, frame_of: &dyn Fn(f64) -> i64) {
        let clock = self.clock.clone();
        let mut host = OnRender::new(e, &clock, frame);
        let op = self.lines[i].op.clone();
        let words = match &op {
            Op::Arm(s) => self.arm(&mut host, i, s.clone(), frame, frame_of),
            Op::Toggle(s) => match self.moves.find(s.kind, &s.deck) {
                Some(id) => {
                    self.moves.cancel(&mut host, id);
                    format!("cancelled move {} on {}", s.kind.name(), s.deck)
                }
                None => self.arm(&mut host, i, s.clone(), frame, frame_of),
            },
            Op::Cancel(k, d) => {
                let any = self.moves.cancel_kind(&mut host, *k, d);
                format!("{} move {} on {d}", if any { "cancelled" } else { "nothing to cancel:" }, k.name())
            }
            Op::CancelAll => {
                self.moves.cancel_all(&mut host);
                "cancelled every move".to_string()
            }
        };
        let key = self.lines[i].key.clone();
        self.log.push((frame, format!("{key}: {words}")));
    }

    fn arm(&mut self, host: &mut OnRender<'_>, i: usize, spec: MoveSpec, frame: i64, frame_of: &dyn Fn(f64) -> i64) -> String {
        let (kind, deck) = (spec.kind, spec.deck.clone());
        let held = self.lines[i].held;
        match self.moves.arm(host, spec) {
            Ok(id) => {
                if !held {
                    self.back.insert(i, Back::Moving(id));
                }
                format!("armed: {}", self.moves.last_word(kind, &deck).unwrap_or(""))
            }
            Err(why) => {
                // nothing was armed, so nothing holds the strip: it is put back on the phrase line all the same, as live
                if !held {
                    self.back.insert(i, Back::At(phrase_after(frame, frame_of)));
                }
                format!("not armed: {why}")
            }
        }
    }

    /// The calls that put line `i` back: the strip unmuted, a move still in flight cancelled.
    fn put_back(&mut self, e: &mut Engine, i: usize, frame: i64) {
        let undo = self.lines[i].undo.clone();
        for call in &undo {
            match call.noun.as_str() {
                "moves" => {
                    let p = &call.params;
                    if let Some(k) = p.get("kind").and_then(JsonValue::as_str).and_then(MoveKind::parse) {
                        let d = p.get("deck").and_then(JsonValue::as_str).unwrap_or("A").to_ascii_uppercase();
                        let clock = self.clock.clone();
                        self.moves.cancel_kind(&mut OnRender::new(&mut *e, &clock, frame), k, &d);
                    }
                }
                _ => {
                    for ev in djlive::nets::call_events(call).unwrap_or_default() {
                        let _ = crate::beatscript_render::apply_event(e, &ev);
                    }
                }
            }
        }
        let key = self.lines[i].key.clone();
        self.log.push((frame, format!("{key}: put back")));
    }
}

/// The first phrase line (a multiple of [`PHRASE_BEATS`] from the song's first bar line) whose frame is after `frame`.
fn phrase_after(frame: i64, frame_of: &dyn Fn(f64) -> i64) -> f64 {
    let mut k = 1.0;
    while frame_of(k * PHRASE_BEATS) <= frame {
        k += 1.0;
    }
    k * PHRASE_BEATS
}

/// A deck as the moves see it in a render: the song's beat and tempo at the render's frame, and the engine's word on whether
/// it plays. None for a deck the engine does not have.
fn song_grid(e: &Engine, deck: &str, beat: f64, bpm: f64) -> Option<Grid> {
    e.deck(deck).map(|d| Grid { beat, bpm, playing: d.playing })
}

/// The engine as the moves see it on a render's frame. A deck's beat is the song's, read half a frame on, so a bar line the
/// frame lands on (to the nearest frame, as every event of the render is placed) counts as crossed: live, the runtime fires a
/// line on the pass after its bar line and the move lands on the next line after that; this is the same reading on a grid of
/// frames, and it does not hang on which side of the line a rounding put the frame.
struct OnRender<'a> {
    e: &'a mut Engine,
    beat: f64,
    bpm: f64,
}

impl<'a> OnRender<'a> {
    fn new(e: &'a mut Engine, clock: &Clock, frame: i64) -> OnRender<'a> {
        OnRender { e, beat: OnRender::beat_seen(clock, frame), bpm: clock.bpm_at(frame) }
    }

    fn beat_seen(clock: &Clock, frame: i64) -> f64 {
        clock.beat_at(frame) + 0.5 * clock.bpm_at(frame) / 60.0 / clock.sr
    }
}

impl MoveHost for OnRender<'_> {
    fn move_decks(&self) -> Vec<String> {
        MoveHost::move_decks(&*self.e)
    }
    fn grid(&self, deck: &str) -> Option<Grid> {
        song_grid(&*self.e, deck, self.beat, self.bpm)
    }
    fn eq_low(&self, deck: &str) -> Option<f64> {
        MoveHost::eq_low(&*self.e, deck)
    }
    fn set_eq_low(&mut self, deck: &str, v: f64) -> bool {
        MoveHost::set_eq_low(&mut *self.e, deck, v)
    }
    fn set_mute(&mut self, deck: &str, on: bool) -> bool {
        MoveHost::set_mute(&mut *self.e, deck, on)
    }
    fn fire_trick(&mut self, deck: &str, kind: TrickKind, on: bool, args: Option<TrickArgs>) -> Result<(), String> {
        MoveHost::fire_trick(&mut *self.e, deck, kind, on, args)
    }
    fn can_loop(&self, deck: &str) -> bool {
        MoveHost::can_loop(&*self.e, deck)
    }
    fn loop_beats(&mut self, deck: &str, beats: f64) {
        MoveHost::loop_beats(&mut *self.e, deck, beats)
    }
    fn loop_halve(&mut self, deck: &str) {
        MoveHost::loop_halve(&mut *self.e, deck)
    }
    fn exit_loop(&mut self, deck: &str) {
        MoveHost::exit_loop(&mut *self.e, deck)
    }
    fn step_seed(&mut self, deck: &str) -> bool {
        MoveHost::step_seed(&mut *self.e, deck)
    }
    fn crossfade_to(&mut self, deck: &str) {
        MoveHost::crossfade_to(&mut *self.e, deck)
    }
    fn vocoder_swap(&mut self) -> Result<(), String> {
        MoveHost::vocoder_swap(&mut *self.e)
    }
    fn key(&self, deck: &str) -> Option<Key> {
        MoveHost::key(&*self.e, deck)
    }
}

/// The same, read only: what [`Moves::state`] asks (where each move is). It changes nothing, so the calls that would are
/// never made through it and do nothing.
struct Reading<'a> {
    e: &'a Engine,
    beat: f64,
    bpm: f64,
}

impl MoveHost for Reading<'_> {
    fn move_decks(&self) -> Vec<String> {
        MoveHost::move_decks(self.e)
    }
    fn grid(&self, deck: &str) -> Option<Grid> {
        song_grid(self.e, deck, self.beat, self.bpm)
    }
    fn eq_low(&self, deck: &str) -> Option<f64> {
        MoveHost::eq_low(self.e, deck)
    }
    fn set_eq_low(&mut self, _: &str, _: f64) -> bool {
        false
    }
    fn set_mute(&mut self, _: &str, _: bool) -> bool {
        false
    }
    fn fire_trick(&mut self, _: &str, _: TrickKind, _: bool, _: Option<TrickArgs>) -> Result<(), String> {
        Err("read only".into())
    }
    fn can_loop(&self, deck: &str) -> bool {
        MoveHost::can_loop(self.e, deck)
    }
    fn loop_beats(&mut self, _: &str, _: f64) {}
    fn loop_halve(&mut self, _: &str) {}
    fn exit_loop(&mut self, _: &str) {}
    fn step_seed(&mut self, _: &str) -> bool {
        false
    }
    fn crossfade_to(&mut self, _: &str) {}
    fn vocoder_swap(&mut self) -> Result<(), String> {
        Err("read only".into())
    }
    fn key(&self, deck: &str) -> Option<Key> {
        MoveHost::key(self.e, deck)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves_of(src: &str) -> ProgramMoves {
        let c = djlive::check(src).compiled.unwrap_or_else(|| panic!("{src}"));
        let net = djlive::nets::control_net(&c, 16);
        ProgramMoves::new(&c, 64.0, &net, Clock::steady(120.0, 22_050.0))
    }

    /// The net keeps the filter exit (it is one of its subnets); the bass swap is played here; the vocoder handoff and the
    /// seed step cannot act and say why.
    #[test]
    fn the_lines_that_act_and_the_ones_that_cannot() {
        let m = moves_of("tempo 120\ndeck A: kick x ~ x ~\ndeck B: bass c2\nat bar 2: control: move filter_exit on A over 2 bars\nat bar 4: move bass_swap on B over 8 bars\nat bar 6: control: move vocoder_handoff on A\nat bar 8: move phrase_sync on B\nat bar 9: do moves arm kind=phrase_sync deck=B do=cut\n");
        assert_eq!(m.lines.len(), 2, "{:?}", m.played);
        assert!(!m.plays("move filter_exit on A over 2 bars"), "the control net plays it: {:?}", m.played);
        assert_eq!(m.due.iter().map(|d| d.0).collect::<Vec<_>>(), [16.0, 36.0]);
        let words: Vec<(usize, &str)> = m.unplayed.iter().map(|u| (u.line, u.why.as_str())).collect();
        assert_eq!(words.len(), 2, "{words:?}");
        assert_eq!(words[0].0, 6);
        assert!(words[0].1.contains("no vocoder"), "{words:?}");
        assert_eq!(words[1].0, 7);
        assert!(words[1].1.contains("not a seed"), "{words:?}");
    }

    /// A filter exit with `for N bars` is beside the net (it does not take a hold on a move): played here, with its hold.
    #[test]
    fn a_held_filter_exit_is_played_here() {
        let m = moves_of("tempo 120\ndeck A: kick x ~ x ~\nat bar 2 for 4 bars: move filter_exit on A over 2 bars\n");
        assert_eq!(m.lines.len(), 1);
        assert_eq!(m.due.iter().map(|d| (d.0, d.1)).collect::<Vec<_>>(), [(8.0, 1), (24.0, 0)]);
    }

    #[test]
    fn a_hold_ends_where_it_says_and_every_fires_each_time() {
        let m = moves_of("tempo 120\ndeck A: kick x ~ x ~\nat bar 1 for 2 bars: move echo_out on A over 4 bars\nevery 4 bars: move loop_roll on A\n");
        // (beat, phase): the hold's end before the line that fires on its beat
        let d: Vec<(f64, u8)> = m.due.iter().map(|d| (d.0, d.1)).collect();
        assert_eq!(d, [(4.0, 1), (12.0, 0), (16.0, 1), (32.0, 1), (48.0, 1)]);
        assert!(m.lines[0].held && !m.lines[1].held);
    }

    #[test]
    fn a_program_without_moves_has_nothing_to_do() {
        let m = moves_of("tempo 120\ndeck A: kick x ~ x ~\nmaster: filter lp 400\n");
        assert!(m.is_empty() && m.unplayed.is_empty());
    }

    #[test]
    fn phrase_lines_are_strictly_after() {
        let f = |b: f64| (b * 1000.0).round() as i64;
        assert_eq!(phrase_after(0, &f), 32.0);
        assert_eq!(phrase_after(f(20.0), &f), 32.0);
        assert_eq!(phrase_after(f(32.0), &f), 64.0);
    }
}
