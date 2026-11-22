//! Doc comments in check's report (project-datum 6662): each `sections` entry of a section with a doc comment (the `#`
//! lines directly above its `section` line) carries it as `doc`, and `docs` lists every one (sections, lets, defs, the
//! arrange). They are comments: the CID and the canonical text are the same with them, without them, or with others.

mod support;

use djengine::json::JsonValue;
use support::*;

const BARE: &str = "\
tempo 132
key am
let theme = a2 b2 c3 d3 e3 c3 e3 ~
section lurk 4 bars {
  deck A: kick x ~ ~ ~ | bass theme
}
section stomp 4 bars {
  deck A: kick x x x x | bass theme
  deck B: hihat ~ x ~ x
}
arrange {
  lurk stomp stomp
}
";

const COMMENTED: &str = "\
#! name: Hall of the Rave King
tempo 132
key am
# the theme the hall hums
let theme = a2 b2 c3 d3 e3 c3 e3 ~
# lurk: kick and the theme, quiet and sneaking
section lurk 4 bars {
  deck A: kick x ~ ~ ~ | bass theme
}
# a note with a blank line under it is no doc

section stomp 4 bars {
  deck A: kick x x x x | bass theme
  deck B: hihat ~ x ~ x
}
# creep in, then stomp
arrange {
  lurk stomp stomp
}
";

fn check(text: &str) -> JsonValue {
    let r = parse(&btwapi::check_json(text));
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", btwapi::check_json(text));
    r
}

#[test]
fn a_sections_doc_is_in_check() {
    let r = check(COMMENTED);
    let secs = get(&r, "sections").as_array().unwrap();
    let doc = |i: usize| secs[i].get("doc").and_then(JsonValue::as_str).map(str::to_string);
    assert_eq!(s(&secs[0], "name"), "lurk");
    assert_eq!(doc(0).as_deref(), Some("kick and the theme, quiet and sneaking"), "its `lurk:` dropped");
    // a blank line between breaks it: stomp has no doc, and its entries are as they were
    assert_eq!(s(&secs[1], "name"), "stomp");
    assert_eq!(doc(1), None);
    let docs = get(&r, "docs").as_array().unwrap();
    let got: Vec<(String, String, String)> = docs.iter().map(|d| (s(d, "kind").to_string(), s(d, "name").to_string(), s(d, "doc").to_string())).collect();
    assert_eq!(
        got,
        vec![
            ("let".into(), "theme".into(), "the theme the hall hums".into()),
            ("section".into(), "lurk".into(), "kick and the theme, quiet and sneaking".into()),
            ("arrange".into(), "arrange".into(), "creep in, then stomp".into()),
        ]
    );
}

#[test]
fn a_program_without_doc_comments_has_the_report_it_had() {
    let r = check(BARE);
    assert!(r.get("docs").is_none());
    for e in get(&r, "sections").as_array().unwrap() {
        assert!(e.get("doc").is_none(), "{e:?}");
    }
}

#[test]
fn doc_comments_never_move_the_cid() {
    let (a, b) = (check(BARE), check(COMMENTED));
    assert_eq!(s(&a, "cid"), s(&b, "cid"), "the same program with and without its doc comments");
    assert_eq!(s(&a, "canonical"), s(&b, "canonical"));
    assert!(!s(&b, "canonical").contains('#'), "comments are not in the canonical text");
    let other = COMMENTED.replace("quiet and sneaking", "something else entirely");
    assert_eq!(s(&check(&other), "cid"), s(&a, "cid"), "nor does a different doc");
    assert_eq!(djlive::compile::program_cid(s(&b, "canonical")), s(&b, "cid"));
}
