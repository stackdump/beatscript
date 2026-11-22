//! F5: scale degrees, Roman numerals, a key per section and voice leading (docs/BTW-EXTENSIONS.md, docs/btw/ext-scale.md).
//!
//! * `deg(1 b3 5' 1,)`: a group of degrees with accidentals and octave marks ([`ScaleDeg`], parsed by
//!   [`Parser::deg_atom`], one slot like `[..]`).
//! * Roman numerals as steps (`I`, `ii°`, `V7/7`, `bVII`, `#iv`): [`parse_roman`] reads one, [`roman_events`] plays it
//!   against the key of the section it is in, by the role it is in (the bass note in `bass` / `melody`, the chord in
//!   `harmony`, the chord one note at a time in `arp`).
//! * `key` as the first statements of a section ([`Lower::section_keys`], [`section_keys`]).
//! * `voice-lead` on a harmony part ([`ScaleCtx`], [`voice_lead`]): each chord is voiced (inversion and octave) for the
//!   least total semitone motion from the chord before.
//!
//! Everything here is pure and total: a numeral is a table lookup, a voicing is a search over a handful of candidates.

use std::collections::BTreeMap;

use crate::ast::*;
use crate::compile::Compiler;
use crate::diag::{Diagnostic, Span};
use crate::expand::{Lower, Lowered, SegmentLines};
use crate::lex::{Tok, HALF_DIM_SIGN};
use crate::music::{self, Key};
use crate::parse::{edit_distance, Parser};
use crate::pattern::{Ctx, Ev, Sound, NO_SPAN};

type R<T> = Result<T, Diagnostic>;

/// The most a degree may count (four octaves of a seven-note scale).
pub const MAX_DEGREE: u32 = 28;

// ---------------------------------------------------------------------------------------------------------------------
// degrees: deg(1 b3 5' 1,)
// ---------------------------------------------------------------------------------------------------------------------

/// One degree inside `deg( )`: its number, an accidental (-1 flat, +1 sharp) and a net octave shift (`5'` is +1, `1,` -1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleDeg {
    pub n: u32,
    pub acc: i8,
    pub oct: i8,
}

impl ScaleDeg {
    /// As it is written back: `b3`, `#4`, `5'`, `1,`, `5''`.
    pub fn text(&self) -> String {
        let mut s = String::new();
        match self.acc {
            -1 => s.push('b'),
            1 => s.push('#'),
            _ => {}
        }
        s.push_str(&self.n.to_string());
        for _ in 0..self.oct.max(0) {
            s.push('\'');
        }
        for _ in 0..(-self.oct).max(0) {
            s.push(',');
        }
        s
    }

    /// The MIDI note: degree `n` of the key in `octave`, an accidental, the octave marks.
    pub fn midi(&self, key: Key, octave: i32) -> i32 {
        key.degree(self.n, octave) + i32::from(self.acc) + 12 * i32::from(self.oct)
    }
}

/// A word that is a flat or sharp degree: `b3`, `#4` (either case of the `b`). (accidental, degree).
pub fn degree_word(w: &str) -> Option<(i8, u32)> {
    let mut it = w.chars();
    let acc = match it.next()? {
        'b' | 'B' => -1,
        '#' => 1,
        _ => return None,
    };
    let digits = it.as_str();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((acc, digits.parse().ok()?))
}

/// Can a degree's comma mark (`1,`) follow this token?
pub(crate) fn is_degree_tok(t: &Tok) -> bool {
    match t {
        Tok::Num { int, .. } => *int,
        // a degree a macro puts in place (`deg($i, 3)`) is the degree it will be
        Tok::Dollar(_) => true,
        Tok::Apostrophe => true,
        Tok::Word(w) => degree_word(w).is_some(),
        _ => false,
    }
}

impl Parser<'_> {
    /// `deg( ... )`; the `deg` is the next token.
    pub(crate) fn deg_atom(&mut self) -> R<Atom> {
        let name = self.word("`deg`")?;
        self.expect(Tok::LParen, "`(`")?;
        let inside = self.at;
        let seqs = self.deg_stack()?;
        self.warn_comma_list(name.span, inside);
        if seqs.iter().any(|s| s.terms.is_empty()) {
            // (`deg(-1 3)`: a degree counts from 1 and goes below the tonic by octave marks)
            let negative = self.peek() == &Tok::Minus && matches!(self.peek_n(1), Tok::Num { .. });
            let help = if negative { "a degree counts from 1: `-1` is no degree; the tonic an octave down is `1,` and the seventh below it `7,`" } else { "deg(1 b3 5 b7)" };
            return Err(Diagnostic::error(name.span.to(self.span()), if negative { "a degree cannot be negative" } else { "`deg( )` has no degrees in it" })
                .expected(&["a degree like 1, b3, #4, 5' or 1,", "`~`", "`[`", "`<`"])
                .found(self.found())
                .help(help));
        }
        if self.peek() != &Tok::RParen {
            return Err(self.expected(&["a degree", "`)`"], &format!("expected `)` to close deg( ... ), found {}", self.found())).help("a degree is 1, b3, #4, 5' (an octave up) or 1, (an octave down)"));
        }
        self.bump();
        Ok(Atom::Deg(seqs))
    }

    /// `deg(1, 3, 5)`: degrees written as a comma list. A comma and a space after a degree is its octave-down mark, so this is
    /// three degrees an octave down, not the chord; a warning says so (the chord is `deg(1,3,5)` or `deg([1, 3, 5])`).
    fn warn_comma_list(&mut self, name: Span, from: usize) {
        let mut shape: Vec<&Tok> = Vec::new();
        for t in &self.toks[from..self.at.min(self.toks.len())] {
            // (the octave-up marks of a degree are part of it)
            if t.tok != Tok::Apostrophe {
                shape.push(&t.tok);
            }
        }
        let listy = shape.len() >= 3 && shape.len() % 2 == 1 && shape.iter().enumerate().all(|(k, t)| if k % 2 == 0 { is_degree_tok(t) } else { **t == Tok::OctDown });
        if listy {
            let n = shape.len() / 2 + 1;
            self.diags.push(
                Diagnostic::warning(name.to(self.prev_span()), format!("a comma and a space after a degree is its octave-down mark: this is {n} degrees, each but the last an octave down, not a chord"))
                    .help("a chord is `deg(1,3,5)` (commas glued to what follows) or `deg([1, 3, 5])`; an octave down is `1,`"),
            );
        }
    }

    /// `seq { , seq }`: the sequences of a group, stacked.
    fn deg_stack(&mut self) -> R<Vec<Seq>> {
        let mut seqs = vec![self.deg_seq()?];
        while self.peek() == &Tok::Comma {
            self.bump();
            seqs.push(self.deg_seq()?);
        }
        Ok(seqs)
    }

    fn deg_seq(&mut self) -> R<Seq> {
        let start = self.span();
        let mut terms = Vec::new();
        while matches!(self.peek(), Tok::Num { .. } | Tok::Word(_) | Tok::Tilde | Tok::LBrack | Tok::Lt | Tok::LParen) {
            terms.push(self.deg_term()?);
        }
        let span = if terms.is_empty() { Span::new(start.start, start.start) } else { start.to(self.prev_span()) };
        Ok(Seq { span, terms })
    }

    fn adjacent(&self) -> bool {
        self.span().start == self.prev_span().end
    }

    /// One degree term: a degree with its marks, `~`, `x`, a `[group]`, an `<alternation>` or a `(term)`.
    fn deg_term(&mut self) -> R<Term> {
        let open = self.span();
        self.nested(open, "groups and alternations", false, |p| p.deg_term_inner())
    }

    fn deg_term_inner(&mut self) -> R<Term> {
        let start = self.span();
        let atom = match self.peek().clone() {
            Tok::Tilde => {
                self.bump();
                Atom::Rest
            }
            Tok::LParen => {
                // `($i + 1)`: a degree computed (F2), as in any pattern; else `(deg)`, one degree term in parentheses
                let from = self.at;
                match self.paren_int("a scale degree") {
                    Ok((n, sp)) => {
                        if n < 1 {
                            return Err(Diagnostic::error(sp, "a degree counts from 1: 1, 2, 3 ...").found(n.to_string()).help("a rest is `~`; an octave is `5'` up and `1,` down"));
                        }
                        let oct = self.deg_marks();
                        Atom::ScaleDeg(ScaleDeg { n: n.min(i64::from(u32::MAX)) as u32, acc: 0, oct })
                    }
                    Err(arith) => {
                        // the tokens up to the `)` say which it was meant to be
                        let close = {
                            let mut depth = 0usize;
                            let mut at = from;
                            loop {
                                match self.toks[at].tok {
                                    Tok::LParen => depth += 1,
                                    Tok::RParen => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break at;
                                        }
                                    }
                                    Tok::Eof | Tok::Newline => break at,
                                    _ => {}
                                }
                                at += 1;
                            }
                        };
                        let arithmetic = self.toks[from + 1..close].iter().any(|t| matches!(t.tok, Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent | Tok::Dollar(_)) || matches!(&t.tok, Tok::Word(w) if w == "min" || w == "max"));
                        self.at = from;
                        self.bump();
                        let inner = self.deg_term();
                        match inner {
                            Ok(inner) if self.peek() == &Tok::RParen => {
                                self.bump();
                                return Ok(inner);
                            }
                            Ok(_) if arithmetic => return Err(arith),
                            Ok(_) => return Err(self.expected(&["`)`"], &format!("expected `)` to close the parenthesis, found {}", self.found()))),
                            Err(_) if arithmetic => return Err(arith),
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            Tok::LBrack => {
                let open = self.bump().span;
                let seqs = self.deg_stack()?;
                if seqs.iter().any(|s| s.terms.is_empty()) {
                    return Err(Diagnostic::error(open.to(self.span()), "a group has nothing in it").expected(&["a degree", "`~`"]).found(self.found()));
                }
                if self.peek() != &Tok::RBrack {
                    return Err(self.expected(&["`]`", "another degree"], &format!("expected `]` to close the group, found {}", self.found())));
                }
                self.bump();
                Atom::Group(seqs)
            }
            Tok::Lt => {
                let open = self.bump().span;
                let s = self.deg_seq()?;
                if s.terms.is_empty() {
                    return Err(Diagnostic::error(open.to(self.span()), "an alternation has nothing in it").expected(&["a degree to play on the first bar"]).found(self.found()));
                }
                if self.peek() != &Tok::Gt {
                    return Err(self.expected(&["`>`", "another degree"], &format!("expected `>` to close the alternation, found {}", self.found())));
                }
                self.bump();
                Atom::Alt(s.terms)
            }
            Tok::Num { value, int } => {
                let sp = self.bump().span;
                if !int || value < 1.0 {
                    return Err(Diagnostic::error(sp, "a degree counts from 1: 1, 2, 3 ...").found(sp.text(self.src).to_string()).help("an octave is `5'` up and `1,` down, a flat third is `b3`"));
                }
                let oct = self.deg_marks();
                Atom::ScaleDeg(ScaleDeg { n: value as u32, acc: 0, oct })
            }
            Tok::Word(w) => {
                if w == "x" {
                    self.bump();
                    Atom::Hit
                } else if let Some((acc, n)) = degree_word(&w) {
                    self.bump();
                    if n == 0 {
                        return Err(Diagnostic::error(self.prev_span(), "a degree counts from 1: 1, 2, 3 ...").found(format!("`{w}`")));
                    }
                    let oct = self.deg_marks();
                    Atom::ScaleDeg(ScaleDeg { n, acc, oct })
                } else {
                    return Err(Diagnostic::error(self.span(), format!("`{w}` is not a degree"))
                        .expected(&["a degree like 1, b3, #4, 5' or 1,", "`x`", "`~`", "`[`", "`<`"])
                        .found(format!("`{w}`"))
                        .help("inside deg( ) every step is a degree of the key: deg(1 b3 5 b7); a note or a numeral is written outside it"));
                }
            }
            _ => {
                return Err(self.expected(&["a degree like 1, b3, #4, 5' or 1,", "`~`", "`[`", "`<`"], &format!("expected a degree, found {}", self.found())));
            }
        };
        let (accent, slide, marks) = self.term_marks()?;
        let mut fast = None;
        if self.peek() == &Tok::Star {
            self.bump();
            let (n, sp) = self.int("a repeat count")?;
            if n == 0 || n > 64 {
                return Err(Diagnostic::error(sp, "a repeat count is 1 to 64"));
            }
            fast = Some(n);
        }
        Ok(Term { span: start.to(self.prev_span()), atom, fast, accent, slide, marks })
    }

    /// The octave marks glued to a degree: `'` up, `,` down; the net shift.
    fn deg_marks(&mut self) -> i8 {
        let mut oct = 0i8;
        while self.peek() == &Tok::Apostrophe && self.adjacent() {
            self.bump();
            oct = oct.saturating_add(1);
        }
        while self.peek() == &Tok::OctDown && self.adjacent() {
            self.bump();
            oct = oct.saturating_sub(1);
        }
        oct
    }

    /// A Roman numeral as a step: the word was read; take an inversion glued to it (`V7/7`) and spell the
    /// numeral the canonical way. Any other word is a note (or a name of a `let`) as before.
    pub(crate) fn roman_step(&mut self, word: Word) -> Atom {
        Atom::Note(self.roman_word(word))
    }

    /// The word with an inversion glued to it taken in (`V7` `/` `7` is `V7/7`) and, when it is a numeral, spelled
    /// the canonical way.
    pub(crate) fn roman_word(&mut self, mut word: Word) -> Word {
        if roman_head(&word.text).is_some() && self.peek() == &Tok::Slash && self.adjacent() {
            if let Tok::Num { value, int: true } = self.peek_n(1).clone() {
                let slash_end = self.span().end;
                if self.toks[(self.at + 1).min(self.toks.len() - 1)].span.start == slash_end {
                    self.bump();
                    let n = self.bump();
                    word.text = format!("{}/{}", word.text, value as u32);
                    word.span = word.span.to(n.span);
                }
            }
        }
        if let Ok(r) = parse_roman(&word.text) {
            word.text = r.text();
        }
        word
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Roman numerals
// ---------------------------------------------------------------------------------------------------------------------

const NUMERALS: [&str; 7] = ["i", "ii", "iii", "iv", "v", "vi", "vii"];

/// What follows a numeral: the seventh, the triad's colour, a suspension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// A triad: major in upper case, minor in lower.
    Triad,
    /// `7`: dominant in upper case, minor seventh in lower.
    Seventh,
    /// `maj7`: major seventh in upper case, minor-major in lower.
    Maj7,
    /// `°` / `dim`: diminished triad.
    Dim,
    /// `°7` / `dim7`: fully diminished seventh.
    Dim7,
    /// `ø`: half-diminished seventh.
    HalfDim,
    /// `aug`: augmented triad.
    Aug,
    Sus2,
    Sus4,
    /// `6`: added sixth.
    Sixth,
    /// `9`: ninth (dominant in upper case, minor in lower).
    Ninth,
}

/// Every spelling of a quality after the numeral, for did-you-mean.
const QUALITY_WORDS: [&str; 13] = ["7", "maj7", "\u{b0}", "dim", "\u{b0}7", "dim7", "\u{f8}", "\u{f8}7", "aug", "sus2", "sus4", "6", "9"];

impl Quality {
    fn parse(q: &str) -> Option<Quality> {
        Some(match q {
            "" => Quality::Triad,
            "7" => Quality::Seventh,
            "maj7" => Quality::Maj7,
            "\u{b0}" | "dim" => Quality::Dim,
            "\u{b0}7" | "dim7" => Quality::Dim7,
            "\u{f8}" | "\u{f8}7" => Quality::HalfDim,
            "aug" => Quality::Aug,
            "sus2" => Quality::Sus2,
            "sus4" => Quality::Sus4,
            "6" => Quality::Sixth,
            "9" => Quality::Ninth,
            _ => return None,
        })
    }

    /// The canonical spelling.
    pub fn text(self) -> &'static str {
        match self {
            Quality::Triad => "",
            Quality::Seventh => "7",
            Quality::Maj7 => "maj7",
            Quality::Dim => "\u{b0}",
            Quality::Dim7 => "\u{b0}7",
            Quality::HalfDim => "\u{f8}",
            Quality::Aug => "aug",
            Quality::Sus2 => "sus2",
            Quality::Sus4 => "sus4",
            Quality::Sixth => "6",
            Quality::Ninth => "9",
        }
    }

    /// The case a quality fixes whatever was typed: a diminished chord is lower case, an augmented or suspended one
    /// upper (it has no third that case could change). None: the case says major or minor.
    pub fn forced_case(self) -> Option<bool> {
        match self {
            Quality::Dim | Quality::Dim7 | Quality::HalfDim => Some(false),
            Quality::Aug | Quality::Sus2 | Quality::Sus4 => Some(true),
            _ => None,
        }
    }

    /// The tones of the chord as semitones above its root, root first.
    pub fn intervals(self, upper: bool) -> &'static [i32] {
        match (self, upper) {
            (Quality::Triad, true) => &[0, 4, 7],
            (Quality::Triad, false) => &[0, 3, 7],
            (Quality::Seventh, true) => &[0, 4, 7, 10],
            (Quality::Seventh, false) => &[0, 3, 7, 10],
            (Quality::Maj7, true) => &[0, 4, 7, 11],
            (Quality::Maj7, false) => &[0, 3, 7, 11],
            (Quality::Dim, _) => &[0, 3, 6],
            (Quality::Dim7, _) => &[0, 3, 6, 9],
            (Quality::HalfDim, _) => &[0, 3, 6, 10],
            (Quality::Aug, _) => &[0, 4, 8],
            (Quality::Sus2, _) => &[0, 2, 7],
            (Quality::Sus4, _) => &[0, 5, 7],
            (Quality::Sixth, true) => &[0, 4, 7, 9],
            (Quality::Sixth, false) => &[0, 3, 7, 9],
            (Quality::Ninth, true) => &[0, 4, 7, 10, 14],
            (Quality::Ninth, false) => &[0, 3, 7, 10, 14],
        }
    }

    /// The factors of the chord, by name (the tones an inversion may put in the bass).
    pub fn factors(self) -> &'static [u8] {
        match self {
            Quality::Sus2 => &[1, 2, 5],
            Quality::Sus4 => &[1, 4, 5],
            Quality::Triad | Quality::Dim | Quality::Aug => &[1, 3, 5],
            Quality::Seventh | Quality::Maj7 | Quality::Dim7 | Quality::HalfDim => &[1, 3, 5, 7],
            Quality::Sixth => &[1, 3, 5, 6],
            Quality::Ninth => &[1, 3, 5, 7, 9],
        }
    }
}

/// A Roman numeral step: `bVII`, `ii°`, `V7/7`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roman {
    /// -1 for `b`, +1 for `#`: the root a semitone below or above the scale's degree.
    pub acc: i8,
    /// 1..=7.
    pub degree: u8,
    /// Upper case (a major third) or lower (minor); the canonical case of the qualities that fix it.
    pub upper: bool,
    pub quality: Quality,
    /// The tone in the bass: 3, 5 or 7 (`I/3` is first inversion).
    pub inversion: Option<u8>,
}

/// Why a word is not a numeral step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RomanErr {
    /// Not a numeral at all: not ours to explain.
    NotRoman,
    /// `Iv`: upper and lower case in one numeral.
    Mixed(String),
    /// `VIII`, `IIII`: more than the seven.
    TooHigh(String),
    /// `Imaj`, `Vsus3`: the numeral is good, the rest is not a quality.
    Quality { numeral: String, rest: String },
    /// `I/7`, `Isus4/3`: the chord has no such tone.
    NoTone { tone: u8, chord: String },
    /// `I/4`: an inversion names 3, 5 or 7.
    BadInversion(String),
}

fn is_roman_letter(c: char) -> bool {
    matches!(c, 'i' | 'v' | 'I' | 'V')
}

/// The numeral at the head of a word (after a `b` or `#`): (accidental, the run of numeral letters, the rest).
fn split_head(body: &str) -> Option<(i8, &str, &str)> {
    let (acc, rest) = if let Some(r) = body.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = body.strip_prefix('b').filter(|r| r.starts_with(is_roman_letter)) {
        (-1, r)
    } else {
        (0, body)
    };
    let run = rest.chars().take_while(|c| is_roman_letter(*c)).count();
    if run == 0 {
        return None;
    }
    let (n, q) = rest.split_at(run);
    Some((acc, n, q))
}

/// Does the word begin with a valid numeral (any quality after it)? (accidental, degree 1..=7).
pub fn roman_head(word: &str) -> Option<(i8, u8)> {
    let body = word.split('/').next().unwrap_or(word);
    let (acc, run, _) = split_head(body)?;
    let one_case = run.chars().all(|c| c.is_ascii_uppercase()) || run.chars().all(|c| c.is_ascii_lowercase());
    let d = NUMERALS.iter().position(|n| *n == run.to_ascii_lowercase())?;
    one_case.then_some((acc, d as u8 + 1))
}

/// Read a numeral step: `I`, `bVII`, `ii°`, `V7/7`.
pub fn parse_roman(s: &str) -> Result<Roman, RomanErr> {
    let (body, inv) = match s.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let (acc, run, q) = split_head(body).ok_or(RomanErr::NotRoman)?;
    let upper = run.chars().all(|c| c.is_ascii_uppercase());
    if !upper && !run.chars().all(|c| c.is_ascii_lowercase()) {
        return Err(RomanErr::Mixed(run.to_string()));
    }
    let Some(d) = NUMERALS.iter().position(|n| *n == run.to_ascii_lowercase()) else {
        return Err(RomanErr::TooHigh(run.to_string()));
    };
    let ql = q.to_lowercase();
    let Some(quality) = Quality::parse(&ql) else {
        return Err(RomanErr::Quality { numeral: run.to_string(), rest: q.to_string() });
    };
    let upper = quality.forced_case().unwrap_or(upper);
    let degree = d as u8 + 1;
    let inversion = match inv {
        None => None,
        Some(n) => match n.parse::<u8>() {
            Ok(t @ (3 | 5 | 7)) => {
                if !quality.factors().contains(&t) {
                    let r = Roman { acc, degree, upper, quality, inversion: None };
                    return Err(RomanErr::NoTone { tone: t, chord: r.text() });
                }
                Some(t)
            }
            _ => return Err(RomanErr::BadInversion(n.to_string())),
        },
    };
    Ok(Roman { acc, degree, upper, quality, inversion })
}

/// Is this word a whole, valid numeral step? (`let vi` and `section iv` are refused: they are reserved.)
pub fn is_roman(word: &str) -> bool {
    parse_roman(word).is_ok()
}

/// The canonical spelling of a word: a Roman numeral the canonical way, anything else as a pitch word is spelled.
pub fn canonical_word(w: &str) -> String {
    match parse_roman(w) {
        Ok(r) => r.text(),
        Err(_) => music::canonical_pitch_word(w),
    }
}

impl Roman {
    /// The canonical text: `bVII`, `ii°`, `V7`, `I/3`.
    pub fn text(&self) -> String {
        let n = NUMERALS[usize::from(self.degree) - 1];
        let n = if self.upper { n.to_ascii_uppercase() } else { n.to_string() };
        let acc = match self.acc {
            -1 => "b",
            1 => "#",
            _ => "",
        };
        let inv = self.inversion.map_or(String::new(), |i| format!("/{i}"));
        format!("{acc}{n}{}{inv}", self.quality.text())
    }

    /// The scale a numeral counts its degrees in: the key's own when it has seven notes, else the diatonic scale it
    /// comes from (a pentatonic key counts in its major or minor scale, the blues in the minor).
    fn degrees(key: Key) -> &'static [i64] {
        match key.mode {
            0..=5 => key.intervals(),
            6 => Key { tonic: key.tonic, mode: 0 }.intervals(),
            _ => Key { tonic: key.tonic, mode: 1 }.intervals(),
        }
    }

    /// The triad the key itself builds on its scale degree `degree` (1..=7, 8 is 1 again), as the numeral that names it:
    /// `i`, `VI`, `vii°`, `IIIaug`. A key of fewer than seven notes counts in the scale it comes from, as every numeral does.
    pub fn triad(key: Key, degree: u32) -> Roman {
        let iv = Roman::degrees(key);
        let d = (degree.max(1) as usize - 1) % 7;
        let at = |i: usize| iv[i % 7] + 12 * (i / 7) as i64;
        let (third, fifth) = (at(d + 2) - at(d), at(d + 4) - at(d));
        let (quality, upper) = match (third, fifth) {
            (3, 7) => (Quality::Triad, false),
            (3, 6) => (Quality::Dim, false),
            (4, 8) => (Quality::Aug, true),
            _ => (Quality::Triad, true),
        };
        Roman { acc: 0, degree: d as u8 + 1, upper, quality, inversion: None }
    }

    /// The root as a MIDI note: the key's degree in `octave`, a semitone lower for `b`, higher for `#`.
    pub fn root(&self, key: Key, octave: i32) -> i32 {
        music::midi(key.tonic, octave) + Roman::degrees(key)[usize::from(self.degree) - 1] as i32 + i32::from(self.acc)
    }

    /// The chord in root position, root first.
    pub fn notes(&self, key: Key, octave: i32) -> Vec<i32> {
        let root = self.root(key, octave);
        self.quality.intervals(self.upper).iter().map(|i| root + i).collect()
    }

    /// The index in [`Roman::notes`] of the tone in the bass (0: the root).
    pub fn bass_index(&self) -> usize {
        self.inversion.and_then(|t| self.quality.factors().iter().position(|f| *f == t)).unwrap_or(0)
    }
}

/// The notes with the `k` lowest raised an octave (`k` = 1 is first inversion), lowest first.
pub fn invert(base: &[i32], k: usize) -> Vec<i32> {
    let mut v: Vec<i32> = base.iter().enumerate().map(|(i, n)| if i < k { n + 12 } else { *n }).collect();
    v.sort_unstable();
    v
}

/// A numeral that did not read, said so it can be fixed. None when the word is not meant as a numeral.
pub(crate) fn roman_error(w: &Word, e: RomanErr) -> Option<Diagnostic> {
    let text = w.text.as_str();
    let d = match e {
        RomanErr::NotRoman => return None,
        RomanErr::Mixed(run) => {
            let rest = text.trim_start_matches(['b', '#']).strip_prefix(run.as_str()).unwrap_or("");
            Diagnostic::error(w.span, format!("`{text}` mixes upper and lower case in its numeral"))
                .found(format!("`{text}`"))
                .help(format!("upper case is a major chord, lower case minor: `{}{rest}` or `{}{rest}`", run.to_ascii_uppercase(), run.to_ascii_lowercase()))
        }
        RomanErr::TooHigh(run) => Diagnostic::error(w.span, format!("`{run}` is not a numeral: a scale has seven degrees, I to VII"))
            .found(format!("`{text}`"))
            .help("an eighth is the tonic again: `I`; a chord higher up is written with `oct=` on the part"),
        RomanErr::Quality { numeral, rest } => {
            // not within a letter of any quality: some other word (a name that happens to begin with i or v: `vel`)
            let lower = rest.to_lowercase();
            let near = QUALITY_WORDS.iter().copied().filter(|q| edit_distance(&lower, q) <= 1).min_by_key(|q| edit_distance(&lower, q))?;
            let (acc, _, _) = split_head(text.split('/').next().unwrap_or(text))?;
            let pre = match acc {
                -1 => "b",
                1 => "#",
                _ => "",
            };
            let inv = text.split_once('/').map_or(String::new(), |(_, i)| format!("/{i}"));
            Diagnostic::error(w.span, format!("`{rest}` is not a chord quality after the numeral `{numeral}`"))
                .expected(&QUALITY_WORDS)
                .found(format!("`{rest}`"))
                .help(format!("did you mean `{pre}{numeral}{near}{inv}`?"))
        }
        RomanErr::NoTone { tone, chord } => {
            let r = parse_roman(&chord).ok();
            let tones = r.as_ref().map_or(String::new(), |r| r.quality.factors().iter().map(|f| f.to_string()).collect::<Vec<_>>().join(" "));
            let inverts: Vec<String> = r.as_ref().map_or(Vec::new(), |r| r.quality.factors().iter().filter(|f| matches!(**f, 3 | 5 | 7)).map(|f| format!("`{chord}/{f}`")).collect());
            let mut help = format!("the tones of `{chord}` are {tones}, so it inverts as {}", inverts.join(" or "));
            if tone == 7 && r.as_ref().is_some_and(|r| matches!(r.quality, Quality::Triad | Quality::Dim | Quality::Aug)) {
                help.push_str(&format!("; for the seventh in the bass write a seventh chord: `{chord}7/7`"));
            }
            Diagnostic::error(w.span, format!("`{chord}` has no {} to put in the bass", tone_name(tone))).found(format!("`{text}`")).help(help)
        }
        RomanErr::BadInversion(n) => Diagnostic::error(w.span, format!("`/{n}` is not an inversion: it names the tone in the bass, `/3` (first inversion), `/5` (second) or `/7` (third)")).found(format!("`{text}`")),
    };
    Some(d)
}

fn tone_name(t: u8) -> &'static str {
    match t {
        3 => "third",
        5 => "fifth",
        _ => "seventh",
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// evaluation: a part's role, voice leading
// ---------------------------------------------------------------------------------------------------------------------

/// What a part's role does with a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    /// A drum: no pitch to give a numeral.
    Drum,
    /// `bass`, `melody`: one note at a time: the chord's bass note.
    Single,
    /// `harmony`: the chord's notes together.
    Chord,
    /// `arp`: the chord one note at a time over the slot.
    Arp,
}

impl RoleKind {
    pub fn of(role: &str) -> RoleKind {
        match role {
            "harmony" => RoleKind::Chord,
            "arp" => RoleKind::Arp,
            "bass" | "melody" => RoleKind::Single,
            _ => RoleKind::Drum,
        }
    }
}

/// What a part carries while its pattern is read: its role, whether it voice-leads, and the chord it played last (in
/// the order the pattern is read: bar by bar, left to right).
#[derive(Clone, Debug)]
pub struct ScaleCtx {
    pub role: RoleKind,
    pub lead: bool,
    pub prev: Option<Vec<i32>>,
}

impl ScaleCtx {
    pub fn new(role: &str, lead: bool) -> ScaleCtx {
        ScaleCtx { role: RoleKind::of(role), lead, prev: None }
    }
}

/// Semitones the voices move from `prev` to `cand`: the notes in order, one to one when there are as many, else the
/// shorter chord matched in order against the longer one's best notes (a voice that comes in or drops out does not move).
pub fn motion(prev: &[i32], cand: &[i32]) -> i64 {
    let (mut a, mut b) = (prev.to_vec(), cand.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    let (small, large) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    const INF: i64 = i64::MAX / 4;
    // dp[i][j]: the first i notes of the shorter matched, in order, into the first j of the longer
    let mut dp = vec![vec![INF; large.len() + 1]; small.len() + 1];
    for cell in dp[0].iter_mut() {
        *cell = 0;
    }
    for i in 1..=small.len() {
        for j in 1..=large.len() {
            let step = dp[i - 1][j - 1].saturating_add(i64::from((small[i - 1] - large[j - 1]).abs()));
            dp[i][j] = dp[i][j - 1].min(step);
        }
    }
    dp[small.len()][large.len()]
}

/// The voicing of the chord `base` (root position, lowest first) that moves least from `prev`. Candidates are every
/// inversion (only `bass`, when the numeral wrote one) in every octave from three below to three above the written
/// one; ties go to the lower inversion, then the lower octave. With nothing before it (the first chord), the root
/// position (or the written inversion) as written.
pub fn voice_lead(prev: Option<&[i32]>, base: &[i32], bass: Option<usize>) -> Vec<i32> {
    let Some(prev) = prev else { return invert(base, bass.unwrap_or(0)) };
    let invs: Vec<usize> = match bass {
        Some(b) => vec![b],
        None => (0..base.len()).collect(),
    };
    let mut best: Option<(i64, Vec<i32>)> = None;
    for inv in invs {
        let shape = invert(base, inv);
        for oct in -3..=3i32 {
            let cand: Vec<i32> = shape.iter().map(|n| n + 12 * oct).collect();
            if cand.iter().any(|n| !(0..=127).contains(n)) {
                continue;
            }
            let cost = motion(prev, &cand);
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                best = Some((cost, cand));
            }
        }
    }
    best.map_or_else(|| invert(base, bass.unwrap_or(0)), |(_, v)| v)
}

fn ev(at: f64, note: i32) -> Ev {
    Ev { at, sound: Sound::Note(note), vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }
}

/// The steps an `arp` numeral plays in its slot.
const ARP_STEPS: usize = 4;

/// A chord (root position `base`) as the part's role plays it. `bass` is the index of the tone a written inversion
/// puts in the bass; `led` is false for a chord that is voiced as written (and is still the chord before for the next one).
fn play_chord(ctx: &mut Ctx, base: &[i32], bass: Option<usize>, led: bool) -> Vec<Ev> {
    let voiced = if ctx.scale.lead && led { voice_lead(ctx.scale.prev.as_deref(), base, bass) } else { invert(base, bass.unwrap_or(0)) };
    if ctx.scale.lead {
        ctx.scale.prev = Some(voiced.clone());
    }
    match ctx.scale.role {
        RoleKind::Arp => {
            // four steps over the slot; a ninth chord drops its fifth to fit
            let notes: Vec<i32> = if voiced.len() > ARP_STEPS { voiced.iter().enumerate().filter(|(i, _)| *i != 2).map(|(_, n)| *n).collect() } else { voiced };
            (0..ARP_STEPS).map(|k| ev(k as f64 / ARP_STEPS as f64, notes[k % notes.len()])).collect()
        }
        _ => voiced.into_iter().map(|n| ev(0.0, n)).collect(),
    }
}

/// `chord(am7)` or `chord(V7)` in a part that voice-leads (and no octave written): the chord, voiced from the one
/// before (`bass`: the tone a numeral's inversion puts lowest).
pub(crate) fn lead_chord(ctx: &mut Ctx, base: &[i32], bass: Option<usize>) -> Vec<Ev> {
    play_chord(ctx, base, bass, true)
}

/// A degree of `deg( )`: one note.
pub(crate) fn eval_degree(d: &ScaleDeg, span: Span, ctx: &mut Ctx) -> Vec<Ev> {
    if d.n == 0 || d.n > MAX_DEGREE {
        let mut e = Diagnostic::error(span, format!("scale degree {} is outside the scale", d.n)).help(format!("degrees go 1..{MAX_DEGREE} (four octaves of a seven-note scale); `5'` is the fifth an octave up"));
        e.found = d.text();
        push_err(ctx, e);
        return Vec::new();
    }
    vec![ev(0.0, d.midi(ctx.key, ctx.octave))]
}

fn push_err(ctx: &mut Ctx, d: Diagnostic) {
    if !ctx.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
        ctx.diags.push(d);
    }
}

/// A Roman numeral step, read where a pitch is read. None: the word is not meant as a numeral (the caller
/// reports it as it always has).
pub(crate) fn roman_events(w: &Word, ctx: &mut Ctx) -> Option<Vec<Ev>> {
    let r = match parse_roman(&w.text) {
        Ok(r) => r,
        Err(e) => {
            return roman_error(w, e).map(|mut d| {
                if let Some(h) = comment_hint(ctx.src, w.span.start) {
                    d = d.help(h);
                }
                push_err(ctx, d);
                Vec::new()
            })
        }
    };
    let base = r.notes(ctx.key, ctx.octave);
    let bass = r.bass_index();
    let written = r.inversion.map(|_| bass);
    Some(match ctx.scale.role {
        RoleKind::Drum => {
            push_err(
                ctx,
                Diagnostic::error(w.span, format!("`{}` is a chord, and a drum part has no pitch to give it", r.text()))
                    .help("numerals go in bass, melody, harmony or arp: `deck B: harmony I V vi IV`"),
            );
            Vec::new()
        }
        RoleKind::Single => vec![ev(0.0, base[bass])],
        RoleKind::Chord | RoleKind::Arp => play_chord(ctx, &base, written, true),
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// the compiler: the part's `voice-lead`, a key per section
// ---------------------------------------------------------------------------------------------------------------------

/// `voice-lead` / `voice-lead=on` / `=off` on a part: whether it is on. None (an error is reported) for another value
/// or a part that plays no chords.
pub(crate) fn voice_lead_param(c: &mut Compiler, pr: &Param, role: &str) -> Option<bool> {
    let on = match &pr.value {
        ParamValue::Word(w) if w.text == "on" => true,
        ParamValue::Word(w) if w.text == "off" => false,
        other => {
            let (sp, found) = match other {
                ParamValue::Word(w) => (w.span, format!("`{}`", w.text)),
                ParamValue::Num(n) => (n.span, n.span.text(c.src).to_string()),
                ParamValue::Range(a, b) => (a.span.to(b.span), String::new()),
            };
            c.err(Diagnostic::error(sp, "`voice-lead` is `on` or `off`").expected(&["on", "off"]).found(found).help("voice-lead   or   voice-lead=on"));
            return None;
        }
    };
    if on && !matches!(role, "harmony" | "arp") {
        c.err(Diagnostic::error(pr.name.span, format!("`voice-lead` shapes the chords of a part; `{role}` plays one note at a time")).help("put it on a `harmony` (or `arp`) part: harmony I vi IV V voice-lead"));
        return None;
    }
    Some(on)
}

/// The key in force in a section: its own `key`, else the program's.
pub(crate) fn key_of(seg: &SegmentLines, program: Key) -> Key {
    seg.key.as_ref().and_then(|w| Key::parse(&w.text)).unwrap_or(program)
}

/// Every section on the timeline and the key in force in it. A `key` that is not a key is an error (once, whatever the
/// number of times the section plays).
pub(crate) fn section_keys(lw: &Lowered, program: Key, diags: &mut Vec<Diagnostic>) -> BTreeMap<String, Key> {
    let mut out = BTreeMap::new();
    let mut reported: Vec<Span> = Vec::new();
    for seg in &lw.segments {
        if let Some(w) = &seg.key {
            if Key::parse(&w.text).is_none() && !reported.contains(&w.span) {
                reported.push(w.span);
                diags.push(
                    Diagnostic::error(w.span, format!("`{}` is not a key", w.text))
                        .expected(&["a key like am, c, f#m or d:dorian"])
                        .found(format!("`{}`", w.text))
                        .help(format!("modes: {}", music::MODES.join(", "))),
                );
            }
        }
        out.insert(seg.name.clone(), key_of(seg, program));
    }
    out
}

impl Lower<'_> {
    /// The `key` statements at the head of a section's body: the last one is the section's key. Answers that key and how
    /// many statements they are (the caller expands the rest). A section that plays in several stretches (F10, a stack)
    /// is read once, so a key said twice is said once.
    pub(crate) fn section_keys(&mut self, body: &[Stmt]) -> (Option<Word>, usize) {
        let mut n = 0;
        let mut key: Option<Word> = None;
        for st in body {
            match &st.kind {
                StmtKind::Setting(Setting::Key(k)) => {
                    // (once, however many times the section plays)
                    if n > 0 && !self.diags.iter().any(|d| d.span == st.span) {
                        self.diags.push(Diagnostic::warning(st.span, "key is set twice in this section; the last one wins"));
                    }
                    key = Some(k.clone());
                    n += 1;
                }
                _ => break,
            }
        }
        (key, n)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the text: sharp words in the lexer, deg( ) printed
// ---------------------------------------------------------------------------------------------------------------------

/// A word that begins with a `#`, for the lexer. `at` is the `#`; `in_parens` is whether the line has an open
/// parenthesis, `after_step` whether something came before on the line. `#4` inside parentheses is a sharp degree;
/// `#iv` after a step (and followed by a space, a `/`, a closing mark or the end) is a sharp numeral. Anything else
/// starting with `#` is a comment. The length of the word.
pub(crate) fn sharp_word_len(src: &str, at: usize, in_parens: bool, after_step: bool) -> Option<usize> {
    let b = src.as_bytes();
    if at > 0 && (b[at - 1] as char).is_ascii_alphanumeric() {
        return None;
    }
    let rest = &src[at + 1..];
    let first = rest.chars().next()?;
    let word_len = |s: &str| -> usize {
        let mut n = 0;
        for c in s.chars() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '\u{b0}' || c == HALF_DIM_SIGN {
                n += c.len_utf8();
            } else {
                break;
            }
        }
        n
    };
    if in_parens && first.is_ascii_digit() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let after = rest[digits..].chars().next();
        return after.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_')).then_some(1 + digits);
    }
    if after_step && is_roman_letter(first) {
        let n = word_len(rest);
        let valid = parse_roman(&rest[..n]).is_ok_and(|r| r.acc == 0 && r.inversion.is_none());
        let after = rest[n..].chars().next();
        let boundary = after.is_none_or(|c| c.is_whitespace() || matches!(c, '/' | ')' | ']' | '>' | ',' | '*' | '!' | '^' | '|'));
        return (valid && boundary && next_word_is_step(&rest[n..])).then_some(1 + n);
    }
    None
}

/// What follows a sharp numeral on its line: nothing, a mark (`|`, `]`, `/`) or a step (`x`, a note, a numeral, a call). A word
/// that is none of these (`#vi is the sixth`, `#I like it`) means the `#` began a comment.
fn next_word_is_step(rest: &str) -> bool {
    let rest = rest.trim_start_matches([' ', '\t', '\r']);
    let n = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '#' | '-' | '\u{b0}' | HALF_DIM_SIGN)).count();
    if n == 0 || !rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '#') {
        return true;
    }
    let w: String = rest.chars().take(n).collect();
    let after = rest[w.len()..].chars().next();
    // (a part's param, `vel=90` or `voice-lead`, and a `when` guard follow a pattern too)
    w == "x" || after == Some('(') || after == Some('=') || music::parse_note(&w).is_some() || music::Chord::parse(&w).is_some() || is_roman(w.trim_start_matches('#')) || w.starts_with('#') || w == "when" || w == "voice-lead"
}

/// The hint for a word that is wrong where a comment may have been meant: on its line, after a sharp numeral (`#I`, a
/// word that begins with `#`), with only plain words in between. `#I like it` is a sharp numeral and two words; a comment
/// starts with `#` and a space.
pub(crate) fn comment_hint(src: &str, at: usize) -> Option<String> {
    let at = at.min(src.len());
    let line = &src[src[..at].rfind('\n').map_or(0, |n| n + 1)..at];
    let mut words = line.split_whitespace().rev();
    let sharp = loop {
        let w = words.next()?;
        if let Some(r) = w.strip_prefix('#') {
            break (!r.is_empty() && is_roman(w)).then_some(w)?;
        }
        if !w.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\u{b0}' || c == HALF_DIM_SIGN) {
            return None;
        }
    };
    Some(format!("if `{sharp}` was meant as a comment: a comment starts with `# ` (hash and a space); `#iv` is a sharp numeral"))
}

/// `deg(1 b3 5' 1,)`, canonical: one space between degrees, accidentals and marks glued, a stack comma glued to what
/// follows it (a stack after a degree that ends in a comma mark takes a space before its comma, so the two are told apart).
pub fn print_deg(seqs: &[Seq]) -> String {
    format!("deg({})", deg_stack(seqs))
}

fn deg_stack(seqs: &[Seq]) -> String {
    let mut out = String::new();
    for (i, s) in seqs.iter().enumerate() {
        if i > 0 {
            out.push_str(if out.ends_with(',') { " ," } else { "," });
        }
        out.push_str(&deg_seq(s));
    }
    out
}

fn deg_seq(s: &Seq) -> String {
    s.terms.iter().map(deg_term).collect::<Vec<_>>().join(" ")
}

fn deg_term(t: &Term) -> String {
    let a = match &t.atom {
        Atom::Rest => "~".to_string(),
        Atom::Hit => "x".to_string(),
        Atom::ScaleDeg(d) => d.text(),
        Atom::Group(seqs) => format!("[{}]", deg_stack(seqs)),
        Atom::Alt(ts) => format!("<{}>", ts.iter().map(deg_term).collect::<Vec<_>>().join(" ")),
        // not made by the parser; printed as a step so a hand-built tree still shows
        other => crate::print::seq(&Seq { span: t.span, terms: vec![Term { span: t.span, atom: other.clone(), fast: None, accent: false, slide: false, marks: Marks::default() }] }),
    };
    let a = format!("{a}{}{}{}", if t.accent { "!" } else { "" }, if t.slide { "^" } else { "" }, t.marks.text());
    match t.fast {
        Some(k) => format!("{a}*{k}"),
        None => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerals_read_by_case_and_quality() {
        let r = parse_roman("V7/7").unwrap();
        assert_eq!((r.degree, r.upper, r.quality, r.inversion), (5, true, Quality::Seventh, Some(7)));
        assert_eq!(parse_roman("bVII").unwrap().acc, -1);
        assert_eq!(parse_roman("#iv").unwrap().acc, 1);
        assert_eq!(parse_roman("vii\u{b0}").unwrap().text(), "vii\u{b0}");
        assert_eq!(parse_roman("VII\u{b0}").unwrap().text(), "vii\u{b0}", "a diminished chord is lower case");
        assert_eq!(parse_roman("Idim").unwrap().text(), "i\u{b0}");
        assert_eq!(parse_roman("vii\u{f8}7").unwrap().text(), "vii\u{f8}");
        assert_eq!(parse_roman("iaug").unwrap().text(), "Iaug");
        assert_eq!(parse_roman("ISUS4").unwrap().text(), "Isus4");
        assert_eq!(parse_roman("IMAJ7").unwrap().text(), "Imaj7");
        assert_eq!(parse_roman("imaj7").unwrap().text(), "imaj7");
        assert_eq!(parse_roman("ii9").unwrap().quality.intervals(false), &[0, 3, 7, 10, 14]);
        assert!(matches!(parse_roman("Iv"), Err(RomanErr::Mixed(_))));
        assert!(matches!(parse_roman("VIII"), Err(RomanErr::TooHigh(_))));
        assert!(matches!(parse_roman("Imaj"), Err(RomanErr::Quality { .. })));
        assert!(matches!(parse_roman("I/7"), Err(RomanErr::NoTone { tone: 7, .. })));
        assert!(matches!(parse_roman("Isus4/3"), Err(RomanErr::NoTone { tone: 3, .. })));
        assert!(matches!(parse_roman("I/4"), Err(RomanErr::BadInversion(_))));
        assert!(matches!(parse_roman("c3"), Err(RomanErr::NotRoman)));
        assert!(matches!(parse_roman("b3"), Err(RomanErr::NotRoman)));
        assert!(parse_roman("bb4").is_err());
    }

    #[test]
    fn no_word_of_the_language_reads_as_a_numeral() {
        use crate::parse::{ACTION_WORDS, EXT_WORDS, KEY_VERBS, PARAMS, ROLES, SUGAR};
        let mut words: Vec<&str> = Vec::new();
        words.extend(ROLES);
        words.extend(ACTION_WORDS);
        words.extend(PARAMS);
        words.extend(EXT_WORDS);
        words.extend(KEY_VERBS);
        words.extend(SUGAR.iter().map(|(w, _)| *w));
        words.extend(crate::compile::MOVES);
        words.extend(["x", "deck", "master", "A", "B", "do", "let", "def", "repeat", "section", "arrange", "import", "end", "loop", "sample", "use", "restricted", "evolve", "gen", "gens", "stop", "when", "every", "on", "at", "after", "before", "for", "over", "bars", "bar", "tempo", "swing", "key", "phrase", "next", "from", "pair", "pairs", "with", "group", "mashup", "up", "down", "updown", "lp", "hp", "off", "open", "play", "pause", "jump", "add", "rotate", "transpose", "octave", "invert", "reverse", "thin", "vel", "inst", "kit", "sound", "route", "seed", "license", "else", "own", "synth"]);
        for w in words {
            assert!(!is_roman(w), "`{w}` is a word of the language and reads as a numeral");
        }
        // and none of the pitch words: notes, keys, chords
        for w in ["a", "b", "bb", "bb4", "eb", "c#3", "am", "f#m7", "dsus4", "d:dorian"] {
            assert!(!is_roman(w), "{w}");
        }
    }

    #[test]
    fn the_notes_of_a_numeral_in_a_key() {
        let am = Key::parse("am").unwrap();
        // harmony is octave 3: A3 = 57
        assert_eq!(parse_roman("i").unwrap().notes(am, 3), vec![57, 60, 64]);
        assert_eq!(parse_roman("VI").unwrap().notes(am, 3), vec![65, 69, 72]);
        assert_eq!(parse_roman("III").unwrap().notes(am, 3), vec![60, 64, 67]);
        assert_eq!(parse_roman("VII").unwrap().notes(am, 3), vec![67, 71, 74]);
        assert_eq!(parse_roman("bVII").unwrap().notes(am, 3), vec![66, 70, 73]);
        let c = Key::C_MAJOR;
        assert_eq!(parse_roman("V7").unwrap().notes(c, 3), vec![55, 59, 62, 65]);
        assert_eq!(parse_roman("bVII").unwrap().notes(c, 3)[0], 58);
        assert_eq!(parse_roman("#iv\u{b0}").unwrap().notes(c, 3), vec![54, 57, 60]);
    }

    #[test]
    fn inversions_raise_the_lower_tones() {
        let c = Key::C_MAJOR;
        let r = parse_roman("I/3").unwrap();
        assert_eq!(invert(&r.notes(c, 3), r.bass_index()), vec![52, 55, 60]);
        let r = parse_roman("I/5").unwrap();
        assert_eq!(invert(&r.notes(c, 3), r.bass_index()), vec![55, 60, 64]);
        let r = parse_roman("V7/7").unwrap();
        assert_eq!(invert(&r.notes(c, 3), r.bass_index()), vec![65, 67, 71, 74]);
    }

    #[test]
    fn motion_is_the_voices_moved() {
        assert_eq!(motion(&[57, 60, 64], &[57, 60, 65]), 1);
        assert_eq!(motion(&[57, 60, 64], &[55, 60, 64]), 2);
        // a voice that joins or leaves does not move
        assert_eq!(motion(&[60, 64, 67], &[60, 64, 67, 70]), 0);
        assert_eq!(motion(&[60, 64, 67, 70], &[60, 64, 67]), 0);
        assert_eq!(motion(&[60, 64, 67], &[62, 65, 69, 72]), 5);
    }

    #[test]
    fn voice_leading_picks_the_least_motion_and_breaks_ties_low() {
        // i VI III VII in A minor from A3 C4 E4
        let am = Key::parse("am").unwrap();
        let chord = |s: &str| parse_roman(s).unwrap().notes(am, 3);
        let first = voice_lead(None, &chord("i"), None);
        assert_eq!(first, vec![57, 60, 64]);
        let second = voice_lead(Some(&first), &chord("VI"), None);
        assert_eq!(second, vec![57, 60, 65]);
        let third = voice_lead(Some(&second), &chord("III"), None);
        assert_eq!(third, vec![55, 60, 64]);
        let fourth = voice_lead(Some(&third), &chord("VII"), None);
        assert_eq!(fourth, vec![55, 59, 62]);
        // an unmoving chord stays
        assert_eq!(voice_lead(Some(&first), &chord("i"), None), first);
    }

    #[test]
    fn degrees_print_with_their_marks() {
        let d = ScaleDeg { n: 5, acc: 0, oct: 2 };
        assert_eq!(d.text(), "5''");
        assert_eq!(ScaleDeg { n: 1, acc: 0, oct: -1 }.text(), "1,");
        assert_eq!(ScaleDeg { n: 3, acc: -1, oct: 0 }.text(), "b3");
        assert_eq!(ScaleDeg { n: 4, acc: 1, oct: 1 }.text(), "#4'");
        let am = Key::parse("am").unwrap();
        assert_eq!(ScaleDeg { n: 3, acc: -1, oct: 0 }.midi(am, 3), 57 + 3 - 1);
        assert_eq!(ScaleDeg { n: 1, acc: 0, oct: 1 }.midi(am, 3), 69);
    }

    #[test]
    fn sharp_words_are_a_degree_in_parentheses_a_numeral_after_a_step_and_else_a_comment() {
        // `#4` in parentheses
        assert_eq!(sharp_word_len("deg(1 #4)", 6, true, true), Some(2));
        assert_eq!(sharp_word_len("deg(1 #4)", 6, false, true), None);
        // `#iv` after a step
        assert_eq!(sharp_word_len("harmony I #iv V", 10, false, true), Some(3));
        assert_eq!(sharp_word_len("#iv", 0, false, false), None, "a comment at the head of a line");
        assert_eq!(sharp_word_len("kick x # iv", 7, false, true), None);
        assert_eq!(sharp_word_len("kick x #intro", 7, false, true), None, "a comment that begins with a numeral's letters is still a comment");
        assert_eq!(sharp_word_len("kick x #iv.", 7, false, true), None);
    }
}
