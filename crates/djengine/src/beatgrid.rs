//! Beat grids for audio djrust did not generate (docs/BEAT-DETECTION.md).
//!
//! Pure: no I/O, no clock, no models (`no_std` + `alloc`, so it builds for wasm). The neural
//! detector (Beat This!, `djhost::analysis`, behind the `neural` feature) hands its beat and
//! downbeat times to [`fit_grid`]; without it [`dsp_analyze`] finds a tempo and a phase from the
//! signal alone. Both answer a [`BeatGrid`], and everything that uses a grid works on that:
//!
//! - [`fit_grid`]: a **steady-first** fit. Beats that sit on one tempo become one exact grid
//!   (`origin + k * period`); only a track that really drifts keeps its own beat times. Bar 1 is
//!   a downbeat: the bar phase is the one the detected downbeats vote for.
//! - [`trim_to_bars`]: where a loop starts (the first downbeat) and how many whole bars it runs.
//! - [`estimate_latency`]: how late a recording of a clock-slaved source is, from where its beats
//!   fall against the clock's.
//! - [`FrameGrid`]: the grid in a recording's own frames, with bar and beat lines to snap a
//!   scrub, a freeze or a capture to.
//!
//! Times are seconds in the analysed audio unless a name says frames.

// `!(x > 0.0)` is deliberate: it is also true for NaN
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use alloc::vec::Vec;

use libm::{atan2, cos, exp, fabs, floor, log, round, sin, sqrt};

/// The slowest and fastest tempo a grid is folded into (octave errors are halved or doubled
/// into this range).
pub const BPM_RANGE: (f64, f64) = (70.0, 180.0);

/// Where a grid came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridSource {
    /// Found in the signal by [`dsp_analyze`]: a tempo and a phase, bar 1 a guess from accents.
    Dsp,
    /// Beat This!'s beats and downbeats through [`fit_grid`].
    Neural,
    /// The tempo the audio itself carries (a library loop's `bpm`, a vocal line's): beat 1 is its
    /// first sample. Nothing was detected.
    Known,
}

impl GridSource {
    pub fn name(self) -> &'static str {
        match self {
            GridSource::Dsp => "dsp",
            GridSource::Neural => "neural",
            GridSource::Known => "known",
        }
    }
    pub fn parse(s: &str) -> Option<GridSource> {
        match s {
            "dsp" => Some(GridSource::Dsp),
            "neural" => Some(GridSource::Neural),
            "known" => Some(GridSource::Known),
            _ => None,
        }
    }
}

/// The beats of a piece of audio.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatGrid {
    pub source: GridSource,
    /// Seconds per beat (the mean, when the grid is not steady).
    pub period: f64,
    /// Beats in a bar (3 or 4 mostly).
    pub beats_per_bar: u32,
    /// One tempo all the way: `beats` are exact multiples of `period`. False: they are the
    /// detected times (the track drifts or is played, not sequenced).
    pub steady: bool,
    /// 0..1: how much to trust it (how well the beats agree, how clearly the bar phase won).
    pub confidence: f64,
    /// RMS distance of the detected beats from the fitted ones, ms (0 for none).
    pub residual_ms: f64,
    /// Every beat in the audio, seconds, ascending, with no gaps.
    pub beats: Vec<f64>,
    /// Index in `beats` of the first downbeat (bar 1).
    pub first_downbeat: usize,
}

impl BeatGrid {
    pub fn bpm(&self) -> f64 {
        60.0 / self.period
    }

    /// Seconds in a bar.
    pub fn bar_period(&self) -> f64 {
        self.period * self.beats_per_bar as f64
    }

    /// Time of the first downbeat: where bar 1 starts.
    pub fn first_downbeat_time(&self) -> f64 {
        self.beats.get(self.first_downbeat).copied().unwrap_or(0.0)
    }

    /// Time of beat `k`, counted from bar 1 (`k = 0` is the first downbeat; negative numbers
    /// are before it). Beyond the audio the grid goes on at its last intervals.
    pub fn time(&self, k: i64) -> f64 {
        let n = self.beats.len() as i64;
        let j = k + self.first_downbeat as i64;
        if n == 0 {
            return k as f64 * self.period;
        }
        if (0..n).contains(&j) {
            return self.beats[j as usize];
        }
        if self.steady || n < 2 {
            return self.beats[0] + (j as f64) * self.period;
        }
        // run on at the average of the last (or first) few intervals
        let m = (n - 1).min(4) as usize;
        if j < 0 {
            let p = (self.beats[m] - self.beats[0]) / m as f64;
            self.beats[0] + j as f64 * p
        } else {
            let last = n as usize - 1;
            let p = (self.beats[last] - self.beats[last - m]) / m as f64;
            self.beats[last] + (j - last as i64) as f64 * p
        }
    }

    /// The downbeats in the audio, seconds.
    pub fn downbeats(&self) -> Vec<f64> {
        let bpb = self.beats_per_bar.max(1) as usize;
        (self.first_downbeat % bpb..self.beats.len())
            .step_by(bpb)
            .map(|j| self.beats[j])
            .collect()
    }

    /// Whole bars in `duration` seconds from the first downbeat.
    pub fn bars_after_first(&self, duration: f64) -> u32 {
        let d = duration - self.first_downbeat_time();
        if d <= 0.0 {
            0
        } else {
            floor(d / self.bar_period()) as u32
        }
    }

    /// The beat number (from bar 1) nearest `t`, and its time.
    pub fn nearest_beat(&self, t: f64) -> (i64, f64) {
        let k0 = floor((t - self.time(0)) / self.period) as i64;
        let mut best = (k0, self.time(k0));
        for k in [k0 - 1, k0 + 1, k0 + 2] {
            let tk = self.time(k);
            if fabs(tk - t) < fabs(best.1 - t) {
                best = (k, tk);
            }
        }
        best
    }

    /// The grid in a recording whose frame 0 is this audio's `t = 0` (`origin_frame` is where
    /// the audio starts in the recording's own frame counter).
    pub fn frame_grid(&self, sample_rate: f64, origin_frame: f64) -> FrameGrid {
        FrameGrid {
            origin: origin_frame + self.first_downbeat_time() * sample_rate,
            period: self.period * sample_rate,
            beats_per_bar: self.beats_per_bar.max(1),
        }
    }
}

// MARK: fitting

/// What [`fit_grid`] is allowed.
#[derive(Clone, Copy, Debug)]
pub struct FitOpts {
    /// The tempo the caller expects (a tag, the deck, the DSP's guess). Only used to pick the
    /// octave when the beats alone could be read either way.
    pub hint_bpm: Option<f64>,
    /// A fit is steady when no beat is further than this from its grid line, ms (and the rest
    /// of the tests pass).
    pub steady_tolerance_ms: f64,
}

impl Default for FitOpts {
    fn default() -> FitOpts {
        FitOpts { hint_bpm: None, steady_tolerance_ms: 35.0 }
    }
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Least squares `t = a + b n` over `(n, t)`.
fn line_fit(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = pts.len() as f64;
    if pts.len() < 2 {
        return None;
    }
    let sn: f64 = pts.iter().map(|p| p.0).sum();
    let st: f64 = pts.iter().map(|p| p.1).sum();
    let snn: f64 = pts.iter().map(|p| p.0 * p.0).sum();
    let snt: f64 = pts.iter().map(|p| p.0 * p.1).sum();
    let d = n * snn - sn * sn;
    if fabs(d) < 1e-12 {
        return None;
    }
    let b = (n * snt - sn * st) / d;
    let a = (st - b * sn) / n;
    Some((a, b))
}

/// Fold a tempo into [`BPM_RANGE`] by octaves, the one nearest `hint` when given.
fn fold_bpm(mut bpm: f64, hint: Option<f64>) -> f64 {
    if !(bpm > 0.0) {
        return bpm;
    }
    while bpm < BPM_RANGE.0 {
        bpm *= 2.0;
    }
    while bpm > BPM_RANGE.1 {
        bpm /= 2.0;
    }
    if let Some(h) = hint.filter(|h| *h > 0.0) {
        // either octave of a tempo that fits the range: take the one nearer the hint
        let mut best = bpm;
        for c in [bpm / 2.0, bpm * 2.0] {
            if c >= BPM_RANGE.0 * 0.8 && c <= BPM_RANGE.1 * 1.25 && fabs(log(c / h)) < fabs(log(best / h)) {
                best = c;
            }
        }
        return best;
    }
    bpm
}

/// A steady-first grid from detected beats and downbeats (seconds). `duration` is the audio's
/// length, so the grid covers all of it. None with fewer than four beats or no sensible tempo.
///
/// 1. The beats are numbered by their median interval and a line `t = a + T n` fitted to them,
///    dropping outliers until the fit settles.
/// 2. The grid is **steady** when every kept beat is within [`FitOpts::steady_tolerance_ms`] of
///    it, nearly all beats were kept, and the two halves of the track agree on the tempo within
///    1%. A steady grid is exact: `beats[k] = a + T k`.
/// 3. Otherwise the detected beats are kept as they are (gaps filled), so a drifting or played
///    track is followed rather than forced onto a tempo it does not keep.
/// 4. Bar 1 is a downbeat: each detected downbeat votes for the beat number mod the bar length
///    it sits on, and the first beat of the winning phase is bar 1.
pub fn fit_grid(beats: &[f64], downbeats: &[f64], duration: f64, opts: FitOpts) -> Option<BeatGrid> {
    let mut bt: Vec<f64> = beats.iter().copied().filter(|t| t.is_finite() && *t >= 0.0).collect();
    bt.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    bt.dedup_by(|a, b| fabs(*a - *b) < 0.02);
    if bt.len() < 4 {
        return None;
    }
    let mut iv: Vec<f64> = bt.windows(2).map(|w| w[1] - w[0]).collect();
    let t0 = median(&mut iv);
    if !(t0 > 0.0) {
        return None;
    }
    // the octave: a median interval outside the range is half or double a beat
    let t_octave = 60.0 / fold_bpm(60.0 / t0, opts.hint_bpm);
    let t_init = t_octave;
    // number the beats; with a halved/doubled interval some beats are missing or extra, so
    // number against the folded period and let the fit drop what does not agree
    // the period to number by: the mean of the intervals that are one beat (a stray or a missed
    // beat makes odd ones), much nearer the truth than the median of quantised intervals
    let one: Vec<f64> = iv.iter().copied().filter(|d| *d > 0.7 * t0 && *d < 1.3 * t0).collect();
    let t_init = if one.is_empty() {
        t_init
    } else {
        t_init * (one.iter().sum::<f64>() / one.len() as f64 / t0)
    };
    let mut keep: Vec<(f64, f64)> = bt.iter().map(|t| (round((t - bt[0]) / t_init), *t)).collect();
    let mut fit = line_fit(&keep)?;
    // a stray or missed beat shifts the sequential numbering after it: number against the
    // fitted line instead, and fit again
    for _ in 0..3 {
        let renum: Vec<(f64, f64)> = bt.iter().map(|t| (round((t - fit.0) / fit.1), *t)).collect();
        match line_fit(&renum) {
            Some(f) if f.1 > 0.0 => {
                fit = f;
                keep = renum;
            }
            _ => break,
        }
    }
    for _ in 0..6 {
        let resid: Vec<f64> = keep.iter().map(|(n, t)| t - (fit.0 + fit.1 * n)).collect();
        let mut abs: Vec<f64> = resid.iter().map(|r| fabs(*r)).collect();
        let mad = median(&mut abs);
        let cut = (3.5 * 1.4826 * mad).max(0.015).min(0.2 * fit.1);
        let next: Vec<(f64, f64)> = keep
            .iter()
            .zip(&resid)
            .filter(|(_, r)| fabs(**r) <= cut)
            .map(|(p, _)| *p)
            .collect();
        if next.len() < 4 || next.len() == keep.len() {
            break;
        }
        keep = next;
        let Some(f) = line_fit(&keep) else { break };
        fit = f;
        // renumber with the refined period (a long grid walks off a rough one)
        keep = keep
            .iter()
            .map(|(_, t)| (round((t - fit.0) / fit.1), *t))
            .collect();
        if let Some(f) = line_fit(&keep) {
            fit = f;
        }
    }
    let (a, period) = fit;
    if !(period > 0.0) {
        return None;
    }
    let all: Vec<(f64, f64)> = bt.iter().map(|t| (round((t - a) / period), *t)).collect();
    let resid: Vec<f64> = all.iter().map(|(n, t)| t - (a + period * n)).collect();
    let inliers = resid.iter().filter(|r| fabs(**r) <= opts.steady_tolerance_ms / 1000.0).count();
    let frac = inliers as f64 / resid.len() as f64;
    let rms = {
        let v: Vec<f64> = resid
            .iter()
            .filter(|r| fabs(**r) <= opts.steady_tolerance_ms / 1000.0)
            .map(|r| r * r)
            .collect();
        if v.is_empty() {
            0.0
        } else {
            sqrt(v.iter().sum::<f64>() / v.len() as f64)
        }
    };
    // do the two halves agree on the tempo?
    let half = all.len() / 2;
    let drift = match (line_fit(&all[..half]), line_fit(&all[half..])) {
        (Some(x), Some(y)) if x.1 > 0.0 => fabs(y.1 / x.1 - 1.0),
        _ => 0.0,
    };
    let steady = frac >= 0.9 && drift <= 0.01;

    // bar length from the downbeats, else 4
    let mut db: Vec<f64> = downbeats.iter().copied().filter(|t| t.is_finite() && *t >= 0.0).collect();
    db.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let mut bpb = 4u32;
    if db.len() >= 2 {
        let mut gaps: Vec<f64> = db.windows(2).map(|w| (w[1] - w[0]) / period).collect();
        let g = round(median(&mut gaps)) as u32;
        if (2..=7).contains(&g) {
            bpb = g;
        }
    }

    // the beats the grid is made of
    let (grid_beats, steady_out) = if steady {
        let n_lo = floor((0.0 - a) / period) as i64;
        let n_hi = floor((duration.max(bt[bt.len() - 1]) - a) / period) as i64;
        let v: Vec<f64> = (n_lo.max(-4)..=n_hi)
            .map(|n| a + period * n as f64)
            .filter(|t| *t >= -0.5 * period)
            .collect();
        (v, true)
    } else {
        (fill_gaps(&bt, period), false)
    };
    if grid_beats.len() < 2 {
        return None;
    }

    // bar phase: which beat of the bar each detected downbeat is, by vote
    let mut votes = alloc::vec![0.0f64; bpb as usize];
    for d in &db {
        let j = nearest_index(&grid_beats, *d);
        if fabs(grid_beats[j] - d) <= 0.4 * period {
            votes[j % bpb as usize] += 1.0;
        }
    }
    let total: f64 = votes.iter().sum();
    let (phase, best) = votes
        .iter()
        .enumerate()
        .fold((0usize, -1.0f64), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
    let first_downbeat = phase.min(grid_beats.len() - 1);
    let phase_share = if total > 0.0 { best / total } else { 0.0 };

    // confidence: the beats agree, nearly all kept, the bar phase is clear, enough of them
    let tight = 1.0 - (rms * 1000.0 / opts.steady_tolerance_ms).min(1.0) * 0.5;
    let count = (bt.len() as f64 / 16.0).min(1.0);
    let mut conf = frac * tight * (0.5 + 0.5 * count) * (0.4 + 0.6 * phase_share);
    if !steady {
        conf *= 0.8;
    }
    Some(BeatGrid {
        source: GridSource::Neural,
        period,
        beats_per_bar: bpb,
        steady: steady_out,
        confidence: conf.clamp(0.0, 1.0),
        residual_ms: rms * 1000.0,
        beats: grid_beats,
        first_downbeat,
    })
}

fn nearest_index(v: &[f64], t: f64) -> usize {
    let mut lo = 0usize;
    let mut hi = v.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if v[mid] < t {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        0
    } else if lo >= v.len() {
        v.len() - 1
    } else if fabs(v[lo] - t) < fabs(v[lo - 1] - t) {
        lo
    } else {
        lo - 1
    }
}

/// Detected beats with the ones the detector missed put back: a gap of about two periods gets
/// one beat in the middle, three gets two, and so on.
fn fill_gaps(bt: &[f64], period: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(bt.len() + 8);
    for (i, t) in bt.iter().enumerate() {
        if i > 0 {
            let gap = t - bt[i - 1];
            let n = round(gap / period) as i64;
            if (2..=8).contains(&n) {
                for k in 1..n {
                    out.push(bt[i - 1] + gap * k as f64 / n as f64);
                }
            }
        }
        out.push(*t);
    }
    out
}

// MARK: phase refinement

/// A neural detector reports beats on its own frame grid (Beat This!: 20 ms) and sits a frame
/// early or late of the attack. For a **steady** grid, move the whole grid to the sound: each
/// beat looks for the strongest onset within `window` seconds, and the grid moves by the median
/// of how far those onsets are from it, if at least half the beats found one and they agree to
/// within 8 ms. Returns the shift in seconds (None: left as it was).
pub fn refine_to_onsets(g: &mut BeatGrid, onsets: &[(f64, f64)], window: f64) -> Option<f64> {
    if !g.steady || g.beats.len() < 8 || onsets.is_empty() {
        return None;
    }
    let max = onsets.iter().map(|o| o.1).fold(0.0, f64::max);
    if !(max > 0.0) {
        return None;
    }
    let mut offs: Vec<f64> = Vec::new();
    for b in &g.beats {
        // the strongest onset in reach, if it is a real one
        let best = onsets
            .iter()
            .filter(|(t, w)| fabs(t - b) <= window && *w >= 0.2 * max)
            .fold(None::<(f64, f64)>, |m, o| match m {
                Some(x) if x.1 >= o.1 => Some(x),
                _ => Some(*o),
            });
        if let Some((t, _)) = best {
            offs.push(t - b);
        }
    }
    if offs.len() * 2 < g.beats.len() {
        return None;
    }
    let med = median(&mut offs.clone());
    let mut dev: Vec<f64> = offs.iter().map(|o| fabs(o - med)).collect();
    if median(&mut dev) > 0.008 {
        return None;
    }
    for b in g.beats.iter_mut() {
        *b += med;
    }
    Some(med)
}

// MARK: trimming a loop to bars

/// How a loop may be cut.
#[derive(Clone, Copy, Debug)]
pub struct TrimOpts {
    /// A first downbeat this close to the start (seconds) is already the start: nothing is cut
    /// from the head.
    pub start_tolerance: f64,
    /// The loop may run this much of a bar short of a whole number of bars (the tempo of the
    /// loop is then worked out from its real length).
    pub end_tolerance_bars: f64,
    /// The most bars kept (0: no limit).
    pub max_bars: u32,
}

impl Default for TrimOpts {
    fn default() -> TrimOpts {
        TrimOpts { start_tolerance: 0.030, end_tolerance_bars: 0.04, max_bars: 16 }
    }
}

/// The cut [`trim_to_bars`] makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    /// First frame kept (on the first downbeat).
    pub start: usize,
    /// Frames kept.
    pub frames: usize,
    pub bars: u32,
    /// `bars * beats_per_bar`.
    pub beats: u32,
    /// The tempo of the kept audio, from its real length.
    pub bpm: f64,
    /// Whether it differs from the whole audio (frames cut from the head or tail).
    pub changed: bool,
}

/// Cut `total` frames of audio at `sample_rate` to start on the grid's first downbeat and run
/// whole bars. None when there is not one whole bar after the downbeat.
pub fn trim_to_bars(grid: &BeatGrid, total: usize, sample_rate: f64, opts: TrimOpts) -> Option<Trim> {
    let dur = total as f64 / sample_rate;
    let mut t0 = grid.first_downbeat_time();
    // a loop whose downbeat is in the first moments starts there already
    let first = if t0 <= opts.start_tolerance { 0.0 } else { t0 };
    // a downbeat before the audio (negative) counts as the start too
    if t0 < 0.0 {
        t0 = 0.0;
    }
    let _ = t0;
    let bar = grid.bar_period();
    let avail = (dur - first) / bar;
    let mut bars = floor(avail + opts.end_tolerance_bars) as i64;
    if bars < 1 {
        return None;
    }
    if opts.max_bars > 0 {
        bars = bars.min(opts.max_bars as i64);
    }
    let start = (round(first * sample_rate) as usize).min(total);
    let want = round(bars as f64 * bar * sample_rate) as usize;
    let frames = want.min(total - start);
    if frames < 2 {
        return None;
    }
    let beats = bars as u32 * grid.beats_per_bar;
    let bpm = beats as f64 * 60.0 * sample_rate / frames as f64;
    Some(Trim { start, frames, bars: bars as u32, beats, bpm, changed: start != 0 || frames != total })
}

// MARK: latency

/// How late a recording runs against a clock: the answer of [`estimate_latency`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatencyEstimate {
    /// Milliseconds the recording's beats come after the clock's (negative: before).
    pub ms: f64,
    /// 0..1: how tightly the beats agree on it (the circular resultant length, thinned when few
    /// beats were seen).
    pub confidence: f64,
    pub beats_used: usize,
    /// The beats' spread around the answer, ms.
    pub spread_ms: f64,
}

/// The offset of detected beats (`detected`, seconds on the recording's clock) from a reference
/// grid of beats `period` apart, one of them at `origin`. The offsets are averaged on the circle
/// of one period, so a beat that is late by 35 ms and one early by 5 ms agree on 15 ms rather
/// than cancelling, and the answer is unambiguous within half a period. `weights` (one per beat;
/// empty: all 1) lets stronger onsets count more.
pub fn estimate_latency(detected: &[f64], weights: &[f64], period: f64, origin: f64) -> Option<LatencyEstimate> {
    if detected.len() < 4 || !(period > 0.0) {
        return None;
    }
    let two_pi = 2.0 * core::f64::consts::PI;
    let (mut c, mut s, mut w_sum) = (0.0, 0.0, 0.0);
    for (i, t) in detected.iter().enumerate() {
        let w = weights.get(i).copied().unwrap_or(1.0).max(0.0);
        let ph = two_pi * ((t - origin) / period);
        c += w * cos(ph);
        s += w * sin(ph);
        w_sum += w;
    }
    if !(w_sum > 0.0) {
        return None;
    }
    let r = sqrt(c * c + s * s) / w_sum;
    let mean = atan2(s, c) / two_pi * period; // seconds, -period/2 .. period/2
    // circular standard deviation, back in seconds
    let spread = if r > 1e-9 { sqrt((-2.0 * log(r)).max(0.0)) / two_pi * period } else { period / 2.0 };
    let n = detected.len() as f64;
    let thin = (n / 12.0).min(1.0);
    Some(LatencyEstimate {
        ms: mean * 1000.0,
        confidence: (r * thin).clamp(0.0, 1.0),
        beats_used: detected.len(),
        spread_ms: spread * 1000.0,
    })
}

// MARK: lines in a recording's frames

/// What a line of a [`FrameGrid`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Beat,
    Bar,
}

impl Unit {
    pub fn parse(s: &str) -> Option<Unit> {
        match s {
            "beat" | "beats" => Some(Unit::Beat),
            "bar" | "bars" => Some(Unit::Bar),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Unit::Beat => "beat",
            Unit::Bar => "bar",
        }
    }
}

/// Which end of a span the snapped bar line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The span ends on the line (the bars before the head).
    End,
    /// The span starts on the line (the bars from the head on).
    Start,
}

impl Anchor {
    pub fn parse(s: &str) -> Option<Anchor> {
        match s {
            "end" => Some(Anchor::End),
            "start" => Some(Anchor::Start),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Anchor::End => "end",
            Anchor::Start => "start",
        }
    }
}

/// A steady grid in a recording's own frame counter (the input history's tap): bar 1 at
/// `origin`, a beat every `period` frames. It goes on in both directions, so it can name the bar
/// lines of audio that has not been analysed yet (while the player keeps the tempo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameGrid {
    pub origin: f64,
    pub period: f64,
    pub beats_per_bar: u32,
}

impl FrameGrid {
    pub fn bar_frames(&self) -> f64 {
        self.period * self.beats_per_bar as f64
    }

    fn unit_frames(&self, u: Unit) -> f64 {
        match u {
            Unit::Beat => self.period,
            Unit::Bar => self.bar_frames(),
        }
    }

    /// Frame of line `k` (`k = 0` is bar 1's line, whichever unit).
    pub fn line(&self, k: i64, u: Unit) -> f64 {
        self.origin + k as f64 * self.unit_frames(u)
    }

    /// The line nearest `frame`: its number and frame.
    pub fn nearest(&self, frame: f64, u: Unit) -> (i64, f64) {
        let k = round((frame - self.origin) / self.unit_frames(u)) as i64;
        (k, self.line(k, u))
    }

    /// The last line at or before `frame` (within half a frame of it counts as at).
    pub fn at_or_before(&self, frame: f64, u: Unit) -> (i64, f64) {
        let k = floor((frame - self.origin) / self.unit_frames(u) + 1e-9) as i64;
        (k, self.line(k, u))
    }

    /// The first line after `frame`.
    pub fn after(&self, frame: f64, u: Unit) -> (i64, f64) {
        let (k, _) = self.at_or_before(frame, u);
        (k + 1, self.line(k + 1, u))
    }

    /// The line `n` lines from the one nearest `frame` (negative `n`: back in time).
    pub fn step(&self, frame: f64, n: i64, u: Unit) -> f64 {
        let (k, _) = self.nearest(frame, u);
        self.line(k + n, u)
    }

    /// A span of exactly `bars` bars (a whole number of frames each way, the bar length rounded
    /// once for the whole span, so the loop is as long as the music says) around a head at
    /// `head`, snapped to the nearest bar line `L`: `Anchor::End` is the bars before `L`,
    /// `Anchor::Start` the bars from `L` on. None when it does not lie inside what is recorded
    /// (`oldest` ..= `newest`).
    pub fn span(&self, head: f64, bars: u32, anchor: Anchor, oldest: f64, newest: f64) -> Option<(i64, i64)> {
        if bars == 0 || !(self.period > 0.0) {
            return None;
        }
        let (_, l) = self.nearest(head, Unit::Bar);
        let len = self.bar_frames() * bars as f64;
        let (a, b) = match anchor {
            Anchor::End => (l - len, l),
            Anchor::Start => (l, l + len),
        };
        let (a, b) = (round(a) as i64, round(b) as i64);
        // the length is the rounded span, so two spans of the same bars are the same size
        let (a, b) = match anchor {
            Anchor::End => (b - round(len) as i64, b),
            Anchor::Start => (a, a + round(len) as i64),
        };
        (a as f64 >= oldest && b as f64 <= newest).then_some((a, b))
    }

    /// The lines in `[from, to]`, each with whether it is a bar line.
    pub fn lines_between(&self, from: f64, to: f64, u: Unit) -> Vec<(f64, bool)> {
        let step = self.unit_frames(u);
        if !(step > 0.0) || to < from {
            return Vec::new();
        }
        let k0 = (floor((from - self.origin) / step)) as i64;
        let bpb = self.beats_per_bar as i64;
        let mut out = Vec::new();
        let mut k = k0;
        loop {
            let f = self.origin + k as f64 * step;
            if f > to {
                break;
            }
            if f >= from {
                let bar = match u {
                    Unit::Bar => true,
                    Unit::Beat => k.rem_euclid(bpb) == 0,
                };
                out.push((f, bar));
            }
            k += 1;
            if out.len() > 4096 {
                break;
            }
        }
        out
    }
}

// MARK: DSP detector

/// What [`dsp_analyze`] looks for.
#[derive(Clone, Copy, Debug)]
pub struct DspOpts {
    pub min_bpm: f64,
    pub max_bpm: f64,
    /// Seconds of audio to listen to (the first this many); 0: all.
    pub max_seconds: f64,
}

impl Default for DspOpts {
    fn default() -> DspOpts {
        DspOpts { min_bpm: 75.0, max_bpm: 175.0, max_seconds: 90.0 }
    }
}

/// The onset envelope of mono audio: per hop, the half-wave rectified rise of the log energy of
/// three bands (low, mid, high), the low band counting most. Returns the envelope, the low-band
/// envelope alone (for accents), and the frames per second.
pub fn onset_envelope(samples: &[f32], sample_rate: f64) -> (Vec<f32>, Vec<f32>, f64) {
    let hop = ((sample_rate / 172.0) as usize).max(32);
    let fps = sample_rate / hop as f64;
    let n_hops = samples.len() / hop;
    if n_hops < 8 {
        return (Vec::new(), Vec::new(), fps);
    }
    // three bands from one-pole filters: low < ~180 Hz, mid to ~2.5 kHz, high above
    let lp = |fc: f64| 1.0 - exp(-2.0 * core::f64::consts::PI * fc / sample_rate);
    let (a_lo, a_mid) = (lp(180.0), lp(2500.0));
    let (mut y1, mut y2) = (0.0f64, 0.0f64);
    let mut e = alloc::vec![[0.0f64; 3]; n_hops];
    for (h, frame) in samples.chunks_exact(hop).take(n_hops).enumerate() {
        let mut acc = [0.0f64; 3];
        for &x in frame {
            let x = x as f64;
            y1 += a_lo * (x - y1);
            y2 += a_mid * (x - y2);
            let lo = y1;
            let mid = y2 - y1;
            let hi = x - y2;
            acc[0] += lo * lo;
            acc[1] += mid * mid;
            acc[2] += hi * hi;
        }
        for b in 0..3 {
            e[h][b] = acc[b] / hop as f64;
        }
    }
    let weights = [1.0, 0.8, 0.6];
    let mut env = alloc::vec![0.0f32; n_hops];
    let mut low = alloc::vec![0.0f32; n_hops];
    for h in 1..n_hops {
        let mut tot = 0.0;
        for b in 0..3 {
            let cur = log(1.0 + 1.0e4 * e[h][b]);
            let prev = log(1.0 + 1.0e4 * e[h - 1][b]);
            let d = (cur - prev).max(0.0);
            if b == 0 {
                low[h] = d as f32;
            }
            tot += weights[b] * d;
        }
        env[h] = tot as f32;
    }
    (env, low, fps)
}

fn env_at(env: &[f32], x: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    let i = floor(x) as usize;
    if i + 1 >= env.len() {
        return 0.0;
    }
    let f = x - i as f64;
    env[i] as f64 * (1.0 - f) + env[i + 1] as f64 * f
}

/// Onsets of an envelope: its local peaks above `rel` of the maximum, as (seconds, strength).
pub fn envelope_peaks(env: &[f32], fps: f64, rel: f64) -> Vec<(f64, f64)> {
    let max = env.iter().cloned().fold(0.0f32, f32::max) as f64;
    if max <= 0.0 {
        return Vec::new();
    }
    let floor_v = rel * max;
    let min_gap = (0.06 * fps) as usize; // 60 ms
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut last = usize::MAX;
    for i in 1..env.len().saturating_sub(1) {
        let v = env[i] as f64;
        if v >= floor_v && env[i] >= env[i - 1] && env[i] > env[i + 1] {
            if last != usize::MAX && i - last < min_gap {
                if let Some(prev) = out.last_mut() {
                    if v > prev.1 {
                        *prev = ((i as f64 + 0.5) / fps, v);
                        last = i;
                    }
                }
                continue;
            }
            out.push(((i as f64 + 0.5) / fps, v));
            last = i;
        }
    }
    out
}

/// The strongest comb of period `p` hops over all phases: (score, on-beat mean, phase in hops).
/// The score is the envelope's mean on the comb less 0.6 of its mean halfway between the teeth,
/// so a comb at half the tempo (whose teeth include the real beats' off-beats) does not tie with
/// the real one.
fn comb(env: &[f32], p: f64) -> (f64, f64, f64) {
    let steps = (libm::ceil(p) as usize * 2).max(8);
    let mut best = (f64::MIN, 0.0, 0.0);
    for s in 0..steps {
        let ph = p * s as f64 / steps as f64;
        let (mut on, mut off, mut n) = (0.0, 0.0, 0usize);
        let mut x = ph;
        while x < env.len() as f64 - 1.0 - p {
            on += env_at(env, x);
            off += env_at(env, x + 0.5 * p);
            n += 1;
            x += p;
        }
        if n > 0 {
            let on = on / n as f64;
            let score = on - 0.6 * off / n as f64;
            if score > best.0 {
                best = (score, on, ph);
            }
        }
    }
    best
}

/// A grid from the signal alone: a tempo and a phase from the onset envelope (autocorrelation to
/// find the candidates, a fine comb to choose and refine), bar 1 a guess at the beat the low
/// band hits hardest. Steady by construction; its confidence is how much the comb beats the
/// envelope's average, and is capped (0.7) because bar 1 is a guess. None for silence or less
/// than four seconds.
pub fn dsp_analyze(samples: &[f32], sample_rate: f64, opts: DspOpts) -> Option<BeatGrid> {
    let take = if opts.max_seconds > 0.0 {
        ((opts.max_seconds * sample_rate) as usize).min(samples.len())
    } else {
        samples.len()
    };
    let (env, low, fps) = onset_envelope(&samples[..take], sample_rate);
    if env.len() < (4.0 * fps) as usize {
        return None;
    }
    let mean = env.iter().map(|v| *v as f64).sum::<f64>() / env.len() as f64;
    if !(mean > 1e-6) {
        return None;
    }
    let dev: Vec<f64> = env.iter().map(|v| *v as f64 - mean).collect();
    let lag_lo = floor(fps * 60.0 / opts.max_bpm) as usize;
    let lag_hi = libm::ceil(fps * 60.0 / opts.min_bpm) as usize;
    let max_lag = (lag_hi * 4).min(dev.len() / 2);
    let mut ac = alloc::vec![0.0f64; max_lag + 1];
    for (lag, slot) in ac.iter_mut().enumerate() {
        let mut s = 0.0;
        for i in lag..dev.len() {
            s += dev[i] * dev[i - lag];
        }
        *slot = s / (dev.len() - lag) as f64;
    }
    if ac.is_empty() || ac[0] <= 0.0 {
        return None;
    }
    // candidates: autocorrelation peaks in range, with their multiples reinforcing them
    let mut cands: Vec<(f64, f64)> = Vec::new();
    for lag in lag_lo.max(2)..=lag_hi.min(max_lag.saturating_sub(1)) {
        if ac[lag] > ac[lag - 1] && ac[lag] >= ac[lag + 1] {
            let mut s = ac[lag];
            if lag * 2 <= max_lag {
                s += 0.5 * ac[lag * 2];
            }
            if lag * 4 <= max_lag {
                s += 0.25 * ac[lag * 4];
            }
            // a soft preference for the middle of the range
            let bpm = 60.0 * fps / lag as f64;
            let prior = exp(-0.5 * (log(bpm / 125.0) / 0.45) * (log(bpm / 125.0) / 0.45));
            cands.push((s * (0.6 + 0.4 * prior), lag as f64));
        }
    }
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(core::cmp::Ordering::Equal));
    cands.truncate(4);
    if cands.is_empty() {
        return None;
    }
    // choose by the fine comb around each candidate (±2.5%, steps of 0.05%)
    let mut best: Option<(f64, f64, f64)> = None; // (score, period hops, phase hops)
    let mut on_mean = 0.0;
    for (_, lag) in &cands {
        let mut k = -50i32;
        while k <= 50 {
            let p = lag * (1.0 + 0.0005 * k as f64);
            let (score, on, ph) = comb(&env, p);
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, p, ph));
                on_mean = on;
            }
            k += 1;
        }
    }
    let (_, p_hops, ph_hops) = best?;
    let contrast = (on_mean - mean) / (mean.max(1e-9) * 4.0);
    let period = p_hops / fps;
    let n_beats = floor((env.len() as f64 / fps - ph_hops / fps) / period) as usize;
    if n_beats < 4 {
        return None;
    }
    // the whole audio's grid, from the phase of the listened part
    let dur = samples.len() as f64 / sample_rate;
    // an envelope value stands for its whole hop: its middle is the time
    let t0 = (ph_hops + 0.5) / fps;
    let total_beats = floor((dur - t0) / period) as usize + 1;
    let beats: Vec<f64> = (0..total_beats).map(|k| t0 + k as f64 * period).collect();
    // bar 1: the beat of four whose low band is loudest, summed over the bars
    let bpb = 4usize;
    let mut acc = [0.0f64; 4];
    for (k, t) in beats.iter().enumerate() {
        let x = t * fps;
        if (x as usize) < low.len() {
            // a small window: the onset peak may be a hop off the comb
            let v = env_at(&low, x).max(env_at(&low, x + 1.0)).max(env_at(&low, x - 1.0));
            acc[k % bpb] += v;
        }
    }
    let (down, top) = acc
        .iter()
        .enumerate()
        .fold((0usize, f64::MIN), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
    let total: f64 = acc.iter().sum();
    let share = if total > 0.0 { top / total } else { 0.25 };
    let phase_clarity = ((share - 0.25) / 0.75).clamp(0.0, 1.0);
    let conf = (0.35 + 0.65 * contrast.clamp(0.0, 1.0)) * (0.6 + 0.4 * phase_clarity);
    Some(BeatGrid {
        source: GridSource::Dsp,
        period,
        beats_per_bar: bpb as u32,
        steady: true,
        confidence: conf.clamp(0.0, 0.7),
        residual_ms: 0.0,
        beats,
        first_downbeat: down,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// A click track: a click every beat (a louder, lower one on bar 1), `first` seconds in.
    pub(crate) fn click_track(bpm: f64, first: f64, seconds: f64, sr: f64, accent: bool) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        let mut v = vec![0.0f32; n];
        let period = 60.0 / bpm;
        let mut k = 0usize;
        loop {
            let t = first + k as f64 * period;
            if t >= seconds - 0.05 {
                break;
            }
            let down = k % 4 == 0;
            let (f, a) = if down && accent { (110.0, 1.0) } else { (880.0, 0.5) };
            let start = (t * sr) as usize;
            for i in 0..(0.04 * sr) as usize {
                if start + i < n {
                    let x = i as f64 / sr;
                    v[start + i] += (a * exp(-x * 90.0) * sin(2.0 * core::f64::consts::PI * f * x)) as f32;
                }
            }
            k += 1;
        }
        v
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        fabs(a - b) <= tol
    }

    #[test]
    fn fits_steady_beats_and_puts_bar_one_on_a_downbeat() {
        let p = 60.0 / 124.0;
        // beats from 0.30 s; the downbeats are the 3rd beat of the first four (index 2) and every 4th
        let beats: Vec<f64> = (0..48).map(|k| 0.30 + k as f64 * p).collect();
        let downs: Vec<f64> = (0..12).map(|k| beats[2 + 4 * k]).collect();
        let g = fit_grid(&beats, &downs, 25.0, FitOpts::default()).unwrap();
        assert!(g.steady);
        assert!(near(g.bpm(), 124.0, 0.01), "{}", g.bpm());
        assert_eq!(g.beats_per_bar, 4);
        assert!(near(g.first_downbeat_time(), beats[2], 0.002), "{}", g.first_downbeat_time());
        assert!(g.confidence > 0.8, "{}", g.confidence);
    }

    #[test]
    fn jittery_beats_with_missing_and_extra_ones_still_make_one_grid() {
        let p = 0.5;
        let mut beats: Vec<f64> = (0..40).map(|k| 1.0 + k as f64 * p + if k % 3 == 0 { 0.012 } else { -0.008 }).collect();
        beats.remove(7); // a missed beat
        beats.insert(12, 1.0 + 11.4 * p); // a stray one
        let downs: Vec<f64> = (0..10).map(|k| 1.0 + (1 + 4 * k) as f64 * p).collect();
        let g = fit_grid(&beats, &downs, 21.0, FitOpts::default()).unwrap();
        assert!(g.steady, "{g:?}");
        assert!(near(g.period, 0.5, 0.002));
        assert!(near(g.first_downbeat_time(), 1.0 + p, 0.003));
    }

    #[test]
    fn a_drifting_track_keeps_its_beats() {
        // 100 -> 120 bpm over the track
        let mut t = 0.5;
        let mut beats = vec![];
        for k in 0..60 {
            beats.push(t);
            let bpm = 100.0 + 20.0 * k as f64 / 60.0;
            t += 60.0 / bpm;
        }
        let downs: Vec<f64> = beats.iter().step_by(4).copied().collect();
        let g = fit_grid(&beats, &downs, t, FitOpts::default()).unwrap();
        assert!(!g.steady);
        assert_eq!(g.first_downbeat, 0);
        // and the beats are the detected ones
        assert!(near(g.time(10), beats[10], 1e-9));
    }

    #[test]
    fn octave_errors_are_folded_into_range() {
        // beats at 62 bpm come out of a detector that ran at half the tempo: fold up to 124
        let p = 60.0 / 62.0;
        let beats: Vec<f64> = (0..20).map(|k| k as f64 * p).collect();
        let g = fit_grid(&beats, &[], 20.0, FitOpts::default()).unwrap();
        assert!(g.bpm() >= BPM_RANGE.0 && g.bpm() <= BPM_RANGE.1, "{}", g.bpm());
    }

    #[test]
    fn beats_quantised_to_the_models_frames_still_fit_one_tempo() {
        // Beat This! reports beats on a 20 ms grid, so the intervals alternate (0.60, 0.62 ...)
        // and a rough period must not walk the numbering off over a long track
        for bpm in [98.0, 124.0, 140.0] {
            let p = 60.0 / bpm;
            let beats: Vec<f64> = (0..90).map(|k| round((0.5 + k as f64 * p) / 0.02) * 0.02).collect();
            let downs: Vec<f64> = beats.iter().step_by(4).copied().collect();
            let g = fit_grid(&beats, &downs, 60.0, FitOpts::default()).unwrap();
            assert!(g.steady, "{bpm}: {g:?}");
            assert!(near(g.bpm(), bpm, bpm * 0.002), "{bpm}: {}", g.bpm());
            assert!(g.confidence > 0.7, "{bpm}: {}", g.confidence);
        }
    }

    #[test]
    fn a_grid_a_frame_off_is_moved_onto_the_attacks() {
        // the model's beats are 15 ms late of the clicks; the clicks' onsets say where they are
        let p = 0.5;
        let truth: Vec<f64> = (0..40).map(|k| 0.30 + k as f64 * p).collect();
        let late: Vec<f64> = truth.iter().map(|t| t + 0.015).collect();
        let downs: Vec<f64> = late.iter().step_by(4).copied().collect();
        let mut g = fit_grid(&late, &downs, 21.0, FitOpts::default()).unwrap();
        let onsets: Vec<(f64, f64)> = truth.iter().map(|t| (t + 0.002, 1.0)).collect();
        let shift = refine_to_onsets(&mut g, &onsets, 0.045).unwrap();
        assert!(near(shift, -0.013, 0.002), "{shift}");
        assert!(near(g.first_downbeat_time(), 0.30 + 0.002, 0.001), "{}", g.first_downbeat_time());
        // too few onsets, or onsets that disagree: left alone
        let mut g2 = fit_grid(&late, &downs, 21.0, FitOpts::default()).unwrap();
        assert!(refine_to_onsets(&mut g2, &onsets[..5], 0.045).is_none());
        let scattered: Vec<(f64, f64)> =
            truth.iter().enumerate().map(|(k, t)| (t + if k % 2 == 0 { 0.03 } else { -0.03 }, 1.0)).collect();
        assert!(refine_to_onsets(&mut g2, &scattered, 0.045).is_none());
        assert!(near(g2.first_downbeat_time(), 0.315, 0.001), "left alone: {}", g2.first_downbeat_time());
    }

    #[test]
    fn too_few_beats_make_no_grid() {
        assert!(fit_grid(&[0.5, 1.0, 1.5], &[0.5], 3.0, FitOpts::default()).is_none());
    }

    #[test]
    fn trim_starts_on_the_downbeat_and_runs_whole_bars() {
        let sr = 48_000.0;
        let p = 0.5; // 120 bpm, bar = 2 s
        let beats: Vec<f64> = (0..20).map(|k| 0.9 + k as f64 * p).collect();
        let downs: Vec<f64> = (0..5).map(|k| beats[1 + 4 * k]).collect();
        let g = fit_grid(&beats, &downs, 10.0, FitOpts::default()).unwrap();
        // downbeat at 1.4 s, 10 s of audio: (10 - 1.4)/2 = 4.3 bars -> 4 bars
        let t = trim_to_bars(&g, (10.0 * sr) as usize, sr, TrimOpts::default()).unwrap();
        assert_eq!(t.bars, 4);
        assert_eq!(t.beats, 16);
        assert!(near(t.start as f64 / sr, 1.4, 0.001));
        assert!(near(t.bpm, 120.0, 0.05), "{}", t.bpm);
        assert!(t.changed);
    }

    #[test]
    fn a_loop_already_on_the_downbeat_is_not_cut_at_the_head() {
        let sr = 48_000.0;
        let beats: Vec<f64> = (0..16).map(|k| 0.012 + k as f64 * 0.5).collect();
        let downs: Vec<f64> = beats.iter().step_by(4).copied().collect();
        let g = fit_grid(&beats, &downs, 8.0, FitOpts::default()).unwrap();
        let t = trim_to_bars(&g, (8.0 * sr) as usize, sr, TrimOpts::default()).unwrap();
        assert_eq!(t.start, 0);
        assert_eq!(t.bars, 4); // 7.988 s of music after the downbeat is 3.99 bars: within tolerance
        assert!(t.frames <= (8.0 * sr) as usize);
    }

    #[test]
    fn less_than_a_bar_is_not_trimmed() {
        let sr = 48_000.0;
        let beats: Vec<f64> = (0..8).map(|k| 0.5 + k as f64 * 0.5).collect();
        let g = fit_grid(&beats, &[0.5, 2.5], 3.0, FitOpts::default()).unwrap();
        assert!(trim_to_bars(&g, (2.2 * sr) as usize, sr, TrimOpts::default()).is_none());
    }

    #[test]
    fn latency_from_delayed_beats() {
        let p = 60.0 / 128.0;
        // the clock's beat 0 is at t = 0; the source arrives 35 ms late, ±4 ms of jitter
        let beats: Vec<f64> = (0..32)
            .map(|k| k as f64 * p + 0.035 + if k % 2 == 0 { 0.004 } else { -0.004 })
            .collect();
        let e = estimate_latency(&beats, &[], p, 0.0).unwrap();
        assert!(near(e.ms, 35.0, 1.5), "{e:?}");
        assert!(e.confidence > 0.9, "{e:?}");
    }

    #[test]
    fn latency_is_negative_for_early_beats_and_survives_wrapping() {
        let p = 0.5;
        let beats: Vec<f64> = (0..20).map(|k| k as f64 * p - 0.02).collect();
        let e = estimate_latency(&beats, &[], p, 0.0).unwrap();
        assert!(near(e.ms, -20.0, 0.5), "{e:?}");
        // beats scattered all over the beat: nothing to say
        let noise: Vec<f64> = (0..20).map(|k| k as f64 * p + ((k * 7) % 10) as f64 * 0.05).collect();
        let e = estimate_latency(&noise, &[], p, 0.0).unwrap();
        assert!(e.confidence < 0.5, "{e:?}");
    }

    #[test]
    fn frame_grid_lines_and_snapping() {
        let g = FrameGrid { origin: 1000.0, period: 500.0, beats_per_bar: 4 };
        assert_eq!(g.bar_frames(), 2000.0);
        assert_eq!(g.nearest(2900.0, Unit::Bar), (1, 3000.0));
        assert_eq!(g.nearest(2900.0, Unit::Beat), (4, 3000.0));
        assert_eq!(g.at_or_before(2999.0, Unit::Bar), (0, 1000.0));
        assert_eq!(g.at_or_before(3000.0, Unit::Bar), (1, 3000.0));
        assert_eq!(g.after(3000.0, Unit::Bar), (2, 5000.0));
        // before the origin the grid goes on
        assert_eq!(g.nearest(-900.0, Unit::Bar), (-1, -1000.0));
        let l = g.lines_between(900.0, 3600.0, Unit::Beat);
        let frames: Vec<f64> = l.iter().map(|x| x.0).collect();
        assert_eq!(frames, vec![1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 3500.0]);
        assert!(l[0].1 && !l[1].1 && l[4].1);
    }

    #[test]
    fn spans_are_exact_bars_on_bar_lines() {
        let g = FrameGrid { origin: 1000.0, period: 22_050.0, beats_per_bar: 4 };
        let bar = g.bar_frames();
        // a head 0.4 bar past bar 3's line snaps back to it
        let head = g.line(3, Unit::Bar) + 0.4 * bar;
        let (a, b) = g.span(head, 2, Anchor::End, 0.0, 1.0e7).unwrap();
        assert_eq!(b, g.line(3, Unit::Bar).round() as i64);
        assert_eq!(b - a, (2.0 * bar).round() as i64);
        assert_eq!(a, g.line(1, Unit::Bar).round() as i64);
        let (a, b) = g.span(head, 4, Anchor::Start, 0.0, 1.0e7).unwrap();
        assert_eq!(a, g.line(3, Unit::Bar).round() as i64);
        assert_eq!(b - a, (4.0 * bar).round() as i64);
        // 0.6 bar past it snaps forward to bar 4
        let (_, b) = g.span(g.line(3, Unit::Bar) + 0.6 * bar, 1, Anchor::End, 0.0, 1.0e7).unwrap();
        assert_eq!(b, g.line(4, Unit::Bar).round() as i64);
        // outside what is recorded
        assert!(g.span(head, 2, Anchor::Start, 0.0, head).is_none());
        assert!(g.span(g.line(1, Unit::Bar), 2, Anchor::End, 2000.0, 1.0e7).is_none());
        // stepping
        assert_eq!(g.step(head, -1, Unit::Bar), g.line(2, Unit::Bar));
        // the head is 1.6 beats past the line: the nearest beat is the 2nd, two on is the 4th
        assert_eq!(g.step(head, 2, Unit::Beat), g.line(3, Unit::Bar) + 4.0 * g.period);
    }

    #[test]
    fn dsp_finds_the_tempo_and_phase_of_a_click_track() {
        let sr = 44_100.0;
        for (bpm, first) in [(124.0, 0.30), (96.0, 0.11), (150.0, 0.45)] {
            let audio = click_track(bpm, first, 24.0, sr, true);
            let g = dsp_analyze(&audio, sr, DspOpts::default()).unwrap();
            assert!(near(g.bpm(), bpm, bpm * 0.004), "{bpm}: {}", g.bpm());
            // a beat lands within 12 ms of a click
            let p = 60.0 / bpm;
            let off = (g.time(0) - first).rem_euclid(p);
            let off = off.min(p - off);
            assert!(off < 0.012, "{bpm}: phase off by {off}");
            assert!(g.confidence > 0.3, "{bpm}: {}", g.confidence);
        }
    }

    #[test]
    fn dsp_puts_bar_one_on_the_accented_beat() {
        let sr = 44_100.0;
        let first = 0.37;
        let audio = click_track(128.0, first, 24.0, sr, true);
        let g = dsp_analyze(&audio, sr, DspOpts::default()).unwrap();
        // the accent (low, loud click) is on the clicks numbered 0, 4, 8 ...: bar 1 lands there
        let p = 60.0 / 128.0;
        let n = (g.first_downbeat_time() - first) / p;
        let off = (n - round(n)) * p;
        assert!(fabs(off) < 0.012, "off the click by {off}");
        assert_eq!((round(n) as i64).rem_euclid(4), 0, "bar 1 is click {n}");
    }

    #[test]
    fn dsp_says_nothing_about_silence_or_a_blip() {
        assert!(dsp_analyze(&vec![0.0f32; 44_100 * 10], 44_100.0, DspOpts::default()).is_none());
        assert!(dsp_analyze(&vec![0.1f32; 1000], 44_100.0, DspOpts::default()).is_none());
    }
}
