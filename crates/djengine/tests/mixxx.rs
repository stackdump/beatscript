//! Mixxx mapping support, on a mapping written for these tests (never a copy of one of
//! Mixxx's files: those are GPL, docs/MIXXX-MAPPINGS.md).

use djengine::control::surface::{ControlType, DJActionKind as K};
use djengine::mixxx::{self, discover, profile::Why, vocab};

/// A small controller: two decks, a mixer, a crossfader, a script-driven jog, a Latin-1 name
/// and an entity, so the reader's corners are on one page.
const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE MixxxMIDIPreset [ <!ENTITY noteOn "0x90"> ]>
<MixxxMIDIPreset mixxxVersion="2.2" schemaVersion="1">
  <info><name>Acme Pad Two</name><author>Test &amp; Co</author><description>for the tests</description></info>
  <controller id="Acme Pad Two">
    <scriptfiles><file filename="acme.js" functionprefix="Acme"/></scriptfiles>
    <controls>
      <control><group>[Channel1]</group><key>play</key><status>&noteOn;</status><midino>0x01</midino><options><normal/></options></control>
      <control><group>[Channel1]</group><key>play</key><status>0x80</status><midino>0x01</midino><options><normal/></options></control>
      <control><group>[Channel2]</group><key>play</key><status>0x91</status><midino>0x01</midino><normal/></control>
      <control><group>[Channel1]</group><key>cue_default</key><status>0x90</status><midino>2</midino></control>
      <control><group>[Channel1]</group><key>hotcue_3_activate</key><status>0x90</status><midino>0x10</midino></control>
      <control><group>[Channel1]</group><key>hotcue_3_clear</key><status>0x90</status><midino>0x11</midino></control>
      <control><group>[Channel1]</group><key>volume</key><status>0xB0</status><midino>0x07</midino><options><normal/></options></control>
      <control><group>[Channel2]</group><key>volume</key><status>0xB1</status><midino>0x07</midino><options><invert/></options></control>
      <control><group>[Channel1]</group><key>filterLow</key><status>0xB0</status><midino>0x08</midino></control>
      <control><group>[Channel1]</group><key>rate</key><status>0xB0</status><midino>0x09</midino><options><soft-takeover/></options></control>
      <control><group>[Master]</group><key>crossfader</key><status>0xB0</status><midino>0x0A</midino></control>
      <control><group>[Master]</group><key>crossfader</key><status>0xB0</status><midino>0x0A</midino><options><script-binding/></options></control>
      <control><group>[Channel3]</group><key>play</key><status>0x92</status><midino>0x01</midino></control>
      <control><group>[Channel1]</group><key>beatloop_4_toggle</key><status>0x90</status><midino>0x20</midino></control>
      <control><group>[Channel1]</group><key>jog</key><status>0xB0</status><midino>0x30</midino><options><rot64/></options></control>
      <control><group>[Channel2]</group><key>jog</key><status>0xB0</status><midino>0x31</midino><options><selectknob/></options></control>
      <control><group>[Channel1]</group><key>volume</key><status>0x90</status><midino>0x40</midino></control>
      <control><group>[Master]</group><key>volume</key><status>0xB0</status><midino>0x0B</midino><options><fourteen-bit-msb/></options></control>
      <control><group>[Master]</group><key>volume</key><status>0xB0</status><midino>0x2B</midino><options><fourteen-bit-lsb/></options></control>
      <control><group>[Channel1]</group><key>AcmeDeck.wheel</key><status>0xB0</status><midino>0x31</midino><options><script-binding/></options></control>
    </controls>
    <outputs>
      <output><group>[Channel1]</group><key>play</key><status>0x90</status><midino>0x01</midino><on>0x7F</on><off>0x00</off><minimum>0.5</minimum></output>
      <output><group>[Channel1]</group><key>cue_default</key><status>0x90</status><midino>0x02</midino><on>0x7F</on><off>0x00</off></output>
      <output><group>[Channel1]</group><key>pfl</key><status>0xB0</status><midino>0x05</midino><on>0x7F</on><off>0x00</off></output>
    </outputs>
  </controller>
</MixxxMIDIPreset>"#;

fn built() -> mixxx::MixxxProfile {
    mixxx::build(&mixxx::parse(FIXTURE.as_bytes()).unwrap(), "Acme Pad Two MIDI 1")
}

#[test]
fn the_reader_gets_the_info_controls_options_outputs_and_entities() {
    let p = mixxx::parse(FIXTURE.as_bytes()).unwrap();
    assert_eq!((p.name.as_str(), p.author.as_str()), ("Acme Pad Two", "Test & Co"));
    assert_eq!(p.controller_id, "Acme Pad Two");
    assert_eq!(p.scripts[0].filename, "acme.js");
    assert_eq!(p.scripts[0].prefix, "Acme");
    assert_eq!(p.controls.len(), 20);
    // the entity, hex and decimal, and the status' channel
    assert_eq!(p.controls[0].status, 0x90);
    assert_eq!(p.controls[3].midino, 2);
    assert_eq!(p.controls[2].channel(), 1);
    // options inside <options>, bare under <control>, and several kinds
    assert!(p.controls[7].options.invert);
    assert!(p.controls[9].options.soft_takeover);
    assert!(p.controls[11].options.script_binding);
    assert!(p.controls[14].options.rot64 && p.controls[14].options.is_relative());
    assert!(p.controls[15].options.relative && !p.controls[15].options.rot64);
    assert!(p.controls[17].options.fourteen_bit_msb && p.controls[18].options.fourteen_bit_lsb);
    // outputs with their on / off and thresholds
    assert_eq!(p.outputs.len(), 3);
    assert_eq!((p.outputs[0].on, p.outputs[0].off, p.outputs[0].minimum), (0x7F, 0, 0.5));
}

#[test]
fn a_file_that_is_not_a_mixxx_mapping_is_an_error_not_a_panic() {
    assert!(mixxx::parse(b"<html/>").is_err());
    assert!(mixxx::parse(b"not xml at all").is_err());
    assert!(mixxx::parse(b"").is_err());
    // the newer root name is read, and Latin-1 bytes do not stop it
    let latin = b"<MixxxControllerPreset><info><name>Caf\xe9 Pad</name></info></MixxxControllerPreset>";
    assert!(mixxx::parse(latin).unwrap().name.starts_with("Caf"));
}

#[test]
fn the_translation_table_maps_the_vocabulary() {
    use vocab::{translate, Drive, Translation::*};
    let t = |g: &str, k: &str| match translate(g, k, false) {
        Maps(t) => t,
        other => panic!("{g} {k}: {other:?}"),
    };
    let ch1 = |k: &str| t("[Channel1]", k);
    assert_eq!((ch1("play").action, ch1("play").deck.as_deref()), (K::Play, Some("A")));
    assert_eq!(t("[Channel2]", "play").deck.as_deref(), Some("B"));
    assert_eq!(ch1("cue_default").action, K::Cue);
    assert_eq!(ch1("pfl").action, K::Pfl);
    assert_eq!(ch1("sync_enabled").action, K::Sync);
    assert_eq!(ch1("volume").action, K::Volume);
    assert_eq!(ch1("volume").drive, Drive::Continuous);
    assert_eq!(ch1("pregain").action, K::Gain);
    assert_eq!(ch1("filterLow").action, K::EqLow);
    assert_eq!(ch1("filterMid").action, K::EqMid);
    assert_eq!(ch1("filterHigh").action, K::EqHigh);
    assert_eq!(ch1("rate").action, K::Pitch);
    assert_eq!((ch1("loop_in").action, ch1("loop_out").action), (K::LoopIn, K::LoopOut));
    assert_eq!((ch1("loop_halve").action, ch1("loop_double").action), (K::LoopHalve, K::LoopDouble));
    // hot cues are 1-based in Mixxx, 0-based here
    let h = ch1("hotcue_4_activate");
    assert_eq!((h.action, h.slot), (K::Hotcue, Some(3)));
    assert_eq!((ch1("hotcue_1_clear").action, ch1("hotcue_1_clear").slot), (K::Clearcue, Some(0)));
    assert_eq!(t("[Master]", "crossfader").action, K::Crossfader);
    assert_eq!(t("[Master]", "volume").action, K::Master);
    assert_eq!(t("[Master]", "headMix").action, K::HeadphoneMix);
    assert_eq!(t("[Master]", "headGain").action, K::HeadphoneLevel);
    // the racks
    let q = t("[QuickEffectRack1_[Channel2]]", "super1");
    assert_eq!((q.action, q.deck.as_deref()), (K::Filter, Some("B")));
    let e = t("[EqualizerRack1_[Channel1]_Effect1]", "parameter3");
    assert_eq!((e.action, e.deck.as_deref()), (K::EqHigh, Some("A")));
    assert_eq!(t("[EffectRack1_EffectUnit2]", "mix").deck.as_deref(), Some("B"));
    assert_eq!(t("[AutoDJ]", "enabled").action, K::AutoMix);
    // approximations say what differs
    assert!(ch1("LoadSelectedTrack").approx.is_some());
    assert!(ch1("play").approx.is_none());
    // what has no target says so, with a reason
    for (g, k) in [("[Channel3]", "play"), ("[Channel1]", "beatloop_4_toggle"), ("[Channel1]", "jog"), ("[Sampler1]", "play"), ("[Library]", "MoveVertical"), ("[Channel1]", "keylock")] {
        match translate(g, k, false) {
            Unsupported(r) => assert!(!r.is_empty(), "{g} {k}"),
            other => panic!("{g} {k}: {other:?}"),
        }
    }
    // a script-bound control is the script's, whatever its name
    assert_eq!(translate("[Channel1]", "play", true), Script);
    // every example the table lists maps
    for (g, k) in vocab::mapped_examples() {
        assert!(matches!(translate(g, k, false), Maps(_)), "{g} {k}");
    }
}

#[test]
fn the_profile_has_a_control_for_every_message_and_a_binding_for_what_maps() {
    let m = built();
    let p = &m.profile;
    assert_eq!(p.name, "Mixxx: Acme Pad Two");
    assert!(p.matches("Acme Pad Two MIDI 1"));
    assert!(!p.matches("APC mini mk2 Control"));
    // note on and note off of one key are one control
    let n1 = p.group("note-ch1").unwrap();
    assert_eq!(n1.kind, ControlType::Note);
    assert_eq!(n1.channel, Some(0));
    assert_eq!(n1.control_numbers(), vec![0x01, 0x02, 0x10, 0x11, 0x20, 0x40]);
    assert_eq!(p.group("note-ch2").unwrap().control_numbers(), vec![0x01]);
    assert!(p.group("cc-ch1").is_some() && p.group("cc-ch2").is_some());
    assert_eq!(p.group("cc14-ch1").unwrap().control_numbers(), vec![0x0B], "the LSB rides with its MSB");
    assert_eq!(p.group("rel-ch1").unwrap().kind, ControlType::Relative);
    let bound = |g: &str, i: usize| p.layout.iter().find(|b| b.group == g && b.index == Some(i));
    let play = bound("note-ch1", 0).unwrap();
    assert_eq!((play.action, play.deck.as_deref()), (K::Play, Some("A")));
    assert_eq!(bound("note-ch2", 0).unwrap().deck.as_deref(), Some("B"));
    assert_eq!(bound("note-ch1", 2).unwrap().slot, Some(2));
    assert_eq!(bound("cc-ch1", 0).unwrap().action, K::Volume);
    assert_eq!(bound("cc14-ch1", 0).unwrap().action, K::Master);
    // every control is named for the monitor and the drawing
    assert_eq!(n1.name(0), Some("[Channel1] play"));
}

#[test]
fn the_report_says_what_did_not_map_and_why() {
    let r = built().report;
    let why = |g: &str, k: &str| r.unmapped.iter().find(|u| u.group == g && u.key == k).map(|u| (u.why, u.reason.clone()));
    assert_eq!(why("[Channel3]", "play").unwrap().0, Why::Unsupported);
    assert!(why("[Channel3]", "play").unwrap().1.contains("two decks"));
    assert_eq!(why("[Channel1]", "beatloop_4_toggle").unwrap().0, Why::Unsupported);
    assert_eq!(why("[Channel1]", "AcmeDeck.wheel").unwrap().0, Why::Script);
    // a button on a fader's target, a fader on nothing
    assert_eq!(why("[Channel1]", "volume").map(|x| x.0), Some(Why::Mismatch));
    // the second claim on a MIDI message is shadowed, or (same control) the same control
    assert_eq!(r.count(Why::Script), 1, "the wheel (the script-bound crossfader sits on a message the plain one owns)");
    // the jog on an encoder maps to nothing, and says so
    assert_eq!(why("[Channel1]", "jog").unwrap().0, Why::Unsupported);
    assert_eq!(r.mapped.len() + r.unmapped.len(), 17, "{r:#?}");
    assert!(r.notes.iter().any(|n| n.contains("script file")));
    // the JSON a caller reads
    let j = r.to_json(true);
    assert_eq!(j.get("mapped").and_then(|v| v.as_f64()), Some(r.mapped.len() as f64));
    assert!(j.get("unmapped_list").and_then(|v| v.as_array()).unwrap().len() == r.unmapped.len());
}

#[test]
fn invert_and_encoders_are_fixed_in_front_of_the_router() {
    let m = built();
    // <invert/> on deck B's volume (ch2 cc 7): 127 - v
    assert_eq!(m.prefilter.apply(&[0xB1, 0x07, 100]), [0xB1, 0x07, 27]);
    assert_eq!(m.prefilter.apply(&[0xB1, 0x07, 0]), [0xB1, 0x07, 127]);
    // deck A's volume is as it came
    assert_eq!(m.prefilter.apply(&[0xB0, 0x07, 100]), [0xB0, 0x07, 100]);
    // <rot64/>: 64 + n → n as two's complement (65 → +1, 63 → -1 = 0x7F)
    assert_eq!(m.prefilter.apply(&[0xB0, 0x30, 65]), [0xB0, 0x30, 1]);
    assert_eq!(m.prefilter.apply(&[0xB0, 0x30, 63]), [0xB0, 0x30, 0x7F]);
    assert_eq!(m.prefilter.apply(&[0xB0, 0x30, 64]), [0xB0, 0x30, 0]);
    // a <selectknob/> is two's complement already
    assert_eq!(m.prefilter.apply(&[0xB1, 0x31, 0x7F]), [0xB1, 0x31, 0x7F]);
    // notes are not touched
    assert_eq!(m.prefilter.apply(&[0x90, 0x01, 127]), [0x90, 0x01, 127]);
}

#[test]
fn the_profile_decodes_what_the_mapping_says_the_controller_sends() {
    let m = built();
    let p = &m.profile;
    let ev = p.parse(&[0x90, 0x10, 100]).unwrap();
    assert_eq!((ev.group.as_str(), ev.index, ev.pressed), ("note-ch1", 2, true));
    let off = p.parse(&[0x80, 0x10, 0]).unwrap();
    assert!(!off.pressed);
    let fader = p.parse(&[0xB1, 0x07, 127]).unwrap();
    assert_eq!((fader.group.as_str(), fader.value), ("cc-ch2", 1.0));
    // a message the mapping does not mention is undecoded, not misread
    assert!(p.parse(&[0x90, 0x7E, 100]).is_none());
    assert!(p.parse(&[0x95, 0x01, 100]).is_none());
}

#[test]
fn leds_come_from_the_note_outputs_whose_notes_are_controls() {
    let m = built();
    let l = m.profile.leds.as_ref().unwrap();
    assert_eq!(l.single_groups, vec!["note-ch1".to_string()]);
    assert_eq!((l.single_on, l.single_off), (0x7F, 0));
    assert_eq!((m.report.outputs_total, m.report.outputs_lit), (3, 2));
}

#[test]
fn discovery_matches_by_name_and_an_override_beats_it() {
    let c = |file: &str, name: &str, id: &str| discover::Candidate { id: format!("/m/{file}"), file: file.into(), name: name.into(), controller_id: id.into() };
    let cands = vec![
        c("Hercules_DJControl_Inpulse_200.midi.xml", "Hercules DJControl Inpulse 200", "DJControl Inpulse 200"),
        c("Hercules_DJControl_Inpulse_300.midi.xml", "Hercules DJControl Inpulse 300", "DJControl Inpulse 300"),
        c("Numark DJ2Go.midi.xml", "Numark DJ2Go", ""),
        c("Numark DJ2GO2 Touch.midi.xml", "Numark DJ2GO2 Touch", ""),
        c("Pioneer-DDJ-400-scripts.midi.xml", "Pioneer DDJ-400", "DDJ-400"),
    ];
    let name = |port: &str| match discover::pick(port, &cands, &[]) {
        discover::Pick::Name(c) => Some(c.file.clone()),
        _ => None,
    };
    assert_eq!(name("Hercules DJControl Inpulse 200").as_deref(), Some("Hercules_DJControl_Inpulse_200.midi.xml"));
    assert_eq!(name("DJControl Inpulse 300 MIDI 1").as_deref(), Some("Hercules_DJControl_Inpulse_300.midi.xml"));
    assert_eq!(name("DJControl Inpulse 200:DJControl Inpulse 200 MIDI 1 20:0").as_deref(), Some("Hercules_DJControl_Inpulse_200.midi.xml"));
    assert_eq!(name("Numark DJ2GO2 Touch").as_deref(), Some("Numark DJ2GO2 Touch.midi.xml"));
    assert_eq!(name("DDJ-400").as_deref(), Some("Pioneer-DDJ-400-scripts.midi.xml"));
    // one shared word is not a match, nor is a brand
    assert_eq!(name("Numark"), None);
    assert_eq!(name("Hercules"), None);
    assert_eq!(name("Inpulse 500"), None);
    assert_eq!(name("Some Synth"), None);
    // an override: by file, by name, to nothing, and to something that is not there
    let ov = |port: &str, m: &str| vec![discover::Override { port: port.into(), mapping: m.into() }];
    let pick = |port: &str, o: &[discover::Override]| discover::pick(port, &cands, o);
    assert!(matches!(pick("My Odd Pad", &ov("odd pad", "Numark DJ2Go.midi.xml")), discover::Pick::Override(c) if c.name == "Numark DJ2Go"));
    assert!(matches!(pick("My Odd Pad", &ov("ODD", "numark dj2go")), discover::Pick::Override(c) if c.name == "Numark DJ2Go"));
    assert!(matches!(pick("Hercules DJControl Inpulse 200", &ov("inpulse", "none")), discover::Pick::Disabled));
    assert!(matches!(pick("My Odd Pad", &ov("odd", "gone.midi.xml")), discover::Pick::OverrideMissing(_)));
    // the override beats the name
    assert!(matches!(pick("Hercules DJControl Inpulse 200", &ov("inpulse", "Numark DJ2Go")), discover::Pick::Override(c) if c.name == "Numark DJ2Go"));
}
