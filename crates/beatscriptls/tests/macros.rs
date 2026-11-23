//! A `macro` line in the editor (docs/beatscript/ext-control.md): completion offers the ids of the line's version after the head (the
//! language's own table, `djlive::macros`), then `on` and a deck, then `over`; hover says what the head and an id are.

use beatscriptls::complete::{candidates, EnvNames, Item};
use beatscriptls::{Analysis, Encoding};
use djengine::json::JsonValue;

fn offered(text: &str) -> Vec<Item> {
    let at = text.find('|').unwrap();
    let text = text.replacen('|', "", 1);
    candidates(&Analysis::new(&text), at, &EnvNames::web()).0
}

fn labels(text: &str) -> Vec<String> {
    offered(text).into_iter().map(|i| i.label).collect()
}

#[test]
fn after_the_head_the_ids_of_its_version_are_offered() {
    let mut l = labels("deck A: kick x ~ x ~\nmacro |\n");
    l.sort();
    let mut want: Vec<String> = djlive::macros::V1.iter().map(|e| e.id.to_string()).collect();
    want.sort();
    assert_eq!(l, want);
    // on the control deck, after a time word, and as typed
    assert!(labels("at bar 8: control: macro |\n").contains(&"breakdown".to_string()));
    assert_eq!(labels("macro@1 cathed|\n"), ["cathedral"]);
    let riser = offered("macro |\n").into_iter().find(|i| i.label == "riser").unwrap();
    assert!(riser.doc.contains("rises over") && riser.detail.contains("transition"), "{riser:?}");
}

#[test]
fn then_on_a_deck_and_over() {
    assert_eq!(labels("macro breakdown |\n"), ["on", "over"]);
    assert_eq!(labels("macro breakdown on |\n"), ["A", "B"]);
    assert_eq!(labels("macro breakdown on A |\n"), ["over"]);
    assert_eq!(labels("macro breakdown over 4 bars |\n"), ["on"]);
    // a one-shot of its own length takes no `over`
    assert_eq!(labels("macro cut |\n"), ["on"]);
}

fn hover(text: &str, needle: &str) -> String {
    let at = text.find(needle).unwrap() + 1;
    let h = beatscriptls::hover::hover(&Analysis::new(text), at, &EnvNames::web(), Encoding::Utf8);
    h.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string()
}

#[test]
fn hover_says_what_the_head_and_the_id_are() {
    let text = "deck A: kick x ~ x ~\ncontrol: macro tape_stop on A over 2 bars\n";
    assert!(hover(text, "macro").contains("DJ macro catalogue"), "{}", hover(text, "macro"));
    let id = hover(text, "tape_stop");
    assert!(id.contains("Tape Stop") && id.contains("held"), "{id}");
}
