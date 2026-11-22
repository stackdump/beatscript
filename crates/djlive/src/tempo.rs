//! F3 tempo curves and the exact tempo map (docs/btw/ext-tempo.md).
//!
//! ```text
//! setting     = "tempo" value [ "->" value "over" dur [ shape ] ] ;     (* the program's first bar *)
//! section-set = the same, a statement inside a section: from its first bar ;
//! timed       = when ":" "tempo" value [ "->" value "over" dur [ shape ] ] ;   (* at bar 16: tempo 128 -> 140 over 4 bars *)
//! ```
//!
//! BPM moves **linearly in beats** (shaped by `shape`), so the bar clock never moves: a bar is four beats at any
//! tempo, and every timing word of the language (`at bar 16 beat 3`, `every 2 beats`, `over 1/2 bar`) is in bars and
//! beats. What a curve changes is the seconds: they are the integral of `60 / bpm` over the beats, in closed form per
//! piece (a constant `a` over `L` beats is `60 L / a`; a linear ramp `a -> b` is `60 L ln(b / a) / (b - a)`; `exp`,
//! `log` and `step` have their closed forms too, the s-curve a numerical one that is exact to the last digit). The
//! [`TempoMap`] is built once, when the program is compiled (sections and timed lines included), and answers
//! [`TempoMap::seconds_at`], [`TempoMap::bar_at`], [`TempoMap::total_seconds`] and [`TempoMap::bpm_at`]; it is
//! [`Compiled::tempo_map`]. `check` reports the song's length through it, the evolve report's seconds use it, the
//! offline render (`djhost::btw_render`) follows it, and the runtime sets the tempo each service pass from the program's
//! position ([`crate::runtime::LiveHost::set_tempo`]).

use crate::ast::{Action, Body, Line, Num, Setting, Stmt, StmtKind};
use crate::compile::{Compiled, Compiler, Timing};
use crate::diag::{Diagnostic, Span};
use crate::dur::{Dur, Pos};
use crate::expand::{FlatLine, Lower, Scope, SegmentLines};
use crate::lex::Tok;
use crate::parse::Parser;
use crate::rat::Rat;
use crate::shape::Shape;

type R<T> = Result<T, Diagnostic>;

/// The tempo bounds of the language, in BPM.
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 300.0;
/// The tempo the map counts in before the program's first tempo (the decks keep theirs there; a length has to be
/// worked out in something): the one `evolve`'s report always used.
pub const DEFAULT_BPM: f64 = 120.0;
/// The tempo the runtime sends only when it moved this far (BPM).
pub const SEND_EPS: f64 = 0.01;

// ---------------------------------------------------------------------------------------------------------------------
// the tree
// ---------------------------------------------------------------------------------------------------------------------

/// `100 -> 130 over 8 bars exp`, as written.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoRamp {
    pub from: Num,
    pub to: Num,
    pub over: Dur,
    pub over_span: Span,
    pub shape: Shape,
    pub span: Span,
}

/// What a `tempo` says: a value, or a ramp.
#[derive(Clone, Debug, PartialEq)]
pub enum TempoSpec {
    Set(Num),
    Ramp(TempoRamp),
}

/// The canonical text of what follows `tempo`: `126`, `100 -> 130 over 8 bars exp`.
pub fn print_spec(s: &TempoSpec) -> String {
    match s {
        TempoSpec::Set(n) => crate::print::numn(n),
        TempoSpec::Ramp(r) => print_ramp(r),
    }
}

/// `100 -> 130 over 8 bars exp`.
pub fn print_ramp(r: &TempoRamp) -> String {
    let mut t = format!("{} -> {} over {}", crate::print::numn(&r.from), crate::print::numn(&r.to), r.over.canonical());
    if let Some(w) = r.shape.canonical() {
        t.push(' ');
        t.push_str(&w);
    }
    t
}

impl Parser<'_> {
    /// What follows `tempo`: `value`, or `value -> value over dur [shape]`.
    pub(crate) fn tempo_spec(&mut self) -> R<TempoSpec> {
        let start = self.span();
        let from = self.num("a tempo in BPM")?;
        if !self.eat(&Tok::Arrow) {
            if self.word_is("over") {
                return Err(Diagnostic::error(self.span(), "`over` has nothing to move: write `tempo 100 -> 130 over 8 bars`").found("`over`"));
            }
            if let Some((shape, sp)) = self.shape_opt()? {
                return Err(Diagnostic::error(sp, format!("`{}` needs a ramp", shape.name())).help("tempo 100 -> 130 over 8 bars exp"));
            }
            return Ok(TempoSpec::Set(from));
        }
        let to = self.num("the tempo to move to")?;
        if !self.word_is("over") {
            return Err(self.expected(&["`over`"], &format!("a tempo ramp needs a length, found {}", self.found())).help("tempo 100 -> 130 over 8 bars"));
        }
        let Some((over, over_span)) = self.over()? else { unreachable!("`over` was the next word") };
        if over.is_zero() {
            return Err(Diagnostic::error(over_span, "a tempo ramp takes some time").help("over 8 bars, over 2 beats"));
        }
        let shape = match self.shape_opt()? {
            Some((s, _)) => s,
            None => Shape::Linear,
        };
        Ok(TempoSpec::Ramp(TempoRamp { from, to, over, over_span, shape, span: start.to(self.prev_span()) }))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the map
// ---------------------------------------------------------------------------------------------------------------------

/// One tempo of the program, where it begins: it jumps to `from` at `at`, then (for a ramp, `over` > 0) moves to `to`
/// over `over` bars along `shape`, then holds `to` until the next event.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoEvent {
    /// Where, in bars from the program's first bar (exact).
    pub at: Rat,
    pub from: f64,
    pub to: f64,
    /// The ramp's length in bars; zero for a plain tempo.
    pub over: Rat,
    pub shape: Shape,
    /// The text it came from (for the diagnostics about it).
    pub span: Span,
}

impl TempoEvent {
    /// A plain `tempo N` at `at`.
    pub fn set(at: Rat, bpm: f64, span: Span) -> TempoEvent {
        TempoEvent { at, from: bpm, to: bpm, over: Rat::ZERO, shape: Shape::Linear, span }
    }

    /// A ramp `from -> to` over `over` bars from `at`.
    pub fn ramp(at: Rat, from: f64, to: f64, over: Rat, shape: Shape, span: Span) -> TempoEvent {
        TempoEvent { at, from, to, over, shape, span }
    }
}

/// Anything that is a number of bars: a bar count, a position, a length.
pub trait IntoBars {
    fn into_bars(self) -> Rat;
}

impl IntoBars for Rat {
    fn into_bars(self) -> Rat {
        self
    }
}
impl IntoBars for u32 {
    fn into_bars(self) -> Rat {
        Rat::int(i64::from(self))
    }
}
impl IntoBars for i64 {
    fn into_bars(self) -> Rat {
        Rat::int(self)
    }
}
impl IntoBars for usize {
    fn into_bars(self) -> Rat {
        Rat::int(self as i64)
    }
}
impl IntoBars for Dur {
    fn into_bars(self) -> Rat {
        self.as_bars()
    }
}
impl IntoBars for Pos {
    fn into_bars(self) -> Rat {
        self.as_bars()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Hold(f64),
    /// `a` to `b` along `shape` over `len_beats` beats; a ramp that the next tempo cuts short is a piece shorter than
    /// that.
    Ramp { a: f64, b: f64, len_beats: f64, shape: Shape },
}

#[derive(Clone, Debug, PartialEq)]
struct Piece {
    start: Rat,
    /// None: for ever (the last piece).
    end: Option<Rat>,
    kind: Kind,
    /// Seconds from bar 0 to `start`.
    secs0: f64,
    /// Before the program's first tempo: the map's own assumption.
    assumed: bool,
}

/// The program's tempo as a function of the bar: pieces, each a constant or a ramp, with the seconds integrated in
/// closed form. Exact in bars (`Rat`), `f64` in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    events: Vec<TempoEvent>,
    pieces: Vec<Piece>,
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap::new(Vec::new())
    }
}

impl TempoMap {
    /// The map of these events, in any order (the order given breaks a tie: of two at one bar the later wins).
    pub fn new(events: Vec<TempoEvent>) -> TempoMap {
        // sorted by place, one to a place (the last said wins)
        let mut sorted: Vec<TempoEvent> = Vec::new();
        for e in events {
            match sorted.iter().position(|x| x.at == e.at) {
                Some(i) => sorted[i] = e,
                None => sorted.push(e),
            }
        }
        sorted.sort_by_key(|e| e.at);
        let mut pieces: Vec<Piece> = Vec::new();
        let first_at = sorted.first().map(|e| e.at);
        if first_at.is_none_or(|a| a > Rat::ZERO) {
            pieces.push(Piece { start: Rat::ZERO, end: first_at, kind: Kind::Hold(DEFAULT_BPM), secs0: 0.0, assumed: true });
        }
        for (i, e) in sorted.iter().enumerate() {
            let next = sorted.get(i + 1).map(|n| n.at);
            if e.over.is_zero() {
                pieces.push(Piece { start: e.at, end: next, kind: Kind::Hold(e.to), secs0: 0.0, assumed: false });
                continue;
            }
            let ramp_end = e.at.add(e.over).unwrap_or(e.at);
            let cut = next.is_some_and(|n| n < ramp_end);
            let end = if cut { next } else { Some(ramp_end) };
            let len_beats = e.over.mul(Rat::int(4)).map_or(4.0, Rat::to_f64);
            pieces.push(Piece { start: e.at, end, kind: Kind::Ramp { a: e.from, b: e.to, len_beats, shape: e.shape }, secs0: 0.0, assumed: false });
            if !cut && next.is_none_or(|n| n > ramp_end) {
                pieces.push(Piece { start: ramp_end, end: next, kind: Kind::Hold(e.to), secs0: 0.0, assumed: false });
            }
        }
        // the seconds at each piece's start
        let mut secs = 0.0;
        for p in &mut pieces {
            p.secs0 = secs;
            if let Some(end) = p.end {
                let len = end.sub(p.start).map_or(0.0, Rat::to_f64);
                secs += p.secs_to(len);
            }
        }
        TempoMap { events: sorted, pieces }
    }

    /// A tempo that never changes.
    pub fn constant(bpm: f64) -> TempoMap {
        TempoMap::new(vec![TempoEvent::set(Rat::ZERO, bpm, Span::default())])
    }

    /// The tempos of the program, in order of place.
    pub fn events(&self) -> &[TempoEvent] {
        &self.events
    }

    /// The two maps say the same tempos in the same places (the text they came from may differ).
    pub fn same_tempos(&self, other: &TempoMap) -> bool {
        self.events.len() == other.events.len() && self.events.iter().zip(&other.events).all(|(a, b)| (a.at, a.from, a.to, a.over, a.shape) == (b.at, b.from, b.to, b.over, b.shape))
    }

    /// The program says no tempo at all (the decks keep theirs).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The tempo the program says is one number from its first bar to its last.
    pub fn constant_bpm(&self) -> Option<f64> {
        let first = self.pieces.first()?;
        if first.assumed {
            return None;
        }
        let mut bpm: Option<f64> = None;
        for p in &self.pieces {
            let b = match p.kind {
                Kind::Hold(b) => b,
                Kind::Ramp { a, b, .. } if a == b => a,
                Kind::Ramp { .. } => return None,
            };
            match bpm {
                None => bpm = Some(b),
                Some(x) if x == b => {}
                Some(_) => return None,
            }
        }
        bpm
    }

    /// One tempo from the first bar on (what a program with `tempo 126` has).
    pub fn is_constant(&self) -> bool {
        self.constant_bpm().is_some()
    }

    /// The tempo changes in the course of the program, so the runtime has to set it as it goes.
    pub fn drives(&self) -> bool {
        !self.is_empty() && !self.is_constant()
    }

    /// Index of the piece that holds `bar`.
    fn locate(&self, bar: Rat) -> usize {
        self.pieces.iter().rposition(|p| p.start <= bar).unwrap_or(0)
    }

    fn locate_f(&self, bars: f64) -> usize {
        self.pieces.iter().rposition(|p| p.start.to_f64() <= bars + 1e-12).unwrap_or(0)
    }

    /// The tempo the program has said by `bar`: None before its first tempo.
    pub fn declared_at(&self, bar: impl IntoBars) -> Option<f64> {
        let bar = bar.into_bars();
        let p = &self.pieces[self.locate(bar)];
        if p.assumed {
            return None;
        }
        let x = bar.sub(p.start).map_or(0.0, Rat::to_f64);
        Some(p.bpm_at(x))
    }

    /// [`declared_at`](Self::declared_at) for a position that is a float (the runtime's).
    pub fn declared_at_f(&self, bars: f64) -> Option<f64> {
        let p = &self.pieces[self.locate_f(bars)];
        if p.assumed {
            return None;
        }
        Some(p.bpm_at((bars - p.start.to_f64()).max(0.0)))
    }

    /// The tempo at `bar`: [`DEFAULT_BPM`] before the program's first tempo.
    pub fn bpm_at(&self, bar: impl IntoBars) -> f64 {
        self.declared_at(bar).unwrap_or(DEFAULT_BPM)
    }

    /// Seconds from bar 0 to `bar`: the integral of `60 / bpm` over the beats.
    pub fn seconds_at(&self, bar: impl IntoBars) -> f64 {
        let bar = bar.into_bars();
        if bar.is_neg() {
            return 0.0;
        }
        let p = &self.pieces[self.locate(bar)];
        p.secs0 + p.secs_to(bar.sub(p.start).map_or(0.0, Rat::to_f64))
    }

    /// [`seconds_at`](Self::seconds_at) for a position that is a float.
    pub fn seconds_at_f(&self, bars: f64) -> f64 {
        if bars <= 0.0 {
            return 0.0;
        }
        let p = &self.pieces[self.locate_f(bars)];
        p.secs0 + p.secs_to((bars - p.start.to_f64()).max(0.0))
    }

    /// Seconds from bar `a` to bar `b`.
    pub fn seconds_between(&self, a: impl IntoBars, b: impl IntoBars) -> f64 {
        self.seconds_at(b) - self.seconds_at(a)
    }

    /// The length of a song of `bars` bars, in seconds (the same as `seconds_at`).
    pub fn total_seconds(&self, bars: impl IntoBars) -> f64 {
        self.seconds_at(bars)
    }

    /// The bar (a fraction of one) that falls `seconds` in: the inverse of [`seconds_at`](Self::seconds_at).
    pub fn bar_at(&self, seconds: f64) -> f64 {
        if seconds <= 0.0 || seconds.is_nan() {
            return 0.0;
        }
        let i = self.pieces.iter().rposition(|p| p.secs0 <= seconds).unwrap_or(0);
        let p = &self.pieces[i];
        let len = p.end.map(|e| e.sub(p.start).map_or(0.0, Rat::to_f64));
        p.start.to_f64() + p.bars_for(seconds - p.secs0, len)
    }

    /// Same as [`bar_at`](Self::bar_at).
    pub fn bars_at(&self, seconds: f64) -> f64 {
        self.bar_at(seconds)
    }

    /// The tempo as a staircase over the first `beats` beats, for a renderer that has to play constant tempos: each
    /// step is `(start beat, bpm)` and its bpm is the constant tempo that takes exactly the seconds the real curve takes
    /// over the step (`60 * beats / seconds`), so the steps add up to the exact length and every step's start lands on
    /// the exact second. A constant piece is one step; a ramp is cut into steps of `step_beats`.
    pub fn staircase(&self, beats: f64, step_beats: f64) -> Vec<(f64, f64)> {
        let step = step_beats.max(1.0 / 64.0);
        let mut out: Vec<(f64, f64)> = Vec::new();
        for p in &self.pieces {
            let (s0, s1) = (p.start.to_f64() * 4.0, p.end.map_or(f64::INFINITY, |e| e.to_f64() * 4.0));
            if s0 >= beats {
                break;
            }
            let hi = s1.min(beats);
            match p.kind {
                Kind::Hold(b) => out.push((s0, b)),
                Kind::Ramp { .. } => {
                    let n = (((hi - s0) / step).ceil() as usize).max(1);
                    for k in 0..n {
                        let (b0, b1) = (s0 + k as f64 * step, (s0 + (k + 1) as f64 * step).min(hi));
                        let secs = p.secs_to((b1 - s0) / 4.0) - p.secs_to((b0 - s0) / 4.0);
                        out.push((b0, if secs > 0.0 { 60.0 * (b1 - b0) / secs } else { p.bpm_at((b0 - s0) / 4.0) }));
                    }
                }
            }
        }
        out
    }

    /// The lowest and highest tempo the program says (the assumed 120 before its first tempo does not count).
    pub fn range(&self) -> Option<(f64, f64)> {
        let mut r: Option<(f64, f64)> = None;
        for p in self.pieces.iter().filter(|p| !p.assumed) {
            let (a, b) = match p.kind {
                Kind::Hold(b) => (b, b),
                Kind::Ramp { a, b, .. } => (a, b),
            };
            // (a cut ramp does not reach its end, but it is a bound, and `step` / `s-curve` stay between the two)
            let (lo, hi) = (a.min(b), a.max(b));
            r = Some(r.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
        }
        r
    }
}

impl Piece {
    /// The tempo `x` bars into this piece.
    fn bpm_at(&self, x: f64) -> f64 {
        match self.kind {
            Kind::Hold(b) => b,
            Kind::Ramp { a, b, len_beats, shape } => a + (b - a) * shape.apply(4.0 * x / len_beats),
        }
    }

    /// Seconds from the piece's start to `x` bars into it.
    fn secs_to(&self, x: f64) -> f64 {
        match self.kind {
            Kind::Hold(b) => 240.0 * x / b,
            Kind::Ramp { a, b, len_beats, shape } => {
                let t = (4.0 * x / len_beats).clamp(0.0, 1.0);
                60.0 * len_beats * ramp_integral(a, b, shape, t)
            }
        }
    }

    /// The bars (from the piece's start) that take `secs` seconds; `len` is the piece's length (None: for ever).
    fn bars_for(&self, secs: f64, len: Option<f64>) -> f64 {
        match self.kind {
            Kind::Hold(b) => secs * b / 240.0,
            Kind::Ramp { .. } => {
                let top = len.unwrap_or(1e12);
                // the seconds grow with the bars: bisect to the last digit
                let (mut lo, mut hi) = (0.0, top);
                for _ in 0..200 {
                    let mid = 0.5 * (lo + hi);
                    if self.secs_to(mid) < secs {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                    if hi - lo <= 1e-15 * hi.max(1.0) {
                        break;
                    }
                }
                0.5 * (lo + hi)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the integrals
// ---------------------------------------------------------------------------------------------------------------------

/// `ln(1 + z) / z`, with its series where `z` is too small to divide by.
fn ln1p_over(z: f64) -> f64 {
    if z.abs() < 1e-6 {
        1.0 - z / 2.0 + z * z / 3.0
    } else {
        z.ln_1p() / z
    }
}

/// `integral from 0 to t of du / bpm(u)`, `bpm(u) = a + (b - a) shape(u)`, per beat-fraction `u`: multiplied by
/// `60 * L` it is the seconds of the first `t` of a ramp `L` beats long.
fn ramp_integral(a: f64, b: f64, shape: Shape, t: f64) -> f64 {
    let d = b - a;
    let k = crate::shape::EXP_K;
    let lam = k * std::f64::consts::LN_2;
    let c = k.exp2() - 1.0;
    match shape {
        Shape::Linear => t / a * ln1p_over(d * t / a),
        Shape::Exp => {
            // bpm = p + q e^(lam u), p = a - d / c, q = d / c: the integral is (1/p) (t - ln(bpm(t) / a) / lam)
            let p = a - d / c;
            if p <= 0.0 {
                return gauss(|u| 1.0 / (a + d * shape.apply(u)), t);
            }
            let bt = a + d * shape.apply(t);
            (t - (bt / a).ln() / lam) / p
        }
        Shape::Log => {
            // the mirror: bpm = p' + q' e^(lam w) in w = 1 - u, p' = b + d / c
            let p = b + d / c;
            if p <= 0.0 {
                return gauss(|u| 1.0 / (a + d * shape.apply(u)), t);
            }
            let bt = a + d * shape.apply(t);
            (t - (a / bt).ln() / lam) / p
        }
        Shape::SCurve => gauss(|u| 1.0 / (a + d * shape.apply(u)), t),
        Shape::Step(n) => {
            // n constant stretches of 1/n of the ramp each, the j-th at a + d j / (n - 1) (the last two the same)
            let n = n.max(2);
            let nf = f64::from(n);
            let mut sum = 0.0;
            for j in 0..n {
                let (lo, hi) = (f64::from(j) / nf, (f64::from(j) + 1.0) / nf);
                let seg = hi.min(t) - lo;
                if seg <= 0.0 {
                    break;
                }
                sum += seg / (a + d * (f64::from(j) / (nf - 1.0)).min(1.0));
            }
            sum
        }
    }
}

/// `integral from 0 to t of f(u) du`: 16 panels of 8-point Gauss-Legendre, exact to the last digit for a smooth `f`.
fn gauss(f: impl Fn(f64) -> f64, t: f64) -> f64 {
    const X: [f64; 4] = [0.183_434_642_495_649_8, 0.525_532_409_916_329, 0.796_666_477_413_626_7, 0.960_289_856_497_536_3];
    const W: [f64; 4] = [0.362_683_783_378_362, 0.313_706_645_877_887_3, 0.222_381_034_453_374_5, 0.101_228_536_290_376_3];
    const PANELS: usize = 16;
    let h = t / PANELS as f64;
    let mut sum = 0.0;
    for p in 0..PANELS {
        let mid = (p as f64 + 0.5) * h;
        for i in 0..4 {
            sum += W[i] * (f(mid + X[i] * 0.5 * h) + f(mid - X[i] * 0.5 * h));
        }
    }
    sum * 0.5 * h
}

// ---------------------------------------------------------------------------------------------------------------------
// the program: lines to events
// ---------------------------------------------------------------------------------------------------------------------

impl Lower<'_> {
    /// A `tempo` statement inside a section (from its first bar), a `def` or a `repeat` (from where that body starts):
    /// it becomes a timed line, `Action::Tempo`, which the compiler reads as an event of the tempo map.
    pub(crate) fn tempo_stmt(&mut self, st: &Stmt, sc: &Scope, out: &mut Vec<FlatLine>, segs: &mut Vec<SegmentLines>) {
        let spec = match &st.kind {
            StmtKind::Setting(Setting::Tempo(n)) => TempoSpec::Set(*n),
            StmtKind::Setting(Setting::TempoRamp(r)) => TempoSpec::Ramp(r.clone()),
            _ => return,
        };
        let line = Line { when: None, hold: None, target: None, control: None, body: Body::Action(Action::Tempo(spec)), guard: None };
        self.line(st.span, &line, sc, out, segs);
    }
}

/// The event a `tempo` says at `at`, or None where a value is out of range (said by [`Compiler::tempo_line`], not here).
fn quiet_event(span: Span, at: Rat, spec: &TempoSpec) -> Option<TempoEvent> {
    let ok = |n: &Num| (MIN_BPM..=MAX_BPM).contains(&n.value);
    match spec {
        TempoSpec::Set(n) => ok(n).then(|| TempoEvent::set(at, n.value, span)),
        TempoSpec::Ramp(r) => (ok(&r.from) && ok(&r.to)).then(|| TempoEvent::ramp(at, r.from.value, r.to.value, r.over.as_bars(), r.shape, span)),
    }
}

impl Compiler<'_> {
    /// A tempo is 40 to 300 BPM.
    pub(crate) fn tempo_value(&mut self, n: &Num) -> bool {
        if (MIN_BPM..=MAX_BPM).contains(&n.value) {
            return true;
        }
        self.err(
            Diagnostic::error(n.span, format!("a tempo is {} to {} BPM", MIN_BPM as u32, MAX_BPM as u32))
                .found(n.span.text(self.src).to_string())
                .help("tempo 126   (nothing slower than 40, nothing faster than 300)"),
        );
        false
    }

    /// A top-level `tempo` setting: an event at the program's first bar. Answers the tempo it starts at.
    pub(crate) fn tempo_setting(&mut self, span: Span, spec: &TempoSpec) -> Option<f64> {
        let ev = self.tempo_event(span, Rat::ZERO, spec)?;
        let first = ev.from;
        self.tempos.push(ev);
        Some(first)
    }

    /// The event a `tempo` says at `at`, or None (said why) when a value is out of range.
    fn tempo_event(&mut self, span: Span, at: Rat, spec: &TempoSpec) -> Option<TempoEvent> {
        match spec {
            TempoSpec::Set(n) => self.tempo_value(n).then(|| TempoEvent::set(at, n.value, span)),
            TempoSpec::Ramp(r) => {
                let (a, b) = (self.tempo_value(&r.from), self.tempo_value(&r.to));
                (a && b).then(|| TempoEvent::ramp(at, r.from.value, r.to.value, r.over.as_bars(), r.shape, span))
            }
        }
    }

    /// A tempo on a timed line (`at bar 16: tempo 128 -> 140 over 4 bars`, a `tempo` in a section): an event of the
    /// tempo map, not a control. It happens once, at a place: every kind of time word that is a place works
    /// (`at`, `after`, `before`, none), `every` and `on phrase` repeat and have no one place.
    pub(crate) fn tempo_line(&mut self, fl: &FlatLine, spec: &TempoSpec) {
        let l = &fl.line;
        if let Some((_, tsp)) = l.target {
            self.err(Diagnostic::error(tsp, "`tempo` takes no target: it is the tempo of the whole program").help("at bar 16: tempo 140"));
            return;
        }
        if let Some((_, hsp)) = l.hold {
            self.err(Diagnostic::error(hsp, "a tempo has nothing to put back").help("change it again with a later line: at bar 20: tempo 126"));
            return;
        }
        let at = match fl.timing {
            Timing::Now => Rat::ZERO,
            Timing::At(b) => Rat::int(i64::from(b)),
            Timing::AtPos(r) => r,
            Timing::Every(_) | Timing::EveryFrom(..) | Timing::EveryPos { .. } | Timing::NextMult(_) => {
                let at = l.when.as_ref().map_or(fl.span, |(_, s)| *s);
                self.err(
                    Diagnostic::error(at, "a tempo change happens once, at one place; this time word repeats")
                        .help("at bar 16: tempo 140   after drop: tempo 128   before drop 2 bars: tempo 100 -> 140 over 2 bars   (a `repeat 4 every 8 bars { ... }` puts one in each copy)"),
                );
                return;
            }
        };
        if let Some(ev) = self.tempo_event(fl.span, at, spec) {
            self.tempos.push(ev);
        }
    }

    /// F10 meets F3: two sections that play at the same time (a stack) may both set the tempo on one bar only to the same
    /// thing, as two stacked `key`s may. The tempo of a stretch is the one its sections agree on, or the one of the only
    /// section that sets any; two that differ are an error naming both. (Sections that play one after the other never
    /// clash: the later one's tempo is the later one's, and a line outside every section is the default they override.)
    pub(crate) fn tempo_clashes(&mut self, lw: &crate::expand::Lowered) {
        // (the placement it is in: name and first bar, the bar it says it on, what it says, the line it is on)
        let mut seen: Vec<(String, u32, Rat, TempoEvent, usize)> = Vec::new();
        let mut said: Vec<(String, u32, String, u32, Rat)> = Vec::new();
        for seg in &lw.segments {
            for fl in &seg.lines {
                if fl.phase.is_some() {
                    continue;
                }
                let Body::Action(Action::Tempo(spec)) = &fl.line.body else { continue };
                let Some((name, start)) = crate::mixmoves::section_of_path(&fl.path) else { continue };
                let at = match fl.timing {
                    Timing::At(b) => Rat::int(i64::from(b)),
                    Timing::AtPos(r) => r,
                    _ => continue,
                };
                let Some(ev) = quiet_event(fl.span, at, spec) else { continue };
                let Some(me) = lw.placements.iter().find(|p| p.name == name && p.start == start) else { continue };
                let covers = |p: &crate::linked::Placed| at >= Rat::int(i64::from(p.start)) && at < Rat::int(i64::from(p.start) + i64::from(p.bars));
                if !covers(me) {
                    continue;
                }
                for (oname, ostart, oat, oev, oline) in &seen {
                    if (oname.as_str(), *ostart) == (name, start) || *oat != at {
                        continue;
                    }
                    let Some(other) = lw.placements.iter().find(|p| p.name == *oname && p.start == *ostart) else { continue };
                    if !covers(other) || (oev.from, oev.to, oev.over, oev.shape) == (ev.from, ev.to, ev.over, ev.shape) {
                        continue;
                    }
                    let key = (oname.clone(), *ostart, name.to_string(), start, at);
                    if said.contains(&key) {
                        continue;
                    }
                    said.push(key);
                    self.err(
                        Diagnostic::error(fl.span, format!("sections `{oname}` and `{name}` play at the same time and both set the tempo on bar {at}, to different things: line {oline} and this one"))
                            .help("a stretch of a song has one tempo: give them the same, set it in one of them, or play them one after the other with then(...)"),
                    );
                }
                seen.push((name.to_string(), start, at, ev, crate::diag::line_col(self.src, fl.span.start).0));
            }
        }
    }

    /// Once every line is read: the map, the tempo at bar 0 (what the nets are made at) and a warning for a ramp that
    /// the next tempo cuts short.
    pub(crate) fn settle_tempo(&mut self) {
        let map = TempoMap::new(self.tempos.clone());
        for w in map.events().windows(2) {
            let (e, n) = (&w[0], &w[1]);
            if !e.over.is_zero() && e.at.add(e.over).is_ok_and(|end| end > n.at) {
                self.diags.push(
                    Diagnostic::warning(e.span, format!("this tempo ramp is cut off at bar {} by the next tempo and never reaches {}", n.at, crate::print::num(e.to)))
                        .help("give it a shorter `over`, or move the next tempo later"),
                );
            }
        }
        if !self.tempos.is_empty() {
            self.tempo = map.declared_at(Rat::ZERO);
        }
        self.tempo_map = map;
    }

    /// The tempo a net that begins on bar `start` is made at (what the nets of a program with a tempo curve say, so a
    /// net that lands on its bar line is already at the tempo the curve has there): None where the tempo is constant
    /// (the nets carry the program's own `tempo`, as they always did).
    pub(crate) fn tempo_for_net(&self, start: u32) -> Option<f64> {
        if self.tempo_map.drives() {
            self.tempo_map.declared_at(start)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// what check says
// ---------------------------------------------------------------------------------------------------------------------

impl Compiled {
    /// The program's tempo through its bars: the exact map.
    pub fn tempo_map(&self) -> &TempoMap {
        &self.tempo_map
    }

    /// The bars the program lasts (a song's, or its `end at bar N`; None when it plays on by itself).
    pub fn length_bars(&self) -> Option<u32> {
        self.end.or_else(|| self.song_bars())
    }

    /// How long the program lasts, in seconds, through the tempo curve.
    pub fn length_seconds(&self) -> Option<f64> {
        self.length_bars().map(|b| self.tempo_map.total_seconds(b))
    }

    /// `length: 3m42s with the tempo curve, 64 bars`, `length: 1m49s at 140, 64 bars`: what `check` says of the song's
    /// length (None for a program that plays on by itself, with no `end`).
    pub fn length_report(&self) -> Option<String> {
        let secs = self.length_seconds()?;
        let bars = self.length_bars()?;
        let human = crate::evolve::human(secs);
        let how = match (self.tempo_map.constant_bpm(), self.tempo_map.is_empty()) {
            (Some(b), _) => format!("at {}", crate::print::num(b)),
            (None, true) => format!("at {} (the program says no tempo)", crate::print::num(DEFAULT_BPM)),
            (None, false) => {
                if self.tempo_map.declared_at(0u32).is_none() {
                    format!("with the tempo curve (counting {} before the first tempo)", crate::print::num(DEFAULT_BPM))
                } else {
                    "with the tempo curve".to_string()
                }
            }
        };
        Some(format!("length: {human} {how}, {bars} bars{}", if self.looped { ", then it starts again" } else { "" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rat {
        Rat::new(n, d).unwrap()
    }

    fn ev(at: i64, from: f64, to: f64, over: i64, shape: Shape) -> TempoEvent {
        TempoEvent { at: Rat::int(at), from, to, over: Rat::int(over), shape, span: Span::default() }
    }

    /// The seconds of a ramp's first `t`, by brute force (Simpson over a few hundred thousand panels).
    fn brute(a: f64, b: f64, shape: Shape, len_beats: f64, t: f64) -> f64 {
        let n = 200_000usize;
        let f = |u: f64| 1.0 / (a + (b - a) * shape.apply(u));
        let h = t / n as f64;
        let mut s = f(0.0) + f(t);
        for i in 1..n {
            s += f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
        }
        60.0 * len_beats * s * h / 3.0
    }

    #[test]
    fn a_constant_tempo_is_sixty_l_over_a_seconds() {
        let m = TempoMap::constant(120.0);
        assert_eq!(m.seconds_at(4u32), 8.0);
        assert_eq!(m.total_seconds(16u32), 32.0);
        assert_eq!(m.seconds_at(r(1, 2)), 1.0);
        assert!(m.is_constant() && !m.drives() && m.constant_bpm() == Some(120.0));
        let m = TempoMap::constant(140.0);
        assert!((m.seconds_at(64u32) - 64.0 * 4.0 * 60.0 / 140.0).abs() < 1e-9);
    }

    #[test]
    fn a_linear_ramp_is_sixty_l_ln_b_over_a_over_b_minus_a() {
        // 100 -> 130 over 8 bars (32 beats), then 130 for ever
        let m = TempoMap::new(vec![ev(0, 100.0, 130.0, 8, Shape::Linear)]);
        let want = 60.0 * 32.0 * (130.0f64 / 100.0).ln() / 30.0;
        assert!((m.seconds_at(8u32) - want).abs() < 1e-9, "{} {want}", m.seconds_at(8u32));
        // then 130: 8 more bars is 8 * 4 * 60 / 130
        assert!((m.seconds_at(16u32) - want - 8.0 * 240.0 / 130.0).abs() < 1e-9);
        // a falling ramp is the same formula
        let m = TempoMap::new(vec![ev(0, 140.0, 100.0, 4, Shape::Linear)]);
        let want = 60.0 * 16.0 * (100.0f64 / 140.0).ln() / -40.0;
        assert!((m.seconds_at(4u32) - want).abs() < 1e-9);
        // a ramp whose ends are the same is a constant (no 0 / 0)
        let m = TempoMap::new(vec![ev(0, 120.0, 120.0, 4, Shape::Linear)]);
        assert!((m.seconds_at(4u32) - 8.0).abs() < 1e-12);
        // and one that is a hair off is a hair off
        let m = TempoMap::new(vec![ev(0, 120.0, 120.000001, 4, Shape::Linear)]);
        assert!((m.seconds_at(4u32) - 8.0).abs() < 1e-6);
    }

    #[test]
    fn every_shape_integrates_as_a_brute_force_integral_does() {
        for shape in [Shape::Linear, Shape::Exp, Shape::Log, Shape::SCurve, Shape::Step(4), Shape::Step(2), Shape::Step(7)] {
            let tol = if matches!(shape, Shape::Step(_)) { 1e-5 } else { 1e-7 };
            for (a, b) in [(100.0, 130.0), (140.0, 90.0), (60.0, 280.0), (300.0, 40.0), (126.0, 126.0)] {
                let m = TempoMap::new(vec![ev(0, a, b, 4, shape)]);
                // (not on a jump of a `step`: a brute-force sum straddles it)
                for t in [0.1, 0.26, 0.49, 0.77, 0.9, 1.0] {
                    let got = m.seconds_at_f(4.0 * t);
                    let want = brute(a, b, shape, 16.0, t);
                    assert!((got - want).abs() < tol * want.max(1.0), "{shape:?} {a}->{b} t={t}: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn seconds_and_bars_are_each_others_inverse() {
        let shapes = [Shape::Linear, Shape::Exp, Shape::Log, Shape::SCurve, Shape::Step(5)];
        for shape in shapes {
            let m = TempoMap::new(vec![TempoEvent::set(Rat::ZERO, 90.0, Span::default()), ev(4, 90.0, 150.0, 6, shape), ev(14, 150.0, 150.0, 0, Shape::Linear), ev(20, 150.0, 70.0, 3, shape)]);
            for b in [r(0, 1), r(1, 3), r(7, 2), r(4, 1), r(9, 1), r(10, 1), r(14, 1), r(41, 3), r(20, 1), r(22, 1), r(23, 1), r(40, 1)] {
                let s = m.seconds_at(b);
                assert!((m.bar_at(s) - b.to_f64()).abs() < 1e-9, "{shape:?} bar {b}: {s} s -> {}", m.bar_at(s));
            }
            // and the seconds of the bars add up to the total
            let sum: f64 = (0..40u32).map(|k| m.seconds_between(k, k + 1)).sum();
            assert!((sum - m.total_seconds(40u32)).abs() < 1e-9, "{shape:?}");
        }
    }

    #[test]
    fn the_tempo_between_the_events() {
        let m = TempoMap::new(vec![ev(0, 100.0, 130.0, 8, Shape::Linear), ev(16, 140.0, 140.0, 0, Shape::Linear)]);
        assert_eq!(m.declared_at(0u32), Some(100.0));
        assert!((m.declared_at(4u32).unwrap() - 115.0).abs() < 1e-12);
        assert_eq!(m.declared_at(8u32), Some(130.0));
        assert_eq!(m.declared_at(15u32), Some(130.0));
        assert_eq!(m.declared_at(16u32), Some(140.0));
        assert_eq!(m.declared_at(99u32), Some(140.0));
        assert!(m.drives() && !m.is_constant());
        assert_eq!(m.range(), Some((100.0, 140.0)));
        // before its first tempo a program says nothing, and the length counts 120
        let m = TempoMap::new(vec![ev(8, 140.0, 140.0, 0, Shape::Linear)]);
        assert_eq!((m.declared_at(0u32), m.declared_at(7u32), m.declared_at(8u32)), (None, None, Some(140.0)));
        assert_eq!(m.bpm_at(0u32), DEFAULT_BPM);
        assert!((m.seconds_at(8u32) - 8.0 * 240.0 / 120.0).abs() < 1e-12);
        assert!(m.drives());
        // no tempo at all
        let m = TempoMap::default();
        assert!(m.is_empty() && !m.drives() && m.declared_at(3u32).is_none());
    }

    #[test]
    fn a_ramp_that_the_next_tempo_cuts_short_stops_there() {
        let m = TempoMap::new(vec![ev(0, 100.0, 200.0, 8, Shape::Linear), ev(4, 120.0, 120.0, 0, Shape::Linear)]);
        // at bar 4 the tempo jumps to 120, whatever the ramp was on its way to
        assert!((m.declared_at(r(399, 100)).unwrap() - 149.875).abs() < 1e-9);
        assert_eq!(m.declared_at(4u32), Some(120.0));
        // the ramp got half way: 4 of its 8 bars (16 of 32 beats), 100 to 150
        let ramp_secs = 60.0 * 32.0 * (150.0f64 / 100.0).ln() / 100.0;
        assert!((m.seconds_at(4u32) - ramp_secs).abs() < 1e-9);
        // two events at one place: the later wins
        let m = TempoMap::new(vec![ev(0, 100.0, 100.0, 0, Shape::Linear), ev(0, 130.0, 130.0, 0, Shape::Linear)]);
        assert_eq!(m.declared_at(0u32), Some(130.0));
    }

    #[test]
    fn the_staircase_adds_up_to_the_exact_length() {
        let m = TempoMap::new(vec![ev(0, 100.0, 130.0, 8, Shape::SCurve), ev(8, 130.0, 130.0, 0, Shape::Linear), ev(12, 130.0, 100.0, 4, Shape::Linear)]);
        for step in [1.0, 0.25, 2.0] {
            let st = m.staircase(64.0, step);
            // every step's start is in beats, in order; the seconds they take add up to the curve's
            let mut secs = 0.0;
            for (i, (b0, bpm)) in st.iter().enumerate() {
                let b1 = st.get(i + 1).map_or(64.0, |n| n.0);
                secs += 60.0 * (b1 - b0) / bpm;
            }
            assert!((secs - m.seconds_at(16u32)).abs() < 1e-9, "step {step}: {secs} vs {}", m.seconds_at(16u32));
        }
    }

    #[test]
    fn tempo_spec_is_read_and_printed() {
        let p = crate::parse::parse("tempo 100 -> 130 over 8 bars exp\n");
        assert!(p.ok(), "{:?}", p.diagnostics);
        assert_eq!(crate::print::canonical(&p.program), "tempo 100 -> 130 over 8 bars exp\n");
    }
}
