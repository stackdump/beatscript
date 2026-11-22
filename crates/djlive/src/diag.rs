//! Spans and diagnostics: every error and warning knows where in the text it is, what was
//! expected and what was found, and renders with a caret.

use djengine::json::{JsonObject, JsonValue};

/// A byte range of the program text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end: end.max(start) }
    }

    /// From the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    pub fn len(self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.end == self.start
    }

    pub fn text(self, src: &str) -> &str {
        src.get(self.start..self.end).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    /// What the check found out and has nothing wrong to say about (a chance's period, a program's length, an assertion
    /// measured on a render): not an error, and not a warning, which is for what may be wrong.
    Info,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// One thing to tell the person typing.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    /// What would have been accepted here (empty: not an "expected" error).
    pub expected: Vec<String>,
    /// What was there instead (empty: nothing to quote).
    pub found: String,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity: Severity::Error, span, message: message.into(), expected: Vec::new(), found: String::new(), help: None }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity: Severity::Warning, ..Diagnostic::error(span, message) }
    }

    /// A note: what the check found and nothing is wrong (severity [`Severity::Info`]).
    pub fn info(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity: Severity::Info, ..Diagnostic::error(span, message) }
    }

    pub fn expected(mut self, e: &[&str]) -> Diagnostic {
        self.expected = e.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn found(mut self, f: impl Into<String>) -> Diagnostic {
        self.found = f.into();
        self
    }

    pub fn help(mut self, h: impl Into<String>) -> Diagnostic {
        self.help = Some(h.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// 1-based line and column (in characters) of the start, and the column just past the end on
    /// that line (the span's end, or the line's end when it runs over).
    pub fn position(&self, src: &str) -> (usize, usize, usize) {
        let (line, col) = line_col(src, self.span.start);
        let (end_line, end_col) = line_col(src, self.span.end);
        let end_col = if end_line == line { end_col.max(col + 1) } else { col + 1 };
        (line, col, end_col)
    }

    /// The rustc-like form: message, `--> line:col`, the source line and a caret under the span.
    pub fn render(&self, src: &str) -> String {
        let (line, col, end_col) = self.position(src);
        // (a CR-only file is lines, and an editor's byte-order mark is no character of the line: R-19)
        let shown = src.replace("\r\n", "\n").replace('\r', "\n").replace('\u{feff}', "");
        let text = shown.lines().nth(line - 1).unwrap_or("");
        let gutter = line.to_string();
        let pad = " ".repeat(gutter.len());
        let carets = "^".repeat((end_col - col).max(1));
        let mut s = format!("{}: {}\n{pad}--> {line}:{col}\n{pad} |\n{gutter} | {text}\n{pad} | {}{carets}", self.severity.name(), self.message, " ".repeat(col - 1));
        if !self.expected.is_empty() {
            s.push_str(&format!("\n{pad} = expected: {}", list_or(&self.expected)));
        }
        if let Some(h) = &self.help {
            s.push_str(&format!("\n{pad} = help: {h}"));
        }
        s
    }

    pub fn to_json(&self, src: &str) -> JsonValue {
        let (line, col, end_col) = self.position(src);
        let mut o = JsonObject::new();
        o.set("severity", self.severity.name().into());
        o.set("line", JsonValue::int(line as i64));
        o.set("col", JsonValue::int(col as i64));
        o.set("end_col", JsonValue::int(end_col as i64));
        o.set("start", JsonValue::int(self.span.start as i64));
        o.set("end", JsonValue::int(self.span.end as i64));
        o.set("message", self.message.clone().into());
        o.set("expected", JsonValue::Array(self.expected.iter().map(|e| JsonValue::str(e.clone())).collect()));
        o.set("found", self.found.clone().into());
        o.set("help", self.help.clone().map_or(JsonValue::Null, Into::into));
        o.set("text", self.render(src).into());
        JsonValue::Object(o)
    }
}

/// "a, b or c".
pub fn list_or(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} or {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// 1-based (line, column in characters) of a byte offset. A line ends at `\n`, `\r\n` or a lone `\r`; a byte-order mark at the start is
/// no character (R-19).
pub fn line_col(src: &str, at: usize) -> (usize, usize) {
    let at = at.min(src.len());
    let before = &src[..floor_char(src, at)];
    let (mut line, mut col) = (1, 1);
    for (i, c) in before.char_indices() {
        match c {
            '\n' => (line, col) = (line + 1, 1),
            '\r' if !src[i + 1..].starts_with('\n') => (line, col) = (line + 1, 1),
            '\u{feff}' if i == 0 => {}
            _ => col += 1,
        }
    }
    (line, col)
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// All of `ds` rendered, errors first.
pub fn render_all(ds: &[Diagnostic], src: &str) -> String {
    let mut v: Vec<&Diagnostic> = ds.iter().collect();
    v.sort_by_key(|d| (d.severity != Severity::Error, d.span.start));
    v.iter().map(|d| d.render(src)).collect::<Vec<_>>().join("\n\n")
}
