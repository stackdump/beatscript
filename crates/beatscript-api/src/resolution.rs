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
//!   expansion (a template's) is listed `nested`, with the CID where the call names it.
//!
//! The descriptions the MCP tools give of these fields are [`REFS_DOC`], [`EXPANDED_CID_DOC`] and [`RESOLUTION_DOC`], and
//! docs/BEATSCRIPT.md says them in the same words (beatscript-serve's exposure test holds the two together).

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

/// A program's references, resolved: what the three fields are made of.
#[derive(Clone, Debug, Default)]
pub struct References {
    /// The references written in the program, in order.
    pub sites: Vec<Site>,
    /// The program with every reference written out, when it has any.
    pub expanded: Option<String>,
    /// The CID of [`References::expanded`] (canonical), or the program's own CID when it has no reference.
    pub expanded_cid: String,
    /// Every reference written out, in the order the stages wrote them (templates, seeds, gen calls, stored programs): the
    /// program's own and those inside another's expansion.
    pub written: Vec<String>,
}

impl References {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty() && self.written.is_empty()
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
    let expanded_cid = match &expanded {
        Some(body) => djlive::compile::program_cid(&djlive::print::canonical(&djlive::parse::parse_with(body, env).program)),
        None => cid.to_string(),
    };
    let sites = if expanded.is_some() { djlive::refs::sites(text, env).unwrap_or_default() } else { Vec::new() };
    References { sites, expanded, expanded_cid, written }
}

/// The generator a reference's kind names: a `gen` call's own (`seeded@2`), the beats generator's seeds for a `seed(...)`.
fn generator_of(s: &Site) -> Option<String> {
    match s.kind {
        "gen" => s.generator.clone(),
        "seed" => Some(format!("seeds@{}", djlive::seed::VERSION)),
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
    } else if call.starts_with("seed(") {
        "seed"
    } else if djlive::gen::is_call_word(call) {
        "gen"
    } else {
        "stored"
    }
}

/// `resolution`: each distinct reference once, its pin and its generator (see the module). `text` is the program, for a
/// `gen` call's host key and tempo.
pub fn resolution_json(r: &References, text: &str) -> JsonValue {
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
        out.push(JsonValue::Object(o));
    }
    // what another reference's expansion held, written out in turn: its call, and its pin where the call names it (a gen call on
    // a part line or a section is written as a reference to the program it resolved to, `<cid>.drop`: that one is the call's own)
    let pinned: Vec<&str> = r.sites.iter().filter(|s| s.kind == "gen").filter_map(|s| s.resolved.as_deref()).collect();
    for call in &r.written {
        if seen.contains(call) || call.split('.').next().is_some_and(|c| pinned.contains(&c)) {
            continue;
        }
        seen.push(call.clone());
        let kind = kind_of_call(call);
        let cid = match kind {
            "stored" => call.split('.').next().filter(|c| djlive::stored::is_cid(c)).map(str::to_string),
            _ => None,
        };
        let mut o = JsonObject::new();
        o.set("call", JsonValue::str(call.clone()));
        o.set("kind", JsonValue::str(kind));
        o.set("cid", opt_str(cid));
        o.set("generator", JsonValue::Null);
        o.set("nested", JsonValue::Bool(true));
        out.push(JsonValue::Object(o));
    }
    JsonValue::Array(out)
}

/// The fields a report gains for a program that checks with CID `cid`: `expanded_cid` always, and `refs` and `resolution` when
/// it has references (so every other report is as it was, but for `expanded_cid`).
pub fn fields(text: &str, cid: &str, expand: bool) -> Vec<(&'static str, JsonValue)> {
    let r = of(text, cid);
    let mut out = vec![("expanded_cid", JsonValue::str(r.expanded_cid.clone()))];
    if !r.is_empty() {
        out.push(("refs", refs_json(&r, text, expand)));
        out.push(("resolution", resolution_json(&r, text)));
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
