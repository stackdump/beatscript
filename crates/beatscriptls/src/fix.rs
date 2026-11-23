//! Quick-fixes: the repairs of the text a diagnostic offers, as LSP code actions, and "fix all" (every certain repair at once).
//!
//! The repairs are the fixer's (run e8d44b49, branch `feat/btw-fixer`: `djlive::diag::Fix` on `Diagnostic::fixes`, and
//! `beatscript check --fix`). Until that lands on main, [`fixes_of`] is an adapter that reads the same repairs out of what a
//! diagnostic already says: a `did you mean `x`?` help is the fixer's `respell` (certain when it names one word, likely
//! when it names several). [`QuickFix`] has the fixer's shape (span, replacement, confidence, note), so the merge is one
//! function: `fixes_of` maps `d.fixes` and keeps the adapter only for a diagnostic with none.
//!
//! The policy is Matt's and the fixer's: a fix adds or changes what is said; what has to go is commented out with
//! `# fixer: why`, never deleted. The adapter only respells and closes blocks, so it never deletes.

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::{Diagnostic, Span};

use crate::analysis::Analysis;
use crate::json::{arr, obj, s, text_edit};
use crate::text::Encoding;

/// How sure a repair is (the fixer's `Confidence`). Fix-all applies the certain ones; an editor offers the likely ones to click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Certain,
    Likely,
}

/// One repair: `span` becomes `replacement` (an empty span inserts). The fixer's `Fix`.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickFix {
    pub span: Span,
    pub replacement: String,
    pub confidence: Confidence,
    /// For a person: "`kcik` -> `kick`".
    pub note: String,
}

/// The repairs of one diagnostic. (Merge with feat/btw-fixer: `d.fixes.iter().map(|f| QuickFix { span: f.span, replacement:
/// f.replacement.clone(), confidence: .., note: f.note.clone() })`, and [`adapter`] when that is empty.)
pub fn fixes_of(d: &Diagnostic, src: &str) -> Vec<QuickFix> {
    adapter(d, src)
}

/// The words a `did you mean ...?` names, in backticks: `did you mean `kick`?`, `did you mean `a` or `b`?`.
pub fn did_you_mean(help: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = help;
    while let Some(k) = rest.find("did you mean ") {
        let tail = &rest[k + "did you mean ".len()..];
        let end = tail.find('?').unwrap_or(tail.len());
        let mut q = &tail[..end];
        while let Some(a) = q.find('`') {
            let Some(b) = q[a + 1..].find('`') else { break };
            let w = &q[a + 1..a + 1 + b];
            if !w.is_empty() && !w.contains(char::is_whitespace) && !out.iter().any(|o| o == w) {
                out.push(w.to_string());
            }
            q = &q[a + 2 + b..];
        }
        rest = &tail[end..];
    }
    out
}

/// The word the person typed that a respelling replaces: the diagnostic's `found` (`` `kcik` ``), else the span's text.
fn typed_word(d: &Diagnostic, src: &str) -> Option<(String, Span)> {
    let in_span = d.span.text(src);
    let found = d.found.trim().trim_matches('`');
    let word = if !found.is_empty() && !found.contains(' ') && in_span.contains(found) { found } else { in_span.trim() };
    if word.is_empty() || word.contains(char::is_whitespace) {
        return None;
    }
    // the word inside the span, at a word boundary
    let base = d.span.start;
    let mut from = 0;
    while let Some(k) = in_span[from..].find(word) {
        let at = from + k;
        let before = in_span[..at].chars().next_back();
        let after = in_span[at + word.len()..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
        if edge(before) && edge(after) {
            return Some((word.to_string(), Span::new(base + at, base + at + word.len())));
        }
        from = at + word.len().max(1);
    }
    None
}

/// The repairs the diagnostic's own words say (see the module doc).
pub fn adapter(d: &Diagnostic, src: &str) -> Vec<QuickFix> {
    let mut out = Vec::new();
    if let Some(help) = crate::diagnostics::help_of(d) {
        let mut names = did_you_mean(&help);
        let mut conf = if names.len() == 1 { Confidence::Certain } else { Confidence::Likely };
        // a voice name that only names a family: the voice with a sound of its own it is nearest (a guess: likely)
        if names.is_empty() {
            if let Some(k) = help.find("a voice with a sound of its own: `") {
                let rest = &help[k + "a voice with a sound of its own: `".len()..];
                if let Some(e) = rest.find('`') {
                    names.push(rest[..e].to_string());
                    conf = Confidence::Likely;
                }
            }
        }
        if let Some((typed, span)) = typed_word(d, src) {
            for n in names.iter().filter(|n| **n != typed) {
                // a `$param` respelled keeps its `$`; a name is not respelled into a `name=` or a phrase
                if n.contains('=') {
                    continue;
                }
                out.push(QuickFix { span, replacement: n.clone(), confidence: conf, note: format!("`{typed}` -> `{n}`") });
            }
        }
    }
    // a block that is never closed: close it at the end of the text
    if d.message.contains("is never closed") && d.message.contains('{') {
        let end = src.trim_end().len();
        let nl = if src[..end].ends_with('\n') || end == 0 { "" } else { "\n" };
        out.push(QuickFix { span: Span::new(end, end), replacement: format!("{nl}}}"), confidence: Confidence::Likely, note: "close the block at the end of the text".into() });
    }
    out
}

/// A `WorkspaceEdit` of these edits on `uri`.
pub fn workspace_edit(a: &Analysis, uri: &str, edits: &[(Span, String)], enc: Encoding) -> JsonValue {
    let mut changes = JsonObject::new();
    changes.set(uri, arr(edits.iter().map(|(sp, t)| text_edit(&a.index, &a.text, *sp, t, enc)).collect()));
    obj([("changes", JsonValue::Object(changes))])
}

/// Every certain repair of the document that does not overlap another, in text order: what fix-all applies.
pub fn fix_all(a: &Analysis) -> Vec<(usize, QuickFix)> {
    let mut all: Vec<(usize, QuickFix)> = Vec::new();
    for (i, d) in a.diagnostics().iter().enumerate() {
        let fs = fixes_of(d, &a.text);
        // a diagnostic with more than one certain repair is a choice, not a fix
        let certain: Vec<QuickFix> = fs.into_iter().filter(|f| f.confidence == Confidence::Certain).collect();
        if certain.len() == 1 {
            all.push((i, certain.into_iter().next().unwrap()));
        }
    }
    all.sort_by_key(|(_, f)| (f.span.start, f.span.end));
    let mut out: Vec<(usize, QuickFix)> = Vec::new();
    for (i, f) in all {
        if let Some((_, last)) = out.last() {
            if f.span.start < last.span.end || (f.span == last.span && f.span.is_empty()) {
                continue;
            }
        }
        out.push((i, f));
    }
    out
}

/// `textDocument/codeAction` for quick-fixes: one action per repair of a diagnostic the range touches, then fix-all when
/// there is more than one certain repair in the document (or when the client asks for `source.fixAll`).
pub fn code_actions(a: &Analysis, uri: &str, range: Span, only: &[String], enc: Encoding) -> Vec<JsonValue> {
    let wants = |kind: &str| only.is_empty() || only.iter().any(|o| kind == o || kind.starts_with(&format!("{o}.")));
    let mut out = Vec::new();
    if wants("quickfix") {
        for (i, d) in a.diagnostics().iter().enumerate() {
            let touches = d.span.start <= range.end && range.start <= d.span.end;
            if !touches {
                continue;
            }
            for f in fixes_of(d, &a.text) {
                out.push(obj([
                    ("title", s(f.note.clone())),
                    ("kind", s("quickfix")),
                    ("diagnostics", arr(vec![crate::diagnostics::to_lsp(a, d, i, enc)])),
                    ("isPreferred", JsonValue::Bool(f.confidence == Confidence::Certain)),
                    ("edit", workspace_edit(a, uri, &[(f.span, f.replacement.clone())], enc)),
                ]));
            }
        }
    }
    let all = fix_all(a);
    // `source.fixAll.btw`: what an editor set up before the rename asks for (crate::legacy); it gets the kind it asked for
    let legacy = only.iter().any(|o| o == crate::legacy::FIX_ALL);
    let explicit = legacy || only.iter().any(|o| o == "source.fixAll" || o == "source.fixAll.beatscript" || o == "source");
    if (all.len() > 1 && wants("quickfix")) || (explicit && !all.is_empty()) {
        let edits: Vec<(Span, String)> = all.iter().map(|(_, f)| (f.span, f.replacement.clone())).collect();
        out.push(obj([
            ("title", s(format!("Fix all certain problems ({})", all.len()))),
            ("kind", s(if legacy { crate::legacy::FIX_ALL } else if explicit { "source.fixAll.beatscript" } else { "quickfix" })),
            ("diagnostics", arr(all.iter().map(|(i, _)| crate::diagnostics::to_lsp(a, &a.diagnostics()[*i], *i, enc)).collect())),
            ("edit", workspace_edit(a, uri, &edits, enc)),
        ]));
    }
    out
}

/// The text with every fix-all repair applied (what `beatscript check --fix` would write, from the certain repairs alone).
pub fn apply_all(a: &Analysis) -> String {
    let mut t = a.text.clone();
    for (_, f) in fix_all(a).into_iter().rev() {
        t.replace_range(f.span.start..f.span.end, &f.replacement);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_you_mean_reads_one_or_several_words() {
        assert_eq!(did_you_mean("did you mean `kick`?"), ["kick"]);
        assert_eq!(did_you_mean("did you mean `a` or `b`? the sections are a, b"), ["a", "b"]);
        assert!(did_you_mean("rename it").is_empty());
        assert!(did_you_mean("did you mean the parameter `vel=` ?").iter().all(|w| w == "vel="));
    }
}
