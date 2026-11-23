//! F7 seeded chance: `x?0.3`, `choose [a b c] weights 3 1 1 seed=4`, `maybe(p, 0.3, 4)` (docs/BEATSCRIPT-EXTENSIONS.md,
//! docs/beatscript/ext-chance.md).
//!
//! **Chance is a function of the text and the bar, never of a runtime random.** A hit is kept (or an option picked) by an
//! integer hash of `(seed, bar, cycle, step, note)` against the exact probability: [`hash`] is SplitMix64's finalizer
//! chained over the inputs, a probability is a [`Rat`] (`0.3` is `3/10`, exactly as typed) and the comparison is
//! integer (`hash < floor(p * 2^32)`), so the same program is the same notes on every machine and in every run, and
//! every bar can differ.
//!
//! * `bar` is the pattern cycle being read from the top (the net's bar for a bar part; the part's own cycle for a `/N`
//!   part), `cycle` the cycle the term itself is in (it differs from `bar` inside `<..>`, `fast`, `slow` and `every`, so the
//!   same term in two places makes two decisions), `step` the event's place in the cycle in 960ths (60 to a sixteenth)
//!   taken from the slot the term fills, and `note` the MIDI note (-1 for a drum hit).
//! * The seed is the term's own (`choose ... seed=4`, `maybe(p, 0.3, 4)`), else the part's `seed=`, else a hash of the
//!   part's text (as `markov` does): [`crate::pattern::Ctx::seed`].
//! * A pattern with chance is unrolled over [`unrolled_period`] bars (the section's length up to 64; [`DEFAULT_BARS`]
//!   with no sections), and `check` says so.

use crate::ast::*;
use crate::compile::{Compiler, MAX_BARS};
use crate::diag::{Diagnostic, Span};
use crate::lex::Tok;
use crate::parse::Parser;
use crate::pattern::{self, Ctx, Ev, Sound};
use crate::rat::Rat;

/// The bars a chance pattern is unrolled over in a program with no sections.
pub const DEFAULT_BARS: u32 = 16;
/// The most a chance pattern is unrolled over: one net.
pub const MAX_UNROLL: u32 = MAX_BARS as u32;

/// The words F7 gives a meaning to, for the editor's colours (`highlight.rs`).
pub const HIGHLIGHT_KEYWORDS: [&str; 2] = ["choose", "weights"];

// ---------------------------------------------------------------------------------------------------------------
// the syntax tree

/// A chance, `0.7` or `70%`: as typed (what is printed) and exactly (what is compared).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prob {
    pub value: f64,
    pub rat: Rat,
    pub span: Span,
}

/// `x*8?0.7`: the term in front, its hits each kept with the chance.
#[derive(Clone, Debug, PartialEq)]
pub struct ChanceTerm {
    pub term: Term,
    pub prob: Prob,
}

/// One weight of a `choose`, as typed and exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weight {
    pub value: f64,
    pub rat: Rat,
    pub span: Span,
}

/// `choose [a b c] weights 3 1 1 seed=4`: one option a bar, by weight.
#[derive(Clone, Debug, PartialEq)]
pub struct Choose {
    pub span: Span,
    /// The terms inside the brackets: one option each.
    pub options: Vec<Term>,
    /// One weight an option, in the order of the options.
    pub weights: Vec<Weight>,
    /// The weights as whole numbers over one denominator (what the hash is walked over).
    pub ints: Vec<u64>,
    /// `seed=4`, the term's own.
    pub seed: Option<Num>,
}

/// A number typed with at most six decimals, exactly: the rational it is (`0.3` is `3/10`) and the `f64` that prints as it was
/// typed (so `33.3333%` and `0.333333` are one value, whichever way the lexer's division rounded). None for more decimals, a
/// negative number or a huge one.
pub fn exact_decimal(v: f64) -> Option<(Rat, f64)> {
    if !v.is_finite() || !(0.0..=1e9).contains(&v) {
        return None;
    }
    let k = (v * 1e6).round();
    if (v - k / 1e6).abs() > 1e-9 {
        return None;
    }
    Some((Rat::new(k as i64, 1_000_000).ok()?, k / 1e6))
}

/// Why a weight was refused: too big (a billion is the most), or typed with more than six decimals.
fn bad_weight(v: f64, at: crate::diag::Span, src: &str) -> Diagnostic {
    let why = if v.is_finite() && v > 1e9 { "a weight is at most 1000000000" } else { "a weight has at most six decimals" };
    Diagnostic::error(at, why).found(at.text(src).to_string()).help("weights are relative: `weights 3 1 1` picks the first option three times as often as each of the others")
}

/// A probability that is exactly what was typed: `0..=1` with at most six decimals (`0.3`, `30%`, `0.333333`).
pub fn exact_prob(v: f64) -> Option<Rat> {
    exact_decimal(v).map(|(r, _)| r).filter(|r| *r <= Rat::ONE)
}

/// A whole-number form of the weights over their common denominator, None past what a hash is walked over.
fn integer_weights(ws: &[Rat]) -> Option<Vec<u64>> {
    fn gcd(a: u128, b: u128) -> u128 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let mut d: u128 = 1;
    for w in ws {
        let den = u128::try_from(w.denom()).ok()?;
        d = d.checked_mul(den / gcd(d, den))?;
    }
    ws.iter()
        .map(|w| {
            let num = u128::try_from(w.numer()).ok()?;
            let v = num.checked_mul(d / u128::try_from(w.denom()).ok()?)?;
            u64::try_from(v).ok().filter(|v| *v < (1u64 << 62))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// the hash

/// SplitMix64's finalizer.
fn mix(z: u64) -> u64 {
    let mut z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The hash a decision is made from: the finalizer chained over the inputs, the top 32 bits of it.
pub fn hash(seed: i64, bar: i64, cycle: i64, step: i64, note: i64) -> u32 {
    let mut h: u64 = 0x2438_6e5c_a1b0_5d1f;
    for v in [seed, bar, cycle, step, note] {
        h = mix(h ^ (v as u64));
    }
    (h >> 32) as u32
}

/// `floor(p * 2^32)`: a hash below it keeps the hit. `p = 1` is `2^32` (every hash is below), `p = 0` is 0 (none is).
pub fn threshold(p: Rat) -> u64 {
    let n = u128::try_from(p.numer().max(0)).unwrap_or(0);
    let d = u128::try_from(p.denom().max(1)).unwrap_or(1);
    u64::try_from((n << 32) / d).unwrap_or(1 << 32)
}

/// Does a hash keep a hit of chance `p`.
pub fn kept(h: u32, p: Rat) -> bool {
    u64::from(h) < threshold(p)
}

/// The note a decision is about: a drum hit is -1, a note its MIDI number.
fn note_of(e: &Ev) -> i64 {
    match e.sound {
        Sound::Note(n) => i64::from(n),
        Sound::Hit => -1,
    }
}

/// The option a hash picks: the hash as a share of the weights' sum, walked over the running total. A weight of 0 is
/// never picked; the sum is not 0 (a `choose` says so when it is read).
pub fn pick(weights: &[u64], h: u32) -> usize {
    let total: u128 = weights.iter().map(|w| u128::from(*w)).sum();
    let r = (u128::from(h) * total) >> 32;
    let mut run: u128 = 0;
    for (i, w) in weights.iter().enumerate() {
        run += u128::from(*w);
        if r < run {
            return i;
        }
    }
    weights.len().saturating_sub(1)
}

/// What the note of a `choose` decision is, so it never shares a hash with a hit's.
const CHOOSE_NOTE: i64 = -2;

// ---------------------------------------------------------------------------------------------------------------
// where a term is

/// Where the chance terms are while a pattern is read: the bar from the top, and the slot (offset, width, as exact
/// fractions of the part's cycle) that the term in hand fills.
#[derive(Clone, Copy, Debug)]
pub struct ChanceCtx {
    /// The pattern cycle being read from the top (the net's bar for a bar part).
    pub bar: i64,
    /// The slot the term fills: None when the nesting is so deep it passes `i64` (the place is then the term's own).
    pub slot: Option<(Rat, Rat)>,
    /// Read every chance as if it came up: each `?` and `maybe` hit is kept and every option of a `choose` plays at once. That
    /// is the most the part can ever play, which is what `assert never ...` is judged on (constraint.rs); the notes a program
    /// plays are read with it off.
    pub all: bool,
}

impl Default for ChanceCtx {
    fn default() -> ChanceCtx {
        ChanceCtx { bar: 0, slot: Some((Rat::ZERO, Rat::ONE)), all: false }
    }
}

impl ChanceCtx {
    /// The slot becomes the `j`th of `n` equal parts of `parent`.
    pub fn enter(&mut self, parent: Option<(Rat, Rat)>, j: usize, n: usize) {
        self.slot = parent.and_then(|(off, w)| {
            let n = i64::try_from(n.max(1)).ok()?;
            let j = i64::try_from(j).ok()?;
            let part = w.div(Rat::int(n)).ok()?;
            Some((off.add(part.mul(Rat::int(j)).ok()?).ok()?, part))
        });
    }

    /// The place of an event `at` (0..1) in the slot, in 960ths of the cycle.
    pub fn step_of(&self, at: f64) -> i64 {
        let (off, w) = self.slot.map_or((0.0, 1.0), |(o, w)| (o.to_f64(), w.to_f64()));
        ((off + w * at) * 960.0).round() as i64
    }
}

/// How many bars of the net a chance pattern covers: the structural period `p` (what the alternations and the cycles
/// already make) repeated until it holds `unroll` bars, and never more than one net unless `p` is longer already. A
/// deck whose period is longer than a net keeps it.
pub fn unrolled_period(p: u64, unroll: u32) -> u64 {
    let p = p.max(1);
    let want = u64::from(unroll.clamp(1, MAX_UNROLL));
    if p >= want {
        return p;
    }
    let up = want.div_ceil(p) * p;
    if up <= MAX_BARS {
        up
    } else {
        (MAX_BARS / p).max(1) * p
    }
}

// ---------------------------------------------------------------------------------------------------------------
// reading and printing

impl Parser<'_> {
    /// `?prob` after a term (and again, `x?0.5?0.5`): wraps the term in a chance. The mark and its number are glued to the
    /// term, and the number is `0..=1` or a percent.
    pub(crate) fn chance_suffix(&mut self, mut term: Term) -> Result<Term, Diagnostic> {
        let mut wrapped = false;
        while self.peek() == &Tok::Question {
            let q = self.span();
            if q.start != self.prev_span().end {
                return Err(Diagnostic::error(q, "a chance is written glued to its term: x?0.3").help("x?0.3, x*8?0.7, [x ~ x ~]?50% (no space before the `?` or after it)"));
            }
            self.bump();
            let at = self.span();
            // `x?(1 / 4)`: arithmetic in parentheses; a def's argument that is arithmetic is put in place in parentheses, so
            // `x?$p` with `p = 1 / 4 + 1 / 4` is this
            // a bare `x?` (pattern sugar, sugar.rs) is `x?0.5`: nothing glued after the `?` that a chance could be
            // (a number apart from its `?` is still the error it was: `x? 0.3` is not `x?` and a degree); `x??` is two
            // halves, `x?0.5?0.5`, as the grammar's `{ "?" [ prob ] }` says
            let glued = at.start == q.end;
            let bare = if glued { matches!(self.peek(), Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Comma | Tok::Pipe | Tok::Tilde | Tok::Question | Tok::Newline | Tok::Eof) } else { !matches!(self.peek(), Tok::Num { .. } | Tok::LParen) };
            let (value, at) = if bare {
                (0.5, q)
            } else if self.peek() == &Tok::LParen {
                if at.start != q.end {
                    return Err(Diagnostic::error(at, "a chance is written glued to its `?`: x?0.3").help("x?0.3, x*8?0.7 (no space between the `?` and the number)"));
                }
                let (r, sp) = self.paren_rat()?;
                (r.to_f64(), sp)
            } else {
                let Tok::Num { value, .. } = self.peek().clone() else {
                    return Err(self.expected(&["a chance, 0 to 1 or a percent"], &format!("expected the chance after `?`, found {}", self.found())).help("x?0.3 keeps the hit in 30% of the bars; x?30% says the same"));
                };
                if at.start != q.end {
                    return Err(Diagnostic::error(at, "a chance is written glued to its `?`: x?0.3").help("x?0.3, x*8?0.7 (no space between the `?` and the number)"));
                }
                self.bump();
                (value, at)
            };
            let Some((rat, value)) = exact_decimal(value).filter(|(r, _)| *r <= Rat::ONE) else {
                let why = if (0.0..=1.0).contains(&value) { "a chance has at most six decimals (0.333333)" } else { "a chance is 0 to 1, or 0% to 100%" };
                return Err(Diagnostic::error(at, why).found(at.text(self.src).to_string()).help("x?0.3, x?30%: 0 never keeps the hit and 1 always does"));
            };
            // `x?1` always keeps the hit and `x?0` never does, and a chance of a rest is a rest: each is the program written
            // out (`x`, `~`), so it has that program's CID and `check` does not say it differs every bar (L-09)
            if rat == Rat::ONE || matches!(term.atom, Atom::Rest) {
                wrapped = true;
                continue;
            }
            if rat == Rat::ZERO {
                term = Term { span: term.span.to(at), atom: Atom::Rest, fast: None, accent: false, slide: false, marks: Marks::default() };
                wrapped = true;
                continue;
            }
            let span = term.span.to(at);
            term = Term { span, atom: Atom::Chance(Box::new(ChanceTerm { term, prob: Prob { value, rat, span: at } })), fast: None, accent: false, slide: false, marks: Marks::default() };
            wrapped = true;
        }
        if wrapped && matches!(self.peek(), Tok::Star | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp) {
            return Err(Diagnostic::error(self.span(), format!("{} comes before the chance", self.found())).help("x*8?0.7, c2!?0.5: the repeat and the marks first, then the `?`"));
        }
        Ok(term)
    }

    /// `choose [a b c] weights 3 1 1 [seed=4]` (the `choose` is the next token). One weight for each option, read by
    /// count so that a degree step that follows is not taken for a weight.
    pub(crate) fn choose_atom(&mut self) -> Result<Atom, Diagnostic> {
        let start = self.span();
        self.bump();
        if self.peek() != &Tok::LBrack {
            return Err(self
                .expected(&["`[`"], &format!("`choose` takes its options in brackets, found {}", self.found()))
                .help("choose [a b c] weights 3 1 1: one of a, b, c a bar, a three times as often as b or c"));
        }
        self.bump();
        let seq = self.seq()?;
        if self.peek() == &Tok::Comma {
            return Err(Diagnostic::error(self.span(), "a choice picks one term: a comma would stack").help("choose [a b c] weights 1 1 1: the options are the terms, one space apart; to play two at once, group them: choose [[a, b] c] weights 1 1"));
        }
        if self.peek() != &Tok::RBrack {
            return Err(self.expected(&["`]`", "another option"], &format!("expected `]` to close the options, found {}", self.found())));
        }
        let close = self.bump().span;
        if seq.terms.is_empty() {
            return Err(Diagnostic::error(start.to(close), "a choice has no options").help("choose [a b c] weights 1 1 1"));
        }
        let n = seq.terms.len();
        if !self.word_is("weights") {
            return Err(self
                .expected(&["`weights`"], &format!("`choose` needs its weights after the options, found {}", self.found()))
                .help(format!(
                    "{}choose [{}] weights {}",
                    if matches!(self.peek(), Tok::Word(w) if crate::parse::nearest(w, ["weights"]).is_some()) { "did you mean `weights`? " } else { "" },
                    crate::print::seq(&seq),
                    vec!["1"; n].join(" ")
                )));
        }
        self.bump();
        let mut weights: Vec<Weight> = Vec::new();
        for i in 0..n {
            let at = self.span();
            match self.peek().clone() {
                Tok::Num { value, .. } => {
                    self.bump();
                    let Some((rat, value)) = exact_decimal(value) else {
                        return Err(bad_weight(value, at, self.src));
                    };
                    weights.push(Weight { value, rat, span: at });
                }
                // `weights (1 + 1) 1 1`: arithmetic in parentheses (a def's argument that is arithmetic is put in place so)
                Tok::LParen => {
                    let (r, sp) = self.paren_rat()?;
                    if r.is_neg() {
                        return Err(Diagnostic::error(sp, "a weight is not negative").help("weights 3 1 1: how often each option is picked, against the others (0 never)"));
                    }
                    let Some((rat, value)) = exact_decimal(r.to_f64()) else {
                        return Err(bad_weight(r.to_f64(), sp, self.src));
                    };
                    weights.push(Weight { value, rat, span: sp });
                }
                Tok::Minus => return Err(Diagnostic::error(at, "a weight is not negative").help("weights 3 1 1: how often each option is picked, against the others (0 never)")),
                _ => {
                    let s = if i == 1 { "" } else { "s" };
                    return Err(Diagnostic::error(start.to(self.prev_span()), format!("this `choose` has {n} options and {i} weight{s}: it needs one weight for each option"))
                        .found(self.found())
                        .help(format!("choose [{}] weights {}", crate::print::seq(&seq), vec!["1"; n].join(" "))));
                }
            }
        }
        let rats: Vec<Rat> = weights.iter().map(|w| w.rat).collect();
        let Some(ints) = integer_weights(&rats) else {
            return Err(Diagnostic::error(weights[0].span.to(weights[n - 1].span), "these weights are too fine to count").help("weights 3 1 1 or 0.5 0.25 0.25"));
        };
        if ints.iter().all(|w| *w == 0) {
            return Err(Diagnostic::error(weights[0].span.to(weights[n - 1].span), "every weight is 0: nothing could be picked").help("weights 3 1 1: give at least one option a weight above 0"));
        }
        let mut seed = None;
        if self.word_is("seed") && self.peek_n(1) == &Tok::Eq {
            self.bump();
            self.bump();
            let (v, sp) = self.int("a seed")?;
            seed = Some(Num::lit(f64::from(v), true, sp));
        } else if let Tok::Num { int: false, value } = self.peek().clone() {
            // `choose [a b] weights 0.5 0.25 0.25`: one weight too many (a fraction is no step a pattern could go on with)
            return Err(Diagnostic::error(self.span(), format!("this `choose` has {n} options and more than {n} weights: {} is one too many", crate::print::num(value)))
                .help(format!("choose [{}] weights {}", crate::print::seq(&seq), vec!["1"; n].join(" "))));
        } else if let Tok::Num { int: true, value } = self.peek().clone() {
            // `choose [a b] weights 3 1 2`: the 2 is a step after the choice, because the weights are read by count
            let at = self.span();
            self.diags.push(
                Diagnostic::warning(at, format!("this {value} is read as a degree step after the choice: its {n} weights were read already"))
                    .help(format!("weights are counted: one for each of the {n} options. A seed written right after them (seed=4) ends the choice")),
            );
        } else if let Tok::Num { int: false, value } = self.peek().clone() {
            // `choose [a] weights 0.5 0.5`: one weight too many, and a decimal is no degree step to read it as
            let s = if n == 1 { "" } else { "s" };
            return Err(Diagnostic::error(self.span(), format!("this `choose` has {n} option{s} and more than {n} weight{s}: it needs one weight for each option"))
                .found(crate::print::num(value))
                .help(format!("choose [{}] weights {}", crate::print::seq(&seq), vec!["1"; n].join(" "))));
        }
        let span = start.to(self.prev_span());
        Ok(Atom::Choose(Box::new(Choose { span, options: seq.terms, weights, ints, seed })))
    }
}

/// `x*8?0.7` in the canonical form.
pub fn print_chance(c: &ChanceTerm) -> String {
    format!("{}?{}", crate::print::term(&c.term), crate::print::num(c.prob.value))
}

/// `choose [a b c] weights 3 1 1 seed=4` in the canonical form.
pub fn print_choose(c: &Choose) -> String {
    let opts = c.options.iter().map(crate::print::term).collect::<Vec<_>>().join(" ");
    let ws = c.weights.iter().map(|w| crate::print::num(w.value)).collect::<Vec<_>>().join(" ");
    let seed = c.seed.map_or(String::new(), |s| format!(" seed={}", crate::print::num(s.value)));
    format!("choose [{opts}] weights {ws}{seed}")
}

// ---------------------------------------------------------------------------------------------------------------
// reading a pattern: what is kept

/// `x?0.7`: the term's events, each kept when the hash of its place is below the chance.
pub(crate) fn eval_chance(c: &ChanceTerm, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    let evs = pattern::eval_term(&c.term, cycle, ctx);
    if ctx.chance.all {
        return evs;
    }
    let seed = i64::from(ctx.seed);
    evs.into_iter().filter(|e| kept(hash(seed, ctx.chance.bar, cycle, ctx.chance.step_of(e.at), note_of(e)), c.prob.rat)).collect()
}

/// `choose [a b c] weights 3 1 1`: the one option the hash of the slot picks, in the whole slot.
pub(crate) fn eval_choose(c: &Choose, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    if ctx.chance.all {
        // every option that could be picked (a weight of 0 never is), each in the whole slot
        let mut evs = Vec::new();
        for (o, _) in c.options.iter().zip(&c.ints).filter(|(_, w)| **w > 0) {
            evs.extend(pattern::eval_term(o, cycle, ctx));
        }
        return evs;
    }
    let seed = c.seed.map_or(i64::from(ctx.seed), |s| s.value as i64);
    let h = hash(seed, ctx.chance.bar, cycle, ctx.chance.step_of(0.0), CHOOSE_NOTE);
    let i = pick(&c.ints, h);
    pattern::eval_term(&c.options[i], cycle, ctx)
}

/// `maybe(p, 0.3)` / `maybe(p, 0.3, 4)`: the events of `p`, each kept with the chance: `p` with `?0.3` on every hit.
pub(crate) fn eval_maybe(c: &Call, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    let pos: Vec<&Arg> = c.args.iter().filter(|a| !matches!(a, Arg::Named(..))).collect();
    if !pattern::arity(c, ctx, "maybe([x ~ x ~], 0.3) or maybe([x ~ x ~], 0.3, 4): a pattern, the chance each hit is kept, a seed", 2, 3, pos.len()) {
        return Vec::new();
    }
    let Some(v) = pattern::num_arg(ctx, pos[1], "the chance, 0 to 1") else { return Vec::new() };
    let Some(p) = exact_prob(v) else {
        let why = if (0.0..=1.0).contains(&v) { "a chance has at most six decimals (0.333333)" } else { "a chance is 0 to 1" };
        ctx.err(Diagnostic::error(pos[1].span(), why).found(pos[1].span().text(ctx.src).to_string()).help("maybe([x ~ x ~], 0.3, 4)"));
        return Vec::new();
    };
    let seed = match pos.get(2) {
        Some(a) => match pattern::int_arg(ctx, a, "the seed", 0, 4_294_967_295) {
            Some(s) => s,
            None => return Vec::new(),
        },
        None => i64::from(ctx.seed),
    };
    let evs = pattern::eval_seq(&pattern::pattern_arg(pos[0]), cycle, ctx);
    if ctx.chance.all {
        return evs;
    }
    evs.into_iter().filter(|e| kept(hash(seed, ctx.chance.bar, cycle, ctx.chance.step_of(e.at), note_of(e)), p)).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// what check says

/// Does a pattern have any chance in it (a `?`, a `choose` or a `maybe`).
pub fn has_chance(seq: &Seq) -> bool {
    seq.terms.iter().any(term_has_chance)
}

fn term_has_chance(t: &Term) -> bool {
    match &t.atom {
        Atom::Chance(_) | Atom::Choose(_) => true,
        Atom::Group(seqs) => seqs.iter().any(has_chance),
        Atom::Alt(ts) => ts.iter().any(term_has_chance),
        Atom::Call(c) => call_has_chance(c),
        Atom::Sized(s) => term_has_chance(&s.term),
        _ => false,
    }
}

fn call_has_chance(c: &Call) -> bool {
    c.name.text == "maybe"
        || c.args.iter().any(|a| match a {
            Arg::Pat(s) => has_chance(s),
            Arg::Call(c) => call_has_chance(c),
            Arg::Named(_, v) => matches!(&**v, Arg::Pat(s) if has_chance(s)) || matches!(&**v, Arg::Call(c) if call_has_chance(c)),
            _ => false,
        })
}

/// One chance term of a pattern, for the report.
#[derive(Clone, Debug, PartialEq)]
pub struct ChanceReport {
    pub span: Span,
    /// The term as the canonical text says it.
    pub text: String,
    /// `keep`: each hit kept with `prob`; `choose`: one option of `options` a bar, by `weights`.
    pub kind: &'static str,
    pub prob: Option<f64>,
    pub weights: Vec<f64>,
    pub seed: u32,
}

/// Every chance term of a part's pattern, with the seed it uses (its own, else the part's).
pub fn chance_terms(pattern: &Seq, part_seed: u32) -> Vec<ChanceReport> {
    let mut out = Vec::new();
    for t in &pattern.terms {
        collect_term(t, part_seed, &mut out);
    }
    out
}

fn collect_seq(s: &Seq, part_seed: u32, out: &mut Vec<ChanceReport>) {
    for t in &s.terms {
        collect_term(t, part_seed, out);
    }
}

fn collect_term(t: &Term, part_seed: u32, out: &mut Vec<ChanceReport>) {
    match &t.atom {
        Atom::Chance(c) => {
            out.push(ChanceReport { span: t.span, text: crate::print::term(t), kind: "keep", prob: Some(c.prob.value), weights: Vec::new(), seed: part_seed });
            collect_term(&c.term, part_seed, out);
        }
        Atom::Choose(c) => {
            let seed = c.seed.map_or(part_seed, |s| s.value as u32);
            out.push(ChanceReport { span: t.span, text: crate::print::term(t), kind: "choose", prob: None, weights: c.weights.iter().map(|w| w.value).collect(), seed });
            for o in &c.options {
                collect_term(o, part_seed, out);
            }
        }
        Atom::Group(seqs) => seqs.iter().for_each(|s| collect_seq(s, part_seed, out)),
        Atom::Alt(ts) => ts.iter().for_each(|x| collect_term(x, part_seed, out)),
        Atom::Call(c) => collect_call(c, part_seed, out),
        Atom::Sized(s) => collect_term(&s.term, part_seed, out),
        _ => {}
    }
}

fn collect_call(c: &Call, part_seed: u32, out: &mut Vec<ChanceReport>) {
    if c.name.text == "maybe" {
        let pos: Vec<&Arg> = c.args.iter().filter(|a| !matches!(a, Arg::Named(..))).collect();
        let prob = match pos.get(1) {
            Some(Arg::Num(n)) => Some(n.value),
            _ => None,
        };
        let seed = match pos.get(2) {
            Some(Arg::Num(n)) => n.value as u32,
            _ => part_seed,
        };
        out.push(ChanceReport { span: c.span, text: crate::print::call(c), kind: "keep", prob, weights: Vec::new(), seed });
    }
    for a in &c.args {
        match a {
            Arg::Pat(s) => collect_seq(s, part_seed, out),
            Arg::Call(inner) => collect_call(inner, part_seed, out),
            Arg::Named(_, v) => {
                if let Arg::Pat(s) = &**v {
                    collect_seq(s, part_seed, out);
                }
            }
            _ => {}
        }
    }
}

impl Compiler<'_> {
    /// Every chance term of a part is said: its probability or weights, its seed, and how many bars the pattern is
    /// unrolled over (the net's length: the pattern differs every bar until it comes round).
    pub(crate) fn chance_notes(&mut self, part: &Part, part_seed: u32, bars: usize) {
        for r in chance_terms(&part.pattern, part_seed) {
            let what = match r.kind {
                "choose" => format!("picks one of {} options a bar, weights {}", r.weights.len(), r.weights.iter().map(|w| crate::print::num(*w)).collect::<Vec<_>>().join(":")),
                _ => format!("keeps each hit with probability {}", r.prob.map_or("?".to_string(), crate::print::num)),
            };
            let msg = format!("chance: `{}` {what} (seed {}); it differs every bar, unrolled over {bars} bar{}", r.text, r.seed, if bars == 1 { "" } else { "s" });
            self.note(r.span, msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_pinned() {
        // golden values: a change here moves every chance program's notes (and so its nets' CIDs)
        assert_eq!(hash(0, 0, 0, 0, 0), GOLDEN[0]);
        assert_eq!(hash(1, 2, 3, 4, 5), GOLDEN[1]);
        assert_eq!(hash(7, 12, 12, 480, 36), GOLDEN[2]);
        assert_eq!(hash(-1, -1, -1, -1, -1), GOLDEN[3]);
    }
    /// Computed independently (the same finalizer in Python), so a port in another language can check itself against them.
    const GOLDEN: [u32; 4] = [2_418_580_401, 982_550_501, 2_935_515_582, 922_761_234];

    #[test]
    fn a_change_of_any_input_changes_the_hash() {
        let base = hash(4, 9, 9, 120, 36);
        assert_ne!(base, hash(5, 9, 9, 120, 36));
        assert_ne!(base, hash(4, 10, 9, 120, 36));
        assert_ne!(base, hash(4, 9, 10, 120, 36));
        assert_ne!(base, hash(4, 9, 9, 121, 36));
        assert_ne!(base, hash(4, 9, 9, 120, 37));
        // the order of the inputs matters (a bar is not a step)
        assert_ne!(hash(4, 9, 9, 120, 36), hash(4, 9, 9, 36, 120));
        assert_ne!(hash(1, 2, 2, 3, 3), hash(2, 1, 1, 3, 3));
    }

    #[test]
    fn the_threshold_is_the_chance_times_two_to_the_thirty_second_rounded_down() {
        assert_eq!(threshold(Rat::ZERO), 0);
        assert_eq!(threshold(Rat::ONE), 1u64 << 32);
        assert_eq!(threshold(Rat::new(1, 2).unwrap()), 1u64 << 31);
        assert_eq!(threshold(exact_prob(0.3).unwrap()), 1_288_490_188);
        assert_eq!(threshold(exact_prob(0.7).unwrap()), 3_006_477_107);
        assert!(!kept(0, Rat::ZERO));
        assert!(kept(0, exact_prob(0.001).unwrap()));
        assert!(kept(u32::MAX, Rat::ONE));
        assert!(!kept(u32::MAX, exact_prob(0.999999).unwrap()));
        // the comparison is strict: a hash equal to the threshold is not kept
        assert!(!kept(1_288_490_188, exact_prob(0.3).unwrap()));
        assert!(kept(1_288_490_187, exact_prob(0.3).unwrap()));
    }

    #[test]
    fn a_chance_is_exactly_what_was_typed() {
        assert_eq!(exact_prob(0.3), Some(Rat::new(3, 10).unwrap()));
        assert_eq!(exact_prob(0.0), Some(Rat::ZERO));
        assert_eq!(exact_prob(1.0), Some(Rat::ONE));
        assert_eq!(exact_prob(0.333333), Some(Rat::new(333_333, 1_000_000).unwrap()));
        assert_eq!(exact_prob(0.3333333), None, "seven decimals are more than a chance holds");
        assert_eq!(exact_prob(1.5), None);
        assert_eq!(exact_prob(-0.1), None);
        assert_eq!(exact_prob(f64::NAN), None);
    }

    #[test]
    fn the_empirical_rate_over_4096_bars_is_the_chance() {
        for (p, tol) in [(0.1, 0.02), (0.3, 0.025), (0.5, 0.03), (0.7, 0.025), (0.9, 0.02)] {
            let r = exact_prob(p).unwrap();
            for seed in [0i64, 1, 4, 12345] {
                let n = (0..4096).filter(|bar| kept(hash(seed, *bar, *bar, 0, -1), r)).count();
                let rate = n as f64 / 4096.0;
                assert!((rate - p).abs() < tol, "p {p} seed {seed}: {rate}");
            }
        }
    }

    #[test]
    fn a_pick_follows_the_weights() {
        // the boundaries of 3 : 1 : 1 over the hash's range
        let w = [3u64, 1, 1];
        assert_eq!(pick(&w, 0), 0);
        assert_eq!(pick(&w, 2_576_980_377), 0);
        assert_eq!(pick(&w, 2_576_980_378), 1);
        assert_eq!(pick(&w, 3_435_973_836), 1);
        assert_eq!(pick(&w, 3_435_973_837), 2);
        assert_eq!(pick(&w, u32::MAX), 2);
        // a weight of 0 is never picked, whatever the hash
        for h in [0, 1, u32::MAX / 2, u32::MAX] {
            assert_eq!(pick(&[0, 5, 0], h), 1);
            assert_ne!(pick(&[2, 0, 2], h), 1);
        }
        // evenly spread hashes land in proportion
        let mut counts = [0usize; 3];
        for i in 0..10_000u64 {
            counts[pick(&w, ((i * (u64::from(u32::MAX) / 10_000)) & 0xffff_ffff) as u32)] += 1;
        }
        assert_eq!(counts.iter().sum::<usize>(), 10_000);
        assert!((counts[0] as i64 - 6000).abs() <= 2 && (counts[1] as i64 - 2000).abs() <= 2, "{counts:?}");
        // and a hash walked over many bars does too
        let mut counts = [0usize; 3];
        for bar in 0..4096 {
            counts[pick(&w, hash(4, bar, bar, 0, CHOOSE_NOTE))] += 1;
        }
        let share = |i: usize| counts[i] as f64 / 4096.0;
        assert!((share(0) - 0.6).abs() < 0.03 && (share(1) - 0.2).abs() < 0.03 && (share(2) - 0.2).abs() < 0.03, "{counts:?}");
    }

    #[test]
    fn weights_become_whole_numbers_over_one_denominator() {
        let r = |a: i64, b: i64| Rat::new(a, b).unwrap();
        assert_eq!(integer_weights(&[r(3, 1), r(1, 1), r(1, 1)]), Some(vec![3, 1, 1]));
        assert_eq!(integer_weights(&[r(1, 2), r(1, 4), r(1, 4)]), Some(vec![2, 1, 1]));
        assert_eq!(integer_weights(&[r(1, 3), r(1, 2)]), Some(vec![2, 3]));
        assert_eq!(integer_weights(&[r(0, 1), r(1, 10)]), Some(vec![0, 1]));
    }

    #[test]
    fn a_pattern_with_chance_is_unrolled_over_the_section_up_to_a_net() {
        assert_eq!(unrolled_period(1, 16), 16);
        assert_eq!(unrolled_period(1, 8), 8);
        assert_eq!(unrolled_period(1, 1), 1);
        assert_eq!(unrolled_period(1, 100), 64, "a net is at most 64 bars");
        assert_eq!(unrolled_period(3, 16), 18, "whole copies of the alternation's period");
        assert_eq!(unrolled_period(4, 6), 8);
        assert_eq!(unrolled_period(7, 16), 21);
        assert_eq!(unrolled_period(7, 64), 63, "never past a net: the most whole copies that fit");
        assert_eq!(unrolled_period(48, 16), 48, "a longer period is kept");
        assert_eq!(unrolled_period(100, 16), 100, "a chained deck stays as it is");
    }

    #[test]
    fn a_slot_is_an_exact_fraction_of_the_cycle() {
        let mut c = ChanceCtx::default();
        assert_eq!(c.step_of(0.0), 0);
        assert_eq!(c.step_of(0.5), 480);
        let root = c.slot;
        c.enter(root, 1, 4);
        assert_eq!(c.slot, Some((Rat::new(1, 4).unwrap(), Rat::new(1, 4).unwrap())));
        assert_eq!(c.step_of(0.0), 240);
        let inner = c.slot;
        c.enter(inner, 1, 2);
        assert_eq!(c.slot, Some((Rat::new(3, 8).unwrap(), Rat::new(1, 8).unwrap())));
        assert_eq!(c.step_of(0.0), 360);
        assert_eq!(c.step_of(0.5), 420);
        // a nest so deep that the fractions pass i64 loses the slot, never panics
        let mut c = ChanceCtx::default();
        for _ in 0..40 {
            let s = c.slot;
            c.enter(s, 0, 64);
        }
        assert_eq!(c.slot, None);
        assert_eq!(c.step_of(0.25), 240);
    }
}
