//! The BeatScript ontology as BeatScript's `#! genre:` and `#! tags:` header lines see it (docs/beatscript/ext-linked.md): a
//! deterministic snapshot of its music terms (https://beatscript.app/ontology/ns#, forked from the bitwrap ontology 0.2.0),
//! compiled in.
//!
//! `data/beatscript-ontology.json` is the source of truth for tests and for a machine that is offline: the terms an
//! author may name (genres, genre families, scales, grooves, instruments, instrument roles, song sections, song
//! structures, Euclidean rhythms, chord progressions, engine concepts), sorted by IRI, one a line, with their local
//! name, label and kind. It says where it came from, when, and whether it was checked against the release's
//! checksums (`verified`: the release's manifest and lock). Refresh it from the ontology release in this repository with
//! `python3 tools/ontology/refresh.py` (offline; nothing at build time, run time or in the test suite fetches anything),
//! then `cargo test -p djhost`.

use std::sync::OnceLock;

use djengine::json::{JsonObject, JsonValue};
use djlive::linked::{Ontology, StaticOntology, KIND_ORDER};

/// The snapshot file's text.
pub const SNAPSHOT: &str = include_str!("../data/beatscript-ontology.json");

/// The cached vocabulary, read once.
pub fn vocabulary() -> &'static StaticOntology {
    static V: OnceLock<StaticOntology> = OnceLock::new();
    V.get_or_init(|| StaticOntology::from_json(SNAPSHOT).expect("crates/djhost/data/beatscript-ontology.json is a snapshot (a test reads it)"))
}

/// One line on where the vocabulary is from: `BeatScript ontology 0.4.0, 277 terms, taken 2026-11-23, checked against the
/// release's checksums`.
pub fn describe() -> String {
    let v = vocabulary();
    let proof = if v.verified { "checked against the release's checksums" } else { "UNVERIFIED: not checked against the release's checksums" };
    format!("BeatScript ontology {}, {} terms, taken {}, {proof}", v.version, v.terms().len(), v.fetched)
}

/// The terms (of one `kind`, or all) as `btw terms` answers them: kind, local name, IRI and label, in the
/// vocabulary's order.
pub fn terms_json(kind: Option<&str>) -> Result<JsonValue, String> {
    let v = vocabulary();
    if let Some(k) = kind {
        if !KIND_ORDER.contains(&k) {
            return Err(format!("no kind {k:?} of term: one of {}", KIND_ORDER.join(", ")));
        }
    }
    let terms: Vec<JsonValue> = v
        .terms()
        .iter()
        .filter(|t| kind.is_none_or(|k| k == t.kind))
        .map(|t| JsonValue::Object(JsonObject::from_pairs([("kind", JsonValue::str(t.kind.clone())), ("local", JsonValue::str(t.local.clone())), ("iri", JsonValue::str(t.iri.clone())), ("label", JsonValue::str(t.label.clone()))])))
        .collect();
    Ok(JsonValue::Object(JsonObject::from_pairs([
        ("namespace", JsonValue::str(djlive::linked::BW_NS)),
        ("version", JsonValue::str(v.version.clone())),
        ("fetched", JsonValue::str(v.fetched.clone())),
        ("verified", v.verified.into()),
        ("kinds", JsonValue::Array(KIND_ORDER.iter().map(|k| JsonValue::str(*k)).collect())),
        ("count", JsonValue::int(terms.len() as i64)),
        ("terms", JsonValue::Array(terms)),
    ])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use djlive::linked;

    #[test]
    fn the_snapshot_reads_and_says_where_it_is_from() {
        let v = vocabulary();
        assert_eq!(v.terms().len(), 277);
        assert!(v.verified, "the committed snapshot was checked against the release's checksums");
        assert_eq!(v.version, "0.4.0");
        assert!(v.fetched.len() == 10 && v.fetched.starts_with("20"), "{}", v.fetched);
        assert!(describe().contains("checked against the release's checksums"), "{}", describe());
    }

    #[test]
    fn the_snapshot_file_is_sorted_one_term_a_line_and_small() {
        let lines: Vec<&str> = SNAPSHOT.lines().filter(|l| l.trim_start().starts_with("{\"iri\"")).collect();
        assert_eq!(lines.len(), vocabulary().terms().len());
        let iris: Vec<&str> = lines.iter().map(|l| l.split('"').nth(3).unwrap()).collect();
        let mut sorted = iris.clone();
        sorted.sort();
        assert_eq!(iris, sorted, "sorted by IRI: a refresh is a small diff");
        assert!(SNAPSHOT.len() < 64 * 1024, "{} bytes", SNAPSHOT.len());
        for key in ["\"namespace\"", "\"fetched\"", "\"source\"", "\"source_sha256\"", "\"refresh\""] {
            assert!(SNAPSHOT.contains(key), "{key}: where it is from and how to refresh it");
        }
        assert!(SNAPSHOT.contains("tools/ontology/refresh.py"));
    }

    #[test]
    fn every_term_is_under_the_base_with_its_kind_and_local_name_in_its_iri() {
        for t in vocabulary().terms() {
            assert_eq!(t.iri, format!("{}{}/{}", linked::BW_BASE, t.kind, t.local), "{t:?}");
            assert!(!t.label.is_empty());
        }
    }

    #[test]
    fn the_vocabulary_has_what_a_tag_may_name_and_the_euclidean_class() {
        let v = vocabulary();
        for (kind, n) in [("genre", 23), ("scale", 9), ("groove", 7), ("instrument", 30), ("section", 10), ("role", 8), ("family", 4), ("rhythm", 57)] {
            assert_eq!(v.terms().iter().filter(|t| t.kind == kind).count(), n, "{kind}");
        }
        assert_eq!(v.euclid_class(), Some("https://beatscript.app/ontology/ns#EuclideanRhythm"));
        // a rhythm has its shape, so a program's e(k,n,rot) can be linked to the ones of that shape
        let kick = v.terms().iter().find(|t| t.local == "techno-kick").unwrap();
        assert_eq!(kick.shape, Some(linked::Shape { steps: 16, pulses: 4, rotation: 0 }));
        assert!(v.terms().iter().filter(|t| t.kind == "rhythm").all(|t| t.shape.is_some()));
        assert!(v.terms().iter().filter(|t| t.kind != "rhythm").all(|t| t.shape.is_none()));
    }

    #[test]
    fn terms_are_listed_by_kind_and_a_wrong_kind_says_the_kinds() {
        let all = terms_json(None).unwrap();
        assert_eq!(all.get("count").and_then(JsonValue::as_f64), Some(277.0));
        let g = terms_json(Some("genre")).unwrap();
        assert_eq!(g.get("count").and_then(JsonValue::as_f64), Some(23.0));
        assert!(g.get("terms").and_then(JsonValue::as_array).unwrap().iter().any(|t| t.get("iri").and_then(JsonValue::as_str) == Some("https://beatscript.app/ontology/genre/techno")));
        assert!(terms_json(Some("flavour")).unwrap_err().contains("one of genre, instrument"));
    }
}
