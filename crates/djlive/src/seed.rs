//! `seed(genre, n[, structure])`: a track of the beats generator, as BeatScript (docs/BTW.md, "Start from a seed").
//!
//! A seed reference is a value: the generated track written out as BeatScript, never audio. It is resolved before a
//! program is lowered, so everything downstream (the compiler, the browser, the phone) only ever sees plain BeatScript:
//!
//! ```text
//! arrange from seed(techno, 42)                          the whole track: lets, sections and an arrange
//! section intro from seed(techno, 42)[1..8]              bars 1 to 8 as one section
//! deck B: bass seed(techno, 42).bass[1..8] inst=fm-bass  one part's bars as a pattern
//! let riff = seed(techno, 42).bass[5..8]                 the same, named
//! ```
//!
//! The generator is `djrust::gen::compose` (beats.bitwrap.io's, byte for byte: `djrust/tests/beats_composer.rs` holds it
//! to beats' own JavaScript) and the notes are what `musicality::note_log` hears it play. Seeds are versioned ([`VERSION`]):
//! a reference always means the same music, so the CID of the unexpanded text is stable, and [`expand`] gives the
//! literal text it stands for, which plays the same (tests/seed.rs).
//!
//! Pure: no I/O and no clock. A small cache keeps the last few tracks, because composing one is the slow part.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use djengine::gen::compose_with;
use djengine::json::JsonValue;
use djengine::musicality::note_log;

use crate::diag::{Diagnostic, Span};
use crate::lex::{lex, Tok, Token};
use crate::parse::{nearest, ROLES};

/// The seeds this BeatScript knows: `#! seeds: v1` (also what a program with no such header means).
pub const VERSION: &str = "v1";
/// The song structures of the generator, the default first (beats.bitwrap.io's "standard").
pub const STRUCTURES: [&str; 4] = ["standard", "loop", "minimal", "extended"];
/// Drum kits a part may name with `kit=` (the web engine's).
pub const KITS: [&str; 6] = ["drums", "drums-808", "drums-breakbeat", "drums-cr78", "drums-lofi", "drums-v8"];
/// The decks the parts go on: the drums and the bass on A, the rest on B.
const DECK_A: [&str; 5] = ["kick", "snare", "hihat", "clap", "bass"];
const DECK_B: [&str; 3] = ["melody", "harmony", "arp"];
/// The most bars of changing music a part can carry (an alternation's cap).
pub(crate) const MAX_PERIOD: usize = 64;

/// The genres of seeds v1: beats.bitwrap.io's, the genres of the bitwrap ontology (https://ontology.bitwrap.io/genre/...).
/// The engine's tables have a few more (dj-only); a seed names only these, so that every seed is a beats track and
/// `tests/seed.rs` can hold it to beats' own JavaScript.
pub const GENRES: [&str; 19] = [
    "ambient", "blues", "bossa", "country", "dnb", "dubstep", "edm", "funk", "garage", "house", "jazz", "lofi", "metal", "reggae", "speedcore", "synthwave", "techno", "trance", "trap",
];

/// A word the parser reads as a reference to music the engine makes (one word since `collapse`): `seed(...)...`, or a
/// `gen@N ...` call (gen.rs).
pub fn is_ref_word(w: &str) -> bool {
    w.starts_with("seed(") || crate::gen::is_call_word(w)
}

/// The genres a seed may name ([`GENRES`]).
pub fn genres() -> Vec<String> {
    GENRES.iter().map(|s| s.to_string()).collect()
}

/// `seed(genre, n[, structure])[.part][[a..b]]`, as read.
#[derive(Clone, Debug, PartialEq)]
pub struct SeedRef {
    pub genre: String,
    pub genre_span: Span,
    pub n: u32,
    pub structure: Option<String>,
    pub part: Option<String>,
    pub part_span: Span,
    /// Bars `a..b`, 1-based, both in.
    pub bars: Option<(u32, u32)>,
    pub bars_span: Span,
    pub span: Span,
}

impl SeedRef {
    pub fn structure(&self) -> &str {
        self.structure.as_deref().unwrap_or(STRUCTURES[0])
    }

    /// The canonical spelling: `seed(techno, 42).bass[1..8]`; the default structure is left out and `[3..3]` is `[3]`.
    pub fn text(&self) -> String {
        let mut s = format!("seed({}, {}", self.genre, self.n);
        if self.structure() != STRUCTURES[0] {
            s.push_str(&format!(", {}", self.structure()));
        }
        s.push(')');
        if let Some(p) = &self.part {
            s.push('.');
            s.push_str(p);
        }
        match self.bars {
            Some((a, b)) if a == b => s.push_str(&format!("[{a}]")),
            Some((a, b)) => s.push_str(&format!("[{a}..{b}]")),
            None => {}
        }
        s
    }
}

/// A seed reference in a token stream: tokens `start..end`.
pub(crate) struct Run {
    pub start: usize,
    pub end: usize,
    pub span: Span,
    pub r: Result<SeedRef, Diagnostic>,
}

fn touches(a: &Token, b: &Token) -> bool {
    a.span.end == b.span.start
}

/// Every `seed(` in `toks`, read as far as it goes.
pub(crate) fn runs(toks: &[Token]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let is_seed = matches!(&toks[i].tok, Tok::Word(w) if w == "seed") && toks.get(i + 1).is_some_and(|t| t.tok == Tok::LParen && touches(&toks[i], t));
        if !is_seed {
            i += 1;
            continue;
        }
        let (end, r) = read_ref(toks, i);
        let span = toks[i].span.to(toks[end - 1].span);
        out.push(Run { start: i, end, span, r: r.map(|mut s| {
            s.span = span;
            s
        }) });
        i = end;
    }
    out
}

const SHAPE: &str = "seed(genre, n), seed(genre, n, structure), then `.part` and `[a..b]` bars: seed(techno, 42).bass[1..8]";

/// Read one reference from `toks[i]` (the word `seed`): where it ends, and the reference or what is wrong with it.
fn read_ref(toks: &[Token], i: usize) -> (usize, Result<SeedRef, Diagnostic>) {
    let mut j = i + 2;
    let at = |j: usize| toks.get(j).map(|t| &t.tok);
    let here = |j: usize| toks.get(j).or(toks.last()).map_or(Span::new(0, 0), |t| t.span);
    let fail = |j: usize, msg: String| (j.max(i + 2).min(toks.len()), Err(Diagnostic::error(toks[i].span.to(here(j.saturating_sub(1).max(i))), msg).help(SHAPE)));
    let close = |mut j: usize| {
        // the rest of the parentheses go with the error, so that one mistake is one error
        while j < toks.len() && !matches!(toks[j].tok, Tok::RParen | Tok::Newline | Tok::Eof) {
            j += 1;
        }
        if at(j) == Some(&Tok::RParen) {
            j + 1
        } else {
            j
        }
    };
    let Some(Tok::Word(genre)) = at(j).cloned() else {
        let (_, e) = fail(j + 1, "`seed(` takes a genre first: seed(techno, 42)".into());
        return (close(j), e);
    };
    let genre_span = toks[j].span;
    j += 1;
    if at(j) != Some(&Tok::Comma) {
        let (_, e) = fail(j + 1, format!("`seed({genre}` needs a seed number after the genre: seed({genre}, 42)"));
        return (close(j), e);
    }
    j += 1;
    let n = match at(j) {
        Some(Tok::Num { value, int: true }) if (0.0..=4_294_967_295.0).contains(value) => *value as u32,
        _ => {
            let (_, e) = fail(j + 1, "a seed is a whole number, 0 to 4294967295".into());
            return (close(j), e);
        }
    };
    j += 1;
    let mut structure = None;
    if at(j) == Some(&Tok::Comma) {
        j += 1;
        match at(j).cloned() {
            Some(Tok::Word(w)) if STRUCTURES.contains(&w.as_str()) => structure = Some(w),
            Some(Tok::Word(w)) => {
                let mut d = Diagnostic::error(toks[j].span, format!("`{w}` is not a song structure of the generator")).expected(&STRUCTURES).found(format!("`{w}`"));
                if let Some(n) = nearest(&w, STRUCTURES) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                return (close(j), Err(d));
            }
            _ => {
                let (_, e) = fail(j + 1, format!("the third argument of seed() is a structure: {}", STRUCTURES.join(", ")));
                return (close(j), e);
            }
        }
        j += 1;
    }
    if at(j) != Some(&Tok::RParen) {
        let (_, e) = fail(j + 1, "expected `)` to close seed(...)".into());
        return (close(j), e);
    }
    j += 1;
    let mut part = None;
    let mut part_span = Span::new(0, 0);
    if at(j) == Some(&Tok::Dot) {
        match at(j + 1).cloned() {
            Some(Tok::Word(w)) if ROLES.contains(&w.as_str()) => {
                part_span = toks[j + 1].span;
                part = Some(w);
                j += 2;
            }
            Some(Tok::Word(w)) => {
                let mut d = Diagnostic::error(toks[j + 1].span, format!("`.{w}`: a seed's parts are its roles")).expected(&ROLES).found(format!("`{w}`"));
                if let Some(n) = nearest(&w, ROLES) {
                    d = d.help(format!("did you mean `.{n}`?"));
                }
                return (j + 2, Err(d));
            }
            _ => return (j + 1, Err(Diagnostic::error(toks[j].span, "a `.` after seed(...) picks a part: .kick .bass .melody ...").expected(&ROLES))),
        }
    }
    let mut bars = None;
    let mut bars_span = Span::new(0, 0);
    // (spaced or not: a seed is a part's whole pattern, so no group can follow it)
    if at(j) == Some(&Tok::LBrack) && matches!(at(j + 1), Some(Tok::Num { .. })) && matches!(at(j + 2), Some(Tok::DotDot | Tok::RBrack)) {
        let open = toks[j].span;
        let num = |j: usize| match at(j) {
            Some(Tok::Num { value, int: true }) if *value >= 1.0 && *value <= 100_000.0 => Some(*value as u32),
            _ => None,
        };
        let shut = |mut k: usize| {
            while k < toks.len() && !matches!(toks[k].tok, Tok::RBrack | Tok::Newline | Tok::Eof) {
                k += 1;
            }
            if at(k) == Some(&Tok::RBrack) {
                k + 1
            } else {
                k
            }
        };
        let bad = |k: usize, msg: &str| (shut(k), Err(Diagnostic::error(open.to(here(k)), msg.to_string()).help("bars count from 1: [1..8] is the first eight, [9] the ninth")));
        let Some(a) = num(j + 1) else { return bad(j + 1, "a seed's bars are `[a..b]`, whole numbers from 1") };
        let (b, k) = if at(j + 2) == Some(&Tok::DotDot) {
            match num(j + 3) {
                Some(b) => (b, j + 4),
                None => return bad(j + 3, "a seed's bars are `[a..b]`, whole numbers from 1"),
            }
        } else {
            (a, j + 2)
        };
        if at(k) != Some(&Tok::RBrack) {
            return bad(k, "expected `]` to close the bars");
        }
        if b < a {
            return (k + 1, Err(Diagnostic::error(open.to(toks[k].span), format!("a seed's bars run backwards: {a}..{b}")).help(format!("[{b}..{a}]"))));
        }
        bars = Some((a, b));
        bars_span = open.to(toks[k].span);
        j = k + 1;
    }
    (j, Ok(SeedRef { genre, genre_span, n, structure, part, part_span, bars, bars_span, span: Span::new(0, 0) }))
}

/// Each reference made one word token (its canonical spelling) for the parser, and what is wrong with any of them.
pub(crate) fn collapse(toks: Vec<Token>) -> (Vec<Token>, Vec<Diagnostic>) {
    let rs = runs(&toks);
    if rs.is_empty() {
        return (toks, Vec::new());
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut diags = Vec::new();
    let mut i = 0;
    for r in rs {
        out.extend_from_slice(&toks[i..r.start]);
        let text = match r.r {
            Ok(s) => s.text(),
            Err(d) => {
                diags.push(d);
                "seed()".to_string()
            }
        };
        out.push(Token { tok: Tok::Word(text), span: r.span });
        i = r.end;
    }
    out.extend_from_slice(&toks[i..]);
    (out, diags)
}

// ---------------------------------------------------------------- the track

/// One note of the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Note {
    pub note: u8,
    pub vel: u8,
    /// In sixteenths.
    pub len: u8,
}

/// One role of the track: its sound and, bar by bar, step by step, its notes.
#[derive(Clone, Debug)]
pub struct PartTrack {
    pub role: &'static str,
    /// The generator's instrument (a voice, or a kit for a drum).
    pub voice: String,
    pub bars: Vec<[Vec<Note>; 16]>,
}

/// A generated track, as notes on the sixteenth grid.
#[derive(Clone, Debug)]
pub struct Track {
    pub genre: String,
    pub n: u32,
    pub structure: String,
    pub name: String,
    pub tempo: f64,
    pub swing: f64,
    /// A BeatScript key: `c`, `am`, `d:dorian`.
    pub key: String,
    pub bars: u32,
    /// (name, first bar from 0, bars).
    pub sections: Vec<(String, u32, u32)>,
    /// In the order of [`ROLES`]; only the roles it has.
    pub parts: Vec<PartTrack>,
    /// The generator's project's model CID (sha256 of its canonical JSON), as beats' fixtures name it.
    pub model_sha: String,
    /// A loop (structure `loop`): no song form, it goes round.
    pub looped: bool,
}

impl Track {
    pub fn part(&self, role: &str) -> Option<&PartTrack> {
        self.parts.iter().find(|p| p.role == role)
    }
}

/// The track of `(genre, n, structure)`; the genre and the structure must be ones the generator has (see [`genres`]).
pub fn track(genre: &str, n: u32, structure: &str) -> Arc<Track> {
    static CACHE: Mutex<Vec<Arc<Track>>> = Mutex::new(Vec::new());
    if let Ok(c) = CACHE.lock() {
        if let Some(t) = c.iter().find(|t| t.genre == genre && t.n == n && t.structure == structure) {
            return t.clone();
        }
    }
    let t = Arc::new(make_track(genre, n, structure));
    if let Ok(mut c) = CACHE.lock() {
        if c.len() >= 8 {
            c.remove(0);
        }
        c.push(t.clone());
    }
    t
}

const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

fn key_word(root: i64, scale: &str) -> String {
    let tonic = SHARP[root.rem_euclid(12) as usize];
    match scale.to_ascii_lowercase().as_str() {
        "major" | "" => tonic.to_string(),
        "minor" | "naturalminor" => format!("{tonic}m"),
        "pentatonic" | "majorpentatonic" => format!("{tonic}:pent"),
        "minpentatonic" | "minorpentatonic" => format!("{tonic}:minpent"),
        "harmonicminor" => format!("{tonic}:harmonic"),
        m @ ("dorian" | "mixolydian" | "phrygian" | "blues") => format!("{tonic}:{m}"),
        _ => tonic.to_string(),
    }
}

/// The role of a generator net: `kick-0` is the kick, `bass-12` the bass, `snare` the snare.
fn net_role(id: &str) -> Option<&'static str> {
    let base = match id.rsplit_once('-') {
        Some((b, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => b,
        _ => id,
    };
    ROLES.iter().copied().find(|r| *r == base)
}

fn make_track(genre: &str, n: u32, structure: &str) -> Track {
    // beats' "loop" is no structure at all: the generator's one looping pattern (as its UI and fixtures ask for it)
    let mut ov = vec![("seed", JsonValue::Number(f64::from(n)))];
    if structure != "loop" {
        ov.push(("structure", JsonValue::str(structure)));
    }
    let p = compose_with(genre, &ov);
    // a song is as long as its structure; a loop (no structure) as long as its nets take to come round
    let looped = p.structure.iter().all(|s| s.steps <= 0);
    let steps = if looped {
        let gcd = |mut a: usize, mut b: usize| {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        };
        p.nets.iter().filter(|(id, nb)| net_role(id).is_some() && !nb.transitions.is_empty()).map(|(_, nb)| nb.transitions.len()).fold(16, |a, n| (a / gcd(a, n) * n).min(MAX_PERIOD * 16))
    } else {
        note_log::length(&p).max(0) as usize
    };
    let bars = steps.div_ceil(16).max(1);
    let mut parts: BTreeMap<&'static str, PartTrack> = BTreeMap::new();
    for e in note_log::capture(&p, Some((bars * 16) as i64)) {
        let Some(role) = net_role(&e.net) else { continue };
        let step = (e.tick - 1).max(0) as usize;
        if step >= bars * 16 || !(0..=127).contains(&e.note) {
            continue;
        }
        let voice = p.nets.get(&e.net).map(|nb| nb.track.instrument.clone()).unwrap_or_default();
        let pt = parts.entry(role).or_insert_with(|| PartTrack { role, voice, bars: vec![Default::default(); bars] });
        let note = Note { note: e.note as u8, vel: e.velocity.clamp(1, 127) as u8, len: e.steps.round().clamp(1.0, 16.0) as u8 };
        let slot = &mut pt.bars[step / 16][step % 16];
        if !slot.iter().any(|x| x.note == note.note) {
            slot.push(note);
            slot.sort();
        }
    }
    let mut sections = Vec::new();
    let mut at = 0u32;
    for s in &p.structure {
        let b = (s.steps.max(0) as u32).div_ceil(16);
        if b > 0 {
            sections.push((s.name.clone(), at, b));
            at += b;
        }
    }
    if sections.is_empty() || at < bars as u32 {
        sections.push((if looped { "groove".into() } else if sections.is_empty() { "track".into() } else { "tail".into() }, at, bars as u32 - at));
    }
    let model = p.canonical_json();
    Track {
        genre: genre.into(),
        n,
        structure: structure.into(),
        name: p.name.clone(),
        tempo: p.tempo,
        swing: p.swing,
        key: key_word(p.root_note.unwrap_or(60), &p.scale_name),
        bars: bars as u32,
        sections,
        parts: ROLES.iter().filter_map(|r| parts.remove(r)).collect(),
        model_sha: djengine::cid::hex(&djengine::cid::sha256(model.as_bytes())),
        looped,
    }
}

// ---------------------------------------------------------------- writing it out

pub(crate) fn is_drum(role: &str) -> bool {
    matches!(role, "kick" | "snare" | "hihat" | "clap")
}

/// The note a drum part's `x` plays (compile.rs).
pub(crate) fn drum_note(role: &str) -> u8 {
    match role {
        "kick" => 36,
        "snare" => 38,
        "hihat" => 42,
        _ => 39,
    }
}

pub fn note_name(n: u8) -> String {
    format!("{}{}", SHARP[usize::from(n % 12)], i32::from(n / 12) - 1)
}

/// How a part sounds over some bars: its params and the velocity from which a hit is accented.
struct Feel {
    params: Vec<(&'static str, String)>,
    accent: u8,
}

fn feel(pt: &PartTrack, bars: &[[Vec<Note>; 16]]) -> Feel {
    let notes: Vec<Note> = bars.iter().flat_map(|b| b.iter().flatten().copied()).collect();
    let drum = is_drum(pt.role);
    let mut params = Vec::new();
    if drum {
        if KITS.contains(&pt.voice.as_str()) {
            params.push(("kit", pt.voice.clone()));
        }
    } else if !pt.voice.is_empty() {
        // a name that starts with a digit cannot be written (`inst=808-bass` reads as 808 - bass)
        let v = if pt.voice.starts_with(|c: char| c.is_ascii_digit()) { "sub-bass".to_string() } else { pt.voice.clone() };
        params.push(("inst", v));
    }
    let Some(lo) = notes.iter().map(|n| n.vel).min() else { return Feel { params, accent: u8::MAX } };
    // a hit well above the quietest is an accent (`!`: 27 louder, compile.rs); the rest are a range
    let accent = if notes.iter().any(|n| n.vel >= lo.saturating_add(20)) { lo.saturating_add(20) } else { u8::MAX };
    let hi = notes.iter().map(|n| n.vel).filter(|v| *v < accent).max().unwrap_or(lo);
    params.push(("vel", if hi > lo { format!("{lo}..{hi}") } else { lo.to_string() }));
    let mut lens: BTreeMap<u8, usize> = BTreeMap::new();
    for n in &notes {
        *lens.entry(n.len).or_default() += 1;
    }
    let len = lens.iter().max_by_key(|(l, c)| (**c, std::cmp::Reverse(**l))).map_or(1, |(l, _)| *l);
    if len != if drum { 1 } else { 2 } {
        params.push(("len", len.to_string()));
    }
    Feel { params, accent }
}

fn slot_text(role: &str, slot: &[Note], accent: u8) -> String {
    let words: Vec<String> = slot
        .iter()
        .map(|n| {
            let w = if is_drum(role) && n.note == drum_note(role) { "x".to_string() } else { note_name(n.note) };
            if n.vel >= accent {
                format!("{w}!")
            } else {
                w
            }
        })
        .collect();
    match words.len() {
        0 => "~".into(),
        1 => words.into_iter().next().unwrap_or_default(),
        _ => format!("[{}]", words.join(",")),
    }
}

/// One bar as mini-notation, on the coarsest grid its notes sit on: `x x x x`, `c2 ~ ~ c2 ~ ~ d#2 ~`.
pub(crate) fn bar_text(role: &str, bar: &[Vec<Note>; 16], accent: u8) -> String {
    if bar.iter().all(Vec::is_empty) {
        return "~".into();
    }
    let terms = [1usize, 2, 4, 8, 16].into_iter().find(|t| bar.iter().enumerate().all(|(i, s)| s.is_empty() || i % (16 / t) == 0)).unwrap_or(16);
    (0..terms).map(|k| slot_text(role, &bar[k * (16 / terms)], accent)).collect::<Vec<_>>().join(" ")
}

/// The shortest run of bars the whole repeats: `a b a b` is `a b`.
pub(crate) fn period(texts: &[String]) -> usize {
    let n = texts.len();
    (1..=n).find(|p| n % p == 0 && (0..n).all(|i| texts[i] == texts[i % p])).unwrap_or(n)
}

/// A pattern of bars: one bar is its own text, more are an alternation of them, each squeezed into its bar.
fn bars_pattern(terms: &[String]) -> String {
    if terms.len() == 1 {
        return terms[0].clone();
    }
    let inner: Vec<String> = terms.iter().map(|t| if t.contains(' ') { format!("[{t}]") } else { t.clone() }).collect();
    format!("<{}>", inner.join(" "))
}

/// Why some bars cannot be written as one part.
#[derive(Debug)]
struct TooLong(usize);

/// A part's pattern over `bars` (with each distinct bar named by `name` when it gives one) and its feel, or None when it is
/// silent there.
fn part_pattern(pt: &PartTrack, bars: &[[Vec<Note>; 16]], name: &mut dyn FnMut(&str, &str) -> Option<String>) -> Result<Option<(String, Feel)>, TooLong> {
    if bars.iter().all(|b| b.iter().all(Vec::is_empty)) {
        return Ok(None);
    }
    let f = feel(pt, bars);
    let texts: Vec<String> = bars.iter().map(|b| bar_text(pt.role, b, f.accent)).collect();
    let p = period(&texts);
    if p > MAX_PERIOD {
        return Err(TooLong(p));
    }
    let terms: Vec<String> = texts[..p].iter().map(|t| if t == "~" { t.clone() } else { name(pt.role, t).unwrap_or_else(|| t.clone()) }).collect();
    Ok(Some((bars_pattern(&terms), f)))
}

fn param_text(params: &[(&'static str, String)], skip: &BTreeSet<String>) -> String {
    params.iter().filter(|(k, _)| !skip.contains(*k)).map(|(k, v)| format!(" {k}={v}")).collect()
}

/// The deck lines of bars `a..b` (0-based, b out) of `roles`, indented by `indent`.
fn deck_lines(t: &Track, a: usize, b: usize, roles: &[&str], indent: &str, name: &mut dyn FnMut(&str, &str) -> Option<String>) -> Result<Vec<String>, (&'static str, TooLong)> {
    let mut out = Vec::new();
    for (deck, deck_roles) in [("A", &DECK_A[..]), ("B", &DECK_B[..])] {
        let mut items = Vec::new();
        for role in deck_roles.iter().filter(|r| roles.contains(r)) {
            let Some(pt) = t.part(role) else { continue };
            match part_pattern(pt, &pt.bars[a..b], name) {
                Ok(Some((pat, f))) => items.push(format!("{role} {pat}{}", param_text(&f.params, &BTreeSet::new()))),
                Ok(None) => {}
                Err(e) => return Err((pt.role, e)),
            }
        }
        if items.is_empty() {
            continue;
        }
        let head = format!("{indent}deck {deck}: ");
        let pad = format!("{indent}{}", " ".repeat("deck A: ".len() - 2));
        for (k, it) in items.iter().enumerate() {
            out.push(if k == 0 { format!("{head}{it}") } else { format!("{pad}| {it}") });
        }
    }
    Ok(out)
}

fn section_name(raw: &str) -> String {
    let mut s: String = raw.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    s = s.trim_matches('_').to_string();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s = format!("s_{s}");
    }
    if crate::parse::reserved_as(&s, "section").is_some() {
        s.push_str("_part");
    }
    s
}

// ---------------------------------------------------------------- expansion

/// What a reference stands for where it stands.
enum Form {
    /// The whole pattern of a part of this role; the params the line already says.
    Part(&'static str, BTreeSet<String>),
    /// The value of a `let`: a pattern with no params.
    Let,
    /// `section NAME from ...`: from this token.
    Section(String, usize),
    /// `arrange from ...`: from this token.
    Arrange(usize),
}

/// A program with its seed references written out.
#[derive(Clone, Debug)]
pub struct Expansion {
    pub text: String,
    /// (the place in the original, the place of what replaced it in `text`).
    pub edits: Vec<(Span, Span)>,
    /// The references, as written in canonical form.
    pub refs: Vec<String>,
    pub warnings: Vec<Diagnostic>,
}

impl Expansion {
    /// A place in [`Expansion::text`] as a place in the original: inside what a reference became, the reference.
    pub fn back(&self, sp: Span) -> Span {
        let map = |at: usize, end: bool| -> usize {
            let mut delta: isize = 0;
            for (o, n) in &self.edits {
                if at < n.start || (end && at == n.start) {
                    break;
                }
                if at < n.end || (end && at == n.end) {
                    return if end { o.end } else { o.start };
                }
                delta = n.end as isize - o.end as isize;
            }
            (at as isize - delta).max(0) as usize
        };
        let s = map(sp.start, false);
        Span::new(s, map(sp.end, true).max(s))
    }
}

/// The `#! seeds:` version a program asks for, if it says one: an error when it is not one this BeatScript has.
fn version_check(src: &str, comments: &[Span]) -> Option<Diagnostic> {
    for c in comments {
        let line = c.text(src).trim();
        let Some(rest) = line.strip_prefix("#!") else { continue };
        let Some(v) = rest.trim().strip_prefix("seeds:") else { continue };
        let v = v.trim();
        if v != VERSION {
            return Some(Diagnostic::error(*c, format!("this BeatScript has seeds {VERSION}, not `{v}`")).help(format!("#! seeds: {VERSION} (a seed always means the same music in one version)")));
        }
    }
    None
}

/// Does the program say this setting (`tempo`, `key`, `swing`) at the head of a line?
fn says(toks: &[Token], word: &str) -> bool {
    toks.iter().enumerate().any(|(i, t)| matches!(&t.tok, Tok::Word(w) if w == word) && (i == 0 || toks[i - 1].tok == Tok::Newline) && !matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Colon)))
}

/// `src` with every seed reference written out as BeatScript, or None when it has none. Errors are at the references.
pub fn expand(src: &str) -> Option<Result<Expansion, Vec<Diagnostic>>> {
    let norm = crate::lex::normalise_input(src);
    let src: &str = &norm;
    let lexed = lex(src);
    let toks = &lexed.tokens;
    let rs = runs(toks);
    if rs.is_empty() {
        return None;
    }
    let mut errors: Vec<Diagnostic> = Vec::new();
    if let Some(d) = version_check(src, &lexed.comments) {
        errors.push(d);
    }
    let known = genres();
    let mut warnings = Vec::new();
    let mut pieces: Vec<(Span, String, String)> = Vec::new();
    let has = |w: &str| says(toks, w);
    let mut whole_seen = false;
    for run in &rs {
        let s = match &run.r {
            Ok(s) => s.clone(),
            Err(d) => {
                errors.push(d.clone());
                continue;
            }
        };
        if !known.contains(&s.genre) {
            let mut d = Diagnostic::error(s.genre_span, format!("`{}` is not a genre of the generator", s.genre)).found(format!("`{}`", s.genre));
            d = match nearest(&s.genre, known.iter().map(String::as_str)) {
                Some(n) => d.help(format!("did you mean `{n}`? The genres: {}", known.join(", "))),
                None => d.help(format!("the genres: {}", known.join(", "))),
            };
            errors.push(d);
            continue;
        }
        let form = match form_of(toks, run) {
            Ok(f) => f,
            Err(d) => {
                errors.push(d);
                continue;
            }
        };
        let t = track(&s.genre, s.n, s.structure());
        let (a, b) = match s.bars {
            Some((a, b)) if b > t.bars => {
                errors.push(Diagnostic::error(s.bars_span, format!("bars {a}..{b}: {} has {} bars", s.text().split(')').next().map(|x| format!("{x})")).unwrap_or_default(), t.bars)).help(format!("[1..{}] is the whole track", t.bars)));
                continue;
            }
            Some((a, b)) => (a as usize - 1, b as usize),
            None => (0, t.bars as usize),
        };
        let role = match (&form, &s.part) {
            (_, Some(p)) => ROLES.iter().copied().find(|r| r == p),
            (Form::Part(r, _), None) => Some(*r),
            _ => None,
        };
        if let (Some(r), true) = (role, s.part.is_some() || matches!(form, Form::Part(..))) {
            if t.part(r).is_none() {
                let at = if s.part.is_some() { s.part_span } else { run.span };
                let have: Vec<&str> = t.parts.iter().map(|p| p.role).collect();
                warnings.push(Diagnostic::warning(at, format!("{} {} has no {r}: it is silent", s.genre, s.n)).help(format!("its parts: {}", have.join(", "))));
            }
        }
        let refx = s.text();
        let roles: Vec<&str> = match role {
            Some(r) => vec![r],
            None => ROLES.to_vec(),
        };
        let too_long = |r: &str, p: usize| Diagnostic::error(run.span, format!("the {r} of {refx} changes over {p} bars, more than a part can hold ({MAX_PERIOD})")).help("pick fewer bars: [1..16]");
        match form {
            Form::Part(..) | Form::Let => {
                let (said, is_let) = match &form {
                    Form::Part(_, s) => (s.clone(), false),
                    _ => (BTreeSet::new(), true),
                };
                let r = role.unwrap_or("bass");
                let text = match t.part(r) {
                    None => "~".to_string(),
                    Some(pt) => match part_pattern(pt, &pt.bars[a..b], &mut |_, _| None) {
                        Ok(None) => "~".to_string(),
                        Ok(Some((pat, f))) => {
                            if is_let {
                                pat
                            } else {
                                format!("{pat}{}", param_text(&f.params, &said))
                            }
                        }
                        Err(TooLong(p)) => {
                            errors.push(too_long(r, p));
                            continue;
                        }
                    },
                };
                pieces.push((run.span, text, refx));
            }
            Form::Section(name, from) => {
                let start = toks[from].span;
                match deck_lines(&t, a, b, &roles, "  ", &mut |_, _| None) {
                    Ok(lines) => {
                        let mut out = format!("# from {refx}\nsection {name} {} bars {{\n", b - a);
                        for l in lines {
                            out.push_str(&l);
                            out.push('\n');
                        }
                        out.push('}');
                        pieces.push((start.to(run.span), out, refx));
                    }
                    Err((r, TooLong(p))) => errors.push(too_long(r, p)),
                }
            }
            Form::Arrange(from) => {
                if whole_seen {
                    errors.push(Diagnostic::error(toks[from].span.to(run.span), "a program has one `arrange from seed(...)`").help("give the other one its sections: `section drop from seed(techno, 42)[17..28]`"));
                    continue;
                }
                whole_seen = true;
                let start = toks[from].span;
                match whole(&t, a, b, &roles, &refx, !has("tempo"), !has("key"), !has("swing")) {
                    Ok(text) => pieces.push((start.to(run.span), text, refx)),
                    Err((r, TooLong(p))) => errors.push(too_long(r, p)),
                }
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    let mut text = String::with_capacity(src.len() * 4);
    let mut edits = Vec::new();
    let mut refs = Vec::new();
    let mut at = 0;
    for (sp, rep, r) in pieces {
        text.push_str(&src[at..sp.start]);
        let s = text.len();
        text.push_str(&rep);
        edits.push((sp, Span::new(s, text.len())));
        refs.push(r);
        at = sp.end;
    }
    text.push_str(&src[at..]);
    Some(Ok(Expansion { text, edits, refs, warnings }))
}

/// Where a reference stands decides what it is written out as.
fn form_of(toks: &[Token], run: &Run) -> Result<Form, Diagnostic> {
    let line_start = toks[..run.start].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |i| i + 1);
    let head: Vec<&Tok> = toks[line_start..run.start].iter().map(|t| &t.tok).collect();
    let word = |t: &Tok, w: &str| matches!(t, Tok::Word(x) if x == w);
    match head.as_slice() {
        [s, Tok::Word(name), f] if word(s, "section") && word(f, "from") => return Ok(Form::Section(name.clone(), line_start)),
        [a, f] if word(a, "arrange") && word(f, "from") => return Ok(Form::Arrange(line_start)),
        _ => {}
    }
    let prev = run.start.checked_sub(1).map(|i| &toks[i].tok);
    if let Some(Tok::Word(w)) = prev {
        if let Some(role) = ROLES.iter().copied().find(|r| r == w) {
            let mut said = BTreeSet::new();
            let mut j = run.end;
            while j < toks.len() && !matches!(toks[j].tok, Tok::Pipe | Tok::Newline | Tok::Eof | Tok::RBrace | Tok::Comma | Tok::RParen) {
                if let (Tok::Word(k), Some(Tok::Eq)) = (&toks[j].tok, toks.get(j + 1).map(|t| &t.tok)) {
                    said.insert(k.clone());
                }
                j += 1;
            }
            return Ok(Form::Part(role, said));
        }
    }
    if let (Some(Tok::Eq), Some(Tok::Word(_)), Some(l)) = (prev, run.start.checked_sub(2).map(|i| &toks[i].tok), run.start.checked_sub(3).map(|i| &toks[i].tok)) {
        if word(l, "let") {
            return match &run.r {
                Ok(s) if s.part.is_none() => Err(Diagnostic::error(run.span, "a `let` of a seed names one part: say which").help(format!("let riff = {}", SeedRef { part: Some("bass".into()), ..s.clone() }.text()))),
                _ => Ok(Form::Let),
            };
        }
    }
    if matches!(prev, Some(Tok::Num { .. })) && run.start >= 3 && toks[run.start - 2].tok == Tok::Slash {
        return Err(Diagnostic::error(run.span, "a seed's part plays the track's own bars of sixteen steps: it takes no cycle of its own").help("drop the `/N`"));
    }
    Err(Diagnostic::error(run.span, "seed(...) goes where a pattern goes, or makes sections")
        .help("bass seed(techno, 42)[1..8]   let riff = seed(techno, 42).bass[1..8]   section intro from seed(techno, 42)[1..8]   arrange from seed(techno, 42)"))
}

/// `arrange from seed(...)`: the settings it needs, a `let` for each bar of each part, the sections and the arrange.
#[allow(clippy::too_many_arguments)]
fn whole(t: &Track, a: usize, b: usize, roles: &[&str], refx: &str, tempo: bool, key: bool, swing: bool) -> Result<String, (&'static str, TooLong)> {
    let mut out = format!("# from {refx}: \"{}\", the beats generator, seeds {VERSION}\n", t.name.replace('"', "'"));
    if tempo {
        out.push_str(&format!("tempo {}\n", crate::print::num(t.tempo)));
    }
    if key {
        out.push_str(&format!("key {}\n", t.key));
    }
    if swing && t.swing > 0.0 && t.swing <= 1.0 {
        out.push_str(&format!("swing {}\n", crate::print::num(t.swing)));
    }
    // the bars of each part, named in the order they first play: kick_a, kick_b, ...
    let mut names: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut lets: Vec<String> = Vec::new();
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    let mut namer = |role: &str, text: &str| -> Option<String> {
        let k = (role.to_string(), text.to_string());
        if let Some(n) = names.get(&k) {
            return Some(n.clone());
        }
        let c = count.entry(role.to_string()).or_default();
        let n = format!("{role}_{}", letters(*c));
        *c += 1;
        lets.push(format!("let {n} = {text}"));
        names.insert(k, n.clone());
        Some(n)
    };
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for (raw, s0, len) in &t.sections {
        let (s0, s1) = ((*s0 as usize).max(a), ((*s0 + *len) as usize).min(b));
        if s0 >= s1 {
            continue;
        }
        // a section longer than a part can change over is cut in pieces of 64 bars
        let mut k = s0;
        while k < s1 {
            let e = (k + MAX_PERIOD).min(s1);
            let body = deck_lines(t, k, e, roles, "  ", &mut namer)?;
            let header = e - k;
            let base = section_name(raw);
            let same = sections.iter().find(|(n, bl)| (n == &base || n.starts_with(&format!("{base}_"))) && bl.first().is_some_and(|h| h == &header.to_string()) && bl[1..] == body[..]);
            let name = match same {
                Some((n, _)) => n.clone(),
                None => {
                    let taken = |n: &str| sections.iter().any(|(m, _)| m == n);
                    let mut n = base.clone();
                    let mut i = 2;
                    while taken(&n) {
                        n = format!("{base}_{i}");
                        i += 1;
                    }
                    let mut bl = vec![header.to_string()];
                    bl.extend(body);
                    sections.push((n.clone(), bl));
                    n
                }
            };
            order.push(name);
            k = e;
        }
    }
    for l in &lets {
        out.push_str(l);
        out.push('\n');
    }
    for (n, bl) in &sections {
        out.push_str(&format!("section {n} {} bars {{\n", bl[0]));
        for l in &bl[1..] {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("}\n");
    }
    out.push_str(&format!("arrange {}", order.join(" ")));
    if t.looped && a == 0 && b == t.bars as usize {
        out.push_str("\nloop");
    }
    Ok(out)
}

/// What a `seed(...)` reference resolves to: the piece it names as a program of its own (the track's tempo and key, its lets,
/// sections and arrange), and that program's CID, the pin. Every place the same piece stands (a part line, a `let`) resolves
/// to the same CID, and seeds v1 always resolve as they did (`tests/seed.rs` pins the writer). `role` is a part line's own role
/// when the reference names no part.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub call: String,
    /// The generated track's name.
    pub name: String,
    /// The generator's project hash (beats' `modelCID`).
    pub model: String,
    pub text: String,
    pub cid: String,
}

pub fn resolve(word: &str, role: Option<&str>) -> Result<Resolved, Diagnostic> {
    let toks = lex(word).tokens;
    let Some(run) = runs(&toks).into_iter().next() else { return Err(Diagnostic::error(Span::new(0, word.len()), format!("`{word}` is no seed reference"))) };
    let mut s = run.r?;
    if !GENRES.contains(&s.genre.as_str()) {
        return Err(Diagnostic::error(s.genre_span, format!("`{}` is not a genre of the generator", s.genre)));
    }
    if s.part.is_none() {
        s.part = role.filter(|r| ROLES.contains(r)).map(str::to_string);
    }
    let t = track(&s.genre, s.n, s.structure());
    let (a, b) = match s.bars {
        Some((_, b)) if b > t.bars => return Err(Diagnostic::error(s.bars_span, format!("the track has {} bars", t.bars))),
        Some((a, b)) => (a as usize - 1, b as usize),
        None => (0, t.bars as usize),
    };
    let roles: Vec<&str> = match &s.part {
        Some(p) => ROLES.iter().copied().filter(|r| r == p).collect(),
        None => ROLES.to_vec(),
    };
    let call = s.text();
    let text = whole(&t, a, b, &roles, &call, true, true, true).map_err(|(r, TooLong(p))| Diagnostic::error(s.span, format!("the {r} of {call} changes over {p} bars, more than a part can hold ({MAX_PERIOD})")))?;
    let cid = crate::compile::program_cid(&crate::print::canonical(&crate::parse::parse(&text).program));
    Ok(Resolved { call, name: t.name.clone(), model: t.model_sha.clone(), text, cid })
}

/// 0 is `a`, 25 `z`, 26 `aa`.
fn letters(mut n: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'a' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}
