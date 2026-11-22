use super::recipe::{energy_curve, parse_key, Bank, BankInfo};
use super::store::SetStore;
use super::*;
use crate::stores::temp_path;

/// Loop sets that need no building: genre g, seed s has 6 groups, a tempo and a key from s.
struct Fake;

impl BankInfo for Fake {
    fn bank(&self, spec: &str) -> Option<Bank> {
        let (g, s) = spec.split_once(':')?;
        let s: u32 = s.parse().ok()?;
        if g == "nope" {
            return None;
        }
        let keys = ["A minor", "C major", "E minor", "D minor", "F# minor", "G major"];
        let base = if g == "dnb" { 174.0 } else { 160.0 };
        Some(Bank { groups: 6, bpm: base + (s % 5) as f64, key: keys[(s % 6) as usize].into() })
    }
}

fn recipe(genres: &[&str], n: u32, seed: u32) -> Recipe {
    Recipe { genres: genres.iter().map(|g| g.to_string()).collect(), segments: n, seed, ..Recipe::default() }
}

#[test]
fn a_set_round_trips_through_its_json_and_keeps_its_cid() {
    let mut s = SetList::new("Friday");
    let mut seg = Segment::new(Source::Pair { spec: "dnb:82819".into(), group: 3 }, 16);
    seg.transition = Transition { kind: TransitionKind::Move(MoveKind::EchoOut), bars: 2 };
    seg.inserts = vec![InsertAuto::new(InsertKind::FilterSweep, 4, 0.14), InsertAuto::new(InsertKind::DelayThrow, 2, 0.5)];
    seg.tag = Tag { energy: Some(4), genre: Some("dnb".into()) };
    s.segments.push(seg);
    s.segments.push(Segment::new(Source::Net { spec: "house:42".into() }, 32));
    s.segments.push(Segment::new(Source::Track { cid: "zQmExample".into() }, 8));
    s.note = "warm-up".into();

    let text = s.to_json().js_stringified();
    let back = SetList::from_json(&JsonValue::parse(&text).unwrap()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.cid(), s.cid());
    assert_eq!(back.canonical(), s.canonical());
    assert!(s.cid().starts_with('z'));

    // the CID is of the content: a change moves it, putting it back moves it back
    let mut t = s.clone();
    t.set_bars(0, 17).unwrap();
    assert_ne!(t.cid(), s.cid());
    t.set_bars(0, 16).unwrap();
    assert_eq!(t.cid(), s.cid());
    // key order in the file does not matter: the canonical text sorts
    let shuffled = r#"{"segments":[],"name":"x","type":"djrust.setlist","format":1}"#;
    let tidy = r#"{"format":1,"name":"x","segments":[],"type":"djrust.setlist"}"#;
    assert_eq!(
        SetList::from_json(&JsonValue::parse(shuffled).unwrap()).unwrap().cid(),
        SetList::from_json(&JsonValue::parse(tidy).unwrap()).unwrap().cid()
    );
}

/// A pinned CID: it only changes when the format does (which is a decision, not an accident).
#[test]
fn the_cid_of_a_known_set_is_pinned() {
    let mut s = SetList::new("pin");
    s.segments.push(Segment::new(Source::Pair { spec: "dnb:1".into(), group: 1 }, 16));
    assert_eq!(
        s.canonical(),
        r#"{"format":1,"license":"CC0-1.0","name":"pin","segments":[{"bars":16,"source":{"group":1,"kind":"pair","spec":"dnb:1"},"transition":{"bars":16,"kind":"phrase_sync"}}],"type":"djrust.setlist","visibility":"public"}"#
    );
    assert_eq!(s.cid(), djengine::cid::share_cid_canonical(&s.canonical()));
}

/// A set saved before sets had rights (no license key) keeps its CID and reads as CC0 / public.
#[test]
fn a_set_saved_before_rights_keeps_its_cid_and_reads_as_cc0_public() {
    let old = r#"{"format":1,"name":"pin","segments":[{"bars":16,"source":{"group":1,"kind":"pair","spec":"dnb:1"},"transition":{"bars":16,"kind":"phrase_sync"}}],"type":"djrust.setlist"}"#;
    let s = SetList::from_json(&JsonValue::parse(old).unwrap()).unwrap();
    assert_eq!(s.canonical(), old);
    assert_eq!(s.cid(), djengine::cid::share_cid_canonical(old));
    assert_eq!(s.rights(), djengine::rights::Rights::default());
}

#[test]
fn a_sets_cid_changes_with_its_license_and_visibility_and_a_fork_keeps_them() {
    use djengine::rights::{License, Visibility};
    let mut s = SetList::new("lic");
    s.segments.push(Segment::new(Source::Pair { spec: "dnb:1".into(), group: 1 }, 16));
    let cc0 = s.cid();
    s.set_license(License::CcBy4);
    s.set_author(Some("did:key:zAlice"));
    let by = s.cid();
    assert_ne!(by, cc0);
    let mut p = s.clone();
    p.set_visibility(Visibility::Private);
    assert_ne!(p.cid(), by);
    assert!(p.rights().check_shareable().is_err());

    let (f, w) = s.fork("lic remix", Some("bob"));
    assert!(w.is_empty());
    assert_eq!(f.rights().license, License::CcBy4);
    assert_eq!(f.rights().forked_from.as_deref(), Some(by.as_str()));
    assert_eq!(f.rights().credits[0].author.as_deref(), Some("did:key:zAlice"));
    let back = SetList::from_json(&f.to_json()).unwrap();
    assert_eq!(back, f);

    s.set_license(License::AllRightsReserved);
    assert_eq!(s.fork("x", None).1.len(), 1, "forking an all-rights-reserved set warns");
}

#[test]
fn bad_json_says_which_segment_and_what() {
    let bad = |t: &str| SetList::from_json(&JsonValue::parse(t).unwrap()).unwrap_err();
    assert!(bad(r#"{"name":"x"}"#).contains("segments"));
    assert!(bad(r#"{"name":"","segments":[]}"#).contains("name"));
    assert!(bad(r#"{"name":"x","segments":[{"source":{"kind":"pair","spec":"dnb:1","group":0}}]}"#).contains("segment 1"));
    assert!(bad(r#"{"name":"x","segments":[{"source":{"kind":"net","spec":"a"},"bars":2}]}"#).contains("bars"));
    assert!(bad(r#"{"name":"x","segments":[{"source":{"kind":"net","spec":"a"},"transition":{"kind":"explode"}}]}"#).contains("one of"));
    assert!(bad(r#"{"type":"other","name":"x","segments":[]}"#).contains("not a set list"));
}

#[test]
fn sources_parse_from_the_words_a_person_types() {
    assert_eq!(Source::parse("dnb:82819#3").unwrap(), Source::Pair { spec: "dnb:82819".into(), group: 3 });
    assert_eq!(Source::parse("DNB:82819 3").unwrap(), Source::Pair { spec: "dnb:82819".into(), group: 3 });
    assert_eq!(Source::parse("pair:dnb:82819:3").unwrap(), Source::Pair { spec: "dnb:82819".into(), group: 3 });
    assert_eq!(Source::parse("net:house:42").unwrap(), Source::Net { spec: "house:42".into() });
    assert_eq!(Source::parse("track:zQm1").unwrap(), Source::Track { cid: "zQm1".into() });
    assert!(Source::parse("dnb").is_err());
}

#[test]
fn the_edits_insert_move_remove_and_change() {
    let mut s = SetList::new("e");
    let p = |t: &str| JsonValue::parse(t).unwrap();
    for g in 1..=3 {
        edit(&mut s, "insert", &p(&format!(r#"{{"source":"dnb:1#{g}","bars":8}}"#))).unwrap();
    }
    assert_eq!(s.segments.len(), 3);
    edit(&mut s, "insert", &p(r#"{"at":1,"source":"net:house:9","bars":32,"transition":"echo_out","transition_bars":2}"#)).unwrap();
    assert_eq!(s.segments[0].source, Source::Net { spec: "house:9".into() });
    assert_eq!(s.segments[0].transition, Transition { kind: TransitionKind::Move(MoveKind::EchoOut), bars: 2 });
    edit(&mut s, "move", &p(r#"{"index":1,"to":4}"#)).unwrap();
    assert_eq!(s.segments[3].source, Source::Net { spec: "house:9".into() });
    edit(&mut s, "bars", &p(r#"{"index":1,"bars":64}"#)).unwrap();
    assert_eq!(s.segments[0].bars, 64);
    edit(&mut s, "transition", &p(r#"{"index":2,"transition":"bass_swap"}"#)).unwrap();
    assert_eq!(s.segments[1].transition.kind, TransitionKind::Move(MoveKind::BassSwap));
    assert_eq!(s.segments[1].transition.bars, MoveKind::BassSwap.default_bars());
    edit(&mut s, "inserts", &p(r#"{"index":2,"inserts":[{"kind":"delay_throw","bars":2,"amount":0.4}]}"#)).unwrap();
    assert_eq!(s.segments[1].inserts, vec![InsertAuto::new(InsertKind::DelayThrow, 2, 0.4)]);
    edit(&mut s, "tag", &p(r#"{"index":3,"tag":{"energy":5,"genre":"dnb"}}"#)).unwrap();
    assert_eq!(s.segments[2].tag.energy, Some(5));
    edit(&mut s, "remove", &p(r#"{"index":1}"#)).unwrap();
    assert_eq!(s.segments.len(), 3);
    edit(&mut s, "rename", &p(r#"{"name":"Renamed"}"#)).unwrap();
    assert_eq!(s.name, "Renamed");
    // a bad edit changes nothing
    let before = s.clone();
    assert!(edit(&mut s, "bars", &p(r#"{"index":1,"bars":1}"#)).is_err());
    assert!(edit(&mut s, "remove", &p(r#"{"index":9}"#)).is_err());
    assert!(edit(&mut s, "move", &p(r#"{"index":1,"to":9}"#)).is_err());
    assert!(edit(&mut s, "nope", &p("{}")).unwrap_err().contains("one of"));
    assert!(edit(&mut s, "insert", &p(r#"{"source":"dnb","bars":8}"#)).is_err());
    assert_eq!(s, before);
}

#[test]
fn a_recipe_makes_the_same_set_every_time_from_its_seed() {
    let r = recipe(&["dnb", "jungle"], 12, 7);
    let a = generate(&r, &Fake).unwrap();
    let b = generate(&r, &Fake).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.cid(), b.cid());
    assert_ne!(a.cid(), generate(&recipe(&["dnb", "jungle"], 12, 8), &Fake).unwrap().cid());
    assert_eq!(a.segments.len(), 12);
    // dnb then jungle: six of each, in that order
    let genres: Vec<_> = a.segments.iter().map(|s| s.tag.genre.clone().unwrap()).collect();
    assert_eq!(genres[..6], ["dnb"; 6]);
    assert_eq!(genres[6..], ["jungle"; 6]);
    // the name says what it is, and 16 bars each by default
    assert!(a.name.contains("dnb→jungle") && a.segments.iter().all(|s| s.bars == 16));
}

#[test]
fn a_generated_set_uses_only_whole_a_b_pairs_and_each_group_once() {
    let a = generate(&recipe(&["dnb"], 12, 3), &Fake).unwrap();
    let mut seen = std::collections::HashSet::new();
    for s in &a.segments {
        let Source::Pair { spec, group } = &s.source else { panic!("a generated set is pairs") };
        assert!((1..=6).contains(group));
        assert!(seen.insert((spec.clone(), *group)), "{spec} #{group} twice");
    }
    // 12 groups of 6-group sets: two different sets
    let specs: std::collections::HashSet<_> = a.segments.iter().map(|s| s.source.text().split(" #").next().unwrap().to_string()).collect();
    assert_eq!(specs.len(), 2);
    a.check_pairs(&Fake).unwrap();
    let mut bad = a.clone();
    bad.segments[0].source = Source::Pair { spec: "dnb:1".into(), group: 7 };
    assert!(bad.check_pairs(&Fake).unwrap_err().contains("groups 1-6"));
}

#[test]
fn the_next_loop_set_is_the_best_match_to_the_one_before() {
    // the second set of a recipe is picked among candidates for key and tempo
    let a = generate(&recipe(&["dnb"], 12, 5), &Fake).unwrap();
    let spec_of = |i: usize| match &a.segments[i].source {
        Source::Pair { spec, .. } => spec.clone(),
        _ => unreachable!(),
    };
    let (x, y) = (Fake.bank(&spec_of(0)).unwrap(), Fake.bank(&spec_of(6)).unwrap());
    let kx = parse_key(&x.key).unwrap();
    let ky = parse_key(&y.key).unwrap();
    // 6 candidates over 6 keys: at least one mixes, and the pick has to be one that does
    assert!(crate::perform::harmonic(kx, ky).mixes(), "{} -> {}", x.key, y.key);
}

#[test]
fn varied_transitions_never_repeat_and_the_set_ends_on_an_echo_out() {
    let a = generate(&recipe(&["techno"], 12, 11), &Fake).unwrap();
    for w in a.segments.windows(2) {
        assert_ne!(w[0].transition.kind, w[1].transition.kind, "{} then {}", w[0].transition.text(), w[1].transition.text());
    }
    assert_eq!(a.segments.last().unwrap().transition.kind, TransitionKind::Move(MoveKind::EchoOut));
    // not varying: one transition throughout (the last still an echo out)
    let mut r = recipe(&["techno"], 6, 11);
    r.vary = false;
    r.transition = TransitionKind::Cut;
    let b = generate(&r, &Fake).unwrap();
    assert!(b.segments[..5].iter().all(|s| s.transition.kind == TransitionKind::Cut));
    // inserts off: none, and the same transitions as with them on
    let mut r2 = recipe(&["techno"], 12, 11);
    r2.inserts = false;
    let c = generate(&r2, &Fake).unwrap();
    assert!(c.segments.iter().all(|s| s.inserts.is_empty()));
    assert!(a.segments.iter().zip(&c.segments).all(|(x, y)| x.transition == y.transition && x.source == y.source));
    // the energy goes up and comes down
    let e: Vec<u8> = (0..12).map(|i| energy_curve(i, 12)).collect();
    assert!(e[0] < e[5] && e[11] < e[5], "{e:?}");
}

#[test]
fn a_recipe_that_cannot_be_made_says_why() {
    assert!(generate(&recipe(&["nope"], 4, 1), &Fake).unwrap_err().contains("nope"));
    assert!(generate(&recipe(&["dnb"], 0, 1), &Fake).is_err());
    assert!(generate(&Recipe { genres: vec![], ..Recipe::default() }, &Fake).is_err());
    assert_eq!(Recipe::parse_genres("dnb then jungle"), ["dnb", "jungle"]);
    assert_eq!(Recipe::parse_genres("DnB, House > techno"), ["dnb", "house", "techno"]);
    assert_eq!(parse_key("F# minor"), Some(crate::perform::Key { pc: 6, minor: true }));
    assert_eq!(parse_key("Bb major"), Some(crate::perform::Key { pc: 10, minor: false }));
    assert_eq!(parse_key("nonsense"), None);
}

#[test]
fn the_store_saves_lists_loads_duplicates_edits_and_deletes() {
    let dir = temp_path("setlists");
    let st = SetStore::new(&dir);
    assert!(st.list().is_empty());
    let a = generate(&recipe(&["dnb"], 6, 2), &Fake).unwrap();
    let cid = st.save(&a).unwrap();
    assert_eq!(cid, a.cid());
    assert_eq!(st.save(&a).unwrap(), cid, "the same set is the same file");
    assert_eq!(st.list().len(), 1);
    // by cid, by a part of it, by name
    assert_eq!(st.load(&cid).unwrap(), a);
    assert_eq!(st.load(&cid[cid.len() - 8..]).unwrap(), a);
    assert_eq!(st.load("6×16").unwrap(), a);
    assert!(st.load("zzz-not-there").unwrap_err().contains("no saved set"));
    // duplicate: another set, the first is untouched
    let (copy, ccid) = st.duplicate(&cid, None).unwrap();
    assert_ne!(ccid, cid);
    assert_eq!(copy.segments, a.segments);
    assert_eq!(st.list().len(), 2);
    assert!(st.resolve("dnb").unwrap_err().contains("matches 2"), "a name that is two sets is ambiguous");
    assert_eq!(st.resolve("copy").unwrap().cid, ccid);
    // edit = replace: the new content has a new cid, the old file goes
    let mut e = st.load("copy").unwrap();
    e.set_bars(0, 32).unwrap();
    let ncid = st.replace(&ccid, &e).unwrap();
    assert_ne!(ncid, ccid);
    assert_eq!(st.list().len(), 2);
    assert!(st.load(&ccid).is_err());
    assert_eq!(st.load(&ncid).unwrap().segments[0].bars, 32);
    // delete
    st.delete(&ncid).unwrap();
    assert_eq!(st.list().len(), 1);
    // a file that was edited by hand (its name is no longer its CID) is not listed
    let bad = dir.join("zHandEdited.json");
    std::fs::write(&bad, a.to_json().js_stringified()).unwrap();
    assert_eq!(st.list().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
