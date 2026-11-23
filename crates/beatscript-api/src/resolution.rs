//! What a program's references resolve to, as `check`, `listen`, `generate` and `genre_template` answer it (project-datum
//! 6668, 6680). A program's CID is of its canonical text: the *recipe*, with its `gen@2 ...`, `seed(...)`, `template(...)` and
//! `<cid>.drop` references as written. Three fields say what the recipe makes:
//!
//! - `refs`: each reference, where it stands and what it resolved to ([`djlive::refs::sites`]): `kind` (gen, seed, stored,
//!   template), `half` (engine or structure, docs/BEATSCRIPT.md "Two halves"), `call`, `place` (part, let, section, arrange),
//!   `name`, `line`/`col`, `resolved` (the CID it is pinned to), `generator`, `fitted` (what a `gen` call took from the
//!   program: key, bpm) and `track`; with `expand`, each one's `expansion` (the BeatScript it is written out as there).
//! - `expanded_cid`: the CID of the program with every reference written out ([`djlive::stored::expand_text`], canonical):
//!   the *result*. The same recipe always gives the same one; a program with no references has `expanded_cid == cid`.
//! - `resolution`: the manifest, each distinct reference once: the CID it is pinned to and the generator (and version) that
//!   made it; a `gen` call also says the key and tempo it was generated in. A reference written out inside another one's
//!   expansion (a template's) is listed `nested`, with its own pin (a gen call a role wrote is resolved in the key and tempo of
//!   the written-out program). A template's entry has `parts` ([`djlive::nested`], roadmap 6680 item 2): each role it binds, at
//!   any depth, with the CID of what it is written out as.
//!
//! The descriptions the MCP tools give of these fields are [`REFS_DOC`], [`EXPANDED_CID_DOC`], [`RESOLUTION_DOC`] and
//! [`PARTS_DOC`], and docs/BEATSCRIPT.md says them in the same words (beatscript-serve's exposure test holds the two together).

use djengine::json::{JsonObject, JsonValue};
use djlive::env::Env;
use djlive::refs::Site;

use crate::report::{int, opt_str, strs};

/// What `refs` is, in the words of the docs and the MCP descriptions.
pub const REFS_DOC: &str = "refs lists each reference of the program (a gen@N call, a seed(...), a template(...), a <cid>.section borrowed from a stored program): its kind, half (engine or structure), call, place (part, let, section or arrange), name, line and col, resolved (the CID it is pinned to), generator, fitted (what a gen call took from the program: key, bpm) and track";
/// What `expanded_cid` is.
pub const EXPANDED_CID_DOC: &str = "expanded_cid is the CID of the program with every reference written out as plain BeatScript: cid addresses the recipe, expanded_cid the result; the same recipe always gives the same expanded_cid, and a program with no references has expanded_cid equal to cid";
/// What `resolution` is.
pub const RESOLUTION_DOC: &str = "resolution is the manifest: each reference once, with the CID it is pinned to and the generator and version that made it (seeded@2), and for a gen call the key and bpm it was generated in";
/// What a template's `parts` are (roadmap 6680 item 2).
pub const PARTS_DOC: &str = "a template's entry in resolution has parts: each role its reference binds, at any depth (a role of a form bound to a role is a part of that part), as {path, role, kind, call, cid, resolves, generator, parts}: path is where it stands (episodes/ground), kind what it was bound to (gen, stored, theme, notes or template), call the source as bound, cid the CID of what it is written out as (a program of its own: the key and tempo it plays in and the lines its binding wrote, every reference in them written out), resolves the CID its source is pinned to; the same recipe always gives the same parts and part CIDs";

/// What `imports` is.
pub const IMPORTS_DOC: &str = "imports lists each `use SOURCE as NAME` line of the program: its name, what the name is (is: a section, a line, a source for a role or a phrase), from (the source, in canonical spelling), source (its kind: stored, gen, template, form, theme, phrase or genre), resolved (what it is pinned to: a CID, or the library's CID for a theme or a phrase), line and col, and what (a sentence)";

/// A program's references, resolved: what the three fields are made of.
#[derive(Clone, Debug, Default)]
pub struct References {
    /// The references written in the program, in order.
    pub sites: Vec<Site>,
    /// The program with every reference written out, when it has any.
    pub expanded: Option<String>,
    /// The CID of [`References::expanded`] (canonical), or the program's own CID when it has no reference.
    pub expanded_cid: String,
    /// The same expansion's CID in the other version of the language (`djlive::compile::renamed_cid`: a template's written-out
    /// program is pinned by its BeatScript 1 CID, a BeatScript 2 recipe's expansion has its BeatScript 2 one); empty with none.
    pub expanded_twin: String,
    /// Every reference written out, in the order the stages wrote them (templates, seeds, gen calls, stored programs): the
    /// program's own and those inside another's expansion.
    pub written: Vec<String>,
    /// Its `use SOURCE as NAME` lines, resolved (djlive::uses).
    pub imports: Vec<djlive::uses::Import>,
    /// The nested parts of its template's expansion ([`djlive::nested::parts`]): empty with no template, or no role bound.
    pub parts: Vec<djlive::nested::Part>,
}

impl References {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty() && self.written.is_empty() && self.imports.is_empty()
    }
}

/// The references of `text`, a program that checks with CID `cid` (its recipe CID).
pub fn of(text: &str, cid: &str) -> References {
    let env = crate::web_env().env();
    of_with(text, cid, &env)
}

/// [`of`] against `env`.
pub fn of_with(text: &str, cid: &str, env: &Env) -> References {
    let (expanded, written) = match djlive::stored::expand_text(text, env) {
        Some(Ok((body, refs, _))) => (Some(body), refs),
        _ => (None, Vec::new()),
    };
    let canonical = expanded.as_ref().map(|body| djlive::print::canonical(&djlive::parse::parse_with(body, env).program));
    let expanded_cid = canonical.as_deref().map_or_else(|| cid.to_string(), djlive::compile::program_cid);
    let expanded_twin = canonical.as_deref().map(djlive::compile::renamed_cid).unwrap_or_default();
    let sites = if expanded.is_some() { djlive::refs::sites(text, env).unwrap_or_default() } else { Vec::new() };
    let imports = if expanded.is_some() { djlive::uses::imports(text, env).unwrap_or_default() } else { Vec::new() };
    let parts = if sites.iter().any(|s| s.kind == "template") { djlive::nested::parts(text, env) } else { Vec::new() };
    References { sites, expanded, expanded_cid, expanded_twin, written, imports, parts }
}

/// `imports`: each `use` line of `text`, its name and its source (see [`IMPORTS_DOC`]).
pub fn imports_json(imports: &[djlive::uses::Import], text: &str) -> JsonValue {
    JsonValue::Array(
        imports
            .iter()
            .map(|i| {
                let (line, col) = djlive::diag::line_col(text, i.line.start);
                let mut o = JsonObject::new();
                o.set("name", JsonValue::str(i.name.clone()));
                o.set("is", JsonValue::str(i.makes.name()));
                o.set("from", JsonValue::str(i.call.clone()));
                o.set("source", JsonValue::str(i.source.kind()));
                o.set("resolved", opt_str(i.resolved.clone()));
                o.set("generator", opt_str(i.generator.clone()));
                o.set("line", int(line));
                o.set("col", int(col));
                o.set("what", JsonValue::str(i.what.clone()));
                JsonValue::Object(o)
            })
            .collect(),
    )
}

/// The generator a reference's kind names: a `gen` call's own (`seeded@2`), the beats generator's seeds for a `seed(...)`.
fn generator_of(s: &Site) -> Option<String> {
    match s.kind {
        "gen" | "use" => s.generator.clone(),
        "seed" => Some(djlive::seed::generator_of(&s.call)),
        _ => None,
    }
}

/// `refs`: each reference of `text` (see the module); with `expand`, each one's `expansion`.
pub fn refs_json(r: &References, text: &str, expand: bool) -> JsonValue {
    JsonValue::Array(
        r.sites
            .iter()
            .map(|s| {
                let (line, col) = djlive::diag::line_col(text, s.span.start);
                let mut o = JsonObject::new();
                o.set("kind", JsonValue::str(s.kind));
                o.set("half", JsonValue::str(s.half));
                o.set("call", JsonValue::str(s.call.clone()));
                o.set("place", JsonValue::str(s.place));
                o.set("name", opt_str(s.name.clone()));
                o.set("line", int(line));
                o.set("col", int(col));
                o.set("start", int(s.span.start));
                o.set("end", int(s.span.end));
                o.set("resolved", opt_str(s.resolved.clone()));
                o.set("generator", opt_str(generator_of(s)));
                o.set("fitted", strs(s.fitted.iter().cloned()));
                o.set("track", opt_str(s.track.clone()));
                if expand {
                    o.set("expansion", JsonValue::str(s.text.clone()));
                }
                JsonValue::Object(o)
            })
            .collect(),
    )
}

/// The kind of a reference as a stage writes it (`template(...)`, `seed(...)`, `gen@2 ...`, `<cid>.drop`).
fn kind_of_call(call: &str) -> &'static str {
    if call.starts_with("template(") {
        "template"
    } else if call.starts_with("seed(") || call.starts_with("seed@") {
        "seed"
    } else if djlive::gen::is_call_word(call) {
        "gen"
    } else {
        "stored"
    }
}

/// A template's nested parts (see [`PARTS_DOC`]); `with_text` adds each one's `text`.
pub fn parts_json(parts: &[djlive::nested::Part], with_text: bool) -> JsonValue {
    JsonValue::Array(
        parts
            .iter()
            .map(|p| {
                let mut o = JsonObject::new();
                o.set("path", JsonValue::str(p.path.clone()));
                o.set("role", JsonValue::str(p.role.clone()));
                o.set("kind", JsonValue::str(p.kind));
                o.set("call", JsonValue::str(p.call.clone()));
                o.set("cid", JsonValue::str(p.cid.clone()));
                o.set("resolves", opt_str(p.resolves.clone()));
                o.set("generator", opt_str(p.generator.clone()));
                if with_text {
                    o.set("text", JsonValue::str(p.text.clone()));
                }
                o.set("parts", parts_json(&p.parts, with_text));
                JsonValue::Object(o)
            })
            .collect(),
    )
}

/// `resolution`: each distinct reference once, its pin and its generator (see the module). `text` is the program, for a
/// `gen` call's host key and tempo.
pub fn resolution_json(r: &References, text: &str) -> JsonValue {
    resolution_json_with(r, text, false)
}

/// [`resolution_json`]; `with_text` gives each of a template's parts its `text` (check's `expand`).
pub fn resolution_json_with(r: &References, text: &str, with_text: bool) -> JsonValue {
    let host = djlive::gen::host_of(&djlive::lex::lex(text).tokens);
    let mut out: Vec<JsonValue> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for s in &r.sites {
        if seen.contains(&s.call) {
            continue;
        }
        seen.push(s.call.clone());
        let mut o = JsonObject::new();
        o.set("call", JsonValue::str(s.call.clone()));
        o.set("kind", JsonValue::str(s.kind));
        o.set("cid", opt_str(s.resolved.clone()));
        o.set("generator", opt_str(generator_of(s)));
        match s.kind {
            "gen" => {
                // the request it was generated from: its own params, the program's key and tempo where it says none
                if let Ok(res) = djlive::gen::resolve_word(&s.call, &host) {
                    o.set("key", if res.request.key.is_empty() { JsonValue::Null } else { JsonValue::str(res.request.key.clone()) });
                    o.set("bpm", res.request.bpm.map_or(JsonValue::Null, JsonValue::Number));
                }
                o.set("fitted", strs(s.fitted.iter().cloned()));
            }
            "seed" => o.set("model", opt_str(s.model.clone())),
            _ => {}
        }
        o.set("nested", JsonValue::Bool(false));
        if s.kind == "template" {
            o.set("parts", parts_json(&r.parts, with_text));
        }
        out.push(JsonValue::Object(o));
    }
    // what another reference's expansion held, written out in turn: its call and its pin. A gen call a template's role wrote is
    // resolved in the key and tempo the written-out program says at its top; a gen call on a part line or a section is written as
    // a reference to the program it resolved to (`<cid>.drop`), and that one is the call's own, so it is not listed again
    let mut pinned: Vec<String> = r.sites.iter().filter(|s| s.kind == "gen").filter_map(|s| s.resolved.clone()).collect();
    let written_host = r.expanded.as_deref().map(|b| djlive::gen::host_of(&djlive::lex::lex(b).tokens)).unwrap_or_else(|| host.clone());
    for call in &r.written {
        if seen.contains(call) || call.split('.').next().is_some_and(|c| pinned.iter().any(|p| p == c)) {
            continue;
        }
        seen.push(call.clone());
        let kind = kind_of_call(call);
        let mut o = JsonObject::new();
        o.set("call", JsonValue::str(call.clone()));
        o.set("kind", JsonValue::str(kind));
        match kind {
            "gen" => match djlive::gen::resolve_word(call, &written_host) {
                Ok(res) => {
                    o.set("cid", JsonValue::str(res.cid.clone()));
                    o.set("generator", JsonValue::str(res.generator.clone()));
                    o.set("key", if res.request.key.is_empty() { JsonValue::Null } else { JsonValue::str(res.request.key.clone()) });
                    o.set("bpm", res.request.bpm.map_or(JsonValue::Null, JsonValue::Number));
                    o.set("fitted", strs(res.fitted.iter().map(|f| f.to_string())));
                    pinned.push(res.cid);
                }
                Err(_) => {
                    o.set("cid", JsonValue::Null);
                    o.set("generator", JsonValue::Null);
                }
            },
            "stored" => {
                o.set("cid", opt_str(call.split('.').next().filter(|c| djlive::stored::is_cid(c)).map(str::to_string)));
                o.set("generator", JsonValue::Null);
            }
            _ => {
                o.set("cid", JsonValue::Null);
                o.set("generator", JsonValue::Null);
            }
        }
        o.set("nested", JsonValue::Bool(true));
        out.push(JsonValue::Object(o));
    }
    JsonValue::Array(out)
}

/// What an expansion is based on: each reference the recipe wrote out into it, once, with its pin where it has one, as the
/// `"<call> -> <cid>"` lines a store keeps beside a program (`meta.based_on`, read back as `# from:` lines): the manifest's
/// entries, then the parts of a template that are no reference of their own (a theme, notes, a form). A pin that is the
/// expansion itself (a template with nothing to write out after it: its written-out program is the whole expansion) is left
/// off: a program is never based on itself.
pub fn sources(r: &References, text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // the expansion itself, in either version (a BeatScript 2 recipe's template is pinned by the BeatScript 1 CID of what it
    // writes, which is this expansion's twin): never a source of itself
    let own = [r.expanded_cid.clone(), r.expanded_twin.clone()];
    let mut add = |call: &str, cid: Option<&str>| {
        let cid = cid.filter(|c| !own.iter().any(|o| o == c));
        let line = match cid {
            Some(c) => format!("{call} -> {c}"),
            None => call.to_string(),
        };
        if !call.is_empty() && !out.contains(&line) {
            out.push(line);
        }
    };
    if let JsonValue::Array(a) = resolution_json(r, text) {
        for e in &a {
            add(e.get("call").and_then(JsonValue::as_str).unwrap_or(""), e.get("cid").and_then(JsonValue::as_str));
        }
    }
    for p in djlive::nested::flatten(&r.parts) {
        if !matches!(p.kind, "gen" | "stored") {
            add(&p.call, None);
        }
    }
    out
}

/// The fields a report gains for a program that checks with CID `cid`: `expanded_cid` always, and `refs` and `resolution` when
/// it has references (so every other report is as it was, but for `expanded_cid`).
pub fn fields(text: &str, cid: &str, expand: bool) -> Vec<(&'static str, JsonValue)> {
    let r = of(text, cid);
    let mut out = vec![("expanded_cid", JsonValue::str(r.expanded_cid.clone()))];
    if !r.is_empty() {
        out.push(("refs", refs_json(&r, text, expand)));
        out.push(("resolution", resolution_json_with(&r, text, expand)));
    }
    if !r.imports.is_empty() {
        out.push(("imports", imports_json(&r.imports, text)));
    }
    out
}

/// `{expanded_cid, refs, resolution}` of a program as one JSON object string: what beatscript's `listen` and beatscript-serve's `generate`
/// and `genre_template` add to their answers. A program that does not check answers `{}`.
pub fn fields_json(text: &str) -> String {
    let env = crate::web_env().env();
    let Some(c) = djlive::compile::check_with(text, &env).compiled else { return "{}".to_string() };
    JsonValue::Object(JsonObject::from_pairs(fields(text, &c.cid, false))).js_stringified()
}
