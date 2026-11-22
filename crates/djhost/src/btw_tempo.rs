//! A btw program's tempo curve (F3, docs/btw/ext-tempo.md) in the offline render ([`crate::btw_render`]).
//!
//! The arrangement format has no op for "the tempo is now X", and the engine's net clock takes its tempo from the deck's
//! base tempo (the net's own, which a section's net says in its envelope) times the tempo fader. So a program whose tempo
//! moves is rendered as a **staircase** of constant tempos ([`djlive::tempo::TempoMap::staircase`]): every step is a
//! tempo-fader move of each deck that plays (`pitch`, with the fader's travel set to the smallest of the engine's ranges
//! that reaches the curve), and every event of the arrangement carries the tempo it falls in (`bpm`), which is what the
//! renderer's own clock (`Arrangement::seconds_at`) reads to put a move on its exact second.
//!
//! The tempo of a step is the constant one that takes **exactly** the seconds the curve takes over the step
//! (`60 * beats / seconds`, from the closed-form integral), so the render is as long as `total_seconds` says, to the
//! frame, and a section's net lands on the exact second its bar line is at.
//!
//! The tempo fader reaches +-50 % of a net's own tempo at most (the engine's `PITCH_RANGES`): a curve that goes further
//! inside one net is held at the edge, and [`notes`] says so. A program is made of sections (each section's nets are
//! made at the tempo it begins at) so that a long curve stays inside the travel.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeEvent, ArrangeOp};
use djlive::ast::Deck;
use djlive::compile::{Compiled, DeckIr};

/// The fader's travels, the engine's own (`djrust::mixer::PITCH_RANGES`): +-8 %, +-16 %, +-50 %.
const RANGES: [f64; 3] = [0.08, 0.16, 0.50];
/// A ramp is cut into steps of this many beats (a sixteenth of a bar).
const STEP_BEATS: f64 = 0.25;
/// The tempo a net has when its envelope says none.
const NET_DEFAULT_BPM: f64 = 120.0;

/// What a section is, to the curve: where it begins (in beats) and the nets it loads.
pub type Set<'a> = (u32, u32, &'a BTreeMap<Deck, DeckIr>);

/// A tempo curve laid out for the renderer.
pub struct Curve {
    /// (start beat, the constant tempo of the step), in order: what every event's `bpm` is read from.
    steps: Vec<(f64, f64)>,
    /// The tempo each section's nets are made at in the render (None: the program's own): a net that lands takes
    /// the tempo of the net before it, see [`Curve::net_tempo`].
    net_tempos: Vec<Option<f64>>,
    /// The moves of the tempo fader (and its travel), in order of beat.
    events: Vec<ArrangeEvent>,
    /// What could not be done, in words.
    pub notes: Vec<String>,
}

impl Curve {
    /// The curve of a program over its first `beats` beats, or None where its tempo is no curve (one number, or none).
    pub fn of(c: &Compiled, sets: &[Set<'_>], beats: f64) -> Option<Curve> {
        let map = c.tempo_map();
        if !map.drives() || beats <= 0.0 {
            return None;
        }
        // A section's nets are asked for 2 beats before its bar line (they land on it), and the engine takes the new net's own
        // tempo as the deck's base from the moment they are asked for: the fader's steps are relative to the net that was
        // asked for last. So a section's window opens when its nets are asked for.
        let asked = |k: usize| if k == 0 { 0.0 } else { f64::from(sets[k].0) * 4.0 - 2.0 };
        // where the tempo changes, and where a window opens
        let mut cuts: Vec<f64> = map.staircase(beats, STEP_BEATS).iter().map(|s| s.0).collect();
        cuts.extend((0..sets.len()).map(asked).filter(|b| *b < beats));
        cuts.extend(sets.iter().map(|s| f64::from(s.0) * 4.0).filter(|b| *b < beats));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut steps: Vec<(f64, f64)> = Vec::new();
        for (i, b0) in cuts.iter().enumerate() {
            let b1 = cuts.get(i + 1).copied().unwrap_or(beats);
            let secs = map.seconds_at_f(b1 / 4.0) - map.seconds_at_f(b0 / 4.0);
            let bpm = if secs > 0.0 { 60.0 * (b1 - b0) / secs } else { map.bpm_at(0u32) };
            steps.push((*b0, bpm));
        }
        // A net lands at the tick before its bar line (the engine drops that tick and starts the new net's first one a tick
        // of the new net's own tempo later), and the engine sets the tempo to the new net's own as it lands. So a net lands
        // exactly on its bar line when its own tempo is the one the tick before it was played at: the render makes each
        // section's nets (but the first's) at the tempo the curve has just before their bar line, whatever the program's
        // own is, and the fader moves them to where the curve is.
        let tempo_before = |beat: f64| steps.iter().rev().find(|s| s.0 < beat - 1e-9).map(|s| s.1);
        let net_tempos: Vec<Option<f64>> = (0..sets.len()).map(|k| if k == 0 { None } else { tempo_before(f64::from(sets[k].0) * 4.0) }).collect();
        let mut events = Vec::new();
        let mut notes = Vec::new();
        for (k, (start, _bars, nets)) in sets.iter().enumerate() {
            let (w0, w1) = (asked(k), if k + 1 < sets.len() { asked(k + 1) } else { beats }.min(beats));
            if w0 >= beats {
                continue;
            }
            let inside: Vec<&(f64, f64)> = steps.iter().filter(|s| s.0 >= w0 - 1e-9 && s.0 < w1 - 1e-9).collect();
            for (deck, ir) in nets.iter() {
                let base = net_tempos[k].or(ir.tempo).unwrap_or(NET_DEFAULT_BPM);
                // the step that is furthest from the net's own tempo, on whichever side (the note names that tempo itself: the
                // largest deviation's size with the sign of any step above the net's own said 560 for a curve that goes down to 40)
                let worst = inside.iter().map(|s| s.1).fold(base, |w, b| if (b / base - 1.0).abs() > (w / base - 1.0).abs() { b } else { w });
                let need = (worst / base - 1.0).abs();
                let range = RANGES.iter().copied().find(|r| *r >= need - 1e-12).unwrap_or(RANGES[RANGES.len() - 1]);
                if need > RANGES[RANGES.len() - 1] + 1e-9 {
                    notes.push(format!(
                        "deck {}{}: the tempo goes to {} from the net's own {}: the engine's tempo fader reaches {}% (it is held at the edge); make the curve in sections, each net is made at the tempo it begins at",
                        deck.name(),
                        if k > 0 { format!(" in the section at bar {start}") } else { String::new() },
                        round(worst),
                        round(base),
                        (range * 100.0).round()
                    ));
                }
                events.push(event(w0, ArrangeOp::Live { method: "pitch_range".into(), params: JsonValue::Object(JsonObject::from_pairs([("deck", JsonValue::str(deck.name())), ("range", JsonValue::Number(range))])) }, format!("deck {} tempo travel ±{}%", deck.name(), (range * 100.0).round())));
                let mut last: Option<f64> = None;
                for (at, bpm) in &inside {
                    let pitch = ((bpm / base - 1.0) / range).clamp(-1.0, 1.0);
                    // the first of a section always (a net is asked for with the fader as it was), then what moved
                    if last.is_none_or(|l| (l - pitch).abs() > 1e-12) {
                        events.push(event(*at, ArrangeOp::Deck(DeckAction::Param { deck: deck.name().to_string(), name: "pitch".into(), value: pitch }), format!("deck {} tempo {}", deck.name(), round(*bpm))));
                        last = Some(pitch);
                    }
                }
            }
        }
        Some(Curve { steps, net_tempos, events, notes })
    }

    /// The tempo the arrangement starts at.
    pub fn start_bpm(&self) -> f64 {
        self.steps.first().map_or(120.0, |s| s.1)
    }

    /// The tempo the nets of section `k` are made at in the render, where it is not the program's own: the tempo the
    /// curve has just before their bar line (so that they land exactly on it).
    pub fn net_tempo(&self, k: usize) -> Option<f64> {
        self.net_tempos.get(k).copied().flatten()
    }

    /// The moves of the tempo fader, to be added to the arrangement's events.
    pub fn events(&self) -> Vec<ArrangeEvent> {
        self.events.clone()
    }

    /// The tempo of the step a beat is in.
    pub fn bpm_at(&self, beat: f64) -> f64 {
        self.steps.iter().rev().find(|s| s.0 <= beat + 1e-9).map_or_else(|| self.start_bpm(), |s| s.1)
    }

    /// Every event says the tempo it falls in (what `Arrangement::seconds_at` reads).
    pub fn tag(&self, events: &mut [ArrangeEvent]) {
        for e in events {
            e.bpm = self.bpm_at(e.at);
        }
    }
}

/// The net's raw-nets envelope, made at `tempo` where that is given (the render's own, [`Curve::net_tempo`]).
pub fn envelope(ir: &DeckIr, tempo: Option<f64>) -> JsonValue {
    let env = ir.envelope(0);
    match (tempo, env.as_object()) {
        (Some(t), Some(o)) => {
            let mut o = o.clone();
            o.set("tempo", JsonValue::Number(t));
            JsonValue::Object(o)
        }
        _ => env,
    }
}

fn event(at: f64, op: ArrangeOp, text: String) -> ArrangeEvent {
    ArrangeEvent { at, bpm: 0.0, op, via: "btw".into(), text }
}

fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// What the render could not do of a program's tempo curve (empty for a program whose tempo is one number).
pub fn notes(c: &Compiled) -> Vec<String> {
    let sets: Vec<Set<'_>> = if c.arranged { c.segments.iter().map(|s| (s.start, s.bars, &s.decks)).collect() } else { vec![(0, 16, &c.decks)] };
    let beats = sets.last().map_or(64.0, |(s, b, _)| f64::from(s + b) * 4.0);
    Curve::of(c, &sets, beats).map(|cv| cv.notes).unwrap_or_default()
}
