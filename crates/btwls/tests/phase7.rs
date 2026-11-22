//! Phase 7: refactors (extract let, inline let), proved by the nets they compile to.

mod common;

use btwls::refactor::{apply, extract_let, inline_let, verify};
use btwls::Analysis;
use common::*;
use djengine::json::JsonValue;
use djlive::Span;

const TEXT: &str = "tempo 120\nlet riff = c2 ~ [e2 g2] ~\nlet n = 2 * 4\nsection verse n bars {\n  deck A: kick x ~ x ~ | hihat x*8\n  deck B: bass riff riff\n}\narrange verse verse\n";

fn sel(text: &str, needle: &str) -> Span {
    let k = text.find(needle).unwrap();
    Span::new(k, k + needle.len())
}

#[test]
fn extract_a_whole_pattern_into_a_let() {
    let a = Analysis::checked(TEXT);
    let m = extract_let(&a, sel(TEXT, "x ~ x ~")).expect("offered");
    assert_eq!(m.title, "Extract `x ~ x ~` into `let pat1`");
    let after = apply(TEXT, &m.edits);
    assert!(after.contains("let pat1 = x ~ x ~\nsection verse n bars {") && after.contains("deck A: kick pat1 | hihat"), "{after}");
    verify(&a, &m).expect("the same nets");
}

#[test]
fn extracting_part_of_a_pattern_is_refused_with_the_reason() {
    let a = Analysis::checked(TEXT);
    // `x ~` of `x ~ x ~`: a let stands in one step, so the bar would change
    let k = TEXT.find("x ~ x ~").unwrap();
    let m = extract_let(&a, Span::new(k, k + 3)).expect("offered (it is proved on resolve)");
    let err = verify(&a, &m).unwrap_err();
    assert!(err.contains("would play differently"), "{err}");
}

#[test]
fn a_number_extracts_as_n1_and_a_header_is_not_offered() {
    let text = "tempo 120\ndeck A: kick x ~ x ~ vel=90\n";
    let a = Analysis::checked(text);
    let m = extract_let(&a, sel(text, "90")).expect("offered");
    assert!(m.title.ends_with("`let n1`"));
    verify(&a, &m).unwrap();
    // not in a section's header or a let
    let a = Analysis::checked(TEXT);
    assert!(extract_let(&a, sel(TEXT, "2 * 4")).is_none());
    assert!(extract_let(&a, sel(TEXT, "verse n bars")).is_none());
}

#[test]
fn inline_a_pattern_let_comments_it_out_and_plays_the_same() {
    let a = Analysis::checked(TEXT);
    let at = TEXT.find("bass riff").unwrap() + 6;
    let m = inline_let(&a, at).expect("offered");
    assert_eq!(m.title, "Inline `let riff` (2 uses)");
    let after = apply(TEXT, &m.edits);
    assert!(after.contains("# let riff = c2 ~ [e2 g2] ~  (inlined into its 2 uses)\n"), "{after}");
    assert!(after.contains("deck B: bass [c2 ~ [e2 g2] ~] [c2 ~ [e2 g2] ~]"), "{after}");
    verify(&a, &m).unwrap();
}

#[test]
fn inline_a_number_let_puts_the_number_it_came_to() {
    let a = Analysis::checked(TEXT);
    let at = TEXT.find("let n").unwrap() + 4;
    let m = inline_let(&a, at).expect("offered");
    let after = apply(TEXT, &m.edits);
    assert!(after.contains("section verse 8 bars {") && after.contains("# let n = 2 * 4  (inlined into its 1 use)"), "{after}");
    verify(&a, &m).unwrap();
}

#[test]
fn the_lsp_offers_unresolved_and_resolve_proves() {
    let mut s = server(TEXT);
    let k = TEXT.find("x ~ x ~").unwrap();
    let (l, c) = find(TEXT, "x ~ x ~", 0, 0);
    let list = req(&mut s, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{l},"character":{c}}},"end":{{"line":{l},"character":{}}}}},"context":{{"diagnostics":[],"only":["refactor"]}}}}"#, c + 7));
    let acts = list.as_array().unwrap();
    assert_eq!(acts.len(), 1, "{}", list.js_stringified());
    assert!(acts[0].get("edit").is_none());
    let resolved = req(&mut s, "codeAction/resolve", &acts[0].js_stringified());
    golden("refactor-extract-resolved", &resolved);
    // a part of the pattern: offered, refused on resolve
    let _ = k;
    let bad = req(&mut s, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{l},"character":{c}}},"end":{{"line":{l},"character":{}}}}},"context":{{"diagnostics":[],"only":["refactor.extract"]}}}}"#, c + 3));
    let p = JsonValue::parse(&bad.at(0).unwrap().js_stringified()).unwrap();
    let Err((_, m)) = s.request("codeAction/resolve", &p) else { panic!("resolved a change of sound") };
    assert!(m.contains("would play differently"), "{m}");
}
