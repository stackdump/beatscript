//! A hand-written recursive-descent parser. It reads every statement it can: an error in one
//! statement is recorded and the parser goes on from the next line, so one evaluation reports all
//! of them.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::dur::{Dur, Pos};
use crate::env::Env;
use crate::lex::{lex, Tok, Token};

pub const ROLES: [&str; 8] = ["kick", "snare", "hihat", "clap", "bass", "melody", "harmony", "arp"];
pub const ACTION_WORDS: [&str; 19] = [
    "swap", "move", "load", "queue", "play", "stop", "sync", "mute", "unmute", "filter", "delay", "reverb", "crush", "resonance", "level", "low", "mid", "high",
    "crossfader",
];
/// Bars in the default phrase.
const PHRASE: u32 = 8;
pub const PARAMS: [&str; 11] = ["vel", "oct", "len", "inst", "seed", "kit", "sound", "route", "glide", "decay", "pitch-drop"];

/// The sugar words: `vocoder on` is `do vocoder on`. (word, the registry noun it stands for.)
pub const SUGAR: [(&str, &str); 14] = [
    ("vocoder", "vocoder"),
    ("vocal", "vocal"),
    ("voice", "voice"),
    ("trick", "trick"),
    ("freeze", "freeze"),
    ("loops", "loops"),
    ("clips", "clips"),
    ("wheel", "wheel"),
    ("td3", "td3"),
    ("pd705", "kits"),
    ("effects", "effects"),
    ("aux", "aux"),
    ("automix", "automix"),
    ("record", "record"),
];
/// Words the extensions (docs/BTW-EXTENSIONS.md) give a meaning to. Most are syntax in ONE position only (`pad(` before a
/// parenthesis, `step` after `over 4 bars`, `in:` in a def header), so they are not reserved: they name a let, a def or a
/// section as they did before the extensions, and the places where the language reads them as syntax are told apart by
/// position (`reserved`, `prepass`; docs/BTW.md "Decisions"). A feature that adds a word adds it here, and to
/// [`EXT_RESERVED`] only if it starts a construct that a name could not be told from.
pub const EXT_WORDS: [&str; 65] = [
    "stack", "then", "silence", "wire", "assert", "sing", "duck", "under", "mix", "choose", "maybe", "weights", "deg", "lfo", "sine", "tri", "square", "saw", "exp", "log", "step", "beat", "beats", "in", "out", "ends", "period",
    "length", "knob", "pad", "fader", "held", "min", "max", "and", "or", "not", "voice-lead", "voice", "genre", "tags", "replay", "test", "eq", "attack", "release", "center", "depth", "phase", "tempo", "s-curve", "linear", "emit", "chords", "drums", "rhythm", "pattern", "set", "until", "drive",
    "never", "beside", "during", "station", "forms",
];
/// The words of the extensions that stay reserved: each starts a construct (`stack(`, `choose [`, `lfo sine`, `wire a.out`,
/// `assert ...`, `sing "..."`, `emit ...`, an operator of a guard) that a name in its place could not be told from.
pub const EXT_RESERVED: [&str; 17] = ["stack", "then", "silence", "wire", "assert", "sing", "under", "choose", "maybe", "deg", "lfo", "and", "or", "not", "voice-lead", "s-curve", "emit"];
/// Every word that can start a statement: what a typo at the start of a line is compared with.
pub const STATEMENT_WORDS: [&str; 28] = [
    "tempo", "swing", "key", "let", "def", "repeat", "section", "arrange", "end", "loop", "sample", "use", "set", "wire", "assert", "evolve", "import", "deck", "master", "every", "on", "at", "after", "before", "do", "launch",
    "control", "station",
];
pub const KEY_VERBS: [&str; 6] = ["status", "detect", "set", "lock", "tempo", "compat"];
/// What the control deck takes, after `control:` (docs/btw/ext-control.md): the engine calls that take no target.
pub const CONTROL_DECK_WORDS: [&str; 10] = ["move", "swap pairs", "load pair", "queue", "crossfader", "tempo", "duck", "mix", "do", "a sugar noun (freeze, vocoder, wheel ...)"];

/// Is a `control` with `next` after it and `prev` before it the control deck's head? Before a colon, where a line's head goes:
/// at the start of a line, in a block, or after a time word's colon. (`after control:` names a section.)
pub fn control_head_at(next: Option<&Tok>, prev: Option<&Tok>) -> bool {
    next == Some(&Tok::Colon) && matches!(prev, None | Some(Tok::Newline | Tok::Colon | Tok::LBrace))
}

/// A second head on a line (`deck A: control: ...`, `control: master: ...`): a line has one.
fn two_heads(at: Span, first: &str) -> Diagnostic {
    Diagnostic::error(at, format!("a line has one head, and this one has `{first}:` already"))
        .help("the control deck is a deck of its own, beside the others: `control: move echo_out on B`, `deck A: kick x ~ x ~`, `master: filter lp 2k`")
}

pub fn sugar_noun(word: &str) -> Option<&'static str> {
    SUGAR.iter().find(|(w, _)| *w == word).map(|(_, n)| *n).or((word == "mashup").then_some("mashup")).or((word == "key").then_some("key"))
}

/// What a parse gives: the program (every statement that read) and every diagnostic.
pub struct Parsed {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
    pub comments: Vec<Span>,
    /// What each `import` brought: its pinned CID and the lets and defs of that snippet (and of
    /// the snippets it imports), their tokens' spans all on the import line.
    pub imports: Vec<Imported>,
    /// The `let`s of this text that stand for a number, put in place of every use before the text was read: an error
    /// at a use says which `let` it was ([`name_lets`]).
    pub scalar_lets: Vec<ScalarLet>,
    /// The programs a `set` plays, in order, each pinned to its CID (set.rs).
    pub plays: Vec<Played>,
}

/// A `let` of one number (or arithmetic that came to one), as the reader put it in place.
#[derive(Clone, Debug)]
pub struct ScalarLet {
    pub name: String,
    /// The name in the `let` itself.
    pub at: Span,
    /// The number it came to, and the text it was written as (`1 -1`).
    pub value: f64,
    pub text: String,
}

/// A track of a set: its CID and its canonical text, as the store gave them.
#[derive(Clone, Debug)]
pub struct Played {
    pub cid: String,
    pub text: String,
    pub span: Span,
}

/// Errors at a use of a scalar `let` (`bass pat` where `let pat = 1 -1` is the number 0) say which `let` that was: the
/// use is the number's token, and without this the message is about a number the person never typed there.
pub fn name_lets(diags: &mut [Diagnostic], lets: &[ScalarLet], src: &str) {
    if lets.is_empty() {
        return;
    }
    for d in diags.iter_mut().filter(|d| d.is_error() && !d.message.contains("is the let on line") && !d.help.as_deref().is_some_and(|h| h.contains("is the `let` on line") || h.contains("is the let on line"))) {
        let used = d.span.text(src);
        let Some(l) = lets.iter().rev().find(|l| l.name == used && l.at != d.span) else { continue };
        let line = crate::diag::line_col(src, l.at.start).0;
        let note = format!("`{}` is the `let` on line {line}: `let {} = {}` is the number {}", l.name, l.name, l.text, crate::print::num(l.value));
        d.help = Some(match d.help.take() {
            Some(h) => format!("{h} ({note})"),
            None => note,
        });
    }
}

#[derive(Clone, Debug)]
pub struct Imported {
    pub span: Span,
    pub cid: String,
    pub lets: Vec<LetDef>,
    pub defs: Vec<DefDef>,
    /// The samples and loops that snippet names (and its imports'): a program that imports it uses them too.
    pub samples: Vec<SampleDecl>,
    /// The `wire` statements of that snippet (and of the snippets it imports): a saved composition keeps its wiring.
    pub wires: Vec<crate::ports::WireStmt>,
    /// The programs this import draws on, for a link to carry (share.rs): the snippets it imports, each with what it imports in
    /// turn, then the snippet itself, as (CID, canonical text), each once.
    pub closure: Vec<(String, String)>,
}

/// The programs some imports bring, in the order a store should take them (what a program needs before the program that needs
/// it), each once: the closure a link carries.
pub fn closure_of(imports: &[Imported]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for imp in imports {
        for (cid, text) in &imp.closure {
            if !out.iter().any(|(c, _)| c == cid) {
                out.push((cid.clone(), text.clone()));
            }
        }
    }
    out
}

/// Why a name cannot be given to a `let`, a `def` or a section. The words of the extensions are reserved only where
/// they start a construct ([`EXT_RESERVED`]); a Roman numeral names a let, a def or a section too (a `let` of that name
/// wins over the numeral in a pattern), unless its canonical spelling is another word (`idim` is `i°`).
pub fn reserved(name: &str) -> Option<String> {
    let words: [&[&str]; 5] = [&ROLES, &ACTION_WORDS, &PARAMS, &EXT_RESERVED, &["x", "deck", "master", "A", "B", "do", "let", "def", "repeat", "section", "arrange", "import", "end", "loop", "sample", "use", "restricted", "evolve", "gen", "gens", "stop", "when", "every", "on", "at", "after", "before", "for", "over", "bars", "bar", "tempo", "swing", "key", "phrase", "next", "from", "pair", "pairs", "with", "group", "mashup"]];
    if words.iter().any(|w| w.contains(&name)) || sugar_noun(name).is_some() {
        return Some(format!("`{name}` is a word of the language"));
    }
    if crate::music::parse_note(name).is_some() || crate::music::Chord::parse(name).is_some() || crate::music::Key::parse(name).is_some() {
        return Some(format!("`{name}` reads as a note, chord or key"));
    }
    if crate::pattern::GENERATORS.contains(&name) || crate::pattern::TRANSFORMS.contains(&name) {
        return Some(format!("`{name}` is a generator or transform"));
    }
    if crate::stored::is_cid(name) {
        return Some(format!("`{name}` reads as a stored program's CID"));
    }
    if crate::scale::is_roman(name) {
        let canon = crate::scale::canonical_word(name);
        if canon != name {
            return Some(format!("`{name}` reads as the Roman numeral `{canon}`"));
        }
    }
    None
}

/// [`reserved`] for the kind of thing being named (`"def"`, `"let"`, `"section"`, `"sound"`): a def may be called `stack` or `then`
/// (they start a construct only after a `deck A:`; at the start of a line `stack(A)` calls the def).
pub fn reserved_as(name: &str, what: &str) -> Option<String> {
    if what == "def" && matches!(name, "stack" | "then") {
        return None;
    }
    reserved(name)
}

/// What to say after a refused name: a name that is free, close to the one that was written. (`a` and `i` are a note and a
/// numeral, `pad` is a word of the language: `a1` is a note too, so the choice is checked.)
pub fn rename_hint(name: &str) -> String {
    let own = |c: &String| reserved(c).is_none();
    let cand = [format!("{name}1"), format!("my_{name}"), format!("{name}_x"), format!("the_{name}")].into_iter().find(own).unwrap_or_else(|| format!("my_{name}"));
    format!("rename it, for example `{cand}`")
}

impl Parsed {
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

pub fn parse(src: &str) -> Parsed {
    parse_with(src, &Env::none())
}

/// Parse with an environment: `import` reads its snippets from it.
pub fn parse_with(src: &str, env: &Env) -> Parsed {
    // (F13: `knob(N)` and `pad(N)` are checked against the connected controllers' size)
    crate::perform::with_bank(env.bank, || parse_depth(src, env, 0))
}

const MAX_IMPORT_DEPTH: usize = 4;

/// The tokens the statement parser reads for `src` (after the prepass: scalar lets put in place), for the grammar's
/// recognizer (grammar/recognize.rs): the grammar describes these, not the raw text.
pub(crate) fn grammar_tokens(src: &str) -> Vec<Token> {
    let lexed = lex(src);
    let mut diags = lexed.errors;
    let (mut imports, mut lets, mut ratios) = (Vec::new(), Vec::new(), BTreeSet::new());
    prepass(crate::algebra::collapse_lets(lexed.tokens), src, &Env::none(), 0, &mut diags, &mut imports, &mut lets, &mut ratios).toks
}

fn parse_depth(src: &str, env: &Env, depth: usize) -> Parsed {
    let lexed = lex(src);
    let mut diags = lexed.errors;
    let mut imports = Vec::new();
    let mut scalar_lets = Vec::new();
    let mut ratios = BTreeSet::new();
    // a generator call is one word to the parser: its canonical spelling, its version written (gen.rs resolves it)
    let (toks, gen_diags) = crate::gen::collapse(lexed.tokens);
    diags.extend(gen_diags);
    // a seed reference is one word to the parser: its canonical spelling (seed.rs writes it out before lowering)
    let (toks, seed_diags) = crate::seed::collapse(toks);
    diags.extend(seed_diags);
    // and so is a template reference (`arrange from template(deep-house, @tempest_such_stuff)`, template.rs)
    let (toks, template_diags) = crate::template::collapse(toks);
    diags.extend(template_diags);
    // the same for a stored program's: its CID and `.section.role`, one word (stored.rs)
    let toks = crate::stored::collapse(toks);
    let pre = prepass(crate::algebra::collapse_lets(toks), src, env, depth, &mut diags, &mut imports, &mut scalar_lets, &mut ratios);
    let mut p = Parser { toks: pre.toks, at: 0, src, diags, depth: 0, ratios };
    let mut stmts = p.statements(false);
    // pin each import to its CID: the canonical text says which snippet, not what it was called
    let mut it = imports.iter();
    for st in &mut stmts {
        if let StmtKind::Import(i) = &mut st.kind {
            if let Some(imp) = it.next() {
                i.what.text = imp.cid.clone();
            }
        }
    }
    crate::ports::canonicalize(&mut stmts, &imports);
    let mut diagnostics = p.diags;
    // pin each `play` of a set to its CID, the same way
    let mut plays = Vec::new();
    for st in &mut stmts {
        let StmtKind::Set(block) = &mut st.kind else { continue };
        for it in &mut block.items {
            let SetItem::Play(pl) = it else { continue };
            // (a generator call is a track the set's check resolves: the call is its own pin)
            if crate::gen::is_call_word(&pl.what.text) {
                continue;
            }
            match &env.snippets {
                None => diagnostics.push(Diagnostic::error(pl.what.span, "there are no saved programs here to play").help("a set plays programs from the app's store (`btw save` first)")),
                Some(snips) => match snips.resolve(&pl.what.text) {
                    Ok((cid, text)) => {
                        pl.what.text = cid.clone();
                        plays.push(Played { cid, text, span: pl.span });
                    }
                    Err(e) => diagnostics.push(Diagnostic::error(pl.what.span, e).help("`btw list` shows the saved programs; `play` takes a name, a CID or the end of one")),
                },
            }
        }
    }
    for d in &mut diagnostics {
        explain_lets(d, src, &pre.subs, &pre.scalars);
    }
    diagnostics.sort_by_key(|d| d.span.start);
    name_lets(&mut diagnostics, &scalar_lets, src);
    Parsed { program: Program { stmts }, diagnostics, comments: lexed.comments, imports, scalar_lets, plays }
}

/// A scalar `let` put in place of one use of its name: what an error at that place may say about it.
#[derive(Clone, Debug)]
struct Sub {
    span: Span,
    name: String,
    value: String,
    /// The let's own name (its line).
    at: Span,
}

struct Prepassed {
    toks: Vec<Token>,
    subs: Vec<Sub>,
    /// Every scalar let's name.
    scalars: BTreeSet<String>,
}

/// An error that stands on a word a scalar `let` was put in place of says so (the let is read as a number wherever its name
/// is, so a name that is also a let gets a number where a name was meant), and an error at a word with a `-` joined to a
/// let's name (`n-1`) says to put spaces round the minus.
fn explain_lets(d: &mut Diagnostic, src: &str, subs: &[Sub], scalars: &BTreeSet<String>) {
    if !d.is_error() {
        return;
    }
    let minus = |w: &str| -> Option<String> {
        let parts: Vec<&str> = w.split('-').collect();
        let named = parts.iter().filter(|p| scalars.contains(**p)).count();
        (parts.len() > 1 && named > 0 && parts.iter().all(|p| scalars.contains(*p) || (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))).then(|| format!("a `-` joined to a name is part of the name: write `{}` with spaces round the minus", parts.join(" - ")))
    };
    let add = |extra: String, d: &mut Diagnostic| {
        d.help = Some(match d.help.take() {
            Some(h) => format!("{h}; {extra}"),
            None => extra,
        });
    };
    if let Some(sub) = subs.iter().find(|s| s.span == d.span) {
        let line = crate::diag::line_col(src, sub.at.start).0;
        if let Some(m) = minus(sub.value.trim_matches('`')) {
            add(m, d);
        } else if d.expected.iter().any(|e| e.contains("name")) || d.message.contains("name") {
            add(format!("`{}` is the let on line {line} (= {}), put in place as a value: a name that is also a let reads as the value; rename the let or the name", sub.name, sub.value), d);
        }
    } else if let Some(m) = minus(d.span.text(src)) {
        add(m, d);
    }
}

fn describe_value(t: &Tok) -> String {
    match t {
        Tok::Num { value, .. } => crate::print::num(*value),
        Tok::Word(w) => format!("`{w}`"),
        Tok::Str(s) => format!("\"{s}\""),
        other => other.describe(),
    }
}

/// A `let` value of a number and a unit (`5ms`, `6db`): also put in place of the name, both tokens.
pub(crate) fn is_unit_value(v: &[Token]) -> bool {
    matches!(v, [Token { tok: Tok::Num { .. }, span: n }, Token { tok: Tok::Unit(_), span: u }] if n.end == u.start)
}

/// Does the line that `out` ends in (the tokens read so far) begin with this word?
fn line_starts_with(out: &[Token], word: &str) -> bool {
    let start = out.iter().rposition(|t| matches!(t.tok, Tok::Newline | Tok::LBrace)).map_or(0, |k| k + 1);
    matches!(out.get(start), Some(Token { tok: Tok::Word(w), .. }) if w == word)
}

/// Is the line that `out` ends in an `assert never ...`?
fn line_is_assert_never(out: &[Token]) -> bool {
    let start = out.iter().rposition(|t| matches!(t.tok, Tok::Newline | Tok::LBrace)).map_or(0, |k| k + 1);
    matches!((out.get(start).map(|t| &t.tok), out.get(start + 1).map(|t| &t.tok)), (Some(Tok::Word(a)), Some(Tok::Word(n))) if a == "assert" && n == "never")
}

/// Is `toks[i]`, a word that names a scalar `let`, the let's name here, or a word of the language that is read where it
/// stands? A few words of the extensions are syntax in one position only; there the word is what the language reads, and
/// the let is not put in place (docs/BTW.md "Decisions", reserved by position): a function before `(` (`min(`, `pad(`), a
/// wave after `lfo`, a shape after a duration (`over 4 bars step 4`), a `beat` or `beats` after a number, the first word of
/// an `assert` (`ends`, `never`), a constraint's `beside`, `during` and `ends on`, `in` in an `assert`, a port in a def header or
/// after `emit` or a dot (`a.out`), a mix item's `eq` or `pan`, the `weights` of a `choose`, `mix {` and `duck B`, and the words of
/// a `set` block.
pub(crate) fn is_syntax_here(w: &str, toks: &[Token], i: usize, out: &[Token], in_assert: bool) -> bool {
    let next = toks.get(i + 1).map(|t| &t.tok);
    let prev = out.last().map(|t| &t.tok);
    let prev_word = |p: &str| matches!(prev, Some(Tok::Word(x)) if x == p);
    // a duration's unit: `4 bars`, `(n + 1) beats` (`at bar step` has a `bar` that no number comes before)
    let after_duration = ["bars", "bar", "beats", "beat"].iter().any(|u| prev_word(u)) && matches!(out.len().checked_sub(2).map(|k| &out[k].tok), Some(Tok::Num { .. } | Tok::RParen));
    if prev == Some(&Tok::Dot) || next == Some(&Tok::Dot) {
        return true;
    }
    match w {
        "min" | "max" | "knob" | "pad" | "fader" | "held" => next == Some(&Tok::LParen),
        // (`rests` names a `let` as any word may, and is the call only before its `(`)
        n if crate::answer::ANSWERS.contains(&n) => next == Some(&Tok::LParen),
        "sine" | "tri" | "square" | "saw" => prev_word("lfo"),
        "exp" | "log" | "linear" | "step" => after_duration,
        "beat" | "beats" => matches!(prev, Some(Tok::Num { .. } | Tok::RParen)),
        "weights" => prev == Some(&Tok::RBrack),
        "length" | "period" | "never" => prev_word("assert"),
        // (`assert ends`, and `assert bass ends on 1`)
        "ends" => prev_word("assert") || (in_assert && matches!(next, Some(Tok::Word(n)) if n == "on")),
        // (a constraint's `beside` and `during`: `assert never A.snare beside A.snare`)
        "beside" | "during" => in_assert && line_is_assert_never(out),
        "in" => in_assert,
        "eq" | "pan" => prev_word("A") || prev_word("B") || prev_word("master"),
        "chords" | "drums" | "rhythm" | "pattern" => prev_word("emit"),
        // (a set's own `mix 8 bars with crossfade` is the other `mix`: a number follows it at the start of a line)
        "mix" => next == Some(&Tok::LBrace) || (matches!(next, Some(Tok::Num { .. })) && matches!(prev, None | Some(Tok::Newline | Tok::LBrace))),
        // `set {` at the start of a statement, and an `until` on a set's `play` line
        "set" => next == Some(&Tok::LBrace) && matches!(prev, None | Some(Tok::Newline)),
        // `station {` at the start of a statement, and its `forms` line (station.rs)
        "station" => next == Some(&Tok::LBrace) && matches!(prev, None | Some(Tok::Newline)),
        "forms" => matches!(prev, None | Some(Tok::Newline | Tok::LBrace)) && matches!(next, Some(Tok::Word(_))),
        "until" => line_starts_with(out, "play") || line_starts_with(out, "next"),
        "duck" => matches!(next, Some(Tok::Word(n)) if n == "A" || n == "B"),
        // `control:`, the control deck's head, at the head of a line (after its time word, if it has one)
        "control" => control_head_at(next, prev),
        // `master: drive 0.6`: the verb after a target's colon (a section, a let or a def may be named `drive` anywhere else)
        "drive" => {
            let target = out.len().checked_sub(2).map(|k| &out[k].tok);
            prev == Some(&Tok::Colon) && (matches!(target, Some(Tok::Word(t)) if t == "master" || t == "A" || t == "B") || matches!(target, Some(Tok::Dollar(_))))
        }
        _ => false,
    }
}

/// The two things done to the tokens before they are read as statements: `import`s are resolved
/// (their scalar lets join the text's), and a scalar `let name = one-token` is put in place of every
/// later use of `name`.
///
/// `lets` gets every scalar `let` of a number (an error at a use says which `let` it was, [`name_lets`]). `ratios` gets the
/// names whose number came of arithmetic and is a ratio no short decimal spells (`let third = 1/3`): the number put in place
/// of `third` is the f64 nearest to 1/3, and the parser, told these names ([`Parser::ratios`]), says what it is at each use
/// (`1 / 3`, as `1/3` typed there would be) instead of the rounded f64.
#[allow(clippy::too_many_arguments)]
fn prepass(toks: Vec<Token>, src: &str, env: &Env, depth: usize, diags: &mut Vec<Diagnostic>, imports: &mut Vec<Imported>, lets: &mut Vec<ScalarLet>, ratios: &mut BTreeSet<String>) -> Prepassed {
    let mut scalars: BTreeMap<String, Token> = BTreeMap::new();
    // a let of a number and a unit (`5ms`): both tokens
    let mut unit_scalars: BTreeMap<String, (Token, Token)> = BTreeMap::new();
    // where each scalar let's name stands (the line an error names)
    let mut scalar_at: BTreeMap<String, Span> = BTreeMap::new();
    let mut subs: Vec<Sub> = Vec::new();
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut at_start = true;
    let mut brace = 0i32;
    // in a def's header (names, parameters, ports): nothing is put in place; in an `assert`: `in` is its own
    let (mut in_header, mut in_assert) = (false, false);
    let (mut def_name_at, mut section_name_at): (Option<usize>, Option<usize>) = (None, None);
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        // an import line
        if at_start && brace == 0 && matches!(&t.tok, Tok::Word(w) if w == "import") {
            let arg = toks.get(i + 1);
            let query = match arg.map(|a| &a.tok) {
                Some(Tok::Word(w)) => Some(w.clone()),
                Some(Tok::Str(s)) => Some(s.clone()),
                _ => None,
            };
            if let (Some(q), Some(a)) = (query, arg) {
                let span = t.span.to(a.span);
                match resolve_import(&q, span, env, depth) {
                    Ok(imp) => {
                        for l in &imp.lets {
                            if l.value.len() == 1 {
                                scalars.insert(l.name.text.clone(), l.value[0].clone());
                                set_ratio(ratios, &l.name.text, l.ratio);
                                scalar_at.insert(l.name.text.clone(), l.name.span);
                            } else if is_unit_value(&l.value) {
                                unit_scalars.insert(l.name.text.clone(), (l.value[0].clone(), l.value[1].clone()));
                                scalar_at.insert(l.name.text.clone(), l.name.span);
                            }
                        }
                        imports.push(imp);
                    }
                    Err(d) => {
                        diags.push(d);
                        // keep the imports and the statements in step
                        imports.push(Imported { span, cid: q, lets: Vec::new(), defs: Vec::new(), samples: Vec::new(), wires: Vec::new(), closure: Vec::new() });
                    }
                }
            }
        }
        // a scalar let
        if at_start && brace == 0 && matches!(&t.tok, Tok::Word(w) if w == "let") {
            // a let of a number and a unit (`let atk = 5ms`): put in place as both tokens
            if let (Some(Token { tok: Tok::Word(name), span: nspan }), Some(Token { tok: Tok::Eq, .. }), Some(num @ Token { tok: Tok::Num { .. }, .. }), Some(unit @ Token { tok: Tok::Unit(_), .. })) = (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3), toks.get(i + 4)) {
                let ends = matches!(toks.get(i + 5).map(|t| &t.tok), None | Some(Tok::Newline | Tok::Eof));
                if ends && num.span.end == unit.span.start {
                    match reserved(name) {
                        Some(why) => diags.push(Diagnostic::error(*nspan, format!("`{name}` cannot be a name: {why}")).help(rename_hint(name))),
                        None => {
                            unit_scalars.insert(name.clone(), (num.clone(), unit.clone()));
                            scalar_at.insert(name.clone(), *nspan);
                        }
                    }
                    out.extend_from_slice(&toks[i..i + 5]);
                    i += 5;
                    at_start = false;
                    continue;
                }
            }
            // a let of arithmetic (`let n = 2 * 4`): folded to one number, put in place of every later `n`
            if let (Some(Token { tok: Tok::Word(name), span: nspan }), Some(Token { tok: Tok::Eq, .. })) = (toks.get(i + 1), toks.get(i + 2)) {
                let end = (i + 3..toks.len()).find(|&j| matches!(toks[j].tok, Tok::Newline | Tok::Eof)).unwrap_or(toks.len());
                if end > i + 4 {
                    let folded = match crate::expr::fold_let(&toks[i + 3..end], &scalars) {
                        Ok(f) => f,
                        Err(d) => {
                            // the error is said once; the name still stands for a number so that nothing else is said of it
                            let span = toks[i + 3].span.to(toks[end - 1].span);
                            diags.push(d);
                            Some(Token { tok: Tok::Num { value: 0.0, int: true }, span })
                        }
                    };
                    if let Some(v) = folded {
                        match reserved(name) {
                            Some(why) => diags.push(Diagnostic::error(*nspan, format!("`{name}` cannot be a name: {why}")).help(rename_hint(name))),
                            None => {
                                scalars.insert(name.clone(), v.clone());
                                set_ratio(ratios, name, matches!(v.tok, Tok::Num { value, .. } if crate::print::ratio_text(value).is_some()));
                                if let Tok::Num { value, .. } = v.tok {
                                    lets.push(ScalarLet { name: name.clone(), at: *nspan, value, text: toks[i + 3].span.to(toks[end - 1].span).text(src).to_string() });
                                }
                                scalar_at.insert(name.clone(), *nspan);
                            }
                        }
                        out.push(t.clone());
                        out.push(toks[i + 1].clone());
                        out.push(toks[i + 2].clone());
                        out.push(v);
                        i = end;
                        at_start = false;
                        continue;
                    }
                }
            }
            if let (Some(Token { tok: Tok::Word(name), span: nspan }), Some(Token { tok: Tok::Eq, .. }), Some(val)) = (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3)) {
                let ends = matches!(toks.get(i + 4).map(|t| &t.tok), None | Some(Tok::Newline | Tok::Eof));
                // (a seed reference is one word to the parser but bars of a pattern: a pattern let, not a value)
                let simple = matches!(&val.tok, Tok::Word(w) if !crate::seed::is_ref_word(w) && !crate::stored::is_ref_word(w)) || matches!(val.tok, Tok::Num { .. } | Tok::Str(_));
                if ends && simple {
                    let mut v = val.clone();
                    // (`let b = third` is the number `third` is, and says it as `third` does; a literal is never a ratio)
                    let mut ratio = false;
                    if let Tok::Word(w) = &val.tok {
                        if let Some(prev) = scalars.get(w) {
                            v = Token { tok: prev.tok.clone(), span: val.span };
                            ratio = ratios.contains(w);
                        }
                    }
                    match reserved(name) {
                        Some(why) => diags.push(Diagnostic::error(*nspan, format!("`{name}` cannot be a name: {why}")).help(rename_hint(name))),
                        None => {
                            scalars.insert(name.clone(), v.clone());
                            set_ratio(ratios, name, ratio);
                            if let (Tok::Num { value, .. }, Tok::Num { .. }) = (&v.tok, &val.tok) {
                                lets.push(ScalarLet { name: name.clone(), at: *nspan, value: *value, text: val.span.text(src).to_string() });
                            }
                            scalar_at.insert(name.clone(), *nspan);
                        }
                    }
                    out.push(t.clone());
                    out.push(toks[i + 1].clone());
                    out.push(toks[i + 2].clone());
                    out.push(v);
                    i += 4;
                    at_start = false;
                    continue;
                }
            }
        }
        if at_start {
            if let Tok::Word(w) = &t.tok {
                match w.as_str() {
                    "def" => {
                        in_header = true;
                        def_name_at = Some(i + 1);
                    }
                    "assert" => in_assert = true,
                    "section" => section_name_at = Some(i + 1),
                    _ => {}
                }
            }
        }
        match &t.tok {
            Tok::Newline => {
                at_start = true;
                in_assert = false;
                in_header = false;
            }
            Tok::LBrace => {
                brace += 1;
                at_start = false;
                in_header = false;
            }
            Tok::RBrace => {
                brace -= 1;
                at_start = false;
            }
            _ => at_start = false,
        }
        if let Tok::Word(w) = &t.tok {
            let known = scalars.contains_key(w) || unit_scalars.contains_key(w);
            // the name of a def or a section that is also a scalar let: said, and the name is left as it is
            if known && (def_name_at == Some(i) || section_name_at == Some(i)) {
                let what = if def_name_at == Some(i) { "def" } else { "section" };
                let (line, value) = scalar_here(w, &scalars, &unit_scalars, &scalar_at, src);
                diags.push(Diagnostic::error(t.span, format!("`{w}` is the let on line {line} (= {value}): a {what} needs a name of its own")).help(rename_hint(w)));
                out.push(t.clone());
                i += 1;
                continue;
            }
            // not a parameter's name (`name=`), not a def's header, not a word the language reads where it stands
            let named = toks.get(i + 1).map(|n| &n.tok) != Some(&Tok::Eq) && !in_header && !is_syntax_here(w, &toks, i, &out, in_assert);
            if named {
                if let Some(v) = scalars.get(w) {
                    subs.push(Sub { span: t.span, name: w.clone(), value: describe_value(&v.tok), at: scalar_at.get(w).copied().unwrap_or(t.span) });
                    out.push(Token { tok: v.tok.clone(), span: t.span });
                    i += 1;
                    continue;
                }
                if let Some((n, u)) = unit_scalars.get(w) {
                    out.push(Token { tok: n.tok.clone(), span: t.span });
                    out.push(Token { tok: u.tok.clone(), span: Span::new(t.span.end, t.span.end) });
                    i += 1;
                    continue;
                }
            }
        }
        out.push(t.clone());
        i += 1;
    }
    let names = scalars.keys().chain(unit_scalars.keys()).cloned().collect();
    Prepassed { toks: out, subs, scalars: names }
}

/// (the line of a scalar let, its value as text), for what an error says about it.
fn scalar_here(w: &str, scalars: &BTreeMap<String, Token>, units: &BTreeMap<String, (Token, Token)>, at: &BTreeMap<String, Span>, src: &str) -> (usize, String) {
    let line = at.get(w).map_or(0, |s| crate::diag::line_col(src, s.start).0);
    let value = match (scalars.get(w), units.get(w)) {
        (Some(v), _) => describe_value(&v.tok),
        (None, Some((n, u))) => format!("{}{}", describe_value(&n.tok), if let Tok::Unit(u) = &u.tok { u.as_str() } else { "" }),
        _ => String::new(),
    };
    (line, value)
}

fn set_ratio(ratios: &mut BTreeSet<String>, name: &str, ratio: bool) {
    if ratio {
        ratios.insert(name.to_string());
    } else {
        ratios.remove(name);
    }
}

fn resolve_import(query: &str, span: Span, env: &Env, depth: usize) -> Result<Imported, Diagnostic> {
    let Some(snips) = env.snippets else {
        return Err(Diagnostic::error(span, "there are no saved snippets here to import from").help("import needs the app's snippet store (or `live save` first)"));
    };
    if depth >= MAX_IMPORT_DEPTH {
        return Err(Diagnostic::error(span, format!("imports nest more than {MAX_IMPORT_DEPTH} deep")).help("flatten the snippet that imports this one"));
    }
    let (cid, text) = snips.resolve(query).map_err(|e| {
        let help = if crate::env::is_not_here(&e) { crate::env::NOT_HERE_HELP } else { "`live list` shows the saved snippets; import takes a name in \"quotes\" or a CID" };
        Diagnostic::error(span, e).help(help)
    })?;
    // the references to stored programs the snippet makes are written out first, so that what it brings is plain BeatScript; the
    // programs it borrowed from travel with it (stored.rs)
    let (expanded, borrowed) = match crate::stored::expand(&text, env) {
        None => (text.clone(), Vec::new()),
        Some(Ok((e, uses))) => (e.text, uses),
        Some(Err(errs)) => return Err(Diagnostic::error(span, format!("the snippet `{query}` does not read: {}", errs[0].message))),
    };
    let inner = parse_depth(&expanded, env, depth + 1);
    if let Some(d) = inner.diagnostics.iter().find(|d| d.is_error()) {
        return Err(Diagnostic::error(span, format!("the snippet `{query}` does not read: {}", d.message)));
    }
    let mut lets = Vec::new();
    let mut defs = Vec::new();
    let mut wires = Vec::new();
    let mut samples = Vec::new();
    let mut closure = closure_of(&inner.imports);
    for (c, t) in borrowed {
        if !closure.iter().any(|(x, _)| *x == c) {
            closure.push((c, t));
        }
    }
    for imp in &inner.imports {
        lets.extend(imp.lets.clone());
        defs.extend(imp.defs.clone());
        wires.extend(imp.wires.clone());
        samples.extend(imp.samples.clone());
    }
    if !closure.iter().any(|(c, _)| *c == cid) {
        closure.push((cid.clone(), text.clone()));
    }
    for st in &inner.program.stmts {
        match &st.kind {
            StmtKind::Let(l) => lets.push(l.clone()),
            StmtKind::Def(d) => defs.push(d.clone()),
            StmtKind::Wire(w) => wires.push(w.clone()),
            StmtKind::Sample(d) => samples.push(d.clone()),
            _ => {}
        }
    }
    // every token now says it came from this import line
    for l in &mut lets {
        l.name.span = span;
        l.value = crate::lex::put_at(&l.value, span);
    }
    for d in &mut defs {
        d.name.span = span;
        for p in &mut d.params {
            p.span = span;
        }
        d.body = crate::lex::put_at(&d.body, span);
        for p in d.ports.ins.iter_mut().chain(d.ports.outs.iter_mut()) {
            p.span = span;
        }
    }
    for w in &mut wires {
        for word in [&mut w.from.def, &mut w.from.port, &mut w.to.def, &mut w.to.port] {
            word.span = span;
        }
    }
    Ok(Imported { span, cid, lets, defs, samples, wires, closure })
}

/// A pattern from tokens (a `let`'s value): all of them must be pattern terms.
pub fn parse_seq_tokens(mut toks: Vec<Token>, src: &str) -> R<Seq> {
    let end = toks.last().map_or(0, |t| t.span.end);
    toks.push(Token { tok: Tok::Eof, span: Span::new(end, end) });
    let mut p = Parser { toks, at: 0, src, diags: Vec::new(), depth: 0, ratios: BTreeSet::new() };
    let seq = p.seq()?;
    if p.peek() != &Tok::Eof {
        return Err(p.expected(&["a step", "a note", "`~`", "`[`", "`<`", "a generator like e(5,8)"], &format!("expected a pattern term, found {}", p.found())));
    }
    if seq.terms.is_empty() {
        return Err(Diagnostic::error(Span::new(end, end), "a pattern has nothing in it"));
    }
    Ok(seq)
}

/// Parse the statements of already-lexed tokens (a `def`'s or `repeat`'s body once its parameters
/// are filled in). `src` is the text the tokens' spans point into.
pub fn parse_tokens(mut toks: Vec<Token>, src: &str) -> (Vec<Stmt>, Vec<Diagnostic>) {
    let end = toks.last().map_or(0, |t| t.span.end);
    toks.push(Token { tok: Tok::Eof, span: Span::new(end, end) });
    let mut p = Parser { toks, at: 0, src, diags: Vec::new(), depth: 0, ratios: BTreeSet::new() };
    let stmts = p.statements(false);
    (stmts, p.diags)
}

pub(crate) struct Parser<'a> {
    pub(crate) toks: Vec<Token>,
    pub(crate) at: usize,
    pub(crate) src: &'a str,
    pub(crate) diags: Vec<Diagnostic>,
    /// How many groups, alternations, calls, `deg( )`s and sections the text is inside of just now ([`MAX_NESTING`]).
    pub(crate) depth: usize,
    /// The names of the `let`s whose number is a ratio that came of arithmetic (`let third = 1/3`): the number put in place of
    /// such a name keeps being said as that fraction (see [`prepass`]). Empty where the tokens are a body filled in.
    pub(crate) ratios: BTreeSet<String>,
}

/// How deep the text may nest: groups `[ ]`, alternations `< >`, calls and their arguments, `deg( )` and sections,
/// counted together by one counter ([`Parser::nested`]). The parser is recursive, so a text that nests without a
/// bound would overflow the stack of the thread that reads it (an editor, a share link, the control socket).
pub const MAX_NESTING: usize = 64;

type R<T> = Result<T, Diagnostic>;

impl<'a> Parser<'a> {
    pub(crate) fn peek(&self) -> &Tok {
        &self.toks[self.at.min(self.toks.len() - 1)].tok
    }

    pub(crate) fn peek_n(&self, n: usize) -> &Tok {
        &self.toks[(self.at + n).min(self.toks.len() - 1)].tok
    }

    pub(crate) fn span(&self) -> Span {
        self.toks[self.at.min(self.toks.len() - 1)].span
    }

    pub(crate) fn span_n(&self, n: usize) -> Span {
        self.toks[(self.at + n).min(self.toks.len() - 1)].span
    }

    pub(crate) fn prev_span(&self) -> Span {
        self.toks[self.at.saturating_sub(1)].span
    }

    pub(crate) fn bump(&mut self) -> Token {
        let t = self.toks[self.at.min(self.toks.len() - 1)].clone();
        if self.at < self.toks.len() - 1 {
            self.at += 1;
        }
        t
    }

    /// Run `f` one level deeper. Text that nests more than [`MAX_NESTING`] is an error at its opener (`open`), before
    /// anything below it is read. `fatal`: nothing after it is read either (a section's block holds whole statements,
    /// and every line after it would be one more error of the same kind).
    pub(crate) fn nested<T>(&mut self, open: Span, what: &str, fatal: bool, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        if self.depth >= MAX_NESTING {
            if fatal {
                self.at = self.toks.len() - 1;
            }
            return Err(Diagnostic::error(open, format!("{what} nest more than {MAX_NESTING} deep")).help("flatten it: name the inner part with a `let`, or write it out longhand"));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    pub(crate) fn skip_line(&mut self) {
        while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
            self.at += 1;
        }
    }

    pub(crate) fn word_is(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Word(x) if x == w)
    }

    pub(crate) fn found(&self) -> String {
        self.peek().describe()
    }

    /// "expected X, found Y" at the current token.
    pub(crate) fn expected(&self, what: &[&str], message: &str) -> Diagnostic {
        Diagnostic::error(self.span(), message).expected(what).found(self.found())
    }

    pub(crate) fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, t: Tok, what: &str) -> R<Span> {
        if self.peek() == &t {
            Ok(self.bump().span)
        } else {
            Err(self.expected(&[what], &format!("expected {what}, found {}", self.found())))
        }
    }

    pub(crate) fn expect_word(&mut self, w: &str) -> R<Span> {
        if self.word_is(w) {
            Ok(self.bump().span)
        } else {
            let mut d = self.expected(&[&format!("`{w}`")], &format!("expected `{w}`, found {}", self.found()));
            if let Tok::Word(found) = self.peek() {
                if nearest(found, [w]).is_some() {
                    d = d.help(format!("did you mean `{w}`?"));
                }
            }
            Err(d)
        }
    }

    pub(crate) fn word(&mut self, what: &str) -> R<Word> {
        match self.peek().clone() {
            Tok::Word(text) => {
                let span = self.bump().span;
                Ok(Word { text, span })
            }
            _ => Err(self.expected(&[what], &format!("expected {what}, found {}", self.found()))),
        }
    }

    /// A number: a literal as written, or arithmetic over numbers (`60 + 6`, `(2 + 1) * 4`, `$i * 2`; see
    /// [`crate::expr`]). A lone literal keeps its spelling and its exact value.
    pub(crate) fn num(&mut self, what: &str) -> R<Num> {
        if self.starts_arith() {
            return self.arith_num(what);
        }
        self.num_lit(what)
    }

    /// One number literal (with its `-`), never arithmetic: where numbers stand side by side (`rotate 1 -2`).
    pub(crate) fn num_lit(&mut self, what: &str) -> R<Num> {
        let neg = if self.peek() == &Tok::Minus {
            self.bump();
            true
        } else {
            false
        };
        match self.peek().clone() {
            Tok::Num { value, int } => {
                let t = self.bump();
                let value = if neg { -value } else { value };
                let mut n = Num::lit(value, int, t.span);
                // a number put in place of a `let`'s name (`let third = 1/3`, `level third`) says what it is, as `1/3` typed here would
                if !int && !self.ratios.is_empty() && self.ratios.contains(t.span.text(self.src)) {
                    n.canon = crate::print::ratio_text(value).map(|r| crate::expr::intern(&r));
                }
                Ok(n)
            }
            _ => Err(self.expected(&[what], &format!("expected {what}, found {}", self.found()))),
        }
    }

    pub(crate) fn int(&mut self, what: &str) -> R<(u32, Span)> {
        let n = self.num(what)?;
        self.whole(n, what)
    }

    /// A whole number written as a literal, never arithmetic.
    pub(crate) fn int_lit(&mut self, what: &str) -> R<(u32, Span)> {
        let n = self.num_lit(what)?;
        self.whole(n, what)
    }

    fn whole(&self, n: Num, what: &str) -> R<(u32, Span)> {
        if !n.int || n.value < 0.0 || n.value > u32::MAX as f64 {
            return Err(Diagnostic::error(n.span, format!("{what} is a whole number")).found(n.span.text(self.src).to_string()));
        }
        Ok((n.value as u32, n.span))
    }

    pub(crate) fn end_of_statement(&mut self) -> R<()> {
        match self.peek() {
            Tok::Newline | Tok::Eof => Ok(()),
            // (the `}` that ends a block on the line of its last statement: `section a 4 bars { deck A: kick x }`)
            Tok::RBrace if self.unclosed_braces() > 0 => Ok(()),
            _ => {
                let mut d = self.expected(&["the end of the line"], &format!("expected the end of the line, found {}", self.found()));
                if let Some(h) = self.trailing_hint() {
                    d = d.help(h);
                }
                Err(d)
            }
        }
    }

    /// How many `{` before the current token are not closed.
    pub(crate) fn unclosed_braces(&self) -> i64 {
        self.toks[..self.at.min(self.toks.len())].iter().fold(0, |n, t| match t.tok {
            Tok::LBrace => n + 1,
            Tok::RBrace => n - 1,
            _ => n,
        })
    }

    /// What may have been meant by the token that stops a statement short: a keyword with a letter wrong, a guard written without
    /// its `when`, a mark of `deg( )` outside it.
    fn trailing_hint(&self) -> Option<String> {
        match self.peek() {
            Tok::Word(w) => {
                let near = nearest(w, ["when", "over", "for", "every", "from", "in", "on", "under", "weights", "seed", "exp", "log", "linear", "step", "s-curve", "and", "or", "attack", "release"])?;
                Some(format!("did you mean `{near}`?"))
            }
            Tok::Eq | Tok::Bang if self.peek_n(1) == &Tok::Eq => Some("a comparison belongs to a guard: `... when $i == 1` (is `when` misspelt?)".into()),
            Tok::Apostrophe => Some("`'` is the octave mark of a degree and is read inside `deg( )`: `deg(5')`".into()),
            Tok::Comma | Tok::OctDown => Some("a comma stacks notes inside `deg( )` and separates the arguments of a call: `deg(1,3,5)`, `e(5, 8)`; steps of a pattern are separated by spaces".into()),
            Tok::Plus => Some("`+` is arithmetic, and a number is where it stands; a tolerance is `\u{b1}` or `+-`: `assert length == 3m30s \u{b1} 5s`".into()),
            Tok::Num { .. } => Some("a number after the end of a value: is an operator missing (`+ - * /`), or a unit (`3m30s`)?".into()),
            Tok::Unit(u) => Some(format!("the unit `{u}` is read where the language takes one: `6db` in a `duck` or a mix item, `5ms` and `120ms` after `attack=` and `release=`, `3m30s` in `assert length`, `20m` or `1h30m` after a set's `until`; a filter's cutoff may be `300hz`, anywhere else a number is plain")),
            _ => None,
        }
    }

    // ---- statements

    /// Statements up to the end of the text, or (in a block) the `}` that closes it.
    pub(crate) fn statements(&mut self, in_block: bool) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        // the last statement failed: a `{` that starts the next line is the body of its header, and is skipped without a word
        let mut failed = false;
        loop {
            while self.peek() == &Tok::Newline {
                self.at += 1;
            }
            match self.peek() {
                Tok::Eof => break,
                Tok::RBrace if in_block => break,
                Tok::RBrace => {
                    self.diags.push(Diagnostic::error(self.span(), "a `}` with no `{` before it"));
                    self.skip_line();
                    continue;
                }
                Tok::LBrace if failed => {
                    let from = self.at;
                    self.skip_statement(from);
                    continue;
                }
                _ => {}
            }
            let from = self.at;
            match self.statement() {
                Ok(s) => {
                    stmts.push(s);
                    failed = false;
                }
                Err(d) => {
                    self.diags.push(d);
                    // the rest of the statement, and of its block if it has one (a header that did not read leaves its
                    // body, which would be read as statements of its own)
                    self.skip_statement(from);
                    failed = true;
                }
            }
        }
        stmts
    }

    /// Skip what is left of the statement that began at token `from` and failed: to the end of its line, and when it opened
    /// a `{` that is not closed, to the `}` that closes it. The `}` of an enclosing block is left.
    pub(crate) fn skip_statement(&mut self, from: usize) {
        let braces = |toks: &[Token]| toks.iter().fold(0i64, |n, t| match t.tok {
            Tok::LBrace => n + 1,
            Tok::RBrace => n - 1,
            _ => n,
        });
        let from = from.min(self.toks.len());
        let mut depth = braces(&self.toks[from..self.at.min(self.toks.len())]);
        // (the braces open before the statement are its enclosing blocks: counted when a `}` first needs them)
        let mut enclosing: Option<i64> = None;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Newline if depth <= 0 => return,
                Tok::RBrace if depth <= 0 && *enclosing.get_or_insert_with(|| braces(&self.toks[..from])) > 0 => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                _ => {}
            }
            self.at += 1;
        }
    }

    /// The tokens of a `{ ... }` block, braces off (the `{` is the next token).
    pub(crate) fn block_tokens(&mut self, what: &str) -> R<Vec<Token>> {
        let open = self.expect(Tok::LBrace, "`{`")?;
        let mut depth = 1;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Tok::Eof => return Err(Diagnostic::error(open, format!("this `{{` of {what} is never closed")).help("a block ends with a `}` on a line of its own")),
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                }
                _ => {}
            }
            out.push(self.bump());
        }
        while matches!(out.first(), Some(Token { tok: Tok::Newline, .. })) {
            out.remove(0);
        }
        while matches!(out.last(), Some(Token { tok: Tok::Newline, .. })) {
            out.pop();
        }
        Ok(out)
    }

    pub(crate) fn statement(&mut self) -> R<Stmt> {
        let start = self.span();
        let Tok::Word(w) = self.peek().clone() else {
            return Err(self
                .expected(&["a setting (tempo, key, swing)", "`deck A:`", "`master:`", "`every N bars:`", "`on phrase:`", "`at bar N:`", "`let`", "`def`", "`section`", "`do`"], &format!("a statement cannot start with {}", self.found()))
                .help("a line is `deck A: kick x ~ x ~`, `master: filter lp 2k`, `every 16 bars: swap pairs`; see docs/BTW.md"));
        };
        let key_is_setting = || {
            // `key am`, `key d:dorian`: a setting. `key detect deck=A`: the registry's `key`.
            match (self.peek_n(1), self.peek_n(2)) {
                (Tok::Word(k), _) => !(KEY_VERBS.contains(&k.as_str()) && !matches!(self.peek_n(2), Tok::Colon)),
                _ => true,
            }
        };
        let kind = match w.as_str() {
            "tempo" => {
                self.bump();
                // `tempo 126`, or a curve: `tempo 100 -> 130 over 8 bars exp` (F3, tempo.rs)
                match self.tempo_spec()? {
                    crate::tempo::TempoSpec::Set(n) => StmtKind::Setting(Setting::Tempo(n)),
                    crate::tempo::TempoSpec::Ramp(r) => StmtKind::Setting(Setting::TempoRamp(r)),
                }
            }
            "swing" => {
                self.bump();
                StmtKind::Setting(Setting::Swing(self.num("a swing amount 0..1")?))
            }
            "key" if key_is_setting() => {
                self.bump();
                StmtKind::Setting(Setting::Key(self.key_word()?))
            }
            "let" => {
                self.bump();
                let name = self.word("a name for the value")?;
                self.expect(Tok::Eq, "`=`")?;
                // F10: `let both = stack(a, b)` names a composition of sections (algebra.rs)
                if self.sexpr_next() {
                    let expr = self.sexpr()?;
                    self.end_of_statement()?;
                    return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::LetSection(crate::algebra::LetSection { name, expr }) });
                }
                let mut value = Vec::new();
                while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    value.push(self.bump());
                }
                if value.is_empty() {
                    return Err(Diagnostic::error(self.prev_span(), format!("`let {} =` has no value", name.text)).help("let riff = c2 ~ [e2 g2] ~   or   let bars = 8"));
                }
                if let Some(d) = crate::perform::let_with_input(&name, &value) {
                    return Err(d);
                }
                let ratio = value.len() == 1 && self.ratios.contains(&name.text);
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let(LetDef { name, value, ratio }) });
            }
            "def" => {
                self.bump();
                let name = self.word("a name for the def")?;
                self.expect(Tok::LParen, "`(`")?;
                let mut params = Vec::new();
                if self.peek() != &Tok::RParen {
                    loop {
                        params.push(self.word("a parameter name")?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
                let ports = self.def_ports()?;
                let body = self.block_tokens(&format!("`def {}`", name.text))?;
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Def(DefDef { name, params, body, ports }) });
            }
            "repeat" => {
                self.bump();
                let (n, nsp) = self.int("how many times (a whole number written out)")?;
                if n == 0 || n > 64 {
                    return Err(Diagnostic::error(nsp, "a repeat is 1 to 64 times").help("the count is written out, so a program always ends"));
                }
                let every = if self.word_is("every") {
                    self.bump();
                    let (m, _) = self.int("a number of bars")?;
                    if !self.word_is("bars") && !self.word_is("bar") {
                        return Err(self.expected(&["`bars`"], &format!("expected `bars`, found {}", self.found())));
                    }
                    self.bump();
                    Some(m)
                } else {
                    None
                };
                let body = self.block_tokens("`repeat`")?;
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Repeat(RepeatBlock { n, every, body }) });
            }
            "section" => {
                self.bump();
                let name = self.word("a name for the section")?;
                // `section intro from seed(techno, 42)[1..8]` (seed.rs)
                if self.word_is("from") {
                    self.bump();
                    let what = self.seed_word(true)?;
                    self.end_of_statement()?;
                    return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Seed(SeedStmt { section: Some(name), what }) });
                }
                let (bars, bars_span) = self.int("the section's length in bars")?;
                if bars == 0 || bars > 256 {
                    return Err(Diagnostic::error(bars_span, "a section is 1 to 256 bars"));
                }
                if !self.word_is("bars") && !self.word_is("bar") {
                    return Err(self.expected(&["`bars`"], &format!("expected `bars`, found {}", self.found())));
                }
                self.bump();
                self.expect(Tok::LBrace, "`{`")?;
                let body = self.nested(start, "sections", true, |p| Ok(p.statements(true)))?;
                if self.peek() != &Tok::RBrace {
                    // (a block cut short by a text nested too deep was said once, at the opener that was too deep)
                    if self.peek() == &Tok::Eof && self.diags.iter().any(|d| d.message.ends_with(&format!("nest more than {MAX_NESTING} deep"))) {
                        return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Section(SectionBlock { name, bars, bars_span, body }) });
                    }
                    return Err(Diagnostic::error(start, format!("the `{{` of section `{}` is never closed", name.text)));
                }
                self.bump();
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Section(SectionBlock { name, bars, bars_span, body }) });
            }
            "arrange" if matches!(self.peek_n(1), Tok::LBrace) => {
                self.bump();
                self.arrange_block(start)?
            }
            "arrange" if matches!(self.peek_n(1), Tok::Word(f) if f == "from") => {
                self.bump();
                self.bump();
                StmtKind::Seed(SeedStmt { section: None, what: self.seed_word(false)? })
            }
            "arrange" => {
                self.bump();
                // F10: names, `stack(...)`, `then(...)`, `silence` (algebra.rs); a bare list of names is still `Arrange`
                self.arrange_stmt()?
            }
            "start" if matches!(self.peek_n(1), Tok::Word(_)) && matches!(self.peek_n(2), Tok::Newline | Tok::Eof) => {
                self.bump();
                StmtKind::Start(self.word("an arrangement name")?)
            }
            // `launch drop on pad(2)`; a call of a def of that name (`launch(A)`) is told apart by what follows the word
            "launch" if !matches!(self.peek_n(1), Tok::LParen | Tok::Colon) => {
                self.bump();
                return self.launch_statement(start);
            }
            "end" => {
                self.bump();
                if self.word_is("at") {
                    self.bump();
                    self.expect_word("bar")?;
                    let (n, sp) = self.int("a bar number")?;
                    if n == 0 {
                        return Err(Diagnostic::error(sp, "a program cannot end before it begins").help("end at bar 64"));
                    }
                    StmtKind::End(EndWhen::At(n))
                } else if self.word_is("after") {
                    self.bump();
                    StmtKind::End(EndWhen::After(self.word("a section name")?))
                } else {
                    StmtKind::End(EndWhen::Song)
                }
            }
            "loop" if matches!(self.peek_n(1), Tok::Str(_)) => {
                self.bump();
                let Tok::Str(text) = self.peek().clone() else { unreachable!() };
                let sp = self.bump().span;
                let src = Word { text, span: sp };
                self.expect_word("as")?;
                let name = self.sample_name()?;
                let otherwise = self.sample_else()?;
                StmtKind::Sample(SampleDecl { kind: SampleKind::Loop, name, src, license: None, otherwise })
            }
            "loop" => {
                self.bump();
                StmtKind::Loop
            }
            "sample" => {
                self.bump();
                let name = self.sample_name()?;
                self.expect_word("from")?;
                let src = match self.peek().clone() {
                    Tok::Str(text) => {
                        let span = self.bump().span;
                        Word { text, span }
                    }
                    _ => return Err(self.expected(&["a quoted path or \"sha256:...\""], &format!("`sample NAME from \"...\"` needs the file in quotes, found {}", self.found())).help("sample harp from \"~/samples/harp.wav\"")),
                };
                let mut license = None;
                if self.word_is("license") {
                    self.bump();
                    let w = self.word("a license tag")?;
                    if crate::assets::AssetRights::parse(&w.text).is_none() {
                        return Err(Diagnostic::error(w.span, format!("`{}` is not a sample license", w.text)).expected(&crate::assets::AssetRights::IDS).found(format!("`{}`", w.text)));
                    }
                    license = Some(w);
                }
                let otherwise = self.sample_else()?;
                StmtKind::Sample(SampleDecl { kind: SampleKind::Sample, name, src, license, otherwise })
            }
            "set" if matches!(self.peek_n(1), Tok::LBrace) => {
                self.bump();
                let block = self.set_block(start)?;
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Set(block) });
            }
            "station" if matches!(self.peek_n(1), Tok::LBrace) => {
                self.bump();
                let block = self.station_block(start)?;
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Station(block) });
            }
            "use" => {
                self.bump();
                let w = self.word("`restricted`")?;
                if w.text != "restricted" {
                    return Err(Diagnostic::error(w.span, format!("`use {}`: only `use restricted` exists", w.text)).help("use restricted"));
                }
                StmtKind::UseRestricted
            }
            "wire" => return self.wire_statement(start),
            "assert" => return self.assert_statement(start),
            "evolve" => {
                self.bump();
                let block = self.evolve_block(start)?;
                return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Evolve(block) });
            }
            "import" => {
                self.bump();
                let what = match self.peek().clone() {
                    Tok::Str(text) => {
                        let span = self.bump().span;
                        Word { text, span }
                    }
                    _ => self.word("a saved snippet's name or CID")?,
                };
                StmtKind::Import(ImportRef { what })
            }
            _ => StmtKind::Line(self.line()?),
        };
        self.end_of_statement()?;
        Ok(Stmt { span: start.to(self.prev_span()), kind })
    }

    /// `NAME on pad(N) [next bar | next phrase [8 | 16 | 32]]` (the `launch` is read): the scene, the pad that brings it in, and the bar line it
    /// lands on (the next phrase line, 8 bars, when it says no more).
    fn launch_statement(&mut self, start: Span) -> R<Stmt> {
        let scene = self.word("a section or an arrangement to launch: `launch drop on pad(2)`")?;
        if !self.word_is("on") {
            return Err(self
                .expected(&["`on pad(N)`"], &format!("expected `on pad(N)` after `launch {}`, found {}", scene.text, self.found()))
                .help(format!("a launch is started by a pad: `launch {} on pad(2)`", scene.text)));
        }
        self.bump();
        if let Tok::Word(w) = self.peek().clone() {
            if crate::perform::is_input_word(&w) && w != "pad" {
                return Err(Diagnostic::error(self.span(), format!("a launch is started by a pad: `launch {} on pad(2)`", scene.text)).found(format!("`{w}`")));
            }
        }
        let pad_at = self.expect_word("pad")?;
        self.expect(Tok::LParen, "`(`")?;
        let (n, n_span) = self.int_lit("a pad number")?;
        let close = self.expect(Tok::RParen, "`)`")?;
        let pad = crate::perform::pad_number(n, n_span)?;
        let quantum = if self.word_is("next") {
            self.bump();
            if self.word_is("bar") {
                self.bump();
                LaunchQuantum::Bar
            } else {
                self.expect_word("phrase")?;
                if let Tok::Num { .. } = self.peek() {
                    let (n, sp) = self.int_lit("a phrase length: 8, 16 or 32")?;
                    if ![8, 16, 32].contains(&n) {
                        return Err(Diagnostic::error(sp, "a phrase is 8, 16 or 32 bars").found(n.to_string()));
                    }
                    LaunchQuantum::Phrase(n)
                } else {
                    LaunchQuantum::Phrase(PHRASE)
                }
            }
        } else {
            LaunchQuantum::Phrase(PHRASE)
        };
        self.end_of_statement()?;
        Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Launch(LaunchStmt { scene, pad, pad_span: pad_at.to(close), quantum }) })
    }

    /// `{ play X [until W]  mix N bars with T [, tempo A -> B] ... }` (the `set` is read).
    fn set_block(&mut self, start: Span) -> R<SetBlock> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut items = Vec::new();
        loop {
            while self.peek() == &Tok::Newline {
                self.at += 1;
            }
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => return Err(Diagnostic::error(start, "the `{` of this `set` is never closed")),
                Tok::Word(w) if w == "play" => {
                    let istart = self.bump().span;
                    let what = match self.peek().clone() {
                        Tok::Str(text) => {
                            let span = self.bump().span;
                            Word { text, span }
                        }
                        _ => self.word("a saved program's name or CID, or a generator call")?,
                    };
                    let mut until = None;
                    if self.word_is("until") {
                        let us = self.bump().span;
                        until = Some(self.until_when(us)?);
                    }
                    items.push(SetItem::Play(PlayItem { what, until, span: istart.to(self.prev_span()) }));
                }
                Tok::Word(w) if w == "mix" => {
                    let istart = self.bump().span;
                    let (bars, bars_span) = self.int("a number of bars")?;
                    if !self.word_is("bars") && !self.word_is("bar") {
                        return Err(self.expected(&["`bars`"], &format!("expected `bars`, found {}", self.found())).help("mix 8 bars with crossfade"));
                    }
                    self.bump();
                    self.expect_word("with")?;
                    let with = self.word("a transition (crossfade, filter_exit, echo_out, cut)")?;
                    let mut tempo = None;
                    if self.eat(&Tok::Comma) {
                        self.expect_word("tempo")?;
                        let a = self.num("the tempo to start from")?;
                        self.expect(Tok::Arrow, "`->`")?;
                        let b = self.num("the tempo to end on")?;
                        tempo = Some((a.value, b.value));
                    }
                    items.push(SetItem::Mix(MixItem { bars, bars_span, with, tempo, span: istart.to(self.prev_span()) }));
                }
                Tok::Word(w) if w == "next" => {
                    let istart = self.bump().span;
                    let step = self.seed_step()?;
                    let mut until = None;
                    if self.word_is("until") {
                        let us = self.bump().span;
                        until = Some(self.until_when(us)?);
                    }
                    items.push(SetItem::Next(NextItem { step, until, span: istart.to(self.prev_span()) }));
                }
                other => {
                    let _ = other;
                    return Err(self.expected(&["`play`", "`mix`", "`next`", "`}`"], &format!("a set holds `play`, `mix` and `next` lines, found {}", self.found())));
                }
            }
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
                return Err(self.expected(&["the end of the line"], &format!("expected the end of the line, found {}", self.found())));
            }
        }
        Ok(SetBlock { items })
    }

    /// `seed=+N` or `seed=~`: the step from a track to the next (`next` in a set or a station).
    fn seed_step(&mut self) -> R<SeedStep> {
        if !self.word_is("seed") {
            return Err(self.expected(&["`seed=+1`", "`seed=~`"], &format!("`next` takes a step, `seed=+1` or `seed=~`, found {}", self.found())).help("next seed=+1   next seed=~"));
        }
        self.bump();
        self.expect(Tok::Eq, "`=`")?;
        match self.peek().clone() {
            Tok::Tilde => {
                self.bump();
                Ok(SeedStep::Hash)
            }
            Tok::Plus => {
                self.bump();
                let (n, sp) = self.int_lit("a whole number of seeds to count on")?;
                if n == 0 {
                    return Err(Diagnostic::error(sp, "`seed=+0` steps nowhere: count on by 1 or more").help("next seed=+1"));
                }
                Ok(SeedStep::By(n))
            }
            _ => Err(self.expected(&["`+`", "`~`"], &format!("a step is `seed=+N` (count on) or `seed=~` (a hash of the track before), found {}", self.found())).help("next seed=+1   next seed=~")),
        }
    }

    /// `{ from gen@2 techno seed=42  next seed=~  forms seeded rondo }` (the `station` is read).
    fn station_block(&mut self, start: Span) -> R<StationBlock> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut items = Vec::new();
        loop {
            while self.peek() == &Tok::Newline {
                self.at += 1;
            }
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => return Err(Diagnostic::error(start, "the `{` of this `station` is never closed")),
                Tok::Word(w) if w == "from" => {
                    let istart = self.bump().span;
                    let what = self.word("the first track's generator call: gen@2 techno seed=42")?;
                    items.push(StationItem::From(what, istart.to(self.prev_span())));
                }
                Tok::Word(w) if w == "next" => {
                    let istart = self.bump().span;
                    let step = self.seed_step()?;
                    items.push(StationItem::Next(step, istart.to(self.prev_span())));
                }
                Tok::Word(w) if w == "forms" => {
                    let istart = self.bump().span;
                    let mut names = Vec::new();
                    while let Tok::Word(_) = self.peek() {
                        names.push(self.word("a form")?);
                    }
                    if names.is_empty() {
                        return Err(self.expected(&["a form"], &format!("`forms` names the forms in turn, found {}", self.found())).help("forms seeded passacaglia rondo fugue"));
                    }
                    items.push(StationItem::Forms(names, istart.to(self.prev_span())));
                }
                _ => return Err(self.expected(&["`from`", "`next`", "`forms`", "`}`"], &format!("a station holds `from`, `next` and `forms` lines, found {}", self.found())).help("station {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia rondo\n}")),
            }
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
                return Err(self.expected(&["the end of the line"], &format!("expected the end of the line, found {}", self.found())));
            }
        }
        Ok(StationBlock { items })
    }

    /// The seconds the number at the cursor is, when its unit (`h`, `m`, `s`) touches it: `20m` is 1200.
    fn time_part(&self) -> Option<f64> {
        let (Token { tok: Tok::Num { value, .. }, span: n }, Token { tok: Tok::Unit(u), span: us }) = (self.toks.get(self.at)?, self.toks.get(self.at + 1)?) else { return None };
        let unit = match u.as_str() {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            _ => return None,
        };
        (n.end == us.start).then_some(value * unit)
    }

    /// What follows `until`: `20m`, `1h30m`, `bar 96`, `drop`, `after drop`.
    fn until_when(&mut self, at: Span) -> R<(UntilWhen, Span)> {
        let start = self.span();
        let when = match self.peek().clone() {
            Tok::Num { .. } if self.time_part().is_some() => {
                // `20m`, `90s`, `1h`, `3m30s`, `1h30m`: a number and its unit, then the next ones while they touch
                let mut secs = 0.0;
                while let Some(part) = self.time_part() {
                    if secs != 0.0 && self.span().start != self.prev_span().end {
                        break;
                    }
                    self.bump();
                    self.bump();
                    secs += part;
                }
                UntilWhen::Seconds(secs)
            }
            Tok::Word(w) if w == "bar" => {
                self.bump();
                let (n, sp) = self.int("a bar number")?;
                if n == 0 {
                    return Err(Diagnostic::error(sp, "a track cannot be cut before it begins").help("until bar 96"));
                }
                UntilWhen::Bar(n)
            }
            Tok::Word(w) if w == "after" => {
                self.bump();
                UntilWhen::After(self.word("a section name")?.text)
            }
            Tok::Word(w) => {
                self.bump();
                UntilWhen::Section(w)
            }
            _ => return Err(Diagnostic::error(at, "`until` takes a time (20m), `bar N`, a section name or `after SECTION`").help("play deep-house-60 until 20m")),
        };
        Ok((when, start.to(self.prev_span())))
    }

    /// The name a `sample` or a `loop ... as` gives: a plain word that is not a note or a keyword.
    /// The reference after `from` (one word since seed::collapse and stored::collapse): a seed, and after `section NAME` a stored
    /// program's section too (`stored`).
    pub(crate) fn seed_word(&mut self, stored: bool) -> R<Word> {
        match self.peek() {
            Tok::Word(w) if crate::seed::is_ref_word(w) || w.starts_with("template(") => self.word("a seed"),
            Tok::Word(w) if stored && crate::stored::is_ref_word(w) => self.word("a stored program"),
            Tok::Word(w) if crate::stored::is_ref_word(w) => Err(Diagnostic::error(self.span(), "`arrange from` takes a seed of the generator or a template: another program as a whole is a fork").help("fork it and edit the copy, or borrow its sections: section hook from <cid>.drop")),
            _ if stored => Err(self.expected(&["seed(genre, n)", "template(name, phrase)", "a stored program's CID"], &format!("`from` takes a seed of the generator, a template or a stored program, found {}", self.found())).help("section intro from seed(techno, 42)[1..8]   section hook from <cid>.drop")),
            _ => Err(self.expected(&["seed(genre, n)", "template(name, phrase)"], &format!("`from` takes a seed of the generator or a template, found {}", self.found())).help("section intro from seed(techno, 42)[1..8]   arrange from seed(techno, 42)   arrange from template(deep-house, @tempest_such_stuff)")),
        }
    }

    pub(crate) fn sample_name(&mut self) -> R<Word> {
        if let Tok::Num { .. } = self.peek() {
            // a `let` above gave this word a value, so here it is that number, not a name
            let at = self.span();
            let text = at.text(self.src);
            if text.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                return Err(Diagnostic::error(at, format!("a `let` name cannot name a sound: `{text}` is a value here"))
                    .help(format!("name the sound something the program does not `let`: {text}_sound")));
            }
        }
        let w = self.word("a name for the sound")?;
        if let Some(why) = reserved(&w.text) {
            return Err(Diagnostic::error(w.span, format!("`{}` cannot name a sound: {why}", w.text)).help(rename_hint(&w.text)));
        }
        Ok(w)
    }

    /// `else VOICE`, when there.
    pub(crate) fn sample_else(&mut self) -> R<Option<Word>> {
        if self.word_is("else") {
            self.bump();
            return Ok(Some(self.word("an instrument of ours to play instead")?));
        }
        Ok(None)
    }

    /// `every 8 bars { rule ... stop when gen == 12 }` (the `evolve` is read).
    pub(crate) fn evolve_block(&mut self, start: Span) -> R<EvolveBlock> {
        self.expect_word("every")?;
        let (every, esp) = self.int("a number of bars")?;
        if every == 0 || every > 64 {
            return Err(Diagnostic::error(esp, "a generation is 1 to 64 bars long"));
        }
        if !self.word_is("bars") && !self.word_is("bar") {
            return Err(self.expected(&["`bars`"], &format!("expected `bars`, found {}", self.found())));
        }
        self.bump();
        self.expect(Tok::LBrace, "`{`")?;
        let mut rules = Vec::new();
        let mut last: Option<(u32, Span)> = None;
        loop {
            while self.peek() == &Tok::Newline {
                self.at += 1;
            }
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => return Err(Diagnostic::error(start, "the `{` of this `evolve` is never closed")),
                Tok::Word(w) if w == "stop" => {
                    let sp = self.bump().span;
                    self.expect_word("when")?;
                    self.expect_word("gen")?;
                    let (op, _) = self.cmp_op()?;
                    let (n, nsp) = self.int("the generation to stop at")?;
                    let n = match op {
                        CmpOp::Eq | CmpOp::Ge => n,
                        CmpOp::Gt => n + 1,
                        _ => return Err(Diagnostic::error(sp.to(nsp), "evolution stops when `gen == N` or `gen >= N`").help("stop when gen == 12")),
                    };
                    if last.is_some() {
                        return Err(Diagnostic::error(sp, "an evolve stops once").help("keep one `stop when gen == N` or `for N gens`"));
                    }
                    last = Some((n, sp.to(nsp)));
                }
                Tok::Word(w) if w == "for" => {
                    let sp = self.bump().span;
                    let (n, nsp) = self.int("how many generations")?;
                    if !self.word_is("gens") && !self.word_is("gen") {
                        return Err(self.expected(&["`gens`"], &format!("expected `gens`, found {}", self.found())));
                    }
                    self.bump();
                    if last.is_some() {
                        return Err(Diagnostic::error(sp, "an evolve stops once").help("keep one `stop when gen == N` or `for N gens`"));
                    }
                    last = Some((n, sp.to(nsp)));
                }
                _ => rules.push(self.evolve_rule()?),
            }
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return Err(self.expected(&["the end of the line"], &format!("expected the end of the line, found {}", self.found())));
            }
        }
        Ok(EvolveBlock { every, rules, last, span: start.to(self.prev_span()) })
    }

    pub(crate) fn cmp_op(&mut self) -> R<(CmpOp, Span)> {
        let sp = self.span();
        let op = match (self.peek().clone(), self.peek_n(1).clone()) {
            (Tok::Eq, Tok::Eq) => {
                self.bump();
                CmpOp::Eq
            }
            (Tok::Bang, Tok::Eq) => {
                self.bump();
                CmpOp::Ne
            }
            (Tok::Lt, Tok::Eq) => {
                self.bump();
                CmpOp::Le
            }
            (Tok::Gt, Tok::Eq) => {
                self.bump();
                CmpOp::Ge
            }
            (Tok::Lt, _) => CmpOp::Lt,
            (Tok::Gt, _) => CmpOp::Gt,
            _ => return Err(self.expected(&["`==`", "`!=`", "`<`", "`<=`", "`>`", "`>=`"], &format!("expected a comparison, found {}", self.found()))),
        };
        self.bump();
        Ok((op, sp.to(self.prev_span())))
    }

    /// `[gen] [% M] OP N` after a `when`.
    pub(crate) fn gen_cond(&mut self) -> R<GenCond> {
        let start = self.span();
        self.expect_word("gen")?;
        let modulo = if self.eat(&Tok::Percent) {
            let (m, sp) = self.int_lit("the modulus")?;
            if m == 0 {
                return Err(Diagnostic::error(sp, "a modulus of 0"));
            }
            Some(m)
        } else {
            None
        };
        let (op, _) = self.cmp_op()?;
        let (value, _) = self.int_lit("a generation number")?;
        Ok(GenCond { modulo, op, value, span: start.to(self.prev_span()) })
    }

    pub(crate) fn evolve_rule(&mut self) -> R<EvolveRule> {
        let start = self.span();
        let target = if self.word_is("deck") {
            self.bump();
            EvolveTarget::Deck(self.deck_word()?)
        } else {
            let w = self.word("a role (melody, hihat, bass ...) or `deck A`")?;
            if !ROLES.contains(&w.text.as_str()) {
                let mut d = Diagnostic::error(w.span, format!("`{}` is not a role", w.text)).expected(&ROLES.map(|r| r)).found(format!("`{}`", w.text));
                if let Some(n) = nearest(&w.text, ROLES) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                return Err(d);
            }
            EvolveTarget::Role(w)
        };
        self.expect(Tok::Colon, "`:` after the target")?;
        let mut ops = vec![self.evolve_op()?];
        while self.eat(&Tok::Pipe) {
            ops.push(self.evolve_op()?);
        }
        let when = if self.word_is("when") {
            self.bump();
            Some(self.gen_cond()?)
        } else {
            None
        };
        Ok(EvolveRule { span: start.to(self.prev_span()), target, ops, when })
    }

    pub(crate) fn evolve_op(&mut self) -> R<EvolveOp> {
        let name = self.word("a transform: rotate, transpose, octave, invert, reverse, thin")?;
        let start = name.span;
        let mut args = Vec::new();
        let mut seed = None;
        loop {
            match self.peek().clone() {
                Tok::Num { .. } | Tok::Minus => args.push(EvolveArg::Num(self.num_lit("a number")?)),
                Tok::Lt => {
                    self.bump();
                    let mut v = Vec::new();
                    while matches!(self.peek(), Tok::Num { .. } | Tok::Minus) {
                        v.push(self.num_lit("a number")?);
                    }
                    if v.is_empty() || self.peek() != &Tok::Gt {
                        return Err(self.expected(&["a number", "`>`"], &format!("expected numbers and `>`, found {}", self.found())));
                    }
                    self.bump();
                    args.push(EvolveArg::Alt(v));
                }
                Tok::Word(w) if w == "seed" && self.peek_n(1) == &Tok::Eq => {
                    self.bump();
                    self.bump();
                    seed = Some(self.num_lit("a seed")?);
                }
                Tok::Word(w) if crate::perform::is_input_word(&w) && self.peek_n(1) == &Tok::LParen => return Err(crate::perform::in_evolve(&w, self.span())),
                _ => break,
            }
        }
        Ok(EvolveOp { span: start.to(self.prev_span()), name, args, seed })
    }

    /// `am`, `f#m`, `d:dorian`.
    pub(crate) fn key_word(&mut self) -> R<Word> {
        let mut w = self.word("a key like am, c, f#m or d:dorian")?;
        if self.peek() == &Tok::Colon && matches!(self.peek_n(1), Tok::Word(_)) {
            self.bump();
            let m = self.word("a mode")?;
            w.text = format!("{}:{}", w.text, m.text);
            w.span = w.span.to(m.span);
        }
        Ok(w)
    }

    /// `dnb:2602`, `acid-bass`, `techno:42:standard`.
    pub(crate) fn spec_word(&mut self) -> R<Word> {
        let mut w = self.word("a loop-set or genre spec like dnb:2602")?;
        // (`param=delay.mix`: the lexer reads a dot between two words as one token, `Dot`)
        while (self.peek() == &Tok::Colon && matches!(self.peek_n(1), Tok::Word(_) | Tok::Num { .. })) || (self.peek() == &Tok::Dot && matches!(self.peek_n(1), Tok::Word(_))) {
            let sep = if self.bump().tok == Tok::Dot { '.' } else { ':' };
            let t = self.bump();
            let text = match t.tok {
                Tok::Word(x) => x,
                _ => t.span.text(self.src).to_string(),
            };
            w.text = format!("{}{sep}{}", w.text, text);
            w.span = w.span.to(t.span);
        }
        Ok(w)
    }

    /// A line, and the `when` guard after it if there is one (docs/btw/ext-arith.md).
    pub(crate) fn line(&mut self) -> R<Line> {
        let mut l = self.line_body()?;
        if self.word_is("when") {
            // a guard guards the whole line (docs/btw/ext-arith.md); after a line of several parts that reads as if it guarded the
            // last, so it is said
            if let Body::Parts(parts) = &l.body {
                if parts.len() > 1 {
                    self.diags.push(
                        Diagnostic::warning(self.span(), format!("this `when` guards all {} parts of the line, not only the last", parts.len()))
                            .help("to guard one part, give it its own line: `deck A: kick x ~ x ~ when $i % 2 == 1` and `deck A: snare ~ x ~ x`"),
                    );
                }
            }
            l.guard = Some(self.guard()?);
        }
        Ok(l)
    }

    fn line_body(&mut self) -> R<Line> {
        let mut when = None;
        let mut hold = None;
        if self.word_is("every") || self.word_is("on") || self.word_is("at") || self.word_is("after") || self.word_is("before") || self.word_is("for") {
            let s = self.span();
            if !self.word_is("for") {
                let w = self.when()?;
                when = Some((w, s.to(self.prev_span())));
            }
            if self.word_is("for") {
                let fs = self.span();
                self.bump();
                let (d, dsp) = self.dur("a length (`4 bars`, `2 beats`)")?;
                if d.is_zero() {
                    return Err(Diagnostic::error(dsp, format!("`for {}` would be over before it began", dsp.text(self.src).trim())));
                }
                hold = Some((d, fs.to(self.prev_span())));
                if when.is_none() {
                    when = Some((When::Now, fs.to(self.prev_span())));
                }
            }
            self.expect(Tok::Colon, "`:` after the time word")?;
        }
        let mut target = None;
        if self.word_is("deck") || self.word_is("master") {
            let s = self.span();
            let t = if self.word_is("master") {
                self.bump();
                Target::Master
            } else {
                self.bump();
                Target::Deck(self.deck_word()?)
            };
            let sp = s.to(self.prev_span());
            self.expect(Tok::Colon, "`:` after the target")?;
            target = Some((t, sp));
            if self.control_head_next() {
                return Err(two_heads(self.span(), sp.text(self.src)));
            }
        } else if self.control_head_next() {
            // `control:`, the control deck: a line of engine calls only (docs/btw/ext-control.md)
            let s = self.span();
            self.bump();
            self.bump();
            let head = s.to(self.prev_span());
            let body = self.control_deck_body(head)?;
            return Ok(Line { when, hold, target: None, control: Some(head), body, guard: None });
        }
        if let Some(body) = self.mixmoves_body(target.map(|(_, sp)| sp))? {
            return Ok(Line { when, hold, target, control: None, body, guard: None });
        }
        if self.word_is("sing") {
            let s = self.sing_call()?;
            return Ok(Line { when, hold, target, control: None, body: Body::Sing(s), guard: None });
        }
        // (F10: `stack(kick ..., snare ...)`, `then(...)` and `silence` over parts start a part line too)
        // (a `stack(` or `then(` with no `deck A:` before it, and no part or `stack` inside, is a call of a def of that name;
        // `silence` is no call)
        let part_inside = matches!(self.peek_n(2), Tok::Word(w) if ROLES.contains(&w.as_str()) || crate::algebra::WORDS.contains(&w.as_str()));
        let role_next = matches!(self.peek(), Tok::Word(w) if ROLES.contains(&w.as_str())) || (self.pexpr_next() && (target.is_some() || !self.sexpr_next() || part_inside));
        if role_next {
            let Some((Target::Deck(_), _)) = target else {
                return Err(Diagnostic::error(self.span(), format!("`{}` is a part, and a part needs a deck", self.peek().describe().trim_matches('`')))
                    .help("deck A: kick x ~ x ~"));
            };
            if let Some((_, ws)) = &when {
                return Err(Diagnostic::error(*ws, "a pattern plays all the time: a time word cannot hold a part")
                    .help("use `<a b>` to change over the bars, a `section` to change the pattern over the song, and a `deck A:` line with no time word"));
            }
            let parts = self.parts()?;
            return Ok(Line { when, hold, target, control: None, body: Body::Parts(parts), guard: None });
        }
        if self.word_is("do") {
            let d = self.do_call(false)?;
            return Ok(Line { when, hold, target, control: None, body: Body::Do(d), guard: None });
        }
        if let Tok::Word(w) = self.peek().clone() {
            if sugar_noun(&w).is_some() && !ACTION_WORDS.contains(&w.as_str()) {
                let d = self.do_call(true)?;
                return Ok(Line { when, hold, target, control: None, body: Body::Do(d), guard: None });
            }
            if self.peek_n(1) == &Tok::LParen && !ACTION_WORDS.contains(&w.as_str()) && !(w == "drive" && target.is_some()) {
                let name = self.word("a def's name")?;
                self.bump();
                let mut args: Vec<Vec<Token>> = Vec::new();
                let mut cur: Vec<Token> = Vec::new();
                let mut depth = 0i32;
                let mut angle = false;
                loop {
                    // `<=` and `>=` of a comparison (`gate(2 <= 3)`) are not an alternation's brackets: `=` never follows one
                    let compares = matches!(self.peek(), Tok::Lt | Tok::Gt) && self.peek_n(1) == &Tok::Eq && self.span().end == self.toks[(self.at + 1).min(self.toks.len() - 1)].span.start;
                    match self.peek() {
                        Tok::Newline | Tok::Eof => {
                            let d = Diagnostic::error(name.span, format!("`{}(` is never closed", name.text));
                            return Err(if angle {
                                d.help("a `<` or `>` in an argument is the bracket of an alternation `<a b>`: write a comparison in the def (`when $x < 2`), not in the call")
                            } else {
                                d.help("the arguments end with `)` on the same line")
                            });
                        }
                        Tok::Lt | Tok::Gt if compares => {}
                        Tok::LParen | Tok::LBrack | Tok::Lt | Tok::LBrace => {
                            angle |= self.peek() == &Tok::Lt;
                            depth += 1;
                        }
                        Tok::RBrack | Tok::Gt | Tok::RBrace => {
                            // (a `>` with no `<` before it, `gate(A, 3 > 2)`, is a comparison in a call as much as a `<` is)
                            angle |= self.peek() == &Tok::Gt;
                            depth -= 1;
                        }
                        Tok::RParen => {
                            if depth == 0 {
                                self.bump();
                                break;
                            }
                            depth -= 1;
                        }
                        Tok::Comma if depth == 0 => {
                            self.bump();
                            args.push(std::mem::take(&mut cur));
                            continue;
                        }
                        _ => {}
                    }
                    cur.push(self.bump());
                }
                if !cur.is_empty() || !args.is_empty() {
                    args.push(cur);
                }
                if let Some((t, tsp)) = target {
                    let _ = t;
                    return Err(Diagnostic::error(tsp, format!("`{}(...)` takes no target before it", name.text)).help("pass the deck as an argument: drop(A, 8)"));
                }
                let span = name.span.to(self.prev_span());
                return Ok(Line { when, hold, target, control: None, body: Body::Invoke(Invocation { name, args, span }), guard: None });
            }
        }
        let action = self.action()?;
        Ok(Line { when, hold, target, control: None, body: Body::Action(action), guard: None })
    }

    /// Is the next thing the control deck's head, `control:`? `control` is no reserved word: it is the head only before a
    /// colon in the place a target takes, so `control(A)` still calls a def of that name and a section, a let or a def may
    /// be named `control` (`after control: ...`).
    pub(crate) fn control_head_next(&self) -> bool {
        self.word_is("control") && self.peek_n(1) == &Tok::Colon
    }

    /// What the control deck holds, after `control:` (docs/btw/ext-control.md): an engine call that takes no target, as the
    /// line with no head holds it: `move`, `swap pairs`, `load pair`, `queue`, `crossfader`, a timed `tempo`, a `duck`, a
    /// `mix`, a `do` and its sugar nouns. A part, a note, a strip's action, a deck's transport, a `sing` or a def's call is an
    /// error that says where it goes.
    fn control_deck_body(&mut self, head: Span) -> R<Body> {
        let Tok::Word(w) = self.peek().clone() else {
            return Err(self
                .expected(&CONTROL_DECK_WORDS, &format!("the control deck holds an engine call, found {}", self.found()))
                .help("control: move echo_out on B   control: swap pairs with filter_exit   control: freeze phrase bars=2"));
        };
        let at = self.span();
        let next = self.peek_n(1).clone();
        let deck_line = "a strip's action goes on its deck or the master: `deck A:`, `deck B:` or `master:`";
        match w.as_str() {
            "deck" | "master" | "control" if matches!(next, Tok::Colon) || (w == "deck" && matches!(next, Tok::Word(_) | Tok::Dollar(_))) => Err(two_heads(at, head.text(self.src).trim_end_matches(':').trim())),
            "every" | "on" | "at" | "after" | "before" | "for" if !matches!(next, Tok::LParen) => Err(Diagnostic::error(at, format!("`{w}` is a time word: it comes before the head"))
                .help(format!("{w} ...: control: ..., as in `at bar 12: control: swap pairs with filter_exit`"))),
            "duck" | "mix" if next != Tok::LParen => Ok(self.mixmoves_body(None)?.expect("a duck or a mix")),
            "do" => Ok(Body::Do(self.do_call(false)?)),
            // `key detect deck=A` is the registry's `key`; `key am` is the program's setting
            "key" if !matches!(&next, Tok::Word(k) if KEY_VERBS.contains(&k.as_str())) => Err(Diagnostic::error(at, "`key` is a setting of the program, structure: it takes no head")
                .help("key am   (a section's own key opens its body; the registry's `key` is `control: key detect deck=A`)")),
            "swing" => Err(Diagnostic::error(at, "`swing` is a setting of the whole program, structure: it takes no head").help("swing 0.1")),
            "sing" => Err(Diagnostic::error(at, "`sing` sings a deck's part, so it is no engine call: it takes no `control:`")
                .help("deck B: sing \"robots on the road\" on melody   (or with no head: the deck that has the role)")),
            "crossfader" | "tempo" | "swap" | "move" | "queue" => Ok(Body::Action(self.action()?)),
            "load" if matches!(&next, Tok::Word(p) if p == "pair") && matches!(self.peek_n(2), Tok::Word(_)) => Ok(Body::Action(self.action()?)),
            "load" | "play" | "stop" | "sync" | "mute" | "unmute" => {
                let ex = if w == "load" { "deck B: load techno:42".to_string() } else { format!("deck A: {w}") };
                Err(Diagnostic::error(at, format!("`{w}` is a deck's: the control deck holds the engine calls that take no deck")).help(format!("{ex}   (a pair of decks: control: load pair dnb:2602)")))
            }
            "filter" | "delay" | "reverb" | "crush" | "drive" | "resonance" | "level" | "low" | "mid" | "high" => {
                let ex = match w.as_str() {
                    "filter" => "master: filter lp 400   or   deck A: filter hp 400",
                    "low" | "mid" | "high" => "deck A: low 0.5",
                    "level" => "deck A: level 0.7   or   master: level 0.8",
                    _ => "master: reverb 0.3   or   deck A: reverb 0.3",
                };
                let ex = ex.replace("reverb", &w);
                Err(Diagnostic::error(at, format!("`{w}` turns a deck's or the master's strip, so it is not a line of the control deck")).help(format!("{deck_line}: {ex}")))
            }
            "macro" => Err(Diagnostic::error(at, "`macro` is designed and not built yet (docs/btw/ext-control.md)")
                .help("an aux macro fires with `control: aux macro=cathedral deck=A bars=2`, a trick with `control: trick on deck=A kind=riser`")),
            _ if ROLES.contains(&w.as_str()) || self.pexpr_next() => Err(Diagnostic::error(at, format!("the control deck holds engine calls, never parts or notes: `{w}` is a part"))
                .help("a part goes on a note deck: `deck A: kick x ~ x ~`")),
            _ if sugar_noun(&w).is_some() && !ACTION_WORDS.contains(&w.as_str()) => Ok(Body::Do(self.do_call(true)?)),
            _ if next == Tok::LParen => Err(Diagnostic::error(at, format!("`{w}(...)` is a def's call: it names its decks itself, and takes no `control:` before it"))
                .help(format!("{w}(...) with no head; inside the def, `control:` goes on its engine lines"))),
            _ => {
                let mut d = self.expected(&CONTROL_DECK_WORDS, &format!("`{w}` is not an engine call the control deck takes"));
                d.found = format!("`{w}`");
                // (the words, not the last entry, which describes the sugar nouns: they are compared one by one)
                let words = CONTROL_DECK_WORDS[..CONTROL_DECK_WORDS.len() - 1].iter().map(|x| x.split(' ').next().unwrap_or(""));
                let near = nearest(&w, words.chain(SUGAR.iter().map(|(s, _)| *s)));
                Err(match near {
                    Some(n) => d.help(format!("did you mean `{n}`?")),
                    None => d.help("control: move echo_out on B   control: swap pairs with filter_exit   control: freeze phrase bars=2"),
                })
            }
        }
    }

    /// `do noun [verb] [text] key=value ... [over N bars]`, or, with `sugar`, the same without `do`.
    pub(crate) fn do_call(&mut self, sugar: bool) -> R<DoCall> {
        if !sugar {
            self.expect_word("do")?;
        }
        let noun = self.word("a noun of the control language (see `describe`)")?;
        let mut verb = None;
        if let Tok::Word(w) = self.peek() {
            if w != "over" && w != "when" && self.peek_n(1) != &Tok::Eq {
                verb = Some(self.word("a verb")?);
            }
        }
        let mut positional = Vec::new();
        while let Tok::Str(text) = self.peek().clone() {
            let span = self.bump().span;
            positional.push((text, span));
        }
        let mut params = Vec::new();
        while matches!(self.peek(), Tok::Word(_)) && self.peek_n(1) == &Tok::Eq {
            let name = self.word("a parameter name")?;
            self.bump();
            let value = self.do_value()?;
            params.push(DoParam { span: name.span.to(value.span()), name, value });
        }
        let over = self.over()?;
        // how a ramp goes (F4, shape.rs)
        let shape = self.shape_opt()?;
        Ok(DoCall { sugar, noun, verb, positional, params, over, shape })
    }

    pub(crate) fn do_value(&mut self) -> R<DoValue> {
        match self.peek().clone() {
            Tok::Str(text) => {
                let span = self.bump().span;
                Ok(DoValue::Str(text, span))
            }
            Tok::Num { .. } | Tok::Minus | Tok::LParen | Tok::Dollar(_) => self.do_number(),
            Tok::Word(w) if (w == "min" || w == "max" || crate::perform::is_input_word(&w)) && self.peek_n(1) == &Tok::LParen => self.do_number(),
            Tok::LBrack => {
                let open = self.bump().span;
                let mut items = Vec::new();
                if self.peek() != &Tok::RBrack {
                    loop {
                        items.push(self.do_value()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                let close = self.expect(Tok::RBrack, "`]`")?;
                Ok(DoValue::List(items, open.to(close)))
            }
            // `lfo tri 1 bar depth=0.2 center=0.3`: a value that keeps moving (F4, shape.rs)
            Tok::Word(w) if w == "lfo" && matches!(self.peek_n(1), Tok::Word(_)) => Ok(DoValue::Lfo(self.lfo_spec(true)?)),
            Tok::Word(_) => Ok(DoValue::Word(self.spec_word()?)),
            _ => Err(self.expected(&["a number", "a word", "a \"string\"", "a [list]"], &format!("expected a value, found {}", self.found()))),
        }
    }

    /// A `do` value that is a number: `0.5`, `2k`, a ratio `1/4` (a word to the registry: its param is text),
    /// arithmetic (`0.25 * 2`), or `a -> b`.
    fn do_number(&mut self) -> R<DoValue> {
        let k = usize::from(self.peek() == &Tok::Minus);
        let ratio = matches!(self.peek_n(k), Tok::Num { .. }) && self.peek_n(k + 1) == &Tok::Slash && matches!(self.peek_n(k + 2), Tok::Num { .. }) && !crate::expr::is_op(self.peek_n(k + 3));
        if ratio {
            let a = self.num_lit("a value")?;
            self.bump();
            let b = self.num_lit("the bottom of the ratio")?;
            let span = a.span.to(b.span);
            return Ok(DoValue::Word(Word { text: format!("{}/{}", crate::print::num(a.value), crate::print::num(b.value)), span }));
        }
        let a = crate::perform::allowing(|| self.num("a value"))?;
        if self.eat(&Tok::Arrow) {
            let b = crate::perform::allowing(|| self.num("the value to move to"))?;
            Ok(DoValue::Ramp(a, b))
        } else {
            Ok(DoValue::Num(a))
        }
    }

    pub(crate) fn deck_word(&mut self) -> R<Deck> {
        match self.peek() {
            Tok::Word(w) if w == "A" => {
                self.bump();
                Ok(Deck::A)
            }
            Tok::Word(w) if w == "B" => {
                self.bump();
                Ok(Deck::B)
            }
            _ => Err(self.expected(&["`A`", "`B`"], &format!("expected a deck, `A` or `B`, found {}", self.found()))),
        }
    }

    pub(crate) fn when(&mut self) -> R<When> {
        if self.word_is("every") {
            self.bump();
            let (d, sp) = self.dur("a length (`4 bars`, `2 beats`)")?;
            if d.is_zero() {
                // (in the unit it was written in: `every 0 beats`, not "0 bars": R-16; the old wording "would never be" kept)
                return Err(Diagnostic::error(sp, format!("every {} would never be", sp.text(self.src).trim())).help("every 1 bars, every 2 beats ..."));
            }
            if d.as_bars() < crate::dur::min_every() {
                return Err(Diagnostic::error(sp, "that is faster than the clock: `every` is at least 1/16 of a beat").help("every 1/16 beat is the finest (that is every 1/64 bar)"));
            }
            let from = if self.word_is("from") {
                self.bump();
                Some(self.pos_from()?.0)
            } else {
                None
            };
            // whole bars all through are the forms the language always had; anything finer is a position
            Ok(match (d.whole_bars(), from.map(Pos::whole_bar)) {
                (Some(n), None) => When::Every(n),
                (Some(n), Some(Some(f))) => When::EveryFrom(n, f),
                _ => When::EveryPos(d, from),
            })
        } else if self.word_is("on") {
            self.bump();
            if self.word_is("next") {
                self.bump();
                if self.word_is("bar") {
                    self.bump();
                    return Ok(When::NextBar);
                }
                self.expect_word("phrase")?;
                if let Tok::Num { .. } = self.peek() {
                    let (n, sp) = self.int("a phrase length: 8, 16 or 32")?;
                    if ![8, 16, 32].contains(&n) {
                        return Err(Diagnostic::error(sp, "a phrase is 8, 16 or 32 bars").found(n.to_string()));
                    }
                    return Ok(When::NextPhrase(n));
                }
                return Ok(When::NextPhrase(PHRASE));
            }
            self.expect_word("phrase")?;
            Ok(When::Phrase)
        } else if self.word_is("after") {
            self.bump();
            Ok(When::After(self.word("a section name")?))
        } else if self.word_is("before") {
            self.bump();
            let name = self.word("a section name")?;
            let (d, _) = self.dur("how long before it (`4 bars`, `2 beats`)")?;
            Ok(match d.whole_bars() {
                Some(n) => When::Before(name, n),
                None => When::BeforeDur(name, d),
            })
        } else {
            self.expect_word("at")?;
            let (p, _) = self.pos()?;
            Ok(match p.whole_bar() {
                Some(n) => When::At(n),
                None => When::AtPos(p),
            })
        }
    }

    // ---- parts and mini-notation

    pub(crate) fn parts(&mut self) -> R<Vec<Part>> {
        let mut parts = self.part_or_expr()?;
        while self.eat(&Tok::Pipe) {
            parts.extend(self.part_or_expr()?);
        }
        Ok(parts)
    }

    pub(crate) fn part(&mut self) -> R<Part> {
        let role = match self.peek().clone() {
            Tok::Word(w) if ROLES.contains(&w.as_str()) => self.word("a role")?,
            _ => {
                return Err(self
                    .expected(&ROLES.map(|r| r), &format!("expected a role, found {}", self.found()))
                    .help("a part is `role pattern`: kick x ~ x ~"))
            }
        };
        let cycle = self.part_cycle(&role)?;
        let pat_start = self.span();
        let pattern = self.seq()?;
        if pattern.terms.is_empty() {
            return Err(Diagnostic::error(pat_start, format!("`{}` has no pattern", role.text))
                .expected(&["`x`", "`~`", "a note like c3", "a degree like 3", "`[`", "`<`", "a generator like e(5,8)"])
                .found(self.found())
                .help(format!("{} x ~ x ~", role.text)));
        }
        let mut params = Vec::new();
        while matches!(self.peek(), Tok::Word(_)) && (self.peek_n(1) == &Tok::Eq || self.word_is("voice-lead")) {
            params.push(self.param()?);
        }
        Ok(Part { span: role.span.to(self.prev_span()), role, cycle, pattern, params, right: false })
    }

    pub(crate) fn param(&mut self) -> R<Param> {
        let name = self.word("a parameter name")?;
        // a bare `voice-lead` means `voice-lead=on` (F5)
        if name.text == "voice-lead" && self.peek() != &Tok::Eq {
            let value = ParamValue::Word(Word { text: "on".into(), span: name.span });
            return Ok(Param { span: name.span, name, value });
        }
        self.expect(Tok::Eq, "`=`")?;
        let value = match self.peek().clone() {
            Tok::Word(w) if !((w == "min" || w == "max" || crate::perform::is_input_word(&w)) && self.peek_n(1) == &Tok::LParen) => ParamValue::Word(self.spec_word()?),
            _ => {
                let a = self.num("a number or a name")?;
                if self.eat(&Tok::DotDot) {
                    let b = self.num("the top of the range")?;
                    ParamValue::Range(a, b)
                } else {
                    ParamValue::Num(a)
                }
            }
        };
        let end = self.prev_span();
        Ok(Param { span: name.span.to(end), name, value })
    }

    pub(crate) fn starts_atom(&self) -> bool {
        match self.peek() {
            // `when` after a line is its guard, never a step
            // a bare `voice-lead` is a part's switch (F5), not a step
            Tok::Word(w) if w == "voice-lead" => false,
            Tok::Word(w) => self.peek_n(1) != &Tok::Eq && w != "when",
            // `(` is a computed degree: `($i + 1)`
            Tok::Num { .. } | Tok::Tilde | Tok::LBrack | Tok::Lt | Tok::LParen => true,
            _ => false,
        }
    }

    pub(crate) fn seq(&mut self) -> R<Seq> {
        let start = self.span();
        let mut terms = Vec::new();
        while self.starts_atom() {
            terms.push(self.term()?);
        }
        let span = if terms.is_empty() { Span::new(start.start, start.start) } else { start.to(self.prev_span()) };
        Ok(Seq { span, terms })
    }

    pub(crate) fn term(&mut self) -> R<Term> {
        let start = self.span();
        let atom = self.atom()?;
        let (accent, slide, marks) = self.term_marks()?;
        let mut fast = None;
        if self.peek() == &Tok::Star {
            self.bump();
            // `x*4`, or `x*(expr)` with the count computed
            let (n, sp) = if self.peek() == &Tok::LParen {
                let (n, sp) = self.paren_int("a repeat count")?;
                (n.clamp(0, i64::from(u32::MAX)) as u32, sp)
            } else {
                self.int_lit("a repeat count")?
            };
            if n == 0 || n > 64 {
                return Err(Diagnostic::error(sp, "a repeat count is 1 to 64"));
            }
            fast = Some(n);
        }
        let term = self.chance_suffix(Term { span: start.to(self.prev_span()), atom, fast, accent, slide, marks })?;
        self.size_suffix(term)
    }

    /// The marks glued to a step, each at most once and in any order: `!` an accent, `^` a slide, `.` a ghost,
    /// `&` a flam, `:N` a ratchet (N strikes in the step, 2 to 8). An accent and a ghost contradict each other.
    pub(crate) fn term_marks(&mut self) -> R<(bool, bool, Marks)> {
        let (mut accent, mut slide) = (false, false);
        let mut marks = Marks::default();
        let mut ghost_at = None;
        let mut accent_at = None;
        loop {
            let adjacent = self.span().start == self.prev_span().end;
            if !adjacent {
                break;
            }
            match self.peek() {
                Tok::Bang if !accent => {
                    accent = true;
                    accent_at = Some(self.span());
                }
                Tok::Caret if !slide => slide = true,
                Tok::Ghost if !marks.ghost => {
                    marks.ghost = true;
                    ghost_at = Some(self.span());
                }
                Tok::Amp if !marks.flam => marks.flam = true,
                // `:3`: only a number glued to the colon makes it a ratchet
                Tok::Colon if marks.ratchet.is_none() && matches!(self.peek_n(1), Tok::Num { .. }) && self.span_n(1).start == self.span().end => {
                    let colon = self.bump().span;
                    let (n, sp) = self.int_lit("a ratchet")?;
                    let (lo, hi) = RATCHET_RANGE;
                    if n < u32::from(lo) || n > u32::from(hi) {
                        return Err(Diagnostic::error(colon.to(sp), format!("a ratchet strikes its step {lo} to {hi} times")).found(format!("`:{n}`")).help("x:3 is three hits inside one sixteenth step"));
                    }
                    marks.ratchet = Some(n as u8);
                    continue;
                }
                Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp => {
                    return Err(Diagnostic::error(self.span(), format!("{} is already on this step", self.peek().describe())).help("each mark (! ^ . & :N) goes on a step at most once"));
                }
                _ => break,
            }
            self.bump();
        }
        if let (Some(a), Some(g)) = (accent_at, ghost_at) {
            return Err(Diagnostic::error(a.to(g), "a step is accented (`!`) or a ghost (`.`), not both").help("x! is a hard hit, x. a soft one"));
        }
        Ok((accent, slide, marks))
    }

    /// `c2~3` (elongate), glued to the term after its marks, repeat and chance: pattern sugar, written out before the
    /// pattern is evaluated (sugar.rs). (There is no replicate mark: `x:3` is the ratchet, a mark of [`Self::term_marks`].)
    fn size_suffix(&mut self, term: Term) -> R<Term> {
        use crate::sugar::{Mark, Sized};
        let m = self.span();
        // a `~` that is not glued to the step before it and to a count after it is a rest of its own: `x ~ 3`, `x~ 3`, `x ~3`
        if self.peek() != &Tok::Tilde || m.start != self.prev_span().end {
            return Ok(term);
        }
        let Tok::Num { value, int } = self.peek_n(1).clone() else {
            return Ok(term);
        };
        let n_at = self.span_n(1);
        if n_at.start != m.end {
            return Ok(term);
        }
        let mark = Mark::Elongate;
        let usage = "c2~3 is c2 ~ ~: the step takes three slots (it and two rests)";
        self.bump();
        self.bump();
        if !int || !(1.0..=64.0).contains(&value) {
            return Err(Diagnostic::error(n_at, format!("the count of `{}` is a whole number from 1 to 64", mark.text())).found(n_at.text(self.src).to_string()).help(usage));
        }
        let at = m.to(n_at);
        if self.span().start == at.end {
            if self.peek() == &Tok::Tilde {
                return Err(Diagnostic::error(self.span(), "a step takes one `~n`").help("one count for all the slots it takes: `c2~5`"));
            }
            // (a mark after the count: `c2~3!`, `x~2:3`, the ratchet's `:N` glued to the count as to a step)
            let ratchet = self.peek() == &Tok::Colon && matches!(self.peek_n(1), Tok::Num { .. }) && self.span_n(1).start == self.span().end;
            if ratchet || matches!(self.peek(), Tok::Star | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp | Tok::Question) {
                return Err(Diagnostic::error(self.span(), format!("{} comes before the `{}`", self.found(), mark.text()))
                    .help("c2!:2*2?0.5~3: the marks, the repeat and the chance first, then the count"));
            }
        }
        let span = term.span.to(at);
        Ok(Term { span, atom: Atom::Sized(Box::new(Sized { term, mark, n: value as u32, at })), fast: None, accent: false, slide: false, marks: Marks::default() })
    }

    pub(crate) fn atom(&mut self) -> R<Atom> {
        let open = self.span();
        self.nested(open, "groups and alternations", false, |p| p.atom_inner())
    }

    fn atom_inner(&mut self) -> R<Atom> {
        match self.peek().clone() {
            Tok::Tilde => {
                self.bump();
                Ok(Atom::Rest)
            }
            Tok::Num { value, int } => {
                let sp = self.bump().span;
                if !int || value < 1.0 {
                    return Err(Diagnostic::error(sp, "a number in a pattern is a scale degree: 1, 2, 3 ...")
                        .found(sp.text(self.src).to_string())
                        .help("use `*` to repeat (x*4) and a note name for a pitch (c3)"));
                }
                Ok(Atom::Degree(value as u32))
            }
            Tok::LParen => {
                // a degree computed: `($i + 1)`
                let (n, sp) = self.paren_int("a scale degree")?;
                if n < 1 {
                    return Err(Diagnostic::error(sp, "a number in a pattern is a scale degree: 1, 2, 3 ...").found(n.to_string()).help("a rest is `~`; a note name gives a pitch (c3)"));
                }
                Ok(Atom::Degree(n.min(i64::from(u32::MAX)) as u32))
            }
            Tok::LBrack => {
                let open = self.bump().span;
                let mut seqs = vec![self.seq()?];
                while self.eat(&Tok::Comma) {
                    seqs.push(self.seq()?);
                }
                if seqs.iter().any(|s| s.terms.is_empty()) {
                    return Err(Diagnostic::error(open.to(self.span()), "a group has nothing in it")
                        .expected(&["a step", "a note", "`~`"])
                        .found(self.found()));
                }
                if self.peek() != &Tok::RBrack {
                    return Err(self.expected(&["`]`", "another term", "`,`"], &format!("expected `]` to close the group opened here, found {}", self.found())).help(format!("the group opens at column {}", crate::diag::line_col(self.src, open.start).1)));
                }
                self.bump();
                Ok(Atom::Group(seqs))
            }
            Tok::Lt => {
                let open = self.bump().span;
                let s = self.seq()?;
                if s.terms.is_empty() {
                    return Err(Diagnostic::error(open.to(self.span()), "an alternation has nothing in it").expected(&["a term to play on the first bar"]).found(self.found()));
                }
                if self.peek() != &Tok::Gt {
                    return Err(self.expected(&["`>`", "another term"], &format!("expected `>` to close the alternation, found {}", self.found())));
                }
                self.bump();
                Ok(Atom::Alt(s.terms))
            }
            Tok::Word(w) => {
                // a seed reference (one word since seed::collapse): the bars of a generated part; a stored program's part, the same
                if crate::seed::is_ref_word(&w) || crate::stored::is_ref_word(&w) {
                    return Ok(Atom::Note(self.word("a seed")?));
                }
                if w == "deg" && self.peek_n(1) == &Tok::LParen {
                    return self.deg_atom();
                }
                // `name(` is a generator or transform call; a step word (`x`, `c2`) before a `(` is a step, and the
                // `(` is a computed degree after it: `c2 ($i + 1)`
                if self.peek_n(1) == &Tok::LParen && !is_step_word(&w) {
                    return Ok(Atom::Call(self.call()?));
                }
                if w == "choose" {
                    return self.choose_atom();
                }
                if w == "maybe" {
                    return Err(self.expected(&["`(`"], &format!("`maybe` takes its pattern and chance in parentheses, found {}", self.peek_n(1).describe())).help("maybe([x ~ x ~], 0.3, 4): each hit kept with chance 0.3, seed 4"));
                }
                let word = self.word("a step")?;
                if w == "x" {
                    Ok(Atom::Hit)
                } else {
                    // a Roman numeral (F5) takes its inversion with it; any other word is a note
                    Ok(self.roman_step(word))
                }
            }
            _ => Err(self
                .expected(&["`x`", "`~`", "a note like c3", "a degree like 3", "`[`", "`<`", "a generator like e(5,8)"], &format!("expected a step, found {}", self.found()))),
        }
    }

    pub(crate) fn call(&mut self) -> R<Call> {
        let open = self.span();
        let spelt = if let Tok::Word(w) = self.peek() { w.clone() } else { String::new() };
        self.nested(open, "calls", false, |p| p.call_inner()).map_err(|d| {
            // an error inside the parentheses of a name that is no generator: the name may be a typo (`dg(1 3 5)`)
            let known = crate::pattern::GENERATORS.contains(&spelt.as_str()) || crate::pattern::TRANSFORMS.contains(&spelt.as_str()) || crate::sugar::CALLS.contains(&spelt.as_str()) || crate::answer::ANSWERS.contains(&spelt.as_str());
            if d.help.is_some() || known || spelt.is_empty() {
                return d;
            }
            match nearest(&spelt, crate::pattern::GENERATORS.iter().chain(crate::pattern::TRANSFORMS.iter()).chain(crate::sugar::CALLS.iter()).chain(crate::answer::ANSWERS.iter()).chain(["deg", "maybe", "choose"].iter()).copied()) {
                Some(n) => d.help(format!("`{spelt}(` is no generator or transform: did you mean `{n}`?")),
                None => d,
            }
        })
    }

    fn call_inner(&mut self) -> R<Call> {
        let name = self.word("a generator name")?;
        // (F13: a performed input is read while the program plays, and a pattern is compiled once)
        if crate::perform::is_input_word(&name.text) && self.peek() == &Tok::LParen {
            return Err(crate::perform::not_here(&name.text, name.span));
        }
        self.expect(Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        if self.peek() != &Tok::RParen {
            loop {
                args.push(self.arg()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        if self.peek() != &Tok::RParen {
            return Err(self.expected(&["`,`", "`)`"], &format!("expected `)` to close {}(...), found {}", name.text, self.found())));
        }
        self.bump();
        Ok(Call { span: name.span.to(self.prev_span()), name, args })
    }

    pub(crate) fn arg(&mut self) -> R<Arg> {
        match self.peek().clone() {
            Tok::Num { .. } | Tok::Minus | Tok::LParen | Tok::Dollar(_) => Ok(Arg::Num(self.num("a number")?)),
            Tok::Word(w) if (w == "min" || w == "max" || crate::perform::is_input_word(&w)) && self.peek_n(1) == &Tok::LParen => Ok(Arg::Num(self.num("a number")?)),
            Tok::LBrack => {
                let start = self.span();
                let mut seq = self.seq()?;
                seq.span = start.to(self.prev_span());
                Ok(Arg::Pat(seq))
            }
            Tok::Word(w) if w == "deg" && self.peek_n(1) == &Tok::LParen => {
                // `transpose(deg(1 3 5), 2)`: a degree group as a pattern (F5)
                let start = self.span();
                let atom = self.deg_atom()?;
                let span = start.to(self.prev_span());
                Ok(Arg::Pat(Seq { span, terms: vec![Term { span, atom, fast: None, accent: false, slide: false, marks: Marks::default() }] }))
            }
            Tok::Word(_) if self.peek_n(1) == &Tok::LParen => Ok(Arg::Call(Box::new(self.call()?))),
            Tok::Word(_) => {
                if self.peek_n(1) == &Tok::Eq {
                    let name = self.word("a name")?;
                    self.bump();
                    let v = match self.peek() {
                        Tok::Word(w) if !((w == "min" || w == "max") && self.peek_n(1) == &Tok::LParen) => Arg::Word(self.word("a value")?),
                        _ => Arg::Num(self.num("a value")?),
                    };
                    return Ok(Arg::Named(name, Box::new(v)));
                }
                let w = self.key_word()?;
                // a Roman numeral may take its inversion with it: `V7/7` (F5)
                Ok(Arg::Word(self.roman_word(w)))
            }
            _ => Err(self.expected(&["a number", "a key or chord like am7", "a name=value"], &format!("expected an argument, found {}", self.found()))),
        }
    }

    // ---- control

    pub(crate) fn action(&mut self) -> R<Action> {
        let Tok::Word(w) = self.peek().clone() else {
            return Err(self.expected(&ACTION_WORDS.map(|a| a), &format!("expected what to do, found {}", self.found())));
        };
        match w.as_str() {
            "swap" => {
                self.bump();
                self.expect_word("pairs")?;
                let with = if self.word_is("with") {
                    self.bump();
                    Some(self.word("a move like echo_out")?)
                } else {
                    None
                };
                Ok(Action::SwapPairs { with })
            }
            "move" => {
                self.bump();
                let kind = self.word("a move like bass_swap")?;
                let deck = if self.word_is("on") {
                    self.bump();
                    Some(self.deck_word()?)
                } else {
                    None
                };
                let over = self.over()?;
                Ok(Action::Move { kind, deck, over })
            }
            "load" => {
                self.bump();
                if self.word_is("pair") && matches!(self.peek_n(1), Tok::Word(_)) {
                    self.bump();
                    let spec = self.spec_word()?;
                    let group = if self.word_is("group") {
                        self.bump();
                        let (n, sp) = self.int("a group number")?;
                        Some(Num::lit(n as f64, true, sp))
                    } else {
                        None
                    };
                    Ok(Action::LoadPair { spec, group })
                } else {
                    Ok(Action::Load { spec: self.spec_word()? })
                }
            }
            "queue" => {
                self.bump();
                let verb = self.word("a queue verb: play, pause, stop, next, loop, jump N, add NAME")?;
                let mut arg = None;
                let mut n = None;
                match verb.text.as_str() {
                    "jump" => {
                        let (v, sp) = self.int("a segment number")?;
                        n = Some(Num::lit(v as f64, true, sp));
                    }
                    "add" => arg = Some(self.word("a saved set's name or CID")?),
                    "loop" if matches!(self.peek(), Tok::Word(_)) => arg = Some(self.word("on or off")?),
                    _ => {}
                }
                Ok(Action::Queue { verb, arg, n })
            }
            "play" | "stop" | "sync" | "mute" | "unmute" => Ok(Action::Transport(self.word("a word")?)),
            "filter" => {
                let name = self.word("filter")?;
                let mode = self.word("`lp`, `hp`, `off` or `open`")?;
                if mode.text == "lp" || mode.text == "hp" {
                    // a value, a ramp (with a shape) or an lfo (F4, shape.rs)
                    self.finish_change(name, Some(mode))
                } else {
                    Ok(Action::Change { name, mode: Some(mode), from: None, to: None, over: None })
                }
            }
            "delay" | "reverb" | "crush" | "resonance" | "drive" | "level" | "low" | "mid" | "high" | "crossfader" => {
                let name = self.word("a name")?;
                self.finish_change(name, None)
            }
            // `at bar 16: tempo 128 -> 140 over 4 bars` (F3, tempo.rs)
            "tempo" => {
                self.bump();
                Ok(Action::Tempo(self.tempo_spec()?))
            }
            _ => {
                // what a line may start with: an action, and the statements (the typo of `assert` or `wire` is read here)
                let mut expected: Vec<&str> = ACTION_WORDS.to_vec();
                expected.extend(["duck", "mix", "sing", "assert", "wire"]);
                let mut d = self.expected(&expected, &format!("`{w}` is not something to do"));
                d.found = format!("`{w}`");
                let near = nearest(&w, STATEMENT_WORDS.iter().chain(["duck", "mix", "sing", "stack", "then", "silence", "drive"].iter()).chain(ACTION_WORDS.iter()).chain(ROLES.iter()).copied());
                // (`dck B: ...` is a deck and `dck B under ...` a duck: the words after tell which)
                let near = match near {
                    Some("deck" | "duck") if matches!(self.peek_n(1), Tok::Word(d) if d == "A" || d == "B") => Some(if self.peek_n(2) == &Tok::Colon { "deck" } else { "duck" }),
                    n => n,
                };
                if w == "emit" {
                    d = d.help("`emit` is a statement of the body of a def that has `out:` ports: `def walker() out: bass { emit c2 }`");
                } else if w == "set" {
                    d = d.help("a set is `set {` with its brace on the same line, then `play` and `mix` lines: set { play NAME  mix 8 bars with crossfade  play OTHER }");
                } else if w == "macro" {
                    d = d.help("`macro` is designed and not built yet (docs/btw/ext-control.md): an aux macro fires with `aux macro=cathedral deck=A bars=2`, a trick with `trick on deck=A kind=riser`");
                } else if let Some(n) = near {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                Err(d)
            }
        }
    }

    /// `value [-> value] [over N bars]`.
    pub(crate) fn change(&mut self, hz: bool) -> R<(Num, Option<Num>, Option<(Dur, Span)>)> {
        let from = crate::perform::allowing(|| self.num("a value"))?;
        // (a filter's cutoff may be written in Hz, `300hz`: the number is the same)
        self.eat_hz(hz);
        let to = if self.eat(&Tok::Arrow) {
            let to = crate::perform::allowing(|| self.num("the value to move to"))?;
            self.eat_hz(hz);
            Some(to)
        } else {
            None
        };
        let over = self.over()?;
        Ok((from, to, over))
    }

    /// The unit `hz` glued to the number just read, when `hz` is allowed here.
    fn eat_hz(&mut self, hz: bool) {
        if hz && matches!(self.peek(), Tok::Unit(u) if u == "hz") && self.span().start == self.prev_span().end {
            self.bump();
        }
    }

    /// `over DUR` (`over 8 bars`, `over 1/2 bar`, `over 2 beats`), when there.
    pub(crate) fn over(&mut self) -> R<Option<(Dur, Span)>> {
        if !self.word_is("over") {
            return Ok(None);
        }
        self.bump();
        let (d, sp) = self.dur("a length (`8 bars`, `2 beats`)")?;
        Ok(Some((d, sp)))
    }
}

/// A word that is a step on its own (`x`, a note such as `c2` or `a#3`) and never the name of a call, so that a `(`
/// after it opens the next step (a computed degree), not an argument list.
fn is_step_word(w: &str) -> bool {
    w == "x" || (!crate::pattern::GENERATORS.contains(&w) && !crate::pattern::TRANSFORMS.contains(&w) && crate::music::parse_note(w).is_some())
}

/// The closest of `words` to `w` (edit distance at most 2), for "did you mean".
pub fn nearest<'a>(w: &str, words: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for c in words {
        let d = edit_distance(w, c);
        if d <= 2 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}
