//! A program's references, one by one (docs/beatscript/ext-ffi.md): what each one calls or borrows, which half of the language it is
//! (docs/BEATSCRIPT.md "Two halves"), where it stands, what it resolves to, and the BeatScript it is written out as there. `check`
//! reports them (`refs`), the editor shows or hides each one's expansion, and **eject** writes one out in place, under a line
//! that says where it came from:
//!
//! ```text
//! let bed = gen@1 techno seed=42 part=bass bars=17..20
//! ```
//!
//! ejects to
//!
//! ```text
//! # from: gen@1 techno seed=42 part=bass bars=17..20 -> z4EBG9j...
//! let bed_1 = c2 ~ c2 ~
//! ...
//! let bed = <bed_1 bed_2 bed_3 bed_4>
//! ```
//!
//! The `# from:` line is a comment: the canonical text (and so the CID) of the ejected program is that of the same notes typed by
//! hand, while the line keeps the call and the CID it resolved to, as provenance (the editor's hover, the program's JSON-LD
//! `isBasedOn`, lineage). Pure: no I/O, no clock.

use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::parse::ROLES;
use crate::seed::Expansion;

/// The two halves of the language (docs/BEATSCRIPT.md "Two halves: structure and engine").
pub const ENGINE: &str = "engine";
pub const STRUCTURE: &str = "structure";

/// One reference of a program.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// `gen` (a call of the seeded generator, `gen@N ...`: gen.rs), `seed` (a track of the beats generator, `seed(...)`: seed.rs),
    /// `stored` (a stored program's section or part, by CID) or `template` (a phrase or form template).
    pub kind: &'static str,
    /// [`ENGINE`] for a generator call, [`STRUCTURE`] for what is borrowed as source.
    pub half: &'static str,
    /// The reference in canonical spelling.
    pub call: String,
    /// Where it is written in the source.
    pub span: Span,
    /// What eject replaces: the reference, or the whole `let`, `section` or `arrange` line it stands in.
    pub replaced: Span,
    /// `part`, `let`, `section` or `arrange`.
    pub place: &'static str,
    /// The let's or section's name; the part's role.
    pub name: Option<String>,
    /// What replaces [`Site::replaced`] when it is written out (no `# from` line).
    pub text: String,
    /// What it resolves to: a generator call's result as a program of its own ([`crate::seed::resolve`]), a stored program's
    /// CID, a template's written-out program.
    pub resolved: Option<String>,
    /// A generator call's track: the generator's project hash.
    pub model: Option<String>,
    /// The generated track's or program's name.
    pub track: Option<String>,
    /// A `gen` call's generator and version (`seeded@2`).
    pub generator: Option<String>,
    /// What a `gen` call was fitted to: `key`, `bpm` (the program's own, where the call says none).
    pub fitted: Vec<String>,
}

/// The line `src[at..]` starts on: where it starts.
fn line_start(src: &str, at: usize) -> usize {
    src[..at.min(src.len())].rfind('\n').map_or(0, |i| i + 1)
}

/// The place a reference written at `orig` stands in, and its name.
fn place(src: &str, orig: Span) -> (&'static str, Option<String>) {
    let ls = line_start(src, orig.start);
    let head = src[ls..orig.start].trim_start();
    let at = src[orig.start..orig.end].trim_start();
    let words: Vec<&str> = at.split_whitespace().collect();
    if at.starts_with("section ") {
        return ("section", words.get(1).map(|s| s.to_string()));
    }
    if at.starts_with("arrange ") {
        return ("arrange", None);
    }
    if at.starts_with("let ") {
        return ("let", words.get(1).map(|s| s.to_string()));
    }
    let hw: Vec<&str> = head.split_whitespace().collect();
    if hw.first() == Some(&"let") && hw.last() == Some(&"=") {
        return ("let", hw.get(1).map(|s| s.to_string()));
    }
    // a part: the role is the last word before it
    let role = hw.last().map(|w| w.trim_start_matches('|')).filter(|w| ROLES.contains(w)).map(str::to_string);
    ("part", role)
}

/// The pieces of one expansion stage, as sites of `kind`.
fn pieces(src: &str, exp: &Expansion, kind: &'static str, out: &mut Vec<Site>) {
    for (k, (orig, new)) in exp.edits.iter().enumerate() {
        let call = exp.refs.get(k).cloned().unwrap_or_default();
        let (place, name) = place(src, *orig);
        let half = if matches!(kind, "gen" | "seed") { ENGINE } else { STRUCTURE };
        let text = exp.text[new.start..new.end].to_string();
        // the reference itself, inside what is replaced
        let lead: &str = match kind {
            "gen" => "gen",
            "seed" => "seed",
            "template" => "template(",
            _ => call.split('.').next().unwrap_or(&call),
        };
        let here = &src[orig.start..orig.end];
        // a seed's head is `seed(` or `seed@N(`, as it is written (`seed@1(` is canonically `seed(`)
        let found = if kind == "seed" { here.match_indices("seed").map(|(i, _)| i).find(|i| matches!(here.as_bytes().get(i + 4), Some(b'(' | b'@'))) } else { here.find(lead) };
        let span = match found {
            Some(i) if place != "part" => Span::new(orig.start + i, orig.end),
            _ => *orig,
        };
        out.push(Site { kind, half, call, span, replaced: *orig, place, name, text, resolved: None, model: None, track: None, generator: None, fitted: Vec::new() });
    }
}

/// Every reference of `src`, in order, resolved. Errors are the expansion's, at the references (as `check` gives them).
pub fn sites(src: &str, env: &Env) -> Result<Vec<Site>, Vec<Diagnostic>> {
    let norm = crate::lex::normalise_input(src);
    let src: &str = &norm;
    let mut out = Vec::new();
    if let Some(r) = crate::template::expand(src, env) {
        let e = r?;
        pieces(src, &e, "template", &mut out);
    }
    // a source bound to a template's role is the template's (ejected with it), not a reference of its own
    let rest = crate::template::masked(src);
    let src: &str = &rest;
    if let Some(r) = crate::seed::expand(src) {
        let e = r?;
        pieces(src, &e, "seed", &mut out);
    }
    if let Some(r) = crate::gen::expand(src) {
        let (e, progs) = r?;
        let host = crate::gen::host_of(&crate::lex::lex(src).tokens);
        // a part line's or a section's call is a stored reference to the program it resolved to: written out in turn
        let then = crate::gen::with_generated(&progs, || crate::stored::expand(&e.text, env)).transpose()?.map(|(x, _)| x);
        let mut mine = Vec::new();
        pieces(src, &e, "gen", &mut mine);
        for (k, s) in mine.iter_mut().enumerate() {
            let new = e.edits[k].1;
            if let Some(f) = &then {
                if let Some((o2, n2)) = f.edits.iter().find(|(o2, _)| o2.start <= new.start && new.end <= o2.end) {
                    s.text = f.text[n2.start..n2.end].to_string();
                    s.replaced = e.back(*o2);
                    let (place, name) = place(src, s.replaced);
                    s.place = place;
                    s.name = name;
                }
            }
            if let Ok(r) = crate::gen::resolve_word(&s.call, &host) {
                s.resolved = Some(r.cid);
                s.generator = Some(r.generator);
                s.fitted = r.fitted.iter().map(|f| f.to_string()).collect();
                s.track = r.text.lines().find_map(|l| l.strip_prefix("#! name:")).map(|n| n.trim().to_string());
            }
        }
        out.extend(mine);
    }
    if let Some(r) = crate::stored::expand(src, env) {
        let (e, _) = r?;
        pieces(src, &e, "stored", &mut out);
    }
    for s in &mut out {
        match s.kind {
            "gen" => {}
            "seed" => {
                let role = if s.place == "part" { s.name.as_deref() } else { None };
                if let Ok(r) = crate::seed::resolve(&s.call, role) {
                    s.resolved = Some(r.cid);
                    s.model = Some(r.model);
                    s.track = Some(r.name);
                }
            }
            "stored" => s.resolved = s.call.split('.').next().map(str::to_string),
            _ => {
                let parsed = crate::parse::parse(&s.text);
                s.resolved = Some(crate::compile::program_cid(&crate::print::canonical(&parsed.program)));
            }
        }
    }
    out.sort_by_key(|s| s.replaced.start);
    Ok(out)
}

/// The reference whose text holds byte `at` of `src`.
pub fn site_at(src: &str, env: &Env, at: usize) -> Option<Result<Site, Vec<Diagnostic>>> {
    match sites(src, env) {
        Err(e) => Some(Err(e)),
        Ok(v) => v.into_iter().find(|s| s.replaced.start <= at && at <= s.replaced.end).map(Ok),
    }
}

/// The provenance line eject writes above what it wrote out: `# from: <call> -> <cid>`.
pub fn from_line(s: &Site) -> String {
    match &s.resolved {
        Some(c) => format!("# from: {} -> {c}", s.call),
        None => format!("# from: {}", s.call),
    }
}

/// A `# from: <call> -> <cid>` line read back: the call and the CID (docs/beatscript/ext-ffi.md, provenance).
pub fn read_from_line(line: &str) -> Option<(String, Option<String>)> {
    let rest = line.trim().strip_prefix('#')?.trim_start().strip_prefix("from:")?.trim();
    if rest.is_empty() {
        return None;
    }
    Some(match rest.rsplit_once(" -> ") {
        Some((call, cid)) if crate::stored::is_cid(cid.trim()) => (call.trim().to_string(), Some(cid.trim().to_string())),
        _ => (rest.to_string(), None),
    })
}

/// `src` with the reference at byte `at` written out in place (the others left as they are), under its `# from:` line. The
/// line goes above the statement the reference is in, indented as it is; a section's or a whole track's `# from ...` comment is
/// replaced by it. None when nothing at `at` is a reference.
pub fn eject(src: &str, env: &Env, at: usize) -> Option<Result<String, Vec<Diagnostic>>> {
    let norm = crate::lex::normalise_input(src);
    let site = match site_at(&norm, env, at)? {
        Ok(s) => s,
        Err(e) => return Some(Err(e)),
    };
    Some(Ok(eject_site(&norm, &site)))
}

/// [`eject`] of a site already found.
pub fn eject_site(src: &str, s: &Site) -> String {
    let mut text = s.text.clone();
    // a section's or a track's expansion opens with its own `# from ...` comment: the provenance line says it now
    if text.starts_with("# from") {
        text = text.split_once('\n').map_or(String::new(), |(_, rest)| rest.to_string());
    }
    // the statement's first line: a part's line may go on over `|` lines
    let mut head = line_start(src, s.replaced.start);
    while head > 0 && src[head..].trim_start().starts_with('|') {
        head = line_start(src, head - 1);
    }
    let indent: String = src[head..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut out = String::with_capacity(src.len() + text.len() + 80);
    out.push_str(&src[..head]);
    out.push_str(&indent);
    out.push_str(&from_line(s));
    out.push('\n');
    out.push_str(&src[head..s.replaced.start]);
    // what was written out, its later lines at the statement's indent
    let mut first = true;
    for l in text.split('\n') {
        if !first {
            out.push('\n');
            if !l.is_empty() && s.place != "part" {
                out.push_str(&indent);
            }
        }
        out.push_str(l);
        first = false;
    }
    out.push_str(&src[s.replaced.end..]);
    out
}

/// The program with every reference written out (templates, seeds, `gen` calls, borrowed sections, in the order `check` writes
/// them), canonical, and its CID: the **expanded CID** of a recipe (docs/beatscript/ext-ffi.md). The same recipe always gives the same
/// expanded text and CID, and the expanded text plays as the recipe does. None for a program with no references (its expanded
/// CID is its own). Errors are the references'.
pub fn expanded(src: &str, env: &Env) -> Result<Option<(String, String)>, Vec<Diagnostic>> {
    match crate::stored::expand_text(src, env) {
        None => Ok(None),
        Some(Err(e)) => Err(e),
        Some(Ok((text, _, _))) => {
            let parsed = crate::parse::parse_with(&text, env);
            if let Some(d) = parsed.diagnostics.iter().find(|d| d.is_error()) {
                return Err(vec![d.clone()]);
            }
            let canon = crate::print::canonical(&parsed.program);
            let cid = crate::compile::program_cid(&canon);
            Ok(Some((canon, cid)))
        }
    }
}

/// One thing a program is based on: a call or a borrow its text still makes, or one an eject wrote out (`# from:`).
#[derive(Clone, Debug, PartialEq)]
pub struct Basis {
    /// The call or reference, canonical: `gen@2 techno seed=42 section=drop part=bass`, `z4EB....drop`.
    pub call: String,
    /// What it resolves to (the pinned CID), when that is known.
    pub cid: Option<String>,
    /// `called` (an engine call in the text), `borrowed` (a structural reference in the text) or `vendored` (written out by an
    /// eject, kept by its `# from:` line).
    pub how: &'static str,
}

/// What a program is based on, from its canonical text and its comments (`Compiled.notes`): the calls and borrows it makes,
/// then the ones it vendored. Resolving a `gen` call needs the generator (none: its CID is left out). The provenance a
/// program's JSON-LD carries as `isBasedOn` (progld.rs) and lineage counts.
pub fn based_on(text: &str, notes: &[String]) -> Vec<Basis> {
    let mut out: Vec<Basis> = Vec::new();
    let toks = crate::lex::lex(text).tokens;
    let host = crate::gen::host_of(&toks);
    for r in crate::gen::runs(&toks) {
        if let Ok(c) = r.r {
            let cid = crate::gen::resolve(&c, &host).ok().map(|x| x.cid);
            out.push(Basis { call: c.text(), cid, how: "called" });
        }
    }
    for r in crate::seed::runs(&toks) {
        if let Ok(s) = r.r {
            let cid = crate::seed::resolve(&s.text(), None).ok().map(|x| x.cid);
            out.push(Basis { call: s.text(), cid, how: "called" });
        }
    }
    for cid in crate::stored::cids_of(text) {
        out.push(Basis { call: cid.clone(), cid: Some(cid), how: "borrowed" });
    }
    for n in notes {
        if let Some((call, cid)) = read_from_line(&format!("# {n}")) {
            if !out.iter().any(|b| b.call == call && b.cid == cid) {
                out.push(Basis { call, cid, how: "vendored" });
            }
        }
    }
    out
}
