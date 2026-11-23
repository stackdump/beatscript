//! Regression tests of the fixes of review lens 3 (runtime and hosts) that live in `djhost`: each is the repro of a finding (R-xx
//! of the review) and fails without the fix.

use djengine::json::JsonValue;
use djengine::session::arrangement::Arrangement;
use djhost::beatscript_render::{arrangement, runtime_controls};
use djlive::env::Env;

/// The render's events of one kind of op, as (beat, tempo the event carries, the op's JSON).
fn render_events(src: &str, op: &str) -> Vec<(f64, f64, String)> {
    let c = djlive::check(src).compiled.unwrap_or_else(|| panic!("{src}"));
    let (j, _) = arrangement(&c);
    let (a, bad) = Arrangement::from_json(&j).unwrap();
    assert_eq!(bad, 0);
    a.events.iter().filter(|e| e.via == "btw" && e.op.op() == op).map(|e| (e.at, e.bpm, e.op.json().canonical_js())).collect()
}

/// R-02 (what was kept and what is new): a program with no sections evaluated while a deck plays has its bar 0 on the bar it was
/// evaluated in and its music on bar 1, as every program written before the extensions has always run live, and the offline render
/// starts the music on its bar 0 (nothing was playing): a timed line of such a program lands one bar later, counted from the first bar
/// you hear, in the render than it did live (a song counts from its first section in both). That is documented (docs/BEATSCRIPT.md "Time
/// words") and pinned here. What is new, a tempo curve, starts on the first bar you hear in both, so the tempo at a music position is
/// the same live and in the render. The runtime of `runtime_controls` is the real one on a fake clock (deck A plays from beat 0, the
/// program is evaluated on bar 0 and its music lands on the clock's bar 1, beat 4).
#[test]
fn r02_a_flat_programs_tempo_curve_is_the_same_live_and_in_the_render_and_its_timed_lines_are_kept_as_they_were() {
    let src = "tempo 100 -> 140 over 4 bars\ndeck A: kick x ~ x ~\ndeck B: bass c2\nat bar 2: master: level 0.5\nat bar 3 beat 2: master: level 0.25\nat bar 4: duck B under A.kick 6db\n";
    let live = runtime_controls(src, &Env::none(), 8, 0.25).expect("the program runs");
    // (the live beats from the first bar you hear, the clock's bar 1)
    let live_at = |noun: &str| -> Vec<f64> { live.iter().filter(|c| c.noun == noun).map(|c| c.beat - 4.0).collect() };
    let render_at = |op: &str| -> Vec<f64> { render_events(src, op).iter().map(|e| e.0).collect() };
    // the timed lines: bar 2 is the clock's bar 2, the second bar you hear (music beat 4); the render has it a bar later, on its own bar 2
    assert_eq!(live_at("mixer"), vec![4.0, 10.0], "{live:?}");
    assert_eq!(render_at("master"), vec![8.0, 14.0]);
    assert_eq!(live_at("trick"), vec![12.0], "{live:?}");
    assert_eq!(render_at("trick"), vec![16.0]);
    // the tempo curve: the same tempo at the same music beat, to a step of the render's staircase (the render holds each step's mean;
    // the runtime sends the position's own)
    let sent = |beat: f64| -> f64 {
        live.iter().filter(|c| c.noun == "deck" && c.beat - 4.0 <= beat + 1e-9 && c.params.get("action").and_then(JsonValue::as_str) == Some("tempo")).filter_map(|c| c.params.get("bpm").and_then(JsonValue::as_f64)).last().expect("a tempo was sent")
    };
    let mut checked = 0;
    for op in ["master", "trick"] {
        for (beat, bpm, _) in render_events(src, op) {
            assert!((bpm - sent(beat)).abs() < 2.0, "at music beat {beat}: the render is at {bpm} BPM, the runtime sent {} (a quarter of the ramp, 10 BPM, apart when the curve began a bar early)", sent(beat));
            checked += 1;
        }
    }
    assert_eq!(checked, 3);
    // the ramp starts on the first bar you hear: the first tempo sent is the first of the curve, 100, and 140 is reached four bars from there
    let first = live.iter().find(|c| c.noun == "deck" && c.params.get("action").and_then(JsonValue::as_str) == Some("tempo")).unwrap();
    assert_eq!(first.params.get("bpm").and_then(JsonValue::as_f64), Some(100.0), "{first:?}");
    assert_eq!(sent(12.0), 130.0);
    assert_eq!(sent(16.0), 140.0);
    // and a song with the same curve in a section counts the same: the song's lines are on the music bars of the render
    let song = src.replace("tempo 100 -> 140 over 4 bars\n", "").replace("deck A:", "section one 8 bars {\n  tempo 100 -> 140 over 4 bars\n  deck A:").replace("at bar 4: duck B under A.kick 6db\n", "at bar 4: duck B under A.kick 6db\n}\narrange one\n");
    let as_song = runtime_controls(&song, &Env::none(), 8, 0.25).expect("the song runs");
    let song_at = |noun: &str| -> Vec<f64> { as_song.iter().filter(|c| c.noun == noun).map(|c| c.beat - 4.0).collect() };
    assert_eq!(song_at("mixer"), render_events(&song, "master").iter().map(|e| e.0).collect::<Vec<_>>());
    assert_eq!(song_at("trick"), render_events(&song, "trick").iter().map(|e| e.0).collect::<Vec<_>>());
    let song_tempos = |v: &[djhost::beatscript_render::ReplayedCall]| -> Vec<(f64, f64)> { v.iter().filter(|c| c.noun == "deck").filter_map(|c| c.params.get("bpm").and_then(JsonValue::as_f64).map(|b| (c.beat, b))).collect() };
    assert_eq!(song_tempos(&as_song), song_tempos(&live), "a song's curve and a program's with no sections start on the same bar");
}

/// R-06: `knob(N)` and `pad(N)` were checked against the connected controllers' bank, so acceptance depended on the machine: `eval`,
/// `check`, `fmt`, `share`, `open` and `test` used it, `LiveStore::check` (save, import, jsonld) used the default bank, and a program
/// saved fine could fail to load with an MPK mini (8 knobs) attached. Now every one of them checks against the same fixed ceiling and
/// the bank is a run-time matter.
#[test]
fn r06_the_same_program_checks_the_same_whatever_controllers_are_connected() {
    use djhost::beatscript_env::BeatscriptEnv;
    use djhost::live_store::LiveStore;
    use djlive::perform::InputBank;
    let src = "tempo 120\ndeck A: kick x ~ x ~\nmaster: level knob(12)\nevery 2 bars: master: level 0.5 when pad(40) > 0\n";
    let dir = std::env::temp_dir().join(format!("djhost-fixc-r06-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = LiveStore::new(&dir);
    let mut cids = Vec::new();
    for bank in [None, Some(InputBank { knobs: 8, pads: 16 }), Some(InputBank { knobs: 9, pads: 64 }), Some(InputBank::DEFAULT)] {
        let be = match bank {
            Some(b) => BeatscriptEnv::new(Some(&store)).with_bank(b),
            None => BeatscriptEnv::new(Some(&store)),
        };
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.compiled.is_some(), "{bank:?}: {}", djlive::diag::render_all(&c.diagnostics, src));
        cids.push(c.compiled.unwrap().cid);
    }
    assert!(cids.windows(2).all(|w| w[0] == w[1]), "{cids:?}");
    // the store's own check (what save, import and jsonld use) agrees, and a saved program loads
    assert_eq!(store.check(src).compiled.map(|c| c.cid), Some(cids[0].clone()));
    let saved = store.save("twelve knobs", src).expect("saved");
    assert_eq!(saved.cid, cids[0]);
    assert!(store.load(&saved.cid).is_ok());
    assert!(store.jsonld_text(src).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-17: a saved name with a newline in it was in the CID but written as a space by the file, so the object no longer matched its own
/// name, was unlisted, and a second save of the program made another. The name is one line before the CID is made.
/// R-18: deleting a snippet saved before there were rights (`<cid>.json`, no `.beatscript`) removed it and then answered an error.
#[test]
fn r17_r18_a_name_is_one_line_and_a_legacy_snippet_deletes_cleanly() {
    use djengine::json::{JsonObject, JsonValue};
    use djhost::live_store::LiveStore;
    let dir = std::env::temp_dir().join(format!("djhost-fixc-r17-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = LiveStore::new(&dir);
    let src = "tempo 120\ndeck A: kick x ~ x ~\n";
    let first = store.save("two\nlines\r\nhere", src).expect("saved");
    assert_eq!(first.name, "two lines  here", "one line: each control character is a space");
    // it is listed, under that name, and loads
    let list = store.list();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].name, first.name);
    assert!(store.load(&first.object).is_ok());
    // the same program saved again under the same name is the same object, not a second
    let again = store.save("two\nlines\r\nhere", src).expect("saved");
    assert_eq!(again.object, first.object);
    assert_eq!(store.list().len(), 1);
    // a name of nothing but a newline is no name: the default one
    let blank = store.save("\n", "tempo 100\ndeck A: kick x\n").expect("saved");
    assert!(blank.name.starts_with("btw "), "{}", blank.name);
    // R-18: a snippet from before there were rights
    let c = djlive::check("deck A: snare ~ x ~ x\n").compiled.unwrap();
    let doc = JsonValue::Object(JsonObject::from_pairs([("cid", JsonValue::str(c.cid.clone())), ("format", JsonValue::int(1)), ("name", JsonValue::str("old")), ("text", JsonValue::str(c.text.clone())), ("type", JsonValue::str("djrust.live"))]));
    std::fs::write(dir.join(format!("{}.json", c.cid)), doc.canonical_js()).unwrap();
    assert!(store.list().iter().any(|e| e.name == "old"));
    let gone = store.delete("old").expect("a legacy snippet deletes without an error");
    assert_eq!(gone.name, "old");
    assert!(!store.list().iter().any(|e| e.name == "old"));
    assert!(!dir.join(format!("{}.json", c.cid)).exists());
    // deleting what is not there is still an error
    assert!(store.delete("old").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// L-13: the render's note for a tempo curve past the tempo fader's travel named the wrong tempo: `tempo 300 -> 40 over 1 bar exp` and a
/// later `40 -> 300` said "the tempo goes to 560 from the net's own 300" (the size of the largest deviation with the sign of any step
/// above the net's own). It names the tempo of the step that is furthest away, on whichever side.
#[test]
fn l13_the_render_note_names_the_tempo_the_curve_goes_to() {
    let down = "tempo 300 -> 40 over 1 bar exp\ndeck A: kick x ~ x ~\nat bar 2: tempo 40 -> 300 over 1 beat step 1024\n";
    let notes = djhost::beatscript_tempo::notes(&djlive::check(down).compiled.unwrap());
    assert_eq!(notes.len(), 1, "{notes:?}");
    let goes_to = |n: &str| -> f64 { n.split("the tempo goes to ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap() };
    let t = goes_to(&notes[0]);
    assert!(t < 60.0 && t >= 39.0, "the curve goes down to about 40: {}", notes[0]);
    assert!(notes[0].contains("from the net's own 300"), "{}", notes[0]);
    // and up: the curve's top
    let up = "tempo 100 -> 250 over 2 bars\ndeck A: kick x ~ x ~\n";
    let notes = djhost::beatscript_tempo::notes(&djlive::check(up).compiled.unwrap());
    assert_eq!(notes.len(), 1, "{notes:?}");
    let t = goes_to(&notes[0]);
    assert!(t > 240.0 && t <= 250.0, "{}", notes[0]);
}
