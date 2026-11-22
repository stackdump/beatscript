//! `gen@N genre seed=S ...`: a call of the seeded generator, the engine's side of the language (docs/BTW.md "Two halves", "Call
//! the generator"; long note docs/btw/ext-ffi.md).
//!
//! The generator is the one the MCP `generate` tool, `POST /api/generate` and beatscript.app's compose panel ("seeded, no model")
//! run: `seeded@N` (btwapi::generate), which prints a whole BeatScript program from a genre, a seed, a tempo, a key and a length.
//! A `gen` call is that program as a value: opaque until it is resolved, then pinned by the CID of the program it resolves to.
//! The version is in the text (`gen@2`; a bare `gen` is `gen@2` and the canonical text says so), so a call means the same music
//! for ever: a generator that would print another program for the same inputs is a new version, and the old one stays callable.
//!
//! ```text
//! let bed = gen@2 techno seed=42 section=drop part=bass    the bass of its drop, a let a bar: `~ bed_* rot 2` mutates it
//! deck B: arp gen@2 house seed=7 section=drop              a part of it, on this line's role, with its params
//! section hook from gen@2 techno seed=42 section=drop      a section of it: its parts, never its control lines
//! arrange from gen@2 techno seed=42 bpm=128 key=am         the whole program
//! ```
//!
//! **It fits the program it is in.** Unless the call says its own `bpm=` or `key=`, it is generated at the program's `tempo` and in
//! its `key` (the seeded generator takes both), so two seeds crossed by `morph` or `xfer` never clash; `check` says when a call
//! was fitted. That is part of the resolution: the same program text is the same call, the same generated program, the same CID.
//!
//! **What it is written out as.** The structural half reads the result: a part line and a section are a stored program's
//! (stored.rs: the generated program is the source, its section's parts with their params, never its control lines); a `let` is
//! the notes the part plays there, a let a bar (`let bed_1 = ...`, `let bed = <bed_1 bed_2>`), the way a theme is written
//! (docs/btw/forms.md), so arrange mutations reach every bar. A host with no generator installed says so at the call (`gen@2 is
//! not here`): hosts differ in whether it is there, never in what the music is.
//!
//! Pure: the generator is a pure function (no I/O, no clock), installed once by the host ([`install`]; btwapi does).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::diag::{Diagnostic, Span};
use crate::lex::{lex, Tok, Token};
use crate::parse::{nearest, ROLES};
use crate::seed::Expansion;

/// What a `gen` call asks the generator for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenRequest {
    pub genre: String,
    /// A whole number or a word (the generator hashes a word).
    pub seed: String,
    /// None: the genre's own.
    pub bpm: Option<f64>,
    /// A key word (`am`, `f#m`, `d:dorian`); empty: the genre's own.
    pub key: String,
    /// `short`, `standard` or `long`.
    pub length: String,
}

/// A generator of programs, by version: the seeded generator (btwapi::generate) is the one hosts install.
pub trait Generator: Sync {
    /// Its name, as `#! generator:` says it: `seeded`.
    fn name(&self) -> &str;
    /// The versions it can still print, oldest first.
    fn versions(&self) -> Vec<u32>;
    /// The genres a version knows.
    fn genres(&self, version: u32) -> Vec<String>;
    /// The program a request makes, with its `#!` header lines; an error for an input the version refuses.
    fn generate(&self, version: u32, req: &GenRequest) -> Result<String, String>;
}

static GENERATOR: OnceLock<&'static dyn Generator> = OnceLock::new();

/// Install the generator `gen` calls (once; later calls are ignored).
pub fn install(g: &'static dyn Generator) {
    let _ = GENERATOR.set(g);
}

/// The installed generator, if any.
pub fn generator() -> Option<&'static dyn Generator> {
    GENERATOR.get().copied()
}

/// A bare `gen` is this version, for ever (the canonical text writes it).
pub const BARE: u32 = 2;
/// The lengths the seeded generator takes.
pub const LENGTHS: [&str; 3] = ["short", "standard", "long"];
/// The names a call takes, the generator's own (seeded's `generate`) and then which piece of the program it stands for.
pub const PARAMS: [&str; 6] = ["seed", "bpm", "key", "length", "section", "part"];

/// `gen@N genre seed=S [bpm=B] [key=K] [length=L] [section=X] [part=R]`, as read.
#[derive(Clone, Debug, PartialEq)]
pub struct GenCall {
    pub version: u32,
    pub version_span: Span,
    pub genre: String,
    pub genre_span: Span,
    pub seed: String,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub length: Option<String>,
    pub section: Option<String>,
    pub section_span: Span,
    pub part: Option<String>,
    pub part_span: Span,
    pub span: Span,
}

impl GenCall {
    /// The canonical spelling: `gen@2 techno seed=42`, then `bpm=`, `key=` and `length=` when said (the default length says
    /// nothing), then `section=` and `part=`.
    pub fn text(&self) -> String {
        let mut s = format!("gen@{} {} seed={}", self.version, self.genre, self.seed);
        if let Some(b) = self.bpm {
            s.push_str(&format!(" bpm={}", crate::print::num(b)));
        }
        if let Some(k) = &self.key {
            s.push_str(&format!(" key={k}"));
        }
        if let Some(l) = self.length.as_deref().filter(|l| *l != "standard") {
            s.push_str(&format!(" length={l}"));
        }
        if let Some(x) = &self.section {
            s.push_str(&format!(" section={x}"));
        }
        if let Some(p) = &self.part {
            s.push_str(&format!(" part={p}"));
        }
        s
    }

    /// The call without which piece of the program it names: what is generated.
    pub fn program_text(&self) -> String {
        GenCall { section: None, part: None, ..self.clone() }.text()
    }
}

/// A word the parser reads as a `gen` call (one word since [`collapse`]).
pub fn is_call_word(w: &str) -> bool {
    w.starts_with("gen@") && w.contains(' ')
}

fn touches(a: &Token, b: &Token) -> bool {
    a.span.end == b.span.start
}

/// `gen` or `gen@N` followed by a word (the genre): a call. `evolve`'s `gen` (`when gen % 4 == 0`, `stop when gen == 3`) is
/// followed by a mark, never by a word, so the two never meet.
fn head(toks: &[Token], i: usize) -> Option<Option<u32>> {
    let Tok::Word(w) = &toks[i].tok else { return None };
    let v = if w == "gen" {
        None
    } else {
        let n = w.strip_prefix("gen@")?;
        if n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(n.parse::<u32>().unwrap_or(u32::MAX))
    };
    matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Word(_))).then_some(v)
}

const SHAPE: &str = "gen@2 GENRE seed=N [bpm=B] [key=K] [length=short|standard|long] [section=NAME] [part=ROLE]: gen@2 techno seed=42 section=drop part=bass";

/// A call in a token stream: tokens `start..end`.
pub(crate) struct Run {
    pub start: usize,
    pub end: usize,
    pub span: Span,
    pub r: Result<GenCall, Diagnostic>,
}

/// Every `gen` call in `toks`, read as far as it goes.
pub(crate) fn runs(toks: &[Token]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let Some(v) = head(toks, i) else {
            i += 1;
            continue;
        };
        let (end, r) = read(toks, i, v);
        let span = toks[i].span.to(toks[end.max(i + 1) - 1].span);
        out.push(Run { start: i, end: end.max(i + 1), span, r: r.map(|mut c| {
            c.span = span;
            c
        }) });
        i = end.max(i + 1);
    }
    out
}

/// Read one call from `toks[i]`: its genre, then `name=value` pairs, each name once. The first word that
/// is not one of its names (a part's `inst=`), a second use of a name, or the end of the line ends the call: what follows
/// belongs to the line.
fn read(toks: &[Token], i: usize, version: Option<u32>) -> (usize, Result<GenCall, Diagnostic>) {
    let at = |j: usize| toks.get(j).map(|t| &t.tok);
    let skip = |mut j: usize| {
        // the rest of the call goes with the error, so one mistake is one error
        while j < toks.len() && !matches!(toks[j].tok, Tok::Newline | Tok::Eof | Tok::Pipe) {
            j += 1;
        }
        j
    };
    let Some(Tok::Word(genre)) = at(i + 1).cloned() else { return (i + 1, Err(Diagnostic::error(toks[i].span, "a call names a genre").help(SHAPE))) };
    let mut c = GenCall {
        version: version.unwrap_or(BARE),
        version_span: toks[i].span,
        genre: genre.to_ascii_lowercase(),
        genre_span: toks[i + 1].span,
        seed: String::new(),
        bpm: None,
        key: None,
        length: None,
        section: None,
        section_span: Span::new(0, 0),
        part: None,
        part_span: Span::new(0, 0),
        span: Span::new(0, 0),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut j = i + 2;
    loop {
        let (Some(Tok::Word(k)), Some(Tok::Eq)) = (at(j), at(j + 1)) else { break };
        // (`seed=42` or `seed = 42`: a param's `=` is free, as a part's is)
        if !PARAMS.contains(&k.as_str()) || seen.contains(k) {
            break;
        }
        let k = k.clone();
        let key = toks[j].span;
        let vat = j + 2;
        let vsp = toks.get(vat).map_or(toks[j + 1].span, |t| t.span);
        let bad = |msg: String| (skip(vat), Err(Diagnostic::error(key.to(vsp), msg).help(SHAPE)));
        let mut next = vat + 1;
        match (k.as_str(), at(vat)) {
            ("seed", Some(Tok::Num { value, int: true })) if *value >= 0.0 => c.seed = format!("{}", *value as u64),
            ("seed", Some(Tok::Word(w))) => c.seed = w.to_ascii_lowercase(),
            ("seed", _) => return bad("a seed is a whole number or a word: seed=42, seed=sunrise".into()),
            ("bpm", Some(Tok::Num { value, .. })) if (40.0..=300.0).contains(value) => c.bpm = Some(*value),
            ("bpm", _) => return bad("`bpm=` is a tempo from 40 to 300".into()),
            ("key", Some(Tok::Word(w))) => {
                // `d:dorian` is three tokens, glued
                let mut word = w.clone();
                if matches!(at(vat + 1), Some(Tok::Colon)) && toks.get(vat + 1).is_some_and(|t| touches(&toks[vat], t)) {
                    if let (Some(Tok::Word(m)), true) = (at(vat + 2), toks.get(vat + 2).is_some_and(|t| touches(&toks[vat + 1], t))) {
                        word = format!("{w}:{m}");
                        next = vat + 3;
                    }
                }
                if crate::music::Key::parse(&word).is_none() {
                    return bad(format!("`key={word}` is no key: key=am, key=f#m, key=d:dorian"));
                }
                c.key = Some(crate::music::canonical_pitch_word(&word));
            }
            ("key", _) => return bad("`key=` takes a key: key=am".into()),
            ("length", Some(Tok::Word(w))) => {
                let l = match w.as_str() {
                    "short" | "minimal" => "short",
                    "standard" | "medium" => "standard",
                    "long" | "extended" => "long",
                    _ => {
                        let mut d = Diagnostic::error(vsp, format!("`length={w}`: the generator's lengths are short, standard and long")).expected(&LENGTHS).found(format!("`{w}`"));
                        if let Some(n) = nearest(w, LENGTHS) {
                            d = d.help(format!("did you mean `length={n}`?"));
                        }
                        return (skip(vat), Err(d));
                    }
                };
                c.length = Some(l.to_string());
            }
            ("length", _) => return bad("`length=` is short, standard or long".into()),
            ("section", Some(Tok::Word(w))) => {
                c.section = Some(w.clone());
                c.section_span = vsp;
            }
            ("section", _) => return bad("`section=` names one of the program's sections: section=drop".into()),
            ("part", Some(Tok::Word(w))) if ROLES.contains(&w.as_str()) => {
                c.part = Some(w.clone());
                c.part_span = vsp;
            }
            ("part", Some(Tok::Word(w))) => {
                let mut d = Diagnostic::error(vsp, format!("`part={w}`: a program's parts are its roles")).expected(&ROLES).found(format!("`{w}`"));
                if let Some(n) = nearest(w, ROLES) {
                    d = d.help(format!("did you mean `part={n}`?"));
                }
                return (skip(vat), Err(d));
            }
            ("part", _) => return bad("`part=` takes a role: part=bass".into()),
            _ => break,
        }
        seen.insert(k);
        j = next;
    }
    if c.seed.is_empty() {
        return (j, Err(Diagnostic::error(toks[i].span.to(c.genre_span), format!("a generator call says its seed: gen@{} {} seed=42", c.version, c.genre)).help(SHAPE)));
    }
    (j, Ok(c))
}

/// Each call made one word token (its canonical spelling) for the parser, and what is wrong with any of them.
pub(crate) fn collapse(toks: Vec<Token>) -> (Vec<Token>, Vec<Diagnostic>) {
    let rs = runs(&toks);
    if rs.is_empty() {
        return (toks, Vec::new());
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut diags = Vec::new();
    let mut i = 0;
    for r in rs {
        out.extend_from_slice(&toks[i..r.start]);
        let text = match r.r {
            Ok(c) => c.text(),
            Err(d) => {
                diags.push(d);
                "gen@0 error".to_string()
            }
        };
        out.push(Token { tok: Tok::Word(text), span: r.span });
        i = r.end;
    }
    out.extend_from_slice(&toks[i..]);
    (out, diags)
}

// ---------------------------------------------------------------- resolving a call

/// What the program a call is in says for itself, at its top level: the key and the tempo a call is fitted to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Host {
    pub key: Option<String>,
    pub tempo: Option<f64>,
}

/// The `key` and `tempo` lines of `toks` at the top level (not in a section, def or repeat). A tempo that moves (`tempo 100 ->
/// 130 over 8 bars`) is fitted to where it starts.
pub fn host_of(toks: &[Token]) -> Host {
    let mut h = Host::default();
    let mut depth = 0i32;
    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::LBrace => depth += 1,
            Tok::RBrace => depth -= 1,
            Tok::Word(w) if depth == 0 && (i == 0 || toks[i - 1].tok == Tok::Newline) && !matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Colon)) => match (w.as_str(), toks.get(i + 1).map(|t| &t.tok)) {
                ("key", Some(Tok::Word(k))) if h.key.is_none() => {
                    let mut word = k.clone();
                    if let (Some(Tok::Colon), Some(Tok::Word(m))) = (toks.get(i + 2).map(|t| &t.tok), toks.get(i + 3).map(|t| &t.tok)) {
                        word = format!("{k}:{m}");
                    }
                    if crate::music::Key::parse(&word).is_some() {
                        h.key = Some(crate::music::canonical_pitch_word(&word));
                    }
                }
                ("tempo", Some(Tok::Num { value, .. })) if h.tempo.is_none() && (40.0..=300.0).contains(value) => h.tempo = Some(*value),
                _ => {}
            },
            _ => {}
        }
    }
    h
}

/// A call resolved: the program it is, and its CID, the pin.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// The call as written, canonical (what it says, not what it was fitted to).
    pub call: String,
    /// The generator and version: `seeded@2`.
    pub generator: String,
    /// The request the generator was given: the call's own params, and the program's key and tempo where it says none.
    pub request: GenRequest,
    /// What was fitted to the program: `key` and `bpm`, when the call took the program's.
    pub fitted: Vec<&'static str>,
    /// The generated program, its `#!` header lines and all.
    pub text: String,
    /// Its CID (of its canonical text): the pin.
    pub cid: String,
}

impl Resolved {
    /// `gen@2 techno seed=42 -> z4EB...`, and what it was fitted to.
    pub fn note(&self) -> String {
        let mut s = format!("{} -> {} ({})", self.call, self.cid, self.generator);
        if !self.fitted.is_empty() {
            let what: Vec<String> = self
                .fitted
                .iter()
                .map(|f| match *f {
                    "key" => format!("the program's key {}", self.request.key),
                    _ => format!("its tempo {}", self.request.bpm.map(crate::print::num).unwrap_or_default()),
                })
                .collect();
            s.push_str(&format!(": generated in {}", what.join(" and at ")));
        }
        s
    }
}

/// Resolve `c` in a program that says `host`. Errors are at the call.
pub fn resolve(c: &GenCall, host: &Host) -> Result<Resolved, Diagnostic> {
    let Some(g) = generator() else {
        return Err(Diagnostic::error(c.version_span.to(c.genre_span), format!("gen@{} is not here: this host has no generator installed", c.version)).help("check it where the seeded generator is (beatscript.app, btwd, the MCP check): the language never fetches"));
    };
    let versions = g.versions();
    if !versions.contains(&c.version) {
        let have: Vec<String> = versions.iter().map(|v| format!("gen@{v}")).collect();
        return Err(Diagnostic::error(c.version_span, format!("this BeatScript has {}, not `gen@{}`", have.join(", "), c.version)).help(format!("gen@{} (a version always means the same music)", versions.last().copied().unwrap_or(BARE))));
    }
    let genres = g.genres(c.version);
    if !genres.contains(&c.genre) {
        let mut d = Diagnostic::error(c.genre_span, format!("`{}` is not a genre of {}@{}", c.genre, g.name(), c.version)).found(format!("`{}`", c.genre));
        d = match nearest(&c.genre, genres.iter().map(String::as_str)) {
            Some(n) => d.help(format!("did you mean `{n}`? The genres: {}", genres.join(", "))),
            None => d.help(format!("the genres: {}", genres.join(", "))),
        };
        return Err(d);
    }
    let mut fitted = Vec::new();
    let key = match (&c.key, &host.key) {
        (Some(k), _) => k.clone(),
        (None, Some(k)) => {
            fitted.push("key");
            k.clone()
        }
        (None, None) => String::new(),
    };
    let bpm = match (c.bpm, host.tempo) {
        (Some(b), _) => Some(b),
        (None, Some(t)) => {
            fitted.push("bpm");
            Some(t)
        }
        (None, None) => None,
    };
    let request = GenRequest { genre: c.genre.clone(), seed: c.seed.clone(), bpm, key, length: c.length.clone().unwrap_or_else(|| "standard".into()) };
    let text = g.generate(c.version, &request).map_err(|e| Diagnostic::error(c.span, format!("{}@{} refuses it: {e}", g.name(), c.version)))?;
    let cid = crate::compile::program_cid(&crate::print::canonical(&crate::parse::parse(&text).program));
    Ok(Resolved { call: c.text(), generator: format!("{}@{}", g.name(), c.version), request, fitted, text, cid })
}

/// Resolve a call word as [`collapse`] spells it, in a program that says `host`.
pub fn resolve_word(word: &str, host: &Host) -> Result<Resolved, Diagnostic> {
    let toks = lex(word).tokens;
    let Some(run) = runs(&toks).into_iter().next() else { return Err(Diagnostic::error(Span::new(0, word.len()), format!("`{word}` is no generator call"))) };
    resolve(&run.r?, host)
}

// ---------------------------------------------------------------- the programs a check made

thread_local! {
    /// The programs the calls being checked on this thread were resolved to, (cid, text): the sources their part lines and
    /// sections are borrowed from (stored.rs), for as long as that check runs and no longer.
    static GENERATED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with `programs` known as generated (stored.rs finds a source there before it asks the host).
pub fn with_generated<T>(programs: &[(String, String)], f: impl FnOnce() -> T) -> T {
    let n = GENERATED.with(|g| {
        let mut g = g.borrow_mut();
        let n = g.len();
        g.extend(programs.iter().cloned());
        n
    });
    let out = f();
    GENERATED.with(|g| g.borrow_mut().truncate(n));
    out
}

/// The text of a program a call being checked resolved to.
pub fn generated(cid: &str) -> Option<String> {
    GENERATED.with(|g| g.borrow().iter().find(|(c, _)| c == cid).map(|(_, t)| t.clone()))
}

// ---------------------------------------------------------------- writing it out

/// Where a call stands decides what it is written out as.
enum Form {
    /// `let NAME = gen ...`: from the `let` word, the notes a let a bar.
    Let { name: String, from: usize },
    /// After a part's role word: a stored reference to the generated program (stored.rs writes it out).
    Part,
    /// `section NAME from gen ...`: the same.
    Section,
    /// `arrange from gen ...`: from the `arrange` word, the whole program.
    Arrange { from: usize },
    /// `play gen ...` in a set, `from gen ...` in a station: a track, which the set's or the station's check resolves (station.rs).
    Track,
}

fn form_of(toks: &[Token], run: &Run) -> Result<Form, Diagnostic> {
    let line_start = toks[..run.start].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |i| i + 1);
    let head: Vec<&Tok> = toks[line_start..run.start].iter().map(|t| &t.tok).collect();
    let word = |t: &Tok, w: &str| matches!(t, Tok::Word(x) if x == w);
    match head.as_slice() {
        [s, Tok::Word(_), f] if word(s, "section") && word(f, "from") => return Ok(Form::Section),
        [a, f] if word(a, "arrange") && word(f, "from") => return Ok(Form::Arrange { from: line_start }),
        [l, Tok::Word(name), Tok::Eq] if word(l, "let") => return Ok(Form::Let { name: name.clone(), from: line_start }),
        [p] if word(p, "play") || word(p, "from") => return Ok(Form::Track),
        _ => {}
    }
    if let Some(Tok::Word(w)) = run.start.checked_sub(1).map(|i| &toks[i].tok) {
        if ROLES.contains(&w.as_str()) {
            return Ok(Form::Part);
        }
    }
    Err(Diagnostic::error(run.span, "a generator call goes where a pattern goes, or makes a section or the whole track").help("let bed = gen@2 techno seed=42 section=drop part=bass   deck B: bass gen@2 techno seed=42 section=drop   section hook from gen@2 techno seed=42 section=drop   arrange from gen@2 techno seed=42"))
}

/// The sections a generated program plays, in order.
fn sections_of(text: &str) -> Vec<String> {
    let c = crate::compile::check_with(text, &crate::env::Env::none());
    let mut out: Vec<String> = Vec::new();
    if let Some(c) = c.compiled {
        for s in &c.segments {
            if s.header.is_some() && !s.chain && !s.name.contains('+') && !out.contains(&s.name) {
                out.push(s.name.clone());
            }
        }
    }
    out
}

/// The notes `role` plays in section `name` of the generated program, as a let a bar named after `let_name`.
fn let_bars(r: &Resolved, c: &GenCall, let_name: &str) -> Result<String, Diagnostic> {
    let comp = crate::compile::check_with(&r.text, &crate::env::Env::none()).compiled.ok_or_else(|| Diagnostic::error(c.span, format!("{} does not compile", r.generator)))?;
    let section = c.section.as_deref().unwrap_or_default();
    let role = c.part.as_deref().unwrap_or_default();
    let Some(seg) = comp.segments.iter().find(|s| s.header.is_some() && !s.chain && s.name == section) else {
        return Err(no_section(r, c));
    };
    let hits: Vec<&crate::compile::PartIr> = seg.decks.values().flat_map(|d| d.parts.iter()).filter(|p| p.role == role).collect();
    let part = match hits.as_slice() {
        [p] => *p,
        [] => {
            let have: Vec<&str> = seg.decks.values().flat_map(|d| d.parts.iter().map(|p| p.role.as_str())).filter(|r| ROLES.contains(r)).collect();
            return Err(Diagnostic::error(if c.part.is_some() { c.part_span } else { c.span }, format!("the {section} of {} has no {role}; it plays {}", c.program_text(), if have.is_empty() { "nothing".to_string() } else { have.join(", ") })));
        }
        _ => return Err(Diagnostic::error(c.part_span, format!("the {section} of {} plays {role} on both decks", c.program_text())).help(format!("borrow the section: section hook from {}", c.text()))),
    };
    if part.cycle_steps != 16 {
        return Err(Diagnostic::error(c.part_span, format!("its {role} has a cycle of its own ({} steps): a let a bar cannot hold it", part.cycle_steps)).help(format!("play it on a part line: {role} {}", c.text())));
    }
    let bars: Vec<[Vec<crate::seed::Note>; 16]> = part
        .cycles
        .iter()
        .map(|hs| {
            let mut bar: [Vec<crate::seed::Note>; 16] = Default::default();
            for h in hs {
                let slot = &mut bar[usize::from(h.step.min(15))];
                if !slot.iter().any(|n| n.note == h.note) {
                    // an accent is written as `!`: the writer marks the notes at or over its threshold
                    slot.push(crate::seed::Note { note: h.note, vel: if h.accent { 127 } else { 1 }, len: h.dur.max(1) });
                    slot.sort();
                }
            }
            bar
        })
        .collect();
    let texts: Vec<String> = bars.iter().map(|b| crate::seed::bar_text(role, b, 64)).collect();
    let p = crate::seed::period(&texts).max(1);
    if p > crate::seed::MAX_PERIOD {
        return Err(Diagnostic::error(c.span, format!("its {role} changes over {p} bars, more than a let can hold ({})", crate::seed::MAX_PERIOD)));
    }
    let one = |t: &str| if t.contains(' ') { t.to_string() } else { format!("[{t}]") };
    let mut out = String::new();
    for (k, t) in texts[..p].iter().enumerate() {
        out.push_str(&format!("let {let_name}_{} = {}\n", k + 1, one(t)));
    }
    out.push_str(&format!("let {let_name} = <{}>", (1..=p).map(|k| format!("{let_name}_{k}")).collect::<Vec<_>>().join(" ")));
    Ok(out)
}

fn no_section(r: &Resolved, c: &GenCall) -> Diagnostic {
    let have = sections_of(&r.text);
    let what = c.program_text();
    match &c.section {
        None => Diagnostic::error(c.span, format!("{what} is a program of sections: say which one this is")).help(format!("its sections: {}; {} section={}", have.join(", "), c.text(), have.first().map_or("drop", String::as_str))),
        Some(s) => {
            let names: Vec<&str> = have.iter().map(String::as_str).collect();
            let mut d = Diagnostic::error(c.section_span, format!("{what} has no section `{s}`")).expected(&names).found(format!("`{s}`"));
            d = match nearest(s, have.iter().map(String::as_str)) {
                Some(n) => d.help(format!("did you mean `section={n}`? Its sections: {}", have.join(", "))),
                None => d.help(format!("its sections: {}", have.join(", "))),
            };
            d
        }
    }
}

/// The whole generated program, for `arrange from`: its body (no `#!` header), without the settings the program says itself,
/// under a `# from:` line.
fn whole(r: &Resolved, src_toks: &[Token]) -> String {
    let says = |w: &str| src_toks.iter().enumerate().any(|(i, t)| matches!(&t.tok, Tok::Word(x) if x == w) && (i == 0 || src_toks[i - 1].tok == Tok::Newline) && !matches!(src_toks.get(i + 1).map(|t| &t.tok), Some(Tok::Colon)));
    let mut out = format!("# from: {} -> {}\n", r.call, r.cid);
    for l in r.text.lines() {
        let t = l.trim_start();
        if t.starts_with("#!") {
            continue;
        }
        let first = t.split_whitespace().next().unwrap_or("");
        if l == t && matches!(first, "tempo" | "key" | "swing") && says(first) {
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim_end_matches('\n').to_string()
}

/// `src` with every `gen` call written out, the programs they resolved to (cid, text), and a note for each, or None when it has
/// none. A part line's or a section's call is written as a reference to the generated program (`<cid>.drop`), which the stored
/// stage writes out with that program known ([`with_generated`]). Errors are at the calls.
pub fn expand(src: &str) -> Option<Result<(Expansion, Vec<(String, String)>), Vec<Diagnostic>>> {
    let norm = crate::lex::normalise_input(src);
    let src: &str = &norm;
    let toks = lex(src).tokens;
    let rs = runs(&toks);
    if rs.is_empty() {
        return None;
    }
    let host = host_of(&toks);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut programs: Vec<(String, String)> = Vec::new();
    let mut pieces: Vec<(Span, String, String)> = Vec::new();
    for run in &rs {
        let c = match &run.r {
            Ok(c) => c.clone(),
            Err(d) => {
                errors.push(d.clone());
                continue;
            }
        };
        let form = match form_of(&toks, run) {
            Ok(f) => f,
            Err(d) => {
                errors.push(d);
                continue;
            }
        };
        if matches!(form, Form::Track) {
            continue;
        }
        let r = match resolve(&c, &host) {
            Ok(r) => r,
            Err(d) => {
                errors.push(d);
                continue;
            }
        };
        warnings.push(Diagnostic::info(run.span, r.note()));
        if !programs.iter().any(|(x, _)| *x == r.cid) {
            // a source is its canonical text, which hashes to its CID (stored.rs re-hashes it)
            programs.push((r.cid.clone(), crate::print::canonical(&crate::parse::parse(&r.text).program)));
        }
        match form {
            Form::Arrange { from } => pieces.push((toks[from].span.to(run.span), whole(&r, &toks), c.text())),
            Form::Let { name, from } => {
                if c.part.is_none() {
                    errors.push(Diagnostic::error(run.span, "a `let` of a generator call names one part: say which").help(format!("let {name} = {} part=bass", c.text())));
                    continue;
                }
                if c.section.is_none() || !sections_of(&r.text).contains(c.section.as_ref().unwrap_or(&String::new())) {
                    errors.push(no_section(&r, &c));
                    continue;
                }
                match let_bars(&r, &c, &name) {
                    Ok(t) => pieces.push((toks[from].span.to(run.span), t, c.text())),
                    Err(d) => errors.push(d),
                }
            }
            Form::Track => {}
            Form::Part | Form::Section => {
                let have = sections_of(&r.text);
                match &c.section {
                    Some(s) if have.contains(s) => {}
                    _ => {
                        errors.push(no_section(&r, &c));
                        continue;
                    }
                }
                let mut w = r.cid.clone();
                for n in [&c.section, &c.part].into_iter().flatten() {
                    w.push('.');
                    w.push_str(n);
                }
                pieces.push((run.span, w, c.text()));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    // (only tracks of a set or a station: nothing to write out here)
    if pieces.is_empty() {
        return None;
    }
    let mut text = String::with_capacity(src.len() * 2);
    let mut edits = Vec::new();
    let mut refs = Vec::new();
    let mut at = 0;
    for (sp, rep, r) in pieces {
        text.push_str(&src[at..sp.start]);
        let s = text.len();
        text.push_str(&rep);
        edits.push((sp, Span::new(s, text.len())));
        refs.push(r);
        at = sp.end;
    }
    text.push_str(&src[at..]);
    Some(Ok((Expansion { text, edits, refs, warnings }, programs)))
}
