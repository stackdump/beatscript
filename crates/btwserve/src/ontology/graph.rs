//! The asserted ontology in memory (ontology-bitwrap-io `index.go`, `nterm.go`, `mcp_terms.go`, `genres.go`): subject
//! -> predicate -> objects, read from the release's `ns.nt`. The term views, the class list and the genre tools answer
//! from it, not from the query engine: the engine holds the materialized graph, whose `rdfs:subClassOf` is the
//! inferred closure, so "direct superclass" can only be answered from what was asserted.
//!
//! JSON is built as the Go server builds it: a Go map becomes an object with sorted keys, a Go struct keeps its field
//! order, and `sort.Slice` (unstable) is Go's own pdqsort, ported in [`go_sort_slice`], so rows with equal keys come
//! out in the order the live server puts them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use crate::compose::gojson;

pub const NS: &str = "https://beatscript.app/ontology/ns#";
pub const SITE_BASE: &str = "https://beatscript.app/ontology";
pub const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub const SKOS: &str = "http://www.w3.org/2004/02/skos/core#";
pub const OWL: &str = "http://www.w3.org/2002/07/owl#";
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const DCT_SOURCE: &str = "http://purl.org/dc/terms/source";

/// One RDF term in SPARQL 1.1 JSON results form (`term` in nterm.go).
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub kind: &'static str, // uri | literal | bnode
    pub value: String,
    pub lang: String,
    pub datatype: String,
}

impl Term {
    /// `{"type","value","xml:lang"?,"datatype"?}`, the struct's field order with omitempty.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), json!(self.kind));
        m.insert("value".into(), json!(self.value));
        if !self.lang.is_empty() {
            m.insert("xml:lang".into(), json!(self.lang));
        }
        if !self.datatype.is_empty() {
            m.insert("datatype".into(), json!(self.datatype));
        }
        Value::Object(m)
    }
}

/// `parseTerm`: one N-Triples-encoded term: `<iri>`, `_:label`, `"lex"`, `"lex"@lang` or `"lex"^^<datatype>`.
pub fn parse_term(s: &str) -> Option<Term> {
    if s.starts_with('<') && s.ends_with('>') && s.len() >= 2 {
        return Some(Term { kind: "uri", value: s[1..s.len() - 1].to_string(), lang: String::new(), datatype: String::new() });
    }
    if let Some(l) = s.strip_prefix("_:") {
        return Some(Term { kind: "bnode", value: l.to_string(), lang: String::new(), datatype: String::new() });
    }
    if s.starts_with('"') {
        let end = closing_quote(s)?;
        let lex = unescape(&s[1..end])?;
        let mut t = Term { kind: "literal", value: lex, lang: String::new(), datatype: String::new() };
        let rest = &s[end + 1..];
        if let Some(l) = rest.strip_prefix('@') {
            t.lang = l.to_string();
        } else if rest.starts_with("^^<") && rest.ends_with('>') {
            t.datatype = rest[3..rest.len() - 1].to_string();
        } else if !rest.is_empty() {
            return None;
        }
        return Some(t);
    }
    None
}

fn closing_quote(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'"' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn unescape(s: &str) -> Option<String> {
    if !s.contains('\\') {
        return Some(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.char_indices();
    while let Some((_, c)) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let (i, e) = it.next()?;
        match e {
            't' => out.push('\t'),
            'b' => out.push('\u{8}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'f' => out.push('\u{c}'),
            '"' | '\'' | '\\' => out.push(e),
            'u' | 'U' => {
                let n = if e == 'U' { 8 } else { 4 };
                let hex = s.get(i + 1..i + 1 + n)?;
                let r = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)?;
                out.push(r);
                for _ in 0..n {
                    it.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The asserted graph: subject value -> predicate IRI -> objects in file order (`index` in index.go).
#[derive(Default)]
pub struct Index {
    m: HashMap<String, HashMap<String, Vec<Term>>>,
}

impl Index {
    /// `indexNT`: lines that are not `s p o .` are skipped, as are terms that do not parse.
    pub fn from_nt(b: &[u8]) -> Index {
        let mut idx = Index::default();
        for raw in String::from_utf8_lossy(b).split('\n') {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || !line.ends_with(" .") {
                continue;
            }
            let line = &line[..line.len() - 2];
            let Some(s_end) = line.find(' ') else { continue };
            let rest = &line[s_end + 1..];
            let Some(p_end) = rest.find(' ') else { continue };
            let (Some(s), Some(p), Some(o)) = (parse_term(&line[..s_end]), parse_term(&rest[..p_end]), parse_term(rest[p_end + 1..].trim())) else { continue };
            idx.m.entry(s.value).or_default().entry(p.value).or_default().push(o);
        }
        idx
    }

    fn objs(&self, s: &str, p: &str) -> &[Term] {
        self.m.get(s).and_then(|ps| ps.get(p)).map_or(&[], |v| v.as_slice())
    }

    pub fn first(&self, s: &str, p: &str) -> String {
        self.objs(s, p).first().map(|t| t.value.clone()).unwrap_or_default()
    }

    /// Every object's value, sorted.
    pub fn all(&self, s: &str, p: &str) -> Vec<String> {
        let mut v: Vec<String> = self.objs(s, p).iter().map(|t| t.value.clone()).collect();
        v.sort();
        v
    }

    pub fn is_a(&self, s: &str, class: &str) -> bool {
        self.objs(s, RDF_TYPE).iter().any(|t| t.value == class)
    }

    pub fn has(&self, s: &str) -> bool {
        self.m.get(s).is_some_and(|ps| !ps.is_empty())
    }

    /// `num`: the first object parsed as a float64.
    pub fn num(&self, s: &str, p: &str) -> Option<f64> {
        let v = self.first(s, p);
        if v.is_empty() {
            return None;
        }
        go_parse_float(&v)
    }

    pub fn local(iri: &str) -> String {
        match iri.rfind(['#', '/']) {
            Some(k) => iri[k + 1..].to_string(),
            None => iri.to_string(),
        }
    }

    /// Subjects typed `class`, sorted.
    pub fn of_type(&self, class: &str) -> Vec<String> {
        let mut out: Vec<String> = self.m.keys().filter(|s| self.is_a(s, class)).cloned().collect();
        out.sort();
        out
    }

    /// `directSupers`: the asserted superclasses, less any another listed one already implies.
    pub fn direct_supers(&self, class: &str) -> Vec<String> {
        let asserted: BTreeSet<String> = self.all(class, &format!("{RDFS}subClassOf")).into_iter().filter(|s| s != class && s.contains("://")).collect();
        let mut out: Vec<String> = asserted
            .iter()
            .filter(|s| {
                !asserted.iter().any(|o| {
                    let mut seen = BTreeSet::from([class.to_string()]);
                    o != *s && self.reaches_super(o, s, &mut seen)
                })
            })
            .cloned()
            .collect();
        out.sort();
        out
    }

    fn reaches_super(&self, from: &str, to: &str, seen: &mut BTreeSet<String>) -> bool {
        if seen.contains(from) {
            return false;
        }
        seen.insert(from.to_string());
        for s in self.all(from, &format!("{RDFS}subClassOf")) {
            if s == to || self.reaches_super(&s, to, seen) {
                return true;
            }
        }
        false
    }

    /// `nsClasses`: the owl:Class IRIs in the ontology namespace, sorted.
    pub fn ns_classes(&self) -> Vec<String> {
        let mut out: Vec<String> = self.m.keys().filter(|s| s.starts_with(NS) && self.is_a(s, &format!("{OWL}Class"))).cloned().collect();
        out.sort();
        out
    }

    /// `closeMatches`: every concept linked to iri by skos:closeMatch, in either direction, sorted.
    pub fn close_matches(&self, iri: &str) -> Vec<String> {
        let cm = format!("{SKOS}closeMatch");
        let mut set: BTreeSet<String> = self.all(iri, &cm).into_iter().collect();
        for (s, ps) in &self.m {
            if ps.get(&cm).is_some_and(|os| os.iter().any(|o| o.value == iri)) {
                set.insert(s.clone());
            }
        }
        set.into_iter().collect()
    }

    /// `definition`: its own skos:definition, else a closeMatching concept's (named in `via`), else its rdfs:comment.
    pub fn definition(&self, iri: &str) -> (String, String) {
        let d = self.first(iri, &format!("{SKOS}definition"));
        if !d.is_empty() {
            return (d, String::new());
        }
        for c in self.close_matches(iri) {
            let d = self.first(&c, &format!("{SKOS}definition"));
            if !d.is_empty() {
                return (d, c);
            }
        }
        (self.first(iri, &format!("{RDFS}comment")), String::new())
    }

    pub fn label(&self, iri: &str) -> String {
        for p in [format!("{RDFS}label"), format!("{SKOS}prefLabel")] {
            let l = self.first(iri, &p);
            if !l.is_empty() {
                return l;
            }
        }
        Index::local(iri)
    }

    /// `view`: describe_term's JSON shape for one IRI (the `termView` struct, its field order, omitempty).
    pub fn view(&self, iri: &str) -> Option<Value> {
        if !self.has(iri) {
            return None;
        }
        let owl_class = format!("{OWL}Class");
        let mut v = Map::new();
        v.insert("iri".into(), json!(iri));
        v.insert("label".into(), json!(self.label(iri)));
        put_list(&mut v, "types", self.all(iri, RDF_TYPE));
        let (def, via) = self.definition(iri);
        put_str(&mut v, "definition", &def);
        put_str(&mut v, "definition_from", &via);
        let c = self.first(iri, &format!("{RDFS}comment"));
        if c != def {
            put_str(&mut v, "comment", &c);
        }
        if self.is_a(iri, &owl_class) {
            put_list(&mut v, "superclasses", self.direct_supers(iri));
            let mut subs: Vec<String> = self.m.keys().filter(|s| s.as_str() != iri && self.is_a(s, &owl_class) && self.direct_supers(s).iter().any(|x| x == iri)).cloned().collect();
            subs.sort();
            put_list(&mut v, "subclasses", subs);
        }
        let cms = self.close_matches(iri);
        let mut sources = self.all(iri, DCT_SOURCE);
        for c in &cms {
            sources.extend(self.all(c, DCT_SOURCE));
        }
        put_list(&mut v, "close_match", cms);
        let mut seen = BTreeSet::new();
        sources.retain(|s| seen.insert(s.clone()));
        put_list(&mut v, "sources", sources);
        Some(Value::Object(v))
    }

    /// list_classes: one row per class and direct superclass (a class with none gets one row without).
    pub fn classes_json(&self) -> String {
        let mut rows = Vec::new();
        for c in self.ns_classes() {
            let sups = self.direct_supers(&c);
            if sups.is_empty() {
                rows.push(json!({"class": c}));
            }
            for s in sups {
                rows.push(json!({"class": c, "superclass": s}));
            }
        }
        let out = gojson::obj([
            ("note", json!("direct superclasses as asserted; the materialized graph also holds the inferred closure")),
            ("rows", Value::Array(rows.into_iter().map(gojson::sorted).collect())),
            ("vars", json!(["class", "superclass"])),
        ]);
        gojson::indent(&out, " ")
    }

    /// list_genres (genres.go `listGenres`).
    pub fn list_genres(&self, family: &str, min_bpm: i64, max_bpm: i64) -> String {
        let mut rows: Vec<(String, Option<f64>, Value)> = Vec::new();
        for iri in self.of_type(&format!("{NS}Genre")) {
            let fam = Index::local(&self.first(&iri, &format!("{NS}inFamily")));
            let bpm = self.num(&iri, &format!("{NS}bpm"));
            if !family.is_empty() && fam != family {
                continue;
            }
            if let Some(b) = bpm {
                let b = b as i64; // Go's int(f): truncation toward zero
                if (min_bpm > 0 && b < min_bpm) || (max_bpm > 0 && b > max_bpm) {
                    continue;
                }
            }
            let genre = Index::local(&iri);
            let row = gojson::sorted(json!({
                "genre": genre, "iri": iri, "family": fam, "bpm": bpm.map_or(Value::Null, gojson::num),
                "scale": Index::local(&self.first(&iri, &format!("{NS}defaultScale"))),
                "groove": Index::local(&self.first(&iri, &format!("{NS}defaultGroove"))),
                "definition": self.first(&iri, &format!("{SKOS}definition")),
            }));
            rows.push((genre, bpm, row));
        }
        go_sort_slice(&mut rows, |a, b| match (a.1, b.1) {
            (Some(fa), Some(fb)) => fa < fb,
            _ => a.0 < b.0,
        });
        let n = rows.len();
        let out = gojson::obj([("count", json!(n)), ("genres", Value::Array(rows.into_iter().map(|r| r.2).collect()))]);
        gojson::indent(&out, " ")
    }

    /// get_genre (genres.go `getGenre`).
    pub fn get_genre(&self, slug: &str) -> Result<String, String> {
        let slug = slug.trim().to_lowercase();
        if !is_slug(&slug) {
            return Err("genre must be a slug such as techno".into());
        }
        let n = |p: &str| format!("{NS}{p}");
        let iri = format!("{SITE_BASE}/genre/{slug}");
        if !self.is_a(&iri, &n("Genre")) {
            let known: Vec<String> = self.of_type(&n("Genre")).iter().map(|s| Index::local(s)).collect();
            return Err(format!("no such genre {slug}; known: {}", known.join(", ")));
        }
        let num = |s: &str, p: &str| self.num(s, &n(p)).map_or(Value::Null, gojson::num);
        let mut out = BTreeMap::<String, Value>::new();
        out.insert("genre".into(), json!(slug));
        out.insert("iri".into(), json!(iri));
        out.insert("label".into(), json!(self.label(&iri)));
        out.insert("definition".into(), json!(self.first(&iri, &format!("{SKOS}definition"))));
        let fam = self.first(&iri, &n("inFamily"));
        out.insert("family".into(), json!(Index::local(&fam)));
        if !fam.is_empty() {
            out.insert("family_definition".into(), json!(self.first(&fam, &format!("{SKOS}definition"))));
        }
        let mut props = BTreeMap::<String, Value>::new();
        for p in ["bpm", "rootNote", "swing", "humanize", "durationVariation", "melodyDensity", "melodyDuration", "bassDensity", "bassDuration", "syncopation", "ghostNotes", "modalInterchange", "polyrhythm"] {
            if let Some(v) = self.num(&iri, &n(p)) {
                props.insert(p.into(), gojson::num(v));
            }
        }
        for p in ["drumFills", "walkingBass", "callResponse", "tensionCurve"] {
            if self.first(&iri, &n(p)) == "true" {
                props.insert(p.into(), json!(true));
            }
        }
        out.insert("properties".into(), Value::Object(props.into_iter().collect()));
        let sc = self.first(&iri, &n("defaultScale"));
        out.insert("scale".into(), gojson::sorted(json!({"name": Index::local(&sc), "intervals": self.first(&sc, &n("intervals")), "definition": self.first(&sc, &format!("{SKOS}definition"))})));
        let gr = self.first(&iri, &n("defaultGroove"));
        out.insert("groove".into(), gojson::sorted(json!({"name": Index::local(&gr), "definition": self.first(&gr, &format!("{SKOS}definition"))})));
        let mut rh = Map::new();
        for role in ["hihat", "kick", "snare"] {
            let r = self.first(&iri, &n(&format!("{role}Rhythm")));
            rh.insert(role.into(), gojson::sorted(json!({"pulses": num(&r, "pulses"), "steps": num(&r, "steps"), "rotation": num(&r, "rotation"), "midi_note": num(&r, "midiNote")})));
        }
        out.insert("rhythms".into(), Value::Object(rh));
        let mut ins = BTreeMap::<String, Value>::new();
        for p in ["melodyInstrument", "arpInstrument", "bassInstrument", "drumKit"] {
            let v = self.first(&iri, &n(p));
            if !v.is_empty() {
                ins.insert(p.into(), json!(Index::local(&v)));
            }
        }
        out.insert("instruments".into(), Value::Object(ins.into_iter().collect()));
        let mut progs: Vec<Value> = self
            .all(&iri, &n("hasProgression"))
            .into_iter()
            .map(|p| gojson::sorted(json!({"iri": p, "degrees": self.first(&p, &n("degrees")), "steps_per_chord": num(&p, "stepsPerChord")})))
            .collect();
        progs.sort_by(|a, b| a["iri"].as_str().cmp(&b["iri"].as_str()));
        out.insert("progressions".into(), if progs.is_empty() { Value::Null } else { Value::Array(progs) });
        let structs: Vec<Value> = self
            .of_type(&n("SongStructure"))
            .into_iter()
            .filter(|s| self.first(s, &n("forFamily")) == fam)
            .map(|s| gojson::sorted(json!({"structure": Index::local(&s), "kind": self.first(&s, &n("templateKind")), "label": self.label(&s)})))
            .collect();
        out.insert("structures".into(), if structs.is_empty() { Value::Null } else { Value::Array(structs) });
        let mut links = Vec::new();
        for p in [format!("{SKOS}closeMatch"), format!("{SKOS}relatedMatch")] {
            links.extend(self.all(&iri, &p));
        }
        out.insert("wikidata".into(), if links.is_empty() { Value::Null } else { json!(links) });
        Ok(gojson::indent(&Value::Object(out.into_iter().collect()), " "))
    }
}

fn put_str(m: &mut Map<String, Value>, k: &str, v: &str) {
    if !v.is_empty() {
        m.insert(k.into(), json!(v));
    }
}

fn put_list(m: &mut Map<String, Value>, k: &str, v: Vec<String>) {
    if !v.is_empty() {
        m.insert(k.into(), json!(v));
    }
}

/// `reSlug`: `^[a-z0-9][a-z0-9-]{0,80}$`.
pub fn is_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 81 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// strconv.ParseFloat for the lexical forms the release holds (integers and decimals).
fn go_parse_float(s: &str) -> Option<f64> {
    if s.is_empty() || s.eq_ignore_ascii_case("nan") || s.to_ascii_lowercase().contains("inf") {
        return s.parse().ok();
    }
    s.parse::<f64>().ok()
}

// ---------- Go's sort.Slice (sort/zsortfunc.go, pdqsort), ported literally ----------

/// `sort.Slice(x, less)`: Go's unstable pdqsort, so equal elements end in the order Go leaves them.
pub fn go_sort_slice<T>(x: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let n = x.len();
    let limit = usize::BITS - n.leading_zeros();
    let mut d = Data { x, less: &less };
    pdqsort(&mut d, 0, n, limit as usize);
}

struct Data<'a, T, F: Fn(&T, &T) -> bool> {
    x: &'a mut [T],
    less: &'a F,
}

impl<T, F: Fn(&T, &T) -> bool> Data<'_, T, F> {
    fn less(&self, i: usize, j: usize) -> bool {
        (self.less)(&self.x[i], &self.x[j])
    }
    fn swap(&mut self, i: usize, j: usize) {
        self.x.swap(i, j)
    }
}

#[derive(PartialEq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}

fn insertion_sort<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) {
    for i in a + 1..b {
        let mut j = i;
        while j > a && d.less(j, j - 1) {
            d.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn sift_down<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, lo: usize, hi: usize, first: usize) {
    let mut root = lo;
    loop {
        let mut child = 2 * root + 1;
        if child >= hi {
            break;
        }
        if child + 1 < hi && d.less(first + child, first + child + 1) {
            child += 1;
        }
        if !d.less(first + root, first + child) {
            return;
        }
        d.swap(first + root, first + child);
        root = child;
    }
}

fn heap_sort<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) {
    let first = a;
    let hi = b - a;
    let mut i = (hi as isize - 1) / 2;
    while i >= 0 {
        sift_down(d, i as usize, hi, first);
        i -= 1;
    }
    let mut i = hi as isize - 1;
    while i >= 0 {
        d.swap(first, first + i as usize);
        sift_down(d, 0, i as usize, first);
        i -= 1;
    }
}

fn pdqsort<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, mut a: usize, mut b: usize, mut limit: usize) {
    const MAX_INSERTION: usize = 12;
    let (mut was_balanced, mut was_partitioned) = (true, true);
    loop {
        let length = b - a;
        if length <= MAX_INSERTION {
            insertion_sort(d, a, b);
            return;
        }
        if limit == 0 {
            heap_sort(d, a, b);
            return;
        }
        if !was_balanced {
            break_patterns(d, a, b);
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(d, a, b);
        if hint == Hint::Decreasing {
            reverse_range(d, a, b);
            pivot = (b - 1) - (pivot - a);
            hint = Hint::Increasing;
        }
        if was_balanced && was_partitioned && hint == Hint::Increasing && partial_insertion_sort(d, a, b) {
            return;
        }
        if a > 0 && !d.less(a - 1, pivot) {
            a = partition_equal(d, a, b, pivot);
            continue;
        }
        let (mid, already) = partition(d, a, b, pivot);
        was_partitioned = already;
        let (left, right) = (mid - a, b - mid);
        let threshold = length / 8;
        if left < right {
            was_balanced = left >= threshold;
            pdqsort(d, a, mid, limit);
            a = mid + 1;
        } else {
            was_balanced = right >= threshold;
            pdqsort(d, mid + 1, b, limit);
            b = mid;
        }
    }
}

fn partition<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize, pivot: usize) -> (usize, bool) {
    d.swap(a, pivot);
    let (mut i, mut j) = (a as isize + 1, b as isize - 1);
    while i <= j && d.less(i as usize, a) {
        i += 1;
    }
    while i <= j && !d.less(j as usize, a) {
        j -= 1;
    }
    if i > j {
        d.swap(j as usize, a);
        return (j as usize, true);
    }
    d.swap(i as usize, j as usize);
    i += 1;
    j -= 1;
    loop {
        while i <= j && d.less(i as usize, a) {
            i += 1;
        }
        while i <= j && !d.less(j as usize, a) {
            j -= 1;
        }
        if i > j {
            break;
        }
        d.swap(i as usize, j as usize);
        i += 1;
        j -= 1;
    }
    d.swap(j as usize, a);
    (j as usize, false)
}

fn partition_equal<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize, pivot: usize) -> usize {
    d.swap(a, pivot);
    let (mut i, mut j) = (a as isize + 1, b as isize - 1);
    loop {
        while i <= j && !d.less(a, i as usize) {
            i += 1;
        }
        while i <= j && d.less(a, j as usize) {
            j -= 1;
        }
        if i > j {
            break;
        }
        d.swap(i as usize, j as usize);
        i += 1;
        j -= 1;
    }
    i as usize
}

fn partial_insertion_sort<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) -> bool {
    const MAX_STEPS: usize = 5;
    const SHORTEST_SHIFTING: usize = 50;
    let mut i = a + 1;
    for _ in 0..MAX_STEPS {
        while i < b && !d.less(i, i - 1) {
            i += 1;
        }
        if i == b {
            return true;
        }
        if b - a < SHORTEST_SHIFTING {
            return false;
        }
        d.swap(i, i - 1);
        if i - a >= 2 {
            let mut j = i - 1;
            while j >= 1 {
                if !d.less(j, j - 1) {
                    break;
                }
                d.swap(j, j - 1);
                j -= 1;
            }
        }
        if b - i >= 2 {
            for j in i + 1..b {
                if !d.less(j, j - 1) {
                    break;
                }
                d.swap(j, j - 1);
            }
        }
    }
    false
}

fn break_patterns<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) {
    let length = b - a;
    if length >= 8 {
        let mut random = length as u64;
        let modulus = 1usize << (usize::BITS - length.leading_zeros());
        let start = a + (length / 4) * 2 - 1;
        for idx in start..=a + (length / 4) * 2 + 1 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let mut other = (random as usize) & (modulus - 1);
            if other >= length {
                other -= length;
            }
            d.swap(idx, a + other);
        }
    }
}

fn choose_pivot<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) -> (usize, Hint) {
    const SHORTEST_NINTHER: usize = 50;
    const MAX_SWAPS: usize = 4 * 3;
    let l = b - a;
    let mut swaps = 0;
    let (mut i, mut j, mut k) = (a + l / 4, a + l / 4 * 2, a + l / 4 * 3);
    if l >= 8 {
        if l >= SHORTEST_NINTHER {
            i = median(d, i - 1, i, i + 1, &mut swaps);
            j = median(d, j - 1, j, j + 1, &mut swaps);
            k = median(d, k - 1, k, k + 1, &mut swaps);
        }
        j = median(d, i, j, k, &mut swaps);
    }
    match swaps {
        0 => (j, Hint::Increasing),
        MAX_SWAPS => (j, Hint::Decreasing),
        _ => (j, Hint::Unknown),
    }
}

fn order2<T, F: Fn(&T, &T) -> bool>(d: &Data<T, F>, a: usize, b: usize, swaps: &mut usize) -> (usize, usize) {
    if d.less(b, a) {
        *swaps += 1;
        return (b, a);
    }
    (a, b)
}

fn median<T, F: Fn(&T, &T) -> bool>(d: &Data<T, F>, a: usize, b: usize, c: usize, swaps: &mut usize) -> usize {
    let (a, b) = order2(d, a, b, swaps);
    let (b, c) = order2(d, b, c, swaps);
    let _ = c;
    let (_a, b) = order2(d, a, b, swaps);
    b
}

fn reverse_range<T, F: Fn(&T, &T) -> bool>(d: &mut Data<T, F>, a: usize, b: usize) {
    let (mut i, mut j) = (a, b - 1);
    while i < j {
        d.swap(i, j);
        i += 1;
        j -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms() {
        assert_eq!(parse_term("<http://x/a>").unwrap().value, "http://x/a");
        let t = parse_term(r#""a\"bé"@en"#).unwrap();
        assert_eq!((t.value.as_str(), t.lang.as_str()), ("a\"bé", "en"));
        let t = parse_term(r#""5"^^<http://www.w3.org/2001/XMLSchema#integer>"#).unwrap();
        assert_eq!(t.datatype, "http://www.w3.org/2001/XMLSchema#integer");
        assert!(parse_term(r#""x" junk"#).is_none());
    }

    /// Go's sort.Slice on ([key, original index]) by key, for inputs whose result Go fixes (recorded from Go 1.26:
    /// `sort.Slice` over 0..n with less = key[i] < key[j]).
    #[test]
    fn go_pdqsort_matches_go() {
        let keys = [140, 110, 128, 140, 108, 95, 174, 140, 120, 108, 90, 85, 110, 124, 70, 100, 130, 160, 75];
        let run = |n: usize| {
            let mut v: Vec<(i32, usize)> = (0..n).map(|i| (keys[i % 19], i)).collect();
            go_sort_slice(&mut v, |a, b| a.0 < b.0);
            v.into_iter().map(|x| x.1).collect::<Vec<_>>()
        };
        // 19: the insertion-sort-free pdqsort path at the genre count; 60: the ninther and partial insertion paths
        assert_eq!(run(19), [14, 18, 11, 10, 5, 15, 9, 4, 12, 1, 8, 13, 2, 16, 7, 0, 3, 17, 6]);
        assert_eq!(
            run(60),
            [
                52, 14, 33, 37, 56, 18, 30, 11, 49, 48, 10, 29, 43, 5, 24, 15, 53, 34, 9, 28, 47, 42, 4, 23, 50, 58, 20, 31, 39, 12, 1, 8, 46, 27, 51, 32, 13, 40, 59, 21, 2, 35, 16, 54, 38, 22, 0, 57, 41, 3, 7, 19, 26,
                45, 55, 36, 17, 25, 6, 44
            ]
        );
    }
}
