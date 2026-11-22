//! Syntax classes for the editor, as one JSON array `[[start, end, "class"], ...]` in text order:
//! `btwd highlight` (UTF-8 byte offsets, what the card renderer slices with) and the wasm module's
//! `btw_highlight` (UTF-16 offsets, what a JS string slices with).
//!
//! The classes are `djlive::highlight`'s (`comment keyword role action deck number step rest generator
//! param operator word`), with these additions the page needs: the structure words (`section`, `def`,
//! `let` ...) are `keyword`, the transforms of an `evolve` rule are `action`, a quoted string is `string`,
//! a `$name` of a def is `param`, and a character the language cannot read is `error`.

use djengine::json::JsonValue;
use djlive::diag::Span;
use djlive::highlight::{highlight, Class};

use crate::utf16::Utf16Index;

/// Words that are `word` to the app's highlighter and structure to the page.
const KEYWORDS: [&str; 25] = [
    "let", "def", "repeat", "section", "arrange", "import", "end", "loop", "evolve", "use", "restricted", "sample", "from", "license", "else", "do", "for", "gens", "gen", "when", "after", "before", "next", "as", "launch",
];

/// An `evolve` rule's transforms.
const TRANSFORMS: [&str; 6] = ["rotate", "transpose", "octave", "invert", "reverse", "thin"];

pub fn class_name(c: Class) -> &'static str {
    match c {
        Class::Comment => "comment",
        Class::Keyword => "keyword",
        Class::Role => "role",
        Class::Action => "action",
        Class::Deck => "deck",
        Class::Number => "number",
        Class::Step => "step",
        Class::Rest => "rest",
        Class::Generator => "generator",
        Class::Param => "param",
        Class::Operator => "operator",
        Class::Word => "word",
    }
}

/// Every token's class in text order: `(span, class)` with byte offsets.
pub fn spans(text: &str) -> Vec<(Span, &'static str)> {
    let mut out: Vec<(Span, &'static str)> = highlight(text)
        .into_iter()
        .map(|(sp, c)| {
            let t = sp.text(text);
            let name = match c {
                Class::Word if KEYWORDS.contains(&t) => "keyword",
                Class::Word if TRANSFORMS.contains(&t) => "action",
                Class::Operator if t.starts_with('"') => "string",
                Class::Operator if t.starts_with('$') => "param",
                c => class_name(c),
            };
            (sp, name)
        })
        .collect();
    for d in djlive::lex::lex(text).errors {
        // the lexer can report a token it also read (`50%3`: a percent glued to a numeral): the error span is
        // the one to show, and an editor painting the spans in order must not meet the range twice
        out.retain(|(sp, _)| sp.end <= d.span.start || sp.start >= d.span.end);
        out.push((d.span, "error"));
    }
    out.sort_by_key(|(s, _)| (s.start, s.end));
    out
}

/// `[[start, end, "class"], ...]` with UTF-8 byte offsets.
pub fn highlight_json(text: &str) -> String {
    to_json(spans(text).into_iter().map(|(s, c)| (s.start, s.end, c)))
}

/// The same with UTF-16 offsets, for slicing a JS string.
pub fn highlight_json_utf16(text: &str) -> String {
    let ix = Utf16Index::new(text);
    to_json(spans(text).into_iter().map(|(s, c)| (ix.at(s.start), ix.at(s.end), c)))
}

fn to_json(spans: impl Iterator<Item = (usize, usize, &'static str)>) -> String {
    JsonValue::Array(spans.map(|(s, e, c)| JsonValue::Array(vec![JsonValue::int(s as i64), JsonValue::int(e as i64), JsonValue::str(c)])).collect()).js_stringified()
}
