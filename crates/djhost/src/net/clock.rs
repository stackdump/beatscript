//! The shared beat clock (docs/NET-PROTOCOL.md §6): each side's monotonic time, the NTP-style
//! offset between two sides, and beat points (where a master is on its grid, and when).
//!
//! Pure apart from [`mono_ns`] and [`master_point`]; no threads.

use std::sync::OnceLock;
use std::time::Instant;

use djengine::json::{JsonObject, JsonValue};

use crate::deck::Engine;
use djengine::mixer::TempoSource;

/// This process's monotonic clock, in ns since its first use. Never the wall clock: a phone's
/// NTP can jump.
pub fn mono_ns() -> i64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as i64
}

/// Where one side's master is on its beat grid (§6.3).
#[derive(Clone, Debug, PartialEq)]
pub struct BeatPoint {
    pub bpm: f64,
    /// Beats since the grid origin (beat 0 = bar 0's downbeat; 4 beats a bar).
    pub beat: f64,
    /// The side's `mono_ns` when the master rendered `beat`.
    pub t: i64,
    /// The render frame `beat` falls on (the sender's own frame count; 0 when not known).
    pub frame: i64,
    pub rate: f64,
    /// Something plays: when false the tempo holds, the phase means nothing.
    pub playing: bool,
    /// What sets the tempo: a deck's name, `free`.
    pub source: String,
    /// The side's output latency, ns (its acoustic beat is `t + out_ns`).
    pub out_ns: i64,
}

impl BeatPoint {
    pub fn beats_per_ns(&self) -> f64 {
        self.bpm / 60e9
    }
    /// The beat at time `t` on the same clock as [`BeatPoint::t`].
    pub fn beat_at(&self, t: i64) -> f64 {
        self.beat + (t - self.t) as f64 * self.beats_per_ns()
    }
    /// The beat at render frame `f`.
    pub fn beat_at_frame(&self, f: i64) -> f64 {
        if self.rate <= 0.0 {
            return self.beat;
        }
        self.beat + (f - self.frame) as f64 * self.bpm / 60.0 / self.rate
    }
    /// This point on another clock: `t + offset` (offset = that clock − this one).
    pub fn shifted(&self, offset_ns: i64) -> BeatPoint {
        BeatPoint {
            t: self.t + offset_ns,
            ..self.clone()
        }
    }

    pub fn to_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("bpm", JsonValue::from(self.bpm)),
            ("beat", JsonValue::from(self.beat)),
            ("t", JsonValue::int(self.t)),
            ("frame", JsonValue::int(self.frame)),
            ("rate", JsonValue::from(self.rate)),
            ("playing", self.playing.into()),
            ("source", JsonValue::str(self.source.clone())),
            ("out_ns", JsonValue::int(self.out_ns)),
        ]))
    }

    pub fn from_json(v: &JsonValue) -> Option<BeatPoint> {
        let n = |k: &str| v.get(k).and_then(JsonValue::as_f64);
        let bpm = n("bpm").filter(|b| b.is_finite() && *b > 0.0)?;
        Some(BeatPoint {
            bpm,
            beat: n("beat").filter(|b| b.is_finite())?,
            t: n("t")? as i64,
            frame: n("frame").unwrap_or(0.0) as i64,
            rate: n("rate").unwrap_or(0.0),
            playing: v.get("playing").and_then(JsonValue::as_bool).unwrap_or(false),
            source: v
                .get("source")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string(),
            out_ns: n("out_ns").unwrap_or(0.0) as i64,
        })
    }
}

/// `x` folded into −0.5 ..< 0.5 (a phase difference in beats).
pub fn wrap_half(x: f64) -> f64 {
    let w = x - x.round();
    if w >= 0.5 {
        w - 1.0
    } else {
        w
    }
}

/// When each render frame was rendered, on this process's clock. The main thread sees the
/// rendered frame count only when it looks (every pass, ±a block and a pass late, and later when
/// the renderer runs in bursts), so `now − frame/rate` is the true line plus a positive lag: the
/// least of it over the last two seconds is the line. Two seconds is short enough to follow the
/// device clock's drift against this one.
#[derive(Clone, Debug, Default)]
pub struct FrameClock {
    seen: std::collections::VecDeque<(i64, i64)>,
    rate: f64,
}

impl FrameClock {
    const WINDOW_NS: i64 = 2_000_000_000;

    /// Saw `frame` rendered at `now`; the time it was rendered at.
    pub fn push(&mut self, frame: i64, now: i64, rate: f64) -> i64 {
        if rate <= 0.0 {
            return now;
        }
        if (rate - self.rate).abs() > 1e-9 {
            self.seen.clear();
            self.rate = rate;
        }
        let off = now - (frame as f64 * 1e9 / rate) as i64;
        // a frame count that went back (a new engine) starts again
        if self.seen.back().is_some_and(|(_, o)| off < *o - Self::WINDOW_NS) {
            self.seen.clear();
        }
        self.seen.push_back((now, off));
        while self.seen.front().is_some_and(|(t, _)| now - t > Self::WINDOW_NS) {
            self.seen.pop_front();
        }
        self.time_of(frame).unwrap_or(now)
    }

    /// When `frame` was (or will be) rendered.
    pub fn time_of(&self, frame: i64) -> Option<i64> {
        let off = self.seen.iter().map(|(_, o)| *o).min()?;
        Some(off + (frame as f64 * 1e9 / self.rate) as i64)
    }
}

/// [`master_point`] dated by a [`FrameClock`]: `t` is when its frame was rendered, not when the
/// main thread looked.
pub fn master_point_dated(engine: &Engine, out_ns: i64, fc: &mut FrameClock) -> BeatPoint {
    let mut p = master_point(engine, out_ns);
    p.t = fc.push(p.frame, p.t, p.rate);
    p
}

/// The engine's master as a beat point, now: the master-tempo deck's beat clock, else the free
/// tempo running from frame 0 (`playing` false). The rendered frame is read before and after,
/// so a render block landing in between is retried. `out_ns`: the output's latency. `t` is when
/// the main thread looked: [`master_point_dated`] dates it by the frame instead.
pub fn master_point(engine: &Engine, out_ns: i64) -> BeatPoint {
    let rate = engine.sample_rate;
    let mut last = None;
    for _ in 0..4 {
        let f0 = engine.loop_frames();
        let (src, bpm) = engine.master_tempo();
        let deck = match &src {
            TempoSource::Deck(d) => engine.beat_state(d).map(|b| (d.clone(), b)),
            TempoSource::Free => None,
        };
        let t = mono_ns();
        let p = match deck {
            Some((d, b)) => BeatPoint {
                bpm: b.bpm,
                beat: b.beat,
                t,
                frame: f0,
                rate,
                playing: engine.deck(&d).is_some_and(|m| m.mirror.is_playing()),
                source: d,
                out_ns,
            },
            None => BeatPoint {
                bpm,
                beat: f0 as f64 * bpm / 60.0 / rate.max(1.0),
                t,
                frame: f0,
                rate,
                playing: false,
                source: "free".into(),
                out_ns,
            },
        };
        let same = engine.loop_frames() == f0;
        last = Some(p);
        if same {
            break;
        }
    }
    last.expect("one pass")
}

/// One NTP exchange: the peer's send `t1`, the host's receive `t2` and send `t3`, the peer's
/// receive `t4`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exchange {
    pub t1: i64,
    pub t2: i64,
    pub t3: i64,
    pub t4: i64,
}

impl Exchange {
    /// Host clock − peer clock.
    pub fn offset(&self) -> i64 {
        ((self.t2 - self.t1) + (self.t3 - self.t4)) / 2
    }
    /// The round trip less the host's turnaround.
    pub fn delay(&self) -> i64 {
        (self.t4 - self.t1) - (self.t3 - self.t2)
    }
}

/// The offset from the last [`OffsetEstimator::KEEP`] exchanges: the one with the least delay
/// (queueing only ever adds), among those within 3× of it.
#[derive(Clone, Debug, Default)]
pub struct OffsetEstimator {
    samples: Vec<Exchange>,
}

impl OffsetEstimator {
    pub const KEEP: usize = 16;

    pub fn push(&mut self, e: Exchange) {
        if e.delay() < 0 {
            return; // a clock went backwards or the stamps are wrong
        }
        self.samples.push(e);
        let over = self.samples.len().saturating_sub(Self::KEEP);
        self.samples.drain(..over);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// (offset, delay) of the best exchange.
    pub fn best(&self) -> Option<(i64, i64)> {
        let best = self.samples.iter().min_by_key(|e| e.delay())?;
        Some((best.offset(), best.delay()))
    }

    /// How far the kept offsets spread (max − min) among the good exchanges, ns: a rough error bar.
    pub fn spread(&self) -> i64 {
        let Some((_, d)) = self.best() else { return 0 };
        let good: Vec<i64> = self
            .samples
            .iter()
            .filter(|e| e.delay() <= 3 * d.max(1))
            .map(Exchange::offset)
            .collect();
        good.iter().max().unwrap_or(&0) - good.iter().min().unwrap_or(&0)
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Beat points smoothed over a few seconds: the slope is fixed at the tempo, the intercept is
/// the mean of the points carried to the newest one's time. A tempo change or a jump of more
/// than a sixteenth (a seek, a load) starts again.
#[derive(Clone, Debug, Default)]
pub struct BeatFit {
    points: Vec<BeatPoint>,
}

impl BeatFit {
    const KEEP: usize = 48;

    pub fn push(&mut self, p: BeatPoint) {
        if let Some(last) = self.points.last() {
            let tempo_changed = (last.bpm - p.bpm).abs() > 1e-6;
            let jumped = (self.predict(p.t).unwrap_or(p.beat) - p.beat).abs() > 0.25;
            if tempo_changed || jumped || last.playing != p.playing || p.t < last.t {
                self.points.clear();
            }
        }
        self.points.push(p);
        let over = self.points.len().saturating_sub(Self::KEEP);
        self.points.drain(..over);
    }

    fn predict(&self, t: i64) -> Option<f64> {
        let p = self.point()?;
        Some(p.beat_at(t))
    }

    /// The smoothed point, dated at the newest sample.
    pub fn point(&self) -> Option<BeatPoint> {
        let last = self.points.last()?;
        let sum: f64 = self.points.iter().map(|p| p.beat_at(last.t) - last.beat).sum();
        Some(BeatPoint {
            beat: last.beat + sum / self.points.len() as f64,
            ..last.clone()
        })
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(bpm: f64, beat: f64, t: i64) -> BeatPoint {
        BeatPoint {
            bpm,
            beat,
            t,
            frame: 0,
            rate: 48_000.0,
            playing: true,
            source: "A".into(),
            out_ns: 0,
        }
    }

    #[test]
    fn the_offset_comes_from_the_least_delayed_exchange() {
        // host clock = peer clock + 5 ms; one-way 2 ms, then a congested one
        let off = 5_000_000;
        let mut est = OffsetEstimator::default();
        let ex = |t1: i64, up: i64, down: i64| Exchange {
            t1,
            t2: t1 + up + off,
            t3: t1 + up + off + 100_000,
            t4: t1 + up + 100_000 + down,
        };
        est.push(ex(0, 9_000_000, 1_000_000)); // asymmetric queueing: offset off by 4 ms
        est.push(ex(1_000_000_000, 2_000_000, 2_000_000));
        let (o, d) = est.best().unwrap();
        assert_eq!(o, off);
        assert_eq!(d, 4_000_000);
    }

    #[test]
    fn a_beat_point_carries_to_any_time_and_frame() {
        let mut p = point(120.0, 10.0, 1_000_000_000);
        assert!((p.beat_at(1_500_000_000) - 11.0).abs() < 1e-9);
        p.frame = 48_000;
        assert!((p.beat_at_frame(72_000) - 11.0).abs() < 1e-9);
        let j = BeatPoint::from_json(&p.to_json()).unwrap();
        assert_eq!(j, p);
        assert_eq!(p.shifted(7).t, 1_000_000_007);
    }

    #[test]
    fn the_fit_averages_jitter_away_and_restarts_on_a_jump() {
        let mut fit = BeatFit::default();
        let bpm = 128.0;
        for k in 0..40i64 {
            let t = k * 50_000_000;
            let true_beat = 100.0 + t as f64 * bpm / 60e9;
            let jitter = if k % 2 == 0 { 0.01 } else { -0.01 };
            fit.push(point(bpm, true_beat + jitter, t));
        }
        let p = fit.point().unwrap();
        let want = 100.0 + p.t as f64 * bpm / 60e9;
        assert!((p.beat - want).abs() < 0.002, "{} vs {want}", p.beat);
        fit.push(point(bpm, 500.0, 2_000_000_000 + 50_000_000));
        assert_eq!(fit.point().unwrap().beat, 500.0);
    }

    #[test]
    fn the_frame_clock_finds_the_render_line_under_late_looks() {
        let mut fc = FrameClock::default();
        let rate = 48_000.0;
        // frames rendered in 512-frame blocks from t = 1 s; looked at 0-30 ms late
        let mut worst = 0i64;
        for k in 0..400i64 {
            let frame = k * 512;
            let rendered = 1_000_000_000 + (frame as f64 * 1e9 / rate) as i64;
            let late = (k * 7919 % 31) * 1_000_000;
            let t = fc.push(frame, rendered + late, rate);
            if k > 200 {
                worst = worst.max((t - rendered).abs());
            }
        }
        assert!(worst < 1_000_000, "worst {worst} ns");
    }

    #[test]
    fn phase_wraps_to_half_a_beat() {
        assert!((wrap_half(3.25) - 0.25).abs() < 1e-12);
        assert!((wrap_half(-0.75) - 0.25).abs() < 1e-12);
        assert!((wrap_half(0.5) + 0.5).abs() < 1e-12);
    }
}
