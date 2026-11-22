//! Mini-notation, evaluated: a pattern and a cycle number give the events of that cycle, each at a
//! position 0..1 within the bar. Generators (`e`, `markov`, `arp`, `chord`) are terms like any
//! other and are built on the engine's own (`djengine::gen`), never forked.

use djengine::gen::core::{scale_notes, GenParams};
use djengine::gen::markov::markov_melody;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::music::{self, Chord, Key};
use crate::scale::Roman;

/// The most events one part may play in one bar (one cycle of its pattern). `x*64` inside a group inside a `*64` is
/// 4096; nested once more it is 262,144 and, a level on, a billion: the events were made before anything counted
/// them. A bar holds 16 steps, so this is far more than any part can use.
pub const MAX_EVENTS: usize = 4096;
/// The most terms one bar of a part may read (every term of every copy of a repeat counts), so that nested `*N` of
/// rests, which make no events, are bounded too.
pub const MAX_READS: usize = 8192;

/// What an event plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    /// `x`: the part's own note (a drum's note, a tonal part's root).
    Hit,
    Note(i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ev {
    /// 0 <= at < 1: where in the bar.
    pub at: f64,
    pub sound: Sound,
    /// A velocity the event brings itself (a generator's); None: the part's.
    pub vel: Option<i32>,
    pub accent: bool,
    pub slide: bool,
    /// The term's expression marks (ghost, flam, ratchet): [`Marks`].
    pub marks: Marks,
    /// Where it came from in the text: an index into the part's [`SpanTable`] (NO_SPAN: unknown).
    pub sid: u32,
}

pub const NO_SPAN: u32 = u32::MAX;

/// The source spans events trace back to: each entry is a chain, the innermost token first and the terms
/// that held it after (a group, a `let` use, a transform), so a hit knows every place in the text it is.
#[derive(Clone, Debug, Default)]
pub struct SpanTable {
    pub chains: Vec<Vec<crate::diag::Span>>,
    memo: std::collections::HashMap<(u32, usize, usize), u32>,
}

impl SpanTable {
    /// The chain `sid` with `span` after it.
    pub fn extend(&mut self, sid: u32, span: crate::diag::Span) -> u32 {
        if let Some(i) = self.memo.get(&(sid, span.start, span.end)) {
            return *i;
        }
        let mut chain = if sid == NO_SPAN { Vec::new() } else { self.chains[sid as usize].clone() };
        chain.push(span);
        self.chains.push(chain);
        let i = (self.chains.len() - 1) as u32;
        self.memo.insert((sid, span.start, span.end), i);
        i
    }
}

/// What a part knows when its pattern is read.
pub struct Ctx<'a> {
    pub key: Key,
    pub octave: i32,
    pub vel: i32,
    pub seed: u32,
    /// The note a hit plays in a tonal part (None in a drum part): what a transposed hit becomes.
    pub hit_root: Option<i32>,
    pub src: &'a str,
    pub diags: &'a mut Vec<Diagnostic>,
    pub spans: &'a mut SpanTable,
    /// Where the chance terms are (F7, chance.rs): the bar being read and the slot the term in hand fills.
    pub chance: crate::chance::ChanceCtx,
    /// The part's role as numerals see it, and its voice leading (F5, scale.rs).
    pub scale: crate::scale::ScaleCtx,
    /// Terms read in the bar being read ([`MAX_READS`]); [`eval_part`] sets it to 0 for each bar.
    pub reads: usize,
    /// The bar was more than a bar can hold ([`MAX_EVENTS`], [`MAX_READS`]) and the error is said: every term answers
    /// nothing from here on, so that the work stops instead of going on to the end of the multiplication.
    pub over: bool,
}

impl Ctx<'_> {
    pub(crate) fn err(&mut self, d: Diagnostic) {
        if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
            self.diags.push(d);
        }
    }

    /// Whether the events made so far (`n`) are more than a bar can hold. Said once, at the term `at` that was being
    /// read (the group, the `*N`), and the bar is over.
    pub(crate) fn full(&mut self, n: usize, at: Span) -> bool {
        if !self.over && n > MAX_EVENTS {
            self.over = true;
            self.err(
                Diagnostic::error(at, format!("this part plays more than {MAX_EVENTS} events in one bar"))
                    .help("nested `*N` repeats and groups multiply (`[[x*64]*64]*64` is 262,144 hits): a bar holds 16 steps, so write fewer or smaller repeats"),
            );
        }
        self.over
    }

    /// One more term read in this bar; false (said once, at `at`) when the bar has taken more than [`MAX_READS`].
    fn read(&mut self, at: Span) -> bool {
        self.reads += 1;
        if !self.over && self.reads > MAX_READS {
            self.over = true;
            self.err(
                Diagnostic::error(at, format!("this pattern takes more than {MAX_READS} steps to read in one bar"))
                    .help("nested `*N` repeats and groups multiply (`[[~*64]*64]*64` is 262,144 steps, hits or not): write fewer or smaller repeats"),
            );
        }
        !self.over
    }
}

/// The events of one bar of a part's pattern: [`eval_seq`] with the bar's own budget ([`MAX_READS`]).
pub fn eval_part(seq: &Seq, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    ctx.reads = 0;
    eval_seq(seq, cycle, ctx)
}

/// How many bars a pattern takes to come round (alternations make it longer). None: too long.
pub fn period(seq: &Seq) -> Option<u64> {
    let mut p = 1u64;
    for t in &seq.terms {
        p = lcm(p, term_period(t)?)?;
    }
    Some(p)
}

fn term_period(t: &Term) -> Option<u64> {
    let inner = match &t.atom {
        Atom::Group(seqs) | Atom::Deg(seqs) => {
            let mut p = 1;
            for s in seqs {
                p = lcm(p, period(s)?)?;
            }
            p
        }
        Atom::Alt(terms) => {
            let mut p = 1;
            for x in terms {
                p = lcm(p, term_period(x)?)?;
            }
            p.checked_mul(terms.len() as u64)?
        }
        Atom::Call(c) => call_period(c)?,
        Atom::Chance(c) => term_period(&c.term)?,
        // (sugar is counted written out, meter::part_bars: a `~n` reaching here is its step, as in eval_atom)
        Atom::Sized(s) => term_period(&s.term)?,
        Atom::Choose(c) => {
            let mut p = 1;
            for x in &c.options {
                p = lcm(p, term_period(x)?)?;
            }
            p
        }
        _ => 1,
    };
    match t.fast {
        // played k times a bar: the inner pattern comes round k times as fast
        Some(k) => Some(inner / gcd(inner, u64::from(k))),
        None => Some(inner),
    }
}

/// How many bars a transform makes of its pattern: `every(n, ..)` comes round every n, `slow(p, n)`
/// takes n times as long, the others as long as their pattern.
fn call_period(c: &Call) -> Option<u64> {
    let pats = || -> Option<u64> {
        let mut p = 1;
        for a in &c.args {
            if let Arg::Pat(s) = a {
                p = lcm(p, period(s)?)?;
            } else if let Arg::Call(inner) = a {
                p = lcm(p, call_period(inner)?)?;
            }
        }
        Some(p)
    };
    let int0 = || match c.args.first() {
        Some(Arg::Num(n)) if n.value >= 1.0 => Some(n.value as u64),
        _ => None,
    };
    let int1 = || match c.args.get(1) {
        Some(Arg::Num(n)) if n.value >= 1.0 => Some(n.value as u64),
        _ => None,
    };
    match c.name.text.as_str() {
        "every" => lcm(pats()?, int0().unwrap_or(1)),
        "slow" => pats()?.checked_mul(int1().unwrap_or(1)),
        "fast" => {
            let p = pats()?;
            let k = int1().unwrap_or(1);
            Some(p / gcd(p, k))
        }
        "transpose" | "octave" | "reverse" | "rotate" | "degrade" | "humanize" | "maybe" => pats(),
        // (the answer to a pattern comes round when the call and the reply both have)
        name if crate::answer::ANSWERS.contains(&name) => pats(),
        _ => Some(1),
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: u64, b: u64) -> Option<u64> {
    let l = a.checked_mul(b / gcd(a, b))?;
    (l <= crate::compile::MAX_SONG_BARS).then_some(l)
}

pub fn eval_seq(seq: &Seq, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    let n = seq.terms.len();
    let mut out = Vec::new();
    let outer = ctx.chance.slot;
    for (j, t) in seq.terms.iter().enumerate() {
        ctx.chance.enter(outer, j, n);
        for e in eval_term(t, cycle, ctx) {
            out.push(Ev { at: (j as f64 + e.at) / n as f64, ..e });
        }
        if ctx.full(out.len(), t.span) {
            ctx.chance.slot = outer;
            return Vec::new();
        }
    }
    ctx.chance.slot = outer;
    out
}

pub(crate) fn eval_term(t: &Term, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    if !ctx.read(t.span) {
        return Vec::new();
    }
    let mut evs = match t.fast {
        None => eval_atom(t, cycle, ctx),
        Some(k) => {
            let mut out = Vec::new();
            let outer = ctx.chance.slot;
            for i in 0..i64::from(k) {
                ctx.chance.enter(outer, i as usize, k as usize);
                for e in eval_atom(t, cycle * i64::from(k) + i, ctx) {
                    out.push(Ev { at: (i as f64 + e.at) / f64::from(k), ..e });
                }
                if ctx.full(out.len(), t.span) {
                    ctx.chance.slot = outer;
                    return Vec::new();
                }
            }
            ctx.chance.slot = outer;
            out
        }
    };
    if t.accent || t.slide || !t.marks.is_plain() {
        for e in &mut evs {
            e.accent |= t.accent;
            e.slide |= t.slide;
            e.marks = t.marks.or(e.marks);
        }
    }
    // the term these events came out of is the next place they are in the text
    for e in &mut evs {
        e.sid = ctx.spans.extend(e.sid, t.span);
    }
    evs
}

fn eval_atom(t: &Term, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    match &t.atom {
        Atom::Rest => Vec::new(),
        Atom::Hit => vec![Ev { at: 0.0, sound: Sound::Hit, vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }],
        Atom::Degree(d) => {
            if *d > 28 {
                ctx.err(Diagnostic::error(t.span, format!("scale degree {d} is out of reach")).help("degrees go 1..28 (four octaves of a seven-note scale)"));
                return Vec::new();
            }
            vec![Ev { at: 0.0, sound: Sound::Note(ctx.key.degree(*d, ctx.octave)), vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }]
        }
        Atom::Note(w) => match music::parse_note(&w.text) {
            Some((pc, oct)) => vec![Ev { at: 0.0, sound: Sound::Note(music::midi(pc, oct.unwrap_or(ctx.octave))), vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }],
            None => {
                // a Roman numeral (F5): a chord of the key of the section, played by the part's role
                if let Some(evs) = crate::scale::roman_events(w, ctx) {
                    return evs;
                }
                let mut d = Diagnostic::error(w.span, format!("`{}` is not a step, a note or a scale degree", w.text))
                    .expected(&["`x`", "`~`", "a note like c3, f#2 or bb4", "a degree like 3", "`[`", "`<`", "a generator like e(5,8)"])
                    .found(format!("`{}`", w.text));
                if crate::parse::PARAMS.contains(&w.text.as_str()) {
                    d = d.help(format!("parameters take `=`: {}=90", w.text));
                } else if let Some(n) = crate::parse::nearest(&w.text, ["x"]) {
                    d = d.help(format!("did you mean `{n}`?"));
                } else if let Some(n) = crate::parse::nearest(&w.text, ["choose", "maybe", "deg", "voice-lead", "when"]) {
                    d = d.help(format!("did you mean `{n}`? (`choose [..] weights ..`, `maybe(..)`, `deg(..)` are words of a pattern, `voice-lead` and `when` follow it)"));
                } else if let Some(n) = crate::parse::nearest(&w.text, crate::parse::PARAMS) {
                    d = d.help(format!("did you mean the parameter `{n}={}`?", if n == "inst" || n == "kit" || n == "sound" || n == "route" { "name" } else { "90" }));
                }
                if let Some(h) = crate::scale::comment_hint(ctx.src, w.span.start) {
                    d = d.help(h);
                }
                ctx.err(d);
                Vec::new()
            }
        },
        Atom::Group(seqs) | Atom::Deg(seqs) => {
            let mut out = Vec::new();
            for s in seqs {
                out.extend(eval_seq(s, cycle, ctx));
                if ctx.full(out.len(), t.span) {
                    return Vec::new();
                }
            }
            out
        }
        Atom::ScaleDeg(d) => crate::scale::eval_degree(d, t.span, ctx),
        Atom::Alt(terms) => {
            let n = terms.len() as i64;
            let pick = cycle.rem_euclid(n) as usize;
            eval_term(&terms[pick], cycle.div_euclid(n), ctx)
        }
        Atom::Call(c) => eval_call(c, cycle, ctx),
        Atom::Chance(c) => crate::chance::eval_chance(c, cycle, ctx),
        Atom::Choose(c) => crate::chance::eval_choose(c, cycle, ctx),
        // (sugar is written out before a part is evaluated, compile.rs: a `~n` reaching here is its step)
        Atom::Sized(s) => eval_term(&s.term, cycle, ctx),
    }
}

/// The arguments of a call, sorted into positionals and named.
struct Args<'a> {
    pos: Vec<&'a Arg>,
    named: Vec<(&'a Word, &'a Arg)>,
}

impl<'a> Args<'a> {
    fn of(c: &'a Call) -> Args<'a> {
        let mut a = Args { pos: Vec::new(), named: Vec::new() };
        for x in &c.args {
            match x {
                Arg::Named(n, v) => a.named.push((n, v)),
                other => a.pos.push(other),
            }
        }
        a
    }

    fn named_num(&self, name: &str) -> Option<f64> {
        self.named.iter().find(|(n, _)| n.text == name).and_then(|(_, v)| match v {
            Arg::Num(n) => Some(n.value),
            _ => None,
        })
    }
}

pub(crate) fn arity(c: &Call, ctx: &mut Ctx, usage: &str, lo: usize, hi: usize, n: usize) -> bool {
    if n < lo || n > hi {
        ctx.err(Diagnostic::error(c.span, format!("{}() takes {}, and this has {n}", c.name.text, if lo == hi { format!("{lo}") } else { format!("{lo} to {hi}") }))
            .help(usage.to_string()));
        return false;
    }
    true
}

pub(crate) fn num_arg(ctx: &mut Ctx, a: &Arg, what: &str) -> Option<f64> {
    match a {
        Arg::Num(n) => Some(n.value),
        other => {
            ctx.err(Diagnostic::error(other.span(), format!("expected {what}")).found(other.span().text(ctx.src).to_string()));
            None
        }
    }
}

pub(crate) fn int_arg(ctx: &mut Ctx, a: &Arg, what: &str, lo: i64, hi: i64) -> Option<i64> {
    let v = num_arg(ctx, a, what)?;
    if v.fract() != 0.0 || (v as i64) < lo || (v as i64) > hi {
        ctx.err(Diagnostic::error(a.span(), format!("{what} is a whole number from {lo} to {hi}")).found(a.span().text(ctx.src).to_string()));
        return None;
    }
    Some(v as i64)
}

fn key_arg(ctx: &mut Ctx, a: &Arg) -> Option<Key> {
    match a {
        Arg::Word(w) => match Key::parse(&w.text) {
            Some(k) => Some(k),
            None => {
                ctx.err(
                    Diagnostic::error(w.span, format!("`{}` is not a key", w.text))
                        .expected(&["a key like am, c, f#m or d:dorian"])
                        .found(format!("`{}`", w.text))
                        .help(format!("modes: {}", music::MODES.join(", "))),
                );
                None
            }
        },
        other => {
            ctx.err(Diagnostic::error(other.span(), "expected a key like am").found(other.span().text(ctx.src).to_string()));
            None
        }
    }
}

/// The chord `chord()` and `arp()` are given: a symbol (`am7`), or a Roman numeral (`V7`, `bVII`, `ii°`, `V7/7`) that is
/// read in the key in force and as a numeral is read for a step of a part (scale.rs).
enum ChordArg {
    Symbol(Chord),
    Numeral(Roman),
}

impl ChordArg {
    /// The chord in root position, root first, with the root (a symbol's) or the key's tonic (a numeral's) in `oct`.
    fn base(&self, key: Key, oct: i32) -> Vec<i32> {
        match self {
            ChordArg::Symbol(c) => c.notes(oct),
            ChordArg::Numeral(r) => r.notes(key, oct),
        }
    }

    /// The index in [`ChordArg::base`] of the tone a written inversion puts in the bass: only a numeral writes one.
    fn written_bass(&self) -> Option<usize> {
        match self {
            ChordArg::Symbol(_) => None,
            ChordArg::Numeral(r) => r.inversion.map(|_| r.bass_index()),
        }
    }

    /// The notes as written, lowest first.
    fn notes(&self, key: Key, oct: i32) -> Vec<i32> {
        crate::scale::invert(&self.base(key, oct), self.written_bass().unwrap_or(0))
    }
}

fn chord_arg(ctx: &mut Ctx, a: &Arg) -> Option<ChordArg> {
    match a {
        Arg::Word(w) => {
            // a symbol first: a word that is both stays the symbol it always was
            if let Some(c) = Chord::parse(&w.text) {
                return Some(ChordArg::Symbol(c));
            }
            match crate::scale::parse_roman(&w.text) {
                Ok(r) => Some(ChordArg::Numeral(r)),
                Err(e) => {
                    // a numeral that does not read says why, in numeral terms; any other word is not a chord
                    let d = crate::scale::roman_error(w, e).unwrap_or_else(|| {
                        Diagnostic::error(w.span, format!("`{}` is not a chord", w.text))
                            .expected(&["a chord like am7, c, cmaj7, g7 or f#m7b5", "a Roman numeral like V7 or bVII (read in the key)"])
                            .found(format!("`{}`", w.text))
                    });
                    ctx.err(d);
                    None
                }
            }
        }
        other => {
            ctx.err(Diagnostic::error(other.span(), "expected a chord like am7 or a Roman numeral like V7").found(other.span().text(ctx.src).to_string()));
            None
        }
    }
}

pub const GENERATORS: [&str; 5] = ["e", "euclid", "markov", "arp", "chord"];
/// Calls that take a pattern and give another.
pub const TRANSFORMS: [&str; 10] = ["transpose", "octave", "reverse", "rotate", "every", "degrade", "slow", "fast", "humanize", "maybe"];

fn hash32(parts: &[i64]) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    ((h >> 32) ^ h) as u32
}

/// A call's argument as a pattern: a `[...]`, a name that was a `let`, a call, a step.
pub(crate) fn pattern_arg(a: &Arg) -> Seq {
    let term = |atom: Atom, span: crate::diag::Span| Term { span, atom, fast: None, accent: false, slide: false, marks: Marks::default() };
    match a {
        Arg::Pat(s) => s.clone(),
        Arg::Call(c) => Seq { span: c.span, terms: vec![term(Atom::Call((**c).clone()), c.span)] },
        Arg::Word(w) => Seq { span: w.span, terms: vec![term(if w.text == "x" { Atom::Hit } else { Atom::Note(w.clone()) }, w.span)] },
        Arg::Num(n) => Seq { span: n.span, terms: vec![term(Atom::Degree(n.value.max(1.0) as u32), n.span)] },
        Arg::Named(n, v) => {
            let _ = n;
            pattern_arg(v)
        }
    }
}

/// How many equal steps a pattern is: a `[...]` on its own is the steps inside it.
pub(crate) fn slots(p: &Seq) -> usize {
    if let [Term { atom: Atom::Call(c), fast: None, .. }] = p.terms.as_slice() {
        if TRANSFORMS.contains(&c.name.text.as_str()) || crate::answer::ANSWERS.contains(&c.name.text.as_str()) {
            // a transform keeps the steps of the pattern it is given (`every(n, p, q)` and `slow`
            // are the exceptions that are not asked for in the same breath)
            let first = if c.name.text == "every" { c.args.get(1) } else { c.args.first() };
            if let Some(a) = first {
                return slots(&pattern_arg(a));
            }
        }
    }
    if let [Term { atom: Atom::Group(seqs) | Atom::Deg(seqs), fast: None, .. }] = p.terms.as_slice() {
        if let [inner] = seqs.as_slice() {
            return slots(inner);
        }
    }
    p.terms.len().max(1)
}

fn shift_notes(evs: Vec<Ev>, by: i32, ctx: &Ctx) -> Vec<Ev> {
    evs.into_iter()
        .map(|e| match (e.sound, ctx.hit_root) {
            (Sound::Note(n), _) => Ev { sound: Sound::Note(n + by), ..e },
            (Sound::Hit, Some(root)) => Ev { sound: Sound::Note(root + by), ..e },
            (Sound::Hit, None) => e,
        })
        .collect()
}

fn eval_transform(c: &Call, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    let args = Args::of(c);
    let n_pos = args.pos.len();
    let name = c.name.text.as_str();
    match name {
        "transpose" | "octave" => {
            if !arity(c, ctx, &format!("{name}(riff, {}): a pattern and {}", if name == "octave" { "1" } else { "5" }, if name == "octave" { "octaves" } else { "semitones" }), 2, 2, n_pos) {
                return Vec::new();
            }
            let Some(k) = int_arg(ctx, args.pos[1], if name == "octave" { "the octaves" } else { "the semitones" }, -48, 48) else { return Vec::new() };
            let evs = eval_seq(&pattern_arg(args.pos[0]), cycle, ctx);
            shift_notes(evs, (k as i32) * if name == "octave" { 12 } else { 1 }, ctx)
        }
        "reverse" => {
            if !arity(c, ctx, "reverse(riff)", 1, 1, n_pos) {
                return Vec::new();
            }
            let p = pattern_arg(args.pos[0]);
            let w = 1.0 / slots(&p) as f64;
            eval_seq(&p, cycle, ctx).into_iter().map(|e| Ev { at: (1.0 - e.at - w).rem_euclid(1.0), ..e }).collect()
        }
        "rotate" => {
            if !arity(c, ctx, "rotate(riff, 2): a pattern and how many steps to move it", 2, 2, n_pos) {
                return Vec::new();
            }
            let Some(k) = int_arg(ctx, args.pos[1], "the steps", -64, 64) else { return Vec::new() };
            let p = pattern_arg(args.pos[0]);
            let w = 1.0 / slots(&p) as f64;
            eval_seq(&p, cycle, ctx).into_iter().map(|e| Ev { at: (e.at + k as f64 * w).rem_euclid(1.0), ..e }).collect()
        }
        "every" => {
            if !arity(c, ctx, "every(4, fill) or every(4, fill, groove): plays `fill` on every 4th bar, `groove` (or a rest) on the others", 2, 3, n_pos) {
                return Vec::new();
            }
            let Some(n) = int_arg(ctx, args.pos[0], "the bar count", 1, 64) else { return Vec::new() };
            if cycle.rem_euclid(n) == n - 1 {
                eval_seq(&pattern_arg(args.pos[1]), cycle, ctx)
            } else if let Some(other) = args.pos.get(2) {
                eval_seq(&pattern_arg(other), cycle, ctx)
            } else {
                Vec::new()
            }
        }
        "degrade" => {
            if !arity(c, ctx, "degrade(riff, 0.3) or degrade(riff, 0.3, 7): a pattern, the share to drop, a seed", 2, 3, n_pos) {
                return Vec::new();
            }
            let Some(amount) = num_arg(ctx, args.pos[1], "the share to drop, 0..1") else { return Vec::new() };
            if !(0.0..=1.0).contains(&amount) {
                ctx.err(Diagnostic::error(args.pos[1].span(), "the share to drop is 0 to 1"));
                return Vec::new();
            }
            let seed = match args.pos.get(2) {
                Some(a) => int_arg(ctx, a, "the seed", 0, 4_294_967_295).unwrap_or(0),
                None => i64::from(ctx.seed),
            };
            let evs = eval_seq(&pattern_arg(args.pos[0]), cycle, ctx);
            evs.into_iter()
                .filter(|e| {
                    let note = match e.sound {
                        Sound::Note(n) => i64::from(n),
                        Sound::Hit => -1,
                    };
                    let h = hash32(&[seed, cycle, (e.at * 960.0).round() as i64, note]);
                    f64::from(h) / f64::from(u32::MAX) >= amount
                })
                .collect()
        }
        "slow" => {
            if !arity(c, ctx, "slow(riff, 2): a pattern stretched over 2 bars", 2, 2, n_pos) {
                return Vec::new();
            }
            let Some(k) = int_arg(ctx, args.pos[1], "the bars to stretch over", 1, 16) else { return Vec::new() };
            let p = pattern_arg(args.pos[0]);
            let inner = eval_seq(&p, cycle.div_euclid(k), ctx);
            let part = cycle.rem_euclid(k) as f64;
            inner
                .into_iter()
                .filter(|e| e.at * k as f64 >= part - 1e-9 && e.at * (k as f64) < part + 1.0 - 1e-9)
                .map(|e| Ev { at: e.at * k as f64 - part, ..e })
                .collect()
        }
        "fast" => {
            if !arity(c, ctx, "fast(riff, 2): a pattern played 2 times a bar", 2, 2, n_pos) {
                return Vec::new();
            }
            let Some(k) = int_arg(ctx, args.pos[1], "the repeats", 1, 16) else { return Vec::new() };
            let p = pattern_arg(args.pos[0]);
            let mut out = Vec::new();
            for i in 0..k {
                for e in eval_seq(&p, cycle * k + i, ctx) {
                    out.push(Ev { at: (i as f64 + e.at) / k as f64, ..e });
                }
                if ctx.full(out.len(), c.span) {
                    return Vec::new();
                }
            }
            out
        }
        "maybe" => crate::chance::eval_maybe(c, cycle, ctx),
        _ => {
            // humanize(riff, seed [, amount])
            if !arity(c, ctx, "humanize(riff, 7) or humanize(riff, 7, 12): a pattern, a seed, how far velocities wander", 1, 3, n_pos) {
                return Vec::new();
            }
            let seed = match args.pos.get(1) {
                Some(a) => int_arg(ctx, a, "the seed", 0, 4_294_967_295).unwrap_or(0),
                None => i64::from(ctx.seed),
            };
            let amount = match args.pos.get(2) {
                Some(a) => int_arg(ctx, a, "how far velocities wander", 0, 60).unwrap_or(10),
                None => 10,
            };
            let base = ctx.vel;
            eval_seq(&pattern_arg(args.pos[0]), cycle, ctx)
                .into_iter()
                .map(|e| {
                    let note = match e.sound {
                        Sound::Note(n) => i64::from(n),
                        Sound::Hit => -1,
                    };
                    let h = hash32(&[seed, cycle, (e.at * 960.0).round() as i64, note, 7]);
                    let jitter = (h % (2 * amount as u32 + 1)) as i32 - amount as i32;
                    Ev { vel: Some((e.vel.unwrap_or(base) + jitter).clamp(1, 127)), ..e }
                })
                .collect()
        }
    }
}

fn eval_call(c: &Call, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    if TRANSFORMS.contains(&c.name.text.as_str()) {
        return eval_transform(c, cycle, ctx);
    }
    if crate::answer::ANSWERS.contains(&c.name.text.as_str()) {
        return crate::answer::eval_rests(c, cycle, ctx);
    }
    let args = Args::of(c);
    match c.name.text.as_str() {
        "e" | "euclid" => crate::meter::eval_euclid(c, &args.pos, ctx),
        "markov" => {
            // markov(density) | markov(key, density) | markov(key, density, seed)
            if !arity(c, ctx, "markov(am, 0.7): key, density, seed", 1, 3, args.pos.len()) {
                return Vec::new();
            }
            let (key, rest) = if matches!(args.pos[0], Arg::Word(_)) { (key_arg(ctx, args.pos[0]), &args.pos[1..]) } else { (Some(ctx.key), &args.pos[..]) };
            let Some(key) = key else { return Vec::new() };
            let Some(first) = rest.first() else {
                ctx.err(Diagnostic::error(c.span, "markov() needs a density 0..1").help("markov(am, 0.7)"));
                return Vec::new();
            };
            let Some(density) = num_arg(ctx, first, "a density 0..1") else { return Vec::new() };
            if !(0.0..=1.0).contains(&density) {
                ctx.err(Diagnostic::error(first.span(), "a density is 0 to 1").found(first.span().text(ctx.src).to_string()));
                return Vec::new();
            }
            let seed = match rest.get(1) {
                Some(a) => int_arg(ctx, a, "the seed", 0, 4_294_967_295).unwrap_or(0) as f64,
                None => args.named_num("seed").unwrap_or(f64::from(ctx.seed)),
            };
            let root = music::midi(key.tonic, ctx.octave) as i64;
            let p = GenParams {
                scale: scale_notes(root, key.intervals(), 2),
                root_note: root,
                velocity: i64::from(ctx.vel),
                density,
                steps: Some(16),
                seed,
                ..GenParams::default()
            };
            let nb = markov_melody(&p);
            let mut out: Vec<Ev> = nb
                .bindings
                .iter()
                .filter_map(|(id, b)| {
                    let i: usize = id.strip_prefix('t')?.parse().ok()?;
                    Some(Ev { at: i as f64 / 16.0, sound: Sound::Note(b.note as i32), vel: Some(b.velocity as i32), accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN })
                })
                .collect();
            out.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap());
            out
        }
        "arp" => {
            // arp(chord) | arp(chord, mode) | arp(chord, mode, n)
            if !arity(c, ctx, "arp(am7, up, 8): chord, mode, steps", 1, 3, args.pos.len()) {
                return Vec::new();
            }
            let Some(chord) = chord_arg(ctx, args.pos[0]) else { return Vec::new() };
            let mode = match args.pos.get(1) {
                Some(Arg::Word(w)) if matches!(w.text.as_str(), "up" | "down" | "updown") => w.text.clone(),
                Some(other) => {
                    ctx.err(Diagnostic::error(other.span(), "an arp mode is up, down or updown").expected(&["up", "down", "updown"]).found(other.span().text(ctx.src).to_string()));
                    return Vec::new();
                }
                None => "up".into(),
            };
            let n = match args.pos.get(2) {
                Some(a) => match int_arg(ctx, a, "the step count", 1, 64) {
                    Some(n) => n,
                    None => return Vec::new(),
                },
                None => 8,
            };
            let mut notes = chord.notes(ctx.key, ctx.octave);
            if mode == "down" {
                notes.reverse();
            } else if mode == "updown" && notes.len() > 2 {
                let back: Vec<i32> = notes[1..notes.len() - 1].iter().rev().copied().collect();
                notes.extend(back);
            }
            (0..n).map(|i| Ev { at: i as f64 / n as f64, sound: Sound::Note(notes[i as usize % notes.len()]), vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }).collect()
        }
        "chord" => {
            if !arity(c, ctx, "chord(am7) or chord(am7, 3): chord, octave", 1, 2, args.pos.len()) {
                return Vec::new();
            }
            let Some(chord) = chord_arg(ctx, args.pos[0]) else { return Vec::new() };
            let oct = match args.pos.get(1) {
                Some(a) => match int_arg(ctx, a, "the octave", -1, 8) {
                    Some(o) => o as i32,
                    None => return Vec::new(),
                },
                // no octave written, in a part that voice-leads: voiced from the chord before (F5)
                None if ctx.scale.lead => return crate::scale::lead_chord(ctx, &chord.base(ctx.key, ctx.octave), chord.written_bass()),
                None => ctx.octave,
            };
            let notes = chord.notes(ctx.key, oct);
            if ctx.scale.lead {
                ctx.scale.prev = Some(notes.clone());
            }
            notes.into_iter().map(|n| Ev { at: 0.0, sound: Sound::Note(n), vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN }).collect()
        }
        other => {
            let mut d = Diagnostic::error(c.name.span, format!("`{other}` is not a generator"))
                .expected(&["e(k, n)", "markov(key, density)", "arp(chord, mode, n)", "chord(chord)", "transpose(p, n)", "reverse(p)", "rotate(p, n)", "every(n, p)", "degrade(p, a)", "slow(p, n)", "fast(p, n)", "octave(p, n)", "humanize(p, seed)", "rests(p, q)"])
                .found(format!("`{other}`"));
            if let Some(n) = crate::parse::nearest(other, GENERATORS.iter().chain(TRANSFORMS.iter()).chain(crate::sugar::CALLS.iter()).chain(crate::answer::ANSWERS.iter()).copied()) {
                d = d.help(format!("did you mean `{n}`?"));
            }
            ctx.err(d);
            Vec::new()
        }
    }
}
