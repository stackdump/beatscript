//! Features that meet, sixth file: sing (F9, `sing "lyrics" on melody voice=alto`) against defs, repeats, imports, wired
//! ports and arithmetic (F2, F11), sections that are stacked or chained (F10), degrees and numerals (F5), chance and
//! polymeter (F6, F7), sub-bar timing (F1), the editor's tables and the runtime's log. The tests the integrator writes
//! when F9 landed on the branches that were built apart. (`ext_cross.rs` to `ext_cross5.rs` are the first five files;
//! each feature's own tests are in `ext_<feature>.rs`.)

mod common;

use common::{reformat, Rng};
use djengine::json::JsonValue;
use djlive::compile::{check_with, Compiled, Effect, Timing};
use djlive::diag;
use djlive::env::{Env, StaticSnippets, StaticVoices};
use djlive::runtime::{Grid, Live, LiveHost};

fn absent() -> StaticVoices {
    StaticVoices::default()
}

fn compile(src: &str) -> Compiled {
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let c = check_with(src, &env);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn compile_in(src: &str, env: &Env) -> Compiled {
    let c = check_with(src, env);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errors(src: &str) -> Vec<(usize, usize, String)> {
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let c = check_with(src, &env);
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone())).collect()
}

/// The `create` call of control `i`.
fn create(c: &Compiled, i: usize) -> JsonValue {
    let Effect::Calls(cs) = &c.controls[i].effect else { panic!("control {i} is not a call") };
    assert_eq!((cs[0].noun.as_str(), cs[1].noun.as_str()), ("vocal", "vocal"));
    cs[0].params.clone()
}

fn num(v: &JsonValue, k: &str) -> f64 {
    v.get(k).and_then(JsonValue::as_f64).unwrap_or_else(|| panic!("no {k} in {}", v.canonical_js()))
}

fn text(v: &JsonValue, k: &str) -> String {
    v.get(k).and_then(JsonValue::as_str).unwrap_or_else(|| panic!("no {k} in {}", v.canonical_js())).to_string()
}

/// (start in beats, midi) of every sung note.
fn notes(v: &JsonValue) -> Vec<(f64, i64)> {
    v.get("notes").and_then(JsonValue::as_array).expect("notes").iter().map(|n| (n.get("start").and_then(JsonValue::as_f64).unwrap(), n.get("midi").and_then(JsonValue::as_f64).unwrap() as i64)).collect()
}

fn midis(v: &JsonValue) -> Vec<i64> {
    notes(v).into_iter().map(|n| n.1).collect()
}

/// The canonical text is a fixed point, and 40 random reformattings keep the CID.
fn stable(src: &str, env: &Env, seed: u64) -> Compiled {
    let c = compile_in(src, env);
    let again = compile_in(&c.text, env);
    assert_eq!(again.text, c.text, "the canonical text is not a fixed point:\n{}", c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check_with(&text, env);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
    c
}

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

// ---- F9 against defs, repeats, imports and arithmetic (F2)

const HOOK: &str = "def hook(dk, words, v, n) {\n  deck $dk: melody c4 e4 g4 e4\n  deck $dk: sing $words on melody voice=$v bars=$n\n}\n";
/// The same, with the part written once outside it, so that a repeat may call it.
const SING_ONLY: &str = "def sung(dk, words, v, n) {\n  deck $dk: sing $words on melody voice=$v bars=$n\n}\n";

#[test]
fn a_def_passes_the_words_the_voice_and_the_bars_into_a_sing_line() {
    let src = format!("{HOOK}hook(B, \"robots on the road\", robot, 2)\n");
    let c = stable(&src, &Env { voices: Some(&absent()), ..Env::default() }, 0xf9_0001);
    assert_eq!(c.controls.len(), 1);
    let cr = create(&c, 0);
    assert_eq!((text(&cr, "preset").as_str(), num(&cr, "bars")), ("robot", 2.0));
    assert_eq!(text(&cr, "lyrics"), "robots on the road robots on");
    // the line is the def's own text, the call site is where it is lit from
    assert_eq!(c.controls[0].span.text(&src), "deck $dk: sing $words on melody voice=$v bars=$n");
    assert_eq!(c.controls[0].via.iter().map(|s| s.text(&src)).collect::<Vec<_>>(), ["hook(B, \"robots on the road\", robot, 2)"]);
    // the same line written out is the same sound
    let by_hand = compile("deck B: melody c4 e4 g4 e4\ndeck B: sing \"robots on the road\" on melody voice=robot bars=2\n");
    assert_eq!(create(&by_hand, 0), cr);
}

#[test]
fn arithmetic_gives_the_bars_of_a_sing_line_in_a_repeat_and_in_a_def_argument() {
    // `$i` counts from 0: bars 1, 2, 3 in four-bar steps
    let src = "deck B: melody c4 e4 g4 e4\nrepeat 3 every 4 bars {\n  deck B: sing \"la\" on melody voice=robot bars=1 + $i\n}\n";
    let c = stable(src, &Env { voices: Some(&absent()), ..Env::default() }, 0xf9_0002);
    assert_eq!(c.controls.len(), 3);
    assert_eq!(c.controls.iter().map(|k| k.timing).collect::<Vec<_>>(), [Timing::Now, Timing::At(4), Timing::At(8)]);
    assert_eq!((0..3).map(|i| num(&create(&c, i), "bars")).collect::<Vec<_>>(), [1.0, 2.0, 3.0]);
    // the canonical form folds the arithmetic to the whole number it comes to
    for want in ["bars=1 ", "bars=2 ", "bars=3 "] {
        assert!(c.controls.iter().any(|k| k.key.contains(want) || k.key.contains(&want.trim_end().to_string())), "{want}: {:?}", c.controls.iter().map(|k| &k.key).collect::<Vec<_>>());
    }
    // an argument that is arithmetic is put in place wrapped in parentheses, and reads as one
    let src = format!("{SING_ONLY}deck B: melody c4 e4 g4 e4\nsung(B, \"la\", robot, 1 + 2 * 2)\nrepeat 2 every 8 bars {{\n  sung(B, \"hey\", robot, 2 * ($i + 1))\n}}\n");
    let c = compile(&src);
    let bars: Vec<f64> = (0..c.controls.len()).map(|i| num(&create(&c, i), "bars")).collect();
    assert_eq!(bars, [5.0, 2.0, 4.0]);
    // not a whole number: said where it is, also when only a later copy of the repeat comes to it (copy 1 is 1, copy 2 is 3/2)
    let e = errors("deck B: melody c4 e4 g4 e4\nrepeat 2 every 4 bars {\n  deck B: sing \"la\" on melody voice=robot bars=$i / 2 + 1\n}\n");
    assert!(e.iter().any(|(l, _, m)| *l == 3 && m.contains("whole number")), "{e:?}");
    // and an out of range one is the same error as a literal
    let e = errors("deck B: melody c4 e4 g4 e4\ndeck B: sing \"la\" on melody voice=robot bars=40 - 2\n");
    assert!(e.iter().any(|(_, _, m)| m.contains("1 to 32")), "{e:?}");
}

#[test]
fn a_when_guard_after_a_sing_line_picks_the_copies_of_a_repeat_that_sing() {
    let src = "deck B: melody c4 e4 g4 e4\nrepeat 4 every 4 bars {\n  deck B: sing \"la\" on melody voice=robot when $i % 2 == 1\n}\n";
    let c = stable(src, &Env { voices: Some(&absent()), ..Env::default() }, 0xf9_0003);
    assert_eq!(c.controls.iter().map(|k| k.timing).collect::<Vec<_>>(), [Timing::At(4), Timing::At(12)]);
}

#[test]
fn a_sing_line_comes_through_an_import_with_its_string_and_its_voice() {
    let snips = snippets(&[("hooks", HOOK)]);
    let env = Env { snippets: Some(&snips), voices: Some(&absent()), ..Env::default() };
    let src = "import \"hooks\"\ntempo 120\nsection verse 4 bars {\n  hook(B, \"hello again\", robot-choir, 2)\n}\narrange verse\n";
    let c = compile_in(src, &env);
    assert_eq!(c.controls.len(), 1);
    let cr = create(&c, 0);
    assert_eq!((text(&cr, "preset").as_str(), num(&cr, "bars"), text(&cr, "lyrics").as_str()), ("robot-choir", 2.0, "hello again hello again"));
    // the control points at text that is there (the call in the importing text), and the importing text reads in 40 ways
    assert!(c.controls[0].span.end <= src.len() && c.controls[0].via.iter().all(|v| v.end <= src.len() && !v.is_empty()), "{:?} {:?}", c.controls[0].span, c.controls[0].via);
    let mut rng = Rng(0xf9_0004);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check_with(&text, &env);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
    // the same line from an unknown voice is the voice's error, with the nearest names, where the `voice=` value is in the text
    let src = "import \"hooks\"\nhook(B, \"hi\", robott, 1)\n";
    let v = absent();
    let env = Env { snippets: Some(&snips), voices: Some(&v), ..Env::default() };
    let r = check_with(src, &env);
    assert!(!r.ok());
    let d = r.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.message.contains("is not a voice") && d.help.as_deref().is_some_and(|h| h.contains("robot")), "{d:?}");
}

// ---- F9 against wired ports (F11)

#[test]
fn a_wired_melody_is_sung_on_where_it_was_laid() {
    let src = "\
def riff() out: melody {
  emit c4 e4 g4 e4
}
def lay() in: melody {
  deck B: melody $in
  deck B: sing \"la la la\" on melody voice=$v
}
wire riff.out -> lay.in
lay()
";
    // `$v` is no parameter of `lay`: it is the program's problem, said where it stands
    let e = errors(src);
    assert!(e.iter().any(|(l, _, _)| *l == 6), "{e:?}");
    let src = src.replace(" voice=$v", " voice=robot");
    let c = compile(&src);
    assert_eq!(c.controls.len(), 1);
    assert_eq!(midis(&create(&c, 0)), [60, 64, 67, 64]);
    stable(&src, &Env { voices: Some(&absent()), ..Env::default() }, 0xf9_0005);
}

// ---- F9 against stacked and chained sections (F10)

const SECS: &str = "tempo 120\nkey am\nsection intro 4 bars {\n  deck A: kick x ~ x ~\n}\nsection verse 8 bars {\n  deck B: melody c4 e4 g4 e4\n  deck B: sing \"robots on the road\" on melody voice=robot\n}\nsection extra 4 bars {\n  deck A: hihat x*4\n}\n";

#[test]
fn a_sing_line_in_a_section_that_is_chained_fires_where_its_section_starts() {
    let c = compile(&format!("{SECS}arrange then(intro, verse)\n"));
    assert_eq!(c.controls.len(), 1);
    assert_eq!(c.controls[0].timing, Timing::At(4));
    let cr = create(&c, 0);
    assert_eq!(num(&cr, "bars"), 8.0);
    assert_eq!(notes(&cr).len(), 32);
}

#[test]
fn a_sing_line_in_a_section_that_is_stacked_fires_once_over_the_stretch_that_has_its_part() {
    // verse (8 bars) stacked with extra (4 bars): two stretches; the song sounds the verse in both and its line was said in the first
    let c = compile(&format!("{SECS}arrange stack(verse, extra)\n"));
    assert_eq!(c.controls.len(), 1, "{:?}", c.controls.iter().map(|k| (&k.key, k.timing)).collect::<Vec<_>>());
    assert_eq!(c.controls[0].timing, Timing::At(0));
    // the line sings over the section: all eight bars, not only the four of the first stretch
    let cr = create(&c, 0);
    assert_eq!(num(&cr, "bars"), 8.0);
    assert_eq!(notes(&cr).len(), 32);
    // a section that is alone in its stretch and one that is stacked sing the same line
    let alone = compile(&format!("{SECS}arrange verse\n"));
    assert_eq!(create(&alone, 0), cr);
    // the other order, and with the other section the longer: the line is the verse's and fires at the verse
    let c = compile(&format!("{SECS}arrange stack(extra, verse)\n"));
    assert_eq!(c.controls.len(), 1);
    assert_eq!(create(&c, 0), cr);
    let c = compile(&format!("{SECS}arrange then(intro, stack(extra, verse))\n"));
    assert_eq!(c.controls.len(), 1);
    assert_eq!(c.controls[0].timing, Timing::At(4));
    // two sections that both sing, stacked, are two lines, each its own
    let two = SECS.replace("deck A: hihat x*4\n", "deck B: bass c2 ~ e2 ~\n  deck B: sing \"low low\" on bass voice=robot\n");
    let c = compile(&format!("{two}arrange stack(verse, extra)\n"));
    assert_eq!(c.controls.len(), 2);
}

#[test]
fn a_section_stacked_with_a_shorter_one_is_sung_on_the_notes_it_plays_in_every_stretch() {
    // chance is a function of the text and the bar: the verse's bars 5 to 8, played in the stretch of the stack that has no
    // `extra`, are the bars 5 to 8 of the verse alone, and so are the sung notes (not the first four bars again)
    let secs = "tempo 120\nsection verse 8 bars {\n  deck B: melody c4?0.5 e4?0.5 g4?0.5 e4?0.5 c5?0.5 e5?0.5 g5?0.5 e5?0.5\n  deck B: sing \"la la la la la la la la la la la la la la la la\" on melody voice=robot\n}\nsection extra 4 bars {\n  deck A: hihat x*4\n}\n";
    let alone = compile(&format!("{secs}arrange verse\n"));
    let stacked = compile(&format!("{secs}arrange stack(verse, extra)\n"));
    assert_eq!(stacked.controls.len(), 1);
    let (a, b) = (create(&alone, 0), create(&stacked, 0));
    assert_eq!(midis(&a), midis(&b));
    assert_eq!(notes(&a), notes(&b));
    let (first, second): (Vec<_>, Vec<_>) = notes(&a).into_iter().partition(|n| n.0 < 16.0);
    assert!(!first.is_empty() && !second.is_empty());
    assert_ne!(first.iter().map(|n| n.1).collect::<Vec<_>>(), second.iter().map(|n| n.1).collect::<Vec<_>>(), "the two halves of the verse differ, or this test shows nothing");
    // `bars=` takes the first of them, in any stretch
    let two = compile(&format!("{}arrange stack(verse, extra)\n", secs.replace("on melody voice=robot", "on melody voice=robot bars=6")));
    assert_eq!(num(&create(&two, 0), "bars"), 6.0);
    assert!(notes(&create(&two, 0)).iter().all(|n| n.0 < 24.0));
}

#[test]
fn a_sing_line_follows_the_stack_it_is_sung_in_through_the_canonical_form_and_the_reformattings() {
    let src = format!("{SECS}arrange stack(verse, extra) then(intro, verse)\n");
    let c = stable(&src, &Env { voices: Some(&absent()), ..Env::default() }, 0xf9_0006);
    // the verse is sung where it is placed, as many times as it is placed
    assert_eq!(c.controls.iter().map(|k| k.timing).collect::<Vec<_>>(), [Timing::At(0), Timing::At(12)]);
}

// ---- F9 against degrees and numerals (F5), polymeter (F6) and chance (F7): it reads the notes the part compiled to

#[test]
fn a_part_written_in_degrees_numerals_chance_or_polymeter_is_sung_on_the_notes_it_plays() {
    let sing = "deck B: sing \"la la la la\" on melody voice=robot bars=1\n";
    // degrees of the key: a minor, so 1 3 5 3 is a e g e (the part is written with deg( ) and its octave)
    let degs = compile(&format!("key am\ndeck B: melody deg(1 3 5 3)\n{sing}"));
    let notes_by_hand = compile(&format!("key am\ndeck B: melody a4 c5 e5 c5\n{sing}"));
    assert_eq!(midis(&create(&degs, 0)), midis(&create(&notes_by_hand, 0)));
    assert_eq!(midis(&create(&degs, 0)).len(), 4);
    // a section's own key re-keys its degrees, and the line follows
    let sec = |key: &str| format!("section s 2 bars {{\n  key {key}\n  deck B: melody deg(1 3 5 3)\n  deck B: sing \"la la\" on melody voice=robot bars=1\n}}\narrange s\n");
    assert_ne!(midis(&create(&compile(&sec("cm")), 0)), midis(&create(&compile(&sec("dm")), 0)));
    // a Roman numeral bass
    let bass = compile("key am\ndeck B: bass i VI III VII\ndeck B: sing \"low low\" on bass voice=robot bars=4\n");
    assert!(!midis(&create(&bass, 0)).is_empty());
    // a chord's top note: i is a minor, the highest of a c e a is the sung one
    let one = compile("key am\ndeck B: harmony i\ndeck B: sing \"ah\" on harmony voice=robot bars=1\n");
    assert_eq!(midis(&create(&one, 0)).len(), 1);
    // polymeter: 12 sixteenths in a 16 bar come round after three bars, and the line covers those it is told to
    let poly = compile("deck B: melody/12 c4 e4 g4 ~\ndeck B: sing \"la la la\" on melody voice=robot bars=3\n");
    assert_eq!(num(&create(&poly, 0), "bars"), 3.0);
    assert_eq!(notes(&create(&poly, 0)).len(), 12);
    // chance: a function of the text and the bar, so the sung notes are the same ones every time
    let src = "deck B: melody c4?0.5 e4 g4?0.5 e4\ndeck B: sing \"la la la la\" on melody voice=robot bars=4\n";
    let a = compile(src);
    assert_eq!(create(&a, 0), create(&compile(src), 0));
    assert!(notes(&create(&a, 0)).len() <= 16 && notes(&create(&a, 0)).len() >= 8);
}

// ---- F9 against sub-bar timing (F1) and the runtime (the log says the notice where the line fired)

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (beat, noun, params)
    calls: Vec<(f64, String, JsonValue)>,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[usize::from(deck == "B")]
    }
    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[usize::from(deck == "B")] = on;
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.clone()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    fn run(&mut self, live: &mut Live, beats: f64) {
        for _ in 0..(beats * 4.0).round() as usize {
            self.beat += 0.25;
            live.service(self);
        }
    }
}

#[test]
fn the_fallback_notice_is_said_in_the_log_where_a_sing_line_fires_on_a_bar_line_and_between_two() {
    let v = absent();
    let env = Env { voices: Some(&v), ..Env::default() };
    let src = "tempo 120\ndeck B: melody c4 e4 g4 e4\nat bar 1: deck B: sing \"la\" on melody voice=alto\nat bar 2 beat 3: deck B: sing \"hey\" on melody voice=tenor\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", diag::render_all(&c.diagnostics, src));
    let comp = c.compiled.unwrap();
    assert_eq!(comp.controls.len(), 2);
    assert!(comp.controls.iter().all(|k| k.notice.as_deref() == Some(djlive::sing::FALLBACK)));
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    let mut live = Live::new();
    assert!(live.eval_with(src, &env).ok);
    h.run(&mut live, 4.0 * 8.0);
    let said: Vec<&String> = live.words.iter().filter(|w| w.contains(djlive::sing::FALLBACK)).collect();
    assert_eq!(said.len(), 2, "{:?}", live.words);
    // the one between bar lines says the bar it was due at, to the thousandth, as the line it belongs to does
    assert!(said[0].starts_with("bar 1: ") && said[1].starts_with("bar 2.750: "), "{said:?}");
    let creates = h.calls.iter().filter(|c| c.1 == "vocal" && c.2.get("action").and_then(JsonValue::as_str) == Some("create")).count();
    assert_eq!(creates, 2, "{:?}", h.calls.iter().map(|c| (&c.1, c.2.canonical_js())).collect::<Vec<_>>());
}

// ---- the editor's tables, edited by every feature, still know every feature's words

#[test]
fn the_editors_tables_keep_every_features_words_next_to_sing_and_voice() {
    use djlive::highlight::{highlight, Class};
    let src = "key am\nsection verse 8 bars {\n  deck B: melody deg(1 3 5 3)?0.5 | harmony i VI voice-lead\n  deck B: sing \"la\" on melody voice=alto bars=2 when $i == 1\n  duck B under A.kick 6db attack=5ms\n  mix { A level 0.7 }\n}\narrange stack(verse, then(verse, verse))\n";
    let c: Vec<(&str, Class)> = highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect();
    let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
    assert_eq!(class_of("sing"), Some(Class::Keyword), "{c:?}");
    assert_eq!(class_of("voice"), Some(Class::Param));
    assert_eq!(class_of("voice-lead"), Some(Class::Param));
    assert_eq!(class_of("attack"), Some(Class::Param));
    assert_eq!(class_of("deg"), Some(Class::Generator));
    assert_eq!(class_of("when"), Some(Class::Keyword));
    assert_eq!(class_of("under"), Some(Class::Keyword));
    assert_eq!(class_of("mix"), Some(Class::Keyword));
    assert_eq!(class_of("stack"), Some(Class::Keyword));
    assert_eq!(class_of("then"), Some(Class::Keyword));
    assert_eq!(class_of("melody"), Some(Class::Role));
    // and a program of all of them is one the compiler takes
    compile("key am\nsection verse 8 bars {\n  deck A: kick x ~ x ~\n  deck B: melody deg(1 3 5 3)?0.5 | harmony i VI voice-lead\n  deck B: sing \"la\" on melody voice=alto bars=2\n  duck B under A.kick 6db attack=5ms\n  mix { A level 0.7 }\n}\narrange verse\n");
}
