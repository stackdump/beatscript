//! Pitch words: notes (`c3`, `f#2`, `bb4`), keys (`am`, `d:dorian`) and chords (`am7`), and the
//! scales the key names (the generator's own tables, `djrust::gen::core::scale_intervals`).

use djengine::gen::core::scale_intervals;

/// A pitch class 0..11 (C = 0) from `a`..`g` and an accidental.
fn pitch_class(letter: char, acc: i32) -> u8 {
    let base = match letter {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        _ => 11,
    };
    (base + acc).rem_euclid(12) as u8
}

/// The tonic at the start of `s`: letter and accidental, and the rest.
fn tonic(s: &str) -> Option<(u8, &str)> {
    let mut it = s.chars();
    let l = it.next().map(|c| c.to_ascii_lowercase()).filter(|c| ('a'..='g').contains(c))?;
    let rest = it.as_str();
    match rest.chars().next() {
        Some('#') => Some((pitch_class(l, 1), &rest[1..])),
        // a `b` after the letter is a flat unless it is the start of a quality or a mode (`bb`, `ab`,
        // but not the `b` of `...b5`): a flat is only read where the rest cannot be a word of its own
        Some('b') => Some((pitch_class(l, -1), &rest[1..])),
        _ => Some((pitch_class(l, 0), rest)),
    }
}

/// `c3`, `f#2`, `bb4`, `a` (octave: None). Octave 4 is middle C.
pub fn parse_note(s: &str) -> Option<(u8, Option<i32>)> {
    let lower = s.to_ascii_lowercase();
    let (pc, rest) = tonic(&lower)?;
    if rest.is_empty() {
        return Some((pc, None));
    }
    let oct: i32 = rest.parse().ok()?;
    (-1..=9).contains(&oct).then_some((pc, Some(oct)))
}

/// The canonical spelling of a pitch word: a note (`Bb4` is `a#4`), a key (`Bbm` is `a#m`,
/// `D:Dorian` is `d:dorian`) or a chord (`Am7`, `BBMAJ7`): lower case, sharps for flats. A word that is
/// none of those is left as it is. Where a name is both (`am`, `c`) the spelling is the same.
pub fn canonical_pitch_word(w: &str) -> String {
    const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    let lower = w.to_ascii_lowercase();
    let is_pitch = parse_note(&lower).is_some() || Key::parse(&lower).is_some() || Chord::parse(&lower).is_some();
    if !is_pitch {
        return w.to_string();
    }
    match tonic(&lower) {
        Some((pc, rest)) => format!("{}{}", SHARP[usize::from(pc)], rest),
        None => w.to_string(),
    }
}

/// MIDI number of pitch class `pc` in octave `oct` (C4 = 60).
pub fn midi(pc: u8, oct: i32) -> i32 {
    12 * (oct + 1) + i32::from(pc)
}

/// The octave a part of this role plays in before `oct=` says another: bass 2, harmony 3, the rest 4.
pub fn role_octave(role: &str) -> i32 {
    match role {
        "bass" => 2,
        "harmony" => 3,
        _ => 4,
    }
}

pub const MODES: [&str; 9] = ["major", "minor", "dorian", "mixolydian", "phrygian", "harmonic", "pent", "minpent", "blues"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub tonic: u8,
    pub mode: usize,
}

impl Key {
    pub const C_MAJOR: Key = Key { tonic: 0, mode: 0 };

    /// `am`, `c`, `f#m`, `bbmaj`, `d:dorian`, `a:minor`.
    pub fn parse(s: &str) -> Option<Key> {
        let lower = s.to_ascii_lowercase();
        let s = lower.as_str();
        if let Some((t, m)) = s.split_once(':') {
            let (pc, rest) = tonic(t)?;
            if !rest.is_empty() {
                return None;
            }
            let mode = MODES.iter().position(|x| *x == m)?;
            return Some(Key { tonic: pc, mode });
        }
        let (pc, rest) = tonic(s)?;
        let mode = match rest {
            "" | "maj" | "major" => 0,
            "m" | "min" | "minor" => 1,
            _ => return None,
        };
        Some(Key { tonic: pc, mode })
    }

    pub fn intervals(self) -> &'static [i64] {
        scale_intervals(match self.mode {
            0 => "MajorScale",
            1 => "MinorScale",
            2 => "DorianScale",
            3 => "MixolydianScale",
            4 => "PhrygianScale",
            5 => "HarmonicMinScale",
            6 => "PentatonicScale",
            7 => "MinPentatonicScale",
            _ => "BluesScale",
        })
    }

    /// Scale degree `deg` (1 = the tonic; 8 on a seven-note scale = the tonic an octave up) in `oct`.
    pub fn degree(self, deg: u32, oct: i32) -> i32 {
        let iv = self.intervals();
        let i = (deg.max(1) - 1) as usize;
        midi(self.tonic, oct) + iv[i % iv.len()] as i32 + 12 * (i / iv.len()) as i32
    }

    /// MIDI note `m` moved `k` degrees along this key's scale, so the result is a tone of the key: a note off the scale
    /// snaps down to a scale tone first (as the arrange pitch operators do). `k` = the scale's length is an octave.
    pub fn shift_degrees(self, m: i32, k: i64) -> i32 {
        let iv = self.intervals();
        let n = iv.len() as i64;
        let rel = i64::from(m - i32::from(self.tonic));
        let (oct, pc) = (rel.div_euclid(12), rel.rem_euclid(12));
        let pos = oct * n + iv.iter().rposition(|x| *x <= pc).unwrap_or(0) as i64 + k;
        (i64::from(self.tonic) + pos.div_euclid(n) * 12 + iv[pos.rem_euclid(n) as usize]) as i32
    }

    /// The name as it is written back: `am`, `c`, `d:dorian`.
    pub fn text(self) -> String {
        const NAMES: [&str; 12] = ["c", "c#", "d", "eb", "e", "f", "f#", "g", "ab", "a", "bb", "b"];
        let n = NAMES[usize::from(self.tonic)];
        match self.mode {
            0 => n.to_string(),
            1 => format!("{n}m"),
            m => format!("{n}:{}", MODES[m]),
        }
    }
}

/// A chord: its root and the semitones above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub root: u8,
    pub intervals: Vec<i32>,
}

const QUALITIES: [(&str, &[i32]); 16] = [
    ("m7b5", &[0, 3, 6, 10]),
    ("maj7", &[0, 4, 7, 11]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("maj", &[0, 4, 7]),
    ("min", &[0, 3, 7]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("m7", &[0, 3, 7, 10]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("m6", &[0, 3, 7, 9]),
    ("m", &[0, 3, 7]),
    ("7", &[0, 4, 7, 10]),
    ("9", &[0, 4, 7, 10, 14]),
    ("6", &[0, 4, 7, 9]),
    ("5", &[0, 7]),
];

impl Chord {
    /// `am7`, `c`, `cmaj7`, `f#m7b5`, `g7`, `dsus4`.
    pub fn parse(s: &str) -> Option<Chord> {
        let lower = s.to_ascii_lowercase();
        let (root, rest) = tonic(&lower)?;
        if rest.is_empty() {
            return Some(Chord { root, intervals: vec![0, 4, 7] });
        }
        QUALITIES.iter().find(|(q, _)| *q == rest).map(|(_, iv)| Chord { root, intervals: iv.to_vec() })
    }

    /// The notes with the root in octave `oct`.
    pub fn notes(&self, oct: i32) -> Vec<i32> {
        self.intervals.iter().map(|i| midi(self.root, oct) + i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes() {
        assert_eq!(parse_note("c3"), Some((0, Some(3))));
        assert_eq!(parse_note("f#2"), Some((6, Some(2))));
        assert_eq!(parse_note("bb4"), Some((10, Some(4))));
        assert_eq!(parse_note("b"), Some((11, None)));
        assert_eq!(parse_note("eb"), Some((3, None)));
        assert_eq!(parse_note("h3"), None);
        assert_eq!(parse_note("c3x"), None);
        assert_eq!(midi(0, 4), 60);
    }

    #[test]
    fn keys_and_chords() {
        assert_eq!(Key::parse("am"), Some(Key { tonic: 9, mode: 1 }));
        assert_eq!(Key::parse("c"), Some(Key::C_MAJOR));
        assert_eq!(Key::parse("d:dorian"), Some(Key { tonic: 2, mode: 2 }));
        assert_eq!(Key::parse("d:nope"), None);
        assert_eq!(Key::parse("am").unwrap().text(), "am");
        assert_eq!(Key::parse("f#m").unwrap().degree(1, 3), 54);
        assert_eq!(Chord::parse("am7").unwrap().intervals, vec![0, 3, 7, 10]);
        assert_eq!(Chord::parse("c").unwrap().notes(4), vec![60, 64, 67]);
        assert_eq!(Chord::parse("f#m7b5").unwrap().root, 6);
        assert_eq!(Chord::parse("cx"), None);
    }

    #[test]
    fn degrees_climb_the_scale() {
        let a = Key::parse("am").unwrap();
        assert_eq!(a.degree(1, 2), 45);
        assert_eq!(a.degree(3, 2), 48);
        assert_eq!(a.degree(8, 2), 57);
    }
}
