//! Beat and downbeat analysis for audio djrust did not generate (docs/BEAT-DETECTION.md).
//!
//! The pure parts (the grid, the fitter, the DSP detector, trimming, the latency estimate) are
//! `djengine::beatgrid`. This module is what needs a thread, a file or a model:
//!
//! - [`Backend`]: something that finds beats and downbeats in audio. The real one is Beat This!
//!   ([`neural`], the `neural` feature); tests plug in their own.
//! - [`analyze`]: the DSP grid first (always, in a fraction of a second), then the backend's beats
//!   through the steady-first fitter. A backend that is missing, fails, or finds less than the
//!   DSP did leaves the DSP grid, with a note saying why.
//! - [`worker::Worker`]: all of it on a thread of its own, so analysis never blocks the window or
//!   the audio. Results come back as events; a grid is cached by the content hash of its audio.
//! - [`models`]: where the model files live, and fetching them with a checksum.

use std::time::Instant;

use djengine::beatgrid::{
    dsp_analyze, envelope_peaks, fit_grid, onset_envelope, refine_to_onsets, BeatGrid, DspOpts, FitOpts,
    GridSource,
};
use djengine::json::{JsonObject, JsonValue};

pub mod cache;
pub mod live;
pub mod models;
#[cfg(feature = "neural")]
pub mod neural;
pub mod worker;

/// What a backend found: beat and downbeat times, seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawBeats {
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
}

/// Something that finds beats in mono audio. It may be slow (seconds) and is only ever called
/// from the analysis worker.
pub trait Backend: Send {
    fn name(&self) -> &'static str;
    fn detect(&mut self, mono: &[f32], sample_rate: f64) -> Result<RawBeats, String>;
}

/// What [`analyze`] is asked to do.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub fit: FitOpts,
    pub dsp: DspOpts,
    /// A backend's grid below this confidence loses to the DSP grid.
    pub min_neural_confidence: f64,
    /// The audio's own tempo, when it has one (a library loop's, a vocal line's): its grid is
    /// that tempo from the first sample, and nothing is detected (no models are loaded for it).
    pub known_bpm: Option<f64>,
}

impl Default for Options {
    fn default() -> Options {
        Options { fit: FitOpts::default(), dsp: DspOpts::default(), min_neural_confidence: 0.30, known_bpm: None }
    }
}

/// Onsets (or beats snapped to onsets) with their strengths, for the latency estimate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detections {
    pub times: Vec<f64>,
    pub weights: Vec<f64>,
    /// 1: these are beats. 4: these are onsets on the sixteenth grid (the DSP fallback), so a
    /// reference period of `beat / 4` is the one to fold them on.
    pub fold: u32,
}

/// The answer of [`analyze`].
#[derive(Clone, Debug)]
pub struct Analysis {
    /// The best grid: the backend's when it made one, else the DSP's. None for audio with no
    /// beat to find (silence, a few seconds).
    pub grid: Option<BeatGrid>,
    /// The DSP grid on its own (what is there while the backend is still running).
    pub dsp: Option<BeatGrid>,
    pub detections: Detections,
    /// `neural`, `dsp` or `none`.
    pub method: &'static str,
    /// Why the backend's answer was not used (or what it said), empty when it was.
    pub note: String,
    /// Seconds the analysis took.
    pub seconds: f64,
    /// Seconds of audio analysed.
    pub duration: f64,
}

/// Mono from interleaved or separate channels: the mean.
pub fn mixdown(left: &[f32], right: Option<&[f32]>) -> Vec<f32> {
    match right {
        Some(r) => left.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect(),
        None => left.to_vec(),
    }
}

/// A key for a piece of audio: FNV-1a over its samples' bits, its length and its rate. The same
/// audio is the same key whatever it is called or where it sits.
pub fn content_key(mono: &[f32], sample_rate: f64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    };
    for b in (sample_rate as u32).to_le_bytes() {
        eat(b);
    }
    for b in (mono.len() as u64).to_le_bytes() {
        eat(b);
    }
    for v in mono {
        for b in v.to_bits().to_le_bytes() {
            eat(b);
        }
    }
    format!("{h:016x}")
}

/// The onsets of `mono` as detections (seconds, strength), for the latency estimate and for
/// snapping a backend's beats to the sound.
pub fn dsp_onsets(mono: &[f32], sample_rate: f64) -> Vec<(f64, f64)> {
    let (env, _, fps) = onset_envelope(mono, sample_rate);
    envelope_peaks(&env, fps, 0.12)
}

/// Snap each beat to the strongest onset within `window` seconds; a beat with none near keeps
/// its time (and a weight of a tenth).
fn snap_to_onsets(beats: &[f64], onsets: &[(f64, f64)], window: f64) -> (Vec<f64>, Vec<f64>) {
    let max = onsets.iter().map(|o| o.1).fold(0.0, f64::max).max(1e-9);
    let mut times = Vec::with_capacity(beats.len());
    let mut weights = Vec::with_capacity(beats.len());
    for b in beats {
        let near = onsets
            .iter()
            .filter(|(t, _)| (t - b).abs() <= window)
            .fold(None::<(f64, f64)>, |best, o| match best {
                Some(x) if x.1 >= o.1 => Some(x),
                _ => Some(*o),
            });
        match near {
            Some((t, w)) => {
                times.push(t);
                weights.push(w / max);
            }
            None => {
                times.push(*b);
                weights.push(0.1);
            }
        }
    }
    (times, weights)
}

/// A grid from a tempo the audio carries: `bpm` from its first sample, four to a bar.
pub fn known_grid(bpm: f64, duration: f64) -> Option<BeatGrid> {
    if !(20.0..=400.0).contains(&bpm) || duration <= 0.0 {
        return None;
    }
    let period = 60.0 / bpm;
    let n = libm_ceil(duration / period - 1e-3).max(2.0) as usize;
    Some(BeatGrid {
        source: GridSource::Known,
        period,
        beats_per_bar: 4,
        steady: true,
        confidence: 1.0,
        residual_ms: 0.0,
        beats: (0..n).map(|k| k as f64 * period).collect(),
        first_downbeat: 0,
    })
}

fn libm_ceil(x: f64) -> f64 {
    x.ceil()
}

/// Whether the audio has too little in it to find a beat: silent, a blink long, or all silence
/// but a sound or two. `Some(why)` for the note; the DSP detector and the models are not run on
/// it. (A drum loop's short hits and a held vocal are not "sparse": those go to the detectors, and
/// a result of none is marked in the note, see [`analyze`].)
pub fn sparse_reason(mono: &[f32], sample_rate: f64) -> Option<String> {
    let win = (sample_rate * 0.05) as usize;
    if win == 0 || mono.len() < win * 4 {
        return Some("too short to find a beat".into());
    }
    let rms: Vec<f64> = mono.chunks(win).filter(|c| c.len() == win).map(|c| (c.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / win as f64).sqrt()).collect();
    let peak = rms.iter().cloned().fold(0.0, f64::max);
    if peak < 1e-4 {
        return Some("silent".into());
    }
    // 35 dB under the loudest 50 ms
    let active = rms.iter().filter(|r| **r > peak * 0.0178).count() as f64 / rms.len() as f64;
    if active < 0.08 {
        return Some(format!("nearly silent ({:.0}% of it sounds): no beat grid", active * 100.0));
    }
    None
}

/// What needs no detection: a known tempo, or audio too sparse to find a beat in. None: analyse.
pub fn pre_analysis(mono: &[f32], sample_rate: f64, opts: &Options) -> Option<Analysis> {
    let duration = mono.len() as f64 / sample_rate;
    let quick = |grid: Option<BeatGrid>, method: &'static str, note: String| Analysis {
        grid,
        dsp: None,
        detections: Detections::default(),
        method,
        note,
        seconds: 0.0,
        duration,
    };
    if let Some(bpm) = opts.known_bpm {
        if let Some(g) = known_grid(bpm, duration) {
            return Some(quick(Some(g), "known", format!("the clip's own tempo, {bpm:.1} bpm")));
        }
    }
    sparse_reason(mono, sample_rate).map(|why| quick(None, "none", why))
}

/// Analyse mono audio. `backend` None (or failing) leaves the DSP grid; `why_none` says why
/// there is no backend (models missing, the feature off) for the note.
pub fn analyze(
    mono: &[f32],
    sample_rate: f64,
    backend: Option<&mut dyn Backend>,
    why_none: &str,
    opts: Options,
) -> Analysis {
    let t0 = Instant::now();
    if let Some(a) = pre_analysis(mono, sample_rate, &opts) {
        return a;
    }
    let duration = mono.len() as f64 / sample_rate;
    let dsp = dsp_analyze(mono, sample_rate, opts.dsp);
    let onsets = dsp_onsets(mono, sample_rate);
    let dsp_det = Detections {
        times: onsets.iter().map(|o| o.0).collect(),
        weights: onsets.iter().map(|o| o.1).collect(),
        fold: 4,
    };
    let mut note = String::new();
    let mut best = None;
    let mut detections = dsp_det.clone();
    let mut method = if dsp.is_some() { "dsp" } else { "none" };
    match backend {
        None => note = why_none.to_string(),
        Some(b) => match b.detect(mono, sample_rate) {
            Err(e) => note = format!("{}: {e}", b.name()),
            Ok(raw) => {
                let fit = FitOpts { hint_bpm: dsp.as_ref().map(|g| g.bpm()).or(opts.fit.hint_bpm), ..opts.fit };
                match fit_grid(&raw.beats, &raw.downbeats, duration, fit) {
                    None => note = format!("{} found no usable beats", b.name()),
                    Some(g) if g.confidence < opts.min_neural_confidence => {
                        note = format!(
                            "{} grid unsure ({:.0}%), kept the DSP grid",
                            b.name(),
                            g.confidence * 100.0
                        )
                    }
                    Some(mut g) => {
                        // the model's beats sit on its 20 ms frames: put a steady grid on the attacks
                        if let Some(shift) = refine_to_onsets(&mut g, &onsets, 0.045) {
                            note = format!("grid moved {:+.0} ms onto the attacks", shift * 1000.0);
                        }
                        let (times, weights) = snap_to_onsets(&raw.beats, &onsets, 0.05);
                        detections = Detections { times, weights, fold: 1 };
                        method = "neural";
                        best = Some(g);
                    }
                }
            }
        },
    }
    // nothing found by either: say so once, quietly, rather than the detectors' own complaints
    if best.is_none() && dsp.is_none() && method == "none" {
        note = "no steady beat found (a vocal, a pad or a one-shot?): no grid".to_string();
    }
    Analysis {
        grid: best.or_else(|| dsp.clone()),
        dsp,
        detections,
        method,
        note,
        seconds: t0.elapsed().as_secs_f64(),
        duration,
    }
}

// MARK: JSON

fn num(x: f64) -> JsonValue {
    JsonValue::Number((x * 1e6).round() / 1e6)
}

/// A grid as JSON (the cache's and the control language's): a steady grid is its first beat, its
/// period and its count; one that drifts lists its beats.
pub fn grid_to_json(g: &BeatGrid, with_beats: bool) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("source", JsonValue::str(g.source.name()));
    o.set("bpm", num(g.bpm()));
    o.set("period", num(g.period));
    o.set("beats_per_bar", JsonValue::int(g.beats_per_bar as i64));
    o.set("steady", JsonValue::Bool(g.steady));
    o.set("confidence", num(g.confidence));
    o.set("residual_ms", num(g.residual_ms));
    o.set("first_downbeat_time", num(g.first_downbeat_time()));
    o.set("first_downbeat", JsonValue::int(g.first_downbeat as i64));
    o.set("beat_count", JsonValue::int(g.beats.len() as i64));
    if let Some(b0) = g.beats.first() {
        o.set("first_beat", num(*b0));
    }
    if with_beats && !g.steady {
        o.set("beats", JsonValue::Array(g.beats.iter().map(|t| num(*t)).collect()));
    }
    JsonValue::Object(o)
}

/// [`grid_to_json`] read back (None for anything malformed).
pub fn grid_from_json(v: &JsonValue) -> Option<BeatGrid> {
    let f = |k: &str| v.get(k).and_then(JsonValue::as_f64);
    let period = f("period").filter(|p| *p > 0.0)?;
    let count = f("beat_count")? as usize;
    let steady = v.get("steady").and_then(JsonValue::as_bool)?;
    let beats: Vec<f64> = if steady {
        let b0 = f("first_beat")?;
        (0..count).map(|k| b0 + k as f64 * period).collect()
    } else {
        v.get("beats")?.as_array()?.iter().filter_map(JsonValue::as_f64).collect()
    };
    if beats.len() < 2 || beats.len() != count {
        return None;
    }
    Some(BeatGrid {
        source: GridSource::parse(v.get("source")?.as_str()?)?,
        period,
        beats_per_bar: (f("beats_per_bar")? as u32).clamp(1, 12),
        steady,
        confidence: f("confidence")?,
        residual_ms: f("residual_ms").unwrap_or(0.0),
        first_downbeat: (f("first_downbeat")? as usize).min(beats.len() - 1),
        beats,
    })
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// A click on every beat, a low loud one on each bar's first, from `first` seconds.
    pub fn click_track(bpm: f64, first: f64, seconds: f64, sr: f64) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        let mut v = vec![0.0f32; n];
        let period = 60.0 / bpm;
        let mut k = 0usize;
        loop {
            let t = first + k as f64 * period;
            if t >= seconds - 0.05 {
                break;
            }
            let (f, a) = if k % 4 == 0 { (110.0, 1.0) } else { (880.0, 0.5) };
            let start = (t * sr) as usize;
            for i in 0..(0.04 * sr) as usize {
                if start + i < n {
                    let x = i as f64 / sr;
                    v[start + i] += (a * (-x * 90.0).exp() * (2.0 * std::f64::consts::PI * f * x).sin()) as f32;
                }
            }
            k += 1;
        }
        v
    }

    /// A four-on-the-floor loop that sounds like something: kick on every beat, a snare burst on
    /// 2 and 4, hats on the offbeat eighths, a bass note per bar (root, then the fifth on the
    /// 3rd bar) so bar 1 is audibly a bar line. `first` seconds of silence in.
    #[cfg_attr(not(feature = "neural"), allow(dead_code))]
    pub fn drum_loop(bpm: f64, first: f64, seconds: f64, sr: f64) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        let mut v = vec![0.0f32; n];
        let beat = 60.0 / bpm;
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 8_388_608.0 - 1.0
        };
        let tau = 2.0 * std::f64::consts::PI;
        let mut k = 0usize;
        loop {
            let t = first + k as f64 * beat;
            if t >= seconds - 0.1 {
                break;
            }
            let s0 = (t * sr) as usize;
            // kick: sine sweep 160 -> 45 Hz
            for i in 0..(0.18 * sr) as usize {
                if s0 + i < n {
                    let x = i as f64 / sr;
                    let ph = tau * (45.0 * x + 115.0 * (1.0 - (-x * 30.0).exp()) / 30.0);
                    v[s0 + i] += (0.9 * (-x * 18.0).exp() * ph.sin()) as f32;
                }
            }
            // snare on the 2 and the 4
            if k % 2 == 1 {
                for i in 0..(0.12 * sr) as usize {
                    if s0 + i < n {
                        let x = i as f64 / sr;
                        v[s0 + i] += (0.5 * (-x * 30.0).exp()) as f32 * noise();
                    }
                }
            }
            // hat on the offbeat
            let h0 = ((t + beat / 2.0) * sr) as usize;
            for i in 0..(0.04 * sr) as usize {
                if h0 + i < n {
                    let x = i as f64 / sr;
                    v[h0 + i] += (0.25 * (-x * 120.0).exp()) as f32 * noise();
                }
            }
            // bass: once a bar, the root; every 4th bar the fifth
            if k % 4 == 0 {
                let f = if (k / 16) % 2 == 1 { 73.4 } else { 55.0 };
                for i in 0..(beat * 3.5 * sr) as usize {
                    if s0 + i < n {
                        let x = i as f64 / sr;
                        v[s0 + i] += (0.35 * (-x * 1.2).exp() * (tau * f * x).sin()) as f32;
                    }
                }
            }
            k += 1;
        }
        v
    }

    /// A backend that knows where the clicks are (the oracle a real model approximates), with
    /// `jitter` seconds of error alternating in sign.
    pub struct Oracle {
        pub bpm: f64,
        pub first: f64,
        pub jitter: f64,
        pub calls: usize,
    }

    impl Backend for Oracle {
        fn name(&self) -> &'static str {
            "oracle"
        }
        fn detect(&mut self, mono: &[f32], sr: f64) -> Result<RawBeats, String> {
            self.calls += 1;
            let dur = mono.len() as f64 / sr;
            let p = 60.0 / self.bpm;
            let beats: Vec<f64> = (0..)
                .map(|k| self.first + k as f64 * p)
                .take_while(|t| *t < dur - 0.05)
                .enumerate()
                .map(|(k, t)| t + if k % 2 == 0 { self.jitter } else { -self.jitter })
                .collect();
            let downbeats = beats.iter().step_by(4).copied().collect();
            Ok(RawBeats { beats, downbeats })
        }
    }

    pub struct Broken;
    impl Backend for Broken {
        fn name(&self) -> &'static str {
            "broken"
        }
        fn detect(&mut self, _: &[f32], _: f64) -> Result<RawBeats, String> {
            Err("model blew up".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    const SR: f64 = 44_100.0;

    #[test]
    fn the_backends_beats_make_the_grid_and_bar_one_is_a_downbeat() {
        let audio = click_track(124.0, 0.30, 20.0, SR);
        let mut o = Oracle { bpm: 124.0, first: 0.30, jitter: 0.006, calls: 0 };
        let a = analyze(&audio, SR, Some(&mut o), "", Options::default());
        assert_eq!(a.method, "neural");
        let g = a.grid.unwrap();
        assert_eq!(g.source, GridSource::Neural);
        assert!((g.bpm() - 124.0).abs() < 0.05, "{}", g.bpm());
        assert!((g.first_downbeat_time() - 0.30).abs() < 0.003, "{}", g.first_downbeat_time());
        assert!(g.steady && g.confidence > 0.8);
        // the DSP grid is kept too, and agrees
        assert!((a.dsp.unwrap().bpm() - 124.0).abs() < 0.5);
        // snapped to the clicks, the detections are exact
        assert_eq!(a.detections.fold, 1);
    }

    #[test]
    fn without_models_the_dsp_grid_stands_and_says_why() {
        let audio = click_track(110.0, 0.2, 20.0, SR);
        let a = analyze(&audio, SR, None, "models not downloaded", Options::default());
        assert_eq!(a.method, "dsp");
        assert_eq!(a.note, "models not downloaded");
        let g = a.grid.unwrap();
        assert_eq!(g.source, GridSource::Dsp);
        assert!((g.bpm() - 110.0).abs() < 0.5, "{}", g.bpm());
        assert_eq!(a.detections.fold, 4);
        assert!(!a.detections.times.is_empty());
    }

    #[test]
    fn a_failing_backend_falls_back_to_dsp() {
        let audio = click_track(100.0, 0.2, 20.0, SR);
        let a = analyze(&audio, SR, Some(&mut Broken), "", Options::default());
        assert_eq!(a.method, "dsp");
        assert!(a.note.contains("model blew up"), "{}", a.note);
        assert!(a.grid.is_some());
    }

    #[test]
    fn silence_has_no_grid() {
        let a = analyze(&vec![0.0; 44_100 * 8], SR, None, "no models", Options::default());
        assert_eq!(a.method, "none");
        assert!(a.grid.is_none());
    }

    #[test]
    fn a_clip_that_knows_its_tempo_gets_that_grid_and_nothing_is_detected() {
        // sparse audio (a few syllables of a vocal) that no detector could grid
        let mut v = vec![0.0f32; (SR * 3.871) as usize];
        for k in 0..8 {
            let at = (k as f64 * 0.484 * SR) as usize;
            for i in 0..8000 {
                v[at + i] = (i as f32 * 0.07).sin() * 0.4;
            }
        }
        let o = Options { known_bpm: Some(124.0), ..Options::default() };
        let a = analyze(&v, SR, Some(&mut Broken), "", o);
        assert_eq!(a.method, "known");
        let g = a.grid.unwrap();
        assert_eq!(g.source, GridSource::Known);
        assert!((g.bpm() - 124.0).abs() < 1e-9);
        assert_eq!(g.beats.len(), 8);
        assert_eq!((g.first_downbeat, g.beats[0]), (0, 0.0));
        assert!(a.note.contains("124"), "{}", a.note);
        // the backend (which would have failed noisily) was never asked
        assert!(!a.note.contains("blew up"));
        // and a grid round-trips through JSON with its source
        assert_eq!(grid_from_json(&grid_to_json(&g, true)).unwrap().source, GridSource::Known);
        // without a tempo the same clip says once, quietly, that it has no steady beat
        let b = analyze(&v, SR, Some(&mut Broken), "", Options::default());
        assert!(b.grid.is_none() || b.method != "known");
        if b.grid.is_none() {
            assert!(!b.note.contains("blew up") || b.method == "none");
        }
    }

    #[test]
    fn content_key_follows_the_audio_not_the_name() {
        let a = click_track(120.0, 0.0, 3.0, SR);
        let mut b = a.clone();
        assert_eq!(content_key(&a, SR), content_key(&b, SR));
        b[1000] += 0.001;
        assert_ne!(content_key(&a, SR), content_key(&b, SR));
        assert_ne!(content_key(&a, SR), content_key(&a, 48_000.0));
    }

    #[test]
    fn grids_round_trip_through_json() {
        let audio = click_track(124.0, 0.30, 12.0, SR);
        let a = analyze(&audio, SR, Some(&mut Oracle { bpm: 124.0, first: 0.30, jitter: 0.0, calls: 0 }), "", Options::default());
        let g = a.grid.unwrap();
        let back = grid_from_json(&JsonValue::parse(&grid_to_json(&g, true).canonical_js()).unwrap()).unwrap();
        assert_eq!(back.first_downbeat, g.first_downbeat);
        assert!((back.period - g.period).abs() < 1e-6);
        assert!((back.time(5) - g.time(5)).abs() < 1e-4);
        assert!(grid_from_json(&JsonValue::Null).is_none());
    }
}
