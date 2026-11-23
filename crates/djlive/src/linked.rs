//! F14 ontology links (docs/beatscript/ext-linked.md): the `#! genre:` and `#! tags:` header lines of a `.beatscript` file,
//! checked against the bitwrap ontology, and the JSON-LD document a program becomes.
//!
//! ```text
//! #! genre: https://ontology.bitwrap.io/genre/techno      (or `techno`, `genre:techno`, `bw:Techno`)
//! #! tags: acid, instrument:supersaw, four on floor        (terms of the ontology: local name, label, CURIE or IRI)
//! ```
//!
//! The header lines are comments to the language: they are not in the program's CID (as `#! license:` is not), and
//! a change of genre or tags is a new *object* of the same program, as a new licence is. This module is the pure
//! part. It has the header model ([`Links`], [`header_lines`]), the vocabulary the host hands in ([`Ontology`], a
//! trait so the check stays free of I/O and the tests can use a fake), the resolution of a spelling to the
//! published IRI with a did-you-mean ([`find`], [`canonical_links`], [`check_headers`], which `check_with` calls
//! when the [`Env`](crate::env::Env) has an ontology), and the JSON-LD builder ([`jsonld`], [`pretty`]).
//! Fetching and caching the vocabulary is the host's (`djhost::ontology`); nothing here touches the network.

use std::collections::BTreeMap;

use djengine::json::{utf16_cmp, JsonObject, JsonValue};
use djengine::rights::Rights;

use crate::compile::Compiled;
use crate::diag::{Diagnostic, Severity, Span};
use crate::env::suggest;

/// The vocabulary's namespace: its classes and properties (`bw:Genre`, `bw:pulses`). The CURIE prefix `bw:`.
pub const BW_NS: &str = "https://ontology.bitwrap.io/ns#";
/// Every term an author names is under this (`https://ontology.bitwrap.io/genre/techno`).
pub const BW_BASE: &str = "https://ontology.bitwrap.io/";
/// The namespace of the properties of a program in the JSON-LD (`djrust:genre`, `djrust:Program`). Nothing is
/// published at it yet: it is a name, not a place to fetch.
pub const DJRUST_NS: &str = "https://beats.bitwrap.io/ns/djrust#";
/// The class of a Euclidean rhythm, as the published ontology names it.
pub const EUCLID_CLASS: &str = "https://ontology.bitwrap.io/ns#EuclideanRhythm";

/// The kinds of term a header may name, in the order that settles a bare word two kinds both have (`bossa` is a
/// genre and a groove: the genre; `groove:bossa` says the other). The kind is also the path segment of its IRI.
pub const KIND_ORDER: [&str; 11] = ["genre", "instrument", "scale", "groove", "section", "family", "role", "rhythm", "progression", "structure", "concept"];

/// The shape of a Euclidean rhythm: `pulses` hits over `steps`, rotated by `rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Shape {
    pub steps: u32,
    pub pulses: u32,
    pub rotation: u32,
}

/// One term of the vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    /// `https://ontology.bitwrap.io/genre/techno`.
    pub iri: String,
    /// One of [`KIND_ORDER`] (a fake vocabulary may add its own).
    pub kind: String,
    /// The last segment of the IRI: `techno`.
    pub local: String,
    /// Its `rdfs:label`: `four on floor`.
    pub label: String,
    /// A Euclidean rhythm's pulses, steps and rotation.
    pub shape: Option<Shape>,
}

impl Term {
    pub fn new(kind: &str, local: &str, label: &str) -> Term {
        Term { iri: format!("{BW_BASE}{kind}/{local}"), kind: kind.into(), local: local.into(), label: label.into(), shape: None }
    }

    pub fn rhythm(local: &str, label: &str, pulses: u32, steps: u32, rotation: u32) -> Term {
        Term { shape: Some(Shape { steps, pulses, rotation }), ..Term::new("rhythm", local, label) }
    }

    /// "a genre", "an instrument".
    pub fn a_kind(&self) -> String {
        a_kind(&self.kind)
    }
}

fn a_kind(kind: &str) -> String {
    let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    format!("{article} {kind}")
}

/// The vocabulary a header is checked against: handed in by the host (`Env::ontology`), so this crate stays pure.
pub trait Ontology {
    /// Every term, in a fixed order (the order is the vocabulary's own; lookups do not depend on it).
    fn terms(&self) -> &[Term];
    /// The class of Euclidean rhythms, when the vocabulary has one: `bw:EuclideanRhythm`.
    fn euclid_class(&self) -> Option<&str> {
        None
    }
}

/// A vocabulary held in memory: the host's cached snapshot, or a fake in a test.
#[derive(Clone, Debug, Default)]
pub struct StaticOntology {
    terms: Vec<Term>,
    pub euclid_class: Option<String>,
    /// The published version, and the day the snapshot was fetched (empty for a hand-made one).
    pub version: String,
    pub fetched: String,
    /// The snapshot came from the published namespace and matched its checksums (false: unverified, say so).
    pub verified: bool,
}

impl StaticOntology {
    /// Terms sorted by IRI (a repeated IRI is kept once).
    pub fn new(mut terms: Vec<Term>) -> StaticOntology {
        terms.sort_by(|a, b| a.iri.cmp(&b.iri));
        terms.dedup_by(|a, b| a.iri == b.iri);
        StaticOntology { terms, euclid_class: Some(EUCLID_CLASS.to_string()), ..StaticOntology::default() }
    }

    /// The snapshot file of `djhost` (`crates/djhost/data/bitwrap-ontology.json`).
    pub fn from_json(text: &str) -> Result<StaticOntology, String> {
        let v = JsonValue::parse(text).map_err(|e| format!("the ontology snapshot is not JSON: {e}"))?;
        let list = v.get("terms").and_then(JsonValue::as_array).ok_or("the ontology snapshot has no `terms`")?;
        let mut terms = Vec::with_capacity(list.len());
        for (i, t) in list.iter().enumerate() {
            let s = |k: &str| t.get(k).and_then(JsonValue::as_str).map(str::to_string).ok_or_else(|| format!("ontology snapshot term {i}: no `{k}`"));
            let n = |k: &str| t.get(k).and_then(JsonValue::as_f64).filter(|x| *x >= 0.0 && x.fract() == 0.0 && *x < 1e6).map(|x| x as u32);
            let shape = match (n("pulses"), n("steps"), n("rotation")) {
                (Some(pulses), Some(steps), Some(rotation)) => Some(Shape { steps, pulses, rotation }),
                _ => None,
            };
            terms.push(Term { iri: s("iri")?, kind: s("kind")?, local: s("local")?, label: s("label")?, shape });
        }
        let mut o = StaticOntology::new(terms);
        o.euclid_class = v.get("classes").and_then(|c| c.get("euclidean_rhythm")).and_then(JsonValue::as_str).map(str::to_string);
        let get = |k: &str| v.get(k).and_then(JsonValue::as_str).unwrap_or("").to_string();
        o.version = get("version");
        o.fetched = get("fetched");
        o.verified = v.get("verified").and_then(JsonValue::as_bool).unwrap_or(false);
        Ok(o)
    }
}

impl Ontology for StaticOntology {
    fn terms(&self) -> &[Term] {
        &self.terms
    }
    fn euclid_class(&self) -> Option<&str> {
        self.euclid_class.as_deref()
    }
}

// ---- finding a term

/// The way a spelling is compared: lower case, with spaces, underscores and hyphens one hyphen
/// (`Four on Floor`, `four_on_floor` and `four-on-floor` are the same name).
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.trim().chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push('-');
        }
        gap = false;
        out.extend(c.to_lowercase());
    }
    out
}

fn rank(kind: &str) -> usize {
    KIND_ORDER.iter().position(|k| *k == kind).unwrap_or(KIND_ORDER.len())
}

/// Why a spelling is not a term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss<'a> {
    /// Nothing was written.
    Empty,
    /// No such term: the nearest ones, written the way the author wrote (a name for a name, an IRI for an IRI).
    Unknown { suggestions: Vec<String> },
    /// There is such a term, of another kind (`groove:bossa` where a genre is wanted).
    WrongKind { term: &'a Term, want: String },
}

/// The term `text` names: a full IRI, a CURIE (`bw:Techno`, or `genre:techno` with the kind as the prefix), or a
/// bare name (local name or label). `want` restricts it to a kind (the genre line wants a genre).
pub fn find<'a>(o: &'a dyn Ontology, text: &str, want: Option<&str>) -> Result<&'a Term, Miss<'a>> {
    let t = text.trim();
    if t.is_empty() {
        return Err(Miss::Empty);
    }
    let terms = o.terms();
    let wanted = |x: &Term| want.is_none_or(|w| w == x.kind);
    // the terms named `name`, optionally of one kind: the wanted kind first, else the wrong kind it is
    let by_name = |name: &str, kind: Option<&str>, as_iri: bool| -> Result<&'a Term, Miss<'a>> {
        let n = norm(name);
        let mut hits: Vec<&'a Term> = terms.iter().filter(|x| kind.is_none_or(|k| k == x.kind) && (norm(&x.local) == n || norm(&x.label) == n)).collect();
        hits.sort_by(|a, b| rank(&a.kind).cmp(&rank(&b.kind)).then(a.iri.cmp(&b.iri)));
        if let Some(h) = hits.iter().find(|x| wanted(x)) {
            return Ok(h);
        }
        if let (Some(h), Some(w)) = (hits.first(), want) {
            return Err(Miss::WrongKind { term: h, want: w.to_string() });
        }
        let pool: Vec<&Term> = terms.iter().filter(|x| kind.is_none_or(|k| k == x.kind) && wanted(x)).collect();
        let mut names: Vec<String> = pool.iter().map(|x| x.local.clone()).collect();
        names.sort();
        names.dedup();
        let mut s = suggest(&n, &names, 3);
        if as_iri {
            s = s.into_iter().filter_map(|l| pool.iter().find(|x| x.local == l).map(|x| x.iri.clone())).collect();
        }
        Err(Miss::Unknown { suggestions: s })
    };
    // a full IRI
    if t.contains("://") {
        if let Some(x) = terms.iter().find(|x| x.iri == t) {
            return if wanted(x) { Ok(x) } else { Err(Miss::WrongKind { term: x, want: want.unwrap_or_default().to_string() }) };
        }
        let tail = t.rsplit(['#', '/']).next().unwrap_or("");
        if t.starts_with(BW_NS) {
            // the namespace's own spelling (`ns#Techno`): the term of that name, whatever its kind
            return by_name(tail, None, false);
        }
        if t.starts_with(BW_BASE) {
            // under the vocabulary's base but not a term of it: the IRIs it may have meant
            let suggestions = match by_name(tail, None, true) {
                Ok(x) => vec![x.iri.clone()],
                Err(Miss::WrongKind { term, .. }) => vec![term.iri.clone()],
                Err(Miss::Unknown { suggestions }) => suggestions,
                Err(Miss::Empty) => Vec::new(),
            };
            return Err(Miss::Unknown { suggestions });
        }
        return Err(Miss::Unknown { suggestions: Vec::new() });
    }
    // a CURIE: `bw:Name`, or `kind:name`
    if let Some((prefix, rest)) = t.split_once(':') {
        let p = norm(prefix);
        if p == "bw" {
            return by_name(rest, None, false);
        }
        if let Some(k) = KIND_ORDER.iter().find(|k| **k == p) {
            return by_name(rest, Some(k), false);
        }
    }
    by_name(t, None, false)
}

/// A header value that is not a term: what to say, and what to write instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// `genre` or `tag`.
    pub what: &'static str,
    /// As written.
    pub given: String,
    pub message: String,
    pub help: Option<String>,
    /// What would be accepted, for an answer with no near name (a genre line lists the genres).
    pub expected: Vec<String>,
}

impl Problem {
    /// One line: ``unknown genre `tehcno`: did you mean `techno`?``.
    pub fn text(&self) -> String {
        match &self.help {
            Some(h) => format!("{}: {h}", self.message),
            None => self.message.clone(),
        }
    }

    pub fn diagnostic(&self, span: Span, severity: Severity) -> Diagnostic {
        let mut d = Diagnostic::error(span, self.message.clone()).found(self.given.clone());
        d.severity = severity;
        d.expected = self.expected.clone();
        d.help = self.help.clone();
        d
    }
}

fn explain(o: &dyn Ontology, what: &'static str, given: &str, miss: Miss) -> Problem {
    let q = format!("`{}`", given.trim());
    match miss {
        Miss::Empty => Problem {
            what,
            given: given.to_string(),
            message: format!("a {what} line names no {what}"),
            help: Some(if what == "genre" { "write one: `#! genre: techno`".into() } else { "write terms, comma apart: `#! tags: acid, supersaw`".into() }),
            expected: Vec::new(),
        },
        Miss::WrongKind { term, want } => Problem {
            what,
            given: given.to_string(),
            message: format!("{q} is {}, not {}", term.a_kind(), a_kind(&want)),
            help: Some(format!("the genre line takes a genre: `{}` or `genre:{}`", term.local, term.local)).filter(|_| want == "genre"),
            expected: Vec::new(),
        },
        Miss::Unknown { suggestions } => {
            let expected = if suggestions.is_empty() && what == "genre" {
                let mut g: Vec<String> = o.terms().iter().filter(|t| t.kind == "genre").map(|t| t.local.clone()).collect();
                g.sort();
                g
            } else {
                Vec::new()
            };
            let help = match suggestions.as_slice() {
                [] if what == "genre" => None,
                [] => Some("`djctl btw terms` lists the vocabulary".to_string()),
                [one] => Some(format!("did you mean `{one}`?")),
                many => Some(format!("did you mean {}?", many.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(" or "))),
            };
            Problem { what, given: given.to_string(), message: format!("unknown {what} {q}"), help, expected }
        }
    }
}

/// The genre `text` names (a term of kind `genre`), or why not.
pub fn resolve_genre<'a>(o: &'a dyn Ontology, text: &str) -> Result<&'a Term, Problem> {
    find(o, text, Some("genre")).map_err(|m| explain(o, "genre", text, m))
}

/// The term `text` names as a tag (any kind), or why not.
pub fn resolve_tag<'a>(o: &'a dyn Ontology, text: &str) -> Result<&'a Term, Problem> {
    find(o, text, None).map_err(|m| explain(o, "tag", text, m))
}

// ---- the header model

/// What a program is linked to: its genre (the full IRI), and its tags (full IRIs, sorted, each once: a set).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    pub genre: Option<String>,
    pub tags: Vec<String>,
}

impl Links {
    /// Tags sorted and de-duplicated, so the same set is the same object whatever order it was written in.
    pub fn new(genre: Option<String>, mut tags: Vec<String>) -> Links {
        tags.sort();
        tags.dedup();
        Links { genre: genre.filter(|g| !g.is_empty()), tags }
    }

    pub fn is_empty(&self) -> bool {
        self.genre.is_none() && self.tags.is_empty()
    }

    /// The header lines: `#! genre: IRI` and `#! tags: IRI, IRI`, each when there is something to say.
    pub fn header_text(&self) -> String {
        let mut s = String::new();
        if let Some(g) = &self.genre {
            s.push_str(&format!("#! genre: {g}\n"));
        }
        if !self.tags.is_empty() {
            s.push_str(&format!("#! tags: {}\n", self.tags.join(", ")));
        }
        s
    }
}

/// The most tags a program names. A tag is looked up in the vocabulary (with a did-you-mean when it is not there), which costs about a
/// millisecond each, so a header of 20,000 of them kept `check` busy for fifteen seconds and `jsonld` for forty (R-24): past this
/// many is an error that says so, and the rest are not looked up.
pub const MAX_TAGS: usize = 64;

fn too_many_tags(given: String, nth: usize) -> Problem {
    Problem {
        what: "tag",
        given,
        message: format!("a program has at most {MAX_TAGS} tags: this is the {nth}th"),
        help: Some(format!("keep the {MAX_TAGS} that matter most; a genre and a few tags say what a program is, and the vocabulary is searched with `djctl btw terms`")),
        expected: Vec::new(),
    }
}

/// `a, b, c` as the strings between the commas, trimmed; empty ones dropped.
pub fn split_tags(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// The genre and tags as an author wrote them, before they are looked up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawLinks {
    pub genre: Option<String>,
    pub tags: Vec<String>,
}

impl RawLinks {
    pub fn is_empty(&self) -> bool {
        self.genre.is_none() && self.tags.is_empty()
    }
}

/// One `#! key: value` line of the header block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderLine {
    pub key: String,
    pub value: String,
    /// The whole line (without its newline) in the text.
    pub line: Span,
    /// The value, trimmed.
    pub value_span: Span,
}

/// Where the header block ends: the first byte of the first line that does not start with `#!`.
pub fn header_end(src: &str) -> usize {
    let mut at = 0;
    for raw in src.split_inclusive('\n') {
        if !raw.trim_end().starts_with("#!") {
            break;
        }
        at += raw.len();
    }
    at
}

/// The header block: the leading run of lines that start with `#!` (as `live_store` reads them). A line with no
/// `key:` (`#! btw 1`) is not listed.
pub fn header_lines(src: &str) -> Vec<HeaderLine> {
    let mut out = Vec::new();
    let mut at = 0;
    for raw in src.split_inclusive('\n') {
        let start = at;
        at += raw.len();
        let t = raw.trim_end();
        let Some(h) = t.strip_prefix("#!") else { break };
        let Some((k, v)) = h.split_once(':') else { continue };
        let after_colon = start + 2 + k.len() + 1;
        let lead = v.len() - v.trim_start().len();
        let value = v.trim();
        out.push(HeaderLine {
            key: k.trim().to_string(),
            value: value.to_string(),
            line: Span::new(start, start + t.len()),
            value_span: Span::new(after_colon + lead, after_colon + lead + value.len()),
        });
    }
    out
}

/// The genre and tags the text's header block says, as written (several `tags:` lines are one set; the last
/// `genre:` line wins here: `check_headers` is what complains about a second).
pub fn read_links(src: &str) -> RawLinks {
    let mut r = RawLinks::default();
    for h in header_lines(src) {
        match h.key.as_str() {
            "genre" => r.genre = Some(h.value).filter(|v| !v.is_empty()),
            "tags" => r.tags.extend(split_tags(&h.value)),
            _ => {}
        }
    }
    r
}

/// Genre and tags resolved: every spelling to the IRI it names, or every problem found.
pub fn canonical_links(o: &dyn Ontology, genre: Option<&str>, tags: &[String]) -> Result<Links, Vec<Problem>> {
    let mut problems = Vec::new();
    let mut g = None;
    if let Some(text) = genre.filter(|g| !g.trim().is_empty()) {
        match resolve_genre(o, text) {
            Ok(t) => g = Some(t.iri.clone()),
            Err(p) => problems.push(p),
        }
    }
    let mut ts = Vec::new();
    if tags.len() > MAX_TAGS {
        // (not looked up: it is the cost of that which is capped)
        problems.push(too_many_tags(format!("{} tags", tags.len()), MAX_TAGS + 1));
        return Err(problems);
    }
    for tag in tags {
        match resolve_tag(o, tag) {
            Ok(t) => ts.push(t.iri.clone()),
            Err(p) => problems.push(p),
        }
    }
    if problems.is_empty() {
        Ok(Links::new(g, ts))
    } else {
        Err(problems)
    }
}

fn with_severity(mut d: Diagnostic, s: Severity) -> Diagnostic {
    d.severity = s;
    d
}

/// Check the text's header lines against the vocabulary, with the diagnostics at the value they are about
/// (`severity`: a warning from `check`, an error from save and import). The links that did resolve come back too.
pub fn check_headers(src: &str, o: &dyn Ontology, severity: Severity) -> (Links, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut genre: Option<String> = None;
    let mut seen_genre = false;
    let mut tags = Vec::new();
    let mut tag_count = 0usize;
    for h in &header_lines(src) {
        match h.key.as_str() {
            "genre" => {
                if seen_genre {
                    diags.push(with_severity(Diagnostic::error(h.line, "a program has one genre").found(h.value.clone()).help("keep one `#! genre:` line"), severity));
                    continue;
                }
                seen_genre = true;
                if h.value.contains(',') {
                    diags.push(with_severity(Diagnostic::error(h.value_span, "a program has one genre").found(h.value.clone()).help("one genre, and the rest as `#! tags:`"), severity));
                    continue;
                }
                match resolve_genre(o, &h.value) {
                    Ok(t) => genre = Some(t.iri.clone()),
                    Err(p) => diags.push(p.diagnostic(if h.value.is_empty() { h.line } else { h.value_span }, severity)),
                }
            }
            "tags" => {
                let mut offset = 0;
                for piece in h.value.split(',') {
                    let lead = piece.len() - piece.trim_start().len();
                    let item = piece.trim();
                    let from = h.value_span.start + offset + lead;
                    offset += piece.len() + 1;
                    if item.is_empty() {
                        continue;
                    }
                    tag_count += 1;
                    if tag_count > MAX_TAGS {
                        // said once, at the first one over; the rest are not looked up (R-24)
                        if tag_count == MAX_TAGS + 1 {
                            diags.push(too_many_tags(item.to_string(), tag_count).diagnostic(Span::new(from, from + item.len()), severity));
                        }
                        continue;
                    }
                    match resolve_tag(o, item) {
                        Ok(t) => tags.push(t.iri.clone()),
                        Err(p) => diags.push(p.diagnostic(Span::new(from, from + item.len()), severity)),
                    }
                }
            }
            _ => {}
        }
    }
    // a genre or tags line after the program has begun is a comment, and says nothing: say so
    let end = header_end(src);
    let mut at = end;
    for raw in src[end..].split_inclusive('\n') {
        let start = at;
        at += raw.len();
        let t = raw.trim_end();
        if let Some((key, _)) = t.strip_prefix("#!").and_then(|r| r.split_once(':')) {
            let key = key.trim();
            if key == "genre" || key == "tags" {
                diags.push(
                    Diagnostic::warning(Span::new(start, start + t.len()), format!("this `#! {key}:` line is not read: header lines are the first lines of the text")).help("move it to the top, above the program's first line"),
                );
            }
        }
    }
    (Links::new(genre, tags), diags)
}

// ---- what a program says of its structure

/// A section as the song places it: where it starts, how long it is, and the `key` it sets itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    /// The bar it starts on (0-based, as bars are everywhere in BeatScript).
    pub start: u32,
    pub bars: u32,
    /// The key the section's own first statements set; None: it sets none (it is in the program's, or in a stack the
    /// key of the stretch it plays in: [`Compiled::section_key`]).
    pub key: Option<crate::music::Key>,
}

/// What the compiler keeps for the JSON-LD (`Compiled::linked`): read from the lowered program, so it sees what plays
/// (a def's body once it is called, a `repeat`'s copies, arithmetic worked out, a part with a cycle of its own, a chance
/// term) and not the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Structure {
    /// Every placement of a section on the timeline, in the order the arrangement wrote them (a section placed twice is
    /// two; two sections stacked start on one bar). Empty for a song with no sections (a flat program, a bare `evolve`).
    pub placements: Vec<Placed>,
    /// The Euclidean rhythms, by shape, with how many times the expanded song says each (a def called twice, a `repeat 4`
    /// and a section placed twice count their copies; a copy of a part carried into the later stretch of a stack is not
    /// another).
    pub euclids: BTreeMap<Shape, u32>,
}

/// The placements of a plan, with the key each section sets (the head of its body, the last `key` wins as it does in
/// `scale.rs`). Called once by `expand::lower`.
pub(crate) fn placements(plan: &crate::algebra::Plan, sections: &BTreeMap<String, crate::expand::SectionInfo>) -> Vec<Placed> {
    use crate::ast::{Setting, StmtKind};
    let own_key = |name: &str| {
        let mut key = None;
        for st in sections.get(name).map(|s| s.block.body.as_slice()).unwrap_or_default() {
            match &st.kind {
                StmtKind::Setting(Setting::Key(k)) => key = crate::music::Key::parse(&k.text),
                _ => break,
            }
        }
        key
    };
    plan.placements.iter().map(|p| Placed { name: p.name.clone(), start: p.start, bars: p.bars, key: own_key(&p.name) }).collect()
}

/// The structure of a lowered program. Called once by `check_with`.
pub(crate) fn structure(lw: &crate::expand::Lowered) -> Structure {
    use crate::ast::{Arg, Body};
    let mut euclids: BTreeMap<Shape, u32> = BTreeMap::new();
    // The part lines of a scope with the deck and role each part is for: a section's own part of a role shadows the song's part of
    // that role in that section (as `scope_parts` and `Compiler::build` do), so a song's part that every section shadows plays
    // nowhere and its `e()` calls are not counted (L-18).
    let roles = |lines: &[crate::expand::FlatLine]| -> Vec<(crate::ast::Deck, String)> {
        let mut out = Vec::new();
        for fl in lines {
            if let (Body::Parts(ps), Some((crate::ast::Target::Deck(d), _))) = (&fl.line.body, fl.line.target) {
                out.extend(ps.iter().map(|p| (d, p.role.text.clone())));
            }
        }
        out
    };
    let shadows: Vec<Vec<(crate::ast::Deck, String)>> = lw.segments.iter().map(|s| roles(&s.lines)).collect();
    let plays_somewhere = |d: Option<crate::ast::Deck>, role: &str| shadows.is_empty() || shadows.iter().any(|own| !own.iter().any(|(od, orole)| Some(*od) == d && orole == role));
    // (a carried copy of a stack's part, `phase`, is the same part again)
    let globals = lw.globals.iter().map(|l| (l, true));
    let sections = lw.segments.iter().flat_map(|s| s.lines.iter()).map(|l| (l, false));
    for (line, is_global) in globals.chain(sections).filter(|(l, _)| l.phase.is_none()) {
        let Body::Parts(parts) = &line.line.body else { continue };
        let deck = match line.line.target {
            Some((crate::ast::Target::Deck(d), _)) => Some(d),
            _ => None,
        };
        for part in parts {
            if is_global && !plays_somewhere(deck, &part.role.text) {
                continue;
            }
            crate::meter::visit_calls(&part.pattern, &mut |call| {
                if !matches!(call.name.text.as_str(), "e" | "euclid") {
                    return;
                }
                let whole = |i: usize| match call.args.get(i) {
                    Some(Arg::Num(n)) if n.value.fract() == 0.0 && n.value.abs() <= 1e9 => Some(n.value as i64),
                    _ => None,
                };
                let rot = if call.args.len() > 2 { whole(2) } else { Some(0) };
                let (Some(k), Some(n), Some(rot)) = (whole(0), whole(1), rot) else { return };
                // (what `check` let through is `0 <= k <= n <= 64`; a rotation is taken modulo the steps, as the engine does)
                if n < 1 || n > crate::meter::MAX_EUCLID_STEPS || k < 0 || k > n {
                    return;
                }
                let shape = Shape { steps: n as u32, pulses: k as u32, rotation: rot.rem_euclid(n) as u32 };
                *euclids.entry(shape).or_insert(0) += 1;
            });
        }
    }
    Structure { placements: lw.placements.clone(), euclids }
}

// ---- the JSON-LD document

fn share_iri(cid: &str) -> String {
    format!("{}#{cid}", crate::share::BASE)
}

/// Where the program's `@context` is published: a stable, versioned URL (a change to the vocabulary is a new version,
/// never an edit in place), served as `application/ld+json` with CORS, the way https://pflow.xyz/schema is. The
/// document a program gets names this URL; [`context`] is the definition it serves, and `tests/ext_linked.rs` keeps the
/// copy the site embeds (`docs/btw-context-v1.jsonld`) equal to it.
pub const CONTEXT_URL: &str = "https://beatscript.app/context/v1.jsonld";

/// The `@context` of a program's document, as the definition [`CONTEXT_URL`] serves: `{"@context": {..}}`.
pub fn context_document() -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs([("@context", context())]))
}

/// The terms of the `@context`.
pub fn context() -> JsonValue {
    let term = |iri: &str, ty_id: bool, container: Option<&str>| {
        let mut o = JsonObject::from_pairs([("@id", JsonValue::str(iri))]);
        if ty_id {
            o.set("@type", JsonValue::str("@id"));
        }
        if let Some(c) = container {
            o.set("@container", JsonValue::str(c));
        }
        JsonValue::Object(o)
    };
    let mut c = JsonObject::new();
    c.set("bw", JsonValue::str(BW_NS));
    c.set("djrust", JsonValue::str(DJRUST_NS));
    c.set("dct", JsonValue::str("http://purl.org/dc/terms/"));
    c.set("author", JsonValue::str("dct:creator"));
    c.set("bars", JsonValue::str("djrust:bars"));
    c.set("cid", JsonValue::str("djrust:cid"));
    c.set("credit", term("djrust:credit", false, Some("@set")));
    c.set("euclid", term("djrust:euclid", false, Some("@set")));
    c.set("forkedFrom", term("djrust:forkedFrom", true, None));
    c.set("genre", term("djrust:genre", true, None));
    c.set("key", JsonValue::str("djrust:key"));
    c.set("license", term("dct:license", true, None));
    c.set("licenseId", JsonValue::str("djrust:licenseId"));
    c.set("name", JsonValue::str("dct:title"));
    c.set("sameShapeAs", term("djrust:sameShapeAs", true, Some("@set")));
    c.set("sections", term("djrust:section", false, Some("@list")));
    c.set("start", JsonValue::str("djrust:start"));
    c.set("tag", term("djrust:tag", true, Some("@set")));
    c.set("tempo", JsonValue::str("djrust:tempo"));
    c.set("uses", JsonValue::str("djrust:uses"));
    c.set("visibility", JsonValue::str("djrust:visibility"));
    JsonValue::Object(c)
}

/// What the document says besides the program itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts<'a> {
    pub name: Option<&'a str>,
    /// None: the document makes no claim about rights (a program that was never saved).
    pub rights: Option<&'a Rights>,
    pub links: Option<&'a Links>,
}

/// A section of the song as the document lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionFact {
    pub name: String,
    /// The bar it starts on.
    pub start: u32,
    pub bars: u32,
    /// The key it sets itself (None: it sets none).
    pub key: Option<crate::music::Key>,
}

/// The sections of a compiled song, one for each time the arrangement places one, in timeline order (by start bar; two
/// stacked sections start on one bar, in the order the arrangement wrote them). They are the song's sections, not its
/// stretches: `stack(drums, wash)` lists `drums` and `wash`, each with its own length, and never a `drums+wash` (a
/// stack cuts the song where its parts start and end; the compiled segments are those cuts). A section that plays in
/// several stretches, a net chained past 64 bars or an `evolve`'s generations is still one section with its bars as
/// written. A song with no sections lists none; one the placements do not cover (a bare `evolve`, a program chained
/// from nets of 64 bars) lists its segments, a chain as one.
pub fn sections(c: &Compiled) -> Vec<SectionFact> {
    if !c.arranged {
        return Vec::new();
    }
    if !c.linked.placements.is_empty() {
        let mut out: Vec<(usize, &Placed)> = c.linked.placements.iter().enumerate().collect();
        out.sort_by_key(|(i, p)| (p.start, *i));
        return out.into_iter().map(|(_, p)| SectionFact { name: p.name.clone(), start: p.start, bars: p.bars, key: p.key }).collect();
    }
    let mut out: Vec<SectionFact> = Vec::new();
    for s in &c.segments {
        match out.last_mut() {
            Some(last) if s.chain => last.bars += s.bars,
            _ => out.push(SectionFact { name: s.name.clone(), start: s.start, bars: s.bars, key: None }),
        }
    }
    out
}

/// A program as linked data. [`pretty`] sorts the keys, so the same facts are the same text. Nothing in it is a
/// time, a random number or a place on this machine.
pub fn jsonld(c: &Compiled, f: &Facts, onto: Option<&dyn Ontology>) -> JsonValue {
    let mut d = jsonld_inline(c, f, onto);
    if let JsonValue::Object(o) = &mut d {
        o.set("@context", JsonValue::str(CONTEXT_URL));
    }
    d
}

/// [`jsonld`] with the terms spelled out in `@context` instead of named by URL: for a reader that is offline (`btw jsonld --inline`).
pub fn jsonld_inline(c: &Compiled, f: &Facts, onto: Option<&dyn Ontology>) -> JsonValue {
    let mut d = JsonObject::new();
    d.set("@context", context());
    d.set("@id", JsonValue::str(share_iri(&c.cid)));
    d.set("@type", JsonValue::str("djrust:Program"));
    d.set("cid", JsonValue::str(c.cid.clone()));
    if let Some(n) = f.name.filter(|n| !n.is_empty()) {
        d.set("name", JsonValue::str(n));
    }
    if let Some(l) = f.links {
        if let Some(g) = &l.genre {
            d.set("genre", JsonValue::str(g.clone()));
        }
        if !l.tags.is_empty() {
            d.set("tag", JsonValue::Array(l.tags.iter().cloned().map(JsonValue::str).collect()));
        }
    }
    if let Some(k) = c.key {
        d.set("key", JsonValue::str(crate::music::canonical_pitch_word(&k.text())));
    }
    if let Some(t) = c.tempo {
        d.set("tempo", JsonValue::Number(t));
    }
    let secs = sections(c);
    if !secs.is_empty() {
        let nodes = secs.into_iter().map(|x| {
            let mut o = JsonObject::from_pairs([("@type", JsonValue::str("djrust:Section")), ("name", JsonValue::str(x.name)), ("start", JsonValue::int(i64::from(x.start))), ("bars", JsonValue::int(i64::from(x.bars)))]);
            if let Some(k) = x.key {
                o.set("key", JsonValue::str(crate::music::canonical_pitch_word(&k.text())));
            }
            JsonValue::Object(o)
        });
        d.set("sections", JsonValue::Array(nodes.collect()));
    }
    if let Some(r) = f.rights {
        d.set("licenseId", JsonValue::str(r.license.id()));
        if let Some(u) = r.license.url() {
            d.set("license", JsonValue::str(u));
        }
        d.set("visibility", JsonValue::str(r.visibility.id()));
        if let Some(a) = &r.author {
            d.set("author", JsonValue::str(a.clone()));
        }
        if let Some(p) = &r.forked_from {
            d.set("forkedFrom", JsonValue::str(share_iri(p)));
        }
        if !r.credits.is_empty() {
            let credits = r.credits.iter().map(|cr| {
                let mut o = JsonObject::new();
                o.set("@id", JsonValue::str(share_iri(&cr.cid)));
                o.set("@type", JsonValue::str("djrust:Program"));
                o.set("cid", JsonValue::str(cr.cid.clone()));
                if let Some(n) = &cr.name {
                    o.set("name", JsonValue::str(n.clone()));
                }
                if let Some(a) = &cr.author {
                    o.set("author", JsonValue::str(a.clone()));
                }
                o.set("licenseId", JsonValue::str(cr.license.id()));
                if let Some(u) = cr.license.url() {
                    o.set("license", JsonValue::str(u));
                }
                JsonValue::Object(o)
            });
            d.set("credit", JsonValue::Array(credits.collect()));
        }
    }
    if !c.linked.euclids.is_empty() {
        let class = match onto.and_then(|o| o.euclid_class()) {
            Some(c) if c != EUCLID_CLASS => c.to_string(),
            _ => "bw:EuclideanRhythm".to_string(),
        };
        let nodes = c.linked.euclids.iter().map(|(s, uses)| {
            let mut o = JsonObject::new();
            o.set("@type", JsonValue::str(class.clone()));
            o.set("bw:pulses", JsonValue::int(i64::from(s.pulses)));
            o.set("bw:steps", JsonValue::int(i64::from(s.steps)));
            o.set("bw:rotation", JsonValue::int(i64::from(s.rotation)));
            o.set("uses", JsonValue::int(i64::from(*uses)));
            // the rhythms the ontology names with this shape: F6's table (`meter::EUCLID_TABLE`, the published individuals),
            // and any more the host's vocabulary has. The role is not asked: E(3,8) is one pattern whatever plays it.
            let named = crate::meter::euclid_names(i64::from(s.pulses), i64::from(s.steps), i64::from(s.rotation));
            let mut same: Vec<&str> = named.iter().map(|t| t.iri).collect();
            if let Some(onto) = onto {
                same.extend(onto.terms().iter().filter(|t| t.shape == Some(*s)).map(|t| t.iri.as_str()));
            }
            same.sort();
            same.dedup();
            // a name only when the ontology has one rhythm of the shape ("house snare"); six genres share E(4,16)
            if let [only] = named.as_slice() {
                o.set("name", JsonValue::str(only.label));
            }
            if !same.is_empty() {
                o.set("sameShapeAs", JsonValue::Array(same.into_iter().map(JsonValue::str).collect()));
            }
            JsonValue::Object(o)
        });
        d.set("euclid", JsonValue::Array(nodes.collect()));
    }
    JsonValue::Object(d)
}

/// A document as text: keys sorted (UTF-16 order, as `canonical_js`), two-space indent, `"key": value`, no newline
/// at the end. The same facts are the same bytes.
pub fn pretty(v: &JsonValue) -> String {
    fn go(v: &JsonValue, depth: usize, out: &mut String) {
        let pad = |n: usize| "  ".repeat(n);
        match v {
            JsonValue::Array(a) if !a.is_empty() => {
                out.push_str("[\n");
                for (i, x) in a.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    go(x, depth + 1, out);
                    out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            JsonValue::Object(o) if !o.is_empty() => {
                let mut keys: Vec<&str> = o.keys().collect();
                keys.sort_by(|a, b| utf16_cmp(a, b));
                out.push_str("{\n");
                for (i, k) in keys.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&JsonValue::str(*k).js_stringified());
                    out.push_str(": ");
                    go(o.get(k).unwrap(), depth + 1, out);
                    out.push_str(if i + 1 < keys.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
            other => out.push_str(&other.js_stringified()),
        }
    }
    let mut out = String::new();
    go(v, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_by_case_and_separator() {
        assert_eq!(norm("  Four  on_Floor-"), "four-on-floor");
        assert_eq!(norm("pre - chorus"), "pre-chorus");
        assert_eq!(norm(""), "");
        assert_eq!(norm("808"), "808");
    }

    #[test]
    fn the_header_block_is_the_leading_run_of_bang_lines() {
        assert_eq!(header_end(""), 0);
        assert_eq!(header_end("tempo 1\n#! genre: x\n"), 0);
        assert_eq!(header_end("#! a: 1\n#! b: 2\ntempo 1\n"), "#! a: 1\n#! b: 2\n".len());
        assert_eq!(header_end("#! a: 1"), 7);
        assert_eq!(header_lines("# not a header\n#! genre: x\n"), Vec::new());
        // a line with no colon is skipped, not the end of the block
        let h = header_lines("#! btw 1\n#! genre: x\n");
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].key.as_str(), h[0].value.as_str()), ("genre", "x"));
    }

    #[test]
    fn a_snapshot_that_is_not_one_is_refused_with_a_reason() {
        assert!(StaticOntology::from_json("not json").unwrap_err().contains("not JSON"));
        assert!(StaticOntology::from_json("{}").unwrap_err().contains("no `terms`"));
        assert!(StaticOntology::from_json(r#"{"terms":[{"iri":"x"}]}"#).unwrap_err().contains("no `kind`"));
        let o = StaticOntology::from_json(r#"{"version":"9","verified":true,"terms":[{"iri":"https://ontology.bitwrap.io/genre/b","kind":"genre","local":"b","label":"b"},{"iri":"https://ontology.bitwrap.io/genre/a","kind":"genre","local":"a","label":"a"}]}"#).unwrap();
        assert_eq!(o.terms().iter().map(|t| t.local.as_str()).collect::<Vec<_>>(), ["a", "b"], "sorted by IRI");
        assert_eq!((o.version.as_str(), o.verified, o.euclid_class()), ("9", true, None));
    }

    #[test]
    fn tags_split_on_commas_and_drop_the_empty() {
        assert_eq!(split_tags(" a , b ,, c,"), ["a", "b", "c"]);
        assert!(split_tags("  ").is_empty());
    }

    #[test]
    fn links_are_a_set() {
        let l = Links::new(Some(String::new()), vec!["b".into(), "a".into(), "b".into()]);
        assert_eq!((l.genre, l.tags), (None, vec!["a".to_string(), "b".to_string()]));
    }
}
