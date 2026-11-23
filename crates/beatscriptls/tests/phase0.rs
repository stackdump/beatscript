//! Phase 0: the server's lifecycle, documents and diagnostics, positions in both encodings.

mod common;

use beatscriptls::Server;
use common::*;
use djengine::json::JsonValue;

fn msg(s: &mut Server, m: &str) -> Vec<JsonValue> {
    s.handle(&JsonValue::parse(m).unwrap())
}

#[test]
fn initialize_answers_the_capabilities() {
    let mut s = Server::new();
    let out = msg(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#);
    golden("initialize", &out[0]);
}

#[test]
fn a_request_before_initialize_is_refused() {
    let mut s = Server::new();
    let out = msg(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"textDocument/hover","params":{}}"#);
    assert_eq!(out[0].get("error").and_then(|e| e.get("code")).and_then(JsonValue::as_f64), Some(-32002.0));
}

#[test]
fn utf8_is_chosen_when_offered() {
    let mut s = Server::new();
    let out = msg(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]}}}}"#);
    assert_eq!(out[0].get("result").and_then(|r| r.get("capabilities")).and_then(|c| c.get("positionEncoding")).and_then(JsonValue::as_str), Some("utf-8"));
    assert_eq!(s.encoding, beatscriptls::Encoding::Utf8);
}

#[test]
fn open_and_change_publish_diagnostics() {
    let mut s = server("");
    let open = msg(&mut s, r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///a.beatscript","languageId":"btw","version":1,"text":"tempo 120\ndeck A: kcik x ~ x ~\n"}}}"#);
    golden("diagnostics-open", &open[0]);
    // an incremental change: `kcik` -> `kick`
    let change = msg(&mut s, r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///a.beatscript","version":2},"contentChanges":[{"range":{"start":{"line":1,"character":8},"end":{"line":1,"character":12}},"text":"kick"}]}}"#);
    let p = change[0].get("params").unwrap();
    assert_eq!(p.get("version").and_then(JsonValue::as_f64), Some(2.0));
    assert_eq!(p.get("diagnostics").and_then(JsonValue::as_array).map(Vec::len), Some(0), "{}", change[0].js_stringified());
    assert_eq!(s.doc("file:///a.beatscript").unwrap().analysis.text, "tempo 120\ndeck A: kick x ~ x ~\n");
    let close = msg(&mut s, r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///a.beatscript"}}}"#);
    assert!(close[0].js_stringified().contains("\"diagnostics\":[]"));
    assert!(s.doc("file:///a.beatscript").is_none());
}

#[test]
fn diagnostics_count_utf16_after_a_wide_comment() {
    // the comment holds `𝄞` (two UTF-16 units, four bytes): the error's column differs by encoding
    let text = "# 𝄞 riff\ndeck A: kick x ~ 𝄞\n";
    let mut s = server(text);
    let d16 = req(&mut s, "beatscript/diagnostics", &doc());
    let start16 = d16.at(0).unwrap().get("range").unwrap().get("start").unwrap().clone();
    s.encoding = beatscriptls::Encoding::Utf8;
    let d8 = req(&mut s, "beatscript/diagnostics", &doc());
    let start8 = d8.at(0).unwrap().get("range").unwrap().get("start").unwrap().clone();
    assert_eq!(start16.get("line").and_then(JsonValue::as_f64), Some(1.0));
    assert_eq!(start16.get("character").and_then(JsonValue::as_f64), Some(17.0));
    assert_eq!(start8.get("character").and_then(JsonValue::as_f64), Some(17.0), "{}", d8.js_stringified());
}

#[test]
fn unknown_methods_are_method_not_found() {
    let mut s = server("tempo 120\n");
    let out = msg(&mut s, r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/nope","params":{}}"#);
    assert_eq!(out[0].get("error").and_then(|e| e.get("code")).and_then(JsonValue::as_f64), Some(-32601.0));
}

#[test]
fn every_corpus_program_analyses_without_errors() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../beatscript-api/tests/corpus");
    let mut n = 0;
    for e in std::fs::read_dir(root).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "beatscript") {
            let text = std::fs::read_to_string(&p).unwrap();
            let a = beatscriptls::Analysis::checked(text);
            let errs: Vec<_> = a.check().diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
            assert!(errs.is_empty(), "{}: {errs:?}", p.display());
            n += 1;
        }
    }
    assert!(n > 3);
}
