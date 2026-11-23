//! F6 Euclidean rhythms and polymeter (docs/BEATSCRIPT-EXTENSIONS.md, docs/beatscript/ext-meter.md).
//!
//! * `e(k, n[, rot])` is checked here ([`eval_euclid`]): `0 <= k <= n`, `1 <= n <= 64`, any whole rotation; `check` names
//!   it when the BeatScript ontology does ([`euclid_name`]).
//! * `role/N` ([`Parser::part_cycle`]): the part's cycle is `N` sixteenth steps, not 16. Its hits are unrolled over
//!   `lcm(N, 16)` steps into the bar cycles of the net ([`part_bars`]); the program's period is reported by `check`
//!   ([`report_period`]) and exposed ([`period_of`]).
//!
//! Pure: no I/O, no clock. The ontology table below is a constant copied from the published vocabulary.

use std::collections::BTreeMap;

use djengine::gen::euclidean::{bjorklund, rotated};

use crate::ast::*;
use crate::compile::{Compiler, DeckIr, MAX_BARS};
use crate::diag::{Diagnostic, Span};
use crate::expand::{FlatLine, Lowered};
use crate::lex::Tok;
use crate::parse::Parser;
use crate::pattern::{self, Ctx, Ev, Sound, NO_SPAN};

/// The cycle of a part with no `/N`: a bar of sixteenth steps.
pub const BAR_STEPS: u32 = 16;
/// The longest cycle `role/N` takes, in sixteenth steps.
pub const MAX_CYCLE: u32 = 64;
/// The most steps one `e(k, n)` spreads its hits over.
pub const MAX_EUCLID_STEPS: i64 = 64;

// ---------------------------------------------------------------------------------------------------------------
// the ontology's Euclidean rhythms

/// The namespace of the BeatScript ontology (https://beatscript.app/ontology/ns#, forked from the bitwrap ontology 0.2.0).
pub const ONTOLOGY_NS: &str = "https://beatscript.app/ontology/ns#";
/// Where the ontology keeps its rhythm individuals: `<RHYTHM_NS><genre>-<role>`.
pub const RHYTHM_NS: &str = "https://beatscript.app/ontology/rhythm/";

/// The ontology's class of Euclidean rhythms: "K pulses spread as evenly as possible over N steps, then rotated".
pub fn euclid_class_iri() -> &'static str {
    "https://beatscript.app/ontology/ns#EuclideanRhythm"
}

/// The ontology's properties of a Euclidean rhythm, for a JSON-LD export (F14): `(pulses, steps, rotation, midiNote,
/// forRole)`, all in [`ONTOLOGY_NS`].
pub const EUCLID_PROPERTY_IRIS: [&str; 5] = [
    "https://beatscript.app/ontology/ns#pulses",
    "https://beatscript.app/ontology/ns#steps",
    "https://beatscript.app/ontology/ns#rotation",
    "https://beatscript.app/ontology/ns#midiNote",
    "https://beatscript.app/ontology/ns#forRole",
];

/// One Euclidean rhythm the BeatScript ontology names: an individual of `ns:EuclideanRhythm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EuclidTerm {
    /// The ontology's own words for it: "house snare" (the label is "house snare: E(3,8) rotation 0").
    pub label: &'static str,
    /// Its IRI: `https://beatscript.app/ontology/rhythm/house-snare`.
    pub iri: &'static str,
    /// The drum role it is for (`ns:forRole`): kick, snare or hihat.
    pub role: &'static str,
    pub pulses: u32,
    pub steps: u32,
    pub rotation: u32,
    /// `ns:midiNote`.
    pub midi_note: u8,
}

impl EuclidTerm {
    /// `E(3,8) rotation 0`, as the ontology's label says it.
    pub fn parameters(&self) -> String {
        format!("E({},{}) rotation {}", self.pulses, self.steps, self.rotation)
    }
}

/// Where the table came from. It is the 57 `ns:EuclideanRhythm` individuals of the published ontology
/// (https://beatscript.app/ontology/ns, Turtle, read on 2026-11-17): the genres' kick, snare and hihat rhythms. The
/// ontology has no traditional names (tresillo, cinquillo ...): a pattern is named when it is one of these.
pub const EUCLID_TABLE_SOURCE: &str = "https://beatscript.app/ontology/ns (57 ns:EuclideanRhythm individuals, read 2026-11-17)";

/// Every Euclidean rhythm the ontology names, in the order the ontology lists them (by genre, then kick, snare,
/// hihat).
pub const EUCLID_TABLE: [EuclidTerm; 57] = [
    EuclidTerm { label: "ambient kick", iri: "https://beatscript.app/ontology/rhythm/ambient-kick", role: "kick", pulses: 2, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "ambient snare", iri: "https://beatscript.app/ontology/rhythm/ambient-snare", role: "snare", pulses: 1, steps: 16, rotation: 8, midi_note: 38 },
    EuclidTerm { label: "ambient hihat", iri: "https://beatscript.app/ontology/rhythm/ambient-hihat", role: "hihat", pulses: 3, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "blues kick", iri: "https://beatscript.app/ontology/rhythm/blues-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "blues snare", iri: "https://beatscript.app/ontology/rhythm/blues-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "blues hihat", iri: "https://beatscript.app/ontology/rhythm/blues-hihat", role: "hihat", pulses: 6, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "bossa kick", iri: "https://beatscript.app/ontology/rhythm/bossa-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "bossa snare", iri: "https://beatscript.app/ontology/rhythm/bossa-snare", role: "snare", pulses: 5, steps: 16, rotation: 3, midi_note: 37 },
    EuclidTerm { label: "bossa hihat", iri: "https://beatscript.app/ontology/rhythm/bossa-hihat", role: "hihat", pulses: 6, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "country kick", iri: "https://beatscript.app/ontology/rhythm/country-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "country snare", iri: "https://beatscript.app/ontology/rhythm/country-snare", role: "snare", pulses: 4, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "country hihat", iri: "https://beatscript.app/ontology/rhythm/country-hihat", role: "hihat", pulses: 8, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "dnb kick", iri: "https://beatscript.app/ontology/rhythm/dnb-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "dnb snare", iri: "https://beatscript.app/ontology/rhythm/dnb-snare", role: "snare", pulses: 2, steps: 8, rotation: 2, midi_note: 38 },
    EuclidTerm { label: "dnb hihat", iri: "https://beatscript.app/ontology/rhythm/dnb-hihat", role: "hihat", pulses: 7, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "dubstep kick", iri: "https://beatscript.app/ontology/rhythm/dubstep-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "dubstep snare", iri: "https://beatscript.app/ontology/rhythm/dubstep-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "dubstep hihat", iri: "https://beatscript.app/ontology/rhythm/dubstep-hihat", role: "hihat", pulses: 5, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "edm kick", iri: "https://beatscript.app/ontology/rhythm/edm-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "edm snare", iri: "https://beatscript.app/ontology/rhythm/edm-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 40 },
    EuclidTerm { label: "edm hihat", iri: "https://beatscript.app/ontology/rhythm/edm-hihat", role: "hihat", pulses: 8, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "funk kick", iri: "https://beatscript.app/ontology/rhythm/funk-kick", role: "kick", pulses: 5, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "funk snare", iri: "https://beatscript.app/ontology/rhythm/funk-snare", role: "snare", pulses: 4, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "funk hihat", iri: "https://beatscript.app/ontology/rhythm/funk-hihat", role: "hihat", pulses: 8, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "garage kick", iri: "https://beatscript.app/ontology/rhythm/garage-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "garage snare", iri: "https://beatscript.app/ontology/rhythm/garage-snare", role: "snare", pulses: 3, steps: 16, rotation: 2, midi_note: 38 },
    EuclidTerm { label: "garage hihat", iri: "https://beatscript.app/ontology/rhythm/garage-hihat", role: "hihat", pulses: 7, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "house kick", iri: "https://beatscript.app/ontology/rhythm/house-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "house snare", iri: "https://beatscript.app/ontology/rhythm/house-snare", role: "snare", pulses: 3, steps: 8, rotation: 0, midi_note: 38 },
    EuclidTerm { label: "house hihat", iri: "https://beatscript.app/ontology/rhythm/house-hihat", role: "hihat", pulses: 6, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "jazz kick", iri: "https://beatscript.app/ontology/rhythm/jazz-kick", role: "kick", pulses: 3, steps: 12, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "jazz snare", iri: "https://beatscript.app/ontology/rhythm/jazz-snare", role: "snare", pulses: 2, steps: 12, rotation: 3, midi_note: 38 },
    EuclidTerm { label: "jazz hihat", iri: "https://beatscript.app/ontology/rhythm/jazz-hihat", role: "hihat", pulses: 5, steps: 12, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "lofi kick", iri: "https://beatscript.app/ontology/rhythm/lofi-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "lofi snare", iri: "https://beatscript.app/ontology/rhythm/lofi-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "lofi hihat", iri: "https://beatscript.app/ontology/rhythm/lofi-hihat", role: "hihat", pulses: 5, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "metal kick", iri: "https://beatscript.app/ontology/rhythm/metal-kick", role: "kick", pulses: 8, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "metal snare", iri: "https://beatscript.app/ontology/rhythm/metal-snare", role: "snare", pulses: 4, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "metal hihat", iri: "https://beatscript.app/ontology/rhythm/metal-hihat", role: "hihat", pulses: 8, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "reggae kick", iri: "https://beatscript.app/ontology/rhythm/reggae-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "reggae snare", iri: "https://beatscript.app/ontology/rhythm/reggae-snare", role: "snare", pulses: 2, steps: 16, rotation: 6, midi_note: 37 },
    EuclidTerm { label: "reggae hihat", iri: "https://beatscript.app/ontology/rhythm/reggae-hihat", role: "hihat", pulses: 8, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "speedcore kick", iri: "https://beatscript.app/ontology/rhythm/speedcore-kick", role: "kick", pulses: 8, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "speedcore snare", iri: "https://beatscript.app/ontology/rhythm/speedcore-snare", role: "snare", pulses: 4, steps: 16, rotation: 2, midi_note: 40 },
    EuclidTerm { label: "speedcore hihat", iri: "https://beatscript.app/ontology/rhythm/speedcore-hihat", role: "hihat", pulses: 12, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "synthwave kick", iri: "https://beatscript.app/ontology/rhythm/synthwave-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "synthwave snare", iri: "https://beatscript.app/ontology/rhythm/synthwave-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "synthwave hihat", iri: "https://beatscript.app/ontology/rhythm/synthwave-hihat", role: "hihat", pulses: 4, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "techno kick", iri: "https://beatscript.app/ontology/rhythm/techno-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "techno snare", iri: "https://beatscript.app/ontology/rhythm/techno-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 38 },
    EuclidTerm { label: "techno hihat", iri: "https://beatscript.app/ontology/rhythm/techno-hihat", role: "hihat", pulses: 5, steps: 8, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "trance kick", iri: "https://beatscript.app/ontology/rhythm/trance-kick", role: "kick", pulses: 4, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "trance snare", iri: "https://beatscript.app/ontology/rhythm/trance-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 40 },
    EuclidTerm { label: "trance hihat", iri: "https://beatscript.app/ontology/rhythm/trance-hihat", role: "hihat", pulses: 8, steps: 16, rotation: 0, midi_note: 42 },
    EuclidTerm { label: "trap kick", iri: "https://beatscript.app/ontology/rhythm/trap-kick", role: "kick", pulses: 3, steps: 16, rotation: 0, midi_note: 36 },
    EuclidTerm { label: "trap snare", iri: "https://beatscript.app/ontology/rhythm/trap-snare", role: "snare", pulses: 2, steps: 16, rotation: 4, midi_note: 40 },
    EuclidTerm { label: "trap hihat", iri: "https://beatscript.app/ontology/rhythm/trap-hihat", role: "hihat", pulses: 10, steps: 16, rotation: 0, midi_note: 42 },
];

/// The ontology's rhythms with these pulses, steps and rotation (the rotation taken modulo the steps, as the engine's
/// `rotated` does), in the order of [`EUCLID_TABLE`]. The role is not asked: `E(3,8)` is one pattern whatever plays it.
pub fn euclid_names(k: i64, n: i64, rot: i64) -> Vec<&'static EuclidTerm> {
    if n < 1 || k < 0 {
        return Vec::new();
    }
    let rot = rot.rem_euclid(n);
    EUCLID_TABLE.iter().filter(|t| i64::from(t.pulses) == k && i64::from(t.steps) == n && i64::from(t.rotation) == rot).collect()
}

/// The first rhythm of the ontology with these pulses, steps and rotation, if it names one.
pub fn euclid_name(k: i64, n: i64, rot: i64) -> Option<EuclidTerm> {
    euclid_names(k, n, rot).first().map(|t| **t)
}

/// The rhythms of the ontology with these parameters that are for this drum role (`kick`, `snare`, `hihat`).
pub fn euclid_names_for_role(role: &str, k: i64, n: i64, rot: i64) -> Vec<&'static EuclidTerm> {
    euclid_names(k, n, rot).into_iter().filter(|t| t.role == role).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// e(k, n, rot)

/// `e(k, n)` / `e(k, n, rot)` / `euclid(...)`: the hits of Bjorklund's spread of `k` over `n` steps, rotated, each at
/// its step's place in the slot. Every way it can be wrong is an error with a line and column and what to write.
pub(crate) fn eval_euclid(c: &Call, pos: &[&Arg], ctx: &mut Ctx) -> Vec<Ev> {
    const USAGE: &str = "e(5,8) or e(3,8,2): hits, steps, rotation (0 <= hits <= steps; 1 to 64 steps; any whole rotation)";
    if !pattern::arity(c, ctx, USAGE, 2, 3, pos.len()) {
        return Vec::new();
    }
    let (Some(k), Some(n)) = (pattern::int_arg(ctx, pos[0], "the hit count", 0, MAX_EUCLID_STEPS), pattern::int_arg(ctx, pos[1], "the step count", 1, MAX_EUCLID_STEPS)) else {
        return Vec::new();
    };
    if k > n {
        ctx.err(
            Diagnostic::error(c.span, format!("{k} hits do not fit in {n} steps"))
                .help(format!("at most {n} hits fit in {n} steps (e({n},{n}) is every step); e(5,8) is five hits over eight steps")),
        );
        return Vec::new();
    }
    let rot = match pos.get(2) {
        Some(a) => match pattern::num_arg(ctx, a, "a whole rotation") {
            Some(v) if v.fract() == 0.0 && v.abs() <= 1e9 => v as i64,
            Some(_) => {
                ctx.err(Diagnostic::error(a.span(), "the rotation is a whole number of steps (negative turns it the other way)").found(a.span().text(ctx.src).to_string()).help("e(3,8,2): the same three hits turned two steps"));
                return Vec::new();
            }
            None => return Vec::new(),
        },
        None => 0,
    };
    let pat = rotated(&bjorklund(k, n), rot);
    pat.iter()
        .enumerate()
        .filter(|(_, v)| **v == 1)
        .map(|(i, _)| Ev { at: i as f64 / n as f64, sound: Sound::Hit, vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// role/N

impl Parser<'_> {
    /// `/N` right after a role word (the `role` is already read): the part's cycle in sixteenth steps. `/16` is the
    /// bar, the default, and reads as no cycle at all (the canonical text does not print it).
    pub(crate) fn part_cycle(&mut self, role: &Word) -> Result<Option<(u32, Span)>, Diagnostic> {
        if self.peek() != &Tok::Slash {
            return Ok(None);
        }
        let slash = self.span();
        if slash.start != role.span.end {
            return Err(Diagnostic::error(slash, format!("a part's cycle is written glued to its role: {}/12", role.text)).help(format!("{}/12 x x ~ x: twelve sixteenths a cycle, with no space round the `/`", role.text)));
        }
        self.bump();
        let at = self.span();
        if self.peek() == &Tok::LParen {
            // `hihat/(4 + 8)`: arithmetic, the way a repeat count takes it (`x*(1 + 1)`); a def's argument that is arithmetic is put
            // in place in parentheses, so `hihat/$n` with `n = 4 + 8` is this
            if at.start != slash.end {
                return Err(Diagnostic::error(at, format!("a part's cycle is written glued to its role: {}/12", role.text)).help(format!("{}/12 x x ~ x: no space after the `/`", role.text)));
            }
            let (n, sp) = self.paren_int("a cycle")?;
            if n < 1 || n > i64::from(MAX_CYCLE) {
                return Err(Diagnostic::error(sp, format!("a cycle is a whole number of sixteenth steps from 1 to {MAX_CYCLE}")).found(sp.text(self.src).to_string()).help(format!("{}/12: twelve steps a cycle; {}/16 is the bar", role.text, role.text)));
            }
            let n = n as u32;
            return Ok((n != BAR_STEPS).then(|| (n, role.span.to(sp))));
        }
        let Tok::Num { value, int } = self.peek().clone() else {
            return Err(self.expected(&["a number of sixteenth steps, 1 to 64"], &format!("expected the cycle's length in steps after `{}/`, found {}", role.text, self.found())).help(format!("{}/12 x x ~ x", role.text)));
        };
        if at.start != slash.end {
            return Err(Diagnostic::error(at, format!("a part's cycle is written glued to its role: {}/12", role.text)).help(format!("{}/12 x x ~ x: no space after the `/`", role.text)));
        }
        self.bump();
        if !int || value < 1.0 || value > f64::from(MAX_CYCLE) {
            return Err(Diagnostic::error(at, format!("a cycle is a whole number of sixteenth steps from 1 to {MAX_CYCLE}")).found(at.text(self.src).to_string()).help(format!("{}/12: twelve steps a cycle; {}/16 is the bar", role.text, role.text)));
        }
        let n = value as u32;
        Ok((n != BAR_STEPS).then(|| (n, role.span.to(at))))
    }
}

/// What a part's cycle is: its `/N` steps, else the bar's 16.
pub fn cycle_steps(part: &Part) -> u32 {
    part.cycle.map_or(BAR_STEPS, |(n, _)| n)
}

/// `/12` for a part with its own cycle, nothing for a bar's: the text after the role in the canonical form.
pub fn cycle_suffix(part: &Part) -> String {
    part.cycle.map_or(String::new(), |(n, _)| format!("/{n}"))
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// `lcm` of two, None past `u64`.
pub(crate) fn lcm(a: u64, b: u64) -> Option<u64> {
    if a == 0 || b == 0 {
        return Some(0);
    }
    a.checked_mul(b / gcd(a, b))
}

/// How many bars the net takes to say this part once all the way round, alternations (`<a b>`, counted in the part's own
/// cycles) and the cycle against the bar both counted: `lcm(alternation * N, 16) / 16`. A part with no `/N` is
/// [`pattern::period`] exactly. None when it passes [`crate::compile::MAX_SONG_BARS`].
pub fn part_bars(part: &Part) -> Option<u64> {
    // pattern sugar is its longhand (sugar.rs): `iter(p, 4)` is four bars, `poly(8, [x ~ ~])` three
    let long;
    let part = if crate::sugar::has_sugar(&part.pattern) {
        long = crate::sugar::part(part).0;
        &long
    } else {
        part
    };
    let a = pattern::period(&part.pattern)?;
    let n = u64::from(cycle_steps(part));
    if n == u64::from(BAR_STEPS) {
        return Some(a);
    }
    let steps = a.checked_mul(n)?;
    let bars = lcm(steps, u64::from(BAR_STEPS))? / u64::from(BAR_STEPS);
    (bars <= crate::compile::MAX_SONG_BARS).then_some(bars)
}

/// Where the net's bars and steps are, for a hit `step` (0-based, in the part's cycle steps) of pattern cycle `c`: the
/// bar of the net it falls in and its sixteenth step there. `period` is the net's length in bars.
pub(crate) fn place(c: usize, step: i64, cycle_steps: u32, period: usize) -> (usize, u8) {
    let abs = (c as i64) * i64::from(cycle_steps) + step;
    let bar = abs.div_euclid(16).rem_euclid(period as i64) as usize;
    (bar, abs.rem_euclid(16) as u8)
}

/// How many pattern cycles a part says in a net of `period` bars.
pub(crate) fn cycles_in_net(period: usize, cycle_steps: u32) -> usize {
    (period * 16 / cycle_steps as usize).max(1)
}

// ---------------------------------------------------------------------------------------------------------------
// the program's period

/// How a program's parts come round: the bars and steps (sixteenths) until everything plays the same again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Period {
    pub bars: u64,
    pub steps: u64,
    /// The parts with a cycle of their own, `hihat/12`, in the order met.
    pub polymeter: Vec<String>,
    /// The parts with chance in them (`kick on deck A`), in the order met. A chance part differs every bar, so a program
    /// with any has no period: `bars` is what the rest comes round in, the same wherever the section is placed (L-10).
    pub chance: Vec<String>,
}

impl Period {
    /// `3 bars = 48 steps`.
    pub fn text(&self) -> String {
        format!("{} bar{} = {} steps", self.bars, if self.bars == 1 { "" } else { "s" }, self.steps)
    }
}

/// The period of a set of decks' nets: the least common multiple of their lengths in bars (each net is the least
/// common multiple of its parts' cycles with the bar, and of their alternations), with the parts that have a cycle of
/// their own named.
pub fn period_of(decks: &BTreeMap<Deck, DeckIr>) -> Period {
    let mut bars = 1u64;
    let mut poly: Vec<String> = Vec::new();
    let mut chance: Vec<String> = Vec::new();
    for (d, ir) in decks {
        // a deck with chance is unrolled over the section's bars to say its dice: that length is no period, so the deck
        // counts by what its parts come round in without them (a deck with none counts by its net, as it always did)
        let deck_bars = if ir.parts.iter().any(|p| p.chance) { ir.parts.iter().fold(1u64, |a, p| lcm(a, p.structural_bars).unwrap_or(u64::MAX)) } else { ir.period as u64 };
        bars = lcm(bars, deck_bars).unwrap_or(u64::MAX);
        for p in &ir.parts {
            if p.cycle_steps != BAR_STEPS {
                let t = format!("{}/{}", p.role, p.cycle_steps);
                if !poly.contains(&t) {
                    poly.push(t);
                }
            }
            if p.chance {
                let t = format!("{} on deck {}", p.role, d.name());
                if !chance.contains(&t) {
                    chance.push(t);
                }
            }
        }
    }
    Period { bars, steps: bars.saturating_mul(16), polymeter: poly, chance }
}

/// The period of a compiled program: of its nets (a song with sections: of each section, in order, by name).
pub fn program_period(c: &crate::compile::Compiled) -> Vec<(String, Period)> {
    if c.arranged {
        c.segments.iter().filter(|s| !s.chain).map(|s| (s.name.clone(), period_of(&s.decks))).collect()
    } else {
        vec![(String::new(), period_of(&c.decks))]
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// "a, b and c".
fn and_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// A scope's parts, by deck.
type ScopeParts<'a> = BTreeMap<Deck, Vec<&'a Part>>;

/// The parts of a scope, by deck: the section's own and the song's, the section's winning on a role.
fn scope_parts<'a>(globals: &'a [FlatLine], own: &'a [FlatLine]) -> ScopeParts<'a> {
    let mut by: ScopeParts = BTreeMap::new();
    for fl in own.iter().chain(globals.iter()) {
        let Body::Parts(ps) = &fl.line.body else { continue };
        let Some((Target::Deck(d), _)) = fl.line.target else { continue };
        let slot = by.entry(d).or_default();
        for p in ps {
            // (a right copy, `jux`, is the part's twin: it comes round with the deck as any part does)
            if p.right || !slot.iter().any(|q| q.role.text == p.role.text) {
                slot.push(p);
            }
        }
    }
    by
}

fn has_cycle(parts: &ScopeParts) -> bool {
    parts.values().flatten().any(|p| p.cycle.is_some())
}

/// What a scope's parts come to: (bars, the parts with their own cycle, the first such part's span).
fn scope_period(parts: &ScopeParts) -> Option<(u64, Vec<String>, Span)> {
    let mut bars = 1u64;
    let mut poly: Vec<String> = Vec::new();
    let mut at = None;
    for ps in parts.values() {
        let mut deck_bars = 1u64;
        for p in ps {
            deck_bars = lcm(deck_bars, part_bars(p)?)?;
            if let Some((n, sp)) = p.cycle {
                let t = format!("{}/{n}", p.role.text);
                if !poly.contains(&t) {
                    poly.push(t);
                }
                at.get_or_insert(sp);
            }
        }
        bars = lcm(bars, deck_bars)?;
    }
    Some((bars, poly, at?))
}

/// The note `check` makes of a program with parts of their own cycle: `period: 3 bars = 48 steps (hihat/12 against the
/// bar)`, once for the program, or once for each section of a song. A warning in severity, never an error.
pub(crate) fn report_period(c: &mut Compiler, lw: &Lowered) {
    let mut scopes: Vec<(Option<&str>, ScopeParts)> = Vec::new();
    if lw.arranged {
        for seg in &lw.segments {
            scopes.push((Some(seg.name.as_str()), scope_parts(&lw.globals, &seg.lines)));
        }
    } else {
        scopes.push((None, scope_parts(&lw.globals, &[])));
    }
    let mut seen: Vec<String> = Vec::new();
    for (name, parts) in scopes {
        if !has_cycle(&parts) {
            continue;
        }
        let Some((bars, poly, at)) = scope_period(&parts) else { continue };
        let msg = format!("period: {} bar{} = {} steps ({} against the bar{}){}", bars, plural(bars), bars * 16, and_list(&poly), if poly.len() > 1 { " and each other" } else { "" }, name.map_or(String::new(), |n| format!(", in section {n}")));
        if !seen.contains(&msg) {
            seen.push(msg.clone());
            c.note(at, msg);
        }
    }
}

impl Compiler<'_> {
    /// A note (severity `Info`: what the check found, nothing wrong) said once for its place and words, however many scopes
    /// build the same part.
    pub(crate) fn note(&mut self, span: Span, msg: String) {
        if !self.diags.iter().any(|d| d.span == span && d.message == msg) {
            self.diags.push(Diagnostic::info(span, msg));
        }
    }

    /// The same, for a thing that may be wrong (a value that is held to a range, a division that can be by zero): a warning.
    pub(crate) fn warn_note(&mut self, span: Span, msg: String) {
        if !self.diags.iter().any(|d| d.span == span && d.message == msg) {
            self.diags.push(Diagnostic::warning(span, msg));
        }
    }

    /// A deck whose parts have a cycle of their own must fit one net: if the parts' cycles, the bar and their
    /// alternations take more than [`MAX_BARS`] to come round, that is an error naming the parts and the way out.
    pub(crate) fn polymeter_fits(&mut self, deck: Deck, parts: &[&Part], period: u64) -> bool {
        if period <= MAX_BARS || !parts.iter().any(|p| p.cycle.is_some()) {
            return true;
        }
        let each: Vec<String> = parts
            .iter()
            .filter(|p| p.cycle.is_some())
            .map(|p| format!("{}{} ({} bar{})", p.role.text, cycle_suffix(p), part_bars(p).unwrap_or(0), plural(part_bars(p).unwrap_or(0))))
            .collect();
        let at = parts.iter().find_map(|p| p.cycle.map(|(_, sp)| sp)).unwrap_or_default();
        self.err(
            Diagnostic::error(at, format!("deck {}'s parts take {period} bars to come round: {} against the bar; a net is at most {MAX_BARS}", deck.name(), and_list(&each)))
                .help("the period is the least common multiple of the cycles and the 16-step bar: give the parts cycles that share a factor with each other and with 16 (/8, /12, /24, /32, /48 are a few bars at most), or put them on two decks"),
        );
        false
    }

    /// `e(3,8)` and `e(4,16)` that the ontology names (for the part's own drum role) are said, once each, to a program that is
    /// linked to the ontology (it has a `#! genre:` or `#! tags:` line). A program that never asked is not told: there is no
    /// information level for a note, so these were warnings on every `e(4,16)` of every old program (R-20).
    pub(crate) fn euclid_notes(&mut self, part: &Part) {
        if !matches!(part.role.text.as_str(), "kick" | "snare" | "hihat") {
            return;
        }
        if crate::linked::read_links(self.src).is_empty() {
            return;
        }
        let mut found: Vec<(Span, String)> = Vec::new();
        visit_calls(&part.pattern, &mut |call| {
            if !matches!(call.name.text.as_str(), "e" | "euclid") {
                return;
            }
            let num = |i: usize| match call.args.get(i) {
                Some(Arg::Num(n)) if n.value.fract() == 0.0 && n.value.abs() <= 1e9 => Some(n.value as i64),
                _ => None,
            };
            let (Some(k), Some(n)) = (num(0), num(1)) else { return };
            let rot = if call.args.len() > 2 { num(2) } else { Some(0) };
            let Some(rot) = rot else { return };
            let names = euclid_names_for_role(&part.role.text, k, n, rot);
            let Some(first) = names.first() else { return };
            let text = if call.args.len() > 2 { format!("e({k},{n},{rot})") } else { format!("e({k},{n})") };
            let genres: Vec<String> = names.iter().map(|t| t.label.trim_end_matches(part.role.text.as_str()).trim().to_string()).collect();
            let msg = if names.len() == 1 {
                format!("{text}: the ontology's {} rhythm ({})", first.label, first.iri)
            } else {
                format!("{text}: the ontology's {} rhythm of {} ({} and {} more)", part.role.text, and_list(&genres), first.iri, names.len() - 1)
            };
            found.push((call.span, msg));
        });
        for (sp, msg) in found {
            self.note(sp, msg);
        }
    }
}

/// Every call (a generator or a transform) in a pattern, outermost first, through groups, alternations, arguments and the
/// terms of F7's chance.
pub(crate) fn visit_calls<'a>(seq: &'a Seq, f: &mut impl FnMut(&'a Call)) {
    for t in &seq.terms {
        visit_term_calls(t, f);
    }
}

fn visit_term_calls<'a>(t: &'a Term, f: &mut impl FnMut(&'a Call)) {
    match &t.atom {
        Atom::Group(seqs) => seqs.iter().for_each(|s| visit_calls(s, f)),
        Atom::Alt(ts) => ts.iter().for_each(|x| visit_term_calls(x, f)),
        Atom::Call(c) => visit_call(c, f),
        Atom::Chance(c) => visit_term_calls(&c.term, f),
        Atom::Sized(s) => visit_term_calls(&s.term, f),
        Atom::Choose(c) => c.options.iter().for_each(|x| visit_term_calls(x, f)),
        _ => {}
    }
}

fn visit_call<'a>(c: &'a Call, f: &mut impl FnMut(&'a Call)) {
    f(c);
    for a in &c.args {
        visit_arg(a, f);
    }
}

fn visit_arg<'a>(a: &'a Arg, f: &mut impl FnMut(&'a Call)) {
    match a {
        Arg::Pat(s) => visit_calls(s, f),
        Arg::Call(c) => visit_call(c, f),
        Arg::Named(_, v) => visit_arg(v, f),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part_of(src: &str) -> Part {
        let p = crate::parse::parse(src);
        assert!(p.ok(), "{:?}", p.diagnostics);
        let StmtKind::Line(l) = &p.program.stmts[0].kind else { panic!("a line") };
        let Body::Parts(ps) = &l.body else { panic!("parts") };
        ps[0].clone()
    }

    #[test]
    fn lcm_is_checked() {
        assert_eq!(lcm(12, 16), Some(48));
        assert_eq!(lcm(7, 9), Some(63));
        assert_eq!(lcm(0, 5), Some(0));
        assert_eq!(lcm(u64::MAX, 2), None);
        assert_eq!(and_list(&[]), "");
        assert_eq!(and_list(&["a".into()]), "a");
        assert_eq!(and_list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }

    #[test]
    fn a_part_comes_round_over_lcm_of_its_cycle_and_the_bar() {
        for (n, bars) in [(16u32, 1u64), (12, 3), (7, 7), (1, 1), (64, 4), (48, 3), (63, 63), (10, 5), (32, 2)] {
            let p = part_of(&format!("deck A: hihat/{n} x\n"));
            assert_eq!(part_bars(&p), Some(bars), "/{n}");
            assert_eq!(cycle_steps(&p), n);
        }
        // alternations are counted in the part's own cycles
        assert_eq!(part_bars(&part_of("deck A: hihat/12 <x ~>\n")), Some(3));
        assert_eq!(part_bars(&part_of("deck A: hihat/12 <x ~ x>\n")), Some(9));
        assert_eq!(part_bars(&part_of("deck A: hihat <x ~ x>\n")), Some(3), "no cycle of its own: as before");
        assert_eq!(cycle_suffix(&part_of("deck A: hihat/12 x\n")), "/12");
        assert_eq!(cycle_suffix(&part_of("deck A: hihat/16 x\n")), "");
    }

    #[test]
    fn a_hit_is_placed_in_the_net_by_its_absolute_step() {
        // the first cycle of twelve: step 5 is bar 0 step 5; the second cycle begins at step 12: its step 5 is 17, bar 1 step 1
        assert_eq!(place(0, 5, 12, 3), (0, 5));
        assert_eq!(place(1, 5, 12, 3), (1, 1));
        // the fourth cycle of twelve is steps 36..48 = bar 2; the fifth wraps to the net's start
        assert_eq!(place(3, 11, 12, 3), (2, 15));
        assert_eq!(place(4, 0, 12, 3), (0, 0));
        // a hit rounded up to the end of the bar belongs to the next
        assert_eq!(place(0, 16, 16, 4), (1, 0));
        assert_eq!(place(3, 16, 16, 4), (0, 0));
        assert_eq!(cycles_in_net(3, 12), 4);
        assert_eq!(cycles_in_net(1, 16), 1);
        assert_eq!(cycles_in_net(1, 8), 2);
        assert_eq!(cycles_in_net(21, 7), 48);
    }

    #[test]
    fn the_table_is_in_the_ontologys_order_and_rhythms_are_found_by_their_parameters() {
        let first = EUCLID_TABLE.first().unwrap();
        assert_eq!((first.label, first.pulses, first.steps, first.rotation), ("ambient kick", 2, 16, 0));
        let last = EUCLID_TABLE.last().unwrap();
        assert_eq!((last.label, last.pulses, last.steps, last.rotation), ("trap hihat", 10, 16, 0));
        for t in EUCLID_TABLE.iter() {
            assert!(euclid_names(i64::from(t.pulses), i64::from(t.steps), i64::from(t.rotation)).contains(&t));
            assert_eq!(euclid_name(i64::from(t.pulses), i64::from(t.steps), i64::from(t.rotation)).map(|f| (f.pulses, f.steps, f.rotation)), Some((t.pulses, t.steps, t.rotation)));
        }
        assert_eq!(euclid_names(5, 8, 0).iter().map(|t| t.label).collect::<Vec<_>>(), vec!["lofi hihat", "techno hihat"]);
        assert_eq!(euclid_names(3, 16, 0).len(), 9);
        assert_eq!(EUCLID_PROPERTY_IRIS.len(), 5);
        assert!(EUCLID_PROPERTY_IRIS.iter().all(|i| i.starts_with(ONTOLOGY_NS)));
    }
}
