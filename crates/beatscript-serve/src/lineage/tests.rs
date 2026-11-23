use super::*;

const ROOT: &str = "tempo 124\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";
const FAST: &str = "tempo 126\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";
const UP: &str = "tempo 124\nkey am\ndeck A: kick x ~ x ~ | bass b1 ~ d2 ~\n";
const FAST2: &str = "tempo 128\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n";
const SECRET: &str = "tempo 100\nkey c\ndeck A: kick x ~ ~ ~\n";
const UNDER_SECRET: &str = "tempo 101\nkey c\ndeck A: kick x ~ ~ ~\n";

fn canon(text: &str) -> (String, String) {
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
    assert_eq!(rep["ok"], true, "{text}: {rep}");
    (rep["cid"].as_str().unwrap().to_string(), rep["canonical"].as_str().unwrap().to_string())
}

fn put(dir: &Path, text: &str, meta: Value) -> String {
    let (cid, canonical) = canon(text);
    std::fs::write(dir.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": canonical, "meta": meta})).unwrap()).unwrap();
    cid
}

struct Fam {
    _d: tempfile::TempDir,
    l: Lineage,
    root: String,
    fast: String,
    up: String,
    fast2: String,
    secret: String,
    under: String,
}

fn family() -> Fam {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let root = put(p, ROOT, json!({"license": "CC0-1.0", "visibility": "public", "name": "night bus"}));
    let fast = put(p, FAST, json!({"license": "CC-BY-4.0", "visibility": "public", "name": "night bus (faster)", "forked_from": root, "credits": ["ann"]}));
    let up = put(p, UP, json!({"license": "CC0-1.0", "visibility": "public", "forked_from": root}));
    let fast2 = put(p, FAST2, json!({"license": "CC0-1.0", "visibility": "public", "forked_from": fast}));
    let secret = put(p, SECRET, json!({"license": "CC0-1.0", "visibility": "private", "forked_from": root}));
    let under = put(p, UNDER_SECRET, json!({"license": "CC0-1.0", "visibility": "public", "forked_from": secret}));
    let l = Lineage::new(p, Vec::new());
    Fam { _d: d, l, root, fast, up, fast2, secret, under }
}

#[test]
fn the_tree_has_ancestors_siblings_and_forks_with_musical_diffs() {
    let f = family();
    let t = f.l.tree(&f.fast).expect("a tree");
    assert_eq!(t.root.cid, f.fast);
    assert_eq!(t.ancestors.iter().map(|n| n.cid.as_str()).collect::<Vec<_>>(), [f.root.as_str()]);
    assert_eq!(t.siblings.iter().map(|n| n.cid.as_str()).collect::<Vec<_>>(), [f.up.as_str()]);
    assert_eq!(t.forks.iter().map(|n| n.cid.as_str()).collect::<Vec<_>>(), [f.fast2.as_str()]);
    let e = t.edge_to(&f.fast).unwrap();
    assert_eq!(e.from, f.root);
    let c = e.change.as_ref().unwrap();
    assert!(!c.same);
    assert_eq!(c.summary, ["tempo 124 -> 126"]);
    let e2 = t.edge_to(&f.fast2).unwrap().change.as_ref().unwrap();
    assert_eq!(e2.summary, ["tempo 126 -> 128"]);
    let sib = t.edge_to(&f.up).unwrap().change.as_ref().unwrap();
    assert!(sib.summary.iter().any(|l| l.contains("bass")), "{:?}", sib.summary);
    assert_eq!(t.root.fork_count, 1);
    assert_eq!(t.root.credits, ["ann"]);
}

#[test]
fn private_programs_never_appear() {
    let f = family();
    // the root's forks: the two public ones, not the private one (and not what hangs under it)
    let t = f.l.tree(&f.root).unwrap();
    let all: Vec<&str> = t.forks.iter().map(|n| n.cid.as_str()).collect();
    assert!(!all.contains(&f.secret.as_str()) && !all.contains(&f.under.as_str()), "{all:?}");
    assert_eq!(t.root.fork_count, 2);
    assert_eq!(f.l.fork_count(&f.root), 2);
    let doc = serde_json::to_string(&t.json_ld("https://beatscript.app")).unwrap();
    assert!(!doc.contains(&f.secret), "{doc}");
    // the private program has no tree, and no compare
    assert!(f.l.tree(&f.secret).is_none());
    assert!(f.l.compare(&f.root, &f.secret).is_none());
    // a public program under a private one: its chain is cut there, and the private CID is not shown
    let t = f.l.tree(&f.under).unwrap();
    assert!(t.ancestors.is_empty() && t.ancestors_cut);
    let doc = t.json_ld("https://beatscript.app");
    assert!(!serde_json::to_string(&doc["ancestors"]).unwrap().contains(&f.secret));
    assert!(!serde_json::to_string(&doc["derivations"]).unwrap().contains(&f.secret));
    let html = page::tree_page("https://beatscript.app", &t, &doc);
    assert!(html.contains("not public here"));
    assert!(!html.contains(&f.secret), "private CID in the page");
    assert!(doc["programs"][0]["forkedFrom"].is_null());
    assert!(!serde_json::to_string(&doc).unwrap().contains(&f.secret));
}

#[test]
fn a_text_that_says_it_is_not_public_or_does_not_hash_is_not_shown() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let hidden = put(p, "#! visibility: private\ntempo 90\ndeck A: kick x\n", json!({"visibility": "public"}));
    // `#!` lines are not canonical: store the declaring text itself, as an old server might have
    std::fs::write(p.join(format!("{hidden}.json")), serde_json::to_vec(&json!({"cid": hidden, "text": "#! visibility: private\ntempo 90\ndeck A: kick x\n", "meta": {"visibility": "public"}})).unwrap()).unwrap();
    let (other, _) = canon("tempo 91\ndeck A: kick x\n");
    std::fs::write(p.join(format!("{other}.json")), serde_json::to_vec(&json!({"cid": other, "text": "tempo 92\ndeck A: kick x\n", "meta": {}})).unwrap()).unwrap();
    let l = Lineage::new(p, Vec::new());
    assert!(l.tree(&hidden).is_none());
    assert!(l.tree(&other).is_none());
    assert!(l.tree("znotacid").is_none());
}

#[test]
fn an_example_can_be_an_ancestor() {
    let d = tempfile::tempdir().unwrap();
    let (ex, text) = canon(ROOT);
    let kid = put(d.path(), FAST, json!({"visibility": "public", "forked_from": ex}));
    let l = Lineage::new(d.path(), vec![Example { cid: ex.clone(), name: "night-bus".into(), license: "CC0-1.0".into(), text }]);
    let t = l.tree(&kid).unwrap();
    assert_eq!(t.ancestors.len(), 1);
    assert!(t.ancestors[0].example);
    assert_eq!(l.fork_count(&ex), 1);
    let te = l.tree(&ex).unwrap();
    assert_eq!(te.forks.len(), 1);
    assert_eq!(te.edge_to(&kid).unwrap().change.as_ref().unwrap().summary, ["tempo 124 -> 126"]);
}

#[test]
fn a_cycle_in_the_rights_ends() {
    let d = tempfile::tempdir().unwrap();
    let (a, _) = canon(ROOT);
    let (b, _) = canon(FAST);
    put(d.path(), ROOT, json!({"visibility": "public", "forked_from": b}));
    put(d.path(), FAST, json!({"visibility": "public", "forked_from": a}));
    let l = Lineage::new(d.path(), Vec::new());
    let t = l.tree(&a).unwrap();
    assert_eq!(t.ancestors.len(), 1);
    assert!(t.ancestors_cut);
    assert!(t.forks.is_empty(), "b is already the ancestor");
}

#[test]
fn compare_any_two() {
    let f = family();
    let (a, b, c) = f.l.compare(&f.up, &f.fast2).unwrap();
    assert_eq!((a.cid.as_str(), b.cid.as_str()), (f.up.as_str(), f.fast2.as_str()));
    let c = c.unwrap();
    assert!(c.summary.iter().any(|l| l == "tempo 124 -> 128"), "{:?}", c.summary);
    let doc = compare_json_ld("https://beatscript.app", &a, &b, Some(&c));
    assert_eq!(doc["@type"], "Comparison");
    assert_eq!(doc["from"]["@id"], json!(f.up));
    assert_eq!(doc["sameSong"], false);
    assert!(doc["song"].as_array().unwrap().iter().any(|s| s["field"] == "tempo"));
    let html = page::compare_page("https://beatscript.app", &a, &b, Some(&c), &doc);
    assert!(html.contains("tempo 124 -&gt; 128"));
    assert!(html.contains("<td>tempo</td><td>124</td><td>128</td>"), "{html}");
    // a program and itself
    let (_, _, same) = f.l.compare(&f.root, &f.root).unwrap();
    assert!(same.unwrap().text_same);
}

#[test]
fn the_document_names_every_node_and_edge() {
    let f = family();
    let t = f.l.tree(&f.root).unwrap();
    let d = t.json_ld("https://beatscript.app");
    assert_eq!(d["@context"], json!([SCHEMA, "https://schema.org"]));
    assert_eq!(d["@type"], "Lineage");
    assert_eq!(d["@id"], json!(format!("https://beatscript.app/p/{}/tree", f.root)));
    assert_eq!(d["forkCount"], 2);
    assert_eq!(d["descendantCount"], 3);
    assert_eq!(d["forks"].as_array().unwrap().len(), 2);
    assert_eq!(d["programs"].as_array().unwrap().len(), 4);
    assert_eq!(d["programs"][0]["name"], "night bus");
    assert_eq!(d["programs"][0]["@type"], json!(["Program", "MusicComposition"]));
    let ders = d["derivations"].as_array().unwrap();
    assert_eq!(ders.len(), 3);
    for e in ders {
        assert_eq!(e["@type"], "Derivation");
        assert!(e["change"].is_array(), "{e}");
        assert!(e["compare"].as_str().unwrap().contains("/compare/"));
    }
    let html = page::tree_page("https://beatscript.app", &t, &d);
    assert!(html.contains("<script type=\"application/ld+json\">"));
    assert!(html.contains("tempo 124 -&gt; 126"));
    assert!(html.contains("tempo 126 -&gt; 128"));
    assert!(html.contains("2 forks, 3 in all generations"), "{html}");
}

#[test]
fn a_new_file_is_found_without_waiting_for_the_rescan() {
    let f = family();
    assert_eq!(f.l.fork_count(&f.fast2), 0);
    let late = put(f._d.path(), "tempo 130\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n", json!({"visibility": "public", "forked_from": f.fast2}));
    // asking for the new program itself reads it, and rescans
    assert!(f.l.tree(&late).is_some());
    assert_eq!(f.l.fork_count(&f.fast2), 1);
}

#[test]
fn names_are_escaped_in_the_page() {
    let d = tempfile::tempdir().unwrap();
    let c = put(d.path(), ROOT, json!({"visibility": "public", "name": "<script>alert(1)</script>"}));
    let l = Lineage::new(d.path(), Vec::new());
    let t = l.tree(&c).unwrap();
    let ld = t.json_ld("https://beatscript.app");
    let html = page::tree_page("https://beatscript.app", &t, &ld);
    assert!(!html.contains("<script>alert"), "{html}");
}

#[test]
fn the_program_page_names_its_forks() {
    let html = "<html><body><main>\n<h1>x</h1>\n</main>\n</body></html>";
    let out = page::with_fork_line(html, "zABC", 2);
    assert!(out.contains("<p class=\"lineage\"><a href=\"/p/zABC/tree\">Family tree</a> · 2 forks</p>\n</main>"), "{out}");
    assert!(page::with_fork_line(html, "zABC", 0).contains("no forks yet"));
    assert_eq!(page::with_fork_line("no main", "z", 1), "no main");
}

/// The curation signal (roadmap 6670): how many public programs call or borrow a CID or a generator call, and how many
/// vendored it (a `# from:` line saved as `meta.based_on`). A private program counts for nothing.
#[test]
fn a_call_and_its_cid_count_who_draws_on_them() {
    beatscript_api::generate::install();
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let call = "gen@2 techno seed=42 section=drop part=bass";
    let calls = format!("tempo 128\nkey c\nlet bed = {call}\nsection groove 4 bars {{\n  deck A: bass bed\n}}\narrange groove\n");
    let a = put(p, &calls, json!({"visibility": "public"}));
    let b = put(p, &format!("{calls}deck B: arp c4 e4 g4 e4\n"), json!({"visibility": "public"}));
    let _hidden = put(p, &format!("{calls}deck B: arp c4 ~ g4 ~\n"), json!({"visibility": "private"}));
    let site = &djlive::refs::sites(&calls, &beatscript_api::web_env().env()).unwrap()[0];
    let pin = site.resolved.clone().unwrap();
    // one that vendored it: the notes, and the `# from:` line its save kept
    let ej = djlive::refs::eject(&calls, &beatscript_api::web_env().env(), calls.find("gen@2").unwrap()).unwrap().unwrap();
    let v = put(p, &ej, json!({"visibility": "public", "based_on": [format!("{call} -> {pin}")]}));
    let l = Lineage::new(p, Vec::new());
    l.refresh(true);
    assert_eq!(l.curation(call), Curation { referenced_by: 2, vendored_from: 1 });
    assert_eq!(l.curation(&pin), Curation { referenced_by: 2, vendored_from: 1 });
    assert_eq!(l.curation("gen@2 techno seed=43 section=drop part=bass"), Curation::default());
    // and a node's JSON-LD carries it (none of these is itself drawn on)
    let n = l.node(&a).unwrap();
    assert_eq!(n.curation, Curation::default());
    assert!(l.node(&b).is_some() && l.node(&v).is_some());
}

// ---------- the rename ----------

/// The family, with the root's BeatScript 2 twin stored as the store writes it (crate::gate::write_twin) and a fork of the
/// twin.
fn renamed_family() -> (Fam, String, String) {
    let f = family();
    let (_, root_text) = canon(ROOT);
    let (twin, _) = crate::gate::write_twin(f._d.path(), &f.root, &root_text, &json!({"license": "CC0-1.0", "visibility": "public", "name": "night bus"}), "2026-11-22T00:00:00Z", |v| v.to_string()).unwrap();
    let twin_fork = put(f._d.path(), &format!("#! beatscript 2\n{}", FAST.replace("126", "130")), json!({"visibility": "public", "forked_from": twin}));
    f.l.refresh(true);
    (f, twin, twin_fork)
}

#[test]
fn a_renamed_edge_is_labelled_and_is_not_a_fork() {
    let (f, twin, twin_fork) = renamed_family();
    // the old program: its forks are what they were, the twin is named, not counted
    assert_eq!(f.l.fork_count(&f.root), 2, "fast and up; the twin is not a fork");
    let t = f.l.tree(&f.root).unwrap();
    assert!(t.forks.iter().all(|n| n.cid != twin), "the twin is not in the forks");
    assert_eq!(t.root.renamed_to, twin);
    assert_eq!(f.l.renamed_to(&f.root).as_deref(), Some(twin.as_str()));
    let ld = t.json_ld("https://beatscript.app");
    assert_eq!(ld["programs"][0]["renamedTo"], twin);
    // the twin: its parent is the old program, by a renamed edge with the label and no diff
    let t = f.l.tree(&twin).unwrap();
    assert_eq!(t.ancestors.first().map(|n| n.cid.as_str()), Some(f.root.as_str()));
    assert!(t.root.renamed);
    assert_eq!(t.root.renamed_from, f.root);
    assert_eq!(t.root.fork_count, 1, "its own fork");
    let e = t.edge_to(&twin).unwrap();
    assert!(e.renamed);
    assert_eq!(e.change.as_ref().unwrap().summary, [RENAMED_LABEL]);
    let ld = t.json_ld("https://beatscript.app");
    let d = ld["derivations"].as_array().unwrap().iter().find(|d| d["to"] == twin).unwrap();
    assert_eq!(d["derivation"], "renamed");
    assert_eq!(d["isBasedOn"], json!({"@id": f.root, "derivation": "renamed"}));
    assert_eq!(d["change"], json!([RENAMED_LABEL]));
    assert_eq!(d["sameSong"], true);
    assert_eq!(ld["programs"][0]["renamedFrom"], f.root);
    // siblings are the old program's real forks; a fork's siblings never list the twin
    let sib: Vec<&str> = t.siblings.iter().map(|n| n.cid.as_str()).collect();
    assert_eq!(sib.len(), 2, "{sib:?}");
    let t = f.l.tree(&f.fast).unwrap();
    assert!(t.siblings.iter().all(|n| n.cid != twin));
    assert_eq!(t.siblings.len(), 1);
    // the twin's own fork is an ordinary fork
    let t = f.l.tree(&twin_fork).unwrap();
    assert!(!t.edge_to(&twin_fork).unwrap().renamed);
    assert_eq!(f.l.renamed_to(&twin), None);
}

#[test]
fn a_fork_that_only_says_renamed_is_a_fork() {
    // `derivation: renamed` on a program that is not the parent's twin is not a rename
    let f = family();
    let liar = put(f._d.path(), "tempo 140\nkey am\ndeck A: kick x ~ x ~ | bass a1 ~ c2 ~\n", json!({"visibility": "public", "forked_from": f.root, "derivation": "renamed"}));
    f.l.refresh(true);
    assert_eq!(f.l.fork_count(&f.root), 3);
    assert!(!f.l.tree(&liar).unwrap().root.renamed);
}

#[test]
fn an_example_in_beatscript_2_is_known_by_its_legacy_cid() {
    let d = tempfile::tempdir().unwrap();
    let text = format!("#! beatscript 2\n# an example\n{ROOT}");
    let (cid, canonical) = canon(&text);
    let old = djlive::compile::renamed_cid(&canonical);
    // a program forked from the example before the rename names its legacy CID
    let fork = put(d.path(), FAST, json!({"visibility": "public", "forked_from": old}));
    let l = Lineage::new(d.path(), vec![Example { cid: cid.clone(), name: "night-bus".into(), license: "CC0-1.0".into(), text }]);
    let t = l.tree(&fork).unwrap();
    assert!(!t.ancestors_cut);
    assert_eq!(t.ancestors[0].cid, old);
    assert!(t.ancestors[0].example);
    assert_eq!(t.ancestors[0].renamed_to, cid);
    assert_eq!(l.renamed_to(&old).as_deref(), Some(cid.as_str()));
    assert_eq!(l.node(&cid).unwrap().renamed_from, old);
}

#[test]
fn the_pages_say_renamed() {
    let (f, twin, _) = renamed_family();
    let origin = "https://beatscript.app";
    let t = f.l.tree(&twin).unwrap();
    let html = page::tree_page(origin, &t, &t.json_ld(origin));
    assert!(html.contains(&page::esc(RENAMED_LABEL)), "{html}");
    assert!(html.contains("Renamed from BeatScript 1"));
    let t = f.l.tree(&f.root).unwrap();
    let html = page::tree_page(origin, &t, &t.json_ld(origin));
    assert!(html.contains(&format!("{origin}/p/{twin}")), "the old program's tree links its twin");
    let base = "<main><h1>x</h1></main>";
    assert!(page::with_renamed_line(base, &f.root, &twin, true).contains(&format!("href=\"/p/{twin}\"")));
    assert!(page::with_renamed_line(base, &twin, &f.root, false).contains("Renamed from BeatScript 1"));
}

/// The save round trip (6670 item 4): a program saved through the storage gate keeps the `# from:` lines of the text as sent
/// in `meta.based_on` (gate::write_program; the stored text is canonical), and lineage counts it as vendoring what they name:
/// by call and by CID, on the source's node and in its JSON-LD. A `# from:` line that pins nothing is a comment.
#[test]
fn a_save_keeps_the_from_lines_and_the_source_counts_the_vendoring() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let src = put(p, ROOT, json!({"visibility": "public", "name": "the source"}));
    let line = format!("{src}.drop -> {src}");
    let text = format!("#! name: vendored\n# from: {line}\n# from: a comment that pins nothing\n{UP}# from: {line}\n");
    let a = crate::gate::admit(&text, "", "").unwrap();
    assert_eq!(a.based_on(), [line.clone()]);
    assert!(crate::gate::write_program(p, &a, json!({"license": "CC0-1.0", "visibility": "public", "name": "vendored"}), |v| v.to_string()).unwrap().is_some());
    let file: Value = serde_json::from_slice(&std::fs::read(p.join(format!("{}.json", a.cid()))).unwrap()).unwrap();
    assert_eq!(file["meta"]["based_on"], json!([line]));
    assert!(!file["text"].as_str().unwrap().contains('#'), "the stored text is canonical: {file}");
    let l = Lineage::new(p, Vec::new());
    l.refresh(true);
    assert_eq!(l.curation(&src), Curation { referenced_by: 0, vendored_from: 1 });
    assert_eq!(l.curation(&format!("{src}.drop")), Curation { referenced_by: 0, vendored_from: 1 });
    assert_eq!(l.curation("a comment that pins nothing"), Curation::default());
    let n = l.node(&src).unwrap();
    assert_eq!(n.json_ld("https://beatscript.app")["vendoredFrom"], 1);
    assert!(n.json_ld("https://beatscript.app").get("referencedBy").is_none());
    // the same notes saved again with no lines: first write wins, the kept lines stay
    let again = crate::gate::admit(UP, "", "").unwrap();
    assert_eq!(again.cid(), a.cid());
    assert!(crate::gate::write_program(p, &again, json!({}), |v| v.to_string()).unwrap().is_none());
    let file: Value = serde_json::from_slice(&std::fs::read(p.join(format!("{}.json", a.cid()))).unwrap()).unwrap();
    assert_eq!(file["meta"]["based_on"], json!([line]));
}

/// An example's file is its text as written: its own `# from:` lines count as vendoring, as a stored program's kept ones do.
#[test]
fn an_examples_own_from_lines_count_as_vendoring() {
    let d = tempfile::tempdir().unwrap();
    let src = put(d.path(), ROOT, json!({"visibility": "public"}));
    let text = format!("#! name: borrowed and written out\n# from: {src}.drop -> {src}\n{FAST}");
    let (ex, _) = canon(&text);
    let l = Lineage::new(d.path(), vec![Example { cid: ex, name: "written-out".into(), license: "CC0-1.0".into(), text }]);
    assert_eq!(l.curation(&src), Curation { referenced_by: 0, vendored_from: 1 });
}

/// A line names one program at most: a borrow must be pinned by the program it names, so `# from: <victim> -> <other>` adds
/// nothing to the victim (and is not kept); expand's own `# from <cid>.drop` over a borrowed section counts as that borrow.
#[test]
fn a_line_counts_only_for_its_pin() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let victim = put(p, ROOT, json!({"visibility": "public"}));
    let other = put(p, FAST, json!({"visibility": "public"}));
    let forged = crate::gate::admit(&format!("# from: {victim} -> {other}\n# from: {victim}.drop -> {other}\n{UP}"), "", "").unwrap();
    assert!(forged.based_on().is_empty(), "{:?}", forged.based_on());
    crate::gate::write_program(p, &forged, json!({}), |v| v.to_string()).unwrap();
    let expanded = crate::gate::admit(&format!("# from {other}.drop\n{FAST2}"), "", "").unwrap();
    assert_eq!(expanded.based_on(), [format!("{other}.drop -> {other}")]);
    crate::gate::write_program(p, &expanded, json!({}), |v| v.to_string()).unwrap();
    let l = Lineage::new(p, Vec::new());
    l.refresh(true);
    assert_eq!(l.curation(&victim), Curation::default());
    assert_eq!(l.curation(&other), Curation { referenced_by: 0, vendored_from: 1 });
}

/// A program that names sources with `use` is based on them (djlive `uses.rs`): the stored program a line names, by its CID (a
/// borrow, as `<cid>.bass` written in a part is), and the library a theme comes from; both count in the curation signal.
#[test]
fn a_use_line_is_what_a_program_is_based_on() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let root = put(p, ROOT, json!({"visibility": "public"}));
    let (_, root_text) = canon(ROOT);
    beatscript_api::known::know(&root, &root_text).unwrap();
    let named = format!("tempo 124\nkey am\nuse {root}.bass as walk\nuse theme ode_to_joy as tune\ndeck A: bass walk\ndeck B: melody tune\n");
    let a = put(p, &named, json!({"visibility": "public"}));
    let l = Lineage::new(p, Vec::new());
    l.refresh(true);
    assert_eq!(l.curation(&root).referenced_by, 1);
    assert_eq!(l.curation("theme ode_to_joy").referenced_by, 1);
    assert!(l.node(&a).is_some());
}

/// A recipe and its expansion stored beside it (roadmap 6680): the recipe's node `expandsTo` the expansion (check's expanded_cid),
/// the expansion's is `expandedFrom` the recipe and `isBasedOn` each reference it wrote out (`derivation: expanded`), each tree
/// shows the other among its programs, and the expansion's sources are not counted twice for curation.
#[test]
fn a_recipe_expands_to_its_stored_expansion_and_it_is_based_on_each_source() {
    beatscript_api::generate::install();
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let call = "gen@2 techno seed=42 section=drop part=bass";
    let recipe = format!("tempo 128\nkey c\nlet bed = {call}\nsection groove 4 bars {{\n  deck A: bass bed\n}}\narrange groove\n");
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(&recipe)).unwrap();
    let x = rep["expanded_cid"].as_str().unwrap().to_string();
    let (text, xc) = djlive::refs::expanded(&recipe, &beatscript_api::web_env().env()).unwrap().unwrap();
    assert_eq!(xc, x, "the expansion stored is check's expanded_cid");
    let r = put(p, &recipe, json!({"visibility": "public", "name": "bed", "expands_to": x}));
    let pin = djlive::refs::sites(&recipe, &beatscript_api::web_env().env()).unwrap()[0].resolved.clone().unwrap();
    assert_eq!(put(p, &text, json!({"visibility": "public", "name": "bed", "source": "expansion", "expanded_from": r, "based_on": [format!("{call} -> {pin}")]})), x);
    let l = Lineage::new(p, Vec::new());
    l.refresh(true);
    let rn = l.node(&r).unwrap();
    assert_eq!(rn.expands_to.as_deref(), Some(x.as_str()));
    assert!(rn.expanded_from.is_empty() && rn.based_on.is_empty());
    let xn = l.node(&x).unwrap();
    assert_eq!(xn.expanded_from, vec![r.clone()]);
    assert_eq!(xn.based_on, vec![(call.to_string(), Some(pin.clone()))]);
    assert!(xn.expands_to.is_none(), "a program with no references is its own expansion");
    let j = xn.json_ld("https://beatscript.app");
    assert_eq!(j["expandedFrom"], json!([r]));
    assert_eq!(j["isBasedOn"], json!([{"@type": "CreativeWork", "name": call, "derivation": "expanded", "resolves": pin}]));
    assert_eq!(rn.json_ld("https://beatscript.app")["expandsTo"], json!(x));
    // each tree shows the other
    let t = l.tree(&r).unwrap().json_ld("https://beatscript.app");
    assert!(t["programs"].as_array().unwrap().iter().any(|n| n["@id"] == json!(x)), "{t}");
    let t = l.tree(&x).unwrap().json_ld("https://beatscript.app");
    assert!(t["programs"].as_array().unwrap().iter().any(|n| n["@id"] == json!(r)), "{t}");
    // the call is drawn on once: by the recipe (the expansion is the recipe's own, not a second program that vendored it)
    assert_eq!(l.curation(call), Curation { referenced_by: 1, vendored_from: 0 });
    // the expansion's meta alone, with no `expands_to` on its recipe, still names the recipe
    let d2 = tempfile::tempdir().unwrap();
    let r2 = put(d2.path(), &recipe, json!({"visibility": "public"}));
    put(d2.path(), &text, json!({"visibility": "public", "source": "expansion", "expanded_from": r2}));
    let l2 = Lineage::new(d2.path(), Vec::new());
    l2.refresh(true);
    assert_eq!(l2.node(&x).unwrap().expanded_from, vec![r2.clone()]);
    assert_eq!(l2.node(&r2).unwrap().expands_to.as_deref(), Some(x.as_str()), "computed from the recipe's text when its meta does not say");
}
