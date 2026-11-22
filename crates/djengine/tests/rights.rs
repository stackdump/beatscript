//! The rights block (docs/LICENSING.md): defaults, parsing, the private gate, forks and credits.

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{License, Rights, Visibility, PRIVATE_REFUSAL};

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
fn a_fork_of_cc_by_credits_the_parents_author() {
    let parent = Rights { license: License::CcBy4, author: Some("did:key:zAlice".into()), ..Rights::default() };
    let f = parent.fork("zQmAliceTrack01", Some("Night Bus"), Some("did:key:zBob"));
    assert_eq!(f.rights.credits.len(), 1);
    let c = &f.rights.credits[0];
    assert_eq!((c.cid.as_str(), c.author.as_deref(), c.license), ("zQmAliceTrack01", Some("did:key:zAlice"), License::CcBy4));
    assert_eq!(c.name.as_deref(), Some("Night Bus"));
    assert!(f.rights.credit_line().contains("did:key:zAlice"));
    assert!(f.rights.credit_line().contains("Night Bus"));
    // the credit is in the JSON and reads back
    assert_eq!(Rights::from_json(&block(&f.rights)).unwrap(), f.rights);
}

#[test]
fn credits_are_inherited_down_a_chain_of_forks_and_never_dropped() {
    let alice = Rights { license: License::CcBy4, author: Some("alice".into()), ..Rights::default() };
    let bob = alice.fork("zQmAlice0000000", None, Some("bob")).rights;
    // Bob relicenses his remix CC0: Alice's credit stays
    let bob_cc0 = Rights { license: License::Cc0, ..bob.clone() };
    let carol = bob_cc0.fork("zQmBob000000000", None, Some("carol")).rights;
    assert_eq!(carol.credits.len(), 1, "a CC0 parent adds no credit but keeps its own");
    assert_eq!(carol.credits[0].author.as_deref(), Some("alice"));
    // Bob keeps CC BY: Carol credits both
    let carol2 = bob.fork("zQmBob000000000", None, Some("carol")).rights;
    let who: Vec<_> = carol2.credits.iter().map(|c| c.author.clone().unwrap()).collect();
    assert_eq!(who, ["alice", "bob"]);
    // forking the same parent twice does not credit it twice
    let again = carol2.fork("zQmBob000000000", None, None).rights;
    assert_eq!(again.credits.len(), 2);
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
