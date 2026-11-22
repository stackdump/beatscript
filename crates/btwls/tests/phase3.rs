//! Phase 3: go to definition, find references, document highlight, rename.

mod common;

use btwls::Analysis;
use common::*;
use djengine::json::JsonValue;

const PROGRAM: &str = include_str!("programs/symbols.btw");

fn rename_params(line: u32, ch: u32, new: &str) -> String {
    format!(r#"{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":{line},"character":{ch}}},"newName":"{new}"}}"#)
}

/// Apply a WorkspaceEdit's edits on the test document to its text.
fn apply(text: &str, edit: &JsonValue) -> String {
    let ix = btwls::LineIndex::new(text);
    let edits = edit.get("changes").and_then(|c| c.get(URI)).and_then(JsonValue::as_array).unwrap();
    let mut spans: Vec<(usize, usize, String)> = edits
        .iter()
        .map(|e| {
            let r = btwls::json::get_range(e.get("range").unwrap()).unwrap();
            let sp = ix.span(text, r, btwls::Encoding::Utf16);
            (sp.start, sp.end, e.get("newText").unwrap().as_str().unwrap().to_string())
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut t = text.to_string();
    for (a, b, n) in spans {
        t.replace_range(a..b, &n);
    }
    t
}

#[test]
fn definition_of_a_section_from_the_arrange() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "arrange both drop", 0, 14);
    let got = req(&mut s, "textDocument/definition", &at(l, c));
    golden("definition-section", &got);
    assert_eq!(got.get("range").unwrap().get("start").unwrap().get("line").unwrap().as_i64(), Some(24));
}

#[test]
fn definition_of_a_parameter_and_a_port() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "$oct", 0, 2);
    let got = req(&mut s, "textDocument/definition", &at(l, c));
    let (dl, dc) = find(PROGRAM, "def bed(oct)", 0, 8);
    assert_eq!(btwls::json::get_range(got.get("range").unwrap()).unwrap().start, btwls::Position::new(dl, dc));
    // `bed.chords` in the wire goes to bed's in-port, not to changes' out-port of the same name
    let (l, c) = find(PROGRAM, "bed.chords", 0, 5);
    let got = req(&mut s, "textDocument/definition", &at(l, c));
    let (pl, pc) = find(PROGRAM, "in: chords", 0, 4);
    assert_eq!(btwls::json::get_range(got.get("range").unwrap()).unwrap().start, btwls::Position::new(pl, pc));
}

#[test]
fn no_definition_on_a_note_or_a_role() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "kick x", 0, 1);
    assert_eq!(req(&mut s, "textDocument/definition", &at(l, c)), JsonValue::Null);
}

#[test]
fn references_and_highlights() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "let riff", 0, 5);
    let with = req(&mut s, "textDocument/references", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":{l},"character":{c}}},"context":{{"includeDeclaration":true}}}}"#));
    let without = req(&mut s, "textDocument/references", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":{l},"character":{c}}},"context":{{"includeDeclaration":false}}}}"#));
    assert_eq!(with.as_array().unwrap().len(), 3);
    assert_eq!(without.as_array().unwrap().len(), 2);
    let hl = req(&mut s, "textDocument/documentHighlight", &at(l, c));
    golden("highlight-let", &hl);
    let kinds: Vec<i64> = hl.as_array().unwrap().iter().map(|h| h.get("kind").unwrap().as_i64().unwrap()).collect();
    assert_eq!(kinds, [3, 2, 2]);
}

#[test]
fn renaming_a_section_renames_it_in_the_arrange_and_the_end() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "section drop", 0, 9);
    let prep = req(&mut s, "textDocument/prepareRename", &at(l, c));
    assert_eq!(prep.get("placeholder").and_then(JsonValue::as_str), Some("drop"));
    let edit = req(&mut s, "textDocument/rename", &rename_params(l, c, "peak"));
    golden("rename-section", &edit);
    let renamed = apply(PROGRAM, &edit);
    assert!(renamed.contains("section peak 16 bars") && renamed.contains("arrange both peak") && renamed.contains("end after peak"), "{renamed}");
    // the program means the same: same CID
    let (a, b) = (Analysis::checked(PROGRAM), Analysis::checked(renamed.as_str()));
    let (ca, cb) = (a.check().compiled.as_ref().unwrap(), b.check().compiled.as_ref().unwrap());
    let sorted = |c: &djlive::Compiled| {
        let mut v: Vec<String> = c.net_cids().into_values().collect();
        v.sort();
        v
    };
    assert_eq!(sorted(ca), sorted(cb), "the nets are the same");
}

#[test]
fn renaming_a_parameter_keeps_the_dollar() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "$oct", 0, 1);
    // prepareRename names the range without the `$`
    let prep = req(&mut s, "textDocument/prepareRename", &at(l, c));
    let r = btwls::json::get_range(prep.get("range").unwrap()).unwrap();
    assert_eq!(r.end.character - r.start.character, 3);
    let edit = req(&mut s, "textDocument/rename", &rename_params(l, c, "octave_n"));
    let renamed = apply(PROGRAM, &edit);
    assert!(renamed.contains("def bed(octave_n)") && renamed.contains("oct=$octave_n"), "{renamed}");
    assert!(Analysis::checked(renamed.as_str()).check().compiled.is_some());
}

#[test]
fn renaming_a_port_renames_the_wire() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "$chords", 0, 2);
    let edit = req(&mut s, "textDocument/rename", &rename_params(l, c, "changes_in"));
    let renamed = apply(PROGRAM, &edit);
    assert!(renamed.contains("in: changes_in") && renamed.contains("$changes_in") && renamed.contains("-> bed.changes_in"), "{renamed}");
    // the out-port of `changes` keeps its name
    assert!(renamed.contains("out: chords"));
}

fn rename_err(s: &mut btwls::Server, l: u32, c: u32, new: &str) -> String {
    let p = JsonValue::parse(&rename_params(l, c, new)).unwrap();
    match s.request("textDocument/rename", &p) {
        Err((_, m)) => m,
        Ok(v) => panic!("renamed to {new}: {}", v.js_stringified()),
    }
}

#[test]
fn a_new_name_the_language_would_read_otherwise_is_refused() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "section drop", 0, 9);
    assert_eq!(rename_err(&mut s, l, c, "a"), "`a` cannot be a name: `a` reads as a note, chord or key");
    assert_eq!(rename_err(&mut s, l, c, "kick"), "`kick` cannot be a name: `kick` is a word of the language");
    assert_eq!(rename_err(&mut s, l, c, "stack"), "`stack` cannot be a name: `stack` is a word of the language");
    assert_eq!(rename_err(&mut s, l, c, "two words"), "`two words` is not one name: a name is letters, digits, `_` and `-`, starting with a letter");
    assert_eq!(rename_err(&mut s, l, c, "intro"), "`intro` is already the section on line 19");
    // a def may be called `stack` (reserved_as)
    let (l, c) = find(PROGRAM, "def bed", 0, 5);
    assert!(s.request("textDocument/rename", &JsonValue::parse(&rename_params(l, c, "stack")).unwrap()).is_ok());
}

#[test]
fn prepare_rename_refuses_what_is_not_a_name() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "c2 ~", 0, 0);
    let p = JsonValue::parse(&at(l, c)).unwrap();
    let Err((_, m)) = s.request("textDocument/prepareRename", &p) else { panic!("renamed a note") };
    assert_eq!(m, "`c2` is not a name of this program: `c2` reads as a note, chord or key");
    let (l, c) = find(PROGRAM, "kick x", 0, 0);
    let Err((_, m)) = s.request("textDocument/prepareRename", &JsonValue::parse(&at(l, c)).unwrap()) else { panic!() };
    assert_eq!(m, "`kick` is not a name of this program: `kick` is a word of the language");
}
