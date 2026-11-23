//! `use SOURCE as NAME`: a name for a source, written out where it is used (docs/BEATSCRIPT.md "Name a source: `use`", long note
//! docs/beatscript/ext-roles.md "Name a source").
//!
//! ```text
//! use z4EBG9j9….drop as hook                              a stored program's section: `hook` is a section (arrange intro hook)
//! use z4EBG9j9….drop.bass as riff                         one part of it: `riff` is a line, a let (deck A: bass riff)
//! use gen@2 house seed=7 section=drop part=bass as walk   a seed's part: a line, a let a bar (`~ walk_* rot 2`)
//! use template(passacaglia).ground as ground              a canned section: a section of a form template (templates v1)
//! use genre@1 techno.drop as drop                         a section of a genre template (`genre_template`'s, frozen)
//! use theme ode_to_joy as tune                            a theme of the public-domain library: a line, in the program's key
//! use phrase @tyger_burning_bright as tyger               a phrasebook line: a phrase (sing tyger on melody)
//! use template(passacaglia, ground=walk) as episode_form  a form: a source for a role (episodes=episode_form)
//! ```
//!
//! **What NAME is** follows from its source ([`Makes`]): a section (a stored or generated program's section, a canned section,
//! a genre template's section), a line (a part of one, a theme), a source for a role (a form, a whole stored program, a whole
//! seed) or a phrase. The `use` line is written out in place, first of all (templates, seeds, `gen` calls and stored programs
//! come after it: compile.rs), as the definition a person would have typed there: `section hook from <cid>.drop`, `let riff =
//! <cid>.drop.bass`, a theme's lets; the stages after it write those out in turn. In a role binding (`ground=walk`) NAME stands
//! for its source itself, which the role reads as it reads any source (a theme moved into the role's key, a form nested); in
//! a phrase's place (`sing tyger on melody`, `template(deep-house, tyger)`) it stands for the phrase's `@id`.
//!
//! **Pinned and total.** A CID names its text; a `gen@N` call its version; a canned section is templates v1 (and the form's own
//! example phrase when it says none); a genre template is `genre@N`, a frozen file (`djengine::genre_templates`); a theme and a
//! phrase are their libraries' (each file's CID). What is in the repo resolves offline; a CID goes through the host's snippets
//! as every stored reference does. A name is used after its `use` only, so no `use` reaches itself, and what it writes is
//! bounded by the stages it hands on to.
//!
//! Pure: no I/O and no clock.

use std::cell::RefCell;

use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::gen::GenCall;
use crate::lex::{lex, normalise_input, Tok, Token};
use crate::parse::{nearest, ROLES};
use crate::roles::FormRef;
use crate::seed::Expansion;
use crate::template::PhraseRef;

/// The kinds of source, each with how it is written and what it names: one more kind is one more arm of [`Source`].
pub const KINDS: [(&str, &str, &str); 7] = [
    ("stored", "<cid>.drop  <cid>.drop.bass  <cid>", "a stored program's section (a section), part (a line) or whole (a source for a role)"),
    ("gen", "gen@2 techno seed=42 section=drop [part=bass]", "a seed's section (a section), part (a line) or whole (a source for a role)"),
    ("template", "template(passacaglia).ground  template(deep-house, @tempest_such_stuff).night", "a canned section: a section of a template, written out (a section)"),
    ("form", "template(passacaglia, ground=...)", "a form, its roles bound or not (a source for a role)"),
    ("theme", "theme ode_to_joy", "a theme of the public-domain library, moved into the program's key (a line)"),
    ("phrase", "phrase @tempest_such_stuff", "a phrasebook line (a phrase, for sing and template)"),
    ("genre", "genre@1 techno.drop", "a section of a genre template, the genre_template starter frozen (a section)"),
];

/// A bare `genre` is this version, and the canonical text writes it.
pub const GENRE_BARE: u32 = 1;

const SHAPE: &str = "use SOURCE as NAME, a source being <cid>.drop (a section), <cid>.drop.bass (a line), gen@2 techno seed=42 section=drop [part=bass], template(passacaglia).ground (a canned section), template(passacaglia, ground=...) (a form, for a role), theme ode_to_joy, phrase @tempest_such_stuff or genre@1 techno.drop";

// ---------------------------------------------------------------- the source

/// What NAME is, by its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Makes {
    /// A section: `arrange` places it.
    Section,
    /// A line, a let: a part plays it, a mutation changes it.
    Line,
    /// A source for a role (`episodes=NAME`): a form, a whole program, a whole seed.
    Role,
    /// A phrase: `sing NAME on melody`, `template(deep-house, NAME)`.
    Phrase,
}

impl Makes {
    /// As the answers and the notes say it.
    pub fn name(self) -> &'static str {
        match self {
            Makes::Section => "section",
            Makes::Line => "line",
            Makes::Role => "source",
            Makes::Phrase => "phrase",
        }
    }
}

/// A `use` line's source, as read.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Stored { cid: String, section: Option<String>, part: Option<String> },
    Gen(GenCall),
    Form(FormRef),
    /// A section of a template's written-out program: its name, its phrase (None: the template's own example's), its roles.
    Canned { tpl: String, phrase: Option<PhraseRef>, roles: Vec<crate::roles::Binding>, section: String },
    Theme(String),
    Phrase(String),
    Genre { version: u32, genre: String, section: String },
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

impl Source {
    /// The canonical spelling: what the canonical text writes after `use`.
    pub fn text(&self) -> String {
        match self {
            Source::Stored { cid, section, part } => crate::roles::Source::Stored { cid: cid.clone(), section: section.clone(), part: part.clone() }.text(),
            Source::Gen(c) => c.text(),
            Source::Form(f) => f.text(),
            Source::Canned { tpl, phrase, roles, section } => {
                let b = crate::roles::bindings_text(tpl, roles);
                match phrase {
                    Some(PhraseRef::Id(id)) => format!("template({tpl}, @{id}{b}).{section}"),
                    Some(PhraseRef::Text(t)) => format!("template({tpl}, {}{b}).{section}", quote(t)),
                    Some(PhraseRef::Name(n)) => format!("template({tpl}, {n}{b}).{section}"),
                    None => format!("template({tpl}{b}).{section}"),
                }
            }
            Source::Theme(id) => format!("theme {id}"),
            Source::Phrase(id) => format!("phrase @{id}"),
            Source::Genre { version, genre, section } => format!("genre@{version} {genre}.{section}"),
        }
    }

    /// Its kind: one of [`KINDS`].
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Stored { .. } => "stored",
            Source::Gen(_) => "gen",
            Source::Form(_) => "form",
            Source::Canned { .. } => "template",
            Source::Theme(_) => "theme",
            Source::Phrase(_) => "phrase",
            Source::Genre { .. } => "genre",
        }
    }

    /// What it makes NAME.
    pub fn makes(&self) -> Makes {
        match self {
            Source::Stored { part: Some(_), .. } => Makes::Line,
            Source::Stored { section: Some(_), .. } => Makes::Section,
            Source::Stored { .. } => Makes::Role,
            Source::Gen(c) if c.part.is_some() => Makes::Line,
            Source::Gen(c) if c.section.is_some() => Makes::Section,
            Source::Gen(_) | Source::Form(_) => Makes::Role,
            Source::Canned { .. } | Source::Genre { .. } => Makes::Section,
            Source::Theme(_) => Makes::Line,
            Source::Phrase(_) => Makes::Phrase,
        }
    }

    /// A `gen` call is the engine's; every other source is structure (docs/BEATSCRIPT.md "Two halves").
    pub fn half(&self) -> &'static str {
        match self {
            Source::Gen(_) => crate::refs::ENGINE,
            _ => crate::refs::STRUCTURE,
        }
    }
}

/// A `use` line, read: its source, where it is, the name it gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    pub source: Source,
    pub source_span: Span,
    pub name: String,
    pub name_span: Span,
    /// The token the `as` is.
    as_at: usize,
}

/// A `use` line in a token stream (a stream whose `gen` calls are one word: gen::collapse): tokens `start..end`, its line's
/// own, and how deep in braces it stands. `Err(None)`: what is wrong was said already (a `gen` call that does not read).
pub(crate) struct Run {
    pub start: usize,
    pub end: usize,
    pub span: Span,
    pub depth: i32,
    pub r: Result<Decl, Option<Diagnostic>>,
}

fn touches(a: &Token, b: &Token) -> bool {
    a.span.end == b.span.start
}

/// Every `use` line that is not `use restricted`, read.
pub(crate) fn runs(toks: &[Token]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for i in 0..toks.len() {
        match toks[i].tok {
            Tok::LBrace => depth += 1,
            Tok::RBrace => depth -= 1,
            _ => {}
        }
        // (a line's first word, or a block's on its opening line: `section s 4 bars { use ... }`)
        let head = i == 0 || matches!(toks[i - 1].tok, Tok::Newline | Tok::LBrace);
        if !head || !matches!(&toks[i].tok, Tok::Word(w) if w == "use") {
            continue;
        }
        let restricted = matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Word(w)) if w == "restricted") && matches!(toks.get(i + 2).map(|t| &t.tok), None | Some(Tok::Newline | Tok::Eof | Tok::RBrace));
        if restricted {
            continue;
        }
        let (end, r) = read(toks, i);
        let last = end.max(i + 1) - 1;
        out.push(Run { start: i, end, span: toks[i].span.to(toks[last.min(toks.len() - 1)].span), depth, r });
    }
    out
}

/// Read the `use` line at `toks[i]`: where its line ends, and the declaration or what is wrong with it.
fn read(toks: &[Token], i: usize) -> (usize, Result<Decl, Option<Diagnostic>>) {
    let end = (i..toks.len()).find(|k| matches!(toks[*k].tok, Tok::Newline | Tok::Eof | Tok::RBrace)).unwrap_or(toks.len());
    let at = |k: usize| if k < end { toks.get(k).map(|t| &t.tok) } else { None };
    let sp = |k: usize| toks.get(k.min(end.saturating_sub(1)).max(i)).map_or(Span::default(), |t| t.span);
    let err = |d: Diagnostic| (end, Err(Some(d)));
    let (k, source) = match read_source(toks, i + 1, end) {
        Ok(x) => x,
        Err(e) => return (end, Err(e)),
    };
    let source_span = sp(i + 1).to(sp(k.max(i + 2) - 1));
    let st = source.text();
    match at(k) {
        Some(Tok::Word(w)) if w == "as" => {}
        None => return err(Diagnostic::error(toks[i].span.to(sp(k.max(i + 2) - 1)), format!("`use {st}` gives its source a name: say `as` and the name")).help(format!("use {st} as NAME"))),
        Some(t) => return err(Diagnostic::error(sp(k), format!("expected `as NAME` after the source, found {}", t.describe())).help(format!("use {st} as NAME"))),
    }
    let Some(Tok::Word(name)) = at(k + 1).cloned() else {
        return err(Diagnostic::error(sp(k), "`as` takes the name the source is given").help(format!("use {st} as NAME")));
    };
    let name_span = sp(k + 1);
    if !crate::roles::is_name(&name) {
        return err(Diagnostic::error(name_span, format!("`{name}` is no name for a source: a name is lower case letters, digits and `_`, a letter first")).help(format!("use {st} as my_{}", name.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_"))));
    }
    if let Some(why) = crate::parse::reserved(&name) {
        return err(Diagnostic::error(name_span, format!("{why}: it cannot name a source")).help(format!("use {st} as my_{name}")));
    }
    if k + 2 < end {
        return err(Diagnostic::error(sp(k + 2).to(sp(end - 1)), "a `use` line ends after its name").help(format!("use {st} as {name}")));
    }
    (end, Ok(Decl { source, source_span, name, name_span, as_at: k }))
}

/// The source at `toks[j]` (before `end`, the line's end): where it ends, and the source.
fn read_source(toks: &[Token], j: usize, end: usize) -> Result<(usize, Source), Option<Diagnostic>> {
    let at = |k: usize| if k < end { toks.get(k).map(|t| &t.tok) } else { None };
    let sp = |k: usize| toks.get(k.min(end.saturating_sub(1))).map_or(Span::default(), |t| t.span);
    let glued = |k: usize| k > 0 && k < end && touches(&toks[k - 1], &toks[k]);
    let e = |at: Span, msg: String, help: String| Err(Some(Diagnostic::error(at, msg).help(help)));
    // `.NAME` glued after `toks[k - 1]`: a section's name
    let section_after = |k: usize| -> Option<(String, usize)> {
        if at(k) == Some(&Tok::Dot) && glued(k) && glued(k + 1) {
            if let Some(Tok::Word(s)) = at(k + 1) {
                return Some((s.clone(), k + 2));
            }
        }
        None
    };
    match at(j).cloned() {
        None => e(sp(j.saturating_sub(1)), "`use` names a source and gives it a name".into(), SHAPE.into()),
        Some(Tok::Word(w)) if w == "gen@0 error" => Err(None),
        Some(Tok::Word(w)) if crate::gen::is_call_word(&w) => crate::roles::word_call(&w, sp(j)).map(|c| (j + 1, Source::Gen(c))).map_err(Some),
        Some(Tok::Word(w)) if crate::stored::is_cid(&w) => {
            let mut names: Vec<(String, Span)> = Vec::new();
            let mut k = j + 1;
            while at(k) == Some(&Tok::Dot) && glued(k) && glued(k + 1) {
                let Some(Tok::Word(n)) = at(k + 1) else { break };
                names.push((n.clone(), sp(k + 1)));
                k += 2;
            }
            let is_role = |n: &str| ROLES.contains(&n);
            let (section, part) = match names.as_slice() {
                [] => (None, None),
                [(n, _)] if is_role(n) => (None, Some(n.clone())),
                [(n, _)] => (Some(n.clone()), None),
                [(a, sa), (b, sb)] => {
                    if is_role(a) {
                        return e(*sa, format!("`.{a}.{b}`: a section comes before a part"), format!("{w}.{b}.{a}"));
                    }
                    if !is_role(b) {
                        let help = nearest(b, ROLES).map_or(format!("the parts are the roles: {}", ROLES.join(", ")), |n| format!("did you mean `.{n}`?"));
                        return Err(Some(Diagnostic::error(*sb, format!("`.{b}`: a part of a stored program is one of its roles")).expected(&ROLES).found(format!("`{b}`")).help(help)));
                    }
                    (Some(a.clone()), Some(b.clone()))
                }
                _ => return e(sp(j).to(sp(k - 1)), "a stored program is a CID, a section and a part at most".into(), "<cid>.drop.bass".into()),
            };
            Ok((k, Source::Stored { cid: w, section, part }))
        }
        Some(Tok::Word(w)) if w == "template" && at(j + 1) == Some(&Tok::LParen) && glued(j + 1) => {
            // a phrase as the second argument (an id, words in quotes, or a name that is no role's: no `=` after it) is a template
            // that makes a whole track, of which `use` takes a section; with none it is a form
            let phrase = matches!(at(j + 2), Some(Tok::Word(_)))
                && at(j + 3) == Some(&Tok::Comma)
                && match at(j + 4) {
                    Some(Tok::Str(_)) => true,
                    Some(Tok::Word(p)) => p.starts_with('@') || (crate::roles::is_name(p) && at(j + 5) != Some(&Tok::Eq)),
                    _ => false,
                };
            if phrase {
                let (k, r) = crate::template::read_ref(toks, j);
                let t = r.map_err(Some)?;
                match section_after(k) {
                    Some((section, k2)) => Ok((k2, Source::Canned { tpl: t.name, phrase: Some(t.phrase), roles: t.roles, section })),
                    None => e(
                        sp(j).to(sp(k.max(j + 1) - 1)),
                        "a template with a phrase makes a whole track: `use` takes one of its sections".into(),
                        format!("{}.<section>, as template(passacaglia).ground; a form for a role takes no phrase: template(passacaglia, ground=...)", t.text()),
                    ),
                }
            } else {
                let (k, f) = crate::roles::Reader::new(&toks[..end]).form(j, 1).map_err(Some)?;
                Ok(match section_after(k) {
                    Some((section, k2)) => (k2, Source::Canned { tpl: f.name, phrase: None, roles: f.roles, section }),
                    None => (k, Source::Form(f)),
                })
            }
        }
        Some(Tok::Word(w)) if w == "theme" => match at(j + 1) {
            Some(Tok::Word(id)) if crate::roles::is_name(id) => Ok((j + 2, Source::Theme(id.clone()))),
            _ => e(sp(j), "`theme` takes a theme of the public-domain library by its id".into(), "use theme ode_to_joy as tune (list_examples with themes, or /themes, lists them)".into()),
        },
        Some(Tok::Word(w)) if w == "phrase" => match at(j + 1).cloned() {
            Some(Tok::Word(p)) if p.len() > 1 && p.starts_with('@') => {
                let id = &p[1..];
                if djengine::vocal::phoneme::phrase_lyric(id).is_none() {
                    let ids = djengine::vocal::phoneme::phrase_ids();
                    let help = match crate::env::suggest(id, &ids, 3).first() {
                        Some(n) => format!("did you mean `@{n}`?"),
                        None => "list_examples with phrasebook lists the lines (docs/beatscript/found-phrases.md)".to_string(),
                    };
                    return Err(Some(Diagnostic::error(sp(j + 1), format!("`{p}` is not a phrase of the phrasebook")).found(format!("`{p}`")).help(help)));
                }
                Ok((j + 2, Source::Phrase(id.to_string())))
            }
            Some(Tok::Word(p)) => e(sp(j + 1), format!("a phrase is `@` and its id: phrase @{p}"), format!("use phrase @{p} as NAME")),
            _ => e(sp(j), "`phrase` takes a phrasebook line by its id".into(), "use phrase @tempest_such_stuff as line".into()),
        },
        Some(Tok::Word(w)) if w == "genre" || w.starts_with("genre@") => {
            let version = match w.strip_prefix("genre@") {
                None => GENRE_BARE,
                Some(n) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => n.parse().unwrap_or(u32::MAX),
                Some(_) => return e(sp(j), format!("`{w}` is no version of the genre templates"), "genre@1 techno.drop".into()),
            };
            match (at(j + 1).cloned(), section_after(j + 2)) {
                (Some(Tok::Word(g)), Some((section, k))) if glued(j + 2) => Ok((k, Source::Genre { version, genre: g, section })),
                _ => e(sp(j).to(sp(j + 1)), "`genre` takes a genre and one of its template's sections, a `.` between: genre@1 techno.drop".into(), "use genre@1 techno.drop as drop (genre_template shows a genre's sections)".into()),
            }
        }
        Some(Tok::Word(w)) if w == "seed" || w.starts_with("seed(") => {
            e(sp(j), "`seed(...)` is no source for `use`: write it where it is used, or name a seed of the generator".into(), "use gen@2 techno seed=42 section=drop as drop".into())
        }
        Some(Tok::LBrack | Tok::Lt) => e(sp(j), "notes need no `use`: a `let` names them".into(), "let riff = d2 ~ a1 ~".into()),
        Some(Tok::Word(w)) if w.starts_with('@') => e(sp(j), "a phrasebook line is named with `phrase`".into(), format!("use phrase {w} as NAME")),
        Some(Tok::Word(w)) => {
            let help = if djengine::themes::theme(&w).is_some() {
                format!("did you mean `use theme {w} as NAME`? ({w} is a theme)")
            } else if let Some(n) = nearest(&w, ["theme", "phrase", "genre", "template", "gen"]) {
                format!("did you mean `{n}`? {SHAPE}")
            } else {
                SHAPE.to_string()
            };
            Err(Some(Diagnostic::error(sp(j), format!("`{w}` is no source for `use`")).found(format!("`{w}`")).help(help)))
        }
        Some(t) => e(sp(j), format!("expected a source after `use`, found {}", t.describe()), SHAPE.into()),
    }
}

/// Each `use` line's source made one word (its canonical spelling) for the parser, what is wrong with any of them (that line's
/// tokens are left out, so one mistake is one error), and the phrases the lines name: (name, phrasebook id).
pub(crate) fn collapse(toks: Vec<Token>) -> (Vec<Token>, Vec<Diagnostic>, Vec<(String, String)>) {
    let rs = runs(&toks);
    if rs.is_empty() {
        return (toks, Vec::new(), Vec::new());
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut diags = Vec::new();
    let mut phrases = Vec::new();
    let mut i = 0;
    for r in rs {
        out.extend_from_slice(&toks[i..r.start]);
        match r.r {
            Ok(d) => {
                out.push(toks[r.start].clone());
                out.push(Token { tok: Tok::Word(d.source.text()), span: d.source_span });
                out.extend_from_slice(&toks[d.as_at..r.end]);
                if let Source::Phrase(id) = &d.source {
                    phrases.push((d.name.clone(), id.clone()));
                }
            }
            Err(Some(e)) => diags.push(e),
            Err(None) => {}
        }
        i = r.end;
    }
    out.extend_from_slice(&toks[i..]);
    (out, diags, phrases)
}

thread_local! {
    /// The phrases the `use phrase` lines of the program being parsed on this thread name: what `sing NAME on ...` sings.
    static PHRASES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with `phrases` named (the parser, around a program's statements).
pub(crate) fn with_phrases<T>(phrases: &[(String, String)], f: impl FnOnce() -> T) -> T {
    let n = PHRASES.with(|p| {
        let mut p = p.borrow_mut();
        let n = p.len();
        p.extend(phrases.iter().cloned());
        n
    });
    let out = f();
    PHRASES.with(|p| p.borrow_mut().truncate(n));
    out
}

/// The phrasebook id a `use phrase` line gave `name`, in the program being parsed.
pub(crate) fn phrase_named(name: &str) -> Option<String> {
    PHRASES.with(|p| p.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, id)| id.clone()))
}

// ---------------------------------------------------------------- where a name is used

/// The place a name is used in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// After `ROLE=` in a template: the role reads the source.
    Role,
    /// `sing NAME on`, `template(T, NAME`: a phrase.
    Phrase,
    /// Where a section goes: an `arrange` line, a section line of `arrange { }`, `after` / `before` / `launch` / `start` / `in`.
    Arrange,
    /// Where a line goes: a part's pattern, a mutation.
    Line,
    /// Anything else (a `let` of names, a call): left to the checker.
    Other,
}

/// One use of a name.
struct Occ {
    at: Span,
    place: Place,
    /// It stands in a `use` line (another one's source).
    in_use: bool,
}

/// For each token: the first token of its line (a `|` line goes with the part line above it).
fn line_heads(toks: &[Token]) -> Vec<usize> {
    let mut out = Vec::with_capacity(toks.len());
    let mut head = 0;
    for (k, t) in toks.iter().enumerate() {
        if k > 0 && toks[k - 1].tok == Tok::Newline && t.tok != Tok::Pipe {
            head = k;
        }
        out.push(head);
    }
    out
}

/// For each token: the `template` word whose parentheses it is inside (the innermost), if any.
fn template_context(toks: &[Token]) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity(toks.len());
    let mut stack: Vec<(usize, i32)> = Vec::new();
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::LParen => {
                depth += 1;
                if k > 0 && matches!(&toks[k - 1].tok, Tok::Word(w) if w == "template") && touches(&toks[k - 1], t) {
                    stack.push((k - 1, depth));
                }
                out.push(stack.last().map(|s| s.0));
            }
            Tok::RParen => {
                out.push(stack.last().map(|s| s.0));
                if stack.last().is_some_and(|s| s.1 == depth) {
                    stack.pop();
                }
                depth -= 1;
            }
            Tok::Newline => {
                stack.clear();
                depth = 0;
                out.push(None);
            }
            _ => out.push(stack.last().map(|s| s.0)),
        }
    }
    out
}

/// For each token: is it inside an `arrange { }` block?
fn arrange_blocks(toks: &[Token], heads: &[usize]) -> Vec<bool> {
    let mut out = Vec::with_capacity(toks.len());
    let mut block: Option<i32> = None;
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::LBrace => {
                depth += 1;
                if block.is_none() && matches!(&toks[heads[k]].tok, Tok::Word(w) if w == "arrange") {
                    block = Some(depth);
                }
            }
            Tok::RBrace => {
                if block == Some(depth) {
                    block = None;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(block.is_some());
    }
    out
}

/// Is `w` the name, or (for a line a let a bar) one of its bars (`tune_3`) or their glob (`tune_` before a glued `*`)?
fn names_it(w: &str, name: &str, bars: bool, glob_next: bool) -> bool {
    if w == name {
        return true;
    }
    if !bars {
        return false;
    }
    match w.strip_prefix(name).and_then(|r| r.strip_prefix('_')) {
        Some("") => glob_next,
        Some(r) => r.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Every use of `name` in `toks` but its own `as NAME`, with its place.
fn uses_of(toks: &[Token], name: &str, bars: bool, own: Span, heads: &[usize], tpl: &[Option<usize>], blocks: &[bool], use_lines: &[(usize, usize)]) -> Vec<Occ> {
    let mut out = Vec::new();
    for (k, t) in toks.iter().enumerate() {
        let Tok::Word(w) = &t.tok else { continue };
        let glob_next = toks.get(k + 1).is_some_and(|n| n.tok == Tok::Star && touches(t, n));
        if t.span == own || !names_it(w, name, bars, glob_next) {
            continue;
        }
        let prev = k.checked_sub(1).map(|p| &toks[p].tok);
        let next = toks.get(k + 1).map(|n| &n.tok);
        // a stored program's section (`<cid>.name`), a deck's part (`A.kick`)
        if prev == Some(&Tok::Dot) && touches(&toks[k - 1], t) {
            continue;
        }
        // a definition's own name (`let tune = ...`, `section hook ...`) is no use of it: that it is bound is said once, at the `use`
        if k > 0 && heads[k] == k - 1 && matches!(prev, Some(Tok::Word(h)) if matches!(h.as_str(), "let" | "section" | "def")) {
            continue;
        }
        let in_use = use_lines.iter().any(|(s, e)| (*s..*e).contains(&k));
        let place = if let Some(ti) = tpl[k] {
            if next == Some(&Tok::Eq) || k == ti + 2 {
                // a role's own name, the template's
                continue;
            }
            if prev == Some(&Tok::Eq) {
                Place::Role
            } else if k == ti + 4 && toks[ti + 3].tok == Tok::Comma {
                Place::Phrase
            } else {
                Place::Other
            }
        } else {
            if prev == Some(&Tok::Eq) {
                // a param's value (`inst=tune`)
                continue;
            }
            let head = &toks[heads[k]].tok;
            let pw = |x: &str| matches!(prev, Some(Tok::Word(p)) if p == x);
            if pw("sing") && matches!(next, Some(Tok::Word(o)) if o == "on") {
                Place::Phrase
            } else if pw("theme") || pw("phrase") || pw("genre") {
                continue;
            } else if pw("after") || pw("before") || pw("launch") || pw("start") || pw("in") {
                Place::Arrange
            } else if blocks[k] {
                if matches!(head, Tok::Tilde | Tok::At) {
                    Place::Line
                } else {
                    Place::Arrange
                }
            } else {
                match head {
                    Tok::Word(h) if h == "arrange" => Place::Arrange,
                    Tok::Word(h) if h == "deck" => Place::Line,
                    Tok::Pipe => Place::Line,
                    _ => Place::Other,
                }
            }
        };
        // in a `use` line, only a role's or a phrase's place is a use of a name (the rest is the line's own source)
        if in_use && !matches!(place, Place::Role | Place::Phrase) {
            continue;
        }
        out.push(Occ { at: t.span, place, in_use });
    }
    out
}

/// The names a program defines itself, at the head of a line: `let`, `section`, `def`, `arrange NAME:`; with where.
fn defined(toks: &[Token]) -> Vec<(String, Span, &'static str)> {
    let mut out = Vec::new();
    for (k, t) in toks.iter().enumerate() {
        if !(k == 0 || matches!(toks[k - 1].tok, Tok::Newline | Tok::LBrace)) {
            continue;
        }
        let Tok::Word(w) = &t.tok else { continue };
        let what = match w.as_str() {
            "let" => "a let",
            "section" => "a section",
            "def" => "a def",
            "arrange" if toks.get(k + 2).is_some_and(|c| c.tok == Tok::Colon) => "an arrangement",
            _ => continue,
        };
        if let Some(Token { tok: Tok::Word(n), span }) = toks.get(k + 1) {
            out.push((n.clone(), *span, what));
        }
    }
    out
}

// ---------------------------------------------------------------- resolving and writing out

/// One `use` of a program, resolved: what `check` reports of it (`imports`, and a reference of kind `use` in `refs`).
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub name: String,
    pub makes: Makes,
    pub source: Source,
    /// The source in canonical spelling.
    pub call: String,
    /// The source, in the text.
    pub span: Span,
    pub name_span: Span,
    /// The whole `use` line, from `use` to the name.
    pub line: Span,
    /// What it is pinned to: a CID (a stored program, a generated one, a canned section's or a genre template's written-out
    /// program), or a library's CID (a theme's, a phrase's); None for a form.
    pub resolved: Option<String>,
    /// A `gen` call's generator and version (`seeded@2`), and what it was fitted to.
    pub generator: Option<String>,
    pub fitted: Vec<String>,
    /// What the line is written out as where the name is used outside roles (else nothing): with `full`, through every stage
    /// after it (what eject writes).
    pub text: String,
    /// The other places it is written: a role's or a phrase's, each (where, what).
    pub also: Vec<(Span, String)>,
    /// What the name is, in a sentence (check's note at the line).
    pub what: String,
}

struct Out {
    exp: Expansion,
    programs: Vec<(String, String)>,
    imports: Vec<Import>,
    /// The edits of `exp` that are `use` lines.
    line_edits: Vec<usize>,
}

/// What a resolved source writes: its definition, its text in a role, its phrase.
struct Resolved {
    def: String,
    role: String,
    phrase: String,
    resolved: Option<String>,
    generator: Option<String>,
    fitted: Vec<String>,
    what: String,
}

/// The phrase a template's own example says (`arrange from template(passacaglia, @eccl3_season)`): what a canned section of a
/// template is written out with when it names none.
fn example_phrase(tpl: &crate::template::Template) -> String {
    tpl.example.split_whitespace().find_map(|w| w.strip_prefix('@')).map(|w| format!("@{}", w.trim_end_matches([',', ')']))).unwrap_or_else(|| "@tempest_such_stuff".into())
}

/// A program borrowed from (canonical), compiled: its CID, its sections as written and as played (with their bars).
struct Borrowed {
    cid: String,
    canon: String,
    written: Vec<String>,
    played: Vec<(String, u32)>,
}

fn borrowed(text: &str) -> Result<Borrowed, String> {
    let canon = crate::print::canonical(&crate::parse::parse(text).program);
    let cid = crate::compile::program_cid(&canon);
    let written: Vec<String> = crate::parse::parse(&canon).program.stmts.iter().filter_map(|s| if let crate::ast::StmtKind::Section(b) = &s.kind { Some(b.name.text.clone()) } else { None }).collect();
    let chk = crate::compile::check_with(&canon, &Env::none());
    let Some(comp) = chk.compiled else {
        let first = chk.diagnostics.iter().find(|d| d.is_error()).map_or(String::new(), |d| d.message.clone());
        return Err(first);
    };
    Ok(Borrowed { cid, canon, written, played: crate::roles::sequence(&comp) })
}

/// The section `section` of a borrowed program, or why not: `what` says the program in the message.
fn pick_section(b: &Borrowed, section: &str, what: &str, at: Span) -> Result<u32, Diagnostic> {
    if let Some((_, bars)) = b.played.iter().find(|(n, _)| n == section) {
        return Ok(*bars);
    }
    let names: Vec<&str> = b.played.iter().map(|(n, _)| n.as_str()).collect();
    if b.written.iter().any(|w| w == section) {
        return Err(Diagnostic::error(at, format!("section `{section}` of {what} is written but never played")).help(format!("its sections that play: {}", names.join(", "))));
    }
    let mut d = Diagnostic::error(at, format!("{what} has no section `{section}`")).expected(&names).found(format!("`{section}`"));
    d = match nearest(section, names.iter().copied()) {
        Some(n) => d.help(format!("did you mean `{n}`? Its sections: {}", names.join(", "))),
        None => d.help(format!("its sections: {}", names.join(", "))),
    };
    Err(d)
}

fn bars_word(n: usize) -> String {
    format!("{n} bar{}", if n == 1 { "" } else { "s" })
}

/// Resolve one source: what it writes, its pin. `phrases`: the phrases of the `use` lines above (a canned section's phrase may
/// be one's name); `programs`: the programs the `use` lines above wrote, to which this one's is added.
fn resolve(d: &Decl, env: &Env, host: &crate::gen::Host, phrases: &[(String, String)], programs: &mut Vec<(String, String)>) -> Result<Resolved, Diagnostic> {
    let name = &d.name;
    let at = d.source_span;
    let call = d.source.text();
    let mk = |def: String, role: String, what: String| Resolved { def, role, phrase: String::new(), resolved: None, generator: None, fitted: Vec::new(), what };
    Ok(match &d.source {
        Source::Stored { cid, section, part } => {
            let short = djengine::rights::short_cid(cid);
            let (def, what) = match (section, part) {
                (_, Some(p)) => (format!("let {name} = {call}"), format!("`{name}` is a line: the {p} of {}{short}, a let (its pattern; its params stay behind)", section.as_ref().map_or(String::new(), |s| format!("{s} of ")))),
                (Some(s), None) => (format!("section {name} from {call}"), format!("`{name}` is a section: the {s} of {short}")),
                (None, None) => (String::new(), format!("`{name}` is a source for a role: {short}, its sections in its order")),
            };
            Resolved { resolved: Some(cid.clone()), ..mk(def, call.clone(), what) }
        }
        Source::Gen(c) => {
            let (def, what) = match (&c.section, &c.part) {
                (s, Some(p)) => (format!("let {name} = {call}"), format!("`{name}` is a line: the {p} of {}{}, a let a bar (`{name}_*`)", s.as_ref().map_or(String::new(), |s| format!("the {s} of ")), c.program_text())),
                (Some(s), None) => (format!("section {name} from {call}"), format!("`{name}` is a section: the {s} of {}", c.program_text())),
                (None, None) => (String::new(), format!("`{name}` is a source for a role: the seed {}", c.program_text())),
            };
            let mut r = mk(def, call.clone(), what);
            if let Ok(g) = crate::gen::resolve(c, host) {
                r.resolved = Some(g.cid);
                r.generator = Some(g.generator);
                r.fitted = g.fitted.iter().map(|f| f.to_string()).collect();
            }
            r
        }
        Source::Form(f) => {
            if crate::template::find(&f.name).is_none() {
                let known = crate::template::names();
                let help = nearest(&f.name, known.iter().map(String::as_str)).map_or(format!("the templates: {}", known.join(", ")), |n| format!("did you mean `{n}`? The templates: {}", known.join(", ")));
                return Err(Diagnostic::error(f.name_span, format!("`{}` is not a template", f.name)).help(help));
            }
            mk(String::new(), call.clone(), format!("`{name}` is a source for a role: the form {call}, for a role that takes sections"))
        }
        Source::Canned { tpl, phrase, roles, section } => {
            let known = crate::template::names();
            let Some(t) = crate::template::find(tpl) else {
                let help = nearest(tpl, known.iter().map(String::as_str)).map_or(format!("the templates: {}", known.join(", ")), |n| format!("did you mean `{n}`? The templates: {}", known.join(", ")));
                return Err(Diagnostic::error(at, format!("`{tpl}` is not a template")).help(help));
            };
            let p = match phrase {
                None => example_phrase(t),
                Some(PhraseRef::Id(id)) => format!("@{id}"),
                Some(PhraseRef::Text(s)) => quote(s),
                Some(PhraseRef::Name(n)) => match phrases.iter().rev().find(|(x, _)| x == n) {
                    Some((_, id)) => format!("@{id}"),
                    None => return Err(Diagnostic::error(at, format!("no `use phrase` line above names `{n}`")).help(format!("use phrase @tempest_such_stuff as {n}, above this line"))),
                },
            };
            let tref = format!("template({tpl}, {p}{})", crate::roles::bindings_text(tpl, roles));
            let prog = format!("arrange from {tref}\n");
            let text = match crate::gen::with_generated(programs, || crate::stored::expand_text(&prog, env)) {
                Some(Ok((t, _, _))) => t,
                Some(Err(ds)) => return Err(Diagnostic::error(at, format!("{tref} does not write out: {}", ds.first().map_or("", |d| d.message.as_str()))).help("check it as `arrange from` alone to see where")),
                None => return Err(Diagnostic::error(at, format!("{tref} wrote nothing (this is a bug)"))),
            };
            let b = borrowed(&text).map_err(|m| Diagnostic::error(at, format!("{tref} does not compile: {m}")))?;
            let bars = pick_section(&b, section, &format!("the {tpl} template"), at)?;
            let def = format!("section {name} from {}.{section}", b.cid);
            let role = format!("{}.{section}", b.cid);
            let what = format!("`{name}` is a section: the {section} of {tref} ({}, templates {})", bars_word(bars as usize), crate::template::VERSION);
            let cid = b.cid.clone();
            if !programs.iter().any(|(c, _)| *c == b.cid) {
                programs.push((b.cid, b.canon));
            }
            Resolved { resolved: Some(cid), ..mk(def, role, what) }
        }
        Source::Theme(id) => {
            let Some(t) = djengine::themes::theme(id) else {
                let ids: Vec<&str> = djengine::themes::themes().iter().map(|t| t.id.as_str()).collect();
                let help = nearest(id, ids.iter().copied()).map_or("list_examples with themes, or /themes, lists the library".to_string(), |n| format!("did you mean `{n}`?"));
                return Err(Diagnostic::error(at, format!("`{id}` is no theme of the public-domain library")).found(format!("`{id}`")).help(help));
            };
            let into = host.key.as_deref().and_then(crate::music::Key::parse);
            let n = into.map_or(0, |k| crate::roles::theme_shift(t, k));
            let mut lets: Vec<String> = t.bars.iter().enumerate().map(|(i, bar)| format!("let {name}_{} = {}", i + 1, crate::roles::transpose_bar(bar, n))).collect();
            lets.push(format!("let {name} = <{}>", (1..=t.bars.len()).map(|i| format!("{name}_{i}")).collect::<Vec<_>>().join(" ")));
            let moved = match (n, &host.key) {
                (0, Some(k)) => format!("in {}, the program's key {k} takes it as it is", t.key),
                (0, None) => format!("in {}, as the library has it", t.key),
                (n, k) => format!("moved {n:+} semitones from {} into {}", t.key, k.clone().unwrap_or_default()),
            };
            let what = format!(
                "`{name}` is a line: {id} ({}, {}, {}), {} of {} steps, {moved}; play it as `{}{} {name}`, its bars are `{name}_1` ... `{name}_{}`",
                t.work,
                t.composer,
                t.year,
                bars_word(t.bars.len()),
                t.steps,
                t.role,
                t.cycle_mark(),
                t.bars.len()
            );
            Resolved { resolved: Some(crate::compile::program_cid(djengine::themes::themes_text())), ..mk(lets.join("\n"), id.clone(), what) }
        }
        Source::Phrase(id) => {
            let (cite, lib) = match djengine::vocal::phoneme::pd_phrase(id) {
                Some(p) => (format!("\"{}\" ({}, {}, {})", p.text, p.work, p.author, p.year), Some(crate::compile::program_cid(djengine::vocal::phoneme::pd_phrasebook_text()))),
                None => (djengine::vocal::phoneme::phrase(id).map_or(String::new(), |p| format!("\"{}\", a stock phrase", p.words)), None),
            };
            Resolved { phrase: format!("@{id}"), resolved: lib, ..mk(String::new(), String::new(), format!("`{name}` is a phrase: @{id}, {cite}")) }
        }
        Source::Genre { version, genre, section } => {
            if !djengine::genre_templates::VERSIONS.contains(version) {
                let have: Vec<String> = djengine::genre_templates::VERSIONS.iter().map(|v| format!("genre@{v}")).collect();
                return Err(Diagnostic::error(at, format!("this BeatScript has {}, not `genre@{version}`", have.join(", "))).help(format!("genre@{GENRE_BARE} {genre}.{section} (a version always means the same template)")));
            }
            let genres = djengine::genre_templates::genres(*version);
            let Some(t) = djengine::genre_templates::find(*version, genre) else {
                let help = if genres.is_empty() {
                    "this build has no genre templates".to_string()
                } else {
                    nearest(genre, genres.iter().copied()).map_or(format!("the genres: {}", genres.join(", ")), |n| format!("did you mean `{n}`? The genres: {}", genres.join(", ")))
                };
                return Err(Diagnostic::error(at, format!("genre@{version} has no template for `{genre}`")).found(format!("`{genre}`")).help(help));
            };
            let b = borrowed(&t.text).map_err(|m| Diagnostic::error(at, format!("the {genre} template does not compile: {m} (this is a bug)")))?;
            let bars = pick_section(&b, section, &format!("the {genre} template"), at)?;
            let def = format!("section {name} from {}.{section}", b.cid);
            let role = format!("{}.{section}", b.cid);
            let what = format!("`{name}` is a section: the {section} of the {genre} template (genre@{version}), {}", bars_word(bars as usize));
            let cid = b.cid.clone();
            if !programs.iter().any(|(c, _)| *c == b.cid) {
                programs.push((b.cid, b.canon));
            }
            Resolved { resolved: Some(cid), ..mk(def, role, what) }
        }
    })
}

/// A source re-read from its text with the names of the `use` lines above written in (`ground=walk` is `ground=<walk's
/// source>`): the source, or the error, at `at`.
fn reread(text: &str, at: Span) -> Result<Source, Diagnostic> {
    let line = format!("use {text} as reread_source\n");
    let (toks, _) = crate::gen::collapse(lex(&line).tokens);
    match runs(&toks).into_iter().next().map(|r| r.r) {
        Some(Ok(d)) => Ok(d.source),
        Some(Err(Some(mut e))) => {
            e.span = at;
            Err(e)
        }
        _ => Err(Diagnostic::error(at, "this source does not read with the names above it written in")),
    }
}

/// The lines a program says at its top level that a definition is written out under (its `tempo` and `key`), to be cut off again.
fn host_lines(host: &crate::gen::Host) -> String {
    let mut s = String::new();
    if let Some(t) = host.tempo {
        s.push_str(&format!("tempo {}\n", crate::print::num(t)));
    }
    if let Some(k) = &host.key {
        s.push_str(&format!("key {k}\n"));
    }
    s
}

fn expand_full(src: &str, env: &Env, full: bool) -> Option<Result<Out, Vec<Diagnostic>>> {
    let lexed = lex(src);
    let (toks, _) = crate::gen::collapse(lexed.tokens);
    let rs = runs(&toks);
    if rs.is_empty() {
        return None;
    }
    let mut errors: Vec<Diagnostic> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    let mut decls: Vec<(Span, Decl)> = Vec::new();
    for r in &rs {
        match &r.r {
            _ if r.depth != 0 => errors.push(Diagnostic::error(r.span, "a `use` goes at the top of the program, not in a block").help("move the `use ... as NAME` line above, out of the block")),
            Ok(d) => decls.push((r.span, d.clone())),
            Err(Some(e)) => errors.push(e.clone()),
            Err(None) => {}
        }
    }
    let use_lines: Vec<(usize, usize)> = rs.iter().map(|r| (r.start, r.end)).collect();
    let heads = line_heads(&toks);
    let tpl = template_context(&toks);
    let blocks = arrange_blocks(&toks, &heads);
    let own_defs: Vec<(String, Span, &'static str)> = {
        let in_use = |sp: Span| rs.iter().any(|r| r.span.start <= sp.start && sp.end <= r.span.end);
        defined(&toks).into_iter().filter(|(_, sp, _)| !in_use(*sp)).collect()
    };
    let host = crate::gen::host_of(&toks);
    let line_of = |at: usize| crate::diag::line_col(src, at).0;
    // the names: each once, and none a program's own
    for (k, (_, d)) in decls.iter().enumerate() {
        let bars = d.source.makes() == Makes::Line;
        if let Some((_, other)) = decls[..k].iter().find(|(_, o)| o.name == d.name) {
            errors.push(Diagnostic::error(d.name_span, format!("`{}` is named already, by the `use` on line {}", d.name, line_of(other.name_span.start))).help(format!("give this one another name: use {} as {}_2", d.source.text(), d.name)));
        } else if let Some((n, sp, what)) = own_defs.iter().find(|(n, _, _)| names_it(n, &d.name, bars, false)) {
            errors.push(Diagnostic::error(d.name_span, format!("`{}` is bound already: {what} `{n}` on line {}", d.name, line_of(sp.start))).help(format!("name the source otherwise: use {} as {}_2", d.source.text(), d.name)));
        }
    }
    // where each name is used, in order
    let mut occs: Vec<Vec<Occ>> = Vec::new();
    for (line, d) in &decls {
        let makes = d.source.makes();
        let v = uses_of(&toks, &d.name, makes == Makes::Line, d.name_span, &heads, &tpl, &blocks, &use_lines);
        let st = d.source.text();
        for o in &v {
            if line.start <= o.at.start && o.at.end <= line.end {
                errors.push(Diagnostic::error(o.at, format!("`{}` is used in its own `use`", d.name)).help("a source cannot name itself: bind the role to another source"));
                continue;
            }
            if o.at.start < line.start {
                errors.push(Diagnostic::error(o.at, format!("`{}` is used before its `use` (line {})", d.name, line_of(line.start))).help(format!("move `use {st} as {}` above line {}", d.name, line_of(o.at.start))));
                continue;
            }
            let wrong = match (o.place, makes) {
                (Place::Role, Makes::Phrase) => Some((format!("`{}` is a phrase, and a role takes a source", d.name), "a phrase is sung: sing NAME on melody, or template(deep-house, NAME)".to_string())),
                (Place::Phrase, m) if m != Makes::Phrase => Some((format!("`{}` is a {}, not a phrase", d.name, m.name()), format!("a phrase is a phrasebook line: use phrase @tempest_such_stuff as {}", d.name))),
                (Place::Arrange, Makes::Line) => Some((format!("`{}` is a line (a let), and this place takes a section", d.name), format!("play it in a section: section verse 8 bars {{ deck B: melody {} }}", d.name))),
                (Place::Line, Makes::Section) => Some((format!("`{}` is a section, and a part plays a line", d.name), match &d.source {
                    Source::Stored { cid, section: Some(s), .. } => format!("arrange it, or name one part of it: use {cid}.{s}.bass as {}_bass", d.name),
                    Source::Gen(c) => format!("arrange it, or name one part of it: use {} part=bass as {}_bass", c.text(), d.name),
                    _ => format!("arrange it: arrange {}", d.name),
                })),
                (Place::Arrange | Place::Line | Place::Other, Makes::Role) => Some((format!("`{}` is a source for a role, and goes after a role's `=`", d.name), format!("template(rondo, @shanty_drunken_sailor, episodes={})", d.name))),
                (Place::Arrange | Place::Line | Place::Other, Makes::Phrase) => Some((format!("`{}` is a phrase: it is sung", d.name), format!("sing {} on melody", d.name))),
                _ => None,
            };
            if let Some((m, h)) = wrong {
                errors.push(Diagnostic::error(o.at, m).help(h));
            }
        }
        if v.is_empty() {
            warnings.push(Diagnostic::warning(d.name_span, format!("`{}` is never used", d.name)).help("use it, or take the `use` line out"));
        }
        occs.push(v);
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    // resolve each, in order: a source's role and phrase places take the names of the lines above
    let mut programs: Vec<(String, String)> = Vec::new();
    let mut phrases: Vec<(String, String)> = Vec::new();
    // (one entry a line, in order: None for one that did not resolve)
    let mut done: Vec<Option<(Decl, Resolved)>> = Vec::new();
    for (_, d) in &decls {
        let mut d = d.clone();
        let mut inner: Vec<(Span, String)> = Vec::new();
        for (k, entry) in done.iter().enumerate() {
            let Some((_, r)) = entry else { continue };
            for o in occs[k].iter().filter(|o| o.in_use && d.source_span.start <= o.at.start && o.at.end <= d.source_span.end) {
                inner.push((o.at, if o.place == Place::Phrase { r.phrase.clone() } else { r.role.clone() }));
            }
        }
        if !inner.is_empty() {
            inner.sort_by_key(|(s, _)| s.start);
            let mut text = String::new();
            let mut at = d.source_span.start;
            for (s, w) in &inner {
                text.push_str(&src[at..s.start]);
                text.push_str(w);
                at = s.end;
            }
            text.push_str(&src[at..d.source_span.end]);
            match reread(&text, d.source_span) {
                Ok(s) => d.source = s,
                Err(e) => {
                    errors.push(e);
                    done.push(None);
                    continue;
                }
            }
        }
        match resolve(&d, env, &host, &phrases, &mut programs) {
            Ok(r) => {
                if let Source::Phrase(id) = &d.source {
                    phrases.push((d.name.clone(), id.clone()));
                }
                done.push(Some((d, r)));
            }
            Err(e) => {
                errors.push(e);
                done.push(None);
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    // what each writes, and where
    let mut pieces: Vec<(Span, String, bool)> = Vec::new();
    let mut imports: Vec<Import> = Vec::new();
    let prefix = host_lines(&host);
    for (k, ((line, orig), entry)) in decls.iter().zip(&done).enumerate() {
        let Some((d, r)) = entry else { continue };
        let outside = occs[k].iter().any(|o| !matches!(o.place, Place::Role | Place::Phrase));
        let def = if outside { r.def.clone() } else { String::new() };
        pieces.push((*line, def.clone(), true));
        let mut also = Vec::new();
        for o in occs[k].iter().filter(|o| !o.in_use) {
            match o.place {
                Place::Role => also.push((o.at, r.role.clone())),
                Place::Phrase => also.push((o.at, r.phrase.clone())),
                _ => {}
            }
        }
        for (s, w) in &also {
            pieces.push((*s, w.clone(), false));
        }
        warnings.push(Diagnostic::info(*line, r.what.clone()));
        // eject's text: through every stage after this one
        let text = if full && !def.is_empty() {
            let snippet = format!("{prefix}{def}\n");
            match crate::gen::with_generated(&programs, || crate::stored::expand_text(&snippet, env)) {
                Some(Ok((t, _, _))) => {
                    let skip = prefix.lines().count();
                    t.lines().skip(skip).collect::<Vec<_>>().join("\n")
                }
                _ => def.clone(),
            }
        } else {
            def.clone()
        };
        imports.push(Import {
            name: d.name.clone(),
            makes: d.source.makes(),
            source: d.source.clone(),
            call: orig.source.text(),
            span: d.source_span,
            name_span: d.name_span,
            line: *line,
            resolved: r.resolved.clone(),
            generator: r.generator.clone(),
            fitted: r.fitted.clone(),
            text,
            also,
            what: r.what.clone(),
        });
    }
    pieces.sort_by_key(|(s, _, _)| s.start);
    let mut text = String::with_capacity(src.len() * 2);
    let mut edits = Vec::new();
    let mut refs = Vec::new();
    let mut line_edits = Vec::new();
    let mut at = 0;
    for (sp, rep, is_line) in pieces {
        text.push_str(&src[at..sp.start]);
        let s = text.len();
        text.push_str(&rep);
        if is_line {
            line_edits.push(edits.len());
            if let Some(i) = imports.iter().find(|i| i.line == sp) {
                refs.push(i.call.clone());
            }
        }
        edits.push((sp, Span::new(s, text.len())));
        at = sp.end;
    }
    text.push_str(&src[at..]);
    Some(Ok(Out { exp: Expansion { text, edits, refs, warnings }, programs, imports, line_edits }))
}

/// `src` with every `use` line written out (the definition its source makes, where its name is used outside roles) and every
/// role's and phrase's use of a name written as its source, and the programs the lines borrow from (a canned section's and a
/// genre template's, each (CID, canonical text), for the stored stage to find: gen::with_generated); None when it has no `use`
/// line. Errors are at the lines and the names.
pub fn expand(src: &str, env: &Env) -> Option<Result<(Expansion, Vec<(String, String)>), Vec<Diagnostic>>> {
    let norm = normalise_input(src);
    expand_full(&norm, env, false).map(|r| r.map(|o| (o.exp, o.programs)))
}

/// Every `use` of `src`, resolved, in order (empty for a program with none). Errors are the expansion's.
pub fn imports(src: &str, env: &Env) -> Result<Vec<Import>, Vec<Diagnostic>> {
    let norm = normalise_input(src);
    match expand_full(&norm, env, false) {
        None => Ok(Vec::new()),
        Some(Err(e)) => Err(e),
        Some(Ok(o)) => Ok(o.imports),
    }
}

/// What refs.rs reads: the imports (their text written out through every stage, for eject), the written-out program with the
/// `use` lines' definitions blanked (so the stages after this one do not list them again as references of their own, as a
/// template's role sources are not: template::masked), the expansion (to map a place back) and the programs borrowed from.
pub(crate) fn for_refs(src: &str, env: &Env) -> Option<Result<(Vec<Import>, String, Expansion, Vec<(String, String)>), Vec<Diagnostic>>> {
    let o = match expand_full(src, env, true)? {
        Ok(o) => o,
        Err(e) => return Some(Err(e)),
    };
    let mut masked = o.exp.text.clone();
    for &k in &o.line_edits {
        let n = o.exp.edits[k].1;
        let blank: String = masked[n.start..n.end].chars().map(|c| if c == '\n' { "\n".to_string() } else { " ".repeat(c.len_utf8()) }).collect();
        masked.replace_range(n.start..n.end, &blank);
    }
    Some(Ok((o.imports, masked, o.exp, o.programs)))
}

/// The CIDs of the stored programs the `use` lines of `src` name, in order: what a store credits like an `import`.
pub fn stored_cids(src: &str) -> Vec<String> {
    let norm = normalise_input(src);
    let (toks, _) = crate::gen::collapse(lex(&norm).tokens);
    let mut out: Vec<String> = Vec::new();
    for r in runs(&toks) {
        if let Ok(Decl { source: Source::Stored { cid, .. }, .. }) = r.r {
            if !out.contains(&cid) {
                out.push(cid);
            }
        }
    }
    out
}

/// What the `use` lines of a canonical text are based on, beyond the CIDs and `gen` calls refs.rs finds itself: each theme,
/// phrase and genre template, with the CID of what it comes from (the library's file, the template's program).
pub fn bases(text: &str) -> Vec<(String, Option<String>)> {
    let (toks, _) = crate::gen::collapse(lex(text).tokens);
    let mut out = Vec::new();
    for r in runs(&toks) {
        let Ok(d) = r.r else { continue };
        let cid = match &d.source {
            Source::Theme(_) => Some(crate::compile::program_cid(djengine::themes::themes_text())),
            Source::Phrase(id) => djengine::vocal::phoneme::pd_phrase(id).map(|_| crate::compile::program_cid(djengine::vocal::phoneme::pd_phrasebook_text())),
            Source::Genre { version, genre, .. } => djengine::genre_templates::find(*version, genre).map(|t| crate::compile::program_cid(&crate::print::canonical(&crate::parse::parse(&t.text).program))),
            Source::Canned { .. } | Source::Form(_) => None,
            Source::Stored { .. } | Source::Gen(_) => continue,
        };
        out.push((d.source.text(), cid));
    }
    out
}

/// Is the line `toks[at]` stands on a `use` line (of a source, not `use restricted`)? Its CIDs and calls are the line's source,
/// not references of their own where they stand (stored.rs, gen.rs leave them to this stage).
pub(crate) fn on_use_line(toks: &[Token], at: usize) -> bool {
    let head = toks[..at].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |k| k + 1);
    matches!(toks.get(head).map(|t| &t.tok), Some(Tok::Word(w)) if w == "use") && !matches!(toks.get(head + 1).map(|t| &t.tok), Some(Tok::Word(w)) if w == "restricted")
}

// ---------------------------------------------------------------- what can be named

/// The sections a text writes, in order, once each (a template's text with its slots: `section sung $BARS bars {` is `sung`).
fn sections_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        if let Some(n) = l.strip_prefix("section ").and_then(|r| r.split_whitespace().next()) {
            if !out.iter().any(|o| o == n) {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// The sections a template writes (what `use template(NAME).SECTION` takes), and the phrase it is written with when it names
/// none (its own example's); None for no such template.
pub fn template_sections(name: &str) -> Option<(Vec<String>, String)> {
    let t = crate::template::find(name)?;
    Some((sections_in(t.text()), example_phrase(t)))
}

/// The sections genre template `genre` of version `version` writes (what `use genre@N GENRE.SECTION` takes).
pub fn genre_sections(version: u32, genre: &str) -> Option<Vec<String>> {
    djengine::genre_templates::find(version, genre).map(|t| sections_in(&t.text))
}
