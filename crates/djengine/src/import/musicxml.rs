//! `Session/MusicXMLImport.swift`: MusicXML (`.musicxml`, `.xml`, and the zipped `.mxl`) →
//! the same [`Parsed`] a MIDI file gives, so one converter ([`super::midi::convert`])
//! quantises it, sorts the roles and reports. Reads score-partwise documents: each part's
//! measures, `divisions`, the first time signature, `<sound tempo>`, notes with `<chord/>`,
//! ties, `<backup>` / `<forward>`, written→sounding transposition, one source per (part,
//! staff), and unpitched parts as drums by `<midi-unpitched>`. Grace notes, repeats and
//! tempo changes after the first are counted, not played. Time becomes 480 ticks per quarter.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::midi::{MidiImportError, Parsed, RawNote, Source};
use super::xml::{self, XmlNode};

pub const TICKS_PER_QUARTER: i64 = 480;

// MARK: .mxl container

fn u16_at(d: &[u8], i: usize) -> usize {
    d[i] as usize | (d[i + 1] as usize) << 8
}
fn u32_at(d: &[u8], i: usize) -> usize {
    u16_at(d, i) | u16_at(d, i + 2) << 16
}

/// `MiniZip`: just enough zip for an .mxl. Reads the central directory; entries are stored
/// or raw-deflated; CRCs are not checked.
struct MiniZip<'a> {
    data: &'a [u8],
    entries: BTreeMap<String, (usize, usize, usize)>, // method, compressed size, local offset
}

impl<'a> MiniZip<'a> {
    fn new(data: &'a [u8]) -> Result<Self, MidiImportError> {
        if data.len() <= 22 {
            return Err(MidiImportError::bad("not a zip file"));
        }
        let mut eocd: Option<usize> = None;
        let mut i = data.len() - 22;
        let floor = (data.len() as i64 - 22 - 65_535).max(0) as usize;
        loop {
            if u32_at(data, i) == 0x0605_4B50 {
                eocd = Some(i);
                break;
            }
            if i == 0 || i <= floor {
                break;
            }
            i -= 1;
        }
        let Some(eocd) = eocd else {
            return Err(MidiImportError::bad(
                "not a zip file (no central directory)",
            ));
        };
        let count = u16_at(data, eocd + 10);
        let mut p = u32_at(data, eocd + 16);
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            if !(p + 46 <= data.len() && u32_at(data, p) == 0x0201_4B50) {
                break;
            }
            let name_len = u16_at(data, p + 28);
            let extra_len = u16_at(data, p + 30);
            let comment_len = u16_at(data, p + 32);
            if p + 46 + name_len > data.len() {
                break;
            }
            let name = String::from_utf8_lossy(&data[p + 46..p + 46 + name_len]).into_owned();
            entries.insert(
                name,
                (
                    u16_at(data, p + 10),
                    u32_at(data, p + 20),
                    u32_at(data, p + 42),
                ),
            );
            p += 46 + name_len + extra_len + comment_len;
        }
        Ok(MiniZip { data, entries })
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, MidiImportError> {
        let Some(&(method, size, h)) = self.entries.get(name) else {
            return Ok(None);
        };
        let corrupt = || MidiImportError::Bad(format!("corrupt zip entry {name}"));
        if h + 30 > self.data.len() {
            return Err(corrupt());
        }
        let start = h + 30 + u16_at(self.data, h + 26) + u16_at(self.data, h + 28);
        if start + size > self.data.len() {
            return Err(corrupt());
        }
        let raw = &self.data[start..start + size];
        match method {
            0 => Ok(Some(raw.to_vec())),
            8 => miniz_oxide::inflate::decompress_to_vec(raw)
                .map(Some)
                .map_err(|_| MidiImportError::Bad(format!("could not inflate {name}"))),
            m => Err(MidiImportError::Bad(format!(
                "zip entry {name}: unsupported compression {m}"
            ))),
        }
    }
}

/// `MusicXMLImport.unpack(mxl:)`: the score inside a zipped `.mxl`: META-INF/container.xml
/// names the root file.
pub fn unpack(mxl: &[u8]) -> Result<Vec<u8>, MidiImportError> {
    let zip = MiniZip::new(mxl)?;
    let Some(container) = zip.read("META-INF/container.xml")? else {
        return Err(MidiImportError::bad(
            "the .mxl has no META-INF/container.xml",
        ));
    };
    let tree = parse_xml(&container)?;
    let path = tree
        .find("rootfile")
        .and_then(|r| r.attr("full-path"))
        .map(String::from);
    if let Some(path) = path {
        if let Some(xml) = zip.read(&path)? {
            return Ok(xml);
        }
    }
    Err(MidiImportError::bad(
        "the .mxl's container names no readable score file",
    ))
}

fn parse_xml(data: &[u8]) -> Result<XmlNode, MidiImportError> {
    xml::parse(data).map_err(|e| MidiImportError::Bad(format!("unreadable XML: {e}")))
}

fn rint(d: f64) -> i64 {
    libm::round(d) as i64
}

struct Stream {
    notes: Vec<RawNote>,
    drums: bool,
    staff: i64,
}

/// `MusicXMLImport.parse`.
pub fn parse(data: &[u8]) -> Result<Parsed, MidiImportError> {
    let unzipped;
    let bytes: &[u8] = if data.starts_with(&[0x50, 0x4B]) {
        unzipped = unpack(data)?;
        &unzipped
    } else {
        data
    };
    let root = parse_xml(bytes)?;
    if root.name != "score-partwise" {
        return Err(MidiImportError::Bad(if root.name == "score-timewise" {
            "score-timewise MusicXML is not supported (re-export as partwise)".to_string()
        } else {
            format!("not a MusicXML score (root element {})", root.name)
        }));
    }

    let mut out = Parsed {
        format: 1,
        ticks_per_quarter: TICKS_PER_QUARTER,
        tempos: Vec::new(),
        time_signature: None,
        title: root
            .find("work-title")
            .and_then(XmlNode::text_or_none)
            .or_else(|| root.find("movement-title").and_then(XmlNode::text_or_none)),
        sources: Vec::new(),
        last_tick: 0,
        warnings: Vec::new(),
    };

    // score-part id → name, GM program (0-based), midi-unpitched by instrument id
    let mut part_name: BTreeMap<String, String> = BTreeMap::new();
    let mut part_program: BTreeMap<String, i64> = BTreeMap::new();
    let mut unpitched: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(list) = root.find("part-list") {
        for sp in list.all("score-part") {
            let id = sp.attr("id").unwrap_or("").to_string();
            part_name.insert(
                id.clone(),
                sp.child("part-name")
                    .and_then(XmlNode::text_or_none)
                    .unwrap_or_else(|| id.clone()),
            );
            for mi in sp.all("midi-instrument") {
                if let Some(prog) = mi.child("midi-program").and_then(XmlNode::int) {
                    part_program.entry(id.clone()).or_insert((prog - 1).max(0));
                }
                if let (Some(u), Some(iid)) = (
                    mi.child("midi-unpitched").and_then(XmlNode::int),
                    mi.attr("id"),
                ) {
                    unpitched.insert(iid.to_string(), (u - 1).max(0));
                }
            }
        }
    }

    let mut skipped: BTreeMap<&'static str, i64> = BTreeMap::new();
    for (part_index, part) in root.all("part").enumerate() {
        let id = part
            .attr("id")
            .map(String::from)
            .unwrap_or_else(|| format!("P{}", part_index + 1));
        let mut divisions = 1.0f64;
        let mut transpose: i64 = 0;
        let mut measure_start = 0.0f64; // quarters
        let mut streams: BTreeMap<String, Stream> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        // "<stream>|<pitch>" → index of a note waiting for its tie stop
        let mut open: BTreeMap<String, usize> = BTreeMap::new();

        for measure in part.all("measure") {
            let mut cursor = 0.0f64; // divisions
            let mut reached = 0.0f64;
            let mut last_start = 0.0f64;
            for el in &measure.children {
                // tick(div) uses the current `divisions`
                let tick = |div: f64, divisions: f64, measure_start: f64| -> i64 {
                    rint((measure_start + div / divisions) * TICKS_PER_QUARTER as f64)
                };
                let tempo = |node: &XmlNode,
                             at: f64,
                             out: &mut Parsed,
                             divisions: f64,
                             measure_start: f64| {
                    if let Some(t) = node.attr("tempo").and_then(|v| v.parse::<f64>().ok()) {
                        if t > 0.0 {
                            out.tempos
                                .push((tick(at, divisions, measure_start), rint(60_000_000.0 / t)));
                        }
                    }
                };
                match el.name.as_str() {
                    "attributes" => {
                        if let Some(d) = el.child("divisions").and_then(XmlNode::double) {
                            if d > 0.0 {
                                divisions = d;
                            }
                        }
                        if out.time_signature.is_none() {
                            if let Some(t) = el.child("time") {
                                if let (Some(n), Some(d)) = (
                                    t.child("beats").and_then(XmlNode::int),
                                    t.child("beat-type").and_then(XmlNode::int),
                                ) {
                                    out.time_signature = Some((n, d));
                                }
                            }
                        }
                        if let Some(t) = el.child("transpose") {
                            transpose = t.child("chromatic").and_then(XmlNode::int).unwrap_or(0)
                                + 12 * t.child("octave-change").and_then(XmlNode::int).unwrap_or(0);
                        }
                    }
                    "direction" => {
                        let mut sounds = Vec::new();
                        el.descendants("sound", &mut sounds);
                        for s in sounds {
                            tempo(s, cursor, &mut out, divisions, measure_start);
                        }
                    }
                    "sound" => tempo(el, cursor, &mut out, divisions, measure_start),
                    "backup" => {
                        cursor -= el
                            .child("duration")
                            .and_then(XmlNode::double)
                            .unwrap_or(0.0)
                    }
                    "forward" => {
                        cursor += el
                            .child("duration")
                            .and_then(XmlNode::double)
                            .unwrap_or(0.0);
                        reached = reached.max(cursor);
                    }
                    "barline" => {
                        if el.child("repeat").is_some() || el.child("ending").is_some() {
                            *skipped.entry("repeatsNotExpanded").or_default() += 1;
                        }
                    }
                    "note" => {
                        let dur = el
                            .child("duration")
                            .and_then(XmlNode::double)
                            .unwrap_or(0.0);
                        let is_chord = el.child("chord").is_some();
                        let start = if is_chord { last_start } else { cursor };
                        // `defer` in Swift: runs however the case is left
                        let advance =
                            |cursor: &mut f64, last_start: &mut f64, reached: &mut f64| {
                                if !is_chord {
                                    *last_start = *cursor;
                                    *cursor += dur;
                                }
                                *reached = reached.max(*cursor);
                            };
                        if el.child("rest").is_some() {
                            advance(&mut cursor, &mut last_start, &mut reached);
                            continue;
                        }
                        if el.child("grace").is_some() {
                            *skipped.entry("graceNotes").or_default() += 1;
                            advance(&mut cursor, &mut last_start, &mut reached);
                            continue;
                        }
                        let staff = el.child("staff").and_then(XmlNode::int).unwrap_or(1);
                        let drums = el.child("unpitched").is_some();
                        let mut pitch: i64;
                        let written = el.child("pitch").and_then(|p| {
                            let step = p.child("step").map(|s| s.text.clone())?;
                            let octave = p.child("octave").and_then(XmlNode::int)?;
                            Some((p, step, octave))
                        });
                        if let Some((p, step, octave)) = written {
                            let base = match step.as_str() {
                                "C" => 0,
                                "D" => 2,
                                "E" => 4,
                                "F" => 5,
                                "G" => 7,
                                "A" => 9,
                                "B" => 11,
                                _ => 0,
                            };
                            let alter =
                                rint(p.child("alter").and_then(XmlNode::double).unwrap_or(0.0));
                            pitch = (octave + 1) * 12 + base + alter + transpose;
                        } else if drums {
                            let found = el
                                .child("instrument")
                                .and_then(|i| i.attr("id"))
                                .and_then(|iid| unpitched.get(iid));
                            match found {
                                Some(&u) => pitch = u,
                                None => {
                                    *skipped.entry("unpitchedWithoutMidiNote").or_default() += 1;
                                    advance(&mut cursor, &mut last_start, &mut reached);
                                    continue;
                                }
                            }
                        } else {
                            advance(&mut cursor, &mut last_start, &mut reached);
                            continue;
                        }
                        pitch = pitch.clamp(0, 127);
                        let key =
                            format!("{}|{}|{}", part_index, staff, if drums { "d" } else { "p" });
                        if !streams.contains_key(&key) {
                            streams.insert(
                                key.clone(),
                                Stream {
                                    notes: Vec::new(),
                                    drums,
                                    staff,
                                },
                            );
                            order.push(key.clone());
                        }
                        let tk = |div: f64| {
                            rint((measure_start + div / divisions) * TICKS_PER_QUARTER as f64)
                        };
                        let s0 = tk(start);
                        let s1 = tk(start + dur);
                        let ties: Vec<&str> =
                            el.all("tie").filter_map(|t| t.attr("type")).collect();
                        let tie_key = format!("{key}|{pitch}");
                        let stream = streams.get_mut(&key).unwrap();
                        let mut handled = false;
                        if ties.contains(&"stop") {
                            if let Some(&i) = open.get(&tie_key) {
                                if stream.notes[i].end == s0 {
                                    stream.notes[i].end = s1;
                                    if !ties.contains(&"start") {
                                        open.remove(&tie_key);
                                    }
                                    handled = true;
                                }
                            }
                        }
                        if !handled {
                            stream.notes.push(RawNote {
                                start: s0,
                                end: s1.max(s0 + 1),
                                pitch,
                                velocity: 90,
                            });
                            if ties.contains(&"start") {
                                open.insert(tie_key, stream.notes.len() - 1);
                            }
                        }
                        advance(&mut cursor, &mut last_start, &mut reached);
                    }
                    _ => {}
                }
            }
            measure_start += reached / divisions;
            out.last_tick = out
                .last_tick
                .max(rint(measure_start * TICKS_PER_QUARTER as f64));
        }

        for key in &order {
            let st = &streams[key];
            if st.notes.is_empty() {
                continue;
            }
            let base = part_name.get(&id).cloned().unwrap_or_else(|| id.clone());
            let name = base
                + &if st.staff > 1 {
                    format!(" (staff {})", st.staff)
                } else {
                    String::new()
                };
            let mut notes = st.notes.clone();
            notes.sort_by_key(|n| (n.start, n.pitch));
            out.sources.push(Source {
                track: part_index as i64,
                channel: if st.drums { 9 } else { 0 },
                name,
                program: if st.drums {
                    None
                } else {
                    part_program.get(&id).copied()
                },
                notes,
                label: Some(format!("{}:{}", id, st.staff)),
                drums: Some(st.drums),
            });
        }
    }
    out.tempos.sort_by_key(|t| t.0);
    for (k, n) in &skipped {
        out.warnings.push(format!("{k}: {n}"));
    }
    if !out.sources.iter().any(|s| !s.notes.is_empty()) {
        return Err(MidiImportError::NoNotes);
    }
    Ok(out)
}
