//! Partial statements, for an editor: the statement the cursor is in, read as far as it goes. The parser proper drops a
//! statement with an error (and goes on from the next line); an editor asking "what can come here?" in the middle of a line
//! wants the opposite: the statement up to the cursor, as much of it as reads (the partial AST), and what the parser would
//! have taken at the cursor (its `expected`, which the error at the end of the text carries).
//!
//! Kept out of the parser's own loop on purpose: it reads with the same [`Parser`], on a copy of the tokens cut at the
//! cursor, so nothing the parser does for a whole program changes.

use std::collections::BTreeSet;

use crate::ast::Stmt;
use crate::diag::{Diagnostic, Span};
use crate::lex::{lex, Tok, Token};
use crate::parse::Parser;

/// The statement the cursor is in, up to the cursor.
#[derive(Clone, Debug)]
pub struct Partial {
    /// Where the statement starts.
    pub start: usize,
    /// The statement's tokens before the cursor (the word being typed, when the cursor is in or just after one, is not
    /// among them: it is [`Partial::word`]).
    pub tokens: Vec<Token>,
    /// The word the cursor is in or just after (what a completion replaces), and its span. Empty at a space.
    pub word: String,
    pub word_span: Span,
    /// The longest prefix of [`Partial::tokens`] that reads as a statement: the partial AST.
    pub stmt: Option<Stmt>,
    /// How many of the tokens that statement took.
    pub read: usize,
    /// The parser's error at the cursor, when the tokens before it read up to the cursor and then wanted more: its
    /// `expected` is what may come here. None when the statement is complete at the cursor or broke earlier.
    pub wants: Option<Diagnostic>,
}

/// The statement around byte `at` of `src`, read up to `at`.
pub fn statement_at(src: &str, at: usize) -> Partial {
    let at = at.min(src.len());
    let lexed = lex(src);
    let toks = lexed.tokens;
    // the statement starts after the last newline, `{` or `}` before the cursor
    // (or after a line break in the text that is not a `|` continuation: the lexer keeps no newline at the end of the text)
    let mut start_ix = 0;
    for (i, t) in toks.iter().enumerate() {
        if t.span.start >= at {
            break;
        }
        if matches!(t.tok, Tok::Newline | Tok::LBrace | Tok::RBrace) && t.span.end <= at {
            start_ix = i + 1;
            continue;
        }
        let next = toks.get(i + 1).map_or(at, |n| n.span.start.min(at)).max(t.span.end);
        let gap = &src[t.span.end.min(src.len())..next.min(src.len())];
        if let Some(nl) = gap.rfind(['\n', '\r']) {
            if !gap[nl + 1..].trim_start().starts_with('|') && !src[next.min(src.len())..].trim_start().starts_with('|') {
                start_ix = i + 1;
            }
        }
    }
    let mut before: Vec<Token> = toks[start_ix..].iter().take_while(|t| t.span.start < at && !matches!(t.tok, Tok::Eof | Tok::Newline)).cloned().collect();
    // the word being typed: a word (or `$name`) that the cursor is in or at the end of
    let (mut word, mut word_span) = (String::new(), Span::new(at, at));
    if let Some(last) = before.last() {
        if last.span.end >= at {
            if let Tok::Word(_) | Tok::Dollar(_) | Tok::Num { .. } = &last.tok {
                word_span = Span::new(last.span.start, at);
                word = src[word_span.start..word_span.end].to_string();
                before.pop();
            }
        }
    }
    let start = before.first().map_or(word_span.start, |t| t.span.start);
    let (stmt, read, wants) = read_prefix(src, &before, at);
    Partial { start, tokens: before, word, word_span, stmt, read, wants }
}

/// Read `toks` as one statement; on an error, read ever shorter prefixes until one reads. The first error is kept as what
/// the statement wants at the cursor when it stands at the end of the tokens (it found the end, not a token it could not
/// take).
fn read_prefix(src: &str, toks: &[Token], at: usize) -> (Option<Stmt>, usize, Option<Diagnostic>) {
    let mut wants = None;
    let mut n = toks.len();
    loop {
        if n == 0 {
            return (None, 0, wants);
        }
        let mut v: Vec<Token> = toks[..n].to_vec();
        let end = Span::new(at, at);
        v.push(Token { tok: Tok::Eof, span: end });
        let mut p = Parser { toks: v, at: 0, src, diags: Vec::new(), depth: 0, ratios: BTreeSet::new() };
        match p.statement() {
            Ok(s) => return (Some(s), n, wants),
            Err(d) => {
                if n == toks.len() && wants.is_none() && d.span.start >= toks.last().map_or(0, |t| t.span.end) {
                    wants = Some(d);
                }
                n -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Body, StmtKind};

    #[test]
    fn a_line_cut_at_a_param_reads_up_to_it_and_says_what_it_wants() {
        let src = "deck A: kick x ~ x ~ vel=";
        let p = statement_at(src, src.len());
        assert_eq!(p.word, "");
        // the line reads without the dangling `vel=`
        let Some(Stmt { kind: StmtKind::Line(l), .. }) = &p.stmt else { panic!("{:?}", p.stmt) };
        let Body::Parts(parts) = &l.body else { panic!() };
        assert_eq!(parts[0].role.text, "kick");
        assert!(p.wants.is_some());
    }

    #[test]
    fn the_word_being_typed_is_not_read() {
        let src = "tempo 120\nsection drop 16 ba";
        let p = statement_at(src, src.len());
        assert_eq!((p.word.as_str(), p.word_span), ("ba", Span::new(src.len() - 2, src.len())));
        assert_eq!(p.wants.map(|d| d.expected), Some(vec!["`bars`".to_string()]));
    }

    #[test]
    fn a_new_line_at_the_end_of_the_text_starts_a_statement() {
        let src = "end after drop\n";
        let p = statement_at(src, src.len());
        assert!(p.tokens.is_empty() && p.word.is_empty(), "{:?}", p.tokens);
        // a `|` line continues the one before
        let src = "deck A: kick x\n  | sn";
        assert_eq!(statement_at(src, src.len()).tokens.len(), 6);
    }

    #[test]
    fn inside_a_block_the_statement_starts_after_the_brace() {
        let src = "section a 4 bars {\n  deck A: kick x\n  deck B: ";
        let p = statement_at(src, src.len());
        assert_eq!(p.start, src.rfind("deck").unwrap());
        assert!(p.wants.is_some_and(|d| !d.expected.is_empty()));
    }
}
