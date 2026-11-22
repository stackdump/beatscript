//! The master mix as a byte stream a network player can play while it is being made, with no
//! length known: an endless WAV (RIFF sizes at their maximum, as internet radio's PCM streams
//! do) or a FLAC stream of VERBATIM frames (no compression, so no encoder to port: every
//! frame is its header, the raw 16-bit samples and two CRCs). Stereo, 16-bit.
//!
//! FLAC is the safer bet for a player that wants a container which says it never ends: its
//! STREAMINFO's "total samples 0" means unknown, and each frame stands on its own, so a player
//! that joins late or drops a packet resynchronises on the next frame.

/// Frames per FLAC block (block size code 12: 4096); ~85 ms at 48 kHz.
pub const FLAC_BLOCK: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Wav,
    Flac,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "wav" | "wave" | "pcm" => Some(Format::Wav),
            "flac" => Some(Format::Flac),
            _ => None,
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Flac => "flac",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Format::Wav => "audio/wav",
            Format::Flac => "audio/flac",
        }
    }
}

/// A float sample as 16-bit PCM, clipped.
pub fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// An endless WAV's header: PCM, stereo, 16-bit; the RIFF and data sizes at their maximum.
pub fn wav_header(sample_rate: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&u32::MAX.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&sample_rate.to_le_bytes());
    h.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    h.extend_from_slice(&4u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(u32::MAX - 36).to_le_bytes());
    h
}

/// Interleaved little-endian 16-bit frames.
pub fn wav_frames(left: &[f32], right: &[f32], out: &mut Vec<u8>) {
    for (l, r) in left.iter().zip(right) {
        out.extend_from_slice(&to_i16(*l).to_le_bytes());
        out.extend_from_slice(&to_i16(*r).to_le_bytes());
    }
}

/// CRC-8, polynomial x^8 + x^2 + x + 1 (FLAC's frame-header check).
pub fn crc8(data: &[u8]) -> u8 {
    let mut c = 0u8;
    for &b in data {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            };
        }
    }
    c
}

/// CRC-16, polynomial x^16 + x^15 + x^2 + 1 (FLAC's frame footer).
pub fn crc16(data: &[u8]) -> u16 {
    let mut c = 0u16;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x8005
            } else {
                c << 1
            };
        }
    }
    c
}

/// FLAC's "UTF-8" coding of a frame number (up to 36 bits).
pub fn utf8_number(n: u64, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
        return;
    }
    let bytes = match n {
        0..=0x7FF => 2,
        0x800..=0xFFFF => 3,
        0x1_0000..=0x1F_FFFF => 4,
        0x20_0000..=0x3FF_FFFF => 5,
        0x400_0000..=0x7FFF_FFFF => 6,
        _ => 7,
    };
    let lead: u8 = if bytes == 7 { 0xFE } else { !(0xFFu8 >> bytes) };
    let first_bits = if bytes == 7 { 0 } else { 7 - bytes };
    let shift = 6 * (bytes - 1);
    out.push(lead | ((n >> shift) as u8 & ((1u16 << first_bits) - 1) as u8));
    for k in (0..bytes - 1).rev() {
        out.push(0x80 | ((n >> (6 * k)) as u8 & 0x3F));
    }
}

fn rate_code(sr: u32) -> u8 {
    match sr {
        88_200 => 0b0001,
        176_400 => 0b0010,
        192_000 => 0b0011,
        8_000 => 0b0100,
        16_000 => 0b0101,
        22_050 => 0b0110,
        24_000 => 0b0111,
        32_000 => 0b1000,
        44_100 => 0b1001,
        48_000 => 0b1010,
        96_000 => 0b1011,
        _ => 0b0000, // from STREAMINFO
    }
}

/// A FLAC stream of VERBATIM frames: stereo, 16-bit, fixed blocks of [`FLAC_BLOCK`].
pub struct FlacStream {
    pub sample_rate: u32,
    frame: u64,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl FlacStream {
    pub fn new(sample_rate: u32) -> FlacStream {
        FlacStream {
            sample_rate,
            frame: 0,
            l: Vec::with_capacity(FLAC_BLOCK),
            r: Vec::with_capacity(FLAC_BLOCK),
        }
    }

    /// `fLaC` and a STREAMINFO block (the last metadata block): total samples 0 (unknown), no
    /// frame sizes, no MD5.
    pub fn header(&self) -> Vec<u8> {
        let mut h = b"fLaC".to_vec();
        h.extend_from_slice(&[0x80, 0, 0, 34]);
        h.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
        h.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
        h.extend_from_slice(&[0; 6]); // min / max frame size: unknown
                                      // sample rate (20) | channels - 1 (3) | bits - 1 (5) | total samples (36)
        let packed: u64 = ((self.sample_rate as u64 & 0xF_FFFF) << 44) | (1 << 41) | (15 << 36);
        h.extend_from_slice(&packed.to_be_bytes());
        h.extend_from_slice(&[0; 16]); // MD5: none
        h
    }

    /// Add frames; out gets every whole block they complete.
    pub fn push(&mut self, left: &[f32], right: &[f32], out: &mut Vec<u8>) {
        for (l, r) in left.iter().zip(right) {
            self.l.push(*l);
            self.r.push(*r);
            if self.l.len() == FLAC_BLOCK {
                self.block(out);
            }
        }
    }

    fn block(&mut self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0xFF, 0xF8]);
        out.push((0b1100 << 4) | rate_code(self.sample_rate));
        out.push((0b0001 << 4) | (0b100 << 1)); // left/right, 16-bit
        utf8_number(self.frame, out);
        let c = crc8(&out[start..]);
        out.push(c);
        for ch in [&self.l, &self.r] {
            out.push(0b0000_0010); // VERBATIM, no wasted bits
            for v in ch.iter() {
                out.extend_from_slice(&to_i16(*v).to_be_bytes());
            }
        }
        let c = crc16(&out[start..]);
        out.extend_from_slice(&c.to_be_bytes());
        self.frame += 1;
        self.l.clear();
        self.r.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crcs_match_flacs_check_values() {
        // the standard check value of each CRC over "123456789"
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn frame_numbers_code_as_utf8() {
        let code = |n| {
            let mut v = Vec::new();
            utf8_number(n, &mut v);
            v
        };
        assert_eq!(code(0), [0]);
        assert_eq!(code(0x7F), [0x7F]);
        assert_eq!(code(0x80), [0xC2, 0x80]);
        assert_eq!(code(0x7FF), [0xDF, 0xBF]);
        assert_eq!(code(0x800), [0xE0, 0xA0, 0x80]);
        assert_eq!(code(0x10000), [0xF0, 0x90, 0x80, 0x80]);
        assert_eq!(code(0x7FFF_FFFF).len(), 6);
        assert_eq!(code(0x8000_0000)[0], 0xFE);
    }

    #[test]
    fn wav_header_is_an_endless_stereo_pcm16() {
        let h = wav_header(48_000);
        assert_eq!(h.len(), 44);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([h[24], h[25], h[26], h[27]]), 48_000);
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 2);
        let mut b = Vec::new();
        wav_frames(&[1.5, -1.0], &[0.0, 0.5], &mut b);
        assert_eq!(b, [0xFF, 0x7F, 0, 0, 0x01, 0x80, 0x00, 0x40]);
    }

    #[test]
    fn flac_streaminfo_and_frames_are_laid_out_as_the_format_says() {
        let mut f = FlacStream::new(48_000);
        let h = f.header();
        assert_eq!(h.len(), 4 + 4 + 34);
        // sample rate 48000 in the first 20 bits of byte 18..
        let packed = u64::from_be_bytes(h[18..26].try_into().unwrap());
        assert_eq!(packed >> 44, 48_000);
        assert_eq!((packed >> 41) & 7, 1);
        assert_eq!((packed >> 36) & 31, 15);
        assert_eq!(packed & 0xF_FFFF_FFFF, 0);
        let mut out = Vec::new();
        f.push(
            &vec![0.25; FLAC_BLOCK - 1],
            &vec![-0.25; FLAC_BLOCK - 1],
            &mut out,
        );
        assert!(out.is_empty(), "no frame before a whole block");
        f.push(&[0.25; 2], &[-0.25; 2], &mut out);
        // header 6 bytes (frame 0), 2 subframes of 1 + 2 x 4096, CRC-16
        assert_eq!(out.len(), 6 + 2 * (1 + 2 * FLAC_BLOCK) + 2);
        assert_eq!(&out[..4], &[0xFF, 0xF8, 0xCA, 0x18]);
        assert_eq!(out[5], crc8(&out[..5]));
        let n = out.len();
        assert_eq!(
            u16::from_be_bytes([out[n - 2], out[n - 1]]),
            crc16(&out[..n - 2])
        );
        assert_eq!(&out[7..9], &to_i16(0.25).to_be_bytes());
    }
}
