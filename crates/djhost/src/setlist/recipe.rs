//! Making a set from a recipe: "12 segments, dnb then jungle, 16 bars each, vary the transitions".
//!
//! Deterministic: the same [`Recipe`] (seed included) over the same banks makes the same set,
//! byte for byte, so the same CID. Only complementary A / B pairs are used: a segment is group
//! `g` of a loop set, which plays that group's own A on deck A and its own B on deck B, and a
//! group is only used when the set has both its sides. Between one set and the next the
//! generator looks at a few candidate loop sets of the genre and takes the one whose key mixes
//! with the one before (`perform::harmonic`) and whose tempo is nearest.

use djengine::rng::Mulberry32;

use crate::loop_collections as lc;
use crate::perform::{harmonic, Key, MoveKind};

use super::{
    clamp_bars, InsertAuto, InsertKind, Segment, SetList, Source, Tag, Transition, TransitionKind, DEFAULT_BARS, MAX_SEGMENTS,
};

/// What the generator needs to know of a loop set.
#[derive(Clone, Debug, PartialEq)]
pub struct Bank {
    /// Groups that have both an A and a B (1..=groups).
    pub groups: u32,
    pub bpm: f64,
    /// "A minor".
    pub key: String,
}

/// The loop sets, behind a seam so the tests can fake them (building a real one scores
/// candidates and takes a moment).
pub trait BankInfo {
    fn bank(&self, spec: &str) -> Option<Bank>;
}

/// The real loop sets (`loop_collections`).
pub struct LoopBanks;

impl BankInfo for LoopBanks {
    fn bank(&self, spec: &str) -> Option<Bank> {
        let b = lc::bank(spec)?;
        let mut groups = 0;
        for g in 1..=b.loops.iter().map(|l| l.group).max().unwrap_or(0) {
            let a = b.loops.iter().any(|l| l.group == g && l.side == "A");
            let bb = b.loops.iter().any(|l| l.group == g && l.side == "B");
            if a && bb {
                groups = g;
            } else {
                break;
            }
        }
        (groups > 0).then(|| Bank { groups, bpm: b.bpm, key: b.key })
    }
}

/// "A minor" / "C# major" / "Bb minor" as a [`Key`].
pub fn parse_key(s: &str) -> Option<Key> {
    let (root, mode) = s.trim().split_once(' ')?;
    let mut ch = root.chars();
    let base: i32 = match ch.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let acc: i32 = match ch.next() {
        None => 0,
        Some('#') | Some('♯') => 1,
        Some('b') | Some('♭') => -1,
        Some(_) => return None,
    };
    let minor = match mode.trim().to_ascii_lowercase().as_str() {
        "minor" | "min" | "m" => true,
        "major" | "maj" => false,
        _ => return None,
    };
    Some(Key { pc: (base + acc).rem_euclid(12) as u8, minor })
}

/// What to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub name: Option<String>,
    pub segments: u32,
    /// In order: the segments are shared out among them as evenly as they go, the first ones
    /// taking the remainder ("dnb then jungle").
    pub genres: Vec<String>,
    pub bars: u32,
    /// A different transition on each segment (never the same twice running), else `transition`.
    pub vary: bool,
    /// What every segment uses when `vary` is off.
    pub transition: TransitionKind,
    /// Master filter sweeps and delay throws on the exits.
    pub inserts: bool,
    pub seed: u32,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe {
            name: None,
            segments: 12,
            genres: vec!["dnb".into()],
            bars: DEFAULT_BARS,
            vary: true,
            transition: TransitionKind::Move(MoveKind::FilterExit),
            inserts: true,
            seed: 1,
        }
    }
}

impl Recipe {
    /// The genres from words: `"dnb then jungle"`, `"dnb, jungle"`, `"dnb > jungle"`.
    pub fn parse_genres(s: &str) -> Vec<String> {
        s.replace(" then ", ",").replace(" and ", ",").replace(['>', '→', '+', '/', ';'], ",")
            .split(',')
            .map(|g| g.trim().to_ascii_lowercase())
            .filter(|g| !g.is_empty())
            .collect()
    }
}

/// How hard segment `i` of `n` hits: a rise, a peak and a way down, 1 to 5.
pub fn energy_curve(i: usize, n: usize) -> u8 {
    if n <= 1 {
        return 3;
    }
    let t = i as f64 / (n - 1) as f64;
    let v = 0.2 + 0.8 * (std::f64::consts::PI * t).sin();
    ((v * 5.0).ceil() as i64).clamp(1, 5) as u8
}

/// Candidate loop sets looked at for each new one.
const CANDIDATES: usize = 6;

/// Make the set. Err: a genre with no loop set, or a recipe that asks for nothing.
pub fn generate(recipe: &Recipe, banks: &dyn BankInfo) -> Result<SetList, String> {
    let n = recipe.segments as usize;
    if n == 0 || n > MAX_SEGMENTS {
        return Err(format!("segments is a number 1-{MAX_SEGMENTS}"));
    }
    if recipe.genres.is_empty() {
        return Err("a recipe needs at least one genre".into());
    }
    let mut rng = Mulberry32::new(recipe.seed as f64);
    // the genres' shares
    let k = recipe.genres.len();
    let share = |g: usize| n / k + usize::from(g < n % k);
    let mut sources: Vec<(Source, String)> = Vec::with_capacity(n);
    let mut prev: Option<Bank> = None;
    let mut used: Vec<String> = Vec::new();
    for (gi, genre) in recipe.genres.iter().enumerate() {
        let mut want = share(gi);
        while want > 0 {
            // candidates: fresh seeds of this genre, the best match to what plays before it
            let mut best: Option<(f64, String, Bank)> = None;
            let mut tried = 0;
            while tried < CANDIDATES {
                let seed = 1 + rng.next_int(999_999);
                let spec = format!("{genre}:{seed}");
                if used.contains(&spec) {
                    continue;
                }
                tried += 1;
                let Some(b) = banks.bank(&spec) else { continue };
                if b.groups == 0 {
                    continue;
                }
                let score = match &prev {
                    None => 0.0,
                    Some(p) => {
                        let mix = match (parse_key(&p.key), parse_key(&b.key)) {
                            (Some(x), Some(y)) if harmonic(x, y).mixes() => 20.0,
                            _ => 0.0,
                        };
                        mix - (p.bpm - b.bpm).abs()
                    }
                };
                if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
                    best = Some((score, spec, b));
                }
            }
            let Some((_, spec, b)) = best else {
                return Err(format!("no loop set could be made for genre {genre:?} (genres: djapp_loopsets list)"));
            };
            used.push(spec.clone());
            for g in 1..=b.groups.min(want as u32) {
                sources.push((Source::Pair { spec: spec.clone(), group: g }, genre.clone()));
            }
            want -= b.groups.min(want as u32) as usize;
            prev = Some(b);
        }
    }
    let mut set = SetList::new(&recipe.name.clone().unwrap_or_else(|| {
        format!("{} {}×{} #{}", recipe.genres.join("→"), n, recipe.bars, recipe.seed)
    }));
    let mut last_kind: Option<TransitionKind> = None;
    for (i, (source, genre)) in sources.into_iter().enumerate() {
        let mut seg = Segment::new(source, clamp_bars(recipe.bars as i64));
        let kind = if i + 1 == n {
            TransitionKind::Move(MoveKind::EchoOut)
        } else if recipe.vary {
            let pool: Vec<TransitionKind> = TransitionKind::VARIETY.into_iter().filter(|k| Some(*k) != last_kind).collect();
            pool[rng.next_int(pool.len())]
        } else {
            recipe.transition
        };
        last_kind = Some(kind);
        seg.transition = Transition::new(kind);
        // always drawn, so the draws after it do not shift when the inserts are switched
        let throw_it = rng.next() < 0.6;
        if recipe.inserts {
            let sweep = match kind {
                TransitionKind::Move(MoveKind::EchoOut) => Some(0.3),
                TransitionKind::Move(MoveKind::LoopRoll) => Some(0.74),
                TransitionKind::Move(MoveKind::FilterExit) => Some(0.36),
                _ => None,
            };
            if let Some(a) = sweep {
                seg.inserts.push(InsertAuto::new(InsertKind::FilterSweep, 4.min(seg.bars / 2).max(1), a));
            }
            if throw_it && i + 1 != n {
                seg.inserts.push(InsertAuto::new(InsertKind::DelayThrow, 2, 0.5));
            }
        }
        seg.tag = Tag { energy: Some(energy_curve(i, n)), genre: Some(genre) };
        set.segments.push(seg);
    }
    Ok(set)
}
