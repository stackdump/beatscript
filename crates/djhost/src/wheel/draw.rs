//! The Wheel's drawing and its controls' arithmetic, shared by every app that draws the wheel:
//! model metres to the screen ([`Fit`]), a place on the path and its tangent ([`WheelLayout::at`]),
//! the place nearest a pointer ([`WheelLayout::nearest`]), a drag's distance the short way round
//! ([`WheelLayout::along`]), the magnets moved on between polls on the voice's clock
//! ([`MagnetTracker`]), the tempo ladder's labels and places, and the captions.
//!
//! Ported from djandroid's `ui/WheelPage.kt` and `MagnetTracker.kt` (0.3.27), so the phone and
//! the desktop draw and play the same wheel.

use super::{WheelLayout, WheelMotion};

/// A point on the path and its unit tangent (model metres, y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct At {
    pub x: f64,
    pub y: f64,
    pub tx: f64,
    pub ty: f64,
}

/// Model metres → screen units: the path's box fitted inside `w` × `h` with `margin` all round.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f64,
    ox: f64,
    oy: f64,
}

impl Fit {
    pub fn new(l: &WheelLayout, w: f64, h: f64, margin: f64) -> Fit {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut grow = |x: f64, y: f64| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        };
        for s in &l.shapes {
            for (x, y) in s.xs.iter().zip(&s.ys) {
                grow(*x, *y);
            }
        }
        for p in l.pivot.iter().chain(l.anchor.iter()) {
            if p.len() >= 2 {
                grow(p[0], p[1]);
            }
        }
        if x0 > x1 {
            (x0, y0, x1, y1) = (0.0, 0.0, 0.0, 0.0);
        }
        let bw = (x1 - x0).max(1e-6);
        let bh = (y1 - y0).max(1e-6);
        // a straight path has no extent across it: the other dimension sets the scale
        let scale = ((w - 2.0 * margin) / bw).min((h - 2.0 * margin) / bh).max(1e-9);
        Fit { scale, ox: w / 2.0 - scale * (x0 + x1) / 2.0, oy: h / 2.0 - scale * (y0 + y1) / 2.0 }
    }

    /// Model → screen (relative to the box's top left).
    pub fn px(&self, x: f64, y: f64) -> (f64, f64) {
        (self.ox + self.scale * x, self.oy + self.scale * y)
    }

    /// Screen → model.
    pub fn model(&self, sx: f64, sy: f64) -> (f64, f64) {
        ((sx - self.ox) / self.scale, (sy - self.oy) / self.scale)
    }
}

impl WheelLayout {
    /// The segment holding `z` (a periodic path's z wraps); `seg` when it is a valid hint.
    pub fn segment_of(&self, z: f64, seg: Option<usize>) -> usize {
        if let Some(s) = seg.filter(|s| *s < self.shapes.len()) {
            return s;
        }
        if self.periodic {
            return 0;
        }
        let mut best = (0, f64::MAX);
        for (i, s) in self.shapes.iter().enumerate() {
            let d = if z < s.start_m {
                s.start_m - z
            } else if z > s.start_m + s.length_m {
                z - s.start_m - s.length_m
            } else {
                0.0
            };
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    /// Where `z` is on the drawing (extrapolated past an open segment's ends).
    pub fn at(&self, z_in: f64, seg: Option<usize>) -> At {
        let Some(s) = self.shapes.get(self.segment_of(z_in, seg)) else {
            return At { x: 0.0, y: 0.0, tx: 1.0, ty: 0.0 };
        };
        let (xs, ys, zs) = (&s.xs, &s.ys, &s.zs);
        let n = zs.len().min(xs.len()).min(ys.len());
        if n < 2 {
            return At { x: xs.first().copied().unwrap_or(0.0), y: ys.first().copied().unwrap_or(0.0), tx: 1.0, ty: 0.0 };
        }
        let mut z = z_in;
        if s.closed && s.length_m > 0.0 {
            z = s.start_m + ((z - s.start_m) % s.length_m + s.length_m) % s.length_m;
        }
        let unit = |a: usize, b: usize| {
            let d = (xs[b] - xs[a]).hypot(ys[b] - ys[a]);
            let d = if d > 0.0 { d } else { 1.0 };
            ((xs[b] - xs[a]) / d, (ys[b] - ys[a]) / d)
        };
        if z <= zs[0] || z >= zs[n - 1] {
            let (a, b, base, over) = if z <= zs[0] { (0, 1, 0, z - zs[0]) } else { (n - 2, n - 1, n - 1, z - zs[n - 1]) };
            let (tx, ty) = unit(a, b);
            return At { x: xs[base] + tx * over, y: ys[base] + ty * over, tx, ty };
        }
        let (mut lo, mut hi) = (0, n - 1);
        while hi - lo > 1 {
            let m = (lo + hi) / 2;
            if zs[m] <= z {
                lo = m
            } else {
                hi = m
            }
        }
        let dz = zs[hi] - zs[lo];
        let w = (z - zs[lo]) / if dz > 0.0 { dz } else { 1.0 };
        let (tx, ty) = unit(lo, hi);
        At { x: xs[lo] + (xs[hi] - xs[lo]) * w, y: ys[lo] + (ys[hi] - ys[lo]) * w, tx, ty }
    }

    /// The nearest place on the path to model point (x, y): its z and segment.
    pub fn nearest(&self, x: f64, y: f64) -> (f64, usize) {
        let (mut bz, mut bs, mut bd) = (0.0, 0, f64::MAX);
        for (si, s) in self.shapes.iter().enumerate() {
            let n = s.xs.len().min(s.ys.len()).min(s.zs.len());
            for i in 1..n {
                let (ax, ay) = (s.xs[i - 1], s.ys[i - 1]);
                let (dx, dy) = (s.xs[i] - ax, s.ys[i] - ay);
                let l2 = dx * dx + dy * dy;
                let l2 = if l2 > 0.0 { l2 } else { 1e-18 };
                let t = (((x - ax) * dx + (y - ay) * dy) / l2).clamp(0.0, 1.0);
                let d = (x - (ax + dx * t)).hypot(y - (ay + dy * t));
                if d < bd {
                    bd = d;
                    bs = si;
                    bz = s.zs[i - 1] + (s.zs[i] - s.zs[i - 1]) * t;
                }
            }
        }
        (bz, bs)
    }

    /// How far a pointer went along the path from `from` to `to`, the short way round a
    /// periodic one.
    pub fn along(&self, from: f64, to: f64) -> f64 {
        let mut d = to - from;
        if self.periodic {
            let c = self.extent_m;
            if d > c / 2.0 {
                d -= c
            } else if d < -c / 2.0 {
                d += c
            }
        }
        d
    }

    /// The drawing's aspect (width over height): tall for tubes, racks and arcs, wide for rails.
    pub fn aspect(&self) -> f64 {
        match self.path.as_str() {
            "tube" | "rack" | "arc" => 0.9,
            "rail" | "incline" => 2.0,
            _ => 1.0,
        }
    }

    /// One line on how to play it.
    pub fn how_to_play(&self) -> String {
        match self.law.as_str() {
            "drive" => format!("Drag the {} round · flick it to keep it spinning", self.path),
            "motor" => "The motor turns it · drag to push it along".into(),
            "bow" => "The bow keeps it moving · drag to bow it yourself".into(),
            "gravity" if self.shapes.len() > 1 => "Click a tube to drop a magnet down it".into(),
            "gravity" => "Click to drop a magnet".into(),
            "pendulum" => "Click to pull the pendulum back and let it swing".into(),
            "spring" => "Click to pluck the spring".into(),
            "rail" => "Slide the magnet along the rail".into(),
            _ => String::new(),
        }
    }
}

/// How an instrument is played, for its tile in the picker.
pub fn law_name(law: &str, path: &str) -> String {
    match law {
        "drive" => format!("hand-spun {path}"),
        "motor" => format!("motor {path}"),
        "bow" => format!("bowed {path}"),
        "gravity" if path == "incline" => "rolled down a slope".into(),
        "gravity" => "dropped down tubes".into(),
        "pendulum" => "a swinging pendulum".into(),
        "spring" => "a magnet on a spring".into(),
        "rail" => "slid by hand".into(),
        other => other.to_string(),
    }
}

/// Rung place `p` (×1 at 0, ×(p+1) above, 1/(1−p) below) as a ratio to the deck.
pub fn rung_ratio(p: f64) -> f64 {
    if p >= 0.0 { p + 1.0 } else { 1.0 / (1.0 - p) }
}

/// The rung nearest ratio `r`: ×n is rung n−1, 1/n is rung 1−n.
pub fn ratio_rung(r: f64) -> i64 {
    if r >= 1.0 { r.round() as i64 - 1 } else { 1 - (1.0 / r).round() as i64 }
}

/// A place on the fine sync slider (log2 of the ratio), caught by a doubling within 0.15.
pub fn sync_detent(x: f64) -> f64 {
    let k = x.round();
    if (x - k).abs() < 0.15 { k } else { x }
}

/// The fine sync slider's span (log2 of the ratio: ×½ … ×16) and its detents (the doublings).
pub const SYNC_FINE: (f64, f64) = (-1.0, 4.0);

/// Rung `i` of the ladder: ×1 at 0, ×(i+1) above, ½, ⅓, … below.
pub fn rung_label(i: i64) -> String {
    if i >= 0 { rung_label_n((i + 1) as u32, true) } else { rung_label_n((1 - i) as u32, false) }
}

/// A rung of the tempo ladder: ×n, or ½, ⅓, … for the divisors.
pub fn rung_label_n(n: u32, multiple: bool) -> String {
    const FRACTIONS: [char; 9] = ['½', '⅓', '¼', '⅕', '⅙', '⅐', '⅛', '⅑', '⅒'];
    match n {
        // just under the deck's tempo reads as 1/1 otherwise: the same rung
        _ if multiple || n <= 1 => format!("×{}", n.max(1)),
        2..=10 => FRACTIONS[n as usize - 2].to_string(),
        _ => format!("1/{n}"),
    }
}

/// The trigger's pull in words.
pub fn pull_name(p: f64) -> &'static str {
    if p <= 0.02 {
        "open: silent, no drag"
    } else if p >= 0.98 {
        "short: silent, full brake"
    } else if (p - 0.5).abs() < 0.06 {
        "matched: loudest"
    } else if p < 0.5 {
        "light: quiet, little drag"
    } else {
        "heavy: quiet, strong brake"
    }
}

/// A resistance: "470 Ω", "2 kΩ".
pub fn ohms(r: f64) -> String {
    if r >= 1000.0 { format!("{:.0} kΩ", r / 1000.0) } else { format!("{r:.0} Ω") }
}

/// The load stepper's stops (Ω).
pub const LOADS: [f64; 6] = [8.0, 32.0, 100.0, 470.0, 2000.0, 10000.0];

/// The load `d` stops from `ohms` along [`LOADS`].
pub fn step_load(ohms: f64, d: i32) -> f64 {
    let i = LOADS.iter().position(|l| *l >= ohms - 1e-6).unwrap_or(LOADS.len() - 1) as i32;
    LOADS[(i + d).clamp(0, LOADS.len() as i32 - 1) as usize]
}

const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// "E3" → 52 (MIDI); "Bb2" and "C#4" too. None for anything else.
pub fn midi_of(name: &str) -> Option<i32> {
    let s = name.trim();
    let mut c = s.chars();
    let letter = c.next()?.to_ascii_uppercase();
    let base = [0, 2, 4, 5, 7, 9, 11]["CDEFGAB".find(letter)?];
    let rest = c.as_str();
    let (acc, num) = match rest.chars().next()? {
        '#' => (1, &rest[1..]),
        'b' => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let oct: i32 = num.parse().ok()?;
    Some((oct + 1) * 12 + base + acc)
}

/// 52 → "E3".
pub fn name_of(midi: i32) -> String {
    format!("{}{}", NAMES[midi.rem_euclid(12) as usize], midi.div_euclid(12) - 1)
}

/// The root `d` semitones from `root`, kept within C1 … C6 (the Key card's stepper).
pub fn step_root(root: &str, d: i32) -> String {
    midi_of(root).map_or_else(|| root.to_string(), |m| name_of((m + d).clamp(24, 84)))
}

/// The notes as the preset winds them, without cents ("E3", not "E3+12c"): the sequence field's
/// text before anything is typed.
pub fn preset_sequence(l: &WheelLayout) -> String {
    l.notes
        .iter()
        .map(|n| n.name.split(['+', '-']).next().unwrap_or(&n.name).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

// MARK: the magnets between polls

/// One poll of the magnets, with the wall time it was taken at (s, any monotonic base).
#[derive(Debug, Clone)]
struct Sample {
    z: Vec<f64>,
    v: Vec<f64>,
    on: Vec<bool>,
    seg: Vec<usize>,
    voice_s: f64,
    wall_s: f64,
}

/// The magnets at the display's rate: each poll ([`WheelMotion`]) is moved on at its speeds on the
/// voice's clock, mapped onto the wall clock, and a poll that disagrees with where the drawing had
/// them is eased in rather than jumped to (djandroid's `MagnetTracker`).
#[derive(Debug, Clone, Default)]
pub struct MagnetTracker {
    extent: f64,
    cur: Option<Sample>,
    /// voice clock − wall clock (s), smoothed; NaN before the first sample.
    offset: Option<f64>,
    corr: Vec<f64>,
    corr_at: f64,
}

/// How fast the clock offset follows a poll (per poll).
const OFFSET_GAIN: f64 = 0.05;
/// An offset this far off is a new clock (the voice stalled or restarted): take it.
const RESET_S: f64 = 0.08;
const EASE_S: f64 = 0.06;
const SNAP_M: f64 = 0.1;
const STILL_MPS: f64 = 1e-4;
/// How far a sample is moved on at most (a stalled poll doesn't fling the magnets).
const MAX_AHEAD_S: f64 = 0.1;
const MAX_BACK_S: f64 = 0.05;

impl MagnetTracker {
    /// For a path `extent` long when periodic (0: open).
    pub fn new(extent: f64) -> MagnetTracker {
        MagnetTracker { extent, ..Default::default() }
    }

    /// Take a poll made at wall time `wall_s`.
    pub fn feed(&mut self, m: &WheelMotion, wall_s: f64) {
        let s = Sample {
            z: m.magnets.iter().map(|x| x.z).collect(),
            v: m.magnets.iter().map(|x| x.v).collect(),
            on: m.magnets.iter().map(|x| x.on).collect(),
            seg: m.magnets.iter().map(|x| x.segment as usize).collect(),
            voice_s: m.voice_s,
            wall_s,
        };
        let o = s.voice_s - wall_s;
        let before: Option<Vec<f64>> = self.cur.as_ref().map(|p| (0..p.z.len()).map(|k| self.place(k, wall_s)).collect());
        self.offset = Some(match self.offset {
            Some(prev) if (o - prev).abs() <= RESET_S => prev + (o - prev) * OFFSET_GAIN,
            _ => o,
        });
        let mut c = vec![0.0; s.z.len()];
        if let (Some(p), Some(before)) = (self.cur.as_ref(), before) {
            if p.z.len() == s.z.len() {
                for k in 0..c.len() {
                    if !s.on[k] || !p.on[k] || p.seg[k] != s.seg[k] {
                        continue;
                    }
                    let d = self.wrap(before[k] - self.project(&s, k, wall_s));
                    // a real jump (a magnet dropped back in, a hand on the rail) is drawn as one
                    if d.abs() < SNAP_M {
                        c[k] = d;
                    }
                }
            }
        }
        self.cur = Some(s);
        self.corr = c;
        self.corr_at = wall_s;
    }

    /// The magnets to draw at wall time `wall_s`: (z, segment) for each one that is on.
    pub fn magnets(&self, wall_s: f64) -> Vec<(f64, usize)> {
        let Some(s) = &self.cur else { return vec![] };
        (0..s.z.len()).filter(|k| s.on[*k]).map(|k| (self.place(k, wall_s), s.seg[k])).collect()
    }

    /// Nothing will move until the next poll: every magnet still and the last correction eased
    /// out. A drawing can stop asking for frames then.
    pub fn idle(&self, wall_s: f64) -> bool {
        let Some(s) = &self.cur else { return true };
        if s.v.iter().zip(&s.on).any(|(v, on)| *on && v.abs() > STILL_MPS) {
            return false;
        }
        let left = (-(wall_s - self.corr_at) / EASE_S).exp();
        self.corr.iter().all(|c| c.abs() * left < 1e-5)
    }

    fn place(&self, k: usize, wall_s: f64) -> f64 {
        let Some(s) = &self.cur else { return 0.0 };
        let left = (-(wall_s - self.corr_at).max(0.0) / EASE_S).exp();
        self.project(s, k, wall_s) + self.corr.get(k).copied().unwrap_or(0.0) * left
    }

    /// The sample's guess for magnet `k` at wall time: on at its speed, on the voice's clock.
    fn project(&self, s: &Sample, k: usize, wall_s: f64) -> f64 {
        let dt = (wall_s + self.offset.unwrap_or(s.voice_s - s.wall_s) - s.voice_s).clamp(-MAX_BACK_S, MAX_AHEAD_S);
        s.z[k] + s.v[k] * dt
    }

    fn wrap(&self, d: f64) -> f64 {
        if self.extent <= 0.0 {
            return d;
        }
        let mut x = d % self.extent;
        if x > self.extent / 2.0 {
            x -= self.extent
        } else if x < -self.extent / 2.0 {
            x += self.extent
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wheel::WheelMagnet;

    #[test]
    fn the_ladder_reads_and_round_trips() {
        assert_eq!(rung_label(0), "×1");
        assert_eq!(rung_label(3), "×4");
        assert_eq!(rung_label(-1), "½");
        assert_eq!(rung_label(-2), "⅓");
        assert_eq!(rung_label(-11), "1/12");
        assert_eq!(rung_label_n(1, false), "×1");
        for i in -9..12 {
            assert_eq!(ratio_rung(rung_ratio(i as f64)), i);
        }
        assert_eq!(sync_detent(1.9), 2.0);
        assert_eq!(sync_detent(1.5), 1.5);
    }

    #[test]
    fn notes_and_loads_step() {
        assert_eq!(midi_of("E3"), Some(52));
        assert_eq!(midi_of("Bb2"), Some(46));
        assert_eq!(midi_of("C#4"), Some(61));
        assert_eq!(midi_of("H2"), None);
        assert_eq!(name_of(52), "E3");
        assert_eq!(step_root("C1", -1), "C1");
        assert_eq!(step_root("B3", 1), "C4");
        assert_eq!(step_load(100.0, 1), 470.0);
        assert_eq!(step_load(9.0, -1), 8.0);
        assert_eq!(step_load(10000.0, 1), 10000.0);
    }

    #[test]
    fn every_preset_draws_and_a_pointer_finds_the_path() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (w, t) = crate::wheel::Wheel::start(48_000.0, tx, 1024, None, stop).unwrap();
        t.join().unwrap();
        for p in crate::wheel::presets() {
            let l = w.load(&p.id).unwrap();
            let fit = Fit::new(&l, 400.0, 400.0 / l.aspect(), 40.0);
            for s in &l.shapes {
                // a point of the path maps back onto itself
                let mid = s.xs.len() / 2;
                let (sx, sy) = fit.px(s.xs[mid], s.ys[mid]);
                assert!((0.0..=400.0).contains(&sx) && sx.is_finite() && sy.is_finite(), "{}", p.id);
                let (mx, my) = fit.model(sx, sy);
                let (z, _) = l.nearest(mx, my);
                let a = l.at(z, None);
                assert!((a.x - s.xs[mid]).hypot(a.y - s.ys[mid]) < 0.05 * l.extent_m.max(0.1), "{}", p.id);
            }
            assert!(!l.how_to_play().is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn the_tracker_moves_magnets_on_and_eases_a_correction() {
        let m = |z: f64, v: f64, t: f64| WheelMotion {
            magnets: vec![WheelMagnet { z, v, on: true, segment: 0 }],
            voice_s: t,
        };
        let mut tr = MagnetTracker::new(1.0);
        tr.feed(&m(0.0, 1.0, 10.0), 100.0);
        let z = tr.magnets(100.05)[0].0;
        assert!((z - 0.05).abs() < 1e-9, "{z}");
        // the next poll is 1 cm behind where the drawing had it: eased, not jumped
        tr.feed(&m(0.09, 1.0, 10.1), 100.1);
        let z = tr.magnets(100.1)[0].0;
        assert!((z - 0.1).abs() < 1e-6, "{z}");
        assert!(!tr.idle(100.1));
        tr.feed(&m(0.5, 0.0, 10.2), 100.2);
        assert!(tr.idle(102.0));
    }
}

#[cfg(test)]
mod play_tests {
    #[test]
    fn the_first_play_kicks_a_cord_motor() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (w, t) = crate::wheel::Wheel::start(48_000.0, tx, 1024, None, stop).unwrap();
        t.join().unwrap();
        let l = w.load("hoop-looper").unwrap();
        w.set_motor(false, l.speed_mps);
        assert!(!w.playing());
        assert!(w.play_pause());
        assert!(w.layout().unwrap().motor, "motor on");
        assert!(!w.play_pause());
        assert!(w.play_pause());
    }
}

