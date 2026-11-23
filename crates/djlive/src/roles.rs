//! Roles: a form template's parts bound to sources (docs/BEATSCRIPT.md "Phrase templates", long note docs/beatscript/ext-roles.md).
//!
//! ```text
//! arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 section=drop part=bass)
//! arrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy, ground=[d2 ~ a1 ~ b1 ~ f#1 ~])
//! arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876))
//! arrange from template(cross, @psalm42_deep_calleth, a=gen@1 dubstep seed=54876, b=gen@1 house seed=7)
//! ```
//!
//! **The form supplies the structure; the sources fill its roles.** A form names its roles (the passacaglia's `ground` and
//! `upper`, the canon's `subject` and `ground`, ...: [`specs`]); any of them can be bound to a source, and a role nobody binds
//! keeps the form's own material, so an unbound template is the text it always was. Everything a binding writes is structure,
//! native BeatScript text: a role is a let (a let a bar, `NAME_1` ... and `NAME = <NAME_1 ...>`) or a run of sections, and the
//! form's sections and mutation plan read it by name (`~ subject_* rot 2`). The sources:
//!
//! - an engine call, `gen@1 dubstep seed=54876 [section=drop] [part=bass]`: written as a `let NAME = gen@1 ...` line (or a
//!   `section NAME from gen@1 ...`), so the call stays a pinned reference in the written-out text and the gen stage writes its
//!   notes after this one (compile.rs: template, seed, gen, stored). The form fills in what it leaves out: the role's own part,
//!   and the first section that plays it (the drop or the chorus first);
//! - a public-domain theme by id (`ode_to_joy`): its bars, moved into the key the role plays in (to the relative key when its
//!   mode is the other one);
//! - notes, a bar each: `[d2 ~ a1 ~]`, `<[d2 ~ a1 ~] [g1 ~ a1 ~]>`;
//! - a stored program's part or section by CID (`<cid>.drop.bass`, `<cid>.drop`), and a whole stored program (`<cid>`: its
//!   sections, in the order it plays them) where a role takes sections;
//! - another template (`template(passacaglia, ground=...)`, no phrase: the phrase is the outer form's): its sections, named
//!   after the role (`ep_ground`), and its mutation plan, spread over the places the role plays in;
//! - one section of another template (`template(cross).both`, `.both.arp`): where a line goes, the notes the section's part
//!   plays as it first plays, moved into the role's key; where sections go, that section as it first plays;
//! - a stored set (`<cid>` of a `set { play ... }`): where sections go, its tracks in order, each its sections; its track
//!   (`<cid>.track2`) is that track's program, so `<cid>.track2.drop.bass` is one part of it.
//!
//! **The form extensions** (docs/beatscript/forms.md "Extensions"): a section where a line goes gives the role its own part of it
//! (`subject=<cid>.both` is the both section's melody); a seed role's parts are bound one at a time (`a.bass=[d2 ~ a1 ~]`);
//! a form takes numbers too ([`params`]: the canon's `delay=` and `interval=`); and the quodlibet names its roles (`a`, `b`,
//! `ground`, `third`). Each has a default that keeps every reference that does not say it written out as it was.
//!
//! **Rules.** A source plays in the key of the sections it fills: the program's at the top (a `gen` call is fitted to it, a
//! theme moved into it), an inner form's own (its sections open with its `key`, its calls say it). Everything is pinned: the
//! same text is the same written-out program and the same nets. Composition stays total: forms nest [`MAX_NESTING`] deep, a
//! line is at most [`MAX_LINE_BARS`] bars, and a role that takes sections takes at most [`MAX_SECTION_BARS`] bars of them.
//!
//! Pure: no I/O and no clock (a `gen` call is the installed generator's, a pure function; a stored program is the host's text).

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::gen::{GenCall, Host};
use crate::lex::{lex, Tok, Token};
use crate::music::Key;
use crate::parse::{nearest, ROLES};
use crate::template::Template;

/// How deep templates nest: a form, a form in one of its roles, and one more.
pub const MAX_NESTING: usize = 3;
/// The most bars a line (a role that is a let) may be.
pub const MAX_LINE_BARS: usize = 16;
/// The most bars a role that takes sections takes, one pass of them: a whole seed of standard length, not a long one.
pub const MAX_SECTION_BARS: u32 = 128;
/// How deep the reader follows `template(` inside `template(` before it stops (the nesting cap is said when it is written out).
const MAX_READ_DEPTH: usize = 8;

const BINDING: &str = "ROLE=SOURCE after the phrase: template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass); a source is a gen call, a theme id (ode_to_joy), notes ([d2 ~ a1 ~]), a stored program's <cid>.drop.bass, a stored set's <cid>.track2, template(NAME, ROLE=SOURCE) or one section of it, template(cross).both";

// ---------------------------------------------------------------- the reference

/// A role of a template bound to a source: `ground=gen@1 dubstep seed=54876 part=bass`. A part of a seed role is bound as
/// `a.bass=...` (`role` is then `a.bass`), and a form's number as `delay=2` (the source a [`Source::Number`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub role: String,
    pub role_span: Span,
    pub source: Source,
    pub source_span: Span,
}

/// What a role is bound to.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// An engine call of the seeded generator (gen.rs).
    Gen(GenCall),
    /// A stored program by CID, a section of it, a part of it; with `track`, the program is the stored set's track of that
    /// number (`<cid>.track2.drop.bass`).
    Stored { cid: String, track: Option<u32>, section: Option<String>, part: Option<String> },
    /// A theme of the public-domain library, by id.
    Theme(String),
    /// Notes, a bar each, canonical.
    Notes(Vec<String>),
    /// Another template's output, or one section of it.
    Form(FormRef),
    /// A whole number: a form's parameter (the canon's `delay=6`, `interval=-12`), never a role's source.
    Number(i64),
}

/// `template(NAME, ROLE=SOURCE ...)` where a source goes: a form with no phrase of its own; `.both` after it is one of its
/// sections (and `.both.arp` that section's part).
#[derive(Clone, Debug, PartialEq)]
pub struct FormRef {
    pub name: String,
    pub name_span: Span,
    pub roles: Vec<Binding>,
    pub span: Span,
    pub section: Option<String>,
    pub part: Option<String>,
}

impl Source {
    /// The canonical spelling.
    pub fn text(&self) -> String {
        match self {
            Source::Gen(c) => c.text(),
            Source::Stored { cid, track, section, part } => {
                let mut s = cid.clone();
                if let Some(k) = track {
                    s.push_str(&format!(".track{k}"));
                }
                for n in [section, part].into_iter().flatten() {
                    s.push('.');
                    s.push_str(n);
                }
                s
            }
            Source::Theme(id) => id.clone(),
            Source::Notes(bars) if bars.len() == 1 => format!("[{}]", bars[0]),
            Source::Notes(bars) => format!("<{}>", bars.iter().map(|b| format!("[{b}]")).collect::<Vec<_>>().join(" ")),
            Source::Form(f) => f.text(),
            Source::Number(n) => n.to_string(),
        }
    }
}

impl FormRef {
    /// The canonical spelling: `template(passacaglia, ground=gen@1 dubstep seed=54876 section=drop part=bass)`, and a section
    /// after it: `template(cross).both.arp`.
    pub fn text(&self) -> String {
        let mut s = format!("template({}{})", self.name, bindings_text(&self.name, &self.roles));
        for n in [&self.section, &self.part].into_iter().flatten() {
            s.push('.');
            s.push_str(n);
        }
        s
    }
}

/// Where a binding goes in the canonical spelling: the form's roles in its order, each seed role's parts (`a.bass`) right after
/// it in the order of its parts, then the form's numbers in its order; a name it does not know after them all.
fn rank(form: &str, name: &str) -> (usize, usize) {
    let roles = specs(form);
    if let Some((r, sub)) = name.split_once('.') {
        if let Some(i) = roles.iter().position(|s| s.name == r) {
            return (i, roles[i].parts.iter().position(|p| p.0 == sub).map_or(usize::MAX, |j| j + 1));
        }
    }
    if let Some(i) = roles.iter().position(|s| s.name == name) {
        return (i, 0);
    }
    match params(form).iter().position(|p| p.name == name) {
        Some(i) => (roles.len() + i, 0),
        None => (usize::MAX, 0),
    }
}

/// `, role=source` for each binding, in the order the form names its roles (one it does not name after them, as written): a
/// seed role's parts right after it, the form's numbers after its roles.
pub fn bindings_text(form: &str, roles: &[Binding]) -> String {
    let mut v: Vec<&Binding> = roles.iter().collect();
    v.sort_by_key(|b| rank(form, &b.role));
    v.iter().map(|b| format!(", {}={}", b.role, b.source.text())).collect()
}

/// The stored programs (and sets) every template reference in `toks` binds a role to, its inner forms' too, in order, once
/// each: what a store credits (stored::cids_of). A set's tracks are not here: only a check that has the set knows them.
pub(crate) fn stored_cids(toks: &[Token]) -> Vec<String> {
    fn walk(roles: &[Binding], out: &mut Vec<String>) {
        for b in roles {
            match &b.source {
                Source::Stored { cid, .. } if !out.contains(cid) => out.push(cid.clone()),
                Source::Form(f) => walk(&f.roles, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for r in crate::template::runs(toks) {
        if let Ok(t) = r.r {
            walk(&t.roles, &mut out);
        }
    }
    out
}

/// `trackN` after a CID: the stored set's track N (1 on).
fn track_number(n: &str) -> Option<u32> {
    let d = n.strip_prefix("track")?;
    if d.is_empty() || d.starts_with('0') || d.len() > 4 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    d.parse().ok()
}

/// Reads bindings from a token stream: the lexer's (template.rs `expand`), or the parser's, where a `gen` call is already one word.
pub(crate) struct Reader<'t> {
    toks: &'t [Token],
    gens: Vec<crate::gen::Run>,
}

fn is_name(w: &str) -> bool {
    w.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A `gen` call the parser already made one word of, read back; its places are the word's.
fn word_call(w: &str, at: Span) -> Result<GenCall, Diagnostic> {
    if w == "gen@0 error" {
        // a call gen.rs could not read, and has said why: no second error for it
        return Ok(GenCall { version: 0, version_span: at, genre: "error".into(), genre_span: at, seed: "0".into(), bpm: None, key: None, length: None, section: None, section_span: at, part: None, part_span: at, span: at });
    }
    let toks = lex(w).tokens;
    let run = crate::gen::runs(&toks).into_iter().next().ok_or_else(|| Diagnostic::error(at, format!("`{w}` is no generator call")))?;
    let mut c = run.r.map_err(|mut d| {
        d.span = at;
        d
    })?;
    c.version_span = at;
    c.genre_span = at;
    c.section_span = at;
    c.part_span = at;
    c.span = at;
    Ok(c)
}

/// The canonical text of one bar of notes (what is inside its `[ ]`), or why it is not one.
fn canon_bar(inner: &str, at: Span) -> Result<String, Diagnostic> {
    use crate::ast::{Atom, Body, StmtKind};
    let p = crate::parse::parse(&format!("deck B: melody [{inner}]\n"));
    if let Some(d) = p.diagnostics.iter().find(|d| d.is_error()) {
        // (the pattern's own words: the same mistake says the same thing in a part and in a role)
        return Err(Diagnostic::error(at, d.message.clone()).help(format!("{}; a role's notes are a bar in brackets: [d2 ~ a1 ~ b1 ~ f#1 ~]", d.help.clone().unwrap_or_default())));
    }
    let seq = p.program.stmts.into_iter().find_map(|s| match s.kind {
        StmtKind::Line(l) => match l.body {
            Body::Parts(ps) => ps.into_iter().next().map(|p| p.pattern),
            _ => None,
        },
        _ => None,
    });
    let Some(seq) = seq else { return Err(Diagnostic::error(at, "these notes do not read as a bar").help("[d2 ~ a1 ~ b1 ~ f#1 ~]")) };
    // `[a b c]` is the bar `a b c`: its steps are the let's steps, which the mutations move
    match seq.terms.as_slice() {
        [t] if t.fast.is_none() && !t.accent && !t.slide => match &t.atom {
            Atom::Group(seqs) if seqs.len() == 1 => Ok(crate::print::seq(&seqs[0])),
            _ => Ok(crate::print::seq(&seq)),
        },
        _ => Ok(crate::print::seq(&seq)),
    }
}

impl<'t> Reader<'t> {
    pub(crate) fn new(toks: &'t [Token]) -> Reader<'t> {
        Reader { toks, gens: crate::gen::runs(toks) }
    }

    fn at(&self, j: usize) -> Option<&Tok> {
        self.toks.get(j).map(|t| &t.tok)
    }

    fn span(&self, j: usize) -> Span {
        self.toks.get(j).or(self.toks.last()).map_or(Span::new(0, 0), |t| t.span)
    }

    /// The bindings `, ROLE=SOURCE` from `toks[j]` (a `,` or the `)`), up to the token that is neither: where they end.
    pub(crate) fn bindings(&self, mut j: usize, depth: usize) -> Result<(usize, Vec<Binding>), Diagnostic> {
        let mut out = Vec::new();
        while self.at(j) == Some(&Tok::Comma) {
            j += 1;
            // a role, or one part of a seed role: `a.bass` (the lexer makes a `.` only between touching words)
            let sub = match (self.at(j), self.at(j + 1), self.at(j + 2)) {
                (Some(Tok::Word(_)), Some(Tok::Dot), Some(Tok::Word(s))) => Some(s.clone()),
                _ => None,
            };
            let eq = if sub.is_some() { j + 3 } else { j + 1 };
            let (Some(Tok::Word(role)), Some(Tok::Eq)) = (self.at(j).cloned(), self.at(eq)) else {
                let what = match self.at(j) {
                    Some(Tok::Word(w)) if w.starts_with('@') => "a template has one phrase, and it comes first".to_string(),
                    Some(Tok::Str(_)) => "a template has one phrase, and it comes first".to_string(),
                    _ => "after the phrase, a template takes its roles: ROLE=SOURCE".to_string(),
                };
                return Err(Diagnostic::error(self.span(j), what).help(BINDING));
            };
            if !is_name(&role) {
                return Err(Diagnostic::error(self.span(j), format!("`{role}` is no role: a role is a name, ground or subject")).help(BINDING));
            }
            let role_span = self.span(j).to(self.span(eq - 1));
            let role = match sub {
                Some(s) if is_name(&s) => format!("{role}.{s}"),
                Some(s) => return Err(Diagnostic::error(self.span(j + 2), format!("`{s}` is no part: a part of a role is one of its parts, a.bass")).help(BINDING)),
                None => role,
            };
            j = eq + 1;
            let start = j;
            // a form's number: `delay=6`, `interval=-12`
            if let Some((end, n)) = self.number(j)? {
                out.push(Binding { role, role_span, source: Source::Number(n), source_span: self.span(start).to(self.span(end - 1)) });
                j = end;
                continue;
            }
            let (end, source) = self.source(j, depth)?;
            let source_span = self.span(start).to(self.span(end.max(start + 1) - 1));
            out.push(Binding { role, role_span, source, source_span });
            j = end;
        }
        Ok((j, out))
    }

    /// A whole number at `toks[j]` (`6`, `-12`): where it ends and its value; None when no number stands there.
    fn number(&self, j: usize) -> Result<Option<(usize, i64)>, Diagnostic> {
        let neg = self.at(j) == Some(&Tok::Minus);
        let k = if neg { j + 1 } else { j };
        match self.at(k) {
            Some(Tok::Num { value, int: true }) if value.abs() < 1e9 => Ok(Some((k + 1, if neg { -(*value as i64) } else { *value as i64 }))),
            Some(Tok::Num { .. }) => Err(Diagnostic::error(self.span(j).to(self.span(k)), "a form's number is a whole number").help("delay=6, interval=-12")),
            _ if neg => Err(Diagnostic::error(self.span(j), "a `-` here starts a number").help("interval=-12")),
            _ => Ok(None),
        }
    }

    /// Where the bracket at `toks[open]` closes (the index after it), counting `[ ]`, `< >` and `( )` together.
    fn close(&self, open: usize) -> Result<usize, Diagnostic> {
        let mut depth = 0usize;
        let mut j = open;
        while let Some(t) = self.at(j) {
            match t {
                Tok::LBrack | Tok::Lt | Tok::LParen => depth += 1,
                Tok::RBrack | Tok::Gt | Tok::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(j + 1);
                    }
                }
                Tok::Newline | Tok::Eof => break,
                _ => {}
            }
            j += 1;
        }
        Err(Diagnostic::error(self.span(open), "these notes are not closed on their line").help("[d2 ~ a1 ~ b1 ~ f#1 ~]"))
    }

    /// One bar of notes, `[ ... ]` at `toks[j]`: its canonical text and where it ends.
    fn bar(&self, j: usize) -> Result<(usize, String), Diagnostic> {
        let end = self.close(j)?;
        if self.at(end - 1) != Some(&Tok::RBrack) {
            return Err(Diagnostic::error(self.span(j), "a bar of notes closes with `]`").help("[d2 ~ a1 ~ b1 ~ f#1 ~]"));
        }
        // the tokens as written: glued where they touch, a space where they do not
        let mut inner = String::new();
        for (k, t) in self.toks[j + 1..end - 1].iter().enumerate() {
            if k > 0 && self.toks[j + k].span.end != t.span.start {
                inner.push(' ');
            }
            inner.push_str(&crate::print::tokens(std::iter::once(&t.tok)));
        }
        if inner.trim().is_empty() {
            return Err(Diagnostic::error(self.span(j).to(self.span(end - 1)), "a bar of notes has a step in it at least").help("[d2 ~ a1 ~]"));
        }
        Ok((end, canon_bar(&inner, self.span(j).to(self.span(end - 1)))?))
    }

    /// A source at `toks[j]`, and where it ends.
    fn source(&self, j: usize, depth: usize) -> Result<(usize, Source), Diagnostic> {
        let here = self.span(j);
        match self.at(j) {
            Some(Tok::Word(w)) if w == "template" && self.at(j + 1) == Some(&Tok::LParen) => {
                let (mut end, mut f) = self.form(j, depth + 1)?;
                // one of its sections, and that section's part: `template(cross).both`, `template(cross).both.arp`
                let mut names: Vec<(String, Span)> = Vec::new();
                while self.at(end) == Some(&Tok::Dot) && names.len() < 3 {
                    let Some(Tok::Word(n)) = self.at(end + 1) else { break };
                    names.push((n.clone(), self.span(end + 1)));
                    end += 2;
                }
                let is_role = |n: &str| ROLES.contains(&n);
                match names.as_slice() {
                    [] => {}
                    [(s, at)] if is_role(s) => return Err(Diagnostic::error(*at, format!("`.{s}`: a form's section comes first, then its part")).help(format!("{}.SECTION.{s}", f.text()))),
                    [(s, _)] => f.section = Some(s.clone()),
                    [(s, sa), (p, pa)] => {
                        if is_role(s) {
                            return Err(Diagnostic::error(*sa, format!("`.{s}.{p}`: a section comes before a part")).help(format!("{}.{p}.{s}", f.text())));
                        }
                        if !is_role(p) {
                            return Err(Diagnostic::error(*pa, format!("`.{p}`: a part of a form's section is one of the roles")).expected(&ROLES).found(format!("`{p}`")));
                        }
                        f.section = Some(s.clone());
                        f.part = Some(p.clone());
                    }
                    _ => return Err(Diagnostic::error(self.span(j).to(self.span(end - 1)), "a form's section is a section and a part at most").help("template(cross).both.arp")),
                }
                if !names.is_empty() {
                    f.span = self.span(j).to(self.span(end - 1));
                }
                Ok((end, Source::Form(f)))
            }
            Some(Tok::Word(w)) if crate::gen::is_call_word(w) => Ok((j + 1, Source::Gen(word_call(w, here)?))),
            Some(Tok::Word(_)) if self.gens.iter().any(|r| r.start == j) => {
                let r = self.gens.iter().find(|r| r.start == j).expect("found");
                Ok((r.end, Source::Gen(r.r.clone()?)))
            }
            Some(Tok::Word(w)) if crate::stored::is_cid(w) => {
                let mut names: Vec<(String, Span)> = Vec::new();
                let mut k = j + 1;
                while self.at(k) == Some(&Tok::Dot) && self.span(k).start == self.span(k - 1).end {
                    let Some(Tok::Word(n)) = self.at(k + 1) else { break };
                    names.push((n.clone(), self.span(k + 1)));
                    k += 2;
                }
                let is_role = |n: &str| ROLES.contains(&n);
                // a stored set's track first: `<cid>.track2`, then its section and part
                let track = names.first().and_then(|(n, _)| track_number(n));
                if track.is_some() {
                    names.remove(0);
                }
                let (section, part) = match names.as_slice() {
                    [] => (None, None),
                    [(n, _)] if is_role(n) => (None, Some(n.clone())),
                    [(n, _)] => (Some(n.clone()), None),
                    [(a, sa), (b, sb)] => {
                        if is_role(a) {
                            return Err(Diagnostic::error(*sa, format!("`.{a}.{b}`: a section comes before a part")).help(format!("{w}.{b}.{a}")));
                        }
                        if !is_role(b) {
                            return Err(Diagnostic::error(*sb, format!("`.{b}`: a part of a stored program is one of its roles")).expected(&ROLES).found(format!("`{b}`")));
                        }
                        (Some(a.clone()), Some(b.clone()))
                    }
                    _ => return Err(Diagnostic::error(here.to(self.span(k - 1)), "a stored program is a CID, a section and a part at most (a stored set's: a track, then those)").help("<cid>.drop.bass, <cid>.track2.drop.bass")),
                };
                Ok((k, Source::Stored { cid: w.clone(), track, section, part }))
            }
            Some(Tok::Word(w)) if is_name(w) => Ok((j + 1, Source::Theme(w.clone()))),
            Some(Tok::LBrack) => {
                let (end, bar) = self.bar(j)?;
                Ok((end, Source::Notes(vec![bar])))
            }
            Some(Tok::Lt) => {
                let mut bars = Vec::new();
                let mut k = j + 1;
                loop {
                    match self.at(k) {
                        Some(Tok::LBrack) => {
                            let (end, bar) = self.bar(k)?;
                            bars.push(bar);
                            k = end;
                        }
                        Some(Tok::Gt) if !bars.is_empty() => return Ok((k + 1, Source::Notes(bars))),
                        _ => return Err(Diagnostic::error(self.span(k), "bars of notes go in brackets, one after another, in `< >`").help("<[d2 ~ a1 ~] [g1 ~ a1 ~]>")),
                    }
                }
            }
            _ => Err(Diagnostic::error(here, "a role is bound to a source").help(BINDING)),
        }
    }

    /// `template(NAME, ROLE=SOURCE ...)` at `toks[i]` as a source: where it ends, and the form.
    fn form(&self, i: usize, depth: usize) -> Result<(usize, FormRef), Diagnostic> {
        if depth > MAX_READ_DEPTH {
            return Err(Diagnostic::error(self.span(i), format!("templates nest more than {MAX_READ_DEPTH} deep here")).help(format!("a form takes forms {MAX_NESTING} deep")));
        }
        let mut j = i + 2;
        let Some(Tok::Word(name)) = self.at(j).cloned().filter(|t| matches!(t, Tok::Word(w) if !w.starts_with('@'))) else {
            return Err(Diagnostic::error(self.span(i).to(self.span(j)), "`template(` takes a template's name first: template(passacaglia, ground=...)").help(BINDING));
        };
        let name_span = self.span(j);
        j += 1;
        let phrase = matches!(self.at(j + 1), Some(Tok::Str(_))) || matches!(self.at(j + 1), Some(Tok::Word(w)) if w.starts_with('@'));
        if self.at(j) == Some(&Tok::Comma) && phrase {
            return Err(Diagnostic::error(self.span(j + 1), format!("a template inside another takes no phrase: the phrase is the outer form's, sung in its own section")).help(format!("template({name}, ROLE=SOURCE)")));
        }
        let (k, roles) = self.bindings(j, depth)?;
        if self.at(k) != Some(&Tok::RParen) {
            return Err(Diagnostic::error(self.span(k), format!("expected `)` to close template({name}, ...)")).help(BINDING));
        }
        let span = self.span(i).to(self.span(k));
        Ok((k + 1, FormRef { name, name_span, roles, span, section: None, part: None }))
    }
}

// ---------------------------------------------------------------- the roles of each form

/// A role of a form: what it is, what fills it when nobody binds it, and how the form's text holds it.
pub struct RoleSpec {
    pub name: &'static str,
    /// What the role is, for the docs, the notes and the written-out comment.
    pub what: &'static str,
    /// What plays it when it is not bound.
    pub default: &'static str,
    /// The let of the default in the template's text (its bars `ID_1` ...).
    pub id: &'static str,
    /// The let a bound line is written as (its bars `NAME_1` ...): what the sections and the plan read.
    pub let_name: &'static str,
    /// The part a whole seed gives it.
    pub part: &'static str,
    /// The default's cycle on the parts that play it (`/20`); a bound line plays with its own.
    pub cycle: &'static str,
    /// Only parts play the role (no let is made of its bars), so a bar of any cycle fills it.
    pub flex: bool,
    /// The comment directly above the default's lets that says what it is (gone once the role is bound).
    pub above: &'static str,
    /// The section a source of sections stands in for (it and its places in the arrangement): rondo's `episode`.
    pub section: Option<&'static str>,
    /// What an inner form's names start with in this role.
    pub prefix: &'static str,
    /// A seed role (cross): the lines it is, (part, the default's let, the let it is written as).
    pub parts: &'static [(&'static str, &'static str, &'static str)],
    /// Lets of the form's text that only the default is made of, gone once the role is bound (the quodlibet's `third` is the
    /// fate motif moved by hand: `beethoven_fate`).
    pub drops: &'static [&'static str],
}

const fn line(name: &'static str, what: &'static str, default: &'static str, id: &'static str, let_name: &'static str, part: &'static str, cycle: &'static str, flex: bool, above: &'static str) -> RoleSpec {
    RoleSpec { name, what, default, id, let_name, part, cycle, flex, above, section: None, prefix: "", parts: &[], drops: &[] }
}

// (the quodlibet's lines are written `tune_a` and `tune_b`: a let named `a` or `b` would read as a note)
const QUODLIBET: [RoleSpec; 4] = [
    line("a", "the first tune: it leads the canon section, and the second is drawn toward it", "Pachelbel's first violin, `pachelbel_violin`", "pachelbel_violin", "tune_a", "melody", "", false, ""),
    line("b", "the second tune: it leads the ode section and is drawn toward the first, then toward the third", "Beethoven's Ode to Joy, `ode_to_joy`", "ode_to_joy", "tune_b", "melody", "", false, ""),
    line("ground", "the bass under every section", "Pachelbel's ground, `pachelbel_ground`", "pachelbel_ground", "ground", "bass", "", true, ""),
    RoleSpec {
        name: "third",
        what: "the third line: it leads the fate section, and the other two are pulled toward it",
        default: "Beethoven's fate motif a semitone down, in B minor (D major's relative minor), `fate_b`",
        id: "fate_b",
        let_name: "third",
        part: "melody",
        cycle: "",
        flex: false,
        above: "# the fate motif a semitone down, in B minor (D major's relative minor): the third line the others are pulled toward",
        section: None,
        prefix: "",
        parts: &[],
        drops: &["beethoven_fate"],
    },
];

const PASSACAGLIA: [RoleSpec; 2] = [
    line(
        "ground",
        "the bass line under every section, never changed",
        "Holst's five-beat ostinato from Mars, `mars_ostinato`",
        "mars_ostinato",
        "ostinato",
        "bass",
        "/20",
        true,
        "# the ground: five beats against the four of the bar, so it comes round every five bars",
    ),
    line("upper", "the line above the ground, changed a little more every pass", "the form's own line in G minor, `upper`", "upper", "upper", "melody", "", true, ""),
];

const CANON: [RoleSpec; 2] = [
    line("subject", "the line chased by copies of itself, a bar and a beat behind", "Pachelbel's first violin, `pachelbel_violin`", "pachelbel_violin", "subject", "melody", "", false, ""),
    line("ground", "the bass under the chase", "Pachelbel's ground, `pachelbel_ground`", "pachelbel_ground", "ground", "bass", "", true, ""),
];

const RONDO: [RoleSpec; 2] = [
    line("refrain", "the tune that comes back, changed on each return", "Joplin's Entertainer, `the_entertainer`", "the_entertainer", "tune", "melody", "", true, ""),
    RoleSpec {
        name: "episodes",
        what: "what plays between the refrains: a line, the episodes' figure, or sections: a seed's, a stored program's, another form's",
        default: "Bach's Prelude in C, `bach_prelude_c`, the episode's figure",
        id: "bach_prelude_c",
        let_name: "figure",
        part: "arp",
        cycle: "",
        flex: true,
        above: "",
        section: Some("episode"),
        prefix: "ep_",
        parts: &[],
        drops: &[],
    },
];

const FUGUE: [RoleSpec; 2] = [
    // (`subject` names a section of the fugue, so the line is the dux, as a fugue's subject was called)
    line("subject", "the theme that enters, is answered a fifth up and re-enters transposed", "Grieg's Mountain King, `mountain_king`", "mountain_king", "dux", "melody", "", false, ""),
    line("counter", "the countersubject, drawn toward the subject", "Bach's Badinerie, `badinerie`", "badinerie", "countersubject", "melody", "", true, ""),
];

const THEME_AND_VARIATIONS: [RoleSpec; 1] = [line("theme", "the one source every variation changes", "Dvorak's New World Largo, `new_world_largo`", "new_world_largo", "air", "melody", "", true, "")];

const CROSS: [RoleSpec; 2] = [
    RoleSpec {
        name: "a",
        what: "the first source, its melody, bass and chords: a whole seed, or a line for its melody",
        default: "Beethoven's Ode to Joy over Pachelbel's ground, in D",
        id: "",
        let_name: "",
        part: "melody",
        cycle: "",
        flex: false,
        above: "",
        section: None,
        prefix: "",
        parts: &[("melody", "ode_to_joy", "a_melody"), ("bass", "pachelbel_ground", "a_bass"), ("harmony", "a_harmony", "a_harmony")],
        drops: &[],
    },
    RoleSpec {
        name: "b",
        what: "the second source, its melody, bass and chords: a whole seed, or a line for its melody",
        default: "Bach's Badinerie over a B minor bass, D major's relative minor",
        id: "",
        let_name: "",
        part: "melody",
        cycle: "",
        flex: false,
        above: "",
        section: None,
        prefix: "",
        parts: &[("melody", "badinerie", "b_melody"), ("bass", "b_bass", "b_bass"), ("harmony", "b_harmony", "b_harmony")],
        drops: &[],
    },
];

/// The roles of the form `name`, in the order the canonical spelling binds them (none: a template that takes no roles yet).
pub fn specs(name: &str) -> &'static [RoleSpec] {
    match name {
        "quodlibet" => &QUODLIBET,
        "passacaglia" => &PASSACAGLIA,
        "canon" => &CANON,
        "rondo" => &RONDO,
        "fugue" => &FUGUE,
        "theme-and-variations" => &THEME_AND_VARIATIONS,
        "cross" => &CROSS,
        _ => &[],
    }
}

/// The templates that take roles, in the docs' order.
pub fn forms_with_roles() -> Vec<&'static str> {
    crate::template::TEMPLATES.iter().map(|t| t.name).filter(|n| !specs(n).is_empty()).collect()
}

/// A number a form takes after its roles (`delay=6`): what it is, what it is when not given, and the whole numbers it may be.
pub struct ParamSpec {
    pub name: &'static str,
    /// What it is, for the docs, the notes and the header.
    pub what: &'static str,
    /// What it is when the reference does not say it (the form as it always was).
    pub default: &'static str,
    /// The least and the most it may be (the canon's `delay` is also less than its subject's length).
    pub min: i64,
    pub max: i64,
    /// A binding that shows it: `delay=6`.
    pub example: &'static str,
}

const CANON_PARAMS: [ParamSpec; 2] = [
    ParamSpec {
        name: "delay",
        what: "how far behind the leader the chase's copy is, in beats (4 is a bar): its bars come round that many beats late",
        default: "a bar behind (4 beats); half a bar (2) for a subject of one bar",
        min: 1,
        max: 4 * MAX_LINE_BARS as i64 - 1,
        example: "delay=6",
    },
    ParamSpec {
        name: "interval",
        what: "the interval of the chase's copy from the leader, in semitones (-12 an octave down, 7 a fifth up, 0 in unison)",
        default: "an octave down (-12)",
        min: -24,
        max: 24,
        example: "interval=7",
    },
];

/// The numbers the form `name` takes after its roles, in the order the canonical spelling gives them (none: it takes none).
pub fn params(name: &str) -> &'static [ParamSpec] {
    match name {
        "canon" => &CANON_PARAMS,
        _ => &[],
    }
}

// ---------------------------------------------------------------- the text, line by line

/// A written-out form being bound: its lines, and which of them a binding wrote (never renamed again).
struct Doc {
    lines: Vec<String>,
    fixed: Vec<bool>,
}

fn is_comment(l: &str) -> bool {
    l.trim_start().starts_with('#')
}

/// `line` with every name in it passed through `f`: a run of letters, digits and `_` that starts with a letter (and a `*` glued
/// after it: a glob), not right after `#`, `@`, `$`, `.` or `=` (a sharp, a phrase, a parameter, a stored program's section, a
/// param's value). A string's text is left as it is.
fn map_words(line: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let is_w = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = String::with_capacity(line.len() + 16);
    let mut k = 0;
    let mut in_str = false;
    while k < chars.len() {
        let (i, c) = chars[k];
        if in_str {
            out.push(c);
            if c == '\\' && k + 1 < chars.len() {
                out.push(chars[k + 1].1);
                k += 1;
            } else if c == '"' {
                in_str = false;
            }
            k += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            k += 1;
            continue;
        }
        let prev = if k == 0 { None } else { Some(chars[k - 1].1) };
        if is_w(c) && !prev.is_some_and(is_w) {
            let mut e = k;
            while e < chars.len() && is_w(chars[e].1) {
                e += 1;
            }
            if e < chars.len() && chars[e].1 == '*' {
                e += 1;
            }
            let end = chars.get(e).map_or(line.len(), |x| x.0);
            let word = &line[i..end];
            let candidate = c.is_ascii_alphabetic() && !prev.is_some_and(|p| matches!(p, '#' | '@' | '$' | '.' | '='));
            match candidate.then(|| f(word)).flatten() {
                Some(r) => out.push_str(&r),
                None => out.push_str(word),
            }
            k = e;
            continue;
        }
        out.push(c);
        k += 1;
    }
    out
}

impl Doc {
    fn new(text: &str) -> Doc {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let fixed = vec![false; lines.len()];
        Doc { lines, fixed }
    }

    fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        while s.ends_with('\n') {
            s.pop();
        }
        s
    }

    fn splice(&mut self, at: std::ops::Range<usize>, with: Vec<String>) {
        let n = with.len();
        self.lines.splice(at.clone(), with);
        self.fixed.splice(at, std::iter::repeat_n(true, n));
    }

    /// The lines of the let `id` and its bars (`let ID_1 = ...` ... `let ID = ...`), together.
    fn block(&self, id: &str) -> Option<std::ops::Range<usize>> {
        let is_bar = |l: &str| l.strip_prefix("let ").and_then(|r| r.strip_prefix(id)).and_then(|r| r.strip_prefix('_')).is_some_and(|r| r.split(' ').next().is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) && r.contains(" = "));
        let is_whole = |l: &str| l.strip_prefix("let ").and_then(|r| r.strip_prefix(id)).is_some_and(|r| r.starts_with(" = "));
        let s = self.lines.iter().position(|l| is_bar(l) || is_whole(l))?;
        let mut e = s;
        while e < self.lines.len() && (is_bar(&self.lines[e]) || is_whole(&self.lines[e])) {
            e += 1;
        }
        Some(s..e)
    }

    /// Every line no binding wrote and no comment: `f` over its names.
    fn rename(&mut self, f: &mut dyn FnMut(&str) -> Option<String>) {
        for (l, fixed) in self.lines.iter_mut().zip(&self.fixed) {
            if !*fixed && !is_comment(l) {
                *l = map_words(l, f);
            }
        }
    }

    /// The section `name`: from its doc comment (the `#` lines directly above it) to its closing `}` (or its one line).
    fn section(&self, name: &str) -> Option<(usize, usize, usize)> {
        let head = format!("section {name} ");
        let s = self.lines.iter().position(|l| l.starts_with(&head))?;
        let mut d = s;
        while d > 0 && is_comment(&self.lines[d - 1]) {
            d -= 1;
        }
        let e = if self.lines[s].trim_end().ends_with('{') { (s..self.lines.len()).find(|i| self.lines[*i] == "}")? } else { s };
        Some((d, s, e))
    }

    /// The arrange block: the line of `arrange {` and of its `}`.
    fn arrange(&self) -> Option<(usize, usize)> {
        let s = self.lines.iter().position(|l| l.starts_with("arrange {"))?;
        let e = (s..self.lines.len()).find(|i| self.lines[*i].starts_with('}'))?;
        Some((s, e))
    }
}

// ---------------------------------------------------------------- keys and themes

/// Is the key a minor one (minor, dorian, phrygian, harmonic minor, the minor pentatonic, the blues)?
fn minor(k: Key) -> bool {
    matches!(k.mode, 1 | 2 | 4 | 5 | 7 | 8)
}

/// The semitones that move a theme into `to`: to its tonic, or to its relative key when the theme's mode is the other one; the
/// nearest way, -6 to 5.
fn theme_shift(t: &djengine::themes::Theme, to: Key) -> i32 {
    let from = Key::parse(&t.key).map_or(0, |k| i32::from(k.tonic));
    let target = i32::from(to.tonic)
        + match (t.mode.as_str(), minor(to)) {
            ("minor", false) => 9,
            ("major", true) => 3,
            _ => 0,
        };
    let d = (target - from).rem_euclid(12);
    if d > 5 {
        d - 12
    } else {
        d
    }
}

/// The semitones that move notes in key `from` into key `to`, by the rule a theme is moved by ([`theme_shift`]).
fn key_shift(from: Key, to: Key) -> i32 {
    let target = i32::from(to.tonic)
        + match (minor(from), minor(to)) {
            (true, false) => 9,
            (false, true) => 3,
            _ => 0,
        };
    let d = (target - i32::from(from.tonic)).rem_euclid(12);
    if d > 5 {
        d - 12
    } else {
        d
    }
}

/// A bar's notes moved `n` semitones (sharps, as the canonical text spells them); steps that are no note with an octave stay.
fn transpose_bar(bar: &str, n: i32) -> String {
    if n == 0 {
        return bar.to_string();
    }
    const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    let mut out = String::with_capacity(bar.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match crate::music::parse_note(word) {
            Some((pc, Some(oct))) => {
                let m = crate::music::midi(pc, oct) + n;
                out.push_str(&format!("{}{}", SHARP[m.rem_euclid(12) as usize], m.div_euclid(12) - 1));
            }
            _ => out.push_str(word),
        }
        word.clear();
    };
    for c in bar.chars() {
        if c.is_ascii_alphanumeric() || c == '#' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

// ---------------------------------------------------------------- binding

/// What a binding is checked and written out against.
pub(crate) struct Ctx<'a, 'e> {
    pub env: &'a Env<'e>,
    /// The key and tempo the written-out program says at its top level (what a `gen` call there is fitted to).
    pub host: Host,
    /// The phrase's slots: an inner form is filled with them, and then its vocal is taken out.
    pub slots: &'a [(String, String)],
    pub errors: Vec<Diagnostic>,
    /// What each binding wrote, said at it.
    pub notes: Vec<Diagnostic>,
    /// The programs a binding drew on that the written-out text does not name (a stored set: its tracks are named), (cid, text),
    /// for a link to carry (compile.rs `plays`).
    pub drawn: Vec<(String, String)>,
    /// The inner forms written out and compiled for one of their sections ([`Ctx::form_program`]), by their canonical text.
    forms: Vec<(String, Rc<crate::compile::Compiled>, String)>,
}

impl<'a, 'e> Ctx<'a, 'e> {
    pub(crate) fn new(env: &'a Env<'e>, host: Host, slots: &'a [(String, String)]) -> Ctx<'a, 'e> {
        Ctx { env, host, slots, errors: Vec::new(), notes: Vec::new(), drawn: Vec::new(), forms: Vec::new() }
    }
}

/// Does section `s` of a compiled program play `part` once, on the bar's grid?
fn plays_part(c: &crate::compile::Compiled, s: &str, part: &str) -> bool {
    c.segments.iter().find(|g| g.header.is_some() && !g.chain && g.name == s).is_some_and(|g| {
        let hits: Vec<&crate::compile::PartIr> = g.decks.values().flat_map(|d| d.parts.iter()).filter(|p| p.role == part).collect();
        matches!(hits.as_slice(), [p] if p.cycle_steps == 16)
    })
}

/// A line a binding writes: the let lines, how many bars, its cycle, and what to say of it.
struct LineOut {
    lets: Vec<String>,
    bars: usize,
    cycle: String,
    note: String,
    /// The line is one let however long it is (a stored part, written out by the stored stage): its bars cannot be named.
    one_let: bool,
}

/// A run of sections a binding writes: the text (lets and sections), the plays in order (a section with the mutation lines
/// that follow it), and the bars of one pass.
struct Run {
    text: Vec<String>,
    plays: Vec<Vec<String>>,
    bars: u32,
}

/// A program the generator printed, compiled: what the form picks its sections and parts from.
struct Generated {
    call: GenCall,
    comp: crate::compile::Compiled,
}

impl Generated {
    /// The sections it plays, in order, each with its length (all of its stretches).
    fn plays(&self) -> Vec<(String, u32)> {
        sequence(&self.comp)
    }

    /// Does section `s` play `part` once, on the bar's grid?
    fn has(&self, s: &str, part: &str) -> bool {
        plays_part(&self.comp, s, part)
    }

    /// The section a part is taken from: the drop, the chorus, then the first that plays it.
    fn section_for(&self, parts: &[&str]) -> Option<String> {
        let plays = self.plays();
        let mut order: Vec<String> = ["drop", "chorus"].iter().map(|s| s.to_string()).filter(|s| plays.iter().any(|(n, _)| n == s)).collect();
        for (n, _) in plays {
            if !order.contains(&n) {
                order.push(n);
            }
        }
        order.into_iter().find(|s| parts.iter().all(|p| self.has(s, p)))
    }
}

/// The sections a compiled program plays, in its order, each with its length (an evolving section is all its stretches).
fn sequence(c: &crate::compile::Compiled) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (k, s) in c.segments.iter().enumerate() {
        if s.header.is_some() && !s.chain && !s.name.contains('+') {
            let bars = s.bars + c.segments[k + 1..].iter().take_while(|x| x.chain).map(|x| x.bars).sum::<u32>();
            out.push((s.name.clone(), bars));
        }
    }
    out
}

impl Ctx<'_, '_> {
    fn err(&mut self, d: Diagnostic) {
        self.errors.push(d);
    }

    /// The program a call prints, in `key` (None: the program's top-level key), at the program's tempo.
    fn generated(&mut self, c: &GenCall, at: Span) -> Option<Generated> {
        let host = self.host.clone();
        let whole = GenCall { section: None, part: None, ..c.clone() };
        let r = match crate::gen::resolve(&whole, &host) {
            Ok(r) => r,
            Err(mut d) => {
                if d.span == Span::default() {
                    d.span = at;
                }
                self.err(d);
                return None;
            }
        };
        match crate::compile::check_with(&r.text, &Env::none()).compiled {
            Some(comp) => Some(Generated { call: whole, comp }),
            None => {
                self.err(Diagnostic::error(at, format!("{} does not compile", r.generator)));
                None
            }
        }
    }

    /// The bars a `let NAME = CALL` is written out as (the gen stage's own writing of it), or why not.
    fn probe(&mut self, call: &GenCall, at: Span) -> Option<usize> {
        let mut src = String::new();
        if let Some(t) = self.host.tempo {
            src.push_str(&format!("tempo {}\n", crate::print::num(t)));
        }
        if let Some(k) = &self.host.key {
            src.push_str(&format!("key {k}\n"));
        }
        src.push_str(&format!("let probe = {}\n", call.text()));
        match crate::gen::expand(&src) {
            Some(Ok((e, _))) => Some(e.text.lines().filter(|l| l.starts_with("let probe_")).count()),
            Some(Err(ds)) => {
                for d in ds {
                    self.err(Diagnostic { span: at, ..d });
                }
                None
            }
            None => None,
        }
    }

    /// A stored program the host has, compiled as stored.rs compiles a source (re-hashed, in a plain environment).
    fn stored(&mut self, cid: &str, at: Span) -> Option<crate::compile::Compiled> {
        match self.try_stored(cid, at) {
            Ok(c) => Some(c),
            Err(d) => {
                self.err(d);
                None
            }
        }
    }

    /// [`Ctx::stored`], or why not, said to no one.
    fn try_stored(&self, cid: &str, at: Span) -> Result<crate::compile::Compiled, Diagnostic> {
        let missing = || Diagnostic::error(at, crate::env::not_here(cid)).help(crate::env::NOT_HERE_HELP);
        let text = match crate::gen::generated(cid) {
            Some(t) => t,
            None => match self.env.snippets.map(|s| s.resolve(cid)) {
                Some(Ok((got, text))) if got == cid => text,
                Some(Err(e)) if e.contains("not the program it names") => return Err(Diagnostic::error(at, e)),
                _ => return Err(missing()),
            },
        };
        if crate::compile::program_cid(&text) != cid {
            return Err(Diagnostic::error(at, format!("the program {} is not the program it names: refused", djengine::rights::short_cid(cid))));
        }
        let plain = Env { registry: None, catalog: None, snippets: self.env.snippets, assets: None, private: false, voices: None, ontology: None, bank: None };
        crate::compile::check_with(&text, &plain).compiled.ok_or_else(|| Diagnostic::error(at, format!("the program {} does not compile", djengine::rights::short_cid(cid))).help("it can be borrowed from once it checks"))
    }

    /// A program a binding drew on that the written-out text will not name (a stored set), for the link to carry.
    fn draw(&mut self, cid: &str, text: &str) {
        if !self.drawn.iter().any(|(c, _)| c == cid) {
            self.drawn.push((cid.to_string(), text.to_string()));
        }
    }

    /// What `<cid>.trackN` names, with the section after it (if any): track N of a stored set (its CID, and that section), or,
    /// after a program that is no set, its section called `trackN`, as such a reference always read (a section may be named
    /// `track2`). (the CID, the section, the description of where it is), or why not.
    fn track_ref(&mut self, cid: &str, k: u32, section: Option<&String>, at: Span) -> Option<(String, Option<String>, String)> {
        let comp = self.stored(cid, at)?;
        let short = djengine::rights::short_cid(cid);
        let Some(set) = &comp.set else {
            let name = format!("track{k}");
            let names: Vec<String> = sequence(&comp).into_iter().map(|(n, _)| n).collect();
            if section.is_none() && names.contains(&name) {
                return Some((cid.to_string(), Some(name), short));
            }
            self.err(Diagnostic::error(at, format!("{short} is not a set, and has no section `{name}`: `.{name}` names a track of a stored set")).help(format!("its sections: {}", names.join(", "))));
            return None;
        };
        let n = set.tracks.len();
        if k == 0 || k as usize > n {
            self.err(Diagnostic::error(at, format!("the set {short} has {n} track{}: `.track{k}` is none of them", if n == 1 { "" } else { "s" })).help(format!("track1 to track{n}")));
            return None;
        }
        self.draw(cid, &comp.text);
        Some((set.tracks[k as usize - 1].cid.clone(), section.cloned(), format!("track {k} of the set {short}")))
    }

    /// A name after a set's CID that is no track (`.track0`, `.verse`): the error that says a set has tracks, counted from 1.
    fn not_a_track(&mut self, b: &Binding, cid: &str, comp: &crate::compile::Compiled, name: &str) {
        let short = djengine::rights::short_cid(cid);
        let n = comp.set.as_ref().map_or(0, |s| s.tracks.len());
        let d = if name.starts_with("track") {
            Diagnostic::error(b.source_span, format!("the set {short} has {n} track{}, counted from 1: `.{name}` is none of them", if n == 1 { "" } else { "s" }))
        } else {
            Diagnostic::error(b.source_span, format!("{short} is a set: it has tracks, not sections, and `.{name}` is no track"))
        };
        self.err(d.help(format!("{}={cid}.track1 (track1 to track{n})", b.role)));
    }

    /// The tracks of a stored set, in order, a track that is itself a set its tracks in turn: (its path, the track numbers from the
    /// outer set in: `[2, 1]` is track 1 of track 2; cid; compiled).
    fn set_tracks(&mut self, cid: &str, comp: &crate::compile::Compiled, at: Span, path: &[usize], out: &mut Vec<(Vec<usize>, String, crate::compile::Compiled)>) -> Option<()> {
        if path.len() >= crate::stored::MAX_DEPTH {
            self.err(Diagnostic::error(at, format!("sets nest more than {} deep here", crate::stored::MAX_DEPTH)).help(format!("bind one of its tracks: {cid}.track1")));
            return None;
        }
        self.draw(cid, &comp.text);
        for (i, t) in comp.set.as_ref().map(|s| s.tracks.clone()).unwrap_or_default().into_iter().enumerate() {
            let tc = self.stored(&t.cid, at)?;
            let mut here = path.to_vec();
            here.push(i + 1);
            if tc.set.is_some() {
                self.set_tracks(&t.cid, &tc, at, &here, out)?;
            } else {
                out.push((here, t.cid.clone(), tc));
            }
        }
        Some(())
    }

    /// A stored set in a role of sections (`episodes=<cid>`): its tracks in order, each its sections in the order it plays them
    /// (one play a track), each written `section PREFIXtK_NAME from <track>.NAME`. Its mixes and cuts stay behind. With the note.
    fn set_run(&mut self, b: &Binding, cid: &str, comp: &crate::compile::Compiled, p: &str) -> Option<(Run, String)> {
        let at = b.source_span;
        let short = djengine::rights::short_cid(cid);
        let mut tracks = Vec::new();
        self.set_tracks(cid, comp, at, &[], &mut tracks)?;
        let (mut text, mut plays, mut bars) = (Vec::new(), Vec::new(), 0u32);
        let mut named: Vec<(String, Vec<usize>)> = Vec::new();
        let mut written: Vec<String> = Vec::new();
        // a path as a name (`2_1`) and in words (`track 1 of track 2`)
        let tag = |path: &[usize]| path.iter().map(usize::to_string).collect::<Vec<_>>().join("_");
        let words = |path: &[usize]| path.iter().rev().map(|k| format!("track {k}")).collect::<Vec<_>>().join(" of ");
        for (path, tcid, tc) in &tracks {
            // a track the set plays twice is written once, under its first place
            let k = match named.iter().find(|x| x.0 == *tcid) {
                Some(x) => x.1.clone(),
                None => {
                    named.push((tcid.clone(), path.clone()));
                    path.clone()
                }
            };
            let seq = sequence(tc);
            if seq.is_empty() {
                self.err(Diagnostic::error(at, format!("{} of the set {short} has no sections: `{}` takes a track's sections", words(path), b.role)).help(format!("bind another track, or a section of one: {}={cid}.track1.SECTION", b.role)));
                return None;
            }
            let mut play = Vec::new();
            for (n, nb) in &seq {
                // (a section's later generations, `drop@2`, are borrowed as the section: its first)
                let n = n.split('@').next().unwrap_or(n);
                let sec = format!("{p}t{}_{n}", tag(&k));
                if !written.contains(&sec) {
                    written.push(sec.clone());
                    text.push(format!("# {sec}: the {n} of {} of the set {short}", words(&k)));
                    text.push(format!("section {sec} from {tcid}.{n}"));
                }
                play.push(sec);
                bars += nb;
            }
            plays.push(play);
        }
        let note = format!("{}: the set {short}, {} track{}: {} section{}, {bars} bars a pass; its mixes and cuts stay behind", b.role, tracks.len(), if tracks.len() == 1 { "" } else { "s" }, written.len(), if written.len() == 1 { "" } else { "s" });
        Some((Run { text, plays, bars }, note))
    }

    /// A form written out on its own and compiled (for one of its sections): its program and its key, made once a check. The
    /// program is a function of the reference alone, so its plan's dice (a program's CID seeds them) are the same in any program:
    /// it is in the form's own key (no section of it says another) and at the form's own tempo, the host's only when a role in
    /// it is bound to a seed (a seed is generated at the program's tempo). What its bindings say is its own, not the user's.
    fn form_program(&mut self, f: &FormRef, depth: usize) -> Option<(Rc<crate::compile::Compiled>, String)> {
        let bare = FormRef { section: None, part: None, ..f.clone() };
        let id = bare.text();
        if let Some((_, c, k)) = self.forms.iter().find(|x| x.0 == id) {
            return Some((c.clone(), k.clone()));
        }
        let Some(tpl) = crate::template::find(&f.name) else {
            // (inner_form says what is wrong with the name)
            let _ = inner_form(self, &bare, "", depth);
            return None;
        };
        let key = own_key(tpl);
        let own_tempo = tpl.text().lines().find_map(|l| l.strip_prefix("tempo ")).and_then(|t| t.trim().parse::<f64>().ok());
        let tempo = if binds_gen(&bare.roles) { self.host.tempo.or(own_tempo) } else { own_tempo };
        let host = std::mem::replace(&mut self.host, Host { key: Some(key.clone()), tempo });
        let said = self.notes.len();
        let run = inner_form(self, &bare, "", depth);
        self.host = host;
        self.notes.truncate(said);
        let run = run?;
        let mut prog = String::new();
        if let Some(t) = tempo {
            prog.push_str(&format!("tempo {}\n", crate::print::num(t)));
        }
        prog.push_str(&format!("key {key}\n"));
        for l in &run.text {
            prog.push_str(l);
            prog.push('\n');
        }
        // its plan as it stands, so a section is what it is the first time it plays
        prog.push_str("arrange {\n");
        for l in run.plays.iter().flatten() {
            prog.push_str(&format!("  {l}\n"));
        }
        prog.push_str("}\n");
        let c = crate::compile::check_with(&prog, self.env);
        let Some(comp) = c.compiled else {
            let first = c.diagnostics.iter().find(|d| d.is_error()).map_or(String::new(), |d| format!(": {}", d.message));
            self.err(Diagnostic::error(f.span, format!("the {} written out on its own does not compile{first}", f.name)).help("bind its roles to sources that check"));
            return None;
        };
        let rc = Rc::new(comp);
        self.forms.push((id, rc.clone(), key.clone()));
        Some((rc, key))
    }

    /// The first stretch of section `s` of a form's program, or the error that says which sections it has.
    fn form_segment<'c>(&mut self, f: &FormRef, comp: &'c crate::compile::Compiled, at: Span) -> Option<(usize, &'c crate::compile::Segment)> {
        let s = f.section.clone().unwrap_or_default();
        if let Some(k) = comp.segments.iter().position(|g| g.header.is_some() && !g.chain && g.name == s) {
            return Some((k, &comp.segments[k]));
        }
        let mut names: Vec<String> = Vec::new();
        for (n, _) in sequence(comp) {
            let n = n.split('@').next().unwrap_or(&n).to_string();
            if !names.contains(&n) {
                names.push(n);
            }
        }
        let bare = FormRef { section: None, part: None, ..f.clone() }.text();
        let mut d = Diagnostic::error(at, format!("{bare} has no section `{s}`")).expected(&names.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{s}`"));
        let sung = if s == "sung" || s == "voices" { " (its sung section is the outer form's phrase, not one of its sections here)" } else { "" };
        d = match nearest(&s, names.iter().map(String::as_str)) {
            Some(n) => d.help(format!("did you mean `.{n}`? Its sections: {}{sung}", names.join(", "))),
            None => d.help(format!("its sections: {}{sung}", names.join(", "))),
        };
        self.err(d);
        None
    }

    /// A form's section where a line goes (`subject=template(cross).both`): the notes its part (the role's own, or the one it
    /// names: `.both.arp`) plays the first time the section plays, a let a bar, moved from the form's key into `key`.
    fn form_line(&mut self, b: &Binding, f: &FormRef, name: &str, part: &str, key: &str, depth: usize) -> Option<LineOut> {
        let at = b.source_span;
        let (comp, from) = self.form_program(f, depth + 1)?;
        let (_, seg) = self.form_segment(f, &comp, at)?;
        let s = f.section.clone().unwrap_or_default();
        let want = f.part.clone().unwrap_or_else(|| part.to_string());
        let bare = FormRef { section: None, part: None, ..f.clone() }.text();
        let hits: Vec<&crate::compile::PartIr> = seg.decks.values().flat_map(|d| d.parts.iter()).filter(|p| p.role == want).collect();
        let have: Vec<&str> = seg.decks.values().flat_map(|d| d.parts.iter().map(|p| p.role.as_str())).filter(|r| ROLES.contains(r)).collect();
        let p = match hits.as_slice() {
            [p] => *p,
            [] => {
                self.err(Diagnostic::error(at, format!("the {s} of {bare} plays no {want}; it plays {}", if have.is_empty() { "nothing".to_string() } else { have.join(", ") })).help(format!("name one of its parts: {}={bare}.{s}.{}", b.role, have.first().copied().unwrap_or("melody"))));
                return None;
            }
            _ => {
                self.err(Diagnostic::error(at, format!("the {s} of {bare} plays {want} on both decks")).help(format!("name another of its parts: {}={bare}.{s}.PART", b.role)));
                return None;
            }
        };
        if p.cycle_steps != 16 {
            self.err(Diagnostic::error(at, format!("the {want} of the {s} of {bare} has a cycle of its own ({} steps): a line here is bars of 16", p.cycle_steps)).help(format!("name another of its parts: {}={bare}.{s}.PART", b.role)));
            return None;
        }
        let texts = crate::gen::bar_texts(p, &want);
        let per = crate::seed::period(&texts).max(1);
        if per > MAX_LINE_BARS {
            self.err(Diagnostic::error(at, format!("its {want} is {per} bars before it repeats: a line is at most {MAX_LINE_BARS}")).help("name a part that repeats sooner, or another section"));
            return None;
        }
        let n = match (Key::parse(&from), Key::parse(key)) {
            (Some(a), Some(z)) => key_shift(a, z),
            _ => 0,
        };
        let bar = |t: &str| {
            let t = transpose_bar(t, n);
            if t.contains(' ') {
                t
            } else {
                format!("[{t}]")
            }
        };
        let mut lets: Vec<String> = texts[..per].iter().enumerate().map(|(i, t)| format!("let {name}_{} = {}", i + 1, bar(t))).collect();
        lets.push(format!("let {name} = <{}>", (1..=per).map(|i| format!("{name}_{i}")).collect::<Vec<_>>().join(" ")));
        let moved = if n == 0 { format!("in {from}") } else { format!("moved {n:+} semitones from {from} into {key}") };
        Some(LineOut { lets, bars: per, cycle: String::new(), note: format!("the {want} of the {s} of {bare} as it first plays: {per} bar{}, {moved}", if per == 1 { "" } else { "s" }), one_let: false })
    }

    /// A form's section where sections go (`episodes=template(cross).both`): that section as it first plays, its parts written
    /// out (as a stored program's section is), under the name of the section the role stands in for, in its own key.
    fn form_section(&mut self, b: &Binding, f: &FormRef, single: &str, key: &str, depth: usize) -> Option<Run> {
        let at = b.source_span;
        let (comp, _) = self.form_program(f, depth + 1)?;
        let (k, seg) = self.form_segment(f, &comp, at)?;
        let s = f.section.clone().unwrap_or_default();
        let bare = FormRef { section: None, part: None, ..f.clone() }.text();
        let bars = seg.bars + comp.segments[k + 1..].iter().take_while(|x| x.chain).map(|x| x.bars).sum::<u32>();
        let mut text = vec![format!("# {single}: the {s} of {bare}, as it first plays (its control lines stay behind)"), format!("section {single} {bars} bars {{")];
        if let Some(sk) = comp.section_keys.get(&s).copied().or(comp.key) {
            if Key::parse(key).map(|x| x.text()) != Some(sk.text()) {
                text.push(format!("  key {}", sk.text()));
            }
        }
        let mut any = false;
        for (deck, ir) in &seg.decks {
            let items: Vec<&crate::compile::PartIr> = ir.parts.iter().filter(|p| ROLES.contains(&p.role.as_str()) && f.part.as_deref().map_or(true, |o| p.role == o)).collect();
            for (i, p) in items.iter().enumerate() {
                any = true;
                text.push(if i == 0 { format!("  deck {}: {}", deck.name(), p.text) } else { format!("  {}| {}", " ".repeat("deck A: ".len() - 2), p.text) });
            }
        }
        if !any {
            self.err(Diagnostic::error(at, format!("the {s} of {bare} plays no parts{}", f.part.as_deref().map_or(String::new(), |o| format!(" of `{o}`")))).help("take another of its sections"));
            return None;
        }
        text.push("}".to_string());
        Some(Run { text, plays: Vec::new(), bars })
    }

    /// A gen call made to play in `key` (the sections it fills): it says the key itself when that is not the program's.
    fn keyed(&self, c: &GenCall, key: &str) -> GenCall {
        let mut c = c.clone();
        if c.key.is_none() && self.host.key.as_deref() != Some(key) {
            c.key = Some(key.to_string());
        }
        c
    }

    /// The line `b` binds role `spec` (or a seed role's `part`, written as `name`) to, in `key`; `depth` is the form's.
    fn line(&mut self, b: &Binding, name: &str, part: &str, flex: bool, key: &str, depth: usize) -> Option<LineOut> {
        let at = b.source_span;
        match &b.source {
            Source::Gen(c) => {
                let mut c = self.keyed(c, key);
                if c.part.is_none() {
                    c.part = Some(part.to_string());
                }
                let p = c.part.clone().unwrap_or_default();
                if c.section.is_none() {
                    let g = self.generated(&c, at)?;
                    match g.section_for(&[p.as_str()]) {
                        Some(s) => c.section = Some(s),
                        None => {
                            self.err(Diagnostic::error(at, format!("{} plays no {p} on the bar's grid in any section", g.call.text())).help(format!("bind another part: {}={} part=melody", b.role, g.call.text())));
                            return None;
                        }
                    }
                }
                let bars = self.probe(&c, at)?;
                if bars > MAX_LINE_BARS {
                    self.err(Diagnostic::error(at, format!("its {p} is {bars} bars before it repeats: a line is at most {MAX_LINE_BARS}")).help("bind a part that repeats sooner, or another section"));
                    return None;
                }
                let call = c.text();
                let fitted = if c.key.is_some() && b_key(b).is_none() { format!(", in {key}") } else { String::new() };
                Some(LineOut { lets: vec![format!("let {name} = {call}")], bars, cycle: String::new(), note: format!("{call}: {bars} bar{}{fitted}", if bars == 1 { "" } else { "s" }), one_let: false })
            }
            Source::Theme(id) => {
                let Some(t) = djengine::themes::theme(id) else {
                    let ids: Vec<&str> = djengine::themes::themes().iter().map(|t| t.id.as_str()).collect();
                    let mut d = Diagnostic::error(at, format!("`{id}` is no theme of the public-domain library")).expected(&ids).found(format!("`{id}`"));
                    d = match nearest(id, ids.iter().copied()) {
                        Some(n) => d.help(format!("did you mean `{n}`? The themes: {}", ids.join(", "))),
                        None => d.help(format!("the themes: {}", ids.join(", "))),
                    };
                    self.err(d);
                    return None;
                };
                if !flex && t.steps != 16 {
                    self.err(Diagnostic::error(at, format!("`{id}` is in {} ({} steps a bar): this role's copies are made of bars of 16", t.meter, t.steps)).help("a theme in 4/4 or 2/4, or notes"));
                    return None;
                }
                let k = Key::parse(key).unwrap_or(Key::C_MAJOR);
                let n = theme_shift(t, k);
                let mut lets: Vec<String> = t.bars.iter().enumerate().map(|(i, bar)| format!("let {name}_{} = {}", i + 1, transpose_bar(bar, n))).collect();
                lets.push(format!("let {name} = <{}>", (1..=t.bars.len()).map(|i| format!("{name}_{i}")).collect::<Vec<_>>().join(" ")));
                let moved = if n == 0 { format!("in {}", t.key) } else { format!("moved {n:+} semitones from {} into {key}", t.key) };
                Some(LineOut { lets, bars: t.bars.len(), cycle: t.cycle_mark(), note: format!("{id} ({}, {}, {}): {} bar{}, {moved}", t.work, t.composer, t.year, t.bars.len(), if t.bars.len() == 1 { "" } else { "s" }), one_let: false })
            }
            Source::Notes(bars) => {
                if bars.len() > MAX_LINE_BARS {
                    self.err(Diagnostic::error(at, format!("these notes are {} bars: a line is at most {MAX_LINE_BARS}", bars.len())).help("bind fewer bars: a line loops"));
                    return None;
                }
                let mut lets: Vec<String> = bars.iter().enumerate().map(|(i, bar)| format!("let {name}_{} = {bar}", i + 1)).collect();
                lets.push(format!("let {name} = <{}>", (1..=bars.len()).map(|i| format!("{name}_{i}")).collect::<Vec<_>>().join(" ")));
                Some(LineOut { lets, bars: bars.len(), cycle: String::new(), note: format!("notes: {} bar{}", bars.len(), if bars.len() == 1 { "" } else { "s" }), one_let: false })
            }
            Source::Stored { cid, track, section, part: p } => {
                let short = djengine::rights::short_cid(cid);
                if p.is_none() && section.is_none() {
                    let set = track.is_none() && self.try_stored(cid, at).is_ok_and(|c| c.set.is_some());
                    let help = if set { format!("{}={cid}.track1.SECTION.{part}", b.role) } else { format!("{}={}.bass", b.role, b.source.text()) };
                    self.err(Diagnostic::error(at, format!("`{}` takes a line: one part of a stored program{}", b.role, if set { "; this is a set, so one part of one of its tracks" } else { "" })).help(help));
                    return None;
                }
                // a set's track is that track's program (and after a program that is no set, `.track2` is its section `track2`)
                let (cid, section, of) = match track {
                    Some(k) => self.track_ref(cid, *k, section.as_ref(), at)?,
                    None => (cid.clone(), section.clone(), short.clone()),
                };
                // a section where a line goes: the role's own part of it
                let inferred = p.is_none();
                let p = p.clone().unwrap_or_else(|| part.to_string());
                if let Some(comp) = self.stored(&cid, at) {
                    if comp.set.is_some() {
                        match &section {
                            Some(s) => self.not_a_track(b, &cid, &comp, s),
                            None => self.err(Diagnostic::error(at, format!("{} is a set: `{}` takes one part of one of its tracks", djengine::rights::short_cid(&cid), b.role)).help(format!("{}={cid}.track1.SECTION.{p}", b.role))),
                        }
                        return None;
                    }
                    let plays = sequence(&comp);
                    if let Some(s) = &section {
                        if !plays.iter().any(|(n, _)| n == s) {
                            let names: Vec<String> = plays.iter().map(|(n, _)| n.clone()).collect();
                            self.err(Diagnostic::error(at, format!("{of} has no section `{s}`")).help(format!("its sections: {}", names.join(", "))));
                            return None;
                        }
                        if inferred && !plays_part(&comp, s, &p) {
                            let have: Vec<String> = comp.segments.iter().filter(|g| g.header.is_some() && !g.chain && g.name == *s).flat_map(|g| g.decks.values().flat_map(|d| d.parts.iter().map(|x| x.role.clone()))).filter(|r| ROLES.contains(&r.as_str())).collect();
                            self.err(Diagnostic::error(at, format!("the {s} of {of} plays no {p} on the bar's grid: `{}` takes its {p}", b.role)).help(format!("name one of its parts: {}={}.{}", b.role, b.source.text(), have.first().map_or("melody", String::as_str))));
                            return None;
                        }
                    }
                }
                let r = Source::Stored { cid: cid.clone(), track: None, section: section.clone(), part: Some(p.clone()) }.text();
                let note = match (inferred, &section) {
                    (true, Some(s)) => format!("the {p} of the {s} of {of}: one let"),
                    _ => format!("the {p} of {of}: one let"),
                };
                Some(LineOut { lets: vec![format!("let {name}_1 = {r}"), format!("let {name} = <{name}_1>")], bars: 1, cycle: String::new(), note, one_let: true })
            }
            Source::Form(f) if f.section.is_some() => self.form_line(b, f, name, part, key, depth),
            Source::Form(f) => {
                self.err(Diagnostic::error(f.span, format!("`{}` takes a line, and a form is sections", b.role)).help(format!("bind it to a gen call's part, a theme, notes, a stored program's part, or one section of the form: {}={}.SECTION", b.role, f.text())));
                None
            }
            Source::Number(n) => {
                self.err(Diagnostic::error(at, format!("`{}` is a role: it is bound to a source, not the number {n}", b.role)).help(BINDING));
                None
            }
        }
    }
}

/// Does any role (of this form, or of a form in it) bind a seed of the generator?
fn binds_gen(roles: &[Binding]) -> bool {
    roles.iter().any(|b| match &b.source {
        Source::Gen(_) => true,
        Source::Form(f) => binds_gen(&f.roles),
        _ => false,
    })
}

/// The key a binding's own call says, if any.
fn b_key(b: &Binding) -> Option<&str> {
    match &b.source {
        Source::Gen(c) => c.key.as_deref(),
        _ => None,
    }
}

/// Write a bound line in place of the default's let `id` (and its bars): the comment, the lets; the parts that played the
/// default play it (with its cycle), and every name of the default's bars is the line's (`id_3` is the line's third bar, or
/// the one it comes round to).
fn apply_line(doc: &mut Doc, id: &str, name: &str, default_cycle: &str, above: &str, comment: String, out: &LineOut) -> bool {
    let Some(r) = doc.block(id) else { return false };
    let mut start = r.start;
    if !above.is_empty() && start > 0 && doc.lines[start - 1] == above {
        start -= 1;
    }
    let mut with = vec![comment];
    with.extend(out.lets.iter().cloned());
    doc.splice(start..r.end, with);
    // the parts that played it: their cycle is the line's
    if default_cycle != out.cycle {
        for (l, fixed) in doc.lines.iter_mut().zip(&doc.fixed) {
            if *fixed || is_comment(l) {
                continue;
            }
            for role in ROLES {
                let from = format!("{role}{default_cycle} {id}");
                let to = format!("{role}{} {id}", out.cycle);
                let mut s = String::new();
                let mut rest: &str = l;
                while let Some(i) = rest.find(&from) {
                    let after = &rest[i + from.len()..];
                    let before_ok = i == 0 || !rest[..i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                    let after_ok = !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
                    s.push_str(&rest[..i]);
                    s.push_str(if before_ok && after_ok { &to } else { &from });
                    rest = after;
                }
                s.push_str(rest);
                *l = s;
            }
        }
    }
    let k = out.bars.max(1);
    let bar_of = |w: &str| -> Option<String> {
        let n: usize = w.strip_prefix(id)?.strip_prefix('_')?.parse().ok()?;
        Some(format!("{name}_{}", (n.max(1) - 1) % k + 1))
    };
    doc.rename(&mut |w| {
        if w == id {
            Some(name.to_string())
        } else if w.strip_prefix(id).is_some_and(|r| r == "_*") {
            Some(format!("{name}_*"))
        } else {
            bar_of(w)
        }
    });
    true
}

/// The places role `section` plays in the arrangement, each given a share of `plays` in order (as even as they go; with fewer
/// plays than places, they come round).
fn fill_slots(doc: &mut Doc, section: &str, plays: &[Vec<String>]) {
    let Some((s, e)) = doc.arrange() else { return };
    let slots: Vec<usize> = (s + 1..e).filter(|i| doc.lines[*i].trim() == section).collect();
    let n = slots.len();
    let m = plays.len();
    if n == 0 || m == 0 {
        return;
    }
    // from the last, so the earlier places stay where they are
    for (k, i) in slots.iter().enumerate().rev() {
        let share: Vec<&Vec<String>> = if m >= n { plays[k * m / n..(k + 1) * m / n].iter().collect() } else { vec![&plays[k % m]] };
        let lines: Vec<String> = share.into_iter().flatten().map(|l| format!("  {l}")).collect();
        doc.splice(*i..*i + 1, lines);
    }
}

/// The plays of an arrange block's lines: each section with the mutation lines after it (those before the first go with it).
fn plays_of(lines: &[String], sections: &[String]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut lead: Vec<String> = Vec::new();
    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        if sections.iter().any(|s| s == t) {
            let mut p = std::mem::take(&mut lead);
            p.insert(0, t.to_string());
            out.push(p);
        } else if let Some(last) = out.last_mut() {
            last.push(t.to_string());
        } else {
            lead.push(t.to_string());
        }
    }
    if let (Some(first), false) = (out.first_mut(), lead.is_empty()) {
        first.extend(lead);
    }
    out
}

/// The top-level `key` line of a template's text.
fn own_key(tpl: &Template) -> String {
    crate::music::canonical_pitch_word(tpl.text().lines().find_map(|l| l.strip_prefix("key ")).map_or("c", str::trim))
}

/// The header line a bound form says its roles with (the fourth comment of a template: the one that names its themes), and a
/// line for each number the reference gives.
fn header(specs: &[RoleSpec], bound: &BTreeMap<&str, String>, params: &[ParamSpec], given: &BTreeMap<&str, i64>) -> Vec<String> {
    let mut out: Vec<String> = specs.iter().map(|s| format!("# {}: {}; {}", s.name, bound.get(s.name).cloned().unwrap_or_else(|| s.default.replace('`', "")), s.what)).collect();
    out.extend(params.iter().filter_map(|p| given.get(p.name).map(|n| format!("# {}: {n}; {}", p.name, p.what))));
    out
}

/// A delay of `d` beats in words: `a bar`, `half a bar`, `a bar and 2 beats`, `3 beats`.
fn beats_words(d: i64) -> String {
    let (q, r) = (d / 4, d % 4);
    let bars = match q {
        0 => String::new(),
        1 => "a bar".to_string(),
        n => format!("{n} bars"),
    };
    let beats = match r {
        0 => String::new(),
        1 => "a beat".to_string(),
        2 if q == 0 => "half a bar".to_string(),
        n => format!("{n} beats"),
    };
    match (bars.is_empty(), beats.is_empty()) {
        (false, false) => format!("{bars} and {beats}"),
        (false, true) => bars,
        _ => beats,
    }
}

/// An interval of `n` semitones in words: `an octave down`, `a fifth up`, `in unison`, `10 semitones up`.
fn interval_words(n: i64) -> String {
    let dir = if n < 0 { "down" } else { "up" };
    let name = match n.abs() {
        0 => return "in unison".to_string(),
        1 => "a semitone",
        2 => "a whole tone",
        3 => "a minor third",
        4 => "a major third",
        5 => "a fourth",
        7 => "a fifth",
        12 => "an octave",
        19 => "an octave and a fifth",
        24 => "two octaves",
        a => return format!("{a} semitones {dir}"),
    };
    format!("{name} {dir}")
}

/// The canon's chase copy of subject `s` (`k` bars, `s_1` ...) `d` beats behind: its bars come round `d / 4` bars late (the last
/// first), and the beats left over move each bar along (`rotate`, wrapping in the bar, as the form's own `close` copy does).
fn follow_text(s: &str, k: usize, d: i64) -> String {
    let (q, r) = ((d / 4) as usize % k.max(1), (d % 4) as usize);
    let order: Vec<String> = (0..k).map(|i| format!("{s}_{}", (i + k - q) % k + 1)).collect();
    match (q, r) {
        (0, r) => format!("rotate({s}, {})", 4 * r),
        (_, 0) => format!("<{}>", order.join(" ")),
        (_, r) => format!("<{}>", order.iter().map(|b| format!("rotate({b}, {})", 4 * r)).collect::<Vec<_>>().join(" ")),
    }
}

/// How the chase plays its copy `follow` at `n` semitones from the leader: `octave(follow, -1)` an octave down, `transpose(follow,
/// 7)` a fifth up, `follow` in unison.
fn interval_text(follow: &str, n: i64) -> String {
    if n == 0 {
        follow.to_string()
    } else if n % 12 == 0 {
        format!("octave({follow}, {})", n / 12)
    } else {
        format!("transpose({follow}, {n})")
    }
}

/// `name` is no role (nor number) of the form, with the nearest.
fn unknown_role(tpl: &Template, name: &str, at: Span, names: &[&str], pnames: &[&str]) -> Diagnostic {
    let all: Vec<&str> = names.iter().chain(pnames).copied().collect();
    let numbers = if pnames.is_empty() { String::new() } else { format!("; its numbers: {}", pnames.join(", ")) };
    let d = Diagnostic::error(at, format!("`{name}` is no role of the {}", tpl.name)).expected(&all).found(format!("`{name}`"));
    match nearest(name, all.iter().copied()) {
        Some(n) => d.help(format!("did you mean `{n}`? The {}'s roles: {}{numbers}", tpl.name, names.join(", "))),
        None => d.help(format!("the {}'s roles: {}{numbers}", tpl.name, names.join(", "))),
    }
}

/// Is binding `b` well formed for the form: a role with a source, a seed role's part with a source, a number of the form with a
/// whole number in its range? The error when not.
fn check_binding(tpl: &Template, specs: &[RoleSpec], pspecs: &[ParamSpec], b: &Binding) -> Option<Diagnostic> {
    let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
    let pnames: Vec<&str> = pspecs.iter().map(|p| p.name).collect();
    let number_in_role = || Diagnostic::error(b.source_span, format!("`{}` is a role: it is bound to a source, not a number", b.role)).help(BINDING);
    if let Some((base, sub)) = b.role.split_once('.') {
        let Some(s) = specs.iter().find(|s| s.name == base) else { return Some(unknown_role(tpl, base, b.role_span, &names, &pnames)) };
        if s.parts.is_empty() {
            let with: Vec<String> = TEMPLATES_WITH_PARTS.iter().map(|x| x.to_string()).collect();
            return Some(Diagnostic::error(b.role_span, format!("`{base}` is one line: it has no parts to bind one of")).help(format!("{base}=SOURCE binds it; the roles with parts: {}", with.join(", "))));
        }
        let ps: Vec<&str> = s.parts.iter().map(|p| p.0).collect();
        if !ps.contains(&sub) {
            let d = Diagnostic::error(b.role_span, format!("`{sub}` is no part of `{base}`")).expected(&ps).found(format!("`{sub}`"));
            return Some(match nearest(sub, ps.iter().copied()) {
                Some(n) => d.help(format!("did you mean `{base}.{n}`? `{base}`'s parts: {}", ps.join(", "))),
                None => d.help(format!("`{base}`'s parts: {}", ps.join(", "))),
            });
        }
        return matches!(b.source, Source::Number(_)).then(number_in_role);
    }
    if let Some(p) = pspecs.iter().find(|p| p.name == b.role) {
        return match b.source {
            Source::Number(n) if n < p.min || n > p.max => Some(Diagnostic::error(b.source_span, format!("`{}` is {} to {}: {}", p.name, p.min, p.max, p.what)).help(p.example)),
            Source::Number(_) => None,
            _ => Some(Diagnostic::error(b.source_span, format!("`{}` is a whole number: {}", p.name, p.what)).help(p.example)),
        };
    }
    if names.contains(&b.role.as_str()) {
        return matches!(b.source, Source::Number(_)).then(number_in_role);
    }
    Some(unknown_role(tpl, &b.role, b.role_span, &names, &pnames))
}

/// The roles that have parts of their own, bound one at a time (`a.bass=`), as `form.role`.
const TEMPLATES_WITH_PARTS: [&str; 2] = ["cross.a", "cross.b"];

/// The form `tpl` written out (`text`: its default, as unbound) with `roles` bound. An inner form (`depth` > 1) has every name
/// it defines start with `prefix` and its sections in `key`; the top one says its roles in its header.
pub(crate) fn bind_form(cx: &mut Ctx, tpl: &'static Template, text: &str, roles: &[Binding], prefix: &str, key: &str, depth: usize) -> Option<String> {
    let specs = specs(tpl.name);
    let pspecs = params(tpl.name);
    let errs = cx.errors.len();
    if specs.is_empty() && pspecs.is_empty() && !roles.is_empty() {
        let with = forms_with_roles();
        cx.err(Diagnostic::error(roles[0].role_span, format!("`{}` takes no roles yet: the templates with roles are {}", tpl.name, with.join(", "))));
        return None;
    }
    // each role (or part of one, or number) once, and only the form's
    for (i, b) in roles.iter().enumerate() {
        if let Some(d) = check_binding(tpl, specs, pspecs, b) {
            cx.err(d);
        } else if roles[..i].iter().any(|x| x.role == b.role) {
            cx.err(Diagnostic::error(b.role_span, format!("`{}` is bound twice", b.role)).help("bind each role once"));
        }
    }
    if cx.errors.len() > errs {
        return None;
    }
    let given: BTreeMap<&str, i64> = roles.iter().filter_map(|b| if let Source::Number(n) = b.source { Some((b.role.as_str(), n)) } else { None }).collect();
    let mut doc = Doc::new(text);
    let mut bound: BTreeMap<&str, String> = BTreeMap::new();
    let mut lens: BTreeMap<&str, usize> = BTreeMap::new();
    let mut one_let: Vec<&str> = Vec::new();
    for spec in specs {
        let whole = roles.iter().find(|b| b.role == spec.name);
        // a seed role: a line for each of its parts; a part bound alone (`a.bass=`) in place of what the whole gives it
        if !spec.parts.is_empty() {
            let subs: Vec<&Binding> = roles.iter().filter(|b| b.role.split_once('.').is_some_and(|(r, _)| r == spec.name)).collect();
            if whole.is_none() && subs.is_empty() {
                continue;
            }
            let part_of = |b: &Binding| b.role.split_once('.').and_then(|(_, p)| spec.parts.iter().position(|x| x.0 == p)).unwrap_or(0);
            let taken: Vec<usize> = subs.iter().copied().map(|s| part_of(s)).collect();
            let twice = |k: usize| -> Diagnostic {
                let s = subs.iter().copied().find(|s| part_of(s) == k).expect("taken");
                Diagnostic::error(s.role_span, format!("`{}`'s {} is bound twice: by `{}=` and by `{}=`", spec.name, spec.parts[k].0, spec.name, s.role)).help("bind each part once")
            };
            // (part, what it writes, the source it says, bound alone)
            let mut outs: Vec<(usize, LineOut, String, bool)> = Vec::new();
            if let Some(b) = whole {
                let src = b.source.text();
                let mut mine: Vec<(usize, LineOut)> = Vec::new();
                // a whole seed, or a section, gives each part it is not bound alone
                let section_whole = matches!(&b.source, Source::Stored { section: Some(_), part: None, .. }) || matches!(&b.source, Source::Form(f) if f.section.is_some() && f.part.is_none());
                if (section_whole || matches!(&b.source, Source::Gen(c) if c.part.is_none())) && taken.len() == spec.parts.len() {
                    let alone: Vec<String> = spec.parts.iter().map(|p| format!("{}.{}", spec.name, p.0)).collect();
                    cx.err(Diagnostic::error(b.source_span, format!("`{}=` gives nothing: each of its parts is bound alone", spec.name)).help(format!("drop `{}=` or one of {}", spec.name, alone.join(", "))));
                    return None;
                }
                match &b.source {
                    Source::Gen(c) if c.part.is_none() => {
                        let c = cx.keyed(c, key);
                        let need: Vec<usize> = (0..spec.parts.len()).filter(|k| !taken.contains(k)).collect();
                        let section = match &c.section {
                            Some(s) => Some(s.clone()),
                            None if need.is_empty() => None,
                            None => {
                                let g = cx.generated(&c, b.source_span)?;
                                let all: Vec<&str> = need.iter().map(|k| spec.parts[*k].0).collect();
                                g.section_for(&all)
                            }
                        };
                        for k in need {
                            let (part, _, let_name) = spec.parts[k];
                            let call = GenCall { section: section.clone(), part: Some(part.to_string()), ..c.clone() };
                            let one = Binding { source: Source::Gen(call), ..b.clone() };
                            mine.push((k, cx.line(&one, &format!("{prefix}{let_name}"), part, false, key, depth)?));
                        }
                    }
                    Source::Gen(c) => {
                        let part = c.part.clone().unwrap_or_default();
                        let Some(k) = spec.parts.iter().position(|p| p.0 == part) else {
                            let ps: Vec<&str> = spec.parts.iter().map(|p| p.0).collect();
                            cx.err(Diagnostic::error(b.source_span, format!("`{}` takes a seed's {}: not its {part}", b.role, ps.join(", "))).help(format!("{}={}", b.role, c.program_text())));
                            return None;
                        };
                        if taken.contains(&k) {
                            cx.err(twice(k));
                            return None;
                        }
                        let (p, _, let_name) = spec.parts[k];
                        mine.push((k, cx.line(b, &format!("{prefix}{let_name}"), p, false, key, depth)?));
                    }
                    _ if section_whole => {
                        for (k, (part, _, let_name)) in spec.parts.iter().enumerate().filter(|(k, _)| !taken.contains(k)) {
                            let source = match &b.source {
                                Source::Stored { cid, track, section, .. } => Source::Stored { cid: cid.clone(), track: *track, section: section.clone(), part: Some(part.to_string()) },
                                Source::Form(f) => Source::Form(FormRef { part: Some(part.to_string()), ..f.clone() }),
                                s => s.clone(),
                            };
                            let one = Binding { source, ..b.clone() };
                            mine.push((k, cx.line(&one, &format!("{prefix}{let_name}"), part, false, key, depth)?));
                        }
                    }
                    _ => {
                        // a line: the part of a stored program (or of a form's section) it is, else the melody
                        let k = match &b.source {
                            Source::Stored { part: Some(p), .. } => spec.parts.iter().position(|x| x.0 == p).unwrap_or(0),
                            Source::Form(FormRef { part: Some(p), .. }) => spec.parts.iter().position(|x| x.0 == p).unwrap_or(0),
                            _ => 0,
                        };
                        if taken.contains(&k) {
                            cx.err(twice(k));
                            return None;
                        }
                        let (p, _, let_name) = spec.parts[k];
                        mine.push((k, cx.line(b, &format!("{prefix}{let_name}"), p, false, key, depth)?));
                    }
                }
                let said: Vec<String> = mine.iter().map(|(k, out)| format!("its {} {}", spec.parts[*k].0, out.note)).collect();
                if !said.is_empty() {
                    cx.notes.push(Diagnostic::info(b.source_span, format!("{}: {}", spec.name, said.join("; "))));
                }
                outs.extend(mine.into_iter().map(|(k, o)| (k, o, src.clone(), false)));
            }
            for s in subs.iter().copied() {
                let k = part_of(s);
                let (part, _, let_name) = spec.parts[k];
                let out = cx.line(s, &format!("{prefix}{let_name}"), part, false, key, depth)?;
                cx.notes.push(Diagnostic::info(s.source_span, format!("{}: {} (`{prefix}{let_name}_*`)", s.role, out.note)));
                outs.push((k, out, s.source.text(), true));
            }
            outs.sort_by_key(|o| o.0);
            for (k, out, src, alone) in &outs {
                let (part, id, let_name) = spec.parts[*k];
                let comment = if *alone { format!("# {}.{part}, bound: {src}", spec.name) } else { format!("# {}'s {part}, bound: {src}", spec.name) };
                if !apply_line(&mut doc, &format!("{prefix}{id}"), &format!("{prefix}{let_name}"), "", "", comment, out) {
                    cx.err(Diagnostic::error(whole.map_or(subs[0].source_span, |b| b.source_span), format!("the {} template has no `{id}` to bind (this is a bug in the template)", tpl.name)));
                    return None;
                }
            }
            let mut said = whole.map_or_else(|| spec.default.replace('`', ""), |b| b.source.text());
            for s in &subs {
                said.push_str(&format!(", {}={}", s.role, s.source.text()));
            }
            bound.insert(spec.name, said);
            continue;
        }
        let Some(b) = whole else { continue };
        let src = b.source.text();
        // a role that takes sections, bound to sections
        let sections_source = match &b.source {
            Source::Form(f) => f.part.is_none(),
            Source::Gen(c) => c.part.is_none() && (c.section.is_some() || spec.section.is_some()),
            Source::Stored { part, .. } => part.is_none(),
            _ => false,
        };
        if let (Some(section), true) = (spec.section, sections_source) {
            let p = format!("{prefix}{}", spec.prefix);
            let mut said: Option<String> = None;
            let mut set_cid: Option<String> = None;
            let run = match &b.source {
                Source::Form(f) if f.section.is_some() => cx.form_section(b, f, &format!("{prefix}{section}"), key, depth)?,
                Source::Form(f) => inner_form(cx, f, &p, depth + 1)?,
                Source::Gen(c) => {
                    let c = cx.keyed(c, key);
                    let g = cx.generated(&c, b.source_span)?;
                    let plays = g.plays();
                    let pick: Vec<(String, u32)> = match &c.section {
                        Some(s) => match plays.iter().find(|(n, _)| n == s) {
                            Some(x) => vec![x.clone()],
                            None => {
                                let names: Vec<String> = plays.iter().map(|(n, _)| n.clone()).collect();
                                cx.err(Diagnostic::error(b.source_span, format!("{} has no section `{s}`", g.call.text())).help(format!("its sections: {}", names.join(", "))));
                                return None;
                            }
                        },
                        None => plays,
                    };
                    borrowed(&p, &format!("{prefix}{section}"), &pick, &|n| GenCall { section: Some(n.to_string()), part: None, ..c.clone() }.text(), &c.program_text())
                }
                Source::Stored { cid, track, section: s, .. } => {
                    let short = djengine::rights::short_cid(cid);
                    // a set's track is that track's program (and after a program that is no set, `.track2` is its section `track2`)
                    let (cid, s, of) = match track {
                        Some(k) => cx.track_ref(cid, *k, s.as_ref(), b.source_span)?,
                        None => (cid.clone(), s.clone(), short.clone()),
                    };
                    let comp = cx.stored(&cid, b.source_span)?;
                    if comp.set.is_some() {
                        // a set: its tracks in order, each its sections
                        if let Some(s) = &s {
                            cx.not_a_track(b, &cid, &comp, s);
                            return None;
                        }
                        let (run, note) = cx.set_run(b, &cid, &comp, &p)?;
                        said = Some(note);
                        set_cid = Some(cid.clone());
                        run
                    } else {
                        let plays = sequence(&comp);
                        let pick: Vec<(String, u32)> = match &s {
                            Some(s) => match plays.iter().find(|(n, _)| n == s) {
                                Some(x) => vec![x.clone()],
                                None => {
                                    let names: Vec<String> = plays.iter().map(|(n, _)| n.clone()).collect();
                                    cx.err(Diagnostic::error(b.source_span, format!("{of} has no section `{s}`")).help(format!("its sections: {}", names.join(", "))));
                                    return None;
                                }
                            },
                            None => plays,
                        };
                        if pick.is_empty() {
                            cx.err(Diagnostic::error(b.source_span, format!("{of} has no sections to take")).help(format!("bind one of its parts: {}={}.bass", b.role, b.source.text())));
                            return None;
                        }
                        borrowed(&p, &format!("{prefix}{section}"), &pick, &|n| format!("{cid}.{n}"), &of)
                    }
                }
                _ => unreachable!("a sections source"),
            };
            if run.bars > MAX_SECTION_BARS {
                let one = set_cid.map_or(String::new(), |c| format!("; or one of the set's tracks: {}={c}.track1", b.role));
                cx.err(Diagnostic::error(b.source_span, format!("this is {} bars of sections a pass: `{}` takes at most {MAX_SECTION_BARS}", run.bars, b.role)).help(format!("bind a section of it, or a shorter one (length=standard){one}")));
                return None;
            }
            let Some((d, _, e)) = doc.section(&format!("{prefix}{section}")) else {
                cx.err(Diagnostic::error(b.source_span, format!("the {} template has no section `{section}` (this is a bug in the template)", tpl.name)));
                return None;
            };
            let mut with = vec![format!("# {}, bound: {src}", spec.name)];
            with.extend(run.text);
            doc.splice(d..e + 1, with);
            fill_slots(&mut doc, &format!("{prefix}{section}"), &run.plays);
            let said = said.unwrap_or_else(|| format!("{}: {} section{}, {} bars a pass", spec.name, run.plays.len(), if run.plays.len() == 1 { "" } else { "s" }, run.bars));
            cx.notes.push(Diagnostic::info(b.source_span, said));
            bound.insert(spec.name, src);
            continue;
        }
        // a line
        let name = format!("{prefix}{}", spec.let_name);
        let out = cx.line(b, &name, spec.part, spec.flex, key, depth)?;
        let comment = format!("# {}, bound: {src}", spec.name);
        if !apply_line(&mut doc, &format!("{prefix}{}", spec.id), &name, spec.cycle, spec.above, comment, &out) {
            cx.err(Diagnostic::error(b.source_span, format!("the {} template has no `{}` to bind (this is a bug in the template)", tpl.name, spec.id)));
            return None;
        }
        // the lets only the default was made of go with it
        for id in spec.drops {
            if let Some(r) = doc.block(&format!("{prefix}{id}")) {
                doc.lines.drain(r.clone());
                doc.fixed.drain(r);
            }
        }
        cx.notes.push(Diagnostic::info(b.source_span, format!("{}: {} (`{name}_*`)", spec.name, out.note)));
        lens.insert(spec.name, out.bars);
        if out.one_let {
            one_let.push(spec.name);
        }
        bound.insert(spec.name, src);
    }
    // the canon's copies are made of its subject's bars: a bar behind is the last bar first
    if tpl.name == "canon" {
        if let Some(k) = lens.get("subject").copied() {
            let s = format!("{prefix}subject");
            let follow = format!("let {prefix}follow = ");
            if let Some(i) = doc.lines.iter().position(|l| l.starts_with(&follow)) {
                doc.lines[i] = if k == 1 { format!("{follow}rotate({s}, 8)") } else { format!("{follow}<{s}_{k} {}>", (1..k).map(|n| format!("{s}_{n}")).collect::<Vec<_>>().join(" ")) };
                doc.fixed[i] = true;
            }
            if k == 1 {
                for l in doc.lines.iter_mut().filter(|l| is_comment(l)) {
                    *l = l.replace("the copy a bar behind", "the copy half a bar behind").replace("a bar behind, and a beat behind", "half a bar behind, and a beat behind");
                }
            }
        }
        canon_numbers(cx, &mut doc, roles, &lens, one_let.contains(&"subject"), prefix)?;
    }
    // a bar of one step the plan morphs or crosses toward is written in brackets (`[d2]`, as a seed's is): a bare `d2` is no
    // pattern `morph` or `xfer` takes. Only there: anywhere else the bare step checks, and its text (so its CID, so the plan's
    // dice) stays as it always was
    let targets: Vec<String> = doc
        .lines
        .iter()
        .filter(|l| matches!(l.trim_start().chars().next(), Some('~' | '@')))
        .flat_map(|l| l.split_whitespace().collect::<Vec<_>>().windows(2).filter(|w| w[0] == "morph" || w[0] == "xfer").map(|w| w[1].to_string()).collect::<Vec<_>>())
        .collect();
    for (l, fixed) in doc.lines.iter_mut().zip(&doc.fixed) {
        if !*fixed {
            continue;
        }
        if let Some((head, body)) = l.split_once(" = ") {
            // (a stored program's part is a reference the stored stage writes out: it stays as it is)
            if head.strip_prefix("let ").is_some_and(|n| targets.iter().any(|t| t == n)) && !body.contains(' ') && !body.starts_with('[') && !crate::stored::is_ref_word(body) {
                *l = format!("{head} = [{body}]");
            }
        }
    }
    if depth == 1 {
        // the header says what fills each role: the fourth comment line, the one that named the themes
        if doc.lines.len() > 3 && is_comment(&doc.lines[3]) && !doc.lines[3].starts_with("# phrase:") && !doc.lines[3].starts_with("# vocal:") {
            doc.splice(3..4, header(specs, &bound, pspecs, &given));
        }
    }
    if cx.errors.len() > errs {
        return None;
    }
    Some(doc.text())
}

/// The canon's `delay=` and `interval=`: the chase's copy (`follow`) that many beats behind the leader, played at that interval
/// from it. Without them the copy is a bar behind (half a bar for a one-bar subject) and an octave down, as it always was. The
/// round's third voice (`close`) stays a beat behind, in unison.
fn canon_numbers(cx: &mut Ctx, doc: &mut Doc, roles: &[Binding], lens: &BTreeMap<&str, usize>, one_let: bool, prefix: &str) -> Option<()> {
    let num = |name: &str| roles.iter().find(|b| b.role == name).and_then(|b| if let Source::Number(n) = b.source { Some((n, b.source_span)) } else { None });
    let follow = format!("{prefix}follow");
    if let Some((d, at)) = num("delay") {
        // the subject: the bound line, else the form's own theme
        let (s, k) = match lens.get("subject") {
            Some(k) => (format!("{prefix}subject"), *k),
            None => (format!("{prefix}{}", CANON[0].id), djengine::themes::theme(CANON[0].id).map_or(1, |t| t.bars.len())),
        };
        let most = 4 * k as i64 - 1;
        if one_let && d > most {
            // (a stored part is one let, however many bars it has: its copy moves within the bar)
            cx.err(Diagnostic::error(at, format!("`delay` is 1 to {most} beats here: the subject is a stored part, one let (`{prefix}subject_1`), so its copy moves within the bar")).help("delay=2; for a delay of a bar or more bind the subject as notes, a theme or a seed"));
            return None;
        }
        if d > most {
            cx.err(Diagnostic::error(at, format!("`delay` is 1 to {most} beats for a subject of {k} bar{}: {} beats is the subject itself, in unison", if k == 1 { "" } else { "s" }, 4 * k)).help(if k == 1 { "delay=2 (half a bar behind)".to_string() } else { "delay=4 (a bar behind)".to_string() }));
            return None;
        }
        let head = format!("let {follow} = ");
        let text = follow_text(&s, k, d);
        if let Some(i) = doc.lines.iter().position(|l| l.starts_with(&head)) {
            doc.lines[i] = format!("{head}{text}");
            doc.fixed[i] = true;
        }
        let w = beats_words(d);
        for l in doc.lines.iter_mut().filter(|l| is_comment(l)) {
            for was in ["half a bar behind", "a bar behind"] {
                if l.contains(&format!("the copy {was}")) || l.contains(&format!("{was}, and a beat behind")) {
                    *l = l.replacen(&format!("the copy {was}"), &format!("the copy {w} behind"), 1).replacen(&format!("copies: {was}, and"), &format!("copies: {w} behind, and"), 1);
                    break;
                }
            }
        }
        cx.notes.push(Diagnostic::info(at, format!("delay: the chase's copy {w} behind the leader (`{follow}` = {text})")));
    }
    if let Some((n, at)) = num("interval") {
        let was = format!("octave({follow}, -1)");
        let now = interval_text(&follow, n);
        for (l, fixed) in doc.lines.iter_mut().zip(&doc.fixed) {
            if !is_comment(l) && !*fixed {
                *l = l.replace(&was, &now);
            }
        }
        let w = interval_words(n);
        for l in doc.lines.iter_mut().filter(|l| is_comment(l) && l.contains("an octave down")) {
            *l = l.replace("an octave down", &w);
        }
        cx.notes.push(Diagnostic::info(at, format!("interval: the chase's copy {w} (`{now}`)")));
    }
    Some(())
}
/// Sections taken from a program (a seed's, a stored one's): each played section once as `section PREFIXNAME from REF`, under
/// its doc comment, and the plays in its order. One section stands in for `section` itself.
fn borrowed(prefix: &str, single: &str, pick: &[(String, u32)], reference: &dyn Fn(&str) -> String, of: &str) -> Run {
    let mut text = Vec::new();
    let mut plays = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    if let [(n, bars)] = pick {
        // one section keeps the name its places in the arrangement already have
        text.push(format!("# {single}: the {n} of {of}"));
        text.push(format!("section {single} from {}", reference(n)));
        return Run { text, plays: Vec::new(), bars: *bars };
    }
    for (n, _) in pick {
        if !seen.contains(&n.as_str()) {
            seen.push(n);
            text.push(format!("# {prefix}{n}: the {n} of {of}"));
            text.push(format!("section {prefix}{n} from {}", reference(n)));
        }
        plays.push(vec![format!("{prefix}{n}")]);
    }
    Run { text, plays, bars: pick.iter().map(|p| p.1).sum() }
}

/// An inner form: written out with the outer phrase's slots, its vocal taken out (its sung section, the vocal's lets and
/// one-shots), every name it defines given `prefix`, its roles bound, its sections in its own key; its arrange block made plays.
fn inner_form(cx: &mut Ctx, f: &FormRef, prefix: &str, depth: usize) -> Option<Run> {
    if depth > MAX_NESTING {
        cx.err(Diagnostic::error(f.span, format!("forms nest {MAX_NESTING} deep at most, and this is the {}th", depth)).help("bind this role to a gen call, a stored program's sections or notes"));
        return None;
    }
    let known = crate::template::names();
    let Some(tpl) = crate::template::find(&f.name) else {
        let mut d = Diagnostic::error(f.name_span, format!("`{}` is not a template", f.name)).expected(&known.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{}`", f.name));
        d = match nearest(&f.name, known.iter().map(String::as_str)) {
            Some(n) => d.help(format!("did you mean `{n}`? The templates: {}", known.join(", "))),
            None => d.help(format!("the templates: {}", known.join(", "))),
        };
        cx.err(d);
        return None;
    };
    if tpl.form.is_none() {
        cx.err(Diagnostic::error(f.name_span, format!("`{}` is a groove, not a form: its voice is in every section, so it cannot be written inside another", f.name)).help(format!("the forms: {}", crate::template::forms().map(|(t, _)| t.name).collect::<Vec<_>>().join(", "))));
        return None;
    }
    let key = own_key(tpl);
    let text = crate::template::write_out(tpl, cx.slots, |_| false);
    let mut doc = Doc::new(&text);
    // the header: what it is goes in the outer form's comment
    while doc.lines.first().is_some_and(|l| is_comment(l) || l.trim().is_empty()) {
        doc.lines.remove(0);
        doc.fixed.remove(0);
    }
    // the vocal: its section, its lets, its one-shots
    let sung: Option<String> = doc.lines.iter().position(|l| l.trim_start().starts_with("deck B: sing ")).and_then(|i| (0..i).rev().find(|k| doc.lines[*k].starts_with("section "))).and_then(|k| doc.lines[k].split_whitespace().nth(1).map(str::to_string));
    if let Some(name) = &sung {
        if let Some((d, _, e)) = doc.section(name) {
            let e = if doc.lines.get(e + 1).is_some_and(|l| l.trim().is_empty()) { e + 1 } else { e };
            doc.lines.drain(d..=e);
            doc.fixed.drain(d..=e);
        }
    }
    let keep: Vec<bool> = doc
        .lines
        .iter()
        .map(|l| {
            let t = l.trim();
            !(t.starts_with("let vox_") || t.starts_with("# the vocal line:") || sung.as_deref() == Some(t) || ((t.starts_with("~ ") || t.starts_with("@ ")) && t.split_whitespace().nth(1).is_some_and(|w| w.starts_with("vox_"))))
        })
        .collect();
    let mut i = 0;
    doc.lines.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    doc.fixed.truncate(doc.lines.len());
    if !doc.lines.iter().any(|l| !l.starts_with("let shk ") && l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == "shk")) {
        if let Some(k) = doc.lines.iter().position(|l| l.starts_with("let shk ")) {
            doc.lines.remove(k);
            doc.fixed.remove(k);
        }
    }
    // every name it defines, prefixed
    let mut defined: Vec<String> = Vec::new();
    for l in &doc.lines {
        let mut w = l.split_whitespace();
        if let (Some("let" | "section"), Some(n)) = (w.next(), w.next()) {
            defined.push(n.to_string());
        }
    }
    doc.rename(&mut |w| {
        if defined.iter().any(|d| d == w) {
            return Some(format!("{prefix}{w}"));
        }
        let stem = w.strip_suffix('*')?;
        defined.iter().any(|d| d.starts_with(stem)).then(|| format!("{prefix}{w}"))
    });
    // and the doc comments of its sections say the new names
    for k in 0..doc.lines.len() {
        if let Some(n) = doc.lines[k].strip_prefix("section ").and_then(|r| r.split_whitespace().next()).map(str::to_string) {
            let bare = n.strip_prefix(prefix).unwrap_or(&n).to_string();
            if k > 0 && doc.lines[k - 1].starts_with(&format!("# {bare}")) {
                doc.lines[k - 1] = format!("# {n}{}", &doc.lines[k - 1][2 + bare.len()..]);
            }
        }
    }
    let text = bind_form(cx, tpl, &doc.text(), &f.roles, prefix, &key, depth)?;
    let mut doc = Doc::new(&text);
    // its sections in its own key, when the program's is another
    if cx.host.key.as_deref() != Some(key.as_str()) {
        let mut k = 0;
        while k < doc.lines.len() {
            if doc.lines[k].starts_with("section ") && doc.lines[k].trim_end().ends_with('{') && !doc.lines.get(k + 1).is_some_and(|l| l.trim_start().starts_with("key ")) {
                doc.lines.insert(k + 1, format!("  key {key}"));
                doc.fixed.insert(k + 1, true);
                k += 1;
            }
            k += 1;
        }
    }
    // its arrange block: the plays, in order
    let Some((s, e)) = doc.arrange() else {
        cx.err(Diagnostic::error(f.span, format!("the {} template has no arrange block (this is a bug in the template)", tpl.name)));
        return None;
    };
    let mut a = s;
    while a > 0 && is_comment(&doc.lines[a - 1]) {
        a -= 1;
    }
    let lines: Vec<String> = doc.lines[s + 1..e].to_vec();
    let mut bars: BTreeMap<String, u32> = BTreeMap::new();
    let mut names: Vec<String> = Vec::new();
    for l in &doc.lines {
        if let Some(r) = l.strip_prefix("section ") {
            let w: Vec<&str> = r.split_whitespace().collect();
            if let [n, b, "bars", ..] = w.as_slice() {
                bars.insert(n.to_string(), b.parse().unwrap_or(0));
            }
            if let Some(n) = w.first() {
                names.push(n.to_string());
            }
        }
    }
    let plays = plays_of(&lines, &names);
    let total: u32 = plays.iter().map(|p| bars.get(&p[0]).copied().unwrap_or(0)).sum();
    doc.lines.drain(a..=e);
    doc.fixed.drain(a..=e);
    let mut body = vec![format!("# the {} written out, its names starting {prefix}{}", tpl.name, if cx.host.key.as_deref() != Some(key.as_str()) { format!(", in its own key: each of its sections says `key {key}`") } else { String::new() })];
    body.extend(doc.lines.into_iter().skip_while(|l| l.trim().is_empty()));
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    Some(Run { text: body, plays, bars: total })
}
