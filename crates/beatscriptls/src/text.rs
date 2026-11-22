//! Positions: djlive speaks byte offsets into UTF-8; an LSP client speaks `{line, character}` where a character is a UTF-16
//! code unit (the default) or a UTF-8 byte (when both sides agree on `utf-8` in `initialize`). [`LineIndex`] converts both ways
//! for one text. A line ends at `\n`, `\r\n` or a lone `\r` (the LSP's rule, and djlive's `line_col`).

use djlive::diag::Span;

/// How the client counts `character` in a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    /// UTF-16 code units: the LSP default, and what JavaScript's string offsets are.
    #[default]
    Utf16,
    /// UTF-8 bytes: djlive's own offsets.
    Utf8,
}

impl Encoding {
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf16 => "utf-16",
            Encoding::Utf8 => "utf-8",
        }
    }

    pub fn parse(s: &str) -> Option<Encoding> {
        match s {
            "utf-16" => Some(Encoding::Utf16),
            "utf-8" => Some(Encoding::Utf8),
            _ => None,
        }
    }
}

/// A position as the client sends it: a 0-based line and a 0-based character in [`Encoding`] units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Position {
        Position { line, character }
    }
}

/// A range of positions, end exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Where each line of one text starts, and whether the text is all ASCII (then every encoding counts the same).
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
    ascii: bool,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let b = text.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => starts.push(i + 1),
                b'\r' if b.get(i + 1) != Some(&b'\n') => starts.push(i + 1),
                _ => {}
            }
            i += 1;
        }
        LineIndex { starts, len: text.len(), ascii: text.is_ascii() }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// The byte where line `line` (0-based) starts; the end of the text past the last line.
    pub fn line_start(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.len)
    }

    /// The byte where line `line`'s text ends (before its line break).
    pub fn line_end(&self, text: &str, line: usize) -> usize {
        let next = self.starts.get(line + 1).copied().unwrap_or(self.len);
        let b = text.as_bytes();
        let mut e = next;
        if e > self.line_start(line) && e <= b.len() && e > 0 && b[e - 1] == b'\n' {
            e -= 1;
        }
        if e > self.line_start(line) && e > 0 && e <= b.len() && b[e - 1] == b'\r' {
            e -= 1;
        }
        e
    }

    /// The 0-based line a byte offset is on.
    pub fn line_of(&self, at: usize) -> usize {
        match self.starts.binary_search(&at.min(self.len)) {
            Ok(l) => l,
            Err(l) => l - 1,
        }
    }

    /// A byte offset as a position. An offset inside a character counts as the character's start.
    pub fn position(&self, text: &str, at: usize, enc: Encoding) -> Position {
        let at = floor_char(text, at.min(self.len));
        let line = self.line_of(at);
        let start = self.starts[line];
        let character = if self.ascii || enc == Encoding::Utf8 { at - start } else { text[start..at].chars().map(char::len_utf16).sum() };
        Position { line: line as u32, character: character as u32 }
    }

    /// A span as a range.
    pub fn range(&self, text: &str, span: Span, enc: Encoding) -> Range {
        Range { start: self.position(text, span.start, enc), end: self.position(text, span.end.max(span.start), enc) }
    }

    /// A position as a byte offset. A line past the end is the end of the text; a character past the end of its line is the
    /// line's end; a character inside a UTF-8 or UTF-16 sequence is that character's start.
    pub fn offset(&self, text: &str, p: Position, enc: Encoding) -> usize {
        let line = p.line as usize;
        if line >= self.starts.len() {
            return self.len;
        }
        let start = self.starts[line];
        let end = self.line_end(text, line);
        let want = p.character as usize;
        if self.ascii || enc == Encoding::Utf8 {
            return floor_char(text, (start + want).min(end));
        }
        let mut units = 0;
        for (i, c) in text[start..end].char_indices() {
            if units >= want {
                return start + i;
            }
            units += c.len_utf16();
            if units > want {
                return start + i;
            }
        }
        end
    }

    /// A range as a span.
    pub fn span(&self, text: &str, r: Range, enc: Encoding) -> Span {
        Span::new(self.offset(text, r.start, enc), self.offset(text, r.end, enc))
    }
}

pub(crate) fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_lines_and_breaks() {
        let t = "ab\ncd\r\nef\rg";
        let ix = LineIndex::new(t);
        assert_eq!(ix.line_count(), 4);
        assert_eq!(ix.position(t, 4, Encoding::Utf16), Position::new(1, 1));
        assert_eq!(ix.position(t, 7, Encoding::Utf16), Position::new(2, 0));
        assert_eq!(ix.position(t, 10, Encoding::Utf16), Position::new(3, 0));
        assert_eq!(ix.offset(t, Position::new(1, 99), Encoding::Utf16), 5);
        assert_eq!(ix.offset(t, Position::new(9, 0), Encoding::Utf16), t.len());
    }

    #[test]
    fn utf16_and_utf8_count_differently() {
        // `é` is 2 bytes and 1 unit, `𝄞` is 4 bytes and 2 units
        let t = "# é𝄞x\nkick";
        let ix = LineIndex::new(t);
        let x = t.find('x').unwrap();
        assert_eq!(ix.position(t, x, Encoding::Utf8), Position::new(0, 8));
        assert_eq!(ix.position(t, x, Encoding::Utf16), Position::new(0, 5));
        assert_eq!(ix.offset(t, Position::new(0, 5), Encoding::Utf16), x);
        assert_eq!(ix.offset(t, Position::new(0, 8), Encoding::Utf8), x);
        // inside the surrogate pair: the character's start
        assert_eq!(ix.offset(t, Position::new(0, 4), Encoding::Utf16), t.find('𝄞').unwrap());
        for at in 0..=t.len() {
            if t.is_char_boundary(at) {
                for enc in [Encoding::Utf8, Encoding::Utf16] {
                    assert_eq!(ix.offset(t, ix.position(t, at, enc), enc), at, "{at} {enc:?}");
                }
            }
        }
    }
}
