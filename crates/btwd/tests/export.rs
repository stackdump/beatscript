//! `btwd export` in-process (docs/EXPORT.md): the MIDI file read back against the engine's own notes, the stems
//! added up against the master, and the same program giving the same bytes.

#![allow(dead_code)]

#[path = "../src/render.rs"]
mod render;
#[path = "../src/export.rs"]
mod export;

use std::path::PathBuf;
use std::time::Instant;

use djengine::import::midi as imidi;
use djengine::smf;
use export::{ExportOptions, Rights};

/// Two sections, two decks, every kind of part: a kick on the quarters at a fixed velocity, hats with a `vel=` range
/// (the engine's seeded picks), a bass, a chord pad, a melody. 4 bars at 128.
const SONG: &str = "#! name: export-test\ntempo 128\nkey am\nsection one 2 bars {\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ vel=112 | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ vel=70..95 | bass ~ a2 a2 a2 ~ a2 a2 a2 ~ a2 a2 a2 ~ a2 a2 a2 inst=sub-bass\n  deck B: harmony chord(am) inst=warm-pad len=16 vel=80\n}\nsection two 2 bars {\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ vel=112 | bass ~ f2 f2 f2 ~ f2 f2 f2 ~ f2 f2 f2 ~ f2 f2 f2 inst=sub-bass\n  deck B: harmony chord(f) inst=warm-pad len=16 vel=80 | melody e5 ~ ~ ~ ~ ~ d5 ~ c5 ~ ~ ~ ~ ~ ~ ~ inst=detuned-saw len=8 vel=105\n}\narrange one two\n";

/// A tempo that moves: 120 -> 140 over 2 bars, then held.
const RAMP: &str = "tempo 120 -> 140 over 2 bars\nsection one 4 bars {\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n}\narrange one\n";

fn opts() -> ExportOptions {
    ExportOptions { sr: 44_100, max_bars: 64, rights: Rights::default(), threads: 1 }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("btwd-export-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn the_midi_file_holds_the_notes_the_engine_played() {
    let e = export::midi(SONG, &opts()).unwrap();
    let p = imidi::parse(&e.midi).unwrap();
    assert_eq!(p.format, 1);
    assert_eq!(p.ticks_per_quarter, i64::from(smf::PPQ));
    let meta = smf::read_meta(&e.midi).unwrap();
    // a conductor track, then a track per part that played, named deck and role, in the program's order
    let names: Vec<&str> = meta.names.iter().map(String::as_str).collect();
    assert_eq!(names, vec!["export-test", "A kick", "A hihat", "A bass", "B harmony", "B melody"]);
    assert_eq!(e.parts.len(), 5);
    // every part's notes, read back, are the engine's: step, key (GM drums on channel 10), velocity
    for (k, part) in e.parts.iter().enumerate() {
        let track = (k + 1) as i64;
        let got: Vec<(i64, i64, i64, i64)> = p.sources.iter().filter(|s| s.track == track).flat_map(|s| s.notes.iter().map(move |n| (n.start, n.pitch, n.velocity, s.channel))).collect();
        let mut want: Vec<(i64, i64, i64, i64)> = e
            .played
            .iter()
            .filter(|n| n.deck == part.deck && n.part == part.role)
            .map(|n| (n.step * i64::from(smf::STEP_TICKS), i64::from(if part.drum { smf::gm_drum_note(&part.role, n.note) } else { n.note }), i64::from(n.velocity), i64::from(part.channel)))
            .collect();
        want.sort();
        want.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
        let mut got = got;
        got.sort();
        assert_eq!(got, want, "{}", part.title());
        assert_eq!(part.notes, e.played.iter().filter(|n| n.deck == part.deck && n.part == part.role).count());
    }
    // and the engine's notes are the program's: the kick on the quarters, every bar, at 112, on channel 10
    let kick: Vec<(i64, i64, i64, i64)> = p.sources.iter().filter(|s| s.track == 1).flat_map(|s| s.notes.iter().map(move |n| (n.start, n.pitch, n.velocity, s.channel))).collect();
    let want: Vec<(i64, i64, i64, i64)> = (0..16).map(|q| (q * 480, 36, 112, 9)).collect();
    assert_eq!(kick, want);
    // the hats' velocities are the seeded picks inside 70..95, not all one value
    let hats: Vec<i64> = p.sources.iter().filter(|s| s.track == 2).flat_map(|s| s.notes.iter().map(|n| n.velocity)).collect();
    assert_eq!(hats.len(), 8, "two bars of four hats");
    assert!(hats.iter().all(|v| (70..=95).contains(v)), "{hats:?}");
    assert!(hats.iter().any(|v| *v != hats[0]), "{hats:?}");
    // tonal parts: their own channels (not 10), and the program change of their instrument
    let bass = &e.parts[2];
    assert_eq!((bass.role.as_str(), bass.drum, bass.channel, bass.program), ("bass", false, 0, Some(38)));
    assert!(meta.programs.contains(&(3, 0, 38)), "{:?}", meta.programs);
    assert!(meta.programs.contains(&(4, 1, 89)), "warm pad: {:?}", meta.programs);
    // the conductor: 128 bpm, A minor (no sharps), a marker at each section, the song's end
    assert_eq!(meta.tempos, vec![(0, 468_750)]);
    assert_eq!(meta.key, Some(smf::KeySig { sharps: 0, minor: true }));
    assert_eq!(meta.markers, vec![(0, "one".to_string()), (2 * 4 * 480, "two".to_string())]);
    assert!(meta.ends.iter().all(|t| *t >= 4 * 4 * 480));
    assert_eq!(meta.copyright.as_deref(), Some("License: CC0-1.0"));
}

#[test]
fn a_tempo_ramp_is_a_tempo_map() {
    let e = export::midi(RAMP, &opts()).unwrap();
    assert!(e.tempo_curve);
    let m = smf::read_meta(&e.midi).unwrap();
    let bpm: Vec<f64> = m.tempos.iter().map(|(_, us)| 60_000_000.0 / f64::from(*us)).collect();
    assert!(m.tempos.len() > 4, "a staircase: {:?}", m.tempos);
    assert!((bpm[0] - 120.0).abs() < 1.5, "{bpm:?}");
    assert!(bpm.windows(2).all(|w| w[1] >= w[0] - 1e-9), "rising: {bpm:?}");
    assert!((bpm.last().unwrap() - 140.0).abs() < 0.5, "{bpm:?}");
    // the ramp is over by bar 2: no tempo event after it
    assert!(m.tempos.iter().all(|(t, _)| *t <= 2 * 4 * 480), "{:?}", m.tempos);
    assert!(e.notes.iter().any(|n| n.contains("tempo moves")));
}

#[test]
fn the_same_program_gives_the_same_midi_bytes() {
    let a = export::midi(SONG, &opts()).unwrap();
    let b = export::midi(SONG, &opts()).unwrap();
    assert_eq!(a.midi, b.midi);
    assert_eq!(a.cid, b.cid);
    // the stored rights are the copyright line
    let mut o = opts();
    o.rights = Rights { license: Some("CC-BY-4.0".into()), author: Some("someone".into()), ..Rights::default() };
    let c = export::midi(SONG, &o).unwrap();
    assert_eq!(smf::read_meta(&c.midi).unwrap().copyright.as_deref(), Some("someone. License: CC-BY-4.0"));
}

#[test]
fn the_stems_add_up_to_the_master_and_are_the_same_bytes_every_time() {
    let (d1, d2) = (tmp("a"), tmp("b"));
    let e = export::stems(SONG, &opts(), &d1, &mut |_| {}).unwrap();
    // a file per deck and per part, the master, the MIDI and the manifest
    let files: Vec<&str> = e.files.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(files, vec!["master.wav", "deck-A.wav", "deck-B.wav", "A-kick.wav", "A-hihat.wav", "A-bass.wav", "B-harmony.wav", "B-melody.wav", "song.mid", "manifest.json"]);
    let (p, d) = (e.parts_sum.unwrap(), e.decks_sum.unwrap());
    assert!(p.peak > 0.1, "the master is not silent: {p:?}");
    // float rounding only: the limiter's gains are replayed on every part
    assert!(p.max_abs < 1e-4 && p.rms < 1e-5, "parts: {p:?}");
    assert!(d.max_abs < 1e-4 && d.rms < 1e-5, "decks: {d:?}");
    // every stem has its part in it, and only its part: the kick's stem is silent where the kick is
    for (f, _) in e.files.iter().filter(|(_, k)| *k == "part" || *k == "deck") {
        let mut w = export::WavIn::open(&d1.join(f)).unwrap();
        let x = w.read(usize::MAX >> 4).unwrap();
        assert!(x.iter().any(|v| v.abs() > 1e-3), "{f} is silent");
    }
    // the stems' MIDI is the MIDI export's
    assert_eq!(std::fs::read(d1.join("song.mid")).unwrap(), export::midi(SONG, &opts()).unwrap().midi);
    // the manifest: the CID, the tempo, the key, the license, a line per part
    let man = std::fs::read_to_string(d1.join("manifest.json")).unwrap();
    for s in [format!("\"cid\":\"{}\"", e.cid), "\"tempo\":128".into(), "\"key\":\"am\"".into(), "\"license\":\"CC0-1.0\"".into(), "\"generator\":\"BeatScript\"".into(), "\"file\":\"A-kick.wav\"".into(), "\"gm_program\":39".into()] {
        assert!(man.contains(&s), "{s} in {man}");
    }
    // the zip, twice (the second time on three threads): the same bytes
    let e2 = export::stems(SONG, &ExportOptions { threads: 3, ..opts() }, &d2, &mut |_| {}).unwrap();
    let (mut z1, mut z2) = (Vec::new(), Vec::new());
    export::zip(&d1, &export::bundle_order(&e), "x", &mut z1).unwrap();
    export::zip(&d2, &export::bundle_order(&e2), "x", &mut z2).unwrap();
    assert!(z1 == z2, "the same zip");
    assert_eq!(&z1[..4], b"PK\x03\x04");
    assert_eq!(&z1[30..30 + "x/manifest.json".len()], b"x/manifest.json");
    let _ = std::fs::remove_dir_all(&d1);
    let _ = std::fs::remove_dir_all(&d2);
}

#[test]
fn the_stems_master_is_the_render() {
    let d = tmp("m");
    export::stems(SONG, &opts(), &d, &mut |_| {}).unwrap();
    let mut w = export::WavIn::open(&d.join("master.wav")).unwrap();
    let master = w.read(usize::MAX >> 4).unwrap();
    let o = render::Options { sr: 44_100, format: render::Format::F32le, ..render::Options::default() };
    let mut pcm = Vec::new();
    let mut err = Vec::new();
    let mut ev = render::Events::new(&mut err);
    render::render_to(SONG, &o, &mut pcm, &mut ev, Instant::now()).unwrap();
    let streamed: Vec<f32> = pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert_eq!(master.len(), streamed.len());
    assert!(master == streamed, "the stems' master is the streamed render, sample for sample");
    let _ = std::fs::remove_dir_all(&d);
}

/// A melody with a `jux`: its twin net (`melody_r`) is the part's right copy, so it is in the part's MIDI track and stem, not a
/// part of its own.
const JUX: &str = "#! name: jux-export\ntempo 120\nkey am\nlet lead = c4 ~ ~ ~ e4 ~ ~ ~\nsection s 2 bars {\n  deck A: kick x ~ ~ ~ | melody lead\n}\narrange {\n  ~ lead jux rot 2\n  s\n}\n";

#[test]
fn the_right_copy_of_a_jux_is_in_its_part_in_the_midi_and_in_the_stems() {
    let e = export::midi(JUX, &opts()).unwrap();
    let names: Vec<String> = smf::read_meta(&e.midi).unwrap().names;
    assert_eq!(names, vec!["jux-export", "A kick", "A melody"], "no `melody_r` part");
    // both copies' notes are the melody's: four notes a bar over two bars from the left copy, the same again from the right
    let melody = e.parts.iter().find(|p| p.role == "melody").unwrap();
    assert_eq!(melody.notes, 8, "{:?}", e.played);
    assert!(e.played.iter().all(|n| n.part != "melody_r"));
    // the stems: a file for the part (both copies in it, so it is on both sides), none for the twin, and they add up to the master
    let d = tmp("jux");
    let s = export::stems(JUX, &opts(), &d, &mut |_| {}).unwrap();
    let files: Vec<&str> = s.files.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(files, vec!["master.wav", "deck-A.wav", "A-kick.wav", "A-melody.wav", "song.mid", "manifest.json"]);
    let p = s.parts_sum.unwrap();
    assert!(p.max_abs < 1e-4 && p.rms < 1e-5, "parts: {p:?}");
    let _ = std::fs::remove_dir_all(&d);
}
