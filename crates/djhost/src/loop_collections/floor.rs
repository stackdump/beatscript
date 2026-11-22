//! Floor Drums and Sub Bass: the loop bank's group 9, Floor & Sub (9A and 9B in `td3.rs`, which
//! lists and loads them). This file builds their projects.
//!
//! Like the acid loops, each is a hand-built project (djhost `Engine::load`) on the acid genre's
//! nets, so its voices, channels and key are the acid loops' and the vocoder's chord pads fit.
//! Neither has a seed: the 16 steps are written out below, so a loop is the same project on
//! every load (and so the same model CID).
//!
//! - Floor Drums: four on the floor, a clap on two and four, open hats on the offbeats and
//!   closed hats on the sixteenths between. The drums switch does nothing to it.
//! - Sub Bass: a deep, sparse A minor line on the sub-bass voice, low enough to sit under a
//!   track's own bass. With drums it carries Floor Drums' kick and offbeat hat (no clap or
//!   sixteenths), as the acid loops carry a kick and a hat.

use djengine::pflow::{NetBundle, Project};
use djengine::TrackSpec;

use crate::loop_collections::acid_bank::Td3Loop;

const STEPS: usize = 16;
pub(crate) const BPM: f64 = 125.0;

pub const FLOOR_DRUMS: &str = "floor-drums";
pub const SUB_BASS: &str = "sub-bass";

/// General MIDI drum keys, as the engine's synth kit reads them (`wave::synth::drum_kind`).
const KICK: u8 = 36;
const CLAP: u8 = 39;
const CLOSED_HAT: u8 = 42;
/// The synth kit plays 49 as its open hat (beats' drum map).
const OPEN_HAT: u8 = 49;

/// A1, the acid loops' root an octave down: under a track's bass, not on it.
const A1: u8 = 33;
const C2: u8 = 36;
const E1: u8 = 28;
const G1: u8 = 31;

/// One hit: (step, note, velocity).
type Hit = (usize, u8, u8);
/// One bass note: (step, note, velocity, length in steps).
type Note = (usize, u8, u8, f64);

const KICKS: [Hit; 4] = [(0, KICK, 124), (4, KICK, 120), (8, KICK, 124), (12, KICK, 120)];
const CLAPS: [Hit; 2] = [(4, CLAP, 104), (12, CLAP, 108)];
const OPEN_HATS: [Hit; 4] = [(2, OPEN_HAT, 96), (6, OPEN_HAT, 92), (10, OPEN_HAT, 96), (14, OPEN_HAT, 92)];
/// The sixteenths between the kicks and the open hats, the "a" a touch harder than the "e".
const CLOSED_HATS: [Hit; 8] = [
    (1, CLOSED_HAT, 52),
    (3, CLOSED_HAT, 66),
    (5, CLOSED_HAT, 52),
    (7, CLOSED_HAT, 66),
    (9, CLOSED_HAT, 52),
    (11, CLOSED_HAT, 66),
    (13, CLOSED_HAT, 52),
    (15, CLOSED_HAT, 70),
];

/// Long on the one, a push on the "a" of one, the root again, up to the minor third, back to the
/// root, then G1 and E1 walking down so the bar turns round as a fifth falling to the root.
const SUB_LINE: [Note; 7] = [
    (0, A1, 116, 2.75),
    (3, A1, 92, 0.9),
    (6, A1, 104, 1.75),
    (10, C2, 100, 1.4),
    (12, A1, 108, 1.75),
    (14, G1, 96, 0.9),
    (15, E1, 90, 0.9),
];

/// The two loops, as the phone lists them.
pub fn bank() -> Vec<Td3Loop> {
    vec![
        Td3Loop {
            id: FLOOR_DRUMS.into(),
            group: 1,
            side: String::new(),
            name: "Floor Drums".into(),
            description: "Four on the floor: kick on every beat, clap on two and four, open hats on the offbeats.".into(),
            bpm: BPM,
            key: crate::loop_collections::acid_bank::KEY.into(),
        },
        Td3Loop {
            id: SUB_BASS.into(),
            group: 2,
            side: String::new(),
            name: "Sub Bass".into(),
            description: "A deep, sparse A minor sub line to sit under a track. Drums add the kick and offbeat hat.".into(),
            bpm: BPM,
            key: crate::loop_collections::acid_bank::KEY.into(),
        },
    ]
}

/// `id` as the bank names it ("Floor-Drums" → "floor-drums"), when it is one of these.
pub fn normalize(id: &str) -> Option<String> {
    let id = id.trim().to_ascii_lowercase();
    bank().into_iter().any(|l| l.id == id).then_some(id)
}

/// A 16-step ring on `like`'s track and role, playing `hits` (ms lengths).
fn ring(like: &NetBundle, hits: impl IntoIterator<Item = (usize, u8, u8, f64)>, generator: &str) -> NetBundle {
    let mut r = NetBundle::ring(STEPS as i64);
    let mut beats = 0;
    for (step, note, vel, ms) in hits {
        r.bind(step as i64, note as i64, like.track.channel, vel as i64, ms.round() as i64);
        beats += 1;
    }
    r.track = like.track.clone();
    r.track.ring_size = Some(STEPS as f64);
    r.track.beats = Some(beats as f64);
    r.track.generator = Some(generator.into());
    r.role = like.role.clone();
    r
}

fn drums(hits: &[Hit]) -> impl Iterator<Item = (usize, u8, u8, f64)> + '_ {
    hits.iter().map(|&(s, n, v)| (s, n, v, 50.0))
}

/// The loop as a project to load. `drums` only matters to Sub Bass (see the module doc).
pub fn project(id: &str, drums_on: bool) -> Option<(Project, Td3Loop)> {
    let id = normalize(id)?;
    let info = bank().into_iter().find(|l| l.id == id)?;
    let floor = id == FLOOR_DRUMS;
    let mut proj = TrackSpec::parse("acid:303").ok()?.project();
    let keep: &[&str] = match (floor, drums_on) {
        (true, _) => &["kick", "snare", "hihat"],
        (false, true) => &["kick", "hihat", "bass"],
        (false, false) => &["bass"],
    };
    for k in proj.nets.keys().to_vec() {
        // the named nets only: a riff group's variants ("bass-2") go too
        if !keep.contains(&k.as_str()) {
            proj.nets.remove(&k);
        }
    }
    let step_ms = 60_000.0 / BPM / 4.0;
    let generator = if floor { "floor-drums" } else { "sub-bass" };
    if let Some(n) = proj.nets.get_mut("kick") {
        *n = ring(n, drums(&KICKS), generator);
    }
    if let Some(n) = proj.nets.get_mut("snare") {
        *n = ring(n, drums(&CLAPS), generator);
    }
    if let Some(n) = proj.nets.get_mut("hihat") {
        let mut hats: Vec<Hit> = OPEN_HATS.to_vec();
        if floor {
            hats.extend(CLOSED_HATS);
            hats.sort_by_key(|h| h.0);
        }
        *n = ring(n, drums(&hats), generator);
    }
    if let Some(n) = proj.nets.get_mut("bass") {
        let notes = SUB_LINE.iter().map(|&(s, note, v, len)| (s, note, v, step_ms * len * 0.95));
        *n = ring(n, notes, generator);
        n.track.instrument = "sub-bass".into();
    }
    proj.name = info.name.clone();
    proj.tempo = BPM;
    proj.swing = 0.0;
    proj.humanize = 0.0;
    proj.bars = Some(1);
    proj.structure.clear();
    proj.initial_mutes.clear();
    proj.stingers.clear();
    Some((proj, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(p: &Project) -> Vec<String> {
        p.nets.keys().to_vec()
    }

    #[test]
    fn two_loops_with_generic_names() {
        let b = bank();
        assert_eq!(b.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Floor Drums", "Sub Bass"]);
        assert_eq!(normalize(" Sub-Bass ").as_deref(), Some(SUB_BASS));
        assert!(normalize("1A").is_none() && normalize("").is_none());
        // never an acid-bank id, so the two banks cannot answer for each other
        for l in &b {
            assert!(crate::loop_collections::acid_bank::normalize(&l.id).is_none(), "{}", l.id);
        }
    }

    #[test]
    fn floor_drums_is_four_on_the_floor() {
        let (p, _) = project(FLOOR_DRUMS, false).unwrap();
        assert_eq!(keys(&p), ["kick", "snare", "hihat"]);
        let kick = p.nets.get("kick").unwrap();
        assert_eq!(kick.bindings.keys().to_vec(), ["t0", "t4", "t8", "t12"]);
        assert!(kick.bindings.values().all(|b| b.note == KICK as i64));
        let snare = p.nets.get("snare").unwrap();
        assert_eq!(snare.bindings.keys().to_vec(), ["t4", "t12"]);
        let hat = p.nets.get("hihat").unwrap();
        assert_eq!(hat.bindings.len(), 12);
        assert_eq!(hat.bindings.get("t2").unwrap().note, OPEN_HAT as i64);
        assert_eq!(hat.bindings.get("t1").unwrap().note, CLOSED_HAT as i64);
        // the drums switch is the loop itself
        assert_eq!(project(FLOOR_DRUMS, true).unwrap().0.model_cid(), p.model_cid());
    }

    #[test]
    fn sub_bass_is_low_and_on_the_sub_voice() {
        for drums in [false, true] {
            let (p, _) = project(SUB_BASS, drums).unwrap();
            let bass = p.nets.get("bass").expect("a bass net");
            assert_eq!(bass.track.instrument, "sub-bass");
            assert_eq!(bass.transitions.len(), STEPS);
            assert!(bass.bindings.values().all(|b| b.note < 40), "a sub line stays under E2");
            if drums {
                assert_eq!(keys(&p), ["kick", "hihat", "bass"]);
                assert_eq!(p.nets.get("hihat").unwrap().bindings.len(), 4, "offbeat hats only");
            } else {
                assert_eq!(keys(&p), ["bass"]);
            }
        }
    }

    #[test]
    fn every_load_is_the_same_project() {
        for l in bank() {
            for drums in [false, true] {
                let (a, _) = project(&l.id, drums).unwrap();
                let (b, _) = project(&l.id, drums).unwrap();
                assert_eq!(a.model_cid(), b.model_cid());
                assert_eq!(a.tempo, l.bpm);
                assert_eq!(a.bars, Some(1));
                assert!(a.structure.is_empty());
            }
        }
    }
}
