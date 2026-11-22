//! MIDI ports via midir: the cross-platform stand-in for `MIDIIn.swift` / `MIDIOut`
//! (CoreMIDI on macOS, the ALSA sequencer on Linux, WinMM on Windows).
//!
//! CoreMIDI differences worth knowing: midir has no setup-changed notification, so hot-plug
//! is polling (`MidiHub::connect_matching` again, `drop_missing`), and port names are the
//! backend's (ALSA: `"APC mini mk2:APC mini mk2 Control 20:0"`, CoreMIDI: the device's
//! display name). The profile patterns match both. Timestamps are midir's microseconds
//! since the connection opened, not CoreMIDI host time.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::controllers::normalize_midi;
use crate::HostError;

fn be(e: impl std::fmt::Display) -> HostError {
    HostError::Backend(e.to_string())
}

/// One MIDI 1.0 channel-voice message (`MIDIMessage`).
#[derive(Clone, Debug, PartialEq)]
pub struct MidiMessage {
    pub port: String,
    pub bytes: [u8; 3],
    pub time_us: u64,
}

/// Any MIDI message as it arrived, every byte: clock, transport and SysEx included (the
/// monitor's feed). `at` is the host time the callback ran, comparable across ports.
#[derive(Clone, Debug, PartialEq)]
pub struct RawMidi {
    pub port: String,
    pub bytes: Vec<u8>,
    pub at: std::time::Instant,
}

/// Called from midir's thread after a message is queued (see [`MidiHub::set_waker`]).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// What a midir callback does with one message: the monitor's copy, the normalised message for
/// the router, and the waker (so the thread that drains the channel runs now, not at its next
/// timer tick). The body of the closure `connect_matching` hands to midir, apart from the
/// connection so a test can run it.
fn deliver(
    tx: &Sender<MidiMessage>,
    raw_tx: Option<&Sender<RawMidi>>,
    wake: Option<&Waker>,
    port: &str,
    time_us: u64,
    bytes: &[u8],
) {
    if let Some(raw) = raw_tx {
        let _ = raw.send(RawMidi {
            port: port.to_string(),
            bytes: bytes.to_vec(),
            at: std::time::Instant::now(),
        });
    }
    if let Some(b) = normalize_midi(bytes) {
        let _ = tx.send(MidiMessage {
            port: port.to_string(),
            bytes: b,
            time_us,
        });
        // only what the router will read wakes the host: a clock (24 a beat) or a SysEx for
        // the monitor waits for the next tick
        if let Some(w) = wake {
            w();
        }
    }
}

/// Input connections to every wanted source, and output connections for LED feedback.
pub struct MidiHub {
    client: String,
    tx: Sender<MidiMessage>,
    raw_tx: Option<Sender<RawMidi>>,
    wake: Option<Waker>,
    inputs: BTreeMap<String, MidiInputConnection<()>>,
    outputs: BTreeMap<String, MidiOutputConnection>,
}

impl MidiHub {
    /// A hub and the channel its messages arrive on. The callbacks run on midir's threads:
    /// they only normalise and send.
    pub fn new(client: &str) -> Result<(MidiHub, Receiver<MidiMessage>), HostError> {
        MidiInput::new(client).map_err(be)?; // fail early when there is no backend
        let (tx, rx) = channel();
        Ok((
            MidiHub {
                client: client.to_string(),
                tx,
                raw_tx: None,
                wake: None,
                inputs: BTreeMap::new(),
                outputs: BTreeMap::new(),
            },
            rx,
        ))
    }

    /// A hub that also hands every message, whole, to a second channel (the MIDI monitor).
    #[allow(clippy::type_complexity)]
    pub fn with_raw(
        client: &str,
    ) -> Result<(MidiHub, Receiver<MidiMessage>, Receiver<RawMidi>), HostError> {
        let (mut hub, rx) = MidiHub::new(client)?;
        let (raw_tx, raw_rx) = channel();
        hub.raw_tx = Some(raw_tx);
        Ok((hub, rx, raw_rx))
    }

    /// Call `wake` from the MIDI callback after each message is queued, for connections made
    /// from now on. A host whose loop is driven by something else (an egui app repaints on a
    /// timer, and the system may throttle a window nobody can see) passes
    /// `egui::Context::request_repaint` here, so a controller is answered when it moves rather
    /// than at the next tick. It runs on midir's thread: it must be cheap and never block.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    /// Every input and output port the platform reports right now.
    pub fn scan(&self) -> Result<djengine::gear::PortScan, HostError> {
        Ok(djengine::gear::PortScan {
            inputs: self.input_names()?,
            outputs: self.output_names()?,
        })
    }

    /// Names of every input port the platform reports right now.
    pub fn input_names(&self) -> Result<Vec<String>, HostError> {
        let input = MidiInput::new(&self.client).map_err(be)?;
        input
            .ports()
            .iter()
            .map(|p| input.port_name(p).map_err(be))
            .collect()
    }

    /// Names of every output port.
    pub fn output_names(&self) -> Result<Vec<String>, HostError> {
        let out = MidiOutput::new(&self.client).map_err(be)?;
        out.ports()
            .iter()
            .map(|p| out.port_name(p).map_err(be))
            .collect()
    }

    /// Connect every input whose name satisfies `wanted` and is not connected yet. Returns
    /// the names newly connected (`MIDIIn.connect(where:)`, minus what it already had).
    pub fn connect_matching(
        &mut self,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<String>, HostError> {
        let probe = MidiInput::new(&self.client).map_err(be)?;
        let mut added = Vec::new();
        for p in probe.ports() {
            let name = probe.port_name(&p).map_err(be)?;
            if self.inputs.contains_key(&name) || !wanted(&name) {
                continue;
            }
            // `connect` consumes the MidiInput, so each port gets its own
            let input = MidiInput::new(&self.client).map_err(be)?;
            let port = input
                .ports()
                .into_iter()
                .find(|q| input.port_name(q).ok().as_deref() == Some(&name))
                .ok_or_else(|| be(format!("{name} went away")))?;
            let tx = self.tx.clone();
            let raw_tx = self.raw_tx.clone();
            let wake = self.wake.clone();
            let from = name.clone();
            let conn = input
                .connect(
                    &port,
                    "in",
                    move |time_us, bytes, _| {
                        deliver(&tx, raw_tx.as_ref(), wake.as_ref(), &from, time_us, bytes);
                    },
                    (),
                )
                .map_err(be)?;
            self.inputs.insert(name.clone(), conn);
            added.push(name);
        }
        Ok(added)
    }

    /// Forget connections to ports that are no longer present (hot-unplug).
    pub fn drop_missing(&mut self) -> Result<Vec<String>, HostError> {
        let present = self.input_names()?;
        let gone: Vec<String> = self
            .inputs
            .keys()
            .filter(|n| !present.contains(n))
            .cloned()
            .collect();
        for n in &gone {
            self.inputs.remove(n);
            self.outputs.remove(n);
        }
        let outs = self.output_names()?;
        self.outputs.retain(|n, _| outs.contains(n));
        Ok(gone)
    }

    pub fn connected(&self) -> impl Iterator<Item = &str> {
        self.inputs.keys().map(String::as_str)
    }

    /// Send whole messages of any length (clock, transport, SysEx, notes) to `port`, opening
    /// it on first use.
    pub fn send_bytes(&mut self, port: &str, messages: &[&[u8]]) -> Result<(), HostError> {
        if messages.is_empty() {
            return Ok(());
        }
        self.open_output(port)?;
        let conn = self.outputs.get_mut(port).unwrap();
        for m in messages {
            conn.send(m).map_err(be)?;
        }
        Ok(())
    }

    fn open_output(&mut self, port: &str) -> Result<(), HostError> {
        if self.outputs.contains_key(port) {
            return Ok(());
        }
        let out = MidiOutput::new(&self.client).map_err(be)?;
        let p = out
            .ports()
            .into_iter()
            .find(|q| out.port_name(q).ok().as_deref() == Some(port))
            .ok_or_else(|| be(format!("no MIDI output named {port:?}")))?;
        let conn = out.connect(&p, "out").map_err(be)?;
        self.outputs.insert(port.to_string(), conn);
        Ok(())
    }

    /// Close the output to `port` (it went away, or another process needs it).
    pub fn close_output(&mut self, port: &str) {
        self.outputs.remove(port);
    }

    /// Send 3-byte channel-voice messages (LED feedback) to the output port named `port`,
    /// opening it on first use. Controllers expose their output under the same name as their
    /// input; an output that does not exist is an error the caller may ignore (not every
    /// surface has LEDs).
    pub fn send(&mut self, port: &str, messages: &[[u8; 3]]) -> Result<(), HostError> {
        if messages.is_empty() {
            return Ok(());
        }
        self.open_output(port)?;
        let conn = self.outputs.get_mut(port).unwrap();
        for m in messages {
            conn.send(m).map_err(be)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// What the callback does for a controller message: the router's channel and the monitor's
    /// get it, and the waker runs once; for a clock byte only the monitor's copy is made and
    /// nothing is woken.
    #[test]
    fn a_controller_message_wakes_the_host_and_a_clock_does_not() {
        let (tx, rx) = channel();
        let (raw_tx, raw_rx) = channel();
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let wake: Waker = Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        deliver(&tx, Some(&raw_tx), Some(&wake), "APC", 7, &[0x90, 60, 100]);
        let m = rx.try_recv().expect("the router's copy");
        assert_eq!(
            (m.port.as_str(), m.bytes, m.time_us),
            ("APC", [0x90, 60, 100], 7)
        );
        assert_eq!(raw_rx.try_recv().unwrap().bytes, vec![0x90, 60, 100]);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        deliver(&tx, Some(&raw_tx), Some(&wake), "TD-3", 8, &[0xF8]);
        assert!(rx.try_recv().is_err(), "a clock is not for the router");
        assert_eq!(raw_rx.try_recv().unwrap().bytes, vec![0xF8]);
        assert_eq!(woken.load(Ordering::SeqCst), 1, "and wakes nothing");
        // no waker, no monitor: the router's copy alone, as before
        deliver(&tx, None, None, "APC", 9, &[0xB0, 1, 2]);
        assert_eq!(rx.try_recv().unwrap().bytes, [0xB0, 1, 2]);
    }
}
