//! encoding/json, where it shows on the wire: the encoder's output (HTML-safe escaping, U+2028/2029, `\u00XX` for
//! control characters, float64 formatting), and `json.NewDecoder(http.MaxBytesReader(...)).Decode(&struct)` on a
//! request body, with the scanner's error messages word for word (a client sees them in the 400's message).

use serde_json::Value;

// ---------- the encoder ----------

/// `json.NewEncoder(w).Encode(v)` minus the newline: keys in the order the map holds them (build Go's struct order,
/// or sort for a Go map).
pub fn encode(v: &Value) -> String {
    let mut s = String::new();
    write(&mut s, v);
    s
}

fn write(s: &mut String, v: &Value) {
    match v {
        Value::Null => s.push_str("null"),
        Value::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                s.push_str(&float(n.as_f64().unwrap()));
            } else {
                s.push_str(&n.to_string());
            }
        }
        Value::String(t) => string(s, t),
        Value::Array(a) => {
            s.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                write(s, x);
            }
            s.push(']');
        }
        Value::Object(m) => {
            s.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                string(s, k);
                s.push(':');
                write(s, x);
            }
            s.push('}');
        }
    }
}

/// encoding/json's float64: the shortest representation that round-trips, `'f'` form, or `'e'` below 1e-6 and from
/// 1e21 (with `e-07` cleaned to `e-7`).
pub fn float(f: f64) -> String {
    if !f.is_finite() {
        return "null".into();
    }
    let a = f.abs();
    if a != 0.0 && (a < 1e-6 || a >= 1e21) {
        let e = format!("{f:e}"); // 1e-7, 1.5e21
        let (m, x) = e.split_once('e').unwrap();
        let x = if x.starts_with('-') { x.to_string() } else { format!("+{x}") };
        return format!("{m}e{x}");
    }
    let s = format!("{f}");
    if s == "-0" {
        return "-0".into();
    }
    s
}

fn string(s: &mut String, t: &str) {
    s.push('"');
    for c in t.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\x08' => s.push_str("\\b"), // go1.22+
            '\x0c' => s.push_str("\\f"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => s.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
}

// ---------- the decoder ----------

pub enum DecodeError {
    /// http.MaxBytesError: the body is over the limit
    TooLarge,
    /// the error's text, as `%v` prints it
    Bad(String),
}

/// strconv-style quoting of one byte as the scanner names it in an error (`quoteChar`).
pub fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        _ => {
            let ch = char::from(c); // string(byte) in Go is the rune U+00XX
            let inner = match ch {
                '\x07' => "\\a".into(),
                '\x08' => "\\b".into(),
                '\x0c' => "\\f".into(),
                '\n' => "\\n".into(),
                '\r' => "\\r".into(),
                '\t' => "\\t".into(),
                '\x0b' => "\\v".into(),
                '\\' => "\\\\".into(),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => format!("\\x{:02x}", c as u32),
                // not printable in strconv.IsPrint: C1 controls, NBSP, soft hyphen
                c if (0x80..0xa0).contains(&(c as u32)) || c as u32 == 0xa0 || c as u32 == 0xad => format!("\\u{:04x}", c as u32),
                c => c.to_string(),
            };
            format!("'{inner}'")
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum St {
    BeginValue,
    BeginValueOrEmpty,
    BeginStringOrEmpty,
    BeginString,
    EndValue,
    EndTop,
    InString,
    InStringEsc,
    EscU(u8),
    Neg,
    One,
    Zero,
    Dot,
    Dot0,
    E,
    ESign,
    E0,
    Lit(&'static [u8], usize, &'static str), // the literal, the next byte's index, its name
    Error,
}

#[derive(Clone, Copy, PartialEq)]
enum Ps {
    ObjectKey,
    ObjectValue,
    ArrayValue,
}

#[derive(PartialEq)]
enum Op {
    Continue,
    End,
    EndCompound,
    Error,
}

/// encoding/json's scanner, byte by byte.
struct Scanner {
    step: St,
    ps: Vec<Ps>,
    err: Option<String>,
}

const MAX_DEPTH: usize = 10000;

impl Scanner {
    fn error(&mut self, c: u8, ctx: &str) -> Op {
        self.step = St::Error;
        self.err = Some(format!("invalid character {} {ctx}", quote_char(c)));
        Op::Error
    }

    fn push(&mut self, p: Ps) -> Op {
        self.ps.push(p);
        if self.ps.len() > MAX_DEPTH {
            self.step = St::Error;
            self.err = Some("exceeded max depth".into());
            return Op::Error;
        }
        Op::Continue
    }

    fn step(&mut self, c: u8) -> Op {
        let sp = matches!(c, b' ' | b'\t' | b'\r' | b'\n');
        match self.step {
            St::Error => Op::Error,
            St::BeginValueOrEmpty => {
                if sp {
                    return Op::Continue;
                }
                if c == b']' {
                    return self.end_value(c);
                }
                self.begin_value(c)
            }
            St::BeginValue => self.begin_value(c),
            St::BeginStringOrEmpty => {
                if sp {
                    return Op::Continue;
                }
                if c == b'}' {
                    let n = self.ps.len();
                    self.ps[n - 1] = Ps::ObjectValue;
                    return self.end_value(c);
                }
                self.begin_string(c)
            }
            St::BeginString => self.begin_string(c),
            St::EndValue => self.end_value(c),
            St::EndTop => {
                if !sp {
                    self.error(c, "after top-level value");
                }
                Op::End
            }
            St::InString => {
                if c == b'"' {
                    self.step = St::EndValue;
                    return Op::Continue;
                }
                if c == b'\\' {
                    self.step = St::InStringEsc;
                    return Op::Continue;
                }
                if c < 0x20 {
                    return self.error(c, "in string literal");
                }
                Op::Continue
            }
            St::InStringEsc => match c {
                b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => {
                    self.step = St::InString;
                    Op::Continue
                }
                b'u' => {
                    self.step = St::EscU(0);
                    Op::Continue
                }
                _ => self.error(c, "in string escape code"),
            },
            St::EscU(k) => {
                if c.is_ascii_hexdigit() {
                    self.step = if k == 3 { St::InString } else { St::EscU(k + 1) };
                    return Op::Continue;
                }
                self.error(c, "in \\u hexadecimal character escape")
            }
            St::Neg => match c {
                b'0' => {
                    self.step = St::Zero;
                    Op::Continue
                }
                b'1'..=b'9' => {
                    self.step = St::One;
                    Op::Continue
                }
                _ => self.error(c, "in numeric literal"),
            },
            St::One => {
                if c.is_ascii_digit() {
                    return Op::Continue;
                }
                self.zero(c)
            }
            St::Zero => self.zero(c),
            St::Dot => {
                if c.is_ascii_digit() {
                    self.step = St::Dot0;
                    return Op::Continue;
                }
                self.error(c, "after decimal point in numeric literal")
            }
            St::Dot0 => {
                if c.is_ascii_digit() {
                    return Op::Continue;
                }
                if c == b'e' || c == b'E' {
                    self.step = St::E;
                    return Op::Continue;
                }
                self.end_value(c)
            }
            St::E => {
                if c == b'+' || c == b'-' {
                    self.step = St::ESign;
                    return Op::Continue;
                }
                self.esign(c)
            }
            St::ESign => self.esign(c),
            St::E0 => {
                if c.is_ascii_digit() {
                    return Op::Continue;
                }
                self.end_value(c)
            }
            St::Lit(lit, i, name) => {
                if c == lit[i] {
                    self.step = if i + 1 == lit.len() { St::EndValue } else { St::Lit(lit, i + 1, name) };
                    return Op::Continue;
                }
                let ctx = format!("in literal {name} (expecting {})", quote_char(lit[i]));
                self.error(c, &ctx)
            }
        }
    }

    fn zero(&mut self, c: u8) -> Op {
        if c == b'.' {
            self.step = St::Dot;
            return Op::Continue;
        }
        if c == b'e' || c == b'E' {
            self.step = St::E;
            return Op::Continue;
        }
        self.end_value(c)
    }

    fn esign(&mut self, c: u8) -> Op {
        if c.is_ascii_digit() {
            self.step = St::E0;
            return Op::Continue;
        }
        self.error(c, "in exponent of numeric literal")
    }

    fn begin_value(&mut self, c: u8) -> Op {
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
            return Op::Continue;
        }
        match c {
            b'{' => {
                self.step = St::BeginStringOrEmpty;
                self.push(Ps::ObjectKey)
            }
            b'[' => {
                self.step = St::BeginValueOrEmpty;
                self.push(Ps::ArrayValue)
            }
            b'"' => {
                self.step = St::InString;
                Op::Continue
            }
            b'-' => {
                self.step = St::Neg;
                Op::Continue
            }
            b'0' => {
                self.step = St::Zero;
                Op::Continue
            }
            b't' => {
                self.step = St::Lit(b"true", 1, "true");
                Op::Continue
            }
            b'f' => {
                self.step = St::Lit(b"false", 1, "false");
                Op::Continue
            }
            b'n' => {
                self.step = St::Lit(b"null", 1, "null");
                Op::Continue
            }
            b'1'..=b'9' => {
                self.step = St::One;
                Op::Continue
            }
            _ => self.error(c, "looking for beginning of value"),
        }
    }

    fn begin_string(&mut self, c: u8) -> Op {
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
            return Op::Continue;
        }
        if c == b'"' {
            self.step = St::InString;
            return Op::Continue;
        }
        self.error(c, "looking for beginning of object key string")
    }

    fn end_value(&mut self, c: u8) -> Op {
        let n = self.ps.len();
        if n == 0 {
            self.step = St::EndTop;
            return self.step(c);
        }
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
            self.step = St::EndValue;
            return Op::Continue;
        }
        match self.ps[n - 1] {
            Ps::ObjectKey => {
                if c == b':' {
                    self.ps[n - 1] = Ps::ObjectValue;
                    self.step = St::BeginValue;
                    return Op::Continue;
                }
                self.error(c, "after object key")
            }
            Ps::ObjectValue => {
                if c == b',' {
                    self.ps[n - 1] = Ps::ObjectKey;
                    self.step = St::BeginString;
                    return Op::Continue;
                }
                if c == b'}' {
                    self.ps.pop();
                    self.step = if self.ps.is_empty() { St::EndTop } else { St::EndValue };
                    return Op::EndCompound;
                }
                self.error(c, "after object key:value pair")
            }
            Ps::ArrayValue => {
                if c == b',' {
                    self.step = St::BeginValue;
                    return Op::Continue;
                }
                if c == b']' {
                    self.ps.pop();
                    self.step = if self.ps.is_empty() { St::EndTop } else { St::EndValue };
                    return Op::EndCompound;
                }
                self.error(c, "after array element")
            }
        }
    }
}

/// `Decoder.readValue`: the length of the first JSON value in `buf`, as the decoder finds it when the body is
/// `buf` (`over` = the body went on past the limit: MaxBytesReader's error arrives with the last bytes).
fn read_value(buf: &[u8], over: bool) -> Result<usize, DecodeError> {
    let mut s = Scanner { step: St::BeginValue, ps: Vec::new(), err: None };
    for (i, &c) in buf.iter().enumerate() {
        match s.step(c) {
            Op::End => return Ok(i),
            Op::EndCompound => {
                // scanEnd is delayed one byte: the decoder invents a space rather than wait for it
                if s.ps.is_empty() {
                    return Ok(i + 1);
                }
            }
            Op::Error => return Err(DecodeError::Bad(s.err.unwrap_or_default())),
            Op::Continue => {}
        }
    }
    if over {
        return Err(DecodeError::TooLarge);
    }
    if s.step(b' ') == Op::End {
        return Ok(buf.len());
    }
    if buf.iter().any(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n')) {
        return Err(DecodeError::Bad("unexpected EOF".into()));
    }
    Err(DecodeError::Bad("EOF".into()))
}

/// A parsed (already scanned, so valid) JSON value, with strings decoded as Go decodes them.
enum J {
    Null,
    Bool,
    Num,
    Str(String),
    Arr,
    Obj(Vec<(String, J)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn value(&mut self, keep: bool) -> J {
        self.ws();
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        return J::Obj(out);
                    }
                    if self.b[self.i] == b',' {
                        self.i += 1;
                        self.ws();
                    }
                    let k = self.string();
                    self.ws();
                    self.i += 1; // ':'
                    let v = self.value(keep);
                    if keep {
                        out.push((k, v));
                    }
                }
            }
            b'[' => {
                self.i += 1;
                loop {
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        return J::Arr;
                    }
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                    self.value(false);
                }
            }
            b'"' => J::Str(self.string()),
            b't' => {
                self.i += 4;
                J::Bool
            }
            b'f' => {
                self.i += 5;
                J::Bool
            }
            b'n' => {
                self.i += 4;
                J::Null
            }
            _ => {
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                J::Num
            }
        }
    }

    /// `unquote`: escapes, surrogate pairs (a lone one is U+FFFD), and each invalid UTF-8 byte as U+FFFD.
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
                    let e = self.b[self.i + 1];
                    self.i += 2;
                    match e {
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0c'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let r = self.hex4();
                            if (0xd800..0xdc00).contains(&r) && self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                                let save = self.i;
                                self.i += 2;
                                let r2 = self.hex4();
                                if (0xdc00..0xe000).contains(&r2) {
                                    out.push(char::from_u32(0x10000 + ((r - 0xd800) << 10) + (r2 - 0xdc00)).unwrap_or('\u{fffd}'));
                                    continue;
                                }
                                self.i = save;
                                out.push('\u{fffd}');
                                continue;
                            }
                            out.push(char::from_u32(r).unwrap_or('\u{fffd}'));
                        }
                        other => out.push(char::from(other)),
                    }
                }
                c if c < 0x80 => {
                    out.push(char::from(c));
                    self.i += 1;
                }
                _ => {
                    let (ch, n) = decode_rune(&self.b[self.i..]);
                    out.push(ch);
                    self.i += n;
                }
            }
        }
    }

    fn hex4(&mut self) -> u32 {
        let s = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap_or("0");
        self.i += 4;
        u32::from_str_radix(s, 16).unwrap_or(0xfffd)
    }
}

/// `utf8.DecodeRune`: one rune, or (U+FFFD, 1) for an invalid byte.
fn decode_rune(b: &[u8]) -> (char, usize) {
    let n = match b[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return ('\u{fffd}', 1),
    };
    if b.len() >= n {
        if let Ok(s) = std::str::from_utf8(&b[..n]) {
            return (s.chars().next().unwrap(), n);
        }
    }
    ('\u{fffd}', 1)
}

/// Go's case-insensitive field match (after the exact one).
fn fold_eq(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_lowercase).eq(b.chars().flat_map(char::to_lowercase))
}

/// `json.NewDecoder(MaxBytesReader(body, max)).Decode(&v)` where v is a struct of string fields `fields` (JSON
/// names). `go_type` is the struct's type as `%v` prints it (`server.textBody`) and `struct_name` its name in a field
/// error (`textBody`; "" for an anonymous struct). `body` holds at most `max + 1` bytes of the request.
pub fn decode_strings(body: &[u8], max: usize, fields: &[&str], go_type: &str, struct_name: &str) -> Result<Vec<String>, DecodeError> {
    let over = body.len() > max;
    let buf = &body[..body.len().min(max)];
    let n = read_value(buf, over)?;
    let mut p = P { b: &buf[..n], i: 0 };
    let v = p.value(true);
    let mut out = vec![String::new(); fields.len()];
    let mut first_err: Option<String> = None;
    let kind = |j: &J| match j {
        J::Bool => "bool",
        J::Num => "number",
        J::Str(_) => "string",
        J::Arr => "array",
        J::Obj(_) => "object",
        J::Null => "null",
    };
    match v {
        J::Null => {}
        J::Obj(kv) => {
            for (k, val) in kv {
                let idx = fields.iter().position(|f| *f == k).or_else(|| fields.iter().position(|f| fold_eq(f, &k)));
                let Some(idx) = idx else { continue };
                match val {
                    J::Null => {}
                    J::Str(s) => out[idx] = s,
                    other => {
                        if first_err.is_none() {
                            first_err = Some(format!("json: cannot unmarshal {} into Go struct field {struct_name}.{} of type string", kind(&other), fields[idx]));
                        }
                    }
                }
            }
        }
        other => return Err(DecodeError::Bad(format!("json: cannot unmarshal {} into Go value of type {go_type}", kind(&other)))),
    }
    match first_err {
        Some(e) => Err(DecodeError::Bad(e)),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(b: &str) -> String {
        match decode_strings(b.as_bytes(), 1 << 17, &["text", "url"], "server.textBody", "textBody") {
            Ok(v) => format!("ok {v:?}"),
            Err(DecodeError::TooLarge) => "too large".into(),
            Err(DecodeError::Bad(m)) => m,
        }
    }

    #[test]
    fn go_messages() {
        // what the Go server answers (go1.26, probed against btwsite)
        for (b, want) in [
            ("", "EOF"),
            ("   ", "EOF"),
            ("{", "unexpected EOF"),
            ("xyz", "invalid character 'x' looking for beginning of value"),
            ("[]", "json: cannot unmarshal array into Go value of type server.textBody"),
            ("\"s\"", "json: cannot unmarshal string into Go value of type server.textBody"),
            ("5", "json: cannot unmarshal number into Go value of type server.textBody"),
            ("5x", "json: cannot unmarshal number into Go value of type server.textBody"),
            ("true", "json: cannot unmarshal bool into Go value of type server.textBody"),
            ("{\"text\":5}", "json: cannot unmarshal number into Go struct field textBody.text of type string"),
            ("{\"text\":{\"a\":1}}", "json: cannot unmarshal object into Go struct field textBody.text of type string"),
            ("{\"text\":true,\"url\":5}", "json: cannot unmarshal bool into Go struct field textBody.text of type string"),
            ("{\"text\" 1}", "invalid character '1' after object key"),
            ("{\"text\":1 2}", "invalid character '2' after object key:value pair"),
            ("{\"a\":tru}", "invalid character '}' in literal true (expecting 'e')"),
            ("{\"text\":\"\\u12\"}", "invalid character '\"' in \\u hexadecimal character escape"),
            ("{\"text\":\"\\q\"}", "invalid character 'q' in string escape code"),
            ("[1,2", "unexpected EOF"),
            ("-", "unexpected EOF"),
            ("{,}", "invalid character ',' looking for beginning of object key string"),
            ("{'a':1}", "invalid character '\\'' looking for beginning of object key string"),
            ("{\"text\":-}", "invalid character '}' in numeric literal"),
            ("{\"text\":01}", "invalid character '1' after object key:value pair"),
            ("{\"text\":nul}", "invalid character '}' in literal null (expecting 'l')"),
            ("{\"text\":1.}", "invalid character '}' after decimal point in numeric literal"),
            ("{\"text\":1e+}", "invalid character '}' in exponent of numeric literal"),
            ("{\"text\":\"a\x01\"}", "invalid character '\\x01' in string literal"),
            ("\x7f", "invalid character '\\x7f' looking for beginning of value"),
        ] {
            assert_eq!(msg(b), want, "{b:?}");
        }
        assert_eq!(msg("null"), "ok [\"\", \"\"]");
        assert_eq!(msg("{\"TEXT\":\"a b\"} junk"), "ok [\"a b\", \"\"]");
        assert_eq!(msg("{\"text\":\"\\ud800x\\u00e9\"}"), "ok [\"\u{fffd}x\u{e9}\", \"\"]");
        assert_eq!(quote_char(0xa0), "'\\u00a0'");
    }

    #[test]
    fn floats() {
        assert_eq!(float(120.0), "120");
        assert_eq!(float(127.5), "127.5");
        assert_eq!(float(1e-7), "1e-7");
        assert_eq!(float(1e21), "1e+21");
        assert_eq!(float(0.000001), "0.000001");
        assert_eq!(encode(&serde_json::json!({"a": "<\u{2028}\x08\x1f"})), "{\"a\":\"\\u003c\\u2028\\b\\u001f\"}");
    }
}
