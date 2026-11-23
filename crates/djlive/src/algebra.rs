//! F10, composition made literal: `stack(a, b)` (parallel, the tensor) and `then(a, b)` (sequential,
//! composition) over sections and over parts, with `silence` as the unit of both (docs/beatscript/ext-algebra.md,
//! docs/BEATSCRIPT-EXTENSIONS.md section 3).
//!
//! * **Section expressions** ([`SExpr`]) are read by `arrange` and by `let NAME = sexpr`. They are normalised the
//!   moment they are read, by the laws of a symmetric monoidal category: both operations are associative (nested ones
//!   flatten), `silence` is dropped, a one-operand `stack` / `then` is its operand, `stack` is symmetric (its operands
//!   are sorted by canonical text: the braiding is the sort) and `then` is not reordered. The canonical text is the
//!   normal form, so programs equal by the laws have one CID.
//! * **Lowering** ([`Lower::plan_arrangement`]) turns the normal form into a timeline: every section named in it is a
//!   [`Placement`] (the section at a start bar), and the song is cut into [`Stretch`]es at every bar where a placement
//!   starts or ends. A stretch holds the placements that play over it, so stacked sections of unequal length come out as
//!   consecutive segments of the song (the shorter's decks stop where it ends) and nothing downstream of `expand`
//!   (the compiler, the runtime, the offline render) has to know about stacking.
//! * **Parts** (`deck A: stack(kick x ~ x ~, snare ~ x ~ x)`): `stack` is `|`, `then` of parts of one role is the
//!   per-bar alternation of their patterns. Both are desugared when read, so the canonical text is the plain `|` and
//!   `<...>` text.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::expand::{FlatLine, Lower, Lowered, Scope, SectionInfo, SegmentLines};
use crate::lex::Tok;
use crate::parse::{nearest, Parser};
use crate::print;

type R<T> = Result<T, Diagnostic>;

/// The words of section and part expressions (they are in `parse::EXT_WORDS` too: none can name a section).
pub const WORDS: [&str; 3] = ["stack", "then", "silence"];

/// How deep `stack(then(stack(...)))` may nest, and how many lets one name may pass through.
pub const MAX_NEST: usize = 32;
/// The most times sections may be placed on a timeline (one bar at least each, so a plain `arrange` of a whole song
/// cannot reach it, and a stack of lets that double each other stops here).
pub const MAX_PLACEMENTS: usize = crate::compile::MAX_SONG_BARS as usize;

// ---- section expressions

/// A section expression, always in normal form (see [`SExpr::then`], [`SExpr::stack`]): no `Silence` inside a
/// `Stack` or `Then`, no `Stack` directly in a `Stack`, no `Then` directly in a `Then`, every `Stack` and `Then` has at
/// least two operands, and a `Stack`'s operands are sorted by [`SExpr::canonical`].
#[derive(Clone, Debug, PartialEq)]
pub enum SExpr {
    /// A section's name, or the name of a `let NAME = sexpr`.
    Name(Word),
    /// `silence`: the empty section, the unit of `stack` and of `then`.
    Silence(Span),
    /// `stack(a, b ...)`: at the same time.
    Stack(Vec<SExpr>, Span),
    /// `then(a, b ...)`: one after the other.
    Then(Vec<SExpr>, Span),
}

/// `let NAME = stack(a, b)`: a name for a composition.
#[derive(Clone, Debug, PartialEq)]
pub struct LetSection {
    pub name: Word,
    pub expr: SExpr,
}

impl SExpr {
    pub fn span(&self) -> Span {
        match self {
            SExpr::Name(w) => w.span,
            SExpr::Silence(s) | SExpr::Stack(_, s) | SExpr::Then(_, s) => *s,
        }
    }

    pub fn is_silence(&self) -> bool {
        matches!(self, SExpr::Silence(_))
    }

    /// The normal form's text: `a`, `silence`, `stack(a, b)`, `then(a, b)`, one space after each comma. This is what the
    /// canonical form of a program prints (an `arrange` prints its top-level `then` as a plain list).
    pub fn canonical(&self) -> String {
        match self {
            SExpr::Name(w) => w.text.clone(),
            SExpr::Silence(_) => "silence".into(),
            SExpr::Stack(xs, _) => format!("stack({})", xs.iter().map(SExpr::canonical).collect::<Vec<_>>().join(", ")),
            SExpr::Then(xs, _) => format!("then({})", xs.iter().map(SExpr::canonical).collect::<Vec<_>>().join(", ")),
        }
    }

    /// `then(xs)` by the laws: nested `then`s flatten (associativity), `silence` goes (unit), one operand is itself,
    /// none is `silence`. Order is never touched.
    pub fn then(xs: Vec<SExpr>, span: Span) -> SExpr {
        let mut flat = Vec::new();
        for x in xs {
            match x {
                SExpr::Then(inner, _) => flat.extend(inner.into_iter().filter(|i| !i.is_silence())),
                SExpr::Silence(_) => {}
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => SExpr::Silence(span),
            1 => flat.pop().expect("one"),
            _ => SExpr::Then(flat, span),
        }
    }

    /// `stack(xs)` by the laws: nested `stack`s flatten, `silence` goes, one operand is itself, and the operands are
    /// sorted by canonical text (symmetry: the braiding is the sort). Equal operands stay, as many as there are
    /// (`stack` is not idempotent: two copies of a section are two sections).
    pub fn stack(xs: Vec<SExpr>, span: Span) -> SExpr {
        let mut flat = Vec::new();
        for x in xs {
            match x {
                SExpr::Stack(inner, _) => flat.extend(inner.into_iter().filter(|i| !i.is_silence())),
                SExpr::Silence(_) => {}
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => SExpr::Silence(span),
            1 => flat.pop().expect("one"),
            _ => {
                flat.sort_by_cached_key(SExpr::canonical);
                SExpr::Stack(flat, span)
            }
        }
    }

    /// This expression with the laws applied all the way down (a no-op for anything the parser made, since it
    /// normalises as it reads; it is how a test builds an expression freely and gets its normal form).
    pub fn normalized(&self) -> SExpr {
        match self {
            SExpr::Name(_) | SExpr::Silence(_) => self.clone(),
            SExpr::Stack(xs, sp) => SExpr::stack(xs.iter().map(SExpr::normalized).collect(), *sp),
            SExpr::Then(xs, sp) => SExpr::then(xs.iter().map(SExpr::normalized).collect(), *sp),
        }
    }

    /// The top-level `then` as a list (an `arrange`'s items): a bare operand is a list of one.
    pub fn items(&self) -> Vec<SExpr> {
        match self {
            SExpr::Then(xs, _) => xs.clone(),
            other => vec![other.clone()],
        }
    }

    /// Every section name in it, in reading order.
    pub fn names(&self) -> Vec<&Word> {
        match self {
            SExpr::Name(w) => vec![w],
            SExpr::Silence(_) => Vec::new(),
            SExpr::Stack(xs, _) | SExpr::Then(xs, _) => xs.iter().flat_map(SExpr::names).collect(),
        }
    }
}

/// The statement an `arrange` of this expression is: the plain `Arrange` list when its items are all names (so a plain
/// `arrange a b c` is what it has always been), `ArrangeExpr` when a stack, a `then` inside one, or `silence` is left.
pub fn arrange_kind(e: SExpr) -> StmtKind {
    let items = e.items();
    if items.iter().all(|i| matches!(i, SExpr::Name(_))) {
        StmtKind::Arrange(items.into_iter().filter_map(|i| if let SExpr::Name(w) = i { Some(w) } else { None }).collect())
    } else {
        StmtKind::ArrangeExpr(items)
    }
}

/// The items of an `arrange` statement, either kind.
pub fn arrange_items(kind: &StmtKind) -> Vec<SExpr> {
    match kind {
        StmtKind::Arrange(words) => words.iter().cloned().map(SExpr::Name).collect(),
        StmtKind::ArrangeExpr(items) => items.clone(),
        _ => Vec::new(),
    }
}

/// The canonical text of an `arrange`'s items (`arrange a stack(b, c) d`).
pub fn arrange_text(items: &[SExpr]) -> String {
    format!("arrange {}", items.iter().map(SExpr::canonical).collect::<Vec<_>>().join(" "))
}

// ---- reading them

/// `let x = then(a)` is `let x = a` by the laws (a one-operand `then` is its operand), and `let x = a` is the scalar
/// `let` that puts `a` where `x` is used. So before the scalar lets are read, a `let` whose composition is a lone name (or
/// `silence`) once the laws are applied is rewritten to that name: one program, one text, and the canonical text stays a
/// fixed point (`let x = a` does not read back as a composition).
pub(crate) fn collapse_lets(toks: Vec<crate::lex::Token>) -> Vec<crate::lex::Token> {
    use crate::lex::Token;
    let word = |t: Option<&Token>, w: Option<&str>| matches!(t, Some(Token { tok: Tok::Word(x), .. }) if w.is_none_or(|w| x == w));
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut at_start = true;
    let mut brace = 0i32;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if at_start && brace == 0 && word(Some(t), Some("let")) && word(toks.get(i + 1), None) && matches!(toks.get(i + 2).map(|t| &t.tok), Some(Tok::Eq)) && (word(toks.get(i + 3), Some("stack")) || word(toks.get(i + 3), Some("then"))) && matches!(toks.get(i + 4).map(|t| &t.tok), Some(Tok::LParen)) {
            let end = toks.last().map_or(0, |t| t.span.end);
            let mut rest: Vec<Token> = toks[i + 3..].to_vec();
            rest.push(Token { tok: Tok::Eof, span: Span::new(end, end) });
            let mut p = Parser { toks: rest, at: 0, src: "", diags: Vec::new(), depth: 0, ratios: Default::default() };
            if let Ok(e) = p.sexpr() {
                let used = p.at;
                let ends = matches!(toks.get(i + 3 + used).map(|t| &t.tok), None | Some(Tok::Newline | Tok::Eof));
                let single = match &e {
                    SExpr::Name(w) => Some(Token { tok: Tok::Word(w.text.clone()), span: w.span }),
                    SExpr::Silence(sp) => Some(Token { tok: Tok::Word("silence".into()), span: *sp }),
                    _ => None,
                };
                if let (true, Some(v)) = (ends, single) {
                    out.extend_from_slice(&toks[i..i + 3]);
                    out.push(v);
                    i += 3 + used;
                    at_start = false;
                    continue;
                }
            }
        }
        match &t.tok {
            Tok::Newline => at_start = true,
            Tok::LBrace => {
                brace += 1;
                at_start = false;
            }
            Tok::RBrace => {
                brace -= 1;
                at_start = false;
            }
            _ => at_start = false,
        }
        out.push(t.clone());
        i += 1;
    }
    out
}

impl Parser<'_> {
    /// `stack(` or `then(` is next.
    pub(crate) fn sexpr_next(&self) -> bool {
        matches!(self.peek(), Tok::Word(w) if w == "stack" || w == "then") && self.peek_n(1) == &Tok::LParen
    }

    /// The help for a section expected where something else stands: a `$name` is the macros' (a def's argument, a repeat's
    /// `$i`), and `arrange` and `let` are statements of the top of the program, where there are none.
    fn no_macro_help(&self) -> String {
        if matches!(self.peek(), Tok::Dollar(_)) {
            "a section is named in the text: `arrange` and `let` are at the top of the program, outside every def and repeat, so there is no `$name` to put in place of one".to_string()
        } else {
            "arrange intro stack(drums, bass) outro".to_string()
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek() == &Tok::Newline {
            self.at += 1;
        }
    }

    /// One section expression, normalised.
    pub(crate) fn sexpr(&mut self) -> R<SExpr> {
        self.sexpr_at(0)
    }

    fn sexpr_at(&mut self, depth: usize) -> R<SExpr> {
        let Tok::Word(w) = self.peek().clone() else {
            return Err(self.expected(&["a section name", "stack(...)", "then(...)", "silence"], &format!("expected a section, found {}", self.found())).help(self.no_macro_help()));
        };
        let at = self.span();
        match w.as_str() {
            "silence" => {
                self.bump();
                Ok(SExpr::Silence(at))
            }
            "stack" | "then" => {
                self.bump();
                if self.peek() != &Tok::LParen {
                    return Err(self.expected(&["`(`"], &format!("`{w}` takes its sections in parentheses, found {}", self.found())).help(format!("{w}(a, b)")));
                }
                if depth >= MAX_NEST {
                    return Err(Diagnostic::error(at, format!("stack and then nest more than {MAX_NEST} deep")).help("name the inner composition with `let NAME = ...`"));
                }
                self.bump();
                let mut ops = Vec::new();
                loop {
                    self.skip_newlines();
                    ops.push(self.sexpr_at(depth + 1)?);
                    // the operand is read: a newline may precede its `,` or the `)`, and if the `)` is missing the
                    // error is at the end of this line (not on the next line's first word, which the error recovery
                    // would then skip)
                    let mark = self.at;
                    self.skip_newlines();
                    if !self.eat(&Tok::Comma) {
                        if self.peek() != &Tok::RParen {
                            self.at = mark;
                            return Err(self.expected(&["`,`", "`)`"], &format!("expected `)` to close {w}(...), found {}", self.found())));
                        }
                        break;
                    }
                }
                let close = self.bump().span;
                let span = at.to(close);
                Ok(if w == "stack" { SExpr::stack(ops, span) } else { SExpr::then(ops, span) })
            }
            _ => {
                if self.peek_n(1) == &Tok::LParen {
                    let mut d = Diagnostic::error(at, format!("`{w}(` is not a way to compose sections")).expected(&["stack(...)", "then(...)"]).found(format!("`{w}(`"));
                    if let Some(n) = nearest(&w, ["stack", "then"]) {
                        d = d.help(format!("did you mean `{n}`?"));
                    } else {
                        d = d.help("compose sections with stack(a, b) (together) and then(a, b) (one after the other)");
                    }
                    return Err(d);
                }
                Ok(SExpr::Name(self.word("a section name")?))
            }
        }
    }

    /// How a named arrangement ends: `then NAME`, `loop`, `loop N`, `end`, or nothing.
    pub(crate) fn arr_end(&mut self) -> R<crate::ast::ArrEnd> {
        use crate::ast::ArrEnd;
        Ok(match self.peek().clone() {
            Tok::Word(w) if w == "then" => {
                self.bump();
                ArrEnd::Then(self.word("an arrangement name")?)
            }
            Tok::Word(w) if w == "loop" => {
                let sp = self.bump().span;
                if matches!(self.peek(), Tok::Num { int: true, .. }) {
                    let (n, nsp) = self.int("a number of passes")?;
                    if n == 0 || n > 256 {
                        return Err(Diagnostic::error(nsp, "`loop N` repeats 1 to 256 times").help("arrange set: drop wubs loop 4 then close"));
                    }
                    ArrEnd::Times(n, sp.to(nsp))
                } else {
                    ArrEnd::Loop
                }
            }
            Tok::Word(w) if w == "end" => {
                self.bump();
                ArrEnd::End
            }
            _ => ArrEnd::Fall,
        })
    }

    /// After `arrange`: the items, as many as there are, read as `then` of them.
    pub(crate) fn arrange_stmt(&mut self) -> R<StmtKind> {
        use crate::ast::ArrDecl;
        // `arrange NAME: a b c then NAME | loop | loop N | end`: a named arrangement (flow.rs)
        let name = if matches!(self.peek(), Tok::Word(_)) && self.peek_n(1) == &Tok::Colon {
            let w = self.word("an arrangement name")?;
            self.bump();
            // `arrange NAME: { ... }`: a named arrangement with mutations (mutate.rs)
            if self.peek() == &Tok::LBrace {
                return self.named_arrange_block(w.span, w);
            }
            Some(w)
        } else {
            None
        };
        let mut items = Vec::new();
        while let Tok::Word(w) = self.peek() {
            if name.is_some() && (matches!(w.as_str(), "loop" | "end") || (w == "then" && self.peek_n(1) != &Tok::LParen)) {
                break;
            }
            items.push(self.sexpr()?);
        }
        if let Some(name) = name {
            let (Some(first), Some(last)) = (items.first(), items.last()) else {
                return Err(self.expected(&["a section name"], &format!("`arrange {}:` needs the sections in order, found {}", name.text, self.found())).help("arrange open: intro build then main"));
            };
            let _ = (first, last);
            let ends = self.arr_end()?;
            return Ok(StmtKind::ArrangeNamed(ArrDecl { name, items, ends }));
        }
        let (Some(first), Some(last)) = (items.first(), items.last()) else {
            return Err(self.expected(&["a section name"], &format!("`arrange` needs the sections in order, found {}", self.found())).help(self.no_macro_help()));
        };
        let span = first.span().to(last.span());
        Ok(arrange_kind(SExpr::then(items, span)))
    }

    // ---- over parts

    /// `stack(` / `then(` / `silence` is next, after `deck X:` or `|`.
    pub(crate) fn pexpr_next(&self) -> bool {
        self.sexpr_next() || matches!(self.peek(), Tok::Word(w) if w == "silence")
    }

    /// A part, or a part expression, as the parts it comes to.
    pub(crate) fn part_or_expr(&mut self) -> R<Vec<Part>> {
        if self.pexpr_next() {
            let start = self.span();
            let e = self.pexpr(0)?;
            return lower_pexpr(e, start);
        }
        Ok(vec![self.part()?])
    }

    fn pexpr(&mut self, depth: usize) -> R<PExpr> {
        let Tok::Word(w) = self.peek().clone() else {
            return Err(self.expected(&crate::parse::ROLES.map(|r| r), &format!("expected a part, found {}", self.found())).help("a part is `role pattern`: kick x ~ x ~"));
        };
        let at = self.span();
        if w == "silence" {
            self.bump();
            return Ok(PExpr::Silence);
        }
        if self.sexpr_next() {
            self.bump();
            if depth >= MAX_NEST {
                return Err(Diagnostic::error(at, format!("stack and then nest more than {MAX_NEST} deep")));
            }
            self.bump();
            let mut ops = Vec::new();
            loop {
                self.skip_newlines();
                ops.push(self.pexpr(depth + 1)?);
                let mark = self.at;
                self.skip_newlines();
                if !self.eat(&Tok::Comma) {
                    if self.peek() != &Tok::RParen {
                        self.at = mark;
                        return Err(self.expected(&["`,`", "`)`"], &format!("expected `)` to close {w}(...), found {}", self.found())));
                    }
                    break;
                }
            }
            let span = at.to(self.bump().span);
            return Ok(if w == "stack" { PExpr::Stack(ops) } else { PExpr::Then(ops, span) });
        }
        Ok(PExpr::Part(self.part()?))
    }
}

/// A part expression as read: parts, `silence`, `stack` and `then`.
enum PExpr {
    Part(Part),
    Silence,
    Stack(Vec<PExpr>),
    Then(Vec<PExpr>, Span),
}

/// The laws over parts, then the parts: `stack` flattens and its parts are sorted by their canonical text (so
/// `stack(a, b)` and `stack(b, a)` are one text), `then` of parts of one role is the alternation of their patterns.
fn lower_pexpr(e: PExpr, start: Span) -> R<Vec<Part>> {
    let parts = emit(e)?;
    if parts.is_empty() {
        return Err(Diagnostic::error(start, "`silence` is not a part: a deck line needs at least one").help("deck A: kick x ~ x ~"));
    }
    Ok(parts)
}

fn emit(e: PExpr) -> R<Vec<Part>> {
    match e {
        PExpr::Part(p) => Ok(vec![p]),
        PExpr::Silence => Ok(Vec::new()),
        PExpr::Stack(xs) => {
            let mut out = Vec::new();
            for x in xs {
                out.extend(emit(x)?);
            }
            out.sort_by_cached_key(print::part);
            Ok(out)
        }
        PExpr::Then(xs, span) => {
            // associativity: a `then` inside a `then` is its operands, in place
            let mut flat = Vec::new();
            flatten_then(xs, &mut flat);
            let mut singles = Vec::new();
            for x in flat {
                let mut ps = emit(x)?;
                match ps.len() {
                    0 => {}
                    1 => singles.push(ps.pop().expect("one")),
                    _ => {
                        let sp = ps[0].span.to(ps[ps.len() - 1].span);
                        return Err(Diagnostic::error(sp, "`then` follows one part with another of its role, and this operand is a stack of several")
                            .help("stack the roles outside: stack(then(kick .., kick ..), then(snare .., snare ..))"));
                    }
                }
            }
            match singles.len() {
                0 => Ok(Vec::new()),
                1 => Ok(singles),
                _ => Ok(vec![then_parts(singles, span)?]),
            }
        }
    }
}

fn flatten_then(xs: Vec<PExpr>, out: &mut Vec<PExpr>) {
    for x in xs {
        match x {
            PExpr::Then(inner, _) => flatten_then(inner, out),
            other => out.push(other),
        }
    }
}

/// `then(p, q ...)` of parts of one role: one part whose pattern is `<p q ...>` (a bar of each in turn).
fn then_parts(parts: Vec<Part>, span: Span) -> R<Part> {
    let first = &parts[0];
    let first_params = params_text(first);
    for p in &parts[1..] {
        if p.role.text != first.role.text {
            let mut d = Diagnostic::error(p.role.span, format!("`then` follows a part with another part of its role: `{}` is not `{}`", p.role.text, first.role.text))
                .found(format!("`{}`", p.role.text))
                .help("`stack(a, b)` plays two roles together; `then(a, b)` plays the bars of one role in turn");
            d.expected = vec![format!("`{}`", first.role.text)];
            return Err(d);
        }
        if p.cycle.map(|c| c.0) != first.cycle.map(|c| c.0) {
            // (F6: `hihat/12` is a cycle of its own; `<p q>` alternates by that cycle, so the operands must share it)
            let show = |c: &Part| c.cycle.map_or("a bar".to_string(), |x| format!("/{}", x.0));
            return Err(Diagnostic::error(p.span, format!("`then` makes one `{}` part, so its operands share their cycle: this one is {}, the first {}", first.role.text, show(p), show(first)))
                .help("give every operand the same cycle (hihat/12 ... , hihat/12 ...)"));
        }
        if params_text(p) != first_params {
            return Err(Diagnostic::error(p.span, format!("`then` makes one `{}` part, so its operands share their parameters: this one has {}, the first {}", first.role.text, show_params(&params_text(p)), show_params(&first_params)))
                .help("give every operand the same parameters (vel= oct= len= inst= ...)"));
        }
    }
    let terms: Vec<Term> = parts.iter().map(|p| operand_term(&p.pattern)).collect();
    let last = &parts[parts.len() - 1];
    let alt = Term { span, atom: Atom::Alt(terms), fast: None, accent: false, slide: false, marks: Marks::default() };
    Ok(Part { span: first.span.to(last.span), role: first.role.clone(), cycle: first.cycle, pattern: Seq { span: first.pattern.span.to(last.pattern.span), terms: vec![alt] }, params: first.params.clone(), right: false })
}

/// What an alternation holds for one operand: its only term, or its pattern squeezed into one slot.
fn operand_term(seq: &Seq) -> Term {
    if seq.terms.len() == 1 {
        seq.terms[0].clone()
    } else {
        Term { span: seq.span, atom: Atom::Group(vec![seq.clone()]), fast: None, accent: false, slide: false, marks: Marks::default() }
    }
}

/// A part's parameters as canonical text (sorted, as `print::part` writes them).
fn params_text(p: &Part) -> String {
    let full = print::part(p);
    let head = format!("{}{} {}", p.role.text, crate::meter::cycle_suffix(p), print::seq(&p.pattern));
    full.strip_prefix(&head).unwrap_or("").trim().to_string()
}

fn show_params(t: &str) -> String {
    if t.is_empty() {
        "none".into()
    } else {
        format!("`{t}`")
    }
}

// ---- lowering: the timeline

/// A section on the timeline: where one of its plays.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placement {
    pub name: String,
    /// The name's token where the expression wrote it.
    pub span: Span,
    pub start: u32,
    pub bars: u32,
}

impl Placement {
    pub(crate) fn end(&self) -> u32 {
        self.start.saturating_add(self.bars)
    }
}

/// A stretch of the song over which the same placements play: one segment of the compiled song.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stretch {
    pub start: u32,
    pub bars: u32,
    /// Indices into [`Plan::placements`], in placement order.
    pub layers: Vec<usize>,
    /// Not the first stretch of its item of the `arrange`: a continuation (the runtime reads a chain of stretches as one
    /// section, `a+b` of 8 bars).
    pub chain: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub placements: Vec<Placement>,
    pub stretches: Vec<Stretch>,
    /// For each placement, the stretches it plays over, in order.
    pub covers: Vec<Vec<usize>>,
    /// Where each item of the `arrange` begins, in bars (ascending).
    pub item_starts: Vec<u32>,
    /// The `arrange` statement (empty when the song is the sections in the order written).
    pub span: Span,
}

impl Plan {
    /// The stretches a placement plays over, in order.
    pub(crate) fn covered(&self, i: usize) -> &[usize] {
        &self.covers[i]
    }

    /// The name a stretch goes by: its layers' names, joined (`a`, or `drums+bass`).
    pub(crate) fn stretch_name(&self, k: usize) -> String {
        self.stretches[k].layers.iter().map(|&i| self.placements[i].name.as_str()).collect::<Vec<_>>().join("+")
    }

    /// The segment for stretch `k`: its name, place, and which `section ... {` lines to light (the header the playhead
    /// sits on is the layer that plays longest; the others are lit as lines).
    pub(crate) fn segment(&self, k: usize, sections: &BTreeMap<String, SectionInfo>) -> SegmentLines {
        let s = &self.stretches[k];
        let primary = s.layers.iter().copied().max_by_key(|&i| (self.placements[i].end(), std::cmp::Reverse(i))).unwrap_or(s.layers[0]);
        let header = sections.get(&self.placements[primary].name).map(|x| x.span);
        let header_more = s.layers.iter().filter(|&&i| i != primary).filter_map(|&i| sections.get(&self.placements[i].name).map(|x| x.span)).collect();
        let item = self.item_starts.iter().copied().filter(|&i| i <= s.start).max().unwrap_or(0);
        SegmentLines { name: self.stretch_name(k), start: s.start, bars: s.bars, lines: Vec::new(), header, key: None, chain: s.chain, header_more, phase: if s.chain { s.start - item } else { 0 } }
    }
}

struct PlaceCx<'a, 'b> {
    sections: &'a BTreeMap<String, SectionInfo<'b>>,
    lets: &'a BTreeMap<String, SExpr>,
    out: Vec<Placement>,
    over: bool,
    /// The `arrange` statement: where "too many placements" is said (empty: at the name that went over).
    at: Span,
}

impl Lower<'_> {
    /// The names a section expression uses must be sections or lets above it (`known`); `later` are lets below (said
    /// so), `bad` lets that did not read (their use is not said again). Errors name the nearest.
    pub(crate) fn check_section_names(&mut self, e: &SExpr, sections: &BTreeMap<String, SectionInfo>, known: &BTreeMap<String, SExpr>, later: &BTreeSet<String>, bad: &BTreeSet<String>) -> bool {
        let mut ok = true;
        for w in e.names() {
            if sections.contains_key(&w.text) || known.contains_key(&w.text) {
                continue;
            }
            ok = false;
            if bad.contains(&w.text) || self.declined.contains(&w.text) {
                continue;
            }
            if later.contains(&w.text) {
                self.err(Diagnostic::error(w.span, format!("`{}` is a let below this line: a let can only use sections and the lets above it", w.text)).help("move that let up"));
                continue;
            }
            let mut all: Vec<String> = sections.keys().cloned().collect();
            all.extend(known.keys().cloned());
            let mut d = Diagnostic::error(w.span, format!("there is no section `{}`", w.text)).expected(&all.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{}`", w.text));
            // (the words of a composition are no sections, and a typo of one of them is what this is: `silenc`)
            let mut cands = all.clone();
            cands.extend(["silence", "stack", "then"].iter().map(|s| s.to_string()));
            if let Some(n) = crate::env::suggest(&w.text, &cands, 1).first() {
                d = d.help(format!("did you mean `{n}`?"));
            }
            self.err(d);
        }
        ok
    }

    /// The `let NAME = sexpr` statements, in order, with their names checked: the ones that read (by name) and the ones
    /// that did not.
    pub(crate) fn section_lets(&mut self, decls: &[&LetSection], sections: &BTreeMap<String, SectionInfo>) -> (BTreeMap<String, SExpr>, BTreeSet<String>) {
        let mut known: BTreeMap<String, SExpr> = BTreeMap::new();
        let mut bad: BTreeSet<String> = BTreeSet::new();
        let mut later: BTreeSet<String> = decls.iter().map(|l| l.name.text.clone()).collect();
        for l in decls {
            // a let naming itself is "below": it is not known yet (`later` still holds it)
            if self.check_section_names(&l.expr, sections, &known, &later, &bad) {
                known.insert(l.name.text.clone(), l.expr.clone());
            } else {
                bad.insert(l.name.text.clone());
            }
            later.remove(&l.name.text);
        }
        (known, bad)
    }

    /// The arrangement as a plan: every section placed, the song cut into stretches. `places` (for `after NAME`,
    /// `before NAME N bars`, `end after NAME`) gets each placement. A plain `arrange a b c` is one placement a stretch.
    pub(crate) fn plan_arrangement(&mut self, items: &[SExpr], span: Span, sections: &BTreeMap<String, SectionInfo>, lets: &BTreeMap<String, SExpr>, said: bool) -> Plan {
        let mut cx = PlaceCx { sections, lets, out: Vec::new(), over: false, at: span };
        let mut at = 0u32;
        let mut item_starts = Vec::new();
        for it in items {
            item_starts.push(at);
            let b = self.place_expr(it, at, 0, &mut cx);
            at = at.saturating_add(b);
        }
        let over = cx.over;
        let placements = cx.out;
        if at > crate::expand::MAX_SONG_BARS {
            self.err(Diagnostic::error(span, format!("the song is {at} bars; the most is {}", crate::expand::MAX_SONG_BARS)));
        }
        if said && placements.is_empty() && !over && items.iter().all(|i| i.names().is_empty()) {
            self.err(Diagnostic::error(span, "this `arrange` has no sections in it: `silence` is the empty song").help("arrange intro build drop"));
        }
        // cut the song where a placement starts or ends, and where an item of the arrange does
        let mut cuts: BTreeSet<u32> = BTreeSet::new();
        for p in &placements {
            cuts.insert(p.start);
            cuts.insert(p.end());
        }
        for s in &item_starts {
            if *s < at {
                cuts.insert(*s);
            }
        }
        let cuts: Vec<u32> = cuts.into_iter().collect();
        // sweep the cuts left to right with the placements that are playing (a placement is in the sweep from its start
        // to its end): a plain list has one at a time, so this is linear in the song
        let mut by_start: Vec<usize> = (0..placements.len()).collect();
        by_start.sort_by_key(|&i| (placements[i].start, i));
        let starts_of_items: BTreeSet<u32> = item_starts.iter().copied().collect();
        let mut next = 0;
        let mut active: Vec<usize> = Vec::new();
        let mut stretches = Vec::new();
        let mut covers: Vec<Vec<usize>> = vec![Vec::new(); placements.len()];
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            active.retain(|&i| placements[i].end() > a);
            while next < by_start.len() && placements[by_start[next]].start <= a {
                active.push(by_start[next]);
                next += 1;
            }
            let mut layers = active.clone();
            layers.sort_unstable();
            layers.retain(|&i| placements[i].start <= a && a < placements[i].end());
            if layers.is_empty() {
                continue;
            }
            for &i in &layers {
                covers[i].push(stretches.len());
            }
            stretches.push(Stretch { start: a, bars: b - a, layers, chain: !starts_of_items.contains(&a) });
        }
        for p in &placements {
            self.places.entry(p.name.clone()).or_default().push((p.start, p.bars));
        }
        Plan { placements, stretches, covers, item_starts, span }
    }

    /// Place `e` starting at bar `at`; answers how many bars it is. Unknown names were reported already.
    fn place_expr(&mut self, e: &SExpr, at: u32, depth: usize, cx: &mut PlaceCx) -> u32 {
        if cx.over {
            return 0;
        }
        match e {
            SExpr::Silence(_) => 0,
            SExpr::Name(w) => {
                if let Some(s) = cx.sections.get(&w.text) {
                    if cx.out.len() >= MAX_PLACEMENTS {
                        cx.over = true;
                        // (at the `arrange`, not at the name deep inside a `let` that was the one that went over)
                        let at = if cx.at == Span::default() { w.span } else { cx.at };
                        self.err(Diagnostic::error(at, format!("this arrangement places sections more than {MAX_PLACEMENTS} times")).help("fewer or shorter compositions: a `let` that names the one before it twice doubles the placements each time"));
                        return 0;
                    }
                    cx.out.push(Placement { name: w.text.clone(), span: w.span, start: at, bars: s.block.bars });
                    return s.block.bars;
                }
                let lets = cx.lets;
                if let Some(x) = lets.get(&w.text) {
                    if depth >= MAX_NEST * 8 {
                        cx.over = true;
                        self.err(Diagnostic::error(w.span, "lets nest too deep").help(format!("a chain of lets is at most {} long when each is a `then` or `stack` of the one before (a hop costs two of the {} levels): name the part with fewer lets", MAX_NEST * 4, MAX_NEST * 8)));
                        return 0;
                    }
                    return self.place_expr(x, at, depth + 1, cx);
                }
                0
            }
            SExpr::Then(xs, _) => {
                let mut acc = 0u32;
                for x in xs {
                    acc = acc.saturating_add(self.place_expr(x, at.saturating_add(acc), depth + 1, cx));
                }
                acc
            }
            SExpr::Stack(xs, _) => {
                let mut longest = 0u32;
                for x in xs {
                    longest = longest.max(self.place_expr(x, at, depth + 1, cx));
                }
                longest
            }
        }
    }

    /// The lines of every placed section: each placement's body is expanded once, at its own bar; its part lines play
    /// in every stretch it covers (phased to the bar of the section the stretch begins at), its controls fire once, in
    /// the first. Two placements playing together may not both define one deck's role: the error names both.
    pub(crate) fn layer_lines(&mut self, plan: &Plan, sections: &BTreeMap<String, SectionInfo>, out: &mut Lowered) {
        let mut per: Vec<Vec<FlatLine>> = Vec::new();
        // F5: the `key` a placement's section sets (the head of its body), read once per placement
        let mut keys: Vec<Option<Word>> = Vec::new();
        for (i, p) in plan.placements.iter().enumerate() {
            let covered = plan.covered(i);
            let Some(&first) = covered.first() else {
                per.push(Vec::new());
                keys.push(None);
                continue;
            };
            let sec = &sections[&p.name];
            let (key, keyed) = self.section_keys(&sec.block.body);
            keys.push(key);
            let alone = covered.len() == 1 && plan.stretches[first].layers.len() == 1 && plan.stretches[first].start == p.start && plan.stretches[first].bars == p.bars;
            let scope = Scope { base: Some(p.start), depth: 0, path: format!("[{}@{}] ", p.name, p.start), segment: Some(first), via: Vec::new(), timed: false };
            let mut lines = Vec::new();
            for st in sec.block.body.iter().skip(keyed) {
                if !alone && matches!(st.kind, StmtKind::Evolve(_)) {
                    self.err(Diagnostic::error(st.span, format!("section `{}` has an `evolve` and plays together with another section", p.name)).help("an evolve rewrites its section's nets generation by generation, so the section plays alone: put it in a `then`, not in a `stack`"));
                    continue;
                }
                self.stmt(st, &scope, &mut lines, &mut out.segments);
            }
            per.push(lines);
        }
        self.stretch_keys(plan, &keys, out);
        let dropped = self.stack_conflicts(plan, &per);
        for (i, lines) in per.into_iter().enumerate() {
            let covered = plan.covered(i);
            let Some(&first) = covered.first() else { continue };
            for (j, fl) in lines.into_iter().enumerate() {
                if dropped.contains(&(i, j)) {
                    continue;
                }
                for &k in &covered[1..] {
                    if !self.budget(fl.span) {
                        break;
                    }
                    let mut carried = fl.clone();
                    carried.phase = Some(plan.stretches[k].start - plan.placements[i].start);
                    out.segments[k].lines.push(carried);
                }
                out.segments[first].lines.push(fl);
            }
        }
        // a section placed again and again writes its lines out again each time: if that is what went over the line cap,
        // the arrangement is the place to look, not a `repeat` (there may be none) or the section's own line
        if plan.span != Span::default() && plan.placements.len() > sections.len() {
            for d in self.diags.iter_mut().filter(|d| d.message.starts_with("this program expands to more than")) {
                d.span = plan.span;
                d.message = format!("this arrangement expands to more than {} lines: a section's lines are written out again each time it is placed", crate::expand::MAX_LINES);
                d.help = Some("fewer placements: a shorter `arrange`, or compositions (`let`s) that do not name the one before twice".to_string());
            }
        }
    }

    /// The key of every stretch (F5 meets F10): a stretch sounds in ONE key, the key of the first section playing in it
    /// that sets one (in the order of the normal form), else the program's. Two sections that play at the same time and
    /// set different keys are an error naming both (two keys cannot sound at once); two that set the same key agree. A
    /// section that plays in several stretches (a longer one stacked with a shorter) is in its own key all through; one
    /// that sets none follows the stretch: the key of whatever keyed sections play with it there.
    fn stretch_keys(&mut self, plan: &Plan, keys: &[Option<Word>], out: &mut Lowered) {
        // (once for a pair of `key` statements, however many stretches and items the sections meet in)
        let mut said: BTreeSet<(usize, usize, usize, usize)> = BTreeSet::new();
        for (k, st) in plan.stretches.iter().enumerate() {
            let mut first: Option<(usize, &Word)> = None;
            for &i in &st.layers {
                let Some(w) = &keys[i] else { continue };
                match first {
                    None => first = Some((i, w)),
                    Some((j, fw)) => {
                        // (a `key` that is not a key is the compiler's error; two that parse and differ clash)
                        if let (Some(a), Some(b)) = (crate::music::Key::parse(&fw.text), crate::music::Key::parse(&w.text)) {
                            if a != b && said.insert((fw.span.start, fw.span.end, w.span.start, w.span.end)) {
                                let at = crate::diag::line_col(self.src, fw.span.start).0;
                                let (na, nb) = (&plan.placements[j].name, &plan.placements[i].name);
                                self.err(
                                    Diagnostic::error(w.span, format!("sections `{na}` and `{nb}` play at the same time, in different keys: `key {}` (line {at}) and `key {}`", fw.text, w.text))
                                        .help("a stretch of a song sounds in one key: give them the same key, or play them one after the other with then(...)"),
                                );
                            }
                        }
                    }
                }
            }
            if let Some((_, w)) = first {
                out.segments[k].key = Some(w.clone());
            }
        }
    }

    /// Parts two placements that play at the same time both define (same deck, same role): an error naming both. Answers
    /// the lines to leave out, `(placement, line)`, so the compiler does not say it a second time.
    fn stack_conflicts(&mut self, plan: &Plan, per: &[Vec<FlatLine>]) -> BTreeSet<(usize, usize)> {
        let mut dropped = BTreeSet::new();
        let mut said: BTreeSet<(usize, usize, Deck, String)> = BTreeSet::new();
        for st in &plan.stretches {
            let mut owner: BTreeMap<(Deck, String), (usize, Span)> = BTreeMap::new();
            for &i in &st.layers {
                for (j, fl) in per[i].iter().enumerate() {
                    let Body::Parts(ps) = &fl.line.body else { continue };
                    let Some((Target::Deck(d), _)) = fl.line.target else { continue };
                    for part in ps {
                        match owner.get(&(d, part.role.text.clone())) {
                            Some(&(other, first_at)) if other != i => {
                                dropped.insert((i, j));
                                if said.insert((other, i, d, part.role.text.clone())) {
                                    let (a, b) = (&plan.placements[other].name, &plan.placements[i].name);
                                    let at = crate::diag::line_col(self.src, first_at.start).0;
                                    let who = if a == b { format!("section `{a}` is stacked with itself, and it defines") } else { format!("sections `{a}` and `{b}` play at the same time, and both define") };
                                    self.err(
                                        Diagnostic::error(part.role.span, format!("{who} deck {}'s `{}` (the other is on line {at})", d.name(), part.role.text))
                                            .help("a deck has one part per role at a time: give one of them another role or deck, or play them one after the other with then(...)"),
                                    );
                                }
                            }
                            Some(_) => {}
                            None => {
                                owner.insert((d, part.role.text.clone()), (i, part.role.span));
                            }
                        }
                    }
                }
            }
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> SExpr {
        SExpr::Name(Word { text: s.into(), span: Span::default() })
    }
    fn sp() -> Span {
        Span::default()
    }
    fn st(xs: Vec<SExpr>) -> SExpr {
        SExpr::Stack(xs, sp())
    }
    fn th(xs: Vec<SExpr>) -> SExpr {
        SExpr::Then(xs, sp())
    }
    fn sil() -> SExpr {
        SExpr::Silence(sp())
    }

    #[test]
    fn then_flattens_drops_silence_and_collapses() {
        assert_eq!(SExpr::then(vec![n("a"), th(vec![n("b"), n("c")])], sp()).canonical(), "then(a, b, c)");
        assert_eq!(SExpr::then(vec![th(vec![n("a"), n("b")]), n("c")], sp()).canonical(), "then(a, b, c)");
        assert_eq!(SExpr::then(vec![n("a"), sil(), n("b")], sp()).canonical(), "then(a, b)");
        assert_eq!(SExpr::then(vec![n("a"), sil()], sp()).canonical(), "a");
        assert_eq!(SExpr::then(vec![sil(), sil()], sp()).canonical(), "silence");
        assert_eq!(SExpr::then(vec![], sp()).canonical(), "silence");
        // order is meaning
        assert_eq!(SExpr::then(vec![n("b"), n("a")], sp()).canonical(), "then(b, a)");
        // a then inside a stack stays
        assert_eq!(SExpr::then(vec![n("a"), st(vec![n("b"), n("c")])], sp()).canonical(), "then(a, stack(b, c))");
    }

    #[test]
    fn stack_flattens_sorts_and_keeps_every_copy() {
        assert_eq!(SExpr::stack(vec![n("b"), n("a")], sp()).canonical(), "stack(a, b)");
        assert_eq!(SExpr::stack(vec![n("c"), st(vec![n("b"), n("a")])], sp()).canonical(), "stack(a, b, c)");
        assert_eq!(SExpr::stack(vec![n("a"), n("a")], sp()).canonical(), "stack(a, a)");
        assert_eq!(SExpr::stack(vec![sil(), n("a")], sp()).canonical(), "a");
        assert_eq!(SExpr::stack(vec![sil()], sp()).canonical(), "silence");
        // sorted by the operand's canonical text: a name, then a `then(...)`
        assert_eq!(SExpr::stack(vec![n("z"), th(vec![n("a"), n("b")]), n("m")], sp()).canonical(), "stack(m, then(a, b), z)");
    }

    #[test]
    fn normalising_twice_is_normalising_once() {
        let messy = st(vec![n("z"), th(vec![th(vec![n("a")]), sil(), st(vec![n("c"), st(vec![n("b")])])]), sil()]);
        let once = messy.normalized();
        assert_eq!(once.canonical(), "stack(then(a, stack(b, c)), z)");
        assert_eq!(once.normalized(), once);
    }

    #[test]
    fn an_arrange_of_names_stays_the_plain_list() {
        assert!(matches!(arrange_kind(SExpr::then(vec![n("a"), n("b"), n("c")], sp())), StmtKind::Arrange(w) if w.len() == 3));
        assert!(matches!(arrange_kind(n("a")), StmtKind::Arrange(w) if w.len() == 1));
        assert!(matches!(arrange_kind(SExpr::then(vec![n("a"), st(vec![n("b"), n("c")])], sp())), StmtKind::ArrangeExpr(i) if i.len() == 2));
        assert!(matches!(arrange_kind(sil()), StmtKind::ArrangeExpr(i) if i.len() == 1));
        let items = SExpr::then(vec![n("a"), st(vec![n("c"), n("b")])], sp()).normalized().items();
        assert_eq!(arrange_text(&items), "arrange a stack(b, c)");
    }

    #[test]
    fn the_section_names_of_an_expression_in_reading_order() {
        let e = SExpr::then(vec![n("a"), st(vec![n("c"), n("b")]), sil(), n("d")], sp()).normalized();
        assert_eq!(e.names().iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_let_of_a_composition_that_is_a_lone_name_is_rewritten_to_it() {
        let toks = |s: &str| crate::lex::lex(s).tokens;
        let text = |v: Vec<crate::lex::Token>| crate::print::tokens(v.iter().map(|t| &t.tok));
        assert_eq!(text(collapse_lets(toks("let x = then(a, silence)\n"))), "let x=a");
        assert_eq!(text(collapse_lets(toks("let x = stack(then(a))\n"))), "let x=a");
        assert_eq!(text(collapse_lets(toks("let x = then(silence)\n"))), "let x=silence");
        // a real composition, a lone name and a pattern are left alone
        assert_eq!(text(collapse_lets(toks("let x = then(a, b)\n"))), "let x=then(a, b)");
        assert_eq!(text(collapse_lets(toks("let x = a\n"))), "let x=a");
        assert_eq!(text(collapse_lets(toks("let r = x ~ x ~\n"))), "let r=x ~ x ~");
        // a let inside a block is not at the top: untouched
        assert_eq!(text(collapse_lets(toks("def f() {\n  let x = then(a)\n}\n"))), "def f() { let x=then(a) }");
    }
}
