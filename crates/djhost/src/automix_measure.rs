//! Loudness through auto-mix hand-offs, measured (`AutoMixMeasure.swift`, dj-vevr-se's docs/AUTOMIX.md
//! section 8): an offline set on the same engine path as a replay (the offline conductor's blocks and
//! housekeeping, deterministic composes), then per hand-off the outgoing deck alone before it,
//! the incoming alone after it, and the mix's short-term loudness (BS.1770, 3 s) per beat
//! across it. `djctl automix` prints it.
//!
//! The pure maths (the dip, the bump, the rounding of the report, the curve's filtering) is
//! `djengine::automix::measure`; this is the driver. It renders through [`Renderer`] (the strip
//! EQ and DJ filter as `fx::strip`'s biquads) where Swift renders through AVAudioEngine's
//! offline graph (Apple's EQ unit), so the levels agree to what those two filters differ by, not
//! to the bit; the hand-off's frames (when the incoming deck's first tick lands, when the run
//! clears) are the engine's and agree with Swift's exactly (`automixdeck_golden.rs`).

use djengine::automix::measure::AutoMixHandoffLevels;
use djengine::deck::level_match::LoudnessMeter;
use djengine::json::{JsonObject, JsonValue};
use djengine::musicality::loudness::IntegratedLoudness;
use djengine::TrackSpec;

use crate::deck::{engine, EngineConfig};

/// `OfflineConductor.blockFrames`.
const BLOCK: usize = 128;

/// An offline auto-mix set's measurements (`AutoMixMeasure`).
#[derive(Clone, Debug)]
pub struct AutoMixMeasure {
    pub genre: String,
    pub seed: f64,
    pub bars: i64,
    pub every: i64,
    pub crossfade_only: bool,
    pub handoffs: Vec<AutoMixHandoffLevels>,
    pub sample_peak_db: f64,
    pub true_peak_dbtp: f64,
    /// The whole set's audio, when asked for (`keepAudio`).
    pub audio: Option<(Vec<f32>, Vec<f32>)>,
    pub sample_rate: f64,
}

/// `(x * 100).rounded() / 100`.
fn r(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl AutoMixMeasure {
    /// `AutoMixMeasure.dictionary`.
    pub fn json(&self) -> JsonValue {
        JsonObject::from_pairs([
            ("genre", JsonValue::str(self.genre.as_str())),
            ("seed", JsonValue::Number(self.seed)),
            ("bars", JsonValue::int(self.bars)),
            ("every", JsonValue::int(self.every)),
            ("crossfadeOnly", JsonValue::Bool(self.crossfade_only)),
            ("samplePeakDB", JsonValue::Number(r(self.sample_peak_db))),
            ("truePeakDBTP", JsonValue::Number(r(self.true_peak_dbtp))),
            (
                "handoffs",
                JsonValue::Array(self.handoffs.iter().map(|h| h.json()).collect()),
            ),
        ])
        .into()
    }

    /// `AutoMixMeasure.run`: render a set: `genre` / `seed` (a song in `structure`) on deck A,
    /// the crossfader on A, auto-mix on with `bars` / `every`, until `count` hand-offs have
    /// finished and 8 more bars have played. `crossfade_only`: no mix-in / mix-out rests (the
    /// gain law alone).
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        genre: &str,
        seed: f64,
        structure: Option<&str>,
        bars: i64,
        every: i64,
        count: usize,
        crossfade_only: bool,
        sample_rate: f64,
        keep_audio: bool,
    ) -> Result<AutoMixMeasure, String> {
        let mut m = AutoMixMeasure {
            genre: genre.into(),
            seed,
            bars,
            every,
            crossfade_only,
            handoffs: Vec::new(),
            sample_peak_db: -200.0,
            true_peak_dbtp: -200.0,
            audio: None,
            sample_rate,
        };
        let (mut e, mut rd) = engine(EngineConfig::new(sample_rate));
        e.deterministic = true;
        e.auto_mix_host.crossfade_only = crossfade_only;
        let spec = TrackSpec::new(genre, seed, structure);
        if !e.generate("A", &spec) {
            return Err(format!("{genre}:{seed} did not compose"));
        }
        e.load_demo("B");
        e.set_crossfader(0.0);
        e.set_auto_mix(true, bars, every);
        e.play("A");
        let every_frames = ((sample_rate / 100.0) as i64).max(1);
        let mut next_housekeeping = 0i64;
        let mut frame = 0i64;
        let (mut left, mut right): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        let (mut bl, mut br) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
        // per hand-off: the frame its incoming deck started (its first tick) and ended
        let (mut starts, mut ends): (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
        let (mut was_running, mut start_seen) = (false, false);
        let mut bpm = e.effective_bpm("A").unwrap_or(120.0);
        let mut tail = -1i64;
        let limit = (sample_rate * 60.0 * 30.0) as usize; // half an hour, at most
        while left.len() < limit {
            // the offline conductor's step: the housekeeping if due, then the block
            if frame >= next_housekeeping {
                next_housekeeping = frame + every_frames;
                e.service();
            }
            rd.render_block(&mut bl, &mut br);
            frame += BLOCK as i64;
            left.extend_from_slice(&bl);
            right.extend_from_slice(&br);
            if let Some(run) = e.auto_mix.run.clone() {
                was_running = true;
                let inc = e
                    .deck(&run.inc)
                    .map(|d| (d.mirror.tick(), d.mirror.tick_phase()));
                if let (false, Some((tick, phase))) = (start_seen, inc) {
                    if tick > 0 {
                        // the first tick landed in this block: back to it by the ticks' own clock
                        let spt =
                            sample_rate * 60.0 / (e.effective_bpm(&run.inc).unwrap_or(120.0) * 4.0);
                        let into = ((tick - 1) as f64 + phase) * spt;
                        starts.push(left.len() as i64 - into.round() as i64);
                        bpm = e.effective_bpm(&run.out).unwrap_or(bpm);
                        start_seen = true;
                    }
                }
            } else if was_running {
                was_running = false;
                start_seen = false;
                ends.push(left.len() as i64);
                if ends.len() >= count {
                    tail = left.len() as i64 + (sample_rate * 60.0 / bpm * 4.0 * 9.0) as i64;
                }
            }
            if tail >= 0 && left.len() as i64 >= tail {
                break;
            }
        }
        let bar_frames = sample_rate * 60.0 / bpm * 4.0;
        let window = (bar_frames * 8.0) as i64;
        // short-term loudness per beat across the whole set
        let mut meter = LoudnessMeter::new(sample_rate);
        let beat = bar_frames / 4.0;
        let mut st: Vec<(i64, f64)> = Vec::new();
        let mut next_beat = beat;
        let mut i = 0usize;
        while i < left.len() {
            let n = 256.min(left.len() - i);
            meter.push(&left[i..i + n], Some(&right[i..i + n]), n);
            i += n;
            while i as f64 >= next_beat {
                st.push((i as i64, meter.short_term_lufs()));
                next_beat += beat;
            }
        }
        let lufs = |from: i64, to: i64| -> f64 {
            let a = from.max(0) as usize;
            let b = (to.max(0) as usize).min(left.len());
            if b <= a {
                return -200.0;
            }
            IntegratedLoudness::integrated(&left[a..b], Some(&right[a..b]), sample_rate)
                .unwrap_or(-200.0)
        };
        for (k, (s, en)) in starts.iter().zip(&ends).enumerate() {
            let row = e.auto_mix.log.get(k);
            m.handoffs.push(AutoMixHandoffLevels::from_curve(
                row.map_or(k as i64, |h| h.n),
                row.map_or("?", |h| h.from.as_str()),
                row.map_or("?", |h| h.to.as_str()),
                row.map_or("", |h| h.move_.as_str()),
                lufs(s - window, *s),
                lufs(*en, en + window),
                &st,
                *s,
                *en,
                window,
            ));
        }
        let peak = left.iter().chain(&right).fold(0f32, |p, v| p.max(v.abs()));
        m.sample_peak_db = if peak > 0.0 {
            20.0 * (peak as f64).log10()
        } else {
            -200.0
        };
        m.true_peak_dbtp = djengine::fx::true_peak::dbtp(&left, Some(&right));
        if keep_audio {
            m.audio = Some((left, right));
        }
        Ok(m)
    }
}
