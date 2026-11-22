//! Sets (docs/BTW.md, "Sets"): programs of programs. CID stability, the timeline and a mix on a fake clock, nesting,
//! `until` cuts, and the key / tempo / length diagnostics.

use djengine::json::JsonValue;
use djlive::ast::Deck;
use djlive::env::{Env, StaticSnippets};
use djlive::runtime::{Grid, Live, LiveHost};

const ALPHA: &str = "# alpha\ntempo 130\nkey am\ndeck A: kick x ~ ~ ~ x ~ ~ ~ vel=100 | bass a2 ~ ~ ~ vel=100\nend at bar 8\n";
const BETA: &str = "# beta\ntempo 122\nkey em\ndeck B: kick x ~ x ~ vel=100 | melody e4 ~ g4 ~ vel=100\nend at bar 8\n";

/// The saved programs: name, CID and canonical text, as the store gives them.
fn store(progs: &[(&str, &str)]) -> StaticSnippets {
    let mut s = StaticSnippets::default();
    for (name, text) in progs {
        let c = djlive::check(text).compiled.unwrap_or_else(|| panic!("{name} does not compile"));
        s.items.push((name.to_string(), c.cid, c.text));
    }
    s
}

fn compile(text: &str, snips: &StaticSnippets) -> djlive::Check {
    djlive::compile::check_with(text, &Env { snippets: Some(snips), ..Env::default() })
}

fn ok(text: &str, snips: &StaticSnippets) -> djlive::Compiled {
    let c = compile(text, snips);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    c.compiled.unwrap()
}

fn errors(text: &str, snips: &StaticSnippets) -> Vec<String> {
    compile(text, snips).diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

fn warnings(text: &str, snips: &StaticSnippets) -> Vec<String> {
    compile(text, snips).diagnostics.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect()
}

const SET: &str = "set {\n  play alpha\n  mix 4 bars with crossfade\n  play beta\n}\n";

#[test]
fn a_set_is_a_program_whose_cid_covers_its_tracks_and_transitions() {
    let s = store(&[("alpha", ALPHA), ("beta", BETA)]);
    let c = ok(SET, &s);
    let (ca, cb) = (&s.items[0].1, &s.items[1].1);
    // canonical: the tracks pinned by CID
    assert_eq!(c.text, format!("set {{\n  play {ca}\n  mix 4 bars with crossfade\n  play {cb}\n}}\n"));
    // names, CIDs, the end of a CID, quotes, spacing: one program, one CID
    let other = format!("set  {{\n\n   play   \"alpha\"   \n mix 4   bars   with crossfade\n  play {cb}\n }}");
    assert_eq!(ok(&other, &s).cid, c.cid);
    let tail = &ca[ca.len() - 10..];
    assert_eq!(ok(&format!("set {{ play {tail}\n mix 4 bars with crossfade\n play beta }}"), &s).cid, c.cid);
    // a different mix, or a different track, is a different set
    assert_ne!(ok(&SET.replace("4 bars", "8 bars"), &s).cid, c.cid);
    assert_ne!(ok(&SET.replace("crossfade", "echo_out"), &s).cid, c.cid);
    let s2 = store(&[("alpha", &ALPHA.replace("a2", "c3")), ("beta", BETA)]);
    assert_ne!(ok(SET, &s2).cid, c.cid);
    // times are one spelling
    let long = "# long\ndeck A: kick x ~ ~ ~\nloop\n";
    let s3 = store(&[("long", long)]);
    let a = ok("set { play long until 20m }", &s3);
    assert_eq!(a.cid, ok("set { play long until 1200s }", &s3).cid);
    assert_eq!(a.cid, ok("set { play long until 0.3333333333333333h }", &s3).cid);
}

#[test]
fn the_timeline_and_the_report_say_where_each_track_and_mix_sit() {
    let s = store(&[("alpha", ALPHA), ("beta", BETA)]);
    let c = ok(SET, &s);
    let r = c.set.as_ref().unwrap();
    // 8 + 8 - 4 bars
    assert_eq!(r.total_bars, 12);
    assert_eq!(c.song_bars(), Some(12));
    assert_eq!(c.end, Some(12));
    assert_eq!((r.tracks[0].start, r.tracks[0].bars, r.tracks[0].tail_mix), (0, 8, 4));
    assert_eq!((r.tracks[1].start, r.tracks[1].bars, r.tracks[1].head_mix), (4, 8, 4));
    assert_eq!(r.tracks[0].label, "alpha");
    assert_eq!(r.tracks[0].camelot.as_deref(), Some("8A"));
    assert_eq!(r.tracks[1].camelot.as_deref(), Some("9A"));
    let m = &r.mixes[0];
    assert_eq!((m.start, m.bars, m.key_ok), (4, 4, Some(true)));
    assert_eq!(m.plan, "mix 1→2: 130→122 over 4 bars (crossfade)");
    // minutes: 4 bars at 130, four mix bars between, 4 bars at 122
    assert!(r.seconds > 20.0 && r.seconds < 30.0, "{}", r.seconds);
    // the stretches: track 1's alone, the four one-bar mix stretches, track 2's alone
    let names: Vec<(&str, u32, u32)> = c.segments.iter().map(|s| (s.name.as_str(), s.start, s.bars)).collect();
    assert_eq!(names, vec![("1. track", 0, 4), ("mix 1→2", 4, 1), ("mix 1→2", 5, 1), ("mix 1→2", 6, 1), ("mix 1→2", 7, 1), ("2. track", 8, 4)]);
    // the clean ends are one section each (the playhead moves through a whole track), a mix is another
    assert!(c.segments[2].chain && c.segments[3].chain && !c.segments[1].chain && !c.segments[5].chain);
}

#[test]
fn a_mix_fades_the_parts_one_track_has_and_ramps_the_tempo_bar_by_bar() {
    let s = store(&[("alpha", ALPHA), ("beta", BETA)]);
    let c = ok(SET, &s);
    let mix: Vec<_> = c.segments.iter().filter(|s| s.name.starts_with("mix")).collect();
    assert_eq!(mix.len(), 4);
    let vel = |seg: &djlive::compile::Segment, d: Deck, role: &str| -> u8 {
        seg.decks.get(&d).and_then(|ir| ir.parts.iter().find(|p| p.role == role)).and_then(|p| p.cycles[0].first()).map_or(0, |h| h.vel)
    };
    // alpha's bass (deck A only it has) falls, beta's melody (deck B) rises
    let bass: Vec<u8> = mix.iter().map(|s| vel(s, Deck::A, "bass")).collect();
    let mel: Vec<u8> = mix.iter().map(|s| vel(s, Deck::B, "melody")).collect();
    assert!(bass.windows(2).all(|w| w[0] > w[1]), "{bass:?}");
    assert!(mel.windows(2).all(|w| w[0] < w[1]), "{mel:?}");
    // the kick is on A for alpha and on B for beta: different decks, each its own fade
    assert!(vel(mix[0], Deck::A, "kick") > vel(mix[3], Deck::A, "kick"));
    // the tempo goes 130 -> 122 in four steps, at the middle of each bar
    let tempos: Vec<f64> = mix.iter().map(|s| s.decks.values().next().unwrap().tempo.unwrap()).collect();
    for (k, t) in tempos.iter().enumerate() {
        let want = 130.0 + (122.0 - 130.0) * (k as f64 + 0.5) / 4.0;
        assert!((t - want).abs() < 1e-9, "{tempos:?}");
    }
    // both decks have a net in every mix bar; alone, track 1 had only A and track 2 only B
    assert!(mix.iter().all(|s| s.decks.len() == 2));
    assert_eq!(c.segments[0].decks.keys().copied().collect::<Vec<_>>(), vec![Deck::A]);
    assert_eq!(c.segments.last().unwrap().decks.keys().copied().collect::<Vec<_>>(), vec![Deck::B]);
}

#[test]
fn a_role_both_tracks_have_on_a_deck_swaps_at_the_middle_of_the_mix() {
    // both have a kick on deck A
    let one = "tempo 128\ndeck A: kick x ~ ~ ~ vel=90\nend at bar 6\n";
    let two = "tempo 128\ndeck A: kick x ~ x ~ vel=90\nend at bar 6\n";
    let s = store(&[("one", one), ("two", two)]);
    let c = ok("set { play one\n mix 4 bars with crossfade\n play two }", &s);
    let mix: Vec<_> = c.segments.iter().filter(|s| s.name.starts_with("mix")).collect();
    let steps = |seg: &djlive::compile::Segment| seg.decks[&Deck::A].parts.iter().find(|p| p.role == "kick").unwrap().cycles[0].len();
    // one kick hit a bar while track one plays it, two once track two has taken the role; never both
    assert_eq!(mix.iter().map(|s| steps(s)).collect::<Vec<_>>(), vec![1, 1, 2, 2]);
    assert!(mix.iter().all(|s| s.decks[&Deck::A].parts.iter().filter(|p| p.role == "kick").count() == 1));
}

/// A fake clock like the runtime's own tests: four beats a bar, a deck that plays when told. It notes the beat of each
/// thing the runtime does, and the program bar the clock was at on each beat, so a test can say "on set bar 8".
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, beat, tempo of the net)
    loads: Vec<(String, f64, f64)>,
    stops: Vec<(String, f64)>,
    calls: Vec<(f64, String)>,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        Some(Grid { deck: "A".into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }
    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        let tempo = env.canonical_js().split("\"tempo\":").nth(1).and_then(|r| r.split([',', '}']).next()).and_then(|t| t.parse().ok()).unwrap_or(0.0);
        self.loads.push((deck.into(), self.beat, tempo));
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
        if !on {
            self.stops.push((deck.into(), self.beat));
        }
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, format!("{noun} {}", params.canonical_js())));
        Ok(JsonValue::Null)
    }
}

struct Run {
    h: Fake,
    live: Live,
    /// (program bar, the section header's start, ended), one per step
    seen: Vec<(i64, Option<usize>, bool)>,
    /// The program bar the set began on.
    origin: i64,
    /// beat -> program bar
    bars: Vec<(f64, i64)>,
}

impl Run {
    /// The set bar a beat was in (0 is the set's first).
    fn rel(&self, beat: f64) -> i64 {
        self.bars.iter().find(|(b, _)| (*b - beat).abs() < 1e-9).map_or(i64::MIN, |(_, l)| l - self.origin)
    }
}

fn run_set(text: &str, snips: &StaticSnippets, bars: usize) -> Run {
    let mut h = Fake { beat: 8.0, playing: [true, false], ..Fake::default() };
    let mut live = Live::new();
    let ev = live.eval_with(text, &Env { snippets: Some(snips), ..Env::default() });
    assert!(ev.ok, "{:?}", ev.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    let (mut seen, mut barmap) = (Vec::new(), Vec::new());
    let (mut origin, mut offset): (Option<i64>, Option<f64>) = (None, None);
    for _ in 0..bars * 16 {
        h.beat += 0.25;
        live.service(&mut h);
        let m = live.marks();
        if live.is_running() && origin.is_none() {
            origin = Some(live.origin());
            offset = Some(m.pos - h.beat / 4.0);
        }
        // the clock is the beat: four to a bar, from where the runtime's bar count was when it began (it reads 0 once the program has ended)
        let bar = offset.map_or(m.pos, |o| h.beat / 4.0 + o).floor() as i64;
        barmap.push((h.beat, bar));
        seen.push((bar, m.header.map(|(s, _)| s.start), m.ended));
    }
    Run { h, live, seen, origin: origin.unwrap_or(0), bars: barmap }
}

#[test]
fn on_a_fake_clock_the_decks_alternate_across_the_mix_the_tempo_ramps_and_the_outgoing_deck_stops() {
    let s = store(&[("alpha", ALPHA), ("beta", BETA)]);
    let r = run_set(SET, &s, 20);
    let loads: Vec<(String, i64, f64)> = r.h.loads.iter().map(|l| (l.0.clone(), r.rel(l.1), l.2)).collect();
    eprintln!("loads {loads:?}\nstops {:?}", r.h.stops.iter().map(|s| (s.0.clone(), r.rel(s.1))).collect::<Vec<_>>());
    // track 1 is on A alone, at its own tempo, until the mix; the mix's first net (set bar 4) is loaded a bar ahead
    assert!(loads.iter().take_while(|l| l.1 < 3).all(|l| l.0 == "A" && l.2 == 130.0), "{loads:?}");
    let b_first = loads.iter().find(|l| l.0 == "B").unwrap();
    assert_eq!(b_first.1, 3, "B comes in with the mix: {loads:?}");
    // in the mix both decks are loaded every bar, the tempo falling a step a bar from 130 to 122
    let ramp: Vec<f64> = loads.iter().filter(|l| l.0 == "A" && (3..7).contains(&l.1)).map(|l| l.2).collect();
    assert_eq!(ramp.len(), 4, "{loads:?}");
    assert!(ramp.windows(2).all(|w| w[0] > w[1]) && ramp[0] < 130.0 && ramp[3] > 122.0, "{ramp:?}");
    // when the mix is over the outgoing deck stops, and B plays alone at its own tempo
    let stop_a = r.h.stops.iter().find(|s| s.0 == "A").map(|s| r.rel(s.1)).expect("A stops");
    assert_eq!(stop_a, 8, "A plays the last bar of the mix and stops on the bar line the mix ends on");
    assert_eq!(loads.iter().rev().find(|l| l.0 == "B").unwrap().2, 122.0);
    // after set bar 12 the set has ended: the decks stop, and the program says so
    assert!(r.seen.last().unwrap().2, "ended");
    let stop_b = r.h.stops.iter().find(|s| s.0 == "B").map(|s| r.rel(s.1)).expect("B stops at the end");
    assert_eq!(stop_b, 12, "B stops on the bar line the set ends on");
    // the playhead sat on track 1's `play` line, then the `mix` line, then track 2's `play` line
    let headers: Vec<usize> = r.seen.iter().filter_map(|x| x.1).fold(Vec::new(), |mut v, s| {
        if v.last() != Some(&s) {
            v.push(s);
        }
        v
    });
    assert_eq!(headers.len(), 3, "{headers:?}");
    let src = &djlive::compile::check_with(SET, &Env { snippets: Some(&s), ..Env::default() }).compiled.unwrap().source;
    let lines: Vec<&str> = headers.iter().map(|h| src[*h..].lines().next().unwrap()).collect();
    assert_eq!(lines, vec!["play alpha", "mix 4 bars with crossfade", "play beta"]);
}

#[test]
fn a_tracks_control_lines_run_only_while_the_track_does() {
    let a = "# a\ndeck A: kick x ~ ~ ~\nevery 2 bars: master: reverb 0.2\nend at bar 6\n";
    let b = "# b\ndeck B: kick x ~ ~ ~\nend at bar 6\n";
    let s = store(&[("a", a), ("b", b)]);
    let c = ok("set { play a\n play b }", &s);
    assert_eq!(c.controls.len(), 1);
    assert_eq!(c.controls[0].window, Some((0, 6)));
    assert_eq!(c.controls[0].timing, djlive::compile::Timing::EveryFrom(2, 2));
    let r = run_set("set { play a\n play b }", &s, 14);
    let bars: Vec<i64> = r.h.calls.iter().filter(|c| c.1.contains("reverb.mix")).map(|c| r.rel(c.0)).collect();
    // every 2 bars from bar 2 while track a plays (bars 0 to 5): bars 2 and 4, and not 6, 8 or 10
    assert_eq!(bars, vec![2, 4], "{bars:?}");
}

#[test]
fn a_set_can_play_a_set() {
    let s0 = store(&[("alpha", ALPHA), ("beta", BETA), ("gamma", "# gamma\ntempo 122\nkey em\ndeck A: kick x ~ ~ ~\nend at bar 8\n")]);
    let inner = ok(SET, &s0);
    let mut s = s0.clone();
    s.items.push(("inner".into(), inner.cid.clone(), inner.text.clone()));
    let outer = ok("set { play inner\n mix 2 bars with crossfade\n play gamma }", &s);
    let r = outer.set.as_ref().unwrap();
    assert!(r.tracks[0].nested && !r.tracks[1].nested);
    assert_eq!(r.tracks[0].bars, 12);
    assert_eq!(r.total_bars, 12 + 8 - 2);
    // the inner set's tracks are all in the outer's inline bundle
    assert_eq!(outer.plays.len(), 4, "{:?}", outer.plays.iter().map(|p| p.0.clone()).collect::<Vec<_>>());
    // and it plays through to the end
    let r = run_set("set { play inner\n mix 2 bars with crossfade\n play gamma }", &s, 26);
    assert!(r.seen.last().unwrap().2);
}

#[test]
fn until_cuts_a_long_or_looping_track_and_a_track_with_an_end_plays_to_it() {
    let long = "# long\ntempo 120\ndeck A: kick x ~ ~ ~\nloop\n";
    let song = "# song\ntempo 120\nsection intro 4 bars {\n deck A: kick x ~ ~ ~\n}\nsection drop 8 bars {\n deck A: kick x x x x\n}\nsection outro 4 bars {\n deck A: kick x ~ ~ ~\n}\narrange intro drop outro\n";
    let endless = "# endless\ndeck A: kick x ~ ~ ~\n";
    let s = store(&[("long", long), ("song", song), ("endless", endless)]);
    let bars = |t: &str| ok(t, &s).set.unwrap().tracks[0].bars;
    // a time is read at the track's own tempo: 20 minutes at 120 is 600 bars
    assert_eq!(bars("set { play long until 20m }"), 600);
    assert_eq!(bars("set { play long until 90s }"), 45);
    assert_eq!(bars("set { play long until bar 7 }"), 7);
    // a song plays to its end; until a section stops where it starts, until after it stops where it ends
    assert_eq!(bars("set { play song }"), 16);
    assert_eq!(bars("set { play song until drop }"), 4);
    assert_eq!(bars("set { play song until after drop }"), 12);
    assert_eq!(bars("set { play song until bar 10 }"), 10);
    // a cut past the end is a warning, and the track just ends
    assert_eq!(bars("set { play song until 20m }"), 16);
    assert!(warnings("set { play song until 20m }", &s).iter().any(|w| w.contains("ends at bar 16")));
    // a looping track runs round up to the cut: two rounds of the song's sections
    let looped = "# looped\ntempo 120\nsection first 2 bars {\n deck A: kick x ~ ~ ~\n}\nsection second 2 bars {\n deck A: kick x x x x\n}\narrange first second\nloop\n";
    let s2 = store(&[("looped", looped)]);
    let c = ok("set { play looped until bar 6 }", &s2);
    assert_eq!(c.set.as_ref().unwrap().tracks[0].bars, 6);
    assert_eq!(c.segments.iter().map(|s| (s.start, s.bars)).collect::<Vec<_>>(), vec![(0, 2), (2, 2), (4, 2)]);
    // no end and no cut: an error that says which
    assert!(errors("set { play endless }", &s).iter().any(|e| e.contains("no end")));
    assert!(errors("set { play long }", &s).iter().any(|e| e.contains("loops")));
    assert!(errors("set { play song until finale }", &s).iter().any(|e| e.contains("no section `finale`")));
}

#[test]
fn check_reports_tempo_and_key_between_tracks() {
    let s = store(&[
        ("alpha", ALPHA),
        ("fast", "# fast\ntempo 150\nkey am\ndeck A: kick x ~ ~ ~\nend at bar 8\n"),
        ("clash", "# clash\ntempo 128\nkey f#\ndeck B: kick x ~ ~ ~\nend at bar 8\n"),
        ("nokey", "# nokey\ntempo 130\ndeck B: kick x ~ ~ ~\nend at bar 8\n"),
    ]);
    // 130 -> 150 is a 15% jump
    let w = warnings("set { play alpha\n mix 4 bars with crossfade\n play fast }", &s);
    assert!(w.iter().any(|m| m.contains("130→150") && m.contains("15%")), "{w:?}");
    // keys: 8A to 2B clash; the same key is fine; an unknown key is said in the report, not guessed
    let w = warnings("set { play alpha\n mix 4 bars with crossfade\n play clash }", &s);
    assert!(w.iter().any(|m| m.contains("keys clash") && m.contains("8A") && m.contains("2B")), "{w:?}");
    let c = ok("set { play alpha\n mix 4 bars with crossfade\n play nokey }", &s);
    assert_eq!(c.set.unwrap().mixes[0].key_ok, None);
    // the explicit ramp is the plan, and a ramp that is not where the tracks are is said
    let c = ok("set { play alpha\n mix 8 bars with crossfade, tempo 130 -> 130\n play nokey }", &s);
    assert_eq!(c.set.unwrap().mixes[0].plan, "mix 1→2: 130→130 over 8 bars (crossfade)");
    let w = warnings("set { play alpha\n mix 4 bars with crossfade, tempo 140 -> 130\n play nokey }", &s);
    assert!(w.iter().any(|m| m.contains("ramps 140→130")), "{w:?}");
    // a mix that does not fit, two in a row, an unknown transition, an unknown track
    assert!(errors("set { play alpha\n mix 12 bars with crossfade\n play nokey }", &s).iter().any(|e| e.contains("does not fit")));
    assert!(errors("set { play alpha\n mix 2 bars with crossfade\n mix 2 bars with cut\n play nokey }", &s).iter().any(|e| e.contains("two `mix`es")));
    let e = compile("set { play alpha\n mix 4 bars with crossfad\n play nokey }", &s).diagnostics;
    assert!(e.iter().any(|d| d.message.contains("not a transition") && d.help.as_deref().is_some_and(|h| h.contains("crossfade"))));
    assert!(errors("set { play nothing-here }", &s).iter().any(|e| e.contains("no saved")));
    // the mixes at both ends of a short track must not overlap
    assert!(errors("set { play alpha\n mix 5 bars with crossfade\n play nokey\n mix 5 bars with crossfade\n play alpha }", &s).iter().any(|e| e.contains("overlap each other")));
    // a set holds the set, and nothing else
    assert!(errors("deck A: kick x\nset { play alpha }", &s).iter().any(|e| e.contains("nothing else")));
    // a cut has no overlap
    let w = warnings("set { play alpha\n mix 4 bars with cut\n play nokey }", &s);
    assert!(w.iter().any(|m| m.contains("a cut has no overlap")));
    let c = ok("set { play alpha\n mix 4 bars with cut\n play nokey }", &s);
    assert_eq!(c.set.unwrap().total_bars, 16);
}

#[test]
fn the_camelot_wheel_and_its_neighbours() {
    use djlive::music::Key;
    use djlive::set::{camelot, camelot_compatible};
    let k = |s: &str| camelot(Key::parse(s).unwrap());
    assert_eq!((k("am").as_str(), k("c").as_str(), k("em").as_str(), k("g").as_str()), ("8A", "8B", "9A", "9B"));
    assert_eq!((k("dm").as_str(), k("f#").as_str(), k("bbm").as_str()), ("7A", "2B", "3A"));
    assert!(camelot_compatible("8A", "8B") && camelot_compatible("8A", "9A") && camelot_compatible("8A", "7A") && camelot_compatible("12A", "1A"));
    assert!(!camelot_compatible("8A", "10A") && !camelot_compatible("8A", "9B") && !camelot_compatible("8A", "2B"));
}

#[test]
fn a_share_link_of_a_set_always_carries_its_tracks_however_long_it_is() {
    use djlive::share;
    let s = store(&[("alpha", ALPHA), ("beta", BETA)]);
    let c = ok(SET, &s);
    let (url, inlined) = share::encode_with_tracks(&c, None, false);
    assert!(inlined && url.chars().count() <= share::INLINE_LIMIT, "{} chars", url.chars().count());
    // a receiver with nothing saved opens it: the tracks came in the link, and it is the same set
    let nothing = StaticSnippets::default();
    let opened = share::open(&url, &Env { snippets: Some(&nothing), ..Env::default() }).unwrap();
    assert_eq!(opened.cid, c.cid);
    assert!(opened.check.compiled.is_some());
    assert_eq!(opened.check.compiled.unwrap().set.unwrap().total_bars, 12);
    // a track that is not the program its CID names is refused
    let link = share::parse_link(&url).unwrap();
    assert_eq!(link.tracks.len(), 2);
    let mut bad = c.clone();
    bad.plays[0].1 = bad.plays[0].1.replace("a2", "c3");
    let (bad_url, _) = share::encode_with_tracks(&bad, None, false);
    let e = share::open(&bad_url, &Env { snippets: Some(&nothing), ..Env::default() }).err().expect("refused");
    assert!(e.contains("not the program it names"), "{e}");
    // too long to be a short link: it still carries the tracks (a link that named them by CID alone would open here and not
    // on a machine without them), and says it is long; the receiver needs nothing saved
    let mut big = c.clone();
    // text that does not compress
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..300 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        big.plays[0].1.push_str(&format!("# {x:x}\n"));
    }
    let (url2, inlined2) = share::encode_with_tracks(&big, None, false);
    assert!(inlined2 && url2.chars().count() > share::INLINE_LIMIT && share::parse_link(&url2).unwrap().tracks.len() == 2);
    // (its first track now has other text than its CID names: refused, not quietly read from storage)
    let e = share::open(&url2, &Env { snippets: Some(&s), ..Env::default() }).err().expect("a carried program is the one its CID names");
    assert!(e.contains("not the program it names"), "{e}");
    // past what a link may carry, it is only the program, and says it did not carry them
    let mut huge = c.clone();
    huge.plays[0].1.push_str(&"# x\n".repeat(share::MAX_CARRIED / 4));
    let (url3, inlined3) = share::encode_with_tracks(&huge, None, false);
    assert!(!inlined3 && share::parse_link(&url3).unwrap().tracks.is_empty());
}

/// Property: any reformatting of a set (spaces, blank lines, comments, how a track is named, how a time is spelled) is the
/// same program with the same CID; changing one thing that is a program (a track, a mix, a cut) is another.
#[test]
fn any_reformatting_of_a_set_keeps_its_cid_and_any_real_change_moves_it() {
    let long = "# long\ntempo 120\ndeck A: kick x ~ ~ ~\nloop\n";
    let s = store(&[("alpha", ALPHA), ("beta", BETA), ("long", long)]);
    let (ca, cb, cl) = (s.items[0].1.clone(), s.items[1].1.clone(), s.items[2].1.clone());
    let base = ok("set {\n  play alpha\n  mix 4 bars with crossfade\n  play beta\n  mix 2 bars with echo_out, tempo 122 -> 120\n  play long until 20m\n}\nend\n", &s);
    let mut x: u64 = 0xdead_beef_cafe_f00d;
    let mut rnd = |n: u64| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) % n
    };
    for _ in 0..60 {
        let sp = |r: u64| " ".repeat(1 + r as usize);
        let nl = |r: u64| if r == 0 { "\n".to_string() } else { format!("{}\n", "\n".repeat(r as usize)) };
        let name = |n: &str, cid: &String, r: u64| match r {
            0 => n.to_string(),
            1 => format!("\"{n}\""),
            2 => cid.clone(),
            // the end of a CID that starts with a digit reads as a number: it goes in quotes
            _ if cid[cid.len() - 12..].starts_with(|c: char| c.is_ascii_digit()) => format!("\"{}\"", &cid[cid.len() - 12..]),
            _ => cid[cid.len() - 12..].to_string(),
        };
        let time = ["20m", "1200s", "0.3333333333333333h", "20m"][rnd(4) as usize];
        let text = format!(
            "# {c}\nset{a}{{{n1}play{a}{p1}{n2}mix{a}4{a}bars{a}with{a}crossfade{n3}play{a}{p2}{n4}mix 2 bar{a}with echo_out{a},{a}tempo{a}122{a}->{a}120{n5}play {p3} until{a}{t}{n6}}}{n7}end\n",
            c = rnd(1000),
            a = sp(rnd(3)),
            n1 = nl(rnd(3)),
            n2 = nl(rnd(3)),
            n3 = nl(rnd(3)),
            n4 = nl(rnd(3)),
            n5 = nl(rnd(3)),
            n6 = nl(rnd(3)),
            n7 = nl(rnd(3)),
            p1 = name("alpha", &ca, rnd(4)),
            p2 = name("beta", &cb, rnd(4)),
            p3 = name("long", &cl, rnd(4)),
            t = time,
        );
        let c = compile(&text, &s);
        let comp = c.compiled.unwrap_or_else(|| panic!("{text:?}: {:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        assert_eq!(comp.cid, base.cid, "{text:?}");
        assert_eq!(comp.text, base.text);
    }
    // real changes
    let changed = |t: &str| ok(t, &s).cid;
    let b = "set {\n  play alpha\n  mix 4 bars with crossfade\n  play beta\n  mix 2 bars with echo_out, tempo 122 -> 120\n  play long until 20m\n}\nend\n";
    for variant in [b.replace("mix 4", "mix 3"), b.replace("crossfade", "filter_exit"), b.replace("until 20m", "until 19m"), b.replace("122 -> 120", "122 -> 121"), b.replace("play beta", "play alpha"), b.replace("}\nend\n", "}\n")] {
        assert_ne!(changed(&variant), base.cid, "{variant}");
    }
}
