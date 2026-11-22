//! Musical keys for mixing (docs/KEY.md): what key a track is in, how it is written on the Camelot
//! wheel, whether two decks' keys mix, and how far to transpose one so they do.
//!
//! - **Generated tracks** carry their key: a project's `rootNote` and `scaleName`, a loop set's
//!   "A minor". [`Key::from_project`] and [`Key::parse`] read them; no audio is analysed.
//! - **External audio** is estimated from its chroma ([`detect`]): djrust's 12-bin chroma
//!   (`musicality::loop_key::chroma`, FFT magnitudes 60 Hz – 5 kHz folded onto pitch classes)
//!   correlated against the 24 rotations of published major / minor key profiles. Three are
//!   here, and [`Profile::Ensemble`] (the default) averages their correlations per key:
//!   - Krumhansl & Kessler 1982, probe-tone ratings (Psychological Review 89, table 1), the
//!     profile djrust's loop-root estimate already uses;
//!   - Temperley 2007, the Kostka–Payne corpus profile (D. Temperley, *Music and Probability*,
//!     MIT Press 2007; the numbers as music21 publishes them, `TemperleyKostkaPayne`);
//!   - Albrecht & Shanahan 2013, the corpus profile of "The Use of Large Corpora to Train a New
//!     Type of Key-Finding Algorithm" (Music Perception 31(1)).
//!
//!   Each profile errs differently (KK leans to the relative major / minor, Temperley to the
//!   dominant), so the average is the safer guess on mixed material; the per-profile answers
//!   are kept in [`Detection::votes`] so a caller can see when they disagree.
//! - **Camelot**: the wheel DJs write keys on, 1A–12A minor and 1B–12B major. A step round the
//!   wheel is a fifth; A ↔ B at one number is the relative major / minor. [`compat`] grades a
//!   pair of keys by that geometry, and [`suggest_transpose`] finds the smallest shift (±6
//!   semitones) that makes a second key mix with the first.
//!
//! Pure: no I/O, `no_std` + `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};
use crate::musicality::loop_key;

/// Pitch class names, as the generator writes them (flats for Eb, Ab, Bb).
pub const NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/// A key: a tonic pitch class and a mode, major or minor. Church modes and pentatonics fold onto
/// the one their third makes (dorian, phrygian, blues → minor; mixolydian, lydian → major) with
/// the same tonic: that is what a DJ hears and what the wheel can say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    /// 0 = C.
    pub root: u8,
    pub minor: bool,
}

/// Is a generator / free-text scale or mode name a minor one? None when it names no mode.
fn mode_is_minor(word: &str) -> Option<bool> {
    let w = word.trim().to_ascii_lowercase();
    let w = w.strip_suffix("scale").unwrap_or(&w).trim().to_string();
    let w = w.replace([' ', '_', '-'], "");
    Some(match w.as_str() {
        "m" | "min" | "minor" | "aeolian" | "dorian" | "phrygian" | "locrian" | "harmonicmin"
        | "harmonicminor" | "melodicminor" | "minpentatonic" | "minorpentatonic" | "blues" => true,
        "" | "maj" | "major" | "ionian" | "mixolydian" | "lydian" | "pentatonic" | "majorpentatonic" => false,
        _ => return None,
    })
}

impl Key {
    pub const fn new(root: u8, minor: bool) -> Key {
        Key { root: root % 12, minor }
    }

    /// "A minor", "C major".
    pub fn name(&self) -> String {
        format!("{} {}", NAMES[self.root as usize], if self.minor { "minor" } else { "major" })
    }

    /// "Am", "C", "F#m".
    pub fn short(&self) -> String {
        format!("{}{}", NAMES[self.root as usize], if self.minor { "m" } else { "" })
    }

    /// The Camelot number (1-12) and letter: minors are A, majors B. C major is 8B, A minor 8A;
    /// each step up the wheel is a fifth up.
    pub fn camelot_parts(&self) -> (u8, char) {
        // 8B is C major; a fifth (7 semitones) is one step: n = 8 + 7·root mod 12 (as 1-12)
        let base = if self.minor { (self.root as i32 + 3) % 12 } else { self.root as i32 }; // relative major
        let n = ((8 - 1 + 7 * base) % 12 + 12) % 12 + 1;
        (n as u8, if self.minor { 'A' } else { 'B' })
    }

    /// "8A".
    pub fn camelot(&self) -> String {
        let (n, l) = self.camelot_parts();
        format!("{n}{l}")
    }

    /// The key a Camelot code names ("8A", "12b"). None for anything else.
    pub fn from_camelot(code: &str) -> Option<Key> {
        let c = code.trim();
        let letter = c.chars().last()?.to_ascii_uppercase();
        let minor = match letter {
            'A' => true,
            'B' => false,
            _ => return None,
        };
        let n: i32 = c[..c.len() - 1].parse().ok()?;
        if !(1..=12).contains(&n) {
            return None;
        }
        // invert n = 8 + 7·base: base = 7·(n - 8) (7 is its own inverse mod 12)
        let base = ((7 * (n - 8)) % 12 + 12) % 12;
        let root = if minor { (base + 9) % 12 } else { base };
        Some(Key::new(root as u8, minor))
    }

    /// A key as people and the generator write it: "A minor", "Am", "F#m", "Bb major", "C",
    /// "D dorian", "C minor pentatonic", "Eb_min", "8A" (Camelot). None when it is not one.
    pub fn parse(text: &str) -> Option<Key> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        if t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Key::from_camelot(t);
        }
        // the tonic: a letter and an optional accidental
        let mut chars = t.char_indices();
        let (_, letter) = chars.next()?;
        if !('A'..='G').contains(&letter.to_ascii_uppercase()) {
            return None;
        }
        let mut cut = letter.len_utf8();
        let mut tonic = String::new();
        tonic.push(letter.to_ascii_uppercase());
        if let Some((i, c)) = chars.next() {
            if matches!(c, '#' | '♯' | 'b' | '♭') {
                // "b" is a flat only when what follows is not a word starting with it ("bm"? no:
                // "Bbm" is B flat minor, "Bm" is B minor: a 'b' right after the letter is a flat)
                tonic.push(if matches!(c, '#' | '♯') { '#' } else { 'b' });
                cut = i + c.len_utf8();
            }
        }
        let root = loop_key::pitch_class(&tonic)?;
        let rest = t[cut..].trim_start_matches([' ', '_', '-']);
        // the mode: the rest of the text, or its first word ("minor pentatonic")
        let minor = match mode_is_minor(rest) {
            Some(m) => m,
            None => {
                let first = rest.split([' ', '_', '-']).next().unwrap_or("");
                mode_is_minor(first)?
            }
        };
        Some(Key::new(root, minor))
    }

    /// A generated project's key: its `rootNote` (MIDI) and `scaleName` ("MinorScale",
    /// "DorianScale"). None when it has no root.
    pub fn from_root_scale(root_note: i64, scale: &str) -> Key {
        Key::new(root_note.rem_euclid(12) as u8, mode_is_minor(scale).unwrap_or(false))
    }

    /// The key a generated project was composed in. None when it has no `rootNote`.
    pub fn from_project(p: &crate::pflow::Project) -> Option<Key> {
        p.root_note.map(|r| Key::from_root_scale(r, &p.scale_name))
    }

    /// This key moved by `semitones`.
    pub fn transposed(&self, semitones: i32) -> Key {
        Key::new(((self.root as i32 + semitones).rem_euclid(12)) as u8, self.minor)
    }

    /// The relative major / minor (same notes, other tonic): A minor ↔ C major.
    pub fn relative(&self) -> Key {
        if self.minor {
            Key::new((self.root + 3) % 12, false)
        } else {
            Key::new((self.root + 9) % 12, true)
        }
    }

    /// `{name, short, camelot, root, minor}`.
    pub fn json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("name", JsonValue::String(self.name())),
            ("short", JsonValue::String(self.short())),
            ("camelot", JsonValue::String(self.camelot())),
            ("root", JsonValue::str(NAMES[self.root as usize])),
            ("minor", JsonValue::Bool(self.minor)),
        ]))
    }
}

// ---------------------------------------------------------------------------------------------
// Compatibility

/// How two keys sit on the Camelot wheel, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    /// The same key.
    Same,
    /// Relative major / minor (8A ↔ 8B): the same notes.
    Relative,
    /// One step round the wheel (8A ↔ 7A / 9A): a fifth apart, one note differs.
    Adjacent,
    /// One step round and across (8A ↔ 9B, 8B ↔ 7A): works for a phrase, a little colour.
    Diagonal,
    /// Two steps round (8A ↔ 10A): the "energy boost"; two notes differ. Fine on a short blend.
    Boost,
    /// The same tonic, other mode (A minor ↔ A major): a deliberate change of colour.
    Parallel,
    /// Anything else: notes will rub.
    Clash,
}

impl Compat {
    pub fn name(self) -> &'static str {
        match self {
            Compat::Same => "same",
            Compat::Relative => "relative",
            Compat::Adjacent => "adjacent",
            Compat::Diagonal => "diagonal",
            Compat::Boost => "boost",
            Compat::Parallel => "parallel",
            Compat::Clash => "clash",
        }
    }

    /// 0..1, how safely the two mix (1 = same key).
    pub fn score(self) -> f64 {
        match self {
            Compat::Same => 1.0,
            Compat::Relative => 0.9,
            Compat::Adjacent => 0.8,
            Compat::Diagonal => 0.6,
            Compat::Boost => 0.5,
            Compat::Parallel => 0.4,
            Compat::Clash => 0.0,
        }
    }

    /// Mixes without a warning: same, relative or one step.
    pub fn harmonic(self) -> bool {
        matches!(self, Compat::Same | Compat::Relative | Compat::Adjacent)
    }

    /// Good enough to blend for a phrase (everything but a clash and a parallel change).
    pub fn blendable(self) -> bool {
        self.harmonic() || matches!(self, Compat::Diagonal | Compat::Boost)
    }

    /// A line for the deck panel: "8A → 9A: adjacent, mixes".
    pub fn hint(self) -> &'static str {
        match self {
            Compat::Same => "same key: mixes",
            Compat::Relative => "relative major/minor: mixes",
            Compat::Adjacent => "a fifth apart: mixes",
            Compat::Diagonal => "a step across the wheel: blend briefly",
            Compat::Boost => "two steps (energy boost): blend briefly",
            Compat::Parallel => "same tonic, other mode: a deliberate colour change",
            Compat::Clash => "keys clash: transpose or cut",
        }
    }
}

/// How `a` and `b` mix, by the Camelot wheel.
pub fn compat(a: Key, b: Key) -> Compat {
    if a == b {
        return Compat::Same;
    }
    let (na, la) = a.camelot_parts();
    let (nb, lb) = b.camelot_parts();
    // steps round the wheel, 0..6
    let d = ((na as i32 - nb as i32).rem_euclid(12)).min((nb as i32 - na as i32).rem_euclid(12));
    match (d, la == lb) {
        (0, false) => Compat::Relative,
        (1, true) => Compat::Adjacent,
        (1, false) => Compat::Diagonal,
        (2, true) => Compat::Boost,
        _ if a.root == b.root => Compat::Parallel,
        _ => Compat::Clash,
    }
}

/// A shift for `b` that makes it mix with `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transpose {
    /// Semitones to move `b`, -6..=6 (0: leave it).
    pub semitones: i32,
    /// `b` after the move.
    pub key: Key,
    pub compat: Compat,
}

impl Transpose {
    pub fn json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("semitones", JsonValue::Number(self.semitones as f64)),
            ("key", self.key.json()),
            ("compat", JsonValue::str(self.compat.name())),
        ]))
    }
}

/// The smallest shift of `b` (±6 semitones) that makes it mix harmonically with `a`. Each
/// semitone moved costs 6 points against the compatibility's 100 (a stretched part sounds less
/// like itself the further it moves), so a 1-semitone move to a relative key beats a 5-semitone
/// move to the same key; ties go upwards. `b` already harmonic: 0.
pub fn suggest_transpose(a: Key, b: Key) -> Transpose {
    let mut best: Option<(i32, Transpose)> = None;
    for s in -6..=6i32 {
        let k = b.transposed(s);
        let c = compat(a, k);
        if !c.harmonic() {
            continue;
        }
        let rank = (c.score() * 100.0) as i32 * 10 - s.abs() * 60 + (s > 0) as i32;
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, Transpose { semitones: s, key: k, compat: c }));
        }
    }
    let t = best.map(|(_, t)| t).unwrap_or(Transpose { semitones: 0, key: b, compat: compat(a, b) });
    // already harmonic: the best move is none, unless moving buys the same key
    let here = compat(a, b);
    if here.harmonic() && t.compat.score() <= here.score() {
        return Transpose { semitones: 0, key: b, compat: here };
    }
    t
}

/// The two decks' keys side by side, for the panel and `key status`: each key, how they mix, and
/// the transposition of B onto A that would make them.
pub fn pair_json(a: Option<Key>, b: Option<Key>) -> JsonValue {
    let mut o = JsonObject::new();
    match (a, b) {
        (Some(a), Some(b)) => {
            let c = compat(a, b);
            o.set("compat", JsonValue::str(c.name()));
            o.set("score", JsonValue::Number(c.score()));
            o.set("harmonic", JsonValue::Bool(c.harmonic()));
            o.set("hint", JsonValue::String(format!("{} → {}: {}", a.camelot(), b.camelot(), c.hint())));
            if !c.harmonic() {
                o.set("suggest", suggest_transpose(a, b).json());
            }
        }
        _ => {
            o.set("compat", JsonValue::Null);
            o.set("hint", JsonValue::str("a deck has no key yet"));
        }
    }
    JsonValue::Object(o)
}

// ---------------------------------------------------------------------------------------------
// Detection

/// A key profile: how strongly each scale degree (tonic first) belongs to a major and a minor key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    KrumhanslKessler,
    Temperley,
    AlbrechtShanahan,
    /// The mean of the three correlations, per key (the default).
    Ensemble,
}

/// Krumhansl & Kessler 1982, table 1 (probe-tone ratings).
const KK: ([f64; 12], [f64; 12]) = (
    [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88],
    [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17],
);
/// Temperley 2007, the Kostka–Payne corpus (music21 `TemperleyKostkaPayne`).
const TEMPERLEY: ([f64; 12], [f64; 12]) = (
    [0.748, 0.060, 0.488, 0.082, 0.670, 0.460, 0.096, 0.715, 0.104, 0.366, 0.057, 0.400],
    [0.712, 0.084, 0.474, 0.618, 0.049, 0.460, 0.105, 0.747, 0.404, 0.067, 0.133, 0.330],
);
/// Albrecht & Shanahan 2013, the corpus profile.
const ALBRECHT_SHANAHAN: ([f64; 12], [f64; 12]) = (
    [0.238, 0.006, 0.111, 0.006, 0.137, 0.094, 0.016, 0.214, 0.009, 0.080, 0.008, 0.081],
    [0.220, 0.006, 0.104, 0.123, 0.019, 0.103, 0.012, 0.214, 0.062, 0.022, 0.061, 0.052],
);

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::KrumhanslKessler => "krumhansl",
            Profile::Temperley => "temperley",
            Profile::AlbrechtShanahan => "albrecht",
            Profile::Ensemble => "ensemble",
        }
    }

    pub fn parse(s: &str) -> Option<Profile> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "kk" | "krumhansl" | "krumhansl-kessler" => Profile::KrumhanslKessler,
            "temperley" | "kostka-payne" => Profile::Temperley,
            "albrecht" | "albrecht-shanahan" | "as" => Profile::AlbrechtShanahan,
            "" | "ensemble" | "all" => Profile::Ensemble,
            _ => return None,
        })
    }

    pub const ALL: [Profile; 3] = [Profile::KrumhanslKessler, Profile::Temperley, Profile::AlbrechtShanahan];

    fn tables(self) -> Option<&'static ([f64; 12], [f64; 12])> {
        match self {
            Profile::KrumhanslKessler => Some(&KK),
            Profile::Temperley => Some(&TEMPERLEY),
            Profile::AlbrechtShanahan => Some(&ALBRECHT_SHANAHAN),
            Profile::Ensemble => None,
        }
    }
}

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let (ma, mb) = (a.iter().sum::<f64>() / 12.0, b.iter().sum::<f64>() / 12.0);
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let (x, y) = (a[i] - ma, b[i] - mb);
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da <= 0.0 || db <= 0.0 {
        return 0.0;
    }
    num / libm::sqrt(da * db)
}

/// The correlation of `chroma` with every key under `profile`: `[major[12], minor[12]]` by tonic.
pub fn scores(chroma: &[f64; 12], profile: Profile) -> [[f64; 12]; 2] {
    let mut out = [[0.0f64; 12]; 2];
    match profile.tables() {
        Some((maj, min)) => {
            for (m, table) in [maj, min].iter().enumerate() {
                for tonic in 0..12 {
                    let mut rot = [0.0f64; 12];
                    for i in 0..12 {
                        rot[(i + tonic) % 12] = table[i];
                    }
                    out[m][tonic] = pearson(chroma, &rot);
                }
            }
        }
        None => {
            for p in Profile::ALL {
                let s = scores(chroma, p);
                for m in 0..2 {
                    for t in 0..12 {
                        out[m][t] += s[m][t] / Profile::ALL.len() as f64;
                    }
                }
            }
        }
    }
    out
}

/// A key estimated from audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub key: Key,
    /// The winning correlation, -1..1.
    pub correlation: f64,
    /// How clearly it beat the next key, 0..1.
    pub confidence: f64,
    /// Enough pitch to trust (false for drums and noise: the chroma is flat).
    pub tonal: bool,
    pub profile: Profile,
    /// Each single profile's own answer (for the ensemble), to see when they disagree.
    pub votes: Vec<(Profile, Key)>,
    pub chroma: [f64; 12],
}

impl Detection {
    pub fn json(&self) -> JsonValue {
        let mut o = match self.key.json() {
            JsonValue::Object(o) => o,
            _ => JsonObject::new(),
        };
        o.set("correlation", JsonValue::Number(round3(self.correlation)));
        o.set("confidence", JsonValue::Number(round3(self.confidence)));
        o.set("tonal", JsonValue::Bool(self.tonal));
        o.set("profile", JsonValue::str(self.profile.name()));
        o.set("source", JsonValue::str("audio"));
        let mut votes = JsonObject::new();
        for (p, k) in &self.votes {
            votes.set(p.name(), JsonValue::String(k.camelot()));
        }
        o.set("votes", JsonValue::Object(votes));
        o.set("chroma", JsonValue::Array(self.chroma.iter().map(|v| JsonValue::Number(round3(*v))).collect()));
        JsonValue::Object(o)
    }
}

fn round3(x: f64) -> f64 {
    libm::round(x * 1000.0) / 1000.0
}

fn best_of(s: &[[f64; 12]; 2]) -> (f64, Key, f64) {
    let mut best = (f64::NEG_INFINITY, Key::new(0, false));
    for (m, row) in s.iter().enumerate() {
        for (t, r) in row.iter().enumerate() {
            if *r > best.0 {
                best = (*r, Key::new(t as u8, m == 1));
            }
        }
    }
    let mut second = f64::NEG_INFINITY;
    for (m, row) in s.iter().enumerate() {
        for (t, r) in row.iter().enumerate() {
            if Key::new(t as u8, m == 1) != best.1 && *r > second {
                second = *r;
            }
        }
    }
    (best.0, best.1, second)
}

/// The key a chroma points at, under `profile`.
pub fn from_chroma(chroma: &[f64; 12], profile: Profile) -> Detection {
    let s = scores(chroma, profile);
    let (r, key, second) = best_of(&s);
    let confidence = ((r - second) / 0.25).clamp(0.0, 1.0);
    let mean = 1.0 / 12.0;
    let total: f64 = chroma.iter().sum();
    let peak = chroma.iter().cloned().fold(0.0f64, f64::max) / if total > 0.0 { total } else { 1.0 };
    let tonal = total > 0.0 && r >= 0.5 && peak >= mean * 1.5;
    let votes = if profile == Profile::Ensemble {
        Profile::ALL.iter().map(|p| (*p, best_of(&scores(chroma, *p)).1)).collect()
    } else {
        Vec::new()
    };
    Detection { key, correlation: r, confidence, tonal, profile, votes, chroma: *chroma }
}

/// How the chroma is taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromaOpts {
    pub min_hz: f64,
    pub max_hz: f64,
    /// Harmonic subtraction, 0..1: each semitone's 2nd–6th harmonics (an octave, a twelfth, two
    /// octaves, a major seventeenth, a nineteenth up) lose `strength / h` of its energy before
    /// folding, so a bright synth's overtones stop voting for the key a fifth above. 0: off.
    pub harmonics: f64,
}

impl ChromaOpts {
    /// djrust's loop-root chroma: 60 Hz – 5 kHz, no subtraction.
    pub const PLAIN: ChromaOpts = ChromaOpts { min_hz: 60.0, max_hz: 5000.0, harmonics: 0.0 };
    /// What [`detect`] uses (docs/KEY.md, "Overtones"): chosen on rendered generated tracks,
    /// whose keys are known.
    pub const DEFAULT: ChromaOpts = ChromaOpts { min_hz: 50.0, max_hz: 4000.0, harmonics: 0.8 };
}

const SIZE: usize = 8192;
const HOP: usize = 4096;
const MAX_SECONDS: f64 = 60.0;
/// Semitones from MIDI 0.
const SEMIS: usize = 128;
/// Harmonics 2..=6 and the semitones each lands above its fundamental.
const HARMONIC_SEMIS: [(f64, usize); 5] = [(2.0, 12), (3.0, 19), (4.0, 24), (5.0, 28), (6.0, 31)];

/// The 12-bin chroma of `x` (mono) at `sr` under `o`: FFT magnitudes (fourth-root compressed,
/// so one loud partial does not own a frame) summed per semitone over the frames, harmonics
/// subtracted lowest note first, folded onto pitch classes and normalised to sum to 1. Long
/// audio is read with a larger hop so the cost stays bounded. All zeros for silence or audio
/// shorter than one 8192-sample frame.
pub fn chroma_with(x: &[f32], sr: f64, o: ChromaOpts) -> [f64; 12] {
    use crate::musicality::fft::{hann_norm, Fft};
    let mut out = [0.0f64; 12];
    if x.len() < SIZE || !(sr > 0.0) {
        return out;
    }
    let half = SIZE / 2;
    let bin_hz = sr / SIZE as f64;
    let mut semi_of: Vec<i16> = alloc::vec![-1; half];
    for (k, c) in semi_of.iter_mut().enumerate().skip(1) {
        let f = k as f64 * bin_hz;
        if f >= o.min_hz && f < o.max_hz.min(sr / 2.0) {
            let midi = libm::round(69.0 + 12.0 * libm::log2(f / 440.0));
            if (0.0..SEMIS as f64).contains(&midi) {
                *c = midi as i16;
            }
        }
    }
    let max_frames = (MAX_SECONDS * sr / HOP as f64) as usize;
    let frames = (x.len() - SIZE) / HOP + 1;
    let step = (frames / max_frames.max(1)).max(1);
    let window = hann_norm(SIZE);
    let mut fft = Fft::new(SIZE);
    let mut buf = alloc::vec![0.0f64; SIZE];
    let mut semis = [0.0f64; SEMIS];
    let mut f = 0;
    while f < frames {
        let start = f * HOP;
        for i in 0..SIZE {
            buf[i] = x[start + i] as f64 * window[i];
        }
        let p = fft.zrip(&buf);
        for k in 1..half {
            if semi_of[k] >= 0 {
                semis[semi_of[k] as usize] += libm::sqrt(libm::sqrt(p.re[k] * p.re[k] + p.im[k] * p.im[k]));
            }
        }
        f += step;
    }
    if o.harmonics > 0.0 {
        for m in 0..SEMIS {
            let e = semis[m];
            if e <= 0.0 {
                continue;
            }
            for (h, up) in HARMONIC_SEMIS {
                if m + up < SEMIS {
                    semis[m + up] = (semis[m + up] - e * o.harmonics / h).max(0.0);
                }
            }
        }
    }
    for (m, e) in semis.iter().enumerate() {
        out[m % 12] += e;
    }
    let total: f64 = out.iter().sum();
    if total > 0.0 {
        for v in out.iter_mut() {
            *v /= total;
        }
    }
    out
}

/// [`detect`] with the chroma taken under `o`.
pub fn detect_with(x: &[f32], sr: f64, profile: Profile, o: ChromaOpts) -> Option<Detection> {
    let c = chroma_with(x, sr, o);
    if c.iter().sum::<f64>() <= 0.0 {
        return None;
    }
    Some(from_chroma(&c, profile))
}

/// The key of a mono signal at `sr`. None when it is shorter than the analysis frame (8192
/// samples) or silent.
pub fn detect(x: &[f32], sr: f64, profile: Profile) -> Option<Detection> {
    detect_with(x, sr, profile, ChromaOpts::DEFAULT)
}

/// [`detect`] on a stereo (or mono) pair, mixed to mono.
pub fn detect_stereo(left: &[f32], right: Option<&[f32]>, sr: f64, profile: Profile) -> Option<Detection> {
    match right {
        Some(r) if r.len() == left.len() => {
            let m: Vec<f32> = left.iter().zip(r).map(|(a, b)| (a + b) * 0.5).collect();
            detect(&m, sr, profile)
        }
        _ => detect(left, sr, profile),
    }
}
