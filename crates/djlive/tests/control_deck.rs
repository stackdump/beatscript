//! The control deck (roadmap 6671, docs/beatscript/ext-control.md): `control:` is a head in the place a target takes, for the engine
//! calls that take no target (`move`, `swap pairs`, `load pair`, `queue`, `crossfader`, a timed `tempo`, `duck`, `mix`, `do` and
//! its sugar). Its canonical text is its own and survives reformatting; it lowers to exactly what the line with no head
//! lowers to (the runtime controls, the deck nets, the control net's events, the tempo map); a part, a strip's action, a
//! deck's transport, a `sing` or a def's call on it is an error that says where it goes; `control` stays a free name; and the
//! programs written before it keep their CIDs.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, Compiled, Control};
use djlive::highlight::{highlight, Class};

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

/// A control as the runtime runs it: everything but where its text is (its span and the spans inside its effect, a mix item's
/// say, and the calls that expanded it).
fn ran(c: &Control) -> String {
    no_spans(&format!("{} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?}", c.key, c.timing, c.effect, c.hold, c.undo, c.restore, c.notice, c.window))
}

/// `Span { start: 63, end: 74 }` written `Span`.
fn no_spans(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("Span { start: ") {
        out.push_str(&rest[..at]);
        out.push_str("Span");
        rest = &rest[at..];
        rest = &rest[rest.find('}').map_or(rest.len(), |e| e + 1)..];
    }
    out.push_str(rest);
    out
}

/// What a program lowers to, with its text left out: the runtime controls, the nets of every deck and segment, the control
/// net's lines and events, the tempo map and the song's shape.
fn lowered(src: &str) -> (Vec<String>, Vec<String>, Vec<String>, String, Compiled) {
    let c = compiled(src);
    let controls = c.controls.iter().map(ran).collect();
    let mut nets: Vec<String> = c.decks.iter().map(|(d, ir)| format!("{d:?} {}", ir.net_cid())).collect();
    for s in &c.segments {
        nets.push(format!("{} {} {} {:?} {}", s.name, s.start, s.bars, s.gen, s.chain));
        nets.extend(s.decks.iter().map(|(d, ir)| format!("  {d:?} {}", ir.net_cid())));
    }
    nets.push(format!("end {:?} loop {} tempo {:?}", c.end, c.looped, c.tempo));
    let bars = c.song_bars().unwrap_or(32).max(1);
    let net = djlive::nets::control_net(&c, bars);
    let mut events: Vec<String> = net.lines.keys().cloned().collect();
    events.extend(net.residual.iter().map(|(k, why)| format!("beside {k}: {why}")));
    events.extend(net.events(None).into_iter().map(|(t, e)| format!("{t} {}", e.text())));
    let tempo = format!("{:?}", (0..=bars).map(|b| c.tempo_map.bpm_at(b)).collect::<Vec<_>>());
    (controls, nets, events, tempo, c)
}

/// Each pair: a program with the control deck, and the same program with its `control:` heads taken off.
const PAIRS: &[(&str, &str)] = &[
    ("deck A: kick x ~ x ~\ncontrol: move echo_out on B\n", "deck A: kick x ~ x ~\nmove echo_out on B\n"),
    ("deck A: kick x ~ x ~\nevery 16 bars: control: swap pairs with filter_exit\n", "deck A: kick x ~ x ~\nevery 16 bars: swap pairs with filter_exit\n"),
    ("deck A: kick x ~ x ~\non phrase: control: move filter_exit on B over 4 bars\n", "deck A: kick x ~ x ~\non phrase: move filter_exit on B over 4 bars\n"),
    ("deck A: kick x ~ x ~\ncontrol: load pair dnb:2602 group 2\nat bar 32: control: queue next\n", "deck A: kick x ~ x ~\nload pair dnb:2602 group 2\nat bar 32: queue next\n"),
    ("deck A: kick x ~ x ~\ncontrol: crossfader 0 -> 1 over 8 bars exp\n", "deck A: kick x ~ x ~\ncrossfader 0 -> 1 over 8 bars exp\n"),
    ("deck A: kick x ~ x ~\ncontrol: crossfader lfo sine 2 bars center=0.5 depth=0.3\n", "deck A: kick x ~ x ~\ncrossfader lfo sine 2 bars center=0.5 depth=0.3\n"),
    ("tempo 128\ndeck A: kick x ~ x ~\nat bar 16: control: tempo 128 -> 140 over 4 bars\n", "tempo 128\ndeck A: kick x ~ x ~\nat bar 16: tempo 128 -> 140 over 4 bars\n"),
    ("deck A: kick x ~ x ~\ndeck B: bass c2 ~ c2 ~\ncontrol: duck B under A.kick 6db attack=5ms\n", "deck A: kick x ~ x ~\ndeck B: bass c2 ~ c2 ~\nduck B under A.kick 6db attack=5ms\n"),
    ("deck A: kick x ~ x ~\ncontrol: do vocoder set mix=0 -> 1 over 8 bars\nevery 16 bars: control: freeze phrase bars=2\n", "deck A: kick x ~ x ~\ndo vocoder set mix=0 -> 1 over 8 bars\nevery 16 bars: freeze phrase bars=2\n"),
    ("deck A: kick x ~ x ~\nat bar 8 for 4 bars: control: trick on deck=A kind=stutter\n", "deck A: kick x ~ x ~\nat bar 8 for 4 bars: trick on deck=A kind=stutter\n"),
    ("deck A: kick x ~ x ~\ncontrol: key detect deck=A\n", "deck A: kick x ~ x ~\nkey detect deck=A\n"),
    // Matt's example (docs/BEATSCRIPT.md "Two halves"): a section sequences the control deck like any deck
    (
        "section drop 16 bars {\n  deck A: kick x x x x | bass c2 ~ c2 ~\n  deck B: arp arp(am7, up, 8)\n  master: filter lp 400 -> 8k over 8 bars\n  control: move echo_out on B\n  at bar 12: control: swap pairs with filter_exit\n}\narrange drop\n",
        "section drop 16 bars {\n  deck A: kick x x x x | bass c2 ~ c2 ~\n  deck B: arp arp(am7, up, 8)\n  master: filter lp 400 -> 8k over 8 bars\n  move echo_out on B\n  at bar 12: swap pairs with filter_exit\n}\narrange drop\n",
    ),
    // a mix in a section, put back where it ends
    (
        "section verse 4 bars {\n  deck A: kick x ~ x ~\n  control: mix { A level 0.7, B eq low -3 }\n}\nsection hook 4 bars {\n  deck A: kick x x x x\n}\narrange verse hook\n",
        "section verse 4 bars {\n  deck A: kick x ~ x ~\n  mix { A level 0.7, B eq low -3 }\n}\nsection hook 4 bars {\n  deck A: kick x x x x\n}\narrange verse hook\n",
    ),
    // a repeat and its guard, and a def's body
    (
        "deck A: kick x ~ x ~\nrepeat 4 every 8 bars {\n  at bar 7: control: move echo_out on B over 1 bars when $i % 2 == 1\n}\n",
        "deck A: kick x ~ x ~\nrepeat 4 every 8 bars {\n  at bar 7: move echo_out on B over 1 bars when $i % 2 == 1\n}\n",
    ),
    (
        "def exit_fx(dk, n) {\n  deck $dk: kick x ~ x ~\n  at bar $n: control: move filter_exit on $dk over 4 bars\n}\nexit_fx(A, 8)\n",
        "def exit_fx(dk, n) {\n  deck $dk: kick x ~ x ~\n  at bar $n: move filter_exit on $dk over 4 bars\n}\nexit_fx(A, 8)\n",
    ),
    // a guard that reads an input is decided while the program plays, on the control deck as off it
    ("deck A: kick x ~ x ~\non phrase: control: swap pairs when knob(1) > 0.5\n", "deck A: kick x ~ x ~\non phrase: swap pairs when knob(1) > 0.5\n"),
];

#[test]
fn the_control_deck_lowers_to_what_the_line_with_no_head_lowers_to() {
    for (deck, bare) in PAIRS {
        let (a, b) = (lowered(deck), lowered(bare));
        assert_eq!(a.0, b.0, "the runtime controls differ:\n{deck}\n{bare}");
        assert_eq!(a.1, b.1, "the nets differ:\n{deck}\n{bare}");
        assert_eq!(a.2, b.2, "the control net's lines and events differ:\n{deck}\n{bare}");
        assert_eq!(a.3, b.3, "the tempo maps differ:\n{deck}\n{bare}");
        assert!(a.4.tempo_map.same_tempos(&b.4.tempo_map), "{deck}");
        // two texts, so two CIDs: the control deck is its own spelling
        assert_ne!(a.4.cid, b.4.cid, "{deck}");
        assert!(a.4.text.contains("control: ") && !b.4.text.contains("control:"), "{}", a.4.text);
    }
}

#[test]
fn the_control_decks_canonical_text_is_a_fixed_point_and_survives_reformatting() {
    for (n, (src, _)) in PAIRS.iter().enumerate() {
        let c = compiled(src);
        // its own canonical spelling: printed with its head, read back to itself
        let again = compiled(&c.text);
        assert_eq!(again.text, c.text, "pair {n}: not a fixed point");
        assert_eq!(again.cid, c.cid);
        let mut rng = Rng(0x0c07_7201u64 ^ (n as u64 * 0x9e37_79b9));
        for round in 0..60 {
            let text = reformat(src, &mut rng);
            let r = check(&text);
            let got = r.compiled.unwrap_or_else(|| panic!("pair {n} round {round} no longer compiles:\n{text}\n{:?}", r.diagnostics));
            assert_eq!(got.cid, c.cid, "pair {n} round {round} forked:\n{text}");
        }
    }
    // the canonical text of the head: one space after its colon, after the time word's
    let c = compiled("deck A: kick x ~ x ~\nat bar 12:control:swap   pairs with filter_exit\n");
    assert!(c.text.contains("\nat bar 12: control: swap pairs with filter_exit\n"), "{}", c.text);
    // an edit of the call changes the CID
    assert_ne!(compiled("control: move echo_out on B\n").cid, compiled("control: move echo_out on A\n").cid);
}

/// The first error of `src`: its message and help.
fn error(src: &str) -> (String, String) {
    let c = check(src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("no error for:\n{src}"));
    (d.message.clone(), d.help.clone().unwrap_or_default())
}

#[test]
fn the_control_deck_holds_no_part_and_no_strip_action() {
    let k = "deck A: kick x ~ x ~\n";
    // parts and notes
    for line in ["control: kick x ~ x ~", "control: bass c2 ~ c2 ~ | kick x", "control: stack(kick x, snare ~ x)", "at bar 4: control: melody c4"] {
        let (m, h) = error(&format!("{k}{line}\n"));
        assert!(m.contains("never parts or notes"), "{line}: {m}");
        assert!(h.contains("deck A: kick x ~ x ~"), "{line}: {h}");
    }
    // a strip's action: on its deck or the master
    for line in ["control: filter lp 400", "control: delay 0.4", "control: drive 0.6", "control: level 0.8", "control: low 0", "at bar 8: control: reverb 0.3 -> 0 over 4 bars", "control: filter off"] {
        let (m, h) = error(&format!("{k}{line}\n"));
        assert!(m.contains("not a line of the control deck"), "{line}: {m}");
        assert!(h.contains("`deck A:`, `deck B:` or `master:`"), "{line}: {h}");
    }
    // a deck's transport
    for line in ["control: play", "control: mute", "control: load techno:42"] {
        let (m, h) = error(&format!("{k}{line}\n"));
        assert!(m.contains("is a deck's"), "{line}: {m}");
        assert!(h.contains("deck A:") || h.contains("deck B: load"), "{line}: {h}");
    }
    // a sung part, a def's call, a setting
    assert!(error("deck B: melody c4 e4\ncontrol: sing \"la la\" on melody\n").0.contains("`sing` sings a deck's part"));
    let (m, h) = error("def fx_out(dk) {\n  deck $dk: kick x\n}\ncontrol: fx_out(A)\n");
    assert!(m.contains("is a def's call") && h.contains("inside the def"), "{m} / {h}");
    assert!(error(&format!("{k}control: key am\n")).0.contains("`key` is a setting"));
    assert!(error(&format!("{k}control: swing 0.1\n")).0.contains("`swing` is a setting"));
    // a tempo with no time word is the setting
    let (m, h) = error(&format!("{k}control: tempo 140\n"));
    assert!(m.contains("a tempo with no time word is a setting") && h.contains("at bar 16: control: tempo"), "{m} / {h}");
    let (m, _) = error(&format!("section s 4 bars {{\n  {k}  control: tempo 140\n}}\narrange s\n"));
    assert!(m.contains("a tempo with no time word is a setting"), "{m}");
    // a repeating tempo is refused as it is with no head
    assert!(error(&format!("{k}every 8 bars: control: tempo 140\n")).0.contains("this time word repeats"));
    // one head a line, and the time word before it
    for line in ["deck A: control: move echo_out on B", "master: control: queue next", "control: deck A: kick x ~ x ~", "control: master: filter lp 400", "control: control: queue next"] {
        let (m, h) = error(&format!("{k}{line}\n"));
        assert!(m.contains("a line has one head"), "{line}: {m}");
        assert!(h.contains("control: move echo_out on B"), "{line}: {h}");
    }
    let (m, h) = error(&format!("{k}control: at bar 4: queue next\n"));
    assert!(m.contains("is a time word: it comes before the head") && h.contains("at bar 12: control:"), "{m} / {h}");
    // a typo, and what is not built yet
    let (m, h) = error(&format!("{k}control: mvoe echo_out on B\n"));
    assert!(m.contains("not an engine call") && h.contains("did you mean `move`?"), "{m} / {h}");
    let (m, h) = error(&format!("{k}control: macro riser over 8 bars on A\n"));
    assert!(m.contains("`macro` is designed and not built yet") && h.contains("aux macro=cathedral"), "{m} / {h}");
    assert!(error(&format!("{k}macro riser over 8 bars on A\n")).1.contains("designed and not built yet"));
    // the checks of the call itself are the line's with no head: a move needs its deck, a pair swap a move it can use
    assert!(error(&format!("{k}control: move echo_out\n")).0.contains("needs the deck it acts on"));
    assert!(error(&format!("{k}control: swap pairs with bass_swap\n")).0.contains("is not a move a pair swap can use"));
    assert!(error(&format!("{k}control: move bass_swap on B over 2 beats\n")).0.contains("a move takes whole bars"));
}

#[test]
fn a_note_deck_keeps_the_rules_it_had() {
    // the engine calls with no target still refuse one, and say so as before
    let k = "deck A: kick x ~ x ~\n";
    assert!(error(&format!("{k}deck A: move echo_out on B\n")).0.contains("takes no target"));
    assert!(error(&format!("{k}master: swap pairs\n")).0.contains("takes no target"));
    assert!(error(&format!("{k}deck B: crossfader 1\n")).0.contains("`crossfader` takes no target"));
    assert!(error(&format!("{k}at bar 4: master: tempo 140\n")).0.contains("`tempo` takes no target"));
    // and a strip's action still needs one
    assert!(error(&format!("{k}filter lp 400\n")).0.contains("`filter` needs a target"));
}

#[test]
fn control_is_a_free_name() {
    assert!(djlive::parse::reserved("control").is_none(), "`control` must stay a name: it is the head only before a colon");
    // a section, a let pattern, a def and a scalar let named `control`, each read where it is a name
    for src in [
        "section control 4 bars {\n  deck A: kick x ~ x ~\n  control: queue next\n}\nafter control: control: move echo_out on B\narrange control\n",
        "let control = x ~ x ~\ndeck A: kick control\ncontrol: move echo_out on B\n",
        "def control(dk) {\n  deck $dk: kick x ~ x ~\n  control: move echo_out on $dk\n}\ncontrol(A)\n",
        "let control = 4\ndeck A: kick x ~ x ~\nat bar control: control: queue next\n",
    ] {
        let c = compiled(src);
        assert!(!c.controls.is_empty(), "{src}");
        assert_eq!(compiled(&c.text).cid, c.cid, "{src}");
    }
    // the scalar let is put in place where it is a name, and not in the head
    let c = compiled("let control = 4\ndeck A: kick x ~ x ~\nat bar control: control: queue next\n");
    assert!(c.text.contains("at bar 4: control: queue next"), "{}", c.text);
}

#[test]
fn the_head_is_coloured_as_a_head_and_the_name_as_a_name() {
    let src = "control: move echo_out on B\nafter control: control: queue next\nlet control = x ~ x ~\n";
    let cls: Vec<(&str, Class, usize)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c, s.start)).collect();
    let control: Vec<Class> = cls.iter().filter(|(t, _, _)| *t == "control").map(|(_, c, _)| *c).collect();
    assert_eq!(control, vec![Class::Keyword, Class::Word, Class::Keyword, Class::Word], "{cls:?}");
}

#[test]
fn the_programs_written_before_it_keep_their_cids() {
    // the pins of crates/beatscript-api/tests/cid-pins.json (written before the control deck), for every example program this crate
    // compiles on its own (one that needs a host's store, its registry or its voices is beatscript-api's cid_pins to check)
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pins = JsonValue::parse(&std::fs::read_to_string(root.join("crates/beatscript-api/tests/cid-pins.json")).unwrap()).unwrap();
    let mut seen = 0;
    for (path, pin) in pins.as_object().unwrap().iter() {
        if !path.starts_with("examples/beatscript/") || pin.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(path)) else { continue };
        assert!(!src.lines().any(|l| l.trim_start().starts_with("control:")), "{path} has a control deck: it is not a program from before it");
        let Some(c) = check(&src).compiled else { continue };
        // the examples are BeatScript 2 now (a new CID by design: djlive::version); their BeatScript 1 CID is the pin
        let legacy = djlive::compile::program_cid(djlive::version::legacy_canonical(&c.text));
        assert_eq!(Some(legacy.as_str()), pin.get("legacy_cid").or(pin.get("cid")).and_then(JsonValue::as_str), "{path} changed its CID");
        seen += 1;
    }
    assert!(seen >= 20, "compared only {seen} programs");
}
