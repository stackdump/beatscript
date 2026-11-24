//! A BeatScript program as an arrangement (`djrust.arrangement/1`), so the offline renderer can play it
//! (`djctl btw render`, `djapp_render`): the nets of each section loaded a bar before the section
//! begins (they land on its bar line, as the live runtime does), a deck a section does not use
//! stopped, and the control lines the arrangement format has an op for (docs/beatscript/ext-render.md).
//!
//! **What is in the render.** The arrangement has ops for a strip's volume, EQ, mute, the master level, the crossfader, a deck's
//! play and stop and the engine's live methods (`trick`), so these control lines are in it, as the runtime would send them:
//!
//! * plain values (`deck A: level 0.6`, `master: level 0.9`, `deck A: low 0.3`, `crossfader 0.2`), `deck A: mute` / `unmute` /
//!   `play` / `stop`, with their `for N bars` / `for 2 beats` put back (a mute is unmuted where the hold ends);
//! * **ramps**, linear and every shape (`deck A: level 0 -> 1 over 8 bars exp`), and **lfos** (`master: level lfo sine 1 bar
//!   center=0.8 depth=0.1`): a moving value is sampled on a grid of [`SAMPLE_BEATS`] (a sixty-fourth of a bar) and thinned by
//!   the runtime's own rule (a value is sent when it moved by `RAMP_EPS`, an lfo's by its own fraction of the depth), the leaps of
//!   a `step N` ramp and a square lfo on their exact beats, the last value of a ramp (and of an lfo that ends) exactly;
//! * `duck` (a `trick` live op: the pump, which the offline engine runs as the app does) and `mix` (a section's snapshot, put back
//!   on the bar the section ends on to what the render's own lines left, the engine's starting positions at first);
//! * the values a **performed** line reads (F13) are read 1/2, as everywhere in a render (docs/beatscript/ext-perform.md).
//!
//! Every event is placed in beats, and under a tempo curve (F3) carries the tempo of the step it falls in
//! ([`crate::beatscript_tempo`]), so it lands on the second the tempo map gives its beat.
//!
//! **What is not**: the arrangement format has no op for an insert (a filter, delay, reverb or crush), a move, a vocal or a
//! `do` call to anything else, so those lines are silent in a render. They are not dropped quietly: [`NotInRender`] counts them
//! and says which, and `djctl btw render` and `btw test` print it.
//!
//! **The control net** (docs/design/NETS-ALL-THE-WAY-DOWN.md, phase 1). The lines the arrangement has no op for (filters,
//! inserts, moves) are played by [`render`] from the program's control net (`djlive::nets`), merged with the arrangement's own
//! events frame by frame, so the offline render hears what the net says. A `macro` line (docs/beatscript/ext-control.md) is
//! the net's `fire` (and a held one's `release`), applied through [`crate::fx_view::fire`]: the registry's `effects fire`, as
//! the booth runs it. The `move` lines the net does not hold (a loop roll, a bass swap, a phrase sync's cut, a move with `for N
//! bars`) are armed on the booth's own scheduler, [`crate::perform::Moves`], serviced on the render's clock
//! ([`crate::offline_moves`]). A line neither holds is named in [`Render::beside`] and is not heard (a move that cannot act
//! offline, with why).

use std::collections::{BTreeMap, BTreeSet};

use djengine::json::{JsonObject, JsonValue};
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeEvent, ArrangeOp, Arrangement};
use djengine::session::DeckSource;
use djlive::ast::Deck;
use djlive::compile::{Call, Compiled, DeckIr, Effect, Ramp, Timing};
use djlive::diag::line_col;
use djlive::mixmoves::{Band, MixTarget};
use djlive::nets::Event;
use djlive::rat::Rat;
use djlive::shape::Lfo;

/// How finely a moving value (a ramp, an lfo) is sampled: a sixty-fourth of a bar, a sixteenth of a beat (31 ms at 120 BPM; the
/// runtime's own pass is about 16 ms). A value that did not move by the runtime's own threshold is not sent, so a long ramp costs a
/// few hundred events however fine the grid is. The grid is on absolute multiples of it, so the events fall on the tempo
/// staircase's own steps ([`crate::beatscript_tempo`]); a ramp's start and end are always on it, whatever beat they are.
pub const SAMPLE_BEATS: f64 = 1.0 / 16.0;

/// The arrangement, and how many beats it is (a song with a timeline: all its sections).
pub fn arrangement(c: &Compiled) -> (JsonValue, f64) {
    let (j, beats, _) = arrangement_report(c);
    (j, beats)
}

/// The arrangement, how many beats it is, and the control lines it could not carry.
pub fn arrangement_report(c: &Compiled) -> (JsonValue, f64, NotInRender) {
    build(c, true)
}

/// How many bars a program is, as a render for anyone else plays it (`beatscript render`, a clip from a render): an arranged song
/// its sections, unless an `end` (`end at bar N`, `end after SECTION`) cuts it shorter (the live runtime stops there); a
/// program with no sections its `end at bar N`, else 16 bars. (The desktop's [`arrangement`] plays a program with no
/// sections for 16 bars whatever its `end` says.)
pub fn program_bars(c: &Compiled) -> u32 {
    if c.arranged {
        let song = c.song_bars().unwrap_or(16);
        c.end.map_or(song, |e| e.min(song))
    } else {
        c.end.unwrap_or(16)
    }
    .max(1)
}

/// The arrangement of a program with no sections played for `bars` bars (not the 16 [`arrangement`] gives it), and how many
/// beats it is. An arranged song is its sections, whatever `bars` says.
pub fn arrangement_bars(c: &Compiled, bars: u32) -> (JsonValue, f64) {
    let (j, beats, _) = build_flat(c, true, bars.max(1));
    (j, beats)
}

/// `own_lines`: the program's own mixer and transport lines are in the arrangement (what a render is). A replay
/// ([`arrangement_with_replay`]) has the calls of every such line from its run, so it leaves them out and carries the section
/// loads, the `duck` and the `mix` lines only.
fn build(c: &Compiled, own_lines: bool) -> (JsonValue, f64, NotInRender) {
    build_flat(c, own_lines, 16)
}

/// [`build`], a program with no sections played for `flat_bars` bars.
fn build_flat(c: &Compiled, own_lines: bool, flat_bars: u32) -> (JsonValue, f64, NotInRender) {
    let bpm = c.tempo.unwrap_or(120.0);
    let sets: Vec<(u32, u32, &BTreeMap<Deck, DeckIr>)> = if c.arranged {
        c.segments.iter().map(|s| (s.start, s.bars, &s.decks)).collect()
    } else {
        vec![(0, flat_bars, &c.decks)]
    };
    let beats = sets.last().map_or(64.0, |(s, b, _)| f64::from(s + b) * 4.0);
    // a tempo curve (F3): a staircase of tempo-fader moves, and every event says the tempo it falls in
    let curve = crate::beatscript_tempo::Curve::of(c, &sets, beats);
    let mut decks: BTreeMap<String, DeckSource> = BTreeMap::new();
    let mut events: Vec<ArrangeEvent> = Vec::new();
    let mut playing: Vec<String> = Vec::new();
    let mut prev: Vec<Deck> = Vec::new();
    for (k, (start, _bars, nets)) in sets.iter().enumerate() {
        for (d, ir) in nets.iter() {
            let src = DeckSource::RawNets(crate::beatscript_tempo::envelope(ir, curve.as_ref().and_then(|cv| cv.net_tempo(k))));
            if k == 0 {
                decks.insert(d.name().to_string(), src);
                playing.push(d.name().to_string());
            } else {
                // a bar ahead, so the net lands on the section's bar line
                let at = f64::from(*start) * 4.0 - 2.0;
                events.push(ArrangeEvent { at, bpm, op: ArrangeOp::Deck(DeckAction::Load { deck: d.name().to_string(), source: src }), via: "btw".into(), text: format!("section at bar {start}: deck {}", d.name()) });
                // a deck that comes in starts on the section's bar line: one that is not playing starts its net the moment it
                // plays, so started a beat ahead (as it was) its notes and its bars fell a beat early, and so did a macro on it
                if !prev.contains(d) {
                    events.push(ArrangeEvent { at: f64::from(*start) * 4.0, bpm, op: ArrangeOp::Deck(DeckAction::Play { deck: d.name().to_string(), on: true }), via: "btw".into(), text: format!("deck {} in", d.name()) });
                }
            }
        }
        if k > 0 {
            for d in &prev {
                if !nets.contains_key(d) {
                    events.push(ArrangeEvent { at: f64::from(*start) * 4.0, bpm, op: ArrangeOp::Deck(DeckAction::Play { deck: d.name().to_string(), on: false }), via: "btw".into(), text: format!("deck {} out", d.name()) });
                }
            }
        }
        prev = nets.keys().copied().collect();
    }
    // the program's control lines (placed on exact beats; the tag below says the tempo each one falls in)
    let (moves, left_out) = control_events(c, bpm, beats, own_lines);
    events.extend(moves);
    if let Some(cv) = &curve {
        events.extend(cv.events());
    }
    // the render runs to the last move's bar: a last event at the song's end keeps it as long as the song
    events.push(ArrangeEvent { at: beats - 0.01, bpm, op: ArrangeOp::Live { method: "cue".into(), params: JsonValue::Object(Default::default()) }, via: "btw".into(), text: "the end".into() });
    let bpm = curve.as_ref().map_or(bpm, crate::beatscript_tempo::Curve::start_bpm);
    if let Some(cv) = &curve {
        cv.tag(&mut events);
    }
    let mut a = Arrangement::new(bpm, decks, events);
    playing.sort();
    a.start.playing = Some(playing);
    (a.json(), beats, left_out)
}

/// The most times one repeating line may fire in a render (`every 1/64 bar` over a long song is a flood, not a mix).
const MAX_FIRES: usize = 4096;

/// The places (in bars from the start, exact) a control fires at within a render of `total` bars: a bar line, or a time
/// between two (`at bar 4 beat 2`, `every 2 beats`).
pub(crate) fn fire_pos(t: Timing, total: u32) -> Vec<Rat> {
    let end = Rat::int(i64::from(total));
    let every = |from: Rat, step: Rat| -> Vec<Rat> {
        let mut out = Vec::new();
        let mut at = from;
        while at < end && out.len() < MAX_FIRES && !step.is_zero() {
            out.push(at);
            match at.add(step) {
                Ok(next) => at = next,
                Err(_) => break,
            }
        }
        out
    };
    match t {
        Timing::Now | Timing::NextMult(_) => vec![Rat::ZERO],
        Timing::At(b) => vec![Rat::int(i64::from(b))],
        Timing::AtPos(p) => vec![p],
        Timing::Every(n) => every(Rat::int(i64::from(n.max(1))), Rat::int(i64::from(n.max(1)))),
        Timing::EveryFrom(n, f) => every(Rat::int(i64::from(f)), Rat::int(i64::from(n.max(1)))),
        Timing::EveryPos { step, from } => every(from, step),
    }
}

/// A mix knob as the action that moves it.
fn mix_op(t: MixTarget, v: f64) -> DeckAction {
    let param = |deck: Deck, name: &str| DeckAction::Param { deck: deck.name().to_string(), name: name.to_string(), value: v };
    match t {
        MixTarget::Level(d) => param(d, "volume"),
        MixTarget::Eq(d, Band::Low) => param(d, "eqLow"),
        MixTarget::Eq(d, Band::Mid) => param(d, "eqMid"),
        MixTarget::Eq(d, Band::High) => param(d, "eqHigh"),
        MixTarget::Master => DeckAction::Master(v),
    }
}

fn live_op(c: &djlive::compile::Call) -> ArrangeOp {
    ArrangeOp::Live { method: c.noun.clone(), params: c.params.clone() }
}

/// What a knob is called in the render's own record of the mixer: a strip's parameter by deck and name, the master and the
/// crossfader by theirs. None for an action that sets no knob (a deck's play).
fn knob_of(op: &DeckAction) -> Option<((String, String), f64)> {
    match op {
        DeckAction::Param { deck, name, value } => Some(((deck.clone(), name.clone()), *value)),
        DeckAction::Master(v) => Some((("".into(), "master".into()), *v)),
        DeckAction::Crossfader(v) => Some((("".into(), "crossfader".into()), *v)),
        _ => None,
    }
}

/// The arrangement actions a registry call stands for: a `mixer` call (a strip's volume, EQ, mute, the master level, the
/// crossfader) or a deck's `play` / `stop`. None for anything else: the format has no op for it.
fn call_ops(noun: &str, p: &JsonValue) -> Vec<DeckAction> {
    match noun {
        "mixer" => mixer_ops(noun, p),
        "deck" => {
            let Some(deck) = p.get("deck").and_then(JsonValue::as_str) else { return Vec::new() };
            match p.get("action").and_then(JsonValue::as_str) {
                Some("play") => vec![DeckAction::Play { deck: deck.to_string(), on: true }],
                Some("stop" | "pause") => vec![DeckAction::Play { deck: deck.to_string(), on: false }],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

// ---- what a render cannot carry

/// A control line the render has no op for.
#[derive(Clone, Debug, PartialEq)]
pub struct LeftOut {
    /// 1-based, in the program text that was compiled.
    pub line: usize,
    pub col: usize,
    /// What kind of thing it is: `filter`, `delay`, `reverb`, `crush`, `move`, `vocal`, or the registry noun.
    pub kind: String,
    /// The line as written (its first line).
    pub text: String,
}

/// The control lines of a program a render is silent about: the arrangement has no op for an insert, a move, a vocal or most `do`
/// calls. One entry a line of the text (a line a `def` or a `repeat` copies counts once), in the order of the text.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NotInRender {
    pub lines: Vec<LeftOut>,
}

impl NotInRender {
    /// How many control lines.
    pub fn count(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// How many lines of each kind.
    pub fn kinds(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for l in &self.lines {
            *m.entry(l.kind.clone()).or_default() += 1;
        }
        m
    }

    /// `3 control lines (filter x2, reverb x1)`.
    pub fn headline(&self) -> String {
        let kinds: Vec<String> = self.kinds().into_iter().map(|(k, n)| format!("{k} x{n}")).collect();
        format!("{} control line{} ({})", self.count(), if self.count() == 1 { "" } else { "s" }, kinds.join(", "))
    }

    /// The first `n` lines as `line 5 `master: filter lp 300 -> 2000 over 8 bars``, and how many more there are.
    pub fn first(&self, n: usize) -> Vec<String> {
        let mut v: Vec<String> = self.lines.iter().take(n).map(|l| format!("line {} `{}`", l.line, l.text)).collect();
        if self.lines.len() > n {
            v.push(format!("{} more", self.lines.len() - n));
        }
        v
    }

    /// One line, for a report's note: `not in this render: 3 control lines (filter x2, reverb x1): line 5 `...`, line 9 `...`, 1 more`.
    pub fn note(&self) -> String {
        format!("not in this render: {}: {}", self.headline(), self.first(3).join(", "))
    }

    pub fn to_json(&self) -> JsonValue {
        let kinds = JsonObject::from_pairs(self.kinds().into_iter().map(|(k, n)| (k, JsonValue::int(n as i64))));
        JsonValue::Object(JsonObject::from_pairs([
            ("count", JsonValue::int(self.count() as i64)),
            ("kinds", JsonValue::Object(kinds)),
            (
                "lines",
                JsonValue::Array(
                    self.lines
                        .iter()
                        .map(|l| {
                            JsonValue::Object(JsonObject::from_pairs([("line", JsonValue::int(l.line as i64)), ("col", JsonValue::int(l.col as i64)), ("kind", JsonValue::str(l.kind.clone())), ("text", JsonValue::str(l.text.clone()))]))
                        })
                        .collect(),
                ),
            ),
            ("note", JsonValue::str(self.note())),
        ]))
    }
}

/// Where a control line is in the text that was compiled, and what to call it: the line itself (a line a `def` of the same text
/// copies is one place, the def's own line); but a line that came from an **import** has no text of its own here (its span is the
/// `import`), so it is the call that brought it (one place each), named by the call and the canonical line it made.
pub fn place_of(c: &Compiled, ctl: &djlive::compile::Control) -> (usize, String) {
    let own = ctl.span.text(&c.source).lines().next().unwrap_or("").trim();
    if !own.is_empty() && !own.starts_with("import") {
        return (ctl.span.start, own.to_string());
    }
    // (the key is `[section@bar] at bar 5: sweep(): master: filter lp 300`: without the place it was made in)
    let mut key = ctl.key.as_str();
    while key.starts_with('[') {
        match key.find("] ") {
            Some(i) => key = &key[i + 2..],
            None => break,
        }
    }
    (ctl.via.last().map_or(ctl.span.start, |v| v.start), key.to_string())
}

/// What kind of thing a call is, in the words a person uses: an insert is its effect (`filter`, `delay`, `reverb`, `crush`), an FX unit knob its name (`drive`).
fn kind_of(noun: &str, p: &JsonValue) -> String {
    let text = |k: &str| p.get(k).and_then(JsonValue::as_str).map(str::to_string);
    match noun {
        "inserts" => text("effect").or_else(|| text("param").and_then(|x| x.split('.').next().map(str::to_string))).unwrap_or_else(|| "insert".into()),
        "fx" => text("param").unwrap_or_else(|| "fx".into()),
        "moves" => "move".into(),
        "effects" if text("action").as_deref() == Some("fire") => text("id").map_or_else(|| "macro".into(), |id| format!("macro {id}")),
        "deck" => text("action").map_or_else(|| "deck".into(), |a| format!("deck {a}")),
        other => other.into(),
    }
}

/// The kind of the first call of an effect that the render has no op for; None when it carries all of it (or the effect is a duck
/// or a mix, which have their own way in).
fn uncarried(e: &Effect) -> Option<String> {
    let first = |calls: &[Call]| calls.iter().find(|c| call_ops(&c.noun, &c.params).is_empty()).map(|c| kind_of(&c.noun, &c.params));
    match e {
        Effect::Calls(cs) => first(cs),
        Effect::Ramp(r) => first(&r.prelude).or_else(|| first(&[r.at(0.0)])),
        Effect::Lfo(l) => first(&l.prelude).or_else(|| first(&[l.call_at(0.0)])),
        Effect::Duck(_) | Effect::Mix(_) | Effect::Performed(_) => None,
    }
}

/// Whether the render carries any of an effect.
fn carries_any(e: &Effect) -> bool {
    match e {
        Effect::Calls(cs) => cs.iter().any(|c| !call_ops(&c.noun, &c.params).is_empty()),
        Effect::Ramp(r) => !call_ops(&r.noun, &r.at(0.0).params).is_empty(),
        Effect::Lfo(l) => !call_ops(&l.noun, &l.call_at(0.0).params).is_empty(),
        Effect::Duck(_) | Effect::Mix(_) => true,
        Effect::Performed(_) => false,
    }
}

// ---- a moving value, sampled

/// The calls a ramp that began at `start` (beats) makes when it is looked at on the sampling grid, as the runtime's `move_ramps`
/// would send them: the value at each point (`Ramp::at`, so a shape and the six-decimal rounding are the runtime's own), only when
/// it moved by `RAMP_EPS`, the end value always. A ramp stops where the next run of its line begins (`cut`), and nothing is sent
/// at or after `limit` (the render's end). The leaps of a `step N` shape are on their exact beats.
fn ramp_samples(r: &Ramp, start: f64, cut: f64, limit: f64) -> Vec<(f64, Call)> {
    let len = r.bars.to_f64() * 4.0;
    if len <= 0.0 {
        return Vec::new();
    }
    let end = start + len;
    let hi = end.min(cut).min(limit);
    let mut pts = vec![start];
    let mut k = (start / SAMPLE_BEATS).floor() as i64 + 1;
    while (k as f64) * SAMPLE_BEATS < hi - 1e-9 {
        pts.push(k as f64 * SAMPLE_BEATS);
        k += 1;
    }
    pts.extend(r.shape.breaks().into_iter().map(|t| start + t * len));
    pts.push(end);
    pts.sort_by(f64::total_cmp);
    pts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut last: Option<f64> = None;
    let mut out = Vec::new();
    for p in pts {
        if p >= cut - 1e-9 || p >= limit - 1e-9 {
            break;
        }
        let t = if p >= end - 1e-9 { 1.0 } else { ((p - start) / len).clamp(0.0, 1.0) };
        let v = r.value(t);
        if t >= 1.0 || last.is_none_or(|l| (l - v).abs() >= djlive::runtime::RAMP_EPS) {
            out.push((p, r.at(t)));
            last = Some(v);
        }
    }
    out
}

/// The same for an lfo (`Lfo::sample`, which is the runtime's own `LfoRun`): looked at on the grid and at the leaps of a square wave,
/// and, when it ends by itself before the next run of its line begins, at its end, where the runtime sends the last value once.
fn lfo_samples(l: &Lfo, hold: Option<djlive::dur::Dur>, start: f64, cut: f64, limit: f64) -> Vec<(f64, Call)> {
    let start_bars = start / 4.0;
    let stop = l.stops_at(start_bars, 0.0, hold).map(|s| s * 4.0);
    let hi = stop.unwrap_or(f64::INFINITY).min(cut).min(limit);
    let mut pts = vec![start];
    let mut k = (start / SAMPLE_BEATS).floor() as i64 + 1;
    while (k as f64) * SAMPLE_BEATS < hi - 1e-9 {
        pts.push(k as f64 * SAMPLE_BEATS);
        k += 1;
    }
    pts.extend(l.leaps(start_bars, start_bars, hi / 4.0).into_iter().map(|b| b * 4.0));
    if let Some(s) = stop.filter(|s| *s < cut - 1e-9) {
        pts.push(s);
    }
    pts.sort_by(f64::total_cmp);
    pts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    pts.retain(|p| *p < cut - 1e-9 && *p < limit - 1e-9);
    let bars: Vec<f64> = pts.iter().map(|p| p / 4.0).collect();
    l.sample(start_bars, hold, &bars).into_iter().map(|(p, c)| (p * 4.0, c)).collect()
}

/// What a timeline entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    /// A hold ends: what a line changed is put back, before any line fires on that beat.
    Back = 0,
    /// A line fires.
    Fire = 1,
    /// A moving value is looked at (after the lines of its beat have fired, as the runtime's pass does).
    Sample = 2,
}

/// The events being made, and the render's own record of what it has set on the mixer (a section's mix reads it before it
/// changes a knob, as the runtime reads the real mixer; there is no mixer to read offline).
struct Out {
    events: Vec<ArrangeEvent>,
    state: BTreeMap<(String, String), f64>,
    bpm: f64,
    /// Where the events being made fall, in beats.
    beat: f64,
}

impl Out {
    /// An action of the mixer or a deck, kept in the record.
    fn act(&mut self, op: DeckAction, text: String) {
        if let Some((key, v)) = knob_of(&op) {
            self.state.insert(key, v);
        }
        self.live(ArrangeOp::Deck(op), text);
    }

    fn live(&mut self, op: ArrangeOp, text: String) {
        self.events.push(ArrangeEvent { at: self.beat, bpm: self.bpm, op, via: "btw".into(), text });
    }
}

/// The program's control lines as arrangement events, in the order the engine should meet them (a section's mix put back before
/// the next section's is set), and the lines it has no op for. A duck is the `trick` pump with the program's tempo choosing its
/// release note; a mix, a plain value, a mute, a ramp and an lfo are `param` / `master` / `crossfader` actions, a deck's play
/// and stop `play` ones. Events at or after the render's end are left out. See the module's words for what is sampled and how.
///
/// `own_lines` false leaves the program's own mixer and transport lines out (a replay has their calls): only `duck` and `mix`.
pub fn control_events(c: &Compiled, bpm: f64, beats: f64, own_lines: bool) -> (Vec<ArrangeEvent>, NotInRender) {
    let total = (beats / 4.0).ceil() as u32;
    // a line with a performed value or `when` (F13) has no controller here: it is read with every input at 1/2, the value the
    // compiler checks it with, so `when held(key)` is true and `when knob(1) < 0.5` is not (docs/beatscript/ext-perform.md)
    let mut plans: BTreeMap<usize, Effect> = BTreeMap::new();
    let mut left = NotInRender::default();
    let mut said: BTreeSet<usize> = BTreeSet::new();
    for (i, ctl) in c.controls.iter().enumerate() {
        let e = match &ctl.effect {
            Effect::Performed(p) => match p.at_half() {
                Some(e) => e,
                None => continue,
            },
            other => other.clone(),
        };
        if !matches!(e, Effect::Duck(_) | Effect::Mix(_)) {
            if !own_lines {
                continue;
            }
            if let Some(kind) = uncarried(&e) {
                let (anchor, text) = place_of(c, ctl);
                if said.insert(anchor) {
                    let (line, col) = line_col(&c.source, anchor);
                    left.lines.push(LeftOut { line, col, kind, text });
                }
            }
            if !carries_any(&e) {
                continue;
            }
        }
        plans.insert(i, e);
    }
    left.lines.sort_by_key(|l| (l.line, l.col));
    // (beat, phase, control, number): the number is the fire's, or the index of the sample's call
    let mut timeline: Vec<(f64, Phase, usize, usize)> = Vec::new();
    let mut samples: Vec<Call> = Vec::new();
    for (&i, plan) in &plans {
        let ctl = &c.controls[i];
        let fires = fire_pos(ctl.timing, total);
        let at: Vec<f64> = fires.iter().map(|r| r.to_f64() * 4.0).collect();
        for (k, &b) in at.iter().enumerate() {
            timeline.push((b, Phase::Fire, i, k));
            if let Some(h) = ctl.hold {
                if let Ok(back) = fires[k].add(h.as_bars()) {
                    timeline.push((back.to_f64() * 4.0, Phase::Back, i, k));
                }
            }
            // a moving value is looked at until it ends, or until its line fires again (which starts it afresh)
            let cut = at.get(k + 1).copied().unwrap_or(f64::INFINITY);
            let moving = match plan {
                Effect::Ramp(r) => ramp_samples(r, b, cut, beats),
                Effect::Lfo(l) => lfo_samples(l, ctl.hold, b, cut, beats),
                _ => Vec::new(),
            };
            for (p, call) in moving {
                samples.push(call);
                timeline.push((p, Phase::Sample, i, samples.len() - 1));
            }
        }
    }
    timeline.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
    let mut out = Out { events: Vec::new(), state: BTreeMap::new(), bpm, beat: 0.0 };
    let mut priors: BTreeMap<(usize, usize), Vec<(MixTarget, f64)>> = BTreeMap::new();
    for (beat, phase, i, k) in timeline {
        if beat >= beats {
            continue;
        }
        out.beat = beat;
        let ctl = &c.controls[i];
        match (&plans[&i], phase) {
            (Effect::Duck(d), Phase::Fire) => out.live(live_op(&d.on_call(c.tempo.unwrap_or(bpm))), ctl.key.clone()),
            (Effect::Duck(d), _) => out.live(live_op(&d.off_call()), format!("put back {}", ctl.key)),
            (Effect::Mix(m), Phase::Fire) => {
                let mut prior = Vec::new();
                for it in &m.items {
                    let op = mix_op(it.target, it.knob);
                    prior.push((it.target, knob_of(&op).and_then(|(key, _)| out.state.get(&key).copied()).unwrap_or_else(|| it.target.neutral())));
                    out.act(op, ctl.key.clone());
                }
                if m.restore {
                    priors.insert((i, k), prior);
                }
            }
            (Effect::Mix(_), _) => {
                for (t, v) in priors.remove(&(i, k)).unwrap_or_default().into_iter().rev() {
                    out.act(mix_op(t, v), format!("put back {}", ctl.key));
                }
            }
            (Effect::Calls(cs), Phase::Fire) => {
                for call in cs {
                    for op in call_ops(&call.noun, &call.params) {
                        out.act(op, ctl.key.clone());
                    }
                }
            }
            (_, Phase::Back) => {
                // a hold ends: the calls that put the line back (a mute is unmuted, a deck that was started is stopped)
                for call in &ctl.undo {
                    for op in call_ops(&call.noun, &call.params) {
                        out.act(op, format!("put back {}", ctl.key));
                    }
                }
            }
            (Effect::Ramp(_) | Effect::Lfo(_), Phase::Sample) => {
                let call = &samples[k];
                for op in call_ops(&call.noun, &call.params) {
                    // a value the mixer already has is not sent again
                    if knob_of(&op).is_some_and(|(key, v)| out.state.get(&key) == Some(&v)) {
                        continue;
                    }
                    out.act(op, ctl.key.clone());
                }
            }
            _ => {}
        }
    }
    let out = out.events;
    (out, left)
}

/// The program's `duck` and `mix` lines as arrangement events (what a render of a replay carries of the program itself): see
/// [`control_events`].
pub fn mix_events(c: &Compiled, bpm: f64, beats: f64) -> Vec<ArrangeEvent> {
    control_events(c, bpm, beats, false).0
}

/// One registry call a control line makes, on the render's timeline: what the live runtime would hand its host on that beat.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedCall {
    /// Beats from the song's first bar line.
    pub beat: f64,
    /// 0: a hold ends and the line is put back; 1: the line fires; 2: a moving value is looked at (the order on one beat).
    pub phase: u8,
    /// The control's index in `Compiled::controls`, and its key.
    pub control: usize,
    pub key: String,
    pub call: Call,
}

/// Every registry call the program's control lines make that `keep` takes, in the order the runtime makes them, on the beats
/// [`control_events`] puts its own on: a line's calls on each beat it fires (a ramp's or an lfo's setup first), its put-back
/// where a `for` hold ends, and a ramp's or an lfo's value sampled on the same grid and thinned by the same rule (the runtime's
/// `RAMP_EPS`, an lfo's own fraction of its depth), the leaps of a step shape and the last value exact. Whatever the noun: this
/// is how a render plays the lines the arrangement format has no op for (an insert, the FX unit, the vocoder, the Wheel) the
/// way a host that takes the calls themselves (the browser's engine) hears them. `duck` and `mix` lines are not calls here
/// (the arrangement carries them). A performed line is read with its inputs at 1/2, as everywhere in a render. Calls at or
/// after `beats` are left out.
///
/// A line that fires on a bar line is the last word on what it sets (the runtime's `fire_bar`): a ramp that began before it on
/// the same insert or knob ends there, without its last value (the riser's `master: filter hp 100 -> 5000 over 8 bars` that
/// reaches its end on the very bar line the next section's `master: filter off` is on).
pub fn control_calls(c: &Compiled, beats: f64, keep: &dyn Fn(&Call) -> bool) -> Vec<TimedCall> {
    let total = (beats / 4.0).ceil() as u32;
    let mut out: Vec<TimedCall> = Vec::new();
    let any = |cs: &[Call]| cs.iter().any(keep);
    let resolved = |ctl: &djlive::compile::Control| match &ctl.effect {
        Effect::Performed(p) => p.at_half(),
        other => Some(other.clone()),
    };
    // the slots each bar line's lines set (lines on a bar line only: the runtime's `fire_bar`; a time between bar lines is not
    // the last word on anything)
    let mut on_bar: BTreeMap<i64, BTreeSet<djlive::runtime::Slot>> = BTreeMap::new();
    for ctl in &c.controls {
        if !matches!(ctl.timing, Timing::Now | Timing::At(_) | Timing::Every(_) | Timing::EveryFrom(..) | Timing::NextMult(_)) {
            continue;
        }
        let Some(e) = resolved(ctl) else { continue };
        let slots = djlive::runtime::effect_slots(&e);
        if slots.is_empty() {
            continue;
        }
        for at in fire_pos(ctl.timing, total) {
            if let Some(l) = at.as_int() {
                on_bar.entry(l).or_default().extend(slots.iter().cloned());
            }
        }
    }
    // where a ramp that began at `start` (beats) is ended by a line on its slot: the first such bar line after it
    let ended_at = |r: &Ramp, start: f64| -> f64 {
        let Some(slot) = djlive::runtime::slot_of(&r.at(0.0)) else { return f64::INFINITY };
        on_bar.iter().find(|(l, s)| (**l as f64) * 4.0 > start + 1e-9 && s.contains(&slot)).map_or(f64::INFINITY, |(l, _)| *l as f64 * 4.0)
    };
    for (i, ctl) in c.controls.iter().enumerate() {
        let Some(e) = resolved(ctl) else { continue };
        let takes = match &e {
            Effect::Calls(cs) => any(cs),
            Effect::Ramp(r) => any(&r.prelude) || keep(&r.at(0.0)),
            Effect::Lfo(l) => any(&l.prelude) || keep(&l.call_at(0.0)),
            Effect::Duck(_) | Effect::Mix(_) | Effect::Performed(_) => false,
        } || any(&ctl.undo);
        if !takes {
            continue;
        }
        let mut push = |beat: f64, phase: u8, call: &Call| {
            if beat < beats && keep(call) {
                out.push(TimedCall { beat, phase, control: i, key: ctl.key.clone(), call: call.clone() });
            }
        };
        let fires = fire_pos(ctl.timing, total);
        let at: Vec<f64> = fires.iter().map(|r| r.to_f64() * 4.0).collect();
        for (k, &b) in at.iter().enumerate() {
            match &e {
                Effect::Calls(cs) => cs.iter().for_each(|x| push(b, Phase::Fire as u8, x)),
                Effect::Ramp(r) => r.prelude.iter().for_each(|x| push(b, Phase::Fire as u8, x)),
                Effect::Lfo(l) => l.prelude.iter().for_each(|x| push(b, Phase::Fire as u8, x)),
                _ => {}
            }
            if let Some(h) = ctl.hold {
                if let Ok(back) = fires[k].add(h.as_bars()) {
                    let bb = back.to_f64() * 4.0;
                    ctl.undo.iter().for_each(|x| push(bb, Phase::Back as u8, x));
                }
            }
            let cut = at.get(k + 1).copied().unwrap_or(f64::INFINITY);
            let moving = match &e {
                Effect::Ramp(r) => ramp_samples(r, b, cut.min(ended_at(r, b)), beats),
                Effect::Lfo(l) => lfo_samples(l, ctl.hold, b, cut, beats),
                _ => Vec::new(),
            };
            for (p, call) in moving {
                push(p, Phase::Sample as u8, &call);
            }
        }
    }
    // (stable: a line's calls keep their order on a beat)
    out.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.phase.cmp(&b.phase)).then(a.control.cmp(&b.control)));
    out
}

// ---- a performance, replayed offline (F13)

/// One control call a replay made: the program position it was made at, and the call.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayedCall {
    /// Beats from the program's own first bar line (program bar 0), on the clock the replay ran on.
    pub beat: f64,
    pub noun: String,
    pub params: JsonValue,
}

/// The clock and the host of a replay: deck A plays from beat 0 and the runtime is serviced a quarter beat at a time. The
/// controllers are not asked (the performance answers), the calls are written down.
struct ReplayHost {
    beat: f64,
    calls: Vec<ReplayedCall>,
}

impl djlive::runtime::LiveHost for ReplayHost {
    fn grid(&self) -> Option<djlive::runtime::Grid> {
        Some(djlive::runtime::Grid { deck: "A".into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, _deck: &str) -> bool {
        true
    }

    fn load_net(&mut self, _deck: &str, _envelope: &JsonValue) -> Result<(), String> {
        Ok(())
    }

    fn set_playing(&mut self, _deck: &str, _on: bool) {}

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push(ReplayedCall { beat: self.beat, noun: noun.to_string(), params: params.clone() });
        Ok(JsonValue::Null)
    }
}

/// The most bars a replay runs, whatever the program or the performance says (a song is at most this long: `MAX_SONG_BARS`).
pub const MAX_REPLAY_BARS: u32 = djlive::compile::MAX_SONG_BARS as u32;

/// The most beats a render may be asked for with `--beats` (four thousand and ninety-six bars): longer is not a render but a memory
/// the size of a day's recording.
pub const MAX_RENDER_BEATS: f64 = 16_384.0;

/// A beat count from a caller (`djctl btw render --beats N`): finite, above zero and at most [`MAX_RENDER_BEATS`], else the words that say so.
/// (NaN and a negative or zero count wrote an empty file and said nothing; infinity and a huge count never returned.)
pub fn render_beats(x: f64) -> Result<f64, String> {
    if x > 0.0 && x <= MAX_RENDER_BEATS {
        Ok(x)
    } else {
        Err(format!("--beats is 0 to {MAX_RENDER_BEATS} beats (a bar is 4), not {x}"))
    }
}

/// A bar count from a caller (`djctl btw replay --bars N`): from 1 to [`MAX_REPLAY_BARS`].
pub fn replay_bar_count(x: f64) -> Result<u32, String> {
    if x >= 1.0 && x <= f64::from(MAX_REPLAY_BARS) && x.fract() == 0.0 {
        Ok(x as u32)
    } else {
        Err(format!("--bars is a whole number of bars from 1 to {MAX_REPLAY_BARS}, not {x}"))
    }
}

/// A performance with a value **past the program's end** is refused: nothing reads an input after the program has ended, so it is
/// not a performance of this program. (`bar == end` is fine: the last bar the program plays is `end - 1` of its own count, and the
/// clock's own bar of it is `end`.)
pub fn refuse_past_end(c: &Compiled, perf: &djlive::perform::Performance) -> Result<(), String> {
    match (c.end, perf.span_bars()) {
        (Some(e), Some((_, last))) if last > i64::from(e) => Err(format!("refused: this performance has a value at bar {last}, and the program ends at bar {e}")),
        _ => Ok(()),
    }
}

/// How many bars a replay runs: the program's own length when it has one (at most [`MAX_REPLAY_BARS`]), else past the last value the
/// performance holds. Refused: a value past the program's end ([`refuse_past_end`]), and a performance that reaches further than a
/// replay can run.
pub fn replay_bars(c: &Compiled, perf: &djlive::perform::Performance) -> Result<u32, String> {
    refuse_past_end(c, perf)?;
    let last = perf.span_bars().map_or(0, |(_, last)| last.max(0));
    match c.end {
        Some(e) => Ok(e.saturating_add(1).min(MAX_REPLAY_BARS)),
        None if last > i64::from(MAX_REPLAY_BARS) - 2 => Err(format!("refused: this performance reaches bar {last}, and a replay runs at most {MAX_REPLAY_BARS} bars (--bars N says how many)")),
        None => Ok(perf.span_bars().map_or(8, |_| last as u32 + 2)),
    }
}

/// The controls of a performance: the program `text` run on a fake clock from its first bar for `bars` bars with its inputs read
/// from `perf`, the calls it makes (the same calls a live run of the performance made, with the same numbers). Refused when the
/// performance is of another program than `text`.
pub fn replay_controls(text: &str, env: &djlive::env::Env, perf: &djlive::perform::Performance, bars: u32) -> Result<Vec<ReplayedCall>, String> {
    if bars > MAX_REPLAY_BARS {
        return Err(format!("a replay runs at most {MAX_REPLAY_BARS} bars, not {bars}"));
    }
    run_program(text, env, Some(perf), bars, 0.25)
}

/// The calls the runtime itself makes for a program with no performance: `text` run on the same fake clock as a replay (deck A
/// plays from beat 0, the program lands on bar 1, so a song's own beat 0 is `beat - 4`), serviced every `step` beats for `bars`
/// bars. What the render's moving values are compared to (the runtime's own values at the same beats: the tests of
/// `tests/beatscript_render_controls.rs`).
pub fn runtime_controls(text: &str, env: &djlive::env::Env, bars: u32, step: f64) -> Result<Vec<ReplayedCall>, String> {
    run_program(text, env, None, bars, step)
}

fn run_program(text: &str, env: &djlive::env::Env, perf: Option<&djlive::perform::Performance>, bars: u32, step: f64) -> Result<Vec<ReplayedCall>, String> {
    let mut live = djlive::Live::new();
    let ev = live.eval_with(text, env);
    if !ev.ok {
        let n = ev.diagnostics.iter().filter(|d| d.is_error()).count();
        return Err(format!("the program has {n} error{}: `btw check` says where", if n == 1 { "" } else { "s" }));
    }
    if let Some(p) = perf {
        live.replay(p.clone())?;
    }
    let mut host = ReplayHost { beat: 0.0, calls: Vec::new() };
    let passes = (f64::from(bars) * 4.0 / step).round() as usize;
    for k in 0..passes {
        live.service(&mut host);
        host.beat = (k + 1) as f64 * step;
    }
    Ok(host.calls)
}

/// What an offline render honours of a replay's controls, and what it leaves out.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReplayRender {
    /// Calls that became events of the arrangement.
    pub honoured: usize,
    /// The nouns of the calls that did not, with how many of each: the arrangement format has an op for a strip's volume, EQ
    /// and mute, the master level and the crossfader (the `mixer` noun) and a deck's play and stop, not for an insert, a move or
    /// a `do` call.
    pub left_out: std::collections::BTreeMap<String, usize>,
}

/// The arrangement of a program with the mixer calls of a replay in it: the program's sections, `duck` and `mix` lines, and an event
/// for each `mixer` call the replay made (a strip's `volume`, `low` / `mid` / `high`, `mute`, the `master` level, the `crossfader`:
/// the replay ran every mixer line of the program, ramps and lfos included, so the render does not play them a second time) and each
/// deck `play` / `stop`. The program's bar 0 is the clock's bar 0 and its song starts on bar 1 (the bar a program lands on), so a call
/// is placed a bar before its beat.
pub fn arrangement_with_replay(c: &Compiled, calls: &[ReplayedCall]) -> (JsonValue, f64, ReplayRender) {
    use djengine::session::arrangement::Arrangement;
    let (j, beats, _) = build(c, false);
    let bpm = c.tempo.unwrap_or(120.0);
    let mut report = ReplayRender::default();
    let Some((mut a, _)) = Arrangement::from_json(&j) else { return (j, beats, report) };
    for call in calls {
        let at = (call.beat - 4.0).max(0.0);
        let ops = call_ops(&call.noun, &call.params);
        if ops.is_empty() {
            *report.left_out.entry(call.noun.clone()).or_default() += 1;
            continue;
        }
        for op in ops {
            report.honoured += 1;
            a.events.push(ArrangeEvent { at, bpm, op: ArrangeOp::Deck(op), via: "btw replay".into(), text: format!("{} {}", call.noun, call.params.canonical_js()) });
        }
    }
    a.events.sort_by(|x, y| x.at.partial_cmp(&y.at).unwrap_or(std::cmp::Ordering::Equal));
    (a.json(), beats, report)
}

/// The arrangement actions a `mixer` call stands for (none for any other noun).
fn mixer_ops(noun: &str, p: &JsonValue) -> Vec<DeckAction> {
    if noun != "mixer" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let deck = p.get("deck").and_then(JsonValue::as_str);
    if let Some(d) = deck {
        for (key, name) in [("volume", "volume"), ("low", "eqLow"), ("mid", "eqMid"), ("high", "eqHigh")] {
            if let Some(v) = p.get(key).and_then(JsonValue::as_f64) {
                out.push(DeckAction::Param { deck: d.to_string(), name: name.to_string(), value: v });
            }
        }
        if let Some(m) = p.get("mute").and_then(JsonValue::as_bool) {
            out.push(DeckAction::Param { deck: d.to_string(), name: "mute".to_string(), value: if m { 1.0 } else { 0.0 } });
        }
    }
    if let Some(v) = p.get("master").and_then(JsonValue::as_f64) {
        out.push(DeckAction::Master(v));
    }
    if let Some(v) = p.get("crossfader").and_then(JsonValue::as_f64) {
        out.push(DeckAction::Crossfader(v));
    }
    out
}

/// What [`render`] made.
pub struct Render {
    pub audio: djengine::session::wav::RenderedAudio,
    /// The net events applied, and how many the engine refused (with their beat and why).
    pub applied: usize,
    pub refused: Vec<(f64, String)>,
    /// The control lines the net does not hold (key, why): not heard in this render.
    pub beside: Vec<(String, String)>,
}

/// Apply one control event to the offline engine.
pub fn apply_event(e: &mut djengine::host::deck::engine::Engine, ev: &Event) -> Result<(), String> {
    use djengine::fx::insert::{InsertKind, InsertParam};
    use djengine::fx::strip::StripKnob;
    use djengine::fx::tricks::{TrickArgs, TrickKind};
    use djengine::fx::unit::FxParam;
    use djengine::host::deck::inserts::MASTER;
    use djengine::host::deck::tricks::TrickQuant;
    // the registry says `Master`; the engine's chain for it is `master`
    let strip = |t: &str| if t.eq_ignore_ascii_case("master") { MASTER.to_string() } else { t.to_string() };
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(format!("the engine refused {what}")) };
    match ev {
        Event::Insert { target, effect, on } => {
            let k = InsertKind::from_id(effect).ok_or_else(|| format!("no insert effect {effect:?}"))?;
            ok(e.set_insert_on(&strip(target), k, *on), &ev.text())
        }
        Event::Set { target, param, value } => match param.as_str() {
            "master" => {
                e.set_master(*value);
                Ok(())
            }
            "crossfader" => {
                e.set_crossfader(*value);
                Ok(())
            }
            // the FX unit's drive on a deck's chain (the master chain's distortion; djrust `fx::unit`); `Master` is both decks'
            "drive" => {
                let decks: Vec<String> = if target.eq_ignore_ascii_case("master") { e.decks.iter().map(|d| d.name.clone()).collect() } else { vec![target.clone()] };
                for d in &decks {
                    ok(e.set_fx(d, FxParam::Drive, *value), &ev.text())?;
                }
                Ok(())
            }
            "volume" => ok(e.set_volume(target, *value), &ev.text()),
            "low" => ok(e.set_strip(target, StripKnob::EqLow, *value), &ev.text()),
            "mid" => ok(e.set_strip(target, StripKnob::EqMid, *value), &ev.text()),
            "high" => ok(e.set_strip(target, StripKnob::EqHigh, *value), &ev.text()),
            p => {
                let ip = InsertParam::from_id(p).ok_or_else(|| format!("no knob {p:?}"))?;
                ok(e.set_insert_param(&strip(target), ip, *value), &ev.text())
            }
        },
        Event::Mute { deck, on } => ok(e.set_deck_mute(deck, *on), &ev.text()),
        Event::Trick { deck, kind, on, amount, bars, beats } => {
            let k = TrickKind::parse(kind).ok_or_else(|| format!("no trick {kind:?}"))?;
            let args = on.then(|| {
                let mut a = TrickArgs { ..k.default_args() };
                if let Some(v) = amount {
                    a.amount = *v;
                }
                if let Some(v) = bars {
                    a.bars = *v;
                }
                if let Some(v) = beats {
                    a.beats = *v;
                }
                a
            });
            e.trick(deck, k, *on, args, TrickQuant::Now)
        }
        // a `macro` line (docs/beatscript/ext-control.md): the registry's `effects fire` / `release`, as the booth runs them
        Event::Fire { target, id, bars, latch } => crate::fx_view::fire(e, target, id, *bars, *latch),
        Event::Release { target, id } => crate::fx_view::release(e, target, id),
    }
}

/// Render a program offline with its control net: the arrangement's moves and the net's events on one
/// timeline, each on the frame of its beat. `controls: false` renders as before (notes, sounds and
/// sections only), for comparison.
pub fn render(c: &Compiled, sample_rate: f64, controls: bool) -> Result<Render, String> {
    let (json, beats) = arrangement(c);
    let arr = parsed(&json)?;
    render_with(c, &arr, beats, beats, sample_rate, controls, None)
}

/// An arrangement this module made, read back (every event must parse).
pub fn parsed(json: &JsonValue) -> Result<Arrangement, String> {
    let (arr, bad) = Arrangement::from_json(json).ok_or("not an arrangement")?;
    if bad > 0 {
        return Err(format!("{bad} arrangement events did not parse"));
    }
    Ok(arr)
}

/// What [`render_with`] shows the engine to: once when the start stands (its first nets installed, nothing rendered) and
/// after every block. A stem plays the other parts' lanes at level 0 here (`crate::clip_source`).
pub type Watch<'a> = &'a mut dyn FnMut(&mut djengine::host::deck::offline::OfflineConductor);

/// [`render`] of `arr`, an arrangement of `c` that is `beats` long ([`arrangement`], [`arrangement_bars`]): its first `until`
/// beats (the same frames as the whole render's, which is stepped from frame 0 all the same), with `watch` shown the engine.
pub fn render_with(c: &Compiled, arr: &Arrangement, beats: f64, until: f64, sample_rate: f64, controls: bool, mut watch: Option<Watch<'_>>) -> Result<Render, String> {
    use djengine::host::deck::offline::{arrangement_conductor, stand_on_start, ArrangeRenderOptions, OfflineConductor};
    use djengine::session::wav::RenderedAudio;
    let opts = ArrangeRenderOptions { sample_rate, ..Default::default() };
    let mut cond = arrangement_conductor(arr, &opts);
    let mut refused = stand_on_start(&mut cond, arr, None, None);
    if let Some(w) = watch.as_mut() {
        // the first section's nets installed now, so what the watch does to them is there before the first frame
        cond.pump();
        w(&mut cond);
    }
    let net = djlive::nets::control_net(c, (beats / 4.0).ceil() as u32);
    let ev: Vec<(i64, Event)> = if controls { net.events(None) } else { Vec::new() };
    // the move lines the net leaves beside it, on the booth's own scheduler (crate::offline_moves); none without controls
    let clock = crate::offline_moves::Clock::of_arrangement(arr, sample_rate);
    let mut moves = controls.then(|| crate::offline_moves::ProgramMoves::new(c, beats, &net, clock)).filter(|m| !m.is_empty() || !m.unplayed.is_empty());
    // one timeline: (frame, 0 = an arrangement move | 1 = a net event, index); the arrangement first on a tie
    let frame_of = |at: f64| (arr.seconds_at(at) * sample_rate).round() as i64;
    let mut tl: Vec<(i64, u8, usize)> = arr.events.iter().enumerate().map(|(i, e)| (frame_of(e.at), 0, i)).collect();
    tl.extend(ev.iter().enumerate().map(|(i, (t, _))| (frame_of(*t as f64 / 4.0), 1, i)));
    tl.sort();
    let want = (arr.seconds_at(until.min(beats)) * sample_rate).round().max(0.0) as usize;
    let (mut left, mut right) = (Vec::with_capacity(want), Vec::with_capacity(want));
    let (mut next, mut applied) = (0, 0);
    // a macro's call on a deck that does not play yet (one that comes in on the bar the macro starts on) waits for it: made just
    // before the deck starts (the engine starts an engagement on a deck that does not play at once, so it starts with the deck), or
    // at that bar if the deck never comes. (the frame of that bar, the net event)
    let mut deferred: Vec<(i64, usize)> = Vec::new();
    let fire = |e: &mut djengine::host::deck::engine::Engine, i: usize, applied: &mut usize, refused: &mut Vec<(f64, String)>| match apply_event(e, &ev[i].1) {
        Ok(()) => *applied += 1,
        Err(why) => refused.push((ev[i].0 as f64 / 4.0, why)),
    };
    while left.len() < want {
        let frame = cond.engine().frame();
        while next < tl.len() && tl[next].0 <= frame {
            let (_, kind, i) = tl[next];
            next += 1;
            let e = cond.engine_mut();
            if kind == 0 {
                let at = arr.events[i].at;
                if let ArrangeOp::Deck(DeckAction::Play { deck, on: true }) = &arr.events[i].op {
                    let mut k = 0;
                    while k < deferred.len() {
                        if matches!(&ev[deferred[k].1].1, Event::Fire { target, .. } if target == deck) {
                            let (_, j) = deferred.remove(k);
                            fire(e, j, &mut applied, &mut refused);
                        } else {
                            k += 1;
                        }
                    }
                }
                match &arr.events[i].op {
                    ArrangeOp::Live { method, params } => {
                        if let Err(why) = e.apply_live_op(method, params) {
                            refused.push((at, format!("{method}: {why}")));
                        }
                    }
                    op => {
                        if !e.apply_arrange_op(op) {
                            refused.push((at, format!("{} refused", op.op())));
                        }
                    }
                }
            } else {
                if let Event::Fire { target, .. } = &ev[i].1 {
                    if e.deck(target).is_some_and(|d| !d.playing) {
                        let b = ev[i].0 as f64 / 16.0;
                        let bar = if (b - b.round()).abs() < 1e-9 { b.round() + 1.0 } else { b.ceil() };
                        deferred.push((frame_of(bar * 4.0), i));
                        continue;
                    }
                }
                fire(e, i, &mut applied, &mut refused);
            }
        }
        let mut k = 0;
        while k < deferred.len() {
            if deferred[k].0 <= frame {
                let (_, j) = deferred.remove(k);
                fire(cond.engine_mut(), j, &mut applied, &mut refused);
            } else {
                k += 1;
            }
        }
        // the moves: what is due on this frame, then the scheduler's pass on the render's clock (every block)
        if let Some(m) = moves.as_mut() {
            m.run(cond.engine_mut(), frame, &frame_of);
        }
        // a render cut short of its arrangement steps the engine in the blocks the whole render does (its last one is not cut
        // at `until`, the frames past it are dropped), so its frames are the whole render's however the engine takes a block
        let mut n = if until < beats { OfflineConductor::BLOCK_FRAMES } else { (want - left.len()).min(OfflineConductor::BLOCK_FRAMES) };
        if let Some(t) = tl.get(next) {
            n = n.min((t.0 - frame).max(1) as usize);
        }
        if let Some(d) = deferred.iter().map(|(f, _)| *f).min() {
            n = n.min((d - frame).max(1) as usize);
        }
        if let Some(f) = moves.as_ref().and_then(|m| m.next_frame(cond.engine(), frame, &frame_of)) {
            n = n.min((f - frame).max(1) as usize);
        }
        let (l, r) = cond.step_frames(n);
        left.extend_from_slice(l);
        right.extend_from_slice(r);
        if let Some(w) = watch.as_mut() {
            w(&mut cond);
        }
    }
    left.truncate(want);
    right.truncate(want);
    // a move line played on the scheduler is heard: not beside; one that cannot act says why
    let beside = match &moves {
        None => net.residual,
        Some(m) => net.residual.into_iter().filter(|(k, _)| !m.plays(k)).map(|(k, why)| match m.why_unplayed(&k) {
            Some(w) => (k, w.to_string()),
            None => (k, why),
        }).collect(),
    };
    Ok(Render { audio: RenderedAudio { sample_rate, left, right }, applied, refused, beside })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIX_SONG: &str = "tempo 124\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n}\nsection two 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  duck B under A.kick 6db\n  mix { A level 0.7, B eq low -3 }\n}\nsection three 4 bars {\n  deck A: kick x x x x\n  mix { master level 0.5 }\n}\narrange one two three\n";

    fn moves(a: &Arrangement) -> Vec<(f64, String)> {
        a.events.iter().filter(|e| e.via == "btw" && (e.op.op() == "trick" || e.op.op() == "param" || e.op.op() == "master")).map(|e| (e.at, e.op.json().canonical_js())).collect()
    }

    /// A duck and a mix are in the render: the pump on at the section's bar line and off where it ends, the mixer set and
    /// put back (to what the render's own lines left: the engine's starting positions).
    #[test]
    fn a_duck_and_a_mix_are_events_of_the_render() {
        let c = djlive::check(MIX_SONG).compiled.unwrap();
        let (j, beats) = arrangement(&c);
        assert_eq!(beats, 48.0);
        let (a, bad) = Arrangement::from_json(&j).unwrap();
        assert_eq!(bad, 0, "every event reads back");
        let m = moves(&a);
        let on = r#"{"action":"on","amount":0.499,"deck":"B","div":16,"kind":"pump","op":"trick","source":"A"}"#;
        let off = r#"{"action":"off","deck":"B","kind":"pump","op":"trick"}"#;
        // section two is bars 4..8 (beats 16..32)
        assert_eq!(m[0], (16.0, on.to_string()));
        assert_eq!(m[1].0, 16.0);
        assert!(m[1].1.contains(r#""name":"volume""#) && m[1].1.contains(r#""value":0.7"#), "{m:?}");
        assert!(m[2].1.contains(r#""name":"eqLow""#) && m[2].1.contains(r#""value":0.4375"#), "{m:?}");
        // put back on the bar line the section ends on, the pump off, the knobs as the render had them
        let back: Vec<_> = m.iter().filter(|(at, _)| *at == 32.0).collect();
        assert_eq!(back.len(), 4, "{m:?}");
        assert_eq!(back[0].1, off);
        assert!(back[1].1.contains(r#""name":"eqLow""#) && back[1].1.contains(r#""value":0.5"#), "{back:?}");
        assert!(back[2].1.contains(r#""name":"volume""#) && back[2].1.contains(r#""value":0.8"#), "{back:?}");
        // section three: the master is set; its put back is the render's end, which is left out
        assert!(m.iter().any(|(at, j)| *at == 32.0 && j.contains(r#""op":"master""#) && j.contains("0.5")), "{m:?}");
        assert!(m.iter().all(|(at, _)| *at < 48.0));
    }

    /// The sound of it: the offline engine runs the pump as the app does, so B's stem dips just after each kick of A and is back
    /// before the next one, and the engine refuses nothing of what the program sent.
    #[test]
    fn the_pump_ducks_deck_b_under_deck_as_kick_in_the_render() {
        use crate::deck::offline::{render_arrangement, ArrangeRenderOptions};
        let render = |extra: &str| {
            let src = format!("tempo 124\ndeck A: kick x ~ x ~\ndeck B: bass c2 len=16\n{extra}");
            let c = djlive::check(&src).compiled.unwrap_or_else(|| panic!("{src}"));
            let (j, _) = arrangement(&c);
            let (arr, bad) = Arrangement::from_json(&j).unwrap();
            assert_eq!(bad, 0);
            render_arrangement(&arr, ArrangeRenderOptions { stems: true, beats: Some(16.0), sample_rate: 22_050.0, ..ArrangeRenderOptions::default() })
        };
        let plain = render("");
        let ducked = render("duck B under A.kick 12db\n");
        assert!(ducked.refused.is_empty(), "the engine took every move: {:?}", ducked.refused);
        let stem = |r: &crate::deck::offline::ArrangeRender| r.stems.iter().find(|(n, _)| n == "B").map(|(_, a)| a.clone()).expect("B's stem");
        let (p, d) = (stem(&plain), stem(&ducked));
        let rms = |a: &djengine::session::wav::RenderedAudio, from: f64, to: f64| {
            let (i, j) = ((from * a.sample_rate) as usize, (to * a.sample_rate) as usize);
            (a.left[i..j].iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / (j - i) as f64).sqrt()
        };
        // a beat is 60/124 s; the kicks fall on beats 0 and 2 of each bar. Look at the second bar's second kick (beat 6).
        let beat = 60.0 / 124.0;
        let kick = 6.0 * beat;
        let (near_p, near_d) = (rms(&p, kick + 0.005, kick + 0.030), rms(&d, kick + 0.005, kick + 0.030));
        let (late_p, late_d) = (rms(&p, kick + 0.35, kick + 0.45), rms(&d, kick + 0.35, kick + 0.45));
        assert!(near_p > 0.001, "deck B plays: {near_p}");
        assert!(near_d / near_p < 0.7, "B is down just after the kick: {} of {}", near_d, near_p);
        assert!(late_d / late_p > 0.95, "and back before the next one: {} of {}", late_d, late_p);
    }

    /// A duck and a mix at a time between two bar lines (F1 x F8): the event lands on that beat, a `for 2 beats` hold is put back
    /// exactly two beats later, the rest on the bar line the section ends on.
    #[test]
    fn a_duck_and_a_mix_between_bar_lines_are_events_at_that_beat() {
        let src = "tempo 124\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  at bar 1 beat 2: mix { A level 0.5 }\n  at bar 2 beat 1 for 2 beats: mix { master level 0.2 }\n  at bar 2 beat 2: duck B under A.kick 6db\n}\nsection two 4 bars {\n  deck A: kick x ~ x ~\n}\narrange one two\n";
        let c = djlive::check(src).compiled.unwrap();
        let (j, beats) = arrangement(&c);
        assert_eq!(beats, 32.0);
        let (a, bad) = Arrangement::from_json(&j).unwrap();
        assert_eq!(bad, 0);
        let m = moves(&a);
        let at = |t: f64, frag: &str| m.iter().any(|(at, j)| (*at - t).abs() < 1e-9 && j.contains(frag));
        assert!(at(6.0, r#""name":"volume""#) && at(6.0, r#""value":0.5"#), "{m:?}");
        assert!(at(9.0, r#""op":"master""#) && at(9.0, "0.2"), "{m:?}");
        // two beats later, to what the render held (the engine's own start)
        assert!(at(11.0, r#""op":"master""#) && at(11.0, r#""value":1"#), "{m:?}");
        assert!(at(10.0, r#""action":"on""#) && at(10.0, r#""kind":"pump""#), "{m:?}");
        // the section's bar line, 16.0: the pump off and the fader back
        assert!(at(16.0, r#""action":"off""#), "{m:?}");
        assert!(at(16.0, r#""name":"volume""#) && at(16.0, r#""value":0.8"#), "{m:?}");
        assert_eq!(m.len(), 6, "{m:?}");
        // outside a section, `every 2 beats` fires on each of them within the render
        let c = djlive::check("tempo 124\ndeck A: kick x ~ x ~\nevery 2 beats from bar 1: mix { master level 0.4 }\n").compiled.unwrap();
        let (j, _) = arrangement(&c);
        let (a, _) = Arrangement::from_json(&j).unwrap();
        let ats: Vec<f64> = moves(&a).iter().map(|(at, _)| *at).collect();
        assert_eq!(&ats[..4], &[4.0, 6.0, 8.0, 10.0]);
        assert!(ats.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 1e-9), "{ats:?}");
    }

    /// F13: the offline render has no controllers, so a mix or a duck with a performed `when` is read with every input at 1/2:
    /// `when held(key)` is true, `when knob(1) < 0.5` is not.
    #[test]
    fn a_mix_with_a_performed_guard_is_in_the_render_when_it_holds_at_one_half() {
        let src = "tempo 124\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  at bar 1: mix { A level 0.7 } when held(key)\n  at bar 2: mix { B level 0.3 } when knob(1) < 0.5\n  at bar 3: mix { master level 0.4 } when fader(A) >= 0.5\n}\narrange one\n";
        let c = djlive::check(src).compiled.unwrap();
        let (j, _) = arrangement(&c);
        let (a, bad) = Arrangement::from_json(&j).unwrap();
        assert_eq!(bad, 0);
        let m = moves(&a);
        assert!(m.iter().any(|(at, j)| *at == 4.0 && j.contains(r#""name":"volume""#) && j.contains("0.7")), "held(key) is true at 1/2: {m:?}");
        assert!(!m.iter().any(|(_, j)| j.contains("0.3")), "knob(1) < 1/2 is false at 1/2: {m:?}");
        assert!(m.iter().any(|(at, j)| *at == 12.0 && j.contains(r#""op":"master""#) && j.contains("0.4")), "fader(A) >= 1/2 holds at 1/2: {m:?}");
    }

    #[test]
    fn a_program_without_either_renders_as_it_did() {
        let c = djlive::check("tempo 140\nsection one 4 bars {\n  deck A: kick x ~ x ~\n}\narrange one\n").compiled.unwrap();
        let (j, _) = arrangement(&c);
        let (a, _) = Arrangement::from_json(&j).unwrap();
        assert!(moves(&a).is_empty());
    }

    const PERFORMED: &str = "tempo 120\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  every 1 bars: deck A: level knob(1)\n  every 2 bars: master: filter lp 200 + knob(2) * 7800\n}\narrange one\n";

    /// A performance of PERFORMED: knob(1) is the bar number over eight, knob(2) 1/2.
    fn performance_of_performed() -> djlive::perform::Performance {
        let cid = djlive::check(PERFORMED).compiled.unwrap().cid;
        let mut r = Vec::new();
        for bar in 1..=4 {
            r.push(djlive::perform::Reading { bar, at: 0, input: "knob(1)".into(), value: bar as f64 / 8.0 });
            r.push(djlive::perform::Reading { bar, at: 0, input: "knob(2)".into(), value: 0.5 });
        }
        djlive::perform::Performance::new(cid, r)
    }

    #[test]
    fn a_performance_replays_offline_into_the_same_calls_the_literal_program_makes() {
        let perf = performance_of_performed();
        let calls = replay_controls(PERFORMED, &djlive::env::Env::none(), &perf, 6).unwrap();
        let levels: Vec<(f64, String)> = calls.iter().filter(|c| c.noun == "mixer").map(|c| (c.beat, c.params.canonical_js())).collect();
        // the song lands on program bar 1 (beat 4): its `every 1 bars` line fires from bar 2 of the program, with the logged values
        assert!(!levels.is_empty(), "{calls:?}");
        assert!(levels.iter().any(|(_, p)| p == r#"{"deck":"A","volume":0.25}"#), "{levels:?}");
        assert!(levels.iter().any(|(_, p)| p == r#"{"deck":"A","volume":0.375}"#), "{levels:?}");
        // the sweep has the cutoff the knob made: 200 + 0.5 * 7800 = 4100 Hz
        let want = djlive::compile::sweep_knob(4100.0, false);
        assert!(calls.iter().any(|c| c.noun == "inserts" && c.params.get("value").and_then(JsonValue::as_f64) == Some(want)), "{calls:?}");
        // a replay is a function of the program and the performance: the same twice
        assert_eq!(replay_controls(PERFORMED, &djlive::env::Env::none(), &perf, 6).unwrap(), calls);
    }

    #[test]
    fn a_performance_of_another_program_is_refused_offline_too() {
        let perf = performance_of_performed();
        let e = replay_controls("tempo 100\nmaster: level knob(1)\n", &djlive::env::Env::none(), &perf, 4).unwrap_err();
        assert!(e.starts_with("refused:"), "{e}");
        let e = replay_controls("tempo 100\nmaster: level 0.5 0.6\n", &djlive::env::Env::none(), &perf, 4).unwrap_err();
        assert!(e.contains("error"), "{e}");
    }

    #[test]
    fn the_render_honours_the_mixer_calls_of_a_replay_and_says_what_it_leaves_out() {
        let perf = performance_of_performed();
        let comp = djlive::check(PERFORMED).compiled.unwrap();
        let calls = replay_controls(PERFORMED, &djlive::env::Env::none(), &perf, replay_bars(&comp, &perf).unwrap()).unwrap();
        let (j, beats, report) = arrangement_with_replay(&comp, &calls);
        assert_eq!(beats, 16.0);
        let (a, bad) = Arrangement::from_json(&j).unwrap();
        assert_eq!(bad, 0);
        let params: Vec<(f64, String, f64)> = a
            .events
            .iter()
            .filter(|e| e.via == "btw replay")
            .filter_map(|e| match &e.op {
                ArrangeOp::Deck(DeckAction::Param { name, value, .. }) => Some((e.at, name.clone(), *value)),
                _ => None,
            })
            .collect();
        // the strip's volume, from the performed knob, in the render at the song's own beats
        assert!(params.iter().any(|(at, n, v)| n == "volume" && *v == 0.25 && (*at - 4.0).abs() < 1e-9), "{params:?}");
        assert!(report.honoured >= 3, "{report:?}");
        // the filter's insert sets are not in the arrangement format: said, counted
        assert!(report.left_out.get("inserts").copied().unwrap_or(0) >= 2, "{report:?}");
        assert!(a.events.windows(2).all(|w| w[0].at <= w[1].at), "in order");
    }

    #[test]
    fn a_song_is_an_arrangement_with_a_load_before_each_section() {
        let c = djlive::check("tempo 140\nsection one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck A: kick x x x x\n  deck B: bass c2\n}\narrange one two\n").compiled.unwrap();
        let (j, beats) = arrangement(&c);
        assert_eq!(beats, 32.0);
        let (a, bad) = Arrangement::from_json(&j).unwrap();
        assert_eq!(bad, 0);
        assert_eq!(a.bpm, 140.0);
        assert!(a.decks.contains_key("A") && !a.decks.contains_key("B"));
        // A is reloaded a bar ahead of bar 4 (beat 16 - 2), B comes in
        assert!(a.events.iter().any(|e| (e.at - 14.0).abs() < 1e-9 && e.op.op() == "load" && e.op.deck().as_deref() == Some("A")));
        assert!(a.events.iter().any(|e| e.op.op() == "load" && e.op.deck().as_deref() == Some("B")));
        assert!(a.events.iter().any(|e| e.op.op() == "play" && e.op.deck().as_deref() == Some("B")));
    }
}
