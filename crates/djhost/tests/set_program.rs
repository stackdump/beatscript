//! A recorded set written out as a BeatScript `set` (djhost::set_program): the program a fixture
//! recording is (three programs, a bass swap, a filter exit, a tempo change and a line that is no
//! program), its CIDs from run to run, that `check` accepts it with the tracks' texts, a generator
//! track played as its seed, and the restricted case.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeEvent, ArrangeOp, Arrangement};
use djengine::session::model::DeckSource;
use djengine::setrec::{self, AudioRef, Entry, EntryKind, RecordedSet};
use djengine::TrackSpec;
use djhost::live_store::Object;
use djhost::set_program::{cli_with, set_program, Options, SetProgram, TAKE_TYPE};
use djlive::env::{Env, StaticSnippets};

const ALPHA: &str = "# alpha\ntempo 124\nkey am\ndeck A: kick x ~ ~ ~ x ~ ~ ~ vel=100 | bass a2 ~ ~ ~ vel=100\nloop\n";
const BETA: &str = "# beta\ntempo 124\nkey em\nsection intro 8 bars {\n deck B: kick x ~ x ~ vel=100\n}\nsection drop 16 bars {\n deck B: kick x x x x vel=100 | melody e4 ~ g4 ~ vel=100\n}\nsection outro 8 bars {\n deck B: kick x ~ x ~ vel=100\n}\narrange intro drop outro\n";
const GAMMA: &str = "# gamma\ntempo 128\nkey bm\ndeck A: kick x ~ ~ ~ vel=100 | bass b1 ~ b1 ~ vel=100\nend at bar 64\n";
const AUTHOR: &str = "did:key:z6MkTestAuthor";

/// The saved programs: name, program CID and canonical text, as the store gives them.
fn store() -> StaticSnippets {
    let mut s = StaticSnippets::default();
    for (name, text) in [("alpha", ALPHA), ("beta", BETA), ("gamma", GAMMA)] {
        let c = djlive::check(text).compiled.unwrap_or_else(|| panic!("{name} does not compile"));
        s.items.push((name.to_string(), c.cid, c.text));
    }
    s
}

/// The store, and the programs a set program carries (a generator track's seed).
fn with_tracks(sp: &SetProgram) -> StaticSnippets {
    let mut s = store();
    for (cid, text) in &sp.tracks {
        s.items.push((cid.clone(), cid.clone(), text.clone()));
    }
    s
}

fn beat(bar: i64) -> f64 {
    ((bar - 1) * 4) as f64
}

/// 72 bars at 124, the master at 128 from bar 43; the crossfader across at bars 25 and 45, moved
/// from the controller and the window; `more` added.
fn arrangement(more: Vec<ArrangeEvent>) -> Arrangement {
    let ev = |bar: i64, bpm: f64, x: f64, via: &str| ArrangeEvent { at: beat(bar), bpm, op: ArrangeOp::Deck(DeckAction::Crossfader(x)), via: via.into(), text: String::new() };
    let mut events = vec![ev(25, 124.0, 1.0, "Inpulse crossfader"), ev(43, 128.0, 0.6, "window"), ev(45, 128.0, 0.0, "Inpulse crossfader"), ev(72, 128.0, 0.0, "window")];
    events.extend(more);
    Arrangement::new(124.0, BTreeMap::new(), events)
}

fn entry(arr: &Arrangement, bar: i64, off: f64, kind: EntryKind, deck: &str, cid: &str, label: &str) -> Entry {
    let b = beat(bar) + off;
    Entry { t: arr.seconds_at(b), beat: b, kind, deck: deck.into(), cid: cid.into(), label: label.into() }
}

fn net_cid(name: &str) -> String {
    djengine::cid::share_cid(&JsonValue::parse(&format!(r#"{{"name":"{name}","parts":{{}}}}"#)).unwrap())
}

fn mashup_cid() -> String {
    djengine::cid::share_cid(&JsonValue::parse(r#"{"type":"djrust.mashup","name":"night bus x techno:7"}"#).unwrap())
}

/// The fixture: `extra` lines and `more` arrangement moves added.
fn recording_with(extra: &[(i64, EntryKind, &str, String, &str)], more: Vec<ArrangeEvent>) -> RecordedSet {
    let s = store();
    let (a, b, c) = (&s.items[0].1, &s.items[1].1, &s.items[2].1);
    let arr = arrangement(more);
    let mut list = vec![
        entry(&arr, 1, 0.0, EntryKind::Track, "A", a, "alpha"),
        // the raw nets the program loads on the decks: its own sections, not tracks of their own
        entry(&arr, 1, 0.25, EntryKind::Track, "A", &net_cid("live deck A"), "live deck A"),
        entry(&arr, 17, 0.0, EntryKind::Track, "B", b, "beta"),
        entry(&arr, 17, 0.5, EntryKind::Track, "B", &net_cid("live deck B"), "live deck B"),
        entry(&arr, 20, 0.0, EntryKind::Transition, "", "", "bass_swap B: swapped"),
        entry(&arr, 25, 0.0, EntryKind::Transition, "", "", "crossfade A → B"),
        entry(&arr, 41, 0.0, EntryKind::Track, "A", c, "gamma"),
        entry(&arr, 43, 0.0, EntryKind::Transition, "", "", "filter_exit B: done"),
        entry(&arr, 45, 0.0, EntryKind::Transition, "", "", "crossfade B → A"),
    ];
    for (bar, kind, deck, cid, label) in extra {
        list.push(entry(&arr, *bar, 0.0, *kind, deck, cid, label));
    }
    list.sort_by(|x, y| x.t.partial_cmp(&y.t).unwrap());
    RecordedSet {
        name: "friday".into(),
        started: "2026-11-20T22:00:00Z".into(),
        bpm: 124.0,
        duration: arr.seconds_at(beat(73)),
        audio: Some(AudioRef { cid: setrec::raw_cid(b"take"), file: "set.wav".into(), frames: 48_000, sample_rate: 48_000.0, channels: 2, format: "wav-f32".into() }),
        tracklist: list,
        arrangement: arr.json(),
        bars: (1, 72),
        audio_offset: 0.0,
        forked_from: None,
    }
}

/// The fixture with its one line that is no program: a mashup on deck A at bar 33.
fn recording() -> RecordedSet {
    recording_with(&[(33, EntryKind::Mashup, "A", mashup_cid(), "night bus x techno:7")], Vec::new())
}

fn opts(s: &StaticSnippets) -> Options<'_> {
    Options { author: Some(AUTHOR), programs: Some(s) }
}

/// The text from `set {` on (the header is the live store's, checked line by line).
fn body(text: &str) -> &str {
    &text[text.find("set {").expect("a set")..]
}

fn compiles(text: &str, s: &StaticSnippets) -> djlive::Compiled {
    let chk = djlive::compile::check_with(text, &Env { snippets: Some(s), ..Env::default() });
    chk.compiled.unwrap_or_else(|| panic!("{:?}\n{text}", chk.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()))
}

#[test]
fn a_recorded_set_is_written_as_a_set_of_its_programs() {
    let s = store();
    let (a, b, c) = (&s.items[0].1, &s.items[1].1, &s.items[2].1);
    let sp = set_program(&recording(), &opts(&s)).unwrap();
    let mashup = djengine::rights::short_cid(&mashup_cid());
    let want = format!(
        "set {{
  # 0:00 alpha
  play {a} until bar 24
  # performed: bass_swap
  mix 8 bars with crossfade
  # 0:30 beta
  play {b} until bar 28
  # not a program: mashup night bus x techno:7 ({mashup}), bars 33-40: the track before plays on through them
  mix 4 bars with filter_exit, tempo 124 -> 128
  # 1:17 gamma
  play {c} until bar 32
}}
"
    );
    assert_eq!(body(&sp.text), want, "{}", sp.text);
    // BeatScript 2: the version line first, so the set is a new program's (its CID of the `beatscript` type)
    assert!(sp.text.starts_with("#! beatscript 2\n"), "{}", sp.text);
    assert_eq!(djlive::version::of_source(&sp.text), djlive::version::Version::V2);
    // the header: the live store's, so `btw import` reads the file back as it is
    for h in ["#! name: friday\n", "#! license: CC0-1.0\n", "#! visibility: public\n", &format!("#! author: {AUTHOR}\n")] {
        assert!(sp.text.contains(h), "{h:?} in {}", sp.text);
    }
    let (obj, _) = Object::from_file(&sp.text).unwrap();
    assert_eq!((obj.name.as_str(), obj.rights.author.as_deref()), ("friday", Some(AUTHOR)));
    assert!(!sp.text.contains("use restricted") && sp.tracks.is_empty());
}

#[test]
fn the_program_and_take_cids_are_the_same_from_run_to_run() {
    let s = store();
    let rec = recording();
    let one = set_program(&rec, &opts(&s)).unwrap();
    let two = set_program(&RecordedSet::from_json(&rec.to_json()).unwrap(), &opts(&s)).unwrap();
    assert_eq!(one, two);
    assert!(one.program_cid.starts_with('z') && one.take_cid.starts_with('z'));
    // the take: canonical, named by its CID, naming the program by CID (its text holds no take)
    let t = &one.take;
    assert_eq!(t.get("type").and_then(JsonValue::as_str), Some(TAKE_TYPE));
    assert_eq!(t.get("format").and_then(JsonValue::as_f64), Some(1.0));
    assert_eq!(t.get("program").and_then(JsonValue::as_str), Some(one.program_cid.as_str()));
    assert_eq!(t.get("recording").and_then(JsonValue::as_str), Some(rec.cid().as_str()));
    assert_eq!(t.get("author").and_then(JsonValue::as_str), Some(AUTHOR));
    assert_eq!(t.get("tracklist").and_then(JsonValue::as_array).map(Vec::len), Some(rec.tracklist.len()));
    assert_eq!(t.get("arrangement"), Some(&rec.arrangement));
    assert!(t.get("tracks").is_none());
    // what it was played with, as the recording says: the controls of its moves
    assert_eq!(t.get("controls").and_then(JsonValue::strings), Some(vec!["Inpulse crossfader".to_string(), "window".to_string()]));
    assert_eq!(one.take_cid, djengine::cid::share_cid(t));
    assert!(!one.text.contains(&one.take_cid));
    // comments and the header are not the program: another author, the same program, another take
    let other = set_program(&rec, &Options { author: Some("did:key:z6MkSomeoneElse"), programs: Some(&s) }).unwrap();
    assert_eq!(other.program_cid, one.program_cid);
    assert_ne!(other.take_cid, one.take_cid);
    assert_eq!(one.to_json().get("program_cid").and_then(JsonValue::as_str), Some(one.program_cid.as_str()));
}

#[test]
fn check_accepts_the_program_when_it_has_the_tracks_texts() {
    let s = store();
    let (a, b, c) = (&s.items[0].1, &s.items[1].1, &s.items[2].1);
    let sp = set_program(&recording(), &opts(&s)).unwrap();
    let chk = djlive::compile::check_with(&sp.text, &Env { snippets: Some(&s), ..Env::default() });
    let warnings: Vec<String> = chk.diagnostics.iter().map(|d| d.message.clone()).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
    let comp = compiles(&sp.text, &s);
    // the same CID as the language gives it, of the same canonical text
    assert_eq!(comp.cid, sp.program_cid);
    assert_eq!(comp.text, format!("#! beatscript 2\nset {{\n  play {a} until bar 24\n  mix 8 bars with crossfade\n  play {b} until bar 28\n  mix 4 bars with filter_exit, tempo 124 -> 128\n  play {c} until bar 32\n}}\n"));
    // the set as played: 72 bars, each track where the recording had it
    let r = comp.set.as_ref().unwrap();
    assert_eq!(r.total_bars, 72);
    assert_eq!(r.tracks.iter().map(|t| (t.start, t.bars)).collect::<Vec<_>>(), vec![(0, 24), (16, 28), (40, 32)]);
    assert_eq!(r.mixes[1].plan, "mix 2→3: 124→128 over 4 bars (filter_exit)");
}

#[test]
fn a_generator_track_the_booth_loaded_is_played_as_its_seed() {
    let s = store();
    // techno:7 loaded on deck A at bar 33: a plain generator track, so `arrange from seed(techno, 7)`
    let spec = TrackSpec::new("techno", 7.0, Some("standard"));
    let net = djengine::cid::share_cid(&spec.payload());
    let load = ArrangeEvent { at: beat(33), bpm: 124.0, op: ArrangeOp::Deck(DeckAction::Load { deck: "A".into(), source: DeckSource::Net(spec.clone()) }), via: "Inpulse browser".into(), text: String::new() };
    let rec = recording_with(&[(33, EntryKind::Track, "A", net.clone(), "techno:7")], vec![load]);
    let sp = set_program(&rec, &opts(&s)).unwrap();
    let seeded = djlive::check("#! beatscript 2\narrange from seed(techno, 7)\n").compiled.unwrap();
    assert_eq!(sp.tracks, vec![(seeded.cid.clone(), seeded.text.clone())]);
    let b = body(&sp.text);
    assert!(b.contains(&format!("  # 1:01 techno:7: the generator's notes, as arrange from seed(techno, 7)\n  play {} until bar 12\n", seeded.cid)), "{b}");
    // beta goes up to it (no move between them: a cut), and the mix into gamma is from it
    assert!(b.contains("until bar 16\n  # 1:01 techno:7"), "{b}");
    // the take carries its text, so `check` reads the set where nothing is saved but the programs
    assert_eq!(sp.take.get("tracks").and_then(|t| t.get(&seeded.cid)).and_then(JsonValue::as_str), Some(seeded.text.as_str()));
    let comp = compiles(&sp.text, &with_tracks(&sp));
    assert_eq!(comp.cid, sp.program_cid);
    assert_eq!(comp.set.as_ref().unwrap().total_bars, 72);
    // a net with anything changed (a part seed here) is not the seed's music: it stays a comment
    let mut changed = spec.clone();
    changed.part_seeds.insert("bass".into(), 3.0);
    let load = ArrangeEvent { at: beat(33), bpm: 124.0, op: ArrangeOp::Deck(DeckAction::Load { deck: "A".into(), source: DeckSource::Net(changed) }), via: "Inpulse browser".into(), text: String::new() };
    let sp = set_program(&recording_with(&[(33, EntryKind::Track, "A", net.clone(), "techno:7")], vec![load]), &opts(&s)).unwrap();
    assert!(sp.tracks.is_empty() && body(&sp.text).contains("# not a program: track techno:7"), "{}", sp.text);
    // nor is one the arrangement does not hold (it cannot be told from one with changes)
    let sp = set_program(&recording_with(&[(33, EntryKind::Track, "A", net, "techno:7")], Vec::new()), &opts(&s)).unwrap();
    assert!(sp.tracks.is_empty(), "{}", sp.text);
}

#[test]
fn a_sample_on_a_deck_makes_the_set_restricted_and_private() {
    let s = store();
    let sp = set_program(&recording_with(&[(9, EntryKind::Track, "B", "ab".repeat(32), "a sample")], Vec::new()), &opts(&s)).unwrap();
    assert!(sp.text.contains("#! visibility: private\n"), "{}", sp.text);
    assert!(sp.text.ends_with("}\nuse restricted\n"), "{}", sp.text);
    assert!(sp.text.contains("  # not a program: track a sample (a…abababab), outside audio, bars 9-16: the track before plays on through them\n"), "{}", sp.text);
    let comp = compiles(&sp.text, &s);
    assert_eq!(comp.cid, sp.program_cid);
    assert!(comp.rights.declared_private);
    // `use restricted` is in the program: another program than the public one
    assert_ne!(sp.program_cid, set_program(&recording_with(&[], Vec::new()), &opts(&s)).unwrap().program_cid);
}

#[test]
fn what_is_not_a_program_here_is_said_with_its_bars_and_never_guessed() {
    // nothing to look CIDs up in: no line is a program, and there is no set
    let e = set_program(&recording(), &Options { author: None, programs: None }).unwrap_err();
    assert!(e.contains("no line of the tracklist is a program"), "{e}");
    // a live-coded program nobody marked: its nets are one gap before the first program, not a track each
    let s = store();
    let arr = arrangement(Vec::new());
    let mut rec = recording();
    rec.tracklist.retain(|e| e.label != "alpha");
    for k in 0..3 {
        rec.tracklist.push(entry(&arr, 1 + 4 * k, 1.0, EntryKind::Track, "A", &net_cid(&format!("live deck A {k}")), "live deck A"));
    }
    rec.tracklist.sort_by(|x, y| x.beat.partial_cmp(&y.beat).unwrap());
    let sp = set_program(&rec, &Options { author: None, programs: Some(&s) }).unwrap();
    let b = body(&sp.text);
    assert!(
        b.starts_with("set {\n  # gap: 16 bars (track live deck, 4 net loads, a live-coded program the tracklist does not name): the set starts at bar 17 of the recording\n  # 0:30 beta\n"),
        "{b}"
    );
    // the moves into beta were from no program: said where they fell, and no mix before the first play
    assert!(b.contains("  # performed: bass_swap B: swapped\n  # performed: crossfade A → B\n"), "{b}");
    assert!(!b.contains("mix 8 bars"), "{b}");
    // no author: none in the header, none in the take
    assert!(!sp.text.contains("#! author:") && sp.take.get("author").is_none(), "{}", sp.text);
    // a line that is no program before the first one is a gap too
    rec.tracklist.retain(|e| !e.label.starts_with("live deck"));
    rec.tracklist.push(entry(&arr, 9, 0.0, EntryKind::LoopSet, "B", &net_cid("loops"), "loop set 3"));
    rec.tracklist.sort_by(|x, y| x.beat.partial_cmp(&y.beat).unwrap());
    let sp = set_program(&rec, &Options { author: None, programs: Some(&s) }).unwrap();
    let short = djengine::rights::short_cid(&net_cid("loops"));
    assert!(body(&sp.text).starts_with(&format!("set {{\n  # gap: 8 bars (loopset loop set 3 ({short})): the set starts at bar 17 of the recording\n")), "{}", sp.text);
}

#[test]
fn the_cli_writes_the_program_and_the_take_by_their_cids() {
    let s = store();
    let rec = recording();
    let dir = std::env::temp_dir().join(format!("djsetprog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("rec");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("set.json"), rec.to_json().canonical_js()).unwrap();
    let out = dir.join("out");
    let answer = JsonValue::parse(&cli_with(&[src.display().to_string(), "--out".into(), out.display().to_string()], &s).unwrap()).unwrap();
    let want = set_program(&rec, &Options { author: djhost::tracks::default_author().as_deref(), programs: Some(&s) }).unwrap();
    assert_eq!(answer.get("program_cid").and_then(JsonValue::as_str), Some(want.program_cid.as_str()));
    assert_eq!(answer.get("take_cid").and_then(JsonValue::as_str), Some(want.take_cid.as_str()));
    assert_eq!(std::fs::read_to_string(out.join("friday.beatscript")).unwrap(), want.text);
    let take = JsonValue::parse(&std::fs::read_to_string(out.join(format!("{}.take.json", want.take_cid))).unwrap()).unwrap();
    assert_eq!(djengine::cid::share_cid(&take), want.take_cid, "the file is named by its content's CID");
    // set.json itself names the same recording
    let again = cli_with(&[src.join("set.json").display().to_string(), "-o".into(), out.display().to_string()], &s).unwrap();
    assert_eq!(JsonValue::parse(&again).unwrap().get("take_cid"), answer.get("take_cid"));
    assert!(cli_with(&[], &s).unwrap_err().contains("set-program <recording dir | set.json>"));
    // a generator track it plays is written beside it, a program `--with` can hand to check
    let spec = TrackSpec::new("techno", 7.0, Some("standard"));
    let load = ArrangeEvent { at: beat(33), bpm: 124.0, op: ArrangeOp::Deck(DeckAction::Load { deck: "A".into(), source: DeckSource::Net(spec.clone()) }), via: String::new(), text: String::new() };
    let seeded = recording_with(&[(33, EntryKind::Track, "A", djengine::cid::share_cid(&spec.payload()), "techno:7")], vec![load]);
    std::fs::write(src.join("set.json"), seeded.to_json().canonical_js()).unwrap();
    let answer = JsonValue::parse(&cli_with(&[src.display().to_string(), "--out".into(), out.display().to_string()], &s).unwrap()).unwrap();
    let track = out.join("tracks").join("seed-techno-7.beatscript");
    assert_eq!(answer.get("tracks").and_then(JsonValue::strings), Some(vec![track.display().to_string()]));
    let (obj, _) = Object::from_file(&std::fs::read_to_string(&track).unwrap()).unwrap();
    let file = std::fs::read_to_string(&track).unwrap();
    assert!(file.starts_with("#! beatscript 2\n"), "{file}");
    assert_eq!(djlive::check(&file).compiled.unwrap().cid, djlive::check("#! beatscript 2\narrange from seed(techno, 7)\n").compiled.unwrap().cid, "{}", obj.text);
    let _ = std::fs::remove_dir_all(&dir);
}
