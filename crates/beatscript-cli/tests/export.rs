//! `beatscript export` in-process (docs/EXPORT.md): the MIDI file read back against the engine's own notes, the stems
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
    ExportOptions { sr: 44_100, max_bars: 64, rights: Rights::default(), threads: 1, dry: false }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("beatscript-export-test-{}-{name}", std::process::id()));
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

/// The dubstep example (beatscript 2): a master filter swept in every section, a master reverb, deck levels, a deck filter and
/// resonance ramps, a riser and a DJ move. Its first 24 bars: the intro, the build and half the drop.
const DUBSTEP: &str = include_str!("../../../site/examples/hand-dubstep-140-v4.beatscript");

fn wav(path: &std::path::Path) -> Vec<f32> {
    export::WavIn::open(path).unwrap().read(usize::MAX >> 4).unwrap()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Bars `from..to` of an interleaved stereo stem at 140 bpm.
fn bars(x: &[f32], from: u32, to: u32) -> &[f32] {
    let frame = |b: u32| ((f64::from(b) * 4.0 * 60.0 / 140.0 * 44_100.0).round() as usize * 2).min(x.len());
    &x[frame(from)..frame(to)]
}

#[test]
fn dry_stems_are_each_part_before_its_deck_and_the_master_with_the_moves_in_the_manifest() {
    let o = ExportOptions { max_bars: 24, threads: 4, dry: true, ..opts() };
    let d = tmp("dry");
    let e = export::stems(DUBSTEP, &o, &d, &mut |_| {}).unwrap();
    // a file per part, the master, the MIDI and the manifest; no deck stems
    let files: Vec<&str> = e.files.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(files, vec!["master.wav", "A-kick.wav", "A-hihat.wav", "A-snare.wav", "A-bass.wav", "B-harmony.wav", "B-melody.wav", "wheel.wav", "song.mid", "manifest.json"]);
    assert_eq!(export::bundle_folder(&e), format!("beatscript-{}-dry", &e.cid[..12]));
    for p in &e.parts {
        assert!(wav(&d.join(p.file())).iter().any(|v| v.abs() > 1e-3), "{} is silent", p.file());
    }
    // the same notes as the wet export, and its master is the mix
    let w = tmp("wet");
    let wet = export::stems(DUBSTEP, &ExportOptions { dry: false, ..o.clone() }, &w, &mut |_| {}).unwrap();
    assert_eq!(e.played, wet.played);
    assert!(wav(&d.join("master.wav")) == wav(&w.join("master.wav")), "the dry bundle's master is the mix");
    // the intro: deck A at level 0.4 under a master lowpass at 400 Hz. Dry, the hats are the hats; wet, they are nearly gone
    let (dh, wh) = (wav(&d.join("A-hihat.wav")), wav(&w.join("A-hihat.wav")));
    let (dry_intro, wet_intro) = (rms(bars(&dh, 0, 2)), rms(bars(&wh, 0, 2)));
    assert!(dry_intro > 4.0 * wet_intro, "the intro's hats: dry {dry_intro}, wet {wet_intro}");
    // no master and no deck processing: the master's and the decks' lines moved somewhere else give the same dry stems,
    // sample for sample (and different wet ones)
    let moved = DUBSTEP
        .replace("master: filter lp", "master: filter hp")
        .replace("master: reverb 0.15", "master: reverb 0.6")
        .replace("deck A: level 0.4", "deck A: level 0.9")
        .replace("deck A: level 0.9\n  deck B: level 0.9", "deck A: level 0.2\n  deck B: level 0.5");
    assert_ne!(moved, DUBSTEP);
    let d2 = tmp("dry2");
    let e2 = export::stems(&moved, &o, &d2, &mut |_| {}).unwrap();
    assert_ne!(e2.cid, e.cid);
    for p in &e.parts {
        assert!(wav(&d.join(p.file())) == wav(&d2.join(p.file())), "{}: the master's and the decks' lines reach the dry stem", p.file());
    }
    let w2 = tmp("wet2");
    export::stems(&moved, &ExportOptions { dry: false, ..o.clone() }, &w2, &mut |_| {}).unwrap();
    assert!(wav(&w.join("A-hihat.wav")) != wav(&w2.join("A-hihat.wav")), "the wet stem hears them");
    // the manifest: dry, no sum check, the master's lines, and every removed move with its bar time
    let man = std::fs::read_to_string(d.join("manifest.json")).unwrap();
    let v = djengine::json::JsonValue::parse(&man).unwrap();
    assert_eq!(v.get("stems").and_then(|s| s.get("dry")).and_then(|x| x.as_bool()), Some(true), "{man}");
    assert!(v.get("stems").and_then(|s| s.get("parts_sum_check")).is_some_and(|x| matches!(x, djengine::json::JsonValue::Null)));
    let lines = |k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let text = |a: &djengine::json::JsonValue| a.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let master = lines("master_automation");
    let sweep = master.iter().find(|a| text(a) == "master: filter lp 400 -> 6000 over 8 bars").unwrap_or_else(|| panic!("the intro's sweep in {man}"));
    let series = sweep.get("series").and_then(|s| s.as_array()).unwrap();
    let pts = series[0].get("points").and_then(|p| p.as_array()).unwrap();
    assert!(pts.len() > 8, "a ramp's points: {pts:?}");
    let first = pts[0].as_array().unwrap();
    let last = pts[pts.len() - 1].as_array().unwrap();
    assert_eq!(first[0].as_f64(), Some(0.0));
    assert!(last[0].as_f64().unwrap() <= 32.0 && last[0].as_f64().unwrap() > 28.0, "it ends at bar 9: {last:?}");
    assert!(master.iter().any(|a| text(a).starts_with("master: reverb")), "{man}");
    let fx = v.get("stems").and_then(|s| s.get("master_fx_lines")).and_then(|x| x.as_array()).unwrap();
    assert!(fx.iter().any(|l| l.as_str() == Some("master: filter lp 400 -> 6000 over 8 bars")), "{fx:?}");
    // a part carries its deck's lines: deck A's level in the intro, at bar 1, and its level in the build, at bar 9
    let parts = lines("parts");
    let kick = parts.iter().find(|p| p.get("file").and_then(|f| f.as_str()) == Some("A-kick.wav")).unwrap();
    let auto = kick.get("automation").and_then(|a| a.as_array()).unwrap();
    let level = |t: &str| auto.iter().find(|a| text(a) == t).unwrap_or_else(|| panic!("{t} in {auto:?}")).clone();
    let intro = level("deck A: level 0.4");
    assert_eq!(intro.get("scope").and_then(|s| s.as_str()), Some("deck"));
    assert_eq!(intro.get("fires").and_then(|f| f.as_array()).map(|f| f.iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>()), Some(vec![0.0]));
    assert!(intro.get("series").and_then(|s| s.as_array()).is_some_and(|s| s.iter().any(|x| x.get("param").and_then(|p| p.as_str()) == Some("volume"))), "{intro:?}");
    assert!(level("deck A: level 0.9").get("fires").and_then(|f| f.as_array()).is_some_and(|f| f.iter().any(|x| x.as_f64() == Some(32.0))));
    // deck B's parts carry deck B's lines, not deck A's
    let pad = parts.iter().find(|p| p.get("file").and_then(|f| f.as_str()) == Some("B-harmony.wav")).unwrap();
    assert!(pad.get("automation").and_then(|a| a.as_array()).unwrap().iter().all(|a| a.get("deck").and_then(|x| x.as_str()) == Some("B")));
    // the wet manifest has none of it
    let wman = std::fs::read_to_string(w.join("manifest.json")).unwrap();
    assert!(!wman.contains("automation") && !wman.contains("\"dry\""), "{wman}");
    // the dry export is deterministic too
    let d3 = tmp("dry3");
    let e3 = export::stems(DUBSTEP, &ExportOptions { threads: 1, ..o.clone() }, &d3, &mut |_| {}).unwrap();
    let (mut z1, mut z3) = (Vec::new(), Vec::new());
    export::zip(&d, &export::bundle_order(&e), "x", &mut z1).unwrap();
    export::zip(&d3, &export::bundle_order(&e3), "x", &mut z3).unwrap();
    assert!(z1 == z3, "the same dry zip");
    for x in [d, d2, d3, w, w2] {
        let _ = std::fs::remove_dir_all(x);
    }
}
