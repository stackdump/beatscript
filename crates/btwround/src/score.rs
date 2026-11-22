//! Scoring a recovery. L0: sample identical. L1/L2: `same_as` and its diff (see `btwd::same_as`).

use crate::wav::Pcm;

#[derive(Clone, Debug)]
pub struct L0 {
    pub frames_a: usize,
    pub frames_b: usize,
    /// Largest absolute sample difference over the common length (and the length mismatch counts as infinite).
    pub max_diff: i32,
    pub differing: usize,
    pub rms_diff: f64,
    pub rms_ref: f64,
}

impl L0 {
    pub fn identical(&self) -> bool {
        self.frames_a == self.frames_b && self.differing == 0
    }
    /// Within one LSB everywhere (rounding between two paths that sum decks differently).
    pub fn within_one_lsb(&self) -> bool {
        self.frames_a == self.frames_b && self.max_diff <= 1
    }
}

pub fn l0(a: &Pcm, b: &Pcm) -> L0 {
    let n = a.samples.len().min(b.samples.len());
    let (mut max_diff, mut differing, mut sq, mut sqr) = (0i32, 0usize, 0f64, 0f64);
    for i in 0..n {
        let d = (a.samples[i] as i32 - b.samples[i] as i32).abs();
        if d > 0 {
            differing += 1;
        }
        max_diff = max_diff.max(d);
        sq += (d as f64).powi(2);
        sqr += (a.samples[i] as f64).powi(2);
    }
    let nn = n.max(1) as f64;
    L0 { frames_a: a.frames(), frames_b: b.frames(), max_diff, differing, rms_diff: (sq / nn).sqrt(), rms_ref: (sqr / nn).sqrt() }
}

/// Sum of squared sample error of `r` against `target` (f64), `r` zero-padded or cut to the target's length.
pub fn energy_error(target: &[f32], r: &Pcm) -> f64 {
    let mut e = 0f64;
    for (i, t) in target.iter().enumerate() {
        let x = r.samples.get(i).copied().unwrap_or(0) as f32;
        let d = (*t - x) as f64;
        e += d * d;
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l0_counts() {
        let a = Pcm { sr: 44100, samples: vec![0, 10, -5, 7] };
        let mut b = a.clone();
        assert!(l0(&a, &b).identical());
        b.samples[2] = -4;
        let s = l0(&a, &b);
        assert!(!s.identical() && s.within_one_lsb() && s.differing == 1);
    }
}
