//! Words to phones (docs/beatscript/phonemes.md, docs/VOICE.md "Typed lyrics").
//!
//! - **An escape** (`/w ee - aa r/`) is phonemes already: no lexicon is asked ([`super::phoneme`]).
//! - **The BeatScript lexicon** (`data/lexicon.txt`, CC0, ours): English words in the BeatScript phoneme set.
//! - **A booth dictionary**, only where one is installed ([`install_dictionary`]): the desktop DJ booth links a
//!   private crate with a larger, separately licensed dictionary. No public build links it (the browser, the
//!   site's server, the public crate): `scripts/check-public-build.sh` fails a build that does.
//! - **The rules.** Written here from the spelling conventions of English, nothing taken from espeak-ng or any
//!   GPL code: digraphs, soft `c` and `g`, the silent final `e` that lengthens the vowel before it, the common
//!   vowel pairs, `-ed` and `-es`. They are for names, coinages and typos: rough, and meant to be; a word they
//!   sound out is reported (`Via::Rules`), so it can be respelled, escaped or proposed for the lexicon.
//! - **Numbers** are spelled out (`124` is "one hundred twenty four").
//!
//! Pure: `no_std` + `alloc`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use once_cell::race::OnceBox;

use super::phone::{parse_phones, Ph, Sym};
use super::phoneme::{self, EscPhone, EscWord};

/// A dictionary a host installs (the booth's): a word to its phones.
pub type Dictionary = fn(&str) -> Option<Vec<Ph>>;

struct Installed {
    lookup: Dictionary,
    size: usize,
}

static INSTALLED: OnceBox<Installed> = OnceBox::new();

/// Install a host's dictionary (once; a second call is ignored). Words the BeatScript lexicon does not
/// know are looked up in it before the rules. Only the desktop booth does this.
pub fn install_dictionary(lookup: Dictionary, size: usize) {
    let _ = INSTALLED.set(alloc::boxed::Box::new(Installed { lookup, size }));
}

/// Whether a host dictionary is installed (never in a public build).
pub fn dictionary_installed() -> bool {
    INSTALLED.get().is_some()
}

/// How many words the lexicons hold: the BeatScript lexicon's, and an installed dictionary's.
pub fn dictionary_size() -> usize {
    phoneme::lexicon_size() + INSTALLED.get().map_or(0, |d| d.size)
}

/// The pronunciation the lexicons give `word` (lowercase letters and apostrophes): the BeatScript
/// lexicon's, else an installed dictionary's.
pub fn lookup(word: &str) -> Option<Vec<Ph>> {
    lookup_via(word).map(|(p, _)| p)
}

fn lookup_via(word: &str) -> Option<(Vec<Ph>, Via)> {
    if let Some(p) = phoneme::lexicon_lookup(word) {
        return Some((p, Via::Lexicon));
    }
    INSTALLED.get().and_then(|d| (d.lookup)(word)).filter(|p| !p.is_empty()).map(|p| (p, Via::Dictionary))
}

/// Where a pronunciation came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The BeatScript lexicon.
    Lexicon,
    /// A host's installed dictionary (the booth's).
    Dictionary,
    /// The letter-to-sound rules (the word is reported).
    Rules,
    /// Written in phonemes (`/…/`, a phrasebook phrase).
    Escape,
}

impl Via {
    pub fn name(self) -> &'static str {
        match self {
            Via::Lexicon => "lexicon",
            Via::Dictionary => "dictionary",
            Via::Rules => "rules",
            Via::Escape => "escape",
        }
    }
}

/// A word's phones: the lexicons', else the rules'. `word` is lowercase letters and apostrophes.
pub fn pronounce(word: &str) -> (Vec<Ph>, Via) {
    if let Some(x) = lookup_via(word) {
        return x;
    }
    // possessives and plurals of known words: "claude's", "macs"
    for suffix in ["'s", "s"] {
        if let Some(stem) = word.strip_suffix(suffix) {
            if stem.len() >= 2 {
                if let Some((mut p, via)) = lookup_via(stem) {
                    let last = p.last().map(|x| x.sym);
                    let add = match last {
                        Some(Sym::S | Sym::Z | Sym::SH | Sym::ZH | Sym::CH | Sym::JH) => alloc::vec![Sym::IH, Sym::Z],
                        Some(Sym::P | Sym::T | Sym::K | Sym::F | Sym::TH) => alloc::vec![Sym::S],
                        _ => alloc::vec![Sym::Z],
                    };
                    for s in add {
                        p.push(Ph { sym: s, stress: 0 });
                    }
                    return (p, via);
                }
            }
        }
    }
    (rules(word), Via::Rules)
}

// MARK: letter-to-sound

fn is_vowel_letter(c: u8) -> bool {
    matches!(c, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// Letter-to-sound rules for English spelling.
pub fn rules(word: &str) -> Vec<Ph> {
    let w: Vec<u8> = word.bytes().filter(|b| b.is_ascii_lowercase() || *b == b'\'').filter(|b| *b != b'\'').collect();
    let n = w.len();
    let mut out: Vec<Sym> = Vec::new();
    if n == 0 {
        return Vec::new();
    }
    // a lone letter reads as its name
    if n == 1 {
        return letter_name(w[0]);
    }
    let at = |i: isize| -> u8 { if i < 0 || i as usize >= n { 0 } else { w[i as usize] } };
    let starts = |i: usize, s: &str| w[i..].starts_with(s.as_bytes());
    // a final silent e: consonant + e at the end, with a vowel earlier
    let has_vowel_before = |i: usize| w[..i].iter().any(|c| is_vowel_letter(*c) || *c == b'y');
    let silent_e = n >= 3 && w[n - 1] == b'e' && !is_vowel_letter(w[n - 2]) && has_vowel_before(n - 2);
    let end = if silent_e { n - 1 } else { n };
    // -ed / -es endings
    let mut tail: Option<Vec<Sym>> = None;
    let mut stop = end;
    if n > 3 && w.ends_with(b"ed") && !is_vowel_letter(w[n - 3]) && has_vowel_before(n - 2) {
        stop = n - 2;
        tail = Some(if matches!(w[n - 3], b't' | b'd') {
            alloc::vec![Sym::IH, Sym::D]
        } else if matches!(w[n - 3], b'p' | b'k' | b'f' | b's' | b'x') || (w[n - 3] == b'h' && n > 4 && matches!(w[n - 4], b's' | b'c')) {
            alloc::vec![Sym::T]
        } else {
            alloc::vec![Sym::D]
        });
    } else if n > 3 && w.ends_with(b"es") && matches!(w[n - 3], b's' | b'x' | b'z' | b'h' | b'c' | b'g') {
        stop = n - 2;
        tail = Some(alloc::vec![Sym::IH, Sym::Z]);
    }
    let mut i = 0usize;
    while i < stop {
        let c = w[i];
        let next = at(i as isize + 1);
        let prev = at(i as isize - 1);
        let long_here = silent_e && i + 2 == n - 1 && !is_vowel_letter(next);
        // two-letter and three-letter units first
        macro_rules! eat {
            ($n:expr, $($s:expr),+) => {{ $(out.push($s);)+ i += $n; continue; }};
        }
        if starts(i, "tch") { eat!(3, Sym::CH) }
        if starts(i, "sch") { eat!(3, Sym::S, Sym::K) }
        if starts(i, "ch") { eat!(2, Sym::CH) }
        if starts(i, "sh") { eat!(2, Sym::SH) }
        if starts(i, "th") { eat!(2, Sym::TH) }
        if starts(i, "ph") { eat!(2, Sym::F) }
        if starts(i, "wh") { eat!(2, Sym::W) }
        if starts(i, "ck") { eat!(2, Sym::K) }
        if starts(i, "ng") { eat!(2, Sym::NG) }
        if starts(i, "nk") { eat!(2, Sym::NG, Sym::K) }
        if starts(i, "qu") { eat!(2, Sym::K, Sym::W) }
        if i == 0 && starts(i, "kn") { eat!(2, Sym::N) }
        if i == 0 && starts(i, "wr") { eat!(2, Sym::R) }
        if i == 0 && starts(i, "gn") { eat!(2, Sym::N) }
        if i + 2 == n && starts(i, "mb") { eat!(2, Sym::M) }
        if starts(i, "gh") {
            if i == 0 { eat!(2, Sym::G) }
            // night, though: silent after a vowel
            i += 2;
            continue;
        }
        // doubled consonants are one
        if !is_vowel_letter(c) && c == next && c != b'c' {
            i += 1;
            continue;
        }
        if is_vowel_letter(c) || (c == b'y' && i > 0 && !is_vowel_letter(next)) {
            // vowel pairs
            let pair = |s: &str| starts(i, s);
            if pair("ee") || pair("ea") { eat!(2, Sym::IY) }
            if pair("ai") || pair("ay") || pair("ei") || pair("ey") { eat!(2, Sym::EY) }
            if pair("oa") { eat!(2, Sym::OW) }
            if pair("oo") { eat!(2, Sym::UW) }
            if pair("ou") { eat!(2, Sym::AW) }
            if pair("ow") { if i + 2 >= n { eat!(2, Sym::OW) } else { eat!(2, Sym::AW) } }
            if pair("oi") || pair("oy") { eat!(2, Sym::OY) }
            if pair("au") || pair("aw") { eat!(2, Sym::AO) }
            if pair("ie") { eat!(2, Sym::IY) }
            if pair("ue") || pair("ew") || pair("ui") || pair("eu") { eat!(2, Sym::UW) }
            // vowel + r
            if next == b'r' && !is_vowel_letter(at(i as isize + 2)) {
                match c {
                    b'a' => eat!(2, Sym::AA, Sym::R),
                    b'o' => eat!(2, Sym::AO, Sym::R),
                    _ => eat!(2, Sym::ER),
                }
            }
            let s = match (c, long_here) {
                (b'a', true) => alloc::vec![Sym::EY],
                (b'e', true) => alloc::vec![Sym::IY],
                (b'i', true) => alloc::vec![Sym::AY],
                (b'o', true) => alloc::vec![Sym::OW],
                (b'u', true) => alloc::vec![Sym::Y, Sym::UW],
                (b'a', _) => alloc::vec![Sym::AE],
                (b'e', _) => alloc::vec![Sym::EH],
                (b'i', _) => alloc::vec![Sym::IH],
                (b'o', _) => alloc::vec![Sym::AA],
                (b'u', _) => alloc::vec![Sym::AH],
                // y as a vowel: "my", "city"
                (b'y', _) => alloc::vec![if i + 1 == n { Sym::IY } else { Sym::IH }],
                _ => Vec::new(),
            };
            out.extend(s);
            i += 1;
            continue;
        }
        match c {
            b'b' => out.push(Sym::B),
            b'c' => out.push(if matches!(next, b'e' | b'i' | b'y') { Sym::S } else { Sym::K }),
            b'd' => out.push(Sym::D),
            b'f' => out.push(Sym::F),
            b'g' => out.push(if matches!(next, b'e' | b'i' | b'y') && i > 0 { Sym::JH } else { Sym::G }),
            b'h' => out.push(Sym::HH),
            b'j' => out.push(Sym::JH),
            b'k' => out.push(Sym::K),
            b'l' => out.push(Sym::L),
            b'm' => out.push(Sym::M),
            b'n' => out.push(Sym::N),
            b'p' => out.push(Sym::P),
            b'q' => out.push(Sym::K),
            b'r' => out.push(Sym::R),
            b's' => out.push(if is_vowel_letter(prev) && is_vowel_letter(next) { Sym::Z } else { Sym::S }),
            b't' => out.push(Sym::T),
            b'v' => out.push(Sym::V),
            b'w' => out.push(Sym::W),
            b'x' => {
                if i == 0 {
                    out.push(Sym::Z)
                } else {
                    out.push(Sym::K);
                    out.push(Sym::S)
                }
            }
            b'y' => out.push(Sym::Y),
            b'z' => out.push(Sym::Z),
            _ => {}
        }
        i += 1;
    }
    if let Some(t) = tail {
        out.extend(t);
    }
    // stress: the first vowel (a spelling has no better guess)
    let mut first = true;
    out.into_iter()
        .map(|sym| {
            let stress = if sym.is_vowel() && first {
                first = false;
                1
            } else {
                0
            };
            Ph { sym, stress }
        })
        .collect()
}

fn letter_name(c: u8) -> Vec<Ph> {
    let names: [&str; 26] = [
        "EY1", "B IY1", "S IY1", "D IY1", "IY1", "EH1 F", "JH IY1", "EY1 CH", "AY1", "JH EY1", "K EY1", "EH1 L", "EH1 M", "EH1 N",
        "OW1", "P IY1", "K Y UW1", "AA1 R", "EH1 S", "T IY1", "Y UW1", "V IY1", "D AH1 B AH0 L Y UW0", "EH1 K S", "W AY1", "Z IY1",
    ];
    parse_phones(names.get((c.wrapping_sub(b'a')) as usize).copied().unwrap_or(""))
}

// MARK: text

/// Spell a number out (0..=999 999) as lowercase words.
pub fn number_words(n: u64) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
        "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    fn below_thousand(n: u64, out: &mut Vec<String>) {
        if n >= 100 {
            out.push(ONES[(n / 100) as usize].to_string());
            out.push("hundred".to_string());
        }
        let r = n % 100;
        if r == 0 {
            return;
        }
        if r < 20 {
            out.push(ONES[r as usize].to_string());
        } else {
            out.push(TENS[(r / 10) as usize].to_string());
            if r % 10 != 0 {
                out.push(ONES[(r % 10) as usize].to_string());
            }
        }
    }
    if n == 0 {
        return "zero".to_string();
    }
    let mut parts = Vec::new();
    if n >= 1000 {
        below_thousand((n / 1000) % 1000, &mut parts);
        parts.push("thousand".to_string());
    }
    if n % 1000 != 0 {
        below_thousand(n % 1000, &mut parts);
    }
    parts.join(" ")
}

/// A lyric's word, with what follows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Lowercase letters and apostrophes, or `_` for a rest; an escaped word is written as it was (`/r ay v/`).
    pub text: String,
    /// A comma, full stop, line break or the like came after it: a phrase ends here.
    pub phrase_end: bool,
    /// An escaped word's phonemes (`/…/`): sung as written, no lexicon asked.
    pub escaped: Option<Vec<EscPhone>>,
}

/// Whether the `=` before byte `next` makes `word` (the letters right before it) the label of an annotated escape: an
/// escape starts at `next`, and `word` is letters and apostrophes.
pub fn annotates(word: &str, ranges: &[(usize, usize)], next: usize) -> bool {
    !word.trim_matches('\'').is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') && ranges.iter().any(|r| r.0 == next)
}

/// Lyrics text to words. Letters (and apostrophes) make words; digits are spelled; `-` splits a
/// word into pieces (`ro-bo-tic`); a lone `_` is a rest; punctuation and line breaks end a
/// phrase. Anything else is skipped.
///
/// **An annotated escape**, `word=/phonemes/` (`jingle=/j ii ng g uh l/`), is the escape and nothing else: the word
/// before the `=` is its label, there for whoever reads the lyric (and for the lexicon harvest, [`lyric_pairs`]), and is
/// not sung. The word is the letters (and apostrophes) touching the `=`; a hyphen ends it (`lan-tern=/…/` sings `lan`
/// and the escape).
pub fn words(text: &str) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut cur = String::new();
    let mut num = String::new();
    fn flush(out: &mut Vec<Word>, cur: &mut String, num: &mut String) {
        if !num.is_empty() {
            let n: u64 = num.parse().unwrap_or(0);
            for w in number_words(n.min(999_999)).split(' ') {
                out.push(Word { text: w.to_string(), phrase_end: false, escaped: None });
            }
            num.clear();
        }
        let t = cur.trim_matches('\'');
        if !t.is_empty() {
            out.push(Word { text: t.to_string(), phrase_end: false, escaped: None });
        }
        cur.clear();
    }
    // the escapes: a `/` with a phoneme after it, to the next `/`; a `/` with a space after it (or no partner) is a phrase
    // break, as it always was (`phoneme::escape_ranges`)
    let ranges = phoneme::escape_ranges(text);
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if let Some(&(s, e)) = ranges.iter().find(|r| r.0 == at) {
            flush(&mut out, &mut cur, &mut num);
            let (words, _) = phoneme::read_escape(&text[s + 1..e - 1], s + 1);
            for w in words {
                match w {
                    EscWord::Rest => out.push(Word { text: "_".to_string(), phrase_end: false, escaped: None }),
                    EscWord::Phones(p) => {
                        let shown = p.iter().map(|x| phoneme::symbol_of(x.sym)).collect::<Vec<_>>().join(" ");
                        out.push(Word { text: alloc::format!("/{shown}/"), phrase_end: false, escaped: Some(p) });
                    }
                }
            }
            while chars.peek().is_some_and(|(i, _)| *i < e) {
                chars.next();
            }
            continue;
        }
        let chars_peek_alpha = |chars: &mut core::iter::Peekable<core::str::CharIndices>| chars.peek().is_some_and(|(_, n)| n.is_ascii_alphabetic());
        // an annotated escape (`jingle=/j ii ng g uh l/`): the word is the escape's label, kept in the text and not sung
        if c == '=' && num.is_empty() && annotates(&cur, &ranges, at + 1) {
            cur.clear();
            continue;
        }
        if c.is_ascii_alphabetic() {
            if !num.is_empty() {
                flush(&mut out, &mut cur, &mut num);
            }
            cur.push(c.to_ascii_lowercase());
        } else if c == '\'' || c == '’' {
            if !cur.is_empty() && chars_peek_alpha(&mut chars) {
                cur.push('\'');
            }
        } else if c.is_ascii_digit() {
            if !cur.is_empty() {
                flush(&mut out, &mut cur, &mut num);
            }
            num.push(c);
        } else if c == '_' {
            flush(&mut out, &mut cur, &mut num);
            out.push(Word { text: "_".to_string(), phrase_end: false, escaped: None });
        } else if c == '-' {
            flush(&mut out, &mut cur, &mut num);
        } else if matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | '\n' | '\r' | '/' | '|') {
            flush(&mut out, &mut cur, &mut num);
            if let Some(w) = out.last_mut() {
                w.phrase_end = true;
            }
        } else {
            flush(&mut out, &mut cur, &mut num);
        }
    }
    flush(&mut out, &mut cur, &mut num);
    out
}

/// Where a word's pronunciation in a lyric came from, for the lexicon harvest (docs/beatscript/lexicon-harvest.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PairSource {
    /// `word=/phonemes/`: someone wrote the word's pronunciation down (a strong signal).
    Annotated,
    /// A word no lexicon knows, as the letter-to-sound rules sounded it out (a weak one).
    Rules,
}

impl PairSource {
    pub fn name(self) -> &'static str {
        match self {
            PairSource::Annotated => "annotated",
            PairSource::Rules => "rule",
        }
    }
}

/// A word of a lyric and the phonemes it was sung with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricPair {
    /// Lowercase letters and apostrophes.
    pub word: String,
    /// In the BeatScript phoneme set, one space between symbols, as a lexicon line writes them (`j ii ng g uh l`): no
    /// `-` cuts, `:` or `!` marks, or rests.
    pub phones: String,
    pub source: PairSource,
}

/// The (word, phonemes) pairs of a lyric that the lexicon could learn from, in order, each pair once: every annotated
/// escape (`jingle=/j ii ng g uh l/`) and every word no lexicon knows, with the rules' pronunciation. An escape whose
/// symbols are not all in the set gives no pair (check refuses it), nor does a word an author cut with hyphens
/// (`lan-tern`: its pieces are not words), a plain escape (it has no word) or a word a lexicon knows.
pub fn lyric_pairs(text: &str) -> Vec<LyricPair> {
    let mut out: Vec<LyricPair> = Vec::new();
    let mut push = |p: LyricPair| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    let ranges = phoneme::escape_ranges(text);
    for &(s, e) in &ranges {
        let Some(before) = text[..s].strip_suffix('=') else { continue };
        let word: String = before.chars().rev().take_while(|c| c.is_ascii_alphabetic() || *c == '\'' || *c == '’').collect::<Vec<_>>().into_iter().rev().collect();
        let word = word.replace('’', "'").to_ascii_lowercase();
        let word = word.trim_matches('\'');
        if word.is_empty() {
            continue;
        }
        let (words, issues) = phoneme::read_escape(&text[s + 1..e - 1], s + 1);
        if !issues.is_empty() {
            continue;
        }
        let phones: Vec<&str> = words
            .iter()
            .flat_map(|w| match w {
                EscWord::Phones(p) => p.iter().map(|x| phoneme::symbol_of(x.sym)).collect::<Vec<_>>(),
                EscWord::Rest => Vec::new(),
            })
            .collect();
        if phones.is_empty() {
            continue;
        }
        push(LyricPair { word: word.to_string(), phones: phones.join(" "), source: PairSource::Annotated });
    }
    // the words the rules sounded out; a word cut with hyphens is the author spelling it already (as check says)
    let cut: Vec<String> = text
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?'))
        .filter(|w| w.contains('-') && !w.contains('/'))
        .flat_map(|w| w.split('-').map(|x| x.to_lowercase()))
        .collect();
    for w in words(text) {
        if w.escaped.is_some() || w.text == "_" || cut.contains(&w.text) {
            continue;
        }
        let (p, via) = pronounce(&w.text);
        if via == Via::Rules && !p.is_empty() {
            push(LyricPair { word: w.text.clone(), phones: phoneme::spell(&p), source: PairSource::Rules });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn name(p: &[Ph]) -> String {
        p.iter().map(|x| x.sym.name()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_beatscript_lexicon_comes_first_and_no_dictionary_is_installed_here() {
        // a public build (this test binary) has the BeatScript lexicon and nothing else
        assert!(!dictionary_installed());
        assert_eq!(dictionary_size(), phoneme::lexicon_size());
        for (w, want) in [("the", "DH AH"), ("on", "AA N"), ("i", "AY"), ("rave", "R EY V"), ("bass", "B EY S")] {
            let (p, via) = pronounce(w);
            assert_eq!(via, Via::Lexicon, "{w}");
            assert_eq!(name(&p), want, "{w}");
        }
        // a single letter the lexicon lacks reads as its name, by the rules
        assert_eq!(name(&pronounce("p").0), "P IY");
        assert!(lookup("zzzxqv").is_none());
    }

    #[test]
    fn the_rules_give_a_plausible_sound_to_unknown_words() {
        let r = |w: &str| name(&rules(w));
        assert_eq!(r("zorp"), "Z AA R P".replace("AA R", "AO R"));
        assert_eq!(r("blike"), "B L AY K");
        assert_eq!(r("thun"), "TH AH N");
        assert_eq!(r("shoo"), "SH UW");
        assert_eq!(r("quib"), "K W IH B");
        assert_eq!(r("fleen"), "F L IY N");
        assert_eq!(r("grashed"), "G R AE SH T");
        assert_eq!(r("q"), "K Y UW");
        // a pronounce() of an unknown word goes to the rules
        let (_, via) = pronounce("zxorbl");
        assert_eq!(via, Via::Rules);
        // every output of the rules is a phone with one stressed vowel at most
        for w in ["anthropic", "kubernetes", "xylophone", "znork", "ipo"] {
            let p = rules(w);
            assert!(!p.is_empty(), "{w}");
            assert!(p.iter().filter(|x| x.stress == 1).count() <= 1);
        }
    }

    #[test]
    fn plurals_and_possessives_build_on_the_lexicon() {
        let (p, via) = pronounce("kicks");
        assert_eq!(via, Via::Lexicon);
        assert_eq!(p.last().unwrap().sym, Sym::S);
        let (p, _) = pronounce("rave's");
        assert_eq!(name(&p), "R EY V Z");
    }

    #[test]
    fn an_escape_is_a_word_of_its_own_and_a_lone_slash_still_ends_a_phrase() {
        let w = words("we are /dh uh/ rave / again");
        let t: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["we", "are", "/dh uh/", "rave", "again"]);
        assert!(w[2].escaped.is_some() && w[0].escaped.is_none());
        assert!(w[3].phrase_end, "a lone slash is a phrase break");
        let w = words("/h ay - _ - y aa:/");
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].text, "_");
        assert!(w[2].escaped.as_ref().unwrap()[1].hold);
    }

    #[test]
    fn a_phrase_break_next_to_an_escape_ends_the_phrase_and_opens_nothing() {
        // 6634: each of these is the same words, escapes and phrase ends as the spelling beside it, with a comma (or a
        // line break) for the break, which was never ambiguous
        for (slash, plain) in [
            ("/h ii/ ! / /h ii/ !", "/h ii/ !, /h ii/ !"),
            ("/h ii/ / /h aa/", "/h ii/, /h aa/"),
            ("/ /h ii/", "/h ii/"),
            ("/h ii/ /", "/h ii/,"),
            ("/h ii/!/ /h aa/", "/h ii/!, /h aa/"),
            ("/h ii/ / / /h aa/", "/h ii/\n\n/h aa/"),
            ("deep er in to night / hold me close / we move slow", "deep er in to night, hold me close, we move slow"),
            ("we /dh uh/ rave / and /dh uh/ rave", "we /dh uh/ rave, and /dh uh/ rave"),
            ("hello / /h ii/ / world", "hello, /h ii/, world"),
        ] {
            assert_eq!(words(slash), words(plain), "{slash:?} against {plain:?}");
        }
        // the record's lyric: two escaped words, a phrase ends after each
        let w = words("/h ii/ ! / /h ii/ !");
        assert_eq!(w.len(), 2);
        assert!(w.iter().all(|w| w.escaped.is_some() && w.phrase_end));
        // a lone break in plain words ends the phrase before it and starts none of an escape
        let w = words("night / hold me close / we move");
        let t: Vec<(&str, bool)> = w.iter().map(|w| (w.text.as_str(), w.phrase_end)).collect();
        assert_eq!(t, vec![("night", true), ("hold", false), ("me", false), ("close", true), ("we", false), ("move", false)]);
        assert!(w.iter().all(|w| w.escaped.is_none()));
    }

    #[test]
    fn an_annotated_escape_is_the_escape_and_its_word_is_not_sung() {
        // the word before `=` is the escape's label: the words are exactly the plain escape's
        assert_eq!(words("jingle=/j ii ng g uh l/ all the way"), words("/j ii ng g uh l/ all the way"));
        assert_eq!(words("we /dh uh/ rave, o'er=/oh r/ night"), words("we /dh uh/ rave, /oh r/ night"));
        // a word, a space, then `=`: not an annotation (the `=` is skipped as it always was, the word is sung)
        assert_eq!(words("jingle =/j ii/"), words("jingle /j ii/"));
        // `=` with no escape after it is as before
        assert_eq!(words("a=b"), words("a b"));
        // a hyphen ends the label: `lan` is sung
        assert_eq!(words("lan-tern=/l ae n t er n/"), words("lan /l ae n t er n/"));
        // digits are not a label
        assert_eq!(words("24=/t w eh n t ii/"), words("24 /t w eh n t ii/"));
    }

    #[test]
    fn lyric_pairs_are_the_annotations_and_the_words_the_rules_sounded_out() {
        let p = lyric_pairs("Jingle=/j ii ng g uh l/ zorblax the /dh uh/ way, jingle=/j ii ng g uh l/, lan-tern");
        let got: Vec<(&str, &str, PairSource)> = p.iter().map(|p| (p.word.as_str(), p.phones.as_str(), p.source)).collect();
        assert_eq!(got, vec![("jingle", "j ii ng g uh l", PairSource::Annotated), ("zorblax", "z aw r b l ae k s", PairSource::Rules)]);
        // marks, cuts and rests are not part of a lexicon line; a bad escape gives no pair; a plain escape has no word
        let p = lyric_pairs("hey=/h ay: - _/ bad=/zz/ /h ii/");
        assert_eq!(p, vec![LyricPair { word: "hey".into(), phones: "h ay".into(), source: PairSource::Annotated }]);
    }

    #[test]
    fn the_seventeen_hand_escaped_words_of_6665_are_in_the_lexicon() {
        // Matt approved them on 2026-11-22 by asking for the harvest: the words Claude had escaped by hand in that day's
        // tracks, so check no longer reports them
        let lyric = "beauty joy pass nothingness god rest ye merry gentlemen dismay jingle bells fun ride horse open sleigh";
        assert!(crate::vocal::fit::ruled_words(lyric).is_empty(), "{:?}", crate::vocal::fit::ruled_words(lyric));
        assert!(lyric_pairs(lyric).is_empty());
        for (w, want) in [("gentlemen", "JH EH N T AH L M AH N"), ("jingle", "JH IH NG G AH L"), ("bells", "B EH L Z"), ("open", "OW P AH N"), ("ye", "Y IY")] {
            assert_eq!(name(&lookup(w).unwrap()), want, "{w}");
        }
    }

    #[test]
    fn numbers_are_spelled_and_text_splits_into_words() {
        assert_eq!(number_words(124), "one hundred twenty four");
        assert_eq!(number_words(6_000_000 % 1_000_000 + 740), "seven hundred forty");
        assert_eq!(number_words(1000), "one thousand");
        let w = words("I P O, I P O, Claude is on the road!");
        let t: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["i", "p", "o", "i", "p", "o", "claude", "is", "on", "the", "road"]);
        assert!(w[2].phrase_end && w[5].phrase_end && w[10].phrase_end && !w[0].phrase_end);
        let w = words("ro-bo-tic _ it's 140");
        let t: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["ro", "bo", "tic", "_", "it's", "one", "hundred", "forty"]);
    }
}
