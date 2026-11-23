//! F8 mix moves (docs/BEATSCRIPT-EXTENSIONS.md, docs/beatscript/ext-mix.md): `duck` and `mix { }`.
//!
//! ```text
//! duck  = "duck" deck "under" deck "." role db { "attack=" time | "release=" time } ;
//! mix   = "mix" "{" item { ( "," | NEWLINE ) item } "}" ;
//! item  = deck "level" arith | deck "eq" band arith [ "db" ] | "master" "level" arith | deck "pan" arith ;
//! ```
//!
//! Both are lines, so they take a time word and `for N bars` like any other: `at bar 8: mix { ... }`,
//! `for 4 bars: mix { ... }`.
//!
//! * A **duck** is the engine's own sidechain, the `trick` noun's `pump`: `duck B under A.kick 6db` is
//!   `trick {action: on, deck: B, kind: pump, source: A, amount, div}`, `amount` the depth of the duck (6 dB is
//!   0.499 of the gain taken off) and `div` the note division that comes nearest to `release` at the tempo. The
//!   pump hears deck A's low end (a 120 Hz detector on its pre-fader signal), so a `kick` keys it (a `bass`
//!   too, said); any other role cannot, which is an error and not a silence. The pump's attack is its own 2 ms.
//! * A **mix** is `mixer` calls: `A level 0.7` is `mixer {deck: A, volume: 0.7}`, `A eq low -3` is
//!   `mixer {deck: A, low: 0.4375}` (the mixer's EQ knob is 0..1, flat at 0.5, -24 dB at 0 and +6 dB at 1),
//!   `master level 0.9` is `mixer {master: 0.9}`. In a `section` (or with `for N bars`) the mixer is read first
//!   (`mixer {}` answers its state) and put back as it was when the section ends.
//!
//! Pure: no clock, no I/O. The runtime asks [`MixPlan::fire`] what to send and what puts it back.

use std::collections::{BTreeMap, BTreeSet};

use djengine::json::{JsonObject, JsonValue};

use crate::ast::{Body, Deck, Target, Word};
use crate::compile::{Call, Compiled, Compiler, Control, Effect, Timing};
use crate::diag::{Diagnostic, Span};
use crate::dur::{Dur, Pos};
use crate::env::suggest;
use crate::expand::{FlatLine, Lowered, SegmentLines};
use crate::expr::{eval, parse_arith, Cursor, Expr, NoHooks, Val};
use crate::lex::Tok;
use crate::parse::{nearest, Parser, ROLES};
use crate::print::num;
use crate::rat::Rat;

type R<T> = Result<T, Diagnostic>;

/// The words this feature adds to the language (the editor colours them as keywords).
pub const KEYWORDS: [&str; 4] = ["duck", "under", "mix", "eq"];
/// A duck's parameters (`attack=5ms`).
pub const PARAMS: [&str; 2] = ["attack", "release"];
/// What an item of a mix block can set.
pub const ITEMS: [&str; 3] = ["level", "eq", "pan"];

/// A duck with no `attack=`.
pub const DEFAULT_ATTACK_MS: f64 = 5.0;
/// A duck with no `release=`.
pub const DEFAULT_RELEASE_MS: f64 = 120.0;
/// The deepest duck, in dB (the pump's depth is a gain, so 60 dB is as good as silence).
pub const MAX_DUCK_DB: f64 = 60.0;
/// The longest attack and release a duck may ask for.
pub const MAX_ATTACK_MS: f64 = 2000.0;
pub const MAX_RELEASE_MS: f64 = 10_000.0;
/// The pump's attack (`djrust::fx::tricks::Pump`), fixed: an `attack=` slower than this is not available.
pub const PUMP_ATTACK_MS: f64 = 2.0;
/// An attack up to here sounds like the pump's own.
pub const FAST_ATTACK_MS: f64 = 10.0;
/// The note divisions a pump releases over (`djrust::fx::tricks::snap_div`).
pub const PUMP_DIVS: [u32; 7] = [4, 6, 8, 12, 16, 24, 32];
/// The mixer's EQ: 0 is `EQ_MIN_DB`, 0.5 is flat, 1 is `EQ_MAX_DB` (`Deck.eqDecibels`).
pub const EQ_MIN_DB: f64 = -24.0;
pub const EQ_MAX_DB: f64 = 6.0;
/// Where the engine starts (`djrust::mixer::StripState::new`, `MixerState::new`): what a snapshot goes back to
/// when the mixer cannot be read and the program has said nothing before.
pub const NEUTRAL_LEVEL: f64 = 0.8;
pub const NEUTRAL_EQ: f64 = 0.5;
pub const NEUTRAL_MASTER: f64 = 1.0;

// ---- the syntax tree

/// `duck B under A.kick 6db attack=5ms release=120ms`.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckStmt {
    pub span: Span,
    /// The deck that goes down.
    pub deck: Deck,
    pub deck_span: Span,
    /// The deck whose part keys the duck.
    pub source: Deck,
    pub source_span: Span,
    /// The part (`kick`).
    pub role: Word,
    /// How far it goes down.
    pub db: f64,
    pub db_span: Span,
    pub attack: Option<Time>,
    pub release: Option<Time>,
}

/// A time as written, in milliseconds (`5ms` is 5, `0.12s` is 120).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Time {
    pub ms: f64,
    pub span: Span,
}

/// `mix { A level 0.7, B level 0.9, A eq low -3 }`.
#[derive(Clone, Debug, PartialEq)]
pub struct MixBlock {
    pub span: Span,
    pub items: Vec<MixItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixItem {
    pub span: Span,
    pub what: MixWhat,
    /// The number, read as arithmetic (evaluated when the program is compiled).
    pub value: Expr,
    /// A `db` written after the number (it is only for `eq`, and it is not printed).
    pub unit: Option<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixWhat {
    /// `A level 0.7`: the strip's fader.
    Level(Deck),
    /// `A eq low -3`: a band's gain in dB.
    Eq(Deck, Band),
    /// `master level 0.9`.
    MasterLevel,
    /// `A pan -1`: parsed, and refused (the mixer has no pan).
    Pan(Deck),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    Low,
    Mid,
    High,
}

impl Band {
    pub const ALL: [&'static str; 3] = ["low", "mid", "high"];

    pub fn parse(w: &str) -> Option<Band> {
        match w {
            "low" => Some(Band::Low),
            "mid" => Some(Band::Mid),
            "high" => Some(Band::High),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Band::Low => "low",
            Band::Mid => "mid",
            Band::High => "high",
        }
    }
}

// ---- reading

impl Parser<'_> {
    /// The body of a `duck` or `mix` line, when that is what the line is (the next word). `target` is the span
    /// of a `deck X:` / `master:` written before it: neither takes one.
    pub(crate) fn mixmoves_body(&mut self, target: Option<Span>) -> R<Option<Body>> {
        let word = match self.peek() {
            // (`duck(` and `mix(` are calls of a def of that name: reserved by position, docs/BEATSCRIPT.md "Decisions")
            Tok::Word(w) if (w == "duck" || w == "mix") && self.peek_n(1) != &Tok::LParen => w.clone(),
            _ => return Ok(None),
        };
        if let Some(tsp) = target {
            let help = if word == "duck" { "duck B under A.kick 6db: a duck names both decks itself" } else { "mix { A level 0.7, B level 0.9 }: a mix names the decks it moves" };
            return Err(Diagnostic::error(tsp, format!("`{word}` takes no `deck X:` or `master:` before it")).help(help));
        }
        Ok(Some(if word == "duck" { Body::Duck(self.duck()?) } else { Body::Mix(self.mix()?) }))
    }

    /// `A` or `B`, with a did-you-mean when it is something else.
    fn deck_here(&mut self, what: &str) -> R<(Deck, Span)> {
        match self.peek().clone() {
            Tok::Word(w) if w == "A" || w == "B" => {
                let sp = self.bump().span;
                Ok((if w == "A" { Deck::A } else { Deck::B }, sp))
            }
            Tok::Word(w) => {
                let mut d = self.expected(&["`A`", "`B`"], &format!("expected {what}, `A` or `B`, found `{w}`"));
                if w.eq_ignore_ascii_case("a") || w.eq_ignore_ascii_case("b") {
                    d = d.help(format!("decks are upper case: `{}`", w.to_uppercase()));
                } else if ROLES.contains(&w.as_str()) {
                    d = d.help(format!("a part belongs to a deck: `A.{w}` or `B.{w}`"));
                }
                Err(d)
            }
            _ => Err(self.expected(&["`A`", "`B`"], &format!("expected {what}, `A` or `B`, found {}", self.found()))),
        }
    }

    fn duck(&mut self) -> R<DuckStmt> {
        let start = self.expect_word("duck")?;
        let (deck, deck_span) = self.deck_here("the deck to lower")?;
        if !self.word_is("under") {
            let other = if deck == Deck::A { "B" } else { "A" };
            return Err(self
                .expected(&["`under`"], &format!("expected `under`, found {}", self.found()))
                .help(format!("{}a duck is `duck {} under {other}.kick 6db`: deck {} goes down under {other}'s kick", if matches!(self.peek(), Tok::Word(w) if crate::parse::nearest(w, ["under"]).is_some()) { "did you mean `under`? " } else { "" }, deck.name(), deck.name())));
        }
        self.bump();
        let (source, source_span) = self.deck_here("the deck whose part keys the duck")?;
        if self.peek() != &Tok::Dot {
            let mut d = Diagnostic::error(source_span, format!("`under` takes a deck's part, like `{0}.kick`, and not just deck {0}", source.name())).expected(&["`A.kick`"]).found(self.found());
            match self.peek() {
                Tok::Word(w) if ROLES.contains(&w.as_str()) => d = d.help(format!("write `{}.{w}`, with no space", source.name())),
                _ => d = d.help(format!("name the part that keys it: duck {} under {}.kick 6db", deck.name(), source.name())),
            }
            return Err(d);
        }
        self.bump();
        let role = match self.peek().clone() {
            Tok::Word(w) => {
                let word = self.word("a part")?;
                if !ROLES.contains(&w.as_str()) {
                    let mut d = Diagnostic::error(word.span, format!("`{w}` is not a part")).expected(&ROLES.map(|r| r)).found(format!("`{w}`"));
                    if let Some(n) = nearest(&w, ROLES) {
                        d = d.help(format!("did you mean `{n}`?"));
                    }
                    return Err(d);
                }
                word
            }
            _ => return Err(self.expected(&ROLES.map(|r| r), &format!("expected a part after `{}.`, found {}", source.name(), self.found()))),
        };
        let (db, db_span) = self.duck_db()?;
        let (mut attack, mut release): (Option<Time>, Option<Time>) = (None, None);
        loop {
            let name = match (self.peek(), self.peek_n(1)) {
                (Tok::Word(w), Tok::Eq) => w.clone(),
                (Tok::Word(w), n) if PARAMS.contains(&w.as_str()) => {
                    return Err(self.expected(&["`=`"], &format!("`{w}` is written `{w}={}ms`, found {} after it", if w == "attack" { "5" } else { "120" }, n.describe())).help(format!("{w}={}ms", if w == "attack" { "5" } else { "120" })));
                }
                _ => break,
            };
            let nsp = self.span();
            if !PARAMS.contains(&name.as_str()) {
                let mut d = Diagnostic::error(nsp, format!("`{name}` is not a parameter of a duck")).expected(&PARAMS.map(|p| p)).found(format!("`{name}`"));
                if let Some(n) = nearest(&name, PARAMS) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                return Err(d);
            }
            self.bump();
            self.bump();
            let t = self.duck_time(&name)?;
            let slot = if name == "attack" { &mut attack } else { &mut release };
            if slot.is_some() {
                return Err(Diagnostic::error(nsp, format!("`{name}` is given twice")));
            }
            *slot = Some(t);
        }
        Ok(DuckStmt { span: start.to(self.prev_span()), deck, deck_span, source, source_span, role, db, db_span, attack, release })
    }

    /// `6db`: how far the deck goes down. No sign (it is an amount), and the unit is glued to the number.
    fn duck_db(&mut self) -> R<(f64, Span)> {
        if matches!(self.peek(), Tok::Minus | Tok::Plus) {
            let sp = self.span();
            let sign = if self.peek() == &Tok::Minus { "-" } else { "+" };
            return Err(Diagnostic::error(sp, format!("a duck is how far the deck goes down, so it takes no sign: `{sign}6db`")).help("duck B under A.kick 6db"));
        }
        if !matches!(self.peek(), Tok::Num { .. }) {
            // (a def's arithmetic argument is put in place in parentheses: the unit glues to a literal, not to that)
            let help = if self.peek() == &Tok::LParen { "a duck's depth and times are numbers with their unit glued (`6db`, `5ms`): arithmetic is for the numbers of a `mix`" } else { "duck B under A.kick 6db" };
            return Err(self.expected(&["a depth like `6db`"], &format!("expected how far to lower it, like `6db`, found {}", self.found())).help(help));
        }
        let n = self.num("a depth in decibels")?;
        match self.peek().clone() {
            Tok::Unit(u) if u == "db" => {
                let usp = self.bump().span;
                Ok((n.value, n.span.to(usp)))
            }
            Tok::Unit(u) => Err(Diagnostic::error(self.span(), format!("a duck's depth is in decibels, not `{u}`")).found(format!("`{}{u}`", num(n.value))).help(format!("{}db", num(n.value)))),
            Tok::Word(w) if w == "db" => Err(Diagnostic::error(n.span.to(self.span()), "a unit is glued to its number: `6db`, not `6 db`").help(format!("write {}db", num(n.value)))),
            _ => Err(Diagnostic::error(n.span, "a duck's depth needs its unit").found(n.span.text(self.src).to_string()).help(format!("write {}db", num(n.value)))),
        }
    }

    /// `5ms` or `0.12s`, in milliseconds.
    fn duck_time(&mut self, what: &str) -> R<Time> {
        if !matches!(self.peek(), Tok::Num { .. }) {
            return Err(self.expected(&["a time like `5ms`"], &format!("expected a time after `{what}=`, like `5ms` or `0.1s`, found {}", self.found())));
        }
        let n = self.num("a time")?;
        let factor = match self.peek().clone() {
            Tok::Unit(u) if u == "ms" => 1.0,
            Tok::Unit(u) if u == "s" => 1000.0,
            Tok::Unit(u) => return Err(Diagnostic::error(self.span(), format!("`{what}` is a time: `ms` or `s`, not `{u}`")).found(format!("`{}{u}`", num(n.value))).help(format!("{what}={}ms", num(n.value)))),
            Tok::Word(w) if w == "ms" || w == "s" => return Err(Diagnostic::error(n.span.to(self.span()), format!("a unit is glued to its number: `{}{w}`", num(n.value)))),
            _ => return Err(Diagnostic::error(n.span, format!("`{what}` needs its unit")).found(n.span.text(self.src).to_string()).help(format!("{what}={}ms", num(n.value)))),
        };
        let usp = self.bump().span;
        let ms = match Rat::from_f64(n.value).and_then(|r| r.mul(Rat::int(factor as i64)).ok()) {
            Some(r) => r.to_f64(),
            None => n.value * factor,
        };
        Ok(Time { ms, span: n.span.to(usp) })
    }

    fn mix(&mut self) -> R<MixBlock> {
        let start = self.expect_word("mix")?;
        let open = match self.peek() {
            Tok::LBrace => self.bump().span,
            _ => return Err(self.expected(&["`{`"], &format!("expected `{{` after `mix`, found {}", self.found())).help("mix { A level 0.7, B level 0.9, A eq low -3 }")),
        };
        let mut items = Vec::new();
        loop {
            while self.peek() == &Tok::Newline {
                self.bump();
            }
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => return Err(Diagnostic::error(open, "the `{` of this `mix` is never closed").help("a mix ends with a `}`")),
                _ => {}
            }
            match self.mix_item() {
                Ok(i) => items.push(i),
                Err(d) => {
                    self.skip_to_close();
                    return Err(d);
                }
            }
            match self.peek() {
                Tok::Comma => {
                    self.bump();
                }
                Tok::Newline | Tok::RBrace | Tok::Eof => {}
                Tok::Arrow => {
                    // (F4 meets F8) a mix item is one number set once: a ramp is a control line's
                    let d = Diagnostic::error(self.span(), "a mix item sets one number, once: it takes no ramp")
                        .found("`->`")
                        .help("move a level over time with a control line: `deck A: level 0 -> 1 over 4 bars exp` (a mix is for what a section sets as it begins)");
                    self.skip_to_close();
                    return Err(d);
                }
                _ => {
                    let d = self.expected(&["`,`", "a new line", "`}`"], &format!("expected `,` or the end of the line after a mix item, found {}", self.found())).help("mix { A level 0.7, B level 0.9 }");
                    self.skip_to_close();
                    return Err(d);
                }
            }
        }
        if items.is_empty() {
            return Err(Diagnostic::error(start.to(self.prev_span()), "a mix has nothing in it").help("mix { A level 0.7 }"));
        }
        Ok(MixBlock { span: start.to(self.prev_span()), items })
    }

    /// After an error inside a `{ }`: on to the `}` that closes it (left unread, for `skip_line`), so the lines of the
    /// block are not read again as statements of their own.
    fn skip_to_close(&mut self) {
        let mut depth = 1;
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.at += 1;
        }
    }

    fn mix_item(&mut self) -> R<MixItem> {
        let start = self.span();
        let what = match self.peek().clone() {
            Tok::Word(w) if w == "master" => {
                self.bump();
                match self.peek().clone() {
                    Tok::Word(l) if l == "level" => {
                        self.bump();
                        MixWhat::MasterLevel
                    }
                    Tok::Word(l) => {
                        let mut d = Diagnostic::error(self.span(), format!("the master has a `level`, and `{l}` is not it")).expected(&["`level`"]).found(format!("`{l}`"));
                        d = d.help(if nearest(&l, ["level"]).is_some() { "did you mean `level`?" } else { "master level 0.9" }.to_string());
                        return Err(d);
                    }
                    _ => return Err(self.expected(&["`level`"], &format!("expected `level` after `master`, found {}", self.found())).help("master level 0.9")),
                }
            }
            Tok::Word(w) if w == "A" || w == "B" => {
                let (deck, _) = self.deck_here("a deck")?;
                let verb = match self.peek().clone() {
                    Tok::Word(v) => v,
                    _ => return Err(self.expected(&ITEMS.map(|i| i), &format!("expected what to set on deck {}, found {}", deck.name(), self.found())).help(format!("{} level 0.7   or   {} eq low -3", deck.name(), deck.name()))),
                };
                let vsp = self.span();
                match verb.as_str() {
                    "level" => {
                        self.bump();
                        MixWhat::Level(deck)
                    }
                    "eq" => {
                        self.bump();
                        MixWhat::Eq(deck, self.band()?)
                    }
                    "pan" => {
                        self.bump();
                        MixWhat::Pan(deck)
                    }
                    other => {
                        let mut d = Diagnostic::error(vsp, format!("`{other}` is not something a mix sets")).expected(&ITEMS.map(|i| i)).found(format!("`{other}`"));
                        d = d.help(if Band::parse(other).is_some() {
                            format!("an EQ band is `{} eq {other} -3`", deck.name())
                        } else if let Some(n) = nearest(other, ITEMS) {
                            format!("did you mean `{n}`?")
                        } else {
                            format!("{} level 0.7   or   {} eq low -3", deck.name(), deck.name())
                        });
                        return Err(d);
                    }
                }
            }
            Tok::Word(w) => {
                let mut d = Diagnostic::error(self.span(), format!("expected a deck, `A` or `B`, or `master`, found `{w}`")).expected(&["`A`", "`B`", "`master`"]).found(format!("`{w}`"));
                if w.eq_ignore_ascii_case("a") || w.eq_ignore_ascii_case("b") {
                    d = d.help(format!("decks are upper case: `{}`", w.to_uppercase()));
                } else if let Some(n) = nearest(&w, ["master"]) {
                    d = d.help(format!("did you mean `{n}`?"));
                } else {
                    d = d.help("A level 0.7   or   master level 0.9");
                }
                return Err(d);
            }
            _ => return Err(self.expected(&["`A`", "`B`", "`master`"], &format!("expected what to set (a deck or `master`), found {}", self.found())).help("mix { A level 0.7 }")),
        };
        let value = self.mix_value(&what)?;
        let unit = match self.peek().clone() {
            Tok::Unit(u) if u == "db" => Some(self.bump().span),
            Tok::Word(w) if w == "db" => Some(self.bump().span),
            Tok::Unit(u) => return Err(Diagnostic::error(self.span(), format!("a mix item is a plain number (an EQ in dB), not `{u}`")).found(format!("`{u}`"))),
            _ => None,
        };
        Ok(MixItem { span: start.to(self.prev_span()), what, value, unit })
    }

    fn band(&mut self) -> R<Band> {
        match self.peek().clone() {
            Tok::Word(w) => match Band::parse(&w) {
                Some(b) => {
                    self.bump();
                    Ok(b)
                }
                None => {
                    let mut d = Diagnostic::error(self.span(), format!("`{w}` is not an EQ band")).expected(&Band::ALL).found(format!("`{w}`"));
                    d = d.help(match nearest(&w, Band::ALL) {
                        Some(n) => format!("did you mean `{n}`?"),
                        None => "eq low -3   (low, mid or high, in dB)".to_string(),
                    });
                    Err(d)
                }
            },
            _ => Err(self.expected(&Band::ALL, &format!("expected an EQ band, found {}", self.found())).help("A eq low -3")),
        }
    }

    /// The number of an item, as arithmetic.
    fn mix_value(&mut self, what: &MixWhat) -> R<Expr> {
        let starts = match self.peek() {
            Tok::Num { .. } | Tok::Minus | Tok::LParen | Tok::Dollar(_) => true,
            Tok::Word(w) => (w == "min" || w == "max") && self.peek_n(1) == &Tok::LParen,
            _ => false,
        };
        if !starts {
            // (F13 meets F8) a mix item is one number set once: an input is a control line's
            if let Tok::Word(w) = self.peek() {
                if crate::perform::is_input_word(w) && self.peek_n(1) == &Tok::LParen {
                    // the whole call, as written: read it as arithmetic and take the first input in it
                    let mut c = Cursor { toks: &self.toks, at: self.at };
                    let at = parse_arith(&mut c).ok().and_then(|e| first_input(&e)).map_or(self.span(), |(_, sp)| sp);
                    return Err(no_input_in_a_mix(at, at.text(self.src), what));
                }
            }
            // (F4 meets F8) a mix item is one number set once: an lfo is a control line's
            if matches!(self.peek(), Tok::Word(w) if w == "lfo") {
                return Err(Diagnostic::error(self.span(), "a mix item sets one number, once: it takes no lfo")
                    .found("`lfo`")
                    .help("keep a level moving with a control line: `deck A: level lfo sine 2 bars center=0.7 depth=0.2` (a mix is for what a section sets as it begins)"));
            }
            let help = match what {
                MixWhat::Eq(d, b) => format!("{} eq {} -3   (dB, {EQ_MIN_DB} to +{EQ_MAX_DB})", d.name(), b.name()),
                MixWhat::Level(d) => format!("{} level 0.7   (0 to 1)", d.name()),
                MixWhat::MasterLevel => "master level 0.9   (0 to 1)".to_string(),
                MixWhat::Pan(d) => format!("{} pan -1", d.name()),
            };
            return Err(self.expected(&["a number"], &format!("expected a number, found {}", self.found())).help(help));
        }
        let mut c = Cursor { toks: &self.toks, at: self.at };
        let e = parse_arith(&mut c)?;
        let at = c.at;
        self.at = at;
        Ok(e)
    }
}

/// A mix item is one number, set once, when the line fires; an input is read while the program plays, at every bar line, which is
/// a control line's (F13 meets F8).
fn no_input_in_a_mix(at: Span, call: &str, what: &MixWhat) -> Diagnostic {
    let name = call.split('(').next().unwrap_or(call);
    let call = if call.contains('(') { call.to_string() } else { format!("{name}(1)") };
    let line = match what {
        MixWhat::Level(d) => format!("deck {}: level {call}", d.name()),
        MixWhat::MasterLevel => format!("master: level {call}"),
        MixWhat::Eq(d, b) => format!("deck {}: {} {call}", d.name(), b.name()),
        MixWhat::Pan(d) => format!("deck {}: level {call}", d.name()),
    };
    Diagnostic::error(at, format!("a mix item sets one number, once: it takes no performed input (`{name}(...)`)"))
        .found(format!("`{call}`"))
        .help(format!("a control line reads the controller at each bar line: `{line}`; a mix is for what a section sets as it begins, and `when held(key)` after its brace decides whether it fires"))
}

/// The first input call in an expression, with its place.
fn first_input(e: &Expr) -> Option<(String, Span)> {
    match e {
        Expr::Call(n, args, sp) => {
            if crate::perform::is_input_word(n) {
                Some((n.clone(), *sp))
            } else {
                args.iter().find_map(first_input)
            }
        }
        Expr::Neg(x, _) | Expr::Not(x, _) => first_input(x),
        Expr::Bin(_, a, b, _) | Expr::Cmp(_, a, b, _) | Expr::And(a, b, _) | Expr::Or(a, b, _) => first_input(a).or_else(|| first_input(b)),
        Expr::Num(..) | Expr::Word(..) | Expr::Unbound(..) => None,
    }
}

// ---- printing

/// A value as the canonical text writes it: the number it comes to, so `0.5 + 0.2` and `0.7` are one program (and the
/// text is a fixed point: `Expr::canonical` prints a short decimal as itself, which reads back as the same rational). A value
/// that is no decimal of at most six places is an error where the item is compiled, so it is never rounded here (L-08).
fn print_value(e: &Expr) -> String {
    e.canonical()
}

/// What an item sets, without its number: `A level`, `A eq low`, `master level`.
pub fn item_label(i: &MixItem) -> String {
    match i.what {
        MixWhat::Level(d) => format!("{} level", d.name()),
        MixWhat::Eq(d, b) => format!("{} eq {}", d.name(), b.name()),
        MixWhat::MasterLevel => "master level".to_string(),
        MixWhat::Pan(d) => format!("{} pan", d.name()),
    }
}

pub fn print_item(i: &MixItem) -> String {
    format!("{} {}", item_label(i), print_value(&i.value))
}

pub fn ms_text(ms: f64) -> String {
    format!("{}ms", num(ms))
}

/// `duck B under A.kick 6db attack=5ms release=120ms`: both times always, the defaults filled in, so a duck has one
/// spelling.
pub fn print_duck(d: &DuckStmt) -> String {
    format!(
        "duck {} under {}.{} {}db attack={} release={}",
        d.deck.name(),
        d.source.name(),
        d.role.text,
        num(d.db),
        ms_text(d.attack.map_or(DEFAULT_ATTACK_MS, |t| t.ms)),
        ms_text(d.release.map_or(DEFAULT_RELEASE_MS, |t| t.ms)),
    )
}

/// `mix { A level 0.7, B level 0.9 }` on one line (the form a control is keyed by).
pub fn print_mix_line(m: &MixBlock) -> String {
    format!("mix {{ {} }}", m.items.iter().map(print_item).collect::<Vec<_>>().join(", "))
}

/// The canonical text of a line whose body is a `mix`: its head (a time word), `mix {`, one item a line, `}`.
pub fn mix_block_text(head: &str, m: &MixBlock, pad: &str) -> String {
    let mut s = format!("{pad}{head}mix {{");
    for i in &m.items {
        s.push_str(&format!("\n{pad}  {}", print_item(i)));
    }
    s.push_str(&format!("\n{pad}}}"));
    s
}

// ---- what a duck and a mix come to

/// `A` or `B` as the registry's deck word.
fn deck_word(d: Deck) -> JsonValue {
    JsonValue::str(d.name())
}

fn round(x: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (x * k).round() / k
}

/// A pump's note division and its length in ms: the one of [`PUMP_DIVS`] whose note at `bpm` comes nearest `release_ms`
/// (by ratio).
pub fn pump_div(release_ms: f64, bpm: f64) -> (u32, f64) {
    let note = |div: u32| 240_000.0 / (bpm.max(1.0) * f64::from(div));
    let off = |div: u32| (note(div) / release_ms.max(0.001)).ln().abs();
    let mut best = PUMP_DIVS[0];
    for d in PUMP_DIVS {
        if off(d) < off(best) {
            best = d;
        }
    }
    (best, note(best))
}

/// The pump's depth for a duck of `db`: the share of the gain taken off (6 dB is 0.499).
pub fn pump_depth(db: f64) -> f64 {
    round(1.0 - 10f64.powf(-db / 20.0), 3)
}

/// How a part of the source deck keys the engine's sidechain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keying {
    /// The detector is made for it.
    Kick,
    /// It hears it, with the kick: the detector is a 120 Hz low-pass on the whole deck.
    Low,
    /// It cannot (a snare, a hat, a melody are not low).
    No,
}

pub fn keying(role: &str) -> Keying {
    match role {
        "kick" => Keying::Kick,
        "bass" => Keying::Low,
        _ => Keying::No,
    }
}

/// A duck, as compiled: what the runtime sends when it fires.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckPlan {
    /// The deck that goes down.
    pub deck: Deck,
    /// The deck whose part keys it, and the part.
    pub source: Deck,
    pub role: String,
    pub db: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    /// The program's tempo, when it has one (the pump's release is a note division, chosen at the tempo).
    pub bpm: Option<f64>,
}

impl DuckPlan {
    /// The duck as the canonical text says it.
    pub fn text(&self) -> String {
        format!("duck {} under {}.{} {}db attack={} release={}", self.deck.name(), self.source.name(), self.role, num(self.db), ms_text(self.attack_ms), ms_text(self.release_ms))
    }

    fn on_params(&self, div: Option<u32>) -> Call {
        let mut o = JsonObject::from_pairs([
            ("action", JsonValue::str("on")),
            ("deck", deck_word(self.deck)),
            ("kind", JsonValue::str("pump")),
            ("source", deck_word(self.source)),
            ("amount", JsonValue::Number(pump_depth(self.db))),
        ]);
        if let Some(d) = div {
            o.set("div", JsonValue::int(i64::from(d)));
        }
        Call { noun: "trick".into(), params: JsonValue::Object(o) }
    }

    /// `trick {action: on, deck, kind: pump, source, amount, div}`: at `bpm` (the program's, else the deck's).
    pub fn on_call(&self, bpm: f64) -> Call {
        self.on_params(Some(pump_div(self.release_ms, bpm).0))
    }

    /// `trick {action: off, deck, kind: pump}`.
    pub fn off_call(&self) -> Call {
        Call { noun: "trick".into(), params: JsonValue::Object(JsonObject::from_pairs([("action", JsonValue::str("off")), ("deck", deck_word(self.deck)), ("kind", JsonValue::str("pump"))])) }
    }

    /// What it says in a check: the call, and what the engine does of what was asked.
    pub fn describe(&self, bpm: Option<f64>) -> String {
        let at = bpm.or(self.bpm);
        let mut s = String::new();
        match at {
            Some(b) => {
                let (div, ms) = pump_div(self.release_ms, b);
                s.push_str(&format!("{}  (release {} is a 1/{div} note at {} BPM: {} ms", self.on_call(b).text(), ms_text(self.release_ms), num(b), num(round(ms, 0))));
            }
            None => s.push_str(&format!("{}  (div: the note nearest {}, chosen from the deck's tempo when it fires", self.on_params(None).text(), ms_text(self.release_ms))),
        }
        s.push_str(&format!("; depth {} of the gain", num(pump_depth(self.db))));
        if self.attack_ms > FAST_ATTACK_MS {
            s.push_str(&format!("; attack {} asked, the pump's is {} ms)", ms_text(self.attack_ms), num(PUMP_ATTACK_MS)));
        } else {
            s.push_str(&format!("; the pump attacks in {} ms)", num(PUMP_ATTACK_MS)));
        }
        s
    }
}

/// One thing a mix sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MixTarget {
    Level(Deck),
    Eq(Deck, Band),
    Master,
}

impl MixTarget {
    /// The `mixer` call that puts the knob at `v` (the mixer's own 0..1).
    pub fn call(self, v: f64) -> Call {
        let v = JsonValue::Number(round(v, 6));
        let pairs: Vec<(&str, JsonValue)> = match self {
            MixTarget::Level(d) => vec![("deck", deck_word(d)), ("volume", v)],
            MixTarget::Eq(d, b) => vec![("deck", deck_word(d)), (b.name(), v)],
            MixTarget::Master => vec![("master", v)],
        };
        Call { noun: "mixer".into(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }
    }

    /// Where the engine starts it.
    pub fn neutral(self) -> f64 {
        match self {
            MixTarget::Level(_) => NEUTRAL_LEVEL,
            MixTarget::Eq(..) => NEUTRAL_EQ,
            MixTarget::Master => NEUTRAL_MASTER,
        }
    }
}

/// An EQ's gain in dB as the mixer's knob (0..1, 0.5 flat): `-24` is 0, `+6` is 1.
pub fn eq_knob(db: f64) -> f64 {
    round(if db < 0.0 { 0.5 + db / 48.0 } else { 0.5 + db / 12.0 }, 6)
}

/// The knob as a gain in dB.
pub fn eq_db(knob: f64) -> f64 {
    if knob < 0.5 {
        -24.0 * (0.5 - knob) / 0.5
    } else {
        6.0 * (knob - 0.5) / 0.5
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixSet {
    pub target: MixTarget,
    /// The value in the mixer's own unit (0..1).
    pub knob: f64,
    /// What it sets (`A eq low`) and the item as the canonical text says it (`A eq low -3`).
    pub label: String,
    pub text: String,
    pub span: Span,
    /// What it goes back to when the mixer cannot be read: the program's own last line before it (`from_program`), or the
    /// engine's starting position.
    pub fallback: f64,
    pub from_program: bool,
}

impl MixSet {
    pub fn call(&self) -> Call {
        self.target.call(self.knob)
    }
}

/// A mix block, as compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct MixPlan {
    pub items: Vec<MixSet>,
    /// The mixer is read first and put back (a section's snapshot, or a `for N bars`); otherwise it just sets.
    pub restore: bool,
}

/// What a mix does when it fires.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MixFire {
    /// The calls that set it, in the order written.
    pub sets: Vec<Call>,
    /// The calls that put it back (last set first), from what the mixer held.
    pub back: Vec<Call>,
    /// What to say in `btw get`'s words.
    pub words: Vec<String>,
}

/// The mixer as a read of it gives it: every strip's fader and EQ knobs, and the master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MixerRead {
    pub values: BTreeMap<MixTarget, f64>,
}

/// `mixer {}`'s answer (`{set: [], mixer: {strips: [{deck, volume, eq: {low, mid, high}}], master}}`, or the bare state).
pub fn read_mixer(v: &JsonValue) -> MixerRead {
    let m = v.get("mixer").unwrap_or(v);
    let mut r = MixerRead::default();
    if let Some(strips) = m.get("strips").and_then(JsonValue::as_array) {
        for s in strips {
            let deck = match s.get("deck").and_then(JsonValue::as_str) {
                Some("A") => Deck::A,
                Some("B") => Deck::B,
                _ => continue,
            };
            if let Some(x) = s.get("volume").and_then(JsonValue::as_f64) {
                r.values.insert(MixTarget::Level(deck), x);
            }
            for (name, band) in [("low", Band::Low), ("mid", Band::Mid), ("high", Band::High)] {
                if let Some(x) = s.get("eq").and_then(|e| e.get(name)).and_then(JsonValue::as_f64) {
                    r.values.insert(MixTarget::Eq(deck, band), x);
                }
            }
        }
    }
    if let Some(x) = m.get("master").and_then(JsonValue::as_f64) {
        r.values.insert(MixTarget::Master, x);
    }
    r
}

impl MixPlan {
    /// Fire it: what to send, and (when it restores) what puts each knob back, from `read` (the answer of `mixer {}`: None
    /// when the host cannot say) or, for a knob it does not give, from the program's own last line, else neutral.
    pub fn fire(&self, read: Option<&JsonValue>) -> MixFire {
        let mut f = MixFire { sets: self.items.iter().map(MixSet::call).collect(), ..MixFire::default() };
        if !self.restore {
            return f;
        }
        let state = read.map(read_mixer);
        let mut unread: Vec<&MixSet> = Vec::new();
        for it in &self.items {
            match state.as_ref().and_then(|s| s.values.get(&it.target)) {
                Some(v) => f.back.push(it.target.call(*v)),
                None => {
                    f.back.push(it.target.call(it.fallback));
                    unread.push(it);
                }
            }
        }
        f.back.reverse();
        if !unread.is_empty() {
            let list: Vec<String> = unread.iter().map(|i| format!("{} to {} ({})", i.label, num(round(i.fallback, 4)), if i.from_program { "the program's own last line" } else { "neutral" })).collect();
            f.words.push(format!("mix: the mixer cannot be read here, so it goes back {}", list.join(", ")));
        }
        f
    }
}

// ---- the compile pass

/// What the program's own earlier lines set on the mixer, by the position (in bars, exact enough to order them) they fire
/// at: the value a snapshot goes back to when the mixer cannot be read.
#[derive(Default)]
struct Known {
    events: Vec<(f64, MixTarget, f64)>,
    /// The decks a duck lowers so far, by the section it is in (None: outside every section).
    ducks: Vec<(Option<usize>, Deck)>,
    /// The knobs a mix holds changed (and puts back) so far, and for how long.
    held: Vec<Held>,
}

/// A knob held by a mix that is put back: from where to where (in bars), and the item that holds it.
struct Held {
    target: MixTarget,
    from: Rat,
    to: Rat,
    span: Span,
}

/// Where a control fires, in bars from the start of the program, when that is one place: a bar line (`Now`, `at bar 4`) or a
/// time between bar lines (`at bar 4 beat 2`). A line that repeats has no one place.
fn pos_of(t: Timing) -> Option<Rat> {
    match t {
        Timing::Now => Some(Rat::ZERO),
        Timing::At(b) => Some(Rat::int(i64::from(b))),
        Timing::AtPos(p) => Some(p),
        _ => None,
    }
}

/// The bar a time falls in (`at bar 4 beat 2` is in bar 4).
fn bar_of(t: Timing) -> Option<u32> {
    pos_of(t).and_then(|p| u32::try_from(p.floor()).ok())
}

/// A position as a check says it: `bar 6`, `bar 4 beat 2`.
fn pos_text(r: Rat) -> String {
    Pos::from_bars(r).map_or_else(|| format!("bar {r}"), Pos::canonical)
}

/// How close two positions have to be to be one (a rational that went through a float).
const POS_EPS: f64 = 1e-9;

impl Known {
    fn from_controls(cs: &[Control]) -> Known {
        let mut k = Known::default();
        for c in cs {
            let Some(at) = pos_of(c.timing).map(Rat::to_f64) else { continue };
            match &c.effect {
                Effect::Calls(calls) => {
                    for call in calls {
                        for (t, v) in call_sets(call) {
                            k.events.push((at, t, v));
                        }
                    }
                }
                Effect::Ramp(r) if r.noun == "mixer" => {
                    let mut o = r.base.as_object().cloned().unwrap_or_default();
                    o.set(r.field.as_str(), JsonValue::Number(r.to));
                    for (t, v) in call_sets(&Call { noun: "mixer".into(), params: JsonValue::Object(o) }) {
                        k.events.push((at + r.bars.to_f64(), t, v));
                    }
                }
                _ => {}
            }
        }
        k
    }

    /// The last value set on `t` at or before `pos`.
    fn at(&self, pos: f64, t: MixTarget) -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for (b, x, v) in &self.events {
            if *x == t && *b <= pos + POS_EPS && best.is_none_or(|(bb, _)| *b >= bb) {
                best = Some((*b, *v));
            }
        }
        best.map(|(_, v)| v)
    }
}

/// What a `mixer` call sets: `{deck: A, volume: v}`, `{deck: A, low: v}`, `{master: v}`.
fn call_sets(c: &Call) -> Vec<(MixTarget, f64)> {
    let mut out = Vec::new();
    if c.noun != "mixer" {
        return out;
    }
    let Some(o) = c.params.as_object() else { return out };
    let deck = match o.get("deck").and_then(JsonValue::as_str) {
        Some("A") => Some(Deck::A),
        Some("B") => Some(Deck::B),
        _ => None,
    };
    if let Some(v) = o.get("master").and_then(JsonValue::as_f64) {
        out.push((MixTarget::Master, v));
    }
    if let Some(d) = deck {
        if let Some(v) = o.get("volume").and_then(JsonValue::as_f64) {
            out.push((MixTarget::Level(d), v));
        }
        for b in [Band::Low, Band::Mid, Band::High] {
            if let Some(v) = o.get(b.name()).and_then(JsonValue::as_f64) {
                out.push((MixTarget::Eq(d, b), v));
            }
        }
    }
    out
}

type Roles = BTreeSet<(Deck, String)>;

/// The parts each deck has in a set of lines.
fn roles_of(lines: &[FlatLine]) -> Roles {
    let mut out = BTreeSet::new();
    for fl in lines {
        let Body::Parts(ps) = &fl.line.body else { continue };
        let Some((Target::Deck(d), _)) = fl.line.target else { continue };
        for p in ps {
            out.insert((d, p.role.text.clone()));
        }
    }
    out
}

/// The parts that count where a line is: the song's own, and (in a section) the section's.
struct Scopes {
    /// Every part anywhere: what a line outside every section may name.
    all: Roles,
    /// The song's own parts and each section's, in timeline order.
    per_seg: Vec<Roles>,
}

/// The section a line was written in, as its path says (`[drop@16] `, maybe more after it): its name and first bar.
pub(crate) fn section_of_path(path: &str) -> Option<(&str, u32)> {
    let inner = path.strip_prefix('[')?.split_once(']')?.0;
    let (name, start) = inner.split_once('@')?;
    Some((name, start.parse().ok()?))
}

/// The bars a line's section plays over: from its first bar to the bar after its last. A section stacked with others is cut
/// into stretches (`a+b`, then `b`); its lines sit in the first of them and a copy of each goes on into the stretches that
/// follow (`phase` is set on the copy), so the end of the first stretch is not where the section ends: this follows the
/// copies of the line itself, which is exact even when the same section is placed twice in a row.
pub(crate) fn section_span(fl: &FlatLine, seg: &SegmentLines, lw: &Lowered) -> (u32, u32) {
    let mut end = seg.start + seg.bars;
    for s in lw.segments.iter().filter(|s| s.start > seg.start) {
        if s.start == end && s.chain && s.lines.iter().any(|l| l.phase.is_some() && l.span == fl.span && l.path == fl.path) {
            end = s.start + s.bars;
        }
    }
    (seg.start, end)
}

/// The section's own name (a stretch of a stack is `a+b`; a line knows which of them it is in).
fn section_name(fl: &FlatLine, seg: &SegmentLines) -> String {
    section_of_path(&fl.path).map_or_else(|| seg.name.clone(), |(n, _)| n.to_string())
}

fn when_span(fl: &FlatLine) -> Span {
    fl.line.when.as_ref().map_or(fl.span, |(_, s)| *s)
}

impl Compiler<'_> {
    /// Every `duck` and `mix` line becomes a control. Run once the other controls are made, in the song's order (the lines
    /// outside every section first, then each section's), so a section's snapshot is put back before the next one's is taken.
    pub(crate) fn mix_controls(&mut self, lw: &Lowered) {
        let has = |lines: &[FlatLine]| lines.iter().any(|f| matches!(f.line.body, Body::Duck(_) | Body::Mix(_)));
        if !has(&lw.globals) && !lw.segments.iter().any(|s| has(&s.lines)) {
            return;
        }
        let mut known = Known::from_controls(&self.controls);
        let global = roles_of(&lw.globals);
        let per_seg: Vec<Roles> = lw.segments.iter().map(|s| global.iter().cloned().chain(roles_of(&s.lines)).collect()).collect();
        let mut all = global;
        for r in &per_seg {
            all.extend(r.iter().cloned());
        }
        let scopes = Scopes { all, per_seg };
        for fl in &lw.globals {
            self.mix_line(fl, None, &scopes, lw, &mut known);
        }
        for (i, seg) in lw.segments.iter().enumerate() {
            for fl in &seg.lines {
                self.mix_line(fl, Some(i), &scopes, lw, &mut known);
            }
        }
    }

    fn mix_line(&mut self, fl: &FlatLine, seg: Option<usize>, scopes: &Scopes, lw: &Lowered, known: &mut Known) {
        let section = seg.map(|i| &lw.segments[i]);
        // (F10: a line carried into a later stretch of a stacked section already fired in the first)
        if fl.phase.is_some() {
            return;
        }
        match &fl.line.body {
            Body::Duck(d) => self.duck_control(fl, d, section, seg, scopes, lw, known),
            Body::Mix(m) => self.mix_control(fl, m, section, lw, known),
            _ => {}
        }
    }

    /// How long a duck or a mix holds: a `for N bars` (or `for 2 beats`), else (in a section) to the end of the section: the
    /// bar where the section itself ends, which for a section stacked with a shorter or a longer one is not the end of the
    /// stretch its lines are in. `Ok(None)`: it is not put back by the clock.
    fn scope_hold(&mut self, fl: &FlatLine, seg: Option<&SegmentLines>, lw: &Lowered, what: &str) -> Result<Option<Dur>, ()> {
        if let Some(n) = fl.hold {
            return Ok(Some(n));
        }
        let Some(seg) = seg else { return Ok(None) };
        let (start, end) = section_span(fl, seg, lw);
        let from = match fl.timing {
            Timing::At(b) => Rat::int(i64::from(b)),
            Timing::AtPos(p) => p,
            _ => {
                self.err(
                    Diagnostic::error(when_span(fl), format!("a {what} in a section fires once and is put back when the section ends: this time word repeats it"))
                        .help("use `at bar N`, or put the line outside the section where `every N bars` just sets it again"),
                );
                return Err(());
            }
        };
        if from < Rat::int(i64::from(start)) || from >= Rat::int(i64::from(end)) {
            self.err(
                Diagnostic::error(when_span(fl), format!("this {what} fires on {}, outside section `{}` (bars {} to {})", pos_text(from), section_name(fl, seg), start, end - 1))
                    .help("a section's lines happen in the section: `at bar N` counts from its first bar"),
            );
            return Err(());
        }
        match from.sub(Rat::int(i64::from(end))).and_then(Rat::neg).ok().and_then(Dur::from_bars) {
            Some(d) => Ok(Some(d)),
            None => Err(()),
        }
    }

    /// A registry that is there must have what the calls name: the noun, its params, the verb.
    fn check_calls(&mut self, calls: &[Call], span: Span) -> bool {
        let Some(reg) = self.env.registry else { return true };
        let mut ok = true;
        for c in calls {
            let Some(spec) = reg.noun(&c.noun) else {
                let all = reg.noun_names();
                let mut d = Diagnostic::error(span, format!("the control language here has no `{}` noun, which this line is made of", c.noun));
                if let Some(n) = suggest(&c.noun, &all, 1).first() {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                self.err(d);
                ok = false;
                continue;
            };
            let Some(o) = c.params.as_object() else { continue };
            for (k, _) in o.iter() {
                if k == "action" {
                    let a = o.get("action").and_then(JsonValue::as_str).unwrap_or("");
                    if !spec.actions.is_empty() && spec.action(a).is_none() {
                        self.err(Diagnostic::error(span, format!("`{}` has no verb `{a}` here, which this line needs", spec.name)));
                        ok = false;
                    }
                } else if spec.param(k).is_none() {
                    self.err(Diagnostic::error(span, format!("`{}` has no param `{k}` here, which this line needs", spec.name)).expected(&spec.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()));
                    ok = false;
                }
            }
        }
        ok
    }

    #[allow(clippy::too_many_arguments)]
    fn duck_control(&mut self, fl: &FlatLine, d: &DuckStmt, section: Option<&SegmentLines>, seg: Option<usize>, scopes: &Scopes, lw: &Lowered, known: &mut Known) {
        let before = self.diags.len();
        // the engine has one pump on a deck: a second duck of it replaces the first
        if known.ducks.contains(&(seg, d.deck)) {
            self.diags.push(
                Diagnostic::warning(d.deck_span, format!("deck {} is already ducked {}: the engine has one pump on a deck, so this duck replaces the other", d.deck.name(), if seg.is_some() { "in this section" } else { "by an earlier line" }))
                    .help("one duck a deck, under the part that matters"),
            );
        } else if seg.is_some() && known.ducks.contains(&(None, d.deck)) {
            self.diags.push(
                Diagnostic::warning(d.deck_span, format!("deck {} is ducked outside the sections too: this duck replaces it and turns the pump off where the section ends", d.deck.name()))
                    .help("put the duck in the sections, or leave it outside them"),
            );
        }
        known.ducks.push((seg, d.deck));
        let key = (d.source, d.role.text.clone());
        if d.deck == d.source {
            let other = if d.deck == Deck::A { "B" } else { "A" };
            self.err(
                Diagnostic::error(d.source_span.to(d.role.span), format!("deck {} cannot duck under itself", d.deck.name()))
                    .help(format!("a duck lowers one deck under the other's part: duck {} under {other}.{} {}db", d.deck.name(), d.role.text, num(d.db))),
            );
        }
        match keying(&d.role.text) {
            Keying::Kick => {}
            Keying::Low => self.diags.push(
                Diagnostic::warning(d.role.span, format!("the engine's sidechain hears a deck's low end: `{}.bass` keys it together with the kick", d.source.name()))
                    .help("`kick` is what it is made for; a bass note ducks the other deck on its own onset too"),
            ),
            Keying::No => self.err(
                Diagnostic::error(d.role.span, format!("`{}` cannot key a duck: the engine's sidechain listens for a deck's kick (a low-pass detector), and a {} is not low", d.role.text, d.role.text))
                    .expected(&["kick", "bass"])
                    .found(format!("`{}`", d.role.text))
                    .help(format!("duck {} under {}.kick {}db", d.deck.name(), d.source.name(), num(d.db))),
            ),
        }
        let roles = match seg {
            Some(i) => &scopes.per_seg[i],
            None => &scopes.all,
        };
        if !roles.contains(&key) {
            let has: Vec<&str> = roles.iter().filter(|(dk, _)| *dk == d.source).map(|(_, r)| r.as_str()).collect();
            let here = match section {
                Some(s) => format!("in section `{}`", s.name),
                None => "in this program".to_string(),
            };
            let mut diag = Diagnostic::error(d.source_span.to(d.role.span), format!("deck {} has no `{}` part {here}, so there is nothing to key the duck", d.source.name(), d.role.text));
            diag = diag.help(if has.is_empty() { format!("give deck {} the part: deck {}: {} x ~ x ~", d.source.name(), d.source.name(), d.role.text) } else { format!("deck {} plays: {}", d.source.name(), has.join(", ")) });
            self.err(diag);
        } else if seg.is_none() && lw.arranged {
            // outside every section: the sections where the part is missing have nothing to duck on
            let lacking: Vec<&str> = lw.segments.iter().zip(&scopes.per_seg).filter(|(_, r)| !r.contains(&key)).map(|(s, _)| s.name.as_str()).collect();
            if !lacking.is_empty() {
                self.diags.push(Diagnostic::warning(d.source_span.to(d.role.span), format!("deck {} has no `{}` in section {}: the duck idles there", d.source.name(), d.role.text, lacking.join(", "))));
            }
        }
        if !(d.db > 0.0 && d.db <= MAX_DUCK_DB) {
            self.err(
                Diagnostic::error(d.db_span, if d.db <= 0.0 { "a duck of 0db does nothing".to_string() } else { format!("a duck is at most {MAX_DUCK_DB}db") })
                    .found(d.db_span.text(self.src).to_string())
                    .help("a pumping duck is 3db to 12db"),
            );
        }
        for (what, t, min, max) in [("attack", d.attack, 0.0, MAX_ATTACK_MS), ("release", d.release, 1.0, MAX_RELEASE_MS)] {
            if let Some(t) = t {
                if t.ms < min || t.ms > max {
                    self.err(Diagnostic::error(t.span, format!("`{what}` is {} to {}", ms_text(min), ms_text(max))).found(t.span.text(self.src).to_string()));
                }
            }
        }
        let attack = d.attack.map_or(DEFAULT_ATTACK_MS, |t| t.ms);
        let release = d.release.map_or(DEFAULT_RELEASE_MS, |t| t.ms);
        if attack > FAST_ATTACK_MS && attack <= MAX_ATTACK_MS {
            self.diags.push(
                Diagnostic::warning(d.attack.map_or(d.span, |t| t.span), format!("the engine's pump attacks in {} ms: `attack={}` cannot be slower than that", num(PUMP_ATTACK_MS), ms_text(attack)))
                    .help("an attack up to 10ms sounds as the pump does"),
            );
        }
        if let (Some(bpm), true) = (self.tempo, release >= 1.0) {
            let (div, ms) = pump_div(release, bpm);
            if (ms / release).max(release / ms) > 1.4 {
                self.diags.push(
                    Diagnostic::warning(d.release.map_or(d.span, |t| t.span), format!("the pump releases over a note: at {} BPM the nearest to `release={}` is a 1/{div}, {} ms", num(bpm), ms_text(release), num(round(ms, 0))))
                        .help("the longest is a quarter note, the shortest a 1/32"),
                );
            }
        }
        let plan = DuckPlan { deck: d.deck, source: d.source, role: d.role.text.clone(), db: d.db, attack_ms: attack, release_ms: release, bpm: self.tempo };
        let Ok(hold) = self.scope_hold(fl, section, lw, "duck") else { return };
        if !self.check_calls(&[plan.on_call(self.tempo.unwrap_or(120.0)), plan.off_call()], d.span) {
            return;
        }
        if self.diags[before..].iter().any(Diagnostic::is_error) {
            return;
        }
        let undo = vec![plan.off_call()];
        // a `when held(key)` after it is read while the program plays (F13): the line is the duck, decided when it is due
        let Some(effect) = self.performed(fl, Effect::Duck(plan)) else { return };
        self.controls.push(Control { key: format!("{}{}", fl.path, crate::print::line(&fl.line)), timing: fl.timing, effect, span: fl.span, hold, undo, restore: None, via: fl.via.clone(), notice: None, window: None, lead: None });
    }

    /// An item's number, evaluated.
    fn mix_number(&mut self, e: &Expr, what: &MixWhat) -> Option<f64> {
        match eval(e, &NoHooks) {
            // a value is exactly what was written or an error, as a chance and a weight are: a decimal of at most six places
            // (`B level 1/3` would be a rounded 0.333333, and `1/3` and `0.333333` one program)
            Ok(Val::Num(r)) if r.decimal6().is_none() => {
                self.err(
                    Diagnostic::error(e.span(), "a mix value has at most six decimals").found(format!("{r}")).help(format!("write the number you mean: {} is {}", e.span().text(self.src), crate::print::num(round(r.to_f64(), 6)))),
                );
                None
            }
            Ok(Val::Num(r)) => Some(r.to_f64()),
            Ok(other) => {
                self.err(Diagnostic::error(e.span(), format!("a mix item is a number, not {}", other.describe())));
                None
            }
            Err(d) => {
                // (an input inside arithmetic: `A level 0.5 + knob(1)`)
                let d = match (d.message.contains("is a performed input"), first_input(e)) {
                    (true, Some((_, span))) => no_input_in_a_mix(span, span.text(self.src), what),
                    _ => d,
                };
                self.err(d);
                None
            }
        }
    }

    fn mix_control(&mut self, fl: &FlatLine, m: &MixBlock, section: Option<&SegmentLines>, lw: &Lowered, known: &mut Known) {
        let before = self.diags.len();
        let mut sets: Vec<MixSet> = Vec::new();
        for it in &m.items {
            let Some(v) = self.mix_number(&it.value, &it.what) else { continue };
            let shown = it.value.span().text(self.src).to_string();
            let (target, knob) = match it.what {
                MixWhat::Pan(d) => {
                    self.err(
                        Diagnostic::error(it.span, format!("the mixer has no pan on a strip, so `{} pan` has nothing to move", d.name()))
                            .help("a net deck's parts can be panned for some bars with `do aux macro=hard-left deck=A bars=4` (also hard-right, mono, ping-pong, auto-pan)"),
                    );
                    continue;
                }
                MixWhat::Level(d) => (MixTarget::Level(d), v),
                MixWhat::MasterLevel => (MixTarget::Master, v),
                MixWhat::Eq(d, b) => {
                    if !(EQ_MIN_DB..=EQ_MAX_DB).contains(&v) {
                        self.err(Diagnostic::error(it.value.span(), format!("an EQ band is {EQ_MIN_DB} to +{EQ_MAX_DB} dB (the mixer's knob: -24 dB at 0, flat at 0.5, +6 dB at 1)")).found(shown).help(format!("{} eq {} -3", d.name(), b.name())));
                        continue;
                    }
                    (MixTarget::Eq(d, b), eq_knob(v))
                }
            };
            if !matches!(it.what, MixWhat::Eq(..)) {
                if let Some(u) = it.unit {
                    self.err(Diagnostic::error(u, "only an EQ band is in dB: a level is the fader's 0 to 1").help("A level 0.7"));
                    continue;
                }
                if !(0.0..=1.0).contains(&v) {
                    self.err(Diagnostic::error(it.value.span(), "a level is 0 to 1 (or 0% to 100%)").found(shown).help("an EQ band is in dB: A eq low -3"));
                    continue;
                }
            }
            if let Some(prev) = sets.iter().find(|s| s.target == target) {
                let at = crate::diag::line_col(self.src, prev.span.start);
                self.err(Diagnostic::error(it.span, format!("`{}` is set twice in this mix (line {})", prev.label, at.0)).help("one value for each knob"));
                continue;
            }
            sets.push(MixSet { target, knob, label: item_label(it), text: print_item(it), span: it.span, fallback: target.neutral(), from_program: false });
        }
        let Ok(hold) = self.scope_hold(fl, section, lw, "mix") else { return };
        if self.diags[before..].iter().any(Diagnostic::is_error) {
            return;
        }
        let restore = hold.is_some();
        // a mix is put back to what the mixer held before it: two that hold one knob at once (the same section twice over
        // with `for`, or two sections stacked) cannot both go back to it, the one that ends first would put back the
        // other's value and the one that ends last would put back that one
        if let (Some(from), Some(h)) = (pos_of(fl.timing), hold) {
            if let Ok(to) = from.add(h.as_bars()) {
                let mut clash = false;
                for s in &sets {
                    if let Some(prev) = known.held.iter().find(|x| x.target == s.target && x.from < to && from < x.to) {
                        let line = crate::diag::line_col(self.src, prev.span.start).0;
                        self.err(
                            Diagnostic::error(s.span, format!("`{}` is held by two mixes at once: this one from {} to {}, and the one on line {line} from {} to {}", s.label, pos_text(from), pos_text(to), pos_text(prev.from), pos_text(prev.to)))
                                .help("a mix is put back to what the mixer held before it, so two that overlap cannot both be: move one to another knob, or let them follow each other (sections one after the other, `at bar N` after the other has ended)"),
                        );
                        clash = true;
                    }
                }
                if clash {
                    return;
                }
                for s in &sets {
                    known.held.push(Held { target: s.target, from, to, span: s.span });
                }
            }
        }
        let at = pos_of(fl.timing).map(Rat::to_f64);
        for s in &mut sets {
            if let Some(v) = at.and_then(|p| known.at(p, s.target)) {
                s.fallback = v;
                s.from_program = true;
            }
        }
        if !restore {
            // what it sets stays: the lines after it find it there
            if let Some(p) = at {
                for s in &sets {
                    known.events.push((p, s.target, s.knob));
                }
            }
        }
        let calls: Vec<Call> = sets.iter().map(MixSet::call).collect();
        if !self.check_calls(&calls, m.span) {
            return;
        }
        let Some(effect) = self.performed(fl, Effect::Mix(MixPlan { items: sets, restore })) else { return };
        self.controls.push(Control { key: format!("{}{}", fl.path, crate::print::line(&fl.line)), timing: fl.timing, effect, span: fl.span, hold, undo: Vec::new(), restore: None, via: fl.via.clone(), notice: None, window: None, lead: None });
    }
}

// ---- what check says

/// The section a control's key says it is in (`[drop@16] ...`).
fn section_of(key: &str) -> Option<&str> {
    let rest = key.strip_prefix('[')?;
    let (inner, _) = rest.split_once(']')?;
    inner.split('@').next()
}

impl Compiled {
    /// What the `duck` and `mix` lines come to, one line each: the registry calls they make, what the engine does of what was
    /// asked, and when the mixer is put back (`describe`-style).
    pub fn mix_report(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in &self.controls {
            let back = |hold: Option<Dur>| match (hold, pos_of(c.timing)) {
                (Some(h), Some(p)) => match p.add(h.as_bars()) {
                    Ok(end) => format!("put back on {}", pos_text(end)),
                    Err(_) => format!("put back after {h}"),
                },
                (Some(h), None) => format!("put back after {h}"),
                (None, _) => "kept until the program stops".to_string(),
            };
            let in_sec = section_of(&c.key).map_or(String::new(), |s| format!(" in {s}"));
            // (a line with a performed `when` is the same line, decided when it is due)
            let when = match &c.effect {
                Effect::Performed(p) if p.guard.is_some() => format!("; only when `{}` is true at the bar it is due", p.guard_text),
                _ => String::new(),
            };
            match c.effect.base() {
                Effect::Duck(d) => {
                    let keyed = self
                        .hits_per_bar(d.source, &d.role, c.timing)
                        .map_or(String::new(), |(lo, hi)| format!("; deck {}'s {} plays {} a bar", d.source.name(), d.role, if lo == hi { format!("{lo} hit{}", if lo == 1 { "" } else { "s" }) } else { format!("{lo} to {hi} hits") }));
                    out.push(format!("{}{in_sec}: {}{keyed}; {}{when}", d.text(), d.describe(self.tempo), back(c.hold)));
                }
                Effect::Mix(m) => {
                    let calls: Vec<String> = m.items.iter().map(|i| i.call().text()).collect();
                    let how = if m.restore { format!("reads the mixer first, {}", back(c.hold)) } else { "just sets".to_string() };
                    out.push(format!("mix{in_sec}: {} -> {}; {how}{when}", m.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>().join(", "), calls.join(", ")));
                }
                _ => {}
            }
        }
        out
    }

    /// How many hits a bar a part has in the nets playing when a control fires: (fewest, most), over its cycles.
    fn hits_per_bar(&self, deck: Deck, role: &str, at: Timing) -> Option<(usize, usize)> {
        let bar = bar_of(at).unwrap_or(0);
        let decks = if self.arranged { &self.segments.iter().rev().find(|s| s.start <= bar).or_else(|| self.segments.first())?.decks } else { &self.decks };
        let part = decks.get(&deck)?.parts.iter().find(|p| p.role == role)?;
        let counts: Vec<usize> = part.cycles.iter().map(Vec::len).collect();
        Some((*counts.iter().min()?, *counts.iter().max()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::check;

    fn canon(src: &str) -> String {
        let c = check(src);
        c.compiled.unwrap_or_else(|| panic!("{}", crate::diag::render_all(&c.diagnostics, src))).text
    }

    fn first_error(src: &str) -> Diagnostic {
        let c = check(src);
        assert!(c.compiled.is_none(), "{src:?} compiled");
        c.diagnostics.into_iter().find(Diagnostic::is_error).expect("an error")
    }

    #[test]
    fn a_duck_prints_with_both_times_and_one_spelling() {
        let want = "deck A: kick x ~ x ~\ndeck B: bass c2\nduck B under A.kick 6db attack=5ms release=120ms\n";
        assert_eq!(canon("deck A: kick x ~ x ~\ndeck B: bass c2\nduck B under A.kick 6db\n"), want);
        assert_eq!(canon("deck A: kick x ~ x ~\ndeck B: bass c2\nduck  B  under  A.kick  6.0db  release=0.12s   attack=5ms\n"), want);
        assert_eq!(canon("deck A: kick x ~ x ~\ndeck B: bass c2\nduck B under A.kick 6db release=2s\n").lines().last().unwrap(), "duck B under A.kick 6db attack=5ms release=2000ms");
    }

    #[test]
    fn a_mix_keeps_its_time_word_in_the_canonical_text() {
        let t = canon("for 4 bars: mix { A level 0.3 }\nevery 8 bars from 4: mix { master level 0.8, B eq high 3 }\non next phrase: mix { A eq mid -1 }\n");
        assert_eq!(t, "for 4 bars: mix {\n  A level 0.3\n}\nevery 8 bars from 4: mix {\n  master level 0.8\n  B eq high 3\n}\non next phrase: mix {\n  A eq mid -1\n}\n");
        assert_eq!(canon(&t), t);
        // a duck too
        let t = canon("deck A: kick x\nat bar 8 for 4 bars: duck B under A.kick 3db\n");
        assert_eq!(t.lines().last().unwrap(), "at bar 8 for 4 bars: duck B under A.kick 3db attack=5ms release=120ms");
        assert_eq!(canon(&t), t);
    }

    #[test]
    fn two_ducks_of_one_deck_are_a_warning() {
        let c = check("deck A: kick x\nduck B under A.kick 6db\nduck B under A.kick 3db\n");
        assert!(c.ok() && c.diagnostics.iter().any(|d| d.message.contains("already ducked by an earlier line")), "{:?}", c.diagnostics);
        let c = check("deck A: kick x\nduck B under A.kick 6db\nsection one 4 bars {\n  duck B under A.kick 3db\n}\n");
        assert!(c.ok() && c.diagnostics.iter().any(|d| d.message.contains("ducked outside the sections too")), "{:?}", c.diagnostics);
        // a duck in each of two sections, or two decks, is fine
        let c = check("deck A: kick x\ndeck B: kick x\nsection one 4 bars {\n  duck B under A.kick 3db\n}\nsection two 4 bars {\n  duck B under A.kick 3db\n  duck A under B.kick 3db\n}\n");
        assert!(c.ok() && c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    }

    #[test]
    fn a_mix_prints_one_item_a_line() {
        let src = "section drop 8 bars {\n  deck A: kick x ~ x ~\n  mix { A level 0.7, B level 0.9\n    A eq low -3db }\n}\n";
        let t = canon(src);
        assert!(t.contains("  mix {\n    A level 0.7\n    B level 0.9\n    A eq low -3\n  }\n"), "{t}");
        assert_eq!(canon(&t), t);
        // outside a section, and with a time word
        let t = canon("at bar 4: mix { master level 0.8 }\n");
        assert_eq!(t, "at bar 4: mix {\n  master level 0.8\n}\n");
    }

    #[test]
    fn arithmetic_in_an_item_is_folded_to_its_value() {
        let t = canon("mix { A level 0.5 + 0.2, A eq low 0 - 3, B level 1 / 4, master level (1 - 0.1) * 0.5 }\n");
        assert_eq!(t, "mix {\n  A level 0.7\n  A eq low -3\n  B level 0.25\n  master level 0.45\n}\n");
        assert_eq!(canon(&t), t);
        // the same program as its folded spelling
        assert_eq!(check("mix { A level 0.5 + 0.2 }\n").compiled.unwrap().cid, check("mix { A level 0.7 }\n").compiled.unwrap().cid);
        let c = check("mix { A level 0.5 + 0.2 }\n").compiled.unwrap();
        let Effect::Mix(m) = &c.controls[0].effect else { panic!() };
        assert_eq!(m.items[0].knob, 0.7);
    }

    #[test]
    fn the_pump_is_chosen_from_the_depth_and_the_release() {
        assert_eq!(pump_depth(6.0), 0.499);
        assert_eq!(pump_depth(12.0), 0.749);
        // 120 ms at 124 BPM is a 1/16 (121 ms); at 90 BPM the 1/24 comes nearest (111 ms)
        assert_eq!(pump_div(120.0, 124.0).0, 16);
        assert_eq!(pump_div(120.0, 90.0).0, 24);
        // longer than the pump goes: a quarter note, whatever the tempo
        assert_eq!(pump_div(5000.0, 124.0).0, 4);
        assert_eq!(pump_div(1.0, 124.0).0, 32);
        let p = DuckPlan { deck: Deck::B, source: Deck::A, role: "kick".into(), db: 6.0, attack_ms: 5.0, release_ms: 120.0, bpm: Some(124.0) };
        assert_eq!(p.on_call(124.0).text(), r#"trick {"action":"on","amount":0.499,"deck":"B","div":16,"kind":"pump","source":"A"}"#);
        assert_eq!(p.off_call().text(), r#"trick {"action":"off","deck":"B","kind":"pump"}"#);
    }

    #[test]
    fn the_eq_knob_is_the_mixers_own_scale() {
        assert_eq!(eq_knob(0.0), 0.5);
        assert_eq!(eq_knob(-3.0), 0.4375);
        assert_eq!(eq_knob(-24.0), 0.0);
        assert_eq!(eq_knob(3.0), 0.75);
        assert_eq!(eq_knob(6.0), 1.0);
        for db in [-24.0, -12.0, -3.0, 0.0, 2.0, 6.0] {
            assert!((eq_db(eq_knob(db)) - db).abs() < 1e-4, "{db}");
        }
    }

    #[test]
    fn the_mixer_is_read_from_its_state() {
        let j = JsonValue::parse(r#"{"set":[],"mixer":{"strips":[{"deck":"A","volume":0.65,"eq":{"high":0.5,"mid":0.5,"low":0.3,"filter":0.5}},{"deck":"B","volume":0.8,"eq":{"high":0.5,"mid":0.5,"low":0.5,"filter":0.5}}],"master":0.9}}"#).unwrap();
        let r = read_mixer(&j);
        assert_eq!(r.values[&MixTarget::Level(Deck::A)], 0.65);
        assert_eq!(r.values[&MixTarget::Eq(Deck::A, Band::Low)], 0.3);
        assert_eq!(r.values[&MixTarget::Master], 0.9);
        assert_eq!(r.values.len(), 9);
        // a host that cannot answer: nothing read
        assert!(read_mixer(&JsonValue::Null).values.is_empty());
    }

    /// The engine's own answer to `mixer {}` (`djapp`'s `c_mixer`: `{set, mixer: MixerState::to_json}`) reads back, and the
    /// engine's starting positions are the neutral ones a snapshot falls back to.
    #[test]
    fn the_engines_own_mixer_state_is_what_is_read() {
        use djengine::mixer::MixerState;
        let mut m = MixerState::new(&["A", "B"]);
        m.strips[0].volume = 0.65;
        m.strips[0].eq.eq_low = 0.3;
        m.master = 0.75;
        let answer = JsonValue::Object(JsonObject::from_pairs([("set", JsonValue::Array(Vec::new())), ("mixer", m.to_json())]));
        let r = read_mixer(&answer);
        assert_eq!(r.values[&MixTarget::Level(Deck::A)], 0.65);
        assert_eq!(r.values[&MixTarget::Eq(Deck::A, Band::Low)], 0.3);
        assert_eq!(r.values[&MixTarget::Master], 0.75);
        // a fresh mixer is the engine's start
        let fresh = read_mixer(&MixerState::new(&["A", "B"]).to_json());
        for (t, v) in fresh.values {
            assert_eq!(v, t.neutral(), "{t:?}");
        }
        // and the knob scale is the engine's: the EQ's dB is `fx::strip::eq_decibels`
        for db in [-24.0, -9.0, -3.0, 0.0, 3.0, 6.0] {
            assert!((djengine::fx::strip::eq_decibels(eq_knob(db)) - db).abs() < 1e-3, "{db}");
        }
    }

    fn set(target: MixTarget, knob: f64, label: &str, fallback: f64, from_program: bool) -> MixSet {
        MixSet { target, knob, label: label.into(), text: format!("{label} {knob}"), span: Span::default(), fallback, from_program }
    }

    #[test]
    fn a_snapshot_goes_back_to_what_the_mixer_held_or_to_the_programs_own_line() {
        let plan = MixPlan { items: vec![set(MixTarget::Level(Deck::A), 0.7, "A level", 0.6, true), set(MixTarget::Master, 0.9, "master level", 1.0, false)], restore: true };
        let read = JsonValue::parse(r#"{"mixer":{"strips":[{"deck":"A","volume":0.55}],"master":0.75}}"#).unwrap();
        let f = plan.fire(Some(&read));
        assert_eq!(f.sets.iter().map(Call::text).collect::<Vec<_>>(), [r#"mixer {"deck":"A","volume":0.7}"#, r#"mixer {"master":0.9}"#]);
        assert_eq!(f.back.iter().map(Call::text).collect::<Vec<_>>(), [r#"mixer {"master":0.75}"#, r#"mixer {"deck":"A","volume":0.55}"#]);
        assert!(f.words.is_empty());
        // no read: the program's own line, else neutral, and it says so
        let f = plan.fire(None);
        assert_eq!(f.back.iter().map(Call::text).collect::<Vec<_>>(), [r#"mixer {"master":1}"#, r#"mixer {"deck":"A","volume":0.6}"#]);
        assert!(f.words[0].contains("cannot be read") && f.words[0].contains("the program's own last line") && f.words[0].contains("neutral"), "{:?}", f.words);
        // a plain mix reads nothing and puts nothing back
        let plain = MixPlan { restore: false, ..plan };
        let f = plain.fire(Some(&read));
        assert_eq!(f.sets.len(), 2);
        assert!(f.back.is_empty() && f.words.is_empty());
    }

    #[test]
    fn errors_say_where_and_what_to_write() {
        let e = first_error("duck B under A.kick 6 db\n");
        assert!(e.message.contains("glued") && e.help.as_deref().unwrap().contains("6db"), "{e:?}");
        let e = first_error("duck B under A.kick 6\n");
        assert!(e.message.contains("needs its unit") && e.help.as_deref() == Some("write 6db"), "{e:?}");
        let e = first_error("duck B under A.kick 6ms\n");
        assert!(e.message.contains("decibels"), "{e:?}");
        let e = first_error("duck B under A.kick -6db\n");
        assert!(e.message.contains("no sign"), "{e:?}");
        let e = first_error("duck B under A\n");
        assert!(e.message.contains("takes a deck's part"), "{e:?}");
        let e = first_error("duck B under A kick 6db\n");
        assert_eq!(e.help.as_deref(), Some("write `A.kick`, with no space"));
        let e = first_error("duck B under A.kik 6db\n");
        assert_eq!(e.help.as_deref(), Some("did you mean `kick`?"));
        let e = first_error("duck B beneath A.kick 6db\n");
        assert!(e.message.contains("expected `under`"), "{e:?}");
        let e = first_error("duck b under A.kick 6db\n");
        assert_eq!(e.help.as_deref(), Some("decks are upper case: `B`"));
        let e = first_error("duck B under A.kick 6db atack=5ms\n");
        assert_eq!(e.help.as_deref(), Some("did you mean `attack`?"));
        let e = first_error("duck B under A.kick 6db attack=5\n");
        assert!(e.message.contains("needs its unit"), "{e:?}");
        let e = first_error("duck B under A.kick 6db release=5db\n");
        assert!(e.message.contains("a time"), "{e:?}");
        let e = first_error("duck B under A.kick 6db attack=5ms attack=6ms\n");
        assert!(e.message.contains("twice"), "{e:?}");
        let e = first_error("duck B under A.kick 6db attack 5ms\n");
        assert!(e.message.contains("is written `attack=5ms`"), "{e:?}");
        // the lexer's own: a unit in the wrong case
        let c = check("duck B under A.kick 6dB\n");
        let lexer = c.diagnostics.iter().find(|d| d.message.contains("not a number or a word")).unwrap();
        assert!(lexer.help.as_deref().unwrap().contains("`6db`"), "{:?}", c.diagnostics);
    }

    #[test]
    fn a_mix_block_names_what_is_wrong() {
        let e = first_error("mix { A leval 0.7 }\n");
        assert_eq!(e.help.as_deref(), Some("did you mean `level`?"));
        let e = first_error("mix { A eq lo -3 }\n");
        assert_eq!(e.help.as_deref(), Some("did you mean `low`?"));
        let e = first_error("mix { A low -3 }\n");
        assert_eq!(e.help.as_deref(), Some("an EQ band is `A eq low -3`"));
        let e = first_error("mix { a level 0.7 }\n");
        assert_eq!(e.help.as_deref(), Some("decks are upper case: `A`"));
        let e = first_error("mix { master leval 0.7 }\n");
        assert_eq!(e.help.as_deref(), Some("did you mean `level`?"));
        let e = first_error("mix { }\n");
        assert!(e.message.contains("nothing in it"), "{e:?}");
        let e = first_error("mix { A level 0.7 B level 0.9 }\n");
        assert!(e.message.contains("expected `,`"), "{e:?}");
        let e = first_error("mix A level 0.7\n");
        assert!(e.message.contains("expected `{`"), "{e:?}");
        let e = first_error("mix {\n  A level 0.7\n");
        assert!(e.message.contains("never closed"), "{e:?}");
        // one error, not one for every line left in the block
        let c = check("mix {\n  A leval 0.7\n  B level 0.9\n}\n");
        assert_eq!(c.diagnostics.iter().filter(|d| d.is_error()).count(), 1, "{:?}", c.diagnostics);
        let e = first_error("deck A: mix { A level 0.7 }\n");
        assert!(e.message.contains("takes no `deck X:`"), "{e:?}");
    }

    #[test]
    fn the_numbers_of_a_mix_have_ranges() {
        assert!(first_error("mix { A level 1.5 }\n").message.contains("0 to 1"));
        assert!(first_error("mix { A eq low -30 }\n").message.contains("-24 to +6"));
        assert!(first_error("mix { A eq low 9 }\n").message.contains("-24 to +6"));
        assert!(first_error("mix { A level 0.7db }\n").message.contains("only an EQ band is in dB"));
        assert!(first_error("mix { A level 0.7, A level 0.8 }\n").message.contains("set twice"));
        assert!(first_error("mix { A level 1 / 0 }\n").message.contains("division by zero"));
        let e = first_error("mix { A pan -1 }\n");
        assert!(e.message.contains("no pan") && e.help.as_deref().unwrap().contains("aux macro=hard-left"), "{e:?}");
        assert!(first_error("duck B under A.kick 0db\ndeck A: kick x\n").message.contains("does nothing"));
        assert!(first_error("duck B under A.kick 70db\ndeck A: kick x\n").message.contains("at most"));
        assert!(first_error("duck B under A.kick 6db release=0ms\ndeck A: kick x\n").message.contains("`release` is"));
    }

    #[test]
    fn a_duck_needs_a_deck_with_the_part_that_keys_it() {
        let e = first_error("duck B under B.kick 6db\ndeck B: kick x\n");
        assert!(e.message.contains("cannot duck under itself"), "{e:?}");
        let e = first_error("duck B under A.kick 6db\ndeck A: snare x\n");
        assert!(e.message.contains("no `kick` part") && e.help.as_deref() == Some("deck A plays: snare"), "{e:?}");
        let e = first_error("duck B under A.kick 6db\n");
        assert!(e.help.as_deref().unwrap().starts_with("give deck A the part"), "{e:?}");
        let e = first_error("duck B under A.snare 6db\ndeck A: snare x\n");
        assert!(e.message.contains("cannot key a duck") && e.help.as_deref() == Some("duck B under A.kick 6db"), "{e:?}");
        // bass keys it, with a word of warning
        let c = check("duck B under A.bass 6db\ndeck A: bass c2\n");
        assert!(c.ok() && c.diagnostics.iter().any(|d| d.message.contains("low end")), "{:?}", c.diagnostics);
        // in a section: the part is the section's (or the song's)
        let src = "section one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n  duck B under A.kick 6db\n}\narrange one two\n";
        let e = first_error(src);
        assert!(e.message.contains("in section `two`"), "{e:?}");
        let ok = "deck A: kick x ~ x ~\nsection one 4 bars {\n  duck B under A.kick 6db\n}\narrange one\n";
        assert!(check(ok).ok());
        // outside every section the sections without the part are named
        let c = check("duck B under A.kick 6db\nsection one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n}\narrange one two\n");
        assert!(c.ok() && c.diagnostics.iter().any(|d| d.message.contains("no `kick` in section two")), "{:?}", c.diagnostics);
    }

    #[test]
    fn a_section_holds_its_duck_and_its_mix_to_its_last_bar() {
        let src = "tempo 124\nsection intro 4 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 8 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  duck B under A.kick 6db\n  mix { A level 0.7 }\n  at bar 2: mix { B level 0.5 }\n}\narrange intro drop\n";
        let c = check(src);
        let comp = c.compiled.unwrap_or_else(|| panic!("{}", crate::diag::render_all(&c.diagnostics, src)));
        let ctl: Vec<_> = comp.controls.iter().filter(|c| matches!(c.effect, Effect::Duck(_) | Effect::Mix(_))).collect();
        assert_eq!(ctl.len(), 3);
        // drop is bars 4..12: it fires on its first bar and is put back on the bar it ends on (12)
        assert_eq!((ctl[0].timing, ctl[0].hold), (Timing::At(4), Some(Dur::bars(8))));
        assert_eq!((ctl[1].timing, ctl[1].hold), (Timing::At(4), Some(Dur::bars(8))));
        assert_eq!((ctl[2].timing, ctl[2].hold), (Timing::At(6), Some(Dur::bars(6))));
        assert_eq!(ctl[0].undo[0].text(), r#"trick {"action":"off","deck":"B","kind":"pump"}"#);
        let Effect::Mix(m) = &ctl[1].effect else { panic!() };
        assert!(m.restore);
        // the report names the calls and the bar it is put back on
        let rep = comp.mix_report();
        assert_eq!(rep.len(), 3);
        assert!(rep[0].contains(r#"trick {"action":"on","amount":0.499,"deck":"B","div":16,"kind":"pump","source":"A"}"#) && rep[0].contains("put back on bar 12") && rep[0].contains("plays 2 hits a bar"), "{rep:?}");
        assert!(rep[1].contains(r#"mixer {"deck":"A","volume":0.7}"#) && rep[1].contains("reads the mixer first") && rep[1].contains("put back on bar 12"), "{rep:?}");
        // a repeating time word does not fit a snapshot
        let e = first_error("section one 4 bars {\n  deck A: kick x\n  every 2 bars: mix { A level 0.5 }\n}\n");
        assert!(e.message.contains("fires once"), "{e:?}");
        let e = first_error("section one 4 bars {\n  deck A: kick x\n  at bar 9: mix { A level 0.5 }\n}\n");
        assert!(e.message.contains("outside section `one`"), "{e:?}");
    }

    #[test]
    fn outside_a_section_a_mix_just_sets_and_for_n_bars_restores() {
        let comp = check("at bar 4: mix { A level 0.7 }\nat bar 8 for 2 bars: mix { A eq low -3 }\nduck B under A.kick 6db\ndeck A: kick x\n").compiled.unwrap();
        let Effect::Mix(a) = &comp.controls[0].effect else { panic!() };
        let Effect::Mix(b) = &comp.controls[1].effect else { panic!() };
        assert!(!a.restore && comp.controls[0].hold.is_none());
        assert!(b.restore && comp.controls[1].hold == Some(Dur::bars(2)));
        assert_eq!(b.items[0].call().text(), r#"mixer {"deck":"A","low":0.4375}"#);
        // a duck with nothing to say how long it lasts is kept until the program stops
        assert_eq!(comp.controls[2].hold, None);
        assert!(comp.mix_report()[2].contains("kept until the program stops"), "{:?}", comp.mix_report());
    }

    #[test]
    fn the_fallback_is_the_programs_own_last_line_then_neutral() {
        let src = "deck A: level 0.6\nmaster: level 0.9\nsection one 4 bars {\n  deck A: kick x\n  mix { A level 0.3, B level 0.2, master level 0.5, A eq mid -6 }\n}\n";
        let comp = check(src).compiled.unwrap();
        let Effect::Mix(m) = &comp.controls.last().unwrap().effect else { panic!() };
        let by = |t: MixTarget| m.items.iter().find(|i| i.target == t).unwrap();
        assert_eq!((by(MixTarget::Level(Deck::A)).fallback, by(MixTarget::Level(Deck::A)).from_program), (0.6, true));
        assert_eq!((by(MixTarget::Master).fallback, by(MixTarget::Master).from_program), (0.9, true));
        assert_eq!((by(MixTarget::Level(Deck::B)).fallback, by(MixTarget::Level(Deck::B)).from_program), (NEUTRAL_LEVEL, false));
        assert_eq!(by(MixTarget::Eq(Deck::A, Band::Mid)).fallback, NEUTRAL_EQ);
    }

    #[test]
    fn a_def_can_take_the_decks_and_the_levels() {
        let src = "def pump(under, src, db) {\n  duck $under under $src.kick $db\n  mix { A level 0.6 }\n}\ndeck A: kick x ~ x ~\npump(B, A, 6db)\n";
        let c = check(src);
        let comp = c.compiled.unwrap_or_else(|| panic!("{}", crate::diag::render_all(&c.diagnostics, src)));
        assert!(comp.text.contains("duck B under A.kick 6db attack=5ms release=120ms") || comp.text.contains("duck $under under $src.kick $db"), "{}", comp.text);
        assert_eq!(comp.mix_report().len(), 2);
    }
}
