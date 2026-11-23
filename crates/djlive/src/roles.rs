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
//!   after the role (`ep_ground`), and its mutation plan, spread over the places the role plays in.
//!
//! **Rules.** A source plays in the key of the sections it fills: the program's at the top (a `gen` call is fitted to it, a
//! theme moved into it), an inner form's own (its sections open with its `key`, its calls say it). Everything is pinned: the
//! same text is the same written-out program and the same nets. Composition stays total: forms nest [`MAX_NESTING`] deep, a
//! line is at most [`MAX_LINE_BARS`] bars, and a role that takes sections takes at most [`MAX_SECTION_BARS`] bars of them.
//!
//! Pure: no I/O and no clock (a `gen` call is the installed generator's, a pure function; a stored program is the host's text).

use std::collections::BTreeMap;

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

const BINDING: &str = "ROLE=SOURCE after the phrase: template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass); a source is a gen call, a theme id (ode_to_joy), notes ([d2 ~ a1 ~]), a stored program's <cid>.drop.bass, or template(NAME, ROLE=SOURCE)";

// ---------------------------------------------------------------- the reference

/// A role of a template bound to a source: `ground=gen@1 dubstep seed=54876 part=bass`.
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
    /// A stored program by CID, a section of it, a part of it.
    Stored { cid: String, section: Option<String>, part: Option<String> },
    /// A theme of the public-domain library, by id.
    Theme(String),
    /// Notes, a bar each, canonical.
    Notes(Vec<String>),
    /// Another template's output.
    Form(FormRef),
}

/// `template(NAME, ROLE=SOURCE ...)` where a source goes: a form with no phrase of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct FormRef {
    pub name: String,
    pub name_span: Span,
    pub roles: Vec<Binding>,
    pub span: Span,
}

impl Source {
    /// The canonical spelling.
    pub fn text(&self) -> String {
        match self {
            Source::Gen(c) => c.text(),
            Source::Stored { cid, section, part } => {
                let mut s = cid.clone();
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
        }
    }
}

impl FormRef {
    /// The canonical spelling: `template(passacaglia, ground=gen@1 dubstep seed=54876 section=drop part=bass)`.
    pub fn text(&self) -> String {
        format!("template({}{})", self.name, bindings_text(&self.name, &self.roles))
    }
}

/// `, role=source` for each binding, in the order the form names its roles (one it does not name after them, as written).
pub fn bindings_text(form: &str, roles: &[Binding]) -> String {
    let order = specs(form);
    let rank = |b: &Binding| order.iter().position(|s| s.name == b.role).unwrap_or(usize::MAX);
    let mut v: Vec<&Binding> = roles.iter().collect();
    v.sort_by_key(|b| rank(b));
    v.iter().map(|b| format!(", {}={}", b.role, b.source.text())).collect()
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
            let (Some(Tok::Word(role)), Some(Tok::Eq)) = (self.at(j).cloned(), self.at(j + 1)) else {
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
            let role_span = self.span(j);
            j += 2;
            let start = j;
            let (end, source) = self.source(j, depth)?;
            let source_span = self.span(start).to(self.span(end.max(start + 1) - 1));
            out.push(Binding { role, role_span, source, source_span });
            j = end;
        }
        Ok((j, out))
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
                let (end, f) = self.form(j, depth + 1)?;
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
                    _ => return Err(Diagnostic::error(here.to(self.span(k - 1)), "a stored program is a CID, a section and a part at most").help("<cid>.drop.bass")),
                };
                Ok((k, Source::Stored { cid: w.clone(), section, part }))
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
        Ok((k + 1, FormRef { name, name_span, roles, span }))
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
}

const fn line(name: &'static str, what: &'static str, default: &'static str, id: &'static str, let_name: &'static str, part: &'static str, cycle: &'static str, flex: bool, above: &'static str) -> RoleSpec {
    RoleSpec { name, what, default, id, let_name, part, cycle, flex, above, section: None, prefix: "", parts: &[] }
}

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
    },
];

/// The roles of the form `name`, in the order the canonical spelling binds them (none: a template that takes no roles yet).
pub fn specs(name: &str) -> &'static [RoleSpec] {
    match name {
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
}

/// A line a binding writes: the let lines, how many bars, its cycle, and what to say of it.
struct LineOut {
    lets: Vec<String>,
    bars: usize,
    cycle: String,
    note: String,
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
        self.comp.segments.iter().find(|g| g.header.is_some() && !g.chain && g.name == s).is_some_and(|g| {
            let hits: Vec<&crate::compile::PartIr> = g.decks.values().flat_map(|d| d.parts.iter()).filter(|p| p.role == part).collect();
            matches!(hits.as_slice(), [p] if p.cycle_steps == 16)
        })
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
        let missing = || Diagnostic::error(at, crate::env::not_here(cid)).help(crate::env::NOT_HERE_HELP);
        let text = match crate::gen::generated(cid) {
            Some(t) => t,
            None => match self.env.snippets.map(|s| s.resolve(cid)) {
                Some(Ok((got, text))) if got == cid => text,
                Some(Err(e)) if e.contains("not the program it names") => {
                    self.err(Diagnostic::error(at, e));
                    return None;
                }
                _ => {
                    self.err(missing());
                    return None;
                }
            },
        };
        if crate::compile::program_cid(&text) != cid {
            self.err(Diagnostic::error(at, format!("the program {} is not the program it names: refused", djengine::rights::short_cid(cid))));
            return None;
        }
        let plain = Env { registry: None, catalog: None, snippets: self.env.snippets, assets: None, private: false, voices: None, ontology: None, bank: None };
        match crate::compile::check_with(&text, &plain).compiled {
            Some(c) => Some(c),
            None => {
                self.err(Diagnostic::error(at, format!("the program {} does not compile", djengine::rights::short_cid(cid))).help("it can be borrowed from once it checks"));
                None
            }
        }
    }

    /// A gen call made to play in `key` (the sections it fills): it says the key itself when that is not the program's.
    fn keyed(&self, c: &GenCall, key: &str) -> GenCall {
        let mut c = c.clone();
        if c.key.is_none() && self.host.key.as_deref() != Some(key) {
            c.key = Some(key.to_string());
        }
        c
    }

    /// The line `b` binds role `spec` (or a seed role's `part`, written as `name`) to, in `key`.
    fn line(&mut self, b: &Binding, name: &str, part: &str, flex: bool, key: &str) -> Option<LineOut> {
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
                Some(LineOut { lets: vec![format!("let {name} = {call}")], bars, cycle: String::new(), note: format!("{call}: {bars} bar{}{fitted}", if bars == 1 { "" } else { "s" }) })
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
                Some(LineOut { lets, bars: t.bars.len(), cycle: t.cycle_mark(), note: format!("{id} ({}, {}, {}): {} bar{}, {moved}", t.work, t.composer, t.year, t.bars.len(), if t.bars.len() == 1 { "" } else { "s" }) })
            }
            Source::Notes(bars) => {
                if bars.len() > MAX_LINE_BARS {
                    self.err(Diagnostic::error(at, format!("these notes are {} bars: a line is at most {MAX_LINE_BARS}", bars.len())).help("bind fewer bars: a line loops"));
                    return None;
                }
                let mut lets: Vec<String> = bars.iter().enumerate().map(|(i, bar)| format!("let {name}_{} = {bar}", i + 1)).collect();
                lets.push(format!("let {name} = <{}>", (1..=bars.len()).map(|i| format!("{name}_{i}")).collect::<Vec<_>>().join(" ")));
                Some(LineOut { lets, bars: bars.len(), cycle: String::new(), note: format!("notes: {} bar{}", bars.len(), if bars.len() == 1 { "" } else { "s" }) })
            }
            Source::Stored { cid, section, part: p } => {
                let Some(p) = p else {
                    self.err(Diagnostic::error(at, format!("`{}` takes a line: one part of a stored program", b.role)).help(format!("{}={}.bass", b.role, b.source.text())));
                    return None;
                };
                if let Some(comp) = self.stored(cid, at) {
                    let plays = sequence(&comp);
                    if let Some(s) = section {
                        if !plays.iter().any(|(n, _)| n == s) {
                            let names: Vec<String> = plays.iter().map(|(n, _)| n.clone()).collect();
                            self.err(Diagnostic::error(at, format!("{} has no section `{s}`", djengine::rights::short_cid(cid))).help(format!("its sections: {}", names.join(", "))));
                            return None;
                        }
                    }
                }
                let r = Source::Stored { cid: cid.clone(), section: section.clone(), part: Some(p.clone()) }.text();
                Some(LineOut { lets: vec![format!("let {name}_1 = {r}"), format!("let {name} = <{name}_1>")], bars: 1, cycle: String::new(), note: format!("the {p} of {}: one let", djengine::rights::short_cid(cid)) })
            }
            Source::Form(f) => {
                self.err(Diagnostic::error(f.span, format!("`{}` takes a line, and a form is sections", b.role)).help("bind it to a gen call's part, a theme, notes or a stored program's part"));
                None
            }
        }
    }
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

/// The header line a bound form says its roles with (the fourth comment of a template: the one that names its themes).
fn header(specs: &[RoleSpec], bound: &BTreeMap<&str, String>) -> Vec<String> {
    specs.iter().map(|s| format!("# {}: {}; {}", s.name, bound.get(s.name).cloned().unwrap_or_else(|| s.default.replace('`', "")), s.what)).collect()
}

/// The form `tpl` written out (`text`: its default, as unbound) with `roles` bound. An inner form (`depth` > 1) has every name
/// it defines start with `prefix` and its sections in `key`; the top one says its roles in its header.
pub(crate) fn bind_form(cx: &mut Ctx, tpl: &'static Template, text: &str, roles: &[Binding], prefix: &str, key: &str, depth: usize) -> Option<String> {
    let specs = specs(tpl.name);
    let errs = cx.errors.len();
    if specs.is_empty() && !roles.is_empty() {
        let with = forms_with_roles();
        cx.err(Diagnostic::error(roles[0].role_span, format!("`{}` takes no roles yet: the templates with roles are {}", tpl.name, with.join(", "))));
        return None;
    }
    // each role once, and only the form's
    let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
    for (i, b) in roles.iter().enumerate() {
        if !names.contains(&b.role.as_str()) {
            let mut d = Diagnostic::error(b.role_span, format!("`{}` is no role of the {}", b.role, tpl.name)).expected(&names).found(format!("`{}`", b.role));
            d = match nearest(&b.role, names.iter().copied()) {
                Some(n) => d.help(format!("did you mean `{n}`? The {}'s roles: {}", tpl.name, names.join(", "))),
                None => d.help(format!("the {}'s roles: {}", tpl.name, names.join(", "))),
            };
            cx.err(d);
        } else if roles[..i].iter().any(|x| x.role == b.role) {
            cx.err(Diagnostic::error(b.role_span, format!("`{}` is bound twice", b.role)).help("bind each role once"));
        }
    }
    if cx.errors.len() > errs {
        return None;
    }
    let mut doc = Doc::new(text);
    let mut bound: BTreeMap<&str, String> = BTreeMap::new();
    let mut lens: BTreeMap<&str, usize> = BTreeMap::new();
    for spec in specs {
        let Some(b) = roles.iter().find(|b| b.role == spec.name) else { continue };
        let src = b.source.text();
        // a seed role: a line for each of its parts
        if !spec.parts.is_empty() {
            let mut outs: Vec<(usize, LineOut)> = Vec::new();
            match &b.source {
                Source::Gen(c) if c.part.is_none() => {
                    let c = cx.keyed(c, key);
                    let section = match &c.section {
                        Some(s) => Some(s.clone()),
                        None => {
                            let g = cx.generated(&c, b.source_span)?;
                            let all: Vec<&str> = spec.parts.iter().map(|p| p.0).collect();
                            g.section_for(&all)
                        }
                    };
                    for (k, (part, _, let_name)) in spec.parts.iter().enumerate() {
                        let call = GenCall { section: section.clone(), part: Some(part.to_string()), ..c.clone() };
                        let one = Binding { source: Source::Gen(call), ..b.clone() };
                        outs.push((k, cx.line(&one, &format!("{prefix}{let_name}"), part, false, key)?));
                    }
                }
                Source::Gen(c) => {
                    let part = c.part.clone().unwrap_or_default();
                    let Some(k) = spec.parts.iter().position(|p| p.0 == part) else {
                        let ps: Vec<&str> = spec.parts.iter().map(|p| p.0).collect();
                        cx.err(Diagnostic::error(b.source_span, format!("`{}` takes a seed's {}: not its {part}", b.role, ps.join(", "))).help(format!("{}={}", b.role, c.program_text())));
                        return None;
                    };
                    let (p, _, let_name) = spec.parts[k];
                    outs.push((k, cx.line(b, &format!("{prefix}{let_name}"), p, false, key)?));
                }
                _ => {
                    // a line: the part of a stored program it is, else the melody
                    let k = match &b.source {
                        Source::Stored { part: Some(p), .. } => spec.parts.iter().position(|x| x.0 == p).unwrap_or(0),
                        _ => 0,
                    };
                    let (p, _, let_name) = spec.parts[k];
                    outs.push((k, cx.line(b, &format!("{prefix}{let_name}"), p, false, key)?));
                }
            }
            let mut said = Vec::new();
            for (k, out) in &outs {
                let (part, id, let_name) = spec.parts[*k];
                let comment = format!("# {}'s {part}, bound: {src}", spec.name);
                if !apply_line(&mut doc, &format!("{prefix}{id}"), &format!("{prefix}{let_name}"), "", "", comment, out) {
                    cx.err(Diagnostic::error(b.source_span, format!("the {} template has no `{id}` to bind (this is a bug in the template)", tpl.name)));
                    return None;
                }
                said.push(format!("its {part} {}", out.note));
            }
            cx.notes.push(Diagnostic::info(b.source_span, format!("{}: {}", spec.name, said.join("; "))));
            bound.insert(spec.name, src);
            continue;
        }
        // a role that takes sections, bound to sections
        let sections_source = match &b.source {
            Source::Form(_) => true,
            Source::Gen(c) => c.part.is_none() && (c.section.is_some() || spec.section.is_some()),
            Source::Stored { part, .. } => part.is_none(),
            _ => false,
        };
        if let (Some(section), true) = (spec.section, sections_source) {
            let p = format!("{prefix}{}", spec.prefix);
            let run = match &b.source {
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
                Source::Stored { cid, section: s, .. } => {
                    let comp = cx.stored(cid, b.source_span)?;
                    let plays = sequence(&comp);
                    let pick: Vec<(String, u32)> = match s {
                        Some(s) => match plays.iter().find(|(n, _)| n == s) {
                            Some(x) => vec![x.clone()],
                            None => {
                                let names: Vec<String> = plays.iter().map(|(n, _)| n.clone()).collect();
                                cx.err(Diagnostic::error(b.source_span, format!("{} has no section `{s}`", djengine::rights::short_cid(cid))).help(format!("its sections: {}", names.join(", "))));
                                return None;
                            }
                        },
                        None => plays,
                    };
                    if pick.is_empty() {
                        cx.err(Diagnostic::error(b.source_span, format!("{} has no sections to take", djengine::rights::short_cid(cid))).help(format!("bind one of its parts: {}={cid}.bass", b.role)));
                        return None;
                    }
                    borrowed(&p, &format!("{prefix}{section}"), &pick, &|n| format!("{cid}.{n}"), &djengine::rights::short_cid(cid))
                }
                _ => unreachable!("a sections source"),
            };
            if run.bars > MAX_SECTION_BARS {
                cx.err(Diagnostic::error(b.source_span, format!("this is {} bars of sections a pass: `{}` takes at most {MAX_SECTION_BARS}", run.bars, b.role)).help("bind a section of it, or a shorter one (length=standard)"));
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
            cx.notes.push(Diagnostic::info(b.source_span, format!("{}: {} section{}, {} bars a pass", spec.name, run.plays.len(), if run.plays.len() == 1 { "" } else { "s" }, run.bars)));
            bound.insert(spec.name, src);
            continue;
        }
        // a line
        let name = format!("{prefix}{}", spec.let_name);
        let out = cx.line(b, &name, spec.part, spec.flex, key)?;
        let comment = format!("# {}, bound: {src}", spec.name);
        if !apply_line(&mut doc, &format!("{prefix}{}", spec.id), &name, spec.cycle, spec.above, comment, &out) {
            cx.err(Diagnostic::error(b.source_span, format!("the {} template has no `{}` to bind (this is a bug in the template)", tpl.name, spec.id)));
            return None;
        }
        cx.notes.push(Diagnostic::info(b.source_span, format!("{}: {} (`{name}_*`)", spec.name, out.note)));
        lens.insert(spec.name, out.bars);
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
    }
    if depth == 1 {
        // the header says what fills each role: the fourth comment line, the one that named the themes
        if doc.lines.len() > 3 && is_comment(&doc.lines[3]) && !doc.lines[3].starts_with("# phrase:") && !doc.lines[3].starts_with("# vocal:") {
            doc.splice(3..4, header(specs, &bound));
        }
    }
    if cx.errors.len() > errs {
        return None;
    }
    Some(doc.text())
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
