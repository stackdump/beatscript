//! What the Wheel remembers between launches: the instrument last played, the deck its sync ×
//! follows, and per instrument the note sequence wound on it, its build knobs and the finger's
//! gear. djandroid keeps the same in its preferences (`PlayerViewModel.loadWithSequence`); here
//! it is one JSON file, so the restore order (load → sequence → knobs) lives in one place.

use std::path::{Path, PathBuf};

use djengine::json::{JsonObject, JsonValue};

use super::{Wheel, WheelKnobs, WheelLayout};

/// The finger's gear on a cord before one is set: the drawing moves this many times slower than
/// the cord.
pub const DEFAULT_GEAR: f64 = 6.0;
/// The instrument a first launch puts on the wheel.
pub const DEFAULT_PRESET: &str = "hoop-looper";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Kept {
    pub sequence: Option<String>,
    pub knobs: Option<WheelKnobs>,
    pub gear: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub path: Option<PathBuf>,
    pub preset: String,
    pub follow: String,
    pub per: Vec<(String, Kept)>,
}

impl Memory {
    /// Nothing remembered, saved nowhere.
    pub fn new() -> Memory {
        Memory { path: None, preset: DEFAULT_PRESET.into(), follow: "A".into(), per: vec![] }
    }

    /// Read `path` (missing or unreadable: a fresh memory that saves there).
    pub fn open(path: &Path) -> Memory {
        let mut m = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| JsonValue::parse(&t).ok())
            .map(|v| Memory::from_json(&v))
            .unwrap_or_else(Memory::new);
        m.path = Some(path.to_path_buf());
        m
    }

    pub fn from_json(v: &JsonValue) -> Memory {
        let mut m = Memory::new();
        if let Some(p) = v.get("preset").and_then(|x| x.as_str()) {
            m.preset = p.to_string();
        }
        if let Some(f) = v.get("follow").and_then(|x| x.as_str()).filter(|f| matches!(*f, "A" | "B")) {
            m.follow = f.to_string();
        }
        if let Some(JsonValue::Object(o)) = v.get("instruments") {
            for (id, k) in o.iter() {
                let knobs = k.get("knobs").and_then(|j| {
                    Some(WheelKnobs {
                        magnets: j.get("magnets")?.as_f64()? as u32,
                        carrier_g: j.get("carrier_g")?.as_f64()?,
                        braking: j.get("braking")?.as_bool()?,
                        release_m: j.get("release_m")?.as_f64()?,
                        load_ohms: j.get("load_ohms")?.as_f64()?,
                        timbre: j.get("timbre")?.as_str()?.to_string(),
                        decay_ms: j.get("decay_ms")?.as_f64()?,
                    })
                });
                m.per.push((
                    id.to_string(),
                    Kept {
                        sequence: k.get("sequence").and_then(|x| x.as_str()).map(str::to_string),
                        knobs,
                        gear: k.get("gear").and_then(|x| x.as_f64()),
                    },
                ));
            }
        }
        m
    }

    pub fn to_json(&self) -> JsonValue {
        let mut per = JsonObject::new();
        for (id, k) in &self.per {
            let mut o = JsonObject::new();
            if let Some(s) = &k.sequence {
                o.set("sequence", JsonValue::str(s.clone()));
            }
            if let Some(g) = k.gear {
                o.set("gear", JsonValue::Number(g));
            }
            if let Some(n) = &k.knobs {
                o.set(
                    "knobs",
                    JsonValue::Object(JsonObject::from_pairs([
                        ("magnets", JsonValue::Number(n.magnets as f64)),
                        ("carrier_g", JsonValue::Number(n.carrier_g)),
                        ("braking", JsonValue::Bool(n.braking)),
                        ("release_m", JsonValue::Number(n.release_m)),
                        ("load_ohms", JsonValue::Number(n.load_ohms)),
                        ("timbre", JsonValue::str(n.timbre.clone())),
                        ("decay_ms", JsonValue::Number(n.decay_ms)),
                    ])),
                );
            }
            per.set(id.clone(), JsonValue::Object(o));
        }
        JsonValue::Object(JsonObject::from_pairs([
            ("preset", JsonValue::str(self.preset.clone())),
            ("follow", JsonValue::str(self.follow.clone())),
            ("instruments", JsonValue::Object(per)),
        ]))
    }

    /// Write it where it was opened from (nowhere: nothing). Atomic: a temp file renamed.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = &self.path else { return Ok(()) };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.part");
        std::fs::write(&tmp, self.to_json().js_stringified())?;
        std::fs::rename(&tmp, p)
    }

    pub fn kept(&self, id: &str) -> Kept {
        self.per.iter().find(|(i, _)| i == id).map(|(_, k)| k.clone()).unwrap_or_default()
    }

    fn kept_mut(&mut self, id: &str) -> &mut Kept {
        if let Some(i) = self.per.iter().position(|(x, _)| x == id) {
            return &mut self.per[i].1;
        }
        self.per.push((id.to_string(), Kept::default()));
        &mut self.per.last_mut().expect("pushed").1
    }

    pub fn gear(&self, id: &str) -> f64 {
        self.kept(id).gear.unwrap_or(DEFAULT_GEAR)
    }

    pub fn set_gear(&mut self, id: &str, g: f64) {
        self.kept_mut(id).gear = Some(g);
    }

    /// `None` forgets it (the preset's own notes).
    pub fn set_sequence(&mut self, id: &str, s: Option<&str>) {
        self.kept_mut(id).sequence = s.map(str::to_string).filter(|s| !s.trim().is_empty());
    }

    pub fn set_knobs(&mut self, id: &str, k: &WheelKnobs) {
        self.kept_mut(id).knobs = Some(k.clone());
    }

    /// Load `id` on `w` with what it had last time: its sequence, then its build knobs. A
    /// restore that fails falls back to what loaded and is said in the second value.
    pub fn load(&mut self, w: &Wheel, id: &str) -> Result<(WheelLayout, Vec<String>), String> {
        let mut l = w.load(id)?;
        self.preset = id.to_string();
        let k = self.kept(id);
        let mut notes = vec![];
        if let Some(s) = k.sequence.filter(|s| *s != l.sequence) {
            match w.set_sequence(&s) {
                Ok(x) => l = x,
                Err(e) => notes.push(format!("the saved notes {s:?} would not wind: {e}")),
            }
        }
        if let Some(knobs) = k.knobs {
            match w.set_knobs(&knobs) {
                Ok(x) => l = x,
                Err(e) => notes.push(format!("the saved build knobs would not apply: {e}")),
            }
        }
        Ok((l, notes))
    }
}

impl Default for Memory {
    fn default() -> Self {
        Memory::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_round_trips_and_restores_an_instrument() {
        let dir = std::env::temp_dir().join(format!("djhost-wheel-mem-{}", std::process::id()));
        let path = dir.join("wheel.json");
        let _ = std::fs::remove_dir_all(&dir);
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (w, t) = Wheel::start(48_000.0, tx, 1024, None, stop).unwrap();
        t.join().unwrap();
        let mut m = Memory::open(&path);
        let (l, _) = m.load(&w, "hoop-looper").unwrap();
        let mut k = WheelKnobs {
            magnets: 3,
            carrier_g: 5.0,
            braking: false,
            release_m: l.release_m,
            load_ohms: l.load_ohms,
            timbre: l.timbre.clone(),
            decay_ms: l.decay_ms,
        };
        m.set_knobs("hoop-looper", &k);
        m.set_sequence("hoop-looper", Some("C E G"));
        m.set_gear("hoop-looper", 2.5);
        m.follow = "B".into();
        m.save().unwrap();
        let mut back = Memory::open(&path);
        assert_eq!(back.to_json(), m.to_json());
        assert_eq!(back.gear("hoop-looper"), 2.5);
        assert_eq!(back.gear("other"), DEFAULT_GEAR);
        let (l2, notes) = back.load(&w, "hoop-looper").unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!((l2.magnets, l2.braking, l2.sequence.as_str()), (3, false, "C E G"));
        k.magnets = 1;
        assert_ne!(back.kept("hoop-looper").knobs, Some(k));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
