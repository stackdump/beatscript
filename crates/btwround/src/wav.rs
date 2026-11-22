//! Minimal WAV reader/writer: 16-bit PCM or 32-bit float, stereo or mono, to interleaved stereo i16.
//! Tolerates a streaming header whose data length is bogus (btwserve streams WAV before it knows the size).

pub const SAMPLE_RATE: u32 = 44100;

/// Interleaved stereo s16 frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pcm {
    pub sr: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.sr as f64
    }
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn parse(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut fmt, mut ch, mut sr, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let declared = u32le(bytes, i + 4) as usize;
        let body = i + 8;
        if id == b"fmt " {
            fmt = u16le(bytes, body);
            ch = u16le(bytes, body + 2);
            sr = u32le(bytes, body + 4);
            bits = u16le(bytes, body + 14);
        } else if id == b"data" {
            let end = (body + declared).min(bytes.len());
            let data = &bytes[body..end];
            return decode(data, fmt, ch, sr, bits);
        }
        i = body + declared + (declared & 1);
    }
    Err("no data chunk".into())
}

fn decode(data: &[u8], fmt: u16, ch: u16, sr: u32, bits: u16) -> Result<Pcm, String> {
    if ch != 1 && ch != 2 {
        return Err(format!("{ch} channels: only mono and stereo"));
    }
    let bytes_per = (bits / 8) as usize;
    let frame = bytes_per * ch as usize;
    if frame == 0 {
        return Err("zero-size sample".into());
    }
    let n = data.len() / frame;
    let mut out = Vec::with_capacity(n * 2);
    for f in 0..n {
        let mut v = [0i16; 2];
        for c in 0..ch as usize {
            let o = f * frame + c * bytes_per;
            v[c] = match (fmt, bits) {
                (1, 16) | (0xFFFE, 16) => i16::from_le_bytes([data[o], data[o + 1]]),
                (3, 32) | (0xFFFE, 32) => {
                    let x = f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
                    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
                }
                _ => return Err(format!("unsupported WAV format {fmt} / {bits} bits")),
            };
        }
        if ch == 1 {
            v[1] = v[0];
        }
        out.push(v[0]);
        out.push(v[1]);
    }
    Ok(Pcm { sr, samples: out })
}

pub fn read(path: &std::path::Path) -> Result<Pcm, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&bytes)
}

pub fn encode(p: &Pcm) -> Vec<u8> {
    let data_len = (p.samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&p.sr.to_le_bytes());
    b.extend_from_slice(&(p.sr * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in &p.samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_bogus_length() {
        let p = Pcm { sr: 44100, samples: vec![1, -2, 300, -400, 32767, -32768] };
        let mut bytes = encode(&p);
        assert_eq!(parse(&bytes).unwrap(), p);
        // a streaming header: the data length claims more than the file holds
        bytes[40..44].copy_from_slice(&0x3fff_ffffu32.to_le_bytes());
        assert_eq!(parse(&bytes).unwrap(), p);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"nope").is_err());
    }
}
