//! Integration of fixer a (the reader puts a scalar `let`'s number in place of its name) with fixer b (a ratio that is no
//! short decimal prints as the fraction): a name put in place of a `let` of a ratio says the fraction at its use, as the
//! same ratio typed there would, so the same music is one program. And an old program's literals, `0.3333333333333333`
//! among them, print as they always did.


use djlive::compile::{check, check_with, Compiled};
use djlive::env::{Env, StaticSnippets};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn text(src: &str) -> String {
    ok(src).text
}

#[test]
fn a_name_put_in_place_of_a_ratio_prints_as_the_fraction_the_ratio_typed_there_would() {
    // the case of the review: two spellings of the same music, which were two CIDs
    let by_name = ok("let third = 1/3\nmaster: level third\n");
    let typed = ok("let third = 1/3\nmaster: level 1/3\n");
    assert_eq!(by_name.text, "let third = 1 / 3\nmaster: level 1 / 3\n");
    assert_eq!(by_name.text, typed.text);
    assert_eq!(by_name.cid, typed.cid);
    // the text reads back as itself
    assert_eq!(ok(&by_name.text).text, by_name.text);
    assert_eq!(ok(&by_name.text).cid, by_name.cid);
    // the number is the number it was
    assert_eq!(by_name.controls[0].key, typed.controls[0].key);
}

#[test]
fn every_lone_number_slot_says_the_fraction() {
    let t = text("let third = 1/3\nswing third\nmaster: delay third\nmaster: level third\ndeck A: kick x ~ x ~\nat bar 2: master: reverb third\n");
    for line in ["swing 1 / 3", "master: delay 1 / 3", "master: level 1 / 3", "master: reverb 1 / 3"] {
        assert!(t.contains(line), "{line:?} in\n{t}");
    }
    assert!(!t.contains("0.3333"), "{t}");
    // in a section's body too
    let s = text("let third = 1/3\nsection verse 4 bars {\n  master: level third\n}\narrange verse\n");
    assert!(s.contains("  master: level 1 / 3\n"), "{s}");
    assert!(!s.contains("0.3333"), "{s}");
}

#[test]
fn a_ratio_reached_by_a_copy_or_by_arithmetic_on_a_ratio_says_the_fraction() {
    let t = text("let third = 1/3\nlet again = third\nlet two = third * 2\nlet back = third * 3\nmaster: level again\nmaster: level two\nmaster: level back\n");
    assert!(t.contains("let again = 1 / 3\n"), "{t}");
    assert!(t.contains("let two = 2 / 3\n"), "{t}");
    // (a whole number is a whole number)
    assert!(t.contains("let back = 1\n"), "{t}");
    for line in ["master: level 1 / 3\n", "master: level 2 / 3\n", "master: level 1\n"] {
        assert!(t.contains(line), "{line:?} in\n{t}");
    }
    assert!(!t.contains("0.3333") && !t.contains("0.6666"), "{t}");
    assert_eq!(ok(&t).cid, ok(&t.replace("let again = 1 / 3", "let again = 2/6")).cid);
}

#[test]
fn a_short_decimal_is_as_it_was_and_a_use_in_arithmetic_is_said_by_the_arithmetic() {
    let t = text("let half = 1/2\nlet fifth = 1/5\nmaster: level half\nmaster: level fifth\n");
    assert!(t.contains("master: level 0.5\n") && t.contains("master: level 0.2\n"), "{t}");
    // a use that is part of arithmetic was always said by the arithmetic
    let a = text("let third = 1/3\nmaster: level third * 2\nmaster: level 1 / third / 10\n");
    assert!(a.contains("master: level 2 / 3\n") && a.contains("master: level 0.3\n"), "{a}");
}

#[test]
fn an_old_literal_of_sixteen_digits_stays_as_it_was_written_in_a_let_and_at_its_use() {
    // the text and the CID the pre-extension build prints for this program (checked against it)
    let src = "let old = 0.3333333333333333\nmaster: level old\nmaster: level 0.3333333333333333\nmaster: level 0.6666666666666666\n";
    let c = ok(src);
    assert_eq!(c.text, "let old = 0.3333333333333333\nmaster: level 0.3333333333333333\nmaster: level 0.3333333333333333\nmaster: level 0.6666666666666666\n");
    assert_eq!(c.cid, "z4EBG9jGy61pNBrgnrYK8KuK9xvvrXmgLQYj3o2GXsVESZzoZ2V");
    // beside a let of arithmetic, which is a fraction: each keeps its own
    let both = text("let third = 1/3\nlet old = 0.3333333333333333\nmaster: level third\nmaster: level old\n");
    assert_eq!(both, "let third = 1 / 3\nlet old = 0.3333333333333333\nmaster: level 1 / 3\nmaster: level 0.3333333333333333\n");
    // a name that is a copy of a literal is the literal
    assert!(text("let old = 0.3333333333333333\nlet same = old\nmaster: level same\n").ends_with("master: level 0.3333333333333333\n"));
}

#[test]
fn a_ratio_let_of_an_import_says_the_fraction_where_the_program_uses_it() {
    let lib = ok("let third = 1/3\nlet old = 0.3333333333333333\n");
    let mut snips = StaticSnippets::default();
    snips.items.push(("lib".to_string(), lib.cid.clone(), lib.text.clone()));
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let c = check_with(&format!("import {}\nmaster: level third\nmaster: level old\n", lib.cid), &env);
    let t = c.compiled.unwrap_or_else(|| panic!("{:?}", c.diagnostics)).text;
    assert!(t.ends_with("master: level 1 / 3\nmaster: level 0.3333333333333333\n"), "{t}");
}

#[test]
fn a_def_body_reads_back_as_the_same_number() {
    // (a body is kept as tokens and read when a call expands it: the number is the number, whichever way it is spelt)
    let d = ok("let third = 1/3\ndef lay() {\n  master: level third\n}\nlay()\nmaster: level third\n");
    assert_eq!(ok(&d.text).cid, d.cid);
    assert_eq!(ok(&d.text).controls.iter().map(|c| c.key.clone()).collect::<Vec<_>>(), d.controls.iter().map(|c| c.key.clone()).collect::<Vec<_>>());
}
