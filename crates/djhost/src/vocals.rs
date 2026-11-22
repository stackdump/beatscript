//! Saved vocal lines (docs/VOICE.md): lyrics sung by the formant voice, kept as one canonical,
//! content-addressed JSON object each (`djengine::vocal::VocalLine`), with its own license
//! (CC0 by default), visibility, author and lineage. [`VocalStore`] keeps them as `<cid>.json` in
//! `<support>/vocals` (`$DJ_VOCALS_DIR`), as [`crate::tracks::TrackStore`] keeps tracks: a file
//! whose name is not its content's CID is not listed, and an edit (a new license, a rename)
//! writes the new object and drops the old file.

use std::path::{Path, PathBuf};

use djengine::rights::{short_cid, License, Visibility};
use djengine::vocal::VocalLine;

use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

// ---- the voices a line can sing in (what `sing ... voice=NAME` and `vocal create preset=NAME` are checked against)

/// The names a neural singing voice answers to (`sing "..." on melody voice=alto`, `vocal create preset=alto`): the four parts of a
/// choir that are not the formant `soprano`. **There is no neural voice engine in this build**: docs/VOICE.md lists Kokoro as
/// "not built", and the `neural` feature of this crate is the beat detector's (docs/BEAT-DETECTION.md), not a singer. So none of
/// these can be sung yet: a line that names one is told so at check (`djlive::sing::FALLBACK`) and plays on [`FALLBACK_VOICE`].
pub const NEURAL_VOICES: [&str; 4] = ["alto", "tenor", "baritone", "bass"];

/// The formant preset a neural voice falls back to when the neural voice is not installed.
pub const FALLBACK_VOICE: &str = djengine::vocal::DEFAULT_PRESET;

/// A neural singing voice is built into this binary. It is not (there is none to build): the one switch to flip when an engine
/// lands, together with the models' files in [`neural_models_dir`].
pub const NEURAL_ENGINE_BUILT: bool = false;

/// `$DJRUST_VOICE_MODELS`, else `<support>/models/voice`: where a neural voice's model files would live.
pub fn neural_models_dir() -> PathBuf {
    env_or_support("DJRUST_VOICE_MODELS", "models/voice")
}

/// The neural voice can sing here: an engine is built and its models are in `dir` (any file in it; the engine checks them
/// against its pins when it loads them).
pub fn neural_installed_in(engine_built: bool, dir: &Path) -> bool {
    engine_built && std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

/// [`neural_installed_in`] for this build and this machine.
pub fn neural_installed() -> bool {
    neural_installed_in(NEURAL_ENGINE_BUILT, &neural_models_dir())
}

/// Every voice a line may name: the formant presets, then the neural voices.
pub fn voice_names() -> Vec<String> {
    djengine::fx::formant::PRESETS.iter().map(|p| p.name.to_string()).chain(NEURAL_VOICES.iter().map(|n| n.to_string())).collect()
}

pub fn is_neural_voice(name: &str) -> bool {
    NEURAL_VOICES.contains(&name)
}

/// The voice a line with none named sings in: the neural one when it is installed (`alto`), else the formant default.
pub fn default_voice(neural_installed: bool) -> &'static str {
    if neural_installed {
        NEURAL_VOICES[0]
    } else {
        FALLBACK_VOICE
    }
}

/// One saved line, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cid: String,
    pub name: String,
    pub lyrics: String,
    pub key: String,
    pub bpm: f64,
    pub bars: u32,
    pub license: License,
    pub visibility: Visibility,
}

pub struct VocalStore {
    pub directory: PathBuf,
}

impl VocalStore {
    pub fn new(directory: impl Into<PathBuf>) -> VocalStore {
        VocalStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_VOCALS_DIR", "vocals")
    }

    pub fn open_default() -> VocalStore {
        VocalStore::new(VocalStore::default_directory())
    }

    fn path(&self, cid: &str) -> PathBuf {
        self.directory.join(format!("{cid}.json"))
    }

    pub fn save(&self, v: &VocalLine) -> Result<String, String> {
        let cid = v.cid();
        write_atomic(&self.path(&cid), pretty_sorted(&v.to_json()).as_bytes()).map_err(|e| format!("vocal: {e}"))?;
        Ok(cid)
    }

    fn read(&self, cid: &str) -> Option<VocalLine> {
        VocalLine::from_json(&read_json(&self.path(cid))?).ok()
    }

    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| {
                let l = self.read(&cid)?;
                (l.cid() == cid).then(|| Entry {
                    cid,
                    name: l.name.clone(),
                    lyrics: l.lyrics.clone(),
                    key: l.key.name(),
                    bpm: l.bpm,
                    bars: l.bars,
                    license: l.rights.license,
                    visibility: l.rights.visibility,
                })
            })
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.cid.cmp(&b.cid)));
        v
    }

    /// Which line `query` means: a whole CID, its head or tail, or a name (exact, then a part).
    pub fn resolve(&self, query: &str) -> Result<Entry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("vocal: which one? (a cid, or a name; `vocal list`)".into());
        }
        let all = self.list();
        let lq = q.to_lowercase();
        let tiers: [Box<dyn Fn(&Entry) -> bool>; 4] = [
            Box::new(|e| e.cid == q),
            Box::new(|e| e.name.to_lowercase() == lq),
            Box::new(|e| e.cid.starts_with(q) || e.cid.ends_with(q)),
            Box::new(|e| e.name.to_lowercase().contains(&lq) || e.lyrics.to_lowercase().contains(&lq)),
        ];
        for t in &tiers {
            let hits: Vec<&Entry> = all.iter().filter(|e| t(e)).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                _ => {
                    let names: Vec<String> = hits.iter().take(6).map(|e| format!("{} ({})", e.name, short_cid(&e.cid))).collect();
                    return Err(format!("vocal {q:?} matches {}: {}", hits.len(), names.join(", ")));
                }
            }
        }
        Err(format!("no saved vocal line matches {q:?} (`vocal list`)"))
    }

    pub fn load(&self, query: &str) -> Result<VocalLine, String> {
        let e = self.resolve(query)?;
        self.read(&e.cid).ok_or_else(|| format!("vocal {}: unreadable", e.cid))
    }

    pub fn delete(&self, query: &str) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        std::fs::remove_file(self.path(&e.cid)).map_err(|err| format!("vocal: {err}"))?;
        Ok(e)
    }

    /// Save `new` and drop `old_cid` (an edit of a saved line: a new license, a new name).
    pub fn replace(&self, old_cid: &str, new: &VocalLine) -> Result<String, String> {
        let cid = self.save(new)?;
        if cid != old_cid {
            let _ = std::fs::remove_file(self.path(old_cid));
        }
        Ok(cid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::key::Key;
    use djengine::vocal::Params;

    fn store() -> VocalStore {
        let d = crate::stores::temp_path("vocals-test");
        let _ = std::fs::remove_dir_all(&d);
        VocalStore::new(d)
    }

    fn line(lyrics: &str) -> VocalLine {
        VocalLine::create(Params::new(lyrics, Key::parse("A minor").unwrap(), 120.0, 2)).unwrap().0
    }

    #[test]
    fn the_neural_voice_is_installed_only_with_an_engine_and_its_models() {
        let d = crate::stores::temp_path("vocals-neural");
        let _ = std::fs::remove_dir_all(&d);
        // no folder, an empty one, one with a file: and an engine or not
        assert!(!neural_installed_in(true, &d));
        std::fs::create_dir_all(&d).unwrap();
        assert!(!neural_installed_in(true, &d), "an empty folder holds no models");
        std::fs::write(d.join("voice.onnx"), b"x").unwrap();
        assert!(neural_installed_in(true, &d));
        assert!(!neural_installed_in(false, &d), "models with no engine to sing them are not a voice");
        let _ = std::fs::remove_dir_all(&d);
        // this build has no engine: the neural voice is not installed, whatever is on the disk
        assert!(!NEURAL_ENGINE_BUILT && !neural_installed());
    }

    #[test]
    fn the_voice_names_are_the_formant_presets_then_the_neural_voices() {
        let v = voice_names();
        assert_eq!(v.len(), djengine::fx::formant::PRESETS.len() + NEURAL_VOICES.len());
        for p in djengine::fx::formant::PRESETS.iter() {
            assert!(v.iter().any(|n| n == p.name) && !is_neural_voice(p.name), "{}", p.name);
        }
        assert!(NEURAL_VOICES.iter().all(|n| v.contains(&n.to_string()) && is_neural_voice(n)));
        // a name is one or the other, and the fallback is a formant voice the vocal noun accepts
        assert!(!NEURAL_VOICES.iter().any(|n| djengine::fx::formant::preset(n).is_some()), "a neural name must not shadow a formant preset");
        assert!(djengine::fx::formant::preset(FALLBACK_VOICE).is_some());
        assert_eq!((default_voice(false), default_voice(true)), (FALLBACK_VOICE, "alto"));
    }

    #[test]
    fn lines_save_by_cid_list_resolve_edit_and_delete() {
        let s = store();
        assert!(s.list().is_empty());
        let a = line("buy the macs");
        let cid = s.save(&a).unwrap();
        assert_eq!(cid, a.cid());
        assert_eq!(s.save(&a).unwrap(), cid, "saving twice is one entry");
        let b = line("six million lines of proof");
        s.save(&b).unwrap();
        assert_eq!(s.list().len(), 2);
        // by cid tail, by name, by a word of the lyrics
        assert_eq!(s.resolve(&cid[cid.len() - 8..]).unwrap().cid, cid);
        assert_eq!(s.load("buy the macs").unwrap(), a);
        assert_eq!(s.resolve("million").unwrap().name, b.name);
        assert!(s.resolve("nope").is_err());
        // a new license is a new CID, and the old file goes
        let mut c = a.clone();
        c.rights.license = License::CcBy4;
        let ncid = s.replace(&cid, &c).unwrap();
        assert_ne!(ncid, cid);
        assert_eq!(s.list().len(), 2);
        assert!(s.resolve(&cid).is_err());
        assert_eq!(s.delete(&ncid).unwrap().license, License::CcBy4);
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_dir_all(&s.directory);
    }
}
