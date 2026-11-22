//! Mashups from stems (docs/MASHUPS.md): drums from one generated track, bass from another, the
//! melody from a third, beat-synced, key-aware, content-addressed with parent CIDs.

use djengine::json::JsonValue;
use djengine::key::Key;
use djengine::loopsets::Side;
use djengine::mashup::{Mashup, Source};
use djengine::stems::{DemucsStems, NetStems, StemError, StemSource};

fn three_way() -> Mashup {
    let mut m = Mashup::new("Three Way");
    m.add_source(Source::parse("A", "techno:7").unwrap()).unwrap();
    m.add_source(Source::parse("B", "dnb:2602").unwrap()).unwrap();
    m.add_source(Source::parse("C", "house:42").unwrap()).unwrap();
    m.assign("drums", "A", "drums").unwrap();
    m.assign("bass", "B", "bass").unwrap();
    m.assign("melody", "C", "melody").unwrap();
    m
}

#[test]
fn a_generated_tracks_stems_are_its_nets() {
    let p = Source::net("A", "techno", 7, Side::A).project().unwrap();
    let stems = NetStems.stems(Some(&p)).unwrap();
    let ids: Vec<&str> = stems.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["drums", "bass", "harmony", "melody"]);
    let drums = &stems[0];
    assert!(drums.nets.contains(&"kick".to_string()) && drums.nets.contains(&"snare".to_string()));
    assert!(!drums.class.tonal() && stems[1].class.tonal());
}

#[test]
fn demucs_is_a_stub_that_says_so() {
    assert!(!DemucsStems.available());
    let planned: Vec<String> = DemucsStems.stems(None).unwrap().into_iter().map(|s| s.id).collect();
    assert_eq!(planned, ["drums", "bass", "other", "vocals"]);
    match DemucsStems.audio_stem(&[&[0.0; 4]], 44_100.0, "drums") {
        Err(StemError::Unavailable(e)) => assert!(e.contains("demucs-rs")),
        other => panic!("{other:?}"),
    }
    // an audio part is modelled, and realising it is the clear error
    let mut m = Mashup::new("x");
    m.add_source(Source::audio("A", "zAudio", "song.wav", Key::parse("Am"), Some(120.0))).unwrap();
    m.assign("vocals", "A", "vocals").unwrap();
    assert!(m.realize().unwrap_err().contains("not built in yet"));
}

#[test]
fn sources_carry_their_key_and_tempo() {
    let m = three_way();
    let a = m.source("A").unwrap();
    assert_eq!(a.bpm, Some(128.0));
    assert_eq!(a.key, Some(Key::new(6, false)), "techno:7 is F# major");
    assert_eq!(m.source("B").unwrap().key, Some(Key::new(7, true)), "dnb:2602 is G minor");
    assert_eq!(m.bpm, 128.0, "the first source sets the tempo");
    assert!(Source::parse("X", "nope:1").is_err());
    assert!(Source::parse("X", "techno:x").is_err());
}

#[test]
fn clashing_keys_are_warned_and_auto_key_fixes_them() {
    let mut m = three_way();
    // drums carry no key: the target is the bass's source (G minor), melody from A minor clashes
    assert_eq!(m.target_key(), Some(Key::new(7, true)));
    let clashes = m.clashes();
    assert_eq!(clashes.len(), 1, "{clashes:?}");
    assert_eq!(clashes[0].slot, "melody");
    let moved = m.auto_key();
    assert_eq!(moved.len(), 1);
    assert!(m.clashes().is_empty(), "after auto_key nothing clashes");
    let mel = m.parts.iter().find(|p| p.slot == "melody").unwrap();
    assert!(mel.transpose.abs() <= 6 && mel.transpose != 0);
}

#[test]
fn a_mashup_is_content_addressed_with_its_parents() {
    let m = three_way();
    let parents = m.parents();
    assert_eq!(parents.len(), 3);
    assert_eq!(parents[0], m.source("A").unwrap().cid());
    assert!(parents.iter().all(|c| c.starts_with('z')));
    // the same content is the same CID; any change is a new one
    let again = Mashup::from_json(&JsonValue::parse(&m.canonical()).unwrap()).unwrap();
    assert_eq!(again.cid(), m.cid());
    let mut louder = m.clone();
    louder.part_mut("bass").unwrap().level = 0.5;
    assert_ne!(louder.cid(), m.cid());
    // a fork points back
    let f = m.fork(Some("Three Way (edit)"));
    assert_eq!(f.forked_from.as_deref(), Some(m.cid().as_str()));
    assert_ne!(f.cid(), m.cid());
    assert_eq!(f.to_json().get("forkedFrom").and_then(|v| v.as_str()), Some(m.cid().as_str()));
}

#[test]
fn realising_takes_each_parts_nets_at_one_tempo_and_key() {
    let mut m = three_way();
    m.auto_key();
    let t = m.parts.iter().find(|p| p.slot == "melody").unwrap().transpose;
    m.part_mut("bass").unwrap().level = 0.5;
    let payload = m.realize().unwrap();
    assert_eq!(payload.get("tempo").and_then(JsonValue::as_f64), Some(128.0));
    assert_eq!(payload.get("mashup").and_then(JsonValue::as_str), Some(m.cid().as_str()));
    let nets = payload.get("nets").and_then(JsonValue::as_object).unwrap();
    let ids: Vec<&str> = nets.iter().map(|(k, _)| k).collect();
    for want in ["kick", "snare", "hihat", "bass", "melody"] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    assert!(!ids.contains(&"harmony"), "harmony was not assigned");
    // the melody moved by its transpose, the drums never
    let c = djengine::mashup::Source::parse("C", "house:42").unwrap().project().unwrap();
    let orig = c.nets.get("melody").unwrap();
    let got = djengine::pflow::NetBundle::from_json(nets.iter().find(|(k, _)| *k == "melody").unwrap().1);
    let (k0, b0) = orig.bindings.iter().next().unwrap();
    assert_eq!(got.bindings.get(k0).unwrap().note, b0.note + t as i64);
    // the bass at half level: half the velocity
    let b = djengine::mashup::Source::parse("B", "dnb:2602").unwrap().project().unwrap();
    let ob = b.nets.get("bass").unwrap();
    let gb = djengine::pflow::NetBundle::from_json(nets.iter().find(|(k, _)| *k == "bass").unwrap().1);
    let (k1, b1) = ob.bindings.iter().next().unwrap();
    assert_eq!(gb.bindings.get(k1).unwrap().velocity, ((b1.velocity as f64) * 0.5).round() as i64);
    // the payload is a share a deck can play: it parses into a project with those nets
    let src = djengine::mashup::Source::raw("M", payload.clone());
    let p = src.project().expect("a raw-nets project");
    assert_eq!(p.tempo, 128.0);
    assert_eq!(p.nets.len(), nets.len());
    assert_eq!(djengine::mashup::mashup_of(&payload), Some(m.cid().as_str()));
}

#[test]
fn mute_and_solo_pick_what_sounds() {
    let mut m = three_way();
    m.part_mut("drums").unwrap().mute = true;
    let slots: Vec<&str> = m.audible().iter().map(|p| p.slot.as_str()).collect();
    assert_eq!(slots, ["bass", "melody"]);
    m.part_mut("melody").unwrap().solo = true;
    let slots: Vec<&str> = m.audible().iter().map(|p| p.slot.as_str()).collect();
    assert_eq!(slots, ["melody"]);
    let nets = m.realize().unwrap();
    let ids: Vec<String> = nets.get("nets").and_then(JsonValue::as_object).unwrap().iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(ids, ["melody"]);
    m.part_mut("melody").unwrap().mute = true;
    // the only solo is muted: back to every unmuted part
    let slots: Vec<&str> = m.audible().iter().map(|p| p.slot.as_str()).collect();
    assert_eq!(slots, ["bass"]);
}

#[test]
fn two_sources_bringing_the_same_net_are_renamed_apart() {
    let mut m = Mashup::new("double bass");
    m.add_source(Source::parse("A", "techno:7").unwrap()).unwrap();
    m.add_source(Source::parse("B", "house:42").unwrap()).unwrap();
    m.assign("bass", "A", "bass").unwrap();
    m.assign("bass2", "B", "bass").unwrap();
    let p = m.realize().unwrap();
    let ids: Vec<String> = p.get("nets").and_then(JsonValue::as_object).unwrap().iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(ids, ["bass", "bass_b"]);
    assert!(m.assign("x", "A", "vocals").unwrap_err().contains("no stem"));
    m.remove_source("B").unwrap();
    assert_eq!(m.parts.len(), 1, "a source's parts go with it");
}
