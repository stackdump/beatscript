//! The public-domain theme library on the site (docs/beatscript/forms.md, "The theme library"), the melodic twin of the phrasebook:
//! one corpus, many views. The corpus is `djengine`'s `data/themes-pd.txt`, CC0, every melody composed and first published
//! before 1929 and written from the melody itself. Its CID is the CID of that text (`djlive::compile::program_cid`, as the
//! phrasebook's is). Served as
//!
//! - `/themes`: a page, every theme with its bars, key, meter, source and pairings (docs.rs);
//! - `/themes.txt` and `/themes.jsonld`: the file, and the same corpus as linked data, one node a theme;
//! - `/themes/<CID>.txt` and `/themes/<CID>.jsonld`: the same, for the one CID this server ships (any other is `404`).
//!
//! The MCP tools read it from here too: `list_examples` with `themes` ([`list`]) and `genre_template` with `theme`
//! ([`splice`]). The linked data names its terms in the BeatScript ontology ([`CONTEXT_TERMS`]; a test holds each to a term).

use djengine::themes::{theme, themes, themes_text, themes_version, Theme, MODES, ROLES};
use serde_json::{json, Map, Value};

use crate::phrasebook::{ONTOLOGY_CONCEPT, ONTOLOGY_NS};

/// The page's path.
pub const PATH: &str = "/themes";
/// The page's title, and what it says it is.
pub const TITLE: &str = "Public-domain theme library";
pub const BLURB: &str = "Melodies, basses and figures composed and published before 1929, CC0, each written as BeatScript bars in a stated key, to borrow and morph: the melodic twin of the phrasebook.";

/// The text the corpus is, exactly as shipped: what its CID hashes and what `/themes.txt` is.
pub fn text() -> &'static str {
    themes_text()
}

/// The corpus's CID.
pub fn cid() -> String {
    djlive::compile::program_cid(text())
}

fn origin_of(origin: &str) -> &str {
    origin.trim_end_matches('/')
}

/// `<origin>/themes`.
pub fn url(origin: &str) -> String {
    format!("{}{PATH}", origin_of(origin))
}

/// A theme's IRI: `<origin>/themes#<id>` (the reference programs' linked data names their themes by it).
pub fn theme_iri(origin: &str, id: &str) -> String {
    format!("{}#{id}", url(origin))
}

/// A role's concept in the ontology.
pub fn role_iri(role: &str) -> String {
    format!("{ONTOLOGY_CONCEPT}theme-role-{role}")
}

/// A mode's concept in the ontology.
pub fn mode_iri(mode: &str) -> String {
    format!("{ONTOLOGY_CONCEPT}theme-mode-{mode}")
}

/// The document's own context terms: the key a node writes and the ontology term (a local name in [`ONTOLOGY_NS`]) it stands
/// for. The rest are schema.org's (`name`, `composer`, `musicalKey`, `dateCreated`, `description`).
pub const CONTEXT_TERMS: &[(&str, &str)] = &[
    ("ThemeLibrary", "ThemeLibrary"),
    ("ThemeEntry", "ThemeEntry"),
    ("inThemeLibrary", "inThemeLibrary"),
    ("role", "themeRole"),
    ("mode", "themeMode"),
    ("meter", "timeSignature"),
    ("steps", "cycleSteps"),
    ("bars", "barPattern"),
    ("lets", "letText"),
    ("part", "partText"),
    ("pairsWith", "pairsWith"),
    ("work", "themeWork"),
];

/// The context object the document carries beside BeatScript's own and schema.org's.
pub fn context() -> Value {
    let mut c = Map::new();
    c.insert("ont".into(), json!(ONTOLOGY_NS));
    c.insert("xsd".into(), json!("http://www.w3.org/2001/XMLSchema#"));
    for (key, local) in CONTEXT_TERMS {
        let v = match *key {
            "inThemeLibrary" | "role" | "mode" | "pairsWith" => json!({"@id": format!("ont:{local}"), "@type": "@id"}),
            "steps" => json!({"@id": format!("ont:{local}"), "@type": "xsd:integer"}),
            "bars" => json!({"@id": format!("ont:{local}"), "@container": "@list"}),
            _ => json!(format!("ont:{local}")),
        };
        c.insert((*key).into(), v);
    }
    Value::Object(c)
}

/// One theme as a node of the document.
fn entry_node(origin: &str, cid: &str, t: &Theme) -> Value {
    json!({
        "@id": theme_iri(origin, &t.id),
        "@type": ["ThemeEntry", "MusicComposition"],
        "inThemeLibrary": cid,
        "name": t.id,
        "work": t.work,
        "composer": {"@type": "Person", "name": t.composer},
        "dateCreated": t.year.to_string(),
        "role": role_iri(&t.role),
        "musicalKey": t.key,
        "mode": mode_iri(&t.mode),
        "meter": t.meter,
        "steps": t.steps.to_string(),
        "bars": t.bars,
        "lets": t.lets(),
        "part": t.part(),
        "pairsWith": t.pairs.iter().map(|p| theme_iri(origin, p)).collect::<Vec<_>>(),
        "description": t.note,
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
    })
}

/// The corpus as linked data: one node for the library (its `@id` is its CID) and one a theme.
pub fn jsonld(origin: &str) -> Value {
    let cid = cid();
    let entries: Vec<Value> = themes().iter().map(|t| entry_node(origin, &cid, t)).collect();
    json!({
        "@context": ["https://beatscript.app/schema", "https://schema.org", context()],
        "@type": ["ThemeLibrary", "Dataset"],
        "@version": "1",
        "@id": cid,
        "name": "BeatScript public-domain theme library",
        "description": "Melodies, basses and figures composed and published before 1929, each written from the melody itself as BeatScript bars in a stated key (a let a bar), with its role, meter, source, year and the themes it morphs well with. CC0; every melody is in the public domain.",
        "url": url(origin),
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
        "licenseId": "CC0-1.0",
        "version": themes_version().to_string(),
        "distribution": [
            {"@type": "DataDownload", "contentUrl": format!("{}{PATH}/{cid}.txt", origin_of(origin)), "encodingFormat": "text/plain"},
            {"@type": "DataDownload", "contentUrl": format!("{}{PATH}/{cid}.jsonld", origin_of(origin)), "encodingFormat": "application/ld+json"},
        ],
        "hasPart": entries,
    })
}

/// The answer for a path of the data routes, as `phrasebook::data`: outer None, not one of them; `Some(None)`, a `404`.
pub fn data(path: &str, origin: &str) -> Option<Option<crate::phrasebook::Data>> {
    use crate::phrasebook::Data;
    const TXT: &str = "text/plain; charset=utf-8";
    const LD: &str = "application/ld+json; charset=utf-8";
    let ld = || {
        let mut s = serde_json::to_string_pretty(&jsonld(origin)).expect("the document serializes");
        s.push('\n');
        s
    };
    match path {
        "/themes.txt" => return Some(Some(Data { content_type: TXT, body: text().to_string(), immutable: false })),
        "/themes.jsonld" => return Some(Some(Data { content_type: LD, body: ld(), immutable: false })),
        _ => {}
    }
    let rest = path.strip_prefix("/themes/")?;
    let Some((name, ext)) = rest.rsplit_once('.') else { return Some(None) };
    if !matches!(ext, "txt" | "jsonld") || name != cid() {
        return Some(None);
    }
    Some(Some(match ext {
        "txt" => Data { content_type: TXT, body: text().to_string(), immutable: true },
        _ => Data { content_type: LD, body: ld(), immutable: false },
    }))
}

/// Whether `path` is one of the data routes.
pub fn is_data_route(path: &str) -> bool {
    matches!(path, "/themes.txt" | "/themes.jsonld") || path.starts_with("/themes/")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One theme as a tool answers it: its bars and the `let` lines and part that play it, where it is from, and its pairings.
pub fn theme_json(t: &Theme) -> Value {
    json!({
        "id": t.id, "role": t.role, "key": t.key, "mode": t.mode, "meter": t.meter, "steps": t.steps,
        "bars": t.bars, "lets": t.lets(), "part": t.part(),
        "work": t.work, "composer": t.composer, "year": t.year, "pairs": t.pairs, "note": t.note,
    })
}

/// The theme library for `list_examples`: every theme, or those in a `key` (a key word: `d`, `bm`, `db`, matched on its
/// tonic), of a `role` and by a `composer` (any part of the name, any case). Err: a role that is not one, or nothing
/// matching, with what there is.
pub fn list(key: Option<&str>, role: Option<&str>, composer: Option<&str>) -> Result<Value, String> {
    let norm = |s: Option<&str>| s.map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty());
    let (key, role, composer) = (norm(key), norm(role), norm(composer));
    if let Some(r) = &role {
        if !ROLES.iter().any(|x| x.0 == r) {
            return Err(format!("no theme has the role {r:?}; the roles are {}", ROLES.iter().map(|x| x.0).collect::<Vec<_>>().join(", ")));
        }
    }
    let want_tonic = match &key {
        Some(k) => Some(tonic(k).ok_or_else(|| format!("key is a key word (d, bm, db, f#m), not {k:?}"))?),
        None => None,
    };
    let picked: Vec<Value> = themes()
        .iter()
        .filter(|t| want_tonic.map_or(true, |pc| tonic(&t.key) == Some(pc)))
        .filter(|t| role.as_ref().map_or(true, |r| &t.role == r))
        .filter(|t| composer.as_ref().map_or(true, |c| t.composer.to_ascii_lowercase().contains(c.as_str())))
        .map(theme_json)
        .collect();
    Ok(json!({
        "cid": cid(),
        "version": themes_version(),
        "license": "CC0-1.0",
        "about": "Public-domain melodies written as BeatScript bars, to borrow as found material: paste a theme's lets, play it with its part, morph it toward a pairing (~ a_* morph b_1 0.25), or start from genre_template with theme. The form templates are built from them (docs/forms.md).",
        "roles": ROLES.iter().map(|(r, about)| json!({"id": r, "about": about})).collect::<Vec<_>>(),
        "count": picked.len(),
        "themes": picked,
    }))
}

/// A key word's tonic as a pitch class: `d` 2, `db` 1, `f#m` 6. None: not a key word.
pub fn tonic(k: &str) -> Option<i32> {
    let k = k.trim().to_ascii_lowercase();
    let mut c = k.chars();
    let base: i32 = match c.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = c.collect();
    let (acc, mode) = match rest.as_bytes().first() {
        Some(b'#') => (1, &rest[1..]),
        Some(b'b') => (-1, &rest[1..]),
        _ => (0, rest.as_str()),
    };
    if !matches!(mode, "" | "m" | "maj" | "min" | "minor" | "major") && !mode.starts_with(':') {
        return None;
    }
    Some((base + acc).rem_euclid(12))
}

/// The semitones that take a theme from its key's tonic to `to` (a pitch class): the nearest way, -6 to 5.
pub fn shift(t: &Theme, to: i32) -> i32 {
    let from = tonic(&t.key).unwrap_or(0);
    let d = (to - from).rem_euclid(12);
    if d > 5 {
        d - 12
    } else {
        d
    }
}

/// What a genre template's part for each role is, as `genres::template` writes it.
fn role_part(role: &str) -> &'static str {
    match role {
        "melody" => "melody tuneline",
        "bass" => "bass bassline",
        "harmony" => "harmony chords",
        _ => "arp arpline",
    }
}

/// `genre_template` with `theme`: the theme's lets put in the program after its own, moved to the program's key (`tonic`, a
/// pitch class) with `transpose(...)` when it is in another, and played by the part of its role in every section that has one.
/// Returns the text and what the answer says of the theme. Err: no such theme, with the ids.
pub fn splice(program: &str, id: &str, tonic_to: i32) -> Result<(String, Value), String> {
    let id = id.trim().trim_start_matches('@');
    let t = theme(id).ok_or_else(|| format!("no theme {id:?}; the themes are {} (list_examples with themes true)", themes().iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")))?;
    let n = shift(t, tonic_to);
    let name = if n == 0 { t.id.clone() } else { format!("{}_here", t.id) };
    let mut lets = format!("# the theme: {}, {} ({}), from the public-domain theme library (CC0)\n{}\n", t.work, t.composer, t.year, t.lets());
    if n != 0 {
        lets.push_str(&format!("# in this key: {} {n:+} semitones from its own ({})\nlet {name} = transpose({}, {n})\n", t.id, t.key, t.id));
    }
    let old = role_part(&t.role);
    let new = format!("{}{} {name}", t.role, t.cycle_mark());
    let mut out = String::with_capacity(program.len() + lets.len());
    let mut placed = false;
    let mut sections = 0;
    for l in program.lines() {
        if !placed && l.starts_with("section ") {
            out.push_str(&lets);
            out.push('\n');
            placed = true;
        }
        let trimmed = l.trim_start();
        let is_part = (trimmed.starts_with("deck A:") || trimmed.starts_with("deck B:")) && trimmed.split(" | ").any(|p| p.trim_start_matches("deck A: ").trim_start_matches("deck B: ").starts_with(old));
        if is_part {
            sections += 1;
            out.push_str(&l.replacen(old, &new, 1));
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    if !placed {
        out.push_str(&lets);
    }
    let mut j = theme_json(t);
    j["transposed"] = json!(n);
    j["played_by"] = json!(new);
    j["played_in_sections"] = json!(sections);
    if sections == 0 {
        j["note"] = json!(format!("this genre's song structure has no {} part, so the theme's lets are in the program but nothing plays them: add `{new}` to a section's deck line", t.role));
    }
    Ok((out, j))
}

/// The page's body and its table of contents, `(level, id, text)` for each role.
pub fn page(origin: &str) -> (String, Vec<(u8, String, String)>) {
    let cid = cid();
    let ts = themes();
    let mut b = String::new();
    b.push_str(&format!("<h1>{}</h1>\n", esc(TITLE)));
    b.push_str(&format!(
        "<p>{} melodies, basses and figures, each <strong>composed and first published before 1929</strong> and written from the melody itself, never from a modern arrangement, edition or recording. \
         Each is BeatScript bars in a stated key: paste its <code>let</code> lines into a program, play it with its part, and transpose it with <code>transpose(id, n)</code> or <code>~ id_* up n</code>. \
         The file is CC0, curated by hand and small on purpose: the melodic twin of the <a href=\"/phrasebook\">phrasebook</a>. The <a href=\"/docs/forms\">form templates</a> are built from it.</p>\n",
        ts.len()
    ));
    b.push_str(&format!(
        "<p>The library has one CID, the CID of its text: <code>{cid}</code>. Its views are the file, <a href=\"{PATH}.txt\">/themes.txt</a>, and linked data, <a href=\"{PATH}.jsonld\">/themes.jsonld</a>, \
         each also at <code>/themes/&lt;CID&gt;.txt</code> and <code>/themes/&lt;CID&gt;.jsonld</code> for this CID only.</p>\n",
        cid = esc(&cid)
    ));
    let mut toc = Vec::new();
    for (role, about) in ROLES {
        let list: Vec<&Theme> = ts.iter().filter(|t| t.role == role).collect();
        if list.is_empty() {
            continue;
        }
        let id = format!("role-{role}");
        let mut name = role.to_string();
        name[..1].make_ascii_uppercase();
        toc.push((2u8, id.clone(), name.clone()));
        b.push_str(&format!("<h2 id=\"{id}\"><a class=\"anchor\" href=\"#{id}\" aria-label=\"Link to this section\">#</a>{}</h2>\n<p>A theme of this role is {}.</p>\n", esc(&name), esc(about)));
        b.push_str("<table class=\"phrasebook\">\n<thead><tr><th>Theme</th><th>From</th><th>Year</th><th>Key</th><th>Meter</th><th>Play it</th><th>Pairs with</th></tr></thead>\n<tbody>\n");
        for t in list {
            let pairs: Vec<String> = t.pairs.iter().map(|p| format!("<a href=\"#{p}\"><code>{p}</code></a>", p = esc(p))).collect();
            b.push_str(&format!(
                "<tr id=\"{id}\"><td><code>{id}</code></td><td>{composer}, <cite>{work}</cite></td><td>{year}</td><td>{key} {mode}</td><td>{meter}</td><td><code>{part}</code></td><td>{pairs}</td></tr>\n",
                id = esc(&t.id),
                composer = esc(&t.composer),
                work = esc(&t.work),
                year = t.year,
                key = esc(&t.key),
                mode = esc(&t.mode),
                meter = esc(&t.meter),
                part = esc(&t.part()),
                pairs = pairs.join(", "),
            ));
        }
        b.push_str("</tbody>\n</table>\n");
        for t in ts.iter().filter(|t| t.role == role) {
            b.push_str(&format!("<p><code>{}</code>: {}</p>\n<pre><code class=\"language-beatscript\">{}</code></pre>\n", esc(&t.id), esc(&t.note), esc(&t.lets())));
        }
    }
    b.push_str(&format!("<p>The modes are {}. The linked data is at <a href=\"{PATH}.jsonld\">/themes.jsonld</a>, in the terms of <a href=\"/ontology/\">the BeatScript ontology</a>.</p>\n", MODES.iter().map(|(m, about)| format!("<code>{m}</code> ({})", esc(about))).collect::<Vec<_>>().join(", ")));
    let _ = origin;
    (b, toc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_checks_as_its_part_plays_it() {
        // a theme's lets and its part make a program that checks with no warning, in its own key
        let env = djlive::env::Env { voices: Some(&djlive::env::StaticVoices::default()), ..djlive::env::Env::default() };
        for t in themes() {
            let src = format!("tempo 100\nkey {}{}\n{}\nsection heard 4 bars {{\n  deck A: {}\n}}\narrange {{\n  heard\n}}\n", t.key, if t.mode == "minor" { "m" } else { "" }, t.lets(), t.part());
            let c = djlive::compile::check_with(&src, &env);
            let bad: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error() || d.severity == djlive::Severity::Warning).map(|d| d.message.clone()).collect();
            assert!(c.ok() && bad.is_empty(), "{}: {bad:?}\n{src}", t.id);
        }
    }

    #[test]
    fn the_document_has_a_node_a_theme_and_its_terms_are_the_ontologys() {
        let v = jsonld("https://beatscript.app/");
        assert_eq!(v["@id"], cid());
        assert_eq!(v["hasPart"].as_array().unwrap().len(), themes().len());
        let ctx = context();
        for n in v["hasPart"].as_array().unwrap() {
            assert_eq!(n["inThemeLibrary"], cid());
            assert!(n["dateCreated"].as_str().unwrap().parse::<u32>().unwrap() < 1929);
            for (k, _) in n.as_object().unwrap() {
                assert!(k.starts_with('@') || ctx.get(k).is_some() || ["name", "composer", "musicalKey", "dateCreated", "description", "license"].contains(&k.as_str()), "{k} is in no context");
            }
            for p in n["pairsWith"].as_array().unwrap() {
                let id = p.as_str().unwrap().rsplit('#').next().unwrap();
                assert!(theme(id).is_some(), "{p}");
            }
        }
        for (key, local) in CONTEXT_TERMS {
            let t = &ctx[*key];
            assert_eq!(t.get("@id").and_then(Value::as_str).or_else(|| t.as_str()).unwrap(), format!("ont:{local}"));
        }
    }

    #[test]
    fn a_data_route_answers_for_its_cid_only() {
        let o = "https://beatscript.app";
        let c = cid();
        assert_eq!(data("/themes.txt", o).unwrap().unwrap().body, text());
        assert!(data(&format!("/themes/{c}.txt"), o).unwrap().unwrap().immutable);
        assert!(matches!(data("/themes/nope.txt", o), Some(None)));
        assert!(matches!(data(&format!("/themes/{}.txt", djlive::compile::program_cid("tempo 1\n")), o), Some(None)));
        assert!(data("/themes", o).is_none() && is_data_route("/themes.jsonld") && !is_data_route("/themes"));
    }

    #[test]
    fn the_library_filters_by_key_role_and_composer() {
        let all = list(None, None, None).unwrap();
        assert_eq!(all["count"].as_u64().unwrap() as usize, themes().len());
        let d = list(Some("d"), None, None).unwrap();
        assert!(d["count"].as_u64().unwrap() >= 4 && d["themes"].as_array().unwrap().iter().all(|t| tonic(t["key"].as_str().unwrap()) == Some(2)));
        assert!(list(Some("dm"), None, None).unwrap()["count"] == d["count"], "a key is matched on its tonic");
        let b = list(None, Some("bass"), None).unwrap();
        assert!(b["themes"].as_array().unwrap().iter().all(|t| t["role"] == "bass"));
        assert!(list(None, None, Some("beethoven")).unwrap()["count"].as_u64().unwrap() >= 2);
        assert!(list(None, Some("tune"), None).unwrap_err().contains("the roles are"));
        assert!(list(Some("h"), None, None).is_err());
        assert_eq!((tonic("db"), tonic("bm"), tonic("f#m"), tonic("c"), tonic("bb")), (Some(1), Some(11), Some(6), Some(0), Some(10)));
    }

    #[test]
    fn a_theme_spliced_into_a_program_is_moved_to_its_key() {
        let prog = "tempo 120\nkey e\nlet tuneline = c4\nsection intro 8 bars {\n  deck B: harmony chords | melody tuneline inst=flute\n}\narrange intro\n";
        let (t, j) = splice(prog, "ode_to_joy", 4).unwrap();
        assert!(t.contains("let ode_to_joy_1 = ") && t.contains("let ode_to_joy_here = transpose(ode_to_joy, 2)") && t.contains("melody ode_to_joy_here inst=flute"), "{t}");
        assert_eq!((j["transposed"].as_i64(), j["played_in_sections"].as_i64()), (Some(2), Some(1)));
        let (t, j) = splice(prog, "@gymnopedie_chords", 2).unwrap();
        assert!(t.contains("harmony/12 gymnopedie_chords") && j["transposed"] == 0, "{t}");
        let (_, j) = splice(prog, "mars_ostinato", 7).unwrap();
        assert_eq!(j["played_in_sections"], 0);
        assert!(j["note"].as_str().unwrap().contains("no bass part"));
        assert!(splice(prog, "nope", 0).unwrap_err().contains("ode_to_joy"));
    }

    #[test]
    fn the_page_lists_every_theme_with_no_inline_style() {
        let (html, toc) = page("https://beatscript.app");
        for t in themes() {
            assert!(html.contains(&format!("<tr id=\"{}\">", t.id)) && html.contains(&format!("<td>{}</td>", t.year)), "{}", t.id);
        }
        assert!(!toc.is_empty() && html.contains(&cid()));
        assert!(!html.contains("style=") && !html.contains("<style"));
    }
}
