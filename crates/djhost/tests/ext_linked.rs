//! F14 ontology links on the host (docs/beatscript/ext-linked.md): `#! genre:` and `#! tags:` header lines kept by the
//! snippet store through save, load, export, import and fork; validated against the cached vocabulary (an unknown
//! term is an error in save and import, a warning in check); a `<object cid>.jsonld` beside each object; and the
//! `btw jsonld` document. Everything runs on the compiled-in snapshot: nothing here touches the network.

use std::path::PathBuf;

use djengine::json::JsonValue;
use djengine::rights::{License, Rights};
use djhost::live_store::{LinkEdit, LiveStore, Object};
use djlive::linked::{self, Links};

const TECHNO: &str = "https://beatscript.app/ontology/genre/techno";
// a bare `acid` is the acid genre since ontology 0.4.0 (a word two kinds both have is the genre: djlive::linked::KIND_ORDER)
const ACID: &str = "https://beatscript.app/ontology/genre/acid";
const FLOOR: &str = "https://beatscript.app/ontology/groove/four-on-floor";
const DROP: &str = "https://beatscript.app/ontology/section/drop";

const PROGRAM: &str = "tempo 128\nkey am\ndeck A: kick e(4,16) | snare e(2,16,4)\n";

fn store(tag: &str) -> (LiveStore, PathBuf) {
    let dir = djhost::stores::temp_path(&format!("ext-linked-{tag}"));
    (LiveStore::new(&dir), dir)
}

fn headed(header: &str) -> String {
    format!("{header}{PROGRAM}")
}

/// The diagnostics about the header block. The program has `e(4,16)`, which F6's `check` says the ontology names (a note
/// about the program, always there: the table is constant), so a count of all the diagnostics would count those too.
fn about_the_header<'a>(c: &'a djlive::Check, src: &str) -> Vec<&'a djlive::diag::Diagnostic> {
    let end = linked::header_end(src);
    c.diagnostics.iter().filter(|d| d.span.end <= end || d.message.contains("line is not read")).collect()
}

fn text_of<'a>(v: &'a JsonValue, k: &str) -> Option<&'a str> {
    v.get(k).and_then(JsonValue::as_str)
}

fn files(dir: &PathBuf) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.')).collect()).unwrap_or_default();
    v.sort();
    v
}

#[test]
fn header_lines_are_saved_as_full_iris_and_come_back_on_load_and_list() {
    let (s, dir) = store("save");
    let (e, w) = s.save_with("night bus", &headed("#! genre: Techno\n#! tags: four on floor, acid, drop\n"), None).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(e.links, Links { genre: Some(TECHNO.into()), tags: vec![ACID.into(), FLOOR.into(), DROP.into()] }, "full IRIs, sorted: a set");
    assert_eq!(s.list()[0].links, e.links);
    let snip = s.load("night bus").unwrap();
    assert_eq!(snip.links, e.links);
    assert_eq!(snip.text, djlive::check(PROGRAM).compiled.unwrap().text, "the saved text is the program: the header lines are comments");
    assert_eq!(snip.cid, djlive::check(PROGRAM).compiled.unwrap().cid, "and not in its CID");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_same_links_in_any_spelling_or_order_are_the_same_object() {
    let (s, dir) = store("spelling");
    let a = s.save_with("p", &headed("#! genre: bw:Techno\n#! tags: acid, drop\n"), None).unwrap().0;
    let (s2, dir2) = store("spelling2");
    let b = s2.save_with("p", &headed(&format!("#! genre: {TECHNO}\n#! tags: section:drop\n#! tags: genre:acid, Acid\n")), None).unwrap().0;
    assert_eq!(a.links, b.links);
    assert_eq!(a.object, b.object, "the same object: the links are what is addressed, not how they were typed");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn a_btw_file_round_trips_its_links_through_export_and_import() {
    let (s, dir) = store("export");
    let a = s.save_with("night bus", &headed("#! genre: techno\n#! tags: acid, drop\n"), Some(Rights { license: License::CcBy4, author: Some("matt".into()), ..Rights::default() })).unwrap().0;
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    s.export_to("night bus", &out).unwrap();
    let file = std::fs::read_to_string(out.join("night-bus.beatscript")).unwrap();
    // the header: rights, then the links, as full IRIs, then the program
    let head: Vec<&str> = file.lines().take_while(|l| l.starts_with("#!")).collect();
    assert_eq!(head, ["#! btw 1", "#! name: night bus", "#! license: CC-BY-4.0", "#! visibility: public", "#! author: matt", &format!("#! genre: {TECHNO}") as &str, &format!("#! tags: {ACID}, {DROP}")], "{file}");
    // it is still a program as it stands, with the same CID
    assert_eq!(djlive::check(&file).compiled.unwrap().cid, a.cid);
    // and the same object on the other side
    let (s2, dir2) = store("import");
    let (b, _) = s2.import_file(&out.join("night-bus.beatscript")).unwrap();
    assert_eq!(b.links, a.links);
    assert_eq!(b.object, a.object);
    assert_eq!(s2.load_object("night bus").unwrap().links, a.links);
    // import takes any spelling too, and a file with no link lines has no links
    let (d, _) = s2.import_text(&format!("#! name: other\n#! genre: genre:house\n#! tags: bw:Acid\ndeck A: snare ~ x ~ x\n"), "o").unwrap();
    assert_eq!(d.links, Links { genre: Some("https://beatscript.app/ontology/genre/house".into()), tags: vec![ACID.into()] });
    let (e, _) = s2.import_text("#! name: bare\ndeck A: hihat x*8\n", "o").unwrap();
    assert!(e.links.is_empty());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn links_survive_fork_resave_and_a_rights_change() {
    let (s, dir) = store("keep");
    let a = s.save_with("night bus", &headed("#! genre: techno\n#! tags: acid\n"), None).unwrap().0;
    // a fork carries them
    let (f, _) = s.fork("night bus", Some("night bus 2"), Some("did:key:z6MkFork")).unwrap();
    assert_eq!(f.links, a.links);
    assert_eq!(f.rights.forked_from.as_deref(), Some(a.cid.as_str()));
    // the same program saved again with no header lines keeps its links (as it keeps its rights)
    let (again, _) = s.save_with("the night bus", PROGRAM, None).unwrap();
    assert_eq!(again.links, a.links);
    assert_eq!(again.name, "the night bus");
    // a new license is a new object with the same links
    let b = s.set_rights("the night bus", Some(License::CcBy4), None).unwrap();
    assert_ne!(b.object, again.object);
    assert_eq!(b.links, a.links);
    // header lines in the text are the statement: they replace
    let (c, _) = s.save_with("the night bus", &headed("#! genre: house\n"), None).unwrap();
    assert_eq!(c.links, Links { genre: Some("https://beatscript.app/ontology/genre/house".into()), tags: vec![] });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_change_of_genre_or_tags_is_a_new_object_of_the_same_program() {
    let (s, dir) = store("edit");
    let a = s.save_with("night bus", &headed("#! genre: techno\n"), None).unwrap().0;
    let before = files(&dir);
    assert_eq!(before.len(), 2, "the object and its JSON-LD: {before:?}");
    // tags only: the genre is left as it is
    let b = s.set_links("night bus", &LinkEdit::new(None, Some("acid, drop"))).unwrap();
    assert_eq!(b.cid, a.cid, "the same program");
    assert_ne!(b.object, a.object, "a new object");
    assert_eq!(b.links, Links { genre: Some(TECHNO.into()), tags: vec![ACID.into(), DROP.into()] });
    assert_eq!(s.list().len(), 1, "the old one went");
    assert_eq!(files(&dir).len(), 2, "with its JSON-LD: {:?}", files(&dir));
    assert!(!dir.join(format!("{}.beatscript", a.object)).exists() && !dir.join(format!("{}.jsonld", a.object)).exists());
    // genre only, then clear the tags
    let c = s.set_links("night bus", &LinkEdit::new(Some("dnb"), None)).unwrap();
    assert_eq!(c.links.genre.as_deref(), Some("https://beatscript.app/ontology/genre/dnb"));
    assert_eq!(c.links.tags.len(), 2);
    let d = s.set_links("night bus", &LinkEdit::new(Some(""), Some(""))).unwrap();
    assert!(d.links.is_empty());
    // with no links an object is the object it was before there were links
    let plain = s.save_with("plain", "deck A: kick x\n", None).unwrap().0;
    let o = s.load_object("plain").unwrap();
    assert!(o.links.is_empty());
    assert!(!o.to_json().canonical_js().contains("genre") && !o.to_json().canonical_js().contains("tags"));
    assert_eq!(plain.object, Object { name: o.name.clone(), text: o.text.clone(), rights: o.rights.clone(), links: Links::default() }.cid());
    // an unknown term changes nothing
    let e = s.set_links("night bus", &LinkEdit::new(Some("tehcno"), None)).unwrap_err();
    assert!(e.contains("unknown genre `tehcno`") && e.contains("did you mean `techno`?"), "{e}");
    assert_eq!(s.load("night bus").unwrap().links, d.links);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_unknown_genre_or_tag_is_an_error_in_save_and_import_with_a_did_you_mean_and_a_place() {
    let (s, dir) = store("errors");
    let e = s.save_with("x", &headed("#! genre: tehcno\n"), None).unwrap_err();
    assert!(e.starts_with("btw: not saved, unknown genre `tehcno` (line 1, column 11): did you mean `techno`?"), "{e}");
    let e = s.save_with("x", &headed("#! genre: techno\n#! tags: acid, acidd, nonesuch\n"), None).unwrap_err();
    assert!(e.contains("unknown tag `acidd` (line 2, column 16): did you mean `acid`") && e.contains("unknown tag `nonesuch`"), "{e}");
    let e = s.save_with("x", &headed("#! genre: groove:four-on-floor\n"), None).unwrap_err();
    assert!(e.contains("is a groove, not a genre"), "{e}");
    assert!(s.list().is_empty(), "nothing was kept");
    let e = s.import_text(&format!("#! name: x\n#! tags: sub-bas\n{PROGRAM}"), "x").unwrap_err();
    assert!(e.starts_with("btw: not imported, unknown tag `sub-bas` (line 2, column 10): did you mean `sub-bass`?"), "{e}");
    assert!(s.list().is_empty());
    assert!(files(&dir).is_empty() || files(&dir).iter().all(|f| f.starts_with('.')), "{:?}", files(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn check_says_it_as_a_warning_the_program_still_runs_and_its_cid_is_the_same() {
    let (s, dir) = store("check");
    let src = headed("#! genre: tehcno\n#! tags: acid, drumz\n");
    let c = s.check(&src);
    assert!(c.ok(), "a header line is a comment");
    let ds: Vec<_> = about_the_header(&c, &src).into_iter().map(|d| (d.severity, d.position(&src), d.message.clone(), d.help.clone())).collect();
    assert_eq!(ds.len(), 2, "{ds:?}");
    assert_eq!((ds[0].0, ds[0].1 .0, ds[0].1 .1, ds[0].2.as_str(), ds[0].3.as_deref()), (djlive::Severity::Warning, 1, 11, "unknown genre `tehcno`", Some("did you mean `techno`?")));
    assert_eq!((ds[1].1 .0, ds[1].1 .1), (2, 16));
    assert!(ds[1].3.as_deref().is_some_and(|h| h.contains("drums")), "{ds:?}");
    assert_eq!(c.compiled.unwrap().cid, djlive::check(PROGRAM).compiled.unwrap().cid);
    // clean headers: nothing to say
    let clean = headed("#! genre: techno\n#! tags: acid, drop\n");
    let c = s.check(&clean);
    assert!(about_the_header(&c, &clean).is_empty(), "{:?}", c.diagnostics);
    assert!(c.diagnostics.iter().all(|d| d.message.contains("the ontology's")), "only F6's notes: {:?}", c.diagnostics);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_jsonld_is_written_beside_each_object_and_follows_it() {
    let (s, dir) = store("sidecar");
    let a = s.save_with("night bus", &headed("#! genre: techno\n#! tags: acid\n"), Some(Rights { author: Some("did:key:z6MkTest".into()), ..Rights::default() })).unwrap().0;
    assert_eq!(files(&dir), [format!("{}.beatscript", a.object), format!("{}.jsonld", a.object)]);
    let on_disk = std::fs::read_to_string(dir.join(format!("{}.jsonld", a.object))).unwrap();
    assert!(on_disk.ends_with("}\n"));
    let doc = s.jsonld("night bus").unwrap();
    assert_eq!(on_disk, format!("{}\n", linked::pretty(&doc)), "the file is the verb's answer");
    assert_eq!(text_of(&doc, "@type"), Some("Program"));
    assert_eq!(text_of(&doc, "@id"), Some(format!("https://beatscript.app/p/{}", a.cid).as_str()));
    assert_eq!(text_of(&doc, "genre"), Some(TECHNO));
    assert_eq!(doc.get("tag").unwrap().strings().unwrap(), [ACID]);
    assert_eq!((text_of(&doc, "key"), doc.get("tempo").and_then(JsonValue::as_f64)), (Some("am"), Some(128.0)));
    assert_eq!(text_of(&doc, "name"), Some("night bus"));
    assert_eq!(text_of(&doc, "licenseId"), Some("CC0-1.0"));
    assert_eq!(text_of(&doc, "license"), Some("https://creativecommons.org/publicdomain/zero/1.0/"));
    assert_eq!(text_of(&doc, "author"), Some("did:key:z6MkTest"));
    assert_eq!(text_of(&doc, "visibility"), Some("public"));
    // the Euclidean terms are linked to the ontology's class and to the named rhythms of the same shape
    let eu = doc.get("euclid").and_then(JsonValue::as_array).unwrap();
    assert_eq!(eu.len(), 2);
    assert_eq!(text_of(&eu[0], "@type"), Some("bw:EuclideanRhythm"));
    let kick = eu.iter().find(|e| e.get("bw:pulses").and_then(JsonValue::as_f64) == Some(4.0)).unwrap();
    assert!(kick.get("sameShapeAs").unwrap().strings().unwrap().contains(&"https://beatscript.app/ontology/rhythm/techno-kick".to_string()));
    // a new license: a new object, and its JSON-LD with it; the old pair goes
    let b = s.set_rights("night bus", Some(License::CcBy4), None).unwrap();
    assert_eq!(files(&dir), [format!("{}.beatscript", b.object), format!("{}.jsonld", b.object)]);
    assert!(std::fs::read_to_string(dir.join(format!("{}.jsonld", b.object))).unwrap().contains("\"licenseId\": \"CC-BY-4.0\""));
    // a fork has its own, naming its parent
    let (f, _) = s.fork("night bus", Some("fork"), Some("did:key:z6MkFork")).unwrap();
    let fdoc = std::fs::read_to_string(dir.join(format!("{}.jsonld", f.object))).unwrap();
    assert!(fdoc.contains(&format!("\"forkedFrom\": \"{}\"", a.cid)) && fdoc.contains(&format!("\"https://beatscript.app/p/{}\"", a.cid)), "{fdoc}");
    assert!(fdoc.contains("\"credit\": ["), "a CC BY parent is credited: {fdoc}");
    // a .jsonld is not a program: the list has two, and a delete takes both files
    assert_eq!(s.list().len(), 2);
    s.delete(&f.object).unwrap();
    assert_eq!(files(&dir).len(), 2);
    // the same facts are the same bytes: writing it again changes nothing
    let again = std::fs::read_to_string(dir.join(format!("{}.jsonld", b.object))).unwrap();
    s.set_links("night bus", &LinkEdit::new(Some("techno"), None)).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(format!("{}.jsonld", b.object))).unwrap(), again);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn jsonld_of_a_text_takes_its_links_and_its_rights_from_its_header_only() {
    let (s, dir) = store("text");
    // a bare program: no links, no claim about rights
    let doc = s.jsonld_text(PROGRAM).unwrap();
    assert!(doc.get("genre").is_none() && doc.get("licenseId").is_none() && doc.get("name").is_none());
    assert_eq!(text_of(&doc, "@id"), Some(format!("https://beatscript.app/p/{}", djlive::check(PROGRAM).compiled.unwrap().cid).as_str()));
    // a .beatscript file: its name, links and rights
    let file = headed("#! btw 1\n#! name: night bus\n#! license: CC-BY-4.0\n#! author: matt\n#! genre: genre:techno\n#! tags: acid, four on floor\n");
    let doc = s.jsonld_text(&file).unwrap();
    assert_eq!((text_of(&doc, "name"), text_of(&doc, "genre"), text_of(&doc, "licenseId"), text_of(&doc, "author")), (Some("night bus"), Some(TECHNO), Some("CC-BY-4.0"), Some("matt")));
    assert_eq!(doc.get("tag").unwrap().strings().unwrap(), [ACID, FLOOR]);
    // the same file saved and asked for by id is the same document
    s.import_text(&file, "x").unwrap();
    assert_eq!(linked::pretty(&s.jsonld("night bus").unwrap()), linked::pretty(&doc));
    // an unknown term, a program with errors and a missing id are errors that say so
    assert!(s.jsonld_text(&headed("#! genre: tehcno\n")).unwrap_err().contains("did you mean `techno`?"));
    assert!(s.jsonld_text("deck A: kick q\n").unwrap_err().contains("1 error"));
    assert!(s.jsonld("zzz-nothing").is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_skip_test_line_among_the_header_lines_confuses_neither_the_rights_nor_the_links() {
    // examples/beatscript files that need a host say `#! skip-test: <why>` in their header; so can a file with rights and links. The
    // reason may itself say `genre:` or a comma; the line is not kept by anything, and no line number moves for it
    let skip = "#! skip-test: imports the saved snippet \"harmony\", genre: not this one, tags: nor these\n";
    let file = format!("{skip}#! btw 1\n#! name: night bus\n#! license: CC-BY-4.0\n#! visibility: public\n#! author: matt\n#! genre: techno\n#! tags: acid, drop\n{PROGRAM}");
    let (s, dir) = store("skip");
    let (e, w) = s.import_text(&file, "fallback").unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!((e.name.as_str(), e.rights.license, e.rights.author.as_deref()), ("night bus", License::CcBy4, Some("matt")));
    assert_eq!(e.links, Links { genre: Some(TECHNO.into()), tags: vec![ACID.into(), DROP.into()] }, "the reason's `genre:` is not the genre");
    // the same object as the file without the skip line: the line is not part of anything kept
    let (s2, dir2) = store("skip-plain");
    let plain = file.replacen(skip, "", 1);
    let (e2, _) = s2.import_text(&plain, "fallback").unwrap();
    assert_eq!(e.object, e2.object);
    assert!(!s.load("night bus").unwrap().text.contains("skip-test"));
    // after the links, and between the rights lines, it is the same
    let moved = format!("#! btw 1\n#! name: night bus\n#! license: CC-BY-4.0\n#! genre: techno\n#! skip-test: why\n#! visibility: public\n#! author: matt\n#! tags: acid, drop\n{PROGRAM}");
    let (s3, dir3) = store("skip-moved");
    assert_eq!(s3.import_text(&moved, "x").unwrap().0.object, e.object);
    // the document and `check` through the host: the same, and clean
    let doc = s.jsonld_text(&file).unwrap();
    assert_eq!((text_of(&doc, "genre"), text_of(&doc, "licenseId"), text_of(&doc, "name")), (Some(TECHNO), Some("CC-BY-4.0"), Some("night bus")));
    assert_eq!(doc.get("tag").unwrap().strings().unwrap(), [ACID, DROP]);
    let c = s.check(&file);
    assert!(c.ok());
    assert!(about_the_header(&c, &file).is_empty(), "{:?}", c.diagnostics);
    // a mistake in a link is said at its own line, which counts the skip line
    let bad = format!("{skip}#! name: x\n#! tags: acid, acidd\n{PROGRAM}");
    let msg = s.import_text(&bad, "x").unwrap_err();
    assert!(msg.contains("unknown tag `acidd` (line 3, column 16): did you mean `acid`"), "{msg}");
    let c = s.check(&bad);
    let at: Vec<usize> = about_the_header(&c, &bad).iter().map(|d| d.position(&bad).0).collect();
    assert_eq!(at, [3]);
    // a skip line after the program has begun is a comment like any `#!` line there, and a genre line under it says it is not read
    let late = format!("{PROGRAM}#! skip-test: no\n#! genre: techno\n");
    let c = s.check(&late);
    assert_eq!(about_the_header(&c, &late).len(), 1, "{:?}", c.diagnostics);
    assert!(about_the_header(&c, &late)[0].message.contains("line is not read"));
    for d in [dir, dir2, dir3] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_file_edited_by_hand_is_not_listed_and_one_with_a_withdrawn_term_is_still_read_as_it_is() {
    let (s, dir) = store("edited");
    let a = s.save_with("night bus", &headed("#! genre: techno\n"), None).unwrap().0;
    let path = dir.join(format!("{}.beatscript", a.object));
    // changing the genre by hand makes the object something else: its name is no longer its content's CID
    let edited = std::fs::read_to_string(&path).unwrap().replace("genre/techno", "genre/house");
    std::fs::write(&path, edited).unwrap();
    assert!(s.list().is_empty(), "an edit shows");
    let _ = std::fs::remove_dir_all(&dir);
    // a term the vocabulary no longer has (it was refreshed) does not lose the program: the store reads what it kept
    let comp = djlive::check(PROGRAM).compiled.unwrap();
    let o = Object { name: "old".into(), text: comp.text.clone(), rights: Rights::default(), links: Links::new(Some("https://beatscript.app/ontology/genre/withdrawn".into()), vec![]) };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.beatscript", o.cid())), o.to_file()).unwrap();
    let l = s.list();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].links.genre.as_deref(), Some("https://beatscript.app/ontology/genre/withdrawn"));
    // but saving it again names the problem, and a re-save of its text without header lines keeps what it has
    assert_eq!(s.save_with("old", PROGRAM, None).unwrap().0.links, l[0].links);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_vocabulary_is_in_the_binary_and_says_where_it_is_from() {
    let d = djhost::ontology::describe();
    assert!(d.starts_with("BeatScript ontology 0.4.0, 277 terms, taken 20"), "{d}");
    // BeatScript's own environment has it, so any check through the host does
    let be = djhost::beatscript_env::BeatscriptEnv::new(None);
    assert!(be.env().ontology.is_some());
    let src = headed("#! tags: nope\n");
    let c = djlive::compile::check_with(&src, &be.env());
    assert_eq!(about_the_header(&c, &src).len(), 1);
}

// ---- the law: the same links are the same object, in any spelling and order

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// One of the many ways to write a term of the vocabulary.
fn spelled(t: &djlive::linked::Term, rng: &mut Rng) -> String {
    match rng.below(5) {
        0 => t.iri.clone(),
        1 => format!("{}:{}", t.kind, t.local),
        2 => t.local.to_uppercase(),
        3 => t.label.replace('-', " "),
        _ => t.local.clone(),
    }
}

#[test]
fn random_spellings_and_orders_of_the_same_links_are_the_same_object_file_and_document() {
    use djlive::linked::Ontology;
    let onto = djhost::ontology::vocabulary();
    // the terms whose spelling is not ambiguous by name: a random spelling of any of them must find it
    let usable: Vec<&djlive::linked::Term> = onto.terms().iter().filter(|t| t.kind != "rhythm" && t.kind != "structure" && t.kind != "progression").collect();
    let genres: Vec<&djlive::linked::Term> = onto.terms().iter().filter(|t| t.kind == "genre").collect();
    let mut rng = Rng(0x00f1_4e55_0000_0001);
    let mut checked = 0;
    for round in 0..40 {
        let g = genres[rng.below(genres.len())];
        let tags: Vec<&djlive::linked::Term> = (0..rng.below(6)).map(|_| usable[rng.below(usable.len())]).collect();
        // the terms the names resolve to (a bare name two kinds share goes to the first kind: ask the resolver, not the table)
        let write = |rng: &mut Rng| -> String {
            let mut order: Vec<usize> = (0..tags.len()).collect();
            for i in (1..order.len()).rev() {
                order.swap(i, rng.below(i + 1));
            }
            let names: Vec<String> = order.iter().map(|&i| spelled(tags[i], rng)).collect();
            let mut h = format!("#! genre: {}\n", spelled(g, rng));
            if !names.is_empty() {
                h.push_str(&format!("#! tags: {}\n", names.join(", ")));
            }
            h
        };
        let (a, da) = store(&format!("law-a{round}"));
        let (b, db) = store(&format!("law-b{round}"));
        let ha = write(&mut rng);
        let hb = write(&mut rng);
        let (Ok((ea, _)), Ok((eb, _))) = (a.save_with("p", &headed(&ha), None), b.save_with("p", &headed(&hb), None)) else {
            // a spelling of one kind that reads as another (`bass` the role written as its label, `edm` the family) is
            // a different tag, not a failure of the law: only the same *resolved* set is compared below
            let _ = (std::fs::remove_dir_all(&da), std::fs::remove_dir_all(&db));
            continue;
        };
        let want = {
            let mut ts: Vec<String> = linked::canonical_links(onto, None, &ha.lines().find(|l| l.starts_with("#! tags:")).map(|l| linked::split_tags(&l[8..])).unwrap_or_default()).unwrap().tags;
            ts.sort();
            ts
        };
        assert_eq!(ea.links.tags, want, "round {round}: {ha}");
        assert!(ea.links.tags.windows(2).all(|w| w[0] < w[1]), "a set: sorted, each once");
        // a different spelling of the same resolved links is the same object, the same file, the same document
        if ea.links == eb.links {
            assert_eq!(ea.object, eb.object, "round {round}");
            assert_eq!(std::fs::read(da.join(format!("{}.beatscript", ea.object))).unwrap(), std::fs::read(db.join(format!("{}.beatscript", eb.object))).unwrap());
            assert_eq!(std::fs::read(da.join(format!("{}.jsonld", ea.object))).unwrap(), std::fs::read(db.join(format!("{}.jsonld", eb.object))).unwrap());
            checked += 1;
        }
        // export and import is the same object
        let out = da.join("out");
        std::fs::create_dir_all(&out).unwrap();
        a.export_to("p", &out).unwrap();
        let (c, dc) = store(&format!("law-c{round}"));
        let (ec, _) = c.import_file(&out.join("p.beatscript")).unwrap();
        assert_eq!(ec.object, ea.object, "round {round}");
        assert_eq!(ec.links, ea.links);
        for d in [da, db, dc] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    assert!(checked >= 5, "only {checked} rounds compared two spellings (the tags drawn were too ambiguous)");
}

// ---- the registry: `btw jsonld`, `btw terms` and `btw save {genre, tags}` are the control language's

#[test]
fn the_registry_has_the_verbs_and_a_program_can_call_them_through_do() {
    use djhost::control_registry::{noun, Effect};
    let n = noun("btw").unwrap();
    let jsonld = n.action("jsonld").expect("btw jsonld");
    assert_eq!(jsonld.effect, Effect::Read);
    assert!(jsonld.params.iter().any(|p| p.name == "id") && jsonld.params.iter().any(|p| p.name == "text"));
    let terms = n.action("terms").expect("btw terms");
    assert_eq!(terms.effect, Effect::Read);
    assert!(terms.params.iter().any(|p| p.name == "kind"));
    let save = n.action("save").unwrap();
    assert!(save.params.iter().any(|p| p.name == "genre") && save.params.iter().any(|p| p.name == "tags"));
    // the probes the round trip sends are JSON, and name something that is not there (so a probe refuses before it acts)
    assert!(JsonValue::parse(jsonld.probe).is_ok() && JsonValue::parse(terms.probe).is_ok());
    // the whole of it is documented where the tools are described
    assert!(n.doc.contains("jsonld {id | text}") && n.doc.contains("terms {kind}"));
    // and a program can call them: checked against the registry like any other line
    let be = djhost::beatscript_env::BeatscriptEnv::new(None);
    for line in ["do btw jsonld id=\"night bus\"\n", "do btw terms kind=genre\n", "do btw save name=\"x\" genre=techno tags=\"acid, drop\"\n"] {
        let c = djlive::compile::check_with(line, &be.env());
        assert!(c.ok(), "{line}{}", djlive::diag::render_all(&c.diagnostics, line));
    }
}
