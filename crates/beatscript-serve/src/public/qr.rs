//! The QR encoder of github.com/skip2/go-qrcode (v0.0.0-20200617195104-da1b6568686e), ported line for line so a code
//! is the same matrix as Go's: its segmenting of the data into numeric / alphanumeric / byte runs, the smallest
//! version that holds them, the padding, the Reed-Solomon blocks and their interleaving, and its own penalty score
//! for choosing the mask (which is not quite the standard's). Only what the site uses: recovery level Low, and the
//! bitmap with its 4-module quiet zone (`QRCode.Bitmap`).

/// (version, blocks as (count, codewords, data codewords), remainder bits): go-qrcode's `versions`, level Low.
const LOW: &[(usize, &[(usize, usize, usize)], usize)] = &[
    (1, &[(1, 26, 19)], 0),
    (2, &[(1, 44, 34)], 7),
    (3, &[(1, 70, 55)], 7),
    (4, &[(1, 100, 80)], 7),
    (5, &[(1, 134, 108)], 7),
    (6, &[(2, 86, 68)], 7),
    (7, &[(2, 98, 78)], 0),
    (8, &[(2, 121, 97)], 0),
    (9, &[(2, 146, 116)], 0),
    (10, &[(2, 86, 68), (2, 87, 69)], 0),
    (11, &[(4, 101, 81)], 0),
    (12, &[(2, 116, 92), (2, 117, 93)], 0),
    (13, &[(4, 133, 107)], 0),
    (14, &[(3, 145, 115), (1, 146, 116)], 3),
    (15, &[(5, 109, 87), (1, 110, 88)], 3),
    (16, &[(5, 122, 98), (1, 123, 99)], 3),
    (17, &[(1, 135, 107), (5, 136, 108)], 3),
    (18, &[(5, 150, 120), (1, 151, 121)], 3),
    (19, &[(3, 141, 113), (4, 142, 114)], 3),
    (20, &[(3, 135, 107), (5, 136, 108)], 3),
    (21, &[(4, 144, 116), (4, 145, 117)], 4),
    (22, &[(2, 139, 111), (7, 140, 112)], 4),
    (23, &[(4, 151, 121), (5, 152, 122)], 4),
    (24, &[(6, 147, 117), (4, 148, 118)], 4),
    (25, &[(8, 132, 106), (4, 133, 107)], 4),
    (26, &[(10, 142, 114), (2, 143, 115)], 4),
    (27, &[(8, 152, 122), (4, 153, 123)], 4),
    (28, &[(3, 147, 117), (10, 148, 118)], 3),
    (29, &[(7, 146, 116), (7, 147, 117)], 3),
    (30, &[(5, 145, 115), (10, 146, 116)], 3),
    (31, &[(13, 145, 115), (3, 146, 116)], 3),
    (32, &[(17, 145, 115)], 3),
    (33, &[(17, 145, 115), (1, 146, 116)], 3),
    (34, &[(13, 145, 115), (6, 146, 116)], 3),
    (35, &[(12, 151, 121), (7, 152, 122)], 0),
    (36, &[(6, 151, 121), (14, 152, 122)], 0),
    (37, &[(17, 152, 122), (4, 153, 123)], 0),
    (38, &[(4, 152, 122), (18, 153, 123)], 0),
    (39, &[(20, 147, 117), (4, 148, 118)], 0),
    (40, &[(19, 148, 118), (6, 149, 119)], 0),
];

/// go-qrcode's `formatBitSequence[0x08 | mask].regular` (level Low).
const FORMAT_LOW: [u32; 8] = [0x77c4, 0x72f3, 0x7daa, 0x789d, 0x662f, 0x6318, 0x6c41, 0x6976];

const VERSION_BITS: [u32; 41] = [
    0, 0, 0, 0, 0, 0, 0, 0x07c94, 0x085bc, 0x09a99, 0x0a4d3, 0x0bbf6, 0x0c762, 0x0d847, 0x0e60d, 0x0f928, 0x10b78, 0x1145d, 0x12a17, 0x13532,
    0x149a6, 0x15683, 0x168c9, 0x177ec, 0x18ec4, 0x191e1, 0x1afab, 0x1b08e, 0x1cc1a, 0x1d33f, 0x1ed75, 0x1f250, 0x209d5, 0x216f0, 0x228ba,
    0x2379f, 0x24b0b, 0x2542e, 0x26a64, 0x27541, 0x28c69,
];

const ALIGN: [&[usize]; 41] = [
    &[], &[], &[6, 18], &[6, 22], &[6, 26], &[6, 30], &[6, 34], &[6, 22, 38], &[6, 24, 42], &[6, 26, 46], &[6, 28, 50], &[6, 30, 54], &[6, 32, 58],
    &[6, 34, 62], &[6, 26, 46, 66], &[6, 26, 48, 70], &[6, 26, 50, 74], &[6, 30, 54, 78], &[6, 30, 56, 82], &[6, 30, 58, 86], &[6, 34, 62, 90],
    &[6, 28, 50, 72, 94], &[6, 26, 50, 74, 98], &[6, 30, 54, 78, 102], &[6, 28, 54, 80, 106], &[6, 32, 58, 84, 110], &[6, 30, 58, 86, 114],
    &[6, 34, 62, 90, 118], &[6, 26, 50, 74, 98, 122], &[6, 30, 54, 78, 102, 126], &[6, 26, 52, 78, 104, 130], &[6, 30, 56, 82, 108, 134],
    &[6, 34, 60, 86, 112, 138], &[6, 30, 58, 86, 114, 142], &[6, 34, 62, 90, 118, 146], &[6, 30, 54, 78, 102, 126, 150],
    &[6, 24, 50, 76, 102, 128, 154], &[6, 28, 54, 80, 106, 132, 158], &[6, 32, 58, 84, 110, 136, 162], &[6, 26, 54, 82, 110, 138, 166],
    &[6, 30, 58, 86, 114, 142, 170],
];

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Mode {
    None = 1,
    Numeric = 2,
    Alnum = 4,
    Byte = 8,
}

struct Encoder {
    min_version: usize,
    max_version: usize,
    count_bits: [usize; 3], // numeric, alphanumeric, byte
}

fn push_bits(b: &mut Vec<bool>, v: u32, n: usize) {
    for i in (0..n).rev() {
        b.push(v >> i & 1 == 1);
    }
}

impl Encoder {
    fn new(t: usize) -> Encoder {
        match t {
            0 => Encoder { min_version: 1, max_version: 9, count_bits: [10, 9, 8] },
            1 => Encoder { min_version: 10, max_version: 26, count_bits: [12, 11, 16] },
            _ => Encoder { min_version: 27, max_version: 40, count_bits: [14, 13, 16] },
        }
    }

    fn char_bits(&self, m: Mode) -> usize {
        match m {
            Mode::Numeric => self.count_bits[0],
            Mode::Alnum => self.count_bits[1],
            _ => self.count_bits[2],
        }
    }

    fn encoded_length(&self, m: Mode, n: usize) -> Result<usize, &'static str> {
        if m == Mode::None {
            return Err("mode not supported");
        }
        let cb = self.char_bits(m);
        if n > (1usize << cb) - 1 {
            return Err("length too long to be represented");
        }
        let mut len = 4 + cb;
        match m {
            Mode::Numeric => {
                len += 10 * (n / 3);
                if n % 3 != 0 {
                    len += 1 + 3 * (n % 3);
                }
            }
            Mode::Alnum => len += 11 * (n / 2) + 6 * (n % 2),
            _ => len += 8 * n,
        }
        Ok(len)
    }

    fn encode(&self, data: &[u8]) -> Result<Vec<bool>, &'static str> {
        if data.is_empty() {
            return Err("no data to encode");
        }
        // classifyDataModes
        let mut actual: Vec<(Mode, &[u8])> = Vec::new();
        let (mut start, mut mode, mut highest) = (0, Mode::None, Mode::None);
        for (i, &v) in data.iter().enumerate() {
            let m = match v {
                0x30..=0x39 => Mode::Numeric,
                0x20 | 0x24 | 0x25 | 0x2a | 0x2b | 0x2d | 0x2e | 0x2f | 0x3a | 0x41..=0x5a => Mode::Alnum,
                _ => Mode::Byte,
            };
            if m != mode {
                if i > 0 {
                    actual.push((mode, &data[start..i]));
                    start = i;
                }
                mode = m;
            }
            if m > highest {
                highest = m;
            }
        }
        actual.push((mode, &data[start..]));
        // optimiseDataModes
        let mut optimised: Vec<(Mode, Vec<u8>)> = Vec::new();
        let mut i = 0;
        while i < actual.len() {
            let mode = actual[i].0;
            let mut n = actual[i].1.len();
            let mut j = i + 1;
            while j < actual.len() {
                let (nm, nd) = (actual[j].0, actual[j].1.len());
                if nm > mode {
                    break;
                }
                let co = self.encoded_length(mode, n + nd)?;
                let s1 = self.encoded_length(mode, n)?;
                let s2 = self.encoded_length(nm, nd)?;
                if co < s1 + s2 {
                    j += 1;
                    n += nd;
                } else {
                    break;
                }
            }
            let mut d = Vec::with_capacity(n);
            for s in &actual[i..j] {
                d.extend_from_slice(s.1);
            }
            optimised.push((mode, d));
            i = j;
        }
        let mut opt_len = 0;
        for (m, d) in &optimised {
            opt_len += self.encoded_length(*m, d.len())?;
        }
        if self.encoded_length(highest, data.len())? <= opt_len {
            optimised = vec![(highest, data.to_vec())];
        }
        let mut out = Vec::new();
        for (m, d) in &optimised {
            self.encode_raw(d, *m, &mut out);
        }
        Ok(out)
    }

    fn encode_raw(&self, data: &[u8], m: Mode, out: &mut Vec<bool>) {
        push_bits(out, m as u32 >> 1, 4); // 0001 numeric, 0010 alphanumeric, 0100 byte
        push_bits(out, data.len() as u32, self.char_bits(m));
        match m {
            Mode::Numeric => {
                for c in data.chunks(3) {
                    let mut v = 0u32;
                    for d in c {
                        v = v * 10 + u32::from(d - 0x30);
                    }
                    push_bits(out, v, 1 + 3 * c.len());
                }
            }
            Mode::Alnum => {
                for c in data.chunks(2) {
                    let mut v = 0u32;
                    for d in c {
                        v = v * 45 + alnum(*d);
                    }
                    push_bits(out, v, if c.len() > 1 { 11 } else { 6 });
                }
            }
            _ => {
                for b in data {
                    push_bits(out, u32::from(*b), 8);
                }
            }
        }
    }
}

fn alnum(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => u32::from(c - b'0'),
        b'A'..=b'Z' => u32::from(c - b'A') + 10,
        b' ' => 36,
        b'$' => 37,
        b'%' => 38,
        b'*' => 39,
        b'+' => 40,
        b'-' => 41,
        b'.' => 42,
        b'/' => 43,
        _ => 44, // ':'
    }
}

// ---------- Reed-Solomon over GF(2^8), x^8 + x^4 + x^3 + x^2 + 1 ----------

fn gf_tables() -> ([u8; 256], [u8; 256]) {
    let (mut exp, mut log) = ([0u8; 256], [0u8; 256]);
    let mut x: u16 = 1;
    for (i, e) in exp.iter_mut().enumerate().take(255) {
        *e = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
    }
    exp[255] = exp[0];
    (exp, log)
}

/// The `n` error-correction codewords of `data` (the remainder of data·x^n by the generator ∏(x − α^i), i < n).
fn rs_ec(data: &[u8], n: usize) -> Vec<u8> {
    let (exp, log) = gf_tables();
    let mul = |a: u8, b: u8| if a == 0 || b == 0 { 0 } else { exp[(usize::from(log[a as usize]) + usize::from(log[b as usize])) % 255] };
    // generator, highest degree first
    let mut g = vec![1u8];
    for i in 0..n {
        let mut ng = vec![0u8; g.len() + 1];
        for (j, &c) in g.iter().enumerate() {
            ng[j] ^= c;
            ng[j + 1] ^= mul(c, exp[i]);
        }
        g = ng;
    }
    let mut rem = vec![0u8; n];
    for &d in data {
        let f = d ^ rem[0];
        rem.rotate_left(1);
        rem[n - 1] = 0;
        for k in 0..n {
            rem[k] ^= mul(g[k + 1], f);
        }
    }
    rem
}

// ---------- the symbol ----------

struct Symbol {
    module: Vec<Vec<bool>>,
    used: Vec<Vec<bool>>,
    size: usize,
    qz: usize,
}

impl Symbol {
    fn new(size: usize, qz: usize) -> Symbol {
        let n = size + 2 * qz;
        Symbol { module: vec![vec![false; n]; n], used: vec![vec![false; n]; n], size, qz }
    }
    fn get(&self, x: usize, y: usize) -> bool {
        self.module[y + self.qz][x + self.qz]
    }
    fn empty(&self, x: usize, y: usize) -> bool {
        !self.used[y + self.qz][x + self.qz]
    }
    fn set(&mut self, x: usize, y: usize, v: bool) {
        self.module[y + self.qz][x + self.qz] = v;
        self.used[y + self.qz][x + self.qz] = true;
    }
    fn pattern(&mut self, x: usize, y: usize, p: &[&[u8]]) {
        for (j, row) in p.iter().enumerate() {
            for (i, v) in row.iter().enumerate() {
                self.set(x + i, y + j, *v == 1);
            }
        }
    }

    fn penalty(&self) -> i64 {
        let n = self.size;
        let mut p1 = 0i64;
        for outer in 0..2 {
            for a in 0..n {
                let at = |b: usize| if outer == 0 { self.get(a, b) } else { self.get(b, a) };
                let mut last = at(0);
                let mut count = 1;
                for b in 1..n {
                    let v = at(b);
                    if v != last {
                        count = 1;
                        last = v;
                    } else {
                        count += 1;
                        if count == 6 {
                            p1 += 3 + 1;
                        } else if count > 6 {
                            p1 += 1;
                        }
                    }
                }
            }
        }
        let mut p2 = 0i64;
        for y in 1..n {
            for x in 1..n {
                let c = self.get(x, y);
                if c == self.get(x - 1, y) && c == self.get(x, y - 1) && c == self.get(x - 1, y - 1) {
                    p2 += 1;
                }
            }
        }
        let mut p3 = 0i64;
        for outer in 0..2 {
            for a in 0..n {
                let mut buf: u16 = 0;
                for b in 0..n {
                    let v = if outer == 0 { self.get(b, a) } else { self.get(a, b) };
                    buf = buf.wrapping_shl(1);
                    if v {
                        buf |= 1;
                    }
                    match buf & 0x7ff {
                        0x05d | 0x5d0 => {
                            p3 += 40;
                            buf = 0xff;
                        }
                        _ => {
                            if b == n - 1 && buf & 0x7f == 0x5d {
                                p3 += 40;
                                buf = 0xff;
                            }
                        }
                    }
                }
            }
        }
        let total = (n * n) as i64;
        let mut dark = 0i64;
        for x in 0..n {
            for y in 0..n {
                if self.get(x, y) {
                    dark += 1;
                }
            }
        }
        let dev = (total / 2 - dark).abs();
        p1 + p2 * 3 + p3 + 10 * (dev / (total / 20))
    }
}

const FINDER: [&[u8]; 7] = [&[1, 1, 1, 1, 1, 1, 1], &[1, 0, 0, 0, 0, 0, 1], &[1, 0, 1, 1, 1, 0, 1], &[1, 0, 1, 1, 1, 0, 1], &[1, 0, 1, 1, 1, 0, 1], &[1, 0, 0, 0, 0, 0, 1], &[1, 1, 1, 1, 1, 1, 1]];
const HBORDER: [&[u8]; 1] = [&[0, 0, 0, 0, 0, 0, 0, 0]];
const VBORDER: [&[u8]; 8] = [&[0], &[0], &[0], &[0], &[0], &[0], &[0], &[0]];
const ALIGNMENT: [&[u8]; 5] = [&[1, 1, 1, 1, 1], &[1, 0, 0, 0, 1], &[1, 0, 1, 0, 1], &[1, 0, 0, 0, 1], &[1, 1, 1, 1, 1]];

fn build(version: usize, mask: usize, data: &[bool]) -> Symbol {
    let size = 21 + (version - 1) * 4;
    let mut s = Symbol::new(size, 4);
    let fp = 7;
    s.pattern(0, 0, &FINDER);
    s.pattern(0, fp, &HBORDER);
    s.pattern(fp, 0, &VBORDER);
    s.pattern(size - fp, 0, &FINDER);
    s.pattern(size - fp - 1, fp, &HBORDER);
    s.pattern(size - fp - 1, 0, &VBORDER);
    s.pattern(0, size - fp, &FINDER);
    s.pattern(0, size - fp - 1, &HBORDER);
    s.pattern(fp, size - fp - 1, &VBORDER);
    for &x in ALIGN[version] {
        for &y in ALIGN[version] {
            if !s.empty(x, y) {
                continue;
            }
            s.pattern(x - 2, y - 2, &ALIGNMENT);
        }
    }
    let mut v = true;
    for i in fp + 1..size - fp {
        s.set(i, fp - 1, v);
        s.set(fp - 1, i, v);
        v = !v;
    }
    // format info: 15 bits, f.At(i) is bit i from the most significant
    let f = FORMAT_LOW[mask];
    let at = |i: usize| f >> (14 - i) & 1 == 1;
    let l = 14;
    for i in 0..=7 {
        s.set(size - i - 1, fp + 1, at(l - i));
    }
    for i in 0..=5 {
        s.set(fp + 1, i, at(l - i));
    }
    s.set(fp + 1, fp, at(l - 6));
    s.set(fp + 1, fp + 1, at(l - 7));
    s.set(fp, fp + 1, at(l - 8));
    for i in 9..=14 {
        s.set(14 - i, fp + 1, at(l - i));
    }
    for i in 8..=14 {
        s.set(fp + 1, size - fp + i - 8, at(l - i));
    }
    s.set(fp + 1, size - fp - 1, true);
    if version >= 7 {
        let vb = VERSION_BITS[version];
        let vat = |i: usize| vb >> (17 - i) & 1 == 1;
        for i in 0..18 {
            s.set(i / 3, size - fp - 4 + i % 3, vat(17 - i));
            s.set(size - fp - 4 + i % 3, i / 3, vat(17 - i));
        }
    }
    // data
    let (mut x_off, mut up) = (1usize, true);
    let (mut x, mut y) = (size as i64 - 2, size as i64 - 1);
    for (i, &bit) in data.iter().enumerate() {
        let (xx, yy) = ((x as usize) + x_off, y as usize);
        let (xo, yv) = (x as usize + x_off, y as usize);
        let m = match mask {
            0 => (yv + xo) % 2 == 0,
            1 => yv % 2 == 0,
            2 => xo % 3 == 0,
            3 => (yv + xo) % 3 == 0,
            4 => (yv / 2 + xo / 3) % 2 == 0,
            5 => (yv * xo) % 2 + (yv * xo) % 3 == 0,
            6 => ((yv * xo) % 2 + (yv * xo) % 3) % 2 == 0,
            _ => ((yv + xo) % 2 + (yv * xo) % 3) % 2 == 0,
        };
        s.set(xx, yy, m != bit);
        if i == data.len() - 1 {
            break;
        }
        loop {
            if x_off == 1 {
                x_off = 0;
            } else {
                x_off = 1;
                if up {
                    if y > 0 {
                        y -= 1;
                    } else {
                        up = false;
                        x -= 2;
                    }
                } else if y < size as i64 - 1 {
                    y += 1;
                } else {
                    up = true;
                    x -= 2;
                }
            }
            if x == 5 {
                x -= 1;
            }
            if s.empty((x as usize) + x_off, y as usize) {
                break;
            }
        }
    }
    s
}

/// `qrcode.New(content, qrcode.Low).Bitmap()`: the modules, quiet zone included, `true` = dark.
pub fn bitmap(content: &str) -> Result<Vec<Vec<bool>>, String> {
    let mut chosen = None;
    let mut err: Option<&'static str> = None;
    for t in 0..3 {
        let enc = Encoder::new(t);
        match enc.encode(content.as_bytes()) {
            Err(e) => {
                err = Some(e);
                continue;
            }
            Ok(bits) => {
                err = None;
                let v = LOW.iter().filter(|v| v.0 >= enc.min_version && v.0 <= enc.max_version).find(|v| data_bits(v) >= bits.len());
                if let Some(v) = v {
                    chosen = Some((*v, bits));
                    break;
                }
            }
        }
    }
    if let Some(e) = err {
        return Err(e.to_string());
    }
    let Some(((version, blocks, rem), mut data)) = chosen else { return Err("content too long to encode".into()) };
    let ndata = data_bits(&(version, blocks, rem));
    // terminator, then padding to a codeword, then the pad bytes
    let free = ndata - data.len();
    data.extend(std::iter::repeat_n(false, free.min(4)));
    if data.len() != ndata {
        data.extend(std::iter::repeat_n(false, (8 - data.len() % 8) % 8));
        let pads = [0xecu32, 0x11];
        let mut i = 0;
        while ndata - data.len() >= 8 {
            push_bits(&mut data, pads[i], 8);
            i = 1 - i;
        }
    }
    // blocks, each with its EC codewords, interleaved
    let bytes: Vec<u8> = data.chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | u8::from(*b))).collect();
    let mut blks: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut at = 0;
    for &(n, cw, dcw) in blocks {
        for _ in 0..n {
            let d = bytes[at..at + dcw].to_vec();
            at += dcw;
            let ec = rs_ec(&d, cw - dcw);
            blks.push((d, ec));
        }
    }
    let mut out = Vec::new();
    let maxd = blks.iter().map(|b| b.0.len()).max().unwrap_or(0);
    for i in 0..maxd {
        for b in &blks {
            if let Some(c) = b.0.get(i) {
                push_bits(&mut out, u32::from(*c), 8);
            }
        }
    }
    let maxe = blks.iter().map(|b| b.1.len()).max().unwrap_or(0);
    for i in 0..maxe {
        for b in &blks {
            if let Some(c) = b.1.get(i) {
                push_bits(&mut out, u32::from(*c), 8);
            }
        }
    }
    out.extend(std::iter::repeat_n(false, rem));
    let mut best: Option<(i64, Symbol)> = None;
    for mask in 0..8 {
        let s = build(version, mask, &out);
        let p = s.penalty();
        if best.as_ref().is_none_or(|(bp, _)| p < *bp) {
            best = Some((p, s));
        }
    }
    Ok(best.unwrap().1.module)
}

fn data_bits(v: &(usize, &[(usize, usize, usize)], usize)) -> usize {
    v.1.iter().map(|b| 8 * b.0 * b.2).sum()
}

/// `qrSVG`: one path of black runs on white, the quiet zone included.
pub fn svg(bm: &[Vec<bool>]) -> String {
    use std::fmt::Write;
    let n = bm.len();
    let mut b = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}" shape-rendering="crispEdges"><rect width="{n}" height="{n}" fill="#fff"/><path fill="#000" d=""##);
    for (y, row) in bm.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            let _ = write!(b, "M{x0} {y}h{}v1h-{}z", x - x0, x - x0);
        }
    }
    b.push_str(r#""/></svg>"#);
    b
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    /// Bitmaps from go-qrcode itself (`qrcode.New(s, qrcode.Low).Bitmap()`, rows of 0/1 and a newline, SHA-256):
    /// byte, numeric, alphanumeric and mixed segments, versions 1 to 30.
    #[test]
    fn same_matrix_as_go_qrcode() {
        let long = format!("https://beatscript.app/p#zdpuAyGqNP8rmqxhRGzV.{}", "A".repeat(1958));
        let mixed = "1234ABCD/".repeat(50);
        for (s, n, sum) in [
            ("https://beatscript.app/p/zdpuAyGqNP8rmqxhRGzVeEyNKqjFbCpaXaR3yD6nQvZ5z5wdN", 41, "eefca8c68d0c74429743637eac765ed6e7cc08536b63db132ba8d889722572dc"),
            ("0123456789012345", 29, "2e3ec9d9db3e0f50ef81f40d66c65451d44838784b413af2b3f5b3b5a89d11c6"),
            ("HELLO WORLD 123", 29, "9e4e3ee4ec4b764e73cd00f096cf703b27e0891937af09869d9beae2b3fe3ed1"),
            ("a", 29, "0416c36da839df54c9ad78aa055db0798ea94fabc494e39998b5596004ab1c81"),
            ("https://beatscript.app/p#zdpuAyGqNP8rmqxhRGzVeEyNKqjFbCpaXaR3yD6nQvZ5z5wdN.eJwrKUotyc9LUShKTUtMTVRIy8xJLVYoSMwrSUtNU0jLTE4tVkjKT0nNTE_RK0gtysvMT1WwVQAAcPoQ_g", 57, "6c208ced99536f704cfb68fd90a65036e07e3f6265772bb1c3dea40f8193f8ce"),
            (long.as_str(), 133, "0e05d8fb978dde3773dbfcd6fcb14317451273834441856f65488133c293eaf3"),
            (mixed.as_str(), 69, "498ef554c85e9a7bb997e9419d4ae1dde69d907713c8b0c2d95a3b28eb5de7f4"),
        ] {
            let bm = super::bitmap(s).unwrap();
            let mut t = String::new();
            for r in &bm {
                for v in r {
                    t.push(if *v { '1' } else { '0' });
                }
                t.push('\n');
            }
            assert_eq!(bm.len(), n, "{s}");
            assert_eq!(hex::encode(Sha256::digest(t.as_bytes())), sum, "{s}");
        }
    }
}
