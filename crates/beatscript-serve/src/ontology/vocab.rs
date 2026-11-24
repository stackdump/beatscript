//! The BeatScript vocabulary's sources, as beatscript-serve itself has them, and the test that holds the embedded ontology to
//! them. The BeatScript ontology's vocabulary (`ontology/generated/beatscript.ttl`, written by
//! `tools/ontology/beatscript.py`) is generated from the dump [`sources`] writes, so a term is never typed by hand twice:
//!
//! - the grammar: djlive's `grammar.ebnf`, the language's one source (a term per production, keyword, token class);
//! - the instruments: `beatscript_api::instruments_json()`, what `/api/instruments` and the `instruments` tool answer;
//! - the control registry: beatscript-api's snapshot of the DJ app's nouns and their verbs;
//! - djlive's registries: generators, pattern transforms, evolve and mutation operators, sugar words;
//! - the language's versions (`djlive::version`): each one's version line and what its program CIDs are the CIDs of, and
//!   which is current;
//! - both MCP servers' tools/list: BeatScript's (`crate::mcp`, with the production defaults) and the ontology's;
//! - the public-domain phrasebook's closed vocabularies (`djengine::vocal::phoneme`'s meter families and feels), and the terms
//!   its linked data names (`beatscript_serve::phrasebook::CONTEXT_TERMS`): the lines themselves are data, not vocabulary.
//!
//! Dump: `ONTOLOGY_VOCAB_OUT=<path> cargo test -p beatscript-serve --bin beatscript-serve vocab_dump` (writes the JSON, sorted keys).
//! `ontology_matches_beatscript_serve` fails when the release in `ontology/release/` and this code disagree, in either
//! direction: a production, keyword, voice, kit, palette, family, noun, verb, operator or tool that one has and the
//! other does not, or a production's EBNF, a tool's description, schema or flags that differ. The fix is always the
//! same: `ontology/build.sh` (dump, generate, build the release, lock).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::graph::{Index, NS, RDF_TYPE, SITE_BASE, SKOS};
use crate::compose::gojson;

pub const GRAMMAR: &str = include_str!("../../../djlive/grammar.ebnf");
const PUBLIC: &str = "https://beatscript.app";

/// beatscript-serve's BeatScript tools/list with the production configuration (the defaults: 16384-byte programs, 60 checks a
/// minute, 30 renders an hour, the queue's 50/20/30/8/30).
pub fn beatscript_tools() -> Vec<Value> {
    crate::mcp::tools::tool_list(PUBLIC, 16384, 60, 30, (50, 20, 30, 8, 30))
}

/// Every version of the language, oldest first (a version djlive adds goes here too: `language_versions_are_data`
/// names them).
pub const VERSIONS: [djlive::version::Version; 2] = [djlive::version::Version::V1, djlive::version::Version::V2];

/// The language's versions as the dump writes them (`default`: the version of a text with no version line).
pub fn versions() -> Vec<Value> {
    use djlive::version::{Version, CURRENT};
    VERSIONS
        .iter()
        .map(|v| json!({"number": v.number(), "line": v.line(), "canonical_head": v.canonical_head(), "cid_format": v.format(), "cid_type": v.doc_type(), "current": *v == CURRENT, "default": *v == Version::default()}))
        .collect()
}

/// The generator's genre families by number, as djrust's structure tables number them (beats' structure.js
/// `familyEDM=0, familySong=1, familyJazz=2, familyChill=3`; the ontology's `family/<name>`).
pub const GENRE_FAMILIES: [&str; 4] = ["edm", "song", "jazz", "chill"];

/// The generator's grooves (djrust `tables.json` `grooves`, by name) and the ontology's `groove/<slug>` for each.
pub const GROOVE_SLUGS: [(&str, &str); 7] = [
    ("FourOnFloor", "four-on-floor"),
    ("Offbeat", "offbeat"),
    ("Sidechained", "sidechained"),
    ("SyncoPocket", "syncopated-pocket"),
    ("Breakbeat", "breakbeat"),
    ("Walking", "walking"),
    ("Bossa", "bossa"),
];

/// Every genre the seeded generator (`generate`, `gen@N`) knows (`beatscript_api::generate::genres()`), with what djrust's
/// tables say of it: tempo, root note, scale (by name and by its intervals), the three Euclidean drum rhythms, the feel
/// parameters, the family and groove, the chord progressions, the drum-style variants, and the instrument candidates per
/// role (every name, a voice or not: the generator script keeps the ones BeatScript can write). A native genre (djrust's
/// own, not in beats' catalogue) also says which beats genre's musicality rules it follows.
pub fn generator_genres() -> Vec<Value> {
    use djengine::gen::tables::BeatsTables;
    let t = BeatsTables::shared();
    let groove_name: BTreeMap<i64, &str> = t.grooves.iter().map(|(k, v)| (*v, k.as_str())).collect();
    beatscript_api::generate::genres()
        .into_iter()
        .map(|id| {
            let d = &t.genres[&id];
            let theory = d.theory.as_ref();
            let groove = t.theme_table(&id).and_then(|th| groove_name.get(&th.groove).copied());
            let groove = groove.map(|n| GROOVE_SLUGS.iter().find(|(name, _)| *name == n).map(|(_, s)| *s).unwrap_or_else(|| panic!("genre {id}: groove {n} has no slug in vocab::GROOVE_SLUGS")));
            let family = t.family_of(&id);
            let family = GENRE_FAMILIES.get(family as usize).unwrap_or_else(|| panic!("genre {id}: family {family} is not in vocab::GENRE_FAMILIES"));
            let mut params = serde_json::Map::new();
            for (k, v) in [
                ("swing", d.swing),
                ("humanize", d.humanize),
                ("durationVariation", d.duration_variation),
                ("melodyDensity", d.melody_density),
                ("melodyDuration", d.melody_duration as f64),
                ("bassDensity", d.bass_density),
                ("bassDuration", d.bass_duration as f64),
                ("syncopation", d.syncopation),
                ("ghostNotes", d.ghost_notes),
                ("modalInterchange", d.modal_interchange),
                ("polyrhythm", d.polyrhythm as f64),
            ] {
                // modal interchange and polyrhythm are only said of the genres that have them (as the bitwrap ontology does)
                if v != 0.0 || !matches!(k, "modalInterchange" | "polyrhythm") {
                    params.insert(k.into(), json!(v));
                }
            }
            let flags: Vec<&str> = [("drumFills", d.drum_fills), ("walkingBass", d.walking_bass), ("callResponse", d.call_response), ("tensionCurve", d.tension_curve)]
                .into_iter()
                .filter(|(_, on)| *on)
                .map(|(k, _)| k)
                .collect();
            let progressions: Vec<Value> = theory
                .map(|th| th.chord_progs.iter().map(|p| json!({"degrees": p.chords.iter().map(|c| c.root + 1).collect::<Vec<_>>(), "steps_per": p.steps_per})).collect())
                .unwrap_or_default();
            let mut variants = Vec::new();
            for (role, vs) in theory.map(|th| &th.drum_styles).into_iter().flatten() {
                for (label, v) in vs {
                    variants.push(json!({"role": role, "label": label, "hits_add": v.hits_add, "hits_mul": v.hits_mul.unwrap_or(1.0), "rotation_add": v.rotation_add}));
                }
            }
            let instruments: serde_json::Map<String, Value> = t.instrument_set(&id).map(|m| m.iter().map(|(r, v)| (r.to_string(), json!(v))).collect()).unwrap_or_default();
            json!({
                "id": id,
                "name": d.name,
                "bpm": d.bpm,
                "root_note": d.root_note,
                "scale": d.scale_name,
                "scale_intervals": djengine::gen::core::scale_intervals(&d.scale_name),
                "rhythms": {"kick": d.kick, "snare": d.snare, "hihat": d.hihat},
                "params": params,
                "flags": flags,
                "family": family,
                "groove": groove,
                "native": t.is_native(&id),
                "like": t.native.get(&id).map(|n| n.like.clone()),
                "progressions": progressions,
                "drum_variants": variants,
                "instruments": instruments,
            })
        })
        .collect()
}

/// The sound each voice family word makes when an `inst=` name contains it and has no voice of its own: the engine's
/// generic tonal voice for that word (djrust `wave::synth::tonal_voice`). A word whose generic voice is the same as a
/// name with no family word at all (the drum words) is left out: it picks no tonal sound.
pub fn family_sounds(families: &[String]) -> Vec<Value> {
    use djengine::wave::synth::tonal_voice;
    let key = |v: &djengine::wave::synth::VoiceSpec| (v.wave.clone(), v.attack.to_bits(), v.decay_scale.to_bits(), v.level.to_bits(), v.max_decay.to_bits());
    let plain = key(&tonal_voice(Some("\u{1}"), None));
    families
        .iter()
        .filter_map(|f| {
            let v = tonal_voice(Some(f), None);
            (key(&v) != plain).then(|| json!({"family": f, "wave": v.wave}))
        })
        .collect()
}

/// The dump tools/ontology/beatscript.py reads.
pub fn sources() -> Value {
    let snapshot: Value = serde_json::from_str(beatscript_api::snapshot::SNAPSHOT_JSON).expect("snapshot.json");
    let instruments: Value = serde_json::from_str(&beatscript_api::instruments_json()).expect("instruments json");
    let tools = beatscript_tools();
    let auth: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).filter(|n| crate::mcp::tools::auth_tool(n)).collect();
    gojson::sorted_deep(json!({
        "source": {"repo": "beatscript-serve (beatscript.app)", "commit": option_env!("BEATSCRIPT_SERVE_COMMIT").unwrap_or("working tree")},
        "grammar_ebnf": GRAMMAR,
        "versions": versions(),
        "generator": {
            "genres": generator_genres(),
            "family_sounds": family_sounds(&serde_json::from_value::<Vec<String>>(instruments["families"].clone()).expect("families")),
        },
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
            // TODO(acid-bass): once feat/acid-bass-voice lands, chain its tonal knobs, which are kept out of PARAMS (and so
            // out of parse::reserved()) on purpose: djlive::parse::PARAMS.iter().chain(djlive::compile::ACID_PARAMS.iter())
            "part_params": djlive::parse::PARAMS,
        },
        "phrasebook": {
            "version": djengine::vocal::phoneme::pd_phrasebook_version(),
            "families": djengine::vocal::phoneme::PD_FAMILIES.iter().map(|f| json!({"id": f.id, "name": f.name, "about": f.about, "groove": beatscript_serve::found::groove(f.id), "genres": beatscript_serve::found::FAMILY_GROOVES.iter().find(|(id, _, _)| *id == f.id).map(|(_, _, g)| g.to_vec()).unwrap_or_default()})).collect::<Vec<_>>(),
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
        errs.push(format!("{what} {x:?} is in beatscript-serve but not in the ontology"));
    }
    for x in have.difference(&want) {
        errs.push(format!("{what} {x:?} is in the ontology but not in beatscript-serve"));
    }
}

fn strs(v: &Value) -> BTreeSet<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

/// What `ontology_matches_beatscript_serve` finds wrong (empty: they agree).
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

    // the language's versions: each with its version line and its CID document's format and type, and the current one
    let versions = src["versions"].as_array().unwrap();
    let have_v = by(g, "LanguageVersion", "versionLine");
    same("language version", versions.iter().map(|v| v["line"].as_str().unwrap().to_string()).collect(), have_v.keys().cloned(), &mut errs);
    for v in versions {
        let line = v["line"].as_str().unwrap();
        let Some(s) = have_v.get(line) else { continue };
        for (p, want) in [("programCidFormat", v["cid_format"].to_string()), ("programCidType", v["cid_type"].as_str().unwrap().to_string())] {
            let got = g.first(s, &format!("{NS}{p}"));
            if got != want {
                errs.push(format!("language version {line}: the ontology's {p} is {got:?}, djlive's {want:?}"));
            }
        }
        if v["current"] == json!(true) && g.first(&format!("{SITE_BASE}/concept/product-beatscript-language"), &format!("{NS}currentVersion")) != *s {
            errs.push(format!("the current language version is {line} in djlive, not in the ontology"));
        }
    }

    // instruments
    let inst = &src["instruments"];
    let voices = by(g, "BeatScriptVoice", "writtenAs").into_keys().filter_map(|w| w.strip_prefix("inst=").map(str::to_string));
    same("voice", strs(&inst["voices"]), voices, &mut errs);
    let kits = by(g, "DrumKit", "writtenAs").into_keys().filter_map(|w| w.strip_prefix("kit=").map(str::to_string));
    same("kit", strs(&inst["kits"]), kits, &mut errs);
    same("voice family", strs(&inst["families"]), by(g, "VoiceFamily", "writtenAs").into_keys(), &mut errs);
    same("palette", strs(&inst["palettes"]), by(g, "Palette", "writtenAs").into_keys(), &mut errs);
    // each family word's generic sound
    let fams = by(g, "VoiceFamily", "writtenAs");
    for f in src["generator"]["family_sounds"].as_array().unwrap() {
        let (name, wave) = (f["family"].as_str().unwrap(), f["wave"].as_str().unwrap());
        if let Some(s) = fams.get(name) {
            let have = g.first(s, &format!("{NS}familyWave"));
            if have != wave {
                errs.push(format!("voice family {name}: the ontology's familyWave is {have:?}, the engine's generic voice plays {wave:?}"));
            }
        }
    }

    // the generator's genres: the same set, and each one's tempo, root, scale, family, groove, drum rhythms and progressions
    let genres = by(g, "Genre", "genreId");
    let gen = src["generator"]["genres"].as_array().unwrap();
    same("genre", gen.iter().map(|x| x["id"].as_str().unwrap().to_string()).collect(), genres.keys().cloned(), &mut errs);
    for x in gen {
        let id = x["id"].as_str().unwrap();
        let Some(s) = genres.get(id) else { continue };
        let p = |local: &str| format!("{NS}{local}");
        let mut differs = |what: &str, have: String, want: String| {
            if have != want {
                errs.push(format!("genre {id}: {what} is {have:?} in the ontology, {want:?} in the generator"));
            }
        };
        let num = |v: &Value| v.as_f64().map(|n| n.to_string()).unwrap_or_default();
        differs("bpm", g.num(s, &p("bpm")).map(|n| n.to_string()).unwrap_or_default(), num(&x["bpm"]));
        differs("root note", g.num(s, &p("rootNote")).map(|n| n.to_string()).unwrap_or_default(), num(&x["root_note"]));
        let iv: Vec<String> = x["scale_intervals"].as_array().unwrap().iter().map(|n| n.to_string()).collect();
        differs("scale's intervals", g.first(&g.first(s, &p("defaultScale")), &p("intervals")), iv.join(" "));
        differs("family", Index::local(&g.first(s, &p("inFamily"))), x["family"].as_str().unwrap().to_string());
        differs("groove", Index::local(&g.first(s, &p("defaultGroove"))), x["groove"].as_str().unwrap_or_default().to_string());
        for role in ["kick", "snare", "hihat"] {
            let r = g.first(s, &p(&format!("{role}Rhythm")));
            let have: Vec<String> = ["pulses", "steps", "rotation"].iter().map(|k| g.num(&r, &p(k)).map(|n| n.to_string()).unwrap_or_default()).collect();
            let want: Vec<String> = x["rhythms"][role].as_array().unwrap().iter().take(3).map(num).collect();
            differs(&format!("{role} rhythm"), have.join(","), want.join(","));
        }
        let mut have: Vec<String> = g.all(s, &p("hasProgression")).iter().map(|pr| g.first(pr, &p("degrees"))).collect();
        let mut want: Vec<String> = x["progressions"].as_array().unwrap().iter().map(|pr| pr["degrees"].as_array().unwrap().iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" ")).collect();
        have.sort();
        want.sort();
        differs("progressions", have.join(" | "), want.join(" | "));
    }

    // the roles a part plays (djlive's ROLES, what a section's roles and a role= name): the ontology's instrument roles
    let roles = g.of_type(&format!("{NS}InstrumentRole")).into_iter().map(|r| g.label(&r));
    same("instrument role", strs(&src["language"]["roles"]), roles, &mut errs);

    // the control registry: nouns, each noun's verbs, and the other names it answers to (its skos:altLabel: the noun btw
    // is also beatscript)
    let nouns = by(g, "ControlNoun", "writtenAs");
    let want: BTreeSet<String> = src["control_nouns"].as_array().unwrap().iter().map(|n| format!("do {}", n["name"].as_str().unwrap())).collect();
    same("control noun", want, nouns.keys().cloned(), &mut errs);
    for n in src["control_nouns"].as_array().unwrap() {
        let name = n["name"].as_str().unwrap();
        if let Some(s) = nouns.get(&format!("do {name}")) {
            let verbs = n["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
            let have = g.all(s, &format!("{NS}acceptsVerb")).into_iter().map(|v| g.label(&v));
            same(&format!("verb of {name}"), verbs, have, &mut errs);
            same(&format!("alias of {name}"), strs(&n["aliases"]), g.all(s, &format!("{SKOS}altLabel")), &mut errs);
        }
    }

    // djlive's registries
    let l = &src["language"];
    same("evolve operator", strs(&l["evolve_ops"]), by(g, "EvolveOperator", "writtenAs").into_keys(), &mut errs);
    same("mutation operator", strs(&l["mutation_ops"]), by(g, "MutationOperator", "writtenAs").into_keys(), &mut errs);
    let params = by(g, "PartParameter", "writtenAs").into_keys().filter_map(|w| w.strip_suffix('=').map(str::to_string));
    same("part parameter", strs(&l["part_params"]), params, &mut errs);
    let templates: BTreeSet<String> = l["templates"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    same("phrase template", templates, by(g, "PhraseTemplate", "writtenAs").into_keys(), &mut errs);
    let mut gens = strs(&l["generators"]);
    gens.extend(["choose", "deg"].iter().filter(|w| g.has(&format!("{SITE_BASE}/concept/keyword-{w}"))).map(|w| w.to_string()));
    same("generator", gens, by(g, "PatternGenerator", "writtenAs").into_keys(), &mut errs);
    let have_t: BTreeSet<String> = by(g, "PatternTransform", "writtenAs").into_keys().collect();
    for t in strs(&l["pattern_transforms"]) {
        if !have_t.contains(&t) {
            errs.push(format!("pattern transform {t:?} is in beatscript-serve but not in the ontology"));
        }
    }

    // the public-domain phrasebook: each meter family and feel the corpus is filed under, and every term its linked data writes
    let pb = &src["phrasebook"];
    same("meter family", pb["families"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "MeterFamily", "writtenAs").into_keys(), &mut errs);
    same("feel", pb["feels"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect(), by(g, "Feel", "writtenAs").into_keys(), &mut errs);
    for f in pb["families"].as_array().unwrap() {
        let iri = beatscript_serve::phrasebook::family_iri(f["id"].as_str().unwrap());
        if !g.has(&iri) {
            errs.push(format!("the phrasebook's family {iri} is not in the ontology"));
        }
    }
    for f in pb["feels"].as_array().unwrap() {
        let iri = beatscript_serve::phrasebook::feel_iri(f["id"].as_str().unwrap());
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
        if !g.has(&beatscript_serve::themes::role_iri(id)) {
            errs.push(format!("the theme role {} is not in the ontology", beatscript_serve::themes::role_iri(id)));
        }
    }
    for (id, _) in djengine::themes::MODES {
        if !g.has(&beatscript_serve::themes::mode_iri(id)) {
            errs.push(format!("the theme mode {} is not in the ontology", beatscript_serve::themes::mode_iri(id)));
        }
    }
    for (key, local) in beatscript_serve::themes::CONTEXT_TERMS {
        if !g.has(&format!("{NS}{local}")) {
            errs.push(format!("the theme library's linked data writes `{key}`, which is `ns:{local}`, and the ontology has no such term"));
        }
    }
    for (key, local) in beatscript_serve::phrasebook::CONTEXT_TERMS {
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
                    errs.push(format!("tool {server}/{name}: {what} differs (ontology {have:?}, beatscript-serve {want:?})"));
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

    /// The release in ontology/release/ against beatscript-serve's own grammar, instruments, registry and tools.
    #[test]
    fn ontology_matches_beatscript_serve() {
        let o = super::super::Ontology::load(PUBLIC).unwrap();
        let errs = drift(&o.graph);
        assert!(errs.is_empty(), "the ontology {} and beatscript-serve disagree ({} differences; run ontology/build.sh):\n{}", o.version, errs.len(), errs.join("\n"));
    }
}
