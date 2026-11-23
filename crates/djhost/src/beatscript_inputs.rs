//! The controllers' side of BeatScript's performed inputs (F13, docs/beatscript/ext-perform.md): which control a program's `knob(3)`,
//! `pad(5)`, `fader(A)`, `held(key)` and `held(c4)` read, and the value it has.
//!
//! The values are the ones the bindings see: every message from a plugged-in controller, a drawn control or the computer
//! keyboard takes one road (`djapp::hwio::midi_in`), and its decoded value is noted in the [`Activity`] the Map window draws
//! from. This module only *reads* that (`Activity::value`, and the held flag through `glow`), so the Aux and APC layouts, the
//! bindings and what a control is bound to are unchanged: a knob that is bound to a reverb still moves the reverb, and a program
//! that names the same knob reads where it is.
//!
//! Which control a name means is a rule over the devices that are attached (in the order they were attached):
//!
//! | input | the control |
//! |---|---|
//! | `knob(N)` | the N-th control (1 is the first) of the first attached device's `knobs` group (the MPK mini's eight), else of its `faders` group (the APC mini's nine) |
//! | `pad(N)` | the N-th control of the first attached device's note pad group: `pads` (the APC's 64, the PD705's 9) or `padNotes` (the MPK's sixteen). While it is held, its velocity (at least 1/127); else 0 |
//! | `fader(A)` | deck A's channel fader: the mixer's volume, whichever control moved it |
//! | `held(key)` | the lowest key held of the first attached device with a 128-note `keys` group (the MPK), over 127; 0 when none |
//! | `held(c4)` | 1 while that note is held on the same keys, else 0 |
//!
//! A device that is attached but untouched reads 0 for a pad and a key, and 1/2 for a knob (a knob nothing has moved is at home, in the
//! middle: the value `check` reads it as, so a program that opens a filter with it does not start shut); a name no attached device has
//! a control for is None (the program reads the input's home and its words say no controller is connected).

use std::sync::Arc;

use djengine::control::hwview::Activity;
use djengine::control::surface::{ControlType, Group, SurfaceProfile};
use djlive::perform::{note_number, Input, InputBank};

/// What the controllers say, read at one moment.
pub struct Board<'a> {
    /// The last value of every control, and whether it is held (`djapp`'s `App::activity`).
    pub activity: &'a Activity,
    /// The profiles of the devices that are attached, in the order they were attached.
    pub devices: &'a [Arc<SurfaceProfile>],
    /// A deck's channel fader (`A`, `B`), 0..1: the mixer's own volume.
    pub fader: &'a dyn Fn(&str) -> Option<f64>,
}

fn size(g: &Group) -> usize {
    g.control_numbers().len()
}

/// A control is held while its note is down: [`Activity::glow`] is 1 for ever only for a held control (a control that moved
/// fades to 0 in 0.6 s).
fn held(a: &Activity, device: &str, group: &str, index: usize) -> bool {
    a.glow(device, group, index, f64::INFINITY) >= 1.0
}

impl Board<'_> {
    fn find(&self, ids: &[&str], want: impl Fn(&Group) -> bool) -> Option<(&SurfaceProfile, &Group)> {
        for id in ids {
            for d in self.devices {
                if let Some(g) = d.groups.iter().find(|g| g.id == *id && want(g)) {
                    return Some((&**d, g));
                }
            }
        }
        None
    }

    fn knobs(&self) -> Option<(&SurfaceProfile, &Group)> {
        self.find(&["knobs", "faders"], |g| size(g) > 0)
    }

    fn pads(&self) -> Option<(&SurfaceProfile, &Group)> {
        // (each device in turn: its `pads` before its `padNotes`; the MPK's `pads` are program changes, which have no release)
        for d in self.devices {
            for id in ["pads", "padNotes"] {
                if let Some(g) = d.groups.iter().find(|g| g.id == id && g.kind == ControlType::Note && size(g) > 0) {
                    return Some((&**d, g));
                }
            }
        }
        None
    }

    fn keys(&self) -> Option<(&SurfaceProfile, &Group)> {
        self.find(&["keys"], |g| g.kind == ControlType::Note && size(g) == 128)
    }

    /// How many knobs and pads the connected controllers have: what a program's `knob(N)` and `pad(N)` are checked against. A
    /// kind no device has stays the default, so a program may still be written before the gear is plugged in.
    pub fn bank(&self) -> InputBank {
        InputBank { knobs: self.knobs().map_or(InputBank::DEFAULT.knobs, |(_, g)| size(g) as u32), pads: self.pads().map_or(InputBank::DEFAULT.pads, |(_, g)| size(g) as u32) }
    }

    /// Whether any attached controller answers for this kind of input.
    pub fn connected(&self, kind: &str) -> bool {
        match kind {
            "knob" => self.knobs().is_some(),
            "pad" => self.pads().is_some(),
            "held" => self.keys().is_some(),
            "fader" => true,
            _ => false,
        }
    }

    /// One input's value 0..1, as `LiveHost::input` asks: `kind` is `knob`, `pad`, `fader` or `held`, `id` the number, the deck,
    /// `key` or the note. None when no attached controller has such a control (or the name is not one).
    pub fn read(&self, kind: &str, id: &str) -> Option<f64> {
        match kind {
            "knob" => {
                let n: usize = id.parse().ok().filter(|n| *n >= 1)?;
                let (d, g) = self.knobs()?;
                if n > size(g) {
                    return None;
                }
                let home = Input::Knob(n as u32).home();
                Some(self.activity.value(&d.name, &g.id, n - 1).unwrap_or(home).clamp(0.0, 1.0))
            }
            "pad" => {
                let n: usize = id.parse().ok().filter(|n| *n >= 1)?;
                let (d, g) = self.pads()?;
                if n > size(g) {
                    return None;
                }
                if held(self.activity, &d.name, &g.id, n - 1) {
                    Some(self.activity.value(&d.name, &g.id, n - 1).unwrap_or(1.0).clamp(1.0 / 127.0, 1.0))
                } else {
                    Some(0.0)
                }
            }
            "fader" => (self.fader)(id),
            "held" => {
                let (d, g) = self.keys()?;
                if id == "key" {
                    let lowest = (0..size(g)).find(|i| held(self.activity, &d.name, &g.id, *i));
                    return Some(lowest.map_or(0.0, |i| i as f64 / 127.0));
                }
                let note = usize::from(note_number(id)?);
                Some(if held(self.activity, &d.name, &g.id, note) { 1.0 } else { 0.0 })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::control::profiles;

    const MPK: &str = "MPK Mini Mk II";
    const APC: &str = "APC mini mk2";

    fn board<'a>(a: &'a Activity, devices: &'a [Arc<SurfaceProfile>], fader: &'a dyn Fn(&str) -> Option<f64>) -> Board<'a> {
        Board { activity: a, devices, fader }
    }

    fn volume(d: &str) -> Option<f64> {
        match d {
            "A" => Some(0.8),
            "B" => Some(0.25),
            _ => None,
        }
    }

    #[test]
    fn the_names_of_the_profiles_are_the_ones_this_module_reads() {
        assert_eq!(profiles::mpk_mini_mk2().name, MPK);
        assert_eq!(profiles::apc_mini_mk2().name, APC);
    }

    #[test]
    fn a_knob_reads_the_value_its_control_last_had() {
        let mut a = Activity::new();
        let devs = [profiles::mpk_mini_mk2()];
        // untouched: attached, a knob nothing has moved rests in the middle (a pad and a key read 0)
        assert_eq!(board(&a, &devs, &volume).read("knob", "3"), Some(0.5));
        a.moved(MPK, "knobs", 2, 0.25, false, 0.0);
        assert_eq!(board(&a, &devs, &volume).read("knob", "3"), Some(0.25));
        a.moved(MPK, "knobs", 2, 0.0, false, 0.0);
        assert_eq!(board(&a, &devs, &volume).read("knob", "3"), Some(0.0), "a knob that was turned to 0 is at 0");
        assert_eq!(board(&a, &devs, &volume).read("knob", "1"), Some(0.5));
        assert_eq!(board(&a, &devs, &volume).read("pad", "1"), Some(0.0));
        // the MPK has eight
        assert_eq!(board(&a, &devs, &volume).read("knob", "8"), Some(0.5));
        assert_eq!(board(&a, &devs, &volume).read("knob", "9"), None);
        assert_eq!(board(&a, &devs, &volume).read("knob", "0"), None);
        assert_eq!(board(&a, &devs, &volume).bank().knobs, 8);
    }

    #[test]
    fn with_no_mpk_the_apc_faders_are_the_knob_bank_and_the_apc_pads_are_the_pads() {
        let mut a = Activity::new();
        let devs = [profiles::apc_mini_mk2()];
        a.moved(APC, "faders", 0, 1.0, false, 0.0);
        a.moved(APC, "faders", 8, 0.25, false, 0.0);
        let b = board(&a, &devs, &volume);
        assert_eq!(b.read("knob", "1"), Some(1.0));
        assert_eq!(b.read("knob", "9"), Some(0.25), "faders 1-8 and the master");
        assert_eq!(b.bank(), InputBank { knobs: 9, pads: 64 });
        // a pad: its velocity while held, 0 after
        a.moved(APC, "pads", 4, 100.0 / 127.0, true, 0.0);
        let b = board(&a, &devs, &volume);
        assert_eq!(b.read("pad", "5"), Some(100.0 / 127.0));
        assert_eq!(b.read("pad", "6"), Some(0.0));
        a.moved(APC, "pads", 4, 0.0, false, 1.0);
        assert_eq!(board(&a, &devs, &volume).read("pad", "5"), Some(0.0));
        assert_eq!(board(&a, &devs, &volume).read("pad", "65"), None);
    }

    #[test]
    fn the_first_attached_device_with_the_control_answers() {
        let mut a = Activity::new();
        // the MPK first: its knobs; the APC's faders are not the bank while it is there
        let devs = [profiles::mpk_mini_mk2(), profiles::apc_mini_mk2()];
        a.moved(MPK, "knobs", 0, 0.75, false, 0.0);
        a.moved(APC, "faders", 0, 0.1, false, 0.0);
        assert_eq!(board(&a, &devs, &volume).read("knob", "1"), Some(0.75));
        // the MPK's pads are `padNotes` (16): the APC's 64 are `pads`: the first device with either answers, the MPK
        assert_eq!(board(&a, &devs, &volume).bank(), InputBank { knobs: 8, pads: 16 });
        a.moved(MPK, "padNotes", 0, 1.0, true, 0.0);
        assert_eq!(board(&a, &devs, &volume).read("pad", "1"), Some(1.0));
        // the other way round: the APC's pads
        let devs = [profiles::apc_mini_mk2(), profiles::mpk_mini_mk2()];
        assert_eq!(board(&a, &devs, &volume).bank(), InputBank { knobs: 8, pads: 64 }, "knobs: the first device with a `knobs` group");
    }

    #[test]
    fn held_reads_the_lowest_key_down_and_a_named_key() {
        let mut a = Activity::new();
        let devs = [profiles::mpk_mini_mk2()];
        assert_eq!(board(&a, &devs, &volume).read("held", "key"), Some(0.0));
        assert_eq!(board(&a, &devs, &volume).read("held", "c4"), Some(0.0));
        a.moved(MPK, "keys", 64, 0.8, true, 0.0);
        a.moved(MPK, "keys", 60, 0.5, true, 0.0);
        let b = board(&a, &devs, &volume);
        assert_eq!(b.read("held", "key"), Some(60.0 / 127.0), "the lowest of the two");
        assert_eq!(b.read("held", "c4"), Some(1.0), "middle C is 60");
        assert_eq!(b.read("held", "e4"), Some(1.0));
        assert_eq!(b.read("held", "d4"), Some(0.0));
        // released: no longer held, whatever its last velocity was
        a.moved(MPK, "keys", 60, 0.0, false, 1.0);
        let b = board(&a, &devs, &volume);
        assert_eq!(b.read("held", "c4"), Some(0.0));
        assert_eq!(b.read("held", "key"), Some(64.0 / 127.0));
        // a name that is not a note
        assert_eq!(b.read("held", "nope"), None);
        // no keys attached
        let none = [profiles::apc_mini_mk2()];
        assert_eq!(board(&a, &none, &volume).read("held", "key"), None);
        assert!(!board(&a, &none, &volume).connected("held"));
    }

    #[test]
    fn a_fader_is_the_decks_own_and_nothing_attached_answers_for_what_is_not_there() {
        let a = Activity::new();
        let b = board(&a, &[], &volume);
        assert_eq!(b.read("fader", "A"), Some(0.8));
        assert_eq!(b.read("fader", "B"), Some(0.25));
        assert_eq!(b.read("fader", "C"), None);
        assert_eq!(b.read("knob", "1"), None);
        assert_eq!(b.read("pad", "1"), None);
        assert_eq!(b.read("bogus", "1"), None);
        assert_eq!(b.bank(), InputBank::DEFAULT, "no device: the default bank");
    }

    #[test]
    fn every_shipped_profile_has_the_controls_the_rules_name() {
        // the rule names groups by id: a profile that changes one is found here
        let mpk = profiles::mpk_mini_mk2();
        assert_eq!(mpk.groups.iter().find(|g| g.id == "knobs").map(size), Some(8));
        assert_eq!(mpk.groups.iter().find(|g| g.id == "keys").map(size), Some(128));
        assert_eq!(mpk.groups.iter().find(|g| g.id == "padNotes").map(size), Some(16));
        let apc = profiles::apc_mini_mk2();
        assert_eq!(apc.groups.iter().find(|g| g.id == "pads").map(size), Some(64));
        assert_eq!(apc.groups.iter().find(|g| g.id == "faders").map(size), Some(9));
        let pd = profiles::pd705();
        assert_eq!(pd.groups.iter().find(|g| g.id == "pads").map(size), Some(9));
    }
}
