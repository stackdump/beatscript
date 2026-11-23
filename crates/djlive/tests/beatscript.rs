//! BeatScript: lets, defs, repeat, sections and arrange, `do` against a registry, the timing words,
//! instruments, imports, and the canonical form.

use djengine::json::JsonValue;
use djlive::compile::{check_with, Effect, Timing};
use djlive::env::*;

fn p(name: &str, types: &[&str], options: &[&str], required: bool) -> ParamSpec {
    ParamSpec { name: name.into(), types: types.iter().map(|s| s.to_string()).collect(), options: options.iter().map(|s| s.to_string()).collect(), required, doc: String::new(), ..Default::default() }
}

fn noun(name: &str, default: &str, actions: &[(&str, &[ParamSpec])], params: Vec<ParamSpec>) -> NounSpec {
    NounSpec {
        name: name.into(),
        default_action: default.into(),
        actions: actions.iter().map(|(a, ps)| ActionSpec { name: a.to_string(), effect: "write".into(), params: ps.to_vec() }).collect(),
        params,
    }
}

/// A small registry shaped like the real one.
fn registry() -> StaticRegistry {
    let mut r = StaticRegistry::default();
    let mut add = |n: NounSpec| {
        r.nouns.insert(n.name.clone(), n);
    };
    add(noun(
        "vocoder",
        "status",
        &[("status", &[]), ("on", &[]), ("off", &[]), ("set", &[])],
        vec![p("mix", &["number"], &[], false), p("bands", &["integer"], &[], false), p("instance", &["integer"], &[], false)],
    ));
    add(noun("freeze", "status", &[("status", &[]), ("on", &[]), ("off", &[]), ("phrase", &[p("bars", &["integer"], &[], false)])], vec![p("bars", &["integer"], &[], false), p("input", &["string", "number"], &[], false)]));
    add(noun(
        "trick",
        "status",
        &[("status", &[]), ("on", &[]), ("off", &[])],
        vec![p("deck", &["string"], &["A", "B"], false), p("kind", &["string"], &["stutter", "brake", "spin"], false)],
    ));
    add(noun(
        "vocal",
        "status",
        &[("status", &[]), ("create", &[p("lyrics", &["string"], &[], true), p("key", &["string"], &[], false), p("bars", &["integer"], &[], false)])],
        vec![p("lyrics", &["string"], &[], false), p("key", &["string"], &[], false), p("bars", &["integer"], &[], false)],
    ));
    add(noun("mixer", "", &[], vec![p("deck", &["string"], &[], false), p("mute", &["boolean"], &[], false), p("volume", &["number"], &[], false), p("crossfader", &["number"], &[], false), p("master", &["number"], &[], false)]));
    add(noun("inserts", "status", &[("status", &[]), ("on", &[]), ("set", &[])], vec![p("target", &["string"], &[], false), p("effect", &["string"], &[], false), p("on", &["boolean"], &[], false), p("param", &["string"], &[], false), p("value", &["number"], &[], false)]));
    r.aliases.insert("live".into(), "btw".into());
    r
}

fn catalog() -> StaticCatalog {
    StaticCatalog {
        voices: ["bass", "wobble-lead", "detuned-saw", "hoover", "pwm-lead", "warm-pad", "sub-bass", "voice-robot"].iter().map(|s| s.to_string()).collect(),
        families: ["bass", "reese", "acid", "sub", "pad"].iter().map(|s| s.to_string()).collect(),
        kits: ["drums", "drums-808"].iter().map(|s| s.to_string()).collect(),
        palettes: vec!["studio".into(), "synth".into()],
    }
}

fn ok(src: &str) -> djlive::compile::Compiled {
    let reg = registry();
    let cat = catalog();
    let env = Env { registry: Some(&reg), catalog: Some(&cat), snippets: None, ..Env::default() };
    let c = check_with(src, &env);
    assert!(c.ok(), "{src}\n{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn err(src: &str) -> Vec<(usize, usize, String)> {
    let reg = registry();
    let cat = catalog();
    let env = Env { registry: Some(&reg), catalog: Some(&cat), snippets: None, ..Env::default() };
    let c = check_with(src, &env);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| {
        let (l, col, _) = d.position(src);
        (l, col, d.message.clone())
    }).collect()
}

// ---- let

#[test]
fn a_scalar_let_is_put_where_it_is_used() {
    let c = ok("let late = 8\nlet root = c2\nat bar late: master: level 0.5\ndeck A: bass root ~ root ~\n");
    assert_eq!(c.controls[0].timing, Timing::At(8));
    assert!(c.text.contains("at bar 8: master: level 0.5"), "{}", c.text);
    assert!(c.text.contains("bass c2 ~ c2 ~"), "{}", c.text);
}

#[test]
fn a_pattern_let_is_a_group_where_it_is_used() {
    let a = ok("let riff = c2 ~ [e2 g2] ~\ndeck A: bass riff\n");
    let b = ok("deck A: bass [c2 ~ [e2 g2] ~]\n");
    assert_eq!(a.decks[&djlive::ast::Deck::A].net_cid(), b.decks[&djlive::ast::Deck::A].net_cid());
    // twice in a bar is each half the bar
    let c = ok("let riff = c2 e2\ndeck A: bass riff riff\n");
    let d = ok("deck A: bass [c2 e2] [c2 e2]\n");
    assert_eq!(c.decks[&djlive::ast::Deck::A].net_cid(), d.decks[&djlive::ast::Deck::A].net_cid());
}

#[test]
fn a_let_cannot_take_a_word_of_the_language() {
    let e = err("let kick = x ~ x ~\n");
    assert!(e[0].2.contains("cannot name"), "{e:?}");
    let e = err("let riff = c2\nlet riff = e2\n");
    assert!(!e.is_empty());
}

// ---- def and call

#[test]
fn a_def_expands_at_each_call_and_the_text_is_kept_as_written() {
    let src = "def drop(deck, bars) {\n  deck $deck: kick x ~ x ~\n  at bar 1: master: level 0.9\n}\ndrop(A, 8)\n";
    let c = ok(src);
    assert!(c.decks.contains_key(&djlive::ast::Deck::A));
    assert!(c.text.contains("def drop(deck, bars) {"), "{}", c.text);
    assert!(c.text.contains("deck $deck: kick x ~ x ~"), "{}", c.text);
    assert!(c.text.ends_with("drop(A, 8)\n"), "{}", c.text);
}

#[test]
fn a_call_with_the_wrong_number_of_arguments_says_so() {
    let e = err("def fill(a, b) {\n  deck $a: kick x\n}\nfill(A)\n");
    assert!(e[0].2.contains("takes 2 arguments"), "{e:?}");
}

#[test]
fn recursion_is_rejected_with_the_cycle_named() {
    let e = err("def ping(x) {\n  pong($x)\n}\ndef pong(x) {\n  ping($x)\n}\nping(1)\n");
    assert!(e.iter().any(|(_, _, m)| m.contains("calls itself") && m.contains("->")), "{e:?}");
    let e = err("def again(x) {\n  again($x)\n}\nagain(1)\n");
    assert!(e.iter().any(|(_, _, m)| m.contains("calls itself")), "{e:?}");
}

#[test]
fn an_unknown_param_in_a_def_says_which() {
    let e = err("def hit(a) {\n  deck $b: kick x\n}\nhit(A)\n");
    assert!(e.iter().any(|(_, _, m)| m.contains("`$b` is not a parameter")), "{e:?}");
}

// ---- repeat

#[test]
fn repeat_runs_its_body_a_written_number_of_times_with_the_index() {
    let c = ok("repeat 3 {\n  at bar 2: master: level 0.5\n}\n");
    assert_eq!(c.controls.len(), 3);
    assert!(c.text.contains("repeat 3 {"));
    let c = ok("repeat 2 {\n  deck B: level 0.$i\n}\n".replace("0.$i", "1").as_str());
    assert_eq!(c.controls.len(), 2);
}

#[test]
fn repeat_shifts_each_copy_by_its_every() {
    let c = ok("section one 8 bars {\n  repeat 4 every 2 bars {\n    master: delay 0.5\n  }\n}\narrange one\n");
    let ats: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(ats, vec![Timing::At(0), Timing::At(2), Timing::At(4), Timing::At(6)]);
}

#[test]
fn repeat_needs_a_written_count() {
    assert!(!err("repeat 0 {\n}\n").is_empty());
    assert!(!err("repeat 100 {\n  master: level 1\n}\n").is_empty());
}

// ---- sections

#[test]
fn sections_make_a_timeline_and_lines_in_them_count_from_their_start() {
    let src = "section intro 8 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 16 bars {\n  deck A: kick x x x x | bass c2\n  at bar 4: master: level 0.5\n  master: filter lp 2k -> 8k over 8 bars\n}\narrange intro drop\n";
    let c = ok(src);
    assert!(c.arranged);
    assert_eq!(c.segments.len(), 2);
    assert_eq!((c.segments[0].start, c.segments[0].bars), (0, 8));
    assert_eq!((c.segments[1].start, c.segments[1].bars), (8, 16));
    assert_eq!(c.song_bars(), Some(24));
    assert_eq!(c.section_at(9), Some(("drop", 1, 16)));
    let times: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(times, vec![Timing::At(12), Timing::At(8)]);
    assert_ne!(c.segments[0].decks[&djlive::ast::Deck::A].net_cid(), c.segments[1].decks[&djlive::ast::Deck::A].net_cid());
}

#[test]
fn lines_outside_sections_apply_to_every_section_and_a_section_overrides_a_role() {
    let src = "deck A: hihat x*8\nsection one 4 bars {\n  deck A: kick x ~ x ~\n}\nsection two 4 bars {\n  deck A: hihat x*4 | snare ~ x\n}\narrange one two\n";
    let c = ok(src);
    let one: Vec<&str> = c.segments[0].decks[&djlive::ast::Deck::A].parts.iter().map(|p| p.role.as_str()).collect();
    let two = &c.segments[1].decks[&djlive::ast::Deck::A];
    assert!(one.contains(&"hihat") && one.contains(&"kick"), "{one:?}");
    assert!(two.parts.iter().any(|p| p.role == "snare"));
    // section two's own hihat wins over the song's
    let hh = two.parts.iter().find(|p| p.role == "hihat").unwrap();
    assert_eq!(hh.cycles[0].len(), 4);
}

#[test]
fn arrange_names_a_section_that_is_not_there_with_a_suggestion() {
    let e = err("section intro 4 bars {\n  deck A: kick x\n}\narrange intr\n");
    assert!(e[0].2.contains("no section `intr`"), "{e:?}");
    let reg = registry();
    let env = Env { registry: Some(&reg), catalog: None, snippets: None, ..Env::default() };
    let src = "section intro 4 bars {\n  deck A: kick x\n}\narrange intr\n";
    let c = check_with(src, &env);
    assert!(c.diagnostics.iter().any(|d| d.help.as_deref() == Some("did you mean `intro`?")));
}

#[test]
fn sections_in_written_order_are_the_arrangement_when_there_is_no_arrange() {
    let c = ok("section one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n}\n");
    assert_eq!(c.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["one", "two"]);
}

#[test]
fn after_and_before_a_section_are_bars_of_the_timeline() {
    let src = "section one 8 bars {\n  deck A: kick x\n}\nsection two 8 bars {\n  deck A: kick x x\n}\narrange one two\nafter one: master: reverb 0.4\nbefore two 2 bars: master: filter lp 2k\n";
    let c = ok(src);
    let times: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(times, vec![Timing::At(8), Timing::At(6)]);
}

// ---- timing words

#[test]
fn the_timing_words_resolve_to_bars() {
    let c = ok("every 4 bars from 2: master: delay 0.2\non next phrase 16: master: reverb 0.1\non next bar: master: crush 0.1\nat bar 3: master: level 0.5\nevery 8 bars: master: reverb 0.3\n");
    let t: Vec<Timing> = c.controls.iter().map(|c| c.timing).collect();
    assert_eq!(t, vec![Timing::EveryFrom(4, 2), Timing::NextMult(16), Timing::Now, Timing::At(3), Timing::Every(8)]);
}

#[test]
fn for_n_bars_puts_it_back_where_the_verb_has_an_off() {
    let c = ok("at bar 4 for 2 bars: master: filter lp 800\n");
    let ctl = &c.controls[0];
    assert_eq!(ctl.hold, Some(djlive::dur::Dur::bars(2)));
    assert_eq!(ctl.undo.len(), 1);
    assert_eq!(ctl.undo[0].params.get("on").and_then(JsonValue::as_bool), Some(false));
    let c = ok("for 3 bars: deck A: mute\n");
    assert_eq!(c.controls[0].undo[0].params.get("mute").and_then(JsonValue::as_bool), Some(false));
    let c = ok("for 8 bars: do freeze on\n");
    assert_eq!(c.controls[0].undo[0].params.get("action").and_then(JsonValue::as_str), Some("off"));
    // a level has no off
    let e = err("for 4 bars: master: level 0.2\n");
    assert!(e[0].2.contains("needs something that can be put back"), "{e:?}");
}

// ---- do and sugar

#[test]
fn do_is_checked_against_the_registry_with_the_line_and_column() {
    let c = ok("do vocoder set mix=0.5 bands=16\n");
    let Effect::Calls(cs) = &c.controls[0].effect else { panic!() };
    assert_eq!(cs[0].noun, "vocoder");
    assert_eq!(cs[0].params.get("action").and_then(JsonValue::as_str), Some("set"));
    let e = err("do vocoder set mx=0.5\n");
    assert_eq!(e[0], (1, 16, "`vocoder` has no param `mx`".to_string()));
    let e = err("do vocoer on\n");
    assert!(e[0].2.contains("not a noun"));
    let e = err("do vocoder onn\n");
    assert!(e[0].2.contains("no verb `onn`"));
    let e = err("do vocoder set mix=\"loud\"\n");
    assert!(e[0].2.contains("`mix` is"), "{e:?}");
    let e = err("do vocoder set bands=2.5\n");
    assert!(e[0].2.contains("whole number"), "{e:?}");
    let e = err("do trick on deck=C\n");
    assert!(e[0].2.contains("not a value of `deck`"), "{e:?}");
    let e = err("do vocal create bars=2\n");
    assert!(e[0].2.contains("needs `lyrics`"), "{e:?}");
}

#[test]
fn sugar_is_do_without_the_do() {
    let a = ok("vocoder set mix=0.5\n");
    let b = ok("do vocoder set mix=0.5\n");
    assert_eq!(a.controls[0].effect, b.controls[0].effect);
    let c = ok("vocal create \"robots on the road\" bars=4\n");
    let Effect::Calls(cs) = &c.controls[0].effect else { panic!() };
    assert_eq!(cs[0].params.get("lyrics").and_then(JsonValue::as_str), Some("robots on the road"));
    assert!(c.text.contains("vocal create \"robots on the road\" bars=4"), "{}", c.text);
    ok("freeze on\nfreeze phrase bars=2\ntrick on deck=A kind=stutter\n");
}

#[test]
fn a_ramp_works_on_any_numeric_param() {
    let c = ok("do vocoder set mix=0 -> 1 over 8 bars\n");
    let Effect::Ramp(r) = &c.controls[0].effect else { panic!() };
    assert_eq!((r.field.as_str(), r.from, r.to, r.bars), ("mix", 0.0, 1.0, djlive::dur::Dur::bars(8)));
    assert_eq!(r.at(0.5).params.get("mix").and_then(JsonValue::as_f64), Some(0.5));
    let e = err("do vocoder set mix=0 -> 1\n");
    assert!(e[0].2.contains("needs a length"), "{e:?}");
    let e = err("do vocoder set mix=0.3 over 8 bars\n");
    assert!(e[0].2.contains("nothing to move"), "{e:?}");
    let e = err("do trick on kind=stutter -> brake over 4 bars\n");
    assert!(!e.is_empty());
}

#[test]
fn key_is_still_a_setting_and_also_a_noun() {
    let c = ok("key am\n");
    assert!(c.key.is_some());
}

// ---- instruments

#[test]
fn an_unknown_instrument_is_an_error_with_suggestions_and_a_family_word_is_a_warning() {
    let e = err("deck A: bass c2 inst=wobble-leed\n");
    assert!(e[0].2.contains("not an instrument"), "{e:?}");
    let reg = registry();
    let cat = catalog();
    let env = Env { registry: Some(&reg), catalog: Some(&cat), snippets: None, ..Env::default() };
    let c = check_with("deck A: bass c2 inst=wobble-leed\n", &env);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.help.as_deref().unwrap().contains("`wobble-lead`"), "{:?}", d.help);
    // reese is a family word: it plays the generic bass voice, and says so
    let c = check_with("deck A: bass c2 inst=reese\n", &env);
    assert!(c.ok());
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("no voice of its own")), "{:?}", c.diagnostics);
    // a real voice is quiet
    let c = check_with("deck A: bass c2 inst=wobble-lead\n", &env);
    assert!(c.ok() && c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert!(!err("deck A: kick x kit=drums-909\n").is_empty());
    ok("deck A: kick x kit=drums-808\n");
    assert!(!err("deck A: bass c2 kit=drums\n").is_empty());
}

#[test]
fn a_part_can_set_its_roles_sound_and_the_deck_keeps_it_with_the_net() {
    let c = ok("deck A: bass c2 sound=synth:wobble-lead\n");
    let ir = &c.decks[&djlive::ast::Deck::A];
    assert_eq!(ir.sounds(), vec![("bass".to_string(), "synth:wobble-lead".to_string())]);
    let plain = ok("deck A: bass c2\n");
    assert_eq!(ir.net_cid(), plain.decks[&djlive::ast::Deck::A].net_cid());
    assert_ne!(ir.load_key(), plain.decks[&djlive::ast::Deck::A].load_key());
}

// ---- pattern transforms

fn steps(c: &djlive::compile::Compiled, role: &str, cycle: usize) -> Vec<(u8, u8)> {
    let ir = &c.decks[&djlive::ast::Deck::A];
    ir.parts.iter().find(|p| p.role == role).unwrap().cycles[cycle].iter().map(|h| (h.step, h.note)).collect()
}

#[test]
fn transforms_change_a_pattern_and_the_same_text_gives_the_same_notes() {
    let base = steps(&ok("deck A: bass [c2 e2 g2 b2]\n"), "bass", 0);
    assert_eq!(base.iter().map(|x| x.1).collect::<Vec<_>>(), vec![36, 40, 43, 47]);
    let t = steps(&ok("deck A: bass transpose([c2 e2 g2 b2], 5)\n"), "bass", 0);
    assert_eq!(t.iter().map(|x| x.1).collect::<Vec<_>>(), vec![41, 45, 48, 52]);
    let o = steps(&ok("deck A: bass octave([c2 e2 g2 b2], 1)\n"), "bass", 0);
    assert_eq!(o.iter().map(|x| x.1).collect::<Vec<_>>(), vec![48, 52, 55, 59]);
    let r = steps(&ok("deck A: bass reverse([c2 e2 g2 b2])\n"), "bass", 0);
    assert_eq!(r.iter().map(|x| x.1).collect::<Vec<_>>(), vec![47, 43, 40, 36]);
    let ro = steps(&ok("deck A: bass rotate([c2 e2 g2 b2], 1)\n"), "bass", 0);
    assert_eq!(ro.iter().map(|x| x.1).collect::<Vec<_>>(), vec![47, 36, 40, 43]);
    // by name, through a let, nested
    let n = steps(&ok("let riff = c2 e2 g2 b2\ndeck A: bass reverse(transpose(riff, 12))\n"), "bass", 0);
    assert_eq!(n.iter().map(|x| x.1).collect::<Vec<_>>(), vec![59, 55, 52, 48]);
}

#[test]
fn every_fast_slow_degrade_humanize() {
    let c = ok("deck A: bass every(2, [c2 c2 c2 c2], [e2 ~ ~ ~]) | hihat fast([x ~], 4)\n");
    let ir = &c.decks[&djlive::ast::Deck::A];
    let bass = ir.parts.iter().find(|p| p.role == "bass").unwrap();
    assert_eq!(bass.cycles.len(), 2);
    assert_eq!(bass.cycles[0].len(), 1);
    assert_eq!(bass.cycles[1].len(), 4);
    // slow: one bar of four notes over two bars
    let c = ok("deck A: bass slow([c2 e2 g2 b2], 2)\n");
    let bass = &c.decks[&djlive::ast::Deck::A].parts[0];
    assert_eq!(bass.cycles.len(), 2);
    assert_eq!(bass.cycles[0].iter().map(|h| h.note).collect::<Vec<_>>(), vec![36, 40]);
    assert_eq!(bass.cycles[1].iter().map(|h| h.note).collect::<Vec<_>>(), vec![43, 47]);
    // degrade: seeded, so the same text is the same hits, and a different seed differs
    let a = ok("deck A: hihat degrade([x*16], 0.5, 1)\n");
    let b = ok("deck A: hihat degrade([x*16], 0.5, 1)\n");
    let d = ok("deck A: hihat degrade([x*16], 0.5, 2)\n");
    let n_of = |c: &djlive::compile::Compiled| c.decks[&djlive::ast::Deck::A].net_cid();
    assert_eq!(n_of(&a), n_of(&b));
    assert_ne!(n_of(&a), n_of(&d));
    let hits = a.decks[&djlive::ast::Deck::A].parts[0].cycles[0].len();
    assert!(hits > 2 && hits < 14, "{hits}");
    let h1 = ok("deck A: hihat humanize([x*8], 3)\n");
    let h2 = ok("deck A: hihat humanize([x*8], 4)\n");
    assert_ne!(n_of(&h1), n_of(&h2));
    assert!(!err("deck A: bass transpose([c2], 99)\n").is_empty());
    assert!(!err("deck A: bass transpse([c2], 3)\n").is_empty());
}

// ---- accents and slides

#[test]
fn accents_and_slides_ride_on_steps() {
    let c = ok("deck A: bass c2! e2^ g2 ~ route=td3\n");
    let ir = &c.decks[&djlive::ast::Deck::A];
    let bass = &ir.parts[0];
    let h = &bass.cycles[0];
    assert!(h[0].accent && !h[0].slide);
    assert!(h[1].slide && !h[1].accent);
    assert!(h[0].vel > 100);
    // the slide holds into the next step
    assert!(h[1].dur >= 2);
    assert!(c.text.contains("c2! e2^ g2 ~"), "{}", c.text);
    assert!(!err("deck A: kick x! route=td3\n").is_empty());
}

#[test]
fn a_part_routed_to_the_td3_fills_its_sixteen_steps_with_accents_and_slides() {
    let c = ok("deck A: bass c2! e2^ g2 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ c3 route=td3\n");
    let td3: Vec<_> = c.controls.iter().filter(|c| c.key.starts_with("route td3")).collect();
    assert_eq!(td3.len(), 1);
    let Effect::Calls(cs) = &td3[0].effect else { panic!() };
    assert_eq!(cs.len(), 17);
    let step = |i: usize| cs[i].params.clone();
    assert_eq!(step(0).get("note").and_then(JsonValue::as_str), Some("C"));
    assert_eq!(step(0).get("accent").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(step(1).get("slide").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(step(1).get("note").and_then(JsonValue::as_str), Some("E"));
    assert_eq!(step(5).get("rest").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(step(15).get("transpose").and_then(JsonValue::as_str), Some("up"));
    assert_eq!(cs[16].params.get("active").and_then(JsonValue::as_f64), Some(16.0));
    // not routed: no calls
    assert!(ok("deck A: bass c2! e2^\n").controls.is_empty());
}

#[test]
fn resonance_is_a_filter_param_of_a_deck_or_the_master() {
    let c = ok("master: resonance 0.7\ndeck A: resonance 0.2 -> 0.8 over 4 bars\n");
    let Effect::Calls(cs) = &c.controls[0].effect else { panic!() };
    assert_eq!(cs[1].params.get("param").and_then(JsonValue::as_str), Some("filter.res"));
    assert_eq!(cs[1].params.get("value").and_then(JsonValue::as_f64), Some(0.7));
    let Effect::Ramp(r) = &c.controls[1].effect else { panic!() };
    assert_eq!((r.field.as_str(), r.from, r.to), ("value", 0.2, 0.8));
    assert!(!err("resonance 0.5\n").is_empty());
    assert!(!err("master: resonance 1.5\n").is_empty());
}

// ---- import

fn with_snippets<R>(items: &[(&str, &str)], f: impl FnOnce(&Env) -> R) -> R {
    let reg = registry();
    let cat = catalog();
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let canon = djlive::check(text).compiled.unwrap();
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    let env = Env { registry: Some(&reg), catalog: Some(&cat), snippets: Some(&snips), ..Env::default() };
    f(&env)
}

#[test]
fn an_import_brings_a_snippets_lets_and_defs_and_is_pinned_by_its_cid() {
    let lib = "let riff = c2 e2 g2 b2\nlet tempo_a = 120\ndef hit(deck) {\n  deck $deck: kick x ~ x ~\n}\n".to_string();
    with_snippets(&[("riffs", &lib)], |env| {
        let src = "import \"riffs\"\ndeck A: bass riff\nhit(B)\n";
        let c = check_with(src, env);
        assert!(c.ok(), "{:?}", c.diagnostics);
        let comp = c.compiled.unwrap();
        assert!(comp.decks.contains_key(&djlive::ast::Deck::A) && comp.decks.contains_key(&djlive::ast::Deck::B));
        // the canonical text names the CID, not the name: the program is pinned
        let lib_cid = djlive::check(&lib).compiled.unwrap().cid;
        assert!(comp.text.starts_with(&format!("import {lib_cid}\n")), "{}", comp.text);
        // the same program written with the CID is the same program
        let by_cid = check_with(&format!("import {lib_cid}\ndeck A: bass riff\nhit(B)\n"), env).compiled.unwrap();
        assert_eq!(by_cid.cid, comp.cid);
        // and a scalar let of the snippet is put where the program uses it
        let c2 = check_with(&format!("import {lib_cid}\ntempo tempo_a\n"), env).compiled.unwrap();
        assert_eq!(c2.tempo, Some(120.0));
    });
}

#[test]
fn an_import_that_is_not_there_or_has_no_store_is_an_error() {
    with_snippets(&[], |env| {
        let c = check_with("import \"nothing\"\n", env);
        assert!(!c.ok());
        assert!(c.diagnostics.iter().any(|d| d.message.contains("no saved snippet")), "{:?}", c.diagnostics);
    });
    let c = check_with("import zQmNothing\n", &Env::none());
    assert!(!c.ok());
    assert!(c.diagnostics[0].message.contains("no saved snippets"), "{:?}", c.diagnostics);
}

#[test]
fn an_imported_name_cannot_clash_with_one_of_the_programs() {
    with_snippets(&[("riffs", "let riff = c2 e2\n")], |env| {
        let c = check_with("import \"riffs\"\nlet riff = g2 b2\n", env);
        assert!(!c.ok(), "{:?}", c.diagnostics);
    });
}

#[test]
fn the_same_text_gives_the_same_cids_every_time_and_in_any_order_of_parts() {
    let src = "section verse 4 bars {\n  deck A: kick x ~ x ~ | snare ~ x ~ x\n}\nsection chorus 4 bars {\n  deck A: bass c2\n}\narrange verse chorus\n";
    let first = ok(src);
    for _ in 0..5 {
        let again = ok(src);
        assert_eq!(first.cid, again.cid);
        assert_eq!(first.net_cids(), again.net_cids());
    }
    let swapped = ok(&src.replace("kick x ~ x ~ | snare ~ x ~ x", "snare ~ x ~ x | kick x ~ x ~"));
    assert_eq!(first.net_cids(), swapped.net_cids(), "the nets do not depend on the order parts are written in");
}

#[test]
fn a_program_always_ends_nested_repeats_and_calls_are_capped() {
    let src = "repeat 64 {\n  repeat 64 {\n    repeat 64 {\n      master: delay 0.1\n    }\n  }\n}\n";
    let e = err(src);
    assert!(e.iter().any(|(_, _, m)| m.contains("more than 4096 lines")), "{e:?}");
    // a def that calls another is fine, within the depth
    ok("def inner(x) {\n  master: delay 0.1\n}\ndef outer(x) {\n  inner($x)\n}\nouter(1)\n");
}

#[test]
fn end_and_loop_are_read_and_checked() {
    let c = ok("section one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n}\narrange one two\n");
    assert_eq!((c.end, c.looped), (Some(8), false), "a song ends after its last section by default");
    let c = ok("section one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n}\narrange one two\nloop\n");
    assert_eq!((c.end, c.looped), (None, true));
    let c = ok("section one 4 bars {\n  deck A: kick x\n}\nsection two 4 bars {\n  deck A: snare x\n}\narrange one two\nend after one\n");
    assert_eq!(c.end, Some(4));
    assert!(c.text.ends_with("end after one\n"));
    assert_eq!(ok("deck A: kick x\nend at bar 64\n").end, Some(64));
    assert_eq!(ok("deck A: kick x\n").end, None, "a program with no sections plays on");
    assert!(err("deck A: kick x\nend\n")[0].2.contains("no sections"));
    assert!(err("section one 4 bars {\n  deck A: kick x\n}\nend after nope\n")[0].2.contains("no section"));
    assert!(!err("section one 4 bars {\n  deck A: kick x\n}\nend\nloop\n").is_empty(), "ends or loops, not both");
    assert!(!err("deck A: kick x\nend at bar 4\nend at bar 8\n").is_empty());
    assert!(!err("end at bar 0\n").is_empty());
}

// ---- long programs: nets of 64 bars chained

/// An `n`-bar program: a drum deck and a bass deck, every part its own n-bar alternation (so each net really is
/// n bars), the bars rotating through a few variants.
fn long_program(n: usize) -> String {
    let bar = |role: &str, k: usize| -> String {
        match role {
            "kick" => ["x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~", "x ~ ~ x x ~ ~ ~ x ~ x ~ x ~ ~ ~"][k % 2].to_string(),
            "hihat" => ["x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~", "x x x x x x x x x x x x x x x x"][k % 2].to_string(),
            _ => ["a2 ~ a2 a2 ~ a2 ~ a2 e2 ~ e2 e2 ~ e2 ~ e2", "f2 ~ f2 f2 ~ f2 ~ f2 c2 ~ c2 c2 ~ c2 ~ c2", "g2 ~ g2 g2 ~ g2 ~ g2 d2 ~ d2 d2 ~ d2 ~ d2"][k % 3].to_string(),
        }
    };
    let part = |role: &str, name: &str| format!("{name} <{}>", (0..n).map(|k| format!("[{}]", bar(role, k))).collect::<Vec<_>>().join(" "));
    format!("tempo 120\nkey am\ndeck A: {} | {}\ndeck B: {} inst=sub-bass\n", part("kick", "kick"), part("hihat", "hihat"), part("bass", "bass"))
}

#[test]
fn a_program_longer_than_one_net_is_chained_from_nets_of_64_bars() {
    // an 80-bar program was refused: a net is at most 64 bars (the roll's 1024 steps). It is chained now.
    let c = ok(&long_program(80));
    assert!(c.arranged && c.flat_clock && c.looped, "a program with no sections that is too long for one net loops from its chain");
    assert_eq!(c.song_bars(), Some(80));
    assert_eq!(c.segments.iter().map(|s| (s.start, s.bars)).collect::<Vec<_>>(), vec![(0, 64), (64, 16)]);
    for seg in &c.segments {
        for ir in seg.decks.values() {
            assert!(ir.period <= 64, "every net is at most 64 bars");
            assert!(ir.envelope(0).as_object().is_some(), "the roll takes every net: {:?}", ir.envelope(0));
        }
    }
    // the second net starts where the first left off: bar 64 of the alternation
    let kick = |seg: usize, bar: usize| c.segments[seg].decks[&djlive::ast::Deck::A].parts.iter().find(|p| p.role == "kick").unwrap().cycles[bar].clone();
    // (64 % 2 == 0: bar 64 is a "variant 0" bar, like bar 0; bar 65 like bar 1)
    assert_eq!(kick(1, 0), kick(0, 0));
    assert_eq!(kick(1, 1), kick(0, 1));
    // the window sees one program, not two sections
    assert_eq!(c.section_at(70), Some(("main", 70, 80)));
    assert!(c.warnings.iter().any(|d| d.message.contains("takes 80 bars to come round")), "{:?}", c.warnings);
}

#[test]
fn a_256_bar_program_compiles_and_every_net_stays_small() {
    let src = long_program(256);
    let t = std::time::Instant::now();
    let c = ok(&src);
    let ms = t.elapsed().as_millis();
    assert_eq!(c.song_bars(), Some(256));
    assert_eq!(c.segments.len(), 4, "four nets of 64 bars");
    assert!(c.segments.iter().all(|s| s.bars == 64 && s.decks.values().all(|ir| ir.period == 64)));
    // 256 bars at 120 BPM is 8 minutes 32 seconds: four times the old limit
    assert_eq!(c.song_bars().unwrap() as f64 * 4.0 * 60.0 / 120.0, 512.0);
    // the same text is the same CIDs, chained or not
    let again = ok(&src);
    assert_eq!(c.net_cids(), again.net_cids());
    assert_eq!(c.cid, again.cid);
    assert!(ms < 5000, "{ms} ms");
    // and as the longhand of one deck written out, the song is one whole: 256 bars
    let nets = c.song_nets().unwrap();
    assert_eq!(nets[&djlive::ast::Deck::A].period, 256);
}

#[test]
fn an_arranged_section_longer_than_its_pattern_is_not_chained_and_a_long_pattern_in_a_section_is() {
    // 200 bars of a 4-bar pattern: one net of 4 bars, looping (nothing to chain)
    let c = ok("section a 200 bars {\n  deck A: kick <x ~ x ~ x ~ x ~>\n}\narrange a\n".replace("section a ", "section sa ").replace("arrange a", "arrange sa").as_str());
    assert_eq!(c.segments.len(), 1);
    // 100 bars of a 100-bar pattern: two nets, 64 and 36
    let long = format!("section sb 100 bars {{\n  deck A: {}\n}}\narrange sb\n", long_program(100).lines().nth(2).unwrap().trim_start_matches("deck A: ").split(" | ").next().unwrap());
    let c = ok(&long);
    assert_eq!(c.segments.iter().map(|s| (s.start, s.bars, s.chain)).collect::<Vec<_>>(), vec![(0, 64, false), (64, 36, true)]);
    assert_eq!(c.section_at(70), Some(("sb", 70, 100)));
    // an end after it
    assert_eq!(c.end, Some(100));
}

#[test]
fn a_plan_summarises_what_it_adds_changes_and_removes_in_one_line() {
    use djlive::runtime::{DeckChange, DeckDiff, Plan};
    use djlive::ast::Deck;
    let d = |change| DeckDiff { deck: Deck::A, section: None, change, parts: Vec::new() };
    let p = Plan { decks: vec![d(DeckChange::Added), d(DeckChange::Changed), d(DeckChange::Changed), d(DeckChange::Same)], controls_added: vec!["a".into(), "b".into()], controls_removed: vec!["c".into()], controls_kept: 0, tempo_changed: false };
    assert_eq!(p.counts(), (3, 2, 1));
    assert_eq!(p.summary(), "+3 added · 2 changed · 1 removed");
    assert_eq!(Plan::default().summary(), "no changes");
}
