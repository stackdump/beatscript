//! `btw test` (F12, docs/beatscript/ext-assert.md): the assertions of a BeatScript program, judged. The static ones (`ends`, `period`,
//! `length`, `key`) are re-checked from `djlive` ([`djlive::compile::check_report`]); the `level` ones are measured on an
//! **offline render** of the program (`beatscript_render::arrangement` played by `render_arrangement`, the engine djapp plays
//! on) and judged by `djlive::assertion::LevelCheck::judge`.
//!
//! **How a deck's level is measured.** One render, with the engine's *stems* on: the offline renderer already keeps each
//! mixer strip's share of the master block by block (`ArrangeRender::stems`: `A`, `B`, ... after the fader, the crossfader
//! and the master level, adding up to the master mix), so deck `A`'s level is the RMS of the `A` stem over the bars of the
//! section and `master` is the RMS of the master mix. That is the cheapest honest way (one render, not one a deck), and
//! it is deterministic: the renderer is the same every time, the windows are exact bar lines (`Arrangement::seconds_at`,
//! which follows the tempo the arrangement carries), and nothing here reads a clock. The RMS is linear, over both
//! channels (`sqrt(sum(l² + r²) / (2 n))`), so full-scale is about 0.7 and silence is exactly 0.
//!
//! **What the render plays** is what `beatscript_render` carries (docs/beatscript/ext-render.md): the notes and sections, and the control lines
//! on the mixer (a deck's level, EQ, mute, the master level, the crossfader, ramps of every shape and lfos on them, holds), `duck`
//! and `mix`. So a deck that fades in over a section is measured fading in. The lines it has no op for (a filter, a delay, a
//! reverb, a move, a `do` call) are silent in it, and the report says how many there are ([`Report::not_in_render`]).
//!
//! Only as much as the assertions need is rendered: from bar 0 to the end of the last window asked for (the whole
//! program for an assertion with no `in`; a program with no end is 16 bars, as `btw render`'s).

use djengine::json::{JsonObject, JsonValue};
use djengine::session::arrangement::Arrangement;
use djengine::session::wav::RenderedAudio;
use djlive::assertion::{Checked, Status};
use djlive::compile::Compiled;
use djlive::diag::{line_col, Diagnostic, Span};
use djlive::env::Env;

use crate::beatscript_render::NotInRender;
use crate::deck::offline::{render_arrangement, ArrangeRenderOptions};

/// The sample rates a measurement render may use (Hz): what the `btw test` verb, `djctl btw test` and `djctl btw render --sr` all take.
pub const SAMPLE_RATES: std::ops::RangeInclusive<f64> = 8_000.0..=192_000.0;

/// A sample rate from a caller (a verb's `sample_rate`, a CLI `--sr`): a finite number in [`SAMPLE_RATES`], else the words that say so.
/// (NaN, infinity, zero and a negative rate would panic the renderer or never return.)
pub fn sample_rate(x: f64) -> Result<f64, String> {
    if SAMPLE_RATES.contains(&x) {
        Ok(x)
    } else {
        Err(format!("the sample rate is {} to {} Hz, not {x}", SAMPLE_RATES.start(), SAMPLE_RATES.end()))
    }
}

/// How to render for the measurement.
#[derive(Clone, Debug)]
pub struct Options {
    pub sample_rate: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options { sample_rate: 44_100.0 }
    }
}

/// One assertion, judged.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub span: Span,
    /// 1-based, of the statement in the text that was tested.
    pub line: usize,
    pub col: usize,
    /// The canonical text: `assert level A < 2 * B in drop`.
    pub text: String,
    pub pass: bool,
    /// Proved by `check` (`false`) or measured on a render (`true`).
    pub rendered: bool,
    /// What it came to: `the program is 3m31s`, `A 0.083, B 0.051: 0.083 < 0.102`.
    pub detail: String,
    /// Every level read, as measured.
    pub measured: Vec<(String, f64)>,
    /// The seconds of the render each level was measured over, one `(from, to)` for each stretch the section plays in (the
    /// whole program when there is no `in`): the bar lines through the render's own clock, which is the tempo map's
    /// (`Compiled::tempo_map().seconds_at(bar)`) under a tempo curve. Empty for a static.
    pub windows: Vec<(f64, f64)>,
    /// What to write instead (a static that is false), or a remark on the measurement.
    pub note: Option<String>,
}

impl Outcome {
    /// `assert level A < 2 * B in drop: A 0.083, B 0.051: 0.083 < 0.102: pass`.
    pub fn line_text(&self) -> String {
        format!("{}: {}: {}", self.text, self.detail, if self.pass { "pass" } else { "fail" })
    }
}

/// What the render was.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub bars: u32,
    pub seconds: f64,
    pub sample_rate: f64,
    /// Moves the engine refused (a live op it could not take); the measurement is of what it did play.
    pub refused: Vec<String>,
}

/// Every assertion of a program, judged.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub cid: String,
    pub outcomes: Vec<Outcome>,
    /// None when no assertion needed a render.
    pub rendered: Option<Rendered>,
    /// The performed inputs the program reads (`knob(3)`, `held(key)` ...), as `check` names them (F13). The render has no
    /// controllers, so each of them reads 1/2 in it: the value a performed line is checked with; a line with a `when` is in the
    /// render if its guard is true then. A level, EQ, mute, crossfader, ramp or lfo on the mixer is in the render, performed or not;
    /// a filter, an insert or a `do` call is not (see `not_in_render`).
    pub performed: Vec<String>,
    /// The control lines the render the levels were measured on had no op for (a filter, a delay, a reverb, a move, a `do` call):
    /// silent in the measurement, so a level assertion about what they do is not judged by it. Empty when no render was made.
    pub not_in_render: NotInRender,
}

/// What a report says of a program that is performed: the levels are measured with every input at 1/2.
pub const PERFORMED_NOTE: &str = "this program is performed: levels are measured with every input at 1/2 (the render has no controllers), and only the lines a render can carry are in it (mixer levels, EQ, mute, crossfader, play and stop, duck, mix)";

impl Report {
    pub fn passed(&self) -> usize {
        self.outcomes.iter().filter(|o| o.pass).count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes.len() - self.passed()
    }

    /// Every assertion held (a program with none is fine).
    pub fn ok(&self) -> bool {
        self.failed() == 0
    }

    /// One line a assertion, then a summary.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for o in &self.outcomes {
            s.push_str(&format!("{}\n", o.line_text()));
            if let (false, Some(n)) = (o.pass, &o.note) {
                s.push_str(&format!("    help: {n}\n"));
            }
        }
        if !self.performed.is_empty() && self.rendered.is_some() {
            s.push_str(&format!("note: {PERFORMED_NOTE} ({})\n", self.performed.join(", ")));
        }
        // what the measured render could not play: the levels are of the program without those lines
        if self.rendered.is_some() && !self.not_in_render.is_empty() {
            s.push_str(&format!("note: {}\n", self.not_in_render.note()));
        }
        s.push_str(&match (self.outcomes.len(), self.failed()) {
            (0, _) => "no assertions in this program".to_string(),
            (n, 0) => format!("{n} assertion{} pass", if n == 1 { "" } else { "s" }),
            (n, f) => format!("{f} of {n} assertion{} fail", if n == 1 { "" } else { "s" }),
        });
        s
    }

    /// The answer of the `btw test` verb.
    pub fn to_json(&self) -> JsonValue {
        let results: Vec<JsonValue> = self
            .outcomes
            .iter()
            .map(|o| {
                let mut m = JsonObject::new();
                for (w, v) in &o.measured {
                    m.set(w.as_str(), JsonValue::Number(*v));
                }
                JsonValue::Object(JsonObject::from_pairs([
                    ("line", JsonValue::int(o.line as i64)),
                    ("col", JsonValue::int(o.col as i64)),
                    ("assertion", JsonValue::str(o.text.clone())),
                    ("kind", JsonValue::str(if o.rendered { "level" } else { "static" })),
                    ("status", JsonValue::str(if o.pass { "pass" } else { "fail" })),
                    ("pass", JsonValue::Bool(o.pass)),
                    ("detail", JsonValue::str(o.detail.clone())),
                    ("text", JsonValue::str(o.line_text())),
                    ("measured", JsonValue::Object(m)),
                    ("windows", JsonValue::Array(o.windows.iter().map(|(a, b)| JsonValue::Array(vec![JsonValue::Number(*a), JsonValue::Number(*b)])).collect())),
                    ("help", o.note.clone().map_or(JsonValue::Null, JsonValue::str)),
                ]))
            })
            .collect();
        let rendered = match &self.rendered {
            Some(r) => JsonValue::Object(JsonObject::from_pairs([
                ("bars", JsonValue::int(i64::from(r.bars))),
                ("seconds", JsonValue::Number(r.seconds)),
                ("sample_rate", JsonValue::Number(r.sample_rate)),
                ("refused", JsonValue::Array(r.refused.iter().map(|x| JsonValue::str(x.clone())).collect())),
            ])),
            None => JsonValue::Null,
        };
        JsonValue::Object(JsonObject::from_pairs([
            ("ok", JsonValue::Bool(self.ok())),
            ("cid", JsonValue::str(self.cid.clone())),
            ("assertions", JsonValue::int(self.outcomes.len() as i64)),
            ("passed", JsonValue::int(self.passed() as i64)),
            ("failed", JsonValue::int(self.failed() as i64)),
            ("rendered", rendered),
            ("summary", JsonValue::str(self.text().lines().last().unwrap_or("").to_string())),
            ("results", JsonValue::Array(results)),
            ("not_in_render", if self.rendered.is_some() && !self.not_in_render.is_empty() { self.not_in_render.to_json() } else { JsonValue::Null }),
            (
                "performed",
                if self.performed.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::Object(JsonObject::from_pairs([
                        ("inputs", JsonValue::Array(self.performed.iter().map(|x| JsonValue::str(x.clone())).collect())),
                        ("read_as", JsonValue::Number(0.5)),
                        ("note", JsonValue::str(PERFORMED_NOTE)),
                    ]))
                },
            ),
        ]))
    }
}

/// What one render left to measure: the arrangement (for its clock), the master mix and each strip's stem.
struct Rendering {
    arr: Arrangement,
    master: RenderedAudio,
    stems: Vec<(String, RenderedAudio)>,
}

/// The root-mean-square of `audio` over frames `[from, to)`, both channels (0 for no frames): the sum of squares and the count.
fn energy(audio: &RenderedAudio, from: usize, to: usize) -> (f64, usize) {
    let to = to.min(audio.left.len()).min(audio.right.len());
    if from >= to {
        return (0.0, 0);
    }
    let sum: f64 = audio.left[from..to].iter().chain(&audio.right[from..to]).map(|x| f64::from(*x) * f64::from(*x)).sum();
    (sum, 2 * (to - from))
}

/// The bars the render has to cover for these assertions, and how many bars the program has to measure "all of".
fn whole_program_bars(c: &Compiled) -> u32 {
    c.end.or_else(|| c.song_bars()).unwrap_or(16)
}

fn static_outcome(c: &Compiled, a: &Checked) -> Outcome {
    let (line, col) = line_col(&c.source, a.span.start);
    Outcome { span: a.span, line, col, text: a.text.clone(), pass: a.status == Status::Pass, rendered: false, detail: a.message.clone(), measured: Vec::new(), windows: Vec::new(), note: a.help.clone() }
}

/// Judge every assertion of a compiled program (from [`djlive::compile::check_report`], so a false static one is a failure
/// here, not a refusal). Renders once, and only when a `level` assertion asks for it.
pub fn run(c: &Compiled, o: &Options) -> Report {
    let total = whole_program_bars(c);
    let pending: Vec<&Checked> = c.assertions.iter().filter(|a| a.level.is_some()).collect();
    let windows_of = |a: &Checked| -> Vec<(u32, u32)> {
        let l = a.level.as_ref().expect("a level assertion");
        let w = if l.windows.is_empty() { vec![(0, total)] } else { l.windows.clone() };
        // nothing plays past the program's end
        w.into_iter().map(|(s, e)| (s.min(total), e.min(total))).collect()
    };
    let upto = pending.iter().flat_map(|a| windows_of(a)).map(|(_, e)| e).max().unwrap_or(0);
    let mut rendered = None;
    let mut audio: Option<Rendering> = None;
    let mut not_in_render = NotInRender::default();
    if !pending.is_empty() && upto > 0 {
        let (json, _, left) = crate::beatscript_render::arrangement_report(c);
        not_in_render = left;
        if let Some((arr, _)) = Arrangement::from_json(&json) {
            let beats = f64::from(upto) * 4.0;
            let r = render_arrangement(&arr, ArrangeRenderOptions { sample_rate: o.sample_rate, beats: Some(beats), stems: true, ..Default::default() });
            rendered = Some(Rendered {
                bars: upto,
                seconds: r.audio.frames() as f64 / o.sample_rate,
                sample_rate: o.sample_rate,
                refused: r.refused.iter().map(|(at, why)| format!("beat {at}: {why}")).collect(),
            });
            audio = Some(Rendering { arr, master: r.audio, stems: r.stems });
        }
    }
    let outcomes = c
        .assertions
        .iter()
        .map(|a| {
            let Some(level) = &a.level else { return static_outcome(c, a) };
            let (line, col) = line_col(&c.source, a.span.start);
            let base = Outcome { span: a.span, line, col, text: a.text.clone(), pass: false, rendered: true, detail: String::new(), measured: Vec::new(), windows: Vec::new(), note: None };
            let Some(Rendering { arr, master, stems }) = &audio else {
                return Outcome { detail: "the program could not be rendered".into(), note: Some("`btw render` says why".into()), ..base };
            };
            let sr = o.sample_rate;
            let frame = |bar: u32| (arr.seconds_at(f64::from(bar) * 4.0) * sr).round().max(0.0) as usize;
            let windows = windows_of(a);
            let seconds: Vec<(f64, f64)> = windows.iter().map(|(s, e)| (arr.seconds_at(f64::from(*s) * 4.0), arr.seconds_at(f64::from(*e) * 4.0))).collect();
            let measure = |word: &str| -> f64 {
                let source: Option<&RenderedAudio> = if word == "master" { Some(master) } else { stems.iter().find(|(n, _)| n == word).map(|(_, s)| s) };
                let Some(src) = source else { return 0.0 };
                let (mut sum, mut n) = (0.0, 0usize);
                for (s, e) in &windows {
                    let (x, k) = energy(src, frame(*s), frame(*e));
                    sum += x;
                    n += k;
                }
                if n == 0 {
                    0.0
                } else {
                    (sum / n as f64).sqrt()
                }
            };
            let v = level.judge(&measure);
            let empty = windows.iter().all(|(s, e)| s >= e);
            let note = if empty { Some(format!("nothing of {} plays: the program ends at bar {total}", level.section.as_deref().unwrap_or("the program"))) } else { None };
            Outcome { pass: v.pass, detail: v.text, measured: v.measured, windows: seconds, note, ..base }
        })
        .collect();
    let performed = djlive::perform::performed_inputs(&c.controls).iter().map(djlive::perform::Input::id).collect();
    Report { cid: c.cid.clone(), outcomes, rendered, performed, not_in_render }
}

/// Check `src` (a false static assertion is a failure of the report, not a refusal) and judge its assertions. The
/// program's own errors come back as they are.
pub fn run_text(src: &str, env: &Env, o: &Options) -> Result<Report, Vec<Diagnostic>> {
    let c = djlive::compile::check_report(src, env);
    match c.compiled {
        Some(comp) => Ok(run(&comp, o)),
        None => Err(c.diagnostics),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(src: &str) -> Report {
        run_text(src, &Env::none(), &Options { sample_rate: 22_050.0 }).unwrap_or_else(|d| panic!("{}", djlive::diag::render_all(&d, src)))
    }

    fn outcome<'a>(r: &'a Report, text: &str) -> &'a Outcome {
        r.outcomes.iter().find(|o| o.text == text).unwrap_or_else(|| panic!("no assertion `{text}` in {:?}", r.outcomes.iter().map(|o| &o.text).collect::<Vec<_>>()))
    }

    /// A quiet intro of one soft hi-hat, then two bars with a loud kick and bass on deck A and a soft hi-hat on deck B.
    const SONG: &str = "tempo 120
section quiet 2 bars {
  deck A: hihat x ~ ~ ~ vel=30
}
section loud 2 bars {
  deck A: kick x ~ x ~ vel=127 | bass c2 c2 c2 c2 vel=127
  deck B: hihat x ~ ~ ~ vel=20
}
arrange quiet loud
";

    #[test]
    fn a_clearly_louder_deck_is_louder_in_its_section() {
        let r = report(&format!("{SONG}assert level A > 2 * B in loud\nassert level A < B in loud\nassert level A > 0.05 in loud\nassert level A < 0.05 in quiet\nassert level master > 0\n"));
        assert_eq!(r.outcomes.len(), 5);
        let pass = |t: &str| outcome(&r, t).pass;
        assert!(pass("assert level A > 2 * B in loud"), "{}", r.text());
        assert!(!pass("assert level A < B in loud"), "{}", r.text());
        // the windows isolate the sections: the same deck is loud in one and quiet in the other
        assert!(pass("assert level A > 0.05 in loud") && pass("assert level A < 0.05 in quiet"), "{}", r.text());
        assert!(pass("assert level master > 0"));
        assert_eq!(r.passed(), 4);
        assert_eq!(r.failed(), 1);
        assert!(!r.ok());
        // the report says what was measured, in the contract's words
        let o = outcome(&r, "assert level A > 2 * B in loud");
        assert!(o.rendered);
        assert!(o.detail.starts_with("A 0.") && o.detail.contains(", B 0.") && o.detail.contains(" > "), "{}", o.detail);
        assert!(o.line_text().ends_with(": pass") && o.line_text().starts_with("assert level A > 2 * B in loud: A "), "{}", o.line_text());
        let (a, b) = (o.measured[0].1, o.measured[1].1);
        assert!(o.measured[0].0 == "A" && o.measured[1].0 == "B" && a > 2.0 * b && b > 0.0, "{:?}", o.measured);
        // four bars at 120 BPM are eight seconds; only the windows' last bar was needed
        let rd = r.rendered.as_ref().expect("a level assertion renders");
        assert_eq!((rd.bars, rd.seconds, rd.sample_rate), (4, 8.0, 22_050.0));
    }

    #[test]
    fn an_unused_deck_is_exactly_silent_and_the_master_hears_the_one_deck() {
        let r = report("tempo 120\ndeck A: kick x ~ x ~ vel=127\nassert level B == 0\nassert level A > 0\nassert level master > 0.9 * A\nassert level master < 1.1 * A\nassert level A != 0\nassert level B > 0\n");
        let pass = |t: &str| outcome(&r, t).pass;
        assert!(pass("assert level B == 0") && pass("assert level A > 0") && pass("assert level A != 0"), "{}", r.text());
        assert!(pass("assert level master > 0.9 * A") && pass("assert level master < 1.1 * A"), "{}", r.text());
        assert!(!pass("assert level B > 0"));
        // a program with no end and no sections is measured over 16 bars, as `btw render`'s
        assert_eq!(r.rendered.as_ref().unwrap().bars, 16);
        assert_eq!(outcome(&r, "assert level B == 0").detail, "B 0: 0 == 0");
    }

    #[test]
    fn an_end_bounds_the_render_and_a_section_past_it_says_so() {
        let r = report("tempo 120\ndeck A: kick x ~ x ~ vel=127\nend at bar 2\nassert level A > 0\n");
        assert_eq!(r.rendered.as_ref().unwrap().bars, 2);
        assert!(r.ok(), "{}", r.text());
        // a section that never plays: the song ends after `intro`, `outro` is behind the end
        let r = report("tempo 120\nsection intro 2 bars {\n  deck A: kick x ~ x ~ vel=127\n}\nsection outro 2 bars {\n  deck A: kick x ~ x ~ vel=127\n}\narrange intro outro\nend after intro\nassert level A > 0 in outro\n");
        let o = &r.outcomes[0];
        assert!(!o.pass && o.note.as_deref().is_some_and(|n| n.contains("ends at bar 2")), "{o:?}");
    }

    #[test]
    fn the_measurement_is_the_same_every_time() {
        let src = format!("{SONG}assert level A > 2 * B in loud\nassert level master >= A\n");
        let (a, b) = (report(&src), report(&src));
        assert_eq!(a, b);
        assert_eq!(a.to_json().js_stringified(), b.to_json().js_stringified());
    }

    #[test]
    fn statics_are_rechecked_without_a_render_and_a_false_one_is_a_failure_not_a_refusal() {
        let r = report("tempo 120\ndeck A: kick x\nend at bar 8\nassert ends\nassert length == 16s\nassert length == 1m\nassert key == am\n");
        assert!(r.rendered.is_none(), "no level assertion, no render");
        let by = |t: &str| outcome(&r, t);
        assert!(by("assert ends").pass && by("assert length == 16s").pass);
        let bad = by("assert length == 1m");
        assert!(!bad.pass && !bad.rendered && bad.detail == "the program is 16s", "{bad:?}");
        assert_eq!(bad.line_text(), "assert length == 1m: the program is 16s: fail");
        assert!(!by("assert key == am").pass);
        assert_eq!(r.failed(), 2);
        assert_eq!(r.text().lines().last(), Some("2 of 4 assertions fail"));
        // `check` itself refuses it
        assert!(djlive::check("tempo 120\ndeck A: kick x\nend at bar 8\nassert length == 1m\n").compiled.is_none());
    }

    #[test]
    fn a_program_with_no_assertions_has_nothing_to_fail_and_one_with_errors_is_not_tested() {
        let r = report("deck A: kick x\n");
        assert!(r.ok() && r.outcomes.is_empty() && r.text() == "no assertions in this program");
        let e = run_text("deck A: kick x\nassert level A < B in nowhere\n", &Env::none(), &Options::default()).unwrap_err();
        assert!(e.iter().any(|d| d.message.contains("there is no section `nowhere`")), "{e:?}");
    }

    #[test]
    fn the_json_answer_carries_each_assertion_with_its_place_and_its_measures() {
        let r = report(&format!("{SONG}assert ends\nassert level A > 2 * B in loud\n"));
        let j = r.to_json();
        assert_eq!(j.get("ok").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(j.get("assertions").and_then(JsonValue::as_f64), Some(2.0));
        let rows = j.get("results").and_then(JsonValue::as_array).unwrap();
        assert_eq!(rows[0].get("kind").and_then(JsonValue::as_str), Some("static"));
        assert_eq!(rows[1].get("kind").and_then(JsonValue::as_str), Some("level"));
        assert_eq!(rows[1].get("line").and_then(JsonValue::as_f64), Some(11.0));
        assert!(rows[1].get("measured").and_then(|m| m.get("A")).and_then(JsonValue::as_f64).is_some_and(|a| a > 0.0));
        assert!(j.get("rendered").and_then(|r| r.get("bars")).is_some());
        assert_eq!(j.get("summary").and_then(JsonValue::as_str), Some("2 assertions pass"));
    }

    /// F13 meets F12: the render has no controllers. A performed program is measured with every input at 1/2, and the report
    /// says so, in its text and its JSON; a program with no input says nothing of it.
    #[test]
    fn a_performed_program_is_measured_with_every_input_at_one_half_and_the_report_says_so() {
        let src = format!("{SONG}every 1 bars: deck A: level knob(1)\nevery 2 bars: master: filter lp 200 + knob(2) * 7800 when held(key)\nassert level A > 2 * B in loud\n");
        let r = report(&src);
        assert!(r.ok(), "{}", r.text());
        assert_eq!(r.performed.len(), 3);
        assert!(["held(key)", "knob(1)", "knob(2)"].iter().all(|i| r.performed.iter().any(|x| x == i)), "{:?}", r.performed);
        let t = r.text();
        assert!(t.contains("note: this program is performed: levels are measured with every input at 1/2") && t.contains("knob(1)") && t.contains("held(key)"), "{t}");
        // the note comes before the summary line
        assert_eq!(t.lines().last(), Some("1 assertion pass"));
        let j = r.to_json();
        let p = j.get("performed").expect("performed");
        assert_eq!(p.get("read_as").and_then(JsonValue::as_f64), Some(0.5));
        assert_eq!(p.get("inputs").and_then(JsonValue::as_array).map(Vec::len), Some(3));
        // a program that reads nothing says nothing
        let plain = report(&format!("{SONG}assert level A > 2 * B in loud\n"));
        assert!(plain.performed.is_empty() && !plain.text().contains("performed") && plain.to_json().get("performed") == Some(&JsonValue::Null));
        // and the program with the inputs measures as the program with the value 1/2 written out does (the lines the render honours
        // are the same lines: a level and a filter are not in a render, performed or not)
        let literal = report(&format!("{SONG}every 1 bars: deck A: level 0.5\nevery 2 bars: master: filter lp 4100\nassert level A > 2 * B in loud\n"));
        assert_eq!(r.outcomes[0].measured, literal.outcomes[0].measured);
    }

    /// A deck that fades in over a section is measured fading in (the render carries the mixer's ramps: docs/beatscript/ext-render.md): with
    /// the same text but a deck A at a constant level, the section measures differently, and `assert level A < B` changes its answer.
    #[test]
    fn a_level_assertion_over_a_faded_in_deck_measures_the_fade() {
        let song = |a_line: &str| format!("tempo 120\nsection fade 4 bars {{\n  deck A: bass c2 len=16\n  deck B: bass c2 len=16\n  deck B: level 0.7\n  {a_line}\n}}\narrange fade\nassert level A < B in fade\n");
        let levels = |r: &Report| {
            let m = &r.outcomes[0].measured;
            (m.iter().find(|(w, _)| w == "A").unwrap().1, m.iter().find(|(w, _)| w == "B").unwrap().1)
        };
        let fading = report(&song("deck A: level 0 -> 1 over 4 bars"));
        let flat = report(&song("deck A: level 1"));
        let ((fa, fb), (pa, pb)) = (levels(&fading), levels(&flat));
        // B is untouched by A's line
        assert!((fb - pb).abs() < 1e-9 * pb.max(1.0), "{fb} vs {pb}");
        // a level that rises evenly from 0 to 1 over the section has the root of the mean of its square, 1/sqrt(3), of a constant 1
        let ratio = fa / pa;
        assert!((0.52..0.64).contains(&ratio), "the fade measures {ratio:.3} of the constant level, 1/sqrt(3) = 0.577 by the line");
        // A fading in is quieter than B at 0.7 over the section; the same deck at a constant 1 is louder
        assert!(fading.ok() && fa < fb, "{}", fading.text());
        assert!(!flat.ok() && pa > pb, "{}", flat.text());
        // and the render the fade is measured on has no lines left out
        assert!(fading.not_in_render.is_empty() && !fading.text().contains("not in this render"));
    }

    /// What the render has no op for is said in the report: a filter's sweep is silent in the measurement, and the note says how many
    /// lines that is, which, and where.
    #[test]
    fn the_report_says_how_many_control_lines_the_render_could_not_play() {
        let src = format!("{SONG}master: filter lp 300 -> 2k over 4 bars\ndeck B: reverb 0.4\nassert level A > 2 * B in loud\n");
        let r = report(&src);
        assert_eq!(r.not_in_render.count(), 2, "{:?}", r.not_in_render);
        let t = r.text();
        assert!(t.contains("note: not in this render: 2 control lines (filter x1, reverb x1): line 10 `master: filter lp 300 -> 2k over 4 bars`, line 11 `deck B: reverb 0.4`"), "{t}");
        // the note comes before the summary
        assert_eq!(t.lines().last(), Some("1 assertion pass"));
        let j = r.to_json();
        let n = j.get("not_in_render").expect("not_in_render");
        assert_eq!(n.get("count").and_then(JsonValue::as_f64), Some(2.0));
        assert_eq!(n.get("kinds").and_then(|k| k.get("filter")).and_then(JsonValue::as_f64), Some(1.0));
        // a program with none, or one that needed no render, says nothing
        let plain = report(&format!("{SONG}assert level A > 2 * B in loud\n"));
        assert!(plain.not_in_render.is_empty() && !plain.text().contains("not in this render") && plain.to_json().get("not_in_render") == Some(&JsonValue::Null));
        let static_only = report(&format!("{SONG}master: filter lp 300\nassert ends\n"));
        assert!(static_only.rendered.is_none() && !static_only.text().contains("not in this render"));
    }

    /// A duck with a performed `when` is in the render when the guard is true with the inputs at 1/2.
    #[test]
    fn a_duck_with_a_performed_guard_is_in_the_render_when_it_holds_at_one_half() {
        let song = |duck: Option<&str>| format!("tempo 124\ndeck A: kick x ~ x ~\ndeck B: bass c2 len=16\n{}assert level B > 0\n", duck.map_or(String::new(), |g| format!("duck B under A.kick 12db{g}\n")));
        let level_b = |guard: Option<&str>| {
            let r = report(&song(guard));
            outcome(&r, "assert level B > 0").measured.iter().find(|(w, _)| w == "B").map(|x| x.1).expect("B measured")
        };
        let plain = level_b(None);
        let always = level_b(Some(""));
        let ducked = level_b(Some(" when held(key)"));
        let not = level_b(Some(" when knob(1) < 0.5"));
        let held_c4 = level_b(Some(" when held(c4)"));
        assert!(always < 0.99 * plain, "the duck itself is in the render: {always} vs {plain}");
        assert!(ducked < 0.99 * plain, "1/2 is a key down: the duck is in the render: {ducked} vs {plain}");
        assert!((not - plain).abs() < 1e-9, "knob(1) < 1/2 is false at 1/2: no duck: {not} vs {plain}");
        assert!(held_c4 < 0.99 * plain, "held(c4) at 1/2 is not 0: {held_c4}");
        assert!((ducked - always).abs() < 1e-12, "a guard that holds is the duck as it was: {ducked} vs {always}");
    }
}
