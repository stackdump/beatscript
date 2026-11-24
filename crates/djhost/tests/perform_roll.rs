//! The loop roll on the real engine (`djhost::perform`, with djrust's `Engine` as the `MoveHost`): on a net deck it is the
//! engine's own net loop, 4 → 2 → 1 → ½ beat from the bar line, a bar a stage, let go on the drop, and the halves and the
//! exit are deck actions the session records. On a sample deck with a grid it is the sample loop, as it always was.

mod common_track;

use common_track::{Stepper, BLOCK};
use djengine::json::JsonValue;
use djengine::session::action::DeckAction;
use djengine::session::model::SessionEventKind;
use djengine::TrackSpec;
use djhost::perform::{MoveHost, MoveKind, MoveSpec, Moves};

const SR: f64 = 48_000.0;

/// A techno song at 480 bpm: a step (a quarter beat) is 1500 frames, a bar 24 000.
fn fast() -> TrackSpec {
    let mut s = TrackSpec::new("techno", 42.0, Some("standard"));
    s.extra.set("bpm", JsonValue::Number(480.0));
    s
}

/// Service the moves before each block, as the booth's service pass does, until they are idle; the frame each time `read`
/// changes, with what it reads then (the first entry is what it read when the roll was armed).
fn run_roll<T: PartialEq + Copy + std::fmt::Debug>(st: &mut Stepper, m: &mut Moves, read: impl Fn(&Stepper) -> T) -> Vec<(i64, T)> {
    let mut seen = vec![(st.frame, read(st))];
    for _ in 0..4000 {
        if m.is_idle() {
            break;
        }
        m.service(&mut st.e, st.frame as f64 / SR);
        st.block();
        let now = read(st);
        if seen.last().unwrap().1 != now {
            seen.push((st.frame, now));
        }
    }
    assert!(m.is_idle(), "the roll is over: {seen:?}");
    // a few blocks more: the exit has landed
    st.step(4);
    let now = read(st);
    if seen.last().unwrap().1 != now {
        seen.push((st.frame, now));
    }
    seen
}

fn loop_actions(s: &djengine::session::model::Session) -> Vec<String> {
    s.events
        .iter()
        .filter_map(|e| match &e.kind {
            SessionEventKind::Action(DeckAction::Loop { deck, op }) => Some(format!("{deck} {op}")),
            _ => None,
        })
        .collect()
}

#[test]
fn a_net_deck_rolls_its_own_loop_four_two_one_half_from_the_bar_line() {
    let mut st = Stepper::new(SR, true);
    assert!(st.e.generate("A", &fast()));
    st.e.start_session().unwrap(); // a session starts with the decks at rest
    st.e.play("A");
    st.run_to_tick("A", 22); // bar 2 has begun
    assert!(MoveHost::can_loop(&st.e, "A"), "a net deck loops");
    let mut m = Moves::new();
    m.arm(&mut st.e, MoveSpec::new(MoveKind::LoopRoll, "A")).unwrap(); // a bar a stage
    let seen = run_roll(&mut st, &mut m, |s| s.e.net_loop("A"));
    let windows: Vec<_> = seen.iter().map(|s| s.1).collect();
    // from the next bar line (step 32, bar 3), each stage half the last, then let go
    let l = 32;
    assert_eq!(windows, [None, Some((l, l + 16)), Some((l, l + 8)), Some((l, l + 4)), Some((l, l + 2)), None], "{seen:?}");
    // a bar a stage (to a block or two: the roll counts its beats on the service clock)
    for w in seen[1..].windows(2) {
        let d = w[1].0 - w[0].0;
        assert!((d - 24_000).abs() <= 2 * BLOCK as i64, "a stage is a bar: {d} frames ({seen:?})");
    }
    assert!(m.last_word(MoveKind::LoopRoll, "A").unwrap().contains("released on the drop"));
    // the halves and the exit are deck actions: the action log carries the roll
    let s = st.e.finish_session().unwrap();
    assert_eq!(loop_actions(&s), ["A halve", "A halve", "A halve", "A exit"]);
    // and the deck plays on from where it was
    let t = st.tick("A");
    st.step(40);
    assert!(st.tick("A") > t);
}

#[test]
fn a_cancelled_net_roll_lets_go_at_once() {
    let mut st = Stepper::new(SR, true);
    assert!(st.e.generate("A", &fast()));
    st.e.play("A");
    st.run_to_tick("A", 6);
    let mut m = Moves::new();
    let id = m.arm(&mut st.e, MoveSpec::new(MoveKind::LoopRoll, "A")).unwrap();
    assert!(st.until(1000, |s| {
        m.service(&mut s.e, s.frame as f64 / SR);
        s.e.net_loop("A") == Some((16, 32))
    }));
    assert!(m.cancel(&mut st.e, id));
    st.step(2);
    assert_eq!(st.e.net_loop("A"), None);
}

#[test]
fn a_sample_deck_rolls_its_sample_loop_as_before_and_records_no_deck_action() {
    let mut st = Stepper::new(SR, true);
    assert!(st.e.load_demo("A")); // 120 bpm, with a grid: a beat is 24 000 frames
    st.step(1);
    st.e.start_session().unwrap();
    st.e.play("A");
    st.step(100);
    assert!(MoveHost::can_loop(&st.e, "A"));
    let mut m = Moves::new();
    m.arm(&mut st.e, MoveSpec::new(MoveKind::LoopRoll, "A")).unwrap();
    let spb = SR * 0.5;
    let seen = run_roll(&mut st, &mut m, |s| {
        let x = s.e.deck("A").unwrap().sample.as_ref().unwrap();
        match (x.looping, x.loop_start, x.loop_end) {
            (true, Some(a), Some(b)) => Some(((b - a) / spb * 1000.0).round() / 1000.0),
            _ => None,
        }
    });
    let beats: Vec<_> = seen.iter().map(|s| s.1).collect();
    assert_eq!(beats, [None, Some(4.0), Some(2.0), Some(1.0), Some(0.5), None], "{seen:?}");
    let s = st.e.finish_session().unwrap();
    assert!(loop_actions(&s).is_empty(), "a sample deck's roll calls the loop as it did: {:?}", loop_actions(&s));
}
