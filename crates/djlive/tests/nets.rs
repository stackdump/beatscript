//! Control lines as a net (docs/design/NETS-ALL-THE-WAY-DOWN.md, phase 1): a ramp is a chain whose
//! transitions carry the values, a move is a subnet that puts its strip back by construction, and the
//! whole thing runs on the engine's own net executor.

use djlive::compile::{sweep_knob, Compiled};
use djlive::nets::{control_net, parse_label, Event, TICKS_PER_BAR};

fn compile(src: &str) -> Compiled {
    let c = djlive::check(src);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics);
    c.compiled.unwrap()
}

fn sets(ev: &[(i64, Event)], param: &str) -> Vec<(i64, f64)> {
    ev.iter().filter_map(|(t, e)| if let Event::Set { param: p, value, .. } = e { (p == param).then_some((*t, *value)) } else { None }).collect()
}

const RAMP: &str = "tempo 120\ndeck A: kick x ~ x ~\nmaster: filter lp 400 -> 8000 over 4 bars\n";

#[test]
fn a_ramp_is_a_chain_a_step_a_sixteenth() {
    let c = compile(RAMP);
    let n = control_net(&c, 8);
    assert!(n.residual.is_empty(), "{:?}", n.residual);
    let ev = n.events(None);
    // the filter is switched on, then one knob value per sixteenth over four bars, 65 in all
    assert_eq!(ev[0], (0, Event::Insert { target: "Master".into(), effect: "filter".into(), on: true }));
    let s = sets(&ev, "filter.sweep");
    assert_eq!(s.len(), 65);
    assert_eq!(s[0], (0, sweep_knob(400.0, false)));
    assert_eq!(s[64], (4 * TICKS_PER_BAR, sweep_knob(8000.0, false)), "it ends where it says, on the bar line it ends on");
    // the values are the ones the host's ramp sends: the net is a table of `Ramp::at`
    let Some(djlive::compile::Effect::Ramp(r)) = c.controls.iter().map(|c| &c.effect).find(|e| matches!(e, djlive::compile::Effect::Ramp(_))).cloned() else { panic!("no ramp") };
    for (t, v) in &s {
        let want = r.at(*t as f64 / 64.0).params.get("value").and_then(|v| v.as_f64()).unwrap();
        assert_eq!(*v, want, "tick {t}");
    }
    // after it the chain is spent: nothing fires in the other four bars
    assert!(ev.iter().all(|(t, _)| *t <= 64));
}

#[test]
fn a_ramp_starts_on_the_bar_it_is_tapped_to() {
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\nat bar 2: master: reverb 0.1 -> 0.6 over 2 bars\n");
    let n = control_net(&c, 8);
    let s = sets(&n.events(None), "reverb.mix");
    assert_eq!(s.first().map(|x| x.0), Some(2 * TICKS_PER_BAR), "the first value lands on bar 2's line, not a tick after");
    assert_eq!(s.last().map(|x| x.0), Some(4 * TICKS_PER_BAR));
    assert_eq!(s.last().map(|x| x.1), Some(0.6));
}

#[test]
fn an_every_line_fires_each_time_and_the_chains_overlap() {
    // a one-bar ramp every bar: each firing is its own token in the same chain
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\nevery 1 bars: master: filter lp 300 -> 2800 over 1 bars\n");
    let n = control_net(&c, 6);
    let ev = n.events(None);
    let s = sets(&ev, "filter.sweep");
    // fires on bars 1, 2, 3, 4, 5 (never 0): 17 values each, the end of one on the line the next begins on (two tokens, one chain)
    assert_eq!(s.len(), 5 * 17);
    assert_eq!(s.iter().filter(|(t, _)| t % TICKS_PER_BAR == 0).count(), 5 * 2, "each ramp starts and ends on a bar line");
    assert_eq!(s.iter().filter(|(t, _)| *t == 2 * TICKS_PER_BAR).count(), 2);
}

#[test]
fn the_events_are_typed_labels_that_round_trip() {
    for e in [
        Event::Set { target: "Master".into(), param: "filter.sweep".into(), value: 0.431 },
        Event::Set { target: String::new(), param: "crossfader".into(), value: 1.0 },
        Event::Insert { target: "A".into(), effect: "delay".into(), on: false },
        Event::Mute { deck: "B".into(), on: true },
        Event::Trick { deck: "A".into(), kind: "filter".into(), on: true, amount: Some(0.9), bars: Some(4.0), beats: None },
        Event::Trick { deck: "A".into(), kind: "wash".into(), on: false, amount: None, bars: None, beats: None },
    ] {
        assert_eq!(Event::parse(&e.text()).unwrap(), e);
    }
    assert_eq!(parse_label("mute A on ; trick A filter off").unwrap().len(), 2);
    assert!(Event::parse("explode A").is_err());
}

const MOVE: &str = "tempo 120\ndeck A: kick x ~ x ~\nat bar 4: move filter_exit on A over 2 bars\n";

#[test]
fn a_move_mutes_at_its_end_and_puts_the_strip_back_on_the_next_phrase_line() {
    let c = compile(MOVE);
    let n = control_net(&c, 16);
    assert!(n.residual.is_empty(), "{:?}", n.residual);
    let ev = n.events(None);
    let mute = |on| Event::Mute { deck: "A".into(), on };
    let at = |e: &Event| ev.iter().filter(|(_, x)| x == e).map(|(t, _)| *t).collect::<Vec<_>>();
    // bar 4: the filter rides; bar 6: the strip is cut and the filter off; bar 8: the phrase line puts it back
    assert_eq!(at(&Event::Trick { deck: "A".into(), kind: "filter".into(), on: true, amount: Some(0.9), bars: Some(2.0), beats: None }), vec![4 * 16]);
    assert_eq!(at(&mute(true)), vec![6 * 16]);
    assert_eq!(at(&mute(false)), vec![8 * 16]);
    // a restore comes of exactly one mute: nothing else unmutes, and it happens once
    assert_eq!(ev.iter().filter(|(_, e)| matches!(e, Event::Mute { .. })).count(), 2);
}

#[test]
fn a_move_that_ends_on_a_phrase_line_is_put_back_on_the_next_one() {
    // the end (bar 8) is itself a phrase line: the put-back is the first line strictly after it
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\nat bar 6: move filter_exit on A over 2 bars\n");
    let ev = control_net(&c, 24).events(None);
    let mute: Vec<(i64, bool)> = ev.iter().filter_map(|(t, e)| if let Event::Mute { on, .. } = e { Some((*t, *on)) } else { None }).collect();
    assert_eq!(mute, vec![(8 * 16, true), (16 * 16, false)]);
}

#[test]
fn two_moves_that_end_before_one_phrase_line_are_put_back_once() {
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\nat bar 1: move filter_exit on A over 1 bars\nat bar 3: move echo_out on A over 1 bars\n");
    let n = control_net(&c, 16);
    assert!(n.residual.is_empty(), "{:?}", n.residual);
    let ev = n.events(None);
    let mutes: Vec<(i64, bool)> = ev.iter().filter_map(|(t, e)| if let Event::Mute { on, .. } = e { Some((*t, *on)) } else { None }).collect();
    // two cuts (bars 2 and 4); each move's own put-back is on bar 8's line (one each: they are two nets of their own,
    // each unmuting the strip it muted: the same strip, so the same event twice, idempotent)
    assert_eq!(mutes.iter().filter(|m| m.1).count(), 2);
    assert!(mutes.iter().filter(|m| !m.1).all(|m| m.0 == 8 * 16));
    assert!(mutes.iter().any(|m| !m.1));
}

#[test]
fn a_strip_a_move_muted_is_never_both_muted_and_free() {
    // the net's own invariant, read off the marking at every sixteenth: muted + free = 1 (a P-invariant)
    let c = compile(MOVE);
    let n = control_net(&c, 16);
    for t in 0..(16 * 16) {
        let m = n.marking_at(t);
        let (muted, free) = (m.get("r0.muted").copied().unwrap_or(0), m.get("r0.free").copied().unwrap_or(0));
        assert_eq!(muted + free, 1, "tick {t}: {m:?}");
    }
}

#[test]
fn the_marking_is_the_state_of_the_control_lines_at_a_tick() {
    let c = compile(RAMP);
    let n = control_net(&c, 8);
    // mid ramp (bar 2): one token in the ramp's chain, one in the clock; the chain's place says how far it is
    let m = n.marking_at(32);
    let chain: Vec<&String> = m.keys().filter(|p| p.starts_with("r0.s")).collect();
    assert_eq!(chain.len(), 1, "{m:?}");
    // the events up to it are exactly the prefix of the stream: a seek is a marking computation
    let all = n.events(None);
    let upto = n.events(Some(32));
    assert_eq!(upto, all.iter().filter(|(t, _)| *t < 32).cloned().collect::<Vec<_>>());
}

#[test]
fn what_is_not_in_the_alphabet_stays_beside_the_net_and_says_so() {
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\nevery 4 bars: swap pairs with echo_out\nat bar 2: move bass_swap on B over 4 bars\nmaster: reverb 0.3\n");
    let n = control_net(&c, 8);
    let keys: Vec<&str> = n.residual.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys.len(), 2, "{:?}", n.residual);
    assert!(n.residual.iter().any(|(k, why)| k.contains("swap pairs") && why.contains("autodj")));
    assert!(n.residual.iter().any(|(k, why)| k.contains("bass_swap") && why.contains("phase 2")));
    // the reverb line is in
    let s = sets(&n.events(None), "reverb.mix");
    assert_eq!(s, vec![(0, 0.3)]);
}

#[test]
fn the_net_is_deterministic_and_its_cid_covers_every_value() {
    let a = control_net(&compile(RAMP), 8);
    let b = control_net(&compile(RAMP), 8);
    assert_eq!(a.cid(), b.cid());
    assert_eq!(a.model_cid(), b.model_cid());
    // one end of the sweep moved: another net, in both CIDs (the engine's model CID hashes `action` + `targetNet`)
    let c = control_net(&compile("tempo 120\ndeck A: kick x ~ x ~\nmaster: filter lp 400 -> 9000 over 4 bars\n"), 8);
    assert_ne!(a.cid(), c.cid());
    assert_ne!(a.model_cid(), c.model_cid());
}

#[test]
fn a_program_with_no_control_lines_has_no_net() {
    let c = compile("tempo 120\ndeck A: kick x ~ x ~\n");
    let n = control_net(&c, 8);
    assert_eq!(n.size(), (0, 0, 0));
    assert!(n.events(None).is_empty());
    assert!(n.residual.is_empty());
}

#[test]
fn deck_net_cids_do_not_move_and_the_performance_cid_sees_what_they_cannot() {
    // two programs with the same notes and different sweeps: today they share every net CID
    let a = compile(RAMP);
    let b = compile("tempo 120\ndeck A: kick x ~ x ~\nmaster: filter lp 400 -> 9000 over 4 bars\n");
    assert_eq!(a.net_cids(), b.net_cids(), "the deck nets do not hold the controls");
    assert_ne!(a.cid, b.cid, "the program CID (of the text) does");
    let (pa, pb) = (djlive::nets::performance_cid(&a, 8), djlive::nets::performance_cid(&b, 8));
    assert_ne!(pa, pb, "and so does the performance's, which is of what plays");
    // and two texts that play the same are the same performance, whatever the spelling
    let a2 = compile("tempo 120\n\ndeck A: kick x ~ x ~   # same\nmaster:   filter lp 400 -> 8k over 4 bars\n");
    assert_eq!(pa, djlive::nets::performance_cid(&a2, 8));
    // no control lines: the performance is its deck nets, the document says so and carries no control net
    let plain = compile("tempo 120\ndeck A: kick x ~ x ~\n");
    assert!(djlive::nets::performance_doc(&plain, 8).get("control").is_none());
}
