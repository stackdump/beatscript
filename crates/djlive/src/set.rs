//! Sets: programs of programs (docs/BTW.md, "Sets"). `set { play X  mix 8 bars with crossfade  play Y ... }`
//! compiles to an ordinary arranged program: one segment list and one control clock, so the runtime, `end`, `loop`,
//! hot reload and the markers treat it as one timeline. The tracks are pinned by CID (parse.rs), so a set's CID
//! covers them.
//!
//! The engine has two decks and a track plays on both, so a mix cannot give each track a deck: it puts both tracks'
//! parts on the same two decks, one bar at a time (N one-bar segments), fades the parts only one track has by
//! velocity along the transition's curve, swaps the roles both tracks have at the middle of the mix, and ramps the
//! tempo bar by bar. Pure, like the rest of the crate.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};

use crate::assets::{self, RightsReport};
use crate::ast::*;
use crate::compile::{Compiled, Control, Effect, Hit, PartIr, Segment, Timing};
use crate::diag::{Diagnostic, Span};
use crate::dur::Dur;
use crate::env::Env;
use crate::music::Key;
use crate::parse::Parsed;
use crate::rat::Rat;
use crate::shape::Shape;
use crate::tempo::{TempoEvent, TempoMap};

/// The transitions a `mix` can name.
pub const TRANSITIONS: [&str; 4] = ["crossfade", "filter_exit", "echo_out", "cut"];

/// A tempo difference above this fraction of the slower track is a warning.
const TEMPO_GAP: f64 = 0.08;
/// A ramp that moves the tempo more than this many BPM a bar is a warning.
const BPM_PER_BAR: f64 = 3.0;

/// One track on the set's timeline, for `check`.
#[derive(Clone, Debug, PartialEq)]
pub struct SetTrack {
    pub cid: String,
    /// The first comment of the program, or the end of its CID: a handle for a person.
    pub label: String,
    /// The bar of the set it starts on, and how long it plays (its head and tail overlaps included).
    pub start: u32,
    pub bars: u32,
    /// How many of those bars are shared with a mix (head, tail).
    pub head_mix: u32,
    pub tail_mix: u32,
    pub tempo: Option<f64>,
    pub key: Option<String>,
    pub camelot: Option<String>,
    /// How it is cut: `until 20m` and the like, as written.
    pub cut: Option<String>,
    /// The track is itself a set.
    pub nested: bool,
    pub span: Span,
}

/// One mix, for `check`.
#[derive(Clone, Debug, PartialEq)]
pub struct SetMix {
    /// 1-based tracks it goes between.
    pub from: usize,
    pub to: usize,
    pub start: u32,
    pub bars: u32,
    pub transition: String,
    pub tempo_from: f64,
    pub tempo_to: f64,
    /// Compatible by the Camelot wheel (None: a key is unknown).
    pub key_ok: Option<bool>,
    /// `mix 3→4: 130→122 over 16 bars`.
    pub plan: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SetReport {
    pub tracks: Vec<SetTrack>,
    pub mixes: Vec<SetMix>,
    pub total_bars: u32,
    pub seconds: f64,
}

impl SetReport {
    pub fn to_json(&self) -> JsonValue {
        let tracks: Vec<JsonValue> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut o = JsonObject::new();
                o.set("n", JsonValue::int(i as i64 + 1));
                o.set("cid", JsonValue::str(t.cid.clone()));
                o.set("label", JsonValue::str(t.label.clone()));
                o.set("start", JsonValue::int(i64::from(t.start)));
                o.set("bars", JsonValue::int(i64::from(t.bars)));
                o.set("head_mix", JsonValue::int(i64::from(t.head_mix)));
                o.set("tail_mix", JsonValue::int(i64::from(t.tail_mix)));
                o.set("tempo", t.tempo.map_or(JsonValue::Null, JsonValue::Number));
                o.set("key", t.key.clone().map_or(JsonValue::Null, JsonValue::str));
                o.set("camelot", t.camelot.clone().map_or(JsonValue::Null, JsonValue::str));
                o.set("cut", t.cut.clone().map_or(JsonValue::Null, JsonValue::str));
                o.set("set", t.nested.into());
                JsonValue::Object(o)
            })
            .collect();
        let mixes: Vec<JsonValue> = self
            .mixes
            .iter()
            .map(|m| {
                let mut o = JsonObject::new();
                o.set("from", JsonValue::int(m.from as i64));
                o.set("to", JsonValue::int(m.to as i64));
                o.set("start", JsonValue::int(i64::from(m.start)));
                o.set("bars", JsonValue::int(i64::from(m.bars)));
                o.set("transition", JsonValue::str(m.transition.clone()));
                o.set("tempo_from", JsonValue::Number(m.tempo_from));
                o.set("tempo_to", JsonValue::Number(m.tempo_to));
                o.set("key_ok", m.key_ok.map_or(JsonValue::Null, JsonValue::from));
                o.set("plan", JsonValue::str(m.plan.clone()));
                JsonValue::Object(o)
            })
            .collect();
        let mut o = JsonObject::new();
        o.set("tracks", JsonValue::Array(tracks));
        o.set("mixes", JsonValue::Array(mixes));
        o.set("total_bars", JsonValue::int(i64::from(self.total_bars)));
        o.set("minutes", JsonValue::Number((self.seconds / 60.0 * 10.0).round() / 10.0));
        JsonValue::Object(o)
    }
}

/// The Camelot code of a key: A minor is `8A`, C major `8B`; a mode takes its relative major's number and the letter of
/// its family (dorian, phrygian, the minor scales: A; mixolydian, the pentatonic: B).
pub fn camelot(k: Key) -> String {
    // (semitones up to the relative major, letter)
    let (up, letter) = match k.mode {
        0 | 6 => (0, 'B'),
        1 | 5 | 7 | 8 => (3, 'A'),
        2 => (10, 'A'),
        3 => (5, 'B'),
        _ => (8, 'A'),
    };
    let major = (usize::from(k.tonic) + up) % 12;
    let n = (major * 7 % 12 + 7) % 12 + 1;
    format!("{n}{letter}")
}

/// Same code, one step round the wheel, or the same number's other letter.
pub fn camelot_compatible(a: &str, b: &str) -> bool {
    let split = |s: &str| (s[..s.len() - 1].parse::<i32>().unwrap_or(0), s.chars().last().unwrap_or('A'));
    let ((na, la), (nb, lb)) = (split(a), split(b));
    let d = (na - nb).rem_euclid(12);
    (la == lb && (d == 0 || d == 1 || d == 11)) || (la != lb && d == 0)
}

/// A track's own timeline, bar 0 on: its stretches, as far as it plays.
struct Tl {
    comp: Compiled,
    segs: Vec<TSeg>,
    len: u32,
}

#[derive(Clone)]
struct TSeg {
    name: String,
    start: u32,
    bars: u32,
    decks: BTreeMap<Deck, crate::compile::DeckIr>,
}

impl Tl {
    fn at(&self, bar: u32) -> Option<(&TSeg, u32)> {
        self.segs.iter().find(|s| s.start <= bar && bar < s.start + s.bars).map(|s| (s, bar - s.start))
    }

    fn tempo(&self) -> f64 {
        self.comp.tempo.unwrap_or(120.0)
    }

    /// The tempo the track has at its own bar `bar`: its curve's (F3, tempo.rs), else its one tempo.
    fn tempo_at(&self, bar: u32) -> f64 {
        let m = &self.comp.tempo_map;
        if m.is_empty() {
            self.tempo()
        } else {
            m.bpm_at(bar)
        }
    }
}

fn rotate(ir: &crate::compile::DeckIr, by: u32) -> crate::compile::DeckIr {
    fn turn(p: &mut crate::compile::PartIr, by: u32) {
        for grid in p.grids_mut() {
            let n = grid.len();
            if n > 1 {
                let by = by as usize % n;
                grid.rotate_left(by);
            }
        }
        // (the right copy of a `jux`'d part goes round with it)
        if let Some(t) = &mut p.twin {
            turn(t, by);
        }
    }
    let mut ir = ir.clone();
    for p in &mut ir.parts {
        turn(p, by);
    }
    ir
}

/// The track's timeline up to its `until`/`end`: the stretches that play, and how long.
fn timeline(comp: &Compiled, until: Option<&(UntilWhen, Span)>, span: Span, diags: &mut Vec<Diagnostic>) -> Option<(Vec<TSeg>, u32)> {
    let tempo = comp.tempo.unwrap_or(120.0);
    let natural: Option<u32> = if comp.arranged { if comp.looped { None } else { comp.end.or_else(|| comp.song_bars()) } } else { comp.end };
    let segs: Vec<TSeg> = if comp.arranged {
        comp.segments.iter().map(|s| TSeg { name: s.name.clone(), start: s.start, bars: s.bars, decks: s.decks.clone() }).collect()
    } else {
        Vec::new()
    };
    let song = comp.song_bars();
    let cut: Option<u32> = match until {
        None => None,
        // (through the curve when the track's tempo moves: its seconds are not bars at the first tempo)
        Some((UntilWhen::Seconds(s), _)) => Some(((if comp.tempo_map.drives() { comp.tempo_map.bar_at(*s) } else { s * tempo / 240.0 }).round() as u32).max(1)),
        Some((UntilWhen::Bar(n), _)) => Some(*n),
        Some((UntilWhen::Section(n), sp)) | Some((UntilWhen::After(n), sp)) => {
            let after = matches!(until, Some((UntilWhen::After(_), _)));
            let found = comp.segments.iter().filter(|s| !s.chain && s.name == *n).map(|s| if after { s.start + s.bars } else { s.start }).next();
            // an `after` runs to the end of the section's chained stretches
            let found = found.map(|f| if after { let mut e = f; for s in comp.segments.iter().filter(|s| s.start >= f && s.chain) { if s.start == e { e = s.start + s.bars } } e } else { f });
            if found.is_none() {
                let names: Vec<String> = comp.segments.iter().filter(|s| !s.chain).map(|s| s.name.clone()).collect();
                diags.push(Diagnostic::error(*sp, format!("the track has no section `{n}` to cut at")).help(if names.is_empty() { "this track has no sections: cut it with a time (20m) or `bar N`".to_string() } else { format!("its sections: {}", names.join(", ")) }));
                return None;
            }
            found
        }
    };
    let len = match (natural, cut) {
        (Some(n), Some(c)) => {
            if c > n {
                diags.push(Diagnostic::warning(span, format!("the track ends at bar {n}, before its `until` (bar {c}): the set goes on from bar {n}")));
            }
            n.min(c)
        }
        (Some(n), None) => n,
        (None, Some(c)) => c,
        (None, None) => {
            diags.push(Diagnostic::error(span, if comp.looped { "this track loops, so it never ends: cut it with `until`" } else { "this track has no end (it is not a song with sections, and it has no `end`): cut it with `until`" }).help("play NAME until 20m"));
            return None;
        }
    };
    if len == 0 {
        diags.push(Diagnostic::error(span, "the track would play for no bars"));
        return None;
    }
    let mut out: Vec<TSeg> = Vec::new();
    if comp.arranged {
        // a looping song goes round again up to the cut
        let total = song.unwrap_or(0).max(1);
        let mut at = 0u32;
        let mut round = 0u32;
        'fill: loop {
            for s in &segs {
                let start = round * total + s.start;
                if start >= len {
                    break 'fill;
                }
                out.push(TSeg { name: s.name.clone(), start, bars: s.bars.min(len - start), decks: s.decks.clone() });
                at = start + s.bars;
            }
            if !comp.looped || at >= len {
                break;
            }
            round += 1;
        }
    } else {
        out.push(TSeg { name: "track".into(), start: 0, bars: len, decks: comp.decks.clone() });
    }
    Some((out, len))
}

/// A bar of one track as a single-cycle one-bar part list, faded by `scale`.
fn one_bar(tl: &Tl, bar: u32, deck: Deck, scale: f64) -> Vec<PartIr> {
    let Some((seg, local)) = tl.at(bar) else { return Vec::new() };
    let Some(ir) = seg.decks.get(&deck) else { return Vec::new() };
    // the bar of one part, faded; its twin (the right copy of a `jux`'d part) is the same bar of its own
    fn faded(p: &PartIr, local: u32, scale: f64) -> PartIr {
        let n = p.cycles.len().max(1);
        let hits = p.cycles.get(local as usize % n).cloned().unwrap_or_default();
        let hits: Vec<Hit> = hits
            .into_iter()
            .map(|h| Hit { vel: ((f64::from(h.vel) * scale).round() as i64).clamp(1, 127) as u8, sid: u32::MAX, ..h })
            .collect();
        PartIr { cycles: vec![hits], may: Default::default(), spans: Vec::new(), via: Vec::new(), twin: p.twin.as_ref().map(|t| Box::new(faded(t, local, scale))), ..p.clone() }
    }
    ir.parts.iter().filter(|_| scale >= 0.02).map(|p| faded(p, local, scale)).collect()
}

/// The curve of a transition at `t` (0 to 1 across the mix): (outgoing level, incoming level).
fn curve(transition: &str, t: f64) -> (f64, f64) {
    match transition {
        "filter_exit" => ((1.0 - t).powi(2), (1.5 * t).min(1.0)),
        "echo_out" => ((1.0 - t).powi(3), t),
        _ => (1.0 - t, t),
    }
}

/// A track's line that would go on past the track is cut at the track's end on the set's clock (`from` to `end`, in set bars):
/// an lfo counts its stop from its own song's first bar (and has none outside a section, where it runs until the program
/// stops), and a duck with no `for` holds until the program stops. In a set the program is the whole set.
fn bound_to_track(c: &mut Control, from: u32, end: u32) {
    fn lfo(e: &mut Effect, from: u32, end: u32) {
        match e {
            Effect::Lfo(l) => l.until = Some(l.until.map_or(end, |u| (from + u).min(end))),
            Effect::Performed(p) => lfo(&mut p.base, from, end),
            _ => {}
        }
    }
    lfo(&mut c.effect, from, end);
    if c.hold.is_none() && matches!(c.effect.base(), Effect::Duck(_)) {
        if let Timing::At(t) = c.timing {
            c.hold = end.checked_sub(t).filter(|n| *n > 0).map(Dur::bars);
        }
    }
}

fn key_of(c: &Compiled) -> Option<Key> {
    c.key
}

/// A track of a set as its block says it: where it stands, where it is cut, and what makes it.
struct TrackItem<'a> {
    span: Span,
    until: &'a Option<(UntilWhen, Span)>,
    what: TrackWhat,
}

enum TrackWhat {
    /// A saved program, pinned by the reader (`parsed.plays`).
    Stored,
    /// A generator call: `play gen@2 techno seed=42`.
    Gen(String),
    /// `next seed=+1`: the call before it, its seed stepped.
    Next(SeedStep),
}

/// Compile a parsed program that holds a `set`. None when there are errors (they are in `diags`).
pub fn check(parsed: &Parsed, src: &str, env: &Env, rights: RightsReport, diags: &mut Vec<Diagnostic>) -> Option<Compiled> {
    let mut block: Option<(&SetBlock, Span)> = None;
    let (mut end, mut looped, mut declared_private) = (None::<EndWhen>, false, rights.declared_private);
    for st in &parsed.program.stmts {
        match &st.kind {
            StmtKind::Set(b) => {
                if block.is_some() {
                    diags.push(Diagnostic::error(st.span, "a program has one `set`: put the tracks of both in one").help("or `play` a set from this one"));
                }
                block = Some((b, st.span));
            }
            StmtKind::End(e) => end = Some(e.clone()),
            StmtKind::Loop => looped = true,
            StmtKind::UseRestricted => declared_private = true,
            _ => diags.push(Diagnostic::error(st.span, "a program with a `set` holds the set, `end`, `loop` and `use restricted`, nothing else").help("the tracks have their own decks and sections: `play` them")),
        }
    }
    let (block, set_span) = block?;
    // the shape: play, (mix,) play ...
    let mut plays: Vec<TrackItem> = Vec::new();
    let mut mixes: Vec<Option<&MixItem>> = Vec::new();
    let mut pending_mix: Option<&MixItem> = None;
    for it in &block.items {
        match it {
            SetItem::Play(p) => {
                if !plays.is_empty() {
                    mixes.push(pending_mix.take());
                }
                let what = if crate::gen::is_call_word(&p.what.text) { TrackWhat::Gen(p.what.text.clone()) } else { TrackWhat::Stored };
                plays.push(TrackItem { span: p.span, until: &p.until, what });
            }
            SetItem::Next(n) => {
                if plays.is_empty() {
                    diags.push(Diagnostic::error(n.span, "`next` steps the seed of the track before it: `play` a generator call first").help("set {\n  play gen@2 techno seed=42\n  next seed=+1\n}"));
                    continue;
                }
                mixes.push(pending_mix.take());
                plays.push(TrackItem { span: n.span, until: &n.until, what: TrackWhat::Next(n.step) });
            }
            SetItem::Mix(m) => {
                if plays.is_empty() {
                    diags.push(Diagnostic::error(m.span, "a `mix` goes between two `play`s, not before the first"));
                } else if pending_mix.is_some() {
                    diags.push(Diagnostic::error(m.span, "two `mix`es in a row: a `mix` goes between two `play`s"));
                }
                pending_mix = Some(m);
            }
        }
    }
    if let Some(m) = pending_mix {
        diags.push(Diagnostic::error(m.span, "a `mix` goes between two `play`s, not after the last"));
    }
    if plays.is_empty() {
        diags.push(Diagnostic::error(set_span, "an empty `set`: `play` a program").help("set { play my-track }"));
    }
    if plays.iter().filter(|p| matches!(p.what, TrackWhat::Stored)).count() != parsed.plays.len() {
        // a play that did not resolve is already an error
        return None;
    }
    // the tracks a generator call or a `next` makes (station.rs steps the seed), with the stored ones in their places
    let mut resolved: Vec<crate::parse::Played> = Vec::new();
    let mut generated: Vec<bool> = Vec::new();
    {
        let mut stored = parsed.plays.iter();
        let mut prev: Option<String> = None;
        for p in &plays {
            let call = match &p.what {
                TrackWhat::Stored => {
                    if let Some(pl) = stored.next() {
                        resolved.push(pl.clone());
                        generated.push(false);
                    }
                    prev = None;
                    continue;
                }
                TrackWhat::Gen(c) => c.clone(),
                TrackWhat::Next(step) => match &prev {
                    Some(c) => match crate::station::stepped_call(c, *step) {
                        Ok(n) => n,
                        Err(e) => {
                            diags.push(Diagnostic::error(p.span, e));
                            return None;
                        }
                    },
                    None => {
                        diags.push(Diagnostic::error(p.span, "`next` steps the seed of a generator call: the track before it is a saved program, which has none").help("play gen@2 techno seed=42   next seed=+1"));
                        return None;
                    }
                },
            };
            match crate::gen::resolve_word(&call, &crate::gen::Host::default()) {
                Ok(r) => {
                    if matches!(p.what, TrackWhat::Next(_)) {
                        diags.push(Diagnostic::info(p.span, format!("next: {call} -> {} ({})", r.cid, r.generator)));
                    }
                    resolved.push(crate::parse::Played { cid: r.cid.clone(), text: r.text.clone(), span: p.span });
                    generated.push(true);
                }
                Err(mut d) => {
                    d.span = p.span;
                    diags.push(d);
                    return None;
                }
            }
            prev = Some(call);
        }
    }
    // the tracks
    let mut tls: Vec<Tl> = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    for (i, (p, played)) in plays.iter().zip(&resolved).enumerate() {
        let c = crate::compile::check_with(&played.text, env);
        let Some(comp) = c.compiled else {
            let n = c.diagnostics.iter().filter(|d| d.is_error()).count();
            let first = c.diagnostics.iter().find(|d| d.is_error()).map_or(String::new(), |d| format!(": {}", d.message));
            diags.push(Diagnostic::error(p.span, format!("track {} does not compile ({n} error{}){first}", i + 1, if n == 1 { "" } else { "s" })));
            continue;
        };
        for (c, t) in &comp.plays {
            if !texts.iter().any(|(x, _)| x == c) {
                texts.push((c.clone(), t.clone()));
            }
        }
        // (a generated track is no program a link need carry: the call makes it again)
        if !generated[i] && !texts.iter().any(|(x, _)| *x == played.cid) {
            texts.push((played.cid.clone(), played.text.clone()));
        }
        let Some((segs, len)) = timeline(&comp, p.until.as_ref(), p.span, diags) else { continue };
        tls.push(Tl { comp, segs, len });
    }
    if diags.iter().any(Diagnostic::is_error) || tls.len() != plays.len() {
        return None;
    }
    // the mixes: lengths, and that they fit
    let mut overlaps: Vec<u32> = Vec::new();
    for (j, m) in mixes.iter().enumerate() {
        let Some(m) = m else {
            overlaps.push(0);
            continue;
        };
        if !TRANSITIONS.contains(&m.with.text.as_str()) {
            let names: Vec<String> = TRANSITIONS.iter().map(|s| s.to_string()).collect();
            let mut d = Diagnostic::error(m.with.span, format!("`{}` is not a transition", m.with.text)).expected(&TRANSITIONS).found(format!("`{}`", m.with.text));
            if let Some(n) = crate::env::suggest(&m.with.text, &names, 1).first() {
                d = d.help(format!("did you mean `{n}`?"));
            }
            diags.push(d);
            overlaps.push(0);
            continue;
        }
        let n = if m.with.text == "cut" {
            if m.bars > 0 {
                diags.push(Diagnostic::warning(m.bars_span, "a cut has no overlap: the bars are ignored (the next track starts where this one ends)"));
            }
            0
        } else {
            m.bars
        };
        if n == 0 && m.with.text != "cut" {
            diags.push(Diagnostic::error(m.bars_span, "a mix takes at least one bar").help("mix 8 bars with crossfade"));
        }
        let (a, b) = (&tls[j], &tls[j + 1]);
        if n > a.len || n > b.len {
            diags.push(Diagnostic::error(m.bars_span, format!("a mix of {n} bars does not fit: track {} plays {} bars and track {} plays {}", j + 1, a.len, j + 2, b.len)));
        }
        overlaps.push(n);
    }
    for i in 1..tls.len().saturating_sub(1) {
        let (h, t) = (overlaps[i - 1], overlaps[i]);
        if h + t > tls[i].len {
            let sp = mixes[i].map_or(plays[i].span, |m| m.span);
            diags.push(Diagnostic::error(sp, format!("the mixes at both ends of track {} ({h} and {t} bars) overlap each other: it plays {} bars", i + 1, tls[i].len)));
        }
    }
    if diags.iter().any(Diagnostic::is_error) {
        return None;
    }
    // where each track starts on the set's clock
    let mut bases: Vec<u32> = Vec::new();
    let mut at = 0u32;
    for (i, tl) in tls.iter().enumerate() {
        bases.push(at);
        at += tl.len - overlaps.get(i).copied().unwrap_or(0);
    }
    let total = at;
    let mut segments: Vec<Segment> = Vec::new();
    let mut controls: Vec<Control> = Vec::new();
    // the key in force in each of the set's stretches is its track's (F5): a stretch of the track keeps its section's key
    let mut section_keys: BTreeMap<String, Key> = BTreeMap::new();
    let mut report = SetReport { total_bars: total, ..SetReport::default() };
    for (i, tl) in tls.iter().enumerate() {
        let head = if i > 0 { overlaps[i - 1] } else { 0 };
        let tail = overlaps.get(i).copied().unwrap_or(0);
        let (keep_a, keep_b) = (head, tl.len - tail);
        let pspan = plays[i].span;
        // the stretches of the track that are the track's alone
        let mut first = true;
        for s in &tl.segs {
            let (a, b) = (s.start.max(keep_a), (s.start + s.bars).min(keep_b));
            if a >= b {
                continue;
            }
            let decks = s.decks.iter().map(|(d, ir)| (*d, if a > s.start { rotate(ir, a - s.start) } else { ir.clone() })).collect();
            let name = format!("{}. {}", i + 1, s.name);
            if let Some(k) = tl.comp.section_keys.get(&s.name).copied().or(tl.comp.key) {
                section_keys.insert(name.clone(), k);
            }
            segments.push(Segment { name, start: bases[i] + a, bars: b - a, decks, gen: None, header: Some(pspan), lines: vec![pspan], chain: !first });
            first = false;
        }
        // its control lines, moved to its place and bounded to it
        for c in &tl.comp.controls {
            let base = bases[i];
            let timing = match c.timing {
                Timing::Now | Timing::NextMult(_) => Timing::At(base),
                Timing::At(b) => Timing::At(base + b),
                Timing::Every(n) => Timing::EveryFrom(n, base + n),
                Timing::EveryFrom(n, f) => Timing::EveryFrom(n, base + f),
                // sub-bar times (exact, in bars) move by the track's base the same way
                Timing::AtPos(p) => p.add(Rat::int(i64::from(base))).map_or(Timing::At(base), Timing::AtPos),
                Timing::EveryPos { step, from } => Timing::EveryPos { step, from: from.add(Rat::int(i64::from(base))).unwrap_or(from) },
            };
            let mut c = c.clone();
            c.timing = timing;
            c.key = format!("{}. {}", i + 1, c.key);
            c.span = pspan;
            c.via = Vec::new();
            c.window = Some((base, base + tl.len));
            bound_to_track(&mut c, base, base + tl.len);
            controls.push(c);
        }
        let k = key_of(&tl.comp);
        let sp = tl.comp.source.lines().find(|l| l.trim_start().starts_with('#')).map(|l| l.trim_start_matches(['#', '!', ' ']).trim().to_string());
        report.tracks.push(SetTrack {
            cid: resolved[i].cid.clone(),
            label: env.snippets.and_then(|s| s.label(&resolved[i].cid)).or_else(|| tl.comp.notes.first().cloned()).or(sp).unwrap_or_else(|| djengine::rights::short_cid(&resolved[i].cid)),
            start: bases[i],
            bars: tl.len,
            head_mix: head,
            tail_mix: tail,
            tempo: tl.comp.tempo,
            key: k.map(|k| k.text()),
            camelot: k.map(camelot),
            cut: plays[i].until.as_ref().map(|(u, _)| crate::print::until_word(u)),
            nested: tl.comp.set.is_some(),
            span: pspan,
        });
    }
    // the mixes, bar by bar
    for (j, m) in mixes.iter().enumerate() {
        let n = overlaps[j];
        let (a, b) = (&tls[j], &tls[j + 1]);
        // the tempo each track has where the mix is: the outgoing one's at the bar the mix begins, the incoming one's where it ends
        let (ta, tb) = (a.tempo_at(a.len - n), b.tempo_at(n));
        let (from, to) = m.and_then(|m| m.tempo).unwrap_or((ta, tb));
        let name = m.map_or("cut", |m| m.with.text.as_str()).to_string();
        let mspan = m.map_or(plays[j + 1].span, |m| m.span);
        let (ka, kb) = (key_of(&a.comp), key_of(&b.comp));
        let key_ok = match (ka, kb) {
            (Some(x), Some(y)) => Some(camelot_compatible(&camelot(x), &camelot(y))),
            _ => None,
        };
        if let (Some(x), Some(y), Some(false)) = (ka, kb, key_ok) {
            diags.push(Diagnostic::warning(mspan, format!("mix {}→{}: {} → {}: the keys clash ({} → {})", j + 1, j + 2, camelot(x), camelot(y), x.text(), y.text())).help("a track in a neighbouring key (one step round the Camelot wheel, or the relative major or minor) mixes cleanly"));
        }
        let gap = (ta - tb).abs() / ta.min(tb);
        if m.is_some() && n > 0 {
            if gap > TEMPO_GAP {
                diags.push(Diagnostic::warning(mspan, format!("mix {}→{}: {ta}→{tb} BPM is a {:.0}% jump", j + 1, j + 2, gap * 100.0)).help("a gentler step, or a longer mix"));
            } else if n > 0 && (from - to).abs() / f64::from(n) > BPM_PER_BAR {
                diags.push(Diagnostic::warning(mspan, format!("mix {}→{}: the tempo moves {:.1} BPM a bar over {n} bars", j + 1, j + 2, (from - to).abs() / f64::from(n))).help("a longer mix makes the ramp gentler"));
            }
        }
        if let Some((f, t)) = m.and_then(|m| m.tempo) {
            if (f - ta).abs() > 0.5 || (t - tb).abs() > 0.5 {
                diags.push(Diagnostic::warning(mspan, format!("the mix ramps {f}→{t}, but the tracks are at {ta} and {tb}: the tempo jumps at each end")));
            }
        }
        let plan = if n == 0 { format!("mix {}→{}: a cut at bar {}", j + 1, j + 2, bases[j + 1]) } else { format!("mix {}→{}: {}→{} over {n} bars ({name})", j + 1, j + 2, num(from), num(to)) };
        report.mixes.push(SetMix { from: j + 1, to: j + 2, start: bases[j + 1], bars: n, transition: name.clone(), tempo_from: from, tempo_to: to, key_ok, plan });
        for k in 0..n {
            let t = (f64::from(k) + 0.5) / f64::from(n);
            let (lo, li) = curve(&name, t);
            let tempo = from + (to - from) * t;
            let (ob, ib) = (a.len - n + k, k);
            let mut decks: BTreeMap<Deck, crate::compile::DeckIr> = BTreeMap::new();
            for d in [Deck::A, Deck::B] {
                let (has_o, has_i) = (a.at(ob).is_some_and(|(s, _)| s.decks.contains_key(&d)), b.at(ib).is_some_and(|(s, _)| s.decks.contains_key(&d)));
                if !has_o && !has_i {
                    continue;
                }
                let outp = one_bar(a, ob, d, 1.0);
                let inp = one_bar(b, ib, d, 1.0);
                let roles_o: Vec<&str> = outp.iter().map(|p| p.role.as_str()).collect();
                let roles_i: Vec<&str> = inp.iter().map(|p| p.role.as_str()).collect();
                let mut parts: Vec<PartIr> = Vec::new();
                for p in one_bar(a, ob, d, lo) {
                    // a role both have swaps at the middle: the outgoing's until then, at full level
                    if roles_i.contains(&p.role.as_str()) {
                        if t < 0.5 {
                            if let Some(full) = outp.iter().find(|x| x.role == p.role) {
                                parts.push(full.clone());
                            }
                        }
                    } else {
                        parts.push(p);
                    }
                }
                for p in one_bar(b, ib, d, li) {
                    if roles_o.contains(&p.role.as_str()) {
                        if t >= 0.5 {
                            if let Some(full) = inp.iter().find(|x| x.role == p.role) {
                                parts.push(full.clone());
                            }
                        }
                    } else {
                        parts.push(p);
                    }
                }
                if parts.is_empty() {
                    continue;
                }
                let (oi, ii) = (a.at(ob).and_then(|(s, _)| s.decks.get(&d)), b.at(ib).and_then(|(s, _)| s.decks.get(&d)));
                decks.insert(d, crate::compile::DeckIr { deck: d, period: 1, parts, tempo: Some(tempo), swing: ii.and_then(|x| x.swing).or(oi.and_then(|x| x.swing)), key: ii.and_then(|x| x.key).or(oi.and_then(|x| x.key)) });
            }
            segments.push(Segment { name: format!("mix {}→{}", j + 1, j + 2), start: bases[j + 1] + k, bars: 1, decks, gen: None, header: Some(mspan), lines: vec![mspan], chain: k > 0 });
        }
    }
    segments.sort_by_key(|s| s.start);
    // the set's tempo through its bars, in the program's own tempo machinery (F3, tempo.rs: what `length`, the plan and the
    // runtime's curve follower read): each track's tempo, curve and all, where it plays alone, and each mix's ramp over its bars
    let mut events: Vec<TempoEvent> = Vec::new();
    for (i, tl) in tls.iter().enumerate() {
        let head = if i > 0 { overlaps[i - 1] } else { 0 };
        let tail = overlaps.get(i).copied().unwrap_or(0);
        let (alone_from, alone_to, base) = (Rat::int(i64::from(head)), Rat::int(i64::from(tl.len - tail)), Rat::int(i64::from(bases[i])));
        if let Ok(at) = base.add(alone_from) {
            events.push(TempoEvent::set(at, tl.tempo_at(head), plays[i].span));
        }
        for e in tl.comp.tempo_map.events().iter().filter(|e| e.at >= alone_from && e.at < alone_to) {
            if let Ok(at) = e.at.add(base) {
                events.push(TempoEvent { at, ..e.clone() });
            }
        }
    }
    for (j, r) in report.mixes.iter().enumerate().filter(|(j, _)| overlaps[*j] > 0) {
        let span = mixes[j].map_or(plays[j + 1].span, |m| m.span);
        events.push(TempoEvent::ramp(Rat::int(i64::from(r.start)), r.tempo_from, r.tempo_to, Rat::int(i64::from(r.bars)), Shape::Linear, span));
    }
    let tempo_map = TempoMap::new(events);
    report.seconds = tempo_map.total_seconds(total);
    // the rights of the set are those of its tracks
    let mut uses = Vec::new();
    for (i, tl) in tls.iter().enumerate() {
        for a in &tl.comp.rights.assets {
            let mut a = a.clone();
            a.via_import = Some(resolved[i].cid.clone());
            a.span = plays[i].span;
            uses.push(a);
        }
    }
    let rights = assets::report(uses, declared_private);
    if rights.private_required && !rights.declared_private {
        let a = rights.assets.iter().find(|a| a.rights == assets::AssetRights::RestrictedPrivate).expect("a restricted asset set the tier");
        diags.push(Diagnostic::error(a.span, format!("this track uses {}, which is restricted: the set must declare `use restricted` and stays private", a.label())).help("add `use restricted` after the set"));
    }
    if diags.iter().any(Diagnostic::is_error) {
        return None;
    }
    let end_bar = match &end {
        Some(EndWhen::At(n)) => Some((*n).min(total)),
        _ => (!looped).then_some(total),
    };
    if let Some(EndWhen::After(w)) = &end {
        diags.push(Diagnostic::error(w.span, "`end after` names a section, and a set has tracks: use `end` (after the last track) or `end at bar N`"));
        return None;
    }
    let text = crate::print::canonical(&parsed.program);
    let first = tls.first()?;
    let warnings = diags.clone();
    Some(Compiled {
        rights,
        cid: crate::compile::program_cid(&text),
        text,
        tempo: first.comp.tempo,
        swing: first.comp.swing,
        key: first.comp.key,
        decks: segments.first().map(|s| s.decks.clone()).unwrap_or_default(),
        controls,
        warnings,
        arranged: true,
        segments,
        notes: parsed.comments.iter().map(|sp| sp.text(src).trim_start_matches('#').trim().to_string()).filter(|n| !n.is_empty()).collect(),
        end: end_bar,
        looped,
        loop_from: 0,
        flow: None,
        flat_clock: false,
        source: src.to_string(),
        flat_lines: Vec::new(),
        lineage: Vec::new(),
        evolves: Vec::new(),
        set: Some(report),
        station: None,
        plays: texts,
        section_keys,
        // a set has no sections of its own to export and no `assert` (a set holds nothing but its tracks)
        linked: crate::linked::Structure::default(),
        tempo_map,
        assertions: Vec::new(),
        sing: Vec::new(),
        launches: Default::default(),
    })
}

fn num(x: f64) -> String {
    crate::print::num(x)
}

/// The inline tracks a link may carry: JSON `{cid: text}`, only when it adds no more than `budget` characters.
pub fn bundle(plays: &[(String, String)]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(plays.iter().map(|(c, t)| (c.as_str(), JsonValue::str(t.clone()))).collect::<Vec<_>>()))
}

/// Snippets answered from the programs a link carries first (each checked against the CID it is under), then from the
/// store: the closure a program is checked against. With no store, a program the link does not carry is not here
/// ([`crate::env::not_here`]).
pub struct Inline<'a> {
    pub tracks: BTreeMap<String, String>,
    pub store: Option<&'a dyn crate::env::Snippets>,
}

impl crate::env::Snippets for Inline<'_> {
    fn label(&self, cid: &str) -> Option<String> {
        self.store.and_then(|s| s.label(cid))
    }

    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        if let Some((cid, text)) = self.tracks.iter().find(|(c, _)| c.as_str() == query || (query.len() >= 8 && c.ends_with(query))) {
            if crate::compile::program_cid(text) != *cid {
                return Err(format!("the program {cid} in this link is not the program it names: refused"));
            }
            return Ok((cid.clone(), text.clone()));
        }
        match self.store {
            Some(s) => s.resolve(query).map_err(|e| format!("{e} (the link names {query}, and this machine does not have it)")),
            None => Err(crate::env::not_here(query)),
        }
    }
}
