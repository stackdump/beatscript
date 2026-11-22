//! A recorded set (docs/RECORDING.md): the master output as an audio file, a timestamped
//! tracklist naming every track, mashup, loop set and transition by its CID, and the action-log
//! arrangement that replays it, as one canonical, content-addressed object.
//!
//! - **Canonical**: [`RecordedSet::to_json`] has sorted keys and no paths or wall-clock fields
//!   outside `started` (when it was played is part of what it is); [`RecordedSet::cid`] is the
//!   same `z…` share CID every other object here uses.
//! - **Replayable**: `arrangement` is the action log's own arrangement (schema
//!   `djrust.arrangement/1`) for the recorded bars: the decks it started on, the engine's state
//!   then and every move on its beat. `log play` plays it live, `djctl render` offline.
//! - **Tracklist**: derived from the arrangement ([`tracklist`]), so it cannot disagree with the
//!   replay, plus the host's marks for what the arrangement does not name (a loop set picked, a
//!   Perform move, an Auto-DJ or queue handover).
//! - **Audio**: the WAV's own content ID ([`raw_cid`], CIDv1 raw + sha2-256), frames and rate. A
//!   replay re-renders it; the CID says whether a render matches the take bit for bit.
//! - **Signed and forkable**: the signature is over the CID and lives beside the object (djhost
//!   signs with the booth's key); [`RecordedSet::fork`] makes an editable copy whose
//!   `forkedFrom` is this one, and `parents` lists every track / mashup CID it played.
//!
//! Pure: no I/O, `no_std` + `alloc`.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};
use crate::session::arrangement::{ArrangeOp, Arrangement};
use crate::session::model::DeckSource;

pub const SCHEMA: &str = "djrust.set-recording/1";

/// What a tracklist entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    /// A track on a deck (generated, a share, or audio).
    Track,
    /// A realised mashup on a deck.
    Mashup,
    /// A loop set (or one of its pairs) picked / loaded.
    LoopSet,
    /// A handover between decks: a crossfade, a Perform move, an Auto-DJ / queue transition.
    Transition,
}

impl EntryKind {
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::Track => "track",
            EntryKind::Mashup => "mashup",
            EntryKind::LoopSet => "loopset",
            EntryKind::Transition => "transition",
        }
    }

    pub fn parse(s: &str) -> Option<EntryKind> {
        Some(match s {
            "track" => EntryKind::Track,
            "mashup" => EntryKind::Mashup,
            "loopset" | "loop_set" | "loopsets" => EntryKind::LoopSet,
            "transition" => EntryKind::Transition,
            _ => return None,
        })
    }
}

/// One line of the tracklist.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Seconds from the recording's start.
    pub t: f64,
    /// Beats from the start (the arrangement's timeline).
    pub beat: f64,
    pub kind: EntryKind,
    /// "A", "B", or "" (a transition between them, a set-wide mark).
    pub deck: String,
    /// The content ID: a track's share CID, a mashup's CID, a sample's content id; empty for a
    /// transition with nothing of its own.
    pub cid: String,
    /// In words: "techno:7", "crossfade A → B", "bass_swap".
    pub label: String,
}

impl Entry {
    pub fn to_json(&self) -> JsonValue {
        let (bar, beat) = crate::session::arrangement::bar_beat(self.beat);
        JsonValue::Object(JsonObject::from_pairs([
            ("t", JsonValue::Number(round3(self.t))),
            ("beat", JsonValue::Number(round3(self.beat))),
            ("bar", JsonValue::Number(bar as f64)),
            ("beatInBar", JsonValue::Number(round3(beat))),
            ("time", JsonValue::String(clock(self.t))),
            ("kind", JsonValue::str(self.kind.name())),
            ("deck", JsonValue::String(self.deck.clone())),
            ("cid", JsonValue::String(self.cid.clone())),
            ("label", JsonValue::String(self.label.clone())),
        ]))
    }

    pub fn from_json(v: &JsonValue) -> Option<Entry> {
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).unwrap_or("").to_string();
        Some(Entry {
            t: v.get("t").and_then(JsonValue::as_f64)?,
            beat: v.get("beat").and_then(JsonValue::as_f64).unwrap_or(0.0),
            kind: EntryKind::parse(&s("kind"))?,
            deck: s("deck"),
            cid: s("cid"),
            label: s("label"),
        })
    }

    /// "12:34  A  track  techno:7  z…ab12".
    pub fn line(&self) -> String {
        format!("{}  {:<2} {:<10} {}  {}", clock(self.t), self.deck, self.kind.name(), self.label, self.cid)
    }
}

/// "m:ss" (or "h:mm:ss").
pub fn clock(t: f64) -> String {
    let s = libm::floor(t.max(0.0)) as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn round3(x: f64) -> f64 {
    libm::round(x * 1000.0) / 1000.0
}

/// What a deck source is, for the tracklist: (kind, cid, label). None: nothing (`none`).
pub fn source_entry(src: &DeckSource) -> Option<(EntryKind, String, String)> {
    match src {
        DeckSource::None => None,
        DeckSource::Net(spec) => {
            let payload = spec.payload();
            let label = match payload.get("genre").and_then(JsonValue::as_str) {
                Some(g) => format!("{g}:{}", spec.seed),
                None => format!("seed {}", spec.seed),
            };
            Some((EntryKind::Track, crate::cid::share_cid(&payload), label))
        }
        DeckSource::RawNets(p) => {
            let name = p.get("name").and_then(JsonValue::as_str).unwrap_or("track").to_string();
            match crate::mashup::mashup_of(p) {
                Some(m) => Some((EntryKind::Mashup, m.to_string(), name)),
                None => Some((EntryKind::Track, crate::cid::share_cid(p), name)),
            }
        }
        DeckSource::Sample { id, title, .. } => Some((EntryKind::Track, id.clone(), title.clone())),
    }
}

/// The tracklist an arrangement implies: what each deck held at the start, every load and swap
/// (a track or a mashup), and each crossfade from one side past the middle to the other. Sorted
/// by beat. Things the arrangement does not name (a loop set picked, a Perform move) come from
/// the host's marks, merged by [`merge`].
pub fn tracklist(arr: &Arrangement) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut held: BTreeMap<String, String> = BTreeMap::new();
    let push = |out: &mut Vec<Entry>, held: &mut BTreeMap<String, String>, beat: f64, deck: &str, src: &DeckSource| {
        if let Some((kind, cid, label)) = source_entry(src) {
            // a reload of what the deck already holds is not a new line
            if held.get(deck) == Some(&cid) {
                return;
            }
            held.insert(deck.to_string(), cid.clone());
            out.push(Entry { t: arr.seconds_at(beat), beat, kind, deck: deck.to_string(), cid, label });
        }
    };
    for (deck, src) in &arr.decks {
        push(&mut out, &mut held, 0.0, deck, src);
    }
    // the crossfader's side: -1 A, 0 the middle, +1 B (0..1, 0.5 the middle)
    let side = |x: f64| if x < 0.45 { -1 } else if x > 0.55 { 1 } else { 0 };
    let mut fader: Option<i32> = None;
    for e in &arr.events {
        match &e.op {
            ArrangeOp::Deck(crate::session::action::DeckAction::Load { deck, source }) => {
                push(&mut out, &mut held, e.at, deck, source)
            }
            ArrangeOp::Swap { deck, source, .. } => push(&mut out, &mut held, e.at, deck, source),
            ArrangeOp::Deck(crate::session::action::DeckAction::Crossfader(x)) => {
                let s = side(*x);
                if s != 0 {
                    if let Some(prev) = fader.filter(|p| *p != s) {
                        let (from, to) = if prev < 0 { ("A", "B") } else { ("B", "A") };
                        out.push(Entry {
                            t: arr.seconds_at(e.at),
                            beat: e.at,
                            kind: EntryKind::Transition,
                            deck: String::new(),
                            cid: String::new(),
                            label: format!("crossfade {from} → {to}"),
                        });
                    }
                    fader = Some(s);
                }
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(core::cmp::Ordering::Equal));
    out
}

/// The derived tracklist with the host's marks, by time. A mark for what a derived line already
/// names (the same deck and CID within 2 s: a loop-set pad loads a track) relabels that line with
/// the mark's kind and words instead of adding one; any other mark is added.
pub fn merge(mut derived: Vec<Entry>, marks: &[Entry]) -> Vec<Entry> {
    for m in marks {
        if let Some(d) = derived
            .iter_mut()
            .find(|d| !m.cid.is_empty() && d.cid == m.cid && d.deck == m.deck && (d.t - m.t).abs() < 2.0)
        {
            d.kind = m.kind;
            if !m.label.is_empty() {
                d.label = m.label.clone();
            }
            continue;
        }
        let dup = derived.iter().any(|d| d.kind == m.kind && d.label == m.label && (d.t - m.t).abs() < 0.05);
        if !dup {
            derived.push(m.clone());
        }
    }
    derived.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(core::cmp::Ordering::Equal));
    derived
}

/// Entries timed on the arrangement (from the downbeat of the first recorded bar) moved onto the
/// audio's clock: `offset` seconds of the arrangement passed before the audio began. Nothing goes
/// below 0 (what the decks held at the start is at 0:00 of the take).
pub fn on_audio_clock(mut v: Vec<Entry>, offset: f64) -> Vec<Entry> {
    for e in v.iter_mut() {
        e.t = (e.t - offset).max(0.0);
    }
    v
}

/// The recorded audio.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioRef {
    /// The file's content ID ([`raw_cid`] of its bytes).
    pub cid: String,
    /// The file name it was written as (no directory: paths are not content).
    pub file: String,
    pub frames: u64,
    pub sample_rate: f64,
    pub channels: u32,
    /// "wav-f32", "wav-s16".
    pub format: String,
}

impl AudioRef {
    pub fn seconds(&self) -> f64 {
        if self.sample_rate > 0.0 {
            self.frames as f64 / self.sample_rate
        } else {
            0.0
        }
    }

    pub fn to_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("cid", JsonValue::String(self.cid.clone())),
            ("file", JsonValue::String(self.file.clone())),
            ("frames", JsonValue::Number(self.frames as f64)),
            ("sampleRate", JsonValue::Number(self.sample_rate)),
            ("channels", JsonValue::Number(self.channels as f64)),
            ("format", JsonValue::String(self.format.clone())),
        ]))
    }

    pub fn from_json(v: &JsonValue) -> Option<AudioRef> {
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).map(str::to_string);
        let n = |k: &str| v.get(k).and_then(JsonValue::as_f64);
        Some(AudioRef {
            cid: s("cid")?,
            file: s("file").unwrap_or_default(),
            frames: n("frames")? as u64,
            sample_rate: n("sampleRate")?,
            channels: n("channels").unwrap_or(2.0) as u32,
            format: s("format").unwrap_or_else(|| "wav-f32".into()),
        })
    }
}

/// CIDv1, raw codec (0x55), sha2-256, base58btc `z…`: the content ID of a file's bytes.
pub fn raw_cid(bytes: &[u8]) -> String {
    raw_cid_of_digest(&crate::cid::sha256(bytes))
}

/// [`raw_cid`] from a digest computed elsewhere (a file hashed as it was written).
pub fn raw_cid_of_digest(digest: &[u8; 32]) -> String {
    let mut v = alloc::vec![0x01u8, 0x55, 0x12, 0x20];
    v.extend_from_slice(digest);
    let mut s = String::from("z");
    s.push_str(&crate::cid::encode_base58(&v));
    s
}

/// A recorded set.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedSet {
    pub name: String,
    /// When it was played (ISO 8601, UTC), as the host stamps it; part of the content.
    pub started: String,
    /// The master tempo at the start.
    pub bpm: f64,
    /// Seconds recorded.
    pub duration: f64,
    pub audio: Option<AudioRef>,
    pub tracklist: Vec<Entry>,
    /// The replay: an arrangement (`djrust.arrangement/1`) of the recorded bars.
    pub arrangement: JsonValue,
    /// The bars of the action log it covers (1-based, inclusive).
    pub bars: (i64, i64),
    /// Seconds of the arrangement (from its first bar's downbeat) before the audio began: the
    /// tracklist's times are on the audio's clock, the replay's beats on the arrangement's.
    pub audio_offset: f64,
    pub forked_from: Option<String>,
}

impl RecordedSet {
    /// Every track and mashup CID it played, in order, no repeats: the lineage edges.
    pub fn parents(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for e in &self.tracklist {
            if matches!(e.kind, EntryKind::Track | EntryKind::Mashup | EntryKind::LoopSet)
                && !e.cid.is_empty()
                && !v.contains(&e.cid)
            {
                v.push(e.cid.clone());
            }
        }
        v
    }

    /// The canonical JSON the CID is of.
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("schema", JsonValue::str(SCHEMA));
        o.set("name", JsonValue::String(self.name.clone()));
        o.set("started", JsonValue::String(self.started.clone()));
        o.set("bpm", JsonValue::Number(round3(self.bpm)));
        o.set("duration", JsonValue::Number(round3(self.duration)));
        o.set("audio", self.audio.as_ref().map(AudioRef::to_json).unwrap_or(JsonValue::Null));
        o.set("tracklist", JsonValue::Array(self.tracklist.iter().map(Entry::to_json).collect()));
        o.set("arrangement", self.arrangement.clone());
        o.set("audioOffset", JsonValue::Number(round3(self.audio_offset)));
        o.set(
            "bars",
            JsonValue::Array(alloc::vec![JsonValue::Number(self.bars.0 as f64), JsonValue::Number(self.bars.1 as f64)]),
        );
        o.set("parents", JsonValue::Array(self.parents().into_iter().map(JsonValue::String).collect()));
        if let Some(f) = &self.forked_from {
            o.set("forkedFrom", JsonValue::String(f.clone()));
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<RecordedSet, String> {
        if v.get("schema").and_then(JsonValue::as_str) != Some(SCHEMA) {
            return Err(format!("record: not a recorded set (schema {SCHEMA})"));
        }
        let s = |k: &str| v.get(k).and_then(JsonValue::as_str).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(JsonValue::as_f64).unwrap_or(0.0);
        let bars = v.get("bars").and_then(JsonValue::as_array).map(|a| {
            (a.first().and_then(JsonValue::as_f64).unwrap_or(1.0) as i64, a.get(1).and_then(JsonValue::as_f64).unwrap_or(1.0) as i64)
        });
        Ok(RecordedSet {
            name: s("name"),
            started: s("started"),
            bpm: n("bpm"),
            duration: n("duration"),
            audio: v.get("audio").and_then(AudioRef::from_json),
            tracklist: v
                .get("tracklist")
                .and_then(JsonValue::as_array)
                .map(|a| a.iter().filter_map(Entry::from_json).collect())
                .unwrap_or_default(),
            arrangement: v.get("arrangement").cloned().unwrap_or(JsonValue::Null),
            bars: bars.unwrap_or((1, 1)),
            audio_offset: n("audioOffset"),
            forked_from: v.get("forkedFrom").and_then(JsonValue::as_str).map(str::to_string),
        })
    }

    pub fn canonical(&self) -> String {
        self.to_json().canonical_js()
    }

    pub fn cid(&self) -> String {
        crate::cid::share_cid_canonical(&self.canonical())
    }

    /// The arrangement, parsed, to replay.
    pub fn replay(&self) -> Option<Arrangement> {
        Arrangement::from_json(&self.arrangement).map(|(a, _)| a)
    }

    /// A copy to edit and perform again: the same replay and tracklist, no audio (a fork is a
    /// new take), pointing back at this one.
    pub fn fork(&self, name: Option<&str>) -> RecordedSet {
        let mut f = self.clone();
        f.forked_from = Some(self.cid());
        f.audio = None;
        if let Some(n) = name {
            f.name = n.to_string();
        }
        f
    }

    /// The tracklist as text, one line each (a cue sheet).
    pub fn cue_sheet(&self) -> String {
        let mut s = format!("{}  ({}, {})\n", self.name, clock(self.duration), self.cid());
        for e in &self.tracklist {
            s.push_str(&e.line());
            s.push('\n');
        }
        s
    }

    /// For lists: name, cid, started, duration, entries, audio cid.
    pub fn summary_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("cid", JsonValue::String(self.cid())),
            ("name", JsonValue::String(self.name.clone())),
            ("started", JsonValue::String(self.started.clone())),
            ("duration", JsonValue::Number(round3(self.duration))),
            ("time", JsonValue::String(clock(self.duration))),
            ("entries", JsonValue::Number(self.tracklist.len() as f64)),
            ("audio", self.audio.as_ref().map(|a| JsonValue::String(a.cid.clone())).unwrap_or(JsonValue::Null)),
            ("forkedFrom", self.forked_from.clone().map(JsonValue::String).unwrap_or(JsonValue::Null)),
        ]))
    }
}
