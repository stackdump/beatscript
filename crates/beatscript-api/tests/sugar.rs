//! Pattern sugar (djlive/src/sugar.rs, docs/BEATSCRIPT.md "Pattern sugar") is proved by `same_as`: each sugar and the
//! longhand the docs give for it compile to the same nets. The canonical text keeps the sugar; `check` says what is
//! wrong with it at its line and column, with help.

use djengine::json::JsonValue;

fn program(parts: &str) -> String {
    format!("tempo 124\nkey am\n{parts}\n")
}

fn same(a: &str, b: &str) -> JsonValue {
    JsonValue::parse(&beatscript_api::same_as_json(a, b)).expect("same_as answers JSON")
}

/// `a` and `b` are the same song by their nets (and are not the same text).
fn proves(a: &str, b: &str) {
    let (a, b) = (program(a), program(b));
    let r = same(&a, &b);
    assert_eq!(
        r.get("ok").and_then(JsonValue::as_bool),
        Some(true),
        "both compile:\n{a}\n{b}\n{}",
        r.js_stringified()
    );
    assert_eq!(
        r.get("same").and_then(JsonValue::as_bool),
        Some(true),
        "same nets:\n{a}\n{b}\n{}",
        r.js_stringified()
    );
    assert_eq!(
        r.get("text_same").and_then(JsonValue::as_bool),
        Some(false),
        "two texts:\n{a}\n{b}"
    );
}

fn differs(a: &str, b: &str) {
    let r = same(&program(a), &program(b));
    assert_eq!(
        r.get("same").and_then(JsonValue::as_bool),
        Some(false),
        "{a} / {b}: {}",
        r.js_stringified()
    );
}

#[test]
fn x_colon_n_is_the_ratchet_not_a_replicate() {
    // there is no replicate mark: `x:3` is three strikes inside one sixteenth step (an expression mark), not `x x x`
    differs("deck A: kick x:3 ~", "deck A: kick x x x ~");
    // the marks are the step's, and the sugar writes them out with it
    proves("deck A: hihat x:3~2 x", "deck A: hihat x:3 ~ x");
    proves("deck B: bass c2.&~3 e2!^", "deck B: bass c2.& ~ ~ e2!^");
    proves("deck A: hihat ply([x:2 x.], 2)", "deck A: hihat [x:2*2 x.*2]");
    proves("deck A: hihat off([x:3 ~ ~ ~], 2)", "deck A: hihat [x:3 ~ ~ ~, rotate([x:3 ~ ~ ~], 2)]");
    // a marked group is one step of a call's pattern, its marks on every hit in it
    proves("deck A: hihat palindrome([x x].)", "deck A: hihat <[[x x].] reverse([x x].)>");
}

#[test]
fn elongate_is_the_step_and_rests() {
    proves("deck B: bass c2~3 e2", "deck B: bass c2 ~ ~ e2");
    proves(
        "deck B: melody [a4~2 c5] e5~4 ~",
        "deck B: melody [a4 ~ c5] e5 ~ ~ ~ ~",
    );
    proves(
        "deck B: harmony [a3,c4,e4]~3 [g3,b3,d4]",
        "deck B: harmony [a3,c4,e4] ~ ~ [g3,b3,d4]",
    );
}

#[test]
fn a_bare_chance_is_a_half() {
    // the canonical text is the longhand, so the two are one program
    let a = program("deck A: hihat x*8?");
    let fmt = JsonValue::parse(&beatscript_api::report::fmt_json(&a)).unwrap();
    assert!(
        fmt.get("canonical")
            .and_then(JsonValue::as_str)
            .unwrap()
            .contains("hihat x*8?0.5"),
        "{}",
        fmt.js_stringified()
    );
    let r = same(&a, &program("deck A: hihat x*8?0.5"));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(r.get("text_same").and_then(JsonValue::as_bool), Some(true));
}

#[test]
fn chance_and_markov_under_sugar_keep_their_longhand_seeds() {
    // no seed written: the implicit seed is of the part written out, so the dice fall the same way
    proves(
        "deck A: hihat x*2?0.6~3 x*2?0.6",
        "deck A: hihat x*2?0.6 ~ ~ x*2?0.6",
    );
    proves(
        "deck B: melody markov(am, 0.6)~2 ~",
        "deck B: melody markov(am, 0.6) ~ ~",
    );
    proves(
        "deck A: clap ply([~ x?0.5 ~ x], 2)",
        "deck A: clap [~*2 x*2?0.5 ~*2 x*2]",
    );
}

#[test]
fn poly_walks_each_pattern_the_same_steps_a_bar() {
    proves(
        "deck B: melody poly([c4 e4], [g4 a4 b4])",
        "deck B: melody [[c4 e4], <[g4 a4] [b4 g4] [a4 b4]>]",
    );
    proves(
        "deck B: melody poly(4, [c4 e4 g4])",
        "deck B: melody <[c4 e4 g4 c4] [e4 g4 c4 e4] [g4 c4 e4 g4]>",
    );
    proves("deck A: kick poly(8, [x ~ ~]) | hihat poly(8, [x x ~ x ~])", "deck A: kick <[x ~ ~ x ~ ~ x ~] [~ x ~ ~ x ~ ~ x] [~ ~ x ~ ~ x ~ ~]> | hihat <[x x ~ x ~ x x ~] [x ~ x x ~ x ~ x] [x ~ x ~ x x ~ x] [~ x x ~ x ~ x x] [~ x ~ x x ~ x ~]>");
    // a pattern whose steps divide the count comes round in a bar
    proves(
        "deck A: hihat poly(8, [x ~])",
        "deck A: hihat [x ~ x ~ x ~ x ~]",
    );
}

#[test]
fn ply_plays_each_step_again_in_its_slot() {
    proves(
        "deck A: hihat ply([x ~ [x x] x], 2)",
        "deck A: hihat [x*2 ~*2 [x*2 x*2] x*2]",
    );
    proves(
        "deck B: bass ply([c2 <e2 g2>], 3)",
        "deck B: bass [c2*3 <e2*3 g2*3>]",
    );
    proves("deck A: hihat ply([x*2 x], 2)", "deck A: hihat [x*4 x*2]");
}

#[test]
fn palindrome_is_the_pattern_then_backwards() {
    proves(
        "deck B: melody palindrome([c4 e4 g4 b4])",
        "deck B: melody <[c4 e4 g4 b4] reverse([c4 e4 g4 b4])>",
    );
    proves(
        "deck B: melody palindrome([c4 e4 g4 b4])",
        "deck B: melody <[c4 e4 g4 b4] [b4 g4 e4 c4]>",
    );
}

#[test]
fn iter_turns_the_pattern_a_part_a_bar() {
    proves(
        "deck B: melody iter([c4 e4 g4 b4], 4)",
        "deck B: melody <[c4 e4 g4 b4] rotate([c4 e4 g4 b4], -1) rotate([c4 e4 g4 b4], -2) rotate([c4 e4 g4 b4], -3)>",
    );
    proves(
        "deck B: melody iter([c4 e4 g4 b4], 4)",
        "deck B: melody <[c4 e4 g4 b4] [e4 g4 b4 c4] [g4 b4 c4 e4] [b4 c4 e4 g4]>",
    );
    proves(
        "deck A: kick iter([x ~ ~ x ~ ~ x ~], 2)",
        "deck A: kick <[x ~ ~ x ~ ~ x ~] [~ ~ x ~ x ~ ~ x]>",
    );
}

#[test]
fn off_stacks_a_moved_copy() {
    proves(
        "deck B: melody off([c4 ~ e4 ~ g4 ~ ~ ~], 1, 12)",
        "deck B: melody [c4 ~ e4 ~ g4 ~ ~ ~, transpose(rotate([c4 ~ e4 ~ g4 ~ ~ ~], 1), 12)]",
    );
    proves("deck A: hihat off([x ~ ~ ~], 2)", "deck A: hihat [x ~ x ~]");
}

#[test]
fn sugar_through_lets_defs_sections_and_nesting() {
    proves(
        "let riff = [c4 e4 g4 b4]\ndeck B: melody palindrome(ply(riff, 2)) oct=4",
        "deck B: melody <[c4*2 e4*2 g4*2 b4*2] reverse([c4*2 e4*2 g4*2 b4*2])> oct=4",
    );
    proves(
        "def hats(n) { deck A: hihat x~$n x ~~$n }\nhats(3)",
        "deck A: hihat x ~ ~ x ~ ~ ~",
    );
    proves(
        "section verse 4 bars {\n  deck A: kick x~2 ~~2\n}\nsection chorus 4 bars {\n  deck A: kick iter([x ~ x x], 2)\n}\narrange verse chorus verse",
        "section verse 4 bars {\n  deck A: kick x ~ ~ ~\n}\nsection chorus 4 bars {\n  deck A: kick <[x ~ x x] [x x x ~]>\n}\narrange verse chorus verse",
    );
    proves(
        "deck B: melody transpose(off([a4 ~ c5 ~], 2), 3)",
        "deck B: melody transpose([a4 ~ c5 ~, rotate([a4 ~ c5 ~], 2)], 3)",
    );
}

#[test]
fn the_canonical_text_keeps_the_sugar() {
    let a = program("deck A: kick x~3   ~~2 x:3~2 | hihat ply( [x x] ,2 )\ndeck B: melody poly( 4 ,[c4 e4 g4] ) | bass iter([c2 e2],2) | harmony palindrome([a3 c4]) | arp off([a4 c5],1,12)");
    let fmt = JsonValue::parse(&beatscript_api::report::fmt_json(&a)).unwrap();
    let c = fmt.get("canonical").and_then(JsonValue::as_str).unwrap();
    for want in [
        "kick x~3 ~~2 x:3~2",
        "hihat ply([x x], 2)",
        "melody poly(4, [c4 e4 g4])",
        "bass iter([c2 e2], 2)",
        "harmony palindrome([a3 c4])",
        "arp off([a4 c5], 1, 12)",
    ] {
        assert!(c.contains(want), "{want} in {c}");
    }
    // and reading it back is the same program
    let again = JsonValue::parse(&beatscript_api::report::fmt_json(c)).unwrap();
    assert_eq!(again.get("canonical").and_then(JsonValue::as_str), Some(c));
}

/// The first diagnostic of `parts`: (line, col, message, help).
fn first_error(parts: &str) -> (i64, i64, String, String) {
    let r = JsonValue::parse(&beatscript_api::check_json(&program(parts))).unwrap();
    let d = r
        .get("diagnostics")
        .and_then(JsonValue::as_array)
        .and_then(|a| {
            a.iter()
                .find(|d| d.get("severity").and_then(JsonValue::as_str) == Some("error"))
                .cloned()
        })
        .unwrap_or_else(|| panic!("no error for {parts}"));
    let n = |k: &str| d.get(k).and_then(JsonValue::as_f64).unwrap() as i64;
    let s = |k: &str| {
        d.get(k)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string()
    };
    (n("line"), n("col"), s("message"), s("help"))
}

#[test]
fn check_says_what_is_wrong_with_sugar_where_it_is() {
    let cases: &[(&str, i64, &str, &str)] = &[
        (
            "deck A: kick x~0",
            16,
            "the count of `~` is a whole number from 1 to 64",
            "c2~3 is c2 ~ ~",
        ),
        (
            "deck A: kick x~2~3",
            17,
            "a step takes one `~n`",
            "c2~5",
        ),
        (
            "deck A: kick x~2*3",
            17,
            "`*` comes before the `~`",
            "the marks, the repeat and the chance first",
        ),
        // the expression marks go on the step, before the count
        (
            "deck A: kick x~2:3",
            17,
            "`:` comes before the `~`",
            "the marks",
        ),
        (
            "deck A: kick x~2.",
            17,
            "comes before the `~`",
            "the marks",
        ),
        (
            "deck A: kick <x~2 ~>",
            16,
            "a term of `< >` is a whole bar",
            "write it 2 times",
        ),
        (
            "deck A: kick iter([x ~ x], 2)",
            28,
            "its 3 steps do not divide by 2",
            "1, 3",
        ),
        (
            "deck A: kick ply(e(3,8), 2)",
            18,
            "`e(..)` is a call",
            "ply([x ~ ~ x ~ ~ x ~], 2)",
        ),
        ("deck A: kick ply([x*40], 2)", 19, "`*80`", "ply by less"),
        (
            "deck B: melody off([c4 e4], 1, 99)",
            32,
            "the semitones is a whole number from -48 to 48",
            "off(riff, 2)",
        ),
        (
            "deck A: kick palindrome(x, 2)",
            14,
            "palindrome() takes 1, and this has 2",
            "palindrome(riff)",
        ),
        (
            "deck A: kick poly(0, [x ~])",
            19,
            "the steps a bar is a whole number from 1 to 64",
            "poly([a b], [c d e])",
        ),
        (
            "deck A: kick poly(64, [poly(64, [x x x x x x x])])",
            34,
            "more than 16384 steps",
            "multiply",
        ),
    ];
    for (parts, col, msg, help) in cases {
        let (line, c, m, h) = first_error(parts);
        assert_eq!((line, c), (3, *col), "{parts}: {m}");
        assert!(m.contains(msg), "{parts}: {m}");
        assert!(h.contains(help), "{parts}: {h}");
    }
}

#[test]
fn a_let_or_a_def_prints_its_sugar_as_typed() {
    // a let's value and a def's body are printed from their tokens: a glued `~3` stays glued (one long note) and a
    // spaced one stays three steps, so the canonical text reads back as the same program
    for (src, want) in [
        // (`x:2` is a ratchet, printed touching as it always was; `x:$n` too, its count a def's argument)
        ("let r = c2~3 e2 x:2\ndeck B: bass r", "let r = c2~3 e2 x:2"),
        ("let r = 1 ~ 3 5 1~ 3\ndeck B: bass r", "let r = 1 ~ 3 5 1 ~ 3"),
        ("def hats(n) {\n  deck A: hihat x:$n ~~$n\n}\nhats(4)", "deck A: hihat x:$n ~~$n"),
    ] {
        let a = program(src);
        let fmt = JsonValue::parse(&beatscript_api::report::fmt_json(&a)).unwrap();
        let c = fmt.get("canonical").and_then(JsonValue::as_str).unwrap_or_else(|| panic!("{a}: {}", fmt.js_stringified()));
        assert!(c.contains(want), "{want} in {c}");
        let again = JsonValue::parse(&beatscript_api::report::fmt_json(c)).unwrap();
        assert_eq!(again.get("canonical").and_then(JsonValue::as_str), Some(c));
        let r = same(&a, c);
        assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(true), "{c}");
    }
    proves("let r = c2~3 e2\ndeck B: bass r", "deck B: bass c2 ~ ~ e2");
    differs("let r = c2~3 e2\ndeck B: bass r", "let r = c2 ~ 3 e2\ndeck B: bass r");
    proves("def hats(n) {\n  deck A: hihat x:$n ~~$n\n}\nhats(4)", "deck A: hihat x:4 ~ ~ ~ ~");
}

#[test]
fn an_arrange_mutation_turns_the_steps_of_the_longhand() {
    // a mutation works on a pattern's steps: a sugared `let` is mutated as its longhand is
    let arranged = |acid: &str| {
        format!("let acid = {acid}\nsection verse 4 bars {{\n  deck B: bass acid\n}}\narrange {{\n  verse\n  ~ acid rot 1 | rev\n  verse\n  @ acid swap\n  verse\n}}")
    };
    proves(&arranged("c2~3 e2:2 g2~2 ~~2"), &arranged("c2 ~ ~ e2:2 g2 ~ ~ ~"));
    proves(&arranged("iter([c2 e2 g2 b2], 2)"), &arranged("<[c2 e2 g2 b2] [g2 b2 c2 e2]>"));
}

#[test]
fn two_bare_chances_are_two_halves() {
    let r = same(&program("deck A: hihat x*8??"), &program("deck A: hihat x*8?0.5?0.5"));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(true), "{}", r.js_stringified());
    assert_eq!(r.get("text_same").and_then(JsonValue::as_bool), Some(true), "{}", r.js_stringified());
}

#[test]
fn a_part_with_a_cycle_of_its_own_counts_its_bars_written_out() {
    // the period `check` notes for a `/N` part is the longhand's: `iter(p, 2)` is an alternation of two bars, not one
    let period = |parts: &str| -> Vec<String> {
        let r = JsonValue::parse(&beatscript_api::check_json(&program(parts))).unwrap();
        r.get("diagnostics")
            .and_then(JsonValue::as_array)
            .unwrap()
            .iter()
            .filter_map(|d| d.get("message").and_then(JsonValue::as_str))
            .filter(|m| m.starts_with("period:"))
            .map(str::to_string)
            .collect()
    };
    let (a, b) = (period("deck A: kick x ~ | hihat/12 iter([x ~ x x], 2) ~"), period("deck A: kick x ~ | hihat/12 <[x ~ x x] [x x x ~]> ~"));
    assert!(!a.is_empty(), "a period note");
    assert_eq!(a, b);
    proves("deck A: kick x ~ | hihat/12 iter([x ~ x x], 2) ~", "deck A: kick x ~ | hihat/12 <[x ~ x x] [x x x ~]> ~");
}

#[test]
fn the_example_is_its_longhand() {
    let sugar = include_str!("../../../examples/beatscript/sugar.beatscript");
    let longhand = include_str!("sugar_fixtures/longhand.beatscript");
    let r = same(sugar, longhand);
    assert_eq!(
        r.get("same").and_then(JsonValue::as_bool),
        Some(true),
        "{}",
        r.js_stringified()
    );
    assert_eq!(r.get("basis").and_then(JsonValue::as_str), Some("sections"));
}
