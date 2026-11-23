//! F11 typed ports on defs, `wire`, and imports glued along ports (docs/beatscript/ext-ports.md,
//! docs/BEATSCRIPT-EXTENSIONS.md section 3, F11).
//!
//! A `def` may declare a boundary: `def walker(oct) in: chords, key out: bass { ... }`. The `in:` ports are values
//! the def needs and the `out:` ports are values it makes with `emit`. `wire changes.out -> walker.chords` glues
//! an output to an input. The wires are a DAG of defs; a def's out value is a function of its in values and its
//! own text (a parameter never reaches an `emit`, so the value is the same wherever the def is wired); a call of a
//! def with ports expands its body with `$chords` (or `$in`) standing for the wired value. A def with no ports
//! is the macro it always was.
//!
//! Where things are:
//!
//! * [`PortType`], [`Ports`], [`WireStmt`]: what the parser makes (the def header, the `wire` statement), what
//!   `print` writes (one canonical spelling: ports sorted by type, a sole port named `in` / `out`).
//! * `impl Parser`: the header and the `wire` statement.
//! * `impl Lower`: the checks (every `emit`, every wire: types, once per input, no loop) and the expansion (an
//!   in port's tokens put where `$chords` stands, an out port's tokens made on demand and kept).
//! * [`wiring_cycles`]: the loop finder, a pure function so a law can be tested against an oracle.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Arg, Atom, DefDef, Seq, Stmt, StmtKind, Word};
use crate::diag::{line_col, Diagnostic, Span};
use crate::expand::Lower;
use crate::lex::{Tok, Token};
use crate::music::{Chord, Key};
use crate::parse::{nearest, parse_seq_tokens, Imported, Parser};

type R<T> = Result<T, Diagnostic>;

/// How deep a chain of wired defs may go while a value is made (the wiring is a DAG, so this only stops a
/// mistake in this module from running away).
const MAX_CHAIN: usize = 64;

// ---- the types

/// What a port carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortType {
    /// Chord symbols (`am7`) and Roman numerals (`V7`), as a progression: `<chord(am7) chord(g7)>`.
    Chords,
    /// A key word: `am`, `d:dorian`.
    Key,
    /// A pattern for a bass part.
    Bass,
    /// A pattern for a melody part.
    Melody,
    /// A pattern of hits and rests for a drum part.
    Drums,
    /// A pattern of hits and rests that fits any part (a hit in a tonal part plays its root).
    Rhythm,
    /// A number of bars.
    Bars,
    /// A tempo in BPM.
    Tempo,
    /// Any pattern: the type of a port that takes a bass, a melody, drums or a rhythm.
    Pattern,
}

impl PortType {
    /// In the order the contract lists them: the order ports are printed in.
    pub const ALL: [PortType; 9] = [PortType::Chords, PortType::Key, PortType::Bass, PortType::Melody, PortType::Drums, PortType::Rhythm, PortType::Bars, PortType::Tempo, PortType::Pattern];

    pub fn name(self) -> &'static str {
        match self {
            PortType::Chords => "chords",
            PortType::Key => "key",
            PortType::Bass => "bass",
            PortType::Melody => "melody",
            PortType::Drums => "drums",
            PortType::Rhythm => "rhythm",
            PortType::Bars => "bars",
            PortType::Tempo => "tempo",
            PortType::Pattern => "pattern",
        }
    }

    pub fn parse(s: &str) -> Option<PortType> {
        PortType::ALL.iter().copied().find(|t| t.name() == s)
    }

    /// `bass`, `melody`, `drums`, `rhythm` and `pattern` carry patterns.
    pub fn is_pattern(self) -> bool {
        matches!(self, PortType::Bass | PortType::Melody | PortType::Drums | PortType::Rhythm | PortType::Pattern)
    }

    /// Can an input of this type take what an output of type `have` makes? The same type, any pattern kind into
    /// a `pattern` input, and a `rhythm` (hits and rests only, so it fits any part) into a bass, melody or drums
    /// input. Nothing else: chords are not a key and a bass is not drums.
    pub fn accepts(self, have: PortType) -> bool {
        self == have || (self == PortType::Pattern && have.is_pattern()) || (have == PortType::Rhythm && matches!(self, PortType::Bass | PortType::Melody | PortType::Drums))
    }

    /// How an output of this type is told of in a type error: "`drums.out` is a drums pattern".
    pub fn is_a(self) -> &'static str {
        match self {
            PortType::Chords => "chords",
            PortType::Key => "a key",
            PortType::Bass => "a bass pattern",
            PortType::Melody => "a melody pattern",
            PortType::Drums => "a drums pattern",
            PortType::Rhythm => "a rhythm pattern",
            PortType::Bars => "a bar count",
            PortType::Tempo => "a tempo",
            PortType::Pattern => "a pattern",
        }
    }

    /// How an input of this type is told of: "`walker.chords` wants chords".
    pub fn wants(self) -> &'static str {
        match self {
            PortType::Bars => "bars",
            PortType::Pattern => "any pattern",
            other => other.is_a(),
        }
    }

    /// A help line: what a port of this type carries.
    pub fn carries(self) -> String {
        format!("a `{}` port carries {}", self.name(), self.made_of())
    }

    /// What a value of this type is made of.
    pub fn made_of(self) -> &'static str {
        match self {
            PortType::Chords => "chord symbols like am7 and fmaj7, Roman numerals like V7, or chord(am7) calls, as a pattern",
            PortType::Key => "one key word: am, f#m, d:dorian",
            PortType::Bass | PortType::Melody | PortType::Pattern => "a pattern: notes, degrees, x and ~, groups, alternations, generators",
            PortType::Drums | PortType::Rhythm => "hits and rests: x ~ [x x] e(5,8)",
            PortType::Bars => "a whole number of bars, 1 to 256",
            PortType::Tempo => "a tempo in BPM, 40 to 300",
        }
    }
}

/// A side of a def's boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dir {
    In,
    Out,
}

impl Dir {
    pub fn word(self) -> &'static str {
        match self {
            Dir::In => "in",
            Dir::Out => "out",
        }
    }

    pub fn other(self) -> Dir {
        match self {
            Dir::In => Dir::Out,
            Dir::Out => Dir::In,
        }
    }
}

// ---- the declaration

/// One port of a def: its type, and where the type is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub ty: PortType,
    pub span: Span,
}

/// A def's boundary: the interface of the open system the def is. At most one port of a type on a side, so a port
/// is named by its type (or `in` / `out` when it is the only one on its side). Kept sorted by type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ports {
    pub ins: Vec<Port>,
    pub outs: Vec<Port>,
}

impl Ports {
    pub fn is_empty(&self) -> bool {
        self.ins.is_empty() && self.outs.is_empty()
    }

    pub fn side(&self, d: Dir) -> &[Port] {
        match d {
            Dir::In => &self.ins,
            Dir::Out => &self.outs,
        }
    }

    pub fn side_mut(&mut self, d: Dir) -> &mut Vec<Port> {
        match d {
            Dir::In => &mut self.ins,
            Dir::Out => &mut self.outs,
        }
    }

    /// The port a name means on a side: a type's name, or `in` / `out` when it is the only port there.
    pub fn find(&self, d: Dir, name: &str) -> Option<&Port> {
        let side = self.side(d);
        side.iter().find(|p| p.ty.name() == name).or_else(|| (side.len() == 1 && name == d.word()).then(|| &side[0]))
    }

    /// The one spelling a port has in canonical text: `in` / `out` when it is alone on its side, else its type.
    pub fn name_of(&self, d: Dir, p: &Port) -> &'static str {
        if self.side(d).len() == 1 {
            d.word()
        } else {
            p.ty.name()
        }
    }

    /// Every canonical port name of a side, in order.
    pub fn names(&self, d: Dir) -> Vec<&'static str> {
        self.side(d).iter().map(|p| self.name_of(d, p)).collect()
    }

    /// The names `$name` may take for an in port: its type, and `in` when it is the only one.
    pub fn dollar_names(&self, p: &Port) -> Vec<&'static str> {
        let mut v = vec![p.ty.name()];
        if self.ins.len() == 1 {
            v.push("in");
        }
        v
    }

    /// The header as canonical text: ` in: chords, key out: bass` (empty when there are no ports).
    pub fn header(&self) -> String {
        let mut s = String::new();
        if !self.ins.is_empty() {
            s.push_str(&format!(" in: {}", self.ins.iter().map(|p| p.ty.name()).collect::<Vec<_>>().join(", ")));
        }
        if !self.outs.is_empty() {
            s.push_str(&format!(" out: {}", self.outs.iter().map(|p| p.ty.name()).collect::<Vec<_>>().join(", ")));
        }
        s
    }
}

/// `changes.out`: a def's name and one of its ports'.
#[derive(Clone, Debug, PartialEq)]
pub struct PortRef {
    pub def: Word,
    pub port: Word,
}

/// `wire changes.out -> walker.chords`.
#[derive(Clone, Debug, PartialEq)]
pub struct WireStmt {
    pub from: PortRef,
    pub to: PortRef,
}

impl WireStmt {
    /// The canonical text of the statement.
    pub fn text(&self) -> String {
        format!("wire {}.{} -> {}.{}", self.from.def.text, self.from.port.text, self.to.def.text, self.to.port.text)
    }

    fn pair_span(&self) -> Span {
        self.from.def.span.to(self.to.port.span)
    }
}

// ---- parsing

impl Parser<'_> {
    /// The `in: ...` and `out: ...` of a def's header (after its `)`), if any.
    pub(crate) fn def_ports(&mut self) -> R<Ports> {
        let mut ports = Ports::default();
        for d in [Dir::In, Dir::Out] {
            if self.word_is(d.word()) {
                self.bump();
                if self.peek() != &Tok::Colon {
                    return Err(self.expected(&["`:`"], &format!("`{}` is followed by a colon and the port types, found {}", d.word(), self.found())).help(format!("def walker(oct) {}: chords", d.word())));
                }
                self.bump();
                *ports.side_mut(d) = self.port_list(d)?;
            }
        }
        if self.word_is("in") || self.word_is("out") {
            let which = if self.word_is("in") { "in" } else { "out" };
            let why = if which == "in" && !ports.outs.is_empty() { "write `in:` before `out:`" } else { "each of `in:` and `out:` is said once" };
            return Err(Diagnostic::error(self.span(), format!("`{which}:` is in the wrong place in the def's header")).help(why).found(self.found()));
        }
        Ok(ports)
    }

    fn port_list(&mut self, d: Dir) -> R<Vec<Port>> {
        let type_names: Vec<&str> = PortType::ALL.iter().map(|t| t.name()).collect();
        let mut v: Vec<Port> = Vec::new();
        loop {
            let Tok::Word(text) = self.peek().clone() else {
                return Err(self.expected(&type_names, &format!("expected a port type after `{}:`, found {}", d.word(), self.found())).help(format!("def walker(oct) {}: chords, key", d.word())));
            };
            let span = self.bump().span;
            let Some(ty) = PortType::parse(&text) else {
                let mut diag = Diagnostic::error(span, format!("`{text}` is not a port type")).expected(&type_names).found(format!("`{text}`"));
                if text == "in" || text == "out" {
                    diag = diag.help("each of `in:` and `out:` is said once, `in:` first");
                } else if let Some(n) = nearest(&text, type_names.iter().copied()) {
                    diag = diag.help(format!("did you mean `{n}`?"));
                }
                return Err(diag);
            };
            if let Some(prev) = v.iter().find(|p| p.ty == ty) {
                let at = line_col(self.src, prev.span.start);
                return Err(Diagnostic::error(span, format!("`{}` is already a port of `{}:` (line {})", ty.name(), d.word(), at.0))
                    .help("a port is named by its type, so each type is on a side once (`in:` and `out:` may each have it)"));
            }
            v.push(Port { ty, span });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        v.sort_by_key(|p| p.ty);
        Ok(v)
    }

    /// `wire a.out -> b.chords` (the `wire` word is the next token).
    pub(crate) fn wire_statement(&mut self, start: Span) -> R<Stmt> {
        self.bump();
        let from = self.port_ref()?;
        if self.peek() != &Tok::Arrow {
            return Err(self
                .expected(&["`->`"], &format!("a wire goes from an output to an input: expected `->`, found {}", self.found()))
                .help("wire changes.out -> walker.chords"));
        }
        self.bump();
        let to = self.port_ref()?;
        self.end_of_statement()?;
        Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Wire(WireStmt { from, to }) })
    }

    fn port_ref(&mut self) -> R<PortRef> {
        let def = self.word("the name of a def")?;
        if self.peek() != &Tok::Dot {
            return Err(self
                .expected(&["`.` and a port"], &format!("`{}` needs a port: `{}.out`, found {}", def.text, def.text, self.found()))
                .help("wire changes.out -> walker.chords"));
        }
        self.bump();
        let port = self.word("a port name (`out`, `in`, or a type like `chords`)")?;
        Ok(PortRef { def, port })
    }
}

/// After a parse: rewrite each wire's port names to their canonical spelling (a sole port is `in` / `out`, else
/// its type), where the def is known, so two spellings of one wire are one program. A name that cannot be found
/// is left as written; the lowering says what is wrong with it.
pub(crate) fn canonicalize(stmts: &mut [Stmt], imports: &[Imported]) {
    let mut known: BTreeMap<String, Ports> = BTreeMap::new();
    for imp in imports {
        for d in &imp.defs {
            known.insert(d.name.text.clone(), d.ports.clone());
        }
    }
    for st in stmts.iter() {
        if let StmtKind::Def(d) = &st.kind {
            known.insert(d.name.text.clone(), d.ports.clone());
        }
    }
    for st in stmts.iter_mut() {
        if let StmtKind::Wire(w) = &mut st.kind {
            for (r, d) in [(&mut w.from, Dir::Out), (&mut w.to, Dir::In)] {
                if let Some(ports) = known.get(&r.def.text) {
                    if let Some(p) = ports.find(d, &r.port.text) {
                        r.port.text = ports.name_of(d, p).to_string();
                    }
                }
            }
        }
    }
}

// ---- emit statements

/// `emit [PORT] tokens`: a statement of a def's body that makes an out port's value.
pub(crate) struct Emit {
    /// The `emit` word through the last token.
    pub(crate) span: Span,
    pub(crate) word: Span,
    /// Everything after `emit`: the port's name (maybe) and the value.
    pub(crate) toks: Vec<Token>,
}

pub(crate) struct Split {
    pub(crate) emits: Vec<Emit>,
    /// The body without its `emit` statements: the lines the def expands to.
    pub(crate) rest: Vec<Token>,
    /// Where an `emit` starts a statement inside a block (not allowed).
    pub(crate) nested: Vec<Span>,
}

/// A def body cut into its `emit` statements (at the top of the body: the first word of a statement) and the rest.
pub(crate) fn split_emits(body: &[Token]) -> Split {
    let mut out = Split { emits: Vec::new(), rest: Vec::new(), nested: Vec::new() };
    let mut depth = 0i32;
    let mut at_start = true;
    let mut i = 0;
    while i < body.len() {
        let t = &body[i];
        if at_start && matches!(&t.tok, Tok::Word(w) if w == "emit") {
            if depth == 0 {
                let mut j = i + 1;
                while j < body.len() && body[j].tok != Tok::Newline {
                    j += 1;
                }
                let toks = body[i + 1..j].to_vec();
                let last = toks.last().map_or(t.span, |l| l.span);
                out.emits.push(Emit { span: t.span.to(last), word: t.span, toks });
                i = j + 1;
                at_start = true;
                continue;
            }
            out.nested.push(t.span);
        }
        match &t.tok {
            Tok::Newline => at_start = true,
            Tok::LBrace => {
                depth += 1;
                at_start = true;
            }
            Tok::RBrace => {
                depth -= 1;
                at_start = false;
            }
            _ => at_start = false,
        }
        out.rest.push(t.clone());
        i += 1;
    }
    out
}

// ---- checking a value against a type

/// A word that reads as a chord: a symbol (`am7`, `fmaj7`) or a Roman numeral (`V7`, `ii°`, `bVII`, `#iv`) in exactly
/// the spellings a `harmony` part reads (F5, `scale::is_roman`).
pub fn is_chord_word(w: &str) -> bool {
    Chord::parse(w).is_some() || is_roman(w)
}

/// `I`, `ii`, `V7`, `vii°`, `bVII`, `#iv`, `Imaj7`: a Roman numeral as F5 (`scale.rs`) reads one, so that a progression a
/// port takes and the part that plays it agree on every spelling.
pub fn is_roman(w: &str) -> bool {
    crate::scale::is_roman(w)
}

/// The matching `)` of the `(` at `open`, if there is one.
fn close_paren(toks: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in toks.iter().enumerate().skip(open) {
        match t.tok {
            Tok::LParen => depth += 1,
            Tok::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn check_chords(toks: &[Token]) -> Result<(), (Span, String)> {
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match &t.tok {
            Tok::Dollar(_) | Tok::Tilde | Tok::LBrack | Tok::RBrack | Tok::Lt | Tok::Gt | Tok::Comma | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp => {}
            Tok::Star => match toks.get(i + 1).map(|n| &n.tok) {
                // `*2`
                Some(Tok::Num { int: true, .. } | Tok::Dollar(_)) => i += 1,
                _ => return Err((t.span, "`*` is followed by a whole number".to_string())),
            },
            Tok::Slash => match toks.get(i + 1).map(|n| &n.tok) {
                // an inversion: `I/3`
                Some(Tok::Num { value, int: true }) if [3.0, 5.0, 7.0].contains(value) => i += 1,
                _ => return Err((t.span, "a `/` in chords is an inversion: `I/3` (first), `I/5` (second) or `V7/7` (third)".to_string())),
            },
            Tok::Word(w) if toks.get(i + 1).map(|n| &n.tok) == Some(&Tok::LParen) => {
                if w != "chord" {
                    return Err((t.span, format!("`{w}(...)` is not a chord: the call that makes chords is `chord(am7)`")));
                }
                let Some(close) = close_paren(toks, i + 1) else { return Err((t.span, "this `chord(` is never closed".to_string())) };
                match toks.get(i + 2).map(|n| (&n.tok, n.span)) {
                    Some((Tok::Dollar(_), _)) => {}
                    Some((Tok::Word(c), _)) if is_chord_word(c) => {}
                    Some((other, sp)) => return Err((sp, format!("`chord(` takes a chord like am7, found {}", other.describe()))),
                    None => return Err((t.span, "`chord(` takes a chord like am7".to_string())),
                }
                i = close;
            }
            Tok::Word(w) => {
                if !is_chord_word(w) {
                    let why = if w == "x" {
                        "`x` is a hit: chords are symbols like am7"
                    } else if crate::music::parse_note(w).is_some() {
                        "that is a note: a note belongs in a bass or melody port, chords are symbols like am7"
                    } else {
                        "a chord is a symbol like am7, fmaj7, g7, or a Roman numeral like V7"
                    };
                    return Err((t.span, format!("`{w}` is not a chord: {why}")));
                }
            }
            other => return Err((t.span, format!("{} is not part of a chord progression", other.describe()))),
        }
        i += 1;
    }
    Ok(())
}

fn check_key(toks: &[Token]) -> Result<(), (Span, String)> {
    if toks.iter().any(|t| matches!(t.tok, Tok::Dollar(_))) {
        return Ok(());
    }
    let span = toks[0].span.to(toks[toks.len() - 1].span);
    let text = match toks.iter().map(|t| &t.tok).collect::<Vec<_>>().as_slice() {
        [Tok::Word(k)] => k.clone(),
        [Tok::Word(k), Tok::Colon, Tok::Word(m)] => format!("{k}:{m}"),
        _ => return Err((span, "a key is one word like am, or `d:dorian`".to_string())),
    };
    if Key::parse(&text).is_none() {
        return Err((span, format!("`{text}` is not a key: a key is like am, c, f#m or d:dorian")));
    }
    Ok(())
}

/// A number of bars or a tempo: one number in range, or (once arithmetic is read) an expression of numbers.
fn check_number(toks: &[Token], lo: f64, hi: f64, whole: bool, what: &str) -> Result<(), (Span, String)> {
    let span = toks[0].span.to(toks[toks.len() - 1].span);
    if toks.iter().any(|t| matches!(t.tok, Tok::Dollar(_))) {
        return Ok(());
    }
    if let [Token { tok: Tok::Num { value, int }, .. }] = toks {
        if *value < lo || *value > hi || (whole && !*int) {
            return Err((span, format!("{what} is {}{lo} to {hi}", if whole { "a whole number, " } else { "" })));
        }
        return Ok(());
    }
    // (F13 meets F11) a port carries what is known when the program is read; an input is read while it plays
    if let Some(w) = toks.windows(2).find_map(|w| match (&w[0].tok, &w[1].tok) {
        (Tok::Word(name), Tok::LParen) if crate::perform::is_input_word(name) => Some(name.clone()),
        _ => None,
    }) {
        return Err((span, format!("{what} is one number, fixed when the program is read: `{w}(...)` is read while the program plays, so a port cannot carry it")));
    }
    let arithmetic = toks.iter().all(|t| matches!(&t.tok, Tok::Num { .. } | Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent | Tok::LParen | Tok::RParen | Tok::Comma) || matches!(&t.tok, Tok::Word(w) if w == "min" || w == "max"));
    if arithmetic && toks.iter().any(|t| matches!(t.tok, Tok::Num { .. })) {
        return Ok(());
    }
    Err((span, format!("{what} is one number")))
}

/// Calls that keep a pattern rhythmic (they move or thin hits and make no pitch).
const RHYTHM_CALLS: [&str; 16] = ["e", "euclid", "reverse", "rotate", "every", "degrade", "slow", "fast", "humanize", "maybe", "poly", "ply", "palindrome", "iter", "off", "rests"];

fn rhythmic(seq: &Seq, kind: &str) -> Result<(), (Span, String)> {
    for t in &seq.terms {
        match &t.atom {
            Atom::Hit | Atom::Rest => {}
            Atom::Note(w) => return Err((w.span, format!("`{}` is a pitch: {kind} is hits and rests (x ~ [x x] e(5,8))", w.text))),
            Atom::Degree(_) | Atom::Deg(_) | Atom::ScaleDeg(_) => return Err((t.span, format!("a scale degree is a pitch: {kind} is hits and rests (x ~ [x x] e(5,8))"))),
            Atom::Group(seqs) => {
                for s in seqs {
                    rhythmic(s, kind)?;
                }
            }
            Atom::Alt(terms) => rhythmic(&Seq { span: seq.span, terms: terms.clone() }, kind)?,
            // chance thins hits (`x*8?0.7`) and `choose` picks one of its options: both stay rhythmic if what they hold is
            Atom::Chance(c) => rhythmic(&Seq { span: seq.span, terms: vec![c.term.clone()] }, kind)?,
            Atom::Choose(c) => rhythmic(&Seq { span: seq.span, terms: c.options.clone() }, kind)?,
            Atom::Sized(s) => rhythmic(&Seq { span: seq.span, terms: vec![s.term.clone()] }, kind)?,
            Atom::Call(c) => {
                if !RHYTHM_CALLS.contains(&c.name.text.as_str()) {
                    return Err((c.name.span, format!("`{}(...)` makes pitches: {kind} is hits and rests (x ~ [x x] e(5,8))", c.name.text)));
                }
                for a in &c.args {
                    rhythmic_arg(a, kind)?;
                }
            }
        }
    }
    Ok(())
}

fn rhythmic_arg(a: &Arg, kind: &str) -> Result<(), (Span, String)> {
    match a {
        Arg::Pat(s) => rhythmic(s, kind),
        Arg::Call(c) => {
            if !RHYTHM_CALLS.contains(&c.name.text.as_str()) {
                return Err((c.name.span, format!("`{}(...)` makes pitches: {kind} is hits and rests (x ~ [x x] e(5,8))", c.name.text)));
            }
            c.args.iter().try_for_each(|a| rhythmic_arg(a, kind))
        }
        _ => Ok(()),
    }
}

/// Is `toks` one term of a pattern: a single token, a `[...]` or `<...>` group, or one call `f(...)`?
fn one_term(toks: &[Token]) -> bool {
    let close = |open: usize, up: &Tok, down: &Tok| -> bool {
        let mut depth = 0;
        for (i, t) in toks.iter().enumerate().skip(open) {
            if &t.tok == up {
                depth += 1;
            } else if &t.tok == down {
                depth -= 1;
                if depth == 0 {
                    return i == toks.len() - 1;
                }
            }
        }
        false
    };
    match toks {
        [] => false,
        [_] => true,
        [Token { tok: Tok::LBrack, .. }, ..] => close(0, &Tok::LBrack, &Tok::RBrack),
        [Token { tok: Tok::Lt, .. }, ..] => close(0, &Tok::Lt, &Tok::Gt),
        [Token { tok: Tok::Word(_), .. }, Token { tok: Tok::LParen, .. }, ..] => close(1, &Tok::LParen, &Tok::RParen),
        _ => false,
    }
}

/// The tokens that stand where `$name` is: a pattern or a chord progression of more than one term goes in as one
/// group, as a `let` pattern does (so `bass $in`, `transpose($in, 12)` and `<$in c2>` all mean what they say); a
/// key, a number, or a single term goes in as it is.
pub(crate) fn as_value(ty: PortType, v: &[Token]) -> Vec<Token> {
    if (ty.is_pattern() || ty == PortType::Chords) && !one_term(v) {
        let at = v.first().map_or(Span::default(), |t| t.span);
        let mut out = Vec::with_capacity(v.len() + 2);
        out.push(Token { tok: Tok::LBrack, span: at });
        out.extend(v.iter().cloned());
        out.push(Token { tok: Tok::RBrack, span: at });
        return out;
    }
    v.to_vec()
}

/// A chords value that is more than a lone chord word, with each bare chord symbol written as the call that plays
/// it: `<am7 g7>` is `<chord(am7) chord(g7)>`, so `harmony $chords` plays the progression (a bare symbol is no
/// step of a part). A lone word (`am7`) stays as it is, a chord name that `arp($in, up, 8)` and `chord($in)` take;
/// Roman numerals and what is inside a call are left alone.
pub(crate) fn chord_calls(toks: &[Token]) -> Vec<Token> {
    if matches!(toks, [Token { tok: Tok::Word(_), .. }]) {
        return toks.to_vec();
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut depth = 0i32;
    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::LParen => depth += 1,
            Tok::RParen => depth -= 1,
            Tok::Word(w) if depth == 0 && toks.get(i + 1).map(|n| &n.tok) != Some(&Tok::LParen) && Chord::parse(w).is_some() => {
                out.push(Token { tok: Tok::Word("chord".to_string()), span: t.span });
                out.push(Token { tok: Tok::LParen, span: t.span });
                out.push(t.clone());
                out.push(Token { tok: Tok::RParen, span: t.span });
                continue;
            }
            _ => {}
        }
        out.push(t.clone());
    }
    out
}

/// `toks` with each `$name` standing for something that fits a pattern: a hit (or a count after a `*`).
fn fill_holes(toks: &[Token]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    for t in toks {
        if matches!(t.tok, Tok::Dollar(_)) {
            let tok = if matches!(out.last().map(|p| &p.tok), Some(Tok::Star)) { Tok::Num { value: 1.0, int: true } } else { Tok::Word("x".to_string()) };
            out.push(Token { tok, span: t.span });
        } else {
            out.push(t.clone());
        }
    }
    out
}

impl Lower<'_> {
    /// Does `toks` fit a port of type `ty`? `$names` (holes) fit anything: the value they stand for is checked
    /// when it is made.
    pub(crate) fn fits(&self, ty: PortType, toks: &[Token]) -> Result<(), Diagnostic> {
        if toks.is_empty() {
            return Err(Diagnostic::error(Span::default(), "nothing to emit"));
        }
        let bad = |(span, message): (Span, String)| Diagnostic::error(span, message);
        match ty {
            PortType::Chords => check_chords(toks).map_err(bad),
            PortType::Key => check_key(toks).map_err(bad),
            PortType::Bars => check_number(toks, 1.0, 256.0, true, "a number of bars").map_err(bad),
            PortType::Tempo => check_number(toks, 40.0, 300.0, false, "a tempo").map_err(bad),
            PortType::Bass | PortType::Melody | PortType::Pattern => parse_seq_tokens(fill_holes(toks), self.src).map(|_| ()),
            PortType::Drums | PortType::Rhythm => {
                let seq = parse_seq_tokens(fill_holes(toks), self.src)?;
                let seq = self.put_lets(&seq);
                rhythmic(&seq, if ty == PortType::Drums { "a drums pattern" } else { "a rhythm" }).map_err(bad)
            }
        }
    }
}

// ---- state while lowering

/// One wire, resolved: the def and canonical out port it comes from.
#[derive(Clone, Debug)]
pub(crate) struct Feed {
    pub(crate) from_def: String,
    pub(crate) from_port: String,
    /// The output as the wire's text wrote it (`changes.out`), for what is said about it.
    pub(crate) from_text: String,
    pub(crate) span: Span,
}

#[derive(Default)]
pub(crate) struct PortState {
    /// (consumer def, its canonical in-port name) -> the output that feeds it.
    pub(crate) fed: BTreeMap<(String, String), Feed>,
    /// The values made so far: (def, canonical out-port name) -> its tokens (None: it could not be made).
    pub(crate) values: BTreeMap<(String, String), Option<Vec<Token>>>,
    /// Defs whose declaration, emits or wiring are wrong: they are not run (the error is already said).
    pub(crate) broken: BTreeSet<String>,
    /// Defs with ports that a line called.
    pub(crate) called: BTreeSet<String>,
    /// The wires written in this program's own text: (consumer def, span).
    pub(crate) own: Vec<(String, Span)>,
    /// Errors said once: (def, what).
    pub(crate) said: BTreeSet<(String, String)>,
}

/// The loops in a wiring, given as (from, to) pairs of def names: each loop once, as the defs in order (`a`, `b`
/// means a feeds b feeds a). Deterministic: names are walked in order.
pub fn wiring_cycles(edges: &[(String, String)]) -> Vec<Vec<String>> {
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (a, b) in edges {
        adj.entry(a.as_str()).or_default().insert(b.as_str());
        adj.entry(b.as_str()).or_default();
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Fresh,
        Open,
        Done,
    }
    let mut mark: BTreeMap<&str, Mark> = adj.keys().map(|k| (*k, Mark::Fresh)).collect();
    let mut found: Vec<Vec<String>> = Vec::new();
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let roots: Vec<&str> = adj.keys().copied().collect();
    for root in roots {
        if mark[root] != Mark::Fresh {
            continue;
        }
        // an explicit stack: the path, and for each node on it the successors still to try
        let mut path: Vec<&str> = vec![root];
        let mut todo: Vec<Vec<&str>> = vec![adj[root].iter().rev().copied().collect()];
        mark.insert(root, Mark::Open);
        while let Some(frontier) = todo.last_mut() {
            match frontier.pop() {
                Some(next) => match mark[next] {
                    Mark::Fresh => {
                        mark.insert(next, Mark::Open);
                        path.push(next);
                        todo.push(adj[next].iter().rev().copied().collect());
                    }
                    Mark::Open => {
                        let at = path.iter().position(|n| *n == next).unwrap_or(0);
                        let cyc: Vec<String> = path[at..].iter().map(|s| s.to_string()).collect();
                        let mut key = cyc.clone();
                        key.sort();
                        if seen.insert(key) {
                            found.push(cyc);
                        }
                    }
                    Mark::Done => {}
                },
                None => {
                    todo.pop();
                    if let Some(done) = path.pop() {
                        mark.insert(done, Mark::Done);
                    }
                }
            }
        }
    }
    found
}

// ---- lowering: checks

type End = (String, String, PortType);

impl Lower<'_> {
    /// Check every def's declaration and emits, every wire, and the wiring as a whole; build what a call needs.
    pub(crate) fn ports_setup(&mut self, parsed: &crate::parse::Parsed) {
        let names: Vec<String> = self.defs.keys().cloned().collect();
        for name in &names {
            let def = self.defs[name].def.clone();
            if !def.ports.is_empty() {
                self.check_port_def(&def);
            }
        }
        // the wires: an import's, then the program's own
        let mut wires: Vec<(WireStmt, Span, bool)> = Vec::new();
        for imp in &parsed.imports {
            for w in &imp.wires {
                wires.push((w.clone(), imp.span, true));
            }
        }
        for st in &parsed.program.stmts {
            if let StmtKind::Wire(w) = &st.kind {
                wires.push((w.clone(), st.span, false));
            }
        }
        let mut edges: Vec<(String, String)> = Vec::new();
        let mut edge_span: BTreeMap<(String, String), Span> = BTreeMap::new();
        for (w, span, imported) in &wires {
            let shown = |r: &PortRef, lw: &Self| if *imported { format!("{}.{}", r.def.text, r.port.text) } else { r.def.span.to(r.port.span).text(lw.src).to_string() };
            let (from_text, to_text) = (shown(&w.from, self), shown(&w.to, self));
            let Some((from, to)) = self.resolve_wire(w, &from_text, &to_text) else { continue };
            let key = (to.0.clone(), to.1.clone());
            if let Some(prev) = self.ports.fed.get(&key) {
                let at = line_col(self.src, prev.span.start).0;
                let d = Diagnostic::error(w.pair_span(), format!("`{to_text}` is wired twice: from `{}` (line {at}) and from `{from_text}`", prev.from_text))
                    .help("an input takes one wire: take one of them out");
                self.err(d);
                continue;
            }
            self.ports.fed.insert(key, Feed { from_def: from.0.clone(), from_port: from.1.clone(), from_text, span: *span });
            edges.push((from.0.clone(), to.0.clone()));
            edge_span.entry((from.0.clone(), to.0.clone())).or_insert(*span);
            if !imported {
                self.ports.own.push((to.0.clone(), *span));
            }
        }
        for cyc in wiring_cycles(&edges) {
            let last = cyc.last().cloned().unwrap_or_default();
            let first = cyc.first().cloned().unwrap_or_default();
            let at = edge_span.get(&(last, first)).copied().unwrap_or_default();
            let mut shown = cyc.clone();
            shown.push(cyc[0].clone());
            self.err(Diagnostic::error(at, format!("the wires make a loop: {}", shown.join(" -> "))).help("a value cannot feed itself, through any number of defs: take one of these wires out"));
            for n in cyc {
                self.ports.broken.insert(n);
            }
        }
    }

    /// The defs and canonical ports of a wire, or an error where it is wrong: unknown def or port, wrong
    /// direction, a type error.
    fn resolve_wire(&mut self, w: &WireStmt, from_text: &str, to_text: &str) -> Option<(End, End)> {
        let from = self.resolve_end(&w.from, Dir::Out)?;
        let to = self.resolve_end(&w.to, Dir::In)?;
        if !to.2.accepts(from.2) {
            let mut d = Diagnostic::error(w.pair_span(), format!("type error: `{from_text}` is {}, `{to_text}` wants {}", from.2.is_a(), to.2.wants()));
            d = d.help(to.2.carries());
            self.err(d);
            return None;
        }
        Some((from, to))
    }

    fn resolve_end(&mut self, r: &PortRef, d: Dir) -> Option<End> {
        let Some(info) = self.defs.get(&r.def.text) else {
            if self.declined.contains(&r.def.text) {
                return None;
            }
            let with_ports: Vec<String> = self.defs.iter().filter(|(_, i)| !i.def.ports.is_empty()).map(|(n, _)| n.clone()).collect();
            let all: Vec<String> = self.defs.keys().cloned().collect();
            let mut diag = Diagnostic::error(r.def.span, format!("`{}` is not a def", r.def.text)).found(format!("`{}`", r.def.text));
            let mut near = crate::env::suggest(&r.def.text, &with_ports, 1);
            if near.is_empty() {
                near = crate::env::suggest(&r.def.text, &all, 1);
            }
            if let Some(n) = near.first() {
                diag = diag.help(format!("did you mean `{n}`?"));
            } else {
                diag = diag.help("a wire joins two defs that declare ports: def changes() out: chords { ... }");
            }
            self.err(diag);
            return None;
        };
        let ports = info.def.ports.clone();
        if ports.is_empty() {
            self.err(Diagnostic::error(r.def.span, format!("`{}` has no ports", r.def.text)).help(format!("declare them in its header: def {}() {}: chords {{ ... }}", r.def.text, d.word())));
            return None;
        }
        let Some(p) = ports.find(d, &r.port.text) else {
            let names = ports.names(d);
            let mut diag = Diagnostic::error(r.port.span, format!("`{}` has no {} port `{}`", r.def.text, if d == Dir::In { "input" } else { "output" }, r.port.text)).found(format!("`{}`", r.port.text));
            if names.is_empty() {
                diag = diag.help(format!("`{}` declares no `{}:` ports; a wire goes from an output (`out:`) to an input (`in:`)", r.def.text, d.word()));
            } else {
                diag = diag.expected(&names);
                if ports.find(d.other(), &r.port.text).is_some() {
                    diag = diag.help(format!("`{}.{}` is an {}: a wire goes from an output to an input", r.def.text, r.port.text, if d == Dir::In { "output" } else { "input" }));
                } else if ports.side(d).len() > 1 && (r.port.text == "in" || r.port.text == "out") {
                    diag = diag.help(format!("`{}` has several {} ports, so name one: {}", r.def.text, d.word(), names.iter().map(|n| format!("{}.{n}", r.def.text)).collect::<Vec<_>>().join(" or ")));
                } else if let Some(n) = nearest(&r.port.text, names.iter().copied()) {
                    diag = diag.help(format!("did you mean `{n}`?"));
                }
            }
            self.err(diag);
            return None;
        };
        Some((r.def.text.clone(), ports.name_of(d, p).to_string(), p.ty))
    }

    /// A def's declaration: names, the emits (each out port once, each of the right type), what an emit may name.
    fn check_port_def(&mut self, def: &DefDef) {
        let before = self.diags.len();
        let name = &def.name.text;
        let ports = &def.ports;
        // a parameter and a port share the `$name` space
        let mut dollar: BTreeMap<&str, PortType> = BTreeMap::new();
        for p in &ports.ins {
            for n in ports.dollar_names(p) {
                dollar.insert(n, p.ty);
            }
        }
        for p in &def.params {
            if dollar.contains_key(p.text.as_str()) {
                self.err(Diagnostic::error(p.span, format!("`{}` is both a parameter and an in port of `{name}`", p.text)).help("`$name` would mean both: rename the parameter"));
            }
        }
        let split = split_emits(&def.body);
        for sp in &split.nested {
            self.err(Diagnostic::error(*sp, "`emit` belongs directly in the def's body, not inside a block").help("an out port is made once, at the top of the def"));
        }
        if ports.outs.is_empty() {
            for e in &split.emits {
                self.err(Diagnostic::error(e.word, format!("`{name}` has no `out:` ports, so there is nothing to emit")).help(format!("def {name}(...) out: chords {{ emit ... }}")));
            }
        } else {
            let mut made: BTreeMap<PortType, Span> = BTreeMap::new();
            for e in &split.emits {
                let Ok((port, value)) = self.resolve_emit(def, e) else { continue };
                let ty = port.ty;
                if let Some(prev) = made.get(&ty) {
                    let at = line_col(self.src, prev.start).0;
                    self.err(Diagnostic::error(e.word, format!("`{name}` emits its `{}` out twice (line {at})", ty.name())).help("each out port is made once"));
                    continue;
                }
                made.insert(ty, e.word);
                self.check_emit_refs(def, &value, &dollar);
                // a plain forward of an input: the types must agree here, not only when it flows
                if let [Token { tok: Tok::Dollar(n), span }] = value.as_slice() {
                    if let Some(from) = dollar.get(n.as_str()) {
                        if !ty.accepts(*from) {
                            self.err(Diagnostic::error(*span, format!("type error: `${n}` is {}, and `{name}` emits it as its {} out ({})", from.is_a(), ty.name(), ty.wants())).help(ty.carries()));
                        }
                        continue;
                    }
                }
                if let Err(mut d) = self.fits(ty, &value) {
                    if d.span == Span::default() {
                        d.span = e.span;
                    }
                    d.help = Some(match d.help.take() {
                        Some(h) => format!("{h} (in the `{}` out of `{name}`)", ty.name()),
                        None => format!("in the `{}` out of `{name}`: {}", ty.name(), ty.carries()),
                    });
                    self.err(d);
                }
            }
            for p in &ports.outs {
                if !made.contains_key(&p.ty) {
                    self.err(Diagnostic::error(def.name.span, format!("`{name}` declares `out: {}` and never emits it", p.ty.name())).help(format!("emit {} ...", ports.name_of(Dir::Out, p))));
                }
            }
        }
        if self.diags[before..].iter().any(Diagnostic::is_error) {
            self.ports.broken.insert(name.clone());
        }
    }

    /// The out port an `emit` is for, and its value tokens (an error said once, here).
    fn resolve_emit(&mut self, def: &DefDef, e: &Emit) -> Result<(Port, Vec<Token>), ()> {
        let name = &def.name.text;
        let outs = &def.ports.outs;
        if let Some(Token { tok: Tok::Word(w), span }) = e.toks.first() {
            if let Some(p) = def.ports.find(Dir::Out, w) {
                let value = e.toks[1..].to_vec();
                if value.is_empty() {
                    self.err(Diagnostic::error(e.span, format!("`emit {w}` has no value")).help(format!("emit {w} ...")));
                    return Err(());
                }
                return Ok((p.clone(), value));
            }
            if PortType::parse(w).is_some() || w == "out" {
                let have = def.ports.names(Dir::Out);
                self.err(Diagnostic::error(*span, format!("`{name}` has no out port `{w}`")).expected(&have).found(format!("`{w}`")).help(format!("its out ports: {}", have.join(", "))));
                return Err(());
            }
        }
        if e.toks.is_empty() {
            self.err(Diagnostic::error(e.word, "`emit` has no value").help("emit <chord(am7) chord(g7)>"));
            return Err(());
        }
        if outs.len() == 1 {
            return Ok((outs[0].clone(), e.toks.clone()));
        }
        let have = def.ports.names(Dir::Out);
        self.err(Diagnostic::error(e.word, format!("`{name}` has several out ports: say which one this `emit` makes")).expected(&have).help(format!("emit {} ...", have[0])));
        Err(())
    }

    /// What `$name` an emit may use: an in port. A parameter is refused (an out value is fixed by the inputs, so
    /// it is the same wherever the def is wired), and an unknown name is said with the nearest.
    fn check_emit_refs(&mut self, def: &DefDef, value: &[Token], dollar: &BTreeMap<&str, PortType>) {
        for t in value {
            let Tok::Dollar(n) = &t.tok else { continue };
            if dollar.contains_key(n.as_str()) {
                continue;
            }
            if def.params.iter().any(|p| p.text == *n) {
                self.err(
                    Diagnostic::error(t.span, format!("an `emit` cannot use the parameter `${n}`: what a def emits is fixed by its inputs"))
                        .help("that way it is the same wherever the def is wired. Write the value out, or make it an input: in: key"),
                );
                continue;
            }
            let all: Vec<String> = dollar.keys().map(|s| s.to_string()).collect();
            let mut d = Diagnostic::error(t.span, format!("`${n}` is not an in port of `{}`", def.name.text)).found(format!("`${n}`"));
            if let Some(s) = crate::env::suggest(n, &all, 1).first() {
                d = d.help(format!("did you mean `${s}`?"));
            } else if all.is_empty() {
                d = d.help(format!("`{}` has no `in:` ports", def.name.text));
            }
            self.err(d);
        }
    }

    /// At the end of lowering: a wire in the program's own text whose consumer no line called (said only when some
    /// other def with ports was called: a library of modules and wires, called by nothing, is not a mistake).
    pub(crate) fn port_warnings(&mut self) {
        let own = std::mem::take(&mut self.ports.own);
        // a program that calls nothing with ports is a library of modules: nothing is expected to flow yet
        if self.ports.called.is_empty() {
            return;
        }
        let mut told: BTreeSet<String> = BTreeSet::new();
        for (to, span) in own {
            if !self.ports.called.contains(&to) && !self.ports.broken.contains(&to) && told.insert(to.clone()) {
                self.diags.push(Diagnostic::warning(span, format!("`{to}` is wired but never called: nothing flows into it")).help(format!("call it where it should play: {to}(...)")));
            }
        }
    }
}

// ---- lowering: a call

impl Lower<'_> {
    /// A call of `def`: the body to expand (without its `emit` statements) and, in `subs`, the wired value of each
    /// in port. `None`: the call fails (the error is said).
    pub(crate) fn port_call<'d>(&mut self, def: &'d DefDef, at: Span, subs: &mut BTreeMap<String, Vec<Token>>) -> Option<Cow<'d, [Token]>> {
        let name = def.name.text.clone();
        if def.ports.is_empty() {
            let split = split_emits(&def.body);
            if let Some(sp) = split.emits.first().map(|e| e.word).or_else(|| split.nested.first().copied()) {
                if self.ports.said.insert((name.clone(), "emit".to_string())) {
                    self.err(Diagnostic::error(sp, format!("`emit` is for a def with `out:` ports, and `{name}` declares none")).help(format!("def {name}(...) out: chords {{ emit ... }}")));
                }
                return None;
            }
            return Some(Cow::Borrowed(&def.body[..]));
        }
        if self.ports.broken.contains(&name) {
            return None;
        }
        self.ports.called.insert(name.clone());
        let mut ok = true;
        for p in &def.ports.ins {
            let port = def.ports.name_of(Dir::In, p);
            match self.in_value(&name, port, p.ty, at, 0) {
                Some(v) => {
                    let v = as_value(p.ty, &v);
                    for n in def.ports.dollar_names(p) {
                        subs.insert(n.to_string(), v.clone());
                    }
                }
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        Some(Cow::Owned(split_emits(&def.body).rest))
    }

    /// What is wired to `def`'s in port `port`: the value of the output that feeds it.
    fn in_value(&mut self, def: &str, port: &str, ty: PortType, at: Span, depth: usize) -> Option<Vec<Token>> {
        let key = (def.to_string(), port.to_string());
        let Some(feed) = self.ports.fed.get(&key).cloned() else {
            if self.ports.said.insert((def.to_string(), format!("unwired {port}"))) {
                let mut d = Diagnostic::error(at, format!("`{def}.{port}` is not wired: `{def}` needs {} and nothing feeds it", ty.wants()));
                let hints = self.wire_suggestions(def, port, ty);
                d = d.help(match hints.first() {
                    Some(h) => h.clone(),
                    None => format!("wire something.out -> {def}.{port}, from a def that declares out: {}", ty.name()),
                });
                self.err(d);
            }
            return None;
        };
        self.out_value(&feed.from_def, &feed.from_port, at, depth + 1)
    }

    /// Wires that would feed `def.port`: from a def whose output fits.
    fn wire_suggestions(&self, def: &str, port: &str, want: PortType) -> Vec<String> {
        let mut v = Vec::new();
        for (n, info) in &self.defs {
            if n == def {
                continue;
            }
            for p in &info.def.ports.outs {
                if want.accepts(p.ty) {
                    v.push(format!("wire {n}.{} -> {def}.{port}", info.def.ports.name_of(Dir::Out, p)));
                }
            }
        }
        v.truncate(3);
        v
    }

    /// The value of `def`'s out port `port`: made once, from the def's inputs and its `emit`.
    fn out_value(&mut self, def: &str, port: &str, at: Span, depth: usize) -> Option<Vec<Token>> {
        let key = (def.to_string(), port.to_string());
        if let Some(v) = self.ports.values.get(&key) {
            return v.clone();
        }
        let made = self.make_outs(def, at, depth);
        for (p, v) in made {
            self.ports.values.insert((def.to_string(), p), v);
        }
        // a def whose values could not be made is said once and not tried again
        for p in self.defs.get(def).map(|i| i.def.ports.names(Dir::Out)).unwrap_or_default() {
            self.ports.values.entry((def.to_string(), p.to_string())).or_insert(None);
        }
        self.ports.values.get(&key).cloned().flatten()
    }

    /// Every out value of `def`, by canonical port name.
    fn make_outs(&mut self, def: &str, at: Span, depth: usize) -> Vec<(String, Option<Vec<Token>>)> {
        let Some(info) = self.defs.get(def) else { return Vec::new() };
        let d = info.def.clone();
        if self.ports.broken.contains(def) {
            return Vec::new();
        }
        if depth > MAX_CHAIN {
            if self.ports.said.insert((def.to_string(), "chain".to_string())) {
                self.err(Diagnostic::error(at, format!("`{def}` is more than {MAX_CHAIN} wires away from the call that needs it")).help("a chain of wired defs is at most 64 long: join some of them into one def"));
            }
            return Vec::new();
        }
        // its inputs, from whatever feeds them
        let mut subs: BTreeMap<String, Vec<Token>> = BTreeMap::new();
        for p in &d.ports.ins {
            let port = d.ports.name_of(Dir::In, p);
            let Some(v) = self.in_value(def, port, p.ty, at, depth) else { return Vec::new() };
            let v = as_value(p.ty, &v);
            for n in d.ports.dollar_names(p) {
                subs.insert(n.to_string(), v.clone());
            }
        }
        let mut out = Vec::new();
        for e in split_emits(&d.body).emits {
            let Some((port, value)) = emit_target(&d, &e) else { continue };
            let name = d.ports.name_of(Dir::Out, &port).to_string();
            let Some(toks) = self.fill(&value, &subs, &format!("def {def}")) else {
                out.push((name, None));
                continue;
            };
            if let Err(mut diag) = self.fits(port.ty, &toks) {
                if diag.span == Span::default() {
                    diag.span = e.span;
                }
                let line = line_col(self.src, at.start).0;
                diag.help = Some(match diag.help.take() {
                    Some(h) => format!("{h} (in the `{}` out of `{def}`, used by the call on line {line})", port.ty.name()),
                    None => format!("in the `{}` out of `{def}`, used by the call on line {line}: {}", port.ty.name(), port.ty.carries()),
                });
                self.err(diag);
                out.push((name, None));
                continue;
            }
            let toks = if port.ty == PortType::Chords { chord_calls(&toks) } else { toks };
            out.push((name, Some(toks)));
        }
        out
    }
}

/// `resolve_emit` without saying anything (the declaration check already did): the port and the value tokens.
fn emit_target(def: &DefDef, e: &Emit) -> Option<(Port, Vec<Token>)> {
    if let Some(Token { tok: Tok::Word(w), .. }) = e.toks.first() {
        if let Some(p) = def.ports.find(Dir::Out, w) {
            return Some((p.clone(), e.toks[1..].to_vec()));
        }
    }
    match def.ports.outs.as_slice() {
        [only] => Some((only.clone(), e.toks.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::check;

    fn sorted(mut v: Vec<Vec<String>>) -> Vec<Vec<String>> {
        for c in &mut v {
            // a cycle as a rotation of its smallest name first
            let at = (0..c.len()).min_by_key(|i| c[*i].clone()).unwrap_or(0);
            c.rotate_left(at);
        }
        v.sort();
        v
    }

    fn e(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    fn lower_for(src: &str) -> Lower<'_> {
        Lower {
            src,
            diags: Vec::new(),
            defs: BTreeMap::new(),
            pats: BTreeMap::new(),
            dims: BTreeMap::new(),
            places: BTreeMap::new(),
            arranged: false,
            cyclic: BTreeSet::new(),
            declined: BTreeSet::new(),
            produced: 0,
            work: 0,
            over_budget: false,
            evolves: Vec::new(),
            ports: PortState::default(),
            quiet: Vec::new(),
        }
    }

    fn toks(s: &str) -> Vec<Token> {
        crate::lex::lex(s).tokens.into_iter().filter(|t| t.tok != Tok::Eof).collect()
    }

    #[test]
    fn the_lattice_of_types() {
        use PortType::*;
        for t in PortType::ALL {
            assert!(t.accepts(t), "{t:?}");
            assert_eq!(PortType::parse(t.name()), Some(t));
        }
        for t in [Bass, Melody, Drums, Rhythm, Pattern] {
            assert!(Pattern.accepts(t));
        }
        for t in [Bass, Melody, Drums] {
            assert!(t.accepts(Rhythm), "{t:?} takes a rhythm");
            assert!(!Rhythm.accepts(t), "a rhythm input does not take {t:?}");
        }
        assert!(!Chords.accepts(Drums) && !Drums.accepts(Chords));
        assert!(!Pattern.accepts(Chords) && !Pattern.accepts(Key) && !Pattern.accepts(Bars));
        assert!(!Bass.accepts(Melody) && !Drums.accepts(Bass));
        assert!(!Bars.accepts(Tempo) && !Tempo.accepts(Bars));
        assert!(!Bass.accepts(Pattern), "a pattern is not known to be a bass");
    }

    #[test]
    fn chord_words_and_numerals() {
        for w in ["am7", "fmaj7", "c", "g7", "dsus4", "f#m7b5", "I", "ii", "V7", "vii\u{b0}", "bVII", "Imaj7", "iv", "VI", "vi"] {
            assert!(is_chord_word(w), "{w}");
        }
        for w in ["x", "kick", "c3", "foo", "IVi", "VIII", "", "q"] {
            assert!(!is_chord_word(w), "{w}");
        }
    }

    #[test]
    fn a_header_prints_sorted_and_with_the_sole_port_named_in_and_out() {
        let c = check("def walker(oct) in: key, chords out: bass {\n  deck A: bass c2\n  emit bass c2\n}\n");
        let text = c.compiled.unwrap().text;
        assert!(text.starts_with("def walker(oct) in: chords, key out: bass {\n"), "{text}");
        let mut p = Ports::default();
        p.ins.push(Port { ty: PortType::Chords, span: Span::default() });
        assert_eq!(p.name_of(Dir::In, &p.ins[0]), "in");
        assert!(p.find(Dir::In, "in").is_some() && p.find(Dir::In, "chords").is_some() && p.find(Dir::In, "key").is_none());
        p.ins.push(Port { ty: PortType::Key, span: Span::default() });
        assert!(p.find(Dir::In, "in").is_none(), "with two inputs `in` names neither");
        assert_eq!(p.names(Dir::In), vec!["chords", "key"]);
    }

    #[test]
    fn loops_are_found_and_named() {
        assert!(wiring_cycles(&[e("a", "b"), e("b", "c")]).is_empty());
        assert_eq!(sorted(wiring_cycles(&[e("a", "b"), e("b", "a")])), vec![vec!["a".to_string(), "b".to_string()]]);
        assert_eq!(wiring_cycles(&[e("a", "a")]), vec![vec!["a".to_string()]]);
        assert_eq!(sorted(wiring_cycles(&[e("a", "b"), e("b", "c"), e("c", "a"), e("c", "d")])).len(), 1);
        // two separate loops
        assert_eq!(wiring_cycles(&[e("a", "b"), e("b", "a"), e("x", "y"), e("y", "x")]).len(), 2);
    }

    #[test]
    fn values_fit_their_types() {
        let l = lower_for("");
        assert!(l.fits(PortType::Chords, &toks("<chord(am7) chord(fmaj7)> V7 ~")).is_ok());
        assert!(l.fits(PortType::Chords, &toks("am7 fmaj7 I/3")).is_ok());
        assert!(l.fits(PortType::Chords, &toks("x ~ x")).is_err());
        assert!(l.fits(PortType::Chords, &toks("c3 e3")).is_err());
        assert!(l.fits(PortType::Key, &toks("am")).is_ok());
        assert!(l.fits(PortType::Key, &toks("d:dorian")).is_ok());
        assert!(l.fits(PortType::Key, &toks("x")).is_err());
        assert!(l.fits(PortType::Bars, &toks("8")).is_ok());
        assert!(l.fits(PortType::Bars, &toks("0")).is_err() && l.fits(PortType::Bars, &toks("2.5")).is_err());
        assert!(l.fits(PortType::Tempo, &toks("128")).is_ok() && l.fits(PortType::Tempo, &toks("5")).is_err());
        assert!(l.fits(PortType::Drums, &toks("x ~ [x x] e(5,8)")).is_ok());
        assert!(l.fits(PortType::Drums, &toks("c3 ~")).is_err());
        assert!(l.fits(PortType::Rhythm, &toks("markov(am, 0.5)")).is_err());
        assert!(l.fits(PortType::Bass, &toks("c2 ~ e2 [g2 b2]")).is_ok());
        assert!(l.fits(PortType::Bass, &toks("$in ~ x")).is_ok());
    }

    #[test]
    fn emits_are_cut_from_the_body() {
        let t = toks("deck A: kick x\nemit <am7 g7>\nrepeat 2 {\n  master: delay 0.1\n}\nemit bass x");
        let s = split_emits(&t);
        assert_eq!(s.emits.len(), 2);
        assert!(s.nested.is_empty());
        assert!(!s.rest.iter().any(|t| t.tok == Tok::Word("emit".into())));
        let t = toks("repeat 2 {\n  emit x\n}");
        let s = split_emits(&t);
        assert!(s.emits.is_empty());
        assert_eq!(s.nested.len(), 1);
    }
}
