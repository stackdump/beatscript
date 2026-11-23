//! The programs store, read and written in exactly the Go server's format (`<data>/programs/<cid>.json`,
//! `{"cid","text","meta"}`), so the Go and Rust servers can share one data directory while both run.
//! Examples answer first, as in Go (`Backend.Get`).
//!
//! The rename (BeatScript 1 -> 2): examples are read from `*.beatscript` and the legacy `*.btw`; an example written in
//! BeatScript 2 is also answered under its legacy CID, with its legacy text, so old example links and stored forks that name
//! one keep resolving. [`Store::renamed_to`] and [`Store::renamed_from`] pair a program with its twin when both are here.
//!
//! Read-only (`-programs-readonly`, a shadow next to the Go server): nothing is written; a program that arrives in a
//! `<cid>.<packed>` URL is checked exactly as for storing and then held in memory (at most `MAX_LINKS`), so the
//! render works and the Go server's store stays the Go server's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct Program {
    pub cid: String,
    /// Canonical text.
    pub text: String,
}

/// `^z[1-9A-HJ-NP-Za-km-z]{20,80}$` (Go's `reCID`).
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() != Some(&b'z') || !(21..=81).contains(&b.len()) {
        return false;
    }
    b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// The CheckReport of `text` (beatscript-api's, the same JSON `beatscript check` prints), as serde JSON.
pub fn check(text: &str) -> Value {
    serde_json::from_str(&beatscript_api::check_json(text)).unwrap_or(Value::Null)
}

/// `api.DeclaresNotPublic`: a `#! visibility:` line whose value is anything but `public`, in any spelling.
pub fn declares_not_public(text: &str) -> bool {
    text.split('\n').any(|l| {
        let Some(rest) = l.trim().strip_prefix("#!") else { return false };
        let squeezed: String = rest.split_whitespace().collect::<String>().to_lowercase();
        matches!(squeezed.strip_prefix("visibility:"), Some(v) if v != "public")
    })
}

fn header_name(text: &str) -> String {
    for l in text.split('\n') {
        if let Some(rest) = l.trim().strip_prefix("#!") {
            if let Some(v) = rest.trim().strip_prefix("name:") {
                let v: String = v.trim().chars().collect();
                let v = if v.len() > 80 { v[..v.floor_char_boundary(80)].to_string() } else { v };
                return v.chars().filter(|c| (*c as u32) >= 0x20 && *c != '\u{7f}').collect();
            }
        }
    }
    String::new()
}

/// Go's json.Marshal escapes <, > and & (HTML-safe); the stored files keep that shape.
fn go_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

pub const MAX_LINKS: usize = 4096;

pub struct Store {
    dir: PathBuf,
    examples: HashMap<String, Program>,
    readonly: bool,
    links: std::sync::Mutex<HashMap<String, Program>>,
}

impl Store {
    pub fn open(dir: &Path, examples: Option<&Path>, readonly: bool) -> std::io::Result<(Store, Vec<String>)> {
        let dir = dir.to_path_buf();
        if !readonly {
            std::fs::create_dir_all(&dir)?;
        }
        let mut ex = HashMap::new();
        let mut warnings = Vec::new();
        if let Some(edir) = examples {
            match std::fs::read_dir(edir) {
                Err(e) => warnings.push(format!("no examples: {e}")),
                Ok(rd) => {
                    for e in rd.flatten() {
                        let p = e.path();
                        // `.btw` is the legacy spelling, read as an alias
                        if !matches!(p.extension().and_then(|x| x.to_str()), Some("beatscript" | "btw")) {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(&p) else { continue };
                        let rep = check(&text);
                        let ok = rep["ok"] == true && rep["restricted"] != true && rep["uses_assets"] != true && !declares_not_public(&text);
                        match (ok, rep["cid"].as_str(), rep["canonical"].as_str()) {
                            (true, Some(cid), Some(canon)) => {
                                ex.insert(cid.to_string(), Program { cid: cid.to_string(), text: canon.to_string() });
                                // a BeatScript 2 example is also its legacy CID, with the legacy text (the CID of that text)
                                if djlive::version::of_canonical(canon) == djlive::version::Version::V2 {
                                    let old = djlive::version::legacy_canonical(canon).to_string();
                                    let old_cid = djlive::compile::program_cid(&old);
                                    ex.entry(old_cid.clone()).or_insert(Program { cid: old_cid, text: old });
                                }
                            }
                            _ => warnings.push(format!("example {} left out: it does not check, or is restricted", p.display())),
                        }
                    }
                }
            }
        }
        Ok((Store { dir, examples: ex, readonly, links: std::sync::Mutex::new(HashMap::new()) }, warnings))
    }

    /// The program named by `cid`, or None. A stored file whose text does not hash to its CID is not a program.
    pub fn get(&self, cid: &str) -> Option<Program> {
        if !is_cid(cid) {
            return None;
        }
        if let Some(p) = self.examples.get(cid) {
            return Some(p.clone());
        }
        if let Some(p) = self.links.lock().unwrap().get(cid) {
            return Some(p.clone());
        }
        let b = std::fs::read(self.dir.join(format!("{cid}.json"))).ok()?;
        let v: Value = serde_json::from_slice(&b).ok()?;
        let text = v["text"].as_str()?;
        if v["cid"].as_str() != Some(cid) || text.is_empty() || djlive::compile::program_cid(text) != cid {
            return None;
        }
        Some(Program { cid: cid.to_string(), text: text.to_string() })
    }

    /// The `meta` of a stored program's file (Go's `Meta` as read back), None for an example, a held link or no file.
    pub fn file_meta(&self, cid: &str) -> Option<Value> {
        if !is_cid(cid) {
            return None;
        }
        let b = std::fs::read(self.dir.join(format!("{cid}.json"))).ok()?;
        let v: Value = serde_json::from_slice(&b).ok()?;
        Some(v.get("meta").cloned().unwrap_or(Value::Null))
    }

    /// Whether `cid` is one of the examples (by its CID, or by its legacy CID).
    pub fn is_example(&self, cid: &str) -> bool {
        self.examples.contains_key(cid)
    }

    /// Every CID an example answers to: each one's own, and the legacy CID of each BeatScript 2 example.
    pub fn example_cids(&self) -> impl Iterator<Item = String> + '_ {
        self.examples.keys().cloned()
    }

    /// For a BeatScript 1 program here (stored or an example), the CID of its BeatScript 2 twin, when the twin is here too
    /// (a stored file that hashes to it, or an example): where `/p/<cid>` redirects. None for a BeatScript 2 program, one
    /// that is not here, or one whose twin is not.
    pub fn renamed_to(&self, cid: &str) -> Option<String> {
        self.twin(cid, djlive::version::Version::V1)
    }

    /// The other way: for a BeatScript 2 program here, its legacy (BeatScript 1) CID when that program is here too.
    pub fn renamed_from(&self, cid: &str) -> Option<String> {
        self.twin(cid, djlive::version::Version::V2)
    }

    fn twin(&self, cid: &str, from: djlive::version::Version) -> Option<String> {
        let p = self.get(cid)?;
        if djlive::version::of_canonical(&p.text) != from {
            return None;
        }
        let other = djlive::compile::renamed_cid(&p.text);
        self.get(&other).map(|_| other)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `programStore.put`'s write: `{"cid","text","meta"}` (meta as given, already in Go's field order) through a
    /// temp file and link(2), so a race with the other server stores once. `text` is the program as sent: the canonical
    /// text is stored, and its `# from:` lines are kept as `meta.based_on` (gate::write_program). Ok(Some(bytes)) when
    /// this call stored it, Ok(None) when the file was already there.
    pub fn put_new(&self, cid: &str, text: &str, meta: Value, encode: impl Fn(&Value) -> String) -> std::io::Result<Option<usize>> {
        let a = admit_as(cid, text, meta["license"].as_str().unwrap_or(""), meta["visibility"].as_str().unwrap_or(""))?;
        if self.readonly {
            let mut l = self.links.lock().unwrap();
            if l.contains_key(cid) {
                return Ok(None);
            }
            if l.len() >= MAX_LINKS {
                l.clear();
            }
            l.insert(cid.to_string(), Program { cid: cid.to_string(), text: a.canonical().to_string() });
            hold_twin(&mut l, cid, a.canonical(), &meta);
            return Ok(Some(0));
        }
        beatscript_serve::gate::write_program(&self.dir, &a, meta, encode)
    }

    /// The stored meta (`api.Meta`: license, credits, author, name, forked_from…) of `cid`, None when it has none
    /// here (an example, a link only kept in memory, no such program). For the export's manifest.
    pub fn meta(&self, cid: &str) -> Option<Value> {
        if !is_cid(cid) {
            return None;
        }
        let b = std::fs::read(self.dir.join(format!("{cid}.json"))).ok()?;
        let v: Value = serde_json::from_slice(&b).ok()?;
        (v["cid"].as_str() == Some(cid)).then(|| v["meta"].clone()).filter(Value::is_object)
    }

    /// Store `canonical` as an anonymous `PUT /api/programs/<cid>` does (first write wins; meta CC0, public, "put").
    pub fn put_canonical(&self, cid: &str, canonical: &str) -> std::io::Result<()> {
        let a = admit_as(cid, canonical, "", "")?;
        if self.readonly {
            let mut l = self.links.lock().unwrap();
            if l.len() >= MAX_LINKS {
                l.clear();
            }
            l.insert(cid.to_string(), Program { cid: cid.to_string(), text: canonical.to_string() });
            hold_twin(&mut l, cid, canonical, &Value::Null);
            return Ok(());
        }
        let path = self.dir.join(format!("{cid}.json"));
        if path.exists() {
            return Ok(());
        }
        let mut meta = serde_json::Map::new();
        meta.insert("license".into(), json!("CC0-1.0"));
        meta.insert("visibility".into(), json!("public"));
        let name = header_name(canonical);
        if !name.is_empty() {
            meta.insert("name".into(), json!(name));
        }
        meta.insert("created".into(), json!(rfc3339_now()));
        meta.insert("source".into(), json!("put"));
        // link(2): a race between the two servers stores once (Go writes with link too)
        beatscript_serve::gate::write_program(&self.dir, &a, Value::Object(meta), go_json).map(|_| ())
    }
}

/// Read-only: a BeatScript 1 program held in memory is held with its BeatScript 2 twin, as the store writes both.
fn hold_twin(l: &mut HashMap<String, Program>, cid: &str, canonical: &str, meta: &Value) {
    if djlive::version::of_canonical(canonical) != djlive::version::Version::V1 {
        return;
    }
    if let Ok(y) = beatscript_serve::gate::admit_twin(cid, canonical, meta) {
        l.insert(y.cid().to_string(), Program { cid: y.cid().to_string(), text: y.canonical().to_string() });
    }
}

/// A stored recipe's expansion (roadmap 6680 item 3, docs/beatscript/ext-template.md "The stored expansion"): what `put_expansion`
/// stored, or found stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    /// Its CID: the recipe's `expanded_cid`.
    pub cid: String,
    /// This call stored it (false: it was there already; first write wins, as for any program).
    pub created: bool,
    /// The bytes it took (0 when it was there, or is only held in memory).
    pub bytes: usize,
}

/// The recipe's meta with what it expands to (`expands_to`) when that is another program: so the store says which program is
/// its expansion without writing it out again.
pub fn with_expands_to(meta: &mut Value, recipe_cid: &str, rep: &Value) {
    if let Some(x) = rep["expanded_cid"].as_str().filter(|x| *x != recipe_cid && is_cid(x)) {
        if let Value::Object(m) = meta {
            m.insert("expands_to".into(), json!(x));
        }
    }
}

impl Store {
    /// Store a recipe's expansion beside it: the program with every reference written out as plain BeatScript (as `check`'s
    /// `expanded_cid` names it: djlive::refs::expanded in the checker's environment), canonical, under its own CID, through the
    /// storage gate. Its meta: the recipe's licence, credits, name and author (the expansion is the same track, and holds the
    /// recipe's own text), `source: "expansion"`, `expanded_from` the recipe, and `based_on` every reference it wrote out with the
    /// CID it is pinned to (beatscript_api::resolution::sources). None for a program with no references.
    pub fn put_expansion(&self, recipe_cid: &str, recipe: &str, recipe_meta: &Value) -> std::io::Result<Option<Expansion>> {
        let env = beatscript_api::web_env().env();
        let Ok(Some((canonical, cid))) = djlive::refs::expanded(recipe, &env) else { return Ok(None) };
        if cid == recipe_cid || !is_cid(&cid) {
            return Ok(None);
        }
        let s = |k: &str| recipe_meta[k].as_str().unwrap_or("").trim().to_string();
        let license = s("license");
        let refs = beatscript_api::resolution::of_with(recipe, recipe_cid, &env);
        let based_on = beatscript_api::resolution::sources(&refs, recipe);
        // (what it was written out from is the store's own provenance, kept by the gate beside the canonical text, as `# from:`
        // lines are for a text as sent)
        let a = admit_as(&cid, &canonical, &license, "")?.with_based_on(based_on.clone());
        let mut meta = serde_json::Map::new();
        meta.insert("license".into(), json!(a.license()));
        meta.insert("visibility".into(), json!("public"));
        for k in ["name"] {
            if !s(k).is_empty() {
                meta.insert(k.into(), json!(s(k)));
            }
        }
        if let Some(c) = recipe_meta["credits"].as_array().filter(|c| !c.is_empty()) {
            meta.insert("credits".into(), Value::Array(c.clone()));
        }
        if !s("author").is_empty() {
            meta.insert("author".into(), json!(s("author")));
        }
        meta.insert("created".into(), json!(rfc3339_now()));
        meta.insert("source".into(), json!("expansion"));
        meta.insert("expanded_from".into(), json!(recipe_cid));
        if !based_on.is_empty() {
            meta.insert("based_on".into(), json!(based_on));
        }
        if self.readonly {
            let mut l = self.links.lock().unwrap();
            if l.contains_key(&cid) {
                return Ok(Some(Expansion { cid, created: false, bytes: 0 }));
            }
            if l.len() >= MAX_LINKS {
                l.clear();
            }
            l.insert(cid.clone(), Program { cid: cid.clone(), text: canonical.clone() });
            hold_twin(&mut l, &cid, &canonical, &Value::Object(meta));
            return Ok(Some(Expansion { cid, created: true, bytes: 0 }));
        }
        let n = beatscript_serve::gate::write_program(&self.dir, &a, Value::Object(meta), go_json)?;
        Ok(Some(Expansion { cid, created: n.is_some(), bytes: n.unwrap_or(0) }))
    }
}

/// The storage gate's verdict for a program a caller has already checked (`text`: as sent, or canonical), as an
/// io::Error (PermissionDenied) when it is refused or is not the CID the caller named.
pub fn admit_as(cid: &str, text: &str, license: &str, visibility: &str) -> std::io::Result<beatscript_serve::gate::Admitted> {
    let denied = |m: String| std::io::Error::new(std::io::ErrorKind::PermissionDenied, m);
    let a = beatscript_serve::gate::admit(text, license, visibility).map_err(|r| denied(r.message))?;
    if a.cid() != cid {
        return Err(denied(format!("that text is program {}, not {cid}", a.cid())));
    }
    Ok(a)
}

pub fn rfc3339_now() -> String {
    // seconds since the epoch to a UTC RFC 3339 stamp (civil-from-days, Howard Hinnant's algorithm)
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "tempo 120\ndeck A: kick x ~ x ~\n";

    fn canonical(text: &str) -> (String, String) {
        let r = check(text);
        (r["cid"].as_str().unwrap().to_string(), r["canonical"].as_str().unwrap().to_string())
    }

    #[test]
    fn examples_answer_under_both_cids_and_btw_files_are_read() {
        let d = tempfile::tempdir().unwrap();
        let ex = d.path().join("examples");
        std::fs::create_dir_all(&ex).unwrap();
        std::fs::write(ex.join("two.beatscript"), "#! beatscript 2\n# a comment\ntempo 128\ndeck A: kick x x x x\n").unwrap();
        std::fs::write(ex.join("one.btw"), "tempo 90\ndeck A: kick x ~ ~ ~\n").unwrap();
        let (st, w) = Store::open(&d.path().join("programs"), Some(&ex), false).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let (new, canon) = canonical("#! beatscript 2\ntempo 128\ndeck A: kick x x x x\n");
        let old = djlive::compile::renamed_cid(&canon);
        assert_eq!(st.get(&new).unwrap().text, canon);
        // the legacy CID answers with the legacy text, which hashes to it
        let legacy = st.get(&old).expect("the legacy CID of a BeatScript 2 example");
        assert_eq!(legacy.text, djlive::version::legacy_canonical(&canon));
        assert_eq!(canonical(&legacy.text).0, old, "the legacy text checks as the legacy CID");
        assert!(st.is_example(&old) && st.is_example(&new));
        assert_eq!(st.renamed_to(&old).as_deref(), Some(new.as_str()));
        assert_eq!(st.renamed_from(&new).as_deref(), Some(old.as_str()));
        assert_eq!(st.renamed_to(&new), None);
        // a `.btw` example is read; a BeatScript 1 example has no twin here
        let (one, _) = canonical("tempo 90\ndeck A: kick x ~ ~ ~\n");
        assert!(st.get(&one).is_some());
        assert_eq!(st.renamed_to(&one), None);
        assert_eq!(st.example_cids().count(), 3);
    }

    #[test]
    fn a_stored_program_is_paired_with_its_twin_and_a_tampered_twin_is_not_one() {
        let d = tempfile::tempdir().unwrap();
        let (st, _) = Store::open(&d.path().join("programs"), None, false).unwrap();
        let (x, xt) = canonical(V1);
        assert_eq!(st.renamed_to(&x), None, "not stored");
        st.put_canonical(&x, &xt).unwrap();
        let y = djlive::compile::renamed_cid(&xt);
        assert_eq!(st.renamed_to(&x).as_deref(), Some(y.as_str()), "stored with its twin");
        assert_eq!(st.renamed_from(&y).as_deref(), Some(x.as_str()));
        assert_eq!(st.file_meta(&y).unwrap()["derivation"], "renamed");
        // a twin file whose text is not its CID's is not a program: no redirect to it
        let path = st.dir().join(format!("{y}.json"));
        let mut f: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        f["text"] = json!(format!("{}deck B: snare ~ x ~ x\n", f["text"].as_str().unwrap()));
        std::fs::write(&path, f.to_string()).unwrap();
        assert!(st.get(&y).is_none());
        assert_eq!(st.renamed_to(&x), None);
        assert!(st.get(&x).is_some(), "the old program still answers");
    }

    #[test]
    fn read_only_holds_the_twin_in_memory() {
        let d = tempfile::tempdir().unwrap();
        let (st, _) = Store::open(&d.path().join("programs"), None, true).unwrap();
        let (x, xt) = canonical(V1);
        st.put_canonical(&x, &xt).unwrap();
        let y = djlive::compile::renamed_cid(&xt);
        assert_eq!(st.renamed_to(&x).as_deref(), Some(y.as_str()));
        assert!(!d.path().join("programs").exists(), "nothing written");
    }
}
