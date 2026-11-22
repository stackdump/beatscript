//! Lyrics to notes: the syllables of a text laid onto a bar grid in a key (docs/VOICE.md).
//!
//! The fit is deterministic (a seed from the lyrics, the key and the user's seed), and its
//! result is written into the vocal line's canonical JSON: a later change to this algorithm
//! cannot move a saved line.
//!
//! - **Rhythm.** The syllables are spread over the bars evenly, and inside a bar they take the
//!   strongest steps of a sixteenth grid first (the beat, the half bar, the backbeats, the
//!   off-beats): four syllables sit on the quarters, eight on the eighths, and more fill in
//!   the sixteenths. A note lasts until the next one, with a sixteenth of air before it, at most
//!   half a bar; a phrase's last note rings on.
//! - **Pitch.** A walk in the key's scale. A word's stressed syllable lands on a chord tone
//!   (root, third or fifth), the others step; the last note of the line comes home to the root.
//!   The register is around the middle of a voice (A3 to G4 for the root).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use djrust::fx::formant::Onset;
use djrust::rng::Mulberry32;

use super::lexicon::{pronounce, words, Via};
use super::phoneme::esc_phones;
use super::phone::{syllabify, to_sung, Sung};
use crate::key::Key;

/// A syllable of the lyrics, before it has a note.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    /// The word it belongs to (on its first syllable; "" on the next ones).
    pub word: String,
    pub sung: Sung,
    /// The note to come is a rest (a `_` in the lyrics).
    pub rest: bool,
    /// Its word ends a phrase, and this is its last syllable.
    pub phrase_end: bool,
    pub via: Via,
    /// `:` in an escape: the syllable is held over one more note.
    pub hold: bool,
    /// `!` in an escape: the syllable is accented.
    pub accent: bool,
}

/// The lyrics' syllables, in order. An escaped word (`/…/`) is sung as written, whatever lexicon is there.
pub fn syllables(lyrics: &str) -> Vec<Slot> {
    let mut out = Vec::new();
    for w in words(lyrics) {
        if w.text == "_" {
            out.push(Slot {
                word: String::new(),
                sung: Sung { pre: Vec::new(), syl: djrust::fx::formant::Syllable::vowel(5), post: Vec::new(), stress: 0 },
                rest: true,
                phrase_end: w.phrase_end,
                via: Via::Lexicon,
                hold: false,
                accent: false,
            });
            continue;
        }
        let (phones, via, mods) = match &w.escaped {
            Some(e) => {
                let mods: Vec<(bool, bool)> = e.iter().filter(|p| p.sym.is_vowel()).map(|p| (p.hold, p.accent)).collect();
                (esc_phones(e), Via::Escape, mods)
            }
            None => {
                let (p, via) = pronounce(&w.text);
                (p, via, Vec::new())
            }
        };
        if via == Via::Escape && !phones.iter().any(|p| p.sym.is_vowel()) {
            // consonants on their own (`/s/`, `/sh/`): one note of them, no vowel added
            let cs: Vec<Onset> = phones.iter().filter_map(|p| p.sym.onset()).collect();
            if let Some((first, rest)) = cs.split_first() {
                out.push(Slot {
                    word: w.text.clone(),
                    sung: Sung { pre: Vec::new(), syl: djrust::fx::formant::Syllable::consonant(*first), post: rest.to_vec(), stress: 0 },
                    rest: false,
                    phrase_end: w.phrase_end,
                    via,
                    hold: false,
                    accent: false,
                });
            }
            continue;
        }
        let syls = syllabify(&phones);
        let n = syls.len();
        for (k, s) in syls.iter().enumerate() {
            let (hold, accent) = mods.get(k).copied().unwrap_or((false, false));
            out.push(Slot {
                word: if k == 0 { w.text.clone() } else { String::new() },
                sung: to_sung(s),
                rest: false,
                phrase_end: w.phrase_end && k + 1 == n,
                via,
                hold,
                accent,
            });
        }
    }
    out
}

/// How many notes a lyric takes: a syllable each, a held one two, a rest one.
pub fn notes_needed(slots: &[Slot]) -> usize {
    slots.iter().map(|s| 1 + usize::from(s.hold)).sum()
}

/// The words of a lyric the rules sounded out (no lexicon knew them): what to respell, escape or propose.
pub fn ruled_words(lyrics: &str) -> Vec<String> {
    let mut v: Vec<String> = syllables(lyrics).into_iter().filter(|s| s.via == Via::Rules && !s.word.is_empty()).map(|s| s.word).collect();
    v.dedup();
    v
}

/// A note's place and pitch (beats from the start of the loop).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub start: f64,
    pub dur: f64,
    pub midi: u8,
}

/// Step priority inside a bar of sixteenths: the strongest first.
const PRIORITY: [usize; 16] = [0, 8, 4, 12, 6, 10, 2, 14, 7, 11, 3, 15, 5, 9, 1, 13];

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

/// The hash the seed starts from (FNV-1a).
fn fnv(text: &str, key: Key, seed: u32) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in text.bytes().chain([key.root, key.minor as u8]).chain(seed.to_le_bytes()) {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The most syllables `bars` bars take.
pub fn capacity(bars: u32) -> usize {
    bars as usize * 16
}

/// Lay `slots` on `bars` bars of 4/4 in `key`. Err when there are more syllables than sixteenths.
pub fn place(slots: &[Slot], key: Key, bars: u32, seed: u32, lyrics: &str) -> Result<Vec<Place>, String> {
    let n = slots.len();
    if bars == 0 {
        return Err("bars must be at least 1".to_string());
    }
    if n > capacity(bars) {
        return Err(alloc::format!("{n} syllables do not fit in {bars} bar(s): at most {}", capacity(bars)));
    }
    let mut rng = Mulberry32::new(fnv(lyrics, key, seed) as f64);
    let scale: &[i32; 7] = if key.minor { &MINOR } else { &MAJOR };
    // the root near A3..G4 (MIDI 57..=68)
    let mut root = 48 + key.root as i32;
    while root < 57 {
        root += 12;
    }
    while root > 68 {
        root -= 12;
    }
    let degree_midi = |d: i32| -> i32 {
        let oct = d.div_euclid(7);
        let idx = d.rem_euclid(7) as usize;
        root + 12 * oct + scale[idx]
    };
    let mut out = Vec::with_capacity(n);
    let mut k = 0usize;
    let mut deg: i32 = 0;
    for b in 0..bars as usize {
        let count = ((b + 1) * n) / bars as usize - (b * n) / bars as usize;
        let mut steps: Vec<usize> = PRIORITY[..count].to_vec();
        steps.sort_unstable();
        for (j, &s) in steps.iter().enumerate() {
            let slot = &slots[k];
            let next = steps.get(j + 1).copied().unwrap_or(16);
            let gap = next - s;
            let mut len = if gap >= 3 { gap - 1 } else { gap };
            len = len.min(if slot.phrase_end { 12 } else { 8 }).max(1);
            if slot.phrase_end && j + 1 == steps.len() {
                len = (16 - s).clamp(1, 12);
            }
            // pitch
            let last = k + 1 == n;
            let stressed = slot.sung.stress == 1 && !slot.word.is_empty();
            let t = rng.next();
            if last {
                deg = if t < 0.7 { 0 } else { 4 };
            } else if k == 0 {
                deg = if t < 0.6 { 0 } else { 2 };
            } else if stressed {
                // a chord tone within a fifth of here, not the one we are on
                let near: Vec<i32> = [0, 2, 4, 7, 9, -3, -1].into_iter().filter(|c| *c != deg && (*c - deg).abs() <= 4).collect();
                deg = if near.is_empty() { 0 } else { near[rng.next_int(near.len())] };
            } else {
                let step = if t < 0.35 { -1 } else if t < 0.7 { 1 } else if t < 0.85 { 0 } else if t < 0.93 { -2 } else { 2 };
                deg += step;
                // keep the line near home
                if deg > 9 {
                    deg -= 2;
                }
                if deg < -3 {
                    deg += 2;
                }
            }
            let midi = degree_midi(deg).clamp(36, 96) as u8;
            if !slot.rest {
                out.push(Place { start: (b * 16 + s) as f64 / 4.0, dur: len as f64 / 4.0, midi });
            } else {
                // a rest takes its place and sounds nothing: keep the index aligned
                out.push(Place { start: (b * 16 + s) as f64 / 4.0, dur: 0.0, midi });
            }
            k += 1;
        }
    }
    Ok(out)
}

/// Whether a cluster consonant is a stop (for the doc and the tests).
pub fn is_stop(o: Onset) -> bool {
    matches!(o, Onset::B | Onset::D | Onset::G | Onset::P | Onset::T | Onset::K)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Key {
        Key::parse(s).unwrap()
    }

    #[test]
    fn the_hook_has_eight_syllables_and_fits_two_bars() {
        let s = syllables("I P O, Claude is on the road");
        assert_eq!(s.len(), 8, "{:?}", s.iter().map(|x| x.word.as_str()).collect::<Vec<_>>());
        // claude and road are not in the BeatScript lexicon: the rules sound them out
        assert!(s.iter().filter(|x| !x.word.is_empty()).all(|x| x.via != Via::Escape));
        let p = place(&s, key("Bb minor"), 2, 0, "I P O, Claude is on the road").unwrap();
        assert_eq!(p.len(), 8);
        // four a bar: on the quarters
        assert_eq!(p[0].start, 0.0);
        assert_eq!(p[1].start, 1.0);
        assert_eq!(p[4].start, 4.0);
        // every note inside the loop and in the voice's register
        for n in &p {
            assert!(n.start + n.dur <= 8.0 + 1e-9);
            assert!((40..=90).contains(&n.midi));
        }
        // the last note is home: B flat (pitch class 10) or its fifth (5)
        assert!(matches!(p[7].midi % 12, 10 | 5), "{}", p[7].midi % 12);
    }

    #[test]
    fn notes_are_in_the_key() {
        let s = syllables("one two three four five six seven eight nine ten eleven twelve");
        for k in ["C major", "A minor", "F# minor", "Bb major"] {
            let p = place(&s, key(k), 2, 7, "x").unwrap();
            let kk = key(k);
            let scale: &[i32; 7] = if kk.minor { &MINOR } else { &MAJOR };
            for n in &p {
                let pc = ((n.midi as i32 - kk.root as i32).rem_euclid(12)) as i32;
                assert!(scale.contains(&pc), "{k}: {pc}");
            }
        }
    }

    #[test]
    fn the_fit_is_deterministic_and_the_seed_changes_the_tune() {
        let s = syllables("buy the macs buy the macs");
        let a = place(&s, key("A minor"), 2, 1, "t").unwrap();
        assert_eq!(a, place(&s, key("A minor"), 2, 1, "t").unwrap());
        let b = place(&s, key("A minor"), 2, 2, "t").unwrap();
        assert!(a.iter().zip(&b).any(|(x, y)| x.midi != y.midi));
        // the rhythm does not depend on the seed
        assert!(a.iter().zip(&b).all(|(x, y)| x.start == y.start));
    }

    #[test]
    fn too_many_syllables_are_refused_and_a_dense_line_uses_sixteenths() {
        let s = syllables(&"la ".repeat(40));
        assert!(place(&s, key("C major"), 2, 0, "x").is_err());
        let p = place(&s, key("C major"), 3, 0, "x").unwrap();
        assert_eq!(p.len(), 40);
        for w in p.windows(2) {
            assert!(w[1].start > w[0].start);
            assert!(w[0].start + w[0].dur <= w[1].start + 1e-9);
        }
    }
}
