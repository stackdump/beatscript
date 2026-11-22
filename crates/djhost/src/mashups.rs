//! Where mashups are kept (docs/MASHUPS.md): `<support>/mashups/<cid>.json` (`$DJ_MASHUPS_DIR`),
//! canonical JSON named by its own content ID, as set lists are. Saving the same mashup twice is
//! one file; an edit is a new mashup (and its `forkedFrom` keeps the line).

use std::path::PathBuf;

use djengine::json::JsonValue;
use djengine::mashup::Mashup;

use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

pub struct MashupStore {
    pub directory: PathBuf,
}

/// One saved mashup, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cid: String,
    pub name: String,
    pub parts: usize,
    pub sources: usize,
    pub bpm: f64,
}

impl MashupStore {
    pub fn new(directory: impl Into<PathBuf>) -> MashupStore {
        MashupStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_MASHUPS_DIR", "mashups")
    }

    pub fn open_default() -> MashupStore {
        MashupStore::new(MashupStore::default_directory())
    }

    fn path(&self, cid: &str) -> PathBuf {
        self.directory.join(format!("{cid}.json"))
    }

    /// Save under its CID; answers the CID.
    pub fn save(&self, m: &Mashup) -> Result<String, String> {
        let cid = m.cid();
        write_atomic(&self.path(&cid), pretty_sorted(&m.to_json()).as_bytes()).map_err(|e| format!("mashup: {e}"))?;
        Ok(cid)
    }

    pub fn get(&self, cid: &str) -> Option<Mashup> {
        let v: JsonValue = read_json(&self.path(cid))?;
        Mashup::from_json(&v).ok()
    }

    /// Every saved mashup whose file name is its CID, by name.
    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| {
                let m = self.get(&cid)?;
                (m.cid() == cid).then(|| Entry { cid, name: m.name.clone(), parts: m.parts.len(), sources: m.sources.len(), bpm: m.bpm })
            })
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.cid.cmp(&b.cid)));
        v
    }

    /// A whole CID, its head or tail, or a name (exact, then a part). One match or why not.
    pub fn resolve(&self, query: &str) -> Result<Entry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("mashup: which one? (a cid or a name; `mashup list`)".into());
        }
        let all = self.list();
        let lq = q.to_lowercase();
        let tiers: [&dyn Fn(&Entry) -> bool; 4] = [
            &|e| e.cid == q,
            &|e| e.cid.starts_with(q) || e.cid.ends_with(q),
            &|e| e.name.to_lowercase() == lq,
            &|e| e.name.to_lowercase().contains(&lq),
        ];
        for t in tiers {
            let hits: Vec<&Entry> = all.iter().filter(|e| t(e)).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                n => return Err(format!("mashup: {q:?} matches {n} mashups; give more of the cid")),
            }
        }
        Err(format!("mashup: no saved mashup {q:?}"))
    }

    pub fn delete(&self, cid: &str) -> Result<(), String> {
        std::fs::remove_file(self.path(cid)).map_err(|e| format!("mashup: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::mashup::Source;

    #[test]
    fn saves_lists_resolves_and_deletes_by_cid() {
        let dir = std::env::temp_dir().join(format!("djmashups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = MashupStore::new(&dir);
        let mut m = Mashup::new("Night Drive");
        m.add_source(Source::parse("A", "techno:7").unwrap()).unwrap();
        m.assign("drums", "A", "drums").unwrap();
        let cid = store.save(&m).unwrap();
        assert_eq!(cid, m.cid());
        assert_eq!(store.save(&m).unwrap(), cid, "saving again is the same file");
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.resolve("night").unwrap().cid, cid);
        assert_eq!(store.resolve(&cid[cid.len() - 6..]).unwrap().cid, cid);
        assert_eq!(store.get(&cid).unwrap().cid(), cid);
        store.delete(&cid).unwrap();
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
