//! `arrange from template(...)`: completion offers a template, a generator call or a seed, then the templates' names, then the phrasebook's
//! ids; the names are the language's own (`djlive::template`), so a template added there is offered here.

mod common;

use btwls::complete::{candidates, EnvNames, Item};
use btwls::Analysis;

/// The items offered with the cursor where `|` is in `text`.
fn offered(text: &str) -> Vec<Item> {
    let at = text.find('|').unwrap();
    let text = text.replacen('|', "", 1);
    candidates(&Analysis::new(&text), at, &EnvNames::web()).0
}

fn labels(text: &str) -> Vec<String> {
    offered(text).into_iter().map(|i| i.label).collect()
}

#[test]
fn after_arrange_from_a_template_or_a_seed_is_offered() {
    let l = labels("tempo 122\narrange from |\n");
    assert_eq!(l, ["template(", "gen@2", "seed("], "{l:?}");
    let items = offered("arrange from |\n");
    assert!(items[0].snippet.as_deref().is_some_and(|s| s.starts_with("template(${1:deep-house}, ")), "{:?}", items[0]);
}

#[test]
fn inside_template_the_names_come_from_the_language() {
    let l = labels("arrange from template(|\n");
    let mut want = djlive::template::names();
    want.sort();
    assert_eq!(l, want, "{l:?}");
    let l = labels("arrange from template(deep-h|\n");
    assert_eq!(l, ["deep-house"], "{l:?}");
}

#[test]
fn after_the_comma_the_phrasebooks_ids_are_offered() {
    let l = labels("arrange from template(deep-house, |\n");
    assert!(l.contains(&"@tempest_such_stuff".to_string()) && l.contains(&"@byron_assyrian_wolf".to_string()) && l.contains(&"@hands_up".to_string()), "{} ids", l.len());
    let l = labels("arrange from template(deep-house, @tempest|\n");
    assert_eq!(l, ["@tempest_such_stuff"], "{l:?}");
}

#[test]
fn an_arrange_that_is_not_from_is_as_it_was() {
    let l = labels("section intro 4 bars {\n  deck A: kick x\n}\narrange intro |\n");
    assert!(l.contains(&"intro".to_string()), "{l:?}");
}
