//! Pattern sugar: short ways to write patterns the language already has a longhand for (docs/BTW.md "Pattern
//! sugar"). Every form here is **rewritten to its longhand before the pattern is evaluated**, so a program with sugar
//! compiles to the same nets as the same program written out, and `same-as` proves it (crates/btwapi/tests/sugar.rs).
//! The canonical text keeps the sugar as typed: formatting never expands it.
//!
//! | sugar | longhand |
//! |---|---|
//! | `c2~3` (elongate) | `c2 ~ ~`: the step takes three slots, the two after it rests |
//! | `x?` | `x?0.5` (printed so: the canonical text is the longhand) |
//! | `poly(p, q)`, `poly(n, p, q)` | `[p', q']`: each pattern walked `n` steps a bar (the first's step count when `n` is left out), as an alternation of the bars it takes to come round |
//! | `ply(p, n)` | `p` with every step played `n` times in its own slot (`*n` on each step, groups and alternations opened) |
//! | `palindrome(p)` | `<[p] reverse(p)>` |
//! | `iter(p, n)` | `<[p] rotate(p, -s) rotate(p, -2s) ...>`, `s` = the steps of `p` over `n` |
//! | `off(p, n)`, `off(p, n, k)` | `[p, rotate(p, n)]`, `[p, transpose(rotate(p, n), k)]` |
//!
//! There is no replicate mark (`x x x` in one term): `x:3` is the ratchet, an expression mark (three strikes inside one
//! sixteenth step, parse.rs `term_marks`), and the spelling of a replicate is not decided.
//!
//! Nothing here is random: each rewrite is a function of the text, so the determinism of the longhand (seeded chance,
//! seeded generators) is the sugar's too. A part's implicit seed is taken from its longhand (compile.rs), so even a
//! chance or `markov` part with sugar is the same notes as its longhand.
//!
//! The ideas are borrowed from TidalCycles and Strudel, as concepts only (see the "Pattern sugar" section of
//! docs/BTW.md for which page each came from); the names, the semantics as rewrites and all of this code are btw's own.

use crate::ast::*;
use crate::diag::{Diagnostic, Span};

/// The sugar calls: each is rewritten here and never reaches the evaluator.
pub const CALLS: [&str; 5] = ["poly", "ply", "palindrome", "iter", "off"];

/// The most terms the longhand of one part may have: a `poly(..)` inside a `poly(..)` copies the inner one's bars into
/// each of its own, so nested sugar multiplies (`poly(64, [poly(64, [x x x x x x x])])` is 29,184 terms written out).
pub const MAX_TERMS: usize = 16_384;

/// `c2~3`: a term with a count of slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Sized {
    pub term: Term,
    pub mark: Mark,
    pub n: u32,
    /// The `~3`.
    pub at: Span,
}

/// The count marks of a step. (One today: a replicate, `x x x` in one term, would be a second, but `x:3` is the
/// ratchet and its spelling is not decided.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `~n`, elongate: the term in the first of `n` slots, rests in the rest.
    Elongate,
}

impl Mark {
    pub fn text(self) -> &'static str {
        match self {
            Mark::Elongate => "~",
        }
    }
}

pub fn print_sized(s: &Sized) -> String {
    format!("{}{}{}", crate::print::term(&s.term), s.mark.text(), s.n)
}

/// Does the pattern use any sugar (an `~n` or a sugar call)?
pub fn has_sugar(seq: &Seq) -> bool {
    seq.terms.iter().any(term_has_sugar)
}

fn term_has_sugar(t: &Term) -> bool {
    match &t.atom {
        Atom::Sized(_) => true,
        Atom::Group(seqs) | Atom::Deg(seqs) => seqs.iter().any(has_sugar),
        Atom::Alt(ts) => ts.iter().any(term_has_sugar),
        Atom::Chance(c) => term_has_sugar(&c.term),
        Atom::Choose(c) => c.options.iter().any(term_has_sugar),
        Atom::Call(c) => call_has_sugar(c),
        _ => false,
    }
}

fn call_has_sugar(c: &Call) -> bool {
    CALLS.contains(&c.name.text.as_str())
        || c.args.iter().any(|a| match a {
            Arg::Pat(s) => has_sugar(s),
            Arg::Call(c) => call_has_sugar(c),
            Arg::Named(_, v) => {
                matches!(&**v, Arg::Pat(s) if has_sugar(s))
                    || matches!(&**v, Arg::Call(c) if call_has_sugar(c))
            }
            _ => false,
        })
}

/// The part with its pattern written out (a clone when it has no sugar), and what was wrong with the sugar.
pub fn part(p: &Part) -> (Part, Vec<Diagnostic>) {
    if !has_sugar(&p.pattern) {
        return (p.clone(), Vec::new());
    }
    let mut d = Desugar {
        diags: Vec::new(),
        terms: 0,
        over: false,
    };
    let pattern = d.seq(&p.pattern);
    (
        Part {
            pattern,
            ..p.clone()
        },
        d.diags,
    )
}

/// A pattern written out, and what was wrong with its sugar.
pub fn seq(s: &Seq) -> (Seq, Vec<Diagnostic>) {
    let mut d = Desugar {
        diags: Vec::new(),
        terms: 0,
        over: false,
    };
    let out = d.seq(s);
    (out, d.diags)
}

struct Desugar {
    diags: Vec<Diagnostic>,
    /// Terms written out so far ([`MAX_TERMS`]).
    terms: usize,
    over: bool,
}

fn rest(span: Span) -> Term {
    one(span, Atom::Rest)
}

fn one(span: Span, atom: Atom) -> Term {
    Term {
        span,
        atom,
        fast: None,
        accent: false,
        slide: false,
        marks: Marks::default(),
    }
}

fn int_num(v: i64, span: Span) -> Arg {
    Arg::Num(Num::lit(v as f64, true, span))
}

fn call(name: &str, span: Span, args: Vec<Arg>) -> Atom {
    Atom::Call(Call {
        name: Word {
            text: name.to_string(),
            span,
        },
        args,
        span,
    })
}

impl Desugar {
    fn err(&mut self, d: Diagnostic) {
        if !self
            .diags
            .iter()
            .any(|x| x.span == d.span && x.message == d.message)
        {
            self.diags.push(d);
        }
    }

    /// `n` more terms written out; false (said once) when the longhand is more than [`MAX_TERMS`].
    fn charge(&mut self, n: usize, at: Span) -> bool {
        self.terms = self.terms.saturating_add(n);
        if !self.over && self.terms > MAX_TERMS {
            self.over = true;
            self.err(
                Diagnostic::error(at, format!("this pattern written out is more than {MAX_TERMS} steps"))
                    .help("`poly(..)`, `iter(..)` and `off(..)` copy what they are given, and nested they multiply: write fewer or smaller counts"),
            );
        }
        !self.over
    }

    fn seq(&mut self, s: &Seq) -> Seq {
        let mut terms = Vec::new();
        for t in &s.terms {
            terms.extend(self.term(t, Where::Seq));
        }
        Seq {
            span: s.span,
            terms,
        }
    }

    /// One term written out: a `~n` is several (`at` says where it stands: they are slots of a sequence).
    fn term(&mut self, t: &Term, at: Where) -> Vec<Term> {
        if self.over {
            return vec![rest(t.span)];
        }
        if let Atom::Sized(s) = &t.atom {
            let inner = self.single(&s.term);
            return match (s.mark, at) {
                (Mark::Elongate, Where::Seq) => {
                    if !self.charge(s.n as usize, t.span) {
                        return vec![rest(t.span)];
                    }
                    let mut v = vec![inner];
                    v.extend(std::iter::repeat_n(rest(s.at), s.n as usize - 1));
                    v
                }
                (Mark::Elongate, Where::Alt) => {
                    self.err(
                        Diagnostic::error(s.at, "`~n` gives a step more slots of its bar, and a term of `< >` is a whole bar")
                            .help(format!("for {} bars of it, write it {} times in the `< >`", s.n, s.n)),
                    );
                    vec![inner]
                }
                (_, Where::Choose) => {
                    self.err(
                        Diagnostic::error(
                            s.at,
                            format!(
                                "`{}` counts slots, and an option of `choose` is one bar's pick",
                                s.mark.text()
                            ),
                        )
                        .help("give the option more weight instead: `choose [a b] weights 2 1`"),
                    );
                    vec![inner]
                }
            };
        }
        vec![self.single(t)]
    }

    /// A term that is one slot: its insides written out.
    fn single(&mut self, t: &Term) -> Term {
        self.charge(1, t.span);
        let atom = match &t.atom {
            Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| self.seq(s)).collect()),
            Atom::Deg(seqs) => Atom::Deg(seqs.iter().map(|s| self.seq(s)).collect()),
            Atom::Alt(ts) => Atom::Alt(ts.iter().flat_map(|x| self.term(x, Where::Alt)).collect()),
            Atom::Chance(c) => Atom::Chance(Box::new(crate::chance::ChanceTerm {
                term: self.single(&c.term),
                prob: c.prob,
            })),
            Atom::Choose(c) => {
                let options = c
                    .options
                    .iter()
                    .flat_map(|x| self.term(x, Where::Choose))
                    .collect();
                Atom::Choose(Box::new(crate::chance::Choose {
                    options,
                    ..(**c).clone()
                }))
            }
            Atom::Call(c) if CALLS.contains(&c.name.text.as_str()) => match self.sugar_call(c) {
                Some(a) => a,
                None => Atom::Rest,
            },
            Atom::Call(c) => Atom::Call(Call {
                args: c.args.iter().map(|a| self.arg(a)).collect(),
                ..c.clone()
            }),
            // (a sized term where one slot is wanted: its term)
            Atom::Sized(s) => return self.single(&s.term),
            other => other.clone(),
        };
        Term { atom, ..t.clone() }
    }

    /// An argument written out: a sugar call becomes the pattern it is.
    fn arg(&mut self, a: &Arg) -> Arg {
        match a {
            Arg::Pat(s) => Arg::Pat(self.seq(s)),
            Arg::Call(c) if CALLS.contains(&c.name.text.as_str()) => {
                let atom = self.sugar_call(c).unwrap_or(Atom::Rest);
                Arg::Pat(Seq {
                    span: c.span,
                    terms: vec![one(c.span, atom)],
                })
            }
            Arg::Call(c) => Arg::Call(Box::new(Call {
                args: c.args.iter().map(|x| self.arg(x)).collect(),
                ..(**c).clone()
            })),
            Arg::Named(n, v) => Arg::Named(n.clone(), Box::new(self.arg(v))),
            other => other.clone(),
        }
    }

    fn arity(&mut self, c: &Call, usage: &str, lo: usize, hi: usize) -> bool {
        let n = c.args.len();
        if n < lo || n > hi {
            self.err(
                Diagnostic::error(
                    c.span,
                    format!(
                        "{}() takes {}, and this has {n}",
                        c.name.text,
                        if lo == hi {
                            format!("{lo}")
                        } else {
                            format!("{lo} to {hi}")
                        }
                    ),
                )
                .help(usage.to_string()),
            );
            return false;
        }
        if let Some(Arg::Named(n, _)) = c.args.iter().find(|a| matches!(a, Arg::Named(..))) {
            self.err(
                Diagnostic::error(
                    n.span,
                    format!("{}() takes no named arguments", c.name.text),
                )
                .help(usage.to_string()),
            );
            return false;
        }
        true
    }

    fn int(&mut self, a: &Arg, what: &str, lo: i64, hi: i64, usage: &str) -> Option<i64> {
        match a {
            Arg::Num(n) if n.value.fract() == 0.0 && (lo..=hi).contains(&(n.value as i64)) => {
                Some(n.value as i64)
            }
            Arg::Num(n) => {
                self.err(
                    Diagnostic::error(
                        n.span,
                        format!("{what} is a whole number from {lo} to {hi}"),
                    )
                    .found(crate::print::num(n.value))
                    .help(usage.to_string()),
                );
                None
            }
            other => {
                self.err(
                    Diagnostic::error(
                        other.span(),
                        format!("expected {what}, a whole number from {lo} to {hi}"),
                    )
                    .help(usage.to_string()),
                );
                None
            }
        }
    }

    /// A pattern argument, written out.
    fn pat(&mut self, a: &Arg, c: &Call, usage: &str) -> Option<(Arg, Seq)> {
        if let Arg::Num(_) = a {
            // (a degree is a pattern of one step, but a number where a pattern goes is more likely an argument in the wrong place)
            if !matches!(c.name.text.as_str(), "ply" | "palindrome") {
                self.err(
                    Diagnostic::error(
                        a.span(),
                        format!("{}() takes its pattern first", c.name.text),
                    )
                    .help(usage.to_string()),
                );
                return None;
            }
        }
        let a = self.arg(a);
        // (`[a b c]` is its three steps: the brackets of the argument are not a group of their own)
        let s = steps_of(crate::pattern::pattern_arg(&a));
        Some((a, s))
    }

    fn sugar_call(&mut self, c: &Call) -> Option<Atom> {
        let sp = c.span;
        match c.name.text.as_str() {
            "palindrome" => {
                let usage = "palindrome(riff): riff, then riff backwards, a bar each";
                if !self.arity(c, usage, 1, 1) {
                    return None;
                }
                let (a, s) = self.pat(&c.args[0], c, usage)?;
                // (the copies are charged for what each holds: `p` is written out once already)
                if !self.charge(terms_count(&s.terms), sp) {
                    return None;
                }
                Some(Atom::Alt(vec![
                    one(sp, Atom::Group(vec![s])),
                    one(sp, call("reverse", sp, vec![a])),
                ]))
            }
            "iter" => {
                let usage =
                    "iter(riff, 4): riff starting a quarter later each bar, round in 4 bars";
                if !self.arity(c, usage, 2, 2) {
                    return None;
                }
                let (a, s) = self.pat(&c.args[0], c, usage)?;
                let n = self.int(&c.args[1], "the parts to turn it in", 1, 16, usage)?;
                let steps = crate::pattern::slots(&s) as i64;
                if steps % n != 0 || steps > 64 {
                    let why = if steps > 64 {
                        format!(
                            "iter() turns a pattern of at most 64 steps, and this one has {steps}"
                        )
                    } else {
                        format!("iter(p, {n}) turns p by 1/{n} of its steps a bar, and its {steps} steps do not divide by {n}")
                    };
                    self.err(
                        Diagnostic::error(c.args[1].span(), why)
                            .help(format!("a count that divides {steps}: {}", divisors(steps))),
                    );
                    return None;
                }
                let by = steps / n;
                let size = terms_count(&s.terms).max(1);
                let mut alts = vec![one(sp, Atom::Group(vec![s]))];
                for k in 1..n {
                    alts.push(one(
                        sp,
                        call(
                            "rotate",
                            sp,
                            vec![a.clone(), int_num(-k * by, c.args[1].span())],
                        ),
                    ));
                }
                if !self.charge(alts.len().saturating_mul(size), sp) {
                    return None;
                }
                Some(Atom::Alt(alts))
            }
            "off" => {
                let usage = "off(riff, 2) or off(riff, 2, 12): riff with a copy of it 2 steps later (12 semitones up)";
                if !self.arity(c, usage, 2, 3) {
                    return None;
                }
                let (a, s) = self.pat(&c.args[0], c, usage)?;
                let by = self.int(&c.args[1], "the steps to move the copy", -64, 64, usage)?;
                if !self.charge(terms_count(&s.terms), sp) {
                    return None;
                }
                let mut copy = call("rotate", sp, vec![a, int_num(by, c.args[1].span())]);
                if let Some(k) = c.args.get(2) {
                    let k = self.int(k, "the semitones", -48, 48, usage)?;
                    let Atom::Call(inner) = copy else {
                        unreachable!()
                    };
                    copy = call(
                        "transpose",
                        sp,
                        vec![Arg::Call(Box::new(inner)), int_num(k, c.args[2].span())],
                    );
                }
                Some(Atom::Group(vec![
                    s,
                    Seq {
                        span: sp,
                        terms: vec![one(sp, copy)],
                    },
                ]))
            }
            "ply" => {
                let usage = "ply(riff, 2): every step of riff played 2 times in its own slot";
                if !self.arity(c, usage, 2, 2) {
                    return None;
                }
                let n = self.int(&c.args[1], "the times to play each step", 1, 16, usage)? as u32;
                if let Arg::Call(inner) = &c.args[0] {
                    if !CALLS.contains(&inner.name.text.as_str()) {
                        return self.not_plied(&inner.name, inner.span);
                    }
                }
                let (_, s) = self.pat(&c.args[0], c, usage)?;
                let terms = s
                    .terms
                    .iter()
                    .map(|t| self.ply(t, n))
                    .collect::<Option<Vec<_>>>()?;
                Some(Atom::Group(vec![Seq {
                    span: s.span,
                    terms,
                }]))
            }
            "poly" => {
                let usage = "poly([a b], [c d e]) or poly(4, [a b c]): patterns walked the same number of steps a bar";
                if !self.arity(c, usage, 1, 9) {
                    return None;
                }
                let (k, pats) = match &c.args[0] {
                    Arg::Num(_) => (
                        Some(self.int(&c.args[0], "the steps a bar", 1, 64, usage)?),
                        &c.args[1..],
                    ),
                    _ => (None, &c.args[..]),
                };
                if pats.is_empty() {
                    self.err(
                        Diagnostic::error(c.span, "poly() needs a pattern to walk")
                            .help(usage.to_string()),
                    );
                    return None;
                }
                let mut seqs = Vec::new();
                for a in pats {
                    seqs.push(self.pat(a, c, usage)?.1);
                }
                let k = k.unwrap_or(seqs[0].terms.len() as i64) as usize;
                let mut stack = Vec::new();
                for s in &seqs {
                    let m = s.terms.len();
                    let bars = m / gcd(m, k);
                    if bars > 64 {
                        self.err(Diagnostic::error(s.span, format!("this pattern of {m} steps walked {k} a bar takes {bars} bars to come round; the most is 64")).help("steps that share a factor with the count come round sooner"));
                        return None;
                    }
                    // each step of the pattern is copied `bars * k / m` times, at what it holds
                    let copies = if m == 0 { 0 } else { bars * k / m };
                    if !self.charge(copies.saturating_mul(terms_count(&s.terms)).max(bars * k), s.span) {
                        return None;
                    }
                    let bar = |b: usize| {
                        one(
                            s.span,
                            Atom::Group(vec![Seq {
                                span: s.span,
                                terms: (0..k).map(|j| s.terms[(b * k + j) % m].clone()).collect(),
                            }]),
                        )
                    };
                    let term = if bars == 1 {
                        bar(0)
                    } else {
                        one(s.span, Atom::Alt((0..bars).map(bar).collect()))
                    };
                    stack.push(Seq {
                        span: s.span,
                        terms: vec![term],
                    });
                }
                Some(Atom::Group(stack))
            }
            _ => None,
        }
    }

    fn not_plied(&mut self, name: &Word, span: Span) -> Option<Atom> {
        self.err(
            Diagnostic::error(span, format!("ply() plays written steps again, and `{}(..)` is a call", name.text))
                .help("ply the pattern inside it, `transpose(ply(riff, 2), 5)`, or write the steps out: `ply([x ~ ~ x ~ ~ x ~], 2)`"),
        );
        None
    }

    /// `t` with each of its steps played `n` times in its own slot.
    fn ply(&mut self, t: &Term, n: u32) -> Option<Term> {
        let atom = match &t.atom {
            Atom::Group(seqs) | Atom::Deg(seqs) => {
                let mut out = Vec::new();
                for s in seqs {
                    out.push(Seq {
                        span: s.span,
                        terms: s
                            .terms
                            .iter()
                            .map(|x| self.ply(x, n))
                            .collect::<Option<Vec<_>>>()?,
                    });
                }
                if matches!(t.atom, Atom::Deg(_)) {
                    Atom::Deg(out)
                } else {
                    Atom::Group(out)
                }
            }
            Atom::Alt(ts) => Atom::Alt(
                ts.iter()
                    .map(|x| self.ply(x, n))
                    .collect::<Option<Vec<_>>>()?,
            ),
            Atom::Chance(c) => Atom::Chance(Box::new(crate::chance::ChanceTerm {
                term: self.ply(&c.term, n)?,
                prob: c.prob,
            })),
            Atom::Choose(c) => Atom::Choose(Box::new(crate::chance::Choose {
                options: c
                    .options
                    .iter()
                    .map(|x| self.ply(x, n))
                    .collect::<Option<Vec<_>>>()?,
                ..(**c).clone()
            })),
            Atom::Call(c) => {
                self.not_plied(&c.name, c.span);
                return None;
            }
            _ => {
                let times = t.fast.unwrap_or(1).saturating_mul(n);
                if times > 64 {
                    self.err(
                        Diagnostic::error(
                            t.span,
                            format!(
                                "ply() makes this step `*{times}`, and a repeat count is 1 to 64"
                            ),
                        )
                        .help("ply by less, or write fewer repeats inside"),
                    );
                    return None;
                }
                return Some(Term {
                    fast: Some(times),
                    ..t.clone()
                });
            }
        };
        Some(Term { atom, ..t.clone() })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Where {
    /// A slot of a sequence (a bar, a group).
    Seq,
    /// A term of `< >`: a bar.
    Alt,
    /// An option of `choose`.
    Choose,
}

/// The steps of a pattern argument: `[a b c]` is the three steps inside its brackets (as `pattern::slots` counts them).
fn steps_of(s: Seq) -> Seq {
    if let [Term {
        atom: Atom::Group(seqs),
        fast: None,
        accent: false,
        slide: false,
        marks,
        ..
    }] = s.terms.as_slice()
    {
        // (`[a b c].` is a marked group, one step: its marks are on every hit in it)
        if let ([inner], true) = (seqs.as_slice(), marks.is_plain()) {
            return steps_of(inner.clone());
        }
    }
    s
}

/// How many terms a term is, written out: what a copy of it costs ([`MAX_TERMS`]).
fn count(t: &Term) -> usize {
    1usize.saturating_add(match &t.atom {
        Atom::Group(ss) | Atom::Deg(ss) => ss.iter().map(|s| terms_count(&s.terms)).fold(0, usize::saturating_add),
        Atom::Alt(ts) => terms_count(ts),
        Atom::Chance(c) => count(&c.term),
        Atom::Choose(c) => terms_count(&c.options),
        Atom::Sized(z) => count(&z.term).saturating_add(z.n as usize),
        Atom::Call(c) => call_count(c),
        _ => 0,
    })
}

fn terms_count(ts: &[Term]) -> usize {
    ts.iter().map(count).fold(0, usize::saturating_add)
}

fn call_count(c: &Call) -> usize {
    c.args.iter().map(arg_count).fold(0, usize::saturating_add)
}

fn arg_count(a: &Arg) -> usize {
    match a {
        Arg::Pat(s) => terms_count(&s.terms),
        Arg::Call(c) => 1usize.saturating_add(call_count(c)),
        Arg::Named(_, v) => arg_count(v),
        _ => 1,
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn divisors(n: i64) -> String {
    (1..=n.min(16))
        .filter(|d| n % d == 0)
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
