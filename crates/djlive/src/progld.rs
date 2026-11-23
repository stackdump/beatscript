//! A program as an embedded JSON-LD block: `<beatscript-view>` with its source in a child `<script
//! type="application/ld+json">`, the way pflow.xyz embeds a net in `<petri-view>` (docs/SCHEMA.md).
//!
//! ```html
//! <beatscript-view data-editor data-ui="bar"><script type="application/ld+json">
//! {"@context":["https://beatscript.app/schema","https://schema.org"],"@type":["Program","MusicComposition"],
//!  "@version":"1","@id":"z4EBG9...","name":"night bus","license":"CC0-1.0","tempo":128,"key":"am","text":"tempo 128\n..."}
//! </script></beatscript-view>
//! ```
//!
//! `text` is the canonical program text, so its CID is `@id`: a reader ([`read_view`]) recomputes it and refuses a
//! block that does not hash to its own name. One builder for every host: beatscript-serve renders it into its pages, the
//! MCP `get` answers it, and the phone (which links djlive) reads it with the same function. Pure: no I/O, no clock.

use djengine::json::{JsonObject, JsonValue};

use crate::compile::{program_cid, Compiled};

/// The vocabulary the block names first in its `@context` (served at that URL: the context for a JSON-LD processor,
/// the doc page for a person).
pub const SCHEMA_URL: &str = "https://beatscript.app/schema";
/// The format version of the block (`@version`), as pflow.xyz versions its documents: a change to the shape is a new
/// version.
pub const FORMAT_VERSION: &str = "1";

/// What the block says besides the compiled program: the rights and provenance kept outside the CID (as the stored
/// program's meta has them).
#[derive(Clone, Copy, Default)]
pub struct Facts<'a> {
    pub name: Option<&'a str>,
    /// The SPDX id (`CC0-1.0`, `CC-BY-4.0`, `all-rights-reserved`). None: CC0, the language's default.
    pub license: Option<&'a str>,
    pub author: Option<&'a str>,
    /// The CID of the program this one was forked from.
    pub forked_from: Option<&'a str>,
    /// The author's signature over the CID (a signed share: the link's `.<sig>`); opaque here, as in share.rs.
    pub signature: Option<&'a str>,
    /// `#! genre:` as written (a name, CURIE or IRI): linked when the vocabulary has the term.
    pub genre: Option<&'a str>,
    /// `#! tags:` as written.
    pub tags: &'a [String],
    /// The bitwrap ontology, to link genre, tags, the key's scale and the instruments to its terms. None: no links.
    pub ontology: Option<&'a dyn crate::linked::Ontology>,
    /// The doc comments of the text as written (djlive::doc; the canonical text has none): each documented section is in
    /// `section` with its doc as `description`. Empty: no `section` key.
    pub docs: &'a [crate::doc::DocComment],
}

/// The IRI of a licence: the Creative Commons deed for CC0 and CC BY, rightsstatements.org's "In Copyright" for all rights
/// reserved. An id the language does not know is given back as it is (None: nothing to link).
pub fn license_iri(id: &str) -> Option<&'static str> {
    use djengine::rights::License;
    match License::parse(id)? {
        License::AllRightsReserved => Some("http://rightsstatements.org/vocab/InC/1.0/"),
        l => l.url(),
    }
}

/// The ontology's scale for a key's mode (`am` is `scale/minor`); None when it has no such term.
fn scale_iri(o: &dyn crate::linked::Ontology, k: crate::music::Key) -> Option<String> {
    let local = match crate::music::MODES.get(k.mode)? {
        &"harmonic" => "harmonic-minor",
        &"pent" => "pentatonic",
        &"minpent" => "minor-pentatonic",
        m => m,
    };
    o.terms().iter().find(|t| t.kind == "scale" && t.local == local).map(|t| t.iri.clone())
}

/// The block's document for a compiled program. Keys in this order (a reader may rely on nothing but the names).
pub fn document(c: &Compiled, f: &Facts) -> JsonValue {
    let mut d = JsonObject::new();
    d.set("@context", JsonValue::Array(vec![JsonValue::str(SCHEMA_URL), JsonValue::str("https://schema.org")]));
    d.set("@type", JsonValue::Array(vec![JsonValue::str("Program"), JsonValue::str("MusicComposition")]));
    d.set("@version", JsonValue::str(FORMAT_VERSION));
    d.set("@id", JsonValue::str(c.cid.clone()));
    if let Some(n) = f.name.map(str::trim).filter(|n| !n.is_empty()) {
        d.set("name", JsonValue::str(n));
    }
    // the licence as an IRI (what a reader links), and its SPDX id beside it (the djrust: document's licenseId)
    let license = f.license.map(str::trim).filter(|l| !l.is_empty()).unwrap_or("CC0-1.0");
    match license_iri(license) {
        Some(u) => d.set("license", JsonValue::str(u)),
        None => d.set("license", JsonValue::str(license)),
    }
    d.set("licenseId", JsonValue::str(djengine::rights::License::parse(license).map_or(license, |l| l.id())));
    if let Some(t) = c.tempo {
        d.set("tempo", JsonValue::Number(t));
    }
    if let Some(k) = c.key {
        d.set("key", JsonValue::str(crate::music::canonical_pitch_word(&k.text())));
        if let Some(iri) = f.ontology.and_then(|o| scale_iri(o, k)) {
            d.set("scale", JsonValue::str(iri));
        }
    }
    if let Some(o) = f.ontology {
        if let Some(g) = f.genre.and_then(|g| crate::linked::find(o, g, Some("genre")).ok()) {
            d.set("genre", JsonValue::str(g.iri.clone()));
        }
        let mut tags: Vec<String> = f.tags.iter().filter_map(|t| crate::linked::find(o, t, None).ok().map(|t| t.iri.clone())).collect();
        tags.sort();
        tags.dedup();
        if !tags.is_empty() {
            d.set("tag", JsonValue::Array(tags.into_iter().map(JsonValue::str).collect()));
        }
        // the voices the parts name that the vocabulary has as instruments (`inst=acid`)
        let mut inst: Vec<String> = c
            .decks
            .values()
            .chain(c.segments.iter().flat_map(|s| s.decks.values()))
            .flat_map(|d| d.parts.iter().filter_map(|p| p.instrument.clone()))
            .filter_map(|n| o.terms().iter().find(|t| t.kind == "instrument" && t.local == n).map(|t| t.iri.clone()))
            .collect();
        inst.sort();
        inst.dedup();
        if !inst.is_empty() {
            d.set("instrument", JsonValue::Array(inst.into_iter().map(JsonValue::str).collect()));
        }
    }
    // the sections' doc comments, carried beside the program (the text and its CID are the same without them)
    let secs: Vec<JsonValue> = f
        .docs
        .iter()
        .filter(|d| d.kind == "section")
        .map(|d| JsonValue::Object(JsonObject::from_pairs([("name", JsonValue::str(d.name.clone())), ("description", JsonValue::str(d.doc.clone()))])))
        .collect();
    if !secs.is_empty() {
        d.set("section", JsonValue::Array(secs));
    }
    d.set("text", JsonValue::str(c.text.clone()));
    // what it is based on (refs.rs `based_on`): the engine calls and borrows its text makes, each with the CID it resolves to,
    // and what an eject vendored into it (its `# from:` lines); outside the CID, like the doc comments
    let basis = crate::refs::based_on(&c.text, &c.notes);
    if !basis.is_empty() {
        let generator = crate::gen::generator().map(|g| g.name().to_string());
        let items: Vec<JsonValue> = basis
            .iter()
            .map(|b| {
                let mut o = JsonObject::from_pairs([("@type", JsonValue::str("CreativeWork")), ("name", JsonValue::str(b.call.clone()))]);
                if let Some(cid) = &b.cid {
                    o.set("resolves", JsonValue::str(cid.clone()));
                }
                if let (Some(v), Some(g)) = (b.call.strip_prefix("gen@").and_then(|r| r.split(' ').next()), &generator) {
                    o.set("pinnedGenerator", JsonValue::str(format!("{g}@{v}")));
                }
                o.set("derivation", JsonValue::str(b.how));
                JsonValue::Object(o)
            })
            .collect();
        d.set("isBasedOn", JsonValue::Array(items));
    }
    // a recipe (a program with references) and the program it writes out to: the expanded CID (refs.rs `expanded`); a borrow
    // needs the host's programs, so only what resolves without one is said here
    if let Ok(Some((_, x))) = crate::refs::expanded(&c.text, &crate::env::Env::none()) {
        if x != c.cid {
            d.set("expandsTo", JsonValue::str(x));
        }
    }
    if let Some(a) = f.author.filter(|a| !a.is_empty()) {
        d.set("author", JsonValue::str(a));
    }
    if let Some(p) = f.forked_from.filter(|p| !p.is_empty()) {
        d.set("forkedFrom", JsonValue::str(p));
    }
    if let Some(s) = f.signature.filter(|s| !s.is_empty()) {
        d.set("signature", JsonValue::str(s));
    }
    JsonValue::Object(d)
}

/// The document as text safe inside a `<script>` element: JSON, with `<`, `>` and `&` written as `<` `>`
/// `&` (they only occur inside strings, so it is the same JSON), and U+2028/U+2029 escaped too. A program's text
/// cannot close the script or open a comment.
pub fn script_json(doc: &JsonValue) -> String {
    let s = doc.js_stringified();
    let mut o = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '<' => o.push_str("\\u003c"),
            '>' => o.push_str("\\u003e"),
            '&' => o.push_str("\\u0026"),
            '\u{2028}' => o.push_str("\\u2028"),
            '\u{2029}' => o.push_str("\\u2029"),
            c => o.push(c),
        }
    }
    o
}

/// The element: `<beatscript-view>` with `data-editor` (the page's editor loads it) and `data-ui` (`bar`: the
/// transport bar), its source in the child script. `editor` false: a view on the page that the editor does not load
/// (a playlist's other items).
pub fn view_html(doc: &JsonValue, editor: bool, ui: &str) -> String {
    let ui: String = ui.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    format!(
        "<beatscript-view{} data-ui=\"{}\"><script type=\"application/ld+json\">{}</script></beatscript-view>",
        if editor { " data-editor" } else { "" },
        if ui.is_empty() { "bar" } else { &ui },
        script_json(doc)
    )
}

/// A block read back and believed: its CID, its canonical text, the whole document.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub cid: String,
    pub text: String,
    pub doc: JsonValue,
}

/// The first `<beatscript-view>` of a page (with `data-editor` if `editor`), its document parsed and checked: `@id` is a
/// CID and `text` hashes to it. The text being canonical is what makes the two agree (a reformatted text has another
/// CID), so a block that does not is refused, not repaired.
pub fn read_view(html: &str, editor: bool) -> Result<Read, String> {
    let mut rest = html;
    loop {
        let at = rest.find("<beatscript-view").ok_or("the page has no <beatscript-view> with a program")?;
        let open_end = rest[at..].find('>').ok_or("an unclosed <beatscript-view> tag")? + at;
        let tag = &rest[at..open_end];
        let body = &rest[open_end + 1..];
        if editor && !tag.contains("data-editor") {
            rest = body;
            continue;
        }
        let s = body.find("<script").ok_or("the <beatscript-view> has no child script")?;
        let s_end = body[s..].find('>').ok_or("an unclosed <script> tag")? + s;
        if !body[s..s_end].contains("application/ld+json") {
            return Err("the <beatscript-view>'s script is not application/ld+json".into());
        }
        let close = body[s_end + 1..].find("</script>").ok_or("the <beatscript-view>'s script is never closed")? + s_end + 1;
        return read_document(&body[s_end + 1..close]);
    }
}

/// The block's JSON on its own (a `.jsonld` body, the MCP `get` field), checked as [`read_view`] checks it.
pub fn read_document(json: &str) -> Result<Read, String> {
    let doc = JsonValue::parse(json.trim()).map_err(|e| format!("the program block is not JSON: {e:?}"))?;
    let cid = doc.get("@id").and_then(JsonValue::as_str).ok_or("the program block has no @id")?.to_string();
    let text = doc.get("text").and_then(JsonValue::as_str).ok_or("the program block has no text")?.to_string();
    let types_ok = match doc.get("@type") {
        Some(JsonValue::Array(a)) => a.iter().any(|t| t.as_str() == Some("Program")),
        Some(t) => t.as_str() == Some("Program"),
        None => false,
    };
    if !types_ok {
        return Err("the block's @type is not a Program".into());
    }
    let got = program_cid(&text);
    if got != cid {
        return Err(format!("the block's text hashes to {got}, not its @id {cid}: it is not that program (or its text is not canonical)"));
    }
    Ok(Read { cid, text, doc })
}

/// The JSON-LD context [`SCHEMA_URL`] serves to a processor: the block's terms in the BeatScript vocabulary (`bs:`,
/// `@vocab`), the rest left to schema.org, which the block names second (so its terms win where both define one:
/// `name`, `text`, `license`, `author`). The beats terms (`forkedFrom`, `signature`) keep their names.
pub fn schema_context() -> JsonValue {
    let term = |iri: &str, id: bool| {
        let mut o = JsonObject::from_pairs([("@id", JsonValue::str(iri))]);
        if id {
            o.set("@type", JsonValue::str("@id"));
        }
        JsonValue::Object(o)
    };
    let mut c = JsonObject::new();
    c.set("@vocab", JsonValue::str(format!("{SCHEMA_URL}#")));
    c.set("bs", JsonValue::str(format!("{SCHEMA_URL}#")));
    c.set("bw", JsonValue::str(crate::linked::BW_NS));
    c.set("schema", JsonValue::str("https://schema.org/"));
    c.set("Program", JsonValue::str("bs:Program"));
    c.set("MusicComposition", JsonValue::str("schema:MusicComposition"));
    c.set("text", JsonValue::str("bs:text"));
    c.set("tempo", JsonValue::str("bs:tempo"));
    c.set("key", JsonValue::str("bs:key"));
    c.set("license", term("bs:license", true));
    c.set("licenseId", JsonValue::str("bs:licenseId"));
    c.set("genre", term("bs:genre", true));
    c.set("tag", JsonValue::Object(JsonObject::from_pairs([("@id", JsonValue::str("bs:tag")), ("@type", JsonValue::str("@id")), ("@container", JsonValue::str("@set"))])));
    c.set("scale", term("bs:scale", true));
    c.set("instrument", JsonValue::Object(JsonObject::from_pairs([("@id", JsonValue::str("bs:instrument")), ("@type", JsonValue::str("@id")), ("@container", JsonValue::str("@set"))])));
    c.set("forkedFrom", term("bs:forkedFrom", false));
    c.set("encoding", term("schema:encoding", false));
    c.set("signature", JsonValue::str("bs:signature"));
    c.set("author", JsonValue::str("bs:author"));
    // provenance: what a program is based on (schema.org's `isBasedOn`, each entry a CreativeWork named by its call), the CID
    // each resolves to, the generator version pinned, and how it came in; a recipe's written-out program, and back
    c.set("isBasedOn", JsonValue::Object(JsonObject::from_pairs([("@id", JsonValue::str("schema:isBasedOn")), ("@container", JsonValue::str("@set"))])));
    c.set("resolves", term("bs:resolves", false));
    c.set("pinnedGenerator", JsonValue::str("bs:pinnedGenerator"));
    c.set("derivation", JsonValue::str("bs:derivation"));
    c.set("expandsTo", term("bs:expandsTo", false));
    c.set("expandedFrom", term("bs:expandedFrom", false));
    // a documented section: `name` and `description` are schema.org's
    c.set("section", JsonValue::Object(JsonObject::from_pairs([("@id", JsonValue::str("bs:section")), ("@container", JsonValue::str("@list"))])));
    JsonValue::Object(JsonObject::from_pairs([("@context", JsonValue::Object(c))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "tempo 128\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";

    fn compiled(src: &str) -> Compiled {
        crate::compile::check(src).compiled.unwrap()
    }

    #[test]
    fn the_block_is_the_program_and_reads_back() {
        let c = compiled(SRC);
        let d = document(&c, &Facts { name: Some("night bus"), signature: Some("0xabc"), ..Facts::default() });
        let j = d.js_stringified();
        assert!(j.starts_with(r#"{"@context":["https://beatscript.app/schema","https://schema.org"],"@type":["Program","MusicComposition"],"@version":"1","@id":""#), "{j}");
        assert!(j.contains(r#""name":"night bus","license":"https://creativecommons.org/publicdomain/zero/1.0/","licenseId":"CC0-1.0","tempo":128,"key":"am","text":"tempo 128\nkey am\n"#), "{j}");
        assert!(j.ends_with(r#","signature":"0xabc"}"#), "{j}");
        let page = format!("<html><body>{}</body></html>", view_html(&d, true, "bar"));
        assert!(page.contains(r#"<beatscript-view data-editor data-ui="bar"><script type="application/ld+json">{"#));
        let r = read_view(&page, true).unwrap();
        assert_eq!(r.cid, c.cid);
        assert_eq!(r.text, c.text);
        assert_eq!(r.doc, d);
    }

    #[test]
    fn a_sections_doc_comment_is_its_description_and_the_cid_stays() {
        let src = "tempo 128\n# drop: the kick and the bass\nsection drop 1 bars {\n  deck A: kick x ~ x ~\n}\narrange drop\n";
        let c = compiled(src);
        let docs = crate::doc::doc_comments(src);
        let j = document(&c, &Facts { docs: &docs, ..Facts::default() }).js_stringified();
        assert!(j.contains(r#""section":[{"name":"drop","description":"the kick and the bass"}],"text":"#), "{j}");
        // the same program without the comment: the same @id, and no section key
        let bare = compiled("tempo 128\nsection drop 1 bars {\n  deck A: kick x ~ x ~\n}\narrange drop\n");
        assert_eq!(bare.cid, c.cid);
        assert!(!document(&bare, &Facts::default()).js_stringified().contains(r#""section""#));
        assert!(read_document(&j).is_ok(), "the block still reads back as its CID");
    }

    #[test]
    fn a_script_cannot_be_closed_from_inside() {
        // a `#! name:` or a string with markup in it stays inside the JSON
        let c = compiled(SRC);
        let d = document(&c, &Facts { name: Some("</script><script>alert(1)</script> & <!--"), ..Facts::default() });
        let h = view_html(&d, true, "bar\"><x");
        assert_eq!(h.matches("</script>").count(), 1, "{h}");
        assert!(!h.contains("<!--") && h.contains(r#"data-ui="barx""#), "{h}");
        assert_eq!(read_view(&h, true).unwrap().doc, d);
    }

    #[test]
    fn a_block_that_is_not_its_cid_is_refused() {
        let c = compiled(SRC);
        let d = document(&c, &Facts::default());
        let tampered = d.js_stringified().replace("kick x ~ x ~", "kick x x x x");
        assert!(read_document(&tampered).unwrap_err().contains("not its @id"));
        // a text that is not canonical hashes to something else too
        let spaced = d.js_stringified().replace("tempo 128\\n", "tempo   128\\n");
        assert!(read_document(&spaced).is_err());
        assert!(read_view("<p>nothing here</p>", true).is_err());
        let not_editor = view_html(&d, false, "bar");
        assert!(read_view(&not_editor, true).is_err());
        assert!(read_view(&not_editor, false).is_ok());
    }

    #[test]
    fn genre_tags_scale_and_instruments_link_to_the_ontology() {
        use crate::linked::{StaticOntology, Term};
        let o = StaticOntology::new(vec![
            Term::new("genre", "techno", "techno"),
            Term::new("scale", "minor", "minor"),
            Term::new("instrument", "acid", "acid"),
            Term::new("groove", "four-on-floor", "four on floor"),
        ]);
        let c = compiled("tempo 128\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~ inst=acid\n");
        let tags = vec!["four on floor".to_string(), "not a term".to_string()];
        let d = document(&c, &Facts { genre: Some("techno"), tags: &tags, ontology: Some(&o), ..Facts::default() });
        let s = |k: &str| d.get(k).map(JsonValue::js_stringified);
        assert_eq!(s("genre").as_deref(), Some(r#""https://ontology.bitwrap.io/genre/techno""#));
        assert_eq!(s("scale").as_deref(), Some(r#""https://ontology.bitwrap.io/scale/minor""#));
        assert_eq!(s("instrument").as_deref(), Some(r#"["https://ontology.bitwrap.io/instrument/acid"]"#));
        assert_eq!(s("tag").as_deref(), Some(r#"["https://ontology.bitwrap.io/groove/four-on-floor"]"#));
        // with no vocabulary there are no links (and the key stays a plain word)
        let bare = document(&c, &Facts { genre: Some("techno"), ..Facts::default() });
        assert!(bare.get("genre").is_none() && bare.get("scale").is_none() && bare.get("key").is_some());
    }

    #[test]
    fn license_and_provenance_when_known() {
        let c = compiled(SRC);
        let d = document(&c, &Facts { license: Some("CC-BY-4.0"), author: Some("did:key:z6Mk"), forked_from: Some("z4EBG9parent"), ..Facts::default() });
        assert_eq!(d.get("license").and_then(JsonValue::as_str), Some("https://creativecommons.org/licenses/by/4.0/"));
        assert_eq!(d.get("licenseId").and_then(JsonValue::as_str), Some("CC-BY-4.0"));
        let arr = document(&c, &Facts { license: Some("all-rights-reserved"), ..Facts::default() });
        assert_eq!(arr.get("license").and_then(JsonValue::as_str), Some("http://rightsstatements.org/vocab/InC/1.0/"));
        assert_eq!(arr.get("licenseId").and_then(JsonValue::as_str), Some("all-rights-reserved"));
        assert_eq!(d.get("author").and_then(JsonValue::as_str), Some("did:key:z6Mk"));
        assert_eq!(d.get("forkedFrom").and_then(JsonValue::as_str), Some("z4EBG9parent"));
        assert!(d.get("signature").is_none() && d.get("name").is_none());
    }
}
