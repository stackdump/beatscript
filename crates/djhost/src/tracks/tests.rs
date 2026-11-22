use super::*;

fn spec() -> TrackSpec {
    TrackSpec::new("dnb", 82819.0, None)
}

fn track(name: &str) -> Track {
    Track::new(name, &spec(), Rights::new(Some("did:key:zAlice")))
}

fn tmp(tag: &str) -> TrackStore {
    let d = std::env::temp_dir().join(format!("djhost-tracks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    TrackStore::new(d)
}

#[test]
fn a_new_track_is_cc0_and_public_and_says_so_in_its_json() {
    let t = Track::new("x", &spec(), Rights::default());
    assert_eq!(t.rights.license, License::Cc0);
    assert_eq!(t.rights.visibility, Visibility::Public);
    let c = t.canonical();
    assert!(c.contains(r#""license":"CC0-1.0""#) && c.contains(r#""visibility":"public""#), "{c}");
    assert!(c.contains(r#""type":"djrust.track""#));
}

#[test]
fn a_track_round_trips_and_keeps_its_cid() {
    let (t, _) = track("Night Bus").fork(None, Some("bob"));
    let back = Track::from_json(&JsonValue::parse(&t.canonical()).unwrap()).unwrap();
    assert_eq!(back, t);
    assert_eq!(back.cid(), t.cid());
    assert!(t.cid().starts_with('z'));
}

#[test]
fn the_cid_changes_when_the_license_or_visibility_changes_and_the_music_cid_does_not() {
    let a = track("n");
    let mut seen = std::collections::BTreeSet::new();
    for l in License::ALL {
        for v in Visibility::ALL {
            let mut t = a.clone();
            t.rights.license = l;
            t.rights.visibility = v;
            assert!(seen.insert(t.cid()), "{l:?} {v:?} has a CID of its own");
            assert_eq!(t.music_cid(), a.music_cid());
        }
    }
    assert_eq!(a.music_cid(), djengine::cid::share_cid(&spec().payload()), "the music is the deck's share");
}

#[test]
fn a_private_track_refuses_sharing() {
    let mut t = track("p");
    t.rights.visibility = Visibility::Private;
    let e = t.share().unwrap_err();
    assert!(e.contains("private") && e.contains("never shared"), "{e}");
    t.rights.visibility = Visibility::Public;
    let s = t.share().unwrap();
    assert_eq!(s.get("cid").and_then(JsonValue::as_str), Some(t.cid().as_str()));
}

#[test]
fn the_license_survives_a_fork_and_all_rights_reserved_warns() {
    for l in License::ALL {
        let mut p = track("parent");
        p.rights.license = l;
        let (c, w) = p.fork(Some("child"), Some("did:key:zBob"));
        assert_eq!(c.rights.license, l);
        assert_eq!(c.rights.forked_from.as_deref(), Some(p.cid().as_str()));
        assert_eq!(c.name, "child");
        assert_eq!(c.spec, p.spec);
        assert_ne!(c.cid(), p.cid());
        assert_eq!(!w.is_empty(), l == License::AllRightsReserved, "{l:?}: {w:?}");
    }
}

#[test]
fn a_fork_of_a_cc_by_track_credits_its_author_in_the_object() {
    let mut p = track("Night Bus");
    p.rights.license = License::CcBy4;
    let (c, w) = p.fork(None, Some("did:key:zBob"));
    assert!(w.is_empty());
    assert_eq!(c.name, "Night Bus (remix)");
    let credit = &c.rights.credits[0];
    assert_eq!(credit.cid, p.cid());
    assert_eq!(credit.author.as_deref(), Some("did:key:zAlice"));
    let json = c.canonical();
    assert!(json.contains(r#""credits":[{"author":"did:key:zAlice","cid":""#), "{json}");
    let shared = c.share().unwrap();
    assert!(shared.get("credit").and_then(JsonValue::as_str).unwrap().contains("did:key:zAlice"));
    // a fork of the fork still credits Alice
    let (g, _) = c.fork(None, Some("carol"));
    assert!(g.rights.credits.iter().any(|x| x.author.as_deref() == Some("did:key:zAlice")));
}

#[test]
fn the_store_saves_lists_relicenses_forks_and_deletes() {
    let st = tmp("store");
    let t = track("Night Bus");
    let cid = st.save(&t).unwrap();
    assert_eq!(cid, t.cid());
    assert_eq!(st.list().len(), 1);
    assert_eq!(st.load("night bus").unwrap(), t);

    let t2 = st.set_rights(&cid, Some(License::CcBy4), None).unwrap();
    assert_ne!(t2.cid(), cid, "a new license is a new object");
    let l = st.list();
    assert_eq!(l.len(), 1, "the old file is dropped");
    assert_eq!(l[0].rights.license, License::CcBy4);

    let (f, _) = st.fork(&t2.cid(), None, Some("bob")).unwrap();
    assert_eq!(st.list().len(), 2);
    assert_eq!(st.load(&f.cid()).unwrap().rights.credits[0].author.as_deref(), Some("did:key:zAlice"));

    st.delete(&f.cid()).unwrap();
    assert_eq!(st.list().len(), 1);
    let _ = std::fs::remove_dir_all(&st.directory);
}

#[test]
fn bad_objects_say_why() {
    let bad = |t: &str| Track::from_json(&JsonValue::parse(t).unwrap()).unwrap_err();
    assert!(bad(r#"{"type":"djrust.setlist"}"#).contains("not a track"));
    assert!(bad(r#"{"type":"djrust.track","name":"x"}"#).contains("spec"));
    assert!(bad(r#"{"type":"djrust.track","name":"x","spec":{"genre":"dnb","seed":1},"license":"unlicensed"}"#).contains("license"));
}
