//! Auto-mix on the live engine (`AutoMixEngine.swift`): the countdown, the incoming track
//! composed onto the idle deck, the mix-out, the start that lands B's first tick on A's bar
//! line, the crossfader's steps, the pokes that pause or end it.
//!
//! Parity form (a): `AutoMixTests` (the rig, `started`, and every test that does not need the
//! session player or the journal) and `MixOutActionTests.theMixOutAheadIsTheHandMixOut`'s
//! twin, with the Swift literals. The Swift rig steps `OfflineConductor`; this one steps
//! `common_track::Stepper` (the same 128-frame blocks, the housekeeping before the block that
//! finds it due), whose housekeeping is `Engine::service`, which ends in `service_auto_mix`.
//! A Swift `perform(.play(...))` is `Rig::play`: `Engine::perform`, which notes the touch first
//! and then does the call, as `applyAction` does.
//! The `automixdeck` golden (`automixdeck_golden.rs`) holds the same calls to the Swift
//! engine's output.
//!
//! Not here: `aSetWithAutoMixReplaysToTheByte` and `aDeferIsRecordedAndReplayed`'s replay half
//! need `SessionPlayer`; `aCheckpointMidHandOffCarriesIt` needs `JournalState(engine:)`: its
//! engine half is `a_checkpoint_mid_hand_off_carries_it` (`restore_auto_mix`).

mod common_track;

use common_track::Stepper;
use djengine::automix::{AutoMix, AutoMixPhase, AutoMixSettings, AutoMixState};
use djengine::deck::turntable::SampleBuffer;
use djengine::session::action::DeckAction;
use djengine::session::model::{DeckSource, SessionEventKind};
use djengine::wander::WanderLock;
use djengine::wave::rests::MixOutTail;
use djengine::wave::TickAnchorFlags;
use djengine::TrackSpec;
use djhost::deck::sample::PreparedSample;
use djhost::deck::{PerformCtx, PreparedTrack};

const RATE: f64 = 22_050.0;

/// `AutoMixRig`: an offline two-deck set. A plays a composed net, B holds the demo loop (as at
/// launch). Both decks' tick anchors are read as it goes.
struct Rig {
    st: Stepper,
    /// The crossfader after each housekeeping tick (every other one), with A's and B's net
    /// ticks as that tick saw them.
    faders: Vec<(i64, f64, i64, i64)>,
}

impl Rig {
    fn new(spec: &TrackSpec) -> Rig {
        let mut st = Stepper::new(RATE, true);
        let pr = st.e.preparer("A").unwrap();
        let track = PreparedTrack::compose(spec, &pr, 0);
        assert!(st.e.load_net("A", track));
        assert!(st.e.load_demo("B"));
        for d in ["A", "B"] {
            st.r.deck_mut(d).unwrap().runner.record_events = true;
        }
        Rig {
            st,
            faders: Vec::new(),
        }
    }

    fn dnb() -> Rig {
        Rig::new(&TrackSpec::new("dnb", 42.0, None))
    }

    fn e(&mut self) -> &mut djhost::deck::Engine {
        &mut self.st.e
    }

    fn tick(&self, deck: &str) -> i64 {
        self.st.tick(deck)
    }

    /// One block.
    fn step(&mut self) {
        let housekeeping = self.st.frame % 512 == 0;
        let seen = (self.tick("A"), self.tick("B"));
        self.st.block();
        if housekeeping {
            self.faders
                .push((self.st.frame, self.st.e.crossfader, seen.0, seen.1));
        }
    }

    /// Step until `done` (at most `limit` blocks); false if it never came.
    fn run_until(&mut self, mut done: impl FnMut(&Rig) -> bool) -> bool {
        let mut n = 0;
        while !done(self) && n < 40_000 {
            self.step();
            n += 1;
        }
        done(self)
    }

    fn run_blocks(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    // MARK: perform, for the actions these tests use

    /// `DJEngine.perform(action)`: `Engine::perform` notes the touch, then does the call.
    fn perform(&mut self, a: DeckAction) {
        let applied = self.e().perform(&a, PerformCtx::default());
        assert!(applied.ran(), "{a:?}: {applied:?}");
    }

    /// `perform(.play(deck:, on:))`.
    fn play(&mut self, deck: &str, on: bool) {
        self.perform(DeckAction::Play {
            deck: deck.into(),
            on,
        });
    }

    /// `perform(.autoMix(on:, bars:, every:))`.
    fn auto_mix(&mut self, on: bool, bars: i64, every: i64) {
        self.perform(DeckAction::AutoMix { on, bars, every });
    }

    /// `perform(.autoMixNow)`.
    fn mix_now(&mut self) {
        self.perform(DeckAction::AutoMixNow);
    }

    /// `perform(.crossfader(v))`.
    fn crossfader(&mut self, v: f64) {
        self.perform(DeckAction::Crossfader(v));
    }

    /// `perform(.hotCue(deck:, slot:))`.
    fn hot_cue(&mut self, deck: &str, slot: usize) {
        self.perform(DeckAction::HotCue {
            deck: deck.into(),
            slot: slot as i64,
        });
    }

    /// `perform(.load(deck:, source: .net(spec)))`.
    fn load_net(&mut self, deck: &str, spec: &TrackSpec) {
        self.perform(DeckAction::Load {
            deck: deck.into(),
            source: DeckSource::Net(spec.clone()),
        });
    }

    /// Turn auto-mix on with `bars`, play A, Mix now: the hand-off bar is A's bar 8.
    fn started(&mut self, bars: i64) {
        self.auto_mix(true, bars, 0);
        self.play("A", true);
        self.run_blocks(4);
        self.mix_now();
        self.run_blocks(4);
    }

    /// The frame `deck` played step `step` of the project it was running then (its anchor
    /// `step + 1`, the tick count after it), the last such anchor.
    fn frame(&self, deck: &str, step: i64) -> Option<i64> {
        self.st
            .r
            .deck(deck)?
            .runner
            .anchors
            .iter()
            .rev()
            .find(|a| a.tick as i64 == step + 1 && !a.flags.contains(TickAnchorFlags::SWAP))
            .map(|a| a.frame)
    }

    fn run_in_flight(&self) -> bool {
        self.st.e.auto_mix.run.is_some()
    }
}

fn mixing(r: &Rig) -> bool {
    r.st.e.auto_mix.phase == AutoMixPhase::Mixing && r.tick("B") > 20
}

// AutoMixTests.aHandOffLandsOnItsBarsAndSwapsTheDecks
#[test]
fn a_hand_off_lands_on_its_bars_and_swaps_the_decks() {
    let mut rig = Rig::dnb();
    rig.e().deck_mut("A").unwrap().auto_advance = true; // auto-next must not restart the old deck
    rig.perform(DeckAction::Wander {
        deck: "A".into(),
        on: true,
    });
    rig.perform(DeckAction::WanderPace {
        deck: "A".into(),
        bars: 8,
    });
    rig.started(4);
    assert_eq!(
        rig.st.e.auto_mix.handoff_bar,
        Some(8),
        "Mix now: the first phrase boundary >= 4 bars on"
    );
    assert_eq!(rig.st.e.auto_mix.phase, AutoMixPhase::Playing);
    // the prep point (bar 4): the next track is composed onto B, stopped, mix-in baked in
    assert!(rig.run_until(|r| r.run_in_flight()));
    assert_eq!(
        rig.tick("A") / 16,
        4,
        "prepared at P = H - 8 ... here H - 4 (Mix now)"
    );
    let run = rig.st.e.auto_mix.run.clone().unwrap();
    assert!(rig.st.e.auto_mix.handoffs == 1 && run.plan.n == 0);
    assert!(["seed", "instrument", "genre"].contains(&run.plan.kind.as_str()));
    {
        let b = rig.st.e.deck("B").unwrap();
        assert!(b.is_net() && b.spec == run.plan.spec && !b.playing);
        let l = b.track.latest.as_ref().unwrap();
        assert_eq!(l.song_rests.mask(0), MixOutTail::REST_MASK);
        assert!(l.song_rests.mask(4 * 16).is_empty());
    }
    assert!(rig.st.e.deck("A").unwrap().walk.held && rig.st.e.deck("B").unwrap().walk.held);
    let wander_step = rig.st.e.deck("A").unwrap().walk.wander.step;
    // the end: B is playing, A stopped on its tail's bar line and stays stopped
    assert!(rig.run_until(|r| !r.run_in_flight()));
    assert_eq!(rig.st.e.auto_mix.log.len(), 1);
    let row = rig.st.e.auto_mix.log[0].clone();
    assert!(row.from == "A" && row.to == "B" && row.bar == 8 && row.n == 0);
    assert!(rig.st.e.deck("B").unwrap().playing && !rig.st.e.deck("A").unwrap().playing);
    assert_eq!(rig.st.e.auto_mix.playing.as_deref(), Some("B"));
    assert_eq!(rig.st.e.crossfader, 1.0);
    assert_eq!(
        rig.st.e.deck("A").unwrap().walk.wander.step,
        wander_step,
        "Wander held from P to the end"
    );
    assert!(!rig.st.e.deck("A").unwrap().walk.held && !rig.st.e.deck("B").unwrap().walk.held);
    assert_eq!(
        rig.st.e.deck("B").unwrap().leader,
        None,
        "B follows no one after the hand-off"
    );
    assert_eq!(
        rig.st
            .e
            .deck("A")
            .unwrap()
            .track
            .latest
            .as_ref()
            .unwrap()
            .tail,
        Some(MixOutTail::new(8 * 16, 4))
    );
    rig.run_blocks(600);
    let a = rig.st.e.deck("A").unwrap();
    assert!(
        !a.playing && a.track.advances == 0,
        "auto-next did not restart A"
    );

    // B's first tick is A's hand-off bar line (step 128), to the sample
    let b_start = rig.frame("B", 0).unwrap();
    let a_h = rig.frame("A", 8 * 16).unwrap();
    assert!(
        (b_start - a_h).abs() <= 1,
        "B starts on A's bar 8: {b_start} vs {a_h}"
    );
    // and stays on it: every bar line of the hand-off
    for k in 1..4 {
        let bk = rig.frame("B", k * 16).unwrap();
        let ak = rig.frame("A", (8 + k) * 16).unwrap();
        assert!((bk - ak).abs() <= 1, "bar {k}: {bk} vs {ak}");
    }
    // A stopped on its bar 12: its last step is the one before
    let a_last = rig
        .st
        .r
        .deck("A")
        .unwrap()
        .runner
        .anchors
        .iter()
        .rev()
        .find(|a| !a.flags.contains(TickAnchorFlags::SWAP))
        .map(|a| a.tick as i64);
    assert_eq!(
        a_last,
        Some(12 * 16),
        "A's last step is the one before bar 12"
    );

    // the crossfader: c0 until the hand-off, then one step per bar, linear in position
    for f in rig.faders.iter().filter(|f| f.3 > 0 && f.3 <= 4 * 16 + 1) {
        let k = 4.min((f.3 - 1) / 16);
        assert!(
            (f.1 - (0.5 + 0.5 * k as f64 / 4.0)).abs() < 1e-12,
            "bar {k}: {}",
            f.1
        );
    }
    assert!(
        rig.faders.iter().filter(|f| f.3 == 0).all(|f| f.1 == 0.5),
        "nothing moves before the hand-off"
    );
    let changes: Vec<_> = rig
        .faders
        .windows(2)
        .filter(|w| w[0].1 != w[1].1)
        .map(|w| w[1])
        .collect();
    assert_eq!(changes.len(), 4, "four steps: {changes:?}");
    for f in changes {
        assert_eq!((f.3 - 1) % 16, 0, "on a bar line: B tick {}", f.3);
    }
}

// AutoMixTests.aLateArmIsCorrectedBeforeItIsHeard
#[test]
fn a_late_arm_is_corrected_before_it_is_heard() {
    let mut rig = Rig::dnb();
    rig.e().auto_mix_host.arm_delay = 1; // the start commands land a housekeeping tick (512 frames) late
    rig.started(4);
    assert!(rig.run_until(|r| r.st.e.auto_mix.log.len() == 1));
    let b_start = rig.frame("B", 0).unwrap();
    let a_h = rig.frame("A", 8 * 16).unwrap();
    assert!(
        b_start - a_h > 100,
        "the start was late: {} frames",
        b_start - a_h
    );
    // corrected in the hand-off's first bar: from bar 1 on, to the sample
    for k in 1..4 {
        let bk = rig.frame("B", k * 16).unwrap();
        let ak = rig.frame("A", (8 + k) * 16).unwrap();
        assert!((bk - ak).abs() <= 1, "bar {k}: {bk} vs {ak}");
    }
}

// AutoMixTests.steeringPausesExpressionDoesNot
#[test]
fn steering_pauses_expression_does_not() {
    use djengine::fx::StripKnob;
    let mut rig = Rig::dnb();
    rig.auto_mix(true, 4, 4);
    rig.play("A", true);
    rig.run_blocks(8);
    assert_eq!(rig.st.e.auto_mix.phase, AutoMixPhase::Playing);
    assert_eq!(rig.st.e.auto_mix.handoff_bar, Some(32));
    // expression: knobs, macros, mutes
    rig.e().set_strip("A", StripKnob::EqLow, 0.2);
    rig.e()
        .fire_master("A", djengine::fx::MasterMacro::SweepLp, None);
    rig.e().set_role_muted("A", "bass", true);
    rig.run_blocks(8);
    assert!(!rig.st.e.auto_mix.paused);
    // steering: a hot cue on A
    rig.hot_cue("A", 0);
    rig.run_blocks(8);
    assert!(rig.st.e.auto_mix.paused && rig.st.e.auto_mix.phase == AutoMixPhase::Paused);
    assert_eq!(rig.st.e.auto_mix.handoff_bar, None);
    let paused_at = rig.tick("A") / 16;
    assert!(rig.run_until(|r| !r.st.e.auto_mix.paused));
    assert_eq!(
        rig.tick("A") / 16,
        paused_at + AutoMix::QUIET_BARS,
        "a phrase of quiet bars"
    );
    // it counts again from where it resumed
    assert_eq!(rig.st.e.auto_mix.handoff_bar, Some(40));
}

// AutoMixTests.aCueIsUsedAsIs
#[test]
fn a_cue_is_used_as_is() {
    let mut rig = Rig::dnb();
    let cue = TrackSpec::new("dnb", 777.0, None);
    rig.auto_mix(true, 4, 0);
    rig.play("A", true);
    rig.run_blocks(8);
    rig.load_net("B", &cue);
    assert!(
        !rig.st.e.auto_mix.paused,
        "a load on the idle deck is a cue, not a pause"
    );
    rig.mix_now();
    assert!(rig.run_until(|r| r.run_in_flight()));
    assert_eq!(rig.st.e.auto_mix.run.as_ref().unwrap().plan.kind, "cued");
    assert_eq!(rig.st.e.deck("B").unwrap().spec.as_ref(), Some(&cue));
    assert!(rig.run_until(|r| !r.run_in_flight()));
    assert_eq!(rig.st.e.auto_mix.log[0].kind, "cued");
    assert_eq!(rig.st.e.deck("B").unwrap().spec.as_ref(), Some(&cue));
    assert!(rig.st.e.deck("B").unwrap().playing && !rig.st.e.deck("A").unwrap().playing);
}

// AutoMixTests.theCrossfaderTouchTakesTheFader
#[test]
fn the_crossfader_touch_takes_the_fader() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(mixing));
    rig.crossfader(0.3);
    assert!(rig.run_until(|r| !r.run_in_flight()));
    assert_eq!(
        rig.st.e.crossfader, 0.3,
        "the user's crossfader stays where they left it"
    );
    assert_eq!(
        rig.st.e.auto_mix.log.len(),
        1,
        "the rest of the hand-off completed"
    );
    assert!(rig.st.e.deck("B").unwrap().playing && !rig.st.e.deck("A").unwrap().playing);
}

// AutoMixTests.playStopAborts
#[test]
fn play_stop_aborts() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(mixing));
    let x = rig.st.e.crossfader;
    rig.play("B", false);
    rig.run_blocks(8);
    assert!(rig.st.e.auto_mix.run.is_none() && rig.st.e.auto_mix.log.is_empty());
    assert!(rig.st.e.auto_mix.paused);
    assert_eq!(rig.st.e.crossfader, x, "nothing undone, nothing moved");
    assert!(!rig.st.e.deck("A").unwrap().walk.held && !rig.st.e.deck("B").unwrap().walk.held);
}

// AutoMixTests.idleReasons
#[test]
fn idle_reasons() {
    let mut rig = Rig::dnb();
    rig.auto_mix(true, 8, 0);
    rig.e().service_auto_mix();
    assert_eq!(rig.st.e.auto_mix.phase, AutoMixPhase::Idle);
    assert_eq!(rig.st.e.auto_mix.idle.as_deref(), Some("start a deck"));
    // both playing: the louder is "playing", it waits for the other
    rig.e().set_playing("A", true);
    rig.e().set_playing("B", true);
    rig.e().set_crossfader(0.2);
    rig.e().service_auto_mix();
    assert_eq!(rig.st.e.auto_mix.playing.as_deref(), Some("A"));
    assert_eq!(rig.st.e.auto_mix.idle.as_deref(), Some("B is playing too"));
    rig.e().set_playing("B", false);
    // a sample the user loaded on the idle deck is never started
    let silence = SampleBuffer::new(vec![0.0; 4410], None, RATE);
    assert!(rig.e().load_sample("B", PreparedSample::new(silence, "x")));
    rig.e().service_auto_mix();
    assert!(rig
        .st
        .e
        .auto_mix
        .idle
        .as_deref()
        .is_some_and(|s| s.starts_with("B holds a sample")));
    // the demo loop (or nothing) is not the user's: replaced by the next track
    assert!(rig.e().load_demo("B"));
    rig.e().service_auto_mix();
    assert!(rig.st.e.auto_mix.idle.is_none() && rig.st.e.auto_mix.phase == AutoMixPhase::Playing);
    // every new-track move locked
    for r in ["kick", "snare", "hihat", "bass", "melody", "harmony", "arp"] {
        rig.e().set_wander_lock("A", &WanderLock::keep(r), true);
    }
    rig.e().service_auto_mix();
    assert!(rig
        .st
        .e
        .auto_mix
        .idle
        .as_deref()
        .is_some_and(|s| s.starts_with("every move is locked")));
    // a sample deck playing: wait for a cue
    rig.e().set_playing("A", false);
    rig.e().set_playing("B", true);
    rig.e().service_auto_mix();
    assert!(rig
        .st
        .e
        .auto_mix
        .idle
        .as_deref()
        .is_some_and(|s| s.starts_with("B plays a sample")));
    rig.auto_mix(false, 8, 0);
    assert_eq!(rig.st.e.auto_mix.phase, AutoMixPhase::Off);
    assert_eq!(rig.st.e.auto_mix_status(), "auto-mix off");
}

// AutoMixTests.theMixOutAheadIsTheHandMixOut
#[test]
fn the_mix_out_ahead_is_the_hand_mix_out() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    // what auto-mix prepares ahead at P (live), from A's state at P ...
    let (tick, tail) = rig.st.e.mix_out_point("A", 8, 4).unwrap();
    let ahead = rig
        .st
        .e
        .move_request("A", tick, Some(tail), None)
        .unwrap()
        .prepare();
    // ... is what the hand mixOut in bar H - 1 (which the replay runs) loads
    assert!(rig.run_until(|r| r
        .st
        .e
        .auto_mix
        .run
        .as_ref()
        .is_some_and(|r| r.mix_out_loaded)));
    let hand = rig.st.e.deck("A").unwrap().track.latest.clone().unwrap();
    let w = ahead.wave();
    assert!(w.start_tick == hand.start_tick && w.tail == hand.tail);
    assert!(w.rests == hand.rests && w.song_rests == hand.song_rests);
    assert!(w.gain == hand.gain && w.track_serial == hand.track_serial);
    assert!(w.energy == hand.energy && w.duck_depth == hand.duck_depth);
}

// AutoMixTests.aCheckpointMidHandOffCarriesIt (the engine half: the run goes through the
// checkpoint's text and a fresh engine takes the hand-off up where it was)
#[test]
fn a_checkpoint_mid_hand_off_carries_it() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(mixing));
    let state = rig.st.e.auto_mix.clone();
    let text = state.checkpoint_json();
    assert_eq!(
        text.get("run")
            .and_then(|r| r.get("in"))
            .and_then(|v| v.as_str()),
        Some("B")
    );
    let back = AutoMixState::from_json(Some(&text));
    assert_eq!(back.run, state.run);
    // stood in a fresh engine, it takes the hand-off up where it was
    let mut fresh = Stepper::new(RATE, true);
    fresh.e.restore_auto_mix(&back);
    assert_eq!(fresh.e.auto_mix.run, state.run);
    assert!(fresh.e.deck("A").unwrap().walk.held && fresh.e.deck("B").unwrap().walk.held);
    assert_eq!(fresh.e.deck("B").unwrap().leader.as_deref(), Some("A"));
    assert!(fresh.e.deck("A").unwrap().track.auto_mixing_out);
    assert_eq!(fresh.e.auto_mix.phase, AutoMixPhase::Preparing);
    // letting go of it releases the holds, and the incoming deck's leader goes back
    fresh.e.restore_auto_mix(&AutoMixState::default());
    assert!(fresh.e.auto_mix.run.is_none());
    assert!(!fresh.e.deck("A").unwrap().walk.held && !fresh.e.deck("B").unwrap().walk.held);
    assert_eq!(fresh.e.deck("B").unwrap().leader, None);
    assert!(!fresh.e.deck("A").unwrap().track.auto_mixing_out);
}

// AutoMixTests.aDeferIsRecordedAndReplayed (the live half): the first compose never lands, the
// hand-off moves a phrase on, the second compose (asked for at the defer) lands while the set
// waits, and the defer is recorded (`recorder?.record(.autoMixDefer)`: by the engine, after
// the calls that were made: here a session records [autoMix, play, autoMixNow, autoMixDefer])
#[test]
fn a_late_compose_defers_the_hand_off_a_phrase() {
    let mut rig = Rig::dnb();
    rig.e().deterministic = false; // live: composes on a worker thread
    rig.e().auto_mix_host.compose_stalls = true; // the first compose never lands
    rig.e().start_session().unwrap();
    rig.auto_mix(true, 4, 0);
    rig.play("A", true);
    rig.run_blocks(4);
    rig.mix_now();
    // to the mix-out point: the incoming is not there, the hand-off moves a phrase on
    assert!(rig.run_until(|r| r
        .st
        .e
        .auto_mix
        .run
        .as_ref()
        .is_some_and(|r| r.handoff_bar == 16)));
    assert_eq!(rig.st.e.auto_mix_host.recorded_defers, vec![0]);
    // the second compose lands while the set waits
    rig.e().wait_auto_mix();
    assert_eq!(
        rig.st.e.auto_mix.run.as_ref().map(|r| r.incoming_ready),
        Some(true)
    );
    // (the mix-out prepared for the new bar line lands too)
    while !rig.st.e.auto_mix_host.ahead.is_finished() {
        std::thread::yield_now();
    }
    assert!(rig.run_until(|r| !r.st.e.auto_mix.log.is_empty()));
    assert_eq!(rig.st.e.auto_mix.log[0].bar, 16);
    assert_eq!(rig.st.e.auto_mix_host.recorded_defers, vec![0], "once");
    let session = rig.e().finish_session().unwrap();
    let actions: Vec<DeckAction> = session
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            SessionEventKind::Action(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        actions,
        [
            DeckAction::AutoMix {
                on: true,
                bars: 4,
                every: 0
            },
            DeckAction::Play {
                deck: "A".into(),
                on: true
            },
            DeckAction::AutoMixNow,
            DeckAction::AutoMixDefer { handoff: 0 },
        ]
    );
}

// SessionRecorder.start(on:): `autoMix = engine.autoMix.start`: the settings and the hand-off
// counter, nothing of where the countdown is
#[test]
fn a_session_starts_from_the_auto_mix_settings_and_counter() {
    let mut rig = Rig::dnb();
    rig.auto_mix(true, 16, 4);
    rig.e().auto_mix.handoffs = 3;
    rig.e().start_session().unwrap();
    let session = rig.e().finish_session().unwrap();
    assert_eq!(
        session.auto_mix,
        AutoMixState::with_start(AutoMixSettings::new(true, 16, 4), 3)
    );
}

// the live path end to end: the incoming composes on a worker thread, the mix-out is prepared
// a phrase ahead, and the hand-off still lands on the bar line
#[test]
fn a_live_hand_off_lands_on_its_bar_line() {
    let mut rig = Rig::dnb();
    rig.e().deterministic = false;
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    rig.e().wait_auto_mix();
    assert!(rig.st.e.auto_mix.run.as_ref().unwrap().incoming_ready);
    assert!(rig.st.e.auto_mix_host.ahead.is_requested());
    while !rig.st.e.auto_mix_host.ahead.is_finished() {
        std::thread::yield_now();
    }
    assert!(rig.run_until(|r| r.st.e.auto_mix.log.len() == 1));
    rig.run_blocks(200);
    assert!(rig.st.e.auto_mix_host.recorded_defers.is_empty());
    let b_start = rig.frame("B", 0).unwrap();
    let a_h = rig.frame("A", 8 * 16).unwrap();
    assert!(
        (b_start - a_h).abs() <= 1,
        "B starts on A's bar 8: {b_start} vs {a_h}"
    );
    assert_eq!(
        rig.st
            .e
            .deck("A")
            .unwrap()
            .track
            .latest
            .as_ref()
            .unwrap()
            .tail,
        Some(MixOutTail::new(8 * 16, 4)),
        "the mix-out prepared ahead is the one that plays"
    );
    assert!(rig.st.e.deck("B").unwrap().playing && !rig.st.e.deck("A").unwrap().playing);
}

// a poke before the mix-out point calls the start off and keeps the plan (AutoMix.md section 5)
#[test]
fn a_steering_poke_before_the_mix_out_cancels_the_start_and_keeps_the_plan() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    let plan = rig.st.e.auto_mix.run.as_ref().unwrap().plan.clone();
    let b_spec = rig.st.e.deck("B").unwrap().spec.clone();
    rig.hot_cue("A", 0);
    assert!(rig.st.e.auto_mix.run.is_none());
    assert!(rig.st.e.auto_mix.paused);
    assert_eq!(
        rig.st.e.auto_mix.kept.as_ref(),
        Some(&plan),
        "kept for later"
    );
    assert!(!rig.st.e.deck("A").unwrap().walk.held && !rig.st.e.deck("B").unwrap().walk.held);
    assert_eq!(
        rig.st.e.deck("B").unwrap().spec,
        b_spec,
        "the incoming deck keeps what was prepared, stopped"
    );
    assert!(!rig.st.e.deck("B").unwrap().playing);
    // a load on the incoming deck is a cue instead: the plan is not kept
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    rig.load_net("B", &TrackSpec::new("house", 5.0, None));
    assert!(rig.st.e.auto_mix.run.is_none() && rig.st.e.auto_mix.kept.is_none());
    assert_eq!(rig.st.e.auto_mix.cued.as_deref(), Some("B"));
    assert!(rig.st.e.auto_mix.paused);
}

// a kept plan is reused when the outgoing configuration did not change (beginRun's `reuse`),
// and the counter does not move for it
#[test]
fn a_kept_plan_is_reused_at_the_next_prep_point() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    let plan = rig.st.e.auto_mix.run.as_ref().unwrap().plan.clone();
    assert_eq!(rig.st.e.auto_mix.handoffs, 1);
    rig.hot_cue("A", 0);
    // resume after a quiet phrase, then Mix now again: the same plan, the incoming still there
    assert!(rig.run_until(|r| !r.st.e.auto_mix.paused));
    rig.mix_now();
    assert!(rig.run_until(|r| r.run_in_flight()));
    let run = rig.st.e.auto_mix.run.clone().unwrap();
    assert_eq!(run.plan, plan, "the kept plan");
    assert!(run.incoming_ready, "nothing composed again");
    assert_eq!(rig.st.e.auto_mix.handoffs, 1, "the counter did not move");
}

// the incoming deck leaves out what the outgoing deck leaves out (beginRun's role copy); the
// roles the idle deck had disabled and the outgoing deck has not are enabled again
#[test]
fn the_incoming_deck_leaves_out_what_the_outgoing_deck_leaves_out() {
    let mut rig = Rig::dnb();
    rig.e().deck_mut("A").unwrap().disabled_roles = vec!["arp".into(), "harmony".into()];
    rig.e().deck_mut("B").unwrap().disabled_roles = vec!["bass".into()];
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    let mut got = rig.st.e.deck("B").unwrap().disabled_roles.clone();
    got.sort();
    assert_eq!(got, vec!["arp".to_string(), "harmony".to_string()]);
}

// beginRun's `inc.setRoleDisabled`: the roles the incoming deck leaves out are in its options'
// role filter, the one every compose on the deck takes (so the incoming track's too and the
// key of its look-ahead), as the outgoing deck's are: the copy goes through the deck's own
// `set_role_disabled`, not a second path that only writes `disabled_roles`
#[test]
fn the_incoming_decks_role_filter_follows_the_roles_it_leaves_out() {
    let mut rig = Rig::dnb();
    for (deck, role) in [("A", "arp"), ("A", "harmony"), ("B", "bass")] {
        rig.e().set_role_disabled(deck, role, true);
    }
    let mute = |rig: &Rig, deck: &str| {
        let mut m = rig.st.e.role_filter(deck).map(|f| f.mute).unwrap_or_default();
        m.sort();
        m
    };
    assert_eq!(mute(&rig, "B"), vec!["bass".to_string()]);
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    let both = vec!["arp".to_string(), "harmony".to_string()];
    assert_eq!(mute(&rig, "A"), both);
    assert_eq!(mute(&rig, "B"), both, "bass is enabled again, arp and harmony are left out");
    assert!(rig.st.e.is_role_disabled("B", "arp") && !rig.st.e.is_role_disabled("B", "bass"));
}

// the tempo: B syncs to A for the hand-off and follows it, then lets go
#[test]
fn the_incoming_deck_syncs_to_the_outgoing_one_and_lets_go() {
    let mut rig = Rig::dnb();
    rig.started(4);
    assert!(rig.run_until(|r| r.st.e.auto_mix.run.as_ref().is_some_and(|r| r.armed)));
    let (a_bpm, b_bpm) = (
        rig.st.e.effective_bpm("A").unwrap(),
        rig.st.e.effective_bpm("B").unwrap(),
    );
    // dnb 42 -> a new dnb-ish track at the same tempo window: within the fader's range of A's
    assert!(
        (b_bpm / a_bpm - 1.0).abs() < 0.081
            || (b_bpm / a_bpm - 0.5).abs() < 0.05
            || (b_bpm / a_bpm - 2.0).abs() < 0.1,
        "{b_bpm} vs {a_bpm}"
    );
    assert_eq!(rig.st.e.deck("B").unwrap().leader.as_deref(), Some("A"));
    assert!(rig.run_until(|r| !r.run_in_flight()));
    assert_eq!(rig.st.e.deck("B").unwrap().leader, None);
}

// AutoMixRealTimeTests.handOffLevelsStayWithinTheMeasuredDip (Swift: release builds only, a
// whole song rendered through AVAudioEngine): the level through a hand-off stays inside the
// measured dip (`AutoMix::MAX_DIP_LU`), never bumps more than the crossfade tolerance over the
// louder track, and never clips. `AutoMixMeasure.run` is `djhost::automix_measure`. techno 42
// (about 10 s) and lofi 7, the second case of the Swift test (about 4 s), both run at the test
// profile's opt-level.
fn levels_stay_within_the_dip(genre: &str, seed: f64, runner_gap_lu: f64) {
    use djengine::deck::level_match::LevelMatch;
    use djhost::automix_measure::AutoMixMeasure;
    let m = AutoMixMeasure::run(
        genre,
        seed,
        Some("standard"),
        8,
        4,
        1,
        false,
        48_000.0,
        false,
    )
    .unwrap();
    let h = m.handoffs.first().expect("a hand-off");
    assert!(
        h.dip_lu() >= -AutoMix::MAX_DIP_LU - runner_gap_lu,
        "{genre} {seed}: dip {}",
        h.dip_lu()
    );
    assert!(
        h.bump_lu() <= LevelMatch::CROSSFADE_TOLERANCE_LU,
        "{genre} {seed}: bump {}",
        h.bump_lu()
    );
    assert!(
        m.true_peak_dbtp < 0.0,
        "{genre} {seed}: true peak {}",
        m.true_peak_dbtp
    );
    eprintln!(
        "{genre} {seed}: dip {:.2} LU, bump {:.2} LU, true peak {:.2} dBTP",
        h.dip_lu(),
        h.bump_lu(),
        m.true_peak_dbtp
    );
}

#[test]
fn hand_off_levels_stay_within_the_measured_dip_techno() {
    levels_stay_within_the_dip("techno", 42.0, 0.0);
}

// lofi 7 is the case `AutoMix::MAX_DIP_SOURCE` names (its drums and bass alone are near silent,
// -39 LUFS short-term; Swift measured -23.64 LU, reproduced on 2026-11-04 with the Swift
// `djctl automix lofi 7 --bars 8 --every 4` built at the pin). Here it measures -23.83 LU, inside
// the limit with no allowance (the earlier measurement of -24.43 LU, which this test used to
// allow 1 LU for, was taken before `FdnReverb` took `energy_curve`'s `SideHighpass`, before
// `samplersInCurve` and before the sampled-loudness work, all 2026-10-30; the hand-off lands on
// the golden's frames and ticks). What is left is in the outgoing deck's own level before the
// hand-off: 0.8 LU louder here than Swift's (-15.27 against -16.07 LUFS; after the hand-off
// starts the per-beat curves agree to 0.15 LU), lofi-specific; the cause is not found, and it
// is not an Apple AU (the strip's EQ is bypassed at rest, the reverb is bypassed below wet 0.005).
#[test]
fn hand_off_levels_stay_within_the_measured_dip_lofi() {
    levels_stay_within_the_dip("lofi", 7.0, 0.0);
}

// `djctl automix --crossfade-only`: no mix-in and no mix-out rests, the gain law alone: the
// incoming track is not resting its melodic lanes and the outgoing deck gets no tail
#[test]
fn crossfade_only_leaves_out_the_rests_and_the_tail() {
    let mut rig = Rig::dnb();
    rig.e().auto_mix_host.crossfade_only = true;
    rig.started(4);
    assert!(rig.run_until(|r| r.run_in_flight()));
    let l = rig.st.e.deck("B").unwrap().track.latest.clone().unwrap();
    assert!(l.song_rests.mask(0).is_empty(), "no mix-in rest");
    assert!(rig.run_until(|r| !r.run_in_flight()));
    let a = rig.st.e.deck("A").unwrap().track.latest.clone().unwrap();
    assert_eq!(a.tail, None, "no mix-out tail");
    assert_eq!(rig.st.e.auto_mix.log.len(), 1);
}

// Deck.wanderContext / wanderVoices / wanderKey / setWanderLock, as the draw reads them
// (wanderdeck's wander.rs; the Swift source `Wander.swift:766-873`)
#[test]
fn the_draw_reads_the_outgoing_decks_wander_context() {
    use djengine::wander::Wander;
    let mut rig = Rig::dnb();
    let c = rig.st.e.wander_context("A").expect("a composed net");
    assert_eq!(c.deck, "A");
    assert_eq!(c.spec, TrackSpec::new("dnb", 42.0, None));
    assert!(c.is_loop, "no structure: a loop");
    assert_eq!(c.pace, 16, "the default pace");
    assert!(c.roles.iter().any(|r| r == "kick") && c.roles.iter().any(|r| r == "bass"));
    assert!(
        c.roles_on.len() == c.roles.len(),
        "a loaded deck plays all its roles"
    );
    // the voices: one per channel, the roles sharing a channel together, the instrument sets
    // of the genre first
    assert!(!c.voices.is_empty());
    let mut channels: Vec<i64> = c.voices.iter().map(|v| v.channel).collect();
    channels.sort_unstable();
    channels.dedup();
    assert_eq!(channels.len(), c.voices.len());
    // (dnb puts each role on a channel of its own; roles that share one would share a voice)
    assert_eq!(c.voices.len(), c.roles.len());
    let kick = c.voices.iter().find(|v| v.roles == ["kick"]).unwrap();
    assert_eq!((kick.channel, kick.instrument.as_str()), (10, "drums"));
    let bass = c.voices.iter().find(|v| v.roles == ["bass"]).unwrap();
    assert_eq!(bass.instrument, "acid");
    assert!(
        bass.options.contains(&bass.instrument),
        "{:?}",
        bass.options
    );
    // the key the track plays in: the genre's scale and the played project's root (the
    // variety pass moves it: -1 for this seed)
    let key = rig.st.e.wander_key("A").unwrap();
    assert_eq!(key.scale, "MinorScale");
    assert_eq!(key.offset, -1);
    assert_eq!(
        key,
        Wander::played_key(
            "dnb",
            rig.st
                .e
                .deck("A")
                .unwrap()
                .project
                .as_ref()
                .unwrap()
                .root_note
        )
        .unwrap()
    );
    // a deck with nothing on it, or a sample, has none
    assert!(rig.st.e.wander_context("B").is_none());
    assert!(rig.st.e.wander_voices("B").is_empty());
    // the locked actions ride along
    rig.e().locked.push("A.wander".into());
    assert!(rig
        .st
        .e
        .wander_context("A")
        .unwrap()
        .locked_actions
        .contains("A.wander"));
}

#[test]
fn a_tempo_or_key_lock_holds_what_the_deck_plays_now() {
    use djengine::wander::Wander;
    let mut rig = Rig::dnb();
    rig.e().set_wander_lock("A", "tempo", true);
    rig.e().set_wander_lock("A", "key", true);
    rig.e().set_wander_lock("A", "bogus", true); // not a dimension: ignored
    let w = rig.st.e.deck("A").unwrap().walk.wander.clone();
    assert!(w.locks.contains("tempo") && w.locks.contains("key") && !w.locks.contains("bogus"));
    assert_eq!(
        w.anchor_bpm,
        Some(Wander::bpm(&TrackSpec::new("dnb", 42.0, None)))
    );
    assert_eq!(w.key, rig.st.e.wander_key("A"));
    let c = rig.st.e.wander_context("A").unwrap();
    assert!(c.locked("tempo") && c.locked("key"));
    assert_eq!((c.anchor_bpm, c.key.clone()), (w.anchor_bpm, w.key.clone()));
    rig.e().set_wander_lock("A", "tempo", false);
    rig.e().set_wander_lock("A", "key", false);
    let w = rig.st.e.deck("A").unwrap().walk.wander.clone();
    assert!(w.locks.is_empty() && w.anchor_bpm.is_none() && w.key.is_none());
}

// MixOutAhead (AutoMix.swift:404-437): one slot per engine; a request replaces what it held,
// `take` hands the prepared project over once, `drop` lets go of it (a prepare still running
// is not waited for and its result is not kept)
#[test]
fn the_mix_out_ahead_slot_hands_over_once() {
    use djengine::wave::rests::MixOutTail;
    use djhost::deck::automix::MixOutAhead;
    let mut rig = Rig::dnb();
    rig.play("A", true);
    rig.run_blocks(8);
    let (tick, tail) = rig.st.e.mix_out_point("A", 8, 4).unwrap();
    assert_eq!(tail, MixOutTail::new(8 * 16, 4));
    let req = |e: &djhost::deck::Engine| e.move_request("A", tick, Some(tail), None).unwrap();
    let mut slot = MixOutAhead::default();
    assert!(slot.is_finished() && !slot.is_requested() && slot.take().is_none());
    slot.request(req(&rig.st.e));
    assert!(slot.is_requested());
    while !slot.is_finished() {
        std::thread::yield_now();
    }
    let p = slot.take().expect("the prepared project");
    assert!(p.wave().start_tick == tick && p.wave().tail == Some(tail));
    assert!(slot.take().is_none(), "handed over once");
    // dropped: nothing to take, even once the prepare has finished
    slot.request(req(&rig.st.e));
    slot.drop_slot();
    assert!(!slot.is_requested() && slot.take().is_none());
    // a second request replaces the first
    slot.request(req(&rig.st.e));
    slot.request(req(&rig.st.e));
    while !slot.is_finished() {
        std::thread::yield_now();
    }
    assert!(slot.take().is_some() && slot.take().is_none());
}
