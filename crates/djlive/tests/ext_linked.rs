//! F14 ontology links (docs/btw/ext-linked.md): `#! genre:` and `#! tags:` header lines validated against a
//! vocabulary through the `Ontology` trait (a fake one here, and the real cached snapshot of `djhost` read off disk:
//! nothing in this file touches the network), their did-you-mean with line and column, and the JSON-LD document.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djengine::rights::{Credit, License, Rights, Visibility};
use djlive::compile::{check, check_with};
use djlive::diag::Severity;
use djlive::env::Env;
use djlive::linked::{self, Facts, Links, Ontology, StaticOntology, Term};

/// A small made-up vocabulary: enough kinds to test each rule, with names that clash on purpose (`bossa` is a genre
/// and a groove; `bass` an instrument and a role).
fn fake() -> StaticOntology {
    StaticOntology::new(vec![
        Term::new("genre", "techno", "techno"),
        Term::new("genre", "house", "house"),
        Term::new("genre", "dnb", "dnb"),
        Term::new("genre", "bossa", "bossa"),
        Term::new("groove", "bossa", "bossa"),
        Term::new("groove", "four-on-floor", "four on floor"),
        Term::new("instrument", "acid", "acid"),
        Term::new("instrument", "bass", "bass"),
        Term::new("role", "bass", "bass"),
        Term::new("section", "drop", "drop"),
        Term::rhythm("techno-kick", "techno kick: E(4,16) rotation 0", 4, 16, 0),
        Term::rhythm("edm-kick", "edm kick: E(4,16) rotation 0", 4, 16, 0),
        Term::rhythm("techno-snare", "techno snare: E(2,16) rotation 4", 2, 16, 4),
    ])
}

fn with<'a>(o: &'a StaticOntology) -> Env<'a> {
    Env { ontology: Some(o), ..Env::default() }
}

const PROGRAM: &str = "tempo 128\nkey am\ndeck A: kick e(4,16) | snare e(2,16,4)\n";

/// The diagnostics that are about the header block (a header line's value, or a `#!` line out of place): what F14 says.
/// A program with `e(4,16)` in it also gets F6's notes that the ontology names the rhythm (`check` always says those: the
/// table is constant), which are about the program and not about the header.
fn about_the_header<'a>(c: &'a djlive::compile::Check, src: &str) -> Vec<&'a djlive::diag::Diagnostic> {
    let end = linked::header_end(src);
    c.diagnostics.iter().filter(|d| d.span.end <= end || d.message.contains("line is not read")).collect()
}

// ---- headers are comments

#[test]
fn header_lines_are_comments_not_in_the_cid_with_or_without_a_vocabulary() {
    let plain = check(PROGRAM).compiled.unwrap();
    let headed = format!("#! genre: techno\n#! tags: acid, four on floor\n{PROGRAM}");
    let o = fake();
    for c in [check(&headed), check_with(&headed, &with(&o))] {
        let c = c.compiled.expect("a program with header lines compiles");
        assert_eq!(c.cid, plain.cid, "the header lines are comments: not in the CID");
        assert_eq!(c.text, plain.text, "nor in the canonical text");
    }
    // a genre or tags change is the same program
    let other = format!("#! genre: house\n#! tags: drop\n{PROGRAM}");
    assert_eq!(check_with(&other, &with(&o)).compiled.unwrap().cid, plain.cid);
}

#[test]
fn with_no_vocabulary_nothing_is_said_about_the_lines() {
    let src = format!("#! genre: no-such-thing\n#! tags: zzz\n{PROGRAM}");
    let c = check(&src);
    assert!(about_the_header(&c, &src).is_empty(), "{:?}", c.diagnostics);
    assert!(c.diagnostics.iter().all(|d| d.message.contains("the ontology's")), "only F6's notes on the rhythms: {:?}", c.diagnostics);
    assert!(c.ok());
}

// ---- finding a term

#[test]
fn a_genre_is_found_by_iri_curie_kind_prefix_local_name_and_label() {
    let o = fake();
    let want = "https://ontology.bitwrap.io/genre/techno";
    for spelling in [
        "https://ontology.bitwrap.io/genre/techno",
        "https://ontology.bitwrap.io/ns#Techno", // the contract's spelling
        "bw:Techno",
        "bw:techno",
        "genre:techno",
        "Genre:Techno",
        "techno",
        "  TECHNO ",
    ] {
        assert_eq!(linked::resolve_genre(&o, spelling).unwrap().iri, want, "{spelling}");
    }
}

#[test]
fn a_bare_word_two_kinds_have_goes_to_the_first_kind_and_the_prefix_says_the_other() {
    let o = fake();
    assert_eq!(linked::resolve_tag(&o, "bossa").unwrap().kind, "genre");
    assert_eq!(linked::resolve_tag(&o, "groove:bossa").unwrap().kind, "groove");
    assert_eq!(linked::resolve_tag(&o, "bass").unwrap().kind, "instrument");
    assert_eq!(linked::resolve_tag(&o, "role:bass").unwrap().kind, "role");
    // a genre line wants a genre, so the bare word is the genre
    assert_eq!(linked::resolve_genre(&o, "bossa").unwrap().kind, "genre");
}

#[test]
fn a_tag_is_found_by_label_with_any_spacing_and_case() {
    let o = fake();
    for spelling in ["four on floor", "Four On Floor", "four-on-floor", "four_on_floor", "groove:four-on-floor", "  four   on  floor "] {
        assert_eq!(linked::resolve_tag(&o, spelling).unwrap().iri, "https://ontology.bitwrap.io/groove/four-on-floor", "{spelling}");
    }
}

#[test]
fn a_wrong_kind_says_what_it_is() {
    let o = fake();
    let e = linked::resolve_genre(&o, "groove:four-on-floor").unwrap_err();
    assert_eq!(e.message, "`groove:four-on-floor` is a groove, not a genre");
    let e = linked::resolve_genre(&o, "acid").unwrap_err();
    assert!(e.message.contains("an instrument, not a genre"), "{}", e.message);
    // by IRI too
    let e = linked::resolve_genre(&o, "https://ontology.bitwrap.io/instrument/acid").unwrap_err();
    assert!(e.message.contains("an instrument, not a genre"), "{}", e.message);
}

#[test]
fn an_unknown_term_has_a_did_you_mean() {
    let o = fake();
    let e = linked::resolve_genre(&o, "tehcno").unwrap_err();
    assert_eq!(e.message, "unknown genre `tehcno`");
    assert_eq!(e.help.as_deref(), Some("did you mean `techno`?"));
    assert_eq!(e.text(), "unknown genre `tehcno`: did you mean `techno`?");
    let e = linked::resolve_tag(&o, "acidd").unwrap_err();
    assert_eq!(e.message, "unknown tag `acidd`");
    assert_eq!(e.help.as_deref(), Some("did you mean `acid`?"));
    // an IRI under the vocabulary's base gets IRIs back
    let e = linked::resolve_genre(&o, "https://ontology.bitwrap.io/genre/tehcno").unwrap_err();
    assert_eq!(e.help.as_deref(), Some("did you mean `https://ontology.bitwrap.io/genre/techno`?"));
    // the right name under the wrong path is pointed at
    let e = linked::resolve_tag(&o, "https://ontology.bitwrap.io/section/acid").unwrap_err();
    assert_eq!(e.help.as_deref(), Some("did you mean `https://ontology.bitwrap.io/instrument/acid`?"));
    // an IRI of somebody else's is unknown, with nothing to suggest
    let e = linked::resolve_tag(&o, "http://example.org/genre/techno").unwrap_err();
    assert!(e.message.starts_with("unknown tag"), "{}", e.message);
    // nothing near a genre: the genres are listed
    let e = linked::resolve_genre(&o, "zzzzzzzz").unwrap_err();
    assert!(e.help.is_none() && e.expected.contains(&"techno".to_string()) && e.expected.contains(&"dnb".to_string()), "{e:?}");
    // an empty value
    assert!(linked::resolve_genre(&o, "  ").unwrap_err().message.contains("names no genre"));
}

#[test]
fn links_are_canonical_sorted_and_a_set() {
    let o = fake();
    let a = linked::canonical_links(&o, Some("bw:Techno"), &["drop".into(), "acid".into(), "four on floor".into(), "acid".into()]).unwrap();
    let b = linked::canonical_links(&o, Some("https://ontology.bitwrap.io/genre/techno"), &["Four-On-Floor".into(), "instrument:acid".into(), "section:drop".into()]).unwrap();
    assert_eq!(a, b, "the same links in any spelling or order");
    assert_eq!(a.tags.len(), 3);
    assert!(a.tags.windows(2).all(|w| w[0] < w[1]), "sorted, each once: {:?}", a.tags);
    assert_eq!(a.header_text(), "#! genre: https://ontology.bitwrap.io/genre/techno\n#! tags: https://ontology.bitwrap.io/groove/four-on-floor, https://ontology.bitwrap.io/instrument/acid, https://ontology.bitwrap.io/section/drop\n");
    // what is written back reads back as the same links
    let back = linked::read_links(&a.header_text());
    let again = linked::canonical_links(&o, back.genre.as_deref(), &back.tags).unwrap();
    assert_eq!(again, a);
    assert!(Links::default().header_text().is_empty());
    // every problem is reported, not just the first
    let e = linked::canonical_links(&o, Some("tehcno"), &["acidd".into(), "drop".into(), "nope".into()]).unwrap_err();
    assert_eq!(e.len(), 3);
}

// ---- check: diagnostics with line and column

#[test]
fn check_says_unknown_genre_with_line_and_column() {
    let o = fake();
    let src = format!("#! genre: tehcno\n#! tags: acid, acidd, four on flor\n{PROGRAM}");
    let c = check_with(&src, &with(&o));
    assert!(c.ok(), "a header line is a comment: a warning never stops a program");
    let ds: Vec<_> = about_the_header(&c, &src).into_iter().map(|d| (d.severity, d.position(&src), d.message.clone(), d.help.clone())).collect();
    assert_eq!(ds.len(), 3, "{ds:?}");
    // sorted by place: the genre on line 1 from column 11 ("#! genre: " is ten characters)
    assert_eq!((ds[0].0, ds[0].1), (Severity::Warning, (1, 11, 17)));
    assert_eq!(ds[0].2, "unknown genre `tehcno`");
    assert_eq!(ds[0].3.as_deref(), Some("did you mean `techno`?"));
    // the tags: column of each item ("#! tags: " is nine, "acid, " six more)
    assert_eq!((ds[1].1, ds[1].2.as_str()), ((2, 16, 21), "unknown tag `acidd`"));
    assert_eq!(ds[1].3.as_deref(), Some("did you mean `acid`?"));
    assert_eq!(ds[2].1 .0, 2);
    assert!(ds[2].2.contains("four on flor"), "{ds:?}");
    assert!(ds[2].3.as_deref().unwrap().contains("four-on-floor"), "{ds:?}");
    // the caret form points under the word
    let r = c.diagnostics[0].render(&src);
    assert!(r.contains("--> 1:11") && r.contains("#! genre: tehcno") && r.contains("^^^^^^"), "{r}");
    // and the program is still the program
    assert_eq!(c.compiled.unwrap().cid, check(PROGRAM).compiled.unwrap().cid);
}

#[test]
fn the_same_check_is_an_error_when_the_host_asks_for_one() {
    let o = fake();
    let src = format!("#! genre: tehcno\n{PROGRAM}");
    let (links, ds) = linked::check_headers(&src, &o, Severity::Error);
    assert!(links.is_empty());
    assert_eq!(ds.len(), 1);
    assert!(ds[0].is_error());
    let (links, ds) = linked::check_headers("#! genre: bw:Techno\n#! tags: acid\n", &o, Severity::Error);
    assert!(ds.is_empty());
    assert_eq!(links.genre.as_deref(), Some("https://ontology.bitwrap.io/genre/techno"));
    assert_eq!(links.tags, vec!["https://ontology.bitwrap.io/instrument/acid".to_string()]);
}

#[test]
fn a_wrong_kind_a_second_genre_and_a_genre_with_commas_are_said() {
    let o = fake();
    let src = format!("#! genre: groove:four-on-floor\n#! genre: house\n#! tags: drop\n{PROGRAM}");
    let (_, ds) = linked::check_headers(&src, &o, Severity::Warning);
    let msgs: Vec<&str> = ds.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(msgs, ["`groove:four-on-floor` is a groove, not a genre", "a program has one genre"], "{msgs:?}");
    assert_eq!(ds[1].position(&src).0, 2);
    let (_, ds) = linked::check_headers("#! genre: techno, house\n", &o, Severity::Warning);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].message, "a program has one genre");
    assert_eq!(ds[0].position("#! genre: techno, house\n"), (1, 11, 24));
}

#[test]
fn a_line_after_the_program_has_begun_is_not_read_and_says_so() {
    let o = fake();
    let src = format!("tempo 128\n#! genre: tehcno\n{}", "deck A: kick x ~ x ~\n");
    let c = check_with(&src, &with(&o));
    assert!(c.ok());
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    let d = &c.diagnostics[0];
    assert_eq!(d.severity, Severity::Warning);
    assert!(d.message.contains("`#! genre:` line is not read"), "{}", d.message);
    assert_eq!(d.position(&src).0, 2);
    // a plain comment before them ends the block too
    let src = "# a comment first\n#! tags: nope\ndeck A: kick x\n";
    assert_eq!(check_with(src, &with(&o)).diagnostics.len(), 1);
    // other `#!` lines are the rights': not ours to judge
    let src = "#! btw 1\n#! name: x\n#! license: CC0-1.0\ndeck A: kick x\n";
    assert!(check_with(src, &with(&o)).diagnostics.is_empty());
}

#[test]
fn header_lines_have_exact_spans_with_crlf_and_odd_spacing() {
    let src = "#!   genre:    techno  \r\n#! tags:acid,   drop ,\r\n#! btw 1\r\ntempo 1\r\n";
    let h = linked::header_lines(src);
    assert_eq!(h.iter().map(|x| (x.key.as_str(), x.value.as_str())).collect::<Vec<_>>(), [("genre", "techno"), ("tags", "acid,   drop ,")]);
    assert_eq!(h[0].value_span.text(src), "techno");
    assert_eq!(h[1].value_span.text(src), "acid,   drop ,");
    assert_eq!(linked::read_links(src), linked::RawLinks { genre: Some("techno".into()), tags: vec!["acid".into(), "drop".into()] });
    assert_eq!(&src[linked::header_end(src)..], "tempo 1\r\n");
    let o = fake();
    let (links, ds) = linked::check_headers(src, &o, Severity::Warning);
    assert!(ds.is_empty(), "{ds:?}");
    assert_eq!(links.tags.len(), 2);
}

// ---- Euclidean terms

/// The Euclidean shapes of a program as the document lists them: (pulses, steps, rotation, uses).
fn shapes(src: &str) -> Vec<(u32, u32, u32, u32)> {
    let c = check(src);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&c.diagnostics, src)));
    comp.linked.euclids.iter().map(|(s, n)| (s.pulses, s.steps, s.rotation, *n)).collect()
}

#[test]
fn the_euclid_calls_of_a_program_are_found_by_shape() {
    let t = shapes("deck A: kick e(3,8) | snare euclid(2,16,4)\ndeck B: hihat e(3,8) | bass e(5,8,10)\n");
    assert_eq!(t, [(3, 8, 0, 2), (5, 8, 2, 1), (2, 16, 4, 1)], "ordered by steps, then pulses; a rotation is taken modulo the steps");
    assert_eq!(shapes("deck A: kick e(0,8)\n").len(), 1, "no pulses is a rhythm of rests");
    assert_eq!(shapes("deck A: kick e(3,8,-6)\n"), [(3, 8, 2, 1)], "a turn the other way is the same rhythm");
    // not rhythms: a call of something else, a transform's name, no euclid at all
    assert!(shapes("deck A: kick every(2,x)\n").is_empty());
}

#[test]
fn the_euclid_calls_are_read_from_what_plays_not_from_the_text() {
    // a def's argument, arithmetic, a def called twice and a section placed twice: the text says `$k`, `$d` and `2 + 1`, and
    // what plays is e(3,8) and e(5,8)
    let src = "def pat(k) {\n  deck B: snare e($k, 8)\n}\ndef hat(d) {\n  deck $d: hihat e(5, 8)\n}\nsection verse 2 bars {\n  deck A: kick e(2 + 1, 8)\n  pat(3)\n  hat(A)\n  hat(B)\n}\narrange verse verse\n";
    assert_eq!(shapes(src), [(3, 8, 0, 4), (5, 8, 0, 4)], "each copy the expansion makes counts: a section placed twice says its lines twice");
    // a def nobody calls plays nothing
    assert!(shapes("def pat(k) {\n  deck B: snare e(3, 8)\n}\ndeck A: kick x\n").is_empty());
    // a part with a cycle of its own, a chance term, a choice, an alternation
    let src = "deck A: hihat/12 e(5,12)?0.5 | kick <e(3,8) e(4,16)> | snare e(2,16,4)?0.25\n";
    assert_eq!(shapes(src), [(3, 8, 0, 1), (5, 12, 0, 1), (2, 16, 4, 1), (4, 16, 0, 1)], "by steps, then pulses");
}

// ---- the JSON-LD document

fn song() -> String {
    "tempo 128\nkey am\nsection intro 8 bars {\n  deck A: kick e(3,8) | hihat e(3,8)\n}\nsection drop 16 bars {\n  deck A: kick e(4,16) | snare e(2,16,4)\n}\narrange intro drop\n".to_string()
}

fn rights() -> Rights {
    Rights {
        license: License::CcBy4,
        visibility: Visibility::Public,
        author: Some("did:key:z6MkTest".into()),
        forked_from: Some("z4EBG9parentparentparent".into()),
        credits: vec![Credit { cid: "z4EBG9parentparentparent".into(), author: Some("did:key:z6MkParent".into()), license: License::CcBy4, name: Some("night bus".into()) }],
    }
}

fn document(src: &str, o: Option<&dyn Ontology>) -> JsonValue {
    let comp = check(src).compiled.unwrap();
    let r = rights();
    let links = Links::new(Some("https://ontology.bitwrap.io/genre/techno".into()), vec!["https://ontology.bitwrap.io/instrument/acid".into(), "https://ontology.bitwrap.io/groove/four-on-floor".into()]);
    linked::jsonld(&comp, &Facts { name: Some("night bus 2"), rights: Some(&r), links: Some(&links) }, o)
}

/// A document against its fixture; `UPDATE_GOLDEN=1` rewrites it (read the diff before keeping it).
fn golden(file: &str, doc: &JsonValue) {
    let doc = linked::pretty(doc);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, format!("{doc}\n")).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(doc, golden.trim_end_matches('\n'), "the JSON-LD changed; if it should, rewrite tests/fixtures/{file} (UPDATE_GOLDEN=1) after reading the diff:\n{doc}");
}

#[test]
fn the_document_is_what_it_is_golden() {
    let o = fake();
    golden("linked-golden.jsonld", &document(&song(), Some(&o)));
}

#[test]
fn kick_e_3_8_is_the_ontologys_house_snare_golden() {
    // the smallest program with a rhythm the ontology names: one E(3,8), on a kick (the ontology's E(3,8) is a snare:
    // the shape is what is named, whatever role plays it), no rights, no links, no name
    let comp = check("tempo 120\ndeck A: kick e(3,8)\n").compiled.unwrap();
    let doc = linked::jsonld(&comp, &Facts::default(), None);
    golden("linked-kick-e38.jsonld", &doc);
    // and the same with the cached vocabulary of the host: it only repeats what the table already says
    let real = real();
    assert_eq!(linked::pretty(&linked::jsonld(&comp, &Facts::default(), Some(&real))), linked::pretty(&doc));
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    assert_eq!(eu.len(), 1);
    let e = &eu[0];
    assert_eq!(e.get("name").and_then(JsonValue::as_str), Some("house snare"));
    assert_eq!(e.get("sameShapeAs").unwrap().strings().unwrap(), ["https://ontology.bitwrap.io/rhythm/house-snare"]);
    assert_eq!((e.get("bw:pulses").and_then(JsonValue::as_f64), e.get("bw:steps").and_then(JsonValue::as_f64), e.get("bw:rotation").and_then(JsonValue::as_f64)), (Some(3.0), Some(8.0), Some(0.0)));
    // the names are the table's, so the snapshot of the host never contradicts them: every rhythm of the snapshot is in the table
    for t in real.terms().iter().filter(|t| t.shape.is_some()) {
        let s = t.shape.unwrap();
        let named = djlive::meter::euclid_names(i64::from(s.pulses), i64::from(s.steps), i64::from(s.rotation));
        assert!(named.iter().any(|n| n.iri == t.iri), "{} is in the snapshot and not in F6's table", t.iri);
    }
    assert_eq!(real.terms().iter().filter(|t| t.shape.is_some()).count(), djlive::meter::EUCLID_TABLE.len(), "the table and the snapshot hold the same individuals");
}

#[test]
fn the_document_has_the_program_its_links_its_sections_and_its_rhythms() {
    let o = fake();
    let doc = document(&song(), Some(&o));
    let cid = check(&song()).compiled.unwrap().cid;
    assert_eq!(doc.get("@type").and_then(JsonValue::as_str), Some("djrust:Program"));
    assert_eq!(doc.get("@id").and_then(JsonValue::as_str), Some(format!("https://beats.bitwrap.io/p#{cid}").as_str()));
    assert_eq!(doc.get("cid").and_then(JsonValue::as_str), Some(cid.as_str()));
    assert_eq!(doc.get("genre").and_then(JsonValue::as_str), Some("https://ontology.bitwrap.io/genre/techno"));
    assert_eq!(doc.get("tag").and_then(JsonValue::as_array).map(Vec::len), Some(2));
    assert_eq!(doc.get("key").and_then(JsonValue::as_str), Some("am"));
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(128.0));
    let secs = doc.get("sections").and_then(JsonValue::as_array).unwrap();
    assert_eq!(secs.iter().map(|s| (s.get("name").and_then(JsonValue::as_str).unwrap(), s.get("bars").and_then(JsonValue::as_f64).unwrap())).collect::<Vec<_>>(), [("intro", 8.0), ("drop", 16.0)]);
    assert_eq!(doc.get("license").and_then(JsonValue::as_str), Some("https://creativecommons.org/licenses/by/4.0/"));
    assert_eq!(doc.get("licenseId").and_then(JsonValue::as_str), Some("CC-BY-4.0"));
    assert_eq!(doc.get("forkedFrom").and_then(JsonValue::as_str), Some("https://beats.bitwrap.io/p#z4EBG9parentparentparent"));
    assert_eq!(doc.get("credit").and_then(JsonValue::as_array).map(Vec::len), Some(1));
    // the Euclidean terms are of the ontology's class, with their shape, and linked to the named rhythms of that shape
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    assert_eq!(eu.len(), 3, "e(3,8) twice is one shape");
    for e in eu {
        assert_eq!(e.get("@type").and_then(JsonValue::as_str), Some("bw:EuclideanRhythm"));
    }
    let kick = eu.iter().find(|e| e.get("bw:steps").and_then(JsonValue::as_f64) == Some(16.0) && e.get("bw:pulses").and_then(JsonValue::as_f64) == Some(4.0)).unwrap();
    // the ontology's own kicks of that shape (F6's table, the published individuals): six genres share E(4,16), so it has no one name
    assert_eq!(
        kick.get("sameShapeAs").unwrap().strings().unwrap(),
        [
            "https://ontology.bitwrap.io/rhythm/country-kick",
            "https://ontology.bitwrap.io/rhythm/edm-kick",
            "https://ontology.bitwrap.io/rhythm/house-kick",
            "https://ontology.bitwrap.io/rhythm/synthwave-kick",
            "https://ontology.bitwrap.io/rhythm/techno-kick",
            "https://ontology.bitwrap.io/rhythm/trance-kick"
        ]
    );
    assert!(kick.get("name").is_none());
    // E(3,8) is one rhythm of the ontology, "house snare": it names the shape (whatever role plays it)
    let tres = eu.iter().find(|e| e.get("bw:steps").and_then(JsonValue::as_f64) == Some(8.0)).unwrap();
    assert_eq!(tres.get("uses").and_then(JsonValue::as_f64), Some(2.0));
    assert_eq!(tres.get("name").and_then(JsonValue::as_str), Some("house snare"));
    assert_eq!(tres.get("sameShapeAs").unwrap().strings().unwrap(), ["https://ontology.bitwrap.io/rhythm/house-snare"]);
    // with no vocabulary handed in the terms are the same: the names are F6's table, which is constant
    let bare = document(&song(), None);
    assert_eq!(bare.get("euclid").and_then(JsonValue::as_array).map(Vec::len), Some(3));
    assert_eq!(bare.get("euclid").unwrap().at(0).unwrap().get("name").and_then(JsonValue::as_str), Some("house snare"));
    // a vocabulary with a rhythm the table does not have adds it
    let mut more = fake();
    more = StaticOntology::new(more.terms().iter().cloned().chain([Term::rhythm("made-up-snare", "made up snare: E(3,8) rotation 0", 3, 8, 0)]).collect());
    let doc = document(&song(), Some(&more));
    let tres = doc.get("euclid").unwrap().at(0).unwrap();
    assert_eq!(tres.get("sameShapeAs").unwrap().strings().unwrap(), ["https://ontology.bitwrap.io/rhythm/house-snare", "https://ontology.bitwrap.io/rhythm/made-up-snare"]);
}

#[test]
fn a_vocabulary_with_another_class_for_rhythms_is_used_in_full() {
    let mut o = fake();
    o.euclid_class = Some("https://example.org/ns#Pulse".into());
    let doc = document(&song(), Some(&o));
    assert_eq!(doc.get("euclid").unwrap().at(0).unwrap().get("@type").and_then(JsonValue::as_str), Some("https://example.org/ns#Pulse"));
}

#[test]
fn the_document_is_deterministic_has_no_timestamps_and_sorted_keys() {
    let o = fake();
    let a = linked::pretty(&document(&song(), Some(&o)));
    let b = linked::pretty(&document(&song(), Some(&o)));
    assert_eq!(a, b);
    // what it says about the program, and nothing about when or where it was written
    let doc = document(&song(), Some(&o));
    let keys: Vec<&str> = doc.as_object().unwrap().keys().collect();
    let mut want = ["@context", "@id", "@type", "cid", "name", "genre", "tag", "key", "tempo", "sections", "licenseId", "license", "visibility", "author", "forkedFrom", "credit", "euclid"];
    want.sort();
    let mut got = keys.clone();
    got.sort();
    assert_eq!(got, want, "{keys:?}");
    // `pretty` sorts the keys of every object, whatever order the document was built in
    let lines: Vec<&str> = a.lines().collect();
    assert_eq!(lines[0], "{");
    assert_eq!(lines[1], format!("  \"@context\": \"{}\",", linked::CONTEXT_URL), "{}", lines[1]);
    let top: Vec<&str> = lines.iter().filter(|l| l.starts_with("  \"")).map(|l| l.trim_start().split('"').nth(1).unwrap()).collect();
    let mut sorted_top = top.clone();
    sorted_top.sort_by(|a, b| djengine::json::utf16_cmp(a, b));
    assert_eq!(top, sorted_top);
    // and reads back as the same value
    assert_eq!(JsonValue::parse(&a).unwrap().canonical_js(), document(&song(), Some(&o)).canonical_js());
}

#[test]
fn a_program_with_no_sections_has_none_and_one_that_was_never_saved_claims_no_rights() {
    let comp = check("tempo 100\ndeck A: kick e(5,8)\n").compiled.unwrap();
    let doc = linked::jsonld(&comp, &Facts::default(), None);
    assert!(doc.get("sections").is_none());
    assert!(doc.get("license").is_none() && doc.get("licenseId").is_none() && doc.get("author").is_none() && doc.get("genre").is_none() && doc.get("tag").is_none() && doc.get("name").is_none());
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(100.0));
    assert!(doc.get("key").is_none());
}

#[test]
fn an_evolving_section_is_one_section_with_all_its_bars() {
    let src = "section drop 32 bars {\n  deck A: kick x ~ x ~ | hihat x*8\n  evolve every 8 bars {\n    hihat: thin 0.1 seed=3\n    stop when gen == 3\n  }\n}\narrange drop\n";
    let comp = check(src).compiled.unwrap();
    let doc = linked::jsonld(&comp, &Facts::default(), None);
    let secs = doc.get("sections").and_then(JsonValue::as_array).unwrap();
    assert_eq!(secs.len(), 1, "{}", linked::pretty(&doc));
    assert_eq!(secs[0].get("bars").and_then(JsonValue::as_f64), Some(32.0));
}

#[test]
fn a_semantic_edit_changes_the_id_and_formatting_does_not() {
    let o = fake();
    let base = linked::pretty(&document(&song(), Some(&o)));
    let mut rng = Rng(0x00f1_4a11);
    for round in 0..30 {
        let text = reformat(&song(), &mut rng);
        assert_eq!(linked::pretty(&document(&text, Some(&o))), base, "round {round}:\n{text}");
    }
    let edited = song().replace("e(4,16)", "e(5,16)");
    let d = document(&edited, Some(&o));
    assert_ne!(linked::pretty(&d), base);
    assert_ne!(d.get("@id"), document(&song(), Some(&o)).get("@id"));
}

// ---- the real vocabulary (the snapshot djhost caches), read off disk

fn real() -> StaticOntology {
    StaticOntology::from_json(include_str!("../../djhost/data/bitwrap-ontology.json")).expect("the cached snapshot reads")
}

#[test]
fn the_cached_snapshot_is_what_the_published_ontology_said() {
    let o = real();
    assert!(o.verified, "the snapshot says it was checked against the release's checksums");
    assert_eq!(o.terms().iter().filter(|t| t.kind == "genre").count(), 19);
    assert_eq!(o.euclid_class(), Some(linked::EUCLID_CLASS));
    assert!(o.terms().iter().all(|t| t.iri.starts_with(linked::BW_BASE) && t.iri.ends_with(&t.local)));
    for k in linked::KIND_ORDER {
        assert!(o.terms().iter().any(|t| t.kind == k), "no {k}");
    }
    let t = linked::resolve_genre(&o, "techno").unwrap();
    assert_eq!(t.iri, "https://ontology.bitwrap.io/genre/techno");
}

#[test]
fn every_genre_of_the_snapshot_is_found_by_every_spelling() {
    let o = real();
    for g in o.terms().iter().filter(|t| t.kind == "genre") {
        for spelling in [g.iri.clone(), g.local.clone(), g.label.to_uppercase(), format!("genre:{}", g.local), format!("bw:{}", g.local)] {
            assert_eq!(linked::resolve_genre(&o, &spelling).unwrap().iri, g.iri, "{spelling}");
        }
    }
}

#[test]
fn the_example_validates_against_the_real_vocabulary_and_typos_get_the_real_suggestions() {
    let o = real();
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw/linked.btw")).unwrap();
    let c = check_with(&src, &with(&o));
    assert!(c.ok());
    assert!(about_the_header(&c, &src).is_empty(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    assert!(c.diagnostics.iter().all(|d| d.message.contains("the ontology's")), "only F6's notes on the rhythms:\n{}", djlive::diag::render_all(&c.diagnostics, &src));
    let (links, _) = linked::check_headers(&src, &o, Severity::Error);
    assert_eq!(links.genre.as_deref(), Some("https://ontology.bitwrap.io/genre/techno"));
    assert_eq!(
        links.tags,
        [
            "https://ontology.bitwrap.io/groove/four-on-floor",
            "https://ontology.bitwrap.io/instrument/acid",
            "https://ontology.bitwrap.io/instrument/supersaw",
            "https://ontology.bitwrap.io/section/drop"
        ]
    );
    let e = linked::resolve_genre(&o, "tehcno").unwrap_err();
    assert_eq!(e.help.as_deref(), Some("did you mean `techno`?"));
    let e = linked::resolve_tag(&o, "supersawe").unwrap_err();
    assert_eq!(e.help.as_deref(), Some("did you mean `supersaw`?"));
    // the example's rhythms are the shapes of techno's own
    let comp = check(&src).compiled.unwrap();
    let doc = linked::jsonld(&comp, &Facts { links: Some(&links), ..Facts::default() }, Some(&o));
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    assert_eq!(eu.len(), 3);
    let same = |steps: f64, pulses: f64| -> Vec<String> { eu.iter().find(|e| e.get("bw:steps").and_then(JsonValue::as_f64) == Some(steps) && e.get("bw:pulses").and_then(JsonValue::as_f64) == Some(pulses)).and_then(|e| e.get("sameShapeAs")).and_then(JsonValue::strings).unwrap_or_default() };
    assert!(same(16.0, 4.0).contains(&"https://ontology.bitwrap.io/rhythm/techno-kick".to_string()));
    assert!(same(8.0, 5.0).contains(&"https://ontology.bitwrap.io/rhythm/techno-hihat".to_string()));
    assert!(same(16.0, 2.0).contains(&"https://ontology.bitwrap.io/rhythm/techno-snare".to_string()), "e(2,16,4) is techno's snare");
}

#[test]
fn a_tag_of_every_kind_of_the_snapshot_resolves_by_its_kind_prefix() {
    let o = real();
    for t in o.terms() {
        let r = linked::resolve_tag(&o, &format!("{}:{}", t.kind, t.local)).unwrap();
        assert_eq!(r.iri, t.iri, "{}", t.iri);
        assert_eq!(linked::resolve_tag(&o, &t.iri).unwrap().iri, t.iri);
    }
}

// ---- the document of a program that uses what the other extensions added

/// The sections of a document as (name, start, bars, key).
fn listed(doc: &JsonValue) -> Vec<(String, u32, u32, Option<String>)> {
    let secs = doc.get("sections").and_then(JsonValue::as_array).map(Vec::as_slice).unwrap_or_default();
    secs.iter()
        .map(|s| {
            let n = |k: &str| s.get(k).and_then(JsonValue::as_f64).unwrap() as u32;
            (s.get("name").and_then(JsonValue::as_str).unwrap().to_string(), n("start"), n("bars"), s.get("key").and_then(JsonValue::as_str).map(str::to_string))
        })
        .collect()
}

fn doc_of(src: &str) -> JsonValue {
    let c = check(src);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&c.diagnostics, src)));
    linked::jsonld(&comp, &Facts::default(), None)
}

/// A stack, a key per section, arithmetic in the tempo and in a def, a part with a cycle of its own, a chance term and
/// Euclidean rhythms, all in one song.
const MIXED: &str = "tempo 100 + 20\nkey am\n\ndef pat(k) {\n  deck B: snare e($k, 8)\n}\n\nsection groove 8 bars {\n  deck A: kick e(3,8) | hihat/12 e(5,12)?0.5\n  pat(3)\n}\n\nsection wash 4 bars {\n  key cm\n  deck B: harmony i VI III VII | bass/12 e(5,12) oct=2\n}\n\nsection riff 2 bars {\n  deck B: melody <a4 c5 e5 g5>\n}\n\narrange stack(groove, wash) then(riff, riff)\n";

#[test]
fn a_stack_lists_its_sections_not_its_stretches_each_with_its_own_key() {
    let doc = doc_of(MIXED);
    // the compiled song is cut where the stack's parts end (`groove+wash`, then `groove`): the document is not
    let comp = check(MIXED).compiled.unwrap();
    assert!(comp.segments.iter().any(|s| s.name == "groove+wash"), "{:?}", comp.segments.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert_eq!(
        listed(&doc),
        [("groove".to_string(), 0, 8, None), ("wash".to_string(), 0, 4, Some("cm".to_string())), ("riff".to_string(), 8, 2, None), ("riff".to_string(), 10, 2, None)],
        "groove is one section of eight bars though a stack cuts it in two; wash sets cm, the others set none; riff plays twice"
    );
    assert!(!linked::pretty(&doc).contains("groove+wash"));
    // the song's own key and the tempo it worked out (the canonical text says the number it comes to: `120`)
    assert_eq!(doc.get("key").and_then(JsonValue::as_str), Some("am"));
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(120.0));
    assert!(comp.text.contains("tempo 120\n"));
}

#[test]
fn the_euclid_terms_of_a_stack_with_a_def_a_cycle_and_a_chance_term_are_all_there() {
    let doc = doc_of(MIXED);
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    let row = |e: &JsonValue| (e.get("bw:pulses").and_then(JsonValue::as_f64).unwrap() as u32, e.get("bw:steps").and_then(JsonValue::as_f64).unwrap() as u32, e.get("uses").and_then(JsonValue::as_f64).unwrap() as u32, e.get("name").and_then(JsonValue::as_str).map(str::to_string));
    // e(3,8) is the kick and `pat(3)`'s snare (the def says `$k`); e(5,12) is the hihat's chance term and the bass's, both with a cycle of 12
    assert_eq!(eu.iter().map(row).collect::<Vec<_>>(), [(3, 8, 2, Some("house snare".to_string())), (5, 12, 2, Some("jazz hihat".to_string()))]);
    // the groove carried into the stretch after `wash` ends says nothing again
    assert_eq!(eu.len(), 2);
}

#[test]
fn the_mixed_song_is_one_document_whatever_its_formatting() {
    let base = linked::pretty(&doc_of(MIXED));
    let mut rng = Rng(0x00f1_4a12);
    for round in 0..30 {
        let text = reformat(MIXED, &mut rng);
        assert_eq!(linked::pretty(&doc_of(&text)), base, "round {round}:\n{text}");
    }
    // the canonical text is the same program, so the same document
    let canon = check(MIXED).compiled.unwrap().text;
    assert_eq!(linked::pretty(&doc_of(&canon)), base);
    // a semantic edit (a key, a section's length) shows
    assert_ne!(linked::pretty(&doc_of(&MIXED.replace("key cm", "key dm"))), base);
    assert_ne!(linked::pretty(&doc_of(&MIXED.replace("section wash 4 bars", "section wash 6 bars"))), base);
}

fn example(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw").join(name)).unwrap()
}

#[test]
fn the_examples_of_the_other_extensions_have_documents_that_match_their_arrangements() {
    // F10: intro, the verse (groove and wash stacked) with the riff twice over it, the coda
    let doc = doc_of(&example("algebra.btw"));
    assert_eq!(
        listed(&doc),
        [("intro".to_string(), 0, 4, None), ("riff".to_string(), 4, 4, None), ("groove".to_string(), 4, 8, None), ("wash".to_string(), 4, 8, None), ("riff".to_string(), 8, 4, None), ("coda".to_string(), 12, 4, None)],
        "by start bar; the three that start on bar 4 in the order of the normal form (the riffs' `then` sorts before the verse)"
    );
    assert_eq!(doc.get("key").and_then(JsonValue::as_str), Some("am"));
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(118.0));
    // F5: the second section sets its own key; a key `cm` is in the section, not the program
    let doc = doc_of(&example("scale.btw"));
    assert_eq!(listed(&doc), [("verse".to_string(), 0, 8, None), ("lift".to_string(), 8, 8, Some("cm".to_string())), ("verse".to_string(), 16, 8, None)]);
    assert_eq!(doc.get("key").and_then(JsonValue::as_str), Some("am"));
    // F2: arithmetic: `tempo 120 + 4 * 2` is 128, a section of `total` bars is 16, `e(2 + 2, 16)` is E(4,16)
    let doc = doc_of(&example("arith.btw"));
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(128.0));
    assert_eq!(listed(&doc), [("groove".to_string(), 0, 16, None)]);
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    assert_eq!(eu.len(), 1);
    assert_eq!((eu[0].get("bw:pulses").and_then(JsonValue::as_f64), eu[0].get("bw:steps").and_then(JsonValue::as_f64)), (Some(4.0), Some(16.0)));
    // F1: timed lines inside the sections change nothing about the sections
    let doc = doc_of(&example("timing.btw"));
    assert_eq!(listed(&doc), [("intro".to_string(), 0, 4, None), ("drop".to_string(), 4, 8, None)]);
    assert_eq!(doc.get("tempo").and_then(JsonValue::as_f64), Some(128.0));
    // F6: parts with a cycle of their own: the three rhythms, E(5,12) on the bass among them
    let doc = doc_of(&example("euclid.btw"));
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    let shapes: Vec<(f64, f64, f64)> = eu.iter().map(|e| (e.get("bw:pulses").and_then(JsonValue::as_f64).unwrap(), e.get("bw:steps").and_then(JsonValue::as_f64).unwrap(), e.get("bw:rotation").and_then(JsonValue::as_f64).unwrap())).collect();
    assert_eq!(shapes, [(5.0, 12.0, 0.0), (2.0, 16.0, 4.0), (4.0, 16.0, 0.0)]);
    assert!(doc.get("sections").is_none(), "a flat program has no sections");
}

#[test]
fn every_example_has_a_document_that_reads_back_and_never_names_a_stretch() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    let mut seen = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "btw") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        // an example that needs a host's registry or snippet store says so in its header and is tested where the host is
        if linked::header_lines(&src).iter().any(|h| h.key == "skip-test") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let doc = doc_of(&src);
        let text = linked::pretty(&doc);
        assert_eq!(JsonValue::parse(&text).unwrap().canonical_js(), doc.canonical_js(), "{name}");
        assert!(listed(&doc).iter().all(|(n, ..)| !n.contains('+')), "{name}: a stretch is not a section");
        seen += 1;
    }
    assert!(seen >= 10, "{seen} examples");
}

/// The context is published (https://beatscript.app/context/v1.jsonld, the copy in beats-bitwrap-io's `internal/btwsite/ld/`),
/// so a change to its terms is a new version: this file is that version, and it must say what `context()` says.
#[test]
fn the_published_context_is_the_one_the_documents_use() {
    let published: JsonValue = JsonValue::parse(include_str!("../../../docs/btw-context-v1.jsonld")).expect("docs/btw-context-v1.jsonld is JSON");
    assert_eq!(linked::pretty(&published), linked::pretty(&linked::context_document()), "the terms changed: publish v2 (a new file and URL) and do not edit v1");
    assert!(linked::CONTEXT_URL.ends_with("/context/v1.jsonld"));
}

#[test]
fn the_inline_form_spells_out_the_terms_the_default_names_by_url() {
    let o = fake();
    let comp = check(&song()).compiled.unwrap();
    let c = linked::pretty(&linked::jsonld_inline(&comp, &linked::Facts::default(), Some(&o)));
    assert!(!linked::pretty(&linked::jsonld(&comp, &linked::Facts::default(), Some(&o))).contains("djrust:cid"));
    assert!(c.contains("\"djrust:cid\""), "{c}");
}
