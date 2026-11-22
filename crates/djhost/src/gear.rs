//! Device discovery and hot-plug for the gear in docs/HUB.md. A [`GearRegistry`] keeps the last
//! port scan and turns each new one into attach / detach changes ([`djengine::gear::diff`]).
//! The scan itself comes from [`crate::midi::MidiHub::scan`] (feature `midi`) or, in tests and
//! replays, from any list of names: midir has no setup-changed notification, so hot-plug is
//! polling, about once a second.

use djengine::gear::{
    diff, presence, unknown_ports, Direction, GearPresence, PortChange, PortScan,
};

/// The ports seen last, and what changed.
#[derive(Clone, Debug, Default)]
pub struct GearRegistry {
    last: PortScan,
    /// Changes so far, newest last (capped), for a log.
    pub history: Vec<PortChange>,
    scans: u64,
}

impl GearRegistry {
    const HISTORY: usize = 64;

    pub fn new() -> GearRegistry {
        GearRegistry::default()
    }

    /// Take a new scan: what came and went since the last one.
    pub fn update(&mut self, scan: PortScan) -> Vec<PortChange> {
        let changes = diff(&self.last, &scan);
        self.last = scan;
        self.scans += 1;
        self.history.extend(changes.iter().cloned());
        let over = self.history.len().saturating_sub(Self::HISTORY);
        self.history.drain(..over);
        changes
    }

    /// Scan the platform's ports through `hub` and take the result.
    #[cfg(feature = "midi")]
    pub fn poll(
        &mut self,
        hub: &crate::midi::MidiHub,
    ) -> Result<Vec<PortChange>, crate::HostError> {
        Ok(self.update(hub.scan()?))
    }

    pub fn scan(&self) -> &PortScan {
        &self.last
    }

    pub fn scans(&self) -> u64 {
        self.scans
    }

    /// Every catalogued device, present or not.
    pub fn presence(&self) -> Vec<GearPresence> {
        presence(&self.last)
    }

    /// Ports that belong to no catalogued device.
    pub fn unknown(&self) -> Vec<(Direction, String)> {
        unknown_ports(&self.last)
    }

    /// The output port of device `id`, if it is plugged in (the first, when it has several).
    pub fn output_of(&self, id: &str) -> Option<String> {
        self.presence()
            .into_iter()
            .find(|p| p.gear.id == id)
            .and_then(|p| p.outputs.into_iter().next())
    }

    /// The input ports of every catalogued device that is plugged in.
    pub fn gear_inputs(&self) -> Vec<String> {
        self.presence().into_iter().flat_map(|p| p.inputs).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(i: &[&str], o: &[&str]) -> PortScan {
        PortScan {
            inputs: i.iter().map(|s| s.to_string()).collect(),
            outputs: o.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn hot_plug_sequence() {
        let mut r = GearRegistry::new();
        let first = r.update(scan(&["TD-3", "btw Bus 1"], &["TD-3", "btw Bus 1"]));
        assert_eq!(first.len(), 4, "everything is new on the first scan");
        assert_eq!(r.output_of("td3").as_deref(), Some("TD-3"));
        assert_eq!(r.output_of("pd705"), None);
        // the PD705 is plugged in, the TD-3 pulled
        let c = r.update(scan(&["DrumPad", "btw Bus 1"], &["DrumPad", "btw Bus 1"]));
        let gone: Vec<_> = c
            .iter()
            .filter(|c| !c.attached)
            .map(|c| c.port.as_str())
            .collect();
        let came: Vec<_> = c.iter().filter(|c| c.attached).map(|c| c.gear).collect();
        assert_eq!(gone, ["TD-3", "TD-3"]);
        assert_eq!(came, [Some("pd705"), Some("pd705")]);
        assert_eq!(r.gear_inputs(), ["DrumPad"]);
        assert_eq!(r.unknown().len(), 2);
        assert!(r.update(r.scan().clone()).is_empty());
        assert_eq!(r.scans(), 3);
        assert_eq!(r.history.len(), 8);
    }
}
