//! Recorded sets (docs/RECORDING.md): the tracklist an arrangement implies, the object's CID,
//! its replay and its forks.

use djengine::json::JsonValue;
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeEvent, ArrangeOp, Arrangement};
use djengine::session::model::DeckSource;
use djengine::setrec::{self, AudioRef, Entry, EntryKind, RecordedSet};
use djengine::TrackSpec;

fn ev(at: f64, op: ArrangeOp) -> ArrangeEvent {
    ArrangeEvent { at, bpm: 120.0, op, via: "test".into(), text: String::new() }
}

fn net(genre: &str, seed: f64) -> DeckSource {
    DeckSource::Net(TrackSpec::new(genre, seed, Some("standard")))
}

fn a_set() -> Arrangement {
    let mut decks = std::collections::BTreeMap::new();
    decks.insert("A".to_string(), net("techno", 7.0));
    decks.insert("B".to_string(), DeckSource::None);
    let mut m = djengine::mashup::Mashup::new("M");
    m.add_source(djengine::mashup::Source::parse("A", "techno:7").unwrap()).unwrap();
    m.assign("drums", "A", "drums").unwrap();
    let mash = m.realize().unwrap();
    let events = vec![
        ev(0.0, ArrangeOp::Deck(DeckAction::Crossfader(0.0))),
        ev(32.0, ArrangeOp::Deck(DeckAction::Load { deck: "B".into(), source: net("house", 42.0) })),
        ev(64.0, ArrangeOp::Deck(DeckAction::Crossfader(0.5))),
        ev(68.0, ArrangeOp::Deck(DeckAction::Crossfader(1.0))),
        // a reload of what B holds is not a new line
        ev(70.0, ArrangeOp::Deck(DeckAction::Load { deck: "B".into(), source: net("house", 42.0) })),
        ev(96.0, ArrangeOp::Deck(DeckAction::Load { deck: "A".into(), source: DeckSource::RawNets(mash) })),
        ev(128.0, ArrangeOp::Deck(DeckAction::Crossfader(0.0))),
    ];
    Arrangement::new(120.0, decks, events)
}

#[test]
fn the_tracklist_names_every_track_mashup_and_crossfade_with_its_cid() {
    let list = setrec::tracklist(&a_set());
    let kinds: Vec<(&str, &str, f64)> = list.iter().map(|e| (e.kind.name(), e.deck.as_str(), e.t)).collect();
    assert_eq!(
        kinds,
        [("track", "A", 0.0), ("track", "B", 16.0), ("transition", "", 34.0), ("mashup", "A", 48.0), ("transition", "", 64.0)]
    );
    assert_eq!(list[0].label, "techno:7");
    let spec = TrackSpec::new("techno", 7.0, Some("standard"));
    assert_eq!(list[0].cid, djengine::cid::share_cid(&spec.payload()), "a track is named by its share CID");
    assert_eq!(list[2].label, "crossfade A → B");
    assert_eq!(list[4].label, "crossfade B → A");
    assert!(list[3].cid.starts_with('z'));
}

#[test]
fn host_marks_merge_in_by_time_without_doubling() {
    let derived = setrec::tracklist(&a_set());
    let mark = Entry { t: 20.0, beat: 40.0, kind: EntryKind::Transition, deck: "".into(), cid: "".into(), label: "bass_swap A".into() };
    let dup = derived[1].clone();
    let merged = setrec::merge(derived.clone(), &[mark.clone(), dup]);
    assert_eq!(merged.len(), derived.len() + 1);
    assert_eq!(merged[2], mark);
    // a loop-set pad's load: the mark relabels the track line it made
    let pad = Entry { t: 16.5, beat: 33.0, kind: EntryKind::LoopSet, deck: "B".into(), cid: derived[1].cid.clone(), label: "house 3A".into() };
    let merged = setrec::merge(derived.clone(), &[pad]);
    assert_eq!(merged.len(), derived.len());
    assert_eq!((merged[1].kind, merged[1].label.as_str()), (EntryKind::LoopSet, "house 3A"));
    // onto the audio's clock
    let moved = setrec::on_audio_clock(derived.clone(), 1.5);
    assert_eq!((moved[0].t, moved[1].t), (0.0, 14.5));
}

fn recorded() -> RecordedSet {
    let arr = a_set();
    RecordedSet {
        name: "Tuesday".into(),
        started: "2026-11-17T21:00:00Z".into(),
        bpm: 120.0,
        duration: 70.0,
        audio: Some(AudioRef {
            cid: setrec::raw_cid(b"RIFF....WAVE"),
            file: "set.wav".into(),
            frames: 70 * 48_000,
            sample_rate: 48_000.0,
            channels: 2,
            format: "wav-f32".into(),
        }),
        tracklist: setrec::tracklist(&arr),
        arrangement: arr.json(),
        bars: (1, 33),
        audio_offset: 0.0,
        forked_from: None,
    }
}

#[test]
fn a_recorded_set_is_canonical_and_content_addressed() {
    let r = recorded();
    let back = RecordedSet::from_json(&JsonValue::parse(&r.canonical()).unwrap()).unwrap();
    assert_eq!(back.canonical(), r.canonical(), "a round trip changes nothing");
    assert_eq!(back.cid(), r.cid());
    let mut other = r.clone();
    other.name = "Wednesday".into();
    assert_ne!(other.cid(), r.cid());
    // parents: every track and mashup played, once
    let p = r.parents();
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(r.to_json().get("parents").is_some());
    assert!(r.cue_sheet().contains("crossfade A → B"));
}

#[test]
fn it_replays_exactly_the_arrangement_it_recorded() {
    let r = recorded();
    let a = r.replay().expect("an arrangement");
    assert_eq!(a.json().canonical_js(), a_set().json().canonical_js());
    // and the tracklist can be re-derived from the replay
    assert_eq!(setrec::tracklist(&a), r.tracklist);
}

#[test]
fn a_fork_points_back_and_drops_the_take() {
    let r = recorded();
    let f = r.fork(Some("Tuesday (rework)"));
    assert_eq!(f.forked_from.as_deref(), Some(r.cid().as_str()));
    assert!(f.audio.is_none());
    assert_eq!(f.arrangement, r.arrangement);
    assert_ne!(f.cid(), r.cid());
}

#[test]
fn raw_cids_are_cidv1_raw_sha256() {
    let c = setrec::raw_cid(b"hello");
    assert!(c.starts_with("zb2rh"), "{c}: the raw-codec CIDv1 prefix in base58btc");
    assert_eq!(c, setrec::raw_cid_of_digest(&djengine::cid::sha256(b"hello")));
    assert_eq!(setrec::clock(3725.4), "1:02:05");
    assert_eq!(setrec::clock(65.0), "1:05");
}
