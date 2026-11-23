//! A QR Code encoder: a function-by-function port of github.com/skip2/go-qrcode
//! v0.0.0-20200617195104-da1b6568686e (encoder.go, qrcode.go, regular_symbol.go, symbol.go,
//! version.go, bitset/, reedsolomon/), which is what the Go server draws its QR codes with.
//!
//! For the same content and recovery level this gives the same module matrix as go-qrcode (the
//! same segment optimisation, version choice, Reed-Solomon blocks and interleaving, and the mask
//! chosen by go-qrcode's own penalty implementation, quirks included), and the same pixels for
//! `image(size)` / `png(size)` (go-qrcode's nearest-module scaling, a negative size meaning pixels
//! per module). The PNG is written as Go's image/png writes a two-colour paletted image (palette
//! [white, black], bit depth 1, colour type 3); only the deflate bytes differ (miniz_oxide, not
//! compress/flate), so the decoded pixels are equal, not the file bytes.
//!
//! The goldens in `qr_golden.txt` come from go-qrcode itself (see the tests).

// Kept in go-qrcode's shape on purpose, so the port reads line for line against the Go.
#![allow(
    clippy::manual_is_multiple_of,
    clippy::explicit_counter_loop,
    clippy::needless_range_loop,
    clippy::if_same_then_else,
    clippy::neg_multiply
)]

/// go-qrcode RecoveryLevel: L 7%, M 15%, Q 25%, H 30%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Low,
    Medium,
    High,
    Highest,
}

// ---------------------------------------------------------------------------------------------
// bitset/bitset.go: an append-only bit array.

#[derive(Clone, Debug, Default)]
struct Bitset {
    bits: Vec<bool>,
}

impl Bitset {
    fn new(v: &[bool]) -> Bitset {
        Bitset { bits: v.to_vec() }
    }

    fn substr(&self, start: usize, end: usize) -> Bitset {
        if start > end || end > self.bits.len() {
            panic!("Out of range start={} end={} numBits={}", start, end, self.bits.len());
        }
        Bitset { bits: self.bits[start..end].to_vec() }
    }

    fn append_bytes(&mut self, data: &[u8]) {
        for &d in data {
            self.append_byte(d, 8);
        }
    }

    fn append_byte(&mut self, value: u8, num_bits: usize) {
        assert!(num_bits <= 8, "numBits {} out of range 0-8", num_bits);
        for i in (0..num_bits).rev() {
            self.bits.push(value & (1 << i) != 0);
        }
    }

    fn append_uint32(&mut self, value: u32, num_bits: usize) {
        assert!(num_bits <= 32, "numBits {} out of range 0-32", num_bits);
        for i in (0..num_bits).rev() {
            self.bits.push(value & (1u32 << i) != 0);
        }
    }

    fn append(&mut self, other: &Bitset) {
        self.bits.extend_from_slice(&other.bits);
    }

    fn append_num_bools(&mut self, num: usize, value: bool) {
        for _ in 0..num {
            self.bits.push(value);
        }
    }

    fn len(&self) -> usize {
        self.bits.len()
    }

    fn at(&self, index: usize) -> bool {
        if index >= self.bits.len() {
            panic!("Index {} out of range", index);
        }
        self.bits[index]
    }

    /// Up to 8 bits starting at index, as a byte (the last, short byte is not left-aligned).
    fn byte_at(&self, index: usize) -> u8 {
        if index >= self.bits.len() {
            panic!("Index {} out of range", index);
        }
        let mut result: u8 = 0;
        let mut i = index;
        while i < index + 8 && i < self.bits.len() {
            result <<= 1;
            if self.at(i) {
                result |= 1;
            }
            i += 1;
        }
        result
    }
}

// ---------------------------------------------------------------------------------------------
// reedsolomon/gf2_8.go: GF(2^8) modulo x^8 + x^4 + x^3 + x^2 + 1.

const GF_EXP_TABLE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        t[i] = x as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    t[255] = 1;
    t
};

/// gfLogTable; log(0) is -1 in Go and never read.
const GF_LOG_TABLE: [i32; 256] = {
    let mut t = [-1i32; 256];
    let mut i = 0;
    while i < 255 {
        t[GF_EXP_TABLE[i] as usize] = i as i32;
        i += 1;
    }
    t
};

fn gf_add(a: u8, b: u8) -> u8 {
    a ^ b
}

fn gf_multiply(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    GF_EXP_TABLE[((GF_LOG_TABLE[a as usize] + GF_LOG_TABLE[b as usize]) % 255) as usize]
}

fn gf_divide(a: u8, b: u8) -> u8 {
    if a == 0 {
        return 0;
    } else if b == 0 {
        panic!("Divide by zero");
    }
    gf_multiply(a, gf_inverse(b))
}

fn gf_inverse(a: u8) -> u8 {
    if a == 0 {
        panic!("No multiplicative inverse of 0");
    }
    GF_EXP_TABLE[(255 - GF_LOG_TABLE[a as usize]) as usize]
}

// ---------------------------------------------------------------------------------------------
// reedsolomon/gf_poly.go: polynomials over GF(2^8), term[i] is the coefficient of x^i.

#[derive(Clone, Debug, Default)]
struct GfPoly {
    term: Vec<u8>,
}

impl GfPoly {
    fn from_data(data: &Bitset) -> GfPoly {
        let mut num_total_bytes = data.len() / 8;
        if data.len() % 8 != 0 {
            num_total_bytes += 1;
        }
        let mut term = vec![0u8; num_total_bytes];
        let mut i = num_total_bytes as isize - 1;
        let mut j = 0;
        while j < data.len() {
            term[i as usize] = data.byte_at(j);
            i -= 1;
            j += 8;
        }
        GfPoly { term }
    }

    fn monomial(term: u8, degree: usize) -> GfPoly {
        if term == 0 {
            return GfPoly::default();
        }
        let mut t = vec![0u8; degree + 1];
        t[degree] = term;
        GfPoly { term: t }
    }

    fn data(&self, num_terms: usize) -> Vec<u8> {
        let mut result = vec![0u8; num_terms];
        let mut i = num_terms - self.term.len();
        for j in (0..self.term.len()).rev() {
            result[i] = self.term[j];
            i += 1;
        }
        result
    }

    fn num_terms(&self) -> usize {
        self.term.len()
    }

    fn normalised(mut self) -> GfPoly {
        while let Some(&0) = self.term.last() {
            self.term.pop();
        }
        self
    }
}

/// gfPolyMultiply: Go adds one monomial at a time; the sum is the same convolution.
fn gf_poly_multiply(a: &GfPoly, b: &GfPoly) -> GfPoly {
    let mut term = vec![0u8; a.num_terms() + b.num_terms()];
    for (i, &x) in a.term.iter().enumerate() {
        for (j, &y) in b.term.iter().enumerate() {
            if x != 0 && y != 0 {
                term[i + j] = gf_add(term[i + j], gf_multiply(x, y));
            }
        }
    }
    GfPoly { term }.normalised()
}

fn gf_poly_add(a: &GfPoly, b: &GfPoly) -> GfPoly {
    let n = a.num_terms().max(b.num_terms());
    let mut term = vec![0u8; n];
    for (i, t) in term.iter_mut().enumerate() {
        *t = match (a.term.get(i), b.term.get(i)) {
            (Some(&x), Some(&y)) => gf_add(x, y),
            (Some(&x), None) => x,
            (None, Some(&y)) => y,
            (None, None) => 0,
        };
    }
    GfPoly { term }.normalised()
}

fn gf_poly_remainder(numerator: &GfPoly, denominator: &GfPoly) -> GfPoly {
    if denominator.term.iter().all(|&t| t == 0) {
        panic!("Remainder by zero");
    }
    let mut remainder = numerator.clone();
    while remainder.num_terms() >= denominator.num_terms() {
        let degree = remainder.num_terms() - denominator.num_terms();
        let coefficient = gf_divide(
            remainder.term[remainder.num_terms() - 1],
            denominator.term[denominator.num_terms() - 1],
        );
        let divisor = gf_poly_multiply(denominator, &GfPoly::monomial(coefficient, degree));
        remainder = gf_poly_add(&remainder, &divisor);
    }
    remainder.normalised()
}

// ---------------------------------------------------------------------------------------------
// reedsolomon/reed_solomon.go

/// reedsolomon.Encode: data followed by its numECBytes error correction bytes.
fn rs_encode(data: &Bitset, num_ec_bytes: usize) -> Bitset {
    let mut ecpoly = GfPoly::from_data(data);
    ecpoly = gf_poly_multiply(&ecpoly, &GfPoly::monomial(1, num_ec_bytes));
    let generator = rs_generator_poly(num_ec_bytes);
    let remainder = gf_poly_remainder(&ecpoly, &generator);
    let mut result = data.clone();
    result.append_bytes(&remainder.data(num_ec_bytes));
    result
}

fn rs_generator_poly(degree: usize) -> GfPoly {
    if degree < 2 {
        panic!("degree < 2");
    }
    let mut generator = GfPoly { term: vec![1] };
    for i in 0..degree {
        let next = GfPoly { term: vec![GF_EXP_TABLE[i], 1] };
        generator = gf_poly_multiply(&generator, &next);
    }
    generator
}

// ---------------------------------------------------------------------------------------------
// encoder.go: data encoding into numeric / alphanumeric / byte segments.

/// dataMode; each is a subset of the next, and the ordering matters (None < Numeric < ...).
type DataMode = u8;
const DATA_MODE_NONE: DataMode = 1;
const DATA_MODE_NUMERIC: DataMode = 2;
const DATA_MODE_ALPHANUMERIC: DataMode = 4;
const DATA_MODE_BYTE: DataMode = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataEncoderType {
    T1To9,
    T10To26,
    T27To40,
}

#[derive(Clone, Debug)]
struct Segment {
    data_mode: DataMode,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct DataEncoder {
    min_version: usize,
    max_version: usize,
    num_numeric_char_count_bits: usize,
    num_alphanumeric_char_count_bits: usize,
    num_byte_char_count_bits: usize,
    data: Vec<u8>,
    actual: Vec<Segment>,
    optimised: Vec<Segment>,
}

// The mode indicators are the same for every encoder type.
const NUMERIC_MODE_INDICATOR: [bool; 4] = [false, false, false, true];
const ALPHANUMERIC_MODE_INDICATOR: [bool; 4] = [false, false, true, false];
const BYTE_MODE_INDICATOR: [bool; 4] = [false, true, false, false];

impl DataEncoder {
    fn new(t: DataEncoderType) -> DataEncoder {
        let (min_version, max_version, n, a, b) = match t {
            DataEncoderType::T1To9 => (1, 9, 10, 9, 8),
            DataEncoderType::T10To26 => (10, 26, 12, 11, 16),
            DataEncoderType::T27To40 => (27, 40, 14, 13, 16),
        };
        DataEncoder {
            min_version,
            max_version,
            num_numeric_char_count_bits: n,
            num_alphanumeric_char_count_bits: a,
            num_byte_char_count_bits: b,
            data: Vec::new(),
            actual: Vec::new(),
            optimised: Vec::new(),
        }
    }

    /// The encoded data, without the terminator bits.
    fn encode(&mut self, data: &[u8]) -> Result<Bitset, String> {
        self.data = data.to_vec();
        self.actual.clear();
        self.optimised.clear();

        if data.is_empty() {
            return Err("no data to encode".to_string());
        }

        let highest_required_mode = self.classify_data_modes();

        self.optimise_data_modes()?;

        // Check if a single segment of the highest required mode would be more efficient.
        let mut optimized_length = 0;
        for s in &self.optimised {
            optimized_length += self.encoded_length(s.data_mode, s.data.len())?;
        }
        let single_byte_segment_length = self.encoded_length(highest_required_mode, self.data.len())?;
        if single_byte_segment_length <= optimized_length {
            self.optimised = vec![Segment { data_mode: highest_required_mode, data: self.data.clone() }];
        }

        let mut encoded = Bitset::default();
        for s in &self.optimised {
            self.encode_data_raw(&s.data, s.data_mode, &mut encoded);
        }
        Ok(encoded)
    }

    fn classify_data_modes(&mut self) -> DataMode {
        let mut start = 0;
        let mut mode = DATA_MODE_NONE;
        let mut highest_required_mode = mode;

        for (i, &v) in self.data.iter().enumerate() {
            let new_mode = if (0x30..=0x39).contains(&v) {
                DATA_MODE_NUMERIC
            } else if v == 0x20
                || v == 0x24
                || v == 0x25
                || v == 0x2a
                || v == 0x2b
                || v == 0x2d
                || v == 0x2e
                || v == 0x2f
                || v == 0x3a
                || (0x41..=0x5a).contains(&v)
            {
                DATA_MODE_ALPHANUMERIC
            } else {
                DATA_MODE_BYTE
            };

            if new_mode != mode {
                if i > 0 {
                    self.actual.push(Segment { data_mode: mode, data: self.data[start..i].to_vec() });
                    start = i;
                }
                mode = new_mode;
            }

            if new_mode > highest_required_mode {
                highest_required_mode = new_mode;
            }
        }

        self.actual.push(Segment { data_mode: mode, data: self.data[start..].to_vec() });

        highest_required_mode
    }

    fn optimise_data_modes(&mut self) -> Result<(), String> {
        let mut i = 0;
        while i < self.actual.len() {
            let mode = self.actual[i].data_mode;
            let mut num_chars = self.actual[i].data.len();

            let mut j = i + 1;
            while j < self.actual.len() {
                let next_num_chars = self.actual[j].data.len();
                let next_mode = self.actual[j].data_mode;

                if next_mode > mode {
                    break;
                }

                let coalesced_length = self.encoded_length(mode, num_chars + next_num_chars)?;
                let seperate_length1 = self.encoded_length(mode, num_chars)?;
                let seperate_length2 = self.encoded_length(next_mode, next_num_chars)?;

                if coalesced_length < seperate_length1 + seperate_length2 {
                    j += 1;
                    num_chars += next_num_chars;
                } else {
                    break;
                }
            }

            let mut optimised = Segment { data_mode: mode, data: Vec::with_capacity(num_chars) };
            for k in i..j {
                optimised.data.extend_from_slice(&self.actual[k].data);
            }
            self.optimised.push(optimised);

            i = j;
        }
        Ok(())
    }

    fn encode_data_raw(&self, data: &[u8], data_mode: DataMode, encoded: &mut Bitset) {
        let mode_indicator = self.mode_indicator(data_mode);
        let char_count_bits = self.char_count_bits(data_mode);

        encoded.append(&Bitset::new(mode_indicator));
        encoded.append_uint32(data.len() as u32, char_count_bits);

        match data_mode {
            DATA_MODE_NUMERIC => {
                let mut i = 0;
                while i < data.len() {
                    let chars_remaining = data.len() - i;
                    let mut value: u32 = 0;
                    let mut bits_used = 1;
                    let mut j = 0;
                    while j < chars_remaining && j < 3 {
                        value *= 10;
                        value += (data[i + j] - 0x30) as u32;
                        bits_used += 3;
                        j += 1;
                    }
                    encoded.append_uint32(value, bits_used);
                    i += 3;
                }
            }
            DATA_MODE_ALPHANUMERIC => {
                let mut i = 0;
                while i < data.len() {
                    let chars_remaining = data.len() - i;
                    let mut value: u32 = 0;
                    let mut j = 0;
                    while j < chars_remaining && j < 2 {
                        value *= 45;
                        value += encode_alphanumeric_character(data[i + j]);
                        j += 1;
                    }
                    let bits_used = if chars_remaining > 1 { 11 } else { 6 };
                    encoded.append_uint32(value, bits_used);
                    i += 2;
                }
            }
            DATA_MODE_BYTE => {
                for &b in data {
                    encoded.append_byte(b, 8);
                }
            }
            _ => {}
        }
    }

    fn mode_indicator(&self, data_mode: DataMode) -> &'static [bool] {
        match data_mode {
            DATA_MODE_NUMERIC => &NUMERIC_MODE_INDICATOR,
            DATA_MODE_ALPHANUMERIC => &ALPHANUMERIC_MODE_INDICATOR,
            DATA_MODE_BYTE => &BYTE_MODE_INDICATOR,
            _ => panic!("Unknown data mode"),
        }
    }

    fn char_count_bits(&self, data_mode: DataMode) -> usize {
        match data_mode {
            DATA_MODE_NUMERIC => self.num_numeric_char_count_bits,
            DATA_MODE_ALPHANUMERIC => self.num_alphanumeric_char_count_bits,
            DATA_MODE_BYTE => self.num_byte_char_count_bits,
            _ => panic!("Unknown data mode"),
        }
    }

    /// The number of bits to encode n symbols in dataMode, or an error if n does not fit the
    /// character count field.
    fn encoded_length(&self, data_mode: DataMode, n: usize) -> Result<usize, String> {
        let mode_indicator = self.mode_indicator(data_mode);
        let char_count_bits = self.char_count_bits(data_mode);

        let max_length = (1usize << char_count_bits) - 1;
        if n > max_length {
            return Err("length too long to be represented".to_string());
        }

        let mut length = mode_indicator.len() + char_count_bits;
        match data_mode {
            DATA_MODE_NUMERIC => {
                length += 10 * (n / 3);
                if n % 3 != 0 {
                    length += 1 + 3 * (n % 3);
                }
            }
            DATA_MODE_ALPHANUMERIC => {
                length += 11 * (n / 2);
                length += 6 * (n % 2);
            }
            DATA_MODE_BYTE => {
                length += 8 * n;
            }
            _ => {}
        }
        Ok(length)
    }
}

fn encode_alphanumeric_character(v: u8) -> u32 {
    let c = v as u32;
    match v {
        b'0'..=b'9' => c - b'0' as u32,
        b'A'..=b'Z' => c - b'A' as u32 + 10,
        b' ' => 36,
        b'$' => 37,
        b'%' => 38,
        b'*' => 39,
        b'+' => 40,
        b'-' => 41,
        b'.' => 42,
        b'/' => 43,
        b':' => 44,
        _ => panic!("encodeAlphanumericCharacter() with non alphanumeric char {}.", v),
    }
}

// ---------------------------------------------------------------------------------------------
// version.go

struct QrCodeVersion {
    version: usize,
    level: Level,
    /// (numBlocks, numCodewords, numDataCodewords)
    block: &'static [(usize, usize, usize)],
    num_remainder_bits: usize,
}

type V = QrCodeVersion;

/// go-qrcode's `versions` table, converted mechanically (its dataEncoderType column is implied by
/// the version number: 1-9, 10-26, 27-40).
static VERSIONS: [QrCodeVersion; 160] = [
    V { version: 1, level: Level::Low, block: &[(1, 26, 19)], num_remainder_bits: 0 },
    V { version: 1, level: Level::Medium, block: &[(1, 26, 16)], num_remainder_bits: 0 },
    V { version: 1, level: Level::High, block: &[(1, 26, 13)], num_remainder_bits: 0 },
    V { version: 1, level: Level::Highest, block: &[(1, 26, 9)], num_remainder_bits: 0 },
    V { version: 2, level: Level::Low, block: &[(1, 44, 34)], num_remainder_bits: 7 },
    V { version: 2, level: Level::Medium, block: &[(1, 44, 28)], num_remainder_bits: 7 },
    V { version: 2, level: Level::High, block: &[(1, 44, 22)], num_remainder_bits: 7 },
    V { version: 2, level: Level::Highest, block: &[(1, 44, 16)], num_remainder_bits: 7 },
    V { version: 3, level: Level::Low, block: &[(1, 70, 55)], num_remainder_bits: 7 },
    V { version: 3, level: Level::Medium, block: &[(1, 70, 44)], num_remainder_bits: 7 },
    V { version: 3, level: Level::High, block: &[(2, 35, 17)], num_remainder_bits: 7 },
    V { version: 3, level: Level::Highest, block: &[(2, 35, 13)], num_remainder_bits: 7 },
    V { version: 4, level: Level::Low, block: &[(1, 100, 80)], num_remainder_bits: 7 },
    V { version: 4, level: Level::Medium, block: &[(2, 50, 32)], num_remainder_bits: 7 },
    V { version: 4, level: Level::High, block: &[(2, 50, 24)], num_remainder_bits: 7 },
    V { version: 4, level: Level::Highest, block: &[(4, 25, 9)], num_remainder_bits: 7 },
    V { version: 5, level: Level::Low, block: &[(1, 134, 108)], num_remainder_bits: 7 },
    V { version: 5, level: Level::Medium, block: &[(2, 67, 43)], num_remainder_bits: 7 },
    V { version: 5, level: Level::High, block: &[(2, 33, 15), (2, 34, 16)], num_remainder_bits: 7 },
    V { version: 5, level: Level::Highest, block: &[(2, 33, 11), (2, 34, 12)], num_remainder_bits: 7 },
    V { version: 6, level: Level::Low, block: &[(2, 86, 68)], num_remainder_bits: 7 },
    V { version: 6, level: Level::Medium, block: &[(4, 43, 27)], num_remainder_bits: 7 },
    V { version: 6, level: Level::High, block: &[(4, 43, 19)], num_remainder_bits: 7 },
    V { version: 6, level: Level::Highest, block: &[(4, 43, 15)], num_remainder_bits: 7 },
    V { version: 7, level: Level::Low, block: &[(2, 98, 78)], num_remainder_bits: 0 },
    V { version: 7, level: Level::Medium, block: &[(4, 49, 31)], num_remainder_bits: 0 },
    V { version: 7, level: Level::High, block: &[(2, 32, 14), (4, 33, 15)], num_remainder_bits: 0 },
    V { version: 7, level: Level::Highest, block: &[(4, 39, 13), (1, 40, 14)], num_remainder_bits: 0 },
    V { version: 8, level: Level::Low, block: &[(2, 121, 97)], num_remainder_bits: 0 },
    V { version: 8, level: Level::Medium, block: &[(2, 60, 38), (2, 61, 39)], num_remainder_bits: 0 },
    V { version: 8, level: Level::High, block: &[(4, 40, 18), (2, 41, 19)], num_remainder_bits: 0 },
    V { version: 8, level: Level::Highest, block: &[(4, 40, 14), (2, 41, 15)], num_remainder_bits: 0 },
    V { version: 9, level: Level::Low, block: &[(2, 146, 116)], num_remainder_bits: 0 },
    V { version: 9, level: Level::Medium, block: &[(3, 58, 36), (2, 59, 37)], num_remainder_bits: 0 },
    V { version: 9, level: Level::High, block: &[(4, 36, 16), (4, 37, 17)], num_remainder_bits: 0 },
    V { version: 9, level: Level::Highest, block: &[(4, 36, 12), (4, 37, 13)], num_remainder_bits: 0 },
    V { version: 10, level: Level::Low, block: &[(2, 86, 68), (2, 87, 69)], num_remainder_bits: 0 },
    V { version: 10, level: Level::Medium, block: &[(4, 69, 43), (1, 70, 44)], num_remainder_bits: 0 },
    V { version: 10, level: Level::High, block: &[(6, 43, 19), (2, 44, 20)], num_remainder_bits: 0 },
    V { version: 10, level: Level::Highest, block: &[(6, 43, 15), (2, 44, 16)], num_remainder_bits: 0 },
    V { version: 11, level: Level::Low, block: &[(4, 101, 81)], num_remainder_bits: 0 },
    V { version: 11, level: Level::Medium, block: &[(1, 80, 50), (4, 81, 51)], num_remainder_bits: 0 },
    V { version: 11, level: Level::High, block: &[(4, 50, 22), (4, 51, 23)], num_remainder_bits: 0 },
    V { version: 11, level: Level::Highest, block: &[(3, 36, 12), (8, 37, 13)], num_remainder_bits: 0 },
    V { version: 12, level: Level::Low, block: &[(2, 116, 92), (2, 117, 93)], num_remainder_bits: 0 },
    V { version: 12, level: Level::Medium, block: &[(6, 58, 36), (2, 59, 37)], num_remainder_bits: 0 },
    V { version: 12, level: Level::High, block: &[(4, 46, 20), (6, 47, 21)], num_remainder_bits: 0 },
    V { version: 12, level: Level::Highest, block: &[(7, 42, 14), (4, 43, 15)], num_remainder_bits: 0 },
    V { version: 13, level: Level::Low, block: &[(4, 133, 107)], num_remainder_bits: 0 },
    V { version: 13, level: Level::Medium, block: &[(8, 59, 37), (1, 60, 38)], num_remainder_bits: 0 },
    V { version: 13, level: Level::High, block: &[(8, 44, 20), (4, 45, 21)], num_remainder_bits: 0 },
    V { version: 13, level: Level::Highest, block: &[(12, 33, 11), (4, 34, 12)], num_remainder_bits: 0 },
    V { version: 14, level: Level::Low, block: &[(3, 145, 115), (1, 146, 116)], num_remainder_bits: 3 },
    V { version: 14, level: Level::Medium, block: &[(4, 64, 40), (5, 65, 41)], num_remainder_bits: 3 },
    V { version: 14, level: Level::High, block: &[(11, 36, 16), (5, 37, 17)], num_remainder_bits: 3 },
    V { version: 14, level: Level::Highest, block: &[(11, 36, 12), (5, 37, 13)], num_remainder_bits: 3 },
    V { version: 15, level: Level::Low, block: &[(5, 109, 87), (1, 110, 88)], num_remainder_bits: 3 },
    V { version: 15, level: Level::Medium, block: &[(5, 65, 41), (5, 66, 42)], num_remainder_bits: 3 },
    V { version: 15, level: Level::High, block: &[(5, 54, 24), (7, 55, 25)], num_remainder_bits: 3 },
    V { version: 15, level: Level::Highest, block: &[(11, 36, 12), (7, 37, 13)], num_remainder_bits: 3 },
    V { version: 16, level: Level::Low, block: &[(5, 122, 98), (1, 123, 99)], num_remainder_bits: 3 },
    V { version: 16, level: Level::Medium, block: &[(7, 73, 45), (3, 74, 46)], num_remainder_bits: 3 },
    V { version: 16, level: Level::High, block: &[(15, 43, 19), (2, 44, 20)], num_remainder_bits: 3 },
    V { version: 16, level: Level::Highest, block: &[(3, 45, 15), (13, 46, 16)], num_remainder_bits: 3 },
    V { version: 17, level: Level::Low, block: &[(1, 135, 107), (5, 136, 108)], num_remainder_bits: 3 },
    V { version: 17, level: Level::Medium, block: &[(10, 74, 46), (1, 75, 47)], num_remainder_bits: 3 },
    V { version: 17, level: Level::High, block: &[(1, 50, 22), (15, 51, 23)], num_remainder_bits: 3 },
    V { version: 17, level: Level::Highest, block: &[(2, 42, 14), (17, 43, 15)], num_remainder_bits: 3 },
    V { version: 18, level: Level::Low, block: &[(5, 150, 120), (1, 151, 121)], num_remainder_bits: 3 },
    V { version: 18, level: Level::Medium, block: &[(9, 69, 43), (4, 70, 44)], num_remainder_bits: 3 },
    V { version: 18, level: Level::High, block: &[(17, 50, 22), (1, 51, 23)], num_remainder_bits: 3 },
    V { version: 18, level: Level::Highest, block: &[(2, 42, 14), (19, 43, 15)], num_remainder_bits: 3 },
    V { version: 19, level: Level::Low, block: &[(3, 141, 113), (4, 142, 114)], num_remainder_bits: 3 },
    V { version: 19, level: Level::Medium, block: &[(3, 70, 44), (11, 71, 45)], num_remainder_bits: 3 },
    V { version: 19, level: Level::High, block: &[(17, 47, 21), (4, 48, 22)], num_remainder_bits: 3 },
    V { version: 19, level: Level::Highest, block: &[(9, 39, 13), (16, 40, 14)], num_remainder_bits: 3 },
    V { version: 20, level: Level::Low, block: &[(3, 135, 107), (5, 136, 108)], num_remainder_bits: 3 },
    V { version: 20, level: Level::Medium, block: &[(3, 67, 41), (13, 68, 42)], num_remainder_bits: 3 },
    V { version: 20, level: Level::High, block: &[(15, 54, 24), (5, 55, 25)], num_remainder_bits: 3 },
    V { version: 20, level: Level::Highest, block: &[(15, 43, 15), (10, 44, 16)], num_remainder_bits: 3 },
    V { version: 21, level: Level::Low, block: &[(4, 144, 116), (4, 145, 117)], num_remainder_bits: 4 },
    V { version: 21, level: Level::Medium, block: &[(17, 68, 42)], num_remainder_bits: 4 },
    V { version: 21, level: Level::High, block: &[(17, 50, 22), (6, 51, 23)], num_remainder_bits: 4 },
    V { version: 21, level: Level::Highest, block: &[(19, 46, 16), (6, 47, 17)], num_remainder_bits: 4 },
    V { version: 22, level: Level::Low, block: &[(2, 139, 111), (7, 140, 112)], num_remainder_bits: 4 },
    V { version: 22, level: Level::Medium, block: &[(17, 74, 46)], num_remainder_bits: 4 },
    V { version: 22, level: Level::High, block: &[(7, 54, 24), (16, 55, 25)], num_remainder_bits: 4 },
    V { version: 22, level: Level::Highest, block: &[(34, 37, 13)], num_remainder_bits: 4 },
    V { version: 23, level: Level::Low, block: &[(4, 151, 121), (5, 152, 122)], num_remainder_bits: 4 },
    V { version: 23, level: Level::Medium, block: &[(4, 75, 47), (14, 76, 48)], num_remainder_bits: 4 },
    V { version: 23, level: Level::High, block: &[(11, 54, 24), (14, 55, 25)], num_remainder_bits: 4 },
    V { version: 23, level: Level::Highest, block: &[(16, 45, 15), (14, 46, 16)], num_remainder_bits: 4 },
    V { version: 24, level: Level::Low, block: &[(6, 147, 117), (4, 148, 118)], num_remainder_bits: 4 },
    V { version: 24, level: Level::Medium, block: &[(6, 73, 45), (14, 74, 46)], num_remainder_bits: 4 },
    V { version: 24, level: Level::High, block: &[(11, 54, 24), (16, 55, 25)], num_remainder_bits: 4 },
    V { version: 24, level: Level::Highest, block: &[(30, 46, 16), (2, 47, 17)], num_remainder_bits: 4 },
    V { version: 25, level: Level::Low, block: &[(8, 132, 106), (4, 133, 107)], num_remainder_bits: 4 },
    V { version: 25, level: Level::Medium, block: &[(8, 75, 47), (13, 76, 48)], num_remainder_bits: 4 },
    V { version: 25, level: Level::High, block: &[(7, 54, 24), (22, 55, 25)], num_remainder_bits: 4 },
    V { version: 25, level: Level::Highest, block: &[(22, 45, 15), (13, 46, 16)], num_remainder_bits: 4 },
    V { version: 26, level: Level::Low, block: &[(10, 142, 114), (2, 143, 115)], num_remainder_bits: 4 },
    V { version: 26, level: Level::Medium, block: &[(19, 74, 46), (4, 75, 47)], num_remainder_bits: 4 },
    V { version: 26, level: Level::High, block: &[(28, 50, 22), (6, 51, 23)], num_remainder_bits: 4 },
    V { version: 26, level: Level::Highest, block: &[(33, 46, 16), (4, 47, 17)], num_remainder_bits: 4 },
    V { version: 27, level: Level::Low, block: &[(8, 152, 122), (4, 153, 123)], num_remainder_bits: 4 },
    V { version: 27, level: Level::Medium, block: &[(22, 73, 45), (3, 74, 46)], num_remainder_bits: 4 },
    V { version: 27, level: Level::High, block: &[(8, 53, 23), (26, 54, 24)], num_remainder_bits: 4 },
    V { version: 27, level: Level::Highest, block: &[(12, 45, 15), (28, 46, 16)], num_remainder_bits: 4 },
    V { version: 28, level: Level::Low, block: &[(3, 147, 117), (10, 148, 118)], num_remainder_bits: 3 },
    V { version: 28, level: Level::Medium, block: &[(3, 73, 45), (23, 74, 46)], num_remainder_bits: 3 },
    V { version: 28, level: Level::High, block: &[(4, 54, 24), (31, 55, 25)], num_remainder_bits: 3 },
    V { version: 28, level: Level::Highest, block: &[(11, 45, 15), (31, 46, 16)], num_remainder_bits: 3 },
    V { version: 29, level: Level::Low, block: &[(7, 146, 116), (7, 147, 117)], num_remainder_bits: 3 },
    V { version: 29, level: Level::Medium, block: &[(21, 73, 45), (7, 74, 46)], num_remainder_bits: 3 },
    V { version: 29, level: Level::High, block: &[(1, 53, 23), (37, 54, 24)], num_remainder_bits: 3 },
    V { version: 29, level: Level::Highest, block: &[(19, 45, 15), (26, 46, 16)], num_remainder_bits: 3 },
    V { version: 30, level: Level::Low, block: &[(5, 145, 115), (10, 146, 116)], num_remainder_bits: 3 },
    V { version: 30, level: Level::Medium, block: &[(19, 75, 47), (10, 76, 48)], num_remainder_bits: 3 },
    V { version: 30, level: Level::High, block: &[(15, 54, 24), (25, 55, 25)], num_remainder_bits: 3 },
    V { version: 30, level: Level::Highest, block: &[(23, 45, 15), (25, 46, 16)], num_remainder_bits: 3 },
    V { version: 31, level: Level::Low, block: &[(13, 145, 115), (3, 146, 116)], num_remainder_bits: 3 },
    V { version: 31, level: Level::Medium, block: &[(2, 74, 46), (29, 75, 47)], num_remainder_bits: 3 },
    V { version: 31, level: Level::High, block: &[(42, 54, 24), (1, 55, 25)], num_remainder_bits: 3 },
    V { version: 31, level: Level::Highest, block: &[(23, 45, 15), (28, 46, 16)], num_remainder_bits: 3 },
    V { version: 32, level: Level::Low, block: &[(17, 145, 115)], num_remainder_bits: 3 },
    V { version: 32, level: Level::Medium, block: &[(10, 74, 46), (23, 75, 47)], num_remainder_bits: 3 },
    V { version: 32, level: Level::High, block: &[(10, 54, 24), (35, 55, 25)], num_remainder_bits: 3 },
    V { version: 32, level: Level::Highest, block: &[(19, 45, 15), (35, 46, 16)], num_remainder_bits: 3 },
    V { version: 33, level: Level::Low, block: &[(17, 145, 115), (1, 146, 116)], num_remainder_bits: 3 },
    V { version: 33, level: Level::Medium, block: &[(14, 74, 46), (21, 75, 47)], num_remainder_bits: 3 },
    V { version: 33, level: Level::High, block: &[(29, 54, 24), (19, 55, 25)], num_remainder_bits: 3 },
    V { version: 33, level: Level::Highest, block: &[(11, 45, 15), (46, 46, 16)], num_remainder_bits: 3 },
    V { version: 34, level: Level::Low, block: &[(13, 145, 115), (6, 146, 116)], num_remainder_bits: 3 },
    V { version: 34, level: Level::Medium, block: &[(14, 74, 46), (23, 75, 47)], num_remainder_bits: 3 },
    V { version: 34, level: Level::High, block: &[(44, 54, 24), (7, 55, 25)], num_remainder_bits: 3 },
    V { version: 34, level: Level::Highest, block: &[(59, 46, 16), (1, 47, 17)], num_remainder_bits: 3 },
    V { version: 35, level: Level::Low, block: &[(12, 151, 121), (7, 152, 122)], num_remainder_bits: 0 },
    V { version: 35, level: Level::Medium, block: &[(12, 75, 47), (26, 76, 48)], num_remainder_bits: 0 },
    V { version: 35, level: Level::High, block: &[(39, 54, 24), (14, 55, 25)], num_remainder_bits: 0 },
    V { version: 35, level: Level::Highest, block: &[(22, 45, 15), (41, 46, 16)], num_remainder_bits: 0 },
    V { version: 36, level: Level::Low, block: &[(6, 151, 121), (14, 152, 122)], num_remainder_bits: 0 },
    V { version: 36, level: Level::Medium, block: &[(6, 75, 47), (34, 76, 48)], num_remainder_bits: 0 },
    V { version: 36, level: Level::High, block: &[(46, 54, 24), (10, 55, 25)], num_remainder_bits: 0 },
    V { version: 36, level: Level::Highest, block: &[(2, 45, 15), (64, 46, 16)], num_remainder_bits: 0 },
    V { version: 37, level: Level::Low, block: &[(17, 152, 122), (4, 153, 123)], num_remainder_bits: 0 },
    V { version: 37, level: Level::Medium, block: &[(29, 74, 46), (14, 75, 47)], num_remainder_bits: 0 },
    V { version: 37, level: Level::High, block: &[(49, 54, 24), (10, 55, 25)], num_remainder_bits: 0 },
    V { version: 37, level: Level::Highest, block: &[(24, 45, 15), (46, 46, 16)], num_remainder_bits: 0 },
    V { version: 38, level: Level::Low, block: &[(4, 152, 122), (18, 153, 123)], num_remainder_bits: 0 },
    V { version: 38, level: Level::Medium, block: &[(13, 74, 46), (32, 75, 47)], num_remainder_bits: 0 },
    V { version: 38, level: Level::High, block: &[(48, 54, 24), (14, 55, 25)], num_remainder_bits: 0 },
    V { version: 38, level: Level::Highest, block: &[(42, 45, 15), (32, 46, 16)], num_remainder_bits: 0 },
    V { version: 39, level: Level::Low, block: &[(20, 147, 117), (4, 148, 118)], num_remainder_bits: 0 },
    V { version: 39, level: Level::Medium, block: &[(40, 75, 47), (7, 76, 48)], num_remainder_bits: 0 },
    V { version: 39, level: Level::High, block: &[(43, 54, 24), (22, 55, 25)], num_remainder_bits: 0 },
    V { version: 39, level: Level::Highest, block: &[(10, 45, 15), (67, 46, 16)], num_remainder_bits: 0 },
    V { version: 40, level: Level::Low, block: &[(19, 148, 118), (6, 149, 119)], num_remainder_bits: 0 },
    V { version: 40, level: Level::Medium, block: &[(18, 75, 47), (31, 76, 48)], num_remainder_bits: 0 },
    V { version: 40, level: Level::High, block: &[(34, 54, 24), (34, 55, 25)], num_remainder_bits: 0 },
    V { version: 40, level: Level::Highest, block: &[(20, 45, 15), (61, 46, 16)], num_remainder_bits: 0 },
];

/// formatBitSequence[..].regular: the 5 data bits to the completed 15-bit Format Information.
const FORMAT_BIT_SEQUENCE: [u32; 32] = [
    0x5412, 0x5125, 0x5e7c, 0x5b4b, 0x45f9, 0x40ce, 0x4f97, 0x4aa0, 0x77c4, 0x72f3, 0x7daa, 0x789d,
    0x662f, 0x6318, 0x6c41, 0x6976, 0x1689, 0x13be, 0x1ce7, 0x19d0, 0x0762, 0x0255, 0x0d0c, 0x083b,
    0x355f, 0x3068, 0x3f31, 0x3a06, 0x24b4, 0x2183, 0x2eda, 0x2bed,
];

/// versionBitSequence: the 18-bit Version Information for versions 7-40.
const VERSION_BIT_SEQUENCE: [u32; 41] = [
    0x00000, 0x00000, 0x00000, 0x00000, 0x00000, 0x00000, 0x00000, 0x07c94, 0x085bc, 0x09a99,
    0x0a4d3, 0x0bbf6, 0x0c762, 0x0d847, 0x0e60d, 0x0f928, 0x10b78, 0x1145d, 0x12a17, 0x13532,
    0x149a6, 0x15683, 0x168c9, 0x177ec, 0x18ec4, 0x191e1, 0x1afab, 0x1b08e, 0x1cc1a, 0x1d33f,
    0x1ed75, 0x1f250, 0x209d5, 0x216f0, 0x228ba, 0x2379f, 0x24b0b, 0x2542e, 0x26a64, 0x27541,
    0x28c69,
];

const FORMAT_INFO_LENGTH_BITS: usize = 15;
const VERSION_INFO_LENGTH_BITS: usize = 18;

impl QrCodeVersion {
    fn format_info(&self, mask_pattern: usize) -> Bitset {
        let mut format_id = match self.level {
            Level::Low => 0x08,
            Level::Medium => 0x00,
            Level::High => 0x18,
            Level::Highest => 0x10,
        };
        if mask_pattern > 7 {
            panic!("Invalid maskPattern {}", mask_pattern);
        }
        format_id |= mask_pattern & 0x7;
        let mut result = Bitset::default();
        result.append_uint32(FORMAT_BIT_SEQUENCE[format_id], FORMAT_INFO_LENGTH_BITS);
        result
    }

    fn version_info(&self) -> Option<Bitset> {
        if self.version < 7 {
            return None;
        }
        let mut result = Bitset::default();
        result.append_uint32(VERSION_BIT_SEQUENCE[self.version], 18);
        Some(result)
    }

    fn num_data_bits(&self) -> usize {
        self.block.iter().map(|&(n, _, d)| 8 * n * d).sum()
    }

    fn num_terminator_bits_required(&self, num_data_bits: usize) -> usize {
        let num_free_bits = self.num_data_bits() as isize - num_data_bits as isize;
        if num_free_bits >= 4 {
            4
        } else {
            num_free_bits as usize
        }
    }

    fn num_blocks(&self) -> usize {
        self.block.iter().map(|&(n, _, _)| n).sum()
    }

    fn num_bits_to_pad_to_codeword(&self, num_data_bits: usize) -> usize {
        if num_data_bits == self.num_data_bits() {
            return 0;
        }
        (8 - num_data_bits % 8) % 8
    }

    fn symbol_size(&self) -> usize {
        21 + (self.version - 1) * 4
    }

    fn quiet_zone_size(&self) -> usize {
        4
    }
}

/// The smallest version of `level` in the encoder's range that holds numDataBits.
fn choose_qr_code_version(
    level: Level,
    encoder: &DataEncoder,
    num_data_bits: usize,
) -> Option<&'static QrCodeVersion> {
    for v in VERSIONS.iter() {
        if v.level != level {
            continue;
        } else if v.version < encoder.min_version {
            continue;
        } else if v.version > encoder.max_version {
            break;
        }
        if v.num_data_bits() >= num_data_bits {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// symbol.go: the module matrix, with a quiet zone the accessors skip.

#[derive(Clone, Debug)]
struct Symbol {
    module: Vec<Vec<bool>>,
    is_used: Vec<Vec<bool>>,
    /// symbolSize + 2*quietZoneSize
    size: usize,
    symbol_size: usize,
    quiet_zone_size: usize,
}

const PENALTY_WEIGHT1: i32 = 3;
const PENALTY_WEIGHT2: i32 = 3;
const PENALTY_WEIGHT3: i32 = 40;
const PENALTY_WEIGHT4: i32 = 10;

impl Symbol {
    fn new(size: usize, quiet_zone_size: usize) -> Symbol {
        let n = size + 2 * quiet_zone_size;
        Symbol {
            module: vec![vec![false; n]; n],
            is_used: vec![vec![false; n]; n],
            size: n,
            symbol_size: size,
            quiet_zone_size,
        }
    }

    fn idx(&self, v: isize) -> usize {
        let i = v + self.quiet_zone_size as isize;
        assert!(i >= 0 && (i as usize) < self.size, "index out of range");
        i as usize
    }

    fn get(&self, x: isize, y: isize) -> bool {
        self.module[self.idx(y)][self.idx(x)]
    }

    fn empty(&self, x: isize, y: isize) -> bool {
        !self.is_used[self.idx(y)][self.idx(x)]
    }

    fn num_empty_modules(&self) -> usize {
        let q = self.quiet_zone_size;
        let mut count = 0;
        for y in 0..self.symbol_size {
            for x in 0..self.symbol_size {
                if !self.is_used[y + q][x + q] {
                    count += 1;
                }
            }
        }
        count
    }

    fn set(&mut self, x: isize, y: isize, v: bool) {
        let (xi, yi) = (self.idx(x), self.idx(y));
        self.module[yi][xi] = v;
        self.is_used[yi][xi] = true;
    }

    fn set_2d_pattern(&mut self, x: isize, y: isize, v: &[&[bool]]) {
        for (j, row) in v.iter().enumerate() {
            for (i, &value) in row.iter().enumerate() {
                self.set(x + i as isize, y + j as isize, value);
            }
        }
    }

    fn bitmap(&self) -> Vec<Vec<bool>> {
        self.module.clone()
    }

    fn penalty_score(&self) -> i32 {
        self.penalty1() + self.penalty2() + self.penalty3() + self.penalty4()
    }

    /// Adjacent modules in a row/column of the same colour (go-qrcode's scoring: 4 at the 6th).
    fn penalty1(&self) -> i32 {
        let n = self.symbol_size as isize;
        let mut penalty = 0;
        for x in 0..n {
            let mut last_value = self.get(x, 0);
            let mut count = 1;
            for y in 1..n {
                let v = self.get(x, y);
                if v != last_value {
                    count = 1;
                    last_value = v;
                } else {
                    count += 1;
                    if count == 6 {
                        penalty += PENALTY_WEIGHT1 + 1;
                    } else if count > 6 {
                        penalty += 1;
                    }
                }
            }
        }
        for y in 0..n {
            let mut last_value = self.get(0, y);
            let mut count = 1;
            for x in 1..n {
                let v = self.get(x, y);
                if v != last_value {
                    count = 1;
                    last_value = v;
                } else {
                    count += 1;
                    if count == 6 {
                        penalty += PENALTY_WEIGHT1 + 1;
                    } else if count > 6 {
                        penalty += 1;
                    }
                }
            }
        }
        penalty
    }

    /// 2x2 blocks of the same colour.
    fn penalty2(&self) -> i32 {
        let n = self.symbol_size as isize;
        let mut penalty = 0;
        for y in 1..n {
            for x in 1..n {
                let top_left = self.get(x - 1, y - 1);
                let above = self.get(x, y - 1);
                let left = self.get(x - 1, y);
                let current = self.get(x, y);
                if current == left && current == above && current == top_left {
                    penalty += 1;
                }
            }
        }
        penalty * PENALTY_WEIGHT2
    }

    /// The 1:1:3:1:1 finder-like pattern, by go-qrcode's int16 shift register.
    fn penalty3(&self) -> i32 {
        let n = self.symbol_size as isize;
        let mut penalty = 0;
        for y in 0..n {
            let mut bit_buffer: i16 = 0x00;
            for x in 0..n {
                bit_buffer = bit_buffer.wrapping_shl(1);
                if self.get(x, y) {
                    bit_buffer |= 1;
                }
                match bit_buffer & 0x7ff {
                    0x05d | 0x5d0 => {
                        penalty += PENALTY_WEIGHT3;
                        bit_buffer = 0xFF;
                    }
                    _ => {
                        if x == n - 1 && (bit_buffer & 0x7f) == 0x5d {
                            penalty += PENALTY_WEIGHT3;
                            bit_buffer = 0xFF;
                        }
                    }
                }
            }
        }
        for x in 0..n {
            let mut bit_buffer: i16 = 0x00;
            for y in 0..n {
                bit_buffer = bit_buffer.wrapping_shl(1);
                if self.get(x, y) {
                    bit_buffer |= 1;
                }
                match bit_buffer & 0x7ff {
                    0x05d | 0x5d0 => {
                        penalty += PENALTY_WEIGHT3;
                        bit_buffer = 0xFF;
                    }
                    _ => {
                        if y == n - 1 && (bit_buffer & 0x7f) == 0x5d {
                            penalty += PENALTY_WEIGHT3;
                            bit_buffer = 0xFF;
                        }
                    }
                }
            }
        }
        penalty
    }

    /// The dark/light balance.
    fn penalty4(&self) -> i32 {
        let n = self.symbol_size as isize;
        let num_modules = (self.symbol_size * self.symbol_size) as i32;
        let mut num_dark_modules = 0;
        for x in 0..n {
            for y in 0..n {
                if self.get(x, y) {
                    num_dark_modules += 1;
                }
            }
        }
        let mut deviation = num_modules / 2 - num_dark_modules;
        if deviation < 0 {
            deviation *= -1;
        }
        PENALTY_WEIGHT4 * (deviation / (num_modules / 20))
    }
}

// ---------------------------------------------------------------------------------------------
// regular_symbol.go

const B0: bool = false;
const B1: bool = true;

const ALIGNMENT_PATTERN_CENTER: [&[isize]; 41] = [
    &[], // Version 0 doesn't exist.
    &[], // Version 1 doesn't use alignment patterns.
    &[6, 18],
    &[6, 22],
    &[6, 26],
    &[6, 30],
    &[6, 34],
    &[6, 22, 38],
    &[6, 24, 42],
    &[6, 26, 46],
    &[6, 28, 50],
    &[6, 30, 54],
    &[6, 32, 58],
    &[6, 34, 62],
    &[6, 26, 46, 66],
    &[6, 26, 48, 70],
    &[6, 26, 50, 74],
    &[6, 30, 54, 78],
    &[6, 30, 56, 82],
    &[6, 30, 58, 86],
    &[6, 34, 62, 90],
    &[6, 28, 50, 72, 94],
    &[6, 26, 50, 74, 98],
    &[6, 30, 54, 78, 102],
    &[6, 28, 54, 80, 106],
    &[6, 32, 58, 84, 110],
    &[6, 30, 58, 86, 114],
    &[6, 34, 62, 90, 118],
    &[6, 26, 50, 74, 98, 122],
    &[6, 30, 54, 78, 102, 126],
    &[6, 26, 52, 78, 104, 130],
    &[6, 30, 56, 82, 108, 134],
    &[6, 34, 60, 86, 112, 138],
    &[6, 30, 58, 86, 114, 142],
    &[6, 34, 62, 90, 118, 146],
    &[6, 30, 54, 78, 102, 126, 150],
    &[6, 24, 50, 76, 102, 128, 154],
    &[6, 28, 54, 80, 106, 132, 158],
    &[6, 32, 58, 84, 110, 136, 162],
    &[6, 26, 54, 82, 110, 138, 166],
    &[6, 30, 58, 86, 114, 142, 170],
];

const FINDER_PATTERN: [&[bool]; 7] = [
    &[B1, B1, B1, B1, B1, B1, B1],
    &[B1, B0, B0, B0, B0, B0, B1],
    &[B1, B0, B1, B1, B1, B0, B1],
    &[B1, B0, B1, B1, B1, B0, B1],
    &[B1, B0, B1, B1, B1, B0, B1],
    &[B1, B0, B0, B0, B0, B0, B1],
    &[B1, B1, B1, B1, B1, B1, B1],
];

const FINDER_PATTERN_SIZE: isize = 7;

const FINDER_PATTERN_HORIZONTAL_BORDER: [&[bool]; 1] = [&[B0, B0, B0, B0, B0, B0, B0, B0]];

const FINDER_PATTERN_VERTICAL_BORDER: [&[bool]; 8] =
    [&[B0], &[B0], &[B0], &[B0], &[B0], &[B0], &[B0], &[B0]];

const ALIGNMENT_PATTERN: [&[bool]; 5] = [
    &[B1, B1, B1, B1, B1],
    &[B1, B0, B0, B0, B1],
    &[B1, B0, B1, B0, B1],
    &[B1, B0, B0, B0, B1],
    &[B1, B1, B1, B1, B1],
];

struct RegularSymbol<'a> {
    version: &'a QrCodeVersion,
    mask: usize,
    data: &'a Bitset,
    symbol: Symbol,
    size: isize,
}

fn build_regular_symbol(
    version: &QrCodeVersion,
    mask: usize,
    data: &Bitset,
    include_quiet_zone: bool,
) -> Symbol {
    let quiet_zone_size = if include_quiet_zone { version.quiet_zone_size() } else { 0 };
    let mut m = RegularSymbol {
        version,
        mask,
        data,
        symbol: Symbol::new(version.symbol_size(), quiet_zone_size),
        size: version.symbol_size() as isize,
    };
    m.add_finder_patterns();
    m.add_alignment_patterns();
    m.add_timing_patterns();
    m.add_format_info();
    m.add_version_info();
    m.add_data();
    m.symbol
}

impl RegularSymbol<'_> {
    fn add_finder_patterns(&mut self) {
        let fp_size = FINDER_PATTERN_SIZE;
        let fp = &FINDER_PATTERN[..];
        let fp_h = &FINDER_PATTERN_HORIZONTAL_BORDER[..];
        let fp_v = &FINDER_PATTERN_VERTICAL_BORDER[..];
        let s = self.size;

        // Top left.
        self.symbol.set_2d_pattern(0, 0, fp);
        self.symbol.set_2d_pattern(0, fp_size, fp_h);
        self.symbol.set_2d_pattern(fp_size, 0, fp_v);

        // Top right.
        self.symbol.set_2d_pattern(s - fp_size, 0, fp);
        self.symbol.set_2d_pattern(s - fp_size - 1, fp_size, fp_h);
        self.symbol.set_2d_pattern(s - fp_size - 1, 0, fp_v);

        // Bottom left.
        self.symbol.set_2d_pattern(0, s - fp_size, fp);
        self.symbol.set_2d_pattern(0, s - fp_size - 1, fp_h);
        self.symbol.set_2d_pattern(fp_size, s - fp_size - 1, fp_v);
    }

    fn add_alignment_patterns(&mut self) {
        let centers = ALIGNMENT_PATTERN_CENTER[self.version.version];
        for &x in centers {
            for &y in centers {
                if !self.symbol.empty(x, y) {
                    continue;
                }
                self.symbol.set_2d_pattern(x - 2, y - 2, &ALIGNMENT_PATTERN);
            }
        }
    }

    fn add_timing_patterns(&mut self) {
        let mut value = true;
        let mut i = FINDER_PATTERN_SIZE + 1;
        while i < self.size - FINDER_PATTERN_SIZE {
            self.symbol.set(i, FINDER_PATTERN_SIZE - 1, value);
            self.symbol.set(FINDER_PATTERN_SIZE - 1, i, value);
            value = !value;
            i += 1;
        }
    }

    fn add_format_info(&mut self) {
        let fp_size = FINDER_PATTERN_SIZE;
        let l = FORMAT_INFO_LENGTH_BITS - 1;
        let f = self.version.format_info(self.mask);
        let s = self.size;

        // Bits 0-7, under the top right finder pattern.
        for i in 0..=7usize {
            self.symbol.set(s - i as isize - 1, fp_size + 1, f.at(l - i));
        }
        // Bits 0-5, right of the top left finder pattern.
        for i in 0..=5usize {
            self.symbol.set(fp_size + 1, i as isize, f.at(l - i));
        }
        // Bits 6-8 on the corner of the top left finder pattern.
        self.symbol.set(fp_size + 1, fp_size, f.at(l - 6));
        self.symbol.set(fp_size + 1, fp_size + 1, f.at(l - 7));
        self.symbol.set(fp_size, fp_size + 1, f.at(l - 8));
        // Bits 9-14 on the underside of the top left finder pattern.
        for i in 9..=14usize {
            self.symbol.set(14 - i as isize, fp_size + 1, f.at(l - i));
        }
        // Bits 8-14 on the right side of the bottom left finder pattern.
        for i in 8..=14usize {
            self.symbol.set(fp_size + 1, s - fp_size + i as isize - 8, f.at(l - i));
        }
        // Always dark symbol.
        self.symbol.set(fp_size + 1, s - fp_size - 1, true);
    }

    fn add_version_info(&mut self) {
        let fp_size = FINDER_PATTERN_SIZE;
        let Some(v) = self.version.version_info() else {
            return;
        };
        let l = VERSION_INFO_LENGTH_BITS - 1;
        let s = self.size;
        for i in 0..v.len() {
            let (a, b) = ((i / 3) as isize, (i % 3) as isize);
            // Above the bottom left finder pattern.
            self.symbol.set(a, s - fp_size - 4 + b, v.at(l - i));
            // Left of the top right finder pattern.
            self.symbol.set(s - fp_size - 4 + b, a, v.at(l - i));
        }
    }

    fn add_data(&mut self) {
        let mut x_offset: isize = 1;
        let mut up = true;

        let mut x = self.size - 2;
        let mut y = self.size - 1;

        for i in 0..self.data.len() {
            let mask = match self.mask {
                0 => (y + x + x_offset) % 2 == 0,
                1 => y % 2 == 0,
                2 => (x + x_offset) % 3 == 0,
                3 => (y + x + x_offset) % 3 == 0,
                4 => (y / 2 + (x + x_offset) / 3) % 2 == 0,
                5 => (y * (x + x_offset)) % 2 + (y * (x + x_offset)) % 3 == 0,
                6 => ((y * (x + x_offset)) % 2 + ((y * (x + x_offset)) % 3)) % 2 == 0,
                7 => ((y + x + x_offset) % 2 + ((y * (x + x_offset)) % 3)) % 2 == 0,
                _ => false,
            };

            // != is XOR.
            self.symbol.set(x + x_offset, y, mask != self.data.at(i));

            if i == self.data.len() - 1 {
                break;
            }

            // Find the next free module.
            loop {
                if x_offset == 1 {
                    x_offset = 0;
                } else {
                    x_offset = 1;
                    if up {
                        if y > 0 {
                            y -= 1;
                        } else {
                            up = false;
                            x -= 2;
                        }
                    } else if y < self.size - 1 {
                        y += 1;
                    } else {
                        up = true;
                        x -= 2;
                    }
                }

                // Skip over the vertical timing pattern entirely.
                if x == 5 {
                    x -= 1;
                }

                if self.symbol.empty(x + x_offset, y) {
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// qrcode.go

/// An encoded QR Code (go-qrcode QRCode, with its lazy encode() done once, in `new`).
pub struct Qr {
    #[allow(dead_code)]
    content: String,
    #[allow(dead_code)]
    level: Level,
    #[allow(dead_code)]
    version_number: usize,
    #[allow(dead_code)]
    mask: usize,
    symbol: Symbol,
}

impl Qr {
    /// go-qrcode qrcode.New: the smallest version that holds `content` at `level`. Errors as Go's:
    /// "no data to encode" for "", "content too long to encode" when no version holds it.
    pub fn new(content: &str, level: Level) -> Result<Qr, String> {
        let encoders = [DataEncoderType::T1To9, DataEncoderType::T10To26, DataEncoderType::T27To40];

        let mut encoder = None;
        let mut encoded = None;
        let mut chosen_version: Option<&'static QrCodeVersion> = None;
        let mut err: Option<String> = None;

        for t in encoders {
            let mut e = DataEncoder::new(t);
            let r = e.encode(content.as_bytes());
            match r {
                Err(e) => {
                    err = Some(e);
                    continue;
                }
                Ok(bits) => {
                    err = None;
                    chosen_version = choose_qr_code_version(level, &e, bits.len());
                    encoded = Some(bits);
                    encoder = Some(e);
                    if chosen_version.is_some() {
                        break;
                    }
                }
            }
        }

        if let Some(e) = err {
            return Err(e);
        }
        let Some(version) = chosen_version else {
            return Err("content too long to encode".to_string());
        };
        let _ = encoder;
        let data = encoded.expect("encoded with a chosen version");

        let (symbol, mask) = qr_encode(version, data, true);
        Ok(Qr { content: content.to_string(), level, version_number: version.version, mask, symbol })
    }

    /// go-qrcode (*QRCode).Bitmap: bitmap[y][x], quiet zone included, true = black.
    pub fn bitmap(&self) -> Vec<Vec<bool>> {
        self.symbol.bitmap()
    }

    /// go-qrcode (*QRCode).Image: the square width and the row-major pixels, true = black.
    /// A positive size is the width in pixels (raised to the module count if smaller); a negative
    /// size is pixels per module. Each pixel takes the module under it, by Go's float mapping.
    pub fn image(&self, size: i32) -> (usize, Vec<bool>) {
        let real_size = self.symbol.size as i64;
        let mut size = size as i64;
        if size < 0 {
            size = size * -1 * real_size;
        }
        if size < real_size {
            size = real_size;
        }
        let size = size as usize;

        let bitmap = self.symbol.bitmap();
        let modules_per_pixel = real_size as f64 / size as f64;
        let mut pix = vec![false; size * size];
        for y in 0..size {
            let y2 = (y as f64 * modules_per_pixel) as usize;
            for x in 0..size {
                let x2 = (x as f64 * modules_per_pixel) as usize;
                if bitmap[y2][x2] {
                    pix[y * size + x] = true;
                }
            }
        }
        (size, pix)
    }

    /// go-qrcode (*QRCode).PNG: `image(size)` as a 1-bit paletted PNG, palette [white, black].
    pub fn png(&self, size: i32) -> Vec<u8> {
        let (w, pix) = self.image(size);
        write_png(w, &pix)
    }
}

/// go-qrcode qrcode.Encode(content, level, size).
pub fn encode_png(content: &str, level: Level, size: i32) -> Result<Vec<u8>, String> {
    Ok(Qr::new(content, level)?.png(size))
}

/// (*QRCode).encode: terminator, padding, blocks and error correction, then the mask with the
/// lowest penalty (the first of equals).
fn qr_encode(version: &QrCodeVersion, mut data: Bitset, include_border: bool) -> (Symbol, usize) {
    let num_terminator_bits = version.num_terminator_bits_required(data.len());
    data.append_num_bools(num_terminator_bits, false);
    add_padding(version, &mut data);

    let encoded = encode_blocks(version, &data);

    const NUM_MASKS: usize = 8;
    let mut best: Option<(Symbol, usize)> = None;
    let mut penalty = 0;
    for mask in 0..NUM_MASKS {
        let s = build_regular_symbol(version, mask, &encoded, include_border);
        let num_empty_modules = s.num_empty_modules();
        if num_empty_modules != 0 {
            panic!(
                "bug: numEmptyModules is {} (expected 0) (version={})",
                num_empty_modules, version.version
            );
        }
        let p = s.penalty_score();
        if best.is_none() || p < penalty {
            best = Some((s, mask));
            penalty = p;
        }
    }
    best.expect("eight masks")
}

fn encode_blocks(version: &QrCodeVersion, data: &Bitset) -> Bitset {
    struct DataBlock {
        data: Bitset,
        ec_start_offset: usize,
    }

    let mut block: Vec<DataBlock> = Vec::with_capacity(version.num_blocks());
    let mut end = 0;
    for &(num_blocks, num_codewords, num_data_codewords) in version.block {
        for _ in 0..num_blocks {
            let start = end;
            end = start + num_data_codewords * 8;
            let num_error_codewords = num_codewords - num_data_codewords;
            block.push(DataBlock {
                data: rs_encode(&data.substr(start, end), num_error_codewords),
                ec_start_offset: end - start,
            });
        }
    }

    let mut result = Bitset::default();

    // Combine data blocks.
    let mut working = true;
    let mut i = 0;
    while working {
        working = false;
        for b in &block {
            if i >= b.ec_start_offset {
                continue;
            }
            result.append(&b.data.substr(i, i + 8));
            working = true;
        }
        i += 8;
    }

    // Combine error correction blocks.
    working = true;
    i = 0;
    while working {
        working = false;
        for b in &block {
            let offset = i + b.ec_start_offset;
            if offset >= b.data.len() {
                continue;
            }
            result.append(&b.data.substr(offset, offset + 8));
            working = true;
        }
        i += 8;
    }

    result.append_num_bools(version.num_remainder_bits, false);
    result
}

fn add_padding(version: &QrCodeVersion, data: &mut Bitset) {
    let num_data_bits = version.num_data_bits();
    if data.len() == num_data_bits {
        return;
    }

    data.append_num_bools(version.num_bits_to_pad_to_codeword(data.len()), false);

    let padding = [
        Bitset::new(&[true, true, true, false, true, true, false, false]),
        Bitset::new(&[false, false, false, true, false, false, false, true]),
    ];
    let mut i = 0;
    while num_data_bits - data.len() >= 8 {
        data.append(&padding[i]);
        i = 1 - i;
    }

    if data.len() != num_data_bits {
        panic!("BUG: got len {}, expected {}", data.len(), num_data_bits);
    }
}

// ---------------------------------------------------------------------------------------------
// PNG, laid out as Go's image/png lays out a two-colour image.Paletted: IHDR (bit depth 1, colour
// type 3), PLTE [white, black], IDAT (zlib), IEND. Rows are written unfiltered.

fn write_png(w: usize, pix: &[bool]) -> Vec<u8> {
    let stride = w.div_ceil(8);
    let mut raw = Vec::with_capacity((stride + 1) * w);
    for y in 0..w {
        raw.push(0u8); // filter: none
        let row = &pix[y * w..(y + 1) * w];
        for chunk in row.chunks(8) {
            let mut b = 0u8;
            for (k, &black) in chunk.iter().enumerate() {
                if black {
                    b |= 0x80 >> k;
                }
            }
            raw.push(b);
        }
    }

    let mut out = Vec::with_capacity(raw.len() / 4 + 128);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&[1, 3, 0, 0, 0]); // bit depth 1, paletted, deflate, no filter set, no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"PLTE", &[0xff, 0xff, 0xff, 0x00, 0x00, 0x00]);
    write_chunk(&mut out, b"IDAT", &zlib_best(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// compress/zlib at BestCompression, as Go's image/png writes the image data: the header 78 DA, the deflate stream
/// of Go's own compress/flate (cards::goflate, byte for byte), the Adler-32. The image is under 32 KiB compressed, so
/// it is one IDAT chunk, as Go's 32 KiB bufio.Writer makes it.
fn zlib_best(raw: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0xda];
    z.extend_from_slice(&crate::cards::goflate::deflate_best(raw));
    let (mut a, mut b) = (1u32, 0u32);
    for &x in raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    z
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
};

/// CRC-32 (IEEE), as PNG chunks carry it.
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    //! Goldens from go-qrcode itself: a scratch Go program ran qrcode.New / Bitmap / PNG over
    //! every case and wrote `qr_golden.txt`, decoding each PNG with image/png. Each line:
    //!
    //! - `case <level> ok <width> <sha256[:24] of Bitmap()> <content>` or
    //!   `case <level> err <message>|<content>`;
    //! - `img <size> <width> <sha256[:24] of the decoded PNG(size) pixels>` for that case;
    //! - `row <0/1...>`: a full Bitmap() dump for that case.
    //!
    //! Hashes are over row-major bytes 0 (white) / 1 (black). Content is `a:<ascii>`, `x:<hex>`
    //! or `g:<set>:<seed>:<n>`, `n` characters from a named set by the LCG in `gen`.

    use super::*;
    use sha2::{Digest, Sha256};

    const GOLDEN: &str = include_str!("qr_golden.txt");

    fn level(s: &str) -> Level {
        match s {
            "0" => Level::Low,
            "1" => Level::Medium,
            "2" => Level::High,
            "3" => Level::Highest,
            _ => panic!("level {s}"),
        }
    }

    fn gen(set: &str, seed: u64, n: usize) -> String {
        let set = match set {
            "L" => "abcdefghijklmnopqrstuvwxyz",
            "D" => "0123456789",
            "A" => "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:",
            "M" => "abcdefghijklmnopqrstuvwxyz0123456789",
            "H" => "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF/",
            "B" => "abcdefghijklmnopqrstuvwxyz234567",
            _ => panic!("set {set}"),
        }
        .as_bytes();
        let mut x = seed;
        let mut s = String::with_capacity(n);
        for _ in 0..n {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            s.push(set[((x >> 33) % set.len() as u64) as usize] as char);
        }
        s
    }

    fn content(spec: &str) -> String {
        if let Some(a) = spec.strip_prefix("a:") {
            a.to_string()
        } else if let Some(x) = spec.strip_prefix("x:") {
            String::from_utf8(hex::decode(x).unwrap()).unwrap()
        } else if let Some(g) = spec.strip_prefix("g:") {
            let p: Vec<&str> = g.split(':').collect();
            gen(p[0], p[1].parse().unwrap(), p[2].parse().unwrap())
        } else {
            panic!("content spec {spec}")
        }
    }

    fn hash(bits: impl Iterator<Item = bool>) -> String {
        let bytes: Vec<u8> = bits.map(u8::from).collect();
        hex::encode(Sha256::digest(&bytes))[..24].to_string()
    }

    /// A minimal PNG reader for what `png` writes (and any 1-bit greyscale-or-paletted PNG with
    /// a two-entry palette): chunk CRCs, IHDR, PLTE, zlib IDAT, all five row filters, 1-bit
    /// unpacking. Returns the width and the pixels, true = black (palette entry 0,0,0).
    fn read_png(png: &[u8]) -> (usize, Vec<bool>) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let mut p = 8;
        let (mut w, mut h) = (0usize, 0usize);
        let mut palette: Vec<[u8; 3]> = Vec::new();
        let mut idat = Vec::new();
        let mut kinds = Vec::new();
        while p < png.len() {
            let len = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
            let kind = &png[p + 4..p + 8];
            let data = &png[p + 8..p + 8 + len];
            let crc = u32::from_be_bytes(png[p + 8 + len..p + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&png[p + 4..p + 8 + len]), "chunk crc");
            kinds.push(String::from_utf8(kind.to_vec()).unwrap());
            match kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                    h = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
                    assert_eq!(&data[8..13], &[1, 3, 0, 0, 0], "bit depth 1, colour type 3");
                }
                b"PLTE" => palette = data.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            p += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "PLTE", "IDAT", "IEND"]);
        assert_eq!(palette, vec![[0xff, 0xff, 0xff], [0, 0, 0]], "palette [white, black]");
        assert_eq!(w, h);
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).expect("zlib");
        let stride = w.div_ceil(8);
        assert_eq!(raw.len(), (stride + 1) * h);
        let mut prev = vec![0u8; stride];
        let mut pix = Vec::with_capacity(w * h);
        for y in 0..h {
            let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
            let mut cur = line[1..].to_vec();
            for i in 0..stride {
                let a = if i > 0 { cur[i - 1] } else { 0 };
                let b = prev[i];
                let c = if i > 0 { prev[i - 1] } else { 0 };
                cur[i] = match line[0] {
                    0 => cur[i],
                    1 => cur[i].wrapping_add(a),
                    2 => cur[i].wrapping_add(b),
                    3 => cur[i].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                    4 => {
                        let pa = (b as i16 - c as i16).abs();
                        let pb = (a as i16 - c as i16).abs();
                        let pc = (a as i16 + b as i16 - 2 * c as i16).abs();
                        let pr = if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c };
                        cur[i].wrapping_add(pr)
                    }
                    f => panic!("filter {f}"),
                };
            }
            for x in 0..w {
                let idx = (cur[x / 8] >> (7 - x % 8)) & 1;
                pix.push(palette[idx as usize] == [0, 0, 0]);
            }
            prev = cur;
        }
        (w, pix)
    }

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn gf_tables_match_go() {
        // Spot values from go-qrcode's gfExpTable / gfLogTable.
        assert_eq!(&GF_EXP_TABLE[..12], &[1, 2, 4, 8, 16, 32, 64, 128, 29, 58, 116, 232]);
        assert_eq!(GF_EXP_TABLE[254], 142);
        assert_eq!(GF_EXP_TABLE[255], 1);
        assert_eq!(&GF_LOG_TABLE[..10], &[-1, 0, 1, 25, 2, 50, 26, 198, 3, 223]);
    }

    #[test]
    fn version_table_shape() {
        for (i, v) in VERSIONS.iter().enumerate() {
            assert_eq!(v.version, i / 4 + 1);
            let total: usize = v.block.iter().map(|&(n, c, _)| n * c).sum();
            let s = v.symbol_size();
            // every module that is not a function pattern holds a codeword bit or a remainder bit
            assert!(total * 8 + v.num_remainder_bits < s * s);
        }
    }

    #[test]
    fn matches_go_qrcode_goldens() {
        let mut cases = 0;
        let mut images = 0;
        let mut dumps = 0;
        let mut errors = 0;
        let mut cur: Option<Qr> = None;
        let mut dump_row = 0;
        for (ln, line) in GOLDEN.lines().enumerate() {
            let at = format!("qr_golden.txt:{}", ln + 1);
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("case ") {
                cases += 1;
                dump_row = 0;
                let (lv, rest) = rest.split_once(' ').unwrap();
                let lv = level(lv);
                if let Some(rest) = rest.strip_prefix("err ") {
                    let (msg, spec) = rest.split_once('|').unwrap();
                    let c = content(spec);
                    let got = Qr::new(&c, lv).err();
                    assert_eq!(got.as_deref(), Some(msg), "{at}");
                    assert_eq!(encode_png(&c, lv, 320).err().as_deref(), Some(msg), "{at}");
                    errors += 1;
                    cur = None;
                } else {
                    let rest = rest.strip_prefix("ok ").unwrap();
                    let mut it = rest.splitn(3, ' ');
                    let width: usize = it.next().unwrap().parse().unwrap();
                    let bm_hash = it.next().unwrap();
                    let c = content(it.next().unwrap());
                    let q = Qr::new(&c, lv).unwrap_or_else(|e| panic!("{at}: {e}"));
                    let bm = q.bitmap();
                    assert_eq!(bm.len(), width, "{at}: width");
                    assert!(bm.iter().all(|r| r.len() == width));
                    assert_eq!(hash(bm.iter().flatten().copied()), bm_hash, "{at}: bitmap ({:?})", lv);
                    cur = Some(q);
                }
            } else if let Some(rest) = line.strip_prefix("img ") {
                let q = cur.as_ref().unwrap();
                let p: Vec<&str> = rest.split(' ').collect();
                let size: i32 = p[0].parse().unwrap();
                let width: usize = p[1].parse().unwrap();
                let (w, pix) = q.image(size);
                assert_eq!(w, width, "{at}: image width");
                assert_eq!(hash(pix.iter().copied()), p[2], "{at}: image pixels");
                let (pw, ppix) = read_png(&q.png(size));
                assert_eq!((pw, &ppix), (w, &pix), "{at}: png pixels");
                images += 1;
            } else if let Some(row) = line.strip_prefix("row ") {
                let q = cur.as_ref().unwrap();
                let want: Vec<bool> = row.bytes().map(|b| b == b'1').collect();
                assert_eq!(q.bitmap()[dump_row], want, "{at}: bitmap row {dump_row}");
                dump_row += 1;
                dumps += 1;
            } else {
                panic!("{at}: unknown line");
            }
        }
        eprintln!("qr goldens: {cases} cases ({errors} errors), {images} images, {dumps} dumped rows");
        assert!(cases > 500 && errors >= 5 && images > 700 && dumps > 0);
    }

    #[test]
    fn encode_png_round_trip() {
        let png = encode_png("https://beatscript.app/pair#abc.def", Level::Medium, 320).unwrap();
        let q = Qr::new("https://beatscript.app/pair#abc.def", Level::Medium).unwrap();
        assert_eq!(read_png(&png), q.image(320));
        assert_eq!(read_png(&png).0, 320);
    }

    /// The pairing QR exactly as Go's server sends it: go-qrcode `Encode(link, Medium, 320)` through image/png and
    /// compress/flate, the same bytes (qr_png_golden.txt, printed by go-qrcode v0.0.0-20200617195104 on Go 1.26).
    #[test]
    fn pairing_png_bytes_are_go_s() {
        use base64::Engine;
        let mut n = 0;
        for line in include_str!("qr_png_golden.txt").lines() {
            let (link, b64) = line.split_once(' ').unwrap();
            let want = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
            assert_eq!(encode_png(link, Level::Medium, 320).unwrap(), want, "{link}");
            n += 1;
        }
        assert_eq!(n, 4);
    }
}
