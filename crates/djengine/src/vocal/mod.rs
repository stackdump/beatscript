//! Vocal lines (docs/VOICE.md, "Typed lyrics"): **words, sung on pitch by the formant voice**.
//!
//! A [`VocalLine`] is lyrics plus the notes they are sung on: typed text goes through the BeatScript
//! lexicon and letter-to-sound rules ([`lexicon`]), or is written in phonemes (`/…/`, [`phoneme`]),
//! into syllables ([`phone`]), onto a bar
//! grid in a key ([`fit`]), and the notes are sung by `djrust::fx::formant` ([`VocalLine::render`]).
//! No model, no recording and no download: the whole path allows a CC0 release. The robot is
//! the aesthetic, not realistic singing.
//!
//! The object is canonical JSON with a content ID, like a track or a set list, and carries its
//! own license (CC0 by default), visibility, author and lineage ([`Rights`]), so it can be
//! shared, forked and credited:
//!
//! ```text
//! {"format":1,"type":"djrust.vocal","name":"I P O","lyrics":"I P O, Claude is on the road",
//!  "key":"Bb minor","bpm":124,"bars":2,"preset":"robot","seed":0,
//!  "notes":[{"start":0,"dur":0.75,"midi":58,"syl":"ay","word":"i"}, …],
//!  "license":"CC0-1.0","visibility":"public"}
//! ```
//!
//! The fitted notes are part of the object (a later change to the fitting cannot move a saved
//! line), so a fork can retune them. Pure: no I/O, no clock; `no_std` + `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use djrust::fx::formant::{preset, FormantSynth, Onset, Syllable};

use crate::json::{JsonObject, JsonValue};
use crate::key::Key;
use crate::rights::{short_cid, Fork, Rights};

pub mod fit;
pub mod lexicon;
pub mod phone;
pub mod phoneme;

pub const FORMAT: i64 = 1;
pub const TYPE: &str = "djrust.vocal";
/// The voice a line is sung by unless it says otherwise.
pub const DEFAULT_PRESET: &str = "robot";

/// One sung syllable.
#[derive(Clone, Debug, PartialEq)]
pub struct VocalNote {
    /// Beats from the start of the loop.
    pub start: f64,
    pub dur: f64,
    pub midi: u8,
    /// The word this syllable begins ("" for its other syllables).
    pub word: String,
    pub syl: Syllable,
    /// Cluster consonants before and after (see [`phone`]).
    pub pre: Vec<Onset>,
    pub post: Vec<Onset>,
    /// `!` in an escape: sung harder (written only when set).
    pub accent: bool,
}

/// What a line is made from.
#[derive(Clone, Debug)]
pub struct Params {
    pub lyrics: String,
    pub name: Option<String>,
    pub key: Key,
    pub bpm: f64,
    pub bars: u32,
    pub preset: String,
    pub seed: u32,
    /// The pitches and rhythm to sing the syllables on, in order (None: fit them to the key).
    pub places: Option<Vec<fit::Place>>,
    pub rights: Rights,
}

impl Params {
    pub fn new(lyrics: &str, key: Key, bpm: f64, bars: u32) -> Params {
        Params {
            lyrics: lyrics.to_string(),
            name: None,
            key,
            bpm,
            bars,
            preset: DEFAULT_PRESET.to_string(),
            seed: 0,
            places: None,
            rights: Rights::new(None),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VocalLine {
    pub name: String,
    pub lyrics: String,
    pub key: Key,
    pub bpm: f64,
    pub bars: u32,
    pub preset: String,
    pub seed: u32,
    pub notes: Vec<VocalNote>,
    pub rights: Rights,
}

fn name_of(lyrics: &str) -> String {
    let t: String = lyrics.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
    let t = t.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    if t.is_empty() {
        "Vocal".to_string()
    } else {
        t
    }
}

fn pitch_text(m: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[(m % 12) as usize], m as i32 / 12 - 1)
}

impl VocalLine {
    /// Fit `p.lyrics` to a key and a grid (or to `p.places`) and make the line. The strings
    /// returned are what the person should hear about (a word the dictionary did not know and
    /// the rules guessed, a syllable with no note).
    pub fn create(p: Params) -> Result<(VocalLine, Vec<String>), String> {
        if !(20.0..=400.0).contains(&p.bpm) {
            return Err(format!("bpm {} is out of range (20 to 400)", p.bpm));
        }
        if !(1..=32).contains(&p.bars) {
            return Err(format!("bars {} is out of range (1 to 32)", p.bars));
        }
        if preset(&p.preset).is_none() {
            return Err(format!("no voice preset {:?} (see `voice presets`)", p.preset));
        }
        let slots = fit::syllables(&p.lyrics);
        let needed = fit::notes_needed(&slots);
        if slots.is_empty() {
            return Err("lyrics: no words to sing".to_string());
        }
        let mut warnings = Vec::new();
        let guessed: Vec<&str> = slots
            .iter()
            .filter(|s| s.via == lexicon::Via::Rules && !s.word.is_empty())
            .map(|s| s.word.as_str())
            .collect();
        if !guessed.is_empty() {
            warnings.push(format!("not in the lexicon, sounded out by rules: {} (respell it, or write its phonemes: /…/)", guessed.join(", ")));
        }
        let places = match &p.places {
            Some(pl) => {
                if pl.len() < needed {
                    warnings.push(format!("{needed} notes needed but {} given: the last {} are not sung", pl.len(), needed - pl.len()));
                } else if pl.len() > needed {
                    warnings.push(format!("{} notes for {needed} syllables: the extra notes rest", pl.len()));
                }
                pl.clone()
            }
            None => fit::place(&slots, p.key, p.bars, p.seed, &p.lyrics)?,
        };
        let mut notes = Vec::new();
        // a syllable takes a place; a held one (`:`) takes the next too, and lasts until that one ends
        let given = p.places.is_some();
        let mut k = 0usize;
        for slot in &slots {
            let Some(pl) = places.get(k) else { break };
            // (a fit of its own lays one place a syllable, so a hold only spans notes it was given)
            let take = if given && slot.hold && k + 1 < places.len() { 2 } else { 1 };
            let end = places[k + take - 1].start + places[k + take - 1].dur;
            k += take;
            if slot.rest || pl.dur <= 0.0 {
                continue;
            }
            notes.push(VocalNote {
                start: pl.start,
                dur: end - pl.start,
                midi: pl.midi,
                word: slot.word.clone(),
                syl: slot.sung.syl,
                pre: slot.sung.pre.clone(),
                post: slot.sung.post.clone(),
                accent: slot.accent,
            });
        }
        let beats = p.bars as f64 * 4.0;
        if notes.iter().any(|n| n.start + n.dur > beats + 1e-6) {
            warnings.push("a note runs past the end of the loop: it wraps".to_string());
        }
        let name = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| name_of(&p.lyrics), str::to_string);
        Ok((
            VocalLine {
                name,
                lyrics: p.lyrics.trim().to_string(),
                key: p.key,
                bpm: p.bpm,
                bars: p.bars,
                preset: p.preset,
                seed: p.seed,
                notes,
                rights: p.rights,
            },
            warnings,
        ))
    }

    /// Whole beats the line spans.
    pub fn beats(&self) -> f64 {
        self.bars as f64 * 4.0
    }

    /// The frames of one pass at `sample_rate`: exactly `bars` bars at `bpm`.
    pub fn frames(&self, sample_rate: f64) -> usize {
        libm::round(self.beats() * 60.0 / self.bpm * sample_rate).max(1.0) as usize
    }

    // MARK: JSON

    fn note_json(n: &VocalNote) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("start", JsonValue::Number(n.start));
        o.set("dur", JsonValue::Number(n.dur));
        o.set("midi", JsonValue::int(n.midi as i64));
        o.set("syl", n.syl.text().into());
        if !n.word.is_empty() {
            o.set("word", n.word.clone().into());
        }
        if !n.pre.is_empty() {
            o.set("pre", JsonValue::Array(n.pre.iter().map(|c| c.letter().into()).collect()));
        }
        if !n.post.is_empty() {
            o.set("post", JsonValue::Array(n.post.iter().map(|c| c.letter().into()).collect()));
        }
        if n.accent {
            o.set("accent", true.into());
        }
        JsonValue::Object(o)
    }

    fn note_from_json(v: &JsonValue) -> Result<VocalNote, String> {
        let num = |k: &str| v.get(k).and_then(JsonValue::as_f64).ok_or_else(|| format!("vocal note: {k}"));
        let midi = num("midi")?;
        if !(0.0..=127.0).contains(&midi) {
            return Err("vocal note: midi 0..127".to_string());
        }
        let syl = v.get("syl").and_then(JsonValue::as_str).and_then(Syllable::parse).ok_or("vocal note: a syllable (syl)")?;
        let onsets = |k: &str| -> Result<Vec<Onset>, String> {
            match v.get(k) {
                None | Some(JsonValue::Null) => Ok(Vec::new()),
                Some(a) => a
                    .as_array()
                    .ok_or_else(|| format!("vocal note: {k} is a list"))?
                    .iter()
                    .map(|c| c.as_str().and_then(Onset::from_text).ok_or_else(|| format!("vocal note: {k} holds a consonant")))
                    .collect(),
            }
        };
        let (start, dur) = (num("start")?, num("dur")?);
        if start < 0.0 || dur <= 0.0 {
            return Err("vocal note: start >= 0 and dur > 0".to_string());
        }
        Ok(VocalNote {
            start,
            dur,
            midi: midi as u8,
            word: v.get("word").and_then(JsonValue::as_str).unwrap_or("").to_string(),
            syl,
            pre: onsets("pre")?,
            post: onsets("post")?,
            accent: v.get("accent").and_then(JsonValue::as_bool).unwrap_or(false),
        })
    }

    /// The canonical JSON the CID is of.
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("format", JsonValue::int(FORMAT));
        o.set("type", TYPE.into());
        o.set("name", self.name.clone().into());
        o.set("lyrics", self.lyrics.clone().into());
        o.set("key", self.key.name().into());
        o.set("bpm", JsonValue::Number(self.bpm));
        o.set("bars", JsonValue::int(self.bars as i64));
        o.set("preset", self.preset.clone().into());
        o.set("seed", JsonValue::int(self.seed as i64));
        o.set("notes", JsonValue::Array(self.notes.iter().map(Self::note_json).collect()));
        self.rights.write_into(&mut o);
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<VocalLine, String> {
        if v.get("type").and_then(JsonValue::as_str) != Some(TYPE) {
            return Err(format!("not a vocal line (type {TYPE})"));
        }
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).map(str::to_string).ok_or_else(|| format!("vocal: {k}"));
        let key = Key::parse(&s("key")?).ok_or("vocal: a key")?;
        let bpm = v.get("bpm").and_then(JsonValue::as_f64).filter(|b| (20.0..=400.0).contains(b)).ok_or("vocal: a bpm (20 to 400)")?;
        let bars = v.get("bars").and_then(JsonValue::as_i64).filter(|b| (1..=32).contains(b)).ok_or("vocal: bars (1 to 32)")? as u32;
        let preset_name = s("preset")?;
        if preset(&preset_name).is_none() {
            return Err(format!("vocal: no voice preset {preset_name:?}"));
        }
        let notes = v
            .get("notes")
            .and_then(JsonValue::as_array)
            .ok_or("vocal: notes")?
            .iter()
            .map(Self::note_from_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(VocalLine {
            name: s("name")?,
            lyrics: s("lyrics")?,
            key,
            bpm,
            bars,
            preset: preset_name,
            seed: v.get("seed").and_then(JsonValue::as_i64).unwrap_or(0).max(0) as u32,
            notes,
            rights: Rights::from_json(v).map_err(|e| format!("vocal: {e}"))?,
        })
    }

    pub fn canonical(&self) -> String {
        self.to_json().canonical_js()
    }

    /// The content ID of the whole object, rights included.
    pub fn cid(&self) -> String {
        crate::cid::share_cid_canonical(&self.canonical())
    }

    /// A remix by `author` (`name` default "<name> (remix)"): the lineage and credits
    /// [`Rights::fork`] gives, the notes as they are.
    pub fn fork(&self, name: Option<&str>, author: Option<&str>) -> (VocalLine, Vec<String>) {
        let Fork { rights, warnings } = self.rights.fork(&self.cid(), Some(&self.name), author);
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| format!("{} (remix)", self.name), str::to_string);
        (VocalLine { name, rights, ..self.clone() }, warnings)
    }

    /// What may leave the machine by CID; a private line is refused.
    pub fn share(&self) -> Result<JsonValue, String> {
        self.rights.check_shareable().map_err(|e| format!("vocal {}: {e}", short_cid(&self.cid())))?;
        let mut o = JsonObject::new();
        o.set("cid", self.cid().into());
        o.set("object", self.to_json());
        Ok(JsonValue::Object(o))
    }

    /// For answers: the facts and the rights in words, no notes.
    pub fn summary_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("cid", self.cid().into());
        o.set("name", self.name.clone().into());
        o.set("lyrics", self.lyrics.clone().into());
        o.set("key", self.key.name().into());
        o.set("bpm", JsonValue::Number(self.bpm));
        o.set("bars", JsonValue::int(self.bars as i64));
        o.set("preset", self.preset.clone().into());
        o.set("notes", JsonValue::int(self.notes.len() as i64));
        o.set("rights", self.rights.to_json());
        JsonValue::Object(o)
    }

    /// The notes as text, for a person: `0.00 Bb3 ay (i)`.
    pub fn notes_text(&self) -> Vec<String> {
        self.notes.iter().map(|n| format!("{:6.2} {:<4} {:<9} {}", n.start, pitch_text(n.midi), n.syl.text(), n.word)).collect()
    }

    // MARK: sound

    /// One pass of the line as mono audio: exactly [`frames`](Self::frames) long, so it loops on
    /// the bar. What spills over the end (a cluster, a held note's release) wraps to the start,
    /// as a loop does. Peaks at about -1.4 dBFS. Deterministic.
    pub fn render(&self, sample_rate: f64) -> Vec<f32> {
        let len = self.frames(sample_rate);
        let pad = (sample_rate * 2.0) as usize;
        let mut buf = vec![0f32; len + 2 * pad];
        let beat = 60.0 / self.bpm * sample_rate;
        let params = preset(&self.preset).or_else(|| preset(DEFAULT_PRESET)).map(|p| p.params).unwrap_or_default();
        let mut synth = FormantSynth::new(sample_rate, params);
        synth.set_gate_times(4.0, 45.0);

        enum Ev {
            On(u8, Syllable),
            Off(u8),
        }
        let ms = |m: f64| (m * 0.001 * sample_rate) as usize;
        let mut evs: Vec<(usize, u8, Ev)> = Vec::new();
        for n in &self.notes {
            let t0 = pad + libm::round(n.start * beat) as usize;
            let t1 = t0 + libm::round(n.dur * beat).max(1.0) as usize;
            // the note is let go early by its coda's length: the consonant closes it
            let off = t1.saturating_sub(ms(n.syl.coda.duration_ms())).max(t0 + ms(30.0)).min(t1);
            evs.push((t0, 1, Ev::On(n.midi, n.syl)));
            evs.push((off, 0, Ev::Off(n.midi)));
            // cluster consonants just before the note, nearest first, each a little overlapped
            let mut cursor = t0;
            for (k, c) in n.pre.iter().enumerate().rev() {
                let d = ms(c.duration_ms());
                let at = cursor.saturating_sub(d);
                let key = (n.midi as usize + 1 + k).min(127) as u8;
                evs.push((at, 1, Ev::On(key, Syllable::consonant(*c))));
                evs.push((at + d, 0, Ev::Off(key)));
                cursor = at + d / 4;
            }
            // and after it
            let mut cursor = t1;
            for (k, c) in n.post.iter().enumerate() {
                let d = ms(c.duration_ms());
                let key = (n.midi as usize + 1 + n.pre.len() + k).min(127) as u8;
                evs.push((cursor, 1, Ev::On(key, Syllable::consonant(*c))));
                evs.push((cursor + d, 0, Ev::Off(key)));
                cursor += d * 3 / 4;
            }
        }
        evs.sort_by_key(|(t, order, _)| (*t, *order));
        let mut pos = 0usize;
        for (t, _, ev) in evs {
            let t = t.min(buf.len());
            if t > pos {
                synth.render(&mut buf[pos..t]);
                pos = t;
            }
            match ev {
                Ev::On(note, syl) => synth.note_on_syllable(note, Some(syl)),
                Ev::Off(note) => synth.note_off(note),
            }
        }
        if pos < buf.len() {
            synth.render(&mut buf[pos..]);
        }
        // fold the two pads back onto the loop
        let mut out = vec![0f32; len];
        for (i, o) in out.iter_mut().enumerate() {
            let mut s = buf[pad + i];
            if let Some(x) = buf.get(pad + i + len) {
                s += *x;
            }
            if pad + i >= len {
                s += buf[pad + i - len];
            }
            *o = s;
        }
        let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
        if peak > 1e-9 {
            let g = 0.85 / peak;
            for x in out.iter_mut() {
                *x *= g;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
