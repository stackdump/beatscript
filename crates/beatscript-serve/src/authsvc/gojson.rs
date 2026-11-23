//! `json.NewDecoder(http.MaxBytesReader(w, r.Body, max)).Decode(&v)` into a struct of string fields, as Go's
//! encoding/json does it, down to the error text (the 400 answer quotes it: `body must be JSON: <err>`).
//!
//! - The decoder reads one value and ignores whatever follows it (`{"a":1} junk` decodes).
//! - Nothing but whitespace: `EOF`. A value cut short: `unexpected EOF`. A syntax error: the scanner's
//!   `invalid character 'x' <context>`.
//! - A value not complete within the first `max` bytes of a longer body: the MaxBytesReader's error (Go's 413).
//! - Then the value is unmarshalled: keys match a field exactly, else case-insensitively (Go's fold); `null` sets
//!   nothing; any other type is `json: cannot unmarshal <kind> into Go struct field <Struct>.<field> of type
//!   string` (the first one is reported, after the whole value is read), and a top-level value that is not an
//!   object or null is `... into Go value of type <pkg.Struct>`.

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Go's `*http.MaxBytesError`.
    TooLarge,
    /// Anything else, with Go's message.
    Bad(String),
}

#[derive(Debug, Clone, PartialEq)]
enum V {
    Null,
    Bool,
    Num,
    Str(String),
    Arr,
    Obj(Vec<(String, V)>),
}

impl V {
    fn kind(&self) -> &'static str {
        match self {
            V::Null => "null",
            V::Bool => "bool",
            V::Num => "number",
            V::Str(_) => "string",
            V::Arr => "array",
            V::Obj(_) => "object",
        }
    }
}

/// strconv.Quote of the byte as a rune (U+0000..U+00FF), between single quotes: the scanner's `quoteChar`.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => return "'\\''".into(),
        b'"' => return "'\"'".into(),
        _ => {}
    }
    let s = match c {
        0x07 => "\\a".to_string(),
        0x08 => "\\b".to_string(),
        0x0c => "\\f".to_string(),
        b'\n' => "\\n".to_string(),
        b'\r' => "\\r".to_string(),
        b'\t' => "\\t".to_string(),
        0x0b => "\\v".to_string(),
        b'\\' => "\\\\".to_string(),
        0x00..=0x1f | 0x7f => format!("\\x{c:02x}"),
        0x80..=0xa0 | 0xad => format!("\\u{c:04x}"),
        _ => (c as char).to_string(),
    };
    format!("'{s}'")
}

fn syntax(c: u8, ctx: &str) -> Scan {
    Scan::Err(format!("invalid character {} {ctx}", quote_char(c)))
}

enum Scan {
    /// The value ends at this offset (exclusive).
    End(usize),
    /// A top-level scalar that is complete at the end of the bytes: a value only if no byte follows (EOF).
    EndAtEof(usize),
    /// Ran out of bytes: (whether anything but whitespace was seen).
    More(bool),
    Err(String),
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// Go's scanner over `b`, from the start, for one top-level value.
fn scan(b: &[u8]) -> Scan {
    #[derive(Clone, Copy, PartialEq)]
    enum P {
        ObjKey,
        ObjVal,
        Arr,
    }
    #[derive(Clone, Copy)]
    enum S {
        BeginValue,
        BeginValueOrEmpty,
        BeginStringOrEmpty,
        BeginString,
        EndValue,
        InString,
        Esc,
        U(u8),
        Neg,
        One,
        Zero,
        Dot,
        Dot0,
        E,
        ESign,
        E0,
        Lit(&'static [u8], &'static str, usize),
    }
    let mut st = S::BeginValue;
    let mut stack: Vec<P> = vec![];
    let mut seen = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if !is_space(c) {
            seen = true;
        }
        // one step of the state machine; `again` re-runs the byte in the next state (Go's tail calls)
        loop {
            match st {
                S::BeginValueOrEmpty | S::BeginValue => {
                    if is_space(c) {
                        break;
                    }
                    if matches!(st, S::BeginValueOrEmpty) && c == b']' {
                        stack.pop();
                        st = S::EndValue;
                        break;
                    }
                    st = match c {
                        b'{' => {
                            stack.push(P::ObjKey);
                            S::BeginStringOrEmpty
                        }
                        b'[' => {
                            stack.push(P::Arr);
                            S::BeginValueOrEmpty
                        }
                        b'"' => S::InString,
                        b'-' => S::Neg,
                        b'0' => S::Zero,
                        b'1'..=b'9' => S::One,
                        b't' => S::Lit(b"rue", "true", 0),
                        b'f' => S::Lit(b"alse", "false", 0),
                        b'n' => S::Lit(b"ull", "null", 0),
                        _ => return syntax(c, "looking for beginning of value"),
                    };
                    break;
                }
                S::BeginStringOrEmpty | S::BeginString => {
                    if is_space(c) {
                        break;
                    }
                    if matches!(st, S::BeginStringOrEmpty) && c == b'}' {
                        stack.pop();
                        st = S::EndValue;
                        break;
                    }
                    if c == b'"' {
                        st = S::InString;
                        break;
                    }
                    return syntax(c, "looking for beginning of object key string");
                }
                S::EndValue => {
                    let Some(&top) = stack.last() else {
                        // the top-level value ended before this byte
                        return Scan::End(i);
                    };
                    if is_space(c) {
                        break;
                    }
                    match top {
                        P::ObjKey => {
                            if c == b':' {
                                *stack.last_mut().unwrap() = P::ObjVal;
                                st = S::BeginValue;
                                break;
                            }
                            return syntax(c, "after object key");
                        }
                        P::ObjVal => {
                            if c == b',' {
                                *stack.last_mut().unwrap() = P::ObjKey;
                                st = S::BeginString;
                                break;
                            }
                            if c == b'}' {
                                stack.pop();
                                st = S::EndValue;
                                break;
                            }
                            return syntax(c, "after object key:value pair");
                        }
                        P::Arr => {
                            if c == b',' {
                                st = S::BeginValue;
                                break;
                            }
                            if c == b']' {
                                stack.pop();
                                st = S::EndValue;
                                break;
                            }
                            return syntax(c, "after array element");
                        }
                    }
                }
                S::InString => {
                    match c {
                        b'"' => st = S::EndValue,
                        b'\\' => st = S::Esc,
                        0..=0x1f => return syntax(c, "in string literal"),
                        _ => {}
                    }
                    break;
                }
                S::Esc => {
                    match c {
                        b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => st = S::InString,
                        b'u' => st = S::U(0),
                        _ => return syntax(c, "in string escape code"),
                    }
                    break;
                }
                S::U(n) => {
                    if !c.is_ascii_hexdigit() {
                        return syntax(c, "in \\u hexadecimal character escape");
                    }
                    st = if n == 3 { S::InString } else { S::U(n + 1) };
                    break;
                }
                S::Neg => {
                    st = match c {
                        b'0' => S::Zero,
                        b'1'..=b'9' => S::One,
                        _ => return syntax(c, "in numeric literal"),
                    };
                    break;
                }
                S::One => {
                    if c.is_ascii_digit() {
                        break;
                    }
                    st = S::Zero;
                    continue;
                }
                S::Zero => {
                    match c {
                        b'.' => st = S::Dot,
                        b'e' | b'E' => st = S::E,
                        _ => {
                            st = S::EndValue;
                            continue;
                        }
                    }
                    break;
                }
                S::Dot => {
                    if c.is_ascii_digit() {
                        st = S::Dot0;
                        break;
                    }
                    return syntax(c, "after decimal point in numeric literal");
                }
                S::Dot0 => {
                    if c.is_ascii_digit() {
                        break;
                    }
                    if c == b'e' || c == b'E' {
                        st = S::E;
                        break;
                    }
                    st = S::EndValue;
                    continue;
                }
                S::E => {
                    if c == b'+' || c == b'-' {
                        st = S::ESign;
                        break;
                    }
                    st = S::ESign;
                    continue;
                }
                S::ESign => {
                    if c.is_ascii_digit() {
                        st = S::E0;
                        break;
                    }
                    return syntax(c, "in exponent of numeric literal");
                }
                S::E0 => {
                    if c.is_ascii_digit() {
                        break;
                    }
                    st = S::EndValue;
                    continue;
                }
                S::Lit(rest, name, k) => {
                    if c != rest[k] {
                        return syntax(c, &format!("in literal {name} (expecting {})", quote_char(rest[k])));
                    }
                    st = if k + 1 == rest.len() { S::EndValue } else { S::Lit(rest, name, k + 1) };
                    break;
                }
            }
        }
        i += 1;
        // an object or array that closes the top level ends the value at once (Go invents a space byte rather than
        // wait for the next one)
        if stack.is_empty() && matches!(st, S::EndValue) && matches!(c, b'}' | b']') {
            return Scan::End(i);
        }
    }
    // at the end of the bytes: a top-level scalar that is complete ends here (Go feeds the scanner a space at EOF)
    if stack.is_empty() && matches!(st, S::EndValue | S::One | S::Zero | S::Dot0 | S::E0) {
        return Scan::EndAtEof(b.len());
    }
    Scan::More(seen)
}

/// The decoded value (the bytes are already known to be one valid JSON value).
struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && is_space(self.b[self.i]) {
            self.i += 1;
        }
    }
    fn value(&mut self) -> V {
        self.ws();
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut kv = vec![];
                loop {
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    if self.b[self.i] == b',' {
                        self.i += 1;
                        self.ws();
                    }
                    let k = self.string();
                    self.ws();
                    self.i += 1; // ':'
                    let v = self.value();
                    kv.push((k, v));
                }
                V::Obj(kv)
            }
            b'[' => {
                self.i += 1;
                let mut depth = 1usize;
                let mut in_str = false;
                while depth > 0 {
                    let c = self.b[self.i];
                    self.i += 1;
                    if in_str {
                        if c == b'\\' {
                            self.i += 1;
                        } else if c == b'"' {
                            in_str = false;
                        }
                        continue;
                    }
                    match c {
                        b'"' => in_str = true,
                        b'[' | b'{' => depth += 1,
                        b']' | b'}' => depth -= 1,
                        _ => {}
                    }
                }
                V::Arr
            }
            b'"' => V::Str(self.string()),
            b't' | b'f' => {
                self.i += if self.b[self.i] == b't' { 4 } else { 5 };
                V::Bool
            }
            b'n' => {
                self.i += 4;
                V::Null
            }
            _ => {
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                V::Num
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        let v = u32::from_str_radix(s, 16).ok()?;
        self.i += 4;
        Some(v)
    }

    /// Go's `unquote`: escapes, surrogate pairs (a lone surrogate is U+FFFD), invalid UTF-8 as U+FFFD per byte.
    fn string(&mut self) -> String {
        self.i += 1; // '"'
        let mut out = String::new();
        loop {
            let c = self.b[self.i];
            match c {
                b'"' => {
                    self.i += 1;
                    return out;
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let r = self.hex4().unwrap_or(0xfffd);
                            if (0xd800..0xdc00).contains(&r) {
                                // a high surrogate: a pair only when a \u low surrogate follows
                                let save = self.i;
                                if self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                                    self.i += 2;
                                    if let Some(r2) = self.hex4() {
                                        if (0xdc00..0xe000).contains(&r2) {
                                            out.push(char::from_u32(0x10000 + ((r - 0xd800) << 10) + (r2 - 0xdc00)).unwrap());
                                            continue;
                                        }
                                    }
                                }
                                self.i = save;
                                out.push('\u{fffd}');
                            } else {
                                out.push(char::from_u32(r).unwrap_or('\u{fffd}'));
                            }
                        }
                        other => out.push(other as char),
                    }
                }
                0..=0x7f => {
                    out.push(c as char);
                    self.i += 1;
                }
                _ => {
                    let n = match c {
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf7 => 4,
                        _ => 1,
                    };
                    match self.b.get(self.i..self.i + n).and_then(|s| std::str::from_utf8(s).ok()) {
                        Some(s) => {
                            out.push_str(s);
                            self.i += n;
                        }
                        None => {
                            out.push('\u{fffd}');
                            self.i += 1;
                        }
                    }
                }
            }
        }
    }
}

/// Go's `foldName` for the keys that can fold to ASCII: ASCII upper case, and the two non-ASCII runes whose fold
/// set holds an ASCII letter (U+017F long s, U+212A Kelvin sign).
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{17f}' => 'S',
            '\u{212a}' => 'K',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// Decode `body` as Go decodes it into `struct <type_name> { <fields> string }` (`type_name` is `pkg.Name`, as Go
/// prints the type). The fields come back in the order given; `None` is a field the value did not set.
pub fn decode_strings(body: &[u8], max: usize, type_name: &str, fields: &[&str]) -> Result<Vec<Option<String>>, DecodeError> {
    let within = &body[..body.len().min(max)];
    let end = match scan(within) {
        Scan::EndAtEof(n) => {
            // the reader would have to read one more byte past the limit to see the scalar end
            if body.len() > max {
                return Err(DecodeError::TooLarge);
            }
            n
        }
        Scan::End(n) => n,
        Scan::Err(m) => return Err(DecodeError::Bad(m)),
        Scan::More(seen) => {
            if body.len() > max {
                return Err(DecodeError::TooLarge);
            }
            return Err(DecodeError::Bad(if seen { "unexpected EOF" } else { "EOF" }.into()));
        }
    };
    let v = Parser { b: &body[..end], i: 0 }.value();
    let mut out: Vec<Option<String>> = vec![None; fields.len()];
    let short = type_name.rsplit('.').next().unwrap_or(type_name);
    let mut err: Option<String> = None;
    match v {
        V::Null => {}
        V::Obj(kv) => {
            for (k, val) in kv {
                let idx = fields.iter().position(|f| *f == k).or_else(|| {
                    let fk = fold(&k);
                    fields.iter().position(|f| fold(f) == fk)
                });
                let Some(idx) = idx else { continue };
                match val {
                    V::Str(s) => out[idx] = Some(s),
                    V::Null => {}
                    other => {
                        if err.is_none() {
                            err = Some(format!("json: cannot unmarshal {} into Go struct field {short}.{} of type string", other.kind(), fields[idx]));
                        }
                    }
                }
            }
        }
        other => err = Some(format!("json: cannot unmarshal {} into Go value of type {type_name}", other.kind())),
    }
    match err {
        Some(e) => Err(DecodeError::Bad(e)),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(b: &str) -> Result<Vec<Option<String>>, DecodeError> {
        decode_strings(b.as_bytes(), 4096, "auth.loginReq", &["type", "address", "signature", "nonce"])
    }
    fn bad(b: &str) -> String {
        match d(b) {
            Err(DecodeError::Bad(m)) => m,
            x => panic!("{b:?}: {x:?}"),
        }
    }

    /// Each message is what Go 1.26's decoder says for the same body (checked against the Go server).
    #[test]
    fn go_messages() {
        assert_eq!(bad(""), "EOF");
        assert_eq!(bad("  \n"), "EOF");
        assert_eq!(bad("{"), "unexpected EOF");
        assert_eq!(bad("{\"type\":\"eth"), "unexpected EOF");
        assert_eq!(bad("nonce=1"), "invalid character 'o' in literal null (expecting 'u')");
        assert_eq!(bad("x"), "invalid character 'x' looking for beginning of value");
        assert_eq!(bad("{x"), "invalid character 'x' looking for beginning of object key string");
        assert_eq!(bad("{\"a\" 1"), "invalid character '1' after object key");
        assert_eq!(bad("{\"a\":1 2"), "invalid character '2' after object key:value pair");
        assert_eq!(bad("{\"a\":[1 2"), "invalid character '2' after array element");
        assert_eq!(bad("{\"a\":\"\n\"}"), "invalid character '\\n' in string literal");
        assert_eq!(bad("{\"a\":\"\\x\"}"), "invalid character 'x' in string escape code");
        assert_eq!(bad("{\"a\":\"\\u12g4\"}"), "invalid character 'g' in \\u hexadecimal character escape");
        assert_eq!(bad("{\"a\":-x}"), "invalid character 'x' in numeric literal");
        assert_eq!(bad("{\"a\":1.x}"), "invalid character 'x' after decimal point in numeric literal");
        assert_eq!(bad("{\"a\":1ex}"), "invalid character 'x' in exponent of numeric literal");
        assert_eq!(bad("{\"a\":tx}"), "invalid character 'x' in literal true (expecting 'r')");
        assert_eq!(bad("{'a':1}"), "invalid character '\\'' looking for beginning of object key string");
        assert_eq!(bad("{\"a\",}"), "invalid character ',' after object key");
        assert_eq!(bad("\u{1}"), "invalid character '\\x01' looking for beginning of value");
        assert_eq!(bad("[1,2]"), "json: cannot unmarshal array into Go value of type auth.loginReq");
        assert_eq!(bad("\"s\""), "json: cannot unmarshal string into Go value of type auth.loginReq");
        assert_eq!(bad("12"), "json: cannot unmarshal number into Go value of type auth.loginReq");
        assert_eq!(bad("12x"), "json: cannot unmarshal number into Go value of type auth.loginReq");
        assert_eq!(bad("true"), "json: cannot unmarshal bool into Go value of type auth.loginReq");
        assert_eq!(bad("{\"type\":1,\"nonce\":true}"), "json: cannot unmarshal number into Go struct field loginReq.type of type string");
        assert_eq!(bad("{\"nonce\":{}}"), "json: cannot unmarshal object into Go struct field loginReq.nonce of type string");
    }

    #[test]
    fn decodes_like_go() {
        assert_eq!(d("null").unwrap(), vec![None, None, None, None]);
        assert_eq!(d("{}").unwrap(), vec![None, None, None, None]);
        let v = d(r#"{"TYPE":"eth","Address":"a","x":{"y":[1,{"z":"]"}]},"nonce":"n1","nonce":"n2","signature":null} trailing junk"#).unwrap();
        assert_eq!(v, vec![Some("eth".into()), Some("a".into()), None, Some("n2".into())]);
        let v = d("{\"\u{17f}ignature\":\"\\u0041\\ud83d\\ude00\\ud800x\"}").unwrap();
        assert_eq!(v[2].as_deref(), Some("A\u{1f600}\u{fffd}x"));
    }

    #[test]
    fn size_limit() {
        let big = format!("{{\"nonce\":\"{}\"}}", "a".repeat(10000));
        assert_eq!(d(&big), Err(DecodeError::TooLarge));
        // a value that is complete inside the limit is read, whatever follows
        let ok = format!("{{\"nonce\":\"n\"}}{}", " ".repeat(10000));
        assert_eq!(d(&ok).unwrap()[3].as_deref(), Some("n"));
        // a syntax error inside the limit is a 400 even when the body is long
        assert_eq!(d(&format!("{{x{}", " ".repeat(10000))), Err(DecodeError::Bad("invalid character 'x' looking for beginning of object key string".into())));
        // a number at the end of the limit needs the next byte to end
        let num = format!("{}{}", " ".repeat(4094), "12");
        assert!(matches!(decode_strings(format!("{num}3").as_bytes(), 4096, "auth.loginReq", &["type"]), Err(DecodeError::TooLarge)));
        assert!(matches!(decode_strings(num.as_bytes(), 4096, "auth.loginReq", &["type"]), Err(DecodeError::Bad(_))));
    }
}
