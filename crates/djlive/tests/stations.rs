//! Radio: stations (docs/btw/radio.md), with no generator: the walk, the tracks and segues as text and CIDs, retuning and
//! locks. The checks with the seeded generator are crates/btwapi/tests/station.rs; the listen numbers crates/btwd.

use djlive::ast::SeedStep;
use djlive::station::{self, Station};

const RADIO: &str = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia rondo fugue\n}\n";

fn radio() -> Station {
    station::read(RADIO).expect("the station reads")
}

fn errors(src: &str) -> Vec<String> {
    match station::read(src) {
        Ok(_) => Vec::new(),
        Err(ds) => ds.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect(),
    }
}

#[test]
fn a_station_reads_its_lines_and_prints_itself_canonically() {
    let s = radio();
    assert_eq!(s.genre, "techno");
    assert_eq!(s.version, 2);
    assert_eq!(s.seed, "42");
    assert_eq!(s.step, SeedStep::Hash);
    assert_eq!(s.forms, ["seeded", "passacaglia", "rondo", "fugue"]);
    assert_eq!(s.tempo, Some(128.0));
    assert_eq!(s.key.as_deref(), Some("am"));
    assert_eq!(s.text(), RADIO, "the station's text is its canonical program");
    // spacing, comments and a default spelled out or not: the canonical text and CID of a program, as for any program
    let messy = "# my radio\ntempo   128\nkey am\nstation {\n\n  from gen@2 techno seed = 42   # the start\n  next seed=~\n  forms seeded passacaglia rondo fugue\n}\n";
    let p = djlive::parse::parse(messy);
    assert!(p.diagnostics.iter().all(|d| !d.is_error()), "{:?}", p.diagnostics);
    assert_eq!(djlive::print::canonical(&p.program), RADIO);
    assert_eq!(station::read(messy).unwrap(), s);
    // a station read from a program has that program's CID, however its lines are ordered or say the defaults
    let late = "station {\n  from gen@2 techno seed=42\n  next seed=+1\n  forms seeded\n}\ntempo 128\n";
    let st = station::read(late).unwrap();
    let p2 = djlive::parse::parse(late);
    assert_eq!(st.cid(), djlive::compile::program_cid(&djlive::print::canonical(&p2.program)));
    assert!(st.track(0).unwrap().text.starts_with(&format!("#! station: {} track 0", st.cid())));
    // a tempo or key the generator cannot be fitted to is an error, not ignored
    assert!(station::read("tempo 400\nstation {\n  from gen@2 techno seed=42\n}\n").is_err());
    // the defaults are left out of the text
    let plain = station::read("station {\n  from gen@2 house seed=7\n}\n").unwrap();
    assert_eq!(plain.step, SeedStep::By(1));
    assert_eq!(plain.forms, ["seeded"]);
    assert_eq!(plain.text(), "station {\n  from gen@2 house seed=7\n}\n");
}

#[test]
fn position_n_is_always_the_same_track() {
    let s = radio();
    for n in [0u64, 1, 2, 3, 4, 7, 50] {
        let a = s.track(n).unwrap();
        let b = radio().track(n).unwrap();
        assert_eq!(a, b, "track {n} twice");
        assert_eq!(a.n, n);
        assert!(a.cid.starts_with('z'));
        // the CID is of the canonical text: the header lines are comments
        let p = djlive::parse::parse(&a.text);
        assert_eq!(djlive::compile::program_cid(&djlive::print::canonical(&p.program)), a.cid);
    }
    // a run of tracks is the tracks one at a time, each naming the one before as its parent
    let run = s.tracks(3, 4).unwrap();
    for (i, t) in run.iter().enumerate() {
        assert_eq!(*t, s.track(3 + i as u64).unwrap());
    }
    assert_eq!(run[1].parent.as_deref(), Some(run[0].cid.as_str()));
    assert!(s.track(0).unwrap().parent.is_none());
    assert!(run[1].text.contains(&format!("#! from: next({}) seed=~", run[0].cid)));
    assert!(run[0].text.starts_with(&format!("#! station: {} track 3\n", s.cid())));
}

#[test]
fn the_forms_turn_and_the_chapter_theme_recurs() {
    let s = radio();
    let t = s.tracks(0, 9).unwrap();
    let forms: Vec<&str> = t.iter().map(|t| t.form.as_str()).collect();
    assert_eq!(forms, ["seeded", "passacaglia", "rondo", "fugue", "seeded", "passacaglia", "rondo", "fugue", "seeded"]);
    // chapter 0 is tracks 0..4: track 0's seed is the theme of all four, bound as the form's first role
    for k in 0..4 {
        assert_eq!(t[k].theme, t[0].seed, "track {k}");
    }
    assert_eq!(t[5].theme, t[4].seed);
    assert!(t[0].text.contains("arrange from gen@2 techno seed=42\n"));
    assert!(t[1].text.contains(&format!("ground=gen@2 techno seed=42, upper=gen@2 techno seed={}", t[1].seed)), "{}", t[1].text);
    assert!(t[2].text.contains(&format!("refrain=gen@2 techno seed=42, episodes=gen@2 techno seed={} part=arp", t[2].seed)), "{}", t[2].text);
    assert!(t[3].text.contains(&format!("subject=gen@2 techno seed=42, counter=gen@2 techno seed={}", t[3].seed)), "{}", t[3].text);
    // the forms sing a pinned public-domain line
    assert!(t[1].phrase.as_deref().is_some_and(|p| station::PHRASES.contains(&p.trim_start_matches('@'))));
    assert!(t[0].phrase.is_none());
    // every track is generated at the station's tempo and in its key
    for x in &t {
        assert!(x.text.contains("\ntempo 128\nkey am\narrange from "), "{}", x.text);
    }
}

#[test]
fn the_walk_counts_on_or_draws_from_the_cid_before() {
    let by = station::read("station {\n  from gen@2 techno seed=42\n  next seed=+3\n}\n").unwrap();
    assert_eq!(by.seeds(4).unwrap(), [42, 45, 48, 51, 54]);
    // seed=+N wraps at 2^32
    let top = station::read("station {\n  from gen@2 techno seed=4294967295\n}\n").unwrap();
    assert_eq!(top.seeds(2).unwrap(), [4294967295, 0, 1]);
    // a counted walk has a track at any position, far past what a list of its seeds holds
    let far = by.track(9_000_000_000_000).unwrap();
    assert_eq!(far.seed, 42u32.wrapping_add((3u64.wrapping_mul(9_000_000_000_000) & 0xffff_ffff) as u32));
    assert!(by.seeds(station::MAX_WALK + 1).is_err());
    assert!(radio().track(station::MAX_WALK + 1).is_err(), "a hashed walk is worked out to MAX_WALK");
    // the last position has its track and the segue after it
    assert_eq!(radio().track(station::MAX_WALK).unwrap().n, station::MAX_WALK);
    assert_eq!(radio().segue(station::MAX_WALK).unwrap().to, station::MAX_WALK + 1);
    // seed=~ draws the next seed from the CID of `arrange from <the call>`
    let s = radio();
    let seeds = s.seeds(3).unwrap();
    assert_eq!(seeds[0], 42);
    assert_eq!(seeds[1], station::fold32(&station::call_cid("gen@2 techno seed=42")));
    assert_eq!(seeds[2], station::fold32(&station::call_cid(&format!("gen@2 techno seed={}", seeds[1]))));
    // a set's `next` steps a call the same way
    assert_eq!(station::stepped_call("gen@2 techno seed=42", SeedStep::Hash).unwrap(), format!("gen@2 techno seed={}", seeds[1]));
    assert_eq!(station::stepped_call("gen@2 techno seed=42 length=long", SeedStep::By(1)).unwrap(), "gen@2 techno seed=43 length=long");
    // a word seed is the generator's hash of it, and track 0 keeps the word
    let w = station::read("station {\n  from gen@2 house seed=night\n}\n").unwrap();
    assert_eq!(w.seeds(1).unwrap()[0], station::seed_value("night"));
    assert!(w.track(0).unwrap().text.contains("seed=night"));
    assert!(w.track(1).unwrap().text.contains(&format!("seed={}", station::seed_value("night") + 1)));
}

#[test]
fn retuning_a_knob_keeps_position_n_and_a_small_turn_keeps_the_walk() {
    let s = radio();
    let n = 6;
    let here = s.track(n).unwrap();
    for (knob, v) in [("tempo", "+1"), ("tempo", "-1"), ("key", "+1"), ("key", "-1"), ("forms", "+1"), ("tempo", "140"), ("key", "dm"), ("forms", "fugue seeded")] {
        let t = s.retune(knob, v).unwrap();
        assert_ne!(t.cid(), s.cid(), "{knob} {v} is another station");
        let there = t.track(n).unwrap();
        assert_eq!(there.n, n, "{knob} {v} keeps the position");
        // the walk is the generator calls': tempo, key and forms keep it, so the same seed plays at n
        assert_eq!(there.seed, here.seed, "{knob} {v} keeps the walk");
        assert_ne!(there.cid, here.cid, "{knob} {v} changes the track");
    }
    assert_eq!(s.retune("tempo", "+1").unwrap().tempo, Some(130.0));
    assert_eq!(s.retune("key", "+1").unwrap().key.as_deref(), Some("em"));
    assert_eq!(s.retune("key", "-1").unwrap().key.as_deref(), Some("dm"));
    assert_eq!(s.retune("forms", "+1").unwrap().forms, ["passacaglia", "rondo", "fugue", "seeded"]);
    // seed and step move the walk, still at n
    let seeded = s.retune("seed", "+1").unwrap();
    assert_eq!(seeded.seed, "43");
    assert_eq!(seeded.track(n).unwrap().n, n);
    assert_eq!(s.retune("step", "+1").unwrap().step, SeedStep::By(1));
    assert_eq!(s.retune("step", "~").unwrap().step, SeedStep::Hash);
    assert_eq!(s.retune("length", "+1").unwrap().length.as_deref(), Some("long"));
    // a genre is named, never notched (its neighbourhoods are phase 2)
    assert_eq!(s.retune("genre", "house").unwrap().genre, "house");
    assert!(s.retune("genre", "+1").is_err());
    // a seed or genre the language would not read back as one word is refused
    for bad in ["1-2", "-5", "a-", "1 2"] {
        assert!(s.retune("seed", bad).is_err(), "seed {bad:?}");
    }
    // a tempo notch and its way back are the same tempo, not a float's ulp away
    let odd = station::read("tempo 63.1\nstation {\n  from gen@2 techno seed=42\n}\n").unwrap();
    assert_eq!(odd.retune("tempo", "+1").unwrap().retune("tempo", "-1").unwrap().tempo, Some(63.1));
    // the key notch reads a key however its mode is spelt
    for (k, up) in [("amin", "em"), ("cmaj", "g"), ("c:dorian", "g:dorian"), ("f#m", "c#m"), ("bb", "f")] {
        let t = station::read(&format!("key {k}\nstation {{\n  from gen@2 techno seed=42\n}}\n")).unwrap();
        assert_eq!(t.retune("key", "+1").unwrap().key.as_deref(), Some(up), "{k}");
    }
    // a genre is a word: nothing else can write itself into the tracks' calls
    for bad in ["techno section=drop", "techno\narrange from z", "", "Techno!", "9am"] {
        assert!(s.retune("genre", bad).is_err(), "{bad:?}");
    }
    assert!(s.retune("tempo", "400").is_err());
    assert!(s.retune("volume", "+1").is_err());
    // retuning back is the station it was
    assert_eq!(s.retune("tempo", "+1").unwrap().retune("tempo", "-1").unwrap(), s);
}

#[test]
fn a_lock_holds_a_knob_and_the_neighbours_are_one_notch_along_the_others() {
    let s = radio();
    let all = s.neighbours(&[]);
    let knobs: Vec<&str> = all.iter().map(|(k, _, _)| k.as_str()).collect();
    for k in ["seed", "step", "forms", "tempo", "key", "length"] {
        assert!(knobs.contains(&k), "{k} in {knobs:?}");
    }
    assert!(!knobs.contains(&"genre"), "a genre has no notch");
    let locked = s.neighbours(&["tempo".to_string(), "key".to_string()]);
    for (k, _, t) in &locked {
        assert!(k != "tempo" && k != "key");
        assert_eq!(t.tempo, s.tempo, "a locked tempo holds");
        assert_eq!(t.key, s.key, "a locked key holds");
    }
    assert_eq!(locked.len(), all.len() - 4);
    // each neighbour differs from the station by its one knob
    for (k, _, t) in &all {
        let mut back = t.clone();
        match k.as_str() {
            "seed" => back.seed = s.seed.clone(),
            "step" => back.step = s.step,
            "forms" => back.forms = s.forms.clone(),
            "tempo" => back.tempo = s.tempo,
            "key" => back.key = s.key.clone(),
            "length" => back.length = s.length.clone(),
            _ => unreachable!(),
        }
        assert_eq!(back, s, "{k}");
    }
}

#[test]
fn the_segue_is_a_cross_of_the_two_seeds_with_its_own_cid() {
    let s = radio();
    let t = s.tracks(4, 2).unwrap();
    let sg = s.segue(4).unwrap();
    assert_eq!((sg.from, sg.to), (4, 5));
    assert!(sg.text.contains(&format!("arrange from template(cross, {}, a=gen@2 techno seed={}, b=gen@2 techno seed={})", sg.phrase, t[0].seed, t[1].seed)), "{}", sg.text);
    assert!(sg.cid.starts_with('z'));
    assert_ne!(sg.cid, t[0].cid);
    assert_ne!(sg.cid, t[1].cid);
    assert_eq!(s.segue(4).unwrap(), sg, "the same segue twice");
    // a sparse genre crosses its melodies
    let amb = station::read("station {\n  from gen@2 ambient seed=7\n  forms cross rondo\n}\n").unwrap();
    assert!(amb.segue(0).unwrap().text.contains("seed=7 part=melody, b=gen@2 ambient seed=8 part=melody)"));
    assert!(amb.track(1).unwrap().text.contains("part=melody)"));
}

#[test]
fn check_says_what_is_wrong_with_a_station() {
    let e = errors("station {\n  next seed=~\n}\n");
    assert!(e.iter().any(|m| m.contains("says where it starts")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42 part=bass\n}\n");
    assert!(e.iter().any(|m| m.contains("whole track")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42 bpm=130\n}\n");
    assert!(e.iter().any(|m| m.contains("`tempo` and `key` lines")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42\n  forms seeded rondoo\n}\n");
    assert!(e.iter().any(|m| m.contains("`rondoo` is no form")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42\n}\ndeck A: kick x ~ x ~\n");
    assert!(e.iter().any(|m| m.contains("holds the station, its `tempo` and its `key`")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42\n  next seed=+0\n}\n");
    assert!(e.iter().any(|m| m.contains("steps nowhere")), "{e:?}");
    let e = errors("station {\n  from gen@2 techno seed=42\n  next seed=3\n}\n");
    assert!(e.iter().any(|m| m.contains("`seed=+N`")), "{e:?}");
    let e = errors("#! stations: v2\nstation {\n  from gen@2 techno seed=42\n}\n");
    assert!(e.iter().any(|m| m.contains("stations v2 is not here")), "{e:?}");
    assert!(errors("#! stations: v1\nstation {\n  from gen@2 techno seed=42\n}\n").is_empty());
}

#[test]
fn a_set_steps_a_generator_call_with_next_and_prints_it_as_written() {
    let src = "set {\n  play gen@2 techno seed=42\n  mix 16 bars with crossfade\n  next seed=+1\n  next seed=~ until 2m\n}\n";
    let p = djlive::parse::parse(src);
    assert!(p.diagnostics.iter().all(|d| !d.is_error()), "{:?}", p.diagnostics);
    assert_eq!(djlive::print::canonical(&p.program), src, "the call is written bare and next as written");
    let other = djlive::parse::parse("set {\n  play gen@2 techno seed=42\n  mix 16 bars with crossfade\n  next seed=+2\n  next seed=~ until 2m\n}\n");
    assert_ne!(djlive::compile::program_cid(&djlive::print::canonical(&p.program)), djlive::compile::program_cid(&djlive::print::canonical(&other.program)), "the step is in the CID");
}

#[test]
fn the_station_words_name_lets_where_no_station_reads_them() {
    let src = "let forms = x ~ x ~\nlet station = ~ x ~ x\ndeck A: kick forms | snare station\n";
    let c = djlive::compile::check(src);
    assert!(c.diagnostics.iter().all(|d| !d.is_error()), "{:?}", c.diagnostics);
}
