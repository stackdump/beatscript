//! The EBNF of `grammar.ebnf`, read into a tree of [`Node`]s: what the railroad diagrams draw, what the recognizer
//! matches tokens against and what the generator writes sentences from.
//!
//! The notation (ISO 14977, the parts the grammar uses):
//!
//! ```text
//! name = alternative { "|" alternative } ;     a production
//! "text" or 'text'                             a terminal: the token(s) written so
//! NAME (upper case, not a production)          a token class of the lexer (NAME, INT, NOTE, ...)
//! [ ... ]  { ... }  ( ... )                    optional, zero or more, a group
//! (* ... *)                                    a comment; after a production's `;` it is that production's note
//! (* == Title == *)                            starts a section of the grammar (the page's headings)
//! ```

use std::collections::BTreeSet;

/// A diagram's node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// One after another.
    Seq(Vec<Node>),
    /// One of.
    Choice(Vec<Node>),
    /// `[ x ]`.
    Opt(Box<Node>),
    /// `{ x }`: none or more.
    Rep(Box<Node>),
    /// A terminal, as written (without its quotes).
    Term(String),
    /// A production, or a token class.
    Ref(String),
}

impl Node {
    /// Every name the node refers to, in order of first use.
    pub fn refs(&self, out: &mut Vec<String>) {
        match self {
            Node::Seq(v) | Node::Choice(v) => v.iter().for_each(|n| n.refs(out)),
            Node::Opt(n) | Node::Rep(n) => n.refs(out),
            Node::Ref(r) => {
                if !out.contains(r) {
                    out.push(r.clone());
                }
            }
            Node::Term(_) => {}
        }
    }

    /// Does the node name `name` anywhere (a production or a class)?
    pub fn refs_name(&self, name: &str) -> bool {
        let mut r = Vec::new();
        self.refs(&mut r);
        r.iter().any(|x| x == name)
    }

    /// Every terminal in the node.
    pub fn terms(&self, out: &mut BTreeSet<String>) {
        match self {
            Node::Seq(v) | Node::Choice(v) => v.iter().for_each(|n| n.terms(out)),
            Node::Opt(n) | Node::Rep(n) => n.terms(out),
            Node::Term(t) => {
                out.insert(t.clone());
            }
            Node::Ref(_) => {}
        }
    }
}

/// One production.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub node: Node,
    /// Its note: the comment after its `;`, for a production written on one line (a longer one's comments go with its
    /// alternatives, in [`Rule::source`]).
    pub note: Option<String>,
    /// The section it is in (the last `(* == ... == *)` above it).
    pub section: String,
    /// Its line in the file (1-based).
    pub line: usize,
    /// The production as written, from its name to its `;`, with the comments inside it and after it (the alternatives'
    /// notes), whitespace as in the file.
    pub source: String,
}

/// A whole grammar.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Grammar {
    pub rules: Vec<Rule>,
}

impl Grammar {
    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// The token classes: the upper-case names that are referred to and are not productions.
    pub fn classes(&self) -> BTreeSet<String> {
        let mut refs = Vec::new();
        for r in &self.rules {
            r.node.refs(&mut refs);
        }
        refs.into_iter().filter(|n| self.rule(n).is_none() && is_class_name(n)).collect()
    }

    /// The sections, in order, each with its rules.
    pub fn sections(&self) -> Vec<(String, Vec<&Rule>)> {
        let mut out: Vec<(String, Vec<&Rule>)> = Vec::new();
        for r in &self.rules {
            match out.last_mut() {
                Some((s, v)) if *s == r.section => v.push(r),
                _ => out.push((r.section.clone(), vec![r])),
            }
        }
        out
    }

    /// What is wrong with it as a grammar: a name used and never defined (a lower-case one; an upper-case one is a
    /// token class), a production defined twice, one that nothing uses (but the first, the start).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for r in &self.rules {
            if !seen.insert(&r.name) {
                out.push(format!("line {}: `{}` is defined twice", r.line, r.name));
            }
        }
        let mut used = BTreeSet::new();
        for r in &self.rules {
            let mut refs = Vec::new();
            r.node.refs(&mut refs);
            for n in refs {
                if self.rule(&n).is_none() && !is_class_name(&n) {
                    out.push(format!("line {}: `{}` uses `{n}`, which is not defined", r.line, r.name));
                }
                used.insert(n);
            }
        }
        for (holder, text) in TEXT_GRAMMARS {
            match (self.rule(holder), self.rule(text)) {
                // a grammar with neither (a small one, in a test) has nothing to say
                (None, None) => {}
                (Some(h), Some(_)) if h.node.refs_name("STRING") => {
                    used.insert((*text).to_string());
                }
                _ => out.push(format!("`{text}` describes the text of the STRING in `{holder}`, and one of the two is missing (or `{holder}` holds no STRING)")),
            }
        }
        for r in self.rules.iter().skip(1) {
            if !used.contains(&r.name) {
                out.push(format!("line {}: nothing uses `{}`", r.line, r.name));
            }
        }
        out
    }
}

/// The productions that describe the text inside a string the program's grammar holds as one `STRING` token: (the
/// production that holds the string, the production for its text). The program's sentences stop at the token, so
/// nothing in the program's grammar uses the second; [`Grammar::problems`] lets it stand, and the drift test reads it
/// over characters ([`super::recognize::recognize_text`]).
pub const TEXT_GRAMMARS: &[(&str, &str)] = &[("lyric", "lyric_text")];

/// A token class's name: upper case, digits and `_`.
pub fn is_class_name(n: &str) -> bool {
    n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Ident(String),
    Str(String),
    Sym(char),
    Comment(String),
}

/// An error in the grammar file, with its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "grammar.ebnf:{}: {}", self.line, self.message)
    }
}

fn lex(src: &str) -> Result<Vec<(T, usize, usize)>, ReadError> {
    // (token, line, byte offset)
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut off = 0usize;
    let adv = |i: &mut usize, off: &mut usize, line: &mut usize, c: char| {
        *i += 1;
        *off += c.len_utf8();
        if c == '\n' {
            *line += 1;
        }
    };
    while i < b.len() {
        let c = b[i];
        let (l0, o0) = (line, off);
        if c.is_whitespace() {
            adv(&mut i, &mut off, &mut line, c);
        } else if c == '(' && b.get(i + 1) == Some(&'*') {
            adv(&mut i, &mut off, &mut line, '(');
            adv(&mut i, &mut off, &mut line, '*');
            let mut text = String::new();
            loop {
                if i >= b.len() {
                    return Err(ReadError { line: l0, message: "a comment is never closed".into() });
                }
                if b[i] == '*' && b.get(i + 1) == Some(&')') {
                    adv(&mut i, &mut off, &mut line, '*');
                    adv(&mut i, &mut off, &mut line, ')');
                    break;
                }
                text.push(b[i]);
                let ch = b[i];
                adv(&mut i, &mut off, &mut line, ch);
            }
            out.push((T::Comment(text), l0, o0));
        } else if c == '"' || c == '\'' {
            let q = c;
            adv(&mut i, &mut off, &mut line, c);
            let mut s = String::new();
            loop {
                match b.get(i) {
                    None | Some('\n') => return Err(ReadError { line: l0, message: "a terminal is never closed".into() }),
                    Some(&ch) if ch == q => {
                        adv(&mut i, &mut off, &mut line, ch);
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        adv(&mut i, &mut off, &mut line, ch);
                    }
                }
            }
            out.push((T::Str(s), l0, o0));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(&ch) = b.get(i) {
                if ch.is_ascii_alphanumeric() || ch == '_' || (ch == '-' && b.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric())) {
                    s.push(ch);
                    adv(&mut i, &mut off, &mut line, ch);
                } else {
                    break;
                }
            }
            out.push((T::Ident(s), l0, o0));
        } else if "=|;[]{}()".contains(c) {
            adv(&mut i, &mut off, &mut line, c);
            out.push((T::Sym(c), l0, o0));
        } else {
            return Err(ReadError { line, message: format!("`{c}` is not EBNF here (a terminal is quoted: \"{c}\")") });
        }
    }
    Ok(out)
}

/// Read a grammar.
pub fn read(src: &str) -> Result<Grammar, ReadError> {
    let toks = lex(src)?;
    let mut rules = Vec::new();
    let mut section = String::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i].0 {
            T::Comment(c) => {
                let t = c.trim();
                if let Some(title) = t.strip_prefix("==").and_then(|t| t.strip_suffix("==")) {
                    section = title.trim().to_string();
                }
                i += 1;
            }
            T::Ident(name) => {
                let (line, start) = (toks[i].1, toks[i].2);
                if toks.get(i + 1).map(|t| &t.0) != Some(&T::Sym('=')) {
                    return Err(ReadError { line, message: format!("expected `=` after `{name}`") });
                }
                let mut p = P { toks: &toks, at: i + 2 };
                let node = p.alt()?;
                let end = match toks.get(p.at) {
                    Some((T::Sym(';'), _, o)) => *o + 1,
                    Some((t, l, _)) => return Err(ReadError { line: *l, message: format!("expected `|` or `;` in `{name}`, found {t:?}") }),
                    None => return Err(ReadError { line, message: format!("`{name}` has no `;`") }),
                };
                let semi_line = toks[p.at].1;
                p.at += 1;
                // the comment on the line of its `;` (it may run on over more lines)
                let mut note = String::new();
                let mut src_end = end;
                while let Some((T::Comment(c), l, o)) = toks.get(p.at) {
                    let t = c.trim();
                    if t.starts_with("==") || *l != semi_line {
                        break;
                    }
                    note.push_str(&t.split_whitespace().collect::<Vec<_>>().join(" "));
                    src_end = o + c.len() + 4;
                    p.at += 1;
                }
                let one_line = line == semi_line;
                let source = tidy(&src[start..src_end.min(src.len())]);
                let note = if one_line { note } else { String::new() };
                rules.push(Rule { name: name.clone(), node, note: Some(note).filter(|n| !n.is_empty()), section: section.clone(), line, source });
                i = p.at;
            }
            t => return Err(ReadError { line: toks[i].1, message: format!("expected a production's name, found {t:?}") }),
        }
    }
    Ok(Grammar { rules })
}

/// A production's text: its lines with their trailing space cut, empty ones left out.
fn tidy(s: &str) -> String {
    s.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")
}

struct P<'a> {
    toks: &'a [(T, usize, usize)],
    at: usize,
}

impl P<'_> {
    fn skip_comments(&mut self) {
        while matches!(self.toks.get(self.at), Some((T::Comment(_), _, _))) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<&T> {
        self.skip_comments();
        self.toks.get(self.at).map(|t| &t.0)
    }

    fn line(&self) -> usize {
        self.toks.get(self.at).or(self.toks.last()).map_or(0, |t| t.1)
    }

    fn alt(&mut self) -> Result<Node, ReadError> {
        let mut alts = vec![self.seq()?];
        while self.peek() == Some(&T::Sym('|')) {
            self.at += 1;
            alts.push(self.seq()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Choice(alts) })
    }

    fn seq(&mut self) -> Result<Node, ReadError> {
        let mut items = Vec::new();
        loop {
            let n = match self.peek().cloned() {
                Some(T::Str(s)) => {
                    self.at += 1;
                    Node::Term(s)
                }
                Some(T::Ident(s)) => {
                    // the name of the next production is never part of this one (its `=` follows it)
                    if matches!(self.toks.get(self.at + 1), Some((T::Sym('='), _, _))) {
                        return Err(ReadError { line: self.line(), message: format!("`{s} =` starts a production: the one before it has no `;`") });
                    }
                    self.at += 1;
                    Node::Ref(s)
                }
                Some(T::Sym(o @ ('[' | '{' | '('))) => {
                    self.at += 1;
                    let inner = self.alt()?;
                    let close = match o {
                        '[' => ']',
                        '{' => '}',
                        _ => ')',
                    };
                    if self.peek() != Some(&T::Sym(close)) {
                        return Err(ReadError { line: self.line(), message: format!("expected `{close}`") });
                    }
                    self.at += 1;
                    match o {
                        '[' => Node::Opt(Box::new(inner)),
                        '{' => Node::Rep(Box::new(inner)),
                        _ => inner,
                    }
                }
                _ => break,
            };
            items.push(n);
        }
        if items.is_empty() {
            return Err(ReadError { line: self.line(), message: "an empty alternative (write \"\" for the empty string)".into() });
        }
        Ok(if items.len() == 1 { items.pop().unwrap() } else { Node::Seq(items) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_notation() {
        let g = read("(* == Core == *)\nprogram = { line } ;  (* the start *)\nline = [ stmt ] NEWLINE ;\nstmt = \"a\" | 'b' ( x | \"c\" \"d\" ) ;\n(* == More == *)\nx = INT ;\n").unwrap();
        assert_eq!(g.rules.len(), 4);
        assert_eq!(g.rules[0].node, Node::Rep(Box::new(Node::Ref("line".into()))));
        assert_eq!(g.rules[0].note.as_deref(), Some("the start"));
        assert_eq!(g.rules[2].node, Node::Choice(vec![Node::Term("a".into()), Node::Seq(vec![Node::Term("b".into()), Node::Choice(vec![Node::Ref("x".into()), Node::Seq(vec![Node::Term("c".into()), Node::Term("d".into())])])])]));
        assert_eq!(g.rules[3].section, "More");
        assert_eq!(g.sections().len(), 2);
        assert_eq!(g.classes().into_iter().collect::<Vec<_>>(), ["INT", "NEWLINE"]);
        assert!(g.problems().is_empty(), "{:?}", g.problems());
        assert_eq!(g.rules[2].source, "stmt = \"a\" | 'b' ( x | \"c\" \"d\" ) ;");
        assert_eq!(g.rules[0].source, "program = { line } ;  (* the start *)");
        // a production over several lines: its comments go with its alternatives, not as one note
        let g = read("a = \"x\"     (* the x *)\n  | \"y\" ;  (* the y *)\n").unwrap();
        assert_eq!(g.rules[0].note, None);
        assert!(g.rules[0].source.contains("(* the x *)") && g.rules[0].source.contains("(* the y *)"));
    }

    #[test]
    fn says_what_is_wrong() {
        assert_eq!(read("a = \"x\"\nb = \"y\" ;").unwrap_err().line, 2);
        assert!(read("a = [ \"x\" ;").is_err());
        assert!(read("a = ;").is_err());
        assert!(read("a = \"x ;").is_err());
        let g = read("a = b | c ;\nb = \"x\" ;\nb = \"y\" ;\nd = \"z\" ;").unwrap();
        let p = g.problems();
        assert!(p.iter().any(|m| m.contains("`b` is defined twice")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("uses `c`")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("nothing uses `d`")), "{p:?}");
    }
}
