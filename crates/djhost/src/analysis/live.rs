//! The analysis and the live engine: where an input's history head is, snapping it to the
//! grid, a bar-exact loop out of the recording, a freeze timed to a bar line, and the clock's
//! reference for a latency estimate. Everything reads the strip's recording (its
//! [`InputTap`](djrust::host::deck::input::InputTap): frames counted from the first one
//! recorded) and the grid in those same frames ([`FrameGrid`]), so nothing here is measured
//! against a tempo the player may not be keeping.
//!
//! Main thread only. Nothing here touches the render thread except through the engine's own
//! history, freeze and clip calls.

use djengine::beatgrid::{Anchor, FrameGrid, Unit};
use djengine::history::{self, Mode};

use djengine::deck::turntable::SampleBuffer;
use crate::deck::Engine;

/// A stretch of an input's recording.
#[derive(Clone, Debug)]
pub struct TapWindow {
    /// Recording frame of `audio[0]`.
    pub start: i64,
    /// One past the last frame.
    pub end: i64,
    pub audio: Vec<f32>,
    pub sample_rate: f64,
}

/// The last `seconds` (at least 2) of input `i`'s recording, or None without a recording.
pub fn tap_window(e: &Engine, i: usize, seconds: f64) -> Option<TapWindow> {
    let tap = e.inputs.get(i)?.mirror.tap.get()?;
    let sr = e.sample_rate;
    let end = tap.end();
    let start = (end - (seconds * sr) as i64).max(tap.oldest());
    if (end - start) as f64 / sr < 2.0 {
        return None;
    }
    let mut audio = Vec::with_capacity((end - start) as usize);
    if tap.copy(start, end, &mut audio) != end || audio.len() as i64 != end - start {
        return None;
    }
    Some(TapWindow { start, end, audio, sample_rate: sr })
}

/// Where input `i`'s history head is, in recording frames.
#[derive(Clone, Copy, Debug)]
pub struct Head {
    /// The recording's end (live).
    pub end: i64,
    /// Frames the head is behind live (0 when live or looping).
    pub lag: f64,
    /// `end - lag`: the head's own frame.
    pub head: f64,
    pub mode: Mode,
    /// Frames of history held, at most.
    pub held: f64,
}

pub fn head(e: &Engine, i: usize) -> Option<Head> {
    let inp = e.inputs.get(i)?;
    let tap = inp.mirror.tap.get()?;
    let mode = inp.mirror.history_mode();
    let end = tap.end();
    let fpb = history::frames_per_beat(e.master_tempo().1, e.sample_rate);
    // the lag is published in beats of the tempo the history was last told: at the master's
    let lag = match mode {
        Mode::Touch | Mode::Play => inp.mirror.history_lag_beats() as f64 * fpb,
        _ => 0.0,
    };
    let held = (inp.mirror.history_filled_beats() as f64 * fpb).min((end - tap.oldest()) as f64);
    Some(Head { end, lag, head: end as f64 - lag, mode, held })
}

/// What a snap did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moved {
    pub unit: Unit,
    /// The line's number counted from bar 1 (in `unit`s).
    pub line: i64,
    /// Frames the head is now behind live.
    pub lag_frames: f64,
    /// How far it moved, ms (positive: toward live).
    pub moved_ms: f64,
    /// True when it ended on live (the line was at or past the newest frame).
    pub live: bool,
}

/// Move input `i`'s history head onto a line of `grid`: the one nearest it (`steps` 0) or `steps`
/// lines on from that one (negative: back in time). A head that is live starts from the last
/// line already recorded. The head ends up playing on from the line (a delay), as when a platter
/// is let go; a line at or past live is live.
pub fn snap_head(e: &mut Engine, i: usize, grid: &FrameGrid, unit: Unit, steps: i64) -> Result<Moved, String> {
    if e.is_frozen(i) {
        return Err("the strip is frozen: the wheel moves the freeze, thaw it first".into());
    }
    let h = head(e, i).ok_or("no recording on that input")?;
    let sr = e.sample_rate;
    let from = if h.mode == Mode::Live || h.mode == Mode::Loop { h.end as f64 } else { h.head };
    let (mut k, mut line) = if matches!(h.mode, Mode::Live | Mode::Loop) {
        grid.at_or_before(from, unit)
    } else {
        grid.nearest(from, unit)
    };
    if steps != 0 {
        k += steps;
        line = grid.line(k, unit);
    }
    if line >= h.end as f64 - 1.0 {
        e.input_live(i);
        return Ok(Moved { unit, line: k, lag_frames: 0.0, moved_ms: (h.end as f64 - from) / sr * 1000.0, live: true });
    }
    if h.end as f64 - line > h.held {
        return Err(format!(
            "that line is {:.1} s back, before the history begins ({:.1} s kept)",
            (h.end as f64 - line) / sr,
            h.held / sr
        ));
    }
    let delta = line - from;
    if !(e.input_touch(i) && e.input_scrub(i, delta / sr)) {
        return Err("the strip keeps no history".into());
    }
    e.input_release(i);
    Ok(Moved { unit, line: k, lag_frames: h.end as f64 - line, moved_ms: delta / sr * 1000.0, live: false })
}

/// A loop of exactly `bars` bars cut from the recording around the head (snapped to the nearest
/// bar line, or the last one recorded when live) and put on the loop row, saved to the library.
#[derive(Clone, Debug)]
pub struct BarLoop {
    pub id: String,
    pub bars: u32,
    pub beats: i64,
    /// The tempo of the music (the grid's), worked out from the cut's real length.
    pub bpm: f64,
    /// Recording frames, start .. end.
    pub start: i64,
    pub end: i64,
    pub title: String,
}

pub fn capture_bars(
    e: &mut Engine,
    i: usize,
    grid: &FrameGrid,
    bars: u32,
    anchor: Anchor,
) -> Result<BarLoop, String> {
    let h = head(e, i).ok_or("no recording on that input")?;
    let tap = e.inputs[i].mirror.tap.get().ok_or("no recording on that input")?;
    let at = match h.mode {
        Mode::Touch | Mode::Play => h.head,
        _ => grid.at_or_before(h.end as f64, Unit::Bar).1,
    };
    let (a, b) = grid
        .span(at, bars, anchor, tap.oldest() as f64, tap.end() as f64)
        .ok_or_else(|| {
            format!(
                "{bars} bar{} {} the head does not fit in the recording (try fewer bars or anchor {})",
                if bars == 1 { "" } else { "s" },
                if anchor == Anchor::End { "before" } else { "after" },
                if anchor == Anchor::End { "start" } else { "end" }
            )
        })?;
    let mut audio = Vec::with_capacity((b - a) as usize);
    if tap.copy(a, b, &mut audio) != b || audio.len() as i64 != b - a {
        return Err("the recording moved on while it was cut: try again".into());
    }
    let sr = e.sample_rate;
    let beats = (bars * grid.beats_per_bar) as i64;
    let bpm = beats as f64 * 60.0 * sr / audio.len() as f64;
    let label = e.inputs[i].settings.label.clone();
    let title = format!("{label} {bars}bar · {bpm:.0}");
    let id = e
        .import_clip(SampleBuffer::new(audio, None, sr), &title, Some(bpm), beats, None)
        .ok_or("the engine would not take the loop")?;
    e.library_save(&id);
    Ok(BarLoop { id, bars, beats, bpm, start: a, end: b, title })
}

/// Loop `bars` bars of the history in place (the strip's own loop, not the row): the head snaps
/// to a bar line and the loop is the bars before it, played at the master tempo as the strip's
/// loop always is. Returns where the head went.
pub fn loop_bars(e: &mut Engine, i: usize, grid: &FrameGrid, bars: u32) -> Result<Moved, String> {
    let mv = snap_head(e, i, grid, Unit::Bar, 0)?;
    let fpb = history::frames_per_beat(e.master_tempo().1, e.sample_rate);
    let beats = grid.bar_frames() * bars as f64 / fpb;
    if !e.input_loop(i, beats) {
        return Err("the strip keeps no history".into());
    }
    Ok(mv)
}

/// A freeze waiting for its bar line.
#[derive(Clone, Copy, Debug)]
pub struct PendingFreeze {
    pub input: usize,
    /// The recording frame the frozen audio should end on (a bar line).
    pub line: f64,
    /// Beats (of the master tempo) the frozen audio spans: `bars` bars of the music.
    pub beats: f64,
    pub bars: u32,
    /// Fire this many frames before the line (the time from asking to the render thread
    /// acting on it, a block or a tick).
    pub lead: f64,
}

/// What a fired freeze reached.
#[derive(Clone, Copy, Debug)]
pub struct Froze {
    /// Where the frozen audio ends, estimated, in frames from the bar line (negative: early).
    pub end_error_frames: f64,
    pub beats: f64,
}

/// Plan a freeze of `bars` bars ending on the next bar line the recording reaches (at least
/// `lead` ahead).
pub fn plan_freeze(e: &Engine, i: usize, grid: &FrameGrid, bars: u32, lead: f64) -> Result<PendingFreeze, String> {
    let h = head(e, i).ok_or("no recording on that input")?;
    let fpb = history::frames_per_beat(e.master_tempo().1, e.sample_rate);
    let beats = grid.bar_frames() * bars as f64 / fpb;
    let max = djengine::fx::granular::MAX_FREEZE_BEATS;
    if beats > max {
        let fit = (max * fpb / grid.bar_frames()).floor() as u32;
        return Err(format!(
            "{bars} bars is {beats:.1} beats at the master tempo; a freeze holds at most {max} (try {})",
            fit.max(1)
        ));
    }
    let (_, line) = grid.after(h.end as f64 + lead, Unit::Bar);
    Ok(PendingFreeze { input: i, line, beats, bars, lead })
}

/// Call every block (or tick): when the recording gets within `lead` of the line, freeze. None
/// until then.
pub fn service_freeze(e: &mut Engine, p: &PendingFreeze) -> Option<Result<Froze, String>> {
    let tap = e.inputs.get(p.input)?.mirror.tap.get()?;
    let end = tap.end() as f64;
    if end + p.lead < p.line {
        return None;
    }
    if !e.input_freeze(p.input, true, Some(p.beats), Some(false)) {
        return Some(Err("the strip keeps no history".into()));
    }
    // the render thread acts on it at the end of the next block: about `lead` on
    Some(Ok(Froze { end_error_frames: end + p.lead - p.line, beats: p.beats }))
}

/// What a latency estimate is measured against.
#[derive(Clone, Debug)]
pub struct LatencyRef {
    pub window: TapWindow,
    /// Seconds between the clock's beats.
    pub period: f64,
    /// Seconds (from the window's start) of one of the clock's beats, as the capture path puts
    /// it: the recording frame of clock beat `b` is `end - (beat_now - b) * fpb`, then `latency`.
    pub origin: f64,
    pub clock_bpm: f64,
}

/// The reference for estimating input `i`'s latency over its last `seconds`: the master clock's
/// beats laid on the recording. Errs with no clock (the source can only be said to be late
/// against something).
pub fn latency_ref(e: &Engine, i: usize, seconds: f64) -> Result<LatencyRef, String> {
    // the recording's end and the clock's beat are two counters read apart: take them again
    // until a read is steady, so both belong to the same block
    let (mut window, mut beat, mut fpb) = (None, 0.0, 0.0);
    for _ in 0..20 {
        let a = e.loop_frames();
        let w = tap_window(e, i, seconds).ok_or("no recording yet on that input (it needs 2 s)")?;
        let (b, f) = e.clock_beat_now().ok_or(
            "no master clock: start a deck or the transport, and slave the source to it (the TD-3 follows djapp's MIDI clock)",
        )?;
        if e.loop_frames() == a {
            (window, beat, fpb) = (Some(w), b, f);
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    let window = window.ok_or("the clock kept moving: try again")?;
    let sr = window.sample_rate;
    let span_beats = (window.end - window.start) as f64 / fpb;
    let b0 = (beat - span_beats).ceil() + 1.0;
    let f0 = window.end as f64 - (beat - b0) * fpb;
    Ok(LatencyRef { origin: (f0 - window.start as f64) / sr, period: fpb / sr, clock_bpm: 60.0 * sr / fpb, window })
}

/// Watches the strips' history heads and snaps one to the grid when a hand lets go of the
/// platter (scrub → delay): the jog scrubs freely under the hand and lands on a beat or bar
/// line when released. Call [`service`](SnapWatch::service) every tick.
#[derive(Default)]
pub struct SnapWatch {
    last: Vec<Mode>,
    /// Ticks to ignore a strip after a snap (our own touch / release shows as a release).
    cooldown: Vec<u32>,
}

/// Ticks a strip is left alone after a snap.
const SNAP_COOLDOWN: u32 = 30;

impl SnapWatch {
    pub fn new() -> SnapWatch {
        SnapWatch::default()
    }

    /// Snap every strip whose platter was just released. `unit` None: snapping is off.
    /// `grid(i)`: the strip's grid, if it has a trusted one. Returns the moves made.
    pub fn service(
        &mut self,
        e: &mut Engine,
        unit: Option<Unit>,
        grid: &dyn Fn(usize) -> Option<FrameGrid>,
    ) -> Vec<(usize, Result<Moved, String>)> {
        let n = e.inputs.len();
        self.last.resize(n, Mode::Live);
        self.cooldown.resize(n, 0);
        let mut out = Vec::new();
        for i in 0..n {
            let mode = e.inputs[i].mirror.history_mode();
            let prev = std::mem::replace(&mut self.last[i], mode);
            self.cooldown[i] = self.cooldown[i].saturating_sub(1);
            if self.cooldown[i] > 0 || !(prev == Mode::Touch && mode == Mode::Play) {
                continue;
            }
            let (Some(u), Some(g)) = (unit, grid(i)) else { continue };
            let r = snap_head(e, i, &g, u, 0);
            self.cooldown[i] = SNAP_COOLDOWN;
            out.push((i, r));
        }
        out
    }
}
