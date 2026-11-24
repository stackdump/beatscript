//! Grids cached by the content hash of their audio and the analyzer that found them:
//! `<dir>/<content key>-<analyzer hash>.json`. The same file or loop analysed twice (or under
//! another name) by the same analyzer is read, not computed. The same audio under two analyzers
//! (the DSP detector and Beat This!, or Beat This! with another model) is two entries, and a
//! result is only ever answered for the analyzer that made it ([`super::analyzers`]). A file from
//! before results named their analyzer (schema `djrust/beatgrid/v1`) is never read: that audio is
//! analysed again.

use std::path::{Path, PathBuf};

use djengine::beatgrid::BeatGrid;
use djengine::json::{JsonObject, JsonValue};

use super::{analyzers, grid_from_json, grid_to_json, Analysis};

/// v2: every entry names its analyzer, and its file name has it (v1 had neither).
pub const SCHEMA: &str = "djrust/beatgrid/v2";

/// What a cache entry holds.
#[derive(Clone, Debug)]
pub struct Cached {
    pub grid: BeatGrid,
    /// The analyzer that found it ([`super::analyzers`]).
    pub analyzer: String,
    /// `neural` or `dsp`.
    pub method: String,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct Cache {
    pub dir: PathBuf,
}

/// FNV-1a over the analyzer id: the short tail of a file name (the file itself holds the id).
fn analyzer_hash(analyzer: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in analyzer.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Cache {
        Cache { dir: dir.into() }
    }

    /// `$DJRUST_ANALYSIS`, else `<support>/analysis`.
    pub fn open_default() -> Cache {
        Cache::new(crate::stores::env_or_support("DJRUST_ANALYSIS", "analysis"))
    }

    /// The file a result of `analyzer` for the audio `key` is kept in.
    fn path(&self, key: &str, analyzer: &str) -> PathBuf {
        // a key is hex we made, but this is a path: keep it a file name
        let safe: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        self.dir.join(format!("{safe}-{:016x}.json", analyzer_hash(analyzer)))
    }

    /// The grid `analyzer` found for the audio `key`, when one is kept. An entry of another
    /// analyzer, of other audio, or of an older schema is not read.
    pub fn get(&self, key: &str, analyzer: &str) -> Option<Cached> {
        let text = std::fs::read_to_string(self.path(key, analyzer)).ok()?;
        let v = JsonValue::parse(&text).ok()?;
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str);
        if s("schema") != Some(SCHEMA) || s("analyzer") != Some(analyzer) || s("key") != Some(key) {
            return None;
        }
        Some(Cached {
            grid: grid_from_json(v.get("grid")?)?,
            analyzer: analyzer.to_string(),
            method: s("method")?.to_string(),
            duration: v.get("duration").and_then(JsonValue::as_f64).unwrap_or(0.0),
        })
    }

    /// Keep what an analysis found: its grid under its analyzer, and, when that is another
    /// analyzer's (Beat This!), its DSP grid under `beatgrid@1` too, so turning Beat This! off
    /// later is answered at once. Answers whether the grid was kept; quietly does nothing for no
    /// grid, a grid no detector found (a known tempo) or an unwritable folder.
    pub fn put(&self, key: &str, a: &Analysis) -> bool {
        let Some(g) = &a.grid else { return false };
        if a.analyzer.is_empty() {
            return false;
        }
        if a.analyzer != analyzers::BEATGRID_DSP {
            if let Some(d) = &a.dsp {
                self.put_grid(key, analyzers::BEATGRID_DSP, "dsp", d, a.duration);
            }
        }
        self.put_grid(key, &a.analyzer, a.method, g, a.duration)
    }

    /// Keep one grid of `analyzer` for the audio `key`.
    pub fn put_grid(&self, key: &str, analyzer: &str, method: &str, grid: &BeatGrid, duration: f64) -> bool {
        let mut o = JsonObject::new();
        o.set("schema", JsonValue::str(SCHEMA));
        o.set("key", JsonValue::str(key));
        o.set("analyzer", JsonValue::str(analyzer));
        o.set("method", JsonValue::str(method));
        o.set("duration", JsonValue::Number(duration));
        o.set("grid", grid_to_json(grid, true));
        crate::stores::write_atomic(&self.path(key, analyzer), JsonValue::Object(o).canonical_js().as_bytes()).is_ok()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many grids are kept (files of this schema's naming; a v1 file, `<key>.json`, is not one).
    pub fn len(&self) -> usize {
        std::fs::read_dir(&self.dir)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.file_name().to_string_lossy().contains('-'))
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{click_track, Oracle};
    use super::super::{analyze, content_key, models, Options};
    use super::*;

    const SR: f64 = 44_100.0;

    fn tmp(name: &str) -> Cache {
        let d = std::env::temp_dir().join(format!("djcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Cache::new(d)
    }

    #[test]
    fn the_same_samples_under_two_analyzers_are_two_entries() {
        let c = tmp("two");
        let audio = click_track(124.0, 0.30, 16.0, SR);
        let key = content_key(&audio, SR);
        let a = analyze(&audio, SR, Some(&mut Oracle { bpm: 124.0, first: 0.30, jitter: 0.0, calls: 0 }), "", Options::default());
        assert_eq!(a.analyzer, "oracle@1");
        assert!(c.put(&key, &a));
        assert_eq!(c.len(), 2, "the backend's grid and the DSP grid, each under its analyzer");
        let n = c.get(&key, "oracle@1").unwrap();
        assert_eq!((n.analyzer.as_str(), n.method.as_str()), ("oracle@1", "neural"));
        let d = c.get(&key, analyzers::BEATGRID_DSP).unwrap();
        assert_eq!((d.analyzer.as_str(), d.method.as_str()), (analyzers::BEATGRID_DSP, "dsp"));
        assert_eq!(d.grid.source, djengine::beatgrid::GridSource::Dsp);
        // an analyzer that made nothing here finds nothing, and so does other audio
        assert!(c.get(&key, "oracle@2").is_none());
        assert!(c.get(&content_key(&audio, 48_000.0), "oracle@1").is_none());
        // a DSP-only analysis is one entry; a grid no detector found is none
        let c2 = tmp("dsp-only");
        let dsp = analyze(&audio, SR, None, "", Options::default());
        assert!(c2.put(&key, &dsp));
        assert_eq!(c2.len(), 1);
        let known = analyze(&audio, SR, None, "", Options { known_bpm: Some(124.0), ..Options::default() });
        assert!(!c2.put(&key, &known));
        assert_eq!(c2.len(), 1);
        let _ = std::fs::remove_dir_all(&c.dir);
        let _ = std::fs::remove_dir_all(&c2.dir);
    }

    #[test]
    fn a_changed_model_sha_misses() {
        let c = tmp("sha");
        let audio = click_track(110.0, 0.2, 12.0, SR);
        let key = content_key(&audio, SR);
        let g = analyze(&audio, SR, None, "", Options::default()).grid.unwrap();
        let pinned = models::analyzer();
        assert!(c.put_grid(&key, &pinned, "neural", &g, 12.0));
        assert_eq!(c.get(&key, &pinned).unwrap().analyzer, pinned);
        let other = analyzers::neural(&"f".repeat(64));
        assert!(c.get(&key, &other).is_none(), "another model is another analyzer");
        // and an entry renamed onto the other model's file is still not read as its
        std::fs::copy(c.path(&key, &pinned), c.path(&key, &other)).unwrap();
        assert!(c.get(&key, &other).is_none());
        let _ = std::fs::remove_dir_all(&c.dir);
    }

    #[test]
    fn an_old_schema_file_is_not_returned() {
        let c = tmp("v1");
        let audio = click_track(100.0, 0.2, 12.0, SR);
        let key = content_key(&audio, SR);
        let g = analyze(&audio, SR, None, "", Options::default()).grid.unwrap();
        // a v1 entry as it was written: no analyzer, named by the content key alone
        let mut o = JsonObject::new();
        o.set("schema", JsonValue::str("djrust/beatgrid/v1"));
        o.set("key", JsonValue::str(&key));
        o.set("method", JsonValue::str("neural"));
        o.set("duration", JsonValue::Number(12.0));
        o.set("grid", grid_to_json(&g, true));
        let v1 = JsonValue::Object(o).canonical_js();
        std::fs::create_dir_all(&c.dir).unwrap();
        std::fs::write(c.dir.join(format!("{key}.json")), &v1).unwrap();
        // even where a v2 entry would be, under any analyzer
        for analyzer in [analyzers::BEATGRID_DSP.to_string(), models::analyzer()] {
            std::fs::write(c.path(&key, &analyzer), &v1).unwrap();
            assert!(c.get(&key, &analyzer).is_none(), "{analyzer}: a v1 file is analysed again, not misread");
        }
        // what is written now is read
        assert!(c.put_grid(&key, analyzers::BEATGRID_DSP, "dsp", &g, 12.0));
        assert!(c.get(&key, analyzers::BEATGRID_DSP).is_some());
        assert_eq!(c.len(), 2, "the v1 file under its own name (`<key>.json`) is not counted");
        let _ = std::fs::remove_dir_all(&c.dir);
    }
}
