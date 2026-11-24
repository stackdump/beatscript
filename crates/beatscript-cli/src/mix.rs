//! The mix, as numbers: what `listen` says about a program's mix past its loudness, so a model can polish a mix by ear
//! (docs/beatscript/polish.md).
//!
//! - **Bands** ([`Spectrum`]): the power in six bands (sub 20-60 Hz, low 60-250, low-mid 250-500, mid 500-2k, high-mid
//!   2k-6k, high 6k up), in dB on the same scale as `rms_db` (a full-scale sine is -3), and the spectral centroid. A
//!   short-time Fourier transform (Hann window, [`Spectrum::n`] points, a frame every quarter of it) of each stem, left and
//!   right taken in one complex transform.
//! - **Stereo**: `width` (the side's share of the energy, `S / (M + S)`: 0 mono, 0.5 two unrelated channels, 1 one channel
//!   against its own inverse) and `correlation` (-1 to 1) of left and right.
//! - **True peak** ([`TruePeak`]): the master's peak between samples, 4x oversampled (BS.1770-4 annex 2, with a windowed
//!   sinc of 12 taps a phase), in dBTP, at the rate listen renders at.
//! - **Loudness over time** ([`short_term`], [`loudness_range`]): the short-term maximum (3 s windows every 100 ms) and the
//!   loudness range (EBU Tech 3342: short-term values gated at -70 LUFS and 20 LU under their mean, 95th less 10th
//!   percentile).
//! - **Clashes** ([`clashes`]): two parts that carry the same band in the same bars at levels close enough to mask each
//!   other, bar by bar, in runs of bars.
//! - **Targets** ([`Targets`]): integrated loudness and true peak against a delivery target, and the warnings that say what
//!   to change, in BeatScript lines.
//! - **Compare** ([`compare`]): two listens' answers side by side: the deltas, and the warnings fixed and introduced.
//!
//! Everything is deterministic: the same samples give the same numbers (nothing here reads a clock or a random source).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use djengine::json::{JsonObject, JsonValue};

/// The bands: name, low edge and high edge in hertz (the high band runs to the Nyquist frequency).
pub const BANDS: [(&str, f64, f64); 6] = [
    ("sub", 20.0, 60.0),
    ("low", 60.0, 250.0),
    ("low_mid", 250.0, 500.0),
    ("mid", 500.0, 2000.0),
    ("high_mid", 2000.0, 6000.0),
    ("high", 6000.0, f64::INFINITY),
];

/// "20-60 Hz", "6k Hz and up".
pub fn band_hz(j: usize) -> String {
    let (_, lo, hi) = BANDS[j];
    let k = |f: f64| if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
    if hi.is_finite() {
        format!("{}-{} Hz", k(lo), k(hi))
    } else {
        format!("{} Hz and up", k(lo))
    }
}

/// Under this a band is reported as null (nothing there).
const BAND_FLOOR_DB: f64 = -90.0;

fn db(p: f64) -> Option<f64> {
    (p > 1e-12).then(|| 10.0 * p.log10())
}

fn num1(x: Option<f64>) -> JsonValue {
    match x {
        Some(v) if v.is_finite() => JsonValue::Number((v * 10.0).round() / 10.0),
        _ => JsonValue::Null,
    }
}

fn num2(x: Option<f64>) -> JsonValue {
    match x {
        Some(v) if v.is_finite() => JsonValue::Number((v * 100.0).round() / 100.0),
        _ => JsonValue::Null,
    }
}

fn num0(x: Option<f64>) -> JsonValue {
    match x {
        Some(v) if v.is_finite() => JsonValue::Number(v.round()),
        _ => JsonValue::Null,
    }
}

// ---------------------------------------------------------------------------------------------
// the transform

/// A radix-2 complex FFT of one size, its tables made once: the bit reversal, and each stage's twiddles side by side.
struct Fft {
    n: usize,
    rev: Vec<usize>,
    twr: Vec<f64>,
    twi: Vec<f64>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        assert!(n.is_power_of_two() && n >= 4);
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let (mut twr, mut twi) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut len = 2;
        while len <= n {
            for k in 0..len / 2 {
                let a = -2.0 * std::f64::consts::PI * k as f64 / len as f64;
                twr.push(a.cos());
                twi.push(a.sin());
            }
            len <<= 1;
        }
        Fft { n, rev, twr, twi }
    }

    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let (mut len, mut off) = (2, 0);
        while len <= n {
            let half = len / 2;
            let (wr, wi) = (&self.twr[off..off + half], &self.twi[off..off + half]);
            for (cr, ci) in re.chunks_exact_mut(len).zip(im.chunks_exact_mut(len)) {
                let (ar, br) = cr.split_at_mut(half);
                let (ai, bi) = ci.split_at_mut(half);
                for k in 0..half {
                    let xr = br[k] * wr[k] - bi[k] * wi[k];
                    let xi = br[k] * wi[k] + bi[k] * wr[k];
                    br[k] = ar[k] - xr;
                    bi[k] = ai[k] - xi;
                    ar[k] += xr;
                    ai[k] += xi;
                }
            }
            off += half;
            len <<= 1;
        }
    }
}

/// One frame of a [`Spectrum`]: the 100 ms sub-block its centre falls in, its power in each band, its power over 20 Hz and
/// the power-weighted sum of its frequencies (for the centroid).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub sub: u32,
    pub bands: [f32; 6],
    pub total: f32,
    pub cnum: f32,
}

/// A stereo signal's short-time spectrum, frame by frame, fed as it renders. Frames are centred on multiples of the hop from
/// the signal's first sample (half a window of silence goes before it), so every spectrum fed from the same start has its
/// frames at the same times.
pub struct Spectrum {
    n: usize,
    hop: usize,
    sub: usize,
    fft: Fft,
    win: Vec<f64>,
    norm: f64,
    band_of: Vec<u8>,
    freq: Vec<f64>,
    ring_l: Vec<f64>,
    ring_r: Vec<f64>,
    w: usize,
    /// Samples pushed, the half window of silence before the first included.
    pushed: u64,
    /// `pushed` when the next frame is due.
    next: u64,
    /// `pushed` just after the last sample that was not zero.
    last_nz: Option<u64>,
    re: Vec<f64>,
    im: Vec<f64>,
    pub frames: Vec<Frame>,
}

impl Spectrum {
    /// The window for a rate: 2048 points at listen's 22.05 kHz (93 ms, a bin every 10.8 Hz), 4096 at 44.1 kHz and up.
    pub fn n(sr: f64) -> usize {
        if sr > 32_000.0 {
            4096
        } else {
            2048
        }
    }

    /// `sub`: samples in a 100 ms sub-block (the meters' block), to date each frame.
    pub fn new(sr: f64, sub: usize) -> Spectrum {
        let n = Spectrum::n(sr);
        let win: Vec<f64> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect();
        let norm = n as f64 * win.iter().map(|w| w * w).sum::<f64>();
        let freq: Vec<f64> = (0..=n / 2).map(|k| k as f64 * sr / n as f64).collect();
        let band_of = freq.iter().map(|f| BANDS.iter().position(|(_, lo, hi)| f >= lo && f < hi).map_or(u8::MAX, |j| j as u8)).collect();
        Spectrum {
            n,
            hop: n / 4,
            sub: sub.max(1),
            fft: Fft::new(n),
            win,
            norm,
            band_of,
            freq,
            ring_l: vec![0.0; n],
            ring_r: vec![0.0; n],
            w: 0,
            pushed: (n / 2) as u64,
            next: n as u64,
            last_nz: None,
            re: vec![0.0; n],
            im: vec![0.0; n],
            frames: Vec::new(),
        }
    }

    fn ring_clear(&self) -> bool {
        self.last_nz.map_or(true, |z| z + self.n as u64 <= self.pushed)
    }

    fn centre_sub(&self) -> u32 {
        ((self.pushed - self.n as u64) / self.sub as u64) as u32
    }

    pub fn push(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.ring_l[self.w] = f64::from(*a);
            self.ring_r[self.w] = f64::from(*b);
            self.w += 1;
            if self.w == self.n {
                self.w = 0;
            }
            self.pushed += 1;
            if *a != 0.0 || *b != 0.0 {
                self.last_nz = Some(self.pushed);
            }
            if self.pushed == self.next {
                self.frame();
                self.next += self.hop as u64;
            }
        }
    }

    /// `count` samples of silence, without a transform for the frames that see nothing else.
    pub fn push_zeros(&mut self, count: usize) {
        let mut left = count as u64;
        // until the ring holds only silence, sample by sample
        while left > 0 && !self.ring_clear() {
            self.ring_l[self.w] = 0.0;
            self.ring_r[self.w] = 0.0;
            self.w += 1;
            if self.w == self.n {
                self.w = 0;
            }
            self.pushed += 1;
            left -= 1;
            if self.pushed == self.next {
                self.frame();
                self.next += self.hop as u64;
            }
        }
        if left == 0 {
            return;
        }
        // the ring is silent and stays so: a silent frame at each frame time passed
        let n = self.n as u64;
        let end = self.pushed + left;
        while self.next <= end {
            self.frames.push(Frame { sub: ((self.next - n) / self.sub as u64) as u32, bands: [0.0; 6], total: 0.0, cnum: 0.0 });
            self.next += self.hop as u64;
        }
        self.w = ((self.w as u64 + left) % n) as usize;
        self.pushed = end;
    }

    /// Half a window of silence after the last sample, so the last of the signal has frames centred on it.
    pub fn finish(&mut self) {
        self.push_zeros(self.n / 2);
    }

    fn frame(&mut self) {
        let sub = self.centre_sub();
        if self.ring_clear() {
            self.frames.push(Frame { sub, bands: [0.0; 6], total: 0.0, cnum: 0.0 });
            return;
        }
        let n = self.n;
        // the ring from its oldest sample (at w), windowed
        let older = n - self.w;
        for (dst, src) in [(&mut self.re, &self.ring_l), (&mut self.im, &self.ring_r)] {
            dst[..older].copy_from_slice(&src[self.w..]);
            dst[older..].copy_from_slice(&src[..self.w]);
            for (x, w) in dst.iter_mut().zip(&self.win) {
                *x *= w;
            }
        }
        self.fft.run(&mut self.re, &mut self.im);
        let mut bands = [0.0f64; 6];
        let (mut total, mut cnum) = (0.0f64, 0.0f64);
        for k in 1..n / 2 {
            let j = self.band_of[k];
            if j == u8::MAX {
                continue;
            }
            // left and right from the one transform of l + i r
            let (a, b, c, d) = (self.re[k], self.im[k], self.re[n - k], self.im[n - k]);
            let pl = ((a + c) * (a + c) + (b - d) * (b - d)) / 4.0;
            let pr = ((b + d) * (b + d) + (a - c) * (a - c)) / 4.0;
            // one-sided, the channels' mean: the signal's mean square in this bin
            let p = (pl + pr) / self.norm;
            bands[j as usize] += p;
            total += p;
            cnum += p * self.freq[k];
        }
        self.frames.push(Frame { sub, bands: bands.map(|x| x as f32), total: total as f32, cnum: cnum as f32 });
    }

    /// The frames whose centre is in sub-blocks `r`.
    pub fn frames_in(&self, r: Range<usize>) -> &[Frame] {
        let a = self.frames.partition_point(|f| (f.sub as usize) < r.start);
        let b = self.frames.partition_point(|f| (f.sub as usize) < r.end);
        &self.frames[a..b.max(a)]
    }
}

/// The mean band powers, the mean power and the centroid (Hz) of some frames.
pub fn band_means<'a>(frames: impl Iterator<Item = &'a Frame>) -> Option<([f64; 6], f64, Option<f64>)> {
    let mut bands = [0.0f64; 6];
    let (mut total, mut cnum, mut n) = (0.0f64, 0.0f64, 0usize);
    for f in frames {
        for j in 0..6 {
            bands[j] += f64::from(f.bands[j]);
        }
        total += f64::from(f.total);
        cnum += f64::from(f.cnum);
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let k = n as f64;
    Some((bands.map(|b| b / k), total / k, (total > 1e-15).then(|| cnum / total)))
}

// ---------------------------------------------------------------------------------------------
// true peak

/// Taps of the three in-between phases (a quarter, a half and three quarters of a sample on): a sinc windowed over its 12
/// taps (Blackman), each phase's taps summing to 1.
fn tp_taps() -> [[f64; 12]; 3] {
    let mut t = [[0.0; 12]; 3];
    for (p, row) in t.iter_mut().enumerate() {
        let frac = (p + 1) as f64 / 4.0;
        for (j, h) in row.iter_mut().enumerate() {
            let x = j as f64 - 5.0 - frac;
            let sinc = if x.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let u = std::f64::consts::PI * x / 6.0;
            let w = 0.42 + 0.5 * u.cos() + 0.08 * (2.0 * u).cos();
            *h = sinc * w;
        }
        let s: f64 = row.iter().sum();
        for h in row.iter_mut() {
            *h /= s;
        }
    }
    t
}

/// The master's true peak: the largest of each sample and the three points between it and the next, both channels, kept as
/// a maximum a sub-block.
pub struct TruePeak {
    taps: [[f64; 12]; 3],
    /// The last 12 samples of each channel, written twice (at `i` and `i + 12`) so they are always in a row.
    hl: [f64; 24],
    hr: [f64; 24],
    i: usize,
    sub: usize,
    n: usize,
    cur: f64,
    pub blocks: Vec<f32>,
}

impl TruePeak {
    pub fn new(sub: usize) -> TruePeak {
        TruePeak { taps: tp_taps(), hl: [0.0; 24], hr: [0.0; 24], i: 0, sub: sub.max(1), n: 0, cur: 0.0, blocks: Vec::new() }
    }

    fn one(&mut self, l: f64, r: f64) {
        self.hl[self.i] = l;
        self.hl[self.i + 12] = l;
        self.hr[self.i] = r;
        self.hr[self.i + 12] = r;
        self.i += 1;
        if self.i == 12 {
            self.i = 0;
        }
        // the last 12, oldest first
        let (wl, wr) = (&self.hl[self.i..self.i + 12], &self.hr[self.i..self.i + 12]);
        let mut m = l.abs().max(r.abs());
        for row in &self.taps {
            let (mut yl, mut yr) = (0.0, 0.0);
            for j in 0..12 {
                yl += row[j] * wl[j];
                yr += row[j] * wr[j];
            }
            m = m.max(yl.abs()).max(yr.abs());
        }
        self.cur = self.cur.max(m);
        self.n += 1;
        if self.n == self.sub {
            self.close();
        }
    }

    fn close(&mut self) {
        self.blocks.push(self.cur as f32);
        self.cur = 0.0;
        self.n = 0;
    }

    pub fn push(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.one(f64::from(*a), f64::from(*b));
        }
    }

    pub fn push_zeros(&mut self, n: usize) {
        for _ in 0..n {
            self.one(0.0, 0.0);
        }
    }

    pub fn finish(&mut self) {
        // the last in-between points ring out of the window
        for _ in 0..6 {
            self.one(0.0, 0.0);
        }
        if self.n > 0 {
            self.close();
        }
    }

    /// dBTP over sub-blocks `r`.
    pub fn over(&self, r: Range<usize>) -> Option<f64> {
        let end = r.end.min(self.blocks.len());
        let p = self.blocks[r.start.min(end)..end].iter().fold(0.0f32, |a, b| a.max(*b));
        (p > 1e-6).then(|| 20.0 * f64::from(p).log10())
    }
}

// ---------------------------------------------------------------------------------------------
// loudness over time

fn lk(e: f64) -> f64 {
    -0.691 + 10.0 * e.log10()
}

/// Short-term loudness (LUFS) of each 3 s window (`win` sub-blocks) of the K-weighted sub-block energies, one a sub-block;
/// a stretch shorter than a window is one value. Silent windows are left out.
pub fn short_term(k: &[f64], win: usize) -> Vec<f64> {
    if k.is_empty() {
        return Vec::new();
    }
    let w = win.max(1).min(k.len());
    let mut out = Vec::new();
    let mut s: f64 = k[..w].iter().sum();
    for i in 0..=k.len() - w {
        if i > 0 {
            s += k[i + w - 1] - k[i - 1];
        }
        let e = s / w as f64;
        if e > 1e-15 {
            out.push(lk(e));
        }
    }
    out
}

/// EBU Tech 3342 loudness range (LU) from short-term values: those over -70 LUFS, then those within 20 LU under their mean
/// (power mean); the 95th percentile less the 10th. None with fewer than two.
pub fn loudness_range(st: &[f64]) -> Option<f64> {
    let abs: Vec<f64> = st.iter().copied().filter(|l| *l > -70.0).collect();
    if abs.len() < 2 {
        return None;
    }
    let mean = lk(abs.iter().map(|l| 10f64.powf((l + 0.691) / 10.0)).sum::<f64>() / abs.len() as f64);
    let mut kept: Vec<f64> = abs.into_iter().filter(|l| *l >= mean - 20.0).collect();
    if kept.len() < 2 {
        return None;
    }
    kept.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| kept[((p * (kept.len() - 1) as f64).round() as usize).min(kept.len() - 1)];
    Some(at(0.95) - at(0.10))
}

// ---------------------------------------------------------------------------------------------
// a signal's mix measures

/// Bands, centroid and stereo of a signal over a window.
#[derive(Clone, Copy, Debug, Default)]
pub struct MixStats {
    pub bands: [Option<f64>; 6],
    /// Mean power of each band (linear), for the shares.
    pub power: [f64; 6],
    pub centroid: Option<f64>,
    pub width: Option<f64>,
    pub correlation: Option<f64>,
}

impl MixStats {
    /// From band means and the window's sums of l², r² and l·r.
    pub fn of(spec: Option<([f64; 6], f64, Option<f64>)>, ll: f64, rr: f64, lr: f64) -> MixStats {
        let mut m = MixStats::default();
        if let Some((b, _, c)) = spec {
            // a band 90 dB down is not in the signal: null, not a number that only says so
            m.bands = b.map(|x| db(x).filter(|d| *d > BAND_FLOOR_DB));
            m.power = b;
            m.centroid = c;
        }
        if ll + rr > 1e-9 {
            m.width = Some((0.5 - lr / (ll + rr)).clamp(0.0, 1.0));
            if ll > 1e-12 && rr > 1e-12 {
                m.correlation = Some((lr / (ll * rr).sqrt()).clamp(-1.0, 1.0));
            }
        }
        m
    }

    /// The share of the power under 250 Hz.
    pub fn low_share(&self) -> f64 {
        let t: f64 = self.power.iter().sum();
        if t <= 0.0 {
            0.0
        } else {
            (self.power[0] + self.power[1]) / t
        }
    }

    pub fn json(&self, o: &mut JsonObject) {
        if self.bands.iter().any(Option::is_some) {
            o.set("bands", bands_json(&self.bands));
        }
        o.set("centroid_hz", num0(self.centroid));
        o.set("width", num2(self.width));
        o.set("correlation", num2(self.correlation));
    }
}

pub fn bands_json(b: &[Option<f64>; 6]) -> JsonValue {
    let mut o = JsonObject::new();
    for (j, (name, ..)) in BANDS.iter().enumerate() {
        o.set(*name, num1(b[j]));
    }
    JsonValue::Object(o)
}

// ---------------------------------------------------------------------------------------------
// clashes

/// Roles that are percussion: two of them sharing a band is a drum kit, not a clash.
const DRUMS: [&str; 4] = ["kick", "snare", "hihat", "clap"];
/// A band is one a part lives in when at least this share of its power (in a bar) is there.
const SHARE: f64 = 0.25;
/// How much of the quieter part's band energy is met, frame by frame, by as much of the other's: at least this is a clash.
const OVERLAP: f64 = 0.5;
/// Both parts are within this many dB of the master's band in the bar (they are what the band is made of).
const OF_MASTER_DB: f64 = 10.0;
/// A band quieter than this (dB) is not heard enough to clash.
const FLOOR_DB: f64 = -60.0;
/// Clashes kept in the answer.
pub const MAX_CLASHES: usize = 8;

/// One part's spectrum, for [`clashes`].
pub struct PartSpectrum<'a> {
    pub deck: String,
    pub role: String,
    pub spec: &'a Spectrum,
}

/// Two parts that mask each other in a band: `a` is the louder there.
#[derive(Clone, Debug)]
pub struct Clash {
    pub a: (String, String),
    pub b: (String, String),
    pub band: usize,
    /// Runs of program bars, 1-based and inclusive.
    pub bars: Vec<(u32, u32)>,
    pub overlap: f64,
    /// How much louder `a` is than `b` in the band over those bars (dB, >= 0).
    pub gap_db: f64,
    pub sections: Vec<String>,
}

impl Clash {
    pub fn bar_count(&self) -> u32 {
        self.bars.iter().map(|(a, b)| b - a + 1).sum()
    }

    pub fn id(&self) -> String {
        let (x, y) = (format!("{}.{}", self.a.0, self.a.1), format!("{}.{}", self.b.0, self.b.1));
        let (x, y) = if x <= y { (x, y) } else { (y, x) };
        format!("clash:{x}/{y}:{}", BANDS[self.band].0)
    }

    pub fn json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("parts", JsonValue::Array(vec![JsonValue::str(format!("{}.{}", self.a.0, self.a.1)), JsonValue::str(format!("{}.{}", self.b.0, self.b.1))]));
        o.set("band", JsonValue::str(BANDS[self.band].0));
        o.set("hz", JsonValue::str(band_hz(self.band)));
        o.set("bars", JsonValue::Array(self.bars.iter().map(|(a, b)| JsonValue::Array(vec![JsonValue::int(i64::from(*a)), JsonValue::int(i64::from(*b))])).collect()));
        o.set("overlap", num2(Some(self.overlap)));
        o.set("gap_db", num1(Some(self.gap_db)));
        o.set("sections", JsonValue::Array(self.sections.iter().cloned().map(JsonValue::str).collect()));
        JsonValue::Object(o)
    }
}

/// "bars 17-32 and 45-52", "bar 3".
pub fn bars_words(runs: &[(u32, u32)]) -> String {
    let one = |(a, b): &(u32, u32)| if a == b { format!("{a}") } else { format!("{a}-{b}") };
    let all: Vec<String> = runs.iter().map(one).collect();
    let single = runs.len() == 1 && runs[0].0 == runs[0].1;
    let list = match all.len() {
        0 => String::new(),
        1 => all[0].clone(),
        n => format!("{} and {}", all[..n - 1].join(", "), all[n - 1]),
    };
    format!("{} {list}", if single { "bar" } else { "bars" })
}

/// The clashes between `parts` over `bars` (each a program bar, 0-based, with its sub-blocks), most bars first, at most
/// [`MAX_CLASHES`]. `master` is the main pass's master; `section_of` names the section a bar is in. A clash needs runs of at
/// least `min_run` bars.
pub fn clashes(parts: &[PartSpectrum], master: &Spectrum, bars: &[(u32, Range<usize>)], section_of: &dyn Fn(u32) -> Option<String>, min_run: u32) -> Vec<Clash> {
    // per part, per bar: per frame band powers and totals
    struct Hit {
        bar: u32,
        overlap: f64,
        pa: f64,
        pb: f64,
    }
    let mut found: BTreeMap<(usize, usize, usize), Vec<Hit>> = BTreeMap::new();
    for (bar, r) in bars {
        let mf = master.frames_in(r.clone());
        let Some((mb, ..)) = band_means(mf.iter()) else { continue };
        let fr: Vec<&[Frame]> = parts.iter().map(|p| p.spec.frames_in(r.clone())).collect();
        let means: Vec<Option<([f64; 6], f64, Option<f64>)>> = fr.iter().map(|f| band_means(f.iter())).collect();
        for i in 0..parts.len() {
            for k in i + 1..parts.len() {
                if DRUMS.contains(&parts[i].role.as_str()) && DRUMS.contains(&parts[k].role.as_str()) {
                    continue;
                }
                let (Some((bi, ti, _)), Some((bk, tk, _))) = (means[i], means[k]) else { continue };
                if fr[i].len() != fr[k].len() || ti <= 0.0 || tk <= 0.0 {
                    continue;
                }
                for j in 0..6 {
                    if bi[j] / ti < SHARE || bk[j] / tk < SHARE {
                        continue;
                    }
                    if db(bi[j].min(bk[j])).map_or(true, |d| d < FLOOR_DB) {
                        continue;
                    }
                    let floor = mb[j] * 10f64.powf(-OF_MASTER_DB / 10.0);
                    if bi[j] < floor || bk[j] < floor {
                        continue;
                    }
                    let (mut both, mut si, mut sk) = (0.0, 0.0, 0.0);
                    for (a, b) in fr[i].iter().zip(fr[k].iter()) {
                        let (x, y) = (f64::from(a.bands[j]), f64::from(b.bands[j]));
                        both += x.min(y);
                        si += x;
                        sk += y;
                    }
                    let ov = both / si.min(sk).max(1e-30);
                    if ov >= OVERLAP {
                        found.entry((i, k, j)).or_default().push(Hit { bar: *bar, overlap: ov, pa: bi[j], pb: bk[j] });
                    }
                }
            }
        }
    }
    let mut out: Vec<Clash> = Vec::new();
    for ((i, k, j), hits) in found {
        // runs of consecutive bars, at least min_run long
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut kept: Vec<&Hit> = Vec::new();
        let mut s = 0;
        while s < hits.len() {
            let mut e = s;
            while e + 1 < hits.len() && hits[e + 1].bar == hits[e].bar + 1 {
                e += 1;
            }
            if hits[e].bar - hits[s].bar + 1 >= min_run {
                runs.push((hits[s].bar + 1, hits[e].bar + 1));
                kept.extend(&hits[s..=e]);
            }
            s = e + 1;
        }
        if kept.is_empty() {
            continue;
        }
        let n = kept.len() as f64;
        let overlap = kept.iter().map(|h| h.overlap).sum::<f64>() / n;
        let (pa, pb) = (kept.iter().map(|h| h.pa).sum::<f64>() / n, kept.iter().map(|h| h.pb).sum::<f64>() / n);
        let (louder, quieter) = if pa >= pb { (i, k) } else { (k, i) };
        let gap = 10.0 * (pa.max(pb) / pa.min(pb).max(1e-30)).log10();
        let mut sections: Vec<String> = Vec::new();
        for h in &kept {
            if let Some(sn) = section_of(h.bar) {
                if !sections.contains(&sn) {
                    sections.push(sn);
                }
            }
        }
        out.push(Clash {
            a: (parts[louder].deck.clone(), parts[louder].role.clone()),
            b: (parts[quieter].deck.clone(), parts[quieter].role.clone()),
            band: j,
            bars: runs,
            overlap,
            gap_db: gap,
            sections,
        });
    }
    out.sort_by(|x, y| y.bar_count().cmp(&x.bar_count()).then(y.overlap.total_cmp(&x.overlap)).then(x.id().cmp(&y.id())));
    out.truncate(MAX_CLASHES);
    out
}

// ---------------------------------------------------------------------------------------------
// targets

/// A delivery target: integrated loudness and a true-peak ceiling.
#[derive(Clone, Debug, PartialEq)]
pub struct Targets {
    pub name: String,
    pub lufs: f64,
    pub true_peak: f64,
}

/// The presets: name, LUFS, dBTP, what it is.
pub const PRESETS: [(&str, f64, f64, &str); 4] = [
    ("streaming", -14.0, -1.0, "Spotify, YouTube, Tidal and most streaming services normalise to about -14 LUFS"),
    ("apple", -16.0, -1.0, "Apple Music's Sound Check"),
    ("broadcast", -23.0, -1.0, "EBU R 128"),
    ("club", -8.0, -0.3, "a loud dance master for DJ play: common practice, not a standard"),
];

impl Default for Targets {
    fn default() -> Self {
        Targets { name: "streaming".into(), lufs: -14.0, true_peak: -1.0 }
    }
}

impl Targets {
    /// A preset's name, or `LUFS/dBTP` (`-12/-1`).
    pub fn parse(s: &str) -> Result<Targets, String> {
        let s = s.trim();
        if let Some((n, l, t, _)) = PRESETS.iter().find(|p| p.0 == s) {
            return Ok(Targets { name: (*n).into(), lufs: *l, true_peak: *t });
        }
        let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
        let bad = || format!("targets is a preset ({}) or LUFS/dBTP such as -12/-1, not `{s}`", names.join(", "));
        let (l, t) = s.split_once('/').ok_or_else(bad)?;
        let (l, t) = (l.trim().parse::<f64>().map_err(|_| bad())?, t.trim().parse::<f64>().map_err(|_| bad())?);
        if !(-40.0..=0.0).contains(&l) || !(-10.0..=0.0).contains(&t) {
            return Err(format!("targets {s}: the loudness is -40 to 0 LUFS and the ceiling -10 to 0 dBTP"));
        }
        Ok(Targets { name: "custom".into(), lufs: l, true_peak: t })
    }

    /// For a cache key (the MCP tool's).
    #[allow(dead_code)]
    pub fn key(&self) -> String {
        format!("{}:{}/{}", self.name, self.lufs, self.true_peak)
    }

    pub fn json(&self, lufs: Option<f64>, tp: Option<f64>) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("name", JsonValue::str(self.name.clone()));
        o.set("lufs", JsonValue::Number(self.lufs));
        o.set("true_peak_db", JsonValue::Number(self.true_peak));
        let ld = lufs.map(|l| l - self.lufs);
        let td = tp.map(|t| t - self.true_peak);
        o.set("lufs_delta", num1(ld));
        o.set("true_peak_delta", num1(td));
        let met = matches!((ld, td), (Some(l), Some(t)) if l.abs() <= 1.0 && t <= 0.0);
        o.set("met", JsonValue::Bool(met));
        JsonValue::Object(o)
    }
}

// ---------------------------------------------------------------------------------------------
// warnings: what to change, in BeatScript lines

/// A warning: a stable id (for [`compare`]) and the words.
pub type Warning = (String, String);

fn level_words(x: f64) -> String {
    let v = (x * 100.0).round() / 100.0;
    JsonValue::Number(v).js_stringified()
}

/// The true-peak and loudness warnings against `t`. `master_level` is the master fader (0..1) in the window.
pub fn target_warnings(t: &Targets, lufs: Option<f64>, tp: Option<f64>, master_level: f64) -> Vec<Warning> {
    let mut out = Vec::new();
    let (Some(l), Some(p)) = (lufs, tp) else { return out };
    let ml = master_level.clamp(0.0, 1.0);
    let at = |db: f64| ml * 10f64.powf(db / 20.0);
    let d = l - t.lufs;
    let over = p > t.true_peak + 0.05;
    let cut = p - t.true_peak;
    // a level that meets the ceiling and the target at once: the loudness asks for a cut at least as deep as the peaks do
    let both = over && d > 1.0 && d >= cut;
    // the peaks stand so far over the loudness that the level the ceiling needs leaves the master well under the target
    let peaky = over && !both && l - cut < t.lufs - 1.0;
    if over {
        let how = if both {
            format!("take {d:.1} dB off (`master: level {}`), which also brings it to the target", level_words(at(-d)))
        } else if peaky {
            format!(
                "its peaks stand {:.1} dB over its loudness, so the level that meets the ceiling (`master: level {}`) would leave it {:.1} LU under the target: round the peaks first with `master: drive` or a lower `vel=` on the loudest hits (BeatScript has no limiter), then set the level",
                p - l,
                level_words(at(-cut)),
                t.lufs - (l - cut)
            )
        } else {
            format!("take {cut:.1} dB off (`master: level {}`), or round the peaks with `master: drive`", level_words(at(-cut)))
        };
        out.push(("true_peak".into(), format!("the master's true peak is {p:.1} dBTP, over the {} ceiling of {} dBTP: {how}", t.name, t.true_peak)));
    }
    if d < -1.0 {
        let need = -d;
        let room = t.true_peak - p;
        let to = at(need);
        let how = if room + 0.05 < need {
            format!("its true peak leaves {:.1} dB of room under {} dBTP, so a level alone would pass it: round the peaks with `master: drive` (BeatScript has no limiter), then raise the level", room.max(0.0), t.true_peak)
        } else if to <= 1.0 {
            format!("`master: level {}`", level_words(to))
        } else {
            format!("the master is at level {} and goes to 1 at most: raise the decks' `level` or the loudest parts' `vel=`", level_words(ml))
        };
        out.push(("loudness:low".into(), format!("the master is {l:.1} LUFS, {need:.1} LU under the {} target of {} LUFS: {how}", t.name, t.lufs)));
    } else if d > 1.0 {
        let how = if both {
            format!("the true peak's `master: level {}` fixes both", level_words(at(-d)))
        } else if peaky {
            format!("once the peaks are rounded, `master: level {}`", level_words(at(-d)))
        } else {
            format!("`master: level {}` gives the room back to the peaks", level_words(at(-d)))
        };
        out.push(("loudness:high".into(), format!("the master is {l:.1} LUFS, {d:.1} LU over the {} target of {} LUFS, which turns it down by as much: {how}", t.name, t.lufs)));
    }
    if p - l < 6.0 && l > -20.0 {
        out.push(("squashed".into(), format!("the master is squashed: its true peak is only {:.1} dB over its loudness (PLR); ease `drive` on the master or the decks", p - l)));
    }
    out
}

/// A clash's warning. `roles_of` answers how many parts a deck has.
pub fn clash_warning(c: &Clash, roles_of: &dyn Fn(&str) -> usize) -> Warning {
    let name = |(d, r): &(String, String)| format!("deck {d} {r}");
    let head = format!("{} and {} clash at {} ({}) in {}", name(&c.a), name(&c.b), band_hz(c.band), BANDS[c.band].0, bars_words(&c.bars));
    let low = c.band <= 1;
    let eq_band = match c.band {
        0 | 1 => "low",
        2 | 3 => "mid",
        _ => "high",
    };
    // the one to change: under 250 Hz the kick and the bass own the band, so the other; else the quieter, unless it is a drum
    // and the louder is not
    let owns_low = |r: &str| r == "kick" || r == "bass";
    let (mv, keep) = if low && owns_low(c.a.1.as_str()) != owns_low(c.b.1.as_str()) {
        if owns_low(c.a.1.as_str()) {
            (&c.b, &c.a)
        } else {
            (&c.a, &c.b)
        }
    } else if DRUMS.contains(&c.b.1.as_str()) && !DRUMS.contains(&c.a.1.as_str()) {
        (&c.a, &c.b)
    } else {
        (&c.b, &c.a)
    };
    // a deck's EQ and filter move every part of it: say so when the deck has others
    let wide = |d: &str| if roles_of(d) > 1 { format!(" (on all of deck {d})") } else { String::new() };
    let fix = if keep.0 == mv.0 {
        let duck = if low && keep.1 == "kick" { format!(" and `duck` it under {}.kick", keep.0) } else { String::new() };
        format!("they share deck {}, so its EQ and a duck move both: lower one's `vel=`, move one an octave (`oct=`), or put the {} on a deck of its own{duck}", keep.0, mv.1)
    } else if low && keep.1 == "kick" {
        format!("`duck {} under {}.kick 6db`, or `mix {{ {} eq low -6 }}`{}", mv.0, keep.0, mv.0, wide(&mv.0))
    } else if low && !owns_low(mv.1.as_str()) {
        format!("`mix {{ {} eq low -6 }}` or `deck {}: filter hp 150`{}", mv.0, mv.0, wide(&mv.0))
    } else {
        format!("`mix {{ {} eq {eq_band} -4 }}`{}, or lower the {}'s `vel=`", mv.0, wide(&mv.0), mv.1)
    };
    (c.id(), format!("{head}: {fix}"))
}

/// A part far under the mix.
pub fn buried_warning(deck: &str, role: &str, under: f64, deck_roles: usize, deck_level: f64) -> Warning {
    let fix = if deck_roles == 1 && deck_level < 0.99 {
        format!("raise `deck {deck}: level` (it is at {})", level_words(deck_level))
    } else {
        "raise its `vel=`".to_string()
    };
    (format!("buried:{deck}.{role}"), format!("deck {deck} {role} is {under:.0} LU under the mix and barely heard: {fix}"))
}

/// A part whose energy is under 250 Hz and that is wide.
pub fn wide_low_warning(deck: &str, role: &str, width: f64) -> Warning {
    (
        format!("wide_low:{deck}.{role}"),
        format!("deck {deck} {role} lives under 250 Hz and is wide (width {width:.2}): low end is best mono, and BeatScript has no width control on a strip, so try a narrower voice (`inst=`)"),
    )
}

/// The master's left and right working against each other.
pub fn phase_warning(corr: f64, widest: Option<(&str, &str, f64)>) -> Warning {
    let who = widest.map(|(d, r, w)| format!(": the widest part is deck {d} {r} (width {w:.2}); try another `inst=` for it")).unwrap_or_default();
    ("phase".into(), format!("the master's left and right work against each other (correlation {corr:.2}), so it thins out in mono{who}"))
}

// ---------------------------------------------------------------------------------------------
// compare

fn f(v: &JsonValue, k: &str) -> Option<f64> {
    v.get(k).and_then(JsonValue::as_f64)
}

/// `cur - prev` for each numeric key, rounded as the key is; a pair `[prev, cur]` when one side is missing, nothing when equal.
fn deltas(prev: &JsonValue, cur: &JsonValue, keys: &[(&str, u8)], o: &mut JsonObject) {
    for (k, places) in keys {
        let (a, b) = (f(prev, k), f(cur, k));
        match (a, b) {
            (Some(a), Some(b)) => {
                let s = 10f64.powi(i32::from(*places));
                let d = ((b - a) * s).round() / s;
                if d != 0.0 {
                    o.set(*k, JsonValue::Number(d));
                }
            }
            (None, None) => {}
            _ => o.set(*k, JsonValue::Array(vec![a.map_or(JsonValue::Null, JsonValue::Number), b.map_or(JsonValue::Null, JsonValue::Number)])),
        }
    }
    if let (Some(pb), Some(cb)) = (prev.get("bands"), cur.get("bands")) {
        let mut bo = JsonObject::new();
        deltas(pb, cb, &BANDS.map(|(n, ..)| (n, 1)), &mut bo);
        if !bo.is_empty() {
            o.set("bands", JsonValue::Object(bo));
        }
    }
}

const SIGNAL_KEYS: [(&str, u8); 8] = [("lufs", 1), ("rms_db", 1), ("peak_db", 1), ("crest_db", 1), ("centroid_hz", 0), ("width", 2), ("correlation", 2), ("true_peak_db", 1)];
const MASTER_KEYS: [(&str, u8); 4] = [("short_term_max_lufs", 1), ("lra", 1), ("plr_db", 1), ("clipped_samples", 0)];

fn arr<'a>(v: &'a JsonValue, k: &str) -> &'a [JsonValue] {
    v.get(k).and_then(JsonValue::as_array).map_or(&[], Vec::as_slice)
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k).and_then(JsonValue::as_str).unwrap_or("")
}

/// The warnings of an answer by id (its `warning_ids` and `warnings`, side by side), in order.
fn warnings_by_id(v: &JsonValue) -> Vec<(String, String)> {
    let ids = arr(v, "warning_ids");
    let ws = arr(v, "warnings");
    ws.iter().enumerate().map(|(i, w)| (ids.get(i).and_then(JsonValue::as_str).map_or_else(|| w.as_str().unwrap_or("").to_string(), str::to_string), w.as_str().unwrap_or("").to_string())).collect()
}

/// What changed from `prev` to `cur` (two listens' answers): deltas (`cur - prev`) of the master, each part and each section
/// (by deck and part, by section name), the targets before and after, and the warnings fixed (in `prev`, not in `cur`, by
/// id), introduced and kept.
pub fn compare(prev: &JsonValue, cur: &JsonValue) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("cid", JsonValue::str(s(prev, "cid").to_string()));
    let mut notes: Vec<JsonValue> = Vec::new();
    if let (Some(pw), Some(cw)) = (prev.get("window"), cur.get("window")) {
        if pw.canonical_js() != cw.canonical_js() {
            notes.push(JsonValue::str(format!(
                "the windows differ (before: {} bars from bar {}, now: {} from bar {}): a delta of a loudness or a band is of different music",
                f(pw, "bars").unwrap_or(0.0),
                f(pw, "from_bar").unwrap_or(0.0),
                f(cw, "bars").unwrap_or(0.0),
                f(cw, "from_bar").unwrap_or(0.0)
            )));
        }
    }
    // the master
    let (pm, cm) = (prev.get("master").cloned().unwrap_or(JsonValue::Null), cur.get("master").cloned().unwrap_or(JsonValue::Null));
    let mut m = JsonObject::new();
    deltas(&pm, &cm, &SIGNAL_KEYS, &mut m);
    deltas(&pm, &cm, &MASTER_KEYS, &mut m);
    o.set("master", JsonValue::Object(m));
    // parts
    let key = |p: &JsonValue| format!("{}.{}", s(p, "deck"), s(p, "part"));
    let pp: BTreeMap<String, &JsonValue> = arr(prev, "parts").iter().map(|p| (key(p), p)).collect();
    let mut parts = Vec::new();
    let mut added = Vec::new();
    let mut seen = BTreeSet::new();
    for c in arr(cur, "parts") {
        let k = key(c);
        seen.insert(k.clone());
        let Some(p) = pp.get(&k) else {
            added.push(JsonValue::str(k));
            continue;
        };
        let mut d = JsonObject::new();
        deltas(p, c, &SIGNAL_KEYS, &mut d);
        if p.get("silent").and_then(JsonValue::as_bool) != c.get("silent").and_then(JsonValue::as_bool) {
            d.set("silent", JsonValue::Array(vec![p.get("silent").cloned().unwrap_or(JsonValue::Null), c.get("silent").cloned().unwrap_or(JsonValue::Null)]));
        }
        if !d.is_empty() {
            let mut e = JsonObject::new();
            e.set("deck", JsonValue::str(s(c, "deck").to_string()));
            e.set("part", JsonValue::str(s(c, "part").to_string()));
            for (kk, vv) in d.iter() {
                e.set(kk.to_string(), vv.clone());
            }
            parts.push(JsonValue::Object(e));
        }
    }
    let removed: Vec<JsonValue> = pp.keys().filter(|k| !seen.contains(*k)).cloned().map(JsonValue::str).collect();
    o.set("parts", JsonValue::Array(parts));
    if !added.is_empty() {
        o.set("parts_added", JsonValue::Array(added));
    }
    if !removed.is_empty() {
        o.set("parts_removed", JsonValue::Array(removed));
    }
    // sections, by name (an unnamed one by its place)
    let skey = |i: usize, v: &JsonValue| v.get("name").and_then(JsonValue::as_str).map_or_else(|| format!("#{}", i + 1), str::to_string);
    let ps: BTreeMap<String, &JsonValue> = arr(prev, "sections").iter().enumerate().map(|(i, v)| (skey(i, v), v)).collect();
    let mut secs = Vec::new();
    for (i, c) in arr(cur, "sections").iter().enumerate() {
        let k = skey(i, c);
        let Some(p) = ps.get(&k) else { continue };
        let mut d = JsonObject::new();
        deltas(p, c, &SIGNAL_KEYS, &mut d);
        if !d.is_empty() {
            let mut e = JsonObject::new();
            e.set("name", JsonValue::str(k));
            for (kk, vv) in d.iter() {
                e.set(kk.to_string(), vv.clone());
            }
            secs.push(JsonValue::Object(e));
        }
    }
    o.set("sections", JsonValue::Array(secs));
    // targets
    if let (Some(pt), Some(ct)) = (prev.get("targets"), cur.get("targets")) {
        let mut t = JsonObject::new();
        let pick = |v: &JsonValue| {
            let mut x = JsonObject::new();
            for k in ["lufs_delta", "true_peak_delta", "met"] {
                x.set(k, v.get(k).cloned().unwrap_or(JsonValue::Null));
            }
            JsonValue::Object(x)
        };
        t.set("before", pick(pt));
        t.set("after", pick(ct));
        o.set("targets", JsonValue::Object(t));
    }
    // warnings
    let (pw, cw) = (warnings_by_id(prev), warnings_by_id(cur));
    let pids: BTreeSet<&str> = pw.iter().map(|(i, _)| i.as_str()).collect();
    let cids: BTreeSet<&str> = cw.iter().map(|(i, _)| i.as_str()).collect();
    let mut fixed: Vec<JsonValue> = Vec::new();
    let mut fseen = BTreeSet::new();
    for (i, w) in &pw {
        if !cids.contains(i.as_str()) && fseen.insert(i.clone()) {
            fixed.push(JsonValue::str(w.clone()));
        }
    }
    let mut introduced: Vec<JsonValue> = Vec::new();
    let mut iseen = BTreeSet::new();
    for (i, w) in &cw {
        if !pids.contains(i.as_str()) && iseen.insert(i.clone()) {
            introduced.push(JsonValue::str(w.clone()));
        }
    }
    let kept = cids.intersection(&pids).count();
    // one line
    let mut said: Vec<String> = Vec::new();
    if let (Some(a), Some(b)) = (f(&pm, "lufs"), f(&cm, "lufs")) {
        said.push(format!("master {a} -> {b} LUFS ({:+.1})", b - a));
    }
    if let (Some(a), Some(b)) = (f(&pm, "true_peak_db"), f(&cm, "true_peak_db")) {
        said.push(format!("true peak {a} -> {b} dBTP"));
    }
    said.push(format!("{} warning{} fixed, {} introduced, {kept} kept", fixed.len(), if fixed.len() == 1 { "" } else { "s" }, introduced.len()));
    o.set("summary", JsonValue::str(said.join("; ")));
    o.set("fixed", JsonValue::Array(fixed));
    o.set("introduced", JsonValue::Array(introduced));
    o.set("kept", JsonValue::int(kept as i64));
    if !notes.is_empty() {
        o.set("notes", JsonValue::Array(notes));
    }
    JsonValue::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 22_050.0;
    const SUB: usize = 2205;

    fn sine(f: f64, amp: f64, phase: f64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (amp * (2.0 * std::f64::consts::PI * f * i as f64 / SR + phase).sin()) as f32).collect()
    }

    /// Deterministic noise in -1..1 (splitmix64: two seeds give unrelated channels).
    fn noise(seed: u64, n: usize) -> Vec<f32> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (0..n)
            .map(|_| {
                x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = x;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                ((z >> 11) as f64 / (1u64 << 52) as f64 - 1.0) as f32
            })
            .collect()
    }

    fn spectrum_of(l: &[f32], r: &[f32]) -> Spectrum {
        let mut s = Spectrum::new(SR, SUB);
        s.push(l, r);
        s.finish();
        s
    }

    fn stats(l: &[f32], r: &[f32]) -> MixStats {
        let s = spectrum_of(l, r);
        let (ll, rr, lr) = l.iter().zip(r).fold((0.0, 0.0, 0.0), |(a, b, c), (x, y)| (a + f64::from(*x) * f64::from(*x), b + f64::from(*y) * f64::from(*y), c + f64::from(*x) * f64::from(*y)));
        // the frames well inside the signal
        let subs = l.len() / SUB;
        MixStats::of(band_means(s.frames_in(1..subs - 1).iter()), ll, rr, lr)
    }

    #[test]
    fn a_full_scale_sine_is_minus_three_in_its_band_and_its_centroid_is_its_frequency() {
        let x = sine(1000.0, 1.0, 0.0, SR as usize * 2);
        let m = stats(&x, &x);
        let mid = m.bands[3].unwrap();
        assert!((mid + 3.0).abs() < 0.1, "{m:?}");
        assert!(m.bands[0].map_or(true, |b| b < -60.0) && m.bands[5].map_or(true, |b| b < -60.0), "{m:?}");
        assert!((m.centroid.unwrap() - 1000.0).abs() < 15.0, "{m:?}");
        assert_eq!(m.width, Some(0.0));
        assert!((m.correlation.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_50_hz_sine_is_sub_and_a_100_hz_one_low() {
        let x = sine(45.0, 0.5, 0.0, SR as usize * 2);
        let m = stats(&x, &x);
        assert!(m.bands[0].unwrap() > m.bands[1].unwrap() + 10.0, "{m:?}");
        let y = sine(120.0, 0.5, 0.0, SR as usize * 2);
        let m = stats(&y, &y);
        assert!(m.bands[1].unwrap() > m.bands[0].unwrap() + 10.0 && m.bands[1].unwrap() > m.bands[2].unwrap() + 10.0, "{m:?}");
        assert!(m.low_share() > 0.99);
    }

    #[test]
    fn stereo_width_and_correlation() {
        let x = noise(1, SR as usize);
        let neg: Vec<f32> = x.iter().map(|v| -v).collect();
        let m = stats(&x, &neg);
        assert!((m.correlation.unwrap() + 1.0).abs() < 1e-9 && (m.width.unwrap() - 1.0).abs() < 1e-9, "{m:?}");
        let y = noise(2, SR as usize);
        let m = stats(&x, &y);
        assert!(m.correlation.unwrap().abs() < 0.05 && (m.width.unwrap() - 0.5).abs() < 0.05, "{m:?}");
        // the bands of a wide signal are the channels' mean, not the mono sum's
        let a = stats(&x, &x);
        let b = stats(&x, &y);
        for j in 2..6 {
            assert!((a.bands[j].unwrap() - b.bands[j].unwrap()).abs() < 1.0, "{a:?} {b:?}");
        }
    }

    #[test]
    fn silence_has_no_bands_and_no_transform() {
        let mut s = Spectrum::new(SR, SUB);
        s.push_zeros(SR as usize * 3);
        s.finish();
        assert!(!s.frames.is_empty());
        assert!(s.frames.iter().all(|f| f.total == 0.0));
        assert!(band_means(s.frames.iter()).unwrap().2.is_none());
        // frames line up whether the silence is pushed or skipped
        let mut a = Spectrum::new(SR, SUB);
        a.push(&vec![0.0; 10_000], &vec![0.0; 10_000]);
        let mut b = Spectrum::new(SR, SUB);
        b.push_zeros(10_000);
        assert_eq!(a.frames.iter().map(|f| f.sub).collect::<Vec<_>>(), b.frames.iter().map(|f| f.sub).collect::<Vec<_>>());
    }

    #[test]
    fn true_peak_finds_the_peak_between_samples() {
        // a quarter of the rate, 45 degrees off: every sample is at 0.707, the wave reaches 1
        let x = sine(SR / 4.0, 1.0, std::f64::consts::FRAC_PI_4, SR as usize);
        let mut t = TruePeak::new(SUB);
        t.push(&x, &x);
        t.finish();
        let tp = t.over(0..usize::MAX).unwrap();
        let sp = 20.0 * f64::from(x.iter().fold(0.0f32, |a, b| a.max(b.abs()))).log10();
        assert!((sp + 3.0).abs() < 0.1, "sample peak {sp}");
        assert!(tp > -0.6 && tp < 0.3, "true peak {tp}");
        // a slow sine: the true peak is the sample peak
        let y = sine(100.0, 0.5, 0.0, SR as usize);
        let mut t = TruePeak::new(SUB);
        t.push(&y, &y);
        t.finish();
        assert!((t.over(0..usize::MAX).unwrap() + 6.02).abs() < 0.05);
    }

    #[test]
    fn loudness_range_of_two_levels() {
        // 20 s at -20 LUFS then 20 s at -30 (K energies a sub-block)
        let e = |l: f64| 10f64.powf((l + 0.691) / 10.0);
        let k: Vec<f64> = (0..400).map(|i| if i < 200 { e(-20.0) } else { e(-30.0) }).collect();
        let st = short_term(&k, 30);
        assert!((st.iter().copied().fold(f64::MIN, f64::max) + 20.0).abs() < 1e-9);
        let lra = loudness_range(&st).unwrap();
        assert!((lra - 10.0).abs() < 0.5, "{lra}");
        // steady: no range
        let k = vec![e(-14.0); 100];
        assert!(loudness_range(&short_term(&k, 30)).unwrap().abs() < 1e-9);
        assert!(loudness_range(&[]).is_none());
    }

    #[test]
    fn targets_parse() {
        assert_eq!(Targets::parse("streaming").unwrap(), Targets::default());
        assert_eq!(Targets::parse("club").unwrap().lufs, -8.0);
        let t = Targets::parse(" -12/-1.5 ").unwrap();
        assert_eq!((t.name.as_str(), t.lufs, t.true_peak), ("custom", -12.0, -1.5));
        assert!(Targets::parse("loud").unwrap_err().contains("streaming"));
        assert!(Targets::parse("-50/-1").is_err() && Targets::parse("-12/3").is_err());
        let j = Targets::default().json(Some(-15.25), Some(-0.5));
        assert_eq!(j.canonical_js(), r#"{"lufs":-14,"lufs_delta":-1.3,"met":false,"name":"streaming","true_peak_db":-1,"true_peak_delta":0.5}"#);
    }

    #[test]
    fn target_warnings_say_the_level_to_write() {
        let t = Targets::default();
        // quiet with room: the master level that gets there
        let w = target_warnings(&t, Some(-20.0), Some(-8.0), 0.5);
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].0, "loudness:low");
        assert!(w[0].1.contains("`master: level 1`"), "{w:?}");
        // quiet with no room: drive first
        let w = target_warnings(&t, Some(-20.0), Some(-2.0), 1.0);
        assert!(w[0].1.contains("master: drive") && w[0].1.contains("no limiter"), "{w:?}");
        // over the ceiling, and loud: one level meets both, and both warnings name it
        let w = target_warnings(&t, Some(-9.0), Some(0.5), 1.0);
        let ids: Vec<&str> = w.iter().map(|(i, _)| i.as_str()).collect();
        assert_eq!(ids, ["true_peak", "loudness:high"], "{w:?}");
        assert!(w[0].1.contains("take 5.0 dB off (`master: level 0.56`)") && w[1].1.contains("`master: level 0.56` fixes both"), "{w:?}");
        // over the ceiling by a little: the level that meets it
        let w = target_warnings(&t, Some(-13.5), Some(0.0), 1.0);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].1.contains("take 1.0 dB off (`master: level 0.89`)"), "{w:?}");
        // peaky and loud (a clipping clap): the level the ceiling needs would leave it far under the target, so the peaks first
        let w = target_warnings(&t, Some(-12.1), Some(5.5), 1.0);
        assert!(w[0].1.contains("round the peaks first") && w[0].1.contains("`master: level 0.47`") && w[0].1.contains("4.6 LU under"), "{w:?}");
        assert!(w[1].1.contains("once the peaks are rounded, `master: level 0.8`"), "{w:?}");
        // on target: nothing
        assert!(target_warnings(&t, Some(-14.3), Some(-1.5), 1.0).is_empty());
    }

    /// Two parts with the same steady band clash; the same parts taking turns (a duck) do not.
    #[test]
    fn clashes_are_found_bar_by_bar_and_a_duck_clears_them() {
        let bar = SUB * 10; // 1 s bars
        let n = bar * 6;
        let bass = sine(45.0, 0.4, 0.0, n);
        // a kick: a 50 Hz burst on every half second, 150 ms long
        let kick_at = |i: usize| (i % (bar / 2)) < (SR * 0.15) as usize;
        let kick: Vec<f32> = (0..n).map(|i| if kick_at(i) { (0.5 * (2.0 * std::f64::consts::PI * 50.0 * i as f64 / SR).sin()) as f32 } else { 0.0 }).collect();
        let ducked: Vec<f32> = bass.iter().enumerate().map(|(i, v)| if kick_at(i) || kick_at(i + 2000) { v * 0.1 } else { *v }).collect();
        let mix = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x + y).collect::<Vec<f32>>();
        let bars: Vec<(u32, Range<usize>)> = (0..6u32).map(|b| (b, (b as usize * 10)..(b as usize * 10 + 10))).collect();
        let run = |b: &[f32]| {
            let (sk, sb, sm) = (spectrum_of(&kick, &kick), spectrum_of(b, b), spectrum_of(&mix(&kick, b), &mix(&kick, b)));
            let parts = [PartSpectrum { deck: "A".into(), role: "kick".into(), spec: &sk }, PartSpectrum { deck: "B".into(), role: "bass".into(), spec: &sb }];
            clashes(&parts, &sm, &bars, &|_| Some("drop".into()), 2)
        };
        let c = run(&bass);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!((c[0].band, c[0].bars.clone(), c[0].id()), (0, vec![(1, 6)], "clash:A.kick/B.bass:sub".to_string()), "{c:?}");
        let (id, w) = clash_warning(&c[0], &|_| 1);
        assert_eq!(id, "clash:A.kick/B.bass:sub");
        assert!(w.contains("duck B under A.kick 6db") && w.contains("bars 1-6"), "{w}");
        assert!(run(&ducked).is_empty(), "{:?}", run(&ducked));
    }

    #[test]
    fn bars_in_words() {
        assert_eq!(bars_words(&[(3, 3)]), "bar 3");
        assert_eq!(bars_words(&[(17, 32)]), "bars 17-32");
        assert_eq!(bars_words(&[(1, 4), (9, 12), (17, 17)]), "bars 1-4, 9-12 and 17");
    }

    #[test]
    fn compare_says_what_moved_and_which_warnings_went() {
        let prev = JsonValue::parse(r#"{"cid":"zA","window":{"bars":8,"from_bar":1},"master":{"lufs":-18.2,"true_peak_db":-3.0,"bands":{"sub":-20,"low":-15}},
            "parts":[{"deck":"A","part":"kick","lufs":-20,"bands":{"sub":-22}},{"deck":"B","part":"pad","lufs":-30}],
            "sections":[{"name":"drop","lufs":-17}],"targets":{"lufs_delta":-4.2,"true_peak_delta":-2,"met":false},
            "warnings":["the master clips","A and B clash"],"warning_ids":["clip","clash:A.kick/B.bass:sub"]}"#)
        .unwrap();
        let cur = JsonValue::parse(r#"{"cid":"zB","window":{"bars":8,"from_bar":1},"master":{"lufs":-14.5,"true_peak_db":-1.2,"bands":{"sub":-20,"low":-13.5}},
            "parts":[{"deck":"A","part":"kick","lufs":-20,"bands":{"sub":-22}},{"deck":"C","part":"lead","lufs":-16}],
            "sections":[{"name":"drop","lufs":-14}],"targets":{"lufs_delta":-0.5,"true_peak_delta":-0.2,"met":true},
            "warnings":["the master clips: 12","deck C lead buried"],"warning_ids":["clip","buried:C.lead"]}"#)
        .unwrap();
        let c = compare(&prev, &cur);
        assert_eq!(c.get("master").unwrap().canonical_js(), r#"{"bands":{"low":1.5},"lufs":3.7,"true_peak_db":1.8}"#);
        assert_eq!(c.get("parts").unwrap().canonical_js(), "[]", "an unchanged part says nothing");
        assert_eq!(c.get("parts_added").unwrap().canonical_js(), r#"["C.lead"]"#);
        assert_eq!(c.get("parts_removed").unwrap().canonical_js(), r#"["B.pad"]"#);
        assert_eq!(c.get("sections").unwrap().canonical_js(), r#"[{"lufs":3,"name":"drop"}]"#);
        assert_eq!(c.get("fixed").unwrap().canonical_js(), r#"["A and B clash"]"#);
        assert_eq!(c.get("introduced").unwrap().canonical_js(), r#"["deck C lead buried"]"#);
        assert_eq!(c.get("kept").and_then(JsonValue::as_f64), Some(1.0));
        assert_eq!(s(&c, "summary"), "master -18.2 -> -14.5 LUFS (+3.7); true peak -3 -> -1.2 dBTP; 1 warning fixed, 1 introduced, 1 kept");
        assert_eq!(c.get("targets").unwrap().get("after").unwrap().get("met").and_then(JsonValue::as_bool), Some(true));
        assert!(c.get("notes").is_none());
    }
}
