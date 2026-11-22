//! One document's analysis, in two tiers.
//!
//! - **The syntax** (lex, parse, the symbol table): about a millisecond on the largest programs, done on every change, and
//!   all that completion, hover, navigation, the outline and the refactors need.
//! - **The check** (`djlive::check_with`: expand, compile, evolve orbits): the diagnostics a compile finds. Usually a few ms,
//!   but seconds on a long evolving song (deep-house-60: ~1.5-3 s, its generations' net CIDs), so it is never on the keystroke path:
//!   it is computed when first asked for ([`Analysis::check`]), or handed in from a worker ([`Analysis::set_check`],
//!   what `btwd lsp` does), and until then the diagnostics are the parser's.

use std::cell::OnceCell;

use djlive::compile::{check_with, Check};
use djlive::diag::Diagnostic;
use djlive::lex::{lex, Tok, Token};
use djlive::Span;

use crate::symbols::Symbols;
use crate::text::LineIndex;

pub struct Analysis {
    pub text: String,
    pub index: LineIndex,
    /// The text's own tokens as the lexer reads them (before any `let` is put in place or any `import` read), ending in `Eof`.
    pub tokens: Vec<Token>,
    pub comments: Vec<Span>,
    /// What the lexer and the parser say (a subset of the check's diagnostics).
    pub parse_diagnostics: Vec<Diagnostic>,
    pub symbols: Symbols,
    /// The `let`s of one number, with the number each came to.
    pub scalar_lets: Vec<djlive::parse::ScalarLet>,
    /// The program as the parser read it (a statement with an error is not in it).
    pub program: djlive::ast::Program,
    /// The doc comments: the `#` lines directly above a `section`, `let`, `def` or `arrange` (djlive::doc).
    pub docs: Vec<djlive::doc::DocComment>,
    check: OnceCell<Check>,
    /// The program's references (`djlive::refs::sites`), worked out when first asked for ([`Analysis::refs`]).
    refs: OnceCell<Result<Vec<djlive::refs::Site>, Vec<Diagnostic>>>,
}

/// The check of a text against the site's environment (`btwapi::web_env`): what `btwd check` and the page say.
pub fn full_check(text: &str) -> Check {
    let env = btwapi::web_env().env();
    check_with(text, &env)
}

impl Analysis {
    /// The syntax tier only; the check waits until it is asked for.
    pub fn new(text: impl Into<String>) -> Analysis {
        let text = text.into();
        let env = btwapi::web_env().env();
        let parsed = djlive::parse::parse_with(&text, &env);
        let lexed = lex(&text);
        let symbols = Symbols::build(&text, &lexed.tokens, &lexed.comments);
        Analysis {
            index: LineIndex::new(&text),
            tokens: lexed.tokens,
            comments: lexed.comments,
            parse_diagnostics: parsed.diagnostics,
            symbols,
            scalar_lets: parsed.scalar_lets,
            program: parsed.program,
            docs: djlive::doc::doc_comments(&text),
            check: OnceCell::new(),
            refs: OnceCell::new(),
            text,
        }
    }

    /// Both tiers now.
    pub fn checked(text: impl Into<String>) -> Analysis {
        let a = Analysis::new(text);
        a.check();
        a
    }

    /// The check, computed now if it has not been.
    pub fn check(&self) -> &Check {
        self.check.get_or_init(|| full_check(&self.text))
    }

    /// The program's references, resolved (a generator call is generated, so this is not free: it is done once per text, when
    /// a hover, a lens or a code action first asks). A text with nothing a reference starts with has none at no cost; Err: what
    /// is wrong with one of them (as `check` says it, at the reference).
    pub fn refs(&self) -> Result<&[djlive::refs::Site], &[Diagnostic]> {
        let r = self.refs.get_or_init(|| if crate::refs::may_have(self) { djlive::refs::sites(&self.text, &btwapi::web_env().env()) } else { Ok(Vec::new()) });
        match r {
            Ok(v) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// The check if it is done.
    pub fn check_ready(&self) -> Option<&Check> {
        self.check.get()
    }

    /// Hand in a check made elsewhere (a worker thread) of this same text.
    pub fn set_check(&self, c: Check) {
        let _ = self.check.set(c);
    }

    /// The diagnostics to show: the check's when it is done, else the parser's.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self.check.get() {
            Some(c) => &c.diagnostics,
            None => &self.parse_diagnostics,
        }
    }

    /// The index of the token at byte `at`, or the one that ends there (the word the cursor is just after). Newlines and the
    /// end are not tokens here.
    pub fn token_at(&self, at: usize) -> Option<usize> {
        let mut best = None;
        for (i, t) in self.tokens.iter().enumerate() {
            if matches!(t.tok, Tok::Newline | Tok::Eof) {
                continue;
            }
            if t.span.start <= at && at < t.span.end {
                return Some(i);
            }
            if t.span.end == at {
                best = Some(i);
            }
            if t.span.start > at {
                break;
            }
        }
        best
    }

    /// The doc comment of the item whose line holds byte `at`, as a hover shows it (trimmed).
    pub fn doc_at(&self, at: usize) -> Option<String> {
        djlive::doc::doc_at(&self.docs, at).map(|d| djlive::doc::hover_text(&d.doc))
    }

    /// Is byte `at` inside a comment (after its `#`)?
    pub fn in_comment(&self, at: usize) -> bool {
        self.comments.iter().any(|c| c.start < at && at <= c.end)
    }
}
