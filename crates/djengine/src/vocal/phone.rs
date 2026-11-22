//! ARPAbet phones, syllables, and how a syllable maps onto the formant voice
//! ([`djrust::fx::formant::Syllable`]).
//!
//! The voice's 39 phones, named as ARPAbet names them; a lyric writes them in the BeatScript phoneme set
//! ([`super::phoneme`], docs/btw/phonemes.md), which maps onto them one to one. A word's phones are cut into
//! syllables by the maximal-onset rule (a consonant run between two vowels gives the second
//! syllable the longest legal English onset), and each syllable becomes one sung note:
//!
//! - the consonant nearest the vowel is the note's **onset**, and the one right after it its
//!   **coda** (the voice closes on it when the note is let go);
//! - the other consonants of a cluster (the `s` and `t` of "street", the `s` of "tracks") are
//!   **cluster** events: a short noise or burst just before the note or just after it.
//!
//! Pure: no I/O; `no_std` + `alloc`.

use alloc::vec::Vec;

use djrust::fx::formant::{Onset, Syllable};

/// A phone (ARPAbet's names), without its stress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sym {
    AA,
    AE,
    AH,
    AO,
    AW,
    AY,
    EH,
    ER,
    EY,
    IH,
    IY,
    OW,
    OY,
    UH,
    UW,
    B,
    CH,
    D,
    DH,
    F,
    G,
    HH,
    JH,
    K,
    L,
    M,
    N,
    NG,
    P,
    R,
    S,
    SH,
    T,
    TH,
    V,
    W,
    Y,
    Z,
    ZH,
}

const NAMES: [(&str, Sym); 39] = [
    ("AA", Sym::AA),
    ("AE", Sym::AE),
    ("AH", Sym::AH),
    ("AO", Sym::AO),
    ("AW", Sym::AW),
    ("AY", Sym::AY),
    ("EH", Sym::EH),
    ("ER", Sym::ER),
    ("EY", Sym::EY),
    ("IH", Sym::IH),
    ("IY", Sym::IY),
    ("OW", Sym::OW),
    ("OY", Sym::OY),
    ("UH", Sym::UH),
    ("UW", Sym::UW),
    ("B", Sym::B),
    ("CH", Sym::CH),
    ("D", Sym::D),
    ("DH", Sym::DH),
    ("F", Sym::F),
    ("G", Sym::G),
    ("HH", Sym::HH),
    ("JH", Sym::JH),
    ("K", Sym::K),
    ("L", Sym::L),
    ("M", Sym::M),
    ("N", Sym::N),
    ("NG", Sym::NG),
    ("P", Sym::P),
    ("R", Sym::R),
    ("S", Sym::S),
    ("SH", Sym::SH),
    ("T", Sym::T),
    ("TH", Sym::TH),
    ("V", Sym::V),
    ("W", Sym::W),
    ("Y", Sym::Y),
    ("Z", Sym::Z),
    ("ZH", Sym::ZH),
];

impl Sym {
    pub fn parse(s: &str) -> Option<Sym> {
        NAMES.iter().find(|(n, _)| *n == s).map(|(_, v)| *v)
    }

    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(_, v)| *v == self).map_or("", |(n, _)| *n)
    }

    pub fn is_vowel(self) -> bool {
        (self as u8) <= Sym::UW as u8
    }

    /// The formant voice's consonant for this phone (None for a vowel).
    pub fn onset(self) -> Option<Onset> {
        Some(match self {
            Sym::B => Onset::B,
            Sym::CH => Onset::Ch,
            Sym::D => Onset::D,
            Sym::DH => Onset::Dh,
            Sym::F => Onset::F,
            Sym::G => Onset::G,
            Sym::HH => Onset::H,
            Sym::JH => Onset::Jh,
            Sym::K => Onset::K,
            Sym::L => Onset::L,
            Sym::M => Onset::M,
            Sym::N => Onset::N,
            Sym::NG => Onset::Ng,
            Sym::P => Onset::P,
            Sym::R => Onset::R,
            Sym::S => Onset::S,
            Sym::SH => Onset::Sh,
            Sym::T => Onset::T,
            Sym::TH => Onset::Th,
            Sym::V => Onset::V,
            Sym::W => Onset::W,
            Sym::Y => Onset::Y,
            Sym::Z => Onset::Z,
            Sym::ZH => Onset::Zh,
            _ => return None,
        })
    }

    /// The vowel's index in the formant table, and its second vowel when it is a diphthong
    /// (a e i o u ah oo ae ih er = 0..9).
    pub fn vowel(self) -> Option<(u8, Option<u8>)> {
        Some(match self {
            Sym::AA => (0, None),
            Sym::AE => (7, None),
            Sym::AH => (5, None),
            Sym::AO => (3, None),
            Sym::AW => (0, Some(6)),
            Sym::AY => (0, Some(8)),
            Sym::EH => (1, None),
            Sym::ER => (9, None),
            Sym::EY => (1, Some(8)),
            Sym::IH => (8, None),
            Sym::IY => (2, None),
            Sym::OW => (3, Some(6)),
            Sym::OY => (3, Some(8)),
            Sym::UH => (6, None),
            Sym::UW => (4, None),
            _ => return None,
        })
    }
}

/// A phone with its stress (0 none, 1 primary, 2 secondary; consonants 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ph {
    pub sym: Sym,
    pub stress: u8,
}

/// `"K L AO1 D"` to phones. A token that is not a phone is skipped.
pub fn parse_phones(text: &str) -> Vec<Ph> {
    text.split_ascii_whitespace()
        .filter_map(|t| {
            let (base, stress) = match t.as_bytes().last() {
                Some(d @ b'0'..=b'2') => (&t[..t.len() - 1], d - b'0'),
                _ => (t, 0),
            };
            Sym::parse(base).map(|sym| Ph { sym, stress })
        })
        .collect()
}

/// One syllable of a word: its consonants either side of the vowel.
#[derive(Clone, Debug, PartialEq)]
pub struct Syl {
    pub onset: Vec<Sym>,
    pub nucleus: Ph,
    pub coda: Vec<Sym>,
}

fn legal_onset(c: &[Sym]) -> bool {
    use Sym::*;
    match c {
        [] => false,
        [NG] => false,
        [_] => true,
        [S, x] => matches!(x, P | T | K | M | N | L | W | F),
        [a, R] => matches!(a, P | B | T | D | K | G | F | TH | SH),
        [a, L] => matches!(a, P | B | K | G | F),
        [a, W] => matches!(a, T | D | K | G | TH),
        [a, Y] => matches!(a, P | B | T | D | K | G | F | V | M | N | HH | L | S | Z | TH),
        [S, a, R] => matches!(a, P | T | K),
        [S, a, L] => matches!(a, P | K),
        [S, K, W] | [S, P, Y] | [S, T, Y] | [S, K, Y] => true,
        _ => false,
    }
}

/// Cut a word's phones into syllables, the maximal onset to each. A word with no vowel (an
/// odd spelling) is one syllable on a schwa.
pub fn syllabify(phones: &[Ph]) -> Vec<Syl> {
    let nuclei: Vec<usize> = phones
        .iter()
        .enumerate()
        .filter(|(_, p)| p.sym.is_vowel())
        .map(|(i, _)| i)
        .collect();
    if nuclei.is_empty() {
        return alloc::vec![Syl {
            onset: phones.iter().map(|p| p.sym).collect(),
            nucleus: Ph { sym: Sym::AH, stress: 0 },
            coda: Vec::new(),
        }];
    }
    let mut out: Vec<Syl> = Vec::new();
    for (k, &n) in nuclei.iter().enumerate() {
        let prev_end = if k == 0 { 0 } else { nuclei[k - 1] + 1 };
        let run: Vec<Sym> = phones[prev_end..n].iter().map(|p| p.sym).collect();
        let (give_to_prev, onset): (usize, Vec<Sym>) = if k == 0 {
            (0, run)
        } else {
            // the longest legal suffix of the run is the onset; the rest closes the last syllable
            let mut cut = run.len();
            for start in 0..run.len() {
                if run.len() - start <= 3 && legal_onset(&run[start..]) {
                    cut = start;
                    break;
                }
            }
            (cut, run[cut..].to_vec())
        };
        if k > 0 {
            let prev = out.last_mut().unwrap();
            prev.coda.extend_from_slice(&phones[prev_end..prev_end + give_to_prev].iter().map(|p| p.sym).collect::<Vec<_>>());
        }
        out.push(Syl { onset, nucleus: phones[n], coda: Vec::new() });
    }
    // the word's last consonants close its last syllable
    let last = *nuclei.last().unwrap();
    let tail: Vec<Sym> = phones[last + 1..].iter().map(|p| p.sym).collect();
    out.last_mut().unwrap().coda.extend(tail);
    out
}

/// A syllable as the voice sings it: the note's own syllable, and the consonants around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sung {
    /// Cluster consonants before the note (earliest first).
    pub pre: Vec<Onset>,
    pub syl: Syllable,
    /// Cluster consonants after the coda.
    pub post: Vec<Onset>,
    pub stress: u8,
}

/// Map a syllable onto the voice (see the module doc).
pub fn to_sung(s: &Syl) -> Sung {
    let (v, g) = s.nucleus.sym.vowel().unwrap_or((5, None));
    let mut pre: Vec<Onset> = s.onset.iter().filter_map(|c| c.onset()).collect();
    let onset = pre.pop().unwrap_or(Onset::None);
    let mut coda_it = s.coda.iter().filter_map(|c| c.onset());
    let coda = coda_it.next().unwrap_or(Onset::None);
    // every cluster member is sung: no phoneme renders silent (a hum, an l and an h sound on their own too)
    let post: Vec<Onset> = coda_it.collect();
    Sung {
        pre,
        syl: Syllable { onset, vowel: v, glide: g, coda },
        post,
        stress: s.nucleus.stress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn syls(p: &str) -> Vec<(Vec<&'static str>, &'static str, Vec<&'static str>)> {
        syllabify(&parse_phones(p))
            .iter()
            .map(|s| {
                (
                    s.onset.iter().map(|x| x.name()).collect(),
                    s.nucleus.sym.name(),
                    s.coda.iter().map(|x| x.name()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn phones_parse_with_stress() {
        let p = parse_phones("K L AO1 D");
        assert_eq!(p.len(), 4);
        assert_eq!((p[2].sym, p[2].stress), (Sym::AO, 1));
        assert!(Sym::AO.is_vowel() && !Sym::K.is_vowel() && Sym::UW.is_vowel() && !Sym::B.is_vowel());
        for (n, s) in NAMES {
            assert_eq!(s.name(), n);
            assert_eq!(Sym::parse(n), Some(s));
            assert_eq!(s.vowel().is_some(), s.is_vowel());
            assert_eq!(s.onset().is_some(), !s.is_vowel());
        }
    }

    #[test]
    fn syllables_take_the_maximal_onset() {
        // robot: R OW1 B AA2 T
        assert_eq!(
            syls("R OW1 B AA2 T"),
            vec![(vec!["R"], "OW", vec![]), (vec!["B"], "AA", vec!["T"])]
        );
        // pastor / extra: s + stop is an onset
        assert_eq!(syls("EH1 K S T R AH0"), vec![(vec![], "EH", vec!["K"]), (vec!["S", "T", "R"], "AH", vec![])]);
        // anything: a single consonant between vowels starts the next syllable; NG ends one
        assert_eq!(
            syls("EH1 N IY0 TH IH2 NG"),
            vec![(vec![], "EH", vec![]), (vec!["N"], "IY", vec![]), (vec!["TH"], "IH", vec!["NG"])]
        );
        // a one-syllable word keeps all its consonants
        assert_eq!(syls("S T R IY1 T S"), vec![(vec!["S", "T", "R"], "IY", vec!["T", "S"])]);
    }

    #[test]
    fn a_syllable_becomes_a_note_with_cluster_events_around_it() {
        let s = syllabify(&parse_phones("S T R IY1 T S"));
        let m = to_sung(&s[0]);
        assert_eq!(m.pre, vec![Onset::S, Onset::T]);
        assert_eq!(m.syl.onset, Onset::R);
        assert_eq!(m.syl.vowel, 2);
        assert_eq!(m.syl.coda, Onset::T);
        assert_eq!(m.post, vec![Onset::S]);
        assert_eq!(m.stress, 1);
        // "right": R AY1 T -> r, a+ih, .t
        let r = to_sung(&syllabify(&parse_phones("R AY1 T"))[0]);
        assert_eq!(r.syl.text(), "ra+ih.t");
        assert!(r.pre.is_empty() && r.post.is_empty());
    }
}
