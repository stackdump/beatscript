//! Golden comparison against `tests/fixtures/swift/sessions.json`, written by
//! `tools/paritydump` (the "sessions, rolls, journal lines" block, and `sessionsall.swift`) from
//! the Swift DJCore. A mismatch is a port bug (or a Swift change to take on purpose).
//!
//! `actions` holds 24 ops as Swift writes them; `allActions` all 75, each with the text Swift
//! derives from it (summary, row clips, auto-mix touch, fold key, journal line);
//! `allActionsSession` is one session with every one of them as an event; `mix20` / `mix12`
//! are `SessionMix` of the two synthwave specs.

use std::collections::{BTreeMap, BTreeSet};

use djengine::automix::engine::touch_of_op;
use djengine::cid::share_cid;
use djengine::import::ScoreImport;
use djengine::journal::event::{action_summary, clip_ids, JournalEntry, JournalEvent};
use djengine::json::{JsonObject, JsonValue};
use djengine::roll;
use djengine::session::action::JSON_OPS;
use djengine::session::mix::SessionMix;
use djengine::session::{
    DeckAction, DeckParams, DeckSource, Session, SessionEvent, SessionEventKind,
};
use djengine::spec::TrackSpec;

/// The committed fixture (`tools/paritydump/regen.sh <dj> HEAD sessions`).
fn committed() -> JsonValue {
    let path = format!(
        "{}/tests/fixtures/swift/sessions.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    JsonValue::parse(&text).unwrap()
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("no {k}"))
}

#[test]
fn rolls_match_swift() {
    let f = committed();
    for row in f.get("rolls").unwrap().as_array().unwrap() {
        let input = JsonValue::parse(s(row, "input")).unwrap();
        match roll::envelope(&input) {
            Ok(env) => {
                assert_eq!(
                    env.canonical_js(),
                    s(row, "canonical"),
                    "{}",
                    s(row, "input")
                );
                assert_eq!(share_cid(&env), s(row, "cid"));
            }
            Err(e) => assert_eq!(e.to_string(), s(row, "error"), "{}", s(row, "input")),
        }
    }
}

#[test]
fn actions_decode_and_re_encode_like_swift() {
    let f = committed();
    for a in f.get("actions").unwrap().as_array().unwrap() {
        let text = a.as_str().unwrap();
        let v = JsonValue::parse(text).unwrap();
        let action = DeckAction::from_json(&v).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(action.json().canonical_js(), v.canonical_js(), "{text}");
    }
}

#[test]
fn the_sample_session_has_swifts_canonical_text_and_cid() {
    let f = committed();
    let want = f.get("session").unwrap();
    let decoded = Session::decode(s(want, "canonical"), Some(s(want, "cid"))).unwrap();
    assert_eq!(decoded.canonical(), s(want, "canonical"));
    assert_eq!(decoded.cid(), s(want, "cid"));
}

#[test]
fn journal_lines_decode_and_re_encode_like_swift() {
    let f = committed();
    for l in f.get("journal").unwrap().as_array().unwrap() {
        let line = l.as_str().unwrap();
        let e = JournalEntry::from_line(line).unwrap_or_else(|| panic!("{line}"));
        assert_eq!(e.line(), line);
    }
}

#[test]
fn a_musicxml_import_matches_swift() {
    let f = committed();
    let want = f.get("musicxml").unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="4.0"><work><work-title>Tiny Score</work-title></work>
<part-list><score-part id="P1"><part-name>Lead</part-name></score-part></part-list>
<part id="P1"><measure number="1">
<attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
<direction><sound tempo="90"/></direction>
<note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>
<note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration></note>
<note><pitch><step>E</step><octave>4</octave></pitch><duration>2</duration></note>
</measure></part></score-partwise>"#;
    let r = ScoreImport::convert(xml.as_bytes(), "/x/tiny.musicxml", &Default::default()).unwrap();
    assert_eq!(r.envelope.canonical_js(), s(want, "envelope"));
    assert_eq!(r.summary.canonical_js(), s(want, "summary"));
    assert_eq!(share_cid(&r.envelope), s(want, "cid"));
}

// ---------------------------------------------------------------------------------------------
// All 75 ops, the session of them, and SessionMix

/// The spec of the dump's session (`TrackSpec(genre: "techno", seed: 42, structure: "standard",
/// cohesion: "v2", extra: ["bpm": 126])`).
fn techno_spec() -> TrackSpec {
    let mut s = TrackSpec::new("techno", 42.0, Some("standard"));
    s.cohesion = "v2".into();
    s.extra = JsonObject::from_pairs([("bpm", JsonValue::Number(126.0))]);
    s
}

/// Every op decodes, re-encodes to the very text Swift wrote, and Swift's derived text matches:
/// the History row (`DeckAction.summary`), the row clips it names (`clipIDs`), the auto-mix
/// touch (`autoMixTouch`), the journal's fold key and the journal line the action is written as.
#[test]
fn every_deck_action_matches_swift() {
    let f = committed();
    let all = f.get("allActions").and_then(JsonValue::as_array).unwrap();
    let mut ops = BTreeSet::new();
    for (k, row) in all.iter().enumerate() {
        let text = s(row, "json");
        let a = DeckAction::from_json(&JsonValue::parse(text).unwrap())
            .unwrap_or_else(|| panic!("{text}"));
        assert_eq!(a.json().js_stringified(), text);
        ops.insert(a.op());
        assert_eq!(action_summary(&a), s(row, "summary"), "{text}: summary");
        let ids: Vec<String> = row
            .get("clipIDs")
            .and_then(JsonValue::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(clip_ids(&a), ids, "{text}: clipIDs");
        match row.get("touch").filter(|t| !matches!(t, JsonValue::Null)) {
            Some(t) => {
                let kind = touch_of_op(&a.op()).unwrap_or_else(|| panic!("{text}: touch"));
                assert_eq!(format!("{kind:?}").to_lowercase(), s(t, "kind"), "{text}");
                assert_eq!(
                    a.deck().as_deref(),
                    Some(s(t, "deck")),
                    "{text}: touch deck"
                );
            }
            None => assert_eq!(touch_of_op(&a.op()), None, "{text}: no touch"),
        }
        let k = k as i64;
        let entry = JournalEntry {
            seq: k + 1,
            time: 1_760_000_000_250 + (k + 1) * 1000,
            frame: 1000 + 7 * k,
            event: JournalEvent::Action(a.clone()),
        };
        assert_eq!(
            JournalEvent::Action(a).fold_key(),
            row.get("foldKey")
                .and_then(JsonValue::as_str)
                .map(String::from),
            "{text}: foldKey"
        );
        assert_eq!(entry.line(), s(row, "line"), "{text}: journal line");
    }
    // Swift's 75 first, then the ones beyond Swift, which its dump cannot have
    let swift_ops: BTreeSet<String> = JSON_OPS[..75].iter().map(|o| o.to_string()).collect();
    assert_eq!(swift_ops.len(), 75);
    assert_eq!(&JSON_OPS[75..], ["beatJump"], "the ops beyond Swift");
    assert_eq!(ops, swift_ops, "the dump covers every op Swift has");
}

/// One session with every action as an event: its canonical text and CID are Swift's, built
/// here from the decoded actions (and read back from Swift's text).
#[test]
fn a_session_of_every_action_has_swifts_canonical_text_and_cid() {
    let f = committed();
    let want = f.get("allActionsSession").unwrap();
    let events: Vec<SessionEvent> = f
        .get("allActions")
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let a = DeckAction::from_json(&JsonValue::parse(s(row, "json")).unwrap()).unwrap();
            SessionEvent {
                frame: 100 + 13 * k as i64,
                kind: SessionEventKind::Action(a),
            }
        })
        .collect();
    assert!(events.len() >= 75);
    let decks = BTreeMap::from([
        ("A".to_string(), DeckSource::Net(techno_spec())),
        ("B".to_string(), DeckSource::None),
    ]);
    let params = BTreeMap::from([
        ("A".to_string(), DeckParams::default()),
        ("B".to_string(), DeckParams::default()),
    ]);
    let session = Session::new(44_100.0, 44_100 * 60, decks, params, 0.5, 1.0, events);
    assert_eq!(session.canonical(), s(want, "canonical"));
    assert_eq!(session.cid(), s(want, "cid"));
    // and Swift's own text reads back to the same session
    let back = Session::decode(s(want, "canonical"), Some(s(want, "cid"))).unwrap();
    assert_eq!(back, session);
}

/// `SessionMix` of the two synthwave specs: the session's canonical text, its CID and the title
/// for a 20 s crossfade; the CID for 12 s (a different plan).
#[test]
fn session_mixes_match_swift() {
    let f = committed();
    let specs = vec![
        TrackSpec::new("synthwave", 1984.0, Some("standard")),
        TrackSpec::new("synthwave", 2077.0, Some("standard")),
    ];
    let mix20 = SessionMix::new(&specs, 20.0, 44_100.0).unwrap();
    let want = f.get("mix20").unwrap();
    assert_eq!(mix20.session.canonical(), s(want, "canonical"));
    assert_eq!(mix20.session.cid(), s(want, "cid"));
    assert_eq!(mix20.title(), s(want, "title"));
    let mix12 = SessionMix::new(&specs, 12.0, 44_100.0).unwrap();
    assert_eq!(mix12.session.cid(), s(f.get("mix12").unwrap(), "cid"));
    assert_ne!(mix12.session.cid(), mix20.session.cid());
}
