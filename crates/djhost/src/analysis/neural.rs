//! Beat This! (ISMIR 2024, JKU Linz; MIT) through the pure-Rust `beat-this` crate on the `rten`
//! runtime: the [`Backend`](super::Backend) behind the `neural` feature. The small model (about
//! 2 M parameters) with its log-mel front end; a 4-minute track takes a few seconds on a laptop
//! core. docs/BEAT-DETECTION.md.

use std::path::Path;

use beat_this::{BeatThis, RtenRuntime, Runtime};

use super::models::{self, ModelFile};
use super::{Backend, RawBeats};

type Model = <RtenRuntime as Runtime>::Model;

pub struct NeuralBackend {
    bt: BeatThis<Model>,
}

impl NeuralBackend {
    /// Load the models in `dir` (the files [`models::ensure`] fetched), checking each against its
    /// pinned hash first.
    pub fn load(dir: &Path) -> Result<NeuralBackend, String> {
        let path = |f: &ModelFile| -> Result<std::path::PathBuf, String> {
            let p = dir.join(f.name);
            let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            models::verify(f, &bytes)?;
            Ok(p)
        };
        let (mel, beat) = (path(&models::MEL)?, path(&models::SMALL)?);
        let bt = BeatThis::new(&RtenRuntime, &mel, &beat).map_err(|e| format!("loading Beat This!: {e:#}"))?;
        Ok(NeuralBackend { bt })
    }

    /// Fetch what is missing (the network), then [`load`](Self::load).
    pub fn fetch_and_load(dir: &Path) -> Result<NeuralBackend, String> {
        models::ensure(dir, &mut models::http_fetch)?;
        NeuralBackend::load(dir)
    }
}

impl Backend for NeuralBackend {
    fn name(&self) -> &'static str {
        "Beat This!"
    }

    /// `beatgrid@2:<SHA-256>` of the small model it loaded (and checked against that pin).
    fn analyzer(&self) -> String {
        models::analyzer()
    }

    fn detect(&mut self, mono: &[f32], sample_rate: f64) -> Result<RawBeats, String> {
        let a = self
            .bt
            .analyze_audio(mono, sample_rate.round() as u32)
            .map_err(|e| format!("{e:#}"))?;
        Ok(RawBeats {
            beats: a.beats.iter().map(|t| *t as f64).collect(),
            downbeats: a.downbeats.iter().map(|t| *t as f64).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::drum_loop;
    use super::super::{analyze, Options};
    use super::*;

    /// The folder holding the two model files, from `DJ_TEST_MODELS` or the default folder, when
    /// they are there. The tests that need them skip otherwise (CI has no network and no models).
    fn models_dir() -> Option<std::path::PathBuf> {
        let d = std::env::var_os("DJ_TEST_MODELS").map(std::path::PathBuf::from).unwrap_or_else(models::default_dir);
        models::status(&d).ready.then_some(d)
    }

    #[test]
    fn beat_this_finds_the_tempo_and_puts_bar_one_on_the_downbeat() {
        let Some(dir) = models_dir() else {
            eprintln!("skipped: no Beat This! models (set DJ_TEST_MODELS)");
            return;
        };
        let mut b = NeuralBackend::load(&dir).unwrap();
        let sr = 44_100.0;
        let mut worst_bpm: f64 = 0.0;
        let mut worst_off_ms: f64 = 0.0;
        for (bpm, first) in [(124.0, 0.30), (98.0, 0.52), (140.0, 0.18)] {
            let audio = drum_loop(bpm, first, 40.0, sr);
            let a = analyze(&audio, sr, Some(&mut b), "", Options::default());
            assert_eq!(a.method, "neural", "{bpm}: {}", a.note);
            let g = a.grid.unwrap();
            let err = (g.bpm() - bpm).abs();
            // bar 1 is a kick: on a beat of the click grid, and on the bar's first beat
            let p = 60.0 / bpm;
            let n = (g.first_downbeat_time() - first) / p;
            let off = (n - n.round()) * p;
            eprintln!(
                "{bpm} bpm first {first}: found {:.3} bpm, bar 1 at {:.3} s (beat {} of the loop, off {:.1} ms), conf {:.2}, {:.1} s",
                g.bpm(), g.first_downbeat_time(), n.round(), off * 1000.0, g.confidence, a.seconds
            );
            worst_bpm = worst_bpm.max(err);
            worst_off_ms = worst_off_ms.max(off.abs() * 1000.0);
            assert!(err < bpm * 0.01, "{bpm}: {}", g.bpm());
            assert!(off.abs() < 0.006, "{bpm}: bar 1 is {off} s off the first kick");
            assert!(g.steady, "{bpm}: a sequenced loop must give a steady grid");
        }
        eprintln!("worst tempo error {worst_bpm:.3} bpm, worst beat offset {worst_off_ms:.1} ms");
    }
}

