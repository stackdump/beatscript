//! `beatscript/numberAt`: what the literal at a position is, so an editor can scrub it (drag, wheel, arrow keys) without ever
//! writing a program the checker refuses. For a number: its span, what it sets (`kind`), the valid range, the step, the unit,
//! and for one end of a range (`a..b`) or a ramp (`a -> b`) the other end. For a note (`c4`, `f#3`) and a Roman numeral (`VI`)
//! the ordered `choices` it can be moved through: the notes of the key in force (or every semitone when the program has no
//! key), the numerals of the key's scale.
//!
//! The limits are one table ([`LIMITS`]) of what the checker accepts, by the place a number stands in. They are not read
//! off the checker at the keystroke (a check can take seconds); `tests/number.rs` proves each row against it instead: the
//! two ends check clean and one step past each is refused. A `do` line's param takes its type and range from the registry
//! ([`djlive::env::ParamSpec`], the schema's `minimum`/`maximum`).
//!
//! `exact` is false when the literal is one operand of arithmetic (`tempo 120 + 6`) or a `$` is near it: the limits are of the
//! whole value, so an editor clamps the literal and lets the check have the last word.

use djengine::json::{JsonObject, JsonValue};
use djlive::env::Registry;
use djlive::lex::{Tok, Token};
use djlive::music::Key;
use djlive::Span;

use crate::analysis::Analysis;
use crate::json::{arr, s, span_range};
use crate::text::Encoding;

/// The limits of a number by where it stands: (kind, label, min, max, step, unit, integer, log scale).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub kind: &'static str,
    pub label: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: f64,
    pub unit: &'static str,
    pub integer: bool,
    /// A cutoff: an editor moves it by a ratio, not a sum.
    pub log: bool,
}

const fn lim(kind: &'static str, label: &'static str, min: f64, max: f64, step: f64, unit: &'static str, integer: bool) -> Limit {
    Limit {
        kind,
        label,
        min: Some(min),
        max: Some(max),
        step,
        unit,
        integer,
        log: false,
    }
}

const fn from(kind: &'static str, label: &'static str, min: f64, step: f64, unit: &'static str, integer: bool) -> Limit {
    Limit {
        kind,
        label,
        min: Some(min),
        max: None,
        step,
        unit,
        integer,
        log: false,
    }
}

/// Every number the language bounds, by its place. Proved against the checker by `tests/number.rs`.
pub const LIMITS: &[Limit] = &[
    lim("tempo", "tempo (BPM)", 40.0, 300.0, 1.0, "bpm", false),
    lim("swing", "swing", 0.0, 1.0, 0.01, "", false),
    lim("vel", "velocity", 1.0, 127.0, 1.0, "", true),
    lim("oct", "octave", -1.0, 8.0, 1.0, "", true),
    lim("len", "note length (steps)", 1.0, 16.0, 1.0, "steps", true),
    lim("glide", "glide (ms)", 1.0, 2000.0, 1.0, "ms", true),
    lim("decay", "note decay (ms)", 10.0, 10000.0, 10.0, "ms", true),
    lim("pitch-drop", "pitch drop (semitones)", 1.0, 24.0, 1.0, "semitones", true),
    lim("ratchet", "ratchet (strikes in the step)", 2.0, 8.0, 1.0, "", true),
    lim("seed", "seed", 0.0, 4_294_967_295.0, 1.0, "", true),
    lim("times", "times in its slot (*n)", 1.0, 64.0, 1.0, "", true),
    lim("chance", "chance", 0.0, 1.0, 0.01, "", false),
    lim("cycle", "cycle (sixteenths)", 1.0, 64.0, 1.0, "steps", true),
    lim("hits", "euclid hits (k)", 0.0, 64.0, 1.0, "", true),
    lim("steps", "euclid steps (n)", 1.0, 64.0, 1.0, "steps", true),
    lim("rotation", "euclid rotation", 0.0, 63.0, 1.0, "steps", true),
    lim("density", "density", 0.0, 1.0, 0.01, "", false),
    lim("amount", "amount", 0.0, 1.0, 0.01, "", false),
    lim("level", "level", 0.0, 1.0, 0.01, "", false),
    lim("eq", "EQ", 0.0, 1.0, 0.01, "", false),
    lim("crossfader", "crossfader", 0.0, 1.0, 0.01, "", false),
    lim("mix", "effect amount", 0.0, 1.0, 0.01, "", false),
    lim("resonance", "resonance", 0.0, 1.0, 0.01, "", false),
    Limit {
        kind: "cutoff-lp",
        label: "low-pass cutoff",
        min: Some(60.0),
        max: Some(20000.0),
        step: 1.0,
        unit: "hz",
        integer: true,
        log: true,
    },
    Limit {
        kind: "cutoff-hp",
        label: "high-pass cutoff",
        min: Some(20.0),
        max: Some(8000.0),
        step: 1.0,
        unit: "hz",
        integer: true,
        log: true,
    },
    lim("repeat", "repeat count", 1.0, 64.0, 1.0, "", true),
    lim("section-bars", "section length", 1.0, 256.0, 1.0, "bars", true),
    from("bars", "bars", 1.0, 1.0, "bars", true),
    from("bar", "bar", 0.0, 1.0, "bar", true),
    lim("beat", "beat", 0.0, 3.0, 1.0, "beat", true),
    lim("degree", "scale degree", 1.0, 64.0, 1.0, "", true),
    lim("rotate", "evolve rotate", -15.0, 15.0, 1.0, "steps", true),
    lim("transpose", "semitones", -48.0, 48.0, 1.0, "semitones", true),
    lim("octaves", "octaves", -4.0, 4.0, 1.0, "octaves", true),
    lim("share", "share", 0.0, 1.0, 0.01, "", false),
    from("db", "depth (dB)", 0.0, 0.5, "db", false),
    from("ms", "time (ms)", 0.0, 1.0, "ms", true),
    lim("mut-int", "mutation amount", -64.0, 64.0, 1.0, "", true),
    lim("passes", "passes (loop N)", 1.0, 256.0, 1.0, "", true),
    lim("every", "every Nth pass (~N, @N)", 1.0, 256.0, 1.0, "", true),
];

pub fn limit(kind: &str) -> Option<&'static Limit> {
    LIMITS.iter().find(|l| l.kind == kind)
}

/// A number's place with no bound of its own.
const FREE: Limit = Limit {
    kind: "number",
    label: "number",
    min: None,
    max: None,
    step: 1.0,
    unit: "",
    integer: false,
    log: false,
};

/// What the literal at a byte offset is.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberAt {
    /// The literal, a `-` before it included.
    pub span: Span,
    pub text: String,
    pub value: f64,
    pub limit: Limit,
    /// The label to show (a `do` param says its own name).
    pub label: String,
    /// The step for the literal: the limit's, or what the written decimals suggest for a free number.
    pub step: f64,
    pub exact: bool,
    pub range: Option<RangeEnd>,
    /// For a note or a numeral: the values it can be moved through, in order, and where it is among them.
    pub choices: Vec<String>,
    pub index: Option<usize>,
}

/// One end of `a..b` (a range, `ordered`: lo <= hi) or `a -> b` (a ramp: either way, but the ends do not cross).
#[derive(Clone, Debug, PartialEq)]
pub struct RangeEnd {
    pub op: &'static str,
    /// `start` or `end`: which end the asked literal is.
    pub role: &'static str,
    pub start: Span,
    pub end: Span,
    pub start_value: f64,
    pub end_value: f64,
}

fn is_num(t: &Tok) -> bool {
    matches!(t, Tok::Num { .. })
}

fn word(t: Option<&Token>) -> Option<&str> {
    match t.map(|t| &t.tok) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    }
}

fn is_op(t: &Tok) -> bool {
    matches!(t, Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent)
}

/// The decimals a step has (0.01: 2).
pub fn decimals(step: f64) -> usize {
    let mut d = 0;
    let mut x = step;
    while d < 6 && (x - x.round()).abs() > 1e-9 {
        x *= 10.0;
        d += 1;
    }
    d
}

/// The canonical text of a number (what `beatscript fmt` prints: the shortest plain decimal, no marks).
pub fn spell(v: f64, decimals: usize) -> String {
    let r = (v * 10f64.powi(decimals as i32)).round() / 10f64.powi(decimals as i32);
    djlive::print::num(if r == 0.0 { 0.0 } else { r })
}

/// The tokens of the statement line holding token `i`: from just after the newline before it to the newline after it.
fn line_bounds(toks: &[Token], i: usize) -> (usize, usize) {
    let mut a = i;
    while a > 0 && !matches!(toks[a - 1].tok, Tok::Newline) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < toks.len() && !matches!(toks[b + 1].tok, Tok::Newline | Tok::Eof) {
        b += 1;
    }
    (a, b)
}

/// The words a statement starts with, after its time word and target heads (`at bar 4: deck A: filter lp 2k` -> `filter`).
fn body_start(toks: &[Token], a: usize, i: usize) -> usize {
    let mut start = a;
    for k in a..i {
        if matches!(toks[k].tok, Tok::Colon) {
            start = k + 1;
        }
    }
    start
}

/// The innermost unclosed `name(` before token `i` on its line: (index of the name, argument index).
fn call_of(toks: &[Token], a: usize, i: usize) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut k = i;
    while k > a {
        k -= 1;
        match toks[k].tok {
            Tok::RParen | Tok::RBrack => depth += 1,
            Tok::LBrack if depth > 0 => depth -= 1,
            Tok::LBrack => return None,
            Tok::LParen if depth > 0 => depth -= 1,
            Tok::LParen => {
                return match k.checked_sub(1).and_then(|p| word(toks.get(p))) {
                    Some(_) => Some((k - 1, commas)),
                    None => None, // a parenthesis of arithmetic: keep looking outward
                };
            }
            Tok::Comma if depth == 0 => commas += 1,
            _ => {}
        }
    }
    None
}

/// The key in force at byte `at`: the last `key` statement on a line before it (a section's own, or the program's).
fn key_at(a: &Analysis, at: usize) -> Option<Key> {
    let toks = &a.tokens;
    let mut key = None;
    for (k, t) in toks.iter().enumerate() {
        if t.span.start >= at {
            break;
        }
        let line_start = k == 0 || matches!(toks[k - 1].tok, Tok::Newline);
        if line_start && word(Some(t)) == Some("key") {
            if let Some(w) = word(toks.get(k + 1)) {
                key = Key::parse(w).or(key);
            }
        }
    }
    key
}

const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

/// The notes a note word can be moved through: the key's scale (every semitone with no key), octave -1 to 9 when it has an
/// octave, one octave of pitch classes when it has none. Spelled as `beatscript fmt` spells them (sharps).
fn note_choices(w: &str, key: Option<Key>) -> Option<(Vec<String>, usize)> {
    let (pc, oct) = djlive::music::parse_note(w)?;
    let in_key = |p: u8| match key {
        Some(k) => k.intervals().iter().any(|iv| (i64::from(k.tonic) + iv).rem_euclid(12) as u8 == p),
        None => true,
    };
    let mut out = Vec::new();
    let mut index = None;
    let octs: Vec<Option<i32>> = match oct {
        Some(_) => (-1..=9).map(Some).collect(),
        None => vec![None],
    };
    for o in octs {
        for p in 0..12u8 {
            let here = p == pc && o == oct;
            if !in_key(p) && !here {
                continue;
            }
            if here {
                index = Some(out.len());
            }
            out.push(match o {
                Some(o) => format!("{}{o}", SHARP[usize::from(p)]),
                None => SHARP[usize::from(p)].to_string(),
            });
        }
    }
    Some((out, index?))
}

const NUMERALS: [&str; 7] = ["i", "ii", "iii", "iv", "v", "vi", "vii"];

/// The numerals of the key's scale, I to VII, each in the case of its triad in the key (upper: a major third), with the
/// word's own quality kept when it does not fix the case.
fn roman_choices(w: &str, key: Option<Key>) -> Option<(Vec<String>, usize)> {
    let r = djlive::scale::parse_roman(w).ok()?;
    let key = key.unwrap_or(Key::C_MAJOR);
    let iv = key.intervals();
    if iv.len() != 7 {
        return None; // a pentatonic or blues key has no seven numerals
    }
    // the word's tail after its numeral: its quality and inversion, kept when the case is the scale's to say
    let body = w.trim_start_matches(['b', '#']);
    let run = body.chars().take_while(|c| matches!(c, 'i' | 'v' | 'I' | 'V')).count();
    let tail = &body[run..];
    let keep_tail = r.quality.forced_case().is_none() && !tail.contains('/');
    let mut out = Vec::new();
    for d in 0..7usize {
        let third = (iv[(d + 2) % 7] + if d + 2 >= 7 { 12 } else { 0 }) - iv[d];
        let n = if third == 4 { NUMERALS[d].to_ascii_uppercase() } else { NUMERALS[d].to_string() };
        let cand = if keep_tail { format!("{n}{tail}") } else { n };
        out.push(if djlive::scale::parse_roman(&cand).is_ok() {
            djlive::scale::canonical_word(&cand)
        } else {
            djlive::scale::canonical_word(NUMERALS[d])
        });
    }
    let index = (r.acc == 0).then_some(usize::from(r.degree) - 1)?;
    // the word as written stays where it is, even if its case is not the scale's (a borrowed chord)
    out[index] = w.to_string();
    Some((out, index))
}

/// The limit of a `do` line's param, from the registry.
fn do_param_limit(toks: &[Token], start: usize, name: &str, reg: &dyn Registry) -> Option<(Limit, String)> {
    let words: Vec<&str> = toks[start..].iter().map_while(|t| word(Some(t))).collect();
    let noun_word = match words.first() {
        Some(&"do") => words.get(1)?,
        Some(w) => w,
        None => return None,
    };
    let noun = reg.noun(djlive::parse::sugar_noun(noun_word).unwrap_or(noun_word))?;
    let verb = words.iter().skip(1).find_map(|v| noun.action(v));
    let p = verb.and_then(|a| a.params.iter().find(|p| p.name == name)).or_else(|| noun.param(name))?;
    let integer = p.types.iter().any(|t| t == "integer") && !p.types.iter().any(|t| t == "number");
    let numeric = integer || p.types.iter().any(|t| t == "number") || p.types.is_empty();
    if !numeric {
        return None;
    }
    let l = Limit {
        kind: "param",
        label: "param",
        min: p.minimum,
        max: p.maximum,
        step: if integer { 1.0 } else { 0.01 },
        unit: "",
        integer,
        log: false,
    };
    Some((l, format!("{} {name}", noun.name)))
}

/// The limit of the number at token `i` (a `Num`), and the label for it.
fn classify(a: &Analysis, i: usize, reg: &dyn Registry) -> (Limit, String) {
    let toks = &a.tokens;
    let (la, _lb) = line_bounds(toks, i);
    let mut j = i; // the token before the number, past a unary minus
    if j > la && matches!(toks[j - 1].tok, Tok::Minus) {
        j -= 1;
    }
    let prev = |n: usize| if j >= la + n { toks.get(j - n) } else { None };
    let next = toks.get(i + 1);
    let named = |l: &'static str| -> (Limit, String) {
        let x = *limit(l).expect("a row of LIMITS");
        (x, x.label.to_string())
    };
    // a range's or ramp's far end: classify by its near end
    if let Some(p) = prev(1) {
        if matches!(p.tok, Tok::DotDot | Tok::Arrow) {
            if let Some(k) = (la..j - 1).rev().find(|k| is_num(&toks[*k].tok)) {
                return classify(a, k, reg);
            }
        }
    }
    // the unit glued after it says what it is
    if let Some(Tok::Unit(u)) = next.map(|t| &t.tok) {
        match u.as_str() {
            "db" => return named("db"),
            "ms" => return named("ms"),
            "hz" => {}
            _ => return (FREE, format!("time ({u})")),
        }
    }
    // a length or a place: `4 bars`, `bar 16`, `beat 3`
    if let Some(w) = word(next) {
        if matches!(w, "bar" | "bars" | "beat" | "beats") {
            return match word(prev(2)).filter(|_| word(prev(1)).is_some()) {
                // `section intro 8 bars`
                Some("section") => named("section-bars"),
                _ => {
                    if word(prev(1)) == Some("repeat") {
                        named("repeat")
                    } else {
                        named("bars")
                    }
                }
            };
        }
    }
    // an arrange mutation's argument (`rot 2`, `thin 0.1`, `morph b 0.5`): its operator's argument kinds say
    if crate::hover::on_mutation_line(toks, i) {
        let mut k = j;
        let mut before = 0usize;
        while k > la {
            k -= 1;
            match &toks[k].tok {
                Tok::Word(w) if djlive::mutate::operator_args(w).is_some() => {
                    let kinds = djlive::mutate::operator_args(w).unwrap_or_default();
                    return match kinds.get(before) {
                        Some(&"int") => (*limit("mut-int").unwrap(), format!("{w} (a whole number)")),
                        Some(&"share") => (*limit("share").unwrap(), format!("{w} (a share)")),
                        _ => (FREE, w.to_string()),
                    };
                }
                Tok::Num { .. } | Tok::Word(_) => before += 1,
                Tok::Minus | Tok::Plus => {}
                // `~3 acid rev`: the count right after the mark
                Tok::Tilde | Tok::At if before == 0 => return named("every"),
                _ => break,
            }
        }
        return (FREE, "number".to_string());
    }
    match word(prev(1)) {
        Some("loop") => return named("passes"),
        Some("bar") if word(prev(2)) != Some("over") => return named("bar"),
        Some("beat") => return named("beat"),
        Some("tempo") => return named("tempo"),
        Some("swing") => return named("swing"),
        Some("repeat") => return named("repeat"),
        Some("lp") if word(prev(2)) == Some("filter") => return named("cutoff-lp"),
        Some("hp") if word(prev(2)) == Some("filter") => return named("cutoff-hp"),
        Some("delay" | "reverb" | "crush" | "drive") => return named("mix"),
        Some("resonance") => return named("resonance"),
        Some("level") => return named("level"),
        Some("low" | "mid" | "high") if word(prev(2)) != Some("eq") => return named("eq"),
        Some("crossfader") => return named("crossfader"),
        Some("rotate") => return named("rotate"),
        Some("transpose") => return named("transpose"),
        Some("octave") => return named("octaves"),
        Some("thin") => return named("share"),
        _ => {}
    }
    // `x:3`, `c3!:4`, `[x x]:2`: a ratchet, the number glued to a colon glued to a step (not `dnb:2602`, a spec word)
    if let (Some(c), Some(s)) = (prev(1), prev(2)) {
        let step = match &s.tok {
            Tok::Word(w) => w == "x" || djlive::music::parse_note(w).is_some(),
            t => is_num(t) || matches!(t, Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp),
        };
        if c.tok == Tok::Colon && j == i && step && s.span.end == c.span.start && c.span.end == toks[i].span.start {
            return named("ratchet");
        }
    }
    match prev(1).map(|t| &t.tok) {
        Some(Tok::Star) if !prev(2).is_some_and(|t| is_num(&t.tok) || matches!(t.tok, Tok::Dollar(_) | Tok::RParen)) => return named("times"),
        Some(Tok::Star) if prev(2).is_some_and(|t| matches!(t.tok, Tok::Word(_) | Tok::RBrack | Tok::Gt | Tok::Tilde)) => return named("times"),
        Some(Tok::Question) => return named("chance"),
        Some(Tok::Slash) if word(prev(2)).is_some_and(|w| djlive::parse::ROLES.contains(&w)) => return named("cycle"),
        Some(Tok::Eq) => {
            let name = word(prev(2)).unwrap_or("");
            return match name {
                "vel" => named("vel"),
                "oct" => named("oct"),
                "len" => named("len"),
                "glide" => named("glide"),
                "decay" => named("decay"),
                "pitch-drop" => named("pitch-drop"),
                "seed" => named("seed"),
                "bars" => named("bars"),
                "depth" | "center" => {
                    // an lfo's: the knob of what it moves; take the control word at the start of the body
                    let st = body_start(toks, la, i);
                    let mut k = st;
                    while k < i {
                        let (l, label) = match word(toks.get(k)) {
                            Some("lp") if word(toks.get(k.wrapping_sub(1))) == Some("filter") => named("cutoff-lp"),
                            Some("hp") if word(toks.get(k.wrapping_sub(1))) == Some("filter") => named("cutoff-hp"),
                            Some("level") => named("level"),
                            Some("delay" | "reverb" | "crush" | "drive") => named("mix"),
                            Some("resonance") => named("resonance"),
                            _ => {
                                k += 1;
                                continue;
                            }
                        };
                        // a depth is a distance, from 0
                        let l = if name == "depth" { Limit { min: Some(0.0), ..l } } else { l };
                        return (l, format!("lfo {name} ({label})"));
                    }
                    (FREE, format!("lfo {name}"))
                }
                _ => {
                    let st = body_start(toks, la, i);
                    match do_param_limit(toks, st, name, reg) {
                        Some(x) => x,
                        None => (FREE, format!("{name}=")),
                    }
                }
            };
        }
        _ => {}
    }
    // an argument of a call
    if let Some((c, argi)) = call_of(toks, la, i) {
        let w = word(toks.get(c)).unwrap_or("");
        let row = match (w, argi) {
            ("e" | "euclid", 0) => Some("hits"),
            ("e" | "euclid", 1) => Some("steps"),
            ("e" | "euclid", 2) => Some("rotation"),
            ("markov", _) if !matches!(toks.get(c + 2).map(|t| &t.tok), Some(Tok::Num { .. })) && argi == 1 => Some("density"),
            ("markov", 0) => Some("density"),
            ("markov", 2) => Some("seed"),
            ("maybe", 1) => Some("chance"),
            ("maybe" | "degrade" | "humanize", 2) => Some("seed"),
            ("degrade", 1) => Some("share"),
            ("transpose", 1) => Some("transpose"),
            ("octave", 1) => Some("octaves"),
            ("arp", 2) => Some("steps"),
            ("chord", 1) => Some("oct"),
            ("knob" | "pad", 0) => None,
            _ => None,
        };
        if let Some(r) = row {
            return named(r);
        }
        if w == "deg" {
            return named("degree");
        }
        return (FREE, format!("{w}() argument {}", argi + 1));
    }
    // a step of a tonal part's pattern: a scale degree
    let st = body_start(toks, la, i);
    let mut k = i;
    let mut role = None;
    while k > st {
        k -= 1;
        if matches!(toks[k].tok, Tok::Pipe) {
            role = word(toks.get(k + 1));
            break;
        }
        if k == st {
            role = word(toks.get(k));
        }
    }
    if i > st && k == st && role.is_none() {
        role = word(toks.get(st));
    }
    if let Some(r) = role {
        if matches!(r, "bass" | "melody" | "harmony" | "arp") {
            return named("degree");
        }
    }
    // `section intro 8 bars` with the word on its own line etc.: a free number
    (FREE, "number".to_string())
}

/// Is the literal at token `i` (its sign at `j`) one operand of arithmetic, or near a `$`?
fn inexact(toks: &[Token], j: usize, i: usize) -> bool {
    let before = j.checked_sub(1).map(|k| &toks[k].tok);
    let after = toks.get(i + 1).map(|t| &t.tok);
    let op_before = before.is_some_and(|t| {
        matches!(t, Tok::Plus | Tok::Slash | Tok::Percent | Tok::Minus)
            || (matches!(t, Tok::Star) && j >= 2 && matches!(toks[j - 2].tok, Tok::Num { .. } | Tok::Dollar(_) | Tok::RParen))
    });
    let op_after = after.is_some_and(|t| is_op(t) && !matches!(t, Tok::Star));
    // `(60 + $i * 4)..100`: inside parentheses of arithmetic
    let in_parens = before.is_some_and(|t| matches!(t, Tok::LParen)) && j >= 2 && !matches!(toks[j - 2].tok, Tok::Word(_));
    op_before || op_after || in_parens
}

/// Is the token before number `i` a `-` that is its sign (not a subtraction)?
fn unary_minus(a: &Analysis, la: usize, i: usize) -> bool {
    let toks = &a.tokens;
    if i <= la || !matches!(toks[i - 1].tok, Tok::Minus) {
        return false;
    }
    if i - 1 == la {
        return true;
    }
    match &toks[i - 2].tok {
        Tok::Num { .. } | Tok::Dollar(_) | Tok::RParen => false,
        // a scalar `let`'s name is a value: `x - 1`
        Tok::Word(w) => !a.scalar_lets.iter().any(|l| l.name == *w),
        _ => true,
    }
}

/// What the literal at byte `at` is, or None when no number, note or numeral is there.
pub fn number_at(a: &Analysis, at: usize, reg: &dyn Registry) -> Option<NumberAt> {
    if a.in_comment(at) {
        return None;
    }
    let toks = &a.tokens;
    let i = a.token_at(at)?;
    let t = &toks[i];
    match &t.tok {
        Tok::Num { value, .. } => {
            let (la, _) = line_bounds(toks, i);
            let neg = unary_minus(a, la, i);
            let j = if neg { i - 1 } else { i };
            let span = Span::new(toks[j].span.start, t.span.end);
            let value = if neg { -value } else { *value };
            let (limit, label) = classify(a, i, reg);
            let exact = !inexact(toks, j, i);
            let step = if limit.kind == "number" || limit.kind == "param" && !limit.integer {
                // a free number moves by its written last decimal (0.25 by 0.01, 120 by 1)
                let text = t.span.text(&a.text);
                let d = text.split_once('.').map_or(0, |(_, f)| f.trim_end_matches(|c: char| !c.is_ascii_digit()).len());
                if text.ends_with(['k', '%']) {
                    limit.step
                } else {
                    10f64.powi(-(d.min(6) as i32))
                }
            } else {
                limit.step
            };
            let range = range_of(a, la, j, i);
            Some(NumberAt {
                span,
                text: span.text(&a.text).to_string(),
                value,
                limit,
                label,
                step,
                exact,
                range,
                choices: Vec::new(),
                index: None,
            })
        }
        Tok::Word(w) => {
            let key = key_at(a, t.span.start);
            let (la, _) = line_bounds(toks, i);
            let st = body_start(toks, la, i);
            // a note or a numeral as a step of a part (not a word of a control line: `filter lp`, `key am`)
            if i == st || word(toks.get(i.wrapping_sub(1))) == Some("key") || matches!(toks.get(i.wrapping_sub(1)).map(|t| &t.tok), Some(Tok::Eq)) {
                return None;
            }
            // a call's name (`e(5, 8)`), a word that names something after a `.` or `deck`
            if matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::LParen | Tok::Dot))
                || matches!(word(toks.get(i.wrapping_sub(1))), Some("deck" | "section" | "let" | "def" | "under"))
            {
                return None;
            }
            if djlive::parse::ROLES.contains(&w.as_str()) || djlive::parse::PARAMS.contains(&w.as_str()) {
                return None;
            }
            // canonical notes are lower case; `A` and `B` are decks
            let (kind, label, found) = if let Some(c) = note_choices(w, key).filter(|_| !w.chars().any(|c| c.is_ascii_uppercase())) {
                ("note", if key.is_some() { "note (in key)" } else { "note" }, c)
            } else if djlive::scale::is_roman(w) {
                ("chord", "chord (degree)", roman_choices(w, key)?)
            } else {
                return None;
            };
            let limit = Limit {
                kind,
                label,
                min: Some(0.0),
                max: Some((found.0.len() - 1) as f64),
                step: 1.0,
                unit: "",
                integer: true,
                log: false,
            };
            Some(NumberAt {
                span: t.span,
                text: w.clone(),
                value: found.1 as f64,
                limit,
                label: label.to_string(),
                step: 1.0,
                exact: true,
                range: None,
                choices: found.0,
                index: Some(found.1),
            })
        }
        _ => None,
    }
}

/// The range or ramp the literal (its sign at `j`, itself at `i`) is an end of.
fn range_of(a: &Analysis, la: usize, j: usize, i: usize) -> Option<RangeEnd> {
    let toks = &a.tokens;
    let lit = |k: usize| -> Option<(Span, f64)> {
        match toks.get(k).map(|t| &t.tok) {
            Some(Tok::Num { value, .. }) => {
                let neg = unary_minus(a, la, k);
                let start = if neg { toks[k - 1].span.start } else { toks[k].span.start };
                Some((Span::new(start, toks[k].span.end), if neg { -value } else { *value }))
            }
            _ => None,
        }
    };
    let me = lit(i)?;
    // the end: `a..b` / `a -> b` with this as `a`
    let after = toks.get(i + 1).map(|t| &t.tok);
    if let Some(op @ (Tok::DotDot | Tok::Arrow)) = after {
        let k = if matches!(toks.get(i + 2).map(|t| &t.tok), Some(Tok::Minus)) { i + 3 } else { i + 2 };
        let other = lit(k)?;
        return Some(RangeEnd {
            op: if *op == Tok::DotDot { ".." } else { "->" },
            role: "start",
            start: me.0,
            end: other.0,
            start_value: me.1,
            end_value: other.1,
        });
    }
    let before = j.checked_sub(1).map(|k| &toks[k].tok);
    if let Some(op @ (Tok::DotDot | Tok::Arrow)) = before {
        let k = j.checked_sub(2)?;
        let other = lit(k)?;
        return Some(RangeEnd {
            op: if *op == Tok::DotDot { ".." } else { "->" },
            role: "end",
            start: other.0,
            end: me.0,
            start_value: other.1,
            end_value: me.1,
        });
    }
    None
}

fn num_or_null(v: Option<f64>) -> JsonValue {
    v.map_or(JsonValue::Null, JsonValue::Number)
}

/// The `beatscript/numberAt` answer.
pub fn to_json(a: &Analysis, n: &NumberAt, enc: Encoding) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("range", span_range(&a.index, &a.text, n.span, enc));
    o.set("text", s(n.text.clone()));
    o.set("kind", s(n.limit.kind));
    o.set("label", s(n.label.clone()));
    o.set("value", JsonValue::Number(n.value));
    o.set("min", num_or_null(n.limit.min));
    o.set("max", num_or_null(n.limit.max));
    o.set("step", JsonValue::Number(n.step));
    o.set("decimals", JsonValue::int(decimals(n.step) as i64));
    o.set("unit", s(n.limit.unit));
    o.set("integer", JsonValue::Bool(n.limit.integer));
    o.set("scale", s(if n.limit.log { "log" } else { "linear" }));
    o.set("exact", JsonValue::Bool(n.exact));
    if let Some(r) = &n.range {
        o.set(
            "span",
            crate::json::obj([
                ("op", s(r.op)),
                ("role", s(r.role)),
                ("ordered", JsonValue::Bool(r.op == "..")),
                ("start", span_range(&a.index, &a.text, r.start, enc)),
                ("end", span_range(&a.index, &a.text, r.end, enc)),
                ("startValue", JsonValue::Number(r.start_value)),
                ("endValue", JsonValue::Number(r.end_value)),
            ]),
        );
    }
    if !n.choices.is_empty() {
        o.set("choices", arr(n.choices.iter().map(|c| s(c.clone())).collect()));
        o.set("index", n.index.map_or(JsonValue::Null, |i| JsonValue::int(i as i64)));
    }
    JsonValue::Object(o)
}
