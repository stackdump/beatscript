//! `Session/MidiImport.swift` and `Session/MusicXMLImport.swift`. The score bytes are built
//! inline exactly as Swift's `SMFSupport.swift` builds them, and the asserts are the
//! literals of `MidiImportTests` and `MusicXMLImportTests`. Where Swift reads the facts
//! through `Share.rawNetsProject()` (phase 1) these read the envelope JSON directly.

use djengine::import::{self, MidiImportError, Options, ScoreImport};
use djengine::json::JsonValue;

// ---- SMFSupport.swift ------------------------------------------------------------------

#[derive(Default)]
struct Smf {
    tracks: Vec<Vec<u8>>,
}

fn vlq(v: usize) -> Vec<u8> {
    let mut out = vec![(v & 0x7F) as u8];
    let mut n = v >> 7;
    while n > 0 {
        out.insert(0, (n & 0x7F) as u8 | 0x80);
        n >>= 7;
    }
    out
}

impl Smf {
    fn track(&mut self, events: Vec<(usize, Vec<u8>)>) {
        let mut ev = events;
        ev.sort_by_key(|e| e.0); // stable, as Swift's sorted(by:) is in practice
        let mut body = Vec::new();
        let mut last = 0;
        for (t, e) in ev {
            body.extend(vlq(t - last));
            body.extend(e);
            last = t;
        }
        body.extend([0x00, 0xFF, 0x2F, 0x00]);
        self.tracks.push(body);
    }

    fn data(&self) -> Vec<u8> {
        let mut d: Vec<u8> = b"MThd".to_vec();
        d.extend([0, 0, 0, 6, 0, 1, 0, self.tracks.len() as u8, 0x01, 0xE0]);
        for t in &self.tracks {
            let n = t.len();
            d.extend(b"MTrk");
            d.extend([
                (n >> 24) as u8,
                (n >> 16 & 255) as u8,
                (n >> 8 & 255) as u8,
                (n & 255) as u8,
            ]);
            d.extend(t);
        }
        d
    }
}

fn note(ch: u8, pitch: u8, at: usize, len: usize) -> Vec<(usize, Vec<u8>)> {
    vec![
        (at, vec![0x90 | ch, pitch, 100]),
        (at + len, vec![0x80 | ch, pitch, 0]),
    ]
}

/// Two bars of 4/4 at 100 BPM: drums, a bass, a single-note lead, a three-note pad.
fn sample_midi() -> Vec<u8> {
    let mut f = Smf::default();
    let mut head = vec![(0, [vec![0xFF, 0x03, 4], b"Demo".to_vec()].concat())];
    head.push((0, vec![0xFF, 0x51, 3, 0x09, 0x27, 0xC0])); // 600000 µs = 100 BPM
    head.push((0, vec![0xFF, 0x58, 4, 4, 2, 24, 8]));
    f.track(head);
    let mut drums = Vec::new();
    for beat in 0..8 {
        drums.extend(note(9, 36, beat * 480, 60));
        if beat % 2 == 1 {
            drums.extend(note(9, 38, beat * 480, 60));
        }
    }
    drums.extend(note(9, 49, 0, 60)); // a crash: no role
    f.track(drums);
    let mut bass = vec![(0, vec![0xC1, 33])];
    bass.extend(note(1, 36, 0, 480));
    bass.extend(note(1, 43, 960, 480));
    f.track(bass);
    let mut lead = note(2, 72, 0, 240);
    lead.extend(note(2, 76, 480, 240));
    lead.extend(note(2, 79, 1920, 480));
    f.track(lead);
    let mut pad = vec![(0, vec![0xC3, 89])];
    for p in [60, 64, 67] {
        pad.extend(note(3, p, 0, 1920));
    }
    for p in [57, 60, 64] {
        pad.extend(note(3, p, 1920, 1920));
    }
    f.track(pad);
    f.data()
}

fn four_voice_midi() -> Vec<u8> {
    let mut f = Smf::default();
    f.track(vec![(0, vec![0xFF, 0x51, 3, 0x07, 0xA1, 0x20])]);
    for (ch, base) in [(0u8, 72u8), (1, 67), (2, 60), (3, 48)] {
        let mut ev = Vec::new();
        for beat in 0..4u8 {
            ev.extend(note(ch, base + beat, beat as usize * 480, 400));
        }
        f.track(ev);
    }
    f.data()
}

fn smpte_variant() -> Vec<u8> {
    let mut b = sample_midi();
    b[12] = 0xE7;
    b[13] = 0x28;
    b
}

fn note_less_midi() -> Vec<u8> {
    let mut f = Smf::default();
    f.track(vec![(0, vec![0xFF, 0x51, 3, 7, 0xA1, 0x20])]);
    f.data()
}

const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="4.0">
  <work><work-title>Tiny Score</work-title></work>
  <part-list>
    <score-part id="P1"><part-name>Lead</part-name></score-part>
    <score-part id="P2"><part-name>Bass</part-name>
      <midi-instrument id="P2-I1"><midi-program>34</midi-program></midi-instrument></score-part>
    <score-part id="P3"><part-name>Kit</part-name>
      <score-instrument id="P3-I36"><instrument-name>Kick</instrument-name></score-instrument>
      <midi-instrument id="P3-I36"><midi-unpitched>36</midi-unpitched></midi-instrument>
      <midi-instrument id="P3-I39"><midi-unpitched>39</midi-unpitched></midi-instrument></score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
      <direction><sound tempo="90"/></direction>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration></note>
      <note><pitch><step>E</step><octave>4</octave></pitch><duration>2</duration><tie type="start"/></note>
    </measure>
    <measure number="2">
      <note><pitch><step>E</step><octave>4</octave></pitch><duration>1</duration><tie type="stop"/></note>
      <note><rest/><duration>1</duration></note>
      <note><grace/><pitch><step>C</step><octave>5</octave></pitch></note>
      <note><pitch><step>G</step><octave>4</octave></pitch><duration>2</duration></note>
      <note><chord/><pitch><step>B</step><octave>4</octave></pitch><duration>2</duration></note>
    </measure>
  </part>
  <part id="P2">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><pitch><step>C</step><octave>2</octave></pitch><duration>4</duration></note>
    </measure>
    <measure number="2">
      <note><pitch><step>G</step><octave>2</octave></pitch><duration>4</duration></note>
    </measure>
  </part>
  <part id="P3">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><unpitched><display-step>C</display-step><display-octave>5</display-octave></unpitched><duration>1</duration><instrument id="P3-I36"/></note>
      <note><rest/><duration>1</duration></note>
      <note><unpitched><display-step>C</display-step><display-octave>5</display-octave></unpitched><duration>1</duration><instrument id="P3-I39"/></note>
      <note><rest/><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;

/// A zip with one deflated entry per file (what an .mxl is). CRCs are left 0.
fn zipped(files: &[(&str, &str)]) -> Vec<u8> {
    fn le16(v: usize) -> Vec<u8> {
        vec![(v & 255) as u8, (v >> 8 & 255) as u8]
    }
    fn le32(v: usize) -> Vec<u8> {
        [le16(v & 0xFFFF), le16(v >> 16 & 0xFFFF)].concat()
    }
    let mut body = Vec::new();
    let mut central = Vec::new();
    for (name, text) in files {
        let raw = text.as_bytes();
        let comp = miniz_oxide::deflate::compress_to_vec(raw, 6);
        let n = name.as_bytes();
        let offset = body.len();
        body.extend(
            [
                le32(0x0403_4B50),
                le16(20),
                le16(0),
                le16(8),
                le16(0),
                le16(0),
                le32(0),
            ]
            .concat(),
        );
        body.extend([le32(comp.len()), le32(raw.len()), le16(n.len()), le16(0)].concat());
        body.extend(n);
        body.extend(&comp);
        central.extend(
            [
                le32(0x0201_4B50),
                le16(20),
                le16(20),
                le16(0),
                le16(8),
                le16(0),
                le16(0),
            ]
            .concat(),
        );
        central.extend([le32(0), le32(comp.len()), le32(raw.len()), le16(n.len())].concat());
        central.extend([le16(0), le16(0), le16(0), le16(0), le32(0), le32(offset)].concat());
        central.extend(n);
    }
    let mut out = body.clone();
    out.extend(&central);
    out.extend(
        [
            le32(0x0605_4B50),
            le16(0),
            le16(0),
            le16(files.len()),
            le16(files.len()),
        ]
        .concat(),
    );
    out.extend([le32(central.len()), le32(body.len()), le16(0)].concat());
    out
}

fn sample_mxl() -> Vec<u8> {
    zipped(&[
        (
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container><rootfiles>
<rootfile full-path="score.xml" media-type="application/vnd.recordare.musicxml+xml"/>
</rootfiles></container>"#,
        ),
        ("score.xml", SAMPLE_XML),
    ])
}

// ---- reading the envelope ----------------------------------------------------------------

fn net<'a>(e: &'a JsonValue, id: &str) -> Option<&'a JsonValue> {
    e.get("nets")?.get(id)
}
fn places(e: &JsonValue, id: &str) -> usize {
    net(e, id)
        .unwrap()
        .get("places")
        .unwrap()
        .as_object()
        .unwrap()
        .len()
}
fn midi<'a>(e: &'a JsonValue, id: &str, t: &str) -> Option<&'a JsonValue> {
    net(e, id)?.get("transitions")?.get(t)?.get("midi")
}
fn note_at(e: &JsonValue, id: &str, t: &str) -> Option<i64> {
    midi(e, id, t)?.get("note")?.as_i64()
}
fn dur_at(e: &JsonValue, id: &str, t: &str) -> Option<i64> {
    midi(e, id, t)?.get("durationSteps")?.as_i64()
}
fn net_ids(e: &JsonValue) -> Vec<String> {
    let mut v: Vec<String> = e
        .get("nets")
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .map(String::from)
        .collect();
    v.sort();
    v
}
fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort();
    v.into_iter().map(String::from).collect()
}

// ---- MidiImportTests -----------------------------------------------------------------------

#[test]
fn parses_notes_tempo_and_signature() {
    let m = import::parse(&sample_midi()).unwrap();
    assert!(m.format == 1 && m.ticks_per_quarter == 480 && m.title.as_deref() == Some("Demo"));
    assert_eq!(m.tempos.first().map(|t| t.1), Some(600_000));
    assert_eq!(m.time_signature, Some((4, 4)));
    let drums = m.sources.iter().find(|s| s.is_drums()).unwrap();
    assert_eq!(drums.notes.len(), 13);
    let bass = m.sources.iter().find(|s| s.channel == 1).unwrap();
    assert_eq!(bass.program, Some(33));
    assert_eq!(
        bass.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [36, 43]
    );
    assert_eq!(bass.notes[0].end - bass.notes[0].start, 480);
}

#[test]
fn sorts_channels_into_roles_and_quantises_to_sixteenths() {
    let r = import::convert_bytes(&sample_midi(), "/x/demo.mid", &Options::default()).unwrap();
    let env = &r.envelope;
    assert_eq!(env.get("tempo").unwrap().as_f64(), Some(100.0));
    assert_eq!(env.get("name").unwrap().as_str(), Some("Demo"));
    assert!(env
        .get("note")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("demo.mid"));
    // 8 beats = 2 bars = 32 steps
    assert_eq!(places(env, "kick"), 32);
    assert_eq!(
        net_ids(env),
        sorted(vec![
            "kick",
            "snare",
            "bass",
            "melody",
            "harmony",
            "harmony-b",
            "harmony-c"
        ])
    );
    // a beat is 4 steps: kicks on 0 4 8 …, snares on the odd beats
    assert_eq!(note_at(env, "kick", "t4"), Some(36));
    assert_eq!(note_at(env, "snare", "t4"), Some(38));
    assert!(midi(env, "snare", "t0").is_none());
    assert_eq!(note_at(env, "snare", "t12"), Some(38));
    assert_eq!(dur_at(env, "bass", "t0"), Some(4));
    assert_eq!(note_at(env, "bass", "t8"), Some(43));
    assert_eq!(note_at(env, "melody", "t4"), Some(76));
    assert_eq!(note_at(env, "melody", "t16"), Some(79));
    let inst = |id: &str| {
        net(env, id)
            .unwrap()
            .get("track")
            .unwrap()
            .get("instrument")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(inst("bass"), "bass");
    assert_eq!(inst("harmony"), "warm-pad");
    // the report says what the grid left out
    let s = &r.summary;
    assert_eq!(
        s.get("dropped")
            .unwrap()
            .get("drumsWithoutARole")
            .unwrap()
            .as_i64(),
        Some(1)
    );
    assert_eq!(s.get("totalBars").unwrap().as_i64(), Some(2));
    assert_eq!(s.get("bars").unwrap().as_i64(), Some(2));
    let roles: Vec<&str> = s
        .get("sources")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.get("role").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(roles, ["drums", "bass", "melody", "harmony"]);
}

#[test]
fn an_excerpt_and_a_map_and_a_tempo_override_apply() {
    let mut o = Options::default();
    o.from = 1;
    o.bars = 1;
    o.tempo = Some(140.0);
    o.map.insert("3".into(), "arp".into());
    o.map.insert("4".into(), "skip".into());
    let r = import::convert_bytes(&sample_midi(), "d.mid", &o).unwrap();
    let env = &r.envelope;
    assert_eq!(env.get("tempo").unwrap().as_f64(), Some(140.0));
    assert_eq!(places(env, "kick"), 16);
    assert_eq!(note_at(env, "arp", "t0"), Some(79)); // bar 2's lead note, now step 0
    assert!(net(env, "harmony").is_none());
    assert!(midi(env, "bass", "t0").is_none());
    assert!(
        r.summary
            .get("dropped")
            .unwrap()
            .get("outsideExcerpt")
            .unwrap()
            .as_i64()
            .unwrap()
            > 0
    );
}

#[test]
fn several_lines_are_spread_over_roles_not_all_melody() {
    let r = import::convert_bytes(&four_voice_midi(), "satb.mid", &Options::default()).unwrap();
    let roles: Vec<&str> = r
        .summary
        .get("sources")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.get("role").unwrap().as_str().unwrap())
        .collect();
    // highest = melody, the inner voices = harmony, the lowest (< G3) = bass
    assert_eq!(roles, ["melody", "harmony", "harmony", "bass"]);
    assert!(r
        .summary
        .get("dropped")
        .unwrap()
        .get("polyphonyOverThree")
        .is_none());
}

#[test]
fn refuses_what_it_cannot_read() {
    assert_eq!(
        import::parse(b"hello world, not midi").unwrap_err(),
        MidiImportError::NotMidi
    );
    assert_eq!(
        import::parse(&smpte_variant()).unwrap_err(),
        MidiImportError::Smpte
    );
    assert_eq!(
        import::parse(&note_less_midi()).unwrap_err(),
        MidiImportError::NoNotes
    );
}

// ---- MusicXMLImportTests -----------------------------------------------------------------------

#[test]
fn reads_parts_ties_chords_grace_notes_tempo_and_drums() {
    let m = import::musicxml::parse(SAMPLE_XML.as_bytes()).unwrap();
    assert!(m.title.as_deref() == Some("Tiny Score") && m.ticks_per_quarter == 480);
    assert_eq!(m.tempos.first().map(|t| t.1), Some(666_667)); // 90 BPM
    assert_eq!(m.time_signature, Some((4, 4)));
    let lead = m.sources.iter().find(|s| s.name == "Lead").unwrap();
    // C4 D4 E4(tied across the barline) · G4+B4 chord; the grace note is left out
    assert_eq!(
        lead.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [60, 62, 64, 67, 71]
    );
    let tied = lead.notes[2];
    assert!(tied.start == 960 && tied.end == 960 + 3 * 480);
    assert!(lead.notes[3].start == 4 * 480 + 2 * 480 && lead.notes[4].start == lead.notes[3].start);
    let bass = m.sources.iter().find(|s| s.name == "Bass").unwrap();
    assert_eq!(bass.program, Some(33));
    assert_eq!(
        bass.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [36, 43]
    );
    let kit = m.sources.iter().find(|s| s.name == "Kit").unwrap();
    assert!(kit.is_drums());
    assert_eq!(
        kit.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [35, 38]
    ); // midi-unpitched is 1-based
    assert!(m.warnings.iter().any(|w| w == "graceNotes: 1"));
}

#[test]
fn converts_through_the_same_role_pipeline_as_midi() {
    let r = ScoreImport::convert(
        SAMPLE_XML.as_bytes(),
        "/x/tiny.musicxml",
        &Options::default(),
    )
    .unwrap();
    let env = &r.envelope;
    assert_eq!(env.get("tempo").unwrap().as_f64(), Some(90.0));
    assert_eq!(env.get("name").unwrap().as_str(), Some("Tiny Score"));
    assert_eq!(places(env, "kick"), 32);
    assert_eq!(note_at(env, "kick", "t0"), Some(36));
    assert_eq!(note_at(env, "snare", "t8"), Some(38));
    assert_eq!(dur_at(env, "bass", "t0"), Some(16));
    assert_eq!(dur_at(env, "melody", "t8"), Some(12)); // the tie
    assert_eq!(note_at(env, "melody", "t24"), Some(67));
    assert_eq!(note_at(env, "melody-b", "t24"), Some(71));
    assert_eq!(
        net(env, "bass")
            .unwrap()
            .get("track")
            .unwrap()
            .get("instrument")
            .unwrap()
            .as_str(),
        Some("bass")
    );
    let w: Vec<&str> = r
        .summary
        .get("warnings")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect();
    assert!(w.contains(&"graceNotes: 1"));
}

#[test]
fn a_zipped_mxl_unpacks_through_its_container() {
    let r = ScoreImport::convert(&sample_mxl(), "/x/tiny.mxl", &Options::default()).unwrap();
    assert_eq!(r.summary.get("title").unwrap().as_str(), Some("Tiny Score"));
    assert_eq!(r.summary.get("parts").unwrap().as_array().unwrap().len(), 4); // kick snare bass melody
}

#[test]
fn refuses_what_it_cannot_read_xml() {
    assert!(import::musicxml::parse(b"<html><body/></html>").is_err());
    assert!(
        ScoreImport::convert(b"hello world, not midi", "/x/song.mp3", &Options::default()).is_err()
    );
    assert!(import::musicxml::parse(b"<score-timewise/>")
        .unwrap_err()
        .to_string()
        .contains("score-timewise"));
}
