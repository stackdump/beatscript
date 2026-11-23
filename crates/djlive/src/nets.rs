// rename-to-beatscript: keep (the control-net name and the btw ACTION are identities, not prose)
//! Control lines as a Petri net (docs/design/NETS-ALL-THE-WAY-DOWN.md, phase 1: the proof).
//!
//! Until now a ramp (`master: filter lp 2k -> 8k over 8 bars`) or a move (`move filter_exit on A`) ran
//! BESIDE the deck nets: [`crate::runtime::Live`] counted bars and called the registry. Here the same
//! lines compile to a net of their own, which runs on the engine's own executor
//! ([`djengine::wave::WaveGraph`]: the one that plays the deck nets) and whose firings are the control
//! events.
//!
//! The net, in one picture:
//!
//! ```text
//!   clock   c0 -k0-> c1 -k1-> c2 ... -k(H-1)-> cH          one silent chain, a transition per sixteenth
//!             \taps: the transition before a tick T also puts a token into whatever starts at T
//!   ramp    r.s0 -t0-> r.s1 -t1-> ... -tK-> r.sK+1          a transition per step; t_k carries `set .. v_k`
//!   move    r.s0 -t0(trick on)-> ... -tL(mute; trick off)-> pend
//!           pend + free -arm-> muted        muted + pulse -restore(unmute)-> free
//!           pend, !free -absorb->           pulse, !muted -drain->             (pulse = the phrase line)
//! ```
//!
//! A transition's **label** is the typed event(s) it emits ([`Event`]): written as the control binding's
//! `action` and `targetNet`, because those two fields are what the engine's model CID hashes (beats'
//! canonical form leaves `macroParams` out). So the net's CID covers every value of every ramp.
//!
//! Time is the engine's own: one advance of the graph is one sixteenth step (`PPQ = 4` ticks a beat).
//! A transition tapped from the clock fires one tick after the clock transition that feeds it, so the
//! tap sits one transition early (`tap_before`) and the first event of a line lands exactly on its bar line.
//!
//! Pure (no I/O, no clock): the same program is the same net, byte for byte.

use std::collections::BTreeMap;

use djengine::cid;
use djengine::json::{JsonObject, JsonValue};
use djengine::wave::WaveGraph;

use crate::compile::{Call, Compiled, Control, Effect, Timing};

/// The id of the control net inside its project.
pub const NET_ID: &str = "ctl";
/// Steps in a bar (the engine's tick: a sixteenth).
pub const TICKS_PER_BAR: i64 = 16;
/// Bars in a phrase: where a move puts its strip back.
pub const PHRASE_BARS: i64 = 8;
/// A filter exit's depth into the high-pass (`djhost::perform::FILTER_EXIT_AMOUNT`).
pub const FILTER_EXIT_AMOUNT: f64 = 0.9;
/// An echo out's dry fade in beats (`djhost::perform::ECHO_CUT_BEATS`).
pub const ECHO_CUT_BEATS: f64 = 0.25;
/// The `action` every control transition of this net carries: to the engine an `Other` control, listed
/// as fired and doing nothing else (the host applies the event).
pub const ACTION: &str = "btw";

/// What a net asks the engine to do: the whole alphabet of phase 1. Small, typed and closed; every
/// registry call a control line compiles to is one of these or is left beside the net (`residual`).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A knob to a value: an insert knob (`filter.sweep`, `delay.mix`) of `Master` / `A` / `B`, the FX unit's `drive` of
    /// `Master` (both decks) / `A` / `B`, or a mixer field (`volume`, `low`, `mid`, `high` of a deck; `master`, `crossfader`
    /// with no target).
    Set { target: String, param: String, value: f64 },
    /// An insert effect on or off.
    Insert { target: String, effect: String, on: bool },
    /// A deck's channel muted or not.
    Mute { deck: String, on: bool },
    /// A performance trick (`filter`, `wash`) on or off, with the args a move gives it.
    Trick { deck: String, kind: String, on: bool, amount: Option<f64>, bars: Option<f64>, beats: Option<f64> },
    /// An entry of the DJ macro catalogue engaged on a deck's next bar line, for whole bars or its own length (`effects fire`, a
    /// `macro` line: macros.rs).
    Fire { target: String, id: String, bars: Option<u32> },
    /// A held one (a tape stop) let go (`effects release`).
    Release { target: String, id: String },
}

fn onoff(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

impl Event {
    /// The label text: `set Master filter.sweep 0.43`, `insert Master filter on`, `mute A on`,
    /// `trick A filter on amount=0.9 bars=4`.
    pub fn text(&self) -> String {
        match self {
            Event::Set { target, param, value } => format!("set {} {param} {value}", if target.is_empty() { "-" } else { target }),
            Event::Insert { target, effect, on } => format!("insert {target} {effect} {}", onoff(*on)),
            Event::Mute { deck, on } => format!("mute {deck} {}", onoff(*on)),
            Event::Trick { deck, kind, on, amount, bars, beats } => {
                let mut s = format!("trick {deck} {kind} {}", onoff(*on));
                for (k, v) in [("amount", amount), ("bars", bars), ("beats", beats)] {
                    if let Some(v) = v {
                        s.push_str(&format!(" {k}={v}"));
                    }
                }
                s
            }
            Event::Fire { target, id, bars } => match bars {
                Some(n) => format!("fire {target} {id} bars={n}"),
                None => format!("fire {target} {id}"),
            },
            Event::Release { target, id } => format!("release {target} {id}"),
        }
    }

    /// The inverse of [`text`](Self::text).
    pub fn parse(s: &str) -> Result<Event, String> {
        let w: Vec<&str> = s.split_whitespace().collect();
        let on = |x: &str| match x {
            "on" => Ok(true),
            "off" => Ok(false),
            o => Err(format!("`{o}` is not on or off")),
        };
        let num = |x: &str| x.parse::<f64>().map_err(|_| format!("`{x}` is not a number"));
        match w.as_slice() {
            ["set", t, p, v] => Ok(Event::Set { target: if *t == "-" { String::new() } else { t.to_string() }, param: p.to_string(), value: num(v)? }),
            ["insert", t, e, o] => Ok(Event::Insert { target: t.to_string(), effect: e.to_string(), on: on(o)? }),
            ["mute", d, o] => Ok(Event::Mute { deck: d.to_string(), on: on(o)? }),
            ["trick", d, k, o, rest @ ..] => {
                let (mut amount, mut bars, mut beats) = (None, None, None);
                for kv in rest {
                    let (k2, v) = kv.split_once('=').ok_or_else(|| format!("`{kv}` is not name=value"))?;
                    match k2 {
                        "amount" => amount = Some(num(v)?),
                        "bars" => bars = Some(num(v)?),
                        "beats" => beats = Some(num(v)?),
                        o => return Err(format!("trick has no arg `{o}`")),
                    }
                }
                Ok(Event::Trick { deck: d.to_string(), kind: k.to_string(), on: on(o)?, amount, bars, beats })
            }
            ["fire", t, id] => Ok(Event::Fire { target: t.to_string(), id: id.to_string(), bars: None }),
            ["fire", t, id, b] => {
                let n = b.strip_prefix("bars=").and_then(|n| n.parse::<u32>().ok()).ok_or_else(|| format!("`{b}` is not bars=N"))?;
                Ok(Event::Fire { target: t.to_string(), id: id.to_string(), bars: Some(n) })
            }
            ["release", t, id] => Ok(Event::Release { target: t.to_string(), id: id.to_string() }),
            _ => Err(format!("`{s}` is not an event")),
        }
    }
}

/// A transition's events as its label (the engine control binding's `targetNet`).
pub fn label(events: &[Event]) -> String {
    events.iter().map(Event::text).collect::<Vec<_>>().join(" ; ")
}

/// The events of a label.
pub fn parse_label(s: &str) -> Result<Vec<Event>, String> {
    s.split(" ; ").filter(|p| !p.trim().is_empty()).map(Event::parse).collect()
}

/// The events one registry call comes to, if it is in the alphabet.
pub fn call_events(c: &Call) -> Option<Vec<Event>> {
    let p = c.params.as_object()?;
    let s = |k: &str| p.get(k).and_then(JsonValue::as_str);
    let n = |k: &str| p.get(k).and_then(JsonValue::as_f64);
    let b = |k: &str| p.get(k).and_then(JsonValue::as_bool);
    match c.noun.as_str() {
        "inserts" => match s("action")? {
            "on" => Some(vec![Event::Insert { target: s("target")?.into(), effect: s("effect")?.into(), on: b("on")? }]),
            "set" => Some(vec![Event::Set { target: s("target")?.into(), param: s("param")?.into(), value: n("value")? }]),
            _ => None,
        },
        // the FX unit's drive on a deck's chain (`master:` is the `fx` noun with no deck: both decks'): a knob of that target
        "fx" => match (s("action")?, s("param")?) {
            ("set", "drive") => Some(vec![Event::Set { target: s("deck").unwrap_or("Master").into(), param: "drive".into(), value: n("value")? }]),
            _ => None,
        },
        // a macro line (macros.rs): the catalogue entry fired, and a held one let go
        "effects" => match s("action")? {
            "fire" => Some(vec![Event::Fire { target: s("target")?.into(), id: s("id")?.into(), bars: n("bars").map(|v| v as u32) }]),
            "release" => Some(vec![Event::Release { target: s("target")?.into(), id: s("id")?.into() }]),
            _ => None,
        },
        "mixer" => {
            if let (Some(d), Some(m)) = (s("deck"), b("mute")) {
                return Some(vec![Event::Mute { deck: d.into(), on: m }]);
            }
            if let Some(v) = n("master") {
                return Some(vec![Event::Set { target: String::new(), param: "master".into(), value: v }]);
            }
            if let Some(v) = n("crossfader") {
                return Some(vec![Event::Set { target: String::new(), param: "crossfader".into(), value: v }]);
            }
            let d = s("deck")?;
            for f in ["volume", "low", "mid", "high"] {
                if let Some(v) = n(f) {
                    return Some(vec![Event::Set { target: d.into(), param: f.into(), value: v }]);
                }
            }
            None
        }
        _ => None,
    }
}

fn calls_events(cs: &[Call]) -> Option<Vec<Event>> {
    let mut v = Vec::new();
    for c in cs {
        v.extend(call_events(c)?);
    }
    Some(v)
}

/// A control line as events at offsets (in ticks) from the tick it fires on.
struct Lowered {
    steps: BTreeMap<i64, Vec<Event>>,
    /// A move that ends by muting this deck: the net puts it back on the next phrase line.
    restore: Option<(String, Vec<Event>)>,
}

fn lower(c: &Control) -> Result<Lowered, String> {
    let mut steps: BTreeMap<i64, Vec<Event>> = BTreeMap::new();
    let mut restore = None;
    match &c.effect {
        Effect::Calls(cs) => {
            if let Some(Call { params, .. }) = cs.first().filter(|c| c.noun == "moves") {
                // a move: the two that end by muting their strip are a subnet; the rest stay beside the net
                let p = params.as_object().ok_or("moves: not an object")?;
                let kind = p.get("kind").and_then(JsonValue::as_str).unwrap_or("");
                let deck = p.get("deck").and_then(JsonValue::as_str).unwrap_or("A").to_string();
                let bars = p.get("bars").and_then(JsonValue::as_f64).unwrap_or(4.0).clamp(1.0, 64.0);
                if c.hold.is_some() {
                    return Err("`for N bars` on a move".into());
                }
                let (begin, trick) = match kind {
                    "filter_exit" => (Event::Trick { deck: deck.clone(), kind: "filter".into(), on: true, amount: Some(FILTER_EXIT_AMOUNT), bars: Some(bars), beats: None }, "filter"),
                    "echo_out" => (Event::Trick { deck: deck.clone(), kind: "wash".into(), on: true, amount: Some(1.0), bars: None, beats: Some(ECHO_CUT_BEATS) }, "wash"),
                    o => return Err(format!("move {o} stays beside the net (phase 2)")),
                };
                steps.entry(0).or_default().push(begin);
                steps.entry(bars as i64 * TICKS_PER_BAR).or_default().extend([
                    Event::Mute { deck: deck.clone(), on: true },
                    Event::Trick { deck: deck.clone(), kind: trick.into(), on: false, amount: None, bars: None, beats: None },
                ]);
                let undo = calls_events(&c.undo).ok_or("a move's put-back is not in the alphabet")?;
                restore = Some((deck, undo));
                return Ok(Lowered { steps, restore });
            }
            let ev = calls_events(cs).ok_or_else(|| format!("call `{}` is not in the event alphabet", cs.iter().find(|c| call_events(c).is_none()).map_or("", |c| c.noun.as_str())))?;
            steps.entry(0).or_default().extend(ev);
        }
        Effect::Ramp(r) => {
            steps.entry(0).or_default().extend(calls_events(&r.prelude).ok_or("the ramp's setup is not in the alphabet")?);
            let k = ticks_of(r.bars.as_bars()).map_err(|e| format!("a ramp's length: {e}"))?;
            if k == 0 {
                return Err("a ramp of no length".into());
            }
            let mut last: Option<Vec<Event>> = None;
            for i in 0..=k {
                let ev = call_events(&r.at(i as f64 / k as f64)).ok_or_else(|| format!("ramp on `{}` is not in the event alphabet", r.noun))?;
                // a step that repeats the one before it says nothing (the engine keeps the knob where it is)
                if last.as_ref() != Some(&ev) || i == k {
                    steps.entry(i).or_default().extend(ev.clone());
                }
                last = Some(ev);
            }
        }
        // the extensions' lines (duck, mix, lfo, sing ...) are phase 2: the arrangement carries what it has an op for
        _ => return Err("this line (a duck, a mix, an lfo, a sing) stays beside the net (phase 2)".into()),
    }
    if let Some(h) = c.hold {
        if !c.undo.is_empty() {
            let undo = calls_events(&c.undo).ok_or("what `for N bars` puts back is not in the alphabet")?;
            let at = ticks_of(h.as_bars()).map_err(|e| format!("`for` length: {e}"))?;
            steps.entry(at).or_default().extend(undo);
        }
    }
    Ok(Lowered { steps, restore })
}

/// The ticks a timing fires on, within `h` ticks.
fn triggers(t: Timing, h: i64) -> Result<Vec<i64>, String> {
    let mut v = Vec::new();
    match t {
        Timing::Now => v.push(0),
        Timing::At(b) => v.push(i64::from(b) * TICKS_PER_BAR),
        Timing::Every(n) => {
            let mut b = i64::from(n);
            while b * TICKS_PER_BAR < h && n > 0 {
                v.push(b * TICKS_PER_BAR);
                b += i64::from(n);
            }
        }
        Timing::EveryFrom(n, f) => {
            let mut b = i64::from(f);
            while b * TICKS_PER_BAR < h && n > 0 {
                v.push(b * TICKS_PER_BAR);
                b += i64::from(n);
            }
        }
        Timing::NextMult(n) => {
            let mut b = 0;
            while b * TICKS_PER_BAR < h && n > 0 {
                v.push(b * TICKS_PER_BAR);
                b += i64::from(n);
            }
        }
        // sub-bar times (`at bar 2 beat 3`, `every 2 beats`): a tick is a sixteenth, so a time on a sixteenth is a place in the
        // net and any other is beside it, with the reason
        Timing::AtPos(p) => v.push(ticks_of(p).map_err(|e| format!("`at` position: {e}"))?),
        Timing::EveryPos { step, from } => {
            let (st, mut at) = (ticks_of(step).map_err(|e| format!("`every` step: {e}"))?, ticks_of(from).map_err(|e| format!("`every` start: {e}"))?);
            if st > 0 {
                while at < h {
                    v.push(at);
                    at += st;
                }
            }
        }
    }
    v.retain(|t| *t < h);
    Ok(v)
}

/// A time in bars (exact) as ticks: it must fall on a sixteenth, the net's step.
fn ticks_of(bars: crate::rat::Rat) -> Result<i64, String> {
    bars.mul(crate::rat::Rat::int(TICKS_PER_BAR)).ok().and_then(|t| t.as_int()).ok_or_else(|| format!("{}/{} of a bar is not on a sixteenth, the net's step", bars.numer(), bars.denom()))
}

#[derive(Default)]
struct Builder {
    places: Vec<(String, i64)>,
    trans: Vec<(String, Vec<Event>)>,
    /// (source, target, inhibitor)
    arcs: Vec<(String, String, bool)>,
}

impl Builder {
    fn place(&mut self, id: String, tokens: i64) {
        self.places.push((id, tokens));
    }
    fn trans(&mut self, id: String, events: Vec<Event>) {
        self.trans.push((id, events));
    }
    fn arc(&mut self, a: &str, b: &str) {
        self.arcs.push((a.to_string(), b.to_string(), false));
    }
    fn inhibit(&mut self, p: &str, t: &str) {
        self.arcs.push((p.to_string(), t.to_string(), true));
    }

    fn project(&self, name: &str, tempo: f64) -> JsonValue {
        let mut places = JsonObject::new();
        for (id, n) in &self.places {
            places.set(id.as_str(), JsonValue::Object(JsonObject::from_pairs([("initial", JsonValue::Array(vec![JsonValue::int(*n)]))])));
        }
        let mut trans = JsonObject::new();
        for (id, ev) in &self.trans {
            let mut o = JsonObject::new();
            if !ev.is_empty() {
                o.set("label", JsonValue::str(label(ev)));
                o.set("control", JsonValue::Object(JsonObject::from_pairs([("action", JsonValue::str(ACTION)), ("targetNet", JsonValue::str(label(ev)))])));
            }
            trans.set(id.as_str(), JsonValue::Object(o));
        }
        let arcs = self
            .arcs
            .iter()
            .map(|(a, b, inh)| {
                let mut o = JsonObject::from_pairs([("source", JsonValue::str(a.as_str())), ("target", JsonValue::str(b.as_str()))]);
                if *inh {
                    o.set("inhibit", JsonValue::Bool(true));
                }
                JsonValue::Object(o)
            })
            .collect();
        let net = JsonObject::from_pairs([("role", JsonValue::str("control")), ("places", JsonValue::Object(places)), ("transitions", JsonValue::Object(trans)), ("arcs", JsonValue::Array(arcs))]);
        JsonValue::Object(JsonObject::from_pairs([("name", JsonValue::str(name)), ("tempo", JsonValue::Number(tempo)), ("nets", JsonValue::Object(JsonObject::from_pairs([(NET_ID, JsonValue::Object(net))])))]))
    }
}

/// The control lines of a program as a net.
#[derive(Clone, Debug)]
pub struct ControlNet {
    /// The net as a project (`djengine::pflow` JSON): what the engine's executor takes.
    pub project: JsonValue,
    /// How many ticks (sixteenths) the clock runs: the horizon.
    pub ticks: i64,
    /// The control lines that are in the net (their `Control::key`), and the transitions of each.
    pub lines: BTreeMap<String, Vec<String>>,
    /// The control lines that are NOT (their key, and why): they still run beside the net.
    pub residual: Vec<(String, String)>,
    /// The tempo the net was made at (a tick is a sixteenth of it).
    pub tempo: f64,
}

/// Compile the control lines of `c` that are in the alphabet to a net that runs for `bars` bars.
pub fn control_net(c: &Compiled, bars: u32) -> ControlNet {
    let h = i64::from(bars) * TICKS_PER_BAR;
    let tempo = c.tempo.unwrap_or(120.0);
    let mut b = Builder::default();
    let mut lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut residual = Vec::new();
    // taps[i]: places the clock transition k_i also puts a token in
    let mut taps: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    fn tap_before(tick: i64, place: &str, taps: &mut BTreeMap<i64, Vec<String>>) {
        if tick >= 1 {
            taps.entry(tick - 1).or_default().push(place.to_string());
        }
    }
    for (n, ctl) in c.controls.iter().enumerate() {
        let low = match lower(ctl) {
            Ok(l) => l,
            Err(why) => {
                residual.push((ctl.key.clone(), why));
                continue;
            }
        };
        let trig = match triggers(ctl.timing, h) {
            Ok(t) => t,
            Err(why) => {
                residual.push((ctl.key.clone(), why));
                continue;
            }
        };
        let last = *low.steps.keys().max().unwrap_or(&0);
        let mut ids = Vec::new();
        let s = |j: i64| format!("r{n}.s{j}");
        for j in 0..=last + 1 {
            b.place(s(j), i64::from(j == 0 && trig.contains(&0)));
        }
        for j in 0..=last {
            let id = format!("r{n}.t{j}");
            b.trans(id.clone(), low.steps.get(&j).cloned().unwrap_or_default());
            b.arc(&s(j), &id);
            b.arc(&id, &s(j + 1));
            if j == last && low.restore.is_some() {
                // the strip's mute is done: the put-back may now be armed
                b.arc(&id, &format!("r{n}.pend"));
            }
            ids.push(id);
        }
        for t in trig.iter().filter(|t| **t > 0) {
            tap_before(*t, &s(0), &mut taps);
        }
        if let Some((_deck, undo)) = &low.restore {
            let (pend, free, muted, pulse) = (format!("r{n}.pend"), format!("r{n}.free"), format!("r{n}.muted"), format!("r{n}.pulse"));
            b.place(pend.clone(), 0);
            b.place(free.clone(), 1);
            b.place(muted.clone(), 0);
            b.place(pulse.clone(), 0);
            for (id, ev) in [("arm", vec![]), ("absorb", vec![]), ("restore", undo.clone()), ("drain", vec![])] {
                b.trans(format!("r{n}.{id}"), ev);
                ids.push(format!("r{n}.{id}"));
            }
            // a strip is muted or free, never both and never neither (a P-invariant: muted + free = 1)
            b.arc(&pend, &format!("r{n}.arm"));
            b.arc(&free, &format!("r{n}.arm"));
            b.arc(&format!("r{n}.arm"), &muted);
            b.arc(&pend, &format!("r{n}.absorb"));
            b.inhibit(&free, &format!("r{n}.absorb"));
            b.arc(&muted, &format!("r{n}.restore"));
            b.arc(&pulse, &format!("r{n}.restore"));
            b.arc(&format!("r{n}.restore"), &free);
            b.arc(&pulse, &format!("r{n}.drain"));
            b.inhibit(&muted, &format!("r{n}.drain"));
            let mut j = 1;
            while j * PHRASE_BARS * TICKS_PER_BAR < h {
                tap_before(j * PHRASE_BARS * TICKS_PER_BAR, &pulse, &mut taps);
                j += 1;
            }
            // the end of the program is a put-back line too (what a line holds changed is put back as it ends)
            tap_before(h - 1, &pulse, &mut taps);
        }
        lines.insert(ctl.key.clone(), ids);
    }
    // the clock: a silent chain, a transition a sixteenth; the lines are tapped off it
    if !taps.is_empty() {
        for i in 0..=h {
            b.place(format!("c{i}"), i64::from(i == 0));
        }
        for i in 0..h {
            let k = format!("k{i}");
            b.trans(k.clone(), Vec::new());
            b.arc(&format!("c{i}"), &k);
            b.arc(&k, &format!("c{}", i + 1));
            for p in taps.get(&i).into_iter().flatten() {
                b.arc(&k, p);
            }
        }
    }
    // named by the program's sound (compile::sound_cid), so a program and its BeatScript 2 upgrade have one control net
    let sound = crate::compile::sound_cid(&c.text);
    let name = format!("btw control {}", &sound[..sound.len().min(12)]);
    ControlNet { project: b.project(&name, tempo), ticks: h, lines, residual, tempo }
}

impl ControlNet {
    /// The CID of the control net alone: the share CID of its project.
    pub fn cid(&self) -> String {
        cid::share_cid(&self.project)
    }

    /// The engine's own model CID of the net (beats' canonical project form): it hashes each control
    /// transition's `action` and `targetNet`, which is where the events are.
    pub fn model_cid(&self) -> String {
        djengine::pflow::Project::from_json(&self.project).model_cid()
    }

    /// Places, transitions and arcs (how big the net is).
    pub fn size(&self) -> (usize, usize, usize) {
        let net = self.project.get("nets").and_then(|n| n.get(NET_ID));
        let count = |k: &str| net.and_then(|n| n.get(k)).map_or(0, |v| v.as_object().map_or_else(|| v.as_array().map_or(0, Vec::len), JsonObject::len));
        (count("places"), count("transitions"), count("arcs"))
    }

    fn graph(&self) -> WaveGraph {
        WaveGraph::from_json(&self.project)
    }

    /// Run the net for `ticks` advances (default: the horizon and the tick on it, where a ramp that ends
    /// with the program has its last value) and list what fired, tick by tick: the event stream of the
    /// performance. Tick `t` is the sixteenth `t` after the program's first bar line.
    pub fn events(&self, ticks: Option<i64>) -> Vec<(i64, Event)> {
        let mut g = self.graph();
        let mut out = Vec::new();
        for t in 0..ticks.unwrap_or(self.ticks + 1) {
            g.advance();
            out.extend(fired(&g, t));
        }
        out
    }

    /// The marking after `tick` advances: `{place: tokens}` for the places that hold any. This is the
    /// whole state of the control lines at that sixteenth (the debugger's view; a seek's target).
    pub fn marking_at(&self, tick: i64) -> BTreeMap<String, i64> {
        let mut g = self.graph();
        for _ in 0..tick {
            g.advance();
        }
        g.marking().remove(NET_ID).unwrap_or_default().into_iter().filter(|(_, n)| *n > 0.0).map(|(p, n)| (p, n as i64)).collect()
    }

    /// The events fired in advance number `tick` (0-based), from a graph already advanced to `tick`.
    pub fn runner(&self) -> Runner {
        Runner { g: self.graph(), tick: 0 }
    }
}

/// The events of the advance a graph just made (`tick` is the 0-based tick it was).
fn fired(g: &WaveGraph, tick: i64) -> Vec<(i64, Event)> {
    let mut v = Vec::new();
    for i in (0..g.control_count).step_by(2) {
        let (ni, t) = (g.controls[i] as usize, g.controls[i + 1] as usize);
        let net = &g.nets[ni];
        let Some(cb) = net.bundle.control_bindings.get(&net.trans_ids[t]) else { continue };
        if cb.action != ACTION {
            continue;
        }
        match parse_label(&cb.target_net) {
            Ok(ev) => v.extend(ev.into_iter().map(|e| (tick, e))),
            Err(e) => debug_assert!(false, "bad event label: {e}"),
        }
    }
    v
}

/// The document that names a whole performance: the nets that play (by CID) and the control net, so
/// two programs that sound the same are one performance whatever their text, and two that differ in a
/// sweep the deck nets cannot see are two. Existing CIDs (text, deck net, section net, generation) are not
/// touched: this is one more document, over them.
pub fn performance_doc(c: &Compiled, bars: u32) -> JsonValue {
    let decks = match c.song_net_cids() {
        Ok(m) => m,
        Err(_) => c.net_cids(),
    };
    let mut o = JsonObject::from_pairs([("format", JsonValue::int(1)), ("type", JsonValue::str("djrust.btw.performance")), ("tempo", JsonValue::Number(c.tempo.unwrap_or(120.0)))]);
    o.set("decks", JsonValue::Object(JsonObject::from_pairs(decks.into_iter().map(|(k, v)| (k, JsonValue::str(v))))));
    let n = control_net(c, bars);
    if n.size().1 > 0 {
        o.set("control", JsonValue::str(n.cid()));
    }
    if !n.residual.is_empty() {
        // still run beside the net: named, so the performance's identity covers them until they are in it
        o.set("beside", JsonValue::Array(n.residual.iter().map(|(k, _)| JsonValue::str(k.as_str())).collect()));
    }
    JsonValue::Object(o)
}

/// The CID of a performance ([`performance_doc`]).
pub fn performance_cid(c: &Compiled, bars: u32) -> String {
    cid::share_cid(&performance_doc(c, bars))
}

/// A control net being played a tick at a time (what a host does from the audio clock).
pub struct Runner {
    g: WaveGraph,
    tick: i64,
}

impl Runner {
    /// Advance one sixteenth; the events that fired on it.
    pub fn step(&mut self) -> Vec<Event> {
        self.g.advance();
        let v = fired(&self.g, self.tick).into_iter().map(|(_, e)| e).collect();
        self.tick += 1;
        v
    }

    pub fn tick(&self) -> i64 {
        self.tick
    }
}
