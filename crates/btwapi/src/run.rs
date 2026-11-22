//! The live runtime behind a host that is a command queue (btwsite/DESIGN.md, `BtwRun`).
//!
//! [`djlive::Live`] is the program that is running: it counts bars off a clock, loads each section's nets a bar
//! ahead, fires timed lines and moves ramps, and reports what it did to a [`LiveHost`]. On the desktop the host
//! is the app's engine. Here the host is a queue: whoever owns the sound (the browser's AudioWorklet engine)
//! reads the commands out and applies them, and tells us where its clock is by calling [`Run::service`] with
//! the grid. Nothing in here has a clock or does I/O, so a native test drives it on a fake one and the wasm
//! module (`djweb::BtwRun`) gives the browser the same answers.
//!
//! Commands (JSON objects, in the order the runtime issued them):
//!
//! ```text
//! {"op":"load_net","deck":"A","envelope":{...}}            a raw-nets share envelope for the deck
//! {"op":"play","deck":"A","on":true}
//! {"op":"sound","deck":"A","role":"bass","option":"synth:reese"}
//! {"op":"call","noun":"inserts","params":{...}}            one call on the control-language registry
//! ```
//!
//! `call` answers `Ok(null)` at once (the registry's real answer is not available to a program anyway);
//! `playing(deck)` reflects the flags passed in with the grid, plus what this runtime has itself switched
//! on or off since (so a `play` it just queued is not queued twice in one pass).
//!
//! The grid is `{"deck":"A","beat":12.5,"bpm":140,"playing":{"A":true,"B":false}}` or `null` (no clock: nothing
//! plays). `beat` counts from bar 0's downbeat of the grid deck, as `Engine::beat_state` does.
//!
//! Spans in `marks()` and `eval()`'s diagnostics are UTF-8 byte offsets into the text that was evaluated, and
//! every one also carries `start16` / `end16` (UTF-16 code units) so JavaScript can slice its string.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::Severity;
use djlive::runtime::{Eval, Grid, Live, LiveHost};

use crate::report::diagnostic_json;
use crate::snapshot::web_env;
use crate::utf16::{add_utf16_spans, Utf16Index};

/// Where the clock deck is, as the page reads it off the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct GridIn {
    pub deck: String,
    pub beat: f64,
    pub bpm: f64,
    /// Which decks play (a deck missing here does not).
    pub playing: BTreeMap<String, bool>,
}

impl GridIn {
    /// Read the grid JSON. `null`, an empty string or `{}` is no clock (None).
    pub fn parse(text: &str) -> Result<Option<GridIn>, String> {
        let t = text.trim();
        if t.is_empty() || t == "null" || t == "undefined" {
            return Ok(None);
        }
        let v = JsonValue::parse(t).map_err(|e| format!("grid: {e}"))?;
        if matches!(v, JsonValue::Null) {
            return Ok(None);
        }
        Ok(GridIn::from_json(&v))
    }

    pub fn from_json(v: &JsonValue) -> Option<GridIn> {
        let deck = v.get("deck").and_then(JsonValue::as_str)?.to_string();
        let beat = v.get("beat").and_then(JsonValue::as_f64)?;
        if !beat.is_finite() {
            return None;
        }
        let bpm = v.get("bpm").and_then(JsonValue::as_f64).filter(|b| b.is_finite() && *b > 0.0).unwrap_or(120.0);
        let mut playing = BTreeMap::new();
        if let Some(JsonValue::Object(o)) = v.get("playing") {
            for (k, x) in o.iter() {
                playing.insert(k.to_string(), x.as_bool().unwrap_or(false));
            }
        }
        Some(GridIn { deck, beat, bpm, playing })
    }
}

/// The host: a clock the caller sets and a queue of what the runtime asked for.
#[derive(Default)]
struct QueueHost {
    grid: Option<Grid>,
    playing: BTreeMap<String, bool>,
    out: Vec<JsonValue>,
}

fn obj<const N: usize>(pairs: [(&str, JsonValue); N]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

impl LiveHost for QueueHost {
    fn grid(&self) -> Option<Grid> {
        self.grid.clone()
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing.get(deck).copied().unwrap_or(false)
    }

    fn load_net(&mut self, deck: &str, envelope: &JsonValue) -> Result<(), String> {
        self.out.push(obj([("op", "load_net".into()), ("deck", deck.into()), ("envelope", envelope.clone())]));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing.insert(deck.to_string(), on);
        self.out.push(obj([("op", "play".into()), ("deck", deck.into()), ("on", on.into())]));
    }

    fn set_sound(&mut self, deck: &str, role: &str, option: &str) -> Result<(), String> {
        self.out.push(obj([("op", "sound".into()), ("deck", deck.into()), ("role", role.into()), ("option", option.into())]));
        Ok(())
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.out.push(obj([("op", "call".into()), ("noun", noun.into()), ("params", params.clone())]));
        Ok(JsonValue::Null)
    }
}

/// A program running on a clock someone else keeps.
pub struct Run {
    live: Live,
    host: QueueHost,
}

impl Default for Run {
    fn default() -> Self {
        Run::new()
    }
}

impl Run {
    pub fn new() -> Run {
        Run { live: Live::new(), host: QueueHost::default() }
    }

    /// The runtime itself (read-only), for tests.
    pub fn live(&self) -> &Live {
        &self.live
    }

    /// Evaluate `text`: with an error nothing changes (the last good program plays on); a good program
    /// is staged and goes out as commands from the `service` passes that follow, on the next bar.
    /// The answer is the desktop's eval answer: `ok`, `errors`, `diagnostics`, `cid`, `nets`, `canonical`,
    /// `changes`, `plan`, `lands`, `lands_at`, `unchanged`, `evolve`, `lineage`, `kept`.
    pub fn eval(&mut self, text: &str) -> JsonValue {
        let ev = self.live.eval_with(text, &web_env().env());
        self.eval_answer(&ev, text)
    }

    /// `eval`'s answer for `text` without staging it.
    pub fn check(&self, text: &str) -> JsonValue {
        let ev = self.live.check_with(text, &web_env().env());
        self.eval_answer(&ev, text)
    }

    fn eval_answer(&self, ev: &Eval, text: &str) -> JsonValue {
        let errors = ev.diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
        let ix = Utf16Index::new(text);
        // the same diagnostic JSON `check` answers with (start16 / end16, `render`)
        let diags: Vec<JsonValue> = ev.diagnostics.iter().map(|d| diagnostic_json(d, text, &ix)).collect();
        let plan = ev.plan.as_ref();
        let strs = |v: Vec<String>| JsonValue::Array(v.into_iter().map(JsonValue::str).collect());
        obj([
            ("ok", ev.ok.into()),
            ("errors", JsonValue::int(errors as i64)),
            ("diagnostics", JsonValue::Array(diags)),
            ("cid", ev.cid.clone().map_or(JsonValue::Null, Into::into)),
            ("nets", JsonValue::Object(JsonObject::from_pairs(ev.nets.iter().map(|(k, v)| (k.as_str(), JsonValue::str(v.clone()))).collect::<Vec<_>>()))),
            ("canonical", ev.canonical.clone().map_or(JsonValue::Null, Into::into)),
            ("changes", plan.map_or(JsonValue::Null, |p| strs(p.lines()))),
            ("plan", plan.map_or(JsonValue::Null, |p| p.to_json())),
            ("lands", ev.lands.map_or(JsonValue::Null, JsonValue::int)),
            (
                "lands_at",
                if ev.ok && !ev.unchanged {
                    if ev.lands.is_some() { "the next bar" } else { "once (nothing plays: the decks start)" }
                } else {
                    ""
                }
                .into(),
            ),
            ("unchanged", ev.unchanged.into()),
            ("evolve", strs(ev.evolve.clone())),
            (
                "lineage",
                JsonValue::Array(
                    ev.lineage
                        .iter()
                        .map(|g| {
                            obj([
                                ("scope", g.scope.clone().into()),
                                ("gen", JsonValue::int(i64::from(g.gen))),
                                ("cid", g.cid.clone().into()),
                                ("forkedFrom", g.forked_from.clone().map_or(JsonValue::Null, Into::into)),
                                ("start_bar", JsonValue::int(i64::from(g.start))),
                                ("bars", JsonValue::int(i64::from(g.bars))),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "kept",
                if ev.ok {
                    JsonValue::Null
                } else {
                    self.live.running_cid().map_or(JsonValue::str("nothing was running"), |c| JsonValue::str(format!("the last good program ({c}) plays on")))
                },
            ),
        ])
    }

    /// End the program, first putting back what its lines still hold changed (a muted strip, a filter):
    /// the decks play on. The commands that does.
    pub fn stop(&mut self) -> JsonValue {
        self.host.out.clear();
        self.live.stop_with(&mut self.host);
        self.take_commands()
    }

    /// One pass: tell the runtime where the clock is, and get what it did. `grid` None: no clock.
    pub fn service(&mut self, grid: Option<&GridIn>) -> JsonValue {
        match grid {
            Some(g) => {
                self.host.grid = Some(Grid { deck: g.deck.clone(), beat: g.beat, bpm: g.bpm });
                self.host.playing = g.playing.clone();
            }
            None => {
                self.host.grid = None;
                self.host.playing.clear();
            }
        }
        self.host.out.clear();
        self.live.service(&mut self.host);
        self.take_commands()
    }

    fn take_commands(&mut self) -> JsonValue {
        JsonValue::Array(std::mem::take(&mut self.host.out))
    }

    /// What to light in the text right now: `Live::marks().to_json()` plus `start16` / `end16` on every span.
    pub fn marks(&self) -> JsonValue {
        let m = self.live.marks();
        let mut j = m.to_json();
        add_utf16_spans(&mut j, &m.source);
        j
    }

    /// `Live::status()`: section, bar, ended, staged, errors, the last words.
    pub fn status(&self) -> JsonValue {
        self.live.status()
    }

    pub fn running_cid(&self) -> Option<String> {
        self.live.running_cid().map(str::to_string)
    }

    // ---- the string forms the wasm layer hands to JavaScript (and the native tests compare against)

    pub fn eval_str(&mut self, text: &str) -> String {
        self.eval(text).js_stringified()
    }

    pub fn stop_str(&mut self) -> String {
        self.stop().js_stringified()
    }

    pub fn service_str(&mut self, grid_json: &str) -> Result<String, String> {
        let g = GridIn::parse(grid_json)?;
        Ok(self.service(g.as_ref()).js_stringified())
    }

    pub fn marks_str(&self) -> String {
        self.marks().js_stringified()
    }

    pub fn status_str(&self) -> String {
        self.status().js_stringified()
    }
}
