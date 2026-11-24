//! Hot reload: the runtime that holds the running program, takes a new one, diffs it, and applies
//! it on the next bar. Pure behind [`LiveHost`]: djapp provides the real decks and registry, the
//! tests a fake host on a fake clock.
//!
//! The rules (docs/BEATSCRIPT.md, "Evaluating and hot reload"):
//!
//! - an evaluation with an error changes nothing: the last good program plays on, the errors are kept;
//! - a good program is staged, and applied in the first three quarters of a bar (so the engine has
//!   the rest of the bar to prepare it), landing on the next bar line (`Engine::load_prepared`
//!   installs at the next bar); one evaluated in the last beat waits for the next bar's first part
//!   and lands a bar later (the answer says which bar);
//! - what did not change is not touched: a deck whose net CID is the same is left alone, a control
//!   line already running keeps its state;
//! - a deck's new net starts where the old one was in its alternation.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};

use crate::ast::Deck;
use crate::compile::{check_with, Call, Compiled, Control, DeckIr, Effect, Ramp, Timing};
use crate::shape::LfoRun;
use crate::env::Env;
use crate::diag::{Diagnostic, Severity, Span};

pub const BEATS_PER_BAR: f64 = 4.0;
/// Bars in a phrase.
pub const PHRASE: u32 = 8;
/// Apply an evaluation only in the first three quarters of a bar.
pub const APPLY_BY: f64 = 0.75;
/// A ramp sends a new value only when it moved this far (the offline render, `djhost::beatscript_render`, thins its samples by the same).
pub const RAMP_EPS: f64 = 0.002;
/// Positions closer than this (in bars) are the same place: a time is reached when the position is within it.
const EPS: f64 = 1e-9;
/// The most times one line that repeats inside a bar may fire in one pass (a stalled clock catches up, not floods).
const MAX_CATCH_UP: i64 = 32;

/// Where the grid deck is.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub deck: String,
    /// Beats from the grid's origin (bar 0's downbeat).
    pub beat: f64,
    pub bpm: f64,
}

/// What the runtime acts on.
pub trait LiveHost {
    /// The deck whose bars are counted (a deck that plays; the program's own first): None when
    /// nothing plays.
    fn grid(&self) -> Option<Grid>;
    fn playing(&self, deck: &str) -> bool;
    /// Put a raw-nets envelope on `deck`: the engine lands it on the next bar line when the deck
    /// plays and at once when it is stopped.
    fn load_net(&mut self, deck: &str, envelope: &JsonValue) -> Result<(), String>;
    fn set_playing(&mut self, deck: &str, on: bool);
    /// Set a role's sound (`sound=synth:reese`) or a drum kit (`kit:drums-808`) on a deck whose net
    /// has landed. A host with no sounds to set leaves it.
    fn set_sound(&mut self, _deck: &str, _role: &str, _option: &str) -> Result<(), String> {
        Ok(())
    }
    /// One call on the control-language registry.
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String>;
    /// Set the tempo (BPM) of the net on `deck`: what a program's tempo curve does, each service pass, from the program's
    /// position (F3, tempo.rs). The default is the registry's `deck tempo`; a host that cannot returns an error, which
    /// the runtime says once in its words (`btw get`) and does not repeat.
    fn set_tempo(&mut self, deck: &str, bpm: f64) -> Result<(), String> {
        let p = JsonValue::Object(JsonObject::from_pairs([("action", JsonValue::str("tempo")), ("deck", JsonValue::str(deck)), ("bpm", JsonValue::Number(bpm))]));
        let r = self.call("deck", &p)?;
        match r.get("applied").and_then(JsonValue::as_str) {
            Some("ignored") => Err(format!("deck {deck} ignored the tempo")),
            _ => Ok(()),
        }
    }
    /// The tempo (BPM) a deck plays at now, so that the program can put it back when it stops or ends (it sets the tempo each pass
    /// from its curve, F3): None when the deck has no clock (empty, stopped with nothing on it) or the host cannot say, and then
    /// nothing is put back for it. Asked once for each deck, when a program that drives the tempo is applied, before its nets land.
    fn tempo_of(&self, _deck: &str) -> Option<f64> {
        None
    }
    /// A performed input (F13: `knob(3)`, `pad(5)`, `fader(A)`, `held(key)`, `held(c4)` of the language) as the controllers have it
    /// now: 0..1, or None when nothing answers for it (the program reads 0 and says so in `btw get`'s words). `kind` is `knob`,
    /// `pad`, `fader` or `held`; `id` the knob or pad number (`3`), the deck (`A`), `key` or the note (`c4`:
    /// `perform::note_number` is its MIDI number). Asked once a bar for each input a running program names.
    fn input(&mut self, _kind: &str, _id: &str) -> Option<f64> {
        None
    }
}

/// Bar lines crossed between two looks at a deck's beat (`djhost::autodj::lines_crossed`: a jump
/// back is a swap to the top, a line too; a small step back is jitter).
pub fn lines_crossed(prev: f64, cur: f64) -> i64 {
    let bar = |b: f64| (b / BEATS_PER_BAR + 1e-9).floor() as i64;
    if cur + 2.0 >= prev {
        (bar(cur) - bar(prev)).max(0)
    } else {
        1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckChange {
    Added,
    Changed,
    Removed,
    Same,
}

impl DeckChange {
    pub fn name(self) -> &'static str {
        match self {
            DeckChange::Added => "added",
            DeckChange::Changed => "changed",
            DeckChange::Removed => "removed",
            DeckChange::Same => "same",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeckDiff {
    pub deck: Deck,
    /// The section (an arranged song): None for a song with no timeline.
    pub section: Option<String>,
    pub change: DeckChange,
    /// Which parts: "kick changed", "bass added", "snare removed".
    pub parts: Vec<String>,
}

/// What an evaluation will do, from the diff.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Plan {
    pub decks: Vec<DeckDiff>,
    pub controls_added: Vec<String>,
    pub controls_removed: Vec<String>,
    pub controls_kept: usize,
    /// The tempo curve is another one (F3, tempo.rs): a tempo line is not a control, so it is said here.
    pub tempo_changed: bool,
}

impl Plan {
    /// Nothing to do.
    pub fn is_empty(&self) -> bool {
        self.decks.iter().all(|d| d.change == DeckChange::Same) && self.controls_added.is_empty() && self.controls_removed.is_empty() && !self.tempo_changed
    }

    /// How many things change, as (added, changed, removed): decks and controls, and the tempo curve (one change).
    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |c: DeckChange| self.decks.iter().filter(|d| d.change == c).count();
        (n(DeckChange::Added) + self.controls_added.len(), n(DeckChange::Changed) + usize::from(self.tempo_changed), n(DeckChange::Removed) + self.controls_removed.len())
    }

    /// `+12 added · 3 changed · 1 removed` (the kinds there are none of are left out; no change at all says so).
    pub fn summary(&self) -> String {
        let (a, c, r) = self.counts();
        let parts: Vec<String> = [(a, "+", "added"), (c, "", "changed"), (r, "", "removed")].iter().filter(|(n, _, _)| *n > 0).map(|(n, sign, w)| format!("{sign}{n} {w}")).collect();
        if parts.is_empty() {
            "no changes".to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// One line a person can read.
    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        for d in &self.decks {
            if d.change == DeckChange::Same {
                continue;
            }
            let what = if d.parts.is_empty() { d.change.name().to_string() } else { d.parts.join(", ") };
            match &d.section {
                Some(sec) => v.push(format!("{sec} deck {}: {}", d.deck.name(), what)),
                None => v.push(format!("deck {}: {}", d.deck.name(), what)),
            }
        }
        for c in &self.controls_added {
            v.push(format!("new: {c}"));
        }
        for c in &self.controls_removed {
            v.push(format!("gone: {c}"));
        }
        if self.tempo_changed {
            v.push("tempo curve changed".to_string());
        }
        v
    }

    pub fn to_json(&self) -> JsonValue {
        let decks = self
            .decks
            .iter()
            .map(|d| {
                JsonValue::Object(JsonObject::from_pairs([
                    ("deck", JsonValue::str(d.deck.name())),
                    ("section", d.section.clone().map_or(JsonValue::Null, JsonValue::str)),
                    ("change", JsonValue::str(d.change.name())),
                    ("parts", JsonValue::Array(d.parts.iter().map(|p| JsonValue::str(p.clone())).collect())),
                ]))
            })
            .collect();
        let strs = |v: &[String]| JsonValue::Array(v.iter().map(|s| JsonValue::str(s.clone())).collect());
        JsonValue::Object(JsonObject::from_pairs([
            ("decks", JsonValue::Array(decks)),
            ("controls_added", strs(&self.controls_added)),
            ("controls_removed", strs(&self.controls_removed)),
            ("controls_kept", JsonValue::int(self.controls_kept as i64)),
            ("tempo_changed", self.tempo_changed.into()),
            ("lines", strs(&self.lines())),
        ]))
    }
}

/// Every net of a program: (section, deck, the net).
fn nets_of(c: &Compiled) -> Vec<(Option<String>, Deck, &DeckIr)> {
    if c.arranged {
        c.segments.iter().flat_map(|s| s.decks.iter().map(move |(d, ir)| (Some(format!("{}@{}", s.name, s.start)), *d, ir))).collect()
    } else {
        c.decks.iter().map(|(d, ir)| (None, *d, ir)).collect()
    }
}

/// The difference between what runs (`old`) and what was evaluated (`new`).
pub fn diff(old: Option<&Compiled>, new: &Compiled) -> Plan {
    let mut plan = Plan::default();
    let news = nets_of(new);
    let olds = old.map(nets_of).unwrap_or_default();
    let mut keys: Vec<(Option<String>, Deck)> = news.iter().map(|(s, d, _)| (s.clone(), *d)).collect();
    for (s, d, _) in &olds {
        if !keys.iter().any(|(ks, kd)| ks == s && kd == d) {
            keys.push((s.clone(), *d));
        }
    }
    keys.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    for (section, d) in keys {
        let n = news.iter().find(|(s, dd, _)| *s == section && *dd == d).map(|x| x.2);
        let o = olds.iter().find(|(s, dd, _)| *s == section && *dd == d).map(|x| x.2);
        let (change, parts) = match (o, n) {
            (None, Some(_)) => (DeckChange::Added, Vec::new()),
            (Some(_), None) => (DeckChange::Removed, Vec::new()),
            (Some(o), Some(n)) => {
                if o.load_key() == n.load_key() {
                    (DeckChange::Same, Vec::new())
                } else {
                    let (fo, fn_) = (o.fingerprints(), n.fingerprints());
                    let mut parts = Vec::new();
                    for (role, f) in &fn_ {
                        match fo.get(role) {
                            None => parts.push(format!("{role} added")),
                            Some(g) if g != f => parts.push(format!("{role} changed")),
                            _ => {}
                        }
                    }
                    for role in fo.keys() {
                        if !fn_.contains_key(role) {
                            parts.push(format!("{role} removed"));
                        }
                    }
                    if parts.is_empty() {
                        parts.push("tempo, swing or key changed".into());
                    }
                    (DeckChange::Changed, parts)
                }
            }
            (None, None) => continue,
        };
        plan.decks.push(DeckDiff { deck: d, section, change, parts });
    }
    let old_keys: Vec<&str> = old.map(|o| o.controls.iter().map(|c| c.key.as_str()).collect()).unwrap_or_default();
    for c in &new.controls {
        if old_keys.contains(&c.key.as_str()) {
            plan.controls_kept += 1;
        } else {
            plan.controls_added.push(c.key.clone());
        }
    }
    for k in old_keys {
        if !new.controls.iter().any(|c| c.key == k) {
            plan.controls_removed.push(k.to_string());
        }
    }
    // a tempo curve is not a control line: another curve is said here
    if let Some(o) = old {
        plan.tempo_changed = (o.tempo_map().drives() || new.tempo_map().drives()) && !o.tempo_map().same_tempos(new.tempo_map());
    }
    plan
}

/// What `eval` and `check` answer.
#[derive(Clone, Debug)]
pub struct Eval {
    pub ok: bool,
    /// Errors and warnings.
    pub diagnostics: Vec<Diagnostic>,
    pub cid: Option<String>,
    pub canonical: Option<String>,
    /// Deck → net CID.
    pub nets: BTreeMap<String, String>,
    pub plan: Option<Plan>,
    /// The program bar it lands on (None: at once, nothing plays; or not staged).
    pub lands: Option<i64>,
    /// The same program is already running and nothing is staged.
    pub unchanged: bool,
    /// The generations of every `evolve`, each with the one it came from.
    pub lineage: Vec<crate::compile::Generation>,
    /// What each `evolve` comes to: its tail and period, in bars and in time.
    pub evolve: Vec<String>,
    /// What each `duck` and `mix` line comes to: the registry calls it makes and when it is put back.
    pub mix: Vec<String>,
    /// How long the program lasts, through its tempo curve: `length: 3m42s with the tempo curve, 64 bars` (F3, tempo.rs).
    pub length: Option<String>,
    /// A set's timeline and mixes (set.rs), as JSON: what `btw check` says of it.
    pub set: Option<JsonValue>,
}

struct RampRun {
    ramp: Ramp,
    /// The program position (in bars, fractional) it began at: the time it was due, not the pass that saw it.
    start: f64,
    last: Option<f64>,
    done: bool,
    /// A ramp that names an input (F13): its ends are read again at every bar boundary and kept inside the bar.
    live: Option<Box<crate::perform::Performed>>,
}

struct Item {
    control: Control,
    start_line: i64,
    fired: bool,
    ramp: Option<RampRun>,
    /// An lfo that is running (F4, shape.rs).
    lfo: Option<LfoRun>,
    /// `for N bars`, `for 2 beats`: the program position (in bars) the effect is put back at.
    undo_at: Option<f64>,
    /// What a mix read off the mixer before it changed it: the calls that put it back (mixmoves.rs).
    snapshot: Vec<Call>,
    /// The loop-row clip a `sing` line put its voice on (the `id` `vocal to_row` answered): it loops until it is stopped, so it is
    /// stopped when the line is removed by an edit, when the program stops or ends, and where a lap of a looped song begins again.
    clip: Option<String>,
    /// What a line that follows its input (R1, [`crate::perform::Follow`]) sent last: it sends again only when the value it reads has
    /// moved, and says so in the words once, when it first sends.
    sent: Option<Effect>,
}

impl Item {
    /// Where the line follows its input, if it does: a plain value with no time word (perform.rs, R1).
    fn follow(&self) -> Option<crate::perform::Follow> {
        match &self.control.effect {
            Effect::Performed(p) => p.follow,
            _ => None,
        }
    }

    /// The call that stops the clip a `sing` line is looping (None: it has none).
    fn clip_stop(&self) -> Option<Call> {
        self.clip.as_ref().map(|id| Call { noun: "loops".into(), params: JsonValue::Object(JsonObject::from_pairs([("action", JsonValue::str("stop")), ("id", JsonValue::str(id.clone()))])) })
    }

    /// The calls that put back what this line changed: its own, and a mix's snapshot.
    fn undo_calls(&self) -> Vec<Call> {
        let mut v = self.control.undo.clone();
        v.extend(self.snapshot.iter().cloned());
        v
    }
}

/// A role's sound to set once its net has landed.
struct PendingSound {
    deck: Deck,
    /// The bar the net lands on.
    at: i64,
    sounds: Vec<(String, String)>,
}

struct Running {
    compiled: Compiled,
    /// What each deck was given: the net CID and its sounds (`DeckIr::load_key`).
    loaded: BTreeMap<Deck, String>,
    items: Vec<Item>,
    /// The program bar its first bar is on: an arranged song counts its bars from here (and from here again each lap of a loop).
    origin: i64,
    /// The program bar the program's music first landed on: never moves. A program with no sections (or one chained from nets) counts
    /// its tempo curve from here (`tempo0`). When a deck played as the program was evaluated that is the bar after the one it was
    /// evaluated in (the bar `clock0` calls bar 1); when nothing played it is bar 0, the same bar. (The offline render starts the music
    /// on its bar 0.)
    first: i64,
    /// The program's net CIDs, as `Compiled::net_cids` has them (kept: the status says them and hashing a net costs a millisecond).
    nets: BTreeMap<String, String>,
    /// The nets' keys, by section (one entry for a song with no timeline).
    keys: Vec<BTreeMap<Deck, String>>,
    pending: Vec<PendingSound>,
    /// Decks the next stretch has no net for: they play out the bar they are on and stop on the bar line (deck, bar).
    stops: Vec<(Deck, i64)>,
    /// The bar a section's start last loaded its nets for (`sync_nets`): a section that starts again (a `loop`'s lap, a section
    /// named twice in `arrange`) loads them once more, from its first bar, though they are the nets playing.
    restarted: Option<i64>,
    /// A scene a pad brought in, whose first nets are loaded for the bar line it lands on (launch.rs): the song goes on until then.
    landing: Option<Landing>,
    /// The scene that plays now, if a pad brought one in: `compiled` is then that scene as a program of its own (the same text and CID),
    /// and `base` is the program as written, which the launches, the edits and the diff are of.
    scene: Option<String>,
    base: Option<Compiled>,
}

/// A scene decided, waiting for its bar line.
struct Landing {
    scene: String,
    /// The program bar it lands on.
    at: i64,
    /// The scene as a program of its own.
    program: Box<Compiled>,
}

/// A scene a hit of a pad has asked for, waiting for its bar line: a later hit before the decision takes its place.
struct Queued {
    scene: String,
    /// Every this many bars, counted from the song's first bar.
    quantum: u32,
}

impl Running {
    /// The program as written: what a launch is of and an edit replaces, whichever scene plays.
    fn written(&self) -> &Compiled {
        self.base.as_ref().unwrap_or(&self.compiled)
    }

    /// The program bar the program's own bar 0 is on: what `at bar N`, `every N bars`, `on phrase` and `end at bar N` count from. A song
    /// counts from its `origin` (the bar its first section plays on, each lap of a loop again); a program with no sections (or one
    /// chained from nets) from bar 0 of the clock, the bar it was evaluated in, as it always has: when a deck played at that time the
    /// music lands on bar 1 (the net lands on the bar line after), and `at bar 1` is the first bar you hear. docs/BEATSCRIPT.md, "Time words".
    fn clock0(&self) -> i64 {
        if self.compiled.arranged && !self.compiled.flat_clock {
            self.origin
        } else {
            0
        }
    }

    /// The program bar the tempo curve's own bar 0 is on. A song's is its `origin`, as for the lines; a program with no sections has
    /// its curve start on the bar its music first lands on (`first`, the first bar you hear: the bar after the one it was evaluated in
    /// when a deck played, as in the render), not a silent bar before. Only a tempo curve counts from here (no program written before
    /// the curve existed has one).
    fn tempo0(&self) -> i64 {
        if self.compiled.arranged && !self.compiled.flat_clock {
            self.origin
        } else {
            self.first
        }
    }
}

/// Where the lfo of an effect says its section ends (`Some(None)`: it has an lfo that does not stop with its section; None: no lfo).
fn lfo_until(e: &Effect) -> Option<Option<u32>> {
    match e {
        Effect::Lfo(l) => Some(l.until),
        Effect::Performed(p) => lfo_until(&p.base),
        _ => None,
    }
}

fn keys_of(c: &Compiled) -> Vec<BTreeMap<Deck, String>> {
    if c.arranged {
        c.segments.iter().map(|s| s.decks.iter().map(|(d, ir)| (*d, ir.load_key())).collect()).collect()
    } else {
        vec![c.decks.iter().map(|(d, ir)| (*d, ir.load_key())).collect()]
    }
}

struct Staged {
    compiled: Compiled,
    plan: Plan,
    /// The program's net CIDs (deck, section → CID), worked out once, when it was evaluated: hashing every net again on each look at the
    /// status was R-07's cost.
    nets: BTreeMap<String, String>,
    /// A scene to start in (a pad hit after the song ended): the program is the one as written, and it begins as the scene.
    scene: Option<String>,
}

/// A program that came to its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Ended {
    pub cid: String,
    /// The program bar it ended on.
    pub bar: i64,
}

/// A hit that sounded: which part of which deck, and every place in the text it is (its token, the terms
/// round it, its `let` and `def` bodies, the line, the calls). `at` is the program bar (fractional) it fell on.
#[derive(Clone, Debug, PartialEq)]
pub struct Fire {
    pub deck: Deck,
    pub role: String,
    pub spans: Vec<Span>,
    /// How many of the last of `spans` are call sites (the calls and repeats that expanded its line).
    pub calls: usize,
    pub at: f64,
}

/// A control line that fired.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlFire {
    pub span: Span,
    pub via: Vec<Span>,
    pub key: String,
    pub at: f64,
}

pub struct Live {
    running: Option<Running>,
    /// Where the program was, in bars, at the last pass (what the firings are taken since).
    pos: Option<f64>,
    /// The hits of the last two bars, newest last.
    fires: std::collections::VecDeque<Fire>,
    cfires: Vec<ControlFire>,
    last_gen: Option<u32>,
    /// The generation that just began, and the bar it began on.
    gen_flash: Option<(u32, f64)>,
    /// How the last program finished, until another is evaluated.
    pub ended: Option<Ended>,
    staged: Option<Staged>,
    /// The last evaluation's errors (and warnings), and the text they are about.
    pub errors: Vec<Diagnostic>,
    pub error_text: Option<String>,
    /// The program bar (0 = the bar it started on) and how far into it.
    line: i64,
    frac: f64,
    /// Where the program was, in bars (line and fraction), at the last pass: what a time between bar lines is
    /// measured against (a line with a time that is not a bar line fires when the position crosses it).
    sub_pos: Option<f64>,
    anchored: bool,
    last: Option<(String, f64)>,
    /// The tempo last sent to each deck by the program's tempo curve (F3): nothing is sent again until it moves.
    tempo_sent: BTreeMap<Deck, f64>,
    /// The decks whose host could not take a tempo, so it is not tried again (said once in `words`).
    tempo_failed: Vec<Deck>,
    /// What each deck's tempo was before the program's curve first took it (read when the program was applied, before its nets
    /// landed), and the decks the curve has set: the program puts those back when it stops or ends (R-04).
    tempo_before: BTreeMap<Deck, f64>,
    tempo_set: Vec<Deck>,
    /// What happened, newest last.
    pub words: Vec<String>,
    /// The performed inputs: the bar's values, the take being logged, the performance being replayed (F13, perform.rs).
    pub(crate) perf: crate::perform::PerfState,
    /// The scene a pad has asked for, until it is decided (launch.rs).
    queued: Option<Queued>,
    /// Which pads of the launches were down at the last pass: a hit is a pad that was up and is down.
    pad_down: BTreeMap<u32, bool>,
    /// The program that came to its end, while it has launches: a pad hit after the end starts its scene (a dead pad would be a stop).
    finished: Option<Compiled>,
}

impl Default for Live {
    fn default() -> Self {
        Live::new()
    }
}

impl Live {
    pub fn new() -> Live {
        Live { running: None, pos: None, fires: Default::default(), cfires: Vec::new(), last_gen: None, gen_flash: None, ended: None, staged: None, errors: Vec::new(), error_text: None, line: 0, frac: 0.0, sub_pos: None, anchored: false, last: None, tempo_sent: BTreeMap::new(), tempo_failed: Vec::new(), tempo_before: BTreeMap::new(), tempo_set: Vec::new(), words: Vec::new(), perf: Default::default(), queued: None, pad_down: BTreeMap::new(), finished: None }
    }

    fn word(&mut self, w: impl Into<String>) {
        self.words.push(w.into());
        if self.words.len() > 100 {
            self.words.remove(0);
        }
    }

    pub fn line(&self) -> i64 {
        self.line
    }

    pub fn frac(&self) -> f64 {
        self.frac
    }

    pub fn running_cid(&self) -> Option<&str> {
        self.running.as_ref().map(|r| r.compiled.cid.as_str())
    }

    /// The CID of the program staged to land on a coming bar line, and the bar it lands on: what the window says of it (cheap: the
    /// window asks every frame, which `status` does not suit).
    pub fn staged_cid(&self) -> Option<&str> {
        self.staged.as_ref().map(|s| s.compiled.cid.as_str())
    }

    /// The bar the staged program lands on (None: at once, nothing plays).
    pub fn staged_lands(&self) -> Option<i64> {
        self.staged.as_ref().and_then(|_| self.lands_now())
    }

    /// The generation of an `evolve` playing now.
    pub fn generation(&self) -> Option<u32> {
        let r = self.running.as_ref()?;
        r.compiled.generation_at(self.line - r.origin)
    }

    pub fn running_text(&self) -> Option<&str> {
        self.running.as_ref().map(|r| r.compiled.text.as_str())
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn has_staged(&self) -> bool {
        self.staged.is_some()
    }

    /// Needs the service pass (something runs, or waits to).
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.staged.is_some()
    }

    /// The song has ended and a pad that launches a scene is waiting for a hit (launch.rs): the pass still watches the pads.
    pub fn armed(&self) -> bool {
        self.finished.is_some()
    }

    /// The bar a program applied now would land on: the next bar when the clock runs and there is
    /// time, the one after when there is not; None when nothing plays (it starts at once).
    fn lands_now(&self) -> Option<i64> {
        if !self.anchored {
            return None;
        }
        // a program that is not running yet starts counting at its own bar 0
        let now = if self.running.is_none() { 0 } else { self.line };
        Some(now + if self.frac <= APPLY_BY { 1 } else { 2 })
    }

    /// What applying `comp` would change of what runs. A scene that plays is a program of its own, so it is diffed with the same scene of
    /// the new program (an edit lands on it), not with the song as written.
    fn plan_for(&self, comp: &Compiled) -> Plan {
        let new = self.running.as_ref().and_then(|r| r.scene.as_deref()).and_then(|n| comp.launches.scene(n)).unwrap_or(comp);
        diff(self.running.as_ref().map(|r| &r.compiled), new)
    }

    /// Check without staging.
    pub fn check(&self, text: &str) -> Eval {
        self.check_with(text, &Env::none())
    }

    pub fn check_with(&self, text: &str, env: &Env) -> Eval {
        let c = check_with(text, env);
        match c.compiled {
            None => Eval { ok: false, diagnostics: c.diagnostics, cid: None, canonical: None, nets: BTreeMap::new(), plan: None, lands: None, unchanged: false, lineage: Vec::new(), evolve: Vec::new(), mix: Vec::new(), length: None, set: None },
            Some(comp) => {
                let plan = self.plan_for(&comp);
                let unchanged = self.running.as_ref().is_some_and(|r| r.compiled.cid == comp.cid) && self.staged.is_none();
                Eval {
                    ok: true,
                    diagnostics: c.diagnostics,
                    cid: Some(comp.cid.clone()),
                    nets: comp.net_cids(),
                    canonical: Some(comp.text.clone()),
                    plan: Some(plan),
                    lands: self.lands_now(),
                    unchanged,
                    lineage: comp.lineage.clone(),
                    evolve: comp.evolve_report(),
                    mix: comp.mix_report(),
                    length: comp.length_report(),
                    set: comp.set.as_ref().map(|r| r.to_json()),
                }
            }
        }
    }

    /// Evaluate `text`: with an error nothing changes (the last good program plays on); a good
    /// program is staged and applied on the next bar by [`service`](Self::service).
    pub fn eval(&mut self, text: &str) -> Eval {
        self.eval_with(text, &Env::none())
    }

    pub fn eval_with(&mut self, text: &str, env: &Env) -> Eval {
        let c = check_with(text, env);
        self.errors = c.diagnostics.clone();
        self.error_text = Some(text.to_string());
        let Some(comp) = c.compiled else {
            let n = c.diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
            self.word(format!("not applied: {n} error{}; the last good program plays on", if n == 1 { "" } else { "s" }));
            return Eval { ok: false, diagnostics: c.diagnostics, cid: None, canonical: None, nets: BTreeMap::new(), plan: None, lands: None, unchanged: false, lineage: Vec::new(), evolve: Vec::new(), mix: Vec::new(), length: None, set: None };
        };
        let plan = self.plan_for(&comp);
        let unchanged = self.running.as_ref().is_some_and(|r| r.compiled.cid == comp.cid);
        let lands = self.lands_now();
        let ev = Eval {
            ok: true,
            diagnostics: c.diagnostics,
            cid: Some(comp.cid.clone()),
            nets: comp.net_cids(),
            canonical: Some(comp.text.clone()),
            plan: Some(plan.clone()),
            lands,
            unchanged: unchanged && self.staged.is_none(),
            lineage: comp.lineage.clone(),
            evolve: comp.evolve_report(),
            mix: comp.mix_report(),
            length: comp.length_report(),
            set: comp.set.as_ref().map(|r| r.to_json()),
        };
        if unchanged {
            // the same program as the running one: a staged one that differed is dropped
            if self.staged.take().is_some() {
                self.word("back to the running program: the staged one is dropped");
            }
            return ev;
        }
        self.ended = None;
        self.finished = None;
        self.word(format!("staged {} ({})", short(&comp.cid), match lands {
            Some(l) => format!("lands on bar {l}"),
            None => "starts at once".into(),
        }));
        self.staged = Some(Staged { compiled: comp, plan, nets: ev.nets.clone(), scene: None });
        ev
    }

    /// End the program, first putting back what its lines still hold changed (a muted strip, a
    /// filter): the decks play on, the control lines and ramps end.
    pub fn stop_with<H: LiveHost>(&mut self, host: &mut H) {
        if let Some(run) = self.running.take() {
            for it in &run.items {
                if it.undo_at.is_some() {
                    for c in it.undo_calls() {
                        self.call(host, &c.noun, &c.params);
                    }
                }
                // a sung line loops on the loop row until it is stopped
                if let Some(c) = it.clip_stop() {
                    self.call(host, &c.noun, &c.params);
                }
            }
            self.running = Some(run);
        }
        self.put_tempo_back(host);
        self.stop();
    }

    /// The tempo the decks had before the program's curve took it goes back (what a program that stops or ends leaves is the
    /// deck's own tempo, not wherever the ramp had got to). A deck whose tempo was not known is left where it is.
    fn put_tempo_back<H: LiveHost>(&mut self, host: &mut H) {
        let set = std::mem::take(&mut self.tempo_set);
        let before = std::mem::take(&mut self.tempo_before);
        self.tempo_sent.clear();
        for d in set {
            let Some(bpm) = before.get(&d).copied() else { continue };
            match host.set_tempo(d.name(), bpm) {
                Ok(()) => self.word(format!("tempo put back: deck {} is at {} BPM again", d.name(), crate::print::num(bpm))),
                Err(e) => self.word(format!("tempo: deck {}: {e}", d.name())),
            }
        }
    }

    /// End the program: the decks play on, the control lines and ramps end. (Use [`stop_with`](Self::stop_with)
    /// where there is a host: it puts back what a line still holds changed.)
    pub fn stop(&mut self) {
        self.perf.stopped();
        self.tempo_before.clear();
        self.tempo_set.clear();
        self.queued = None;
        self.finished = None;
        if self.running.take().is_some() || self.staged.take().is_some() {
            self.word("stopped: the decks play on");
        }
    }

    /// One pass: count the bars, run what is due on them, apply what is staged, move the ramps.
    pub fn service<H: LiveHost>(&mut self, host: &mut H) {
        // ---- the clock (watched when idle too, so an evaluation can say which bar it lands on)
        let before = self.line;
        match host.grid() {
            Some(g) => {
                if !self.anchored {
                    self.anchored = true;
                } else if let Some((d, prev)) = &self.last {
                    if *d == g.deck {
                        self.line += lines_crossed(*prev, g.beat);
                    }
                }
                // (the same epsilon as `lines_crossed`: a beat a hair short of a bar line is the bar line)
                self.frac = ((g.beat / BEATS_PER_BAR) + EPS).fract().max(0.0);
                self.last = Some((g.deck.clone(), g.beat));
            }
            None => {
                self.anchored = false;
                self.last = None;
            }
        }
        // ---- the pads that launch a scene: a hit queues it (launch.rs)
        self.poll_pads(host);
        // ---- the bars crossed since the last pass
        for l in before + 1..=self.line {
            self.fire_bar(host, l);
        }
        if self.line > before {
            // a net that landed on the bar line starts at its own tempo: the curve's goes on it again (F3)
            self.tempo_sent.clear();
        }
        // ---- what is due between bar lines: the position crossed a time
        let now = self.anchored.then(|| self.line as f64 + self.frac);
        let prev = self.sub_pos;
        self.sub_pos = now;
        self.fire_between(host, prev, now);
        // ---- what is staged, in the first part of a bar
        if self.staged.is_some() && (!self.anchored || self.frac <= APPLY_BY) {
            self.apply(host);
        }
        // ---- a song with sections: the next bar's nets are loaded in this one's first part
        if self.running.is_some() && (!self.anchored || self.frac <= APPLY_BY) {
            let target = if self.anchored { self.line + 1 } else { self.line };
            if let Some(mut run) = self.running.take() {
                self.decide_landing(host, &mut run, target);
                self.sync_nets(host, &mut run, target);
                self.running = Some(run);
            }
        }
        self.set_pending_sounds(host);
        self.move_ramps(host);
        self.follow_tempo(host);
        self.watch_fires();
    }

    /// Which hits have sounded since the last pass: the program's position (the published beat of the grid deck, the
    /// same clock everything here runs on) moved from `prev` to now, and every hit of the nets playing in between
    /// whose time fell in it is a firing. Nothing here touches the audio thread: the engine publishes where it is.
    fn watch_fires(&mut self) {
        let now = if self.anchored { Some(self.line as f64 + self.frac) } else { None };
        let (prev, pos) = (self.pos, now);
        self.pos = pos;
        let Some(pos) = pos else { return };
        if let Some(gen) = self.running.as_ref().and_then(|r| r.compiled.generation_at(self.line - r.origin)) {
            if self.last_gen != Some(gen) {
                if self.last_gen.is_some() {
                    self.gen_flash = Some((gen, pos));
                }
                self.last_gen = Some(gen);
            }
        } else {
            self.last_gen = None;
        }
        self.fires.retain(|f| f.at > pos - 2.0);
        self.cfires.retain(|f| f.at > pos - 2.0);
        let Some(prev) = prev else { return };
        // a jump (the clock moved a bar or more at once) is not a firing of everything in it
        if !(pos > prev && pos - prev < 1.5) {
            return;
        }
        let Some(run) = &self.running else { return };
        let c = &run.compiled;
        let mut found = Vec::new();
        for b in (prev.floor() as i64)..=(pos.floor() as i64) {
            let nets: Option<(&BTreeMap<Deck, DeckIr>, i64)> = if c.arranged {
                let rel = b - run.origin;
                if rel < 0 {
                    None
                } else {
                    c.segments.iter().rposition(|s| i64::from(s.start) <= rel).map(|i| (&c.segments[i].decks, rel - i64::from(c.segments[i].start)))
                }
            } else {
                Some((&c.decks, b))
            };
            let Some((decks, local)) = nets else { continue };
            for (d, ir) in decks {
                if !run.loaded.contains_key(d) {
                    continue;
                }
                for p in &ir.parts {
                    let n = p.cycles.len().max(1);
                    let Some(hits) = p.cycles.get(local.rem_euclid(n as i64) as usize) else { continue };
                    for h in hits {
                        let t = b as f64 + f64::from(h.step) / 16.0;
                        if t > prev && t <= pos {
                            let spans = p.spans.get(h.sid as usize).cloned().unwrap_or_default();
                            found.push(Fire { deck: *d, role: p.role.clone(), spans, calls: p.via.len(), at: t });
                        }
                    }
                }
            }
        }
        self.fires.extend(found);
        while self.fires.len() > 2000 {
            self.fires.pop_front();
        }
    }

    /// The hits of the last two bars (see [`Fire`]).
    pub fn fires(&self) -> Vec<Fire> {
        self.fires.iter().cloned().collect()
    }

    /// Sounds of nets that have landed.
    fn set_pending_sounds<H: LiveHost>(&mut self, host: &mut H) {
        let Some(run) = self.running.as_mut() else { return };
        let due: Vec<PendingSound> = {
            let (line, anchored) = (self.line, self.anchored);
            let mut due = Vec::new();
            run.pending.retain(|p| {
                if !anchored || line >= p.at {
                    due.push(PendingSound { deck: p.deck, at: p.at, sounds: p.sounds.clone() });
                    false
                } else {
                    true
                }
            });
            due
        };
        for p in due {
            for (role, opt) in &p.sounds {
                if let Err(e) = host.set_sound(p.deck.name(), role, opt) {
                    self.word(format!("deck {} {role}: {e}", p.deck.name()));
                }
            }
        }
    }

    fn call<H: LiveHost>(&mut self, host: &mut H, noun: &str, params: &JsonValue) {
        if let Err(e) = host.call(noun, params) {
            self.word(format!("{noun}: {e}"));
        }
    }

    fn fire_bar<H: LiveHost>(&mut self, host: &mut H, l: i64) {
        let Some(mut run) = self.running.take() else { return };
        // a deck whose last bar has been played stops on the bar line
        let (due, keep): (Vec<(Deck, i64)>, Vec<(Deck, i64)>) = run.stops.drain(..).partition(|s| s.1 <= l);
        run.stops = keep;
        for (d, _) in due {
            host.set_playing(d.name(), false);
        }
        // a scene a pad brought in begins on its bar line, as a lap of the song would (launch.rs)
        if run.landing.as_ref().is_some_and(|ln| l >= ln.at) {
            self.enter_scene(host, &mut run, l);
        }
        let mut off = run.clock0();
        let mut rel = l - off;
        // a song that loops starts again from its first section when it gets to its end
        if let (true, true, Some(total)) = (run.compiled.looped, run.compiled.arranged, run.compiled.song_bars()) {
            if total > 0 && l - run.origin >= i64::from(total) && run.compiled.flat_clock {
                // a program too long for one net goes round again: its nets from the first, its lines on the clock they had
                run.origin += i64::from(total);
                run.pending.clear();
                self.word(format!("bar {l}: the chain of nets starts again"));
            } else if total > 0 && rel >= i64::from(total) && !run.compiled.flat_clock {
                // (not for a chain of nets: its lines count on from bar 0 of the clock and its lap is the branch above; this branch took
                // it at every bar line once the first lap was over)
                for it in &mut run.items {
                    if it.undo_at.take().is_some() {
                        for c in it.undo_calls() {
                            self.call(host, &c.noun, &c.params);
                        }
                        it.snapshot.clear();
                    }
                    // the voice of the lap that ends stops where the next lap's sings it again
                    if let Some(c) = it.clip_stop() {
                        self.call(host, &c.noun, &c.params);
                        it.clip = None;
                    }
                    it.fired = false;
                    it.ramp = None;
                    it.lfo = None;
                    it.sent = None;
                    it.start_line = l;
                }
                run.pending.clear();
                // (a flow of named arrangements goes back to the bar its looping arrangement begins on, not to bar 0)
                run.origin += i64::from(total - run.compiled.loop_from.min(total));
                off = run.origin;
                rel = l - off;
                self.word(format!("bar {l}: the song starts again (loop)"));
            }
        }
        // the end: what the lines still hold changed is put back, the decks stop on the bar line
        if run.compiled.end.is_some_and(|e| rel >= i64::from(e)) {
            for it in &run.items {
                if it.undo_at.is_some() {
                    for c in it.undo_calls() {
                        self.call(host, &c.noun, &c.params);
                    }
                }
                if let Some(c) = it.clip_stop() {
                    self.call(host, &c.noun, &c.params);
                }
            }
            self.put_tempo_back(host);
            let decks: Vec<Deck> = run.loaded.keys().copied().collect();
            for d in decks {
                host.set_playing(d.name(), false);
            }
            self.word(format!("bar {l}: the program ended (bar {}): the decks stop", rel));
            self.ended = Some(Ended { cid: run.compiled.cid.clone(), bar: l });
            self.staged = None;
            self.queued = None;
            // (a pad that launches a scene starts it after the end: the pad is not dead, and the song is not stopped for good)
            self.finished = Some(run.written().clone()).filter(|c| !c.launches.is_empty());
            return;
        }
        // what a `for N bars` line (or a section's mix) did is put back first, whichever line is first in the text: a line
        // that fires on the bar this one ends on (`at bar 8: mix { master level 0.9 }` where a section's mix ends) is the
        // last word, not undone the moment it was said
        for it in &mut run.items {
            if it.undo_at.is_some_and(|u| l as f64 + EPS >= u) {
                it.undo_at = None;
                for c in it.undo_calls() {
                    self.call(host, &c.noun, &c.params);
                }
                it.snapshot.clear();
                self.word(format!("bar {l}: put back {}", it.control.key));
            }
        }
        // the targets the lines fired on this bar set (a section's `master: filter off`): an older ramp on one of them is over
        let mut fired_slots: Vec<Slot> = Vec::new();
        for it in &mut run.items {
            let follow = it.follow();
            let due = match it.control.timing {
                // a plain value that names an input follows it: due on every bar of its place from its first (R1)
                _ if follow.is_some_and(|f| l >= it.start_line && rel >= i64::from(f.from) && f.until.is_none_or(|u| rel < i64::from(u))) => true,
                _ if follow.is_some() => false,
                Timing::Now => !it.fired && l >= it.start_line,
                Timing::Every(n) => rel > 0 && l >= it.start_line && rel % i64::from(n) == 0,
                Timing::EveryFrom(n, f) => rel >= i64::from(f) && l >= it.start_line && (rel - i64::from(f)) % i64::from(n) == 0,
                Timing::At(b) => !it.fired && rel == i64::from(b) && l >= it.start_line,
                Timing::NextMult(n) => !it.fired && l >= it.start_line && rel >= 0 && rel % i64::from(n) == 0,
                // a time that is not a bar line is fired when the position crosses it: `fire_between`
                Timing::AtPos(_) | Timing::EveryPos { .. } => false,
            };
            // a track's line in a set runs only while the track does
            let due = due && it.control.window.is_none_or(|(a, b)| rel >= i64::from(a) && rel < i64::from(b));
            if due {
                self.fire(host, it, l, l as f64, off);
                fired_slots.extend(effect_slots(&it.control.effect));
            }
            // a ramp that names an input reads it again on every bar line and keeps the value inside the bar
            if let Some(r) = it.ramp.as_mut().filter(|r| !r.done) {
                if let Some(p) = r.live.clone() {
                    if let Effect::Ramp(nr) = self.perf.reread(host, &p, l) {
                        r.ramp = nr;
                    }
                    self.perf_words();
                }
            }
            // so does an lfo whose centre or reach names one: the wave goes on, from the new middle and with the new reach
            if let Some(lr) = it.lfo.as_mut().filter(|lr| !lr.is_done()) {
                if let Some(p) = lr.live().cloned() {
                    if let Effect::Lfo(nl) = self.perf.reread(host, &p, l) {
                        lr.retune(&nl);
                    }
                    self.perf_words();
                }
            }
        }
        // A line that fired on this bar line is the last word on its target: a ramp started before it on the same target (the
        // riser's `filter hp 100 -> 5000 over 8 bars`, which reaches its end on the very line the next section's `filter off` is
        // on) is ended here, without its last value. Before this, move_ramps (later in the same pass) sent the ramp's t = 1 value
        // after the section's own lines, and a page that dedupes repeated values never sent `filter off` again: the whole next
        // section played high-passed and wet (happy-wub-172's rave, rave-duel-138's duel).
        if !fired_slots.is_empty() {
            for it in &mut run.items {
                let Some(r) = it.ramp.as_mut().filter(|r| !r.done && r.start + EPS < l as f64) else { continue };
                if slot_of(&r.ramp.at(0.0)).is_some_and(|s| fired_slots.contains(&s)) {
                    r.done = true;
                }
            }
        }
        self.running = Some(run);
    }

    /// Run an item: `l` is the program bar it falls in and `at` the program position (in bars) it was due at, which
    /// is `l` for a bar line and the exact time for one between bar lines. `off` is the program bar the song's first
    /// bar is on (0 for a program with no timeline).
    fn fire<H: LiveHost>(&mut self, host: &mut H, it: &mut Item, l: i64, at: f64, off: i64) {
        let attempted = std::mem::replace(&mut it.fired, true);
        // a performed line (an input in a value, a `when`) is decided with the values read at this bar (perform.rs)
        let mut performed: Option<Effect> = None;
        let follows = it.follow().is_some();
        let first_send = follows && it.sent.is_none();
        if let Effect::Performed(p) = &it.control.effect {
            let r = self.perf.resolve(host, p, l, crate::perform::slot_of(at, l));
            self.perf_words();
            match r {
                Some(e) if follows => {
                    // a line that follows its input sends when the value it reads has moved: not again for the same one
                    if it.sent.as_ref() == Some(&e) {
                        return;
                    }
                    it.sent = Some(e.clone());
                    performed = Some(e);
                }
                Some(e) => performed = Some(e),
                None => {
                    // (a line that follows its input says it once, when its `when` stops it, not on every bar)
                    if !follows || !attempted || it.sent.take().is_some() {
                        self.word(format!("bar {l}: not run, its `when` is false: {}", it.control.key));
                    }
                    return;
                }
            }
        }
        let effect = performed.as_ref().unwrap_or(&it.control.effect);
        if let Some(h) = it.control.hold {
            it.undo_at = Some(at + h.to_f64());
        } else if let Some(len) = it.control.restore {
            // the first phrase line after the move has ended (counted from the song's first bar: `off`, the caller's, not a read of
            // `self.running`, which the caller has taken out while it runs the lines: that was always 0, a bar early for a song)
            let end = l - off + i64::from(len);
            it.undo_at = Some((off + (end.div_euclid(i64::from(PHRASE)) + 1) * i64::from(PHRASE)) as f64);
        } else if matches!(effect, Effect::Duck(_)) {
            // a duck with no end of its own holds until the program ends or stops
            it.undo_at = Some(f64::INFINITY);
        }
        match effect {
            Effect::Calls(cs) => {
                for c in cs.clone() {
                    match host.call(&c.noun, &c.params) {
                        Ok(answer) => {
                            // a `sing` line puts its voice on the loop row, where it loops until it is stopped: keep which clip
                            if c.noun == "vocal" && c.params.get("action").and_then(JsonValue::as_str) == Some("to_row") {
                                if let Some(id) = answer.get("id").and_then(JsonValue::as_str) {
                                    it.clip = Some(id.to_string());
                                }
                            }
                        }
                        Err(e) => self.word(format!("{}: {e}", c.noun)),
                    }
                }
            }
            Effect::Ramp(r) => {
                for c in r.prelude.clone() {
                    self.call(host, &c.noun, &c.params);
                }
                let live = match &it.control.effect {
                    Effect::Performed(p) if !p.patches.is_empty() => Some(p.clone()),
                    _ => None,
                };
                it.ramp = Some(RampRun { ramp: r.clone(), start: at, last: None, done: false, live });
            }
            Effect::Duck(d) => {
                // the pump's release is a note division: chosen at the program's tempo, else the deck's
                let bpm = d.bpm.or_else(|| host.grid().map(|g| g.bpm)).unwrap_or(120.0);
                let c = d.on_call(bpm);
                self.call(host, &c.noun, &c.params);
            }
            Effect::Mix(m) => {
                // a snapshot reads the mixer before it changes it (once: a second firing while it is held keeps the first)
                let first = m.restore && it.snapshot.is_empty();
                let read = if first { host.call("mixer", &JsonValue::Object(JsonObject::new())).ok() } else { None };
                let f = m.fire(read.as_ref());
                if first {
                    it.snapshot = f.back;
                    for w in f.words {
                        self.word(w);
                    }
                }
                for c in &f.sets {
                    self.call(host, &c.noun, &c.params);
                }
            }
            Effect::Lfo(l) => {
                for c in l.prelude.clone() {
                    self.call(host, &c.noun, &c.params);
                }
                // a song counts its bars from where it landed; the lfo stops where its section ends (or its hold)
                let live = match &it.control.effect {
                    Effect::Performed(p) if p.lfo.is_some() => Some(p.clone()),
                    _ => None,
                };
                it.lfo = Some(LfoRun::new(l, at, off as f64, it.control.hold).with_live(live));
            }
            // (resolved above: `effect` is what the performed line came to, never the line itself)
            Effect::Performed(_) => {}
        }
        let seen = self.line as f64 + self.frac;
        self.cfires.push(ControlFire { span: it.control.span, via: it.control.via.clone(), key: it.control.key.clone(), at: seen });
        let when = if at == l as f64 { format!("bar {l}") } else { format!("bar {at:.3}") };
        // (a line that follows its input says so when it first sends, and not at each turn of the knob)
        if !follows || first_send {
            self.word(format!("{when}: {}", it.control.key));
        }
        // a note the line carries (a sing line's neural fallback to the formant voice) is said where the line fires
        if let Some(n) = &it.control.notice {
            self.word(format!("{when}: {n}"));
        }
    }

    /// The lines whose time is not a bar line (`at bar 2 beat 3`, `every 2 beats`, `before drop 2 beats`) and the
    /// holds that end between bar lines: each fires when the program's position (the deck's beat clock, bars and
    /// the fraction of one) crosses its time, once, in the order of the times. `prev` is the position at the last
    /// pass; with no clock, or a clock that did not move forward, nothing was crossed.
    fn fire_between<H: LiveHost>(&mut self, host: &mut H, prev: Option<f64>, now: Option<f64>) {
        let (Some(prev), Some(now)) = (prev, now) else { return };
        if now <= prev {
            return;
        }
        let Some(mut run) = self.running.take() else { return };
        let off = run.clock0() as f64;
        let mut due: Vec<(f64, usize)> = Vec::new();
        for (i, it) in run.items.iter().enumerate() {
            match it.control.timing {
                Timing::AtPos(p) if !it.fired => {
                    // (a track's line in a set is only due while the track plays: judged on the line's own time, a macro's call
                    // being a beat ahead of it)
                    let lead = it.control.lead.map_or(0.0, |l| l.to_f64());
                    if it.control.window.is_some_and(|(a, b)| p.to_f64() + lead + EPS < f64::from(a) || p.to_f64() + lead + EPS >= f64::from(b)) {
                        continue;
                    }
                    let t = off + p.to_f64();
                    // (a time in the bar the line was put in, `at bar 1/2` of a program evaluated in bar 0, is still to come when it
                    // lands: its first bar is `start_line`, which the program lands on the bar line after the one it was applied in)
                    if t + EPS >= it.start_line as f64 - 1.0 && prev + EPS < t && t <= now + EPS {
                        due.push((t, i));
                    }
                }
                Timing::EveryPos { step, from } => {
                    let (s, f) = (step.to_f64(), off + from.to_f64());
                    if s <= 0.0 || now + EPS < f {
                        continue;
                    }
                    // the k-th time is f + k s: those after the last pass (prev), up to now, not before the line began (and, for a
                    // track's line in a set, before the track ends: the first time that is not before it ends is not one)
                    let end = it.control.window.map_or(f64::INFINITY, |(_, b)| off + f64::from(b) - it.control.lead.map_or(0.0, |l| l.to_f64()));
                    let hi = ((now + EPS - f) / s).floor().min(((end - f) / s - EPS).ceil() - 1.0);
                    let after_prev = if prev + EPS >= f { ((prev + EPS - f) / s).floor() + 1.0 } else { 0.0 };
                    let after_start = ((it.start_line as f64 - EPS - f) / s).ceil().max(0.0);
                    let lo = after_prev.max(after_start).max(hi - (MAX_CATCH_UP - 1) as f64);
                    // (counted, not stepped: a float this large would not move by one)
                    let count = ((hi - lo) as i64 + 1).clamp(0, MAX_CATCH_UP);
                    for j in 0..count {
                        due.push((f + (lo + j as f64) * s, i));
                    }
                }
                _ => {}
            }
        }
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let put_back = |me: &mut Self, host: &mut H, items: &mut [Item]| {
            for it in items.iter_mut() {
                if it.undo_at.is_some_and(|u| now + EPS >= u) {
                    it.undo_at = None;
                    for c in it.undo_calls() {
                        me.call(host, &c.noun, &c.params);
                    }
                    it.snapshot.clear();
                    me.word(format!("bar {now:.3}: put back {}", it.control.key));
                }
            }
        };
        put_back(self, host, &mut run.items);
        for (t, i) in due {
            self.fire(host, &mut run.items[i], t.floor() as i64, t, off as i64);
        }
        put_back(self, host, &mut run.items);
        self.running = Some(run);
    }

    fn move_ramps<H: LiveHost>(&mut self, host: &mut H) {
        let Some(mut run) = self.running.take() else { return };
        let pos = self.line as f64 + self.frac;
        let mut sends: Vec<(String, JsonValue)> = Vec::new();
        for it in &mut run.items {
            if let Some(l) = &mut it.lfo {
                if let Some(c) = l.step(pos) {
                    sends.push((c.noun, c.params));
                }
                if l.is_done() {
                    it.lfo = None;
                }
            }
            let Some(r) = &mut it.ramp else { continue };
            if r.done {
                continue;
            }
            let t = ((pos - r.start) / r.ramp.bars.to_f64()).clamp(0.0, 1.0);
            let v = r.ramp.value(t);
            if t >= 1.0 || r.last.is_none_or(|l| (l - v).abs() >= RAMP_EPS) {
                let c = r.ramp.at(t);
                sends.push((c.noun, c.params));
                r.last = Some(v);
            }
            if t >= 1.0 {
                r.done = true;
            }
        }
        self.running = Some(run);
        for (n, p) in sends {
            self.call(host, &n, &p);
        }
    }

    /// The program's tempo curve (F3, tempo.rs): each pass the tempo the map has at the program's position goes to every
    /// deck the program has a net on, when it moved. A program whose tempo is one number (`tempo 126`) is not driven:
    /// it is in its nets, as it always was. A host that cannot set a tempo is told once in `words` and not asked again.
    fn follow_tempo<H: LiveHost>(&mut self, host: &mut H) {
        let Some(run) = &self.running else { return };
        let map = run.compiled.tempo_map();
        if !self.anchored || !map.drives() {
            return;
        }
        let off = run.tempo0();
        // (the position in the program's own bars, from its first bar: before it lands, its first tempo)
        let rel = (self.line as f64 + self.frac - off as f64).max(0.0);
        let Some(bpm) = map.declared_at_f(rel) else { return };
        // (to six decimals: the position the runtime counts has a nanobar of slack, which must not reach the deck as `100.00000001`)
        let bpm = (bpm * 1e6).round() / 1e6;
        let decks: Vec<Deck> = run.loaded.keys().copied().collect();
        for d in decks {
            if self.tempo_failed.contains(&d) || self.tempo_sent.get(&d).is_some_and(|b| (b - bpm).abs() < crate::tempo::SEND_EPS) {
                continue;
            }
            match host.set_tempo(d.name(), bpm) {
                Ok(()) => {
                    self.tempo_sent.insert(d, bpm);
                    if !self.tempo_set.contains(&d) {
                        self.tempo_set.push(d);
                    }
                }
                Err(e) => {
                    self.tempo_failed.push(d);
                    self.word(format!("tempo curve: deck {}: {e}; this host cannot set the tempo, so the deck stays at its own (the bars and every timing word are still exact)", d.name()));
                }
            }
        }
    }

    fn apply<H: LiveHost>(&mut self, host: &mut H) {
        let Some(st) = self.staged.take() else { return };
        // a program is asked afresh to set the tempo, and what the last one sent is forgotten
        self.tempo_failed.clear();
        self.tempo_sent.clear();
        let fresh = self.running.is_none();
        let clock = self.anchored;
        if fresh {
            // a program's first bar is bar 0
            self.line = 0;
        }
        let land = if clock { self.line + 1 } else { self.line };
        let mut run = self.running.take().unwrap_or(Running {
            compiled: st.compiled.clone(),
            loaded: BTreeMap::new(),
            items: Vec::new(),
            origin: land,
            first: land,
            nets: BTreeMap::new(),
            keys: Vec::new(),
            pending: Vec::new(),
            stops: Vec::new(),
            restarted: None,
            landing: None,
            scene: None,
            base: None,
        });
        // a scene that plays goes on in the program as it is now (an edit is of the song as written, and lands on the scene), or the
        // song starts again when the scene is not in it any more; a scene to start in (a pad hit after the end) is the same
        let written = st.compiled;
        let want = st.scene.clone().or_else(|| run.scene.clone());
        if let Some(ln) = run.landing.take() {
            // (nets loaded for a scene that was to land are not the new program's: the scene is asked for again)
            if let Some(l) = written.launches.table.iter().find(|l| l.scene == ln.scene) {
                self.queued = Some(Queued { scene: ln.scene, quantum: l.quantum });
            }
        }
        if self.queued.as_ref().is_some_and(|q| written.launches.scene(&q.scene).is_none()) {
            self.queued = None;
            self.word("the scene a pad asked for is not in the program any more");
        }
        let (new, nets) = match want.as_deref().and_then(|n| written.launches.scene(n)) {
            Some(sc) => {
                let sc = sc.clone();
                let nets = sc.net_cids();
                run.base = Some(written);
                run.scene = want.clone();
                (sc, nets)
            }
            None => {
                if let Some(gone) = want {
                    self.word(format!("the scene `{gone}` is not in the program any more: the song starts again"));
                    run.origin = land;
                    run.restarted = None;
                }
                run.base = None;
                run.scene = None;
                (written, st.nets)
            }
        };
        run.nets = nets;
        // a program whose tempo moves takes the decks' tempo each pass: what it was is read now, before the nets land (R-04)
        if new.tempo_map().drives() {
            for (_, d, _) in nets_of(&new) {
                if !self.tempo_before.contains_key(&d) {
                    if let Some(t) = host.tempo_of(d.name()) {
                        self.tempo_before.insert(d, t);
                    }
                }
            }
        }
        run.keys = keys_of(&new);
        run.compiled = new.clone();
        self.perf.program_applied(&new.cid, fresh);
        self.perf_words();
        self.sync_nets(host, &mut run, land);
        // ---- the control lines: what runs keeps its state
        let mut items = Vec::new();
        let mut old = std::mem::take(&mut run.items);
        for c in &new.controls {
            match old.iter().position(|i| i.control.key == c.key) {
                Some(p) => {
                    let mut it = old.remove(p);
                    let old_hold = it.control.hold;
                    it.control = c.clone();
                    // the key (the section's start and the line's text) did not change, but the section's length may have: what its end
                    // gave the line (how long a section's `mix` or `duck` is held, where an lfo stops) is worked out again, for the
                    // run that is going on (R-05: they kept the end they had when the line fired)
                    if let (Some(u), Some(oh), Some(nh)) = (it.undo_at, old_hold, c.hold) {
                        if u.is_finite() && oh != nh {
                            it.undo_at = Some(u - oh.to_f64() + nh.to_f64());
                        }
                    }
                    if let (Some(lr), Some(until)) = (it.lfo.as_mut(), lfo_until(&c.effect)) {
                        lr.restop(until, run.clock0() as f64, c.hold);
                    }
                    items.push(it);
                }
                None => items.push(Item { control: c.clone(), start_line: land, fired: false, ramp: None, lfo: None, undo_at: None, snapshot: Vec::new(), clip: None, sent: None }),
            }
        }
        for gone in old {
            // what it had changed is put back before it is forgotten
            if gone.undo_at.is_some() {
                for c in gone.undo_calls() {
                    self.call(host, &c.noun, &c.params);
                }
            }
            // (and a sung line's voice stops looping: an edited line is another line, which sings its own)
            if let Some(c) = gone.clip_stop() {
                self.call(host, &c.noun, &c.params);
            }
            self.word(format!("stopped: {}", gone.control.key));
        }
        run.items = items;
        let lines = st.plan.lines();
        self.word(if lines.is_empty() { format!("applied {}", short(&run.compiled.cid)) } else { format!("applied {}: {}", short(&run.compiled.cid), lines.join("; ")) });
        self.running = Some(run);
        if !clock {
            // nothing plays: what is due on this bar is due now
            let l = self.line;
            self.fire_bar(host, l);
        }
    }

    /// Make what is loaded the nets of the bar `target` (the bar about to start): a net whose key
    /// differs is loaded (it lands on that bar line), a deck the song does not use there is paused,
    /// a role's own sound is set once its net has landed.
    fn sync_nets<H: LiveHost>(&mut self, host: &mut H, run: &mut Running, target: i64) {
        // a scene that lands on that bar line has its nets loaded (`decide_landing`); the song's are not
        if run.landing.as_ref().is_some_and(|ln| target >= ln.at) {
            return;
        }
        let arranged = run.compiled.arranged;
        let mut rel = target - if arranged { run.origin } else { 0 };
        // a song that loops: the bar after its last is its first again, so its first net is loaded in the bar before it begins and
        // lands on it (not a bar late, once the lap has turned)
        if let (true, true, Some(total)) = (run.compiled.looped, arranged, run.compiled.song_bars()) {
            if total > 0 && rel >= i64::from(total) {
                rel -= i64::from(total);
            }
        }
        // nothing is loaded for a bar the program has ended before
        if run.compiled.end.is_some_and(|e| rel >= i64::from(e)) {
            return;
        }
        let (idx, decks, start, chain): (usize, &BTreeMap<Deck, DeckIr>, i64, bool) = if arranged {
            if rel < 0 {
                return;
            }
            let Some(i) = run.compiled.segments.iter().rposition(|s| i64::from(s.start) <= rel) else { return };
            let seg = &run.compiled.segments[i];
            (i, &seg.decks, i64::from(seg.start), seg.chain)
        } else {
            (0, &run.compiled.decks, 0, false)
        };
        let keys = run.keys.get(idx).cloned().unwrap_or_default();
        // the section's first bar: what it loads plays from its own start, even when it is the net that plays now (the section
        // again, by `loop` or a second mention in `arrange`: else that net runs on, and a pattern whose cycle does not divide the
        // section is heard mid-cycle where the section begins). Once for that bar line: this runs on every pass of the bar before.
        // Not a `chain` stretch (the later stretch of a stack, or of a section too long for one net): that is the same section
        // going on, and a net it shares with the stretch before plays on.
        let restart = arranged && rel == start && !chain && run.restarted != Some(target);
        if restart {
            run.restarted = Some(target);
        }
        for (deck, ir) in decks {
            let key = keys.get(deck).cloned().unwrap_or_default();
            if run.loaded.get(deck) == Some(&key) && !restart {
                continue;
            }
            let phase = if ir.period > 1 { ((if arranged { rel - start } else { target.max(0) }) as usize) % ir.period } else { 0 };
            let env = ir.envelope(phase);
            match host.load_net(deck.name(), &env) {
                Ok(()) => {
                    run.loaded.insert(*deck, key);
                    run.stops.retain(|s| s.0 != *deck);
                    if !host.playing(deck.name()) {
                        host.set_playing(deck.name(), true);
                    }
                    let sounds = ir.sounds();
                    run.pending.retain(|p| p.deck != *deck);
                    if !sounds.is_empty() {
                        run.pending.push(PendingSound { deck: *deck, at: target, sounds });
                    }
                }
                Err(e) => self.word(format!("deck {}: {e}", deck.name())),
            }
        }
        let gone: Vec<Deck> = run.loaded.keys().filter(|d| !decks.contains_key(d)).copied().collect();
        for d in gone {
            // this is the bar before the one the deck has no net for: it plays this one out, and stops on the bar line
            if self.anchored && target > self.line {
                run.stops.retain(|s| s.0 != d);
                run.stops.push((d, target));
            } else {
                host.set_playing(d.name(), false);
            }
            run.loaded.remove(&d);
            run.pending.retain(|p| p.deck != d);
        }
    }

    // ---- scenes and launch (launch.rs)

    /// Watch the pads of the program's `launch` lines: a pad that was up and is down is a hit, and queues its scene. A host with no
    /// controller (the page, the phone) answers nothing, so the song plays as written. A replay takes its launches from the log instead.
    fn poll_pads<H: LiveHost>(&mut self, host: &mut H) {
        if self.perf.is_replaying() {
            // (after the end there is no bar line to land on: the log's next launch starts at once)
            if self.running.is_none() && self.staged.is_none() {
                if let Some(scene) = self.perf.replay_launch_after_end() {
                    self.start_scene_after_end(&scene);
                }
            }
            return;
        }
        let table: Vec<(u32, String, u32)> = match (&self.running, &self.finished) {
            (Some(r), _) => r.written().launches.table.iter().map(|l| (l.pad, l.scene.clone(), l.quantum)).collect(),
            (None, Some(f)) if self.staged.is_none() => f.launches.table.iter().map(|l| (l.pad, l.scene.clone(), l.quantum)).collect(),
            _ => return,
        };
        for (pad, scene, quantum) in table {
            let down = host.input("pad", &pad.to_string()).is_some_and(|v| v > 0.0);
            // (a pad seen for the first time is not a hit, whatever it is doing: one held down while the program began is not a launch)
            let was = self.pad_down.insert(pad, down).unwrap_or(down);
            if down && !was {
                self.hit(scene, quantum);
            }
        }
    }

    /// The first bar line a scene asked for now could land on: the engine has the first three quarters of the bar before to load it, so
    /// a hit in the last quarter waits for the next quantum line. None when nothing counts bars.
    fn lands_for(&self, quantum: u32) -> Option<i64> {
        let run = self.running.as_ref()?;
        if !self.anchored {
            return None;
        }
        let earliest = self.line + if self.frac <= APPLY_BY { 1 } else { 2 };
        Some(earliest + (run.first - earliest).rem_euclid(i64::from(quantum.max(1))))
    }

    /// A pad of a launch was hit: its scene is queued for its bar line, in place of one queued before.
    fn hit(&mut self, scene: String, quantum: u32) {
        if self.running.is_none() {
            self.start_scene_after_end(&scene);
            return;
        }
        match self.lands_for(quantum) {
            Some(at) => self.word(format!("bar {}: {scene} queued, lands bar {at}", self.line)),
            None => self.word(format!("{scene} queued: it starts at once, nothing plays")),
        }
        self.queued = Some(Queued { scene, quantum });
    }

    /// A hit after the song ended: the scene starts as the program, at once (there is no bar line to wait for).
    fn start_scene_after_end(&mut self, scene: &str) {
        let Some(f) = self.finished.take() else { return };
        let Some(sc) = f.launches.scene(scene) else {
            self.finished = Some(f);
            return;
        };
        let (plan, nets) = (diff(None, sc), sc.net_cids());
        self.ended = None;
        self.perf.log_launch(self.line, scene);
        self.word(format!("{scene}: the song had ended, the scene starts"));
        self.staged = Some(Staged { compiled: f, plan, nets, scene: Some(scene.to_string()) });
    }

    /// Decide the scene that lands on the bar line `target`, if one is queued for it (or the replay's log has it): its first nets are
    /// loaded now, in the first part of the bar before, as the song's are, and the song plays on until the bar line (`enter_scene`).
    fn decide_landing<H: LiveHost>(&mut self, host: &mut H, run: &mut Running, target: i64) {
        if run.landing.is_some() {
            return;
        }
        let scene = if self.perf.is_replaying() {
            match self.perf.replay_launch(target) {
                Some(s) => s,
                None => return,
            }
        } else {
            let Some(q) = &self.queued else { return };
            // (with nothing counting bars a scene starts at once)
            if self.anchored && (target - run.first).rem_euclid(i64::from(q.quantum.max(1))) != 0 {
                return;
            }
            q.scene.clone()
        };
        self.queued = None;
        let Some(program) = run.written().launches.scene(&scene).cloned() else {
            self.word(format!("{scene}: no such scene in the program"));
            return;
        };
        // the scene's first nets, loaded as if it were the song: its compile, its bar 0 on that bar line
        let mut ln = Landing { scene: scene.clone(), at: target, program: Box::new(program) };
        std::mem::swap(&mut run.compiled, &mut *ln.program);
        let origin = std::mem::replace(&mut run.origin, target);
        let keys = std::mem::replace(&mut run.keys, keys_of(&run.compiled));
        self.sync_nets(host, run, target);
        std::mem::swap(&mut run.compiled, &mut *ln.program);
        run.origin = origin;
        run.keys = keys;
        self.perf.log_launch(target, &scene);
        self.word(format!("{scene}: lands on bar {target}"));
        run.landing = Some(ln);
        if !self.anchored {
            self.enter_scene(host, run, target);
        }
    }

    /// The bar line a scene lands on: what the song's lines hold changed is put back, its ramps and lfos end, and the scene is the
    /// program from its bar 0 (its lines count from here, as in a lap of a looped song; its tempo and key are its own). The song as
    /// written is kept (`base`): the launches, an edit and the diff are of it.
    fn enter_scene<H: LiveHost>(&mut self, host: &mut H, run: &mut Running, l: i64) {
        let Some(ln) = run.landing.take() else { return };
        for it in &run.items {
            if it.undo_at.is_some() {
                for c in it.undo_calls() {
                    self.call(host, &c.noun, &c.params);
                }
            }
            if let Some(c) = it.clip_stop() {
                self.call(host, &c.noun, &c.params);
            }
        }
        let old = std::mem::replace(&mut run.compiled, *ln.program);
        if run.base.is_none() {
            run.base = Some(old);
        }
        run.scene = Some(ln.scene.clone());
        run.origin = l;
        run.keys = keys_of(&run.compiled);
        run.nets = run.compiled.net_cids();
        run.items = run
            .compiled
            .controls
            .iter()
            .map(|c| Item { control: c.clone(), start_line: l, fired: false, ramp: None, lfo: None, undo_at: None, snapshot: Vec::new(), clip: None, sent: None })
            .collect();
        // a scene whose tempo moves takes the decks' tempo from its own curve (its bar 0 is here)
        self.tempo_sent.clear();
        if run.compiled.tempo_map().drives() {
            for (_, d, _) in nets_of(&run.compiled) {
                if !self.tempo_before.contains_key(&d) {
                    if let Some(t) = host.tempo_of(d.name()) {
                        self.tempo_before.insert(d, t);
                    }
                }
            }
        }
        self.word(format!("bar {l}: the scene {} begins", ln.scene));
    }

    // ---- the performed inputs (F13, perform.rs)

    /// What the inputs said (a controller that is not there reads 0) goes in the words `btw get` shows.
    fn perf_words(&mut self) {
        for n in std::mem::take(&mut self.perf.notes) {
            self.word(n);
        }
    }

    /// The inputs the running program reads (`knob(3)`, `fader(A)` ...), each once.
    pub fn performed_inputs(&self) -> Vec<crate::perform::Input> {
        self.running.as_ref().map(|r| crate::perform::performed_inputs(&r.compiled.controls)).unwrap_or_default()
    }

    /// The performance of the running program so far: every value read, by program bar (the last take when the program has read
    /// nothing yet). None before anything was read.
    pub fn performance(&self) -> Option<crate::perform::Performance> {
        self.perf.performance()
    }

    /// Begin a new take now (what was logged is kept as the last take) and log again.
    pub fn perform_start(&mut self) {
        self.perf.start();
        self.word("performance: a new take began".to_string());
    }

    /// Stop logging: what was read is kept, to be saved. The values are still read.
    pub fn perform_stop(&mut self) {
        self.perf.stop_recording();
        self.word("performance: logging stopped".to_string());
    }

    /// Replay `perf`: the program's inputs read the performance's values, by bar, instead of the controllers. Refused when it is
    /// a performance of another program than the one that is running (or staged, and about to run): a log is of one program.
    /// Start the program fresh first (`stop`, then `eval`): the log is by program bar, and a program counts from its first.
    pub fn replay(&mut self, perf: crate::perform::Performance) -> Result<(), String> {
        let have = self.staged.as_ref().map(|s| s.compiled.cid.clone()).or_else(|| self.running.as_ref().map(|r| r.compiled.cid.clone()));
        match have {
            Some(cid) if cid == perf.program => {}
            Some(cid) => return Err(format!("refused: this performance is of program {}, and the program that plays is {cid}", perf.program)),
            None => return Err(format!("refused: no program is running: this performance is of program {} (evaluate it first)", perf.program)),
        }
        self.word(format!("replaying performance {} ({} values): the controllers are not read", short(&perf.cid()), perf.inputs.len()));
        self.perf.replay(perf);
        Ok(())
    }

    /// Whether the inputs are being read from a performance.
    pub fn replaying(&self) -> bool {
        self.perf.is_replaying()
    }

    /// The performance state, for the `perform` verb: the inputs, what is logged (with the take's CID), what is replayed.
    /// `status` has the same without the CID, which hashes the whole take.
    pub fn perform_status(&self) -> JsonValue {
        let mut o = self.perf.status(&self.performed_inputs(), true);
        // the pads that launch a scene, the scene that plays and the one that is queued (launch.rs)
        if let (JsonValue::Object(ob), Some(r)) = (&mut o, &self.running) {
            let w = r.written();
            if !w.launches.is_empty() {
                let mut l = JsonObject::new();
                l.set("pads", w.launches.to_json());
                l.set("scene", r.scene.clone().map_or(JsonValue::Null, JsonValue::str));
                if let Some(q) = &self.queued {
                    l.set("queued", JsonValue::Object(JsonObject::from_pairs([("scene", JsonValue::str(q.scene.clone())), ("lands", self.lands_for(q.quantum).map_or(JsonValue::Null, JsonValue::int))])));
                }
                ob.set("launch", JsonValue::Object(l));
            }
        }
        o
    }

    /// The section being played (name, bar in it, its length), for an arranged song.
    pub fn section(&self) -> Option<(String, i64, u32)> {
        let r = self.running.as_ref()?;
        r.compiled.section_at(self.line - r.origin).map(|(n, b, l)| (n.to_string(), b, l))
    }

    /// The arranged song's length in bars.
    pub fn song_bars(&self) -> Option<u32> {
        self.running.as_ref()?.compiled.song_bars()
    }

    /// The program bar the song began on.
    pub fn origin(&self) -> i64 {
        self.running.as_ref().map_or(0, |r| r.origin)
    }

    /// A deck a running program has put a net on.
    pub fn plays_on(&self, deck: &str) -> bool {
        self.running.as_ref().is_some_and(|r| r.loaded.keys().any(|d| d.name() == deck))
    }

    /// What to light in the text right now (docs/BEATSCRIPT.md, "Seeing it play"): the lines that are playing, the
    /// playhead on the section header, the tokens that just sounded, the control lines that just fired or are
    /// moving, and the generation. Spans are offsets into `source`, the text the program was evaluated from.
    pub fn marks(&self) -> Marks {
        let mut m = Marks::default();
        let Some(run) = &self.running else {
            m.ended = self.ended.is_some();
            return m;
        };
        let c = &run.compiled;
        m.running = true;
        m.source = c.source.clone();
        m.cid = c.cid.clone();
        let pos = self.line as f64 + self.frac;
        m.pos = pos;
        if c.arranged {
            if let Some(seg) = c.segments.iter().rev().find(|s| i64::from(s.start) <= self.line - run.origin) {
                m.active_lines = seg.lines.clone();
                if let (Some(h), Some((_, bar, len))) = (seg.header, c.section_at(self.line - run.origin)) {
                    m.header = Some((h, ((bar as f64 + self.frac) / f64::from(len.max(1))).clamp(0.0, 1.0)));
                }
            }
        } else {
            m.active_lines = c.flat_lines.clone();
        }
        for f in &self.fires {
            m.fires.push(FireMark { deck: f.deck, role: f.role.clone(), spans: f.spans.clone(), calls: f.calls, age_beats: (pos - f.at) * BEATS_PER_BAR });
        }
        for f in &self.cfires {
            m.controls.push(ControlMark { span: f.span, via: f.via.clone(), key: f.key.clone(), age_beats: (pos - f.at) * BEATS_PER_BAR, ramp: None, held: false });
        }
        for it in &run.items {
            let ramp = it.ramp.as_ref().filter(|r| !r.done).map(|r| ((pos - r.start) / r.ramp.bars.to_f64()).clamp(0.0, 1.0)).or_else(|| it.lfo.as_ref().filter(|l| !l.is_done()).map(|l| l.progress(pos)));
            let held = it.undo_at.is_some();
            if ramp.is_some() || held {
                match m.controls.iter_mut().find(|x| x.key == it.control.key) {
                    Some(x) => {
                        x.ramp = ramp;
                        x.held = held;
                    }
                    None => m.controls.push(ControlMark { span: it.control.span, via: it.control.via.clone(), key: it.control.key.clone(), age_beats: f64::MAX, ramp, held }),
                }
            }
        }
        m.generation = c.generation_at(self.line - run.origin);
        if let Some((g, at)) = self.gen_flash {
            m.gen_flash_age = Some((pos - at) * BEATS_PER_BAR).filter(|a| *a < 8.0 && m.generation == Some(g));
        }
        if let Some(g) = m.generation {
            for ev in &c.evolves {
                m.evolve_spans.push(ev.block.span);
                for r in &ev.block.rules {
                    if r.when.as_ref().is_none_or(|w| w.holds(g)) {
                        m.evolve_spans.push(r.span);
                    }
                }
            }
        }
        m
    }

    /// The state, for the `live get` verb and the window.
    pub fn status(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("running", self.running.is_some().into());
        o.set("bar", JsonValue::int(self.line));
        o.set("bar_in_phrase", JsonValue::int(self.line.rem_euclid(8) + 1));
        o.set("phrase", JsonValue::int(self.line.div_euclid(8) + 1));
        o.set("beat_in_bar", JsonValue::Number((self.frac * BEATS_PER_BAR * 100.0).round() / 100.0));
        o.set("clock", self.anchored.into());
        o.set("ended", self.ended.is_some().into());
        if let Some(e) = &self.ended {
            o.set("ended_at_bar", JsonValue::int(e.bar));
            o.set("ended_cid", JsonValue::str(e.cid.clone()));
        }
        if let Some(r) = &self.running {
            o.set("ends_at_bar", r.compiled.end.map_or(JsonValue::Null, |e| JsonValue::int(i64::from(e) + r.clock0())));
            o.set("loops", r.compiled.looped.into());
            o.set("arranged", r.compiled.arranged.into());
            if let Some((name, bar, len)) = r.compiled.section_at(self.line - r.origin) {
                o.set("section", JsonValue::str(name));
                o.set("section_bar", JsonValue::int(bar + 1));
                o.set("section_bars", JsonValue::int(i64::from(len)));
            }
            if let Some(g) = r.compiled.generation_at(self.line - r.origin) {
                o.set("generation", JsonValue::int(i64::from(g)));
            }
            let map = r.compiled.tempo_map();
            if map.drives() {
                let off = r.tempo0();
                if let Some(b) = map.declared_at_f((self.line as f64 + self.frac - off as f64).max(0.0)) {
                    o.set("tempo", JsonValue::Number((b * 100.0).round() / 100.0));
                }
                o.set("tempo_curve", true.into());
            }
            if let Some(total) = r.compiled.song_bars() {
                o.set("song_bars", JsonValue::int(i64::from(total)));
                o.set("song_bar", JsonValue::int(self.line - r.origin + 1));
            }
        }
        if let Some(r) = &self.running {
            o.set("cid", JsonValue::str(r.compiled.cid.clone()));
            o.set("text", JsonValue::str(r.compiled.text.clone()));
            o.set("nets", JsonValue::Object(JsonObject::from_pairs(r.nets.iter().map(|(k, v)| (k.clone(), JsonValue::str(v.clone()))).collect::<Vec<_>>())));
            let controls = r
                .items
                .iter()
                .map(|i| {
                    let state = match (&i.ramp, i.fired) {
                        (Some(rr), _) if !rr.done => "ramping",
                        _ if i.lfo.is_some() => "ramping",
                        (_, true) => "ran",
                        _ => "waiting",
                    };
                    JsonValue::Object(JsonObject::from_pairs([("line", JsonValue::str(i.control.key.clone())), ("state", JsonValue::str(state))]))
                })
                .collect();
            o.set("controls", JsonValue::Array(controls));
        }
        if let Some(r) = &self.running {
            if !r.written().launches.is_empty() {
                o.set("launches", r.written().launches.to_json());
                if let Some(sc) = &r.scene {
                    o.set("scene", JsonValue::str(sc.clone()));
                }
            }
        }
        if let Some(q) = &self.queued {
            let lands = self.lands_for(q.quantum).map_or(JsonValue::Null, JsonValue::int);
            o.set("queued", JsonValue::Object(JsonObject::from_pairs([("scene", JsonValue::str(q.scene.clone())), ("lands", lands)])));
        }
        if let Some(s) = &self.staged {
            o.set(
                "staged",
                JsonValue::Object(JsonObject::from_pairs([
                    ("cid", JsonValue::str(s.compiled.cid.clone())),
                    ("lands", self.lands_now().map_or(JsonValue::Null, JsonValue::int)),
                    ("changes", JsonValue::Array(s.plan.lines().into_iter().map(JsonValue::str).collect())),
                ])),
            );
        }
        if !self.performed_inputs().is_empty() || self.perf.has_take() {
            // (the counts: the take's CID is the `perform` verb's, `perform_status`)
            o.set("performed", self.perf.status(&self.performed_inputs(), false));
        }
        let errs = self.errors.iter().filter(|d| d.severity == Severity::Error).count();
        o.set("errors", JsonValue::int(errs as i64));
        o.set("words", JsonValue::Array(self.words.iter().rev().take(12).rev().map(|w| JsonValue::str(w.clone())).collect()));
        JsonValue::Object(o)
    }
}

fn short(cid: &str) -> String {
    cid.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect()
}

/// A hit that sounded, as the window lights it.
#[derive(Clone, Debug, PartialEq)]
pub struct FireMark {
    pub deck: Deck,
    pub role: String,
    /// Every place in the text: the token first, the call sites last.
    pub spans: Vec<Span>,
    /// How many of the last `spans` are call sites.
    pub calls: usize,
    /// How long ago, in beats.
    pub age_beats: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControlMark {
    pub span: Span,
    pub via: Vec<Span>,
    pub key: String,
    /// Beats since it fired (f64::MAX: not recently).
    pub age_beats: f64,
    /// A ramp's progress 0..1.
    pub ramp: Option<f64>,
    /// A `for N bars` line that is holding something changed.
    pub held: bool,
}

/// Everything the window (or any reader) needs to light the text; see [`Live::marks`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    pub running: bool,
    pub ended: bool,
    pub source: String,
    pub cid: String,
    pub pos: f64,
    /// The section header and how far through the section the playhead is.
    pub header: Option<(Span, f64)>,
    pub active_lines: Vec<Span>,
    pub fires: Vec<FireMark>,
    pub controls: Vec<ControlMark>,
    pub generation: Option<u32>,
    /// Beats since the generation began, while it is still flashing.
    pub gen_flash_age: Option<f64>,
    /// The `evolve` block and the rules that applied in this generation.
    pub evolve_spans: Vec<Span>,
}

impl FireMark {
    /// The token that sounded.
    pub fn token(&self) -> Option<Span> {
        self.spans.first().copied()
    }

    /// The call sites (`groove(A, 16)`) that made the line it is on.
    pub fn call_sites(&self) -> &[Span] {
        &self.spans[self.spans.len().saturating_sub(self.calls)..]
    }

    /// The statement it is in, the line in the text it is playing from.
    pub fn line(&self) -> Option<Span> {
        let n = self.spans.len();
        (n > self.calls).then(|| self.spans[n - self.calls - 1])
    }
}

impl Marks {
    /// As the verbs give it: spans with their line and column in `source`.
    pub fn to_json(&self) -> JsonValue {
        let sp = |s: &Span| {
            let (line, col) = crate::diag::line_col(&self.source, s.start);
            JsonValue::Object(JsonObject::from_pairs([
                ("start", JsonValue::int(s.start as i64)),
                ("end", JsonValue::int(s.end as i64)),
                ("line", JsonValue::int(line as i64)),
                ("col", JsonValue::int(col as i64)),
                ("len", JsonValue::int(s.len() as i64)),
            ]))
        };
        let spans = |v: &[Span]| JsonValue::Array(v.iter().map(sp).collect());
        let round = |x: f64| JsonValue::Number((x * 1000.0).round() / 1000.0);
        let mut o = JsonObject::new();
        o.set("running", self.running.into());
        o.set("ended", self.ended.into());
        o.set("cid", JsonValue::str(self.cid.clone()));
        o.set("bar", round(self.pos));
        o.set("header", self.header.as_ref().map_or(JsonValue::Null, |(s, p)| JsonValue::Object(JsonObject::from_pairs([("span", sp(s)), ("progress", round(*p))]))));
        o.set("active_lines", spans(&self.active_lines));
        o.set(
            "fires",
            JsonValue::Array(
                self.fires
                    .iter()
                    .filter(|f| f.age_beats < 2.0)
                    .map(|f| JsonValue::Object(JsonObject::from_pairs([("deck", JsonValue::str(f.deck.name())), ("role", JsonValue::str(f.role.clone())), ("age_beats", round(f.age_beats)), ("spans", spans(&f.spans)), ("call_sites", spans(f.call_sites()))])))
                    .collect(),
            ),
        );
        o.set(
            "controls",
            JsonValue::Array(
                self.controls
                    .iter()
                    .map(|c| {
                        JsonValue::Object(JsonObject::from_pairs([
                            ("line", JsonValue::str(c.key.clone())),
                            ("span", sp(&c.span)),
                            ("via", spans(&c.via)),
                            ("age_beats", if c.age_beats == f64::MAX { JsonValue::Null } else { round(c.age_beats) }),
                            ("progress", c.ramp.map_or(JsonValue::Null, round)),
                            ("held", c.held.into()),
                        ]))
                    })
                    .collect(),
            ),
        );
        o.set("generation", self.generation.map_or(JsonValue::Null, |g| JsonValue::int(i64::from(g))));
        o.set("generation_flash_beats", self.gen_flash_age.map_or(JsonValue::Null, round));
        o.set("evolve_spans", spans(&self.evolve_spans));
        JsonValue::Object(o)
    }
}

/// What a call sets, as far as one line can supersede another: the noun, its target (or deck) and the effect (`inserts` on
/// Master, `filter`: `filter.sweep` and `filter on/off` are one slot). None for a call that names no such thing.
/// What a call sets: (noun, target, effect), for an insert or an FX knob (a mixer call has none).
pub type Slot = (String, String, String);

/// The slot a call sets, if it has one (`inserts` on a target, an effect or a knob of it).
pub fn slot_of(c: &Call) -> Option<Slot> {
    let p = &c.params;
    let target = p.get("target").or_else(|| p.get("deck")).and_then(JsonValue::as_str)?;
    let effect = p
        .get("effect")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .or_else(|| p.get("param").and_then(JsonValue::as_str).map(|s| s.split('.').next().unwrap_or(s).to_string()))?;
    Some((c.noun.clone(), target.to_string(), effect))
}

/// The slots a line's effect sets when it fires (a ramp's: its prelude's and its own).
pub fn effect_slots(e: &Effect) -> Vec<Slot> {
    match e {
        Effect::Calls(cs) => cs.iter().filter_map(slot_of).collect(),
        Effect::Ramp(r) => r.prelude.iter().chain(std::iter::once(&r.at(0.0))).filter_map(slot_of).collect(),
        _ => Vec::new(),
    }
}
