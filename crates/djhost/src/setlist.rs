//! Set lists (beyond Swift; docs/SETLIST.md): a whole DJ set built in advance. A [`SetList`] is an
//! ordered list of [`Segment`]s: a [`Source`] (a loop-set pair like `dnb:82819` group 3, a net
//! deck's seed / spec, or a saved track's CID) held for some bars, then a [`Transition`] into the
//! next segment (one of the Perform moves, `djhost::perform`), optional [`InsertAuto`] (a Master
//! filter sweep or delay throw over N bars) and an optional [`Tag`] (energy, genre).
//!
//! A set is plain data. It has a canonical JSON ([`SetList::to_json`]: sorted keys, no
//! timestamps) and so a content ID ([`SetList::cid`], the same `z…` CID the shares use): two sets
//! with the same content are the same set. [`store::SetStore`] keeps them as `<cid>.json` in the
//! app-support folder. [`generate`] makes one from a [`Recipe`], deterministically from its seed
//! and only from complementary A / B pairs (a group's own A and B, never one group's A with
//! another's B). The queue that plays them is [`crate::setqueue`].

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{Fork, License, Rights, Visibility};

use crate::perform::MoveKind;

pub mod recipe;
pub mod store;

pub use recipe::{generate, BankInfo, Recipe};

/// The shortest and longest a segment can be, in bars.
pub const MIN_BARS: u32 = 4;
pub const MAX_BARS: u32 = 128;
pub const DEFAULT_BARS: u32 = 16;
/// The longest set, in segments (a guard on the generator and the edit verbs).
pub const MAX_SEGMENTS: usize = 256;

pub fn clamp_bars(n: i64) -> u32 {
    n.clamp(MIN_BARS as i64, MAX_BARS as i64) as u32
}

/// What a segment plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Group `group` of loop-set `spec` ("dnb:82819"): its A on deck A and its B on deck B (the
    /// Loops window's A + B pad).
    Pair { spec: String, group: u32 },
    /// A net deck's seed / spec text (`genre[:seed[:structure]]`, as `deck load spec=`).
    Net { spec: String },
    /// A saved composition, by its content ID (the Library's).
    Track { cid: String },
}

impl Source {
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Pair { .. } => "pair",
            Source::Net { .. } => "net",
            Source::Track { .. } => "track",
        }
    }

    /// For lists and the window: "dnb:82819 #3", "dnb:123", "track zQm…".
    pub fn text(&self) -> String {
        match self {
            Source::Pair { spec, group } => format!("{spec} #{group}"),
            Source::Net { spec } => spec.clone(),
            Source::Track { cid } => format!("track {}", short_cid(cid)),
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("kind", self.kind().into());
        match self {
            Source::Pair { spec, group } => {
                o.set("spec", spec.clone().into());
                o.set("group", JsonValue::int(*group as i64));
            }
            Source::Net { spec } => o.set("spec", spec.clone().into()),
            Source::Track { cid } => o.set("cid", cid.clone().into()),
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Source, String> {
        let kind = v.get("kind").and_then(JsonValue::as_str).ok_or("source: a kind (pair, net or track)")?;
        let text = |k: &str| -> Result<String, String> {
            v.get(k)
                .and_then(JsonValue::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("source {kind}: needs {k}"))
        };
        match kind {
            "pair" => {
                let group = v
                    .get("group")
                    .and_then(JsonValue::as_i64)
                    .filter(|g| (1..=64).contains(g))
                    .ok_or("source pair: group is a number 1-64")?;
                Ok(Source::Pair { spec: text("spec")?.to_ascii_lowercase(), group: group as u32 })
            }
            "net" => Ok(Source::Net { spec: text("spec")? }),
            "track" => Ok(Source::Track { cid: text("cid")? }),
            other => Err(format!("source kind {other:?}: pair, net or track")),
        }
    }

    /// `pair:dnb:82819:3`, `net:dnb:123`, `track:zQm…`, or `dnb:82819#3` / `dnb:82819 3` for a
    /// pair: how a caller types one on a line.
    pub fn parse(text: &str) -> Result<Source, String> {
        let t = text.trim();
        if let Some(r) = t.strip_prefix("track:") {
            return Ok(Source::Track { cid: r.trim().to_string() });
        }
        if let Some(r) = t.strip_prefix("net:") {
            return Ok(Source::Net { spec: r.trim().to_string() });
        }
        let t = t.strip_prefix("pair:").unwrap_or(t);
        let (spec, group) = if let Some((s, g)) = t.rsplit_once('#') {
            (s, g)
        } else if let Some((s, g)) = t.rsplit_once(' ') {
            (s, g)
        } else if let Some((s, g)) = t.rsplit_once(':').filter(|(s, _)| s.contains(':')) {
            (s, g)
        } else {
            return Err(format!("source {text:?}: a pair is `spec#group` (dnb:82819#3), a net `net:spec`, a track `track:cid`"));
        };
        let group: u32 = group.trim().parse().map_err(|_| format!("source {text:?}: the group is a number"))?;
        Ok(Source::Pair { spec: spec.trim().to_ascii_lowercase(), group })
    }
}

fn short_cid(cid: &str) -> String {
    let n = cid.chars().count();
    if n <= 12 {
        cid.to_string()
    } else {
        format!("{}…{}", cid.chars().take(6).collect::<String>(), cid.chars().skip(n - 4).collect::<String>())
    }
}

/// How a segment hands over to the next: a Perform move (`cut` is the bare handover on the
/// phrase line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Cut,
    Move(MoveKind),
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 7] = [
        TransitionKind::Cut,
        TransitionKind::Move(MoveKind::BassSwap),
        TransitionKind::Move(MoveKind::FilterExit),
        TransitionKind::Move(MoveKind::EchoOut),
        TransitionKind::Move(MoveKind::LoopRoll),
        TransitionKind::Move(MoveKind::PhraseSync),
        TransitionKind::Move(MoveKind::VocoderHandoff),
    ];

    /// What a generated set draws from (the vocoder handoff needs a vocoder running).
    pub const VARIETY: [TransitionKind; 5] = [
        TransitionKind::Move(MoveKind::FilterExit),
        TransitionKind::Move(MoveKind::EchoOut),
        TransitionKind::Move(MoveKind::BassSwap),
        TransitionKind::Move(MoveKind::LoopRoll),
        TransitionKind::Move(MoveKind::PhraseSync),
    ];

    pub fn name(self) -> &'static str {
        match self {
            TransitionKind::Cut => "cut",
            TransitionKind::Move(m) => m.name(),
        }
    }

    pub fn parse(s: &str) -> Option<TransitionKind> {
        let t = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        if t == "cut" || t == "none" {
            return Some(TransitionKind::Cut);
        }
        MoveKind::parse(&t).map(TransitionKind::Move)
    }

    pub fn names() -> Vec<&'static str> {
        TransitionKind::ALL.iter().map(|k| k.name()).collect()
    }

    /// The move's `bars` when none is given.
    pub fn default_bars(self) -> u32 {
        match self {
            TransitionKind::Cut => 1,
            TransitionKind::Move(m) => m.default_bars(),
        }
    }
}

/// The transition out of a segment: a kind and its `bars` (a Perform move's, see
/// [`MoveKind::bars_meaning`]: the sweep of a filter exit, the tail of an echo out, the bars per
/// step of a loop roll, the phrase of a bass swap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub kind: TransitionKind,
    pub bars: u32,
}

impl Transition {
    pub fn new(kind: TransitionKind) -> Transition {
        Transition { kind, bars: kind.default_bars() }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("kind", self.kind.name().into());
        o.set("bars", JsonValue::int(self.bars as i64));
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Transition, String> {
        let name = v.get("kind").and_then(JsonValue::as_str).ok_or("transition: a kind")?;
        let kind = TransitionKind::parse(name)
            .ok_or_else(|| format!("transition {name:?}: one of {}", TransitionKind::names().join(", ")))?;
        let bars = match v.get("bars") {
            None | Some(JsonValue::Null) => kind.default_bars(),
            Some(b) => b.as_i64().filter(|n| (1..=32).contains(n)).ok_or("transition: bars is a number 1-32")? as u32,
        };
        Ok(Transition { kind, bars })
    }

    pub fn text(&self) -> String {
        match self.kind {
            TransitionKind::Cut => "cut".into(),
            TransitionKind::Move(m) => format!("{} ({} {})", m.label(), self.bars, if self.bars == 1 { "bar" } else { "bars" }),
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        Transition::new(TransitionKind::Move(MoveKind::PhraseSync))
    }
}

/// What an insert automation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertKind {
    /// The Master's filter ridden from open to `amount` (0..1: left of 0.5 a low-pass, right a
    /// high-pass) over the last `bars` of the segment, back open after the line.
    FilterSweep,
    /// The Master's delay thrown: its mix (and feedback) pushed to `amount` over the last `bars`,
    /// let back down after the line.
    DelayThrow,
}

impl InsertKind {
    pub const ALL: [InsertKind; 2] = [InsertKind::FilterSweep, InsertKind::DelayThrow];

    pub fn name(self) -> &'static str {
        match self {
            InsertKind::FilterSweep => "filter_sweep",
            InsertKind::DelayThrow => "delay_throw",
        }
    }

    pub fn parse(s: &str) -> Option<InsertKind> {
        let t = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match t.as_str() {
            "filter" => Some(InsertKind::FilterSweep),
            "delay" => Some(InsertKind::DelayThrow),
            _ => InsertKind::ALL.into_iter().find(|k| k.name() == t),
        }
    }

    pub fn default_amount(self) -> f64 {
        match self {
            InsertKind::FilterSweep => 0.14,
            InsertKind::DelayThrow => 0.5,
        }
    }
}

/// An insert automation on a segment's exit: it ends on the segment's last line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsertAuto {
    pub kind: InsertKind,
    /// Bars the ramp takes, ending on the line (1-32).
    pub bars: u32,
    /// 0..1, to what (see [`InsertKind`]), rounded to 2 places so the JSON round-trips.
    pub amount: f64,
}

pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl InsertAuto {
    pub fn new(kind: InsertKind, bars: u32, amount: f64) -> InsertAuto {
        InsertAuto { kind, bars: bars.clamp(1, 32), amount: round2(amount.clamp(0.0, 1.0)) }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("kind", self.kind.name().into());
        o.set("bars", JsonValue::int(self.bars as i64));
        o.set("amount", JsonValue::Number(self.amount));
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<InsertAuto, String> {
        let name = v.get("kind").and_then(JsonValue::as_str).ok_or("insert: a kind (filter_sweep or delay_throw)")?;
        let kind = InsertKind::parse(name).ok_or_else(|| format!("insert {name:?}: filter_sweep or delay_throw"))?;
        let bars = match v.get("bars") {
            None | Some(JsonValue::Null) => 4,
            Some(b) => b.as_i64().filter(|n| (1..=32).contains(n)).ok_or("insert: bars is a number 1-32")? as u32,
        };
        let amount = match v.get("amount") {
            None | Some(JsonValue::Null) => kind.default_amount(),
            Some(a) => a.as_f64().filter(|x| (0.0..=1.0).contains(x)).ok_or("insert: amount is a number 0-1")?,
        };
        Ok(InsertAuto::new(kind, bars, amount))
    }

    pub fn text(&self) -> String {
        format!("{} {} bars to {}", self.kind.name(), self.bars, self.amount)
    }
}

/// Free labels on a segment, for planning and the window.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Tag {
    /// 1 (easy) to 5 (peak).
    pub energy: Option<u8>,
    pub genre: Option<String>,
}

impl Tag {
    pub fn is_empty(&self) -> bool {
        self.energy.is_none() && self.genre.is_none()
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        if let Some(e) = self.energy {
            o.set("energy", JsonValue::int(e as i64));
        }
        if let Some(g) = &self.genre {
            o.set("genre", g.clone().into());
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Tag, String> {
        let energy = match v.get("energy") {
            None | Some(JsonValue::Null) => None,
            Some(e) => Some(e.as_i64().filter(|n| (1..=5).contains(n)).ok_or("tag: energy is a number 1-5")? as u8),
        };
        let genre = v.get("genre").and_then(JsonValue::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Ok(Tag { energy, genre })
    }

    pub fn text(&self) -> String {
        match (self.energy, &self.genre) {
            (Some(e), Some(g)) => format!("{g} · energy {e}"),
            (Some(e), None) => format!("energy {e}"),
            (None, Some(g)) => g.clone(),
            _ => String::new(),
        }
    }
}

/// One segment of a set.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub source: Source,
    /// How long it plays before the transition's line, `MIN_BARS..=MAX_BARS`.
    pub bars: u32,
    /// The hand-over into the next segment (the last segment's is used when the queue loops).
    pub transition: Transition,
    pub inserts: Vec<InsertAuto>,
    pub tag: Tag,
}

impl Segment {
    pub fn new(source: Source, bars: u32) -> Segment {
        Segment { source, bars: clamp_bars(bars as i64), transition: Transition::default(), inserts: Vec::new(), tag: Tag::default() }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("source", self.source.to_json());
        o.set("bars", JsonValue::int(self.bars as i64));
        o.set("transition", self.transition.to_json());
        if !self.inserts.is_empty() {
            o.set("inserts", JsonValue::Array(self.inserts.iter().map(InsertAuto::to_json).collect()));
        }
        if !self.tag.is_empty() {
            o.set("tag", self.tag.to_json());
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Segment, String> {
        let source = Source::from_json(v.get("source").ok_or("segment: a source")?)?;
        let bars = match v.get("bars") {
            None | Some(JsonValue::Null) => DEFAULT_BARS,
            Some(b) => b
                .as_i64()
                .filter(|n| (MIN_BARS as i64..=MAX_BARS as i64).contains(n))
                .ok_or_else(|| format!("segment: bars is a number {MIN_BARS}-{MAX_BARS}"))? as u32,
        };
        let transition = match v.get("transition") {
            None | Some(JsonValue::Null) => Transition::default(),
            Some(t) => Transition::from_json(t)?,
        };
        let inserts = match v.get("inserts") {
            None | Some(JsonValue::Null) => Vec::new(),
            Some(a) => a.as_array().ok_or("segment: inserts is a list")?.iter().map(InsertAuto::from_json).collect::<Result<_, _>>()?,
        };
        let tag = match v.get("tag") {
            None | Some(JsonValue::Null) => Tag::default(),
            Some(t) => Tag::from_json(t)?,
        };
        Ok(Segment { source, bars, transition, inserts, tag })
    }

    /// One line, for the window and `list`.
    pub fn text(&self) -> String {
        let mut s = format!("{} · {} bars → {}", self.source.text(), self.bars, self.transition.text());
        for i in &self.inserts {
            s.push_str(&format!(" + {}", i.text()));
        }
        if !self.tag.is_empty() {
            s.push_str(&format!(" [{}]", self.tag.text()));
        }
        s
    }
}

/// The set.
#[derive(Clone, Debug, PartialEq)]
pub struct SetList {
    pub name: String,
    pub note: String,
    pub segments: Vec<Segment>,
    /// The license, visibility, author and lineage (docs/LICENSING.md), in the canonical JSON
    /// and so in the CID. `None`: a set saved before sets had rights; it reads as CC0 and public
    /// ([`SetList::rights`]) and keeps the CID it was saved under until its rights are set.
    pub rights: Option<Rights>,
}

/// The file format's version (in the JSON, so a later change can read an older file).
pub const FORMAT: i64 = 1;

impl SetList {
    pub fn new(name: &str) -> SetList {
        SetList { name: name.trim().to_string(), note: String::new(), segments: Vec::new(), rights: Some(Rights::new(None)) }
    }

    /// The canonical JSON the content ID is of: no times, no paths, nothing that is not the set.
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("format", JsonValue::int(FORMAT));
        o.set("type", "djrust.setlist".into());
        o.set("name", self.name.clone().into());
        if !self.note.is_empty() {
            o.set("note", self.note.clone().into());
        }
        o.set("segments", JsonValue::Array(self.segments.iter().map(Segment::to_json).collect()));
        if let Some(r) = &self.rights {
            r.write_into(&mut o);
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<SetList, String> {
        if v.get("type").and_then(JsonValue::as_str).is_some_and(|t| t != "djrust.setlist") {
            return Err("not a set list (type djrust.setlist)".into());
        }
        let name = v.get("name").and_then(JsonValue::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("setlist: a name")?;
        let note = v.get("note").and_then(JsonValue::as_str).unwrap_or("").to_string();
        let arr = v.get("segments").and_then(JsonValue::as_array).ok_or("setlist: segments (a list)")?;
        if arr.len() > MAX_SEGMENTS {
            return Err(format!("setlist: at most {MAX_SEGMENTS} segments"));
        }
        let segments = arr
            .iter()
            .enumerate()
            .map(|(i, s)| Segment::from_json(s).map_err(|e| format!("segment {}: {e}", i + 1)))
            .collect::<Result<Vec<_>, _>>()?;
        let rights = if Rights::present_in(v) { Some(Rights::from_json(v).map_err(|e| format!("setlist: {e}"))?) } else { None };
        Ok(SetList { name: name.to_string(), note, segments, rights })
    }

    /// The canonical text (sorted keys, JS number formatting): what the CID hashes.
    pub fn canonical(&self) -> String {
        self.to_json().canonical_js()
    }

    /// The content ID: `z…`, as the shares'.
    pub fn cid(&self) -> String {
        djengine::cid::share_cid_canonical(&self.canonical())
    }

    /// The rights in force: the block, or CC0 / public for a set saved before there was one.
    pub fn rights(&self) -> Rights {
        self.rights.clone().unwrap_or_default()
    }

    /// A new license (a new set: the CID changes).
    pub fn set_license(&mut self, l: License) {
        let mut r = self.rights();
        r.license = l;
        self.rights = Some(r);
    }

    /// Public or private (a new set: the CID changes).
    pub fn set_visibility(&mut self, v: Visibility) {
        let mut r = self.rights();
        r.visibility = v;
        self.rights = Some(r);
    }

    /// Name the author (a signer identity); empty clears it.
    pub fn set_author(&mut self, a: Option<&str>) {
        let mut r = self.rights();
        r.author = a.map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
        self.rights = Some(r);
    }

    /// A remix of this set by `author`: the same segments under `name`, pointing at this set's
    /// CID, with the license, visibility and credits [`Rights::fork`] gives (and its warnings).
    pub fn fork(&self, name: &str, author: Option<&str>) -> (SetList, Vec<String>) {
        let Fork { rights, warnings } = self.rights().fork(&self.cid(), Some(&self.name), author);
        (SetList { name: name.trim().to_string(), rights: Some(rights), ..self.clone() }, warnings)
    }

    pub fn total_bars(&self) -> u32 {
        self.segments.iter().map(|s| s.bars).sum()
    }

    /// The set in words, for lists: `12 segments, 192 bars`.
    pub fn summary(&self) -> String {
        let n = self.segments.len();
        format!("{n} segment{}, {} bars", if n == 1 { "" } else { "s" }, self.total_bars())
    }

    pub fn summary_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("cid", self.cid().into());
        o.set("name", self.name.clone().into());
        o.set("segments", JsonValue::int(self.segments.len() as i64));
        o.set("bars", JsonValue::int(self.total_bars() as i64));
        o.set("text", self.summary().into());
        o.set("rights", self.rights().to_json());
        JsonValue::Object(o)
    }

    // ---- edits: each answers an error and changes nothing when it cannot

    fn check_index(&self, i: usize) -> Result<(), String> {
        if i < self.segments.len() {
            Ok(())
        } else {
            Err(format!("no segment {} (the set has {})", i + 1, self.segments.len()))
        }
    }

    /// Insert `seg` so it becomes segment `at` (0-based; `len` appends).
    pub fn insert(&mut self, at: usize, seg: Segment) -> Result<(), String> {
        if self.segments.len() >= MAX_SEGMENTS {
            return Err(format!("a set holds at most {MAX_SEGMENTS} segments"));
        }
        if at > self.segments.len() {
            return Err(format!("cannot insert at {} (the set has {})", at + 1, self.segments.len()));
        }
        self.segments.insert(at, seg);
        Ok(())
    }

    pub fn remove(&mut self, i: usize) -> Result<Segment, String> {
        self.check_index(i)?;
        Ok(self.segments.remove(i))
    }

    /// Move segment `from` so it ends up at `to` (both 0-based).
    pub fn move_segment(&mut self, from: usize, to: usize) -> Result<(), String> {
        self.check_index(from)?;
        self.check_index(to)?;
        let s = self.segments.remove(from);
        self.segments.insert(to, s);
        Ok(())
    }

    pub fn set_bars(&mut self, i: usize, bars: i64) -> Result<(), String> {
        self.check_index(i)?;
        if !(MIN_BARS as i64..=MAX_BARS as i64).contains(&bars) {
            return Err(format!("bars is a number {MIN_BARS}-{MAX_BARS}"));
        }
        self.segments[i].bars = bars as u32;
        Ok(())
    }

    pub fn set_transition(&mut self, i: usize, t: Transition) -> Result<(), String> {
        self.check_index(i)?;
        self.segments[i].transition = t;
        Ok(())
    }

    pub fn set_source(&mut self, i: usize, s: Source) -> Result<(), String> {
        self.check_index(i)?;
        self.segments[i].source = s;
        Ok(())
    }

    pub fn set_inserts(&mut self, i: usize, v: Vec<InsertAuto>) -> Result<(), String> {
        self.check_index(i)?;
        self.segments[i].inserts = v;
        Ok(())
    }

    pub fn set_tag(&mut self, i: usize, t: Tag) -> Result<(), String> {
        self.check_index(i)?;
        self.segments[i].tag = t;
        Ok(())
    }

    /// A copy under a new name (a different set: its CID differs).
    pub fn duplicate(&self, name: &str) -> SetList {
        SetList { name: name.trim().to_string(), ..self.clone() }
    }

    /// Whether every pair's group is a real group of its set and its A and B are both there:
    /// the generator only writes such pairs, and this is how a hand-written set is checked.
    pub fn check_pairs(&self, info: &dyn BankInfo) -> Result<(), String> {
        for (i, s) in self.segments.iter().enumerate() {
            if let Source::Pair { spec, group } = &s.source {
                let b = info.bank(spec).ok_or_else(|| format!("segment {}: no loop set {spec:?}", i + 1))?;
                if *group == 0 || *group > b.groups {
                    return Err(format!("segment {}: {spec} has groups 1-{}, not {group}", i + 1, b.groups));
                }
            }
        }
        Ok(())
    }
}

/// Every verb of the `edit` action, as the control surface names them.
pub const EDIT_OPS: [&str; 10] = ["insert", "remove", "move", "bars", "transition", "inserts", "tag", "source", "rename", "note"];

fn index_of(p: &JsonValue, key: &str, len: usize) -> Result<usize, String> {
    let n = p.get(key).and_then(JsonValue::as_i64).ok_or_else(|| format!("{key} is a segment number (1-based)"))?;
    if n < 1 || n as usize > len {
        return Err(format!("{key} {n}: the set has {len} segments"));
    }
    Ok(n as usize - 1)
}

/// Apply one edit `op` with its params (`index` / `at` / `to` are 1-based, as a person counts)
/// to `set`. All or nothing.
pub fn edit(set: &mut SetList, op: &str, p: &JsonValue) -> Result<(), String> {
    let mut next = set.clone();
    let len = next.segments.len();
    match op {
        "insert" => {
            let at = match p.get("at") {
                None | Some(JsonValue::Null) => len,
                Some(_) => {
                    let n = p.get("at").and_then(JsonValue::as_i64).ok_or("at is a segment number (1-based)")?;
                    if n < 1 || n as usize > len + 1 {
                        return Err(format!("at {n}: 1-{}", len + 1));
                    }
                    n as usize - 1
                }
            };
            let seg = match p.get("segment") {
                Some(s) if s.as_object().is_some() => Segment::from_json(s)?,
                _ => {
                    let src = p.get("source").ok_or("insert: a segment {source, bars, transition, ...} or a source")?;
                    let source = match src.as_str() {
                        Some(t) => Source::parse(t)?,
                        None => Source::from_json(src)?,
                    };
                    let bars = p.get("bars").and_then(JsonValue::as_i64).unwrap_or(DEFAULT_BARS as i64);
                    if !(MIN_BARS as i64..=MAX_BARS as i64).contains(&bars) {
                        return Err(format!("bars is a number {MIN_BARS}-{MAX_BARS}"));
                    }
                    let mut s = Segment::new(source, bars as u32);
                    if let Some(t) = p.get("transition").filter(|t| !matches!(t, JsonValue::Null)) {
                        s.transition = transition_param(t, p)?;
                    }
                    s
                }
            };
            next.insert(at, seg)?;
        }
        "remove" => {
            let i = index_of(p, "index", len)?;
            next.remove(i)?;
        }
        "move" => {
            let from = index_of(p, "index", len)?;
            let to = index_of(p, "to", len)?;
            next.move_segment(from, to)?;
        }
        "bars" => {
            let i = index_of(p, "index", len)?;
            let b = p.get("bars").and_then(JsonValue::as_i64).ok_or("bars is a number")?;
            next.set_bars(i, b)?;
        }
        "transition" => {
            let i = index_of(p, "index", len)?;
            let t = p.get("transition").ok_or("transition: a kind (cut, bass_swap, filter_exit, ...)")?;
            next.set_transition(i, transition_param(t, p)?)?;
        }
        "inserts" => {
            let i = index_of(p, "index", len)?;
            let list = match p.get("inserts") {
                None | Some(JsonValue::Null) => Vec::new(),
                Some(a) => a.as_array().ok_or("inserts is a list of {kind, bars, amount}")?.iter().map(InsertAuto::from_json).collect::<Result<_, _>>()?,
            };
            next.set_inserts(i, list)?;
        }
        "tag" => {
            let i = index_of(p, "index", len)?;
            let t = match p.get("tag") {
                None | Some(JsonValue::Null) => Tag::default(),
                Some(t) => Tag::from_json(t)?,
            };
            next.set_tag(i, t)?;
        }
        "source" => {
            let i = index_of(p, "index", len)?;
            let src = p.get("source").ok_or("source: a source")?;
            let s = match src.as_str() {
                Some(t) => Source::parse(t)?,
                None => Source::from_json(src)?,
            };
            next.set_source(i, s)?;
        }
        "rename" => {
            let n = p.get("name").and_then(JsonValue::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("rename: a name")?;
            next.name = n.to_string();
        }
        "note" => next.note = p.get("note").and_then(JsonValue::as_str).unwrap_or("").to_string(),
        other => return Err(format!("edit op {other:?}: one of {}", EDIT_OPS.join(", "))),
    }
    *set = next;
    Ok(())
}

/// `transition` as a name (`"echo_out"`, with `bars` from the params) or an object.
fn transition_param(t: &JsonValue, p: &JsonValue) -> Result<Transition, String> {
    match t.as_str() {
        Some(name) => {
            let kind = TransitionKind::parse(name).ok_or_else(|| format!("transition {name:?}: one of {}", TransitionKind::names().join(", ")))?;
            let mut tr = Transition::new(kind);
            if let Some(b) = p.get("transition_bars").or_else(|| p.get("fade")).filter(|b| !matches!(b, JsonValue::Null)) {
                tr.bars = b.as_i64().filter(|n| (1..=32).contains(n)).ok_or("transition_bars is a number 1-32")? as u32;
            }
            Ok(tr)
        }
        None => Transition::from_json(t),
    }
}

#[cfg(test)]
mod tests;
