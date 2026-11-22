//! F4 ramp shapes and lfos (docs/btw/ext-shape.md).
//!
//! ```text
//! shape  = "linear" | "exp" | "log" | "s-curve" | "step" int ;
//! change = value | value "->" value "over" dur [ shape ] | lfo ;
//! lfo    = "lfo" wave dur { "depth=" value | "center=" value | "phase=" value } [ "over" dur ] ;
//! wave   = "sine" | "tri" | "square" | "saw" ;
//! ```
//!
//! A **shape** maps a ramp's time `t` (0..1) to its progress; the value is `from + (to - from) * shape(t)`. An
//! **lfo** is a value that keeps moving: `center + depth * wave(t / period + phase)`, sent on the service pass like a
//! ramp, for `over dur` or until its section ends. Both are values of every place a change takes one: a control line
//! (`master: filter lp lfo sine 2 bars center=3k depth=1k`), a `do` value (`value=lfo tri 1 bar depth=0.2`), and the
//! tempo (shapes only, `tempo 100 -> 130 over 8 bars s-curve`: tempo.rs).
//!
//! Where it plugs in: the parser's [`Parser::finish_change`] (the rest of a control line after its name), [`Parser::shape_opt`]
//! (a shape after `over dur`), [`Parser::lfo_spec`]; the AST's [`Motion`] (a shaped ramp or an lfo on a control line,
//! `Action::Shaped`), [`LfoSpec`] (a `do` value, `DoValue::Lfo`); the compiler's [`Compiler::shaped`] and [`Lfo`]
//! (`Effect::Lfo`), and `Ramp.shape`; the runtime's [`LfoRun`].

use djengine::json::JsonValue;

use crate::ast::{Action, Num, Target, Word};
use crate::compile::{Call, Compiler, Effect};
use crate::diag::{Diagnostic, Span};
use crate::dur::Dur;
use crate::lex::Tok;
use crate::parse::{nearest, Parser};

type R<T> = Result<T, Diagnostic>;

/// The steepness of `exp` and `log`: `exp(t) = (2^(k t) - 1) / (2^k - 1)`.
pub const EXP_K: f64 = 4.0;
/// The most steps a `step N` shape takes (a ramp sends one value per step).
pub const MAX_STEPS: u32 = 1024;
/// The shortest lfo period: a quarter of a beat. The runtime looks at the position about sixty times a second; a wave
/// shorter than a few passes is not drawn.
pub fn min_period() -> crate::rat::Rat {
    crate::rat::Rat::new(1, 16).unwrap_or(crate::rat::Rat::ONE)
}

pub const SHAPE_WORDS: [&str; 5] = ["linear", "exp", "log", "s-curve", "step"];
pub const WAVE_WORDS: [&str; 4] = ["sine", "tri", "square", "saw"];
/// The words the editor draws as keywords (highlight.rs): `lfo`, the waves, the shapes.
pub const HIGHLIGHT_KEYWORDS: [&str; 10] = ["lfo", "sine", "tri", "square", "saw", "exp", "log", "step", "s-curve", "linear"];
/// The params of an lfo, drawn as params when they are followed by `=`.
pub const LFO_PARAMS: [&str; 3] = ["depth", "center", "phase"];

// ---------------------------------------------------------------------------------------------------------------------
// shapes
// ---------------------------------------------------------------------------------------------------------------------

/// How a ramp's time becomes its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Shape {
    /// `t`: the default, never printed.
    #[default]
    Linear,
    /// Slow start, fast end: `(2^(k t) - 1) / (2^k - 1)`, `k` = [`EXP_K`].
    Exp,
    /// Fast start, slow end: the mirror of `exp`, `1 - exp(1 - t)`.
    Log,
    /// Smoothstep, `3 t^2 - 2 t^3`.
    SCurve,
    /// `n` values, `floor(t n) / (n - 1)` clamped: it goes from the start to the end in `n - 1` leaps.
    Step(u32),
}

impl Shape {
    /// The progress (0..1) at time `t` (0..1; anything else is clamped).
    pub fn apply(self, t: f64) -> f64 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Shape::Linear => t,
            Shape::Exp => exp_curve(t),
            Shape::Log => 1.0 - exp_curve(1.0 - t),
            Shape::SCurve => t * t * (3.0 - 2.0 * t),
            Shape::Step(n) => {
                let n = f64::from(n.max(2));
                // (a hair of slack so that a `t` that is exactly k / n by hand is not floored a step early)
                (((t * n + 1e-12).floor()) / (n - 1.0)).min(1.0)
            }
        }
    }

    pub fn is_linear(self) -> bool {
        self == Shape::Linear
    }

    /// The times `t` (0..1, ascending) at which the progress jumps: `k / n` for `step n`, none for the smooth shapes. A
    /// sampler that wants the leaps on their own instant, not on the next tick of its grid, looks there (the offline render).
    pub fn breaks(self) -> Vec<f64> {
        match self {
            Shape::Step(n) => (1..n.max(2)).map(|k| f64::from(k) / f64::from(n.max(2))).collect(),
            _ => Vec::new(),
        }
    }

    /// The canonical word(s): None for `linear`, which is the default and is not printed.
    pub fn canonical(self) -> Option<String> {
        match self {
            Shape::Linear => None,
            Shape::Exp => Some("exp".into()),
            Shape::Log => Some("log".into()),
            Shape::SCurve => Some("s-curve".into()),
            Shape::Step(n) => Some(format!("step {n}")),
        }
    }

    /// How the shape is said in a message.
    pub fn name(self) -> String {
        self.canonical().unwrap_or_else(|| "linear".into())
    }
}

fn exp_curve(t: f64) -> f64 {
    ((EXP_K * t).exp2() - 1.0) / (EXP_K.exp2() - 1.0)
}

// ---------------------------------------------------------------------------------------------------------------------
// waves
// ---------------------------------------------------------------------------------------------------------------------

/// An lfo's wave. All four start on the midline and rise (a square starts at the top): the oscillators' own phase, so
/// `phase=` means the same thing for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wave {
    Sine,
    Tri,
    Square,
    Saw,
}

impl Wave {
    pub fn parse(w: &str) -> Option<Wave> {
        Some(match w {
            "sine" => Wave::Sine,
            "tri" => Wave::Tri,
            "square" => Wave::Square,
            "saw" => Wave::Saw,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Wave::Sine => "sine",
            Wave::Tri => "tri",
            Wave::Square => "square",
            Wave::Saw => "saw",
        }
    }

    /// The wave, -1..1, at `cycles` periods in (any real number: only its fraction counts).
    ///
    /// * `sine`: `sin(2 pi p)`.
    /// * `tri`: `(2 / pi) asin(sin(2 pi p))`: 0 at p = 0, 1 at 1/4, 0 at 1/2, -1 at 3/4.
    /// * `square`: 1 for the first half, -1 for the second.
    /// * `saw`: rises, `2 (p - round(p))`: 0 at p = 0, nearly 1 at 1/2, where it falls to -1 and rises to 0 again.
    pub fn at(self, cycles: f64) -> f64 {
        let p = cycles - cycles.floor();
        match self {
            Wave::Sine => (std::f64::consts::TAU * p).sin(),
            Wave::Tri => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Wave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Saw => 2.0 * (p - (p + 0.5).floor()),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the tree
// ---------------------------------------------------------------------------------------------------------------------

/// `lfo sine 2 bars center=3000 depth=1000 phase=0.25 over 16 bars`, as written. A value left out has its default
/// (depth 1, center 0, phase 0).
#[derive(Clone, Debug, PartialEq)]
pub struct LfoSpec {
    pub span: Span,
    pub wave: Wave,
    pub wave_span: Span,
    /// The period.
    pub rate: Dur,
    pub rate_span: Span,
    pub depth: Option<Num>,
    pub center: Option<Num>,
    pub phase: Option<Num>,
    pub over: Option<(Dur, Span)>,
}

impl LfoSpec {
    pub fn depth_value(&self) -> f64 {
        self.depth.map_or(1.0, |n| n.value)
    }

    pub fn center_value(&self) -> f64 {
        self.center.map_or(0.0, |n| n.value)
    }

    pub fn phase_value(&self) -> f64 {
        self.phase.map_or(0.0, |n| n.value)
    }
}

/// What a control line's change moves by, when it is more than a plain value or a linear ramp (those stay
/// `Action::Change`, as they always were).
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// `0.2 -> 0.9 over 4 bars exp`.
    Ramp { from: Num, to: Num, over: (Dur, Span), shape: Shape, shape_span: Span },
    /// `lfo sine 2 bars depth=0.2 center=0.5`.
    Lfo(LfoSpec),
}

// ---------------------------------------------------------------------------------------------------------------------
// the parser
// ---------------------------------------------------------------------------------------------------------------------

impl Parser<'_> {
    /// `[ shape ]` after `over dur`: `exp`, `log`, `s-curve`, `step N`, `linear`. None when the next token is no shape
    /// word; a word that is a near miss of one (`expo`, `scurve`) is an error that says which was meant.
    pub(crate) fn shape_opt(&mut self) -> R<Option<(Shape, Span)>> {
        let Tok::Word(w) = self.peek().clone() else { return Ok(None) };
        let start = self.span();
        let shape = match w.as_str() {
            "linear" => Shape::Linear,
            "exp" => Shape::Exp,
            "log" => Shape::Log,
            "s-curve" => Shape::SCurve,
            "step" => {
                self.bump();
                let (n, nsp) = self.int("how many steps (a whole number)")?;
                if !(2..=MAX_STEPS).contains(&n) {
                    return Err(Diagnostic::error(nsp, format!("`step N` takes N from 2 to {MAX_STEPS}")).found(nsp.text(self.src).to_string()).help("`step 4` is four values, from the start to the end in three leaps"));
                }
                return Ok(Some((Shape::Step(n), start.to(nsp))));
            }
            // `when` starts a guard, never a shape; anything else that is not close to a shape is the caller's to refuse
            "when" => return Ok(None),
            other => {
                if let Some(near) = nearest(other, SHAPE_WORDS.iter().copied()) {
                    return Err(Diagnostic::error(start, format!("`{other}` is not a shape"))
                        .expected(&["linear", "exp", "log", "s-curve", "step N"])
                        .found(format!("`{other}`"))
                        .help(format!("did you mean `{near}`?")));
                }
                return Ok(None);
            }
        };
        self.bump();
        Ok(Some((shape, start)))
    }

    /// The rest of a control line's change after its name (and a filter's `lp` / `hp`): a plain value, a ramp with
    /// its shape, or an lfo.
    pub(crate) fn finish_change(&mut self, name: Word, mode: Option<Word>) -> R<Action> {
        if self.word_is("lfo") {
            let spec = self.lfo_spec(false)?;
            return Ok(Action::Shaped { name, mode, motion: Motion::Lfo(spec) });
        }
        if let Tok::Word(w) = self.peek() {
            if w != "lfo" && crate::parse::nearest(w, ["lfo"]).is_some() {
                return Err(self.expected(&["a value", "`lfo`"], &format!("expected a value, found `{w}`")).help("did you mean `lfo`? master: filter lp lfo sine 2 bars center=3k depth=1k"));
            }
        }
        let (from, to, over) = self.change(name.text == "filter" && mode.is_some())?;
        let Some((shape, shape_span)) = self.shape_opt()? else {
            return Ok(Action::Change { name, mode, from: Some(from), to, over });
        };
        match (to, over) {
            (Some(to), Some(over)) => {
                if shape.is_linear() {
                    // `linear` is the default: the same line as with no shape
                    return Ok(Action::Change { name, mode, from: Some(from), to: Some(to), over: Some(over) });
                }
                Ok(Action::Shaped { name, mode, motion: Motion::Ramp { from, to, over, shape, shape_span } })
            }
            (None, _) => Err(Diagnostic::error(shape_span, format!("`{}` needs a ramp: `{} -> 1 over 4 bars {}`", shape.name(), crate::print::numn(&from), shape.name()))
                .found(shape_span.text(self.src).to_string())
                .help("a shape goes after the length of a ramp: `a -> b over 4 bars exp`")),
            (Some(_), None) => Err(Diagnostic::error(shape_span, format!("`{}` needs the ramp's length: `over` is missing", shape.name()))
                .found(shape_span.text(self.src).to_string())
                .help(format!("a ramp is `a -> b over 4 bars {}`", shape.name()))),
        }
    }

    /// `lfo sine 2 bars [depth=v] [center=v] [phase=v] [over dur]` (the `lfo` is the next word). In a `do` line
    /// (`in_do`) a word followed by `=` that is not one of the three is the line's next param, not a mistake of ours.
    pub(crate) fn lfo_spec(&mut self, in_do: bool) -> R<LfoSpec> {
        let start = self.expect_word("lfo")?;
        // the wave
        let wave_span = self.span();
        let wave = match self.peek().clone() {
            Tok::Word(w) => match Wave::parse(&w) {
                Some(wv) => {
                    self.bump();
                    wv
                }
                None => {
                    let all: Vec<String> = WAVE_WORDS.iter().map(|s| s.to_string()).collect();
                    let mut d = Diagnostic::error(wave_span, format!("`{w}` is not a wave")).expected(&WAVE_WORDS).found(format!("`{w}`"));
                    if let Some(n) = crate::env::suggest(&w, &all, 1).first() {
                        d = d.help(format!("did you mean `{n}`?"));
                    } else {
                        d = d.help("lfo sine 2 bars   lfo tri 1 bar   lfo square 4 beats   lfo saw 8 bars");
                    }
                    return Err(d);
                }
            },
            _ => return Err(self.expected(&WAVE_WORDS, &format!("an lfo needs a wave: `sine`, `tri`, `square` or `saw`, found {}", self.found())).help("lfo sine 2 bars")),
        };
        // the period
        let (rate, rate_span) = self.dur("the lfo's period (`2 bars`, `4 beats`)")?;
        if rate.is_zero() {
            return Err(Diagnostic::error(rate_span, "an lfo's period takes some time").found(rate_span.text(self.src).to_string()).help("lfo sine 2 bars"));
        }
        if rate.as_bars() < min_period() {
            return Err(Diagnostic::error(rate_span, "that is faster than the clock: an lfo's period is at least 1/4 of a beat")
                .found(rate_span.text(self.src).to_string())
                .help("the runtime looks at the position about sixty times a second; `lfo sine 1/4 beat` is the fastest"));
        }
        // depth= center= phase=
        let (mut depth, mut center, mut phase): (Option<Num>, Option<Num>, Option<Num>) = (None, None, None);
        let mut end = rate_span;
        while let (Tok::Word(w), Tok::Eq) = (self.peek().clone(), self.peek_n(1).clone()) {
            let slot = match w.as_str() {
                "depth" => &mut depth,
                "center" => &mut center,
                "phase" => &mut phase,
                other => {
                    if !in_do {
                        if let Some(n) = nearest(other, ["depth", "center", "phase"]) {
                            return Err(Diagnostic::error(self.span(), format!("`{other}` is not a parameter of an lfo"))
                                .expected(&["depth=", "center=", "phase="])
                                .found(format!("`{other}`"))
                                .help(format!("did you mean `{n}`?")));
                        }
                    }
                    break;
                }
            };
            let name_span = self.span();
            self.bump();
            self.bump();
            // (F13: `center=knob(3) * 4k` is a control value: an input may stand, read again at every bar line)
            let v = crate::perform::allowing(|| self.num("a value"))?;
            if w == "phase" && crate::perform::is_live(&v) {
                return Err(Diagnostic::error(v.span, "an lfo takes a literal phase: only `center=` and `depth=` can be performed")
                    .found(v.span.text(self.src).to_string())
                    .help("`master: filter lp lfo sine 2 bars center=knob(3) * 4k depth=1k phase=0.25`: the wave keeps its phase, its middle and its reach follow the knob"));
            }
            // a value is a plain number in the param's own unit: a cutoff in Hz (`3k`), a mix 0 to 1 (`0.3`, `30%`)
            if let (Tok::Unit(u), true) = (self.peek().clone(), self.span().start == self.prev_span().end) {
                let unit = self.span();
                return Err(Diagnostic::error(v.span.to(unit), format!("`{w}` is a plain number in the param's own unit, not `{}{u}`", v.span.text(self.src)))
                    .found(v.span.to(unit).text(self.src).to_string())
                    .help("a filter's cutoffs are in hertz (`3k` is 3000), a level, a mix and the others 0 to 1 (`0.3` or `30%`); `db`, `ms` and `s` are for the time and mix words that take them"));
            }
            if slot.is_some() {
                return Err(Diagnostic::error(name_span, format!("`{w}` is given twice")));
            }
            *slot = Some(v);
            end = v.span;
        }
        let over = self.over()?;
        if let Some((d, sp)) = over {
            if d.is_zero() {
                return Err(Diagnostic::error(sp, "an lfo that runs for no time moves nothing").help("over 16 bars"));
            }
            end = sp;
        }
        // what a value may be
        if let Some(d) = depth {
            if d.value < 0.0 && !crate::perform::is_live(&d) {
                return Err(Diagnostic::error(d.span, "an lfo's depth is not negative").found(d.span.text(self.src).to_string()).help("`phase=0.5` turns the wave over"));
            }
        }
        if let Some(p) = phase {
            if !(0.0..=1.0).contains(&p.value) {
                return Err(Diagnostic::error(p.span, "an lfo's phase is 0 to 1 (a share of one period)").found(p.span.text(self.src).to_string()).help("phase=0.25 starts a quarter of the way round"));
            }
        }
        Ok(LfoSpec { span: start.to(end), wave, wave_span, rate, rate_span, depth, center, phase, over })
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the canonical text
// ---------------------------------------------------------------------------------------------------------------------

/// `lfo sine 2 bars center=3000 depth=1000 over 16 bars`: params sorted by name, one that is its default (depth 1,
/// center 0, phase 0) left out, so two spellings of one lfo are one program.
pub fn print_lfo(l: &LfoSpec) -> String {
    let mut s = format!("lfo {} {}", l.wave.name(), l.rate.canonical());
    for (name, v, default) in [("center", &l.center, 0.0), ("depth", &l.depth, 1.0), ("phase", &l.phase, 0.0)] {
        if let Some(n) = v {
            if n.value != default {
                s.push_str(&format!(" {name}={}", crate::print::numn(n)));
            }
        }
    }
    if let Some((o, _)) = &l.over {
        s.push_str(&format!(" over {}", o.canonical()));
    }
    s
}

/// A control line's shaped change as the canonical text has it (`filter lp 300 -> 2000 over 8 bars exp`).
pub fn print_shaped(name: &Word, mode: Option<&Word>, m: &Motion) -> String {
    let mut s = name.text.clone();
    if let Some(md) = mode {
        s.push_str(&format!(" {}", md.text));
    }
    match m {
        Motion::Ramp { from, to, over, shape, .. } => {
            s.push_str(&format!(" {} -> {} over {}", crate::print::numn(from), crate::print::numn(to), over.0.canonical()));
            if let Some(w) = shape.canonical() {
                s.push(' ');
                s.push_str(&w);
            }
        }
        Motion::Lfo(l) => {
            s.push(' ');
            s.push_str(&print_lfo(l));
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------------
// the compiled lfo
// ---------------------------------------------------------------------------------------------------------------------

/// An lfo as the compiler hands it to the runtime: the registry call it drives (`noun`, `base` with `field` left out),
/// the wave in the field's own unit (the knob for a filter), and how long it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Lfo {
    /// Calls made once when it starts (a filter's `on`).
    pub prelude: Vec<Call>,
    pub noun: String,
    pub base: JsonValue,
    pub field: String,
    pub wave: Wave,
    /// The period, exactly.
    pub period: Dur,
    pub center: f64,
    pub depth: f64,
    /// 0..1 of a period.
    pub phase: f64,
    /// `over dur`: it ends that long after it starts.
    pub over: Option<Dur>,
    /// Where its section ends, in the song's bars (from its first bar line): it stops there. None: it runs until the
    /// program stops or ends (a line outside every section, or a program with no sections).
    pub until: Option<u32>,
}

impl Lfo {
    /// The value `x` bars after it started: `center + depth * wave(x / period + phase)`.
    pub fn value_at(&self, x: f64) -> f64 {
        let p = self.period.to_f64().max(1e-12);
        self.center + self.depth * self.wave.at(x / p + self.phase)
    }

    /// The call that sends the value `x` bars in.
    pub fn call_at(&self, x: f64) -> Call {
        let mut o = self.base.as_object().cloned().unwrap_or_default();
        o.set(self.field.as_str(), JsonValue::Number((self.value_at(x) * 1e6).round() / 1e6));
        Call { noun: self.noun.clone(), params: JsonValue::Object(o) }
    }

    /// The program position (in bars) a run that began at `start` stops at on its own: the earliest of its `over`, the end of its
    /// section (`until`, counted from the program bar `offset` the song's first bar is on) and the end of its `hold`. None: it goes
    /// on until the program stops. What the runtime's [`LfoRun`] stops at, written once.
    pub fn stops_at(&self, start: f64, offset: f64, hold: Option<Dur>) -> Option<f64> {
        let mut stop: Option<f64> = None;
        let mut cut = |x: f64| stop = Some(stop.map_or(x, |s: f64| s.min(x)));
        if let Some(o) = self.over {
            cut(start + o.to_f64());
        }
        if let Some(u) = self.until {
            cut(offset + f64::from(u));
        }
        if let Some(h) = hold {
            cut(start + h.to_f64());
        }
        stop
    }

    /// The program positions (in bars) strictly between `from` and `to` at which the value leaps, for a run that began at `start`:
    /// the edges of a square wave. None for the other waves, which are continuous. (A sampler looks there as well as on its grid.)
    pub fn leaps(&self, start: f64, from: f64, to: f64) -> Vec<f64> {
        if self.wave != Wave::Square {
            return Vec::new();
        }
        let p = self.period.to_f64().max(1e-12);
        // the wave is at an edge where `x / p + phase` is a multiple of 1/2
        let first = ((from - start) / p + self.phase) * 2.0;
        let mut k = first.floor() as i64 + 1;
        let mut out = Vec::new();
        loop {
            let at = start + (k as f64 / 2.0 - self.phase) * p;
            if at >= to - 1e-12 || out.len() >= 4096 {
                break;
            }
            if at > from + 1e-12 {
                out.push(at);
            }
            k += 1;
        }
        out
    }

    /// What the runtime would send if it looked at the lfo at each of `positions` (program bars, ascending) after it began at
    /// `start`, with `hold` as the line's `for`: the same [`LfoRun`] the runtime's pass uses, so a value is sent only when it moved
    /// (the runtime's own threshold) and the last look at the lfo's own end sends the value there, once. For the offline render.
    pub fn sample(&self, start: f64, hold: Option<Dur>, positions: &[f64]) -> Vec<(f64, Call)> {
        let mut run = LfoRun::new(self, start, 0.0, hold);
        let mut out = Vec::new();
        for &p in positions {
            if let Some(c) = run.step(p) {
                out.push((p, c));
            }
            if run.is_done() {
                break;
            }
        }
        out
    }
}

/// An lfo while it runs (runtime.rs): where it started, where it stops, what it last sent.
#[derive(Clone, Debug)]
pub(crate) struct LfoRun {
    lfo: Lfo,
    /// An lfo whose `center=` or `depth=` names an input (F13): read again at every bar line (`retune`).
    live: Option<Box<crate::perform::Performed>>,
    /// The program position (in bars) it began at.
    start: f64,
    /// The program position it stops at: its `over`, its section's end or the end of its hold, whichever comes first.
    stop: Option<f64>,
    last: Option<f64>,
    done: bool,
}

const EPS: f64 = 1e-9;
/// A value is sent only when it moved this far (as a ramp's, `runtime::RAMP_EPS`), or a fraction of the lfo's depth
/// when that is smaller.
const SEND_EPS: f64 = 0.002;

impl LfoRun {
    /// `offset` is the program bar the song's first bar is on (an arranged song counts its bars from there); `hold` the
    /// line's `for` (the lfo stops when what it did is put back).
    pub(crate) fn new(lfo: &Lfo, start: f64, offset: f64, hold: Option<Dur>) -> LfoRun {
        LfoRun { lfo: lfo.clone(), live: None, start, stop: lfo.stops_at(start, offset, hold), last: None, done: false }
    }

    /// Where its section ends has moved (a hot reload that only changed the section's length, under a line whose key did not): the
    /// stop is worked out again from the new `until`, as [`Lfo::stops_at`] has it, for the run that is going on.
    pub(crate) fn restop(&mut self, until: Option<u32>, offset: f64, hold: Option<Dur>) {
        self.lfo.until = until;
        self.stop = self.lfo.stops_at(self.start, offset, hold);
    }

    /// The line that made this run, when it names an input in `center=` or `depth=`.
    pub(crate) fn with_live(mut self, live: Option<Box<crate::perform::Performed>>) -> LfoRun {
        self.live = live.filter(|p| p.lfo.is_some());
        self
    }

    pub(crate) fn live(&self) -> Option<&crate::perform::Performed> {
        self.live.as_deref()
    }

    /// The lfo's middle and reach as the inputs read at this bar line give them: the wave goes on in time, from the new
    /// middle and with the new reach.
    pub(crate) fn retune(&mut self, l: &Lfo) {
        self.lfo.center = l.center;
        self.lfo.depth = l.depth;
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    /// The call to make at program position `pos`, if the value moved: the last pass of its span sends the value
    /// at the end of it, once.
    pub(crate) fn step(&mut self, pos: f64) -> Option<Call> {
        if self.done {
            return None;
        }
        let at_end = self.stop.is_some_and(|e| pos + EPS >= e);
        let at = if at_end { self.stop.unwrap_or(pos) } else { pos };
        let x = (at - self.start).max(0.0);
        let v = self.lfo.value_at(x);
        let eps = (self.lfo.depth.abs() * 0.002).clamp(1e-9, SEND_EPS);
        let send = at_end || self.last.is_none_or(|l| (l - v).abs() >= eps);
        if at_end {
            self.done = true;
        }
        if send {
            self.last = Some(v);
            Some(self.lfo.call_at(x))
        } else {
            None
        }
    }

    /// 0..1: how far through its span (when it has one), else through the period it is in.
    pub(crate) fn progress(&self, pos: f64) -> f64 {
        let x = (pos - self.start).max(0.0);
        match self.stop {
            Some(e) if e > self.start => (x / (e - self.start)).clamp(0.0, 1.0),
            _ => {
                let p = self.lfo.period.to_f64().max(1e-12);
                let c = x / p;
                c - c.floor()
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the compiler
// ---------------------------------------------------------------------------------------------------------------------

/// The range a control line's value may take, as `Compiler::change` checks it: a low-pass cutoff in Hz, a high-pass
/// one, or 0..1.
fn range_of(name: &str, mode: Option<&str>) -> (f64, f64) {
    match (name, mode) {
        ("filter", Some("hp")) => (20.0, 8000.0),
        ("filter", _) => (60.0, 20000.0),
        _ => (0.0, 1.0),
    }
}

impl Compiler<'_> {
    /// A control line's ramp with a shape, or its lfo.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shaped(&mut self, span: Span, target: Option<Target>, tspan: Option<Span>, name: &Word, mode: Option<&Word>, motion: &Motion) -> Option<Effect> {
        match motion {
            Motion::Ramp { from, to, over, shape, .. } => {
                let eff = self.change(span, target, tspan, name, mode, Some(from), Some(to), Some(over))?;
                match eff {
                    Effect::Ramp(mut r) => {
                        r.shape = *shape;
                        Some(Effect::Ramp(r))
                    }
                    other => Some(other),
                }
            }
            Motion::Lfo(l) => {
                // the lfo swings between center - depth and center + depth: the two ends of a ramp, which `change`
                // knows how to check and to turn into the call's own unit (a filter's knob, as a ramp's ends are)
                let (min, max) = range_of(&name.text, mode.map(|m| m.text.as_str()));
                let (c, d) = (l.center_value(), l.depth_value());
                let (mut lo, mut hi) = (c - d, c + d);
                // (a performed centre or depth is checked by what it can be: perform.rs says so, and the swing is held when it plays)
                let performed = l.center.as_ref().is_some_and(crate::perform::is_live) || l.depth.as_ref().is_some_and(crate::perform::is_live);
                if lo < min || hi > max {
                    if !performed {
                        let what = match (name.text.as_str(), mode.map(|m| m.text.as_str())) {
                            ("filter", Some(m)) => format!("a {} cutoff", if m == "hp" { "high-pass" } else { "low-pass" }),
                            (n, _) => format!("`{n}`"),
                        };
                        self.diags.push(
                            Diagnostic::warning(l.span, format!("this lfo swings from {} to {}, outside the range of {what} ({} to {}): it is held at the edge", crate::print::num(lo), crate::print::num(hi), crate::print::num(min), crate::print::num(max)))
                                .help("`center=` is the middle and `depth=` how far it goes either way (the default depth is 1)"),
                        );
                    }
                    lo = lo.max(min).min(max);
                    hi = hi.min(max).max(min);
                }
                // a depth left out is 1 in the target's own unit: a whole swing of a 0..1 knob, but a hertz either side of a cutoff,
                // which nobody hears (L-17): said, with the one thing to write
                if l.depth.is_none() && max - min > 1.0 && !performed {
                    let what = match (name.text.as_str(), mode.map(|m| m.text.as_str())) {
                        ("filter", Some(m)) => format!("a {} cutoff in hertz", if m == "hp" { "high-pass" } else { "low-pass" }),
                        (n, _) => format!("`{n}`"),
                    };
                    self.diags.push(
                        Diagnostic::warning(l.span, format!("this lfo has no `depth=`, so it swings {} either side of its centre: next to {what} that is no movement at all", crate::print::num(d)))
                            .help("write how far it goes either way in the target's own unit: `lfo sine 2 bars center=3k depth=1k` swings a cutoff from 2000 to 4000 Hz"),
                    );
                }
                let (lo_n, hi_n) = (Num::lit(lo, false, l.span), Num::lit(hi, false, l.span));
                let eff = self.change(span, target, tspan, name, mode, Some(&lo_n), Some(&hi_n), Some(&(Dur::bars(1), l.span)))?;
                let Effect::Ramp(r) = eff else { return None };
                Some(Effect::Lfo(Lfo {
                    prelude: r.prelude,
                    noun: r.noun,
                    base: r.base,
                    field: r.field,
                    wave: l.wave,
                    period: l.rate,
                    center: (r.from + r.to) / 2.0,
                    depth: (r.to - r.from) / 2.0,
                    phase: l.phase_value(),
                    over: l.over.map(|(d, _)| d),
                    until: self.seg_end,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn shapes_at_the_quarters_are_the_hand_formulas() {
        let ts = [0.0, 0.25, 0.5, 0.75, 1.0];
        let k = |t: f64| (2f64.powf(4.0 * t) - 1.0) / 15.0;
        for t in ts {
            assert!(close(Shape::Linear.apply(t), t));
            assert!(close(Shape::Exp.apply(t), k(t)));
            assert!(close(Shape::Log.apply(t), 1.0 - k(1.0 - t)));
            assert!(close(Shape::SCurve.apply(t), 3.0 * t * t - 2.0 * t * t * t));
        }
        // by hand: exp 1/15, 1/5, 7/15; log is its mirror 8/15, 4/5, 14/15; the s-curve 5/32, 1/2, 27/32
        for (t, e) in [(0.25, 1.0 / 15.0), (0.5, 0.2), (0.75, 7.0 / 15.0)] {
            assert!(close(Shape::Exp.apply(t), e), "{t}");
            assert!(close(Shape::Log.apply(1.0 - t), 1.0 - e), "{t}");
        }
        assert!(close(Shape::SCurve.apply(0.25), 5.0 / 32.0) && close(Shape::SCurve.apply(0.75), 27.0 / 32.0));
        // the ends are exact
        for s in [Shape::Linear, Shape::Exp, Shape::Log, Shape::SCurve, Shape::Step(5)] {
            assert_eq!((s.apply(0.0), s.apply(1.0)), (0.0, 1.0), "{s:?}");
        }
        // exp starts slow and log starts fast
        assert!(Shape::Exp.apply(0.5) < 0.5 && Shape::Log.apply(0.5) > 0.5);
    }

    #[test]
    fn step_quantises_to_n_values() {
        let s = Shape::Step(4);
        let at: Vec<f64> = [0.0, 0.24, 0.25, 0.49, 0.5, 0.74, 0.75, 0.99, 1.0].iter().map(|t| s.apply(*t)).collect();
        let third = 1.0 / 3.0;
        assert_eq!(at, vec![0.0, 0.0, third, third, 2.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0]);
        // two steps: the start, then the end
        assert_eq!((Shape::Step(2).apply(0.49), Shape::Step(2).apply(0.5)), (0.0, 1.0));
        // a t that is exactly k / n by hand is the k-th step even where a float would round under it
        for n in 2..40u32 {
            for k in 0..n {
                let t = f64::from(k) / f64::from(n);
                assert_eq!(Shape::Step(n).apply(t), f64::from(k) / f64::from(n - 1), "{k}/{n}");
            }
        }
    }

    #[test]
    fn waves_at_known_phases() {
        let q = |w: Wave, p: f64| (w.at(p) * 1e12).round() / 1e12;
        // sine
        assert_eq!([q(Wave::Sine, 0.0), q(Wave::Sine, 0.25), q(Wave::Sine, 0.5), q(Wave::Sine, 0.75), q(Wave::Sine, 1.0)], [0.0, 1.0, 0.0, -1.0, 0.0]);
        // tri: the same zeros and peaks as the sine
        assert_eq!([q(Wave::Tri, 0.0), q(Wave::Tri, 0.125), q(Wave::Tri, 0.25), q(Wave::Tri, 0.5), q(Wave::Tri, 0.75), q(Wave::Tri, 0.875), q(Wave::Tri, 1.0)], [0.0, 0.5, 1.0, 0.0, -1.0, -0.5, 0.0]);
        // square: high for the first half, low for the second
        assert_eq!([q(Wave::Square, 0.0), q(Wave::Square, 0.25), q(Wave::Square, 0.49), q(Wave::Square, 0.5), q(Wave::Square, 0.75), q(Wave::Square, 0.99)], [1.0, 1.0, 1.0, -1.0, -1.0, -1.0]);
        // saw rises 0 -> 1, falls to -1 at the half and rises to 0 again
        assert_eq!([q(Wave::Saw, 0.0), q(Wave::Saw, 0.25), q(Wave::Saw, 0.5), q(Wave::Saw, 0.75), q(Wave::Saw, 1.0)], [0.0, 0.5, -1.0, -0.5, 0.0]);
        assert!(Wave::Saw.at(0.499999) > 0.999);
        // only the fraction of the phase counts, a negative one too
        for w in [Wave::Sine, Wave::Tri, Wave::Square, Wave::Saw] {
            for p in [0.1, 0.3, 0.6, 0.9] {
                assert!(close(w.at(p), w.at(p + 3.0)) && close(w.at(p), w.at(p - 2.0)), "{w:?} {p}");
            }
            assert!((-1.0..=1.0).contains(&w.at(0.37)));
        }
    }

    fn lfo(wave: Wave, period: u32, center: f64, depth: f64) -> Lfo {
        Lfo { prelude: vec![], noun: "mixer".into(), base: JsonValue::parse("{}").unwrap(), field: "master".into(), wave, period: Dur::bars(period), center, depth, phase: 0.0, over: None, until: None }
    }

    #[test]
    fn the_lfo_is_center_plus_depth_times_the_wave() {
        let l = lfo(Wave::Sine, 2, 0.5, 0.25);
        // a period is 2 bars: a quarter of it is half a bar
        assert!(close(l.value_at(0.0), 0.5) && close(l.value_at(0.5), 0.75) && close(l.value_at(1.0), 0.5) && close(l.value_at(1.5), 0.25) && close(l.value_at(2.0), 0.5));
        // a phase moves the start along the wave
        let q = Lfo { phase: 0.25, ..l.clone() };
        assert!(close(q.value_at(0.0), 0.75) && close(q.value_at(1.0), 0.25));
        let c = l.call_at(0.5);
        assert_eq!(c.params.get("master").and_then(JsonValue::as_f64), Some(0.75));
    }

    #[test]
    fn a_run_sends_what_moved_and_the_value_at_the_end_once() {
        let l = Lfo { over: Some(Dur::bars(2)), ..lfo(Wave::Tri, 1, 0.5, 0.5) };
        let mut r = LfoRun::new(&l, 10.0, 0.0, None);
        let v = |c: Option<Call>| c.and_then(|c| c.params.get("master").and_then(JsonValue::as_f64));
        assert_eq!(v(r.step(10.0)), Some(0.5));
        // nothing moved: nothing sent
        assert_eq!(v(r.step(10.0)), None);
        assert_eq!(v(r.step(10.25)), Some(1.0));
        // the last pass of the span sends the value at the end of it: two whole periods in is the start value again
        assert_eq!(v(r.step(12.0)), Some(0.5));
        assert!(r.is_done());
        assert_eq!(v(r.step(12.5)), None);
    }

    #[test]
    fn a_section_end_stops_it_and_the_hold_and_over_cut_it_shorter() {
        let l = Lfo { until: Some(8), ..lfo(Wave::Sine, 4, 0.5, 0.5) };
        // a song that starts on program bar 100: the section ends on 108
        let r = LfoRun::new(&l, 104.0, 100.0, None);
        assert_eq!(r.stop, Some(108.0));
        assert_eq!(LfoRun::new(&l, 104.0, 100.0, Some(Dur::bars(2))).stop, Some(106.0));
        assert_eq!(LfoRun::new(&Lfo { over: Some(Dur::bars(1)), ..l.clone() }, 104.0, 100.0, None).stop, Some(105.0));
        // with no end it runs on, and its progress is how far through its period it is
        let free = Lfo { until: None, ..l };
        let r = LfoRun::new(&free, 0.0, 0.0, None);
        assert_eq!(r.stop, None);
        assert!(close(r.progress(1.0), 0.25) && close(r.progress(5.0), 0.25));
    }

    /// What the offline render samples with (`djhost::btw_render`): the places a shape leaps, an lfo's edges, where it stops, and the
    /// calls the runtime's own run would make when looked at in turn.
    #[test]
    fn the_places_a_value_leaps_and_the_samples_of_a_run_are_the_runtimes_own() {
        // `step n` leaps at k / n; the smooth shapes do not leap
        assert_eq!(Shape::Step(4).breaks(), vec![0.25, 0.5, 0.75]);
        assert_eq!(Shape::Step(2).breaks(), vec![0.5]);
        assert!(Shape::Linear.breaks().is_empty() && Shape::Exp.breaks().is_empty() && Shape::Log.breaks().is_empty() && Shape::SCurve.breaks().is_empty());
        for n in 2..20 {
            for t in Shape::Step(n).breaks() {
                // the leap is on the beat: just before it the value is one step behind, at it the value has leapt
                assert!(Shape::Step(n).apply(t) > Shape::Step(n).apply(t - 1e-6), "{n} {t}");
            }
        }
        // a square wave leaps at every half period (moved by its phase), the others never
        let sq = Lfo { phase: 0.0, ..lfo(Wave::Square, 2, 0.5, 0.5) };
        assert_eq!(sq.leaps(10.0, 10.0, 14.0), vec![11.0, 12.0, 13.0]);
        let ph = Lfo { phase: 0.25, ..sq.clone() };
        assert_eq!(ph.leaps(10.0, 10.0, 14.0), vec![10.5, 11.5, 12.5, 13.5]);
        assert!(lfo(Wave::Sine, 2, 0.5, 0.5).leaps(10.0, 10.0, 14.0).is_empty());
        // (and the wave really does change there)
        assert_eq!((ph.value_at(0.49), ph.value_at(0.5)), (1.0, 0.0));
        // where a run stops by itself: the earliest of over, the section's end and the hold
        let l = Lfo { until: Some(8), over: Some(Dur::bars(3)), ..lfo(Wave::Sine, 4, 0.5, 0.5) };
        assert_eq!(l.stops_at(104.0, 100.0, None), Some(107.0));
        assert_eq!(l.stops_at(104.0, 100.0, Some(Dur::bars(1))), Some(105.0));
        assert_eq!(lfo(Wave::Sine, 4, 0.5, 0.5).stops_at(0.0, 0.0, None), None);
        // `sample` is a run looked at in turn: what moved is sent, the stop sends the end once and nothing after it
        let l = Lfo { over: Some(Dur::bars(2)), ..lfo(Wave::Tri, 1, 0.5, 0.5) };
        let at = |v: Vec<(f64, Call)>| v.into_iter().map(|(p, c)| (p, c.params.get("master").and_then(JsonValue::as_f64).unwrap())).collect::<Vec<_>>();
        assert_eq!(at(l.sample(10.0, None, &[10.0, 10.0, 10.25, 11.0, 12.0, 12.5])), vec![(10.0, 0.5), (10.25, 1.0), (11.0, 0.5), (12.0, 0.5)]);
    }
}
