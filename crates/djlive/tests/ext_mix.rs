//! F8, mix moves (docs/btw/ext-mix.md): `duck B under A.kick 6db attack=5ms release=120ms` and
//! `mix { A level 0.7, A eq low -3 }`. Parse and the canonical form (random reformatting keeps the CID, an edit changes
//! it), the check's report of the registry calls, and the runtime on a fake host with a fake mixer and a fake clock:
//! a section's snapshot applies on its first bar, is read off the mixer first and goes back on the bar the section
//! ends on, whatever ends it.

mod common;

use std::collections::BTreeMap;

use common::{reformat, Rng};
use djengine::json::{JsonObject, JsonValue};
use djlive::compile::{check, Effect, Timing};
use djlive::diag::Severity;
use djlive::highlight::{highlight, Class};
use djlive::runtime::{Grid, Live, LiveHost};

// ---- the program

const SONG: &str = "\
tempo 124
section intro 4 bars {
  deck A: kick x ~ x ~
  deck B: bass c2
}
section drop 8 bars {
  deck A: kick x ~ x ~
  deck B: bass c2
  duck B under A.kick 6db attack=5ms release=120ms
  mix {
    A level 0.7
    master level 0.9
    A eq low -3
  }
}
section outro 4 bars {
  deck A: kick x ~ x ~
  deck B: bass c2
}
arrange intro drop outro
";

// ---- a fake host with a mixer you can read

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, net cid, the bar it lands on)
    loads: Vec<(String, String, i64)>,
    /// (beat when called, noun, params): every call but a read of the mixer
    calls: Vec<(f64, String, String)>,
    /// The beats `mixer {}` was read at.
    reads: Vec<f64>,
    pauses: Vec<String>,
    /// `A.volume`, `A.low`, `master` ...: what the mixer holds (a knob not in here is at the engine's start).
    mixer: BTreeMap<String, f64>,
    /// `mixer {}` answers its state (a host that cannot says nothing).
    blind: bool,
    bpm: f64,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl Fake {
    fn state(&self) -> JsonValue {
        let get = |k: &str, d: f64| JsonValue::Number(self.mixer.get(k).copied().unwrap_or(d));
        let strip = |d: &str| {
            JsonValue::Object(JsonObject::from_pairs([
                ("deck", JsonValue::str(d)),
                ("volume", get(&format!("{d}.volume"), 0.8)),
                ("eq", JsonValue::Object(JsonObject::from_pairs([("high", get(&format!("{d}.high"), 0.5)), ("mid", get(&format!("{d}.mid"), 0.5)), ("low", get(&format!("{d}.low"), 0.5)), ("filter", JsonValue::Number(0.5))]))),
            ]))
        };
        JsonValue::Object(JsonObject::from_pairs([
            ("set", JsonValue::Array(Vec::new())),
            ("mixer", JsonValue::Object(JsonObject::from_pairs([("strips", JsonValue::Array(vec![strip("A"), strip("B")])), ("master", get("master", 1.0))]))),
        ]))
    }

    fn knob(&self, k: &str, d: f64) -> f64 {
        self.mixer.get(k).copied().unwrap_or(d)
    }
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: if self.bpm > 0.0 { self.bpm } else { 120.0 } })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }

    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), djengine::cid::share_cid(env), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
        if !on {
            self.pauses.push(deck.into());
        }
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        if noun == "mixer" {
            let o = params.as_object().cloned().unwrap_or_default();
            if o.iter().next().is_none() {
                self.reads.push(self.beat);
                return Ok(if self.blind { JsonValue::Null } else { self.state() });
            }
            let deck = o.get("deck").and_then(JsonValue::as_str).unwrap_or("").to_string();
            for (k, v) in o.iter() {
                let Some(v) = v.as_f64() else { continue };
                match k {
                    "volume" | "low" | "mid" | "high" => {
                        self.mixer.insert(format!("{deck}.{}", k), v);
                    }
                    "master" => {
                        self.mixer.insert("master".into(), v);
                    }
                    _ => {}
                }
            }
        }
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    /// Advance `beats` in sixteenth-bar steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }

    /// The bar the song's first net landed on (the song's bar 0).
    fn origin(&self) -> i64 {
        self.loads.iter().find(|l| l.0 == "A").map_or(0, |l| l.2)
    }

    /// Run to the start of the song's bar `rel` (just past its bar line).
    fn run_to(&mut self, live: &mut Live, rel: i64) {
        if self.loads.is_empty() {
            // the program lands on the next bar line: one pass loads its first net
            self.run(live, 0.25);
        }
        let want = (self.origin() + rel) as f64 * 4.0;
        if want > self.beat {
            self.run(live, want - self.beat);
        }
    }
}

fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    // the mixer as someone left it
    h.mixer.insert("A.volume".into(), 0.65);
    h.mixer.insert("master".into(), 0.75);
    (h, Live::new())
}

fn bar_of(beat: f64) -> i64 {
    (beat / 4.0).floor() as i64
}

// ---- the syntax and the canonical form

#[test]
fn the_canonical_text_of_the_song_is_the_contracts() {
    let c = check(SONG).compiled.expect("the song compiles");
    assert!(c.text.contains("  duck B under A.kick 6db attack=5ms release=120ms\n"), "{}", c.text);
    assert!(c.text.contains("  mix {\n    A level 0.7\n    master level 0.9\n    A eq low -3\n  }\n"), "{}", c.text);
    // a fixed point
    let again = check(&c.text).compiled.unwrap();
    assert_eq!((again.text, again.cid), (c.text.clone(), c.cid.clone()));
}

#[test]
fn a_duck_has_params_in_any_order_and_one_spelling() {
    let a = check("deck A: kick x\nduck B under A.kick 6db attack=5ms release=120ms\n").compiled.unwrap();
    let b = check("deck A: kick x\nduck B under A.kick 6db release=0.12s attack=0.005s\n").compiled.unwrap();
    let c = check("deck A: kick x\nduck B under A.kick 6db\n").compiled.unwrap();
    let d = check("deck A: kick x\nduck B under A.kick 6.0db  release=120ms\n").compiled.unwrap();
    assert_eq!(a.text, b.text);
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.cid, c.cid, "the defaults are filled in");
    assert_eq!(a.cid, d.cid);
    // a semantic edit is another program
    for edit in ["duck B under A.kick 7db", "duck B under A.kick 6db attack=6ms", "duck B under A.kick 6db release=121ms", "duck A under B.kick 6db"] {
        let src = format!("deck A: kick x\ndeck B: kick x\n{edit}\n");
        let base = check("deck A: kick x\ndeck B: kick x\nduck B under A.kick 6db\n").compiled.unwrap();
        assert_ne!(check(&src).compiled.unwrap().cid, base.cid, "{edit}");
    }
    assert_ne!(
        check("deck A: kick x\ndeck B: bass c2\nduck B under A.kick 6db\n").compiled.unwrap().cid,
        check("deck A: kick x\ndeck B: bass c2\nduck B under A.bass 6db\n").compiled.map(|c| c.cid).unwrap_or_default()
    );
}

#[test]
fn the_same_text_is_the_same_program_and_the_same_report() {
    let (a, b) = (check(SONG).compiled.unwrap(), check(SONG).compiled.unwrap());
    assert_eq!((&a.cid, &a.text, a.mix_report()), (&b.cid, &b.text, b.mix_report()));
    assert_eq!(a.controls, b.controls);
    // and the canonical text compiles to the same controls (a round trip)
    let c = check(&a.text).compiled.unwrap();
    assert_eq!(c.controls.iter().map(|x| (&x.timing, &x.effect, &x.hold)).collect::<Vec<_>>(), a.controls.iter().map(|x| (&x.timing, &x.effect, &x.hold)).collect::<Vec<_>>());
}

#[test]
fn a_mix_is_one_program_however_its_items_are_separated_and_a_semantic_edit_changes_it() {
    let one = check("mix { A level 0.7, B level 0.9, A eq low -3 }\n").compiled.unwrap();
    let two = check("mix {\n  A level 0.7\n  B level 0.9\n\n  A eq low -3db\n}\n").compiled.unwrap();
    let three = check("mix { A level 70%,\n B level 0.90,\n A eq low -3 }\n").compiled.unwrap();
    assert_eq!(one.text, "mix {\n  A level 0.7\n  B level 0.9\n  A eq low -3\n}\n");
    assert_eq!((&one.cid, &one.cid), (&two.cid, &three.cid));
    for edit in ["mix { A level 0.71, B level 0.9, A eq low -3 }", "mix { A level 0.7, B level 0.9, A eq mid -3 }", "mix { A level 0.7, B level 0.9, A eq low -4 }", "mix { B level 0.7, master level 0.9, A eq low -3 }", "mix { A level 0.7, A eq low -3, B level 0.9 }", "mix { A level 0.7, master level 0.9, A eq low -3 }"] {
        assert_ne!(check(&format!("{edit}\n")).compiled.unwrap().cid, one.cid, "{edit}");
    }
}

#[test]
fn random_reformatting_keeps_the_cid() {
    let programs = [
        SONG,
        "deck A: kick x ~ x ~ | bass c2\ndeck B: harmony chord(am)\nduck B under A.kick 9db release=200ms\nat bar 4 for 2 bars: mix { A level 0.4, A eq high 3 }\nevery 8 bars: mix { master level 0.8 }\n",
        "def pumped(under, src, depth) {\n  duck $under under $src.kick $depth attack=2ms\n  mix { A level 0.6, B eq mid -2 }\n}\ndeck A: kick x ~ x ~\npumped(B, A, 6db)\n",
        "repeat 2 every 4 bars {\n  at bar 1: mix { master level 0.5 + $i * 0.1 }\n}\n",
    ];
    for (n, src) in programs.iter().enumerate() {
        let c = check(src);
        let want = c.compiled.unwrap_or_else(|| panic!("program {n}: {}", djlive::diag::render_all(&c.diagnostics, src))).cid;
        let mut rng = Rng(0xd0c5_0000 + n as u64);
        for round in 0..120 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("program {n} round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text))).cid;
            assert_eq!(got, want, "program {n} round {round} forked:\n{text}");
        }
    }
}

#[test]
fn a_def_and_a_repeat_carry_a_duck_and_a_mix() {
    let src = "def pumped(under, src, depth) {\n  duck $under under $src.kick $depth\n  mix { A level 0.6 }\n}\ndeck A: kick x ~ x ~\npumped(B, A, 6db)\nrepeat 2 every 4 bars {\n  at bar 1: mix { master level 0.5 + $i * 0.1 }\n}\n";
    let c = check(src);
    let comp = c.compiled.unwrap_or_else(|| panic!("{}", djlive::diag::render_all(&c.diagnostics, src)));
    let report = comp.mix_report();
    assert_eq!(report.len(), 4, "{report:?}");
    assert!(report[0].starts_with("duck B under A.kick 6db attack=5ms release=120ms"), "{report:?}");
    // the repeat's copies: bar 1 and bar 5, master 0.5 and 0.6
    let masters: Vec<(Timing, String)> = comp.controls.iter().filter(|c| matches!(c.effect, Effect::Mix(_)) && c.key.contains("master")).map(|c| (c.timing, c.key.clone())).collect();
    assert_eq!(masters.len(), 2);
    assert_eq!(masters[0].0, Timing::At(1));
    assert_eq!(masters[1].0, Timing::At(5));
    assert!(masters[0].1.contains("master level 0.5") && masters[1].1.contains("master level 0.6"), "{masters:?}");
    // the canonical text keeps the def and the repeat as written, and is a fixed point
    assert!(comp.text.contains("duck $under under $src.kick $depth"), "{}", comp.text);
    let again = check(&comp.text).compiled.unwrap();
    assert_eq!(again.cid, comp.cid);
}

#[test]
fn the_editor_colours_the_new_words() {
    let src = "duck B under A.kick 6db attack=5ms\nmix { A eq low -3 }\n";
    let classes: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    let class = |t: &str| classes.iter().find(|(s, _)| *s == t).map(|(_, c)| *c).unwrap_or_else(|| panic!("{t} not found in {classes:?}"));
    assert_eq!(class("duck"), Class::Keyword);
    assert_eq!(class("under"), Class::Keyword);
    assert_eq!(class("mix"), Class::Keyword);
    assert_eq!(class("eq"), Class::Keyword);
    assert_eq!(class("attack"), Class::Param);
    assert_eq!(class("kick"), Class::Role);
    assert_eq!(class("db"), Class::Number);
    assert_eq!(class("ms"), Class::Number);
    assert_eq!(class("6"), Class::Number);
}

// ---- what check says

#[test]
fn check_reports_the_registry_calls() {
    let comp = check(SONG).compiled.unwrap();
    let report = comp.mix_report();
    assert_eq!(report.len(), 2, "{report:?}");
    assert_eq!(
        report[0],
        "duck B under A.kick 6db attack=5ms release=120ms in drop: trick {\"action\":\"on\",\"amount\":0.499,\"deck\":\"B\",\"div\":16,\"kind\":\"pump\",\"source\":\"A\"}  (release 120ms is a 1/16 note at 124 BPM: 121 ms; depth 0.499 of the gain; the pump attacks in 2 ms); deck A's kick plays 2 hits a bar; put back on bar 12"
    );
    assert!(
        report[1].starts_with("mix in drop: A level 0.7, master level 0.9, A eq low -3 -> mixer {\"deck\":\"A\",\"volume\":0.7}, mixer {\"master\":0.9}, mixer {\"deck\":\"A\",\"low\":0.4375}; reads the mixer first, put back on bar 12"),
        "{}",
        report[1]
    );
    // with no tempo the release note is the deck's to choose
    let r = check("deck A: kick x\nduck B under A.kick 6db\n").compiled.unwrap().mix_report();
    assert!(r[0].contains("chosen from the deck's tempo when it fires") && !r[0].contains("\"div\""), "{r:?}");
    // a plain mix says it just sets
    let r = check("at bar 3: mix { master level 0.5 }\n").compiled.unwrap().mix_report();
    assert!(r[0].contains("just sets"), "{r:?}");
    // and the live answer carries it
    let live = Live::new();
    let ev = live.check(SONG);
    assert_eq!(ev.mix, report);
}

#[test]
fn check_warns_where_the_engine_cannot_do_what_was_asked() {
    // a slow attack: the pump's is 2 ms
    let c = check("tempo 124\ndeck A: kick x\nduck B under A.kick 6db attack=40ms\n");
    assert!(c.ok());
    let w: Vec<_> = c.diagnostics.iter().filter(|d| d.severity == Severity::Warning).collect();
    assert!(w.iter().any(|d| d.message.contains("attacks in 2 ms")), "{w:?}");
    // a release longer than a quarter note
    let c = check("tempo 124\ndeck A: kick x\nduck B under A.kick 6db release=2s\n");
    assert!(c.diagnostics.iter().any(|d| d.severity == Severity::Warning && d.message.contains("nearest to `release=2000ms` is a 1/4")), "{:?}", c.diagnostics);
    // the defaults are quiet
    let c = check("tempo 124\ndeck A: kick x\nduck B under A.kick 6db\n");
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    // a role that cannot key it is an error, with the way out
    let c = check("deck A: snare x\nduck B under A.snare 6db\n");
    assert!(!c.ok() && c.diagnostics[0].help.as_deref() == Some("duck B under A.kick 6db"), "{:?}", c.diagnostics);
}

#[test]
fn errors_carry_line_and_column() {
    let src = "tempo 124\ndeck A: kick x ~ x ~\nduck B under A.kick 6 db\n";
    let c = check(src);
    let d = &c.diagnostics[0];
    assert_eq!(d.position(src).0, 3);
    assert!(d.message.contains("glued") && d.render(src).contains("--> 3:"), "{}", d.render(src));
    let src = "mix {\n  A level 0.7\n  B levle 0.9\n}\n";
    let c = check(src);
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    let d = &c.diagnostics[0];
    assert_eq!(d.position(src), (3, 5, 10));
    assert_eq!(d.help.as_deref(), Some("did you mean `level`?"));
}

// ---- the runtime

#[test]
fn a_sections_mix_applies_on_its_first_bar_is_read_first_and_goes_back_on_the_bar_it_ends_on() {
    let (mut h, mut live) = playing();
    let ev = live.eval(SONG);
    assert!(ev.ok, "{:?}", ev.diagnostics);
    // the intro: nothing is touched
    h.run_to(&mut live, 3);
    assert!(h.calls_of("mixer").is_empty() && h.reads.is_empty(), "{:?}", h.calls);
    // the drop's first bar (song bar 4): read once, then set
    h.run_to(&mut live, 4);
    assert_eq!(h.reads.len(), 1);
    assert_eq!(bar_of(h.reads[0]) - h.origin(), 4, "read on the drop's bar line");
    let sets = h.calls_of("mixer");
    assert_eq!(sets.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), [r#"{"deck":"A","volume":0.7}"#, r#"{"master":0.9}"#, r#"{"deck":"A","low":0.4375}"#]);
    assert!(sets.iter().all(|s| bar_of(s.0) - h.origin() == 4));
    // in the middle of the section it holds
    h.run_to(&mut live, 11);
    assert_eq!((h.knob("A.volume", 0.8), h.knob("master", 1.0), h.knob("A.low", 0.5)), (0.7, 0.9, 0.4375));
    assert_eq!(h.calls_of("mixer").len(), 3, "nothing is sent again while it holds");
    // the drop's last bar is bar 11: it goes back on the line that ends it, bar 12, the outro's first
    h.run_to(&mut live, 12);
    assert_eq!((h.knob("A.volume", 0.8), h.knob("master", 1.0), h.knob("A.low", 0.5)), (0.65, 0.75, 0.5), "put back as the mixer was");
    let all = h.calls_of("mixer");
    assert_eq!(all.len(), 6);
    assert!(all[3..].iter().all(|c| bar_of(c.0) - h.origin() == 12), "{all:?}");
    // the last set goes back first
    assert_eq!(all[3].1, r#"{"deck":"A","low":0.5}"#);
    assert_eq!(all[5].1, r#"{"deck":"A","volume":0.65}"#);
    assert!(live.words.iter().any(|w| w.contains("put back") && w.contains("mix {")), "{:?}", live.words);
    // and nothing again after it
    h.run_to(&mut live, 15);
    assert_eq!(h.calls_of("mixer").len(), 6);
}

#[test]
fn the_next_sections_snapshot_is_taken_after_the_one_before_is_put_back() {
    let (mut h, mut live) = playing();
    let src = "section one 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection two 4 bars {\n  deck A: kick x\n  mix { A level 0.1 }\n}\nsection three 4 bars {\n  deck A: kick x\n}\narrange one two three\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 5);
    assert_eq!(h.knob("A.volume", 0.8), 0.1);
    // on the line the first ends and the second begins it is the mixer's own 0.65 that the second reads, not the first's 0.3
    let vols: Vec<String> = h.calls_of("mixer").into_iter().map(|c| c.1).collect();
    assert_eq!(vols, [r#"{"deck":"A","volume":0.3}"#, r#"{"deck":"A","volume":0.65}"#, r#"{"deck":"A","volume":0.1}"#]);
    h.run_to(&mut live, 8);
    assert_eq!(h.knob("A.volume", 0.8), 0.65, "back to what the mixer held before the song touched it");
}

#[test]
fn a_mixer_that_cannot_be_read_goes_back_to_the_programs_own_line_and_says_so() {
    let (mut h, mut live) = playing();
    h.blind = true;
    let src = "deck A: level 0.6\nsection one 4 bars {\n  deck A: kick x\n  mix { A level 0.3, master level 0.5 }\n}\nsection two 4 bars {\n  deck A: kick x\n}\narrange one two\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 1);
    assert_eq!(h.knob("A.volume", 0.8), 0.3);
    assert!(live.words.iter().any(|w| w.contains("cannot be read") && w.contains("A level to 0.6 (the program's own last line)") && w.contains("master level to 1 (neutral)")), "{:?}", live.words);
    h.run_to(&mut live, 4);
    // the program's own `deck A: level 0.6`, and the engine's own start for the master
    assert_eq!(h.knob("A.volume", 0.8), 0.6);
    assert_eq!(h.knob("master", 0.0), 1.0);
}

#[test]
fn stopping_the_program_puts_the_mixer_and_the_pump_back_at_once() {
    let (mut h, mut live) = playing();
    assert!(live.eval(SONG).ok);
    h.run_to(&mut live, 6);
    assert_eq!(h.knob("A.volume", 0.8), 0.7);
    let n = h.calls.len();
    live.stop_with(&mut h);
    assert_eq!((h.knob("A.volume", 0.8), h.knob("master", 1.0), h.knob("A.low", 0.5)), (0.65, 0.75, 0.5));
    let tail: Vec<&str> = h.calls[n..].iter().map(|c| c.1.as_str()).collect();
    assert_eq!(tail, ["trick", "mixer", "mixer", "mixer"], "{:?}", &h.calls[n..]);
    assert!(h.calls[n].2.contains(r#""action":"off""#) && h.calls[n].2.contains(r#""kind":"pump""#));
}

#[test]
fn the_end_of_the_song_puts_back_what_the_last_section_holds() {
    let (mut h, mut live) = playing();
    let src = "section one 4 bars {\n  deck A: kick x\n  duck B under A.kick 6db\n  mix { A level 0.3 }\n}\narrange one\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 3);
    assert_eq!(h.knob("A.volume", 0.8), 0.3);
    h.run_to(&mut live, 6);
    assert!(!live.is_running(), "the song ended");
    assert_eq!(h.knob("A.volume", 0.8), 0.65);
    assert_eq!(h.calls_of("trick").len(), 2, "on, and off at the end: {:?}", h.calls);
}

#[test]
fn a_looping_song_applies_and_puts_back_each_lap() {
    let (mut h, mut live) = playing();
    let src = "section one 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection two 4 bars {\n  deck A: kick x\n}\narrange one two\nloop\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 2);
    assert_eq!(h.knob("A.volume", 0.8), 0.3);
    h.run_to(&mut live, 5);
    assert_eq!(h.knob("A.volume", 0.8), 0.65);
    h.run_to(&mut live, 10);
    assert_eq!(h.knob("A.volume", 0.8), 0.3, "the second lap");
    let vols: Vec<String> = h.calls_of("mixer").into_iter().map(|c| c.1).collect();
    assert_eq!(vols, [r#"{"deck":"A","volume":0.3}"#, r#"{"deck":"A","volume":0.65}"#, r#"{"deck":"A","volume":0.3}"#], "set, put back, and set again on the second lap");
}

#[test]
fn outside_a_section_a_mix_just_sets_and_for_n_bars_puts_back() {
    let (mut h, mut live) = playing();
    // (a program with no sections counts its own bars: bar 0 is the one it was evaluated on, so `at bar N` is N - 1 bars after the net lands)
    assert!(live.eval("deck A: kick x\nat bar 2: mix { A level 0.4 }\nat bar 6 for 2 bars: mix { master level 0.2 }\n").ok);
    h.run_to(&mut live, 2);
    assert_eq!(h.knob("A.volume", 0.8), 0.4);
    assert!(h.reads.is_empty(), "a plain mix reads nothing: {:?}", h.reads);
    h.run_to(&mut live, 6);
    assert_eq!((h.knob("master", 1.0), h.reads.len()), (0.2, 1));
    h.run_to(&mut live, 8);
    assert_eq!(h.knob("master", 1.0), 0.75, "back after the two bars");
    assert_eq!(h.knob("A.volume", 0.8), 0.4, "and the plain one stays");
}

#[test]
fn a_duck_is_the_pump_and_is_held_to_the_end_of_its_section() {
    let (mut h, mut live) = playing();
    assert!(live.eval(SONG).ok);
    h.run_to(&mut live, 4);
    let t = h.calls_of("trick");
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].1, r#"{"action":"on","amount":0.499,"deck":"B","div":16,"kind":"pump","source":"A"}"#);
    assert_eq!(bar_of(t[0].0) - h.origin(), 4);
    h.run_to(&mut live, 11);
    assert_eq!(h.calls_of("trick").len(), 1);
    h.run_to(&mut live, 12);
    let t = h.calls_of("trick");
    assert_eq!(t.len(), 2);
    assert_eq!(t[1].1, r#"{"action":"off","deck":"B","kind":"pump"}"#);
    assert_eq!(bar_of(t[1].0) - h.origin(), 12);
}

#[test]
fn the_release_note_follows_the_deck_when_the_program_has_no_tempo() {
    let src = "section one 4 bars {\n  deck A: kick x\n  duck B under A.kick 6db\n}\narrange one\n";
    let (mut h, mut live) = playing();
    h.bpm = 90.0;
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 1);
    assert!(h.calls_of("trick")[0].1.contains(r#""div":24"#), "{:?}", h.calls_of("trick"));
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 1);
    assert!(h.calls_of("trick")[0].1.contains(r#""div":16"#), "{:?}", h.calls_of("trick"));
}

#[test]
fn a_duck_outside_a_section_is_held_until_the_program_stops() {
    let (mut h, mut live) = playing();
    assert!(live.eval("deck A: kick x ~ x ~\nduck B under A.kick 6db release=250ms\n").ok);
    h.run_to(&mut live, 20);
    assert_eq!(h.calls_of("trick").len(), 1, "on once, and it stays");
    assert!(h.calls_of("trick")[0].1.contains(r#""div":8"#), "250 ms is the 1/8 at 120 BPM (250 ms): {:?}", h.calls_of("trick"));
    live.stop_with(&mut h);
    let t = h.calls_of("trick");
    assert_eq!(t.len(), 2);
    assert!(t[1].1.contains(r#""action":"off""#));
    // the marks say it is holding something while it plays
}

#[test]
fn the_marks_light_the_line_and_say_it_is_holding() {
    let (mut h, mut live) = playing();
    assert!(live.eval(SONG).ok);
    h.run_to(&mut live, 4);
    h.run(&mut live, 1.0);
    let m = live.marks();
    let src = &m.source;
    let on_line = |frag: &str| m.controls.iter().find(|c| c.span.text(src).starts_with(frag)).unwrap_or_else(|| panic!("{frag} not in {:?}", m.controls));
    let duck = on_line("duck B under");
    let mix = on_line("mix {");
    assert!(duck.held && mix.held, "holding something changed while the section plays");
    assert!(mix.span.text(src).ends_with('}'), "the mix's span is the whole block: {:?}", mix.span.text(src));
    assert!(duck.age_beats < 4.0 && mix.age_beats < 4.0, "just fired");
    // the JSON of the marks has them with their line
    let j = m.to_json();
    let controls = j.get("controls").and_then(JsonValue::as_array).unwrap();
    assert!(controls.iter().any(|c| c.get("line").and_then(JsonValue::as_str).is_some_and(|l| l.contains("duck B under A.kick")) && c.get("held").and_then(JsonValue::as_bool) == Some(true)));
    // put back: no longer held
    h.run_to(&mut live, 13);
    let m = live.marks();
    assert!(m.controls.iter().filter(|c| c.span.text(&m.source).starts_with("duck") || c.span.text(&m.source).starts_with("mix")).all(|c| !c.held), "{:?}", m.controls);
}

#[test]
fn a_call_site_is_in_the_via_of_a_duck_made_by_a_def() {
    let (mut h, mut live) = playing();
    let src = "def pumped(under, src) {\n  duck $under under $src.kick 6db\n}\nsection one 4 bars {\n  deck A: kick x ~ x ~\n  pumped(B, A)\n}\narrange one\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 1);
    h.run(&mut live, 1.0);
    let m = live.marks();
    let duck = m.controls.iter().find(|c| c.key.contains("duck B under A.kick")).expect("the duck line fired");
    assert_eq!(duck.via.len(), 1);
    assert_eq!(duck.via[0].text(&m.source), "pumped(B, A)");
    assert_eq!(duck.span.text(&m.source), "duck $under under $src.kick 6db");
}

#[test]
fn an_edit_that_drops_a_held_mix_puts_the_mixer_back() {
    let (mut h, mut live) = playing();
    assert!(live.eval(SONG).ok);
    h.run_to(&mut live, 6);
    assert_eq!(h.knob("A.volume", 0.8), 0.7);
    // the mix is edited out while its section plays: the line is gone, so what it held is put back
    let edited = SONG.replace("    A level 0.7\n    master level 0.9\n    A eq low -3\n", "    A level 0.5\n");
    assert!(live.eval(&edited).ok);
    h.run(&mut live, 8.0);
    assert_eq!((h.knob("A.volume", 0.8), h.knob("master", 1.0), h.knob("A.low", 0.5)), (0.65, 0.75, 0.5));
    // the duck, untouched, goes on
    assert_eq!(h.calls_of("trick").len(), 1);
}

// ---- the mixer's own scale

#[test]
fn eq_in_db_is_the_mixers_knob() {
    let comp = check("mix { A eq low -24, A eq mid 0, A eq high 6, B eq low 3 }\n").compiled.unwrap();
    let Effect::Mix(m) = &comp.controls[0].effect else { panic!() };
    let knobs: Vec<f64> = m.items.iter().map(|i| i.knob).collect();
    assert_eq!(knobs, [0.0, 0.5, 1.0, 0.75]);
}

// ---- the example

#[test]
fn the_example_says_what_it_does() {
    let src = include_str!("../../../examples/btw/mix.btw");
    let comp = check(src).compiled.expect("the example compiles");
    let report = comp.mix_report();
    // the first drop (bars 8 to 24), the break (24 to 32), the second drop (32 to 48): each section's lines are its own control
    assert_eq!(report.len(), 5, "{report:#?}");
    assert!(report[0].contains("in drop") && report[0].contains("put back on bar 24") && report[0].contains("1/16 note at 124 BPM"), "{}", report[0]);
    assert!(report[1].contains("mixer {\"deck\":\"B\",\"low\":0.375}") && report[1].contains("put back on bar 24"), "{}", report[1]);
    assert!(report[2].starts_with("mix in break: B level 0.9, master level 0.8") && report[2].contains("put back on bar 32"), "{}", report[2]);
    assert!(report[3].contains("put back on bar 48") && report[4].contains("put back on bar 48"), "{report:#?}");
    // and played: the pump is on for a drop and off at its end, and the mixer ends where it was
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 9);
    assert_eq!((h.calls_of("trick").len(), h.knob("B.low", 0.5)), (1, 0.375));
    h.run_to(&mut live, 33);
    assert_eq!(h.calls_of("trick").len(), 3, "off at 24, on again at 32");
    h.run_to(&mut live, 50);
    assert!(!live.is_running());
    assert_eq!((h.knob("A.volume", 0.8), h.knob("B.low", 0.5), h.knob("master", 1.0)), (0.65, 0.5, 0.75));
    assert_eq!(h.calls_of("trick").len(), 4);
}

// ---- the registry

#[test]
fn a_registry_that_lacks_what_the_line_is_made_of_says_so() {
    use djlive::compile::check_with;
    use djlive::env::{ActionSpec, Env, NounSpec, ParamSpec, StaticRegistry};
    let p = |n: &str| ParamSpec { name: n.into(), types: vec![], options: vec![], required: false, doc: String::new(), ..Default::default() };
    let mut reg = StaticRegistry::default();
    reg.nouns.insert("mixer".into(), NounSpec { name: "mixer".into(), default_action: String::new(), actions: vec![], params: vec![p("deck"), p("volume"), p("master")] });
    reg.nouns.insert(
        "trick".into(),
        NounSpec { name: "trick".into(), default_action: "status".into(), actions: vec![ActionSpec { name: "on".into(), effect: "write".into(), params: vec![] }], params: vec![p("deck"), p("kind"), p("source"), p("amount")] },
    );
    let env = Env { registry: Some(&reg), ..Env::default() };
    // volume and master are there
    assert!(check_with("mix { A level 0.7, master level 0.9 }\n", &env).ok());
    // `low` is not
    let c = check_with("mix { A eq low -3 }\n", &env);
    assert!(!c.ok() && c.diagnostics[0].message.contains("`mixer` has no param `low`"), "{:?}", c.diagnostics);
    // the pump has no `div` and no `off` here
    let c = check_with("deck A: kick x\nduck B under A.kick 6db\n", &env);
    let msgs: Vec<&str> = c.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(msgs.iter().any(|m| m.contains("`trick` has no param `div`")) && msgs.iter().any(|m| m.contains("no verb `off`")), "{msgs:?}");
    // no `trick` noun at all: a did-you-mean
    let mut bare = StaticRegistry::default();
    bare.nouns.insert("tricks".into(), reg.nouns["trick"].clone());
    let env = Env { registry: Some(&bare), ..Env::default() };
    let c = check_with("deck A: kick x\nduck B under A.kick 6db\n", &env);
    assert!(c.diagnostics.iter().any(|d| d.message.contains("no `trick` noun") && d.help.as_deref() == Some("did you mean `tricks`?")), "{:?}", c.diagnostics);
}

// ---- the law: a snapshot is put back, whatever the song

/// (what the item says, the mixer knob it moves, the knob's value, the engine's starting position of it)
fn pool() -> Vec<(&'static str, &'static str, fn(f64) -> f64, f64)> {
    fn same(v: f64) -> f64 {
        v
    }
    fn eq(db: f64) -> f64 {
        ((if db < 0.0 { 0.5 + db / 48.0 } else { 0.5 + db / 12.0 }) * 1e6).round() / 1e6
    }
    vec![
        ("A level", "A.volume", same, 0.8),
        ("B level", "B.volume", same, 0.8),
        ("master level", "master", same, 1.0),
        ("A eq low", "A.low", eq, 0.5),
        ("A eq mid", "A.mid", eq, 0.5),
        ("B eq high", "B.high", eq, 0.5),
    ]
}

/// Random songs of two to six sections, each with a mix of some of the pool: at every bar the mixer is the one it was before
/// the song with the bar's section laid over it, and when the song is over it is as it was. A host that cannot read goes back
/// to the engine's starting position (the program has no mixer line of its own).
#[test]
fn a_snapshot_is_put_back_whatever_the_song() {
    for blind in [false, true] {
        for seed in 0..40u64 {
            let mut rng = Rng(0x51a9_5eed + seed * 7919);
            let pool = pool();
            let n = 2 + rng.below(5) as usize;
            let mut src = String::new();
            // (start, bars, the knobs this section sets)
            let mut secs: Vec<(i64, i64, Vec<(&str, f64)>)> = Vec::new();
            let mut start = 0;
            for k in 0..n {
                let bars = 2 + rng.below(5) as i64;
                let mut items = Vec::new();
                let mut set: Vec<(&str, f64)> = Vec::new();
                for (label, knob, f, _) in &pool {
                    if rng.below(3) == 0 {
                        let (text, v) = if label.contains("eq") {
                            let db = [-24.0, -12.0, -6.0, -3.0, 0.0, 3.0, 6.0][rng.below(7) as usize];
                            (format!("{label} {db}"), f(db))
                        } else {
                            let v = (1 + rng.below(19)) as f64 / 20.0;
                            (format!("{label} {v}"), v)
                        };
                        items.push(text);
                        set.push((knob, v));
                    }
                }
                src.push_str(&format!("section s{k} {bars} bars {{\n  deck A: kick x ~ x ~\n"));
                if !items.is_empty() {
                    if rng.below(2) == 0 {
                        src.push_str(&format!("  mix {{ {} }}\n", items.join(", ")));
                    } else {
                        src.push_str(&format!("  mix {{\n    {}\n  }}\n", items.join("\n    ")));
                    }
                }
                src.push_str("}\n");
                secs.push((start, bars, set));
                start += bars;
            }
            src.push_str(&format!("arrange {}\n", (0..n).map(|k| format!("s{k}")).collect::<Vec<_>>().join(" ")));
            let (mut h, mut live) = playing();
            h.blind = blind;
            // the mixer as someone left it: every knob of the pool somewhere the song does not put it
            let mut initial: BTreeMap<&str, f64> = BTreeMap::new();
            for (_, knob, _, _) in &pool {
                let v = (1 + rng.below(9)) as f64 / 10.0 + 0.003;
                h.mixer.insert((*knob).to_string(), v);
                initial.insert(knob, v);
            }
            let ev = live.eval(&src);
            assert!(ev.ok, "seed {seed}: {:?}\n{src}", ev.diagnostics);
            let neutral: BTreeMap<&str, f64> = pool.iter().map(|(_, k, _, d)| (*k, *d)).collect();
            let mut touched: Vec<&str> = Vec::new();
            for (start, bars, set) in &secs {
                for r in *start..*start + *bars {
                    h.run_to(&mut live, r);
                    for (_, knob, _, _) in &pool {
                        let want = match set.iter().find(|(k, _)| k == knob) {
                            Some((_, v)) => *v,
                            // not set here: as it was (a blind host: back to the engine's start once a section has touched it)
                            None if blind && touched.contains(knob) => neutral[knob],
                            None => initial[knob],
                        };
                        let got = h.mixer[*knob];
                        assert!((got - want).abs() < 1e-9, "seed {seed} blind {blind}: {knob} at song bar {r} is {got}, want {want}\n{src}");
                    }
                }
                touched.extend(set.iter().map(|(k, _)| *k));
            }
            h.run_to(&mut live, start + 3);
            assert!(!live.is_running(), "seed {seed}: the song ended");
            for (_, knob, _, _) in &pool {
                let want = if blind && touched.contains(knob) { neutral[knob] } else { initial[knob] };
                assert!((h.mixer[*knob] - want).abs() < 1e-9, "seed {seed} blind {blind}: {knob} is {}, want {want} at the end\n{src}", h.mixer[*knob]);
            }
        }
    }
}
