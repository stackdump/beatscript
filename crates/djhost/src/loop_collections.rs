//! The loop pads' collections (djandroid's loop cards and djapp's Loops window, moved here from
//! djmobile's `loopsets.rs` / `td3.rs` / `loops.rs`): the Acid Bass bank ([`acid_bank`], with
//! Floor & Sub in [`floor`]) and djrust's loop sets
//! (`djengine::loopsets`): every genre's curated set, and sets generated from a genre and a seed.
//!
//! A collection is named by its spec:
//!
//! - [`ACID_BASS`]: the Acid Bass bank (1A … 9B, `td3.rs`);
//! - `techno`: the genre's curated set;
//! - `techno:4411`: a set generated from that seed, the same on every device (its CID too).
//!
//! A deck holding a loop remembers it as one string ([`deck_id`]): the Acid Bass bank's own ids
//! ("1A") as before, a set's as `<spec>/<id>` ("techno:4411/3A").
//!
//! Every collection's group is a **pair**, an A and a B made to go together (docs/LOOPS.md):
//! [`pair`] hands out both loops and [`pair_report`] measures them with `djengine::looppair`, the
//! same measure for the Acid Bass bank, Floor & Sub and every genre's set. A set's B is made to
//! answer its A where the set is built (`LoopSet::project`, `looppair::complement`), so a loop
//! loaded through [`project`] / [`load`] / [`load_pair`] is already half of a pair.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use djengine::looppair::{self, PairReport};
use djengine::loopsets::{self, LoopSet, Side};
use djengine::pflow::Project;

use self::acid_bank::{self as td3, Td3Loop};

pub mod acid_bank;
pub mod floor;

/// The Acid Bass bank's spec.
pub const ACID_BASS: &str = "acid-bass";

/// A collection the picker lists.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopCollection {
    /// What [`loop_bank`] takes: "acid-bass", "techno", "techno:4411".
    pub spec: String,
    /// "Acid Bass", "Techno", "Techno #4411".
    pub name: String,
    /// The generator genre it is cut from ("acid" for the Acid Bass bank).
    pub genre: String,
}

/// A collection and its loops.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopBank {
    pub spec: String,
    pub name: String,
    pub genre: String,
    /// A generated set's seed (None: curated, or the Acid Bass bank).
    pub seed: Option<u32>,
    /// "A minor".
    pub key: String,
    /// The set's tempo (the Acid Bass bank's groups each have their own: its first).
    pub bpm: f64,
    /// A djrust loop set's content ID (`z…`); None for the Acid Bass bank.
    pub cid: Option<String>,
    pub description: String,
    pub loops: Vec<Td3Loop>,
}

/// Sets already built (a generated one takes a moment: it scores its candidates).
fn cache() -> &'static Mutex<HashMap<String, LoopSet>> {
    static C: OnceLock<Mutex<HashMap<String, LoopSet>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The set `spec` names, built once.
pub fn set(spec: &str) -> Option<LoopSet> {
    let key = spec.trim().to_ascii_lowercase();
    if let Some(s) = cache().lock().ok()?.get(&key) {
        return Some(s.clone());
    }
    let s = LoopSet::parse(&key)?;
    if let Ok(mut c) = cache().lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, s.clone());
    }
    Some(s)
}

/// The Acid Bass bank first, then every genre with a curated set.
pub fn collections() -> Vec<LoopCollection> {
    let mut v = vec![LoopCollection { spec: ACID_BASS.into(), name: "Acid Bass".into(), genre: "acid".into() }];
    for g in loopsets::genres() {
        if loopsets::curated_seeds(&g).is_some() {
            v.push(LoopCollection { spec: g.clone(), name: loopsets::genre_label(&g), genre: g });
        }
    }
    v
}

fn is_acid_bass(spec: &str) -> bool {
    spec.trim().eq_ignore_ascii_case(ACID_BASS)
}

fn info(set: &LoopSet) -> Vec<Td3Loop> {
    set.loops()
        .into_iter()
        .map(|l| Td3Loop {
            id: l.id,
            group: l.group,
            side: if l.side == Side::A { "A".into() } else { "B".into() },
            name: l.name,
            description: l.description,
            bpm: l.bpm,
            key: l.key,
        })
        .collect()
}

/// A collection's name without building it: "Acid Bass", "Techno", "Techno #4411".
pub fn bank_name(spec: &str) -> String {
    if is_acid_bass(spec) {
        return "Acid Bass".into();
    }
    match spec.trim().split_once(':') {
        Some((g, seed)) => format!("{} #{seed}", loopsets::genre_label(g)),
        None => loopsets::genre_label(spec.trim()),
    }
}

/// Collection `spec` and its loops.
pub fn bank(spec: &str) -> Option<LoopBank> {
    if is_acid_bass(spec) {
        let loops = td3::bank();
        return Some(LoopBank {
            spec: ACID_BASS.into(),
            name: "Acid Bass".into(),
            genre: "acid".into(),
            seed: None,
            key: td3::KEY.into(),
            bpm: loops.first().map_or(125.0, |l| l.bpm),
            cid: None,
            description: "One-bar loops: eight acid bass lines and Floor & Sub, all in A minor.".into(),
            loops,
        });
    }
    let s = set(spec)?;
    Some(LoopBank {
        spec: s.spec(),
        name: s.name(),
        genre: s.genre.clone(),
        seed: s.seed,
        key: s.key(),
        bpm: s.bpm,
        cid: Some(s.cid()),
        description: format!(
            "{} loops cut from {} tracks, {:.0} BPM in {}. A plays each part's first riff, B answers it: the same groove, the lead in A's gaps.",
            loopsets::GROUPS * 2,
            loopsets::genre_label(&s.genre),
            s.bpm,
            s.key(),
        ),
        loops: info(&s),
    })
}

/// What a deck holding loop `id` of collection `spec` remembers.
pub fn deck_id(spec: &str, id: &str) -> Option<String> {
    if is_acid_bass(spec) {
        return td3::normalize(id);
    }
    let s = set(spec)?;
    let (g, side) = s.parse_id(id)?;
    Some(format!("{}/{}{}", s.spec(), g.number, if side == Side::A { "A" } else { "B" }))
}

/// A remembered loop's collection and id: "techno:4411/3A" → ("techno:4411", "3A"); "1A" →
/// (acid-bass, "1A").
pub fn split(deck_id: &str) -> (&str, &str) {
    deck_id.rsplit_once('/').unwrap_or((ACID_BASS, deck_id))
}

/// Loop `id` of collection `spec` as a project, and the loop's listing.
pub fn project(spec: &str, id: &str, drums: bool) -> Option<(Project, Td3Loop)> {
    if is_acid_bass(spec) {
        return td3::project(id, drums);
    }
    let s = set(spec)?;
    let p = s.project(id, drums)?;
    let (g, side) = s.parse_id(id)?;
    let want = format!("{}{}", g.number, if side == Side::A { "A" } else { "B" });
    let l = info(&s).into_iter().find(|l| l.id == want)?;
    Some((p, l))
}

/// Group `group` of collection `spec` as a pair: its A and B, as they load on the decks.
pub fn pair(spec: &str, group: u32, drums: bool) -> Option<(Project, Project)> {
    let (a, _) = project(spec, &format!("{group}A"), drums)?;
    let (b, _) = project(spec, &format!("{group}B"), drums)?;
    Some((a, b))
}

/// How group `group`'s B stands to its A (`djengine::looppair::report`): the key, tempo and
/// length they share, and per part and band which of B's notes are unison, fill A's gaps, sit an
/// octave away, or clash.
pub fn pair_report(spec: &str, group: u32, drums: bool) -> Option<PairReport> {
    let (a, b) = pair(spec, group, drums)?;
    Some(looppair::report(&a, &b))
}

/// What a deck playing a remembered loop is called: "Acid Bass 1A · Classic", "Techno 3A · Wired
/// Drift".
pub fn title(deck_id: &str) -> Option<String> {
    let (spec, id) = split(deck_id);
    if is_acid_bass(spec) {
        return td3::title(id);
    }
    let s = set(spec)?;
    let (g, _) = s.parse_id(id)?;
    Some(format!("{} {} · {}", s.name(), id, g.name))
}

/// The genre a remembered loop is in (the vocoder's chord pads follow it).
pub fn genre(deck_id: &str) -> Option<String> {
    let (spec, _) = split(deck_id);
    if is_acid_bass(spec) {
        return Some("acid".into());
    }
    set(spec).map(|s| s.genre)
}

/// The track seed a remembered loop was cut from (the engine's load seed), else the Acid Bass
/// bank's 303.
pub fn load_seed(deck_id: &str) -> f64 {
    let (spec, id) = split(deck_id);
    set(spec)
        .and_then(|s| s.parse_id(id).map(|(g, _)| g.seed as f64))
        .unwrap_or(303.0)
}

/// A remembered loop as collection `spec` (a [`LoopBank::spec`]) numbers it ("techno:4411/3A" →
/// "3A" while that set is picked), or None when it is from another collection. djandroid's
/// `heldHere` (Td3Card.kt).
pub fn held_here<'a>(spec: &str, deck_id: &'a str) -> Option<&'a str> {
    let (s, id) = split(deck_id);
    let same = if is_acid_bass(spec) { is_acid_bass(s) } else { s == spec };
    same.then_some(id)
}

/// The combined A / B readout of a collection's pairs: what each deck holds and the group they
/// share. djandroid's "Loops A / B" card (`Td3PairCard`, Td3Card.kt), here so the desktop and
/// the phone read it the same way.
#[derive(Debug, Clone, PartialEq)]
pub struct PairReadout {
    /// The loop on deck A / deck B, as the collection numbers it ("3A"), if it is from this one.
    pub a: Option<String>,
    pub b: Option<String>,
    /// The group whose A is on deck A and whose B is on deck B: the pair is lit.
    pub lit: Option<u32>,
    /// The first group holding either deck's loop, and its description: one line when its loops
    /// share it, else "3A: … · 3B: …".
    pub group: Option<u32>,
    pub text: Option<String>,
}

/// [`PairReadout`] of `bank` with `held` (each deck's remembered id, [`deck_id`]).
pub fn pair_readout(bank: &LoopBank, held: [Option<&str>; 2]) -> PairReadout {
    let [a, b] = held.map(|h| h.and_then(|h| held_here(&bank.spec, h)).map(str::to_string));
    let lit = a.as_deref().and_then(|a| {
        let g: u32 = a.strip_suffix('A')?.parse().ok()?;
        (b.as_deref() == Some(format!("{g}B").as_str())).then_some(g)
    });
    let holds = |l: &Td3Loop| Some(l.id.as_str()) == a.as_deref() || Some(l.id.as_str()) == b.as_deref();
    let group = bank.loops.iter().find(|l| holds(l)).map(|l| l.group);
    let text = group.map(|g| {
        let gl: Vec<&Td3Loop> = bank.loops.iter().filter(|l| l.group == g).collect();
        let first = gl[0].description.as_str();
        if gl.iter().all(|l| l.description == first) {
            first.to_string()
        } else {
            gl.iter().map(|l| format!("{}: {}", l.id, l.description)).collect::<Vec<_>>().join(" · ")
        }
    });
    PairReadout { a, b, lit, group, text }
}

/// Put loop `id` of collection `spec` on `deck` of `e` and play it; locked to the bar of the other
/// deck when that one plays a track. The deck's remembered id ([`deck_id`]) back.
pub fn load(e: &mut crate::deck::Engine, deck: &str, spec: &str, id: &str, drums: bool) -> Result<String, String> {
    use djengine::session::DeckAction;
    let none = || format!("no loop {id} in {spec}");
    let (proj, _) = project(spec, id, drums).ok_or_else(none)?;
    let did = deck_id(spec, id).ok_or_else(none)?;
    let other = if deck == "A" { "B" } else { "A" };
    let with = e.deck(other).is_some_and(|d| d.project.is_some() && d.mirror.is_playing());
    if !e.load(deck, &proj, load_seed(&did)) {
        return Err(format!("the loop {did} did not load"));
    }
    let ctx = crate::deck::PerformCtx::default;
    e.perform(&DeckAction::Play { deck: deck.into(), on: true }, ctx());
    if with {
        e.perform(&DeckAction::Sync { deck: deck.into(), align: "bar".into() }, ctx());
    }
    Ok(did)
}

/// Group `group` of `spec` on both decks: its A on deck A, its B on deck B, both playing and
/// locked on the bar, so the crossfader swaps the line's answer in and out.
pub fn load_pair(e: &mut crate::deck::Engine, spec: &str, group: u32, drums: bool) -> Result<(String, String), String> {
    let a = load(e, "A", spec, &format!("{group}A"), drums)?;
    let b = load(e, "B", spec, &format!("{group}B"), drums)?;
    e.perform(
        &djengine::session::DeckAction::Sync { deck: "B".into(), align: "bar".into() },
        crate::deck::PerformCtx::default(),
    );
    Ok((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_acid_bass_bank_comes_first_then_the_genres() {
        let c = collections();
        assert_eq!(c[0].spec, ACID_BASS);
        assert!(c.len() > 10, "{c:?}");
        assert!(c.iter().any(|c| c.spec == "techno" && c.name == "Techno"));
    }

    #[test]
    fn a_genre_bank_lists_a_and_b_with_key_and_bpm() {
        let b = bank("techno").unwrap();
        assert_eq!(b.loops.len(), loopsets::GROUPS * 2);
        assert!(b.loops.iter().all(|l| l.bpm == b.bpm && l.key == b.key));
        assert!(b.cid.as_deref().is_some_and(|c| c.starts_with('z')));
        let (p, l) = project("techno", "2b", true).unwrap();
        assert_eq!((l.id.as_str(), l.side.as_str()), ("2B", "B"));
        assert_eq!(p.tempo, b.bpm);
        assert!(p.nets.contains_key("bass") && p.nets.contains_key("kick") && p.nets.contains_key("melody"));
        let (a, _) = project("techno", "2A", false).unwrap();
        assert!(a.nets.contains_key("bass") && !a.nets.contains_key("kick"));
    }

    #[test]
    fn a_generated_bank_is_the_same_bank_again() {
        let a = bank("house:4411").unwrap();
        cache().lock().unwrap().clear();
        let b = bank("House:4411").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.spec, "house:4411");
        assert_ne!(a.cid, bank("house:4412").unwrap().cid);
    }

    #[test]
    fn a_loop_and_a_pair_load_and_play() {
        let (mut e, _r) = crate::deck::engine(crate::deck::EngineConfig::new(48_000.0));
        assert_eq!(load(&mut e, "A", "techno", "3a", true).unwrap(), "techno/3A");
        assert!(e.deck("A").unwrap().project.is_some());
        let (a, b) = load_pair(&mut e, ACID_BASS, 2, false).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("2A", "2B"));
        assert!(load(&mut e, "B", "techno", "12A", true).is_err());
    }

    #[test]
    fn deck_ids_round_trip() {
        assert_eq!(deck_id(ACID_BASS, "3b").as_deref(), Some("3B"));
        assert_eq!(split("3B"), (ACID_BASS, "3B"));
        let id = deck_id("techno:7", "3a").unwrap();
        assert_eq!(id, "techno:7/3A");
        assert_eq!(split(&id), ("techno:7", "3A"));
        assert!(title(&id).unwrap().starts_with("Techno #7 3A · "));
        assert_eq!(title("1A").as_deref(), Some("Acid Bass 1A · Classic"));
        assert_eq!(genre(&id).as_deref(), Some("techno"));
        assert!(deck_id("techno", "9A").is_none() && deck_id("nope", "1A").is_none());
    }

    #[test]
    fn the_pair_readout_lights_a_group_on_both_decks_and_reads_its_description() {
        let acid = bank(ACID_BASS).unwrap();
        let r = pair_readout(&acid, [Some("2A"), Some("2B")]);
        assert_eq!((r.a.as_deref(), r.b.as_deref(), r.lit, r.group), (Some("2A"), Some("2B"), Some(2), Some(2)));
        assert!(r.text.as_deref().is_some_and(|t| !t.is_empty()), "{r:?}");
        // the decks on two groups: not lit, the readout is the first group in the bank's order
        let r = pair_readout(&acid, [Some("3A"), Some("2B")]);
        assert_eq!((r.lit, r.group), (None, Some(2)));
        // nothing held, or a loop of another collection
        let id = deck_id("techno", "1A").unwrap();
        let r = pair_readout(&acid, [None, Some(&id)]);
        assert_eq!((r.a, r.b, r.lit, r.text), (None, None, None, None));
        let techno = bank("techno").unwrap();
        let r = pair_readout(&techno, [Some(&id), Some(&deck_id("techno", "1B").unwrap())]);
        assert_eq!(r.lit, Some(1));
        assert!(r.text.is_some_and(|t| !t.is_empty()));
    }

    // MARK: pairs (docs/LOOPS.md)

    use djengine::looppair::Band;

    fn no_clash(r: &PairReport) -> bool {
        [Band::Low, Band::Mid, Band::High].iter().all(|b| r.band(*b).clash == 0)
    }

    #[test]
    fn every_genre_sets_groups_are_pairs_that_layer() {
        let mut specs: Vec<String> = collections().into_iter().skip(1).map(|c| c.spec).collect();
        specs.push("techno:4411".into());
        specs.push("dnb:7".into());
        assert!(specs.len() > 20, "{specs:?}");
        for spec in specs {
            for g in 1..=loopsets::GROUPS as u32 {
                let r = pair_report(&spec, g, true).unwrap();
                let at = format!("{spec} group {g}: {}", looppair::summary(&r));
                // one key, one tempo, one length, the same parts, rings that line up
                assert!(r.frame_ok() && r.same_parts, "{at}");
                // the groove stacks in unison, nothing of B's starts against A, the lead answers
                assert!(no_clash(&r), "{at}");
                assert_eq!(r.harmony_ratio(), 1.0, "{at}");
                assert!(r.lead_answers(), "{at}");
                // and the bank lists them as a pair: one key, one tempo
                let b = bank(&spec).unwrap();
                let (la, lb) = (&b.loops[(g as usize - 1) * 2], &b.loops[(g as usize - 1) * 2 + 1]);
                assert_eq!((la.bpm, &la.key, la.group), (lb.bpm, &lb.key, lb.group), "{at}");
            }
        }
    }

    #[test]
    fn the_acid_bass_pairs_share_a_call_and_answer_in_the_second_half() {
        // the reference: one bar, one key (A minor), one tempo per group; B is A's first half
        // (the call) and a new second half (the answer), so the crossfader swaps the ending
        for g in 1..=8u32 {
            let (a, b) = (acid_bank::pattern(&format!("{g}A")).unwrap(), acid_bank::pattern(&format!("{g}B")).unwrap());
            let step = |p: &djengine::gear::td3::Td3Pattern, r: std::ops::Range<usize>| {
                p.steps[r].iter().map(|s| (s.rest, s.note, s.accent)).collect::<Vec<_>>()
            };
            assert_eq!(step(&a, 0..8), step(&b, 0..8), "group {g}: the call is shared");
            assert_ne!(step(&a, 8..16), step(&b, 8..16), "group {g}: the answer is B's own");
            let r = pair_report(ACID_BASS, g, true).unwrap();
            assert!(r.frame_ok() && r.same_parts && r.bars == 1, "group {g}: {}", looppair::summary(&r));
            let bass = &r.parts["bass"];
            assert!(bass.unison >= 3, "group {g} shares its call: {}", looppair::summary(&r));
            // the drums are the same in both
            assert!(r.parts["kick"].unison == 4 && r.parts["hihat"].unison == 7, "{}", looppair::summary(&r));
        }
    }

    #[test]
    fn floor_and_sub_split_the_drums_from_the_bass() {
        for drums in [true, false] {
            let r = pair_report(ACID_BASS, 9, drums).unwrap();
            let at = looppair::summary(&r);
            assert!(r.frame_ok(), "{at}");
            // A has no bass, B has: the roles are split, not shared
            assert!(!r.same_parts && r.parts["bass"].a == 0 && r.parts["bass"].b > 0, "{at}");
            assert!(no_clash(&r), "{at}");
        }
        // with drums B carries A's kick and its offbeat hat, in unison
        let r = pair_report(ACID_BASS, 9, true).unwrap();
        assert_eq!((r.parts["kick"].unison, r.parts["hihat"].unison), (4, 4), "{}", looppair::summary(&r));
    }

    #[test]
    fn a_set_pair_loaded_on_both_decks_plays_in_one_key_and_tempo() {
        let (mut e, _r) = crate::deck::engine(crate::deck::EngineConfig::new(48_000.0));
        let (a, b) = load_pair(&mut e, "house:4411", 3, true).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("house:4411/3A", "house:4411/3B"));
        let (pa, pb) = pair("house:4411", 3, true).unwrap();
        assert_eq!((pa.tempo, pa.root_note, pa.bars), (pb.tempo, pb.root_note, pb.bars));
    }
}
