//! F12 assertions (docs/BTW-EXTENSIONS.md, docs/btw/ext-assert.md): statements a program makes about itself.
//!
//! ```text
//! assert ends                                 the program ends (an `end`, a timeline with no `loop`, a bounded evolve)
//! assert period == 3                          the program's period in bars, as `meter` reports it
//! assert period(drop) == 32                   the period of the evolve in section `drop` (`evolve`: the program's own)
//! assert length == 3m30s ± 5s                 how long the program is, through the tempo
//! assert key == dm in climax                  the key in a section (without `in`: the program's)
//! assert level A < 2 * B in drop              measured on an offline render: `btw test`
//! ```
//!
//! The first five are **static**: `check` proves them and a false one is an error at the statement that says the actual
//! value (`assert length == 3m30s ± 5s: the program is 2m58s`), so `eval` refuses the program. A `level` assertion is
//! **render-based**: `check` records it as pending (a note, not a failure) and `btw test` (the verb, `djctl btw test FILE`,
//! `djhost::btw_test`) renders the program offline, measures each deck's RMS in the section and judges it here
//! ([`LevelCheck::judge`]), so the measurement is the host's and the reading of it is the language's.
//!
//! Pure like the rest of the crate: no clock, no I/O, no render.

use crate::ast::{CmpOp, Program, Stmt, StmtKind, Word};
use crate::compile::Compiled;
use crate::constraint::{self, Judged, Relation, Subject};
use crate::diag::{Diagnostic, Span};
use crate::expr::{self, BinOp, Cursor, Expr};
use crate::lex::Tok;
use crate::music::Key;
use crate::parse::{nearest, Parser};
use crate::rat::{Rat, RatError};

type R<T> = Result<T, Diagnostic>;

/// The words that follow `assert` (a constraint on a part may also begin with its name: constraint.rs).
pub const MEASURES: [&str; 6] = ["ends", "period", "length", "level", "key", "never"];
/// What a level expression may name.
pub const LEVEL_WORDS: [&str; 3] = ["A", "B", "master"];
/// Words the highlighter colours as keywords (`in` is already one).
pub const HIGHLIGHT_KEYWORDS: [&str; 7] = ["assert", "ends", "period", "length", "never", "beside", "during"];
/// How close two lengths in seconds must be to be the same: far below a millisecond, above float noise.
const EPS: f64 = 1e-9;
const USAGE: &str = "assert ends   assert length == 3m30s ± 5s   assert level A < 2 * B in drop   assert never A.snare beside A.snare";

// ---------------------------------------------------------------------------------------------------------------------
// the tree
// ---------------------------------------------------------------------------------------------------------------------

/// `assert ...` as a statement of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct AssertStmt {
    pub kind: AssertKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssertKind {
    /// `assert ends`.
    Ends,
    /// `assert period == 3`, `assert period(drop) >= 8`: the program's period, or the evolve in a section's, in bars.
    Period { of: Option<Word>, op: CmpOp, value: u32, value_span: Span },
    /// `assert length == 3m30s ± 5s`, `assert length < 4m`.
    Length { op: CmpOp, target: Clock, tolerance: Option<Clock> },
    /// `assert level A < 2 * B in drop`: arithmetic over the decks' levels (`A`, `B`, `master`), measured.
    Level { lhs: Expr, op: CmpOp, rhs: Expr, section: Option<Word> },
    /// `assert key == dm in climax`; `!=` too.
    Key { op: CmpOp, key: Word, section: Option<Word> },
    /// `assert never A.snare beside A.snare`, `assert never B during sing in verse`: notes that must not meet (constraint.rs).
    Never { x: Subject, rel: Relation, y: Subject, section: Option<Word> },
    /// `assert bass ends on 1 in lift`: the last note of each place the section plays is that degree of its key (constraint.rs).
    EndsOn { part: Subject, degree: u32, degree_span: Span, section: Option<Word> },
}

/// A time as a person writes it, `3m30s`, `90s`, `4m`, `500ms`: exact seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub secs: Rat,
    pub span: Span,
}

impl AssertStmt {
    /// The canonical text of the statement.
    pub fn text(&self) -> String {
        match &self.kind {
            AssertKind::Ends => "assert ends".into(),
            AssertKind::Period { of, op, value, .. } => match of {
                Some(w) => format!("assert period({}) {} {value}", w.text, op.text()),
                None => format!("assert period {} {value}", op.text()),
            },
            AssertKind::Length { op, target, tolerance } => {
                let mut s = format!("assert length {} {}", op.text(), clock_text(target.secs));
                if let Some(t) = tolerance {
                    s.push_str(&format!(" \u{b1} {}", clock_text(t.secs)));
                }
                s
            }
            AssertKind::Level { lhs, op, rhs, section } => {
                let mut s = format!("assert level {} {} {}", level_canonical(lhs, 0), op.text(), level_canonical(rhs, 0));
                if let Some(w) = section {
                    s.push_str(&format!(" in {}", w.text));
                }
                s
            }
            AssertKind::Key { op, key, section } => {
                let mut s = format!("assert key {} {}", op.text(), crate::music::canonical_pitch_word(&key.text));
                if let Some(w) = section {
                    s.push_str(&format!(" in {}", w.text));
                }
                s
            }
            AssertKind::Never { x, rel, y, section } => constraint::never_text(x, *rel, y, section),
            AssertKind::EndsOn { part, degree, section, .. } => constraint::ends_on_text(part, *degree, section),
        }
    }
}

/// The canonical text of a level expression: [`Expr::canonical`]'s, except that a number that is a decimal prints as one
/// (`0.05`, not `1 / 20`: a threshold is a decimal, and `0.05` reads back as the same rational).
fn level_canonical(e: &Expr, outer: u8) -> String {
    // precedence: sum 5, term 6, unary 7, primary 8
    let (prec, text) = match e {
        Expr::Num(v, _) if v.is_int() => (8, v.to_string()),
        // a terminating decimal of at most six places (what can be typed as one) prints as one
        Expr::Num(v, _) if 1_000_000 % v.denom() == 0 => (8, crate::print::num(v.to_f64())),
        Expr::Num(v, _) => (6, format!("{} / {}", v.numer(), v.denom())),
        Expr::Neg(x, _) => (7, format!("-{}", level_canonical(x, 7))),
        Expr::Bin(op, a, b, _) => {
            let (p, sym) = match op {
                BinOp::Add => (5, "+"),
                BinOp::Sub => (5, "-"),
                BinOp::Mul => (6, "*"),
                BinOp::Div => (6, "/"),
                BinOp::Rem => (6, "%"),
            };
            (p, format!("{} {sym} {}", level_canonical(a, p), level_canonical(b, p + 1)))
        }
        Expr::Call(n, args, _) => (8, format!("{n}({})", args.iter().map(|a| level_canonical(a, 0)).collect::<Vec<_>>().join(", "))),
        // a word (a deck), and what a level expression cannot hold
        other => (8, other.canonical()),
    };
    if prec < outer {
        format!("({text})")
    } else {
        text
    }
}

/// Seconds as the canonical text of a clock: minutes and seconds, `3m30s` (`90s` is `1m30s`, `180s` is `3m`, `0.5s` stays).
pub fn clock_text(secs: Rat) -> String {
    let mins = secs.div(Rat::int(60)).map(Rat::floor).unwrap_or(0).max(0);
    let rem = Rat::int(mins).mul(Rat::int(60)).and_then(|m| secs.sub(m)).unwrap_or(secs);
    let rem_text = crate::print::num(rem.to_f64());
    match (mins, rem.is_zero()) {
        (0, _) => format!("{rem_text}s"),
        (m, true) => format!("{m}m"),
        (m, false) => format!("{m}m{rem_text}s"),
    }
}

/// An actual length for a message: rounded to the millisecond, `2m58s`, `3m10.476s`.
pub fn seconds_text(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as i64;
    let (m, rest) = (ms / 60_000, ms % 60_000);
    let (whole, frac) = (rest / 1000, rest % 1000);
    let s = if frac == 0 { format!("{whole}") } else { format!("{whole}.{}", format!("{frac:03}").trim_end_matches('0')) };
    match (m, rest) {
        (0, _) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, _) => format!("{m}m{s}s"),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// reading
// ---------------------------------------------------------------------------------------------------------------------

impl Parser<'_> {
    /// `assert ...` (the `assert` word is the next token).
    pub(crate) fn assert_statement(&mut self, start: Span) -> R<Stmt> {
        self.bump();
        let kind = match self.peek().clone() {
            Tok::Word(w) => match w.as_str() {
                "ends" => {
                    self.bump();
                    AssertKind::Ends
                }
                "period" => self.assert_period()?,
                "length" => self.assert_length()?,
                "level" => self.assert_level()?,
                "key" => self.assert_key()?,
                "never" => self.assert_never()?,
                _ if constraint::starts_subject(&w) => self.assert_ends_on()?,
                _ => {
                    let help = match nearest(&w, MEASURES.into_iter().chain(constraint::first_words())) {
                        Some(n) => format!("did you mean `{n}`?"),
                        None => USAGE.to_string(),
                    };
                    return Err(self.expected(&MEASURES, &format!("`assert` is followed by what to assert, found `{w}`")).help(help));
                }
            },
            _ => return Err(self.expected(&MEASURES, &format!("`assert` is followed by what to assert, found {}", self.found())).help(USAGE)),
        };
        self.end_of_statement()?;
        Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Assert(AssertStmt { kind }) })
    }

    fn assert_period(&mut self) -> R<AssertKind> {
        self.bump();
        let of = if self.peek() == &Tok::LParen {
            self.bump();
            let w = self.word("a section with an evolve, or `evolve`")?;
            self.expect(Tok::RParen, "`)`")?;
            Some(w)
        } else {
            None
        };
        let (op, _) = self.cmp_op().map_err(|d| d.help("assert period == 3   assert period(drop) == 32"))?;
        let (value, value_span) = self.int("a period in bars")?;
        Ok(AssertKind::Period { of, op, value, value_span })
    }

    fn assert_length(&mut self) -> R<AssertKind> {
        self.bump();
        let (op, _) = self.cmp_op().map_err(|d| d.help("assert length == 3m30s ± 5s   assert length < 4m"))?;
        let target = self.clock("a length like `3m30s`, `90s` or `4m`")?;
        let tolerance = if self.peek() == &Tok::PlusMinus {
            let at = self.bump().span;
            if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                return Err(Diagnostic::error(at, format!("a tolerance `±` goes with `==` or `!=`, not `{}`", op.text()))
                    .help("`length < 4m` is exact: write the bound you mean (`length < 4m5s`), or `length == 4m ± 5s`"));
            }
            Some(self.clock("a tolerance like `5s`")?)
        } else {
            None
        };
        Ok(AssertKind::Length { op, target, tolerance })
    }

    /// `3m30s`, `90s`, `4m`, `500ms`: a number with its unit glued to it, minutes optionally followed by seconds.
    fn clock(&mut self, what: &str) -> R<Clock> {
        let first = self.span();
        if self.peek() == &Tok::Minus {
            return Err(Diagnostic::error(first, "a length is not negative").help("a clock is `3m30s`, `90s` or `4m`"));
        }
        let (a, unit) = self.clock_part(what)?;
        let mut secs = match unit.as_str() {
            "m" => a.mul(Rat::int(60)),
            "s" => Ok(a),
            "ms" => a.div(Rat::int(1000)),
            other => {
                return Err(Diagnostic::error(self.prev_span(), format!("`{other}` is not a unit of time: a clock is minutes and seconds"))
                    .expected(&["`m`", "`s`", "`ms`"])
                    .found(format!("the unit `{other}`"))
                    .help("3m30s   90s   4m   500ms"))
            }
        };
        // `3m30s`: minutes run on into seconds
        if unit == "m" && matches!(self.peek(), Tok::Num { .. }) && matches!(self.peek_n(1), Tok::Unit(u) if u == "s") {
            let (b, _) = self.clock_part(what)?;
            secs = secs.and_then(|m| m.add(b));
        }
        let span = first.to(self.prev_span());
        match secs {
            Ok(secs) => Ok(Clock { secs, span }),
            Err(e) => Err(Diagnostic::error(span, format!("this length is not a number the language can hold: {e}"))),
        }
    }

    fn clock_part(&mut self, what: &str) -> R<(Rat, String)> {
        let (value, span) = match self.peek().clone() {
            Tok::Num { value, .. } => (value, self.bump().span),
            _ => return Err(self.expected(&[what], &format!("expected {what}, found {}", self.found())).help("a clock is a number with its unit glued to it: `3m30s`, `90s`, `4m`")),
        };
        let Tok::Unit(unit) = self.peek().clone() else {
            return Err(Diagnostic::error(span, format!("a length needs a unit glued to its number, found `{}`", span.text(self.src)))
                .help("a clock is `3m30s`, `90s` or `4m`: the `m` and the `s` touch their numbers"));
        };
        self.bump();
        let Some(r) = Rat::from_f64(value) else {
            return Err(Diagnostic::error(span, "this number is too large or too precise").help("numbers have at most six decimals"));
        };
        Ok((r, unit))
    }

    fn assert_level(&mut self) -> R<AssertKind> {
        self.bump();
        let lhs = self.level_side()?;
        let (op, _) = self.cmp_op().map_err(|d| d.help("compare two levels: `level A < 2 * B in drop`"))?;
        let rhs = self.level_side()?;
        let mut leaves = Vec::new();
        collect_leaves(&lhs, &mut leaves);
        collect_leaves(&rhs, &mut leaves);
        if leaves.is_empty() {
            return Err(Diagnostic::error(lhs.span().to(rhs.span()), "this reads no level: it names no deck").help("name `A`, `B` or `master`: `level A < 2 * B`"));
        }
        let section = self.in_section()?;
        Ok(AssertKind::Level { lhs, op, rhs, section })
    }

    /// One side of a level comparison: arithmetic over numbers and the words `A`, `B`, `master`, constants folded.
    fn level_side(&mut self) -> R<Expr> {
        let mut c = Cursor { toks: &self.toks, at: self.at };
        let e = expr::parse_arith(&mut c).map_err(|d| if d.help.is_none() { d.help("a level is `A`, `B` or `master`, or arithmetic over them: `2 * B`, `A + B`, `min(A, B)`") } else { d })?;
        self.at = c.at;
        fold_level(&e)
    }

    pub(crate) fn in_section(&mut self) -> R<Option<Word>> {
        if !self.word_is("in") {
            return Ok(None);
        }
        self.bump();
        Ok(Some(self.word("the name of a section")?))
    }

    fn assert_key(&mut self) -> R<AssertKind> {
        self.bump();
        let (op, sp) = self.cmp_op().map_err(|d| d.help("assert key == dm in climax"))?;
        if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
            return Err(Diagnostic::error(sp, format!("keys are compared with `==` and `!=`, not `{}`", op.text())).help("assert key == dm in climax"));
        }
        let key = self.key_word()?;
        if Key::parse(&key.text).is_none() {
            return Err(Diagnostic::error(key.span, format!("`{}` is not a key", key.text))
                .expected(&["a key like am, c, f#m or d:dorian"])
                .found(format!("`{}`", key.text))
                .help(format!("modes: {}", crate::music::MODES.join(", "))));
        }
        let section = self.in_section()?;
        Ok(AssertKind::Key { op, key, section })
    }
}

/// The words of a level expression, in order of first appearance.
fn collect_leaves(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Word(w, _) => {
            if !out.contains(w) {
                out.push(w.clone());
            }
        }
        Expr::Neg(x, _) => collect_leaves(x, out),
        Expr::Bin(_, a, b, _) => {
            collect_leaves(a, out);
            collect_leaves(b, out);
        }
        Expr::Call(_, args, _) => args.iter().for_each(|a| collect_leaves(a, out)),
        _ => {}
    }
}

/// The word a mistyped level word was most likely meant to be: `a` is `A`, `mastr` is `master`.
fn level_word_near(w: &str) -> Option<&'static str> {
    let lower = w.to_lowercase();
    if let Some(exact) = LEVEL_WORDS.iter().find(|x| x.to_lowercase() == lower) {
        return Some(exact);
    }
    // a one-letter word is too short for an edit distance to mean anything
    if lower.len() < 3 {
        return None;
    }
    let lowers = ["master"];
    nearest(&lower, lowers).map(|_| "master")
}

/// Check a level expression (only numbers, `A` / `B` / `master`, `+ - * / %`, `min` / `max`) and fold what has no level in
/// it to one number, so `(1 + 1) * B` and `2 * B` are one program.
fn fold_level(e: &Expr) -> R<Expr> {
    let rat_err = |er: RatError, sp: Span| Diagnostic::error(sp, er.to_string());
    Ok(match e {
        Expr::Num(..) => e.clone(),
        Expr::Word(w, sp) => {
            if !LEVEL_WORDS.contains(&w.as_str()) {
                let help = match level_word_near(w) {
                    Some(n) => format!("did you mean `{n}`?"),
                    None => "a level is read from deck `A`, deck `B` or `master`".to_string(),
                };
                return Err(Diagnostic::error(*sp, format!("`{w}` is not something with a level")).expected(&LEVEL_WORDS).found(format!("`{w}`")).help(help));
            }
            e.clone()
        }
        Expr::Unbound(n, sp) => return Err(Diagnostic::error(*sp, format!("`${n}` has no value here")).help("an `assert` belongs at the top of the program, where there is no `$name`")),
        Expr::Neg(x, sp) => match fold_level(x)? {
            Expr::Num(v, _) => Expr::Num(v.neg().map_err(|er| rat_err(er, *sp))?, *sp),
            other => Expr::Neg(Box::new(other), *sp),
        },
        Expr::Bin(op, a, b, sp) => {
            let (x, y) = (fold_level(a)?, fold_level(b)?);
            match (&x, &y) {
                (Expr::Num(p, _), Expr::Num(q, _)) => {
                    let r = match op {
                        BinOp::Add => p.add(*q),
                        BinOp::Sub => p.sub(*q),
                        BinOp::Mul => p.mul(*q),
                        BinOp::Div => p.div(*q),
                        BinOp::Rem => p.rem(*q),
                    };
                    let r = r.map_err(|er| match er {
                        RatError::DivideByZero => Diagnostic::error(b.span(), "division by zero"),
                        other => rat_err(other, *sp),
                    })?;
                    Expr::Num(r, *sp)
                }
                // a level over a literal zero is a mistake, not a measurement
                (_, Expr::Num(q, _)) if q.is_zero() && matches!(op, BinOp::Div | BinOp::Rem) => return Err(Diagnostic::error(b.span(), if *op == BinOp::Rem { "modulo by zero" } else { "division by zero" })),
                _ => Expr::Bin(*op, Box::new(x), Box::new(y), *sp),
            }
        }
        Expr::Call(name, args, sp) if name == "min" || name == "max" => {
            if args.is_empty() {
                return Err(Diagnostic::error(*sp, format!("`{name}` needs at least one value")).help(format!("{name}(A, B)")));
            }
            let folded = args.iter().map(fold_level).collect::<R<Vec<_>>>()?;
            if folded.iter().all(|a| matches!(a, Expr::Num(..))) {
                let mut vals = folded.iter().filter_map(|a| if let Expr::Num(v, _) = a { Some(*v) } else { None });
                let first = vals.next().expect("non-empty");
                Expr::Num(vals.fold(first, |b, n| if name == "min" { b.min(n) } else { b.max(n) }), *sp)
            } else {
                Expr::Call(name.clone(), folded, *sp)
            }
        }
        // (F13 meets F12) an assertion is judged on an offline render, which has no controllers
        Expr::Call(name, _, sp) if crate::perform::is_input_word(name) => {
            return Err(Diagnostic::error(*sp, format!("`{name}(...)` is a performed input: an assertion is judged on an offline render, which has no controllers, so it cannot read one"))
                .help("write the level you expect as a number (`assert level A < 0.5`); `btw test` measures a performed program with every input at 1/2"))
        }
        Expr::Call(name, _, sp) => return Err(Diagnostic::error(*sp, format!("`{name}(...)` is not a function of levels")).help("`min(A, B)` and `max(A, B)` are")),
        Expr::Cmp(_, _, _, sp) | Expr::Not(_, sp) | Expr::And(_, _, sp) | Expr::Or(_, _, sp) => {
            return Err(Diagnostic::error(*sp, "a level is a number: a comparison goes between two levels, not inside one").help("level A < 2 * B"))
        }
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// checking
// ---------------------------------------------------------------------------------------------------------------------

/// Where an assertion stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    /// A render-based assertion: `btw test` measures it.
    Pending,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Pending => "pending",
        }
    }
}

/// One assertion of a compiled program: what `check` found out about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    /// The statement.
    pub span: Span,
    /// Its canonical text: `assert length == 3m30s ± 5s`.
    pub text: String,
    pub status: Status,
    /// What is: `the program is 2m58s`; for a pending one, where it is measured.
    pub message: String,
    /// What to write instead, for a failure.
    pub help: Option<String>,
    /// A `level` assertion, to be judged on a render.
    pub level: Option<LevelCheck>,
}

impl Checked {
    /// `assert length == 3m30s ± 5s: the program is 2m58s`.
    pub fn line(&self) -> String {
        format!("{}: {}", self.text, self.message)
    }
}

/// A `level` assertion resolved against the timeline: its two sides, the comparison and the bars it is measured over.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelCheck {
    pub lhs: Expr,
    pub op: CmpOp,
    pub rhs: Expr,
    /// The section it is measured in (None: the whole program).
    pub section: Option<String>,
    /// The stretches of the song it is measured over, `[start, end)` in bars: every place the section is on the timeline;
    /// empty for the whole program.
    pub windows: Vec<(u32, u32)>,
}

/// What `btw test` made of a level assertion once each deck was measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub pass: bool,
    pub lhs: f64,
    pub rhs: f64,
    /// Each level the expression reads, as measured (`A`, 0.083).
    pub measured: Vec<(String, f64)>,
    /// `A 0.083, B 0.051: 0.083 < 0.102`.
    pub text: String,
}

impl LevelCheck {
    /// The words the assertion reads (`A`, `B`, `master`), in order of first appearance.
    pub fn leaves(&self) -> Vec<String> {
        let mut v = Vec::new();
        collect_leaves(&self.lhs, &mut v);
        collect_leaves(&self.rhs, &mut v);
        v
    }

    /// Judge the assertion, `measure` giving each word's level (a linear RMS, 0 for silence).
    pub fn judge(&self, measure: &dyn Fn(&str) -> f64) -> Verdict {
        let measured: Vec<(String, f64)> = self.leaves().into_iter().map(|w| (w.clone(), measure(&w))).collect();
        let leaf = |w: &str| measured.iter().find(|(n, _)| n == w).map_or(0.0, |(_, v)| *v);
        let (lhs, rhs) = (eval_level(&self.lhs, &leaf), eval_level(&self.rhs, &leaf));
        let pass = match self.op {
            CmpOp::Eq => lhs == rhs,
            CmpOp::Ne => lhs != rhs,
            CmpOp::Lt => lhs < rhs,
            CmpOp::Le => lhs <= rhs,
            CmpOp::Gt => lhs > rhs,
            CmpOp::Ge => lhs >= rhs,
        };
        let reads = measured.iter().map(|(w, v)| format!("{w} {}", level_text(*v))).collect::<Vec<_>>().join(", ");
        let mut text = format!("{reads}: {} {} {}", level_text(lhs), self.op.text(), level_text(rhs));
        if !lhs.is_finite() || !rhs.is_finite() {
            text.push_str(" (not a number: a division by zero)");
        }
        Verdict { pass, lhs, rhs, measured, text }
    }
}

/// A measured level for a message: three decimals (`0.083`), scientific below a thousandth, `0` for silence.
pub fn level_text(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if !v.is_finite() {
        format!("{v}")
    } else if v.abs() >= 0.001 {
        format!("{v:.3}")
    } else {
        format!("{v:.1e}")
    }
}

fn eval_level(e: &Expr, leaf: &dyn Fn(&str) -> f64) -> f64 {
    match e {
        Expr::Num(r, _) => r.to_f64(),
        Expr::Word(w, _) => leaf(w),
        Expr::Neg(x, _) => -eval_level(x, leaf),
        Expr::Bin(op, a, b, _) => {
            let (x, y) = (eval_level(a, leaf), eval_level(b, leaf));
            match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => x / y,
                BinOp::Rem => x.rem_euclid(y),
            }
        }
        Expr::Call(name, args, _) => {
            let vals: Vec<f64> = args.iter().map(|a| eval_level(a, leaf)).collect();
            vals.into_iter().reduce(|a, b| if name == "min" { a.min(b) } else { a.max(b) }).unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    }
}

/// How long the program is, in seconds: from its first bar to the bar it ends on (a program that never ends: one lap of its
/// song), through the tempo map (F3: [`crate::tempo::TempoMap`], exact under a curve, a section's own tempo and
/// `tempo A -> B over N bars`; the same number `check`'s `length:` line says). This is the one place a length in seconds is
/// made. 0 for a program with no bars to measure (no end and no timeline).
pub fn program_seconds(c: &Compiled) -> f64 {
    c.length_seconds().unwrap_or(0.0)
}

/// The names of the sections on the timeline, in order, each once. A stretch where sections play together (F10:
/// `arrange stack(a, b)`) is a segment named `a+b`: its sections are `a` and `b`, which is what `in` names.
fn section_names(c: &Compiled) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for s in &c.segments {
        for n in s.name.split('+') {
            if !v.iter().any(|x| x == n) {
                v.push(n.to_string());
            }
        }
    }
    v
}

/// The stretches of the timeline (start bar, end bar) where the section `name` plays: every segment that has it, a stretch of
/// a stack that holds it, a later generation of its evolve, the next net of a long one. A stack of unequal lengths is cut into
/// stretches where its sections end, so a shorter section's stretches are the bars it has, not the bars the stack lasts.
pub(crate) fn section_windows(c: &Compiled, name: &str) -> Vec<(u32, u32)> {
    c.segments.iter().filter(|s| s.name.split('+').any(|n| n == name)).map(|s| (s.start, s.start + s.bars)).collect()
}

/// `in NAME` names no section on the timeline.
pub(crate) fn no_section(c: &Compiled, w: &Word) -> Diagnostic {
    let names = section_names(c);
    if names.is_empty() {
        return Diagnostic::error(w.span, format!("there is no section `{}`: this program has no sections", w.text))
            .help("a section is `section drop 16 bars { ... }` with an `arrange`; leave out `in` to mean the whole program");
    }
    let d = Diagnostic::error(w.span, format!("there is no section `{}`", w.text)).expected(&names.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{}`", w.text));
    d.help(match crate::env::suggest(&w.text, &names, 1).first() {
        Some(n) => format!("did you mean `{n}`? the sections are {}", names.join(", ")),
        None => format!("the sections on the timeline are {}", names.join(", ")),
    })
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn holds_u64(a: u64, op: CmpOp, b: u64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        CmpOp::Lt => a < b,
        CmpOp::Le => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::Ge => a >= b,
    }
}

/// Seconds against a target, `tol` wide for `==` and `!=`.
fn holds_secs(a: f64, op: CmpOp, t: f64, tol: f64) -> bool {
    let near = (a - t).abs() <= tol + EPS;
    match op {
        CmpOp::Eq => near,
        CmpOp::Ne => !near,
        CmpOp::Lt => a < t - EPS,
        CmpOp::Le => a <= t + EPS,
        CmpOp::Gt => a > t + EPS,
        CmpOp::Ge => a >= t - EPS,
    }
}

/// A reference to an evolve that is not there.
fn no_evolve(c: &Compiled, w: &Word) -> Diagnostic {
    let scopes: Vec<String> = c.evolves.iter().map(|r| r.scope.clone()).collect();
    let list: Vec<&str> = scopes.iter().map(String::as_str).collect();
    if scopes.is_empty() {
        return Diagnostic::error(w.span, format!("`{}` has no evolve to take the period of: this program has no `evolve`", w.text))
            .help("`assert period == N` is the program's own period; `evolve every 8 bars { ... }` in a section makes an evolve");
    }
    if section_names(c).contains(&w.text) {
        return Diagnostic::error(w.span, format!("section `{}` has no `evolve`", w.text)).expected(&list).found(format!("`{}`", w.text)).help(format!("the evolves are in {}", scopes.join(", ")));
    }
    let d = Diagnostic::error(w.span, format!("there is no evolve in `{}`", w.text)).expected(&list).found(format!("`{}`", w.text));
    d.help(match crate::env::suggest(&w.text, &scopes, 1).first() {
        Some(n) => format!("did you mean `{n}`?"),
        None => format!("the evolves are in {}", scopes.join(", ")),
    })
}

/// Every assertion of `program` (its top-level statements) against the compiled program: the static ones decided, the
/// level ones resolved to their windows and left pending. The diagnostics are the errors of a reference that names nothing
/// (`in dorp`) and one note for each pending assertion; a *false* static is a [`Status::Fail`] here and the caller says
/// whether that refuses the program.
pub fn check(program: &Program, c: &Compiled) -> (Vec<Checked>, Vec<Diagnostic>) {
    let mut out: Vec<Checked> = Vec::new();
    let mut diags = Vec::new();
    for st in &program.stmts {
        let StmtKind::Assert(a) = &st.kind else { continue };
        let text = a.text();
        let mut done = |status: Status, message: String, help: Option<String>, level: Option<LevelCheck>| {
            out.push(Checked { span: st.span, text: text.clone(), status, message, help, level });
        };
        match &a.kind {
            AssertKind::Ends => match (c.end, c.looped) {
                (Some(end), _) => done(Status::Pass, format!("the program ends at bar {end}"), None, None),
                (None, true) => done(Status::Fail, "the program loops: `loop` starts the song again at its end".into(), Some("take out the `loop`, or `end at bar N`".into()), None),
                (None, false) => done(
                    Status::Fail,
                    "the program does not end: nothing stops it".into(),
                    Some("`end at bar N` stops a program; a song with sections and an `arrange` ends after its last section".into()),
                    None,
                ),
            },
            AssertKind::Length { op, target, tolerance } => {
                let want = target.secs.to_f64();
                let tol = tolerance.map_or(0.0, |t| t.secs.to_f64());
                match c.end {
                    None => {
                        let lap = c.song_bars().map(|_| format!(" (one lap is {})", seconds_text(program_seconds(c)))).unwrap_or_default();
                        let why = if c.looped { "the program loops, so it has no length" } else { "the program plays on, so it has no length" };
                        done(Status::Fail, format!("{why}{lap}"), Some("`end at bar N`, or a song that ends, gives it one".into()), None);
                    }
                    Some(_) => {
                        let actual = program_seconds(c);
                        let status = if holds_secs(actual, *op, want, tol) { Status::Pass } else { Status::Fail };
                        let through = if c.tempo_map().drives() { " (through the tempo curve)" } else { "" };
                        done(status, format!("the program is {}{through}", seconds_text(actual)), None, None);
                    }
                }
            }
            AssertKind::Key { op, key, section } => {
                let want = Key::parse(&key.text).expect("a key was checked when it was read");
                let (got, whose) = match section {
                    Some(w) => match c.section_key(&w.text) {
                        Some(k) => (k, format!("the key in {}", w.text)),
                        None => {
                            diags.push(no_section(c, w));
                            continue;
                        }
                    },
                    None => (c.key.unwrap_or(Key::C_MAJOR), "the program's key".to_string()),
                };
                let unset = section.is_none() && c.key.is_none();
                let status = if (got == want) == (*op == CmpOp::Eq) { Status::Pass } else { Status::Fail };
                done(status, format!("{whose} is {}{}", got.text(), if unset { " (no `key` is set: C major)" } else { "" }), None, None);
            }
            AssertKind::Period { of: Some(w), op, value, .. } => {
                let Some(run) = c.evolves.iter().find(|r| r.scope == w.text) else {
                    diags.push(no_evolve(c, w));
                    continue;
                };
                let scope = if run.scope == "evolve" { "the program's evolve".to_string() } else { format!("the evolve in {}", run.scope) };
                match run.analysis.period_bars() {
                    Some(p) => {
                        let status = if holds_u64(p, *op, u64::from(*value)) { Status::Pass } else { Status::Fail };
                        let gens = plural(run.analysis.period.unwrap_or(0), "generation", "generations");
                        done(status, format!("the period of {scope} is {} ({gens} of {} bars)", plural(p, "bar", "bars"), run.analysis.every), None, None);
                    }
                    None => {
                        let limit = crate::evolve::group(u64::from(crate::evolve::SEARCH_GENS));
                        done(Status::Fail, format!("{scope} has no period that could be found (no cycle within {limit} generations of a part)"), None, None);
                    }
                }
            }
            AssertKind::Period { of: None, op, value, .. } => {
                let periods = crate::meter::program_period(c);
                let bars = periods.iter().fold(1u64, |a, (_, p)| crate::meter::lcm(a, p.bars).unwrap_or(u64::MAX));
                let mut poly: Vec<String> = Vec::new();
                for (_, p) in &periods {
                    for t in &p.polymeter {
                        if !poly.contains(t) {
                            poly.push(t.clone());
                        }
                    }
                }
                let against = if poly.is_empty() { String::new() } else { format!(" ({} against the bar)", poly.join(", ")) };
                let mut dice: Vec<String> = Vec::new();
                for (_, p) in &periods {
                    for t in &p.chance {
                        if !dice.contains(t) {
                            dice.push(t.clone());
                        }
                    }
                }
                if !dice.is_empty() {
                    // a chance part is a hash of the bar: it differs every bar and never comes round, whatever length its net is
                    // unrolled over (that is the section's, so it would move with the placement): no period to compare (L-10)
                    let rest = format!("the rest comes round in {}{against}", plural(bars, "bar", "bars"));
                    done(Status::Fail, format!("the program has no period: {} {} chance, which differs every bar ({rest})", dice.join(", "), if dice.len() == 1 { "has" } else { "have" }), Some("chance is decided by a hash of the bar, so it never comes round: assert the period of the parts without it, or drop the assertion".into()), None);
                    continue;
                }
                let status = if holds_u64(bars, *op, u64::from(*value)) { Status::Pass } else { Status::Fail };
                done(status, format!("the program's period is {}{against}", plural(bars, "bar", "bars")), None, None);
            }
            AssertKind::Never { x, rel, y, section } => {
                let (verdict, warnings) = constraint::judge_never(c, &text, st.span, x, *rel, y, section);
                diags.extend(warnings);
                match verdict {
                    Judged::Held(message) => done(Status::Pass, message, None, None),
                    Judged::Broken { message, help } => done(Status::Fail, message, Some(help), None),
                    Judged::Refused(d) => diags.push(d),
                }
            }
            AssertKind::EndsOn { part, degree, degree_span, section } => {
                let (verdict, warnings) = constraint::judge_ends_on(c, st.span, part, *degree, *degree_span, section);
                diags.extend(warnings);
                match verdict {
                    Judged::Held(message) => done(Status::Pass, message, None, None),
                    Judged::Broken { message, help } => done(Status::Fail, message, Some(help), None),
                    Judged::Refused(d) => diags.push(d),
                }
            }
            AssertKind::Level { lhs, op, rhs, section } => {
                let (sec, windows) = match section {
                    Some(w) => {
                        let windows = section_windows(c, &w.text);
                        if windows.is_empty() {
                            diags.push(no_section(c, w));
                            continue;
                        }
                        (Some(w.text.clone()), windows)
                    }
                    None => (None, Vec::new()),
                };
                let where_ = sec.as_ref().map_or("the whole program".to_string(), |s| format!("section {s}"));
                let msg = format!("pending: measured on an offline render of {where_} (`btw test`)");
                diags.push(Diagnostic::info(st.span, format!("{text}: {msg}")));
                done(Status::Pending, msg, None, Some(LevelCheck { lhs: lhs.clone(), op: *op, rhs: rhs.clone(), section: sec, windows }));
            }
        }
    }
    (out, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: i64, d: i64) -> Rat {
        Rat::new(n, d).unwrap()
    }

    #[test]
    fn clocks_print_as_minutes_and_seconds() {
        assert_eq!(clock_text(secs(210, 1)), "3m30s");
        assert_eq!(clock_text(secs(90, 1)), "1m30s");
        assert_eq!(clock_text(secs(180, 1)), "3m");
        assert_eq!(clock_text(secs(5, 1)), "5s");
        assert_eq!(clock_text(secs(0, 1)), "0s");
        assert_eq!(clock_text(secs(1, 2)), "0.5s");
        assert_eq!(clock_text(secs(121, 2)), "1m0.5s");
        assert_eq!(clock_text(secs(5400, 1)), "90m");
    }

    #[test]
    fn actual_lengths_are_rounded_to_the_millisecond() {
        assert_eq!(seconds_text(178.0), "2m58s");
        assert_eq!(seconds_text(190.476_190), "3m10.476s");
        assert_eq!(seconds_text(59.9996), "1m");
        assert_eq!(seconds_text(0.0), "0s");
        assert_eq!(seconds_text(12.5), "12.5s");
    }

    #[test]
    fn levels_print_three_decimals() {
        assert_eq!(level_text(0.0834), "0.083");
        assert_eq!(level_text(0.0), "0");
        assert_eq!(level_text(0.000_04), "4.0e-5");
    }

    #[test]
    fn a_level_is_judged_from_its_measurements() {
        let sp = Span::default();
        let l = LevelCheck {
            lhs: Expr::Word("A".into(), sp),
            op: CmpOp::Lt,
            rhs: Expr::Bin(BinOp::Mul, Box::new(Expr::Num(Rat::int(2), sp)), Box::new(Expr::Word("B".into(), sp)), sp),
            section: Some("drop".into()),
            windows: vec![(8, 16)],
        };
        let v = l.judge(&|w| if w == "A" { 0.083 } else { 0.051 });
        assert!(v.pass);
        assert_eq!(v.text, "A 0.083, B 0.051: 0.083 < 0.102");
        let v = l.judge(&|w| if w == "A" { 0.2 } else { 0.051 });
        assert!(!v.pass);
    }
}
