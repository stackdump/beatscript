//! `<cid>.drop`: a stored program's music as a reference (docs/BTW.md, "Borrow from a stored program").
//!
//! ```text
//! section hook from z4EBG9j9….drop           a section of it: its parts, its key, never its control lines
//! deck B: bass z4EBG9j9….drop inst=hoover    one part of it: the line's role (`.melody` for another), its pattern and params
//! let riff = z4EBG9j9….drop.bass             the same, named
//! ```
//!
//! The same slot a seed has (seed.rs): music from elsewhere, written out as plain BeatScript before the program is lowered, so the
//! compiler, the page and the phone only ever see what a person could have typed. What a seed fixes by a version, a stored
//! program fixes by its hash: the CID is in the canonical text, so the program's own CID never depends on whether the source
//! can be found, and a cycle is impossible (a text cannot contain its own hash).
//!
//! The language never fetches (R-06): the source is found in what the host gives `check`, its snippets (the programs a link
//! carries, then the host's own: share.rs, set.rs `Inline`), each re-hashed against its CID. The source is compiled in a plain
//! environment (no registry, no instrument catalogue), so the music a slice has is the same on every host; whether the slice
//! *checks* is then up to the borrower's own host, at the reference. A program no host has is [`env::not_here`], at the reference.
//!
//! What is brought: a **section**: its decks' parts as they play in its first stretch (the canonical part, its lets and defs
//! already worked out), its length in bars and, when it sounds in another key than this program's, its `key`. Never its tempo, and
//! never its control lines (filters, reverb, `move`, `mix`, `duck`, `sing`, `at bar`, `evolve`): they name sections and times that do
//! not travel, so they stay behind, with a warning that names them. A **part**: its pattern and its params (`kit` `vel` `inst`
//! `oct` `len` ...), and for a part with chance the seed its dice come from, so it plays the same hits on any deck; the line's own
//! params win. A **let** is the pattern alone, read in the key it lands in.
//!
//! Pure: no I/O and no clock.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use djengine::rights::short_cid;

use crate::ast::{Body, Deck, StmtKind};
use crate::compile::{Compiled, DeckIr, PartIr};
use crate::diag::{Diagnostic, Span};
use crate::env::{not_here, Env, NOT_HERE_HELP};
use crate::lex::{lex, Tok, Token};
use crate::music::Key;
use crate::parse::{nearest, ROLES};
use crate::seed::Expansion;

/// How many programs deep a borrow may go (a borrowed program that borrows from another ...): as an `import`'s.
pub const MAX_DEPTH: usize = 4;

thread_local! {
    /// How deep the borrows being resolved on this thread are.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// `z` and 20 to 80 base58 characters: a program's CID (the store's `is_cid`).
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    b.first() == Some(&b'z') && (21..=81).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// A word that starts with a CID and goes on with `.section` and `.role` only: a reference, one word to the parser.
pub fn is_ref_word(w: &str) -> bool {
    w.split('.').next().is_some_and(is_cid)
}

/// A CID and the `.name`s that touch it, one word (the canonical spelling: `z4EBG….drop.bass`), as a seed reference is one word
/// (seed.rs `collapse`): a CID is not a name (parse.rs `reserved`), so nothing else is read that way. A bare CID stays as it is.
pub(crate) fn collapse(toks: Vec<Token>) -> Vec<Token> {
    if !toks.iter().any(|t| matches!(&t.tok, Tok::Word(w) if is_cid(w))) {
        return toks;
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Word(w) = &toks[i].tok {
            if is_cid(w) {
                let (end, _) = extent(&toks, i);
                if end > i + 1 {
                    let text: String = toks[i..end].iter().map(|t| if let Tok::Word(n) = &t.tok { n.clone() } else { ".".to_string() }).collect();
                    out.push(Token { tok: Tok::Word(text), span: toks[i].span.to(toks[end - 1].span) });
                    i = end;
                    continue;
                }
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// The tokens `toks[i]` (a CID) and its `.name`s occupy: where they end, and the names.
fn extent(toks: &[Token], i: usize) -> (usize, Vec<(String, Span)>) {
    let mut j = i + 1;
    let mut names = Vec::new();
    while j + 1 < toks.len() && toks[j].tok == Tok::Dot && toks[j].span.start == toks[j - 1].span.end && toks[j + 1].span.start == toks[j].span.end {
        let Tok::Word(n) = &toks[j + 1].tok else { break };
        names.push((n.clone(), toks[j + 1].span));
        j += 2;
    }
    (j, names)
}

/// A reference, as read: the CID, a section, a role.
#[derive(Clone, Debug, PartialEq)]
struct Ref {
    cid: String,
    section: Option<String>,
    role: Option<String>,
}

impl Ref {
    /// The canonical spelling: `z4EBG….drop.bass`.
    fn text(&self) -> String {
        let mut s = self.cid.clone();
        for p in [&self.section, &self.role].into_iter().flatten() {
            s.push('.');
            s.push_str(p);
        }
        s
    }
}

/// Where a reference stands decides what it is written out as.
enum Form {
    /// After a role word of a part: from that word to the end of the reference. The params the line already says.
    Part { role: &'static str, from: usize, said: BTreeSet<String> },
    /// `let NAME = REF`: the reference alone.
    Let { name: String },
    /// `section NAME from REF`: from the `section` word.
    Section { name: String, from: usize },
}

struct Run {
    span: Span,
    form: Result<Form, Diagnostic>,
    r: Result<Ref, Diagnostic>,
}

/// Where a CID stands.
enum Place {
    /// Where a reference goes.
    Is(Form),
    /// A reference where it cannot be, and why.
    Wrong(Diagnostic),
    /// Nowhere that takes one: a bare CID is an `import`'s or a `play`'s.
    Nowhere,
}

fn word(t: &Tok, w: &str) -> bool {
    matches!(t, Tok::Word(x) if x == w)
}

const SHAPE: &str = "section hook from <cid>.drop   deck B: bass <cid>.drop inst=hoover   let riff = <cid>.drop.bass   (a bare <cid> is a program with no sections)";

/// Every reference in `toks`: a CID where a pattern goes (after a part's role word, after `let NAME =`), or after `section NAME from`;
/// or a CID with `.names` anywhere (which is never anything else). A bare CID elsewhere is an `import` or a `play`'s, and is left alone.
fn runs(toks: &[Token]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let Tok::Word(w) = &toks[i].tok else {
            i += 1;
            continue;
        };
        if !is_cid(w) {
            i += 1;
            continue;
        }
        let (end, names) = extent(toks, i);
        let span = toks[i].span.to(toks[end - 1].span);
        let form = match form_of(toks, i, span) {
            Place::Is(f) => Ok(f),
            Place::Wrong(d) => Err(d),
            // (a CID with `.names` is a reference whatever stands before it, and is said not to be where it stands)
            Place::Nowhere if end > i + 1 => Err(Diagnostic::error(span, "a stored program goes where a pattern goes, or makes a section").help(SHAPE)),
            // a bare CID that is no pattern and makes no section (`import z...`, `play z...`) is no reference
            Place::Nowhere => {
                i = end;
                continue;
            }
        };
        let r = read(w, &names, span);
        out.push(Run { span, form, r });
        i = end;
    }
    out
}

/// What the `.names` after a CID mean: a role is one of the eight, anything else is a section; a section comes first.
fn read(cid: &str, names: &[(String, Span)], span: Span) -> Result<Ref, Diagnostic> {
    let is_role = |n: &str| ROLES.contains(&n);
    let bad_role = |n: &str, at: Span| {
        let mut d = Diagnostic::error(at, format!("`.{n}`: a part of a stored program is one of its roles")).expected(&ROLES).found(format!("`{n}`")).help(SHAPE);
        if let Some(near) = nearest(n, ROLES) {
            d = d.help(format!("did you mean `.{near}`?"));
        }
        d
    };
    match names {
        [] => Ok(Ref { cid: cid.to_string(), section: None, role: None }),
        [(n, _)] if is_role(n) => Ok(Ref { cid: cid.to_string(), section: None, role: Some(n.clone()) }),
        [(n, _)] => Ok(Ref { cid: cid.to_string(), section: Some(n.clone()), role: None }),
        [(a, sa), (b, sb)] => {
            if is_role(a) {
                return Err(Diagnostic::error(*sa, format!("`.{a}.{b}`: a section comes before a part")).help(format!("{cid}.{b}.{a}")));
            }
            if !is_role(b) {
                return Err(bad_role(b, *sb));
            }
            Ok(Ref { cid: cid.to_string(), section: Some(a.clone()), role: Some(b.clone()) })
        }
        _ => Err(Diagnostic::error(span, "a reference to a stored program is a CID, a section and a part at most").help(SHAPE)),
    }
}

/// Where the CID at `toks[i]` stands.
fn form_of(toks: &[Token], i: usize, span: Span) -> Place {
    let line_start = toks[..i].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |k| k + 1);
    let head: Vec<&Tok> = toks[line_start..i].iter().map(|t| &t.tok).collect();
    match head.as_slice() {
        [s, Tok::Word(name), f] if word(s, "section") && word(f, "from") => return Place::Is(Form::Section { name: name.clone(), from: line_start }),
        [l, Tok::Word(name), Tok::Eq] if word(l, "let") => return Place::Is(Form::Let { name: name.clone() }),
        _ => {}
    }
    let prev = i.checked_sub(1).map(|k| &toks[k].tok);
    if let Some(Tok::Word(w)) = prev {
        if let Some(role) = ROLES.iter().copied().find(|r| r == w) {
            let mut said = BTreeSet::new();
            let mut j = i + 1;
            while j < toks.len() && !matches!(toks[j].tok, Tok::Pipe | Tok::Newline | Tok::Eof | Tok::RBrace | Tok::Comma | Tok::RParen) {
                if let (Tok::Word(k), Some(Tok::Eq)) = (&toks[j].tok, toks.get(j + 1).map(|t| &t.tok)) {
                    said.insert(k.clone());
                }
                j += 1;
            }
            return Place::Is(Form::Part { role, from: i - 1, said });
        }
    }
    if matches!(prev, Some(Tok::Num { .. })) && i >= 3 && toks[i - 2].tok == Tok::Slash {
        return Place::Wrong(Diagnostic::error(span, "a stored program's part plays its own bars: it takes no cycle of its own").help("drop the `/N`: the part brings its own cycle"));
    }
    Place::Nowhere
}

/// A stored program, compiled for its parts.
struct Source {
    cid: String,
    comp: Compiled,
    /// The sections it writes, played or not (for the message that says which are not).
    written: Vec<String>,
}

type Cache = Vec<(String, Rc<Source>)>;

/// The program `cid` names, from what the host gives `check`: re-hashed, then compiled in a plain environment (no registry, no
/// catalogue: the same music on every host), with the snippets the host gave so that what it borrows from is found too.
fn source(cache: &mut Cache, env: &Env, cid: &str, at: Span) -> Result<Rc<Source>, Diagnostic> {
    if let Some((_, s)) = cache.iter().find(|(c, _)| c == cid) {
        return Ok(s.clone());
    }
    let missing = || Diagnostic::error(at, not_here(cid)).help(NOT_HERE_HELP);
    let Some(snips) = env.snippets else { return Err(missing()) };
    let text = match snips.resolve(cid) {
        Ok((got, text)) if got == cid => text,
        Ok(_) => return Err(Diagnostic::error(at, format!("the program {} is not the program it names: refused", short_cid(cid)))),
        Err(e) if e.contains("not the program it names") => return Err(Diagnostic::error(at, e)),
        Err(_) => return Err(missing()),
    };
    if crate::compile::program_cid(&text) != cid {
        return Err(Diagnostic::error(at, format!("the program {} is not the program it names: refused", short_cid(cid))));
    }
    let depth = DEPTH.with(Cell::get);
    if depth >= MAX_DEPTH {
        return Err(Diagnostic::error(at, format!("borrows nest more than {MAX_DEPTH} deep")).help("borrow from the program that has the music itself"));
    }
    DEPTH.with(|d| d.set(depth + 1));
    let plain = Env { registry: None, catalog: None, snippets: env.snippets, assets: None, private: false, voices: None, ontology: None, bank: None };
    let checked = crate::compile::check_with(&text, &plain);
    DEPTH.with(|d| d.set(depth));
    let Some(comp) = checked.compiled else {
        let first = checked.diagnostics.iter().find(|d| d.is_error());
        let said = first.map_or(String::new(), |d| format!(": {} (line {})", d.message, d.position(&text).0));
        return Err(Diagnostic::error(at, format!("the program {} does not compile{said}", short_cid(cid))).help("it can be borrowed from once it checks"));
    };
    let written = crate::parse::parse(&comp.text).program.stmts.iter().filter_map(|s| if let StmtKind::Section(b) = &s.kind { Some(b.name.text.clone()) } else { None }).collect();
    let s = Rc::new(Source { cid: cid.to_string(), comp, written });
    cache.push((cid.to_string(), s.clone()));
    Ok(s)
}

/// The sections a program plays, in order (a stack's placement is not one of them).
fn played(c: &Compiled) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in &c.segments {
        if s.header.is_some() && !s.chain && !s.name.contains('+') && !out.contains(&s.name) {
            out.push(s.name.clone());
        }
    }
    out
}

/// What of a stored program a reference reaches: a section's first stretch (its decks, its length, its key), or the program's own
/// decks when it has no sections.
struct Scope<'a> {
    /// The section's name; None for a program with no sections.
    name: Option<String>,
    decks: &'a BTreeMap<Deck, DeckIr>,
    bars: u32,
    key: Option<Key>,
}

/// The scope `r` names in `src`, or why not.
fn scope<'a>(src: &'a Source, r: &Ref, at: Span) -> Result<Scope<'a>, Diagnostic> {
    let c = &src.comp;
    if c.set.is_some() {
        return Err(Diagnostic::error(at, format!("{} is a set: it has no parts of its own to borrow", short_cid(&src.cid))).help("borrow from one of its tracks"));
    }
    let sections = played(c);
    let list = |v: &[String]| v.join(", ");
    match &r.section {
        None if !sections.is_empty() => Err(Diagnostic::error(at, format!("a bare {} is a program with no sections; this one has {}", short_cid(&src.cid), list(&sections))).help(format!("name one: {}.{}", src.cid, sections[0]))),
        None => Ok(Scope { name: None, decks: &c.decks, bars: 0, key: None }),
        Some(name) => {
            let Some(k) = c.segments.iter().position(|s| s.header.is_some() && !s.chain && &s.name == name) else {
                if src.written.contains(name) {
                    return Err(Diagnostic::error(at, format!("section `{name}` of {} is written but never played", short_cid(&src.cid))).help(if sections.is_empty() { "it has no section that plays".to_string() } else { format!("its sections that play: {}", list(&sections)) }));
                }
                let mut d = Diagnostic::error(at, format!("{} has no section `{name}`", short_cid(&src.cid)));
                d = d.help(match (nearest(name, sections.iter().map(String::as_str)), sections.is_empty()) {
                    (Some(n), _) => format!("did you mean `{n}`? Its sections: {}", list(&sections)),
                    (None, false) => format!("its sections: {}", list(&sections)),
                    (None, true) => format!("it has no sections: borrow a part of it, `bass {}`", src.cid),
                });
                return Err(d);
            };
            let seg = &c.segments[k];
            // a section that evolves, or is too long for one net, is several stretches: its length is all of them
            let bars = seg.bars + c.segments[k + 1..].iter().take_while(|s| s.chain).map(|s| s.bars).sum::<u32>();
            Ok(Scope { name: Some(name.clone()), decks: &seg.decks, bars, key: c.section_keys.get(name).copied() })
        }
    }
}

impl Scope<'_> {
    fn what(&self, src: &Source) -> String {
        match &self.name {
            Some(n) => format!("`{n}` of {}", short_cid(&src.cid)),
            None => short_cid(&src.cid),
        }
    }

    /// The parts of this scope that are roles of the language (a sung `vox` part is not one), with the deck each is on.
    fn parts(&self) -> Vec<(Deck, &PartIr)> {
        self.decks.iter().flat_map(|(d, ir)| ir.parts.iter().filter(|p| ROLES.contains(&p.role.as_str())).map(move |p| (*d, p))).collect()
    }
}

/// The part of `role` in `scope`: one, or why not.
fn part_of<'a>(scope: &'a Scope, src: &Source, role: &str, at: Span) -> Result<&'a PartIr, Diagnostic> {
    let all = scope.parts();
    let hits: Vec<&(Deck, &PartIr)> = all.iter().filter(|(_, p)| p.role == role).collect();
    match hits.as_slice() {
        [(_, p)] => Ok(p),
        [] => {
            let have: Vec<&str> = all.iter().map(|(_, p)| p.role.as_str()).collect();
            let plays = if have.is_empty() { "no parts".to_string() } else { format!("it plays: {}", have.join(", ")) };
            Err(Diagnostic::error(at, format!("{} has no `{role}`; {plays}", scope.what(src))).help(if have.is_empty() { "borrow another section".to_string() } else { format!("a part of it: `.{}`", have[0]) }))
        }
        _ => {
            let d: Vec<String> = hits.iter().map(|(d, _)| format!("deck {}", d.name())).collect();
            Err(Diagnostic::error(at, format!("`{role}` is on {} in {}", d.join(" and "), scope.what(src))).help(match &scope.name {
                Some(n) => format!("borrow the section: section hook from {}.{n}", src.cid),
                None => "it plays on more than one deck".to_string(),
            }))
        }
    }
}

/// The part as a person would write it after a role word: its pattern, and its params as `name=value` (sorted), and the cycle it
/// is written with (`/12`, or nothing). From the canonical text of the part, read back (so it is exactly what the source plays).
fn written(part: &PartIr) -> Option<(String, String, Vec<(String, String)>)> {
    let parsed = crate::parse::parse(&format!("deck A: {}\n", part.text));
    let ast = parsed.program.stmts.into_iter().find_map(|s| match s.kind {
        StmtKind::Line(l) => match l.body {
            Body::Parts(ps) => ps.into_iter().next(),
            _ => None,
        },
        _ => None,
    })?;
    let params = ast.params.iter().map(|p| (p.name.text.clone(), crate::print::param(p))).collect();
    Some((crate::meter::cycle_suffix(&ast), crate::print::seq(&ast.pattern), params))
}

/// The word the control line begins with: its target and what it does (`master: filter`, `deck A: reverb`, `duck`).
fn kind_of(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    if let Some(k) = words.iter().position(|w| *w == "master:") {
        return format!("master: {}", words.get(k + 1).copied().unwrap_or(""));
    }
    if let Some(k) = words.iter().position(|w| *w == "deck") {
        if words.get(k + 1).is_some_and(|d| d.ends_with(':')) {
            return format!("deck {} {}", words[k + 1], words.get(k + 2).copied().unwrap_or("")).trim().to_string();
        }
    }
    words.first().map_or(String::new(), |w| w.trim_end_matches(':').to_string())
}

/// The control lines a section has, by kind, in order, once each (they stay behind).
fn left_behind(src: &Source, name: &str) -> Vec<String> {
    let c = &src.comp;
    let prefix = format!("[{name}@");
    let mut kinds: Vec<String> = Vec::new();
    for ctl in c.controls.iter().filter(|ctl| ctl.key.starts_with(&prefix)) {
        let line = ctl.span.text(&c.source).lines().next().unwrap_or("").trim();
        let k = kind_of(line);
        if !k.is_empty() && !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    if c.evolves.iter().any(|e| e.scope == name) {
        kinds.push("evolve".to_string());
    }
    kinds
}

/// The key this program sets for itself (`key am` at the head of a line), else C major.
fn own_key(toks: &[Token]) -> Key {
    for (i, t) in toks.iter().enumerate() {
        if matches!(&t.tok, Tok::Word(w) if w == "key") && (i == 0 || toks[i - 1].tok == Tok::Newline) {
            if let Some(Tok::Word(k)) = toks.get(i + 1).map(|t| &t.tok) {
                if let Some(k) = Key::parse(k) {
                    return k;
                }
            }
        }
    }
    Key::C_MAJOR
}

/// Every reference of `src` that reads, by CID, once each, in order: what a store credits like an `import` (live_store.rs).
pub fn cids_of(src: &str) -> Vec<String> {
    let norm = crate::lex::normalise_input(src);
    let toks = lex(&norm).tokens;
    let mut out: Vec<String> = Vec::new();
    for r in runs(&toks) {
        if let Ok(r) = r.r {
            if !out.contains(&r.cid) {
                out.push(r.cid);
            }
        }
    }
    out
}

/// `src` with every reference to a stored program written out as BeatScript, and the programs it drew on (each after what it
/// draws on), or None when it has none. Errors are at the references.
pub fn expand(src: &str, env: &Env) -> Option<Result<(Expansion, Vec<(String, String)>), Vec<Diagnostic>>> {
    let norm = crate::lex::normalise_input(src);
    let src: &str = &norm;
    let lexed = lex(src);
    let toks = &lexed.tokens;
    let rs = runs(toks);
    if rs.is_empty() {
        return None;
    }
    let mut errors: Vec<Diagnostic> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    let mut pieces: Vec<(Span, String, String)> = Vec::new();
    let mut uses: Vec<(String, String)> = Vec::new();
    let mut cache: Cache = Vec::new();
    let own = own_key(toks);
    for run in &rs {
        let form = match &run.form {
            Ok(f) => f,
            Err(d) => {
                errors.push(d.clone());
                continue;
            }
        };
        let r = match &run.r {
            Ok(r) => r,
            Err(d) => {
                errors.push(d.clone());
                continue;
            }
        };
        let src_ = match source(&mut cache, env, &r.cid, run.span) {
            Ok(s) => s,
            Err(d) => {
                errors.push(d);
                continue;
            }
        };
        // what it drew on, then itself
        for (c, t) in &src_.comp.plays {
            if !uses.iter().any(|(x, _)| x == c) {
                uses.push((c.clone(), t.clone()));
            }
        }
        if !uses.iter().any(|(x, _)| *x == src_.cid) {
            uses.push((src_.cid.clone(), src_.comp.text.clone()));
        }
        let refx = r.text();
        let sc = match scope(&src_, r, run.span) {
            Ok(s) => s,
            Err(d) => {
                errors.push(d);
                continue;
            }
        };
        match form {
            Form::Section { name, from } => {
                let Some(section) = &sc.name else {
                    errors.push(
                        Diagnostic::error(run.span, format!("`section {name} from {}` needs one of its sections", short_cid(&r.cid)))
                            .help(format!("section {name} from {}.<section>   (this program has no sections: borrow a part, `bass {}`)", r.cid, r.cid)),
                    );
                    continue;
                };
                let (indent, only) = ("  ", r.role.as_deref());
                let mut body = Vec::new();
                for (deck, ir) in sc.decks.iter() {
                    let items: Vec<&PartIr> = ir.parts.iter().filter(|p| ROLES.contains(&p.role.as_str()) && only.map_or(true, |o| p.role == o)).collect();
                    for (k, p) in items.iter().enumerate() {
                        body.push(if k == 0 { format!("{indent}deck {}: {}", deck.name(), p.text) } else { format!("{indent}{}| {}", " ".repeat("deck A: ".len() - 2), p.text) });
                    }
                }
                if body.is_empty() {
                    errors.push(Diagnostic::error(run.span, format!("section `{section}` of {} plays no parts{}", short_cid(&r.cid), only.map_or(String::new(), |o| format!(" of `{o}`")))).help("borrow a section that has parts"));
                    continue;
                }
                if sc.bars == 0 || sc.bars > 256 {
                    errors.push(Diagnostic::error(run.span, format!("section `{section}` of {} is {} bars: a section is 1 to 256", short_cid(&r.cid), sc.bars)));
                    continue;
                }
                let mut out = format!("# from {refx}\nsection {name} {} bars {{\n", sc.bars);
                if let Some(k) = sc.key.filter(|k| *k != own) {
                    out.push_str(&format!("{indent}key {}\n", k.text()));
                }
                for l in body {
                    out.push_str(&l);
                    out.push('\n');
                }
                out.push('}');
                let behind = left_behind(&src_, section);
                if !behind.is_empty() {
                    warnings.push(
                        Diagnostic::warning(run.span, format!("section `{section}` of {} has control lines ({}) that a borrowed section does not bring", short_cid(&r.cid), behind.join(", ")))
                            .help("write the ones you want in this section, for example: at bar 5: master: reverb 0.15"),
                    );
                }
                pieces.push((toks[*from].span.to(run.span), out, refx));
            }
            Form::Part { role, from, said } => {
                let want = r.role.as_deref().unwrap_or(role);
                let part = match part_of(&sc, &src_, want, run.span) {
                    Ok(p) => p,
                    Err(d) => {
                        errors.push(d);
                        continue;
                    }
                };
                let Some((cycle, pattern, params)) = written(part) else {
                    errors.push(Diagnostic::error(run.span, format!("the {want} of {} could not be read back", short_cid(&r.cid))));
                    continue;
                };
                let mut keep: Vec<(String, String)> = params.into_iter().filter(|(n, _)| !said.contains(n)).collect();
                // a part with chance draws its dice from a seed that is its deck's and its text's: written down, it plays the same hits here
                if let (Some(seed), true) = (part.seed, part.chance) {
                    if !keep.iter().any(|(n, _)| n == "seed") && !said.contains("seed") && !part_says(part, "seed") {
                        keep.push(("seed".to_string(), format!("seed={seed}")));
                    }
                }
                keep.sort();
                let params: String = keep.iter().map(|(_, p)| format!(" {p}")).collect();
                // (a part with a cycle of its own, `hihat/12`, is written from its role word: the cycle touches it; any other, in place of
                // the reference alone, so an error says it is the reference)
                if cycle.is_empty() {
                    pieces.push((run.span, format!("{pattern}{params}"), refx));
                } else {
                    pieces.push((toks[*from].span.to(run.span), format!("{role}{cycle} {pattern}{params}"), refx));
                }
            }
            Form::Let { name } => {
                let Some(want) = r.role.as_deref() else {
                    let at = r.section.as_ref().map_or(String::new(), |s| format!(".{s}"));
                    errors.push(Diagnostic::error(run.span, "a `let` of a stored program names one part: say which").help(format!("let {name} = {}{at}.bass", r.cid)));
                    continue;
                };
                let part = match part_of(&sc, &src_, want, run.span) {
                    Ok(p) => p,
                    Err(d) => {
                        errors.push(d);
                        continue;
                    }
                };
                let Some((cycle, pattern, _)) = written(part) else {
                    errors.push(Diagnostic::error(run.span, format!("the {want} of {} could not be read back", short_cid(&r.cid))));
                    continue;
                };
                if !cycle.is_empty() {
                    errors.push(Diagnostic::error(run.span, format!("the {want} of {} has a cycle of its own ({}): a `let` is a pattern of bars", short_cid(&r.cid), cycle.trim_start_matches('/'))).help(format!("borrow it as a part: {want} {}", r.text())));
                    continue;
                }
                if part.chance {
                    warnings.push(Diagnostic::warning(run.span, format!("the {want} of {} has chance in it: as a `let` it draws its own dice where it is used", short_cid(&r.cid))).help("borrow it as a part to keep the source's"));
                }
                pieces.push((run.span, pattern, refx));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    let mut text = String::with_capacity(src.len() * 2);
    let mut edits = Vec::new();
    let mut refs = Vec::new();
    let mut at = 0;
    pieces.sort_by_key(|(sp, _, _)| sp.start);
    for (sp, rep, r) in pieces {
        text.push_str(&src[at..sp.start]);
        let s = text.len();
        text.push_str(&rep);
        edits.push((sp, Span::new(s, text.len())));
        refs.push(r);
        at = sp.end;
    }
    text.push_str(&src[at..]);
    Some(Ok((Expansion { text, edits, refs, warnings }, uses)))
}

/// Does the source part say `name=` itself (its params, as written)?
fn part_says(part: &PartIr, name: &str) -> bool {
    written(part).is_some_and(|(_, _, ps)| ps.iter().any(|(n, _)| n == name))
}

/// A program with its templates, its seeds and its stored references written out, as text: what `expand` (the MCP tool,
/// `btwd expand`) answers: the text, the references in the order they were written out in (templates, seeds, then stored
/// programs) and the warnings of the templates and seeds. Errors are at the references of `src`.
pub fn expand_text(src: &str, env: &Env) -> Option<Result<(String, Vec<String>, Vec<Diagnostic>), Vec<Diagnostic>>> {
    // a place in the text of stage `n` as a place in `src`
    fn back_through(stages: &[Expansion], mut sp: Span) -> Span {
        for st in stages.iter().rev() {
            sp = st.back(sp);
        }
        sp
    }
    let mut text = src.to_string();
    let mut refs: Vec<String> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    let mut stages: Vec<Expansion> = Vec::new();
    let mut any = false;
    if let Some(r) = crate::template::expand(&text, env) {
        let e = match r {
            Ok(e) => e,
            Err(ds) => return Some(Err(ds)),
        };
        any = true;
        text = e.text.clone();
        refs.extend(e.refs.iter().cloned());
        warnings.extend(e.warnings.iter().cloned());
        stages.push(e);
    }
    if let Some(r) = crate::seed::expand(&text) {
        let e = match r {
            Ok(e) => e,
            Err(ds) => return Some(Err(ds.into_iter().map(|mut d| { d.span = back_through(&stages, d.span); d }).collect())),
        };
        any = true;
        text = e.text.clone();
        refs.extend(e.refs.iter().cloned());
        let at = stages.len();
        warnings.extend(e.warnings.iter().cloned().map(|mut d| { d.span = back_through(&stages[..at], d.span); d }));
        stages.push(e);
    }
    match expand(&text, env) {
        None => any.then(|| Ok((text, refs, warnings))),
        Some(Err(errs)) => Some(Err(errs.into_iter().map(|mut d| { d.span = back_through(&stages, d.span); d }).collect())),
        Some(Ok((e, _))) => {
            refs.extend(e.refs);
            Some(Ok((e.text, refs, warnings)))
        }
    }
}
