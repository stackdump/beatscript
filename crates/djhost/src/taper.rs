//! A slider's taper: how a thumb position (0 … 1) maps to the value it sets, and back. Linear is
//! the default; the others give the values that matter more of the travel. Shared by djapp and
//! djandroid (whose `ui/Taper.kt` this mirrors) so a knob sweeps the same on both. Which control
//! gets which taper is decided by sweeping it (djandroid docs/DESIGN.md, "Fine sliders").

/// The short stretch at one end of a [`Taper::Log`] that sets its "off" value.
pub const STOP: f64 = 0.04;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Taper {
    /// `lo … hi` over the travel, evenly.
    Linear { lo: f64, hi: f64 },
    /// `lo … hi` with equal travel per doubling (times, heights, speeds, ratios). `off_at`: a
    /// short stop at the start (−1) or the end (+1) that sets `off` (0: "off", "sustained").
    Log { lo: f64, hi: f64, off_at: i8, off: f64 },
    /// A power curve over `0 … max`: `k` > 1 gives the small values more travel (a mild 1.5).
    Pow { max: f64, k: f64 },
    /// A volume fader's gain 0 … 1 by square law: ½ is −12 dB, 0.3 is −20 dB.
    Fader,
}

impl Taper {
    pub const fn log(lo: f64, hi: f64) -> Taper {
        Taper::Log { lo, hi, off_at: 0, off: 0.0 }
    }

    /// Value → thumb position 0 … 1.
    pub fn to_pos(&self, v: f64) -> f64 {
        match *self {
            Taper::Linear { lo, hi } => ((v - lo) / (hi - lo)).clamp(0.0, 1.0),
            Taper::Log { lo, hi, off_at, off } => {
                let (a, b) = Self::span(off_at);
                if off_at != 0 && v == off {
                    return if off_at < 0 { 0.0 } else { 1.0 };
                }
                a + (b - a) * ((v.clamp(lo, hi) / lo).ln() / (hi / lo).ln())
            }
            Taper::Pow { max, k } => (v.clamp(0.0, max) / max).powf(1.0 / k),
            Taper::Fader => v.clamp(0.0, 1.0).sqrt(),
        }
    }

    /// Thumb position 0 … 1 → value.
    pub fn from_pos(&self, p: f64) -> f64 {
        match *self {
            Taper::Linear { lo, hi } => lo + (hi - lo) * p.clamp(0.0, 1.0),
            Taper::Log { lo, hi, off_at, off } => {
                let (a, b) = Self::span(off_at);
                if (off_at < 0 && p < a) || (off_at > 0 && p > b) {
                    return off;
                }
                lo * (((p.clamp(a, b) - a) / (b - a)) * (hi / lo).ln()).exp()
            }
            Taper::Pow { max, k } => max * p.clamp(0.0, 1.0).powf(k),
            Taper::Fader => {
                let p = p.clamp(0.0, 1.0);
                p * p
            }
        }
    }

    fn span(off_at: i8) -> (f64, f64) {
        (if off_at < 0 { STOP } else { 0.0 }, if off_at > 0 { 1.0 - STOP } else { 1.0 })
    }
}

/// The tapers the Wheel's knobs use, from sweeping each with the voice rendered (djmobile's
/// `wheel::sweep`): speed and gear are ratios; the drop's level rises evenly per doubling of the
/// height, the spring's 6 dB per doubling of the pluck; the decay is heard from 5 to ~300 ms; the
/// drops run into one sound under ~0.15 s and are rhythms to about 1 s.
pub mod wheel {
    use super::Taper;
    pub const SPEED: Taper = Taper::log(0.5, 20.0);
    pub const GEAR: Taper = Taper::log(1.0, 20.0);
    pub const HEIGHT: Taper = Taper::log(0.5, 50.0);
    /// Mass divides the coil's drag: by ratio, like the rest.
    pub const CARRIER: Taper = Taper::log(0.5, 100.0);
    /// "sustained" (0) is the stop at the long end, past 1 s.
    pub const DECAY: Taper = Taper::Log { lo: 5.0, hi: 1000.0, off_at: 1, off: 0.0 };
    /// "off" (0) is the stop at the start.
    pub const INTERVAL: Taper = Taper::Log { lo: 0.05, hi: 3.0, off_at: -1, off: 0.0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tapers_round_trip_and_stop() {
        for t in [wheel::SPEED, wheel::CARRIER, Taper::Pow { max: 2.0, k: 1.5 }, Taper::Fader, Taper::Linear { lo: -20.0, hi: 40.0 }] {
            for i in 1..20 {
                let p = i as f64 / 20.0;
                assert!((t.to_pos(t.from_pos(p)) - p).abs() < 1e-9, "{t:?} {p}");
            }
        }
        // a doubling is the same travel
        let a = wheel::SPEED.to_pos(1.0) - wheel::SPEED.to_pos(0.5);
        let b = wheel::SPEED.to_pos(8.0) - wheel::SPEED.to_pos(4.0);
        assert!((a - b).abs() < 1e-9);
        assert_eq!(wheel::DECAY.from_pos(0.99), 0.0);
        assert_eq!(wheel::DECAY.to_pos(0.0), 1.0);
        assert_eq!(wheel::INTERVAL.from_pos(0.01), 0.0);
        assert!((wheel::INTERVAL.from_pos(STOP) - 0.05).abs() < 1e-9);
        assert!((Taper::Fader.from_pos(0.5) - 0.25).abs() < 1e-12);
    }
}
