//! The symbol table: every name the program gives (sections, lets, defs with their parameters and ports, samples, imports)
//! with where it is defined and where it is used, plus the outline (parts, settings, `arrange`, `evolve`, `wire`...) and the
//! folding ranges.
//!
//! Read from the text's own tokens, not the AST: the parser puts a scalar `let` in place of its name before it reads
//! the statements, a `def`'s body is tokens until a call expands it, and a statement with an error is not in the AST at
//! all. A scan of the tokens sees every name as written, in a program that does not check too, which is when an editor
//! asks most.

use djlive::diag::Span;
use djlive::lex::{Tok, Token};

/// What a symbol is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Section,
    /// `let name = value`: a number, a word or a pattern.
    Let,
    /// `let both = stack(a, b)`: a name for a composition of sections.
    LetSection,
    Def,
    /// A parameter of a def (`def bed(oct)`), used as `$oct` in its body.
    Param,
    /// A port of a def (`in: chords out: bass`), used as `$chords` in its body and `bed.chords` in a `wire`.
    Port,
    /// `sample harp from "..."`, `loop "x" as harp`: a sound the program names, used as `inst=harp`.
    Sample,
    Import,
    /// `arrange set: drop wubs loop`: one of several named arrangements, used by `then set` and `start set`.
    Arrangement,
    // outline only: not names a program can use
    Part,
    Setting,
    Arrange,
    Evolve,
    Wire,
    Block,
}

impl Kind {
    /// The kinds that are names (a reference can point at one; rename works on one).
    pub fn is_name(self) -> bool {
        matches!(self, Kind::Section | Kind::Let | Kind::LetSection | Kind::Def | Kind::Param | Kind::Port | Kind::Sample | Kind::Import | Kind::Arrangement)
    }

    /// LSP's `SymbolKind`.
    pub fn lsp(self) -> i64 {
        match self {
            Kind::Section => 2,     // Module
            Kind::Let => 13,        // Variable
            Kind::LetSection => 23, // Struct
            Kind::Def => 12,        // Function
            Kind::Param => 26,      // TypeParameter
            Kind::Port => 11,       // Interface
            Kind::Sample => 1,      // File
            Kind::Import => 4,      // Package
            Kind::Arrangement => 18, // Array
            Kind::Part => 8,        // Field
            Kind::Setting => 20,    // Key
            Kind::Arrange => 18,    // Array
            Kind::Evolve => 24,     // Event
            Kind::Wire => 25,       // Operator
            Kind::Block => 3,       // Namespace
        }
    }

    /// The word for it, as djlive's `reserved_as` takes it and a person reads it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Section => "section",
            Kind::Let | Kind::LetSection => "let",
            Kind::Def => "def",
            Kind::Param => "parameter",
            Kind::Port => "port",
            Kind::Sample => "sound",
            Kind::Import => "import",
            Kind::Arrangement => "arrangement",
            Kind::Part => "part",
            Kind::Setting => "setting",
            Kind::Arrange => "arrange",
            Kind::Evolve => "evolve",
            Kind::Wire => "wire",
            Kind::Block => "block",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: Kind,
    /// The name where it is defined (for an outline-only entry, its first word).
    pub name_span: Span,
    /// The whole statement, block included.
    pub full: Span,
    /// What the outline shows beside the name: `16 bars`, `(oct)`, the value of a let.
    pub detail: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// A def's or a section's `{ ... }`: where its `$params` and its own statements are.
    pub body: Option<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub symbol: usize,
    pub span: Span,
    /// The definition itself (the name in `section drop ...`), not a use.
    pub is_def: bool,
    /// The use is `$name` (the `$` is in the span; a rename keeps it).
    pub dollar: bool,
}

/// A statement as the scan found it: its tokens, its first word, the symbol it opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub first: usize,
    /// One past its last token (its `Newline`, `}` or `Eof` is not in it).
    pub end: usize,
    pub span: Span,
    pub head: String,
    /// How many blocks it is inside.
    pub depth: usize,
    /// The symbol whose block it is in (a section, a def...).
    pub container: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldKind {
    Region,
    Comment,
}

#[derive(Clone, Debug, Default)]
pub struct Symbols {
    pub symbols: Vec<Symbol>,
    pub refs: Vec<Reference>,
    pub statements: Vec<Statement>,
    pub folds: Vec<(Span, FoldKind)>,
}

fn word(t: &Token) -> Option<&str> {
    match &t.tok {
        Tok::Word(w) => Some(w),
        _ => None,
    }
}

impl Symbols {
    /// The names a reference may point at, by text.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (usize, &'a Symbol)> + 'a {
        self.symbols.iter().enumerate().filter(move |(_, s)| s.kind.is_name() && s.name == name)
    }

    /// The reference (or definition) at byte `at`.
    pub fn ref_at(&self, at: usize) -> Option<&Reference> {
        self.refs.iter().find(|r| r.span.start <= at && at <= r.span.end)
    }

    pub fn refs_of(&self, symbol: usize) -> impl Iterator<Item = &Reference> {
        self.refs.iter().filter(move |r| r.symbol == symbol)
    }

    /// The innermost def or section whose body holds byte `at`.
    pub fn container_at(&self, at: usize) -> Option<usize> {
        self.symbols.iter().enumerate().filter(|(_, s)| s.body.is_some_and(|b| b.start < at && at <= b.end)).max_by_key(|(_, s)| s.body.map(|b| b.start)).map(|(i, _)| i)
    }

    /// The statement byte `at` is in (the innermost one).
    pub fn statement_at(&self, at: usize) -> Option<&Statement> {
        self.statements.iter().filter(|s| s.span.start <= at && at <= s.span.end).max_by_key(|s| s.depth)
    }

    /// The symbols with no parent, in text order: the outline's top level.
    pub fn roots(&self) -> Vec<usize> {
        (0..self.symbols.len()).filter(|&i| self.symbols[i].parent.is_none()).collect()
    }

    /// Scan the tokens of `src` (`tokens` ends in `Eof`; `comments` are the lexer's).
    pub fn build(src: &str, tokens: &[Token], comments: &[Span]) -> Symbols {
        let mut b = Builder { src, t: tokens, out: Symbols::default(), def_names: Vec::new(), by_name: Default::default() };
        b.scan();
        b.resolve();
        b.comment_folds(comments);
        b.out.folds.sort_by_key(|(s, _)| (s.start, s.end));
        b.out
    }
}

struct Builder<'a> {
    src: &'a str,
    t: &'a [Token],
    out: Symbols,
    /// The token index of every definition's name (not a use).
    def_names: Vec<usize>,
    /// Name -> the symbols of that name (filled before references are resolved).
    by_name: std::collections::BTreeMap<String, Vec<usize>>,
}

impl Builder<'_> {
    fn add(&mut self, sym: Symbol) -> usize {
        let i = self.out.symbols.len();
        if let Some(p) = sym.parent {
            self.out.symbols[p].children.push(i);
        }
        self.out.symbols.push(sym);
        i
    }

    fn tok(&self, i: usize) -> &Tok {
        &self.t[i.min(self.t.len() - 1)].tok
    }

    fn word_at(&self, i: usize) -> Option<&str> {
        self.t.get(i).and_then(word)
    }

    /// The `}` that closes the `{` at `open` (or the last token).
    fn close_of(&self, open: usize) -> usize {
        let mut depth = 0i64;
        for (k, tk) in self.t.iter().enumerate().skip(open) {
            match tk.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return k;
                    }
                }
                Tok::Eof => return k,
                _ => {}
            }
        }
        self.t.len() - 1
    }

    /// Read statements from `i` until the `}` that closes the block (or the end), inside `container` at `depth`.
    fn scan_block(&mut self, mut i: usize, depth: usize, container: Option<usize>) -> usize {
        loop {
            while matches!(self.tok(i), Tok::Newline) {
                i += 1;
            }
            match self.tok(i) {
                Tok::Eof => return i,
                Tok::RBrace if depth > 0 => return i,
                // a stray `}` at the top: skip it
                Tok::RBrace => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            i = self.statement(i, depth, container);
        }
    }

    fn scan(&mut self) {
        self.scan_block(0, 0, None);
    }

    /// One statement starting at token `i`: its symbols, and the statements of any block it opens. Returns the token after it.
    fn statement(&mut self, i: usize, depth: usize, container: Option<usize>) -> usize {
        let head = self.word_at(i).unwrap_or("").to_string();
        // the statement's own tokens: to the newline at this depth; a `{` opens a block whose statements are read inside
        let mut k = i;
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        loop {
            match self.tok(k) {
                Tok::Newline | Tok::Eof => break,
                Tok::RBrace => break,
                Tok::LBrace => {
                    let close = self.close_of(k);
                    blocks.push((k, close));
                    k = if matches!(self.tok(close), Tok::RBrace) { close + 1 } else { close };
                }
                _ => k += 1,
            }
        }
        let end = k;
        let last = end.saturating_sub(1).max(i);
        let span = self.t[i].span.to(self.t[last].span);
        let si = self.out.statements.len();
        self.out.statements.push(Statement { first: i, end, span, head: head.clone(), depth, container });
        let name_tok = i + 1;
        let name = self.word_at(name_tok).map(str::to_string);
        let body_of = |b: &Builder, (o, c): (usize, usize)| -> Span { Span::new(b.t[o].span.start, b.t[c].span.end) };
        let mut opened: Option<usize> = None;
        match head.as_str() {
            "section" if name.is_some() => {
                let n = name.unwrap();
                // `section drop 16 bars`, `section groove drop_len bars`
                let bars = self.t.get(i + 2).map(|t| t.span.text(self.src).to_string()).unwrap_or_default();
                let detail = if bars.is_empty() { String::new() } else { format!("{bars} bars") };
                let body = blocks.first().map(|b| body_of(self, *b));
                let s = self.add(Symbol { name: n, kind: Kind::Section, name_span: self.t[name_tok].span, full: span, detail, parent: container, children: Vec::new(), body });
                self.def_names.push(name_tok);
                opened = Some(s);
            }
            "let" if name.is_some() && matches!(self.tok(i + 2), Tok::Eq) => {
                let n = name.unwrap();
                let value = if end > i + 3 { self.t[i + 3].span.to(self.t[last].span).text(self.src).trim().to_string() } else { String::new() };
                let composition = matches!(self.word_at(i + 3), Some(w) if djlive::algebra::WORDS.contains(&w)) && (matches!(self.tok(i + 4), Tok::LParen) || self.word_at(i + 3) == Some("silence"));
                let kind = if composition { Kind::LetSection } else { Kind::Let };
                self.add(Symbol { name: n, kind, name_span: self.t[name_tok].span, full: span, detail: value, parent: container, children: Vec::new(), body: None });
                self.def_names.push(name_tok);
            }
            "def" if name.is_some() => {
                let n = name.unwrap();
                let body = blocks.first().map(|b| body_of(self, *b));
                let header_end = blocks.first().map_or(end, |b| b.0);
                let header = self.t[i].span.to(self.t[header_end.saturating_sub(1).max(i)].span);
                let sig = Span::new(self.t[name_tok].span.end, header.end).text(self.src).trim().to_string();
                let d = self.add(Symbol { name: n, kind: Kind::Def, name_span: self.t[name_tok].span, full: span, detail: sig, parent: container, children: Vec::new(), body });
                self.def_names.push(name_tok);
                // parameters: the words between `(` and `)`; ports: the words after `in:` / `out:`
                let mut j = name_tok + 1;
                let mut in_parens = false;
                let mut port: Option<&'static str> = None;
                while j < header_end {
                    match self.tok(j).clone() {
                        Tok::LParen => in_parens = true,
                        Tok::RParen => in_parens = false,
                        Tok::Word(w) if in_parens => {
                            let sp = self.t[j].span;
                            self.add(Symbol { name: w, kind: Kind::Param, name_span: sp, full: sp, detail: "parameter".into(), parent: Some(d), children: Vec::new(), body });
                            self.def_names.push(j);
                        }
                        Tok::Word(w) if (w == "in" || w == "out") && matches!(self.tok(j + 1), Tok::Colon) => {
                            port = Some(if w == "in" { "in" } else { "out" });
                            j += 1;
                        }
                        Tok::Word(w) if port.is_some() => {
                            let sp = self.t[j].span;
                            self.add(Symbol { name: w, kind: Kind::Port, name_span: sp, full: sp, detail: format!("{} port", port.unwrap()), parent: Some(d), children: Vec::new(), body });
                            self.def_names.push(j);
                        }
                        _ => {}
                    }
                    j += 1;
                }
                opened = Some(d);
            }
            "sample" if name.is_some() => {
                self.add(Symbol { name: name.unwrap(), kind: Kind::Sample, name_span: self.t[name_tok].span, full: span, detail: "sample".into(), parent: container, children: Vec::new(), body: None });
                self.def_names.push(name_tok);
            }
            "loop" if matches!(self.tok(i + 1), Tok::Str(_)) && self.word_at(i + 2) == Some("as") && self.word_at(i + 3).is_some() => {
                let n = self.word_at(i + 3).unwrap().to_string();
                self.add(Symbol { name: n, kind: Kind::Sample, name_span: self.t[i + 3].span, full: span, detail: "loop".into(), parent: container, children: Vec::new(), body: None });
                self.def_names.push(i + 3);
            }
            "import" if end > i + 1 => {
                let what = self.t[i + 1].span.text(self.src).trim_matches('"').to_string();
                self.add(Symbol { name: what, kind: Kind::Import, name_span: self.t[i + 1].span, full: span, detail: "import".into(), parent: container, children: Vec::new(), body: None });
            }
            "tempo" | "key" | "swing" => {
                let v = if end > i + 1 { self.t[i + 1].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() };
                self.add(Symbol { name: head.clone(), kind: Kind::Setting, name_span: self.t[i].span, full: span, detail: v, parent: container, children: Vec::new(), body: None });
            }
            // `arrange set: drop wubs then close`: a named arrangement (a name `then` and `start` use)
            // (`arrange set: { ... } loop 2`: a named arrangement with mutations, its block the container)
            "arrange" if name.is_some() && matches!(self.tok(i + 2), Tok::Colon) => {
                let (v, body) = match blocks.first() {
                    Some(b) => {
                        let (d, body) = self.arrange_block(*b, end, last);
                        (d, Some(body))
                    }
                    None => (if end > i + 3 { self.t[i + 3].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() }, None),
                };
                let arr = self.add(Symbol { name: name.unwrap(), kind: Kind::Arrangement, name_span: self.t[name_tok].span, full: span, detail: v, parent: container, children: Vec::new(), body });
                self.def_names.push(name_tok);
                if body.is_some() {
                    opened = Some(arr);
                }
            }
            // `arrange { intro build ~ acid rot 2 drop } loop 4 then outro end`: an arrangement with mutations; its
            // lines are sections and mutations, not statements, and it is the container completion reads
            "arrange" if !blocks.is_empty() => {
                let (detail, body) = self.arrange_block(blocks[0], end, last);
                let arr = self.add(Symbol { name: "arrange".into(), kind: Kind::Arrange, name_span: self.t[i].span, full: span, detail, parent: container, children: Vec::new(), body: Some(body) });
                opened = Some(arr);
            }
            "arrange" => {
                let v = if end > i + 1 { self.t[i + 1].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() };
                self.add(Symbol { name: "arrange".into(), kind: Kind::Arrange, name_span: self.t[i].span, full: span, detail: v, parent: container, children: Vec::new(), body: None });
            }
            // `start set`: the arrangement that plays first
            "start" if name.is_some() && end == i + 2 => {
                self.add(Symbol { name: "start".into(), kind: Kind::Setting, name_span: self.t[i].span, full: span, detail: name.unwrap(), parent: container, children: Vec::new(), body: None });
            }
            // `launch drop on pad(2)`: a pad brings a scene in
            "launch" if name.is_some() => {
                let detail = if end > i + 1 { self.t[i + 1].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() };
                self.add(Symbol { name: "launch".into(), kind: Kind::Setting, name_span: self.t[i].span, full: span, detail, parent: container, children: Vec::new(), body: None });
            }
            "evolve" => {
                let v = Span::new(self.t[i].span.end, blocks.first().map_or(span.end, |b| self.t[b.0].span.start)).text(self.src).trim().to_string();
                let body = blocks.first().map(|b| body_of(self, *b));
                let e = self.add(Symbol { name: "evolve".into(), kind: Kind::Evolve, name_span: self.t[i].span, full: span, detail: v, parent: container, children: Vec::new(), body });
                opened = Some(e);
            }
            "wire" => {
                let v = if end > i + 1 { self.t[i + 1].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() };
                self.add(Symbol { name: "wire".into(), kind: Kind::Wire, name_span: self.t[i].span, full: span, detail: v, parent: container, children: Vec::new(), body: None });
            }
            "repeat" | "set" | "mix" if !blocks.is_empty() => {
                let v = Span::new(self.t[i].span.end, self.t[blocks[0].0].span.start).text(self.src).trim().to_string();
                let body = Some(body_of(self, blocks[0]));
                let r = self.add(Symbol { name: head.clone(), kind: Kind::Block, name_span: self.t[i].span, full: span, detail: v, parent: container, children: Vec::new(), body });
                opened = Some(r);
            }
            _ => {
                // a line of parts: `deck A: kick x ~ | bass ...` (after the last `:` of its head, each `|` starts a part)
                self.parts(i, end, container);
            }
        }
        let _ = si;
        for (o, c) in &blocks {
            // a block that spans lines folds
            let sp = Span::new(self.t[*o].span.start, self.t[*c].span.end);
            if self.src[sp.start..sp.end.min(self.src.len())].contains('\n') {
                self.out.folds.push((Span::new(span.start, self.t[*c].span.start), FoldKind::Region));
            }
            self.scan_block(o + 1, depth + 1, opened.or(container));
        }
        end
    }

    /// An arrange block `{ ... }` (open, close) of a statement that ends at `end`: its outline detail (the sections in order,
    /// how many mutations, its ending) and its body.
    fn arrange_block(&self, (o, c): (usize, usize), end: usize, last: usize) -> (String, Span) {
        // a line is sections (`intro build`, `a, b`) or one mutation (`~ ...`, `@ ...`)
        let mut sections = Vec::new();
        let mut mutations = 0;
        let mut k = o + 1;
        while k < c {
            let mut j = k;
            while j < c && !matches!(self.tok(j), Tok::Newline) {
                j += 1;
            }
            if matches!(self.tok(k), Tok::Tilde | Tok::At) {
                mutations += 1;
            } else {
                sections.extend((k..j).filter_map(|x| self.word_at(x).map(str::to_string)));
            }
            k = j + 1;
        }
        let tail = if c + 1 < end { self.t[c + 1].span.to(self.t[last].span).text(self.src).to_string() } else { String::new() };
        let detail = format!("{}{}{}", sections.join(" "), if mutations > 0 { format!(" · {mutations} mutation{}", if mutations == 1 { "" } else { "s" }) } else { String::new() }, if tail.is_empty() { String::new() } else { format!(" · {tail}") });
        (detail, Span::new(self.t[o].span.start, self.t[c].span.end))
    }

    /// The parts of a pattern line: the role after the line's `:` (`deck A: kick ...`, `every 4 bars: deck A: bass ...`) and
    /// after each `|`.
    fn parts(&mut self, i: usize, end: usize, container: Option<usize>) {
        let roles = djlive::parse::ROLES;
        let deck_at = (i..end).find(|&k| self.word_at(k) == Some("deck") && matches!(self.word_at(k + 1), Some("A" | "B")) && matches!(self.tok(k + 2), Tok::Colon));
        let Some(d) = deck_at else { return };
        let deck = self.word_at(d + 1).unwrap().to_string();
        let mut k = d + 3;
        let mut starts = vec![k];
        while k < end {
            if matches!(self.tok(k), Tok::Pipe) {
                starts.push(k + 1);
            }
            k += 1;
        }
        for (n, &s) in starts.iter().enumerate() {
            let Some(role) = self.word_at(s) else { continue };
            if !roles.contains(&role) {
                continue;
            }
            let stop = starts.get(n + 1).map_or(end, |&e| e - 1);
            let last = stop.saturating_sub(1).max(s);
            let full = self.t[s].span.to(self.t[last].span);
            let detail = format!("deck {deck}");
            self.add(Symbol { name: role.to_string(), kind: Kind::Part, name_span: self.t[s].span, full, detail, parent: container, children: Vec::new(), body: None });
        }
    }

    /// Consecutive comment lines fold.
    fn comment_folds(&mut self, comments: &[Span]) {
        let mut run: Option<(Span, Span)> = None;
        let line_of = |at: usize| self.src[..at].matches('\n').count();
        for c in comments {
            match run {
                Some((first, last)) if line_of(c.start) == line_of(last.start) + 1 && self.src[last.end..c.start].trim().is_empty() => run = Some((first, *c)),
                Some((first, last)) => {
                    if line_of(last.start) > line_of(first.start) {
                        self.out.folds.push((first.to(last), FoldKind::Comment));
                    }
                    run = Some((*c, *c));
                }
                None => run = Some((*c, *c)),
            }
        }
        if let Some((first, last)) = run {
            if line_of(last.start) > line_of(first.start) {
                self.out.folds.push((first.to(last), FoldKind::Comment));
            }
        }
    }

    /// Every word and `$name` that names a symbol: a reference to it.
    fn resolve(&mut self) {
        for (i, s) in self.out.symbols.iter().enumerate() {
            if s.kind.is_name() {
                self.by_name.entry(s.name.clone()).or_default().push(i);
            }
        }
        let defs: std::collections::BTreeSet<usize> = self.def_names.iter().copied().collect();
        for &k in &self.def_names.clone() {
            let sp = self.t[k].span;
            if let Some(&si) = self.by_name.get(self.t[k].span.text(self.src)).and_then(|v| v.iter().find(|&&i| self.out.symbols[i].name_span == sp)) {
                self.out.refs.push(Reference { symbol: si, span: sp, is_def: true, dollar: false });
            }
        }
        for k in 0..self.t.len() {
            if defs.contains(&k) {
                continue;
            }
            let span = self.t[k].span;
            match &self.t[k].tok {
                Tok::Dollar(name) => {
                    // a parameter or a port of the def whose body this is
                    let Some(def) = self.enclosing_def(span.start) else { continue };
                    let hit = self.out.symbols[def].children.iter().copied().find(|&c| self.out.symbols[c].name == *name && matches!(self.out.symbols[c].kind, Kind::Param | Kind::Port));
                    if let Some(c) = hit {
                        self.out.refs.push(Reference { symbol: c, span, is_def: false, dollar: true });
                    }
                }
                Tok::Word(w) => {
                    let next = self.tok(k + 1);
                    let prev = if k > 0 { Some(self.tok(k - 1)) } else { None };
                    // a parameter's name (`vel=`), not its value
                    if matches!(next, Tok::Eq) && !matches!(self.tok(k + 2), Tok::Eq) {
                        continue;
                    }
                    // `bed.chords`: a port of the def before the dot
                    if matches!(prev, Some(Tok::Dot)) && k >= 2 {
                        if let Some(owner) = self.word_at(k - 2) {
                            let def = self.by_name.get(owner).and_then(|v| v.iter().copied().find(|&i| self.out.symbols[i].kind == Kind::Def));
                            if let Some(d) = def {
                                if let Some(&c) = self.out.symbols[d].children.iter().find(|&&c| self.out.symbols[c].kind == Kind::Port && self.out.symbols[c].name == *w) {
                                    self.out.refs.push(Reference { symbol: c, span, is_def: false, dollar: false });
                                }
                            }
                        }
                        continue;
                    }
                    // `then set`, `start set`: an arrangement, before a section of the same name
                    if matches!(prev, Some(Tok::Word(p)) if p == "then" || p == "start") {
                        if let Some(&ai) = self.by_name.get(w.as_str()).and_then(|v| v.iter().find(|&&i| self.out.symbols[i].kind == Kind::Arrangement)) {
                            self.out.refs.push(Reference { symbol: ai, span, is_def: false, dollar: false });
                            continue;
                        }
                    }
                    if let Some(si) = self.pick(w, span.start, matches!(next, Tok::LParen), matches!(next, Tok::Dot)) {
                        self.out.refs.push(Reference { symbol: si, span, is_def: false, dollar: false });
                    }
                }
                _ => {}
            }
        }
        self.out.refs.sort_by_key(|r| r.span.start);
    }

    fn enclosing_def(&self, at: usize) -> Option<usize> {
        self.out.symbols.iter().enumerate().filter(|(_, s)| s.kind == Kind::Def && s.body.is_some_and(|b| b.start < at && at < b.end)).max_by_key(|(_, s)| s.body.unwrap().start).map(|(i, _)| i)
    }

    /// The symbol a word at `at` names, by the place: a call is a def's; before a dot a def's; else a let (the latest one
    /// before it), a section, a composition, a sample.
    fn pick(&self, w: &str, at: usize, call: bool, dotted: bool) -> Option<usize> {
        let cands: Vec<(usize, &Symbol)> = self.by_name.get(w).into_iter().flatten().map(|&i| (i, &self.out.symbols[i])).filter(|(_, s)| !matches!(s.kind, Kind::Param | Kind::Port | Kind::Import)).collect();
        if cands.is_empty() {
            return None;
        }
        if call || dotted {
            return cands.iter().find(|(_, s)| s.kind == Kind::Def).map(|(i, _)| *i);
        }
        let rank = |k: Kind| match k {
            Kind::Let => 0,
            Kind::LetSection => 1,
            Kind::Section => 2,
            Kind::Sample => 3,
            Kind::Arrangement => 4,
            _ => 5,
        };
        let best = cands.iter().filter(|(_, s)| s.kind != Kind::Def).map(|(_, s)| rank(s.kind)).min()?;
        let same: Vec<&(usize, &Symbol)> = cands.iter().filter(|(_, s)| s.kind != Kind::Def && rank(s.kind) == best).collect();
        // a let: the last one defined before the use, else the first
        same.iter().filter(|(_, s)| s.name_span.start < at).max_by_key(|(_, s)| s.name_span.start).or_else(|| same.first()).map(|(i, _)| *i)
    }
}
