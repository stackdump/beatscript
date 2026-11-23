//! The arrangement syntax: `arrange { }` blocks with `~` / `@` mutations and their endings, and named arrangements
//! (`arrange set: ... then close`, `start set`). Symbols, completion, hover and numberAt cover them, and the words
//! completion offers are held to grammar.ebnf (and docs/ARRANGE.md for the operators) by drift tests.

mod common;

use beatscriptls::symbols::Kind;
use beatscriptls::Analysis;
use common::*;
use djengine::json::JsonValue;

const BLOCK: &str = "\
tempo 132
key am
let acid_a = a1 ~ a1 ~ c2 ~ a1 e2
let hoov = a3 ~ c4 ~ e4 ~ ~ ~
section intro 4 bars {
  deck A: kick x ~ x ~
}
section drop 8 bars {
  deck B: bass acid_a | melody hoov
}
section outro 4 bars {
  deck B: melody hoov
}
arrange {
  intro drop
  ~ acid_a rot 2 | thin 0.2, hoov up 2
  drop
  @ hoov rev
  drop
} loop 4 then outro end
";

const NAMED: &str = "\
section intro 4 bars {
  deck A: kick x ~ x ~
}
section drop 8 bars {
  deck A: kick x x x x
}
section outro 4 bars {
  deck A: kick x ~ ~ ~
}
arrange open: intro then set
arrange set: drop drop loop 2
arrange close: outro end
start open
";

fn labels(s: &mut beatscriptls::Server, text: &str, needle: &str, skip: u32) -> Vec<String> {
    let (l, c) = find(text, needle, 0, skip);
    let r = req(s, "textDocument/completion", &at(l, c));
    r.get("items").and_then(JsonValue::as_array).map(|v| v.iter().filter_map(|i| i.get("label").and_then(JsonValue::as_str).map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn the_programs_check() {
    for t in [BLOCK, NAMED] {
        assert!(beatscriptls::analysis::full_check(t).ok(), "{t}\n{:?}", beatscriptls::analysis::full_check(t).diagnostics);
    }
}

#[test]
fn a_block_is_one_outline_entry_with_its_sections_and_mutations() {
    let a = Analysis::new(BLOCK);
    let arr = a.symbols.symbols.iter().find(|s| s.kind == Kind::Arrange).expect("the block");
    assert_eq!(arr.detail, "intro drop drop drop · 2 mutations · loop 4 then outro end");
    assert!(arr.body.is_some(), "the block is a container");
    // no part or setting comes of its lines
    assert!(a.symbols.symbols.iter().all(|s| s.parent != a.symbols.symbols.iter().position(|x| x.kind == Kind::Arrange)), "nothing inside it is a symbol of its own");
    // the sections and the mutations' targets are references to their definitions
    let refs_to = |name: &str, kind: Kind| {
        let (si, _) = a.symbols.named(name).find(|(_, s)| s.kind == kind).unwrap();
        a.symbols.refs_of(si).filter(|r| !r.is_def).count()
    };
    assert_eq!(refs_to("drop", Kind::Section), 3);
    assert_eq!(refs_to("outro", Kind::Section), 1, "the coda after `then`");
    assert_eq!(refs_to("acid_a", Kind::Let), 2, "the section's part and the mutation");
    assert_eq!(refs_to("hoov", Kind::Let), 4);
}

#[test]
fn named_arrangements_are_names_then_and_start_use() {
    let a = Analysis::new(NAMED);
    let names: Vec<&str> = a.symbols.symbols.iter().filter(|s| s.kind == Kind::Arrangement).map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["open", "set", "close"]);
    let (set, _) = a.symbols.named("set").find(|(_, s)| s.kind == Kind::Arrangement).unwrap();
    assert_eq!(a.symbols.refs_of(set).filter(|r| !r.is_def).count(), 1, "`then set`");
    let (open, _) = a.symbols.named("open").next().unwrap();
    assert_eq!(a.symbols.refs_of(open).filter(|r| !r.is_def).count(), 1, "`start open`");
    // go to definition from `then set`
    let mut s = server(NAMED);
    let (l, c) = find(NAMED, "then set", 0, 6);
    let d = req(&mut s, "textDocument/definition", &at(l, c));
    let line = d.as_array().and_then(|v| v.first()).or(Some(&d)).and_then(|x| x.get("range")).and_then(|r| r.get("start")).and_then(|p| p.get("line")).and_then(JsonValue::as_f64);
    assert_eq!(line, Some(10.0), "{d:?}");
    // completion: after `then` and `start`, the arrangements
    assert!(labels(&mut s, NAMED, "then set", 5).contains(&"set".to_string()));
    let ls = labels(&mut s, NAMED, "start open", 6);
    assert_eq!(ls.iter().filter(|l| ["open", "set", "close"].contains(&l.as_str())).count(), 3, "{ls:?}");
}

#[test]
fn completion_inside_a_block_offers_sections_targets_and_operators() {
    let mut s = server(BLOCK);
    // a new line of the block: the sections, and the two marks
    let ls = labels(&mut s, BLOCK, "  drop\n  @", 2);
    assert!(ls.contains(&"intro".to_string()) && ls.contains(&"~".to_string()) && ls.contains(&"@".to_string()), "{ls:?}");
    // after `~ `: the let patterns
    let ls = labels(&mut s, BLOCK, "~ acid_a", 2);
    assert!(ls.contains(&"acid_a".to_string()) && ls.contains(&"hoov".to_string()), "{ls:?}");
    // after the target: every operator djlive has, and only those
    let ls = labels(&mut s, BLOCK, "acid_a rot", 7);
    let mut want: Vec<String> = djlive::mutate::operator_names().into_iter().map(str::to_string).collect();
    want.sort();
    let mut got = ls.clone();
    got.sort();
    assert_eq!(got, want);
    // after `|`: the operators again
    let ls = labels(&mut s, BLOCK, "| thin", 2);
    assert!(ls.contains(&"thin".to_string()) && ls.contains(&"rot".to_string()));
    // after the block: how it ends, the words of grammar.ebnf's block_end
    let ls = labels(&mut s, BLOCK, "} loop", 2);
    for w in beatscriptls::lang::words_of("block_end").iter().filter(|w| w.chars().all(|c| c.is_ascii_lowercase())) {
        assert!(ls.contains(w), "{w}: {ls:?}");
    }
}

#[test]
fn hover_says_what_an_operator_and_a_mark_do() {
    let mut s = server(BLOCK);
    let h = |s: &mut beatscriptls::Server, needle: &str, skip: u32| {
        let (l, c) = find(BLOCK, needle, 0, skip);
        req(s, "textDocument/hover", &at(l, c)).get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string()
    };
    assert!(h(&mut s, "rot 2", 0).contains("rot n"), "{}", h(&mut s, "rot 2", 0));
    assert!(h(&mut s, "rot 2", 0).contains("steps later"));
    assert!(h(&mut s, "@ hoov", 0).contains("one-shot"));
    assert!(h(&mut s, "~ acid_a", 0).contains("persistent"));
}

#[test]
fn number_at_knows_a_mutation_argument_and_loop_n() {
    let reg = &beatscript_api::web_env().registry;
    let a = Analysis::new(BLOCK);
    let k = |needle: &str, skip: usize| beatscriptls::number::number_at(&a, BLOCK.find(needle).unwrap() + skip, reg).map(|n| (n.limit.kind, n.limit.min, n.limit.max));
    assert_eq!(k("rot 2", 4), Some(("mut-int", Some(-64.0), Some(64.0))));
    assert_eq!(k("thin 0.2", 5), Some(("share", Some(0.0), Some(1.0))));
    assert_eq!(k("up 2", 3), Some(("mut-int", Some(-64.0), Some(64.0))));
    assert_eq!(k("loop 4", 5), Some(("passes", Some(1.0), Some(256.0))));
}

// ---- drift: the words offered are the grammar's, the operators' docs are docs/ARRANGE.md's ------------------------

#[test]
fn every_statement_keyword_of_the_grammar_is_offered_with_a_doc() {
    let first = beatscriptls::lang::first_words("statement");
    // what a statement can start with that completion offers some other way: a deck's or the master's line (snippets),
    // a control word (the actions, and `drive`, which is no reserved word: a section may be named so), `for`, `duck`, `mix`, `sing` and the def calls
    let other: Vec<&str> = ["deck", "master", "for", "duck", "mix", "sing", "do", "drive"].into_iter().chain(djlive::parse::ACTION_WORDS).collect();
    let docs: Vec<&str> = beatscriptls::lang::STATEMENTS.iter().map(|s| s.0).collect();
    let words: Vec<&String> = first.iter().filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_lowercase())).collect();
    for w in &words {
        assert!(docs.contains(&w.as_str()) || other.contains(&w.as_str()), "grammar.ebnf starts a statement with `{w}`: lang::STATEMENTS has no entry for it");
    }
    // and no stale entry: every documented statement word is one the grammar starts a statement with
    for d in &docs {
        assert!(first.contains(*d) || *d == "start" || *d == "do", "lang::STATEMENTS offers `{d}`, which grammar.ebnf does not start a statement with");
    }
}

#[test]
fn every_ending_of_the_grammar_has_a_doc() {
    let words = beatscriptls::lang::words_of("block_end");
    let docs: Vec<&str> = beatscriptls::lang::ENDINGS.iter().map(|e| e.0).collect();
    for w in words.iter().filter(|w| w.chars().all(|c| c.is_ascii_lowercase())) {
        assert!(docs.contains(&w.as_str()), "block_end has `{w}`: lang::ENDINGS has no entry");
    }
}

#[test]
fn every_mutation_operator_has_a_signature_and_a_doc() {
    let ops = beatscriptls::lang::mutation_ops();
    assert_eq!(ops.len(), djlive::mutate::operator_names().len());
    for (n, sig, doc) in &ops {
        assert!(!doc.is_empty(), "docs/ARRANGE.md has no row for `{n}`");
        let want = djlive::mutate::operator_args(n).unwrap().len();
        assert_eq!(sig.split_whitespace().count() - 1, want, "`{sig}`: docs/ARRANGE.md says {} argument(s), djlive takes {want}", sig.split_whitespace().count() - 1);
    }
}

#[test]
fn a_repeated_arrangement_collapses_into_a_loop_and_is_proved() {
    let text = "section sa 4 bars {\n  deck A: kick x ~ x ~\n}\nsection sb 4 bars {\n  deck A: kick x x x x\n}\nsection outro 4 bars {\n  deck A: kick x ~ ~ ~\n}\narrange sa sb sa sb sa sb outro\n";
    let a = Analysis::checked(text);
    let at = text.find("arrange sa").unwrap() + 2;
    let m = beatscriptls::refactor::collapse_arrange(&a, at).expect("offered");
    assert_eq!(m.title, "Collapse into `arrange { sa sb } loop 3 then outro end`");
    let after = beatscriptls::refactor::apply(text, &m.edits);
    assert!(after.ends_with("arrange {\n  sa sb\n} loop 3 then outro end\n"), "{after}");
    beatscriptls::refactor::verify(&a, &m).expect("the same nets");
    // nothing repeats: nothing offered
    let t2 = "section sa 4 bars {\n  deck A: kick x\n}\nsection sb 4 bars {\n  deck A: kick x x\n}\narrange sa sb\n";
    assert!(beatscriptls::refactor::collapse_arrange(&Analysis::new(t2), t2.find("arrange").unwrap()).is_none());
}

#[test]
fn a_named_block_is_a_name_and_a_container() {
    let text = "let hoov = a3 ~ c4 ~\nsection intro 4 bars {\n  deck B: melody hoov\n}\narrange set: {\n  intro\n  ~ hoov up 2\n  intro\n} loop 2\nstart set\n";
    let a = Analysis::new(text);
    let (_, set) = a.symbols.named("set").find(|(_, s)| s.kind == Kind::Arrangement).expect("named");
    assert!(set.body.is_some());
    assert_eq!(set.detail, "intro intro · 1 mutation · loop 2");
    let mut s = server(text);
    let ls = labels(&mut s, text, "~ hoov up", 7);
    assert!(ls.contains(&"rot".to_string()), "operators inside a named block: {ls:?}");
}

// ---- `jux`: an operator that takes an operator ----------------------------------------------------------------------

const JUX: &str = "\
key am
let lead = a3 ~ c4 ~ e4 ~ ~ ~
section drop 8 bars {
  deck B: melody lead
}
arrange {
  drop
  ~ lead rot 1 | jux rot 3 | thin 0.2
  drop
}
";

#[test]
fn jux_is_offered_documented_and_followed_by_the_operators_but_itself() {
    assert!(beatscriptls::analysis::full_check(JUX).ok(), "{:?}", beatscriptls::analysis::full_check(JUX).diagnostics);
    let mut s = server(JUX);
    // as an operator, with its signature as docs/ARRANGE.md has it
    let ls = labels(&mut s, JUX, "lead rot", 5);
    assert!(ls.contains(&"jux".to_string()), "{ls:?}");
    let (_, sig, doc) = beatscriptls::lang::mutation_ops().into_iter().find(|(n, _, _)| *n == "jux").unwrap();
    assert_eq!(sig, "jux op");
    assert!(doc.contains("right copy"), "{doc}");
    // after `jux `: the operator the right copy gets, every one but `jux` itself
    let ls = labels(&mut s, JUX, "jux rot", 4);
    let mut want: Vec<String> = djlive::mutate::operator_names().into_iter().filter(|n| *n != "jux").map(str::to_string).collect();
    want.sort();
    let mut got = ls.clone();
    got.sort();
    assert_eq!(got, want);
    // hover on it says what it does
    let (l, c) = find(JUX, "jux rot", 0, 1);
    let h = req(&mut s, "textDocument/hover", &at(l, c)).get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string();
    assert!(h.contains("jux op") && h.contains("right copy"), "{h}");
    // the number inside it is the inner operator's argument
    let reg = &beatscript_api::web_env().registry;
    let a = Analysis::new(JUX);
    let k = |needle: &str, skip: usize| beatscriptls::number::number_at(&a, JUX.find(needle).unwrap() + skip, reg).map(|n| (n.limit.kind, n.limit.min, n.limit.max));
    assert_eq!(k("rot 3", 4), Some(("mut-int", Some(-64.0), Some(64.0))));
    assert_eq!(k("thin 0.2", 5), Some(("share", Some(0.0), Some(1.0))));
}
