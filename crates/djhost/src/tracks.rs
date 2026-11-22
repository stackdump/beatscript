//! Saved tracks (docs/LICENSING.md): a track a person made, kept as one canonical,
//! content-addressed JSON object that carries its own license, visibility, author and lineage.
//!
//! ```text
//! {"format":1,"type":"djrust.track","name":"Night Bus",
//!  "spec":{…the deck's share payload: genre, seed, traits, layers…},
//!  "license":"CC0-1.0","visibility":"public","author":"did:key:z6Mk…",
//!  "forkedFrom":"zQm…","credits":[{"cid":"zQm…","author":"…","license":"CC-BY-4.0","name":"…"}]}
//! ```
//!
//! The music is `spec`: the same share payload a deck plays and a share link carries, so its
//! own share CID ([`Track::music_cid`]) is unchanged by any of this (Swift parity). The track's
//! CID ([`Track::cid`]) is of the whole object (sorted keys, JS numbers; the shares' `z…` CID),
//! so the license is inside what is addressed: a new license is a new CID, and a fork made from
//! it carries the license and the lineage forward ([`Track::fork`]).
//!
//! [`TrackStore`] keeps them as `<cid>.json` in `<support>/tracks` (`$DJ_TRACKS_DIR`), like the
//! set lists: a file whose name is not its content's CID is not listed, and an edit (a new
//! license, a rename) writes the new object and drops the old file ([`TrackStore::replace`]).
//! [`Track::share`] is the one way out by CID, and it refuses a private track.

use std::path::PathBuf;

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{short_cid, Fork, License, Rights, Visibility};
use djengine::spec::TrackSpec;

use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

/// The file format's version.
pub const FORMAT: i64 = 1;
/// The object's `type`.
pub const TYPE: &str = "djrust.track";

/// A saved track.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    /// The share payload (`TrackSpec::payload`): what plays.
    pub spec: JsonValue,
    pub rights: Rights,
}

impl Track {
    /// A new track of `spec`, CC0 and public unless the caller says otherwise.
    pub fn new(name: &str, spec: &TrackSpec, rights: Rights) -> Track {
        Track { name: name.trim().to_string(), spec: normalized(&spec.payload()), rights }
    }

    /// The canonical JSON the CID is of.
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("format", JsonValue::int(FORMAT));
        o.set("type", TYPE.into());
        o.set("name", self.name.clone().into());
        o.set("spec", self.spec.clone());
        self.rights.write_into(&mut o);
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Track, String> {
        if v.get("type").and_then(JsonValue::as_str) != Some(TYPE) {
            return Err(format!("not a track (type {TYPE})"));
        }
        let name = v.get("name").and_then(JsonValue::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("track: a name")?;
        let spec = v.get("spec").filter(|s| s.as_object().is_some()).ok_or("track: a spec (a share payload)")?;
        TrackSpec::from_payload(spec).ok_or("track: the spec needs a genre and a seed")?;
        let rights = Rights::from_json(v).map_err(|e| format!("track: {e}"))?;
        Ok(Track { name: name.to_string(), spec: normalized(spec), rights })
    }

    pub fn canonical(&self) -> String {
        self.to_json().canonical_js()
    }

    /// The track's content ID: of the whole object, rights included.
    pub fn cid(&self) -> String {
        djengine::cid::share_cid_canonical(&self.canonical())
    }

    /// The music's own share CID (the payload's, as a share link names it): rights do not move it.
    pub fn music_cid(&self) -> String {
        djengine::cid::share_cid(&self.spec)
    }

    /// The deck spec to play it.
    pub fn track_spec(&self) -> Option<TrackSpec> {
        TrackSpec::from_payload(&self.spec)
    }

    /// A remix of this track by `author`, called `name` (default "<name> (remix)"), with the
    /// lineage and credits [`Rights::fork`] gives and its warnings (an all-rights-reserved parent).
    pub fn fork(&self, name: Option<&str>, author: Option<&str>) -> (Track, Vec<String>) {
        let Fork { rights, warnings } = self.rights.fork(&self.cid(), Some(&self.name), author);
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| format!("{} (remix)", self.name), str::to_string);
        (Track { name, spec: self.spec.clone(), rights }, warnings)
    }

    /// What may leave the machine by CID: the object itself, its CID and the music's. Private:
    /// refused ([`Rights::check_shareable`]).
    pub fn share(&self) -> Result<JsonValue, String> {
        self.rights.check_shareable().map_err(|e| format!("track {}: {e}", short_cid(&self.cid())))?;
        let mut o = JsonObject::new();
        o.set("cid", self.cid().into());
        o.set("music_cid", self.music_cid().into());
        o.set("object", self.to_json());
        if !self.rights.credits.is_empty() {
            o.set("credit", self.rights.credit_line().into());
        }
        Ok(JsonValue::Object(o))
    }

    /// For answers: the object's fields plus its CIDs, the rights in words and the credit line.
    pub fn summary_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("cid", self.cid().into());
        o.set("music_cid", self.music_cid().into());
        o.set("name", self.name.clone().into());
        if let Some(g) = self.spec.get("genre") {
            o.set("genre", g.clone());
        }
        if let Some(s) = self.spec.get("seed") {
            o.set("seed", s.clone());
        }
        o.set("rights", self.rights.to_json());
        o.set("text", self.rights.text().into());
        JsonValue::Object(o)
    }
}

/// The payload as its canonical text reads back, so a track built in memory and the same track
/// read from its file compare equal (a whole-number seed is one number either way).
fn normalized(v: &JsonValue) -> JsonValue {
    JsonValue::parse(&v.canonical_js()).unwrap_or_else(|_| v.clone())
}

/// One saved track, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cid: String,
    pub name: String,
    /// The music's share CID (what a deck playing it has).
    pub music_cid: String,
    pub rights: Rights,
}

/// Where tracks are kept.
pub struct TrackStore {
    pub directory: PathBuf,
}

impl TrackStore {
    pub fn new(directory: impl Into<PathBuf>) -> TrackStore {
        TrackStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_TRACKS_DIR", "tracks")
    }

    pub fn open_default() -> TrackStore {
        TrackStore::new(TrackStore::default_directory())
    }

    fn path(&self, cid: &str) -> PathBuf {
        self.directory.join(format!("{cid}.json"))
    }

    /// Save `t` under its CID; answers the CID.
    pub fn save(&self, t: &Track) -> Result<String, String> {
        let cid = t.cid();
        write_atomic(&self.path(&cid), pretty_sorted(&t.to_json()).as_bytes()).map_err(|e| format!("track: {e}"))?;
        Ok(cid)
    }

    fn read(&self, cid: &str) -> Option<Track> {
        Track::from_json(&read_json(&self.path(cid))?).ok()
    }

    /// Every saved track, by name then CID (a file whose name is not its CID is left out).
    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| {
                let t = self.read(&cid)?;
                (t.cid() == cid).then(|| Entry { cid, music_cid: t.music_cid(), name: t.name, rights: t.rights })
            })
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.cid.cmp(&b.cid)));
        v
    }

    /// Which track `query` means: a whole CID, its head or tail, or a name (exact, then a part).
    pub fn resolve(&self, query: &str) -> Result<Entry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("track: which one? (a cid, or a name; `track list`)".into());
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
                    let names: Vec<String> = hits.iter().take(6).map(|e| format!("{} ({})", e.name, short_cid(&e.cid))).collect();
                    return Err(format!("track {q:?} matches {}: {}", hits.len(), names.join(", ")));
                }
            }
        }
        Err(format!("no saved track matches {q:?} (`track list`)"))
    }

    pub fn load(&self, query: &str) -> Result<Track, String> {
        let e = self.resolve(query)?;
        self.read(&e.cid).ok_or_else(|| format!("track {}: unreadable", e.cid))
    }

    pub fn delete(&self, query: &str) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        std::fs::remove_file(self.path(&e.cid)).map_err(|err| format!("track: {err}"))?;
        Ok(e)
    }

    /// Save `new` and drop `old_cid` (an edit of a saved track: a new license, a new name).
    pub fn replace(&self, old_cid: &str, new: &Track) -> Result<String, String> {
        let cid = self.save(new)?;
        if cid != old_cid {
            let _ = std::fs::remove_file(self.path(old_cid));
        }
        Ok(cid)
    }

    /// Change a saved track's license and / or visibility: answers the track after (a new CID).
    pub fn set_rights(&self, query: &str, license: Option<License>, visibility: Option<Visibility>) -> Result<Track, String> {
        let e = self.resolve(query)?;
        let mut t = self.read(&e.cid).ok_or_else(|| format!("track {}: unreadable", e.cid))?;
        if let Some(l) = license {
            t.rights.license = l;
        }
        if let Some(v) = visibility {
            t.rights.visibility = v;
        }
        self.replace(&e.cid, &t)?;
        Ok(t)
    }

    /// Fork a saved track and save the fork: answers it and the warnings.
    pub fn fork(&self, query: &str, name: Option<&str>, author: Option<&str>) -> Result<(Track, Vec<String>), String> {
        let parent = self.load(query)?;
        let (child, warnings) = parent.fork(name, author);
        self.save(&child)?;
        Ok((child, warnings))
    }
}

/// The author new objects get when a caller names none: `$DJ_AUTHOR` (a signer identity, such as
/// the wallet's `did:key:z…`), else nobody.
pub fn default_author() -> Option<String> {
    std::env::var("DJ_AUTHOR").ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
}

#[cfg(test)]
mod tests;
