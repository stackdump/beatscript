//! F10, composition made literal (docs/btw/ext-algebra.md): `stack(a, b)` and `then(a, b)` over sections and over
//! parts, `silence` as their unit, `arrange a b c` as `then(a, b, c)`, `let NAME = sexpr`. The canonical form is the
//! normal form by the laws of a symmetric monoidal category (so programs equal by the laws have one CID), the timeline
//! lays stacked sections of unequal length out as consecutive segments, and the property tests at the end are the
//! heart of it: random re-association, unit insertion and operand permutation keep the CID, a swap of two `then`
//! operands changes it, and the compiled song of a random composition is the hand-written longhand's.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic};
use djlive::runtime::{Grid, Live, LiveHost};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}\n{}", src, render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(!c.ok(), "{src}\nshould not compile");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn err(src: &str) -> Diagnostic {
    let mut e = errors(src);
    assert_eq!(e.len(), 1, "{}\nwanted one error, got {:#?}", src, e.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    e.remove(0)
}

fn canon(src: &str) -> String {
    ok(src).text
}

/// The text after the section definitions.
fn tail(text: &str) -> String {
    text.lines().skip_while(|l| !l.starts_with("arrange") && !l.starts_with("let ")).collect::<Vec<_>>().join("\n")
}

/// Sections that define nothing a stack could clash on: a deck's part each, all different.
const SECS: &str = "\
section intro 4 bars {
  deck A: kick x ~ x ~
}
section verse 8 bars {
  deck A: snare ~ x ~ x
}
section chorus 8 bars {
  deck A: hihat x*8
}
section outro 2 bars {
  deck B: bass c2 ~ e2 ~
}
";

fn song(rest: &str) -> String {
    format!("{SECS}{rest}")
}

fn names(c: &Compiled) -> Vec<(String, u32, u32, bool)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.chain)).collect()
}

fn decks_of(c: &Compiled, k: usize) -> Vec<&'static str> {
    c.segments[k].decks.keys().map(|d| d.name()).collect()
}

// ---- the canonical form

#[test]
fn a_plain_arrange_still_prints_as_it_always_did() {
    let c = ok(&song("arrange intro verse outro\n"));
    assert!(c.text.ends_with("arrange intro verse outro\n"), "{}", c.text);
    assert_eq!(names(&c), vec![("intro".into(), 0, 4, false), ("verse".into(), 4, 8, false), ("outro".into(), 12, 2, false)]);
    // the spacing and the line it is on do not matter
    let c2 = ok(&song("arrange   intro\tverse  outro   # the song\n"));
    assert_eq!(c.cid, c2.cid);
}

#[test]
fn arrange_a_b_c_is_then_of_them_however_it_is_associated() {
    let want = canon(&song("arrange intro verse outro\n"));
    for spelling in [
        "arrange then(intro, verse, outro)",
        "arrange then(intro, then(verse, outro))",
        "arrange then(then(intro, verse), outro)",
        "arrange intro then(verse, outro)",
        "arrange then(intro, verse) outro",
        "arrange then(then(intro), then(then(verse), outro))",
        "arrange then(intro,\n   verse,\n   outro)",
        "arrange then ( intro , verse , outro )",
    ] {
        assert_eq!(canon(&song(&format!("{spelling}\n"))), want, "{spelling}");
    }
}

#[test]
fn silence_is_the_unit_of_then_and_of_stack() {
    let want = canon(&song("arrange intro verse\n"));
    for spelling in [
        "arrange intro silence verse",
        "arrange silence intro verse silence",
        "arrange then(intro, silence, verse)",
        "arrange then(silence, then(intro, silence), verse)",
        "arrange then(stack(intro, silence), verse)",
        "arrange stack(silence, intro) stack(verse, silence, silence)",
        "arrange then(stack(silence, silence), intro, verse)",
    ] {
        assert_eq!(canon(&song(&format!("{spelling}\n"))), want, "{spelling}");
    }
    // then(silence) is silence, and an arrange of nothing but silence says so
    let e = err(&song("arrange then(silence)\n"));
    assert!(e.message.contains("no sections"), "{}", e.message);
    err(&song("arrange silence\n"));
    err(&song("arrange stack(silence, silence) silence\n"));
}

#[test]
fn a_one_operand_stack_or_then_is_its_operand() {
    let want = canon(&song("arrange intro\n"));
    for spelling in ["arrange then(intro)", "arrange stack(intro)", "arrange stack(then(intro))", "arrange then(stack(then(stack(intro))))"] {
        assert_eq!(canon(&song(&format!("{spelling}\n"))), want, "{spelling}");
    }
}

#[test]
fn stack_is_symmetric_its_operands_sorted_by_canonical_text() {
    let a = canon(&song("arrange stack(verse, intro)\n"));
    let b = canon(&song("arrange stack(intro, verse)\n"));
    assert_eq!(a, b);
    assert!(a.ends_with("arrange stack(intro, verse)\n"), "{a}");
    // nested stacks flatten and the whole is sorted
    let c = canon(&song("arrange stack(verse, stack(outro, intro), chorus)\n"));
    assert!(c.ends_with("arrange stack(chorus, intro, outro, verse)\n"), "{c}");
    assert_eq!(c, canon(&song("arrange stack(stack(chorus, verse), stack(intro, outro))\n")));
    // a then is an operand like another, sorted by its text: `chorus` < `then(...)` < `verse`
    let d = canon(&song("arrange stack(verse, then(intro, outro), chorus)\n"));
    assert!(d.ends_with("arrange stack(chorus, then(intro, outro), verse)\n"), "{d}");
}

#[test]
fn then_is_not_commutative_and_is_never_reordered() {
    let a = ok(&song("arrange intro verse\n"));
    let b = ok(&song("arrange verse intro\n"));
    assert_ne!(a.cid, b.cid);
    assert!(b.text.ends_with("arrange verse intro\n"));
    // inside a stack, too
    let c = canon(&song("arrange stack(then(intro, verse), chorus)\n"));
    let d = canon(&song("arrange stack(then(verse, intro), chorus)\n"));
    assert_ne!(c, d);
    assert!(d.ends_with("arrange stack(chorus, then(verse, intro))\n"), "{d}");
}

#[test]
fn stack_is_not_idempotent_two_copies_are_two_sections() {
    let src = "section hush 4 bars {\n  master: filter lp 800\n}\narrange stack(hush, hush)\n";
    let c = ok(src);
    assert!(c.text.ends_with("arrange stack(hush, hush)\n"), "{}", c.text);
    // both play at once: two placements on the same four bars, their control fired twice
    assert_eq!(c.segments.len(), 1);
    assert_eq!(c.controls.len(), 2);
}

#[test]
fn a_let_names_a_composition_and_prints_in_normal_form() {
    let c = ok(&song("let both = stack(verse, intro)\narrange both outro\n"));
    assert!(c.text.contains("let both = stack(intro, verse)\n"), "{}", c.text);
    assert!(c.text.ends_with("arrange both outro\n"));
    // the same composition written the other way round, or nested, is the same text
    let d = ok(&song("let both = stack(stack(intro), then(stack(verse)))\narrange both outro\n"));
    assert_eq!(c.cid, d.cid);
    // `both` is a name: it plays as the stack
    assert_eq!(names(&c), vec![("intro+verse".into(), 0, 4, false), ("verse".into(), 4, 4, true), ("outro".into(), 8, 2, false)]);
    // lets can use the lets above them
    let e = ok(&song("let both = stack(intro, verse)\nlet more = then(both, outro)\narrange more\n"));
    assert_eq!(e.segments.len(), 3);
    assert_eq!(e.segments.last().unwrap().name, "outro");
}

#[test]
fn a_let_of_a_lone_section_is_the_scalar_let_it_was() {
    // by the laws `then(intro)` is `intro`, so this is `let first = intro`: the value is put where it is used
    let a = canon(&song("let first = then(intro, silence)\narrange first verse\n"));
    let b = canon(&song("let first = intro\narrange first verse\n"));
    assert_eq!(a, b);
    assert!(a.ends_with("let first = intro\narrange intro verse\n"), "{a}");
    // and the canonical text is a fixed point
    assert_eq!(canon(&a), a);
}

#[test]
fn stack_and_then_over_a_section_expression_may_run_over_lines() {
    let one = canon(&song("arrange stack(intro, verse) outro\n"));
    let many = canon(&song("arrange stack(\n    intro,   # the intro\n    verse\n  ) outro\n"));
    assert_eq!(one, many);
}

#[test]
fn the_canonical_text_is_a_fixed_point_and_random_reformatting_keeps_the_cid() {
    let progs = [
        song("let both = stack(verse, then(intro, intro))\narrange then(both, silence) stack(outro, chorus) then(intro, outro)\n"),
        song("arrange stack(verse, intro) stack(outro, then(chorus, outro))\nend after intro\n"),
        "section hush 4 bars {\n  master: filter lp 800\n}\nsection swell 4 bars {\n  master: reverb 0.4\n}\nlet twice = then(hush, swell)\narrange stack(twice, twice) twice\n".to_string(),
    ];
    for (n, src) in progs.iter().enumerate() {
        let c = ok(src);
        let again = ok(&c.text);
        assert_eq!(again.text, c.text, "program {n}: not a fixed point:\n{}", c.text);
        assert_eq!(again.cid, c.cid);
        let mut rng = Rng(0xa17e_0000 + n as u64);
        for round in 0..120 {
            let text = reformat(src, &mut rng);
            let got = check(&text);
            let cid = got.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{text}\n{}", render_all(&got.diagnostics, &text))).cid;
            assert_eq!(cid, c.cid, "program {n} round {round} forked:\n{text}");
        }
    }
}

// ---- the timeline

#[test]
fn stacked_sections_of_unequal_length_pad_the_shorter_with_silence() {
    // verse is 8 bars, intro 4: the intro's deck plays four bars and stops, the verse goes on
    let c = ok(&song("arrange stack(intro, verse) outro\n"));
    assert_eq!(names(&c), vec![("intro+verse".into(), 0, 4, false), ("verse".into(), 4, 4, true), ("outro".into(), 8, 2, false)]);
    assert_eq!(decks_of(&c, 0), ["A"]);
    let roles = |k: usize| -> Vec<String> { c.segments[k].decks[&Deck::A].parts.iter().map(|p| p.role.clone()).collect() };
    assert_eq!(roles(0), ["kick", "snare"], "both sections' parts of deck A, in the order they are written");
    assert_eq!(roles(1), ["snare"], "the intro is over: its kick is gone, nothing else changes");
    assert_eq!(decks_of(&c, 2), ["B"]);
    assert_eq!(c.song_bars(), Some(10));
    assert_eq!(c.end, Some(10));
    // the other way round: same song
    assert_eq!(ok(&song("arrange stack(verse, intro) outro\n")).cid, c.cid);
}

#[test]
fn a_stack_is_one_item_of_the_arrange_the_runtime_reads_as_one_section() {
    let c = ok(&song("arrange outro stack(chorus, then(intro, outro)) intro\n"));
    // 2 | chorus 8 over intro 4 + outro 2 (the chorus alone for the last 2) | 2 more: one section of 8 bars
    assert_eq!(
        names(&c),
        vec![("outro".into(), 0, 2, false), ("chorus+intro".into(), 2, 4, false), ("chorus+outro".into(), 6, 2, true), ("chorus".into(), 8, 2, true), ("intro".into(), 10, 4, false)]
    );
    assert_eq!(c.sections_at(3), ["chorus", "intro"]);
    assert_eq!(c.sections_at(9), ["chorus"]);
    assert_eq!(c.sections_at(0), ["outro"]);
    assert!(c.sections_at(99).len() == 1 && c.sections_at(-1).is_empty(), "past the end is the last segment's: the runtime ends the song there");
    let (name, bar, len) = c.section_at(3).unwrap();
    assert_eq!((name, bar, len), ("chorus+intro", 1, 8));
    let (name, bar, len) = c.section_at(9).unwrap();
    assert_eq!((name, bar, len), ("chorus", 7, 8));
    assert_eq!(c.section_at(10).unwrap().1, 0, "the next item begins a new section");
}

#[test]
fn after_before_and_end_after_follow_the_sections_in_a_stack() {
    let src = song("arrange stack(intro, verse) outro\nafter intro: master: reverb 0.3\nbefore outro 2 bars: master: delay 0.2\nend after intro\n");
    let c = ok(&src);
    let at = |key: &str| c.controls.iter().find(|k| k.key.contains(key)).map(|k| k.timing);
    assert_eq!(at("reverb"), Some(djlive::compile::Timing::At(4)), "the intro ends on bar 4 though the verse goes on");
    assert_eq!(at("delay"), Some(djlive::compile::Timing::At(6)), "the outro starts on bar 8");
    assert_eq!(c.end, Some(4));
    let c = ok(&song("arrange stack(intro, verse) outro\nafter verse: master: reverb 0.3\nend after verse\n"));
    assert_eq!(c.controls[0].timing, djlive::compile::Timing::At(8));
    assert_eq!(c.end, Some(8));
}

#[test]
fn a_section_may_be_named_more_than_once() {
    let c = ok(&song("arrange intro stack(intro, verse) intro\n"));
    assert_eq!(names(&c), vec![("intro".into(), 0, 4, false), ("intro+verse".into(), 4, 4, false), ("verse".into(), 8, 4, true), ("intro".into(), 12, 4, false)]);
    // a section named twice has no `after`: it says which is meant
    let e = err(&song("arrange intro verse intro\nafter intro: master: reverb 0.3\n"));
    assert!(e.message.contains("plays 2 times"), "{}", e.message);
}

#[test]
fn a_later_stretch_goes_on_from_where_the_section_was() {
    // the kick alternates over three bars; the stack's second stretch begins on bar 4 of an 8-bar section
    let src = "section long 8 bars {\n  deck A: kick <x ~ ~> | hihat x*4 vel=60..90\n}\nsection short 4 bars {\n  deck B: bass c2\n}\narrange stack(long, short)\n";
    let c = ok(src);
    let kick = |k: usize| &c.segments[k].decks[&Deck::A].parts.iter().find(|p| p.role == "kick").unwrap().cycles;
    let hits = |k: usize| kick(k).iter().map(|h| !h.is_empty()).collect::<Vec<_>>();
    // section bars 0..8 of <x ~ ~>: x ~ ~ x ~ ~ x ~
    assert_eq!(hits(0), [true, false, false], "bars 0 to 3 are x ~ ~ x");
    assert_eq!(hits(1), [false, false, true], "bar 4 is ~, 5 ~, 6 x, then 7 is the net's first bar again: ~");
    // and the song is what the longhand says
    let long = ok("section whole 8 bars {\n  deck A: kick <x ~ ~ x ~ ~ x ~>\n}\narrange whole\n");
    let nets = c.song_nets().unwrap();
    let song_kick = &nets[&Deck::A].parts.iter().find(|p| p.role == "kick").unwrap().cycles;
    let long_kick = &long.decks[&Deck::A].parts[0].cycles;
    assert_eq!(song_kick, long_kick);
}

#[test]
fn controls_fire_once_not_once_a_stretch() {
    let src = "section long 8 bars {\n  deck A: kick x\n  master: filter lp 300 -> 2800 over 8 bars\n  at bar 6: master: reverb 0.4\n}\nsection short 2 bars {\n  deck B: bass c2\n  at bar 1: master: delay 0.2\n}\nsection bridge 2 bars {\n  deck B: bass e2\n}\narrange stack(long, then(short, bridge, short))\n";
    let c = ok(src);
    assert_eq!(c.segments.len(), 4);
    let at = |key: &str| c.controls.iter().filter(|k| k.key.contains(key)).map(|k| k.timing).collect::<Vec<_>>();
    assert_eq!(at("filter lp"), [djlive::compile::Timing::At(0)]);
    assert_eq!(at("reverb"), [djlive::compile::Timing::At(6)]);
    assert_eq!(at("delay"), [djlive::compile::Timing::At(1), djlive::compile::Timing::At(5)], "the short section's own bar 1, both times it plays");
    // a carried copy lights its lines in every stretch it plays in
    let lines = |k: usize| c.segments[k].lines.len();
    assert!(lines(3) >= 2, "the stretch where only `long` plays still lights its lines: {}", lines(3));
}

#[test]
fn an_evolve_plays_alone() {
    let evolving = "section drift 8 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 4 bars {\n    bass: transpose 2\n    stop when gen == 1\n  }\n}\nsection other 4 bars {\n  deck B: kick x\n}\n";
    ok(&format!("{evolving}arrange other drift other\n"));
    ok(&format!("{evolving}arrange then(other, drift)\n"));
    let e = err(&format!("{evolving}arrange stack(drift, other)\n"));
    assert!(e.message.contains("evolve") && e.message.contains("drift"), "{}", e.message);
    assert_eq!(e.span.text(&format!("{evolving}arrange stack(drift, other)\n")).lines().next().unwrap(), "evolve every 4 bars {");
}

// ---- errors

#[test]
fn an_unknown_section_in_an_expression_says_where_and_what_it_might_be() {
    let src = song("arrange intro stack(vers, outro)\n");
    let e = err(&src);
    assert_eq!(e.message, "there is no section `vers`");
    let (line, col, _) = e.position(&src);
    assert_eq!((line, col), (13, 21), "the name's own place");
    assert_eq!(e.help.as_deref(), Some("did you mean `verse`?"));
    assert!(e.expected.contains(&"verse".to_string()));
    // inside a let, and inside a then in a stack
    let e = err(&song("let both = stack(intro, vers)\narrange both\n"));
    assert_eq!(e.help.as_deref(), Some("did you mean `verse`?"));
    let e = err(&song("arrange stack(intro, then(verse, choruss))\n"));
    assert_eq!(e.help.as_deref(), Some("did you mean `chorus`?"));
    // a let can be named by the nearest-name help too
    let e = err(&song("let both = stack(intro, verse)\narrange bothh\n"));
    assert_eq!(e.help.as_deref(), Some("did you mean `both`?"));
}

#[test]
fn a_let_uses_sections_and_the_lets_above_it_only() {
    let e = err(&song("let first = stack(intro, later)\nlet later = stack(verse, outro)\narrange first\n"));
    assert!(e.message.contains("`later` is a let below"), "{}", e.message);
    let e = err(&song("let loop_ = stack(intro, loop_)\narrange loop_\n"));
    assert!(e.message.contains("below"), "{}", e.message);
    // one error for the let that does not read, not one more where it is used
    let c = check(&song("let broken = stack(intro, nothing)\narrange broken\n"));
    assert_eq!(c.diagnostics.iter().filter(|d| d.is_error()).count(), 1, "{}", render_all(&c.diagnostics, ""));
}

#[test]
fn a_call_that_is_not_stack_or_then_says_so() {
    let e = err(&song("arrange stak(intro, verse)\n"));
    assert!(e.message.contains("`stak(`"), "{}", e.message);
    assert_eq!(e.help.as_deref(), Some("did you mean `stack`?"));
    let e = err(&song("arrange intro thn(verse, outro)\n"));
    assert_eq!(e.help.as_deref(), Some("did you mean `then`?"));
    let e = err(&song("arrange intro frobnicate(verse)\n"));
    assert!(e.help.as_deref().unwrap().contains("stack(a, b)"), "{e:?}");
}

#[test]
fn malformed_expressions_have_errors_with_a_place() {
    for (src, want) in [
        ("arrange stack(intro, verse\n", "expected `)` to close stack(...)"),
        ("arrange stack intro\n", "`stack` takes its sections in parentheses"),
        ("arrange stack()\n", "expected a section"),
        ("arrange stack(intro,)\n", "expected a section"),
        ("arrange then(intro verse)\n", "expected `)` to close then(...)"),
        ("arrange intro, verse\n", "expected the end of the line"),
        ("arrange\n", "`arrange` needs the sections in order"),
        ("let both = stack(intro, verse\n", "expected `)` to close stack(...)"),
    ] {
        let full = song(src);
        let e = err(&full);
        assert!(e.message.contains(want), "{src}: {}", e.message);
        let (line, col, _) = e.position(&full);
        assert!(line >= 13 && col >= 1, "{src}: {line}:{col}");
    }
}

#[test]
fn deep_nesting_is_refused_before_it_can_hurt() {
    let mut deep = "intro".to_string();
    for _ in 0..40 {
        deep = format!("stack(verse, {deep})");
    }
    let e = err(&song(&format!("arrange {deep}\n")));
    assert!(e.message.contains("nest more than 32"), "{}", e.message);
}

#[test]
fn two_sections_that_play_together_may_not_define_the_same_deck_and_role() {
    let src = "section one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck A: kick x x x x | snare ~ x\n}\narrange stack(one, two)\n";
    let e = err(src);
    assert!(e.message.contains("`one` and `two`") && e.message.contains("deck A's `kick`"), "{}", e.message);
    assert!(e.message.contains("line 2"), "the other is named by its line: {}", e.message);
    let (line, _, _) = e.position(src);
    assert_eq!(line, 5, "at the second definition");
    // one after the other they are fine
    ok("section one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck A: kick x x x x | snare ~ x\n}\narrange then(one, two)\n");
    // the same section stacked with itself
    let e = err("section one 4 bars {\n  deck A: kick x ~ x ~\n}\narrange stack(one, one)\n");
    assert!(e.message.contains("section `one` is stacked with itself"), "{}", e.message);
    // another deck, or another role, is not a clash
    ok("section one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck B: kick x x x x\n  deck A: snare ~ x\n}\narrange stack(one, two)\n");
    // and only where they overlap: a stack of a then
    ok("section one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck A: kick x x x x\n}\nsection long 8 bars {\n  deck B: bass c2\n}\narrange stack(then(one, two), long)\n");
}

#[test]
fn the_songs_own_lines_stay_under_every_stacked_section_and_the_sections_win() {
    let src = "deck A: kick x x x x | snare ~ x ~ x\nsection one 4 bars {\n  deck A: kick x ~ ~ ~\n}\nsection two 8 bars {\n  deck B: bass c2\n}\narrange stack(one, two)\n";
    let c = ok(src);
    let roles = |k: usize, d: Deck| -> Vec<(String, String)> { c.segments[k].decks[&d].parts.iter().map(|p| (p.role.clone(), p.text.clone())).collect() };
    assert_eq!(roles(0, Deck::A), [("snare".to_string(), "snare ~ x ~ x".to_string()), ("kick".to_string(), "kick x ~ ~ ~".to_string())], "the section's own kick wins, the song's snare stays (in the order written)");
    assert_eq!(roles(1, Deck::A), [("kick".to_string(), "kick x x x x".to_string()), ("snare".to_string(), "snare ~ x ~ x".to_string())], "once `one` is over the song's own kick is back");
}

// ---- over parts

#[test]
fn stack_over_parts_is_the_pipe_sorted_and_flattened() {
    let want = canon("deck A: hihat x*8 | kick x ~ x ~ | snare ~ x ~ x\n");
    for spelling in [
        "deck A: stack(kick x ~ x ~, snare ~ x ~ x, hihat x*8)",
        "deck A: stack(snare ~ x ~ x, stack(hihat x*8, kick x ~ x ~))",
        "deck A: stack(hihat x*8, stack(stack(kick x ~ x ~), snare ~ x ~ x))",
        "deck A: stack(silence, kick x ~ x ~, snare ~ x ~ x, silence, hihat x*8)",
        "deck A: stack(\n  kick x ~ x ~,\n  snare ~ x ~ x,\n  hihat x*8\n)",
    ] {
        assert_eq!(canon(&format!("{spelling}\n")), want, "{spelling}");
    }
    // a stack among `|` is sorted within itself, in place
    assert_eq!(canon("deck A: stack(kick x ~ x ~) | stack(snare ~ x ~ x, hihat x*8)\n"), "deck A: kick x ~ x ~ | hihat x*8 | snare ~ x ~ x\n");
    // a bar of `|` as it is written is not reordered: it is existing text
    assert_eq!(canon("deck A: snare ~ x ~ x | kick x ~ x ~\n"), "deck A: snare ~ x ~ x | kick x ~ x ~\n");
    // params travel with their part
    assert_eq!(canon("deck A: stack(snare ~ x vel=90, kick x oct=3)\n"), "deck A: kick x oct=3 | snare ~ x vel=90\n");
}

#[test]
fn then_over_parts_is_the_alternation_of_their_patterns() {
    let want = canon("deck A: kick <[x ~ x ~] [x x x x] ~>\n");
    for spelling in [
        "deck A: then(kick x ~ x ~, kick x x x x, kick ~)",
        "deck A: then(kick x ~ x ~, then(kick x x x x, kick ~))",
        "deck A: then(then(kick x ~ x ~, kick x x x x), kick ~)",
        "deck A: then(then(kick x ~ x ~), then(kick x x x x, silence, kick ~))",
    ] {
        assert_eq!(canon(&format!("{spelling}\n")), want, "{spelling}");
    }
    // not commutative
    assert_ne!(canon("deck A: then(kick x, kick ~)\n"), canon("deck A: then(kick ~, kick x)\n"));
    assert_eq!(canon("deck A: then(kick x, kick ~)\n"), "deck A: kick <x ~>\n");
    // the alternation is a bar of each: the bars it plays are the patterns in turn
    let c = ok("deck A: then(kick x ~ x ~, kick x x x x, kick ~)\n");
    let cycles = &c.decks[&Deck::A].parts[0].cycles;
    let steps: Vec<Vec<u8>> = cycles.iter().map(|b| b.iter().map(|h| h.step).collect()).collect();
    assert_eq!(steps, vec![vec![0, 8], vec![0, 4, 8, 12], vec![]]);
    // an operand's own alternation advances once each time it comes round
    let c = ok("deck A: then(bass <c2 e2>, bass g2)\n");
    let notes: Vec<Vec<u8>> = c.decks[&Deck::A].parts[0].cycles.iter().map(|b| b.iter().map(|h| h.note).collect()).collect();
    assert_eq!(notes, vec![vec![36], vec![43], vec![40], vec![43]], "c2, g2, e2, g2");
    // params are the operands' own and must agree
    assert_eq!(canon("deck A: then(bass c2 oct=3 vel=90, bass e2 vel=90 oct=3)\n"), "deck A: bass <c2 e2> oct=3 vel=90\n");
}

#[test]
fn then_over_parts_needs_one_role_and_one_set_of_params() {
    let e = err("deck A: then(kick x, snare x)\n");
    assert!(e.message.contains("`snare` is not `kick`"), "{}", e.message);
    assert_eq!(e.span.text("deck A: then(kick x, snare x)\n"), "snare");
    let src = "deck A: then(bass c2 vel=90, bass e2)\n";
    let e = err(src);
    assert!(e.message.contains("share their parameters") && e.message.contains("`vel=90`") && e.message.contains("none"), "{}", e.message);
    assert_eq!(e.position(src).0, 1);
    let e = err("deck A: then(stack(kick x, snare x), stack(kick x, snare ~))\n");
    assert!(e.message.contains("stack of several"), "{}", e.message);
    let e = err("deck A: silence\n");
    assert!(e.message.contains("`silence` is not a part"), "{}", e.message);
    let e = err("deck A: stack(kick x, bsss x)\n");
    assert!(e.message.contains("expected a role"), "{}", e.message);
    // a time word cannot hold a part, an expression or not; a deck is needed
    let e = err("at bar 4: deck A: stack(kick x, snare x)\n");
    assert!(e.message.contains("time word"), "{}", e.message);
    let e = err("stack(kick x, snare x)\n");
    assert!(e.message.contains("needs a deck"), "{}", e.message);
}

#[test]
fn part_expressions_are_read_inside_a_def_and_a_repeat_too() {
    let src = "def groove(deck) {\n  deck $deck: stack(kick x ~ x ~, then(snare ~ x, snare x ~))\n}\ngroove(A)\nrepeat 1 {\n  deck B: then(bass c2, bass e2)\n}\n";
    let c = ok(src);
    assert!(c.decks.contains_key(&Deck::A));
    assert_eq!(c.decks[&Deck::A].parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["kick x ~ x ~", "snare <[~ x] [x ~]>"]);
}

#[test]
fn the_words_are_highlighted_as_keywords() {
    use djlive::highlight::{highlight, Class};
    let src = "arrange stack(then(a1, silence), b1)\n";
    let classes: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    for w in ["stack", "then", "silence"] {
        assert!(classes.contains(&(w, Class::Keyword)), "{w}: {classes:?}");
    }
}

// ---- the laws, as properties
//
// A random composition over eight sections (each leaf used once, so nothing clashes) is rewritten by the laws and
// must keep its CID; rewritten by a non-law it must not. An independent oracle (placing the tree by hand) writes the
// song out longhand, and the compiled song must be that longhand's nets.

struct Sec {
    name: &'static str,
    bars: u32,
    deck: &'static str,
    role: &'static str,
    pattern: &'static str,
    params: &'static str,
    /// What the section plays on its bar `k`, as a term of a longhand alternation (cyclic).
    terms: &'static [&'static str],
}

const POOL: [Sec; 8] = [
    Sec { name: "intro", bars: 4, deck: "A", role: "kick", pattern: "x ~ x ~", params: "", terms: &["[x ~ x ~]"] },
    Sec { name: "verse", bars: 8, deck: "A", role: "snare", pattern: "~ x ~ x", params: "", terms: &["[~ x ~ x]"] },
    Sec { name: "chorus", bars: 8, deck: "A", role: "hihat", pattern: "x*8", params: " vel=60..90", terms: &["[x*8]"] },
    Sec { name: "bridge", bars: 4, deck: "A", role: "clap", pattern: "~ x ~ ~", params: "", terms: &["[~ x ~ ~]"] },
    Sec { name: "outro", bars: 2, deck: "B", role: "bass", pattern: "c2 ~ e2 ~", params: "", terms: &["[c2 ~ e2 ~]"] },
    Sec { name: "fill", bars: 6, deck: "B", role: "melody", pattern: "<c4 e4 g4>", params: "", terms: &["c4", "e4", "g4"] },
    Sec { name: "lift", bars: 5, deck: "B", role: "harmony", pattern: "<chord(am) chord(f)>", params: "", terms: &["chord(am)", "chord(f)"] },
    Sec { name: "riser", bars: 8, deck: "B", role: "arp", pattern: "arp(am7, up, 8)", params: "", terms: &["arp(am7, up, 8)"] },
];

fn defs() -> String {
    POOL.iter().map(|s| format!("section {} {} bars {{\n  deck {}: {} {}{}\n}}\n", s.name, s.bars, s.deck, s.role, s.pattern, s.params)).collect()
}

#[derive(Clone, Debug)]
enum T {
    Leaf(usize),
    Silence,
    Stack(Vec<T>),
    Then(Vec<T>),
}

fn gen(rng: &mut Rng, pool: &mut Vec<usize>, n: usize) -> T {
    let mut t = if n == 1 {
        T::Leaf(pool.pop().expect("a leaf left"))
    } else {
        let k = 2 + rng.below((n.min(4) - 1) as u64) as usize;
        let mut sizes = vec![1usize; k];
        for _ in 0..(n - k) {
            let j = rng.below(k as u64) as usize;
            sizes[j] += 1;
        }
        let kids: Vec<T> = sizes.iter().map(|&s| gen(rng, pool, s)).collect();
        if rng.below(2) == 0 {
            T::Stack(kids)
        } else {
            T::Then(kids)
        }
    };
    if rng.below(6) == 0 {
        t = if rng.below(2) == 0 { T::Stack(vec![t]) } else { T::Then(vec![t]) };
    }
    t
}

/// A random tree of 1 to 7 leaves of the pool.
fn tree(rng: &mut Rng) -> T {
    let mut pool: Vec<usize> = (0..POOL.len()).collect();
    for i in (1..pool.len()).rev() {
        pool.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let n = 1 + rng.below(7) as usize;
    gen(rng, &mut pool, n)
}

fn render(t: &T, rng: &mut Rng, top: bool) -> String {
    let seps = [", ", ",", " , ", ",\n    "];
    match t {
        T::Leaf(i) => POOL[*i].name.to_string(),
        T::Silence => "silence".into(),
        T::Stack(xs) => format!("stack({})", xs.iter().map(|x| render(x, rng, false)).collect::<Vec<_>>().join(seps[rng.below(4) as usize])),
        T::Then(xs) => {
            if top && rng.below(2) == 0 {
                xs.iter().map(|x| render(x, rng, false)).collect::<Vec<_>>().join(" ")
            } else {
                format!("then({})", xs.iter().map(|x| render(x, rng, false)).collect::<Vec<_>>().join(seps[rng.below(4) as usize]))
            }
        }
    }
}

fn program(t: &T, rng: &mut Rng) -> String {
    format!("{}arrange {}\n", defs(), render(t, rng, true))
}

/// Group a window of the operands into a nested node of the same kind (associativity).
fn reassociate(t: &T, rng: &mut Rng) -> T {
    fn regroup(xs: &[T], rng: &mut Rng, make: fn(Vec<T>) -> T) -> Vec<T> {
        let mut ys: Vec<T> = xs.iter().map(|x| reassociate(x, rng)).collect();
        if ys.len() >= 3 && rng.below(3) != 0 {
            let len = 2 + rng.below((ys.len() - 2) as u64) as usize;
            let start = rng.below((ys.len() - len + 1) as u64) as usize;
            let group: Vec<T> = ys.drain(start..start + len).collect();
            ys.insert(start, make(group));
        }
        ys
    }
    match t {
        T::Then(xs) => T::Then(regroup(xs, rng, T::Then)),
        T::Stack(xs) => T::Stack(regroup(xs, rng, T::Stack)),
        other => other.clone(),
    }
}

/// Insert `silence` where the laws let it go: as operands, and around a node with `then` or `stack`.
fn units(t: &T, rng: &mut Rng) -> T {
    let mut t2 = match t {
        T::Stack(xs) => T::Stack(xs.iter().map(|x| units(x, rng)).collect()),
        T::Then(xs) => T::Then(xs.iter().map(|x| units(x, rng)).collect()),
        other => other.clone(),
    };
    if let T::Stack(xs) | T::Then(xs) = &mut t2 {
        for _ in 0..rng.below(3) {
            let at = rng.below(xs.len() as u64 + 1) as usize;
            xs.insert(at, T::Silence);
        }
    }
    match rng.below(7) {
        0 => T::Then(vec![t2, T::Silence]),
        1 => T::Then(vec![T::Silence, t2]),
        2 => T::Stack(vec![T::Silence, t2]),
        3 => T::Stack(vec![t2, T::Silence]),
        4 => T::Then(vec![t2]),
        5 => T::Stack(vec![t2]),
        _ => t2,
    }
}

/// Shuffle the operands of every `stack` (symmetry).
fn permute(t: &T, rng: &mut Rng) -> T {
    match t {
        T::Stack(xs) => {
            let mut ys: Vec<T> = xs.iter().map(|x| permute(x, rng)).collect();
            for i in (1..ys.len()).rev() {
                ys.swap(i, rng.below(i as u64 + 1) as usize);
            }
            T::Stack(ys)
        }
        T::Then(xs) => T::Then(xs.iter().map(|x| permute(x, rng)).collect()),
        other => other.clone(),
    }
}

fn leaves(t: &T, out: &mut Vec<usize>) {
    match t {
        T::Leaf(i) => out.push(*i),
        T::Silence => {}
        T::Stack(xs) | T::Then(xs) => xs.iter().for_each(|x| leaves(x, out)),
    }
}

/// The oracle: where each leaf plays, and how many bars the tree is.
fn place(t: &T, at: u32, out: &mut Vec<(usize, u32)>) -> u32 {
    match t {
        T::Leaf(i) => {
            out.push((*i, at));
            POOL[*i].bars
        }
        T::Silence => 0,
        T::Then(xs) => {
            let mut acc = 0;
            for x in xs {
                acc += place(x, at + acc, out);
            }
            acc
        }
        T::Stack(xs) => xs.iter().map(|x| place(x, at, out)).max().unwrap_or(0),
    }
}

fn longhand(t: &T) -> (String, u32) {
    let mut placed = Vec::new();
    let total = place(t, 0, &mut placed);
    let mut text = String::new();
    for deck in ["A", "B"] {
        let mut parts = Vec::new();
        for (i, s) in POOL.iter().enumerate() {
            let Some(&(_, start)) = placed.iter().find(|(l, _)| *l == i) else { continue };
            if s.deck != deck {
                continue;
            }
            let terms: Vec<String> = (0..total)
                .map(|bar| {
                    if bar >= start && bar < start + s.bars {
                        s.terms[((bar - start) as usize) % s.terms.len()].to_string()
                    } else {
                        "~".to_string()
                    }
                })
                .collect();
            parts.push(format!("{} <{}>{}", s.role, terms.join(" "), s.params));
        }
        if !parts.is_empty() {
            text.push_str(&format!("deck {deck}: {}\n", parts.join(" | ")));
        }
    }
    (text, total)
}

const ROUNDS: usize = 160;

#[test]
fn property_re_association_unit_insertion_and_stack_permutation_keep_the_cid() {
    let mut rng = Rng(0x0f10_5eed);
    let mut tested = 0;
    for round in 0..ROUNDS {
        let t = tree(&mut rng);
        let base = program(&t, &mut rng);
        let want = ok(&base);
        let once = reassociate(&t, &mut rng);
        let variants = [
            ("reassociated", reassociate(&once, &mut rng)),
            ("with units", units(&t, &mut rng)),
            ("permuted", permute(&t, &mut rng)),
            ("all three", permute(&units(&reassociate(&t, &mut rng), &mut rng), &mut rng)),
        ];
        for (what, v) in variants {
            let text = program(&v, &mut rng);
            let text = reformat(&text, &mut rng);
            let got = ok(&text);
            assert_eq!(got.text, want.text, "round {round}, {what}:\n{base}\n--- became\n{text}");
            assert_eq!(got.cid, want.cid, "round {round}, {what}");
            tested += 1;
        }
    }
    assert_eq!(tested, ROUNDS * 4);
}

/// The `then` nodes of a tree with at least two operands that are not `silence`, as paths of operand indices.
fn then_paths(t: &T, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    match t {
        T::Then(xs) => {
            if xs.iter().filter(|x| !matches!(x, T::Silence)).count() >= 2 {
                out.push(path.clone());
            }
            for (i, x) in xs.iter().enumerate() {
                path.push(i);
                then_paths(x, path, out);
                path.pop();
            }
        }
        T::Stack(xs) => {
            for (i, x) in xs.iter().enumerate() {
                path.push(i);
                then_paths(x, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn node_at<'a>(t: &'a mut T, path: &[usize]) -> &'a mut T {
    match path.split_first() {
        None => t,
        Some((i, rest)) => match t {
            T::Then(xs) | T::Stack(xs) => node_at(&mut xs[*i], rest),
            _ => unreachable!(),
        },
    }
}

#[test]
fn property_swapping_two_different_then_operands_changes_the_cid() {
    let mut rng = Rng(0x05a9_0f10);
    let mut tested = 0;
    for round in 0..ROUNDS * 2 {
        let t = tree(&mut rng);
        let mut paths = Vec::new();
        then_paths(&t, &mut Vec::new(), &mut paths);
        if paths.is_empty() {
            continue;
        }
        let path = paths[rng.below(paths.len() as u64) as usize].clone();
        let mut swapped = t.clone();
        if let T::Then(xs) = node_at(&mut swapped, &path) {
            let idx: Vec<usize> = (0..xs.len()).filter(|&i| !matches!(xs[i], T::Silence)).collect();
            let a = idx[rng.below(idx.len() as u64) as usize];
            let mut b = idx[rng.below(idx.len() as u64) as usize];
            while b == a {
                b = idx[rng.below(idx.len() as u64) as usize];
            }
            xs.swap(a, b);
        }
        let before = ok(&program(&t, &mut rng));
        let after = ok(&program(&swapped, &mut rng));
        assert_ne!(before.cid, after.cid, "round {round}: swapping two operands of a then left the CID alone\n{}\n{}", tail(&before.text), tail(&after.text));
        tested += 1;
    }
    assert!(tested >= 100, "only {tested} swaps tried");
}

/// A song's segments as (name, start, bars, chain, the net CID of each deck).
type Layout = Vec<(String, u32, u32, bool, Vec<(&'static str, String)>)>;

fn layout(c: &Compiled) -> Layout {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.chain, s.decks.iter().map(|(d, ir)| (d.name(), ir.net_cid())).collect())).collect()
}

#[test]
fn property_stack_a_b_and_stack_b_a_have_one_cid_and_one_compiled_timeline() {
    let mut rng = Rng(0x0b0a_7f10);
    for round in 0..ROUNDS {
        let t = tree(&mut rng);
        let p = permute(&t, &mut rng);
        let a = ok(&program(&t, &mut rng));
        let b = ok(&program(&p, &mut rng));
        assert_eq!(a.cid, b.cid, "round {round}");
        assert_eq!(layout(&a), layout(&b), "round {round}: the compiled timelines differ");
        assert_eq!(a.controls, b.controls);
        assert_eq!(a.song_net_cids(), b.song_net_cids());
    }
    // the two-operand case by hand, with lines to light
    let x = ok(&song("arrange stack(intro, verse)\n"));
    let y = ok(&song("arrange stack(verse, intro)\n"));
    assert_eq!(layout(&x), layout(&y));
}

#[test]
fn property_arrange_a_b_c_is_arrange_then_a_b_c() {
    let mut rng = Rng(0x00c0_ffee_0f10);
    for round in 0..ROUNDS {
        // a plain list may name a section again: any names in any order
        let n = 1 + rng.below(6) as usize;
        let list: Vec<&str> = (0..n).map(|_| POOL[rng.below(POOL.len() as u64) as usize].name).collect();
        let plain = format!("{}arrange {}\n", defs(), list.join(" "));
        let want = ok(&plain);
        // left-nested, or right-nested: then(a, then(b, then(c ...)))
        let mut nested = list[0].to_string();
        for x in &list[1..] {
            nested = format!("then({nested}, {x})");
        }
        let mut right = list[list.len() - 1].to_string();
        for x in list[..list.len() - 1].iter().rev() {
            right = format!("then({x}, {right})");
        }
        for spelling in [format!("then({})", list.join(", ")), nested, right] {
            let got = ok(&format!("{}arrange {spelling}\n", defs()));
            assert_eq!(got.text, want.text, "round {round}: {spelling}");
            assert_eq!(got.cid, want.cid);
            assert_eq!(layout(&got), layout(&want));
            assert_eq!(got.controls, want.controls);
        }
        // and what a plain arrange makes: one segment a name, in order, each section's own bars
        let mut at = 0;
        let names: Vec<(String, u32, u32, bool)> = list
            .iter()
            .map(|n| {
                let s = POOL.iter().find(|s| s.name == *n).unwrap();
                let r = (n.to_string(), at, s.bars, false);
                at += s.bars;
                r
            })
            .collect();
        assert_eq!(self::names(&want), names, "round {round}");
    }
}

#[test]
fn property_the_compiled_song_is_the_hand_written_longhand() {
    let mut rng = Rng(0x0010_9a2d);
    let mut stacks = 0;
    for round in 0..ROUNDS {
        let t = tree(&mut rng);
        let text = program(&t, &mut rng);
        let c = ok(&text);
        let (long, total) = longhand(&t);
        assert_eq!(c.song_bars(), Some(total), "round {round}:\n{}", tail(&text));
        let want = ok(&long).net_cids();
        let got = c.song_net_cids().unwrap_or_else(|e| panic!("round {round}: {e}"));
        assert_eq!(got, want, "round {round}: the song and its longhand differ\n{}\n{long}", tail(&text));
        // the segments cut the song exactly where a leaf starts or ends, and cover it
        let mut placed = Vec::new();
        place(&t, 0, &mut placed);
        let mut cuts: Vec<u32> = placed.iter().flat_map(|(i, s)| [*s, s + POOL[*i].bars]).collect();
        cuts.sort();
        cuts.dedup();
        let seg_cuts: Vec<u32> = c.segments.iter().map(|s| s.start).chain(c.segments.last().map(|s| s.start + s.bars)).collect();
        for cut in &cuts {
            assert!(seg_cuts.contains(cut), "round {round}: no segment edge on bar {cut}: {seg_cuts:?}");
        }
        assert_eq!(c.segments.iter().map(|s| s.bars).sum::<u32>(), total);
        // each segment plays exactly the decks of the leaves that cover it
        for s in &c.segments {
            let mut want_decks: Vec<&str> = placed.iter().filter(|(i, st)| *st <= s.start && s.start < st + POOL[*i].bars).map(|(i, _)| POOL[*i].deck).collect();
            want_decks.sort();
            want_decks.dedup();
            assert_eq!(s.decks.keys().map(|d| d.name()).collect::<Vec<_>>(), want_decks, "round {round}, bar {}", s.start);
        }
        if c.segments.iter().any(|s| s.name.contains('+')) {
            stacks += 1;
        }
    }
    assert!(stacks > ROUNDS / 4, "only {stacks} of {ROUNDS} programs stacked anything");
}

#[test]
fn the_oracle_agrees_with_itself_leaves_are_unique() {
    let mut rng = Rng(7);
    for _ in 0..50 {
        let t = tree(&mut rng);
        let mut l = Vec::new();
        leaves(&t, &mut l);
        let mut m = l.clone();
        m.sort();
        m.dedup();
        assert_eq!(l.len(), m.len());
    }
}

// ---- on a fake host and a fake clock

/// Four beats a bar. The fake deck plays when told, and "installs" a net on the next bar line.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, net cid, the bar it lands on)
    loads: Vec<(String, String, i64)>,
    /// (deck, the bar it was told to stop on)
    pauses: Vec<(String, i64)>,
    /// (beat, noun, params)
    calls: Vec<(f64, String, String)>,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }

    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), djengine::cid::share_cid(env), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
        if !on {
            self.pauses.push((deck.into(), (self.beat / 4.0).floor() as i64));
        }
    }

    fn set_sound(&mut self, _deck: &str, _role: &str, _option: &str) -> Result<(), String> {
        Ok(())
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    /// Run until the song is on its bar number `bar` (0 is its first), or has ended.
    fn run_to_song_bar(&mut self, live: &mut Live, bar: i64) {
        for _ in 0..4000 {
            let st = live.status();
            if st.get("song_bar").and_then(|v| v.as_f64()).is_some_and(|b| b as i64 > bar) {
                return;
            }
            if st.get("ended").and_then(|v| v.as_bool()) == Some(true) {
                return;
            }
            self.run(live, 0.25);
        }
        panic!("never got to song bar {bar}");
    }
}

fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

const STACKED: &str = "\
section lead 8 bars {
  deck A: kick x ~ x ~
  at bar 2: master: reverb 0.4
}
section tail 4 bars {
  deck B: bass c2
  at bar 1: master: delay 0.3
}
section last 4 bars {
  deck A: kick x x x x
}
arrange stack(lead, tail) last
";

#[test]
fn the_shorter_of_two_stacked_sections_stops_its_deck_where_it_ends_and_nothing_else_reloads() {
    let (mut h, mut live) = playing();
    let ev = live.eval(STACKED);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    assert_eq!(ev.nets.len(), 4, "a net a deck a stretch: {:?}", ev.nets.keys().collect::<Vec<_>>());
    h.run_to_song_bar(&mut live, 1);
    assert_eq!(live.section().map(|s| s.0).as_deref(), Some("lead+tail"));
    assert!(h.playing[0] && h.playing[1], "both decks play in the first four bars");
    let loads_a = h.loads.iter().filter(|l| l.0 == "A").count();
    h.run_to_song_bar(&mut live, 5);
    assert_eq!(live.section().map(|s| s.0).as_deref(), Some("lead"));
    assert_eq!(live.section().map(|s| (s.1, s.2)), Some((5, 8)), "bar 5 of the one 8-bar section the two stretches make");
    assert!(!h.playing[1], "the tail is over: its deck B stopped {:?}", h.pauses);
    assert_eq!(h.pauses.iter().filter(|p| p.0 == "B").count(), 1);
    assert_eq!(h.loads.iter().filter(|l| l.0 == "A").count(), loads_a, "the lead's kick is the same net across the boundary: it plays on");
    h.run_to_song_bar(&mut live, 9);
    assert_eq!(live.section().map(|s| s.0).as_deref(), Some("last"));
    assert_eq!(h.loads.iter().filter(|l| l.0 == "A").count(), loads_a + 1, "the last section's kick is a new net");
    // the controls count from each section's own start, once each
    let at = |noun: &str| -> Vec<f64> { h.calls.iter().filter(|c| c.1 == "inserts" && c.2.contains("\"set\"") && c.2.contains(noun)).map(|c| (c.0 / 4.0).floor()).collect() };
    let (reverb, delay) = (at("reverb"), at("delay"));
    assert_eq!((reverb.len(), delay.len()), (1, 1), "once each, not once a stretch: {:?}", h.calls);
    assert_eq!(reverb[0] - delay[0], 1.0, "bar 2 of the lead and bar 1 of the tail");
    // the song ends after the last section: 8 + 4 bars
    h.run_to_song_bar(&mut live, 13);
    assert!(!live.is_running());
}

#[test]
fn the_status_and_the_marks_follow_a_stack_through_its_stretches() {
    let (mut h, mut live) = playing();
    assert!(live.eval(STACKED).ok);
    h.run_to_song_bar(&mut live, 2);
    let m = live.marks();
    let (header, progress) = m.header.expect("a header");
    assert!(header.text(&m.source).starts_with("section lead 8 bars"), "the playhead sits on the longest section: {}", header.text(&m.source));
    assert!(progress > 0.2 && progress < 0.4, "bar 2 of 8: {progress}");
    let lit = |m: &djlive::runtime::Marks, start: &str| m.active_lines.iter().any(|s| s.text(&m.source).starts_with(start));
    assert!(lit(&m, "deck A: kick x ~ x ~") && lit(&m, "deck B: bass c2"), "the lines of both sections are playing");
    assert!(lit(&m, "section tail 4 bars"), "the other section's header is lit as a line");
    h.run_to_song_bar(&mut live, 6);
    let m = live.marks();
    let (header, progress) = m.header.unwrap();
    assert!(header.text(&m.source).starts_with("section lead 8 bars"));
    assert!(progress > 0.7 && progress < 0.85, "the playhead goes on across the boundary: {progress}");
    assert!(lit(&m, "deck A: kick x ~ x ~"), "the lead's lines still play");
    assert!(!lit(&m, "deck B: bass c2") && !lit(&m, "section tail"), "the tail's do not");
    let st = live.status();
    assert_eq!(st.get("section").and_then(|v| v.as_str()), Some("lead"));
    assert_eq!(st.get("song_bars").and_then(|v| v.as_f64()), Some(12.0));
}

#[test]
fn a_hit_of_a_stacked_section_lights_its_own_token_in_the_text() {
    let (mut h, mut live) = playing();
    let src = "section lead 8 bars {\n  deck A: kick x ~ x ~\n}\nsection tail 4 bars {\n  deck B: bass c2 ~ e2 ~\n}\narrange stack(tail, lead)\n";
    assert!(live.eval(src).ok);
    let mut seen: Vec<(String, String)> = Vec::new();
    for _ in 0..(4 * 4 * 14) {
        h.run(&mut live, 0.25);
        for f in live.fires() {
            let key = (f.role.clone(), f.spans[0].text(src).to_string());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    }
    assert!(seen.contains(&("kick".to_string(), "x".to_string())), "{seen:?}");
    assert!(seen.contains(&("bass".to_string(), "c2".to_string())) && seen.contains(&("bass".to_string(), "e2".to_string())), "{seen:?}");
    let c = ok(src);
    // every hit of the part carries the chain up to its section's own line
    let kick = &c.segments[0].decks[&Deck::A].parts[0];
    let h0 = kick.cycles[0][0];
    let chain: Vec<&str> = kick.spans[h0.sid as usize].iter().map(|s| s.text(src)).collect();
    assert_eq!(chain[0], "x");
    assert!(chain.contains(&"deck A: kick x ~ x ~"), "{chain:?}");
    // the same hit in the carried stretch points at the same tokens
    let later = &c.segments[1].decks[&Deck::A].parts[0];
    let h1 = later.cycles[0][0];
    let chain1: Vec<&str> = later.spans[h1.sid as usize].iter().map(|s| s.text(src)).collect();
    assert_eq!(chain, chain1);
}

#[test]
fn an_edit_to_one_stacked_section_reloads_only_what_it_changes() {
    let (mut h, mut live) = playing();
    assert!(live.eval(STACKED).ok);
    h.run_to_song_bar(&mut live, 2);
    let edited = STACKED.replace("bass c2", "bass e2");
    let ev = live.eval(&edited);
    assert!(ev.ok);
    let lines = ev.plan.as_ref().unwrap().lines();
    assert!(lines.iter().any(|l| l.contains("lead+tail@0") && l.contains("bass changed")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("last")), "the section after is the same net: {lines:?}");
}

// ---- what the laws do not say

#[test]
fn the_interchange_law_holds_for_the_music_but_is_not_normalised() {
    // stack(then(a, b), then(c, d)) plays what then(stack(a, c), stack(b, d)) plays when |a| = |c|
    let defs = defs();
    let one = ok(&format!("{defs}arrange stack(then(intro, outro), then(bridge, fill))\n"));
    let two = ok(&format!("{defs}arrange then(stack(intro, bridge), stack(outro, fill))\n"));
    assert_ne!(one.cid, two.cid, "different programs");
    assert_eq!(one.song_net_cids(), two.song_net_cids(), "the same song, bar for bar");
    assert_eq!(one.song_bars(), two.song_bars());
    // when the lengths differ (|intro| = 4, |outro| = 2) the two are different music
    let three = ok(&format!("{defs}arrange then(stack(intro, outro), stack(bridge, fill))\n"));
    let four = ok(&format!("{defs}arrange stack(then(intro, bridge), then(outro, fill))\n"));
    assert_ne!(three.song_net_cids(), four.song_net_cids());
}

#[test]
fn a_section_cut_in_two_is_not_the_section() {
    // every section has its own clock: `at bar 4` counts from its first bar
    let whole = ok("section whole 8 bars {\n  deck A: kick x\n  at bar 4: master: reverb 0.4\n}\narrange whole\n");
    let cut = ok("section first 4 bars {\n  deck A: kick x\n  at bar 4: master: reverb 0.4\n}\nsection second 4 bars {\n  deck A: kick x\n}\narrange then(first, second)\n");
    assert_ne!(whole.cid, cut.cid);
    assert_eq!(cut.controls[0].timing, djlive::compile::Timing::At(4));
}

#[test]
fn stack_takes_no_parameters_of_its_own_and_belongs_where_a_part_does() {
    let e = err("deck A: stack(kick x, snare x) oct=2\n");
    assert!(e.message.contains("expected the end of the line"), "{}", e.message);
    // arrange and a composition's let are statements of the top level, as arrange and let always were
    let e = err(&song("repeat 1 {\n  arrange stack(intro, verse)\n}\n"));
    assert!(e.message.contains("`arrange` belongs at the top"), "{}", e.message);
    let e = err(&song("repeat 1 {\n  let both = stack(intro, verse)\n}\n"));
    assert!(e.message.contains("`let` belongs at the top"), "{}", e.message);
    // two arranges are still one too many
    let e = err(&song("arrange stack(intro, verse)\narrange outro\n"));
    assert!(e.message.contains("one `arrange`"), "{}", e.message);
}

#[test]
fn sections_not_in_the_arrange_are_said_not_to_play_even_when_a_let_hides_them() {
    let c = check(&song("let both = stack(intro, verse)\narrange both\n"));
    let never: Vec<String> = c.diagnostics.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect();
    assert_eq!(never.len(), 2, "{never:?}");
    assert!(never.iter().any(|m| m.contains("section `chorus`")) && never.iter().any(|m| m.contains("section `outro`")), "{never:?}");
    assert!(!never.iter().any(|m| m.contains("`intro`") || m.contains("`verse`")), "{never:?}");
}

#[test]
fn a_composition_of_the_same_section_twice_in_a_row_is_the_song_written_twice() {
    let doubled = ok(&song("arrange then(intro, intro)\n"));
    let listed = ok(&song("arrange intro intro\n"));
    assert_eq!(doubled.cid, listed.cid);
    assert_eq!(names(&doubled), vec![("intro".into(), 0, 4, false), ("intro".into(), 4, 4, false)]);
}

#[test]
fn a_long_plain_arrange_stays_cheap() {
    // 4,000 bars of one section: the timeline is built in one sweep, not by comparing every pair
    let names = vec!["intro"; 4000].join(" ");
    let t = std::time::Instant::now();
    let c = ok(&song(&format!("arrange {names}\n")));
    assert_eq!(c.segments.len(), 4000);
    assert_eq!(c.song_bars(), Some(16000));
    let took = t.elapsed();
    assert!(took.as_secs_f64() < 20.0, "{took:?}");
    eprintln!("4,000 sections compiled in {took:?}");
}

#[test]
fn lets_that_double_each_other_hit_a_limit_not_the_end_of_memory() {
    // 2^24 placements if it were let go on: it is stopped at 16,384 and says so
    let mut src = "section hush 4 bars {\n  master: filter lp 800\n}\nsection swell 4 bars {\n  master: reverb 0.4\n}\nlet s1 = stack(hush, swell)\n".to_string();
    for k in 2..=24 {
        src.push_str(&format!("let s{k} = stack(s{}, s{})\n", k - 1, k - 1));
    }
    src.push_str("arrange s24\n");
    let t = std::time::Instant::now();
    let c = check(&src);
    assert!(!c.ok());
    assert!(c.diagnostics.iter().any(|d| d.message.contains("places sections more than 16384 times")), "{}", render_all(&c.diagnostics, &src));
    assert!(t.elapsed().as_secs_f64() < 20.0, "{:?}", t.elapsed());
}

#[test]
fn the_songs_own_alternations_go_on_across_the_stretches_of_one_stack() {
    // a global `<x ~ x>` restarts at each item of the arrange (as it always did at each section), not at each stretch
    let src = "deck A: hihat <x ~ x>\nsection long 8 bars {\n  deck B: bass c2\n}\nsection short 4 bars {\n  deck B: melody c4\n}\narrange stack(long, short)\n";
    let c = ok(src);
    assert_eq!(c.segments.len(), 2);
    let hat = |k: usize| -> Vec<bool> { c.segments[k].decks[&Deck::A].parts[0].cycles.iter().map(|b| !b.is_empty()).collect() };
    assert_eq!(hat(0), [true, false, true]);
    assert_eq!(hat(1), [false, true, true], "bar 4 of the item is the 5th of x ~ x x ~ x x ~: ~, x, x");
    let long = ok("deck A: hihat <x ~ x x ~ x x ~>\nsection whole 8 bars {\n  deck B: bass c2\n}\narrange whole\n");
    let nets = c.song_nets().unwrap();
    let hats = &nets[&Deck::A].parts[0].cycles;
    assert_eq!(hats, &long.song_nets().unwrap()[&Deck::A].parts[0].cycles);
    // and at the next item it starts again from its first bar
    let again = ok(&format!("{}arrange stack(long, short) long\n", &src[..src.len() - "arrange stack(long, short)\n".len()]));
    let third = &again.segments[2].decks[&Deck::A].parts[0].cycles;
    assert_eq!(third.iter().map(|b| !b.is_empty()).collect::<Vec<_>>(), [true, false, true]);
}

#[test]
fn an_unclosed_paren_is_said_where_the_line_ends_and_the_next_statement_still_reads() {
    let src = song("arrange stack(intro, verse\nsection extra 2 bars {\n  deck B: bass c2\n}\n");
    let c = check(&src);
    let es: Vec<&Diagnostic> = c.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(es.len(), 1, "one error, not a cascade: {}", render_all(&c.diagnostics, &src));
    let (line, col, _) = es[0].position(&src);
    assert_eq!(line, 13, "on the arrange's own line");
    assert_eq!(col, "arrange stack(intro, verse".len() + 1, "at its end");
    assert!(es[0].message.contains("expected `)` to close stack(...), found the end of the line"), "{}", es[0].message);
    // the same over parts
    let e = err("deck A: stack(kick x, snare x\ndeck B: bass c2\n");
    assert!(e.message.contains("expected `)` to close stack(...)"), "{}", e.message);
    assert_eq!(e.position("deck A: stack(kick x, snare x\ndeck B: bass c2\n").0, 1);
}

#[test]
fn what_is_read_back_from_the_canonical_text_is_the_same_text() {
    for src in [
        "deck A: then(bass <c2 e2>, bass g2 ~, bass x!*4, bass [c2, e2]) oct=2\n".replace(" oct=2", ""),
        "deck A: stack(then(kick x, kick ~), then(snare ~ x, snare x ~, snare ~), hihat x*8 vel=60..90)\n".into(),
        "deck B: then(bass c2 ~ e2^ g2!, bass e2 oct=3, silence)\n".replace(" oct=3", ""),
    ] {
        let c = ok(&src);
        assert_eq!(ok(&c.text).text, c.text, "{}", c.text);
        assert_eq!(ok(&c.text).cid, c.cid);
    }
}
