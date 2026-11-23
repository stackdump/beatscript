//! The public-domain phrasebook on the site (docs/beatscript/phonemes.md, "The public-domain phrasebook"): one corpus, many views.
//! The corpus is `djengine`'s `data/phrasebook-pd.txt`, CC0, every text first published before 1929. Its CID is the CID of
//! that text (`djlive::compile::program_cid`, the function a program's CID is: no second hash scheme). Served as
//!
//! - `/phrasebook`: a page, every line with its year, meter, stress as a step pattern and a `sing` example (docs.rs);
//! - `/phrasebook.txt` and `/phrasebook.jsonld`: the file, and the same corpus as linked data, one node a line;
//! - `/phrasebook/<CID>.txt` and `/phrasebook/<CID>.jsonld`: the same, for the one CID this server ships. Any other CID is
//!   `404`: a new version of the corpus is a new CID, and an old one is not served from here.
//!
//! The linked data names its terms in the BeatScript ontology (`https://beatscript.app/ontology/ns#`): [`CONTEXT_TERMS`] is
//! the document's own context, key by key, and a test holds every one to a term the ontology release has.

use djengine::vocal::phoneme::{pd_phrasebook_text, pd_phrasebook_version, pd_phrases, PdPhrase, PD_FAMILIES};
use serde_json::{json, Map, Value};

/// The page's path.
pub const PATH: &str = "/phrasebook";
/// The page's title, and what it says it is.
pub const TITLE: &str = "Public-domain phrasebook";
pub const BLURB: &str = "Lines of verse, scripture, speech and work song first published before 1929, CC0, each with its meter as a step pattern, to sing by id.";

/// The ontology's namespace (`ontology/ns#`) and its concepts' base (`ontology/concept/`): the document's terms are defined there.
pub const ONTOLOGY_NS: &str = "https://beatscript.app/ontology/ns#";
pub const ONTOLOGY_CONCEPT: &str = "https://beatscript.app/ontology/concept/";

/// The text the corpus is, exactly as shipped (LF, one newline at the end): what its CID hashes and what `/phrasebook.txt` is.
pub fn text() -> &'static str {
    pd_phrasebook_text()
}

/// The corpus's CID.
pub fn cid() -> String {
    djlive::compile::program_cid(text())
}

fn origin_of(origin: &str) -> &str {
    origin.trim_end_matches('/')
}

/// `<origin>/phrasebook`.
pub fn url(origin: &str) -> String {
    format!("{}{PATH}", origin_of(origin))
}

pub fn txt_url(origin: &str, cid: &str) -> String {
    format!("{}{PATH}/{cid}.txt", origin_of(origin))
}

pub fn jsonld_url(origin: &str, cid: &str) -> String {
    format!("{}{PATH}/{cid}.jsonld", origin_of(origin))
}

/// The document's own context terms: the key a node writes, and the ontology term (a local name in [`ONTOLOGY_NS`]) it stands for.
/// `@type` coercions are in [`context`]. These are the concepts of the record: a phrasebook entry, its meter (the family and the
/// name), its stress pattern, its feel, the year it was first published, its edition, its source work and its author.
pub const CONTEXT_TERMS: &[(&str, &str)] = &[
    ("Phrasebook", "Phrasebook"),
    ("PhrasebookEntry", "PhrasebookEntry"),
    ("inPhrasebook", "inPhrasebook"),
    ("family", "meterFamily"),
    ("meter", "meter"),
    ("feel", "feel"),
    ("stress", "stressPattern"),
    ("syllables", "syllableCount"),
    ("call", "callSyllables"),
    ("lyric", "lyric"),
    ("firstPublished", "firstPublished"),
    ("edition", "edition"),
    ("sourceWork", "sourceWork"),
    ("sourceAuthor", "sourceAuthor"),
    ("sourceUrl", "sourceUrl"),
];

/// The context object the document carries beside BeatScript's own and schema.org's (as a stored program's JSON-LD carries a
/// local one for its rights): every key mapped to the ontology.
pub fn context() -> Value {
    let mut c = Map::new();
    c.insert("ont".into(), json!(ONTOLOGY_NS));
    c.insert("xsd".into(), json!("http://www.w3.org/2001/XMLSchema#"));
    for (key, local) in CONTEXT_TERMS {
        let coerce = match *key {
            "inPhrasebook" | "family" | "feel" | "sourceUrl" => Some("@id"),
            "syllables" | "call" => Some("xsd:integer"),
            "firstPublished" => Some("xsd:gYear"),
            _ => None,
        };
        c.insert((*key).into(), match coerce {
            Some(t) => json!({"@id": format!("ont:{local}"), "@type": t}),
            None => json!(format!("ont:{local}")),
        });
    }
    Value::Object(c)
}

/// A family's concept in the ontology.
pub fn family_iri(family: &str) -> String {
    format!("{ONTOLOGY_CONCEPT}phrasebook-family-{family}")
}

/// A feel's concept in the ontology.
pub fn feel_iri(feel: &str) -> String {
    format!("{ONTOLOGY_CONCEPT}phrasebook-feel-{feel}")
}

/// One line as a node of the document.
fn entry_node(origin: &str, cid: &str, p: &PdPhrase) -> Value {
    let mut n = Map::new();
    n.insert("@id".into(), json!(format!("{}#{}", url(origin), p.id)));
    n.insert("@type".into(), json!(["PhrasebookEntry", "Quotation"]));
    n.insert("inPhrasebook".into(), json!(cid));
    n.insert("name".into(), json!(p.id));
    n.insert("text".into(), json!(p.text));
    n.insert("lyric".into(), json!(p.lyric));
    n.insert("family".into(), json!(family_iri(&p.family)));
    n.insert("meter".into(), json!(p.meter));
    n.insert("feel".into(), json!(feel_iri(&p.feel)));
    n.insert("stress".into(), json!(p.stress));
    n.insert("syllables".into(), json!(p.syllables.to_string()));
    if p.call > 0 {
        n.insert("call".into(), json!(p.call.to_string()));
    }
    n.insert("firstPublished".into(), json!(p.year.to_string()));
    n.insert("sourceAuthor".into(), json!(p.author));
    n.insert("sourceWork".into(), json!(p.work));
    n.insert("edition".into(), json!(p.edition));
    n.insert("sourceUrl".into(), json!(p.source));
    n.insert("description".into(), json!(p.note));
    Value::Object(n)
}

/// The corpus as linked data: one node for the phrasebook (its `@id` is its CID, as a program block's is) and one a line,
/// each line `Quotation` to schema.org. Keys in this order; built from the file, so it cannot disagree with it.
pub fn jsonld(origin: &str) -> Value {
    let cid = cid();
    let entries: Vec<Value> = pd_phrases().iter().map(|p| entry_node(origin, &cid, p)).collect();
    json!({
        "@context": ["https://beatscript.app/schema", "https://schema.org", context()],
        "@type": ["Phrasebook", "Dataset"],
        "@version": "1",
        "@id": cid,
        "name": "BeatScript public-domain phrasebook",
        "description": "Lines of verse, scripture, speech and work song first published before 1929, to sing by id (sing @tempest_such_stuff on melody). Each carries its source, its meter as a step pattern (x stressed, ~ not), its feel and the lyric BeatScript sings it as. CC0; every text is in the public domain.",
        "url": url(origin),
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
        "licenseId": "CC0-1.0",
        "version": pd_phrasebook_version().to_string(),
        "distribution": [
            {"@type": "DataDownload", "contentUrl": txt_url(origin, &cid), "encodingFormat": "text/plain"},
            {"@type": "DataDownload", "contentUrl": jsonld_url(origin, &cid), "encodingFormat": "application/ld+json"},
        ],
        "hasPart": entries,
    })
}

/// What a data route answers: the body and its type, and whether the bytes can never change (a path named by their CID).
pub struct Data {
    pub content_type: &'static str,
    pub body: String,
    pub immutable: bool,
}

/// The answer for a path of the data routes. Outer `None`: not one of them. `Some(None)`: one of them, and `404`: a CID
/// route for a CID this server does not ship.
pub fn data(path: &str, origin: &str) -> Option<Option<Data>> {
    const TXT: &str = "text/plain; charset=utf-8";
    const LD: &str = "application/ld+json; charset=utf-8";
    let ld = || {
        let mut s = serde_json::to_string_pretty(&jsonld(origin)).expect("the document serializes");
        s.push('\n');
        s
    };
    match path {
        "/phrasebook.txt" => return Some(Some(Data { content_type: TXT, body: text().to_string(), immutable: false })),
        "/phrasebook.jsonld" => return Some(Some(Data { content_type: LD, body: ld(), immutable: false })),
        _ => {}
    }
    let rest = path.strip_prefix("/phrasebook/")?;
    let Some((name, ext)) = rest.rsplit_once('.') else { return Some(None) };
    if !matches!(ext, "txt" | "jsonld") {
        return Some(None);
    }
    // only the CID this server ships: anything else, well formed or not, is not found
    if name != cid() {
        return Some(None);
    }
    Some(Some(match ext {
        "txt" => Data { content_type: TXT, body: text().to_string(), immutable: true },
        _ => Data { content_type: LD, body: ld(), immutable: false },
    }))
}

/// Whether `path` is one of the data routes (a CID or not).
pub fn is_data_route(path: &str) -> bool {
    matches!(path, "/phrasebook.txt" | "/phrasebook.jsonld") || path.starts_with("/phrasebook/")
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// The example a page shows whole: a program to copy, with the part the line is sung on written out (the first bar's notes,
/// then rests), checked with no warning (a test holds it to that).
pub const EXAMPLE: &str = "tempo 80\nkey am\ndeck A: melody <[a3 c4 e4 g4 a4 g4 e4 c4 a3 c4 e4 g4 a4 g4 e4 c4] [a3 c4 e4 g4 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]>\ndeck A: sing @tempest_such_stuff on melody\n";

/// The page's body and the table of contents the docs shell wraps it in: `(level, id, text)` for each family.
pub fn page(origin: &str) -> (String, Vec<(u8, String, String)>) {
    let cid = cid();
    let ps = pd_phrases();
    let mut b = String::new();
    b.push_str(&format!("<h1>{}</h1>\n", esc(TITLE)));
    b.push_str(&format!(
        "<p>{} lines of verse, scripture, speech and work song, each <strong>first published before 1929</strong>, to sing by id: <code>sing @tempest_such_stuff on melody</code>. \
         The file is CC0 and every text in it is in the public domain: quoted from the pre-1929 edition its line names, never from a modern edition or translation. \
         It is curated by hand, in ten families of three to five lines. A line takes as many notes as it has syllables.</p>\n",
        ps.len()
    ));
    b.push_str(&format!(
        "<figure class=\"code\" data-beatscript=\"1\" data-try=\"1\"><figcaption><span class=\"lang\">BeatScript</span><span class=\"acts\"><button type=\"button\" class=\"cp\" hidden>Copy</button><button type=\"button\" class=\"try\" hidden>Try it</button></span></figcaption><pre><code class=\"language-beatscript\">{}</code></pre></figure>\n",
        esc(EXAMPLE)
    ));
    b.push_str("<p>Each line's <code>stress</code> is its meter as a step pattern, one step a syllable, <code>x</code> stressed and <code>~</code> not, written as <code>deck A: hihat ~ x ~ x</code> reads it. \
         <code>feel</code> says how it sits on the grid: <code>straight</code> on the sixteenths, <code>triplet</code> in groups of three. \
         Where a line is a call and an answer, <code>call</code> is how many of its first syllables the leader sings.</p>\n");
    b.push_str(&format!(
        "<p>The corpus has one CID, the CID of its text: <code>{cid}</code>. Its views are the file, <a href=\"{PATH}.txt\">/phrasebook.txt</a>, and linked data, <a href=\"{PATH}.jsonld\">/phrasebook.jsonld</a>, \
         and each is also at <code>/phrasebook/&lt;CID&gt;.txt</code> and <code>/phrasebook/&lt;CID&gt;.jsonld</code>, which answer only for this CID: a new version of the corpus is a new CID.</p>\n",
        cid = esc(&cid)
    ));
    let mut toc = Vec::new();
    for f in PD_FAMILIES {
        let lines: Vec<&PdPhrase> = ps.iter().filter(|p| p.family == f.id).collect();
        if lines.is_empty() {
            continue;
        }
        let id = format!("family-{}", f.id);
        let mut name = f.name.to_string();
        if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        toc.push((2u8, id.clone(), name.clone()));
        b.push_str(&format!("<h2 id=\"{id}\"><a class=\"anchor\" href=\"#{id}\" aria-label=\"Link to this section\">#</a>{}</h2>\n<p>{}</p>\n", esc(&name), esc(f.about)));
        b.push_str("<table class=\"phrasebook\">\n<thead><tr><th>Phrase</th><th>Says</th><th>From</th><th>Year</th><th>Meter</th><th>Stress</th><th>Syllables</th><th>Sing it</th></tr></thead>\n<tbody>\n");
        for p in lines {
            b.push_str(&format!(
                "<tr id=\"{id}\"><td><code>@{id}</code></td><td>{text}</td><td>{author}, <cite>{work}</cite></td><td>{year}</td><td>{meter}</td><td><code>{stress}</code></td><td>{syl}</td><td><code>sing @{id} on melody</code></td></tr>\n",
                id = esc(&p.id),
                text = esc(&p.text),
                author = esc(&p.author),
                work = esc(&p.work),
                year = p.year,
                meter = esc(&p.meter),
                stress = esc(&p.stress),
                syl = p.syllables,
            ));
        }
        b.push_str("</tbody>\n</table>\n");
    }
    b.push_str("<p>Dates are as honestly as they are known: a shanty's year is the earliest collection this wording was read in, the Mother Goose rhymes' is the collection's documented date of 1780, and each line's <code>note</code> in the file says where a date or a text has a caveat. ");
    b.push_str(&format!("The linked data is at <a href=\"{}\">/phrasebook.jsonld</a>, in the terms of <a href=\"/ontology/\">the BeatScript ontology</a>.</p>\n", PATH));
    let _ = origin; // the page's links are the site's own paths
    (b, toc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cid_is_the_programs_cid_function_over_the_files_text() {
        let c = cid();
        assert!(c.starts_with('z') && c.len() > 40, "{c}");
        assert_eq!(c, djlive::compile::program_cid(pd_phrasebook_text()));
        // a different text is a different CID: a new version of the corpus is a new CID
        assert_ne!(c, djlive::compile::program_cid(&format!("{}# one more\n", text())));
        // it names the file whole, as written
        assert!(text().ends_with('\n') && !text().contains('\r'));
    }

    #[test]
    fn the_document_has_a_node_for_the_corpus_and_one_for_every_line() {
        let v = jsonld("https://beatscript.app/");
        assert_eq!(v["@id"], cid());
        assert_eq!(v["@context"][0], "https://beatscript.app/schema");
        assert_eq!(v["@type"][0], "Phrasebook");
        assert_eq!(v["license"], "https://creativecommons.org/publicdomain/zero/1.0/");
        let parts = v["hasPart"].as_array().unwrap();
        assert_eq!(parts.len(), pd_phrases().len());
        // the distribution links the corpus by its CID
        assert_eq!(v["distribution"][0]["contentUrl"], format!("https://beatscript.app/phrasebook/{}.txt", cid()));
        // one node a line, in the file's order, with the fields the file has
        for (n, p) in parts.iter().zip(pd_phrases()) {
            assert_eq!(n["@id"], format!("https://beatscript.app/phrasebook#{}", p.id));
            assert_eq!(n["@type"], json!(["PhrasebookEntry", "Quotation"]));
            assert_eq!(n["inPhrasebook"], cid());
            assert_eq!(n["text"], p.text);
            assert_eq!(n["lyric"], p.lyric);
            assert_eq!(n["stress"], p.stress);
            assert_eq!(n["syllables"], p.syllables.to_string());
            assert_eq!(n["firstPublished"], p.year.to_string());
            assert_eq!(n["sourceUrl"], p.source);
            assert_eq!(n["family"], family_iri(&p.family));
            assert_eq!(n.get("call").is_some(), p.call > 0, "{}", p.id);
        }
        // every key a node writes is a term of the document's own context, or one schema.org gives
        let ctx = context();
        for n in parts {
            for (k, _) in n.as_object().unwrap() {
                assert!(k.starts_with('@') || ctx.get(k).is_some() || ["name", "text", "description"].contains(&k.as_str()), "{k} is in no context");
            }
        }
        // and the document is JSON that round-trips
        let s = serde_json::to_string_pretty(&v).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&s).unwrap(), v);
    }

    #[test]
    fn every_context_term_maps_into_the_ontology() {
        let ctx = context();
        for (key, local) in CONTEXT_TERMS {
            let t = &ctx[*key];
            let id = t.get("@id").and_then(Value::as_str).or_else(|| t.as_str()).unwrap();
            assert_eq!(id, format!("ont:{local}"), "{key}");
        }
        assert_eq!(ctx["ont"], ONTOLOGY_NS);
        assert_eq!(ctx["firstPublished"]["@type"], "xsd:gYear");
        // the family and feel IRIs are concepts of the ontology's; each family the file uses has one
        for p in pd_phrases() {
            assert!(family_iri(&p.family).starts_with(ONTOLOGY_CONCEPT) && feel_iri(&p.feel).starts_with(ONTOLOGY_CONCEPT));
        }
    }

    #[test]
    fn a_data_route_answers_for_the_cid_it_ships_and_no_other() {
        let o = "https://beatscript.app";
        let c = cid();
        let txt = data("/phrasebook.txt", o).unwrap().unwrap();
        assert_eq!(txt.body, text());
        assert!(txt.content_type.starts_with("text/plain") && !txt.immutable);
        let ld = data("/phrasebook.jsonld", o).unwrap().unwrap();
        assert!(ld.content_type.starts_with("application/ld+json"));
        assert_eq!(serde_json::from_str::<Value>(&ld.body).unwrap()["@id"], c);
        // by CID: the same bytes, the text immutable
        let by = data(&format!("/phrasebook/{c}.txt"), o).unwrap().unwrap();
        assert_eq!(by.body, text());
        assert!(by.immutable);
        assert_eq!(data(&format!("/phrasebook/{c}.jsonld"), o).unwrap().unwrap().body, ld.body);
        // another CID, a malformed one, another extension, another depth: not found
        let other = djlive::compile::program_cid("tempo 120\n");
        for bad in [format!("/phrasebook/{other}.txt"), format!("/phrasebook/{other}.jsonld"), format!("/phrasebook/{c}.json"), format!("/phrasebook/{c}"), "/phrasebook/nope.txt".to_string(), format!("/phrasebook/{c}.txt/x"), "/phrasebook/".to_string()] {
            assert!(matches!(data(&bad, o), Some(None)), "{bad} should be a route that is not found");
        }
        // and what is not a phrasebook route is not one
        assert!(data("/phrasebook", o).is_none() && data("/schema", o).is_none());
        assert!(is_data_route("/phrasebook.txt") && is_data_route(&format!("/phrasebook/{c}.txt")) && !is_data_route("/phrasebook"));
    }

    #[test]
    fn the_page_lists_every_line_with_its_year_meter_stress_and_a_sing_example() {
        let (html, toc) = page("https://beatscript.app");
        assert_eq!(toc.len(), 10, "a heading a family");
        for p in pd_phrases() {
            assert!(html.contains(&format!("<tr id=\"{}\">", p.id)), "{}", p.id);
            assert!(html.contains(&format!("<code>sing @{} on melody</code>", p.id)), "{}", p.id);
            assert!(html.contains(&format!("<td>{}</td>", p.year)) && html.contains(&format!("<code>{}</code>", p.stress)), "{}", p.id);
        }
        assert!(html.contains(&cid()) && html.contains("data-try=\"1\""));
        // no inline style (the CSP is style-src 'self'), no raw `<` from the data
        assert!(!html.contains("style=") && !html.contains("<style"), "inline style");
        // what a person reads (the markup's own `data-beatscript` attribute aside) says BeatScript, never the short name
        let mut visible = String::new();
        let mut in_tag = false;
        for c in html.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => visible.push(c),
                _ => {}
            }
        }
        assert!(!visible.replace(&cid(), "").to_lowercase().contains("btw"), "BeatScript is the name, never the short one");
        assert_eq!(html.matches("<table").count(), 10);
    }

    #[test]
    fn the_example_is_a_whole_program_that_checks_with_no_warning() {
        let env = djlive::env::Env { voices: Some(&djlive::env::StaticVoices::default()), ..djlive::env::Env::default() };
        let c = djlive::compile::check_with(EXAMPLE, &env);
        assert!(c.ok() && c.diagnostics.is_empty(), "{}", djlive::diag::render_all(&c.diagnostics, EXAMPLE));
    }
}
