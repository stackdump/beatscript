//! The control deck per pass (docs/beatscript/ext-control.md "Per pass: the switch", docs/ARRANGE.md): a line of an `arrange { }`
//! switches the control deck, `~ control off` from here on, `@ control off` for the next section only, `~ control freeze off` the
//! lines of one call, `~3 control off` on every 3rd pass. A section played with lines held back is a copy of it without them
//! (`drop@2`): their controls, tempo events and the spans the playhead lights are not there, and check's table says which calls
//! each play holds back. The held-back lines are still checked; `control` stays a free name; the text is canonical.

mod common;

use common::{reformat, Rng};
use djlive::ast::StmtKind;
use djlive::compile::{check, Compiled};
use djlive::diag::{render_all, Diagnostic, Severity};
use djlive::mutate::{generations, ArrItem, GenTable, Mutation};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn table(src: &str) -> GenTable {
    generations(src, &djlive::env::Env::none()).unwrap_or_else(|| panic!("no generation table:\n{src}\n{}", render_all(&check(src).diagnostics, src)))
}

fn diags(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics
}

fn has(ds: &[Diagnostic], sev: Severity, msg: &str) -> Option<Diagnostic> {
    ds.iter().find(|d| d.severity == sev && d.message.contains(msg)).cloned()
}

/// Three sections: `drop` has two lines on its control deck (a `mix` and a `freeze`), `lift` two more, reached through a
/// repeat and a def's call (a `crossfader` and a `move`), `calm` none.
fn song(block: &str) -> String {
    format!(
        "tempo 120\nkey am\nlet acid = a2 c3 e3 g3\n\
         def fx(dk) {{\n  at bar 1: control: move filter_exit on $dk over 1 bars\n}}\n\
         section drop 2 bars {{\n  deck A: kick x ~ x ~\n  deck B: bass acid\n  control: mix {{ B level 0.5 }}\n  at bar 1: control: freeze phrase bars=2\n}}\n\
         section lift 2 bars {{\n  deck A: kick x x x x\n  repeat 2 {{\n    at bar $i: control: crossfader 0.5\n  }}\n  fx(B)\n}}\n\
         section calm 2 bars {{\n  deck A: kick x ~ ~ ~\n}}\n{block}\n"
    )
}

/// Each play: its lowered name and the calls it holds back.
fn plays(t: &GenTable) -> Vec<(String, Vec<String>)> {
    t.plays.iter().map(|p| (p.name.clone(), p.control_off.clone())).collect()
}

fn p(name: &str, off: &[&str]) -> (String, Vec<String>) {
    (name.to_string(), off.iter().map(|s| s.to_string()).collect())
}

/// The runtime controls of the placement whose path starts so (`[drop@2@4] `).
fn controls_at(c: &Compiled, path: &str) -> Vec<String> {
    c.controls.iter().filter(|k| k.key.starts_with(path)).map(|k| k.key[path.len()..].to_string()).collect()
}

fn mutations(src: &str) -> Vec<Mutation> {
    let p = djlive::parse::parse(src);
    assert!(p.ok(), "{}", render_all(&p.diagnostics, src));
    p.program
        .stmts
        .iter()
        .filter_map(|s| if let StmtKind::ArrangeBlock(b) = &s.kind { Some(b.items.clone()) } else { None })
        .flatten()
        .filter_map(|i| if let ArrItem::Mutation(m) = i { Some(m) } else { None })
        .collect()
}

#[test]
fn off_holds_back_the_lines_of_the_sections_after_it_and_on_brings_them_back() {
    let src = song("arrange {\n  drop lift\n  ~ control off\n  drop lift calm\n  ~ control on\n  drop\n}");
    let t = table(&src);
    assert_eq!(plays(&t), [p("drop", &[]), p("lift", &[]), p("drop@2", &["freeze", "mix"]), p("lift@2", &["crossfader", "move"]), p("calm", &[]), p("drop", &[])]);
    let c = ok(&src);
    // the first play of each fires its lines, the copies fire none, the last drop fires them again
    let first_drop = controls_at(&c, "[drop@0] ");
    assert!(first_drop.iter().any(|k| k.contains("freeze phrase bars=2")) && first_drop.iter().any(|k| k.starts_with("mix")), "{first_drop:?}");
    assert!(controls_at(&c, "[drop@2@4] ").is_empty(), "{:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    assert_eq!(controls_at(&c, "[drop@10] "), first_drop);
    // lines through a repeat and a def's call are the section's too
    let lift = controls_at(&c, "[lift@2] ");
    assert_eq!(lift.iter().filter(|k| k.contains("crossfader 0.5")).count(), 2, "{lift:?}");
    assert!(lift.iter().any(|k| k.contains("move filter_exit on B")), "{lift:?}");
    assert!(controls_at(&c, "[lift@2@6] ").is_empty());
    // the segments: the copies are there, and the playhead does not light the held-back lines in them
    let names: Vec<&str> = c.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["drop", "lift", "drop@2", "lift@2", "calm", "drop"]);
    let at = src.find("at bar 1: control: freeze").unwrap();
    let lit = |i: usize| c.segments[i].lines.iter().any(|s| s.start <= at && at < s.end);
    assert!(lit(0) && !lit(2) && lit(5), "the freeze line is lit in the plays that fire it");
    // a section with no control-deck line is never copied for a switch
    assert!(!names.contains(&"calm@2"));
}

#[test]
fn a_one_shot_is_for_the_next_section_only() {
    let t = table(&song("arrange {\n  drop\n  @ control off\n  drop\n  drop\n}"));
    assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["freeze", "mix"]), p("drop", &[])]);
    // on top of what the switches say from here on: the next section fires, then the deck is off again
    let t = table(&song("arrange {\n  ~ control off\n  drop\n  @ control on\n  drop\n  drop\n}"));
    assert_eq!(plays(&t), [p("drop", &["freeze", "mix"]), p("drop@2", &[]), p("drop", &["freeze", "mix"])]);
    // a one-shot is laid over the `~` state the next section has: a `~` written after it still reaches that section, in either
    // order, on the whole deck or another call; on the one-shot's own call the one-shot wins
    let t = table(&song("arrange {\n  drop\n  @ control freeze off\n  ~ control off\n  drop\n  drop\n}"));
    assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["freeze", "mix"]), p("drop@2", &["freeze", "mix"])]);
    for block in ["arrange {\n  drop\n  @ control freeze off\n  ~ control mix off\n  drop\n  drop\n}", "arrange {\n  drop\n  ~ control mix off\n  @ control freeze off\n  drop\n  drop\n}"] {
        let t = table(&song(block));
        assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["freeze", "mix"]), p("drop@3", &["mix"])], "{block}");
    }
    let t = table(&song("arrange {\n  drop\n  @ control freeze off\n  ~ control freeze on\n  drop\n  drop\n}"));
    assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["freeze"]), p("drop", &[])]);
    // a one-shot at the end of a pass is read by the next pass's first section
    let t = table(&song("arrange {\n  drop calm\n  @ control off\n} loop 2 then end"));
    assert_eq!(plays(&t), [p("drop", &[]), p("calm", &[]), p("drop@2", &["freeze", "mix"]), p("calm", &[])]);
    let c = ok(&song("arrange {\n  drop calm\n  @ control off\n} loop 2 then end"));
    assert!(c.warnings.iter().any(|d| d.message.contains("has no next section")), "the last pass's one-shot plays nowhere");
}

#[test]
fn a_call_switches_its_own_lines() {
    let src = song("arrange {\n  drop lift\n  ~ control freeze off\n  drop lift\n  ~ control off\n  ~ control move on\n  drop lift\n}");
    let t = table(&src);
    assert_eq!(plays(&t), [p("drop", &[]), p("lift", &[]), p("drop@2", &["freeze"]), p("lift", &[]), p("drop@3", &["freeze", "mix"]), p("lift@2", &["crossfader"])]);
    let c = ok(&src);
    let second = controls_at(&c, "[drop@2@4] ");
    assert!(!second.is_empty() && second.iter().all(|k| k.starts_with("mix")), "the mix alone: {second:?}");
    let last_lift = controls_at(&c, "[lift@2@10] ");
    assert!(!last_lift.is_empty() && last_lift.iter().all(|k| k.contains("move filter_exit")), "the move alone: {last_lift:?}");
}

/// `swap` (the `swap pairs` call) and `voice` (a sugar noun) are also mutation operators: before `on` or `off` at the end of the
/// line they are the switch's calls, as the line would be refused as a mutation. An operator that takes a pattern stays a mutation.
/// A macro line's call is `macro`, whatever its version and wherever the line is read from (a section's own line, a def's body
/// still in tokens): `~ control macro off` holds both back, and the play's table names it once.
#[test]
fn a_macro_line_is_switched_as_macro() {
    let src = "tempo 120\nkey am\n\
               def lift(dk) {\n  at bar 1: control: macro@1 cathedral on $dk over 1 bar\n}\n\
               section drop 2 bars {\n  deck A: kick x ~ x ~\n  control: macro riser on A over 2 bars\n  lift(B)\n}\n\
               arrange {\n  drop\n  ~ control macro off\n  drop\n}\n";
    let t = table(src);
    assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["macro"])]);
    let c = ok(src);
    let first = controls_at(&c, "[drop@0] ");
    assert!(first.iter().any(|k| k.contains("macro riser")) && first.iter().any(|k| k.contains("macro cathedral")), "{first:?}");
    assert!(controls_at(&c, "[drop@2@2] ").is_empty(), "{:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    // `macro@1` is no call name of its own: the switch names the call, and its help names `macro`
    let wrong = src.replace("~ control macro off", "~ control macro@1 off");
    let d = has(&diags(&wrong), Severity::Error, "the control deck has no `macro@1` line");
    assert!(d.as_ref().is_some_and(|d| d.help.as_deref().is_some_and(|h| h.contains("macro"))), "{d:?}");
}

#[test]
fn a_call_that_is_also_an_operator_is_switched() {
    let src = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  at bar 1: control: swap pairs with filter_exit\n  control: voice robot\n}\n\
               arrange {\n  drop\n  ~ control swap off\n  drop\n  ~ control off\n  @ control voice on\n  drop\n  drop\n}\n";
    let ms = mutations(src);
    let calls: Vec<(Option<String>, bool, bool)> = ms.iter().map(|m| (m.switch.as_ref().and_then(|s| s.call.as_ref().map(|c| c.text.clone())), m.switch.as_ref().is_some_and(|s| s.on), m.once)).collect();
    assert_eq!(calls, [(Some("swap".into()), false, false), (None, false, false), (Some("voice".into()), true, true)]);
    let t = table(src);
    assert_eq!(plays(&t), [p("drop", &[]), p("drop@2", &["swap"]), p("drop@2", &["swap"]), p("drop@3", &["swap", "voice"])]);
    assert!(ok(src).text.contains("\n  ~ control swap off\n"));
    // `xfer` takes a pattern: `~ control xfer off` is a mutation of a `let control` (by a `let off`)
    let ms = mutations("tempo 120\nlet control = x ~ x ~\nsection drop 1 bars {\n  deck A: kick control\n}\narrange {\n  drop\n  ~ control xfer off\n  drop\n}\n");
    assert!(ms[0].switch.is_none() && ms[0].targets[0].ops[0].name.text == "xfer");
}

#[test]
fn a_counted_switch_fires_on_every_nth_pass() {
    let src = song("arrange {\n  drop\n  ~2 control off\n  @3 control on\n  drop\n} loop 4 then end");
    let t = table(&src);
    let (f, e): (&[&str], &[&str]) = (&["freeze", "mix"], &[]);
    let offs: Vec<Vec<String>> = t.plays.iter().map(|p| p.control_off.clone()).collect();
    let want: Vec<Vec<String>> = [e, e, e, f, f, e, f, f].iter().map(|o| o.iter().map(|s| s.to_string()).collect()).collect();
    assert_eq!(offs, want);
    assert_eq!(t.fires.len(), 2);
    assert_eq!((t.fires[0].mutation, t.fires[0].every, t.fires[0].once, t.fires[0].passes.clone(), t.fires[0].plays.clone()), (0, 2, false, vec![1, 3], vec![Some(3), Some(7)]));
    assert_eq!((t.fires[1].mutation, t.fires[1].every, t.fires[1].once, t.fires[1].passes.clone(), t.fires[1].plays.clone()), (1, 3, true, vec![2], vec![Some(5)]));
    // a count the block never reaches is the warning a counted mutation has
    let ds = diags(&song("arrange {\n  drop\n  ~3 control off\n  drop\n}"));
    assert!(has(&ds, Severity::Warning, "never fires").is_some(), "{ds:?}");
}

#[test]
fn a_named_block_switches_its_own_plays_and_leaves_the_section_as_written() {
    let src = song("arrange open: drop then set\narrange set: {\n  ~ control off\n  drop\n}");
    let t = table(&src);
    assert_eq!(plays(&t), [p("drop@2", &["freeze", "mix"])]);
    let c = ok(&src);
    assert_eq!(controls_at(&c, "[drop@0] ").len(), 2, "the other arrangement plays it as written");
    assert!(controls_at(&c, "[drop@2@2] ").is_empty());
}

#[test]
fn the_switch_is_canonical_and_keeps_its_text_through_reformatting() {
    let src = song("arrange {\n  drop lift\n  ~3 control  freeze   off\n  @ 2 control on\n  drop\n} loop 6 then calm end");
    let c = ok(&src);
    assert!(c.text.contains("\n  ~3 control freeze off\n  @2 control on\n"), "{}", c.text);
    let again = ok(&c.text);
    assert_eq!((again.text.clone(), again.cid.clone()), (c.text.clone(), c.cid.clone()));
    let mut rng = Rng(0x0c71_9a55);
    for round in 0..40 {
        let text = reformat(&src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{:?}", r.diagnostics));
        assert_eq!(got.cid, c.cid, "round {round} forked:\n{text}");
    }
    // the syntax: a switch is a mutation line with no targets
    let ms = mutations(&src);
    let s = ms[0].switch.as_ref().expect("a switch");
    assert_eq!((ms[0].every, ms[0].once, s.call.as_ref().map(|w| w.text.as_str()), s.on), (3, false, Some("freeze"), false));
    assert!(ms[0].targets.is_empty());
    assert_eq!((ms[1].every, ms[1].once, ms[1].switch.as_ref().map(|s| (s.call.is_none(), s.on))), (2, true, Some((true, true))));
    // a switch is in the CID: off and on are two programs
    let flipped = ok(&src.replace("@ 2 control on", "@ 2 control off"));
    assert_ne!(flipped.cid, c.cid);
    // the same text, the same lowering
    let (a, b) = (ok(&src), ok(&src));
    assert_eq!(a.controls, b.controls);
    assert_eq!(table(&src), table(&src));
}

#[test]
fn control_is_still_a_name_a_let_can_have() {
    let src = "tempo 120\nlet control = x ~ x ~\nsection drop 1 bars {\n  deck A: kick control\n}\narrange {\n  drop\n  ~ control rot 1\n  drop\n}\n";
    let ms = mutations(src);
    assert!(ms[0].switch.is_none());
    assert_eq!(ms[0].targets[0].name.text, "control");
    let t = table(src);
    assert!(t.patterns.contains_key("control"), "{:?}", t.patterns.keys().collect::<Vec<_>>());
    assert_eq!(ok(src).text.matches("~ control rot 1").count(), 1);
}

#[test]
fn what_a_switch_refuses_and_what_it_warns_about() {
    // a call no control-deck line has: the nearest name
    let ds = diags(&song("arrange {\n  drop\n  ~ control crossfadr off\n  lift\n}"));
    let d = has(&ds, Severity::Error, "the control deck has no `crossfadr` line").unwrap_or_else(|| panic!("{ds:?}"));
    assert_eq!(d.help.as_deref(), Some("did you mean `crossfader`?"));
    // a line of its own
    for line in ["~ acid rot 2, control off", "~ control off, acid rot 2", "@ control freeze on, acid up 1"] {
        let src = song(&format!("arrange {{\n  drop\n  {line}\n  drop\n}}"));
        let ds = diags(&src);
        assert!(has(&ds, Severity::Error, "a switch of the control deck is a line of its own").is_some(), "{line}: {ds:?}");
    }
    // `on` or `off`
    for line in ["~ control", "@ control freeze", "~3 control"] {
        let ds = diags(&song(&format!("arrange {{\n  drop\n  {line}\n  drop\n}}")));
        let d = has(&ds, Severity::Error, "says `on` or `off`").unwrap_or_else(|| panic!("{line}: {ds:?}"));
        assert!(d.help.as_deref().unwrap_or("").contains("control off"), "{d:?}");
    }
    // more than a call before its `on` or `off`: said as a switch, not as a mutation of a `let control`
    for line in ["@ control foo bar off", "~ control 3 off", "~ control move echo_out on"] {
        let ds = diags(&song(&format!("arrange {{\n  drop\n  {line}\n  drop\n}}")));
        let d = has(&ds, Severity::Error, "is `control`, at most one call, and `on` or `off`").unwrap_or_else(|| panic!("{line}: {ds:?}"));
        assert!(d.help.as_deref().unwrap_or("").contains("~3 control off") || d.help.as_deref().unwrap_or("").contains("@3 control off"), "{d:?}");
        assert!(!ds.iter().any(|d| d.message.contains("is not a `let` pattern")), "{line}: {ds:?}");
    }
    // nothing on the control deck in a section: an engine call with no head is not on it, and the help says so
    let headless = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  move echo_out on B\n}\narrange {\n  drop\n  ~ control off\n  drop\n}\n";
    let ds = diags(headless);
    let d = has(&ds, Severity::Warning, "no section has a line on the control deck").unwrap_or_else(|| panic!("{ds:?}"));
    assert!(d.help.as_deref().unwrap_or("").contains("has no head: control: move echo_out on B"), "{d:?}");
    let c = ok(headless);
    assert_eq!(c.controls.iter().filter(|k| k.key.contains("move echo_out on B")).count(), 2, "the line with no head fires in both plays");
    // the control deck's lines all outside the sections: a switch reaches none of them
    let outside = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n}\nevery 16 bars: control: freeze phrase bars=2\narrange {\n  drop\n  ~ control off\n  ~ control freeze off\n  drop\n}\n";
    let ds = diags(outside);
    let d = has(&ds, Severity::Warning, "no section has a line on the control deck").unwrap_or_else(|| panic!("{ds:?}"));
    assert!(d.help.as_deref().unwrap_or("").contains("outside every section"), "{d:?}");
    assert!(has(&ds, Severity::Warning, "every `freeze` line of the control deck is outside the sections").is_some(), "{ds:?}");
    assert!(check(outside).ok());
}

#[test]
fn a_line_the_switch_holds_back_is_still_checked() {
    // every play holds the section's line back: what the compiler says of it is said all the same
    let bad = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  for 1 bars: control: crossfader 1\n}\narrange {\n  ~ control off\n  drop\n}\n";
    let ds = diags(bad);
    let d = has(&ds, Severity::Error, "needs something that can be put back").unwrap_or_else(|| panic!("{ds:?}"));
    let plain = has(&diags(&bad.replace("  ~ control off\n", "")), Severity::Error, "needs something that can be put back").unwrap();
    assert_eq!((d.span, d.message), (plain.span, plain.message));
    // said once, where one play fires it and another holds it back
    let twice = bad.replace("  ~ control off\n  drop\n", "  drop\n  ~ control off\n  drop\n");
    assert_eq!(diags(&twice).iter().filter(|d| d.message.contains("needs something that can be put back")).count(), 1);
    // a held-back duck is read by the mix pass in its section: deck A has no snare to key it
    let duck = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  deck B: bass a1 ~ a1 ~\n  control: duck B under A.snare 6db\n}\narrange {\n  ~ control off\n  drop\n}\n";
    let held: Vec<(djlive::diag::Span, String)> = diags(duck).into_iter().filter(Diagnostic::is_error).map(|d| (d.span, d.message)).collect();
    let plain: Vec<(djlive::diag::Span, String)> = diags(&duck.replace("  ~ control off\n", "")).into_iter().filter(Diagnostic::is_error).map(|d| (d.span, d.message)).collect();
    assert!(!held.is_empty(), "the held-back duck is checked");
    assert_eq!(held, plain);
    // a held-back tempo is no event of the tempo map
    let tempo = "tempo 120\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  at bar 1: control: tempo 140\n}\narrange {\n  drop\n  ~ control off\n  drop\n}\n";
    let c = ok(tempo);
    let bpm: Vec<f64> = (0..4u32).map(|b| c.tempo_map.bpm_at(b)).collect();
    assert_eq!(bpm, [120.0, 140.0, 140.0, 140.0], "the first play's tempo stands; the second play says none of its own");
    let c = ok(&tempo.replace("  drop\n  ~ control off\n  drop\n", "  ~ control off\n  drop\n  ~ control on\n  drop\n"));
    let bpm: Vec<f64> = (0..4u32).map(|b| c.tempo_map.bpm_at(b)).collect();
    assert_eq!(bpm, [120.0, 120.0, 120.0, 140.0]);
}

#[test]
fn the_example_program_switches_its_control_deck() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/control-per-pass.beatscript")).unwrap();
    let c = ok(&src);
    let t = table(&src);
    assert!(t.plays.iter().any(|p| !p.control_off.is_empty()) && t.plays.iter().any(|p| p.control_off.is_empty()), "{:?}", plays(&t));
    assert!(t.plays.iter().any(|p| p.control_off.len() == 1), "one call switched alone: {:?}", plays(&t));
    assert!(c.text.contains("control off") && c.text.contains("control on"));
}
