//! Phase 2: the symbol table (definitions and references), the outline, the folding ranges.

mod common;

use beatscriptls::symbols::Kind;
use beatscriptls::Analysis;
use common::*;

const PROGRAM: &str = include_str!("programs/symbols.btw");

#[test]
fn the_program_checks() {
    let a = Analysis::checked(PROGRAM);
    assert!(a.check().compiled.is_some(), "{:?}", a.check().diagnostics);
}

#[test]
fn every_name_is_defined_once_and_used_where_it_is_used() {
    let a = Analysis::new(PROGRAM);
    let sy = &a.symbols;
    let uses = |name: &str, kind: Kind| -> Vec<String> {
        // (a port of `bed`: `changes` has a port of the same name)
        let (i, _) = sy.named(name).filter(|(_, s)| s.kind == kind).last().unwrap_or_else(|| panic!("no {kind:?} {name}"));
        sy.refs_of(i).map(|r| format!("{}{}@{}", if r.is_def { "def " } else { "" }, r.span.text(PROGRAM), a.index.line_of(r.span.start) + 1)).collect()
    };
    assert_eq!(uses("intro", Kind::Section), ["def intro@19", "intro@37"]);
    assert_eq!(uses("drop", Kind::Section), ["def drop@25", "drop@38", "drop@39"]);
    assert_eq!(uses("both", Kind::LetSection), ["def both@37", "both@38"]);
    assert_eq!(uses("outro", Kind::Section), ["def outro@33", "outro@37"]);
    assert_eq!(uses("bars8", Kind::Let), ["def bars8@5", "bars8@19"]);
    assert_eq!(uses("riff", Kind::Let), ["def riff@6", "riff@21", "riff@27"]);
    assert_eq!(uses("harp", Kind::Sample), ["def harp@7", "harp@21"]);
    assert_eq!(uses("bed", Kind::Def), ["def bed@13", "bed@17", "bed@22"]);
    assert_eq!(uses("changes", Kind::Def), ["def changes@9", "changes@17"]);
    assert_eq!(uses("oct", Kind::Param), ["def oct@13", "$oct@14"]);
    assert_eq!(uses("chords", Kind::Port), ["def chords@13", "$chords@14", "chords@17"]);
    // the `out` port of `changes` is used by the wire
    let out_of_changes = sy.symbols.iter().position(|s| s.kind == Kind::Port && s.name == "out").map(|i| sy.refs_of(i).count());
    assert_eq!(out_of_changes, None, "`out:` is the port list's word, `chords` its port");
}

#[test]
fn the_outline_nests_parts_and_parameters() {
    let mut s = server(PROGRAM);
    let got = req(&mut s, "textDocument/documentSymbol", &doc());
    golden("document-symbols", &got);
}

#[test]
fn blocks_and_comment_runs_fold() {
    let mut s = server(PROGRAM);
    let got = req(&mut s, "textDocument/foldingRange", &doc());
    golden("folding-ranges", &got);
    let lines: Vec<(i64, i64)> = got.as_array().unwrap().iter().map(|f| (f.get("startLine").unwrap().as_i64().unwrap(), f.get("endLine").unwrap().as_i64().unwrap())).collect();
    // the comments (0-1), changes (8-10), bed (12-14), intro (18-22), drop (24-30) and its evolve (27-29), outro (32-34)
    assert_eq!(lines, [(0, 1), (8, 10), (12, 14), (18, 22), (24, 30), (27, 29), (32, 34)]);
}

#[test]
fn a_program_with_errors_still_has_its_symbols() {
    // the section header does not read (no `bars`): its name is still a symbol, and the arrange still refers to it
    let text = "section intro 8 {\n  deck A: kick x ~ x ~\n}\narrange intro\n";
    let a = Analysis::new(text);
    assert!(!a.diagnostics().is_empty());
    let (i, _) = a.symbols.named("intro").next().expect("intro");
    assert_eq!(a.symbols.refs_of(i).count(), 2);
}

#[test]
fn the_showcase_symbols() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/btw/showcase.btw")).unwrap();
    let a = Analysis::new(text.as_str());
    let names: Vec<String> = a.symbols.roots().into_iter().map(|i| &a.symbols.symbols[i]).filter(|s| s.kind == Kind::Section).map(|s| s.name.clone()).collect();
    assert_eq!(names[..4], ["intro", "build", "groove", "hook_a"]);
    // `groove` is used in `before groove 2 beats:` and in `stack(groove, ...)`
    let (g, _) = a.symbols.named("groove").find(|(_, s)| s.kind == Kind::Section).unwrap();
    assert!(a.symbols.refs_of(g).filter(|r| !r.is_def).count() >= 2);
}
