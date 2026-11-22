//! `/master.djl` (docs/NET-PROTOCOL.md §8.3): the master as 16-bit PCM in chunks, each with a
//! 64-byte header dating its samples on the sender's frame count, its clock and its beat grid.
//! Pure.

/// The header's size.
pub const HEADER: usize = 64;
pub const MAGIC: &[u8; 4] = b"DJL1";
pub const MIME: &str = "application/x-djnet-pcm";
/// The most frames a chunk carries.
pub const MAX_FRAMES: usize = 4096;

pub const FLAG_CLOCK: u8 = 1;
pub const FLAG_PLAYING: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    pub seq: u32,
    /// The sender's render frame of the first sample.
    pub frame: i64,
    pub n: u32,
    pub rate: u32,
    /// The sender's `mono_ns` when frame `frame + n` was rendered.
    pub t: i64,
    pub flags: u8,
    /// The sender's master beat at `frame` (valid with [`FLAG_CLOCK`]).
    pub beat: f64,
    pub bpm: f64,
    pub channels: u32,
}

impl Chunk {
    pub fn has_clock(&self) -> bool {
        self.flags & FLAG_CLOCK != 0
    }
    pub fn playing(&self) -> bool {
        self.flags & FLAG_PLAYING != 0
    }
    /// The beat at sender frame `f`.
    pub fn beat_at(&self, f: i64) -> f64 {
        self.beat + (f - self.frame) as f64 * self.bpm / 60.0 / self.rate.max(1) as f64
    }
    /// When the sender rendered frame `f` (its clock).
    pub fn time_of(&self, f: i64) -> i64 {
        self.t - ((self.frame + self.n as i64 - f) as f64 * 1e9 / self.rate.max(1) as f64) as i64
    }
    pub fn end(&self) -> i64 {
        self.frame + self.n as i64
    }
    /// The PCM's size after the header.
    pub fn body_len(&self) -> usize {
        self.n as usize * self.channels as usize * 2
    }

    pub fn header(&self) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[0..4].copy_from_slice(MAGIC);
        h[4..8].copy_from_slice(&self.seq.to_le_bytes());
        h[8..16].copy_from_slice(&self.frame.to_le_bytes());
        h[16..20].copy_from_slice(&self.n.to_le_bytes());
        h[20..24].copy_from_slice(&self.rate.to_le_bytes());
        h[24..32].copy_from_slice(&self.t.to_le_bytes());
        h[32] = self.flags;
        h[40..48].copy_from_slice(&self.beat.to_le_bytes());
        h[48..56].copy_from_slice(&self.bpm.to_le_bytes());
        h[56..60].copy_from_slice(&self.channels.to_le_bytes());
        h
    }

    pub fn parse(h: &[u8]) -> Option<Chunk> {
        if h.len() < HEADER || &h[0..4] != MAGIC {
            return None;
        }
        let u32_at = |k: usize| u32::from_le_bytes(h[k..k + 4].try_into().unwrap());
        let i64_at = |k: usize| i64::from_le_bytes(h[k..k + 8].try_into().unwrap());
        let f64_at = |k: usize| f64::from_le_bytes(h[k..k + 8].try_into().unwrap());
        let c = Chunk {
            seq: u32_at(4),
            frame: i64_at(8),
            n: u32_at(16),
            rate: u32_at(20),
            t: i64_at(24),
            flags: h[32],
            beat: f64_at(40),
            bpm: f64_at(48),
            channels: u32_at(56),
        };
        let sane = c.n as usize <= MAX_FRAMES * 4
            && (1..=8).contains(&c.channels)
            && (8_000..=384_000).contains(&c.rate)
            && c.beat.is_finite()
            && c.bpm.is_finite();
        sane.then_some(c)
    }
}

/// Header + interleaved 16-bit stereo for `l` / `r`, appended to `out`.
pub fn encode(c: &Chunk, l: &[f32], r: &[f32], out: &mut Vec<u8>) {
    out.extend_from_slice(&c.header());
    for (a, b) in l.iter().zip(r) {
        out.extend_from_slice(&to_i16(*a).to_le_bytes());
        out.extend_from_slice(&to_i16(*b).to_le_bytes());
    }
}

pub fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// The body as mono (the channels averaged), appended to `out`.
pub fn decode_mono(c: &Chunk, body: &[u8], out: &mut std::collections::VecDeque<f32>) {
    let ch = c.channels.max(1) as usize;
    for frame in body.chunks_exact(2 * ch) {
        let mut sum = 0f32;
        for s in frame.chunks_exact(2) {
            sum += i16::from_le_bytes([s[0], s[1]]) as f32 / 32767.0;
        }
        out.push_back(sum / ch as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_round_trips_and_dates_its_frames() {
        let c = Chunk {
            seq: 3,
            frame: 48_000,
            n: 480,
            rate: 48_000,
            t: 2_010_000_000,
            flags: FLAG_CLOCK | FLAG_PLAYING,
            beat: 8.0,
            bpm: 120.0,
            channels: 2,
        };
        let mut out = Vec::new();
        encode(&c, &[0.5; 480], &[-0.5; 480], &mut out);
        assert_eq!(out.len(), HEADER + c.body_len());
        let p = Chunk::parse(&out).unwrap();
        assert_eq!(p, c);
        assert!((p.beat_at(48_480) - 8.02).abs() < 1e-9);
        assert_eq!(p.time_of(48_000), 2_000_000_000);
        let mut m = std::collections::VecDeque::new();
        decode_mono(&p, &out[HEADER..], &mut m);
        assert_eq!(m.len(), 480);
        assert!(m[0].abs() < 1e-4);
        assert!(Chunk::parse(b"nope").is_none());
    }
}
