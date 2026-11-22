//! The loop bank: one-bar loops in groups of an A and a B, to load on a deck in place of a
//! composed track and mix against the other. Groups 1–8 are Acid Bass lines; group 9 is Floor &
//! Sub (9A Floor Drums, 9B Sub Bass, built in `loops.rs`).
//!
//! A loop is a hand-built project (djhost `Engine::load`): the acid genre's kick and offbeat hat
//! (left out without drums) and a 16-step bass ring on its `acid` voice, the notes from
//! djengine's TD-3 acid generator (`gear::td3::Td3Pattern::acid`), accents as hard velocities,
//! slides and ties as notes held into the next step. Every loop is in A minor, the acid genre's
//! key, so the vocoder's chord pads fit whichever plays.
//!
//! A and B of an acid group share their first half (the B answers the A), so loading 1A on deck A
//! and 1B on deck B and riding the crossfader swaps the line's ending. Floor & Sub pairs a beat
//! with a bass line instead: the crossfader brings the sub in under the drums.

use djengine::gear::td3::{Td3Pattern, Transpose};
use djengine::pflow::{NetBundle, Project};
use djengine::TrackSpec;

/// The pattern's C (the TD-3's middle octave, C2); A minor sits on A2 = 45, the acid genre's root.
const BASE: u8 = 36;
/// A (0 = C).
const ROOT: u8 = 9;
const STEPS: usize = 16;

/// One loop of the bank, as the phone lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Td3Loop {
    /// "1A" … "9B".
    pub id: String,
    /// 1..=9.
    pub group: u32,
    /// "A" or "B".
    pub side: String,
    /// The group's name ("Classic").
    pub name: String,
    pub description: String,
    pub bpm: f64,
    /// "A minor".
    pub key: String,
}

/// Every loop of the bank is in A minor.
pub const KEY: &str = "A minor";

#[derive(Clone, Copy)]
enum Flavor {
    /// The generator's line as it comes.
    Classic,
    /// Slides on every fourth step, accents on the "and": rubbery.
    Squelch,
    /// No rests, octave pumps on the offbeats: driving.
    Rolling,
    /// The root on every eighth: hypnotic, one-note acid.
    Hypno,
    /// Every other note an octave up: the line leaps.
    Leap,
    /// Only the beats and the accents: a sparse, dry line with room around it.
    Minimal,
    /// Slides into most notes, accents only on the one and the three: slow and heavy.
    SlowBurn,
    /// No rests, accents on every beat, a drop an octave before each: fast and pushing.
    Rush,
}

#[derive(Clone, Copy)]
enum Kind {
    Acid(Flavor),
    /// 9A Floor Drums, 9B Sub Bass (`loops.rs`).
    FloorSub,
}

struct Group {
    name: &'static str,
    description: &'static str,
    seed: f64,
    bpm: f64,
    kind: Kind,
}

const GROUPS: [Group; 9] = [
    Group {
        name: "Classic",
        description: "The textbook acid line: accents, a few slides, a rest or two.",
        seed: 303.0,
        bpm: 125.0,
        kind: Kind::Acid(Flavor::Classic),
    },
    Group {
        name: "Squelch",
        description: "Rubbery: slides into every beat, accents on the off-beat.",
        seed: 909.0,
        bpm: 128.0,
        kind: Kind::Acid(Flavor::Squelch),
    },
    Group {
        name: "Rolling",
        description: "Every step plays, octaves pumping on the offbeats: drives a mix.",
        seed: 808.0,
        bpm: 130.0,
        kind: Kind::Acid(Flavor::Rolling),
    },
    Group {
        name: "Hypno",
        description: "One-note acid: the root on every eighth, the line in between.",
        seed: 606.0,
        bpm: 122.0,
        kind: Kind::Acid(Flavor::Hypno),
    },
    Group {
        name: "Leap",
        description: "Octave leaps: every other note jumps up, the line bouncing off the root.",
        seed: 101.0,
        bpm: 126.0,
        kind: Kind::Acid(Flavor::Leap),
    },
    Group {
        name: "Minimal",
        description: "Sparse and dry: notes on the beats and the accents, rests in between.",
        seed: 202.0,
        bpm: 124.0,
        kind: Kind::Acid(Flavor::Minimal),
    },
    Group {
        name: "Slow Burn",
        description: "Slower and heavy: slides into most notes, accents on the one and the three.",
        seed: 404.0,
        bpm: 118.0,
        kind: Kind::Acid(Flavor::SlowBurn),
    },
    Group {
        name: "Rush",
        description: "Fast and pushing: every step plays, an accent on each beat, an octave drop before it.",
        seed: 505.0,
        bpm: 135.0,
        kind: Kind::Acid(Flavor::Rush),
    },
    Group {
        name: "Floor & Sub",
        description: "A: Floor Drums, four on the floor with claps and hats. B: Sub Bass, a deep A minor line. Ride the crossfader to bring the sub in.",
        seed: 0.0,
        bpm: crate::loop_collections::floor::BPM,
        kind: Kind::FloorSub,
    },
];

/// The bank: 1A, 1B, 2A, … 4B.
pub fn bank() -> Vec<Td3Loop> {
    let mut v = Vec::new();
    for (g, grp) in GROUPS.iter().enumerate() {
        for side in ["A", "B"] {
            v.push(Td3Loop {
                id: format!("{}{side}", g + 1),
                group: g as u32 + 1,
                side: side.into(),
                name: grp.name.into(),
                description: grp.description.into(),
                bpm: grp.bpm,
                key: KEY.into(),
            });
        }
    }
    v
}

/// "1A" → (group index 0, B side false).
fn parse_id(id: &str) -> Option<(usize, bool)> {
    let id = id.trim().to_ascii_uppercase();
    let (n, side) = id.split_at(id.len().checked_sub(1)?);
    let g: usize = n.parse().ok()?;
    let b = match side {
        "A" => false,
        "B" => true,
        _ => return None,
    };
    (1..=GROUPS.len()).contains(&g).then_some((g - 1, b))
}

/// The 16 steps of an acid loop (None for Floor & Sub, which is not one acid line).
pub fn pattern(id: &str) -> Option<Td3Pattern> {
    let (g, b) = parse_id(id)?;
    let grp = &GROUPS[g];
    let Kind::Acid(flavor) = grp.kind else { return None };
    let mut p = Td3Pattern::acid(grp.seed, ROOT);
    if b {
        // the answer: the A's first half, a new second half
        let alt = Td3Pattern::acid(grp.seed + 77.0, ROOT);
        p.steps[STEPS / 2..].copy_from_slice(&alt.steps[STEPS / 2..]);
    }
    for (i, s) in p.steps.iter_mut().enumerate() {
        match flavor {
            Flavor::Classic => {}
            Flavor::Squelch => {
                s.slide |= i % 4 == 3;
                s.accent = i % 4 == 2 || (s.accent && i % 2 == 1);
            }
            Flavor::Rolling => {
                s.rest = false;
                if i % 4 == 2 {
                    s.transpose = Transpose::Up;
                }
            }
            Flavor::Hypno => {
                if i % 2 == 0 {
                    s.note = ROOT;
                    s.transpose = Transpose::None;
                    s.rest = false;
                    s.slide = false;
                }
            }
            Flavor::Leap => {
                s.transpose = if i % 2 == 1 { Transpose::Up } else { Transpose::None };
                s.slide &= i % 2 == 0;
            }
            Flavor::Minimal => {
                s.rest = !(i % 4 == 0 || s.accent);
                s.slide = false;
                s.tie = false;
            }
            Flavor::SlowBurn => {
                s.slide = i % 4 != 3;
                s.accent = i % 8 == 0;
                s.rest &= i % 4 == 3;
            }
            Flavor::Rush => {
                s.rest = false;
                s.tie = false;
                s.accent = i % 4 == 0;
                s.transpose = if i % 4 == 3 { Transpose::Down } else { Transpose::None };
            }
        }
    }
    p.steps[0].rest = false;
    Some(p)
}

/// What a deck playing loop `id` is called: "Acid Bass 1A · Classic", "Floor & Sub 9A · Floor Drums".
pub fn title(id: &str) -> Option<String> {
    let (g, b) = parse_id(id)?;
    let grp = &GROUPS[g];
    let id = normalize(id)?;
    Some(match grp.kind {
        Kind::Acid(_) => format!("Acid Bass {id} · {}", grp.name),
        Kind::FloorSub => format!("{} {id} · {}", grp.name, if b { "Sub Bass" } else { "Floor Drums" }),
    })
}

/// The loop as a project to load, `drums`: with the acid genre's kick and offbeat hat (Floor &
/// Sub: see `loops.rs`).
pub fn project(id: &str, drums: bool) -> Option<(Project, Td3Loop)> {
    project_shaped(id, drums, Shape::default())
}

/// How the Acid Bass tricks reshape a loop's line (the phone's own take on djrust's TD-3 tricks,
/// which shape the notes sent to the gear): `accent` hits every note hard, `slide` holds every
/// note into the next step. Default: the line as written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub accent: bool,
    pub slide: bool,
}

/// Is `id` one of the Acid Bass loops (groups 1–8), the ones the Acid Bass tricks reshape.
pub fn is_acid(id: &str) -> bool {
    pattern(id).is_some()
}

/// [`project`], with the line reshaped by `shape` (Floor & Sub ignores it).
pub fn project_shaped(id: &str, drums: bool, shape: Shape) -> Option<(Project, Td3Loop)> {
    let id = normalize(id)?;
    let info = bank().into_iter().find(|l| l.id == id)?;
    let (g, b) = parse_id(&id)?;
    if let Kind::FloorSub = GROUPS[g].kind {
        // the same projects (and model CIDs) as before they joined the bank
        let (proj, _) = crate::loop_collections::floor::project(if b { crate::loop_collections::floor::SUB_BASS } else { crate::loop_collections::floor::FLOOR_DRUMS }, drums)?;
        return Some((proj, info));
    }
    let pat = pattern(&info.id)?;
    let spec = TrackSpec::parse("acid:303").ok()?;
    let mut proj = spec.project();
    let keep: &[&str] = if drums { &["kick", "hihat", "bass"] } else { &["bass"] };
    for k in proj.nets.keys().to_vec() {
        let n = proj.nets.get(&k).unwrap();
        let group = if n.riff_group.is_empty() { k.as_str() } else { n.riff_group.as_str() };
        if !keep.contains(&group) || (group == "bass" && k != "bass") {
            proj.nets.remove(&k);
        }
    }
    let bass = proj.nets.get_mut("bass")?;
    let step_ms = 60_000.0 / info.bpm / 4.0;
    let mut ring = NetBundle::ring(STEPS as i64);
    let mut beats = 0;
    for (i, s) in pat.steps.iter().enumerate() {
        if s.rest {
            continue;
        }
        let held = s.slide || s.tie || shape.slide;
        let dur = if held { step_ms * 1.15 } else { step_ms * 0.55 };
        let vel = if s.accent || shape.accent { 127 } else { 88 };
        ring.bind(i as i64, s.midi_note(BASE) as i64, bass.track.channel, vel, dur.round() as i64);
        beats += 1;
    }
    ring.track = bass.track.clone();
    ring.track.ring_size = Some(STEPS as f64);
    ring.track.beats = Some(beats as f64);
    ring.track.generator = Some("td3".into());
    ring.role = bass.role.clone();
    *bass = ring;
    proj.name = format!("Acid Bass {} {}", info.id, info.name);
    proj.tempo = info.bpm;
    proj.swing = 0.0;
    proj.humanize = 0.0;
    proj.bars = Some(1);
    proj.structure.clear();
    proj.initial_mutes.clear();
    proj.stingers.clear();
    Some((proj, info))
}

/// "1A" or "1a" → "1A" when it is one of the bank's.
pub fn normalize(id: &str) -> Option<String> {
    let (g, b) = parse_id(id)?;
    Some(format!("{}{}", g + 1, if b { "B" } else { "A" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bank_has_nine_groups_of_a_and_b() {
        let b = bank();
        assert_eq!(b.len(), 18);
        assert_eq!(b[0].id, "1A");
        assert_eq!(b[17].id, "9B");
        assert_eq!(normalize("3b").as_deref(), Some("3B"));
        assert_eq!(normalize("7a").as_deref(), Some("7A"));
        assert!(normalize("10A").is_none() && normalize("1C").is_none() && normalize("").is_none());
        let names: Vec<&str> = b.iter().step_by(2).map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Classic", "Squelch", "Rolling", "Hypno", "Leap", "Minimal", "Slow Burn", "Rush", "Floor & Sub"]);
    }

    #[test]
    fn a_shape_reshapes_the_line() {
        let notes = |p: &Project| {
            let n = p.nets.get("bass").unwrap();
            n.bindings.values().map(|b| (b.velocity, b.duration)).collect::<Vec<_>>()
        };
        let (plain, _) = project("1A", false).unwrap();
        let (acc, _) = project_shaped("1A", false, Shape { accent: true, slide: false }).unwrap();
        let (sl, _) = project_shaped("1A", false, Shape { accent: false, slide: true }).unwrap();
        let (p, a, s) = (notes(&plain), notes(&acc), notes(&sl));
        assert_eq!(p.len(), a.len());
        assert!(p.iter().any(|n| n.0 != 127) && a.iter().all(|n| n.0 == 127));
        let long = s[0].1;
        assert!(s.iter().all(|n| n.1 == long) && p.iter().any(|n| n.1 < long));
        assert_eq!(notes(&project_shaped("1A", false, Shape::default()).unwrap().0), p);
        assert!(is_acid("8B") && !is_acid("9A") && !is_acid("10A"));
    }

    #[test]
    fn b_answers_a() {
        for g in 1..=8 {
            let a = pattern(&format!("{g}A")).unwrap();
            let b = pattern(&format!("{g}B")).unwrap();
            assert_eq!(a.steps[..8], b.steps[..8], "group {g}");
            assert_ne!(a.steps[8..], b.steps[8..], "group {g}");
        }
        assert!(pattern("9A").is_none());
    }

    #[test]
    fn every_group_is_its_own_line() {
        let ps: Vec<_> = (1..=8).map(|g| pattern(&format!("{g}A")).unwrap()).collect();
        for i in 0..ps.len() {
            for j in i + 1..ps.len() {
                assert_ne!(ps[i].steps, ps[j].steps, "groups {} and {}", i + 1, j + 1);
            }
        }
        let minimal = pattern("6A").unwrap();
        assert!(minimal.steps.iter().filter(|s| s.rest).count() >= 6, "minimal is sparse");
        assert!(pattern("8A").unwrap().steps.iter().all(|s| !s.rest), "rush never rests");
    }

    #[test]
    fn floor_and_sub_are_the_loops_they_were() {
        for drums in [false, true] {
            let (a, ia) = project("9A", drums).unwrap();
            let (b, ib) = project("9B", drums).unwrap();
            assert_eq!(a.model_cid(), crate::loop_collections::floor::project(crate::loop_collections::floor::FLOOR_DRUMS, drums).unwrap().0.model_cid());
            assert_eq!(b.model_cid(), crate::loop_collections::floor::project(crate::loop_collections::floor::SUB_BASS, drums).unwrap().0.model_cid());
            assert_eq!((ia.id.as_str(), ia.group, ia.side.as_str()), ("9A", 9, "A"));
            assert_eq!((ib.id.as_str(), ib.side.as_str()), ("9B", "B"));
        }
        assert_eq!(title("9a").as_deref(), Some("Floor & Sub 9A · Floor Drums"));
        assert_eq!(title("9B").as_deref(), Some("Floor & Sub 9B · Sub Bass"));
        assert_eq!(title("5A").as_deref(), Some("Acid Bass 5A · Leap"));
    }

    #[test]
    fn every_loop_builds_a_one_bar_project() {
        for l in bank().into_iter().filter(|l| l.group <= 8) {
            for drums in [true, false] {
                let (p, info) = project(&l.id, drums).unwrap();
                assert_eq!(info.id, l.id);
                assert_eq!(p.tempo, l.bpm);
                assert!(p.structure.is_empty());
                let bass = p.nets.get("bass").expect("a bass net");
                assert_eq!(bass.transitions.len(), STEPS);
                assert!(!bass.bindings.is_empty());
                assert_eq!(bass.track.instrument, "acid");
                let keys: Vec<&str> = p.nets.keys().iter().map(|s| s.as_str()).collect();
                if drums {
                    assert!(keys.contains(&"kick") && keys.contains(&"hihat"), "{keys:?}");
                } else {
                    assert_eq!(keys, ["bass"], "{keys:?}");
                }
            }
        }
    }
}
