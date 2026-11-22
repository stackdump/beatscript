//! Regression tests for the findings of the independent review that this branch fixes (L-01 ... L-23, R-01): each
//! test is the reviewer's repro (or the nearest thing that fails without the fix), named by its finding id.

mod common;

use std::time::Instant;

use djlive::compile::check_with;
use djlive::env::Env;
use djlive::{check, print};

fn ok(src: &str) -> djlive::compile::Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String)> {
    let c = check_with(src, &Env::none());
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone())).collect()
}

fn said(e: &[(usize, usize, String)], what: &str) -> bool {
    e.iter().any(|(_, _, m)| m.contains(what))
}

fn count(e: &[(usize, usize, String)], what: &str) -> usize {
    e.iter().filter(|(_, _, m)| m.contains(what)).count()
}

// ---- L-01: nested repeats and calls have a budget

#[test]
fn l01_four_nested_repeats_of_nothing_end_with_an_error_at_once() {
    let src = "repeat 64 {\nrepeat 64 {\nrepeat 64 {\nrepeat 64 {\nmaster: level 0.5 when 1 == 2\n}\n}\n}\n}\n";
    let t = Instant::now();
    let e = errs(src);
    assert!(said(&e, "repeats and calls write out more than"), "{e:?}");
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
}

#[test]
fn l01_a_comment_only_body_is_charged_too() {
    let src = "repeat 64 {\nrepeat 64 {\nrepeat 64 {\nrepeat 64 {\n# nothing\n}\n}\n}\n}\n";
    let t = Instant::now();
    let e = errs(src);
    assert!(said(&e, "repeats and calls write out more than"), "{e:?}");
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
}

#[test]
fn l01_ten_nested_repeats_stop_at_the_first_depth_error() {
    let mut src = String::new();
    for _ in 0..10 {
        src.push_str("repeat 6 {\n");
    }
    src.push_str("master: filter lp 300\n");
    for _ in 0..10 {
        src.push_str("}\n");
    }
    let t = Instant::now();
    let e = errs(&src);
    assert_eq!(count(&e, "nest more than 8 deep"), 1, "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

#[test]
fn l01_twelve_defs_calling_the_next_four_times_stop_at_the_first_depth_error() {
    let mut src = String::from("def hop0() {\n  master: level 0.5\n}\n");
    for k in 1..12 {
        src.push_str(&format!("def hop{k}() {{\n  hop{p}()\n  hop{p}()\n  hop{p}()\n  hop{p}()\n}}\n", p = k - 1));
    }
    src.push_str("hop11()\n");
    let t = Instant::now();
    let e = errs(&src);
    assert_eq!(count(&e, "nest more than 8 deep"), 1, "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

#[test]
fn l01_a_program_that_fits_the_line_cap_still_fits() {
    // 64 x 64 = 4096 lines: exactly the line cap, all of them from repeat copies
    let c = check("repeat 64 {\nrepeat 64 {\nmaster: level 0.5\n}\n}\n");
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, ""));
    let _ = print::line;
    let _ = ok;
}

// ---- L-02: one depth counter for every nesting construct

/// Run `f` on a thread with a small stack: what a text that nests too deep would overflow in a real app (an editor
/// thread, the control socket). A stack overflow aborts the whole test binary, so a test that passes proves the depth
/// cap works whatever stack the caller has.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(1 << 20).spawn(f).unwrap().join().unwrap()
}

fn deep_error(src: String) -> Vec<(usize, usize, String)> {
    small_stack(move || {
        let c = check_with(&src, &Env::none());
        assert!(!c.ok());
        c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(&src).0, d.position(&src).1, d.message.clone())).collect()
    })
}

#[test]
fn l02_nested_groups_are_an_error_at_the_opener() {
    let n = 5000;
    let src = format!("deck A: kick {}x{}\n", "[".repeat(n), "]".repeat(n));
    let e = deep_error(src);
    assert_eq!(count(&e, "nest more than 64 deep"), 1, "{e:?}");
    // the 65th `[` is the first that is too deep: `deck A: kick ` is 13 columns
    assert!(e.iter().any(|(l, c, m)| *l == 1 && *c == 13 + 65 && m.contains("nest more than 64 deep")), "{e:?}");
}

#[test]
fn l02_nested_alternations_are_an_error() {
    let n = 5000;
    let src = format!("deck A: kick {}x{}\n", "<".repeat(n), ">".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
}

#[test]
fn l02_nested_calls_are_an_error() {
    let n = 3000;
    let src = format!("deck A: kick {}x{}\n", "transpose(".repeat(n), ", 1)".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
    let src = format!("deck A: kick {}[x]{}\n", "maybe(".repeat(n), ", 0.5, 1)".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
}

#[test]
fn l02_nested_deg_groups_are_an_error() {
    let n = 3000;
    let src = format!("deck A: melody deg({}1{})\n", "[".repeat(n), "]".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
    let src = format!("deck A: melody deg({}1{})\n", "(".repeat(n), ")".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep") || said(&e, "parenthes") || said(&e, "nest"), "{e:?}");
}

#[test]
fn l02_nested_sections_are_one_error() {
    let n = 2000;
    let mut src = String::new();
    for k in 0..n {
        src.push_str(&format!("section s{k} 1 bars {{\n"));
    }
    for _ in 0..n {
        src.push_str("}\n");
    }
    let e = deep_error(src);
    assert_eq!(count(&e, "nest more than 64 deep"), 1, "{e:?}");
    assert_eq!(e.len(), 1, "no cascade after the first: {e:?}");
}

#[test]
fn l02_sixty_four_levels_still_read() {
    let src = format!("deck A: kick {}x{}\n", "[".repeat(63), "]".repeat(63));
    small_stack(move || {
        let c = check(&src);
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    });
}

#[test]
fn l02_five_hundred_nested_reverses_are_refused_at_once() {
    let n = 500;
    let src = format!("deck A: kick {}x{}\n", "reverse(".repeat(n), ")".repeat(n));
    let t = Instant::now();
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

#[test]
fn l02_a_chain_of_lets_that_wraps_the_one_before_is_refused() {
    let n = 300;
    let mut src = String::from("let pp0 = x ~\n");
    for i in 1..=n {
        src.push_str(&format!("let pp{i} = [[[[[[pp{}]]]]]] x\n", i - 1));
    }
    src.push_str(&format!("deck A: kick pp{n}\n"));
    let t = Instant::now();
    let e = deep_error(src);
    assert!(said(&e, "levels deep once the `let`s in it are put in place"), "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

#[test]
fn l02_a_let_that_doubles_the_one_before_is_refused_before_it_is_built() {
    // 2^40 terms in forty lines: built, it would never fit in memory
    let n = 40;
    let mut src = String::from("let pp0 = x ~\n");
    for i in 1..=n {
        src.push_str(&format!("let pp{i} = pp{} pp{}\n", i - 1, i - 1));
    }
    src.push_str(&format!("deck A: kick pp{n}\n"));
    let t = Instant::now();
    let e = deep_error(src);
    assert!(said(&e, "terms once the `let`s in it are put in place"), "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
    // said for the let that crossed the line (line 14: 2^13 x 2 terms), and again where the rests that stand in for it
    // double past the line (every 13 lets): never at each use, never forty times
    assert_eq!(e[0].0, 14, "{e:?}");
    assert!(count(&e, "terms once the `let`s in it are put in place") <= 3, "{e:?}");
}

#[test]
fn l02_lets_that_stay_within_the_bounds_still_read() {
    let n = 12;
    let mut src = String::from("let pp0 = x ~\n");
    for i in 1..=n {
        src.push_str(&format!("let pp{i} = pp{} pp{}\n", i - 1, i - 1));
    }
    // (the lets are within the cap even where the use is not: only the one used is built)
    src.push_str("deck A: kick pp3\n");
    let c = check(&src);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
}

// ---- L-03 / SYN-03: a bar holds a bounded number of events and the work is bounded too

#[test]
fn l03_nested_repeat_counts_are_refused_with_the_group_span() {
    let src = "deck A: kick [[[x*64]*64]*64]*64\n";
    let t = Instant::now();
    let e = errs(src);
    assert!(said(&e, "events in one bar") || said(&e, "steps to read in one bar"), "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
    let e = errs("deck A: kick [[[[x*64]*64]*64]*64]*64\n");
    assert!(said(&e, "events in one bar") || said(&e, "steps to read in one bar"), "{e:?}");
}

#[test]
fn l03_nested_repeat_counts_of_rests_are_refused_too() {
    // no events are made, but the multiplication is the same work
    let t = Instant::now();
    let e = errs("deck A: kick [[[[[~*64]*64]*64]*64]*64]*64\n");
    assert!(said(&e, "steps to read in one bar"), "{e:?}");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

#[test]
fn l03_the_error_points_at_the_term_and_names_the_way_out() {
    let c = check_with("deck A: kick [[[x*64]*64]*64]*64\n", &Env::none());
    let d = c.diagnostics.iter().find(|d| d.is_error() && d.message.contains("events in one bar")).expect("an error");
    assert!(d.help.as_deref().unwrap_or("").contains("fewer or smaller"), "{d:?}");
}

#[test]
fn l03_a_busy_but_sane_bar_is_not_refused() {
    // 64 x 64 = 4096 events: the most a bar may hold
    let c = check("deck A: hihat [x*64]*64\n");
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, ""));
    let c = check("deck A: hihat x*16 x*16 x*16 x*16 | kick x ~ x ~ | bass c2*8 [e2 g2]*4\n");
    assert!(c.ok());
}

// ---- SYN-02: long operator chains

#[test]
fn syn02_long_chains_of_operators_are_an_error_not_an_overflow() {
    for op in ["+", "-", "*"] {
        let chain = vec!["1"; 800].join(&format!(" {op} "));
        let src = format!("let r = {chain}\nat bar r: master: level 0.5\n");
        let e = deep_error(src);
        assert!(said(&e, "operators"), "{op}: {e:?}");
    }
    for conj in ["and", "or"] {
        let chain = vec!["1 == 1"; 1000].join(&format!(" {conj} "));
        let src = format!("deck A: kick x when {chain}\n");
        let e = deep_error(src);
        assert!(said(&e, "operators"), "{conj}: {e:?}");
    }
}

#[test]
fn syn02_a_chain_within_the_cap_still_reads() {
    let chain = vec!["1"; 100].join(" + ");
    let src = format!("let r = {chain}\nat bar r: master: level 0.5\n");
    small_stack(move || {
        let c = check(&src);
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, &src));
    });
}

#[test]
fn syn02_nested_choose_and_maybe_and_sections_are_errors_not_overflows() {
    let n = 500;
    let src = format!("deck A: kick {}x{}\n", "choose [".repeat(n), "] weights 1".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
    let n = 1000;
    let src = format!("deck A: kick {}x{}\n", "maybe(".repeat(n), ", .5)".repeat(n));
    let e = deep_error(src);
    assert!(said(&e, "nest more than 64 deep"), "{e:?}");
}

// ---- R-01: checked arithmetic on bars in repeat / at bar / every

#[test]
fn r01_bar_arithmetic_that_overflows_is_an_error_with_a_span_not_a_panic() {
    for src in [
        "deck A: kick x ~ x ~\nrepeat 3 every 4294967295 bars {\n  master: level 0.5\n}\n",
        "repeat 2 every 1 bars {\n  at bar 4294967295: master: level 0.5\n}\n",
        "repeat 2 every 1 bars {\n  every 4294967295 bars: master: level 0.5\n}\n",
        "repeat 2 every 1 bars {\n  every 4294967295 bars from 4294967295: master: level 0.5\n}\n",
        "repeat 2 every 1 bars {\n  on phrase: master: level 0.5\n}\nrepeat 2 every 4294967295 bars {\n  on phrase: master: level 0.5\n}\n",
    ] {
        let e = errs(src);
        assert!(said(&e, "past the last bar number"), "{src}: {e:?}");
        assert!(e.iter().all(|(l, c, _)| *l >= 1 && *c >= 1));
    }
}

#[test]
fn r01_a_def_called_at_a_huge_bar_inside_a_repeat() {
    let e = errs("def hit() {\n  master: level 0.5\n}\nrepeat 2 every 1 bars {\n  at bar 4294967295: hit()\n}\n");
    assert!(said(&e, "past the last bar number"), "{e:?}");
}

#[test]
fn r01_ordinary_bar_arithmetic_is_unchanged() {
    let c = check("repeat 4 every 2 bars {\n  at bar 1: master: level 0.5\n}\n");
    assert!(c.ok());
}

// ---- L-04, L-12, SYN-10, SYN-11: what a def puts in place

/// Every flat line the program expands to, printed, in order.
fn lines(src: &str) -> Vec<String> {
    let parsed = djlive::parse::parse(src);
    assert!(parsed.ok(), "{}", djlive::diag::render_all(&parsed.diagnostics, src));
    let mut diags = Vec::new();
    let lw = djlive::expand::lower(&parsed, src, &mut diags);
    assert!(!diags.iter().any(|d| d.is_error()), "{src}\n{}", djlive::diag::render_all(&diags, src));
    let mut out: Vec<String> = lw.globals.iter().map(|f| print::line(&f.line)).collect();
    for s in &lw.segments {
        out.extend(s.lines.iter().map(|f| print::line(&f.line)));
    }
    out
}

/// The net CIDs of a program that must compile.
fn nets(src: &str) -> std::collections::BTreeMap<String, String> {
    ok(src).net_cids()
}

#[test]
fn l04_a_def_argument_of_stars_means_what_it_always_meant() {
    // the base spliced `bass 1*4` (the degree 1 four times); arithmetic had wrapped it, `bass (1 * 4)`, the degree 4 once
    let def = nets("def pulse(p) {\n  deck B: bass $p\n}\npulse(1*4)\n");
    let literal = nets("deck B: bass 1*4\n");
    let wrapped = nets("deck B: bass 4\n");
    assert_eq!(def, literal);
    assert_ne!(def, wrapped);
    // the net the language made before arithmetic (the reviewer's record of the base)
    assert!(def["B"].ends_with("mVpHAozcTT7B"), "{}", def["B"]);
    // the expansion prints as the text of the call, unwrapped
    assert_eq!(lines("def pulse(p) {\n  deck B: bass $p\n}\npulse(1*4)\n"), vec!["deck B: bass 1*4"]);
}

#[test]
fn l04_two_numbers_and_a_star_in_one_argument_are_still_two_terms() {
    let src = "def riff(p) {\n  deck B: bass $p ~ 5\n}\nriff(1*2 3)\n";
    assert_eq!(nets(src), nets("deck B: bass 1*2 3 ~ 5\n"));
    let src = "def riff(p) {\n  deck B: bass $p ~ 5\n}\nriff(3*2)\n";
    assert_eq!(nets(src), nets("deck B: bass 3*2 ~ 5\n"));
    assert_ne!(nets(src), nets("deck B: bass 6 ~ 5\n"));
}

#[test]
fn l04_through_a_repeat_an_import_and_a_wired_call_the_same() {
    let src = "def pulse(p) {\n  deck B: bass $p\n}\nrepeat 1 {\n  pulse(1*4)\n}\n";
    assert_eq!(nets(src), nets("deck B: bass 1*4\n"));
}

#[test]
fn l04_an_imported_def_puts_its_argument_in_place_the_same() {
    let mut snips = djlive::env::StaticSnippets::default();
    let text = "def pulse(p) {\n  deck B: bass $p\n}\n";
    let c = check_with(text, &Env::none());
    let canon = c.compiled.unwrap();
    snips.items.push(("pulses".to_string(), canon.cid.clone(), canon.text.clone()));
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"pulses\"\npulse(1*4)\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    let via_import = c.compiled.unwrap().net_cids();
    assert_eq!(via_import, nets("deck B: bass 1*4\n"));
}

#[test]
fn l04_arithmetic_beside_an_operator_or_in_a_glued_slot_is_still_wrapped() {
    // `$a * 2` with a = 1 + 2 is (1 + 2) * 2
    let l = lines("def cut(a) {\n  at bar $a * 2: master: level 0.5\n}\ncut(1 + 2)\n");
    assert_eq!(l, vec!["at bar 6: master: level 0.5"]);
    // a sum is still a degree where a step of a pattern wants one: the arithmetic of the extension, unchanged
    assert_eq!(nets("def pulse(p) {\n  deck B: bass $p\n}\npulse(1 + 2)\n"), nets("deck B: bass 3\n"));
    assert_eq!(nets("def pulse(p) {\n  deck B: bass $p\n}\npulse(2 * 3 + 1)\n"), nets("deck B: bass 7\n"));
}

#[test]
fn l12_a_min_or_max_argument_is_wrapped_like_the_other_arithmetic() {
    let src = "def fx(a, v) {\n  deck A: kick x*$a\n}\nfx(min(1, 2), 90)\n";
    assert_eq!(nets(src), nets("deck A: kick x*1\n"));
    let src = "def fx(a, v) {\n  deck A: kick x*$a\n}\nfx(max(2, 3), 90)\n";
    assert_eq!(nets(src), nets("deck A: kick x*3\n"));
    // and in a free slot, where it was never needed
    let l = lines("def go(b) {\n  at bar $b: master: level 0.5\n}\ngo(max(1, 3))\n");
    assert_eq!(l, vec!["at bar 3: master: level 0.5"]);
}

#[test]
fn syn11_a_division_argument_in_a_glued_slot_is_wrapped() {
    assert_eq!(nets("def gt(deck, k) {\n  deck $deck: kick x*$k\n}\ngt(A, 8 / 2)\n"), nets("deck A: kick x*4\n"));
    assert_eq!(nets("def gc(p) {\n  deck A: snare x?$p\n}\ngc(1 / 4)\n"), nets("deck A: snare x?0.25\n"));
    assert_eq!(nets("def gh(n) {\n  deck A: hihat/$n x\n}\ngh(12 / 3)\n"), nets("deck A: hihat/4 x\n"));
    // a spaced ratio in an expression keeps the documented reading (a `do` value reads `1/4` as a word)
    let _ = ok("def quiet(v) {\n  master: level $v\n}\nquiet(1/4)\n");
}

#[test]
fn syn11_weights_still_take_a_sum() {
    let src = "def pick(w) {\n  deck A: snare choose [x ~ [x x]] weights $w 1 1\n}\npick(1 + 1)\n";
    assert_eq!(nets(src), nets("deck A: snare choose [x ~ [x x]] weights 2 1 1\n"));
}

#[test]
fn syn10_i_and_n_of_a_repeat_inside_a_def_body_are_the_repeats() {
    let src = "def ramp(bars) {\n  repeat $bars {\n    at bar $i + 1: master: level 0.5\n  }\n}\nramp(3)\n";
    let l = lines(src);
    assert_eq!(l, vec!["at bar 1: master: level 0.5", "at bar 2: master: level 0.5", "at bar 3: master: level 0.5"]);
    // `$n` too
    let src = "def ramp(bars) {\n  repeat $bars {\n    at bar $i + 1: master: level 1 / $n\n  }\n}\nramp(2)\n";
    assert_eq!(lines(src).len(), 2);
}

#[test]
fn syn10_i_outside_a_repeat_says_where_it_exists_and_does_not_suggest_a_name_like_it() {
    let src = "def ffx(ii) {\n  deck A: kick x ~ x ~ when $i == 1\n}\nffx(1)\n";
    let c = check_with(src, &Env::none());
    let msgs: Vec<String> = c.diagnostics.iter().map(|d| format!("{} {}", d.message, d.help.clone().unwrap_or_default())).collect();
    assert!(msgs.iter().any(|m| m.contains("`$i`")), "{msgs:?}");
    assert!(!msgs.iter().any(|m| m.contains("did you mean `$ii`")), "{msgs:?}");
}

// ---- L-11: a let that is not what it looks like says so

#[test]
fn l11_an_error_at_the_use_of_a_scalar_let_names_the_let() {
    let c = check_with("let pat = 1 -1\ndeck B: bass pat\n", &Env::none());
    let d = c.diagnostics.iter().find(|d| d.is_error()).expect("an error");
    assert_eq!(d.position("let pat = 1 -1\ndeck B: bass pat\n").0, 2);
    let h = d.help.clone().unwrap_or_default();
    assert!(h.contains("`let pat = 1 -1`") && h.contains("line 1") && h.contains("the number 0"), "{h}");
    // a let that is a plain number says so too
    let c = check_with("let deep = 29\ndeck B: bass deep\n", &Env::none());
    let h = c.diagnostics.iter().find(|d| d.is_error()).and_then(|d| d.help.clone()).unwrap_or_default();
    assert!(h.contains("`let deep = 29`"), "{h}");
}

#[test]
fn l11_a_percent_mark_glued_to_a_digit_is_not_modulo_and_says_so() {
    let e = errs("let n = 7%2\nat bar 3: master: level 0.5\n");
    assert!(said(&e, "percent mark"), "{e:?}");
    let c = check_with("let n = 7%2\n", &Env::none());
    let d = c.diagnostics.iter().find(|d| d.is_error() && d.message.contains("percent")).unwrap();
    assert!(d.help.as_deref().unwrap_or("").contains("7 % 2"), "{d:?}");
    // with spaces it is modulo, and `50%` alone is still a percentage
    ok("let n = 7 % 4\nat bar n: master: level 0.5\n");
    ok("deck A: kick x?50% x x x\n");
}

#[test]
fn l11_a_let_that_does_not_read_is_said_once_not_at_every_use() {
    let e = errs("let q = 5 5 5 +\ndeck B: bass q\ndeck A: bass q q\n");
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].0, 1);
}

// ---- L-14, L-16, L-23: one error for one mistake, in the words that fit it

#[test]
fn l14_a_section_with_a_bad_length_is_one_error() {
    for head in ["section intro 0 bars {", "section intro 300 bars {", "section intro 1/2 bars {", "section intro 4 beats {"] {
        let src = format!("{head}\n  deck A: kick x ~ x ~\n}}\nsection drop 8 bars {{\n  deck A: kick x x x x\n}}\narrange drop\n");
        let e = errs(&src);
        assert_eq!(e.len(), 1, "{head}: {e:?}");
        assert_eq!(e[0].0, 1, "{head}: {e:?}");
        assert!(!said(&e, "no `{`"), "{e:?}");
    }
}

#[test]
fn l14_the_other_blocks_with_a_bad_head_do_not_cascade_either() {
    let e = errs("repeat 0 {\n  deck A: kick x ~ x ~\n}\nmaster: level 0.5\n");
    assert_eq!(e.len(), 1, "{e:?}");
    let e = errs("def go(a b) {\n  deck A: kick x ~ x ~\n}\n");
    assert_eq!(e.len(), 1, "{e:?}");
    // and the statements after the block are still read: a second mistake is a second error
    let e = errs("section intro 0 bars {\n  deck A: kick x ~ x ~\n}\ndeck B: nothing here\n");
    assert_eq!(e.len(), 2, "{e:?}");
}

#[test]
fn l16_a_weight_too_big_does_not_say_it_has_too_many_decimals() {
    let e = errs("deck A: kick choose [x ~] weights 2147483648 1\n");
    assert!(said(&e, "a weight is at most 1000000000"), "{e:?}");
    assert!(!said(&e, "six decimals"), "{e:?}");
    let e = errs("deck A: kick choose [x ~] weights 0.1234567 1\n");
    assert!(said(&e, "a weight has at most six decimals"), "{e:?}");
}

#[test]
fn l16_zero_lengths_name_the_unit_that_was_written() {
    let e = errs("every 2 beats for 0 beats: master: level 0.5\n");
    assert!(said(&e, "`for 0 beats` would be over before it began"), "{e:?}");
    let e = errs("every 0 beats: master: level 0.5\n");
    assert!(said(&e, "every 0 beats would never be"), "{e:?}");
    let e = errs("every 0 bars: master: level 0.5\n");
    assert!(said(&e, "every 0 bars would never be"), "{e:?}");
}

#[test]
fn l16_the_finest_every_is_the_one_that_is_accepted() {
    let c = check_with("every 1/16 beat: master: level 0.5\n", &Env::none());
    assert!(c.ok());
    let c = check_with("every 1/64 bar: master: level 0.5\n", &Env::none());
    assert!(c.ok());
    for src in ["every 1/65 bar: master: level 0.5\n", "every 1/17 beat: master: level 0.5\n"] {
        let c = check_with(src, &Env::none());
        let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
        let h = d.help.clone().unwrap_or_default();
        assert!(h.contains("1/16 beat") && h.contains("1/64 bar") && !h.contains("1/4 beat"), "{h}");
    }
}

#[test]
fn l16_a_negative_beat_is_not_said_to_be_4() {
    let e = errs("at bar 1 beat -1: master: level 0.5\n");
    assert!(said(&e, "a beat is 0 to 4, and not negative"), "{e:?}");
    let e = errs("at bar 1 beat 4: master: level 0.5\n");
    assert!(said(&e, "not 4 itself"), "{e:?}");
}

/// `n` sections' worth of placements by doubling lets: `let l0 = then(sa, sa)`, `let l1 = then(l0, l0)` ...
fn doubling(k: usize) -> String {
    let mut src = String::from("section sa 1 bars {\n  deck A: kick x ~ x ~\n}\nlet l0 = then(sa, sa)\n");
    for i in 1..k {
        src.push_str(&format!("let l{i} = then(l{}, l{})\n", i - 1, i - 1));
    }
    src.push_str(&format!("arrange l{}\n", k - 1));
    src
}

#[test]
fn l16_an_arrangement_that_expands_too_far_is_said_at_the_arrange() {
    // 2^13 placements: 8192 lines, past the line cap
    let src = doubling(13);
    let e = errs(&src);
    let arrange_line = src.lines().count();
    assert!(e.iter().any(|(l, _, m)| *l == arrange_line && m.contains("this arrangement expands to more than 4096 lines")), "{e:?}");
    assert!(!said(&e, "repeat"), "{e:?}");
    // 2^15 placements: more than a program may place
    let src = doubling(16);
    let e = errs(&src);
    let arrange_line = src.lines().count();
    assert!(e.iter().any(|(l, _, m)| *l == arrange_line && m.contains("places sections more than 16384 times")), "{e:?}");
}

#[test]
fn l23_a_def_with_a_bad_name_is_said_once_not_at_every_call() {
    let e = errs("def d0() {\n  master: level 0.5\n}\nd0()\nat bar 2: d0()\nrepeat 3 {\n  d0()\n}\n");
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(said(&e, "cannot name a def"), "{e:?}");
    assert_eq!(e[0].0, 1);
}

#[test]
fn l23_a_section_with_a_bad_name_is_said_once_not_at_every_use() {
    let src = "section kick 4 bars {\n  deck A: kick x ~ x ~\n}\nsection verse 4 bars {\n  deck A: snare ~ x ~ x\n}\narrange kick verse kick\nat bar 2: master: level 0.5\nend after kick\n";
    let e = errs(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(said(&e, "cannot name a section"), "{e:?}");
    // a mistyped name is still said, with the nearest
    let e = errs("section verse 4 bars {\n  deck A: snare ~ x ~ x\n}\narrange vers\n");
    assert!(said(&e, "there is no section `vers`"), "{e:?}");
}

#[test]
fn l23_a_composition_let_with_a_bad_name_is_said_once() {
    let src = "section a1 4 bars {\n  deck A: kick x ~ x ~\n}\nlet then = stack(a1, a1)\narrange then then\n";
    let e = errs(src);
    assert!(e.iter().filter(|(_, _, m)| m.contains("there is no section")).count() == 0, "{e:?}");
}

// ---- L-22: how deep lets of compositions chain

/// `n` lets, each a `then` of the one before and a section.
fn let_chain(n: usize) -> String {
    let mut src = String::from("section sa 1 bars {\n  deck A: kick x ~ x ~\n}\nlet l0 = then(sa, sa)\n");
    for i in 1..n {
        src.push_str(&format!("let l{i} = then(l{}, sa)\n", i - 1));
    }
    src.push_str(&format!("arrange l{}\n", n - 1));
    src
}

#[test]
fn l22_a_chain_of_128_lets_resolves_and_the_129th_is_refused_as_the_docs_now_say() {
    let found = (100..140).find(|n| {
        let c = check_with(&let_chain(*n), &Env::none());
        !c.ok()
    });
    assert_eq!(found, Some(129), "the first chain too deep");
    let c = check_with(&let_chain(129), &Env::none());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("lets nest too deep") && d.help.as_deref().unwrap_or("").contains("128")), "{:?}", c.diagnostics);
}

#[test]
fn l03_sixty_nested_reverses_are_read_quickly() {
    // (500 were n^3.6: 8.8 s; the nesting cap leaves the ones a person writes)
    let n = 60;
    let src = format!("deck A: kick {}[x ~ x]{}\n", "reverse(".repeat(n), ")".repeat(n));
    let t = Instant::now();
    let c = small_stack(move || check(&src).ok());
    assert!(c, "60 nested reverses are within the bounds");
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}
