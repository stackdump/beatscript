//! Phase 1: quick-fixes from the diagnostics (the fixer's repairs, through the adapter until feat/btw-fixer lands), and fix-all.

mod common;

use beatscriptls::Analysis;
use common::*;
use djengine::json::JsonValue;

const TYPOS: &str = "tempo 120\ndeck A: kcik x ~ x ~ | snare ~ x ~ x\ndeck B: bsas a1 c2 inst=sub-bas\n";

fn actions(s: &mut beatscriptls::Server, line: u32, from: u32, to: u32, only: &str) -> JsonValue {
    req(s, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{line},"character":{from}}},"end":{{"line":{line},"character":{to}}}}},"context":{{"diagnostics":[]{only}}}}}"#))
}

#[test]
fn a_misspelt_role_offers_its_respelling() {
    let mut s = server(TYPOS);
    let got = actions(&mut s, 1, 9, 9, "");
    golden("codeaction-respell", &got);
    let first = got.at(0).unwrap();
    assert_eq!(first.get("title").and_then(JsonValue::as_str), Some("`kcik` -> `kick`"));
    assert_eq!(first.get("isPreferred"), Some(&JsonValue::Bool(true)));
}

#[test]
fn fix_all_applies_every_certain_repair_and_the_program_checks() {
    // the parser's two typos first; the voice is checked once the lines read, so a second round respells it
    let a = Analysis::checked(TYPOS);
    let fixes = beatscriptls::fix::fix_all(&a);
    assert_eq!(fixes.len(), 2, "{fixes:?}");
    let once = beatscriptls::fix::apply_all(&a);
    assert_eq!(once, "tempo 120\ndeck A: kick x ~ x ~ | snare ~ x ~ x\ndeck B: bass a1 c2 inst=sub-bas\n");
    // `sub-bas` is a warning (it plays the `sub` family voice): its respelling is a guess, offered but not applied by fix-all
    let second = Analysis::checked(once.clone());
    assert_eq!(beatscriptls::fix::apply_all(&second), once);
    let w = second.diagnostics().iter().find(|d| d.message.contains("sub-bas")).unwrap();
    let f = beatscriptls::fix::fixes_of(w, &once);
    assert_eq!((f.len(), f[0].replacement.as_str(), f[0].confidence), (1, "sub-bass", beatscriptls::fix::Confidence::Likely));
    let after = Analysis::checked(once);
    assert!(after.check().compiled.is_some(), "{:?}", after.check().diagnostics);
    // and the client can ask for it by kind
    let mut s = server(TYPOS);
    let got = actions(&mut s, 0, 0, 0, r#","only":["source.fixAll"]"#);
    let all = got.as_array().unwrap();
    assert_eq!(all.len(), 1, "{}", got.js_stringified());
    assert_eq!(all[0].get("kind").and_then(JsonValue::as_str), Some("source.fixAll.beatscript"));
    golden("codeaction-fixall", &got);
}

#[test]
fn an_unclosed_block_is_closed_at_the_end() {
    let text = "section a 4 bars {\n  deck A: kick x ~ x ~\narrange a\n";
    let a = Analysis::checked(text);
    let d = a.diagnostics().iter().find(|d| d.message.contains("never closed")).expect("a never-closed error");
    let f = beatscriptls::fix::fixes_of(d, text);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].confidence, beatscriptls::fix::Confidence::Likely);
    let mut t = text.to_string();
    t.replace_range(f[0].span.start..f[0].span.end, &f[0].replacement);
    assert!(t.ends_with("arrange a\n}\n"), "{t:?}");
}

#[test]
fn no_quickfix_where_there_is_no_diagnostic() {
    let mut s = server(TYPOS);
    let got = actions(&mut s, 0, 0, 3, r#","only":["quickfix"]"#);
    // the line has none of its own; fix-all is offered, as there are two certain repairs in the document
    let titles: Vec<&str> = got.as_array().unwrap().iter().filter_map(|a| a.get("title").and_then(JsonValue::as_str)).collect();
    assert_eq!(titles, ["Fix all certain problems (2)"]);
}
