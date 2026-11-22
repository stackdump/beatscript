//! The host half of Mixxx mapping support, on a mapping this test writes to a temp folder
//! (never one of Mixxx's: they are GPL and stay out of the repo, docs/MIXXX-MAPPINGS.md).

use djhost::controllers::DryHost;
use djhost::mixxx::{parse_override, Library, Mixxx, Via};
use djengine::control::controller::Action;
use djengine::control::router::{ControllerRouter, RouteResult};

const MAP: &str = r#"<?xml version="1.0"?>
<MixxxMIDIPreset schemaVersion="1">
  <info><name>Acme Pad Two</name><author>test</author></info>
  <controller id="Acme Pad Two"><controls>
    <control><group>[Channel1]</group><key>play</key><status>0x90</status><midino>0x01</midino></control>
    <control><group>[Channel2]</group><key>volume</key><status>0xB1</status><midino>0x07</midino><options><invert/></options></control>
    <control><group>[Channel1]</group><key>jog</key><status>0xB0</status><midino>0x30</midino><options><script-binding/></options></control>
  </controls></controller>
</MixxxMIDIPreset>"#;

const OTHER: &str = r#"<MixxxMIDIPreset><info><name>Odd Box Five</name></info><controller id="Odd Box Five"><controls>
  <control><group>[Master]</group><key>crossfader</key><status>0xB0</status><midino>0x0A</midino></control>
</controls></controller></MixxxMIDIPreset>"#;

fn folder(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("djhost-mixxx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("Acme Pad Two.midi.xml"), MAP).unwrap();
    std::fs::write(d.join("Odd-Box-Five.midi.xml"), OTHER).unwrap();
    std::fs::write(d.join("broken.midi.xml"), "<nope/>").unwrap();
    std::fs::write(d.join("readme.txt"), "not a mapping").unwrap();
    d
}

fn mixxx_in(d: &std::path::Path) -> Mixxx {
    let mut m = Mixxx::default();
    m.lib = Library::scan(vec![d.to_path_buf()]);
    m
}

fn router() -> ControllerRouter {
    ControllerRouter::with_defaults(&["A".to_string(), "B".to_string()])
}

#[test]
fn the_scan_indexes_mappings_and_says_which_files_it_could_not_read() {
    let d = folder("scan");
    let lib = Library::scan(vec![d.clone()]);
    let names: Vec<&str> = lib.cands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Acme Pad Two", "Odd Box Five"]);
    assert_eq!(lib.skipped.len(), 1);
    assert_eq!(lib.skipped[0].0, "broken.midi.xml");
    // a missing folder is an empty index, not an error
    assert!(Library::scan(vec!["/no/such/folder".into()]).cands.is_empty());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_plugged_in_controller_gets_its_mapping_and_drives_the_decks() {
    let d = folder("plug");
    let mut m = mixxx_in(&d);
    let mut r = router();
    assert_eq!(m.consider("Acme Pad Two MIDI 1", &mut r), Some(Ok("Mixxx: Acme Pad Two".to_string())));
    let l = m.loaded_on("Acme Pad Two MIDI 1").unwrap();
    assert_eq!((l.via, l.file.as_str()), (Via::Name, "Acme Pad Two.midi.xml"));
    assert_eq!(r.profile_name("Acme Pad Two MIDI 1").as_deref(), Some("Mixxx: Acme Pad Two"));
    assert!(r.surface("Acme Pad Two MIDI 1").is_some(), "attached");

    // a press of the mapped play button is deck A's play
    let mut host = DryHost::new();
    let res = r.route("Acme Pad Two MIDI 1", &[0x90, 0x01, 127], 0.0, &mut host);
    assert!(matches!(res, RouteResult::Surface(..)), "{res:?}");
    assert!(host.take_actions().iter().any(|a| matches!(a, Action::PlayToggle { deck } if deck == "A")));

    // the invert is the host's to apply, in front of the router
    assert_eq!(m.filter("Acme Pad Two MIDI 1", &[0xB1, 0x07, 100]), vec![0xB1, 0x07, 27]);
    assert_eq!(m.filter("Some Other Port", &[0xB1, 0x07, 100]), vec![0xB1, 0x07, 100]);

    // asked again, nothing new
    assert_eq!(m.consider("Acme Pad Two MIDI 1", &mut r), None);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn hand_made_profiles_and_added_generic_devices_keep_their_ports() {
    let d = folder("prio");
    // a mapping that would match the APC by name
    std::fs::write(
        d.join("Akai APC mini mk2.midi.xml"),
        MAP.replace("Acme Pad Two", "APC mini mk2"),
    )
    .unwrap();
    let mut m = mixxx_in(&d);
    let mut r = router();
    // the shipped APC profile owns "APC mini mk2 Control": no Mixxx device
    assert_eq!(m.consider("APC mini mk2 Control", &mut r), None);
    assert!(m.loaded_on("APC mini mk2 Control").is_none());
    assert_eq!(r.profile_name("APC mini mk2 Control").as_deref(), Some("APC mini mk2"));
    // so does a generic device a person added by hand
    r.add_profile(std::sync::Arc::new(djengine::control::profiles::generic("Acme Pad Two MIDI 1")));
    assert_eq!(m.consider("Acme Pad Two MIDI 1", &mut r), None);
    assert!(r.profile_name("Acme Pad Two MIDI 1").unwrap().starts_with("Generic MIDI"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_config_override_beats_the_name_and_none_keeps_a_port_bare() {
    let d = folder("ovr");
    let mut m = mixxx_in(&d);
    m.overrides = vec![
        parse_override("weird synth=Odd Box Five").unwrap(),
        parse_override("acme=none").unwrap(),
        parse_override("ghost=gone.midi.xml").unwrap(),
    ];
    assert!(parse_override("no equals").is_none() && parse_override("=x").is_none());
    let mut r = router();
    assert_eq!(m.consider("Weird Synth USB", &mut r), Some(Ok("Mixxx: Odd Box Five".to_string())));
    assert_eq!(m.loaded_on("Weird Synth USB").unwrap().via, Via::Override);
    assert_eq!(m.consider("Acme Pad Two MIDI 1", &mut r), None, "none: no mapping, though the name matches");
    assert!(m.tried()["Acme Pad Two MIDI 1"].contains("override"));
    assert!(matches!(m.consider("Ghost Port", &mut r), Some(Err(e)) if e.contains("gone.midi.xml")));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn load_and_unload_round_trip_and_a_second_unit_is_a_device_of_its_own() {
    let d = folder("rt");
    let mut m = mixxx_in(&d);
    let mut r = router();
    // by hand: any mapping onto any port nobody has a profile for
    assert_eq!(m.load("Mystery Box", "odd box", &mut r), Ok("Mixxx: Odd Box Five".to_string()));
    assert_eq!(r.profile_name("Mystery Box").as_deref(), Some("Mixxx: Odd Box Five"));
    assert!(m.load("Mystery Box", "no such thing", &mut r).is_err());
    // a hand-made profile's port is refused
    assert!(m.load("APC mini mk2 Control", "odd box", &mut r).unwrap_err().contains("hand-made"));
    // the same mapping on a second port: a second device
    assert_eq!(m.load("Another Port", "Odd-Box-Five.midi.xml", &mut r), Ok("Mixxx: Odd Box Five (2)".to_string()));
    // unload by device name, then by port
    let gone = m.unload("Mixxx: Odd Box Five (2)", &mut r).unwrap();
    assert_eq!(gone.port, "Another Port");
    assert!(r.profile_name("Another Port").is_none() && r.surface("Another Port").is_none());
    m.unload("Mystery Box", &mut r).unwrap();
    assert!(r.profile_name("Mystery Box").is_none());
    assert!(m.unload("Mystery Box", &mut r).is_err());
    assert_eq!(m.loaded().count(), 0);
    // and it can be loaded again
    assert!(m.load("Mystery Box", "odd box", &mut r).is_ok());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn off_means_off_and_no_mappings_means_nothing_to_do() {
    let d = folder("off");
    let mut m = mixxx_in(&d);
    m.enabled = false;
    let mut r = router();
    assert_eq!(m.consider("Acme Pad Two MIDI 1", &mut r), None);
    assert!(r.profile_name("Acme Pad Two MIDI 1").is_none());
    let mut none = Mixxx::default();
    assert_eq!(none.consider("Acme Pad Two MIDI 1", &mut r), None);
    assert!(none.tried()["Acme Pad Two MIDI 1"].contains("no Mixxx mappings"));
    let _ = std::fs::remove_dir_all(d);
}
