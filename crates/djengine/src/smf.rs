//! Standard MIDI Files out: a program's notes as a type-1 file a DAW opens (docs/EXPORT.md).
//!
//! [`Song`] is what the exporter collects from a render: one [`Track`] per part (deck x role), each
//! note on its sixteenth step with the velocity the engine played, the tempo map, the key, the
//! section markers. [`write`] turns it into bytes, and the bytes depend on nothing but the song: no
//! clock, no map iteration order, no float formatting, so the same program gives the same file.
//!
//! The file:
//!
//! - track 0 is the conductor: the song's name, its copyright line, 4/4, the key signature, the
//!   tempo map (a ramp is a staircase of tempo events, one a sixteenth of a bar) and a marker at
//!   each section's first bar;
//! - then one track per part, named `A kick`, `B arp`…, with its program change when the part's
//!   instrument has a General MIDI match ([`gm_program`]); drum parts are on channel 10 with GM
//!   percussion notes ([`gm_drum_note`]), the other parts on 1…16 skipping 10;
//! - [`PPQ`] ticks a quarter, so a sixteenth step is 120 ticks.
//!
//! At the same tick a note-off comes before a note-on, so a note struck again on its own pitch is
//! heard twice; a note that would overlap the next one on its pitch and channel is cut there.
//!
//! [`read_meta`] reads the conductor track back (tests, and anyone checking a file): the importer's
//! [`crate::import::midi::parse`] reads the notes.
//!
//! Pure: no I/O, `no_std` + `alloc`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Ticks a quarter note.
pub const PPQ: u16 = 480;
/// Ticks a sixteenth step.
pub const STEP_TICKS: u32 = PPQ as u32 / 4;
/// MIDI channel 10 (0-based 9): General MIDI percussion.
pub const DRUM_CHANNEL: u8 = 9;

/// One note: where it starts and how long it lasts, in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Note {
    pub tick: u32,
    pub len: u32,
    pub key: u8,
    pub velocity: u8,
}

/// A part's track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// `A kick`, `B arp`.
    pub name: String,
    /// 0-based; [`DRUM_CHANNEL`] for drums.
    pub channel: u8,
    /// A GM program (0-based) to set at tick 0, when the part's instrument has a match.
    pub program: Option<u8>,
    pub notes: Vec<Note>,
}

/// A MIDI key signature: sharps (+) or flats (-), and minor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySig {
    pub sharps: i8,
    pub minor: bool,
}

/// What [`write`] makes a file of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub name: String,
    /// The copyright meta event (a license line), if any.
    pub copyright: Option<String>,
    /// `(tick, bpm)`, in order; the first at tick 0 (120 when empty).
    pub tempos: Vec<(u32, f64)>,
    pub key: Option<KeySig>,
    /// `(tick, text)`: each section's name at its first bar.
    pub markers: Vec<(u32, String)>,
    pub tracks: Vec<Track>,
    /// The song's end: every track's end-of-track is here (or after its last note-off).
    pub end: u32,
}

// ---------------------------------------------------------------------------------------------
// General MIDI

/// The GM percussion note a drum hit is written as. The engine already plays GM notes on its drum
/// channels (kick 36, snare 38, clap 39, closed hat 42, open hat 46, toms, cymbals): a note in the
/// GM percussion range (35…81) is kept as it is. Anything else (a drum part written with a pitch)
/// is mapped by its role: kick 36 (Bass Drum 1), snare 38 (Acoustic Snare), clap 39 (Hand Clap),
/// hihat 42 (Closed Hi-Hat), any other drum role 38.
pub fn gm_drum_note(role: &str, note: u8) -> u8 {
    if (35..=81).contains(&note) {
        return note;
    }
    match role {
        "kick" => 36,
        "clap" => 39,
        "hihat" => 42,
        _ => 38,
    }
}

/// The General MIDI program (0-based) for a BeatScript voice, where one is a fair match: what a DAW
/// loads on the track before you pick your own sound. None: no GM program is close (a laser, a
/// noise hit, a talkbox), and the file sets none. The table is the inverse of the importer's
/// ([`crate::import::midi::instrument`]) plus the voices only BeatScript has, by family word.
pub fn gm_program(inst: &str) -> Option<u8> {
    let exact = match inst {
        "piano" => Some(0),
        "electric-piano" => Some(4),
        "harpsichord" => Some(6),
        "clavinet" => Some(7),
        "music-box" => Some(10),
        "vibes" => Some(11),
        "marimba" => Some(12),
        "fm-bell" | "am-bell" => Some(14),
        "organ" => Some(16),
        "rave-organ" => Some(17),
        "acoustic-guitar" => Some(25),
        "electric-guitar" => Some(27),
        "muted-pluck" => Some(28),
        "distorted-guitar" => Some(30),
        "bass" => Some(33),
        "fm-bass" => Some(38),
        "sub-bass" | "808-bass" | "drop-bass" => Some(38),
        "strings" => Some(48),
        "pluck" | "bright-pluck" | "edm-pluck" => Some(45),
        "choir" | "voice-choir-aah" | "voice-bass-choir" => Some(52),
        "voice-choir-ooh" | "voice-soprano" => Some(53),
        "trumpet" => Some(56),
        "brass" | "edm-stab" | "rave-stab" => Some(61),
        "sax" => Some(65),
        "flute" => Some(73),
        "square-lead" | "chiptune" => Some(80),
        "detuned-saw" | "big-saw" | "trance-lead" | "lead" | "duo-lead" | "sync-lead" | "pwm-lead" | "tape-lead" => Some(81),
        "distorted-lead" | "scream-lead" | "screech" | "wobble-lead" | "hoover" => Some(84),
        "warm-pad" | "pad" => Some(89),
        "dark-pad" | "am-pad" => Some(94),
        "glass-pad" => Some(92),
        "sitar" => Some(104),
        "kalimba" => Some(108),
        "steel-drum" => Some(114),
        _ => None,
    };
    if exact.is_some() {
        return exact;
    }
    // a voice named with a family word (`warm-strings`, `reese-bass`): that family's program
    let fam: &[(&str, u8)] = &[("piano", 0), ("organ", 16), ("guitar", 27), ("strings", 48), ("choir", 52), ("pad", 89), ("bass", 38), ("lead", 81), ("pluck", 45), ("bell", 14)];
    fam.iter().find(|(w, _)| inst.split('-').any(|p| p == *w)).map(|(_, p)| *p)
}

/// The key signature of a key given as its tonic pitch class (0 = C) and whether it is minor (its
/// third is flat). Major keys from C♭… to C♯ by the circle of fifths, preferring the spelling
/// with fewer accidentals (F♯ and G♭ both have six: F♯).
pub fn key_sig(tonic: u8, minor: bool) -> KeySig {
    // the major key whose signature it shares: a minor key's relative major is 3 semitones up
    let major = if minor { (tonic + 3) % 12 } else { tonic % 12 };
    // sharps for each major tonic, C..B (negative: flats)
    const SIG: [i8; 12] = [0, -5, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5];
    KeySig { sharps: SIG[usize::from(major)], minor }
}

// ---------------------------------------------------------------------------------------------
// writing

fn vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 5];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(buf[i] | if i > 0 { 0x80 } else { 0 });
    }
}

/// An event of a track at its absolute tick; `ord` breaks ties (lower first).
struct Ev {
    tick: u32,
    ord: u32,
    bytes: Vec<u8>,
}

fn meta(kind: u8, data: &[u8]) -> Vec<u8> {
    let mut b = alloc::vec![0xFF, kind];
    vlq(&mut b, data.len() as u32);
    b.extend_from_slice(data);
    b
}

fn chunk(out: &mut Vec<u8>, mut evs: Vec<Ev>, end: u32) {
    // a stable sort: same tick and ord keep the order they were made in
    evs.sort_by(|a, b| (a.tick, a.ord).cmp(&(b.tick, b.ord)));
    let last = evs.last().map_or(0, |e| e.tick).max(end);
    let mut body = Vec::new();
    let mut at = 0u32;
    for e in &evs {
        vlq(&mut body, e.tick - at);
        at = e.tick;
        body.extend_from_slice(&e.bytes);
    }
    vlq(&mut body, last - at);
    body.extend_from_slice(&[0xFF, 0x2F, 0x00]);
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
}

/// The notes of a track as MIDI will hold them: sorted, and a note cut where the next one on its
/// pitch starts (two notes cannot sound on one key and channel at once).
pub fn playable(notes: &[Note]) -> Vec<Note> {
    let mut v: Vec<Note> = notes.iter().copied().filter(|n| n.key < 128).collect();
    v.sort();
    v.dedup_by(|b, a| a.tick == b.tick && a.key == b.key);
    let mut next_on: BTreeMap<u8, u32> = BTreeMap::new();
    for n in v.iter_mut().rev() {
        if let Some(&t) = next_on.get(&n.key) {
            n.len = n.len.min(t - n.tick);
        }
        n.len = n.len.max(1);
        next_on.insert(n.key, n.tick);
    }
    v
}

/// The type-1 file of `song`.
pub fn write(song: &Song) -> Vec<u8> {
    let mut out = Vec::new();
    let ntracks = 1 + song.tracks.len() as u16;
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&ntracks.to_be_bytes());
    out.extend_from_slice(&PPQ.to_be_bytes());

    // the conductor
    let mut evs: Vec<Ev> = Vec::new();
    let mut ord = 0u32;
    let mut push = |evs: &mut Vec<Ev>, tick: u32, bytes: Vec<u8>| {
        evs.push(Ev { tick, ord, bytes });
        ord += 1;
    };
    push(&mut evs, 0, meta(0x03, song.name.as_bytes()));
    if let Some(c) = &song.copyright {
        push(&mut evs, 0, meta(0x02, c.as_bytes()));
    }
    push(&mut evs, 0, meta(0x58, &[4, 2, 24, 8]));
    if let Some(k) = song.key {
        push(&mut evs, 0, meta(0x59, &[k.sharps as u8, u8::from(k.minor)]));
    }
    let mut tempos = song.tempos.clone();
    if tempos.first().map_or(true, |t| t.0 != 0) {
        tempos.insert(0, (0, tempos.first().map_or(120.0, |t| t.1)));
    }
    let mut last_us: Option<u32> = None;
    for (tick, bpm) in tempos {
        let us = (libm::round(60_000_000.0 / bpm.clamp(1.0, 1000.0)) as u32).min(0xFF_FFFF);
        if last_us == Some(us) {
            continue;
        }
        last_us = Some(us);
        push(&mut evs, tick, meta(0x51, &us.to_be_bytes()[1..]));
    }
    for (tick, text) in &song.markers {
        push(&mut evs, *tick, meta(0x06, text.as_bytes()));
    }
    chunk(&mut out, evs, song.end);

    for t in &song.tracks {
        let ch = t.channel & 0x0F;
        let mut evs: Vec<Ev> = Vec::new();
        evs.push(Ev { tick: 0, ord: 0, bytes: meta(0x03, t.name.as_bytes()) });
        if let Some(p) = t.program {
            evs.push(Ev { tick: 0, ord: 1, bytes: alloc::vec![0xC0 | ch, p & 0x7F] });
        }
        for n in playable(&t.notes) {
            // offs (ord 2) before ons (ord 3) at a tick; within each, by key
            let k = u32::from(n.key);
            evs.push(Ev { tick: n.tick, ord: 3 << 8 | k, bytes: alloc::vec![0x90 | ch, n.key, n.velocity.clamp(1, 127)] });
            evs.push(Ev { tick: n.tick + n.len, ord: 2 << 8 | k, bytes: alloc::vec![0x80 | ch, n.key, 0x40] });
        }
        chunk(&mut out, evs, song.end);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// reading the conductor back

/// The meta events of a file, by track: what [`read_meta`] gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub format: u16,
    pub ppq: u16,
    pub tracks: u16,
    /// Each track's name (empty when it has none).
    pub names: Vec<String>,
    /// `(tick, microseconds a quarter)`.
    pub tempos: Vec<(u32, u32)>,
    pub key: Option<KeySig>,
    pub markers: Vec<(u32, String)>,
    pub copyright: Option<String>,
    /// `(track, channel, program)`.
    pub programs: Vec<(u16, u8, u8)>,
    /// Each track's end-of-track tick.
    pub ends: Vec<u32>,
}

/// The header and meta events of a Standard MIDI File (and its program changes), or why not.
pub fn read_meta(b: &[u8]) -> Result<Meta, String> {
    let bad = |m: &str| m.to_string();
    if b.len() < 14 || &b[0..4] != b"MThd" {
        return Err(bad("no MThd"));
    }
    let u16_at = |p: usize| u16::from_be_bytes([b[p], b[p + 1]]);
    let u32_at = |p: usize| u32::from_be_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]);
    let mut m = Meta { format: u16_at(8), tracks: u16_at(10), ppq: u16_at(12), ..Meta::default() };
    let mut p = 8 + u32_at(4) as usize;
    let mut ti = 0u16;
    while p + 8 <= b.len() {
        let len = u32_at(p + 4) as usize;
        let (start, end) = (p + 8, p + 8 + len);
        if &b[p..p + 4] != b"MTrk" || end > b.len() {
            return Err(bad("bad track chunk"));
        }
        let mut q = start;
        let mut tick = 0u32;
        let mut status = 0u8;
        let mut name = String::new();
        let rd_vlq = |q: &mut usize| -> Result<u32, String> {
            let mut v = 0u32;
            for _ in 0..4 {
                let c = *b.get(*q).ok_or_else(|| bad("truncated"))?;
                *q += 1;
                v = v << 7 | u32::from(c & 0x7F);
                if c & 0x80 == 0 {
                    return Ok(v);
                }
            }
            Err(bad("long vlq"))
        };
        while q < end {
            tick += rd_vlq(&mut q)?;
            let mut s = b[q];
            if s & 0x80 != 0 {
                q += 1;
            } else {
                s = status;
            }
            match s {
                0xFF => {
                    let kind = b[q];
                    q += 1;
                    let n = rd_vlq(&mut q)? as usize;
                    let d = &b[q..(q + n).min(end)];
                    q += n;
                    let text = || String::from_utf8_lossy(d).into_owned();
                    match kind {
                        0x02 => m.copyright = Some(text()),
                        0x03 => name = text(),
                        0x06 => m.markers.push((tick, text())),
                        0x51 if n == 3 => m.tempos.push((tick, u32::from(d[0]) << 16 | u32::from(d[1]) << 8 | u32::from(d[2]))),
                        0x59 if n == 2 => m.key = Some(KeySig { sharps: d[0] as i8, minor: d[1] == 1 }),
                        0x2F => {
                            m.ends.push(tick);
                            q = end;
                        }
                        _ => {}
                    }
                }
                0xF0 | 0xF7 => {
                    let n = rd_vlq(&mut q)? as usize;
                    q += n;
                }
                _ => {
                    status = s;
                    let n = if matches!(s & 0xF0, 0xC0 | 0xD0) { 1 } else { 2 };
                    if s & 0xF0 == 0xC0 {
                        m.programs.push((ti, s & 0x0F, b[q]));
                    }
                    q += n;
                }
            }
        }
        m.names.push(name);
        ti += 1;
        p = end;
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn vlq_bytes() {
        let enc = |v| {
            let mut o = Vec::new();
            vlq(&mut o, v);
            o
        };
        assert_eq!(enc(0), vec![0]);
        assert_eq!(enc(0x7F), vec![0x7F]);
        assert_eq!(enc(0x80), vec![0x81, 0x00]);
        assert_eq!(enc(0x0FFF_FFFF), vec![0xFF, 0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn key_signatures() {
        assert_eq!(key_sig(9, true), KeySig { sharps: 0, minor: true }); // am
        assert_eq!(key_sig(4, false), KeySig { sharps: 4, minor: false }); // E
        assert_eq!(key_sig(5, false), KeySig { sharps: -1, minor: false }); // F
        assert_eq!(key_sig(2, true), KeySig { sharps: -1, minor: true }); // dm
        assert_eq!(key_sig(6, true), KeySig { sharps: 3, minor: true }); // f#m
    }

    #[test]
    fn overlaps_are_cut_and_offs_come_first() {
        let n = |tick, len| Note { tick, len, key: 60, velocity: 100 };
        assert_eq!(playable(&[n(0, 480), n(120, 120)]), vec![n(0, 120), n(120, 120)]);
        let song = Song { name: "t".into(), tempos: vec![(0, 120.0)], tracks: vec![Track { name: "A bass".into(), channel: 0, program: Some(38), notes: vec![n(0, 120), n(120, 120)] }], end: 480, ..Song::default() };
        let b = write(&song);
        let p = crate::import::midi::parse(&b).unwrap();
        let notes = &p.sources[0].notes;
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].start, notes[0].end, notes[1].start, notes[1].end), (0, 120, 120, 240));
        let m = read_meta(&b).unwrap();
        assert_eq!((m.format, m.tracks, m.ppq), (1, 2, PPQ));
        assert_eq!(m.names, vec!["t".to_string(), "A bass".to_string()]);
        assert_eq!(m.programs, vec![(1, 0, 38)]);
        assert_eq!(m.tempos, vec![(0, 500_000)]);
        assert_eq!(m.ends, vec![480, 480]);
    }

    #[test]
    fn programs() {
        assert_eq!(gm_program("sub-bass"), Some(38));
        assert_eq!(gm_program("warm-pad"), Some(89));
        assert_eq!(gm_program("laser"), None);
        assert_eq!(gm_program("reese-bass"), Some(38));
        assert_eq!(gm_drum_note("kick", 36), 36);
        assert_eq!(gm_drum_note("kick", 24), 36);
    }
}
