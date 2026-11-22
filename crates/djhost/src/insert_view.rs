//! The Inserts page's view of the engine's insert chains (`djrust::host::deck::inserts`): the
//! four targets a player picks between (Deck A, Deck B, the Wheel's strip, the Master), each
//! chain's effects and knobs with their display strings, and the tempo its delay follows.
//! Shared by djapp's Inserts window and djandroid's Inserts tab (djmobile `inserts.rs`, where
//! this started), so both name, show and time the same chains.

use djengine::fx::insert::{division_of, InsertKind, InsertParam, InsertSettings, DIVISIONS};
use djengine::json::{JsonObject, JsonValue};

use crate::deck::engine::{crossfade_gain, Engine};
use crate::deck::inserts::{INPUT_PREFIX, MASTER};

/// The targets, in the page's order.
pub const TARGETS: [&str; 4] = ["A", "B", "Wheel", "Master"];

/// The Wheel's input strip (djhost `wheel::INPUT_LABEL`; named here too so a build without the
/// wheel still names the target).
pub const WHEEL_STRIP: &str = "Windwheel";

/// "Deck A", "Deck B", "Wheel", "Master".
pub fn target_label(t: &str) -> String {
    match t {
        "A" | "B" => format!("Deck {t}"),
        other => other.to_string(),
    }
}

/// The engine's name for a page target.
pub fn engine_target(target: &str) -> Result<String, String> {
    match target {
        "A" | "B" => Ok(target.to_string()),
        "Wheel" => Ok(format!("{INPUT_PREFIX}{WHEEL_STRIP}")),
        "Master" => Ok(MASTER.to_string()),
        _ => Err(format!("no target {target} (A, B, Wheel or Master)")),
    }
}

/// One knob, for the page.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertKnob {
    /// "filter.sweep", "delay.mix", …
    pub id: String,
    pub label: String,
    /// 0 … 1.
    pub value: f64,
    /// "LP 1.2 kHz", "3/16", "45 %".
    pub display: String,
    /// Steps for a discrete knob (the delay's divisions); 0: continuous.
    pub steps: u32,
}

/// One effect of a chain.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertEffect {
    /// "filter", "crush", "delay", "reverb".
    pub id: String,
    pub label: String,
    pub on: bool,
    pub knobs: Vec<InsertKnob>,
}

/// A target's chain, for the page.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertChainState {
    /// "A", "B", "Wheel" or "Master".
    pub target: String,
    pub effects: Vec<InsertEffect>,
    /// The tempo its delay follows now (0: nothing playing; it keeps the last).
    pub bpm: f64,
    /// The delay's division at that tempo, ms (0 when no tempo).
    pub delay_ms: f64,
    /// Which deck that tempo comes from ("A" or "B"; empty: none playing).
    pub tempo_from: String,
}

impl InsertChainState {
    /// Any effect of it on (the target switch's dot).
    pub fn any_on(&self) -> bool {
        self.effects.iter().any(|e| e.on)
    }

    /// The line under the target switch: what the delay follows, and its time.
    pub fn tempo_line(&self) -> String {
        let div = self
            .effects
            .iter()
            .find(|e| e.id == "delay")
            .and_then(|e| e.knobs.iter().find(|k| k.id == "delay.time"))
            .map(|k| k.display.clone())
            .unwrap_or_default();
        if self.bpm > 0.0 {
            let from = if self.target == self.tempo_from { "its tempo".to_string() } else { format!("Deck {}", self.tempo_from) };
            format!("Delay follows {from} · {:.1} BPM · {div} = {:.0} ms", self.bpm, self.delay_ms)
        } else {
            let what = if self.target.len() == 1 { "this deck" } else { "the lead deck" };
            format!("Delay follows {what} once it plays · {div}")
        }
    }
}

/// The tempo `target`'s delay follows, and from which deck: a deck its own; the Wheel and the
/// Master the lead deck (playing, loudest through its fader and the crossfader).
pub fn tempo_of(e: &Engine, target: &str) -> (f64, String) {
    let bpm_of = |d: &str| e.deck(d).filter(|x| x.mirror.is_playing()).map(|x| x.mirror.tempo()).filter(|b| *b > 0.0);
    match target {
        "A" | "B" => bpm_of(target).map_or((0.0, String::new()), |b| (b, target.to_string())),
        _ => {
            let mut best: Option<(f64, f64, &str)> = None;
            for d in ["A", "B"] {
                let (Some(b), Some(x)) = (bpm_of(d), e.deck(d)) else { continue };
                let g = x.volume * crossfade_gain(d, e.crossfader);
                if best.is_none_or(|(bg, _, _)| g > bg) {
                    best = Some((g, b, d));
                }
            }
            best.map_or((0.0, String::new()), |(_, b, d)| (b, d.to_string()))
        }
    }
}

/// `target`'s chain as the page shows it (a chain never touched: the defaults, all off).
pub fn chain_state(e: &Engine, target: &str) -> Result<InsertChainState, String> {
    let s = e.insert_settings(&engine_target(target)?).unwrap_or_default();
    Ok(state_of(target, &s, e))
}

fn state_of(target: &str, s: &InsertSettings, e: &Engine) -> InsertChainState {
    let (bpm, from) = tempo_of(e, target);
    let beats = DIVISIONS[division_of(s.get(InsertParam::DelayTime))].0;
    InsertChainState {
        target: target.to_string(),
        effects: InsertKind::ALL
            .iter()
            .map(|k| InsertEffect {
                id: k.id().into(),
                label: k.label().into(),
                on: s.is_on(*k),
                knobs: k
                    .params()
                    .iter()
                    .map(|p| InsertKnob {
                        id: p.id().into(),
                        label: p.label().into(),
                        value: s.get(*p),
                        display: p.display(s.get(*p)),
                        steps: p.steps(),
                    })
                    .collect(),
            })
            .collect(),
        bpm,
        delay_ms: if bpm > 0.0 { beats * 60_000.0 / bpm } else { 0.0 },
        tempo_from: from,
    }
}

/// Switch `effect` ("filter", …) on `target`.
pub fn set_on(e: &mut Engine, target: &str, effect: &str, on: bool) -> Result<(), String> {
    let k = InsertKind::from_id(effect).ok_or_else(|| format!("no insert effect {effect}"))?;
    let t = engine_target(target)?;
    if e.set_insert_on(&t, k, on) { Ok(()) } else { Err(format!("no strip for {target}")) }
}

/// Set knob `param` ("delay.mix", …) on `target` to 0 … 1; the knob's display string back.
pub fn set_param(e: &mut Engine, target: &str, param: &str, v: f64) -> Result<String, String> {
    let p = InsertParam::from_id(param).ok_or_else(|| format!("no insert knob {param}"))?;
    let t = engine_target(target)?;
    if e.set_insert_param(&t, p, v.clamp(0.0, 1.0)) { Ok(p.display(v.clamp(0.0, 1.0))) } else { Err(format!("no strip for {target}")) }
}

/// Every target's knobs, keyed by page target, to keep between launches (the switches are
/// saved too; [`restore`] decides whether they come back on).
pub fn save(e: &Engine) -> JsonValue {
    let mut o = JsonObject::new();
    for t in TARGETS {
        if let Ok(et) = engine_target(t) {
            if let Some(s) = e.insert_settings(&et) {
                o.set(t, s.json());
            }
        }
    }
    JsonValue::Object(o)
}

/// Put [`save`]d chains back: the knobs, and the switches only when `switches` (djandroid's
/// rule: a launch comes up with every insert off). The targets restored.
pub fn restore(e: &mut Engine, v: &JsonValue, switches: bool) -> Vec<String> {
    let mut done = vec![];
    for t in TARGETS {
        let Some(x) = v.get(t) else { continue };
        let Ok(et) = engine_target(t) else { continue };
        let mut s = InsertSettings::from_json(Some(x));
        if !switches {
            for k in InsertKind::ALL {
                s.set_on(k, false);
            }
        }
        if e.apply_insert_settings(&et, &s) {
            done.push(t.to_string());
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_switches_reads_and_round_trips() {
        let mut c = crate::deck::EngineConfig::new(48_000.0);
        c.inputs = vec![WHEEL_STRIP.to_string()];
        let (mut e, _r) = crate::deck::engine(c);
        for t in TARGETS {
            let s = chain_state(&e, t).unwrap();
            assert_eq!(s.effects.len(), InsertKind::ALL.len());
            assert!(!s.any_on());
            set_on(&mut e, t, "delay", true).unwrap();
            let shown = set_param(&mut e, t, "delay.mix", 0.4).unwrap();
            assert!(!shown.is_empty());
            let s = chain_state(&e, t).unwrap();
            assert!(s.any_on(), "{t}");
            assert!(s.tempo_line().starts_with("Delay follows"), "{}", s.tempo_line());
        }
        let saved = save(&e);
        e.inserts_off();
        let back = restore(&mut e, &saved, false);
        assert_eq!(back.len(), 4);
        for t in TARGETS {
            let s = chain_state(&e, t).unwrap();
            assert!(!s.any_on(), "switches come up off");
            let mix = s.effects.iter().flat_map(|x| &x.knobs).find(|k| k.id == "delay.mix").unwrap();
            assert!((mix.value - 0.4).abs() < 1e-9, "the knobs come back");
        }
        assert!(engine_target("C").is_err());
    }
}
