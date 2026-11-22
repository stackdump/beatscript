//! Live mashups from stems (docs/MASHUPS.md): drums from one track, bass from another, the
//! melody from a third, each part with its own level, mute and solo, all on one tempo and moved
//! into one key.
//!
//! A [`Mashup`] is plain data with a canonical JSON and a content ID, like a set list: two
//! mashups of the same parts are the same mashup. Its `parents` are the CIDs of every source
//! track, so a mashup is a node in the lineage graph ("GitHub for music"): it points at what it
//! was made of, and a fork of it points at it (`forkedFrom`).
//!
//! **Generated tracks** are realised now ([`Mashup::realize`]): every instrument of a generated
//! track is its own net ([`crate::stems::NetStems`]), so the mashup is one raw-`nets` share
//! payload holding the chosen nets of each source's loop, at the mashup's tempo, the tonal ones
//! transposed into key and every part's level in its velocities. A deck plays it as it plays any
//! share, and the payload's own CID is what the deck, the action log and a set recording name.
//!
//! **External audio** parts are part of the model (a source of kind `audio`), and the stem
//! source that will split them is a stub ([`crate::stems::DemucsStems`]); realising a mashup
//! with an audio part says so.
//!
//! Pure: no I/O, `no_std` + `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};
use crate::key::{compat, suggest_transpose, Compat, Key};
use crate::loopsets::Side;
use crate::pflow::Project;
use crate::stems::{NetStems, StemClass, StemSource};

pub const SCHEMA: &str = "djrust.mashup/1";

/// What a source track is.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceRef {
    /// A generated track: genre and seed, its `side` riffs as a loop (the loop sets' cut).
    Net { genre: String, seed: u32, side: Side },
    /// A raw-`nets` share payload (a saved track, another mashup, a share link).
    Raw { payload: JsonValue },
    /// External audio by content ID: needs a stem separator (stub for now).
    Audio { cid: String, title: String },
}

/// One source track.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// "A", "B", "C": what parts name it by.
    pub label: String,
    pub source: SourceRef,
    /// Its key: the project's own for a net, else given (or detected by the host).
    pub key: Option<Key>,
    /// Its own tempo.
    pub bpm: Option<f64>,
}

impl Source {
    /// A generated track, its key and tempo read from the composed project.
    pub fn net(label: &str, genre: &str, seed: u32, side: Side) -> Source {
        let mut s = Source {
            label: label.to_string(),
            source: SourceRef::Net { genre: genre.to_string(), seed, side },
            key: None,
            bpm: None,
        };
        if let Some(p) = s.project() {
            s.key = Key::from_project(&p);
            s.bpm = Some(p.tempo);
        }
        s
    }

    /// A raw-`nets` payload; its key, if the payload says (`key`, or `rootNote` + `scaleName`).
    pub fn raw(label: &str, payload: JsonValue) -> Source {
        let key = payload
            .get("key")
            .and_then(JsonValue::as_str)
            .and_then(Key::parse)
            .or_else(|| {
                let r = payload.get("rootNote").and_then(JsonValue::as_f64)?;
                let sc = payload.get("scaleName").and_then(JsonValue::as_str).unwrap_or("");
                Some(Key::from_root_scale(r as i64, sc))
            });
        let bpm = payload.get("tempo").and_then(JsonValue::as_f64);
        Source { label: label.to_string(), source: SourceRef::Raw { payload }, key, bpm }
    }

    pub fn audio(label: &str, cid: &str, title: &str, key: Option<Key>, bpm: Option<f64>) -> Source {
        Source {
            label: label.to_string(),
            source: SourceRef::Audio { cid: cid.to_string(), title: title.to_string() },
            key,
            bpm,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self.source {
            SourceRef::Net { .. } => "net",
            SourceRef::Raw { .. } => "track",
            SourceRef::Audio { .. } => "audio",
        }
    }

    /// The source's content ID: the share CID of a generated track's spec (what a share link of
    /// it is named by) or of a payload; an audio file's own.
    pub fn cid(&self) -> String {
        match &self.source {
            SourceRef::Net { genre, seed, .. } => {
                let spec = crate::TrackSpec::new(genre, *seed as f64, Some("standard"));
                crate::cid::share_cid(&spec.payload())
            }
            SourceRef::Raw { payload } => crate::cid::share_cid(payload),
            SourceRef::Audio { cid, .. } => cid.clone(),
        }
    }

    /// The loop project its net stems come from. None for audio.
    pub fn project(&self) -> Option<Project> {
        match &self.source {
            SourceRef::Net { genre, seed, side } => {
                Some(crate::loopsets::cut(&crate::loopsets::played(genre, *seed), *side, true))
            }
            SourceRef::Raw { payload } => {
                let canonical = payload.canonical_js();
                let ordered = JsonValue::parse(&canonical).unwrap_or_else(|_| payload.clone());
                crate::share::Share::from_payload(&ordered, Some(&crate::cid::share_cid_canonical(&canonical)))
                    .raw_nets_project()
            }
            SourceRef::Audio { .. } => None,
        }
    }

    /// "techno:7", "track z…ab12", "audio Song.wav".
    pub fn text(&self) -> String {
        match &self.source {
            SourceRef::Net { genre, seed, side } => {
                format!("{genre}:{seed}{}", if *side == Side::B { " B" } else { "" })
            }
            SourceRef::Raw { payload } => match payload.get("name").and_then(JsonValue::as_str) {
                Some(n) => format!("track {n}"),
                None => format!("track {}", short(&self.cid())),
            },
            SourceRef::Audio { title, .. } => format!("audio {title}"),
        }
    }

    /// A source as people write it: `techno:7`, `dnb:2602:B`, `track:<cid>` (the host resolves
    /// a saved track to a payload, so this answers only the spec forms), `audio:<cid>`.
    pub fn parse(label: &str, text: &str) -> Result<Source, String> {
        let t = text.trim();
        if let Some(c) = t.strip_prefix("audio:") {
            return Ok(Source::audio(label, c.trim(), c.trim(), None, None));
        }
        let parts: Vec<&str> = t.split(':').collect();
        match parts.as_slice() {
            [g, s] | [g, s, _] if !g.is_empty() => {
                let seed: u32 = s.trim().parse().map_err(|_| format!("source {t:?}: the seed is a whole number"))?;
                let side = match parts.get(2).map(|x| x.trim().to_ascii_uppercase()) {
                    None => Side::A,
                    Some(x) if x == "A" => Side::A,
                    Some(x) if x == "B" => Side::B,
                    Some(x) => return Err(format!("source {t:?}: side {x:?} is A or B")),
                };
                if !crate::loopsets::genres().iter().any(|n| n == g.trim()) {
                    return Err(format!("source {t:?}: no genre {:?}", g.trim()));
                }
                Ok(Source::net(label, g.trim(), seed, side))
            }
            _ => Err(format!("source {t:?}: a generated track is genre:seed (techno:7, dnb:2602:B), audio is audio:<cid>")),
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("label", JsonValue::String(self.label.clone()));
        o.set("kind", JsonValue::str(self.kind()));
        o.set("cid", JsonValue::String(self.cid()));
        match &self.source {
            SourceRef::Net { genre, seed, side } => {
                o.set("genre", JsonValue::String(genre.clone()));
                o.set("seed", JsonValue::Number(*seed as f64));
                o.set("side", JsonValue::str(if *side == Side::B { "B" } else { "A" }));
            }
            SourceRef::Raw { payload } => o.set("payload", payload.clone()),
            SourceRef::Audio { title, .. } => o.set("title", JsonValue::String(title.clone())),
        }
        o.set("key", self.key.map(|k| JsonValue::String(k.camelot())).unwrap_or(JsonValue::Null));
        o.set("bpm", self.bpm.map(JsonValue::Number).unwrap_or(JsonValue::Null));
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Source, String> {
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).map(str::to_string);
        let label = s("label").ok_or("source: no label")?;
        let key = s("key").and_then(|k| Key::parse(&k));
        let bpm = v.get("bpm").and_then(JsonValue::as_f64);
        let src = match s("kind").as_deref() {
            Some("net") => SourceRef::Net {
                genre: s("genre").ok_or("source: a net needs a genre")?,
                seed: v.get("seed").and_then(JsonValue::as_f64).ok_or("source: a net needs a seed")? as u32,
                side: if s("side").as_deref() == Some("B") { Side::B } else { Side::A },
            },
            Some("track") => SourceRef::Raw { payload: v.get("payload").cloned().ok_or("source: a track needs its payload")? },
            Some("audio") => SourceRef::Audio {
                cid: s("cid").ok_or("source: audio needs its cid")?,
                title: s("title").unwrap_or_default(),
            },
            k => return Err(format!("source: kind {k:?} is net, track or audio")),
        };
        Ok(Source { label, source: src, key, bpm })
    }
}

fn short(cid: &str) -> String {
    let n = cid.chars().count();
    if n <= 12 {
        cid.to_string()
    } else {
        format!("{}…{}", cid.chars().take(6).collect::<String>(), cid.chars().skip(n - 4).collect::<String>())
    }
}

/// One part of the mashup: a stem of a source on a slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// What it plays in the mashup ("drums", "bass", "melody", or any name).
    pub slot: String,
    /// The source's label.
    pub source: String,
    /// The stem of that source (`drums`, `bass`, `harmony`, `melody`, or a net's id).
    pub stem: String,
    /// 0..1.
    pub level: f64,
    pub mute: bool,
    pub solo: bool,
    /// Semitones it moves, on top of nothing (drum stems ignore it).
    pub transpose: i32,
}

impl Part {
    pub fn new(slot: &str, source: &str, stem: &str) -> Part {
        Part { slot: slot.to_string(), source: source.to_string(), stem: stem.to_string(), level: 1.0, mute: false, solo: false, transpose: 0 }
    }

    pub fn to_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("slot", JsonValue::String(self.slot.clone())),
            ("source", JsonValue::String(self.source.clone())),
            ("stem", JsonValue::String(self.stem.clone())),
            ("level", JsonValue::Number(round3(self.level))),
            ("mute", JsonValue::Bool(self.mute)),
            ("solo", JsonValue::Bool(self.solo)),
            ("transpose", JsonValue::Number(self.transpose as f64)),
        ]))
    }

    pub fn from_json(v: &JsonValue) -> Result<Part, String> {
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).map(str::to_string);
        let mut p = Part::new(
            &s("slot").ok_or("part: no slot")?,
            &s("source").ok_or("part: no source")?,
            &s("stem").ok_or("part: no stem")?,
        );
        p.level = v.get("level").and_then(JsonValue::as_f64).unwrap_or(1.0).clamp(0.0, 1.0);
        p.mute = v.get("mute").and_then(JsonValue::as_bool).unwrap_or(false);
        p.solo = v.get("solo").and_then(JsonValue::as_bool).unwrap_or(false);
        p.transpose = v.get("transpose").and_then(JsonValue::as_f64).unwrap_or(0.0) as i32;
        Ok(p)
    }

    /// Is this part a pitched one? By its stem's class.
    pub fn tonal(&self) -> bool {
        StemClass::parse(&self.stem).is_none_or(StemClass::tonal)
    }
}

fn round3(x: f64) -> f64 {
    libm::round(x * 1000.0) / 1000.0
}

/// A key warning on a part.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub slot: String,
    /// The part's key after its transpose.
    pub key: Key,
    pub compat: Compat,
    /// The transpose (total, for the part) that would make it mix.
    pub suggest: i32,
    pub suggest_compat: Compat,
}

impl Clash {
    pub fn json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("slot", JsonValue::String(self.slot.clone())),
            ("key", JsonValue::String(self.key.camelot())),
            ("compat", JsonValue::str(self.compat.name())),
            ("suggest_transpose", JsonValue::Number(self.suggest as f64)),
            ("suggest_compat", JsonValue::str(self.suggest_compat.name())),
            (
                "text",
                JsonValue::String(format!(
                    "{}: {} against the mashup's key ({}); transpose {:+} → {}",
                    self.slot,
                    self.key.camelot(),
                    self.compat.name(),
                    self.suggest,
                    self.suggest_compat.name()
                )),
            ),
        ]))
    }
}

/// A mashup.
#[derive(Clone, Debug, PartialEq)]
pub struct Mashup {
    pub name: String,
    /// The tempo it plays at (default: the first source's).
    pub bpm: f64,
    /// The key it is in (default: the first tonal part's source key).
    pub key: Option<Key>,
    pub sources: Vec<Source>,
    pub parts: Vec<Part>,
    /// The mashup this one was forked from.
    pub forked_from: Option<String>,
}

impl Mashup {
    pub fn new(name: &str) -> Mashup {
        Mashup { name: name.to_string(), bpm: 0.0, key: None, sources: Vec::new(), parts: Vec::new(), forked_from: None }
    }

    /// The next free source label: A, B, C, …
    pub fn next_label(&self) -> String {
        for c in 'A'..='Z' {
            let l = c.to_string();
            if !self.sources.iter().any(|s| s.label == l) {
                return l;
            }
        }
        format!("S{}", self.sources.len() + 1)
    }

    pub fn source(&self, label: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.label.eq_ignore_ascii_case(label))
    }

    /// Add a source; the first sets the tempo when none is set.
    pub fn add_source(&mut self, s: Source) -> Result<(), String> {
        if self.source(&s.label).is_some() {
            return Err(format!("mashup: there is already a source {}", s.label));
        }
        if self.bpm <= 0.0 {
            if let Some(b) = s.bpm {
                self.bpm = b;
            }
        }
        self.sources.push(s);
        Ok(())
    }

    /// Remove a source and the parts that use it.
    pub fn remove_source(&mut self, label: &str) -> Result<(), String> {
        let before = self.sources.len();
        self.sources.retain(|s| !s.label.eq_ignore_ascii_case(label));
        if self.sources.len() == before {
            return Err(format!("mashup: no source {label}"));
        }
        self.parts.retain(|p| !p.source.eq_ignore_ascii_case(label));
        Ok(())
    }

    /// Put `stem` of `source` on `slot` (replacing what the slot held).
    pub fn assign(&mut self, slot: &str, source: &str, stem: &str) -> Result<(), String> {
        let src = self.source(source).ok_or_else(|| format!("mashup: no source {source}"))?.clone();
        let stems = match src.source {
            SourceRef::Audio { .. } => crate::stems::DemucsStems.stems(None),
            _ => NetStems.stems(src.project().as_ref()),
        }
        .map_err(|e| format!("mashup: {e}"))?;
        let found = stems
            .iter()
            .find(|s| s.id == stem || (StemClass::parse(stem) == Some(s.class) && s.class != StemClass::Other))
            .ok_or_else(|| {
                let have: Vec<String> = stems.iter().map(|s| s.id.clone()).collect();
                format!("mashup: source {} has no stem {stem:?} ({})", src.label, have.join(", "))
            })?;
        let part = Part::new(slot, &src.label, &found.id);
        match self.parts.iter_mut().find(|p| p.slot == slot) {
            Some(p) => {
                p.source = part.source;
                p.stem = part.stem;
            }
            None => self.parts.push(part),
        }
        Ok(())
    }

    pub fn part_mut(&mut self, slot: &str) -> Result<&mut Part, String> {
        let names: Vec<String> = self.parts.iter().map(|p| p.slot.clone()).collect();
        self.parts
            .iter_mut()
            .find(|p| p.slot.eq_ignore_ascii_case(slot))
            .ok_or_else(|| format!("mashup: no slot {slot:?} (slots: {})", names.join(", ")))
    }

    /// The parts that sound: unmuted, and only the soloed ones while any is soloed.
    pub fn audible(&self) -> Vec<&Part> {
        let any_solo = self.parts.iter().any(|p| p.solo && !p.mute);
        self.parts.iter().filter(|p| !p.mute && p.level > 0.0 && (!any_solo || p.solo)).collect()
    }

    /// The key the mashup is in: as set, else the first tonal part's source key.
    pub fn target_key(&self) -> Option<Key> {
        self.key.or_else(|| {
            self.parts.iter().filter(|p| p.tonal()).find_map(|p| self.source(&p.source).and_then(|s| s.key))
        })
    }

    /// The key a part sounds in: its source's key moved by its transpose.
    pub fn part_key(&self, p: &Part) -> Option<Key> {
        self.source(&p.source).and_then(|s| s.key).map(|k| k.transposed(p.transpose))
    }

    /// Every tonal part not in harmony with the mashup's key, with the transpose that fixes it.
    pub fn clashes(&self) -> Vec<Clash> {
        let Some(target) = self.target_key() else { return Vec::new() };
        let mut out = Vec::new();
        for p in self.parts.iter().filter(|p| p.tonal()) {
            let Some(src_key) = self.source(&p.source).and_then(|s| s.key) else { continue };
            let k = src_key.transposed(p.transpose);
            let c = compat(target, k);
            if c.harmonic() {
                continue;
            }
            let t = suggest_transpose(target, src_key);
            out.push(Clash { slot: p.slot.clone(), key: k, compat: c, suggest: t.semitones, suggest_compat: t.compat });
        }
        out
    }

    /// Set every tonal part's transpose to the smallest one that puts it in harmony with the
    /// mashup's key (`key auto`). Answers the slots moved.
    pub fn auto_key(&mut self) -> Vec<(String, i32)> {
        let Some(target) = self.target_key() else { return Vec::new() };
        let mut moved = Vec::new();
        let keys: Vec<Option<Key>> = self.parts.iter().map(|p| self.source(&p.source).and_then(|s| s.key)).collect();
        for (p, k) in self.parts.iter_mut().zip(keys) {
            let (true, Some(src_key)) = (p.tonal(), k) else { continue };
            let t = suggest_transpose(target, src_key).semitones;
            if p.transpose != t {
                p.transpose = t;
                moved.push((p.slot.clone(), t));
            }
        }
        moved
    }

    /// Every source track's CID, in source order, no repeats: the lineage edges.
    pub fn parents(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in &self.sources {
            let c = s.cid();
            if !v.contains(&c) {
                v.push(c);
            }
        }
        v
    }

    /// The canonical JSON the CID is of.
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("schema", JsonValue::str(SCHEMA));
        o.set("name", JsonValue::String(self.name.clone()));
        o.set("bpm", JsonValue::Number(round3(self.bpm)));
        o.set("key", self.key.map(|k| JsonValue::String(k.camelot())).unwrap_or(JsonValue::Null));
        o.set("sources", JsonValue::Array(self.sources.iter().map(Source::to_json).collect()));
        o.set("parts", JsonValue::Array(self.parts.iter().map(Part::to_json).collect()));
        o.set("parents", JsonValue::Array(self.parents().into_iter().map(JsonValue::String).collect()));
        if let Some(f) = &self.forked_from {
            o.set("forkedFrom", JsonValue::String(f.clone()));
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Mashup, String> {
        if v.get("schema").and_then(JsonValue::as_str) != Some(SCHEMA) {
            return Err(format!("mashup: not a mashup (schema {SCHEMA})"));
        }
        let mut m = Mashup::new(v.get("name").and_then(JsonValue::as_str).unwrap_or("Mashup"));
        m.bpm = v.get("bpm").and_then(JsonValue::as_f64).unwrap_or(0.0);
        m.key = v.get("key").and_then(JsonValue::as_str).and_then(Key::parse);
        for s in v.get("sources").and_then(JsonValue::as_array).map(|a| a.as_slice()).unwrap_or(&[]) {
            m.sources.push(Source::from_json(s)?);
        }
        for p in v.get("parts").and_then(JsonValue::as_array).map(|a| a.as_slice()).unwrap_or(&[]) {
            m.parts.push(Part::from_json(p)?);
        }
        m.forked_from = v.get("forkedFrom").and_then(JsonValue::as_str).map(str::to_string);
        Ok(m)
    }

    pub fn canonical(&self) -> String {
        self.to_json().canonical_js()
    }

    pub fn cid(&self) -> String {
        crate::cid::share_cid_canonical(&self.canonical())
    }

    /// A copy to edit, pointing back at this one.
    pub fn fork(&self, name: Option<&str>) -> Mashup {
        let mut m = self.clone();
        m.forked_from = Some(self.cid());
        if let Some(n) = name {
            m.name = n.to_string();
        }
        m
    }

    /// The mashup as one raw-`nets` share payload a deck plays: each audible part's nets from its
    /// source's loop, renamed apart when two sources bring the same net, the tonal ones
    /// transposed (a drum net never is), and the part's level in its velocities. The payload
    /// carries `mashup` (this mashup's CID) so a deck, the action log and a recording can tell it
    /// from a plain track. An audio part is an error until a stem separator is built in.
    pub fn realize(&self) -> Result<JsonValue, String> {
        let audible = self.audible();
        if audible.is_empty() {
            return Err("mashup: nothing to play (no parts, or every part muted)".into());
        }
        let mut projects: Vec<(String, Project)> = Vec::new();
        let mut nets = JsonObject::new();
        let mut taken: Vec<String> = Vec::new();
        for p in audible {
            let src = self.source(&p.source).ok_or_else(|| format!("mashup: part {} names no source {}", p.slot, p.source))?;
            if let SourceRef::Audio { .. } = src.source {
                return Err(format!("mashup: part {} is audio ({}); {}", p.slot, src.text(), crate::stems::DemucsStems::UNAVAILABLE));
            }
            if !projects.iter().any(|(l, _)| *l == src.label) {
                let pr = src.project().ok_or_else(|| format!("mashup: source {} has no nets", src.label))?;
                projects.push((src.label.clone(), pr));
            }
            let project = &projects.iter().find(|(l, _)| *l == src.label).unwrap().1;
            let stem = NetStems.net_stem(project, &p.stem).map_err(|e| format!("mashup: {}: {e}", p.slot))?;
            for (id, mut nb) in stem {
                let drum = crate::stems::is_drum_net(&id, &nb);
                if !drum && p.transpose != 0 {
                    for k in nb.bindings.keys().to_vec() {
                        if let Some(b) = nb.bindings.get_mut(&k) {
                            b.note = (b.note + p.transpose as i64).clamp(0, 127);
                        }
                    }
                }
                if p.level < 1.0 {
                    let scale = |v: i64| (libm::round(v as f64 * p.level) as i64).clamp(1, 127);
                    for k in nb.bindings.keys().to_vec() {
                        if let Some(b) = nb.bindings.get_mut(&k) {
                            b.velocity = scale(b.velocity);
                        }
                    }
                    nb.track.default_velocity = scale(nb.track.default_velocity);
                }
                let mut name = id.clone();
                if taken.contains(&name) {
                    name = format!("{id}_{}", src.label.to_ascii_lowercase());
                }
                taken.push(name.clone());
                nets.set(&name, nb.to_json());
            }
        }
        let mut o = JsonObject::new();
        o.set("name", JsonValue::String(self.name.clone()));
        o.set("nets", JsonValue::Object(nets));
        o.set("tempo", JsonValue::Number(if self.bpm > 0.0 { round3(self.bpm) } else { 120.0 }));
        o.set("seed", JsonValue::Number(0.0));
        o.set("mashup", JsonValue::String(self.cid()));
        if let Some(k) = self.target_key() {
            o.set("key", JsonValue::String(k.name()));
        }
        Ok(JsonValue::Object(o))
    }

    /// For lists and the panel: name, cid, bpm, key, sources, parts, clashes.
    pub fn summary_json(&self) -> JsonValue {
        let mut o = match self.to_json() {
            JsonValue::Object(o) => o,
            _ => JsonObject::new(),
        };
        // payloads are long; a summary names them
        let srcs: Vec<JsonValue> = self
            .sources
            .iter()
            .map(|s| {
                JsonValue::Object(JsonObject::from_pairs([
                    ("label", JsonValue::String(s.label.clone())),
                    ("kind", JsonValue::str(s.kind())),
                    ("text", JsonValue::String(s.text())),
                    ("cid", JsonValue::String(s.cid())),
                    ("key", s.key.map(|k| k.json()).unwrap_or(JsonValue::Null)),
                    ("bpm", s.bpm.map(JsonValue::Number).unwrap_or(JsonValue::Null)),
                ]))
            })
            .collect();
        o.set("sources", JsonValue::Array(srcs));
        o.set("cid", JsonValue::String(self.cid()));
        o.set("target_key", self.target_key().map(|k| k.json()).unwrap_or(JsonValue::Null));
        o.set("clashes", JsonValue::Array(self.clashes().iter().map(Clash::json).collect()));
        o.set(
            "audible",
            JsonValue::Array(self.audible().iter().map(|p| JsonValue::String(p.slot.clone())).collect()),
        );
        JsonValue::Object(o)
    }
}

/// Is a raw-`nets` payload a realised mashup? Its mashup CID.
pub fn mashup_of(payload: &JsonValue) -> Option<&str> {
    payload.get("mashup").and_then(JsonValue::as_str)
}
