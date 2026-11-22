//! Where each section plays, in bars and seconds: from the compile when it is done (exact: tempo curves, `stack` / `then`,
//! evolve generations), else from the syntax (a plain `arrange` of sections whose lengths are numbers or `let`s, at the
//! program's one tempo), so hover and the inlay hints have an answer while the check is still running.

use crate::analysis::Analysis;
use crate::symbols::Kind;

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    /// The bar it starts on, 0-based.
    pub start: u32,
    pub bars: u32,
    /// Seconds from the start of the song to its start, and how long it plays (None: no tempo).
    pub at_secs: Option<f64>,
    pub secs: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    /// Every placement of every section, in time order.
    pub placed: Vec<Placed>,
    pub total_bars: Option<u32>,
    pub total_secs: Option<f64>,
    /// The program's tempo (its first, for a curve).
    pub tempo: Option<f64>,
    /// From the compile (true) or the syntax (false).
    pub exact: bool,
}

impl Timeline {
    pub fn first(&self, name: &str) -> Option<&Placed> {
        self.placed.iter().find(|p| p.name == name || p.name.split('+').any(|n| n == name))
    }

    pub fn count(&self, name: &str) -> usize {
        self.placed.iter().filter(|p| p.name == name || p.name.split('+').any(|n| n == name)).count()
    }
}

/// `m:ss`, or `h:mm:ss` past the hour.
pub fn mmss(secs: f64) -> String {
    let t = secs.round().max(0.0) as u64;
    let (h, m, s) = (t / 3600, (t / 60) % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A number the syntax says: a literal, or a `let` of one.
pub fn number_of(a: &Analysis, word: &str) -> Option<f64> {
    if let Ok(n) = word.parse::<f64>() {
        return Some(n);
    }
    // a scalar let: the number the parser folded it to (`let n = 4 * 2` is 8)
    if let Some(l) = a.scalar_lets.iter().rev().find(|l| l.name == word) {
        return Some(l.value);
    }
    let (_, l) = a.symbols.named(word).find(|(_, s)| s.kind == Kind::Let)?;
    l.detail.trim().parse::<f64>().ok()
}

/// A section's length in bars as its header says it.
pub fn section_bars(a: &Analysis, sym: &crate::symbols::Symbol) -> Option<u32> {
    let w = sym.detail.strip_suffix(" bars").or_else(|| sym.detail.strip_suffix(" bar"))?;
    number_of(a, w.trim()).filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| n as u32)
}

/// The program's tempo as its top-level `tempo` line says it (a number, or the first of a ramp).
pub fn syntax_tempo(a: &Analysis) -> Option<f64> {
    let t = a.symbols.symbols.iter().find(|s| s.kind == Kind::Setting && s.name == "tempo" && s.parent.is_none())?;
    let first = t.detail.split_whitespace().next()?;
    number_of(a, first)
}

pub fn timeline(a: &Analysis) -> Timeline {
    if let Some(c) = a.check_ready().and_then(|c| c.compiled.as_ref()) {
        if c.arranged && !c.flat_clock {
            let mut placed: Vec<Placed> = Vec::new();
            for s in c.segments.iter().filter(|s| s.header.is_some()) {
                match placed.last_mut() {
                    Some(last) if s.chain => last.bars += s.bars,
                    _ => placed.push(Placed { name: s.name.clone(), start: s.start, bars: s.bars, at_secs: None, secs: None }),
                }
            }
            let has_tempo = c.tempo.is_some();
            for p in &mut placed {
                if has_tempo {
                    p.at_secs = Some(c.tempo_map.seconds_at(p.start));
                    p.secs = Some(c.tempo_map.seconds_between(p.start, p.start + p.bars));
                }
            }
            let total = c.song_bars();
            let total_secs = total.filter(|_| has_tempo).map(|b| c.tempo_map.seconds_between(0u32, b));
            return Timeline { placed, total_bars: total, total_secs, tempo: c.tempo, exact: true };
        }
    }
    // the syntax: a plain arrange of sections
    let tempo = syntax_tempo(a);
    let secs = |bars: u32| tempo.map(|t| f64::from(bars) * 4.0 * 60.0 / t.max(1.0));
    let Some(arr) = a.symbols.symbols.iter().find(|s| s.kind == Kind::Arrange && s.parent.is_none()) else { return Timeline { tempo, ..Timeline::default() } };
    let mut placed = Vec::new();
    let mut bar = 0u32;
    for w in arr.detail.split_whitespace() {
        let Some((_, sec)) = a.symbols.named(w).find(|(_, s)| s.kind == Kind::Section) else { return Timeline { tempo, ..Timeline::default() } };
        let Some(bars) = section_bars(a, sec) else { return Timeline { tempo, ..Timeline::default() } };
        placed.push(Placed { name: w.to_string(), start: bar, bars, at_secs: secs(bar), secs: secs(bars) });
        bar += bars;
    }
    Timeline { placed, total_bars: Some(bar), total_secs: secs(bar), tempo, exact: false }
}
