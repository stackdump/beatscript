//! Key lock (docs/KEY.md, "Key lock"): play audio at another tempo without moving its pitch.
//!
//! A plain tempo change on a sample deck is varispeed: 5 % faster is 5 % sharp (+0.84
//! semitones). Key lock replaces the deck's audio with a copy time-stretched to the new tempo
//! (djrust's WSOLA, `stretch::time_fit`: Hann frames laid at a fixed hop, each read near where
//! the tempo says and nudged to the offset whose waveform best continues the frame before), so
//! the deck plays it at speed 1 and the pitch is the file's own. The copy is rendered from the
//! *original* every time (stretching a stretch would pile up artefacts).
//!
//! Generated (net) decks need none of this: their notes are synthesised at the tempo asked, so
//! their pitch never moves with it ([`inherent`]).
//!
//! This module is the arithmetic and the render; when to re-render and how the deck swaps is the
//! host's (`djapp`'s key-lock service).

use alloc::vec::Vec;

use crate::deck::beat_grid::BeatGrid;

/// Below this many semitones of drift the deck is left as it is (the stretch is not worth its
/// artefacts): 0.1 semitones, about 0.6 % of tempo.
pub const TOLERANCE_SEMITONES: f64 = 0.1;

/// The widest stretch rendered: 0.5x to 2x.
pub const MAX_RATIO: f64 = 2.0;

/// The pitch a plain (varispeed) tempo change by `rate` moves audio by, in semitones.
pub fn varispeed_semitones(rate: f64) -> f64 {
    if !(rate > 0.0) {
        return 0.0;
    }
    12.0 * libm::log2(rate)
}

/// The rate `to_bpm / from_bpm`, None when either is not a tempo or the ratio is past
/// [`MAX_RATIO`] either way.
pub fn ratio(from_bpm: f64, to_bpm: f64) -> Option<f64> {
    if !(from_bpm > 0.0 && to_bpm > 0.0) {
        return None;
    }
    let r = to_bpm / from_bpm;
    (r <= MAX_RATIO && r >= 1.0 / MAX_RATIO).then_some(r)
}

/// Does a deck locked at `rendered` (the rate its current copy was stretched for) need another
/// render to play at `rate`?
pub fn needs_render(rendered: f64, rate: f64) -> bool {
    (varispeed_semitones(rate) - varispeed_semitones(rendered)).abs() > TOLERANCE_SEMITONES
}

/// Net decks synthesise their notes at the tempo asked: their pitch never moves with tempo.
pub fn inherent(is_net_deck: bool) -> bool {
    is_net_deck
}

/// `chans` (equal-length channels) time-stretched to play at `rate` times the speed with the
/// same pitch: `len / rate` frames. Rate 1 (or not a rate) answers a copy.
pub fn stretch(chans: &[&[f32]], rate: f64) -> Vec<Vec<f32>> {
    let n = chans.first().map_or(0, |c| c.len());
    if n == 0 || !(rate > 0.0) || (rate - 1.0).abs() < 1e-9 {
        return chans.iter().map(|c| c.to_vec()).collect();
    }
    let out = libm::round(n as f64 / rate) as usize;
    crate::stretch::time_fit(chans, out.max(1))
}

/// A beat grid for the stretched copy: `rate` times the tempo, the first beat `rate` times
/// earlier.
pub fn grid_at(grid: &BeatGrid, rate: f64) -> BeatGrid {
    BeatGrid::new(grid.bpm * rate, grid.offset / rate, grid.confidence)
}

/// Transpose `chans` by `semitones`, the length kept (a mashup part moved into key). `circular`:
/// it is a loop, so the wrap is seamless.
pub fn transpose(chans: &[&[f32]], semitones: i32, circular: bool) -> Vec<Vec<f32>> {
    crate::stretch::pitch_shift(chans, semitones, circular)
}
