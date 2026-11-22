//! `~3` / `@4`: the count after the mark is understood by the editor: completion goes on to the targets, hover says what
//! it is, `numberAt` knows its bounds, and the outline and diagnostics treat the line as a mutation.

mod common;

use btwls::Analysis;
use common::*;
use djengine::json::JsonValue;

const COUNTED: &str = "\
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
  ~3 acid_a rev, hoov up 2
  drop
  @4 hoov rev
  drop
} loop 6 then outro end
";

fn labels(s: &mut btwls::Server, text: &str, needle: &str, skip: u32) -> Vec<String> {
    let (l, c) = find(text, needle, 0, skip);
    let r = req(s, "textDocument/completion", &at(l, c));
    r.get("items").and_then(JsonValue::as_array).map(|v| v.iter().filter_map(|i| i.get("label").and_then(JsonValue::as_str).map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn the_program_checks_and_the_block_has_its_mutations() {
    assert!(btwls::analysis::full_check(COUNTED).ok(), "{:?}", btwls::analysis::full_check(COUNTED).diagnostics);
    let a = Analysis::new(COUNTED);
    let arr = a.symbols.symbols.iter().find(|s| s.kind == btwls::symbols::Kind::Arrange).expect("the block");
    assert_eq!(arr.detail, "intro drop drop drop · 2 mutations · loop 6 then outro end");
}

#[test]
fn completion_after_the_count_offers_the_targets_and_then_the_operators() {
    let mut s = server(COUNTED);
    // `~3 `: what to change
    let ls = labels(&mut s, COUNTED, "~3 acid_a", 3);
    assert!(ls.contains(&"acid_a".to_string()) && ls.contains(&"hoov".to_string()), "{ls:?}");
    let ls = labels(&mut s, COUNTED, "@4 hoov", 3);
    assert!(ls.contains(&"hoov".to_string()), "{ls:?}");
    // `~3 acid_a `: the operators, as after a bare `~`
    let ls = labels(&mut s, COUNTED, "acid_a rev", 7);
    assert!(ls.contains(&"rot".to_string()) && ls.contains(&"thin".to_string()), "{ls:?}");
}

#[test]
fn hover_says_what_the_count_is() {
    let mut s = server(COUNTED);
    let (l, c) = find(COUNTED, "~3 acid_a", 0, 1);
    let r = req(&mut s, "textDocument/hover", &at(l, c));
    let md = r.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("");
    assert!(md.contains("multiple of 3 (passes 3, 6, 9 ...)"), "{md}");
    // the mark itself is as before
    let (l, c) = find(COUNTED, "~3 acid_a", 0, 0);
    let r = req(&mut s, "textDocument/hover", &at(l, c));
    assert!(r.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").contains("persistent"));
}

#[test]
fn number_at_knows_the_count_is_one_to_256_whole_passes() {
    let reg = &btwapi::web_env().registry;
    let a = Analysis::new(COUNTED);
    let k = |needle: &str, skip: usize| btwls::number::number_at(&a, COUNTED.find(needle).unwrap() + skip, reg).map(|n| (n.limit.kind, n.limit.min, n.limit.max, n.limit.integer));
    assert_eq!(k("~3 acid_a", 1), Some(("every", Some(1.0), Some(256.0), true)));
    assert_eq!(k("@4 hoov", 1), Some(("every", Some(1.0), Some(256.0), true)));
    // the arguments after it are still their operators'
    assert_eq!(k("up 2", 3), Some(("mut-int", Some(-64.0), Some(64.0), true)));
    assert_eq!(k("loop 6", 5), Some(("passes", Some(1.0), Some(256.0), true)));
}
