//! `drive` (docs/BTW.md "Control"): the FX unit's drive, said as `deck A: drive 0.6` or `master: drive 0.6`, ramped like a reverb,
//! played by the offline render through the engine's own `Engine::set_fx` (the master chain's distortion). What it has to be:
//!
//! * **off is off**: `drive 0` (a line, a ramp of zeros) renders the very samples of the program without the line;
//! * **deterministic**: the same text renders the same samples, twice here and by a pinned hash, which is the browser's, the
//!   booth's and the phone's, because they run this engine code (a hash that moves is a change of sound, made on purpose);
//! * **grit, not loudness**: a driven kick has more of its energy in the harmonics than the dry one, and a driven low sine
//!   gains odd harmonics (the curve is odd) and no even ones to speak of;
//! * **no clipping**: drive 1 on a loud kick never takes the master past full scale, which is what `listen` counts.

use djengine::session::wav::RenderedAudio;
use djhost::btw_render::render;

const SR: f64 = 44_100.0;

fn rendered(src: &str) -> RenderedAudio {
    let c = djlive::check(src);
    let compiled = c.compiled.unwrap_or_else(|| panic!("{src}\n{}", djlive::diag::render_all(&c.diagnostics, src)));
    let r = render(&compiled, SR, true).expect("a render");
    assert!(r.refused.is_empty(), "the engine refused a control event: {:?}", r.refused);
    assert!(r.beside.iter().all(|(k, _)| !k.contains("drive")), "a drive line is beside the net, not in it: {:?}", r.beside);
    r.audio
}

/// A kick on deck A: four on the floor, two bars at 140 bpm (3.4 s), then whatever `lines` adds to the section.
fn kick_song(lines: &str) -> String {
    format!("tempo 140\nsection main 2 bars {{\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n{lines}}}\narrange main\n")
}

/// A low sub bass on deck B, held a bar each: 55 Hz.
fn bass_song(lines: &str) -> String {
    format!("tempo 120\nkey am\nsection main 2 bars {{\n  deck B: bass a1 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ len=16 inst=sub-bass\n{lines}}}\narrange main\n")
}

fn fnv(a: &RenderedAudio) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in a.left.iter().chain(a.right.iter()) {
        for b in s.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// The amplitude spectrum of `x` (a power-of-two prefix of it, a Hann window): bin `k` is `k * sr / n` Hz.
fn spectrum(x: &[f32]) -> (Vec<f64>, f64) {
    let n = (1usize << 30).min(x.len().next_power_of_two() / if x.len().is_power_of_two() { 1 } else { 2 });
    let (mut re, mut im): (Vec<f64>, Vec<f64>) = (0..n).map(|i| (f64::from(x[i]) * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()), 0.0)).unzip();
    // an iterative radix-2 FFT
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
    ((0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect(), SR / n as f64)
}

/// Energy of the spectrum between two frequencies.
fn band(s: &(Vec<f64>, f64), lo: f64, hi: f64) -> f64 {
    s.0.iter().enumerate().filter(|(k, _)| (*k as f64 * s.1) >= lo && (*k as f64 * s.1) < hi).map(|(_, a)| a * a).sum()
}

/// The strongest amplitude within a few bins of `hz`.
fn peak_at(s: &(Vec<f64>, f64), hz: f64) -> f64 {
    let k = (hz / s.1).round() as usize;
    (k.saturating_sub(3)..=k + 3).map(|i| s.0[i]).fold(0.0, f64::max)
}

#[test]
fn drive_zero_is_the_program_without_it_to_the_bit() {
    let dry = rendered(&kick_song(""));
    for lines in ["  master: drive 0\n", "  deck A: drive 0\n  deck B: drive 0\n", "  master: drive 0 -> 0 over 2 bars\n  deck A: drive 0 -> 0 over 1 bars exp\n"] {
        let zero = rendered(&kick_song(lines));
        assert!(zero == dry, "{lines:?} changed a sample of the render");
    }
    // and a drive that is turned on and put back to 0 by `for` leaves the bars after it as the dry ones (its tail is the 50 ms fade)
    assert_ne!(rendered(&kick_song("  master: drive 0.6\n")), dry, "a drive above 0 is heard");
}

#[test]
fn a_driven_render_is_the_same_samples_every_time_and_is_pinned() {
    let src = kick_song("  deck A: drive 0.6\n  master: drive 0.1 -> 0.8 over 2 bars\n");
    let (a, b) = (rendered(&src), rendered(&src));
    assert!(a == b, "two renders of one text differ");
    // the hash of the samples (f32 bits, left then right): the engine is libm-only, so it is the same on every target. A change
    // of it is a change of the sound of `drive` (the master chain's distortion, or the FX unit), made on purpose or not at all
    assert_eq!(fnv(&a), PINNED_KICK_DRIVE, "the driven render's samples changed: {:#018x}", fnv(&a));
}

/// The FNV-1a of the render in `a_driven_render_is_the_same_samples_every_time_and_is_pinned`.
const PINNED_KICK_DRIVE: u64 = 0x4f7e_b398_05d9_a325;

#[test]
fn a_driven_kick_has_more_of_its_energy_in_the_harmonics_than_the_dry_one() {
    let dry = rendered(&kick_song(""));
    let driven = rendered(&kick_song("  deck A: drive 0.6\n"));
    // after the first beat (the switch fades in over 50 ms). The kick sweeps down through 150 to 600 Hz on its own, so the harmonics
    // are what is over that, 600 Hz to 3 kHz, as a share of everything the render has: the driven kick has dozens of times the dry one's
    let from = SR as usize / 2;
    let (sd, sv) = (spectrum(&dry.left[from..]), spectrum(&driven.left[from..]));
    let share = |s: &(Vec<f64>, f64)| band(s, 600.0, 3000.0) / band(s, 0.0, SR / 2.0);
    let (rd, rv) = (share(&sd), share(&sv));
    eprintln!("kick energy over 600 Hz as a share of the render: dry {rd:.6}, driven {rv:.6} ({:.0}x)", rv / rd);
    assert!(rv > 10.0 * rd, "drive 0.6 did not put energy into the harmonics: {rd} -> {rv}");
    // and its body is still there: the 30 to 120 Hz band is not lost to the drive (it is a compressed tail, a little more of it)
    assert!(band(&sv, 30.0, 120.0) > 0.5 * band(&sd, 30.0, 120.0), "the body of the kick went away");
}

#[test]
fn drive_on_a_low_sine_adds_the_odd_harmonics() {
    let dry = rendered(&bass_song(""));
    let driven = rendered(&bass_song("  deck B: drive 0.3\n"));
    // 55 Hz held: skip the first beat; both harmonics are measured against the fundamental of the same render
    let from = SR as usize / 2;
    let (sd, sv) = (spectrum(&dry.left[from..from + 65536]), spectrum(&driven.left[from..from + 65536]));
    let rel = |s: &(Vec<f64>, f64), k: f64| peak_at(s, 55.0 * k) / peak_at(s, 55.0);
    let (d3, v3, v2, v4, v5) = (rel(&sd, 3.0), rel(&sv, 3.0), rel(&sv, 2.0), rel(&sv, 4.0), rel(&sv, 5.0));
    eprintln!("bass 55 Hz: 3rd dry {d3:.4} driven {v3:.4}; driven 2nd {v2:.4} 4th {v4:.4} 5th {v5:.4}");
    assert!(v3 > 10.0 * d3.max(1e-4), "the third harmonic did not come up: {d3} -> {v3}");
    assert!(v3 > 3.0 * v2 && v3 > 3.0 * v4, "the even harmonics are not far under the odd ones: 2nd {v2}, 3rd {v3}, 4th {v4}");
    assert!(v5 > 3.0 * v4, "the fifth is not over the fourth: {v5} against {v4}");
}

/// A loud mix: a full-velocity kick, a clap and hats on deck A, a sub bass under them on deck B, both decks and the master at full.
fn loud_song(lines: &str) -> String {
    format!(
        "tempo 140\nkey am\nsection main 2 bars {{\n  deck A: kick x! ~ ~ ~ x! ~ ~ ~ x! ~ ~ ~ x! ~ ~ ~ vel=127 | clap ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ vel=127 | hihat x*16 vel=127\n  deck B: bass a1 ~ ~ ~ a1 ~ ~ ~ a1 ~ ~ ~ a1 ~ ~ ~ len=3 vel=127 inst=sub-bass\n  deck A: level 1\n  deck B: level 1\n  master: level 1\n{lines}}}\narrange main\n"
    )
}

#[test]
fn drive_is_grit_not_loudness_and_never_clips() {
    let dry = rendered(&loud_song(""));
    let driven = rendered(&loud_song("  master: drive 1\n  deck A: drive 1\n"));
    // after the first beat: the drive's switch fades in over 50 ms, so the first hit goes through clean
    let from = SR as usize / 2;
    let peak = |a: &RenderedAudio| a.left[from..].iter().chain(a.right[from..].iter()).fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = |a: &RenderedAudio| (a.left[from..].iter().chain(a.right[from..].iter()).map(|s| f64::from(*s).powi(2)).sum::<f64>() / (2 * (a.left.len() - from)) as f64).sqrt();
    let clipped = |a: &RenderedAudio| a.left.iter().chain(a.right.iter()).filter(|s| s.abs() > 1.0).count();
    let (pd, pv) = (peak(&dry), peak(&driven));
    eprintln!("loud mix: peak dry {pd:.3}, drive 1 {pv:.3}; rms dry {:.3}, drive 1 {:.3}; clipped samples {} / {}", rms(&dry), rms(&driven), clipped(&dry), clipped(&driven));
    // `listen` counts the master's samples over full scale: drive 1 adds none to what the dry mix has
    assert!(clipped(&driven) <= clipped(&dry), "drive 1 clipped the master: {} samples over full scale (dry {})", clipped(&driven), clipped(&dry));
    // grit, not loudness: the curve bends a loud signal over (it does not go past about 0.35 a deck), so the driven mix peaks under the dry one
    assert!(pv < pd, "drive 1 made the mix louder at its peak: {pd} -> {pv}");
    assert!(pv <= 1.0, "a driven mix is over full scale: {pv}");
    // ... and no louder where it counts: the same RMS to within 3 dB (it is the grit that moved, not the level)
    let r = rms(&driven) / rms(&dry);
    assert!((0.7..1.42).contains(&r), "drive 1 changed the loudness of the mix by {r}x");
    // after the switch has faded in nothing is over full scale (the dry mix, as the engine renders it, has samples that are)
    assert_eq!(driven.left[from..].iter().chain(driven.right[from..].iter()).filter(|s| s.abs() > 1.0).count(), 0, "a driven mix clips");
}

#[test]
fn a_ramp_of_drive_is_events_every_sixteenth_of_a_bar_and_the_engine_takes_them_all() {
    let c = djlive::check(&kick_song("  master: drive 0.1 -> 0.9 over 2 bars\n")).compiled.expect("it checks");
    let net = djlive::nets::control_net(&c, 2);
    let ev = net.events(None);
    let drive: Vec<(i64, f64)> = ev.iter().filter_map(|(t, e)| if let djlive::nets::Event::Set { target, param, value } = e { (param == "drive" && target == "Master").then_some((*t, *value)) } else { None }).collect();
    // 2 bars of sixteenths: the first value at tick 0 and the last, exact, at tick 32
    assert_eq!(drive.first(), Some(&(0, 0.1)));
    assert_eq!(drive.last().map(|x| x.0), Some(32));
    assert!((drive.last().unwrap().1 - 0.9).abs() < 1e-9);
    assert_eq!(drive.len(), 33, "a value a sixteenth: {drive:?}");
    assert!(drive.windows(2).all(|w| w[1].1 > w[0].1), "a rising ramp rises at every step");
    // and the render applies each to both decks without a refusal (`rendered` checks `refused`)
    let _ = rendered(&kick_song("  master: drive 0.1 -> 0.9 over 2 bars\n"));
}
