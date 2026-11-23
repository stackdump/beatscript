//! The grammar of BeatScript, owned end to end: `grammar.ebnf` (the one source of truth) read into a tree
//! ([`ebnf`]), drawn as railroad diagrams in SVG ([`railroad`]), laid out as the Grammar page ([`page`]), and held to the
//! real parser both ways ([`recognize`]: every example program is a sentence of the grammar; [`generate`]: every
//! sentence the grammar writes is a program the parser reads). Add a production to grammar.ebnf and the page, the
//! docs' grammar block and the drift tests all follow.
//!
//! Pure: no I/O. The file is compiled in.

pub mod ebnf;
pub mod generate;
pub mod page;
pub mod railroad;
pub mod recognize;

/// `crates/djlive/grammar.ebnf`, the grammar.
pub const SOURCE: &str = include_str!("../../grammar.ebnf");

/// The grammar, read. The file is part of the build: a grammar that does not read is a failing test, never a panic at
/// run time for a caller that checks [`ebnf::read`] itself.
pub fn grammar() -> ebnf::Grammar {
    ebnf::read(SOURCE).unwrap_or_default()
}


/// The grammar block of docs/BEATSCRIPT.md: grammar.ebnf from its first section on (its header is what BEATSCRIPT.md says around
/// it), as an `ebnf` code block. tests/grammar.rs keeps BEATSCRIPT.md's copy equal to it.
pub fn docs_block() -> String {
    let from = SOURCE.find("(* ==").unwrap_or(0);
    format!("```ebnf\n{}```\n", SOURCE[from..].trim_end().to_string() + "\n")
}

/// The markers BEATSCRIPT.md puts the generated block between.
pub const DOCS_BEGIN: &str = "<!-- grammar.ebnf: generated from crates/djlive/grammar.ebnf; BEATSCRIPT_BLESS=1 cargo test -p djlive --test grammar rewrites it -->\n";
pub const DOCS_END: &str = "<!-- /grammar.ebnf -->";
