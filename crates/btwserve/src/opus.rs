//! Ogg Opus (RFC 7845), encoded in-process as the PCM arrives: no ffmpeg, no C toolchain.
//!
//! The encoder is libopus 1.3.1 transpiled to Rust (`unsafe-libopus`), the container the pure-Rust `ogg`
//! crate. Settings follow what the Go route asked ffmpeg for (`-c:a libopus -b:a 96k -application audio
//! -page_duration 200000`): 96 kb/s VBR, the music application, 20 ms frames, a page every 200 ms (10 packets).
//! The first audio page goes out after 60 ms (3 packets) so a player has sound to start on at once, and the two
//! header pages are written before any audio, so the response's first bytes leave as soon as the engine is up.
//!
//! Opus runs at 48 kHz only. The engine is asked for 48 kHz directly (the caller's job), so nothing is resampled;
//! `input_rate` is only the informational field of the OpusHead (what ffmpeg wrote too: the rate it was fed).

use std::io::{self, Write};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use unsafe_libopus as op;

/// 20 ms at 48 kHz.
pub const FRAME: usize = 960;
const CHANNELS: usize = 2;
const PACKETS_PER_PAGE: u32 = 10;
const FIRST_PAGE_PACKETS: u32 = 3;
const MAX_PACKET: usize = 4000;

pub struct OggOpus<W: Write> {
    enc: *mut op::OpusEncoder,
    ogg: PacketWriter<'static, W>,
    serial: u32,
    preskip: u64,
    /// Interleaved stereo f32 not yet a full frame.
    pending: Vec<f32>,
    /// Samples (per channel) handed in so far.
    samples_in: u64,
    /// Frames encoded so far.
    frames: u64,
    in_page: u32,
    first_page_done: bool,
    /// The last encoded packet, held back so the final one can carry the end-of-stream flag and the trimmed granule.
    held: Option<(Vec<u8>, u64)>,
    out: Vec<u8>,
}

// The encoder state is owned by one OggOpus and only touched through &mut self.
unsafe impl<W: Write + Send> Send for OggOpus<W> {}

impl<W: Write> OggOpus<W> {
    /// The encoder, with the two header pages already written to `w`.
    pub fn new(w: W, input_rate: u32, bitrate: i32, serial: u32) -> io::Result<OggOpus<W>> {
        let mut err = 0i32;
        let enc = unsafe { op::opus_encoder_create(48_000, CHANNELS as i32, op::OPUS_APPLICATION_AUDIO, &mut err) };
        if enc.is_null() || err != op::OPUS_OK {
            return Err(io::Error::other(format!("opus_encoder_create: {err}")));
        }
        let mut lookahead = 0i32;
        unsafe {
            op::opus_encoder_ctl!(enc, op::OPUS_SET_BITRATE_REQUEST, bitrate);
            op::opus_encoder_ctl!(enc, op::OPUS_SET_VBR_REQUEST, 1i32);
            op::opus_encoder_ctl!(enc, op::OPUS_SET_SIGNAL_REQUEST, op::OPUS_SIGNAL_MUSIC);
            op::opus_encoder_ctl!(enc, op::OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead);
        }
        let preskip = if lookahead > 0 { lookahead as u64 } else { 312 };
        let mut ogg = PacketWriter::new(w);
        // OpusHead: version 1, 2 channels, pre-skip, input rate, gain 0, mapping family 0
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1);
        head.push(CHANNELS as u8);
        head.extend_from_slice(&(preskip as u16).to_le_bytes());
        head.extend_from_slice(&input_rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        ogg.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;
        let vendor = b"btwserve (unsafe-libopus 1.3.1)";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        ogg.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;
        ogg.inner_mut().flush()?;
        Ok(OggOpus { enc, ogg, serial, preskip, pending: Vec::with_capacity(FRAME * CHANNELS * 2), samples_in: 0, frames: 0, in_page: 0, first_page_done: false, held: None, out: vec![0u8; MAX_PACKET] })
    }

    /// Interleaved stereo samples at 48 kHz.
    pub fn push(&mut self, interleaved: &[f32]) -> io::Result<()> {
        self.samples_in += (interleaved.len() / CHANNELS) as u64;
        self.pending.extend_from_slice(interleaved);
        let whole = self.pending.len() / (FRAME * CHANNELS) * (FRAME * CHANNELS);
        let mut at = 0;
        while at < whole {
            let frame: Vec<f32> = self.pending[at..at + FRAME * CHANNELS].to_vec();
            self.encode(&frame)?;
            at += FRAME * CHANNELS;
        }
        self.pending.drain(..whole);
        Ok(())
    }

    fn encode(&mut self, frame: &[f32]) -> io::Result<()> {
        let n = unsafe { op::opus_encode_float(self.enc, frame.as_ptr(), FRAME as i32, self.out.as_mut_ptr(), MAX_PACKET as i32) };
        if n < 0 {
            return Err(io::Error::other(format!("opus_encode_float: {n}")));
        }
        self.frames += 1;
        let packet = self.out[..n as usize].to_vec();
        let granule = self.frames * FRAME as u64;
        if let Some((p, g)) = self.held.take() {
            self.in_page += 1;
            let limit = if self.first_page_done { PACKETS_PER_PAGE } else { FIRST_PAGE_PACKETS };
            let end = if self.in_page >= limit {
                self.in_page = 0;
                self.first_page_done = true;
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            self.ogg.write_packet(p, self.serial, end, g)?;
            if matches!(end, PacketWriteEndInfo::EndPage) {
                self.ogg.inner_mut().flush()?;
            }
        }
        self.held = Some((packet, granule));
        Ok(())
    }

    /// Pad the last frame, encode the encoder's lookahead out, end the stream with the trimmed granule.
    pub fn finish(&mut self) -> io::Result<()> {
        let want = self.samples_in + self.preskip;
        while self.frames * (FRAME as u64) < want {
            let mut frame = std::mem::take(&mut self.pending);
            frame.resize(FRAME * CHANNELS, 0.0);
            self.encode(&frame)?;
        }
        if let Some((p, _)) = self.held.take() {
            self.ogg.write_packet(p, self.serial, PacketWriteEndInfo::EndStream, want)?;
        }
        self.ogg.inner_mut().flush()?;
        Ok(())
    }
}

impl<W: Write> Drop for OggOpus<W> {
    fn drop(&mut self) {
        if !self.enc.is_null() {
            unsafe { op::opus_encoder_destroy(self.enc) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1.01 s of a stereo sine: the stream reads back as Ogg Opus, the head carries the pre-skip and input rate,
    /// and the last page ends the stream on pre-skip + the samples handed in (so players trim the padding).
    #[test]
    fn ogg_opus_round_trip_shape() {
        let n = 48_480usize;
        let pcm: Vec<f32> = (0..n).flat_map(|i| {
            let x = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.3;
            [x, -x]
        }).collect();
        let mut buf = Vec::new();
        let mut enc = OggOpus::new(&mut buf, 44_100, 96_000, 7).unwrap();
        for c in pcm.chunks(4410 * 2) {
            enc.push(c).unwrap();
        }
        enc.finish().unwrap();
        drop(enc);
        let mut r = ogg::reading::PacketReader::new(std::io::Cursor::new(&buf));
        let head = r.read_packet_expected().unwrap();
        assert_eq!(&head.data[..8], b"OpusHead");
        let preskip = u16::from_le_bytes([head.data[10], head.data[11]]) as u64;
        assert_eq!(u32::from_le_bytes(head.data[12..16].try_into().unwrap()), 44_100);
        assert!(preskip > 0);
        assert_eq!(&r.read_packet_expected().unwrap().data[..8], b"OpusTags");
        let (mut packets, mut last) = (0, None);
        while let Some(p) = r.read_packet().unwrap() {
            packets += 1;
            last = Some((p.absgp_page(), p.last_in_stream()));
        }
        let (gp, eos) = last.unwrap();
        assert!(eos);
        assert_eq!(gp, preskip + n as u64);
        assert_eq!(packets as u64, (n as u64 + preskip).div_ceil(FRAME as u64));
    }
}
