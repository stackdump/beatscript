//! Byte-exact port of Go's `compress/flate` compressor at level 9
//! (`flate.BestCompression`), as of Go 1.26.
//!
//! `deflate_best(data)` returns exactly the bytes Go produces for
//! `flate.NewWriter(&buf, flate.BestCompression)`, one `Write(data)`, then
//! `Close()`. Only that path is ported: the lazy-matching `deflate` step with
//! hash chains, `huffmanBitWriter.writeBlock` with its stored/fixed/dynamic
//! choice, and `close()`'s final empty stored block. No dictionary, no Reset,
//! no other levels.
//!
//! Go's bit writer buffers 48 bits and 240 bytes before writing; that
//! buffering does not change the byte stream, so this port emits bytes as soon
//! as they are complete.

const LOG_WINDOW_SIZE: usize = 15;
const WINDOW_SIZE: usize = 1 << LOG_WINDOW_SIZE;
const WINDOW_MASK: usize = WINDOW_SIZE - 1;

const BASE_MATCH_LENGTH: i64 = 3;
const MIN_MATCH_LENGTH: i64 = 4;
const MAX_MATCH_LENGTH: i64 = 258;
const BASE_MATCH_OFFSET: i64 = 1;

const MAX_FLATE_BLOCK_TOKENS: usize = 1 << 14;
const MAX_STORE_BLOCK_SIZE: usize = 65535;
const HASH_BITS: u32 = 17;
const HASH_SIZE: usize = 1 << HASH_BITS;
const HASH_MASK: u32 = (1 << HASH_BITS) - 1;
const MAX_HASH_OFFSET: i64 = 1 << 24;

// Level 9 parameters: good, lazy, nice, chain (fastSkipHashing = skipNever).
const GOOD: i64 = 32;
const LAZY: i64 = 258;
const NICE: i64 = 258;
const CHAIN: i64 = 4096;

const HASHMUL: u32 = 0x1e35a7bd;

// token layout (token.go)
const LENGTH_SHIFT: u32 = 22;
const OFFSET_MASK: u32 = (1 << LENGTH_SHIFT) - 1;
const MATCH_TYPE: u32 = 1 << 30;

const MAX_NUM_LIT: usize = 286;
const OFFSET_CODE_COUNT: usize = 30;
const END_BLOCK_MARKER: u32 = 256;
const LENGTH_CODES_START: usize = 257;
const CODEGEN_CODE_COUNT: usize = 19;
const BAD_CODE: u8 = 255;

const LENGTH_EXTRA_BITS: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const LENGTH_BASE: [u32; 29] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128,
    160, 192, 224, 255,
];
const OFFSET_EXTRA_BITS: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const OFFSET_BASE: [u32; 30] = [
    0x000000, 0x000001, 0x000002, 0x000003, 0x000004, 0x000006, 0x000008, 0x00000c, 0x000010,
    0x000018, 0x000020, 0x000030, 0x000040, 0x000060, 0x000080, 0x0000c0, 0x000100, 0x000180,
    0x000200, 0x000300, 0x000400, 0x000600, 0x000800, 0x000c00, 0x001000, 0x001800, 0x002000,
    0x003000, 0x004000, 0x006000,
];
const CODEGEN_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Go's `lengthCodes[len]` for `len = length - 3` in 0..=255: the largest
/// code whose base does not exceed it (255, i.e. length 258, is code 28).
fn length_code(len: u32) -> u32 {
    let mut c = 28;
    while LENGTH_BASE[c] > len {
        c -= 1;
    }
    c as u32
}

/// Go's `offsetCode(off)` for `off = distance - 1`.
fn offset_code(off: u32) -> u32 {
    let mut c = 29;
    while OFFSET_BASE[c] > off {
        c -= 1;
    }
    c as u32
}

fn hash4(b: &[u8]) -> u32 {
    let v = (b[3] as u32) | (b[2] as u32) << 8 | (b[1] as u32) << 16 | (b[0] as u32) << 24;
    v.wrapping_mul(HASHMUL) >> (32 - HASH_BITS)
}

fn reverse_bits(number: u16, bit_length: u32) -> u16 {
    (number << (16 - bit_length)).reverse_bits()
}

// ---------------------------------------------------------------- huffman_code.go

#[derive(Clone, Copy, Default)]
struct HCode {
    code: u16,
    len: u16,
}

#[derive(Clone, Copy)]
struct LiteralNode {
    literal: u16,
    freq: i32,
}

#[derive(Clone, Copy, Default)]
struct LevelInfo {
    level: i32,
    last_freq: i32,
    next_char_freq: i32,
    next_pair_freq: i32,
    needed: i32,
}

const MAX_BITS_LIMIT: usize = 16;

struct HuffmanEncoder {
    codes: Vec<HCode>,
}

impl HuffmanEncoder {
    fn new(size: usize) -> Self {
        HuffmanEncoder {
            codes: vec![HCode::default(); size],
        }
    }

    fn fixed_literal() -> Self {
        let mut h = Self::new(MAX_NUM_LIT);
        for ch in 0..MAX_NUM_LIT as u16 {
            let (bits, size) = if ch < 144 {
                (ch + 48, 8)
            } else if ch < 256 {
                (ch + 400 - 144, 9)
            } else if ch < 280 {
                (ch - 256, 7)
            } else {
                (ch + 192 - 280, 8)
            };
            h.codes[ch as usize] = HCode {
                code: reverse_bits(bits, size as u32),
                len: size,
            };
        }
        h
    }

    fn fixed_offset() -> Self {
        let mut h = Self::new(30);
        for ch in 0..30u16 {
            h.codes[ch as usize] = HCode {
                code: reverse_bits(ch, 5),
                len: 5,
            };
        }
        h
    }

    fn bit_length(&self, freq: &[i32]) -> i64 {
        let mut total = 0i64;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                total += f as i64 * self.codes[i].len as i64;
            }
        }
        total
    }

    /// `list` is sorted by increasing frequency and has at least 3 entries.
    fn bit_counts(list: &mut Vec<LiteralNode>, mut max_bits: i32) -> [i32; 17] {
        assert!(
            (max_bits as usize) < MAX_BITS_LIMIT,
            "flate: maxBits too large"
        );
        let n = list.len() as i32;
        list.push(LiteralNode {
            literal: u16::MAX,
            freq: i32::MAX,
        });

        if max_bits > n - 1 {
            max_bits = n - 1;
        }

        let mut levels = [LevelInfo::default(); MAX_BITS_LIMIT + 1];
        let mut leaf_counts = [[0i32; MAX_BITS_LIMIT]; MAX_BITS_LIMIT];

        for level in 1..=max_bits {
            let l = level as usize;
            levels[l] = LevelInfo {
                level,
                last_freq: list[1].freq,
                next_char_freq: list[2].freq,
                next_pair_freq: list[0].freq + list[1].freq,
                needed: 0,
            };
            leaf_counts[l][l] = 2;
            if level == 1 {
                levels[l].next_pair_freq = i32::MAX;
            }
        }

        levels[max_bits as usize].needed = 2 * n - 4;

        let mut level = max_bits as usize;
        loop {
            if levels[level].next_pair_freq == i32::MAX && levels[level].next_char_freq == i32::MAX
            {
                levels[level].needed = 0;
                levels[level + 1].next_pair_freq = i32::MAX;
                level += 1;
                continue;
            }

            let prev_freq = levels[level].last_freq;
            if levels[level].next_char_freq < levels[level].next_pair_freq {
                let nn = leaf_counts[level][level] + 1;
                levels[level].last_freq = levels[level].next_char_freq;
                leaf_counts[level][level] = nn;
                levels[level].next_char_freq = list[nn as usize].freq;
            } else {
                levels[level].last_freq = levels[level].next_pair_freq;
                for j in 0..level {
                    leaf_counts[level][j] = leaf_counts[level - 1][j];
                }
                let ll = levels[level].level as usize;
                levels[ll - 1].needed = 2;
            }

            levels[level].needed -= 1;
            if levels[level].needed == 0 {
                if levels[level].level == max_bits {
                    break;
                }
                let ll = levels[level].level as usize;
                levels[ll + 1].next_pair_freq = prev_freq + levels[level].last_freq;
                level += 1;
            } else {
                while levels[level - 1].needed > 0 {
                    level -= 1;
                }
            }
        }

        let mb = max_bits as usize;
        assert!(
            leaf_counts[mb][mb] == n,
            "leafCounts[maxBits][maxBits] != n"
        );

        let mut bit_count = [0i32; 17];
        let mut bits = 1usize;
        let counts = &leaf_counts[mb];
        let mut lv = mb;
        while lv > 0 {
            bit_count[bits] = counts[lv] - counts[lv - 1];
            bits += 1;
            lv -= 1;
        }
        bit_count
    }

    fn assign_encoding_and_size(&mut self, bit_count: &[i32], list: &[LiteralNode]) {
        let mut code: u16 = 0;
        let mut list_len = list.len();
        for (n, &bits) in bit_count.iter().enumerate() {
            code <<= 1;
            if n == 0 || bits == 0 {
                continue;
            }
            let mut chunk: Vec<LiteralNode> = list[list_len - bits as usize..list_len].to_vec();
            chunk.sort_unstable_by_key(|node| node.literal);
            for node in &chunk {
                self.codes[node.literal as usize] = HCode {
                    code: reverse_bits(code, n as u32),
                    len: n as u16,
                };
                code = code.wrapping_add(1);
            }
            list_len -= bits as usize;
        }
    }

    fn generate(&mut self, freq: &[i32], max_bits: i32) {
        let mut list: Vec<LiteralNode> = Vec::with_capacity(freq.len() + 1);
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                list.push(LiteralNode {
                    literal: i as u16,
                    freq: f,
                });
            } else {
                self.codes[i].len = 0;
            }
        }
        if list.len() <= 2 {
            for (i, node) in list.iter().enumerate() {
                let c = &mut self.codes[node.literal as usize];
                c.code = i as u16;
                c.len = 1;
            }
            return;
        }
        // (freq, literal) is a total order over unique literals, so any sort
        // agrees with Go's sort.Sort.
        list.sort_unstable_by(|a, b| (a.freq, a.literal).cmp(&(b.freq, b.literal)));
        let bc = Self::bit_counts(&mut list, max_bits);
        let n = list.len() - 1; // drop the sentinel pushed by bit_counts
        let mb = (max_bits as usize).min(n - 1);
        self.assign_encoding_and_size(&bc[..mb + 1], &list[..n]);
    }
}

// ---------------------------------------------------------- huffman_bit_writer.go

struct BitWriter {
    out: Vec<u8>,
    bits: u64,
    nbits: u32,
    codegen_freq: [i32; CODEGEN_CODE_COUNT],
    literal_freq: [i32; MAX_NUM_LIT],
    offset_freq: [i32; OFFSET_CODE_COUNT],
    codegen: Vec<u8>,
    literal_encoding: HuffmanEncoder,
    offset_encoding: HuffmanEncoder,
    codegen_encoding: HuffmanEncoder,
    fixed_literal: HuffmanEncoder,
    fixed_offset: HuffmanEncoder,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            bits: 0,
            nbits: 0,
            codegen_freq: [0; CODEGEN_CODE_COUNT],
            literal_freq: [0; MAX_NUM_LIT],
            offset_freq: [0; OFFSET_CODE_COUNT],
            codegen: vec![0; MAX_NUM_LIT + OFFSET_CODE_COUNT + 1],
            literal_encoding: HuffmanEncoder::new(MAX_NUM_LIT),
            offset_encoding: HuffmanEncoder::new(OFFSET_CODE_COUNT),
            codegen_encoding: HuffmanEncoder::new(CODEGEN_CODE_COUNT),
            fixed_literal: HuffmanEncoder::fixed_literal(),
            fixed_offset: HuffmanEncoder::fixed_offset(),
        }
    }

    fn flush(&mut self) {
        while self.nbits != 0 {
            self.out.push(self.bits as u8);
            self.bits >>= 8;
            self.nbits = self.nbits.saturating_sub(8);
        }
        self.bits = 0;
    }

    fn write_bits(&mut self, b: u32, nb: u32) {
        self.bits |= (b as u64) << self.nbits;
        self.nbits += nb;
        while self.nbits >= 8 {
            self.out.push(self.bits as u8);
            self.bits >>= 8;
            self.nbits -= 8;
        }
    }

    fn write_code(&mut self, c: HCode) {
        self.write_bits(c.code as u32, c.len as u32);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        // write_bits keeps fewer than 8 bits pending, and callers only get
        // here byte-aligned (after write_stored_header).
        debug_assert!(self.nbits == 0, "writeBytes with unfinished bits");
        self.out.extend_from_slice(bytes);
    }

    fn generate_codegen(&mut self, num_literals: usize, num_offsets: usize) {
        self.codegen_freq = [0; CODEGEN_CODE_COUNT];
        let mut input: Vec<u8> = Vec::with_capacity(num_literals + num_offsets + 1);
        for i in 0..num_literals {
            input.push(self.literal_encoding.codes[i].len as u8);
        }
        for i in 0..num_offsets {
            input.push(self.offset_encoding.codes[i].len as u8);
        }
        input.push(BAD_CODE);

        let codegen = &mut self.codegen;
        let freq = &mut self.codegen_freq;
        let mut size = input[0];
        let mut count: i32 = 1;
        let mut out_index = 0usize;
        let mut in_index = 1usize;
        while size != BAD_CODE {
            let next_size = input[in_index];
            in_index += 1;
            if next_size == size {
                count += 1;
                continue;
            }
            if size != 0 {
                codegen[out_index] = size;
                out_index += 1;
                freq[size as usize] += 1;
                count -= 1;
                while count >= 3 {
                    let n = count.min(6);
                    codegen[out_index] = 16;
                    out_index += 1;
                    codegen[out_index] = (n - 3) as u8;
                    out_index += 1;
                    freq[16] += 1;
                    count -= n;
                }
            } else {
                while count >= 11 {
                    let n = count.min(138);
                    codegen[out_index] = 18;
                    out_index += 1;
                    codegen[out_index] = (n - 11) as u8;
                    out_index += 1;
                    freq[18] += 1;
                    count -= n;
                }
                if count >= 3 {
                    codegen[out_index] = 17;
                    out_index += 1;
                    codegen[out_index] = (count - 3) as u8;
                    out_index += 1;
                    freq[17] += 1;
                    count = 0;
                }
            }
            count -= 1;
            while count >= 0 {
                codegen[out_index] = size;
                out_index += 1;
                freq[size as usize] += 1;
                count -= 1;
            }
            size = next_size;
            count = 1;
        }
        codegen[out_index] = BAD_CODE;
    }

    fn dynamic_size(&self, extra_bits: i64) -> (i64, usize) {
        let mut num_codegens = self.codegen_freq.len();
        while num_codegens > 4 && self.codegen_freq[CODEGEN_ORDER[num_codegens - 1]] == 0 {
            num_codegens -= 1;
        }
        let header = 3
            + 5
            + 5
            + 4
            + 3 * num_codegens as i64
            + self.codegen_encoding.bit_length(&self.codegen_freq)
            + self.codegen_freq[16] as i64 * 2
            + self.codegen_freq[17] as i64 * 3
            + self.codegen_freq[18] as i64 * 7;
        let size = header
            + self.literal_encoding.bit_length(&self.literal_freq)
            + self.offset_encoding.bit_length(&self.offset_freq)
            + extra_bits;
        (size, num_codegens)
    }

    fn fixed_size(&self, extra_bits: i64) -> i64 {
        3 + self.fixed_literal.bit_length(&self.literal_freq)
            + self.fixed_offset.bit_length(&self.offset_freq)
            + extra_bits
    }

    fn stored_size(input: Option<&[u8]>) -> (i64, bool) {
        match input {
            Some(b) if b.len() <= MAX_STORE_BLOCK_SIZE => (((b.len() + 5) * 8) as i64, true),
            _ => (0, false),
        }
    }

    fn write_dynamic_header(
        &mut self,
        num_literals: usize,
        num_offsets: usize,
        num_codegens: usize,
        is_eof: bool,
    ) {
        self.write_bits(if is_eof { 5 } else { 4 }, 3);
        self.write_bits((num_literals - 257) as u32, 5);
        self.write_bits((num_offsets - 1) as u32, 5);
        self.write_bits((num_codegens - 4) as u32, 4);
        for i in 0..num_codegens {
            let value = self.codegen_encoding.codes[CODEGEN_ORDER[i]].len as u32;
            self.write_bits(value, 3);
        }
        let mut i = 0;
        loop {
            let code_word = self.codegen[i];
            i += 1;
            if code_word == BAD_CODE {
                break;
            }
            let c = self.codegen_encoding.codes[code_word as usize];
            self.write_code(c);
            match code_word {
                16 => {
                    let v = self.codegen[i] as u32;
                    self.write_bits(v, 2);
                    i += 1;
                }
                17 => {
                    let v = self.codegen[i] as u32;
                    self.write_bits(v, 3);
                    i += 1;
                }
                18 => {
                    let v = self.codegen[i] as u32;
                    self.write_bits(v, 7);
                    i += 1;
                }
                _ => {}
            }
        }
    }

    fn write_stored_header(&mut self, length: usize, is_eof: bool) {
        self.write_bits(if is_eof { 1 } else { 0 }, 3);
        self.flush();
        self.write_bits(length as u32 & 0xffff, 16);
        self.write_bits(!(length as u16) as u32, 16);
    }

    fn write_fixed_header(&mut self, is_eof: bool) {
        self.write_bits(if is_eof { 3 } else { 2 }, 3);
    }

    fn index_tokens(&mut self, tokens: &[u32]) -> (usize, usize) {
        self.literal_freq = [0; MAX_NUM_LIT];
        self.offset_freq = [0; OFFSET_CODE_COUNT];
        for &t in tokens {
            if t < MATCH_TYPE {
                self.literal_freq[t as usize] += 1;
                continue;
            }
            let length = (t - MATCH_TYPE) >> LENGTH_SHIFT;
            let offset = t & OFFSET_MASK;
            self.literal_freq[LENGTH_CODES_START + length_code(length) as usize] += 1;
            self.offset_freq[offset_code(offset) as usize] += 1;
        }
        let mut num_literals = self.literal_freq.len();
        while self.literal_freq[num_literals - 1] == 0 {
            num_literals -= 1;
        }
        let mut num_offsets = self.offset_freq.len();
        while num_offsets > 0 && self.offset_freq[num_offsets - 1] == 0 {
            num_offsets -= 1;
        }
        if num_offsets == 0 {
            self.offset_freq[0] = 1;
            num_offsets = 1;
        }
        let lf = self.literal_freq;
        let of = self.offset_freq;
        self.literal_encoding.generate(&lf, 15);
        self.offset_encoding.generate(&of, 15);
        (num_literals, num_offsets)
    }

    fn write_tokens(&mut self, tokens: &[u32], fixed: bool) {
        for &t in tokens {
            let (le, oe) = if fixed {
                (&self.fixed_literal, &self.fixed_offset)
            } else {
                (&self.literal_encoding, &self.offset_encoding)
            };
            if t < MATCH_TYPE {
                let c = le.codes[t as usize];
                self.write_code(c);
                continue;
            }
            let length = (t - MATCH_TYPE) >> LENGTH_SHIFT;
            let lc = length_code(length) as usize;
            let c = le.codes[lc + LENGTH_CODES_START];
            let offset = t & OFFSET_MASK;
            let oc = offset_code(offset) as usize;
            let oc_code = oe.codes[oc];
            self.write_code(c);
            let elb = LENGTH_EXTRA_BITS[lc] as u32;
            if elb > 0 {
                self.write_bits(length - LENGTH_BASE[lc], elb);
            }
            self.write_code(oc_code);
            let eob = OFFSET_EXTRA_BITS[oc] as u32;
            if eob > 0 {
                self.write_bits(offset - OFFSET_BASE[oc], eob);
            }
        }
    }

    /// `huffmanBitWriter.writeBlock`. `tokens` must not yet include the
    /// end-of-block marker. `input` is `None` where Go passes a nil window.
    fn write_block(&mut self, tokens: &mut Vec<u32>, eof: bool, input: Option<&[u8]>) {
        tokens.push(END_BLOCK_MARKER);
        let (num_literals, num_offsets) = self.index_tokens(tokens);

        let mut extra_bits: i64 = 0;
        let (stored_size, storable) = Self::stored_size(input);
        if storable {
            for lc in (LENGTH_CODES_START + 8)..num_literals {
                extra_bits += self.literal_freq[lc] as i64
                    * LENGTH_EXTRA_BITS[lc - LENGTH_CODES_START] as i64;
            }
            for oc in 4..num_offsets {
                extra_bits += self.offset_freq[oc] as i64 * OFFSET_EXTRA_BITS[oc] as i64;
            }
        }

        let mut use_fixed = true;
        let mut size = self.fixed_size(extra_bits);

        self.generate_codegen(num_literals, num_offsets);
        let cf = self.codegen_freq;
        self.codegen_encoding.generate(&cf, 7);
        let (dynamic_size, num_codegens) = self.dynamic_size(extra_bits);

        if dynamic_size < size {
            size = dynamic_size;
            use_fixed = false;
        }

        if storable && stored_size < size {
            let input = input.unwrap_or(&[]);
            self.write_stored_header(input.len(), eof);
            self.write_bytes(input);
            tokens.pop();
            return;
        }

        if use_fixed {
            self.write_fixed_header(eof);
        } else {
            self.write_dynamic_header(num_literals, num_offsets, num_codegens, eof);
        }
        self.write_tokens(tokens, use_fixed);
        tokens.pop();
    }
}

// --------------------------------------------------------------------- deflate.go

struct Compressor {
    w: BitWriter,
    index: i64,
    window: Vec<u8>,
    window_end: i64,
    block_start: i64,
    byte_available: bool,
    sync: bool,
    tokens: Vec<u32>,
    length: i64,
    offset: i64,
    max_insert_index: i64,
    chain_head: i64,
    hash_head: Vec<u32>,
    hash_prev: Vec<u32>,
    hash_offset: i64,
}

impl Compressor {
    fn new() -> Self {
        Compressor {
            w: BitWriter::new(),
            index: 0,
            window: vec![0; 2 * WINDOW_SIZE],
            window_end: 0,
            block_start: 0,
            byte_available: false,
            sync: false,
            tokens: Vec::with_capacity(MAX_FLATE_BLOCK_TOKENS + 1),
            length: MIN_MATCH_LENGTH - 1,
            offset: 0,
            max_insert_index: 0,
            chain_head: -1,
            hash_head: vec![0; HASH_SIZE],
            hash_prev: vec![0; WINDOW_SIZE],
            hash_offset: 1,
        }
    }

    fn fill_deflate(&mut self, b: &[u8]) -> usize {
        let ws = WINDOW_SIZE as i64;
        if self.index >= 2 * ws - (MIN_MATCH_LENGTH + MAX_MATCH_LENGTH) {
            self.window.copy_within(WINDOW_SIZE..2 * WINDOW_SIZE, 0);
            self.index -= ws;
            self.window_end -= ws;
            if self.block_start >= ws {
                self.block_start -= ws;
            } else {
                self.block_start = i32::MAX as i64;
            }
            self.hash_offset += ws;
            if self.hash_offset > MAX_HASH_OFFSET {
                let delta = self.hash_offset - 1;
                self.hash_offset -= delta;
                self.chain_head -= delta;
                for v in self.hash_prev.iter_mut().chain(self.hash_head.iter_mut()) {
                    if *v as i64 > delta {
                        *v = (*v as i64 - delta) as u32;
                    } else {
                        *v = 0;
                    }
                }
            }
        }
        let we = self.window_end as usize;
        let n = b.len().min(self.window.len() - we);
        self.window[we..we + n].copy_from_slice(&b[..n]);
        self.window_end += n as i64;
        n
    }

    fn write_block(&mut self, index: i64) {
        if index > 0 {
            let window = if self.block_start <= index {
                Some(&self.window[self.block_start as usize..index as usize])
            } else {
                None
            };
            self.block_start = index;
            self.w.write_block(&mut self.tokens, false, window);
        }
    }

    fn find_match(
        &self,
        pos: i64,
        prev_head: i64,
        prev_length: i64,
        lookahead: i64,
    ) -> (i64, i64, bool) {
        let min_match_look = MAX_MATCH_LENGTH.min(lookahead);
        let win = &self.window[..(pos + min_match_look) as usize];
        let nice = NICE.min(win.len() as i64 - pos);

        let mut tries = CHAIN;
        let mut length = prev_length;
        let mut offset = 0;
        let mut ok = false;
        if length >= GOOD {
            tries >>= 2;
        }

        let mut w_end = win[(pos + length) as usize];
        let w_pos = &win[pos as usize..];
        let min_index = pos - WINDOW_SIZE as i64;

        let mut i = prev_head;
        while tries > 0 {
            if w_end == win[(i + length) as usize] {
                let a = &win[i as usize..i as usize + min_match_look as usize];
                let b = &w_pos[..min_match_look as usize];
                let n = a.iter().zip(b).take_while(|(x, y)| x == y).count() as i64;
                if n > length && (n > MIN_MATCH_LENGTH || pos - i <= 4096) {
                    length = n;
                    offset = pos - i;
                    ok = true;
                    if n >= nice {
                        break;
                    }
                    w_end = win[(pos + n) as usize];
                }
            }
            if i == min_index {
                break;
            }
            i = self.hash_prev[i as usize & WINDOW_MASK] as i64 - self.hash_offset;
            if i < min_index || i < 0 {
                break;
            }
            tries -= 1;
        }
        (length, offset, ok)
    }

    fn insert_hash(&mut self, index: i64) {
        let i = index as usize;
        let hash = hash4(&self.window[i..i + MIN_MATCH_LENGTH as usize]);
        let hh = &mut self.hash_head[(hash & HASH_MASK) as usize];
        self.hash_prev[i & WINDOW_MASK] = *hh;
        *hh = (index + self.hash_offset) as u32;
    }

    fn deflate(&mut self) {
        if self.window_end - self.index < MIN_MATCH_LENGTH + MAX_MATCH_LENGTH && !self.sync {
            return;
        }
        self.max_insert_index = self.window_end - (MIN_MATCH_LENGTH - 1);

        loop {
            assert!(self.index <= self.window_end, "index > windowEnd");
            let lookahead = self.window_end - self.index;
            if lookahead < MIN_MATCH_LENGTH + MAX_MATCH_LENGTH {
                if !self.sync {
                    break;
                }
                if lookahead == 0 {
                    if self.byte_available {
                        let lit = self.window[(self.index - 1) as usize] as u32;
                        self.tokens.push(lit);
                        self.byte_available = false;
                    }
                    if !self.tokens.is_empty() {
                        self.write_block(self.index);
                        self.tokens.clear();
                    }
                    break;
                }
            }
            if self.index < self.max_insert_index {
                let i = self.index as usize;
                let hash = hash4(&self.window[i..i + MIN_MATCH_LENGTH as usize]);
                let hh = (hash & HASH_MASK) as usize;
                self.chain_head = self.hash_head[hh] as i64;
                self.hash_prev[i & WINDOW_MASK] = self.chain_head as u32;
                self.hash_head[hh] = (self.index + self.hash_offset) as u32;
            }
            let prev_length = self.length;
            let prev_offset = self.offset;
            self.length = MIN_MATCH_LENGTH - 1;
            self.offset = 0;
            let min_index = (self.index - WINDOW_SIZE as i64).max(0);

            if self.chain_head - self.hash_offset >= min_index
                && lookahead > prev_length
                && prev_length < LAZY
            {
                let (nl, no, ok) = self.find_match(
                    self.index,
                    self.chain_head - self.hash_offset,
                    MIN_MATCH_LENGTH - 1,
                    lookahead,
                );
                if ok {
                    self.length = nl;
                    self.offset = no;
                }
            }
            if prev_length >= MIN_MATCH_LENGTH && self.length <= prev_length {
                let tok = MATCH_TYPE
                    + (((prev_length - BASE_MATCH_LENGTH) as u32) << LENGTH_SHIFT)
                    + (prev_offset - BASE_MATCH_OFFSET) as u32;
                self.tokens.push(tok);
                // fastSkipHashing == skipNever, so every position of the match
                // is inserted.
                let new_index = self.index + prev_length - 1;
                let mut index = self.index + 1;
                while index < new_index {
                    if index < self.max_insert_index {
                        self.insert_hash(index);
                    }
                    index += 1;
                }
                self.index = index;
                self.byte_available = false;
                self.length = MIN_MATCH_LENGTH - 1;
                if self.tokens.len() == MAX_FLATE_BLOCK_TOKENS {
                    self.write_block(self.index);
                    self.tokens.clear();
                }
            } else {
                if self.byte_available {
                    let i = self.index - 1;
                    let lit = self.window[i as usize] as u32;
                    self.tokens.push(lit);
                    if self.tokens.len() == MAX_FLATE_BLOCK_TOKENS {
                        self.write_block(i + 1);
                        self.tokens.clear();
                    }
                }
                self.index += 1;
                self.byte_available = true;
            }
        }
    }
}

/// Compress `data` exactly as Go's `flate.NewWriter(w, flate.BestCompression)`
/// does when given the whole input in a single `Write` followed by `Close`.
pub fn deflate_best(data: &[u8]) -> Vec<u8> {
    let mut d = Compressor::new();
    let mut b = data;
    // compressor.write
    while !b.is_empty() {
        d.deflate();
        let n = d.fill_deflate(b);
        b = &b[n..];
    }
    // compressor.close
    d.sync = true;
    d.deflate();
    d.w.write_stored_header(0, true);
    d.w.flush();
    d.w.out
}

#[cfg(test)]
mod tests {
    use super::deflate_best;
    use sha2::{Digest, Sha256};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn lcg_text(seed: u64, n: usize) -> Vec<u8> {
        let alpha = b"abcd efgh\n~x";
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                alpha[((s >> 33) % alpha.len() as u64) as usize]
            })
            .collect()
    }

    fn lcg_raw(seed: u64, n: usize) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (s >> 33) as u8
            })
            .collect()
    }

    // Expected bytes come from Go 1.26 compress/flate at BestCompression.
    const SMALL: &[(&[u8], &str)] = &[
        (b"", "010000ffff"),
        (b"a", "4a04040000ffff"),
        (b"hello, world", "ca48cdc9c9d75128cf2fca4901040000ffff"),
        (b"abcabcabcabcabcabcabcabc", "4a4c4ac68a00010000ffff"),
        (
            b"The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.\n",
            "0ac94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228c94855c849acaa5448c94fd75320453117200000ffff",
        ),
    ];

    // beats-bitwrap-io btwsite/examples/first-beat.btw, hex-encoded.
    const FIRST_BEAT_HEX: &str = "23212062747720310a2321206e616d653a2066697273742d626561740a2321206c6963656e73653a204343302d312e300a2321207669736962696c6974793a207075626c69630a232066697273742d626561743a206120666f75722d6c696e6520626561742e2041206465636b206c696e65206973206f6e6520626172206f66207369787465656e2073746570733a20782069732061206869742c207e206120726573742c207c206a6f696e732070617274732e0a74656d706f203132340a6465636b20413a206b69636b2078207e207e207e2078207e207e207e2078207e207e207e2078207e207e207e207c20736e617265207e207e207e207e2078207e207e207e207e207e207e207e2078207e207e207e207c206869686174207e207e2078207e207e207e2078207e207e207e2078207e207e207e2078207e2076656c3d36302e2e39300a6465636b20423a2062617373206132207e207e206132207e207e206132207e207e207e206333207e206532207e207e207e20696e73743d7375622d62617373207c206861726d6f6e792063686f726428616d2920696e73743d7761726d2d7061640a656e642061742062617220380a";
    const FIRST_BEAT_DEFLATE: &str = "7c905f4bc34010c4dff329a6f445a1096915d1833ed47e924db2256b93bb70bbfd07259f5d72555444b987617f3b331c3b9fa1b21396d97c064f3d3bec24aae515934dac939abdb2c3765be6cba29cd851542ae9c42e0ec3a1eaa4cee6df620e845d38c4bc13cf9848810d1aaef7484414615a5044d841e56ccc1e6a3ca8c3795a135ab105461022ab2d70c55b10af18289a1699713f042c578f596add38eca5dee38c31bdbff40af514f963fae2e32f5f2b2dd9bf5d931eb95b3f9545f152defef1ea50912a68952c3f6544fd8011fc3989575beba1ca53e48a96621ffc05751b627347fdfdcd71a2d8e7033519fb0664e96acfd97b000000ffff";

    #[test]
    fn small_vectors_match_go() {
        for (input, want) in SMALL {
            assert_eq!(
                hex(&deflate_best(input)),
                *want,
                "input {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn example_btw_matches_go() {
        let input: Vec<u8> = (0..FIRST_BEAT_HEX.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&FIRST_BEAT_HEX[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(input.len(), 438);
        assert_eq!(hex(&deflate_best(&input)), FIRST_BEAT_DEFLATE);
    }

    #[test]
    fn large_inputs_match_go_sha256() {
        // 150 KB of small-alphabet text: dynamic blocks, many matches, crosses
        // the 64 KiB window shift twice.
        let out = deflate_best(&lcg_text(42, 150_000));
        assert_eq!(out.len(), 73545);
        assert_eq!(
            hex(&Sha256::digest(&out)),
            "faa2e52e062b22d9fd42e434c5d8ea06535b1609d7ae1a1bbcf33033d6ef1006"
        );
        // 70 KB of pseudo-random bytes: stored blocks.
        let out = deflate_best(&lcg_raw(2, 70_000));
        assert_eq!(out.len(), 70030);
        assert_eq!(
            hex(&Sha256::digest(&out)),
            "8414175be4471a820315678587f4f84cb36f98bed810a59b68bf7825a9db2f72"
        );
        // Highly repetitive: long matches of length 258.
        let out = deflate_best("x ~ ~ ~ ".repeat(100 * 1024 / 8).as_bytes());
        assert_eq!(
            hex(&Sha256::digest(&out)),
            "dbc99610822348184cbfea87d0100f88b327c471fe0e199014519fa96ebecb13"
        );
    }
}
