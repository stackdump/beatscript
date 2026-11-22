//! Grids cached by the content hash of their audio: `<dir>/<key>.json`. The same file or loop
//! analysed twice (or under another name) is read, not computed. A grid the DSP detector made is
//! kept too, but a caller that wants Beat This! asks again (`method` says which made it).

use std::path::{Path, PathBuf};

use djengine::beatgrid::BeatGrid;
use djengine::json::{JsonObject, JsonValue};

use super::{grid_from_json, grid_to_json, Analysis};

pub const SCHEMA: &str = "djrust/beatgrid/v1";

/// What a cache entry holds.
#[derive(Clone, Debug)]
pub struct Cached {
    pub grid: BeatGrid,
    /// `neural` or `dsp`.
    pub method: String,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Cache {
        Cache { dir: dir.into() }
    }

    /// `$DJRUST_ANALYSIS`, else `<support>/analysis`.
    pub fn open_default() -> Cache {
        Cache::new(crate::stores::env_or_support("DJRUST_ANALYSIS", "analysis"))
    }

    fn path(&self, key: &str) -> PathBuf {
        // a key is hex we made, but this is a path: keep it a file name
        let safe: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        self.dir.join(format!("{safe}.json"))
    }

    pub fn get(&self, key: &str) -> Option<Cached> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        let v = JsonValue::parse(&text).ok()?;
        if v.get("schema").and_then(JsonValue::as_str) != Some(SCHEMA) {
            return None;
        }
        Some(Cached {
            grid: grid_from_json(v.get("grid")?)?,
            method: v.get("method")?.as_str()?.to_string(),
            duration: v.get("duration").and_then(JsonValue::as_f64).unwrap_or(0.0),
        })
    }

    /// Keep the analysis's best grid. Quietly does nothing for no grid or an unwritable folder.
    pub fn put(&self, key: &str, a: &Analysis) -> bool {
        let Some(g) = &a.grid else { return false };
        let mut o = JsonObject::new();
        o.set("schema", JsonValue::str(SCHEMA));
        o.set("key", JsonValue::str(key));
        o.set("method", JsonValue::str(a.method));
        o.set("duration", JsonValue::Number(a.duration));
        o.set("grid", grid_to_json(g, true));
        crate::stores::write_atomic(&self.path(key), JsonValue::Object(o).canonical_js().as_bytes()).is_ok()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many grids are kept.
    pub fn len(&self) -> usize {
        std::fs::read_dir(&self.dir)
            .map(|d| d.filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
