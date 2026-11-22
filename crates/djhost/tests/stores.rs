//! The stores on disk and the journal file. Ports the file halves of `CompositionStoreTests`,
//! `SetPlannerTests.aSetSurvivesTheDisk`, `FavoritesTests`, `VoiceVariationStoreTests`,
//! `SessionTests` (the store), `SessionMixTests.roundTripsThroughTheStoreWithAStableCID`,
//! `JournalFileTests` and `SessionPublishTests` (the transports), on temporary folders.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use djengine::cid::share_cid;
use djengine::journal::event::JournalEvent;
use djengine::journal::state::{DeckCheckpoint, JournalState};
use djengine::journal::stores::{Favorite, NotePosition, NoteTarget, TrackNote, VoiceVariation};
use djengine::journal::time;
use djengine::json::JsonValue;
use djengine::session::mix::SessionMix;
use djengine::session::publish::{
    publish_session, LocalOnly, PublishError, PublishStep, SessionBundle,
};
use djengine::session::types::{BeatGrid, VoiceTweak};
use djengine::session::{DeckAction, DeckParams, DeckSource, Session, SessionEvent};
use djengine::setplan::{DjSet, SetItem, SetPlanSlot};
use djengine::spec::TrackSpec;
use djhost::journal::{self, JournalFile};
use djhost::publish::{DirectoryTransport, SshTransport};
use djhost::stores::composition::{CompositionStore, StoreError};
use djhost::stores::favorites::{FavoritesStore, NotesStore};
use djhost::stores::sessions::SessionStore;
use djhost::stores::sets::{SetStore, SetStoreError};
use djhost::stores::variations::VoiceVariationStore;
use djhost::stores::{pretty_sorted, temp_path};

/// A temp folder removed on drop.
struct Dir(PathBuf);
impl Dir {
    fn new(p: &str) -> Dir {
        Dir(temp_path(p))
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parse(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap()
}

fn envelope(name: &str, note: i64) -> JsonValue {
    djengine::roll::envelope(&parse(&format!(
        r#"{{"name":"{name}","tempo":110,"parts":{{"melody":{{"notes":[[0,{note}]]}}}}}}"#
    )))
    .unwrap()
}

// CompositionStoreTests.savesListsAndReloadsAnEnvelopeByAPartOfItsId
#[test]
fn composition_saves_lists_and_reloads_an_envelope_by_a_part_of_its_id() {
    let d = Dir::new("compositions");
    let store = CompositionStore::new(&d.0);
    let a = envelope("Daft Punk-style (original)", 60);
    let request = parse(r#"{"inspiredBy":"Daft Punk","brief":"122 BPM"}"#);
    let saved = store
        .save("inspired", "A", &a, Some(&request), 1_760_000_000_000)
        .unwrap();
    let file = saved
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        !saved.existed && file.contains("-daft-punk-style-original-"),
        "{file}"
    );
    assert!(file.ends_with(&format!("{}.json", &saved.cid[saved.cid.len() - 8..])));
    assert!(file.starts_with("20251009T085320Z-"));

    let rows = store.list();
    assert!(rows.len() == 1 && rows[0].kind == "inspired" && rows[0].deck == "A");
    assert!(rows[0].name == "Daft Punk-style (original)" && rows[0].tempo == Some(110.0));
    assert_eq!(rows[0].cid, saved.cid);
    assert_eq!(rows[0].saved, "2025-10-09T08:53:20Z");

    // the envelope comes back byte-for-byte: same share CID
    let (entry, back) = store.envelope("daft").unwrap();
    assert_eq!(share_cid(&back), saved.cid);
    assert_eq!(entry.id, rows[0].id);
    assert_eq!(
        store
            .envelope(&saved.cid[saved.cid.len() - 8..])
            .unwrap()
            .0
            .id,
        rows[0].id
    );
}

// CompositionStoreTests.theSameEnvelopeIsKeptOnceAndDifferentOnesAreTold
#[test]
fn the_same_envelope_is_kept_once_and_different_ones_are_told() {
    let d = Dir::new("compositions");
    let store = CompositionStore::new(&d.0);
    let a = envelope("Same", 60);
    let first = store
        .save("compose", "A", &a, None, 1_760_000_000_000)
        .unwrap();
    let again = store
        .save("compose", "B", &a, None, 1_760_000_001_000)
        .unwrap();
    assert!(again.existed && again.path == first.path && store.list().len() == 1);
    store
        .save(
            "compose",
            "A",
            &envelope("Same", 62),
            None,
            1_760_000_002_000,
        )
        .unwrap();
    assert_eq!(store.list().len(), 2);
    assert!(matches!(
        store.envelope("same"),
        Err(StoreError::Ambiguous(..))
    )); // two match
    assert_eq!(
        store.envelope("zzz").unwrap_err(),
        StoreError::NoMatch("zzz".into())
    );
}

// CompositionStoreTests.theFileKeepsOneTopLevelKeyPerLineAndIsStillJSON
#[test]
fn the_file_keeps_one_top_level_key_per_line_and_is_still_json() {
    let d = Dir::new("compositions");
    let store = CompositionStore::new(&d.0);
    let saved = store
        .save(
            "compose",
            "A",
            &envelope("Readable", 60),
            Some(&parse(r#"{"parts":{}}"#)),
            1_760_000_000_000,
        )
        .unwrap();
    let text = std::fs::read_to_string(&saved.path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0] == "{" && *lines.last().unwrap() == "}");
    assert!(lines.iter().any(|l| l.starts_with("\"request\":")));
    assert!(lines.iter().any(|l| l.starts_with("\"envelope\":")));
    assert_eq!(parse(&text).get("name").unwrap().as_str(), Some("Readable"));
    // the set planner's inputs: energy, key, bars
    let s2 = store
        .save(
            "compose",
            "A",
            &envelope("Planned", 61),
            Some(&parse(r#"{"energy":3,"key":"A minor"}"#)),
            1_760_000_003_000,
        )
        .unwrap();
    let e = store
        .list()
        .into_iter()
        .find(|e| e.path == s2.path)
        .unwrap();
    assert_eq!(
        (e.energy, e.key.as_deref(), e.bars),
        (Some(1.0), Some("A minor"), Some(1))
    );
}

// SetPlannerTests.aSetSurvivesTheDisk
#[test]
fn a_set_survives_the_disk() {
    let d = Dir::new("sets");
    let store = SetStore::new(&d.0);
    let mut set = DjSet::new("Friday Night");
    set.created = "2026-10-21T00:00:00Z".into();
    set.fade_bars = 6;
    let mut slot = SetPlanSlot::new("peak", 0.9);
    slot.bpm = Some(150.0);
    slot.key = Some("A minor".into());
    slot.bars = Some(36);
    set.plan = vec![slot];
    let mut item = SetItem::new("abc");
    item.slot = Some(0);
    item.mix_out_bar = Some(28);
    set.items = vec![item];
    store.save(&set).unwrap();
    assert!(store.exists("friday night"));
    assert_eq!(
        store
            .list()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Friday Night"]
    );
    assert_eq!(store.load("Friday Night").unwrap(), set);
    assert_eq!(
        store.load("nope").unwrap_err(),
        SetStoreError::NoSet("nope".into())
    );
    let text = std::fs::read_to_string(d.0.join("friday-night.json")).unwrap();
    assert!(text.contains("\"fadeBars\" : 6"), "{text}");
}

fn iron() -> TrackSpec {
    TrackSpec::new("techno", 831_071.0, Some("standard"))
}

// FavoritesTests
#[test]
fn favorites_round_trip_newest_first_and_once_per_track() {
    let d = Dir::new("favorites");
    let s = FavoritesStore::new(d.0.join("favorites.json"));
    assert!(s.list().is_empty(), "a missing file is no favorites");
    let net = |t: &str, sp: TrackSpec| Favorite::new(t, DeckSource::Net(sp), 1_700_000_000_000);
    s.add(&net("techno · Iron Fracture", iron())).unwrap();
    s.add(&net(
        "house · Other",
        TrackSpec::new("house", 7.0, None),
    ))
    .unwrap();
    // starring the same track again moves it up, it doesn't add a second
    s.add(&net("techno · Iron Fracture", iron())).unwrap();
    let all = s.list();
    assert_eq!(
        all.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(),
        ["techno · Iron Fracture", "house · Other"]
    );
    assert_eq!(all[0].source, DeckSource::Net(iron()));
    assert!(s.contains(&share_cid(&iron().payload())));
    s.remove(&all[0].cid()).unwrap();
    assert_eq!(
        s.list()
            .iter()
            .map(|f| f.title.as_str())
            .collect::<Vec<_>>(),
        ["house · Other"]
    );
}

#[test]
fn a_sample_with_no_file_is_not_a_favorite_and_one_with_a_file_keeps_its_grid_and_tags() {
    let d = Dir::new("favorites");
    let s = FavoritesStore::new(d.0.join("favorites.json"));
    s.add(&Favorite::new(
        "demo",
        DeckSource::Sample {
            id: "demo-loop".into(),
            title: "demo".into(),
            grid: None,
        },
        0,
    ))
    .unwrap();
    assert!(s.list().is_empty());
    let mut f = Favorite::new(
        "Dubstep Siren Music 01",
        DeckSource::Sample {
            id: String::new(),
            title: "Dubstep Siren Music 01".into(),
            grid: Some(BeatGrid::new(150.0, 0.0, 1.0)),
        },
        1_700_000_000_000,
    );
    f.file = Some("/Library/Audio/Apple Loops/Apple/03 Dubstep/Dubstep Siren Music 01.caf".into());
    f.tags = vec!["siren".into(), "drop".into()];
    s.add(&f).unwrap();
    let back = s.list().remove(0);
    assert!(back.is_sample() && back.cid() == format!("file:{}", f.file.as_ref().unwrap()));
    assert!(back.file == f.file && back.tags == ["siren", "drop"]);
    let DeckSource::Sample { title, grid, .. } = &back.source else {
        panic!("a sample source")
    };
    assert!(title == "Dubstep Siren Music 01" && grid.map(|g| g.bpm) == Some(150.0));
}

#[test]
fn tags_are_cleaned_filter_and_survive_a_re_star() {
    let d = Dir::new("favorites");
    let s = FavoritesStore::new(d.0.join("favorites.json"));
    let net = Favorite::new(
        "techno · One",
        DeckSource::Net(TrackSpec::new("techno", 1.0, None)),
        0,
    );
    let other = Favorite::new(
        "house · Two",
        DeckSource::Net(TrackSpec::new("house", 2.0, None)),
        0,
    );
    s.add(&net).unwrap();
    s.add(&other).unwrap();
    s.set_tags(
        &net.cid(),
        &["  intro ".into(), "".into(), "Intro".into(), "heavy".into()],
    )
    .unwrap();
    s.set_tags(&other.cid(), &["heavy".into()]).unwrap();
    let all = s.list();
    assert_eq!(
        all.iter().find(|f| f.cid() == net.cid()).unwrap().tags,
        ["intro", "heavy"],
        "trimmed, no blanks or repeats"
    );
    assert_eq!(
        djengine::journal::stores::all_tags(&all),
        ["heavy", "intro"],
        "most used first"
    );
    s.add(&Favorite::new("techno · One", net.source.clone(), 0))
        .unwrap(); // starred again
    assert_eq!(
        s.list()[0].tags,
        ["intro", "heavy"],
        "a re-star keeps the tags"
    );
}

// VoiceVariationStoreTests.savesRenamesAndDeletesAnInstrumentsVariations
#[test]
fn saves_renames_and_deletes_an_instruments_variations() {
    let d = Dir::new("voices");
    let store = VoiceVariationStore::new(d.0.join("voices.json"));
    assert!(store.all().is_empty());
    let mut zap_p = VoiceTweak::default();
    zap_p.set("pitchTime", Some(0.08));
    let zap = VoiceVariation::new("short zap", zap_p, false);
    let mut dark_p = VoiceTweak::default();
    dark_p.set("lowpass", Some(600.0));
    let dark = VoiceVariation::new(" dark ", dark_p, true);
    store.save(&zap, "laser").unwrap();
    store.save(&dark, "laser").unwrap();
    let mut soft_p = VoiceTweak::default();
    soft_p.set("gainDB", Some(-6.0));
    store
        .save(&VoiceVariation::new("soft", soft_p, false), "pluck")
        .unwrap();
    let names = |i: &str| {
        store
            .list(i)
            .iter()
            .map(|v| v.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("laser"), ["short zap", "dark"], "in the order saved");
    assert!(
        store.list("laser")[1].use_synth && store.list("laser")[1].params.lowpass == Some(600.0)
    );
    // the same name again replaces it in place
    let mut p2 = VoiceTweak::default();
    p2.set("pitchTime", Some(0.05));
    store
        .save(&VoiceVariation::new("short zap", p2, false), "laser")
        .unwrap();
    assert_eq!(names("laser"), ["short zap", "dark"]);
    assert_eq!(store.list("laser")[0].params.pitch_time, Some(0.05));
    // a fresh store over the same file reads it back
    let again = VoiceVariationStore::new(&store.file);
    assert!(again.all() == store.all() && again.all().len() == 2);
    store.rename("dark", "muffled", "laser").unwrap();
    assert_eq!(names("laser"), ["short zap", "muffled"]);
    // renaming onto a name in use replaces that one
    store.rename("muffled", "short zap", "laser").unwrap();
    assert_eq!(names("laser"), ["short zap"]);
    assert_eq!(store.list("laser")[0].params.lowpass, Some(600.0));
    store.delete("short zap", "laser").unwrap();
    assert!(store.list("laser").is_empty());
    assert_eq!(store.all().keys().cloned().collect::<Vec<_>>(), ["pluck"]);
    assert_eq!(zap.label("laser"), "laser · short zap");
}

fn session_sample() -> Session {
    let mut decks = BTreeMap::new();
    decks.insert("A".to_string(), DeckSource::Net(iron()));
    decks.insert("C".to_string(), DeckSource::None);
    let mut params = BTreeMap::new();
    params.insert("A".to_string(), DeckParams::default());
    params.insert("C".to_string(), DeckParams::default());
    Session::new(
        48_000.0,
        96_000,
        decks,
        params,
        0.25,
        0.9,
        vec![SessionEvent::action(
            128,
            DeckAction::Play {
                deck: "A".into(),
                on: true,
            },
        )],
    )
}

// SessionTests: the store half
#[test]
fn the_session_store_names_files_by_cid_and_catches_tampering() {
    let d = Dir::new("sessions");
    let store = SessionStore::new(&d.0);
    let s = session_sample();
    let (cid, path) = store.save(&s).unwrap();
    assert!(cid == s.cid() && path.file_name().unwrap().to_string_lossy() == format!("{cid}.json"));
    assert_eq!(store.load(&cid).unwrap(), s);
    assert_eq!(store.list(), [cid.clone()]);
    // idempotent: the same performance is the same file
    assert_eq!(store.save(&s).unwrap().1, path);
    assert_eq!(store.list().len(), 1);
    // tampering is caught by the name
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"frames\":96000", "\"frames\":96001")).unwrap();
    assert_eq!(
        store.load(&cid).unwrap_err(),
        djengine::session::SessionError::CidMismatch
    );
}

// SessionMixTests.roundTripsThroughTheStoreWithAStableCID
#[test]
fn a_mix_round_trips_through_the_store_with_a_stable_cid() {
    let d = Dir::new("sessions");
    let store = SessionStore::new(&d.0);
    let specs = vec![
        TrackSpec::new("synthwave", 1984.0, Some("standard")),
        TrackSpec::new("synthwave", 2077.0, Some("standard")),
    ];
    let mix = SessionMix::new(&specs, 20.0, 44_100.0).unwrap();
    let (cid, path) = store.save(&mix.session).unwrap();
    assert!(
        cid == mix.session.cid()
            && path.file_name().unwrap().to_string_lossy() == format!("{cid}.json")
    );
    let back = store.load(&cid).unwrap();
    assert_eq!(back, mix.session);
    assert_eq!(back.cid(), cid);
    assert_eq!(store.list(), [cid.clone()]);
    let again = SessionMix::new(&specs, 20.0, 44_100.0).unwrap();
    assert_eq!(again.session.cid(), cid);
    assert_eq!(store.save(&again.session).unwrap().1, path);
    assert_eq!(store.list().len(), 1);
    assert_ne!(
        SessionMix::new(&specs, 12.0, 44_100.0)
            .unwrap()
            .session
            .cid(),
        cid
    );
}

// ---- notes and the journal file ----------------------------------------------------------------

fn note() -> TrackNote {
    let mut n = TrackNote::new(
        "n-1",
        NoteTarget::Track {
            cid: share_cid(&iron().payload()),
        },
        "drop hits here",
        time::rounded_ms(1_760_000_000.123),
    );
    n.title = "Neon Vale".into();
    n.position = Some(NotePosition::new(17, 1, Some(31.75)));
    n
}

// JournalFileTests.theNotesStoreIsTheJournalsProjection
#[test]
fn the_notes_store_is_the_journals_projection() {
    let d = Dir::new("notes");
    let store = NotesStore::new(d.0.join("notes.json"));
    let n = note();
    store.add(&n).unwrap();
    store.edit(&n.id, "later", time::rounded_ms(9.0)).unwrap();
    assert_eq!(store.list()[0].text, "later");
    let mut projected = djengine::journal::JournalStoreState::default();
    djengine::journal::file::apply_store_event(&mut projected, &JournalEvent::NoteAdd(n.clone()));
    djengine::journal::file::apply_store_event(
        &mut projected,
        &JournalEvent::NoteEdit {
            id: n.id.clone(),
            text: "later".into(),
            at: time::rounded_ms(9.0),
        },
    );
    assert_eq!(store.list(), projected.notes);
    store.delete(&n.id).unwrap();
    assert!(store.list().is_empty());
}

fn checkpoint(stores: djengine::journal::JournalStoreState) -> JournalEvent {
    let mut s = JournalState::new(11_025.0);
    s.decks
        .insert("A".into(), DeckCheckpoint::new(DeckSource::None));
    s.stores = stores;
    JournalEvent::Checkpoint(s)
}

// JournalFileTests.oneFilePerRunNamedByItsStartAndListedNewestFirst
#[test]
fn one_file_per_run_named_by_its_start_and_listed_newest_first() {
    let d = Dir::new("journal");
    let t = |s: f64| time::rounded_ms(s);
    let mut a = JournalFile::create(&d.0, t(1_760_000_000.0), 11_025.0).unwrap();
    let mut b = JournalFile::create(&d.0, t(1_760_000_000.0), 11_025.0).unwrap();
    let mut c = JournalFile::create(&d.0, t(1_760_000_600.0), 11_025.0).unwrap();
    for j in [&mut a, &mut b, &mut c] {
        j.record(0, t(1_760_000_000.0), checkpoint(Default::default()));
        j.close();
    }
    let names: Vec<String> = journal::list(&d.0)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut want = vec![
        "journal-20251009T090320.000Z.jsonl".to_string(),
        "journal-20251009T085320.000Z.jsonl".into(),
        "journal-20251009T085320.000Z-2.jsonl".into(),
    ];
    want.sort_by(|x, y| y.cmp(x));
    assert_eq!(names, want); // Swift literal
    let text = std::fs::read_to_string(&journal::list(&d.0)[0]).unwrap();
    assert!(text.starts_with("{\"f\":0,\"k\":\"checkpoint\",\"seq\":0,"));
    assert!(text.ends_with('\n'));
    // closed: nothing more is recorded
    c.record(5, t(1_760_000_601.0), JournalEvent::Mark("late".into()));
    assert_eq!(
        std::fs::read_to_string(&journal::list(&d.0)[0]).unwrap(),
        text
    );
    assert!(!c.is_open());
}

// JournalFileTests.everyInputPathLandsInTheJournal (the file half) + the crash behaviour
#[test]
fn recorded_events_read_back_and_a_torn_last_line_is_dropped() {
    let d = Dir::new("journal");
    let mut j = JournalFile::create(&d.0, 1_760_000_000_000, 11_025.0).unwrap();
    let events = vec![
        checkpoint(Default::default()),
        JournalEvent::Action(DeckAction::Crossfader(0.2)),
        JournalEvent::Midi {
            port: "APC mini mk2 Control".into(),
            bytes: vec![0x90, 48, 0x7F],
        },
        JournalEvent::Action(DeckAction::Grid {
            deck: "A".into(),
            bpm: 100.0,
            offset: 0.0,
        }),
    ];
    for (i, e) in events.iter().enumerate() {
        j.record(i as i64 * 128, 1_760_000_000_000 + i as i64, e.clone());
    }
    j.flush();
    assert_eq!(j.recorder.last_seq(), 3);
    let c = journal::read(&j.path).unwrap();
    assert_eq!(
        c.entries
            .iter()
            .map(|e| e.event.clone())
            .collect::<Vec<_>>(),
        events
    );
    assert_eq!(c.entries[3].frame, 384);
    // the app dies mid-write: cut the last line short
    let text = std::fs::read_to_string(&j.path).unwrap();
    std::fs::write(&j.path, &text[..text.len() - 9]).unwrap();
    let torn = journal::read(&j.path).unwrap();
    assert!(torn.truncated && torn.entries.len() == 3);
}

// JournalFileTests.theNewestJournalSaysWhatTheStoresHold
#[test]
fn the_newest_journal_says_what_the_stores_hold() {
    let d = Dir::new("journal");
    assert!(journal::latest_stores(&d.0).is_none());
    let mut first = JournalFile::create(&d.0, time::rounded_ms(1_760_000_000.0), 11_025.0).unwrap();
    first.record(
        0,
        0,
        checkpoint(djengine::journal::JournalStoreState {
            crates: vec!["old".into()],
            ..Default::default()
        }),
    );
    first.close();
    let mut j = JournalFile::create(&d.0, time::rounded_ms(1_760_000_100.0), 11_025.0).unwrap();
    j.record(
        0,
        0,
        checkpoint(djengine::journal::JournalStoreState {
            notes: vec![note()],
            ..Default::default()
        }),
    );
    j.record(1, 1, JournalEvent::NoteDelete { id: note().id });
    j.record(2, 2, JournalEvent::CrateAdd { name: "new".into() });
    j.close();
    assert_eq!(
        journal::latest_stores(&d.0).unwrap(),
        djengine::journal::JournalStoreState {
            crates: vec!["new".into()],
            ..Default::default()
        }
    );
}

#[test]
fn pretty_json_is_sorted_and_indented() {
    let v = parse(r#"{"b":[1,2],"a":{"x":true},"c":[]}"#);
    assert_eq!(
        pretty_sorted(&v),
        "{\n  \"a\" : {\n    \"x\" : true\n  },\n  \"b\" : [\n    1,\n    2\n  ],\n  \"c\" : []\n}"
    );
}

// ---- transports -----------------------------------------------------------------------------------------

fn publish_session_fixture() -> Session {
    session_sample()
}

fn bundle(audio: Option<&[u8]>) -> SessionBundle {
    SessionBundle::new(
        &publish_session_fixture(),
        None,
        audio,
        "t",
        false,
        false,
        &LocalOnly::default(),
    )
    .unwrap()
}

// SessionPublishTests.directoryTransportWritesTheTree
#[test]
fn directory_transport_writes_the_tree() {
    let d = Dir::new("cdn");
    let b = bundle(None);
    let r = publish_session(&b, &DirectoryTransport::new(&d.0), false, &mut |_| {}).unwrap();
    assert!(!r.mirrored && r.reloaded);
    assert!(d.0.join(&b.cid).join("index.md").exists());
    assert!(d.0.join(&b.cid).join("session.json").exists());
    // the staged session.json is the canonical session, and nothing half-written is left
    let text = std::fs::read_to_string(d.0.join(&b.cid).join("session.json")).unwrap();
    assert_eq!(
        Session::decode(text.trim(), Some(&b.cid)).unwrap(),
        publish_session_fixture()
    );
    let leftovers: Vec<_> = std::fs::read_dir(d.0.join(&b.cid))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty());
}

type Calls = Arc<Mutex<Vec<(Vec<String>, Option<Vec<u8>>)>>>;

fn recording(status: i32, out: &'static str) -> (SshTransport, Calls) {
    let calls: Calls = Default::default();
    let c = calls.clone();
    let t = SshTransport {
        run: Box::new(move |argv, stdin| {
            c.lock()
                .unwrap()
                .push((argv.to_vec(), stdin.map(|s| s.to_vec())));
            Ok((status, out.to_string()))
        }),
        ..Default::default()
    };
    (t, calls)
}

// SessionPublishTests.sshStagesInOrderAndMirrorsOnlyOnRequest
#[test]
fn ssh_stages_in_order_and_mirrors_only_on_request() {
    let (t, calls) = recording(0, "");
    let b = bundle(Some(&[9]));
    let quiet = publish_session(&b, &t, false, &mut |_| {}).unwrap();
    assert!(!quiet.mirrored && quiet.reloaded);
    let cmds: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.0.last().unwrap().clone())
        .collect();
    assert_eq!(cmds.len(), 4); // 3 files + reload
    assert!(cmds[0].contains("session.json") && cmds[1].contains("audio.wav"));
    assert!(cmds[2].contains("index.md")); // last, so a half-copy is never indexed
    assert!(cmds[3].contains(&format!("/admin/reload?cid={}", b.cid)));
    assert!(!cmds.iter().any(|c| c.contains("mirror-push")));
    assert_eq!(
        calls.lock().unwrap()[0].1.as_deref(),
        Some(&b.files[0].data[..])
    );
    // non-interactive and bounded: an unreachable origin host fails in seconds, never prompts
    let argv = calls.lock().unwrap()[0].0.clone();
    assert_eq!(
        &argv[..5],
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
    );
    assert_eq!(argv[argv.len() - 2], option_env!("DJHOST_PUBLISH_HOST").unwrap_or("cdn-origin.local"));
    assert!(cmds[0].starts_with(&format!(
        "mkdir -p ~/'cdn-stackdump-com/data/blobs'/{}",
        b.cid
    )));

    calls.lock().unwrap().clear();
    let loud = publish_session(&b, &t, true, &mut |_| {}).unwrap();
    assert!(loud.mirrored);
    assert!(calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .0
        .last()
        .unwrap()
        .contains("mirror-push.sh"));
}

// SessionPublishTests.failedStageStopsBeforeReloadOrMirror
#[test]
fn a_failed_stage_stops_before_reload_or_mirror() {
    let (t, calls) = recording(255, "ssh: connect refused");
    let b = bundle(None);
    let e = publish_session(&b, &t, true, &mut |_| {}).unwrap_err();
    assert!(matches!(e, PublishError::CommandFailed(_, 255, _)));
    assert_eq!(calls.lock().unwrap().len(), 1);
}

// SessionPublishTests.progressNarratesEachStepInOrder
#[test]
fn progress_narrates_each_step_in_order() {
    let (t, _) = recording(0, "");
    let b = bundle(Some(&[9]));
    let mut steps = Vec::new();
    publish_session(&b, &t, true, &mut |s| steps.push(s)).unwrap();
    assert_eq!(
        steps,
        [
            PublishStep::Staging {
                file: "session.json".into(),
                index: 1,
                of: 3
            },
            PublishStep::Staging {
                file: "audio.wav".into(),
                index: 2,
                of: 3
            },
            PublishStep::Staging {
                file: "index.md".into(),
                index: 3,
                of: 3
            },
            PublishStep::Reloading,
            PublishStep::Mirroring,
            PublishStep::Done,
        ]
    );
}

#[test]
fn remote_paths_keep_the_tilde_outside_the_quotes() {
    use djhost::publish::{remote_path, shell_quote};
    assert_eq!(remote_path("~/a b/c"), "~/'a b/c'");
    assert_eq!(remote_path("/srv/it's"), "'/srv/it'\\''s'");
    assert_eq!(shell_quote("x"), "'x'");
}

// SessionPublishTests.stdinWriteToAnExitedProcessFailsInsteadOfCrashing
#[cfg(unix)]
#[test]
fn stdin_write_to_an_exited_process_fails_instead_of_crashing() {
    // `false` exits without reading; a large stdin used to raise SIGPIPE in the app.
    let r = djhost::publish::process(&["false".into()], Some(&vec![7u8; 4_000_000])).unwrap();
    assert_eq!(r.0, 1);
    let echo = djhost::publish::process(&["cat".into()], Some(b"hello")).unwrap();
    assert!(echo.0 == 0 && echo.1 == "hello");
}
