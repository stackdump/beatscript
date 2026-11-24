//! The JSON the web and the CLI answer with (site/DESIGN.md, "JSON shapes"): `check`, `fmt`, `nets`,
//! `generation`. Built here once so the wasm module and `beatscript` cannot disagree.

use std::collections::BTreeSet;

use djengine::json::{JsonObject, JsonValue};
use djlive::assets::{self, RightsReport};
use djlive::compile::{check_with, Compiled};
use djlive::diag::{Diagnostic, Severity};

use crate::snapshot::{web_env, IMPORT_HELP, NO_IMPORT};
use crate::utf16::Utf16Index;

pub(crate) fn obj<const N: usize>(pairs: [(&str, JsonValue); N]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

pub(crate) fn strs<I: IntoIterator<Item = S>, S: Into<String>>(v: I) -> JsonValue {
    JsonValue::Array(v.into_iter().map(|s| JsonValue::str(s.into())).collect())
}

pub(crate) fn opt_str(s: Option<String>) -> JsonValue {
    s.map_or(JsonValue::Null, JsonValue::str)
}

pub(crate) fn opt_num(n: Option<f64>) -> JsonValue {
    n.map_or(JsonValue::Null, JsonValue::Number)
}

pub(crate) fn int(n: usize) -> JsonValue {
    JsonValue::int(n as i64)
}

/// The song's longhand net CIDs by deck, as `djctl btw nets` prints them. A deck whose song is longer than one net
/// (`MAX_BARS`) is chained nets, which has no one CID (the desktop prints none), so it is left out; null: no longhand form.
fn song_nets_json(c: &Compiled) -> JsonValue {
    match c.song_nets() {
        Ok(nets) => {
            let m: std::collections::BTreeMap<String, String> =
                nets.into_iter().filter(|(_, ir)| ir.period as u64 <= djlive::compile::MAX_BARS).map(|(d, ir)| (d.name().to_string(), ir.net_cid())).collect();
            str_map(&m)
        }
        Err(_) => JsonValue::Null,
    }
}

fn str_map<'a>(m: impl IntoIterator<Item = (&'a String, &'a String)>) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(m.into_iter().map(|(k, v)| (k.as_str(), JsonValue::str(v.clone()))).collect::<Vec<_>>()))
}

/// One diagnostic as JSON: `severity`, `line` and `col` (1-based, `col` in characters), `end_col`,
/// byte offsets `start`/`end` and UTF-16 offsets `start16`/`end16`, `message`, `expected`, `found`, `help`
/// and `render` (the caret form, for sending back to a model).
pub fn diagnostic_json(d: &Diagnostic, text: &str, ix: &Utf16Index) -> JsonValue {
    let (line, col, end_col) = d.position(text);
    let (start, end) = (d.span.start.min(text.len()), d.span.end.min(text.len()));
    let help = if d.message == NO_IMPORT { Some(IMPORT_HELP.to_string()) } else { d.help.clone() };
    obj([
        ("severity", JsonValue::str(d.severity.name())),
        ("line", int(line)),
        ("col", int(col)),
        ("end_col", int(end_col)),
        ("start", int(start)),
        ("end", int(end)),
        ("start16", int(ix.at(start))),
        ("end16", int(ix.at(end))),
        ("message", JsonValue::str(d.message.clone())),
        ("expected", strs(d.expected.iter().cloned())),
        ("found", JsonValue::str(d.found.clone())),
        ("help", opt_str(help)),
        ("render", JsonValue::str(render(d, text, help_override(d)))),
    ])
}

fn help_override(d: &Diagnostic) -> Option<&'static str> {
    (d.message == NO_IMPORT).then_some(IMPORT_HELP)
}

fn render(d: &Diagnostic, text: &str, help: Option<&str>) -> String {
    match help {
        None => d.render(text),
        Some(h) => {
            let mut d = d.clone();
            d.help = Some(h.to_string());
            d.render(text)
        }
    }
}

pub(crate) fn diagnostics_json(ds: &[Diagnostic], text: &str, ix: &Utf16Index) -> JsonValue {
    // errors and warnings in the order of the text, as the checker found them
    JsonValue::Array(ds.iter().map(|d| diagnostic_json(d, text, ix)).collect())
}

fn counts(ds: &[Diagnostic]) -> (usize, usize) {
    let errors = ds.iter().filter(|d| d.severity == Severity::Error).count();
    (errors, ds.len() - errors)
}

/// The sections of an arranged song; a section's later generations (and the stretches of one too long for a
/// net) are the same section.
/// Each section the arrange plays, where and for how long, with its doc comment (`doc`, the `#` lines directly above its
/// `section` line, djlive::doc) when it has one: a section without one has the entry it always had.
fn sections(c: &Compiled, docs: &[djlive::doc::DocComment]) -> JsonValue {
    let mut out: Vec<(String, u32, u32)> = Vec::new();
    if c.arranged && !c.flat_clock {
        // a segment with no `section` header is the program's own timeline (an evolve with no sections)
        for s in c.segments.iter().filter(|s| s.header.is_some()) {
            match out.last_mut() {
                Some(last) if s.chain => last.2 += s.bars,
                _ => out.push((s.name.clone(), s.start, s.bars)),
            }
        }
    }
    JsonValue::Array(
        out.into_iter()
            .map(|(n, s, b)| {
                let doc = djlive::doc::section_doc(docs, &n).map(JsonValue::str);
                let mut e = vec![("name", JsonValue::str(n)), ("start", JsonValue::int(i64::from(s))), ("bars", JsonValue::int(i64::from(b)))];
                if let Some(d) = doc {
                    e.push(("doc", d));
                }
                JsonValue::Object(JsonObject::from_pairs(e))
            })
            .collect(),
    )
}

/// The doc comments as `[{kind, name, doc}]`, in the order they are written.
pub fn docs_json(docs: &[djlive::doc::DocComment]) -> JsonValue {
    JsonValue::Array(docs.iter().map(|d| obj([("kind", JsonValue::str(d.kind)), ("name", JsonValue::str(d.name.clone())), ("doc", JsonValue::str(d.doc.clone()))])).collect())
}

fn decks_of(c: &Compiled) -> Vec<String> {
    let mut names: BTreeSet<String> = c.decks.keys().map(|d| d.name().to_string()).collect();
    for s in &c.segments {
        names.extend(s.decks.keys().map(|d| d.name().to_string()));
    }
    names.into_iter().collect()
}

fn lineage_json(c: &Compiled) -> JsonValue {
    JsonValue::Array(
        c.lineage
            .iter()
            .map(|g| {
                obj([
                    ("scope", JsonValue::str(g.scope.clone())),
                    ("gen", JsonValue::int(i64::from(g.gen))),
                    ("cid", JsonValue::str(g.cid.clone())),
                    ("forkedFrom", opt_str(g.forked_from.clone())),
                    ("start_bar", JsonValue::int(i64::from(g.start))),
                    ("bars", JsonValue::int(i64::from(g.bars))),
                ])
            })
            .collect(),
    )
}

/// `arrange { }` with mutations (djlive mutate.rs): each section it plays, where, and the generation of each pattern it
/// reads (with that generation's text, for the editor's Follow), and every mutated pattern's generations. None: the
/// program has no `arrange { }`.
fn arrange_json(text: &str, c: &Compiled, env: &djlive::env::Env) -> Option<JsonValue> {
    let t = djlive::mutate::generations(text, env)?;
    // one segment per play, in order (an `evolve` inside a section chains more segments onto its play); a named block
    // (`arrange NAME: { }`) plays where the flow puts it, among the other arrangements' sections: its plays are its
    // first time through, the first run of segments that are its plays in order
    let all: Vec<_> = c.segments.iter().filter(|s| !s.chain).collect();
    let from = if t.arrangement.is_some() {
        (0..all.len()).find(|&i| t.plays.len() <= all.len() - i && t.plays.iter().zip(&all[i..]).all(|(p, s)| s.name == p.name)).unwrap_or(all.len())
    } else {
        0
    };
    let mut segs = all.into_iter().skip(from);
    let plays = t
        .plays
        .iter()
        .map(|p| {
            let seg = segs.next();
            let mut fields = vec![
                ("section", JsonValue::str(p.section.clone())),
                ("name", JsonValue::str(p.name.clone())),
                ("pass", JsonValue::int(i64::from(p.pass))),
                ("coda", JsonValue::Bool(p.coda)),
                ("start_bar", seg.map_or(JsonValue::Null, |s| JsonValue::int(i64::from(s.start)))),
                ("bars", seg.map_or(JsonValue::Null, |s| JsonValue::int(i64::from(s.bars)))),
                (
                    "reads",
                    JsonValue::Array(
                        p.reads
                            .iter()
                            .map(|r| {
                                let mut fields = vec![("pattern", JsonValue::str(r.pattern.clone())), ("gen", JsonValue::int(i64::from(r.generation))), ("once", JsonValue::Bool(r.once)), ("text", JsonValue::str(r.text.clone()))];
                                // (a `jux`: the right copy's text; only then, so every other program's report is as it was)
                                if let Some(right) = &r.right {
                                    fields.push(("text_right", JsonValue::str(right.clone())));
                                }
                                JsonValue::Object(JsonObject::from_pairs(fields))
                            })
                            .collect(),
                    ),
                ),
            ];
            // (a switch of the control deck held lines of the section back in this play: the calls whose lines do not fire; only
            // then, so every other program's report is as it was)
            if !p.control_off.is_empty() {
                fields.push(("control_off", JsonValue::Array(p.control_off.iter().cloned().map(JsonValue::str).collect())));
            }
            JsonValue::Object(JsonObject::from_pairs(fields))
        })
        .collect();
    let patterns = JsonValue::Object(JsonObject::from_pairs(t.patterns.iter().map(|(name, gens)| {
        (
            name.clone(),
            JsonValue::Array(
                gens.iter()
                    .map(|g| {
                        let mut fields = vec![
                            ("gen", JsonValue::int(i64::from(g.generation))),
                            ("pass", g.pass.map_or(JsonValue::Null, |p| JsonValue::int(i64::from(p)))),
                            ("mutation", g.mutation.map_or(JsonValue::Null, |m| JsonValue::int(m as i64))),
                            ("ops", JsonValue::str(g.ops.clone())),
                            ("text", JsonValue::str(g.text.clone())),
                        ];
                        // only a counted line (`~3`) says how often it fires: every other program's report is as it was
                        if g.every > 1 {
                            fields.push(("every", JsonValue::int(i64::from(g.every))));
                        }
                        if let Some(right) = &g.right {
                            fields.push(("text_right", JsonValue::str(right.clone())));
                        }
                        JsonValue::Object(JsonObject::from_pairs(fields))
                    })
                    .collect(),
            ),
        )
    })));
    let mut fields = vec![("passes", JsonValue::int(i64::from(t.passes))), ("unbounded", JsonValue::Bool(t.unbounded)), ("plays", JsonValue::Array(plays)), ("patterns", patterns)];
    if let Some(a) = &t.arrangement {
        fields.push(("arrangement", JsonValue::str(a.clone())));
    }
    // the plays a counted line (`~3`, `@4`) fired on: its passes (0-based) and, for each, the play that first read it, as
    // an index into `plays` (a section's name is not enough: the same variant plays more than once), or null for a firing at
    // the very end that nothing plays after
    if !t.fires.is_empty() {
        let fires = t.fires.iter().map(|f| {
            obj([
                ("mutation", JsonValue::int(f.mutation as i64)),
                ("every", JsonValue::int(i64::from(f.every))),
                ("once", JsonValue::Bool(f.once)),
                ("passes", JsonValue::Array(f.passes.iter().map(|p| JsonValue::int(i64::from(*p))).collect())),
                ("plays", JsonValue::Array(f.plays.iter().map(|i| i.map_or(JsonValue::Null, |i| JsonValue::int(i as i64))).collect())),
            ])
        });
        fields.push(("fires", JsonValue::Array(fires.collect())));
    }
    Some(JsonValue::Object(JsonObject::from_pairs(fields)))
}

/// How long the program is in bars: the timeline of a song, or where a program with no sections says it ends.
fn length_bars(c: &Compiled) -> Option<u32> {
    if c.arranged && !c.flat_clock {
        c.song_bars()
    } else {
        c.end
    }
}

fn rights_flags(r: &RightsReport) -> (bool, bool) {
    (r.uses_assets(), r.restricted())
}

/// `check(text)`: the whole report as a value.
pub fn check_report(text: &str) -> JsonValue {
    check_report_with(text, false)
}

/// [`check_report`], and with `expand` each reference's `expansion` in `refs` (crate::resolution): the BeatScript it is written
/// out as where it stands (`check` with `expand: true`, `beatscript check --expand`).
pub fn check_report_with(text: &str, expand: bool) -> JsonValue {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let chk = check_with(text, &env);
    let (errors, warnings) = counts(&chk.diagnostics);
    let diagnostics = diagnostics_json(&chk.diagnostics, text, &ix);
    let Some(c) = chk.compiled else {
        let (uses_assets, restricted) = rights_flags(&assets::rights_of_text(text, &env));
        let fixed = djlive::mutate::fix(text);
        let mut rep = obj([
            ("ok", JsonValue::Bool(false)),
            ("errors", int(errors)),
            ("warnings", int(warnings)),
            ("diagnostics", diagnostics),
            ("cid", JsonValue::Null),
            ("canonical", JsonValue::Null),
            ("version", int(djlive::version::of_source(text).number() as usize)),
            ("renamed_cid", JsonValue::Null),
            ("nets", JsonValue::Object(JsonObject::new())),
            ("song_nets", JsonValue::Null),
            ("tempo", JsonValue::Null),
            ("key", JsonValue::Null),
            ("swing", JsonValue::Null),
            ("arranged", JsonValue::Bool(false)),
            ("song_bars", JsonValue::Null),
            ("ends", JsonValue::Bool(false)),
            ("end_bar", JsonValue::Null),
            ("loops", JsonValue::Bool(false)),
            ("sections", JsonValue::Array(Vec::new())),
            ("decks", JsonValue::Array(Vec::new())),
            ("evolve", JsonValue::Array(Vec::new())),
            ("lineage", JsonValue::Array(Vec::new())),
            ("uses_assets", JsonValue::Bool(uses_assets)),
            ("restricted", JsonValue::Bool(restricted)),
            ("seconds", JsonValue::Null),
        ]);
        // an `arrange { }` whose operators are not ones: the text with each put out of action as `id` (mutate.rs)
        if let (Some(f), JsonValue::Object(o)) = (fixed, &mut rep) {
            o.set("fixed", JsonValue::str(f));
        }
        return rep;
    };
    let (uses_assets, restricted) = rights_flags(&c.rights);
    let docs = djlive::doc::doc_comments(text);
    let bars = length_bars(&c);
    // seconds: the bars it plays (an `end` cuts the timeline short) as four beats at the tempo, to the
    // millisecond; None without a tempo (the decks keep their own) or for a program that plays on
    let played = match (c.end, bars) {
        (Some(e), Some(b)) => Some(e.min(b)),
        (Some(e), None) => Some(e),
        (None, b) => b.filter(|_| c.looped),
    };
    let arrange = arrange_json(text, &c, &env);
    let seconds = played.zip(c.tempo).map(|(b, t)| (f64::from(b) * 4.0 * 60.0 / t * 1000.0).round() / 1000.0);
    // named arrangements add a `flow` key; a program without them has the report it always had
    let flow = c.flow.as_ref().map(|f| {
        obj([
            ("arrangements", JsonValue::Array(f.arrangements.iter().map(|(n, e)| obj([("name", JsonValue::str(n.clone())), ("ends", JsonValue::str(e.clone()))])).collect())),
            ("order", strs(f.order.iter().cloned())),
            ("loop_from", f.loop_from.map_or(JsonValue::Null, |v| JsonValue::int(v as i64))),
            ("loop_from_bar", JsonValue::int(i64::from(c.loop_from))),
            ("cycle", JsonValue::Bool(f.cycle)),
            ("unreachable", strs(f.unreachable.iter().cloned())),
            ("finite", JsonValue::Bool(f.loop_from.is_none())),
        ])
    });
    let mut fields = vec![
        ("ok", JsonValue::Bool(true)),
        ("errors", int(errors)),
        ("warnings", int(warnings)),
        ("diagnostics", diagnostics),
        ("cid", JsonValue::str(c.cid.clone())),
        ("canonical", JsonValue::str(c.text.clone())),
        // the language version (djlive::version): 2 for a text that says `#! beatscript 2`, 1 for the old spelling; and the CID
        // the program has in the other version (a BeatScript 1 program's BeatScript 2 CID, and the other way round)
        ("version", int(djlive::version::of_canonical(&c.text).number() as usize)),
        ("renamed_cid", JsonValue::str(djlive::compile::renamed_cid(&c.text))),
        ("nets", str_map(&c.net_cids())),
        ("song_nets", song_nets_json(&c)),
        ("tempo", opt_num(c.tempo)),
        ("key", opt_str(c.key.map(|k| k.text()))),
        ("swing", opt_num(c.swing)),
        ("arranged", JsonValue::Bool(c.arranged)),
        ("song_bars", bars.map_or(JsonValue::Null, |b| JsonValue::int(i64::from(b)))),
        ("ends", JsonValue::Bool(c.end.is_some())),
        ("end_bar", c.end.map_or(JsonValue::Null, |e| JsonValue::int(i64::from(e)))),
        ("loops", JsonValue::Bool(c.looped)),
        ("sections", sections(&c, &docs)),
        ("decks", strs(decks_of(&c))),
        ("evolve", strs(c.evolve_report())),
        ("lineage", lineage_json(&c)),
        ("uses_assets", JsonValue::Bool(uses_assets)),
        ("restricted", JsonValue::Bool(restricted)),
        ("seconds", opt_num(seconds)),
    ];
    if let Some(f) = flow {
        fields.push(("flow", f));
    }
    // `launch` lines (a pad brings a scene in): said only when the program has some, so no other report changes
    if !c.launches.is_empty() {
        fields.push(("launches", c.launches.to_json()));
    }
    if let Some(a) = arrange {
        fields.push(("arrange", a));
    }
    // every doc comment (sections, lets, defs, the arrange): said only when the program has some, so no other report changes
    if !docs.is_empty() {
        fields.push(("docs", docs_json(&docs)));
    }
    // the programs it drew on (an `import`, a set's tracks), leaves first: what a host keeps, fetches or stores with it
    if !c.plays.is_empty() {
        fields.push(("uses", strs(c.plays.iter().map(|(cid, _)| cid.clone()))));
    }
    // a station (radio, djlive::station): its knobs, where its walk starts, the first chapter's tracks and the first segue;
    // said only for a station program, so no other report changes
    if let Some(s) = &c.station {
        fields.push(("station", s.to_json()));
    }
    // what the recipe makes (crate::resolution): `expanded_cid` always (the CID itself for a program with no references), and
    // `refs` and `resolution` when it has references
    fields.extend(crate::resolution::fields(text, &c.cid, expand));
    JsonValue::Object(JsonObject::from_pairs(fields))
}

/// `check(text)` as the JSON string `beatscript check` prints and `beatscript_check` returns.
pub fn check_json(text: &str) -> String {
    check_report(text).js_stringified()
}

/// [`check_report_with`] as a JSON string: `expand` adds each reference's `expansion`.
pub fn check_json_with(text: &str, expand: bool) -> String {
    check_report_with(text, expand).js_stringified()
}

/// `fmt(text)`: the program formatted and in BeatScript 2 (`text`: the canonical form with its `#!` header lines, comments and
/// the blank lines between blocks kept, and the version line `#! beatscript 2` first; the one the editor's fmt, the `fmt` tool
/// and `beatscript fmt` put in front of the user), its canonical text (`canonical`: no comments, what the CID is of), its CID
/// (`cid`), its `version` (2), and the comments. A BeatScript 1 text (`#! btw 1`, or no version line) is upgraded, which is a
/// new CID for the same program: `from_cid` is then the CID of the text as given. A BeatScript 2 text formats to its own CID.
/// [`format_text`] formats without upgrading.
pub fn fmt_json(text: &str) -> String {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let chk = check_with(text, &env);
    let diagnostics = diagnostics_json(&chk.diagnostics, text, &ix);
    let v = match chk.compiled {
        Some(c) => {
            let formatted = upgrade_text(text);
            // the program the text upgrades to: the same statements with the version line first
            let canonical = if formatted.is_some() { djlive::version::upgrade_canonical(&c.text) } else { c.text.clone() };
            let cid = djlive::compile::program_cid(&canonical);
            let mut fields = vec![
                ("ok", JsonValue::Bool(true)),
                ("canonical", JsonValue::str(canonical.clone())),
                ("cid", JsonValue::str(cid.clone())),
                ("version", int(djlive::version::of_canonical(&canonical).number() as usize)),
                ("comments", strs(c.notes.iter().cloned())),
                ("diagnostics", diagnostics),
            ];
            if cid != c.cid {
                fields.push(("from_cid", JsonValue::str(c.cid.clone())));
            }
            if let Some(t) = formatted {
                fields.push(("text", JsonValue::str(t)));
            }
            JsonValue::Object(JsonObject::from_pairs(fields))
        }
        None => obj([("ok", JsonValue::Bool(false)), ("canonical", JsonValue::Null), ("cid", JsonValue::Null), ("comments", JsonValue::Array(Vec::new())), ("diagnostics", diagnostics)]),
    };
    v.js_stringified()
}

/// The text formatted and in BeatScript 2 (djlive::format::format_upgrading), or None when it does not read: what `fmt` gives.
pub fn upgrade_text(text: &str) -> Option<String> {
    djlive::format::format_upgrading(text, &web_env().env()).ok()
}

/// The text formatted in the version it is in (djlive::format: canonical, with the `#!` headers, comments and blank lines kept,
/// a BeatScript 1 text left BeatScript 1, so its CID never moves), or None when it does not read. Parses and prints only: no CID
/// is made, nothing is compiled, so it is cheap and says nothing about the program. The server's autoformat uses it: nothing
/// upgrades a program silently.
pub fn format_text(text: &str) -> Option<String> {
    djlive::format::format(text, &web_env().env()).ok()
}

/// `expand(text)` (`beatscript fmt --expand`, the MCP `expand` tool): the program with every `seed(...)`, every `template(...)` and every
/// reference to a stored program (`<cid>.drop`) written out as the plain BeatScript it stands for (each with a `# from ...` line),
/// to edit from there. `text` is the written-out
/// program; `cid` and `canonical` are its own; `from_cid` is the CID of the program as given (with its references), and with
/// `lineage` the text opens with `#! forked-from: <from_cid>`, so a shared expansion says what it came from. A program with
/// no reference comes back as it is (`refs` empty). The two play the same: djlive tests/seed.rs holds it.
pub fn expand_json(text: &str, lineage: bool) -> String {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let given = check_with(text, &env);
    let fail = |ds: &[Diagnostic]| obj([("ok", JsonValue::Bool(false)), ("text", JsonValue::Null), ("refs", JsonValue::Array(Vec::new())), ("diagnostics", diagnostics_json(ds, text, &ix))]).js_stringified();
    let Some(from) = given.compiled else { return fail(&given.diagnostics) };
    // (the templates, then the seeds they leave, then the stored programs a reference borrows from: the programs the host knows, `known.rs`)
    let (body, refs) = match djlive::stored::expand_text(text, &env) {
        None => (text.to_string(), Vec::new()),
        Some(Err(ds)) => return fail(&ds),
        Some(Ok((body, refs, _))) => (body, refs),
    };
    let out = if lineage && !refs.is_empty() && !body.lines().any(|l| l.trim_start().starts_with("#! forked-from:")) {
        format!("#! forked-from: {}\n{body}", from.cid)
    } else {
        body
    };
    let chk = check_with(&out, &env);
    let oix = Utf16Index::new(&out);
    // the `use` lines it wrote out: each name with its source (crate::resolution::IMPORTS_DOC)
    let imports = djlive::uses::imports(text, &env).unwrap_or_default();
    match chk.compiled {
        Some(c) => {
            let mut fields = vec![
                ("ok", JsonValue::Bool(true)),
                ("text", JsonValue::str(out.clone())),
                ("canonical", JsonValue::str(c.text.clone())),
                ("cid", JsonValue::str(c.cid.clone())),
                ("from_cid", JsonValue::str(from.cid.clone())),
                ("refs", strs(refs.iter().cloned())),
                ("diagnostics", diagnostics_json(&chk.diagnostics, &out, &oix)),
            ];
            // the manifest, as check gives it: each reference once with its pin, and a template's nested parts by their CIDs
            if !refs.is_empty() {
                let r = crate::resolution::of_with(text, &from.cid, &env);
                fields.push(("resolution", crate::resolution::resolution_json(&r, text)));
            }
            if !imports.is_empty() {
                fields.push(("imports", crate::resolution::imports_json(&imports, text)));
            }
            JsonValue::Object(JsonObject::from_pairs(fields)).js_stringified()
        }
        // (it cannot fail when the program it came from checked; if it ever does, say where, in the written-out text)
        None => obj([("ok", JsonValue::Bool(false)), ("text", JsonValue::str(out.clone())), ("refs", strs(refs.into_iter())), ("diagnostics", diagnostics_json(&chk.diagnostics, &out, &oix))]).js_stringified(),
    }
}

/// `instruments()`: the voices, family words, kits and palettes.
pub fn instruments_json() -> String {
    web_env().instruments().js_stringified()
}

/// `generation(text, scope, n)`: the CID of generation `n` of an `evolve` (the first one's scope when none
/// is given), made from generation 0 and the rule alone.
pub fn generation_json(text: &str, scope: Option<&str>, n: u32) -> String {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let chk = check_with(text, &env);
    let fail = |error: String, ds: &[Diagnostic]| obj([("ok", JsonValue::Bool(false)), ("error", JsonValue::str(error)), ("diagnostics", diagnostics_json(ds, text, &ix))]).js_stringified();
    let Some(c) = chk.compiled else { return fail("the program has errors: `check` says where".into(), &chk.diagnostics) };
    let Some(scope) = scope.filter(|s| !s.is_empty()).map(str::to_string).or_else(|| c.evolves.first().map(|r| r.scope.clone())) else {
        return fail("this program has no `evolve`".into(), &chk.diagnostics);
    };
    match c.generation_cid(&scope, n) {
        Some(cid) => obj([("ok", JsonValue::Bool(true)), ("cid", JsonValue::str(cid)), ("scope", JsonValue::str(scope)), ("n", JsonValue::int(i64::from(n)))]).js_stringified(),
        None => {
            let scopes: Vec<String> = c.evolves.iter().map(|r| r.scope.clone()).collect();
            fail(format!("no `evolve` in `{scope}`{}", if scopes.is_empty() { ": this program has none".to_string() } else { format!(" (its evolves are in: {})", scopes.join(", ")) }), &chk.diagnostics)
        }
    }
}

/// `nets(text)`: what the program compiles to, for debugging and tests: the program's CID, each deck's net
/// (its CID, and with `envelopes` the raw-nets envelope the engine loads) and each section's.
pub fn nets_json(text: &str, envelopes: bool) -> String {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let chk = check_with(text, &env);
    let Some(c) = chk.compiled else {
        return obj([("ok", JsonValue::Bool(false)), ("cid", JsonValue::Null), ("diagnostics", diagnostics_json(&chk.diagnostics, text, &ix))]).js_stringified();
    };
    let deck_json = |ir: &djlive::compile::DeckIr| {
        let mut o = JsonObject::new();
        o.set("cid", JsonValue::str(ir.net_cid()));
        if envelopes {
            o.set("envelope", ir.envelope(0));
        }
        JsonValue::Object(o)
    };
    let decks = JsonValue::Object(JsonObject::from_pairs(c.decks.iter().map(|(d, ir)| (d.name(), deck_json(ir))).collect::<Vec<_>>()));
    let segments = JsonValue::Array(
        c.segments
            .iter()
            .map(|s| {
                obj([
                    ("name", JsonValue::str(s.name.clone())),
                    ("start", JsonValue::int(i64::from(s.start))),
                    ("bars", JsonValue::int(i64::from(s.bars))),
                    ("gen", s.gen.map_or(JsonValue::Null, |g| JsonValue::int(i64::from(g)))),
                    ("decks", JsonValue::Object(JsonObject::from_pairs(s.decks.iter().map(|(d, ir)| (d.name(), deck_json(ir))).collect::<Vec<_>>()))),
                ])
            })
            .collect(),
    );
    obj([
        ("ok", JsonValue::Bool(true)),
        ("cid", JsonValue::str(c.cid.clone())),
        ("decks", decks),
        ("segments", segments),
        ("song_nets", song_nets_json(&c)),
    ])
    .js_stringified()
}
