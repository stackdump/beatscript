//! Which mapping suits which MIDI port: by name.
//!
//! A Mixxx mapping names its device three ways: `<info><name>`, `<controller id="…">`, and its
//! file's name. A port is named by the OS (`Hercules DJControl Inpulse 200`, or ALSA's
//! `"DJControl Inpulse 200:DJControl Inpulse 200 MIDI 1 20:0"`). Both are cut into words
//! (letters and digits kept together, noise words like `MIDI` and `USB` dropped) and a mapping
//! matches a port when one's words are a run of the other's. The longest run wins, so
//! `Inpulse 200` beats `Inpulse`; a run must be at least two words and six characters, so a
//! brand alone never matches.
//!
//! A config override (`port text` → `mapping`) beats the name match, and `none` keeps a port
//! from getting a mapping at all.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// What discovery knows about a mapping file without loading it.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Where it is (a path, as the host names it).
    pub id: String,
    /// The file's name, `Hercules_DJControl_Inpulse_200.midi.xml`.
    pub file: String,
    /// `<info><name>`.
    pub name: String,
    /// `<controller id>`.
    pub controller_id: String,
}

const NOISE: &[&str] = &["midi", "usb", "port", "input", "output", "in", "out", "controller", "controllers", "device", "mapping", "preset"];

/// Words of `text`: lowercase letters and digits, runs split at anything else, noise dropped.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            cur.extend(ch.to_lowercase());
        } else if !cur.is_empty() {
            out.push(core::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|w| !NOISE.contains(&w.as_str()));
    out
}

/// The length of the longest run of words of `a` that is a run of `b` (in characters, 0 when
/// the run is under two words).
fn run(a: &[String], b: &[String]) -> usize {
    let mut best = 0;
    for i in 0..a.len() {
        for j in 0..b.len() {
            let mut k = 0;
            let mut chars = 0;
            while i + k < a.len() && j + k < b.len() && a[i + k] == b[j + k] {
                chars += a[i + k].len();
                k += 1;
            }
            if k >= 2 && chars > best {
                best = chars;
            }
        }
    }
    best
}

/// How well `c` names `port`: 0 for not at all. Higher is better.
pub fn score(port: &str, c: &Candidate) -> usize {
    let p = words(port);
    let stem = c.file.strip_suffix(".midi.xml").unwrap_or(&c.file);
    let mut best = 0;
    for name in [c.name.as_str(), c.controller_id.as_str(), stem] {
        let n = words(name);
        // a mapping's words must be inside the port's, or the port's inside the mapping's
        // (a short port name), and the run must be long enough to mean something
        let r = run(&n, &p);
        if r < 6 {
            continue;
        }
        let whole_n = n.iter().map(String::len).sum::<usize>();
        let whole_p = p.iter().map(String::len).sum::<usize>();
        // the run covers all of the mapping's words, or all of the port's: otherwise it is a
        // shared fragment (`Numark` and `Mixtrack` out of two different units)
        if r != whole_n && r != whole_p {
            continue;
        }
        // exact beats containment; of two containments the longer wins
        let exact = if whole_n == whole_p { 1000 } else { 0 };
        best = best.max(exact + r * 10);
    }
    best
}

/// A config override: a port whose name contains `port` (any case) gets the mapping `mapping`
/// names (a file name, with or without `.midi.xml`, or part of the mapping's name); `none`
/// gives it no mapping.
#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub port: String,
    pub mapping: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick<'a> {
    /// An override chose it.
    Override(&'a Candidate),
    /// The name matched it.
    Name(&'a Candidate),
    /// An override says this port gets nothing.
    Disabled,
    /// An override names a mapping that is not there.
    OverrideMissing(String),
    None,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// The mapping a file name (with or without `.midi.xml`), a path, or a name (whole, then a part) says.
pub fn find<'a>(cands: &'a [Candidate], text: &str) -> Option<&'a Candidate> {
    let t = lower(text.trim());
    let t = t.strip_suffix(".midi.xml").unwrap_or(&t).to_string();
    cands
        .iter()
        .find(|c| lower(&c.file).strip_suffix(".midi.xml").unwrap_or(&lower(&c.file)) == t || lower(&c.id) == t)
        .or_else(|| cands.iter().find(|c| lower(&c.name) == t))
        .or_else(|| cands.iter().find(|c| lower(&c.name).contains(&t) || lower(&c.file).contains(&t)))
}

/// The mapping for `port`.
pub fn pick<'a>(port: &str, cands: &'a [Candidate], overrides: &[Override]) -> Pick<'a> {
    let lp = lower(port);
    for o in overrides {
        if !o.port.trim().is_empty() && lp.contains(&lower(o.port.trim())) {
            if o.mapping.trim().eq_ignore_ascii_case("none") {
                return Pick::Disabled;
            }
            return match find(cands, &o.mapping) {
                Some(c) => Pick::Override(c),
                None => Pick::OverrideMissing(o.mapping.clone()),
            };
        }
    }
    cands
        .iter()
        .map(|c| (score(port, c), c))
        .filter(|(s, _)| *s > 0)
        // ties: the shorter file name (the plainer mapping), then the name, so the pick is stable
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.file.len().cmp(&a.1.file.len())).then_with(|| b.1.file.cmp(&a.1.file)))
        .map(|(_, c)| Pick::Name(c))
        .unwrap_or(Pick::None)
}
