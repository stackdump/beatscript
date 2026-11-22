//! The offline render hears the control lines (docs/design/NETS-ALL-THE-WAY-DOWN.md, phase 1): a filter
//! sweep written as a ramp is a chain in the program's control net, the net's events are applied to
//! the engine on their frames, and the spectrum of the render moves with it. Before this the same
//! program rendered with the filter wide open from the first bar to the last.

use djhost::btw_render;

const SR: f64 = 22_050.0;

fn compile(src: &str) -> djlive::compile::Compiled {
    let c = djlive::check(src);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics);
    c.compiled.unwrap()
}

/// Spectral centroid in Hz of `x[at..at+n]` (Hann window, a plain DFT: a test, not a hot path).
fn centroid(l: &[f32], r: &[f32], at: usize, n: usize) -> f64 {
    let x: Vec<f64> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            f64::from(l[at + i] + r[at + i]) * 0.5 * w
        })
        .collect();
    let (mut num, mut den) = (0.0, 0.0);
    for k in 1..n / 2 {
        let (mut re, mut im) = (0.0, 0.0);
        let w = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
        for (i, v) in x.iter().enumerate() {
            re += v * (w * i as f64).cos();
            im -= v * (w * i as f64).sin();
        }
        let mag = (re * re + im * im).sqrt();
        num += mag * (k as f64 * SR / n as f64);
        den += mag;
    }
    num / den.max(1e-12)
}

/// The centroid at `bar` (a window two beats into it, past the net swap and the filter's smoothing).
fn at_bar(a: &djengine::session::wav::RenderedAudio, bar: f64, bpm: f64) -> f64 {
    let t = (bar * 4.0 + 1.0) * 60.0 / bpm;
    centroid(&a.left, &a.right, (t * SR) as usize, 2048)
}

/// Noise, a sub bass and a kick: broadband, so a low-pass shows in the centroid.
const SONG: &str = "tempo 120\ndeck A: hihat x*16 | bass c2 c2 c2 c2 | kick x ~ x ~\n";

#[test]
fn a_filter_sweep_written_in_btw_is_audible_in_the_offline_render() {
    let swept = compile(&format!("{SONG}master: filter lp 250 -> 12k over 8 bars\n"));
    let a = btw_render::render(&swept, SR, true).expect("render");
    assert!(a.refused.is_empty(), "the engine refused: {:?}", a.refused);
    assert!(a.beside.is_empty(), "{:?}", a.beside);
    assert!(a.applied > 65, "the net's events were applied: {}", a.applied);
    // the same program with the net left out: the filter is never touched
    let flat = btw_render::render(&swept, SR, false).expect("render");
    assert_eq!(flat.applied, 0);

    let bars = [0.0, 2.0, 4.0, 6.0, 7.5];
    let with: Vec<f64> = bars.iter().map(|b| at_bar(&a.audio, *b, 120.0)).collect();
    let without: Vec<f64> = bars.iter().map(|b| at_bar(&flat.audio, *b, 120.0)).collect();
    eprintln!("centroid Hz, bars {bars:?}\n  with the net    {with:.0?}\n  without the net {without:.0?}");

    // the sweep opens: the centroid rises across the ramp, bar by bar, and by far
    for w in with.windows(2) {
        assert!(w[1] > w[0] * 1.05, "the centroid should rise across the ramp: {with:?}");
    }
    assert!(with[4] > with[0] * 3.0, "{with:?}");
    // at the start the low-pass at 250 Hz takes the hats away: far below the open render
    assert!(with[0] < without[0] * 0.5, "{with:?} against {without:?}");
    // and by the end it is open: the same colour as the render that never had a filter
    assert!(with[4] > without[4] * 0.6, "{with:?} against {without:?}");
    // the render with no net does not move: the control (every window within a quarter of the first)
    for v in &without {
        assert!((v / without[0] - 1.0).abs() < 0.25, "{without:?}");
    }
}

#[test]
fn a_move_cuts_the_strip_and_the_phrase_line_puts_it_back_in_the_render() {
    // bar 2: a filter exit over 2 bars (the strip is cut on bar 4), the put-back on the phrase line, bar 8
    let src = format!("{SONG}at bar 2: move filter_exit on A over 2 bars\n");
    let c = compile(&src);
    let r = btw_render::render(&c, SR, true).expect("render");
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    let rms = |bar: f64| {
        let (a, n) = ((bar * 4.0 * 0.5 * SR) as usize, (0.5 * SR) as usize);
        let s: f64 = (a..a + n).map(|i| f64::from(r.audio.left[i]).powi(2) + f64::from(r.audio.right[i]).powi(2)).sum();
        (s / (2 * n) as f64).sqrt()
    };
    let before = rms(1.0);
    let muted = rms(5.0);
    let back = rms(9.0);
    eprintln!("rms bar 1 {before:.4}  bar 5 (muted) {muted:.4}  bar 9 (put back) {back:.4}");
    assert!(before > 0.01, "something plays");
    assert!(muted < before * 0.05, "the strip is cut between the move's end and the phrase line");
    assert!(back > before * 0.5, "and it is back after it, by the net, without anything else saying so");
}
