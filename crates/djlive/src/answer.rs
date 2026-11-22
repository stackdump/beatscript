//! Call and response between decks: `rests(p)` and `rests(p, q)` (docs/BTW.md "Call and response").
//!
//! One part is the call, another is the answer, and the answer is what the call leaves: `rests(p)` is a hit on every
//! step where `p` rests, and `rests(p, q)` is `q`, kept only on the steps where `p` rests. Write the call once, as a `let`,
//! and give it to both decks: editing it moves both, because a `let` is text and each deck reads the same text.
//!
//! It is a pattern call like `reverse`, read when the pattern is, with no state between decks: each part's net stays one
//! ring with its own token, and the answer is exactly the events of a hand-written complement, so its nets and CIDs are
//! the longhand's (`btwd check --same-as` proves it). A token handed from one deck to another at run time would make one
//! net depend on another, which the language refuses (a pattern is compiled once, docs/btw/ext-perform.md).
//!
//! **The steps are `p`'s own**, not the sixteenths: `x ~ x ~` is four steps, and its answer is two hits, not fourteen.
//! A pattern of several terms has that many steps; a pattern that is one term is looked through ([`steps`]). A step
//! sounds when `p` has an onset in it (a ghost, a flam, a ratchet or a chord is one onset; a note held by `len=` or a
//! slide does not hold the steps after it).
//!
//! **`p` has no dice.** Each part rolls its own (its seed is of its deck, its role and its text), so a call with chance
//! in it would be a different rhythm on each deck: an error that says so ([`dice`]), where `q` may have chance.
//!
//! `rests` is not among [`crate::pattern::TRANSFORMS`], so it stays a name a `let` may take (`parse::reserved` reads that
//! list): a word is the call only before its `(`, as the pattern sugar is (`sugar::CALLS`).

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::pattern::{self, Ctx, Ev, Sound, MAX_EVENTS, NO_SPAN};
use crate::scale::RoleKind;

/// The answer calls: not transforms (`pattern::TRANSFORMS`), so `rests` stays free as a `let`'s name.
pub const ANSWERS: [&str; 1] = ["rests"];

const USAGE: &str = "rests(call) or rests(call, reply): the call, and the pattern kept where it rests";

/// Why the steps of a pattern cannot be counted (said at the call).
struct Cannot(String);

fn positional(c: &Call) -> Vec<&Arg> {
    c.args.iter().filter(|a| !matches!(a, Arg::Named(..))).collect()
}

/// A whole number argument, when it is one the language could take (the evaluator says the range of each; a count past a
/// million is no step count, and is left for it to refuse).
fn num(a: Option<&&Arg>) -> Option<u64> {
    match a {
        Some(Arg::Num(n)) if (1.0..=1_000_000.0).contains(&n.value) && n.value.fract() == 0.0 => Some(n.value as u64),
        _ => None,
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    (a / gcd(a, b)).saturating_mul(b)
}

/// How many equal steps `p` is in the bar `cycle`. A pattern of several terms has that many; one term is looked
/// through to what it holds: `[..]` to its inside (layers of a `[a b, c d e]` to the grid they share), `<a b>` to the term
/// this bar chooses, `*k` to `k` times the steps of what it repeats, a transform to the steps of its pattern, `e(k,n)` to
/// `n`, `arp(c, m, n)` to `n` (8 when left out); anything else (a note, a rest, a chord) is one step.
fn steps(p: &Seq, cycle: i64) -> Result<u64, Cannot> {
    match p.terms.as_slice() {
        [t] => steps_term(t, cycle),
        ts => Ok(ts.len().max(1) as u64),
    }
}

fn steps_term(t: &Term, cycle: i64) -> Result<u64, Cannot> {
    match t.fast {
        // read as `eval_term` reads it: `k` copies, the first at `cycle * k`
        Some(k) => Ok(steps_atom(t, cycle * i64::from(k))?.saturating_mul(u64::from(k))),
        None => steps_atom(t, cycle),
    }
}

fn steps_atom(t: &Term, cycle: i64) -> Result<u64, Cannot> {
    Ok(match &t.atom {
        Atom::Group(seqs) | Atom::Deg(seqs) => {
            let mut n = 1;
            for s in seqs {
                n = lcm(n, steps(s, cycle)?);
            }
            n
        }
        Atom::Alt(terms) => {
            let n = terms.len() as i64;
            steps_term(&terms[cycle.rem_euclid(n) as usize], cycle.div_euclid(n))?
        }
        Atom::Chance(c) => steps_term(&c.term, cycle)?,
        Atom::Sized(s) => steps_term(&s.term, cycle)?,
        Atom::Call(c) => steps_call(c, cycle)?,
        _ => 1,
    })
}

fn steps_call(c: &Call, cycle: i64) -> Result<u64, Cannot> {
    let pos = positional(c);
    let of = |i: usize, cycle: i64| match pos.get(i) {
        Some(a) => steps(&pattern::pattern_arg(a), cycle),
        None => Ok(1),
    };
    let name = c.name.text.as_str();
    Ok(match name {
        "e" | "euclid" => num(pos.get(1)).unwrap_or(1),
        "arp" => num(pos.get(2)).unwrap_or(8),
        "every" => match num(pos.first()) {
            Some(n) if cycle.rem_euclid(n as i64) == n as i64 - 1 => of(1, cycle)?,
            Some(_) if pos.len() > 2 => of(2, cycle)?,
            _ => 1,
        },
        "fast" => match num(pos.get(1)) {
            Some(k) => of(0, cycle * k as i64)?.saturating_mul(k),
            None => 1,
        },
        "slow" => match num(pos.get(1)) {
            Some(k) => {
                let s = of(0, cycle.div_euclid(k as i64))?;
                if s % k != 0 {
                    return Err(Cannot(format!("`slow(..., {k})` stretches {s} steps over {k} bars, which is not a whole number of steps in a bar")));
                }
                s / k
            }
            None => 1,
        },
        n if pattern::TRANSFORMS.contains(&n) || ANSWERS.contains(&n) => of(0, cycle)?,
        _ => 1,
    })
}

/// The first term of `p` that rolls dice (`?`, `choose`, `maybe`, `degrade`, `markov`), as the canonical text has it.
/// (`humanize` moves velocities, not hits, so it is not dice here.)
pub fn dice(p: &Seq) -> Option<String> {
    p.terms.iter().find_map(dice_term)
}

fn dice_term(t: &Term) -> Option<String> {
    match &t.atom {
        Atom::Chance(_) | Atom::Choose(_) => Some(crate::print::term(t)),
        Atom::Group(seqs) | Atom::Deg(seqs) => seqs.iter().find_map(dice),
        Atom::Alt(terms) => terms.iter().find_map(dice_term),
        Atom::Sized(s) => dice_term(&s.term),
        Atom::Call(c) => dice_call(c),
        _ => None,
    }
}

fn dice_call(c: &Call) -> Option<String> {
    if matches!(c.name.text.as_str(), "maybe" | "degrade" | "markov") {
        return Some(crate::print::call(c));
    }
    c.args.iter().find_map(dice_arg)
}

fn dice_arg(a: &Arg) -> Option<String> {
    match a {
        Arg::Pat(s) => dice(s),
        Arg::Call(c) => dice_call(c),
        Arg::Named(_, v) => dice_arg(v),
        _ => None,
    }
}

/// Where the first argument of the call is in the text, and what was typed there. (A `let` name is put in place of its
/// word before the call is read, so the argument's own span is the `let`'s: the text of the call says where it was written.)
fn first_arg(c: &Call, src: &str) -> (Span, String) {
    let whole = c.span.text(src);
    let found = (|| {
        let inner = whole.strip_prefix(c.name.text.as_str())?.trim_start().strip_prefix('(')?;
        let mut depth = 0i32;
        let mut end = inner.len();
        for (i, ch) in inner.char_indices() {
            match ch {
                '(' | '[' => depth += 1,
                ')' | ']' if depth > 0 => depth -= 1,
                ')' | ',' if depth == 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let arg = &inner[..end];
        let typed = arg.trim();
        if typed.is_empty() {
            return None;
        }
        // (`inner` is the end of `whole`, so its offset is what is left of it)
        let start = c.span.start + (whole.len() - inner.len()) + (arg.len() - arg.trim_start().len());
        Some((Span::new(start, start + typed.len()), typed.to_string()))
    })();
    found.unwrap_or_else(|| (c.span, "its pattern".to_string()))
}

/// The steps of `p` in the bar that sound: one flag a step, true where `p` has an onset. `p` is read as this part reads
/// it (its key, octave and bar), for onsets only: a numeral in a drum part is read as a chord's, and the voice leading of
/// the part is left where it was.
fn sounding(p: &Seq, cycle: i64, n: usize, ctx: &mut Ctx) -> Vec<bool> {
    let saved = ctx.scale.clone();
    if ctx.scale.role == RoleKind::Drum {
        ctx.scale.role = RoleKind::Chord;
    }
    let evs = pattern::eval_seq(p, cycle, ctx);
    ctx.scale = saved;
    let mut heard = vec![false; n];
    for e in evs {
        heard[step_of(e.at, n)] = true;
    }
    heard
}

/// The step of `n` that an onset at `at` (0 <= at < 1) falls in, as the other transforms read a position (a slack of
/// 1e-9 for the float a step's edge is computed with).
fn step_of(at: f64, n: usize) -> usize {
    ((at * n as f64 + 1e-9).floor().max(0.0) as usize).min(n - 1)
}

/// `p` sounds on every step of every bar it takes to come round: whatever `rests` is given, it plays nothing. `cycle` was
/// read already, so the other bars are the ones looked at.
fn never_answers(p: &Seq, cycle: i64, ctx: &mut Ctx) -> bool {
    let Some(period) = pattern::period(p) else { return false };
    let reads = ctx.reads;
    let mut silent = true;
    for c in (0..period as i64).filter(|c| *c != cycle.rem_euclid(period as i64)) {
        let Ok(n) = steps(p, c) else { silent = false; break };
        if n as usize > MAX_EVENTS || !sounding(p, c, n as usize, ctx).iter().all(|h| *h) {
            silent = false;
            break;
        }
    }
    ctx.reads = reads;
    silent
}

/// `rests(p)` and `rests(p, q)`: the events of one bar (`pattern::eval_call` hands the call here).
pub(crate) fn eval_rests(c: &Call, cycle: i64, ctx: &mut Ctx) -> Vec<Ev> {
    if let Some(Arg::Named(n, _)) = c.args.iter().find(|a| matches!(a, Arg::Named(..))) {
        ctx.err(Diagnostic::error(n.span, "rests() takes patterns, not `name=value`").help(USAGE));
        return Vec::new();
    }
    let pos = positional(c);
    if !pattern::arity(c, ctx, USAGE, 1, 2, pos.len()) {
        return Vec::new();
    }
    let p = pattern::pattern_arg(pos[0]);
    let (at, typed) = first_arg(c, ctx.src);
    if let Some(term) = dice(&p) {
        ctx.err(
            Diagnostic::error(at, format!("`rests({typed})` answers a fixed rhythm, and `{typed}` has chance in it (`{term}`)"))
                .help("each part rolls its own dice (its seed is of its deck, its role and its text), so the call and its answer would disagree: take the chance out of the call and put it on the part that plays it (`snare maybe(call, 0.8)`)"),
        );
        return Vec::new();
    }
    let n = match steps(&p, cycle) {
        Ok(n) if n as usize <= MAX_EVENTS => n as usize,
        Ok(n) => {
            ctx.err(Diagnostic::error(at, format!("`rests({typed})` has {n} steps to answer, and a bar holds at most {MAX_EVENTS} events")).help("answer a call with fewer steps"));
            return Vec::new();
        }
        Err(Cannot(why)) => {
            ctx.err(Diagnostic::error(at, format!("`rests({typed})` cannot tell the steps of the call: {why}")).help("write the call out, or slow a pattern whose steps divide over the bars"));
            return Vec::new();
        }
    };
    let heard = sounding(&p, cycle, n, ctx);
    if heard.iter().all(|h| *h) && never_answers(&p, cycle, ctx) {
        ctx.err(Diagnostic::warning(at, format!("`rests({typed})` plays nothing: `{typed}` sounds on every step of its bar")).help("leave a rest in the call, or answer a different pattern"));
    }
    match pos.get(1) {
        None => heard
            .iter()
            .enumerate()
            .filter(|(_, h)| !**h)
            .map(|(i, _)| Ev { at: i as f64 / n as f64, sound: Sound::Hit, vel: None, accent: false, slide: false, marks: Marks::default(), sid: NO_SPAN })
            .collect(),
        Some(q) => pattern::eval_seq(&pattern::pattern_arg(q), cycle, ctx).into_iter().filter(|e| !heard[step_of(e.at, n)]).collect(),
    }
}
