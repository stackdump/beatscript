//! The set queue (beyond Swift; docs/SETLIST.md): plays [`SetList`]s back to back on the bar /
//! phrase grid the decks keep. `play`, `pause`, `stop` (an echo out), `next`, `jump` to a
//! segment, `loop`; what is on now, what is next, how many bars are left.
//!
//! It is not a second scheduler: it is Auto-DJ's, pointed at a list. It counts the grid deck's
//! bar lines with [`autodj::lines_crossed`], keeps its cues in the same [`Cue`] / [`Op`] / `Plan`
//! the Auto-DJ keeps and runs the ones that are Auto-DJ's own (a Perform move armed, a trick on
//! or off, a mute, `load_pair`) through [`autodj::exec`], so a transition shows in the Perform
//! window and `moves status` exactly as an Auto-DJ one does, and what holds (tricks, armed
//! moves) is let go by [`autodj::release`]. What the queue adds is what a list needs: loading
//! a segment's source (a net seed, a saved track), the Master inserts' ramps, and the order.
//!
//! # Timing
//!
//! A segment of `bars` bars starts on a line `S` and ends on `B = S + bars`. The transition into
//! the next one is planned `lead` bars before `B` (what the move needs: a filter exit's sweep, an
//! echo out's tail, a roll's four steps; a deck source is loaded two bars earlier still, muted, to
//! have composed) and every cue is a line relative to `B`, so it lands within one service pass
//! (~10 ms) of the line, as Auto-DJ's. The first segment starts on the next 8-bar phrase of the
//! grid deck (at once when nothing plays). A pair is **requested one bar before** `B`: a playing
//! deck takes a new project at its next bar, and compiling it takes up to half a second, so a
//! request made on the line would land a bar late (docs/SETLIST.md, "the dropout").
//!
//! # What a transition does
//!
//! | next segment | the handover on `B` | the move's effect |
//! |---|---|---|
//! | a pair (both decks) | `load_pair` of its group, requested a bar ahead, installed on the line | `filter_exit` / `bass_swap`: a riser and a high-pass sweep on both decks over the fade; `echo_out`: a delay throw into the line; `loop_roll`: a loop roll 4 → 2 → 1 → ½ beat and a riser; `phrase_sync` / `cut`: none; `vocoder_handoff`: the swap armed one line ahead |
//! | a net seed / saved track | loaded muted on the idle deck, locked on the bar, then a `bass_swap` and `phrase_sync` cut to it | `filter_exit` / `echo_out` / `loop_roll` run on the outgoing deck; `bass_swap` and `phrase_sync` are the swap and the cut alone |
//!
//! A pair has no outgoing deck to swap with, so on a pair `bass_swap` is the filter build.
//! After the last segment the queue echoes out (`loop` wraps to the first instead).

use djengine::fx::tricks::{TrickArgs, TrickKind};
use djengine::json::{JsonObject, JsonValue};

use crate::autodj::{self, AutoDjHost, Mode, Op, Style, RISER_AMOUNT};
use crate::perform::{Grid, MoveKind, MoveSpec, Moves, PhraseAction, BEATS_PER_BAR, FILTER_EXIT_AMOUNT};
use crate::setlist::{InsertAuto, InsertKind, Segment, SetList, Source, Transition, TransitionKind};

/// A set starts on a multiple of this many bars of the grid deck's own bar number.
pub const PHRASE_BARS: i64 = 8;
/// Bars before a deck source's handover that it is loaded (composed) in.
pub const LOAD_AHEAD: i64 = 2;
/// How long a stop's echo out rings, in bars.
pub const STOP_TAIL: u32 = 2;

/// What the queue needs of the booth beyond Auto-DJ's host: put a net seed or a saved track on a
/// deck, lock it, the Master inserts, and the transport.
pub trait QueueHost: AutoDjHost {
    /// A net seed / saved track onto `deck`, muted, playing from its top (not a pair:
    /// [`AutoDjHost::load_pair`] loads those).
    /// `solo`: nothing else plays, so it is the start of the set: audible, crossfaded to and
    /// playing.
    fn load_deck(&mut self, deck: &str, src: &Source, solo: bool) -> Result<(), String>;
    /// Lock `deck` to the other on the bar.
    fn sync_deck(&mut self, deck: &str);
    /// A Master insert effect ("filter", "delay") on or off.
    fn master_on(&mut self, effect: &str, on: bool);
    /// A Master insert knob ("filter.sweep", "delay.mix", "delay.feedback"), 0..1.
    fn master_set(&mut self, param: &str, v: f64);
    /// Both decks playing / paused.
    fn play_all(&mut self, on: bool);
}

/// What a cue does: Auto-DJ's ([`Op`]) or the queue's own.
#[derive(Clone, Debug, PartialEq)]
pub enum QOp {
    Core(Op),
    /// A deck source onto a deck.
    Load { deck: String, src: Source, solo: bool },
    Sync(String),
    /// The next segment is now: the queue's position moves.
    Enter,
    /// The set is over: the decks stop.
    Halt,
}

impl QOp {
    fn text(&self) -> String {
        match self {
            QOp::Core(Op::Arm(s)) => format!("{} {}", s.kind.label(), s.deck),
            QOp::Core(Op::Trick { deck, kind, on, .. }) => format!("{} {deck} {}", kind.name(), if *on { "on" } else { "off" }),
            QOp::Core(Op::Mute { deck, on }) => format!("{deck} {}", if *on { "muted" } else { "unmuted" }),
            QOp::Core(Op::LoadPair { spec, group }) => format!("load {spec} #{group}"),
            QOp::Core(Op::StepSeed(d)) => format!("{d} next seed"),
            QOp::Load { deck, src, .. } => format!("{deck} <- {}", src.text()),
            QOp::Sync(d) => format!("{d} sync"),
            QOp::Enter => "next segment".into(),
            QOp::Halt => "stop".into(),
        }
    }
}

/// What the queue knows when it plans a transition (read off the decks).
#[derive(Clone, Debug, PartialEq)]
pub struct Ctx {
    /// The decks that play (a pair's: both).
    pub playing: Vec<String>,
    /// The deck a deck source is loaded on (the one that is not the outgoing).
    pub inn: String,
    /// The deck that plays out.
    pub out: String,
}

/// A transition's cues, as lines relative to the boundary (negative: before it), with how many
/// bars before the line the first one falls.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub cues: Vec<(i64, QOp)>,
    pub lead: i64,
}

fn arm(kind: MoveKind, deck: &str, bars: u32) -> QOp {
    let mut spec = MoveSpec::new(kind, deck);
    spec.bars = bars;
    QOp::Core(Op::Arm(spec))
}

fn trick(deck: &str, kind: TrickKind, on: bool, args: Option<TrickArgs>) -> QOp {
    QOp::Core(Op::Trick { deck: deck.to_string(), kind, on, args })
}

fn riser(deck: &str, bars: i64) -> QOp {
    trick(deck, TrickKind::Riser, true, Some(TrickArgs { amount: RISER_AMOUNT, bars: bars as f64, ..TrickKind::Riser.default_args() }))
}

/// The fade of a transition from the move's bars: held under half of the segment, so its tail is
/// over before the next one starts.
pub fn fade_of(t: &Transition, seg_bars: u32) -> i64 {
    (t.bars as i64).clamp(1, 8).min(((seg_bars as i64 - 1) / 2).max(1))
}

/// The look of a transition's effects: Auto-DJ's styles, by move.
fn style_of(kind: TransitionKind) -> Option<Style> {
    match kind {
        TransitionKind::Move(MoveKind::FilterExit) | TransitionKind::Move(MoveKind::BassSwap) => Some(Style::Filter),
        TransitionKind::Move(MoveKind::EchoOut) => Some(Style::Echo),
        TransitionKind::Move(MoveKind::LoopRoll) => Some(Style::Roll),
        _ => None,
    }
}

/// The cues of the transition out of `cur` (None: the queue is starting) into `next` (None: the
/// set is over). Pure, so the tests read the whole plan.
pub fn build(cur: Option<&Segment>, next: Option<&Segment>, ctx: &Ctx) -> Built {
    let (kind, bars_of) = match cur {
        Some(c) => (c.transition.kind, c.bars),
        None => (TransitionKind::Cut, 8),
    };
    let tr = cur.map_or(Transition::new(TransitionKind::Cut), |c| c.transition);
    let fade = fade_of(&tr, bars_of);
    let step = (fade / 4).max(1);
    let span = 4 * step;
    let mut v: Vec<(i64, QOp)> = Vec::new();
    let lead;
    let Some(next) = next else {
        // the end: the room rings out from the line, then the decks stop
        // an echo out's own tail rings; anything else ends on the stop's
        let tail = if kind == TransitionKind::Move(MoveKind::EchoOut) { tr.bars.clamp(1, 8).max(STOP_TAIL) } else { STOP_TAIL } as i64;
        for d in &ctx.playing {
            v.push((-1, arm(MoveKind::EchoOut, d, tail as u32)));
        }
        v.push((tail + 1, QOp::Halt));
        return Built { cues: v, lead: 1 };
    };
    match &next.source {
        Source::Pair { spec, group } => {
            let both = &ctx.playing;
            match style_of(kind) {
                Some(Style::Filter) => {
                    for d in both {
                        v.push((-fade, riser(d, fade)));
                        let args = TrickArgs { amount: FILTER_EXIT_AMOUNT, bars: fade as f64, ..TrickKind::Filter.default_args() };
                        v.push((-fade, trick(d, TrickKind::Filter, true, Some(args))));
                        v.push((0, trick(d, TrickKind::Filter, false, None)));
                    }
                    lead = fade + 1;
                }
                Some(Style::Echo) => {
                    for d in both {
                        v.push((-1, trick(d, TrickKind::Delay, true, None)));
                        v.push((0, trick(d, TrickKind::Delay, false, None)));
                    }
                    lead = 2;
                }
                Some(_) => {
                    for d in both {
                        v.push((-span - 1, arm(MoveKind::LoopRoll, d, step as u32)));
                        v.push((-span, riser(d, span)));
                    }
                    lead = span + 1;
                }
                None => {
                    if kind == TransitionKind::Move(MoveKind::VocoderHandoff) {
                        v.push((-1, arm(MoveKind::VocoderHandoff, &ctx.out, 1)));
                    }
                    lead = 2;
                }
            }
            // requested a bar ahead: a playing deck takes a new project at its NEXT bar (`Engine::
            // load_prepared`), and the compile takes 0.06-0.5 s, so a request made on the line
            // would land a whole bar late (docs/SETLIST.md, "the dropout")
            v.push((-1, QOp::Core(Op::LoadPair { spec: spec.clone(), group: *group })));
        }
        src if ctx.playing.is_empty() => {
            // the start of a set with nothing playing: the host puts it on and plays it
            lead = 1;
            v.push((-1, QOp::Load { deck: ctx.inn.clone(), src: src.clone(), solo: true }));
        }
        src => {
            // a deck source: composed muted on the idle deck, locked on the bar, cut to on the line
            lead = match kind {
                TransitionKind::Move(MoveKind::FilterExit) => fade + 1,
                TransitionKind::Move(MoveKind::LoopRoll) => span + 1,
                _ => 2,
            };
            v.push((-(lead + LOAD_AHEAD), QOp::Load { deck: ctx.inn.clone(), src: src.clone(), solo: false }));
            v.push((-1, QOp::Sync(ctx.inn.clone())));
            v.push((-1, QOp::Core(Op::Mute { deck: ctx.inn.clone(), on: false })));
            match kind {
                TransitionKind::Move(MoveKind::FilterExit) => {
                    v.push((-fade - 1, arm(MoveKind::FilterExit, &ctx.out, fade as u32)));
                    v.push((-fade, riser(&ctx.out, fade)));
                }
                TransitionKind::Move(MoveKind::EchoOut) => v.push((-1, arm(MoveKind::EchoOut, &ctx.out, fade as u32))),
                TransitionKind::Move(MoveKind::LoopRoll) => {
                    v.push((-span - 1, arm(MoveKind::LoopRoll, &ctx.out, step as u32)));
                    v.push((-span, riser(&ctx.out, span)));
                }
                TransitionKind::Move(MoveKind::VocoderHandoff) => v.push((-1, arm(MoveKind::VocoderHandoff, &ctx.inn, 1))),
                _ => {}
            }
            // bar-relative (bars = 1): armed a line ahead, both land on the boundary
            if kind != TransitionKind::Move(MoveKind::PhraseSync) && kind != TransitionKind::Cut {
                v.push((-1, arm(MoveKind::BassSwap, &ctx.inn, 1)));
            }
            let mut cut = MoveSpec::new(MoveKind::PhraseSync, &ctx.inn);
            cut.bars = 1;
            cut.action = PhraseAction::Cut;
            v.push((-1, QOp::Core(Op::Arm(cut))));
        }
    }
    v.push((0, QOp::Enter));
    // stable: cues on one line keep the order they were pushed in
    v.sort_by_key(|(off, _)| *off);
    Built { cues: v, lead }
}

/// A Master insert ramp: rides `params` from their base to `peak` between `start` and `end`
/// (positions in bars on the queue's line count), then back to base over `back` bars, then the
/// effect goes off.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub effect: &'static str,
    /// (knob, base, peak).
    pub params: Vec<(&'static str, f64, f64)>,
    pub start: f64,
    pub end: f64,
    pub back: f64,
    on: bool,
    sent: Vec<f64>,
}

/// The delay's resting mix and feedback (what the Master's delay is set to between throws).
pub const DELAY_MIX: f64 = 0.2;
pub const DELAY_FB: f64 = 0.45;
/// The filter's open position.
pub const FILTER_OPEN: f64 = 0.5;

impl Ramp {
    /// The ramp an insert automation makes on a segment that ends on line `end`.
    pub fn of(ins: &InsertAuto, end: f64) -> Ramp {
        let (effect, params, back) = match ins.kind {
            InsertKind::FilterSweep => ("filter", vec![("filter.sweep", FILTER_OPEN, ins.amount)], 0.5),
            InsertKind::DelayThrow => (
                "delay",
                vec![("delay.mix", DELAY_MIX, ins.amount.max(DELAY_MIX)), ("delay.feedback", DELAY_FB, (DELAY_FB + ins.amount * 0.4).min(0.8))],
                1.0,
            ),
        };
        let n = params.len();
        Ramp { effect, params, start: end - ins.bars as f64, end, back, on: false, sent: vec![f64::NAN; n] }
    }

    /// Where the knobs are at position `p`; false: the ramp is over.
    fn drive<H: QueueHost>(&mut self, host: &mut H, p: f64) -> bool {
        if p < self.start {
            return true;
        }
        if !self.on {
            host.master_on(self.effect, true);
            self.on = true;
        }
        let t = if p < self.end {
            (p - self.start) / (self.end - self.start).max(1e-9)
        } else {
            1.0 - ((p - self.end) / self.back.max(1e-9)).min(1.0)
        };
        for (i, (name, base, peak)) in self.params.iter().enumerate() {
            let v = base + (peak - base) * t;
            if self.sent[i].is_nan() || (v - self.sent[i]).abs() >= 0.01 || (t == 0.0 && self.sent[i] != *base) {
                host.master_set(name, v);
                self.sent[i] = v;
            }
        }
        if p >= self.end + self.back {
            host.master_on(self.effect, false);
            return false;
        }
        true
    }

    /// Let go now: the knobs home, the effect off.
    fn release<H: QueueHost>(&mut self, host: &mut H) {
        if self.on {
            for (name, base, _) in &self.params {
                host.master_set(name, *base);
            }
            host.master_on(self.effect, false);
        }
    }
}

/// Where the queue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    /// `play` asked; waiting for the first phrase line.
    Starting,
    Playing,
    Paused,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Stopped => "stopped",
            State::Starting => "starting",
            State::Playing => "playing",
            State::Paused => "paused",
        }
    }
}

/// A transition in flight.
#[derive(Debug)]
struct Run {
    /// The segment it enters (a flat index); None: the queue ends (or stops).
    target: Option<usize>,
    boundary: i64,
    /// The cues left, as lines.
    cues: Vec<(i64, QOp)>,
    /// The deck a deck source plays on (None: a pair).
    deck: Option<String>,
}

/// `next` with nowhere to go (the end of the queue, not looping).
const END: usize = usize::MAX;

/// The queue.
#[derive(Debug)]
pub struct Queue {
    pub items: Vec<SetList>,
    pub looping: bool,
    state: State,
    /// The segment playing (a flat index across the items).
    cur: Option<usize>,
    /// The deck a deck source plays on (None: a pair, both decks).
    cur_deck: Option<String>,
    /// The line the playing segment started on.
    seg_start: i64,
    /// Bar lines counted (anchored to the grid deck's own bar number on the first look).
    line: i64,
    anchored: bool,
    /// How far through its bar the grid deck is.
    frac: f64,
    /// The grid deck and its beat at the last look.
    last: Option<(String, f64)>,
    /// The segment to start at on `play`.
    start_at: usize,
    run: Option<Run>,
    /// What still runs after a segment began: cues past its line.
    tail: Vec<(i64, QOp)>,
    /// What Auto-DJ's machinery keeps of what is armed or on (to let go of).
    held: autodj::Plan,
    ramps: Vec<Ramp>,
    /// A `next` / `jump` that has not been planned yet.
    pending: Option<usize>,
    /// Hand-overs made.
    pub transitions: u64,
    /// The last thing it did, or why it could not.
    pub word: String,
    /// Why it is not counting, if it is not.
    idle: Option<String>,
}

impl Default for Queue {
    fn default() -> Self {
        Queue::new()
    }
}

impl Queue {
    pub fn new() -> Queue {
        Queue {
            items: Vec::new(),
            looping: false,
            state: State::Stopped,
            cur: None,
            cur_deck: None,
            seg_start: 0,
            line: 0,
            anchored: false,
            frac: 0.0,
            last: None,
            start_at: 0,
            run: None,
            tail: Vec::new(),
            held: autodj::Plan {
                mode: Mode::Solo { deck: String::new() },
                style: Style::Auto,
                boundary: 0,
                cues: Vec::new(),
                ids: Vec::new(),
                held: Vec::new(),
            },
            ramps: Vec::new(),
            pending: None,
            transitions: 0,
            word: String::new(),
            idle: Some("stopped".into()),
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Something to draw (the top bar's dot).
    pub fn busy(&self) -> bool {
        self.state != State::Stopped
    }

    /// A transition is running.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    // ---- the list

    pub fn len(&self) -> usize {
        self.items.iter().map(|s| s.segments.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The segment a flat index names: (item, segment in it).
    pub fn locate(&self, flat: usize) -> Option<(usize, usize)> {
        let mut n = flat;
        for (i, s) in self.items.iter().enumerate() {
            if n < s.segments.len() {
                return Some((i, n));
            }
            n -= s.segments.len();
        }
        None
    }

    pub fn segment(&self, flat: usize) -> Option<&Segment> {
        let (i, j) = self.locate(flat)?;
        self.items[i].segments.get(j)
    }

    /// The index after `flat`: the next one, the first when looping, None at the end.
    pub fn after(&self, flat: usize) -> Option<usize> {
        if flat + 1 < self.len() {
            Some(flat + 1)
        } else if self.looping && !self.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    /// The segment playing, as a flat index.
    pub fn current(&self) -> Option<usize> {
        self.cur
    }

    /// Queue a set (it plays after what is there). Err: it has no segments.
    pub fn add(&mut self, set: SetList) -> Result<usize, String> {
        if set.segments.is_empty() {
            return Err(format!("set {:?} has no segments", set.name));
        }
        self.items.push(set);
        Ok(self.items.len())
    }

    /// Empty the queue. What plays carries on (the decks are not touched), but the queue is
    /// stopped: nothing is planned after it. [`Queue::cancel`] lets go of what was armed.
    pub fn clear_items<H: QueueHost>(&mut self, host: &mut H, moves: &mut Moves) {
        self.cancel(host, moves);
        self.items.clear();
        self.halt_now();
    }

    pub fn set_looping(&mut self, on: bool) {
        self.looping = on;
    }

    // ---- the transport

    /// Play: from the top (or `from`, a flat index) when stopped; on from where it was when
    /// paused. Err: nothing queued, or no such segment.
    pub fn play<H: QueueHost>(&mut self, host: &mut H, from: Option<usize>) -> Result<(), String> {
        if self.is_empty() {
            return Err("the queue is empty (queue add)".into());
        }
        if let Some(n) = from.filter(|n| *n >= self.len()) {
            return Err(format!("no segment {} (the queue has {})", n + 1, self.len()));
        }
        match self.state {
            State::Paused => {
                host.play_all(true);
                self.state = State::Playing;
                self.last = None;
                self.word = "resumed".into();
                if let Some(n) = from {
                    self.pending = Some(n);
                }
            }
            State::Stopped => {
                let n = from.unwrap_or(0);
                self.start_at = n;
                self.cur = None;
                self.cur_deck = None;
                self.run = None;
                self.tail.clear();
                self.pending = None;
                self.state = State::Starting;
                self.last = None;
                self.anchored = false;
                self.idle = None;
                self.word = format!("starting at segment {}", n + 1);
            }
            State::Starting => {
                if let Some(n) = from {
                    self.start_at = n;
                }
            }
            State::Playing => {
                if let Some(n) = from {
                    self.pending = Some(n);
                }
            }
        }
        Ok(())
    }

    /// Pause both decks; the count stops with them. Err: not playing.
    pub fn pause<H: QueueHost>(&mut self, host: &mut H) -> Result<(), String> {
        if self.state != State::Playing {
            return Err(format!("not playing ({})", self.state.name()));
        }
        host.play_all(false);
        self.state = State::Paused;
        self.last = None;
        self.word = "paused".into();
        Ok(())
    }

    /// Stop with an echo out: the room rings out from the next line, then the decks stop. When
    /// paused or not yet started, at once. What was armed or held is let go.
    pub fn stop<H: QueueHost>(&mut self, host: &mut H, moves: &mut Moves) {
        match self.state {
            State::Stopped => {}
            State::Starting | State::Paused => {
                self.cancel(host, moves);
                self.halt_now();
                self.word = "stopped".into();
            }
            State::Playing => {
                self.cancel(host, moves);
                let ctx = self.ctx(host);
                let at = self.line + 1;
                let mut cues: Vec<(i64, QOp)> = ctx.playing.iter().map(|d| (at, arm(MoveKind::EchoOut, d, STOP_TAIL))).collect();
                cues.push((at + STOP_TAIL as i64 + 1, QOp::Halt));
                self.run = Some(Run { target: None, boundary: at, cues, deck: None });
                self.word = "stopping: echo out".into();
            }
        }
    }

    fn halt_now(&mut self) {
        self.state = State::Stopped;
        self.cur = None;
        self.cur_deck = None;
        self.run = None;
        self.tail.clear();
        self.pending = None;
        self.idle = Some("stopped".into());
        self.last = None;
    }

    /// Skip to the next segment: the transition runs now, on the next lines. At the end of the
    /// queue (not looping) it ends the set. Err: nothing is playing.
    pub fn next(&mut self) -> Result<(), String> {
        let Some(cur) = self.cur.filter(|_| self.state != State::Stopped) else {
            return Err("nothing is playing".into());
        };
        self.pending = Some(self.after(cur).unwrap_or(END));
        Ok(())
    }

    /// Jump to segment `n` (0-based, across the queue's sets): the transition runs now.
    pub fn jump(&mut self, n: usize) -> Result<(), String> {
        if n >= self.len() {
            return Err(format!("no segment {} (the queue has {})", n + 1, self.len()));
        }
        match self.state {
            State::Stopped => Err("not playing (queue play)".into()),
            State::Starting => {
                self.start_at = n;
                Ok(())
            }
            _ => {
                self.pending = Some(n);
                Ok(())
            }
        }
    }

    /// Let go of everything held: tricks off, moves cancelled, the Master inserts home. The
    /// position stays; what was planned is dropped.
    pub fn cancel<H: QueueHost>(&mut self, host: &mut H, moves: &mut Moves) {
        autodj::release(host, moves, &self.held);
        self.held.ids.clear();
        self.held.held.clear();
        for r in &mut self.ramps {
            r.release(host);
        }
        self.ramps.clear();
        self.run = None;
        self.tail.clear();
        self.pending = None;
    }

    // ---- reading the decks

    fn playing<H: QueueHost>(host: &H, d: &str) -> bool {
        host.grid(d).is_some_and(|g: Grid| g.playing && g.bpm > 0.0)
    }

    fn pick_grid<H: QueueHost>(&self, host: &H) -> Option<String> {
        if let Some(d) = self.last.as_ref().map(|(d, _)| d).filter(|d| Self::playing(host, d)) {
            return Some(d.clone());
        }
        if let Some(d) = self.cur_deck.as_ref().filter(|d| Self::playing(host, d)) {
            return Some(d.clone());
        }
        let mut order = host.move_decks();
        let favoured = if host.crossfader() <= 0.5 { 0 } else { 1 };
        if favoured < order.len() {
            let f = order.remove(favoured);
            order.insert(0, f);
        }
        order.into_iter().find(|d| Self::playing(host, d))
    }

    /// The decks as a transition is planned.
    fn ctx<H: QueueHost>(&self, host: &H) -> Ctx {
        let decks = host.move_decks();
        let playing: Vec<String> = decks.iter().filter(|d| Self::playing(host, d)).cloned().collect();
        let favoured = if host.crossfader() <= 0.5 { 0 } else { 1 };
        let pick = |i: usize| decks.get(i).cloned().unwrap_or_default();
        let (out, inn) = match &self.cur_deck {
            // a deck source plays on one deck: the other takes the next
            Some(d) => (d.clone(), decks.iter().find(|x| *x != d).cloned().unwrap_or_default()),
            None => match playing.as_slice() {
                // one plays: the idle one takes the new source
                [one] => (one.clone(), decks.iter().find(|x| *x != one).cloned().unwrap_or_default()),
                // none: the first deck
                [] => (pick(0), pick(0)),
                // a pair (both): the deck the crossfader favours plays out, the other takes it
                _ => (pick(favoured), pick(1 - favoured.min(1))),
            },
        };
        Ctx { playing, inn, out }
    }

    // ---- planning

    /// Plan the transition into `target` (None: the end). `at_least`: the earliest boundary;
    /// `force`: take it even though the segment has bars left (a skip).
    fn plan<H: QueueHost>(&mut self, host: &H, target: Option<usize>, at_least: i64, force: bool) {
        let ctx = self.ctx(host);
        let next = target.and_then(|t| self.segment(t)).cloned();
        let cur = self.cur.and_then(|c| self.segment(c)).cloned();
        let built = build(cur.as_ref(), next.as_ref(), &ctx);
        let natural = cur.as_ref().map_or(self.line, |c| self.seg_start + c.bars as i64);
        let boundary = if force { at_least } else { natural.max(at_least) };
        let deck = match &next {
            Some(n) if !matches!(n.source, Source::Pair { .. }) => Some(ctx.inn.clone()),
            _ => None,
        };
        self.word = format!(
            "{} → {} (in {} bars)",
            cur.as_ref().map_or("start".to_string(), |c| c.transition.text()),
            next.as_ref().map_or("end".to_string(), |n| n.source.text()),
            (boundary - self.line).max(0),
        );
        self.run = Some(Run { target, boundary, cues: built.cues.into_iter().map(|(o, q)| (boundary + o, q)).collect(), deck });
    }

    /// How many bars before its line a transition into `target` has to start.
    fn lead_for<H: QueueHost>(&self, host: &H, target: Option<usize>) -> i64 {
        let ctx = self.ctx(host);
        let next = target.and_then(|t| self.segment(t));
        let cur = self.cur.and_then(|c| self.segment(c));
        let b = build(cur, next, &ctx);
        // the first cue's own lead (a deck source loads ahead of it)
        -b.cues.iter().map(|(o, _)| *o).min().unwrap_or(0).min(0)
    }

    /// Run one pass: count the grid deck's bar lines, plan a transition when its lead is due,
    /// fire every cue whose line has come, ride the Master inserts.
    pub fn service<H: QueueHost>(&mut self, host: &mut H, moves: &mut Moves) {
        match self.state {
            State::Stopped => return,
            State::Paused => {
                self.last = None;
                return;
            }
            _ => {}
        }
        // ---- the clock
        let grid = self.pick_grid(host);
        let mut beat = None;
        if let Some(g) = &grid {
            let b = host.grid(g).map_or(0.0, |x| x.beat);
            beat = Some(b);
            if !self.anchored {
                self.line = (b / BEATS_PER_BAR + 1e-9).floor() as i64;
                self.anchored = true;
            } else if let Some((d, prev)) = &self.last {
                if d == g {
                    self.line += autodj::lines_crossed(*prev, b);
                }
            }
            self.last = Some((g.clone(), b));
            self.frac = (b / BEATS_PER_BAR).fract().max(0.0);
            self.idle = None;
        } else {
            self.last = None;
            if self.state == State::Playing {
                self.idle = Some("no deck playing".into());
            }
        }
        // ---- a `next` / `jump` asked for: drop what was planned, plan it now
        if let Some(to) = self.pending.take() {
            if self.state == State::Playing && self.cur.is_some() {
                autodj::release(host, moves, &self.held);
                self.held.ids.clear();
                self.held.held.clear();
                self.run = None;
                let target = (to != END).then_some(to);
                // the boundary after the lead it needs, and after the lines it started on
                let lead = self.lead_for(host, target);
                self.plan(host, target, self.line + lead.max(1) + 1, true);
                if let Some(r) = &self.run {
                    // the segment is shortened to the new boundary: the inserts of the old one go
                    let b = r.boundary;
                    for ramp in &mut self.ramps {
                        ramp.release(host);
                    }
                    self.ramps.clear();
                    let _ = b;
                }
            } else if self.state == State::Starting {
                self.start_at = to.min(self.len().saturating_sub(1));
            }
        }
        // ---- starting
        if self.state == State::Starting && self.run.is_none() {
            let target = Some(self.start_at.min(self.len().saturating_sub(1)));
            if grid.is_none() {
                self.plan(host, target, self.line, false);
            } else {
                let lead = self.lead_for(host, target);
                let mut b = (self.line.div_euclid(PHRASE_BARS) + 1) * PHRASE_BARS;
                while b - lead <= self.line {
                    b += PHRASE_BARS;
                }
                self.plan(host, target, b, false);
            }
        }
        // ---- the natural end of the segment: plan the transition when its lead is due
        if self.state == State::Playing && self.run.is_none() && grid.is_some() {
            if let Some(seg) = self.cur.and_then(|c| self.segment(c)) {
                let b = self.seg_start + seg.bars as i64;
                let target = self.cur.and_then(|c| self.after(c));
                let lead = self.lead_for(host, target);
                if self.line >= b - lead {
                    self.plan(host, target, 0, false);
                }
            }
        }
        // ---- fire
        let mut said: Vec<String> = Vec::new();
        let line = self.line;
        let mut tail = std::mem::take(&mut self.tail);
        let mut i = 0;
        while i < tail.len() {
            if tail[i].0 <= line {
                let (_, op) = tail.remove(i);
                said.push(self.exec(host, moves, op, None));
            } else {
                i += 1;
            }
        }
        // (a cue may have added to the tail)
        tail.append(&mut self.tail);
        self.tail = tail;
        while self.run.as_ref().is_some_and(|r| r.cues.first().is_some_and(|(at, _)| *at <= line)) {
            let (op, deck, boundary, target) = {
                let r = self.run.as_mut().unwrap();
                let (_, op) = r.cues.remove(0);
                (op, r.deck.clone(), r.boundary, r.target)
            };
            if op == QOp::Enter {
                if let Some(t) = target {
                    // what is left runs on past the line
                    if let Some(r) = self.run.take() {
                        self.tail.extend(r.cues);
                    }
                    self.enter(t, boundary, deck);
                    said.push(op.text());
                    break;
                }
            }
            said.push(self.exec(host, moves, op, deck.as_deref()));
            if self.state == State::Stopped {
                return;
            }
        }
        if !said.is_empty() {
            self.word = said.join(", ");
        }
        // an ending run with nothing left is done (the Halt did it)
        if self.run.as_ref().is_some_and(|r| r.cues.is_empty()) {
            self.run = None;
        }
        if self.run.is_none() && self.tail.is_empty() {
            self.held.ids.clear();
        }
        // ---- the Master inserts
        if let Some(_b) = beat {
            let p = self.line as f64 + self.frac;
            let mut ramps = std::mem::take(&mut self.ramps);
            ramps.retain_mut(|r| r.drive(host, p));
            self.ramps = ramps;
        }
    }

    fn exec<H: QueueHost>(&mut self, host: &mut H, moves: &mut Moves, op: QOp, deck: Option<&str>) -> String {
        let said = op.text();
        match op {
            QOp::Core(op) => {
                let (text, rebase) = autodj::exec(host, moves, &mut self.held, op);
                if rebase {
                    // the grid deck's clock started over: not a line
                    self.last = None;
                }
                text
            }
            QOp::Load { deck, src, solo } => match host.load_deck(&deck, &src, solo) {
                Ok(()) => {
                    if solo {
                        self.last = None;
                    }
                    said
                }
                Err(e) => format!("{said}: {e}"),
            },
            QOp::Sync(d) => {
                host.sync_deck(&d);
                said
            }
            QOp::Enter => {
                let _ = deck;
                said
            }
            QOp::Halt => {
                host.play_all(false);
                for d in host.move_decks() {
                    host.set_mute(&d, false);
                }
                autodj::release(host, moves, &self.held);
                for r in &mut self.ramps {
                    r.release(host);
                }
                self.ramps.clear();
                self.halt_now();
                self.held.ids.clear();
                self.held.held.clear();
                self.transitions += 1;
                "end: stopped".into()
            }
        }
    }

    /// The queue's position moved to `target`, which began on `boundary`.
    fn enter(&mut self, target: usize, boundary: i64, deck: Option<String>) {
        self.cur = Some(target);
        self.seg_start = boundary;
        self.state = State::Playing;
        self.transitions += 1;
        self.cur_deck = deck;
        self.idle = None;
        if self.last.is_none() || self.cur_deck.is_some() {
            // the grid may have moved to another deck
        }
        if let Some(seg) = self.segment(target).cloned() {
            for ins in &seg.inserts {
                self.ramps.push(Ramp::of(ins, (boundary + seg.bars as i64) as f64));
            }
        }
    }

    // ---- reading

    /// Bars to the end of the playing segment.
    pub fn bars_left(&self) -> Option<i64> {
        if self.state != State::Playing && self.state != State::Paused {
            return None;
        }
        let seg = self.cur.and_then(|c| self.segment(c))?;
        Some((self.seg_start + seg.bars as i64 - self.line).max(0))
    }

    /// "now / next" in words.
    pub fn text(&self) -> String {
        match self.state {
            State::Stopped => "stopped".into(),
            State::Starting => format!("starting at segment {}", self.start_at + 1),
            _ => {
                let Some(cur) = self.cur else { return self.state.name().into() };
                let now = self.segment(cur).map_or(String::new(), |s| s.source.text());
                let next = self.after(cur).and_then(|n| self.segment(n)).map_or("end".to_string(), |s| s.source.text());
                format!("{} {}/{}: {now} · {} bars left · next {next}", self.state.name(), cur + 1, self.len(), self.bars_left().unwrap_or(0))
            }
        }
    }

    fn seg_json(&self, flat: usize) -> JsonValue {
        let mut o = JsonObject::new();
        if let (Some((i, j)), Some(seg)) = (self.locate(flat), self.segment(flat)) {
            o.set("number", JsonValue::int(flat as i64 + 1));
            o.set("set", self.items[i].name.clone().into());
            o.set("set_number", JsonValue::int(i as i64 + 1));
            o.set("index_in_set", JsonValue::int(j as i64 + 1));
            o.set("source", seg.source.to_json());
            o.set("text", seg.source.text().into());
            o.set("bars", JsonValue::int(seg.bars as i64));
            o.set("transition", seg.transition.to_json());
            o.set("tag", seg.tag.to_json());
        }
        JsonValue::Object(o)
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("state", self.state.name().into());
        o.set("loop", self.looping.into());
        o.set("text", self.text().into());
        o.set("word", self.word.clone().into());
        o.set("idle", self.idle.clone().map_or(JsonValue::Null, JsonValue::str));
        o.set("transitions", JsonValue::int(self.transitions as i64));
        o.set("segments", JsonValue::int(self.len() as i64));
        o.set(
            "queue",
            JsonValue::Array(
                self.items
                    .iter()
                    .map(|s| {
                        let mut r = JsonObject::new();
                        r.set("cid", s.cid().into());
                        r.set("name", s.name.clone().into());
                        r.set("segments", JsonValue::int(s.segments.len() as i64));
                        r.set("bars", JsonValue::int(s.total_bars() as i64));
                        JsonValue::Object(r)
                    })
                    .collect(),
            ),
        );
        match self.cur {
            Some(c) if self.state != State::Stopped => {
                o.set("now", self.seg_json(c));
                o.set("next", self.after(c).map_or(JsonValue::Null, |n| self.seg_json(n)));
                o.set("bars_left", self.bars_left().map_or(JsonValue::Null, JsonValue::int));
                o.set(
                    "bars_in",
                    JsonValue::int((self.line - self.seg_start).max(0)),
                );
            }
            _ => {
                o.set("now", JsonValue::Null);
                o.set("next", if self.state == State::Starting { self.seg_json(self.start_at) } else { JsonValue::Null });
                o.set("bars_left", JsonValue::Null);
            }
        }
        match &self.run {
            Some(r) => {
                o.set("transition", format!("{} cues left, on line in {} bars", r.cues.len(), (r.boundary - self.line).max(0)).into());
                o.set("handover_in", JsonValue::int((r.boundary - self.line).max(0)));
            }
            None => o.set("transition", JsonValue::Null),
        }
        JsonValue::Object(o)
    }
}

#[cfg(test)]
mod tests;
