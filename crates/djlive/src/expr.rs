//! Compile-time expressions (docs/BTW-EXTENSIONS.md, feature 2): integer and rational arithmetic,
//! comparisons and `and` / `or` / `not`, over [`Rat`]s and words. Evaluated while the program is read and
//! expanded, never while it plays, and always total: a tree walk with no loops and no recursion beyond the
//! expression's own depth ([`MAX_DEPTH`]); `/` and `%` by zero and an overflow are errors with a span.
//!
//! ```text
//! expr    = or ;
//! or      = and { "or" and } ;
//! and     = not { "and" not } ;
//! not     = [ "not" ] cmp ;
//! cmp     = sum [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) sum ] ;
//! sum     = term { ( "+" | "-" ) term } ;
//! term    = unary { ( "*" | "/" | "%" ) unary } ;
//! unary   = [ "-" ] primary ;
//! primary = number | word | "(" expr ")" | NAME "(" [ expr { "," expr } ] ")" ;   (* min(a, b ...), max(a, b ...) *)
//! ```
//!
//! `$i`, `$n` and a def's `$param` are put in place as tokens before an expression is read, so an
//! expression only ever sees numbers and words. A name followed by `(` other than `min` / `max` is an
//! extension call (`knob(3)` of the performed tier): [`Hooks`] resolves it, and with none it is an error.

use std::collections::BTreeSet;
use std::sync::Mutex;

use crate::ast::{Guard, Num};
use crate::diag::{Diagnostic, Span};
use crate::lex::{Tok, Token};
use crate::parse::Parser;
use crate::rat::{Rat, RatError};

/// The deepest an expression may nest.
pub const MAX_DEPTH: usize = 64;
/// The most operators (`+ - * / %`, comparisons, `and`, `or`, `not`) one expression may have. A chain such as
/// `1 + 1 + 1 + ...` is a left-leaning tree as deep as it is long, and the tree is walked by recursion (evaluating,
/// printing, dropping): this keeps that walk short whatever the stack of the thread that reads the text.
pub const MAX_OPS: usize = 128;

thread_local! {
    static OPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The operators counted for the expression being read; the count before it is back when it is done.
struct OpsScope(usize);

impl OpsScope {
    fn new() -> OpsScope {
        OpsScope(OPS.with(|o| o.replace(0)))
    }
}

impl Drop for OpsScope {
    fn drop(&mut self) {
        OPS.with(|o| o.set(self.0));
    }
}

/// One more operator in the expression being read.
fn count_op(c: &Cursor) -> PR<()> {
    let n = OPS.with(|o| {
        o.set(o.get() + 1);
        o.get()
    });
    if n > MAX_OPS {
        return Err(Diagnostic::error(c.span(), format!("this expression has more than {MAX_OPS} operators")).help("split it up: name a part of it with a `let`, or put it in a `def`"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpKind {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpKind {
    pub fn text(self) -> &'static str {
        match self {
            CmpKind::Eq => "==",
            CmpKind::Ne => "!=",
            CmpKind::Lt => "<",
            CmpKind::Le => "<=",
            CmpKind::Gt => ">",
            CmpKind::Ge => ">=",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(Rat, Span),
    /// A bare word: a deck, a key, a section name. Only `==` and `!=` take one.
    Word(String, Span),
    Neg(Box<Expr>, Span),
    Bin(BinOp, Box<Expr>, Box<Expr>, Span),
    Cmp(CmpKind, Box<Expr>, Box<Expr>, Span),
    Not(Box<Expr>, Span),
    And(Box<Expr>, Box<Expr>, Span),
    Or(Box<Expr>, Box<Expr>, Span),
    /// `min(a, b)`, `max(a, b)` or an extension call.
    Call(String, Vec<Expr>, Span),
    /// A `$name` that was never filled in (outside any `def` or `repeat` that defines it).
    Unbound(String, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Num(_, s) | Expr::Word(_, s) | Expr::Neg(_, s) | Expr::Bin(_, _, _, s) | Expr::Cmp(_, _, _, s) | Expr::Not(_, s) | Expr::And(_, _, s) | Expr::Or(_, _, s) | Expr::Call(_, _, s) | Expr::Unbound(_, s) => *s,
        }
    }

    /// Whether this is just a number as written (no operator): such a value keeps its spelling.
    pub fn is_literal(&self) -> bool {
        matches!(self, Expr::Num(..))
    }

    /// The canonical text of the expression (the form a canonical program prints): fully parenthesised
    /// only where precedence needs it, rationals in lowest terms, one space round a binary operator.
    pub fn canonical(&self) -> String {
        self.print(0)
    }

    /// What a piece of arithmetic that holds no input and no name comes to (`0.25 + 0.25` is `1/2`): the canonical text
    /// prints that number where the arithmetic stood, so equal values are one spelling (L-08). `None` for a literal, a word,
    /// a comparison and anything with an input call (`knob(1)`) in it, whose value is not known until the program plays.
    pub fn folded(&self) -> Option<Rat> {
        match self {
            Expr::Bin(..) | Expr::Neg(..) => {}
            Expr::Call(n, ..) if n == "min" || n == "max" => {}
            _ => return None,
        }
        match eval(self, &NoHooks) {
            Ok(Val::Num(r)) => Some(r),
            _ => None,
        }
    }

    fn print(&self, outer: u8) -> String {
        if let Some(r) = self.folded() {
            return Expr::Num(r, self.span()).print(outer);
        }
        // precedence: or 1, and 2, not 3, cmp 4, sum 5, term 6, unary 7, primary 8
        let (prec, text) = match self {
            // a rational with a decimal of at most six places prints as that decimal (`0.7`, which reads back as the same
            // rational); any other prints as the division it is (`1 / 3`), so that the text says exactly the number and
            // reading it back gives the same text: `2 * (1 / 3)`, `1 / 3 * 2`
            Expr::Num(v, _) if v.is_int() => (8, v.to_string()),
            Expr::Num(v, _) => match v.decimal6() {
                Some(d) => (8, d),
                None => (6, format!("{} / {}", v.numer(), v.denom())),
            },
            Expr::Word(w, _) => (8, w.clone()),
            Expr::Unbound(n, _) => (8, format!("${n}")),
            Expr::Neg(e, _) => (7, format!("-{}", e.print(7))),
            Expr::Bin(op, a, b, _) => {
                let (p, sym) = match op {
                    BinOp::Add => (5, "+"),
                    BinOp::Sub => (5, "-"),
                    BinOp::Mul => (6, "*"),
                    BinOp::Div => (6, "/"),
                    BinOp::Rem => (6, "%"),
                };
                // left associative: the right operand needs parentheses at the same precedence
                (p, format!("{} {sym} {}", a.print(p), b.print(p + 1)))
            }
            Expr::Cmp(k, a, b, _) => (4, format!("{} {} {}", a.print(5), k.text(), b.print(5))),
            Expr::Not(e, _) => (3, format!("not {}", e.print(3))),
            Expr::And(a, b, _) => (2, format!("{} and {}", a.print(2), b.print(3))),
            Expr::Or(a, b, _) => (1, format!("{} or {}", a.print(1), b.print(2))),
            Expr::Call(n, args, _) => (8, format!("{n}({})", args.iter().map(|a| a.print(0)).collect::<Vec<_>>().join(", "))),
        };
        if prec < outer {
            format!("({text})")
        } else {
            text
        }
    }
}

// ---- reading

/// A cursor over tokens: the parser of [`crate::parse`] shares its token vector and position.
pub struct Cursor<'a> {
    pub toks: &'a [Token],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.at.min(self.toks.len() - 1)].tok
    }

    fn peek_n(&self, n: usize) -> &Tok {
        &self.toks[(self.at + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.at.min(self.toks.len() - 1)].span
    }

    fn prev_span(&self) -> Span {
        self.toks[self.at.saturating_sub(1)].span
    }

    fn bump(&mut self) {
        if self.at < self.toks.len() - 1 {
            self.at += 1;
        }
    }

    /// The next two tokens are glued together (no space between): `==`, `<=`, `>=`, `!=`.
    fn glued(&self, n: usize) -> bool {
        let a = &self.toks[(self.at + n).min(self.toks.len() - 1)];
        let b = &self.toks[(self.at + n + 1).min(self.toks.len() - 1)];
        a.span.end == b.span.start
    }

    fn word_is(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Word(x) if x == w)
    }
}

type PR<T> = Result<T, Diagnostic>;

/// Whether the tokens at the cursor can start an expression.
pub fn starts_expr(t: &Tok) -> bool {
    matches!(t, Tok::Num { .. } | Tok::Minus | Tok::LParen | Tok::Word(_) | Tok::Dollar(_))
}

/// A full expression: comparisons and `and` / `or` / `not` included.
pub fn parse_expr(c: &mut Cursor) -> PR<Expr> {
    let _ops = OpsScope::new();
    or(c, 0)
}

/// Arithmetic only (no comparison): the form numbers take where a value is wanted.
pub fn parse_arith(c: &mut Cursor) -> PR<Expr> {
    let _ops = OpsScope::new();
    sum(c, 0)
}

/// One parenthesised expression and nothing after it: `( expr )`. The cursor must be on the `(`.
pub fn parse_paren(c: &mut Cursor) -> PR<Expr> {
    if c.peek() != &Tok::LParen {
        return Err(Diagnostic::error(c.span(), format!("expected `(`, found {}", c.peek().describe())).expected(&["`(`"]).found(c.peek().describe()));
    }
    let _ops = OpsScope::new();
    primary(c, 0)
}

fn too_deep(c: &Cursor) -> Diagnostic {
    Diagnostic::error(c.span(), format!("this expression nests more than {MAX_DEPTH} deep"))
}

fn or(c: &mut Cursor, d: usize) -> PR<Expr> {
    let mut a = and(c, d)?;
    while c.word_is("or") {
        count_op(c)?;
        c.bump();
        let b = and(c, d)?;
        let sp = a.span().to(b.span());
        a = Expr::Or(Box::new(a), Box::new(b), sp);
    }
    Ok(a)
}

fn and(c: &mut Cursor, d: usize) -> PR<Expr> {
    let mut a = not(c, d)?;
    while c.word_is("and") {
        count_op(c)?;
        c.bump();
        let b = not(c, d)?;
        let sp = a.span().to(b.span());
        a = Expr::And(Box::new(a), Box::new(b), sp);
    }
    Ok(a)
}

fn not(c: &mut Cursor, d: usize) -> PR<Expr> {
    if c.word_is("not") {
        count_op(c)?;
        let s = c.span();
        c.bump();
        let e = not(c, d + 1)?;
        let sp = s.to(e.span());
        return Ok(Expr::Not(Box::new(e), sp));
    }
    cmp(c, d)
}

fn cmp(c: &mut Cursor, d: usize) -> PR<Expr> {
    let a = sum(c, d)?;
    let (kind, n) = match (c.peek(), c.peek_n(1)) {
        (Tok::Eq, Tok::Eq) if c.glued(0) => (CmpKind::Eq, 2),
        (Tok::Bang, Tok::Eq) if c.glued(0) => (CmpKind::Ne, 2),
        (Tok::Lt, Tok::Eq) if c.glued(0) => (CmpKind::Le, 2),
        (Tok::Gt, Tok::Eq) if c.glued(0) => (CmpKind::Ge, 2),
        (Tok::Lt, _) => (CmpKind::Lt, 1),
        (Tok::Gt, _) => (CmpKind::Gt, 1),
        _ => return Ok(a),
    };
    count_op(c)?;
    for _ in 0..n {
        c.bump();
    }
    let b = sum(c, d)?;
    let sp = a.span().to(b.span());
    Ok(Expr::Cmp(kind, Box::new(a), Box::new(b), sp))
}

fn sum(c: &mut Cursor, d: usize) -> PR<Expr> {
    let mut a = term(c, d)?;
    loop {
        let op = match c.peek() {
            Tok::Plus => BinOp::Add,
            Tok::Minus => BinOp::Sub,
            _ => return Ok(a),
        };
        count_op(c)?;
        c.bump();
        let b = term(c, d)?;
        let sp = a.span().to(b.span());
        a = Expr::Bin(op, Box::new(a), Box::new(b), sp);
    }
}

fn term(c: &mut Cursor, d: usize) -> PR<Expr> {
    let mut a = unary(c, d)?;
    loop {
        let op = match c.peek() {
            Tok::Star => BinOp::Mul,
            Tok::Slash => BinOp::Div,
            Tok::Percent => BinOp::Rem,
            _ => return Ok(a),
        };
        count_op(c)?;
        c.bump();
        let b = unary(c, d)?;
        let sp = a.span().to(b.span());
        a = Expr::Bin(op, Box::new(a), Box::new(b), sp);
    }
}

fn unary(c: &mut Cursor, d: usize) -> PR<Expr> {
    if d > MAX_DEPTH {
        return Err(too_deep(c));
    }
    if c.peek() == &Tok::Minus {
        let s = c.span();
        c.bump();
        let e = unary(c, d + 1)?;
        let sp = s.to(e.span());
        // a negative literal is one literal
        return Ok(match e {
            Expr::Num(v, _) => match v.neg() {
                Ok(n) => Expr::Num(n, sp),
                Err(_) => Expr::Neg(Box::new(Expr::Num(v, sp)), sp),
            },
            e => Expr::Neg(Box::new(e), sp),
        });
    }
    primary(c, d)
}

fn primary(c: &mut Cursor, d: usize) -> PR<Expr> {
    if d > MAX_DEPTH {
        return Err(too_deep(c));
    }
    let s = c.span();
    match c.peek().clone() {
        Tok::Num { value, .. } => {
            c.bump();
            match Rat::from_f64(value) {
                Some(r) => Ok(Expr::Num(r, s)),
                None => Err(Diagnostic::error(s, "this number is too large or too precise for exact arithmetic").help("numbers are whole or rational, with at most six decimals")),
            }
        }
        Tok::Dollar(n) => {
            c.bump();
            Ok(Expr::Unbound(n, s))
        }
        Tok::LParen => {
            c.bump();
            let e = or(c, d + 1)?;
            if c.peek() != &Tok::RParen {
                return Err(Diagnostic::error(c.span(), "expected `)` to close the parenthesis").expected(&["`)`"]).found(c.peek().describe()).help(format!("it opens at byte {}", s.start)));
            }
            c.bump();
            let sp = s.to(c.prev_span());
            // parentheses are not part of the value: keep the inner expression, widen its span
            Ok(match e {
                Expr::Num(v, _) => Expr::Num(v, sp),
                other => other,
            })
        }
        Tok::Word(w) => {
            c.bump();
            if c.peek() == &Tok::LParen {
                c.bump();
                let mut args = Vec::new();
                if c.peek() != &Tok::RParen {
                    loop {
                        args.push(or(c, d + 1)?);
                        if c.peek() == &Tok::Comma {
                            c.bump();
                        } else {
                            break;
                        }
                    }
                }
                if c.peek() != &Tok::RParen {
                    return Err(Diagnostic::error(c.span(), format!("expected `)` to close `{w}(`")).expected(&["`,`", "`)`"]).found(c.peek().describe()));
                }
                c.bump();
                // (F13: `held(C4)` and `held(c4)` are one input: a note has one spelling)
                let args = crate::perform::canonical_args(&w, args);
                return Ok(Expr::Call(w, args, s.to(c.prev_span())));
            }
            Ok(Expr::Word(w, s))
        }
        other => Err(Diagnostic::error(s, format!("expected a number or a name, found {}", other.describe())).expected(&["a number", "a name", "`(`"]).found(other.describe())),
    }
}

// ---- evaluating

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(Rat),
    Word(String),
    Bool(bool),
}

impl Val {
    pub fn describe(&self) -> String {
        match self {
            Val::Num(n) => format!("the number {n}"),
            Val::Word(w) => format!("the word `{w}`"),
            Val::Bool(b) => format!("{b}"),
        }
    }
}

/// What a host adds to the evaluator: extension calls such as `knob(3)`.
pub trait Hooks {
    /// A call other than `min` / `max`. `None`: not known here.
    fn call(&self, _name: &str, _args: &[Val], _span: Span) -> Option<Result<Val, Diagnostic>> {
        None
    }
}

/// No extension calls.
pub struct NoHooks;

impl Hooks for NoHooks {}

fn rat_err(e: RatError, span: Span) -> Diagnostic {
    Diagnostic::error(span, e.to_string())
}

fn need_num(v: Val, span: Span, what: &str) -> PR<Rat> {
    match v {
        Val::Num(n) => Ok(n),
        other => Err(Diagnostic::error(span, format!("{what} needs a number, found {}", other.describe()))),
    }
}

pub fn eval(e: &Expr, hooks: &dyn Hooks) -> PR<Val> {
    match e {
        Expr::Num(n, _) => Ok(Val::Num(*n)),
        Expr::Word(w, _) => Ok(Val::Word(w.clone())),
        Expr::Unbound(n, sp) => Err(Diagnostic::error(*sp, format!("`${n}` has no value here")).help("`$i` and `$n` are the index and count of the `repeat` they are in; `$name` is a parameter of the `def` it is in")),
        Expr::Neg(x, sp) => {
            let v = need_num(eval(x, hooks)?, x.span(), "`-`")?;
            v.neg().map(Val::Num).map_err(|er| rat_err(er, *sp))
        }
        Expr::Bin(op, a, b, sp) => {
            let (x, y) = (need_num(eval(a, hooks)?, a.span(), "arithmetic")?, need_num(eval(b, hooks)?, b.span(), "arithmetic")?);
            let r = match op {
                BinOp::Add => x.add(y),
                BinOp::Sub => x.sub(y),
                BinOp::Mul => x.mul(y),
                BinOp::Div => x.div(y),
                BinOp::Rem => x.rem(y),
            };
            r.map(Val::Num).map_err(|er| match er {
                RatError::DivideByZero => Diagnostic::error(b.span(), if *op == BinOp::Rem { "modulo by zero" } else { "division by zero" }),
                other => rat_err(other, *sp),
            })
        }
        Expr::Cmp(k, a, b, sp) => {
            let (x, y) = (eval(a, hooks)?, eval(b, hooks)?);
            let holds = match (&x, &y) {
                (Val::Num(p), Val::Num(q)) => match k {
                    CmpKind::Eq => p == q,
                    CmpKind::Ne => p != q,
                    CmpKind::Lt => p < q,
                    CmpKind::Le => p <= q,
                    CmpKind::Gt => p > q,
                    CmpKind::Ge => p >= q,
                },
                (Val::Word(p), Val::Word(q)) => match k {
                    CmpKind::Eq => p == q,
                    CmpKind::Ne => p != q,
                    _ => return Err(Diagnostic::error(*sp, format!("words can be compared with `==` and `!=`, not `{}`", k.text()))),
                },
                (Val::Bool(p), Val::Bool(q)) => match k {
                    CmpKind::Eq => p == q,
                    CmpKind::Ne => p != q,
                    _ => return Err(Diagnostic::error(*sp, format!("conditions can be compared with `==` and `!=`, not `{}`", k.text()))),
                },
                _ => return Err(Diagnostic::error(*sp, format!("cannot compare {} with {}", x.describe(), y.describe())).help("a number is compared with a number, a word with a word")),
            };
            Ok(Val::Bool(holds))
        }
        Expr::Not(x, _) => match eval(x, hooks)? {
            Val::Bool(b) => Ok(Val::Bool(!b)),
            other => Err(Diagnostic::error(x.span(), format!("`not` needs a condition, found {}", other.describe()))),
        },
        Expr::And(a, b, _) | Expr::Or(a, b, _) => {
            let is_and = matches!(e, Expr::And(..));
            let cond = |x: &Expr| -> PR<bool> {
                match eval(x, hooks)? {
                    Val::Bool(b) => Ok(b),
                    other => Err(Diagnostic::error(x.span(), format!("`{}` needs conditions, found {}", if is_and { "and" } else { "or" }, other.describe()))),
                }
            };
            let l = cond(a)?;
            // both sides are always checked, so an error in the right one is not hidden by the left
            let r = cond(b)?;
            Ok(Val::Bool(if is_and { l && r } else { l || r }))
        }
        Expr::Call(name, args, sp) => {
            let vals = args.iter().map(|a| eval(a, hooks)).collect::<PR<Vec<_>>>()?;
            match name.as_str() {
                "min" | "max" => {
                    if vals.is_empty() {
                        return Err(Diagnostic::error(*sp, format!("`{name}` needs at least one number")).help(format!("{name}(a, b)")));
                    }
                    let mut it = vals.into_iter().zip(args.iter());
                    let (first, fe) = it.next().expect("non-empty");
                    let mut best = need_num(first, fe.span(), name)?;
                    for (v, ex) in it {
                        let n = need_num(v, ex.span(), name)?;
                        best = if name == "min" { best.min(n) } else { best.max(n) };
                    }
                    Ok(Val::Num(best))
                }
                _ => match hooks.call(name, &vals, *sp) {
                    Some(r) => r,
                    None if crate::perform::is_input_word(name) => Err(crate::perform::not_here(name, *sp)),
                    None => {
                        let near = crate::parse::nearest(name, ["min", "max", "knob", "pad", "fader", "held"]).map_or(String::new(), |n| format!("did you mean `{n}`? "));
                        Err(Diagnostic::error(*sp, format!("`{name}(...)` is not a function of arithmetic")).help(format!("{near}`min(a, b)` and `max(a, b)` are the functions of the language; `knob(3)`, `pad(5)`, `fader(A)` and `held(key)` are performed inputs, only for control values")))
                    }
                },
            }
        }
    }
}

/// Evaluate to a number.
pub fn eval_num(e: &Expr, hooks: &dyn Hooks) -> PR<Rat> {
    need_num(eval(e, hooks)?, e.span(), "this").map_err(|d| Diagnostic { message: format!("this is not a number: {}", d.message), ..d })
}

/// Evaluate to a condition.
pub fn eval_bool(e: &Expr, hooks: &dyn Hooks) -> PR<bool> {
    match eval(e, hooks)? {
        Val::Bool(b) => Ok(b),
        other => Err(Diagnostic::error(e.span(), format!("this is not a condition: it is {}", other.describe())).help("a condition compares: `$i % 4 == 3`, `$deck == A`")),
    }
}

/// Parse and evaluate `src` as a number (for tests and for hosts).
pub fn eval_str(src: &str) -> Result<Val, Diagnostic> {
    let lexed = crate::lex::lex(src);
    if let Some(d) = lexed.errors.into_iter().next() {
        return Err(d);
    }
    let mut c = Cursor { toks: &lexed.tokens, at: 0 };
    let e = parse_expr(&mut c)?;
    if c.peek() != &Tok::Eof {
        return Err(Diagnostic::error(c.span(), format!("unexpected {} after the expression", c.peek().describe())));
    }
    eval(&e, &NoHooks)
}

// ---- canonical texts, interned

/// The canonical text of an arithmetic expression, kept for good: an AST number is `Copy` and carries its text as
/// a `&'static str`, so the one place that needs it (the canonical printer) reads it without a table to thread
/// through. Equal texts are stored once; a program has a handful, an editing session a few thousand at most.
pub fn intern(s: &str) -> &'static str {
    static TABLE: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = t.get(s) {
        return x;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    t.insert(leaked);
    leaked
}

// ---- the parser's side: arithmetic where a number is wanted

type R<T> = Result<T, Diagnostic>;

pub fn is_op(t: &Tok) -> bool {
    matches!(t, Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent)
}

/// A token that may be part of arithmetic over plain numbers (a `let`'s value, a def's argument).
pub fn numeric_token(t: &Tok) -> bool {
    matches!(t, Tok::Num { .. } | Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent | Tok::LParen | Tok::RParen | Tok::Comma) || matches!(t, Tok::Word(w) if w == "min" || w == "max")
}

/// Whether `toks` (which are all [`numeric_token`]s) is an expression worth reading as one rather than a number
/// or a pattern: it has an operator, a parenthesis or a `min` / `max` call in it.
pub fn has_arithmetic(toks: &[Token]) -> bool {
    toks.len() > 1 && toks.iter().any(|t| is_op(&t.tok) || matches!(t.tok, Tok::LParen) || matches!(&t.tok, Tok::Word(w) if w == "min" || w == "max"))
}

/// Whether a def's argument must be wrapped in parentheses where it is put in place, so that `$x * 2` with
/// `x = 1 + 2` is `(1 + 2) * 2`: it is all numbers and has an operator outside any parenthesis, or is a `min(...)` /
/// `max(...)` call (a leading `-` and the plain ratio `1/4`, which a `do` value reads as a word, are left as they are).
/// Where the argument is put matters too: see [`pattern_like`] and [`needs_parens_glued`].
pub fn needs_parens(arg: &[Token]) -> bool {
    let arg = &*crate::perform::collapse_inputs(arg);
    if arg.len() < 2 || !arg.iter().all(|t| numeric_token(&t.tok)) {
        return false;
    }
    if arg.len() == 3 && matches!(arg[0].tok, Tok::Num { .. }) && arg[1].tok == Tok::Slash && matches!(arg[2].tok, Tok::Num { .. }) {
        return false;
    }
    // a call: `min(1, 2)`, `max(1, 2)` (it does not read where a glued count or cycle is: `x*min(1, 2)`)
    if matches!(&arg[0].tok, Tok::Word(w) if w == "min" || w == "max") && arg[1].tok == Tok::LParen {
        return true;
    }
    let mut depth = 0i32;
    for (i, t) in arg.iter().enumerate() {
        match &t.tok {
            Tok::LParen => depth += 1,
            Tok::RParen => depth -= 1,
            Tok::Plus | Tok::Star | Tok::Slash | Tok::Percent if depth == 0 => return true,
            Tok::Minus if depth == 0 && i > 0 => return true,
            _ => {}
        }
    }
    false
}

/// Whether an argument that [`needs_parens`] could also be read, as it stands, as steps of a pattern with a meaning of
/// its own: numbers and `*` at the top level only (`1*4` is the degree 1 four times, `1*2 3` is two terms). Put in place
/// in a step of a pattern, such an argument is left as written (wrapped, `(1 * 4)` would be the degree 4 once): the
/// meaning it had before arithmetic was added to the language. An argument with any other operator, or a call, was
/// never a pattern, so it is wrapped there as everywhere.
pub fn pattern_like(arg: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in arg {
        match &t.tok {
            Tok::LParen => depth += 1,
            Tok::RParen => depth -= 1,
            Tok::Num { .. } | Tok::Star => {}
            _ if depth > 0 && numeric_token(&t.tok) => {}
            _ => return false,
        }
    }
    true
}

/// [`needs_parens`] for a slot that is written glued to what is before it (`x*$k`, `x?$p`, `hihat/$n`, `weights $w 1 1`):
/// a plain ratio is wrapped there too (`x*(8 / 2)`), since the slot cannot read `8/2` as it stands.
pub fn needs_parens_glued(arg: &[Token]) -> bool {
    if needs_parens(arg) {
        return true;
    }
    arg.len() == 3 && matches!(arg[0].tok, Tok::Num { .. }) && arg[1].tok == Tok::Slash && matches!(arg[2].tok, Tok::Num { .. })
}

/// A `let`'s value that is arithmetic: `run` is its tokens up to the end of the line. With the scalar lets put in
/// place they are all numbers and operators, with at least one operator, parenthesis or `min` / `max` call, and
/// they read as one expression. `Ok(None)`: it is not arithmetic and the old reading stands (a token or a pattern).
/// `Ok(Some(t))`: the value folded to one number token (its span is the whole expression). An error: it looked
/// like arithmetic and could not be one (`let n = 1 / 0`, `let n = (1 + 2`, a value too precise to be a decimal).
pub fn fold_let(run: &[Token], scalars: &std::collections::BTreeMap<String, Token>) -> Result<Option<Token>, Diagnostic> {
    let mut mapped: Vec<Token> = Vec::with_capacity(run.len() + 1);
    for (k, t) in run.iter().enumerate() {
        match &t.tok {
            // (`min(` and `max(` are the functions even when a let is named `min`: reserved by position)
            Tok::Word(w) if scalars.contains_key(w) && !(matches!(w.as_str(), "min" | "max") && run.get(k + 1).is_some_and(|n| n.tok == Tok::LParen)) => match scalars[w].tok {
                Tok::Num { .. } => mapped.push(Token { tok: scalars[w].tok.clone(), span: t.span }),
                _ => return Ok(None),
            },
            tok if numeric_token(tok) => mapped.push(t.clone()),
            _ => return Ok(None),
        }
    }
    if !has_arithmetic(&mapped) {
        return Ok(None);
    }
    let (Some(first), Some(last)) = (mapped.first(), mapped.last()) else { return Ok(None) };
    let span = first.span.to(last.span);
    mapped.push(Token { tok: Tok::Eof, span: Span::new(span.end, span.end) });
    let mut c = Cursor { toks: &mapped, at: 0 };
    let e = parse_arith(&mut c)?;
    if c.peek() != &Tok::Eof {
        // a number or two after the expression: not one expression, so a pattern after all (`3*2 4`)
        return Ok(None);
    }
    let r = eval_num(&e, &NoHooks)?;
    let v = r.to_f64();
    if Rat::from_f64(v) != Some(r) {
        return Err(Diagnostic::error(span, format!("{r} cannot be written as a plain number here")).help("a value put in place of a name is a decimal with at most six places: use a result such as 0.25 or 1/3, not 1/7000000"));
    }
    Ok(Some(Token { tok: Tok::Num { value: v, int: r.is_int() }, span }))
}

impl Parser<'_> {
    /// Whether the number about to be read is arithmetic rather than one literal: a literal with an operator after
    /// it, a `(`, a `$name`, a `min(` / `max(` call (each possibly after a `-`). A lone literal is not: it keeps its
    /// old reading, its spelling and its exact value.
    pub(crate) fn starts_arith(&self) -> bool {
        let k = usize::from(self.peek() == &Tok::Minus);
        match self.peek_n(k) {
            Tok::Num { .. } => is_op(self.peek_n(k + 1)),
            Tok::LParen | Tok::Dollar(_) => true,
            Tok::Word(w) => (w == "min" || w == "max" || crate::perform::is_input_word(w)) && self.peek_n(k + 1) == &Tok::LParen,
            _ => false,
        }
    }

    /// Read arithmetic (`+ - * / %`, parentheses, `min` / `max`) and evaluate it: the number, folded, with the
    /// expression's canonical text kept for the canonical form (a single literal keeps its own spelling).
    pub(crate) fn arith_num(&mut self, what: &str) -> R<Num> {
        let (r, e, span) = self.arith_eval(what)?;
        Ok(num_of(r, &e, span))
    }

    /// Arithmetic or a lone literal, as the exact rational it is: where a length or a position is read (docs/btw/ext-timing.md).
    pub(crate) fn arith_rat(&mut self, what: &str) -> R<(Rat, Span)> {
        if self.starts_arith() {
            let (r, _, span) = self.arith_eval(what)?;
            return Ok((r, span));
        }
        let n = self.num_lit(what)?;
        match Rat::from_f64(n.value) {
            Some(r) => Ok((r, n.span)),
            None => Err(Diagnostic::error(n.span, "this number is too large or too precise for exact arithmetic").help("numbers are whole or rational, with at most six decimals")),
        }
    }

    fn arith_eval(&mut self, what: &str) -> R<(Rat, Expr, Span)> {
        let first = self.at;
        let mut c = Cursor { toks: &self.toks, at: self.at };
        let e = parse_arith(&mut c).map_err(|d| if d.help.is_none() { d.help(format!("{what} may be arithmetic: `60 + 6`, `(2 + 1) * 4`, `min(8, 4 * 2)`")) } else { d });
        let e = e.map_err(|d| self.at_last_token(d))?;
        let last = c.at;
        let span = self.toks[first].span.to(self.toks[last.saturating_sub(1).max(first)].span);
        let r = eval_num(&e, &crate::perform::ParseHooks).map_err(|d| crate::perform::in_place_of(d, what))?;
        self.at = last;
        Ok((r, e, span))
    }

    /// A diagnostic that points at the end of the text points at the last token instead (an expression that was
    /// cut off: `tempo 1 +`).
    fn at_last_token(&self, d: Diagnostic) -> Diagnostic {
        if self.toks.len() > 1 && self.toks.last().is_some_and(|e| e.tok == Tok::Eof && d.span == e.span) {
            return Diagnostic { span: self.toks[self.toks.len() - 2].span, ..d };
        }
        d
    }

    /// `( arith )` read as one whole number, and nothing after the `)`: a degree `($i + 1)`, a count `x*(...)`.
    pub(crate) fn paren_int(&mut self, what: &str) -> R<(i64, Span)> {
        let (r, span) = self.paren_rat()?;
        match r.as_int() {
            Some(n) => Ok((n, span)),
            None => Err(Diagnostic::error(span, format!("{what} is a whole number")).found(format!("{r}")).help("`/` is exact: `(7 / 2)` is 7/2, not 3; use `%` for a remainder")),
        }
    }

    /// `( arith )` as the exact number it comes to, and nothing after the `)`: where a slot that took a bare number
    /// (a chance `x?(1 / 4)`, a part's cycle `hihat/(4 + 8)`, a choice's weight) takes arithmetic.
    pub(crate) fn paren_rat(&mut self) -> R<(Rat, Span)> {
        let first = self.at;
        let mut c = Cursor { toks: &self.toks, at: self.at };
        let e = parse_paren(&mut c).map_err(|d| self.at_last_token(d))?;
        let last = c.at;
        let span = self.toks[first].span.to(self.toks[last - 1].span);
        let r = eval_num(&e, &crate::perform::ParseHooks)?;
        self.at = last;
        Ok((r, span))
    }

    /// `when <condition>` after a line. The `when` is the next token.
    pub(crate) fn guard(&mut self) -> R<Guard> {
        let start = self.bump().span;
        let mut c = Cursor { toks: &self.toks, at: self.at };
        let e = parse_expr(&mut c).map_err(|d| if d.help.is_none() { d.help("a guard is a condition: `when $i % 4 == 3`, `when $deck == A`") } else { d });
        let e = e.map_err(|d| self.at_last_token(d))?;
        let last = c.at;
        self.at = last;
        if crate::perform::mentions_input(&e) {
            crate::perform::check_guard(&e)?;
        }
        Ok(Guard { span: start.to(self.prev_span()), expr: e })
    }
}

/// The AST number for a value that came from `e`: a single literal keeps its own spelling, anything else is
/// remembered as the expression's canonical text.
pub fn num_of(r: Rat, e: &Expr, span: Span) -> Num {
    // arithmetic with an input in it keeps its expression (the value is a placeholder: it is read while the program plays);
    // any other is the number it comes to, as a literal of that value would print (`0.25 + 0.25` is `0.5`, `1/3` is `1 / 3`)
    let canon = if e.is_literal() {
        None
    } else if crate::perform::mentions_input(e) {
        Some(intern(&e.canonical()))
    } else {
        Some(intern(&Expr::Num(r, span).canonical()))
    };
    Num { value: r.to_f64(), int: r.is_int(), span, canon }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(src: &str) -> String {
        match eval_str(src) {
            Ok(Val::Num(r)) => r.to_string(),
            Ok(other) => other.describe(),
            Err(d) => format!("error: {}", d.message),
        }
    }

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(n("2 + 3 * 4"), "14");
        assert_eq!(n("(2 + 3) * 4"), "20");
        assert_eq!(n("10 - 4 - 3"), "3");
        assert_eq!(n("2 * 3 % 4"), "2");
        assert_eq!(n("-3 + 5"), "2");
        assert_eq!(n("- (2 + 1)"), "-3");
    }

    #[test]
    fn rationals_are_exact() {
        assert_eq!(n("1 / 3 + 1 / 6"), "1/2");
        assert_eq!(n("3 / 4 * 4"), "3");
        assert_eq!(n("0.25 + 0.5"), "3/4");
        assert_eq!(n("7 % 2"), "1");
        assert_eq!(n("1/2 % 1/3"), "1/6");
    }

    #[test]
    fn min_and_max() {
        assert_eq!(n("min(3, 7)"), "3");
        assert_eq!(n("max(3, 7, 5)"), "7");
        assert_eq!(n("max(1, 2) * min(4, 3)"), "6");
        assert!(n("min()").starts_with("error"));
    }

    #[test]
    fn comparisons_and_conditions() {
        assert_eq!(n("3 % 4 == 3"), "true");
        assert_eq!(n("7 % 4 == 3 and 2 < 3"), "true");
        assert_eq!(n("not 1 == 1 or 2 >= 3"), "false");
        assert_eq!(n("1 != 2"), "true");
        assert_eq!(n("2 <= 2"), "true");
        assert_eq!(n("A == A"), "true");
        assert_eq!(n("A != B"), "true");
        assert!(n("A < B").starts_with("error"));
        assert!(n("A == 1").starts_with("error"));
    }

    #[test]
    fn errors_are_not_panics() {
        assert!(n("1 / 0").contains("division by zero"));
        assert!(n("1 % 0").contains("modulo by zero"));
        assert!(n("9223372036854775807 * 9223372036854775807").contains("too large"));
        assert!(n("2 +").starts_with("error"));
        assert!(n("(1 + 2").starts_with("error"));
        assert!(n("foo(1)").contains("not a function"));
    }

    #[test]
    fn depth_is_bounded() {
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(n(&deep).contains("nests more than"));
    }

    #[test]
    fn canonical_text_is_a_fixed_point() {
        for src in ["2+3*4", "(2+3)*4", "1/2 + 3/4", "a == b and not c < d", "min(1,2)+max(3,4)", "10-(4-3)", "2*(3 % 4)"] {
            let lexed = crate::lex::lex(src);
            let mut c = Cursor { toks: &lexed.tokens, at: 0 };
            let e = parse_expr(&mut c).unwrap();
            let once = e.canonical();
            let lexed2 = crate::lex::lex(&once);
            let mut c2 = Cursor { toks: &lexed2.tokens, at: 0 };
            let e2 = parse_expr(&mut c2).unwrap();
            assert_eq!(once, e2.canonical(), "{src}");
        }
    }

    #[test]
    fn rational_literals_print_as_the_division_they_are_so_that_the_text_reads_back_the_same() {
        for src in ["2 * 0.3", "0.3 * 2", "0.5 - 0.25", "1/3 * 3/4", "10 - 0.5 - 0.25", "-0.5 * 4", "2 * (1/4)", "(0.1 + 0.2) / 0.3"] {
            let lexed = crate::lex::lex(src);
            let mut c = Cursor { toks: &lexed.tokens, at: 0 };
            let e = parse_expr(&mut c).unwrap();
            let once = e.canonical();
            let lexed2 = crate::lex::lex(&once);
            let mut c2 = Cursor { toks: &lexed2.tokens, at: 0 };
            let e2 = parse_expr(&mut c2).unwrap();
            assert_eq!(once, e2.canonical(), "{src} -> {once}");
            assert_eq!(eval(&e, &NoHooks).unwrap(), eval(&e2, &NoHooks).unwrap(), "{src} -> {once}");
        }
    }

    #[test]
    fn interned_texts_are_stored_once() {
        let a = intern("60 + 6");
        let b = intern(&format!("60 {} 6", '+'));
        assert!(std::ptr::eq(a, b));
        assert_eq!(a, "60 + 6");
    }

    fn toks(src: &str) -> Vec<Token> {
        let mut t = crate::lex::lex(src).tokens;
        t.pop();
        t
    }

    #[test]
    fn a_def_argument_is_wrapped_only_when_it_is_arithmetic_with_an_operator_outside_parentheses() {
        for (src, wrap) in [("1 + 2", true), ("2 * 3", true), ("4 - 1", true), ("a * 2", false), ("-3", false), ("3", false), ("(1 + 2)", false), ("1/4", false), ("min(1, 2)", true), ("max(1, 2) * 3", true), ("(1 + 2) * 3", true), ("x*4", false), ("[x ~ x]", false)] {
            assert_eq!(needs_parens(&toks(src)), wrap, "{src}");
        }
    }

    #[test]
    fn an_argument_that_reads_as_steps_of_a_pattern_is_the_one_with_only_stars_at_the_top() {
        for (src, like) in [("1*4", true), ("1*2 3", true), ("(1 + 2)*3", true), ("3 3", true), ("1 + 2", false), ("1 - 2", false), ("4/2", false), ("min(1, 2)", false), ("2 * 3 + 1", false), ("x*4", false)] {
            assert_eq!(pattern_like(&toks(src)), like, "{src}");
        }
        assert!(needs_parens_glued(&toks("8/2")) && !needs_parens(&toks("8/2")));
        assert!(!needs_parens_glued(&toks("8")) && !needs_parens_glued(&toks("-3")));
    }

    #[test]
    fn a_let_value_is_arithmetic_only_with_an_operator_a_parenthesis_or_a_call() {
        use std::collections::BTreeMap;
        let scalars: BTreeMap<String, Token> = [("n".to_string(), toks("8")[0].clone())].into_iter().collect();
        let fold = |src: &str| fold_let(&toks(src), &scalars).map(|o| o.map(|t| match t.tok { Tok::Num { value, int } => (value, int), other => panic!("{other:?}") }));
        assert_eq!(fold("2 * 4"), Ok(Some((8.0, true))));
        assert_eq!(fold("n / 2 + 1"), Ok(Some((5.0, true))));
        assert_eq!(fold("1 / 4"), Ok(Some((0.25, false))));
        assert_eq!(fold("min(3, n)"), Ok(Some((3.0, true))));
        assert_eq!(fold("-3"), Ok(Some((-3.0, true))));
        assert_eq!(fold("3 3"), Ok(None));
        assert_eq!(fold("3*2 4"), Ok(None));
        assert_eq!(fold("[3 3]"), Ok(None));
        assert_eq!(fold("c2 + 1"), Ok(None));
        assert!(fold("1 / 0").is_err());
        assert!(fold("(1 +").is_err());
    }

    #[test]
    fn unbound_is_an_error_with_help() {
        let e = {
            let lexed = crate::lex::lex("$i + 1");
            let mut c = Cursor { toks: &lexed.tokens, at: 0 };
            parse_expr(&mut c).unwrap()
        };
        let d = eval(&e, &NoHooks).unwrap_err();
        assert!(d.message.contains("`$i`"));
        assert!(d.help.is_some());
    }
}
