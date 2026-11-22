//! Refactors: extract a `let`, inline a `let`.
//!
//! A refactor must not change what the program plays. The canonical text keeps `let`s, so the program's CID changes with
//! any of these by construction; what must not change is the sound: every net the program compiles to (its decks', its
//! sections', its song's). Each refactor is offered cheaply (the list of code actions comes on every selection change) and
//! made and proved in `codeAction/resolve`: the new text is checked and its nets compared with the old text's, and an edit
//! that would change a net is refused with the reason. (Extracting part of a pattern, `x ~` of `x ~ x ~`, would: a `let`
//! stands in one step.)
//!
//! Matt's policy holds: nothing is deleted. Inlining comments the `let` out (`# let riff = ...  (inlined)`), it does not
//! remove it.
//!
//! Collapse a repeated arrangement: `arrange a b a b a b outro` is `arrange { a b } loop 3 then outro end` (an arrange block
//! that loops N passes, then its coda), proved the same way.

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::Span;
use djlive::lex::Tok;

use crate::analysis::{full_check, Analysis};
use crate::fix::workspace_edit;
use crate::json::{int, obj, s};
use crate::symbols::Kind;
use crate::text::Encoding;

/// One refactor, made: a title and the edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
    pub title: String,
    pub kind: &'static str,
    pub edits: Vec<(Span, String)>,
}

/// A name that is free in the program and is no word of the language: `pat1`, `pat2` ... (`n1` ... for a number).
pub fn fresh_name(a: &Analysis, number: bool) -> String {
    let stem = if number { "n" } else { "pat" };
    (1..).map(|k| format!("{stem}{k}")).find(|n| a.symbols.named(n).next().is_none() && djlive::parse::reserved(n).is_none()).unwrap()
}

/// The tokens a selection covers, widened to whole tokens: (first, last) indices.
fn covered(a: &Analysis, sel: Span) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = None;
    for (i, t) in a.tokens.iter().enumerate() {
        if matches!(t.tok, Tok::Eof) {
            break;
        }
        if t.span.end > sel.start && t.span.start < sel.end {
            first.get_or_insert(i);
            last = Some(i);
        }
    }
    Some((first?, last?))
}

/// Extract the selection into a `let` before the top-level statement it is in.
pub fn extract_let(a: &Analysis, sel: Span) -> Option<Made> {
    if sel.is_empty() {
        return None;
    }
    let (f, l) = covered(a, sel)?;
    // a piece of one line: no line break, block, part separator, colon or `=` in it
    if a.tokens[f..=l].iter().any(|t| matches!(t.tok, Tok::Newline | Tok::LBrace | Tok::RBrace | Tok::Pipe | Tok::Colon | Tok::Eq | Tok::Comma)) {
        return None;
    }
    // brackets balanced
    let mut depth = 0i32;
    for t in &a.tokens[f..=l] {
        match t.tok {
            Tok::LBrack | Tok::LParen | Tok::Lt => depth += 1,
            Tok::RBrack | Tok::RParen | Tok::Gt => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
    }
    if depth != 0 {
        return None;
    }
    let span = Span::new(a.tokens[f].span.start, a.tokens[l].span.end);
    // the statement it is in: a line (a pattern or a control), not a header
    let st = a.symbols.statement_at(span.start)?;
    if matches!(st.head.as_str(), "let" | "def" | "section" | "arrange" | "evolve" | "repeat" | "wire" | "import" | "sample" | "end" | "set" | "use" | "assert" | "launch") {
        return None;
    }
    let text = span.text(&a.text).to_string();
    let number = f == l && matches!(a.tokens[f].tok, Tok::Num { .. });
    let name = fresh_name(a, number);
    // before the outermost statement, at the start of its line
    let top = a.symbols.statements.iter().filter(|s| s.depth == 0 && s.span.start <= span.start && span.start <= s.span.end).map(|s| s.span.start).next().unwrap_or(st.span.start);
    let line_start = a.index.line_start(a.index.line_of(top));
    Some(Made { title: format!("Extract `{text}` into `let {name}`"), kind: "refactor.extract", edits: vec![(Span::new(line_start, line_start), format!("let {name} = {text}\n")), (span, name)] })
}

/// Inline the `let` at the cursor into every use, and comment the `let` out.
pub fn inline_let(a: &Analysis, at: usize) -> Option<Made> {
    let r = a.symbols.ref_at(at)?;
    let sym = &a.symbols.symbols[r.symbol];
    if sym.kind != Kind::Let {
        return None;
    }
    let uses: Vec<Span> = a.symbols.refs_of(r.symbol).filter(|u| !u.is_def && !u.dollar).map(|u| u.span).collect();
    if uses.is_empty() {
        return None;
    }
    // a let of a number is put in place as the number it came to; a pattern of several terms as one step `[...]`
    let value = sym.detail.trim().to_string();
    let replacement = match a.scalar_lets.iter().find(|l| l.at == sym.name_span) {
        Some(l) => match djlive::print::ratio_text(l.value) {
            Some(r) if l.text.contains('/') => r,
            _ => djlive::print::num(l.value),
        },
        None => {
            let toks = djlive::lex::lex(&value).tokens;
            let single = toks.len() <= 2 || value.starts_with('[') && value.ends_with(']') && !value[1..value.len() - 1].contains(']');
            if single {
                value.clone()
            } else {
                format!("[{value}]")
            }
        }
    };
    let line = a.index.line_of(sym.full.start);
    let (ls, le) = (a.index.line_start(line), a.index.line_end(&a.text, line));
    let n = uses.len();
    let note = format!("  (inlined into its {} use{})", n, if n == 1 { "" } else { "s" });
    let mut edits: Vec<(Span, String)> = vec![(Span::new(ls, ls), "# ".to_string()), (Span::new(le, le), note)];
    edits.extend(uses.into_iter().map(|u| (u, replacement.clone())));
    edits.sort_by_key(|(s, _)| (s.start, s.end));
    Some(Made { title: format!("Inline `let {}` ({} use{})", sym.name, n, if n == 1 { "" } else { "s" }), kind: "refactor.inline", edits })
}

/// The text with the edits made.
pub fn apply(text: &str, edits: &[(Span, String)]) -> String {
    let mut v: Vec<&(Span, String)> = edits.iter().collect();
    v.sort_by_key(|(s, _)| std::cmp::Reverse((s.start, s.end)));
    let mut t = text.to_string();
    for (sp, new) in v {
        t.replace_range(sp.start..sp.end, new);
    }
    t
}

/// Every net a compiled program plays, sorted: what a refactor must keep.
fn nets(c: &djlive::Compiled) -> Vec<String> {
    let mut v: Vec<String> = c.net_cids().into_values().collect();
    if let Ok(song) = c.song_nets() {
        v.extend(song.into_values().map(|ir| ir.net_cid()));
    }
    v.sort();
    v
}

/// Does the refactored text play the same nets as the original? Err: why not.
pub fn verify(a: &Analysis, made: &Made) -> Result<(), String> {
    let Some(before) = a.check().compiled.as_ref() else { return Err("the program has errors: fix them first, so the refactor can be proved".into()) };
    let after_text = apply(&a.text, &made.edits);
    let after = full_check(&after_text);
    let Some(after_c) = after.compiled.as_ref() else {
        let why = after.diagnostics.iter().find(|d| d.is_error()).map_or(String::new(), |d| format!(": {}", d.message));
        return Err(format!("the refactored program would not check{why}"));
    };
    if nets(before) != nets(after_c) {
        return Err("the refactored program would play differently (its nets change): a `let` stands in one step, so a part of a pattern cannot be extracted".into());
    }
    Ok(())
}

/// The refactors offered for a range: unresolved code actions (`data` says which; `codeAction/resolve` makes and proves them).
pub fn actions(a: &Analysis, uri: &str, range: Span, only: &[String]) -> Vec<JsonValue> {
    let wants = |kind: &str| only.is_empty() || only.iter().any(|o| kind == o || kind.starts_with(&format!("{o}.")));
    let mut out = Vec::new();
    let mut offer = |m: Made, which: &str| {
        out.push(obj([
            ("title", s(m.title)),
            ("kind", s(m.kind)),
            ("data", obj([("refactor", s(which)), ("uri", s(uri)), ("start", int(range.start)), ("end", int(range.end)), ("length", int(a.text.len()))])),
        ]));
    };
    if wants("refactor.extract") {
        if let Some(m) = extract_let(a, range) {
            offer(m, "extract-let");
        }
    }
    if wants("refactor.inline") {
        if let Some(m) = inline_let(a, range.start) {
            offer(m, "inline-let");
        }
    }
    if wants("refactor.rewrite") {
        if let Some(m) = collapse_arrange(a, range.start) {
            offer(m, "collapse-arrange");
        }
    }
    out
}

/// `arrange a b a b a b [coda]`: the run of a unit repeated from the start, as an arrange block's `loop N then coda end`.
pub fn collapse_arrange(a: &Analysis, at: usize) -> Option<Made> {
    let st = a.symbols.statement_at(at)?;
    if st.head != "arrange" || st.depth != 0 {
        return None;
    }
    let toks = &a.tokens[st.first + 1..st.end];
    // only a plain arrange of section names (no block, no name, no composition)
    let mut names = Vec::new();
    for t in toks {
        match &t.tok {
            Tok::Word(w) if !matches!(w.as_str(), "stack" | "then" | "silence") => names.push(w.clone()),
            _ => return None,
        }
    }
    // the shortest unit whose repeats make the longest run from the start (two at least); the rest is the coda
    let mut best: Option<(usize, usize)> = None;
    for u in 1..=names.len() / 2 {
        let mut k = 1;
        while (k + 1) * u <= names.len() && names[k * u..(k + 1) * u] == names[..u] {
            k += 1;
        }
        if k >= 2 && best.is_none_or(|(bu, bk)| k * u > bu * bk) {
            best = Some((u, k));
        }
    }
    let (u, k) = best?;
    let unit = names[..u].join(" ");
    let coda = names[u * k..].join(" ");
    let tail = if coda.is_empty() { format!("loop {k} then end") } else { format!("loop {k} then {coda} end") };
    let new = format!("arrange {{\n  {unit}\n}} {tail}");
    Some(Made { title: format!("Collapse into `arrange {{ {unit} }} {tail}`"), kind: "refactor.rewrite", edits: vec![(st.span, new)] })
}

/// `codeAction/resolve` for a refactor: the action with its edit, once the edit is proved; Err with the reason otherwise.
pub fn resolve(a: &Analysis, action: &JsonValue, enc: Encoding) -> Result<JsonValue, String> {
    let data = action.get("data").ok_or("the action has no data")?;
    let which = data.get("refactor").and_then(JsonValue::as_str).ok_or("not a refactor")?;
    let uri = data.get("uri").and_then(JsonValue::as_str).unwrap_or("");
    let (start, end) = (data.get("start").and_then(JsonValue::as_f64).unwrap_or(0.0) as usize, data.get("end").and_then(JsonValue::as_f64).unwrap_or(0.0) as usize);
    if data.get("length").and_then(JsonValue::as_f64).map(|l| l as usize) != Some(a.text.len()) {
        return Err("the document changed since the action was offered".into());
    }
    let made = match which {
        "extract-let" => extract_let(a, Span::new(start, end)),
        "inline-let" => inline_let(a, start),
        "collapse-arrange" => collapse_arrange(a, start),
        _ => None,
    }
    .ok_or("the refactor no longer applies here")?;
    verify(a, &made)?;
    let mut o = match action {
        JsonValue::Object(o) => o.clone(),
        _ => JsonObject::new(),
    };
    o.set("edit", workspace_edit(a, uri, &made.edits, enc));
    Ok(JsonValue::Object(o))
}

/// The refactors with their edits, unproved (for hosts that take edits in the list).
pub fn made_for(a: &Analysis, range: Span) -> Vec<Made> {
    extract_let(a, range).into_iter().chain(inline_let(a, range.start)).chain(collapse_arrange(a, range.start)).collect()
}
