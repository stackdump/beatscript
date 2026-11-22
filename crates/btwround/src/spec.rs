//! A phase-blind feature: band powers of a short-time spectrum, weighted so every band counts alike.
//!
//! Why not samples: a noise voice (hihat, snare) draws different noise for every hit, so a hit placed at another
//! step is a different waveform; and a kick and a bass at the same frequency interfere by phase. Band powers
//! see neither. Samples stay the judge only for the final L0 check.

pub const NFFT: usize = 4096;
pub const HOP: usize = 512;

/// Bin range [lo, hi) of every band: one band per bin from 32 Hz to about 400 Hz (a bass note's pitch is
/// resolved to a semitone down there), log-spaced bands above.
pub fn band_edges(sr: f64) -> Vec<(usize, usize)> {
    let hz = |f: f64| (f * NFFT as f64 / sr).round() as usize;
    let (lo, split) = (hz(32.0).max(1), hz(400.0));
    let mut edges: Vec<(usize, usize)> = (lo..split).map(|k| (k, k + 1)).collect();
    let n = 28;
    let (a, b) = (400f64.ln(), 18000f64.ln());
    let mut prev = split;
    for i in 1..=n {
        let e = hz((a + (b - a) * i as f64 / n as f64).exp()).max(prev + 1);
        edges.push((prev, e));
        prev = e;
    }
    edges
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub frames: usize,
    pub bands: usize,
    /// frames * bands band powers.
    pub data: Vec<f32>,
}

impl Spec {
    pub fn row(&self, f: usize) -> &[f32] {
        &self.data[f * self.bands..(f + 1) * self.bands]
    }
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
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
}

/// Band powers of interleaved stereo s16 (mixed to mono), frame k covering samples [k*HOP, k*HOP + NFFT).
pub fn spec_of(interleaved: &[i16], sr: f64) -> Spec {
    let edges = band_edges(sr);
    let mono: Vec<f64> = interleaved.chunks_exact(2).map(|c| (c[0] as f64 + c[1] as f64) * 0.5 / 32768.0).collect();
    let hann: Vec<f64> = (0..NFFT).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / NFFT as f64).cos()).collect();
    let frames = if mono.len() >= NFFT { (mono.len() - NFFT) / HOP + 1 } else { 0 };
    let mut data = vec![0f32; frames * edges.len()];
    let (mut re, mut im) = (vec![0f64; NFFT], vec![0f64; NFFT]);
    for f in 0..frames {
        for i in 0..NFFT {
            re[i] = mono[f * HOP + i] * hann[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        for (b, (lo, hi)) in edges.iter().enumerate() {
            let p: f64 = (*lo..*hi).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
            data[f * edges.len() + b] = p as f32;
        }
    }
    Spec { frames, bands: edges.len(), data }
}

/// Per-band weights that make every band's total target power count as 1.
pub fn band_weights(target: &Spec) -> Vec<f32> {
    (0..target.bands)
        .map(|b| {
            let total: f64 = (0..target.frames).map(|f| target.data[f * target.bands + b] as f64).sum();
            (1.0 / (total + 1e-12)) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_lands_in_its_band() {
        let sr = 44100.0;
        let f = 110.0; // a2
        let samples: Vec<i16> = (0..44100).flat_map(|i| {
            let v = ((2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() * 10000.0) as i16;
            [v, v]
        }).collect();
        let s = spec_of(&samples, sr);
        assert!(s.frames > 10);
        let edges = band_edges(sr);
        let row = s.row(s.frames / 2);
        let top = (0..s.bands).max_by(|a, b| row[*a].partial_cmp(&row[*b]).unwrap()).unwrap();
        let (lo, hi) = edges[top];
        let centre = (lo + hi - 1) as f64 / 2.0 * sr / NFFT as f64;
        assert!((centre - f).abs() < 12.0, "peak band centre {centre} Hz");
    }

    #[test]
    fn edges_ascend() {
        let e = band_edges(44100.0);
        assert!(e.windows(2).all(|w| w[0].1 <= w[1].0 || w[0].0 < w[1].0));
        assert!(e.len() > 30);
    }
}
