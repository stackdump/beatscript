//! Perform: DJ performance moves (beyond Swift; docs/PERFORM.md): the classic two-deck moves a
//! DJ makes by hand, each armed now and landing on the bar / phrase grid the decks already keep.
//!
//! Not to be confused with `djengine::fx::tricks` (the inserts on one strip: stutter, delay,
//! wash, filter, ...). A move here is a small schedule over existing engine primitives, the
//! inserts among them:
//!
//! | move | at arm | on its line | after |
//! |---|---|---|---|
//! | `bass_swap` | the incoming deck's low EQ cut | the lows swapped between the decks on the same pass | |
//! | `phrase_sync` | | a seed step (one bar early, so it lands on the line, as auto-next does) or a crossfader cut | |
//! | `filter_exit` | | the `filter` trick rides a high-pass over `bars` from the bar | the deck muted on the downbeat, the filter back open behind it |
//! | `echo_out` | | the `wash` trick on the one: the dry gone in a 16th, the room ringing | the deck muted `bars` later, the wash off behind it |
//! | `loop_roll` | | a 4-beat loop from the bar (a sample deck with a grid, or a net deck) | halved every `bars` (4 → 2 → 1 → ½ beat), released on the next line after the last (the drop) |
//! | `vocoder_handoff` | | carrier and modulator swapped (`Engine::vocoder_swap`) on the phrase line | |
//!
//! `harmonic` is read-only: whether the two decks' keys mix (same, a fifth, relative).
//!
//! Everything goes through [`MoveHost`], which `djrust`'s `Engine` implements below, so the
//! scheduler is pure: the window, the control socket, the FFI (djandroid) and the tests (a fake
//! host with a fake clock) all drive the same [`Moves`]: [`Moves::arm`], [`Moves::cancel`],
//! [`Moves::service`] (each service pass, with a monotonic time), [`Moves::state`] /
//! [`Moves::to_json`].
//!
//! Timing: a line is crossed when the grid deck's beat reaches it, so a move lands within one
//! service pass (~10 ms in djapp) of the line, and every action of one pass lands together (a
//! bass swap's two EQ moves are one pass). A stopped grid fires at once, as a trick does.

use std::collections::BTreeMap;

use djengine::fx::tricks::{TrickArgs, TrickKind};
use djengine::json::{JsonObject, JsonValue};

/// Beats in a bar (the decks' grids are 4/4: `tick / 16`).
pub const BEATS_PER_BAR: f64 = 4.0;

/// The phrase lengths offered for a phrase-quantised move.
pub const PHRASES: [u32; 3] = [8, 16, 32];
/// The lengths offered for a move that runs over bars (a sweep, a tail, a roll's step).
pub const RUNS: [u32; 4] = [1, 2, 4, 8];

/// The loop roll's stages on a deck that loops (a sample deck with a grid, or a net deck), in
/// beats.
pub const ROLL_BEATS: [f64; 4] = [4.0, 2.0, 1.0, 0.5];

/// How far into the high-pass the filter exit rides (the one-knob filter: right of centre is a
/// high-pass, 20 Hz .. 10 kHz exponential).
pub const FILTER_EXIT_AMOUNT: f64 = 0.9;
/// The wash's dry fade on an echo out, in beats: short enough to be the cut.
pub const ECHO_CUT_BEATS: f64 = 0.25;

/// A move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MoveKind {
    BassSwap,
    PhraseSync,
    FilterExit,
    EchoOut,
    LoopRoll,
    VocoderHandoff,
}

impl MoveKind {
    pub const ALL: [MoveKind; 6] = [
        MoveKind::BassSwap,
        MoveKind::PhraseSync,
        MoveKind::FilterExit,
        MoveKind::EchoOut,
        MoveKind::LoopRoll,
        MoveKind::VocoderHandoff,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MoveKind::BassSwap => "bass_swap",
            MoveKind::PhraseSync => "phrase_sync",
            MoveKind::FilterExit => "filter_exit",
            MoveKind::EchoOut => "echo_out",
            MoveKind::LoopRoll => "loop_roll",
            MoveKind::VocoderHandoff => "vocoder_handoff",
        }
    }

    pub fn parse(s: &str) -> Option<MoveKind> {
        let s = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        MoveKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The button's words.
    pub fn label(self) -> &'static str {
        match self {
            MoveKind::BassSwap => "Bass swap",
            MoveKind::PhraseSync => "Phrase sync",
            MoveKind::FilterExit => "Filter exit",
            MoveKind::EchoOut => "Echo out",
            MoveKind::LoopRoll => "Loop roll",
            MoveKind::VocoderHandoff => "Vocoder handoff",
        }
    }

    /// What `bars` means for this move.
    pub fn bars_meaning(self) -> &'static str {
        match self {
            MoveKind::BassSwap | MoveKind::PhraseSync | MoveKind::VocoderHandoff => "phrase (bars)",
            MoveKind::FilterExit => "sweep (bars)",
            MoveKind::EchoOut => "tail (bars)",
            MoveKind::LoopRoll => "bars per step",
        }
    }

    /// The `bars` a move offers.
    pub fn bar_choices(self) -> &'static [u32] {
        match self {
            MoveKind::BassSwap | MoveKind::PhraseSync | MoveKind::VocoderHandoff => &PHRASES,
            _ => &RUNS,
        }
    }

    pub fn default_bars(self) -> u32 {
        match self {
            MoveKind::BassSwap | MoveKind::VocoderHandoff => 8,
            MoveKind::PhraseSync => 16,
            MoveKind::FilterExit | MoveKind::EchoOut => 4,
            MoveKind::LoopRoll => 1,
        }
    }

    /// Whether the move names a deck (the vocoder handoff's deck is only the grid it waits on).
    pub fn one_at_a_time(self) -> bool {
        matches!(self, MoveKind::BassSwap | MoveKind::VocoderHandoff)
    }
}

/// What a phrase sync does on its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PhraseAction {
    /// The deck's next seed, composed one bar early so it comes in on the line
    /// (`Engine::step_seed`, as auto-next steps).
    #[default]
    Seed,
    /// The crossfader cut over to the deck.
    Cut,
}

impl PhraseAction {
    pub fn name(self) -> &'static str {
        match self {
            PhraseAction::Seed => "seed",
            PhraseAction::Cut => "cut",
        }
    }
    pub fn parse(s: &str) -> Option<PhraseAction> {
        match s.trim().to_ascii_lowercase().as_str() {
            "seed" | "next" | "step" => Some(PhraseAction::Seed),
            "cut" | "xfade" | "crossfader" | "transition" => Some(PhraseAction::Cut),
            _ => None,
        }
    }
}

/// A move to arm.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveSpec {
    pub kind: MoveKind,
    /// The deck it acts on: the incoming deck of a bass swap, the deck a phrase sync steps or
    /// cuts to, the one a filter exit / echo out / roll plays on, the grid of a vocoder handoff.
    pub deck: String,
    /// See [`MoveKind::bars_meaning`].
    pub bars: u32,
    /// A phrase sync's action.
    pub action: PhraseAction,
}

impl MoveSpec {
    pub fn new(kind: MoveKind, deck: &str) -> MoveSpec {
        MoveSpec {
            kind,
            deck: deck.to_ascii_uppercase(),
            bars: kind.default_bars(),
            action: PhraseAction::default(),
        }
    }
}

/// A deck's place on its grid, as the host reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Beats from the grid's origin (beat 0 = the downbeat of bar 0).
    pub beat: f64,
    pub bpm: f64,
    pub playing: bool,
}

/// A key: pitch class (0 = C) and mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub pc: u8,
    pub minor: bool,
}

impl Key {
    pub fn name(self) -> String {
        format!(
            "{}{}",
            djengine::musicality::loop_key::NAMES[self.pc as usize % 12],
            if self.minor { "m" } else { "" }
        )
    }
}

/// How two keys mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    Same,
    /// A fifth up or down, same mode (a Camelot neighbour).
    Fifth,
    /// Relative major / minor (the same notes).
    Relative,
    Clash,
}

impl Compat {
    pub fn name(self) -> &'static str {
        match self {
            Compat::Same => "same",
            Compat::Fifth => "fifth",
            Compat::Relative => "relative",
            Compat::Clash => "clash",
        }
    }
    pub fn mixes(self) -> bool {
        self != Compat::Clash
    }
}

/// Whether two keys mix: the same key, a fifth apart in the same mode, or relative major /
/// minor.
pub fn harmonic(a: Key, b: Key) -> Compat {
    let d = (b.pc as i32 - a.pc as i32).rem_euclid(12);
    if a.minor == b.minor {
        match d {
            0 => Compat::Same,
            5 | 7 => Compat::Fifth,
            _ => Compat::Clash,
        }
    } else {
        // the relative minor is a minor third below its major
        let (maj, min) = if a.minor { (b.pc, a.pc) } else { (a.pc, b.pc) };
        if (maj as i32 - min as i32).rem_euclid(12) == 3 {
            Compat::Relative
        } else {
            Compat::Clash
        }
    }
}

/// The harmonic hint: both decks' keys and how they mix (None where a key is unknown).
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub a: Option<Key>,
    pub b: Option<Key>,
    pub compat: Option<Compat>,
}

impl Hint {
    pub fn to_json(&self) -> JsonValue {
        let key = |k: Option<Key>| k.map_or(JsonValue::Null, |k| k.name().into());
        let mut o = JsonObject::new();
        o.set("a", key(self.a));
        o.set("b", key(self.b));
        o.set("compat", self.compat.map_or(JsonValue::Null, |c| c.name().into()));
        o.set("mixes", self.compat.map_or(JsonValue::Null, |c| c.mixes().into()));
        JsonValue::Object(o)
    }
}

/// What the moves need of the engine. `Engine` implements it; the tests fake it.
pub trait MoveHost {
    /// The decks moves can act on, in order (A, B).
    fn move_decks(&self) -> Vec<String>;
    /// Where the deck is on its grid; None: no clock (empty, or a sample with no grid).
    fn grid(&self, deck: &str) -> Option<Grid>;
    fn eq_low(&self, deck: &str) -> Option<f64>;
    fn set_eq_low(&mut self, deck: &str, v: f64) -> bool;
    fn set_mute(&mut self, deck: &str, on: bool) -> bool;
    /// An insert trick on or off, at once (the move has done the timing).
    fn fire_trick(&mut self, deck: &str, kind: TrickKind, on: bool, args: Option<TrickArgs>) -> Result<(), String>;
    /// The deck can loop on its beat: a sample deck with a beat grid, or a net deck
    /// (`Engine::can_loop`).
    fn can_loop(&self, deck: &str) -> bool;
    fn loop_beats(&mut self, deck: &str, beats: f64);
    fn loop_halve(&mut self, deck: &str);
    fn exit_loop(&mut self, deck: &str);
    /// The deck's next seed (a composed net); false: nothing to step.
    fn step_seed(&mut self, deck: &str) -> bool;
    /// The crossfader over to the deck's side.
    fn crossfade_to(&mut self, deck: &str);
    fn vocoder_swap(&mut self) -> Result<(), String>;
    fn key(&self, deck: &str) -> Option<Key>;
}

/// Where a move is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Waiting for its line.
    Armed,
    /// Fired, with more to do.
    Running,
}

/// One move in flight.
#[derive(Clone, Debug)]
struct Active {
    id: u64,
    spec: MoveSpec,
    phase: Phase,
    /// The deck whose grid times it (None: fires on the next service pass).
    grid: Option<String>,
    /// The next beat (on the grid deck) it acts on.
    at: f64,
    /// A roll's stage (0-based), counted in its own beats.
    stage: usize,
    /// Beats since a roll started, on the move's own clock (the looping deck's beat wraps).
    elapsed: f64,
    last_now: Option<f64>,
    /// A bass swap: the incoming deck's low EQ before the cut.
    saved_low: f64,
    /// A roll on a deck that cannot loop (a sample with no grid) is a beat repeat (the
    /// `stutter` trick).
    stutter: bool,
}

/// What a move in flight looks like from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveState {
    pub id: u64,
    pub spec: MoveSpec,
    pub phase: Phase,
    /// Beats left to its next action (None: a stopped grid, at once).
    pub beats_left: Option<f64>,
    /// In words: "armed: bar 33", "sweeping: cut on bar 37", ...
    pub text: String,
}

/// The moves: armed, running and the last word on each.
#[derive(Debug, Default)]
pub struct Moves {
    active: Vec<Active>,
    next_id: u64,
    /// The last thing each move did or why it could not (by kind and deck).
    pub last: BTreeMap<(MoveKind, String), String>,
}

/// The first multiple of `period` beats strictly after `beat`.
pub fn next_line(beat: f64, period: f64) -> f64 {
    let n = (beat / period + 1e-9).floor() + 1.0;
    n * period
}

fn other(host: &dyn MoveHost, deck: &str) -> Option<String> {
    host.move_decks().into_iter().find(|d| d != deck)
}

/// The deck to time on: `first` if its grid plays, else `then` if its does, else None (at once).
fn pick_grid(host: &dyn MoveHost, first: &str, then: Option<&str>) -> Option<String> {
    let plays = |d: &str| host.grid(d).is_some_and(|g| g.playing && g.bpm > 0.0);
    if plays(first) {
        Some(first.to_string())
    } else {
        then.filter(|d| plays(d)).map(str::to_string)
    }
}

impl Moves {
    pub fn new() -> Moves {
        Moves::default()
    }

    /// Arm a move. One of each kind per deck (a bass swap and a vocoder handoff: one at all):
    /// arming again replaces the one waiting (it is cancelled first). The id to cancel by.
    pub fn arm(&mut self, host: &mut dyn MoveHost, spec: MoveSpec) -> Result<u64, String> {
        let mut spec = spec;
        spec.deck = spec.deck.trim().to_ascii_uppercase();
        let decks = host.move_decks();
        if !decks.contains(&spec.deck) {
            return Err(format!("no deck {:?}; one of {}", spec.deck, decks.join(", ")));
        }
        if spec.bars == 0 {
            spec.bars = spec.kind.default_bars();
        }
        spec.bars = spec.bars.min(64);
        let kind = spec.kind;
        let same: Vec<u64> = self
            .active
            .iter()
            .filter(|a| a.spec.kind == kind && (kind.one_at_a_time() || a.spec.deck == spec.deck))
            .map(|a| a.id)
            .collect();
        for id in same {
            self.cancel(host, id);
        }
        let deck = spec.deck.clone();
        let other = other(host, &deck);
        let phrase = spec.bars as f64 * BEATS_PER_BAR;
        let bar = BEATS_PER_BAR;
        let mut saved_low = 0.5;
        let mut stutter = false;
        let (grid, period, early) = match kind {
            // the outgoing deck's phrase: it is the one playing out
            MoveKind::BassSwap => {
                let out = other.clone().ok_or("a bass swap needs two decks")?;
                saved_low = host.eq_low(&deck).unwrap_or(0.5);
                (pick_grid(host, &out, Some(&deck)), phrase, 0.0)
            }
            MoveKind::PhraseSync => match spec.action {
                // the deck's own bar lines, one bar early (the compose has a bar to land)
                PhraseAction::Seed => (pick_grid(host, &deck, None), phrase, bar),
                // the deck playing out holds the phrase
                PhraseAction::Cut => (pick_grid(host, other.as_deref().unwrap_or(&deck), Some(&deck)), phrase, 0.0),
            },
            MoveKind::FilterExit | MoveKind::EchoOut => (pick_grid(host, &deck, None), bar, 0.0),
            MoveKind::LoopRoll => {
                stutter = !host.can_loop(&deck);
                (pick_grid(host, &deck, None), bar, 0.0)
            }
            MoveKind::VocoderHandoff => (pick_grid(host, &deck, other.as_deref()), phrase, 0.0),
        };
        if kind == MoveKind::PhraseSync && spec.action == PhraseAction::Seed && grid.is_none() {
            return Err(format!("phrase sync: deck {deck} is not playing (its seed steps on its own bar lines)"));
        }
        let at = match grid.as_deref().and_then(|g| host.grid(g)) {
            Some(g) => {
                let mut line = next_line(g.beat, period);
                while line - early <= g.beat {
                    line += period;
                }
                line - early
            }
            None => f64::NEG_INFINITY,
        };
        // what happens at once
        if kind == MoveKind::BassSwap && saved_low > 0.0 {
            host.set_eq_low(&deck, 0.0);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.active.push(Active {
            id,
            spec,
            phase: Phase::Armed,
            grid,
            at,
            stage: 0,
            elapsed: 0.0,
            last_now: None,
            saved_low,
            stutter,
        });
        let st = self.state_of(host, id).map(|s| s.text).unwrap_or_default();
        self.note(kind, &deck, st);
        Ok(id)
    }

    /// Cancel a move: one waiting is dropped (a bass swap gives the incoming lows back), one
    /// running is let go (the filter / wash back open, the roll released). False: no such move.
    pub fn cancel(&mut self, host: &mut dyn MoveHost, id: u64) -> bool {
        let Some(i) = self.active.iter().position(|a| a.id == id) else {
            return false;
        };
        let a = self.active.remove(i);
        let deck = a.spec.deck.as_str();
        match (a.spec.kind, &a.phase) {
            (MoveKind::BassSwap, Phase::Armed) => {
                host.set_eq_low(deck, a.saved_low);
            }
            (MoveKind::FilterExit, Phase::Running) => {
                let _ = host.fire_trick(deck, TrickKind::Filter, false, None);
            }
            (MoveKind::EchoOut, Phase::Running) => {
                let _ = host.fire_trick(deck, TrickKind::Wash, false, None);
            }
            (MoveKind::LoopRoll, Phase::Running) => release_roll(host, &a),
            _ => {}
        }
        self.note(a.spec.kind, deck, "cancelled".into());
        true
    }

    /// Cancel the move of `kind` on `deck` (any deck for a one-at-a-time kind).
    pub fn cancel_kind(&mut self, host: &mut dyn MoveHost, kind: MoveKind, deck: &str) -> bool {
        let deck = deck.to_ascii_uppercase();
        let ids: Vec<u64> = self
            .active
            .iter()
            .filter(|a| a.spec.kind == kind && (kind.one_at_a_time() || a.spec.deck == deck))
            .map(|a| a.id)
            .collect();
        let any = !ids.is_empty();
        for id in ids {
            self.cancel(host, id);
        }
        any
    }

    /// Everything in flight cancelled.
    pub fn cancel_all(&mut self, host: &mut dyn MoveHost) {
        let ids: Vec<u64> = self.active.iter().map(|a| a.id).collect();
        for id in ids {
            self.cancel(host, id);
        }
    }

    /// The move of `kind` on `deck` in flight, if one is.
    pub fn find(&self, kind: MoveKind, deck: &str) -> Option<u64> {
        self.active
            .iter()
            .find(|a| a.spec.kind == kind && (kind.one_at_a_time() || a.spec.deck.eq_ignore_ascii_case(deck)))
            .map(|a| a.id)
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }

    /// Whether move `id` is still in flight (armed or running): false once it is over or cancelled.
    pub fn in_flight(&self, id: u64) -> bool {
        self.active.iter().any(|a| a.id == id)
    }

    fn note(&mut self, kind: MoveKind, deck: &str, text: String) {
        let deck = if kind.one_at_a_time() { "*".to_string() } else { deck.to_string() };
        self.last.insert((kind, deck), text);
    }

    /// The last word on a move (what it did, or why not).
    pub fn last_word(&self, kind: MoveKind, deck: &str) -> Option<&str> {
        let deck = if kind.one_at_a_time() { "*".to_string() } else { deck.to_ascii_uppercase() };
        self.last.get(&(kind, deck)).map(String::as_str)
    }

    /// One service pass: every move whose line has come acts. `now` is a monotonic time in
    /// seconds (a roll counts its own beats from it).
    pub fn service(&mut self, host: &mut dyn MoveHost, now: f64) {
        let mut i = 0;
        while i < self.active.len() {
            let done = self.step(host, i, now);
            match done {
                Some(word) => {
                    let a = self.active.remove(i);
                    self.note(a.spec.kind, &a.spec.deck, word);
                }
                None => i += 1,
            }
        }
    }

    /// Advance move `i`; Some(word) when it is over.
    fn step(&mut self, host: &mut dyn MoveHost, i: usize, now: f64) -> Option<String> {
        let a = &mut self.active[i];
        let grid = a.grid.as_deref().and_then(|g| host.grid(g));
        // a roll keeps its own beats: the looping deck's beat wraps
        if a.spec.kind == MoveKind::LoopRoll && a.phase == Phase::Running {
            let bpm = host.grid(&a.spec.deck).map_or(0.0, |g| if g.playing { g.bpm } else { 0.0 });
            if let Some(t) = a.last_now {
                a.elapsed += (now - t).max(0.0) * bpm / 60.0;
            }
            a.last_now = Some(now);
            return roll_step(host, a);
        }
        // the grid stopped: what was waiting acts now
        let beat = match grid {
            Some(g) if g.playing => g.beat,
            _ => f64::INFINITY,
        };
        if beat < a.at {
            return None;
        }
        let deck = a.spec.deck.clone();
        let bars = a.spec.bars as f64;
        let line_bar = if a.at.is_finite() { (a.at / BEATS_PER_BAR).round() as i64 + 1 } else { 0 };
        let on_bar = |what: &str| {
            if line_bar > 0 {
                format!("{what} on bar {line_bar}")
            } else {
                format!("{what} at once (grid stopped)")
            }
        };
        match (a.spec.kind, &a.phase) {
            (MoveKind::BassSwap, _) => {
                let out = other(host, &deck)?;
                let out_low = host.eq_low(&out).unwrap_or(0.5);
                let give = if a.saved_low > 0.0 { a.saved_low } else { out_low.max(0.5) };
                host.set_eq_low(&deck, give);
                host.set_eq_low(&out, 0.0);
                Some(on_bar(&format!("lows swapped {out} → {deck}")))
            }
            (MoveKind::PhraseSync, _) => match a.spec.action {
                PhraseAction::Seed => Some(if host.step_seed(&deck) {
                    // fired a bar early: the seed comes in on the line
                    let lands = if a.at.is_finite() { (a.at / BEATS_PER_BAR).round() as i64 + 2 } else { 0 };
                    format!("{deck} next seed, in on bar {lands}")
                } else {
                    format!("{deck}: no composed net to step")
                }),
                PhraseAction::Cut => {
                    host.crossfade_to(&deck);
                    Some(on_bar(&format!("crossfader cut to {deck}")))
                }
            },
            (MoveKind::FilterExit, Phase::Armed) => {
                let args = TrickArgs { amount: FILTER_EXIT_AMOUNT, bars, ..TrickKind::Filter.default_args() };
                if let Err(e) = host.fire_trick(&deck, TrickKind::Filter, true, Some(args)) {
                    return Some(format!("filter exit: {e}"));
                }
                a.phase = Phase::Running;
                a.at = if beat.is_finite() { a.at + bars * BEATS_PER_BAR } else { f64::NEG_INFINITY };
                if !beat.is_finite() {
                    // a stopped deck has no downbeat to wait for
                    host.set_mute(&deck, true);
                    let _ = host.fire_trick(&deck, TrickKind::Filter, false, None);
                    return Some(format!("{deck} cut at once (stopped)"));
                }
                None
            }
            (MoveKind::FilterExit, Phase::Running) => {
                host.set_mute(&deck, true);
                let _ = host.fire_trick(&deck, TrickKind::Filter, false, None);
                Some(on_bar(&format!("{deck} cut")))
            }
            (MoveKind::EchoOut, Phase::Armed) => {
                let args = TrickArgs { beats: ECHO_CUT_BEATS, amount: 1.0, ..TrickKind::Wash.default_args() };
                if let Err(e) = host.fire_trick(&deck, TrickKind::Wash, true, Some(args)) {
                    return Some(format!("echo out: {e}"));
                }
                a.phase = Phase::Running;
                if !beat.is_finite() {
                    // stopped: nothing rings, so the cut is now
                    host.set_mute(&deck, true);
                    let _ = host.fire_trick(&deck, TrickKind::Wash, false, None);
                    return Some(format!("{deck} cut at once (stopped)"));
                }
                a.at += bars * BEATS_PER_BAR;
                None
            }
            (MoveKind::EchoOut, Phase::Running) => {
                host.set_mute(&deck, true);
                let _ = host.fire_trick(&deck, TrickKind::Wash, false, None);
                Some(on_bar(&format!("{deck} tail over, channel cut")))
            }
            (MoveKind::LoopRoll, _) => {
                if !beat.is_finite() {
                    return Some(format!("{deck} is not playing: nothing to roll"));
                }
                if let Err(e) = start_roll(host, a) {
                    return Some(format!("loop roll: {e}"));
                }
                a.phase = Phase::Running;
                a.elapsed = 0.0;
                a.last_now = Some(now);
                None
            }
            (MoveKind::VocoderHandoff, _) => Some(match host.vocoder_swap() {
                Ok(()) => on_bar("carrier ⇄ modulator"),
                Err(e) => format!("vocoder handoff: {e}"),
            }),
        }
    }

    /// The moves in flight.
    pub fn state(&self, host: &dyn MoveHost) -> Vec<MoveState> {
        self.active.iter().filter_map(|a| self.state_of(host, a.id)).collect()
    }

    fn state_of(&self, host: &dyn MoveHost, id: u64) -> Option<MoveState> {
        let a = self.active.iter().find(|a| a.id == id)?;
        let g = a.grid.as_deref().and_then(|g| host.grid(g)).filter(|g| g.playing);
        let roll_left = || {
            let step = a.spec.bars as f64 * BEATS_PER_BAR;
            (a.stage as f64 + 1.0) * step - a.elapsed
        };
        let beats_left = if a.spec.kind == MoveKind::LoopRoll && a.phase == Phase::Running {
            Some(roll_left().max(0.0))
        } else {
            g.map(|g| (a.at - g.beat).max(0.0)).filter(|_| a.at.is_finite())
        };
        let bar = |b: f64| (b / BEATS_PER_BAR).round() as i64 + 1;
        let when = if a.at.is_finite() { format!("bar {}", bar(a.at)) } else { "at once".into() };
        let text = match (a.spec.kind, &a.phase) {
            (MoveKind::BassSwap, _) => format!("lows cut on {}, swap on {when}", a.spec.deck),
            (MoveKind::PhraseSync, _) => match a.spec.action {
                PhraseAction::Seed => format!("next seed on {}", if a.at.is_finite() { format!("bar {}", bar(a.at) + 1) } else { when }),
                PhraseAction::Cut => format!("cut to {} on {when}", a.spec.deck),
            },
            (MoveKind::FilterExit, Phase::Armed) => format!("sweep from {when}"),
            (MoveKind::FilterExit, Phase::Running) => format!("sweeping: cut on {when}"),
            (MoveKind::EchoOut, Phase::Armed) => format!("echo on {when}"),
            (MoveKind::EchoOut, Phase::Running) => format!("ringing: cut on {when}"),
            (MoveKind::LoopRoll, Phase::Armed) => format!("roll from {when}"),
            (MoveKind::LoopRoll, Phase::Running) => {
                let stages = roll_stages(a.stutter);
                if a.stage < stages.len() {
                    format!("rolling {} beat{}", beat_words(stages[a.stage]), if stages[a.stage] == 1.0 { "" } else { "s" })
                } else {
                    "drop next".into()
                }
            }
            (MoveKind::VocoderHandoff, _) => format!("swap on {when}"),
        };
        Some(MoveState { id, spec: a.spec.clone(), phase: a.phase.clone(), beats_left, text })
    }

    /// The moves as JSON (the socket's `moves status`): what is in flight, the last word on
    /// each, and the harmonic hint.
    pub fn to_json(&self, host: &dyn MoveHost) -> JsonValue {
        let active: Vec<JsonValue> = self
            .state(host)
            .into_iter()
            .map(|s| {
                let mut o = JsonObject::new();
                o.set("id", JsonValue::int(s.id as i64));
                o.set("kind", s.spec.kind.name().into());
                o.set("deck", s.spec.deck.clone().into());
                o.set("bars", JsonValue::int(s.spec.bars as i64));
                if s.spec.kind == MoveKind::PhraseSync {
                    o.set("action", s.spec.action.name().into());
                }
                o.set("phase", if s.phase == Phase::Armed { "armed" } else { "running" }.into());
                o.set("beats_left", s.beats_left.map_or(JsonValue::Null, JsonValue::Number));
                o.set("text", s.text.into());
                JsonValue::Object(o)
            })
            .collect();
        let mut last = JsonObject::new();
        for ((k, d), t) in &self.last {
            last.set(format!("{}:{d}", k.name()), t.clone().into());
        }
        let mut o = JsonObject::new();
        o.set("active", JsonValue::Array(active));
        o.set("last", JsonValue::Object(last));
        o.set("harmonic", harmonic_hint(host).to_json());
        JsonValue::Object(o)
    }
}

/// The two decks' keys and how they mix.
pub fn harmonic_hint(host: &dyn MoveHost) -> Hint {
    let decks = host.move_decks();
    let a = decks.first().and_then(|d| host.key(d));
    let b = decks.get(1).and_then(|d| host.key(d));
    let compat = match (a, b) {
        (Some(a), Some(b)) => Some(harmonic(a, b)),
        _ => None,
    };
    Hint { a, b, compat }
}

/// A roll's stages, in beats: a loop's 4 → 2 → 1 → ½ (a sample deck with a grid, or a net
/// deck); a beat repeat's (a deck that cannot loop) 1 → ½ → ¼ → ⅛, the shortest the stutter
/// has.
pub fn roll_stages(stutter: bool) -> [f64; 4] {
    if stutter {
        [1.0, 0.5, 0.25, 0.125]
    } else {
        ROLL_BEATS
    }
}

fn beat_words(b: f64) -> String {
    match b {
        x if x == 0.5 => "½".into(),
        x if x == 0.25 => "¼".into(),
        x if x == 0.125 => "⅛".into(),
        x => format!("{x}"),
    }
}

fn stutter_args(beats: f64) -> TrickArgs {
    // the stutter's `div` is a note value: 4 = a quarter = one beat
    TrickArgs { div: 4.0 / beats, rise: 0.0, ..TrickKind::Stutter.default_args() }
}

fn start_roll(host: &mut dyn MoveHost, a: &mut Active) -> Result<(), String> {
    a.stage = 0;
    let deck = a.spec.deck.clone();
    if a.stutter {
        host.fire_trick(&deck, TrickKind::Stutter, true, Some(stutter_args(roll_stages(true)[0])))
    } else {
        host.loop_beats(&deck, ROLL_BEATS[0]);
        Ok(())
    }
}

/// A running roll: the next stage when its step is up; after the last, the release on the
/// next line (the drop). Some(word) when released.
fn roll_step(host: &mut dyn MoveHost, a: &mut Active) -> Option<String> {
    let step = a.spec.bars as f64 * BEATS_PER_BAR;
    let stages = roll_stages(a.stutter);
    // a little early is still this stage's end: the counter is not the audio's clock
    let due = (a.stage as f64 + 1.0) * step - 1e-6;
    if a.elapsed < due {
        return None;
    }
    a.stage += 1;
    let deck = a.spec.deck.clone();
    if a.stage < stages.len() {
        if a.stutter {
            let _ = host.fire_trick(&deck, TrickKind::Stutter, true, Some(stutter_args(stages[a.stage])));
        } else {
            host.loop_halve(&deck);
        }
        return None;
    }
    release_roll(host, a);
    Some(format!("{deck} roll released on the drop"))
}

fn release_roll(host: &mut dyn MoveHost, a: &Active) {
    if a.stutter {
        let _ = host.fire_trick(&a.spec.deck, TrickKind::Stutter, false, None);
    } else {
        host.exit_loop(&a.spec.deck);
    }
}

// MARK: the engine

mod engine_host {
    use super::*;
    use crate::deck::engine::Engine;
    use crate::deck::tricks::TrickQuant;
    use crate::deck::PerformCtx;
    use djengine::fx::StripKnob;
    use djengine::session::action::DeckAction;

    fn minor_scale(name: &str) -> bool {
        matches!(name, "Minor" | "Dorian" | "Phrygian" | "HarmonicMin" | "MinPentatonic" | "Blues")
    }

    fn is_net(e: &Engine, deck: &str) -> bool {
        e.deck(deck).is_some_and(|d| d.is_net())
    }

    /// A net deck's loop button as the deck action it is (`loop` with `op`), through
    /// `Engine::perform`, so the action log carries a roll on a net deck.
    fn net_loop_action(e: &mut Engine, deck: &str, op: &str) {
        e.perform(&DeckAction::Loop { deck: deck.to_string(), op: op.to_string() }, PerformCtx::default());
    }

    impl MoveHost for Engine {
        fn move_decks(&self) -> Vec<String> {
            self.decks.iter().take(2).map(|d| d.name.clone()).collect()
        }
        fn grid(&self, deck: &str) -> Option<Grid> {
            let s = self.beat_state(deck)?;
            let playing = self.deck(deck).is_some_and(|d| d.playing);
            Some(Grid { beat: s.beat, bpm: s.bpm, playing })
        }
        fn eq_low(&self, deck: &str) -> Option<f64> {
            self.deck(deck).map(|d| d.strip.eq_low)
        }
        fn set_eq_low(&mut self, deck: &str, v: f64) -> bool {
            self.set_strip(deck, StripKnob::EqLow, v)
        }
        fn set_mute(&mut self, deck: &str, on: bool) -> bool {
            self.set_deck_mute(deck, on)
        }
        fn fire_trick(&mut self, deck: &str, kind: TrickKind, on: bool, args: Option<TrickArgs>) -> Result<(), String> {
            self.trick(deck, kind, on, args, TrickQuant::Now)
        }
        fn can_loop(&self, deck: &str) -> bool {
            // a sample deck with a beat grid, or a net deck (the engine's own loop on its steps)
            Engine::can_loop(self, deck)
        }
        fn loop_beats(&mut self, deck: &str, beats: f64) {
            // no deck action loops N beats: the engine notes it as a live op (`loop_beats`)
            Engine::loop_beats(self, deck, beats)
        }
        fn loop_halve(&mut self, deck: &str) {
            if is_net(self, deck) {
                net_loop_action(self, deck, "halve");
            } else {
                Engine::loop_halve(self, deck)
            }
        }
        fn exit_loop(&mut self, deck: &str) {
            if is_net(self, deck) {
                net_loop_action(self, deck, "exit");
            } else {
                Engine::exit_loop(self, deck)
            }
        }
        fn step_seed(&mut self, deck: &str) -> bool {
            Engine::step_seed(self, deck, 1).is_some()
        }
        fn crossfade_to(&mut self, deck: &str) {
            // A is the left of the crossfader, B the right
            let v = if self.index(deck) == Some(0) { 0.0 } else { 1.0 };
            self.set_crossfader(v);
        }
        fn vocoder_swap(&mut self) -> Result<(), String> {
            Engine::vocoder_swap(self)
        }
        fn key(&self, deck: &str) -> Option<Key> {
            let d = self.deck(deck)?;
            if let Some(s) = &d.sample {
                let name = s.source_file.as_deref().unwrap_or(&s.title);
                return djengine::musicality::loop_key::key_in_name(name)
                    .map(|(pc, minor)| Key { pc: pc % 12, minor });
            }
            let p = d.project.as_ref()?;
            let root = p.root_note?;
            Some(Key { pc: root.rem_euclid(12) as u8, minor: minor_scale(&p.scale_name) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake engine: two decks on a fake clock that moves `bpm` beats a minute.
    #[derive(Default)]
    struct Fake {
        beat: [f64; 2],
        bpm: [f64; 2],
        playing: [bool; 2],
        low: [f64; 2],
        mute: [bool; 2],
        /// The deck loops on its beat (a sample deck with a grid, or a net deck).
        loops: [bool; 2],
        keys: [Option<Key>; 2],
        log: Vec<String>,
        vocoder_ok: bool,
    }

    fn ix(d: &str) -> usize {
        if d == "A" { 0 } else { 1 }
    }

    impl Fake {
        fn new() -> Fake {
            Fake { bpm: [120.0; 2], playing: [true; 2], low: [0.5; 2], vocoder_ok: true, ..Fake::default() }
        }
        /// Both decks `beats` on.
        fn run(&mut self, beats: f64) {
            for i in 0..2 {
                if self.playing[i] {
                    self.beat[i] += beats;
                }
            }
        }
    }

    impl MoveHost for Fake {
        fn move_decks(&self) -> Vec<String> {
            vec!["A".into(), "B".into()]
        }
        fn grid(&self, d: &str) -> Option<Grid> {
            let i = ix(d);
            Some(Grid { beat: self.beat[i], bpm: self.bpm[i], playing: self.playing[i] })
        }
        fn eq_low(&self, d: &str) -> Option<f64> {
            Some(self.low[ix(d)])
        }
        fn set_eq_low(&mut self, d: &str, v: f64) -> bool {
            self.low[ix(d)] = v;
            self.log.push(format!("{d} low {v} @{}", self.beat[0]));
            true
        }
        fn set_mute(&mut self, d: &str, on: bool) -> bool {
            self.mute[ix(d)] = on;
            self.log.push(format!("{d} mute {on} @{}", self.beat[ix(d)]));
            true
        }
        fn fire_trick(&mut self, d: &str, k: TrickKind, on: bool, a: Option<TrickArgs>) -> Result<(), String> {
            let extra = match (k, a) {
                (TrickKind::Stutter, Some(a)) => format!(" div {}", a.div),
                (TrickKind::Filter, Some(a)) => format!(" bars {}", a.bars),
                _ => String::new(),
            };
            self.log.push(format!("{d} {} {on}{extra} @{}", k.name(), self.beat[ix(d)]));
            Ok(())
        }
        fn can_loop(&self, d: &str) -> bool {
            self.loops[ix(d)]
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
            self.log.push(format!("{d} seed @{}", self.beat[ix(d)]));
            true
        }
        fn crossfade_to(&mut self, d: &str) {
            self.log.push(format!("xfade {d} @{}", self.beat[0]));
        }
        fn vocoder_swap(&mut self) -> Result<(), String> {
            if !self.vocoder_ok {
                return Err("nothing on the vocoder".into());
            }
            self.log.push(format!("vocoder swap @{}", self.beat[0]));
            Ok(())
        }
        fn key(&self, d: &str) -> Option<Key> {
            self.keys[ix(d)]
        }
    }

    /// Step the fake clock `beats` on in service passes of 1/40 beat (12.5 ms at 120).
    fn run(m: &mut Moves, h: &mut Fake, now: &mut f64, beats: f64) {
        let steps = (beats * 40.0).round() as usize;
        for _ in 0..steps {
            h.run(0.025);
            *now += 0.025 * 60.0 / h.bpm[0];
            m.service(h, *now);
        }
    }

    #[test]
    fn next_line_is_strictly_after() {
        assert_eq!(next_line(0.0, 32.0), 32.0);
        assert_eq!(next_line(31.9, 32.0), 32.0);
        assert_eq!(next_line(32.0, 32.0), 64.0);
        assert_eq!(next_line(5.0, 4.0), 8.0);
    }

    #[test]
    fn bass_swap_cuts_at_once_and_swaps_on_the_phrase() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [10.0, 10.0];
        let id = m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "B")).unwrap();
        assert_eq!(h.low, [0.5, 0.0], "incoming lows cut at arm");
        let st = m.state(&h);
        assert_eq!(st[0].id, id);
        assert_eq!(st[0].beats_left, Some(22.0)); // the 8-bar line at beat 32
        run(&mut m, &mut h, &mut now, 21.9);
        assert_eq!(h.low, [0.5, 0.0], "not yet");
        run(&mut m, &mut h, &mut now, 0.2);
        assert_eq!(h.low, [0.0, 0.5], "swapped on the line");
        assert!(m.is_idle());
        // both EQ moves landed in the same pass
        let tail: Vec<_> = h.log.iter().rev().take(2).cloned().collect();
        assert!(tail.iter().all(|l| l.ends_with(&tail[0].rsplit('@').next().map(|s| format!("@{s}")).unwrap())), "{tail:?}");
        assert!(m.last_word(MoveKind::BassSwap, "B").unwrap().contains("bar 9"));
    }

    #[test]
    fn a_cancelled_bass_swap_gives_the_lows_back() {
        let (mut m, mut h) = (Moves::new(), Fake::new());
        h.low[1] = 0.7;
        let id = m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "B")).unwrap();
        assert_eq!(h.low[1], 0.0);
        assert!(m.cancel(&mut h, id));
        assert_eq!(h.low, [0.5, 0.7]);
        assert!(m.is_idle());
    }

    #[test]
    fn bass_swap_times_on_the_outgoing_deck() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [30.0, 2.0]; // A plays out near its line; B just started
        m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "B")).unwrap();
        run(&mut m, &mut h, &mut now, 2.05);
        assert_eq!(h.low, [0.0, 0.5]);
    }

    #[test]
    fn phrase_sync_seed_fires_a_bar_early() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [3.0, 0.0];
        let mut s = MoveSpec::new(MoveKind::PhraseSync, "A");
        s.bars = 8;
        m.arm(&mut h, s).unwrap();
        run(&mut m, &mut h, &mut now, 25.05); // to beat 28: the bar before the 32 line
        assert!(h.log.iter().any(|l| l.starts_with("A seed @28")), "{:?}", h.log);
        assert!(m.is_idle());
        assert!(m.last_word(MoveKind::PhraseSync, "A").unwrap().contains("bar 9"));
    }

    #[test]
    fn phrase_sync_seed_inside_the_last_bar_waits_a_phrase() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [29.0, 0.0];
        let mut s = MoveSpec::new(MoveKind::PhraseSync, "A");
        s.bars = 8;
        m.arm(&mut h, s).unwrap();
        run(&mut m, &mut h, &mut now, 10.0);
        assert!(h.log.is_empty(), "no bar left to compose in: the next phrase");
        run(&mut m, &mut h, &mut now, 22.0);
        assert!(h.log.iter().any(|l| l.starts_with("A seed @60")), "{:?}", h.log);
    }

    #[test]
    fn phrase_sync_cut_on_the_16_and_32_bar_lines() {
        for (bars, line) in [(16u32, 64.0), (32, 128.0)] {
            let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
            h.beat = [1.0, 1.0];
            let s = MoveSpec { action: PhraseAction::Cut, bars, ..MoveSpec::new(MoveKind::PhraseSync, "B") };
            m.arm(&mut h, s).unwrap();
            run(&mut m, &mut h, &mut now, line - 1.0 - 0.1);
            assert!(h.log.is_empty());
            run(&mut m, &mut h, &mut now, 0.2);
            assert_eq!(h.log.len(), 1);
            assert!(h.log[0].starts_with(&format!("xfade B @{}", line as i64)), "{:?}", h.log);
        }
    }

    #[test]
    fn filter_exit_sweeps_from_the_bar_and_cuts_on_the_downbeat() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [5.5, 0.0];
        let s = MoveSpec { bars: 2, ..MoveSpec::new(MoveKind::FilterExit, "A") };
        m.arm(&mut h, s).unwrap();
        run(&mut m, &mut h, &mut now, 2.6);
        assert!(h.log[0].starts_with("A filter true bars 2 @8"), "{:?}", h.log);
        assert_eq!(m.state(&h)[0].phase, Phase::Running);
        run(&mut m, &mut h, &mut now, 7.8);
        assert!(!h.mute[0]);
        run(&mut m, &mut h, &mut now, 0.2);
        assert!(h.mute[0], "cut on bar 5's downbeat (beat 16)");
        assert!(h.log.iter().any(|l| l.starts_with("A mute true @16")), "{:?}", h.log);
        assert!(h.log.last().unwrap().starts_with("A filter false"));
        assert!(m.is_idle());
    }

    #[test]
    fn a_cancelled_sweep_opens_the_filter_and_leaves_the_channel() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        let id = m.arm(&mut h, MoveSpec::new(MoveKind::FilterExit, "A")).unwrap();
        run(&mut m, &mut h, &mut now, 5.0);
        assert!(m.cancel(&mut h, id));
        assert!(h.log.last().unwrap().starts_with("A filter false"));
        assert!(!h.mute[0]);
    }

    #[test]
    fn echo_out_throws_on_the_one_and_cuts_after_the_tail() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.beat = [1.0, 0.0];
        let s = MoveSpec { bars: 1, ..MoveSpec::new(MoveKind::EchoOut, "A") };
        m.arm(&mut h, s).unwrap();
        run(&mut m, &mut h, &mut now, 3.1);
        assert!(h.log[0].starts_with("A wash true @4"), "{:?}", h.log);
        assert!(!h.mute[0], "the room rings: the channel is open");
        run(&mut m, &mut h, &mut now, 4.0);
        assert!(h.mute[0]);
        assert!(h.log.iter().any(|l| l.starts_with("A mute true @8")), "{:?}", h.log);
    }

    #[test]
    fn loop_roll_halves_each_step_and_releases_on_the_drop() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.loops[0] = true;
        h.beat = [2.0, 0.0];
        m.arm(&mut h, MoveSpec::new(MoveKind::LoopRoll, "A")).unwrap(); // a bar a step
        run(&mut m, &mut h, &mut now, 2.1);
        assert_eq!(h.log, ["A loop 4"]);
        // the looping deck's own beat wraps: the roll counts its own beats
        h.beat[0] = 4.0;
        run(&mut m, &mut h, &mut now, 4.0);
        assert_eq!(h.log, ["A loop 4", "A halve"]);
        run(&mut m, &mut h, &mut now, 8.0);
        assert_eq!(h.log, ["A loop 4", "A halve", "A halve", "A halve"]);
        assert!(m.state(&h)[0].text.contains('½'), "{}", m.state(&h)[0].text);
        run(&mut m, &mut h, &mut now, 4.0);
        assert_eq!(h.log.last().unwrap(), "A exit loop");
        assert!(m.is_idle());
    }

    /// A deck that cannot loop (a sample with no beat grid) rolls the stutter's beat repeat.
    #[test]
    fn loop_roll_on_a_deck_that_cannot_loop_is_a_beat_repeat() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        m.arm(&mut h, MoveSpec::new(MoveKind::LoopRoll, "B")).unwrap();
        run(&mut m, &mut h, &mut now, 20.1);
        let divs: Vec<_> = h.log.iter().filter(|l| l.contains("stutter true")).map(|l| l.split(" div ").nth(1).unwrap().split(' ').next().unwrap().to_string()).collect();
        assert_eq!(divs, ["4", "8", "16", "32"]);
        assert!(h.log.last().unwrap().starts_with("B stutter false"));
    }

    #[test]
    fn a_cancelled_roll_lets_go_at_once() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        h.loops[0] = true;
        let id = m.arm(&mut h, MoveSpec::new(MoveKind::LoopRoll, "A")).unwrap();
        run(&mut m, &mut h, &mut now, 6.0);
        m.cancel(&mut h, id);
        assert_eq!(h.log.last().unwrap(), "A exit loop");
    }

    #[test]
    fn vocoder_handoff_swaps_on_the_phrase_and_reports_a_refusal() {
        let (mut m, mut h, mut now) = (Moves::new(), Fake::new(), 0.0);
        m.arm(&mut h, MoveSpec::new(MoveKind::VocoderHandoff, "A")).unwrap();
        run(&mut m, &mut h, &mut now, 32.1);
        assert!(h.log[0].starts_with("vocoder swap @32"), "{:?}", h.log);
        h.vocoder_ok = false;
        m.arm(&mut h, MoveSpec::new(MoveKind::VocoderHandoff, "B")).unwrap();
        run(&mut m, &mut h, &mut now, 32.1);
        assert!(m.last_word(MoveKind::VocoderHandoff, "A").unwrap().contains("nothing on the vocoder"));
    }

    #[test]
    fn a_stopped_grid_fires_at_once() {
        let (mut m, mut h) = (Moves::new(), Fake::new());
        h.playing = [false, false];
        m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "B")).unwrap();
        m.service(&mut h, 0.0);
        assert_eq!(h.low, [0.0, 0.5]);
        assert!(m.arm(&mut h, MoveSpec::new(MoveKind::PhraseSync, "A")).is_err(), "a seed waits for its deck's bars");
    }

    #[test]
    fn arming_again_replaces_and_one_swap_at_a_time() {
        let (mut m, mut h) = (Moves::new(), Fake::new());
        m.arm(&mut h, MoveSpec::new(MoveKind::FilterExit, "A")).unwrap();
        m.arm(&mut h, MoveSpec { bars: 8, ..MoveSpec::new(MoveKind::FilterExit, "A") }).unwrap();
        m.arm(&mut h, MoveSpec::new(MoveKind::FilterExit, "B")).unwrap();
        assert_eq!(m.state(&h).len(), 2);
        m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "B")).unwrap();
        m.arm(&mut h, MoveSpec::new(MoveKind::BassSwap, "A")).unwrap();
        assert_eq!(h.low, [0.0, 0.5], "B's cut given back when A's swap replaced it");
        assert_eq!(m.state(&h).len(), 3);
        assert!(m.find(MoveKind::BassSwap, "B").is_some(), "one at a time: any deck finds it");
        assert!(m.arm(&mut h, MoveSpec::new(MoveKind::EchoOut, "C")).is_err());
    }

    #[test]
    fn keys_mix_same_fifth_relative() {
        let k = |pc, minor| Key { pc, minor };
        assert_eq!(harmonic(k(9, true), k(9, true)), Compat::Same);
        assert_eq!(harmonic(k(0, false), k(7, false)), Compat::Fifth);
        assert_eq!(harmonic(k(0, false), k(5, false)), Compat::Fifth);
        assert_eq!(harmonic(k(0, false), k(9, true)), Compat::Relative); // C / Am
        assert_eq!(harmonic(k(9, true), k(0, false)), Compat::Relative);
        assert_eq!(harmonic(k(0, false), k(2, false)), Compat::Clash);
        assert_eq!(harmonic(k(0, false), k(0, true)), Compat::Clash); // parallel is not relative
        let mut h = Fake::new();
        h.keys = [Some(k(9, true)), Some(k(4, true))];
        let hint = harmonic_hint(&h);
        assert_eq!(hint.compat, Some(Compat::Fifth));
        assert_eq!(hint.to_json().get("a").and_then(JsonValue::as_str), Some("Am"));
        h.keys[1] = None;
        assert_eq!(harmonic_hint(&h).compat, None);
    }

    #[test]
    fn json_reports_flight_and_last_words() {
        let (mut m, mut h) = (Moves::new(), Fake::new());
        m.arm(&mut h, MoveSpec::new(MoveKind::EchoOut, "B")).unwrap();
        let j = m.to_json(&h);
        let act = j.get("active").and_then(JsonValue::as_array).unwrap();
        assert_eq!(act.len(), 1);
        assert_eq!(act[0].get("kind").and_then(JsonValue::as_str), Some("echo_out"));
        assert_eq!(act[0].get("phase").and_then(JsonValue::as_str), Some("armed"));
        assert!(j.get("last").and_then(|l| l.get("echo_out:B")).is_some());
        assert!(j.get("harmonic").is_some());
    }

    #[test]
    fn kinds_parse_by_name() {
        for k in MoveKind::ALL {
            assert_eq!(MoveKind::parse(k.name()), Some(k));
        }
        assert_eq!(MoveKind::parse("Bass-Swap"), Some(MoveKind::BassSwap));
        assert_eq!(PhraseAction::parse("transition"), Some(PhraseAction::Cut));
    }
}
