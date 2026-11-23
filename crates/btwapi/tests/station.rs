//! Radio: stations (docs/btw/radio.md), with the seeded generator installed:
//! - determinism: the same station and position give the same track CID (and expanded CID) twice, through the library and
//!   through the JSON answer;
//! - retuning a knob keeps position N, and tempo, key and forms keep the walk;
//! - every track of a chapter and every segue checks clean, in every form, in a dense genre and a sparse one;
//! - the segue is a cross with its own CID, unlike either track's;
//! - a set steps a generator call with `next`;
//! - the example station checks.
//!
//! The listen numbers (locality: a neighbour's tracks are measurably close) are crates/btwd/src/radio_listen.rs.

use djengine::json::JsonValue;
use djlive::diag::Severity;

const RADIO: &str = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia rondo fugue\n}\n";

fn env() -> djlive::env::Env<'static> {
    btwapi::web_env().env()
}

fn check(src: &str) -> djlive::compile::Check {
    btwapi::generate::install();
    djlive::compile::check_with(src, &env())
}

/// The warnings and errors of a check (not its notes).
fn complaints(c: &djlive::compile::Check) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.severity != Severity::Info).map(|d| format!("{:?}: {}", d.severity, d.message)).collect()
}

fn ask(req: &str) -> JsonValue {
    let v = JsonValue::parse(&btwapi::station_json(req)).expect("the answer is JSON");
    assert_eq!(v.get("ok").and_then(JsonValue::as_bool), Some(true), "{req}\n{}", v.js_stringified());
    v
}

fn req(station: &str, more: &str) -> String {
    format!("{{\"station\": {}{more}}}", JsonValue::str(station).js_stringified())
}

fn s<'a>(v: &'a JsonValue, path: &[&str]) -> &'a str {
    let mut at = v;
    for p in path {
        at = match p.parse::<usize>() {
            Ok(i) => &at.as_array().expect("an array")[i],
            Err(_) => at.get(p).unwrap_or_else(|| panic!("no {p} in {}", at.js_stringified())),
        };
    }
    at.as_str().unwrap_or_else(|| panic!("{path:?} is no string"))
}

#[test]
fn a_station_program_checks_and_compiles_to_its_track_0() {
    let c = check(RADIO);
    assert!(complaints(&c).is_empty(), "{:#?}", complaints(&c));
    let comp = c.compiled.expect("it compiles");
    assert_eq!(comp.text, RADIO, "the canonical text is the station's");
    let report = comp.station.as_ref().expect("a station report");
    assert_eq!(report.tracks.len(), 4, "the first chapter, one track of each form");
    assert!(report.segue.is_some());
    assert_eq!(comp.tempo, Some(128.0));
    // it plays its track 0: the generator's techno seed 42 at 128 in A minor
    let t0 = djlive::station::read(RADIO).unwrap().track(0).unwrap();
    let alone = check(&t0.text).compiled.expect("track 0 compiles");
    assert_eq!(comp.song_bars(), alone.song_bars());
    let note = c.diagnostics.iter().find(|d| d.severity == Severity::Info && d.message.starts_with("station techno")).expect("the station's note");
    assert!(note.message.contains("forms seeded passacaglia rondo fugue"), "{}", note.message);
    // the report in the check answer
    let rep = btwapi::check_report(RADIO);
    assert_eq!(rep.get("ok").and_then(JsonValue::as_bool), Some(true));
    // a station that does not say its tempo and key has the from call's own
    let plain = check("station {\n  from gen@2 house seed=7\n}\n");
    assert!(complaints(&plain).is_empty(), "{:#?}", complaints(&plain));
    let st = plain.compiled.unwrap().station.unwrap().station;
    assert!(st.tempo.is_none() && st.key.is_none(), "the station says neither: {st:?}");
    assert!(st.tempo_heard().is_some() && st.key_heard().is_some(), "its tracks play at the from call's own: {st:?}");
    // the station's CID is of its text as written, in the check and in the answer
    let plain_src = "station {\n  from gen@2 house seed=7\n}\n";
    assert_eq!(st.cid(), djlive::station::read(plain_src).unwrap().cid());
    let a = ask(&req(plain_src, ", \"n\": 2"));
    assert_eq!(s(&a, &["station", "cid"]), st.cid());
    assert!(s(&a, &["tracks", "0", "text"]).starts_with(&format!("#! station: {} track 2\n", st.cid())));
    // a notch of the tempo it does not say turns the from call's own, and writes it
    let up = ask(&req(plain_src, ", \"retune\": {\"tempo\": \"+1\"}"));
    let t = up.get("station").unwrap().get("tempo").and_then(JsonValue::as_f64).unwrap();
    assert_eq!(t, st.tempo_heard().unwrap() + 2.0);
    assert!(s(&up, &["station", "text"]).starts_with(&format!("tempo {}", t as i64)));
}

#[test]
fn every_form_of_a_chapter_and_every_segue_checks_clean_in_a_dense_and_a_sparse_genre() {
    for station in [
        "tempo 126\nkey dm\nstation {\n  from gen@2 house seed=7\n  forms seeded passacaglia canon rondo fugue theme-and-variations cross\n}\n",
        "station {\n  from gen@2 ambient seed=11\n  next seed=~\n  forms seeded passacaglia canon rondo fugue theme-and-variations cross\n}\n",
        "station {\n  from gen@2 trap seed=3\n  next seed=+5\n  forms cross fugue\n}\n",
    ] {
        let c = check(station);
        assert!(complaints(&c).is_empty(), "{station}\n{:#?}", complaints(&c));
        let st = djlive::station::read(station).unwrap().with_defaults().unwrap();
        for t in st.tracks(0, st.forms.len() as u64 + 1).unwrap() {
            let ct = check(&t.text);
            assert!(complaints(&ct).is_empty(), "track {} {}\n{}\n{:#?}", t.n, t.form, t.text, complaints(&ct));
        }
        for n in [0u64, 1, 5] {
            let sg = st.segue(n).unwrap();
            let cs = check(&sg.text);
            assert!(complaints(&cs).is_empty(), "segue {n}\n{}\n{:#?}", sg.text, complaints(&cs));
        }
    }
}

#[test]
fn position_n_is_the_same_track_twice_through_the_json_answer() {
    let a = ask(&req(RADIO, ", \"n\": 9, \"count\": 2, \"check\": true"));
    let b = ask(&req(RADIO, ", \"n\": 9, \"count\": 2, \"check\": true"));
    assert_eq!(a.js_stringified(), b.js_stringified(), "the same answer twice");
    assert_eq!(s(&a, &["station", "cid"]), djlive::station::read(RADIO).unwrap().cid());
    let t = djlive::station::read(RADIO).unwrap().track(9).unwrap();
    assert_eq!(s(&a, &["tracks", "0", "cid"]), t.cid);
    assert_eq!(a.get("tracks").unwrap().as_array().unwrap()[0].get("n").and_then(JsonValue::as_f64), Some(9.0));
    // the expanded CID (the written-out program) is the same twice too, and is not the recipe's
    let e = s(&a, &["tracks", "0", "check", "expanded_cid"]);
    assert_eq!(e, s(&b, &["tracks", "0", "check", "expanded_cid"]));
    assert_ne!(e, t.cid);
    for i in 0..2 {
        let ok = a.get("tracks").unwrap().as_array().unwrap()[i].get("check").unwrap().get("ok").and_then(JsonValue::as_bool);
        assert_eq!(ok, Some(true), "track {}", 9 + i);
    }
    // the same request with whitespace and comments in the station: the same station, the same tracks
    let spaced = format!("# radio\n{}", RADIO.replace("from gen@2", "from   gen@2"));
    let c = ask(&req(&spaced, ", \"n\": 9, \"count\": 2"));
    assert_eq!(s(&c, &["tracks", "0", "cid"]), t.cid);
    assert_eq!(s(&c, &["station", "cid"]), s(&a, &["station", "cid"]));
}

#[test]
fn retuning_keeps_position_n() {
    let n = 6;
    let base = ask(&req(RADIO, &format!(", \"n\": {n}")));
    let base_seed = base.get("tracks").unwrap().as_array().unwrap()[0].get("seed").and_then(JsonValue::as_f64).unwrap();
    for (knob, v, walk_kept) in [("tempo", "\"+1\"", true), ("key", "\"-1\"", true), ("forms", "\"+1\"", true), ("seed", "\"+1\"", false), ("step", "\"+1\"", false), ("tempo", "140", true)] {
        let r = ask(&req(RADIO, &format!(", \"n\": {n}, \"retune\": {{\"{knob}\": {v}}}")));
        assert_eq!(r.get("n").and_then(JsonValue::as_f64), Some(n as f64), "{knob}");
        let t = &r.get("tracks").unwrap().as_array().unwrap()[0];
        assert_eq!(t.get("n").and_then(JsonValue::as_f64), Some(n as f64), "{knob} keeps the position");
        assert_ne!(s(&r, &["station", "cid"]), s(&base, &["station", "cid"]), "{knob}: another station");
        assert_ne!(s(&r, &["tracks", "0", "cid"]), s(&base, &["tracks", "0", "cid"]), "{knob}: another track");
        let seed = t.get("seed").and_then(JsonValue::as_f64).unwrap();
        assert_eq!(seed == base_seed, walk_kept, "{knob}: the walk kept or moved");
        // the retuned station's own text gives the same track at n
        let again = ask(&req(s(&r, &["station", "text"]), &format!(", \"n\": {n}")));
        assert_eq!(s(&again, &["tracks", "0", "cid"]), s(&r, &["tracks", "0", "cid"]), "{knob}: the tuned station is a station");
    }
    // a locked knob is refused; the neighbours never turn one
    let v = JsonValue::parse(&btwapi::station_json(&req(RADIO, ", \"retune\": {\"tempo\": \"+1\"}, \"locks\": [\"tempo\"]"))).unwrap();
    assert_eq!(v.get("ok").and_then(JsonValue::as_bool), Some(false));
    assert!(s(&v, &["error"]).contains("locked"));
    let nb = ask(&req(RADIO, ", \"n\": 3, \"neighbours\": true, \"locks\": [\"key\", \"seed\"]"));
    let list = nb.get("neighbours").unwrap().as_array().unwrap();
    assert!(!list.is_empty());
    for x in list {
        let k = x.get("knob").and_then(JsonValue::as_str).unwrap();
        assert!(k != "key" && k != "seed", "{k} is locked");
        assert_eq!(x.get("track").unwrap().get("n").and_then(JsonValue::as_f64), Some(3.0), "{k}");
    }
}

#[test]
fn the_segue_is_a_cross_with_its_own_cid_and_checks_clean() {
    let a = ask(&req(RADIO, ", \"n\": 4, \"count\": 2, \"segue\": true, \"check\": true"));
    let segs = a.get("segues").unwrap().as_array().unwrap();
    assert_eq!(segs.len(), 2);
    let sg = &segs[0];
    assert_eq!(sg.get("form").and_then(JsonValue::as_str), Some("cross"));
    assert_eq!(sg.get("from").and_then(JsonValue::as_f64), Some(4.0));
    let cid = sg.get("cid").and_then(JsonValue::as_str).unwrap();
    assert!(cid.starts_with('z'));
    for t in a.get("tracks").unwrap().as_array().unwrap() {
        assert_ne!(t.get("cid").and_then(JsonValue::as_str), Some(cid), "the segue is its own program");
    }
    let chk = sg.get("check").unwrap();
    assert_eq!(chk.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", chk.js_stringified());
    assert_eq!(chk.get("errors").unwrap().as_array().unwrap().len(), 0);
    assert_ne!(chk.get("expanded_cid").and_then(JsonValue::as_str), Some(cid));
    // its text, checked here, is clean, and twice the same
    let text = sg.get("text").and_then(JsonValue::as_str).unwrap();
    assert!(text.contains("arrange from template(cross, @"), "{text}");
    let c = check(text);
    assert!(complaints(&c).is_empty(), "{:#?}", complaints(&c));
    assert_eq!(c.compiled.unwrap().cid, cid);
    assert_eq!(djlive::station::read(RADIO).unwrap().segue(4).unwrap().cid, cid);
}

#[test]
fn a_set_steps_a_generator_call_with_next() {
    let src = "set {\n  play gen@2 techno seed=42\n  mix 16 bars with crossfade\n  next seed=+1\n  mix 8 bars with echo_out\n  next seed=~\n}\n";
    let c = check(src);
    assert!(complaints(&c).is_empty(), "{:#?}", complaints(&c));
    let comp = c.compiled.unwrap();
    let set = comp.set.expect("a set report");
    assert_eq!(set.tracks.len(), 3);
    let notes: Vec<&str> = c.diagnostics.iter().filter(|d| d.message.starts_with("next: ")).map(|d| d.message.as_str()).collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with("next: gen@2 techno seed=43 -> z"), "{}", notes[0]);
    let third = djlive::station::stepped_call("gen@2 techno seed=43", djlive::ast::SeedStep::Hash).unwrap();
    assert!(notes[1].starts_with(&format!("next: {third} -> z")), "{}", notes[1]);
    // a track is the generated program the call names: the second is seed 43's
    let g43 = djlive::gen::resolve_word("gen@2 techno seed=43", &djlive::gen::Host::default()).unwrap();
    assert_eq!(set.tracks[1].cid, g43.cid);
    // `next` after a saved program, or first, is an error
    let first = check("set {\n  next seed=+1\n}\n");
    assert!(first.diagnostics.iter().any(|d| d.is_error() && d.message.contains("`play` a generator call first")), "{:#?}", first.diagnostics);
}

#[test]
fn the_example_station_checks_clean() {
    let src = include_str!("../../../examples/btw/techno-radio.btw");
    let c = check(src);
    assert!(complaints(&c).is_empty(), "{:#?}", complaints(&c));
    let st = c.compiled.unwrap().station.unwrap();
    assert_eq!(st.station.forms, ["seeded", "passacaglia", "rondo", "fugue"]);
    assert_eq!(st.tracks.len(), 4);
}

#[test]
fn a_bad_request_says_what_is_wrong() {
    for (r, says) in [
        ("{}".to_string(), "`station`"),
        (req(RADIO, ", \"count\": 99"), "`count`"),
        (req(RADIO, ", \"count\": 8, \"check\": true"), "with `check`"),
        (req(RADIO, ", \"nn\": 1"), "`nn` is no field"),
        (req(RADIO, ", \"retune\": {\"genre\": \"+1\"}"), "no notch"),
        (req(RADIO, ", \"locks\": [\"volume\"]"), "`volume` is no knob"),
        (req("station {\n  from gen@2 techno seed=42\n  forms seeded nope\n}\n", ""), "`nope` is no form"),
        (req(RADIO, ", \"n\": 20000"), "10000"),
    ] {
        let v = JsonValue::parse(&btwapi::station_json(&r)).unwrap();
        assert_eq!(v.get("ok").and_then(JsonValue::as_bool), Some(false), "{r}");
        let e = v.get("error").and_then(JsonValue::as_str).unwrap_or_default();
        assert!(e.contains(says), "{r}: {e}");
    }
}
