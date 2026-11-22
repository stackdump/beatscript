//! Doc comments: the `#` lines written directly above a `section`, `let`, `def` or `arrange` line are that item's doc
//! comment, and the editor's hover, check's `sections` and a shared program's JSON-LD say it.
//!
//! The rule (a taste call of Matt's, 2026-11-22, after "Hall of the Rave King" put a `# name: what it does` line above
//! every section):
//!
//! - the contiguous comment-only lines touching the item's line, with no blank line between, are its doc comment; a blank
//!   line, a line of code or a `#!` header line ends the run (a `#!` line is never a doc comment);
//! - each line is read with its `#` marks and the spaces around them off, and the lines are joined with a space (one
//!   paragraph);
//! - a leading `<name>:` that repeats the item's own name is dropped (`# intro: kick alone` above `section intro` is
//!   "kick alone");
//! - the hover trims it to about [`HOVER_MAX`] characters ([`hover_text`]); check and the JSON-LD carry it whole.
//!
//! Comments stay comments: none of this is in the canonical text, so the CID of a program is the same with its doc
//! comments, without them, or with other ones. Pure: the text's own lexer tokens and comment spans, nothing compiled.

use crate::diag::Span;
use crate::lex::{lex, Tok};

/// About how many characters of a doc comment a hover shows.
pub const HOVER_MAX: usize = 280;

/// One item's doc comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocComment {
    /// `section`, `let`, `def` or `arrange`.
    pub kind: &'static str,
    /// The item's name: the section's, the let's, the def's, a named arrangement's (`arrange set: ...`), or `arrange`
    /// for the program's one arrange.
    pub name: String,
    /// The doc comment's text, whole.
    pub doc: String,
    /// The item's line, from its first token to the end of the line.
    pub item: Span,
    /// The comment lines, from the first `#` to the end of the last.
    pub comment: Span,
}

/// Every doc comment in `text`, in the order the items are written.
pub fn doc_comments(text: &str) -> Vec<DocComment> {
    let lexed = lex(text);
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |at: usize| starts.partition_point(|&s| s <= at) - 1;
    let line_end = |l: usize| starts.get(l + 1).map_or(text.len(), |&s| s - 1);
    // the lines that are a comment and nothing else, by line: the comment's span
    let mut only_comment: Vec<Option<Span>> = vec![None; starts.len()];
    for c in &lexed.comments {
        let l = line_of(c.start);
        if text[starts[l]..c.start].trim().is_empty() {
            only_comment[l] = Some(*c);
        }
    }
    // the first token of each line that has one
    let toks = &lexed.tokens;
    let mut out = Vec::new();
    let mut last_line = usize::MAX;
    for (i, t) in toks.iter().enumerate() {
        if matches!(t.tok, Tok::Newline | Tok::Eof) {
            continue;
        }
        let l = line_of(t.span.start);
        if l == last_line {
            continue;
        }
        last_line = l;
        let Tok::Word(w) = &t.tok else { continue };
        let word = |k: usize| match toks.get(k).map(|x| &x.tok) {
            Some(Tok::Word(n)) => Some(n.clone()),
            _ => None,
        };
        let (kind, name) = match w.as_str() {
            "section" => ("section", word(i + 1)),
            "let" => ("let", word(i + 1)),
            "def" => ("def", word(i + 1)),
            "arrange" => ("arrange", Some(word(i + 1).filter(|_| matches!(toks.get(i + 2).map(|x| &x.tok), Some(Tok::Colon))).unwrap_or_else(|| "arrange".to_string()))),
            _ => continue,
        };
        let Some(name) = name else { continue };
        // the comment-only lines touching it, upwards; a `#!` header line, a blank line or code ends the run
        let mut lines = Vec::new();
        let mut k = l;
        while k > 0 {
            let Some(c) = only_comment[k - 1] else { break };
            if c.text(text).starts_with("#!") {
                break;
            }
            lines.push(c);
            k -= 1;
        }
        if lines.is_empty() {
            continue;
        }
        lines.reverse();
        let joined = lines.iter().map(|c| c.text(text).trim_start_matches('#').trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        let doc = strip_name(&joined, &name);
        if doc.is_empty() {
            continue;
        }
        out.push(DocComment { kind, name, doc, item: Span::new(t.span.start, line_end(l)), comment: Span::new(lines[0].start, lines[lines.len() - 1].end) });
    }
    out
}

/// `doc` without a leading `<name>:` that repeats the item's name.
fn strip_name(doc: &str, name: &str) -> String {
    match doc.strip_prefix(name).map(str::trim_start).and_then(|r| r.strip_prefix(':')) {
        Some(rest) => rest.trim().to_string(),
        None => doc.to_string(),
    }
}

/// The doc comment of the item whose line holds byte `at` (a name's definition), if it has one.
pub fn doc_at(docs: &[DocComment], at: usize) -> Option<&DocComment> {
    docs.iter().find(|d| d.item.start <= at && at <= d.item.end)
}

/// The doc comment of section `name` (its first definition with one).
pub fn section_doc<'a>(docs: &'a [DocComment], name: &str) -> Option<&'a str> {
    docs.iter().find(|d| d.kind == "section" && d.name == name).map(|d| d.doc.as_str())
}

/// `doc` as a hover shows it: whole when it is at most [`HOVER_MAX`] characters, else cut at the last space before that
/// with an ellipsis.
pub fn hover_text(doc: &str) -> String {
    if doc.chars().count() <= HOVER_MAX {
        return doc.to_string();
    }
    let cut: String = doc.chars().take(HOVER_MAX).collect();
    let head = match cut.rfind(' ') {
        Some(sp) if sp > HOVER_MAX / 2 => &cut[..sp],
        _ => cut.as_str(),
    };
    format!("{}…", head.trim_end_matches([' ', ',', ';', ':', '.']))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALL: &str = "#! name: Hall of the Rave King
tempo 132
key am

# a: the theme, low and sneaking
section a 4 bars {
  deck A: kick x ~ ~ ~
}
# b: the same theme, faster and louder,
# the whole hall awake
section b 4 bars {
  deck A: kick x x x x
}

# what the stab plays

let stab = a3 c4 e4 ~
# def: not this def's name, so it stays
def pump(n) {
  deck B: hihat ~ x ~ x
}
# sneak in, then run
arrange {
  a
  b
}
";

    #[test]
    fn the_lines_above_an_item_are_its_doc() {
        let d = doc_comments(HALL);
        let got: Vec<(&str, &str, &str)> = d.iter().map(|d| (d.kind, d.name.as_str(), d.doc.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("section", "a", "the theme, low and sneaking"),
                ("section", "b", "the same theme, faster and louder, the whole hall awake"),
                ("def", "pump", "def: not this def's name, so it stays"),
                ("arrange", "arrange", "sneak in, then run"),
            ]
        );
        // `let stab` has a blank line between it and its comment: no doc
        assert!(d.iter().all(|d| d.name != "stab"));
        assert_eq!(section_doc(&d, "b"), Some("the same theme, faster and louder, the whole hall awake"));
        let at = HALL.find("section a").unwrap() + 8;
        assert_eq!(doc_at(&d, at).map(|d| d.name.as_str()), Some("a"));
    }

    #[test]
    fn a_header_line_is_never_a_doc() {
        let d = doc_comments("#! name: x\nsection s 1 bars {\n  deck A: kick x\n}\n");
        assert!(d.is_empty(), "{d:?}");
        let d = doc_comments("# first\n#! genre: techno\nsection s 1 bars {\n  deck A: kick x\n}\n");
        assert!(d.is_empty(), "a `#!` line between ends the run: {d:?}");
        let d = doc_comments("#! genre: techno\n# just this\nsection s 1 bars {\n  deck A: kick x\n}\n");
        assert_eq!(d[0].doc, "just this");
    }

    #[test]
    fn a_trailing_comment_is_not_a_doc_line() {
        let d = doc_comments("tempo 120 # fast\nsection s 1 bars {\n  deck A: kick x\n}\n");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn a_named_arrangement_is_named() {
        let d = doc_comments("# the set\narrange set: s loop\n");
        assert_eq!((d[0].kind, d[0].name.as_str(), d[0].doc.as_str()), ("arrange", "set", "the set"));
    }

    #[test]
    fn the_hover_is_trimmed() {
        assert_eq!(hover_text("short"), "short");
        let long = "word ".repeat(100);
        let h = hover_text(&long);
        assert!(h.ends_with('…') && h.chars().count() <= HOVER_MAX + 1, "{h}");
    }
}
