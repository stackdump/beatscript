//! The slice of BeatScript a recovered program is written in: a tempo, drum parts on deck A,
//! a tonal bass and a chord pad on deck B, one bar of 16 steps repeated to `end at bar N`.
//! (Sections, lets, repeats and evolve are for later rungs; `same_as` compares nets, so the
//! longhand program is a valid answer.)

use std::collections::BTreeMap;

pub const STEPS: usize = 16;
pub const DRUMS: [&str; 4] = ["kick", "snare", "hihat", "clap"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vel {
    Default,
    Fixed(u8),
    Range(u8, u8),
}

impl Vel {
    fn param(self) -> String {
        match self {
            Vel::Default => String::new(),
            Vel::Fixed(v) => format!(" vel={v}"),
            Vel::Range(a, b) => format!(" vel={a}..{b}"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub bpm: f64,
    pub bars: u32,
    /// Drum role -> which steps hit.
    pub drums: BTreeMap<String, [bool; STEPS]>,
    pub vel: BTreeMap<String, Vel>,
    /// Bass voice and a note per step (MIDI number).
    pub bass: Option<(String, [Option<u8>; STEPS])>,
    pub bass_vel: Vel,
    /// A chord name (`am`) and the pad voice, one chord a bar.
    pub chord: Option<(String, String)>,
}

impl Default for Vel {
    fn default() -> Vel {
        Vel::Default
    }
}

pub fn note_name(midi: u8) -> String {
    const N: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", N[(midi % 12) as usize], (midi as i32 / 12) - 1)
}

pub fn tempo_text(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 1e-9 {
        format!("{}", bpm.round() as i64)
    } else {
        format!("{bpm}")
    }
}

impl State {
    pub fn new(bpm: f64, bars: u32) -> State {
        State { bpm, bars, ..State::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.drums.values().all(|d| d.iter().all(|h| !h)) && self.bass.is_none() && self.chord.is_none()
    }

    /// The program text. Parts in the order the examples write them: kick, snare, hihat, clap; bass, harmony.
    pub fn text(&self) -> String {
        let mut s = format!("#! btw 1\ntempo {}\n", tempo_text(self.bpm));
        let mut a: Vec<String> = Vec::new();
        for role in DRUMS {
            if let Some(steps) = self.drums.get(role) {
                if steps.iter().any(|h| *h) {
                    let pat: Vec<&str> = steps.iter().map(|h| if *h { "x" } else { "~" }).collect();
                    let v = self.vel.get(role).copied().unwrap_or(Vel::Default);
                    a.push(format!("{role} {}{}", pat.join(" "), v.param()));
                }
            }
        }
        if !a.is_empty() {
            s += &format!("deck A: {}\n", a.join(" | "));
        }
        let mut b: Vec<String> = Vec::new();
        if let Some((inst, steps)) = &self.bass {
            if steps.iter().any(|n| n.is_some()) {
                let pat: Vec<String> = steps.iter().map(|n| n.map(note_name).unwrap_or_else(|| "~".into())).collect();
                b.push(format!("bass {} inst={inst}{}", pat.join(" "), self.bass_vel.param()));
            }
        }
        if let Some((chord, inst)) = &self.chord {
            b.push(format!("harmony chord({chord}) inst={inst}"));
        }
        if !b.is_empty() {
            s += &format!("deck B: {}\n", b.join(" | "));
        }
        s += &format!("end at bar {}\n", self.bars);
        s
    }

    pub fn with_drum(&self, role: &str, step: usize) -> State {
        let mut n = self.clone();
        n.drums.entry(role.to_string()).or_insert([false; STEPS])[step] = true;
        n
    }

    pub fn with_bass(&self, inst: &str, step: usize, midi: u8) -> State {
        let mut n = self.clone();
        let e = n.bass.get_or_insert_with(|| (inst.to_string(), [None; STEPS]));
        e.1[step] = Some(midi);
        n
    }

    pub fn with_chord(&self, chord: &str, inst: &str) -> State {
        let mut n = self.clone();
        n.chord = Some((chord.to_string(), inst.to_string()));
        n
    }
}

/// Chord names the pad stage tries: major and minor triads on the twelve tonics.
pub fn triads() -> Vec<String> {
    const T: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    let mut v = Vec::new();
    for t in T {
        v.push(t.to_string());
        v.push(format!("{t}m"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_text() {
        assert_eq!(note_name(45), "a2");
        assert_eq!(note_name(60), "c4");
        assert_eq!(note_name(40), "e2");
        assert_eq!(note_name(46), "a#2");
        let mut s = State::new(124.0, 8);
        s = s.with_drum("kick", 0).with_drum("kick", 4).with_bass("sub-bass", 0, 45).with_chord("am", "warm-pad");
        s.vel.insert("kick".into(), Vel::Range(60, 90));
        let t = s.text();
        assert!(t.contains("tempo 124\n"));
        assert!(t.contains("deck A: kick x ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ vel=60..90\n"));
        assert!(t.contains("deck B: bass a2 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ inst=sub-bass | harmony chord(am) inst=warm-pad\n"));
        assert!(t.ends_with("end at bar 8\n"));
        assert_eq!(triads().len(), 24);
    }
}
