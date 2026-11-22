//! `evolve`: a program that applies transforms to itself over time (docs/BTW.md, "Evolving").
//!
//! Generation 0 is the parts as written. Generation n+1 is the rule's transforms applied to generation
//! n's parts (their realized hits: step and note), so generation n is f_n(...f_1(gen 0)), and with a rule
//! that does not mention `gen` it is the composite iterated, f^n, in the monoid of transforms. Every
//! transform is an endomorphism of a part (it takes a part to a part of the same shape), the rule is
//! applied in the order written, and anything random is a hash of (seed, generation, bar, step, note), so
//! the same text is the same generations every time. The evolution is bounded by the text (`stop when
//! gen == N`, `for N gens`): there is no open-ended evolve.
//!
//! A generation's CID is the CID of its nets (content only). A rule that changes nothing (an identity, or a
//! fixed point) gives the same CID every generation; each generation also records the CID of the one it
//! came from, which is the lineage chain a fork's `forkedFrom` is made of.

use std::collections::BTreeMap;

use djengine::cid;
use djengine::json::{JsonObject, JsonValue};

use crate::ast::*;
use crate::compile::{fnv, DeckIr, Hit, PartIr};
use crate::diag::Diagnostic;

pub const TRANSFORMS: [&str; 6] = ["rotate", "transpose", "octave", "invert", "reverse", "thin"];
/// Generations an evolve may make.
pub const MAX_GENS: u32 = 256;

fn drum(role: &str) -> bool {
    matches!(role, "kick" | "snare" | "hihat" | "clap")
}

/// The number a transform's `i`th argument is in generation `gen` (an alternation `<0 5 7 0>` takes one per
/// generation, round).
fn arg_at(op: &EvolveOp, i: usize, gen: u32) -> Option<f64> {
    match op.args.get(i)? {
        EvolveArg::Num(n) => Some(n.value),
        EvolveArg::Alt(v) => v.get((gen.max(1) as usize - 1) % v.len()).map(|n| n.value),
    }
}

/// What is wrong with a rule before anything is evolved, or nothing.
pub fn check(block: &EvolveBlock, decks: &BTreeMap<Deck, DeckIr>, diags: &mut Vec<Diagnostic>) {
    if let Some((n, sp)) = block.last {
        if n == 0 || n > MAX_GENS {
            diags.push(Diagnostic::error(sp, format!("an evolve makes 1 to {MAX_GENS} generations")).found(n.to_string()));
        }
    }
    for r in &block.rules {
        let matches_any = decks.iter().any(|(d, ir)| match &r.target {
            EvolveTarget::Deck(x) => x == d,
            EvolveTarget::Role(w) => ir.parts.iter().any(|p| p.role == w.text),
        });
        if !matches_any {
            let what = match &r.target {
                EvolveTarget::Role(w) => format!("no `{}` part here", w.text),
                EvolveTarget::Deck(d) => format!("no deck {} here", d.name()),
            };
            diags.push(Diagnostic::error(r.span, format!("{what} for this rule to change")).help("an evolve changes parts of its own scope: the section or program it is in"));
        }
        for op in &r.ops {
            let name = op.name.text.as_str();
            if !TRANSFORMS.contains(&name) {
                let mut d = Diagnostic::error(op.name.span, format!("`{name}` is not a transform of an evolve")).expected(&TRANSFORMS.map(|t| t)).found(format!("`{name}`"));
                if let Some(n) = crate::parse::nearest(name, TRANSFORMS) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                diags.push(d);
                continue;
            }
            let (want_args, usage) = match name {
                "rotate" => (1, "rotate 1: steps (sixteenths) to move every hit along its bar"),
                "transpose" => (1, "transpose 5, or transpose <0 5 7 0>: semitones, one per generation"),
                "octave" => (1, "octave 1"),
                "thin" => (1, "thin 0.1 seed=3: the share of hits to drop"),
                _ => (0, ""),
            };
            if op.args.len() != want_args {
                diags.push(Diagnostic::error(op.span, format!("`{name}` takes {} argument{}", want_args, if want_args == 1 { "" } else { "s" })).help(usage.to_string()));
            }
            if op.seed.is_some() && name != "thin" {
                diags.push(Diagnostic::error(op.span, format!("`{name}` has no seed: only `thin` is random")));
            }
            for a in &op.args {
                let vals: Vec<&Num> = match a {
                    EvolveArg::Num(n) => vec![n],
                    EvolveArg::Alt(v) => v.iter().collect(),
                };
                for n in vals {
                    let ok = match name {
                        "rotate" => n.int && (-15.0..=15.0).contains(&n.value),
                        "transpose" => n.int && (-48.0..=48.0).contains(&n.value),
                        "octave" => n.int && (-4.0..=4.0).contains(&n.value),
                        "thin" => (0.0..=1.0).contains(&n.value),
                        _ => true,
                    };
                    if !ok {
                        diags.push(Diagnostic::error(n.span, format!("`{name}` takes {}", match name { "rotate" => "a whole number of steps, -15 to 15", "transpose" => "a whole number of semitones, -48 to 48", "octave" => "a whole number of octaves, -4 to 4", _ => "a share 0 to 1" })));
                    }
                }
            }
            if matches!(name, "transpose" | "octave" | "invert") {
                let hits_a_drum = decks.iter().any(|(d, ir)| {
                    let on = match &r.target {
                        EvolveTarget::Deck(x) => x == d,
                        EvolveTarget::Role(_) => true,
                    };
                    on && ir.parts.iter().any(|p| drum(&p.role) && match &r.target {
                        EvolveTarget::Role(w) => w.text == p.role,
                        EvolveTarget::Deck(_) => false,
                    })
                });
                if hits_a_drum {
                    diags.push(Diagnostic::error(op.span, format!("`{name}` moves pitches: a drum part has none (a drum's note is the drum)")).help("rotate, reverse and thin work on drums"));
                }
            }
        }
    }
}

fn seed_of(op: &EvolveOp, role: &str) -> u32 {
    op.seed.map_or_else(|| fnv(role), |s| s.value as u32)
}

fn apply_op(part: &mut PartIr, op: &EvolveOp, gen: u32) {
    let name = op.name.text.as_str();
    let tonal = !drum(&part.role);
    match name {
        "rotate" => {
            let k = arg_at(op, 0, gen).unwrap_or(0.0) as i32;
            for c in part.grids_mut().into_iter().flatten() {
                for h in c.iter_mut() {
                    h.step = (i32::from(h.step) + k).rem_euclid(16) as u8;
                }
                c.sort_by_key(|h| (h.step, h.note));
            }
        }
        "reverse" => {
            for c in part.grids_mut().into_iter().flatten() {
                for h in c.iter_mut() {
                    h.step = 15 - h.step;
                }
                c.sort_by_key(|h| (h.step, h.note));
            }
        }
        "transpose" | "octave" if tonal => {
            let by = arg_at(op, 0, gen).unwrap_or(0.0) as i32 * if name == "octave" { 12 } else { 1 };
            for c in part.grids_mut().into_iter().flatten() {
                for h in c.iter_mut() {
                    h.note = (i32::from(h.note) + by).clamp(0, 127) as u8;
                }
            }
        }
        "invert" if tonal => {
            // about the part's first note: the axis the line is turned over
            let axis = part.cycles.iter().find_map(|c| c.first().map(|h| i32::from(h.note)));
            if let Some(a) = axis {
                for c in part.grids_mut().into_iter().flatten() {
                    for h in c.iter_mut() {
                        h.note = (2 * a - i32::from(h.note)).clamp(0, 127) as u8;
                    }
                    c.sort_by_key(|h| (h.step, h.note));
                }
            }
        }
        "thin" => {
            let p = arg_at(op, 0, gen).unwrap_or(0.0);
            let seed = seed_of(op, &part.role);
            for grid in part.grids_mut() {
                for (bar, c) in grid.iter_mut().enumerate() {
                    c.retain(|h: &Hit| {
                        let x = f64::from(fnv(&format!("{seed}:{gen}:{bar}:{}:{}", h.step, h.note))) / f64::from(u32::MAX);
                        x >= p
                    });
                }
            }
        }
        _ => {}
    }
}

/// One part one generation on: every rule whose target and `when` hold, in the order written.
pub fn step_part(block: &EvolveBlock, deck: Deck, part: &mut PartIr, gen: u32) {
    for r in &block.rules {
        let on = match &r.target {
            EvolveTarget::Role(w) => w.text == part.role,
            EvolveTarget::Deck(x) => *x == deck,
        };
        if !on || r.when.as_ref().is_some_and(|c| !c.holds(gen)) {
            continue;
        }
        for op in &r.ops {
            apply_op(part, op, gen);
            // a part with a twin (`jux`): both copies move
            if let Some(twin) = part.twin.as_deref_mut() {
                apply_op(twin, op, gen);
            }
        }
    }
}

/// One generation on from `prev`. The rules act on parts one at a time (a part's next state is a function of
/// its own), which is why a program's evolution decomposes: see [`analyse`].
pub fn step(block: &EvolveBlock, prev: &BTreeMap<Deck, DeckIr>, gen: u32) -> BTreeMap<Deck, DeckIr> {
    let mut next = prev.clone();
    for (d, ir) in next.iter_mut() {
        for part in ir.parts.iter_mut() {
            step_part(block, *d, part, gen);
        }
    }
    next
}

/// Where in a part's life the rules stop being new: the highest generation a guard compares against, and
/// how often the rest repeats (the moduli of the guards and the lengths of the `<a b c>` arguments).
fn rule_clock(block: &EvolveBlock, deck: Deck, role: &str) -> (u32, u64) {
    let mut threshold = 0u32;
    let mut round = 1u64;
    for r in &block.rules {
        let on = match &r.target {
            EvolveTarget::Role(w) => w.text == role,
            EvolveTarget::Deck(x) => *x == deck,
        };
        if !on {
            continue;
        }
        if let Some(c) = &r.when {
            match c.modulo {
                Some(m) => round = lcm64(round, u64::from(m.max(1))),
                None => threshold = threshold.max(c.value.saturating_add(1)),
            }
            if c.modulo.is_some() {
                threshold = threshold.max(0);
            }
        }
        for op in &r.ops {
            for a in &op.args {
                if let EvolveArg::Alt(v) = a {
                    round = lcm64(round, v.len() as u64);
                }
            }
        }
    }
    (threshold, round)
}

fn gcd64(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd64(b, a % b)
    }
}

fn lcm64(a: u64, b: u64) -> u64 {
    a / gcd64(a, b).max(1) * b
}

fn state_key(p: &PartIr) -> u128 {
    // two FNV-1a runs over the part's hits: a part's state, as a fingerprint
    let (mut a, mut b): (u64, u64) = (0xcbf2_9ce4_8422_2325, 0x8422_2325_cbf2_9ce4);
    let mut feed = |x: u64| {
        a = (a ^ x).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ x.rotate_left(17)).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
    };
    for (bar, c) in p.cycles.iter().enumerate() {
        feed(0xFFFF_0000 + bar as u64);
        for h in c {
            feed(u64::from(h.step) | u64::from(h.note) << 8 | u64::from(h.dur) << 16 | u64::from(h.vel) << 24);
        }
    }
    // a part with a twin is both copies' state
    if let Some(t) = &p.twin {
        feed(0xFFFF_FFFF_0000_0001);
        for (bar, c) in t.cycles.iter().enumerate() {
            feed(0xFFFF_0000 + bar as u64);
            for h in c {
                feed(u64::from(h.step) | u64::from(h.note) << 8 | u64::from(h.dur) << 16 | u64::from(h.vel) << 24);
            }
        }
    }
    u128::from(a) << 64 | u128::from(b)
}

/// How many generations of one part to look through for its orbit.
pub const SEARCH_GENS: u32 = 20_000;

/// A part's life under the rules: `tail` generations before it settles into a cycle of `period` generations
/// (a fixed point has period 1). `found` is false when the search ran out before the state came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orbit {
    pub deck: Deck,
    pub role: String,
    pub tail: u32,
    pub period: u32,
    pub found: bool,
}

/// The orbit of one part: iterate its generations until (its state, the generation mod the rule's own clock)
/// is one it has had, after the generation the guards stop changing.
pub fn orbit_of(block: &EvolveBlock, deck: Deck, part0: &PartIr) -> Orbit {
    let (threshold, round) = rule_clock(block, deck, &part0.role);
    // `thin` picks its hits by a hash of the generation itself, so a part it acts on is not a function of its
    // state and the clock: a repeated state may only be a generation where nothing happened to drop. What
    // thin does reach for certain is a part with no hits left, which every transform leaves empty: that is its
    // fixed point, and the only one it can prove.
    let thins = block.rules.iter().any(|r| {
        let on = match &r.target {
            EvolveTarget::Role(w) => w.text == part0.role,
            EvolveTarget::Deck(x) => *x == deck,
        };
        on && r.ops.iter().any(|o| o.name.text == "thin" && o.args.iter().any(|a| matches!(a, EvolveArg::Num(n) if n.value > 0.0)))
    });
    let empty = |p: &PartIr| p.cycles.iter().all(Vec::is_empty) && p.twin.as_ref().map_or(true, |t| t.cycles.iter().all(Vec::is_empty));
    let mut seen: BTreeMap<(u128, u64), u32> = BTreeMap::new();
    let mut part = part0.clone();
    if thins && empty(&part) {
        return Orbit { deck, role: part0.role.clone(), tail: 0, period: 1, found: true };
    }
    if !thins && threshold == 0 {
        seen.insert((state_key(&part), 0), 0);
    }
    for gen in 1..=SEARCH_GENS {
        step_part(block, deck, &mut part, gen);
        if thins {
            if empty(&part) {
                return Orbit { deck, role: part0.role.clone(), tail: gen, period: 1, found: true };
            }
            continue;
        }
        if gen < threshold {
            continue;
        }
        let key = (state_key(&part), u64::from(gen) % round);
        if let Some(first) = seen.get(&key) {
            return Orbit { deck, role: part0.role.clone(), tail: *first, period: gen - first, found: true };
        }
        seen.insert(key, gen);
    }
    Orbit { deck, role: part0.role.clone(), tail: SEARCH_GENS, period: 0, found: false }
}

/// What an evolve comes to: every part's orbit, and the program's (the longest tail, the least common
/// multiple of the periods: the parts do not interact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub parts: Vec<Orbit>,
    pub tail: u32,
    /// In generations. None: more than a u64 can hold, or a part's search ran out.
    pub period: Option<u64>,
    pub every: u32,
}

pub fn analyse(block: &EvolveBlock, gen0: &BTreeMap<Deck, DeckIr>) -> Analysis {
    let mut parts = Vec::new();
    for (d, ir) in gen0 {
        for p in &ir.parts {
            // a part no rule touches is a fixed point of period 1
            let touched = block.rules.iter().any(|r| match &r.target {
                EvolveTarget::Role(w) => w.text == p.role,
                EvolveTarget::Deck(x) => x == d,
            });
            parts.push(if touched { orbit_of(block, *d, p) } else { Orbit { deck: *d, role: p.role.clone(), tail: 0, period: 1, found: true } });
        }
    }
    let tail = parts.iter().map(|o| o.tail).max().unwrap_or(0);
    let mut period: Option<u64> = Some(1);
    for o in &parts {
        period = match (period, o.found) {
            (Some(p), true) => p.checked_mul(u64::from(o.period) / gcd64(p, u64::from(o.period)).max(1)),
            _ => None,
        };
    }
    Analysis { parts, tail, period, every: block.every }
}

impl Analysis {
    /// The period in bars.
    pub fn period_bars(&self) -> Option<u64> {
        self.period?.checked_mul(u64::from(self.every))
    }

    /// "tail 3 generations, period 8 generations (64 bars, about 2m04s at 124 BPM)".
    pub fn text(&self, scope: &str, bpm: f64) -> String {
        match self.period_bars() {
            Some(pb) => self.text_secs(scope, pb as f64 * 4.0 * 60.0 / bpm.max(1.0), &format!("at {bpm}")),
            None => self.text_secs(scope, 0.0, ""),
        }
    }

    /// The same with the period's seconds given (through a tempo curve, F3) and how they were worked out
    /// ("at 124", "with the tempo curve").
    pub fn text_secs(&self, scope: &str, secs: f64, how: &str) -> String {
        let tail_bars = u64::from(self.tail) * u64::from(self.every);
        match self.period_bars() {
            Some(pb) => format!(
                "evolve{}: tail {} generation{} ({} bars), then it cycles: period {} generation{} = period {} bars ≈ {} {}",
                if scope.is_empty() { String::new() } else { format!(" in {scope}") },
                self.tail,
                if self.tail == 1 { "" } else { "s" },
                group(tail_bars),
                group(self.period.unwrap_or(0)),
                if self.period == Some(1) { "" } else { "s" },
                group(pb),
                human(secs),
                how
            ),
            None => format!("evolve{}: no cycle found within {} generations of a part (or one too long to count)", if scope.is_empty() { String::new() } else { format!(" in {scope}") }, group(u64::from(SEARCH_GENS))),
        }
    }
}

/// 16016 as "16,016".
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Seconds as "8h36m", "2m04s", "45s".
pub fn human(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{sec:02}s")
    } else {
        format!("{sec}s")
    }
}

/// Generation `n` of an evolve from generation 0 and the rule alone: no chain of generations kept. A part
/// that has come round its cycle is brought back to where it is in it, so generation 10,000,000 costs no
/// more than the length of the orbit. None when a part's orbit was not found and `n` is out of reach to
/// walk.
pub fn nth(block: &EvolveBlock, gen0: &BTreeMap<Deck, DeckIr>, analysis: &Analysis, n: u32) -> Option<BTreeMap<Deck, DeckIr>> {
    let mut out = gen0.clone();
    for (d, ir) in out.iter_mut() {
        for part in ir.parts.iter_mut() {
            let o = analysis.parts.iter().find(|o| o.deck == *d && o.role == part.role);
            let upto = match o {
                Some(o) if o.found && n > o.tail => o.tail + (n - o.tail) % o.period.max(1),
                Some(o) if !o.found && n > SEARCH_GENS * 5 => return None,
                _ => n,
            };
            for gen in 1..=upto {
                step_part(block, *d, part, gen);
            }
        }
    }
    Some(out)
}

/// A generation's CID: of its nets, content only.
pub fn generation_cid(decks: &BTreeMap<Deck, DeckIr>) -> String {
    generation_cid_with(decks, &mut NetCids::default())
}

/// The net CIDs of the last generation, by deck: a deck no rule changed since (most generations of a drum deck whose
/// rule has a `when gen % 7 == k`) has the same net, and building a net's envelope to hash it is most of what a check of
/// a long evolving song costs (deep-house-60: ~5 s of its ~5.2 s). The CIDs are the same as without it.
#[derive(Default)]
pub struct NetCids {
    last: BTreeMap<Deck, (DeckIr, String)>,
}

impl NetCids {
    pub fn net_cid(&mut self, d: Deck, ir: &DeckIr) -> String {
        if let Some((prev, cid)) = self.last.get(&d) {
            if prev == ir {
                return cid.clone();
            }
        }
        let cid = ir.net_cid();
        self.last.insert(d, (ir.clone(), cid.clone()));
        cid
    }
}

/// [`generation_cid`], reusing the net CIDs of decks that did not change.
pub fn generation_cid_with(decks: &BTreeMap<Deck, DeckIr>, cache: &mut NetCids) -> String {
    let nets = JsonObject::from_pairs(decks.iter().map(|(d, ir)| (d.name(), JsonValue::str(cache.net_cid(*d, ir)))).collect::<Vec<_>>());
    cid::share_cid(&JsonValue::Object(JsonObject::from_pairs([("format", JsonValue::int(1)), ("nets", JsonValue::Object(nets)), ("type", JsonValue::str("djrust.live.gen"))])))
}
