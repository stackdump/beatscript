//! The browser engine's reading of a program's control calls, on the offline engine: what `beatscript render` (and so the
//! stream, `listen` and the stems) plays for a control line is what beatscript.app's page plays for it.
//!
//! The page (site/web/js) runs the live runtime and hands every registry call it makes to `engine-calls.js` `mapCall`,
//! which turns it into a few ops on djrust's wasm facade (`Dj`); `engine.js` sends them (a knob that has not moved is not
//! sent again; the program's master is given to the Wheel too) and `engine-worklet.js` applies them. `Dj` is a thin
//! wrapper over the very `Engine` this render steps:
//!
//! | page op (engine-calls.js)            | `Dj`                         | here (the same `Engine` call)                     |
//! |--------------------------------------|------------------------------|---------------------------------------------------|
//! | `{type:'param', deck, name, value}`  | `set_param`                  | `perform(DeckAction::Param)`                      |
//! | `{type:'perform', deck, json}`       | `perform` (`beats::perform`) | `DeckAction::from_json` with the deck, `perform`  |
//! | `{type:'crossfader', value}`         | `set_crossfader`             | `set_crossfader(v.clamp(0, 1))`                   |
//! | `{type:'play', deck, on}`            | `set_playing`                | `perform(DeckAction::Play)`                       |
//! | `{type:'live', method, params}`      | `live_op`                    | `apply_live_op`                                   |
//! | `{type:'wheel', action, params}`     | `WheelWinder` / `WheelVoice` | `beatscript_api::wheel::{Winder, WheelCore}`      |
//!
//! so where a line is played the same way, it is the same code, and the samples are the page's (up to when the call lands:
//! the page acts on a call within a pass of its 25 ms loop, a render on the call's exact frame). [`map_call`] is `mapCall`
//! line for line, and tests/browser_calls.rs holds the two to one list.
//!
//! **What the page does not play, the render still does, the booth's way** (the program says it, and the stream played it
//! before the page could): an insert the page has no op for (a delay, a crush on a deck, the filter's mix) is switched and
//! set on the strip's insert chain (`inserts` live op), as the booth's registry does. The moves are not here: the booth's
//! scheduler (`djhost::offline_moves`, which `MoveScheduler` ports) plays them on the render's clock; a `macro` line and a
//! `duck` are played by the render's control net and arrangement. [`Browser::ignored`] lists what the page itself drops.

use std::collections::{BTreeMap, BTreeSet};

use beatscript_api::wheel::{winds, Args, WheelCore, Winder};
use djengine::host::deck::engine::Engine;
use djengine::host::deck::perform::PerformCtx;
use djengine::json::{JsonObject, JsonValue};
use djengine::session::action::DeckAction;

const DECKS: [&str; 2] = ["A", "B"];
/// The insert filter's knobs the page plays (djrust `InsertParam` ids).
const FILTER_KNOBS: [&str; 2] = ["filter.sweep", "filter.res"];
/// The Wheel's actions (djhost control_registry `A_WHEEL`): the voice's and the winding half's.
const WHEEL_ACTIONS: [&str; 15] = ["load", "key", "sequence", "brightness", "play", "pause", "toggle", "motor", "level", "interval", "sync", "step", "follow", "pluck", "trigger"];
const WHEEL_READS: [&str; 2] = ["status", "presets"];
/// Every move the language has (djlive compile.rs MOVES), and the ones the page's `MoveScheduler` times.
const MOVE_KINDS: [&str; 6] = ["filter_exit", "echo_out", "loop_roll", "bass_swap", "phrase_sync", "vocoder_handoff"];
const SCHEDULED_MOVES: [&str; 5] = ["filter_exit", "echo_out", "loop_roll", "bass_swap", "phrase_sync"];
/// The AudioWorklet's render quantum: the Wheel's voice is rendered this many frames at a time, as the page renders it.
pub const QUANTUM: usize = 128;

/// One op of the page's engine (engine-calls.js `mapCall`'s list), as the worklet applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Param { deck: String, name: String, value: f64 },
    Crossfader(f64),
    /// A deck action as JSON (`{"op":"masterFx","key":"crush-bits","value":0.6}`), on `deck`.
    Perform { deck: String, json: JsonValue },
    Play { deck: String, on: bool },
    /// A djrust live op; with `phrase`, on the next phrase line (`bars` bars) of that deck.
    Live { method: String, params: JsonValue, phrase: Option<(String, u32)> },
    /// A move for the page's `MoveScheduler` (here: the booth's scheduler plays the program's move lines).
    Move { action: String, kind: Option<String>, deck: Option<String>, bars: Option<u32>, cut: bool },
    /// A `wheel` call: the params as the line gave them, with its action.
    Wheel { action: String, params: JsonValue },
}

/// What one call comes to: its ops, and what the page ignores of it (`ignoring ...`, one note per kind).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mapped {
    pub ops: Vec<Op>,
    pub ignored: Option<String>,
}

fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// JS `Number(v)` as `mapCall`'s `num` reads a param: a number, or a string that is one; NaN otherwise.
fn num(v: Option<&JsonValue>) -> f64 {
    match v {
        Some(JsonValue::Number(n)) => *n,
        Some(JsonValue::String(s)) if !s.trim().is_empty() => s.trim().parse::<f64>().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

fn text(p: &JsonObject, k: &str) -> Option<String> {
    p.get(k).and_then(JsonValue::as_str).map(str::to_string)
}

fn obj(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

fn param(deck: &str, name: &str, value: f64) -> Op {
    Op::Param { deck: deck.into(), name: name.into(), value }
}

/// `masterFx`: the master FX unit's slider `key` at `slider` (0..100) as a POSITION (0..1), on both decks' units.
fn master_fx(key: &str, slider: f64) -> Vec<Op> {
    let pos = djengine::json::js_round(slider) / 100.0;
    DECKS.iter().map(|d| Op::Perform { deck: (*d).into(), json: obj(vec![("op", "masterFx".into()), ("key", key.into()), ("value", JsonValue::Number(pos))]) }).collect()
}

fn live(method: &str, params: JsonValue) -> Op {
    Op::Live { method: method.into(), params, phrase: None }
}

fn decks_of(target: Option<&str>) -> Vec<&'static str> {
    match target {
        Some("Master") => DECKS.to_vec(),
        Some("A") => vec!["A"],
        Some("B") => vec!["B"],
        _ => Vec::new(),
    }
}

/// A truthy `on` the way `mapCall` reads it for `inserts`: `p.on !== false` (absent is on).
fn on_flag(p: &JsonObject) -> bool {
    !matches!(p.get("on"), Some(JsonValue::Bool(false)))
}

/// `moveBars`: none or 0 is the move's own default, at most 64.
fn move_bars(kind: &str, bars: f64) -> u32 {
    let def = match kind {
        "bass_swap" | "vocoder_handoff" => 8,
        "phrase_sync" => 16,
        "filter_exit" | "echo_out" => 4,
        "loop_roll" => 1,
        _ => 4,
    };
    let b = if bars.is_finite() { bars.floor().max(0.0) as u32 } else { 0 };
    (if b == 0 { def } else { b }).min(64)
}

/// `phraseAction`: what a phrase sync does on its line.
fn phrase_cut(v: Option<&JsonValue>) -> bool {
    let s = v.and_then(JsonValue::as_str).map(|s| s.trim().to_lowercase()).unwrap_or_default();
    matches!(s.as_str(), "cut" | "xfade" | "crossfader" | "transition")
}

/// engine-calls.js `mapCall`: one registry call as the page's ops, or why it ignores it.
pub fn map_call(noun: &str, params: &JsonValue) -> Mapped {
    let empty = JsonObject::new();
    let p = params.as_object().unwrap_or(&empty);
    let mut ops = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    match noun {
        "mixer" => {
            let deck = text(p, "deck");
            if p.get("crossfader").is_some() && num(p.get("crossfader")).is_finite() {
                ops.push(Op::Crossfader(clamp01(num(p.get("crossfader")))));
            }
            if p.get("master").is_some() && num(p.get("master")).is_finite() {
                ops.push(Op::Perform { deck: "A".into(), json: obj(vec![("op", "master".into()), ("value", JsonValue::Number(clamp01(num(p.get("master")))))]) });
            }
            let strip = [("volume", "volume"), ("gain", "gain"), ("filter", "filter"), ("low", "eqLow"), ("mid", "eqMid"), ("high", "eqHigh")];
            match deck.as_deref() {
                Some(d @ ("A" | "B")) => {
                    for (k, name) in strip {
                        if p.get(k).is_some() && num(p.get(k)).is_finite() {
                            ops.push(param(d, name, clamp01(num(p.get(k)))));
                        }
                    }
                    if let Some(m) = p.get("mute") {
                        let on = matches!(m, JsonValue::Bool(true)) || m.as_str() == Some("true") || m.as_f64() == Some(1.0);
                        ops.push(param(d, "mute", if on { 1.0 } else { 0.0 }));
                    }
                }
                Some(d) if strip.iter().map(|(k, _)| *k).chain(["mute"]).any(|k| p.get(k).is_some()) => skipped.push(format!("mixer deck {d}")),
                _ => {}
            }
            for k in ["pfl", "aux", "headphone_mix", "headphone_level"] {
                if p.get(k).is_some() {
                    skipped.push(format!("mixer {k}"));
                }
            }
        }
        "deck" => match text(p, "deck").as_deref() {
            Some(d @ ("A" | "B")) => match text(p, "action").as_deref() {
                Some("play") => ops.push(Op::Play { deck: d.into(), on: true }),
                Some("stop" | "pause") => ops.push(Op::Play { deck: d.into(), on: false }),
                a => skipped.push(format!("deck {}", a.unwrap_or("undefined"))),
            },
            _ => skipped.push("deck without A or B".into()),
        },
        "inserts" => {
            let target = text(p, "target");
            let decks = decks_of(target.as_deref());
            let pparam = text(p, "param");
            let effect = text(p, "effect").or_else(|| pparam.as_deref().map(|x| x.split('.').next().unwrap_or("").to_string()));
            if decks.is_empty() {
                skipped.push(format!("inserts on {}", target.as_deref().unwrap_or("undefined")));
            } else {
                let action = text(p, "action");
                let on = if action.as_deref() == Some("on") { Some(on_flag(p)) } else { None };
                let value = num(p.get("value"));
                let tg = target.clone().unwrap_or_default();
                match effect.as_deref() {
                    Some("filter") => {
                        let t = if tg == "Master" { "master".to_string() } else { tg.clone() };
                        if action.as_deref() == Some("on") {
                            ops.push(live("inserts", obj(vec![("effect", "filter".into()), ("on", JsonValue::Bool(on != Some(false))), ("target", t.into())])));
                        } else if pparam.as_deref().is_some_and(|x| FILTER_KNOBS.contains(&x)) && value.is_finite() {
                            ops.push(live("inserts", obj(vec![("param", pparam.clone().unwrap().into()), ("target", t.into()), ("value", JsonValue::Number(clamp01(value)))])));
                        } else {
                            skipped.push(format!("inserts {}", pparam.as_deref().unwrap_or("filter")));
                        }
                    }
                    Some("reverb") => {
                        if action.as_deref() == Some("on") && on == Some(false) {
                            decks.iter().for_each(|d| ops.push(param(d, "reverbWet", 0.0)));
                        } else if pparam.as_deref() == Some("reverb.mix") && value.is_finite() {
                            decks.iter().for_each(|d| ops.push(param(d, "reverbWet", clamp01(value))));
                        } else if action.as_deref() != Some("on") {
                            skipped.push(format!("inserts {}", pparam.as_deref().unwrap_or("reverb")));
                        }
                    }
                    Some("crush") if tg == "Master" => {
                        if action.as_deref() == Some("on") && on == Some(false) {
                            ops.extend(master_fx("crush-bits", 0.0));
                        } else if pparam.as_deref() == Some("crush.mix") && value.is_finite() {
                            ops.extend(master_fx("crush-bits", clamp01(value) * 100.0));
                        } else if action.as_deref() != Some("on") {
                            skipped.push(format!("inserts {}", pparam.as_deref().unwrap_or("crush")));
                        }
                    }
                    // switching an effect the page does not have off is a no-op
                    _ if action.as_deref() == Some("on") && on == Some(false) => {}
                    e => skipped.push(format!("inserts {}{}", e.or(pparam.as_deref()).unwrap_or("?"), if tg != "Master" { format!(" on {tg}") } else { String::new() })),
                }
            }
        }
        "fx" => {
            let value = num(p.get("value"));
            let deck = text(p, "deck");
            let decks: Vec<&str> = match deck.as_deref() {
                None => DECKS.to_vec(),
                Some(d @ ("A" | "B")) => vec![if d == "A" { "A" } else { "B" }],
                Some(_) => Vec::new(),
            };
            let action = text(p, "action");
            let pp = text(p, "param");
            if action.as_deref() == Some("set") && pp.as_deref() == Some("drive") && !decks.is_empty() {
                if value.is_finite() {
                    for d in decks {
                        ops.push(Op::Perform { deck: d.into(), json: obj(vec![("op", "fx".into()), ("param", "drive".into()), ("value", JsonValue::Number(clamp01(value)))]) });
                    }
                }
            } else if action.as_deref() == Some("set") && pp.is_some() {
                let on = match (&deck, decks.is_empty()) {
                    (Some(d), true) => format!(" on {d}"),
                    _ => String::new(),
                };
                skipped.push(format!("fx {}{on}", pp.unwrap()));
            } else {
                skipped.push(format!("fx {}", action.as_deref().unwrap_or("?")));
            }
        }
        "vocoder" => {
            let action = text(p, "action").unwrap_or_else(|| "set".into());
            let mut rest = JsonObject::new();
            for (k, v) in p.iter() {
                if k != "action" {
                    rest.set(k, v.clone());
                }
            }
            match action.as_str() {
                "set" | "on" => {
                    if action == "on" && rest.get("on").is_none() {
                        rest.set("on", JsonValue::Bool(true));
                    }
                    if !rest.is_empty() {
                        ops.push(live("vocoder", JsonValue::Object(rest)));
                    }
                }
                "swap" => {
                    rest.set("swap", JsonValue::Bool(true));
                    ops.push(live("vocoder", JsonValue::Object(rest)));
                }
                "add" => ops.push(live("vocoder", obj(vec![("instance", rest.get("instance").cloned().unwrap_or(JsonValue::int(1))), ("on", rest.get("on").cloned().unwrap_or(JsonValue::Bool(true)))]))),
                "remove" => ops.push(live("vocoder", obj(vec![("instance", rest.get("instance").cloned().unwrap_or(JsonValue::int(1))), ("remove", JsonValue::Bool(true))]))),
                a => skipped.push(format!("vocoder {a}")),
            }
        }
        "effects" => match (text(p, "action").as_deref(), text(p, "id")) {
            (Some("fire" | "release"), Some(id)) if !id.is_empty() => skipped.push(format!("macro {id}")),
            _ => skipped.push("effects".into()),
        },
        "moves" => {
            let deck = text(p, "deck").map(|d| d.trim().to_uppercase());
            let action = text(p, "action");
            let kind = text(p, "kind");
            let k = kind.as_deref().unwrap_or("");
            match action.as_deref() {
                Some("arm") if k == "vocoder_handoff" => {
                    let d = deck.as_deref().filter(|d| DECKS.contains(d)).unwrap_or("A");
                    ops.push(Op::Live { method: "vocoder".into(), params: obj(vec![("swap", JsonValue::Bool(true))]), phrase: Some((d.into(), 8)) });
                }
                Some("arm" | "toggle") if SCHEDULED_MOVES.contains(&k) => {
                    if !deck.as_deref().is_some_and(|d| DECKS.contains(&d)) {
                        skipped.push(format!("moves on deck {}", text(p, "deck").unwrap_or_else(|| "undefined".into())));
                    } else if k == "phrase_sync" && !phrase_cut(p.get("do")) {
                        skipped.push("moves phrase_sync seed".into());
                    } else {
                        ops.push(Op::Move { action: action.clone().unwrap(), kind: kind.clone(), deck, bars: Some(move_bars(k, num(p.get("bars")))), cut: k == "phrase_sync" });
                    }
                }
                Some("cancel") if kind.is_none() || MOVE_KINDS.contains(&k) => ops.push(Op::Move { action: "cancel".into(), kind, deck, bars: None, cut: false }),
                Some("cancel_all") => ops.push(Op::Move { action: "cancel_all".into(), kind: None, deck: None, bars: None, cut: false }),
                a => skipped.push(format!("moves {}{}", a.unwrap_or("?"), kind.map(|k| format!(" {k}")).unwrap_or_default())),
            }
        }
        "wheel" => {
            let action = text(p, "action").unwrap_or_else(|| "status".into());
            if WHEEL_ACTIONS.contains(&action.as_str()) {
                let mut o = p.clone();
                o.set("action", action.clone().into());
                ops.push(Op::Wheel { action, params: JsonValue::Object(o) });
            } else if !WHEEL_READS.contains(&action.as_str()) {
                skipped.push(format!("wheel {action}"));
            }
        }
        other => skipped.push(other.to_string()),
    }
    Mapped { ops, ignored: (!skipped.is_empty()).then(|| skipped.join(", ")) }
}

/// engine-calls.js `mapSound`: a part's `kit=` / `sound=` (`kit:NAME` is a drum kit, anything else the role's voice).
pub fn map_sound(deck: &str, role: &str, option: &str) -> Op {
    let json = match option.strip_prefix("kit:") {
        Some(kit) => obj(vec![("op", "setKit".into()), ("deck", deck.into()), ("kit", kit.into())]),
        None => obj(vec![("op", "setRoleVoice".into()), ("deck", deck.into()), ("role", role.into()), ("voice", option.into())]),
    };
    Op::Perform { deck: deck.into(), json }
}

/// The page's engine around the offline one: the ops applied as `engine-worklet.js` applies them, sent as `engine.js` sends
/// them (an unmoved knob is not sent again; a master op is given to the Wheel too), and the Wheel beside the decks.
pub struct Browser {
    /// `engine.js` `lastSent`: the last value of each knob.
    last: BTreeMap<String, String>,
    winder: Winder,
    wheel: WheelCore,
    wheel_used: bool,
    /// The render's frames so far, and the Wheel's current render quantum: the worklet renders it a quantum
    /// ([`QUANTUM`] frames, from the context's first frame) at a time, and a call lands between two.
    frames: u64,
    ql: Vec<f32>,
    qr: Vec<f32>,
    /// What the page ignores of the calls this render made, one entry per kind (its `ignoring ...` notes).
    pub ignored: BTreeSet<String>,
    /// What the engine refused, with why (the page's warnings).
    pub refused: Vec<String>,
}

impl Browser {
    pub fn new(sample_rate: f64) -> Browser {
        Browser { last: BTreeMap::new(), winder: Winder::new(), wheel: WheelCore::new(sample_rate), wheel_used: false, frames: 0, ql: vec![0.0; QUANTUM], qr: vec![0.0; QUANTUM], ignored: BTreeSet::new(), refused: Vec::new() }
    }

    /// engine.js `call`: a registry call through `mapCall`, its ops sent, the program's master given to the Wheel as the decks
    /// get it. `booth`: what to do with a call the page ignores (the render's own way of playing it), given the call.
    pub fn call(&mut self, e: &mut Engine, noun: &str, params: &JsonValue) -> Mapped {
        let m = map_call(noun, params);
        if noun == "moves" {
            // the booth's scheduler plays the program's move lines (djhost::offline_moves), as the page's MoveScheduler ports it,
            // and a vocoder handoff with them
            return m;
        }
        let master_target = (noun == "inserts" && params.get("target").and_then(JsonValue::as_str) == Some("Master")) || (noun == "mixer" && params.get("master").is_some());
        for op in &m.ops {
            if let Op::Wheel { action, params } = op {
                self.wheel_call(e, action, params);
                continue;
            }
            self.post(e, op, true);
            // the program's master reaches the wheel as it reaches the decks: deck A's share of a master op
            if master_target {
                match op {
                    Op::Perform { deck, json } if deck == "A" => match json.get("op").and_then(JsonValue::as_str) {
                        Some("masterFx") => self.wheel_master(json.get("key").and_then(JsonValue::as_str).unwrap_or(""), json.get("value").and_then(JsonValue::as_f64).unwrap_or(0.0)),
                        Some("master") => self.wheel_master("level", json.get("value").and_then(JsonValue::as_f64).unwrap_or(0.0)),
                        _ => {}
                    },
                    Op::Param { deck, name, value } if deck == "A" && name == "reverbWet" => self.wheel_master("reverbWet", *value),
                    _ => {}
                }
            }
        }
        if let Some(w) = &m.ignored {
            self.ignored.insert(w.clone());
        }
        m
    }

    /// A `mixer {master}` the render's arrangement played (`DeckAction::Master`): the page gives it to the Wheel as its level.
    pub fn master_level(&mut self, v: f64) {
        self.wheel_master("level", clamp01(v));
    }

    /// engine.js `run` of a `sound` op (a part's `kit=` / `sound=`, once its net has landed).
    pub fn sound(&mut self, e: &mut Engine, deck: &str, role: &str, option: &str) {
        let op = map_sound(deck, role, option);
        self.post(e, &op, false);
    }

    fn wheel_master(&mut self, name: &str, value: f64) {
        let key = format!("wheelMaster:{name}");
        let val = format!("{value}");
        if self.last.get(&key) == Some(&val) {
            return;
        }
        self.last.insert(key, val);
        if let Err(w) = self.wheel.master(name, value) {
            self.refused.push(format!("wheel master {name}: {w}"));
        }
    }

    /// engine.js `pumpWheel` and the worklet's `onWheel`: a wind (load, key, sequence, brightness) is wound and installed on the
    /// voice before the next call; a `level` is kept by the winding half too (for the next wind) and is the voice's; the rest
    /// are the voice's, with the decks' tempos now.
    fn wheel_call(&mut self, e: &Engine, action: &str, params: &JsonValue) {
        self.wheel_used = true;
        let Ok(a) = Args::parse(&params.canonical_js()) else { return };
        if winds(action) {
            match self.winder.call(&a) {
                Ok(Some(w)) => self.wheel.install(&w),
                Ok(None) => {}
                Err(why) => self.refused.push(format!("the wheel: {why}")),
            }
            return;
        }
        if action == "level" {
            let _ = self.winder.call(&a);
        }
        let tempo = |d: &str| e.deck(d).map_or(0.0, |d| d.mirror.tempo());
        let (ta, tb) = (tempo("A"), tempo("B"));
        if let Err(why) = self.wheel.call(&a, |d| if d == "B" { tb } else if d == "A" { ta } else { 0.0 }) {
            self.refused.push(format!("wheel: {why}"));
        }
    }

    /// engine.js `post`: an op sent (a knob that has not moved is not sent again), and applied as the worklet applies it.
    fn post(&mut self, e: &mut Engine, op: &Op, dedupe: bool) {
        if let Op::Move { .. } = op {
            return;
        }
        if dedupe {
            let (key, val) = match op {
                Op::Param { deck, name, value } => (format!("param:{deck}:{name}"), format!("{value}")),
                Op::Crossfader(v) => ("crossfader::".to_string(), format!("{v}")),
                Op::Play { deck, on } => (format!("play:{deck}:"), format!("{on}")),
                Op::Perform { deck, json } => (format!("perform:{deck}::{}:{}", json.get("op").and_then(JsonValue::as_str).unwrap_or(""), json.get("key").and_then(JsonValue::as_str).unwrap_or("")), json.canonical_js()),
                Op::Live { method, params, phrase } => {
                    if phrase.is_some() || params.get("swap").is_some_and(JsonValue::is_truthy) || params.get("remove").is_some_and(JsonValue::is_truthy) {
                        self.apply(e, op);
                        return;
                    }
                    let mut keys: Vec<&str> = params.as_object().map(|o| o.keys().collect()).unwrap_or_default();
                    keys.sort_unstable();
                    let inst = params.get("instance").map_or("0".to_string(), |v| v.canonical_js());
                    let mut key = format!("live:::{method}:{inst}:{}", keys.join(","));
                    if method == "inserts" {
                        let target = params.get("target").and_then(JsonValue::as_str).unwrap_or("");
                        let which = params.get("param").or_else(|| params.get("effect")).and_then(JsonValue::as_str).unwrap_or("");
                        key.push_str(&format!(":{target}:{which}"));
                    }
                    (key, params.canonical_js())
                }
                Op::Move { .. } | Op::Wheel { .. } => unreachable!("not posted"),
            };
            if self.last.get(&key) == Some(&val) {
                return;
            }
            self.last.insert(key, val);
        }
        self.apply(e, op);
    }

    /// engine-worklet.js `onMessage`: one op on the engine, as the `Dj` method it names does it.
    fn apply(&mut self, e: &mut Engine, op: &Op) {
        match op {
            Op::Param { deck, name, value } => {
                e.perform(&DeckAction::Param { deck: deck.clone(), name: name.clone(), value: *value }, PerformCtx::default());
            }
            Op::Crossfader(v) => e.set_crossfader(v.clamp(0.0, 1.0)),
            Op::Play { deck, on } => {
                e.perform(&DeckAction::Play { deck: deck.clone(), on: *on }, PerformCtx::default());
            }
            Op::Perform { deck, json } => {
                // djrust `beats::perform`: the action's own deck, else this one
                let mut v = json.clone();
                if let JsonValue::Object(o) = &mut v {
                    if o.get("deck").is_none() {
                        o.set("deck", JsonValue::str(deck.clone()));
                    }
                }
                match DeckAction::from_json(&v) {
                    Some(a) => {
                        e.perform(&a, PerformCtx::default());
                    }
                    None => self.refused.push(format!("not a deck action: {}", json.canonical_js())),
                }
            }
            Op::Live { method, params, .. } => {
                if let Err(w) = e.apply_live_op(method, params) {
                    self.refused.push(format!("{method}: {w}"));
                }
            }
            Op::Move { .. } | Op::Wheel { .. } => {}
        }
    }

    /// A call the page ignores, played the booth's way where the render has a way: an insert's switch and knob on the strip's
    /// chain (`inserts` live op: the registry's two calls). True when it was played.
    pub fn booth(&mut self, e: &mut Engine, noun: &str, params: &JsonValue) -> bool {
        if noun != "inserts" {
            return false;
        }
        let Some(p) = params.as_object() else { return false };
        // (a call the page plays in part, `inserts on reverb true`, is not here: the page's way of it stands)
        let raw = p.get("target").and_then(JsonValue::as_str).unwrap_or("");
        let target = if raw.eq_ignore_ascii_case("master") { "master" } else { raw };
        let q = match p.get("action").and_then(JsonValue::as_str) {
            Some("on") => obj(vec![("effect", p.get("effect").cloned().unwrap_or(JsonValue::Null)), ("on", JsonValue::Bool(on_flag(p))), ("target", target.into())]),
            Some("set") => obj(vec![("param", p.get("param").cloned().unwrap_or(JsonValue::Null)), ("target", target.into()), ("value", p.get("value").cloned().unwrap_or(JsonValue::Null))]),
            _ => return false,
        };
        let op = live("inserts", q);
        self.post(e, &op, true);
        true
    }

    /// Whether a `wheel` line has been played: from then on the Wheel is added to the master ([`render_add`](Self::render_add)).
    pub fn wheel_used(&self) -> bool {
        self.wheel_used
    }

    /// The worklet's `render`: the Wheel's next `l.len()` frames added to the master, after the decks, rendered a quantum at a
    /// time on the quanta the page renders (from frame 0), so a call made inside one is heard from the next, as there.
    pub fn render_add(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let at = (self.frames % QUANTUM as u64) as usize;
            if at == 0 {
                self.ql.fill(0.0);
                self.qr.fill(0.0);
                self.wheel.render_add(&mut self.ql, &mut self.qr);
            }
            *a += self.ql[at];
            *b += self.qr[at];
            self.frames += 1;
        }
    }

    /// `n` frames of the render with the Wheel not added (none played yet, or a stem that leaves it out): the quanta go on.
    pub fn skip(&mut self, n: usize) {
        self.frames += n as u64;
    }
}
