//! The formatter: a program printed the canonical way (print.rs: one statement a line, one space between tokens, blocks
//! indented two spaces, every number and note spelled one way) with what the canonical form leaves out put back where it was:
//! the `#!` header lines and every comment, and the blank lines between blocks (a run of them is one). The CID is of the
//! canonical text, which has none of these but BeatScript 2's version line, so the formatted program has the CID of the
//! program it was made from; and a formatted program formats to itself. [`format`] also upgrades a BeatScript 1 text (the old
//! spelling, `#! btw 1` or no version line) to `#! beatscript 2`, a new CID by design (version.rs); [`format_as_is`] does not.
//!
//! How a comment finds its place. The canonical printer says, for every line it prints, where in the text the line came from
//! ([`print::PrintedLine`]). A comment on a line of its own goes above the first line printed from after it, indented as
//! that line is (above a `}`, as the lines inside the block are); a comment after code goes at the end of the last line
//! printed from before it, two spaces from the code. A statement the printer keeps whole (a `mix { }`, a `set { }`, a deck
//! line written over several lines) has one place for its own lines, so a comment inside one is put above it, in the order it
//! had. Nothing is lost and nothing is moved past a line it was not beside, so formatting twice is formatting once.
//!
//! A `sample` or `loop` line is kept as it was written: its canonical text depends on what the host knows about the file
//! (assets.rs), which the formatter does not ask.

use crate::ast::StmtKind;
use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::print::{self, LineRole, PrintedLine};

/// `src` formatted and in BeatScript 2 spelling (version.rs): a BeatScript 1 text (`#! btw 1`, or no version line) comes out
/// with `#! beatscript 2` as its version line, which is a new CID for the same program, by design; a BeatScript 2 text formats
/// to its own CID. Its diagnostics when it does not read, spans in `src` as given. [`format_as_is`] formats without the upgrade.
pub fn format(src: &str, env: &Env) -> Result<String, Vec<Diagnostic>> {
    let up = crate::version::upgrade_source(src);
    if up == src {
        return format_as_is(src, env);
    }
    // the text as given reads first, so a diagnostic points into it and not into the upgraded text
    let parsed = crate::parse::parse_with(src, env);
    if parsed.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(parsed.diagnostics);
    }
    format_as_is(&up, env)
}

/// `src` formatted in the version it is in: the formatted program has the CID of the program it was made from (a BeatScript 1
/// text stays BeatScript 1). Its diagnostics when it does not read. Also an error, with a diagnostic that says so, in the case
/// that should not happen: the text it made does not read back as the same program (then it is not used).
pub fn format_as_is(src: &str, env: &Env) -> Result<String, Vec<Diagnostic>> {
    let (text, canon) = layout(src, env)?;
    // the guard: what is made is the program it was made from, or nothing is made
    let again = crate::parse::parse_with(&text, env);
    if again.diagnostics.iter().any(Diagnostic::is_error) || print::canonical(&again.program) != canon {
        return Err(vec![Diagnostic::error(Span::new(0, 0), "the formatted text did not read back as the same program, so it is not used (this is a bug in fmt)")]);
    }
    Ok(text)
}

/// The formatted text and the canonical text of the program it is of.
fn layout(src: &str, env: &Env) -> Result<(String, String), Vec<Diagnostic>> {
    let parsed = crate::parse::parse_with(src, env);
    if parsed.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(parsed.diagnostics);
    }
    let canon = print::canonical(&parsed.program);
    // the lines, in the order of the text
    let mut lines: Vec<PrintedLine> = Vec::new();
    for st in &parsed.program.stmts {
        let mut ls = print::stmt_lines(st, 0);
        if matches!(st.kind, StmtKind::Sample(_)) {
            if let Some(written) = Some(st.span.text(src).trim()).filter(|w| !w.is_empty() && !w.contains('\n')) {
                ls = vec![PrintedLine { text: written.to_string(), src: st.span.start, role: LineRole::Line, leaf: Some(st.span) }];
            }
        }
        lines.extend(ls);
    }
    // (a line printed from text before the one above it, which the printer does not do, is put after it)
    let mut at = 0;
    for l in &mut lines {
        at = at.max(l.src);
        l.src = at;
    }
    let starts: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |off: usize| starts.partition_point(|s| *s <= off) - 1;
    let text_of_line = |n: usize| {
        let end = starts.get(n + 1).copied().unwrap_or(src.len());
        src.get(starts[n]..end).unwrap_or("")
    };
    let blank_before = |off: usize| {
        let n = line_of(off);
        n > 0 && text_of_line(n - 1).trim().is_empty()
    };

    // where a statement kept whole begins and ends in the lines
    let mut leaves: Vec<(Span, usize, usize)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        match (l.leaf, leaves.last_mut()) {
            (Some(sp), Some(last)) if last.0 == sp => last.2 = i,
            (Some(sp), _) => leaves.push((sp, i, i)),
            (None, _) => {}
        }
    }

    // each comment: above a line (with the offset it sorts by), after a line, or after the last line
    let mut above: Vec<Vec<Span>> = vec![Vec::new(); lines.len()];
    let mut after: Vec<Vec<Span>> = vec![Vec::new(); lines.len()];
    let mut tail: Vec<Span> = Vec::new();
    for &c in &parsed.comments {
        let n = line_of(c.start);
        let own_line = src[starts[n]..c.start].trim().is_empty();
        let inside = leaves.iter().find(|(sp, ..)| sp.start < c.start && c.start < sp.end);
        match inside {
            Some(&(sp, first, last)) => {
                // after the code of a line the statement is printed on, or above the statement
                if !own_line && (first == last || line_of(c.start) == line_of(sp.start)) {
                    after[first].push(c);
                } else {
                    above[first].push(c);
                }
            }
            None => {
                let next = lines.partition_point(|l| l.src <= c.start);
                if own_line || next == 0 {
                    match lines.get(next) {
                        Some(_) => above[next].push(c),
                        None => tail.push(c),
                    }
                } else {
                    after[next - 1].push(c);
                }
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    // `blank` says a blank line goes before what is added, if the text had one there
    let push = |out: &mut Vec<String>, text: String, blank: bool, role: LineRole, prev_open: bool| {
        if blank && !out.is_empty() && !prev_open && role != LineRole::Close {
            out.push(String::new());
        }
        out.push(text);
    };
    let mut prev_open = false;
    for (i, l) in lines.iter().enumerate() {
        let indent = l.text.len() - l.text.trim_start().len();
        let inner = if l.role == LineRole::Close { indent + 2 } else { indent };
        // (the comments above a line, in the order of the text)
        above[i].sort_by_key(|c| c.start);
        for c in &above[i] {
            push(&mut out, format!("{}{}", " ".repeat(inner), c.text(src).trim_end()), blank_before(c.start), LineRole::Line, prev_open);
            prev_open = false;
        }
        // (the comments after a line are one comment, as they would be if the text had them so: a `#` inside a comment is text)
        let mut text = l.text.clone();
        for (k, c) in after[i].iter().enumerate() {
            text.push_str(if k == 0 { "  " } else { " " });
            text.push_str(c.text(src).trim_end());
        }
        // (the blank line before a line of a statement kept whole is before its first line only)
        let first_of_leaf = l.leaf.is_none() || i == 0 || lines[i - 1].leaf != l.leaf;
        push(&mut out, text, first_of_leaf && blank_before(l.src), l.role, prev_open);
        prev_open = l.role == LineRole::Open;
    }
    for c in tail {
        push(&mut out, c.text(src).trim_end().to_string(), blank_before(c.start), LineRole::Line, prev_open);
        prev_open = false;
    }
    let mut text = out.join("\n");
    if !out.is_empty() {
        text.push('\n');
    }
    Ok((text, canon))
}
