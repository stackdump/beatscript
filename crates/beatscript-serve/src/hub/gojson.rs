//! encoding/json as the Go hub uses it, where it shows on the wire: the error messages a bad body gets
//! (`json.NewDecoder(r.Body).Decode(&v)` and `json.Unmarshal`), Go's binding rules (unknown keys ignored, keys
//! matched exactly then case-insensitively, the last duplicate wins, `null` leaves a value alone, the first type
//! error is the one reported), `json.RawMessage` fields kept as their input bytes, and the writer side
//! (`json.Marshal`'s escaping, compaction of raw values).

/// A parsed JSON value with the byte span it came from (a `json.RawMessage` is that span).
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Null,
    Bool(bool),
    /// the literal as written
    Num(String),
    Str(String),
    Arr(Vec<Node>),
    Obj(Vec<(String, Node)>),
}

impl Kind {
    /// What `UnmarshalTypeError.Value` calls a JSON value of this kind (a number is "number", or "number <lit>" when
    /// it does not fit the field: see `num_err`).
    fn word(&self) -> &'static str {
        match self {
            Kind::Null => "null",
            Kind::Bool(_) => "bool",
            Kind::Num(_) => "number",
            Kind::Str(_) => "string",
            Kind::Arr(_) => "array",
            Kind::Obj(_) => "object",
        }
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// Go's `quoteChar`.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        _ => {
            let ch = c as char; // string(c): the byte as a rune (U+0000..U+00FF)
            let printable = !(ch.is_control() || (0x7f..=0x9f).contains(&(c as u32)) || c == 0xad);
            if printable {
                format!("'{ch}'")
            } else if c < 0x80 {
                match c {
                    0x07 => "'\\a'".into(),
                    0x08 => "'\\b'".into(),
                    0x0c => "'\\f'".into(),
                    b'\n' => "'\\n'".into(),
                    b'\r' => "'\\r'".into(),
                    b'\t' => "'\\t'".into(),
                    0x0b => "'\\v'".into(),
                    _ => format!("'\\x{c:02x}'"),
                }
            } else {
                format!("'\\u{:04x}'", c as u32)
            }
        }
    }
}

enum PErr {
    /// ran out of input inside a value
    Eof,
    Syntax(String),
}

fn syn(c: u8, ctx: &str) -> PErr {
    PErr::Syntax(format!("invalid character {} {ctx}", quote_char(c)))
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && is_space(self.b[self.i]) {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn value(&mut self, depth: usize) -> Result<Node, PErr> {
        self.ws();
        let start = self.i;
        let c = self.peek().ok_or(PErr::Eof)?;
        if depth > 10000 {
            return Err(PErr::Syntax("exceeded max depth".into()));
        }
        let kind = match c {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                match self.peek().ok_or(PErr::Eof)? {
                    b'}' => {
                        self.i += 1;
                    }
                    b'"' => loop {
                        let k = self.string()?;
                        self.ws();
                        match self.peek().ok_or(PErr::Eof)? {
                            b':' => self.i += 1,
                            c => return Err(syn(c, "after object key")),
                        }
                        let v = self.value(depth + 1)?;
                        fields.push((k, v));
                        self.ws();
                        match self.peek().ok_or(PErr::Eof)? {
                            b',' => {
                                self.i += 1;
                                self.ws();
                                match self.peek().ok_or(PErr::Eof)? {
                                    b'"' => continue,
                                    c => return Err(syn(c, "looking for beginning of object key string")),
                                }
                            }
                            b'}' => {
                                self.i += 1;
                                break;
                            }
                            c => return Err(syn(c, "after object key:value pair")),
                        }
                    },
                    c => return Err(syn(c, "looking for beginning of object key string")),
                }
                Kind::Obj(fields)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek().ok_or(PErr::Eof)? == b']' {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value(depth + 1)?);
                        self.ws();
                        match self.peek().ok_or(PErr::Eof)? {
                            b',' => self.i += 1,
                            b']' => {
                                self.i += 1;
                                break;
                            }
                            c => return Err(syn(c, "after array element")),
                        }
                    }
                }
                Kind::Arr(items)
            }
            b'"' => Kind::Str(self.string()?),
            b'-' | b'0'..=b'9' => Kind::Num(self.number()?),
            b't' => {
                self.lit("true")?;
                Kind::Bool(true)
            }
            b'f' => {
                self.lit("false")?;
                Kind::Bool(false)
            }
            b'n' => {
                self.lit("null")?;
                Kind::Null
            }
            c => return Err(syn(c, "looking for beginning of value")),
        };
        Ok(Node { kind, start, end: self.i })
    }
    fn lit(&mut self, w: &str) -> Result<(), PErr> {
        let wb = w.as_bytes();
        self.i += 1;
        for &want in &wb[1..] {
            let c = self.peek().ok_or(PErr::Eof)?;
            if c != want {
                return Err(syn(c, &format!("in literal {w} (expecting {})", quote_char(want))));
            }
            self.i += 1;
        }
        Ok(())
    }
    fn number(&mut self) -> Result<String, PErr> {
        let s = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
            match self.peek().ok_or(PErr::Eof)? {
                b'0'..=b'9' => {}
                c => return Err(syn(c, "in numeric literal")),
            }
        }
        if self.peek() == Some(b'0') {
            self.i += 1;
        } else {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            match self.peek().ok_or(PErr::Eof)? {
                b'0'..=b'9' => {}
                c => return Err(syn(c, "after decimal point in numeric literal")),
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            match self.peek().ok_or(PErr::Eof)? {
                b'0'..=b'9' => {}
                c => return Err(syn(c, "in exponent of numeric literal")),
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        Ok(String::from_utf8_lossy(&self.b[s..self.i]).into_owned())
    }
    /// A string literal, unquoted as Go's `unquote` does (invalid UTF-8 and lone surrogates become U+FFFD).
    fn string(&mut self) -> Result<String, PErr> {
        self.i += 1; // the quote
        let s = self.i;
        let mut simple = true;
        loop {
            let c = self.peek().ok_or(PErr::Eof)?;
            match c {
                b'"' => break,
                b'\\' => {
                    simple = false;
                    self.i += 1;
                    let e = self.peek().ok_or(PErr::Eof)?;
                    match e {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.i += 1,
                        b'u' => {
                            self.i += 1;
                            for _ in 0..4 {
                                let h = self.peek().ok_or(PErr::Eof)?;
                                if !h.is_ascii_hexdigit() {
                                    return Err(syn(h, "in \\u hexadecimal character escape"));
                                }
                                self.i += 1;
                            }
                        }
                        c => return Err(syn(c, "in string escape code")),
                    }
                }
                c if c < 0x20 => return Err(syn(c, "in string literal")),
                c => {
                    if c >= 0x80 {
                        simple = false;
                    }
                    self.i += 1;
                }
            }
        }
        let raw = &self.b[s..self.i];
        self.i += 1;
        if simple {
            return Ok(String::from_utf8_lossy(raw).into_owned());
        }
        Ok(unquote(raw))
    }
}

fn hex4(b: &[u8]) -> u32 {
    u32::from_str_radix(std::str::from_utf8(&b[..4]).unwrap_or("0"), 16).unwrap_or(0)
}

/// The body of a valid string literal (between the quotes), as Go decodes it.
fn unquote(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' {
            let e = b[i + 1];
            i += 2;
            match e {
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let r = hex4(&b[i..]);
                    i += 4;
                    if (0xd800..0xe000).contains(&r) {
                        if r < 0xdc00 && i + 6 <= b.len() && b[i] == b'\\' && b[i + 1] == b'u' {
                            let r2 = hex4(&b[i + 2..]);
                            if (0xdc00..0xe000).contains(&r2) {
                                let cp = 0x10000 + ((r - 0xd800) << 10) + (r2 - 0xdc00);
                                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                i += 6;
                                continue;
                            }
                        }
                        out.push('\u{fffd}');
                    } else {
                        out.push(char::from_u32(r).unwrap_or('\u{fffd}'));
                    }
                }
                other => out.push(other as char),
            }
            continue;
        }
        if c < 0x80 {
            out.push(c as char);
            i += 1;
            continue;
        }
        // one UTF-8 sequence, or U+FFFD for one bad byte (utf8.DecodeRune)
        let n = match c {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        };
        if n > 0 && i + n <= b.len() {
            if let Ok(s) = std::str::from_utf8(&b[i..i + n]) {
                out.push_str(s);
                i += n;
                continue;
            }
        }
        out.push('\u{fffd}');
        i += 1;
    }
    out
}

/// `json.NewDecoder(body).Decode(&v)`: the first value of `b`; whatever follows it is not looked at.
pub fn decode_first(b: &[u8]) -> Result<Node, String> {
    let mut p = P { b, i: 0 };
    p.ws();
    if p.i == b.len() {
        return Err("EOF".into());
    }
    match p.value(0) {
        Ok(n) => Ok(n),
        Err(PErr::Eof) => Err("unexpected EOF".into()),
        Err(PErr::Syntax(m)) => Err(m),
    }
}

/// `json.Unmarshal(b, &v)`: the whole input must be one value (and only space after it).
pub fn unmarshal(b: &[u8]) -> Result<Node, String> {
    let mut p = P { b, i: 0 };
    let n = match p.value(0) {
        Ok(n) => n,
        Err(PErr::Eof) => return Err("unexpected end of JSON input".into()),
        Err(PErr::Syntax(m)) => return Err(m),
    };
    p.ws();
    if let Some(c) = p.peek() {
        return Err(syn(c, "after top-level value").into_msg());
    }
    Ok(n)
}

impl PErr {
    fn into_msg(self) -> String {
        match self {
            PErr::Eof => "unexpected EOF".into(),
            PErr::Syntax(m) => m,
        }
    }
}

/// The body of a `decodeBody(w, r, max, &v)`: what the Decoder sees when the body is capped at `max` bytes by
/// `http.MaxBytesReader`. `Err(None)`: the cap was hit before the value ended (413); `Err(Some(m))`: a JSON error.
pub fn decode_capped(body: &[u8], max: usize) -> Result<Node, Option<String>> {
    if body.len() <= max {
        return decode_first(body).map_err(Some);
    }
    // the reader gives max bytes, then fails: a value (or a syntax error) inside them is what the Decoder answers
    match decode_first(&body[..max]) {
        Ok(n) => Ok(n),
        Err(m) if m == "unexpected EOF" || m == "EOF" => Err(None),
        Err(m) => {
            // a number running to the cut could go on: the reader's error comes first then
            Err(Some(m))
        }
    }
}

// ---------- binding: Go's rules for decoding into a struct ----------

/// The decoding state for one value: the first error, and the context Go adds to a type error.
pub struct Bind<'a> {
    pub src: &'a [u8],
    pub err: Option<String>,
    /// the innermost struct's name and the field path (`errorContext`)
    strukt: Option<&'static str>,
    stack: Vec<&'static str>,
    pub disallow_unknown: bool,
}

impl<'a> Bind<'a> {
    pub fn new(src: &'a [u8]) -> Bind<'a> {
        Bind { src, err: None, strukt: None, stack: Vec::new(), disallow_unknown: false }
    }

    /// `UnmarshalTypeError` with the current context.
    fn type_err(&mut self, value: &str, go_type: &str) {
        if self.err.is_some() {
            return;
        }
        self.err = Some(match self.strukt {
            Some(s) => format!("json: cannot unmarshal {value} into Go struct field {s}.{} of type {go_type}", self.stack.join(".")),
            None => format!("json: cannot unmarshal {value} into Go value of type {go_type}"),
        });
    }

    pub fn raw(&self, n: &Node) -> String {
        String::from_utf8_lossy(&self.src[n.start..n.end]).into_owned()
    }

    pub fn string(&mut self, n: &Node, out: &mut String) {
        match &n.kind {
            Kind::Str(s) => *out = s.clone(),
            Kind::Null => {}
            k => self.type_err(k.word(), "string"),
        }
    }
    pub fn boolean(&mut self, n: &Node, out: &mut bool) {
        match &n.kind {
            Kind::Bool(b) => *out = *b,
            Kind::Null => {}
            k => self.type_err(k.word(), "bool"),
        }
    }
    /// an `int` / `int64` field
    pub fn int(&mut self, n: &Node, out: &mut i64, go_type: &str) {
        match &n.kind {
            Kind::Num(lit) => match lit.parse::<i64>() {
                Ok(v) => *out = v,
                Err(_) => self.type_err(&format!("number {lit}"), go_type),
            },
            Kind::Null => {}
            k => self.type_err(k.word(), go_type),
        }
    }
    pub fn float(&mut self, n: &Node, out: &mut f64) {
        match &n.kind {
            Kind::Num(lit) => match lit.parse::<f64>() {
                Ok(v) if v.is_finite() => *out = v,
                _ => self.type_err(&format!("number {lit}"), "float64"),
            },
            Kind::Null => {}
            k => self.type_err(k.word(), "float64"),
        }
    }
    /// a `json.RawMessage` field: the input bytes, `null` included
    pub fn rawmsg(&mut self, n: &Node, out: &mut Option<String>) {
        *out = Some(self.raw(n));
    }

    /// Decode an object into a struct: `fields` are the struct's JSON names in declaration order, `set` stores one
    /// (it is called with the field's index). `go_type` is the struct type's `reflect` string, `name` its
    /// `Type.Name()` ("" for an anonymous struct).
    pub fn object(&mut self, n: &Node, name: &'static str, go_type: &str, fields: &[&'static str], mut set: impl FnMut(&mut Self, usize, &Node)) {
        let Kind::Obj(kv) = &n.kind else {
            if !matches!(n.kind, Kind::Null) {
                self.type_err(n.kind.word(), go_type);
            }
            return;
        };
        for (k, v) in kv {
            let idx = fields.iter().position(|f| f == k).or_else(|| fields.iter().position(|f| fold_eq(f, k)));
            match idx {
                Some(i) => {
                    let (os, ol) = (self.strukt, self.stack.len());
                    self.stack.push(fields[i]);
                    self.strukt = Some(name);
                    set(self, i, v);
                    self.strukt = os;
                    self.stack.truncate(ol);
                }
                None if self.disallow_unknown => {
                    if self.err.is_none() {
                        self.err = Some(format!("json: unknown field {}", go_quote(k)));
                    }
                }
                None => {}
            }
        }
    }
}

/// Go's case folding for field names (ASCII, plus the two non-ASCII letters that fold to ASCII ones).
fn fold_eq(field: &str, key: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                '\u{212a}' => 'k',
                '\u{17f}' => 's',
                c => c.to_ascii_lowercase(),
            })
            .collect()
    };
    norm(field) == norm(key)
}

/// `strconv.Quote` (enough for a JSON object key: printable characters kept, the rest escaped).
pub fn go_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{7}' => o.push_str("\\a"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            '\u{b}' => o.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => o.push_str(&format!("\\x{:02x}", c as u32)),
            '\u{fffd}' => o.push('\u{fffd}'),
            c if c.is_control() || matches!(c as u32, 0x2028 | 0x2029 | 0xad) => {
                if (c as u32) < 0x10000 {
                    o.push_str(&format!("\\u{:04x}", c as u32))
                } else {
                    o.push_str(&format!("\\U{:08x}", c as u32))
                }
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ---------- the writer side ----------

/// A string as `json.Marshal` writes it (HTML-escaped, U+2028/U+2029 escaped).
pub fn jstr(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            '<' => o.push_str("\\u003c"),
            '>' => o.push_str("\\u003e"),
            '&' => o.push_str("\\u0026"),
            '\u{2028}' => o.push_str("\\u2028"),
            '\u{2029}' => o.push_str("\\u2029"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}

pub fn q(s: &str) -> String {
    let mut o = String::new();
    jstr(&mut o, s);
    o
}

/// A `json.RawMessage` as `json.Marshal` writes it: compacted, with `<`, `>`, `&`, U+2028 and U+2029 escaped.
pub fn compact(raw: &str) -> String {
    let mut o = String::with_capacity(raw.len());
    let mut in_str = false;
    let mut esc = false;
    for c in raw.chars() {
        if in_str {
            if esc {
                esc = false;
                o.push(c);
                continue;
            }
            match c {
                '\\' => {
                    esc = true;
                    o.push(c);
                }
                '"' => {
                    in_str = false;
                    o.push(c);
                }
                '<' => o.push_str("\\u003c"),
                '>' => o.push_str("\\u003e"),
                '&' => o.push_str("\\u0026"),
                '\u{2028}' => o.push_str("\\u2028"),
                '\u{2029}' => o.push_str("\\u2029"),
                c => o.push(c),
            }
            continue;
        }
        match c {
            ' ' | '\t' | '\n' | '\r' => {}
            '"' => {
                in_str = true;
                o.push(c);
            }
            c => o.push(c),
        }
    }
    o
}

/// A float64 as encoding/json writes it ('f' format for 1e-6 <= |x| < 1e21, else 'e' with a two-digit exponent).
pub fn jfloat(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let a = x.abs();
    if (1e-6..1e21).contains(&a) {
        // shortest repr that round-trips, in plain decimal
        let s = format!("{x}");
        if s.contains('e') {
            format!("{x:.}")
        } else {
            s
        }
    } else {
        let s = format!("{x:e}"); // like 1e-7 or 1.5e21
        let (m, e) = s.split_once('e').unwrap();
        let (sign, digits) = if let Some(d) = e.strip_prefix('-') { ("-", d) } else { ("+", e) };
        let digits = if digits.len() < 2 { format!("0{digits}") } else { digits.to_string() };
        format!("{m}e{sign}{digits}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(s: &str) -> String {
        match decode_first(s.as_bytes()) {
            Ok(_) => "ok".into(),
            Err(e) => e,
        }
    }

    /// The messages Go 1.26's Decoder gives (checked against `go run` of json.NewDecoder(...).Decode).
    #[test]
    fn decoder_errors() {
        assert_eq!(first(""), "EOF");
        assert_eq!(first("   "), "EOF");
        assert_eq!(first("{"), "unexpected EOF");
        assert_eq!(first("x"), "invalid character 'x' looking for beginning of value");
        assert_eq!(first("{x"), "invalid character 'x' looking for beginning of object key string");
        assert_eq!(first("{\"a\" 1}"), "invalid character '1' after object key");
        assert_eq!(first("{\"a\":1 2}"), "invalid character '2' after object key:value pair");
        assert_eq!(first("[1 2]"), "invalid character '2' after array element");
        assert_eq!(first("{\"a\":tru}"), "invalid character '}' in literal true (expecting 'e')");
        assert_eq!(first("{\"a\":\"\\q\"}"), "invalid character 'q' in string escape code");
        assert_eq!(first("{\"a\":1.}"), "invalid character '}' after decimal point in numeric literal");
        assert_eq!(first("{\"a\":-}"), "invalid character '}' in numeric literal");
        assert_eq!(first("{} trailing"), "ok");
        assert_eq!(first("{\"a\":\"\u{1}\"}"), "invalid character '\\x01' in string literal");
        assert_eq!(unmarshal(b"{} x").unwrap_err(), "invalid character 'x' after top-level value");
        assert_eq!(unmarshal(b"").unwrap_err(), "unexpected end of JSON input");
        assert_eq!(unmarshal(b"{").unwrap_err(), "unexpected end of JSON input");
    }

    #[test]
    fn strings_unquote_as_go() {
        let n = decode_first(br#""a\u00e9\ud83d\ude00\ud800x""#).unwrap();
        let Kind::Str(s) = n.kind else { panic!() };
        assert_eq!(s, "a\u{e9}\u{1f600}\u{fffd}x");
    }

    #[test]
    fn writer() {
        assert_eq!(q("<a&b>\u{2028}"), "\"\\u003ca\\u0026b\\u003e\\u2028\"");
        assert_eq!(compact("{ \"a\" : [1, 2], \"b\":\"<x y>\" }"), "{\"a\":[1,2],\"b\":\"\\u003cx y\\u003e\"}");
        assert_eq!(jfloat(2.5), "2.5");
        assert_eq!(jfloat(3.0), "3");
        assert_eq!(jfloat(1e-7), "1e-07");
        assert_eq!(jfloat(1e21), "1e+21");
    }
}
