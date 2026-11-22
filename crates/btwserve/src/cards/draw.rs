//! Go `card/render.go` + `style.go`: the card PNG. The Go server draws with fogleman/gg and x/image's opentype
//! faces (Go Mono, Go Regular, both bold); this draws the same layout, the same seeded markers and the same glyph
//! placement (26.6 fixed point, advances rounded to whole pixels as `HintingFull` does, unhinted outlines at the
//! sub-pixel dot), with tiny-skia for the shapes and ab_glyph for the outlines. Parity is perceptual, not bytes:
//! the background and every gradient are computed with gg's own formulas; shape and glyph anti-aliasing comes
//! from another rasterizer.

use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, GlyphId, OutlineCurve};
use tiny_skia::{FillRule, LineCap, LineJoin, Mask, Path, PathBuilder, Stroke, Transform};

use super::lex::{class_at, clean_line, clean_text, clock, fmt_bpm, key_label, license_badge, short_cid, Span};
use super::region::{hit_tokens, is_part_line, is_ramp_line, pick_region, RLine};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Wide,
    Square,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Variant::Square => "square",
            Variant::Wide => "wide",
        }
    }
    pub fn size(self) -> (usize, usize) {
        match self {
            Variant::Square => (1080, 1080),
            Variant::Wide => (1200, 630),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub name: String,
    pub start: i64,
    pub bars: i64,
}

/// Everything a card is drawn from (Go `content`).
#[derive(Clone, Debug, Default)]
pub struct Content {
    pub seed: String,
    pub cid: String,
    pub name: String,
    pub license: String,
    pub canonical: String,
    pub spans: Vec<Span>,
    pub lines: Option<Vec<RLine>>,
    pub section: String,
    pub tempo: f64,
    pub key: String,
    pub bars: i64,
    pub seconds: f64,
    pub sections: Vec<Section>,
    pub generic: bool,
    pub site: String,
}

impl Content {
    fn site_name(&self) -> String {
        if self.site.is_empty() {
            "btw".into()
        } else {
            self.site.clone()
        }
    }
}

// ---------- colours (style.go) ----------

#[derive(Clone, Copy, Debug)]
pub struct Nrgba(u8, u8, u8, u8);

const fn rgb(h: u32) -> Nrgba {
    Nrgba((h >> 16) as u8, (h >> 8) as u8, h as u8, 255)
}
fn alpha(c: Nrgba, a: f64) -> Nrgba {
    let a = a.clamp(0.0, 1.0);
    Nrgba(c.0, c.1, c.2, (a * 255.0 + 0.5) as u8)
}
/// color.NRGBA.RGBA(): 16-bit premultiplied.
fn rgba16(c: Nrgba) -> [u32; 4] {
    let a = u32::from(c.3);
    let f = |v: u8| {
        let mut v = u32::from(v);
        v |= v << 8;
        v * a / 0xff
    };
    [f(c.0), f(c.1), f(c.2), a | (a << 8)]
}

const COL_BG0: Nrgba = rgb(0x070b12);
const COL_BG1: Nrgba = rgb(0x0b111c);
const COL_STAGE: Nrgba = rgb(0x0e1522);
const COL_EDGE: Nrgba = rgb(0x1d2a42);
const COL_RULE: Nrgba = rgb(0x182236);
const COL_TEXT: Nrgba = rgb(0xe6ecf5);
const COL_SUB: Nrgba = rgb(0x8d9bb5);
const COL_DIM: Nrgba = rgb(0x66768f);
const COL_ACCENT: Nrgba = rgb(0x5eead4);
const COL_GREEN: Nrgba = rgb(0x4ade80);
const COL_AMBER: Nrgba = rgb(0xffb020);
const COL_AMBER_T: Nrgba = rgb(0xffe3a6);
const COL_BLOCK: Nrgba = rgb(0x16213a);
const COL_BLOCK_T: Nrgba = rgb(0x93a3bf);

fn colour_of(class: &str) -> Nrgba {
    rgb(match class {
        "comment" => 0x7487a6,
        "keyword" => 0xc4a7ff,
        "role" => 0x5eead4,
        "action" | "verb" => 0x7db8ff,
        "deck" => 0xff8aa1,
        "number" => 0xf4c27a,
        "step" | "note" | "string" => 0xa8e6a1,
        "rest" => 0x62759a,
        "generator" => 0xf7b4d6,
        "param" => 0x9fb6d8,
        "operator" | "op" | "punct" => 0x93a3bf,
        "error" => 0xff6b6b,
        _ => 0xd3dbe8, // word
    })
}

// ---------- fonts ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontKind {
    Mono,
    MonoBold,
    Sans,
    SansBold,
}

fn fonts() -> &'static [FontRef<'static>; 4] {
    static F: OnceLock<[FontRef<'static>; 4]> = OnceLock::new();
    F.get_or_init(|| {
        let p = |b: &'static [u8]| FontRef::try_from_slice(b).expect("Go font");
        [
            p(include_bytes!("../../assets/gofont/Go-Mono.ttf")),
            p(include_bytes!("../../assets/gofont/Go-Mono-Bold.ttf")),
            p(include_bytes!("../../assets/gofont/Go-Regular.ttf")),
            p(include_bytes!("../../assets/gofont/Go-Bold.ttf")),
        ]
    })
}

fn font(k: FontKind) -> &'static FontRef<'static> {
    &fonts()[k as usize]
}

/// opentype.NewFace's scale: the size in 26.6 at 72 DPI.
fn ppem26(size: f64) -> i64 {
    (0.5 + size * 64.0) as i64
}

/// sfnt `scale`: x (26.6 · units) / unitsPerEm, rounded half away from zero.
fn sfnt_scale(x: i64, upem: i64) -> i64 {
    let x = if x >= 0 { x + upem / 2 } else { x - upem / 2 };
    x / upem
}

/// GlyphAdvance with HintingFull: in 26.6, quantized to the nearest pixel.
fn advance26(k: FontKind, size: f64, c: char) -> i64 {
    let f = font(k);
    let upem = f.units_per_em().unwrap_or(2048.0) as i64;
    let id = f.glyph_id(c);
    let adv = f.h_advance_unscaled(id) as i64;
    let a = sfnt_scale(adv * ppem26(size), upem);
    (a + 32) & !63
}

/// font.MeasureString, in pixels (no kern table in the Go fonts).
fn measure(k: FontKind, size: f64, s: &str) -> f64 {
    s.chars().map(|c| advance26(k, size, c)).sum::<i64>() as f64 / 64.0
}

// ---------- the canvas ----------

struct Canvas {
    w: usize,
    h: usize,
    pix: Vec<u8>, // RGBA, premultiplied (opaque)
}

impl Canvas {
    /// dst = dst·(1 − sa·ma) + src·ma, Go's draw / raster painters' Over (16-bit arithmetic).
    #[inline]
    fn blend(&mut self, x: usize, y: usize, c: [u32; 4], ma: u32) {
        const M: u32 = 0xffff;
        if ma == 0 {
            return;
        }
        let a = (M - (c[3] * ma / M)) * 0x101;
        let i = (y * self.w + x) * 4;
        for k in 0..4 {
            let d = u32::from(self.pix[i + k]);
            self.pix[i + k] = ((d * a + c[k] * ma) / M >> 8) as u8;
        }
    }

    /// The coverage of `path` (non-zero), as `(x0, y0, mask)` over the path's pixel bounds clipped to the canvas.
    fn coverage(&self, path: &Path) -> Option<(usize, usize, Mask)> {
        let b = path.bounds();
        let x0 = (b.left().floor() as i64 - 1).max(0);
        let y0 = (b.top().floor() as i64 - 1).max(0);
        let x1 = (b.right().ceil() as i64 + 1).min(self.w as i64);
        let y1 = (b.bottom().ceil() as i64 + 1).min(self.h as i64);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let mut m = Mask::new((x1 - x0) as u32, (y1 - y0) as u32)?;
        m.fill_path(path, FillRule::Winding, true, Transform::from_translate(-x0 as f32, -y0 as f32));
        Some((x0 as usize, y0 as usize, m))
    }

    fn paint(&mut self, path: &Path, colour: impl Fn(usize, usize) -> [u32; 4]) {
        let Some((x0, y0, m)) = self.coverage(path) else { return };
        let mw = m.width() as usize;
        for (i, &cov) in m.data().iter().enumerate() {
            if cov == 0 {
                continue;
            }
            let (x, y) = (x0 + i % mw, y0 + i / mw);
            self.blend(x, y, colour(x, y), u32::from(cov) * 0x101);
        }
    }

    fn fill(&mut self, path: Option<Path>, c: Nrgba) {
        if let Some(p) = path {
            let c = rgba16(c);
            self.paint(&p, |_, _| c);
        }
    }

    fn stroke(&mut self, path: Option<Path>, width: f64, c: Nrgba) {
        let Some(p) = path else { return };
        let st = Stroke { width: width as f32, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        if let Some(s) = p.stroke(&st, 1.0) {
            let c = rgba16(c);
            self.paint(&s, |_, _| c);
        }
    }

    /// font.Drawer.DrawString with its baseline at y: each glyph's unhinted outline at the 26.6 dot.
    fn text(&mut self, k: FontKind, size: f64, x: f64, y: f64, c: Nrgba, s: &str) {
        let f = font(k);
        let upem = f.units_per_em().unwrap_or(2048.0) as i64;
        let ppem = ppem26(size);
        let mut dot_x = (x * 64.0).round() as i64;
        let dot_y = (y * 64.0).round() as i64;
        let col = rgba16(c);
        for ch in s.chars() {
            let id: GlyphId = f.glyph_id(ch);
            let adv = advance26(k, size, ch);
            if let Some(o) = f.outline(id) {
                self.glyph(&o.curves, upem, ppem, dot_x, dot_y, col);
            }
            dot_x += adv;
        }
    }

    fn glyph(&mut self, curves: &[OutlineCurve], upem: i64, ppem: i64, dx: i64, dy: i64, col: [u32; 4]) {
        // sfnt LoadGlyph: each point scaled to 26.6 (y down), then moved to the dot
        let k = ppem as f64 / upem as f64;
        let pt = |p: ab_glyph::Point| -> (i64, i64) { ((f64::from(p.x) * k).round() as i64 + dx, -(f64::from(p.y) * k).round() as i64 + dy) };
        let mut segs: Vec<Vec<(i64, i64)>> = Vec::with_capacity(curves.len());
        let (mut minx, mut miny, mut maxx, mut maxy) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for c in curves {
            let pts: Vec<(i64, i64)> = match c {
                OutlineCurve::Line(a, b) => vec![pt(*a), pt(*b)],
                OutlineCurve::Quad(a, b, e) => vec![pt(*a), pt(*b), pt(*e)],
                OutlineCurve::Cubic(a, b, e, g) => vec![pt(*a), pt(*b), pt(*e), pt(*g)],
            };
            for &(x, y) in &pts {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
            segs.push(pts);
        }
        if segs.is_empty() {
            return;
        }
        // dr: floor of the min, ceil of the max (26.6 -> pixels)
        let (rx0, ry0) = (minx.div_euclid(64), miny.div_euclid(64));
        let (rx1, ry1) = ((maxx + 63).div_euclid(64), (maxy + 63).div_euclid(64));
        let (w, h) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
        if w == 0 || h == 0 {
            return;
        }
        let mut r = ab_glyph_rasterizer::Rasterizer::new(w, h);
        let p = |(x, y): (i64, i64)| ab_glyph_rasterizer::point((x - rx0 * 64) as f32 / 64.0, (y - ry0 * 64) as f32 / 64.0);
        for s in &segs {
            match s.len() {
                2 => r.draw_line(p(s[0]), p(s[1])),
                3 => r.draw_quad(p(s[0]), p(s[1]), p(s[2])),
                _ => r.draw_cubic(p(s[0]), p(s[1]), p(s[2]), p(s[3])),
            }
        }
        let mut mask = vec![0u8; w * h];
        r.for_each_pixel(|i, a| mask[i] = (255.99998 * a.clamp(0.0, 1.0)) as u8);
        for (i, &m) in mask.iter().enumerate() {
            if m == 0 {
                continue;
            }
            let (x, y) = (rx0 + (i % w) as i64, ry0 + (i / w) as i64);
            if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
                continue;
            }
            self.blend(x as usize, y as usize, col, u32::from(m) * 0x101);
        }
    }

    fn ellipsize(&self, k: FontKind, size: f64, width: f64, s: &str) -> String {
        if measure(k, size, s) <= width {
            return s.to_string();
        }
        let mut r: Vec<char> = s.chars().collect();
        while r.len() > 1 {
            r.pop();
            let t = format!("{}…", r.iter().collect::<String>().trim_end_matches(' '));
            if measure(k, size, &t) <= width {
                return t;
            }
        }
        "…".into()
    }
}

// ---------- gg's paths ----------

/// gg's DrawEllipticalArc: 16 quadratic segments.
fn arc(pb: &mut PathBuilder, has_current: &mut bool, x: f64, y: f64, r: f64, a1: f64, a2: f64) {
    const N: usize = 16;
    for i in 0..N {
        let p1 = i as f64 / N as f64;
        let p2 = (i + 1) as f64 / N as f64;
        let b1 = a1 + (a2 - a1) * p1;
        let b2 = a1 + (a2 - a1) * p2;
        let (x0, y0) = (x + r * b1.cos(), y + r * b1.sin());
        let (x1, y1) = (x + r * ((b1 + b2) / 2.0).cos(), y + r * ((b1 + b2) / 2.0).sin());
        let (x2, y2) = (x + r * b2.cos(), y + r * b2.sin());
        let (cx, cy) = (2.0 * x1 - x0 / 2.0 - x2 / 2.0, 2.0 * y1 - y0 / 2.0 - y2 / 2.0);
        if i == 0 {
            if *has_current {
                pb.line_to(x0 as f32, y0 as f32);
            } else {
                pb.move_to(x0 as f32, y0 as f32);
                *has_current = true;
            }
        }
        pb.quad_to(cx as f32, cy as f32, x2 as f32, y2 as f32);
    }
}

fn circle(x: f64, y: f64, r: f64) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut hc = false;
    arc(&mut pb, &mut hc, x, y, r, 0.0, 2.0 * std::f64::consts::PI);
    pb.close();
    pb.finish()
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> Option<Path> {
    let (x0, x1, x2, x3) = (x, x + r, x + w - r, x + w);
    let (y0, y1, y2, y3) = (y, y + r, y + h - r, y + h);
    let rad = |d: f64| d.to_radians();
    let mut pb = PathBuilder::new();
    let mut hc = true;
    pb.move_to(x1 as f32, y0 as f32);
    pb.line_to(x2 as f32, y0 as f32);
    arc(&mut pb, &mut hc, x2, y1, r, rad(270.0), rad(360.0));
    pb.line_to(x3 as f32, y2 as f32);
    arc(&mut pb, &mut hc, x2, y2, r, rad(0.0), rad(90.0));
    pb.line_to(x1 as f32, y3 as f32);
    arc(&mut pb, &mut hc, x1, y2, r, rad(90.0), rad(180.0));
    pb.line_to(x0 as f32, y1 as f32);
    arc(&mut pb, &mut hc, x1, y1, r, rad(180.0), rad(270.0));
    pb.close();
    pb.finish()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x as f32, y as f32);
    pb.line_to((x + w) as f32, y as f32);
    pb.line_to((x + w) as f32, (y + h) as f32);
    pb.line_to(x as f32, (y + h) as f32);
    pb.close();
    pb.finish()
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x1 as f32, y1 as f32);
    pb.line_to(x2 as f32, y2 as f32);
    pb.finish()
}

// ---------- gg's gradients ----------

/// A colour stop (sorted by position, as gg keeps them).
type Stops = Vec<(f64, Nrgba)>;

/// gg `getColor`: 16-bit premultiplied.
fn stop_colour(pos: f64, stops: &Stops) -> [u32; 4] {
    if pos <= 0.0 || stops.len() == 1 {
        return rgba16(stops[0].1);
    }
    let last = stops[stops.len() - 1];
    if pos >= last.0 {
        return rgba16(last.1);
    }
    for i in 1..stops.len() {
        if pos < stops[i].0 {
            let t = (pos - stops[i - 1].0) / (stops[i].0 - stops[i - 1].0);
            let (c0, c1) = (rgba16(stops[i - 1].1), rgba16(stops[i].1));
            let mut out = [0u32; 4];
            for k in 0..4 {
                let v = ((f64::from(c0[k]) * (1.0 - t) + f64::from(c1[k]) * t) as i32 >> 8) as u8;
                out[k] = u32::from(v) | (u32::from(v) << 8);
            }
            return out;
        }
    }
    rgba16(last.1)
}

/// gg's radialGradient.ColorAt (pixman's two-circle gradient), at the pixel centre.
fn radial(c0: (f64, f64, f64), c1: (f64, f64, f64), stops: Stops) -> impl Fn(usize, usize) -> [u32; 4] {
    let cd = (c1.0 - c0.0, c1.1 - c0.1, c1.2 - c0.2);
    let dot3 = |a: (f64, f64, f64), b: (f64, f64, f64)| a.0 * b.0 + a.1 * b.1 + a.2 * b.2;
    let a = dot3(cd, (cd.0, cd.1, -cd.2));
    let inva = if a != 0.0 { 1.0 / a } else { 0.0 };
    let mindr = -c0.2;
    move |x, y| {
        let dx = x as f64 + 0.5 - c0.0;
        let dy = y as f64 + 0.5 - c0.1;
        let b = dot3((dx, dy, c0.2), cd);
        let c = dot3((dx, dy, -c0.2), (dx, dy, c0.2));
        if a == 0.0 {
            if b == 0.0 {
                return [0; 4];
            }
            let t = 0.5 * c / b;
            return if t * cd.2 >= mindr { stop_colour(t, &stops) } else { [0; 4] };
        }
        let discr = b * b - a * c;
        if discr >= 0.0 {
            let s = discr.sqrt();
            let (t0, t1) = ((b + s) * inva, (b - s) * inva);
            if t0 * cd.2 >= mindr {
                return stop_colour(t0, &stops);
            } else if t1 * cd.2 >= mindr {
                return stop_colour(t1, &stops);
            }
        }
        [0; 4]
    }
}

/// gg's linearGradient.ColorAt for a horizontal gradient (at the pixel's corner, as gg evaluates it).
fn linear_h(x0: f64, x1: f64, stops: Stops) -> impl Fn(usize, usize) -> [u32; 4] {
    move |x, _| stop_colour((x as f64 - x0) / (x1 - x0), &stops)
}

// ---------- the seeded PRNG ----------

/// splitmix64 seeded by FNV-1a 64 of the seed's bytes (Go `rng`).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: &str) -> Rng {
        let mut h: u64 = 14695981039346656037;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(1099511628211);
        }
        let mut r = Rng(h);
        r.next();
        r
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn intn(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }
    pub fn float(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

// ---------- layout ----------

struct Layout {
    w: f64,
    m: f64,
    name_size: f64,
    name_base: f64,
    brand_size: f64,
    brand_base: f64,
    stage_y: f64,
    stage_h: f64,
    tl_y: f64,
    tl_h: f64,
    tl_font: f64,
    facts_base: f64,
    facts_row2: f64,
    val_size: f64,
    unit_size: f64,
    badge_size: f64,
    cid_size: f64,
    code_sizes: &'static [f64],
    lh_mul: f64,
    gutter: f64,
    pad: f64,
    two_row_facts: bool,
}

fn layout_for(v: Variant) -> Layout {
    if v == Variant::Square {
        return Layout {
            w: 1080.0,
            m: 56.0,
            brand_size: 22.0,
            brand_base: 82.0,
            name_size: 58.0,
            name_base: 158.0,
            stage_y: 196.0,
            stage_h: 604.0,
            tl_y: 826.0,
            tl_h: 58.0,
            tl_font: 18.0,
            facts_base: 962.0,
            facts_row2: 1020.0,
            val_size: 44.0,
            unit_size: 22.0,
            badge_size: 20.0,
            cid_size: 22.0,
            code_sizes: &[28.0, 27.0, 26.0, 25.0, 24.0, 23.0, 22.0],
            lh_mul: 1.5,
            gutter: 86.0,
            pad: 22.0,
            two_row_facts: true,
        };
    }
    Layout {
        w: 1200.0,
        m: 44.0,
        brand_size: 20.0,
        brand_base: 62.0,
        name_size: 36.0,
        name_base: 66.0,
        stage_y: 92.0,
        stage_h: 372.0,
        tl_y: 486.0,
        tl_h: 38.0,
        tl_font: 15.0,
        facts_base: 584.0,
        facts_row2: 0.0,
        val_size: 30.0,
        unit_size: 17.0,
        badge_size: 16.0,
        cid_size: 19.0,
        code_sizes: &[25.0, 24.0, 23.0, 22.0, 21.0, 20.0, 19.0],
        lh_mul: 1.36,
        gutter: 78.0,
        pad: 16.0,
        two_row_facts: false,
    }
}

// ---------- the card ----------

/// `drawCard`: c as a PNG.
pub fn draw_card(c: &Content, v: Variant) -> Vec<u8> {
    let mut c = c.clone();
    let l = layout_for(v);
    let (w, h) = v.size();
    let mut cv = Canvas { w, h, pix: background(w, h).to_vec() };
    let mut r = Rng::new(&c.seed);
    header(&mut cv, &c, &l);
    stage(&mut cv, &mut c, &l, &mut r);
    timeline(&mut cv, &c, &l, &mut r);
    facts(&mut cv, &c, &l);
    encode_png(&cv)
}

fn encode_png(cv: &Canvas) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(cv.w * cv.h * 3);
    for p in cv.pix.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, cv.w as u32, cv.h as u32);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(&rgb).expect("png data");
    }
    out
}

/// The ground, painted once per size (Go `background`: the same bytes as Go's).
fn background(w: usize, h: usize) -> &'static [u8] {
    static WIDE: OnceLock<Vec<u8>> = OnceLock::new();
    static SQUARE: OnceLock<Vec<u8>> = OnceLock::new();
    let cell = if w == 1080 { &SQUARE } else { &WIDE };
    cell.get_or_init(|| {
        let mut pix = vec![0u8; w * h * 4];
        let blooms = [(w as f64 - 40.0, 0.0, w as f64 * 0.55, COL_ACCENT, 0.07), (0.0, h as f64, w as f64 * 0.5, rgb(0x7c5cff), 0.06)];
        for y in 0..h {
            let t = y as f64 / (h - 1) as f64;
            let br = f64::from(COL_BG0.0) * (1.0 - t) + f64::from(COL_BG1.0) * t;
            let bg = f64::from(COL_BG0.1) * (1.0 - t) + f64::from(COL_BG1.1) * t;
            let bb = f64::from(COL_BG0.2) * (1.0 - t) + f64::from(COL_BG1.2) * t;
            for x in 0..w {
                let (mut r, mut g, mut b) = (br, bg, bb);
                for &(bx, by, brad, c, ba) in &blooms {
                    let d = (x as f64 - bx).hypot(y as f64 - by) / brad;
                    if d >= 1.0 {
                        continue;
                    }
                    let a = ba * (1.0 - d);
                    r += (f64::from(c.0) - r) * a;
                    g += (f64::from(c.1) - g) * a;
                    b += (f64::from(c.2) - b) * a;
                }
                let i = (y * w + x) * 4;
                pix[i] = (r + 0.5) as u8;
                pix[i + 1] = (g + 0.5) as u8;
                pix[i + 2] = (b + 0.5) as u8;
                pix[i + 3] = 255;
            }
        }
        pix
    })
}

fn petri_mark(cv: &mut Canvas, x: f64, y: f64) {
    cv.stroke(circle(x + 7.0, y, 6.5), 2.2, COL_ACCENT);
    cv.stroke(line(x + 13.5, y, x + 22.0, y), 2.2, COL_ACCENT);
    cv.fill(rounded_rect(x + 22.0, y - 9.0, 5.0, 18.0, 1.5), COL_ACCENT);
    cv.stroke(line(x + 27.0, y, x + 35.5, y), 2.2, COL_ACCENT);
    cv.stroke(circle(x + 42.0, y, 6.5), 2.2, COL_ACCENT);
    cv.fill(circle(x + 7.0, y, 2.4), COL_ACCENT);
}

fn header(cv: &mut Canvas, c: &Content, l: &Layout) {
    let site = c.site_name();
    let sw = measure(FontKind::Sans, l.brand_size, &site);
    cv.text(FontKind::Sans, l.brand_size, l.w - l.m - sw, l.brand_base, COL_SUB, &site);
    petri_mark(cv, l.w - l.m - sw - 60.0, l.brand_base - l.brand_size * 0.36);
    let mut max_w = l.w - 2.0 * l.m;
    if !l.two_row_facts {
        max_w -= sw + 60.0 + 36.0;
    }
    let name = cv.ellipsize(FontKind::SansBold, l.name_size, max_w, &c.name);
    cv.text(FontKind::SansBold, l.name_size, l.m, l.name_base, COL_TEXT, &name);
}

#[derive(Clone, Copy, PartialEq)]
struct Lamp {
    line: usize,
    col: usize,
    n: usize,
}

struct PLine {
    l: RLine,
    runes: Vec<char>,
    cls: Vec<String>,
    trunc: bool,
}

fn stage(cv: &mut Canvas, c: &mut Content, l: &Layout, r: &mut Rng) {
    let (sx, sy, sw, sh) = (l.m, l.stage_y, l.w - 2.0 * l.m, l.stage_h);
    cv.fill(rounded_rect(sx, sy, sw, sh, 16.0), COL_STAGE);
    cv.stroke(rounded_rect(sx + 0.75, sy + 0.75, sw - 1.5, sh - 1.5, 16.0), 1.5, COL_EDGE);

    let canon_owned = c.canonical.trim_end_matches('\n').to_string();
    let canon: Vec<&str> = canon_owned.split('\n').collect();
    let lines = match &c.lines {
        Some(ls) => ls.clone(),
        None => {
            let (ls, sec) = pick_region(&canon);
            c.section = sec;
            ls
        }
    };
    let mut classes: Option<Vec<String>> = None;
    let mut offsets: Vec<usize> = Vec::new();
    if !c.canonical.is_empty() {
        classes = Some(class_at(&c.canonical, &c.spans));
        let mut off = 0;
        for piece in c.canonical.split_inclusive('\n').chain(if c.canonical.ends_with('\n') { Some("") } else { None }) {
            offsets.push(off);
            off += piece.len();
        }
    }
    if lines.is_empty() {
        return;
    }
    let mut prep: Vec<PLine> = lines
        .iter()
        .map(|rl| {
            let cleaned = clean_line(&rl.text);
            let runes: Vec<char> = cleaned.chars().collect();
            let mut cls = vec![String::new(); runes.len()];
            if rl.idx >= 0 && (rl.idx as usize) < offsets.len() && cleaned == rl.text {
                if let Some(classes) = &classes {
                    let base = offsets[rl.idx as usize];
                    let mut bi = 0;
                    for (k, ru) in runes.iter().enumerate() {
                        if base + bi < classes.len() {
                            cls[k].clone_from(&classes[base + bi]);
                        }
                        bi += ru.len_utf8();
                    }
                }
            }
            PLine { l: rl.clone(), runes, cls, trunc: false }
        })
        .collect();
    let s = |p: &PLine| p.runes.iter().collect::<String>();
    let ramp_lines: Vec<usize> = prep.iter().enumerate().filter(|(_, p)| p.l.idx >= 0 && is_ramp_line(&s(p))).map(|(i, _)| i).collect();
    for p in prep.iter_mut().filter(|p| p.l.idx == -1) {
        for k in p.cls.iter_mut() {
            *k = "comment".into();
        }
    }

    let code_w = sw - l.gutter - l.pad - 6.0;
    let (mut size, mut lh, mut adv) = (0.0, 0.0, 0.0);
    let mut best_trunc: i64 = -1;
    for (i, &sz) in l.code_sizes.iter().enumerate() {
        let a = measure(FontKind::Mono, sz, "0");
        let max_cols = (code_w / a) as usize;
        let trunc = prep.iter().filter(|p| p.runes.len() > max_cols).count() as i64;
        let lh_i = (sz * l.lh_mul).round();
        let mut extra = prep.iter().filter(|p| p.l.gap_top).count() as f64 * lh_i * 0.5;
        if !ramp_lines.is_empty() {
            extra += 7.0;
        }
        let fits = prep.len() as f64 * lh_i + extra <= sh - 2.0 * l.pad;
        if fits {
            if best_trunc < 0 || trunc < best_trunc {
                (size, lh, adv, best_trunc) = (sz, lh_i, a, trunc);
            }
            if trunc <= 1 || trunc * 6 <= prep.len() as i64 {
                (size, lh, adv) = (sz, lh_i, a);
                break;
            }
        } else if i == l.code_sizes.len() - 1 && best_trunc < 0 {
            (size, lh, adv) = (sz, lh_i, a);
        }
    }
    {
        let units = prep.len() as f64 + prep.iter().filter(|p| p.l.gap_top).count() as f64 * 0.5;
        let max = (sh - 2.0 * l.pad - 7.0 * if ramp_lines.is_empty() { 0.0 } else { 1.0 }) / units;
        if max > lh {
            lh = max.min(lh * 1.25).round();
        }
    }
    let max_cols = (code_w / adv) as usize;
    for p in prep.iter_mut() {
        if p.runes.len() > max_cols {
            let keep = max_cols.saturating_sub(1);
            p.runes.truncate(keep);
            p.runes.push('…');
            p.cls.truncate(keep);
            p.cls.push("comment".into());
            p.trunc = true;
        }
    }

    // markers, from the CID's seed
    let parts: Vec<usize> = prep.iter().enumerate().filter(|(_, p)| p.l.idx >= 0 && is_part_line(&s(p))).map(|(i, _)| i).collect();
    let mut greens = std::collections::HashSet::new();
    for &i in &parts {
        if r.float() < 0.85 {
            greens.insert(i);
        }
    }
    if greens.is_empty() && !parts.is_empty() {
        greens.insert(parts[0]);
    }
    let mut cands: Vec<Lamp> = Vec::new();
    for (i, p) in prep.iter().enumerate() {
        if p.l.idx < 0 {
            continue;
        }
        for ht in hit_tokens(&s(p)) {
            if ht.col + ht.n <= p.runes.len() && !(p.trunc && ht.col + ht.n >= p.runes.len().saturating_sub(1)) {
                cands.push(Lamp { line: i, col: ht.col, n: ht.n });
            }
        }
    }
    let mut lamps: Vec<Lamp> = Vec::new();
    let want = 2 + r.intn(2);
    let mut used_line = std::collections::HashSet::new();
    let mut pass = 0;
    while pass < 2 && lamps.len() < want {
        let mut pool = cands.clone();
        while !pool.is_empty() && lamps.len() < want {
            let k = r.intn(pool.len());
            let cd = pool.remove(k);
            if pass == 0 && used_line.contains(&cd.line) {
                continue;
            }
            if lamps.contains(&cd) {
                continue;
            }
            used_line.insert(cd.line);
            lamps.push(cd);
        }
        pass += 1;
    }
    let mut ramp: i64 = -1;
    let mut progress = 0.0;
    if !ramp_lines.is_empty() {
        ramp = ramp_lines[r.intn(ramp_lines.len())] as i64;
        progress = ((0.22 + 0.56 * r.float()) * 20.0).round() / 20.0;
    }
    let lamps_at = |i: usize| lamps.iter().filter(move |lp| lp.line == i).copied().collect::<Vec<_>>();

    // rows
    let mut tops = vec![0.0; prep.len()];
    let mut y = 0.0;
    for (i, p) in prep.iter().enumerate() {
        if p.l.gap_top {
            y += lh * 0.5;
        }
        tops[i] = y;
        y += lh;
        if i as i64 == ramp {
            y += 7.0;
        }
    }
    let total = y;
    let y0 = sy + (l.pad * 0.8).max((sh - total) / 2.0);
    let cx = sx + l.gutter;
    let num_right = sx + l.gutter - 42.0;
    let dot_x = sx + l.gutter - 24.0;

    // glows under the text
    for lp in &lamps {
        let base = y0 + tops[lp.line];
        let x = cx + lp.col as f64 * adv;
        let w = lp.n as f64 * adv;
        let cy = base + lh / 2.0;
        let rad = w / 2.0 + 22.0;
        let g = radial((x + w / 2.0, cy, 2.0), (x + w / 2.0, cy, rad), vec![(0.0, alpha(COL_AMBER, 0.55)), (0.55, alpha(COL_AMBER, 0.16)), (1.0, alpha(COL_AMBER, 0.0))]);
        let (gx0, gy0) = ((x + w / 2.0 - rad).max(sx + 2.0), (cy - rad).max(sy + 2.0));
        let (gx1, gy1) = ((x + w / 2.0 + rad).min(sx + sw - 2.0), (cy + rad).min(sy + sh - 2.0));
        if let Some(p) = rect(gx0, gy0, gx1 - gx0, gy1 - gy0) {
            cv.paint(&p, g);
        }
        cv.fill(rounded_rect(x - 3.0, base + lh * 0.12, w + 6.0, lh * 0.76, 6.0), alpha(COL_AMBER, 0.30));
    }

    for (i, p) in prep.iter().enumerate() {
        let base = y0 + tops[i];
        let baseline = base + lh / 2.0 + size * 0.35;
        if p.l.gap_top {
            let gy = base - lh * 0.25;
            let mut gx = cx;
            while gx < sx + sw - l.pad - 6.0 {
                cv.fill(circle(gx, gy, 1.0), alpha(COL_DIM, 0.55));
                gx += 9.0;
            }
        }
        if p.l.idx >= 0 {
            let num = (p.l.idx + 1).to_string();
            let nw = measure(FontKind::Mono, size * 0.8, &num);
            cv.text(FontKind::Mono, size * 0.8, num_right - nw, baseline, alpha(COL_DIM, 0.85), &num);
        }
        let here = lamps_at(i);
        if !here.is_empty() {
            let g = radial((dot_x, base + lh / 2.0, 0.0), (dot_x, base + lh / 2.0, 12.0), vec![(0.0, alpha(COL_AMBER, 0.65)), (1.0, alpha(COL_AMBER, 0.0))]);
            if let Some(pth) = rect(dot_x - 12.0, base + lh / 2.0 - 12.0, 24.0, 24.0) {
                cv.paint(&pth, g);
            }
            cv.fill(circle(dot_x, base + lh / 2.0, 5.0), COL_AMBER);
        } else if i as i64 == ramp {
            cv.fill(circle(dot_x, base + lh / 2.0, 4.2), alpha(COL_ACCENT, 0.95));
        } else if greens.contains(&i) {
            cv.fill(circle(dot_x, base + lh / 2.0, 4.2), alpha(COL_GREEN, 0.80));
        }
        let mut lit = vec![false; p.runes.len()];
        for lp in &here {
            for k in lp.col..(lp.col + lp.n).min(lit.len()) {
                lit[k] = true;
            }
        }
        let mut k = 0;
        while k < p.runes.len() {
            let mut j = k;
            let (cl, is_lit) = (&p.cls[k], lit[k]);
            while j < p.runes.len() && &p.cls[j] == cl && lit[j] == is_lit {
                j += 1;
            }
            let run: String = p.runes[k..j].iter().collect();
            if !run.trim().is_empty() {
                if is_lit {
                    cv.text(FontKind::MonoBold, size, cx + k as f64 * adv, baseline, COL_AMBER_T, &run);
                } else {
                    cv.text(FontKind::Mono, size, cx + k as f64 * adv, baseline, colour_of(cl), &run);
                }
            }
            k = j;
        }
        if i as i64 == ramp {
            let lead = p.runes.iter().take_while(|c| **c == ' ').count();
            let bx = cx + lead as f64 * adv;
            let bw = (p.runes.len() - lead) as f64 * adv;
            let by = base + lh + 0.5;
            cv.fill(rounded_rect(bx, by, bw, 4.0, 2.0), alpha(COL_ACCENT, 0.20));
            cv.fill(rounded_rect(bx, by, (bw * progress).max(4.0), 4.0, 2.0), COL_ACCENT);
        }
    }
}

fn timeline(cv: &mut Canvas, c: &Content, l: &Layout, r: &mut Rng) {
    let (x0, w) = (l.m, l.w - 2.0 * l.m);
    let mut secs = c.sections.clone();
    let mut total: i64 = secs.iter().filter(|s| s.bars > 0).map(|s| s.bars).sum();
    if secs.is_empty() || total == 0 {
        secs = vec![Section { name: "loop".into(), start: 0, bars: 1 }];
        total = 1;
    }
    if secs.len() > 40 {
        secs.truncate(40);
        total = secs.iter().map(|s| s.bars).sum();
    }
    const GAP: f64 = 4.0;
    let avail = w - GAP * (secs.len() as f64 - 1.0);
    let mut x = x0;
    let head_frac = 0.22 + 0.56 * r.float();
    let mut picked: i64 = secs.iter().position(|s| s.name == c.section).map_or(-1, |i| i as i64);
    if picked < 0 {
        for (i, s) in secs.iter().enumerate() {
            if s.name == "drop" {
                picked = i as i64;
            }
        }
    }
    if picked < 0 && secs.len() == 1 {
        picked = 0;
    }
    for (i, s) in secs.iter().enumerate() {
        let bw = avail * s.bars as f64 / total as f64;
        if s.bars <= 0 {
            continue;
        }
        let on = i as i64 == picked;
        cv.fill(rounded_rect(x, l.tl_y, bw, l.tl_h, 7.0), if on { alpha(COL_ACCENT, 0.15) } else { COL_BLOCK });
        if on {
            cv.stroke(rounded_rect(x + 0.8, l.tl_y + 0.8, bw - 1.6, l.tl_h - 1.6, 6.5), 1.6, alpha(COL_ACCENT, 0.85));
        }
        let mut label = clean_text(&s.name, 24);
        let tc = if on { COL_ACCENT } else { COL_BLOCK_T };
        let kind = if on { FontKind::SansBold } else { FontKind::Sans };
        let (mut lx0, mut lx1) = (x + bw / 2.0, x + bw / 2.0);
        if bw > 26.0 {
            label = cv.ellipsize(kind, l.tl_font, bw - 14.0, &label);
            let lw = measure(kind, l.tl_font, &label);
            cv.text(kind, l.tl_font, x + (bw - lw) / 2.0, l.tl_y + l.tl_h / 2.0 + l.tl_font * 0.35, tc, &label);
            lx0 = x + (bw - lw) / 2.0;
            lx1 = x + (bw + lw) / 2.0;
        }
        if on {
            let mut hx = x + 6.0 + (bw - 12.0) * head_frac;
            if hx > lx0 - 8.0 && hx < lx1 + 8.0 {
                if hx - lx0 < lx1 - hx && lx0 - 12.0 > x + 6.0 {
                    hx = lx0 - 12.0;
                } else if lx1 + 12.0 < x + bw - 6.0 {
                    hx = lx1 + 12.0;
                } else if lx0 - 12.0 > x + 6.0 {
                    hx = lx0 - 12.0;
                }
            }
            let g = linear_h(hx - 9.0, hx + 9.0, vec![(0.0, alpha(COL_AMBER, 0.0)), (0.5, alpha(COL_AMBER, 0.55)), (1.0, alpha(COL_AMBER, 0.0))]);
            if let Some(p) = rect(hx - 9.0, l.tl_y - 5.0, 18.0, l.tl_h + 10.0) {
                cv.paint(&p, g);
            }
            cv.fill(rounded_rect(hx - 1.25, l.tl_y - 5.0, 2.5, l.tl_h + 10.0, 1.2), COL_AMBER);
        }
        x += bw + GAP;
    }
}

fn facts(cv: &mut Canvas, c: &Content, l: &Layout) {
    let mut fs: Vec<(String, String)> = Vec::new();
    if c.generic {
        fs.push(("a track in a few lines".into(), String::new()));
        if !l.two_row_facts {
            fs.push(("same text, same CID".into(), String::new()));
        }
    } else {
        if c.tempo > 0.0 {
            fs.push((fmt_bpm(c.tempo), "BPM".into()));
        }
        let k = key_label(&c.key);
        if !k.is_empty() {
            fs.push((k, String::new()));
        }
        if c.bars > 0 && c.seconds > 0.0 {
            fs.push((c.bars.to_string(), "bars".into()));
            fs.push((clock(c.seconds), String::new()));
        } else if c.bars > 0 {
            fs.push((c.bars.to_string(), "bars".into()));
        } else {
            fs.push(("loop".into(), String::new()));
        }
    }
    let base = l.facts_base;
    let mut right = l.w - l.m;
    let id_text = if c.generic { c.site_name() } else { short_cid(&c.cid) };
    let row_right = if l.two_row_facts { l.facts_row2 } else { base };
    let cw = measure(FontKind::Mono, l.cid_size, &id_text);
    cv.text(FontKind::Mono, l.cid_size, right - cw, row_right, COL_SUB, &id_text);
    right -= cw + 22.0;
    let lic = license_badge(&c.license);
    let bw = measure(FontKind::SansBold, l.badge_size, &lic) + 26.0;
    let bh = l.badge_size * 1.9;
    let by = row_right - l.badge_size * 0.35 - bh / 2.0;
    cv.fill(rounded_rect(right - bw, by, bw, bh, bh / 2.0), alpha(COL_ACCENT, 0.12));
    cv.stroke(rounded_rect(right - bw + 0.75, by + 0.75, bw - 1.5, bh - 1.5, bh / 2.0), 1.5, alpha(COL_ACCENT, 0.8));
    cv.text(FontKind::SansBold, l.badge_size, right - bw + 13.0, row_right, COL_ACCENT, &lic);
    right -= bw + 22.0;

    let mut x = l.m;
    let max_x = if l.two_row_facts { l.w - l.m } else { right };
    for (i, (val, unit)) in fs.iter().enumerate() {
        let vw = measure(FontKind::SansBold, l.val_size, val);
        let uw = if unit.is_empty() { 0.0 } else { measure(FontKind::Sans, l.unit_size, unit) + 8.0 };
        if x + vw + uw > max_x {
            break;
        }
        if i > 0 {
            cv.stroke(line(x - 16.0, base - l.val_size * 0.75, x - 16.0, base + l.val_size * 0.1), 2.0, COL_RULE);
        }
        cv.text(FontKind::SansBold, l.val_size, x, base, COL_TEXT, val);
        if !unit.is_empty() {
            cv.text(FontKind::Sans, l.unit_size, x + vw + 8.0, base, COL_SUB, unit);
        }
        x += vw + uw + 34.0;
    }
}
