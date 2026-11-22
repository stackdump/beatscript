//! Railroad diagrams of the grammar, drawn as SVG by our own layout (no diagram library): a production's tree
//! ([`Node`]) is sized bottom up (its width and how far it reaches above and below the line through it) and drawn
//! left to right along that line.
//!
//! - a terminal is a rounded box with the text written in it; a production is a square-cornered box that links to its
//!   own diagram (`#rule-<name>`); a token class is a dashed box that links to its entry (`#class-<NAME>`);
//! - a sequence runs left to right;
//! - a choice puts its first alternative on the line and the others below it, joined by quarter-circle rails;
//! - an optional is a choice of nothing (the line, above) and its part (below);
//! - a repetition (none or more) is an optional whose part has a loop back under it.
//!
//! The SVG has classes, not colours (`rr`, `rr-term`, `rr-rule`, `rr-class`, `rr-line`): the page's stylesheet ([`CSS`])
//! colours it, light and dark. Coordinates are whole or half pixels, written with at most one decimal, so the same
//! production is the same bytes (the golden tests rely on it).

use super::ebnf::Node;

/// The rails' radius.
const R: f64 = 10.0;
/// The space between two alternatives (and under a loop).
const VS: f64 = 10.0;
/// A box's half height.
const BH: f64 = 12.0;
/// The width of one character of the box text (monospace at 13px).
const CW: f64 = 7.8;
/// The space on each side of a box's text, and between two items of a sequence.
const PAD: f64 = 10.0;
const GAP: f64 = 10.0;

/// What a node takes: its width, and how far it reaches above and below the line through it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Size {
    w: f64,
    up: f64,
    down: f64,
}

/// A node as drawn: its own kind and its parts, sized.
enum Shape {
    Skip,
    Box { text: String, kind: Kind },
    Seq(Vec<Laid>),
    Choice(Vec<Laid>),
    /// Its part, with a loop back under it.
    Loop(Box<Laid>),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Term,
    Rule,
    Class,
}

struct Laid {
    shape: Shape,
    size: Size,
}

/// How a name is drawn: a production (links to its diagram) or a token class (links to its entry).
pub trait Names {
    fn is_rule(&self, name: &str) -> bool;
}

impl Names for super::ebnf::Grammar {
    fn is_rule(&self, name: &str) -> bool {
        self.rule(name).is_some()
    }
}

fn text_w(s: &str) -> f64 {
    s.chars().count() as f64 * CW + 2.0 * PAD
}

fn lay(n: &Node, names: &dyn Names) -> Laid {
    match n {
        Node::Term(t) => {
            let text = if t.is_empty() { "\u{2205}".to_string() } else { t.clone() };
            Laid { size: Size { w: text_w(&text), up: BH, down: BH }, shape: Shape::Box { text, kind: Kind::Term } }
        }
        Node::Ref(r) => {
            let kind = if names.is_rule(r) { Kind::Rule } else { Kind::Class };
            Laid { size: Size { w: text_w(r), up: BH, down: BH }, shape: Shape::Box { text: r.clone(), kind } }
        }
        Node::Seq(v) => {
            let parts: Vec<Laid> = v.iter().map(|x| lay(x, names)).collect();
            let w = parts.iter().map(|p| p.size.w).sum::<f64>() + GAP * (parts.len().saturating_sub(1)) as f64;
            let up = parts.iter().map(|p| p.size.up).fold(0.0, f64::max);
            let down = parts.iter().map(|p| p.size.down).fold(0.0, f64::max);
            Laid { size: Size { w, up, down }, shape: Shape::Seq(parts) }
        }
        Node::Choice(v) => choice(v.iter().map(|x| lay(x, names)).collect()),
        Node::Opt(x) => choice(vec![skip(), lay(x, names)]),
        Node::Rep(x) => {
            let inner = lay(x, names);
            let s = Size { w: inner.size.w + 2.0 * R, up: inner.size.up, down: inner.size.down + VS + R };
            choice(vec![skip(), Laid { size: s, shape: Shape::Loop(Box::new(inner)) }])
        }
    }
}

fn skip() -> Laid {
    Laid { size: Size { w: 0.0, up: 0.0, down: 0.0 }, shape: Shape::Skip }
}

fn choice(alts: Vec<Laid>) -> Laid {
    let iw = alts.iter().map(|a| a.size.w).fold(0.0, f64::max);
    let up = alts[0].size.up;
    let mut down = alts[0].size.down;
    for a in &alts[1..] {
        // each alternative's line sits far enough below the one before, and the rails need their two radii
        down += VS + a.size.up.max(R) + a.size.down;
    }
    Laid { size: Size { w: iw + 4.0 * R, up, down: down.max(if alts.len() > 1 { 2.0 * R } else { 0.0 }) }, shape: Shape::Choice(alts) }
}

/// A number as SVG writes it: at most one decimal, no `-0`.
fn f(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    let r = if r == 0.0 { 0.0 } else { r };
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The id a production's diagram has on the page (`rule-` and its name).
pub fn rule_id(name: &str) -> String {
    format!("rule-{name}")
}

/// The id a token class's entry has (`class-` and its name).
pub fn class_id(name: &str) -> String {
    format!("class-{name}")
}

/// A horizontal line along `y` (none when it would be no length).
fn line(out: &mut String, x1: f64, y: f64, x2: f64, _y2: f64) {
    if (x1 - x2).abs() > 0.01 {
        out.push_str(&format!("<path class=\"rr-line\" d=\"M{} {}H{}\"/>", f(x1), f(y), f(x2)));
    }
}

fn draw(l: &Laid, x: f64, y: f64, out: &mut String) {
    match &l.shape {
        Shape::Skip => {}
        Shape::Box { text, kind } => {
            let (cls, rx) = match kind {
                Kind::Term => ("rr-term", BH),
                Kind::Rule => ("rr-rule", 3.0),
                Kind::Class => ("rr-class", 3.0),
            };
            let w = l.size.w;
            let inner = format!(
                "<g class=\"{cls}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\"/><text x=\"{}\" y=\"{}\">{}</text></g>",
                f(x),
                f(y - BH),
                f(w),
                f(2.0 * BH),
                f(rx),
                f(x + w / 2.0),
                f(y + 4.5),
                esc(text)
            );
            match kind {
                Kind::Term => out.push_str(&inner),
                Kind::Rule => out.push_str(&format!("<a href=\"#{}\">{inner}</a>", esc(&rule_id(text)))),
                Kind::Class => out.push_str(&format!("<a href=\"#{}\">{inner}</a>", esc(&class_id(text)))),
            }
        }
        Shape::Seq(parts) => {
            let mut cx = x;
            for (i, p) in parts.iter().enumerate() {
                draw(p, cx, y, out);
                cx += p.size.w;
                if i + 1 < parts.len() {
                    line(out, cx, y, cx + GAP, y);
                    cx += GAP;
                }
            }
        }
        Shape::Choice(alts) => {
            let w = l.size.w;
            let iw = w - 4.0 * R;
            // the first alternative, on the line
            line(out, x, y, x + 2.0 * R, y);
            draw(&alts[0], x + 2.0 * R, y, out);
            line(out, x + 2.0 * R + alts[0].size.w, y, x + w, y);
            let mut below = y + alts[0].size.down;
            for a in &alts[1..] {
                let ay = below + VS + a.size.up.max(R);
                below = ay + a.size.down;
                // down from the line on the left, along, and up again on the right
                out.push_str(&format!(
                    "<path class=\"rr-line\" d=\"M{} {}a{r} {r} 0 0 1 {r} {r}V{}a{r} {r} 0 0 0 {r} {r}\"/>",
                    f(x),
                    f(y),
                    f(ay - R),
                    r = f(R)
                ));
                draw(a, x + 2.0 * R, ay, out);
                line(out, x + 2.0 * R + a.size.w, ay, x + 2.0 * R + iw, ay);
                out.push_str(&format!(
                    "<path class=\"rr-line\" d=\"M{} {}a{r} {r} 0 0 0 {r} -{r}V{}a{r} {r} 0 0 1 {r} -{r}\"/>",
                    f(x + 2.0 * R + iw),
                    f(ay),
                    f(y + R),
                    r = f(R)
                ));
            }
        }
        Shape::Loop(inner) => {
            let w = l.size.w;
            line(out, x, y, x + R, y);
            draw(inner, x + R, y, out);
            line(out, x + R + inner.size.w, y, x + w, y);
            // the way back: from the end, down under the part and back up to its start
            let ly = y + inner.size.down + VS;
            out.push_str(&format!(
                "<path class=\"rr-line rr-back\" d=\"M{} {}a{r} {r} 0 0 1 {r} {r}V{}a{r} {r} 0 0 1 -{r} {r}H{}a{r} {r} 0 0 1 -{r} -{r}V{}a{r} {r} 0 0 1 {r} -{r}\"/>",
                f(x + R + inner.size.w),
                f(y),
                f(ly - R),
                f(x + R),
                f(y + R),
                r = f(R)
            ));
        }
    }
}

/// One production's diagram, a whole `<svg>`: a start mark, the production, an end mark. `name` is its accessible name.
pub fn svg(name: &str, n: &Node, names: &dyn Names) -> String {
    let l = lay(n, names);
    let margin = 10.0;
    let lead = 20.0;
    let w = l.size.w + 2.0 * lead + 2.0 * margin;
    let h = l.size.up + l.size.down + 2.0 * margin;
    let y = margin + l.size.up;
    let mut out = format!(
        "<svg class=\"rr\" xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"railroad diagram of {}\">",
        f(w),
        f(h),
        f(w),
        f(h),
        esc(name)
    );
    // the start and the end: a double bar each
    let x0 = margin;
    out.push_str(&format!("<path class=\"rr-line rr-end\" d=\"M{} {}v{}M{} {}v{}\"/>", f(x0), f(y - 8.0), f(16.0), f(x0 + 4.0), f(y - 8.0), f(16.0)));
    line(&mut out, x0 + 4.0, y, x0 + lead, y);
    draw(&l, x0 + lead, y, &mut out);
    let xe = x0 + lead + l.size.w;
    line(&mut out, xe, y, xe + lead - 4.0, y);
    out.push_str(&format!("<path class=\"rr-line rr-end\" d=\"M{} {}v{}M{} {}v{}\"/>", f(xe + lead - 4.0), f(y - 8.0), f(16.0), f(xe + lead), f(y - 8.0), f(16.0)));
    out.push_str("</svg>");
    out
}

/// The diagrams' colours, light and dark, from the app's design tokens (btwsite/web/css/btw.css: the stage, the panel
/// and line colours, the accent; the code font).
pub const CSS: &str = r#".rr{--rr-line:#5d6585;--rr-term-bg:#16213e;--rr-term-ink:#eceef6;--rr-term-edge:#e94560;--rr-rule-bg:#12192f;--rr-rule-ink:#88aaff;--rr-rule-edge:#243055;--rr-class-bg:transparent;--rr-class-ink:#56b6c2;--rr-class-edge:#56b6c2;max-width:100%;height:auto;overflow:visible}
@media (prefers-color-scheme:light){:root:not([data-theme=dark]) .rr{--rr-line:#8d95b0;--rr-term-bg:#fff4f6;--rr-term-ink:#1a1a2e;--rr-term-edge:#e94560;--rr-rule-bg:#eef2ff;--rr-rule-ink:#2848b8;--rr-rule-edge:#9fb0e0;--rr-class-ink:#21808c;--rr-class-edge:#21808c}}
:root[data-theme=light] .rr{--rr-line:#8d95b0;--rr-term-bg:#fff4f6;--rr-term-ink:#1a1a2e;--rr-term-edge:#e94560;--rr-rule-bg:#eef2ff;--rr-rule-ink:#2848b8;--rr-rule-edge:#9fb0e0;--rr-class-ink:#21808c;--rr-class-edge:#21808c}
.rr .rr-line{fill:none;stroke:var(--rr-line);stroke-width:1.5}
.rr .rr-end{stroke-width:2}
.rr .rr-back{stroke-dasharray:none}
.rr text{font:13px ui-monospace,"SF Mono",SFMono-Regular,Menlo,Monaco,Consolas,"DejaVu Sans Mono",monospace;text-anchor:middle}
.rr .rr-term rect{fill:var(--rr-term-bg);stroke:var(--rr-term-edge);stroke-width:1.5}
.rr .rr-term text{fill:var(--rr-term-ink)}
.rr .rr-rule rect{fill:var(--rr-rule-bg);stroke:var(--rr-rule-edge);stroke-width:1.5}
.rr .rr-rule text{fill:var(--rr-rule-ink)}
.rr .rr-class rect{fill:var(--rr-class-bg);stroke:var(--rr-class-edge);stroke-width:1.5;stroke-dasharray:4 3}
.rr .rr-class text{fill:var(--rr-class-ink)}
.rr a:hover rect,.rr a:focus rect{stroke-width:2.5}
.rr a:focus{outline:none}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::ebnf::read;

    #[test]
    fn sizes_add_up() {
        let g = read("a = \"x\" ;").unwrap();
        let l = lay(&g.rules[0].node, &g);
        assert_eq!(l.size, Size { w: CW + 2.0 * PAD, up: BH, down: BH });
        let g = read("a = \"x\" b | \"yy\" ;\nb = \"z\" ;").unwrap();
        let l = lay(&g.rules[0].node, &g);
        // the wider alternative and two rails each side; the second alternative below the first
        assert_eq!(l.size.w, (CW + 2.0 * PAD + GAP + CW + 2.0 * PAD) + 4.0 * R);
        assert_eq!(l.size.down, BH + VS + BH + BH);
    }

    #[test]
    fn names_link_and_text_is_escaped() {
        let g = read("a = b \"<&>\" NAME ;\nb = \"z\" ;").unwrap();
        let s = svg("a", &g.rules[0].node, &g);
        assert!(s.contains("<a href=\"#rule-b\"><g class=\"rr-rule\">"), "{s}");
        assert!(s.contains("<a href=\"#class-NAME\"><g class=\"rr-class\">"), "{s}");
        assert!(s.contains("&lt;&amp;&gt;") && !s.contains("<&>"));
        assert!(s.starts_with("<svg class=\"rr\"") && s.ends_with("</svg>"));
    }
}
