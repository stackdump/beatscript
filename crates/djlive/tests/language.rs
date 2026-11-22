//! Parser goldens (the examples of docs/BTW.md, canonical text and its fixed point, error
//! cases with their exact line, column and message), mini-notation, and the compile: pattern text
//! to the net's exact shape, the same text the same CIDs.

use djengine::json::JsonValue;
use djlive::ast::{Deck, StmtKind};
use djlive::compile::{check, Effect, Timing};
use djlive::parse::parse;
use djlive::print::canonical;

const EXAMPLE: &str = "\
tempo 126
key am

deck A: kick e(5,8) | snare ~ x ~ x | bass markov(am, 0.7) | hihat x*8 vel=60..90
deck B: harmony chord(am7) | arp arp(am7, up, 8)

every 16 bars: swap pairs with echo_out
on phrase: deck A: filter lp 12k -> 400 over 4 bars   # a build, every 8 bars
master: filter lp 2k -> 8k over 8 bars
";

fn errors(src: &str) -> Vec<(usize, usize, String)> {
    let c = check(src);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| {
        let (l, col, _) = d.position(src);
        (l, col, d.message.clone())
    }).collect()
}

fn one_error(src: &str) -> (usize, usize, String) {
    let e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    e.into_iter().next().unwrap()
}

// ---- parsing

#[test]
fn the_example_parses_into_its_statements() {
    let p = parse(EXAMPLE);
    assert!(p.ok(), "{:?}", p.diagnostics);
    assert_eq!(p.program.stmts.len(), 7);
    assert!(matches!(p.program.stmts[0].kind, StmtKind::Setting(_)));
    assert_eq!(p.comments.len(), 1);
}

#[test]
fn canonical_text_is_one_statement_a_line_with_no_comments() {
    let p = parse(EXAMPLE);
    assert_eq!(
        canonical(&p.program),
        "\
tempo 126
key am
deck A: kick e(5, 8) | snare ~ x ~ x | bass markov(am, 0.7) | hihat x*8 vel=60..90
deck B: harmony chord(am7) | arp arp(am7, up, 8)
every 16 bars: swap pairs with echo_out
on phrase: deck A: filter lp 12000 -> 400 over 4 bars
master: filter lp 2000 -> 8000 over 8 bars
"
    );
}

#[test]
fn canonical_text_is_a_fixed_point() {
    let once = canonical(&parse(EXAMPLE).program);
    let twice = canonical(&parse(&once).program);
    assert_eq!(once, twice);
}

#[test]
fn spacing_and_comments_do_not_change_the_program() {
    let a = check("deck A: kick x ~ x ~ | snare ~ x ~ x\n").compiled.unwrap();
    let b = check("  deck   A :kick x ~  x ~\n   | snare  ~ x ~ x   # backbeat\n\n\n").compiled.unwrap();
    assert_eq!(a.text, b.text);
    assert_eq!(a.cid, b.cid);
}

#[test]
fn a_line_starting_with_a_pipe_continues_the_statement() {
    let p = parse("deck A: kick x ~ x ~\n  | snare ~ x ~ x\n  # a comment between\n  | hihat x*8\n");
    assert!(p.ok(), "{:?}", p.diagnostics);
    assert_eq!(p.program.stmts.len(), 1);
}

#[test]
fn sharps_are_in_words_and_comments_are_not() {
    let c = check("deck A: bass f#2 ~ a#1 ~ # not a sharp\n");
    assert!(c.ok(), "{:?}", c.diagnostics);
}

#[test]
fn euclid_is_printed_short_and_scale_degrees_parse() {
    let c = check("deck A: bass euclid(3, 8) 1 3 5\n").compiled.unwrap();
    assert!(c.text.contains("bass e(3, 8) 1 3 5"), "{}", c.text);
}

// ---- errors

#[test]
fn error_a_part_with_no_pattern() {
    assert_eq!(one_error("deck A: kick | snare ~ x\n"), (1, 14, "`kick` has no pattern".into()));
}

#[test]
fn error_unknown_word_where_a_step_goes() {
    let (l, c, m) = one_error("deck A: kick x ~ q ~\n");
    assert_eq!((l, c), (1, 18));
    assert!(m.contains("`q` is not a step"), "{m}");
}

#[test]
fn error_a_parameter_without_its_equals_says_so() {
    let c = check("deck A: kick x ~ x ~ vel 90\n");
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.help.as_deref().unwrap_or("").contains("vel=90"), "{d:?}");
}

#[test]
fn error_unclosed_group_names_what_was_expected() {
    let c = check("deck A: kick [x x ~\n");
    let d = &c.diagnostics[0];
    assert!(d.message.contains("expected `]`"), "{d:?}");
    assert!(d.expected.contains(&"`]`".to_string()), "{d:?}");
}

#[test]
fn error_a_part_needs_a_deck() {
    let (l, c, m) = one_error("kick x ~ x ~\n");
    assert_eq!((l, c), (1, 1));
    assert!(m.contains("needs a deck"), "{m}");
}

#[test]
fn error_a_time_word_cannot_hold_a_part() {
    let (_, _, m) = one_error("every 4 bars: deck A: kick x\n");
    assert!(m.contains("a pattern plays all the time"), "{m}");
}

#[test]
fn error_unknown_action_suggests_the_nearest() {
    let c = check("master: fliter lp 2k\n");
    let d = &c.diagnostics[0];
    assert_eq!(d.help.as_deref(), Some("did you mean `filter`?"));
}

#[test]
fn error_a_ramp_needs_its_length_and_a_length_needs_a_ramp() {
    let (_, _, m) = one_error("master: filter lp 2k -> 8k\n");
    assert!(m.contains("needs a length"), "{m}");
    let (_, _, m) = one_error("master: filter lp 2k over 8 bars\n");
    assert!(m.contains("nothing to move"), "{m}");
}

#[test]
fn error_filter_hertz_out_of_range() {
    let (_, _, m) = one_error("master: filter hp 20k\n");
    assert!(m.contains("high-pass cutoff is 20 Hz to 8000 Hz"), "{m}");
}

#[test]
fn error_two_parts_of_one_role() {
    let (l, c, m) = one_error("deck A: kick x ~ x ~\ndeck A: kick x x x x\n");
    assert_eq!((l, c), (2, 9));
    assert!(m.contains("already has a `kick` part (line 1)"), "{m}");
}

#[test]
fn error_a_key_that_is_not_one_and_a_move_that_is_not_one() {
    let (_, _, m) = one_error("key h\n");
    assert!(m.contains("`h` is not a key"), "{m}");
    let c = check("every 8 bars: move bass_swp on B\n");
    assert_eq!(c.diagnostics[0].help.as_deref(), Some("did you mean `bass_swap`?"));
}

#[test]
fn error_generator_arguments() {
    let (_, _, m) = one_error("deck A: kick e(9,8)\n");
    assert!(m.contains("9 hits do not fit in 8 steps"), "{m}");
    let (_, _, m) = one_error("deck A: bass markov(am, 1.5)\n");
    assert!(m.contains("a density is 0 to 1"), "{m}");
    let (_, _, m) = one_error("deck A: harmony chord(zz)\n");
    assert!(m.contains("`zz` is not a chord"), "{m}");
    let c = check("deck A: bass markv(am, 0.5)\n");
    assert_eq!(c.diagnostics[0].help.as_deref(), Some("did you mean `markov`?"));
}

#[test]
fn every_statement_is_read_so_every_error_is_reported() {
    let e = errors("deck A: kick x q\ntempo 900\nmaster: filter lp 1\n");
    assert_eq!(e.len(), 3, "{e:?}");
    assert_eq!(e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 2, 3]);
}

#[test]
fn the_caret_form_points_at_the_span() {
    let src = "deck A: kick x ~ q ~\n";
    let c = check(src);
    let r = c.diagnostics[0].render(src);
    assert!(r.starts_with("error: `q` is not a step"), "{r}");
    assert!(r.contains("--> 1:18"), "{r}");
    assert!(r.contains("1 | deck A: kick x ~ q ~"), "{r}");
    assert!(r.contains("  |                  ^"), "{r}");
}

#[test]
fn a_number_that_runs_into_letters_and_odd_marks_are_lexer_errors() {
    let e = errors("deck A: kick x 2abc\n");
    assert!(e[0].2.contains("not a number or a word"), "{e:?}");
    let e = errors("deck A: kick x@\n");
    assert!(e[0].2.contains("unexpected character `@`"), "{e:?}");
}

#[test]
fn warnings_do_not_stop_a_program() {
    let c = check("deck A: hihat x x x\n");
    assert!(c.ok());
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("sixteenths")), "{:?}", c.diagnostics);
}

// ---- compile: the shape of the net

fn notes_of(env: &JsonValue, net: &str) -> Vec<(usize, i64)> {
    // the notes a net plays: transitions with a midi binding, by step
    let nets = env.get("nets").unwrap();
    let net = nets.get(net).unwrap_or_else(|| panic!("no net {net} in {}", env.canonical_js()));
    let ts = net.get("transitions").and_then(JsonValue::as_object).unwrap();
    let mut v = Vec::new();
    for (k, t) in ts.iter() {
        if let Some(m) = t.get("midi") {
            let step: usize = k.trim_start_matches('t').parse().unwrap();
            v.push((step, m.get("note").and_then(JsonValue::as_f64).unwrap() as i64));
        }
    }
    v.sort();
    v
}

#[test]
fn a_backbeat_compiles_to_rings_of_sixteen_places() {
    let c = check("deck A: kick x ~ x ~ | snare ~ x ~ x\n").compiled.unwrap();
    let ir = &c.decks[&Deck::A];
    assert_eq!(ir.period, 1);
    let env = ir.envelope(0);
    let nets = env.get("nets").and_then(JsonValue::as_object).unwrap();
    assert_eq!(nets.len(), 2);
    for (_, n) in nets.iter() {
        assert_eq!(n.get("places").and_then(JsonValue::as_object).unwrap().len(), 16);
        assert_eq!(n.get("transitions").and_then(JsonValue::as_object).unwrap().len(), 16);
    }
    assert_eq!(notes_of(&env, "kick"), vec![(0, 36), (8, 36)]);
    assert_eq!(notes_of(&env, "snare"), vec![(4, 38), (12, 38)]);
}

#[test]
fn euclid_five_in_eight_is_the_engines_bjorklund() {
    let c = check("deck A: kick e(5,8)\n").compiled.unwrap();
    let env = c.decks[&Deck::A].envelope(0);
    let want: Vec<usize> = djengine::gen::euclidean::bjorklund(5, 8).iter().enumerate().filter(|(_, v)| **v == 1).map(|(i, _)| i * 2).collect();
    let got: Vec<usize> = notes_of(&env, "kick").iter().map(|(s, _)| *s).collect();
    assert_eq!(got, want);
    assert_eq!(got.len(), 5);
}

#[test]
fn groups_repeats_and_rests_share_the_bar() {
    let c = check("deck A: hihat [x x] ~ x*2 x\n").compiled.unwrap();
    let env = c.decks[&Deck::A].envelope(0);
    // four slots of four steps: [x x] = 0 and 2, rest, x*2 = 8 and 10, x = 12
    let steps: Vec<usize> = notes_of(&env, "hihat").iter().map(|(s, _)| *s).collect();
    assert_eq!(steps, vec![0, 2, 8, 10, 12]);
}

#[test]
fn a_stack_plays_its_notes_together() {
    let c = check("deck A: harmony [c3, e3, g3]\n").compiled.unwrap();
    let env = c.decks[&Deck::A].envelope(0);
    let n = env.get("nets").and_then(JsonValue::as_object).unwrap();
    // one note per step per net: the other two go to sibling nets of the role
    assert_eq!(n.len(), 3, "{:?}", n.keys().collect::<Vec<_>>());
    let all: Vec<i64> = ["harmony", "harmony-b", "harmony-c"].iter().flat_map(|k| notes_of(&env, k)).map(|(_, n)| n).collect();
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(sorted, vec![48, 52, 55]);
}

#[test]
fn an_alternation_makes_the_net_as_long_as_it_needs() {
    let c = check("deck A: melody <c4 e4 g4> | kick x ~ x ~\n").compiled.unwrap();
    let ir = &c.decks[&Deck::A];
    assert_eq!(ir.period, 3);
    let env = ir.envelope(0);
    let m = notes_of(&env, "melody");
    assert_eq!(m, vec![(0, 60), (16, 64), (32, 67)]);
    // the kick plays every bar of the three
    assert_eq!(notes_of(&env, "kick").len(), 6);
    // started a bar in, bar 0 of the net plays e4
    let m1 = notes_of(&ir.envelope(1), "melody");
    assert_eq!(m1, vec![(0, 64), (16, 67), (32, 60)]);
}

#[test]
fn nested_alternations_and_fast_alternations() {
    let c = check("deck A: melody <c4 <d4 e4>>\n").compiled.unwrap();
    assert_eq!(c.decks[&Deck::A].period, 4);
    let m: Vec<i64> = notes_of(&c.decks[&Deck::A].envelope(0), "melody").iter().map(|(_, n)| *n).collect();
    assert_eq!(m, vec![60, 62, 60, 64]);
    // <a b>*2 is both in one bar
    let c = check("deck A: melody <c4 e4>*2\n").compiled.unwrap();
    assert_eq!(c.decks[&Deck::A].period, 1);
    assert_eq!(notes_of(&c.decks[&Deck::A].envelope(0), "melody"), vec![(0, 60), (8, 64)]);
}

#[test]
fn degrees_and_notes_follow_the_key_and_the_octave() {
    let c = check("key am\ndeck A: bass 1 3 5 8\n").compiled.unwrap();
    let n: Vec<i64> = notes_of(&c.decks[&Deck::A].envelope(0), "bass").iter().map(|(_, n)| *n).collect();
    // A minor, octave 2: a, c, e, a an octave up
    assert_eq!(n, vec![45, 48, 52, 57]);
    let c = check("deck A: bass x ~ c3 ~ oct=1\n").compiled.unwrap();
    let n: Vec<i64> = notes_of(&c.decks[&Deck::A].envelope(0), "bass").iter().map(|(_, n)| *n).collect();
    assert_eq!(n, vec![24, 48]);
}

#[test]
fn a_chord_and_an_arp_are_the_chords_notes() {
    let c = check("deck B: arp arp(am7, updown, 6) oct=3\n").compiled.unwrap();
    let n: Vec<i64> = notes_of(&c.decks[&Deck::B].envelope(0), "arp").iter().map(|(_, n)| *n).collect();
    // a3 c4 e4 g4 e4 c4
    assert_eq!(n.len(), 6);
    assert_eq!(n[0], 57);
    assert_eq!(n[3], 67);
}

#[test]
fn markov_is_deterministic_and_takes_a_seed() {
    let a = check("deck A: bass markov(am, 0.7)\n").compiled.unwrap();
    let b = check("deck A: bass markov(am, 0.7)\n").compiled.unwrap();
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    let s1 = check("deck A: bass markov(am, 0.7, 1)\n").compiled.unwrap();
    let s2 = check("deck A: bass markov(am, 0.7, 2)\n").compiled.unwrap();
    assert_ne!(s1.decks[&Deck::A].net_cid(), s2.decks[&Deck::A].net_cid());
    // the notes are the key's
    let env = a.decks[&Deck::A].envelope(0);
    let notes = notes_of(&env, "bass");
    assert!(!notes.is_empty());
    let a_minor: Vec<i64> = (0..8).flat_map(|o| [0, 2, 3, 5, 7, 8, 10].map(|i| 9 + o * 12 + i)).collect();
    for (_, n) in notes {
        assert!(a_minor.contains(&n) || a_minor.contains(&(n + 12)) || a_minor.contains(&(n - 12)), "{n}");
    }
}

#[test]
fn velocity_ranges_are_hashes_of_the_place_not_chance() {
    let a = check("deck A: hihat x*8 vel=60..90\n").compiled.unwrap();
    let b = check("deck A: hihat x*8 vel=60..90\n").compiled.unwrap();
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    let ir = &a.decks[&Deck::A];
    let vels: Vec<u8> = ir.parts[0].cycles[0].iter().map(|h| h.vel).collect();
    assert!(vels.iter().all(|v| (60..=90).contains(v)), "{vels:?}");
    assert!(vels.iter().any(|v| *v != vels[0]), "{vels:?}");
}

#[test]
fn the_same_text_is_the_same_net_whatever_the_order_of_parts_and_spacing() {
    let a = check("deck A: kick e(5,8) | snare ~ x ~ x\n").compiled.unwrap();
    let b = check("deck A:   snare ~ x ~ x\ndeck A: kick e(5, 8)\n").compiled.unwrap();
    assert_eq!(a.decks[&Deck::A].net_cid(), b.decks[&Deck::A].net_cid());
    let c = check("deck A: kick e(5,8) | snare ~ x ~ x vel=90\n").compiled.unwrap();
    assert_ne!(a.decks[&Deck::A].net_cid(), c.decks[&Deck::A].net_cid());
}

#[test]
fn the_envelope_is_a_raw_nets_share_the_engine_loads() {
    let c = check("tempo 126\nkey am\ndeck A: kick x ~ x ~ | bass 1 ~ 3 ~\n").compiled.unwrap();
    let env = c.decks[&Deck::A].envelope(0);
    assert_eq!(env.get("tempo").and_then(JsonValue::as_f64), Some(126.0));
    let share = djengine::share::Share::from_payload(&env, None);
    let project = share.raw_nets_project().expect("a raw-nets project");
    assert!(project.nets.contains_key("kick") && project.nets.contains_key("bass"));
}

#[test]
fn net_cids_are_pinned() {
    // a change here is a change in what the same text means: say so in docs/BTW.md
    let c = check("deck A: kick x ~ x ~ | snare ~ x ~ x\n").compiled.unwrap();
    assert_eq!(c.cid, "z4EBG9jF93KbegNhFj6qpGcRyuQPbWGEkXpmj4uLcc6Vgsbni6W");
    assert_eq!(c.decks[&Deck::A].net_cid(), "z4EBG9j14KzNPceJaWT1ErVLVDLRNpqngCQtSUHP9yDu7McaLjf");
}

// ---- compile: control lines

#[test]
fn the_example_control_lines_become_registry_calls() {
    let c = check(EXAMPLE).compiled.unwrap();
    assert_eq!(c.controls.len(), 3);
    let swap = &c.controls[0];
    assert_eq!(swap.timing, Timing::Every(16));
    let Effect::Calls(cs) = &swap.effect else { panic!() };
    assert_eq!(cs[0].noun, "autodj");
    assert_eq!(cs[0].params.canonical_js(), r#"{"action":"set","style":"echo"}"#);
    assert_eq!(cs[1].params.canonical_js(), r#"{"action":"next"}"#);
    let build = &c.controls[1];
    assert_eq!(build.timing, Timing::Every(8));
    let Effect::Ramp(r) = &build.effect else { panic!() };
    assert_eq!((r.noun.as_str(), r.bars, r.field.as_str()), ("inserts", djlive::dur::Dur::bars(4), "value"));
    assert_eq!(r.base.canonical_js(), r#"{"action":"set","param":"filter.sweep","target":"A"}"#);
    // 12 kHz is nearly open (low-pass side), 400 Hz is further in
    assert!(r.from < 0.5 && r.to < r.from, "{} {}", r.from, r.to);
    assert_eq!(r.prelude[0].params.canonical_js(), r#"{"action":"on","effect":"filter","on":true,"target":"A"}"#);
    let master = &c.controls[2];
    assert_eq!(master.timing, Timing::Now);
    let Effect::Ramp(r) = &master.effect else { panic!() };
    assert!(r.from < r.to && r.to < 0.5, "{} {}", r.from, r.to);
}

#[test]
fn every_control_verb_has_its_calls() {
    let cases: &[(&str, &str, &str)] = &[
        ("move bass_swap on B over 8 bars", "moves", r#"{"action":"arm","bars":8,"deck":"B","kind":"bass_swap"}"#),
        ("load pair dnb:2602 group 2", "loopsets", r#"{"action":"load_pair","group":2,"spec":"dnb:2602"}"#),
        ("queue play", "queue", r#"{"action":"play"}"#),
        ("queue jump 3", "queue", r#"{"action":"jump","n":3}"#),
        ("queue loop off", "queue", r#"{"action":"loop","on":false}"#),
        ("deck B: load techno:42", "deck", r#"{"action":"load","deck":"B","spec":"techno:42"}"#),
        ("deck A: sync", "deck", r#"{"action":"sync","deck":"A"}"#),
        ("deck A: mute", "mixer", r#"{"deck":"A","mute":true}"#),
        ("master: level 0.8", "mixer", r#"{"master":0.8}"#),
        ("deck A: level 70%", "mixer", r#"{"deck":"A","volume":0.7}"#),
        ("deck B: low 0", "mixer", r#"{"deck":"B","low":0}"#),
        ("crossfader 1", "mixer", r#"{"crossfader":1}"#),
        ("master: delay 0.4", "inserts", r#"{"action":"set","param":"delay.mix","target":"Master","value":0.4}"#),
        ("master: reverb 0.3", "inserts", r#"{"action":"set","param":"reverb.mix","target":"Master","value":0.3}"#),
    ];
    for (line, noun, last) in cases {
        let src = format!("{line}\n");
        let c = check(&src);
        let comp = c.compiled.unwrap_or_else(|| panic!("{line}: {:?}", c.diagnostics));
        let Effect::Calls(cs) = &comp.controls[0].effect else { panic!("{line}") };
        let last_call = cs.last().unwrap();
        assert_eq!(last_call.noun, *noun, "{line}");
        assert_eq!(last_call.params.canonical_js(), *last, "{line}");
    }
}

#[test]
fn filter_open_and_off_and_a_plain_set() {
    let c = check("master: filter off\nmaster: filter open\ndeck A: filter hp 400\n").compiled.unwrap();
    let Effect::Calls(off) = &c.controls[0].effect else { panic!() };
    assert_eq!(off.len(), 1);
    let Effect::Calls(open) = &c.controls[1].effect else { panic!() };
    assert_eq!(open[1].params.canonical_js(), r#"{"action":"set","param":"filter.sweep","target":"Master","value":0.5}"#);
    let Effect::Calls(hp) = &c.controls[2].effect else { panic!() };
    let v = hp[1].params.get("value").and_then(JsonValue::as_f64).unwrap();
    assert!(v > 0.5 && v < 1.0, "{v}");
}

#[test]
fn the_knob_inverts_the_filters_own_curve() {
    // djrust::fx::insert::sweep_corners is the forward map
    for hz in [100.0, 400.0, 2000.0, 12000.0] {
        let v = djlive::compile::sweep_knob(hz, false);
        let (lp, _) = djengine::fx::insert::sweep_corners(v);
        assert!((lp.unwrap() - hz).abs() / hz < 1e-3, "{hz} -> {v} -> {lp:?}");
    }
    for hz in [40.0, 400.0, 4000.0] {
        let v = djlive::compile::sweep_knob(hz, true);
        let (_, hp) = djengine::fx::insert::sweep_corners(v);
        assert!((hp.unwrap() - hz).abs() / hz < 1e-3, "{hz} -> {v} -> {hp:?}");
    }
}

#[test]
fn identical_control_lines_are_numbered() {
    let c = check("queue next\nqueue next\n").compiled.unwrap();
    assert_eq!(c.controls[0].key, "queue next");
    assert_eq!(c.controls[1].key, "queue next#2");
}

#[test]
fn targets_that_do_not_take_an_action_are_refused_with_the_reason() {
    let (_, _, m) = one_error("deck A: swap pairs\n");
    assert!(m.contains("takes no target"), "{m}");
    let (_, _, m) = one_error("master: low 0.5\n");
    assert!(m.contains("deck strip's EQ"), "{m}");
    let (_, _, m) = one_error("play\n");
    assert!(m.contains("needs a deck"), "{m}");
    let (_, _, m) = one_error("filter lp 2k\n");
    assert!(m.contains("needs a target"), "{m}");
}
