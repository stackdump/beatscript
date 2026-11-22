//! `Session/MidiImport.swift`: Standard MIDI Files (format 0 / 1, ticks per quarter; SMPTE
//! time is refused), quantised onto sixteenth steps and sorted into the deck's roles (kick
//! snare hihat clap bass melody harmony arp). Everything the grid can't hold is counted and
//! reported, never silently dropped.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::json::{JsonObject, JsonValue};
use crate::roll::{self, PianoRollError};

/// `MidiImportError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiImportError {
    NotMidi,
    Smpte,
    NoNotes,
    Bad(String),
}

impl MidiImportError {
    pub fn bad(m: &str) -> Self {
        MidiImportError::Bad(m.to_string())
    }
}

impl From<PianoRollError> for MidiImportError {
    fn from(e: PianoRollError) -> Self {
        MidiImportError::Bad(e.0)
    }
}

impl fmt::Display for MidiImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiImportError::NotMidi => f.write_str("not a Standard MIDI File (no MThd header)"),
            MidiImportError::Smpte => {
                f.write_str("SMPTE-timed MIDI files are not supported (ticks per quarter only)")
            }
            MidiImportError::NoNotes => f.write_str("the file has no notes"),
            MidiImportError::Bad(m) => f.write_str(m),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for MidiImportError {}

/// `MidiImport.RawNote`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawNote {
    /// Ticks.
    pub start: i64,
    pub end: i64,
    pub pitch: i64,
    pub velocity: i64,
}

/// One (track, channel) stream of notes, as the file has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub track: i64,
    /// 0…15.
    pub channel: i64,
    pub name: String,
    pub program: Option<i64>,
    pub notes: Vec<RawNote>,
    /// A source that isn't a MIDI (track, channel) — a MusicXML part and staff — names itself.
    pub label: Option<String>,
    pub drums: Option<bool>,
}

impl Source {
    pub fn key(&self) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None => format!("{}:{}", self.track, self.channel + 1),
        }
    }
    pub fn is_drums(&self) -> bool {
        self.drums.unwrap_or(self.channel == 9)
    }
}

/// `MidiImport.Parsed`.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub format: i64,
    pub ticks_per_quarter: i64,
    /// `(tick, microsPerQuarter)`.
    pub tempos: Vec<(i64, i64)>,
    /// `(numerator, denominator)`.
    pub time_signature: Option<(i64, i64)>,
    pub title: Option<String>,
    pub sources: Vec<Source>,
    pub last_tick: i64,
    /// What the reader itself left out (MusicXML: grace notes, repeats …).
    pub warnings: Vec<String>,
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
    end: usize,
}

impl Rd<'_> {
    fn vlq(&mut self) -> Option<i64> {
        let mut v: i64 = 0;
        for _ in 0..4 {
            if self.p >= self.end {
                return None;
            }
            let c = self.b[self.p] as i64;
            self.p += 1;
            v = v << 7 | (c & 0x7F);
            if c & 0x80 == 0 {
                return Some(v);
            }
        }
        Some(v)
    }
}

/// `MidiImport.parse`.
pub fn parse(b: &[u8]) -> Result<Parsed, MidiImportError> {
    let u32_at = |at: usize| -> usize {
        (b[at] as usize) << 24
            | (b[at + 1] as usize) << 16
            | (b[at + 2] as usize) << 8
            | b[at + 3] as usize
    };
    let u16_at = |at: usize| -> usize { (b[at] as usize) << 8 | b[at + 1] as usize };
    if !(b.len() >= 14 && b[0] == 0x4D && b[1] == 0x54 && b[2] == 0x68 && b[3] == 0x64) {
        return Err(MidiImportError::NotMidi);
    }
    let header_len = u32_at(4);
    let format = u16_at(8);
    let division = u16_at(12);
    if division & 0x8000 != 0 || division == 0 {
        return Err(MidiImportError::Smpte);
    }
    let mut pos = 8 + header_len;
    let mut out = Parsed {
        format: format as i64,
        ticks_per_quarter: division as i64,
        tempos: Vec::new(),
        time_signature: None,
        title: None,
        sources: Vec::new(),
        last_tick: 0,
        warnings: Vec::new(),
    };
    let mut track_index = 0;
    while pos + 8 <= b.len() {
        let is_track =
            b[pos] == 0x4D && b[pos + 1] == 0x54 && b[pos + 2] == 0x72 && b[pos + 3] == 0x6B;
        let len = u32_at(pos + 4);
        let body = pos + 8;
        let end = b.len().min(body + len);
        pos = body + len;
        if !is_track {
            continue;
        }
        parse_track(b, body, end, track_index, &mut out);
        track_index += 1;
    }
    out.tempos.sort_by_key(|t| t.0);
    if !out.sources.iter().any(|s| !s.notes.is_empty()) {
        return Err(MidiImportError::NoNotes);
    }
    Ok(out)
}

fn parse_track(b: &[u8], start: usize, end: usize, index: i64, out: &mut Parsed) {
    let mut rd = Rd { b, p: start, end };
    let mut tick: i64 = 0;
    let mut running: u8 = 0;
    let mut track_name: Option<String> = None;
    let mut programs: BTreeMap<i64, i64> = BTreeMap::new();
    // channel << 8 | pitch → FIFO of starts
    let mut open: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    let mut notes: BTreeMap<i64, Vec<RawNote>> = BTreeMap::new();

    fn close(
        open: &mut BTreeMap<i64, Vec<(i64, i64)>>,
        notes: &mut BTreeMap<i64, Vec<RawNote>>,
        ch: i64,
        pitch: i64,
        t: i64,
    ) {
        let k = ch << 8 | pitch;
        let Some(q) = open.get_mut(&k) else { return };
        if q.is_empty() {
            return;
        }
        let s = q.remove(0);
        notes.entry(ch).or_default().push(RawNote {
            start: s.0,
            end: t.max(s.0),
            pitch,
            velocity: s.1,
        });
    }

    while rd.p < rd.end {
        let Some(delta) = rd.vlq() else { break };
        tick += delta;
        if rd.p >= rd.end {
            break;
        }
        let mut status = b[rd.p];
        if status & 0x80 != 0 {
            rd.p += 1;
            if status < 0xF0 {
                running = status;
            }
        } else {
            status = running; // running status: this byte is data
            if status == 0 {
                break;
            }
        }
        match status {
            0xFF => {
                if rd.p >= rd.end {
                    continue;
                }
                let ty = b[rd.p];
                rd.p += 1;
                let len = match rd.vlq() {
                    Some(l) if rd.p + l as usize <= rd.end => l as usize,
                    _ => {
                        rd.p = rd.end;
                        continue;
                    }
                };
                let d = &b[rd.p..rd.p + len];
                rd.p += len;
                match ty {
                    0x03 => {
                        if track_name.is_none() {
                            track_name = Some(String::from_utf8_lossy(d).into_owned());
                        }
                    }
                    0x51 if len == 3 => {
                        out.tempos
                            .push((tick, (d[0] as i64) << 16 | (d[1] as i64) << 8 | d[2] as i64));
                    }
                    0x58 if len >= 2 => {
                        if out.time_signature.is_none() {
                            out.time_signature = Some((d[0] as i64, 1i64 << d[1].min(6)));
                        }
                    }
                    0x2F => rd.p = rd.end,
                    _ => {}
                }
            }
            0xF0 | 0xF7 => match rd.vlq() {
                Some(len) => rd.p += len as usize,
                None => rd.p = rd.end,
            },
            _ => {
                let kind = status & 0xF0;
                let ch = (status & 0x0F) as i64;
                let need = if kind == 0xC0 || kind == 0xD0 { 1 } else { 2 };
                if rd.p + need > rd.end {
                    rd.p = rd.end;
                    continue;
                }
                let d1 = b[rd.p] as i64;
                let d2 = if need == 2 { b[rd.p + 1] as i64 } else { 0 };
                rd.p += need;
                match kind {
                    0x90 if d2 > 0 => {
                        open.entry(ch << 8 | d1).or_default().push((tick, d2));
                    }
                    0x90 | 0x80 => close(&mut open, &mut notes, ch, d1, tick),
                    0xC0 => {
                        programs.entry(ch).or_insert(d1);
                    }
                    _ => {}
                }
            }
        }
    }
    let leftovers: Vec<(i64, usize)> = open.iter().map(|(k, q)| (*k, q.len())).collect();
    for (k, n) in leftovers {
        for _ in 0..n {
            close(&mut open, &mut notes, k >> 8, k & 0xFF, tick);
        }
    }
    out.last_tick = out.last_tick.max(tick);
    if out.title.is_none() && index == 0 {
        if let Some(n) = &track_name {
            if !n.is_empty() {
                out.title = Some(n.clone());
            }
        }
    }
    for (ch, mut ns) in notes {
        ns.sort_by_key(|n| (n.start, n.pitch));
        out.sources.push(Source {
            track: index,
            channel: ch,
            name: track_name.clone().unwrap_or_default(),
            program: programs.get(&ch).copied(),
            notes: ns,
            label: None,
            drums: None,
        });
    }
}

// MARK: roles

/// `drumRole`: a GM drum note → (role, note played).
fn drum_role(pitch: i64) -> Option<(&'static str, i64)> {
    Some(match pitch {
        35 | 36 => ("kick", 36),
        37 | 38 | 40 => ("snare", 38),
        39 => ("clap", 39),
        42 | 44 => ("hihat", 42),
        46 => ("hihat", 46),
        _ => return None,
    })
}

/// beats' instrument names for a GM program (0-based); None = the role's default.
pub fn instrument(program: Option<i64>) -> Option<&'static str> {
    let p = program?;
    Some(match p {
        0..=3 => "piano",
        4 | 5 => "electric-piano",
        6 => "harpsichord",
        7 => "clavinet",
        8 | 10 => "music-box",
        9 | 11 => "vibes",
        12 | 13 => "marimba",
        14 => "fm-bell",
        16..=23 => "organ",
        24 | 25 => "acoustic-guitar",
        26 | 27 | 31 => "electric-guitar",
        28 => "muted-pluck",
        29 | 30 => "distorted-guitar",
        32..=34 => "bass",
        35 | 38 | 39 => "sub-bass",
        36 | 37 => "rubber-bass",
        40..=45 | 48..=51 => "strings",
        46 => "pluck",
        52..=54 => "choir",
        56 | 59 => "trumpet",
        57 | 58 | 60..=63 => "brass",
        64..=71 => "sax",
        72..=79 => "flute",
        80 => "square-lead",
        81 => "supersaw",
        82..=87 => "lead",
        88..=95 => "warm-pad",
        104 => "sitar",
        114 => "steel-drum",
        _ => return None,
    })
}

fn rint(d: f64) -> i64 {
    libm::round(d) as i64
}

fn mean_pitch(s: &Source) -> f64 {
    s.notes.iter().map(|n| n.pitch).sum::<i64>() as f64 / core::cmp::max(1, s.notes.len()) as f64
}

/// Role for a pitched source from its program and notes (a user `map` overrides this).
fn role_of(s: &Source, step_ticks: f64) -> &'static str {
    if let Some(p) = s.program {
        if (32..=39).contains(&p) {
            return "bass";
        }
        if (88..=95).contains(&p) || (48..=54).contains(&p) {
            return "harmony";
        }
    }
    if mean_pitch(s) < 50.0 {
        return "bass";
    }
    let mut onsets: BTreeMap<i64, i64> = BTreeMap::new();
    for n in &s.notes {
        *onsets.entry(rint(n.start as f64 / step_ticks)).or_default() += 1;
    }
    let chordy =
        onsets.values().filter(|&&c| c > 1).count() as f64 / core::cmp::max(1, onsets.len()) as f64;
    if chordy > 0.35 {
        "harmony"
    } else {
        "melody"
    }
}

// MARK: import

/// `MidiImport.Options`.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// First bar (0-based).
    pub from: i64,
    pub bars: i64,
    pub tempo: Option<f64>,
    /// "track:channel" or "channel" → role, or "skip".
    pub map: BTreeMap<String, String>,
    pub name: Option<String>,
    /// Provenance / licence text, kept in the share's note.
    pub source: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            from: 0,
            bars: 16,
            tempo: None,
            map: BTreeMap::new(),
            name: None,
            source: None,
        }
    }
}

struct Part {
    from: String,
    notes: usize,
}

/// `MidiImport.Result`.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    pub envelope: JsonValue,
    pub summary: JsonValue,
}

pub const MAX_BARS: i64 = 64;

/// The last path component and its stem, as `URL(fileURLWithPath:)` reads them.
fn last_component(file: &str) -> &str {
    let t = file.trim_end_matches('/');
    t.rsplit('/').next().unwrap_or(t)
}

fn stem(file: &str) -> String {
    let last = last_component(file);
    match last.rfind('.') {
        Some(i) if i > 0 => last[..i].to_string(),
        _ => last.to_string(),
    }
}

/// `MidiImport.convert(_ data:)`: a MIDI file → envelope + report.
pub fn convert_bytes(
    data: &[u8],
    file: &str,
    options: &Options,
) -> Result<Converted, MidiImportError> {
    convert(&parse(data)?, file, options)
}

fn src_json(s: &Source, role: &str, kept: usize) -> JsonValue {
    let mut j = JsonObject::from_pairs([
        ("key", JsonValue::String(s.key())),
        ("track", JsonValue::int(s.track)),
        ("channel", JsonValue::int(s.channel + 1)),
        ("notes", JsonValue::int(s.notes.len() as i64)),
        ("role", JsonValue::str(role)),
        ("kept", JsonValue::int(kept as i64)),
    ]);
    if !s.name.is_empty() {
        j.set("name", JsonValue::str(s.name.as_str()));
    }
    if let Some(p) = s.program {
        j.set("program", JsonValue::int(p));
    }
    JsonValue::Object(j)
}

/// `MidiImport.convert(_ m: Parsed, …)`: any `Parsed` score (a MIDI file, MusicXML) →
/// envelope + report.
pub fn convert(m: &Parsed, file: &str, o: &Options) -> Result<Converted, MidiImportError> {
    let tpq = m.ticks_per_quarter;
    let step_ticks = tpq as f64 / 4.0;
    let sig = m.time_signature.unwrap_or((4, 4));
    let steps_per_bar = core::cmp::max(1, 16 * sig.0 / core::cmp::max(1, sig.1));
    if !((0..=64).contains(&o.from) && o.bars >= 1 && o.bars <= MAX_BARS) {
        return Err(MidiImportError::Bad(format!(
            "`from` must be ≥ 0 and `bars` 1…{MAX_BARS}"
        )));
    }
    let total_bars = libm::ceil(m.last_tick as f64 / step_ticks / steps_per_bar as f64) as i64;
    let win_lo = o.from * steps_per_bar;
    let win_hi = (o.from + o.bars) * steps_per_bar;
    let bars = o.bars.min(core::cmp::max(1, total_bars - o.from));
    let steps = bars * steps_per_bar;

    let mut warnings: Vec<String> = m.warnings.clone();
    let mut dropped: BTreeMap<&'static str, i64> = BTreeMap::new();
    // (step, note, dur, vel)
    let mut parts: BTreeMap<String, Vec<(i64, i64, i64, i64)>> = BTreeMap::new();
    let mut part_info: BTreeMap<String, Part> = BTreeMap::new();
    let mut instrument_for: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut sources_json: Vec<JsonValue> = Vec::new();

    let override_role = |s: &Source| -> Option<String> {
        o.map
            .get(&s.key())
            .or_else(|| o.map.get(&format!("{}", s.channel + 1)))
            .cloned()
    };
    let mut role_for: BTreeMap<String, String> = BTreeMap::new();
    let auto: Vec<&Source> = m
        .sources
        .iter()
        .filter(|s| !s.is_drums() && !s.notes.is_empty() && override_role(s).is_none())
        .collect();
    for s in &auto {
        role_for.insert(s.key(), role_of(s, step_ticks).to_string());
    }
    let taken: BTreeSet<String> = m
        .sources
        .iter()
        .filter(|s| !s.is_drums())
        .filter_map(|s| override_role(s))
        .filter(|r| r != "skip")
        .collect();
    if auto.len() > 1 {
        let has_bass = taken.contains("bass")
            || auto
                .iter()
                .any(|s| role_for.get(&s.key()).map(String::as_str) == Some("bass"));
        if !has_bass {
            let low = auto
                .iter()
                .min_by(|a, b| {
                    mean_pitch(a)
                        .partial_cmp(&mean_pitch(b))
                        .unwrap_or(core::cmp::Ordering::Equal)
                })
                .unwrap();
            if mean_pitch(low) < 55.0 {
                role_for.insert(low.key(), "bass".to_string());
            }
        }
        let mut melodies: Vec<&&Source> = auto
            .iter()
            .filter(|s| role_for.get(&s.key()).map(String::as_str) == Some("melody"))
            .collect();
        melodies.sort_by(|a, b| {
            mean_pitch(b)
                .partial_cmp(&mean_pitch(a))
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let melody_taken = taken.contains("melody");
        for (i, extra) in melodies.iter().enumerate() {
            if i > 0 || melody_taken {
                role_for.insert(extra.key(), "harmony".to_string());
            }
        }
    }

    for s in m.sources.iter().filter(|s| !s.notes.is_empty()) {
        let over = override_role(s);
        if over.as_deref() == Some("skip") {
            sources_json.push(src_json(s, "skip", 0));
            continue;
        }
        let mut kept = 0usize;
        if s.is_drums() {
            for n in &s.notes {
                let step = rint(n.start as f64 / step_ticks);
                if !(step >= win_lo && step < win_hi) {
                    *dropped.entry("outsideExcerpt").or_default() += 1;
                    continue;
                }
                let Some((role, note)) = drum_role(n.pitch) else {
                    *dropped.entry("drumsWithoutARole").or_default() += 1;
                    continue;
                };
                parts.entry(role.to_string()).or_default().push((
                    step - win_lo,
                    note,
                    1,
                    n.velocity,
                ));
                part_info.insert(
                    role.to_string(),
                    Part {
                        from: s.key(),
                        notes: 0,
                    },
                );
                kept += 1;
            }
            sources_json.push(src_json(s, "drums", kept));
            continue;
        }
        let role = match over {
            Some(r) => r,
            None => role_for
                .get(&s.key())
                .cloned()
                .unwrap_or_else(|| role_of(s, step_ticks).to_string()),
        };
        if !matches!(role.as_str(), "bass" | "melody" | "harmony" | "arp") {
            return Err(MidiImportError::Bad(format!(
                "`map` role {role} for {}: use bass, melody, harmony, arp or skip",
                s.key()
            )));
        }
        for n in &s.notes {
            let step = rint(n.start as f64 / step_ticks);
            if !(step >= win_lo && step < win_hi) {
                *dropped.entry("outsideExcerpt").or_default() += 1;
                continue;
            }
            let rel = step - win_lo;
            let dur = core::cmp::max(1, rint((n.end - n.start) as f64 / step_ticks));
            parts.entry(role.clone()).or_default().push((
                rel,
                n.pitch,
                dur.min(steps - rel),
                n.velocity,
            ));
            kept += 1;
        }
        match part_info.get_mut(&role) {
            None => {
                part_info.insert(
                    role.clone(),
                    Part {
                        from: s.key(),
                        notes: 0,
                    },
                );
                if let Some(i) = instrument(s.program) {
                    instrument_for.insert(role.clone(), i);
                }
            }
            Some(p) => {
                p.from.push('+');
                p.from.push_str(&s.key());
            }
        }
        sources_json.push(src_json(s, &role, kept));
    }

    // At most three notes start on one step per role: more would be dozens of sibling rings.
    let mut rolls = JsonObject::new();
    for (role, ns) in &parts {
        let mut by_step: BTreeMap<i64, Vec<(i64, i64, i64, i64)>> = BTreeMap::new();
        for n in ns {
            by_step.entry(n.0).or_default().push(*n);
        }
        let mut notes: Vec<JsonValue> = Vec::new();
        let mut count = 0usize;
        for (_, mut here) in by_step {
            here.sort_by_key(|n| n.1);
            if role != "bass" {
                here.reverse(); // keep the top voices; a bass keeps its lowest
            }
            let mut seen = BTreeSet::new();
            here.retain(|n| seen.insert(n.1));
            if here.len() > 3 {
                *dropped.entry("polyphonyOverThree").or_default() += here.len() as i64 - 3;
            }
            for n in here.iter().take(3) {
                notes.push(JsonValue::Array(alloc::vec![
                    JsonValue::int(n.0),
                    JsonValue::int(n.1),
                    JsonValue::int(n.2),
                    JsonValue::int(n.3),
                ]));
                count += 1;
            }
        }
        let mut part = JsonObject::new();
        part.set("notes", JsonValue::Array(notes));
        if let Some(i) = instrument_for.get(role) {
            part.set("instrument", JsonValue::str(*i));
        }
        rolls.set(role.as_str(), JsonValue::Object(part));
        if let Some(p) = part_info.get_mut(role) {
            p.notes = count;
        }
    }
    if rolls.is_empty() {
        return Err(MidiImportError::Bad(format!(
            "no notes fall in bars {}–{}",
            o.from + 1,
            o.from + bars
        )));
    }

    let bpm = if let Some(t) = o.tempo {
        t
    } else if let Some(first) = m.tempos.first() {
        60_000_000.0 / first.1 as f64
    } else {
        120.0
    };
    let clamped = (libm::round(bpm * 100.0) / 100.0).max(40.0).min(300.0); // 666667 µs → 90
    if bpm < 40.0 || bpm > 300.0 {
        warnings.push(format!(
            "tempo {} BPM clamped to {}",
            bpm as i64, clamped as i64
        ));
    }
    if m.tempos.len() > 1 && o.tempo.is_none() {
        warnings.push(format!(
            "{} tempo changes: only the first is kept",
            m.tempos.len()
        ));
    }
    if sig.0 != 4 || sig.1 != 4 {
        warnings.push(format!(
            "time signature {}/{}: bars are {} steps",
            sig.0, sig.1, steps_per_bar
        ));
    }
    if total_bars - o.from > bars {
        warnings.push(format!(
            "excerpt: bars {}–{} of {}; use `from` / `bars` for more",
            o.from + 1,
            o.from + bars,
            total_bars
        ));
    }

    let title = o
        .name
        .clone()
        .or_else(|| m.title.clone())
        .unwrap_or_else(|| stem(file));
    let params = JsonValue::Object(JsonObject::from_pairs([
        ("name", JsonValue::String(title.clone())),
        ("tempo", JsonValue::Number(clamped)),
        ("steps", JsonValue::int(steps)),
        ("parts", JsonValue::Object(rolls)),
    ]));
    let envelope = roll::envelope(&params)?;
    let mut env = envelope.as_object().cloned().unwrap_or_default();
    let mut note = format!("Imported from {}", last_component(file));
    if let Some(src) = &o.source {
        if !src.is_empty() {
            note.push_str(&format!(" — {src}"));
        }
    }
    env.set("note", JsonValue::String(note));

    let parts_json: Vec<JsonValue> = part_info
        .iter()
        .map(|(r, p)| {
            let mut j = JsonObject::from_pairs([
                ("role", JsonValue::str(r.as_str())),
                ("from", JsonValue::str(p.from.as_str())),
                ("notes", JsonValue::int(p.notes as i64)),
            ]);
            if let Some(i) = instrument_for.get(r) {
                j.set("instrument", JsonValue::str(*i));
            }
            JsonValue::Object(j)
        })
        .collect();
    let summary = JsonValue::Object(JsonObject::from_pairs([
        ("file", JsonValue::str(last_component(file))),
        ("title", JsonValue::String(title)),
        ("format", JsonValue::int(m.format)),
        ("tempo", JsonValue::Number(clamped)),
        (
            "timeSignature",
            JsonValue::String(format!("{}/{}", sig.0, sig.1)),
        ),
        ("totalBars", JsonValue::int(total_bars)),
        ("from", JsonValue::int(o.from)),
        ("bars", JsonValue::int(bars)),
        ("steps", JsonValue::int(steps)),
        ("parts", JsonValue::Array(parts_json)),
        ("sources", JsonValue::Array(sources_json)),
        (
            "dropped",
            JsonValue::Object(JsonObject::from_pairs(
                dropped.iter().map(|(k, v)| (*k, JsonValue::int(*v))),
            )),
        ),
        (
            "warnings",
            JsonValue::Array(warnings.into_iter().map(JsonValue::String).collect()),
        ),
    ]));
    Ok(Converted {
        envelope: JsonValue::Object(env),
        summary,
    })
}

/// `ScoreImport`: a score file by extension — `.mid` / `.midi` → [`parse`], `.musicxml` /
/// `.xml` / `.mxl` → [`super::musicxml::parse`]; both then go through [`convert`].
pub struct ScoreImport;

impl ScoreImport {
    pub const EXTENSIONS: [&'static str; 5] = ["mid", "midi", "musicxml", "xml", "mxl"];

    pub fn convert(
        data: &[u8],
        file: &str,
        options: &Options,
    ) -> Result<Converted, MidiImportError> {
        let last = last_component(file);
        let ext = match last.rfind('.') {
            Some(i) if i > 0 => last[i + 1..].to_lowercase(),
            _ => String::new(),
        };
        match ext.as_str() {
            "mid" | "midi" => convert_bytes(data, file, options),
            "musicxml" | "xml" | "mxl" => convert(&super::musicxml::parse(data)?, file, options),
            _ => Err(MidiImportError::Bad(format!(
                "`path` must be one of: {}",
                Self::EXTENSIONS
                    .iter()
                    .map(|e| format!(".{e}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))),
        }
    }
}
