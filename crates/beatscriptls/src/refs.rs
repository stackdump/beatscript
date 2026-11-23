//! A program's references in the editor (roadmap 6668): a generator call (`gen@2 techno seed=42 section=drop part=bass`), a seed
//! (`seed(techno, 42).bass[1..8]`), a stored program's section or part (`<cid>.drop.bass`) and a template (`arrange from
//! template(...)`), as `djlive::refs` lists them. The editor can
//!
//! - **hover** one: which half of the language it is (an engine call or a structural import), its canonical call, the CID it
//!   resolves to, the generator and version, what it was fitted to (the program's key and tempo), and its expansion, cut short
//!   for a long one with a line saying how much was left out;
//! - **show or hide** its expansion inline: `beatscript/expand` answers the references with their ranges and what each is written out
//!   as, and a code lens over each says `show expansion` (command `beatscript.expand.toggle`) and `eject` (`beatscript.eject`). Which
//!   expansions are shown is the **client's** state: the server answers what a reference stands for and keeps nothing;
//! - **eject** one: write it out in place as local `let` / `section` text under its `# from: <call> -> <cid>` line, exactly as
//!   `djlive::refs::eject` writes it, offered as a `refactor.inline` code action made and proved on `codeAction/resolve` like
//!   the other refactors (the ejected program must play the same nets), and as the lens's command;
//! - **copy** one's expansion (`beatscript.expand.copy`): the text eject would write, `# from:` line first, with no edit.
//!
//! The references are worked out once per text ([`Analysis::refs`]): a generator call is generated and compiled to be written
//! out, so that is a few ms the first time a hover, a lens or the lightbulb asks, and nothing after.

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::{Diagnostic, Span};
use djlive::lex::Tok;
use djlive::refs::{Site, ENGINE};

use crate::analysis::Analysis;
use crate::fix::workspace_edit;
use crate::json::{arr, int, obj, pos_json, s, span_range};
use crate::refactor::Made;
use crate::text::Encoding;

/// Show or hide a reference's expansion: the server answers the reference (as one of `beatscript/expand`'s), the client toggles.
pub const TOGGLE: &str = "beatscript.expand.toggle";
/// The text a reference's eject would write (its `# from:` line first), for the client to put on the clipboard.
pub const COPY: &str = "beatscript.expand.copy";
/// Eject a reference: the edit, proved (the same nets); over stdio also sent as `workspace/applyEdit`.
pub const EJECT: &str = "beatscript.eject";
/// The commands `workspace/executeCommand` takes (`executeCommandProvider.commands`).
pub const COMMANDS: [&str; 3] = [TOGGLE, COPY, EJECT];

/// How much of an expansion a hover shows.
pub const HOVER_LINES: usize = 24;
pub const HOVER_CHARS: usize = 2400;

/// Is `w` the head of a generator call (`gen@2`; a bare `gen` is one only before a genre)?
pub fn is_gen_head(w: &str) -> bool {
    w == "gen" || w.strip_prefix("gen@").is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

/// Does the text have anything a reference starts with: `gen@N genre`, `seed(` (or `seed@N(`), `template(` or a CID? Its own tokens only, so a
/// program with none never pays for working them out.
pub fn may_have(a: &Analysis) -> bool {
    let t = &a.tokens;
    t.iter().enumerate().any(|(i, tok)| {
        let Tok::Word(w) = &tok.tok else { return false };
        let next = t.get(i + 1).map(|n| &n.tok);
        ((djlive::seed::head_version(w).is_some() || w == "template") && next == Some(&Tok::LParen)) || (is_gen_head(w) && matches!(next, Some(Tok::Word(_)))) || djlive::stored::is_ref_word(w)
    })
}

/// Every reference of the document, resolved; or what is wrong with one of them (at it).
pub fn sites(a: &Analysis) -> Result<&[Site], &[Diagnostic]> {
    a.refs()
}

/// The reference at byte `at`: the one whose own text holds it, else the one whose statement (what eject replaces) does.
pub fn site_at(a: &Analysis, at: usize) -> Option<&Site> {
    let v = sites(a).ok()?;
    v.iter().find(|s| s.span.start <= at && at <= s.span.end).or_else(|| v.iter().find(|s| s.replaced.start <= at && at <= s.replaced.end))
}

/// The reference whose own text (the call, not its statement) holds byte `at`.
pub fn site_on(a: &Analysis, at: usize) -> Option<&Site> {
    sites(a).ok()?.iter().find(|s| s.span.start <= at && at <= s.span.end)
}

/// What a reference is written out as, in place of what it replaces: its text without the `# from ...` line a section's or a
/// whole track's expansion opens with (the provenance line, [`djlive::refs::from_line`], says it).
pub fn expansion(s: &Site) -> &str {
    if s.text.starts_with("# from") {
        s.text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &s.text
    }
}

/// The text eject writes for a reference: its `# from:` line, then its expansion (what [`COPY`] answers).
pub fn copy_text(s: &Site) -> String {
    format!("{}\n{}", djlive::refs::from_line(s), expansion(s))
}

/// A long CID in a title: `z…8 chars`.
fn short_call(call: &str) -> String {
    let head = call.split(['.', ' ']).next().unwrap_or("");
    if djlive::stored::is_cid(head) {
        format!("{}{}", djengine::rights::short_cid(head), &call[head.len()..])
    } else {
        call.to_string()
    }
}

/// What a kind of reference is, in a phrase (a seed's says its version, read from its head: `seed@2(...)` is seeds v2, which
/// writes its genre's swing).
fn kind_doc(s: &Site) -> String {
    match s.kind {
        "gen" => "a call of the seeded generator".into(),
        "seed" => {
            let v = s.call.split('(').next().and_then(djlive::seed::head_version).unwrap_or(1);
            if v >= djlive::seed::SWING {
                format!("a track of the beats generator (seeds v{v}: the track and its genre's swing)")
            } else {
                format!("a track of the beats generator (seeds v{v})")
            }
        }
        "stored" => "a stored program's section or part, by its CID".into(),
        "template" => "a template, written out as a whole program".into(),
        _ => "a reference".into(),
    }
}

/// What a swung seed@2 brings where it stands: the whole track (`arrange from`) brings its swing, a piece of it its notes alone.
fn swing_doc(s: &Site) -> Option<String> {
    if s.kind != "seed" {
        return None;
    }
    let sw = djlive::seed::swing_of_call(&s.call)?;
    let n = djlive::print::num(sw);
    Some(if s.place == "arrange" {
        format!("writes `swing {n}` (unless the program says its own)")
    } else {
        format!("brings its notes here, not its swing: `swing` is a setting of the whole program, and only `arrange from` writes it; say `swing {n}` at the top for its genre's")
    })
}

/// Where a reference stands, in a phrase.
fn place_doc(s: &Site) -> String {
    let name = s.name.as_deref().unwrap_or("");
    match s.place {
        "let" => format!("the `let {name}`"),
        "section" => format!("the section `{name}`"),
        "arrange" => "the whole track (`arrange from`)".to_string(),
        _ if !name.is_empty() => format!("a `{name}` part"),
        _ => "a part".to_string(),
    }
}

/// What the program a reference is in says of its key and tempo (what a generator call is fitted to).
fn fitted_doc(a: &Analysis, s: &Site) -> Option<String> {
    if s.fitted.is_empty() {
        return None;
    }
    let host = djlive::gen::host_of(&a.tokens);
    let what: Vec<String> = s
        .fitted
        .iter()
        .map(|f| match f.as_str() {
            "key" => format!("the program's key `{}`", host.key.clone().unwrap_or_default()),
            "bpm" => format!("its tempo {}", host.tempo.map(djlive::print::num).unwrap_or_default()),
            other => format!("its `{other}`"),
        })
        .collect();
    Some(format!("fitted to {} (the call does not say its own)", what.join(" and ")))
}

/// The first lines of `text`, at most `lines` of them and `chars` characters, and how many lines were left out.
pub fn truncated(text: &str, lines: usize, chars: usize) -> (String, usize) {
    let all: Vec<&str> = text.split('\n').collect();
    let mut out = String::new();
    let mut n = 0;
    for l in &all {
        if n >= lines || (n > 0 && out.len() + l.len() > chars) {
            break;
        }
        if n > 0 {
            out.push('\n');
        }
        out.push_str(l);
        n += 1;
    }
    (out, all.len() - n)
}

/// The hover of a reference: what it is, what it resolves to, its expansion.
pub fn hover_markdown(a: &Analysis, s: &Site) -> String {
    let half = if s.half == ENGINE { "**engine call**: the text records the call; its result is pinned by the CID it resolves to" } else { "**structural import**: borrowed as source, written out as BeatScript before the program is lowered" };
    let mut m = format!("```beatscript\n{}\n```\n\n`{}`: {} · {half}", s.call, s.kind, kind_doc(s));
    let mut facts: Vec<String> = Vec::new();
    if let Some(cid) = &s.resolved {
        facts.push(format!("resolves to `{cid}`"));
    }
    if let Some(g) = &s.generator {
        facts.push(format!("generator `{g}`"));
    }
    if let Some(t) = &s.track {
        facts.push(format!("track *{t}*"));
    }
    if let Some(f) = fitted_doc(a, s) {
        facts.push(f);
    }
    if let Some(w) = swing_doc(s) {
        facts.push(w);
    }
    facts.push(format!("written out as {}", place_doc(s)));
    m.push_str("\n\n");
    m.push_str(&facts.join(" · "));
    let text = expansion(s);
    let total = text.split('\n').count();
    let (shown, left) = truncated(text, HOVER_LINES, HOVER_CHARS);
    m.push_str(&format!("\n\n**Expansion** ({total} line{}, what *Eject* writes under `# from:`):\n```beatscript\n{shown}\n```", if total == 1 { "" } else { "s" }));
    if left > 0 {
        m.push_str(&format!("\n\n… {left} more line{} not shown ({total} in all): eject it or show its expansion to see them", if left == 1 { "" } else { "s" }));
    }
    m
}

/// The hover of a `# from: <call> -> <cid>` line eject wrote: where the text came from.
pub fn from_line_markdown(line: &str) -> Option<String> {
    let (call, cid) = djlive::refs::read_from_line(line)?;
    let to = cid.map_or(String::new(), |c| format!(", which resolved to `{c}`"));
    Some(format!("```beatscript\n{call}\n```\n\nwritten out (ejected) from this reference{to}: the text below is the program's own now, and this line keeps where it came from (a comment: not in the CID)"))
}

/// One reference as `beatscript/expand` answers it.
pub fn site_json(a: &Analysis, site: &Site, enc: Encoding) -> JsonValue {
    let opt = |v: &Option<String>| v.as_ref().map_or(JsonValue::Null, |x| s(x.clone()));
    let text = expansion(site);
    let mut o = JsonObject::new();
    o.set("range", span_range(&a.index, &a.text, clamp(a, site.span), enc));
    o.set("replaced", span_range(&a.index, &a.text, clamp(a, site.replaced), enc));
    o.set("kind", s(site.kind));
    o.set("half", s(site.half));
    o.set("call", s(site.call.clone()));
    o.set("resolved", opt(&site.resolved));
    o.set("generator", opt(&site.generator));
    o.set("fitted", arr(site.fitted.iter().map(|f| s(f.clone())).collect()));
    o.set("place", s(site.place));
    o.set("name", opt(&site.name));
    o.set("text", s(text));
    o.set("from", s(djlive::refs::from_line(site)));
    o.set("track", opt(&site.track));
    o.set("model", opt(&site.model));
    o.set("lines", int(text.split('\n').count()));
    JsonValue::Object(o)
}

fn clamp(a: &Analysis, sp: Span) -> Span {
    Span::new(sp.start.min(a.text.len()), sp.end.min(a.text.len()))
}

/// `beatscript/expand`: every reference of the document, or the one at byte `at`. `{"references": [...]}`, with `"errors"` (LSP
/// diagnostics, at the references) when one of them cannot be worked out (then there are none).
pub fn expand(a: &Analysis, at: Option<usize>, enc: Encoding) -> JsonValue {
    match sites(a) {
        Ok(v) => {
            let list: Vec<JsonValue> = match at {
                Some(at) => site_at(a, at).map(|x| site_json(a, x, enc)).into_iter().collect(),
                None => v.iter().map(|x| site_json(a, x, enc)).collect(),
            };
            obj([("references", arr(list))])
        }
        Err(ds) => obj([("references", arr(Vec::new())), ("errors", arr(ds.iter().enumerate().map(|(i, d)| crate::diagnostics::to_lsp(a, d, i, enc)).collect()))]),
    }
}

/// The arguments a lens or an action hands a command: the document, the reference's start, and the text's length (an eject of
/// a text that has changed since is refused).
fn command_args(a: &Analysis, uri: &str, site: &Site, enc: Encoding) -> JsonValue {
    obj([("uri", s(uri)), ("position", pos_json(a.index.position(&a.text, site.span.start, enc))), ("length", int(a.text.len()))])
}

fn command(title: &str, name: &str, args: JsonValue) -> JsonValue {
    obj([("title", s(title)), ("command", s(name)), ("arguments", arr(vec![args]))])
}

/// `textDocument/codeLens`: `show expansion` and `eject` over each reference.
pub fn code_lenses(a: &Analysis, uri: &str, enc: Encoding) -> JsonValue {
    let Ok(v) = sites(a) else { return arr(Vec::new()) };
    let mut out = Vec::new();
    for site in v {
        let range = span_range(&a.index, &a.text, clamp(a, site.span), enc);
        let args = command_args(a, uri, site, enc);
        out.push(obj([("range", range.clone()), ("command", command("show expansion", TOGGLE, args.clone()))]));
        out.push(obj([("range", range), ("command", command("eject", EJECT, args))]));
    }
    arr(out)
}

/// The code actions for a reference at the start of `range`: eject it (unresolved: `data.refactor = "eject"`, made and proved
/// on `codeAction/resolve`), and copy its expansion (a command, no edit).
pub fn actions(a: &Analysis, uri: &str, range: Span, only: &[String], enc: Encoding) -> Vec<JsonValue> {
    let wants = |kind: &str| only.is_empty() || only.iter().any(|o| kind == o || kind.starts_with(&format!("{o}.")));
    if !wants("refactor.inline") || !may_have(a) {
        return Vec::new();
    }
    let Some(site) = site_at(a, range.start) else { return Vec::new() };
    let call = short_call(&site.call);
    let copy = format!("Copy the expansion of `{call}`");
    vec![
        obj([
            ("title", s(format!("Eject `{call}` into local text"))),
            ("kind", s("refactor.inline")),
            ("data", obj([("refactor", s("eject")), ("uri", s(uri)), ("start", int(site.span.start)), ("end", int(site.span.end)), ("length", int(a.text.len()))])),
        ]),
        obj([("title", s(copy.clone())), ("kind", s("refactor.inline")), ("command", command(&copy, COPY, command_args(a, uri, site, enc)))]),
    ]
}

/// The edit that makes `old` into `new`: the one stretch between their common start and their common end.
pub fn diff_edit(old: &str, new: &str) -> (Span, String) {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let mut p = ob.iter().zip(nb).take_while(|(x, y)| x == y).count();
    while !old.is_char_boundary(p) || !new.is_char_boundary(p) {
        p -= 1;
    }
    let max = (ob.len() - p).min(nb.len() - p);
    let mut q = ob.iter().rev().zip(nb.iter().rev()).take(max).take_while(|(x, y)| x == y).count();
    while !old.is_char_boundary(ob.len() - q) || !new.is_char_boundary(nb.len() - q) {
        q -= 1;
    }
    (Span::new(p, ob.len() - q), new[p..nb.len() - q].to_string())
}

/// The text with the reference at byte `at` ejected: `djlive::refs::eject_site` of the same site.
pub fn ejected(a: &Analysis, at: usize) -> Option<String> {
    let site = site_at(a, at)?;
    Some(djlive::refs::eject_site(&djlive::lex::normalise_input(&a.text), site))
}

/// Eject as a refactor: one edit, from the text to [`ejected`]. Proved by `refactor::verify` before it is given.
pub fn eject_made(a: &Analysis, at: usize) -> Option<Made> {
    let site = site_at(a, at)?;
    let new = djlive::refs::eject_site(&djlive::lex::normalise_input(&a.text), site);
    Some(Made { title: format!("Eject `{}` into local text", short_call(&site.call)), kind: "refactor.inline", edits: vec![diff_edit(&a.text, &new)] })
}

/// `workspace/executeCommand` for one of [`COMMANDS`] on the reference at byte `at` of an open document. `length`: the text's
/// length when the command was made (an eject of a text that has changed since is refused).
pub fn execute(a: &Analysis, uri: &str, name: &str, at: usize, length: Option<usize>, enc: Encoding) -> Result<JsonValue, String> {
    if let Err(ds) = sites(a) {
        let why = ds.first().map_or(String::new(), |d| format!(": {}", d.message));
        return Err(format!("the references cannot be worked out{why}"));
    }
    match name {
        TOGGLE => Ok(site_at(a, at).map_or(JsonValue::Null, |x| site_json(a, x, enc))),
        COPY => {
            let site = site_at(a, at).ok_or("no reference here")?;
            Ok(obj([("text", s(copy_text(site))), ("call", s(site.call.clone())), ("resolved", site.resolved.as_ref().map_or(JsonValue::Null, |c| s(c.clone())))]))
        }
        EJECT => {
            if length.is_some_and(|l| l != a.text.len()) {
                return Err("the document changed since the command was made: ask again".into());
            }
            let made = eject_made(a, at).ok_or("no reference here")?;
            crate::refactor::verify(a, &made)?;
            Ok(obj([("label", s(made.title.clone())), ("edit", workspace_edit(a, uri, &made.edits, enc))]))
        }
        _ => Err(format!("unknown command {name}: the commands are {}", COMMANDS.join(", "))),
    }
}

/// What a client with no show/hide of its own (LSP4IJ, a plain LSP client) is told when it runs [`TOGGLE`] or [`COPY`] over
/// stdio: the call, its CID and the first lines of its expansion, as a `window/showMessage`.
pub fn notice(command: &str, answer: &JsonValue) -> Option<String> {
    let text = answer.get("text").and_then(JsonValue::as_str)?;
    let call = answer.get("call").and_then(JsonValue::as_str).unwrap_or("");
    let cid = answer.get("resolved").and_then(JsonValue::as_str).map_or(String::new(), |c| format!(" -> {c}"));
    let (shown, left) = truncated(text, HOVER_LINES, HOVER_CHARS);
    let more = if left > 0 { format!("\n… {left} more lines") } else { String::new() };
    let head = if command == COPY { "the text eject would write (copy it from here)" } else { "its expansion" };
    Some(format!("{call}{cid}: {head}\n{shown}{more}"))
}
