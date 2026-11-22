//! The BeatScript phoneme set v1 (docs/btw/phonemes.md): our own symbols for what the formant voice
//! sings, the `/…/` escape in lyrics, the BeatScript lexicon and the phrasebook.
//!
//! - **The set** is 15 vowels and 24 consonants in readable lowercase (`ee`, `ay`, `sh`, `dh`), plus `_`, a
//!   rest. It is part of the language, written for it, CC0. Each symbol maps one to one onto the voice's
//!   phone ([`Sym`]); the IPA and ARPAbet columns of [`SET`] are mappings for people bringing pronunciations
//!   from elsewhere, not data taken from anywhere.
//! - **The escape**: text between two slashes in a lyric is phonemes, space separated, and bypasses every
//!   lexicon: `sing "/w ee - aa r - dh uh - r ay v/" on melody`. `-` cuts it into words (syllables come from
//!   the maximal-onset rule inside a word, as for any word), `_` is a rest. A vowel may carry `:` (held: it
//!   takes one more note) and `!` (accented: sung harder), in either order. A `/` with a space after it
//!   (or at the lyric's end) is a phrase break, not the start of an escape; so is a slash with no partner.
//! - **The lexicon** (`data/lexicon.txt`): English words in the set, written by hand, CC0, versioned. Plain
//!   lyrics look a word up there first, then the letter-to-sound rules ([`super::lexicon::rules`]); a word the
//!   rules sounded out is reported, so its author can respell it or escape it.
//! - **The phrasebook** (`data/phrasebook.txt`): stock calls, pre-phonemized and tuned by hand, each with its
//!   syllable count and a suggested rhythm, for `sing @hands_up on melody`. Original phrases only.
//! - **The public-domain phrasebook** (`data/phrasebook-pd.txt`): lines of verse, scripture, speech and work song first
//!   published before 1929, each with its source, its meter as a step pattern and the lyric BeatScript sings it as, for
//!   `sing @tempest_such_stuff on melody`. CC0 as a file, every text in the public domain; one corpus, one CID.
//!
//! No CMUdict and no other licensed data are anywhere on this path: it is what the browser, the public site
//! and the public crate run. Pure: `no_std` + `alloc`.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use once_cell::race::OnceBox;

use super::phone::{Ph, Sym};

/// What kind of sound a symbol is, and so which source the voice gives it (docs/btw/phonemes.md, "Sounds").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A steady vowel: the voice through one set of formant targets.
    Vowel,
    /// Two vowels in one: the formants glide from the first target to the second.
    Diphthong,
    /// A closure and a burst (voiceless ones: then breath).
    Stop,
    /// Shaped noise (voiced ones: over a buzz).
    Fricative,
    /// Breath through the tract.
    Aspirate,
    /// A dark, closed-mouth hum.
    Nasal,
    /// Voiced, the formants gliding from the consonant's place.
    Liquid,
    /// Voiced, a fast formant glide into the vowel.
    Glide,
    /// A burst, then frication.
    Affricate,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Vowel => "vowel",
            Class::Diphthong => "diphthong",
            Class::Stop => "stop",
            Class::Fricative => "fricative",
            Class::Aspirate => "aspirate",
            Class::Nasal => "nasal",
            Class::Liquid => "liquid",
            Class::Glide => "glide",
            Class::Affricate => "affricate",
        }
    }
}

/// One symbol of the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phoneme {
    /// How a lyric writes it.
    pub symbol: &'static str,
    /// The IPA it stands for (General American).
    pub ipa: &'static str,
    /// The ARPAbet symbol it corresponds to.
    pub arpabet: &'static str,
    pub class: Class,
    /// A word it is heard in, the sound in capitals.
    pub example: &'static str,
    /// The voice's phone.
    pub phone: Sym,
}

const fn ph(symbol: &'static str, ipa: &'static str, arpabet: &'static str, class: Class, example: &'static str, phone: Sym) -> Phoneme {
    Phoneme { symbol, ipa, arpabet, class, example, phone }
}

/// The version of the set (and of the lexicon and the phrasebook files written in it).
pub const VERSION: u32 = 1;
/// The set's name and version, as the ontology and the docs name it.
pub const PHONEME_SET: &str = "beatscript-v1";

/// The set, v1: vowels first, then the consonants by class. Closed: a lyric may use these and no others.
pub const SET: [Phoneme; 39] = [
    ph("aa", "ɑ", "AA", Class::Vowel, "fAther", Sym::AA),
    ph("ae", "æ", "AE", Class::Vowel, "cAt", Sym::AE),
    ph("uh", "ʌ, ə", "AH", Class::Vowel, "bUt, About", Sym::AH),
    ph("aw", "ɔ", "AO", Class::Vowel, "lAW", Sym::AO),
    ph("eh", "ɛ", "EH", Class::Vowel, "bEd", Sym::EH),
    ph("er", "ɝ", "ER", Class::Vowel, "bIRd", Sym::ER),
    ph("ii", "ɪ", "IH", Class::Vowel, "bIt", Sym::IH),
    ph("ee", "i", "IY", Class::Vowel, "sEE", Sym::IY),
    ph("uu", "ʊ", "UH", Class::Vowel, "bOOk", Sym::UH),
    ph("oo", "u", "UW", Class::Vowel, "fOOd", Sym::UW),
    ph("ay", "eɪ", "EY", Class::Diphthong, "dAY", Sym::EY),
    ph("ie", "aɪ", "AY", Class::Diphthong, "mY", Sym::AY),
    ph("oy", "ɔɪ", "OY", Class::Diphthong, "bOY", Sym::OY),
    ph("ow", "aʊ", "AW", Class::Diphthong, "nOW", Sym::AW),
    ph("oh", "oʊ", "OW", Class::Diphthong, "gO", Sym::OW),
    ph("p", "p", "P", Class::Stop, "Pop", Sym::P),
    ph("b", "b", "B", Class::Stop, "Bass", Sym::B),
    ph("t", "t", "T", Class::Stop, "Top", Sym::T),
    ph("d", "d", "D", Class::Stop, "Drop", Sym::D),
    ph("k", "k", "K", Class::Stop, "Kick", Sym::K),
    ph("g", "ɡ", "G", Class::Stop, "Go", Sym::G),
    ph("f", "f", "F", Class::Fricative, "Feel", Sym::F),
    ph("v", "v", "V", Class::Fricative, "Voice", Sym::V),
    ph("s", "s", "S", Class::Fricative, "Sun", Sym::S),
    ph("z", "z", "Z", Class::Fricative, "Zoo", Sym::Z),
    ph("sh", "ʃ", "SH", Class::Fricative, "SHine", Sym::SH),
    ph("zh", "ʒ", "ZH", Class::Fricative, "meaSure", Sym::ZH),
    ph("th", "θ", "TH", Class::Fricative, "THin", Sym::TH),
    ph("dh", "ð", "DH", Class::Fricative, "THis", Sym::DH),
    ph("h", "h", "HH", Class::Aspirate, "Hands", Sym::HH),
    ph("m", "m", "M", Class::Nasal, "Move", Sym::M),
    ph("n", "n", "N", Class::Nasal, "Night", Sym::N),
    ph("ng", "ŋ", "NG", Class::Nasal, "siNG", Sym::NG),
    ph("l", "l", "L", Class::Liquid, "Light", Sym::L),
    ph("r", "ɹ", "R", Class::Liquid, "Rave", Sym::R),
    ph("w", "w", "W", Class::Glide, "We", Sym::W),
    ph("y", "j", "Y", Class::Glide, "Yeah", Sym::Y),
    ph("ch", "tʃ", "CH", Class::Affricate, "CHant", Sym::CH),
    ph("j", "dʒ", "JH", Class::Affricate, "Jump", Sym::JH),
];

/// The symbol a rest is written with (in an escape as in plain lyrics).
pub const REST: &str = "_";

/// The set's entry for `symbol` (exact, lowercase).
pub fn phoneme(symbol: &str) -> Option<&'static Phoneme> {
    SET.iter().find(|p| p.symbol == symbol)
}

/// The set's symbol for a phone.
pub fn symbol_of(phone: Sym) -> &'static str {
    SET.iter().find(|p| p.phone == phone).map_or("?", |p| p.symbol)
}

/// Phones as the set writes them: `w ee aa r`.
pub fn spell(phones: &[Ph]) -> String {
    phones.iter().map(|p| symbol_of(p.sym)).collect::<Vec<_>>().join(" ")
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = alloc::vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// What a symbol that is not in the set probably meant: an ARPAbet or IPA symbol written for one of ours
/// (`AH0`, `ʃ`, `iy`), a capitalised one (`EE`), else the nearest symbol by spelling (one edit away).
pub fn did_you_mean(token: &str) -> Option<&'static str> {
    let bare = token.trim_end_matches(['0', '1', '2']);
    if let Some(p) = SET.iter().find(|p| p.arpabet.eq_ignore_ascii_case(bare) || p.ipa.split(", ").any(|i| i == bare)) {
        return Some(p.symbol);
    }
    let low = token.to_lowercase();
    if let Some(p) = phoneme(&low) {
        return Some(p.symbol);
    }
    let mut best: Option<(usize, &'static str)> = None;
    for p in SET.iter() {
        let d = edit_distance(&low, p.symbol);
        if d <= 1 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, p.symbol));
        }
    }
    best.map(|b| b.1)
}

// MARK: the escape

/// One sung phone of an escape, with its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EscPhone {
    pub sym: Sym,
    /// `:` on a vowel: held over one more note.
    pub hold: bool,
    /// `!` on a vowel: accented.
    pub accent: bool,
}

/// A word of an escape (the stretch between two `-`), or a rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EscWord {
    Phones(Vec<EscPhone>),
    Rest,
}

/// What is wrong in an escape: byte offsets into the text that was read (the lyric), the token, and what
/// it probably meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscIssue {
    pub start: usize,
    pub end: usize,
    pub token: String,
    pub message: String,
    pub did_you_mean: Option<&'static str>,
}

/// The escapes of a lyric: each `(start, end)` byte range of a `/…/` (slashes included).
///
/// A `/` outside an escape opens one when something other than a space comes right after it (`/h ay/`, `/-/`,
/// `//`); the escape runs to the next `/`. A `/` with a space (or the end of the lyric) right after it is a
/// **phrase break** and opens nothing, so `/h ii/ / /h ii/` is an escape, a break and an escape, and `/ /` is two
/// breaks. A `/` that opens an escape and finds no partner is a phrase break too, as it always was. Inside an
/// escape every `/` closes it, spaced or not. The same rule is `lyric_text` in `grammar.ebnf`
/// (`tests/grammar.rs` holds the two together).
pub fn escape_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c != '/' {
            continue;
        }
        match open {
            Some(s) => {
                out.push((s, i + 1));
                open = None;
            }
            None => {
                // a phrase break (a space or the end right after it), else the start of an escape
                if text[i + 1..].chars().next().is_some_and(|n| !n.is_whitespace()) {
                    open = Some(i);
                }
            }
        }
    }
    out
}

/// Read the inside of one escape (`inner`, which starts at byte `at` of the lyric). Unknown symbols and
/// misplaced modifiers are issues; what is valid is still read, so a check reports every problem at once.
pub fn read_escape(inner: &str, at: usize) -> (Vec<EscWord>, Vec<EscIssue>) {
    let mut words = Vec::new();
    let mut issues = Vec::new();
    let mut cur: Vec<EscPhone> = Vec::new();
    let flush = |cur: &mut Vec<EscPhone>, words: &mut Vec<EscWord>| {
        if !cur.is_empty() {
            words.push(EscWord::Phones(core::mem::take(cur)));
        }
    };
    // tokens: runs of non-space; a `-` alone or between symbols cuts a word
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let s = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // a token may still have `-` in it (`uh-p`): each piece is a symbol, each `-` a cut
        let tok = &inner[s..i];
        let mut k = 0usize;
        for (pi, piece) in tok.split('-').enumerate() {
            let ps = s + k;
            k += piece.len() + 1;
            if pi > 0 || piece.is_empty() {
                // a `-` cuts the word here
                flush(&mut cur, &mut words);
            }
            if piece.is_empty() {
                continue;
            }
            if piece == REST {
                flush(&mut cur, &mut words);
                words.push(EscWord::Rest);
                continue;
            }
            let base = piece.trim_end_matches([':', '!']);
            let mods = &piece[base.len()..];
            match phoneme(base) {
                Some(p) => {
                    let vowel = matches!(p.class, Class::Vowel | Class::Diphthong);
                    if !mods.is_empty() && !vowel {
                        issues.push(EscIssue {
                            start: at + ps,
                            end: at + ps + piece.len(),
                            token: piece.to_string(),
                            message: alloc::format!("`{mods}` goes on a vowel (`:` holds it, `!` accents it), not on the consonant `{base}`"),
                            did_you_mean: None,
                        });
                    }
                    let n = |c: char| mods.chars().filter(|x| *x == c).count();
                    if n(':') > 1 || n('!') > 1 {
                        issues.push(EscIssue {
                            start: at + ps,
                            end: at + ps + piece.len(),
                            token: piece.to_string(),
                            message: alloc::format!("`{piece}`: each modifier once (`{base}:`, `{base}!` or `{base}:!`)"),
                            did_you_mean: None,
                        });
                    }
                    cur.push(EscPhone { sym: p.phone, hold: vowel && n(':') > 0, accent: vowel && n('!') > 0 });
                }
                None => {
                    let dym = did_you_mean(base);
                    let mut message = alloc::format!("`{base}` is not a BeatScript phoneme");
                    if let Some(d) = dym {
                        message.push_str(&alloc::format!(": did you mean `{d}`?"));
                    }
                    issues.push(EscIssue { start: at + ps, end: at + ps + base.len(), token: base.to_string(), message, did_you_mean: dym });
                }
            }
        }
    }
    flush(&mut cur, &mut words);
    if words.is_empty() && issues.is_empty() {
        // `//` is the usual way here (two slashes with nothing between them): it may have been meant as a break
        let message = if inner.is_empty() {
            "an escape with nothing in it: write phonemes between the slashes (`/h ay/`), or for a phrase break a `/` with a space after it (`hello / world`)"
        } else {
            "an escape with nothing in it: write phonemes between the slashes (`/h ay/`)"
        };
        issues.push(EscIssue { start: at.saturating_sub(1), end: at + inner.len() + 1, token: alloc::format!("/{inner}/"), message: message.to_string(), did_you_mean: None });
    }
    (words, issues)
}

/// Every problem in every escape of `lyric` (offsets into `lyric`).
pub fn check_escapes(lyric: &str) -> Vec<EscIssue> {
    escape_ranges(lyric).into_iter().flat_map(|(s, e)| read_escape(&lyric[s + 1..e - 1], s + 1).1).collect()
}

/// Phones of an escaped word as the voice takes them: the first vowel stressed, or the accented ones.
pub fn esc_phones(w: &[EscPhone]) -> Vec<Ph> {
    let accented = w.iter().any(|p| p.accent);
    let mut first = true;
    w.iter()
        .map(|p| {
            let stress = if !p.sym.is_vowel() {
                0
            } else if accented {
                u8::from(p.accent)
            } else if first {
                first = false;
                1
            } else {
                0
            };
            Ph { sym: p.sym, stress }
        })
        .collect()
}

/// Phones written in the set (`d r aa p`), `-` ignored; None when a symbol is not in it.
pub fn parse_spelling(text: &str) -> Option<Vec<Ph>> {
    let mut out = Vec::new();
    let mut first = true;
    for t in text.split(|c: char| c.is_ascii_whitespace() || c == '-').filter(|t| !t.is_empty()) {
        let p = phoneme(t)?;
        let stress = if p.phone.is_vowel() && first {
            first = false;
            1
        } else {
            0
        };
        out.push(Ph { sym: p.phone, stress });
    }
    (!out.is_empty()).then_some(out)
}

// MARK: the lexicon

static LEXICON_TEXT: &str = include_str!("../../data/lexicon.txt");

struct Lexicon {
    entries: Vec<(String, Vec<Ph>)>,
}

static LEXICON: OnceBox<Lexicon> = OnceBox::new();

fn data_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("version "))
}

fn lexicon_table() -> &'static Lexicon {
    LEXICON.get_or_init(|| {
        let mut entries: Vec<(String, Vec<Ph>)> = data_lines(LEXICON_TEXT)
            .filter_map(|l| {
                let (w, rest) = l.split_once(|c: char| c.is_ascii_whitespace())?;
                Some((w.to_string(), parse_spelling(rest)?))
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Box::new(Lexicon { entries })
    })
}

/// The lexicon's version line (`version 1`).
pub fn lexicon_version() -> u32 {
    LEXICON_TEXT.lines().find_map(|l| l.trim().strip_prefix("version ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// How many words the lexicon has.
pub fn lexicon_size() -> usize {
    lexicon_table().entries.len()
}

/// The lexicon's phones for `word` (lowercase letters and apostrophes).
pub fn lexicon_lookup(word: &str) -> Option<Vec<Ph>> {
    let t = lexicon_table();
    t.entries.binary_search_by(|e| e.0.as_str().cmp(word)).ok().map(|i| t.entries[i].1.clone())
}

/// Every entry, as the file has them: (word, spelling in the set).
pub fn lexicon_entries() -> Vec<(String, String)> {
    lexicon_table().entries.iter().map(|(w, p)| (w.clone(), spell(p))).collect()
}

/// The raw lines of the lexicon file that do not read (a test keeps this empty).
pub fn lexicon_bad_lines() -> Vec<String> {
    data_lines(LEXICON_TEXT)
        .filter(|l| l.split_once(|c: char| c.is_ascii_whitespace()).and_then(|(_, r)| parse_spelling(r)).is_none())
        .map(str::to_string)
        .collect()
}

// MARK: the phrasebook

static PHRASEBOOK_TEXT: &str = include_str!("../../data/phrasebook.txt");

/// A stock phrase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrase {
    /// What a program writes after `@`.
    pub id: String,
    /// The words it says.
    pub words: String,
    /// Its phonemes in the set, a `-` between syllables.
    pub phonemes: String,
    /// A suggested rhythm: 16 steps a bar, `x` a syllable, `.` a rest, bars joined with spaces.
    pub rhythm: String,
}

impl Phrase {
    /// How many notes it takes (one a syllable).
    pub fn syllables(&self) -> usize {
        self.phonemes.split_ascii_whitespace().filter(|t| phoneme(t).is_some_and(|p| matches!(p.class, Class::Vowel | Class::Diphthong))).count()
    }

    /// The lyric it stands for: its phonemes as one escape, so no lexicon is asked.
    pub fn lyric(&self) -> String {
        alloc::format!("/{}/", self.phonemes)
    }
}

/// Every phrase, in the file's order.
pub fn phrases() -> Vec<Phrase> {
    data_lines(PHRASEBOOK_TEXT)
        .filter_map(|l| {
            let f: Vec<&str> = l.split(" | ").map(str::trim).collect();
            (f.len() == 4).then(|| Phrase { id: f[0].to_string(), words: f[1].to_string(), phonemes: f[2].to_string(), rhythm: f[3].to_string() })
        })
        .collect()
}

/// The phrase `id` names.
pub fn phrase(id: &str) -> Option<Phrase> {
    phrases().into_iter().find(|p| p.id == id)
}

/// The phrasebook's version line.
pub fn phrasebook_version() -> u32 {
    PHRASEBOOK_TEXT.lines().find_map(|l| l.trim().strip_prefix("version ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

// MARK: the public-domain phrasebook

static PD_PHRASEBOOK_TEXT: &str = include_str!("../../data/phrasebook-pd.txt");

/// A family of lines in the public-domain phrasebook: the closed vocabulary its `family` column is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Family {
    /// What the file writes (`iambic`, `common-meter`).
    pub id: &'static str,
    /// What a person calls it.
    pub name: &'static str,
    /// What a line of it is, in a sentence.
    pub about: &'static str,
}

/// The families of the public-domain phrasebook, in the order the docs and the page list them. Closed: a line is one of these.
pub const PD_FAMILIES: [Family; 10] = [
    Family { id: "iambic", name: "iambic pentameter", about: "Lines of ten syllables (eleven with a feminine ending) in pairs, unstressed then stressed: the beat of blank verse and the sonnet." },
    Family { id: "trochaic", name: "trochaic", about: "A stressed syllable then an unstressed one, four to a line, the last often cut short: Blake's Tyger and Hiawatha." },
    Family { id: "anapestic", name: "anapestic", about: "Two unstressed syllables then a stressed one, so it rolls in threes: Byron's Sennacherib and Lear's nonsense." },
    Family { id: "dactylic", name: "dactylic", about: "A stressed syllable then two unstressed ones, also in threes: the long line of Evangeline." },
    Family { id: "common-meter", name: "common meter", about: "The hymn and ballad stanza: a line of four beats, then a line of three (eight syllables, then six)." },
    Family { id: "sprung", name: "sprung rhythm", about: "Gerard Manley Hopkins's rhythm: count the stresses, not the syllables; a stress stands alone or takes up to three slack syllables." },
    Family { id: "nursery", name: "nursery rhyme", about: "Two or three strong beats a line with any number of weak syllables between: a rhyme said aloud." },
    Family { id: "parallelism", name: "parallelism", about: "The verse of the King James Bible: two halves that answer each other (day and night, speech and knowledge), with no fixed foot." },
    Family { id: "oratory", name: "oratory", about: "Speech: no fixed foot, the stresses of the sentence, built in threes and answering clauses." },
    Family { id: "call-and-response", name: "call and response", about: "A work song: the leader sings a line and the crew answers; the stresses are the heaves, the pulls on the rope." },
];

/// How a line sits on the grid, with what it means: `straight` is the sixteenth grid, `triplet` groups of three.
pub const PD_FEELS: [(&str, &str); 2] = [("straight", "the syllables sit on the sixteenth grid"), ("triplet", "the syllables go in groups of three")];

/// A line of the public-domain phrasebook (`data/phrasebook-pd.txt`): its words, how BeatScript sings them, where it came
/// from, and its meter as a step pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdPhrase {
    /// What a program writes after `@` (`sing @tempest_such_stuff on melody`).
    pub id: String,
    /// One of [`PD_FAMILIES`]' ids.
    pub family: String,
    /// The words as printed, `/` a line end or a pause.
    pub text: String,
    /// The same words as BeatScript sings them: lexicon words as they are, every other word as a phoneme escape,
    /// ` / ` a phrase break (a slash with a space after it). `sing "<lyric>" on melody` checks with no warning.
    pub lyric: String,
    /// The work it is from, and where in it.
    pub work: String,
    pub author: String,
    /// The year of first publication, before 1929: the work's, or for a shanty the collection's.
    pub year: u16,
    /// The pre-1929 edition the text was read from, and where the file's reader can find it.
    pub edition: String,
    pub source: String,
    /// The meter as a person names it (`iambic pentameter`).
    pub meter: String,
    /// `straight` or `triplet` (one of [`PD_FEELS`]).
    pub feel: String,
    /// The meter as a step pattern, one step a syllable: `x` stressed, `~` not (`~ x ~ x` is two iambs).
    pub stress: String,
    /// How many notes it takes as sung: the steps of `stress`, the syllables of `lyric`.
    pub syllables: usize,
    /// For call and response, how many of the first syllables are the leader's call (the rest is the answer); 0 otherwise.
    pub call: usize,
    /// One sentence on how it scans, or on the date or the text where there is a caveat.
    pub note: String,
}

impl PdPhrase {
    /// The stress pattern as booleans: true where a syllable is stressed.
    pub fn accents(&self) -> Vec<bool> {
        self.stress.split_ascii_whitespace().map(|s| s == "x").collect()
    }
}

fn pd_parse(l: &str) -> Option<PdPhrase> {
    let f: Vec<&str> = l.split(" | ").map(str::trim).collect();
    if f.len() != 15 {
        return None;
    }
    Some(PdPhrase {
        id: f[0].to_string(),
        family: f[1].to_string(),
        text: f[2].to_string(),
        lyric: f[3].to_string(),
        work: f[4].to_string(),
        author: f[5].to_string(),
        year: f[6].parse().ok()?,
        edition: f[7].to_string(),
        source: f[8].to_string(),
        meter: f[9].to_string(),
        feel: f[10].to_string(),
        stress: f[11].to_string(),
        syllables: f[12].parse().ok()?,
        call: f[13].parse().ok()?,
        note: f[14].to_string(),
    })
}

static PD_TABLE: OnceBox<Vec<PdPhrase>> = OnceBox::new();

/// Every line of the public-domain phrasebook, in the file's order.
pub fn pd_phrases() -> &'static [PdPhrase] {
    PD_TABLE.get_or_init(|| Box::new(data_lines(PD_PHRASEBOOK_TEXT).filter_map(pd_parse).collect()))
}

/// The line `id` names.
pub fn pd_phrase(id: &str) -> Option<&'static PdPhrase> {
    pd_phrases().iter().find(|p| p.id == id)
}

/// The file's text exactly: LF line ends, UTF-8, one newline at the end. It is the corpus's canonical text, so what
/// is hashed for its CID, and what `/phrasebook.txt` serves.
pub fn pd_phrasebook_text() -> &'static str {
    PD_PHRASEBOOK_TEXT
}

/// The public-domain phrasebook's version line.
pub fn pd_phrasebook_version() -> u32 {
    PD_PHRASEBOOK_TEXT.lines().find_map(|l| l.trim().strip_prefix("version ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// The raw lines of the public-domain file that do not read (a test keeps this empty).
pub fn pd_bad_lines() -> Vec<String> {
    data_lines(PD_PHRASEBOOK_TEXT).filter(|l| pd_parse(l).is_none()).map(str::to_string).collect()
}

/// What `sing @id` sings: a stock phrase's phonemes as one escape, or a public-domain line's own lyric.
pub fn phrase_lyric(id: &str) -> Option<String> {
    phrase(id).map(|p| p.lyric()).or_else(|| pd_phrase(id).map(|p| p.lyric.clone()))
}

/// Every id `sing @id` takes: the stock phrases', then the public-domain lines'.
pub fn phrase_ids() -> Vec<String> {
    phrases().into_iter().map(|p| p.id).chain(pd_phrases().iter().map(|p| p.id.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_closed_and_maps_one_to_one_onto_the_voice() {
        assert_eq!(SET.len(), 39);
        for (i, a) in SET.iter().enumerate() {
            for b in &SET[i + 1..] {
                assert_ne!(a.symbol, b.symbol);
                assert_ne!(a.phone, b.phone, "{} and {} are one phone", a.symbol, b.symbol);
                assert_ne!(a.arpabet, b.arpabet);
            }
            assert_eq!(a.phone.name(), a.arpabet, "{}", a.symbol);
            assert!(a.symbol.chars().all(|c| c.is_ascii_lowercase()), "{}", a.symbol);
            assert_eq!(a.phone.is_vowel(), matches!(a.class, Class::Vowel | Class::Diphthong), "{}", a.symbol);
            // every vowel has formant targets; every consonant has a source in the voice
            if a.phone.is_vowel() {
                assert!(a.phone.vowel().is_some());
                assert_eq!(a.phone.vowel().unwrap().1.is_some(), a.class == Class::Diphthong, "{}", a.symbol);
            } else {
                assert!(a.phone.onset().is_some(), "{}", a.symbol);
            }
        }
    }

    #[test]
    fn an_escape_reads_symbols_words_rests_and_modifiers() {
        let (w, issues) = read_escape("w ee - aa: r - dh uh - r ay! v _", 0);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(w.len(), 5);
        assert_eq!(w[4], EscWord::Rest);
        let EscWord::Phones(are) = &w[1] else { panic!() };
        assert_eq!((are[0].sym, are[0].hold, are[0].accent), (Sym::AA, true, false));
        let EscWord::Phones(rave) = &w[3] else { panic!() };
        assert!(rave[1].accent && !rave[1].hold);
        // a hyphen inside a token cuts too
        assert_eq!(read_escape("uh-p", 0).0.len(), 2);
    }

    #[test]
    fn an_unknown_symbol_is_an_issue_at_its_place_with_what_it_meant() {
        let lyric = "we are /dh AH0 xq ʃ/ rave";
        let issues = check_escapes(lyric);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(&lyric[issues[0].start..issues[0].end], "AH0");
        assert_eq!(issues[0].did_you_mean, Some("uh"));
        assert_eq!(&lyric[issues[1].start..issues[1].end], "xq");
        assert_eq!(issues[1].did_you_mean, None);
        assert_eq!(issues[2].did_you_mean, Some("sh"));
        assert!(issues[0].message.contains("did you mean `uh`"));
        // near misses by spelling
        assert_eq!(did_you_mean("shh"), Some("sh"));
        assert!(did_you_mean("ei").is_some());
        assert_eq!(did_you_mean("EE"), Some("ee"));
        assert_eq!(did_you_mean("iy"), Some("ee"));
        // a modifier on a consonant, and an empty escape
        assert_eq!(check_escapes("/s: aa/").len(), 1);
        assert_eq!(check_escapes("a // b").len(), 1);
        // a slash with no partner is not an escape
        assert!(escape_ranges("left / right").is_empty());
    }

    /// The escapes of `t`, as the text they cover.
    fn escapes(t: &str) -> Vec<&str> {
        escape_ranges(t).into_iter().map(|(s, e)| &t[s..e]).collect()
    }

    #[test]
    fn a_slash_with_a_space_after_it_is_a_phrase_break_and_a_phoneme_after_it_opens_an_escape() {
        // 6634: the break and the next escape's opening slash were read as an escape with nothing in it
        assert_eq!(escapes("/h ii/ ! / /h ii/ !"), ["/h ii/", "/h ii/"]);
        assert!(check_escapes("/h ii/ ! / /h ii/ !").is_empty());
        // a break before an escape, between two, after one, and the lyric's own start and end
        assert_eq!(escapes("/ /h ii/"), ["/h ii/"]);
        assert_eq!(escapes("/h ii/ / /h aa/"), ["/h ii/", "/h aa/"]);
        assert_eq!(escapes("/h ii/ /"), ["/h ii/"]);
        assert_eq!(escapes("/ /h ii/ /"), ["/h ii/"]);
        // two breaks in a row, and breaks round plain words (`/ world /` was an escape, and a wrong one)
        assert!(escapes("/ /").is_empty());
        assert!(escapes("/ / /").is_empty());
        assert!(escapes("/").is_empty());
        assert!(escapes("hello / world / again").is_empty());
        assert!(escapes("hello /\nworld /\n").is_empty());
        // a break after the escape's own punctuation, or glued to it
        assert_eq!(escapes("/h ii/!/"), ["/h ii/"]);
        assert_eq!(escapes("/h ii/!/ /h aa/"), ["/h ii/", "/h aa/"]);
        assert_eq!(escapes("/h ii/ !/ /h aa/"), ["/h ii/", "/h aa/"]);
        // two escapes touching, and an escape closed with a space before its slash (the next `/` closes it)
        assert_eq!(escapes("/h ii//h aa/"), ["/h ii/", "/h aa/"]);
        assert_eq!(escapes("/h ii / /h aa/"), ["/h ii /", "/h aa/"]);
        // an opening slash with no partner is a break, as it always was
        assert!(escapes("/h ii").is_empty());
        assert!(escapes("so /h ii/ and /h aa").len() == 1);
        // `//` and `/-/` have nothing in them: the one error left, now with the way out
        assert_eq!(escapes("a // b"), ["//"]);
        let issues = check_escapes("a // b");
        assert_eq!((issues.len(), issues[0].start, issues[0].end), (1, 2, 4));
        assert_eq!(issues[0].token, "//");
        assert!(issues[0].message.starts_with("an escape with nothing in it"), "{}", issues[0].message);
        assert!(issues[0].message.contains("a `/` with a space after it"), "{}", issues[0].message);
        assert_eq!(check_escapes("/-/").len(), 1);
        // a break is the same break at any width of space
        assert_eq!(escapes("/h ii/\t/\u{a0}/h aa/"), ["/h ii/", "/h aa/"]);
    }

    /// The pairing of slashes before 6634: the first slash opens, the next closes, whatever is between them.
    fn first_come_pairing(t: &str) -> Vec<(usize, usize)> {
        let (mut out, mut open) = (Vec::new(), None);
        for (i, _) in t.char_indices().filter(|c| c.1 == '/') {
            match open.take() {
                None => open = Some(i),
                Some(s) => out.push((s, i + 1)),
            }
        }
        out
    }

    #[test]
    fn a_lyric_whose_escapes_open_on_a_phoneme_reads_as_it_always_did() {
        // every string of up to seven of these: where each slash that opens a pair under the old pairing has something
        // that is not a space right after it, the two readings are the same (nothing written so changes). The strings
        // that are left are the ones with an old "escape" that began with a space, the only readings that moved.
        let atoms = ["/", " ", "h", "ii", "-", "_", "!"];
        let mut seen = (0usize, 0usize);
        let mut idx = [0usize; 7];
        for len in 0..=7usize {
            idx.iter_mut().for_each(|i| *i = 0);
            loop {
                let t: String = idx[..len].iter().map(|i| atoms[*i]).collect();
                let (old, new) = (first_come_pairing(&t), escape_ranges(&t));
                let spaced_open = old.iter().any(|(s, _)| t[s + 1..].chars().next().is_none_or(char::is_whitespace));
                if spaced_open {
                    seen.1 += 1;
                } else {
                    seen.0 += 1;
                    assert_eq!(old, new, "{t:?}");
                }
                // whatever it reads: ranges in order, each from a slash that has a non-space after it to the next slash
                let mut at = 0;
                for (s, e) in &new {
                    assert!(*s >= at && t[*s..].starts_with('/') && t[..*e].ends_with('/') && e - s >= 2, "{t:?}");
                    assert!(t[s + 1..].chars().next().is_some_and(|n| !n.is_whitespace()), "{t:?}");
                    assert!(!t[s + 1..e - 1].contains('/'), "{t:?}");
                    at = *e;
                }
                // advance the odometer
                let mut k = len;
                while k > 0 && idx[k - 1] == atoms.len() - 1 {
                    idx[k - 1] = 0;
                    k -= 1;
                }
                if k == 0 {
                    break;
                }
                idx[k - 1] += 1;
            }
        }
        assert!(seen.0 > 100_000 && seen.1 > 10_000, "{seen:?}");
    }

    #[test]
    fn the_lexicon_is_written_in_the_set_sorted_and_versioned() {
        assert_eq!(lexicon_version(), VERSION);
        assert!(lexicon_bad_lines().is_empty(), "{:?}", lexicon_bad_lines());
        assert!(lexicon_size() >= 200, "{}", lexicon_size());
        let words: Vec<String> = data_lines(LEXICON_TEXT).map(|l| l.split_ascii_whitespace().next().unwrap().to_string()).collect();
        let mut sorted = words.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(words, sorted, "the lexicon is sorted, one entry a word");
        for w in ["bass", "rave", "drop", "beat", "music", "tonight", "feel", "hands", "body", "dance", "love", "night", "light", "higher", "the"] {
            assert!(lexicon_lookup(w).is_some(), "{w}");
        }
        assert_eq!(spell(&lexicon_lookup("bass").unwrap()), "b ay s");
    }

    /// 6627: words and vocables a singer reaches for first were sounded out by the spelling rules, and the rules got `hello`
    /// (`h eh l aa`, no `oh` at the end) and `ah` (`ae h`: two sounds, neither an open vowel) wrong. The lexicon says them.
    #[test]
    fn the_lyric_words_and_vocables_6627_names_are_in_the_lexicon() {
        for (w, want) in [
            ("hello", "h eh l oh"),
            ("ah", "aa"),
            ("sing", "s ii ng"),
            ("street", "s t r ee t"),
            ("la", "l aa"),
            ("na", "n aa"),
            ("wow", "w ow"),
            ("sky", "s k ie"),
            ("song", "s aw ng"),
            ("soul", "s oh l"),
            ("cry", "k r ie"),
            ("tomorrow", "t uh m aa r oh"),
            ("oh", "oh"),
            ("hey", "h ay"),
            ("the", "dh uh"),
            ("a", "uh"),
        ] {
            assert_eq!(spell(&lexicon_lookup(w).unwrap_or_else(|| panic!("{w} is not in the lexicon"))), want, "{w}");
        }
        // every entry is a word with a vowel in it (a lexicon word is never a rest or a bare consonant)
        for (w, p) in lexicon_entries() {
            assert!(p.split(' ').any(|s| phoneme(s).is_some_and(|x| x.phone.is_vowel())), "{w}: {p}");
        }
    }

    #[test]
    fn the_docs_table_is_the_set() {
        // docs/btw/phonemes.md leads with the table: one row a symbol, its IPA and ARPAbet as SET has them, and every phrase
        let doc = include_str!("../../../../docs/btw/phonemes.md");
        for p in SET.iter() {
            let row = alloc::format!("| `{}` | {} | {} | {} |", p.symbol, p.example, p.ipa, p.arpabet);
            assert!(doc.contains(&row), "docs/btw/phonemes.md lacks the row {row}");
        }
        assert!(doc.contains("| `_` |"));
        for ph in phrases() {
            assert!(doc.contains(&alloc::format!("| `@{}` |", ph.id)), "the phrase @{} is not in the docs", ph.id);
        }
        assert!(doc.contains(PHONEME_SET));
    }

    #[test]
    fn every_phrase_is_in_the_set_and_its_rhythm_has_a_step_a_syllable() {
        assert_eq!(phrasebook_version(), VERSION);
        let ps = phrases();
        assert!(ps.len() >= 20, "{}", ps.len());
        for p in &ps {
            assert!(p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{}", p.id);
            let (_, issues) = read_escape(&p.phonemes, 0);
            assert!(issues.is_empty(), "{}: {issues:?}", p.id);
            for syl in p.phonemes.split(" - ") {
                let vowels = syl.split_ascii_whitespace().filter(|t| phoneme(t).is_some_and(|x| x.phone.is_vowel())).count();
                assert_eq!(vowels, 1, "{}: a syllable is one vowel: {syl:?}", p.id);
            }
            assert_eq!(p.rhythm.matches('x').count(), p.syllables(), "{}", p.id);
            assert!(p.rhythm.split(' ').all(|bar| bar.len() == 16 && bar.chars().all(|c| c == 'x' || c == '.')), "{}", p.id);
        }
        assert!(phrase("hands_up").is_some() && phrase("nope").is_none());
    }

    // ---- the public-domain phrasebook

    /// The words of a printed text: letters and apostrophes, a hyphen or any other mark between words (`to-morrow` is one).
    fn printed_words(text: &str) -> usize {
        let t = text.to_lowercase().replace("to-morrow", "tomorrow");
        t.split(|c: char| !(c.is_alphabetic() || c == '\'')).filter(|w| !w.is_empty()).count()
    }

    #[test]
    fn the_public_domain_file_is_versioned_cc0_and_canonical_text() {
        assert_eq!(pd_phrasebook_version(), VERSION);
        assert!(pd_bad_lines().is_empty(), "{:?}", pd_bad_lines());
        let t = pd_phrasebook_text();
        // the header says what the file is and why every text may be in it
        let head: String = t.lines().take_while(|l| l.starts_with('#')).collect::<Vec<_>>().join("\n");
        assert!(head.contains("CC0 1.0 Universal") && head.contains("public domain") && head.contains("before 1929"), "{head}");
        // canonical text: it is what the CID hashes, so LF only, one newline at the end, nothing odd in it
        assert!(!t.contains('\r') && t.ends_with('\n') && !t.ends_with("\n\n"), "the file is LF text with one newline at the end");
        assert!(!t.contains('\t') && !t.contains('\u{feff}'));
        assert_eq!(data_lines(t).count(), pd_phrases().len(), "every data line reads");
        assert!(pd_phrases().len() >= 40, "{}", pd_phrases().len());
    }

    #[test]
    fn every_line_is_curated_dated_before_1929_and_in_the_closed_vocabularies() {
        let ps = pd_phrases();
        let own: Vec<String> = phrases().into_iter().map(|p| p.id).collect();
        let mut seen: Vec<&str> = Vec::new();
        for p in ps {
            assert!(!p.id.is_empty() && p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !p.id.starts_with(|c: char| c.is_ascii_digit()), "{}", p.id);
            assert!(!seen.contains(&p.id.as_str()), "{} twice", p.id);
            seen.push(&p.id);
            assert!(!own.contains(&p.id), "{} is also a stock phrase", p.id);
            assert!((1500..1929).contains(&p.year), "{}: first published {}: only before 1929 may be here", p.id, p.year);
            assert!(PD_FAMILIES.iter().any(|f| f.id == p.family), "{}: family {:?}", p.id, p.family);
            assert!(PD_FEELS.iter().any(|f| f.0 == p.feel), "{}: feel {:?}", p.id, p.feel);
            for (what, v) in [("text", &p.text), ("lyric", &p.lyric), ("work", &p.work), ("author", &p.author), ("edition", &p.edition), ("meter", &p.meter), ("note", &p.note)] {
                assert!(!v.trim().is_empty(), "{}: no {what}", p.id);
            }
            assert!(p.source.starts_with("https://archive.org/") || p.source.starts_with("https://en.wikisource.org/"), "{}: source {:?}", p.id, p.source);
            // the edition is the pre-1929 one the text was read from: it names a year, and that year is before 1929
            let edition_year = p.edition.split(|c: char| !c.is_ascii_digit()).filter(|d| d.len() == 4).filter_map(|d| d.parse::<u16>().ok()).filter(|y| *y > 1500).min();
            assert!(edition_year.is_some_and(|y| y < 1929), "{}: the edition names no year before 1929: {:?}", p.id, p.edition);
            // the stress is a step pattern: `x` and `~`, one step a syllable
            assert!(p.stress.split(' ').all(|s| s == "x" || s == "~"), "{}: {:?}", p.id, p.stress);
            assert_eq!(p.stress.split(' ').count(), p.syllables, "{}: syllables is the number of steps", p.id);
            assert!(p.accents().iter().any(|a| *a), "{}: no stress at all", p.id);
            // call and response: the call is part of the line, and only a work song has one
            assert_eq!(p.call > 0, p.family == "call-and-response", "{}", p.id);
            assert!(p.call < p.syllables, "{}", p.id);
            // a line is a short phrase: at most 24 syllables, none long
            assert!((8..=24).contains(&p.syllables), "{}: {} syllables", p.id, p.syllables);
        }
        // about three to five lines of each family, and the seeds the record names
        for f in PD_FAMILIES {
            let n = ps.iter().filter(|p| p.family == f.id).count();
            assert!((3..=5).contains(&n), "{} has {n} lines", f.id);
        }
        for id in ["tempest_such_stuff", "psalm42_deep_calleth", "tyger_burning_bright", "byron_assyrian_wolf", "lear_derry_down_derry", "hiawatha_gitche_gumee", "evangeline_forest_primeval", "gettysburg_four_score"] {
            assert!(pd_phrase(id).is_some(), "the seed {id} is missing");
        }
        assert!(ps.iter().filter(|p| p.id.starts_with("shanty_")).count() >= 3 && ps.iter().any(|p| p.id.starts_with("dickinson_")));
        assert_eq!(pd_phrase("tempest_such_stuff").map(|p| (p.year, p.syllables)), Some((1623, 20)));
        assert!(pd_phrase("nope").is_none());
    }

    #[test]
    fn every_lyric_is_words_in_the_lexicon_or_phoneme_escapes_and_nothing_the_rules_guessed() {
        use crate::vocal::lexicon::words;
        for p in pd_phrases() {
            assert!(check_escapes(&p.lyric).is_empty(), "{}: {:?}", p.id, check_escapes(&p.lyric));
            assert!(p.lyric.chars().all(|c| c.is_ascii_lowercase() || c == '\'' || " ,/-:!_".contains(c)), "{}: {:?}", p.id, p.lyric);
            let ws = words(&p.lyric);
            // a plain word is in the BeatScript lexicon itself; an escaped one is written out
            for w in &ws {
                if w.escaped.is_none() {
                    assert!(lexicon_lookup(&w.text).is_some(), "{}: `{}` is neither in the lexicon nor escaped", p.id, w.text);
                }
            }
            assert!(crate::vocal::fit::ruled_words(&p.lyric).is_empty(), "{}: {:?}", p.id, crate::vocal::fit::ruled_words(&p.lyric));
            // the lyric is the text's words, none dropped and none added (one word each)
            assert_eq!(ws.len(), printed_words(&p.text), "{}: the lyric has {} words, the text {}", p.id, ws.len(), printed_words(&p.text));
            // the phrase break is ` / ` (6634: a slash with a space after it opens nothing, so it can sit beside an escape):
            // no comma, and every slash outside an escape is a break with a space (or the end) after it
            assert!(!p.lyric.contains(','), "{}: a comma in {:?}: the phrase break is ` / `", p.id, p.lyric);
            let mut outside = p.lyric.clone();
            for (s, e) in escape_ranges(&p.lyric).into_iter().rev() {
                outside.replace_range(s..e, " ");
            }
            for (i, _) in outside.match_indices('/') {
                assert!(outside[i + 1..].chars().next().map_or(true, char::is_whitespace), "{}: a slash outside an escape with no space after it in {:?}", p.id, p.lyric);
            }
        }
    }

    #[test]
    fn syllables_are_the_stress_steps_and_the_notes_the_lyric_takes_on_the_voice() {
        for p in pd_phrases() {
            let slots = crate::vocal::fit::syllables(&p.lyric);
            assert_eq!(crate::vocal::fit::notes_needed(&slots), p.syllables, "{}: the voice sings {} notes of `{}`", p.id, crate::vocal::fit::notes_needed(&slots), p.lyric);
            assert!(slots.iter().all(|s| !s.rest && !s.hold), "{}: no rest and no held syllable in a line", p.id);
        }
    }

    #[test]
    fn the_public_domain_lines_are_a_phrasebook_too() {
        let ids = phrase_ids();
        assert_eq!(ids.len(), phrases().len() + pd_phrases().len());
        assert_eq!(ids[0], "hands_up");
        assert!(ids.contains(&"tempest_such_stuff".to_string()));
        // a stock phrase sings its phonemes as one escape, a public-domain line its lyric
        assert_eq!(phrase_lyric("hands_up").unwrap(), phrase("hands_up").unwrap().lyric());
        assert_eq!(phrase_lyric("tempest_such_stuff").as_deref(), Some(pd_phrase("tempest_such_stuff").unwrap().lyric.as_str()));
        assert!(phrase_lyric("nope").is_none());
    }

    #[test]
    fn the_docs_list_every_public_domain_line_as_the_file_has_it() {
        // docs/btw/phonemes.md, "The public-domain phrasebook": one row a line, the same words, year, meter, stress and syllables
        let doc = include_str!("../../../../docs/btw/phonemes.md");
        for p in pd_phrases() {
            assert!(doc.contains(&pd_doc_row(p)), "docs/btw/phonemes.md lacks the row {}", pd_doc_row(p));
        }
        for f in PD_FAMILIES {
            assert!(doc.contains(&alloc::format!("| `{}` | {} |", f.id, f.about)), "the docs lack the family {}", f.id);
        }
        for (feel, what) in PD_FEELS {
            assert!(doc.contains(&alloc::format!("| `{feel}` | {what} |")), "the docs lack the feel {feel}");
        }
    }

    /// The row of the docs' table for a line (the page `/phrasebook` is built from the same fields).
    fn pd_doc_row(p: &PdPhrase) -> String {
        alloc::format!("| `@{}` | {} | {}, {} | {} | {} | `{}` | {} |", p.id, p.text, p.author, p.work, p.year, p.meter, p.stress, p.syllables)
    }
}
