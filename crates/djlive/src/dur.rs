//! F1 sub-bar timing (docs/btw/ext-timing.md): durations and positions in exact bars, and the parser's way of
//! reading them. Small, public and stable on purpose: the tempo curves, shapes and lfos of the other extensions
//! (`over dur`, `every dur`, `at pos`) read their time with [`Parser::dur`] and [`Parser::pos`] and keep it as a
//! [`Dur`] or a [`Pos`].
//!
//! ```text
//! dur  = arith ( "bar" | "bars" | "beat" | "beats" ) ;     (* over 1/2 bar   over 2 beats   every 3/4 bar *)
//! pos  = "bar" arith [ "beat" arith ] ;                    (* at bar 16   at bar 16 beat 3   at bar 16 beat 5/2 *)
//! ```
//!
//! A beat is a quarter of a bar. Both types are one [`Rat`] of bars, so `1/2 bar` and `2 beats` are the same
//! value; the canonical text of a duration is `N bars` when `N` is whole, else `N beats` (`1 beat`) when the beats
//! are whole, else `P/Q bars` in lowest terms; of a position `bar W` when whole, else `bar W beat B`.

use std::fmt;

use crate::diag::{Diagnostic, Span};
use crate::lex::Tok;
use crate::parse::Parser;
use crate::rat::Rat;

/// Beats in a bar.
pub const BEATS_PER_BAR: i64 = 4;

/// The shortest `every`: 1/64 of a bar, a sixteenth of a beat. A line that repeats finer than that is finer than the
/// runtime's clock can follow (and would flood the host), so it is an error that says so.
pub fn min_every() -> Rat {
    Rat::new(1, 64).unwrap_or(Rat::ONE)
}

/// A length of time in bars (exact, never negative): `over 8 bars`, `for 2 beats`, `every 1/2 bar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dur(Rat);

/// A place in the program in bars from its first bar line (exact, never negative): `bar 16` is 16, `bar 16 beat 3`
/// is 16 3/4. Bars and beats count from 0, as bars always did: `beat 0` is the bar line itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos(Rat);

impl Dur {
    pub const ZERO: Dur = Dur(Rat::ZERO);

    /// `n` whole bars.
    pub fn bars(n: u32) -> Dur {
        Dur(Rat::int(i64::from(n)))
    }

    /// A length of `r` bars; `None` when it is negative.
    pub fn from_bars(r: Rat) -> Option<Dur> {
        (!r.is_neg()).then_some(Dur(r))
    }

    /// A length of `r` beats; `None` when it is negative (or too large to count).
    pub fn from_beats(r: Rat) -> Option<Dur> {
        r.div(Rat::int(BEATS_PER_BAR)).ok().and_then(Dur::from_bars)
    }

    /// The length in bars, exactly.
    pub fn as_bars(self) -> Rat {
        self.0
    }

    /// The length in beats, exactly (`None` only if it does not fit).
    pub fn as_beats(self) -> Option<Rat> {
        self.0.mul(Rat::int(BEATS_PER_BAR)).ok()
    }

    pub fn to_f64(self) -> f64 {
        self.0.to_f64()
    }

    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    /// The number of bars when it is a whole number of them.
    pub fn whole_bars(self) -> Option<u32> {
        self.0.as_u32()
    }

    /// The canonical text: `4 bars`, `2 beats`, `1 beat`, `1/3 bars`.
    pub fn canonical(self) -> String {
        if self.0.is_int() {
            return format!("{} bars", self.0);
        }
        match self.as_beats() {
            Some(b) if b.is_int() => {
                if b == Rat::ONE {
                    "1 beat".to_string()
                } else {
                    format!("{b} beats")
                }
            }
            _ => format!("{} bars", self.0),
        }
    }
}

impl fmt::Display for Dur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl Pos {
    pub const START: Pos = Pos(Rat::ZERO);

    /// The bar `n` (its line).
    pub fn bar(n: u32) -> Pos {
        Pos(Rat::int(i64::from(n)))
    }

    /// `r` bars from the start; `None` when it is negative.
    pub fn from_bars(r: Rat) -> Option<Pos> {
        (!r.is_neg()).then_some(Pos(r))
    }

    /// The position in bars, exactly.
    pub fn as_bars(self) -> Rat {
        self.0
    }

    pub fn to_f64(self) -> f64 {
        self.0.to_f64()
    }

    /// The bar it falls in (`bar 16 beat 3` is in bar 16).
    pub fn bar_of(self) -> i64 {
        self.0.floor()
    }

    /// How far into its bar it is, in beats: `0 <= b < 4`.
    pub fn beat_of(self) -> Rat {
        let w = Rat::int(self.0.floor());
        self.0.sub(w).and_then(|r| r.mul(Rat::int(BEATS_PER_BAR))).unwrap_or(Rat::ZERO)
    }

    /// The bar number when it falls exactly on a bar line.
    pub fn whole_bar(self) -> Option<u32> {
        self.0.as_u32()
    }

    /// The canonical text: `bar 16`, `bar 16 beat 3`, `bar 16 beat 5/2`.
    pub fn canonical(self) -> String {
        if self.0.is_int() {
            format!("bar {}", self.0)
        } else {
            format!("bar {} beat {}", self.bar_of(), self.beat_of())
        }
    }

    /// What follows `from` in the canonical text of `every ... from POS`: a whole bar stays a bare number (as
    /// it always was), anything else is spelled `bar W beat B`.
    pub fn canonical_from(self) -> String {
        match self.whole_bar() {
            Some(n) => n.to_string(),
            None => self.canonical(),
        }
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

type R<T> = Result<T, Diagnostic>;

impl Parser<'_> {
    /// A duration: `arith` then `bar`, `bars`, `beat` or `beats`. Never negative. Zero is allowed here (the
    /// caller says what a zero length means); the span covers the number and the unit.
    pub(crate) fn dur(&mut self, what: &str) -> R<(Dur, Span)> {
        let (n, nsp) = self.arith_rat(what)?;
        let unit = match self.peek().clone() {
            Tok::Word(w) if matches!(w.as_str(), "bar" | "bars" | "beat" | "beats") => w,
            _ => {
                let mut help = "a length is `4 bars`, `1/2 bar` or `2 beats`".to_string();
                if let Tok::Word(w) = self.peek() {
                    if let Some(n) = crate::parse::nearest(w, ["bars", "bar", "beats", "beat"]) {
                        help = format!("did you mean `{n}`? {help}");
                    }
                }
                return Err(self.expected(&["`bars`", "`beats`"], &format!("expected `bars` or `beats` after the number, found {}", self.found())).help(help));
            }
        };
        let usp = self.bump().span;
        let span = nsp.to(usp);
        let d = if unit.starts_with("beat") { Dur::from_beats(n) } else { Dur::from_bars(n) };
        match d {
            Some(d) => Ok((d, span)),
            None => Err(Diagnostic::error(span, "a length is not negative").found(span.text(self.src).to_string())),
        }
    }

    /// A position: `bar N` or `bar N beat B` (the `bar` is the next word). `N` and `B` are arithmetic; `N` may be
    /// a rational (`bar 3/2` is bar 1 beat 2); `B` is a beat of the bar, `0 <= B < 4`, counted from 0 like bars.
    pub(crate) fn pos(&mut self) -> R<(Pos, Span)> {
        let start = self.span();
        self.expect_word("bar")?;
        let (bar, bsp) = self.arith_rat("a bar number")?;
        if bar.is_neg() {
            return Err(Diagnostic::error(bsp, "a bar number is not negative").found(bsp.text(self.src).to_string()));
        }
        let mut total = bar;
        let mut end = bsp;
        if self.word_is("beat") {
            self.bump();
            let (b, esp) = self.arith_rat("a beat of the bar")?;
            if b.is_neg() || b >= Rat::int(BEATS_PER_BAR) {
                let what = if b.is_neg() { "a beat is 0 to 4, and not negative" } else { "a beat is 0 to 4 (not 4 itself)" };
                return Err(Diagnostic::error(esp, format!("{what}: beats count from 0 in a bar of four, like bars"))
                    .found(esp.text(self.src).to_string())
                    .help("`at bar 16 beat 3` is the last quarter of bar 16; `at bar 17` is the next bar line"));
            }
            total = b.div(Rat::int(BEATS_PER_BAR)).and_then(|q| bar.add(q)).map_err(|e| Diagnostic::error(esp, e.to_string()))?;
            end = esp;
        }
        match Pos::from_bars(total) {
            Some(p) => Ok((p, start.to(end))),
            None => Err(Diagnostic::error(start.to(end), "a position is not negative")),
        }
    }

    /// What follows `from` in `every DUR from ...`: a bar number as it always was (`from 8`, a rational is
    /// fine), or a position (`from bar 3 beat 2`).
    pub(crate) fn pos_from(&mut self) -> R<(Pos, Span)> {
        if self.word_is("bar") {
            return self.pos();
        }
        let (r, sp) = self.arith_rat("the bar to start on")?;
        match Pos::from_bars(r) {
            Some(p) => Ok((p, sp)),
            None => Err(Diagnostic::error(sp, "a bar number is not negative").found(sp.text(self.src).to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rat {
        Rat::new(n, d).unwrap()
    }

    #[test]
    fn durations_print_the_way_the_contract_says() {
        for (bars, text) in [(r(4, 1), "4 bars"), (r(1, 1), "1 bars"), (r(0, 1), "0 bars"), (r(1, 2), "2 beats"), (r(1, 4), "1 beat"), (r(3, 4), "3 beats"), (r(5, 4), "5 beats"), (r(1, 3), "1/3 bars"), (r(1, 8), "1/8 bars"), (r(5, 2), "10 beats")] {
            assert_eq!(Dur::from_bars(bars).unwrap().canonical(), text);
        }
        assert_eq!(Dur::from_beats(r(2, 1)), Dur::from_bars(r(1, 2)));
        assert!(Dur::from_bars(r(-1, 2)).is_none());
        assert_eq!(Dur::bars(8).whole_bars(), Some(8));
        assert_eq!(Dur::from_bars(r(1, 2)).unwrap().whole_bars(), None);
    }

    #[test]
    fn positions_print_the_way_the_contract_says() {
        for (bars, text) in [(r(16, 1), "bar 16"), (r(67, 4), "bar 16 beat 3"), (r(33, 2), "bar 16 beat 2"), (r(131, 8), "bar 16 beat 3/2"), (r(0, 1), "bar 0"), (r(7, 3), "bar 2 beat 4/3")] {
            assert_eq!(Pos::from_bars(bars).unwrap().canonical(), text, "{bars}");
        }
        let p = Pos::from_bars(r(67, 4)).unwrap();
        assert_eq!((p.bar_of(), p.beat_of(), p.whole_bar()), (16, Rat::int(3), None));
        assert_eq!(Pos::bar(8).canonical_from(), "8");
        assert_eq!(p.canonical_from(), "bar 16 beat 3");
    }
}
