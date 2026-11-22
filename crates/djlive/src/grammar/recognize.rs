//! Is a program a sentence of the grammar? The recognizer reads the tokens the statement parser reads (the lexer's,
//! after the prepass) against `grammar.ebnf`, and says where it could go no further. One direction of the drift test:
//! every example program must be a sentence (tests/grammar.rs).
//!
//! It is a set-of-positions recognizer: for each node and each place in the tokens it computes every place a match could
//! end (memoized per production and place), so an ambiguous grammar is fine and nothing is ever guessed. A terminal is
//! the token written so, or tokens written together that spell it (`==` is two tokens, `seed=` is two); an upper-case
//! name that is no production is a token class ([`class_ends`]).

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use super::ebnf::{Grammar, Node};
use crate::lex::{Tok, Token};

/// The text a token is written as (None: a token no terminal spells: a string, a `$name`, the end).
fn tok_text(t: &Token, src: &str) -> Option<String> {
    Some(match &t.tok {
        Tok::Word(w) => w.clone(),
        Tok::Num { .. } => t.span.text(src).to_string(),
        Tok::Unit(u) => u.clone(),
        Tok::Str(_) | Tok::Dollar(_) | Tok::Newline | Tok::Eof => return None,
        Tok::OctDown => ",".into(),
        other => crate::print::tokens(std::iter::once(other)),
    })
}

/// The token classes the grammar may name, and what each matches. A name here that the grammar does not use is fine;
/// one the grammar uses that is not here fails the drift test.
pub const CLASSES: &[(&str, &str)] = &[
    ("NEWLINE", "the end of a line"),
    ("NAME", "a name: a word (`acid`, `sub-bass`, `drop2`)"),
    ("WORD", "any word"),
    ("INT", "a whole number written out (`4`, `16`)"),
    ("NUMBER", "a number: `0.7`, `2k` (2000), `50%` (0.5), `-3`"),
    ("DECIMAL", "a number with no mark, before a glued unit: `6` in `6db`, `0.5` in `0.5s`"),
    ("STRING", "a string in double quotes"),
    ("PARAM", "a parameter of a def or a repeat: `$i`, `$bars`"),
    ("NOTE", "a note: a letter a to g, a sharp `#` or flat `b`, an octave: `c3`, `f#2`, `bb4`, `a`"),
    ("PITCH", "a note with its octave: `c4`, `f#2`"),
    ("KEY", "a key: a tonic and `m`, or a tonic, `:` and a mode, written as one word: `am`, `c`, `f#m`, `d:dorian`"),
    ("CHORD", "a chord symbol: a tonic and a quality, one word: `am7`, `cmaj7`, `g7`, `dsus4`, `f#m7b5`"),
    ("ROMAN", "a Roman numeral of the key, one word: `i`, `VI`, `ii°`, `V7`, `bVII`, `#iv`"),
    ("SDEG", "a written degree inside deg( ): `b3`, `#4`"),
    ("UNIT", "a unit glued to the number before it: `db`, `ms`, `s`, `m`, `h`, `hz`"),
    ("TOKEN", "any token but the end of a line"),
    ("GLOB", "a name with `*` written into it, all one: `acid_*`, `*_a`"),
    ("SUGAR", "a control noun said with no `do`: `vocoder`, `freeze`, `wheel`, `trick`, `mashup` ..."),
    ("PHRASE", "a phrase of the phrasebook, `@` and its id, one word: `@hands_up`, `@feel_the_bass`, or a line of verse or speech from before 1929: `@tempest_such_stuff`"),
    ("GHOST", "a `.` touching the step before it, with no word after it: a ghost note (`x.`, `c3.`)"),
    ("RATCHET", "a `:` touching the step before it and a whole number touching it: a ratchet (`x:3`)"),
    ("CID", "a saved program's CID: `z` and 20 to 80 base58 characters (`z4EBG9jDzsE2UHZfL8A9ouKLU5vUJsgHC8nVaTE7sWjci7PCukZ`)"),
    // the classes of a lyric's text, read over characters ([`recognize_text`]), not tokens
    ("TEXT", "in a lyric: one character that is not `/`, a space and a letter alike"),
    ("SPACE", "in a lyric: one space, tab or line break"),
    ("PHONES", "in a lyric: what is between an escape's slashes, from something that is not a space to the next `/`: phonemes of the set with a space or `-` between them (`h ii`, `w ee - aa: r`)"),
];

/// Every place a token class matching at `at` could end.
fn class_ends(name: &str, toks: &[Token], src: &str, at: usize) -> Vec<usize> {
    let Some(t) = toks.get(at) else { return Vec::new() };
    let one = |ok: bool| if ok { vec![at + 1] } else { Vec::new() };
    let word = |f: &dyn Fn(&str) -> bool| match &t.tok {
        Tok::Word(w) => f(w),
        _ => false,
    };
    match name {
        "NEWLINE" => one(matches!(t.tok, Tok::Newline)),
        "NAME" | "WORD" => one(word(&|_| true)),
        "INT" => one(matches!(t.tok, Tok::Num { int: true, .. })),
        "NUMBER" => one(matches!(t.tok, Tok::Num { .. })),
        "DECIMAL" => one(matches!(t.tok, Tok::Num { .. }) && t.span.text(src).bytes().all(|b| b.is_ascii_digit() || b == b'.')),
        "STRING" => one(matches!(t.tok, Tok::Str(_))),
        "PARAM" => one(matches!(t.tok, Tok::Dollar(_))),
        "NOTE" => one(word(&|w| crate::music::parse_note(w).is_some())),
        "PITCH" => one(word(&|w| matches!(crate::music::parse_note(w), Some((_, Some(_)))))),
        "KEY" => one(word(&|w| crate::music::Key::parse(w).is_some())),
        "CHORD" => one(word(&|w| crate::music::Chord::parse(w).is_some())),
        "ROMAN" => one(word(&|w| crate::scale::is_roman(w))),
        "SDEG" => one(word(&|w| w.starts_with('#') || (w.starts_with('b') && w[1..].bytes().all(|b| b.is_ascii_digit()) && w.len() > 1))),
        "UNIT" => one(matches!(t.tok, Tok::Unit(_))),
        "TOKEN" => one(!matches!(t.tok, Tok::Newline | Tok::Eof)),
        "SUGAR" => one(word(&|w| crate::parse::sugar_noun(w).is_some())),
        "PHRASE" => one(word(&|w| w.strip_prefix('@').is_some_and(|id| djengine::vocal::phoneme::phrase_lyric(id).is_some()))),
        "CID" => one(word(&crate::stored::is_cid)),
        "GHOST" => one(matches!(t.tok, Tok::Ghost)),
        "RATCHET" => match toks.get(at + 1) {
            Some(n) if matches!(t.tok, Tok::Colon) && matches!(n.tok, Tok::Num { int: true, .. }) && n.span.start == t.span.end && at > 0 && toks[at - 1].span.end == t.span.start => vec![at + 2],
            _ => Vec::new(),
        },
        "GLOB" => {
            // words and `*` written together, with a `*` among them
            let mut end = at;
            let mut star = false;
            while let Some(x) = toks.get(end) {
                if end > at && toks[end - 1].span.end != x.span.start {
                    break;
                }
                match x.tok {
                    Tok::Word(_) => {}
                    Tok::Star => star = true,
                    _ => break,
                }
                end += 1;
            }
            if star && end > at {
                vec![end]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// Every place a class of a lyric's text matching at byte `at` could end.
fn text_class_ends(name: &str, text: &str, at: usize) -> Vec<usize> {
    let rest = text.get(at..).unwrap_or("");
    let one = |ok: &dyn Fn(char) -> bool| rest.chars().next().filter(|c| ok(*c)).map_or(Vec::new(), |c| vec![at + c.len_utf8()]);
    match name {
        "TEXT" => one(&|c| c != '/'),
        "SPACE" => one(&|c| c.is_whitespace()),
        // the engine's own reading of an escape, whole: every symbol in the set, its marks, nothing left over
        "PHONES" => match rest.find('/') {
            Some(n) if rest.chars().next().is_some_and(|c| !c.is_whitespace()) && djengine::vocal::phoneme::read_escape(&rest[..n], 0).1.is_empty() => vec![at + n],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Where a program stops being a sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Miss {
    /// The token index, and its line and column (1-based) in the source.
    pub at: usize,
    pub line: usize,
    pub col: usize,
    /// What the token there is written as.
    pub found: String,
    /// What the grammar would have taken there.
    pub expected: Vec<String>,
}

impl std::fmt::Display for Miss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: the grammar stops at {} (it would take {})", self.line, self.col, self.found, self.expected.join(", "))
    }
}

struct R<'a> {
    g: &'a Grammar,
    idx: HashMap<&'a str, usize>,
    toks: &'a [Token],
    src: &'a str,
    memo: HashMap<(usize, usize), Rc<Vec<usize>>>,
    busy: BTreeSet<(usize, usize)>,
    /// The furthest token any terminal or class was tried at, and what was tried there.
    far: usize,
    wanted: BTreeSet<String>,
    /// Reading the characters of a string's text (`recognize_text`) and not a program's tokens: positions are bytes.
    text: Option<&'a str>,
}

impl<'a> R<'a> {
    fn tried(&mut self, at: usize, what: String) {
        if at > self.far {
            self.far = at;
            self.wanted.clear();
        }
        if at == self.far {
            self.wanted.insert(what);
        }
    }

    fn term(&mut self, t: &str, at: usize) -> Vec<usize> {
        if let Some(text) = self.text {
            // (text has no expected list to keep: the furthest byte is all it says, and it is asked for many times)
            self.far = self.far.max(at);
            return if t.is_empty() {
                vec![at]
            } else if text.get(at..).is_some_and(|r| r.starts_with(t)) {
                vec![at + t.len()]
            } else {
                Vec::new()
            };
        }
        self.tried(at, format!("`{t}`"));
        if t.is_empty() {
            return vec![at];
        }
        let mut acc = String::new();
        let mut i = at;
        while i < self.toks.len() && acc.len() < t.len() {
            if i > at && self.toks[i - 1].span.end != self.toks[i].span.start {
                return Vec::new();
            }
            let Some(x) = tok_text(&self.toks[i], self.src) else { return Vec::new() };
            acc.push_str(&x);
            i += 1;
            if !t.starts_with(&acc) {
                return Vec::new();
            }
        }
        if acc == t {
            vec![i]
        } else {
            Vec::new()
        }
    }

    fn rule(&mut self, i: usize, at: usize) -> Rc<Vec<usize>> {
        if let Some(r) = self.memo.get(&(i, at)) {
            return r.clone();
        }
        if !self.busy.insert((i, at)) {
            // left recursion: no match from here (the grammar has none on purpose)
            return Rc::new(Vec::new());
        }
        let node = &self.g.rules[i].node;
        let ends = self.node(node, at);
        self.busy.remove(&(i, at));
        let r = Rc::new(ends);
        self.memo.insert((i, at), r.clone());
        r
    }

    fn node(&mut self, n: &'a Node, at: usize) -> Vec<usize> {
        let mut out: BTreeSet<usize> = BTreeSet::new();
        match n {
            Node::Term(t) => return self.term(t, at),
            Node::Ref(name) => {
                if let Some(&i) = self.idx.get(name.as_str()) {
                    return (*self.rule(i, at)).clone();
                }
                if let Some(text) = self.text {
                    self.far = self.far.max(at);
                    return text_class_ends(name, text, at);
                }
                self.tried(at, name.clone());
                return class_ends(name, self.toks, self.src, at);
            }
            Node::Seq(items) => {
                let mut here: BTreeSet<usize> = [at].into();
                for it in items {
                    let mut next = BTreeSet::new();
                    for p in here {
                        next.extend(self.node(it, p));
                    }
                    if next.is_empty() {
                        return Vec::new();
                    }
                    here = next;
                }
                out = here;
            }
            Node::Choice(alts) => {
                for a in alts {
                    out.extend(self.node(a, at));
                }
            }
            Node::Opt(x) => {
                out.insert(at);
                out.extend(self.node(x, at));
            }
            Node::Rep(x) => {
                out.insert(at);
                let mut frontier = vec![at];
                while let Some(p) = frontier.pop() {
                    for e in self.node(x, p) {
                        if e > p && out.insert(e) {
                            frontier.push(e);
                        }
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

fn line_col(src: &str, byte: usize) -> (usize, usize) {
    let before = &src[..byte.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Is `tokens` (with `src`, the text they are spans of) a sentence of `start` in `g`?
pub fn recognize_tokens(g: &Grammar, start: &str, toks: &[Token], src: &str) -> Result<(), Miss> {
    let idx: HashMap<&str, usize> = g.rules.iter().enumerate().map(|(i, r)| (r.name.as_str(), i)).collect();
    let Some(&s) = idx.get(start) else {
        return Err(Miss { at: 0, line: 1, col: 1, found: "the start".into(), expected: vec![format!("a production `{start}`")] });
    };
    let mut r = R { g, idx, toks, src, memo: HashMap::new(), busy: BTreeSet::new(), far: 0, wanted: BTreeSet::new(), text: None };
    let ends = r.rule(s, 0);
    if ends.contains(&toks.len()) {
        return Ok(());
    }
    let at = r.far.min(toks.len().saturating_sub(1));
    let (found, byte) = match toks.get(r.far) {
        Some(t) => (match &t.tok {
            Tok::Newline => "the end of the line".to_string(),
            Tok::Eof => "the end".to_string(),
            other => other.describe(),
        }, t.span.start),
        None => ("the end".to_string(), src.len()),
    };
    let (line, col) = line_col(src, byte);
    Err(Miss { at, line, col, found, expected: r.wanted.into_iter().collect() })
}

/// Is `text` a sentence of `start` in `g`, read over its characters? The text inside a string the program's grammar holds as
/// one token (the lyric's, `lyric_text`: [`super::ebnf::TEXT_GRAMMARS`]); the terminals are characters and the classes
/// [`CLASSES`] names for text. `Err` is the furthest byte any terminal or class was tried at.
pub fn recognize_text(g: &Grammar, start: &str, text: &str) -> Result<(), usize> {
    let idx: HashMap<&str, usize> = g.rules.iter().enumerate().map(|(i, r)| (r.name.as_str(), i)).collect();
    let Some(&s) = idx.get(start) else { return Err(0) };
    let mut r = R { g, idx, toks: &[], src: "", memo: HashMap::new(), busy: BTreeSet::new(), far: 0, wanted: BTreeSet::new(), text: Some(text) };
    if r.rule(s, 0).contains(&text.len()) {
        Ok(())
    } else {
        Err(r.far)
    }
}

/// Is `src` a program the grammar describes? Its tokens are the parser's (after the prepass), the end of the text is a
/// line's end.
pub fn recognize(g: &Grammar, src: &str) -> Result<(), Miss> {
    let mut toks: Vec<Token> = crate::parse::grammar_tokens(src).into_iter().filter(|t| !matches!(t.tok, Tok::Eof)).collect();
    if toks.last().is_some_and(|t| !matches!(t.tok, Tok::Newline)) {
        let end = toks.last().unwrap().span.end;
        toks.push(Token { tok: Tok::Newline, span: crate::diag::Span::new(end, end) });
    }
    let start = g.rules.first().map_or("program", |r| r.name.as_str()).to_string();
    recognize_tokens(g, &start, &toks, src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::ebnf::read;

    #[test]
    fn terminals_classes_and_glue() {
        let g = read("program = { line } ;\nline = [ stmt ] NEWLINE ;\nstmt = \"tempo\" NUMBER | \"a\" \"==\" INT | \"x\" \"seed=\" INT | NOTE { NOTE } ;").unwrap();
        assert!(recognize(&g, "tempo 120\na == 3\nx seed=4\nc3 d#4 e\n").is_ok());
        let e = recognize(&g, "tempo 120\na = = 3\n").unwrap_err();
        assert_eq!((e.line, e.col), (2, 3), "{e}");
        assert!(recognize(&g, "c3 q\n").is_err());
        // an ambiguous grammar is fine
        let g = read("program = { line } ;\nline = ( x | x x ) NEWLINE ;\nx = { \"a\" } ;").unwrap();
        assert!(recognize(&g, "a a a\n").is_ok());
    }
}
