//! Navigation over the symbol table: go to definition, find references, document highlight, rename.
//!
//! A section renamed is renamed in the `arrange`, in `after` / `before` / `end after`, in `stack(...)` / `then(...)`: they are
//! references like any other. A parameter or a port renamed keeps the `$` of its uses (`$oct`), and a port keeps the dot of
//! a `wire` (`bed.chords`). A new name is refused when the language would read it as something else (`a` is a note, `kick`
//! a role, `stack` a word of the language: djlive's `reserved_as`) or when another name of the program already has it.

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::Span;
use djlive::lex::{lex, Tok};

use crate::analysis::Analysis;
use crate::json::{arr, int, obj, s, span_range, text_edit};
use crate::symbols::{Kind, Reference};
use crate::text::Encoding;

/// The symbol at byte `at`: (its index, the reference under the cursor).
pub fn symbol_at(a: &Analysis, at: usize) -> Option<(usize, Reference)> {
    let r = *a.symbols.ref_at(at)?;
    Some((r.symbol, r))
}

pub fn location(a: &Analysis, uri: &str, sp: Span, enc: Encoding) -> JsonValue {
    obj([("uri", s(uri)), ("range", span_range(&a.index, &a.text, sp, enc))])
}

/// `textDocument/definition`: the name where it is defined.
pub fn definition(a: &Analysis, uri: &str, at: usize, enc: Encoding) -> JsonValue {
    match symbol_at(a, at) {
        Some((si, _)) => location(a, uri, a.symbols.symbols[si].name_span, enc),
        None => JsonValue::Null,
    }
}

/// `textDocument/references`.
pub fn references(a: &Analysis, uri: &str, at: usize, include_declaration: bool, enc: Encoding) -> JsonValue {
    let Some((si, _)) = symbol_at(a, at) else { return JsonValue::Null };
    arr(a.symbols.refs_of(si).filter(|r| include_declaration || !r.is_def).map(|r| location(a, uri, r.span, enc)).collect())
}

/// `textDocument/documentHighlight`: every place of the name under the cursor; the definition is a write (3), a use a read (2).
pub fn highlight(a: &Analysis, at: usize, enc: Encoding) -> JsonValue {
    let Some((si, _)) = symbol_at(a, at) else { return JsonValue::Null };
    arr(a.symbols.refs_of(si).map(|r| obj([("range", span_range(&a.index, &a.text, r.span, enc)), ("kind", JsonValue::int(if r.is_def { 3 } else { 2 }))])).collect())
}

/// The span of the name itself in a reference (a `$oct` without its `$`).
fn name_span(r: &Reference) -> Span {
    if r.dollar {
        Span::new(r.span.start + 1, r.span.end)
    } else {
        r.span
    }
}

/// `textDocument/prepareRename`: the name under the cursor and its range, or why it cannot be renamed. Only a name the
/// program gives can be (a note, a role, a voice, a word of the language cannot: `a` here is a note, not a name).
pub fn prepare_rename(a: &Analysis, at: usize, enc: Encoding) -> Result<JsonValue, String> {
    let Some((si, r)) = symbol_at(a, at) else {
        let what = a.token_at(at).map(|i| a.tokens[i].span.text(&a.text).to_string()).unwrap_or_default();
        return Err(match djlive::parse::reserved(&what) {
            Some(why) if !what.is_empty() => format!("`{what}` is not a name of this program: {why}"),
            _ if what.is_empty() => "there is no name here".to_string(),
            _ => format!("`{what}` is not a name this program defines"),
        });
    };
    let sym = &a.symbols.symbols[si];
    if sym.kind == Kind::Import {
        return Err("an import names a saved snippet: rename it where it is saved".into());
    }
    Ok(obj([("range", span_range(&a.index, &a.text, name_span(&r), enc)), ("placeholder", s(sym.name.clone()))]))
}

/// Why `new` cannot be the new name of symbol `si` (None: it can).
pub fn refuse_name(a: &Analysis, si: usize, new: &str) -> Option<String> {
    let sym = &a.symbols.symbols[si];
    // one word, as the lexer reads it
    let lexed = lex(new);
    let one_word = lexed.errors.is_empty() && matches!(lexed.tokens.as_slice(), [t, e] if t.tok == Tok::Word(new.to_string()) && e.tok == Tok::Eof);
    if !one_word {
        return Some(format!("`{new}` is not one name: a name is letters, digits, `_` and `-`, starting with a letter"));
    }
    if let Some(why) = djlive::parse::reserved_as(new, sym.kind.word()) {
        return Some(format!("`{new}` cannot be a name: {why}"));
    }
    // a name another symbol of the program has (a parameter or a port only clashes inside its def)
    let clash = a.symbols.named(new).find(|(i, o)| {
        *i != si
            && match (sym.kind, o.kind) {
                (Kind::Param | Kind::Port, Kind::Param | Kind::Port) => o.parent == sym.parent,
                (Kind::Param | Kind::Port, _) | (_, Kind::Param | Kind::Port) => false,
                _ => true,
            }
    });
    if let Some((_, o)) = clash {
        let line = a.index.line_of(o.name_span.start) + 1;
        return Some(format!("`{new}` is already the {} on line {line}", o.kind.word()));
    }
    None
}

/// `textDocument/rename`: a `WorkspaceEdit` changing every place of the name.
pub fn rename(a: &Analysis, uri: &str, at: usize, new: &str, enc: Encoding) -> Result<JsonValue, String> {
    let Some((si, _)) = symbol_at(a, at) else { return Err("there is no name here to rename".into()) };
    if a.symbols.symbols[si].kind == Kind::Import {
        return Err("an import names a saved snippet: rename it where it is saved".into());
    }
    if let Some(why) = refuse_name(a, si, new) {
        return Err(why);
    }
    let edits: Vec<JsonValue> = a.symbols.refs_of(si).map(|r| text_edit(&a.index, &a.text, name_span(r), new, enc)).collect();
    let mut changes = JsonObject::new();
    changes.set(uri, arr(edits));
    Ok(obj([("changes", JsonValue::Object(changes))]))
}

/// `textDocument/documentSymbol`: the outline, nested (a section's parts and settings inside it, a def's parameters and
/// ports inside it).
pub fn document_symbols(a: &Analysis, enc: Encoding) -> JsonValue {
    fn one(a: &Analysis, i: usize, enc: Encoding) -> JsonValue {
        let sym = &a.symbols.symbols[i];
        let mut o = JsonObject::new();
        o.set("name", s(sym.name.clone()));
        if !sym.detail.is_empty() {
            o.set("detail", s(short(&sym.detail, 60)));
        }
        o.set("kind", JsonValue::int(sym.kind.lsp()));
        o.set("range", span_range(&a.index, &a.text, sym.full, enc));
        o.set("selectionRange", span_range(&a.index, &a.text, sym.name_span, enc));
        if !sym.children.is_empty() {
            o.set("children", arr(sym.children.iter().map(|&c| one(a, c, enc)).collect()));
        }
        JsonValue::Object(o)
    }
    arr(a.symbols.roots().into_iter().map(|i| one(a, i, enc)).collect())
}

fn short(t: &str, n: usize) -> String {
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= n {
        t
    } else {
        format!("{}…", t.chars().take(n - 1).collect::<String>())
    }
}

/// `textDocument/foldingRange`: every block that spans lines (sections, defs, `evolve`, `repeat`, `mix`, `set`) and every run of
/// comment lines.
pub fn folding_ranges(a: &Analysis) -> JsonValue {
    arr(a
        .symbols
        .folds
        .iter()
        .filter_map(|(sp, kind)| {
            let (l0, l1) = (a.index.line_of(sp.start), a.index.line_of(sp.end));
            // a block folds to its header line; its `}` stays in view
            let end = if *kind == crate::symbols::FoldKind::Region { l1.saturating_sub(1).max(l0) } else { l1 };
            (end > l0).then(|| {
                let mut o = JsonObject::new();
                o.set("startLine", int(l0));
                o.set("endLine", int(if *kind == crate::symbols::FoldKind::Region { l1 } else { end }));
                if *kind == crate::symbols::FoldKind::Comment {
                    o.set("kind", s("comment"));
                }
                JsonValue::Object(o)
            })
        })
        .collect())
}
