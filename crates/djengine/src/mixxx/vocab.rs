//! Mixxx's control vocabulary (`[Group] key`) onto djapp's target registry.
//!
//! Mixxx names every control by a group and a key (`[Channel1]` `play`). Those names are the
//! interface a mapping is written against and Mixxx documents them publicly; this table is
//! ours, built from that vocabulary and from what djrust's binding layer can do
//! (`control::registry`). Nothing here comes from a mapping file.
//!
//! A control either **maps** (to a registry target, exactly or approximately), is
//! **unsupported** (djrust has nothing to put it on, with the reason), or is **script** (an
//! XML control with `<script-binding/>`: its `key` is a script function, which needs the
//! script engine).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use djrust::control::surface::DJActionKind;

/// How a control must be driven for the target to make sense.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    /// A button (acts on press).
    Trigger,
    /// A fader or knob, a position.
    Continuous,
}

/// A registry target, as a binding's fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub action: DJActionKind,
    /// "A" / "B", or None for the deck-less (the mixer, the master).
    pub deck: Option<String>,
    pub slot: Option<i64>,
    pub param: Option<String>,
    pub drive: Drive,
    /// Mixxx does something here that the target only resembles: what differs.
    pub approx: Option<&'static str>,
}

impl Target {
    fn new(action: DJActionKind, deck: Option<&str>, drive: Drive) -> Target {
        Target { action, deck: deck.map(str::to_string), slot: None, param: None, drive, approx: None }
    }
    fn approx(mut self, why: &'static str) -> Target {
        self.approx = Some(why);
        self
    }
    fn slot(mut self, s: i64) -> Target {
        self.slot = Some(s);
        self
    }
    fn param(mut self, p: &str) -> Target {
        self.param = Some(p.to_string());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Translation {
    Maps(Target),
    Unsupported(String),
    Script,
}

fn no(reason: &str) -> Translation {
    Translation::Unsupported(reason.to_string())
}

/// `[Channel2]` → ("A" / "B"), or Err(reason) for a deck djrust does not have.
fn channel_deck(group: &str) -> Option<Result<&'static str, String>> {
    let n = group.strip_prefix("[Channel")?.strip_suffix(']')?.parse::<u32>().ok()?;
    Some(match n {
        1 => Ok("A"),
        2 => Ok("B"),
        _ => Err(format!("djrust has two decks; deck {n} has nowhere to go")),
    })
}

/// `[QuickEffectRack1_[Channel1]]` / `[EqualizerRack1_[Channel2]_Effect1]` → the channel's deck.
fn inner_channel(group: &str, prefix: &str) -> Option<Result<&'static str, String>> {
    let rest = group.strip_prefix(prefix)?;
    let end = rest.find(']')?;
    channel_deck(&format!("{}]", &rest[..end]))
}

/// `hotcue_3_activate` → (3, "activate").
fn hotcue(key: &str) -> Option<(i64, &str)> {
    let rest = key.strip_prefix("hotcue_")?;
    let us = rest.find('_')?;
    Some((rest[..us].parse().ok()?, &rest[us + 1..]))
}

/// What `[group] key` is in djrust.
pub fn translate(group: &str, key: &str, script_binding: bool) -> Translation {
    use DJActionKind as K;
    use Drive::{Continuous as Cont, Trigger as Trig};
    if script_binding {
        return Translation::Script;
    }
    // a channel's own controls
    if let Some(deck) = channel_deck(group) {
        let deck = match deck {
            Ok(d) => d,
            Err(why) => return Translation::Unsupported(why),
        };
        let d = Some(deck);
        if let Some((n, op)) = hotcue(key) {
            let slot = n - 1;
            return match op {
                "activate" => Translation::Maps(Target::new(K::Hotcue, d, Trig).slot(slot)),
                "clear" => Translation::Maps(Target::new(K::Clearcue, d, Trig).slot(slot)),
                "set" | "goto" | "gotoandplay" | "gotoandstop" | "position" => Translation::Maps(
                    Target::new(K::Hotcue, d, Trig).slot(slot).approx("sets the cue if empty, else jumps to it"),
                ),
                _ => no("hot cue controls other than activate / clear / set / goto"),
            };
        }
        return match key {
            "volume" => Translation::Maps(Target::new(K::Volume, d, Cont)),
            "pregain" => Translation::Maps(Target::new(K::Gain, d, Cont)),
            "filterLow" => Translation::Maps(Target::new(K::EqLow, d, Cont)),
            "filterMid" => Translation::Maps(Target::new(K::EqMid, d, Cont)),
            "filterHigh" => Translation::Maps(Target::new(K::EqHigh, d, Cont)),
            "rate" => Translation::Maps(Target::new(K::Pitch, d, Cont)),
            "play" => Translation::Maps(Target::new(K::Play, d, Trig)),
            "play_stutter" => Translation::Maps(Target::new(K::Play, d, Trig).approx("plays or pauses; no stutter")),
            "cue_default" | "cue_gotoandplay" | "cue_simple" | "cue_gotoandstop" => {
                Translation::Maps(Target::new(K::Cue, d, Trig))
            }
            "cue_set" => no("setting the main cue (djrust's cue is the deck's start)"),
            "pfl" => Translation::Maps(Target::new(K::Pfl, d, Trig)),
            "sync_enabled" | "beatsync" => Translation::Maps(Target::new(K::Sync, d, Trig)),
            "sync_mode" => Translation::Maps(Target::new(K::Sync, d, Trig).approx("one-shot sync, not Mixxx's follower / leader")),
            "beatsync_phase" | "beatsync_tempo" => {
                Translation::Maps(Target::new(K::Sync, d, Trig).approx("syncs tempo and phase together"))
            }
            "loop_in" => Translation::Maps(Target::new(K::LoopIn, d, Trig)),
            "loop_out" => Translation::Maps(Target::new(K::LoopOut, d, Trig)),
            "loop_halve" => Translation::Maps(Target::new(K::LoopHalve, d, Trig)),
            "loop_double" => Translation::Maps(Target::new(K::LoopDouble, d, Trig)),
            "LoadSelectedTrack" | "LoadSelectedTrackAndPlay" => {
                Translation::Maps(Target::new(K::NextTrack, d, Trig).approx("loads the next track, not the one selected in a library"))
            }
            "start" | "start_play" | "start_stop" => {
                Translation::Maps(Target::new(K::Rewind, d, Trig).approx("jumps to the start; does not also play / stop"))
            }
            "quantize" => Translation::Maps(Target::new(K::Quantize, d, Trig).approx("cycles the quantise step")),
            "jog" | "wheel" | "scratch2" | "scratch2_enable" | "scratch" | "scratch_position" | "scratch_position_enable" => {
                no("jog and scratch: needs the scripting layer (a wheel is a script's work); djrust's own jog layer is a separate profile")
            }
            "keylock" => no("key lock"),
            "reverse" | "reverseroll" | "slip_enabled" => no("reverse / slip"),
            "orientation" => no("crossfader assignment per deck"),
            "mute" => no("channel mute"),
            "rate_perm_up" | "rate_perm_down" | "rate_perm_up_small" | "rate_perm_down_small" | "rate_temp_up" | "rate_temp_down" => {
                no("tempo nudge buttons")
            }
            "rateRange" => no("the tempo range (djrust's is a deck setting)"),
            "eject" => no("eject"),
            "playposition" | "back" | "fwd" => no("seeking"),
            _ if key.starts_with("beatloop") || key.starts_with("beatlooproll") => no(
                "fixed-length beat loops (the binding layer has loop in / out / halve / double)",
            ),
            _ if key.starts_with("loop_") || key.starts_with("reloop") => no("loop controls beyond in / out / halve / double"),
            _ if key.starts_with("filter") => no("EQ kills"),
            _ if key.starts_with("beats_") || key.starts_with("beatjump") => no("beat jumps"),
            _ if key.starts_with("pitch") || key.starts_with("rate_") => no("pitch / rate variants"),
            _ => no("no djrust target for this control"),
        };
    }
    if group == "[Master]" {
        return match key {
            "crossfader" => Translation::Maps(Target::new(K::Crossfader, None, Cont)),
            "volume" | "gain" => Translation::Maps(Target::new(K::Master, None, Cont)),
            "headMix" => Translation::Maps(Target::new(K::HeadphoneMix, None, Cont)),
            "headVolume" | "headGain" => Translation::Maps(Target::new(K::HeadphoneLevel, None, Cont)),
            "balance" => no("master balance"),
            _ => no("no djrust target for this master control"),
        };
    }
    if group == "[AutoDJ]" {
        return match key {
            "enabled" => Translation::Maps(Target::new(K::AutoMix, None, Trig)),
            "fade_now" => Translation::Maps(Target::new(K::AutoMixNow, None, Trig)),
            _ => no("no djrust target for this Auto DJ control"),
        };
    }
    if let Some(deck) = inner_channel(group, "[QuickEffectRack1_") {
        return match deck {
            Err(why) => Translation::Unsupported(why),
            Ok(d) => match key {
                "super1" => Translation::Maps(Target::new(K::Filter, Some(d), Cont)),
                _ => no("quick effect controls other than the filter knob"),
            },
        };
    }
    if let Some(deck) = inner_channel(group, "[EqualizerRack1_") {
        return match deck {
            Err(why) => Translation::Unsupported(why),
            Ok(d) => match key {
                "parameter1" => Translation::Maps(Target::new(K::EqLow, Some(d), Cont)),
                "parameter2" => Translation::Maps(Target::new(K::EqMid, Some(d), Cont)),
                "parameter3" => Translation::Maps(Target::new(K::EqHigh, Some(d), Cont)),
                _ => no("EQ kills and parameters beyond low / mid / high"),
            },
        };
    }
    if let Some(unit) = group.strip_prefix("[EffectRack1_EffectUnit").and_then(|r| r.strip_suffix(']')) {
        // an effect unit is a rack of three in Mixxx, a per-deck unit here: unit 1 is deck A's,
        // unit 2 deck B's
        let d = match unit.parse::<u32>() {
            Ok(1) => "A",
            Ok(2) => "B",
            _ => return no("djrust has one FX unit a deck; effect units 3 and 4 have nowhere to go"),
        };
        return match key {
            "mix" => Translation::Maps(
                Target::new(K::ReverbWet, Some(d), Cont).approx("the deck's reverb send, not the unit's wet / dry"),
            ),
            "super1" => Translation::Maps(Target::new(K::FxUnit, Some(d), Cont).param("filter").approx("the deck's filter sweep")),
            _ => no("effect chain slots, parameters and enables (djrust's FX are a fixed unit)"),
        };
    }
    if group.starts_with("[Sampler") {
        return no("samplers");
    }
    if group == "[Library]" || group == "[Playlist]" {
        return no("library browsing");
    }
    if group.starts_with("[Microphone") || group.starts_with("[Auxiliary") {
        return no("microphone and auxiliary inputs (djrust's inputs are strips of their own)");
    }
    if group.starts_with("[Skin]") || group.starts_with("[Recording]") || group.starts_with("[Controls]") {
        return no("Mixxx's own window and recording");
    }
    no("no djrust target for this group")
}

/// Every distinct `(group, key)` this table maps, for the docs and tests: the channel-1 ones,
/// plus the master and the rest.
pub fn mapped_examples() -> Vec<(&'static str, &'static str)> {
    alloc::vec![
        ("[Channel1]", "volume"),
        ("[Channel1]", "pregain"),
        ("[Channel1]", "filterLow"),
        ("[Channel1]", "filterMid"),
        ("[Channel1]", "filterHigh"),
        ("[Channel1]", "rate"),
        ("[Channel1]", "play"),
        ("[Channel1]", "cue_default"),
        ("[Channel1]", "pfl"),
        ("[Channel1]", "sync_enabled"),
        ("[Channel1]", "hotcue_1_activate"),
        ("[Channel1]", "hotcue_1_clear"),
        ("[Channel1]", "loop_in"),
        ("[Channel1]", "loop_out"),
        ("[Channel1]", "loop_halve"),
        ("[Channel1]", "loop_double"),
        ("[Channel1]", "LoadSelectedTrack"),
        ("[Master]", "crossfader"),
        ("[Master]", "volume"),
        ("[Master]", "headMix"),
        ("[Master]", "headGain"),
        ("[QuickEffectRack1_[Channel1]]", "super1"),
        ("[EqualizerRack1_[Channel1]_Effect1]", "parameter1"),
        ("[EffectRack1_EffectUnit1]", "mix"),
        ("[AutoDJ]", "enabled"),
    ]
}
