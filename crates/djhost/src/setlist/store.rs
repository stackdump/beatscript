//! Where sets are kept: `<support>/setlists/<cid>.json` (`$DJ_SETLISTS_DIR`), canonical JSON,
//! named by its own content ID. Saving the same set twice is one file; an edit is a new set, so
//! [`SetStore::replace`] writes the new file and removes the old one.

use std::path::PathBuf;

use djengine::json::JsonValue;

use super::SetList;
use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

pub struct SetStore {
    pub directory: PathBuf,
}

/// One saved set, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cid: String,
    pub name: String,
    pub segments: usize,
    pub bars: u32,
    /// The license, visibility and lineage (CC0 / public for a set saved before sets had them).
    pub rights: djengine::rights::Rights,
}

impl SetStore {
    pub fn new(directory: impl Into<PathBuf>) -> SetStore {
        SetStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_SETLISTS_DIR", "setlists")
    }

    pub fn open_default() -> SetStore {
        SetStore::new(SetStore::default_directory())
    }

    fn path(&self, cid: &str) -> PathBuf {
        self.directory.join(format!("{cid}.json"))
    }

    /// Save `set` under its CID; answers the CID. Saving what is already there is not an error.
    pub fn save(&self, set: &SetList) -> Result<String, String> {
        let cid = set.cid();
        write_atomic(&self.path(&cid), pretty_sorted(&set.to_json()).as_bytes()).map_err(|e| format!("setlist: {e}"))?;
        Ok(cid)
    }

    fn read(&self, cid: &str) -> Option<SetList> {
        let v: JsonValue = read_json(&self.path(cid))?;
        SetList::from_json(&v).ok()
    }

    /// Every saved set, by name then CID. A file that is not a set (or whose name is not its
    /// CID: someone edited it) is left out.
    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| {
                let s = self.read(&cid)?;
                (s.cid() == cid).then(|| Entry { cid, name: s.name.clone(), segments: s.segments.len(), bars: s.total_bars(), rights: s.rights() })
            })
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.cid.cmp(&b.cid)));
        v
    }

    /// Which set `query` means: a whole CID, a part of one (its tail or head, as the Library
    /// takes it), or a name (exact first, then a part). One match, or an error saying why not.
    pub fn resolve(&self, query: &str) -> Result<Entry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("setlist: which one? (a cid, or a name; `setlist list`)".into());
        }
        let all = self.list();
        let lq = q.to_lowercase();
        let tiers: [Box<dyn Fn(&Entry) -> bool>; 4] = [
            Box::new(|e| e.cid == q),
            Box::new(|e| e.name.to_lowercase() == lq),
            Box::new(|e| e.cid.starts_with(q) || e.cid.ends_with(q)),
            Box::new(|e| e.name.to_lowercase().contains(&lq)),
        ];
        for t in &tiers {
            let hits: Vec<&Entry> = all.iter().filter(|e| t(e)).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                _ => {
                    let names: Vec<String> = hits.iter().take(6).map(|e| format!("{} ({})", e.name, &e.cid[..e.cid.len().min(10)])).collect();
                    return Err(format!("setlist {q:?} matches {}: {}", hits.len(), names.join(", ")));
                }
            }
        }
        Err(format!("no saved set matches {q:?} (`setlist list`)"))
    }

    pub fn load(&self, query: &str) -> Result<SetList, String> {
        let e = self.resolve(query)?;
        self.read(&e.cid).ok_or_else(|| format!("setlist {}: unreadable", e.cid))
    }

    pub fn delete(&self, query: &str) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        std::fs::remove_file(self.path(&e.cid)).map_err(|err| format!("setlist: {err}"))?;
        Ok(e)
    }

    /// Save `new` and drop the set `old_cid` it was edited from (when it is a different one).
    pub fn replace(&self, old_cid: &str, new: &SetList) -> Result<String, String> {
        let cid = self.save(new)?;
        if cid != old_cid {
            let _ = std::fs::remove_file(self.path(old_cid));
        }
        Ok(cid)
    }

    /// A saved copy of `query` under `name`.
    pub fn duplicate(&self, query: &str, name: Option<&str>) -> Result<(SetList, String), String> {
        let s = self.load(query)?;
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => format!("{} copy", s.name),
        };
        let copy = s.duplicate(&name);
        let cid = self.save(&copy)?;
        Ok((copy, cid))
    }
}
