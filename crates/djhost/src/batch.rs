//! Batches (beyond Swift; docs/CONTROL.md, "Batches"): several control commands as one request,
//! optionally landing together on the bar grid.
//!
//! A batch is an ordered list of commands, each in any form the control language takes
//! (`{"method": noun, "params": {...}}`, params beside the method, `"noun.verb"`), plus options:
//!
//! | option | meaning |
//! |---|---|
//! | `atomic` | every command is checked against the registry first and none runs if any is invalid; then all of them run in one service pass of the window. Refuses per-command `delay` |
//! | `stop_on_error` | stop at the first command that fails (default on, unless `atomic`) |
//! | `at` | `now` (default), `next_beat`, `next_bar`, `next_phrase` (the next 8-bar line) or `bar:N`: when the batch lands, on the grid deck's own bar numbering |
//! | `deck` | the grid deck to time on (default the first of A, B that plays) |
//! | `delay` (per command) | beats (or `"2 bars"`, `"1 beat"`) after the landing, for a short timed sequence: a filter sweep in steps. At most [`MAX_DELAY_BEATS`]; anything longer is a setlist or the live-code language |
//! | `dry_run` | validate, say when it would land and what each command would change; run nothing |
//!
//! # Timing
//!
//! It is not a second scheduler: it counts the grid deck's lines the way Auto-DJ and the set
//! queue do ([`crate::autodj::lines_crossed`]: a loop's wrap or a swap to the top is a line too),
//! and lands on the service pass that crosses the line (~16 ms in the window), so a batch
//! `at: next_bar` shows up with the Perform moves. The scheduler is pure: [`Batches::service`]
//! takes a [`Clock`] (the grid decks' beats and the time) and answers the [`Fire`]s that are due;
//! the window runs them through its own `control_call` and reports each back with
//! [`Batches::record`]. The tests drive it with a fake clock.
//!
//! With no deck playing there is no grid: a timed `at` lands at once (as the queue's first
//! segment and the Perform moves do), and a `delay` counts beats at 120 BPM.

use djengine::json::{JsonObject, JsonValue};

use crate::control_registry::{self as reg, Effect};
use crate::perform::{Grid, BEATS_PER_BAR};
use crate::setqueue::PHRASE_BARS;

/// Most commands in one batch.
pub const MAX_COMMANDS: usize = 256;
/// The longest `delay`, in beats (4 bars): a short sequence, not a set.
pub const MAX_DELAY_BEATS: f64 = 16.0;
/// Batches waiting or running at once.
pub const MAX_LIVE: usize = 16;
/// Finished batches `status` still knows.
pub const KEEP_DONE: usize = 16;
/// The farthest ahead a `bar:N` may be, in bars.
pub const MAX_LEAD_BARS: i64 = 1024;
/// The tempo a `delay` counts at when no deck plays.
pub const IDLE_BPM: f64 = 120.0;

/// Selector params: they pick what a command acts on, they are not what it changes.
const SELECTORS: [&str; 8] = ["action", "deck", "input", "strip", "target", "instance", "source", "section"];

// ---- the options ----------------------------------------------------------------------------

/// When a batch lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Now,
    NextBeat,
    NextBar,
    /// The next multiple of [`PHRASE_BARS`] bars.
    NextPhrase,
    /// A bar of the grid deck's own numbering (bar 0 is its start, as `log` and the queue count).
    Bar(i64),
}

impl At {
    pub fn parse(s: &str) -> Result<At, String> {
        let t = s.trim().to_ascii_lowercase().replace('-', "_");
        match t.as_str() {
            "" | "now" => Ok(At::Now),
            "next_beat" | "beat" => Ok(At::NextBeat),
            "next_bar" | "bar" => Ok(At::NextBar),
            "next_phrase" | "phrase" => Ok(At::NextPhrase),
            _ => {
                let n = t.strip_prefix("bar:").and_then(|n| n.trim().parse::<i64>().ok());
                match n {
                    Some(n) if n >= 0 => Ok(At::Bar(n)),
                    _ => Err(format!(
                        "at: {s:?} is not one of now, next_beat, next_bar, next_phrase, bar:N (a bar of the grid deck, from 0)"
                    )),
                }
            }
        }
    }

    pub fn text(self) -> String {
        match self {
            At::Now => "now".into(),
            At::NextBeat => "next_beat".into(),
            At::NextBar => "next_bar".into(),
            At::NextPhrase => "next_phrase".into(),
            At::Bar(n) => format!("bar:{n}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub atomic: bool,
    pub stop_on_error: bool,
    pub at: At,
    /// The deck whose grid the batch is timed on (default: the first of A, B that plays).
    pub deck: Option<String>,
    pub label: Option<String>,
    pub dry_run: bool,
}

/// One command of a batch, as the registry reads it.
#[derive(Clone, Debug)]
pub struct Command {
    pub index: usize,
    /// The canonical noun ("" when the command could not be read at all).
    pub noun: String,
    /// The verb ("" for a noun with none).
    pub action: String,
    /// What is sent to the noun: its params, with `action` set where the noun has verbs.
    pub params: JsonValue,
    /// Beats after the landing.
    pub delay: f64,
    pub effect: Effect,
    /// Why it is not valid, if it is not.
    pub error: Option<String>,
}

impl Command {
    fn bad(index: usize, why: String) -> Command {
        Command {
            index,
            noun: String::new(),
            action: String::new(),
            params: JsonValue::Object(JsonObject::new()),
            delay: 0.0,
            effect: Effect::Read,
            error: Some(why),
        }
    }

    /// Read one command in any form the language takes, and check it against the registry: the
    /// noun, the verb, the required params. A command that fails keeps its `error`.
    pub fn parse(index: usize, item: &JsonValue) -> Command {
        let Some(o) = item.as_object() else {
            return Command::bad(index, "expected {\"method\": <noun>, \"params\": {…}}".into());
        };
        let Some(method) = o.get("method").and_then(JsonValue::as_str).filter(|m| !m.is_empty()) else {
            return Command::bad(index, "expected {\"method\": <noun>, \"params\": {…}}: no method".into());
        };
        let delay = match o.get("delay") {
            None | Some(JsonValue::Null) => 0.0,
            Some(v) => match parse_delay(v) {
                Ok(d) => d,
                Err(e) => return Command::bad(index, e),
            },
        };
        // params, or the params beside `method` (as the socket frames it)
        let mut params = match o.get("params") {
            Some(JsonValue::Object(p)) => p.clone(),
            Some(JsonValue::Null) | None => {
                let mut p = JsonObject::new();
                for (k, v) in o.iter().filter(|(k, _)| !matches!(*k, "id" | "method" | "params" | "jsonrpc" | "delay")) {
                    p.set(k, v.clone());
                }
                p
            }
            Some(_) => return Command::bad(index, "params must be an object".into()),
        };
        // "<noun>.<verb>" names both; an explicit action wins, as in the socket
        let (name, dotted) = match method.split_once('.') {
            Some((n, v)) if !n.is_empty() && !v.is_empty() => (n, Some(v)),
            _ => (method, None),
        };
        if let Some(v) = dotted {
            if !params.contains_key("action") {
                params.set("action", JsonValue::str(v));
            }
        }
        let Some(n) = reg::noun(name) else {
            return Command::bad(
                index,
                format!("unknown method {name:?}; one of {} (describe lists them with their verbs)", reg::methods().join(", ")),
            );
        };
        let nope = |params: &JsonObject, why: String| Command {
            noun: n.name.to_string(),
            params: JsonValue::Object(params.clone()),
            ..Command::bad(index, why)
        };
        if n.name == "batch" {
            return nope(&params, "a batch cannot hold a batch".into());
        }
        if n.local {
            return nope(&params, format!("{}: runs in the MCP server, not on the socket: it cannot be batched", n.name));
        }
        let given = params.get("action").and_then(JsonValue::as_str).map(str::to_string);
        // `post` on files runs on the socket's own thread, not the window's
        if n.name == "post" && matches!(given.as_deref(), Some("process" | "measure" | "presets" | "list")) {
            return nope(&params, "post: file actions run on the socket's own thread, not in a batch".into());
        }
        let mut action = String::new();
        let mut effect = if n.actions.is_empty() { params_only_effect(n.name, &params) } else { Effect::Read };
        if !n.actions.is_empty() {
            let name = given.filter(|g| !g.is_empty()).unwrap_or_else(|| n.default_action.to_string());
            if name.is_empty() {
                return nope(&params, format!(
                    "{}: needs an action; one of {}",
                    n.name,
                    n.actions.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
                ));
            }
            let Some(a) = n.action(&name) else {
                return nope(&params, format!(
                    "{}: unknown action {name:?}; one of {}",
                    n.name,
                    n.actions.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
                ));
            };
            for p in a.params.iter().filter(|p| p.required) {
                if params.get(p.name).is_none_or(|v| matches!(v, JsonValue::Null)) {
                    return nope(&params, format!("{} {}: needs \"{}\" ({})", n.name, a.name, p.name, p.doc));
                }
            }
            effect = a.effect;
            params.set("action", JsonValue::str(&name));
            action = name;
        }
        Command {
            index,
            noun: n.name.to_string(),
            action,
            params: JsonValue::Object(params),
            delay,
            effect,
            error: None,
        }
    }

    /// `deck.play`, `mixer`: the command in a word.
    pub fn name(&self) -> String {
        match (self.noun.is_empty(), self.action.is_empty()) {
            (true, _) => "?".into(),
            (false, true) => self.noun.clone(),
            (false, false) => format!("{}.{}", self.noun, self.action),
        }
    }
}

/// A noun with no verbs is a bag of knobs: with none set it reads.
fn params_only_effect(noun: &str, params: &JsonObject) -> Effect {
    if matches!(noun, "status" | "state" | "describe" | "ping" | "methods") {
        return Effect::Read;
    }
    if params.keys().any(|k| !SELECTORS.contains(&k)) {
        Effect::Write
    } else {
        Effect::Read
    }
}

/// A delay: a number of beats, or `"2 beats"`, `"1 bar"`, `"0.5 bars"`.
fn parse_delay(v: &JsonValue) -> Result<f64, String> {
    let beats = match v {
        JsonValue::Number(n) => *n,
        JsonValue::String(t) => {
            let t = t.trim().to_ascii_lowercase();
            let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
            let (num, unit) = t.split_at(split);
            let n: f64 = num.parse().map_err(|_| format!("delay {t:?}: a number of beats, or \"2 beats\" / \"1 bar\""))?;
            match unit.trim() {
                "" | "b" | "beat" | "beats" => n,
                "bar" | "bars" => n * BEATS_PER_BAR,
                u => return Err(format!("delay {t:?}: unit {u:?} is not beats or bars")),
            }
        }
        _ => return Err("delay: a number of beats, or \"2 beats\" / \"1 bar\"".into()),
    };
    if !beats.is_finite() || beats < 0.0 {
        return Err("delay: not before the landing (0 or more)".into());
    }
    if beats > MAX_DELAY_BEATS {
        return Err(format!(
            "delay {beats} beats is over the {MAX_DELAY_BEATS}-beat limit: a longer sequence is a set list or the live-code language"
        ));
    }
    Ok(beats)
}

/// A parsed `batch run` request.
#[derive(Clone, Debug)]
pub struct Plan {
    pub opts: Options,
    pub commands: Vec<Command>,
}

fn flag(p: &JsonValue, key: &str) -> Result<Option<bool>, String> {
    Ok(match p.get(key) {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Bool(b)) => Some(*b),
        Some(JsonValue::String(t)) => match t.as_str() {
            "true" | "on" | "yes" | "1" => Some(true),
            "false" | "off" | "no" | "0" => Some(false),
            _ => return Err(format!("batch: {key} is true or false")),
        },
        Some(_) => return Err(format!("batch: {key} is true or false")),
    })
}

impl Plan {
    /// Read the request's options and commands. Per-command problems stay on the commands (a dry
    /// run reports them all); problems with the request itself are errors.
    pub fn parse(p: &JsonValue) -> Result<Plan, String> {
        let items = match p.get("commands") {
            Some(JsonValue::Array(a)) => a,
            Some(_) => return Err("batch: commands is an array of {method, params}".into()),
            None => return Err("batch: needs \"commands\": an array of {\"method\": <noun>, \"params\": {…}}".into()),
        };
        if items.is_empty() {
            return Err("batch: commands is empty: give at least one {\"method\": <noun>, \"params\": {…}}".into());
        }
        if items.len() > MAX_COMMANDS {
            return Err(format!("batch: {} commands is over the limit of {MAX_COMMANDS}", items.len()));
        }
        let atomic = flag(p, "atomic")?.unwrap_or(false);
        let stop_on_error = flag(p, "stop_on_error")?.unwrap_or(!atomic);
        let dry_run = flag(p, "dry_run")?.unwrap_or(false);
        let at = match p.get("at") {
            None | Some(JsonValue::Null) => At::Now,
            Some(JsonValue::String(t)) => At::parse(t)?,
            Some(_) => return Err("batch: at is now, next_beat, next_bar, next_phrase or bar:N".into()),
        };
        let deck = p.get("deck").and_then(JsonValue::as_str).filter(|d| !d.is_empty()).map(str::to_uppercase);
        let label = p.get("label").and_then(JsonValue::as_str).filter(|l| !l.is_empty()).map(|l| l.chars().take(60).collect());
        let commands: Vec<Command> = items.iter().enumerate().map(|(i, c)| Command::parse(i, c)).collect();
        if atomic {
            if let Some(c) = commands.iter().find(|c| c.delay > 0.0) {
                return Err(format!(
                    "batch: atomic lands every command at once, but command {} has a delay: drop the delay, or drop atomic",
                    c.index
                ));
            }
        }
        Ok(Plan { opts: Options { atomic, stop_on_error, at, deck, label, dry_run }, commands })
    }

    /// The commands that are not valid.
    pub fn invalid(&self) -> Vec<&Command> {
        self.commands.iter().filter(|c| c.error.is_some()).collect()
    }
}

// ---- the clock ------------------------------------------------------------------------------

/// What the scheduler reads of the booth: the time, and each deck's place on its grid.
#[derive(Clone, Debug, Default)]
pub struct Clock {
    /// Seconds, on any steady clock.
    pub time: f64,
    pub grids: Vec<(String, Grid)>,
}

impl Clock {
    /// The grid a batch times on: `prefer`'s when it plays, else the first deck that does.
    pub fn pick(&self, prefer: Option<&str>) -> Option<(&str, &Grid)> {
        let plays = |g: &Grid| g.playing && g.bpm > 0.0;
        prefer
            .and_then(|d| self.grids.iter().find(|(n, g)| n == d && plays(g)))
            .or_else(|| self.grids.iter().find(|(_, g)| plays(g)))
            .map(|(n, g)| (n.as_str(), g))
    }
}

/// Lines of `unit` beats crossed between two looks at a deck's beat. A jump back by more than a
/// couple of beats is a loop's wrap or a swap to the top, which is a line too; a small step back
/// is the platter's jitter.
fn crossed(prev: f64, cur: f64, unit: f64) -> i64 {
    let n = |b: f64| (b / unit + 1e-9).floor() as i64;
    if cur + 2.0 >= prev {
        (n(cur) - n(prev)).max(0)
    } else {
        1
    }
}

fn bar_of(beat: f64) -> i64 {
    (beat / BEATS_PER_BAR + 1e-9).floor() as i64
}

// ---- the scheduler --------------------------------------------------------------------------

/// What a batch waits for.
#[derive(Clone, Debug, PartialEq)]
enum Wait {
    /// `left` more lines of `unit` beats on `deck`'s grid, last seen at beat `last`.
    Lines { left: i64, unit: f64, deck: String, last: f64 },
    Landed,
}

impl Wait {
    /// What `at` waits for from this clock; the note says why a timed one lands at once.
    fn new(at: At, prefer: Option<&str>, clock: &Clock) -> Result<(Wait, Option<String>), String> {
        if at == At::Now {
            return Ok((Wait::Landed, None));
        }
        let Some((deck, g)) = clock.pick(prefer) else {
            return Ok((Wait::Landed, Some("no deck is playing, so there is no grid: it landed at once".into())));
        };
        let (left, unit) = match at {
            At::Now => unreachable!(),
            At::NextBeat => (1, 1.0),
            At::NextBar => (1, BEATS_PER_BAR),
            At::NextPhrase => (PHRASE_BARS - bar_of(g.beat).rem_euclid(PHRASE_BARS), BEATS_PER_BAR),
            At::Bar(n) => {
                let now = bar_of(g.beat);
                if n <= now {
                    return Err(format!("batch: bar {n} has already begun: deck {deck} is in bar {now}"));
                }
                if n - now > MAX_LEAD_BARS {
                    return Err(format!("batch: bar {n} is {} bars away: the limit is {MAX_LEAD_BARS}", n - now));
                }
                (n - now, BEATS_PER_BAR)
            }
        };
        Ok((Wait::Lines { left, unit, deck: deck.to_string(), last: g.beat }, None))
    }

    fn text(&self) -> JsonValue {
        match self {
            Wait::Landed => JsonValue::Null,
            Wait::Lines { left, unit, deck, .. } => JsonValue::Object(JsonObject::from_pairs([
                ("deck", JsonValue::str(deck)),
                ("unit", JsonValue::str(if *unit == 1.0 { "beat" } else { "bar" })),
                ("lines_left", JsonValue::int(*left)),
            ])),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Out {
    Waiting,
    /// Handed out by `service`, not recorded yet.
    Fired,
    Ran(Result<JsonValue, String>),
    Skipped(String),
}

/// Where a batch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for its line.
    Pending,
    /// Landed, with timed commands still to come.
    Running,
    Done,
    Cancelled,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Running => "running",
            State::Done => "done",
            State::Cancelled => "cancelled",
        }
    }
}

/// One batch, waiting or finished.
#[derive(Clone, Debug)]
pub struct Batch {
    pub id: String,
    pub opts: Options,
    pub commands: Vec<Command>,
    out: Vec<Out>,
    wait: Wait,
    state: State,
    /// Beats since the landing.
    elapsed: f64,
    last_time: Option<f64>,
    pub submitted: f64,
    note: Option<String>,
    /// The command whose failure stopped it.
    stopped: Option<usize>,
}

/// A command that is due: run it, then [`Batches::record`] what it answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Fire {
    pub batch: String,
    pub index: usize,
    pub noun: String,
    pub params: JsonValue,
}

fn obj<const N: usize>(pairs: [(&str, JsonValue); N]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

fn num(v: f64) -> JsonValue {
    JsonValue::from((v * 1000.0).round() / 1000.0)
}

impl Batch {
    fn advance(&mut self, clock: &Clock) {
        let dt = self.last_time.map_or(0.0, |t| (clock.time - t).max(0.0));
        self.last_time = Some(clock.time);
        match &mut self.wait {
            Wait::Landed => {
                let bpm = clock.pick(self.opts.deck.as_deref()).map_or(IDLE_BPM, |(_, g)| g.bpm);
                self.elapsed += dt * bpm / 60.0;
            }
            Wait::Lines { left, unit, deck, last } => {
                let Some((d, g)) = clock.pick(self.opts.deck.as_deref()) else {
                    return; // nothing plays: the line does not come until a deck does
                };
                if d != deck {
                    // the grid moved to the other deck: count from there
                    *deck = d.to_string();
                    *last = g.beat;
                } else {
                    *left -= crossed(*last, g.beat, *unit);
                    *last = g.beat;
                }
                if *left <= 0 {
                    self.wait = Wait::Landed;
                    self.elapsed = 0.0;
                    self.state = State::Running;
                }
            }
        }
    }

    /// Commands due now, in order of their delay then their place. An invalid one is recorded
    /// as failed here (it never reaches the app).
    fn collect(&mut self, fires: &mut Vec<Fire>) {
        if self.wait != Wait::Landed {
            return;
        }
        let mut due: Vec<usize> = (0..self.commands.len())
            .filter(|&i| self.out[i] == Out::Waiting && self.commands[i].delay <= self.elapsed + 1e-9)
            .collect();
        due.sort_by(|&a, &b| self.commands[a].delay.total_cmp(&self.commands[b].delay).then(a.cmp(&b)));
        for i in due {
            if self.stopped.is_some() {
                break;
            }
            let c = &self.commands[i];
            match &c.error {
                Some(e) => {
                    let e = e.clone();
                    self.record(i, Err(e));
                }
                None => {
                    fires.push(Fire { batch: self.id.clone(), index: i, noun: c.noun.clone(), params: c.params.clone() });
                    self.out[i] = Out::Fired;
                }
            }
        }
    }

    fn record(&mut self, index: usize, r: Result<JsonValue, String>) {
        let failed = r.is_err();
        self.out[index] = Out::Ran(r);
        if failed && self.opts.stop_on_error && self.stopped.is_none() {
            // what would have fired after it is skipped; what fired before it (and is only waiting
            // to be recorded) is not
            self.stopped = Some(index);
            let key = (self.commands[index].delay, index);
            for i in 0..self.out.len() {
                let later = (self.commands[i].delay, i);
                let after = later.0.total_cmp(&key.0).then(later.1.cmp(&key.1)).is_gt();
                if matches!(self.out[i], Out::Waiting | Out::Fired) && after {
                    self.out[i] = Out::Skipped(format!("stopped: command {index} failed"));
                }
            }
        }
        self.settle();
    }

    fn settle(&mut self) {
        if self.state != State::Cancelled && self.out.iter().all(|o| !matches!(o, Out::Waiting | Out::Fired)) {
            self.state = State::Done;
        }
    }

    fn finished(&self) -> bool {
        matches!(self.state, State::Done | State::Cancelled)
    }

    fn ran(&self) -> usize {
        self.out.iter().filter(|o| matches!(o, Out::Ran(_))).count()
    }

    fn failed(&self) -> usize {
        self.out.iter().filter(|o| matches!(o, Out::Ran(Err(_)))).count()
    }

    /// When it lands, in words.
    fn lands(&self) -> String {
        match &self.wait {
            Wait::Landed => "landed".into(),
            Wait::Lines { left, unit, deck, .. } => {
                let u = if *unit == 1.0 { "beat" } else { "bar" };
                format!("on deck {deck}'s {} {u} line{}", left, if *left == 1 { "" } else { "s" })
            }
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn to_json(&self, full: bool) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("id", JsonValue::str(&self.id));
        o.set("state", self.state.name().into());
        if let Some(l) = &self.opts.label {
            o.set("label", JsonValue::str(l));
        }
        o.set("at", self.opts.at.text().into());
        o.set("atomic", self.opts.atomic.into());
        o.set("stop_on_error", self.opts.stop_on_error.into());
        o.set("commands", JsonValue::int(self.commands.len() as i64));
        o.set("ran", JsonValue::int(self.ran() as i64));
        o.set("failed", JsonValue::int(self.failed() as i64));
        if self.state == State::Done {
            o.set("ok", (self.failed() == 0 && self.stopped.is_none()).into());
        }
        if self.state == State::Pending {
            o.set("waiting_for", self.wait.text());
            o.set("lands", JsonValue::str(self.lands()));
        }
        if let Some(n) = &self.note {
            o.set("note", JsonValue::str(n));
        }
        if let Some(i) = self.stopped {
            o.set("stopped_at", JsonValue::int(i as i64));
        }
        if full {
            let rows = self
                .commands
                .iter()
                .zip(&self.out)
                .map(|(c, out)| {
                    let mut r = JsonObject::new();
                    r.set("index", JsonValue::int(c.index as i64));
                    r.set("method", JsonValue::str(c.name()));
                    if c.delay > 0.0 {
                        r.set("delay", num(c.delay));
                    }
                    match out {
                        Out::Waiting | Out::Fired => r.set("status", "waiting".into()),
                        Out::Skipped(why) => {
                            r.set("status", "skipped".into());
                            r.set("skipped", JsonValue::str(why));
                        }
                        Out::Ran(Ok(v)) => {
                            r.set("status", "ok".into());
                            r.set("ok", true.into());
                            r.set("result", v.clone());
                        }
                        Out::Ran(Err(e)) => {
                            r.set("status", "error".into());
                            r.set("ok", false.into());
                            r.set("error", JsonValue::str(e));
                        }
                    }
                    JsonValue::Object(r)
                })
                .collect();
            o.set("results", JsonValue::Array(rows));
        }
        JsonValue::Object(o)
    }
}

/// Every batch: those waiting or running, and the last few that finished.
#[derive(Debug, Default)]
pub struct Batches {
    next_id: u64,
    live: Vec<Batch>,
    done: Vec<Batch>,
    /// A fixed clock the window uses instead of the decks' (tests, a headless run with no deck).
    pub fixed: Option<Clock>,
}

impl Batches {
    pub fn new() -> Batches {
        Batches::default()
    }

    /// Something is waiting or running.
    pub fn busy(&self) -> bool {
        !self.live.is_empty()
    }

    /// Take a batch. An `atomic` one with an invalid command is refused whole. It is not run
    /// here: [`Batches::service`] fires what is due (so `now` is due at once).
    pub fn submit(&mut self, plan: Plan, clock: &Clock) -> Result<String, String> {
        if plan.opts.atomic {
            let bad = plan.invalid();
            if !bad.is_empty() {
                return Err(format!(
                    "batch: atomic, and not run: {}",
                    bad.iter().map(|c| format!("command {}: {}", c.index, c.error.as_deref().unwrap_or(""))).collect::<Vec<_>>().join("; ")
                ));
            }
        }
        if self.live.len() >= MAX_LIVE {
            return Err(format!("batch: {MAX_LIVE} batches are already waiting: cancel one (batch status lists them)"));
        }
        let (wait, note) = Wait::new(plan.opts.at, plan.opts.deck.as_deref(), clock)?;
        self.next_id += 1;
        let id = format!("b{}", self.next_id);
        let n = plan.commands.len();
        self.live.push(Batch {
            id: id.clone(),
            opts: plan.opts,
            commands: plan.commands,
            out: vec![Out::Waiting; n],
            state: if wait == Wait::Landed { State::Running } else { State::Pending },
            wait,
            elapsed: 0.0,
            last_time: Some(clock.time),
            submitted: clock.time,
            note,
            stopped: None,
        });
        Ok(id)
    }

    /// One pass: advance every batch to `clock` and answer the commands that are due.
    pub fn service(&mut self, clock: &Clock) -> Vec<Fire> {
        let mut fires = Vec::new();
        for b in &mut self.live {
            b.advance(clock);
            b.collect(&mut fires);
        }
        self.retire();
        fires
    }

    /// Whether command `index` of `batch` is still to be run: a failure before it (with
    /// `stop_on_error`) or a cancel skips what was already handed out but not yet run.
    pub fn wants(&self, batch: &str, index: usize) -> bool {
        self.live.iter().any(|b| b.id == batch && b.state != State::Cancelled && b.out.get(index) == Some(&Out::Fired))
    }

    /// What a fired command answered.
    pub fn record(&mut self, batch: &str, index: usize, r: Result<JsonValue, String>) {
        if let Some(b) = self.live.iter_mut().find(|b| b.id == batch) {
            if b.out.get(index) == Some(&Out::Fired) {
                b.record(index, r);
            }
        }
        self.retire();
    }

    fn retire(&mut self) {
        let mut i = 0;
        while i < self.live.len() {
            if self.live[i].finished() {
                let b = self.live.remove(i);
                self.done.push(b);
                if self.done.len() > KEEP_DONE {
                    self.done.remove(0);
                }
            } else {
                i += 1;
            }
        }
    }

    fn find(&self, id: &str) -> Option<&Batch> {
        self.live.iter().chain(self.done.iter()).find(|b| b.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&Batch> {
        self.find(id)
    }

    /// The batch with every command's result.
    pub fn json(&self, id: &str) -> Option<JsonValue> {
        self.find(id).map(|b| b.to_json(true))
    }

    /// `status`: one batch in full, or every one in a line.
    pub fn status(&self, id: Option<&str>) -> Result<JsonValue, String> {
        match id {
            Some(id) => self.json(id).ok_or_else(|| self.unknown(id)),
            None => Ok(obj([
                ("waiting", JsonValue::int(self.live.len() as i64)),
                ("batches", JsonValue::Array(self.done.iter().chain(&self.live).map(|b| b.to_json(false)).collect())),
            ])),
        }
    }

    fn unknown(&self, id: &str) -> String {
        let ids: Vec<&str> = self.live.iter().chain(self.done.iter()).map(|b| b.id.as_str()).collect();
        if ids.is_empty() {
            format!("batch: no batch {id:?}: none has been made yet")
        } else {
            format!("batch: no batch {id:?}; known: {}", ids.join(", "))
        }
    }

    /// Cancel a batch that is waiting or running: what has not run, does not. Answers it in full.
    pub fn cancel(&mut self, id: &str) -> Result<JsonValue, String> {
        let Some(b) = self.live.iter_mut().find(|b| b.id == id) else {
            return match self.done.iter().find(|b| b.id == id) {
                Some(b) => Err(format!("batch: {id} has already finished ({})", b.state.name())),
                None => Err(self.unknown(id)),
            };
        };
        for o in b.out.iter_mut().filter(|o| matches!(o, Out::Waiting | Out::Fired)) {
            *o = Out::Skipped("cancelled".into());
        }
        b.state = State::Cancelled;
        self.retire();
        self.json(id).ok_or_else(|| self.unknown(id))
    }
}

// ---- dry run --------------------------------------------------------------------------------

/// Scalar values a state answer holds under the key `name`, found down to a few levels.
fn find_key<'a>(v: &'a JsonValue, name: &str, depth: usize) -> Option<&'a JsonValue> {
    match v {
        JsonValue::Object(o) => {
            if let Some(x) = o.get(name).filter(|x| !matches!(x, JsonValue::Object(_) | JsonValue::Array(_))) {
                return Some(x);
            }
            if depth == 0 {
                return None;
            }
            o.iter().find_map(|(_, x)| find_key(x, name, depth - 1))
        }
        JsonValue::Array(a) if depth > 0 => a.iter().find_map(|x| find_key(x, name, depth - 1)),
        _ => None,
    }
}

fn same(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Number(x), JsonValue::Number(y)) => (x - y).abs() < 1e-3,
        (JsonValue::Bool(x), JsonValue::Bool(y)) => x == y,
        (JsonValue::String(x), JsonValue::String(y)) => x.eq_ignore_ascii_case(y),
        _ => a.js_stringified() == b.js_stringified(),
    }
}

/// What a command's params would change in `before` (the noun's state now): the params whose
/// name is a scalar key of that state and whose value differs. Best effort: a param the state
/// does not hold by that name is not listed.
pub fn changes(before: &JsonValue, params: &JsonValue) -> Vec<JsonValue> {
    let Some(p) = params.as_object() else { return Vec::new() };
    p.iter()
        .filter(|(k, _)| !SELECTORS.contains(k))
        .filter_map(|(k, to)| {
            let from = find_key(before, k, 3)?;
            (!same(from, to)).then(|| obj([("param", JsonValue::str(k)), ("from", from.clone()), ("to", to.clone())]))
        })
        .collect()
}

impl Plan {
    /// A dry run's answer: whether it would run, when it would land, and each command's verdict,
    /// effect and (via `before`, the noun's state now) what it would change. Runs nothing.
    pub fn dry_run_json(&self, clock: &Clock, before: &mut dyn FnMut(&Command) -> Option<JsonValue>) -> JsonValue {
        let landing = Wait::new(self.opts.at, self.opts.deck.as_deref(), clock);
        let rows: Vec<JsonValue> = self
            .commands
            .iter()
            .map(|c| {
                let mut r = JsonObject::new();
                r.set("index", JsonValue::int(c.index as i64));
                if let Some(e) = &c.error {
                    r.set("valid", false.into());
                    r.set("method", JsonValue::str(c.name()));
                    r.set("error", JsonValue::str(e));
                    return JsonValue::Object(r);
                }
                r.set("valid", true.into());
                r.set("method", JsonValue::str(c.name()));
                r.set("effect", c.effect.name().into());
                if c.delay > 0.0 {
                    r.set("delay", num(c.delay));
                }
                if let Some(a) = reg::noun(&c.noun).and_then(|n| n.action(&c.action)) {
                    r.set("doc", a.doc.into());
                }
                if c.effect != Effect::Read {
                    let ch = before(c).map(|b| changes(&b, &c.params));
                    if let Some(ch) = ch {
                        r.set("changes", JsonValue::Array(ch));
                    }
                }
                JsonValue::Object(r)
            })
            .collect();
        let invalid = self.invalid().len();
        let landing_err = landing.as_ref().err().cloned();
        let valid = invalid == 0 && landing_err.is_none();
        let would_run = if self.opts.atomic {
            if valid { self.commands.len() } else { 0 }
        } else if landing_err.is_some() {
            0
        } else if self.opts.stop_on_error {
            self.commands.iter().position(|c| c.error.is_some()).map_or(self.commands.len(), |i| i + 1)
        } else {
            self.commands.len()
        };
        let mut o = JsonObject::new();
        o.set("dry_run", true.into());
        o.set("valid", valid.into());
        o.set("would_run", JsonValue::int(would_run as i64));
        o.set("atomic", self.opts.atomic.into());
        o.set("stop_on_error", self.opts.stop_on_error.into());
        o.set("at", self.opts.at.text().into());
        match landing {
            Ok((w, note)) => {
                let lands = match &w {
                    Wait::Landed => note.unwrap_or_else(|| "at once".into()),
                    Wait::Lines { left, unit, deck, .. } => format!(
                        "on deck {deck}'s {left} {} line{} from now",
                        if *unit == 1.0 { "beat" } else { "bar" },
                        if *left == 1 { "" } else { "s" }
                    ),
                };
                o.set("lands", lands.into());
            }
            Err(e) => o.set("landing_error", e.into()),
        }
        o.set("live", self.commands.iter().any(|c| c.effect == Effect::Live && c.error.is_none()).into());
        o.set("commands", JsonValue::Array(rows));
        o.set(
            "note",
            "nothing ran. `changes` compares each command's params with its noun's state now, best effort; a command that depends on an earlier one in the batch is compared with the state before the batch."
                .into(),
        );
        JsonValue::Object(o)
    }
}

// ---- tests ----------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn j(s: &str) -> JsonValue {
        JsonValue::parse(s).unwrap()
    }

    fn plan(s: &str) -> Plan {
        Plan::parse(&j(s)).unwrap()
    }

    /// A fake clock: deck A plays at 120 BPM, beat `beat`, `t` seconds in.
    fn at(t: f64, beat: f64) -> Clock {
        Clock { time: t, grids: vec![("A".into(), Grid { beat, bpm: 120.0, playing: true })] }
    }

    fn silent(t: f64) -> Clock {
        Clock { time: t, grids: vec![("A".into(), Grid { beat: 0.0, bpm: 120.0, playing: false })] }
    }

    const PLAY: &str = r#"{"method":"deck","params":{"action":"play","deck":"A"}}"#;
    const MUTE: &str = r#"{"method":"mixer","params":{"deck":"A","mute":true}}"#;

    fn ok(v: &str) -> Result<JsonValue, String> {
        Ok(j(v))
    }

    #[test]
    fn a_command_reads_in_every_form_the_language_takes() {
        // method + params, params beside the method, dotted, dotted with params, an id (JSON-RPC)
        let forms = [
            r#"{"method":"deck","params":{"action":"play","deck":"A"}}"#,
            r#"{"method":"deck","action":"play","deck":"A"}"#,
            r#"{"method":"deck.play","params":{"deck":"A"}}"#,
            r#"{"id":7,"jsonrpc":"2.0","method":"deck.play","deck":"A"}"#,
        ];
        for f in forms {
            let c = Command::parse(0, &j(f));
            assert_eq!(c.error, None, "{f}");
            assert_eq!((c.noun.as_str(), c.action.as_str()), ("deck", "play"), "{f}");
            assert_eq!(c.params.get("deck").and_then(JsonValue::as_str), Some("A"), "{f}");
            assert_eq!(c.params.get("action").and_then(JsonValue::as_str), Some("play"), "{f}");
            assert!(c.params.get("id").is_none() && c.params.get("method").is_none(), "{f}");
            assert_eq!(c.effect, Effect::Write);
            assert_eq!(c.name(), "deck.play");
        }
        // an explicit action beats the dotted verb, as on the socket
        let c = Command::parse(0, &j(r#"{"method":"deck.play","params":{"action":"pause"}}"#));
        assert_eq!(c.action, "pause");
        // a noun with no verbs; an alias; the default verb
        let c = Command::parse(0, &j(r#"{"method":"mixer","crossfader":0.4}"#));
        assert_eq!((c.error.as_deref(), c.action.as_str(), c.effect), (None, "", Effect::Write));
        assert_eq!(Command::parse(0, &j(r#"{"method":"mixer"}"#)).effect, Effect::Read);
        let c = Command::parse(0, &j(r#"{"method":"help"}"#));
        assert_eq!((c.noun.as_str(), c.error.as_deref()), ("describe", None));
        let c = Command::parse(0, &j(r#"{"method":"loopsets"}"#));
        assert_eq!((c.action.as_str(), c.effect), ("status", Effect::Read));
    }

    #[test]
    fn a_command_that_the_registry_does_not_know_says_why() {
        let bad = |s: &str| Command::parse(3, &j(s)).error.unwrap_or_else(|| panic!("{s} should be invalid"));
        assert!(bad(r#"{"method":"nope"}"#).contains("unknown method") && bad(r#"{"method":"nope"}"#).contains("deck"));
        let e = bad(r#"{"method":"deck.fly"}"#);
        assert!(e.contains("unknown action \"fly\"") && e.contains("one of") && e.contains("play"), "{e}");
        let e = bad(r#"{"method":"loopsets.load_pair"}"#);
        assert!(e.contains("needs \"group\""), "{e}");
        assert!(bad(r#"{"method":"batch","params":{"commands":[]}}"#).contains("cannot hold a batch"));
        assert!(bad(r#"{"method":"render"}"#).contains("cannot be batched"));
        assert!(bad(r#"{"method":"post","action":"process"}"#).contains("file actions"));
        assert!(bad(r#"{"params":{}}"#).contains("no method"));
        assert!(bad(r#"["deck"]"#).contains("expected"));
        assert!(bad(r#"{"method":"deck","params":[1]}"#).contains("params must be an object"));
        assert!(bad(r#"{"method":"deck.play","delay":"soon"}"#).contains("delay"));
        // a verb with a bare noun that has verbs and no default
        assert_eq!(Command::parse(3, &j(r#"{"method":"deck"}"#)).error, None, "deck defaults to a verb");
    }

    #[test]
    fn delays_are_beats_or_bars_and_short() {
        let d = |v: &str| parse_delay(&j(v));
        assert_eq!(d("2"), Ok(2.0));
        assert_eq!(d(r#""1 bar""#), Ok(4.0));
        assert_eq!(d(r#""0.5 bars""#), Ok(2.0));
        assert_eq!(d(r#""3 beats""#), Ok(3.0));
        assert_eq!(d(r#""3""#), Ok(3.0));
        assert!(d("-1").is_err() && d(r#""2 fortnights""#).is_err() && d("true").is_err());
        let e = d("17").unwrap_err();
        assert!(e.contains("set list") && e.contains("live-code"), "{e}");
    }

    #[test]
    fn at_reads_the_grid_words() {
        for (s, want) in [
            ("now", At::Now),
            ("", At::Now),
            ("next_beat", At::NextBeat),
            ("next-bar", At::NextBar),
            ("NEXT_BAR", At::NextBar),
            ("next_phrase", At::NextPhrase),
            ("bar:32", At::Bar(32)),
        ] {
            assert_eq!(At::parse(s), Ok(want), "{s}");
        }
        assert!(At::parse("bar:-1").is_err() && At::parse("bar:x").is_err() && At::parse("later").is_err());
        assert_eq!(At::Bar(8).text(), "bar:8");
    }

    #[test]
    fn the_request_is_read_with_its_options() {
        let p = plan(&format!(r#"{{"commands":[{PLAY},{MUTE}],"at":"next_bar","label":"drop"}}"#));
        assert_eq!((p.opts.atomic, p.opts.stop_on_error, p.opts.at), (false, true, At::NextBar), "stop_on_error is on unless atomic");
        assert_eq!(p.opts.label.as_deref(), Some("drop"));
        let p = plan(&format!(r#"{{"commands":[{PLAY}],"atomic":true}}"#));
        assert_eq!((p.opts.atomic, p.opts.stop_on_error), (true, false));
        let p = plan(&format!(r#"{{"commands":[{PLAY}],"atomic":true,"stop_on_error":true,"dry_run":"yes","deck":"b"}}"#));
        assert_eq!((p.opts.stop_on_error, p.opts.dry_run, p.opts.deck.as_deref()), (true, true, Some("B")));
        let e = |s: &str| Plan::parse(&j(s)).unwrap_err();
        assert!(e("{}").contains("needs \"commands\""));
        assert!(e(r#"{"commands":[]}"#).contains("empty"));
        assert!(e(r#"{"commands":"x"}"#).contains("array"));
        assert!(e(&format!(r#"{{"commands":[{PLAY}],"at":"later"}}"#)).contains("at:"));
        assert!(e(&format!(r#"{{"commands":[{PLAY}],"atomic":"maybe"}}"#)).contains("true or false"));
        let many = vec![PLAY; MAX_COMMANDS + 1].join(",");
        assert!(e(&format!(r#"{{"commands":[{many}]}}"#)).contains("over the limit"));
        // atomic lands all at once: a delay is refused
        let e = e(r#"{"commands":[{"method":"deck.play","delay":1}],"atomic":true}"#);
        assert!(e.contains("atomic") && e.contains("delay"), "{e}");
    }

    #[test]
    fn now_is_due_on_the_first_pass() {
        let mut b = Batches::new();
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY},{MUTE}]}}"#)), &at(0.0, 10.0)).unwrap();
        assert_eq!(id, "b1");
        let fires = b.service(&at(0.016, 10.03));
        assert_eq!(fires.iter().map(|f| (f.index, f.noun.as_str())).collect::<Vec<_>>(), vec![(0, "deck"), (1, "mixer")]);
        assert!(b.wants(&id, 0) && b.wants(&id, 1));
        b.record(&id, 0, ok(r#"{"playing":true}"#));
        assert_eq!(b.get(&id).unwrap().state(), State::Running);
        b.record(&id, 1, ok(r#"{"mute":true}"#));
        let done = b.json(&id).unwrap();
        assert_eq!(done.get("state").and_then(JsonValue::as_str), Some("done"));
        assert_eq!(done.get("ok").and_then(JsonValue::as_bool), Some(true));
        let rows = done.get("results").and_then(JsonValue::as_array).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("method").and_then(JsonValue::as_str), Some("deck.play"));
        assert_eq!(rows[0].get("status").and_then(JsonValue::as_str), Some("ok"));
        assert_eq!(rows[1].get("result").and_then(|r| r.get("mute")).and_then(JsonValue::as_bool), Some(true));
        assert!(!b.busy(), "a finished batch is not live");
        assert!(b.service(&at(0.1, 10.2)).is_empty());
    }

    /// The batch waits for the line on a fake clock and lands on the pass that crosses it.
    #[test]
    fn next_bar_lands_on_the_line_and_not_before() {
        let mut b = Batches::new();
        // beat 10.5 is in bar 2 (beats 8..12): the next line is beat 12
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY},{MUTE}],"at":"next_bar"}}"#)), &at(0.0, 10.5)).unwrap();
        let j0 = b.json(&id).unwrap();
        assert_eq!(j0.get("state").and_then(JsonValue::as_str), Some("pending"));
        assert_eq!(j0.get("waiting_for").and_then(|w| w.get("lines_left")).and_then(JsonValue::as_i64), Some(1));
        assert!(j0.get("lands").and_then(JsonValue::as_str).unwrap().contains("deck A"));
        // 16 ms passes are 0.032 beats at 120 BPM; walk the clock a pass at a time
        let mut beat = 10.5;
        let mut t = 0.0;
        let mut landed = None;
        while landed.is_none() {
            t += 0.016;
            beat += 0.016 * 2.0;
            let fires = b.service(&at(t, beat));
            if !fires.is_empty() {
                landed = Some((beat, fires));
            } else {
                assert!(beat < 12.0, "nothing fired before the line: {beat}");
            }
        }
        let (beat, fires) = landed.unwrap();
        assert!((12.0..12.04).contains(&beat), "landed on the first pass past beat 12, at {beat}");
        assert_eq!(fires.len(), 2, "the whole batch lands together");
        assert_eq!(b.get(&id).unwrap().state(), State::Running);
    }

    #[test]
    fn beat_phrase_and_bar_n_count_their_lines() {
        let lines_left = |at: &str, beat: f64| -> Result<i64, String> {
            let mut b = Batches::new();
            let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"{at}"}}"#)), &at_clock(beat))?;
            Ok(b.json(&id).unwrap().get("waiting_for").and_then(|w| w.get("lines_left")).and_then(JsonValue::as_i64).unwrap())
        };
        fn at_clock(beat: f64) -> Clock {
            at(0.0, beat)
        }
        assert_eq!(lines_left("next_beat", 10.5), Ok(1));
        // bar 2.5: the next 8-bar phrase is bar 8, six lines on
        assert_eq!(lines_left("next_phrase", 10.0), Ok(6));
        // on bar 8 itself the next phrase is bar 16
        assert_eq!(lines_left("next_phrase", 32.0), Ok(8));
        assert_eq!(lines_left("bar:5", 10.0), Ok(3));
        let e = lines_left("bar:2", 10.0).unwrap_err();
        assert!(e.contains("already begun") && e.contains("bar 2"), "{e}");
        assert!(lines_left("bar:5000", 10.0).unwrap_err().contains("away"));
        // a beat line is one beat on
        let mut b = Batches::new();
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_beat"}}"#)), &at(0.0, 10.5)).unwrap();
        assert!(b.service(&at(0.1, 10.9)).is_empty());
        assert_eq!(b.service(&at(0.2, 11.05)).len(), 1);
        assert!(b.get(&id).is_some());
        // the third bar line of three
        let mut b = Batches::new();
        b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"bar:5"}}"#)), &at(0.0, 10.0)).unwrap();
        assert!(b.service(&at(1.0, 12.1)).is_empty(), "bar 3");
        assert!(b.service(&at(2.0, 16.1)).is_empty(), "bar 4");
        assert_eq!(b.service(&at(3.0, 20.1)).len(), 1, "bar 5");
    }

    #[test]
    fn a_loop_wrap_is_a_line_and_a_jitter_is_not() {
        let mut b = Batches::new();
        b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_bar"}}"#)), &at(0.0, 11.9)).unwrap();
        // the platter steps back a hair: no line
        assert!(b.service(&at(0.01, 11.8)).is_empty());
        // the deck's loop wrapped to its top: a line, as auto-next and Auto-DJ count it
        assert_eq!(b.service(&at(0.5, 0.2)).len(), 1);
    }

    #[test]
    fn with_no_grid_a_timed_batch_lands_at_once_and_says_so() {
        let mut b = Batches::new();
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_bar"}}"#)), &silent(0.0)).unwrap();
        let j = b.json(&id).unwrap();
        assert_eq!(j.get("state").and_then(JsonValue::as_str), Some("running"));
        assert!(j.get("note").and_then(JsonValue::as_str).unwrap().contains("no deck is playing"));
        assert_eq!(b.service(&silent(0.016)).len(), 1);
    }

    #[test]
    fn a_batch_holds_while_nothing_plays_and_follows_the_other_deck() {
        let mut b = Batches::new();
        b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_bar"}}"#)), &at(0.0, 10.0)).unwrap();
        // A stops: the line does not come
        assert!(b.service(&silent(1.0)).is_empty());
        assert!(b.busy());
        // B plays instead, at beat 21.5 in bar 5: the count restarts from its grid (one line)
        let on_b = Clock { time: 2.0, grids: vec![("A".into(), Grid { beat: 0.0, bpm: 120.0, playing: false }), ("B".into(), Grid { beat: 21.5, bpm: 120.0, playing: true })] };
        assert!(b.service(&on_b).is_empty());
        let later = Clock { time: 3.0, grids: vec![on_b.grids[0].clone(), ("B".into(), Grid { beat: 24.1, bpm: 120.0, playing: true })] };
        assert_eq!(b.service(&later).len(), 1);
    }

    #[test]
    fn a_preferred_deck_times_the_batch() {
        let two = |a: f64, b: f64, t: f64| Clock {
            time: t,
            grids: vec![("A".into(), Grid { beat: a, bpm: 120.0, playing: true }), ("B".into(), Grid { beat: b, bpm: 120.0, playing: true })],
        };
        let mut s = Batches::new();
        // deck B is half a bar behind A: it crosses its own line later
        s.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_bar","deck":"B"}}"#)), &two(11.0, 9.0, 0.0)).unwrap();
        assert!(s.service(&two(12.1, 10.1, 0.5)).is_empty(), "A's line is not the batch's");
        assert_eq!(s.service(&two(14.0, 12.1, 1.0)).len(), 1);
    }

    #[test]
    fn delays_count_beats_from_the_landing() {
        let mut b = Batches::new();
        let id = b
            .submit(
                plan(
                    r#"{"commands":[
                        {"method":"inserts.set","params":{"param":"filter.sweep","value":0.3}},
                        {"method":"inserts.set","params":{"param":"filter.sweep","value":0.6},"delay":1},
                        {"method":"inserts.set","params":{"param":"filter.sweep","value":0.9},"delay":"1 bar"}
                    ],"at":"next_bar"}"#,
                ),
                &at(0.0, 11.9),
            )
            .unwrap();
        // lands at beat 12.0x; the first step goes at once
        let f = b.service(&at(0.1, 12.2));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![0]);
        b.record(&id, 0, ok("{}"));
        assert_eq!(b.get(&id).unwrap().state(), State::Running);
        // 120 BPM: a beat is half a second. 0.4 s later: not yet
        assert!(b.service(&at(0.5, 13.0)).is_empty());
        let f = b.service(&at(0.62, 13.2));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![1], "one beat after the landing");
        b.record(&id, 1, ok("{}"));
        assert!(b.service(&at(1.5, 15.0)).is_empty());
        let f = b.service(&at(2.2, 16.4));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![2], "a bar after the landing");
        b.record(&id, 2, ok("{}"));
        assert_eq!(b.json(&id).unwrap().get("state").and_then(JsonValue::as_str), Some("done"));
    }

    #[test]
    fn delayed_commands_fire_in_delay_order_then_place() {
        let mut b = Batches::new();
        b.submit(
            plan(
                r#"{"commands":[
                    {"method":"deck.play","delay":2},
                    {"method":"deck.pause"},
                    {"method":"deck.stop","delay":1},
                    {"method":"deck.cue"}
                ]}"#,
            ),
            &at(0.0, 0.0),
        )
        .unwrap();
        let f = b.service(&at(0.1, 0.2));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![1, 3]);
        let f = b.service(&at(1.2, 2.4));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![2, 0], "the 1-beat step, then the 2-beat one, if both are due");
    }

    #[test]
    fn stop_on_error_skips_the_rest_and_off_runs_on() {
        for (stop, want_third) in [(true, "skipped"), (false, "ok")] {
            let mut b = Batches::new();
            let id = b
                .submit(plan(&format!(r#"{{"commands":[{PLAY},{MUTE},{PLAY}],"stop_on_error":{stop}}}"#)), &at(0.0, 0.0))
                .unwrap();
            let f = b.service(&at(0.1, 0.2));
            assert_eq!(f.len(), 3);
            b.record(&id, 0, ok("{}"));
            b.record(&id, 1, Err("mixer: no deck Q".into()));
            assert_eq!(b.wants(&id, 2), !stop, "a stopped batch wants no more of its fires");
            if !stop {
                b.record(&id, 2, ok("{}"));
            }
            let j = b.json(&id).unwrap();
            assert_eq!(j.get("state").and_then(JsonValue::as_str), Some("done"));
            assert_eq!(j.get("ok").and_then(JsonValue::as_bool), Some(false));
            assert_eq!(j.get("failed").and_then(JsonValue::as_i64), Some(1));
            let rows = j.get("results").and_then(JsonValue::as_array).unwrap();
            assert_eq!(rows[1].get("error").and_then(JsonValue::as_str), Some("mixer: no deck Q"));
            assert_eq!(rows[2].get("status").and_then(JsonValue::as_str), Some(want_third));
            if stop {
                assert_eq!(j.get("stopped_at").and_then(JsonValue::as_i64), Some(1));
                assert!(rows[2].get("skipped").and_then(JsonValue::as_str).unwrap().contains("command 1"));
            }
        }
    }

    /// An invalid command in a non-atomic batch is a failed command at its turn, never sent.
    #[test]
    fn an_invalid_command_fails_in_its_turn_without_reaching_the_app() {
        let mut b = Batches::new();
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY},{{"method":"nope"}},{MUTE}]}}"#)), &at(0.0, 0.0)).unwrap();
        let f = b.service(&at(0.1, 0.2));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![0], "the invalid one stops it before the third");
        b.record(&id, 0, ok("{}"));
        let j = b.json(&id).unwrap();
        assert_eq!(j.get("state").and_then(JsonValue::as_str), Some("done"));
        let rows = j.get("results").and_then(JsonValue::as_array).unwrap();
        assert!(rows[1].get("error").and_then(JsonValue::as_str).unwrap().contains("unknown method"));
        assert_eq!(rows[2].get("status").and_then(JsonValue::as_str), Some("skipped"));
    }

    #[test]
    fn atomic_refuses_the_whole_batch_for_one_invalid_command() {
        let mut b = Batches::new();
        let e = b
            .submit(plan(&format!(r#"{{"commands":[{PLAY},{{"method":"deck.fly"}},{{"method":"loopsets.load_pair"}}],"atomic":true}}"#)), &at(0.0, 0.0))
            .unwrap_err();
        assert!(e.contains("not run") && e.contains("command 1") && e.contains("command 2"), "{e}");
        assert!(!b.busy(), "nothing was taken");
        assert!(b.service(&at(0.1, 0.2)).is_empty());
        // all valid: every command is due on the one pass, in order
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY},{MUTE},{PLAY}],"atomic":true}}"#)), &at(0.0, 0.0)).unwrap();
        let f = b.service(&at(0.016, 0.03));
        assert_eq!(f.iter().map(|f| f.index).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(f.iter().all(|f| f.batch == id));
    }

    #[test]
    fn cancel_stops_what_has_not_run() {
        let mut b = Batches::new();
        let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY},{MUTE}],"at":"next_phrase"}}"#)), &at(0.0, 10.0)).unwrap();
        assert!(b.service(&at(0.5, 14.0)).is_empty());
        let j = b.cancel(&id).unwrap();
        assert_eq!(j.get("state").and_then(JsonValue::as_str), Some("cancelled"));
        let rows = j.get("results").and_then(JsonValue::as_array).unwrap();
        assert!(rows.iter().all(|r| r.get("skipped").and_then(JsonValue::as_str) == Some("cancelled")));
        assert!(!b.busy());
        // it never fires, however far the clock goes
        assert!(b.service(&at(30.0, 64.0)).is_empty());
        assert!(!b.wants(&id, 0));
        // and it cannot be cancelled twice, nor an unknown one
        assert!(b.cancel(&id).unwrap_err().contains("already finished"));
        assert!(b.cancel("b99").unwrap_err().contains("known: b1"));
        // the finished one is still readable
        assert_eq!(b.status(Some(&id)).unwrap().get("state").and_then(JsonValue::as_str), Some("cancelled"));
        assert!(b.status(Some("zzz")).is_err());
    }

    #[test]
    fn cancel_between_the_steps_of_a_running_batch() {
        let mut b = Batches::new();
        let id = b
            .submit(plan(r#"{"commands":[{"method":"deck.play"},{"method":"deck.pause","delay":2}]}"#), &at(0.0, 0.0))
            .unwrap();
        let f = b.service(&at(0.1, 0.2));
        assert_eq!(f.len(), 1);
        b.record(&id, 0, ok("{}"));
        let j = b.cancel(&id).unwrap();
        assert_eq!(j.get("ran").and_then(JsonValue::as_i64), Some(1));
        let rows = j.get("results").and_then(JsonValue::as_array).unwrap();
        assert_eq!(rows[0].get("status").and_then(JsonValue::as_str), Some("ok"));
        assert_eq!(rows[1].get("status").and_then(JsonValue::as_str), Some("skipped"));
        assert!(b.service(&at(5.0, 10.0)).is_empty());
    }

    #[test]
    fn status_lists_batches_and_keeps_the_last_few() {
        let mut b = Batches::new();
        assert_eq!(b.status(None).unwrap().get("waiting").and_then(JsonValue::as_i64), Some(0));
        assert!(b.status(Some("b1")).unwrap_err().contains("none has been made"));
        for i in 0..(KEEP_DONE + 3) {
            let id = b.submit(plan(&format!(r#"{{"commands":[{PLAY}]}}"#)), &at(i as f64, 0.0)).unwrap();
            let f = b.service(&at(i as f64 + 0.1, 0.2));
            b.record(&id, f[0].index, ok("{}"));
        }
        let all = b.status(None).unwrap();
        assert_eq!(all.get("batches").and_then(JsonValue::as_array).unwrap().len(), KEEP_DONE);
        assert!(b.status(Some("b1")).is_err(), "the oldest is forgotten");
        assert!(b.status(Some(&format!("b{}", KEEP_DONE + 3))).is_ok());
        // waiting ones are bounded
        let mut b = Batches::new();
        for _ in 0..MAX_LIVE {
            b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_phrase"}}"#)), &at(0.0, 1.0)).unwrap();
        }
        assert!(b.submit(plan(&format!(r#"{{"commands":[{PLAY}],"at":"next_phrase"}}"#)), &at(0.0, 1.0)).unwrap_err().contains("already waiting"));
    }

    #[test]
    fn a_dry_run_validates_and_reports_what_would_change() {
        let p = plan(&format!(
            r#"{{"commands":[{MUTE},{{"method":"deck.fly"}},{{"method":"mixer","params":{{"crossfader":0.9}}}},{{"method":"mixer"}}],"at":"next_bar","dry_run":true}}"#
        ));
        let mut asked = Vec::new();
        let r = p.dry_run_json(&at(0.0, 10.0), &mut |c| {
            asked.push(c.index);
            Some(j(r#"{"crossfader":0.5,"decks":[{"name":"A","mute":false}]}"#))
        });
        assert_eq!(r.get("dry_run").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(r.get("valid").and_then(JsonValue::as_bool), Some(false));
        assert_eq!(r.get("would_run").and_then(JsonValue::as_i64), Some(2), "stop_on_error: up to and with the first invalid one");
        assert!(r.get("lands").and_then(JsonValue::as_str).unwrap().contains("deck A"));
        assert_eq!(asked, vec![0, 2], "only writes ask for the state");
        let rows = r.get("commands").and_then(JsonValue::as_array).unwrap();
        // a mute that changes, found in the deck's row
        let ch = rows[0].get("changes").and_then(JsonValue::as_array).unwrap();
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].get("param").and_then(JsonValue::as_str), Some("mute"));
        assert_eq!(ch[0].get("from").and_then(JsonValue::as_bool), Some(false));
        assert_eq!(ch[0].get("to").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(rows[1].get("valid").and_then(JsonValue::as_bool), Some(false));
        assert!(rows[1].get("error").and_then(JsonValue::as_str).unwrap().contains("unknown action"));
        assert_eq!(rows[2].get("changes").and_then(JsonValue::as_array).unwrap()[0].get("from").and_then(JsonValue::as_f64), Some(0.5));
        assert!(rows[3].get("changes").is_none(), "a read changes nothing");
        assert_eq!(rows[3].get("effect").and_then(JsonValue::as_str), Some("read"));
        // atomic: one invalid and none would run
        let p = plan(&format!(r#"{{"commands":[{MUTE},{{"method":"deck.fly"}}],"atomic":true,"dry_run":true}}"#));
        let r = p.dry_run_json(&at(0.0, 10.0), &mut |_| None);
        assert_eq!(r.get("would_run").and_then(JsonValue::as_i64), Some(0));
        // a landing in the past is the dry run's verdict too
        let p = plan(&format!(r#"{{"commands":[{MUTE}],"at":"bar:1","dry_run":true}}"#));
        let r = p.dry_run_json(&at(0.0, 10.0), &mut |_| None);
        assert_eq!(r.get("valid").and_then(JsonValue::as_bool), Some(false));
        assert!(r.get("landing_error").and_then(JsonValue::as_str).unwrap().contains("already begun"));
        // all valid and atomic: all of them
        let p = plan(&format!(r#"{{"commands":[{MUTE},{PLAY}],"atomic":true,"dry_run":true}}"#));
        let r = p.dry_run_json(&at(0.0, 10.0), &mut |_| None);
        assert_eq!((r.get("valid").and_then(JsonValue::as_bool), r.get("would_run").and_then(JsonValue::as_i64)), (Some(true), Some(2)));
    }

    #[test]
    fn changes_skip_selectors_and_equal_values() {
        let before = j(r#"{"deck":"A","bpm":120.0004,"nested":{"deep":{"mute":true}},"name":"Hi"}"#);
        let p = j(r#"{"action":"tempo","deck":"B","bpm":120,"mute":true,"name":"hi","other":1}"#);
        assert!(changes(&before, &p).is_empty(), "selectors, equal numbers, equal flags, a case-only difference and unknown keys are not changes");
        let p = j(r#"{"bpm":128}"#);
        assert_eq!(changes(&before, &p).len(), 1);
    }
}
