//! /ontology/: the fork's integrity (the served release is the vendored one, the vendored one is what its inputs
//! build, it is 0.2.0 and derived from bitwrap 0.2.0, its IRIs are its own, every bitwrap term has its counterpart, the
//! certificate replays, the vocabulary matches the code, no functional property has two values), the language versions,
//! content negotiation, the routes, the MCP tools against recorded answers (`fixtures/`, written by
//! `ontology/fixtures.py`), and genre conventions read the same from the fork.

use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use std::collections::BTreeSet;

use super::graph::OWL;
use super::*;

fn onto() -> Arc<Ontology> {
    static O: OnceLock<Arc<Ontology>> = OnceLock::new();
    O.get_or_init(|| Arc::new(Ontology::load("https://beatscript.app").expect("load"))).clone()
}

fn hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn lines(b: &[u8]) -> BTreeSet<String> {
    String::from_utf8_lossy(b).lines().map(str::to_string).collect()
}

fn graph_of(format: oxigraph::io::RdfFormat, b: &[u8]) -> BTreeSet<String> {
    fork::parse(format, b, None).unwrap().iter().map(fork::nt).collect()
}

/// The fork as released: what is embedded (and so served: `routes_serve_the_release_bytes`) is the vendored release,
/// byte for byte what `ontology/build.sh` builds from the vendored inputs, pinned by its lock and its manifest.
#[test]
fn fork_integrity() {
    use oxigraph::io::{JsonLdProfileSet, RdfFormat};
    let o = onto();
    let built = fork::build_vendored().expect("the release builds from its inputs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ontology/release");
    for (name, bytes) in FILES {
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), bytes, "{name}: embedded is not the vendored file");
        assert!(built[name] == bytes, "{name}: the vendored release is not what its inputs build (run ontology/build.sh)");
    }
    assert_eq!(FILES.map(|f| f.0), fork::FILE_NAMES);
    assert_eq!(LOCK, fork::lock(&built), "release.lock");
    let files = o.manifest["files"].as_object().unwrap();
    for (name, want) in files {
        assert_eq!(hex(file(name).unwrap()), want.as_str().unwrap(), "{name} differs from manifest.json");
    }
    assert_eq!(files.len() + 1, FILES.len(), "manifest lists every file but itself");
    // the upstream input is the bitwrap 0.2.0 release its lock names
    for line in fork::UPSTREAM_LOCK.lines().filter(|l| !l.starts_with('#') && !l.starts_with("source ")) {
        let (sum, name) = line.split_once("  ").unwrap();
        let b = match name {
            "ns.nt" => fork::UPSTREAM_NT,
            "manifest.json" => fork::UPSTREAM_MANIFEST.as_bytes(),
            n => panic!("upstream.lock names {n}"),
        };
        assert_eq!(hex(b), sum, "upstream {name} differs from upstream.lock");
    }

    // version 0.3.0 (0.2.0 plus bitwrap's conventions module), its version IRI, the version before it, derived from the bitwrap 0.2.0 release
    let g = &o.graph;
    let onto_iri = format!("{}ns", fork::BASE);
    assert_eq!(o.version, "0.3.0");
    assert_eq!(g.first(&onto_iri, &format!("{OWL}versionInfo")), "0.3.0");
    assert_eq!(g.first(&onto_iri, &format!("{OWL}versionIRI")), format!("{}0.3.0", fork::BASE));
    assert_eq!(g.first(&onto_iri, &format!("{OWL}priorVersion")), format!("{}0.2.0", fork::BASE));
    assert_eq!(g.first(&onto_iri, fork::PROV_DERIVED), fork::UPSTREAM_RELEASE);
    assert!(fork::UPSTREAM_RELEASE.contains(&format!("-{}.", fork::UPSTREAM_VERSION)));
    let up: Value = serde_json::from_str(fork::UPSTREAM_MANIFEST).unwrap();
    assert_eq!(up["version"], fork::UPSTREAM_VERSION);
    assert_eq!(up["namespace"], fork::UPSTREAM_NS);
    assert_eq!(o.manifest["derived_from"]["version"], fork::UPSTREAM_VERSION);
    assert_eq!(o.manifest["derived_from"]["pack_content_sha256"], up["pack_content_sha256"]);
    assert_eq!(o.manifest["namespace"], NS);

    // every IRI is the fork's own, except the declared upstream links (the objects of prov:wasDerivedFrom)
    let fork_g = fork::parse(RdfFormat::NTriples, file("ns.nt").unwrap(), None).unwrap();
    for t in &fork_g {
        let s = t.subject.to_string();
        assert!(s.starts_with(&format!("<{}", fork::BASE)) || s.starts_with("_:"), "a subject outside the fork's base: {t}");
        assert!(!t.predicate.as_str().starts_with(fork::UPSTREAM_BASE), "a bitwrap predicate: {t}");
        if let oxigraph::model::Term::NamedNode(n) = &t.object {
            if n.as_str().starts_with(fork::UPSTREAM_BASE) {
                assert_eq!(t.predicate.as_str(), fork::PROV_DERIVED, "a bitwrap IRI that is not a derivation link: {t}");
            }
        }
    }
    // every bitwrap 0.2.0 term (and every triple but its header) has its counterpart, linked back, or is wording fork.ttl
    // restates (fork::RESTATABLE), replaced by fork.ttl's
    let have: BTreeSet<String> = fork_g.iter().map(fork::nt).collect();
    let restated = fork::restatements(fork::UPSTREAM_NT, fork::CONVENTIONS_TTL, fork::FORK_TTL).unwrap();
    for (old, new) in &restated {
        assert!(!have.contains(old) && have.contains(new), "{old} is restated as {new}");
    }
    let mut terms = BTreeSet::new();
    for t in fork::parse(RdfFormat::NTriples, fork::UPSTREAM_NT, Some("b")).unwrap() {
        let s = t.subject.to_string();
        if s == format!("<{}ns>", fork::UPSTREAM_BASE) {
            continue;
        }
        let r = fork::nt(&t).replace(&format!("<{}", fork::UPSTREAM_BASE), &format!("<{}", fork::BASE));
        assert!(have.contains(&r) || restated.contains_key(&r), "bitwrap's {t} has no counterpart");
        if s.starts_with('<') {
            terms.insert(s);
        }
    }
    assert!(terms.len() > 1000, "{}", terms.len());
    for s in &terms {
        let rest = &s[1 + fork::UPSTREAM_BASE.len()..s.len() - 1];
        let link = format!("<{}{rest}> <{}> {s} .", fork::BASE, fork::PROV_DERIVED);
        assert!(have.contains(&link), "no derivation link: {link}");
    }

    // the serializations are one graph
    for (f, fmt) in [("ns.ttl", RdfFormat::Turtle), ("ns.jsonld", RdfFormat::JsonLd { profile: JsonLdProfileSet::empty() }), ("ns.rdf", RdfFormat::RdfXml)] {
        assert_eq!(graph_of(fmt, file(f).unwrap()), have, "{f} is not ns.nt's graph");
    }

    // the certificate: about these assertions, replaying to the pack's graph, bound to it by the content digest
    let pack: Value = serde_json::from_slice(file("pack.json").unwrap()).unwrap();
    let (m, graph) = (&pack["manifest"], pack["graph"].as_str().unwrap());
    let asserted = std::str::from_utf8(file("cert/asserted.tsv").unwrap()).unwrap();
    assert_eq!(std::str::from_utf8(file("cert/asserted.sha256").unwrap()).unwrap(), format!("{}\n", hex(asserted.as_bytes())));
    assert_eq!(asserted.lines().map(|l| l.replace('\t', " ") + " .").collect::<BTreeSet<_>>(), have, "asserted.tsv is ns.nt");
    let replayed = fork::replay(asserted, std::str::from_utf8(file("cert/derivations.tsv").unwrap()).unwrap()).expect("the certificate replays");
    assert_eq!(replayed, lines(graph.as_bytes()), "the certificate concludes the pack's graph");
    let cert = &m["certificate"];
    let cfiles: std::collections::BTreeMap<String, String> = cert["files"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
    for (k, v) in &cfiles {
        assert_eq!(v.as_bytes(), file(&format!("cert/{k}")).unwrap(), "pack.json's {k} is cert/{k}");
    }
    let n = |k: &str| cert[k].as_u64().unwrap() as usize;
    let digest = fork::content_digest(graph, cert["format"].as_str().unwrap(), cert["profile_claimed_by_sender"].as_str().unwrap(), n("asserted"), n("derivations"), &cfiles);
    assert_eq!(m["content_sha256"], digest);
    assert_eq!(o.manifest["pack_content_sha256"], digest);
    assert_eq!(m["sha256"], hex(graph.as_bytes()));
    // the engine holds exactly the pack's triples
    assert_eq!(o.sp.len() as u64, o.manifest["pack_triples"].as_u64().unwrap());
    assert_eq!(o.sp.len(), replayed.len());

    // and the BeatScript vocabulary in it is the code's
    let errs = vocab::drift(g);
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

/// The release is consistent where OWL 2 RL would look first and the certificate's schema rules do not: in the
/// materialized graph no subject has two values of an owl:FunctionalProperty (two literals would be a contradiction,
/// two IRIs would make them owl:sameAs). This is why a program's BeatScript 2 CID is never a second ns:programCid of the
/// BeatScript 1 program: the BeatScript 2 program is its own individual, `prov:wasRevisionOf` the first.
#[test]
fn functional_properties_have_one_value() {
    let pack: Value = serde_json::from_slice(file("pack.json").unwrap()).unwrap();
    let triples = fork::parse(oxigraph::io::RdfFormat::NTriples, pack["graph"].as_str().unwrap().as_bytes(), None).unwrap();
    let functional: BTreeSet<String> = triples
        .iter()
        .filter(|t| t.predicate.as_str() == graph::RDF_TYPE && t.object.to_string() == format!("<{OWL}FunctionalProperty>"))
        .map(|t| t.subject.to_string())
        .collect();
    assert!(functional.len() > 30, "{}", functional.len());
    let mut values: std::collections::BTreeMap<(String, String), BTreeSet<String>> = Default::default();
    for t in &triples {
        let p = t.predicate.to_string();
        if functional.contains(&p) {
            values.entry((t.subject.to_string(), p)).or_default().insert(t.object.to_string());
        }
    }
    let clashes: Vec<String> = values.iter().filter(|(_, v)| v.len() > 1).map(|((s, p), v)| format!("{s} {p} {v:?}")).collect();
    assert!(clashes.is_empty(), "functional properties with two values:\n{}", clashes.join("\n"));
}

/// The language's versions are data, generated from djlive::version: BeatScript 2 is a revision of BeatScript 1, each
/// with its version line and what its program CIDs are the CIDs of; the language has its file extensions and media types.
#[test]
fn language_versions_are_data() {
    use djlive::version::{Version, CURRENT, LEGACY_LINE, LINE};
    let g = &onto().graph;
    let c = |s: &str| format!("{}concept/{s}", fork::BASE);
    let n = |p: &str| format!("{NS}{p}");
    let (v1, v2, lang) = (c("beatscript-1"), c("beatscript-2"), c("product-beatscript-language"));
    assert_eq!(g.of_type(&n("LanguageVersion")), [v1.clone(), v2.clone()]);
    assert_eq!(g.first(&v2, "http://www.w3.org/ns/prov#wasRevisionOf"), v1);
    assert_eq!(g.first(&v1, "http://www.w3.org/ns/prov#wasRevisionOf"), "");
    for (iri, v, line) in [(&v1, Version::V1, LEGACY_LINE), (&v2, Version::V2, LINE)] {
        assert_eq!(g.first(iri, &n("versionOf")), lang);
        assert_eq!(g.first(iri, &n("versionLine")), line);
        assert_eq!(g.first(iri, &n("programCidFormat")), v.format().to_string());
        assert_eq!(g.first(iri, &n("programCidType")), v.doc_type());
        assert_eq!(g.label(iri), format!("BeatScript {}", v.number()));
    }
    assert_eq!(g.first(&lang, &n("currentVersion")), if CURRENT == Version::V2 { v2.clone() } else { v1.clone() });
    assert_eq!(g.first(&lang, &n("fileExtension")), ".beatscript");
    assert_eq!(g.all(&lang, &n("legacyFileExtension")), [".btw"]);
    assert_eq!(g.first(&lang, &n("mediaType")), "text/beatscript");
    // what was called btw is found by that name, and says it is the name before version 2
    assert_eq!(g.label(&n("Program")), "BeatScript program");
    assert_eq!(g.all(&n("Program"), "http://www.w3.org/2004/02/skos/core#hiddenLabel"), ["btw program"]);
    assert!(g.all(&v1, "http://www.w3.org/2004/02/skos/core#altLabel").contains(&"btw".to_string()));
    assert!(g.first(&v1, "http://www.w3.org/2004/02/skos/core#historyNote").contains("before language version 2"));
    // the control noun keeps its upstream spelling, btw, and answers to beatscript too
    assert!(g.all(&c("noun-btw"), "http://www.w3.org/2004/02/skos/core#altLabel").contains(&"beatscript".to_string()));
}

#[test]
fn negotiation() {
    assert_eq!(negotiate("", &NS_OFFERS), Some("text/turtle"));
    assert_eq!(negotiate("*/*", &NS_OFFERS), Some("text/turtle"));
    assert_eq!(negotiate("application/ld+json", &NS_OFFERS), Some("application/ld+json"));
    assert_eq!(negotiate("text/html,application/xhtml+xml,*/*;q=0.8", &NS_OFFERS), Some("text/html"));
    assert_eq!(negotiate("application/rdf+xml;q=0.9, application/n-triples", &NS_OFFERS), Some("application/n-triples"));
    assert_eq!(negotiate("application/*", &NS_OFFERS), Some("application/ld+json"));
    assert_eq!(negotiate("image/png", &NS_OFFERS), None);
}

async fn send(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let r = tower::ServiceExt::oneshot(router(onto()), req).await.unwrap();
    let (p, b) = r.into_parts();
    (p.status, p.headers, axum::body::to_bytes(b, 64 << 20).await.unwrap().to_vec())
}

fn get(path: &str, accept: &str) -> Request<Body> {
    Request::get(path).header("accept", accept).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn routes_serve_the_release_bytes() {
    for (accept, f, ct) in [
        ("text/turtle", "ns.ttl", "text/turtle; charset=utf-8"),
        ("application/ld+json", "ns.jsonld", "application/ld+json; charset=utf-8"),
        ("application/rdf+xml", "ns.rdf", "application/rdf+xml; charset=utf-8"),
        ("application/n-triples", "ns.nt", "application/n-triples; charset=utf-8"),
    ] {
        let (s, h, b) = send(get("/ontology/ns", accept)).await;
        assert_eq!(s, 200, "{accept}");
        assert_eq!(h["content-type"], ct);
        assert_eq!(h["vary"], "Accept");
        assert_eq!(b, file(f).unwrap(), "{accept} -> {f}");
        // the ETag ontology.bitwrap.io sends: the first 8 bytes of the SHA-256, in hex
        assert_eq!(h["etag"].to_str().unwrap(), format!("\"{}\"", &hex(&b)[..16]));
    }
    let (s, h, _) = send(get("/ontology/ns", "text/html,*/*;q=0.8")).await;
    assert_eq!((s.as_u16(), h["location"].to_str().unwrap()), (303, "/ontology/"));
    assert_eq!(send(get("/ontology/ns", "image/png")).await.0, 406);
    for f in ["ns.ttl", "pack.json", "manifest.json", "cert/derivations.tsv"] {
        assert_eq!(send(get(&format!("/ontology/release/{f}"), "")).await.2, file(f).unwrap(), "{f}");
    }
    assert_eq!(send(get("/ontology/cert/scope.tsv", "")).await.2, file("cert/scope.tsv").unwrap());
    assert_eq!(send(get("/ontology/release/nope", "")).await.0, 404);
    let (s, _, b) = send(get("/ontology/release", "")).await;
    assert_eq!(s, 200);
    assert_eq!(String::from_utf8(b).unwrap(), onto().release_json());
    let (s, h, b) = send(get("/ontology/", "text/html")).await;
    assert_eq!(s, 200);
    assert_eq!(h["content-type"], "text/html; charset=utf-8");
    let page = String::from_utf8(b).unwrap();
    assert!(page.contains("https://beatscript.app/ontology/ns#") && page.contains(&onto().version));
    assert!(page.contains("Derived from the bitwrap ontology 0.2.0 (ontology.bitwrap.io)"));
    assert!(!page.contains("href=\"https://ontology.bitwrap.io"), "attribution is text, not a link to follow");
    assert!(!page.contains("byte for byte") && !page.contains("rel=\"alternate\" type=\"text/turtle\" href=\"https://ontology.bitwrap.io"));
    assert!(!page.contains("style=") && !page.contains("<style"), "no inline styles: the site's CSP is style-src 'self'");
    assert_eq!(send(get("/ontology", "")).await.0, 308);
    // the version IRI is the release
    let (s, _, b) = send(get(&format!("/ontology/{}", onto().version), "text/turtle")).await;
    assert_eq!((s.as_u16(), b.as_slice()), (200, file("ns.ttl").unwrap()));
    // a term's IRI answers with its triples
    let (s, h, b) = send(get("/ontology/genre/techno", "text/turtle")).await;
    assert_eq!((s.as_u16(), h["content-type"].to_str().unwrap()), (200, "text/turtle; charset=utf-8"));
    let b = String::from_utf8(b).unwrap();
    assert!(b.lines().all(|l| l.starts_with("<https://beatscript.app/ontology/genre/techno> ")), "{b}");
    assert!(b.contains("<http://www.w3.org/ns/prov#wasDerivedFrom> <https://ontology.bitwrap.io/genre/techno> ."));
    assert_eq!(send(get("/ontology/genre/techno", "text/html")).await.1["content-type"], "text/plain; charset=utf-8");
    assert_eq!(send(get("/ontology/concept/grammar-term", "application/n-triples")).await.1["content-type"], "application/n-triples; charset=utf-8");
    assert_eq!(send(get("/ontology/slot/edm-drop-2-3", "")).await.0, 200);
    assert_eq!(send(get("/ontology/genre/polka", "")).await.0, 404);
    assert_eq!(send(get("/ontology/nope/techno", "")).await.0, 404);
    assert_eq!(send(get("/ontology/cert/scope.tsv", "")).await.0, 200);
    assert_eq!(send(Request::post("/ontology/ns").body(Body::empty()).unwrap()).await.0, 405);
}

#[tokio::test]
async fn sparql_protocol() {
    let q = "SELECT ?c WHERE { ?c a <http://www.w3.org/2002/07/owl#Class> } ORDER BY ?c LIMIT 2";
    let (s, h, b) = send(Request::post("/ontology/sparql").header("content-type", "application/sparql-query").body(Body::from(q)).unwrap()).await;
    assert_eq!(s, 200);
    assert_eq!(h["content-type"], "application/sparql-results+json");
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["head"]["vars"], json!(["c"]));
    assert_eq!(v["results"]["bindings"].as_array().unwrap().len(), 2);
    let (s, _, b) = send(get("/ontology/sparql?query=ASK%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D", "")).await;
    assert_eq!((s.as_u16(), String::from_utf8(b).unwrap().as_str()), (200, "{\"boolean\":true,\"head\":{}}\n"));
    let (s, _, b) = send(Request::post("/ontology/sparql").header("content-type", "application/sparql-update").body(Body::from("DROP ALL")).unwrap()).await;
    assert_eq!((s.as_u16(), String::from_utf8(b).unwrap().as_str()), (400, "updates are not accepted\n"));
    let (s, _, _) = send(get("/ontology/sparql?query=DELETE%20WHERE%20%7B%3Fs%20%3Fp%20%3Fo%7D", "")).await;
    assert_eq!(s, 400);
}

async fn mcp(body: Value) -> Value {
    let (s, h, b) = send(Request::post("/ontology/mcp").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await;
    assert_eq!(s, 200);
    assert_eq!(h["content-type"], "application/json");
    serde_json::from_slice(&b).unwrap()
}

fn call(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

#[tokio::test]
async fn mcp_protocol() {
    let r = mcp(json!({"jsonrpc": "2.0", "id": 7, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).await;
    assert_eq!(r["result"]["serverInfo"], json!({"name": "beatscript-ontology", "version": onto().version}));
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    let r = mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["sparql_query", "describe_term", "search_terms", "list_classes", "list_genres", "get_genre", "get_release"]);
    let r = mcp(call("nope", json!({}))).await;
    assert_eq!(r["error"], json!({"code": -32602, "message": "unknown tool nope"}));
    let r = mcp(call("get_genre", json!({}))).await;
    assert_eq!(r["result"]["isError"], true);
    let (s, _, _) = send(Request::post("/ontology/mcp").body(Body::from(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)).unwrap()).await;
    assert_eq!(s, 202);
    let (s, _, _) = send(get("/ontology/mcp", "text/event-stream")).await;
    assert_eq!(s, 405);
    let r = mcp(json!({"jsonrpc": "1.0", "id": 1, "method": "ping"})).await;
    assert_eq!(r["error"]["code"], -32700);
}

/// Each fixture is one tools/call recorded from a server serving this release (`ontology/fixtures.py`):
/// `{"call": {"name", "arguments"}, "text": ...}`, `text` the content text it answered (or `error` for an isError
/// answer). Up to 0.2.0 they were recorded from ontology.bitwrap.io, the Go server this port matches; from the fork
/// on, from beatscript-serve itself, so a change to an answer is a change made on purpose. Sets of rows are compared as sets
/// where the query has no ORDER BY (an engine's row order is not part of its answer).
/// The ontology's page ends with the footer every page has: one link to /authorship (beatscript_serve::rights).
#[test]
fn the_page_has_the_rights_footer() {
    assert!(super::render_page(&onto()).contains(beatscript_serve::rights::FOOTER));
}

#[tokio::test]
async fn mcp_tools_match_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ontology/fixtures");
    let raw = std::fs::read_to_string(dir.join(format!("mcp-{}.json", onto().version))).expect("fixtures for the vendored version (ontology/fixtures.py)");
    let cases: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert!(cases.len() >= 15, "a real sample");
    for (i, c) in cases.into_iter().enumerate() {
        // one client address per call: the sample is larger than one client's rate (20 calls, then one every 3 s)
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": c["call"]}).to_string();
        let req = Request::post("/ontology/mcp").header("content-type", "application/json").header("x-real-ip", format!("198.51.100.{}", i + 1)).body(Body::from(body)).unwrap();
        let r: Value = serde_json::from_slice(&send(req).await.2).unwrap();
        assert!(r.get("result").is_some(), "{}: {r}", c["call"]);
        let res = &r["result"];
        let got = res["content"][0]["text"].as_str().unwrap_or_default();
        let label = c["call"].to_string();
        if let Some(e) = c.get("error") {
            assert_eq!(res["isError"], true, "{label}");
            assert_eq!(got, e.as_str().unwrap(), "{label}");
            continue;
        }
        assert!(res.get("isError").is_none(), "{label}: {got}");
        let want = c["text"].as_str().unwrap();
        if c.get("unordered") == Some(&json!(true)) {
            let (mut a, mut b): (Value, Value) = (serde_json::from_str(got).unwrap(), serde_json::from_str(want).unwrap());
            for v in [&mut a, &mut b] {
                if let Some(rows) = v.get_mut("rows").and_then(Value::as_array_mut) {
                    rows.sort_by_key(|x| x.to_string());
                }
                if let Some(rows) = v.get_mut("triples").and_then(Value::as_array_mut) {
                    rows.sort_by_key(|x| x.to_string());
                }
            }
            assert_eq!(a, b, "{label}");
        } else {
            assert_eq!(got, want, "{label}");
        }
    }
}

/// Genre conventions (beatscript_serve::genres) read the fork exactly as they read bitwrap 0.2.0 and its conventions
/// module: the same genres, the same conventions, the same check style warnings and templates. genres names the fork's
/// IRIs (beatscript.app/ontology/); a reader over the vendored bitwrap inputs only moves them.
#[test]
fn genres_read_the_fork_as_bitwrap() {
    use beatscript_serve::genres::{self, Graph};
    struct G {
        idx: graph::Index,
        upstream: bool,
    }
    impl G {
        fn into_(&self, iri: &str) -> String {
            if self.upstream { iri.replacen(fork::BASE, fork::UPSTREAM_BASE, 1) } else { iri.to_string() }
        }
        fn out(&self, v: String) -> String {
            if self.upstream { fork::rehome(&v) } else { v }
        }
    }
    impl Graph for G {
        fn all(&self, s: &str, p: &str) -> Vec<String> {
            let mut v: Vec<String> = self.idx.all(&self.into_(s), &self.into_(p)).into_iter().map(|x| self.out(x)).collect();
            v.sort();
            v
        }
        fn of_type(&self, class: &str) -> Vec<String> {
            let mut v: Vec<String> = self.idx.of_type(&self.into_(class)).into_iter().map(|x| self.out(x)).collect();
            v.sort();
            v
        }
    }
    let mut nt = fork::UPSTREAM_NT.to_vec();
    nt.push(b'\n');
    for t in fork::parse(oxigraph::io::RdfFormat::Turtle, fork::CONVENTIONS_TTL.as_bytes(), Some("c")).unwrap() {
        nt.extend_from_slice(format!("{} .\n", t).as_bytes());
    }
    let up = G { idx: graph::Index::from_nt(&nt), upstream: true };
    let fk = G { idx: genre_graph(), upstream: false };
    assert_eq!(genres::NS, format!("{}ns#", fork::BASE));
    let names = up.of_type(&format!("{}Genre", genres::NS));
    assert!(names.len() >= 19, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with(fork::BASE)), "{names:?}");
    assert_eq!(names, fk.of_type(&format!("{}Genre", genres::NS)));
    let text = "#! genre: techno\ntempo 128\nkick x . x . x . x .\nbass inst=sub-bass c2 . c2 .\n";
    for iri in &names {
        let slug = genres::local(iri).to_string();
        assert_eq!(genres::Genre::load(&up, iri), genres::Genre::load(&fk, iri), "{slug}");
        assert_eq!(genres::check_style(Some(&up as &dyn Graph), text, &slug), genres::check_style(Some(&fk as &dyn Graph), text, &slug), "{slug}");
        assert_eq!(genres::template_answer(Some(&up as &dyn Graph), &slug).ok(), genres::template_answer(Some(&fk as &dyn Graph), &slug).ok(), "{slug}");
        // what a program wrote before (a bitwrap IRI) still names the genre
        let old = iri.replacen(fork::BASE, fork::UPSTREAM_BASE, 1);
        assert_eq!(genres::find(&fk, &old).map(|g| g.iri), Some(iri.clone()), "{old}");
    }
    // nothing genres answers names bitwrap
    let answer = genres::template_answer(Some(&fk as &dyn Graph), "techno").unwrap().to_string();
    assert!(!answer.contains("bitwrap.io"), "{answer}");
}

/// 6672: the installed genre graph has bitwrap's conventions module (pinned by `ontology/conventions.lock`), so trap's
/// kit is drums-808 and a trap program on the 808 kit, whose chords follow trap's 1 7 6 7, has no style warning.
#[test]
fn genre_graph_has_the_conventions_and_trap_its_808() {
    use beatscript_serve::genres::{self, Graph};
    let lock = include_str!("../../ontology/conventions.lock");
    let pinned: Vec<(&str, &str)> = lock.lines().filter(|l| !l.starts_with('#') && !l.starts_with("source ")).filter_map(|l| l.split_once("  ")).collect();
    assert_eq!(pinned, [(hex(fork::CONVENTIONS_TTL.as_bytes()).as_str(), "conventions.ttl")], "conventions.ttl differs from conventions.lock");

    let g = GenreGraph(genre_graph());
    let trap = genres::find(&g, "trap").unwrap();
    assert_eq!(trap.kit.as_ref().map(|k| k.voice.as_str()), Some("drums-808"));
    assert_eq!((trap.min_bpm, trap.max_bpm), (Some(130.0), Some(170.0)));
    assert_eq!(trap.progressions, [vec![1, 7, 6, 7], vec![1, 4, 1, 7]]);
    // every kit a genre names is a BeatScript kit
    let (voices, kits) = genres::instrument_names();
    assert!(kits.contains("drums-808"), "{kits:?}");
    for x in genres::genres(&g) {
        if let Some(k) = &x.kit {
            assert!(kits.contains(&k.voice), "{}: kit {} is not a BeatScript kit", x.slug, k.voice);
        }
    }

    let style = |text: &str| genres::check_style(Some(&g as &dyn Graph), text, "trap")["warnings"].clone();
    let drums = "deck A: kick x ~ ~ ~ ~ ~ x ~ ~ ~ x ~ ~ ~ ~ ~ kit=drums-808 | snare ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ kit=drums-808 | hihat x*16 kit=drums-808\n";
    let t = format!("tempo 140\nkey em\n{drums}deck B: harmony [e3,g3,b3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~ [c3,e3,g3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~\n");
    assert_eq!(style(&t), json!([]), "{t}");
    let t = format!("tempo 140\nkey em\n{drums}deck B: harmony [e3, g3, b3] ~ ~ ~ ~ ~ ~ ~ [c3, e3, g3] ~ ~ ~ ~ ~ ~ ~\n");
    assert_eq!(style(&t), json!([]), "{t}");
    // a kit trap's family does not use is still unusual
    let w = style("tempo 140\ndeck A: kick x ~ x ~ kit=drums-cr78\n");
    assert_eq!(w[0]["code"], "kit", "{w}");
    assert!(w[0]["message"].as_str().unwrap().contains("(its kit is drums-808)"), "{w}");
    // the bitwrap ontology has no hip-hop or drill genre: no warnings, a note that says so
    for name in ["hip-hop", "drill"] {
        let v = genres::check_style(Some(&g as &dyn Graph), "tempo 90\ndeck A: kick x ~ kit=drums-808\n", name);
        assert_eq!(v["warnings"], json!([]));
        assert!(v["note"].as_str().unwrap().contains("no genre"), "{v}");
    }
    // every genre's template is in style for its genre, with the conventions in
    for x in genres::genres(&g) {
        let text = genres::template(&g, &x, &voices, &kits);
        let v = genres::check_style(Some(&g as &dyn Graph), &text, &x.slug);
        assert_eq!(v["warnings"], json!([]), "{}\n{text}", x.slug);
    }
}
