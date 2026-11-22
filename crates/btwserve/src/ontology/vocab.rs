//! The BeatScript vocabulary's sources, as btwserve itself has them, and the test that holds the embedded ontology to
//! them. The BeatScript ontology's vocabulary (`ontology/generated/beatscript.ttl`, written by
//! `tools/ontology/beatscript.py`) is generated from the dump [`sources`] writes, so a term is never typed by hand twice:
//!
//! - the grammar: djlive's `grammar.ebnf`, the language's one source (a term per production, keyword, token class);
//! - the instruments: `btwapi::instruments_json()`, what `/api/instruments` and the `instruments` tool answer;
//! - the control registry: btwapi's snapshot of the DJ app's nouns and their verbs;
//! - djlive's registries: generators, pattern transforms, evolve and mutation operators, sugar words;
//! - both MCP servers' tools/list: BeatScript's (`crate::mcp`, with the production defaults) and the ontology's;
//! - the public-domain phrasebook's closed vocabularies (`djengine::vocal::phoneme`'s meter families and feels), and the terms
//!   its linked data names (`btwserve::phrasebook::CONTEXT_TERMS`): the lines themselves are data, not vocabulary.
//!
//! Dump: `ONTOLOGY_VOCAB_OUT=<path> cargo test -p btwserve --bin btwserve vocab_dump` (writes the JSON, sorted keys).
//! `ontology_matches_btwserve` fails when the release in `ontology/release/` and this code disagree, in either
//! direction: a production, keyword, voice, kit, palette, family, noun, verb, operator or tool that one has and the
//! other does not, or a production's EBNF, a tool's description, schema or flags that differ. The fix is always the
//! same: `ontology/build.sh` (dump, generate, build the release, lock).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::graph::{Index, NS, RDF_TYPE, SITE_BASE, SKOS};
use crate::compose::gojson;

pub const GRAMMAR: &str = include_str!("../../../djlive/grammar.ebnf");
const PUBLIC: &str = "https://beatscript.app";

/// btwserve's BeatScript tools/list with the production configuration (the defaults: 16384-byte programs, 60 checks a
/// minute, 30 renders an hour, the queue's 50/20/30/8/30).
pub fn beatscript_tools() -> Vec<Value> {
    crate::mcp::tools::tool_list(PUBLIC, 16384, 60, 30, (50, 20, 30, 8, 30))
}

/// The dump tools/ontology/beatscript.py reads.
pub fn sources() -> Value {
    let snapshot: Value = serde_json::from_str(btwapi::snapshot::SNAPSHOT_JSON).expect("snapshot.json");
    let instruments: Value = serde_json::from_str(&btwapi::instruments_json()).expect("instruments json");
    let tools = beatscript_tools();
    let auth: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).filter(|n| crate::mcp::tools::auth_tool(n)).collect();
    gojson::sorted_deep(json!({
        "source": {"repo": "btwserve (beatscript.app)", "commit": option_env!("BTWSERVE_COMMIT").unwrap_or("working tree")},
        "grammar_ebnf": GRAMMAR,
        "instruments": instruments,
        "control_nouns": snapshot["nouns"],
        "language": {
            "generators": djlive::pattern::GENERATORS,
            "pattern_transforms": djlive::pattern::TRANSFORMS.iter().chain(djlive::answer::ANSWERS.iter()).collect::<Vec<_>>(),
            "evolve_ops": djlive::evolve::TRANSFORMS,
            "mutation_ops": djlive::mutate::operator_names(),
            "sugar": djlive::parse::SUGAR.iter().map(|(w, n)| json!([w, n])).collect::<Vec<_>>(),
            "templates": djlive::template::TEMPLATES.iter().map(|t| json!({"name": t.name, "summary": t.summary, "example": t.example})).collect::<Vec<_>>(),
            "forms": djlive::template::forms().map(|(t, f)| json!({"name": t.name, "label": f.label, "shape": f.shape, "plan": f.plan, "mutations": f.mutations, "themes": f.themes, "reference": f.reference.map(|(n, c)| json!({"name": n, "cid": c}))})).collect::<Vec<_>>(),
            "roles": djlive::parse::ROLES,
            "part_params": djlive::parse::PARAMS,
        },
        "phrasebook": {
            "version": djengine::vocal::phoneme::pd_phrasebook_version(),
            "families": djengine::vocal::phoneme::PD_FAMILIES.iter().map(|f| json!({"id": f.id, "name": f.name, "about": f.about, "groove": btwserve::found::groove(f.id), "genres": btwserve::found::FAMILY_GROOVES.iter().find(|(id, _, _)| *id == f.id).map(|(_, _, g)| g.to_vec()).unwrap_or_default()})).collect::<Vec<_>>(),
            "feels": djengine::vocal::phoneme::PD_FEELS.iter().map(|(id, about)| json!({"id": id, "about": about})).collect::<Vec<_>>(),
        },
        "themes": {
            "version": djengine::themes::themes_version(),
            "roles": djengine::themes::ROLES.iter().map(|(id, about)| json!({"id": id, "about": about})).collect::<Vec<_>>(),
            "modes": djengine::themes::MODES.iter().map(|(id, about)| json!({"id": id, "about": about})).collect::<Vec<_>>(),
        },
        "mcp": {
            "beatscript": tools,
            "beatscript_auth": auth,
            "ontology": super::mcp::tool_defs(),
            "ontology_instructions": super::mcp::INSTRUCTIONS,
        },
    }))
}

/// The grammar's productions as `(name, rhs)`: comments removed (quote-aware), split at each `;` outside a string,
/// the right side's whitespace collapsed. tools/ontology/beatscript.py `productions` applies the same rule.
pub fn productions(text: &str) -> Vec<(String, String)> {
    let b: Vec<char> = text.chars().collect();
    let mut code = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '"' {
            let j = (i + 1..b.len()).find(|&j| b[j] == '"').expect("unterminated terminal");
            code.extend(&b[i..=j]);
            i = j + 1;
        } else if b[i] == '(' && b.get(i + 1) == Some(&'*') {
            let j = (i + 2..b.len()).find(|&j| b[j] == '*' && b.get(j + 1) == Some(&')')).expect("unterminated comment");
            code.push(' ');
            i = j + 2;
        } else {
            code.push(b[i]);
            i += 1;
        }
    }
    let mut out = Vec::new();
    let (mut cur, mut in_str) = (String::new(), false);
    for c in code.chars() {
        if c == '"' {
            in_str = !in_str;
        }
        if c == ';' && !in_str {
            let (name, rhs) = cur.split_once('=').expect("a production");
            out.push((name.trim().to_string(), rhs.split_whitespace().collect::<Vec<_>>().join(" ")));
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    assert!(cur.trim().is_empty(), "text after the last production: {cur:?}");
    out
}

fn terminals(rhs: &str) -> Vec<String> {
    rhs.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

/// Every subject of `class` in the asserted graph, by the value of `p` (its name as BeatScript writes it).
fn by(g: &Index, class: &str, p: &str) -> BTreeMap<String, String> {
    g.of_type(&format!("{NS}{class}")).into_iter().map(|s| (g.first(&s, &format!("{NS}{p}")), s)).collect()
}

fn same(what: &str, want: BTreeSet<String>, have: impl IntoIterator<Item = String>, errs: &mut Vec<String>) {
    let have: BTreeSet<String> = have.into_iter().collect();
    for x in want.difference(&have) {
        errs.push(format!("{what} {x:?} is in btwserve but not in the ontology"));
    }
    for x in have.difference(&want) {
        errs.push(format!("{what} {x:?} is in the ontology but not in btwserve"));
    }
}

fn strs(v: &Value) -> BTreeSet<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

/// What `ontology_matches_btwserve` finds wrong (empty: they agree).
pub fn drift(g: &Index) -> Vec<String> {
    let src = sources();
    let mut errs = Vec::new();

    // the grammar: every production with the same EBNF, every keyword, nothing the grammar has dropped
    let prods = productions(GRAMMAR);
    let have = by(g, "GrammarProduction", "writtenAs");
    same("production", prods.iter().map(|p| p.0.clone()).collect(), have.keys().cloned(), &mut errs);
    for (name, rhs) in &prods {
        if let Some(s) = have.get(name) {
            let e = g.first(s, &format!("{NS}ebnf"));
            if &e != rhs {
                errs.push(format!("production {name}: the ontology has `{e}`, the grammar `{rhs}`"));
            }
        }
    }
    let kw: BTreeSet<String> = prods.iter().flat_map(|p| terminals(&p.1)).collect();
    same("keyword", kw, by(g, "Keyword", "keywordText").into_keys(), &mut errs);

    // instruments
    let inst = &src["instruments"];
    let voices = by(g, "BeatScriptVoice", "writtenAs").into_keys().filter_map(|w| w.strip_prefix("inst=").map(str::to_string));
    same("voice", strs(&inst["voices"]), voices, &mut errs);
    let kits = by(g, "DrumKit", "writtenAs").into_keys().filter_map(|w| w.strip_prefix("kit=").map(str::to_string));
    same("kit", strs(&inst["kits"]), kits, &mut errs);
    same("voice family", strs(&inst["families"]), by(g, "VoiceFamily", "writtenAs").into_keys(), &mut errs);
    same("palette", strs(&inst["palettes"]), by(g, "Palette", "writtenAs").into_keys(), &mut errs);

    // the control registry: nouns, and each noun's verbs
    let nouns = by(g, "ControlNoun", "writtenAs");
    let want: BTreeSet<String> = src["control_nouns"].as_array().unwrap().iter().map(|n| format!("do {}", n["name"].as_str().unwrap())).collect();
    same("control noun", want, nouns.keys().cloned(), &mut errs);
    for n in src["control_nouns"].as_array().unwrap() {
        let name = n["name"].as_str().unwrap();
        if let Some(s) = nouns.get(&format!("do {name}")) {
            let verbs = n["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
            let have = g.all(s, &format!("{NS}acceptsVerb")).into_iter().map(|v| g.label(&v));
            same(&format!("verb of {name}"), verbs, have, &mut errs);
        }
    }

    // djlive's registries
    let l = &src["language"];
    same("evolve operator", strs(&l["evolve_ops"]), by(g, "EvolveOperator", "writtenAs").into_keys(), &mut errs);
    same("mutation operator", strs(&l["mutation_ops"]), by(g, "MutationOperator", "writtenAs").into_keys(), &mut errs);
    let templates: BTreeSet<String> = l["templates"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    same("phrase template", templates, by(g, "PhraseTemplate", "writtenAs").into_keys(), &mut errs);
    let mut gens = strs(&l["generators"]);
    gens.extend(["choose", "deg"].iter().filter(|w| g.has(&format!("{SITE_BASE}/concept/keyword-{w}"))).map(|w| w.to_string()));
    same("generator", gens, by(g, "PatternGenerator", "writtenAs").into_keys(), &mut errs);
    let have_t: BTreeSet<String> = by(g, "PatternTransform", "writtenAs").into_keys().collect();
    for t in strs(&l["pattern_transforms"]) {
        if !have_t.contains(&t) {
            errs.push(format!("pattern transform {t:?} is in btwserve but not in the ontology"));
        }
    }

    // the public-domain phrasebook: each meter family and feel the corpus is filed under, and every term its linked data writes
    let pb = &src["phrasebook"];
    same("meter family", pb["families"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "MeterFamily", "writtenAs").into_keys(), &mut errs);
    same("feel", pb["feels"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "Feel", "writtenAs").into_keys(), &mut errs);
    for f in pb["families"].as_array().unwrap() {
        let iri = btwserve::phrasebook::family_iri(f["id"].as_str().unwrap());
        if !g.has(&iri) {
            errs.push(format!("the phrasebook's family {iri} is not in the ontology"));
        }
    }
    for f in pb["feels"].as_array().unwrap() {
        let iri = btwserve::phrasebook::feel_iri(f["id"].as_str().unwrap());
        if !g.has(&iri) {
            errs.push(format!("the phrasebook's feel {iri} is not in the ontology"));
        }
    }
    for term in ["concept/technique-found-phrases"] {
        let iri = format!("{SITE_BASE}/{term}");
        if !g.has(&iri) {
            errs.push(format!("the found-phrases technique {iri} is not in the ontology"));
        }
    }
    // the forms: one term each, linked to the mutation operators its plan uses and to its template; and the theme library's
    // roles, modes and every term its linked data writes
    let forms = l["forms"].as_array().unwrap();
    same("form", forms.iter().map(|f| f["name"].as_str().unwrap().to_string()).collect(), by(g, "Form", "writtenAs").into_keys(), &mut errs);
    for f in forms {
        let iri = format!("{SITE_BASE}/concept/form-{}", f["name"].as_str().unwrap());
        for m in f["mutations"].as_array().unwrap() {
            let op = format!("{SITE_BASE}/concept/mutation-op-{}", m.as_str().unwrap());
            if !g.has(&iri) || !g.all(&iri, &format!("{NS}usesMutation")).contains(&op) {
                errs.push(format!("the form {iri} does not say it uses {op}"));
            }
        }
    }
    let th = &src["themes"];
    same("theme role", th["roles"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "ThemeRole", "writtenAs").into_keys(), &mut errs);
    same("theme mode", th["modes"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "ThemeMode", "writtenAs").into_keys(), &mut errs);
    for (id, _) in djengine::themes::ROLES {
        if !g.has(&btwserve::themes::role_iri(id)) {
            errs.push(format!("the theme role {} is not in the ontology", btwserve::themes::role_iri(id)));
        }
    }
    for (id, _) in djengine::themes::MODES {
        if !g.has(&btwserve::themes::mode_iri(id)) {
            errs.push(format!("the theme mode {} is not in the ontology", btwserve::themes::mode_iri(id)));
        }
    }
    for (key, local) in btwserve::themes::CONTEXT_TERMS {
        if !g.has(&format!("{NS}{local}")) {
            errs.push(format!("the theme library's linked data writes `{key}`, which is `ns:{local}`, and the ontology has no such term"));
        }
    }
    for (key, local) in btwserve::phrasebook::CONTEXT_TERMS {
        if !g.has(&format!("{NS}{local}")) {
            errs.push(format!("the phrasebook's linked data writes `{key}`, which is `ns:{local}`, and the ontology has no such term"));
        }
    }

    // the MCP tools: name, description, schema, flags, for both servers
    let tools = by(g, "McpTool", "toolName");
    let mut want = BTreeSet::new();
    let auth = strs(&src["mcp"]["beatscript_auth"]);
    for (server, list) in [("product-beatscript-mcp", &src["mcp"]["beatscript"]), ("product-beatscript-ontology-mcp", &src["mcp"]["ontology"])] {
        for t in list.as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            let server_iri = format!("{SITE_BASE}/concept/{server}");
            want.insert(format!("{server}/{name}"));
            let Some(s) = g.of_type(&format!("{NS}McpTool")).into_iter().find(|s| g.first(s, &format!("{NS}toolName")) == name && g.first(s, &format!("{NS}providedBy")) == server_iri) else { continue };
            let check = |what: &str, have: String, want: String, errs: &mut Vec<String>| {
                if have != want {
                    errs.push(format!("tool {server}/{name}: {what} differs (ontology {have:?}, btwserve {want:?})"));
                }
            };
            check("description", g.first(&s, &format!("{SKOS}definition")), t["description"].as_str().unwrap().to_string(), &mut errs);
            check("input schema", g.first(&s, &format!("{NS}inputSchema")), serde_json::to_string(&gojson::sorted_deep(t["inputSchema"].clone())).unwrap(), &mut errs);
            let ro = t["annotations"]["readOnlyHint"].as_bool().unwrap_or(server.contains("ontology"));
            check("read-only", g.first(&s, &format!("{NS}readOnly")), ro.to_string(), &mut errs);
            check("needs token", g.first(&s, &format!("{NS}needsToken")), (server.contains("beatscript") && auth.contains(name)).to_string(), &mut errs);
        }
    }
    let have: Vec<String> = tools.values().map(|s| format!("{}/{}", Index::local(&g.first(s, &format!("{NS}providedBy"))), g.first(s, &format!("{NS}toolName")))).collect();
    same("MCP tool", want, have, &mut errs);
    let _ = RDF_TYPE;
    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocab_dump() {
        if let Ok(p) = std::env::var("ONTOLOGY_VOCAB_OUT") {
            std::fs::write(&p, gojson::indent(&sources(), " ") + "\n").unwrap();
            eprintln!("wrote {p}");
        }
    }

    #[test]
    fn grammar_reads() {
        let p = productions(GRAMMAR);
        assert!(p.len() > 100, "{}", p.len());
        assert_eq!(p[0].0, "program");
        assert!(p.iter().any(|(n, r)| n == "term" && r.contains("\"*\"")));
    }

    /// The release in ontology/release/ against btwserve's own grammar, instruments, registry and tools.
    #[test]
    fn ontology_matches_btwserve() {
        let o = super::super::Ontology::load(PUBLIC).unwrap();
        let errs = drift(&o.graph);
        assert!(errs.is_empty(), "the ontology {} and btwserve disagree ({} differences; run ontology/build.sh):\n{}", o.version, errs.len(), errs.join("\n"));
    }
}
