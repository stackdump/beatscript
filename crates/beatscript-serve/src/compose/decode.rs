//! What Go's `json.Decoder` says about a request body, for the answers that name it: a syntax error at its byte
//! offset (encoding/json's scanner, which counts the offending byte), an empty body (`io.EOF`), a cut-off one
//! (`io.ErrUnexpectedEOF`), trailing data after the one value, and the bytes past a `MaxBytesReader` limit.

/// The first value of `b`, as the decoder reads it.
#[derive(Debug, PartialEq)]
pub enum Scan {
    /// Only whitespace: `io.EOF`.
    Empty,
    /// The value ends at this offset (exclusive).
    Value(usize),
    /// A syntax error; `offset` as `json.SyntaxError.Offset`.
    Syntax(usize),
    /// The data ended inside the value: `io.ErrUnexpectedEOF` (or, with more data unread, a limit hit).
    Incomplete,
}

fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

enum E {
    Syn(usize),
    Eof,
}

impl P<'_> {
    fn peek(&self) -> Result<u8, E> {
        self.b.get(self.i).copied().ok_or(E::Eof)
    }
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && ws(self.b[self.i]) {
            self.i += 1;
        }
    }
    fn err(&self) -> E {
        E::Syn(self.i + 1)
    }
    fn value(&mut self) -> Result<(), E> {
        self.skip_ws();
        match self.peek()? {
            b'{' => {
                self.i += 1;
                self.skip_ws();
                if self.peek()? == b'}' {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.skip_ws();
                    if self.peek()? != b'"' {
                        return Err(self.err());
                    }
                    self.string()?;
                    self.skip_ws();
                    if self.peek()? != b':' {
                        return Err(self.err());
                    }
                    self.i += 1;
                    self.value()?;
                    self.skip_ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err()),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                self.skip_ws();
                if self.peek()? == b']' {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.value()?;
                    self.skip_ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err()),
                    }
                }
            }
            b'"' => self.string(),
            b't' => self.lit(b"true"),
            b'f' => self.lit(b"false"),
            b'n' => self.lit(b"null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err()),
        }
    }
    fn lit(&mut self, w: &[u8]) -> Result<(), E> {
        for &c in w {
            if self.peek()? != c {
                return Err(self.err());
            }
            self.i += 1;
        }
        Ok(())
    }
    fn string(&mut self) -> Result<(), E> {
        self.i += 1;
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return Ok(()),
                b'\\' => {
                    let e = self.peek()?;
                    match e {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.i += 1,
                        b'u' => {
                            self.i += 1;
                            for _ in 0..4 {
                                if !self.peek()?.is_ascii_hexdigit() {
                                    return Err(self.err());
                                }
                                self.i += 1;
                            }
                        }
                        _ => return Err(self.err()),
                    }
                }
                c if c < 0x20 => {
                    self.i -= 1;
                    return Err(self.err());
                }
                _ => {}
            }
        }
    }
    fn number(&mut self) -> Result<(), E> {
        // a number ends at the first byte that cannot continue it; at the end of the data the decoder waits for more
        if self.peek()? == b'-' {
            self.i += 1;
        }
        match self.peek()? {
            b'0' => self.i += 1,
            b'1'..=b'9' => {
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            _ => return Err(self.err()),
        }
        let more = |p: &Self| p.i >= p.b.len();
        if more(self) {
            return Err(E::Eof);
        }
        if self.b[self.i] == b'.' {
            self.i += 1;
            if !self.peek()?.is_ascii_digit() {
                return Err(self.err());
            }
            while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if more(self) {
                return Err(E::Eof);
            }
        }
        if matches!(self.b[self.i], b'e' | b'E') {
            self.i += 1;
            if matches!(self.peek()?, b'+' | b'-') {
                self.i += 1;
            }
            if !self.peek()?.is_ascii_digit() {
                return Err(self.err());
            }
            while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if more(self) {
                return Err(E::Eof);
            }
        }
        Ok(())
    }
}

/// Scan the first value of `b` (`complete`: the body ends here; a top-level number at the end is then whole).
pub fn scan(b: &[u8], complete: bool) -> Scan {
    let mut p = P { b, i: 0 };
    p.skip_ws();
    if p.i >= b.len() {
        return Scan::Empty;
    }
    match p.value() {
        Ok(()) => Scan::Value(p.i),
        Err(E::Syn(o)) => Scan::Syntax(o),
        Err(E::Eof) => {
            // a top-level number that runs to the end of a complete body is a value
            if complete && matches!(b[b.iter().position(|c| !ws(*c)).unwrap_or(0)], b'-' | b'0'..=b'9') {
                let s = std::str::from_utf8(b).unwrap_or("").trim();
                if serde_json::from_str::<serde_json::Value>(s).is_ok() {
                    return Scan::Value(b.len());
                }
            }
            Scan::Incomplete
        }
    }
}

/// What a body read under a `MaxBytesReader(limit)` and decoded as one JSON value comes to.
#[derive(Debug, PartialEq)]
pub enum Body {
    /// The value's bytes.
    Ok(usize, usize),
    Empty,
    Syntax(usize),
    UnexpectedEof,
    TooLarge,
    /// Something after the value.
    Trailing,
}

/// `b` is the body as read, at most `limit + 1` bytes (one more than the limit means it is over).
pub fn body(b: &[u8], limit: usize) -> Body {
    let over = b.len() > limit;
    let data = &b[..b.len().min(limit)];
    match scan(data, !over) {
        Scan::Empty if over => Body::TooLarge,
        Scan::Empty => Body::Empty,
        Scan::Syntax(o) => Body::Syntax(o),
        Scan::Incomplete if over => Body::TooLarge,
        Scan::Incomplete => Body::UnexpectedEof,
        Scan::Value(end) => {
            let start = data.iter().position(|c| !ws(*c)).unwrap_or(0);
            let rest = &data[end..];
            if rest.iter().any(|c| !ws(*c)) {
                Body::Trailing
            } else if over {
                Body::TooLarge
            } else {
                Body::Ok(start, end)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        assert_eq!(scan(b"{\"a\":x}", true), Scan::Syntax(6));
        assert_eq!(scan(b"{\"a\":1", true), Scan::Incomplete);
        assert_eq!(scan(b"  ", true), Scan::Empty);
        assert_eq!(scan(b"{} x", true), Scan::Value(2));
        assert_eq!(scan(b"{\"a\":01}", true), Scan::Syntax(7));
        assert_eq!(body(b"{} {}", 100), Body::Trailing);
        assert_eq!(body(b"{\"a\":\"xxxx", 5), Body::TooLarge);
    }
}
