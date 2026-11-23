//! The rights block (docs/LICENSING.md): defaults, parsing, the private gate, forks and credits.

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{Credit, License, Rights, Visibility, PRIVATE_REFUSAL};

fn block(r: &Rights) -> JsonValue {
    let mut o = JsonObject::new();
    r.write_into(&mut o);
    JsonValue::Object(o)
}

#[test]
fn the_default_is_cc0_and_public() {
    let r = Rights::default();
    assert_eq!(r.license, License::Cc0);
    assert_eq!(r.visibility, Visibility::Public);
    assert_eq!(block(&r).canonical_js(), r#"{"license":"CC0-1.0","visibility":"public"}"#);
    // absent fields read as the defaults
    assert_eq!(Rights::from_json(&JsonValue::parse("{}").unwrap()).unwrap(), r);
}

#[test]
fn the_three_licenses_round_trip_by_id() {
    for l in License::ALL {
        let r = Rights { license: l, ..Rights::default() };
        assert_eq!(Rights::from_json(&block(&r)).unwrap().license, l);
        assert_eq!(License::parse(l.id()), Some(l));
    }
    assert_eq!(License::ids(), ["CC0-1.0", "CC-BY-4.0", "all-rights-reserved"]);
    assert_eq!(License::parse("cc by"), Some(License::CcBy4));
    assert_eq!(License::parse("arr"), Some(License::AllRightsReserved));
    assert_eq!(License::parse("https://creativecommons.org/licenses/by/4.0/"), Some(License::CcBy4));
}

#[test]
fn unknown_or_loose_ids_in_an_object_are_refused_and_unlicensed_is_never_a_license() {
    let bad = |t: &str| Rights::from_json(&JsonValue::parse(t).unwrap()).unwrap_err();
    assert!(bad(r#"{"license":"unlicensed"}"#).contains("CC0-1.0, CC-BY-4.0, all-rights-reserved"));
    assert!(bad(r#"{"license":"cc0"}"#).contains("license"), "the object takes the exact id only");
    assert!(bad(r#"{"visibility":"hidden"}"#).contains("public or private"));
    assert_eq!(License::parse("unlicensed"), None);
    assert_eq!(License::parse("Unlicense"), None);
    for l in License::ALL {
        assert!(!l.id().to_lowercase().contains("unlicens") && !l.label().to_lowercase().contains("unlicens"));
    }
}

#[test]
fn private_refuses_sharing_and_public_allows_it() {
    assert!(Rights::default().check_shareable().is_ok());
    let p = Rights { visibility: Visibility::Private, ..Rights::default() };
    assert_eq!(p.check_shareable().unwrap_err(), PRIVATE_REFUSAL);
}

#[test]
fn a_fork_keeps_the_license_and_points_at_its_parent() {
    for l in License::ALL {
        let parent = Rights { license: l, author: Some("did:key:zParent".into()), ..Rights::default() };
        let f = parent.fork("zQmParentCid0000", Some("Night Bus"), Some("did:key:zChild"));
        assert_eq!(f.rights.license, l, "the license survives the fork");
        assert_eq!(f.rights.forked_from.as_deref(), Some("zQmParentCid0000"));
        assert_eq!(f.rights.author.as_deref(), Some("did:key:zChild"));
    }
}

#[test]
fn forking_all_rights_reserved_warns_and_the_others_do_not() {
    let arr = Rights { license: License::AllRightsReserved, author: Some("alice".into()), ..Rights::default() };
    let f = arr.fork("zQmArr000000000", None, None);
    assert_eq!(f.warnings.len(), 1);
    assert!(f.warnings[0].contains("all rights reserved") && f.warnings[0].contains("alice"));
    assert!(Rights::default().fork("zQmCc0000000000", None, None).warnings.is_empty());
    let by = Rights { license: License::CcBy4, author: Some("alice".into()), ..Rights::default() };
    assert!(by.fork("zQmBy0000000000", None, None).warnings.is_empty());
}

#[test]
fn a_fork_of_cc_by_credits_the_parents_author_once() {
    let parent = Rights { license: License::CcBy4, author: Some("did:key:zAlice".into()), ..Rights::default() };
    let f = parent.fork("zQmAliceTrack01", Some("Night Bus"), Some("did:key:zBob"));
    assert_eq!(f.rights.credits.len(), 1, "the lineage credit is the attribution: one credit, not two");
    let c = &f.rights.credits[0];
    assert_eq!((c.cid.as_str(), c.author.as_deref(), c.license), ("zQmAliceTrack01", Some("did:key:zAlice"), License::CcBy4));
    assert_eq!(c.name.as_deref(), Some("forked from: Night Bus"));
    assert!(c.is_fork_parent());
    assert_eq!(c.work_name(), Some("Night Bus"));
    assert!(f.rights.credit_line().contains("did:key:zAlice"));
    assert!(f.rights.credit_line().contains("forked from “Night Bus”"), "{}", f.rights.credit_line());
    // the credit is in the JSON and reads back
    assert_eq!(Rights::from_json(&block(&f.rights)).unwrap(), f.rights);
}

#[test]
fn a_fork_of_a_cc0_parent_keeps_its_author_in_the_lineage_and_names_the_forker() {
    let parent = Rights { author: Some("did:key:zAlice".into()), ..Rights::default() };
    let f = parent.fork("zQmAliceTrack01", Some("Night Bus"), Some("did:key:zBob"));
    assert!(f.warnings.is_empty());
    assert_eq!(f.rights.author.as_deref(), Some("did:key:zBob"), "the fork's author is the forker");
    assert_eq!(f.rights.license, License::Cc0);
    assert_eq!(f.rights.credits.len(), 1);
    let c = &f.rights.credits[0];
    assert_eq!((c.cid.as_str(), c.author.as_deref(), c.license), ("zQmAliceTrack01", Some("did:key:zAlice"), License::Cc0));
    assert!(c.is_fork_parent() && c.work_name() == Some("Night Bus"));
    assert_eq!(c.text(), "by did:key:zAlice · forked from “Night Bus” (z…eTrack01), CC0 1.0");
    assert_eq!(Rights::from_json(&block(&f.rights)).unwrap(), f.rights);
    // an all-rights-reserved parent's author is kept too (and the fork warns)
    let arr = Rights { license: License::AllRightsReserved, ..parent.clone() };
    let f = arr.fork("zQmArr000000000", None, Some("did:key:zBob"));
    assert_eq!((f.rights.credits.len(), f.rights.credits[0].license), (1, License::AllRightsReserved));
    assert_eq!(f.rights.credits[0].name.as_deref(), Some("forked from"));
    assert_eq!(f.rights.credits[0].work_name(), None);
    assert_eq!(f.warnings.len(), 1);
}

#[test]
fn a_parent_that_names_no_author_adds_no_credit_unless_it_is_cc_by() {
    for l in [License::Cc0, License::AllRightsReserved] {
        let anon = Rights { license: l, ..Rights::default() };
        let f = anon.fork("zQmAnon00000000", Some("x"), Some("bob"));
        assert!(f.rights.credits.is_empty(), "{l:?}: {:?}", f.rights.credits);
        assert_eq!(f.rights.forked_from.as_deref(), Some("zQmAnon00000000"), "the parent is still named by forkedFrom");
    }
}

#[test]
fn a_fork_of_a_fork_credits_both_ancestors_down_the_chain() {
    let alice = Rights { author: Some("alice".into()), ..Rights::default() };
    let bob = alice.fork("zQmAlice0000000", Some("A"), Some("bob")).rights;
    let carol = bob.fork("zQmBob000000000", Some("B"), Some("carol")).rights;
    assert_eq!(carol.author.as_deref(), Some("carol"));
    let chain: Vec<_> = carol.credits.iter().map(|c| (c.cid.as_str(), c.author.as_deref().unwrap(), c.work_name().unwrap())).collect();
    assert_eq!(chain, [("zQmAlice0000000", "alice", "A"), ("zQmBob000000000", "bob", "B")]);
    assert!(carol.credits.iter().all(|c| c.is_fork_parent()));
    // a program's fork names its parent by the program's CID, which an unedited fork shares: the
    // two steps are still two credits, told apart by their authors
    let bob2 = alice.fork("zQmProgram00000", None, Some("bob")).rights;
    let carol2 = bob2.fork("zQmProgram00000", None, Some("carol")).rights;
    let who: Vec<_> = carol2.credits.iter().map(|c| c.author.as_deref().unwrap()).collect();
    assert_eq!(who, ["alice", "bob"]);
    assert!(carol2.credits.iter().all(|c| c.cid == "zQmProgram00000"));
}

#[test]
fn credits_are_inherited_down_a_chain_of_forks_and_never_dropped() {
    let alice = Rights { license: License::CcBy4, author: Some("alice".into()), ..Rights::default() };
    let bob = alice.fork("zQmAlice0000000", None, Some("bob")).rights;
    // Bob relicenses his remix CC0: Alice's credit stays, and Bob is in the lineage too
    let bob_cc0 = Rights { license: License::Cc0, ..bob.clone() };
    let carol = bob_cc0.fork("zQmBob000000000", None, Some("carol")).rights;
    let who: Vec<_> = carol.credits.iter().map(|c| (c.author.clone().unwrap(), c.license)).collect();
    assert_eq!(who, [("alice".to_string(), License::CcBy4), ("bob".to_string(), License::Cc0)]);
    // Bob keeps CC BY: Carol credits both
    let carol2 = bob.fork("zQmBob000000000", None, Some("carol")).rights;
    let who: Vec<_> = carol2.credits.iter().map(|c| c.author.clone().unwrap()).collect();
    assert_eq!(who, ["alice", "bob"]);
    // a parent already credited (the same work by the same author) is not credited twice: Bob
    // forks his own program, unedited, so its CID is the same, and Carol forks that
    let bob_again = bob.fork("zQmBob000000000", None, Some("bob")).rights;
    let again = bob_again.fork("zQmBob000000000", None, Some("carol")).rights;
    assert_eq!(again.credits.len(), 2, "{:?}", again.credits);
}

#[test]
fn using_an_object_credits_what_its_license_asks_and_nothing_more() {
    // an import is not a fork: a CC0 one brings its own credits, and adds none for itself
    let cc0 = Rights { author: Some("alice".into()), ..Rights::default() };
    assert!(cc0.credits_for_use("zQmLib000000000", Some("riffs")).is_empty());
    let forked = cc0.fork("zQmAlice0000000", None, Some("bob")).rights;
    assert_eq!(forked.credits_for_use("zQmBob000000000", None).len(), 1, "its lineage comes with it");
    // a CC BY one is credited, by its name and not as a fork parent
    let by = Rights { license: License::CcBy4, ..cc0.clone() };
    let c = by.credits_for_use("zQmLib000000000", Some("riffs"));
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].name.as_deref(), c[0].is_fork_parent()), (Some("riffs"), false));
    let mut r = Rights::default();
    r.add_credits(c.clone());
    r.add_credits(c);
    assert_eq!(r.credits.len(), 1, "added once");
}

#[test]
fn the_fork_note_is_one_line_and_a_name_that_only_looks_like_it_is_not_a_fork_parent() {
    let c = Credit::fork_parent("zQmP", Some(" alice "), License::Cc0, Some("two\nlines"));
    assert_eq!((c.author.as_deref(), c.name.as_deref()), (Some("alice"), Some("forked from: two lines")));
    let plain = Credit { cid: "zQmP".into(), author: None, license: License::CcBy4, name: Some("forked fromage".into()) };
    assert!(!plain.is_fork_parent());
    assert_eq!(plain.work_name(), Some("forked fromage"));
}

#[test]
fn a_cc_by_parent_with_no_author_is_credited_by_cid_and_warns() {
    let anon = Rights { license: License::CcBy4, ..Rights::default() };
    let f = anon.fork("zQmAnon00000000", None, None);
    assert_eq!(f.rights.credits.len(), 1);
    assert_eq!(f.rights.credits[0].author, None);
    assert!(f.warnings[0].contains("names no author"));
}

#[test]
fn a_fork_of_a_private_object_stays_private() {
    let p = Rights { visibility: Visibility::Private, ..Rights::default() };
    assert_eq!(p.fork("zQmPriv00000000", None, None).rights.visibility, Visibility::Private);
}
