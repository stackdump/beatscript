//! `macro ID on DECK over N bars` (docs/beatscript/ext-control.md "macro"): the frozen `macro@1` table (pinned, and the engine's catalogue at
//! the pinned rev), the canonical text (the version only when it is not 1), the lowering to the registry's `effects fire` with its
//! call a beat ahead of a bar-line time, a held one-shot let go, the control net's events, and every refusal with the line that
//! would work.

use djengine::fx::catalogue::{self, Family};
use djengine::json::JsonValue;
use djlive::compile::{check, Compiled, Timing};
use djlive::macros::{self, Length, V1};
use djlive::rat::Rat;

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

/// The first error's message and help.
fn error(src: &str) -> (String, String) {
    let c = check(src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("no error:\n{src}"));
    (d.message.clone(), d.help.clone().unwrap_or_default())
}

fn canonical(src: &str) -> String {
    compiled(src).text
}

const K: &str = "deck A: kick x ~ x ~\n";

/// `macro@1` never changes: a stored program's `macro riser` means what it meant. A change of the table is `macro@2`, a table of
/// its own (macros.rs `VERSIONS`), and this pin stays.
#[test]
fn the_macro_1_table_is_frozen() {
    assert_eq!(macros::table_cid(1).as_deref(), Some("z4EBG9izkd3jD2gJGhqMbBd2cViEEfYCCMeBFe7Qa23eJ9qMqKQ"), "macro@1's table moved: a new catalogue is macro@2, never an edit of macro@1");
    assert_eq!(V1.len(), 39);
    assert_eq!(macros::VERSIONS.len(), 1, "a new version: pin its table here too, and say it in docs/beatscript/ext-control.md");
}

/// The table is the engine's catalogue at the rev this repo pins: the same ids in the same order, each in the same category and
/// family, the same transitions and the same net decks; a transition's length is its own (the riser's a sweep, a held trick
/// held, a song one-shot its own), every other entry's the Effects window's (1 bar, 2 for a sweep). When the pin moves and the
/// catalogue with it, this fails: `macro@1` stays, and the new catalogue is `macro@2`.
#[test]
fn the_table_agrees_with_the_engines_catalogue_at_the_pinned_rev() {
    assert_eq!(catalogue::ENTRIES.len(), V1.len());
    for (e, m) in catalogue::ENTRIES.iter().zip(V1.iter()) {
        assert_eq!(e.id, m.id);
        assert_eq!(e.label, m.label, "{}", m.id);
        assert_eq!(e.category.id(), m.category, "{}", m.id);
        let fam = match e.family {
            Family::Trick(_) => "trick",
            Family::Net(_) => "net",
            Family::Master(_) => "master",
            Family::Aux(_) => "aux",
        };
        assert_eq!(fam, m.family, "{}", m.id);
        assert_eq!(e.transition(), m.transition, "{}", m.id);
        assert_eq!(e.needs_net(), m.needs_net, "{}", m.id);
        let want = match (e.transition(), e.family) {
            (true, Family::Trick(djengine::fx::tricks::TrickKind::Riser)) => Length::Sweep(djengine::fx::tricks::TrickKind::Riser.default_args().bars as u32),
            (true, Family::Trick(_)) => Length::Held(1),
            (true, _) => Length::Own,
            (false, _) if !e.latchable() => Length::Bars(2),
            (false, _) => Length::Bars(1),
        };
        assert_eq!(m.length, want, "{}", m.id);
    }
}

#[test]
fn the_canonical_text_writes_the_version_only_when_it_is_not_1() {
    for (src, want) in [
        ("macro riser on A over 8 bars", "macro riser on A over 8 bars"),
        ("macro@1 riser over 8 bars on A", "macro riser on A over 8 bars"),
        ("macro   cathedral   on B", "macro cathedral on B"),
        ("control: macro@1 breakdown on A over 4 bars", "control: macro breakdown on A over 4 bars"),
        ("at bar 24:control:macro ping-pong over 2 bars on B", "at bar 24: control: macro ping-pong on B over 2 bars"),
    ] {
        let text = canonical(&format!("{K}{src}\n"));
        assert!(text.contains(&format!("\n{want}\n")), "{src}:\n{text}");
        assert_eq!(canonical(&text), text, "a fixed point: {src}");
        // the two spellings of version 1 are one text and one CID
    }
    assert_eq!(compiled(&format!("{K}macro@1 riser on A\n")).cid, compiled(&format!("{K}macro riser on A\n")).cid);
    // a version that is not 1 keeps its `@` in the text (and is refused: there is no macro@2 yet)
    let text = djlive::print::canonical(&djlive::parse::parse(&format!("{K}macro@2 riser on A\n")).program);
    assert!(text.contains("macro@2 riser on A"), "{text}");
}

fn effects_call(c: &Compiled, i: usize) -> String {
    match &c.controls[i].effect {
        djlive::compile::Effect::Calls(cs) => cs.iter().map(|c| format!("{} {}", c.noun, c.params.canonical_js())).collect::<Vec<_>>().join("; "),
        o => format!("{o:?}"),
    }
}

#[test]
fn a_macro_lowers_to_effects_fire_with_its_length() {
    let c = compiled(&format!("{K}macro riser on A over 8 bars\nmacro breakdown on B over 4 bars\nmacro cathedral on B\nmacro feel-build on A\nmacro riser on B\nmacro cut on A\n"));
    assert_eq!(effects_call(&c, 0), r#"effects {"action":"fire","bars":8,"id":"riser","target":"A"}"#);
    assert_eq!(effects_call(&c, 1), r#"effects {"action":"fire","bars":4,"id":"breakdown","target":"B"}"#);
    // with no `over`, the table's own length: the window's 1 bar, 2 for a sweep, the riser's 4; a song one-shot its own (no bars)
    assert_eq!(effects_call(&c, 2), r#"effects {"action":"fire","bars":1,"id":"cathedral","target":"B"}"#);
    assert_eq!(effects_call(&c, 3), r#"effects {"action":"fire","bars":2,"id":"feel-build","target":"A"}"#);
    assert_eq!(effects_call(&c, 4), r#"effects {"action":"fire","bars":4,"id":"riser","target":"B"}"#);
    assert_eq!(effects_call(&c, 5), r#"effects {"action":"fire","id":"cut","target":"A"}"#);
    assert!(c.controls.iter().all(|c| c.hold.is_none() && c.undo.is_empty()));
}

/// The engine starts an engagement on the deck's next bar line, so the call goes a beat ahead of a bar-line time and the macro
/// starts on the bar its line names; a time between bar lines, and one with no beat before it, is kept.
#[test]
fn the_call_goes_a_beat_ahead_of_a_bar_line_time() {
    let q = |n: i64, d: i64| Rat::new(n, d).unwrap();
    let c = compiled(&format!("{K}at bar 24: macro cathedral on B over 2 bars\nevery 16 bars: macro breakdown on A over 4 bars\nevery 4 bars from 8: macro pump on A\nat bar 3 beat 2: macro riser on A\nmacro wash on A\n"));
    assert_eq!(c.controls[0].timing, Timing::AtPos(q(95, 4)));
    assert_eq!(c.controls[1].timing, Timing::EveryPos { step: Rat::int(16), from: q(63, 4) });
    assert_eq!(c.controls[2].timing, Timing::EveryPos { step: Rat::int(4), from: q(31, 4) });
    assert_eq!(c.controls[3].timing, Timing::AtPos(q(7, 2)));
    assert_eq!(c.controls[4].timing, Timing::Now);
    // a section's first bar (a song with sections: `at bar 0` of it is the song's bar 8)
    let s = compiled("section intro 8 bars {\n  deck A: kick x ~ x ~\n}\nsection drop 8 bars {\n  deck A: kick x x x x\n  macro breakdown on A over 4 bars\n}\narrange intro drop\n");
    assert_eq!(s.controls[0].timing, Timing::AtPos(q(31, 4)));
    // the key is the line, so moving it on or off the control deck fires nothing again
    assert_eq!(s.controls[0].key.trim_start_matches(|ch| ch != ']').trim_start_matches("] "), "macro breakdown on A over 4 bars");
}

/// A held one-shot (tape stop, spin back, reverb freeze) is fired with no length and let go after its bars, counted from the
/// bar it starts on.
#[test]
fn a_held_one_shot_is_let_go_after_its_bars() {
    let c = compiled(&format!("{K}at bar 8: macro tape_stop on A over 2 bars\nmacro verb_freeze on B\n"));
    assert_eq!(effects_call(&c, 0), r#"effects {"action":"fire","id":"tape_stop","target":"A"}"#);
    // the call a beat ahead, the release two bars after the bar it starts on
    assert_eq!(c.controls[0].hold.map(|d| d.as_bars()), Some(Rat::new(9, 4).unwrap()));
    assert_eq!(c.controls[0].undo.len(), 1);
    assert_eq!(c.controls[0].undo[0].params.canonical_js(), r#"{"action":"release","id":"tape_stop","target":"A"}"#);
    // no beat before it: the engine's next bar, a bar after the call; held its 1 bar from there
    assert_eq!(c.controls[1].hold.map(|d| d.as_bars()), Some(Rat::int(2)));
}

/// The control net holds a macro: its `fire` (and a held one's `release`) on the tick its call is made.
#[test]
fn the_control_net_fires_it() {
    let c = compiled(&format!("{K}at bar 4: macro riser on A over 8 bars\nat bar 8: macro tape_stop on B over 1 bar\n"));
    let net = djlive::nets::control_net(&c, 16);
    assert!(net.residual.is_empty(), "{:?}", net.residual);
    let ev: Vec<String> = net.events(None).into_iter().map(|(t, e)| format!("{t} {}", e.text())).collect();
    assert!(ev.contains(&"60 fire A riser bars=8".to_string()), "{ev:?}");
    assert!(ev.contains(&"124 fire B tape_stop".to_string()), "{ev:?}");
    assert!(ev.contains(&"144 release B tape_stop".to_string()), "{ev:?}");
    for e in ["fire A riser bars=8", "fire B tape_stop", "release B tape_stop"] {
        assert_eq!(djlive::nets::Event::parse(e).unwrap().text(), e);
    }
}

#[test]
fn what_a_macro_line_refuses_says_what_would_work() {
    // an id not in the table: the nearest
    let (m, h) = error(&format!("{K}macro rizer on A\n"));
    assert!(m.contains("`rizer` is not in the macro catalogue of macro@1") && h == "did you mean `riser`?", "{m} / {h}");
    let (_, h) = error(&format!("{K}macro zzzzzz on A\n"));
    assert!(h.contains("the `macros` list"), "{h}");
    // a version this BeatScript does not have
    let (m, h) = error(&format!("{K}macro@7 riser on A\n"));
    assert!(m.contains("this BeatScript has macro@1, not `macro@7`") && h.contains("macro riser"), "{m} / {h}");
    // the deck is the text's, never the person's focus
    let (m, h) = error(&format!("{K}macro riser over 8 bars\n"));
    assert!(m.contains("needs the deck it fires on") && h == "macro riser on A", "{m} / {h}");
    // whole bars, 1 to 64
    let (m, h) = error(&format!("{K}macro breakdown on A over 2 beats\n"));
    assert!(m.contains("a macro takes whole bars") && h.contains("over 4 bars"), "{m} / {h}");
    let (m, h) = error(&format!("{K}macro breakdown on A over 65 bars\n"));
    assert!(m.contains("1 to 64 bars") && h.contains("over 64 bars"), "{m} / {h}");
    // a one-shot that runs its own length takes no `over`
    let (m, h) = error(&format!("{K}macro cut on A over 2 bars\n"));
    assert!(m.contains("`cut` is a one-shot: it runs its own length") && h == "macro cut on A", "{m} / {h}");
    // the length is `over`, never `for`
    let (m, _) = error(&format!("{K}at bar 4 for 2 bars: macro breakdown on A\n"));
    assert!(m.contains("takes no `for`"), "{m}");
    // it names its deck itself
    let (m, _) = error(&format!("{K}deck A: macro riser on A\n"));
    assert!(m.contains("takes no target"), "{m}");
    // the shape
    assert!(error(&format!("{K}macro on A\n")).0.contains("fires an entry of the DJ macro catalogue"));
    assert!(error(&format!("{K}macro riser on A on B\n")).0.contains("says its deck twice"));
    assert!(error(&format!("{K}macro riser over 2 bars over 4 bars on A\n")).0.contains("says its length twice"));
    // a typo of the head, at the start of a line
    assert!(error(&format!("{K}macor riser on A\n")).1.contains("did you mean `macro`?"));
}

/// `macro` is no reserved word (like `control` and `drive`): a let, a section or a def may still be called `macro`, and a scalar
/// `let` named like an id does not take the id's place.
#[test]
fn macro_is_a_free_name() {
    compiled("let macro = x ~ x ~\ndeck A: kick macro\n");
    compiled("section macro 4 bars {\n  deck A: kick x ~ x ~\n}\narrange macro\n");
    compiled("def macro(dk) {\n  deck $dk: kick x ~ x ~\n}\nmacro(A)\n");
    let c = compiled(&format!("let riser = 4\n{K}at bar riser: macro riser on A\n"));
    assert_eq!(effects_call(&c, 0), r#"effects {"action":"fire","bars":4,"id":"riser","target":"A"}"#);
    assert_eq!(c.controls[0].timing, Timing::AtPos(Rat::new(15, 4).unwrap()));
}

#[test]
fn the_head_is_coloured_as_a_verb() {
    use djlive::highlight::{highlight, Class};
    let src = "control: macro riser on A over 8 bars\nmacro@1 cut on B\n";
    let h: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    assert!(h.contains(&("macro", Class::Action)) && h.contains(&("macro@1", Class::Action)), "{h:?}");
}

#[test]
fn the_table_as_json() {
    let j = macros::table_json(1).unwrap();
    assert_eq!(j.get("head").and_then(JsonValue::as_str), Some("macro"));
    let es = j.get("entries").and_then(JsonValue::as_array).unwrap();
    assert_eq!(es.len(), 39);
    assert_eq!(es[0].canonical_js(), r#"{"category":"transition","default_bars":4,"family":"trick","id":"riser","label":"Riser","length":"sweep","needs_net":false,"takes_over":true,"transition":true}"#);
}
