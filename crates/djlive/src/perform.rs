//! F13 the performed tier (docs/beatscript/ext-perform.md): `knob(3)`, `pad(5)`, `fader(A)`, `held(key)` and `held(c4)` read the
//! controllers while a program plays, and every value read is logged into a **performance** that replays the program's
//! controls exactly.
//!
//! ```text
//! input  = "knob" "(" int ")" | "pad" "(" int ")" | "fader" "(" deck ")" | "held" "(" ( "key" | note ) ")" ;
//! value  = ... | arith-with-inputs ;        (* a control value: 200 + knob(3) * 7800 *)
//! guard  = "when" expr-with-inputs ;        (* on a control line: when held(key) *)
//! ```
//!
//! An input is a number in 0..1, quantised to 1/128 when it is read. It stands only where a program is *played*, never where
//! it is *read*: in a control value (a filter cutoff, a level, a `do` param, a ramp's end) and in a control line's `when`. A
//! pattern is compiled once per section, so an input in one is an error that says so (and what to write instead); a number the
//! program needs while it is read (a tempo, a count, a time, an `evolve` rule's number) is an error that says the same.
//!
//! How it fits together:
//!
//! * **Reading** (parse): an input call in a control value evaluates to 1/2 so that the rest of the language sees a number; the
//!   [`crate::ast::Num`] carries the expression's canonical text (`knob(3)` prints as written). A guard is read as an
//!   expression and kept on the line (`expand` does not decide it: [`mentions_input`]).
//! * **Compiling**: the control compiles as if every input read 1/2 (so the names, the registry and the ranges are checked), and
//!   [`Compiler::performed`] wraps that effect in [`Effect::Performed`]: the compiled effect, the guard, and a [`Patch`] for
//!   every live value saying where it goes in the calls and how it is shaped (the filter's own knob curve, the range it is held to).
//! * **Playing** ([`PerfState`], used by [`crate::runtime::Live`]): a performed line is decided when it is due, with the inputs as
//!   the host answers them at that bar ([`crate::runtime::LiveHost::input`]); each input is read **once a bar** (the value of the
//!   bar is kept for the bar, so a line, a ramp and a guard that name `knob(3)` agree), and a ramp that names one re-reads it at
//!   every bar boundary. Every value read is logged: (program bar, input, value).
//! * **Replaying**: [`crate::runtime::Live::replay`] answers the same reads from a [`Performance`] instead of the host, by bar.
//!   The same program, the same clock and the same log make the same calls, in the same order, with the same numbers.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use djengine::cid;
use djengine::json::{JsonObject, JsonValue};

use crate::ast::{Action, Body, Deck, DoValue, Line, Num, Target};
use crate::compile::{sweep_knob, sweep_span, Compiler, Effect, Timing};
use crate::diag::{Diagnostic, Span};
use crate::expand::FlatLine;
use crate::expr::{BinOp, CmpKind, Expr, Hooks, NoHooks, Val};
use crate::rat::Rat;
use crate::runtime::LiveHost;
use crate::shape::{LfoSpec, Motion};

/// The words that make a call an input.
pub const INPUT_WORDS: [&str; 4] = ["knob", "pad", "fader", "held"];

/// Whether `w` names an input (`knob` ...). A word is an input only when a `(` follows it.
pub fn is_input_word(w: &str) -> bool {
    INPUT_WORDS.contains(&w)
}

/// How many knobs and pads a controller bank has. The connected controllers' own bank (`Env::bank`, `Board::bank()`) is host data, shown
/// by `btw perform` and not consulted by `check`: whether `knob(12)` is acceptable is the same on every machine ([`InputBank::MAX`]),
/// and what the connected controllers do not have reads 0 while the program plays, with a word in `btw get` (R-06).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputBank {
    pub knobs: u32,
    pub pads: u32,
}

impl InputBank {
    /// What a bank is when no connected controller has that kind of control.
    pub const DEFAULT: InputBank = InputBank { knobs: 16, pads: 64 };
    /// The most a program may name, on every machine: a MIDI channel has 128 controllers and 128 notes. `knob(129)` is an error
    /// (there cannot be one); `knob(12)` is fine to check, save and share even where the connected controller has 8 knobs, and reads
    /// 0 there, as an input nothing answers for does.
    pub const MAX: InputBank = InputBank { knobs: 128, pads: 128 };
}

/// One input a performed value or guard reads.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Input {
    /// `knob(3)`: the third knob of the connected bank.
    Knob(u32),
    /// `pad(5)`: a pad, 0 or its velocity 0..1.
    Pad(u32),
    /// `fader(A)`: a deck's channel fader.
    Fader(Deck),
    /// `held(key)`: the lowest MIDI key held, over 127 (0 when none).
    HeldKey,
    /// `held(c4)`: 1 while that MIDI note is held, else 0.
    Held(u8),
}

const SHARPS: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

/// A MIDI note's canonical spelling: 60 is `c4` (octave 4 is middle C, as everywhere in the language).
pub fn note_name(midi: u8) -> String {
    format!("{}{}", SHARPS[usize::from(midi % 12)], i32::from(midi / 12) - 1)
}

/// A note word (`c4`, `F#2`, `bb3`) as its MIDI number: 60 is `c4`. None for a word that is not a note with an octave, or is
/// outside the MIDI range. Hosts use it to answer `held(c4)`.
pub fn note_number(word: &str) -> Option<u8> {
    let (pc, oct) = crate::music::parse_note(word)?;
    let midi = 12 * (oct? + 1) + i32::from(pc);
    u8::try_from(midi).ok().filter(|m| *m <= 127)
}

impl Input {
    /// `knob`, `pad`, `fader` or `held`: the first thing a host is asked.
    pub fn kind(&self) -> &'static str {
        match self {
            Input::Knob(_) => "knob",
            Input::Pad(_) => "pad",
            Input::Fader(_) => "fader",
            Input::HeldKey | Input::Held(_) => "held",
        }
    }

    /// What is inside the parentheses: `3`, `5`, `A`, `key`, `c4`.
    pub fn arg(&self) -> String {
        match self {
            Input::Knob(n) | Input::Pad(n) => n.to_string(),
            Input::Fader(d) => d.name().to_string(),
            Input::HeldKey => "key".into(),
            Input::Held(m) => note_name(*m),
        }
    }

    /// What the input reads when nothing answers for it, or when it has not moved: a knob rests in the middle, 1/2 (the value `check`
    /// reads every input as), so a program that opens a filter with a knob does not start shut; a pad, a key and a fader that is not
    /// there read 0.
    pub fn home(&self) -> f64 {
        match self {
            Input::Knob(_) => 0.5,
            _ => 0.0,
        }
    }

    /// The input as the language writes it, and as a performance logs it: `knob(3)`.
    pub fn id(&self) -> String {
        format!("{}({})", self.kind(), self.arg())
    }

    /// An input from its id (`knob(3)`, `fader(A)`, `held(c4)`): what a performance file names. None when it is not one.
    pub fn parse_id(id: &str) -> Option<Input> {
        let (name, rest) = id.split_once('(')?;
        let arg = rest.strip_suffix(')')?;
        match name {
            "knob" | "pad" => {
                let n: u32 = arg.parse().ok().filter(|n| *n >= 1)?;
                Some(if name == "knob" { Input::Knob(n) } else { Input::Pad(n) })
            }
            "fader" => match arg {
                "A" => Some(Input::Fader(Deck::A)),
                "B" => Some(Input::Fader(Deck::B)),
                _ => None,
            },
            "held" => {
                if arg == "key" {
                    Some(Input::HeldKey)
                } else {
                    let m = note_number(arg)?;
                    (note_name(m) == arg).then_some(Input::Held(m))
                }
            }
            _ => None,
        }
    }
}

/// One argument of an input call, as the readers give it.
enum A {
    Num(Rat),
    Word(String),
    Other,
}

fn needs(name: &str) -> &'static str {
    match name {
        "knob" => "a knob number: `knob(3)`",
        "pad" => "a pad number: `pad(5)`",
        "fader" => "a deck: `fader(A)`",
        _ => "`key` or a note: `held(key)`, `held(c4)`",
    }
}

/// An input call, checked: its argument is the right kind of thing and in range for the bank (`from_host` says the range is the
/// connected controllers' own, else it is the default and the help says so).
fn build(name: &str, args: &[A], span: Span, bank: InputBank, from_host: bool) -> Result<Input, Diagnostic> {
    let [arg] = args else {
        return Err(Diagnostic::error(span, format!("`{name}( )` takes one argument: {}", needs(name))).found(format!("{} arguments", args.len())));
    };
    match (name, arg) {
        ("knob" | "pad", A::Num(r)) => {
            let max = if name == "knob" { bank.knobs } else { bank.pads };
            let Some(n) = r.as_int() else {
                return Err(Diagnostic::error(span, format!("a {name} number is a whole number")).found(format!("{r}")).help(format!("{name}(1) to {name}({max})")));
            };
            if (1..=i64::from(max)).contains(&n) {
                let n = n as u32;
                return Ok(if name == "knob" { Input::Knob(n) } else { Input::Pad(n) });
            }
            let why = if n < 1 {
                format!("there is no `{name}({n})`: {name}s are numbered from 1")
            } else {
                format!("there is no `{name}({n})`: a program may use up to {max} {name}s")
            };
            let _ = from_host;
            let help = format!("{name}(1) to {name}({max}); a controller with fewer reads 0 for the rest, and `btw get` says which input nothing answers for");
            Err(Diagnostic::error(span, why).found(format!("{n}")).help(help))
        }
        ("knob" | "pad", _) => Err(Diagnostic::error(span, format!("`{name}( )` needs {}", needs(name))).found("something else")),
        ("fader", A::Word(w)) if w == "A" => Ok(Input::Fader(Deck::A)),
        ("fader", A::Word(w)) if w == "B" => Ok(Input::Fader(Deck::B)),
        ("fader", A::Word(w)) => {
            let mut d = Diagnostic::error(span, format!("`fader({w})`: a fader belongs to deck A or deck B")).expected(&["A", "B"]).found(format!("`{w}`"));
            if w.eq_ignore_ascii_case("a") || w.eq_ignore_ascii_case("b") {
                d = d.help(format!("deck names are upper case: fader({})", w.to_ascii_uppercase()));
            }
            Err(d)
        }
        ("fader", _) => Err(Diagnostic::error(span, "`fader( )` needs a deck: `fader(A)` or `fader(B)`").expected(&["A", "B"])),
        ("held", A::Word(w)) if w == "key" => Ok(Input::HeldKey),
        ("held", A::Word(w)) => match note_number(w) {
            Some(m) => Ok(Input::Held(m)),
            None => {
                let mut d = Diagnostic::error(span, format!("`held({w})`: held() takes `key` or a note with its octave")).expected(&["key", "a note like c4"]).found(format!("`{w}`"));
                if crate::music::parse_note(w).is_some() {
                    d = d.help(format!("a note needs an octave: held({w}4)"));
                } else {
                    d = d.help("held(key) is the lowest key held; held(c4) is 1 while middle C is held");
                }
                Err(d)
            }
        },
        _ => Err(Diagnostic::error(span, "`held( )` needs `key` or a note: `held(key)`, `held(c4)`").expected(&["key", "a note like c4"])),
    }
}

/// The pad a `launch` names (`launch drop on pad(2)`), checked as `pad(N)` is anywhere: a whole number from 1 to 128, the same on every machine.
pub fn pad_number(n: u32, span: Span) -> Result<u32, Diagnostic> {
    match build("pad", &[A::Num(Rat::int(i64::from(n)))], span, InputBank::MAX, false)? {
        Input::Pad(n) => Ok(n),
        _ => Ok(n),
    }
}

// ---- where an input may be read (the parser's side)

#[derive(Clone, Copy)]
struct Ctx {
    /// A control value is being read: an input may stand.
    allowed: bool,
    bank: Option<InputBank>,
}

thread_local! {
    static CTX: Cell<Ctx> = const { Cell::new(Ctx { allowed: false, bank: None }) };
}

struct Restore(Ctx);

impl Drop for Restore {
    fn drop(&mut self) {
        CTX.with(|c| c.set(self.0));
    }
}

fn ctx() -> Ctx {
    CTX.with(Cell::get)
}

/// Run `f` (a parse) with `bank` as the connected controllers' size. It is host data a parse carries (`Env::bank`) and does not check
/// against: `knob(N)` and `pad(N)` are checked against [`InputBank::MAX`] on every machine (R-06).
pub fn with_bank<T>(bank: Option<InputBank>, f: impl FnOnce() -> T) -> T {
    let old = ctx();
    CTX.with(|c| c.set(Ctx { bank, ..old }));
    let _restore = Restore(old);
    f()
}

/// Run `f` (the reading of a control value) where an input may stand.
pub fn allowing<T>(f: impl FnOnce() -> T) -> T {
    let old = ctx();
    CTX.with(|c| c.set(Ctx { allowed: true, ..old }));
    let _restore = Restore(old);
    f()
}

/// The range an input may name when a program is checked: the same on every machine ([`InputBank::MAX`]), whatever `with_bank` was given.
fn bank_now() -> (InputBank, bool) {
    (InputBank::MAX, false)
}

/// The error for an input where only a number known when the program is read can stand.
pub fn not_here(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(span, format!("`{name}(...)` is a performed input: it is read while the program plays, so it cannot stand here")).help(
        "an input stands in a control value (`master: filter lp 200 + knob(3) * 7800`) or in a control line's `when` (`when held(key)`); a pattern is compiled once per section, and a count, a time, a tempo or an `evolve` rule's number is fixed when the program is read: drive a control line from the input instead (`deck A: level fader(B)`)",
    )
}

/// The error for an input where a number is wanted when the program is read, said in terms of what was wanted there
/// (`expected a tempo in BPM: ...`).
pub fn in_place_of(d: Diagnostic, what: &str) -> Diagnostic {
    if d.message.contains("is a performed input") {
        return Diagnostic { message: format!("expected {what}: {}", d.message), ..d };
    }
    d
}

/// The evaluator the parser uses for a number's arithmetic: an input reads 1/2 (the number the rest of the language sees) where
/// a control value is being read, and is an error anywhere else.
pub struct ParseHooks;

impl Hooks for ParseHooks {
    fn call(&self, name: &str, args: &[Val], span: Span) -> Option<Result<Val, Diagnostic>> {
        if !is_input_word(name) {
            return None;
        }
        if !ctx().allowed {
            return Some(Err(not_here(name, span)));
        }
        let a: Vec<A> = args
            .iter()
            .map(|v| match v {
                Val::Num(r) => A::Num(*r),
                Val::Word(w) => A::Word(w.clone()),
                Val::Bool(_) => A::Other,
            })
            .collect();
        let (bank, from_host) = bank_now();
        Some(build(name, &a, span, bank, from_host).map(|_| Val::Num(Rat::new(1, 2).unwrap_or(Rat::ONE))))
    }
}

/// Whether the canonical text of an expression (a [`Num`]'s `canon`) has an input in it.
pub fn text_has_input(text: &str) -> bool {
    INPUT_WORDS.iter().any(|w| {
        let pat = format!("{w}(");
        text.match_indices(&pat).any(|(i, _)| i == 0 || !text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '#'))
    })
}

/// A number that came from an expression with an input in it: its value is the placeholder, its canonical text the truth.
pub fn is_live(n: &Num) -> bool {
    n.canon.is_some_and(text_has_input)
}

/// A def's argument with each input call (`knob ( 3 )`, `held ( c4 )`) taken for one number, so that the arithmetic round it
/// (`knob(3) * 2`) is seen as arithmetic and keeps its grouping where it is put in place of a `$name`.
pub fn collapse_inputs(toks: &[crate::lex::Token]) -> std::borrow::Cow<'_, [crate::lex::Token]> {
    use crate::lex::Tok;
    if !toks.iter().any(|t| matches!(&t.tok, Tok::Word(w) if is_input_word(w))) {
        return std::borrow::Cow::Borrowed(toks);
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if matches!(&toks[i].tok, Tok::Word(w) if is_input_word(w)) && toks.get(i + 1).is_some_and(|t| t.tok == Tok::LParen) {
            // up to the matching parenthesis
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < toks.len() {
                match toks[j].tok {
                    Tok::LParen => depth += 1,
                    Tok::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j < toks.len() {
                out.push(crate::lex::Token { tok: Tok::Num { value: 0.5, int: false }, span: toks[i].span.to(toks[j].span) });
                i = j + 1;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

// ---- the expression an input stands in

/// Whether an expression has an input call in it.
pub fn mentions_input(e: &Expr) -> bool {
    match e {
        Expr::Call(n, args, _) => is_input_word(n) || args.iter().any(mentions_input),
        Expr::Neg(x, _) | Expr::Not(x, _) => mentions_input(x),
        Expr::Bin(_, a, b, _) | Expr::Cmp(_, a, b, _) | Expr::And(a, b, _) | Expr::Or(a, b, _) => mentions_input(a) || mentions_input(b),
        Expr::Num(..) | Expr::Word(..) | Expr::Unbound(..) => false,
    }
}

/// An input call's arguments as the canonical text prints them: the note of `held(C4)` is spelled one way (`c4`).
pub fn canonical_args(name: &str, args: Vec<Expr>) -> Vec<Expr> {
    if name != "held" {
        return args;
    }
    args.into_iter()
        .map(|a| match a {
            Expr::Word(w, sp) => match note_number(&w) {
                Some(m) => Expr::Word(note_name(m), sp),
                None => Expr::Word(w, sp),
            },
            other => other,
        })
        .collect()
}

/// The performed expression: arithmetic and conditions over `f64`, with the inputs as leaves, evaluated each time its line is
/// due. A condition is 1 or 0, and a number is a condition by being non-zero. Total: a division by zero reads 0.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveExpr {
    Const(f64),
    Input(Input),
    Neg(Box<LiveExpr>),
    Bin(BinOp, Box<LiveExpr>, Box<LiveExpr>),
    Cmp(CmpKind, Box<LiveExpr>, Box<LiveExpr>),
    Not(Box<LiveExpr>),
    And(Box<LiveExpr>, Box<LiveExpr>),
    Or(Box<LiveExpr>, Box<LiveExpr>),
    Min(Vec<LiveExpr>),
    Max(Vec<LiveExpr>),
}

/// Two quantised values are equal within this.
const SAME: f64 = 1e-9;

impl LiveExpr {
    /// The value, reading each input through `read`. Both sides of `and` / `or` and every argument of `min` / `max` are read,
    /// so the inputs a line names are the inputs it reads, whatever their values.
    pub fn eval(&self, read: &mut dyn FnMut(&Input) -> f64) -> f64 {
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        match self {
            LiveExpr::Const(c) => *c,
            LiveExpr::Input(i) => read(i),
            LiveExpr::Neg(x) => -x.eval(read),
            LiveExpr::Bin(op, x, y) => {
                let (p, q) = (x.eval(read), y.eval(read));
                match op {
                    BinOp::Add => p + q,
                    BinOp::Sub => p - q,
                    BinOp::Mul => p * q,
                    BinOp::Div => {
                        if q == 0.0 {
                            0.0
                        } else {
                            p / q
                        }
                    }
                    BinOp::Rem => {
                        if q == 0.0 {
                            0.0
                        } else {
                            p.rem_euclid(q)
                        }
                    }
                }
            }
            LiveExpr::Cmp(k, x, y) => {
                let (p, q) = (x.eval(read), y.eval(read));
                b(match k {
                    CmpKind::Eq => (p - q).abs() < SAME,
                    CmpKind::Ne => (p - q).abs() >= SAME,
                    CmpKind::Lt => p < q - SAME,
                    CmpKind::Le => p <= q + SAME,
                    CmpKind::Gt => p > q + SAME,
                    CmpKind::Ge => p >= q - SAME,
                })
            }
            LiveExpr::Not(x) => b(x.eval(read) == 0.0),
            LiveExpr::And(x, y) => {
                let (p, q) = (x.eval(read), y.eval(read));
                b(p != 0.0 && q != 0.0)
            }
            LiveExpr::Or(x, y) => {
                let (p, q) = (x.eval(read), y.eval(read));
                b(p != 0.0 || q != 0.0)
            }
            LiveExpr::Min(v) => v.iter().map(|x| x.eval(read)).fold(f64::INFINITY, f64::min),
            LiveExpr::Max(v) => v.iter().map(|x| x.eval(read)).fold(f64::NEG_INFINITY, f64::max),
        }
    }

    /// Every input it reads.
    pub fn inputs(&self, out: &mut BTreeSet<Input>) {
        match self {
            LiveExpr::Const(_) => {}
            LiveExpr::Input(i) => {
                out.insert(i.clone());
            }
            LiveExpr::Neg(x) | LiveExpr::Not(x) => x.inputs(out),
            LiveExpr::Bin(_, x, y) | LiveExpr::Cmp(_, x, y) | LiveExpr::And(x, y) | LiveExpr::Or(x, y) => {
                x.inputs(out);
                y.inputs(out);
            }
            LiveExpr::Min(v) | LiveExpr::Max(v) => v.iter().for_each(|x| x.inputs(out)),
        }
    }

    /// The least and the most the expression can be, each input being somewhere in 0..1 (interval arithmetic). A division by a
    /// range that holds 0 is unbounded.
    pub fn bounds(&self) -> (f64, f64) {
        fn corners(a: (f64, f64), b: (f64, f64), f: impl Fn(f64, f64) -> f64) -> (f64, f64) {
            let c = [f(a.0, b.0), f(a.0, b.1), f(a.1, b.0), f(a.1, b.1)];
            (c.iter().copied().fold(f64::INFINITY, f64::min), c.iter().copied().fold(f64::NEG_INFINITY, f64::max))
        }
        match self {
            LiveExpr::Const(c) => (*c, *c),
            LiveExpr::Input(_) => (0.0, 1.0),
            LiveExpr::Neg(x) => {
                let (lo, hi) = x.bounds();
                (-hi, -lo)
            }
            LiveExpr::Bin(op, x, y) => {
                let (p, q) = (x.bounds(), y.bounds());
                match op {
                    BinOp::Add => (p.0 + q.0, p.1 + q.1),
                    BinOp::Sub => (p.0 - q.1, p.1 - q.0),
                    BinOp::Mul => corners(p, q, |a, b| a * b),
                    BinOp::Div => {
                        if q.0 <= 0.0 && q.1 >= 0.0 {
                            (f64::NEG_INFINITY, f64::INFINITY)
                        } else {
                            corners(p, q, |a, b| a / b)
                        }
                    }
                    BinOp::Rem => (0.0, q.0.abs().max(q.1.abs())),
                }
            }
            LiveExpr::Cmp(..) | LiveExpr::Not(_) | LiveExpr::And(..) | LiveExpr::Or(..) => (0.0, 1.0),
            LiveExpr::Min(v) => v.iter().map(LiveExpr::bounds).fold((f64::INFINITY, f64::INFINITY), |a, b| (a.0.min(b.0), a.1.min(b.1))),
            LiveExpr::Max(v) => v.iter().map(LiveExpr::bounds).fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |a, b| (a.0.max(b.0), a.1.max(b.1))),
        }
    }

    /// Whether a division in it can have a zero divisor.
    pub fn may_divide_by_zero(&self) -> bool {
        match self {
            LiveExpr::Const(_) | LiveExpr::Input(_) => false,
            LiveExpr::Neg(x) | LiveExpr::Not(x) => x.may_divide_by_zero(),
            LiveExpr::Bin(op, x, y) => {
                let q = y.bounds();
                (matches!(op, BinOp::Div | BinOp::Rem) && q.0 <= 0.0 && q.1 >= 0.0) || x.may_divide_by_zero() || y.may_divide_by_zero()
            }
            LiveExpr::Cmp(_, x, y) | LiveExpr::And(x, y) | LiveExpr::Or(x, y) => x.may_divide_by_zero() || y.may_divide_by_zero(),
            LiveExpr::Min(v) | LiveExpr::Max(v) => v.iter().any(LiveExpr::may_divide_by_zero),
        }
    }
}

/// An expression with an input in it, as a [`LiveExpr`]. What has no input is folded now (`$i % 2 == 0` is 1 or 0 when the
/// line is expanded), the rest stays to be read while the program plays.
pub fn lower_expr(e: &Expr, _host_bank: Option<InputBank>) -> Result<LiveExpr, Diagnostic> {
    // (the host's bank is not consulted: whether a program checks does not depend on the machine, R-06)
    lower(e, InputBank::MAX, false)
}

fn fold(e: &Expr) -> Result<LiveExpr, Diagnostic> {
    match crate::expr::eval(e, &NoHooks)? {
        Val::Num(r) => Ok(LiveExpr::Const(r.to_f64())),
        Val::Bool(b) => Ok(LiveExpr::Const(if b { 1.0 } else { 0.0 })),
        Val::Word(w) => Err(Diagnostic::error(e.span(), format!("the word `{w}` is not a number")).help("a word is compared with `==` or `!=` to another word; an input is a number")),
    }
}

/// An input call's arguments as the checker reads them: a number or a word, whatever arithmetic wrote it (`knob(0 + 1)`).
fn args_of(args: &[Expr]) -> Vec<A> {
    args.iter()
        .map(|x| match crate::expr::eval(x, &NoHooks) {
            Ok(Val::Num(r)) => A::Num(r),
            Ok(Val::Word(w)) => A::Word(w),
            _ => A::Other,
        })
        .collect()
}

fn lower(e: &Expr, bank: InputBank, from_host: bool) -> Result<LiveExpr, Diagnostic> {
    if !mentions_input(e) {
        return fold(e);
    }
    let sub = |x: &Expr| lower(x, bank, from_host).map(Box::new);
    Ok(match e {
        Expr::Call(name, args, sp) if is_input_word(name) => LiveExpr::Input(build(name, &args_of(args), *sp, bank, from_host)?),
        Expr::Call(name, args, sp) if name == "min" || name == "max" => {
            if args.is_empty() {
                return Err(Diagnostic::error(*sp, format!("`{name}` needs at least one number")));
            }
            let v = args.iter().map(|x| lower(x, bank, from_host)).collect::<Result<Vec<_>, _>>()?;
            if name == "min" {
                LiveExpr::Min(v)
            } else {
                LiveExpr::Max(v)
            }
        }
        Expr::Call(name, _, sp) => {
            return Err(Diagnostic::error(*sp, format!("`{name}(...)` is not a function of arithmetic"))
                .help("`min(a, b)` and `max(a, b)` are the functions of the language; `knob(3)`, `pad(5)`, `fader(A)` and `held(key)` are the inputs"))
        }
        Expr::Neg(x, _) => LiveExpr::Neg(sub(x)?),
        Expr::Not(x, _) => LiveExpr::Not(sub(x)?),
        Expr::Bin(op, x, y, _) => LiveExpr::Bin(*op, sub(x)?, sub(y)?),
        Expr::Cmp(k, x, y, _) => LiveExpr::Cmp(*k, sub(x)?, sub(y)?),
        Expr::And(x, y, _) => LiveExpr::And(sub(x)?, sub(y)?),
        Expr::Or(x, y, _) => LiveExpr::Or(sub(x)?, sub(y)?),
        Expr::Num(..) | Expr::Word(..) | Expr::Unbound(..) => return fold(e),
    })
}

/// A guard that names an input, read where it was written: every input call in it is checked (its argument, its range) so the
/// error points at the call.
pub fn check_guard(e: &Expr) -> Result<(), Diagnostic> {
    fn walk(e: &Expr, bank: InputBank, from_host: bool) -> Result<(), Diagnostic> {
        match e {
            Expr::Call(name, args, sp) => {
                if is_input_word(name) {
                    build(name, &args_of(args), *sp, bank, from_host)?;
                }
                args.iter().try_for_each(|x| walk(x, bank, from_host))
            }
            Expr::Neg(x, _) | Expr::Not(x, _) => walk(x, bank, from_host),
            Expr::Bin(_, a, b, _) | Expr::Cmp(_, a, b, _) | Expr::And(a, b, _) | Expr::Or(a, b, _) => {
                walk(a, bank, from_host)?;
                walk(b, bank, from_host)
            }
            Expr::Num(..) | Expr::Word(..) | Expr::Unbound(..) => Ok(()),
        }
    }
    let (bank, from_host) = bank_now();
    walk(e, bank, from_host)
}

/// A `let` whose value has an input in it: a `let` is put in place while the program is read, an input is read while it plays.
pub fn let_with_input(name: &crate::ast::Word, value: &[crate::lex::Token]) -> Option<Diagnostic> {
    use crate::lex::Tok;
    let at = value.iter().position(|t| matches!(&t.tok, Tok::Word(w) if is_input_word(w)) )?;
    value.get(at + 1).filter(|t| t.tok == Tok::LParen)?;
    Some(
        Diagnostic::error(value[at].span, format!("`let {}` cannot hold an input: a `let` is put in place while the program is read, and an input is read while it plays", name.text))
            .help("write the expression where it is used, or pass it to a `def` as an argument: `sweep(200 + knob(3) * 7800)`"),
    )
}

/// An input in an `evolve` rule: a generation is a function of the rule and its number alone (`btw generation` finds any
/// generation's CID from the program), which a number read from a controller would break.
pub fn in_evolve(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(span, format!("`{name}(...)` cannot stand in an `evolve` rule: every generation is made from generation 0 and the rule alone, and `btw generation` says any generation's CID from the program, which a value read from a controller would break"))
        .help("drive a control line from the input instead (`master: filter lp 200 + knob(3) * 7800`), or write the rule's numbers out: `rotate <0 1 2>`")
}

/// A line whose guard names an input but is not a control line: said at its `when`.
pub fn misplaced_guard(l: &Line) -> Option<Diagnostic> {
    let g = l.guard.as_ref()?;
    match &l.body {
        Body::Parts(_) => Some(
            Diagnostic::error(g.span, "an input cannot decide whether a part plays: a pattern is compiled once per section, and an input is read while the program plays")
                .help("put the input on a control line (`deck A: level fader(B)`, `master: filter lp 300 when held(key)`), or write the choice in the pattern (`<x ~>`, `x?0.5`)"),
        ),
        Body::Invoke(_) => Some(Diagnostic::error(g.span, "a call of a `def` takes no performed guard").help("put `when held(key)` on the control lines inside the def, where the input is read when each line is due")),
        _ => None,
    }
}

// ---- what a performed control is

/// Where a performed value goes in the compiled effect.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    /// A param of one of an effect's calls (or, in a ramp, of its base: the ramp's other params).
    Field { call: usize, key: String },
    /// A ramp's start.
    RampFrom,
    /// A ramp's end.
    RampTo,
}

/// How a value is turned into the number the call carries.
#[derive(Clone, Debug, PartialEq)]
pub enum Xform {
    /// As it is.
    Identity,
    /// A cutoff in hertz as the filter's own knob (`compile::sweep_knob`).
    Sweep { hp: bool },
    /// A place 0..1 along the filter's range as the filter's own knob (`compile::sweep_span`): an input written alone.
    Span { hp: bool },
}

/// One performed value: the expression, where it goes and how it is shaped.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    pub slot: Slot,
    pub expr: LiveExpr,
    /// The expression as the canonical text prints it, for the words.
    pub text: String,
    pub xform: Xform,
    /// The range the value is held to (a level is 0..1, a low-pass cutoff 60 Hz..20 kHz); infinite for a `do` param.
    pub lo: f64,
    pub hi: f64,
    /// The registry wants a whole number.
    pub whole: bool,
}

impl Patch {
    /// The number the call carries for the expression's value `v`.
    pub fn shape(&self, v: f64) -> f64 {
        let v = if v.is_nan() { self.lo.max(0.0).min(self.hi) } else { v.clamp(self.lo, self.hi) };
        let v = match self.xform {
            Xform::Identity => v,
            Xform::Sweep { hp } => sweep_knob(v, hp),
            Xform::Span { hp } => sweep_span(v, hp),
        };
        if self.whole {
            v.round()
        } else {
            v
        }
    }
}

/// A value as the call carries it: a whole number as an integer, else as it is (what `docall` and a plain `change` send).
fn number(v: f64) -> JsonValue {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        JsonValue::int(v as i64)
    } else {
        JsonValue::Number(v)
    }
}

/// Where a performed value line follows its input (R1): the bars of its song, or of its program, it is due on. A value line with no
/// time word does not fire once: it fires on every bar from its first, and sends a call only when the value it reads has moved. In a
/// section that is each bar of it, and it stops with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Follow {
    /// The first bar (a section's first, or 0 at the top of the program).
    pub from: u32,
    /// The bar it is no longer due on (its section's end): None runs as long as the program does.
    pub until: Option<u32>,
}

/// A control line that is performed: its effect as compiled (every input read 1/2), a guard, and the values to put in when it
/// fires.
#[derive(Clone, Debug, PartialEq)]
pub struct Performed {
    pub base: Effect,
    /// `when held(key)`: the line fires only if this is not 0 when it is due.
    pub guard: Option<LiveExpr>,
    /// The guard as written, for the words.
    pub guard_text: String,
    pub patches: Vec<Patch>,
    /// An lfo whose `center=` or `depth=` names an input: its middle and reach are read again at every bar line.
    pub lfo: Option<LfoLive>,
    /// Every input the line reads, in order.
    pub inputs: Vec<Input>,
    /// A plain value with no time word follows its input every bar (R1); None for a line that fires when its time word says.
    pub follow: Option<Follow>,
}

/// A performed lfo's `center=` and `depth=`, in the line's own unit (hertz for a cutoff), and how they become the knob the call
/// carries: the swing is `center - depth` to `center + depth`, held to the range, and each end is turned into the call's unit
/// the way a ramp's ends are (`Xform`), so a cutoff lfo is centred on the knob's own middle of the two.
#[derive(Clone, Debug, PartialEq)]
pub struct LfoLive {
    pub center: LiveExpr,
    pub depth: LiveExpr,
    pub xform: Xform,
    pub lo: f64,
    pub hi: f64,
}

impl LfoLive {
    /// The knob-unit middle and reach for the values `read` gives.
    pub fn knobs(&self, read: &mut dyn FnMut(&Input) -> f64) -> (f64, f64) {
        let c = self.center.eval(read);
        let d = self.depth.eval(read).max(0.0);
        let hold = |v: f64| if v.is_nan() { self.lo.max(0.0).min(self.hi) } else { v.clamp(self.lo, self.hi) };
        let to_knob = |v: f64| match self.xform {
            Xform::Identity => v,
            Xform::Sweep { hp } => sweep_knob(v, hp),
            Xform::Span { hp } => sweep_span(v, hp),
        };
        let (a, b) = (to_knob(hold(c - d)), to_knob(hold(c + d)));
        ((a + b) / 2.0, (b - a) / 2.0)
    }
}

impl Performed {
    /// The effect with every input reading 1/2 (the value the compiler checks a performed line with), or None when its guard is
    /// false then: what an offline render, which has no controllers, does with a performed line (docs/beatscript/ext-perform.md).
    pub fn at_half(&self) -> Option<Effect> {
        let mut half = |_: &Input| 0.5;
        if !self.guard_holds(&mut half) {
            return None;
        }
        Some(self.apply(&mut half))
    }

    /// Whether the line may fire: no guard, or a guard that is not 0.
    pub fn guard_holds(&self, read: &mut dyn FnMut(&Input) -> f64) -> bool {
        self.guard.as_ref().is_none_or(|g| g.eval(read) != 0.0)
    }

    /// The effect with every performed value read now and put in place.
    pub fn apply(&self, read: &mut dyn FnMut(&Input) -> f64) -> Effect {
        let mut effect = self.base.clone();
        for p in &self.patches {
            let v = p.shape(p.expr.eval(read));
            match (&mut effect, &p.slot) {
                (Effect::Calls(cs), Slot::Field { call, key }) => {
                    if let Some(c) = cs.get_mut(*call) {
                        let mut o = c.params.as_object().cloned().unwrap_or_default();
                        o.set(key.as_str(), number(v));
                        c.params = JsonValue::Object(o);
                    }
                }
                (Effect::Ramp(r), Slot::Field { key, .. }) => {
                    let mut o = r.base.as_object().cloned().unwrap_or_default();
                    o.set(key.as_str(), number(v));
                    r.base = JsonValue::Object(o);
                }
                (Effect::Ramp(r), Slot::RampFrom) => r.from = v,
                (Effect::Ramp(r), Slot::RampTo) => r.to = v,
                _ => {}
            }
        }
        if let (Some(lv), Effect::Lfo(l)) = (&self.lfo, &mut effect) {
            let (center, depth) = lv.knobs(read);
            l.center = center;
            l.depth = depth;
        }
        effect
    }
}

/// The range a control's value is held to, the field it is sent as and how it is shaped: `(field, xform, lo, hi)`.
fn change_shape(name: &str, mode: Option<&str>, target: Option<Target>) -> Option<(&'static str, Xform, f64, f64)> {
    Some(match (name, mode) {
        ("filter", Some("lp")) => ("value", Xform::Sweep { hp: false }, 60.0, 20000.0),
        ("filter", Some("hp")) => ("value", Xform::Sweep { hp: true }, 20.0, 8000.0),
        ("delay" | "reverb" | "crush" | "resonance" | "drive", _) => ("value", Xform::Identity, 0.0, 1.0),
        ("level", _) => (if matches!(target, Some(Target::Master)) { "master" } else { "volume" }, Xform::Identity, 0.0, 1.0),
        ("low", _) => ("low", Xform::Identity, 0.0, 1.0),
        ("mid", _) => ("mid", Xform::Identity, 0.0, 1.0),
        ("high", _) => ("high", Xform::Identity, 0.0, 1.0),
        ("crossfader", _) => ("crossfader", Xform::Identity, 0.0, 1.0),
        _ => return None,
    })
}

/// Whether an lfo's `center=` or `depth=` names an input.
fn lfo_has_live(l: &LfoSpec) -> bool {
    l.center.as_ref().is_some_and(is_live) || l.depth.as_ref().is_some_and(is_live)
}

fn list_has_live(v: &DoValue) -> bool {
    match v {
        DoValue::Num(n) => is_live(n),
        DoValue::Ramp(a, b) => is_live(a) || is_live(b),
        DoValue::List(items, _) => items.iter().any(list_has_live),
        _ => false,
    }
}

fn fmt_f(v: f64) -> String {
    if v.is_infinite() {
        return if v < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let r = (v * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

impl Compiler<'_> {
    /// The performed part of a compiled control line: a value with an input in it, and a guard. `effect` is the line compiled
    /// with every input read 1/2. Returns it unchanged for a line with neither, wrapped in [`Effect::Performed`] otherwise,
    /// and None (the error said) when the performed value cannot go where it was put.
    pub(crate) fn performed(&mut self, fl: &FlatLine, effect: Effect) -> Option<Effect> {
        let l = &fl.line;
        let mut patches: Vec<Patch> = Vec::new();
        let mut lfo: Option<LfoLive> = None;
        let text_of = |n: &Num| n.canon.unwrap_or("").to_string();
        // a plain value, a ramp or a shaped ramp (`0 -> knob(1) over 4 bars exp`): the ends are the values
        let change = match &l.body {
            Body::Action(Action::Change { name, mode, from, to, .. }) => Some((name, mode, *from, *to)),
            Body::Action(Action::Shaped { name, mode, motion: Motion::Ramp { from, to, .. } }) => Some((name, mode, Some(*from), Some(*to))),
            _ => None,
        };
        match &l.body {
            Body::Action(Action::Shaped { name, mode, motion: Motion::Lfo(spec) }) if lfo_has_live(spec) => {
                let Some((_, xform, lo, hi)) = change_shape(&name.text, mode.as_ref().map(|m| m.text.as_str()), l.target.map(|(t, _)| t)) else {
                    self.err(Diagnostic::error(name.span, format!("`{}` takes no performed value", name.text)));
                    return None;
                };
                lfo = Some(self.lfo_live(spec, &name.text, xform, lo, hi)?);
            }
            _ if change.is_some() => {
                let (name, mode, from, to) = change.unwrap();
                let live = |n: &Option<Num>| n.as_ref().is_some_and(is_live);
                if live(&from) || live(&to) {
                    let shape = change_shape(&name.text, mode.as_ref().map(|m| m.text.as_str()), l.target.map(|(t, _)| t));
                    let Some((field, xform, lo, hi)) = shape else {
                        self.err(Diagnostic::error(name.span, format!("`{}` takes no performed value", name.text)));
                        return None;
                    };
                    for (n, is_from) in [(from.as_ref(), true), (to.as_ref(), false)] {
                        let Some(n) = n.filter(|n| is_live(n)) else { continue };
                        let expr = self.live_number(n)?;
                        let slot = match (&effect, is_from) {
                            (Effect::Ramp(_), true) => Slot::RampFrom,
                            (Effect::Ramp(_), false) => Slot::RampTo,
                            (Effect::Calls(cs), _) => Slot::Field { call: cs.len().saturating_sub(1), key: field.to_string() },
                            _ => return None,
                        };
                        // an input written alone as a cutoff covers the filter's range, on the filter's own curve (R2): `filter lp knob(1)`
                        if let (LiveExpr::Input(_), Xform::Sweep { hp }) = (&expr, &xform) {
                            patches.push(Patch { slot, expr, text: text_of(n), xform: Xform::Span { hp: *hp }, lo: 0.0, hi: 1.0, whole: false });
                            continue;
                        }
                        let (elo, ehi) = expr.bounds();
                        if ehi < lo || elo > hi {
                            self.err(
                                Diagnostic::error(n.span, format!("this value is always outside the range of `{}`: it is {} to {} and the range is {} to {}", name.text, fmt_f(elo), fmt_f(ehi), fmt_f(lo), fmt_f(hi)))
                                    .found(text_of(n))
                                    .help(if matches!(xform, Xform::Sweep { .. }) {
                                        "an input is 0 to 1: scale it into the range (`200 + knob(3) * 7800`), or write the input alone to cover the range (`filter lp knob(3)`)"
                                    } else {
                                        "an input is 0 to 1: scale it into the range (`200 + knob(3) * 7800`)"
                                    }),
                            );
                            return None;
                        }
                        if elo < lo || ehi > hi {
                            self.warn_note(n.span, format!("{} can be {} to {}: it is held to {} to {}", text_of(n), fmt_f(elo), fmt_f(ehi), fmt_f(lo), fmt_f(hi)));
                        }
                        patches.push(Patch { slot, expr, text: text_of(n), xform: xform.clone(), lo, hi, whole: false });
                    }
                }
            }
            Body::Do(d) => {
                for p in &d.params {
                    let (first, second) = match &p.value {
                        DoValue::Num(n) => (Some((n, 0)), None),
                        DoValue::Ramp(a, b) => (Some((a, 1)), Some((b, 2))),
                        DoValue::Lfo(spec) => {
                            if lfo_has_live(spec) {
                                lfo = Some(self.lfo_live(spec, &p.name.text, Xform::Identity, f64::NEG_INFINITY, f64::INFINITY)?);
                            }
                            continue;
                        }
                        DoValue::List(items, sp) => {
                            if items.iter().any(list_has_live) {
                                self.err(Diagnostic::error(*sp, format!("`{}` is a list, and a performed value cannot be one of its items", p.name.text)).help("an input stands as a param's whole value: `value=knob(2)`, `mix=0 -> knob(2)`"));
                                return None;
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    for (n, which) in [first, second].into_iter().flatten() {
                        if !is_live(n) {
                            continue;
                        }
                        if !self.param_is_numeric(&effect, &p.name.text) {
                            self.err(Diagnostic::error(n.span, format!("`{}` is not a number, so a performed value cannot go there", p.name.text)).help("an input is a number from 0 to 1"));
                            return None;
                        }
                        let whole = self.param_wants_whole(&effect, &p.name.text);
                        let expr = self.live_number(n)?;
                        let slot = match which {
                            0 => Slot::Field { call: 0, key: p.name.text.clone() },
                            1 => Slot::RampFrom,
                            _ => Slot::RampTo,
                        };
                        patches.push(Patch { slot, expr, text: text_of(n), xform: Xform::Identity, lo: f64::NEG_INFINITY, hi: f64::INFINITY, whole });
                    }
                }
            }
            _ => {}
        }
        let guard = match &l.guard {
            Some(g) => match lower_expr(&g.expr, self.env.bank) {
                Ok(e) => {
                    if e.may_divide_by_zero() {
                        self.warn_note(g.span, "this guard can divide by zero: it reads 0 then".into());
                    }
                    Some((e, g.expr.canonical()))
                }
                Err(d) => {
                    self.err(d);
                    return None;
                }
            },
            None => None,
        };
        if patches.is_empty() && guard.is_none() && lfo.is_none() {
            return Some(effect);
        }
        let mut set = BTreeSet::new();
        patches.iter().for_each(|p| p.expr.inputs(&mut set));
        if let Some(lv) = &lfo {
            lv.center.inputs(&mut set);
            lv.depth.inputs(&mut set);
        }
        let (guard, guard_text) = match guard {
            Some((g, t)) => {
                g.inputs(&mut set);
                (Some(g), t)
            }
            None => (None, String::new()),
        };
        // a plain value (not a ramp or an lfo, which re-read their inputs at every bar line already) on a line with no time word
        // follows its input; a line whose only input is a `when` (no value to follow) and a `for N bars` line fire once, as they did
        let follow = (fl.untimed && !patches.is_empty() && lfo.is_none() && matches!(effect, Effect::Calls(_)))
            .then(|| match fl.timing {
                Timing::Now => Some(Follow { from: 0, until: None }),
                Timing::At(b) => Some(Follow { from: b, until: self.seg_end }),
                _ => None,
            })
            .flatten();
        Some(Effect::Performed(Box::new(Performed { base: effect, guard, guard_text, patches, lfo, inputs: set.into_iter().collect(), follow })))
    }

    /// The performed `center=` and `depth=` of an lfo, checked against what the line can be: `lo`..`hi` is the range of the
    /// thing it moves (infinite for a `do` param, whose own check is the registry's).
    fn lfo_live(&mut self, spec: &LfoSpec, name: &str, xform: Xform, lo: f64, hi: f64) -> Option<LfoLive> {
        let part = |me: &mut Self, n: Option<Num>, default: f64| -> Option<LiveExpr> {
            match n {
                Some(n) if is_live(&n) => me.live_number(&n),
                Some(n) => Some(LiveExpr::Const(n.value)),
                None => Some(LiveExpr::Const(default)),
            }
        };
        let center = part(self, spec.center, 0.0)?;
        let depth = part(self, spec.depth, 1.0)?;
        let (c, d) = (center.bounds(), depth.bounds());
        // the lowest and the highest the swing can reach (a depth is never negative)
        let (reach_lo, reach_hi) = (c.0 - d.1.max(0.0), c.1 + d.1.max(0.0));
        let at = spec.center.filter(is_live).or(spec.depth.filter(is_live)).map_or(spec.span, |n| n.span);
        if lo.is_finite() && hi.is_finite() {
            if reach_hi < lo || reach_lo > hi {
                self.err(
                    Diagnostic::error(at, format!("this lfo is always outside the range of `{name}`: it swings {} to {} at most and the range is {} to {}", fmt_f(reach_lo), fmt_f(reach_hi), fmt_f(lo), fmt_f(hi)))
                        .help("an input is 0 to 1: scale it into the range (`center=200 + knob(3) * 7800`)"),
                );
                return None;
            }
            if reach_lo < lo || reach_hi > hi {
                self.warn_note(at, format!("this lfo can swing from {} to {}: it is held to {} to {}", fmt_f(reach_lo), fmt_f(reach_hi), fmt_f(lo), fmt_f(hi)));
            }
        }
        Some(LfoLive { center, depth, xform, lo, hi })
    }

    /// A performed number's expression, read back from its canonical text.
    fn live_number(&mut self, n: &Num) -> Option<LiveExpr> {
        let text = n.canon.unwrap_or("");
        let lexed = crate::lex::lex(text);
        let mut c = crate::expr::Cursor { toks: &lexed.tokens, at: 0 };
        let parsed = crate::expr::parse_expr(&mut c).and_then(|e| lower_expr(&e, self.env.bank));
        match parsed {
            Ok(e) => {
                if e.may_divide_by_zero() {
                    self.warn_note(n.span, format!("{text} can divide by zero: it reads 0 then"));
                }
                Some(e)
            }
            Err(d) => {
                // the same words, said where the number was written
                self.err(Diagnostic { span: n.span, ..d });
                None
            }
        }
    }

    fn registry_param(&self, effect: &Effect, key: &str) -> Option<crate::env::ParamSpec> {
        let noun = match effect {
            Effect::Calls(cs) => cs.first().map(|c| c.noun.clone()),
            Effect::Ramp(r) => Some(r.noun.clone()),
            _ => None,
        }?;
        self.env.registry?.noun(&noun)?.param(key).cloned()
    }

    fn param_wants_whole(&self, effect: &Effect, key: &str) -> bool {
        self.registry_param(effect, key).is_some_and(|s| !s.types.iter().any(|t| t == "number" || t == "any") && s.types.iter().any(|t| t == "integer"))
    }

    fn param_is_numeric(&self, effect: &Effect, key: &str) -> bool {
        self.registry_param(effect, key).is_none_or(|s| s.types.is_empty() || s.types.iter().any(|t| t == "number" || t == "integer" || t == "any"))
    }
}

/// Every input the compiled controls read, sorted, each once.
pub fn performed_inputs(controls: &[crate::compile::Control]) -> Vec<Input> {
    let mut set = BTreeSet::new();
    for c in controls {
        if let Effect::Performed(p) = &c.effect {
            set.extend(p.inputs.iter().cloned());
        }
    }
    set.into_iter().collect()
}

/// The words `check` says of a program with inputs: `performed: knob(3), fader(A) (read when its line is due)`.
pub(crate) fn report(c: &mut Compiler) {
    let inputs = performed_inputs(&c.controls);
    if inputs.is_empty() {
        return;
    }
    let at = c.controls.iter().find(|k| matches!(k.effect, Effect::Performed(_))).map_or(Span::default(), |k| k.span);
    let list = inputs.iter().map(Input::id).collect::<Vec<_>>().join(", ");
    c.note(at, format!("performed: {list} (read when its line is due)"));
}

// ---- reading while the program plays: quantising, the log, the performance

/// An input's value as it is read: 0..1 in steps of 1/128.
pub fn quantise(v: f64) -> f64 {
    if v.is_nan() {
        return 0.0;
    }
    (v.clamp(0.0, 1.0) * 128.0).round() / 128.0
}

/// The last bar a performance may name (a take of twenty-odd days at 120 BPM): a file with a bar beyond it is not a take, and a
/// bar that large would overflow a replay's own count.
pub const MAX_BAR: f64 = 1_048_576.0;

/// A bar is told apart in this many places: a reading is of the bar and one of these 1/64 slots ([`Reading::at`]).
pub const SLOTS: u8 = 64;

/// The slot of the bar a position falls in: 0 for a bar line, 16 for a quarter of the way in. `at` is a position in bars and `bar` the
/// whole bar it is in.
pub fn slot_of(at: f64, bar: i64) -> u8 {
    let into = at - bar as f64;
    if into <= 0.0 {
        return 0;
    }
    ((into * f64::from(SLOTS) + 1e-9).floor() as i64).clamp(0, i64::from(SLOTS) - 1) as u8
}

/// One value read: the program bar, where in the bar (a 1/64 slot, 0 for the bar line), the input as the language writes it and the
/// value (quantised).
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub bar: i64,
    /// The 1/64 of the bar it was read at (a line that fires between bar lines reads when it is due). A reading of the bar line
    /// is 0, and is saved without it, so a performance made before this existed is still the same performance.
    pub at: u8,
    pub input: String,
    pub value: f64,
}

/// A scene a pad brought in (launch.rs): the program bar it landed on and its name. A take logs them beside the inputs, so a replay brings
/// the same scenes in on the same bar lines whatever the pads say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launched {
    pub bar: i64,
    pub scene: String,
}

/// A performance: the inputs read while one program played, by bar. An object with its own CID (the share CID of its canonical
/// JSON, as a program's is), so a performance can be named, saved and replayed by it, and a changed value is another performance.
#[derive(Clone, Debug, PartialEq)]
pub struct Performance {
    /// The CID of the program it was played on (of its canonical text).
    pub program: String,
    /// Sorted by bar, then place in the bar, then input: the canonical order.
    pub inputs: Vec<Reading>,
    /// The scenes brought in, in order: said in the saved object only when there are some (`"launches": [{"bar": 24, "scene": "drop"}]`),
    /// so no other performance's CID moves.
    pub launches: Vec<Launched>,
    /// The log hit its cap ([`MAX_READINGS`]) and stopped: what is here is the start of the take, not all of it. Said in the saved
    /// object (`"truncated": true`, only then, so no other performance's CID moves).
    pub truncated: bool,
}

/// The performance object's `type`.
pub const TYPE: &str = "djrust.performance";

impl Performance {
    pub fn new(program: impl Into<String>, mut inputs: Vec<Reading>) -> Performance {
        inputs.sort_by(|a, b| a.bar.cmp(&b.bar).then_with(|| a.at.cmp(&b.at)).then_with(|| a.input.cmp(&b.input)));
        inputs.dedup_by(|a, b| a.bar == b.bar && a.at == b.at && a.input == b.input);
        Performance { program: program.into(), inputs, launches: Vec::new(), truncated: false }
    }

    pub fn to_json(&self) -> JsonValue {
        let inputs = self
            .inputs
            .iter()
            .map(|r| {
                let mut pairs = vec![("bar", JsonValue::int(r.bar)), ("input", JsonValue::str(r.input.clone())), ("value", JsonValue::Number(r.value))];
                if r.at != 0 {
                    pairs.push(("at", JsonValue::int(i64::from(r.at))));
                }
                JsonValue::Object(JsonObject::from_pairs(pairs))
            })
            .collect();
        let mut pairs = vec![("format", JsonValue::int(1)), ("inputs", JsonValue::Array(inputs)), ("program", JsonValue::str(self.program.clone())), ("type", JsonValue::str(TYPE))];
        if !self.launches.is_empty() {
            let l = self.launches.iter().map(|l| JsonValue::Object(JsonObject::from_pairs([("bar", JsonValue::int(l.bar)), ("scene", JsonValue::str(l.scene.clone()))]))).collect();
            pairs.push(("launches", JsonValue::Array(l)));
        }
        if self.truncated {
            pairs.push(("truncated", true.into()));
        }
        JsonValue::Object(JsonObject::from_pairs(pairs))
    }

    /// The performance's CID: of its canonical JSON.
    pub fn cid(&self) -> String {
        cid::share_cid(&self.to_json())
    }

    /// Read a performance object back, refusing one that is not well formed: a wrong type, an input that is not one, a value
    /// that is not in 0..1 in steps of 1/128, a negative bar.
    pub fn from_json(v: &JsonValue) -> Result<Performance, String> {
        if v.get("type").and_then(JsonValue::as_str) != Some(TYPE) {
            return Err(format!("not a performance: its type is not `{TYPE}`"));
        }
        let program = v.get("program").and_then(JsonValue::as_str).filter(|p| !p.is_empty()).ok_or("a performance names the program it was played on (`program`)")?.to_string();
        let items = v.get("inputs").and_then(JsonValue::as_array).ok_or("a performance has `inputs`")?;
        let mut inputs = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            let bar = it.get("bar").and_then(JsonValue::as_f64).filter(|b| (0.0..=MAX_BAR).contains(b) && b.fract() == 0.0).ok_or_else(|| format!("input {i}: `bar` is a whole number from 0 to {MAX_BAR}"))?;
            let input = it.get("input").and_then(JsonValue::as_str).ok_or_else(|| format!("input {i}: `input` is a name like knob(3)"))?;
            if Input::parse_id(input).is_none() {
                return Err(format!("input {i}: `{input}` is not an input (knob(N), pad(N), fader(A), held(key), held(c4))"));
            }
            let value = it.get("value").and_then(JsonValue::as_f64).filter(|x| (0.0..=1.0).contains(x) && quantise(*x) == *x).ok_or_else(|| format!("input {i}: `value` is 0 to 1 in steps of 1/128"))?;
            let at = match it.get("at") {
                None => 0,
                Some(a) => a.as_f64().filter(|a| (0.0..f64::from(SLOTS)).contains(a) && a.fract() == 0.0).ok_or_else(|| format!("input {i}: `at` is a whole number from 0 to {} (a 64th of the bar)", SLOTS - 1))? as u8,
            };
            inputs.push(Reading { bar: bar as i64, at, input: input.to_string(), value });
        }
        let mut p = Performance::new(program, inputs);
        if let Some(items) = v.get("launches") {
            let items = items.as_array().ok_or("`launches` is a list of {bar, scene}")?;
            for (i, it) in items.iter().enumerate() {
                let bar = it.get("bar").and_then(JsonValue::as_f64).filter(|b| (0.0..=MAX_BAR).contains(b) && b.fract() == 0.0).ok_or_else(|| format!("launch {i}: `bar` is a whole number from 0 to {MAX_BAR}"))?;
                let scene = it.get("scene").and_then(JsonValue::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("launch {i}: `scene` is the name of a section or an arrangement"))?;
                p.launches.push(Launched { bar: bar as i64, scene: scene.to_string() });
            }
            p.launches.sort_by_key(|l| l.bar);
        }
        p.truncated = v.get("truncated").and_then(JsonValue::as_bool).unwrap_or(false);
        Ok(p)
    }

    /// The first and last bar read.
    pub fn span_bars(&self) -> Option<(i64, i64)> {
        Some((self.inputs.first()?.bar, self.inputs.last()?.bar))
    }

    /// The inputs it has values for, each once.
    pub fn input_ids(&self) -> Vec<String> {
        self.inputs.iter().map(|r| r.input.clone()).collect::<BTreeSet<_>>().into_iter().collect()
    }
}

/// The performance being replayed, indexed by bar and place in the bar.
struct ReplayLog {
    perf: Performance,
    /// The performance's CID, worked out once when the replay began (hashing it on every look at the status was R-07's cost).
    cid: String,
    by: BTreeMap<(i64, u8, String), f64>,
}

impl ReplayLog {
    /// What the take read for `id` at that place in the bar; failing that, what it read at the bar line: a take made before a bar
    /// could be read in more than one place has one value for the bar, and replays as it always did.
    fn value(&self, bar: i64, at: u8, id: &str) -> Option<f64> {
        let get = |at: u8| self.by.get(&(bar, at, id.to_string())).copied();
        get(at).or_else(|| if at == 0 { None } else { get(0) })
    }
}

/// What the runtime keeps of the inputs: where they are read from, the bar's values, and the performance being logged.
pub struct PerfState {
    replay: Option<ReplayLog>,
    /// The program the take is of (its CID).
    program: Option<String>,
    entries: Vec<Reading>,
    recording: bool,
    /// The log reached [`MAX_READINGS`] and stopped taking values (said once in `notes`, and in the saved take).
    full: bool,
    /// The take of the program before, kept so it can still be saved.
    last: Option<Performance>,
    /// The place in the program the cache is of: the bar and the 1/64 of it.
    cache_at: (i64, u8),
    cache: BTreeMap<Input, f64>,
    /// Inputs already said to have no controller.
    said: BTreeSet<String>,
    /// Words for the runtime's log (`btw get`), taken by it after each use.
    pub notes: Vec<String>,
    reads: u64,
    /// The scenes a pad brought in during the take (launch.rs), and how many of a replayed take's have been taken.
    launches: Vec<Launched>,
    launch_next: usize,
}

/// The most readings one take holds (a bar's reads of a handful of inputs for about two days of music).
pub const MAX_READINGS: usize = 262_144;

impl Default for PerfState {
    fn default() -> Self {
        PerfState { replay: None, program: None, entries: Vec::new(), recording: true, full: false, last: None, cache_at: (i64::MIN, 0), cache: BTreeMap::new(), said: BTreeSet::new(), notes: Vec::new(), reads: 0, launches: Vec::new(), launch_next: 0 }
    }
}

impl PerfState {
    /// One input's value at the bar line of `bar`: [`PerfState::read_at`] with the place 0.
    pub fn read(&mut self, host: &mut dyn LiveHost, bar: i64, input: &Input) -> f64 {
        self.read_at(host, bar, 0, input)
    }

    /// One input's value for the place `at` (a 1/64 of the bar, [`slot_of`]) of `bar`: asked of the host (or the replayed log) the
    /// first time at that place and kept while the program is there, so a line, a ramp and a guard that are due together and name
    /// `knob(3)` agree. A value that arrives is quantised and logged; none is read as the input's home ([`Input::home`]: 1/2 for a
    /// knob, else 0), with a word once.
    pub fn read_at(&mut self, host: &mut dyn LiveHost, bar: i64, at: u8, input: &Input) -> f64 {
        if self.cache_at != (bar, at) {
            self.cache.clear();
            self.cache_at = (bar, at);
        }
        if let Some(v) = self.cache.get(input) {
            return *v;
        }
        self.reads += 1;
        let id = input.id();
        let raw = match &self.replay {
            Some(r) => r.value(bar, at, &id),
            None => host.input(input.kind(), &input.arg()),
        };
        let v = match raw {
            Some(x) => {
                let q = quantise(x);
                if self.recording && self.program.is_some() {
                    if self.entries.len() < MAX_READINGS {
                        self.entries.push(Reading { bar, at, input: id, value: q });
                    } else if !self.full {
                        // the log is full: the values are still read, but not kept. Said once, and the saved take says it is cut (R-23)
                        self.full = true;
                        self.notes.push(format!("performance: the log is full ({MAX_READINGS} readings): the take stops at bar {bar}; `btw perform start` begins another (the saved take is marked truncated)"));
                    }
                }
                q
            }
            None => {
                let home = input.home();
                if self.said.insert(id.clone()) {
                    let reads = if home == 0.0 { "0" } else { "1/2" };
                    self.notes.push(if self.replay.is_some() {
                        format!("{id}: the performance has no value for it at bar {bar}: it reads {reads}")
                    } else {
                        format!("{id}: no controller is connected for it: it reads {reads}")
                    });
                }
                home
            }
        };
        self.cache.insert(input.clone(), v);
        v
    }

    /// A performed line's effect for the place `at` of `bar`, or None when its guard is not true.
    pub fn resolve(&mut self, host: &mut dyn LiveHost, p: &Performed, bar: i64, at: u8) -> Option<Effect> {
        let mut rd = |i: &Input| self.read_at(host, bar, at, i);
        if !p.guard_holds(&mut rd) {
            return None;
        }
        Some(p.apply(&mut rd))
    }

    /// The same line's effect read again (a running ramp at a bar boundary): no guard.
    pub fn reread(&mut self, host: &mut dyn LiveHost, p: &Performed, bar: i64) -> Effect {
        let mut rd = |i: &Input| self.read(host, bar, i);
        p.apply(&mut rd)
    }

    /// A program was applied: a new take when it is a program with another CID, or a fresh start of it. The take before is kept
    /// (`last`) so it can still be saved; a replay of another program ends.
    pub fn program_applied(&mut self, cid: &str, fresh: bool) {
        if fresh || self.program.as_deref() != Some(cid) {
            self.close_take();
            self.program = Some(cid.to_string());
            self.cache.clear();
            self.cache_at = (i64::MIN, 0);
            self.said.clear();
        }
        if self.replay.as_ref().is_some_and(|r| r.perf.program != cid) {
            self.replay = None;
            self.notes.push("replay ended: another program is running".into());
        }
    }

    /// The program was stopped: its take is kept, the replay ends.
    pub fn stopped(&mut self) {
        self.close_take();
        self.program = None;
        self.replay = None;
        self.cache.clear();
        self.cache_at = (i64::MIN, 0);
    }

    fn close_take(&mut self) {
        if let (Some(p), false) = (&self.program, self.log_is_empty()) {
            let mut take = Performance::new(p.clone(), std::mem::take(&mut self.entries));
            take.launches = std::mem::take(&mut self.launches);
            take.truncated = self.full;
            self.last = Some(take);
        }
        self.entries.clear();
        self.launches.clear();
        self.full = false;
    }

    /// Nothing was read and no scene brought in.
    fn log_is_empty(&self) -> bool {
        self.entries.is_empty() && self.launches.is_empty()
    }

    /// A scene landed on the program bar `bar` (a pad's hit, or the replay of one): logged in the take.
    pub fn log_launch(&mut self, bar: i64, scene: &str) {
        if self.recording && self.program.is_some() {
            self.launches.push(Launched { bar, scene: scene.to_string() });
        }
    }

    /// The scene a replayed take brought in on the program bar `bar`, once. None when the take has none there, or nothing is replayed.
    pub fn replay_launch(&mut self, bar: i64) -> Option<String> {
        let r = self.replay.as_ref()?;
        while r.perf.launches.get(self.launch_next).is_some_and(|l| l.bar < bar) {
            self.launch_next += 1;
        }
        let l = r.perf.launches.get(self.launch_next).filter(|l| l.bar == bar)?;
        self.launch_next += 1;
        Some(l.scene.clone())
    }

    /// The next scene a replayed take brought in, whichever bar: after the song ended there is no bar line to count from.
    pub fn replay_launch_after_end(&mut self) -> Option<String> {
        let r = self.replay.as_ref()?;
        let l = r.perf.launches.get(self.launch_next)?;
        self.launch_next += 1;
        Some(l.scene.clone())
    }

    /// Begin a new take now: what was logged is dropped (kept as the last take), logging is on.
    pub fn start(&mut self) {
        self.close_take();
        self.recording = true;
        self.said.clear();
    }

    /// Stop logging: what was read is kept to be saved.
    pub fn stop_recording(&mut self) {
        self.recording = false;
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Replay `perf`: its values answer the reads in place of the host.
    pub fn replay(&mut self, perf: Performance) {
        let by = perf.inputs.iter().map(|r| ((r.bar, r.at, r.input.clone()), r.value)).collect();
        self.close_take();
        self.recording = true;
        self.cache.clear();
        self.cache_at = (i64::MIN, 0);
        self.said.clear();
        let cid = perf.cid();
        self.launch_next = 0;
        self.replay = Some(ReplayLog { perf, cid, by });
    }

    pub fn is_replaying(&self) -> bool {
        self.replay.is_some()
    }

    pub fn stop_replay(&mut self) {
        self.replay = None;
    }

    /// The program the performance being replayed is of.
    pub fn replaying_program(&self) -> Option<&str> {
        self.replay.as_ref().map(|r| r.perf.program.as_str())
    }

    /// The take so far: the running program's if it has read anything, else the last one.
    pub fn performance(&self) -> Option<Performance> {
        match (&self.program, self.log_is_empty()) {
            (Some(p), false) => {
                let mut take = Performance::new(p.clone(), self.entries.clone());
                take.launches = self.launches.clone();
                take.truncated = self.full;
                Some(take)
            }
            _ => self.last.clone(),
        }
    }

    /// Whether there is a take to say (being logged, or the one before): [`PerfState::performance`] is Some, without building it.
    pub fn has_take(&self) -> bool {
        matches!((&self.program, self.log_is_empty()), (Some(_), false)) || self.last.is_some()
    }

    /// What `btw get` and the `perform` verb say of the take. `with_cid` hashes the take to say its CID (the `perform` verb asks for it:
    /// a take can be hundreds of thousands of readings, and the window looks at the status every frame); without it the counts are all
    /// that is said and nothing is cloned, sorted or hashed.
    pub fn status(&self, inputs: &[Input], with_cid: bool) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("inputs", JsonValue::Array(inputs.iter().map(|i| JsonValue::str(i.id())).collect()));
        o.set("recording", self.recording.into());
        o.set("replaying", self.replay.as_ref().map_or(JsonValue::Null, |r| JsonValue::str(r.cid.clone())));
        o.set("readings", JsonValue::int(self.entries.len() as i64));
        o.set("reads", JsonValue::int(self.reads as i64));
        o.set("log_full", self.full.into());
        if !self.launches.is_empty() {
            o.set("launches", JsonValue::int(self.launches.len() as i64));
        }
        o.set("program", self.program.clone().map_or(JsonValue::Null, JsonValue::str));
        let (count, bars) = if with_cid {
            // the take as a performance (sorted, one value a bar and input): its CID, and the counts of that
            let perf = self.performance();
            o.set("take", perf.as_ref().map_or(JsonValue::Null, |p| JsonValue::str(p.cid())));
            (perf.as_ref().map(|p| p.inputs.len()), perf.as_ref().and_then(Performance::span_bars))
        } else if matches!((&self.program, self.log_is_empty()), (Some(_), false)) {
            // what is being logged, counted where it lies (a bar and input read twice, which a performance keeps once, is counted twice)
            let bars = self.entries.iter().fold(None, |a: Option<(i64, i64)>, r| Some(a.map_or((r.bar, r.bar), |(lo, hi)| (lo.min(r.bar), hi.max(r.bar)))));
            (Some(self.entries.len()), bars)
        } else {
            // the take before, kept to be saved
            (self.last.as_ref().map(|p| p.inputs.len()), self.last.as_ref().and_then(Performance::span_bars))
        };
        o.set("take_readings", count.map_or(JsonValue::Null, |n| JsonValue::int(n as i64)));
        if let Some((a, b)) = bars {
            o.set("take_bars", JsonValue::Array(vec![JsonValue::int(a), JsonValue::int(b)]));
        }
        JsonValue::Object(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        ["knob(3)", "pad(12)", "fader(A)", "fader(B)", "held(key)", "held(c4)", "held(c#2)", "held(g9)"].iter().map(|s| s.to_string()).collect()
    }

    fn lowered(src: &str) -> LiveExpr {
        let lexed = crate::lex::lex(src);
        let mut c = crate::expr::Cursor { toks: &lexed.tokens, at: 0 };
        lower_expr(&crate::expr::parse_expr(&mut c).unwrap(), None).unwrap()
    }

    #[test]
    fn an_input_id_round_trips_and_a_note_is_spelled_one_way() {
        for id in ids() {
            let i = Input::parse_id(&id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(i.id(), id);
        }
        assert_eq!(Input::parse_id("held(C4)"), None, "ids are canonical: lower case");
        assert_eq!(Input::parse_id("held(db4)"), None, "sharps, not flats");
        assert_eq!(Input::parse_id("knob(0)"), None);
        assert_eq!(Input::parse_id("fader(C)"), None);
        assert_eq!(Input::parse_id("held(c)"), None, "a note needs an octave");
        assert_eq!(note_number("c4"), Some(60));
        assert_eq!(note_number("C#4"), Some(61));
        assert_eq!(note_number("g9"), Some(127));
        assert_eq!(note_number("a9"), None, "past the MIDI range");
        assert_eq!(note_name(60), "c4");
    }

    #[test]
    fn values_are_quantised_to_a_hundred_and_twenty_eighth() {
        assert_eq!(quantise(0.0), 0.0);
        assert_eq!(quantise(1.0), 1.0);
        assert_eq!(quantise(0.5), 0.5);
        assert_eq!(quantise(60.0 / 127.0), 60.0 / 128.0);
        assert_eq!(quantise(-3.0), 0.0);
        assert_eq!(quantise(7.0), 1.0);
        assert_eq!(quantise(f64::NAN), 0.0);
        assert_eq!(quantise(0.3), 38.0 / 128.0);
    }

    #[test]
    fn a_performance_is_an_object_with_its_own_cid_and_a_changed_value_is_another() {
        let p = Performance::new("zProg", vec![Reading { bar: 4, at: 0, input: "knob(3)".into(), value: 0.5 }, Reading { bar: 1, at: 0, input: "fader(A)".into(), value: 1.0 }]);
        assert_eq!(p.inputs[0].bar, 1, "sorted by bar");
        let again = Performance::new("zProg", p.inputs.iter().rev().cloned().collect());
        assert_eq!(p.cid(), again.cid(), "the order it was read in is not part of it");
        let mut t = p.clone();
        t.inputs[1].value = 0.5078125;
        assert_ne!(t.cid(), p.cid());
        let other = Performance::new("zOther", p.inputs.clone());
        assert_ne!(other.cid(), p.cid(), "it is a performance of one program");
        let back = Performance::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.cid(), p.cid());
        assert!(p.cid().starts_with('z'));
    }

    #[test]
    fn a_performance_that_is_not_well_formed_is_refused() {
        let ok = Performance::new("zProg", vec![Reading { bar: 0, at: 0, input: "knob(1)".into(), value: 0.25 }]).to_json();
        assert!(Performance::from_json(&ok).is_ok());
        let bad = |from: &str, to: &str| JsonValue::parse(&ok.canonical_js().replace(from, to)).unwrap();
        assert!(Performance::from_json(&bad("djrust.performance", "djrust.live")).unwrap_err().contains("not a performance"));
        assert!(Performance::from_json(&bad("knob(1)", "knob(0)")).unwrap_err().contains("not an input"));
        assert!(Performance::from_json(&bad("0.25", "0.3")).unwrap_err().contains("1/128"));
        assert!(Performance::from_json(&bad("\"bar\":0", "\"bar\":-1")).unwrap_err().contains("whole number from 0"));
    }

    #[test]
    fn the_expression_over_inputs_evaluates_with_total_division() {
        let mut read = |i: &Input| match i {
            Input::Knob(3) => 0.5,
            Input::Knob(4) => 0.0,
            Input::HeldKey => 60.0 / 128.0,
            Input::Fader(Deck::A) => 1.0,
            _ => 0.0,
        };
        assert_eq!(lowered("200 + knob(3) * 7800").eval(&mut read), 4100.0);
        assert_eq!(lowered("knob(3) / knob(4)").eval(&mut read), 0.0, "a division by zero reads 0");
        assert_eq!(lowered("held(key) > 0.4").eval(&mut read), 1.0);
        assert_eq!(lowered("held(key) and fader(A) == 1").eval(&mut read), 1.0);
        assert_eq!(lowered("not held(c4)").eval(&mut read), 1.0);
        assert_eq!(lowered("min(knob(3), fader(A), 0.25)").eval(&mut read), 0.25);
        assert_eq!(lowered("max(knob(3), 0.75)").eval(&mut read), 0.75);
        assert_eq!(lowered("-knob(3) + 1").eval(&mut read), 0.5);
        assert_eq!(lowered("(2 + 3) * knob(3)").eval(&mut read), 2.5, "what has no input is folded");
        let mut set = BTreeSet::new();
        lowered("knob(3) * held(key) + fader(B)").inputs(&mut set);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn bounds_say_what_an_expression_can_be() {
        assert_eq!(lowered("200 + knob(3) * 7800").bounds(), (200.0, 8000.0));
        assert_eq!(lowered("1 - knob(1)").bounds(), (0.0, 1.0));
        assert_eq!(lowered("knob(1) * 2").bounds(), (0.0, 2.0));
        assert_eq!(lowered("-knob(1)").bounds(), (-1.0, 0.0));
        assert!(lowered("1 / knob(1)").may_divide_by_zero());
        assert!(!lowered("1 / (1 + knob(1))").may_divide_by_zero());
        assert_eq!(lowered("min(knob(1), 0.5)").bounds(), (0.0, 0.5));
    }

    #[test]
    fn the_arguments_of_an_input_are_checked_with_the_bank() {
        let (n, sp) = (|r: i64| A::Num(Rat::int(r)), Span::default());
        let bank = InputBank { knobs: 8, pads: 16 };
        assert_eq!(build("knob", &[n(3)], sp, bank, true).unwrap(), Input::Knob(3));
        let low = build("knob", &[n(0)], sp, bank, true).unwrap_err();
        assert!(low.message.contains("numbered from 1"), "{}", low.message);
        let high = build("knob", &[n(9)], sp, bank, true).unwrap_err();
        assert!(high.message.contains("up to 8 knobs"), "{}", high.message);
        let dflt = build("pad", &[n(129)], sp, InputBank::MAX, false).unwrap_err();
        assert!(dflt.message.contains("up to 128 pads"), "{}", dflt.message);
        assert!(dflt.help.unwrap().contains("reads 0 for the rest"));
        assert_eq!(build("fader", &[A::Word("A".into())], sp, bank, true).unwrap(), Input::Fader(Deck::A));
        let c = build("fader", &[A::Word("C".into())], sp, bank, true).unwrap_err();
        assert_eq!(c.expected, ["A", "B"]);
        assert!(build("fader", &[A::Word("a".into())], sp, bank, true).unwrap_err().help.unwrap().contains("upper case"));
        assert_eq!(build("held", &[A::Word("key".into())], sp, bank, true).unwrap(), Input::HeldKey);
        assert_eq!(build("held", &[A::Word("c4".into())], sp, bank, true).unwrap(), Input::Held(60));
        assert!(build("held", &[A::Word("c".into())], sp, bank, true).unwrap_err().help.unwrap().contains("octave"));
        assert!(build("held", &[], sp, bank, true).unwrap_err().message.contains("one argument"));
        assert!(build("knob", &[A::Num(Rat::new(1, 2).unwrap())], sp, bank, true).unwrap_err().message.contains("whole number"));
    }

    struct Scripted(BTreeMap<String, f64>);

    impl LiveHost for Scripted {
        fn grid(&self) -> Option<crate::runtime::Grid> {
            None
        }
        fn playing(&self, _: &str) -> bool {
            false
        }
        fn load_net(&mut self, _: &str, _: &JsonValue) -> Result<(), String> {
            Ok(())
        }
        fn set_playing(&mut self, _: &str, _: bool) {}
        fn call(&mut self, _: &str, _: &JsonValue) -> Result<JsonValue, String> {
            Ok(JsonValue::Null)
        }
        fn input(&mut self, kind: &str, id: &str) -> Option<f64> {
            self.0.get(&format!("{kind}({id})")).copied()
        }
    }

    #[test]
    fn a_value_is_read_once_a_bar_logged_and_replayed_by_bar() {
        let mut host = Scripted([("knob(3)".to_string(), 0.3)].into_iter().collect());
        let mut st = PerfState::default();
        st.program_applied("zProg", true);
        let k = Input::Knob(3);
        assert_eq!(st.read(&mut host, 0, &k), 38.0 / 128.0);
        // the knob moves inside the bar: the bar keeps the value it read
        host.0.insert("knob(3)".into(), 0.9);
        assert_eq!(st.read(&mut host, 0, &k), 38.0 / 128.0);
        assert_eq!(st.read(&mut host, 1, &k), 115.0 / 128.0);
        // a value nobody answers reads 0 and is said once, and is not logged
        assert_eq!(st.read(&mut host, 1, &Input::Pad(2)), 0.0);
        assert_eq!(st.read(&mut host, 2, &Input::Pad(2)), 0.0);
        assert_eq!(st.notes.len(), 1, "{:?}", st.notes);
        assert!(st.notes[0].contains("no controller"));
        let perf = st.performance().unwrap();
        assert_eq!(perf.program, "zProg");
        assert_eq!(perf.inputs, vec![Reading { bar: 0, at: 0, input: "knob(3)".into(), value: 38.0 / 128.0 }, Reading { bar: 1, at: 0, input: "knob(3)".into(), value: 115.0 / 128.0 }]);
        // the replay answers by bar, whatever the host says now, and logs the same take
        let mut r = PerfState::default();
        r.program_applied("zProg", true);
        r.replay(perf.clone());
        host.0.insert("knob(3)".into(), 0.0);
        assert_eq!(r.read(&mut host, 0, &k), 38.0 / 128.0);
        assert_eq!(r.read(&mut host, 1, &k), 115.0 / 128.0);
        assert_eq!(r.performance().unwrap().cid(), perf.cid());
        // a bar the log has no value for reads the input's home (a knob rests at 1/2, a pad at 0), with a word
        assert_eq!(r.read(&mut host, 7, &k), 0.5);
        assert_eq!(r.read(&mut host, 7, &Input::Pad(2)), 0.0);
        assert!(r.notes.iter().any(|n| n.contains("knob(3): the performance has no value for it at bar 7: it reads 1/2")), "{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.contains("pad(2): the performance has no value for it at bar 7: it reads 0")), "{:?}", r.notes);
    }

    #[test]
    fn a_take_is_per_program_and_stopping_keeps_it_to_save() {
        let mut host = Scripted([("knob(1)".to_string(), 0.5)].into_iter().collect());
        let mut st = PerfState::default();
        st.program_applied("zA", true);
        st.read(&mut host, 0, &Input::Knob(1));
        assert_eq!(st.performance().unwrap().program, "zA");
        // another program: a new take; the old one is the last
        st.program_applied("zB", false);
        assert_eq!(st.performance().unwrap().program, "zA", "nothing read yet in B: the last take is still there");
        st.read(&mut host, 1, &Input::Knob(1));
        assert_eq!(st.performance().unwrap().program, "zB");
        st.stopped();
        assert_eq!(st.performance().unwrap().program, "zB");
        // stop_recording keeps what was read and logs no more
        let mut s2 = PerfState::default();
        s2.program_applied("zA", true);
        s2.read(&mut host, 0, &Input::Knob(1));
        s2.stop_recording();
        s2.read(&mut host, 1, &Input::Knob(1));
        assert_eq!(s2.performance().unwrap().inputs.len(), 1);
        s2.start();
        s2.read(&mut host, 2, &Input::Knob(1));
        assert_eq!(s2.performance().unwrap().inputs.len(), 1, "a new take began");
        assert_eq!(s2.performance().unwrap().inputs[0].bar, 2);
    }
}
