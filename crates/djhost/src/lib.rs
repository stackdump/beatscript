//! djhost: the private host modules on top of `djrust::host` (the public live decks in
//! ../djrust). Everything `djrust::host` has is re-exported here, so `djhost::deck`,
//! `djhost::audio` and the rest still resolve; the modules below stay private: MIDI ports and
//! the MIDI clock, controller and gear hubs (PD705, TD-3), HEOS speakers, the control socket,
//! the agent process, publishing and the post tools.

// `deny`, not `forbid`: the exceptions are in `djrust::host::platform`, the OS scheduling and
// timer calls that the MIDI clock needs to wake on time.
#![deny(unsafe_code)]

pub use djrust::host::*;

pub mod agent_process;
pub mod analysis;
pub mod autodj;
pub mod batch;
pub mod beatscript_env;
pub mod beatscript_inputs;
pub mod beatscript_render;
pub mod beatscript_tempo;
pub mod beatscript_test;
pub mod automix_measure;
pub mod control_registry;
pub mod control_socket;
pub mod gear;
pub mod fx_view;
pub mod heos;
pub mod insert_view;
pub mod live_store;
pub mod loop_collections;
pub mod mashups;
pub mod mixxx;
pub mod ontology;
pub mod setlist;
// a recorded set written out as a BeatScript `set` and its take (no wallet: it reads set.json)
pub mod set_program;
pub mod setqueue;
// set recordings sign with the booth identity (djrust-wallet): the `wallet` feature
#[cfg(feature = "wallet")]
pub mod setrec;
#[cfg(feature = "network")]
pub mod net;
pub mod pd705;
pub mod post;
pub mod publish;
pub mod soundpacks;
pub mod perform;
pub mod taper;
pub mod tracks;
pub mod vocals;
pub mod td3;
#[cfg(feature = "wheel")]
pub mod wheel;

#[cfg(feature = "midi")]
use std::fmt;

#[cfg(feature = "midi")]
pub mod clock;
#[cfg(feature = "midi")]
pub mod midi;

/// Every MIDI port the platform reports, sources first.
pub fn midi_ports() -> Result<Vec<MidiPort>, HostError> {
    #[cfg(feature = "midi")]
    {
        let be = |e: &dyn fmt::Display| HostError::Backend(e.to_string());
        let mut out = Vec::new();
        let input = midir::MidiInput::new("djhost").map_err(|e| be(&e))?;
        for p in input.ports() {
            out.push(MidiPort {
                name: input.port_name(&p).map_err(|e| be(&e))?,
                direction: Direction::In,
            });
        }
        let output = midir::MidiOutput::new("djhost").map_err(|e| be(&e))?;
        for p in output.ports() {
            out.push(MidiPort {
                name: output.port_name(&p).map_err(|e| be(&e))?,
                direction: Direction::Out,
            });
        }
        Ok(out)
    }
    #[cfg(not(feature = "midi"))]
    Err(HostError::Disabled("midi"))
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(not(feature = "midi"))]
    fn disabled_backends_say_which_feature() {
        assert_eq!(
            super::midi_ports().unwrap_err().to_string(),
            "djhost was built without the `midi` feature"
        );
    }
}
