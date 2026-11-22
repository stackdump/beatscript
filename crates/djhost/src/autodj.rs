//! Auto-DJ (beyond Swift; docs/AUTO-DJ.md): the decks move on by themselves. Every N bars, on a
//! phrase line, it hands over to the next thing, with the effects automated across the
//! transition, as beats.bitwrap.io's Auto-DJ does for its one track.
//!
//! Where it comes from: beats' Auto-DJ (`public/lib/macros/runtime.js` `autoDjTick`): on a bar
//! cadence it fires macros from a curated **Transition** pool (riser, sweep HP / LP, reverb wash,
//! cathedral, delay throw, dub delay, tape stop, phaser drone: the ones that "resolve naturally on
//! a downbeat"), 1-4 bars long, and its **Regen** swaps in the next track on the N-bar line,
//! pre-rendered a bar early, with a transition macro injected on the new track. The picks are
//! random. djapp has two decks, so the handover is the DJ's, not a regenerate:
//!
//! | what the decks hold | the handover on the line | the effects (`effects` on) |
//! |---|---|---|
//! | **a loop-set pair** (group g of a set on both decks) | the next pair: `load_pair` of group g + 1 (wrapping) | the style's, on both decks |
//! | **two playing decks** (net tracks) | `bass_swap` + `phrase_sync` cut to the incoming deck; the outgoing deck's next seed stepped after | the style's exit move on the outgoing deck |
//! | **one playing net deck** | `phrase_sync` seed: its next seed, in on the line (as auto-next) | the style's, on that deck |
//!
//! The styles (`auto` rotates through the three, so no two in a row are alike; beats picked at
//! random): `filter` = a noise riser and a high-pass sweep that ends on the line (`filter_exit`
//! on a two-deck mix, which mutes the outgoing deck on the downbeat); `echo` = the room rings out
//! from the line (`echo_out`, a delay throw into the handover on a pair / one deck); `roll` = a
//! loop roll 4 -> 2 -> 1 -> 1/2 beat and a riser, dropped on the line (`loop_roll`).
//!
//! Timing: the scheduler counts the grid deck's **bar lines** itself (a deck's beat clock wraps
//! when it loops; a wrap or a swap to the top is a line too, as auto-next counts it) and works
//! on cues in bars before / after the boundary line. The first boundary is the next multiple of N
//! bars on the deck's own grid; after a transition the next is N bars on. Every move is armed a
//! line ahead so it lands on the boundary: a [`Moves`] move is bar-relative when armed with
//! `bars = 1`, and the moves are the same ones the Perform window arms (they show there).
//!
//! The scheduler is pure behind [`AutoDjHost`] (the [`MoveHost`] the moves need, plus what the
//! handover reads and does), so the window, the control socket and the tests (a fake host on a
//! fake clock) all drive the same [`AutoDj`]: [`AutoDj::set_config`], [`AutoDj::next`],
//! [`AutoDj::service`] (each pass), [`AutoDj::to_json`].

use djengine::fx::tricks::{TrickArgs, TrickKind};
use djengine::json::{JsonObject, JsonValue};

use crate::perform::{
    Grid, MoveHost, MoveKind, MoveSpec, Moves, PhraseAction, BEATS_PER_BAR, FILTER_EXIT_AMOUNT,
};

/// The phrase lengths offered (the window's menu; any whole number in range is accepted).
pub const PRESETS: [u32; 4] = [8, 16, 32, 64];
pub const DEFAULT_BARS: u32 = 32;
/// The shortest and longest phrase. A transition needs room: a fade, the line before it, and
/// the tail of the one before.
pub const MIN_BARS: u32 = 8;
pub const MAX_BARS: u32 = 64;
/// The transition lengths offered, in bars (beats' transition macros run 1-4).
pub const FADES: [u32; 4] = [1, 2, 4, 8];
pub const DEFAULT_FADE: u32 = 4;
pub const MAX_FADE: u32 = 8;

/// How far the riser goes (0..1, its level).
pub const RISER_AMOUNT: f64 = 0.5;

/// Which effects the transition runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Rotate through `filter`, `echo`, `roll`.
    Auto,
    /// A riser and a high-pass sweep ending on the line.
    Filter,
    /// The room rings out from the line / a delay throw into it.
    Echo,
    /// A loop roll and a riser, dropped on the line.
    Roll,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Auto, Style::Filter, Style::Echo, Style::Roll];
    /// What `auto` rotates through.
    pub const ROTATION: [Style; 3] = [Style::Filter, Style::Echo, Style::Roll];

    pub fn name(self) -> &'static str {
        match self {
            Style::Auto => "auto",
            Style::Filter => "filter",
            Style::Echo => "echo",
            Style::Roll => "roll",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Style::Auto => "Auto (rotate)",
            Style::Filter => "Filter + riser",
            Style::Echo => "Echo",
            Style::Roll => "Roll + riser",
        }
    }

    pub fn parse(s: &str) -> Option<Style> {
        let s = s.trim().to_ascii_lowercase();
        Style::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// The settings (the config's `autodj.*`).
#[derive(Clone, Debug, PartialEq)]
pub struct AutoDjConfig {
    pub enabled: bool,
    /// Bars between handovers, `MIN_BARS..=MAX_BARS`.
    pub bars: u32,
    /// Bars the transition effects last, `1..=MAX_FADE` (held under half the phrase).
    pub fade: u32,
    pub style: Style,
    /// Run the effects across the transition (off: the bare handover).
    pub effects: bool,
}

impl Default for AutoDjConfig {
    fn default() -> Self {
        AutoDjConfig { enabled: false, bars: DEFAULT_BARS, fade: DEFAULT_FADE, style: Style::Auto, effects: true }
    }
}

pub fn clamp_bars(n: i64) -> u32 {
    n.clamp(MIN_BARS as i64, MAX_BARS as i64) as u32
}

pub fn clamp_fade(n: i64) -> u32 {
    n.clamp(1, MAX_FADE as i64) as u32
}

/// The loop-set pair both decks hold (a collection's group whose A is on deck A and B on B).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LitPair {
    /// The collection (`LoopBank::spec`).
    pub spec: String,
    pub group: u32,
    /// How many groups it has (the next after the last is the first).
    pub groups: u32,
}

impl LitPair {
    /// The group after this one, wrapping.
    pub fn next_group(&self) -> u32 {
        if self.groups == 0 {
            self.group
        } else {
            self.group % self.groups + 1
        }
    }
}

/// What the scheduler reads and does beyond the moves' own host.
pub trait AutoDjHost: MoveHost {
    /// The crossfader, 0 = deck A's side .. 1 = B's.
    fn crossfader(&self) -> f64;
    fn muted(&self, deck: &str) -> bool;
    /// The deck holds a composed net it can step to the next seed.
    fn can_step(&self, deck: &str) -> bool;
    /// The loop-set pair the decks hold, if both decks hold the same group of one collection.
    fn lit_pair(&self) -> Option<LitPair>;
    /// Group `group` of `spec` on both decks, playing and locked on the bar.
    fn load_pair(&mut self, spec: &str, group: u32) -> Result<(), String>;
}

/// What the decks hold when the transition starts: how it hands over.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Pair { pair: LitPair, decks: Vec<String> },
    Mix { out: String, inn: String },
    Solo { deck: String },
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Pair { .. } => "pair",
            Mode::Mix { .. } => "mix",
            Mode::Solo { .. } => "solo",
        }
    }

    pub fn text(&self) -> String {
        match self {
            Mode::Pair { pair, .. } => format!("pair {} -> {} of {}", pair.group, pair.next_group(), pair.spec),
            Mode::Mix { out, inn } => format!("mix {out} -> {inn}"),
            Mode::Solo { deck } => format!("deck {deck}: next seed"),
        }
    }
}

/// One thing a cue does.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Arm a Perform move.
    Arm(MoveSpec),
    Trick { deck: String, kind: TrickKind, on: bool, args: Option<TrickArgs> },
    Mute { deck: String, on: bool },
    LoadPair { spec: String, group: u32 },
    StepSeed(String),
}

impl Op {
    fn text(&self) -> String {
        match self {
            Op::Arm(s) => format!("{} {}", s.kind.label(), s.deck),
            Op::Trick { deck, kind, on, .. } => format!("{} {} {}", kind.name(), deck, if *on { "on" } else { "off" }),
            Op::Mute { deck, on } => format!("{deck} {}", if *on { "muted" } else { "unmuted" }),
            Op::LoadPair { group, .. } => format!("pair {group}"),
            Op::StepSeed(d) => format!("{d} next seed"),
        }
    }
}

/// An op on a line: `off` bars from the boundary (negative: before it).
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub off: i64,
    pub op: Op,
}

fn cue(off: i64, op: Op) -> Cue {
    Cue { off, op }
}

fn arm(kind: MoveKind, deck: &str, bars: u32) -> Op {
    let mut spec = MoveSpec::new(kind, deck);
    spec.bars = bars;
    Op::Arm(spec)
}

fn trick(deck: &str, kind: TrickKind, on: bool, args: Option<TrickArgs>) -> Op {
    Op::Trick { deck: deck.to_string(), kind, on, args }
}

/// The transition's shape from the settings: the style that runs (`auto` resolved by the
/// rotation `n`), the fade in bars, a roll's bars per step, and how many bars before the
/// boundary the first cue falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub style: Style,
    pub fade: u32,
    pub step: u32,
    pub lead: i64,
}

/// The shape for `cfg` on transition number `n`.
pub fn shape(cfg: &AutoDjConfig, n: u64) -> Shape {
    let style = match cfg.style {
        Style::Auto => Style::ROTATION[(n % Style::ROTATION.len() as u64) as usize],
        s => s,
    };
    // a tail rings L bars past the line: the next transition must start after it
    let fade = cfg.fade.clamp(1, ((cfg.bars.max(MIN_BARS) - 1) / 2).max(1));
    let step = (fade / 4).max(1);
    let span = 4 * step;
    let lead = if !cfg.effects {
        2
    } else {
        match style {
            Style::Roll => span as i64 + 1,
            _ => fade as i64 + 1,
        }
    };
    Shape { style, fade, step, lead }
}

/// The cues of a transition: pure, so the tests read the whole plan.
pub fn build(mode: &Mode, cfg: &AutoDjConfig, sh: Shape) -> Vec<Cue> {
    let l = sh.fade as i64;
    let span = 4 * sh.step as i64;
    let mut v: Vec<Cue> = Vec::new();
    let fx = cfg.effects;
    let riser = |deck: &str, bars: i64| {
        trick(deck, TrickKind::Riser, true, Some(TrickArgs { amount: RISER_AMOUNT, bars: bars as f64, ..TrickKind::Riser.default_args() }))
    };
    // the effects that are not the handover's: on the decks that keep sounding (a pair, one deck)
    let on_decks = |v: &mut Vec<Cue>, decks: &[String]| {
        match sh.style {
            Style::Filter => {
                for d in decks {
                    v.push(cue(-l, riser(d, l)));
                    let args = TrickArgs { amount: FILTER_EXIT_AMOUNT, bars: l as f64, ..TrickKind::Filter.default_args() };
                    v.push(cue(-l, trick(d, TrickKind::Filter, true, Some(args))));
                    v.push(cue(0, trick(d, TrickKind::Filter, false, None)));
                }
            }
            Style::Echo => {
                for d in decks {
                    v.push(cue(-1, trick(d, TrickKind::Delay, true, None)));
                    v.push(cue(0, trick(d, TrickKind::Delay, false, None)));
                }
            }
            _ => {
                for d in decks {
                    v.push(cue(-span - 1, arm(MoveKind::LoopRoll, d, sh.step)));
                    v.push(cue(-span, riser(d, span)));
                }
            }
        }
    };
    match mode {
        Mode::Pair { pair, decks } => {
            if fx {
                on_decks(&mut v, decks);
            }
            v.push(cue(0, Op::LoadPair { spec: pair.spec.clone(), group: pair.next_group() }));
        }
        Mode::Solo { deck } => {
            if fx {
                on_decks(&mut v, std::slice::from_ref(deck));
            }
            // armed two lines early: the step fires a bar before the line, the seed is in on it
            let mut spec = MoveSpec::new(MoveKind::PhraseSync, deck);
            spec.bars = 1;
            spec.action = PhraseAction::Seed;
            v.push(cue(-2, Op::Arm(spec)));
        }
        Mode::Mix { out, inn } => {
            v.push(cue(-sh.lead, Op::Mute { deck: inn.clone(), on: false }));
            if fx {
                match sh.style {
                    Style::Filter => {
                        v.push(cue(-l - 1, arm(MoveKind::FilterExit, out, sh.fade)));
                        v.push(cue(-l, riser(out, l)));
                    }
                    Style::Echo => v.push(cue(-1, arm(MoveKind::EchoOut, out, sh.fade))),
                    _ => {
                        v.push(cue(-span - 1, arm(MoveKind::LoopRoll, out, sh.step)));
                        v.push(cue(-span, riser(out, span)));
                    }
                }
            }
            // bar-relative (bars = 1): armed a line ahead, both land on the boundary
            v.push(cue(-1, arm(MoveKind::BassSwap, inn, 1)));
            let mut cut = MoveSpec::new(MoveKind::PhraseSync, inn);
            cut.bars = 1;
            cut.action = PhraseAction::Cut;
            v.push(cue(-1, Op::Arm(cut)));
            // the deck that played out comes up on a new seed for its next turn
            v.push(cue(1, Op::StepSeed(out.clone())));
        }
    }
    v.sort_by_key(|c| c.off);
    v
}

/// A transition in flight.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) mode: Mode,
    pub(crate) style: Style,
    /// The boundary, in the scheduler's line count.
    pub(crate) boundary: i64,
    /// The cues left, with their lines.
    pub(crate) cues: Vec<(i64, Cue)>,
    /// The moves armed, to cancel with it.
    pub(crate) ids: Vec<u64>,
    /// The tricks switched on and not yet off.
    pub(crate) held: Vec<(String, TrickKind)>,
}

/// Lines (bar lines) crossed between two looks at a deck's beat: a jump back by more than a
/// couple of beats is a loop's wrap or a swap to the top, which is a line too (as auto-next
/// counts it); a small step back is the platter's jitter.
pub fn lines_crossed(prev: f64, cur: f64) -> i64 {
    let bar = |b: f64| (b / BEATS_PER_BAR + 1e-9).floor() as i64;
    if cur + 2.0 >= prev {
        (bar(cur) - bar(prev)).max(0)
    } else {
        1
    }
}

/// The scheduler.
#[derive(Debug)]
pub struct AutoDj {
    pub cfg: AutoDjConfig,
    /// Run one transition even while off (`next`).
    once: bool,
    force: bool,
    /// Bar lines counted so far (it starts from the deck's own bar number, so the first
    /// boundary is a multiple of N bars of its grid).
    line: i64,
    /// The grid deck and its beat at the last look.
    last: Option<(String, f64)>,
    /// The line of the next boundary.
    boundary: Option<i64>,
    plan: Option<Plan>,
    /// Transitions made.
    pub transitions: u64,
    /// The last thing it did, or why it could not.
    pub word: String,
    /// Why it is not counting (none: it is).
    idle: Option<String>,
    /// What the last transition (or the last look for one) found the decks holding.
    mode_seen: Option<&'static str>,
}

impl Default for AutoDj {
    fn default() -> Self {
        AutoDj::new()
    }
}

impl AutoDj {
    pub fn new() -> AutoDj {
        AutoDj {
            cfg: AutoDjConfig::default(),
            once: false,
            force: false,
            line: 0,
            last: None,
            boundary: None,
            plan: None,
            transitions: 0,
            word: String::new(),
            idle: Some("off".into()),
            mode_seen: None,
        }
    }

    /// Change the settings. A new phrase length or turning it on starts the count from the
    /// deck's next phrase line; turning it off lets go of what it holds (the call that does it
    /// is [`AutoDj::service`]'s next pass, or [`AutoDj::cancel`]).
    pub fn set_config(&mut self, cfg: AutoDjConfig) {
        let mut cfg = cfg;
        cfg.bars = clamp_bars(cfg.bars as i64);
        cfg.fade = clamp_fade(cfg.fade as i64);
        if cfg.bars != self.cfg.bars || (cfg.enabled && !self.cfg.enabled) {
            if self.plan.is_none() {
                self.boundary = None;
            }
        }
        self.cfg = cfg;
    }

    /// Run one transition now, on the next bar line (it lands `lead` bars later), then carry
    /// on as set. Works while off (one transition, then idle). Err: one is already running.
    pub fn next(&mut self) -> Result<(), String> {
        if self.plan.is_some() {
            return Err("a transition is already running".into());
        }
        self.once = true;
        self.force = true;
        Ok(())
    }

    /// In a transition (its cues not all fired).
    pub fn running(&self) -> bool {
        self.plan.is_some()
    }

    /// On, or a transition in flight.
    pub fn busy(&self) -> bool {
        self.cfg.enabled || self.once || self.plan.is_some()
    }

    /// Let go of everything it holds: the tricks it switched on go off, the moves it armed are
    /// cancelled. The count starts over.
    pub fn cancel<H: AutoDjHost>(&mut self, host: &mut H, moves: &mut Moves) {
        if let Some(plan) = self.plan.take() {
            release(host, moves, &plan);
        }
        self.boundary = None;
        self.last = None;
        self.once = false;
        self.force = false;
    }

    fn pick_grid<H: AutoDjHost>(&self, host: &H) -> Option<String> {
        let playing = |d: &String| host.grid(d).is_some_and(|g| g.playing && g.bpm > 0.0);
        let decks = host.move_decks();
        // the deck that plays out holds the grid through a mix; else the one the crossfader favours
        if let Some(Plan { mode: Mode::Mix { out, .. }, .. }) = &self.plan {
            if playing(out) {
                return Some(out.clone());
            }
        }
        if let Some(d) = self.last.as_ref().map(|(d, _)| d).filter(|d| playing(d)) {
            return Some(d.clone());
        }
        let favoured = if host.crossfader() <= 0.5 { 0 } else { 1 };
        let mut order: Vec<&String> = decks.iter().collect();
        if favoured < order.len() {
            let f = order.remove(favoured);
            order.insert(0, f);
        }
        order.into_iter().find(|d| playing(d)).cloned()
    }

    /// What the decks hold now, or why nothing can be handed over.
    pub fn detect<H: AutoDjHost>(&self, host: &H) -> Result<Mode, String> {
        let playing: Vec<String> = host
            .move_decks()
            .into_iter()
            .filter(|d| host.grid(d).is_some_and(|g| g.playing && g.bpm > 0.0))
            .collect();
        if playing.is_empty() {
            return Err("no deck playing".into());
        }
        if let Some(pair) = host.lit_pair() {
            return Ok(Mode::Pair { pair, decks: playing });
        }
        match playing.as_slice() {
            [d] => {
                if host.can_step(d) {
                    Ok(Mode::Solo { deck: d.clone() })
                } else {
                    Err("no composed net (nothing to step from)".into())
                }
            }
            _ => {
                let loud: Vec<&String> = playing.iter().filter(|d| !host.muted(d)).collect();
                let out = match loud.as_slice() {
                    [d] => (*d).clone(),
                    _ => {
                        let i = if host.crossfader() <= 0.5 { 0 } else { playing.len() - 1 };
                        playing[i].clone()
                    }
                };
                let inn = playing.iter().find(|d| **d != out).cloned().ok_or("a mix needs two playing decks")?;
                Ok(Mode::Mix { out, inn })
            }
        }
    }

    /// One service pass: count the bar lines, start the transition when its lead is due and
    /// fire every cue whose line has come.
    pub fn service<H: AutoDjHost>(&mut self, host: &mut H, moves: &mut Moves) {
        if !self.cfg.enabled && !self.once {
            if self.plan.is_some() {
                self.cancel(host, moves);
                self.word = "off: let go".into();
            }
            self.idle = Some("off".into());
            self.boundary = None;
            self.last = None;
            return;
        }
        let Some(grid) = self.pick_grid(host) else {
            self.idle = Some("no deck playing".into());
            if let Some(plan) = self.plan.take() {
                release(host, moves, &plan);
                self.word = "no deck playing: transition dropped".into();
            }
            self.last = None;
            return;
        };
        let beat = host.grid(&grid).map_or(0.0, |g: Grid| g.beat);
        let crossed = match &self.last {
            Some((d, b)) if *d == grid => lines_crossed(*b, beat),
            _ => 0,
        };
        if self.boundary.is_none() {
            // (re)start the count from the deck's own bar number: a phrase line of its grid
            self.line = (beat / BEATS_PER_BAR + 1e-9).floor() as i64;
        } else {
            self.line += crossed;
        }
        self.last = Some((grid, beat));
        let n = self.cfg.bars as i64;
        let sh = shape(&self.cfg, self.transitions);
        if self.boundary.is_none() {
            let mut b = (self.line.div_euclid(n) + 1) * n;
            while b - sh.lead <= self.line {
                b += n;
            }
            self.boundary = Some(b);
        }
        if self.force && self.plan.is_none() {
            self.force = false;
            self.boundary = Some(self.line + sh.lead + 1);
        }
        self.idle = None;
        // start
        if self.plan.is_none() {
            let b = self.boundary.unwrap_or(self.line + n);
            if self.line > b {
                // missed (a stall, or the deck jumped): the next one
                self.boundary = Some(b + n);
            } else if self.line >= b - sh.lead {
                match self.detect(host) {
                    Ok(mode) => {
                        self.mode_seen = Some(mode.name());
                        let cues = build(&mode, &self.cfg, sh).into_iter().map(|c| (b + c.off, c)).collect();
                        self.word = format!("{}: {} (in {} bars)", sh.style.name(), mode.text(), b - self.line);
                        self.plan = Some(Plan { mode, style: sh.style, boundary: b, cues, ids: Vec::new(), held: Vec::new() });
                    }
                    Err(why) => {
                        self.word = format!("skipped: {why}");
                        self.idle = Some(why);
                        self.boundary = Some(b + n);
                        self.once = false;
                    }
                }
            }
        }
        // fire
        let Some(mut plan) = self.plan.take() else { return };
        let mut said: Vec<String> = Vec::new();
        let mut reset = false;
        while plan.cues.first().is_some_and(|(at, _)| *at <= self.line) {
            let (_, c) = plan.cues.remove(0);
            let (text, rebase) = exec(host, moves, &mut plan, c.op);
            said.push(text);
            reset |= rebase;
        }
        if !said.is_empty() {
            self.word = format!("{}: {}", plan.mode.name(), said.join(", "));
        }
        if reset {
            // the grid deck's beat clock started over: not a line
            self.last = None;
        }
        if plan.cues.is_empty() {
            self.transitions += 1;
            self.boundary = Some(plan.boundary + n);
            self.once = false;
        } else {
            self.plan = Some(plan);
        }
    }

    /// In words: why it is not counting, or where it is.
    pub fn text(&self) -> String {
        if let Some(why) = &self.idle {
            return why.clone();
        }
        match (&self.plan, self.boundary) {
            (Some(p), _) => format!("transition: {} ({} left)", p.mode.text(), p.cues.len()),
            (None, Some(b)) => format!("next handover in {} bars", (b - self.line).max(0)),
            _ => "waiting".into(),
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("enabled", self.cfg.enabled.into());
        o.set("bars", JsonValue::int(self.cfg.bars as i64));
        o.set("fade", JsonValue::int(self.cfg.fade as i64));
        o.set("style", self.cfg.style.name().into());
        o.set("effects", self.cfg.effects.into());
        let state = if self.idle.is_some() {
            "idle"
        } else if self.plan.is_some() {
            "transition"
        } else {
            "waiting"
        };
        o.set("state", state.into());
        o.set("idle", self.idle.clone().map_or(JsonValue::Null, JsonValue::str));
        o.set("text", self.text().into());
        o.set("word", self.word.clone().into());
        o.set("transitions", JsonValue::int(self.transitions as i64));
        o.set(
            "bars_left",
            match (self.boundary, &self.idle) {
                (Some(b), None) => JsonValue::int((b - self.line).max(0)),
                _ => JsonValue::Null,
            },
        );
        match &self.plan {
            Some(p) => {
                o.set("mode", p.mode.name().into());
                o.set("running_style", p.style.name().into());
                o.set("cues_left", JsonValue::int(p.cues.len() as i64));
            }
            None => {
                o.set("mode", self.mode_seen.map_or(JsonValue::Null, |m| m.into()));
            }
        }
        o.set(
            "next_style",
            if self.plan.is_some() { shape(&self.cfg, self.transitions + 1) } else { shape(&self.cfg, self.transitions) }
                .style
                .name()
                .into(),
        );
        o.set("styles", JsonValue::Array(Style::ALL.iter().map(|s| s.name().into()).collect()));
        o.set("presets", JsonValue::Array(PRESETS.iter().map(|&p| JsonValue::int(p as i64)).collect()));
        JsonValue::Object(o)
    }
}

/// Do one cue. The words, and whether the grid deck's clock started over.
pub(crate) fn exec<H: AutoDjHost>(host: &mut H, moves: &mut Moves, plan: &mut Plan, op: Op) -> (String, bool) {
    let said = op.text();
    match op {
        Op::Arm(spec) => match moves.arm(&mut *host, spec) {
            Ok(id) => {
                plan.ids.push(id);
                (said, false)
            }
            Err(e) => (format!("{said}: {e}"), false),
        },
        Op::Trick { deck, kind, on, args } => match host.fire_trick(&deck, kind, on, args) {
            Ok(()) => {
                plan.held.retain(|(d, k)| !(*d == deck && *k == kind));
                if on {
                    plan.held.push((deck, kind));
                }
                (said, false)
            }
            Err(e) => (format!("{said}: {e}"), false),
        },
        Op::Mute { deck, on } => {
            if host.muted(&deck) != on {
                host.set_mute(&deck, on);
            }
            (said, false)
        }
        Op::LoadPair { spec, group } => match host.load_pair(&spec, group) {
            Ok(()) => (said, true),
            Err(e) => (format!("{said}: {e}"), false),
        },
        Op::StepSeed(deck) => {
            if host.can_step(&deck) && host.step_seed(&deck) {
                (said, false)
            } else {
                (format!("{deck}: no composed net to step"), false)
            }
        }
    }
}

/// Let go of what a plan holds.
pub(crate) fn release<H: AutoDjHost>(host: &mut H, moves: &mut Moves, plan: &Plan) {
    for (d, k) in &plan.held {
        let _ = host.fire_trick(d, *k, false, None);
    }
    for id in &plan.ids {
        moves.cancel(&mut *host, *id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two decks on a fake clock (120 bpm: a beat per pass of `STEP`), the loop set's pair, the
    /// crossfader and mutes, and a log of everything the scheduler did.
    struct Fake {
        beat: [f64; 2],
        playing: [bool; 2],
        mute: [bool; 2],
        low: [f64; 2],
        xf: f64,
        net: [bool; 2],
        pair: Option<LitPair>,
        /// A loop's length in beats (None: a song that runs on); the beat wraps at it.
        wrap: Option<f64>,
        log: Vec<String>,
    }

    fn ix(d: &str) -> usize {
        usize::from(d != "A")
    }

    /// 1/32 of a beat: exact in binary, so the fake's lines fall exactly on the bar.
    const STEP: f64 = 0.03125;

    impl Fake {
        fn new() -> Fake {
            Fake {
                beat: [0.0; 2],
                playing: [true; 2],
                mute: [false; 2],
                low: [0.5; 2],
                xf: 0.0,
                net: [true; 2],
                pair: None,
                wrap: None,
                log: Vec::new(),
            }
        }
        fn tick(&mut self) {
            for i in 0..2 {
                if self.playing[i] {
                    self.beat[i] += STEP;
                    if let Some(w) = self.wrap {
                        if self.beat[i] >= w {
                            self.beat[i] -= w;
                        }
                    }
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
            self.log.push(format!("{d} low {v} @{:.2}", self.beat[0] / 4.0));
            true
        }
        fn set_mute(&mut self, d: &str, on: bool) -> bool {
            self.mute[ix(d)] = on;
            self.log.push(format!("{d} mute {on} @{:.2}", self.beat[ix(d)] / 4.0));
            true
        }
        fn fire_trick(&mut self, d: &str, k: TrickKind, on: bool, a: Option<TrickArgs>) -> Result<(), String> {
            let bars = a.map_or(String::new(), |a| format!(" bars {}", a.bars));
            self.log.push(format!("{d} {} {on}{bars} @{:.2}", k.name(), self.beat[ix(d)] / 4.0));
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
            self.log.push(format!("{d} exit loop @{:.2}", self.beat[ix(d)] / 4.0));
        }
        fn step_seed(&mut self, d: &str) -> bool {
            self.log.push(format!("{d} seed @{:.2}", self.beat[ix(d)] / 4.0));
            true
        }
        fn crossfade_to(&mut self, d: &str) {
            self.xf = if d == "A" { 0.0 } else { 1.0 };
            self.log.push(format!("xfade {d} @{:.2}", self.beat[0] / 4.0));
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
        fn can_step(&self, d: &str) -> bool {
            self.net[ix(d)]
        }
        fn lit_pair(&self) -> Option<LitPair> {
            self.pair.clone()
        }
        fn load_pair(&mut self, spec: &str, group: u32) -> Result<(), String> {
            self.log.push(format!("load_pair {spec} {group} @{:.2}", self.beat[0] / 4.0));
            self.pair = self.pair.as_ref().map(|p| LitPair { group, ..p.clone() });
            // the new loops start from their top
            self.beat = [0.0; 2];
            Ok(())
        }
    }

    struct Rig {
        dj: AutoDj,
        moves: Moves,
        h: Fake,
    }

    impl Rig {
        fn new(h: Fake, cfg: AutoDjConfig) -> Rig {
            let mut dj = AutoDj::new();
            dj.set_config(cfg);
            Rig { dj, moves: Moves::new(), h }
        }
        /// `bars` bars on, a pass every 1/32 beat.
        fn run(&mut self, bars: f64) {
            let steps = (bars * 4.0 / STEP).round() as usize;
            let mut now = 0.0;
            for _ in 0..steps {
                self.h.tick();
                now += STEP * 0.5;
                self.dj.service(&mut self.h, &mut self.moves);
                self.moves.service(&mut self.h, now);
            }
        }
    }

    fn on(bars: u32, style: Style) -> AutoDjConfig {
        AutoDjConfig { enabled: true, bars, fade: 4, style, effects: true }
    }

    fn pair() -> LitPair {
        LitPair { spec: "techno".into(), group: 2, groups: 3 }
    }

    #[test]
    fn it_is_off_and_does_nothing_by_default() {
        let mut r = Rig::new(Fake::new(), AutoDjConfig::default());
        r.run(40.0);
        assert!(r.h.log.is_empty(), "{:?}", r.h.log);
        assert_eq!(r.dj.transitions, 0);
        assert_eq!(r.dj.text(), "off");
        assert!(!AutoDjConfig::default().enabled);
    }

    #[test]
    fn the_next_group_wraps() {
        let p = pair();
        assert_eq!(p.next_group(), 3);
        assert_eq!(LitPair { group: 3, ..p.clone() }.next_group(), 1);
        assert_eq!(LitPair { groups: 0, ..p }.next_group(), 2);
    }

    #[test]
    fn the_shape_follows_the_settings() {
        let mut c = on(32, Style::Filter);
        assert_eq!(shape(&c, 0), Shape { style: Style::Filter, fade: 4, step: 1, lead: 5 });
        c.effects = false;
        assert_eq!(shape(&c, 0).lead, 2);
        // auto rotates
        c = on(32, Style::Auto);
        let seq: Vec<Style> = (0..4).map(|n| shape(&c, n).style).collect();
        assert_eq!(seq, vec![Style::Filter, Style::Echo, Style::Roll, Style::Filter]);
        // a fade is held under half the phrase: its tail is over before the next one starts
        let c = AutoDjConfig { fade: 8, ..on(8, Style::Echo) };
        assert_eq!(shape(&c, 0).fade, 3);
        assert!(shape(&c, 0).lead < 8 - 3);
        assert_eq!(shape(&AutoDjConfig { fade: 8, ..on(16, Style::Roll) }, 0), Shape { style: Style::Roll, fade: 7, step: 1, lead: 5 });
    }

    #[test]
    fn lines_count_forward_wraps_and_ignore_jitter() {
        assert_eq!(lines_crossed(3.9, 4.1), 1);
        assert_eq!(lines_crossed(0.0, 0.1), 0);
        assert_eq!(lines_crossed(3.0, 9.0), 2);
        // a wrap or a swap to the top is a line
        assert_eq!(lines_crossed(15.9, 0.1), 1);
        // the platter's jitter is not
        assert_eq!(lines_crossed(4.05, 3.99), 0);
    }

    #[test]
    fn a_pair_moves_to_the_next_group_on_the_phrase_line_with_a_riser_and_a_sweep() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        h.beat = [10.0, 10.0]; // bar 2.5: the first boundary is bar 16 (the next multiple of 16)
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(9.0); // bar 11.5: the lead is 5 bars: the plan started at bar 11
        assert!(r.dj.running(), "{}", r.dj.text());
        assert!(r.h.said("riser").is_empty(), "the riser starts four bars out");
        r.run(1.0);
        // the sweep and the riser start four bars before the line (bar 12) and end on it
        assert_eq!(r.h.said("riser true bars 4").len(), 2, "both decks: {:?}", r.h.log);
        assert!(r.h.said("filter true bars 4").iter().all(|l| l.contains("@12.0")), "{:?}", r.h.log);
        r.run(4.6);
        assert_eq!(r.h.said("load_pair techno 3").len(), 1, "{:?}", r.h.log);
        let off = r.h.said("filter false");
        assert_eq!(off.len(), 2);
        assert!(off.iter().all(|l| l.contains("@16.0")), "released on the line: {off:?}");
        assert_eq!(r.dj.transitions, 1);
        assert!(r.h.beat[0] < 8.0, "the new pair started from its top: {}", r.h.beat[0]);
        let j = r.dj.to_json();
        assert_eq!(j.get("transitions").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(j.get("state").and_then(|v| v.as_str()), Some("waiting"));
        // a line into the next 16-bar run
        assert_eq!(j.get("bars_left").and_then(|v| v.as_f64()), Some(15.0));
    }

    #[test]
    fn a_pair_goes_round_the_groups_and_wraps() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, on(8, Style::Echo));
        r.run(8.0 * 3.0 + 4.0);
        let loads: Vec<_> = r.h.said("load_pair");
        assert_eq!(loads.len(), 3, "{:?}", r.h.log);
        assert!(loads[0].contains("techno 3") && loads[1].contains("techno 1") && loads[2].contains("techno 2"));
        // the delay is thrown a bar before each handover and let go on it
        assert_eq!(r.h.said("delay true").len(), 6);
        assert_eq!(r.h.said("delay false").len(), 6);
    }

    #[test]
    fn the_handover_lands_every_n_bars() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, AutoDjConfig { effects: false, ..on(16, Style::Auto) });
        r.run(80.5);
        assert_eq!(r.h.said("load_pair").len(), 5, "80 bars at 16: {:?}", r.h.log);
        // no effects: the bare handover, nothing else touched
        assert!(r.h.said("riser").is_empty() && r.h.said("filter").is_empty() && r.h.said("delay").is_empty());
    }

    #[test]
    fn a_two_deck_mix_swaps_the_bass_cuts_over_and_steps_the_outgoing_deck() {
        let mut h = Fake::new();
        h.mute[1] = true; // B waits, muted, silent
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(17.5);
        // the filter exit on A: it sweeps from bar 12, mutes on the downbeat of bar 16
        let mute_a: Vec<_> = r.h.said("A mute true");
        assert_eq!(mute_a.len(), 1, "{:?}", r.h.log);
        assert!(mute_a[0].contains("@16.0"), "{mute_a:?}");
        assert!(!r.h.mute[1], "B was brought up");
        assert_eq!(r.h.said("xfade B").len(), 1);
        assert!(r.h.said("xfade B")[0].contains("@16.0"));
        assert_eq!(r.h.low[1], 0.5, "B's lows came back on the line");
        assert_eq!(r.h.low[0], 0.0, "A's lows went");
        // A is on a new seed after (one bar on)
        assert_eq!(r.h.said("A seed").len(), 1);
        assert_eq!(r.h.said("riser true bars 4").len(), 1);
        assert_eq!(r.dj.transitions, 1);
        // the next time round B plays out and A comes in
        r.run(16.0);
        assert_eq!(r.h.said("B mute true").len(), 1, "{:?}", r.h.log);
        assert!(r.h.said("B mute true")[0].contains("@32.0"));
        assert_eq!(r.h.said("xfade A").len(), 1);
        assert!(!r.h.mute[0], "A was brought up for its turn");
    }

    #[test]
    fn echo_rings_out_from_the_line_and_cuts_after_the_tail() {
        let mut h = Fake::new();
        h.mute[1] = true;
        let mut r = Rig::new(h, AutoDjConfig { fade: 2, ..on(16, Style::Echo) });
        r.run(16.5);
        assert_eq!(r.h.said("A wash true").len(), 1, "{:?}", r.h.log);
        assert!(r.h.said("A wash true")[0].contains("@16.0"));
        assert!(r.h.said("A mute").is_empty(), "the tail rings first");
        r.run(2.0);
        assert_eq!(r.h.said("A mute true").len(), 1);
        assert!(r.h.said("A mute true")[0].contains("@18.0"));
    }

    #[test]
    fn a_roll_drops_on_the_line() {
        let mut r = Rig::new(Fake::new(), on(16, Style::Roll));
        r.h.pair = Some(pair());
        r.run(16.5);
        // the roll on both decks of the pair: four stutter stages (a net deck), released on the line
        assert_eq!(r.h.said("stutter true").len(), 8, "{:?}", r.h.log);
        assert_eq!(r.h.said("stutter false").len(), 2, "released on the drop");
        assert_eq!(r.h.said("load_pair techno 3").len(), 1);
    }

    #[test]
    fn one_net_deck_steps_to_its_next_seed_on_the_line() {
        let mut h = Fake::new();
        h.playing[1] = false;
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(16.2);
        let seed = r.h.said("A seed");
        assert_eq!(seed.len(), 1, "{:?}", r.h.log);
        // asked a bar early so it is in on the line
        assert!(seed[0].contains("@15.0") || seed[0].contains("@15.1"), "{seed:?}");
        // the filter rides the same deck, ending on the line
        assert_eq!(r.h.said("filter true bars 4").len(), 1);
        assert!(r.h.said("filter false")[0].contains("@16.0"));
    }

    #[test]
    fn a_deck_with_nothing_to_step_is_idle_with_its_reason() {
        let mut h = Fake::new();
        h.playing[1] = false;
        h.net[0] = false;
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(30.0);
        assert!(r.h.log.is_empty(), "{:?}", r.h.log);
        assert!(r.dj.word.contains("no composed net"), "{}", r.dj.word);
        assert_eq!(r.dj.transitions, 0);
    }

    #[test]
    fn a_stopped_booth_is_idle_and_a_transition_in_flight_is_dropped() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(13.0);
        assert!(r.dj.running(), "{}", r.dj.text());
        assert_eq!(r.h.said("filter true").len(), 2);
        r.h.playing = [false; 2];
        r.run(0.5);
        assert!(!r.dj.running());
        assert_eq!(r.dj.text(), "no deck playing");
        assert_eq!(r.h.said("filter false").len(), 2, "the sweep let go: {:?}", r.h.log);
    }

    #[test]
    fn turning_it_off_mid_transition_lets_go() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, on(16, Style::Filter));
        r.run(13.0);
        assert!(r.dj.running());
        r.dj.set_config(AutoDjConfig { enabled: false, ..r.dj.cfg.clone() });
        r.run(0.1);
        assert!(!r.dj.running() && !r.dj.busy());
        assert_eq!(r.h.said("filter false").len(), 2);
        r.run(20.0);
        assert!(r.h.said("load_pair").is_empty(), "nothing lands after off");
    }

    #[test]
    fn next_runs_one_transition_even_while_off() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, AutoDjConfig { enabled: false, ..on(32, Style::Filter) });
        r.dj.next().unwrap();
        r.run(1.0);
        assert!(r.dj.running());
        assert!(r.dj.next().is_err());
        r.run(7.0);
        assert_eq!(r.h.said("load_pair techno 3").len(), 1, "{:?}", r.h.log);
        assert!(!r.dj.busy(), "one only");
        r.run(40.0);
        assert_eq!(r.h.said("load_pair").len(), 1);
    }

    #[test]
    fn a_looping_deck_whose_beat_wraps_still_counts_its_bars() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        h.wrap = Some(8.0); // a two-bar loop
        let mut r = Rig::new(h, AutoDjConfig { effects: false, ..on(8, Style::Filter) });
        r.run(8.0 * 2.0 + 3.0);
        assert_eq!(r.h.said("load_pair").len(), 2, "{:?}", r.h.log);
    }

    #[test]
    fn a_phrase_change_restarts_the_count() {
        let mut h = Fake::new();
        h.pair = Some(pair());
        let mut r = Rig::new(h, AutoDjConfig { effects: false, ..on(32, Style::Filter) });
        r.run(10.0);
        r.dj.set_config(AutoDjConfig { bars: 8, ..r.dj.cfg.clone() });
        r.run(8.0);
        assert!(!r.h.said("load_pair").is_empty(), "{:?}", r.h.log);
        assert_eq!(clamp_bars(2), MIN_BARS);
        assert_eq!(clamp_bars(500), MAX_BARS);
    }

    #[test]
    fn the_plan_of_a_mix_is_the_perform_moves_on_their_lines() {
        let cfg = on(16, Style::Filter);
        let sh = shape(&cfg, 0);
        let cues = build(&Mode::Mix { out: "A".into(), inn: "B".into() }, &cfg, sh);
        let kinds: Vec<(i64, String)> = cues
            .iter()
            .map(|c| (c.off, match &c.op {
                Op::Arm(s) => format!("{}:{}:{}", s.kind.name(), s.deck, s.bars),
                other => other.text(),
            }))
            .collect();
        assert_eq!(kinds[0], (-5, "B unmuted".to_string()));
        assert!(kinds.contains(&(-5, "filter_exit:A:4".to_string())), "{kinds:?}");
        assert!(kinds.contains(&(-1, "bass_swap:B:1".to_string())));
        assert!(kinds.contains(&(-1, "phrase_sync:B:1".to_string())));
        assert!(kinds.contains(&(1, "A next seed".to_string())));
    }
}
