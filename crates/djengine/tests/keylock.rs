//! Key lock: a stretched copy is the new length and keeps the pitch (docs/KEY.md).

use djengine::keylock;

const SR: f64 = 44_100.0;

/// Frequency by counting rising zero crossings over the middle of the buffer.
fn freq(x: &[f32]) -> f64 {
    let (a, b) = (x.len() / 4, x.len() * 3 / 4);
    let n = (a..b).filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0).count();
    n as f64 * SR / (b - a) as f64
}

fn sine(f: f64, seconds: f64) -> Vec<f32> {
    (0..(seconds * SR) as usize).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / SR).sin() as f32 * 0.5).collect()
}

#[test]
fn a_locked_tempo_change_keeps_the_pitch() {
    let x = sine(440.0, 3.0);
    for rate in [0.8, 0.95, 1.06, 1.25] {
        let out = keylock::stretch(&[&x, &x], rate);
        assert_eq!(out.len(), 2);
        let want = (x.len() as f64 / rate).round() as usize;
        assert_eq!(out[0].len(), want, "rate {rate}: length is len / rate");
        let f = freq(&out[0]);
        assert!((f - 440.0).abs() < 4.0, "rate {rate}: {f} Hz, not 440");
    }
}

#[test]
fn varispeed_moves_pitch_by_the_log_of_the_rate() {
    assert!((keylock::varispeed_semitones(2.0) - 12.0).abs() < 1e-9);
    assert!((keylock::varispeed_semitones(1.05) - 0.8447).abs() < 1e-3);
    assert_eq!(keylock::varispeed_semitones(1.0), 0.0);
    assert!(!keylock::needs_render(1.0, 1.003), "0.05 semitones: not worth a render");
    assert!(keylock::needs_render(1.0, 1.02));
    assert_eq!(keylock::ratio(120.0, 128.0), Some(128.0 / 120.0));
    assert_eq!(keylock::ratio(120.0, 300.0), None, "past 2x");
    let g = djengine::deck::beat_grid::BeatGrid::new(120.0, 1000.0, 0.9);
    let h = keylock::grid_at(&g, 1.25);
    assert_eq!((h.bpm, h.offset), (150.0, 800.0));
    assert!(keylock::inherent(true) && !keylock::inherent(false));
}

#[test]
fn transposing_a_part_moves_its_pitch_and_keeps_its_length() {
    let x = sine(440.0, 2.0);
    let up = keylock::transpose(&[&x], 2, false);
    assert_eq!(up[0].len(), x.len());
    let f = freq(&up[0]);
    let want = 440.0 * 2f64.powf(2.0 / 12.0);
    assert!((f - want).abs() < 6.0, "{f} Hz, want {want}");
}
