//! Constraints (docs/btw/ext-assert.md, "Constraints"): the rules of F12's `assert` that are about the notes.
//!
//! ```text
//! assert never A.snare beside A.snare      no two snares on neighbouring steps
//! assert never B during sing               deck B is silent while the vocal sings
//! assert bass ends on 1                    every section's last bass note is its tonic
//! ```
//!
//! They are **decided by `check`, by exact enumeration**: a program is a finite timeline of bars, each part's hits in each bar
//! are in [`Compiled`] (`PartIr::cycles`, with the arrange mutations, `repeat`, `def`, `evolve` generations and key changes
//! already applied), and a constraint is asked of every step of it. Nothing is sampled and nothing is solved: it is not a
//! pflow proof (a compiled part is a one-token ring whose invariants hold for every program and say nothing about which
//! transitions carry a snare), and it does not claim to be one. A false one refuses the program like every static `assert`.
//!
//! * `beside`: no step where one subject has an onset and the other has one on the next or the previous step, across bar lines and
//!   section seams (and the seam of a `loop`). Two onsets on one step are together, not beside.
//! * `during`: no step where a note of one subject sounds while a note of the other does. A note sounds from its step for its
//!   `len=` (a sixteenth for a drum), and is cut where its section ends. `sing` is the vocal's windows: every note of every `sing`
//!   line, from its onset to where the voice lets go of it, round and round the line's bars through its section.
//! * `ends on D`: in each place a section plays (or `in NAME`), the lowest note of the last onset of the part is degree `D` (1 to 7)
//!   of the section's own key.
//!
//! A part with chance is judged on what it **may** play ([`crate::compile::MayGrid`]: every `?` hit kept, every option of a
//! `choose` at once), so the rule holds for every roll and not for the one the text happens to roll. The vocal is judged on the
//! notes the part plays. Pure like the rest of the crate: no clock, no I/O, no render, the same verdict on every host.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::assertion::{no_section, AssertKind};
use crate::ast::{Deck, Word};
use crate::compile::{Compiled, DeckIr, PartIr};
use crate::diag::{line_col, Diagnostic, Span};
use crate::lex::Tok;
use crate::music::Key;
use crate::parse::{nearest, Parser, ROLES};

type R<T> = Result<T, Diagnostic>;

/// The most bars of a program with no end an assertion walks (a program with sections and an `arrange` has its own length).
pub const MAX_BARS: u64 = 4096;
/// The parts that hold pitches: what `ends on` can be asked of.
pub const PITCHED: [&str; 4] = ["bass", "melody", "harmony", "arp"];
/// How many places a message lists before it says "and N more".
const LISTED: usize = 4;

const USAGE: &str = "assert never A.snare beside A.snare   assert never B during sing   assert bass ends on 1";

// ---------------------------------------------------------------------------------------------------------------------
// the tree
// ---------------------------------------------------------------------------------------------------------------------

/// What a constraint is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Who {
    /// `sing`: the vocal's lines (a time, not a part).
    Sing,
    /// `master`: read, so that the checker can say it has no parts.
    Master,
    /// `A`: every part of the deck.
    Deck(Deck),
    /// `bass`: that role on every deck.
    Role(String),
    /// `A.snare`.
    DeckRole(Deck, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    pub who: Who,
    pub span: Span,
}

impl Subject {
    /// As written: `sing`, `A`, `bass`, `A.snare`.
    pub fn text(&self) -> String {
        match &self.who {
            Who::Sing => "sing".into(),
            Who::Master => "master".into(),
            Who::Deck(d) => d.name().into(),
            Who::Role(r) => r.clone(),
            Who::DeckRole(d, r) => format!("{}.{r}", d.name()),
        }
    }

    /// Does the part `role` of `deck` belong to it?
    fn has(&self, deck: Deck, role: &str) -> bool {
        match &self.who {
            Who::Deck(d) => *d == deck,
            Who::Role(r) => r == role,
            Who::DeckRole(d, r) => *d == deck && r == role,
            Who::Sing | Who::Master => false,
        }
    }

    /// How a message calls it: `the snare` / `deck A's snare` / `deck A`.
    fn called(&self) -> String {
        match &self.who {
            Who::Deck(d) => format!("deck {}", d.name()),
            Who::DeckRole(d, r) => format!("deck {}'s {r}", d.name()),
            Who::Role(r) => format!("the {r}"),
            Who::Sing => "the vocal".into(),
            Who::Master => "the master".into(),
        }
    }
}

/// How two subjects meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Beside,
    During,
}

impl Relation {
    pub fn word(self) -> &'static str {
        match self {
            Relation::Beside => "beside",
            Relation::During => "during",
        }
    }
}

fn tail(section: &Option<Word>) -> String {
    section.as_ref().map_or(String::new(), |w| format!(" in {}", w.text))
}

/// The canonical text of `assert never X beside Y [in S]`.
pub fn never_text(x: &Subject, rel: Relation, y: &Subject, section: &Option<Word>) -> String {
    format!("assert never {} {} {}{}", x.text(), rel.word(), y.text(), tail(section))
}

/// The canonical text of `assert X ends on D [in S]`.
pub fn ends_on_text(part: &Subject, degree: u32, section: &Option<Word>) -> String {
    format!("assert {} ends on {degree}{}", part.text(), tail(section))
}

// ---------------------------------------------------------------------------------------------------------------------
// reading: the one place the syntax is written
// ---------------------------------------------------------------------------------------------------------------------

/// Does a word after `assert` begin a constraint's subject?
pub fn starts_subject(w: &str) -> bool {
    matches!(w, "A" | "B" | "master" | "sing") || ROLES.contains(&w)
}

/// The words `assert` may be followed by that a mistyped one could have been meant as: the constraints' too.
pub fn first_words() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = vec!["never"];
    v.extend(ROLES);
    v.extend(["A", "B", "master", "sing"]);
    v
}

impl Parser<'_> {
    /// `assert never SUBJECT (beside | during) SUBJECT [in NAME]` (`never` is the next token).
    pub(crate) fn assert_never(&mut self) -> R<AssertKind> {
        self.bump();
        let x = self.subject("what must never happen: a part like `A.snare` or `bass`, or a deck")?;
        let rel = match self.peek().clone() {
            Tok::Word(w) if w == "beside" => Relation::Beside,
            Tok::Word(w) if w == "during" => Relation::During,
            other => {
                let mut d = self.expected(&["`beside`", "`during`"], &format!("`never {}` is followed by `beside` or `during`, found {}", x.text(), self.found()));
                d = match &other {
                    Tok::Word(w) => match nearest(w, ["beside", "during"]) {
                        Some(n) => d.help(format!("did you mean `{n}`?")),
                        None => d.help(USAGE),
                    },
                    _ => d.help(USAGE),
                };
                return Err(d);
            }
        };
        self.bump();
        let y = self.subject(if rel == Relation::During { "what it must not sound with: a part, a deck or `sing`" } else { "the part it must not touch" })?;
        let section = self.in_section()?;
        Ok(AssertKind::Never { x, rel, y, section })
    }

    /// `assert SUBJECT ends on DEGREE [in NAME]` (the subject is the next token).
    pub(crate) fn assert_ends_on(&mut self) -> R<AssertKind> {
        let part = self.subject("a part")?;
        if !self.word_is("ends") {
            let mut d = self.expected(&["`ends`"], &format!("`assert {}` is followed by `ends on`, found {}", part.text(), self.found()));
            if let Tok::Word(w) = self.peek().clone() {
                if let Some(n) = nearest(&w, ["ends"]) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
            }
            return Err(if d.help.is_some() { d } else { d.help(format!("assert {} ends on 1", part.text())) });
        }
        self.bump();
        if !self.word_is("on") {
            return Err(self.expected(&["`on`"], &format!("`ends` is followed by `on` and a degree, found {}", self.found())).help(format!("assert {} ends on 1", part.text())));
        }
        self.bump();
        let (degree, degree_span) = self.int("a degree of the key, 1 to 7").map_err(|d| {
            if d.help.is_none() {
                d.help("a degree is a number, 1 to 7: the tonic is 1, the fifth is 5 (a chord numeral like `i` is not one)")
            } else {
                d
            }
        })?;
        let section = self.in_section()?;
        Ok(AssertKind::EndsOn { part, degree, degree_span, section })
    }

    /// `sing`, `A`, `master`, `A.snare` or `snare`.
    fn subject(&mut self, what: &str) -> R<Subject> {
        let start = self.span();
        let deck_of = |w: &str| if w == "A" { Deck::A } else { Deck::B };
        match self.peek().clone() {
            Tok::Word(w) if w == "sing" => {
                self.bump();
                Ok(Subject { who: Who::Sing, span: start })
            }
            Tok::Word(w) if w == "master" => {
                self.bump();
                if self.peek() == &Tok::Dot {
                    return Err(Diagnostic::error(start.to(self.span()), "`master` has no parts: it is the mix of both decks").help("a part belongs to a deck: `A.snare`, `B.bass`"));
                }
                Ok(Subject { who: Who::Master, span: start })
            }
            Tok::Word(w) if w == "A" || w == "B" => {
                self.bump();
                if self.peek() != &Tok::Dot {
                    return Ok(Subject { who: Who::Deck(deck_of(&w)), span: start });
                }
                self.bump();
                let role = match self.peek().clone() {
                    Tok::Word(r) if ROLES.contains(&r.as_str()) => {
                        self.bump();
                        r
                    }
                    Tok::Word(r) => {
                        let mut d = Diagnostic::error(self.span(), format!("`{r}` is not a part")).expected(&ROLES.map(|r| r)).found(format!("`{r}`"));
                        if let Some(n) = nearest(&r, ROLES) {
                            d = d.help(format!("did you mean `{n}`?"));
                        }
                        return Err(d);
                    }
                    _ => return Err(self.expected(&ROLES.map(|r| r), &format!("expected a part after `{w}.`, found {}", self.found()))),
                };
                Ok(Subject { who: Who::DeckRole(deck_of(&w), role), span: start.to(self.prev_span()) })
            }
            Tok::Word(w) if ROLES.contains(&w.as_str()) => {
                self.bump();
                Ok(Subject { who: Who::Role(w), span: start })
            }
            Tok::Word(w) => {
                let mut d = self.expected(&["a part", "`A`", "`B`", "`sing`"], &format!("expected {what}, found `{w}`"));
                if w.eq_ignore_ascii_case("a") || w.eq_ignore_ascii_case("b") {
                    d = d.help(format!("decks are upper case: `{}`", w.to_uppercase()));
                } else if let Some(n) = nearest(&w, first_words().into_iter().filter(|x| *x != "never")) {
                    d = d.help(format!("did you mean `{n}`?"));
                } else {
                    d = d.help(USAGE);
                }
                Err(d)
            }
            _ => Err(self.expected(&["a part", "`A`", "`B`", "`sing`"], &format!("expected {what}, found {}", self.found())).help(USAGE)),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the timeline
// ---------------------------------------------------------------------------------------------------------------------

/// One stretch of the judged timeline.
struct TSeg<'a> {
    /// Its section's name (empty: a program with no sections).
    name: &'a str,
    start: u32,
    bars: u32,
    decks: &'a BTreeMap<Deck, DeckIr>,
    key: Key,
    /// It goes on from the stretch before it: the same section (the next generation of an evolve, the next net of a long one).
    chain: bool,
}

/// The bars a constraint is asked of, in order, and how the end turns (a `loop` goes back to `wrap`; a program with no end is
/// its own period, round and round).
struct Tl<'a> {
    segs: Vec<TSeg<'a>>,
    bars: u32,
    periodic: bool,
    wrap: u32,
}

impl Tl<'_> {
    fn steps(&self) -> u64 {
        u64::from(self.bars) * 16
    }

    /// The step after `g`, and the step before it (a loop's seam: the last step is before the first of the lap).
    fn next(&self, g: u64) -> Option<u64> {
        if g + 1 < self.steps() {
            Some(g + 1)
        } else if self.periodic && self.steps() > 0 {
            Some(u64::from(self.wrap) * 16)
        } else {
            None
        }
    }

    fn prev(&self, g: u64) -> Vec<u64> {
        let mut v = Vec::new();
        if g > 0 {
            v.push(g - 1);
        }
        if self.periodic && g == u64::from(self.wrap) * 16 && self.steps() > 0 {
            v.push(self.steps() - 1);
        }
        v
    }

    fn seg_at(&self, bar: u32) -> usize {
        self.segs.iter().rposition(|s| s.start <= bar).unwrap_or(0)
    }
}

/// The timeline of a compiled program, or why it cannot be decided.
fn timeline(c: &Compiled) -> Result<Tl<'_>, String> {
    if c.arranged {
        let song = c.song_bars().unwrap_or(0);
        let total = c.end.unwrap_or(song);
        let mut segs = Vec::new();
        for (i, s) in c.segments.iter().enumerate() {
            if s.start >= total {
                break;
            }
            // the song's last section goes on past its end, up to an `end at bar N` that is later
            let to = if i + 1 == c.segments.len() { (s.start + s.bars).max(total) } else { s.start + s.bars };
            segs.push(TSeg { name: &s.name, start: s.start, bars: to.min(total) - s.start, decks: &s.decks, key: c.key_in(s), chain: s.chain });
        }
        let periodic = c.looped && c.end.is_none();
        let wrap = if c.loop_from < total { c.loop_from } else { 0 };
        return Ok(Tl { segs, bars: total, periodic, wrap });
    }
    // no sections: the decks' nets, each round and round, so one lap of them all together
    let bars = match c.end {
        Some(e) => u64::from(e),
        None => {
            let mut b = 1u64;
            for p in c.decks.values().map(|ir| ir.period.max(1) as u64).chain(c.sing.iter().filter(|l| l.start.is_some()).map(|l| u64::from(l.cover))) {
                b = crate::meter::lcm(b, p).unwrap_or(u64::MAX);
            }
            b
        }
    };
    if bars > MAX_BARS {
        return Err(format!("this program takes {bars} bars to come round, and an assertion walks at most {MAX_BARS}"));
    }
    let key = c.key.unwrap_or(Key::C_MAJOR);
    let bars = bars as u32;
    Ok(Tl { segs: vec![TSeg { name: "", start: 0, bars, decks: &c.decks, key, chain: false }], bars, periodic: c.end.is_none(), wrap: 0 })
}

/// A note of a part (or of the vocal) on the timeline.
#[derive(Clone)]
struct Ev<'a> {
    /// The step, counted from the first bar.
    at: u64,
    /// How many steps it sounds: its `len=`, cut where its section ends.
    len: u32,
    note: u8,
    deck: Deck,
    role: &'a str,
    /// The part (and its place among the deck's parts) or the `sing` line it comes from.
    part: Option<&'a PartIr>,
    pidx: usize,
    vocal: Option<usize>,
    seg: usize,
    /// The part plays it in the text as rolled; false: only a chance could.
    rolled: bool,
    sid: u32,
}

impl Ev<'_> {
    fn id(&self) -> (u8, usize, Option<usize>, u64, u8) {
        (self.deck as u8, self.pidx, self.vocal, self.at, self.note)
    }

    fn bar(&self) -> u32 {
        (self.at / 16) as u32
    }

    fn step(&self) -> u32 {
        (self.at % 16) as u32
    }
}

/// Does the stretch named `seg` play the section `name`? A stack is `a+b`, and a section an arrange mutation plays in another
/// generation is a copy of its own (`drop@2`, mutate.rs): the section the person wrote is the part before the `@`.
fn plays_section(seg: &str, name: &str) -> bool {
    seg.split('+').any(|n| n.split('@').next() == Some(name))
}

/// A stretch's name as the person wrote it: `drop@2` is `drop`.
fn shown(seg: &str) -> String {
    seg.split('+').map(|n| n.split('@').next().unwrap_or(n)).collect::<Vec<_>>().join("+")
}

/// A part the program wrote: the voice a host adds for a sung line (`vox`) is not one.
fn written(p: &PartIr) -> bool {
    ROLES.contains(&p.role.as_str())
}

/// Every note of the parts a subject names, on the whole timeline; a part with chance gives everything it may play.
fn part_events<'a>(tl: &Tl<'a>, who: &Subject) -> Vec<Ev<'a>> {
    let mut out = Vec::new();
    for (si, seg) in tl.segs.iter().enumerate() {
        let seg_end = u64::from(seg.start + seg.bars) * 16;
        for (deck, ir) in seg.decks {
            for (pidx, p) in ir.parts.iter().enumerate() {
                if !written(p) || !who.has(*deck, &p.role) {
                    continue;
                }
                let grid = if p.may.0.is_empty() { &p.cycles } else { &p.may.0 };
                let n = grid.len().max(1);
                for local in 0..seg.bars {
                    let Some(hits) = grid.get(local as usize % n) else { continue };
                    let rolled = p.cycles.get(local as usize % p.cycles.len().max(1));
                    for h in hits {
                        let at = u64::from(seg.start + local) * 16 + u64::from(h.step);
                        let len = u64::from(h.dur.max(1)).min(seg_end - at) as u32;
                        let rolled = rolled.is_none_or(|r| r.iter().any(|x| x.step == h.step && x.note == h.note));
                        out.push(Ev { at, len, note: h.note, deck: *deck, role: &p.role, part: Some(p), pidx, vocal: None, seg: si, rolled, sid: h.sid });
                    }
                }
            }
        }
    }
    out.sort_by_key(|e| (e.at, e.deck as u8, e.pidx, e.note));
    out
}

/// The vocal as notes: every note of every `sing` line, round and round the line's bars until its section ends.
fn sing_events<'a>(tl: &Tl<'a>, c: &'a Compiled) -> Vec<Ev<'a>> {
    let mut out = Vec::new();
    let end = tl.steps();
    for (li, l) in c.sing.iter().enumerate() {
        let Some(start) = l.start else { continue };
        let until = l.until.map_or(end, |u| (u64::from(u) * 16).min(end));
        let mut pass = u64::from(start) * 16;
        while pass < until {
            for (s, len) in &l.notes {
                let at = pass + u64::from(*s);
                if at >= until {
                    continue;
                }
                let len = u64::from(*len).min(until - at) as u32;
                out.push(Ev { at, len, note: 0, deck: l.deck, role: "vocal", part: None, pidx: usize::MAX, vocal: Some(li), seg: tl.seg_at((at / 16) as u32), rolled: true, sid: u32::MAX });
            }
            pass += u64::from(l.cover) * 16;
        }
    }
    out.sort_by_key(|e| (e.at, e.vocal));
    out
}

/// The bars of the timeline where the section `name` plays.
fn bars_of(tl: &Tl, name: &str) -> Vec<bool> {
    let mut v = vec![false; tl.bars as usize];
    for s in tl.segs.iter().filter(|s| plays_section(s.name, name)) {
        for b in s.start..s.start + s.bars {
            v[b as usize] = true;
        }
    }
    v
}

// ---------------------------------------------------------------------------------------------------------------------
// what a verdict is
// ---------------------------------------------------------------------------------------------------------------------

/// What judging a constraint came to.
pub enum Judged {
    /// It holds: what was checked.
    Held(String),
    /// It does not: what happened and where, and what would fix it.
    Broken { message: String, help: String },
    /// It cannot be judged (it names nothing, or says something that makes no sense, or the program cannot be walked): an error
    /// whether or not false statics refuse the program.
    Refused(Diagnostic),
}

const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

fn pitch_name(midi: u8) -> String {
    format!("{}{}", SHARP[usize::from(midi % 12)], i32::from(midi / 12) - 1)
}

fn bar_step(at: u64) -> String {
    format!("bar {}, step {}", at / 16, at % 16)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The line of the text a hit came from (the part's line, where the call or the `let` that made it is not what is wanted).
fn line_of(c: &Compiled, e: &Ev) -> Option<usize> {
    let p = e.part?;
    let chain = p.spans.get(e.sid as usize)?;
    let line = chain.get(chain.len().checked_sub(1 + p.via.len())?)?;
    (line.end <= c.source.len()).then(|| line_col(&c.source, line.start).0)
}

/// The chance term a hit that only a chance could play comes from (`x?0.5`, `choose [..] ..`), and where it is.
fn chance_of(c: &Compiled, e: &Ev) -> Option<String> {
    let p = e.part?;
    let chain = p.spans.get(e.sid as usize)?;
    chain.iter().take(chain.len().saturating_sub(2 + p.via.len())).find_map(|s| {
        let t = s.text(&c.source);
        let own = t.contains('?') || t.starts_with("choose") || t.starts_with("maybe(");
        own.then(|| {
            let (line, col) = line_col(&c.source, s.start);
            let shown: String = t.chars().take(28).collect();
            format!("`{shown}` at {line}:{col}")
        })
    })
}

/// One bar of a part as a person writes it: `x ~ ~ ~ x ~ x ~ ...`, a note's name for a pitched part, `?` after a hit only a
/// chance could play.
fn bar_of(p: &PartIr, local: u32) -> String {
    let grid = if p.may.0.is_empty() { &p.cycles } else { &p.may.0 };
    let hits = grid.get(local as usize % grid.len().max(1)).map_or(&[][..], Vec::as_slice);
    let rolled = p.cycles.get(local as usize % p.cycles.len().max(1));
    let drum = matches!(p.role.as_str(), "kick" | "snare" | "hihat" | "clap");
    (0..16u8)
        .map(|s| {
            let at: Vec<&crate::compile::Hit> = hits.iter().filter(|h| h.step == s).collect();
            if at.is_empty() {
                return "~".to_string();
            }
            let maybe = !p.may.0.is_empty() && !rolled.is_none_or(|r| at.iter().all(|h| r.iter().any(|x| x.step == h.step && x.note == h.note)));
            let name = if drum { "x".to_string() } else { at.iter().map(|h| pitch_name(h.note)).collect::<Vec<_>>().join(",") };
            let name = if at.len() > 1 && !drum { format!("[{name}]") } else { name };
            if maybe {
                format!("{name}?")
            } else {
                name
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "bars 5, 6, 7" for the places a violation comes up again.
fn also(bars: &[u32]) -> String {
    let shown: Vec<String> = bars.iter().take(LISTED).map(u32::to_string).collect();
    let more = bars.len().saturating_sub(LISTED);
    if bars.is_empty() {
        String::new()
    } else if more > 0 {
        format!(" It comes up again in {} {} and {more} more.", if bars.len() == 1 { "bar" } else { "bars" }, shown.join(", "))
    } else {
        format!(" It comes up again in {} {}.", if bars.len() == 1 { "bar" } else { "bars" }, shown.join(", "))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// never
// ---------------------------------------------------------------------------------------------------------------------

/// Two notes that meet: one of each subject.
struct Meet<'a> {
    /// The step where they meet (for `during`: where both first sound).
    at: u64,
    x: Ev<'a>,
    y: Ev<'a>,
}

type Id = (u8, usize, Option<usize>, u64, u8);

/// Misuse that no timeline could make right, in the language's terms.
fn misuse_never(c: &Compiled, x: &Subject, rel: Relation, y: &Subject) -> Option<Diagnostic> {
    for s in [x, y] {
        if s.who == Who::Master {
            return Some(Diagnostic::error(s.span, "`master` has no parts: it is the mix of both decks").help("a rule about notes names a part or a deck: `A.snare`, `B`; `assert level master < ...` reads the mix"));
        }
    }
    if x.who == Who::Sing {
        return Some(Diagnostic::error(x.span, "`sing` is a time, not a part: it goes after `during`").help("assert never B during sing"));
    }
    if y.who != Who::Sing {
        return None;
    }
    if rel == Relation::Beside {
        return Some(Diagnostic::error(y.span, "`sing` is a time, not a part: `beside` needs a part, and `during` is the word for the vocal").help("assert never B during sing"));
    }
    if c.sing.is_empty() {
        return Some(
            Diagnostic::error(y.span, "there is no `sing` line in this program, so nothing sings")
                .help("write one first: `deck A: sing \"hold the lantern\" on melody`; or name a part or a deck after `during`"),
        );
    }
    if let Some(l) = c.sing.iter().find(|l| l.start.is_none()) {
        let (line, _) = line_col(&c.source, l.span.start);
        return Some(
            Diagnostic::error(y.span, format!("cannot tell when the vocal sings: the `sing` on line {line} is not at a known time ({})", l.why.as_deref().unwrap_or("its time is not known")))
                .help("give that line no time word, or `at bar N`: it then sings through its section"),
        );
    }
    if let Some(l) = c.sing.iter().find(|l| x.has(l.deck, &l.role)) {
        let (line, _) = line_col(&c.source, l.span.start);
        let other = if l.deck == Deck::A { "B" } else { "A" };
        return Some(
            Diagnostic::error(x.span, format!("the vocal is sung on deck {}'s own `{}` (the `sing` on line {line}), so {} sounds whenever it sings", l.deck.name(), l.role, x.called()))
                .help(format!("name the other deck or another part: `assert never {other} during sing`")),
        );
    }
    None
}

/// Every pair of onsets of `xs` and `ys` on neighbouring steps.
fn meetings_beside<'a>(tl: &Tl, xs: &[Ev<'a>], ys: &[Ev<'a>], inset: Option<&[bool]>) -> Vec<Meet<'a>> {
    let ok = |at: u64| inset.is_none_or(|s| s[(at / 16) as usize]);
    let mut onsets: HashMap<u64, Vec<usize>> = HashMap::new();
    for (j, y) in ys.iter().enumerate() {
        onsets.entry(y.at).or_default().push(j);
    }
    let mut seen: HashSet<(Id, Id)> = HashSet::new();
    let mut out = Vec::new();
    for x in xs.iter().filter(|x| ok(x.at)) {
        let mut near = tl.prev(x.at);
        near.extend(tl.next(x.at));
        for g in near.into_iter().filter(|g| ok(*g)) {
            for &j in onsets.get(&g).into_iter().flatten() {
                let y = &ys[j];
                let (a, b) = (x.id(), y.id());
                if seen.insert((a.min(b), a.max(b))) {
                    out.push(Meet { at: x.at.min(y.at), x: x.clone(), y: y.clone() });
                }
            }
        }
    }
    out.sort_by_key(|m| (m.at, m.x.at.max(m.y.at)));
    out
}

/// Every pair of notes of `xs` and `ys` that sound on one step (never a note with itself).
fn meetings_during<'a>(xs: &[Ev<'a>], ys: &[Ev<'a>], inset: Option<&[bool]>) -> Vec<Meet<'a>> {
    let ok = |at: u64| inset.is_none_or(|s| s[(at / 16) as usize]);
    let mut sounding: HashMap<u64, Vec<usize>> = HashMap::new();
    for (j, y) in ys.iter().enumerate() {
        for s in y.at..y.at + u64::from(y.len) {
            sounding.entry(s).or_default().push(j);
        }
    }
    let mut seen: HashSet<(Id, Id)> = HashSet::new();
    let mut out = Vec::new();
    for x in xs {
        for s in (x.at..x.at + u64::from(x.len)).filter(|s| ok(*s)) {
            for &j in sounding.get(&s).into_iter().flatten() {
                let y = &ys[j];
                let (a, b) = (x.id(), y.id());
                if a != b && seen.insert((a.min(b), a.max(b))) {
                    out.push(Meet { at: s, x: x.clone(), y: y.clone() });
                }
            }
        }
    }
    out.sort_by_key(|m| (m.at, m.x.at.max(m.y.at)));
    out
}

/// Judge `assert never X beside|during Y [in S]`.
pub fn judge_never(c: &Compiled, text: &str, at: Span, x: &Subject, rel: Relation, y: &Subject, section: &Option<Word>) -> (Judged, Vec<Diagnostic>) {
    let mut warns = Vec::new();
    if let Some(d) = misuse_never(c, x, rel, y) {
        return (Judged::Refused(d), warns);
    }
    let tl = match timeline(c) {
        Ok(t) => t,
        Err(why) => return (Judged::Refused(Diagnostic::error(at, format!("cannot tell whether this holds: {why}")).help("give the program sections and an `arrange`, or `end at bar N`")), warns),
    };
    let inset = match section {
        Some(w) => {
            if !c.segments.iter().any(|s| plays_section(&s.name, &w.text)) {
                return (Judged::Refused(no_section(c, w)), warns);
            }
            Some(bars_of(&tl, &w.text))
        }
        None => None,
    };
    let xs = part_events(&tl, x);
    let ys = if y.who == Who::Sing { sing_events(&tl, c) } else { part_events(&tl, y) };
    // a rule that holds because nothing plays is a silent failure: say it
    let silent = |s: &Subject, evs: &[Ev]| {
        let n = inset.as_ref().map_or(evs.len(), |set| evs.iter().filter(|e| set[e.bar() as usize]).count());
        (n == 0).then(|| {
            let where_ = section.as_ref().map_or("this program".to_string(), |w| format!("`{}`", w.text));
            let who = if s.who == Who::Sing { "the vocal".to_string() } else { format!("`{}`", s.text()) };
            Diagnostic::warning(at, format!("{text}: {who} plays no note in {where_}, so there is nothing for the rule to hold against"))
                .help("check the name: a part belongs to a deck (`A.snare`), and plays only where a section or the song writes it")
        })
    };
    warns.extend(silent(x, &xs));
    if y.who != x.who {
        warns.extend(silent(y, &ys));
    }
    let found = match rel {
        Relation::Beside => meetings_beside(&tl, &xs, &ys, inset.as_deref()),
        Relation::During => meetings_during(&xs, &ys, inset.as_deref()),
    };
    let Some(first) = found.first() else {
        let bars = inset.as_ref().map_or(tl.bars as usize, |s| s.iter().filter(|b| **b).count());
        let over = match section {
            Some(w) => format!("the {} of `{}`", plural(bars, "bar", "bars"), w.text),
            None => plural(bars, "bar", "bars"),
        };
        let counted = if xs.iter().chain(&ys).any(|e| e.part.is_some_and(|p| !p.may.0.is_empty())) { ", counting every chance hit as played" } else { "" };
        return (Judged::Held(format!("held over {over}{counted}")), warns);
    };
    let (message, help) = describe_never(c, &tl, x, rel, y, first, &found, &ys);
    (Judged::Broken { message, help }, warns)
}

/// `` `snare` on deck A``, `the vocal`: what a hit is, as a noun.
fn what(e: &Ev) -> String {
    match e.vocal {
        Some(_) => "the vocal".to_string(),
        None => format!("`{}` on deck {}", e.role, e.deck.name()),
    }
}

/// `deck B plays `bass` (e2)` (`can play` where only a chance could), `the vocal sings`: what a hit does.
fn plays(e: &Ev, can: bool) -> String {
    match e.vocal {
        Some(_) => if can { "the vocal can sing" } else { "the vocal sings" }.to_string(),
        None => {
            let note = if PITCHED.contains(&e.role) { format!(" ({})", pitch_name(e.note)) } else { String::new() };
            format!("deck {} {} `{}`{note}", e.deck.name(), if can { "can play" } else { "plays" }, e.role)
        }
    }
}

/// The first step at or after `from` that no window of `ys` covers.
fn free_from(ys: &[Ev], from: u64) -> u64 {
    let mut s = from;
    while ys.iter().any(|y| y.at <= s && s < y.at + u64::from(y.len)) && s < from + 4096 {
        s += 1;
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn describe_never(c: &Compiled, tl: &Tl, x: &Subject, rel: Relation, y: &Subject, first: &Meet, all: &[Meet], ys: &[Ev]) -> (String, String) {
    let (a, b) = if first.x.at <= first.y.at { (&first.x, &first.y) } else { (&first.y, &first.x) };
    let (sa, sb) = (shown(tl.segs[a.seg].name), shown(tl.segs[b.seg].name));
    let seg = shown(tl.segs[tl.seg_at((first.at / 16) as u32)].name);
    let may = !(a.rolled && b.rolled);
    let can = if may { "can " } else { "" };
    let vocal = y.who == Who::Sing;
    // (two notes on either side of a seam: from the section the first is in into the one the second is in)
    let section = match (sa.as_str(), sb.as_str()) {
        ("", _) | (_, "") => None,
        (x, y) if x != y && rel == Relation::Beside => Some(format!("from `{x}` into `{y}`")),
        _ => Some(format!("in `{seg}`")),
    };
    let message = match (rel, vocal) {
        (Relation::Beside, _) => {
            // (the last step of a looping song is beside its first: the two are not neighbours in the text of the song)
            let steps = if b.at != a.at + 1 {
                format!("{} and {}, where the song starts over", bar_step(b.at), bar_step(a.at))
            } else if a.bar() == b.bar() {
                format!("bar {}, steps {} and {}", a.bar(), a.step(), b.step())
            } else {
                format!("{} and {}", bar_step(a.at), bar_step(b.at))
            };
            let who = if a.deck == b.deck && a.role == b.role { format!("two `{}` hits {can}touch on deck {}", a.role, a.deck.name()) } else { format!("{} and {} {can}touch", what(a), what(b)) };
            [Some(who), section, Some(steps)].into_iter().flatten().collect::<Vec<_>>().join(", ")
        }
        (Relation::During, true) => [Some(format!("{} at {}, while the vocal sings", plays(&first.x, may), bar_step(first.at))), section].into_iter().flatten().collect::<Vec<_>>().join(" "),
        (Relation::During, false) => [Some(format!("{} while {}, at {}", plays(&first.x, may), plays(&first.y, may), bar_step(first.at))), section].into_iter().flatten().collect::<Vec<_>>().join(" "),
    };
    // help: the bar as written, where it comes from, how long it lasts, where it comes up again, what would do
    let mut help: Vec<String> = Vec::new();
    if let Some(e) = [&first.x, &first.y].into_iter().find(|e| e.part.is_some()) {
        let p = e.part.expect("found for having a part");
        let line = line_of(c, e).map_or(String::new(), |l| format!(" (line {l})"));
        let own = format!("deck {}'s `{}`", e.deck.name(), e.role);
        help.push(format!("Bar {} of {own}{} is `{}`{line}.", e.bar(), if seg.is_empty() { String::new() } else { format!(" in `{seg}`") }, bar_of(p, e.bar() - tl.segs[e.seg].start)));
    }
    for e in [a, b].into_iter().filter(|e| !e.rolled) {
        if let Some(t) = chance_of(c, e) {
            help.push(format!("The {} hit at {} is kept only by chance: {t}.", e.role, bar_step(e.at)));
        }
    }
    if rel == Relation::During {
        let last = |e: &Ev| bar_step(e.at + u64::from(e.len) - 1);
        match (vocal, first.y.vocal) {
            (true, Some(l)) => {
                let (line, _) = line_col(&c.source, c.sing[l].span.start);
                let free = free_from(ys, first.y.at);
                help.push(format!("The `sing` on line {line} holds a note from {} to {}; {} is clear of the vocal from {}.", bar_step(first.y.at), last(&first.y), x.called(), bar_step(free)));
            }
            _ => help.push(format!("{} sounds from {} to {}; {} from {} to {}.", what(&first.x), bar_step(first.x.at), last(&first.x), what(&first.y), bar_step(first.y.at), last(&first.y))),
        }
    }
    let mut again: Vec<u32> = all.iter().map(|m| (m.at / 16) as u32).filter(|bar| *bar != (first.at / 16) as u32).collect();
    again.dedup();
    let again = also(&again);
    let fix = match (rel, vocal) {
        (Relation::Beside, _) => "move or drop one of the two hits".to_string(),
        (Relation::During, true) => format!("move the note to where the vocal rests, or give {} a rest there", x.called()),
        (Relation::During, false) => "move one of the two notes, or shorten one with `len=`".to_string(),
    };
    (message, format!("{}{} To fix it: {fix}.", help.join(" "), again).trim().to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// ends on
// ---------------------------------------------------------------------------------------------------------------------

/// The degree a pitch class is in a key, None when it is no note of the key.
fn degree_of(key: Key, pc: u8) -> Option<usize> {
    (1..=key.intervals().len()).find(|d| key.degree(*d as u32, 4).rem_euclid(12) == i32::from(pc))
}

/// One place a section plays: the bars from `lo` to `hi` (exclusive), and the stretch its last bar is in.
struct Place {
    lo: u32,
    hi: u32,
    last: usize,
}

/// The places a section plays, or the section `name` does: runs of stretches that are one section.
fn places(tl: &Tl, name: Option<&str>) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    let mut open = false;
    for (i, s) in tl.segs.iter().enumerate() {
        let belongs = name.is_none_or(|n| plays_section(s.name, n));
        match out.last_mut() {
            Some(p) if open && belongs && s.chain => {
                p.hi = s.start + s.bars;
                p.last = i;
            }
            _ if belongs => out.push(Place { lo: s.start, hi: s.start + s.bars, last: i }),
            _ => {}
        }
        open = belongs;
    }
    out
}

/// Judge `assert X ends on D [in S]`.
pub fn judge_ends_on(c: &Compiled, at: Span, part: &Subject, degree: u32, degree_span: Span, section: &Option<Word>) -> (Judged, Vec<Diagnostic>) {
    let refused = |d: Diagnostic| (Judged::Refused(d), Vec::new());
    match &part.who {
        Who::Master => return refused(Diagnostic::error(part.span, "`master` has no parts: it is the mix of both decks").help("`ends on` is about a pitched part: `bass`, `A.melody`")),
        Who::Sing => return refused(Diagnostic::error(part.span, "the vocal has no notes of its own to end on: it sings the notes of a part").help("name the part it is sung on: `assert melody ends on 1`")),
        Who::Deck(d) => return refused(Diagnostic::error(part.span, format!("deck {} has several parts: `ends on` is about one of them", d.name())).help(format!("say which: `assert {}.bass ends on {degree}`", d.name()))),
        Who::Role(r) | Who::DeckRole(_, r) if !PITCHED.contains(&r.as_str()) => {
            return refused(Diagnostic::error(part.span, format!("the {r} has no pitch: `ends on` is for bass, melody, harmony and arp")).help("a drum part is judged with `assert never ... beside ...` or `during`"));
        }
        _ => {}
    }
    if !(1..=7).contains(&degree) {
        return refused(Diagnostic::error(degree_span, "a degree is 1 to 7").help("the tonic is 1 and the fifth is 5, whatever the mode: `assert bass ends on 1`"));
    }
    let tl = match timeline(c) {
        Ok(t) => t,
        Err(why) => return refused(Diagnostic::error(at, format!("cannot tell whether this holds: {why}")).help("give the program sections and an `arrange`, or `end at bar N`")),
    };
    if let Some(w) = section {
        if !c.segments.iter().any(|s| plays_section(&s.name, &w.text)) {
            return refused(no_section(c, w));
        }
    }
    // which note is last must not depend on the dice
    let dicey = tl.segs.iter().flat_map(|s| s.decks.iter()).flat_map(|(d, ir)| ir.parts.iter().map(move |p| (*d, p))).find(|(d, p)| written(p) && part.has(*d, &p.role) && p.chance);
    if let Some((deck, p)) = dicey {
        let line = p.spans.first().and_then(|ch| ch.last()).map_or(String::new(), |s| format!(" (line {})", line_col(&c.source, s.start).0));
        return refused(Diagnostic::error(part.span, format!("which note the {} of deck {} ends on depends on the dice: it has chance in it{line}", p.role, deck.name())).help("put the closing note outside the chance, or ask about a part that has none"));
    }
    let evs = part_events(&tl, part);
    let who = part.called();
    // for each place: the lowest note of the part's last onset in it, and the key it is in
    let mut landed: Vec<(String, u64, u8, Key)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for pl in places(&tl, section.as_ref().map(|w| w.text.as_str())) {
        // (the section asked for by name, else the stretch the place ends in: a stack `long+short` is `short` where `in short` says so)
        let name = match (section, shown(tl.segs[pl.last].name).as_str()) {
            (Some(w), _) => format!("`{}`", w.text),
            (None, "") => "the program".to_string(),
            (None, n) => format!("`{n}`"),
        };
        let inside: Vec<&Ev> = evs.iter().filter(|e| e.bar() >= pl.lo && e.bar() < pl.hi).collect();
        let Some(last) = inside.iter().map(|e| e.at).max() else {
            skipped.push(name);
            continue;
        };
        let low = inside.iter().filter(|e| e.at == last).min_by_key(|e| e.note).expect("a last onset has a note");
        landed.push((name, last, low.note, tl.segs[low.seg].key));
    }
    let mut once: Vec<String> = Vec::new();
    for n in skipped {
        if !once.contains(&n) {
            once.push(n);
        }
    }
    let skipped = once;
    let omitted = if skipped.is_empty() { String::new() } else { format!(" (not judged where {who} plays no note: {})", skipped.join(", ")) };
    if landed.is_empty() {
        let where_ = section.as_ref().map_or("this program".to_string(), |w| format!("`{}`", w.text));
        return refused(Diagnostic::error(part.span, format!("{who} plays no note in {where_}, so there is nothing for it to end on")).help("check the name: a part belongs to a deck (`A.bass`), and plays only where a section or the song writes it"));
    }
    let wants = |key: &Key| key.degree(degree, 4).rem_euclid(12) as u8;
    let bad: Vec<&(String, u64, u8, Key)> = landed.iter().filter(|(_, _, note, key)| note % 12 != wants(key)).collect();
    let Some((name, last, note, key)) = bad.first().copied() else {
        let list: Vec<String> = landed.iter().take(LISTED).map(|(n, at, ..)| format!("{n} (bar {})", at / 16)).collect();
        let more = landed.len().saturating_sub(LISTED);
        let shown = if more > 0 { format!("{} and {more} more", list.join(", ")) } else { list.join(", ") };
        return (Judged::Held(format!("{who} ends on degree {degree} in {}: {shown}{omitted}", plural(landed.len(), "place", "places"))), Vec::new());
    };
    let want = SHARP[usize::from(wants(key))];
    let got = SHARP[usize::from(note % 12)];
    let is = degree_of(*key, note % 12).map_or_else(|| format!("{got}, which is no note of the key"), |d| format!("{got}, degree {d}"));
    let message = format!("in {name} (key {}) {who} ends on {is}, not {want}, at {}{omitted}", key.text(), bar_step(*last));
    let from = evs.iter().find(|e| e.at == *last && e.note == *note).and_then(|e| {
        let sp = e.part.and_then(|p| p.spans.get(e.sid as usize)).and_then(|ch| ch.first())?;
        let t = sp.text(&c.source);
        (!t.is_empty() && t.len() <= 12).then(|| format!("The last note comes from `{t}` on line {}. ", line_col(&c.source, sp.start).0))
    });
    // (a program with no sections has no section to say `in` of)
    let scope = (name != "the program").then(|| Word { text: name.trim_matches('`').to_string(), span: Span::default() });
    let half = degree_of(*key, note % 12).map_or_else(String::new, |d| format!(", or write `{}` if that ending is meant", ends_on_text(part, d as u32, &scope)));
    let rest = if bad.len() > 1 { format!(" It ends on another degree in {} too.", bad.iter().skip(1).take(LISTED).map(|(n, ..)| n.clone()).collect::<Vec<_>>().join(", ")) } else { String::new() };
    (Judged::Broken { message, help: format!("{}End on {want}{half}.{rest}", from.unwrap_or_default()) }, Vec::new())
}
