//! Where a mashup's parts come from (docs/MASHUPS.md, "Stem sources"): a [`StemSource`] lists the
//! stems of a track and hands them over.
//!
//! - [`NetStems`] (built in): a generated track's stems are its nets. Every instrument of a
//!   beats / djrust track is its own Petri net, so splitting one is exact and free: the drum nets
//!   (kick, snare, hats, …) are the `drums` stem, and `bass`, `harmony`, `melody` are theirs. A
//!   stem is net data, so it can be re-tempoed and transposed losslessly.
//! - [`DemucsStems`] (stub): audio source separation for external audio, to come from the
//!   pure-Rust Demucs port in Burn being planned in `demucs-rs` (stackdump/demucs-rs). Nothing here depends on
//!   it: the stub says it is unavailable and lists the four stems (`drums`, `bass`, `other`,
//!   `vocals`) the htdemucs model will give. A future `stems` feature implements
//!   [`StemSource::audio_stem`] for it and the rest of the mashup code is unchanged.
//!
//! Pure: no I/O, `no_std` + `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};
use crate::pflow::{NetBundle, Project};

/// What a stem is, so slots can be matched across sources (drums from A, bass from B).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StemClass {
    Drums,
    Bass,
    Harmony,
    Melody,
    Vocals,
    Other,
}

impl StemClass {
    pub fn name(self) -> &'static str {
        match self {
            StemClass::Drums => "drums",
            StemClass::Bass => "bass",
            StemClass::Harmony => "harmony",
            StemClass::Melody => "melody",
            StemClass::Vocals => "vocals",
            StemClass::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<StemClass> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "drums" | "drum" | "beat" | "percussion" => StemClass::Drums,
            "bass" => StemClass::Bass,
            "harmony" | "chords" | "pad" | "keys" => StemClass::Harmony,
            "melody" | "lead" | "hook" | "arp" => StemClass::Melody,
            "vocals" | "vocal" | "vox" => StemClass::Vocals,
            "other" => StemClass::Other,
            _ => return None,
        })
    }

    /// Does a stem of this class carry pitch (and so need transposing into key)?
    pub fn tonal(self) -> bool {
        !matches!(self, StemClass::Drums)
    }
}

/// Is a net a drum part: a drum channel (10-15, `isDrumChannel`) or a drum net name.
pub fn is_drum_net(id: &str, n: &NetBundle) -> bool {
    crate::wave::synth::is_drum_channel(n.track.channel)
        || ["kick", "snare", "hat", "hihat", "clap", "cymbal", "tom", "drum", "perc"].iter().any(|p| id.starts_with(p))
}

/// The class a net of a generated track belongs to.
pub fn net_class(id: &str, n: &NetBundle) -> StemClass {
    if is_drum_net(id, n) {
        return StemClass::Drums;
    }
    let l = id.to_ascii_lowercase();
    if l.starts_with("bass") || l.contains("bass") {
        StemClass::Bass
    } else if l.starts_with("harmony") || l.starts_with("pad") || l.starts_with("chord") {
        StemClass::Harmony
    } else if l.starts_with("melody") || l.starts_with("lead") || l.starts_with("arp") || l.starts_with("hook") {
        StemClass::Melody
    } else if l.starts_with("vox") || l.starts_with("vocal") {
        StemClass::Vocals
    } else {
        StemClass::Other
    }
}

/// One stem a source offers.
#[derive(Clone, Debug, PartialEq)]
pub struct StemInfo {
    /// What a mashup part names it by: the class name, or the net's id for an `other` net.
    pub id: String,
    pub class: StemClass,
    /// The nets it is made of (net stems), empty for audio.
    pub nets: Vec<String>,
}

impl StemInfo {
    pub fn json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("id", JsonValue::String(self.id.clone())),
            ("class", JsonValue::str(self.class.name())),
            ("tonal", JsonValue::Bool(self.class.tonal())),
            ("nets", JsonValue::Array(self.nets.iter().map(|n| JsonValue::String(n.clone())).collect())),
        ]))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StemError {
    /// This source cannot split that kind of track (a net splitter handed audio).
    Unsupported(String),
    /// The splitter is not built in or not ready (the Demucs stub).
    Unavailable(String),
    /// No such stem.
    NotFound(String),
}

impl core::fmt::Display for StemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StemError::Unsupported(s) | StemError::Unavailable(s) | StemError::NotFound(s) => f.write_str(s),
        }
    }
}

/// Audio of one stem: equal-length channels at a rate.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioStem {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: f64,
}

/// Splits a track into stems. Net stems are net data ([`StemSource::net_stem`]); audio stems are
/// sample buffers ([`StemSource::audio_stem`]). A splitter implements the one its kind has.
pub trait StemSource {
    /// "nets", "demucs".
    fn kind(&self) -> &'static str;
    /// Ready to split now.
    fn available(&self) -> bool;
    /// The stems of a net track (its project) or of audio (`None`: the splitter decides from
    /// what it was given at construction).
    fn stems(&self, project: Option<&Project>) -> Result<Vec<StemInfo>, StemError>;
    /// A net stem: its nets, by id, as they are in `project`.
    fn net_stem(&self, _project: &Project, stem: &str) -> Result<Vec<(String, NetBundle)>, StemError> {
        Err(StemError::Unsupported(format!("{}: no net stems ({stem})", self.kind())))
    }
    /// An audio stem of `audio` (mono or stereo channels at `sample_rate`).
    fn audio_stem(&self, _audio: &[&[f32]], _sample_rate: f64, stem: &str) -> Result<AudioStem, StemError> {
        Err(StemError::Unsupported(format!("{}: no audio stems ({stem})", self.kind())))
    }
}

/// A generated track's stems are its nets.
#[derive(Clone, Copy, Debug, Default)]
pub struct NetStems;

impl NetStems {
    /// The music nets of a loop project (no control nets, no stingers), by class.
    fn grouped(project: &Project) -> Vec<StemInfo> {
        let mut out: Vec<StemInfo> = Vec::new();
        for (id, n) in project.nets.iter() {
            if n.role == "control" || n.track.group.as_deref() == Some("stinger") {
                continue;
            }
            let class = net_class(id, n);
            let sid = if class == StemClass::Other { id.to_string() } else { class.name().to_string() };
            match out.iter_mut().find(|s| s.id == sid) {
                Some(s) => s.nets.push(id.to_string()),
                None => out.push(StemInfo { id: sid, class, nets: alloc::vec![id.to_string()] }),
            }
        }
        out.sort_by(|a, b| a.class.cmp(&b.class).then(a.id.cmp(&b.id)));
        out
    }
}

impl StemSource for NetStems {
    fn kind(&self) -> &'static str {
        "nets"
    }
    fn available(&self) -> bool {
        true
    }
    fn stems(&self, project: Option<&Project>) -> Result<Vec<StemInfo>, StemError> {
        let p = project.ok_or_else(|| StemError::Unsupported("nets: audio has no nets (use a stem separator)".into()))?;
        Ok(Self::grouped(p))
    }
    fn net_stem(&self, project: &Project, stem: &str) -> Result<Vec<(String, NetBundle)>, StemError> {
        let s = Self::grouped(project)
            .into_iter()
            .find(|s| s.id == stem || StemClass::parse(stem) == Some(s.class) && s.class != StemClass::Other)
            .ok_or_else(|| {
                let have: Vec<String> = Self::grouped(project).into_iter().map(|s| s.id).collect();
                StemError::NotFound(format!("no stem {stem:?} (it has {})", have.join(", ")))
            })?;
        Ok(s.nets.iter().filter_map(|id| project.nets.get(id).map(|n| (id.clone(), n.clone()))).collect())
    }
}

/// The stems htdemucs gives (the model `demucs-rs` ports).
pub const DEMUCS_STEMS: [&str; 4] = ["drums", "bass", "other", "vocals"];

/// Audio source separation for external audio: a stub until `demucs-rs` is linked.
#[derive(Clone, Copy, Debug, Default)]
pub struct DemucsStems;

impl DemucsStems {
    pub const UNAVAILABLE: &'static str =
        "demucs: audio stem separation is not built in yet (the pure-Rust Demucs port, demucs-rs, is not linked yet; a `stems` feature will plug it in here)";
}

impl StemSource for DemucsStems {
    fn kind(&self) -> &'static str {
        "demucs"
    }
    fn available(&self) -> bool {
        false
    }
    fn stems(&self, project: Option<&Project>) -> Result<Vec<StemInfo>, StemError> {
        if project.is_some() {
            return Err(StemError::Unsupported("demucs: a generated track splits by its nets (NetStems)".into()));
        }
        // what it will give, so a UI can show the slots; asking for one is the error
        Ok(DEMUCS_STEMS
            .iter()
            .map(|s| StemInfo { id: s.to_string(), class: StemClass::parse(s).unwrap_or(StemClass::Other), nets: Vec::new() })
            .collect())
    }
    fn audio_stem(&self, _audio: &[&[f32]], _sample_rate: f64, _stem: &str) -> Result<AudioStem, StemError> {
        Err(StemError::Unavailable(Self::UNAVAILABLE.into()))
    }
}

/// The stem sources this build has, for `mashup sources`.
pub fn sources_json() -> JsonValue {
    let one = |s: &dyn StemSource, what: &str| {
        JsonValue::Object(JsonObject::from_pairs([
            ("kind", JsonValue::str(s.kind())),
            ("available", JsonValue::Bool(s.available())),
            ("splits", JsonValue::str(what)),
        ]))
    };
    JsonValue::Array(alloc::vec![
        one(&NetStems, "generated tracks (every instrument is its own net)"),
        one(&DemucsStems, "external audio (stub: demucs-rs)"),
    ])
}
