//! Canonical text: the parse, printed again. One statement per line, one space between tokens,
//! no comments, a deck's parts on one line, numbers in their shortest form. Two texts that differ
//! only in spacing or comments print the same, so they are one program with one CID.

use crate::ast::*;
use crate::lex::{Tok, Token};

pub fn canonical(p: &Program) -> String {
    let mut s = String::new();
    for st in &p.stmts {
        s.push_str(&stmt_block(st, 0));
        s.push('\n');
    }
    s
}

/// A statement, over as many lines as it takes (a block), `depth` levels in.
pub fn stmt_block(s: &Stmt, depth: usize) -> String {
    stmt_lines(s, depth).into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n")
}

/// What a printed line is in its block: one of a statement's lines, the line that opens a block, or the one that closes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRole {
    Line,
    Open,
    Close,
}

/// One line of a printed statement and where in the text it came from, so that what the canonical form leaves out (the
/// comments, the blank lines, `format.rs`) can be put back beside the line it was beside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintedLine {
    /// The line, with its indentation.
    pub text: String,
    /// The byte offset, in the text the program was read from, of what the line prints: its first token (a block's header
    /// is its first word, its `}` the brace).
    pub src: usize,
    pub role: LineRole,
    /// The statement this line is a part of when that statement is printed whole, in however many lines (a `mix { }`, a
    /// `set { }`, an `arrange { }`): the lines of a block that is printed a line at a time (`section`, `def`, `repeat`,
    /// `evolve`) are not, but their parts are.
    pub leaf: Option<crate::diag::Span>,
}

fn pline(text: String, src: usize, role: LineRole, leaf: Option<crate::diag::Span>) -> PrintedLine {
    PrintedLine { text, src, role, leaf }
}

/// A statement as lines, `depth` levels in. [`stmt_block`] is these joined by newlines, so the canonical text and the lines
/// `fmt` works from are one printer.
pub fn stmt_lines(s: &Stmt, depth: usize) -> Vec<PrintedLine> {
    let pad = "  ".repeat(depth);
    let (start, close_at) = (s.span.start, s.span.end.saturating_sub(1).max(s.span.start));
    let mut out = Vec::new();
    match &s.kind {
        StmtKind::Section(sec) => {
            out.push(pline(format!("{pad}section {} {} bars {{", sec.name.text, sec.bars), start, LineRole::Open, None));
            for inner in &sec.body {
                out.extend(stmt_lines(inner, depth + 1));
            }
            out.push(pline(format!("{pad}}}"), close_at, LineRole::Close, None));
        }
        StmtKind::Def(d) => {
            out.push(pline(format!("{pad}def {}({}){} {{", d.name.text, d.params.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join(", "), d.ports.header()), start, LineRole::Open, None));
            body_lines(&d.body, depth + 1, &mut out);
            out.push(pline(format!("{pad}}}"), close_at, LineRole::Close, None));
        }
        StmtKind::Repeat(r) => {
            let mut head = format!("{pad}repeat {}", r.n);
            if let Some(e) = r.every {
                head.push_str(&format!(" every {e} bars"));
            }
            head.push_str(" {");
            out.push(pline(head, start, LineRole::Open, None));
            body_lines(&r.body, depth + 1, &mut out);
            out.push(pline(format!("{pad}}}"), close_at, LineRole::Close, None));
        }
        StmtKind::Evolve(e) => {
            out.push(pline(format!("{pad}evolve every {} bars {{", e.every), start, LineRole::Open, None));
            for r in &e.rules {
                out.push(pline(format!("{pad}  {}", evolve_rule(r)), r.span.start, LineRole::Line, None));
            }
            if let Some((n, sp)) = e.last {
                out.push(pline(format!("{pad}  stop when gen == {n}"), sp.start, LineRole::Line, None));
            }
            out.push(pline(format!("{pad}}}"), close_at, LineRole::Close, None));
        }
        // a `mix` is a block of its own: its items one a line, whatever the time word before it
        StmtKind::Line(l) if matches!(l.body, Body::Mix(_)) => {
            let Body::Mix(m) = &l.body else { unreachable!() };
            let one = line(l);
            // (a `when` guard after the block goes after its `}`)
            let guard = l.guard.as_ref().map_or(String::new(), |g| format!(" when {}", g.expr.canonical()));
            let one = one.strip_suffix(&guard).unwrap_or(&one);
            let head = one.strip_suffix(&crate::mixmoves::print_mix_line(m)).unwrap_or("");
            whole(format!("{}{guard}", crate::mixmoves::mix_block_text(head, m, &pad)), s, &mut out);
        }
        StmtKind::Set(b) => {
            let mut text = format!("{pad}set {{");
            for it in &b.items {
                text.push_str(&format!("\n{pad}  {}", set_item(it)));
            }
            text.push_str(&format!("\n{pad}}}"));
            whole(text, s, &mut out);
        }
        StmtKind::Station(b) => {
            let mut text = format!("{pad}station {{");
            for it in &b.items {
                text.push_str(&format!("\n{pad}  {}", station_item(it)));
            }
            text.push_str(&format!("\n{pad}}}"));
            whole(text, s, &mut out);
        }
        _ => whole(format!("{pad}{}", stmt(s)), s, &mut out),
    }
    out
}

/// A statement printed whole: its lines, each a part of it.
fn whole(text: String, s: &Stmt, out: &mut Vec<PrintedLine>) {
    for l in text.split('\n') {
        out.push(pline(l.to_string(), s.span.start, LineRole::Line, Some(s.span)));
    }
}

/// The lines of a token body, indented `depth` levels, each with where its first token is.
fn body_lines(toks: &[Token], depth: usize, out: &mut Vec<PrintedLine>) {
    for (d, text, src) in token_lines(toks, depth) {
        let role = if text.starts_with('}') { LineRole::Close } else if text.ends_with('{') { LineRole::Open } else { LineRole::Line };
        out.push(pline(format!("{}{text}", "  ".repeat(d)), src, role, None));
    }
}

fn evolve_rule(r: &EvolveRule) -> String {
    let target = match &r.target {
        EvolveTarget::Role(w) => w.text.clone(),
        EvolveTarget::Deck(d) => format!("deck {}", d.name()),
    };
    let ops: Vec<String> = r
        .ops
        .iter()
        .map(|o| {
            let mut s = o.name.text.clone();
            for a in &o.args {
                match a {
                    EvolveArg::Num(n) => s.push_str(&format!(" {}", num(n.value))),
                    EvolveArg::Alt(v) => s.push_str(&format!(" <{}>", v.iter().map(|n| num(n.value)).collect::<Vec<_>>().join(" "))),
                }
            }
            if let Some(sd) = &o.seed {
                s.push_str(&format!(" seed={}", num(sd.value)));
            }
            s
        })
        .collect();
    let mut s = format!("{target}: {}", ops.join(" | "));
    if let Some(c) = &r.when {
        s.push_str(&format!(" when gen{} {} {}", c.modulo.map_or(String::new(), |m| format!(" % {m}")), c.op.text(), c.value));
    }
    s
}

/// The statements of a token body: one a line, nested blocks indented, each with the depth it is at, its text (not indented)
/// and the byte offset of its first token.
fn token_lines(toks: &[Token], depth: usize) -> Vec<(usize, String, usize)> {
    let mut lines: Vec<(usize, String, usize)> = Vec::new();
    let mut cur: Vec<&Token> = Vec::new();
    let mut d = depth;
    let flush = |cur: &mut Vec<&Token>, lines: &mut Vec<(usize, String, usize)>, d: usize| {
        if !cur.is_empty() {
            lines.push((d, source_tokens(cur.iter().copied()), cur[0].span.start));
            cur.clear();
        }
    };
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::Newline => flush(&mut cur, &mut lines, d),
            Tok::LBrace => {
                cur.push(t);
                flush(&mut cur, &mut lines, d);
                d += 1;
            }
            Tok::RBrace => {
                flush(&mut cur, &mut lines, d);
                d = d.saturating_sub(1);
                cur.push(t);
                // a `when` guard after a `mix { }` goes after its `}` (F8 x F2), on the same line
                if !matches!(toks.get(i + 1), Some(Token { tok: Tok::Word(w), .. }) if w == "when") {
                    flush(&mut cur, &mut lines, d);
                }
            }
            _ => cur.push(t),
        }
    }
    flush(&mut cur, &mut lines, d);
    lines
}

/// Tokens as text: one space between words and numbers, none inside brackets or before marks.
pub fn tokens<'a>(toks: impl Iterator<Item = &'a Tok>) -> String {
    tokens_glued(toks, |_| false)
}

/// [`tokens`] of tokens as the text has them (a `let`'s value, a block's body, a call's arguments): a step's `~3` (pattern
/// sugar, sugar.rs) or `:3` (a ratchet) that touched the step and its count there is printed touching, so `let r = c2~3 e2`
/// stays one long note and `let r = c2 ~ 3 e2` stays three steps, as they read, and `x:$n` in a def stays a ratchet.
pub fn source_tokens<'a>(toks: impl IntoIterator<Item = &'a Token>) -> String {
    let toks: Vec<&Token> = toks.into_iter().collect();
    let mut glued = vec![false; toks.len()];
    for i in 1..toks.len().saturating_sub(1) {
        let (step, mark, count) = (toks[i - 1], toks[i], toks[i + 1]);
        // (a `$name` may stand for the step or the count: `x:$n` in a def)
        if matches!(mark.tok, Tok::Colon | Tok::Tilde)
            && (crate::lex::step_end(&step.tok) || matches!(step.tok, Tok::Dollar(_)))
            && matches!(count.tok, Tok::Num { .. } | Tok::Dollar(_))
            && step.span.end == mark.span.start
            && mark.span.end == count.span.start
        {
            glued[i] = true;
            glued[i + 1] = true;
        }
    }
    tokens_glued(toks.iter().map(|t| &t.tok), |i| glued[i])
}

/// [`tokens`], with no space before the tokens `glued` says.
fn tokens_glued<'a>(toks: impl Iterator<Item = &'a Tok>, glued: impl Fn(usize) -> bool) -> String {
    let mut out = String::new();
    let mut prev: Option<&Tok> = None;
    let mut prev2: Option<&Tok> = None;
    // inside `deg( )` a comma that is a stack mark stays glued to what follows it, and a degree's octave mark `1,` is
    // told from it by its space (F5): the parentheses open, flagged when they are a `deg(`'s
    let mut parens: Vec<bool> = Vec::new();
    let mut prev_stack_comma = false;
    for (i, t) in toks.enumerate() {
        let text = match t {
            // a deck's `A` / `B` stay as they are; a pitch word is spelled the canonical way
            Tok::Word(w) => if w == "A" || w == "B" { w.clone() } else { crate::scale::canonical_word(w) },
            Tok::Num { value, .. } => num(*value),
            Tok::Colon => ":".into(),
            Tok::Comma => ",".into(),
            Tok::Pipe => "|".into(),
            Tok::LBrack => "[".into(),
            Tok::RBrack => "]".into(),
            Tok::Lt => "<".into(),
            Tok::Gt => ">".into(),
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::Star => "*".into(),
            Tok::Eq => "=".into(),
            Tok::Tilde => "~".into(),
            Tok::At => "@".into(),
            Tok::Bang => "!".into(),
            Tok::Slash => "/".into(),
            Tok::Percent => "%".into(),
            Tok::Caret => "^".into(),
            Tok::Ghost => ".".into(),
            Tok::Amp => "&".into(),
            Tok::LBrace => "{".into(),
            Tok::RBrace => "}".into(),
            Tok::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")),
            Tok::Dollar(n) => format!("${n}"),
            Tok::DotDot => "..".into(),
            Tok::Arrow => "->".into(),
            Tok::Minus => "-".into(),
            Tok::Plus => "+".into(),
            Tok::Question => "?".into(),
            Tok::Apostrophe => "'".into(),
            Tok::Dot => ".".into(),
            Tok::PlusMinus => "\u{b1}".into(),
            Tok::Unit(u) => u.clone(),
            Tok::OctDown => ",".into(),
            Tok::Newline | Tok::Eof => continue,
        };
        // the innermost parenthesis decides, as in the lexer: `deg((min($i, 3)) 5)` has a plain comma in its `min(`
        let in_deg = parens.last() == Some(&true);
        let stack_comma = matches!(t, Tok::Comma) && in_deg;
        // a ratchet's count stays glued to its colon (`x:3`), which is glued to its step
        let ratchet = matches!(t, Tok::Num { .. }) && matches!(prev, Some(Tok::Colon)) && matches!(prev2, Some(Tok::Word(_) | Tok::Num { .. } | Tok::Dollar(_) | Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp));
        let tight_before = ratchet
            || matches!(t, Tok::Slash | Tok::Bang | Tok::Caret | Tok::Colon | Tok::RParen | Tok::RBrack | Tok::Gt | Tok::Star | Tok::Eq | Tok::DotDot | Tok::Question | Tok::Apostrophe | Tok::Dot | Tok::Unit(_) | Tok::OctDown | Tok::Ghost | Tok::Amp)
            || (matches!(t, Tok::Comma) && !(stack_comma && matches!(prev, Some(Tok::OctDown))))
            || (matches!(t, Tok::LParen) && matches!(prev, Some(Tok::Word(_))));
        let tight_after_prev = matches!(prev, Some(Tok::LParen | Tok::LBrack | Tok::Lt | Tok::Star | Tok::Eq | Tok::DotDot | Tok::Minus | Tok::Slash | Tok::Question | Tok::Dot)) || prev_stack_comma;
        let minus_then = matches!(prev, Some(Tok::Minus));
        if !out.is_empty() && !tight_before && !tight_after_prev && !minus_then && !glued(i) {
            out.push(' ');
        }
        out.push_str(&text);
        match t {
            Tok::LParen => parens.push(matches!(prev, Some(Tok::Word(w)) if w == "deg")),
            Tok::RParen => {
                parens.pop();
            }
            _ => {}
        }
        prev_stack_comma = stack_comma;
        prev2 = prev;
        prev = Some(t);
    }
    out
}

pub fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// A number as a `let` says it: a decimal of at most six places as that decimal, a ratio that is not one as the fraction it is
/// (`1 / 3`, which reads back as the same number: a `let` of arithmetic folds to it), so that the text keeps the exact value that
/// a rounded `f64` would blur (L-08). Anything that is not exactly such a ratio is printed as it always was.
pub fn num_exact(v: f64) -> String {
    ratio_text(v).unwrap_or_else(|| num(v))
}

/// The fraction (`1 / 3`) that `v` is exactly, when it is a ratio that no decimal of at most six places spells; `None` for a
/// whole number, a short decimal and anything that is not exactly such a ratio.
pub fn ratio_text(v: f64) -> Option<String> {
    if v.fract() == 0.0 {
        return None;
    }
    let r = crate::rat::Rat::from_f64(v).filter(|r| r.to_f64() == v)?;
    r.decimal6().is_none().then(|| format!("{} / {}", r.numer(), r.denom()))
}

/// A number of the tree as the canonical text has it: a literal in its shortest spelling, arithmetic as the
/// expression's canonical text (`60 + 6`).
pub fn numn(n: &Num) -> String {
    match n.canon {
        Some(t) => t.to_string(),
        None => num(n.value),
    }
}

/// How a named arrangement ends, as written after its sections (with its leading space; nothing for `Fall`).
pub fn arr_end(e: &crate::ast::ArrEnd) -> String {
    match e {
        crate::ast::ArrEnd::Fall => String::new(),
        crate::ast::ArrEnd::Then(w) => format!(" then {}", w.text),
        crate::ast::ArrEnd::Loop => " loop".to_string(),
        crate::ast::ArrEnd::Times(n, _) => format!(" loop {n}"),
        crate::ast::ArrEnd::End => " end".to_string(),
    }
}

pub fn stmt(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Setting(Setting::Tempo(n)) => format!("tempo {}", numn(n)),
        StmtKind::Setting(Setting::TempoRamp(r)) => format!("tempo {}", crate::tempo::print_ramp(r)),
        StmtKind::Setting(Setting::Swing(n)) => format!("swing {}", numn(n)),
        StmtKind::Setting(Setting::Key(k)) => format!("key {}", crate::music::canonical_pitch_word(&k.text)),
        StmtKind::Line(l) => line(l),
        StmtKind::Let(l) => match l.value.as_slice() {
            // a number that came of arithmetic is said exactly: a ratio that is no short decimal (`let third = 1/3`) is the
            // fraction, never a rounded f64. A number written as a literal is printed as it always was (`0.3333333333333333`).
            [Token { tok: Tok::Num { value, .. }, .. }] if l.ratio => format!("let {} = {}", l.name.text, num_exact(*value)),
            [Token { tok: Tok::Num { value, .. }, .. }] => format!("let {} = {}", l.name.text, num(*value)),
            _ => format!("let {} = {}", l.name.text, source_tokens(&l.value)),
        },
        StmtKind::End(EndWhen::Song) => "end".into(),
        StmtKind::End(EndWhen::At(n)) => format!("end at bar {n}"),
        StmtKind::End(EndWhen::After(w)) => format!("end after {}", w.text),
        StmtKind::Loop => "loop".into(),
        StmtKind::Arrange(names) => format!("arrange {}", names.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join(" ")),
        StmtKind::ArrangeExpr(items) => crate::algebra::arrange_text(items),
        StmtKind::ArrangeNamed(d) => format!("arrange {}: {}{}", d.name.text, d.items.iter().map(crate::algebra::SExpr::canonical).collect::<Vec<_>>().join(" "), arr_end(&d.ends)),
        StmtKind::Start(w) => format!("start {}", w.text),
        // (the default bar line, the next phrase of 8 bars, says nothing)
        StmtKind::Launch(l) => format!("launch {} on pad({}){}", l.scene.text, l.pad, match l.quantum {
            crate::ast::LaunchQuantum::Bar => " next bar".to_string(),
            crate::ast::LaunchQuantum::Phrase(n) if n == crate::runtime::PHRASE => String::new(),
            crate::ast::LaunchQuantum::Phrase(n) => format!(" next phrase {n}"),
        }),
        StmtKind::ArrangeBlock(b) => crate::mutate::print_block(b),
        StmtKind::LetSection(l) => format!("let {} = {}", l.name.text, l.expr.canonical()),
        StmtKind::Import(i) => format!("import {}", import_word(&i.what.text)),
        StmtKind::UseRestricted => "use restricted".into(),
        StmtKind::Wire(w) => w.text(),
        StmtKind::Assert(a) => a.text(),
        StmtKind::Seed(s) => match &s.section {
            Some(n) => format!("section {} from {}", n.text, s.what.text),
            None => format!("arrange from {}", s.what.text),
        },
        StmtKind::Sample(d) => {
            let else_ = d.otherwise.as_ref().map_or(String::new(), |v| format!(" else {}", v.text));
            match d.kind {
                crate::ast::SampleKind::Loop => format!("loop {} as {}{else_}", quoted(&d.src.text), d.name.text),
                crate::ast::SampleKind::Sample => {
                    let tag = d.license.as_ref().and_then(|w| crate::assets::AssetRights::parse(&w.text)).unwrap_or(crate::assets::AssetRights::RestrictedPrivate);
                    format!("sample {} from {} license {}{else_}", d.name.text, quoted(&d.src.text), tag.id())
                }
            }
        }
        StmtKind::Def(_) | StmtKind::Repeat(_) | StmtKind::Section(_) | StmtKind::Evolve(_) | StmtKind::Set(_) | StmtKind::Station(_) => stmt_block(s, 0),
    }
}

/// A set's line: `play CID until 20m`, `mix 8 bars with crossfade, tempo 140 -> 130`.
fn set_item(it: &SetItem) -> String {
    match it {
        SetItem::Play(p) => {
            // (a generator call is written bare: it is a call, not a name in quotes)
            let what = if crate::gen::is_call_word(&p.what.text) { p.what.text.clone() } else { import_word(&p.what.text) };
            let mut s = format!("play {what}");
            if let Some((u, _)) = &p.until {
                s.push_str(&format!(" until {}", until_word(u)));
            }
            s
        }
        SetItem::Next(n) => {
            let mut s = format!("next {}", step_word(n.step));
            if let Some((u, _)) = &n.until {
                s.push_str(&format!(" until {}", until_word(u)));
            }
            s
        }
        SetItem::Mix(m) => {
            let mut s = format!("mix {} bars with {}", m.bars, m.with.text);
            if let Some((a, b)) = m.tempo {
                s.push_str(&format!(", tempo {} -> {}", num(a), num(b)));
            }
            s
        }
    }
}

/// A station's line: `from gen@2 techno seed=42`, `next seed=~`, `forms seeded rondo`.
fn station_item(it: &StationItem) -> String {
    match it {
        StationItem::From(w, _) => format!("from {}", w.text),
        StationItem::Next(step, _) => format!("next {}", step_word(*step)),
        StationItem::Forms(names, _) => format!("forms {}", names.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")),
    }
}

/// `seed=+1`, `seed=~`.
pub fn step_word(step: SeedStep) -> String {
    match step {
        SeedStep::By(n) => format!("seed=+{n}"),
        SeedStep::Hash => "seed=~".to_string(),
    }
}

/// A time the shortest way that is exact: whole hours, whole minutes, else seconds.
pub fn until_word(u: &UntilWhen) -> String {
    match u {
        UntilWhen::Seconds(s) if *s > 0.0 && s % 3600.0 == 0.0 => format!("{}h", num(s / 3600.0)),
        UntilWhen::Seconds(s) if *s > 0.0 && s % 60.0 == 0.0 => format!("{}m", num(s / 60.0)),
        UntilWhen::Seconds(s) => format!("{}s", num(*s)),
        UntilWhen::Bar(n) => format!("bar {n}"),
        UntilWhen::Section(n) => n.clone(),
        UntilWhen::After(n) => format!("after {n}"),
    }
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn import_word(w: &str) -> String {
    if w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        w.to_string()
    } else {
        format!("\"{w}\"")
    }
}

pub fn line(l: &Line) -> String {
    let mut s = String::new();
    match (&l.when, &l.hold) {
        (Some((When::Now, _)), Some((d, _))) => s.push_str(&format!("for {}: ", d.canonical())),
        (Some((w, _)), hold) => {
            s.push_str(&when(w));
            if let Some((d, _)) = hold {
                s.push_str(&format!(" for {}", d.canonical()));
            }
            s.push_str(": ");
        }
        (None, _) => {}
    }
    if let Some((t, _)) = &l.target {
        s.push_str(&target(t));
        s.push_str(": ");
    }
    // the control deck's head (docs/btw/ext-control.md): its own spelling, so `control: move echo_out on B` and
    // `move echo_out on B` are two texts (and two CIDs) of one lowering
    if l.control.is_some() {
        s.push_str("control: ");
    }
    match &l.body {
        Body::Parts(ps) => s.push_str(&ps.iter().map(part).collect::<Vec<_>>().join(" | ")),
        Body::Action(a) => s.push_str(&action(a)),
        Body::Do(d) => s.push_str(&do_call(d)),
        Body::Sing(c) => s.push_str(&crate::sing::print_call(c)),
        Body::Invoke(i) => {
            s.push_str(&format!("{}({})", i.name.text, i.args.iter().map(|a| source_tokens(a)).collect::<Vec<_>>().join(", ")));
        }
        Body::Duck(d) => s.push_str(&crate::mixmoves::print_duck(d)),
        Body::Mix(m) => s.push_str(&crate::mixmoves::print_mix_line(m)),
    }
    if let Some(g) = &l.guard {
        s.push_str(" when ");
        s.push_str(&g.expr.canonical());
    }
    s
}

pub fn do_call(d: &DoCall) -> String {
    let mut s = if d.sugar { d.noun.text.clone() } else { format!("do {}", d.noun.text) };
    if let Some(v) = &d.verb {
        s.push(' ');
        s.push_str(&v.text);
    }
    for (p, _) in &d.positional {
        s.push_str(&format!(" \"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")));
    }
    let mut params: Vec<&DoParam> = d.params.iter().collect();
    params.sort_by(|a, b| a.name.text.cmp(&b.name.text));
    for p in params {
        s.push_str(&format!(" {}={}", p.name.text, do_value(&p.value)));
    }
    if let Some((o, _)) = &d.over {
        s.push_str(&format!(" over {}", o.canonical()));
    }
    if let Some(w) = d.shape.and_then(|(sh, _)| sh.canonical()) {
        s.push(' ');
        s.push_str(&w);
    }
    s
}

/// A `do` value's number: arithmetic that prints as a bare ratio (`(1/4)` written in parentheses) keeps them,
/// since `1 / 4` with no parentheses is read as the registry's ratio word.
fn do_num(n: &Num) -> String {
    let t = numn(n);
    let ratio = n.canon.is_some() && t.split_once(" / ").is_some_and(|(a, b)| a.trim_start_matches('-').chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()));
    if ratio {
        format!("({t})")
    } else {
        t
    }
}

fn do_value(v: &DoValue) -> String {
    match v {
        DoValue::Num(n) => do_num(n),
        DoValue::Ramp(a, b) => format!("{} -> {}", do_num(a), do_num(b)),
        DoValue::Word(w) => w.text.clone(),
        DoValue::Str(t, _) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")),
        DoValue::List(items, _) => format!("[{}]", items.iter().map(do_value).collect::<Vec<_>>().join(", ")),
        DoValue::Lfo(l) => crate::shape::print_lfo(l),
    }
}

pub fn when(w: &When) -> String {
    match w {
        When::Every(n) => format!("every {n} bars"),
        When::Phrase => "on phrase".into(),
        When::At(n) => format!("at bar {n}"),
        When::EveryFrom(n, f) => format!("every {n} bars from {f}"),
        When::NextBar => "on next bar".into(),
        When::NextPhrase(n) => if *n == 8 { "on next phrase".into() } else { format!("on next phrase {n}") },
        When::After(w) => format!("after {}", w.text),
        When::Before(w, n) => format!("before {} {n} bars", w.text),
        When::Now => String::new(),
        When::EveryPos(d, None) => format!("every {}", d.canonical()),
        When::EveryPos(d, Some(p)) => format!("every {} from {}", d.canonical(), p.canonical_from()),
        When::AtPos(p) => format!("at {}", p.canonical()),
        When::BeforeDur(w, d) => format!("before {} {}", w.text, d.canonical()),
    }
}

pub fn target(t: &Target) -> String {
    match t {
        Target::Deck(d) => format!("deck {}", d.name()),
        Target::Master => "master".into(),
    }
}

/// A part's role and pattern, with no params: what a chance part's implicit seed is of.
pub fn part_pattern(p: &Part) -> String {
    format!("{}{} {}", p.role.text, crate::meter::cycle_suffix(p), seq(&p.pattern))
}

pub fn part(p: &Part) -> String {
    let mut s = format!("{}{} {}", p.role.text, crate::meter::cycle_suffix(p), seq(&p.pattern));
    let mut params: Vec<&Param> = p.params.iter().collect();
    params.sort_by(|a, b| a.name.text.cmp(&b.name.text));
    for pr in params {
        s.push(' ');
        s.push_str(&param(pr));
    }
    s
}

pub(crate) fn param(p: &Param) -> String {
    // an arithmetic end of a range is parenthesised, so `(60 + 12)..100` reads back as it was written
    let end = |n: &Num| match n.canon {
        // (a plain number it folded to needs none: `(60 + 12)..100` is `72..100`)
        Some(t) if t.chars().all(|c| c.is_ascii_digit() || c == '.') => t.to_string(),
        Some(t) => format!("({t})"),
        None => num(n.value),
    };
    let v = match &p.value {
        ParamValue::Num(n) => numn(n),
        ParamValue::Range(a, b) => format!("{}..{}", end(a), end(b)),
        ParamValue::Word(w) => w.text.clone(),
    };
    format!("{}={}", p.name.text, v)
}

pub fn seq(s: &Seq) -> String {
    s.terms.iter().map(term).collect::<Vec<_>>().join(" ")
}

pub(crate) fn term(t: &Term) -> String {
    let a = match &t.atom {
        Atom::Hit => "x".to_string(),
        Atom::Rest => "~".to_string(),
        Atom::Note(w) => crate::scale::canonical_word(&w.text),
        Atom::Degree(d) => d.to_string(),
        Atom::Group(seqs) => format!("[{}]", seqs.iter().map(seq).collect::<Vec<_>>().join(", ")),
        Atom::Alt(ts) => format!("<{}>", ts.iter().map(term).collect::<Vec<_>>().join(" ")),
        Atom::Call(c) => call(c),
        Atom::Chance(c) => crate::chance::print_chance(c),
        Atom::Choose(c) => crate::chance::print_choose(c),
        Atom::Deg(seqs) => crate::scale::print_deg(seqs),
        Atom::ScaleDeg(d) => d.text(),
        Atom::Sized(s) => crate::sugar::print_sized(s),
    };
    let a = format!("{a}{}{}{}", if t.accent { "!" } else { "" }, if t.slide { "^" } else { "" }, t.marks.text());
    match t.fast {
        Some(k) => format!("{a}*{k}"),
        None => a,
    }
}

pub(crate) fn call(c: &Call) -> String {
    let name = if c.name.text == "euclid" { "e" } else { c.name.text.as_str() };
    format!("{}({})", name, c.args.iter().map(arg).collect::<Vec<_>>().join(", "))
}

fn arg(a: &Arg) -> String {
    match a {
        Arg::Num(n) => numn(n),
        Arg::Word(w) => crate::scale::canonical_word(&w.text),
        Arg::Named(n, v) => format!("{}={}", n.text, arg(v)),
        Arg::Pat(sq) => seq(sq),
        Arg::Call(c) => call(c),
    }
}

pub fn action(a: &Action) -> String {
    match a {
        Action::SwapPairs { with } => match with {
            Some(w) => format!("swap pairs with {}", w.text),
            None => "swap pairs".into(),
        },
        Action::Move { kind, deck, over } => {
            let mut s = format!("move {}", kind.text);
            if let Some(d) = deck {
                s.push_str(&format!(" on {}", d.name()));
            }
            if let Some((o, _)) = over {
                s.push_str(&format!(" over {}", o.canonical()));
            }
            s
        }
        Action::LoadPair { spec, group } => match group {
            Some(g) => format!("load pair {} group {}", spec.text, numn(g)),
            None => format!("load pair {}", spec.text),
        },
        Action::Load { spec } => format!("load {}", spec.text),
        Action::Queue { verb, arg, n } => {
            let mut s = format!("queue {}", verb.text);
            if let Some(a) = arg {
                s.push_str(&format!(" {}", a.text));
            }
            if let Some(n) = n {
                s.push_str(&format!(" {}", numn(n)));
            }
            s
        }
        Action::Transport(w) => w.text.clone(),
        Action::Change { name, mode, from, to, over } => {
            let mut s = name.text.clone();
            if let Some(m) = mode {
                s.push_str(&format!(" {}", m.text));
            }
            if let Some(f) = from {
                s.push_str(&format!(" {}", numn(f)));
            }
            if let Some(t) = to {
                s.push_str(&format!(" -> {}", numn(t)));
            }
            if let Some((o, _)) = over {
                s.push_str(&format!(" over {}", o.canonical()));
            }
            s
        }
        Action::Shaped { name, mode, motion } => crate::shape::print_shaped(name, mode.as_ref(), motion),
        Action::Tempo(t) => format!("tempo {}", crate::tempo::print_spec(t)),
    }
}
