//! Inlay hints: what the text implies and does not say.
//!
//! - after a section's header: the bar it starts on and when (`bar 9 · 0:15`), every placement when it is placed more than
//!   once, or that it is not placed;
//! - after the `arrange`: the song's length (`64 bars · 2:04`);
//! - after an `evolve` header: how many generations its section plays (`4 gens`), and `⚠ only 3 of 9 gens fit` when its
//!   `stop when gen == N` asks for more than the section's bars hold;
//! - after a part's pattern: its steps (`8 steps`; `x*8` is eight, `[a b]` one step split in two).
//!
//! Positions are where the hint is drawn, with a space before it (`paddingLeft`).

use djengine::json::{JsonObject, JsonValue};
use djlive::ast::{Atom, Body, EvolveBlock, Part, Stmt, StmtKind};

use crate::analysis::Analysis;
use crate::json::{obj, pos_json, s};
use crate::symbols::Kind;
use crate::text::Encoding;
use crate::timeline::{mmss, section_bars, timeline};

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    /// The byte the hint is drawn at.
    pub at: usize,
    pub label: String,
    pub tooltip: String,
    /// A warning, not a plain fact (drawn the same; its label starts with ⚠).
    pub warn: bool,
}

/// How many steps a pattern takes: its top-level terms, `x*8` as eight.
pub fn steps(p: &Part) -> u32 {
    p.pattern.terms.iter().map(|t| if matches!(t.atom, Atom::Rest | Atom::Hit | Atom::Note(_) | Atom::Degree(_) | Atom::ScaleDeg(_)) { t.fast.unwrap_or(1) } else { 1 }).sum()
}

fn walk<'a>(stmts: &'a [Stmt], section: Option<&'a str>, out: &mut Vec<(Option<&'a str>, &'a Stmt)>) {
    for st in stmts {
        out.push((section, st));
        if let StmtKind::Section(sb) = &st.kind {
            walk(&sb.body, Some(&sb.name.text), out);
        }
    }
}

/// Every hint of the document.
pub fn hints(a: &Analysis) -> Vec<Hint> {
    let mut out = Vec::new();
    let tl = timeline(a);
    let header_end = |sym: &crate::symbols::Symbol| -> usize {
        // after `16 bars`, before the `{`
        sym.body.map_or(sym.full.end, |b| a.text[..b.start].trim_end().len())
    };
    for sym in a.symbols.symbols.iter().filter(|s| s.kind == Kind::Section) {
        let places: Vec<&crate::timeline::Placed> = tl.placed.iter().filter(|p| p.name == sym.name || p.name.split('+').any(|n| n == sym.name)).collect();
        let (label, tooltip) = match places.as_slice() {
            [] if tl.total_bars.is_some() || a.symbols.symbols.iter().any(|s| s.kind == Kind::Arrange) => {
                if tl.placed.is_empty() {
                    continue;
                }
                ("not placed".to_string(), "no arrange places this section".to_string())
            }
            [] => continue,
            [p] => (
                match p.at_secs {
                    Some(t) => format!("bar {} · {}", p.start + 1, mmss(t)),
                    None => format!("bar {}", p.start + 1),
                },
                format!("{} bars, bars {}–{}", p.bars, p.start + 1, p.start + p.bars),
            ),
            many => (format!("bars {}", many.iter().map(|p| (p.start + 1).to_string()).collect::<Vec<_>>().join(", ")), format!("placed {} times", many.len())),
        };
        out.push(Hint { at: header_end(sym), label, tooltip, warn: false });
    }
    if let Some(arr) = a.symbols.symbols.iter().find(|s| s.kind == Kind::Arrange && s.parent.is_none()) {
        if let Some(b) = tl.total_bars {
            let label = match tl.total_secs {
                Some(t) => format!("{b} bars · {}", mmss(t)),
                None => format!("{b} bars"),
            };
            out.push(Hint { at: arr.full.end, label, tooltip: "the song's length".into(), warn: false });
        }
    }
    // evolves and parts, from the program as parsed
    let mut stmts = Vec::new();
    walk(&a.program.stmts, None, &mut stmts);
    for (section, st) in &stmts {
        match &st.kind {
            StmtKind::Evolve(ev) => {
                if let Some(h) = evolve_hint(a, ev, *section, st) {
                    out.push(h);
                }
            }
            StmtKind::Line(line) => {
                if let Body::Parts(parts) = &line.body {
                    for p in parts {
                        let n = steps(p);
                        let cycle = p.cycle.map_or(String::new(), |(c, _)| format!(" of /{c}"));
                        out.push(Hint { at: p.pattern.span.end, label: format!("{n} step{}{cycle}", if n == 1 { "" } else { "s" }), tooltip: format!("`{}` takes {n} steps a cycle", p.role.text), warn: false });
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|h| h.at);
    out
}

fn evolve_hint(a: &Analysis, ev: &EvolveBlock, section: Option<&str>, st: &Stmt) -> Option<Hint> {
    let at = a.text[st.span.start..].find('{').map_or(st.span.end, |k| a.text[..st.span.start + k].trim_end().len());
    let at = at.max(st.span.start);
    let every = ev.every.max(1);
    // the bars the evolve's scope plays: its section's
    let bars = section.and_then(|n| a.symbols.named(n).find(|(_, s)| s.kind == Kind::Section).and_then(|(_, s)| section_bars(a, s)))?;
    let fit = bars.div_ceil(every);
    // the generations the compile placed, when it is done
    let played = a.check_ready().and_then(|c| c.compiled.as_ref()).map(|c| c.lineage.iter().filter(|g| Some(g.scope.as_str()) == section).count() as u32).filter(|n| *n > 0);
    match ev.last {
        Some((last, _)) if last + 1 > fit => Some(Hint {
            at,
            label: format!("⚠ only {fit} of {} gens fit", last + 1),
            tooltip: format!("generations 0 to {last} every {every} bars take {} bars; the section is {bars}: the last {} never play", (last + 1) * every, last + 1 - fit),
            warn: true,
        }),
        Some((last, _)) => Some(Hint { at, label: format!("{} gens", last + 1), tooltip: format!("generations 0 to {last}, one every {every} bars ({} of the section's {bars} bars)", (last + 1) * every), warn: false }),
        None => {
            let n = played.unwrap_or(fit);
            Some(Hint { at, label: format!("{n} gens"), tooltip: format!("a generation every {every} bars over the section's {bars} bars"), warn: false })
        }
    }
}

/// `textDocument/inlayHint` for the hints in `range`.
pub fn inlay_hints(a: &Analysis, range: djlive::Span, enc: Encoding) -> JsonValue {
    JsonValue::Array(
        hints(a)
            .into_iter()
            .filter(|h| h.at >= range.start && h.at <= range.end)
            .map(|h| {
                let mut o = JsonObject::new();
                o.set("position", pos_json(a.index.position(&a.text, h.at, enc)));
                o.set("label", s(h.label));
                o.set("kind", JsonValue::int(1));
                o.set("paddingLeft", JsonValue::Bool(true));
                if !h.tooltip.is_empty() {
                    o.set("tooltip", obj([("kind", s("plaintext")), ("value", s(h.tooltip))]));
                }
                JsonValue::Object(o)
            })
            .collect(),
    )
}
