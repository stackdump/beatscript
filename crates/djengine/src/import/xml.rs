//! A tiny XML tree (`XMLNode` in `Session/MusicXMLImport.swift`): elements, attributes and
//! the character data each element gathers. No namespaces, no external entities; DOCTYPE,
//! comments, processing instructions and CDATA are skipped or read as Foundation's
//! `XMLParser` does for a MusicXML score.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct XmlNode {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn child(&self, n: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.name == n)
    }
    pub fn all<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a XmlNode> + 'a {
        self.children.iter().filter(move |c| c.name == n)
    }
    /// Depth-first, self first.
    pub fn find(&self, n: &str) -> Option<&XmlNode> {
        if self.name == n {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(n))
    }
    pub fn descendants<'a>(&'a self, n: &str, out: &mut Vec<&'a XmlNode>) {
        for c in &self.children {
            if c.name == n {
                out.push(c);
            }
            c.descendants(n, out);
        }
    }
    pub fn int(&self) -> Option<i64> {
        self.text.trim().parse::<i64>().ok()
    }
    pub fn double(&self) -> Option<f64> {
        self.text.trim().parse::<f64>().ok()
    }
    /// The trimmed text, nil when empty (`nilIfEmpty`).
    pub fn text_or_none(&self) -> Option<String> {
        let t = self.text.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }
}

struct P<'a> {
    s: &'a str,
    i: usize,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

impl<'a> P<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn starts(&self, t: &str) -> bool {
        self.rest().starts_with(t)
    }
    fn skip_to(&mut self, end: &str) -> Result<(), String> {
        match self.rest().find(end) {
            Some(j) => {
                self.i += j + end.len();
                Ok(())
            }
            None => Err("unterminated markup".to_string()),
        }
    }
    fn ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
    }
    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        while let Some(c) = self.rest().chars().next() {
            if is_name_char(c) {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
        if self.i == start {
            return Err("expected a name".to_string());
        }
        Ok(self.s[start..self.i].to_string())
    }
}

fn decode_entities(raw: &str) -> Result<String, String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p + 1..];
        let Some(semi) = rest.find(';') else {
            return Err("bad entity".to_string());
        };
        let ent = &rest[..semi];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                let c = u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| "bad character reference".to_string())?;
                out.push(c);
            }
            _ if ent.starts_with('#') => {
                let c = ent[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| "bad character reference".to_string())?;
                out.push(c);
            }
            _ => return Err("unknown entity".to_string()),
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Parse a document; the error is a short message.
pub fn parse(data: &[u8]) -> Result<XmlNode, String> {
    let s = core::str::from_utf8(data).map_err(|_| "not UTF-8".to_string())?;
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut p = P { s, i: 0 };
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;
    loop {
        if p.i >= s.len() {
            break;
        }
        if p.starts("<?") {
            p.skip_to("?>")?;
        } else if p.starts("<!--") {
            p.skip_to("-->")?;
        } else if p.starts("<![CDATA[") {
            p.i += 9;
            let st = p.i;
            p.skip_to("]]>")?;
            let text = &s[st..p.i - 3];
            if let Some(top) = stack.last_mut() {
                top.text.push_str(text);
            }
        } else if p.starts("<!") {
            // DOCTYPE, possibly with an internal subset in [...]
            let mut depth = 0i32;
            loop {
                let Some(c) = p.rest().chars().next() else {
                    return Err("unterminated DOCTYPE".to_string());
                };
                p.i += c.len_utf8();
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    '>' if depth <= 0 => break,
                    _ => {}
                }
            }
        } else if p.starts("</") {
            p.i += 2;
            let n = p.name()?;
            p.ws();
            if !p.starts(">") {
                return Err("bad end tag".to_string());
            }
            p.i += 1;
            let Some(node) = stack.pop() else {
                return Err("unexpected end tag".to_string());
            };
            if node.name != n {
                return Err("mismatched end tag".to_string());
            }
            match stack.last_mut() {
                Some(top) => top.children.push(node),
                None => root = Some(node),
            }
        } else if p.starts("<") {
            p.i += 1;
            let n = p.name()?;
            let mut attrs = Vec::new();
            loop {
                p.ws();
                if p.starts("/>") {
                    p.i += 2;
                    let node = XmlNode {
                        name: n,
                        attrs,
                        text: String::new(),
                        children: Vec::new(),
                    };
                    match stack.last_mut() {
                        Some(top) => top.children.push(node),
                        None => {
                            if root.is_some() {
                                return Err("extra content after the root".to_string());
                            }
                            root = Some(node)
                        }
                    }
                    break;
                }
                if p.starts(">") {
                    p.i += 1;
                    if stack.is_empty() && root.is_some() {
                        return Err("extra content after the root".to_string());
                    }
                    stack.push(XmlNode {
                        name: n,
                        attrs,
                        text: String::new(),
                        children: Vec::new(),
                    });
                    break;
                }
                let an = p.name()?;
                p.ws();
                if !p.starts("=") {
                    return Err("expected =".to_string());
                }
                p.i += 1;
                p.ws();
                let q = p.rest().chars().next().ok_or("eof")?;
                if q != '"' && q != '\'' {
                    return Err("expected a quote".to_string());
                }
                p.i += 1;
                let st = p.i;
                let end = p.rest().find(q).ok_or("unterminated attribute")?;
                let v = decode_entities(&s[st..st + end])?;
                p.i = st + end + 1;
                attrs.push((an, v));
            }
        } else {
            // character data up to the next '<'
            let end = p.rest().find('<').unwrap_or(p.rest().len());
            let raw = &s[p.i..p.i + end];
            p.i += end;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&decode_entities(raw)?);
            } else if !raw.trim().is_empty() {
                return Err("text outside the root element".to_string());
            }
        }
    }
    if !stack.is_empty() {
        return Err("unclosed element".to_string());
    }
    root.ok_or_else(|| "empty".to_string())
}
