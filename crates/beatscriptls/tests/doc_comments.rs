//! Doc comments (project-datum 6662): the `#` lines directly above a `section`, `let`, `def` or `arrange` line show in the
//! hover of its name, at its definition and at every use, above what the hover already said. A blank line between the
//! comment and the item breaks the attachment; a `#!` header line is never a doc comment.

mod common;

use common::*;
use djengine::json::JsonValue;

const HALL: &str = "\
#! name: Hall of the Rave King
tempo 132
key am
# the theme the hall hums: a minor, low
let theme = a2 b2 c3 d3 e3 c3 e3 ~
# lurk: kick and the theme, quiet and sneaking
section lurk 4 bars {
  deck A: kick x ~ ~ ~ | bass theme
}
# this one has a blank line under it, so it is no doc

section storm 4 bars {
  deck A: kick x x x x | bass theme
}
# stomp: every deck at once, the trolls awake
# and the hall shaking
section stomp 4 bars {
  deck A: kick x x x x | bass theme
  deck B: hihat ~ x ~ x
}
# creep in, then storm, then stomp twice
arrange {
  lurk storm
  stomp stomp
}
";

fn hover(s: &mut beatscriptls::Server, needle: &str, nth: usize, skip: u32) -> String {
    let (l, c) = find(HALL, needle, nth, skip);
    let h = req(s, "textDocument/hover", &at(l, c));
    h.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string()
}

#[test]
fn a_section_hover_shows_its_doc_at_the_definition_and_in_arrange() {
    let mut s = server(HALL);
    let def = hover(&mut s, "section lurk", 0, 9);
    assert!(def.starts_with("kick and the theme, quiet and sneaking\n\n"), "the doc first, its `lurk:` dropped: {def}");
    assert!(def.contains("```btw\nsection lurk"), "then what the hover said before: {def}");
    // its use in `arrange`
    let used = hover(&mut s, "  lurk storm", 0, 3);
    assert!(used.starts_with("kick and the theme, quiet and sneaking\n\n"), "{used}");
    // two comment lines are one paragraph; both plays of stomp say it
    for nth in 0..2 {
        let u = hover(&mut s, "stomp", 3 + nth, 1);
        assert!(u.starts_with("every deck at once, the trolls awake and the hall shaking\n\n"), "use {nth}: {u}");
    }
}

#[test]
fn a_blank_line_breaks_the_attachment() {
    let mut s = server(HALL);
    let h = hover(&mut s, "section storm", 0, 9);
    assert!(h.starts_with("```btw\nsection storm"), "no doc: {h}");
    assert!(!h.contains("blank line"), "{h}");
    let u = hover(&mut s, "lurk storm", 0, 6);
    assert!(u.starts_with("```btw\nsection storm"), "{u}");
}

#[test]
fn a_let_and_the_arrange_show_theirs() {
    let mut s = server(HALL);
    let l = hover(&mut s, "let theme", 0, 5);
    assert!(l.starts_with("the theme the hall hums: a minor, low\n\n```btw\nlet theme"), "{l}");
    let u = hover(&mut s, "bass theme", 0, 6);
    assert!(u.starts_with("the theme the hall hums"), "{u}");
    let a = hover(&mut s, "arrange {", 0, 2);
    assert!(a.starts_with("creep in, then storm, then stomp twice\n\n`arrange`"), "{a}");
    // the header line is not the program's first item's doc
    assert!(!hover(&mut s, "tempo 132", 0, 1).contains("Hall of the Rave King"));
}

#[test]
fn a_long_doc_is_trimmed_in_the_hover() {
    let long = format!("# {}\nsection s 1 bars {{\n  deck A: kick x\n}}\n", "sneak ".repeat(80));
    let mut s = server(&long);
    let (l, c) = find(&long, "section s", 0, 8);
    let h = req(&mut s, "textDocument/hover", &at(l, c));
    let md = h.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string();
    let doc = md.split("\n\n").next().unwrap();
    assert!(doc.ends_with('…') && doc.chars().count() <= djlive::doc::HOVER_MAX + 1, "{doc}");
}
