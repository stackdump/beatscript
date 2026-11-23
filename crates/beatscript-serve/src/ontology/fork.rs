//! The BeatScript ontology's release builder: the fork of the bitwrap ontology 0.2.0 that `/ontology/` serves.
//!
//! A release is built from three inputs, all in `ontology/` and all in git, and nothing is fetched:
//!
//! - `upstream/bitwrap-0.2.0/ns.nt`: the bitwrap release the fork derives from (vendored by `sync.sh upstream`, pinned
//!   by `upstream.lock`). Every IRI under `https://ontology.bitwrap.io/` is re-homed under
//!   `https://beatscript.app/ontology/` (`…/ns#Genre` -> `https://beatscript.app/ontology/ns#Genre`, `…/genre/techno` ->
//!   `https://beatscript.app/ontology/genre/techno`), its `owl:Ontology` header is dropped, and every term it describes
//!   gets one `prov:wasDerivedFrom` to the bitwrap IRI it came from. That is the only link back: no `owl:sameAs` or
//!   equivalence, so reasoning over the fork never pulls bitwrap's IRIs in.
//! - `fork.ttl`: the fork's header (version, version IRI, `prov:wasDerivedFrom` the bitwrap 0.2.0 release) and the
//!   terms it adds by hand.
//! - `generated/beatscript.ttl`: the BeatScript vocabulary, generated from beatscript-serve's own code (`vocab.rs` dumps it,
//!   `tools/ontology/beatscript.py` writes the Turtle; `vocab::drift` holds the release to the code).
//!
//! [`build`] turns them into the release files, byte for byte reproducibly: `ns.nt` (sorted), `ns.ttl`, `ns.jsonld`,
//! `ns.rdf`, the RDFS / OWL 2 RL schema closure with its derivation certificate (`cert/`, in open-ontologies'
//! `oo-cert/1` format: one step per inferred triple, premises before conclusions), `pack.json` (the materialized
//! graph and the certificate, bound by open-ontologies' `oo-pack-content/1` digest) and `manifest.json`.
//! `ONTOLOGY_FORK_OUT=<dir> cargo test -p beatscript-serve --bin beatscript-serve fork_write` writes them (`build.sh` does, then the
//! lock); `tests::fork_release_is_the_build` fails when the vendored release is not what the inputs build.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use oxigraph::io::{JsonLdProfileSet, RdfFormat, RdfParser, RdfSerializer};
use oxigraph::model::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::graph::{NS, SITE_BASE};

/// The fork's base: its individuals are `{BASE}genre/techno`, its vocabulary `{BASE}ns#Genre`.
pub const BASE: &str = "https://beatscript.app/ontology/";
pub const UPSTREAM_BASE: &str = "https://ontology.bitwrap.io/";
pub const UPSTREAM_NS: &str = "https://ontology.bitwrap.io/ns#";
pub const UPSTREAM_VERSION: &str = "0.2.0";
/// The bitwrap 0.2.0 release as its own VoID names it (`void:dataDump`): the release states no `owl:versionIRI`, and
/// its header's `owl:versionInfo` still says 0.1.0, so the versioned release file is the IRI that names 0.2.0.
pub const UPSTREAM_RELEASE: &str = "https://ontology.bitwrap.io/release/bitwrap-ontology-0.2.0.tar";
pub const PROV_DERIVED: &str = "http://www.w3.org/ns/prov#wasDerivedFrom";
pub const NAME: &str = "beatscript-ontology";
/// What `pack.json` names as the tool that packed it (open-ontologies writes its own version there).
pub const TOOL: &str = "beatscript-serve-ontology-fork/1";
pub const PROFILE: &str = "owl-rl";
pub const CERTIFICATE_SCOPE: &str = "RDFS and OWL 2 RL schema rules only (rdfs2, rdfs3, rdfs5, rdfs7, rdfs9, rdfs11, eq-sym, prp-inv1, prp-inv2, prp-symp, prp-trp, scm-dom1, scm-dom2, scm-eqc1, scm-eqp1, scm-rng1, scm-rng2); other OWL axioms are not covered";
const SCOPE_TSV: &str = "oo-scope/1\nselector\twhole-store\nsemantics_version\ttemporal/2\ndefault_graph\ttrue\ncomplete\ttrue\ngraph\t*\n";

pub const UPSTREAM_NT: &[u8] = include_bytes!("../../ontology/upstream/bitwrap-0.2.0/ns.nt");
pub const UPSTREAM_MANIFEST: &str = include_str!("../../ontology/upstream/bitwrap-0.2.0/manifest.json");
pub const UPSTREAM_LOCK: &str = include_str!("../../ontology/upstream.lock");
pub const FORK_TTL: &str = include_str!("../../ontology/fork.ttl");
pub const VOCAB_TTL: &str = include_str!("../../ontology/generated/beatscript.ttl");

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
/// The serializations' prefixes. No prefix for bitwrap's base: its IRIs (the `prov:wasDerivedFrom` objects) are written
/// out whole, a path such as `genre/techno` being no Turtle local name.
const PREFIXES: [(&str, &str); 9] = [
    ("rdf", RDF),
    ("rdfs", RDFS),
    ("owl", OWL),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("dct", "http://purl.org/dc/terms/"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("ns", NS),
    ("pflow", "https://ontology.pflow.xyz/ns#"),
];

/// The release files [`build`] writes, in `FILES` order (`ontology.rs`).
pub const FILE_NAMES: [&str; 10] = ["ns.ttl", "ns.nt", "ns.jsonld", "ns.rdf", "manifest.json", "pack.json", "cert/asserted.sha256", "cert/asserted.tsv", "cert/derivations.tsv", "cert/scope.tsv"];

/// One RDF document, parsed. `bnodes`: rename its blank nodes `{prefix}{n}` in the order they first appear, so a
/// Turtle `[]` (which the parser names at random) has the same label every build.
pub fn parse(format: RdfFormat, data: &[u8], bnodes: Option<&str>) -> Result<Vec<Triple>, String> {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut rename = |b: &BlankNode| -> BlankNode {
        let n = names.len();
        let px = bnodes.unwrap_or("");
        BlankNode::new_unchecked(names.entry(b.as_str().to_string()).or_insert_with(|| format!("{px}{n}")).clone())
    };
    let mut out = Vec::new();
    for q in RdfParser::from_format(format).for_slice(data) {
        let mut t: Triple = q.map_err(|e| e.to_string())?.into();
        if bnodes.is_some() {
            if let NamedOrBlankNode::BlankNode(b) = &t.subject {
                t.subject = rename(b).into();
            }
            if let Term::BlankNode(b) = &t.object {
                t.object = rename(b).into();
            }
        }
        out.push(t);
    }
    Ok(out)
}

/// A bitwrap IRI under the fork's base; any other IRI unchanged.
pub fn rehome(iri: &str) -> String {
    match iri.strip_prefix(UPSTREAM_BASE) {
        Some(rest) => format!("{BASE}{rest}"),
        None => iri.to_string(),
    }
}

fn rehome_node(n: &NamedNode) -> NamedNode {
    NamedNode::new_unchecked(rehome(n.as_str()))
}

fn rehome_triple(t: &Triple) -> Triple {
    let subject = match &t.subject {
        NamedOrBlankNode::NamedNode(n) => rehome_node(n).into(),
        s => s.clone(),
    };
    let object = match &t.object {
        Term::NamedNode(n) => rehome_node(n).into(),
        o => o.clone(),
    };
    Triple::new(subject, rehome_node(&t.predicate), object)
}

/// A triple's N-Triples line, without the newline.
pub fn nt(t: &Triple) -> String {
    format!("{t} .")
}

/// The fork's asserted graph: the re-homed upstream with its derivation links, `fork.ttl` and the vocabulary, keyed and
/// sorted by N-Triples line.
pub fn assemble(upstream_nt: &[u8], fork_ttl: &str, vocab_ttl: &str) -> Result<BTreeMap<String, Triple>, String> {
    let header = format!("{UPSTREAM_BASE}ns");
    let mut g = BTreeMap::new();
    let mut terms = BTreeSet::new();
    for t in parse(RdfFormat::NTriples, upstream_nt, Some("b"))? {
        if let NamedOrBlankNode::NamedNode(s) = &t.subject {
            if s.as_str() == header {
                continue; // bitwrap's own header: the fork has its own
            }
            if s.as_str().starts_with(UPSTREAM_BASE) {
                terms.insert(s.as_str().to_string());
            }
        }
        let r = rehome_triple(&t);
        g.insert(nt(&r), r);
    }
    let derived = NamedNode::new_unchecked(PROV_DERIVED);
    for s in terms {
        let t = Triple::new(NamedNode::new_unchecked(rehome(&s)), derived.clone(), NamedNode::new_unchecked(s));
        g.insert(nt(&t), t);
    }
    for (doc, px, name) in [(fork_ttl, "f", "fork.ttl"), (vocab_ttl, "v", "generated/beatscript.ttl")] {
        for t in parse(RdfFormat::Turtle, doc.as_bytes(), Some(px)).map_err(|e| format!("{name}: {e}"))? {
            g.insert(nt(&t), t);
        }
    }
    Ok(g)
}

// ---------- the closure and its certificate ----------

/// A rule pattern position: a variable or a constant (its N-Triples spelling).
#[derive(Clone, Copy)]
enum P {
    V(usize),
    C(&'static str),
}

const TYPE: P = P::C("<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>");
const SUBCLASS: P = P::C("<http://www.w3.org/2000/01/rdf-schema#subClassOf>");
const SUBPROP: P = P::C("<http://www.w3.org/2000/01/rdf-schema#subPropertyOf>");
const DOMAIN: P = P::C("<http://www.w3.org/2000/01/rdf-schema#domain>");
const RANGE: P = P::C("<http://www.w3.org/2000/01/rdf-schema#range>");
const TRANSITIVE: P = P::C("<http://www.w3.org/2002/07/owl#TransitiveProperty>");
const SYMMETRIC: P = P::C("<http://www.w3.org/2002/07/owl#SymmetricProperty>");
const INVERSE: P = P::C("<http://www.w3.org/2002/07/owl#inverseOf>");
const SAMEAS: P = P::C("<http://www.w3.org/2002/07/owl#sameAs>");
const EQ_CLASS: P = P::C("<http://www.w3.org/2002/07/owl#equivalentClass>");
const EQ_PROP: P = P::C("<http://www.w3.org/2002/07/owl#equivalentProperty>");
use P::V;

/// One rule: its name as `derivations.tsv` writes it, its premises in the order the checker matches them, and its
/// conclusion. open-ontologies' `BUILTIN_RULES` for the `owl-rl` profile (`RULES_EVALUATED`), the same table.
pub struct Rule {
    pub name: &'static str,
    body: &'static [[P; 3]],
    head: [P; 3],
}

pub const RULES: &[Rule] = &[
    Rule { name: "rdfs2", body: &[[V(0), V(1), V(2)], [V(1), DOMAIN, V(3)]], head: [V(0), TYPE, V(3)] },
    Rule { name: "rdfs3", body: &[[V(0), V(1), V(2)], [V(1), RANGE, V(3)]], head: [V(2), TYPE, V(3)] },
    Rule { name: "rdfs5", body: &[[V(0), SUBPROP, V(1)], [V(1), SUBPROP, V(2)]], head: [V(0), SUBPROP, V(2)] },
    Rule { name: "rdfs7", body: &[[V(0), V(1), V(2)], [V(1), SUBPROP, V(3)]], head: [V(0), V(3), V(2)] },
    Rule { name: "rdfs9", body: &[[V(0), TYPE, V(1)], [V(1), SUBCLASS, V(2)]], head: [V(0), TYPE, V(2)] },
    Rule { name: "rdfs11", body: &[[V(0), SUBCLASS, V(1)], [V(1), SUBCLASS, V(2)]], head: [V(0), SUBCLASS, V(2)] },
    Rule { name: "prp-trp", body: &[[V(0), TYPE, TRANSITIVE], [V(1), V(0), V(2)], [V(2), V(0), V(3)]], head: [V(1), V(0), V(3)] },
    Rule { name: "prp-symp", body: &[[V(0), TYPE, SYMMETRIC], [V(1), V(0), V(2)]], head: [V(2), V(0), V(1)] },
    Rule { name: "prp-inv1", body: &[[V(0), INVERSE, V(1)], [V(2), V(0), V(3)]], head: [V(3), V(1), V(2)] },
    Rule { name: "prp-inv2", body: &[[V(0), INVERSE, V(1)], [V(2), V(1), V(3)]], head: [V(3), V(0), V(2)] },
    Rule { name: "eq-sym", body: &[[V(0), SAMEAS, V(1)]], head: [V(1), SAMEAS, V(0)] },
    Rule { name: "scm-eqc1", body: &[[V(0), EQ_CLASS, V(1)]], head: [V(0), SUBCLASS, V(1)] },
    Rule { name: "scm-eqc1", body: &[[V(0), EQ_CLASS, V(1)]], head: [V(1), SUBCLASS, V(0)] },
    Rule { name: "scm-eqp1", body: &[[V(0), EQ_PROP, V(1)]], head: [V(0), SUBPROP, V(1)] },
    Rule { name: "scm-eqp1", body: &[[V(0), EQ_PROP, V(1)]], head: [V(1), SUBPROP, V(0)] },
    Rule { name: "scm-dom1", body: &[[V(0), DOMAIN, V(1)], [V(1), SUBCLASS, V(2)]], head: [V(0), DOMAIN, V(2)] },
    Rule { name: "scm-dom2", body: &[[V(0), DOMAIN, V(1)], [V(2), SUBPROP, V(0)]], head: [V(2), DOMAIN, V(1)] },
    Rule { name: "scm-rng1", body: &[[V(0), RANGE, V(1)], [V(1), SUBCLASS, V(2)]], head: [V(0), RANGE, V(2)] },
    Rule { name: "scm-rng2", body: &[[V(0), RANGE, V(1)], [V(2), SUBPROP, V(0)]], head: [V(2), RANGE, V(1)] },
];

/// A triple of terms in N-Triples spelling.
pub type T3 = [String; 3];

/// A conclusion is written only when it is RDF: no literal subject, an IRI predicate.
fn writable(t: &T3) -> bool {
    !t[0].starts_with('"') && t[1].starts_with('<')
}

fn spell(t: &Triple) -> T3 {
    [t.subject.to_string(), t.predicate.to_string(), t.object.to_string()]
}

/// One step of the certificate: the rule, the conclusion, the premises in the rule's order.
pub struct Step {
    pub rule: &'static str,
    pub conclusion: T3,
    pub premises: Vec<T3>,
}

/// The fixpoint of [`RULES`] over `asserted`: each round runs every rule over what the rounds before it hold, and a
/// triple is recorded the first time it is reached (rule order, then the order its premises were added), so every
/// premise of a step is asserted or the conclusion of an earlier step.
pub fn closure(asserted: &[T3]) -> Vec<Step> {
    let mut have: HashSet<T3> = asserted.iter().cloned().collect();
    // predicate -> (subject, object) in the order they were added
    let mut by_p: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for t in asserted {
        by_p.entry(t[1].clone()).or_default().push((t[0].clone(), t[2].clone()));
    }
    let c = |p: P| match p {
        P::C(s) => s,
        P::V(_) => unreachable!(),
    };
    let mut steps = Vec::new();
    loop {
        let mut new: Vec<Step> = Vec::new();
        let mut seen: HashSet<T3> = HashSet::new();
        let empty = Vec::new();
        let get = |p: &str| by_p.get(p).unwrap_or(&empty);
        let index = |p: &str| {
            let mut m: HashMap<&str, Vec<&str>> = HashMap::new();
            for (s, o) in get(p) {
                m.entry(s.as_str()).or_default().push(o.as_str());
            }
            m
        };
        let sup = index(c(SUBCLASS));
        let subp_of = index(c(SUBPROP));
        let mut emit = |rule: &'static str, conclusion: T3, premises: Vec<T3>| {
            if writable(&conclusion) && !have.contains(&conclusion) && seen.insert(conclusion.clone()) {
                new.push(Step { rule, conclusion, premises });
            }
        };
        let t3 = |a: &str, b: &str, c: &str| [a.to_string(), b.to_string(), c.to_string()];
        let (ty, sc, sp, dom, rng) = (c(TYPE), c(SUBCLASS), c(SUBPROP), c(DOMAIN), c(RANGE));
        for (p, cl) in get(dom) {
            for (s, o) in get(p) {
                emit("rdfs2", t3(s, ty, cl), vec![t3(s, p, o), t3(p, dom, cl)]);
            }
        }
        for (p, cl) in get(rng) {
            for (s, o) in get(p) {
                emit("rdfs3", t3(o, ty, cl), vec![t3(s, p, o), t3(p, rng, cl)]);
            }
        }
        for (a, b) in get(sp) {
            for cc in subp_of.get(b.as_str()).into_iter().flatten() {
                emit("rdfs5", t3(a, sp, cc), vec![t3(a, sp, b), t3(b, sp, cc)]);
            }
        }
        for (p, q) in get(sp) {
            for (s, o) in get(p) {
                emit("rdfs7", t3(s, q, o), vec![t3(s, p, o), t3(p, sp, q)]);
            }
        }
        for (x, a) in get(ty) {
            for b in sup.get(a.as_str()).into_iter().flatten() {
                emit("rdfs9", t3(x, ty, b), vec![t3(x, ty, a), t3(a, sc, b)]);
            }
        }
        for (a, b) in get(sc) {
            for cc in sup.get(b.as_str()).into_iter().flatten() {
                emit("rdfs11", t3(a, sc, cc), vec![t3(a, sc, b), t3(b, sc, cc)]);
            }
        }
        for (p, k) in get(ty) {
            if k == c(TRANSITIVE) {
                let succ = index(p);
                for (x, y) in get(p) {
                    for z in succ.get(y.as_str()).into_iter().flatten() {
                        emit("prp-trp", t3(x, p, z), vec![t3(p, ty, k), t3(x, p, y), t3(y, p, z)]);
                    }
                }
            }
        }
        for (p, k) in get(ty) {
            if k == c(SYMMETRIC) {
                for (x, y) in get(p) {
                    emit("prp-symp", t3(y, p, x), vec![t3(p, ty, k), t3(x, p, y)]);
                }
            }
        }
        let inv = c(INVERSE);
        for (p, q) in get(inv) {
            for (x, y) in get(p) {
                emit("prp-inv1", t3(y, q, x), vec![t3(p, inv, q), t3(x, p, y)]);
            }
        }
        for (p, q) in get(inv) {
            for (x, y) in get(q) {
                emit("prp-inv2", t3(y, p, x), vec![t3(p, inv, q), t3(x, q, y)]);
            }
        }
        for (a, b) in get(c(SAMEAS)) {
            emit("eq-sym", t3(b, c(SAMEAS), a), vec![t3(a, c(SAMEAS), b)]);
        }
        for (a, b) in get(c(EQ_CLASS)) {
            emit("scm-eqc1", t3(a, sc, b), vec![t3(a, c(EQ_CLASS), b)]);
            emit("scm-eqc1", t3(b, sc, a), vec![t3(a, c(EQ_CLASS), b)]);
        }
        for (a, b) in get(c(EQ_PROP)) {
            emit("scm-eqp1", t3(a, sp, b), vec![t3(a, c(EQ_PROP), b)]);
            emit("scm-eqp1", t3(b, sp, a), vec![t3(a, c(EQ_PROP), b)]);
        }
        for (rule, pred) in [("scm-dom1", dom), ("scm-rng1", rng)] {
            for (p, c1) in get(pred) {
                for c2 in sup.get(c1.as_str()).into_iter().flatten() {
                    emit(rule, t3(p, pred, c2), vec![t3(p, pred, c1), t3(c1, sc, c2)]);
                }
            }
        }
        for (rule, pred) in [("scm-dom2", dom), ("scm-rng2", rng)] {
            let of = index(pred);
            for (p1, p2) in get(sp) {
                for cl in of.get(p2.as_str()).into_iter().flatten() {
                    emit(rule, t3(p1, pred, cl), vec![t3(p2, pred, cl), t3(p1, sp, p2)]);
                }
            }
        }
        if new.is_empty() {
            return steps;
        }
        for s in &new {
            have.insert(s.conclusion.clone());
            by_p.entry(s.conclusion[1].clone()).or_default().push((s.conclusion[0].clone(), s.conclusion[2].clone()));
        }
        steps.extend(new);
    }
}

/// Replays a certificate as a checker would: every step's premises are asserted or concluded by an earlier step, and
/// its conclusion is what its rule concludes from them. Ok: the asserted triples and every conclusion, as N-Triples
/// lines (the materialized graph).
pub fn replay(asserted_tsv: &str, derivations_tsv: &str) -> Result<BTreeSet<String>, String> {
    let fields = |l: &str| l.split('\t').map(str::to_string).collect::<Vec<_>>();
    let mut have: HashSet<T3> = HashSet::new();
    for (i, l) in asserted_tsv.lines().enumerate() {
        let f = fields(l);
        let [s, p, o]: [String; 3] = f.try_into().map_err(|_| format!("asserted.tsv line {}: not three fields", i + 1))?;
        have.insert([s, p, o]);
    }
    for (i, l) in derivations_tsv.lines().enumerate() {
        let mut f = fields(l);
        let rule = f.remove(0);
        if f.is_empty() || f.len() % 3 != 0 {
            return Err(format!("derivations.tsv line {}: not a rule and whole triples", i + 1));
        }
        let ts: Vec<T3> = f.chunks(3).map(|c| [c[0].clone(), c[1].clone(), c[2].clone()]).collect();
        let (conclusion, premises) = (&ts[0], &ts[1..]);
        if let Some(p) = premises.iter().find(|p| !have.contains(*p)) {
            return Err(format!("derivations.tsv line {}: premise {p:?} is neither asserted nor concluded earlier", i + 1));
        }
        let fits = RULES.iter().filter(|r| r.name == rule && r.body.len() == premises.len()).any(|r| {
            let mut vars: [Option<&str>; 6] = [None; 6];
            let mut ok = true;
            for (pat, t) in r.body.iter().zip(premises) {
                for (p, v) in pat.iter().zip(t) {
                    ok &= match *p {
                        P::C(c) => c == v,
                        P::V(n) => *vars[n].get_or_insert(v.as_str()) == v.as_str(),
                    };
                }
            }
            ok && r.head.iter().zip(conclusion).all(|(p, v)| match *p {
                P::C(c) => c == v,
                P::V(n) => vars[n] == Some(v.as_str()),
            })
        });
        if !fits {
            return Err(format!("derivations.tsv line {}: {rule} does not conclude {conclusion:?} from its premises", i + 1));
        }
        have.insert(conclusion.clone());
    }
    Ok(have.iter().map(|t| format!("{} {} {} .", t[0], t[1], t[2])).collect())
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// open-ontologies' `content_digest` (`oo-pack-content/1`): the graph and the certificate, every part length-prefixed.
pub fn content_digest(graph: &str, format: &str, profile: &str, asserted: usize, derivations: usize, files: &BTreeMap<String, String>) -> String {
    fn put(h: &mut Sha256, b: &[u8]) {
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    }
    let mut h = Sha256::new();
    h.update(b"oo-pack-content/1\n");
    put(&mut h, graph.as_bytes());
    h.update(1u64.to_le_bytes()); // a certificate is present
    put(&mut h, format.as_bytes());
    put(&mut h, profile.as_bytes());
    h.update((asserted as u64).to_le_bytes());
    h.update((derivations as u64).to_le_bytes());
    h.update((files.len() as u64).to_le_bytes());
    for (name, body) in files {
        put(&mut h, name.as_bytes());
        put(&mut h, body.as_bytes());
    }
    hex::encode(h.finalize())
}

fn serialize(format: RdfFormat, g: &BTreeMap<String, Triple>) -> Result<Vec<u8>, String> {
    let mut s = RdfSerializer::from_format(format);
    for (p, iri) in PREFIXES {
        s = s.with_prefix(p, iri).map_err(|e| e.to_string())?;
    }
    let mut w = s.for_writer(Vec::new());
    for t in g.values() {
        w.serialize_triple(t).map_err(|e| e.to_string())?;
    }
    w.finish().map_err(|e| e.to_string())
}

/// The literal value of `<{BASE}ns> <p> ?v` in the fork's header.
fn header(g: &BTreeMap<String, Triple>, p: &str) -> Result<String, String> {
    let s = format!("{BASE}ns");
    g.values()
        .find(|t| matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == s) && t.predicate.as_str() == p)
        .and_then(|t| match &t.object {
            Term::Literal(l) => Some(l.value().to_string()),
            Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .ok_or_else(|| format!("fork.ttl: the ontology header has no {p}"))
}

/// Every release file from the three inputs (and the upstream manifest, for the provenance the manifest records).
pub fn build(upstream_nt: &[u8], upstream_manifest: &str, fork_ttl: &str, vocab_ttl: &str) -> Result<BTreeMap<&'static str, Vec<u8>>, String> {
    debug_assert_eq!(SITE_BASE, BASE.trim_end_matches('/'));
    let g = assemble(upstream_nt, fork_ttl, vocab_ttl)?;
    let version = header(&g, &format!("{OWL}versionInfo"))?;
    let version_iri = header(&g, &format!("{OWL}versionIRI"))?;
    let released = header(&g, "http://purl.org/dc/terms/issued")?;
    let derived_from = header(&g, PROV_DERIVED)?;
    if version_iri != format!("{BASE}{version}") {
        return Err(format!("fork.ttl: owl:versionIRI {version_iri} is not {BASE}{version}"));
    }
    let up: Value = serde_json::from_str(upstream_manifest).map_err(|e| format!("upstream manifest.json: {e}"))?;

    let mut files: BTreeMap<&'static str, Vec<u8>> = BTreeMap::new();
    let nt_text: String = g.keys().map(|l| format!("{l}\n")).collect();
    files.insert("ns.nt", nt_text.into_bytes());
    files.insert("ns.ttl", serialize(RdfFormat::Turtle, &g)?);
    files.insert("ns.jsonld", serialize(RdfFormat::JsonLd { profile: JsonLdProfileSet::empty() }, &g)?);
    files.insert("ns.rdf", serialize(RdfFormat::RdfXml, &g)?);

    // the closure, its certificate and the pack
    let asserted: Vec<T3> = g.values().map(spell).collect();
    let steps = closure(&asserted);
    let asserted_tsv: String = asserted.iter().map(|t| format!("{}\t{}\t{}\n", t[0], t[1], t[2])).collect();
    let mut derivations_tsv = String::new();
    for s in &steps {
        derivations_tsv.push_str(s.rule);
        for t in std::iter::once(&s.conclusion).chain(&s.premises) {
            derivations_tsv.push_str(&format!("\t{}\t{}\t{}", t[0], t[1], t[2]));
        }
        derivations_tsv.push('\n');
    }
    let cert: BTreeMap<String, String> = [
        ("asserted.sha256", format!("{}\n", sha(asserted_tsv.as_bytes()))),
        ("asserted.tsv", asserted_tsv),
        ("derivations.tsv", derivations_tsv),
        ("scope.tsv", SCOPE_TSV.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut materialized: BTreeSet<String> = g.keys().cloned().collect();
    materialized.extend(steps.iter().map(|s| format!("{} {} {} .", s.conclusion[0], s.conclusion[1], s.conclusion[2])));
    let graph: String = materialized.iter().map(|l| format!("{l}\n")).collect();
    let digest = content_digest(&graph, "oo-cert/1", PROFILE, asserted.len(), steps.len(), &cert);
    let pack = json!({
        "manifest": {
            "name": NAME,
            "version": version,
            "created_at": format!("{released}T00:00:00+00:00"),
            "tool_version": TOOL,
            "triples": materialized.len(),
            "sha256": sha(graph.as_bytes()),
            "certificate": {
                "format": "oo-cert/1",
                "files": cert,
                "asserted": asserted.len(),
                "derivations": steps.len(),
                "profile_claimed_by_sender": PROFILE,
            },
            "content_sha256": digest,
        },
        "graph": graph,
    });
    files.insert("pack.json", (serde_json::to_string_pretty(&pack).unwrap() + "\n").into_bytes());
    for (k, v) in &cert {
        files.insert(Box::leak(format!("cert/{k}").into_boxed_str()), v.clone().into_bytes());
    }

    let hashes: BTreeMap<&str, String> = files.iter().map(|(k, v)| (*k, sha(v))).collect();
    let manifest = json!({
        "name": NAME,
        "version": version,
        "released": released,
        "namespace": NS,
        "version_iri": version_iri,
        "derived_from": {
            "name": up["name"],
            "version": up["version"],
            "iri": derived_from,
            "namespace": up["namespace"],
            "pack_content_sha256": up["pack_content_sha256"],
            "ns.nt": sha(upstream_nt),
        },
        "pack_content_sha256": digest,
        "pack_tool_version": TOOL,
        "pack_triples": materialized.len(),
        "license": "CC0-1.0",
        "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
        "certificate_scope": CERTIFICATE_SCOPE,
        "files": hashes,
    });
    files.insert("manifest.json", (serde_json::to_string_pretty(&manifest).unwrap() + "\n").into_bytes());
    debug_assert!(FILE_NAMES.iter().all(|n| files.contains_key(n)) && files.len() == FILE_NAMES.len());
    Ok(files)
}

/// [`build`] from the vendored inputs.
pub fn build_vendored() -> Result<BTreeMap<&'static str, Vec<u8>>, String> {
    build(UPSTREAM_NT, UPSTREAM_MANIFEST, FORK_TTL, VOCAB_TTL)
}

/// `release.lock`: the sha256 of every release file, as `sha256sum` prints them, under a header naming the inputs.
pub fn lock(files: &BTreeMap<&'static str, Vec<u8>>) -> String {
    let mut s = String::from("# The BeatScript ontology release, built by ontology/build.sh (src/ontology/fork.rs): sha256 per file. Do not edit; rebuild.\n");
    s.push_str(&format!("inputs upstream/bitwrap-0.2.0/ns.nt {} fork.ttl {} generated/beatscript.ttl {}\n", sha(UPSTREAM_NT), sha(FORK_TTL.as_bytes()), sha(VOCAB_TTL.as_bytes())));
    for n in FILE_NAMES {
        s.push_str(&format!("{}  {n}\n", sha(&files[n])));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the release built from the vendored inputs into `ONTOLOGY_FORK_OUT` (and its lock beside it).
    #[test]
    fn fork_write() {
        let Ok(dir) = std::env::var("ONTOLOGY_FORK_OUT") else { return };
        let files = build_vendored().unwrap();
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir.join("cert")).unwrap();
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).unwrap();
        }
        std::fs::write(dir.join("../release.lock"), lock(&files)).unwrap();
        eprintln!("wrote {} files into {}", files.len(), dir.display());
    }

    /// The closure, run on the bitwrap 0.2.0 release itself, reaches exactly as many triples as that release's own
    /// pack (open-ontologies, owl-rl), and its certificate replays.
    #[test]
    fn closure_matches_bitwrap_0_2_0() {
        let up: Value = serde_json::from_str(UPSTREAM_MANIFEST).unwrap();
        let asserted: Vec<T3> = parse(RdfFormat::NTriples, UPSTREAM_NT, None).unwrap().iter().map(spell).collect();
        let steps = closure(&asserted);
        assert_eq!((asserted.len() + steps.len()) as u64, up["pack_triples"].as_u64().unwrap());
        let a: String = asserted.iter().map(|t| format!("{}\t{}\t{}\n", t[0], t[1], t[2])).collect();
        let d: String = steps.iter().map(|s| std::iter::once(s.rule.to_string()).chain(std::iter::once(&s.conclusion).chain(&s.premises).flat_map(|t| t.iter().cloned())).collect::<Vec<_>>().join("\t") + "\n").collect();
        assert_eq!(replay(&a, &d).unwrap().len(), asserted.len() + steps.len());
        // a forged step is refused
        let forged = d.replacen("rdfs9", "rdfs11", 1);
        assert!(replay(&a, &forged).is_err());
    }

    /// Against a bitwrap release's own pack.json (`ONTOLOGY_UPSTREAM_PACK=<path>`; skipped without it): the closure is
    /// its graph exactly, its open-ontologies certificate replays here to the same graph, and [`content_digest`] is
    /// its `content_sha256`. Run it when merging a later bitwrap release in.
    #[test]
    fn upstream_pack_reproduced() {
        let Ok(p) = std::env::var("ONTOLOGY_UPSTREAM_PACK") else { return };
        let pack: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let (m, graph) = (&pack["manifest"], pack["graph"].as_str().unwrap());
        let cert = &m["certificate"];
        let files: BTreeMap<String, String> = cert["files"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        let n = |k: &str| cert[k].as_u64().unwrap() as usize;
        let digest = content_digest(graph, cert["format"].as_str().unwrap(), cert["profile_claimed_by_sender"].as_str().unwrap_or(""), n("asserted"), n("derivations"), &files);
        assert_eq!(digest, m["content_sha256"].as_str().unwrap());
        let want: BTreeSet<String> = graph.lines().map(str::to_string).collect();
        assert_eq!(replay(&files["asserted.tsv"], &files["derivations.tsv"]).unwrap(), want, "their certificate, our checker");
        let asserted: Vec<T3> = parse(RdfFormat::NTriples, UPSTREAM_NT, None).unwrap().iter().map(spell).collect();
        let mut got: BTreeSet<String> = asserted.iter().map(|t| format!("{} {} {} .", t[0], t[1], t[2])).collect();
        got.extend(closure(&asserted).iter().map(|s| format!("{} {} {} .", s.conclusion[0], s.conclusion[1], s.conclusion[2])));
        // ns.nt and the pack name their blank nodes differently (open-ontologies relabels on load): the rest is equal
        let named = |s: &BTreeSet<String>| s.iter().filter(|l| !l.contains("_:")).cloned().collect::<BTreeSet<_>>();
        assert_eq!(got.len(), want.len());
        assert_eq!(named(&got), named(&want), "our closure, their graph");
    }

    #[test]
    fn rehome_only_moves_bitwrap() {
        assert_eq!(rehome("https://ontology.bitwrap.io/genre/techno"), "https://beatscript.app/ontology/genre/techno");
        assert_eq!(rehome("https://ontology.bitwrap.io/ns#Genre"), format!("{NS}Genre"));
        assert_eq!(rehome("https://ontology.pflow.xyz/ns#PetriNet"), "https://ontology.pflow.xyz/ns#PetriNet");
    }
}
