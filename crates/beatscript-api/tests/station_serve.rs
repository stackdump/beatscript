//! Radio (docs/beatscript/radio.md), as the server serves it: the check report of a station program (its `station` field, what
//! `/api/check` and the MCP `check` pass on), and what the station answer (`beatscript_api::station_json`: the MCP `station` tool,
//! `POST /api/station`, the page's `beatscript_station`) says of windows of tracks, a counted walk's chapters, locks and neighbours.
//! Determinism, retune, the forms' and segues' checks and bad requests are tests/station.rs's.
//!
//! `cargo test -p beatscript-api --test station_serve`.

use djengine::json::JsonValue;

/// A station whose walk draws each seed from the CID of the track before, two forms in turn.
const HASHED: &str = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia\n}\n";
/// A station whose walk counts on by 3.
const COUNTED: &str = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=+3\n  forms seeded passacaglia\n}\n";

fn quote(s: &str) -> String {
    JsonValue::str(s.to_string()).js_stringified()
}

/// The answer to `{"station": <program>, <extra>}`.
fn ask(program: &str, extra: &str) -> JsonValue {
    let req = format!("{{\"station\": {}{}{extra}}}", quote(program), if extra.is_empty() { "" } else { ", " });
    JsonValue::parse(&beatscript_api::station_json(&req)).unwrap_or_else(|e| panic!("station_json answers JSON: {e:?}"))
}

fn at<'a>(v: &'a JsonValue, path: &[&str]) -> &'a JsonValue {
    let mut v = v;
    for k in path {
        v = match k.parse::<usize>() {
            Ok(i) => v.at(i),
            Err(_) => v.get(k),
        }
        .unwrap_or_else(|| panic!("no {path:?} in {}", v.js_stringified()));
    }
    v
}

fn s<'a>(v: &'a JsonValue, path: &[&str]) -> &'a str {
    at(v, path).as_str().unwrap_or_else(|| panic!("{path:?} is no string"))
}

fn n(v: &JsonValue, path: &[&str]) -> i64 {
    at(v, path).as_i64().unwrap_or_else(|| panic!("{path:?} is no number"))
}

fn is_cid(x: &str) -> bool {
    x.starts_with('z') && x.len() > 20
}

#[test]
fn a_window_of_tracks_is_the_same_tracks_and_each_names_its_parent() {
    let one = ask(HASHED, "\"n\": 3");
    assert_eq!(at(&one, &["ok"]).as_bool(), Some(true), "{}", one.js_stringified());
    assert_eq!(n(&one, &["n"]), 3);
    assert_eq!(n(&one, &["tracks", "0", "n"]), 3);
    assert!(is_cid(s(&one, &["tracks", "0", "cid"])) && is_cid(s(&one, &["station", "cid"])) && is_cid(s(&one, &["station", "start_cid"])));
    assert_eq!(s(&one, &["tracks", "0", "form"]), "passacaglia", "track 3 plays forms[3 mod 2]");
    // track 3 asked as the third of a window from 1: the same track, whose parent is the track before it
    let win = ask(HASHED, "\"n\": 1, \"count\": 3");
    let ts = at(&win, &["tracks"]).as_array().unwrap();
    assert_eq!(ts.len(), 3);
    assert_eq!(s(&win, &["tracks", "2", "cid"]), s(&one, &["tracks", "0", "cid"]));
    assert_eq!(s(&win, &["tracks", "2", "text"]), s(&one, &["tracks", "0", "text"]));
    assert_eq!(s(&one, &["tracks", "0", "parent"]), s(&win, &["tracks", "1", "cid"]));
    // the segue after each track, 3 into 4: a program of its own
    let sg = at(&one, &["segues", "0"]);
    assert_eq!((n(sg, &["from"]), n(sg, &["to"])), (3, 4));
    assert!(is_cid(s(sg, &["cid"])) && s(sg, &["cid"]) != s(&one, &["tracks", "0", "cid"]));
    assert!(s(sg, &["text"]).contains("template(cross, "), "{}", s(sg, &["text"]));
    // segue false: none
    assert_eq!(at(&ask(HASHED, "\"n\": 3, \"segue\": false"), &["segues"]).as_array().map(Vec::len), Some(0));
}

#[test]
fn a_counted_walk_steps_the_seed_and_a_chapter_keeps_its_theme() {
    // s_N = 42 + 3N; forms seeded passacaglia: track 5 is passacaglia, its chapter starts at 4 (theme s_4 = 54)
    let v = ask(COUNTED, "\"n\": 5");
    assert_eq!(n(&v, &["tracks", "0", "seed"]), 57);
    assert_eq!(n(&v, &["tracks", "0", "theme"]), 54);
    assert_eq!(s(&v, &["station", "step"]), "+3");
    let t = s(&v, &["tracks", "0", "text"]);
    assert!(t.contains("ground=gen@2 techno seed=54") && t.contains("upper=gen@2 techno seed=57"), "{t}");
    assert!(t.contains("tempo 128") && t.contains("key am"), "{t}");
}

#[test]
fn a_lock_holds_and_neighbours_turn_only_what_is_unlocked() {
    let base = ask(HASHED, "\"n\": 3");
    // a lock: turning it is refused
    let locked = ask(HASHED, "\"retune\": {\"tempo\": \"+1\"}, \"locks\": [\"tempo\"]");
    assert_eq!(at(&locked, &["ok"]).as_bool(), Some(false));
    assert!(s(&locked, &["error"]).contains("locked"), "{}", locked.js_stringified());
    // neighbours: one notch along each unlocked knob, never a locked one and never the genre, each at n
    let ns = ask(HASHED, "\"n\": 3, \"locks\": [\"tempo\", \"key\"], \"neighbours\": true");
    let list = at(&ns, &["neighbours"]).as_array().unwrap();
    assert!(!list.is_empty());
    for x in list {
        let knob = s(x, &["knob"]);
        assert!(!["tempo", "key", "genre"].contains(&knob), "a neighbour turns {knob}");
        assert_eq!(n(x, &["track", "n"]), 3);
        assert!(is_cid(s(x, &["cid"])) && s(x, &["cid"]) != s(&base, &["station", "cid"]));
    }
    for knob in ["seed", "step", "forms", "length"] {
        assert!(list.iter().any(|x| s(x, &["knob"]) == knob), "no neighbour along {knob}");
    }
    assert_eq!(s(&ns, &["locks", "1"]), "key");
}

#[test]
fn check_of_a_station_program_reports_the_station() {
    let rep = beatscript_api::check_report(HASHED);
    assert_eq!(at(&rep, &["ok"]).as_bool(), Some(true), "{}", rep.js_stringified());
    let st = at(&rep, &["station"]);
    assert_eq!(s(st, &["cid"]), s(&rep, &["cid"]), "a station's CID is its program's");
    assert!(is_cid(s(st, &["start_cid"])));
    assert_eq!(at(st, &["tracks"]).as_array().map(Vec::len), Some(2), "the first pass of the forms");
    assert_eq!((n(st, &["segue", "from"]), n(st, &["segue", "to"])), (0, 1));
    // and the station tool agrees on where the walk starts and what the station is
    let v = ask(HASHED, "");
    assert_eq!(s(&v, &["station", "cid"]), s(st, &["cid"]));
    assert_eq!(s(&v, &["station", "start_cid"]), s(st, &["start_cid"]));
    assert_eq!(s(&v, &["tracks", "0", "cid"]), s(st, &["tracks", "0", "cid"]));
    // a program with no station says nothing of one
    assert!(beatscript_api::check_report("tempo 120\ndeck A: kick x ~ x ~\n").get("station").is_none());
}
