//! The rename (btw -> beatscript) and BeatScript 2: the old custom methods and command ids are still answered, only the new
//! ones are advertised; "Upgrade to BeatScript 2" is offered on a BeatScript 1 text, and document formatting upgrades.
//! (rename-to-beatscript: keep. This file sends the old names on purpose.)

mod common;

use beatscriptls::{Encoding, LineIndex, Server};
use common::*;
use djengine::json::JsonValue;

const V1: &str = "tempo 120\ndeck A: kick x ~ x ~ | snare ~ x ~ x\n";
const SECTIONS: &str = "tempo 126\nkey am\nsection one from gen@2 house seed=42 section=intro\nsection two from gen@2 house seed=42 section=drop\narrange one two\n";

fn apply(text: &str, edits: &JsonValue) -> String {
    let ix = LineIndex::new(text);
    let mut spans: Vec<(usize, usize, String)> = edits
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let sp = ix.span(text, beatscriptls::json::get_range(e.get("range").unwrap()).unwrap(), Encoding::Utf16);
            (sp.start, sp.end, e.get("newText").and_then(JsonValue::as_str).unwrap().to_string())
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut t = text.to_string();
    for (a, b, n) in spans {
        t.replace_range(a..b, &n);
    }
    t
}

fn actions(s: &mut Server, line: u32, character: u32, only: &str) -> Vec<JsonValue> {
    let got = req(s, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{line},"character":{character}}},"end":{{"line":{line},"character":{character}}}}},"context":{{"diagnostics":[]{only}}}}}"#));
    got.as_array().unwrap().clone()
}

fn upgrade_of(acts: &[JsonValue]) -> Option<&JsonValue> {
    acts.iter().find(|a| a.get("title").and_then(JsonValue::as_str) == Some(beatscriptls::upgrade::TITLE))
}

#[test]
fn the_old_methods_answer_as_the_new_ones() {
    beatscript_api::generate::install();
    let mut s = server(SECTIONS);
    for (old, new, params) in [("btw/expand", "beatscript/expand", doc()), ("btw/diagnostics", "beatscript/diagnostics", doc()), ("btw/numberAt", "beatscript/numberAt", at(0, 7))] {
        let a = req(&mut s, old, &params).js_stringified();
        let b = req(&mut s, new, &params).js_stringified();
        assert_eq!(a, b, "{old}");
        assert!(a.len() > 4, "{old}: {a}");
    }
    // something that never was a method is still not one
    assert!(s.request("btw/nonesuch", &JsonValue::parse("{}").unwrap()).is_err());
}

#[test]
fn the_old_command_ids_run_the_new_commands() {
    beatscript_api::generate::install();
    let mut s = Server::new();
    s.handle(&JsonValue::parse(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"applyEdit":true}}}}"#).unwrap());
    s.open(URI, SECTIONS, 1);
    let lenses = req(&mut s, "textDocument/codeLens", &doc());
    let args = lenses.at(0).unwrap().get("command").unwrap().get("arguments").unwrap().js_stringified();
    for (old, new) in beatscriptls::legacy::COMMANDS {
        let a = req(&mut s, "workspace/executeCommand", &format!(r#"{{"command":"{old}","arguments":{args}}}"#)).js_stringified();
        let b = req(&mut s, "workspace/executeCommand", &format!(r#"{{"command":"{new}","arguments":{args}}}"#)).js_stringified();
        assert_eq!(a, b, "{old}");
    }
    // over the wire: an old eject id is sent as workspace/applyEdit, as the new one is
    let out = s.handle(&JsonValue::parse(&format!(r#"{{"jsonrpc":"2.0","id":5,"method":"workspace/executeCommand","params":{{"command":"btw.eject","arguments":{args}}}}}"#)).unwrap());
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].get("method").and_then(JsonValue::as_str), Some("workspace/applyEdit"));
    // an unknown command is refused naming the new ones only
    let Err((_, m)) = s.request("workspace/executeCommand", &JsonValue::parse(&format!(r#"{{"command":"btw.nonesuch","arguments":{args}}}"#)).unwrap()) else { panic!("ran an unknown command") };
    assert!(m.contains("beatscript.eject") && !m.contains("btw.eject"), "{m}");
}

#[test]
fn only_the_new_names_are_advertised_and_diagnostics_say_beatscript() {
    let mut s = Server::new();
    let out = s.handle(&JsonValue::parse(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap());
    let caps = out[0].get("result").unwrap().get("capabilities").unwrap();
    let all = caps.js_stringified();
    assert!(!all.contains("btw"), "{all}");
    assert!(caps.get("experimental").and_then(|e| e.get("beatscript")).is_some(), "{all}");
    assert_eq!(caps.get("documentFormattingProvider"), Some(&JsonValue::Bool(true)));
    let commands = caps.get("executeCommandProvider").and_then(|e| e.get("commands")).and_then(JsonValue::strings).unwrap();
    assert!(commands.iter().all(|c| c.starts_with("beatscript.")), "{commands:?}");
    s.open(URI, "tempo 12x\n", 1);
    let d = s.publish(URI);
    let first = d.get("params").and_then(|p| p.get("diagnostics")).and_then(|d| d.at(0)).unwrap();
    assert_eq!(first.get("source").and_then(JsonValue::as_str), Some("beatscript"));
}

#[test]
fn an_editor_asking_for_the_old_fix_all_kind_gets_it() {
    let typos = "tempo 120\ndeck A: kcik x ~ x ~ | snare ~ x ~ x\ndeck B: bsas a1 c2\n";
    let mut s = server(typos);
    let acts = actions(&mut s, 0, 0, r#","only":["source.fixAll.btw"]"#);
    assert_eq!(acts.len(), 1, "{acts:?}");
    assert_eq!(acts[0].get("kind").and_then(JsonValue::as_str), Some(beatscriptls::legacy::FIX_ALL));
}

#[test]
fn upgrade_is_offered_on_a_beatscript_1_text_and_writes_the_version_line() {
    for text in [V1, "#! btw 1\n#! name: x\ntempo 120\ndeck A: kick x ~ x ~\n", "#! name: x\n#! btw 1\n# a comment\ntempo 120\n"] {
        let mut s = server(text);
        let acts = actions(&mut s, 0, 0, "");
        let up = upgrade_of(&acts).unwrap_or_else(|| panic!("no upgrade on {text:?}: {acts:?}"));
        assert_eq!(up.get("kind").and_then(JsonValue::as_str), Some("refactor.rewrite"));
        let edits = up.get("edit").and_then(|e| e.get("changes")).and_then(|c| c.get(URI)).unwrap();
        let got = apply(text, edits);
        assert_eq!(got, djlive::version::upgrade_source(text), "{text:?}");
        assert_eq!(djlive::version::of_source(&got), djlive::version::Version::V2);
        // it plays the same: the BeatScript 2 program is the BeatScript 1 one under its new CID
        let c1 = djlive::check(text);
        let c2 = djlive::check(&got);
        let (a, b) = (c1.compiled.unwrap(), c2.compiled.unwrap());
        assert_eq!(djlive::compile::sound_cid(&a.text), djlive::compile::sound_cid(&b.text));
        assert_eq!(djlive::compile::renamed_cid(&a.text), b.cid);
        // resolve hands it back as it is (it comes with its edit)
        assert_eq!(req(&mut s, "codeAction/resolve", &up.js_stringified()).js_stringified(), up.js_stringified());
        // and asked for by kind
        assert!(upgrade_of(&actions(&mut s, 0, 0, r#","only":["refactor"]"#)).is_some());
        assert!(upgrade_of(&actions(&mut s, 0, 0, r#","only":["quickfix"]"#)).is_none());
    }
    // not on a text in BeatScript 2, nor on one whose version line does not read, nor away from the header
    for text in ["#! beatscript 2\ntempo 120\n", "#! beatscript 3\ntempo 120\n"] {
        let mut s = server(text);
        assert!(upgrade_of(&actions(&mut s, 0, 0, "")).is_none(), "{text:?}");
    }
    let mut s = server(V1);
    assert!(upgrade_of(&actions(&mut s, 1, 3, "")).is_none());
}

#[test]
fn formatting_upgrades_and_a_formatted_beatscript_2_text_has_no_edit() {
    let messy = "#! name: loose\ntempo   120   # fast\n\n\ndeck A:   kick x ~ x ~\n";
    let mut s = server(messy);
    let edits = req(&mut s, "textDocument/formatting", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"options":{{"tabSize":2,"insertSpaces":true}}}}"#));
    let got = apply(messy, &edits);
    assert_eq!(got, djlive::format::format_upgrading(messy, &beatscript_api::web_env().env()).unwrap());
    assert!(got.starts_with("#! beatscript 2\n#! name: loose\n"), "{got}");
    assert_eq!(beatscript_api::upgrade_text(messy).as_deref(), Some(got.as_str()), "the page's and the CLI's fmt");
    // formatted and in BeatScript 2 already: nothing to do; a text that does not read: nothing either
    for text in [got.as_str(), "deck A: kick |\n"] {
        let mut s = server(text);
        let e = req(&mut s, "textDocument/formatting", &doc());
        assert_eq!(e.js_stringified(), "[]", "{text:?}");
    }
}
