//! F3 tempo curves (docs/beatscript/ext-tempo.md): `tempo 100 -> 130 over 8 bars exp`, a `tempo` in a section, `at bar 16: tempo 128
//! -> 140 over 4 bars`. The canonical forms, the exact tempo map (closed-form seconds, the inverse, the sum of the bars), the
//! errors, what `check` says of the length, the nets made at the tempo the curve has where they begin, and the runtime on a fake
//! clock: the tempo set each pass from the program's position, nothing sent for a tempo that is one number, a host that cannot
//! say so once. And the exactness law: a tempo curve never moves a bar or a timing word.

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check, Compiled, Effect, Timing};
use djlive::rat::Rat;
use djlive::runtime::{Grid, Live, LiveHost};
use djlive::shape::Shape;
use djlive::tempo::{TempoEvent, TempoMap};
use djlive::Span;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

/// (line, column, message, help) of every error.
fn errors(src: &str) -> Vec<(usize, usize, String, String)> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone().unwrap_or_default())).collect()
}

fn r(n: i64, d: i64) -> Rat {
    Rat::new(n, d).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---- canonical forms

#[test]
fn tempo_curves_print_the_way_the_contract_says() {
    for (src, want) in [
        // a plain tempo prints as it always did
        ("tempo 126\n", "tempo 126\n"),
        ("tempo 60 + 6\n", "tempo 66\n"),
        // a curve at the top, with and without a shape
        ("tempo 100 -> 130 over 8 bars\n", "tempo 100 -> 130 over 8 bars\n"),
        ("tempo 100 -> 130 over 8 bars linear\n", "tempo 100 -> 130 over 8 bars\n"),
        ("tempo 100 -> 130 over 8 bars exp\n", "tempo 100 -> 130 over 8 bars exp\n"),
        ("tempo 100 -> 130 over 8 bars log\n", "tempo 100 -> 130 over 8 bars log\n"),
        ("tempo 100 -> 130 over 8 bars s-curve\n", "tempo 100 -> 130 over 8 bars s-curve\n"),
        ("tempo 100 -> 130 over 8 bars step 4\n", "tempo 100 -> 130 over 8 bars step 4\n"),
        ("tempo 100 -> 130 over 1/2 bar\n", "tempo 100 -> 130 over 2 beats\n"),
        ("tempo 100 -> 130 over 6 beats\n", "tempo 100 -> 130 over 6 beats\n"),
        ("tempo 100 -> 120 + 10 over 2 * 4 bars\n", "tempo 100 -> 130 over 8 bars\n"),
        // timed lines
        ("at bar 16: tempo 128 -> 140 over 4 bars\ndeck A: kick x\n", "at bar 16: tempo 128 -> 140 over 4 bars\ndeck A: kick x\n"),
        ("at bar 16: tempo 128\ndeck A: kick x\n", "at bar 16: tempo 128\ndeck A: kick x\n"),
        ("at bar 16 beat 2: tempo 128 -> 140 over 2 beats exp\n", "at bar 16 beat 2: tempo 128 -> 140 over 2 beats exp\n"),
        ("at bar 16 beat 0: tempo 128\n", "at bar 16: tempo 128\n"),
        // in a section, a def and a repeat
        (
            "section verse 8 bars {\n  tempo 120\n  deck A: kick x\n}\nsection chorus 8 bars {\n  tempo 140 -> 150 over 2 bars s-curve\n  deck A: kick x x\n}\narrange verse chorus\n",
            "section verse 8 bars {\n  tempo 120\n  deck A: kick x\n}\nsection chorus 8 bars {\n  tempo 140 -> 150 over 2 bars s-curve\n  deck A: kick x x\n}\narrange verse chorus\n",
        ),
        ("def build(a) {\n  tempo $a -> 140 over 4 bars\n}\nat bar 4: build(100)\n", "def build(a) {\n  tempo $a -> 140 over 4 bars\n}\nat bar 4: build(100)\n"),
    ] {
        let c = ok(src);
        assert_eq!(c.text, want, "{src}");
        // and the canonical text is a fixed point
        assert_eq!(ok(&c.text).text, c.text, "{src}");
    }
}

#[test]
fn two_spellings_of_one_tempo_are_one_program_and_another_tempo_is_another() {
    for (a, b) in [
        ("tempo 100 -> 130 over 8 bars", "tempo 100 -> 130 over 8 bars linear"),
        ("tempo 100 -> 130 over 8 bars", "tempo 100 -> 130 over 32 beats"),
        ("at bar 8: tempo 128", "at bar 8 beat 0: tempo 128"),
        ("at bar 3 beat 2: tempo 128", "at bar 7/2: tempo 128"),
    ] {
        assert_eq!(ok(&format!("{a}\n")).cid, ok(&format!("{b}\n")).cid, "{a} / {b}");
    }
    let base = ok("tempo 100 -> 130 over 8 bars exp\n").cid;
    for other in ["tempo 100 -> 130 over 8 bars log", "tempo 100 -> 130 over 8 bars", "tempo 100 -> 131 over 8 bars exp", "tempo 101 -> 130 over 8 bars exp", "tempo 100 -> 130 over 9 bars exp", "tempo 100", "tempo 130"] {
        assert_ne!(ok(&format!("{other}\n")).cid, base, "{other}");
    }
    assert_ne!(ok("at bar 8: tempo 128\n").cid, ok("at bar 9: tempo 128\n").cid);
    assert_ne!(ok("section verse 4 bars {\n  tempo 120\n  deck A: kick x\n}\narrange verse\n").cid, ok("section verse 4 bars {\n  tempo 121\n  deck A: kick x\n}\narrange verse\n").cid);
}

// ---- the map a program has

#[test]
fn the_tempo_map_of_a_program_is_exact() {
    // a ramp at the top: 100 -> 130 over 8 bars from the program's first bar, then 130
    let c = ok("tempo 100 -> 130 over 8 bars\ndeck A: kick x\n");
    let m = c.tempo_map();
    assert_eq!(c.tempo, Some(100.0));
    assert_eq!(m.events().len(), 1);
    assert!(close(m.seconds_at(8u32), 60.0 * 32.0 * (1.3f64).ln() / 30.0));
    assert!(close(m.seconds_at(16u32), m.seconds_at(8u32) + 8.0 * 240.0 / 130.0));
    assert_eq!((m.declared_at(0u32), m.declared_at(8u32)), (Some(100.0), Some(130.0)));
    assert!(m.drives());
    // a plain tempo is a constant map: not driven
    let c = ok("tempo 126\ndeck A: kick x\n");
    assert!(c.tempo_map().is_constant() && !c.tempo_map().drives());
    assert_eq!(c.tempo_map().constant_bpm(), Some(126.0));
    assert!(close(c.tempo_map().seconds_at(4u32), 16.0 * 60.0 / 126.0));
    // no tempo at all: nothing declared
    let c = ok("deck A: kick x\n");
    assert!(c.tempo_map().is_empty() && c.tempo.is_none());
    // timed lines, in sections and after a section, are events at their bars
    let src = "tempo 100\nat bar 4 beat 2: tempo 105 -> 120 over 2 beats\nsection verse 8 bars {\n  deck A: kick x\n  at bar 2: tempo 110\n}\nsection chorus 8 bars {\n  tempo 140 -> 150 over 2 bars\n  deck A: kick x x\n}\narrange verse chorus\nbefore chorus 2 bars: tempo 130\nafter chorus: tempo 90\n";
    let c = ok(src);
    let ev: Vec<(Rat, f64, f64, Rat)> = c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to, e.over)).collect();
    assert_eq!(
        ev,
        vec![
            (r(0, 1), 100.0, 100.0, r(0, 1)),
            (r(2, 1), 110.0, 110.0, r(0, 1)),
            (r(9, 2), 105.0, 120.0, r(1, 2)),
            (r(6, 1), 130.0, 130.0, r(0, 1)),
            (r(8, 1), 140.0, 150.0, r(2, 1)),
            (r(16, 1), 90.0, 90.0, r(0, 1)),
        ]
    );
    // the tempo in a section holds until another says (a section with none keeps the one before it)
    let m = c.tempo_map();
    assert_eq!(m.declared_at(r(7, 1)), Some(130.0));
    assert_eq!(m.declared_at(r(11, 1)), Some(150.0));
    assert_eq!(m.declared_at(r(16, 1)), Some(90.0));
}

#[test]
fn a_section_and_a_def_say_where_the_tempo_begins() {
    // a def called at a bar puts its tempo there; a repeat's copies shift
    let c = ok("def build(a) {\n  tempo $a -> 140 over 4 bars\n}\nat bar 8: build(100)\nrepeat 3 every 16 bars {\n  tempo 100 + $i * 10\n}\n");
    let at: Vec<(Rat, f64)> = c.tempo_map().events().iter().map(|e| (e.at, e.from)).collect();
    assert_eq!(at, vec![(r(0, 1), 100.0), (r(8, 1), 100.0), (r(16, 1), 110.0), (r(32, 1), 120.0)]);
    // a section played twice has its tempo twice, each from its own first bar
    let c = ok("section verse 4 bars {\n  tempo 100 + 10\n  deck A: kick x\n}\nsection chorus 4 bars {\n  tempo 130\n  deck A: kick x x\n}\narrange verse chorus verse\n");
    let at: Vec<(Rat, f64)> = c.tempo_map().events().iter().map(|e| (e.at, e.from)).collect();
    assert_eq!(at, vec![(r(0, 1), 110.0), (r(4, 1), 130.0), (r(8, 1), 110.0)]);
    // `after` and `before` are places
    let c = ok("section verse 4 bars {\n  deck A: kick x\n  before chorus 1/2 bar: tempo 100 -> 120 over 2 beats\n}\nsection chorus 4 bars {\n  deck A: kick x x\n}\narrange verse chorus\nafter verse: tempo 125\n");
    let at: Vec<(Rat, f64)> = c.tempo_map().events().iter().map(|e| (e.at, e.from)).collect();
    assert_eq!(at, vec![(r(7, 2), 100.0), (r(4, 1), 125.0)]);
}

#[test]
fn the_nets_are_made_at_the_tempo_the_curve_has_where_they_begin() {
    let src = "tempo 100\nsection verse 4 bars {\n  deck A: kick x\n}\nsection chorus 4 bars {\n  tempo 120 -> 140 over 4 bars\n  deck A: kick x x\n}\nsection outro 4 bars {\n  deck A: kick x ~ x ~\n}\narrange verse chorus outro\n";
    let c = ok(src);
    let t: Vec<Option<f64>> = c.segments.iter().map(|s| s.decks[&djlive::ast::Deck::A].tempo).collect();
    // the verse at the top tempo, the chorus where it begins its ramp, the outro where the ramp ended
    assert_eq!(t, vec![Some(100.0), Some(120.0), Some(140.0)]);
    // and a constant tempo is what it was: every net says it, no matter what the sections are
    let c = ok("tempo 126\nsection verse 4 bars {\n  deck A: kick x\n}\nsection chorus 4 bars {\n  tempo 126\n  deck A: kick x x\n}\narrange verse chorus\n");
    assert!(c.tempo_map().is_constant());
    assert!(c.segments.iter().all(|s| s.decks[&djlive::ast::Deck::A].tempo == Some(126.0)));
    // a program with a plain `tempo` keeps its CID, whatever F3 does around it (the pinned fixtures say the same at large)
    let before = ok("tempo 126\nkey am\ndeck A: kick e(5,8) | snare ~ x ~ x\n");
    assert_eq!(before.tempo, Some(126.0));
    assert_eq!(before.decks[&djlive::ast::Deck::A].tempo, Some(126.0));
}

// ---- check says the length

#[test]
fn check_says_how_long_the_song_is_through_the_curve() {
    let src = "tempo 100 -> 140 over 4 bars\nsection verse 4 bars {\n  deck A: kick x\n}\nsection chorus 4 bars {\n  deck A: kick x\n}\narrange verse chorus\nend after chorus\n";
    let c = ok(src);
    assert_eq!(c.length_bars(), Some(8));
    let want = c.tempo_map().total_seconds(8u32);
    assert!(close(want, 60.0 * 16.0 * (1.4f64).ln() / 40.0 + 4.0 * 240.0 / 140.0));
    let rep = c.length_report().unwrap();
    assert!(rep.starts_with("length: ") && rep.contains("with the tempo curve") && rep.contains("8 bars"), "{rep}");
    assert!(rep.contains(&djlive::evolve::human(want)), "{rep}");
    // a constant tempo says it
    let c = ok("tempo 120\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\n");
    assert_eq!(c.length_report().as_deref(), Some("length: 8s at 120, 4 bars"));
    // `end at bar N` is a length too; a program that plays on has none
    assert_eq!(ok("tempo 120\ndeck A: kick x\nend at bar 8\n").length_report().as_deref(), Some("length: 16s at 120, 8 bars"));
    assert_eq!(ok("tempo 120\ndeck A: kick x\n").length_report(), None);
    // no tempo: the 120 the map counts in is said
    let c = ok("section verse 4 bars {\n  deck A: kick x\n}\narrange verse\n");
    assert!(c.length_report().unwrap().contains("the program says no tempo"), "{:?}", c.length_report());
    // a tempo that begins after the song starts: counted at 120 until it does
    let c = ok("at bar 2: tempo 60\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\n");
    assert!(c.length_report().unwrap().contains("counting 120 before the first tempo"), "{:?}", c.length_report());
    // the eval answer carries it
    let live = Live::new();
    let ev = live.check("tempo 100 -> 140 over 4 bars\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\n");
    assert!(ev.ok && ev.length.as_deref().is_some_and(|l| l.contains("with the tempo curve")), "{:?}", ev.length);
}

#[test]
fn the_evolve_report_counts_its_seconds_through_the_curve() {
    let body = "section drop 16 bars {\n  deck A: kick x ~ x ~ | hihat x*8\n  evolve every 4 bars {\n    hihat: rotate 1\n    stop when gen == 3\n  }\n}\narrange drop\n";
    let plain = ok(&format!("tempo 120\n{body}"));
    let curve = ok(&format!("tempo 100 -> 140 over 16 bars\n{body}"));
    let (a, b) = (plain.evolve_report(), curve.evolve_report());
    assert_eq!(a.len(), 1);
    assert!(a[0].contains("at 120") && !a[0].contains("tempo curve"), "{a:?}");
    assert!(b[0].contains("with the tempo curve") && !b[0].contains(" at 1"), "{b:?}");
}

// ---- errors

#[test]
fn errors_say_where_and_what_would_do() {
    // a tempo of 0 or outside 40..300 names the range
    for src in ["tempo 0\n", "tempo 39\n", "tempo 301\n", "tempo 100 -> 400 over 4 bars\n", "tempo 20 -> 100 over 4 bars\n", "at bar 4: tempo 500\n", "section verse 4 bars {\n  tempo 5\n  deck A: kick x\n}\narrange verse\n"] {
        let e = errors(src);
        assert!(e.iter().any(|x| x.2.contains("a tempo is 40 to 300 BPM")), "{src:?}: {e:?}");
    }
    let e = errors("tempo 100 -> 400 over 4 bars\n");
    assert_eq!((e[0].0, e[0].1), (1, 14), "{e:?}");
    // the ramp's grammar
    for (src, want) in [
        ("tempo 100 -> 130\n", "a tempo ramp needs a length"),
        ("tempo 100 over 4 bars\n", "`over` has nothing to move"),
        ("tempo 100 exp\n", "`exp` needs a ramp"),
        ("tempo 100 -> 130 over 0 bars\n", "a tempo ramp takes some time"),
        ("tempo 100 -> 130 over 4\n", "expected `bars` or `beats`"),
        ("tempo 100 -> 130 over 4 bars expo\n", "`expo` is not a shape"),
        ("tempo 100 -> 130 over 4 bars step 1\n", "`step N` takes N from 2"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|x| x.2.contains(want)), "{src:?}: wanted `{want}`, got {e:?}");
    }
    let e = errors("tempo 100 -> 130 over 4 bars expo\n");
    assert!(e[0].3.contains("did you mean `exp`?"), "{e:?}");
    // a time word that repeats has no one place; a target and a hold make no sense
    for (src, want) in [
        ("every 4 bars: tempo 120\ndeck A: kick x\n", "happens once, at one place"),
        ("on phrase: tempo 120\n", "happens once, at one place"),
        ("every 2 beats: tempo 120\n", "happens once, at one place"),
        ("on next phrase: tempo 120\n", "happens once, at one place"),
        ("deck A: tempo 120\n", "takes no target"),
        ("master: tempo 120\n", "takes no target"),
        ("at bar 4 for 2 bars: tempo 120\n", "nothing to put back"),
        ("for 2 bars: tempo 120\n", "nothing to put back"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|x| x.2.contains(want)), "{src:?}: wanted `{want}`, got {e:?}");
    }
    // `swing` is still the top of the program's alone
    let e = errors("section verse 4 bars {\n  swing 0.1\n  deck A: kick x\n}\narrange verse\n");
    assert!(e[0].2.contains("`swing` belongs at the top"), "{e:?}");
    // a section that names the wrong place
    let e = errors("before choruss 2 bars: tempo 120\nsection chorus 4 bars {\n  deck A: kick x\n}\narrange chorus\n");
    assert!(e[0].3.contains("did you mean `chorus`?"), "{e:?}");
}

#[test]
fn a_ramp_that_the_next_tempo_cuts_short_is_a_warning() {
    let c = check("tempo 100 -> 140 over 8 bars\nat bar 4: tempo 120\n");
    assert!(c.compiled.is_some());
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("cut off at bar 4")), "{:?}", c.diagnostics);
    // two tempos at the top is a warning as it was; the last wins
    let c = check("tempo 100\ntempo 110 -> 120 over 2 bars\n");
    assert!(c.diagnostics.iter().any(|d| d.message.contains("tempo is set twice")), "{:?}", c.diagnostics);
    assert_eq!(c.compiled.unwrap().tempo_map().declared_at(0u32), Some(110.0));
}

// ---- the exactness law: bars and timing words do not move

#[test]
fn a_tempo_curve_never_moves_a_bar_or_a_timing_word() {
    let words = "at bar 3 beat 2: master: level 0.5\nevery 2 beats: master: delay 0.1\nevery 1/2 bar from 3: master: reverb 0.1\nat bar 6 for 1/2 bar: master: filter lp 800\nmaster: filter lp 300 -> 2k over 3 beats\nbefore drop 2 beats: master: level 0.4\nafter drop: master: level 0.9\n";
    let song = "section intro 4 bars {\n  deck A: kick x\n}\nsection drop 4 bars {\n  deck A: kick x x\n}\narrange intro drop\n";
    let plain = ok(&format!("tempo 120\n{song}{words}"));
    for curve in [
        "tempo 120 -> 180 over 4 bars\n",
        "tempo 90 -> 160 over 3 bars exp\n",
        "tempo 120\nat bar 2: tempo 80 -> 200 over 2 beats\nat bar 5 beat 2: tempo 60\n",
        "section extra 2 bars {\n  tempo 77\n  deck A: kick x\n}\n",
    ] {
        let src = if curve.starts_with("section") { format!("tempo 120\n{curve}{song}{words}") } else { format!("{curve}{song}{words}") };
        let with = ok(&src);
        let times = |c: &Compiled| -> Vec<Timing> { c.controls.iter().map(|k| k.timing).collect() };
        if curve.starts_with("section") {
            continue;
        }
        assert_eq!(times(&with), times(&plain), "{curve}");
        let holds = |c: &Compiled| -> Vec<_> { c.controls.iter().map(|k| k.hold).collect() };
        assert_eq!(holds(&with), holds(&plain), "{curve}");
        // the ramp's length is in bars (exact), not seconds
        for (a, b) in with.controls.iter().zip(&plain.controls) {
            if let (Effect::Ramp(x), Effect::Ramp(y)) = (&a.effect, &b.effect) {
                assert_eq!(x.bars, y.bars);
            }
        }
        assert_eq!(with.segments.iter().map(|s| (s.start, s.bars)).collect::<Vec<_>>(), plain.segments.iter().map(|s| (s.start, s.bars)).collect::<Vec<_>>());
    }
}

fn random_map(rng: &mut Rng) -> TempoMap {
    let shapes = [Shape::Linear, Shape::Exp, Shape::Log, Shape::SCurve, Shape::Step(3), Shape::Step(8)];
    let mut events = Vec::new();
    let mut at = Rat::ZERO;
    for _ in 0..(1 + rng.below(5)) {
        let from = 40.0 + rng.below(260) as f64;
        let ramp = rng.below(3) != 0;
        let to = if ramp { 40.0 + rng.below(260) as f64 } else { from };
        // positions in sixteenths of a bar, ramps from a beat to a few bars (cut short or not by the next event)
        at = at.add(r(1 + rng.below(80) as i64, 16)).unwrap();
        let over = if ramp { r(1 + rng.below(60) as i64, 16) } else { Rat::ZERO };
        let shape = shapes[rng.below(shapes.len() as u64) as usize];
        events.push(TempoEvent { at, from, to, over, shape, span: Span::default() });
    }
    TempoMap::new(events)
}

#[test]
fn the_bar_of_a_second_is_the_bar_and_the_bars_add_up_for_random_curves() {
    let mut rng = Rng(0x7e3b_0000_0001);
    for round in 0..400 {
        let m = random_map(&mut rng);
        // bars_at(seconds_at(b)) == b to 1e-9, for whole bars and for fractions
        for _ in 0..12 {
            let b = r(rng.below(64 * 16) as i64, 16);
            let s = m.seconds_at(b);
            let back = m.bar_at(s);
            assert!((back - b.to_f64()).abs() < 1e-9, "round {round}: bar {b}: {s} s -> bar {back}\n{:?}", m.events());
            assert_eq!(m.bars_at(s), back);
        }
        // and the sum of the seconds of each bar is the total
        let n = 40u32;
        let sum: f64 = (0..n).map(|k| m.seconds_between(k, k + 1)).sum();
        let total = m.total_seconds(n);
        assert!((sum - total).abs() < 1e-9 * total.max(1.0), "round {round}: {sum} vs {total}");
        // the seconds grow with the bars, never go back, and a bar is never free
        let mut prev = -1.0;
        for k in 0..n * 4 {
            let s = m.seconds_at(r(i64::from(k), 4));
            assert!(s > prev, "round {round}: bar {k}/4");
            prev = s;
        }
        // a tempo is 40 to 300, so a bar takes between 0.8 and 6 seconds
        for k in 0..n {
            let d = m.seconds_between(k, k + 1);
            assert!((0.8 - 1e-9..=6.0 + 1e-9).contains(&d), "round {round}: bar {k} takes {d} s");
        }
    }
}

// ---- the runtime on a fake clock

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat when called, deck, bpm)
    tempos: Vec<(f64, String, f64)>,
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
    /// How many times a tempo was asked for, failed or not.
    asked: usize,
    cannot: bool,
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
    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
    fn set_tempo(&mut self, deck: &str, bpm: f64) -> Result<(), String> {
        self.asked += 1;
        if self.cannot {
            return Err("this host has no tempo control".into());
        }
        self.tempos.push((self.beat, deck.into(), bpm));
        Ok(())
    }
}

impl Fake {
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    /// The tempos sent to `deck`, one a change (the once-a-bar resend left out).
    fn sent(&self, deck: &str) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (b, d, t) in &self.tempos {
            if d == deck && out.last().is_none_or(|l| (l.1 - t).abs() > 1e-9) {
                out.push((*b, *t));
            }
        }
        out
    }
}

/// A deck A that plays at beat 8: the evaluation bar is the deck's bar 2 and the program lands on its bar 3 (beat 12).
fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

/// The beat of program position `p` bars.
fn beat_at(p: f64) -> f64 {
    8.0 + 4.0 * p
}

/// The beat of program position `p` bars of a program with no sections: its bar 0 is the bar it lands on (beat 12), the first you hear,
/// as a song's own bar 0 is, and where the tempo curve starts (docs/BEATSCRIPT.md "Time words", R-02).
fn landed_at(p: f64) -> f64 {
    beat_at(1.0 + p)
}

#[test]
fn the_tempo_is_set_each_pass_from_the_programs_position() {
    for shape in ["", " exp", " log", " s-curve", " step 4"] {
        let (mut h, mut live) = playing();
        let src = format!("tempo 100 -> 130 over 8 bars{shape}\ndeck A: kick x ~ x ~\n");
        assert!(live.eval(&src).ok);
        h.run(&mut live, 4.0 * 12.0);
        let c = ok(&src);
        let map = c.tempo_map();
        let sent = h.sent("A");
        assert!(!sent.is_empty(), "{shape}");
        for (beat, bpm) in &sent {
            // (the curve starts on the bar the music lands on: before it lands, the first tempo)
            let pos = ((beat - landed_at(0.0)) / 4.0).max(0.0);
            let want = map.declared_at_f(pos).unwrap();
            assert!((bpm - want).abs() < 1e-3, "{shape}: at beat {beat} (bar {pos}) sent {bpm}, the map says {want}");
        }
        // it reaches the end of the ramp, and holds there
        assert_eq!(sent.last().unwrap().1, 130.0, "{shape}");
        assert!(sent.iter().filter(|s| s.1 == 130.0).count() == 1, "{shape}: {sent:?}");
        // it never goes back, and it is not sent again until it moves
        assert!(sent.windows(2).all(|w| w[0].1 < w[1].1), "{shape}: {sent:?}");
        // a ramp sends nothing for a deck the program has no net on
        assert!(h.sent("B").is_empty());
    }
}

#[test]
fn a_tempo_that_is_one_number_is_not_driven() {
    let (mut h, mut live) = playing();
    assert!(live.eval("tempo 126\ndeck A: kick x ~ x ~\nat bar 2: master: level 0.5\n").ok);
    h.run(&mut live, 4.0 * 6.0);
    assert_eq!(h.asked, 0, "{:?}", h.tempos);
    // the same two numbers in a section are one number too
    let (mut h, mut live) = playing();
    assert!(live.eval("tempo 126\nsection verse 2 bars {\n  tempo 126\n  deck A: kick x\n}\nsection chorus 2 bars {\n  deck A: kick x x\n}\narrange verse chorus\n").ok);
    h.run(&mut live, 4.0 * 6.0);
    assert_eq!(h.asked, 0);
}

#[test]
fn a_timed_tempo_is_sent_from_its_bar_and_nothing_before_it() {
    let (mut h, mut live) = playing();
    assert!(live.eval("deck A: kick x ~ x ~\nat bar 3: tempo 128 -> 140 over 2 bars\n").ok);
    h.run(&mut live, 4.0 * 8.0);
    let sent = h.sent("A");
    // nothing is said of the tempo before its first (the decks keep theirs), then 128, moving to 140 by bar 5
    assert!(sent.iter().all(|s| s.0 >= landed_at(3.0)), "{sent:?}");
    // (the position the runtime counts has a nanobar of slack in it, so a hair over)
    assert!((sent.first().unwrap().1 - 128.0).abs() < 1e-6, "{sent:?}");
    assert_eq!(sent.last().unwrap().1, 140.0);
    assert_eq!(sent.last().unwrap().0, landed_at(5.0));
}

#[test]
fn a_song_follows_its_sections_tempos_and_loops_back_to_the_first() {
    let src = "section verse 2 bars {\n  tempo 100\n  deck A: kick x\n  deck B: bass 1\n}\nsection chorus 2 bars {\n  tempo 140 -> 150 over 1 bars\n  deck A: kick x x\n  deck B: bass 1\n}\narrange verse chorus\nloop\n";
    let (mut h, mut live) = playing();
    h.playing[1] = true;
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 12.0);
    for deck in ["A", "B"] {
        let sent = h.sent(deck);
        let values: Vec<f64> = sent.iter().map(|s| s.1).collect();
        // the verse's 100, the chorus's ramp 140 .. 150 and its 150, then the lap begins again at 100
        let has = |from: usize, x: f64| values[from..].iter().any(|v| (v - x).abs() < 1e-6);
        assert!(has(0, 100.0) && has(0, 140.0) && has(0, 150.0), "{deck}: {values:?}");
        let first_150 = values.iter().position(|v| (*v - 150.0).abs() < 1e-6).unwrap();
        assert!(has(first_150, 100.0), "{deck}: it goes back to the verse's tempo: {values:?}");
    }
    // the song says its tempo in `get`
    let st = live.status();
    assert_eq!(st.get("tempo_curve").and_then(JsonValue::as_bool), Some(true));
    assert!(st.get("tempo").and_then(JsonValue::as_f64).is_some());
}

#[test]
fn a_host_that_cannot_set_the_tempo_is_told_once_in_the_words_and_not_asked_again() {
    let (mut h, mut live) = playing();
    h.cannot = true;
    assert!(live.eval("tempo 100 -> 130 over 8 bars\ndeck A: kick x ~ x ~\nat bar 2: master: level 0.5\n").ok);
    h.run(&mut live, 4.0 * 10.0);
    assert_eq!(h.asked, 1, "not asked again after it said no");
    let said: Vec<&String> = live.words.iter().filter(|w| w.contains("tempo curve")).collect();
    assert_eq!(said.len(), 1, "{:?}", live.words);
    assert!(said[0].contains("deck A") && said[0].contains("cannot set the tempo") && said[0].contains("timing word"), "{said:?}");
    // the rest of the program is untouched: its control line ran
    assert!(h.calls.iter().any(|c| c.1 == "mixer"));
    // a new program is asked again
    h.cannot = false;
    assert!(live.eval("tempo 90 -> 120 over 8 bars\ndeck A: kick x ~ x ~\n").ok);
    h.run(&mut live, 4.0 * 3.0);
    assert!(!h.tempos.is_empty());
}

#[test]
fn the_default_set_tempo_is_the_registrys_deck_tempo() {
    // a host that only has `call` (as a fake of the app has) gets `deck {action: tempo, deck, bpm}`
    struct OnlyCall {
        inner: Fake,
        calls: Vec<String>,
    }
    impl LiveHost for OnlyCall {
        fn grid(&self) -> Option<Grid> {
            self.inner.grid()
        }
        fn playing(&self, d: &str) -> bool {
            self.inner.playing(d)
        }
        fn load_net(&mut self, d: &str, e: &JsonValue) -> Result<(), String> {
            self.inner.load_net(d, e)
        }
        fn set_playing(&mut self, d: &str, on: bool) {
            self.inner.set_playing(d, on)
        }
        fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
            self.calls.push(format!("{noun} {}", params.canonical_js()));
            Ok(JsonValue::parse(r#"{"applied":"done"}"#).unwrap())
        }
    }
    let mut h = OnlyCall { inner: playing().0, calls: Vec::new() };
    let mut live = Live::new();
    assert!(live.eval("tempo 100 -> 130 over 4 bars\ndeck A: kick x\n").ok);
    for _ in 0..40 {
        h.inner.beat += 0.25;
        live.service(&mut h);
    }
    let decks: Vec<&String> = h.calls.iter().filter(|c| c.starts_with("deck ")).collect();
    assert!(decks.len() > 3, "{:?}", h.calls);
    assert!(decks[0].contains("\"action\":\"tempo\"") && decks[0].contains("\"deck\":\"A\"") && decks[0].contains("\"bpm\":"), "{}", decks[0]);
    // an `ignored` answer is a refusal
    struct Ignores(Fake);
    impl LiveHost for Ignores {
        fn grid(&self) -> Option<Grid> {
            self.0.grid()
        }
        fn playing(&self, d: &str) -> bool {
            self.0.playing(d)
        }
        fn load_net(&mut self, d: &str, e: &JsonValue) -> Result<(), String> {
            self.0.load_net(d, e)
        }
        fn set_playing(&mut self, d: &str, on: bool) {
            self.0.set_playing(d, on)
        }
        fn call(&mut self, _noun: &str, _params: &JsonValue) -> Result<JsonValue, String> {
            Ok(JsonValue::parse(r#"{"applied":"ignored"}"#).unwrap())
        }
    }
    let mut h = Ignores(playing().0);
    let mut live = Live::new();
    assert!(live.eval("tempo 100 -> 130 over 4 bars\ndeck A: kick x\n").ok);
    for _ in 0..8 {
        h.0.beat += 0.25;
        live.service(&mut h);
    }
    assert!(live.words.iter().any(|w| w.contains("ignored the tempo")), "{:?}", live.words);
}

#[test]
fn the_bars_a_line_fires_on_are_the_same_with_or_without_a_curve() {
    // the fake clock is in beats: a timing word fires on the same beat whatever the tempo does
    let words = "at bar 2 beat 2: master: level 0.2\nat bar 3: master: delay 0.3\nevery 1/2 bar from 4: master: reverb 0.1\nat bar 5 for 2 beats: master: filter lp 800\n";
    let fire = |src: &str| {
        let (mut h, mut live) = playing();
        assert!(live.eval(src).ok);
        h.run(&mut live, 4.0 * 8.0);
        h.calls.iter().map(|c| (c.0, c.1.clone(), c.2.clone())).collect::<Vec<_>>()
    };
    let plain = fire(&format!("tempo 120\ndeck A: kick x\n{words}"));
    for curve in ["tempo 100 -> 160 over 6 bars\n", "tempo 80 -> 200 over 2 beats exp\n", "tempo 120\nat bar 2 beat 1: tempo 60\nat bar 4: tempo 180 -> 90 over 3 bars log\n"] {
        let with = fire(&format!("{curve}deck A: kick x\n{words}"));
        assert_eq!(with, plain, "{curve}");
    }
}

// ---- properties of the text

#[test]
fn reformatting_a_program_with_tempo_curves_keeps_its_cid() {
    let programs = [
        "tempo 100 -> 130 over 8 bars exp\nkey am\ndeck A: kick x ~ x ~\n",
        "tempo 124\nat bar 16: tempo 128 -> 140 over 4 bars\nat bar 20 beat 2: tempo 130 -> 100 over 6 beats step 3\ndeck A: kick x\n",
        "section verse 8 bars {\n  tempo 120\n  deck A: kick x\n}\nsection chorus 8 bars {\n  tempo 140 -> 150 over 2 bars s-curve\n  deck A: kick x x\n  at bar 4: tempo 100 + 20\n}\narrange verse chorus\nbefore chorus 2 bars: tempo 130\n",
        "def build(lo, n) {\n  tempo $lo -> 140 over $n bars\n}\nat bar 4: build(100, 2 + 2)\nrepeat 3 every 16 bars {\n  tempo 100 + $i * 10 -> 150 over 4 bars log\n}\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let c = ok(src);
        let again = ok(&c.text);
        assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()), "program {n}");
        let mut rng = Rng(0x7e3b_1000 + n as u64);
        for round in 0..120 {
            let t = reformat(src, &mut rng);
            let got = check(&t);
            let got = got.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{t}\n{}", djlive::diag::render_all(&got.diagnostics, &t)));
            assert_eq!(got.cid, c.cid, "program {n} round {round} forked:\n{t}");
            // (the same tempos where the same bars say: the spans in the text are what a reformatting moves)
            let tempos = |c: &Compiled| -> Vec<(Rat, f64, f64, Rat, Shape)> { c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to, e.over, e.shape)).collect() };
            assert_eq!(tempos(&got), tempos(&c), "program {n} round {round}");
        }
    }
}

#[test]
fn random_tempo_programs_read_back_and_have_the_map_they_say() {
    let mut rng = Rng(0x7e3b_2000_0003);
    let shapes = ["", " exp", " log", " s-curve", " step 4", " linear"];
    for round in 0..200 {
        let mut src = String::new();
        let mut want: Vec<(Rat, f64, f64, Rat)> = Vec::new();
        let first = 40 + rng.below(260);
        src.push_str(&format!("tempo {first}\n"));
        want.push((Rat::ZERO, first as f64, first as f64, Rat::ZERO));
        let mut at = 0i64;
        for _ in 0..rng.below(5) {
            at += 1 + rng.below(8) as i64;
            let (a, b) = (40 + rng.below(260), 40 + rng.below(260));
            let over = 1 + rng.below(4);
            if rng.below(2) == 0 {
                src.push_str(&format!("at bar {at}: tempo {a} -> {b} over {over} bars{}\n", shapes[rng.below(shapes.len() as u64) as usize]));
                want.push((Rat::int(at), a as f64, b as f64, Rat::int(over as i64)));
            } else {
                src.push_str(&format!("at bar {at}: tempo {a}\n"));
                want.push((Rat::int(at), a as f64, a as f64, Rat::ZERO));
            }
        }
        src.push_str("deck A: kick x\n");
        let c = ok(&src);
        let got: Vec<(Rat, f64, f64, Rat)> = c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to, e.over)).collect();
        assert_eq!(got, want, "round {round}:\n{src}");
        assert_eq!(ok(&c.text).cid, c.cid, "round {round}");
    }
}

#[test]
fn another_tempo_curve_is_a_change_the_plan_says() {
    let (mut h, mut live) = playing();
    let a = "deck A: kick x ~ x ~\nat bar 16: tempo 128 -> 140 over 4 bars\n";
    let ev = live.eval(a);
    assert!(ev.ok);
    // a first program is not a change of one
    assert!(!ev.plan.as_ref().unwrap().tempo_changed);
    h.run(&mut live, 4.0 * 2.0);
    // the same curve moved: the nets are the same, no control line is, and the plan still says it
    let ev = live.eval(&a.replace("at bar 16", "at bar 18"));
    assert!(ev.ok);
    let plan = ev.plan.unwrap();
    assert!(plan.tempo_changed && !plan.is_empty(), "{plan:?}");
    assert!(plan.lines().iter().any(|l| l == "tempo curve changed"), "{:?}", plan.lines());
    assert_eq!(plan.to_json().get("tempo_changed").and_then(JsonValue::as_bool), Some(true));
    h.run(&mut live, 4.0 * 2.0);
    // text that moves nothing (the same curve, spelled another way) is no change at all
    let ev = live.eval(&a.replace("at bar 16", "at bar 18").replace("over 4 bars", "over 16 beats"));
    assert!(ev.unchanged);
    // a tempo that is one number changing is a change of the nets, as it always was
    let ev = live.eval("tempo 120\ndeck A: kick x ~ x ~\n");
    assert!(ev.ok);
    h.run(&mut live, 4.0 * 2.0);
    let ev2 = live.eval("tempo 126\ndeck A: kick x ~ x ~\n");
    assert!(!ev2.plan.unwrap().tempo_changed);
}

#[test]
fn a_tempo_port_feeds_a_tempo_line_and_an_import_brings_a_def_with_a_tempo() {
    // F11: a `tempo` port carries a value, and the line in the def that takes it is a tempo from where the def is called
    let src = "def tempo_src() out: tempo {\n  emit 128\n}\ndef consumer() in: tempo {\n  tempo $tempo -> 140 over 4 bars exp\n  deck A: kick x\n}\nwire tempo_src.out -> consumer.tempo\nat bar 8: consumer()\n";
    let c = ok(src);
    let ev: Vec<(Rat, f64, f64, Rat, Shape)> = c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to, e.over, e.shape)).collect();
    assert_eq!(ev, vec![(r(8, 1), 128.0, 140.0, r(4, 1), Shape::Exp)]);
    assert_eq!(ok(&c.text).cid, c.cid);
    // an imported def with a tempo in it, called in a section (the tempo is the section's)
    let mut snips = djlive::env::StaticSnippets::default();
    let lib = "def slowdown(to) {\n  tempo 140 -> $to over 4 bars log\n}\n";
    let canon = ok(lib);
    snips.items.push(("slow".to_string(), canon.cid.clone(), canon.text.clone()));
    let env = djlive::env::Env { snippets: Some(&snips), ..djlive::env::Env::default() };
    let src = "import \"slow\"\nsection verse 4 bars {\n  deck A: kick x\n}\nsection outro 8 bars {\n  slowdown(90)\n  deck A: kick x x\n}\narrange verse outro\n";
    let c = djlive::compile::check_with(src, &env);
    let c = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&djlive::compile::check_with(src, &env).diagnostics, src)));
    let ev: Vec<(Rat, f64, f64, Rat, Shape)> = c.tempo_map().events().iter().map(|e| (e.at, e.from, e.to, e.over, e.shape)).collect();
    assert_eq!(ev, vec![(r(4, 1), 140.0, 90.0, r(4, 1), Shape::Log)]);
    assert_eq!(djlive::compile::check_with(&c.text, &env).compiled.unwrap().cid, c.cid);
}
