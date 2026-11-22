//! The static snapshot btwapi checks against is exactly djhost's registry and catalog: the committed
//! JSON is what the generator makes now, every noun and alias answers as the real table does, and the
//! instruments are the same. `BTW_BLESS=1 cargo test -p btwapi --test snapshot` rewrites the file.

mod support;

use djhost::btw_env::{instruments_json, CatalogView, RegistryView};
use djhost::control_registry;
use djlive::env::{Catalog, NounSpec, Registry};

use support::snapshot_gen;

fn undoc(mut n: NounSpec) -> NounSpec {
    for p in n.params.iter_mut() {
        p.doc.clear();
    }
    for a in n.actions.iter_mut() {
        for p in a.params.iter_mut() {
            p.doc.clear();
        }
    }
    n
}

#[test]
fn the_committed_snapshot_is_what_djhosts_tables_make_now() {
    let want = snapshot_gen::generate();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/snapshot.json");
    if std::env::var("BTW_BLESS").is_ok() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).expect("data/snapshot.json");
    assert!(have == want, "crates/btwapi/data/snapshot.json has drifted from djhost's registry or catalog: regenerate it with `BTW_BLESS=1 cargo test -p btwapi --test snapshot` (or `cargo run -q -p btwapi --example gen_snapshot`) and commit it");
}

#[test]
fn the_snapshot_parses_and_the_built_in_env_is_the_committed_file() {
    let e = btwapi::snapshot::parse(btwapi::snapshot::SNAPSHOT_JSON).unwrap();
    assert!(e.catalog.voices.len() > 50 && e.registry.noun_names().len() > 50);
    assert!(btwapi::snapshot::parse("{}").is_err());
    assert!(btwapi::snapshot::parse("not json").is_err());
}

#[test]
fn every_noun_alias_verb_and_param_answers_as_the_real_registry_does() {
    let web = btwapi::web_env();
    let mut checked = 0;
    for n in control_registry::nouns() {
        for name in std::iter::once(&n.name).chain(n.aliases.iter()) {
            let real = undoc(RegistryView.noun(name).unwrap_or_else(|| panic!("{name}")));
            let snap = web.registry.noun(name).unwrap_or_else(|| panic!("{name} missing from the snapshot"));
            assert_eq!(real, snap, "noun {name}");
            checked += 1;
        }
    }
    assert!(checked > 60, "{checked}");
    assert_eq!(RegistryView.noun_names(), web.registry.noun_names(), "the names a did-you-mean offers, in order");
    assert!(web.registry.noun("nonesuch").is_none());
}

#[test]
fn the_catalog_is_the_engines() {
    let web = btwapi::web_env();
    let real = CatalogView;
    assert_eq!(real.voices(), web.catalog.voices());
    assert_eq!(real.families(), web.catalog.families());
    assert_eq!(real.kits(), web.catalog.kits());
    assert_eq!(real.palettes(), web.catalog.palettes());
}

#[test]
fn instruments_json_is_djhosts_byte_for_byte() {
    assert_eq!(btwapi::instruments_json(), instruments_json().js_stringified());
    let j = support::parse(&btwapi::instruments_json());
    for k in ["voices", "families", "kits", "palettes"] {
        assert!(support::get(&j, k).as_array().is_some_and(|a| !a.is_empty()), "{k}");
    }
    assert!(support::get(&j, "voices").as_array().unwrap().iter().any(|v| v.as_str() == Some("wobble-lead")));
}
