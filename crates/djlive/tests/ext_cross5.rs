//! Features that meet, fifth file: mix moves (F8, `duck B under A.kick 6db`, `mix { A level 0.7 }`) against arithmetic (F2),
//! sub-bar timing (F1), defs, repeats and imports, composition (F10, a section stacked with another) and the runtime's
//! clock. The tests the integrator writes when F8 landed on the branches that were built apart. (`ext_cross.rs` to
//! `ext_cross4.rs` are the first four files; each feature's own tests are in `ext_<feature>.rs`.)

mod common;

use std::collections::BTreeMap;

use common::{reformat, Rng};
use djlive::compile::{check_with, Compiled, Effect, Timing};
use djlive::dur::Dur;
use djengine::json::{JsonObject, JsonValue};
use djlive::env::{Env, StaticSnippets};
use djlive::runtime::{Grid, Live, LiveHost};
use djlive::{check, diag};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errs(src: &str) -> Vec<String> {
    let c = check(src);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.clone().unwrap_or_default())).collect()
}

fn warnings(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect()
}

/// The canonical text is a fixed point, and 40 random reformattings keep the CID.
fn stable(src: &str, seed: u64) -> Compiled {
    let c = ok(src);
    let again = ok(&c.text);
    assert_eq!(again.text, c.text, "the canonical text is not a fixed point:\n{}", c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
    c
}

/// (timing, hold in bars, what it does) of every duck and mix control.
fn moves(c: &Compiled) -> Vec<(Timing, Option<Dur>, String)> {
    c.controls
        .iter()
        .filter_map(|ctl| match &ctl.effect {
            Effect::Duck(d) => Some((ctl.timing, ctl.hold, d.text())),
            Effect::Mix(m) => Some((ctl.timing, ctl.hold, m.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>().join(", "))),
            _ => None,
        })
        .collect()
}

/// A store of saved snippets (each may import the ones before it).
fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let c = check_with(text, &env);
        let canon = c.compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}\n{}", diag::render_all(&c.diagnostics, text)));
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    snips
}

// ---- a fake host with a mixer you can read (as in ext_mix.rs)

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    loads: Vec<(String, i64)>,
    /// (beat when called, noun, params): every call but a read of the mixer
    calls: Vec<(f64, String, String)>,
    mixer: BTreeMap<String, f64>,
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

    /// Advance `beats` in quarter-beat steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn origin(&self) -> i64 {
        self.loads.iter().find(|l| l.0 == "A").map_or(0, |l| l.1)
    }

    /// Run to the start of the song's bar `rel` (just past its bar line).
    fn run_to(&mut self, live: &mut Live, rel: i64) {
        if self.loads.is_empty() {
            self.run(live, 0.25);
        }
        let want = (self.origin() + rel) as f64 * 4.0;
        if want > self.beat {
            self.run(live, want - self.beat);
        }
    }

    /// Run to a beat of the song (from its first bar line): `beat` is counted from the song's start.
    fn run_to_beat(&mut self, live: &mut Live, beat: f64) {
        if self.loads.is_empty() {
            self.run(live, 0.25);
        }
        let want = self.origin() as f64 * 4.0 + beat;
        if want > self.beat {
            self.run(live, want - self.beat);
        }
    }

    /// (song beat, params) of the calls of one noun.
    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        let o = self.origin() as f64 * 4.0;
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0 - o, c.2.clone())).collect()
    }
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }

    fn load_net(&mut self, deck: &str, _env: &JsonValue) -> Result<(), String> {
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        if noun == "mixer" {
            let o = params.as_object().cloned().unwrap_or_default();
            if o.iter().next().is_none() {
                return Ok(self.state());
            }
            let deck = o.get("deck").and_then(JsonValue::as_str).unwrap_or("").to_string();
            for (k, v) in o.iter() {
                let Some(v) = v.as_f64() else { continue };
                match k {
                    "volume" | "low" | "mid" | "high" => {
                        self.mixer.insert(format!("{deck}.{k}"), v);
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

fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    // the mixer as someone left it
    h.mixer.insert("A.volume".into(), 0.65);
    h.mixer.insert("master".into(), 0.75);
    (h, Live::new())
}

// ---- F8 x F2 x defs, repeats, imports

const PUMPED: &str = "def pumped(under, src, depth, k) {\n  duck $under under $src.kick $depth\n  mix { $under level 0.5 + $k / 10 }\n}\n";

#[test]
fn a_duck_and_a_mix_in_a_def_called_from_a_repeat_take_the_decks_and_the_arithmetic() {
    let src = format!("{PUMPED}deck A: kick x ~ x ~\ndeck B: bass c2\nrepeat 3 every 4 bars {{\n  pumped(B, A, 6db, $i)\n}}\n");
    let c = stable(&src, 0xf8_0001);
    let m = moves(&c);
    assert_eq!(m.len(), 6, "{m:#?}");
    let times: Vec<Timing> = m.iter().map(|x| x.0).collect();
    assert_eq!(times, [Timing::Now, Timing::Now, Timing::At(4), Timing::At(4), Timing::At(8), Timing::At(8)]);
    assert!(m.iter().step_by(2).all(|x| x.2 == "duck B under A.kick 6db attack=5ms release=120ms"), "{m:?}");
    assert_eq!(m.iter().skip(1).step_by(2).map(|x| x.2.as_str()).collect::<Vec<_>>(), ["B level 0.5", "B level 0.6", "B level 0.7"]);
    // the call site is in the `via` of each control, and the span of the line is the def's own text
    for ctl in c.controls.iter().filter(|c| matches!(c.effect, Effect::Duck(_) | Effect::Mix(_))) {
        assert!(ctl.via.iter().any(|v| v.text(&src).starts_with("pumped(B, A, 6db, $i)")), "{:?}", ctl.via.iter().map(|v| v.text(&src)).collect::<Vec<_>>());
        assert!(ctl.span.text(&src).starts_with("duck $under") || ctl.span.text(&src).starts_with("mix {"), "{:?}", ctl.span.text(&src));
    }
    // the report names each copy
    assert_eq!(c.mix_report().len(), 6);
}

#[test]
fn the_deck_can_be_a_parameter_of_a_def_and_the_guard_of_a_mix_line_is_kept() {
    // `$deck` where a deck goes, a `when` guard after a `mix { }` (it prints after the closing brace and reads back)
    let src = "def lowered(deck, other) {\n  mix { $deck level 0.4, $other level 0.9 } when $deck == A\n}\ndeck A: kick x\nrepeat 2 every 4 bars {\n  lowered(A, B)\n  lowered(B, A)\n}\n";
    let c = stable(src, 0xf8_0002);
    // only the copy where deck is A is kept (two repeats)
    assert_eq!(moves(&c).len(), 2, "{:?}", moves(&c));
    assert!(c.text.contains("  } when $deck==A\n"), "a def body prints as written, the guard after the brace: {}", c.text);
    // at the top level a guard is a constant, kept as written, after the brace
    let c = stable("at bar 2: mix { A level 0.5 } when 1 == 1\nat bar 3: mix {\n  master level 0.5\n} when 1 == 2\n", 0xf8_0003);
    assert!(c.text.contains("at bar 2: mix {\n  A level 0.5\n} when 1 == 1\n"), "{}", c.text);
    assert_eq!(moves(&c).len(), 1);
    // the same inside a section (a guard on a line with a time word)
    let c = stable("section s 4 bars {\n  deck A: kick x\n  at bar 1: mix { A level 0.2 } when 2 > 1\n}\narrange s\n", 0xf8_0004);
    assert!(c.text.contains("  at bar 1: mix {\n    A level 0.2\n  } when 2 > 1\n"), "{}", c.text);
}

#[test]
fn the_arithmetic_of_a_mix_item_is_one_program_with_its_value_and_a_fixed_point() {
    // the value a mix item comes to is what the canonical text says: `1/2` would read back as a division, so it is folded
    let items = [("A level (1 + 1) / 4", "A level 0.5"), ("A level 1 / 2", "A level 0.5"), ("master level 1 / 4", "master level 0.25"), ("master level 333333 / 1000000", "master level 0.333333"), ("A eq low 0 - 3", "A eq low -3"), ("B eq mid (1 + 2) * -1", "B eq mid -3"), ("B level min(0.9, 1 / 2 + 0.1)", "B level 0.6"), ("A level 50%", "A level 0.5")];
    for (n, (arith, lit)) in items.iter().enumerate() {
        let a = stable(&format!("mix {{ {arith} }}\n"), 0xf8_0010 + n as u64);
        let b = ok(&format!("mix {{ {lit} }}\n"));
        assert_eq!(a.cid, b.cid, "{arith} is {lit}");
        assert_eq!(a.text, b.text);
        assert!(!a.text.contains('/') && !a.text.contains('('), "{}", a.text);
    }
    // in a section, with a time word, in a repeat that computes it, and the canonical text read again
    let src = "section s 8 bars {\n  deck A: kick x\n  at bar 2: mix { A level (1 + 1) / 4, A eq low 0 - 3 }\n}\nrepeat 2 every 4 bars {\n  at bar 1: mix { master level 1 / 2 + $i / 10 }\n}\narrange s\n";
    let c = stable(src, 0xf8_0020);
    assert_eq!(moves(&c).iter().map(|x| x.2.clone()).collect::<Vec<_>>(), ["master level 0.5", "master level 0.6", "A level 0.5, A eq low -3"]);
    // a number that is no decimal of at most six places is an error at the item, as a chance and a weight are (L-08): it is never
    // rounded, so `1/3` and `0.333333` are not one program
    let e = errs("mix { A level 1 / 3 }\n");
    assert!(e.iter().any(|m| m.contains("a mix value has at most six decimals")), "{e:?}");
    let e = errs("mix { master level 1 / 3000000 }\n");
    assert!(e.iter().any(|m| m.contains("a mix value has at most six decimals")), "{e:?}");
    // a number that is no number is an error at the item
    let e = errs("mix { A level 1 / 0 }\n");
    assert!(e.iter().any(|m| m.contains("division by zero") || m.contains("divide")), "{e:?}");
    let e = errs("mix { A level 1 + 1 }\n");
    assert!(e.iter().any(|m| m.contains("a level is 0 to 1")), "{e:?}");
}

#[test]
fn a_duck_depth_is_a_literal_and_the_error_says_where_arithmetic_goes() {
    let src = format!("{PUMPED}deck A: kick x\nrepeat 2 every 4 bars {{\n  pumped(B, A, 6 / 2 + $i, $i)\n}}\n");
    let e = errs(&src);
    assert!(e.iter().any(|m| m.contains("found `(`") && m.contains("arithmetic is for the numbers of a `mix`")), "{e:?}");
    // a unit spelled in the argument is the way, and `-6db` is not a depth
    ok(&format!("{PUMPED}deck A: kick x\npumped(B, A, 3db, 1)\n"));
    assert!(errs("deck A: kick x\nduck B under A.kick -6db\n").iter().any(|m| m.contains("no sign")));
}

#[test]
fn a_duck_and_a_mix_come_through_an_import_and_the_marks_stay_in_the_text() {
    let snips = snippets(&[("pumps", PUMPED)]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"pumps\"\ntempo 124\nsection drop 8 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  pumped(B, A, 6db, 3)\n}\narrange drop\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", diag::render_all(&c.diagnostics, src));
    let imported = c.compiled.unwrap();
    let by_hand = ok("tempo 124\nsection drop 8 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2\n  duck B under A.kick 6db\n  mix { B level 0.8 }\n}\narrange drop\n");
    assert_eq!(moves(&imported), moves(&by_hand));
    assert_eq!(moves(&imported).iter().map(|x| x.1).collect::<Vec<_>>(), [Some(Dur::bars(8)), Some(Dur::bars(8))]);
    assert_eq!(imported.mix_report().len(), 2);
    // a control made by an imported def points at text that is there
    for ctl in &imported.controls {
        assert!(ctl.span.end <= src.len() && ctl.span.start <= ctl.span.end, "{:?}", ctl.span);
        assert!(ctl.via.iter().all(|v| v.end <= src.len()), "{:?}", ctl.via);
    }
    // the same program run through the 40 reformattings of the importing text
    let mut rng = Rng(0xf8_0030);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check_with(&text, &env);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, imported.cid, "round {round} forked:\n{text}");
    }
}

// ---- F8 x F1: a mix or a duck between bar lines

#[test]
fn a_mix_between_bar_lines_in_a_section_is_put_back_where_the_section_ends() {
    let src = "tempo 120\nsection one 4 bars {\n  deck A: kick x\n  deck B: bass c2\n  at bar 1 beat 2: mix { A level 0.5 }\n  at bar 2 beat 1 for 2 beats: mix { master level 0.2 }\n  at bar 2 beat 2: duck B under A.kick 6db\n}\nsection two 4 bars {\n  deck A: kick x\n}\narrange one two\n";
    let c = stable(src, 0xf8_0040);
    let m = moves(&c);
    assert_eq!(m[0].0, Timing::AtPos(djlive::rat::Rat::new(3, 2).unwrap()));
    // to the end of the section (bar 4): from bar 1 beat 2 that is 2 bars and 2 beats; the `for 2 beats` is two beats
    assert_eq!(m[0].1.map(|d| d.canonical()), Some("10 beats".to_string()));
    assert_eq!(m[1].1.map(|d| d.canonical()), Some("2 beats".to_string()));
    let report = c.mix_report().join("\n");
    assert!(report.contains("put back on bar 4") && report.contains("put back on bar 2 beat 3"), "{report}");
    // played: set between the bar lines, put back exactly 2 beats after, the rest on the bar line the section ends on
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to_beat(&mut live, 17.0);
    let mixer: Vec<(f64, String)> = h.calls_of("mixer");
    assert_eq!(
        mixer,
        [(6.0, r#"{"deck":"A","volume":0.5}"#.to_string()), (9.0, r#"{"master":0.2}"#.to_string()), (11.0, r#"{"master":0.75}"#.to_string()), (16.0, r#"{"deck":"A","volume":0.65}"#.to_string())]
    );
    let trick: Vec<f64> = h.calls_of("trick").iter().map(|c| c.0).collect();
    assert_eq!(trick, [10.0, 16.0], "the pump on at bar 2 beat 2 and off where the section ends");
    assert_eq!((h.knob("A.volume", 0.8), h.knob("master", 1.0)), (0.65, 0.75));
}

#[test]
fn a_mix_between_bar_lines_outside_a_section_and_every_two_beats() {
    let (mut h, mut live) = playing();
    assert!(live.eval("deck A: kick x\nat bar 1 beat 2 for 1 beat: mix { A level 0.3 }\nevery 2 beats from bar 2: mix { master level 0.4 }\n").ok);
    h.run_to_beat(&mut live, 12.0);
    let a: Vec<(f64, String)> = h.calls_of("mixer").into_iter().filter(|c| c.1.contains("volume")).collect();
    assert_eq!(a.len(), 2, "{a:?}");
    assert_eq!(a[1].0 - a[0].0, 1.0, "put back after exactly one beat: {a:?}");
    assert_eq!(h.knob("A.volume", 0.8), 0.65);
    let m: Vec<f64> = h.calls_of("mixer").into_iter().filter(|c| c.1.contains("master")).map(|c| c.0).collect();
    assert!(m.len() >= 3 && m.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 1e-9), "{m:?}");
    // a time word that repeats cannot be put back at the end of a section
    let e = errs("section s 4 bars {\n  deck A: kick x\n  every 2 beats: mix { A level 0.3 }\n}\narrange s\n");
    assert!(e.iter().any(|m| m.contains("put back when the section ends: this time word repeats it")), "{e:?}");
    // and one outside the section it is in
    let e = errs("section s 4 bars {\n  deck A: kick x\n  at bar 5 beat 2: mix { A level 0.3 }\n}\narrange s\n");
    assert!(e.iter().any(|m| m.contains("outside section `s`") && m.contains("bar 5 beat 2")), "{e:?}");
}

// ---- F8 x F10: a section stacked with another

#[test]
fn a_mix_in_a_stacked_section_is_put_back_where_its_own_section_ends() {
    let src = "tempo 120\nsection intro 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection verse 8 bars {\n  deck B: bass c2\n  mix { B level 0.2 }\n  duck B under A.kick 6db\n}\nsection outro 4 bars {\n  deck A: kick x\n}\narrange then(stack(intro, verse), outro)\n";
    let c = stable(src, 0xf8_0050);
    // the stack is two stretches (intro+verse, then verse), the lines are in the first only: one control a line
    assert_eq!(c.segments.iter().map(|s| (s.name.as_str(), s.start, s.bars)).collect::<Vec<_>>(), [("intro+verse", 0, 4), ("verse", 4, 4), ("outro", 8, 4)]);
    let m = moves(&c);
    assert_eq!(m.len(), 3, "{m:?}");
    // each is held to the end of its own section: the intro's to bar 4, the verse's to bar 8
    let hold = |text: &str| m.iter().find(|x| x.2.starts_with(text)).and_then(|x| x.1).map(|d| d.canonical());
    assert_eq!(hold("A level"), Some("4 bars".to_string()));
    assert_eq!(hold("B level"), Some("8 bars".to_string()));
    assert_eq!(hold("duck"), Some("8 bars".to_string()));
    let report = c.mix_report().join("\n");
    assert!(report.contains("mix in intro: A level 0.3") && report.contains("put back on bar 4"), "{report}");
    assert!(report.contains("mix in verse: B level 0.2") && report.contains("put back on bar 8"), "{report}");
    // played
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 13);
    let at = |knob: &str| -> Vec<(f64, String)> { h.calls_of("mixer").into_iter().filter(|c| c.1.contains(knob)).collect() };
    assert_eq!(at(r#""deck":"A""#), [(0.0, r#"{"deck":"A","volume":0.3}"#.to_string()), (16.0, r#"{"deck":"A","volume":0.65}"#.to_string())], "the intro's mix goes back where the intro ends, bar 4");
    assert_eq!(at(r#""deck":"B""#), [(0.0, r#"{"deck":"B","volume":0.2}"#.to_string()), (32.0, r#"{"deck":"B","volume":0.8}"#.to_string())], "the verse's goes on to bar 8");
    assert_eq!(h.calls_of("trick").iter().map(|c| c.0).collect::<Vec<_>>(), [0.0, 32.0], "and so does the pump");
}

#[test]
fn a_section_played_twice_in_a_row_is_put_back_each_time() {
    // the same section placed twice: two placements of one name, not one section of 8 bars
    let src = "section one 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection two 4 bars {\n  deck A: kick x\n}\narrange then(one, one, two)\n";
    let c = ok(src);
    assert_eq!(moves(&c).iter().map(|x| (x.0, x.1.map(|d| d.canonical()))).collect::<Vec<_>>(), [(Timing::At(0), Some("4 bars".to_string())), (Timing::At(4), Some("4 bars".to_string()))]);
    let (mut h, mut live) = playing();
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 13);
    let beats: Vec<(f64, bool)> = h.calls_of("mixer").iter().map(|c| (c.0, c.1.contains("0.3"))).collect();
    assert_eq!(beats, [(0.0, true), (16.0, false), (16.0, true), (32.0, false)], "{:?}", h.calls_of("mixer"));
    assert_eq!(h.knob("A.volume", 0.8), 0.65);
}

#[test]
fn a_mix_at_a_later_bar_of_a_stacked_section_and_three_stretches() {
    // three sections of 4, 8 and 12 bars stacked: stretches of 4, 4 and 4; the line of the longest is held to bar 12
    let src = "tempo 120\nsection one 4 bars {\n  deck A: kick x\n  at bar 1: mix { A level 0.3 }\n}\nsection two 8 bars {\n  deck B: bass c2\n  at bar 5: mix { B level 0.2 }\n}\nsection three 12 bars {\n  deck A: hihat x\n  at bar 2: mix { master level 0.5 }\n}\narrange stack(one, two, three)\n";
    let c = ok(src);
    let m = moves(&c);
    let hold = |text: &str| m.iter().find(|x| x.2.starts_with(text)).map(|x| (x.0, x.1.map(|d| d.canonical())));
    assert_eq!(hold("A level"), Some((Timing::At(1), Some("3 bars".to_string()))));
    assert_eq!(hold("B level"), Some((Timing::At(5), Some("3 bars".to_string()))));
    assert_eq!(hold("master"), Some((Timing::At(2), Some("10 bars".to_string()))));
    assert_eq!(c.segments.len(), 3);
    assert_eq!(m.len(), 3, "no copy of a line carried into a later stretch is a control of its own: {m:?}");
}

#[test]
fn two_stacked_sections_cannot_both_hold_one_knob_and_one_after_the_other_can() {
    let both = "section intro 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection verse 8 bars {\n  deck B: bass c2\n  mix { A level 0.9, B level 0.2 }\n}\narrange stack(intro, verse)\n";
    let e = errs(both);
    assert!(e.iter().any(|m| m.contains("`A level` is held by two mixes at once") && m.contains("bar 0 to bar 8") && m.contains("line 3") && m.contains("bar 0 to bar 4")), "{e:?}");
    // different knobs of the same deck are fine; so are the same knob one after the other
    ok(&both.replace("mix { A level 0.9, B level 0.2 }", "mix { A eq low -3, B level 0.2 }"));
    ok(&both.replace("stack(intro, verse)", "then(intro, verse)"));
    // and the same inside one section, with `for`
    let e = errs("section s 8 bars {\n  deck A: kick x\n  at bar 0 for 4 bars: mix { A level 0.3 }\n  at bar 2 for 4 bars: mix { A level 0.5 }\n}\narrange s\n");
    assert!(e.iter().any(|m| m.contains("held by two mixes at once")), "{e:?}");
    ok("section s 8 bars {\n  deck A: kick x\n  at bar 0 for 4 bars: mix { A level 0.3 }\n  at bar 4 for 4 bars: mix { A level 0.5 }\n}\narrange s\n");
}

#[test]
fn a_ducks_warnings_are_not_doubled_by_the_stretches_of_a_stack() {
    // a duck in the longer section of a stack is one duck, not one for each stretch
    let src = "section intro 4 bars {\n  deck A: kick x\n}\nsection verse 8 bars {\n  deck B: bass c2\n  duck B under A.kick 6db\n}\narrange stack(intro, verse)\n";
    let w = warnings(src);
    assert!(!w.iter().any(|m| m.contains("already ducked")), "{w:?}");
    assert_eq!(moves(&ok(src)).len(), 1);
}

// ---- the runtime's order on a bar line

#[test]
fn a_line_that_fires_where_a_section_mix_is_put_back_has_the_last_word() {
    let (mut h, mut live) = playing();
    let src = "tempo 120\nsection one 4 bars {\n  deck A: kick x\n  mix { A level 0.3 }\n}\nsection two 4 bars {\n  deck A: kick x\n}\narrange one two\nat bar 4: mix { A level 0.9 }\n";
    assert!(live.eval(src).ok);
    h.run_to(&mut live, 6);
    // the section's is put back first, then the line that said bar 4 sets: the knob ends where the program said
    assert_eq!(h.calls_of("mixer").iter().map(|c| (c.0, c.1.clone())).collect::<Vec<_>>(), [(0.0, r#"{"deck":"A","volume":0.3}"#.to_string()), (16.0, r#"{"deck":"A","volume":0.65}"#.to_string()), (16.0, r#"{"deck":"A","volume":0.9}"#.to_string())]);
    assert_eq!(h.knob("A.volume", 0.8), 0.9);
}
