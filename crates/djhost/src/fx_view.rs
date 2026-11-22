//! The Effects page's view of the engine: the catalogue's categories and pads (djrust
//! `fx::catalogue`), fired on a target the player picks (Deck A, Deck B, the Wheel's strip, the
//! Master) for 1–4 bars, latched or held, through `Engine::engage` on a deck and
//! `Engine::engage_slot` on a strip with no deck (`host::deck::slot_fx`). Shared by djapp's
//! Effects window and djandroid's Effects tab (djmobile `fx.rs` and its `Player::engage` …,
//! where this started), so a pad does the same thing on both.

use djengine::fx::catalogue::{self, Category, Family};

use crate::deck::slot_fx::slot_entry;
use crate::deck::{Engagement, Engine, TrickSlot};
use crate::insert_view::WHEEL_STRIP;

/// The targets, in the page's order (as the Inserts page's).
pub const TARGETS: [&str; 4] = crate::insert_view::TARGETS;

/// A category of the panel, in panel order.
#[derive(Debug, Clone, PartialEq)]
pub struct FxCategory {
    pub id: String,
    pub label: String,
    /// A one-shot: momentary, no bar length, no latch.
    pub transition: bool,
    /// May stay on until let go.
    pub latchable: bool,
}

/// One pad of the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct FxInfo {
    pub id: String,
    pub label: String,
    /// The [`FxCategory::id`] it sits in.
    pub category: String,
    /// "trick", "net", "master" or "aux": which engine family fires it.
    pub family: String,
    /// Needs a net deck (a song or pan/shape macro).
    pub needs_net: bool,
    /// Plays on a deck only (it works the deck's track): dimmed on the Wheel or the Master.
    pub decks_only: bool,
}

/// What is on, for a pad's light.
#[derive(Debug, Clone, PartialEq)]
pub struct FxActive {
    pub id: String,
    pub latched: bool,
    /// Asked for, waiting for its bar.
    pub pending: bool,
}

pub fn categories() -> Vec<FxCategory> {
    Category::ALL
        .iter()
        .map(|c| FxCategory { id: c.id().into(), label: c.label().into(), transition: c.is_transition(), latchable: c.latches() })
        .collect()
}

pub fn catalogue() -> Vec<FxInfo> {
    catalogue::ENTRIES
        .iter()
        .map(|x| FxInfo {
            id: x.id.into(),
            label: x.label.into(),
            category: x.category.id().into(),
            family: match x.family {
                Family::Trick(_) => "trick",
                Family::Net(_) => "net",
                Family::Master(_) => "master",
                Family::Aux(_) => "aux",
            }
            .into(),
            needs_net: x.needs_net(),
            decks_only: !slot_entry(x.id),
        })
        .collect()
}

/// Where a page target lands: a deck, or a strip with no deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FxTarget {
    Deck(&'static str),
    Slot(TrickSlot),
}

/// "A", "B", "Wheel" (its input strip) or "Master".
pub fn target(e: &Engine, t: &str) -> Result<FxTarget, String> {
    match t {
        _ if t.eq_ignore_ascii_case("wheel") => e
            .input_index(WHEEL_STRIP)
            .map(|i| FxTarget::Slot(TrickSlot::Input(i)))
            .ok_or_else(|| "no input strip for the wheel".to_string()),
        _ if t.eq_ignore_ascii_case("master") => Ok(FxTarget::Slot(TrickSlot::Master)),
        _ if t.eq_ignore_ascii_case("a") => Ok(FxTarget::Deck("A")),
        _ if t.eq_ignore_ascii_case("b") => Ok(FxTarget::Deck("B")),
        _ => Err(format!("no target {t} (A, B, Wheel or Master)")),
    }
}

/// Engage pad `id` on `t`: on its next bar (the Wheel's and the Master's: the lead deck's), for
/// `mode`. Refused with the reason when the category does not allow it, or on the Wheel or the
/// Master when it works a deck's track.
pub fn engage(e: &mut Engine, t: &str, id: &str, mode: Engagement) -> Result<(), String> {
    match target(e, t)? {
        FxTarget::Deck(d) => e.engage(d, id, mode),
        FxTarget::Slot(s) => e.engage_slot(s, id, mode),
    }
}

/// A held pad let go: off at once (a transition runs its own length).
pub fn release(e: &mut Engine, t: &str, id: &str) -> Result<(), String> {
    match target(e, t)? {
        FxTarget::Deck(d) => e.release(d, id),
        FxTarget::Slot(s) => e.release_slot(s, id),
    }
}

/// A latched pad pressed again: off on the next bar.
pub fn unlatch(e: &mut Engine, t: &str, id: &str) -> Result<(), String> {
    match target(e, t)? {
        FxTarget::Deck(d) => e.unlatch(d, id),
        FxTarget::Slot(s) => e.unlatch_slot(s, id),
    }
}

/// What is on, or waiting for its bar, on `t`.
pub fn state(e: &Engine, t: &str) -> Vec<FxActive> {
    let on = match target(e, t) {
        Ok(FxTarget::Deck(d)) => e.fx_state(d),
        Ok(FxTarget::Slot(s)) => e.fx_state_slot(s),
        Err(_) => vec![],
    };
    on.into_iter().map(|s| FxActive { id: s.id, latched: s.latched, pending: s.pending }).collect()
}

/// The filter's knob under a hand: 0 low-pass all the way, 0.5 open, 1 high-pass all the way.
/// It answers at once, on no bar.
pub fn filter_knob(e: &mut Engine, t: &str, v: f64) -> Result<(), String> {
    match target(e, t)? {
        FxTarget::Deck(d) => e.trick_knob(d, djengine::fx::tricks::TrickKind::Filter, v).map(|_| ()),
        FxTarget::Slot(s) => e.slot_knob(s, v),
    }
}

/// PANIC on `t`: a deck's macros, tricks and latches end at once (the aux ones too); on the
/// Wheel or the Master, their effects.
pub fn panic(e: &mut Engine, t: &str) -> Result<(), String> {
    match target(e, t)? {
        FxTarget::Deck(d) => {
            e.panic(d);
            e.cancel_aux_macros(None);
        }
        FxTarget::Slot(s) => e.clear_slot_engagements(s),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_pad_engages_on_the_wheel_and_the_master_and_a_deck_pad_is_refused() {
        let mut c = crate::deck::EngineConfig::new(48_000.0);
        c.inputs = vec![WHEEL_STRIP.to_string()];
        let (mut e, _r) = crate::deck::engine(c);
        let pads = catalogue();
        assert!(!categories().is_empty());
        let slot = pads.iter().find(|p| !p.decks_only && !categories().iter().any(|c| c.id == p.category && c.transition)).expect("an audio trick");
        let deck_only = pads.iter().find(|p| p.decks_only).expect("a deck pad");
        for t in ["Wheel", "Master"] {
            engage(&mut e, t, &slot.id, Engagement::Latch).unwrap();
            assert!(state(&e, t).iter().any(|a| a.id == slot.id), "{t}: lit");
            assert!(engage(&mut e, t, &deck_only.id, Engagement::Bars(1)).is_err(), "{t}: {} refused", deck_only.id);
            filter_knob(&mut e, t, 0.3).unwrap();
            panic(&mut e, t).unwrap();
            assert!(state(&e, t).is_empty(), "{t}: PANIC clears");
        }
        assert!(target(&e, "C").is_err());
    }
}
