//! The programs store, read and written in exactly the Go server's format (`<data>/programs/<cid>.json`,
//! `{"cid","text","meta"}`), so the Go and Rust servers can share one data directory while both run.
//! Examples answer first, as in Go (`Backend.Get`).
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
                        if p.extension().and_then(|x| x.to_str()) != Some("beatscript") {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(&p) else { continue };
                        let rep = check(&text);
                        let ok = rep["ok"] == true && rep["restricted"] != true && rep["uses_assets"] != true && !declares_not_public(&text);
                        match (ok, rep["cid"].as_str(), rep["canonical"].as_str()) {
                            (true, Some(cid), Some(canon)) => {
                                ex.insert(cid.to_string(), Program { cid: cid.to_string(), text: canon.to_string() });
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

    /// Whether `cid` is one of the examples.
    pub fn is_example(&self, cid: &str) -> bool {
        self.examples.contains_key(cid)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `programStore.put`'s write: `{"cid","text","meta"}` (meta as given, already in Go's field order) through a
    /// temp file and link(2), so a race with the other server stores once. Ok(Some(bytes)) when this call stored it,
    /// Ok(None) when the file was already there.
    pub fn put_new(&self, cid: &str, canonical: &str, meta: Value, encode: impl Fn(&Value) -> String) -> std::io::Result<Option<usize>> {
        let a = admit_as(cid, canonical, meta["license"].as_str().unwrap_or(""), meta["visibility"].as_str().unwrap_or(""))?;
        if self.readonly {
            let mut l = self.links.lock().unwrap();
            if l.contains_key(cid) {
                return Ok(None);
            }
            if l.len() >= MAX_LINKS {
                l.clear();
            }
            l.insert(cid.to_string(), Program { cid: cid.to_string(), text: canonical.to_string() });
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

/// The storage gate's verdict for a program a caller has already checked, as an io::Error (PermissionDenied) when
/// it is refused or is not the CID the caller named.
pub fn admit_as(cid: &str, canonical: &str, license: &str, visibility: &str) -> std::io::Result<beatscript_serve::gate::Admitted> {
    let denied = |m: String| std::io::Error::new(std::io::ErrorKind::PermissionDenied, m);
    let a = beatscript_serve::gate::admit(canonical, license, visibility).map_err(|r| denied(r.message))?;
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
