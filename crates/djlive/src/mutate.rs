//! `arrange { ... }`: an arrangement that changes the program as it goes (docs/BEATSCRIPT.md "Arrange mutations").
//!
//! ```text
//! arrange {
//!   intro build drop
//!   ~ acid_a rot 2, hoov_a up 3        # persistent: rebinds the pattern from here on
//!   wubs build2
//!   ~ acid_* drift 0.15                # a glob names several patterns
//!   peak
//!   @ hoov_b rev                       # one-shot: the next section only, then it reverts
//!   ~3 acid_a rev                      # every 3rd pass of the block only (a count right after the mark)
//!   ~ control off                      # the control deck: its lines do not fire in the sections from here on
//!   outro
//! } loop 4 then outro end
//! ```
//!
//! A switch of the control deck (`~ control off`, `@ control freeze on`: docs/beatscript/ext-control.md "Per pass: the switch") is a
//! line of the block too: it rewrites no pattern, and a section a play reads with control-deck lines held back is lowered as a
//! copy of its own ([`crate::ast::SectionBlock::quiet`]), whose held-back lines expand.rs keeps out of every segment.
//!
//! The block is a path through the program's state: a `~` mutation rewrites the **text** of the `let` patterns it names
//! (their token lists), and every later section reads the new text; a `@` mutation does so for the next section only. The
//! pass ([`apply`]) is a desugaring, done when the program is checked and never seen by the canonical form: each section
//! the block plays is lowered with the generation of each pattern it reads, as a plain `let` (`acid_a@3`) and, where two
//! plays of a section read different generations, a copy of the section (`drop@2`). The rest of the language (expand,
//! compile, runtime) sees a plain `arrange a b c`. The table of what each play read is [`generations`].
//!
//! Deterministic: the only randomness is a hash of (program CID, pass, mutation, target, operator), so the same text is
//! the same generations, CID for CID. No clock, no global RNG.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lex::{Tok, Token};
use crate::music::{Chord, Key};
use crate::parse::{Parsed, Parser};

type R<T> = Result<T, Diagnostic>;

// ---- the syntax

/// `arrange { items } ending`, or `arrange NAME: { items } ending`: a named arrangement (flow.rs) whose sections are
/// played with mutations. A named block ends as a named arrangement does (`then NAME`, `loop`, `loop N`, `end`, or
/// nothing), and its mutations run over one pass of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrangeBlock {
    pub items: Vec<ArrItem>,
    pub ends: BlockEnd,
    pub span: Span,
    /// `arrange NAME: { ... }`: its name and how it ends (then `ends` is `End`).
    pub named: Option<(Word, crate::ast::ArrEnd)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArrItem {
    /// A section played.
    Section(Word),
    /// `~ target ops, target ops` (persistent) or `@ target ops` (one-shot).
    Mutation(Mutation),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mutation {
    /// `@`: for the next section only.
    pub once: bool,
    /// `~3` / `@4`: fires on every Nth pass of the block (the 3rd, 6th, 9th ...); 1, the default, on every pass.
    pub every: u32,
    /// The written count's span, for what it can be said about (`~3` in a block that has no third pass).
    pub every_span: Option<Span>,
    /// The patterns it rewrites; empty for a switch of the control deck.
    pub targets: Vec<MutTarget>,
    /// `~ control off`, `@ control freeze on`: the line switches the control deck instead (docs/beatscript/ext-control.md). A line of its
    /// own: it has no targets.
    pub switch: Option<Switch>,
    pub span: Span,
}

/// `control [CALL] on|off`: whether the control deck's lines fire in the sections after it (`~`) or in the next one (`@`): all
/// of them, or the lines of one call (`~ control freeze off`: [`crate::ast::Body::call_name`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Switch {
    pub call: Option<Word>,
    pub on: bool,
    /// `control` to the `on` / `off`.
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MutTarget {
    /// A `let` name, or a glob (`acid_*`).
    pub name: Word,
    pub ops: Vec<MutOp>,
}

impl MutTarget {
    pub fn is_glob(&self) -> bool {
        self.name.text.contains('*')
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MutOp {
    pub name: Word,
    pub args: Vec<MutArg>,
    /// `jux rev`: the operator the right copy gets (`args` is then empty).
    pub inner: Option<Box<MutOp>>,
    /// The operator and its arguments.
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MutArg {
    Num(f64, Span),
    Word(Word),
}

impl MutArg {
    fn span(&self) -> Span {
        match self {
            MutArg::Num(_, s) => *s,
            MutArg::Word(w) => w.span,
        }
    }
    fn text(&self) -> String {
        match self {
            MutArg::Num(v, _) => crate::print::num(*v),
            MutArg::Word(w) => w.text.clone(),
        }
    }
}

/// How the block ends, written after its `}`.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockEnd {
    /// `end`, or nothing: the block plays once.
    End,
    /// `loop`: for ever, each pass a new generation.
    Loop,
    /// `loop N then end` / `loop N then outro end`: N passes, then the coda (read with the last generation), then the end.
    Passes { n: u32, span: Span, coda: Vec<Word> },
}

impl MutOp {
    pub fn text(&self) -> String {
        let mut s = self.name.text.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(&a.text());
        }
        if let Some(inner) = &self.inner {
            s.push(' ');
            s.push_str(&inner.text());
        }
        s
    }
}

/// The canonical text of a block: section names on a line, a mutation a line, the ending after the `}` (`end`, the
/// default, is left out: written or not it is one program).
pub fn print_block(b: &ArrangeBlock) -> String {
    let mut out = match &b.named {
        Some((name, _)) => format!("arrange {}: {{", name.text),
        None => String::from("arrange {"),
    };
    let mut names: Vec<String> = Vec::new();
    let flush = |out: &mut String, names: &mut Vec<String>| {
        if !names.is_empty() {
            out.push_str("\n  ");
            out.push_str(&names.join(" "));
            names.clear();
        }
    };
    for it in &b.items {
        match it {
            ArrItem::Section(w) => names.push(w.text.clone()),
            ArrItem::Mutation(m) => {
                flush(&mut out, &mut names);
                out.push_str("\n  ");
                out.push_str(&mutation_text(m));
            }
        }
    }
    flush(&mut out, &mut names);
    out.push_str("\n}");
    if let Some((_, ends)) = &b.named {
        out.push_str(&crate::print::arr_end(ends));
        return out;
    }
    match &b.ends {
        BlockEnd::End => {}
        BlockEnd::Loop => out.push_str(" loop"),
        BlockEnd::Passes { n, coda, .. } => {
            out.push_str(&format!(" loop {n} then"));
            for w in coda {
                out.push(' ');
                out.push_str(&w.text);
            }
            out.push_str(" end");
        }
    }
    out
}

pub fn mutation_text(m: &Mutation) -> String {
    // the count is glued to the mark, and left out when it is 1, the default: `~1 x` and `~ x` are one program
    let every = if m.every > 1 { m.every.to_string() } else { String::new() };
    let mark = if m.once { "@" } else { "~" };
    if let Some(s) = &m.switch {
        let call = s.call.as_ref().map(|c| format!(" {}", c.text)).unwrap_or_default();
        return format!("{mark}{every} control{call} {}", if s.on { "on" } else { "off" });
    }
    let targets: Vec<String> = m.targets.iter().map(|t| format!("{} {}", t.name.text, t.ops.iter().map(MutOp::text).collect::<Vec<_>>().join(" | "))).collect();
    format!("{mark}{every} {}", targets.join(", "))
}

impl<'a> Parser<'a> {
    /// After `arrange`, at its `{`.
    pub(crate) fn arrange_block(&mut self, start: Span) -> R<StmtKind> {
        let items = self.arrange_block_items(start)?;
        let ends = self.block_end()?;
        Ok(StmtKind::ArrangeBlock(ArrangeBlock { items, ends, span: start.to(self.prev_span()), named: None }))
    }

    /// After `arrange NAME:`, at its `{`: a named arrangement with mutations, ended as a named arrangement is.
    pub(crate) fn named_arrange_block(&mut self, start: Span, name: Word) -> R<StmtKind> {
        let items = self.arrange_block_items(start)?;
        let ends = self.arr_end()?;
        Ok(StmtKind::ArrangeBlock(ArrangeBlock { items, ends: BlockEnd::End, span: start.to(self.prev_span()), named: Some((name, ends)) }))
    }

    /// The `{ ... }` of an `arrange` block: its sections and mutations.
    fn arrange_block_items(&mut self, start: Span) -> R<Vec<ArrItem>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut items = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Newline | Tok::Comma => {
                    self.bump();
                }
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => return Err(Diagnostic::error(start, "the `{` of `arrange` is never closed").help("end the block with `}`")),
                Tok::Tilde | Tok::At => items.push(ArrItem::Mutation(self.mutation()?)),
                Tok::Word(_) => items.push(ArrItem::Section(self.word("a section name")?)),
                _ => {
                    return Err(self
                        .expected(&["a section name", "`~`", "`@`", "`}`"], &format!("in `arrange {{ }}`, expected a section name or a mutation, found {}", self.found()))
                        .help("a line of an `arrange { }` is section names (`intro build drop`), `~ name op` (from here on) or `@ name op` (the next section only)"))
                }
            }
        }
        if !items.iter().any(|i| matches!(i, ArrItem::Section(_))) {
            return Err(Diagnostic::error(start, "an `arrange { }` needs the sections it plays").help("arrange {\n  intro drop\n}"));
        }
        Ok(items)
    }

    fn mutation(&mut self) -> R<Mutation> {
        let at = self.bump();
        let once = at.tok == Tok::At;
        let mark = if once { "@" } else { "~" };
        let (every, every_span) = self.mut_every(mark)?;
        if let Some(switch) = self.switch(mark)? {
            if self.peek() == &Tok::Comma {
                return Err(own_line(self.span(), mark));
            }
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
                return Err(self.expected(&["the end of the line"], &format!("a switch of the control deck ends its line, found {}", self.found())).help(format!("{mark} control off   {mark} control freeze on")));
            }
            return Ok(Mutation { once, every, every_span, targets: Vec::new(), switch: Some(switch), span: at.span.to(self.prev_span()) });
        }
        let mut targets = vec![self.mut_target()?];
        while self.eat(&Tok::Comma) {
            if self.switch_shape().is_some() {
                return Err(own_line(self.span(), mark));
            }
            targets.push(self.mut_target()?);
        }
        if !matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
            return Err(self.expected(&["`|`", "`,`", "the end of the line"], &format!("expected `|` (another operator), `,` (another target) or the end of the line, found {}", self.found())));
        }
        Ok(Mutation { once, every, every_span, targets, switch: None, span: at.span.to(self.prev_span()) })
    }

    /// `control off` / `control on` / `control CALL off` after the mark: the control deck's switch (docs/beatscript/ext-control.md), as
    /// many tokens as it takes. `control` with an operator after it is a mutation of a `let` that is called `control` (`~
    /// control rot 2`): it is no reserved word, and `on` and `off` are no operators, so no program that read before reads
    /// otherwise now.
    fn switch_shape(&self) -> Option<usize> {
        if !self.word_is("control") {
            return None;
        }
        let word = |k: usize| match self.peek_n(k) {
            Tok::Word(w) => Some(w.as_str()),
            _ => None,
        };
        match (word(1), word(2)) {
            (Some("on" | "off"), _) => Some(2),
            (Some(call), Some("on" | "off")) if op_spec(call).is_none() => Some(3),
            _ => None,
        }
    }

    /// The control deck's switch, if the line is one; an error where it starts like one and does not say `on` or `off`.
    fn switch(&mut self, mark: &str) -> R<Option<Switch>> {
        let Some(n) = self.switch_shape() else {
            // `~ control` and `~ control freeze`, the line ending there: a switch with its `on` or `off` left out
            let ends = |t: &Tok| matches!(t, Tok::Newline | Tok::RBrace | Tok::Eof | Tok::Comma);
            let bare = self.word_is("control") && (ends(self.peek_n(1)) || matches!(self.peek_n(1), Tok::Word(w) if op_spec(w).is_none()) && ends(self.peek_n(2)));
            if bare {
                return Err(Diagnostic::error(self.span(), "a switch of the control deck says `on` or `off`").help(format!("{mark} control off   (every line of the control deck)   {mark} control freeze on   (its `freeze` lines)")));
            }
            return Ok(None);
        };
        let start = self.bump().span;
        let call = if n == 3 { Some(self.word("the call whose lines it switches")?) } else { None };
        let on = self.word_is("on");
        let end = self.bump().span;
        Ok(Some(Switch { call, on, span: start.to(end) }))
    }

    /// `~3` / `@4`: a whole number right after the mark (spaced or glued, the same program) is how often the line
    /// fires, every Nth pass of the block. A name never starts with a digit, so it cannot be a target.
    fn mut_every(&mut self, mark: &str) -> R<(u32, Option<Span>)> {
        let Tok::Num { value, int } = self.peek().clone() else { return Ok((1, None)) };
        let sp = self.bump().span;
        if !int {
            return Err(Diagnostic::error(sp, format!("`{mark}N` counts whole passes: `{mark}3` is every 3rd pass")).help(format!("{mark}3 acid rev")));
        }
        if value < 1.0 {
            return Err(Diagnostic::error(sp, format!("`{mark}0` never fires: the passes are counted from 1")).help(format!("`{mark}3` is every 3rd pass, a name alone (`{mark} acid rev`) is every pass")));
        }
        if value > 256.0 {
            return Err(Diagnostic::error(sp, format!("`{mark}N` is 1 to 256, like `loop N`: a loop has no more passes than that")).help(format!("{mark}3 acid rev")));
        }
        Ok((value as u32, Some(sp)))
    }

    fn mut_target(&mut self) -> R<MutTarget> {
        // a name, or a glob: words and `*` written together (`acid_*`, `*_a`)
        let first = self.span();
        if !matches!(self.peek(), Tok::Word(_) | Tok::Star) {
            return Err(self.expected(&["a pattern name"], &format!("a mutation names the `let` pattern it changes, found {}", self.found())).help("~ acid rot 2"));
        }
        let mut text = String::new();
        let mut span = first;
        let mut end = first.start;
        loop {
            let sp = self.span();
            if !text.is_empty() && sp.start != end {
                break;
            }
            match self.peek().clone() {
                Tok::Word(w) => text.push_str(&w),
                Tok::Star => text.push('*'),
                _ => break,
            }
            self.bump();
            end = sp.end;
            span = span.to(sp);
        }
        let name = Word { text, span };
        let mut ops = vec![self.mut_op()?];
        while self.eat(&Tok::Pipe) {
            ops.push(self.mut_op()?);
        }
        Ok(MutTarget { name, ops })
    }

    fn mut_op(&mut self) -> R<MutOp> {
        let name = match self.peek().clone() {
            Tok::Word(_) => self.word("an operator")?,
            _ => return Err(self.expected(&["an operator"], &format!("expected an operator (`rot 2`, `thin 0.1`, `up 3`), found {}", self.found())).help(format!("the operators: {}", OPS.iter().map(|o| o.0).collect::<Vec<_>>().join(" ")))),
        };
        // `jux rev`: what the right copy gets is an operator with its own arguments (`jux rot 3`)
        if name.text == "jux" {
            if !matches!(self.peek(), Tok::Word(_)) {
                return Err(self.expected(&["an operator"], &format!("`jux` takes the operator the right copy gets, found {}", self.found())).help("~ lead jux rev   (the left copy stays, the right one is reversed)"));
            }
            let inner = self.mut_op()?;
            let span = name.span.to(inner.span);
            return Ok(MutOp { name, args: Vec::new(), inner: Some(Box::new(inner)), span });
        }
        let mut args = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Num { value, .. } => {
                    let sp = self.bump().span;
                    args.push(MutArg::Num(value, sp));
                }
                Tok::Minus | Tok::Plus if matches!(self.peek_n(1), Tok::Num { .. }) => {
                    let neg = self.bump();
                    let Tok::Num { value, .. } = self.peek().clone() else { unreachable!() };
                    let sp = self.bump().span;
                    args.push(MutArg::Num(if neg.tok == Tok::Minus { -value } else { value }, neg.span.to(sp)));
                }
                Tok::Word(_) => args.push(MutArg::Word(self.word("an argument")?)),
                _ => break,
            }
        }
        let span = args.last().map_or(name.span, |a| name.span.to(a.span()));
        Ok(MutOp { name, args, inner: None, span })
    }

    fn block_end(&mut self) -> R<BlockEnd> {
        if self.word_is("end") {
            self.bump();
            return Ok(BlockEnd::End);
        }
        if !self.word_is("loop") {
            return Ok(BlockEnd::End);
        }
        self.bump();
        let Tok::Num { value, int } = self.peek().clone() else {
            return Ok(BlockEnd::Loop);
        };
        let sp = self.bump().span;
        if !int || !(1.0..=256.0).contains(&value) {
            return Err(Diagnostic::error(sp, "`loop N` is 1 to 256 passes, a whole number"));
        }
        let mut coda = Vec::new();
        if self.word_is("then") {
            self.bump();
            while matches!(self.peek(), Tok::Word(w) if w != "end") {
                coda.push(self.word("a section name")?);
            }
            if !self.word_is("end") {
                return Err(self.expected(&["`end`"], &format!("`loop N then ...` ends with `end`, found {}", self.found())).help("} loop 4 then outro end"));
            }
            self.bump();
        }
        Ok(BlockEnd::Passes { n: value as u32, span: sp, coda })
    }
}

/// A switch of the control deck written beside a mutation, on one line with a comma: each goes on a line of its own.
fn own_line(at: Span, mark: &str) -> Diagnostic {
    Diagnostic::error(at, "a switch of the control deck is a line of its own, not a target beside a pattern").help(format!("{mark} acid rot 2\n  {mark} control off"))
}

// ---- the operators

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArgKind {
    /// A whole number, signed.
    Int,
    /// A share, 0 to 1.
    Prob,
    /// Another `let` pattern.
    Pat,
    /// An operator, with its own arguments (`jux rev`).
    Op,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Rhythm,
    Time,
    Pitch,
    Feel,
    Cross,
    Generational,
    Chord,
    Stereo,
    Identity,
}

use ArgKind::*;

/// The closed set: name, arguments, family. `id` is the identity (what the fixer writes in place of a bad operator).
const OPS: &[(&str, &[ArgKind], Family)] = &[
    ("rot", &[Int], Family::Rhythm),
    ("rev", &[], Family::Rhythm),
    ("thin", &[Prob], Family::Rhythm),
    ("fill", &[Prob], Family::Rhythm),
    ("euclid", &[Int], Family::Rhythm),
    ("swap", &[], Family::Rhythm),
    ("mirror", &[], Family::Rhythm),
    ("shuffle", &[], Family::Rhythm),
    ("half", &[], Family::Time),
    ("double", &[], Family::Time),
    ("stutter", &[Int], Family::Time),
    ("ratchet", &[Prob], Family::Time),
    ("swing", &[Prob], Family::Time),
    ("up", &[Int], Family::Pitch),
    ("down", &[Int], Family::Pitch),
    ("invert", &[], Family::Pitch),
    ("oct", &[Int], Family::Pitch),
    ("drift", &[Prob], Family::Pitch),
    ("vel", &[Int], Family::Feel),
    ("accent", &[Prob], Family::Feel),
    ("morph", &[Pat, Prob], Family::Cross),
    ("xfer", &[Pat], Family::Cross),
    ("mask", &[Pat], Family::Cross),
    ("grow", &[], Family::Generational),
    ("decay", &[], Family::Generational),
    ("voice", &[Int], Family::Chord),
    ("spread", &[], Family::Chord),
    ("jux", &[Op], Family::Stereo),
    ("id", &[], Family::Identity),
];

/// The operator names, for the docs and the editor.
pub fn operator_names() -> Vec<&'static str> {
    OPS.iter().map(|o| o.0).filter(|n| *n != "id").collect()
}

/// What an operator's arguments are, in order: `"int"` (a whole number, -64 to 64), `"share"` (0 to 1), `"pattern"`
/// (another `let` pattern) or `"operator"` (`jux`: the operator the right copy gets). For an editor: completion, hover and a number scrubber's limits. None: not an operator.
pub fn operator_args(name: &str) -> Option<Vec<&'static str>> {
    op_spec(name).filter(|_| name != "id").map(|(kinds, _)| kinds.iter().map(|k| match k { Int => "int", Prob => "share", Pat => "pattern", Op => "operator" }).collect())
}

fn op_spec(name: &str) -> Option<(&'static [ArgKind], Family)> {
    OPS.iter().find(|o| o.0 == name).map(|o| (o.1, o.2))
}

/// Deterministic randomness: splitmix64 seeded by FNV-1a of (CID, pass, mutation, target, operator).
struct Rng(u64);

impl Rng {
    fn new(cid: &str, pass: u32, mutation: usize, target: usize, op: usize) -> Rng {
        Rng::seeded(cid, pass, mutation, target, op, false)
    }
    /// The right copy's dice: the same hash with one more byte, so the left copy's stream (and every program without a
    /// `jux`) is the one it always was.
    fn new_right(cid: &str, pass: u32, mutation: usize, target: usize, op: usize) -> Rng {
        Rng::seeded(cid, pass, mutation, target, op, true)
    }
    fn seeded(cid: &str, pass: u32, mutation: usize, target: usize, op: usize, right: bool) -> Rng {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |b: &[u8]| {
            for x in b {
                h ^= u64::from(*x);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        eat(cid.as_bytes());
        eat(&pass.to_le_bytes());
        eat(&(mutation as u64).to_le_bytes());
        eat(&(target as u64).to_le_bytes());
        eat(&(op as u64).to_le_bytes());
        if right {
            eat(&[1]);
        }
        Rng(h)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn rest(span: Span) -> Term {
    Term { span, atom: Atom::Rest, fast: None, accent: false, slide: false, marks: Marks::default() }
}

fn sounds(t: &Term) -> bool {
    !matches!(t.atom, Atom::Rest)
}

/// The most steps a mutated pattern may have.
const MAX_STEPS: usize = 64;

/// What an operator works with besides the pattern: the key, and the other patterns (for the cross-pattern ones).
struct Ctx<'a> {
    key: Key,
    others: &'a BTreeMap<String, Seq>,
    /// The octave the pattern is played in: where its `chord(...)` steps are voiced (`voiced`).
    octave: i32,
}

/// A pitch an operator can move: a note with an octave (a MIDI number), a note with none (a pitch class, moved in its
/// octave-less form), a degree of the key (`3`), a written degree (`deg(b3 5')`).
enum Pitch {
    Midi(i32),
    Class(i32),
    Degree(i64),
}

const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

fn read_pitch(a: &Atom) -> Option<Pitch> {
    match a {
        Atom::Note(w) => match crate::music::parse_note(&w.text)? {
            (pc, Some(oct)) => Some(Pitch::Midi(crate::music::midi(pc, oct))),
            (pc, None) => Some(Pitch::Class(i32::from(pc))),
        },
        Atom::Degree(d) => Some(Pitch::Degree(i64::from(*d))),
        _ => None,
    }
}

fn write_pitch(p: Pitch, w: &Word) -> Option<Atom> {
    match p {
        Pitch::Midi(m) if (0..=127).contains(&m) => {
            let oct = m.div_euclid(12) - 1;
            if !(-1..=9).contains(&oct) {
                return None;
            }
            Some(Atom::Note(Word { text: format!("{}{}", SHARP[m.rem_euclid(12) as usize], oct), span: w.span }))
        }
        Pitch::Class(c) => Some(Atom::Note(Word { text: SHARP[c.rem_euclid(12) as usize].to_string(), span: w.span })),
        Pitch::Degree(d) => Some(Atom::Degree(d.clamp(1, 64) as u32)),
        _ => None,
    }
}

/// A pitch's place on the key's scale (snapped down to a scale tone): octaves times the scale's length plus the index.
fn scale_pos(key: Key, m: i32) -> i64 {
    let iv = key.intervals();
    let rel = i64::from(m - i32::from(key.tonic));
    let (oct, pc) = (rel.div_euclid(12), rel.rem_euclid(12));
    let idx = iv.iter().rposition(|x| *x <= pc).unwrap_or(0) as i64;
    oct * iv.len() as i64 + idx
}

fn from_pos(key: Key, pos: i64) -> i32 {
    let iv = key.intervals();
    let n = iv.len() as i64;
    (i64::from(key.tonic) + pos.div_euclid(n) * 12 + iv[pos.rem_euclid(n) as usize]) as i32
}

/// Move every pitch of a term by `f` (a function of the scale position), into its groups and alternations.
fn map_pitches(t: &Term, key: Key, f: &mut dyn FnMut(i64) -> i64) -> Term {
    let atom = match &t.atom {
        Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| Seq { span: s.span, terms: s.terms.iter().map(|x| map_pitches(x, key, f)).collect() }).collect()),
        Atom::Alt(ts) => Atom::Alt(ts.iter().map(|x| map_pitches(x, key, f)).collect()),
        Atom::ScaleDeg(d) => {
            let n = scale_len(key);
            let pos = i64::from(d.n.max(1)) - 1 + i64::from(d.oct) * n;
            let p = f(pos);
            Atom::ScaleDeg(crate::scale::ScaleDeg { n: (p.rem_euclid(n) + 1) as u32, acc: d.acc, oct: p.div_euclid(n).clamp(-4, 4) as i8 })
        }
        a => {
            let w = match a {
                Atom::Note(w) => w.clone(),
                _ => Word { text: String::new(), span: t.span },
            };
            match read_pitch(a) {
                Some(Pitch::Midi(m)) => write_pitch(Pitch::Midi(from_pos(key, f(scale_pos(key, m)))), &w).unwrap_or_else(|| a.clone()),
                Some(Pitch::Class(c)) => {
                    let m = from_pos(key, f(scale_pos(key, c + 60)));
                    write_pitch(Pitch::Class(m), &w).unwrap_or_else(|| a.clone())
                }
                Some(Pitch::Degree(d)) => write_pitch(Pitch::Degree(f(d - 1) + 1), &w).unwrap_or_else(|| a.clone()),
                None => a.clone(),
            }
        }
    };
    Term { atom, ..t.clone() }
}

fn scale_len(key: Key) -> i64 {
    key.intervals().len() as i64
}

/// The first pitch of a pattern, as a scale position (the axis of `invert`).
fn first_pos(terms: &[Term], key: Key) -> Option<i64> {
    for t in terms {
        let p = match &t.atom {
            Atom::Group(seqs) => seqs.iter().find_map(|s| first_pos(&s.terms, key)),
            Atom::Alt(ts) => first_pos(ts, key),
            Atom::ScaleDeg(d) => Some(i64::from(d.n.max(1)) - 1 + i64::from(d.oct) * scale_len(key)),
            a => match read_pitch(a) {
                Some(Pitch::Midi(m)) => Some(scale_pos(key, m)),
                Some(Pitch::Class(c)) => Some(scale_pos(key, c + 60)),
                Some(Pitch::Degree(d)) => Some(d - 1),
                None => None,
            },
        };
        if p.is_some() {
            return p;
        }
    }
    None
}

/// The pattern has a pitch anywhere in it.
fn has_pitch(terms: &[Term]) -> bool {
    terms.iter().any(|t| match &t.atom {
        Atom::Group(seqs) | Atom::Deg(seqs) => seqs.iter().any(|s| has_pitch(&s.terms)),
        Atom::Alt(ts) => has_pitch(ts),
        Atom::ScaleDeg(_) => true,
        // `chord(gm7)` plays notes: it is pitched, and `voiced` writes it out as them for the operators
        Atom::Call(c) => chord_call(c).is_some(),
        a => read_pitch(a).is_some(),
    })
}

/// `chord(gm7)` or `chord(gm7, 4)`: the chord, and the octave written if one is. None: another call, or a chord call that
/// does not read (`check` says what is wrong with it where the pattern is played).
fn chord_call(c: &Call) -> Option<(Chord, Option<i32>)> {
    if c.name.text != "chord" {
        return None;
    }
    let chord = match c.args.first()? {
        Arg::Word(w) => Chord::parse(&w.text)?,
        _ => return None,
    };
    match c.args.get(1) {
        None if c.args.len() == 1 => Some((chord, None)),
        Some(Arg::Num(n)) if c.args.len() == 2 && n.value.fract() == 0.0 && (-1.0..=8.0).contains(&n.value) => Some((chord, Some(n.value as i32))),
        _ => None,
    }
}

/// The steps hold a `chord(...)` call, at any depth `voiced` reaches.
fn has_chord_call(terms: &[Term]) -> bool {
    terms.iter().any(|t| match &t.atom {
        Atom::Group(seqs) => seqs.iter().any(|s| has_chord_call(&s.terms)),
        Atom::Alt(ts) => has_chord_call(ts),
        Atom::Call(c) => chord_call(c).is_some(),
        _ => false,
    })
}

/// A term with each `chord(gm7)` in it written out as the stack of notes it plays, `[g3, a#3, d4, f4]`, so the pitch and
/// chord operators move notes they can see (the same notes `[g3, a#3, d4, f4]` written out would be). `oct` is the octave
/// the chord plays in: the part's, as when it is played; a call with an octave of its own keeps that one. A chord that would
/// leave the MIDI range stays a call.
fn voiced(t: &Term, oct: i32) -> Term {
    let atom = match &t.atom {
        Atom::Call(c) => match chord_call(c) {
            Some((chord, own)) => {
                let notes = chord.notes(own.unwrap_or(oct));
                if notes.iter().all(|m| (0..=127).contains(m)) {
                    return chord_term(t, &notes);
                }
                t.atom.clone()
            }
            None => t.atom.clone(),
        },
        Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| Seq { span: s.span, terms: s.terms.iter().map(|x| voiced(x, oct)).collect() }).collect()),
        Atom::Alt(ts) => Atom::Alt(ts.iter().map(|x| voiced(x, oct)).collect()),
        a => a.clone(),
    };
    Term { atom, ..t.clone() }
}

fn previous_sound(terms: &[Term], i: usize) -> Option<Term> {
    let n = terms.len();
    (1..=n).map(|k| &terms[(i + n - k) % n]).find(|t| sounds(t)).cloned()
}

/// A stacked chord (`[c3,e3,g3]`) as its MIDI notes, low to high; None for anything else.
fn chord_notes(t: &Term) -> Option<Vec<i32>> {
    let Atom::Group(seqs) = &t.atom else { return None };
    if seqs.len() < 2 {
        return None;
    }
    let mut out = Vec::new();
    for s in seqs {
        let [one] = s.terms.as_slice() else { return None };
        match read_pitch(&one.atom)? {
            Pitch::Midi(m) => out.push(m),
            _ => return None,
        }
    }
    out.sort_unstable();
    Some(out)
}

fn chord_term(t: &Term, notes: &[i32]) -> Term {
    let w = Word { text: String::new(), span: t.span };
    let seqs = notes
        .iter()
        .map(|m| {
            let atom = write_pitch(Pitch::Midi(*m), &w).unwrap_or(Atom::Rest);
            Seq { span: t.span, terms: vec![Term { span: t.span, atom, fast: None, accent: false, slide: false, marks: Marks::default() }] }
        })
        .collect();
    Term { atom: Atom::Group(seqs), ..t.clone() }
}

/// A term with `f` applied to each stacked chord in it: the term itself, or the chords of its alternation (`<[a3,c4,e4]
/// [g3,b3,d4]>`) and squeezed groups, as far as the pitch operators reach.
fn map_chords(t: &Term, f: &dyn Fn(Vec<i32>) -> Vec<i32>) -> Term {
    if let Some(notes) = chord_notes(t) {
        return chord_term(t, &f(notes));
    }
    let atom = match &t.atom {
        Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| Seq { span: s.span, terms: s.terms.iter().map(|x| map_chords(x, f)).collect() }).collect()),
        Atom::Alt(ts) => Atom::Alt(ts.iter().map(|x| map_chords(x, f)).collect()),
        a => a.clone(),
    };
    Term { atom, ..t.clone() }
}

fn num_arg(args: &[MutArg], i: usize) -> f64 {
    match args.get(i) {
        Some(MutArg::Num(v, _)) => *v,
        _ => 0.0,
    }
}

/// The pattern a cross operator (`morph b p`, `xfer b`, `mask b`) reads its steps from: the let `b`, as its steps. A let that is
/// one `transpose(x, n)` or `octave(x, n)` of another (`let fate_b_1 = transpose(beethoven_fate_1, -1)`) is the steps of `x`
/// moved by that much, not one step that is a whole bar: a morph toward it takes its notes.
fn pat_arg(args: &[MutArg], ctx: &Ctx) -> Option<Seq> {
    let b = args.iter().find_map(|a| if let MutArg::Word(w) = a { ctx.others.get(&w.text) } else { None })?;
    Some(played_steps(b, ctx, 0).unwrap_or_else(|| b.clone()))
}

/// `transpose(x, n)` / `octave(x, n)` alone, with `x` a pattern let or a pattern: the steps of `x`, `n` semitones (octaves) on.
fn played_steps(b: &Seq, ctx: &Ctx, depth: usize) -> Option<Seq> {
    let [t] = b.terms.as_slice() else { return None };
    let Atom::Call(c) = &t.atom else { return None };
    let per = match c.name.text.as_str() {
        "transpose" => 1i64,
        "octave" => 12,
        _ => return None,
    };
    let n = match c.args.get(1) {
        Some(crate::ast::Arg::Num(n)) if n.value.fract() == 0.0 => n.value as i64 * per,
        _ => return None,
    };
    let inner = match c.args.first()? {
        crate::ast::Arg::Word(w) => ctx.others.get(&w.text)?.clone(),
        crate::ast::Arg::Pat(s) if s.terms.len() == 1 && matches!(&s.terms[0].atom, Atom::Note(w) if ctx.others.contains_key(&w.text)) => {
            let Atom::Note(w) = &s.terms[0].atom else { return None };
            ctx.others.get(&w.text)?.clone()
        }
        crate::ast::Arg::Pat(s) => s.clone(),
        _ => return None,
    };
    let inner = if depth < 4 { played_steps(&inner, ctx, depth + 1).unwrap_or(inner) } else { inner };
    Some(Seq { span: b.span, terms: inner.terms.iter().map(|x| shift_semis(x, n)).collect() })
}

/// A term with every note `n` semitones on (a degree has no semitones, and stays).
fn shift_semis(t: &Term, n: i64) -> Term {
    let atom = match &t.atom {
        Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| Seq { span: s.span, terms: s.terms.iter().map(|x| shift_semis(x, n)).collect() }).collect()),
        Atom::Alt(ts) => Atom::Alt(ts.iter().map(|x| shift_semis(x, n)).collect()),
        a @ Atom::Note(w) => match read_pitch(a) {
            Some(Pitch::Midi(m)) => write_pitch(Pitch::Midi(m + n as i32), w).unwrap_or_else(|| a.clone()),
            Some(Pitch::Class(c)) => write_pitch(Pitch::Class(c + n as i32), w).unwrap_or_else(|| a.clone()),
            _ => a.clone(),
        },
        a => a.clone(),
    };
    Term { atom, ..t.clone() }
}

/// One operator on a pattern's steps (its top-level terms).
fn apply_op(op: &MutOp, seq: &Seq, ctx: &Ctx, rng: &mut Rng) -> Seq {
    // an operator that moves pitches sees a `chord(gm7)` step as the notes it plays
    let written;
    let seq = if has_chord_call(&seq.terms) && op_spec(&op.name.text).is_some_and(|(_, f)| matches!(f, Family::Pitch | Family::Chord)) {
        written = Seq { span: seq.span, terms: seq.terms.iter().map(|t| voiced(t, ctx.octave)).collect() };
        &written
    } else {
        seq
    };
    let mut t: Vec<Term> = seq.terms.clone();
    let l = t.len();
    if l == 0 {
        return seq.clone();
    }
    let a = num_arg(&op.args, 0);
    let key = ctx.key;
    match op.name.text.as_str() {
        "rot" => t.rotate_right((a as i64).rem_euclid(l as i64) as usize),
        "rev" => t.reverse(),
        "thin" => {
            for x in &mut t {
                if sounds(x) && rng.unit() < a {
                    *x = rest(x.span);
                }
            }
        }
        "fill" => {
            let orig = t.clone();
            for i in 0..l {
                if !sounds(&orig[i]) && rng.unit() < a {
                    t[i] = previous_sound(&orig, i).map(|p| Term { span: orig[i].span, ..p }).unwrap_or(Term { atom: Atom::Hit, ..orig[i].clone() });
                }
            }
        }
        "euclid" => {
            let k = (a as i64).clamp(0, l as i64);
            let mask = djengine::gen::euclidean::bjorklund(k, l as i64);
            let snd: Vec<Term> = t.iter().filter(|x| sounds(x)).cloned().collect();
            let mut j = 0;
            t = (0..l)
                .map(|i| {
                    if mask.get(i) == Some(&1) {
                        let x = snd.get(j % snd.len().max(1)).cloned().unwrap_or(Term { atom: Atom::Hit, ..seq.terms[i].clone() });
                        j += 1;
                        Term { span: seq.terms[i].span, ..x }
                    } else {
                        rest(seq.terms[i].span)
                    }
                })
                .collect();
        }
        "swap" => {
            let mut i = 0;
            while i + 1 < l {
                t.swap(i, i + 1);
                i += 2;
            }
        }
        "mirror" => {
            let h = l.div_ceil(2);
            for i in h..l {
                t[i] = seq.terms[l - 1 - i].clone();
            }
        }
        "shuffle" => {
            for i in (1..l).rev() {
                let j = rng.below(i + 1);
                t.swap(i, j);
            }
        }
        "half" => t.truncate(l.div_ceil(2)),
        "double" => {
            if 2 * l <= MAX_STEPS {
                t.extend(seq.terms.iter().cloned());
            }
        }
        "stutter" => {
            let n = (a as i64).max(1) as usize;
            let chunk = l.div_ceil(n);
            t = (0..l).map(|i| seq.terms[i % chunk].clone()).collect();
        }
        "ratchet" => {
            for x in &mut t {
                if sounds(x) && rng.unit() < a {
                    let k = x.fast.unwrap_or(1) * 2;
                    if k <= 64 {
                        x.fast = Some(k);
                    }
                }
            }
        }
        "swing" => {
            // the second step of each pair later by `amt` of a step, on the sixteenth grid: only where a step is two
            // sixteenths or more
            if 16 % l == 0 && l >= 2 {
                let s = 16 / l;
                let d = ((a.clamp(0.0, 1.0) * s as f64).round() as usize).min(s - 1);
                if d > 0 {
                    let mut out = Vec::new();
                    let mut i = 0;
                    while i < l {
                        if i + 1 < l {
                            let mut slots: Vec<Term> = (0..2 * s).map(|_| rest(seq.terms[i].span)).collect();
                            slots[0] = seq.terms[i].clone();
                            slots[s + d] = seq.terms[i + 1].clone();
                            out.push(Term { span: seq.terms[i].span, atom: Atom::Group(vec![Seq { span: seq.span, terms: slots }]), fast: None, accent: false, slide: false, marks: Marks::default() });
                        } else {
                            out.push(seq.terms[i].clone());
                        }
                        i += 2;
                    }
                    t = out;
                }
            }
        }
        "up" | "down" => {
            let k = if op.name.text == "up" { a as i64 } else { -(a as i64) };
            t = t.iter().map(|x| map_pitches(x, key, &mut |p| p + k)).collect();
        }
        "invert" => {
            if let Some(axis) = first_pos(&t, key) {
                t = t.iter().map(|x| map_pitches(x, key, &mut |p| 2 * axis - p)).collect();
            }
        }
        "oct" => {
            let k = a as i64 * scale_len(key);
            t = t.iter().map(|x| map_pitches(x, key, &mut |p| p + k)).collect();
        }
        "drift" => {
            t = t
                .iter()
                .map(|x| {
                    if rng.unit() < a {
                        let k = if rng.next() & 1 == 0 { 1 } else { -1 };
                        map_pitches(x, key, &mut |p| p + k)
                    } else {
                        x.clone()
                    }
                })
                .collect();
        }
        "vel" => {
            // a step's loudness in the text is its accent: `vel +n` accents the n strongest unaccented steps (downbeats
            // first), `vel -n` takes the accent off the last n
            let n = a as i64;
            let strength = |i: usize| -> u32 { ((i * 16 / l) as u32 | 16).trailing_zeros() };
            if n > 0 {
                let mut idx: Vec<usize> = (0..l).filter(|i| sounds(&t[*i]) && !t[*i].accent).collect();
                idx.sort_by_key(|i| (std::cmp::Reverse(strength(*i)), *i));
                for i in idx.into_iter().take(n as usize) {
                    t[i].accent = true;
                }
            } else if n < 0 {
                let idx: Vec<usize> = (0..l).filter(|i| t[*i].accent).collect();
                for i in idx.into_iter().rev().take((-n) as usize) {
                    t[i].accent = false;
                }
            }
        }
        "accent" => {
            for x in &mut t {
                if sounds(x) && rng.unit() < a {
                    x.accent = true;
                }
            }
        }
        "morph" => {
            if let Some(b) = pat_arg(&op.args, ctx).filter(|b| !b.terms.is_empty()) {
                let p = num_arg(&op.args, 1);
                let lb = b.terms.len();
                for i in 0..l {
                    if rng.unit() < p {
                        t[i] = Term { span: t[i].span, ..b.terms[i * lb / l].clone() };
                    }
                }
            }
        }
        "xfer" => {
            if let Some(b) = pat_arg(&op.args, ctx).filter(|b| !b.terms.is_empty()) {
                let snd: Vec<Term> = seq.terms.iter().filter(|x| sounds(x)).cloned().collect();
                let mut j = 0;
                t = b
                    .terms
                    .iter()
                    .map(|bt| {
                        if !sounds(bt) {
                            rest(bt.span)
                        } else if snd.is_empty() {
                            bt.clone()
                        } else {
                            let x = snd[j % snd.len()].clone();
                            j += 1;
                            x
                        }
                    })
                    .collect();
            }
        }
        "mask" => {
            if let Some(b) = pat_arg(&op.args, ctx).filter(|b| !b.terms.is_empty()) {
                let lb = b.terms.len();
                for i in 0..l {
                    if !sounds(&b.terms[i * lb / l]) {
                        t[i] = rest(t[i].span);
                    }
                }
            }
        }
        "grow" => {
            let rests: Vec<usize> = (0..l).filter(|i| !sounds(&t[*i])).collect();
            if !rests.is_empty() {
                let i = rests[rng.below(rests.len())];
                t[i] = previous_sound(&seq.terms, i).map(|p| Term { span: seq.terms[i].span, ..p }).unwrap_or(Term { atom: Atom::Hit, ..seq.terms[i].clone() });
            }
        }
        "decay" => {
            let snd: Vec<usize> = (0..l).filter(|i| sounds(&t[*i])).collect();
            if !snd.is_empty() {
                let i = snd[rng.below(snd.len())];
                t[i] = rest(t[i].span);
            }
        }
        "voice" => {
            let n = a as i64;
            t = t
                .iter()
                .map(|x| {
                    map_chords(x, &|mut notes| {
                        for _ in 0..n.unsigned_abs().min(16) {
                            if n > 0 {
                                let lo = notes.remove(0);
                                notes.push(lo + 12);
                            } else {
                                let hi = notes.pop().unwrap();
                                notes.insert(0, hi - 12);
                            }
                        }
                        notes
                    })
                })
                .collect();
        }
        "spread" => {
            t = t
                .iter()
                .map(|x| {
                    map_chords(x, &|notes| {
                        let mut spread: Vec<i32> = notes.iter().enumerate().map(|(i, m)| if i % 2 == 1 { m + 12 } else { *m }).collect();
                        spread.sort_unstable();
                        spread
                    })
                })
                .collect();
        }
        _ => {}
    }
    Seq { span: seq.span, terms: t }
}

// ---- the check

/// What checking a block's operators found: the diagnostics, and the operators the fixer would turn into `id`.
struct Checked {
    diags: Vec<Diagnostic>,
    bad_ops: Vec<(Span, String)>,
    /// (mutation, target, operator) that are a no-op (a pitch operator on a drum pattern).
    skip: BTreeSet<(usize, usize, usize)>,
    /// Each target's patterns, by (mutation, target).
    resolved: BTreeMap<(usize, usize), Vec<String>>,
}

fn glob_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else { return false };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

const TONAL: [&str; 4] = ["bass", "melody", "harmony", "arp"];

fn check_block(b: &ArrangeBlock, pats: &BTreeMap<String, Seq>, drums: &BTreeSet<String>, octaves: &BTreeMap<String, Vec<i32>>, other_names: &BTreeSet<String>) -> Checked {
    let mut c = Checked { diags: Vec::new(), bad_ops: Vec::new(), skip: BTreeSet::new(), resolved: BTreeMap::new() };
    let names: Vec<String> = pats.keys().cloned().collect();
    let mut mi = 0;
    for it in &b.items {
        let ArrItem::Mutation(m) = it else { continue };
        c.diags.extend(every_warning(b, m));
        for (ti, t) in m.targets.iter().enumerate() {
            let hits: Vec<String> = if t.is_glob() {
                let h: Vec<String> = names.iter().filter(|n| glob_match(&t.name.text, n)).cloned().collect();
                if h.is_empty() {
                    c.diags.push(Diagnostic::warning(t.name.span, format!("`{}` matches no `let` pattern: this mutation does nothing", t.name.text)));
                }
                h
            } else if pats.contains_key(&t.name.text) {
                vec![t.name.text.clone()]
            } else {
                let mut d = Diagnostic::error(t.name.span, format!("`{}` is not a `let` pattern: a mutation rewrites a pattern named with `let`", t.name.text));
                d = if other_names.contains(&t.name.text) {
                    d.help("a mutation acts on a `let` of a pattern (`let acid = c2 ~ eb2 g2`), not on a section, a def or a single value")
                } else {
                    match crate::env::suggest(&t.name.text, &names, 1).first() {
                        Some(n) => d.help(format!("did you mean `{n}`?")),
                        None => d.help("name the pattern first: let acid = c2 ~ eb2 g2"),
                    }
                };
                c.diags.push(d);
                Vec::new()
            };
            let mut forked = false;
            for (oi, op) in t.ops.iter().enumerate() {
                if op.inner.is_some() {
                    // a chain forks once: what follows the first `jux` is the right copy's
                    if forked {
                        let msg = "a chain has one `jux`: what follows it already plays on the right copy".to_string();
                        c.diags.push(Diagnostic::error(op.span, msg.clone()).help("the operator after the first `jux` goes on the right copy (`jux rev | rot 2`); a second fork goes on a line of its own (the fixer comments it out as `id`)"));
                        c.bad_ops.push((op.span, msg));
                        continue;
                    }
                    forked = true;
                }
                check_op(op, (mi, ti, oi), false, &hits, pats, drums, octaves, &mut c);
            }
            c.resolved.insert((mi, ti), hits);
        }
        mi += 1;
    }
    let _ = &mut mi;
    c
}

/// `~3` in a block that never has a 3rd pass: the line fires never. Said at the count, in the block's own terms; the
/// program still checks (a block is edited between `loop 6` and `loop 2`).
fn every_warning(b: &ArrangeBlock, m: &Mutation) -> Option<Diagnostic> {
    let (Some(span), true) = (m.every_span, m.every > 1) else { return None };
    let (mark, n) = (if m.once { "@" } else { "~" }, m.every);
    let more = format!("}} loop {}", (n * 2).min(256));
    let (why, help) = match (&b.named, &b.ends) {
        (Some(_), _) => ("a named `arrange` runs its mutations over one pass (its `loop` replays the result)".to_string(), "count passes in the song's own block: arrange { ... } loop 6 then end".to_string()),
        (None, BlockEnd::End) => ("this `arrange` plays once".to_string(), format!("give it passes: {more}")),
        (None, BlockEnd::Loop) => ("a `loop` with no count is laid out as one pass".to_string(), format!("count the passes: {more}")),
        (None, BlockEnd::Passes { n: k, .. }) if *k < n => (format!("`loop {k}` plays {k} pass{}", if *k == 1 { "" } else { "es" }), format!("`loop {n}` or more reaches it: {more}")),
        _ => return None,
    };
    Some(Diagnostic::warning(span, format!("{why}, so `{mark}{n}` never fires")).help(help))
}

/// One operator of a chain, at (mutation, target, place in the chain): its name, its arguments, and what it does to the
/// patterns it hits (`hits`). `jux op` is checked as the operator it forks the right copy with.
fn check_op(op: &MutOp, at: (usize, usize, usize), under_jux: bool, hits: &[String], pats: &BTreeMap<String, Seq>, drums: &BTreeSet<String>, octaves: &BTreeMap<String, Vec<i32>>, c: &mut Checked) {
    if let Some(inner) = &op.inner {
        if inner.inner.is_some() {
            let msg = "`jux` forks once: its operator cannot be another `jux`".to_string();
            c.diags.push(Diagnostic::error(inner.span, msg.clone()).help("the fixer comments it out as `id`"));
            c.bad_ops.push((inner.span, msg));
        } else {
            check_op(inner, at, true, hits, pats, drums, octaves, c);
        }
        return;
    }
    let Some((kinds, family)) = op_spec(&op.name.text) else {
        let all: Vec<String> = operator_names().iter().map(|s| s.to_string()).collect();
        let near = crate::env::suggest(&op.name.text, &all, 1);
        let d = Diagnostic::error(op.name.span, format!("`{}` is not a mutation operator", op.name.text));
        c.diags.push(d.help(match near.first() {
            Some(n) => format!("did you mean `{n}`? (the fixer comments it out as `id`)"),
            None => format!("the operators: {} (the fixer comments it out as `id`)", all.join(" ")),
        }));
        c.bad_ops.push((op.span, format!("unknown operator `{}`", op.name.text)));
        return;
    };
    let usage = format!("{}{}", op.name.text, kinds.iter().map(|k| match k { Int => " n", Prob => " p", Pat => " <pattern>", Op => " <operator>" }).collect::<String>());
    let mut bad: Option<String> = None;
    if op.args.len() != kinds.len() {
        bad = Some(format!("`{}` takes {} argument{}: `{usage}`", op.name.text, kinds.len(), if kinds.len() == 1 { "" } else { "s" }));
    } else {
        for (k, arg) in kinds.iter().zip(&op.args) {
            let ok = match (k, arg) {
                (Int, MutArg::Num(v, _)) => v.fract() == 0.0 && v.abs() <= 64.0,
                (Prob, MutArg::Num(v, _)) => (0.0..=1.0).contains(v),
                (Pat, MutArg::Word(w)) => pats.contains_key(&w.text),
                _ => false,
            };
            if !ok {
                bad = Some(match k {
                    Int => format!("`{}` takes a whole number, -64 to 64: `{usage}`", op.name.text),
                    Prob => format!("`{}` takes a share from 0 to 1: `{usage}`", op.name.text),
                    Pat => format!("`{}` takes another `let` pattern: `{usage}`", op.name.text),
                    Op => format!("`{}` takes an operator: `{usage}`", op.name.text),
                });
                break;
            }
        }
    }
    if let Some(msg) = bad {
        c.diags.push(Diagnostic::error(op.span, msg.clone()).help("the fixer comments it out as `id`"));
        c.bad_ops.push((op.span, msg));
        return;
    }
    if matches!(family, Family::Pitch | Family::Chord) {
        for h in hits {
            if drums.contains(h) {
                let then = if under_jux { "`jux` gives it no right copy" } else { "it does nothing there" };
                c.diags.push(Diagnostic::warning(op.span, format!("`{}` moves pitches, and `{h}` is a drum pattern: {then}", op.name.text)));
                c.skip.insert(at);
            } else if let (true, Some(o)) = (has_chord_call(&pats[h].terms), octaves.get(h)) {
                // a chord call plays in the octave of the part that reads it: one let, one voicing
                if let [first, second, ..] = o.as_slice() {
                    c.diags.push(Diagnostic::warning(op.span, format!("`{h}` is read in octaves {first} and {second}: `{}` voices its `chord(...)` steps in octave {first}", op.name.text)).help("give the parts that read it the same `oct=`, or write the chord's notes: [g3,a#3,d4,f4]"));
                }
            }
        }
    }
}

// ---- the pass

/// What an `arrange { }` came to: each section it plays, with the generation of each pattern it reads, and each
/// pattern's generations. Read by `check`/`eval` (beatscript-api) and the editor's Follow.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GenTable {
    /// Every section played, in order.
    pub plays: Vec<Play>,
    /// Every mutated pattern's generations, generation 0 (as written) first.
    pub patterns: BTreeMap<String, Vec<Gen>>,
    /// How many passes are laid out (1 for `end` and for an unbounded `loop`, which renders one pass).
    pub passes: u32,
    /// `loop` with no count: it plays on, each lap a new generation (laid out: the first pass).
    pub unbounded: bool,
    /// `arrange NAME: { ... }`: the named arrangement the block is (its plays are where the flow plays it).
    pub arrangement: Option<String>,
    /// Where each `~3` / `@4` line fired (a line with a count above 1; none for any other program).
    pub fires: Vec<Fire>,
}

/// The passes a counted mutation line (`~3`, `@4`) fired on, and the plays that first saw each firing.
#[derive(Clone, Debug, PartialEq)]
pub struct Fire {
    /// The line's index among the block's mutations (0-based), as in [`Gen::mutation`].
    pub mutation: usize,
    pub every: u32,
    pub once: bool,
    /// The passes (0-based) it fired on: `every - 1`, `2 * every - 1` ...
    pub passes: Vec<u32>,
    /// For each firing, the index in [`GenTable::plays`] of the first play that reads it (None: it fired after the last
    /// play, at the end of the last pass with no coda, so nothing plays after it).
    pub plays: Vec<Option<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    /// The section as written.
    pub section: String,
    /// The name it is lowered (and followed) as: the section's own, or `drop@2` for a later variant.
    pub name: String,
    /// The pass (0-based); the coda's plays have the last pass's.
    pub pass: u32,
    pub coda: bool,
    /// The patterns of the program the section reads, each at the generation it reads.
    pub reads: Vec<Read>,
    /// The calls (`move`, `freeze` ...) whose lines on the section's control deck do not fire in this play: a switch
    /// (`~ control off`, `@ control freeze off`) holds them back. Empty: every control-deck line of the section fires.
    pub control_off: Vec<String>,
}

/// Which lines of the control deck fire: the switches of an `arrange { }` applied in order. Every line fires until a switch
/// says otherwise.
#[derive(Clone, Debug, Default, PartialEq)]
struct Switches {
    /// `~ control off` said last of the whole deck.
    off: bool,
    /// A call switched since (`~ control freeze on`): on or off, over what the whole deck is.
    calls: BTreeMap<String, bool>,
}

impl Switches {
    fn fires(&self, call: &str) -> bool {
        self.calls.get(call).copied().unwrap_or(!self.off)
    }

    fn with(mut self, s: &Switch) -> Switches {
        match &s.call {
            None => {
                self.off = !s.on;
                self.calls.clear();
            }
            Some(c) => {
                self.calls.insert(c.text.clone(), s.on);
            }
        }
        self
    }
}

/// The calls of the control-deck lines in these statements (a section's body), through the repeats in them and the defs they
/// call: what a switch can hold back there ([`crate::ast::Body::call_name`]; in a body still in tokens, the word after
/// `control:`, or after `control: do`).
fn control_calls(stmts: &[Stmt], defs: &BTreeMap<String, &[Token]>, out: &mut BTreeSet<String>) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for st in stmts {
        match &st.kind {
            StmtKind::Line(l) => {
                if let (Some(_), Some(c)) = (l.control, l.body.call_name()) {
                    out.insert(c.to_string());
                }
                if let Body::Invoke(inv) = &l.body {
                    if let Some(body) = defs.get(inv.name.text.as_str()) {
                        if seen.insert(inv.name.text.clone()) {
                            token_calls(body, defs, &mut seen, out);
                        }
                    }
                }
            }
            StmtKind::Repeat(r) => token_calls(&r.body, defs, &mut seen, out),
            _ => {}
        }
    }
}

fn token_calls(toks: &[Token], defs: &BTreeMap<String, &[Token]>, seen: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
    for (i, t) in toks.iter().enumerate() {
        let Tok::Word(w) = &t.tok else { continue };
        let next = toks.get(i + 1).map(|t| &t.tok);
        if w == "control" && crate::parse::control_head_at(next, i.checked_sub(1).map(|p| &toks[p].tok)) {
            let word = |k: usize| match toks.get(k).map(|t| &t.tok) {
                Some(Tok::Word(w)) => Some(w.clone()),
                _ => None,
            };
            if let Some(c) = word(i + 2).and_then(|c| if c == "do" { word(i + 3) } else { Some(c) }) {
                out.insert(c);
            }
        } else if next == Some(&Tok::LParen) {
            if let Some(body) = defs.get(w.as_str()) {
                if seen.insert(w.clone()) {
                    token_calls(body, defs, seen, out);
                }
            }
        }
    }
}

/// The engine calls of these statements written with no head (`move echo_out on B` in a section): the control deck's lines in
/// all but the head, which no switch reaches. For the help of a switch that reaches nothing.
fn headless_engine_lines(stmts: &[Stmt], src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for st in stmts {
        if let StmtKind::Line(l) = &st.kind {
            let engine = matches!(&l.body, Body::Do(_) | Body::Duck(_) | Body::Mix(_))
                || matches!(&l.body, Body::Action(Action::Move { .. } | Action::SwapPairs { .. } | Action::LoadPair { .. } | Action::Queue { .. } | Action::Tempo(_)))
                || matches!(&l.body, Body::Action(Action::Change { name, .. } | Action::Shaped { name, .. }) if name.text == "crossfader");
            if engine && l.control.is_none() && l.target.is_none() {
                out.push(format!("line {} `{}`", crate::diag::line_col(src, st.span.start).0, crate::print::line(l)));
            }
        }
    }
    out
}

/// What the switches of a block are said to do: a call no control-deck line has is an error with the nearest names; a switch
/// that reaches no line (the control deck's lines are all outside every section, or there are none) is a warning.
fn switch_diags(b: &ArrangeBlock, in_sections: &BTreeSet<String>, outside: &BTreeSet<String>, headless: &[String]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let outside_help = "the control deck's lines outside every section play the whole song, and a switch reaches a section's lines: put them in the sections that want them, section drop 8 bars { control: freeze phrase bars=2 }";
    for it in &b.items {
        let ArrItem::Mutation(m) = it else { continue };
        let Some(s) = &m.switch else { continue };
        match &s.call {
            Some(c) if in_sections.contains(&c.text) => {}
            Some(c) if outside.contains(&c.text) => out.push(Diagnostic::warning(c.span, format!("every `{}` line of the control deck is outside the sections: this switch does nothing", c.text)).help(outside_help)),
            Some(c) => {
                let all: Vec<String> = in_sections.iter().chain(outside).cloned().collect();
                let d = Diagnostic::error(c.span, format!("the control deck has no `{}` line: a switch names the call of a line on it (`move`, `freeze` ...)", c.text));
                out.push(match crate::env::suggest(&c.text, &all, 1).first() {
                    Some(n) => d.help(format!("did you mean `{n}`?")),
                    None if !all.is_empty() => d.help(format!("its calls: {}", all.join(", "))),
                    None => d.help("write the line in a section first: control: freeze phrase bars=2"),
                });
            }
            None if in_sections.is_empty() => {
                let d = Diagnostic::warning(s.span, "no section has a line on the control deck: this switch does nothing");
                out.push(if !outside.is_empty() {
                    d.help(outside_help)
                } else if let Some(h) = headless.first() {
                    d.help(format!("a switch reaches the lines written on the control deck, and {h} has no head: control: {}", h.split('`').nth(1).unwrap_or("")))
                } else {
                    d.help("a section's `control:` lines are what it switches: section drop 8 bars { control: move echo_out on B }")
                });
            }
            None => {}
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub pattern: String,
    pub generation: u32,
    /// A one-shot (`@`) is on top of that generation for this play.
    pub once: bool,
    /// The pattern's text as this play reads it.
    pub text: String,
    /// The right copy's text, when the pattern is a pair at this generation.
    pub right: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gen {
    pub generation: u32,
    /// The pass and the mutation (0-based, of the block's mutations) that made it; generation 0 has neither.
    pub pass: Option<u32>,
    pub mutation: Option<usize>,
    /// The operators applied, as written.
    pub ops: String,
    pub text: String,
    /// The line's count (`~3`: 3); 1 when it fires on every pass.
    pub every: u32,
    /// After a `jux`: the right copy's text (`text` is the left's); None for a pattern that plays one copy.
    pub right: Option<String>,
}

/// A counted line fired on this pass: note it (the line's first firing starts its entry), with the play about to be pushed.
fn record_fire(fires: &mut Vec<Fire>, mutation: usize, m: &Mutation, pass: u32, play: usize) {
    if !fires.iter().any(|f| f.mutation == mutation) {
        fires.push(Fire { mutation, every: m.every, once: m.once, passes: Vec::new(), plays: Vec::new() });
    }
    let f = fires.iter_mut().find(|f| f.mutation == mutation).unwrap();
    f.passes.push(pass);
    f.plays.push(Some(play));
}

/// The text of a pattern, as the language prints it.
fn seq_text(s: &Seq) -> String {
    crate::print::seq(s)
}

/// One generation of a pattern: the left copy, which is the pattern as it always was, and, once a `jux` has forked it,
/// the right copy.
#[derive(Clone, Debug)]
struct Pair {
    left: Seq,
    right: Option<Seq>,
}

impl Pair {
    fn one(left: Seq) -> Pair {
        Pair { left, right: None }
    }
    /// What an operator on the right copy reads of this pattern: its right copy, or the only copy it has.
    fn right_or_left(&self) -> &Seq {
        self.right.as_ref().unwrap_or(&self.left)
    }
}

/// The let name of a generation: `acid` (generation 0), `acid@3`.
fn gen_name(name: &str, g: u32) -> String {
    if g == 0 {
        name.to_string()
    } else {
        format!("{name}@{g}")
    }
}

/// Each name a pattern line of these statements reads (a word that is a note's place, a transform's argument), and the
/// words of the token bodies (`repeat`).
fn walk_words(stmts: &mut [Stmt], f: &mut dyn FnMut(&mut String)) {
    for st in stmts {
        match &mut st.kind {
            StmtKind::Line(l) => {
                if let Body::Parts(parts) = &mut l.body {
                    for p in parts {
                        walk_seq(&mut p.pattern, f);
                    }
                }
            }
            StmtKind::Repeat(r) => {
                for t in &mut r.body {
                    if let Tok::Word(w) = &mut t.tok {
                        f(w);
                    }
                }
            }
            StmtKind::Section(s) => walk_words(&mut s.body, f),
            _ => {}
        }
    }
}

fn walk_seq(s: &mut Seq, f: &mut dyn FnMut(&mut String)) {
    for t in &mut s.terms {
        walk_term(t, f);
    }
}

fn walk_term(t: &mut Term, f: &mut dyn FnMut(&mut String)) {
    match &mut t.atom {
        Atom::Note(w) => f(&mut w.text),
        Atom::Group(seqs) | Atom::Deg(seqs) => seqs.iter_mut().for_each(|s| walk_seq(s, f)),
        Atom::Alt(ts) => ts.iter_mut().for_each(|x| walk_term(x, f)),
        Atom::Call(c) => walk_call(c, f),
        Atom::Chance(c) => walk_term(&mut c.term, f),
        Atom::Choose(c) => c.options.iter_mut().for_each(|x| walk_term(x, f)),
        // `acid_a~2` (pattern sugar) reads the pattern it holds
        Atom::Sized(s) => walk_term(&mut s.term, f),
        _ => {}
    }
}

fn walk_call(c: &mut Call, f: &mut dyn FnMut(&mut String)) {
    for a in &mut c.args {
        walk_arg(a, f);
    }
}

fn walk_arg(a: &mut Arg, f: &mut dyn FnMut(&mut String)) {
    match a {
        Arg::Word(w) => f(&mut w.text),
        Arg::Pat(s) => walk_seq(s, f),
        Arg::Call(c) => walk_call(c, f),
        Arg::Named(_, v) => walk_arg(v, f),
        Arg::Num(_) => {}
    }
}

/// After each part of these statements' lines that reads a pattern that is a pair (`right_of`: the left copy's let ->
/// the right copy's), the same part reading the right copies, marked [`Part::right`]: the compiler plays it as the
/// part's twin, panned to the right.
fn add_twins(stmts: &mut [Stmt], right_of: &BTreeMap<String, String>) {
    for st in stmts {
        let StmtKind::Line(l) = &mut st.kind else { continue };
        let Body::Parts(parts) = &mut l.body else { continue };
        let mut out = Vec::with_capacity(parts.len());
        for p in parts.drain(..) {
            let mut twin = p.clone();
            let mut pair = false;
            walk_seq(&mut twin.pattern, &mut |w| {
                if let Some(r) = right_of.get(w.as_str()) {
                    *w = r.clone();
                    pair = true;
                }
            });
            out.push(p);
            if pair {
                twin.right = true;
                out.push(twin);
            }
        }
        *parts = out;
    }
}

/// What a `jux` should say beyond its operators being right: where the right copy cannot be heard apart, or leaves the
/// middle of the mix.
fn jux_notes(block: &ArrangeBlock, checked: &Checked, roles: &BTreeMap<String, BTreeSet<String>>, program: &Program) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut mi = 0;
    for it in &block.items {
        let ArrItem::Mutation(m) = it else { continue };
        for (ti, t) in m.targets.iter().enumerate() {
            for (oi, op) in t.ops.iter().enumerate() {
                if op.inner.is_none() || checked.skip.contains(&(mi, ti, oi)) {
                    continue;
                }
                for name in checked.resolved.get(&(mi, ti)).into_iter().flatten() {
                    // the low end leaves the middle: a kick and a bass are the weight of the mix
                    let low: Vec<&str> = ["kick", "bass"].into_iter().filter(|r| roles.get(name).is_some_and(|rs| rs.contains(*r))).collect();
                    if !low.is_empty() {
                        out.push(Diagnostic::warning(op.name.span, format!("`jux` on `{name}`, which a {} plays: the low end leaves the middle, each copy hard to one side", low.join(" and a "))).help("a kick and a bass keep the mix together in the middle: jux a melody, an arp or a hat"));
                    }
                    // a reader `jux` cannot split plays the left copy only
                    if let Some(place) = reads_beyond_section_lines(program, name) {
                        out.push(Diagnostic::warning(op.name.span, format!("`{name}` is also read in {place}, where `jux` cannot split it: that reader plays the left copy only")).help("a section's own lines (`deck A: melody lead`) play both copies"));
                    }
                }
            }
        }
        mi += 1;
    }
    out.dedup_by(|a, b| a.span == b.span && a.message == b.message);
    out
}

/// Where else a pattern is read than in the part lines of a section (which an `arrange { }` lowers to its generation, and
/// to a right copy for a `jux`): a line outside every section, a `repeat`, a `def`, a call of one.
fn reads_beyond_section_lines(program: &Program, name: &str) -> Option<&'static str> {
    fn tokens_read(tokens: &[Token], name: &str) -> bool {
        tokens.iter().any(|t| matches!(&t.tok, Tok::Word(w) if w == name))
    }
    fn line_reads(l: &Line, name: &str) -> bool {
        match &l.body {
            Body::Invoke(i) => i.args.iter().any(|a| tokens_read(a, name)),
            _ => false,
        }
    }
    fn walk(stmts: &[Stmt], name: &str, in_section: bool) -> Option<&'static str> {
        for st in stmts {
            match &st.kind {
                StmtKind::Line(l) => {
                    if line_reads(l, name) {
                        return Some("a call of a `def`");
                    }
                    if let (false, Body::Parts(parts)) = (in_section, &l.body) {
                        let mut found = false;
                        for p in parts {
                            let mut pat = p.pattern.clone();
                            walk_seq(&mut pat, &mut |w| found |= w == name);
                        }
                        if found {
                            return Some("a line outside every section");
                        }
                    }
                }
                StmtKind::Repeat(r) if tokens_read(&r.body, name) => return Some("a `repeat`"),
                StmtKind::Def(d) if tokens_read(&d.body, name) => return Some("a `def`"),
                StmtKind::Section(s) => {
                    if let Some(w) = walk(&s.body, name, true) {
                        return Some(w);
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(&program.stmts, name, false)
}

/// Who reads a pattern: the roles of the parts that read it, and the octaves they play in (each once, in the order the
/// program reads them).
#[derive(Default)]
struct Readers {
    roles: BTreeSet<String>,
    octaves: Vec<i32>,
}

/// The octave a part plays in: its role's, or its `oct=`.
fn part_octave(p: &Part) -> i32 {
    let mut oct = crate::music::role_octave(&p.role.text);
    if let Some(Param { value: ParamValue::Num(n), .. }) = p.params.iter().find(|pr| pr.name.text == "oct") {
        if n.value.fract() == 0.0 && (-1.0..=8.0).contains(&n.value) {
            oct = n.value as i32;
        }
    }
    oct
}

/// The parts that read each name.
fn readers_of(stmts: &[Stmt], out: &mut BTreeMap<String, Readers>) {
    for st in stmts {
        match &st.kind {
            StmtKind::Line(l) => {
                if let Body::Parts(parts) = &l.body {
                    for p in parts {
                        let mut pat = p.pattern.clone();
                        let oct = part_octave(p);
                        walk_seq(&mut pat, &mut |w| {
                            let r = out.entry(w.clone()).or_default();
                            r.roles.insert(p.role.text.clone());
                            if !r.octaves.contains(&oct) {
                                r.octaves.push(oct);
                            }
                        });
                    }
                }
            }
            StmtKind::Section(s) => readers_of(&s.body, out),
            _ => {}
        }
    }
}

/// The program's `let` patterns (a value longer than one token), read.
fn pattern_lets(program: &Program, src: &str) -> BTreeMap<String, (Seq, Span)> {
    let mut out = BTreeMap::new();
    for st in &program.stmts {
        if let StmtKind::Let(l) = &st.kind {
            if l.value.len() > 1 && !crate::parse::is_unit_value(&l.value) {
                if let Ok(seq) = crate::parse::parse_seq_tokens(l.value.clone(), src) {
                    // an operator works on the pattern's steps, so on its sugar written out (sugar.rs): `c2~3 e2` is four
                    // steps here as it is when it plays (what is wrong with the sugar is said where the pattern is played)
                    let seq = if crate::sugar::has_sugar(&seq) { crate::sugar::seq(&seq).0 } else { seq };
                    out.insert(l.name.text.clone(), (seq, l.name.span));
                }
            }
        }
    }
    out
}

fn program_key(program: &Program) -> Key {
    program
        .stmts
        .iter()
        .find_map(|s| if let StmtKind::Setting(Setting::Key(k)) = &s.kind { Key::parse(&k.text) } else { None })
        .unwrap_or(Key::C_MAJOR)
}

/// Tokens of a pattern's text, placed at the `let` it came from (a diagnostic in it points there). Their offsets from one
/// another are kept: the reader tells a glued mark (`a1!`, `c2^`) from a spaced one by them.
/// A let's value as tokens, from its pattern: [`tokens_of`] its text, except that a name with a `.` in it (`acid@2.once1`,
/// `acid@2.R`, `theme.via1`: names only this lowering makes, which the lexer would cut at the `.`) stays one word.
fn seq_tokens(seq: &Seq, at: Span, src_len: usize) -> Vec<Token> {
    let mut names: Vec<String> = Vec::new();
    let mut s = seq.clone();
    walk_seq(&mut s, &mut |w| {
        if w.contains('.') {
            let k = names.iter().position(|n| n == w).unwrap_or_else(|| {
                names.push(w.clone());
                names.len() - 1
            });
            *w = format!("zzlowered{k}");
        }
    });
    let mut toks = tokens_of(&seq_text(&s), at, src_len);
    for t in &mut toks {
        if let Tok::Word(w) = &mut t.tok {
            if let Some(k) = w.strip_prefix("zzlowered").and_then(|k| k.parse::<usize>().ok()) {
                if let Some(n) = names.get(k) {
                    *w = n.clone();
                }
            }
        }
    }
    toks
}

fn tokens_of(text: &str, at: Span, src_len: usize) -> Vec<Token> {
    let base = at.start.min(src_len.saturating_sub(text.len()));
    crate::lex::lex(text).tokens.into_iter().filter(|t| !matches!(t.tok, Tok::Newline | Tok::Eof)).map(|t| Token { span: Span::new(base + t.span.start, base + t.span.end), ..t }).collect()
}

/// Rewrite the program's `arrange { }` (if it has one) into plain sections, `let`s and an `arrange` line, and say what
/// each play reads. `cid` seeds the operators' randomness: the program's own CID, of its canonical text.
pub fn apply(parsed: &mut Parsed, src: &str, cid: &str, diags: &mut Vec<Diagnostic>) -> Option<GenTable> {
    let program = &mut parsed.program;
    let at = program.stmts.iter().position(|s| matches!(s.kind, StmtKind::ArrangeBlock(_)))?;
    for st in &program.stmts[at + 1..] {
        if matches!(st.kind, StmtKind::ArrangeBlock(_) | StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_)) {
            diags.push(Diagnostic::error(st.span, "a song has one `arrange`").help("put every section and mutation in the one `arrange { }`"));
            return None;
        }
    }
    if matches!(&program.stmts[at].kind, StmtKind::ArrangeBlock(b) if b.named.is_none()) && program.stmts.iter().any(|s| matches!(s.kind, StmtKind::ArrangeNamed(_))) {
        diags.push(Diagnostic::error(program.stmts[at].span, "an `arrange { }` among named arrangements needs a name of its own").help("arrange set: {\n  drop\n  ~ acid rot 2\n  drop\n} then close"));
        return None;
    }
    for st in &program.stmts[..at] {
        if matches!(st.kind, StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_)) {
            diags.push(Diagnostic::error(st.span, "a song has one `arrange`").help("put every section and mutation in the one `arrange { }`"));
            return None;
        }
    }
    let StmtKind::ArrangeBlock(block) = program.stmts[at].kind.clone() else { unreachable!() };
    let block_span = program.stmts[at].span;
    let lets = pattern_lets(program, src);
    let pats: BTreeMap<String, Seq> = lets.iter().map(|(k, (s, _))| (k.clone(), s.clone())).collect();
    let key = program_key(program);
    // a drum pattern: read only by drum parts, or with no pitch in it (a chord is a pitch: `has_pitch`)
    let mut readers: BTreeMap<String, Readers> = BTreeMap::new();
    readers_of(&program.stmts, &mut readers);
    let drums: BTreeSet<String> = pats
        .iter()
        .filter(|(n, s)| {
            let r = readers.get(*n).map(|r| &r.roles);
            !has_pitch(&s.terms) || r.is_some_and(|r| !r.is_empty() && r.iter().all(|x| !TONAL.contains(&x.as_str())))
        })
        .map(|(n, _)| n.clone())
        .collect();
    // the roles that read each pattern, for `jux`'s notes
    let roles: BTreeMap<String, BTreeSet<String>> = readers.iter().map(|(n, r)| (n.clone(), r.roles.clone())).collect();
    // the octaves each pattern is read in: its `chord(...)` steps are voiced in the first (the program's reading order)
    let octaves: BTreeMap<String, Vec<i32>> = readers.into_iter().map(|(n, r)| (n, r.octaves)).collect();
    let mut sections: BTreeMap<String, usize> = BTreeMap::new();
    let mut other_names: BTreeSet<String> = BTreeSet::new();
    for (i, st) in program.stmts.iter().enumerate() {
        match &st.kind {
            StmtKind::Section(s) => {
                sections.insert(s.name.text.clone(), i);
                other_names.insert(s.name.text.clone());
            }
            StmtKind::Def(d) => {
                other_names.insert(d.name.text.clone());
            }
            StmtKind::Let(l) => {
                other_names.insert(l.name.text.clone());
            }
            _ => {}
        }
    }
    let checked = check_block(&block, &pats, &drums, &octaves, &other_names);
    let mut errors = checked.diags.iter().any(Diagnostic::is_error);
    diags.extend(checked.diags.iter().cloned());
    // the control deck's lines each section holds (through its repeats and the defs it calls), and those outside every section,
    // for the switches (`~ control off`)
    let mut sec_calls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if block.items.iter().any(|i| matches!(i, ArrItem::Mutation(m) if m.switch.is_some())) {
        let mut defs: BTreeMap<String, &[Token]> = BTreeMap::new();
        for d in parsed.imports.iter().flat_map(|i| i.defs.iter()) {
            defs.insert(d.name.text.clone(), &d.body);
        }
        let program = &parsed.program;
        for st in &program.stmts {
            if let StmtKind::Def(d) = &st.kind {
                defs.insert(d.name.text.clone(), &d.body);
            }
        }
        let mut headless = Vec::new();
        for st in &program.stmts {
            if let StmtKind::Section(s) = &st.kind {
                control_calls(&s.body, &defs, sec_calls.entry(s.name.text.clone()).or_default());
                headless.extend(headless_engine_lines(&s.body, src));
            }
        }
        let mut outside = BTreeSet::new();
        let top: Vec<Stmt> = program.stmts.iter().filter(|s| matches!(s.kind, StmtKind::Line(_) | StmtKind::Repeat(_))).cloned().collect();
        control_calls(&top, &defs, &mut outside);
        let in_sections: BTreeSet<String> = sec_calls.values().flatten().cloned().collect();
        let sd = switch_diags(&block, &in_sections, &outside, &headless);
        errors |= sd.iter().any(Diagnostic::is_error);
        diags.extend(sd);
    }
    let program = &mut parsed.program;
    if !errors {
        diags.extend(jux_notes(&block, &checked, &roles, program));
    }
    // the sections it names
    let coda: Vec<Word> = match &block.ends {
        BlockEnd::Passes { coda, .. } => coda.clone(),
        _ => Vec::new(),
    };
    let mut bad_section = false;
    let section_names: Vec<String> = sections.keys().cloned().collect();
    for w in block.items.iter().filter_map(|i| if let ArrItem::Section(w) = i { Some(w) } else { None }).chain(coda.iter()) {
        if !sections.contains_key(&w.text) {
            bad_section = true;
            let d = Diagnostic::error(w.span, format!("there is no section `{}`", w.text));
            diags.push(match crate::env::suggest(&w.text, &section_names, 1).first() {
                Some(n) => d.help(format!("did you mean `{n}`?")),
                None => d.help(format!("write it first: section {} 8 bars {{ ... }}", w.text)),
            });
        }
    }
    if errors || bad_section {
        return None;
    }

    // ---- walk the path: the generations, and what each play reads
    let (passes, unbounded) = match &block.ends {
        BlockEnd::End => (1, false),
        BlockEnd::Loop => (1, true),
        BlockEnd::Passes { n, .. } => (*n, false),
    };
    let mut table = GenTable { passes, unbounded, ..GenTable::default() };
    // every generation of every pattern a mutation touches
    let mut gens: BTreeMap<String, Vec<Pair>> = BTreeMap::new();
    let mut current: BTreeMap<String, u32> = BTreeMap::new();
    for (n, s) in &pats {
        gens.insert(n.clone(), vec![Pair::one(s.clone())]);
        current.insert(n.clone(), 0);
    }
    for (n, s) in &pats {
        table.patterns.entry(n.clone()).or_default().push(Gen { generation: 0, pass: None, mutation: None, ops: String::new(), text: seq_text(s), every: 1, right: None });
    }
    // a play: (section, pass, coda, the generation of each pattern, the one-shots over them)
    struct Raw {
        section: String,
        word: Word,
        pass: u32,
        coda: bool,
        at: BTreeMap<String, u32>,
        once: BTreeMap<String, Pair>,
        /// Which of its control-deck lines fire.
        ctl: Switches,
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut once: BTreeMap<String, Pair> = BTreeMap::new();
    // the switches of the control deck: from here on (`~`), and on top of that for the next section only (`@`)
    let mut ctl = Switches::default();
    let mut ctl_once: Option<Switches> = None;
    // One target's chain on a pattern. Operators before a `jux` work on the pattern (both copies, when it is already a
    // pair); at the `jux` the right copy is made from the left one and its operator works on it; what follows works on
    // the right copy only. The right copy has dice of its own, so the left one is what it would be without the `jux`.
    let run_chain = |name: &str, base: &Pair, t: &MutTarget, mi: usize, ti: usize, pass: u32, cur_pats: &BTreeMap<String, Pair>| -> Pair {
        let left_pats: BTreeMap<String, Seq> = cur_pats.iter().map(|(n, p)| (n.clone(), p.left.clone())).collect();
        let right_pats: BTreeMap<String, Seq> = cur_pats.iter().map(|(n, p)| (n.clone(), p.right_or_left().clone())).collect();
        let octave = octaves.get(name).and_then(|o| o.first().copied()).unwrap_or_else(|| crate::music::role_octave("harmony"));
        let (lctx, rctx) = (Ctx { key, others: &left_pats, octave }, Ctx { key, others: &right_pats, octave });
        let mut pair = base.clone();
        let mut forked = false;
        for (oi, op) in t.ops.iter().enumerate() {
            if checked.skip.contains(&(mi, ti, oi)) {
                continue;
            }
            if let Some(inner) = &op.inner {
                let mut rng = Rng::new_right(cid, pass, mi, ti, oi);
                pair.right = Some(apply_op(inner, &pair.left, &rctx, &mut rng));
                forked = true;
                continue;
            }
            if !forked {
                pair.left = apply_op(op, &pair.left, &lctx, &mut Rng::new(cid, pass, mi, ti, oi));
            }
            if let Some(r) = &pair.right {
                if forked || base.right.is_some() {
                    let next = apply_op(op, r, &rctx, &mut Rng::new_right(cid, pass, mi, ti, oi));
                    pair.right = Some(next);
                }
            }
        }
        pair
    };
    let current_pats = |gens: &BTreeMap<String, Vec<Pair>>, current: &BTreeMap<String, u32>| -> BTreeMap<String, Pair> { current.iter().map(|(n, g)| (n.clone(), gens[n][*g as usize].clone())).collect() };
    for pass in 0..passes {
        let mut mi = 0;
        for it in &block.items {
            match it {
                ArrItem::Section(w) => {
                    let sw = ctl_once.take().unwrap_or_else(|| ctl.clone());
                    raws.push(Raw { section: w.text.clone(), word: w.clone(), pass, coda: false, at: current.clone(), once: std::mem::take(&mut once), ctl: sw });
                }
                ArrItem::Mutation(m) => {
                    // `~3`: only on every 3rd pass. The line still counts (`mi`), so the lines after it roll the same dice
                    if (pass + 1) % m.every != 0 {
                        mi += 1;
                        continue;
                    }
                    // a switch of the control deck: no pattern changes, the sections after it (or the next one) fire fewer lines
                    if let Some(s) = &m.switch {
                        if m.every > 1 {
                            record_fire(&mut table.fires, mi, m, pass, raws.len());
                        }
                        if m.once {
                            ctl_once = Some(ctl_once.take().unwrap_or_else(|| ctl.clone()).with(s));
                        } else {
                            ctl = ctl.with(s);
                        }
                        mi += 1;
                        continue;
                    }
                    let cur = current_pats(&gens, &current);
                    if m.every > 1 && (0..m.targets.len()).any(|ti| checked.resolved.get(&(mi, ti)).is_some_and(|h| !h.is_empty())) {
                        record_fire(&mut table.fires, mi, m, pass, raws.len());
                    }
                    for (ti, t) in m.targets.iter().enumerate() {
                        for name in checked.resolved.get(&(mi, ti)).cloned().unwrap_or_default() {
                            let base = if m.once { once.get(&name).cloned().unwrap_or_else(|| cur[&name].clone()) } else { gens[&name][current[&name] as usize].clone() };
                            let next = run_chain(&name, &base, t, mi, ti, pass, &cur);
                            if m.once {
                                once.insert(name, next);
                            } else {
                                let list = gens.get_mut(&name).unwrap();
                                list.push(next.clone());
                                let g = (list.len() - 1) as u32;
                                current.insert(name.clone(), g);
                                table.patterns.entry(name).or_default().push(Gen {
                                    generation: g,
                                    pass: Some(pass),
                                    mutation: Some(mi),
                                    ops: t.ops.iter().map(MutOp::text).collect::<Vec<_>>().join(" | "),
                                    text: seq_text(&next.left),
                                    every: m.every,
                                    right: next.right.as_ref().map(seq_text),
                                });
                            }
                        }
                    }
                    mi += 1;
                }
            }
        }
    }
    for w in &coda {
        let sw = ctl_once.take().unwrap_or_else(|| ctl.clone());
        raws.push(Raw { section: w.text.clone(), word: w.clone(), pass: passes.saturating_sub(1), coda: true, at: current.clone(), once: std::mem::take(&mut once), ctl: sw });
    }
    if !once.is_empty() || ctl_once.is_some() {
        diags.push(Diagnostic::warning(block.span, "a one-shot `@` mutation at the end of the `arrange` has no next section to play in"));
    }
    // keep only the patterns a mutation touched in the table
    table.patterns.retain(|_, v| v.len() > 1);
    table.fires.sort_by_key(|f| f.mutation);
    for p in table.fires.iter_mut().flat_map(|f| f.plays.iter_mut()) {
        if p.is_some_and(|i| i >= raws.len()) {
            *p = None;
        }
    }

    // ---- lower it: a `let` per generation read, a section per distinct reading
    // which patterns each section reads (its own lines), and its generation references (`acid@2`)
    let mut reads_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut gen_refs: Vec<(String, u32, Span)> = Vec::new();
    for (name, i) in &sections {
        let mut body = match &program.stmts[*i].kind {
            StmtKind::Section(s) => s.body.clone(),
            _ => unreachable!(),
        };
        let mut set = BTreeSet::new();
        let span = program.stmts[*i].span;
        walk_words(&mut body, &mut |w| {
            if pats.contains_key(w.as_str()) {
                set.insert(w.clone());
            } else if let Some((base, n)) = w.rsplit_once('@') {
                if let (true, Ok(n)) = (pats.contains_key(base), n.parse::<u32>()) {
                    gen_refs.push((base.to_string(), n, span));
                }
            }
        });
        reads_of.insert(name.clone(), set);
    }
    // a let that names other pattern lets (`let theme = <theme_1 theme_2>`) is read through: a section that plays `theme` reads
    // `theme_1` and `theme_2` too, so a mutation of the bars (`~ theme_* rot 2`) reaches the part that plays the alternation
    // (forms.md: "a part plays the alternation; a mutation changes the bars")
    let names_in = |s: &Seq| {
        let mut out = BTreeSet::new();
        let mut c = s.clone();
        walk_seq(&mut c, &mut |w| {
            if pats.contains_key(w.as_str()) {
                out.insert(w.clone());
            }
        });
        out
    };
    let inner: BTreeMap<String, BTreeSet<String>> = pats.iter().map(|(k, s)| (k.clone(), names_in(s))).collect();
    let direct_of = reads_of.clone();
    for set in reads_of.values_mut() {
        let mut todo: Vec<String> = set.iter().cloned().collect();
        while let Some(w) = todo.pop() {
            for n in inner.get(&w).into_iter().flatten() {
                if set.insert(n.clone()) {
                    todo.push(n.clone());
                }
            }
        }
    }
    // the derived lets that read through, by their text: the same reading of the same let is one let; written after the
    // generations they read, in the order they were made (innermost first)
    let mut via: BTreeMap<String, String> = BTreeMap::new();
    let mut via_lets: Vec<(String, Seq)> = Vec::new();
    let mut new_lets: BTreeMap<String, Seq> = BTreeMap::new();
    // a generation that is a pair has its right copy as a let of its own (`acid@3.R`): the left name -> the right one
    let mut right_of: BTreeMap<String, String> = BTreeMap::new();
    let mut put_let = |new_lets: &mut BTreeMap<String, Seq>, name: String, pair: &Pair| {
        new_lets.insert(name.clone(), pair.left.clone());
        if let Some(r) = &pair.right {
            let right = format!("{name}.R");
            new_lets.insert(right.clone(), r.clone());
            right_of.insert(name, right);
        }
    };
    let mut gen_ok = true;
    for (base, n, span) in &gen_refs {
        let have = gens[base].len() as u32;
        if *n >= have {
            gen_ok = false;
            diags.push(Diagnostic::error(*span, format!("`{base}@{n}`: `{base}` has generations 0 to {} in this `arrange`", have - 1)).help(format!("`{base}@0` is `{base}` as written")));
        } else if *n > 0 {
            put_let(&mut new_lets, gen_name(base, *n), &gens[base][*n as usize]);
        } else {
            put_let(&mut new_lets, format!("{base}@0"), &gens[base][0]);
        }
    }
    if !gen_ok {
        return None;
    }
    // each play's reading: pattern -> the let it reads, and the calls whose control-deck lines it holds back
    type Reading = (BTreeMap<String, String>, Vec<String>);
    let mut variants: BTreeMap<String, Vec<Reading>> = BTreeMap::new();
    let mut words: Vec<Word> = Vec::new();
    let mut once_count: BTreeMap<String, u32> = BTreeMap::new();
    // a named block is one arrangement of several: a section the others play as written keeps its name and its text, so
    // the block's mutated readings are the variants after it (`drop@2`)
    if block.named.is_some() {
        for r in &raws {
            variants.entry(r.section.clone()).or_insert_with(|| vec![(BTreeMap::new(), Vec::new())]);
        }
    }
    for r in &raws {
        // the section's control-deck lines this play's switches hold back (a section with none is never copied for a switch)
        let quiet: Vec<String> = sec_calls.get(&r.section).into_iter().flatten().filter(|c| !r.ctl.fires(c)).cloned().collect();
        let mut reading: BTreeMap<String, String> = BTreeMap::new();
        let mut reads: Vec<Read> = Vec::new();
        for p in &reads_of[&r.section] {
            let g = r.at.get(p).copied().unwrap_or(0);
            let (letname, shown, is_once) = match r.once.get(p) {
                Some(s) => {
                    let k = once_count.entry(p.clone()).or_insert(0);
                    *k += 1;
                    let n = format!("{}.once{}", gen_name(p, g), k);
                    put_let(&mut new_lets, n.clone(), s);
                    (n, s, true)
                }
                None => {
                    let n = gen_name(p, g);
                    if g > 0 {
                        put_let(&mut new_lets, n.clone(), &gens[p][g as usize]);
                    }
                    (n, &gens[p][g as usize], false)
                }
            };
            if table.patterns.contains_key(p) || is_once {
                reads.push(Read { pattern: p.clone(), generation: g, once: is_once, text: seq_text(&shown.left), right: shown.right.as_ref().map(seq_text) });
            }
            if letname != *p {
                reading.insert(p.clone(), letname);
            }
        }
        // the lets this section writes that read lets this play has renamed: a let of their own, reading them as it has them
        // (`theme.via1` = `<theme_1@2 theme_2@2>`), innermost first
        let mut derived: BTreeMap<String, String> = BTreeMap::new();
        #[allow(clippy::too_many_arguments)]
        fn derive(q: &str, reading: &BTreeMap<String, String>, pats: &BTreeMap<String, Seq>, inner: &BTreeMap<String, BTreeSet<String>>, new_lets: &BTreeMap<String, Seq>, via: &mut BTreeMap<String, String>, via_lets: &mut Vec<(String, Seq)>, derived: &mut BTreeMap<String, String>, depth: usize) -> Option<String> {
            if let Some(n) = derived.get(q) {
                return (!n.is_empty()).then(|| n.clone());
            }
            if depth > 16 {
                return None;
            }
            let mut map: BTreeMap<String, String> = BTreeMap::new();
            for w in inner.get(q).into_iter().flatten() {
                if w == q {
                    continue;
                }
                if let Some(d) = derive(w, reading, pats, inner, new_lets, via, via_lets, derived, depth + 1) {
                    map.insert(w.clone(), d);
                } else if let Some(r) = reading.get(w) {
                    map.insert(w.clone(), r.clone());
                }
            }
            if map.is_empty() {
                derived.insert(q.to_string(), String::new());
                return None;
            }
            let base = reading.get(q).and_then(|n| new_lets.get(n).or_else(|| via_lets.iter().find(|(m, _)| m == n).map(|(_, s)| s))).or_else(|| pats.get(q)).cloned()?;
            let mut seq = base;
            walk_seq(&mut seq, &mut |w| {
                if let Some(n) = map.get(w.as_str()) {
                    *w = n.clone();
                }
            });
            let key = format!("{q}={}", seq_text(&seq));
            let name = match via.get(&key) {
                Some(n) => n.clone(),
                None => {
                    let n = format!("{}.via{}", reading.get(q).map_or(q, String::as_str), via.len() + 1);
                    via.insert(key, n.clone());
                    via_lets.push((n.clone(), seq));
                    n
                }
            };
            derived.insert(q.to_string(), name.clone());
            Some(name)
        }
        for q in &direct_of[&r.section] {
            if let Some(n) = derive(q, &reading, &pats, &inner, &new_lets, &mut via, &mut via_lets, &mut derived, 0) {
                reading.insert(q.clone(), n);
            }
        }
        let list = variants.entry(r.section.clone()).or_default();
        // a one-shot reading is its own variant every time (its let is its own)
        let reading = (reading, quiet);
        let idx = match list.iter().position(|v| *v == reading) {
            Some(i) => i,
            None => {
                list.push(reading.clone());
                list.len() - 1
            }
        };
        let name = if idx == 0 { r.section.clone() } else { format!("{}@{}", r.section, idx + 1) };
        words.push(Word { text: name.clone(), span: r.word.span });
        table.plays.push(Play { section: r.section.clone(), name, pass: r.pass, coda: r.coda, reads, control_off: reading.1 });
    }
    // the sections: the first variant in place, the others after it
    let mut inserts: Vec<(usize, Stmt)> = Vec::new();
    for (sec, list) in &variants {
        let i = sections[sec];
        let rename = |body: &mut Vec<Stmt>, reading: &BTreeMap<String, String>| {
            walk_words(body, &mut |w| {
                if let Some(n) = reading.get(w.as_str()) {
                    *w = n.clone();
                }
            });
        };
        let original = program.stmts[i].clone();
        for (k, (reading, quiet)) in list.iter().enumerate() {
            let mut st = original.clone();
            if let StmtKind::Section(s) = &mut st.kind {
                rename(&mut s.body, reading);
                s.quiet = quiet.clone();
                if k > 0 {
                    s.name.text = format!("{sec}@{}", k + 1);
                }
            }
            if k == 0 {
                program.stmts[i] = st;
            } else {
                inserts.push((i, st));
            }
        }
    }
    // generation references (`acid@2`) read their own lets
    for i in sections.values() {
        if let StmtKind::Section(s) = &mut program.stmts[*i].kind {
            walk_words(&mut s.body, &mut |w| {
                if let Some(b) = w.strip_suffix("@0") {
                    if pats.contains_key(b) {
                        *w = format!("{b}@0");
                    }
                }
            });
        }
    }
    inserts.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
    // the arrange line, and `loop` for an unbounded one
    let mut at = at;
    program.stmts[at] = match &block.named {
        Some((name, ends)) => {
            table.arrangement = Some(name.text.clone());
            let items = words.into_iter().map(crate::algebra::SExpr::Name).collect();
            Stmt { span: block_span, kind: StmtKind::ArrangeNamed(crate::ast::ArrDecl { name: name.clone(), items, ends: ends.clone() }) }
        }
        None => Stmt { span: block_span, kind: StmtKind::Arrange(words) },
    };
    if unbounded {
        program.stmts.insert(at + 1, Stmt { span: block_span, kind: StmtKind::Loop });
    }
    for (i, st) in inserts {
        program.stmts.insert(i + 1, st);
        if i < at {
            at += 1;
        }
    }
    let _ = at;
    // every part that plays a pair gets the right copy as a twin right after it
    if !right_of.is_empty() {
        for st in &mut program.stmts {
            if let StmtKind::Section(sec) = &mut st.kind {
                add_twins(&mut sec.body, &right_of);
            }
        }
    }
    for (name, seq) in new_lets.into_iter().chain(via_lets) {
        let span = lets.get(name.split(['@', '.']).next().unwrap_or(&name)).map_or(block_span, |(_, sp)| *sp);
        program.stmts.push(Stmt { span, kind: StmtKind::Let(LetDef { name: Word { text: name, span }, value: seq_tokens(&seq, span, src.len()), ratio: false }) });
    }
    Some(table)
}

/// The generation table of a program's `arrange { }`, worked out from its text alone (None: it has none, or it does
/// not check). The same text is the same table.
pub fn generations(src: &str, env: &crate::env::Env) -> Option<GenTable> {
    // a program with references (a generator's let, a borrowed section, a template): the table of what they are written out
    // as, read as `check` reads it (compile.rs: the written-out text, with its CID's dice), so a referenced let's generations
    // are reported like a local one's
    if let Some(Ok((text, _, _))) = crate::stored::expand_text(src, env) {
        if text != src {
            return generations(&text, env);
        }
    }
    let mut parsed = crate::parse::parse_with(src, env);
    if parsed.diagnostics.iter().any(Diagnostic::is_error) {
        return None;
    }
    let cid = crate::compile::sound_cid(&crate::print::canonical(&parsed.program));
    let mut diags = Vec::new();
    let t = apply(&mut parsed, src, &cid, &mut diags)?;
    (!diags.iter().any(Diagnostic::is_error)).then_some(t)
}

/// The fixer: each operator of an `arrange { }` that is not one (or has the wrong arguments) is put out of action as
/// `id`, the identity, and what was written is kept in a comment at the end of its line, not deleted. None: nothing to fix.
pub fn fix(src: &str) -> Option<String> {
    let parsed = crate::parse::parse(src);
    let block = parsed.program.stmts.iter().find_map(|s| if let StmtKind::ArrangeBlock(b) = &s.kind { Some(b.clone()) } else { None })?;
    let lets = pattern_lets(&parsed.program, src);
    let pats: BTreeMap<String, Seq> = lets.into_iter().map(|(k, (s, _))| (k, s)).collect();
    let checked = check_block(&block, &pats, &BTreeSet::new(), &BTreeMap::new(), &BTreeSet::new());
    if checked.bad_ops.is_empty() {
        return None;
    }
    let mut out = src.to_string();
    // from the end, so the earlier spans stay where they are
    let mut bad = checked.bad_ops.clone();
    bad.sort_by_key(|(s, _)| std::cmp::Reverse(s.start));
    // comments go at line ends: gather them per line end first
    let mut notes: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (sp, _) in &bad {
        let eol = src[sp.end..].find('\n').map_or(src.len(), |n| sp.end + n);
        notes.entry(eol).or_default().insert(0, sp.text(src).to_string());
    }
    let mut edits: Vec<(usize, usize, String)> = bad.iter().map(|(sp, _)| (sp.start, sp.end, "id".to_string())).collect();
    for (eol, was) in notes {
        edits.push((eol, eol, format!("  # fixed: was {}", was.iter().map(|w| format!("`{w}`")).collect::<Vec<_>>().join(", "))));
    }
    edits.sort_by_key(|(s, e, _)| std::cmp::Reverse((*s, *e)));
    for (s, e, t) in edits {
        out.replace_range(s..e, &t);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The left copy's dice (and every program without a `jux`'s) are the stream they always were: these draws are what an
    /// independent implementation of the hash (FNV-1a of the CID, pass, mutation, target and operator) and splitmix64 gives.
    #[test]
    fn the_dice_of_the_left_copy_are_the_ones_they_always_were() {
        let mut r = Rng::new("z4EBG9jexample", 1, 2, 3, 4);
        assert_eq!([r.next(), r.next(), r.next()], [0x63c8_952d_4b26_25f8, 0x1bbd_01f1_d31f_4b51, 0xf85c_babd_16c5_0375]);
    }

    /// The right copy draws from another stream, one extra byte into the same hash, and the same way every time.
    #[test]
    fn the_right_copy_draws_from_its_own_stream() {
        let mut r = Rng::new_right("z4EBG9jexample", 1, 2, 3, 4);
        assert_eq!([r.next(), r.next(), r.next()], [0xc6ae_e600_2894_da10, 0x8583_5119_337b_6169, 0x1a17_168b_632a_25e3]);
        assert_ne!(Rng::new("z4EBG9jexample", 1, 2, 3, 4).next(), Rng::new_right("z4EBG9jexample", 1, 2, 3, 4).next());
    }
}
