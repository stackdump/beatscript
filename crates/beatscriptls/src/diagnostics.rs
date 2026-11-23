//! djlive's diagnostics as LSP `Diagnostic`s: the range, the severity, and the message with what was expected and the help on
//! lines of their own (an editor shows the message whole; the caret form is for a terminal and a model).

use djengine::json::JsonValue;
use djlive::diag::{list_or, Diagnostic, Severity};

use crate::analysis::Analysis;
use crate::json::{int, obj, s, span_range};
use crate::text::Encoding;

/// LSP's `DiagnosticSeverity`: 1 error, 2 warning, 3 information.
pub fn severity(sev: Severity) -> i64 {
    match sev {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Info => 3,
    }
}

/// The help a diagnostic carries, as the site says it (an `import` on the web gets the site's own help, as `beatscript-api` does).
pub fn help_of(d: &Diagnostic) -> Option<String> {
    if d.message == beatscript_api::snapshot::NO_IMPORT {
        return Some(beatscript_api::snapshot::IMPORT_HELP.to_string());
    }
    d.help.clone()
}

/// The message an editor shows: the message, then `expected: ...` and `help: ...` lines.
pub fn message_of(d: &Diagnostic) -> String {
    let mut m = d.message.clone();
    if !d.expected.is_empty() {
        m.push_str(&format!("\nexpected: {}", list_or(&d.expected)));
    }
    if let Some(h) = help_of(d) {
        m.push_str(&format!("\nhelp: {h}"));
    }
    m
}

/// One diagnostic. `index` is its place in the document's list: a code action names it by that (in `data`), so the action
/// for it is found again without comparing messages.
pub fn to_lsp(a: &Analysis, d: &Diagnostic, index: usize, enc: Encoding) -> JsonValue {
    let span = djlive::Span::new(d.span.start.min(a.text.len()), d.span.end.min(a.text.len()));
    obj([
        ("range", span_range(&a.index, &a.text, span, enc)),
        ("severity", JsonValue::int(severity(d.severity))),
        ("source", s("beatscript")),
        ("message", s(message_of(d))),
        ("data", obj([("index", int(index)), ("start", int(span.start)), ("end", int(span.end))])),
    ])
}

/// Every diagnostic of the document, in text order.
pub fn all(a: &Analysis, enc: Encoding) -> JsonValue {
    JsonValue::Array(a.diagnostics().iter().enumerate().map(|(i, d)| to_lsp(a, d, i, enc)).collect())
}
