//! `beatscript listen`: what a program sounds like, as a small JSON summary and no audio. The same offline
//! render as `beatscript render` ([`crate::render::render_compiled`], the engine the site streams), watched
//! from the inside by a [`Tap`]: each deck's post-fader stem, the master, and the mixer as it stands
//! one beat into every bar.
//!
//! The summary says, for the window asked (the whole program, a section, or its first `bars`):
//!
//! - **master / decks / parts**: RMS and peak (dBFS) and integrated loudness (LUFS, ITU-R BS.1770-4:
//!   djrust's K-weighting, 400 ms blocks every 100 ms, the absolute and relative gates). A part is a
//!   role of a deck's line (`drums`, `bass`, ...); a deck with several is measured part by part on
//!   renders of the program with only that part kept (budgeted: see [`ListenOptions::budget_seconds`]).
//! - **silent**: every deck or part that has notes in the window but renders under
//!   [`SILENT_LUFS`], with the reason when the mixer says one ([`inaudible`]: master at 0, mute,
//!   level 0, the crossfader, a closed filter, all three EQ bands killed), else `no_sound` (the mixer
//!   lets it through and its voice made nothing).
//! - **detected**: the tempo (djrust's beat-grid analyzer on the master) and the key (djengine's
//!   chroma key detection) next to the declared ones, with a verdict for each.
//! - **sections**: each section's loudness and a per-bar loudness curve; **clipping** on the master.
//! - **macros** (only when the program has a `macro` line): each call in the window, the bar the engine starts it on (`at_bar`,
//!   counted as the time words count, read off the target deck's own clock), `held_bars` for a held one-shot, and whether it
//!   `played`, with the engine's reason when it was `refused` (also a warning).
//!
//! What the offline render leaves out (some control lines) is in `notes`, as `render` says it. The render runs at [`SR`]
//! (22.05 kHz), not the site's 44.1 kHz.
//! The numbers are deterministic: the engine is, and nothing here reads a clock but `ms`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use djengine::json::{JsonObject, JsonValue};
use djengine::musicality::loudness::KWeighting;
use djhost::deck::offline::OfflineConductor;
use djlive::ast::Deck;
use djlive::compile::{Compiled, DeckIr};

use crate::render::{self, Events, Format, Options, Plan, RenderError, Tap};

/// A deck or part with notes that measures under this (integrated, or no block above the absolute gate) is silent.
pub const SILENT_LUFS: f64 = -60.0;
/// The summary's own version: a golden made by another version is not compared number for number.
pub const VERSION: &str = "listen/1";
/// Sub-block of the loudness analysis, seconds (BS.1770: 400 ms blocks, 75 % overlap).
const SUB_SECONDS: f64 = 0.1;
/// The analysis signal for tempo and key: the master, mono, at the render's rate over this.
const DECIMATE: usize = 4;

/// The rate listen renders at: half the site's, for speed (a listen takes a render per part). Loudness comes out within a
/// few tenths of a LU of the 44.1 kHz render, and the tempo and key the same.
pub const SR: u32 = 22_050;

/// What `listen` is asked for.
#[derive(Clone, Debug)]
pub struct ListenOptions {
    /// Only this section (its first stretch on the timeline, and the stretches that continue it).
    pub section: Option<String>,
    /// At most this many bars of the window (from its start).
    pub bars: Option<u32>,
    /// The window is cut at this many bars, and the render (which always starts at bar 0) never goes past this many.
    pub max_bars: u32,
    /// Seconds of audio the part renders may take in all (the main render is not counted). 0: no part renders.
    pub budget_seconds: f64,
    pub sr: u32,
}

impl Default for ListenOptions {
    fn default() -> Self {
        ListenOptions { section: None, bars: None, max_bars: 128, budget_seconds: 900.0, sr: SR }
    }
}

// ---------------------------------------------------------------------------------------------
// the mixer and why a deck is not heard

/// One deck's mixer facts: what `inaudible` reads. The page and the phone report the same fields
/// (djrust `host::deck::audible`), so the site's `get` and `listen` say the same thing.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckMix {
    pub volume: f64,
    pub mute: bool,
    pub crossfader: f64,
    /// The deck's share of the crossfader, 0..1.
    pub crossfade_gain: f64,
    pub master: f64,
    /// EQ bands at the bottom (`low`, `mid`, `high`).
    pub kills: Vec<String>,
    /// The strip's one-knob filter, 0.5 open.
    pub filter: f64,
    /// The insert filter when it is on: (sweep, mix).
    pub insert_filter: Option<(f64, f64)>,
    /// An active vocoder takes the deck as its carrier.
    pub vocoder: bool,
}

/// Why `deck` cannot be heard with this mixer, or None: `(code, words)`. The codes are
/// `master_zero`, `mute`, `level_0`, `crossfader`, `filter_closed`, `eq_killed`.
pub fn inaudible(deck: &str, m: &DeckMix) -> Option<(&'static str, String)> {
    if m.master <= 0.001 {
        return Some(("master_zero", "the master level is at 0".into()));
    }
    if m.mute {
        return Some(("mute", format!("deck {deck} is muted")));
    }
    if m.volume <= 0.001 {
        return Some(("level_0", format!("deck {deck}'s level is at 0")));
    }
    if m.crossfade_gain <= 0.01 {
        let side = if m.crossfader <= 0.5 { "A" } else { "B" };
        return Some(("crossfader", format!("the crossfader is all the way to {side} ({:.2}), which shuts deck {deck}", m.crossfader)));
    }
    let closed = |sweep: f64| sweep <= 0.03 || sweep >= 0.97;
    if closed(m.filter) {
        return Some(("filter_closed", format!("deck {deck}'s filter is closed ({:.2}; 0.5 is open)", m.filter)));
    }
    if let Some((sweep, mix)) = m.insert_filter {
        if closed(sweep) && mix >= 0.5 {
            return Some(("filter_closed", format!("deck {deck}'s insert filter is closed (sweep {sweep:.2}; 0.5 is open)")));
        }
    }
    if m.kills.len() >= 3 {
        return Some(("eq_killed", format!("all three of deck {deck}'s EQ bands are killed")));
    }
    None
}

/// [`DeckMix`] from the djrust `audible` JSON (the page's and the phone's report), for one deck. None when the
/// deck is not in it.
#[allow(dead_code)] // beatscript-serve's get; beatscript has no use for it
pub fn deck_mix_from_json(v: &serde_like::Value, deck: &str) -> Option<DeckMix> {
    serde_like::deck_mix(v, deck)
}

/// The engine's own mixer, for `deck`.
fn deck_mix_of(engine: &djhost::deck::engine::Engine, deck: &str) -> Option<DeckMix> {
    use djengine::fx::insert::{InsertKind, InsertParam};
    use djengine::fx::vocoder::CarrierSource;
    let d = engine.decks.iter().find(|d| d.name == deck)?;
    let kills = [("low", d.strip.eq_low), ("mid", d.strip.eq_mid), ("high", d.strip.eq_high)].iter().filter(|(_, v)| *v <= 0.02).map(|(k, _)| k.to_string()).collect();
    let insert_filter = engine.insert_settings(deck).filter(|s| s.is_on(InsertKind::Filter)).map(|s| (s.get(InsertParam::FilterSweep), s.get(InsertParam::FilterMix)));
    let vocoder = engine.vocoder.instances.iter().any(|v| v.active && !v.settings.bypass && matches!(&v.carrier, CarrierSource::Deck(n) if n == deck));
    Some(DeckMix {
        volume: d.volume,
        mute: d.mute,
        crossfader: engine.crossfader,
        crossfade_gain: djhost::deck::engine::crossfade_gain(deck, engine.crossfader),
        master: engine.master,
        kills,
        filter: d.strip.filter,
        insert_filter,
        vocoder,
    })
}

/// A tiny reader over serde_json-free JSON (beatscript has no serde): `DeckMix` from djengine's JsonValue.
#[allow(dead_code)] // as deck_mix_from_json
pub mod serde_like {
    pub use djengine::json::JsonValue as Value;

    pub fn deck_mix(v: &Value, deck: &str) -> Option<super::DeckMix> {
        let d = v.get("decks")?.get(deck)?;
        let f = |o: &Value, k: &str, dflt: f64| o.get(k).and_then(|x| x.as_f64()).unwrap_or(dflt);
        let crossfader = f(v, "crossfader", 0.5);
        let kills = match d.get("kills") {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        let insert_filter = match d.get("insert_filter") {
            Some(o @ Value::Object(_)) => Some((f(o, "sweep", 0.5), f(o, "mix", 1.0))),
            _ => None,
        };
        Some(super::DeckMix {
            volume: f(d, "volume", 1.0),
            mute: matches!(d.get("mute"), Some(Value::Bool(true))),
            crossfader,
            crossfade_gain: d.get("crossfade_gain").and_then(|x| x.as_f64()).unwrap_or_else(|| djhost::deck::engine::crossfade_gain(deck, crossfader)),
            master: f(v, "master", 1.0),
            kills,
            filter: f(d, "filter", 0.5),
            insert_filter,
            vocoder: matches!(d.get("vocoder"), Some(Value::Bool(true))),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// meters

/// One stereo signal measured in 100 ms sub-blocks: K-weighted mean square (both channels summed, as BS.1770),
/// plain mean square (the channels' mean) and peak.
struct Meter {
    kl: KWeighting,
    kr: KWeighting,
    sub: usize,
    acc_k: f64,
    acc_raw: f64,
    acc_peak: f32,
    acc_n: usize,
    /// Per sub-block: (K-weighted energy, raw mean square, peak).
    blocks: Vec<(f64, f64, f32)>,
}

impl Meter {
    fn new(sr: f64) -> Meter {
        Meter { kl: KWeighting::new(sr), kr: KWeighting::new(sr), sub: ((sr * SUB_SECONDS) as usize).max(1), acc_k: 0.0, acc_raw: 0.0, acc_peak: 0.0, acc_n: 0, blocks: Vec::new() }
    }

    fn push(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            let (ka, kb) = (self.kl.process(f64::from(*a)), self.kr.process(f64::from(*b)));
            self.acc_k += ka * ka + kb * kb;
            self.acc_raw += (f64::from(*a) * f64::from(*a) + f64::from(*b) * f64::from(*b)) * 0.5;
            self.acc_peak = self.acc_peak.max(a.abs()).max(b.abs());
            self.acc_n += 1;
            if self.acc_n == self.sub {
                self.close();
            }
        }
    }

    /// Feed silence (a stem that did not sound in this block) without running the filters for nothing.
    fn push_silence(&mut self, n: usize) {
        let mut left = n;
        while left > 0 {
            let take = left.min(self.sub - self.acc_n);
            // the filters ring down: a few samples of zeros through them keep their state honest
            for _ in 0..take.min(64) {
                let (ka, kb) = (self.kl.process(0.0), self.kr.process(0.0));
                self.acc_k += ka * ka + kb * kb;
            }
            self.acc_n += take;
            left -= take;
            if self.acc_n == self.sub {
                self.close();
            }
        }
    }

    fn close(&mut self) {
        let n = self.acc_n.max(1) as f64;
        self.blocks.push((self.acc_k / n, self.acc_raw / n, self.acc_peak));
        self.acc_k = 0.0;
        self.acc_raw = 0.0;
        self.acc_peak = 0.0;
        self.acc_n = 0;
    }

    fn finish(&mut self) {
        if self.acc_n > 0 {
            self.close();
        }
    }

    /// The measures over sub-blocks `range` (clamped).
    fn stats(&self, range: std::ops::Range<usize>) -> Stats {
        let end = range.end.min(self.blocks.len());
        let start = range.start.min(end);
        let b = &self.blocks[start..end];
        if b.is_empty() {
            return Stats::default();
        }
        let raw = b.iter().map(|x| x.1).sum::<f64>() / b.len() as f64;
        let peak = b.iter().map(|x| x.2).fold(0.0f32, f32::max);
        Stats { lufs: gated(b), rms_db: db_pow(raw), peak_db: db_amp(f64::from(peak)), loudness: b.iter().map(|x| x.0).sum::<f64>() / b.len() as f64 }
    }
}

/// BS.1770-4 integrated loudness over sub-blocks: 400 ms blocks (4 sub-blocks) every 100 ms, the absolute gate at −70 LUFS
/// and the relative gate 10 LU under the mean of what passed it. None: nothing passes (silence).
fn gated(b: &[(f64, f64, f32)]) -> Option<f64> {
    let lk = |e: f64| -0.691 + 10.0 * e.log10();
    let blocks: Vec<f64> = if b.len() < 4 { vec![b.iter().map(|x| x.0).sum::<f64>() / b.len().max(1) as f64] } else { b.windows(4).map(|w| w.iter().map(|x| x.0).sum::<f64>() / 4.0).collect() };
    let abs: Vec<f64> = blocks.into_iter().filter(|e| *e > 0.0 && lk(*e) > -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let rel = lk(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let kept: Vec<f64> = abs.into_iter().filter(|e| lk(*e) > rel).collect();
    if kept.is_empty() {
        return None;
    }
    Some(lk(kept.iter().sum::<f64>() / kept.len() as f64))
}

fn db_pow(ms: f64) -> Option<f64> {
    (ms > 1e-12).then(|| 10.0 * ms.log10())
}

fn db_amp(a: f64) -> Option<f64> {
    (a > 1e-6).then(|| 20.0 * a.log10())
}

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    lufs: Option<f64>,
    rms_db: Option<f64>,
    peak_db: Option<f64>,
    /// Mean K-weighted energy (ungated): a bar's curve point.
    loudness: f64,
}

impl Stats {
    fn silent(&self) -> bool {
        self.lufs.map_or(true, |l| l < SILENT_LUFS)
    }
    fn json(&self, o: &mut JsonObject) {
        o.set("lufs", r1(self.lufs));
        o.set("rms_db", r1(self.rms_db));
        o.set("peak_db", r1(self.peak_db));
    }
}

fn r1(x: Option<f64>) -> JsonValue {
    match x {
        Some(v) if v.is_finite() => JsonValue::Number((v * 10.0).round() / 10.0),
        _ => JsonValue::Null,
    }
}

fn r3(x: f64) -> JsonValue {
    JsonValue::Number((x * 1000.0).round() / 1000.0)
}

// ---------------------------------------------------------------------------------------------
// the tap

/// The window in engine frames and its bars.
#[derive(Clone, Copy, Debug)]
struct Window {
    from_bar: u32,
    bars: u32,
    /// Engine frames of a bar (the render's constant tempo).
    bar_frames: f64,
}

impl Window {
    fn start(&self) -> i64 {
        (f64::from(self.from_bar) * self.bar_frames).round() as i64
    }
    fn end(&self) -> i64 {
        (f64::from(self.from_bar + self.bars) * self.bar_frames).round() as i64
    }
}

struct ListenTap {
    win: Window,
    sr: f64,
    master: Meter,
    stems: BTreeMap<String, Meter>,
    /// Master samples over full scale (the s16 conversion clamps them).
    clipped: u64,
    /// The master, mono, decimated: tempo and key.
    mono: Vec<f32>,
    mono_acc: (f32, usize),
    /// The mixer one beat into each bar of the window: bar -> deck -> facts.
    mix: Vec<BTreeMap<String, DeckMix>>,
    decks: Vec<String>,
    /// Whether this is the main pass (master, clipping, tempo, key, the mixer) or a part pass (stems only).
    main: bool,
    notes: Vec<String>,
}

impl ListenTap {
    fn new(win: Window, sr: f64, main: bool) -> ListenTap {
        ListenTap { win, sr, master: Meter::new(sr), stems: BTreeMap::new(), clipped: 0, mono: Vec::new(), mono_acc: (0.0, 0), mix: Vec::new(), decks: Vec::new(), main, notes: Vec::new() }
    }
}

impl Tap for ListenTap {
    fn start(&mut self, cond: &mut OfflineConductor, _plan: &Plan, notes: &[String]) {
        cond.renderer.enable_stems();
        self.decks = cond.engine().decks.iter().map(|d| d.name.clone()).collect();
        self.notes = notes.to_vec();
    }

    fn block(&mut self, cond: &mut OfflineConductor, at: i64, l: &[f32], r: &[f32]) {
        let n = l.len();
        let stems = cond.renderer.take_stems();
        let (w0, w1) = (self.win.start(), self.win.end());
        let (a, b) = (at, at + n as i64);
        if b <= w0 || a >= w1 {
            return;
        }
        // the part of this block inside the window
        let lo = (w0 - a).max(0) as usize;
        let hi = n - (b - w1).max(0) as usize;
        if let Some(st) = stems {
            for (k, name) in st.names.iter().enumerate() {
                let (sl, sr) = (&st.l[k], &st.r[k]);
                if sl.len() < hi {
                    continue;
                }
                let m = self.stems.entry(name.clone()).or_insert_with(|| {
                    let mut m = Meter::new(self.sr);
                    // a stem that first sounds late still lines up with the window's sub-blocks
                    let behind = ((a + lo as i64 - w0).max(0)) as usize;
                    m.push_silence(behind);
                    m
                });
                let (sl, sr) = (&sl[lo..hi], &sr[lo..hi]);
                if sl.iter().chain(sr.iter()).all(|x| *x == 0.0) {
                    m.push_silence(hi - lo);
                } else {
                    m.push(sl, sr);
                }
            }
        }
        if !self.main {
            return;
        }
        let (ml, mr) = (&l[lo..hi], &r[lo..hi]);
        self.master.push(ml, mr);
        self.clipped += ml.iter().chain(mr.iter()).filter(|x| x.abs() > 1.0).count() as u64;
        for (x, y) in ml.iter().zip(mr) {
            self.mono_acc.0 += (x + y) * 0.5;
            self.mono_acc.1 += 1;
            if self.mono_acc.1 == DECIMATE {
                self.mono.push(self.mono_acc.0 / DECIMATE as f32);
                self.mono_acc = (0.0, 0);
            }
        }
        // the mixer, one beat into each bar this block reaches
        loop {
            let bar = self.mix.len() as u32;
            if bar >= self.win.bars {
                break;
            }
            let probe = w0 + ((f64::from(bar) + 0.25) * self.win.bar_frames).round() as i64;
            if probe >= b {
                break;
            }
            let e = cond.engine();
            let m: BTreeMap<String, DeckMix> = self.decks.iter().filter_map(|d| deck_mix_of(e, d).map(|m| (d.clone(), m))).collect();
            self.mix.push(m);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the program's shape

/// A stretch of the timeline in the window: name, first bar (program bars), bars, its decks.
struct Stretch<'a> {
    name: String,
    start: u32,
    bars: u32,
    decks: &'a BTreeMap<Deck, DeckIr>,
}

fn stretches(c: &Compiled, program_bars: u32) -> Vec<Stretch<'_>> {
    if c.arranged && !c.segments.is_empty() {
        let mut out: Vec<Stretch> = Vec::new();
        for s in &c.segments {
            if s.start >= program_bars {
                break;
            }
            let bars = s.bars.min(program_bars - s.start);
            out.push(Stretch { name: s.name.clone(), start: s.start, bars, decks: &s.decks });
        }
        out
    } else {
        vec![Stretch { name: String::new(), start: 0, bars: program_bars, decks: &c.decks }]
    }
}

fn has_notes(p: &djlive::compile::PartIr) -> bool {
    p.cycles.iter().any(|c| !c.is_empty())
}

/// The part plays something in bar `k` of its stretch (0 = the stretch's first bar). A part on the bar's own sixteen steps
/// plays cycle `k mod cycles`; a part on a cycle of its own (`hihat/12`) or with chance is taken to play every bar it has notes.
fn has_notes_in_bar(p: &djlive::compile::PartIr, k: u32) -> bool {
    if p.cycles.is_empty() {
        return false;
    }
    if p.cycle_steps != 16 || p.chance {
        return has_notes(p);
    }
    !p.cycles[k as usize % p.cycles.len()].is_empty()
}

/// Runs of consecutive bars (`a..b`) in `bars`, sorted, each at least `min` long.
fn runs(bars: &[u32], min: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bars.len() {
        let mut j = i;
        while j + 1 < bars.len() && bars[j + 1] == bars[j] + 1 {
            j += 1;
        }
        let (a, b) = (bars[i], bars[j] + 1);
        if b - a >= min {
            out.push((a, b));
        }
        i = j + 1;
    }
    out
}

/// The roles of each deck through the program, in order of first appearance.
fn roles(c: &Compiled) -> BTreeMap<Deck, Vec<String>> {
    let mut out: BTreeMap<Deck, Vec<String>> = BTreeMap::new();
    let mut add = |m: &BTreeMap<Deck, DeckIr>| {
        for (d, ir) in m {
            let v = out.entry(*d).or_default();
            for p in &ir.parts {
                if !v.contains(&p.role) {
                    v.push(p.role.clone());
                }
            }
        }
    };
    add(&c.decks);
    for s in &c.segments {
        add(&s.decks);
    }
    out
}

/// `c` with, in every deck that has more than one role, only role number `k` kept (a deck without it in a stretch leaves
/// that stretch), and the decks with one role left out: the part pass.
fn solo(c: &Compiled, roles: &BTreeMap<Deck, Vec<String>>, k: usize) -> Compiled {
    let mut v = c.clone();
    let keep = |m: &mut BTreeMap<Deck, DeckIr>| {
        m.retain(|d, ir| {
            let Some(r) = roles.get(d).filter(|r| r.len() > 1).and_then(|r| r.get(k)) else { return false };
            ir.parts.retain(|p| &p.role == r);
            !ir.parts.is_empty()
        });
    };
    keep(&mut v.decks);
    for s in &mut v.segments {
        keep(&mut s.decks);
    }
    v
}

// ---------------------------------------------------------------------------------------------
// tempo and key

fn tempo_json(mono: &[f32], sr: f64, declared: f64) -> JsonObject {
    let mut o = JsonObject::new();
    o.set("declared_bpm", r3(declared));
    let g = djengine::deck::beat_grid::analyze(mono, mono, sr);
    match g {
        Some(g) => {
            o.set("bpm", JsonValue::Number((g.bpm * 10.0).round() / 10.0));
            o.set("bpm_confidence", r3(g.confidence));
            let near = |x: f64| (x - declared).abs() <= declared * 0.02;
            // the beat tracker may lock to a half, a double or a triplet pulse (half-time dubstep at 140 reads as 93.3)
            let m = if near(g.bpm) {
                "same"
            } else if near(g.bpm * 2.0) {
                "half"
            } else if near(g.bpm / 2.0) {
                "double"
            } else if near(g.bpm * 1.5) {
                "two_thirds"
            } else if near(g.bpm / 1.5) {
                "three_halves"
            } else {
                "differs"
            };
            o.set("bpm_match", m.into());
        }
        None => {
            o.set("bpm", JsonValue::Null);
            o.set("bpm_match", "unknown".into());
        }
    }
    o
}

/// djlive's modes that read as minor (minor, dorian, phrygian, harmonic, minpent, blues).
fn minor_mode(mode: usize) -> bool {
    matches!(mode, 1 | 2 | 4 | 5 | 7 | 8)
}

fn key_json(mono: &[f32], sr: f64, declared: Option<djlive::music::Key>) -> JsonObject {
    use djengine::key::{detect, Key, Profile};
    let mut o = JsonObject::new();
    let dk = declared.map(|k| Key { root: k.tonic % 12, minor: minor_mode(k.mode) });
    o.set("declared_key", declared.map_or(JsonValue::Null, |k| JsonValue::str(k.text())));
    match detect(mono, sr, Profile::Ensemble) {
        Some(d) => {
            o.set("key", JsonValue::str(d.key.name()));
            o.set("key_confidence", r3(d.confidence));
            o.set("tonal", JsonValue::Bool(d.tonal));
            // the master's drums flatten the chroma (`tonal` false) even when the key comes out right: a match is a match,
            // a mismatch on a flat chroma is only `unsure`
            let m = match dk {
                None => "undeclared",
                Some(k) if k == d.key => "same",
                Some(k) if k.relative() == d.key => "relative",
                Some(_) if !d.tonal => "unsure",
                Some(_) => "differs",
            };
            o.set("key_match", m.into());
        }
        None => {
            o.set("key", JsonValue::Null);
            o.set("key_match", "unknown".into());
        }
    }
    o
}

// ---------------------------------------------------------------------------------------------
// listen

/// The summary of `text`'s render (see the module), with what its references made ([`referenced`]).
pub fn listen(text: &str, lo: &ListenOptions) -> Result<JsonValue, RenderError> {
    let t0 = Instant::now();
    let c = render::check_text(text)?;
    let mut v = listen_compiled(&c, lo, t0)?;
    referenced(text, &c.cid, &mut v);
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// the parts a reference made

/// Where a reference's notes play: a deck (None: every deck), a role (None: every role of it) and a section (None: wherever
/// the deck plays them).
#[derive(Clone, Debug, PartialEq)]
struct Target {
    deck: Option<String>,
    role: Option<String>,
    section: Option<String>,
}

/// The start of the line byte `at` of `src` is on.
fn line_start(src: &str, at: usize) -> usize {
    src[..at.min(src.len())].rfind('\n').map_or(0, |i| i + 1)
}

/// The deck a byte of `src` plays on (`deck B: ...`, a part line going on over `|` lines) and the role of the part it is in
/// (the word after the line's `deck X:` or the `|` before it).
fn deck_and_role_at(src: &str, at: usize) -> (Option<String>, Option<String>) {
    let ls = line_start(src, at);
    let before = &src[ls..at.min(src.len()).max(ls)];
    // the part: what follows the last `|`, else the line's `deck X:`
    let part = match before.rfind('|') {
        Some(i) => &before[i + 1..],
        None => {
            let t = before.trim_start();
            t.strip_prefix("deck ").and_then(|r| r.find(':').map(|i| &r[i + 1..])).unwrap_or(t)
        }
    };
    let role = part.split_whitespace().next().filter(|w| djlive::parse::ROLES.contains(w)).map(str::to_string);
    let mut l = ls;
    loop {
        let end = src[l..].find('\n').map_or(src.len(), |i| l + i);
        let t = src[l..end].trim_start();
        if let Some(rest) = t.strip_prefix("deck ") {
            return (rest.split(':').next().map(|d| d.trim().to_string()).filter(|d| !d.is_empty() && !d.contains(' ')), role);
        }
        if t.starts_with('|') && l > 0 {
            l = line_start(src, l - 1);
            continue;
        }
        return (None, role);
    }
}

/// The `section NAME ... { }` block byte `at` of `src` is in, if any.
fn section_at(toks: &[djlive::lex::Token], at: usize) -> Option<String> {
    use djlive::lex::Tok;
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut first = 0;
    for (i, t) in toks.iter().enumerate() {
        if t.span.start >= at {
            break;
        }
        if i == 0 || toks[i - 1].tok == Tok::Newline {
            first = i;
        }
        match &t.tok {
            Tok::LBrace => {
                let name = match (&toks[first].tok, toks.get(first + 1).map(|t| &t.tok)) {
                    (Tok::Word(s), Some(Tok::Word(n))) if s == "section" => Some(n.clone()),
                    _ => None,
                };
                stack.push(name);
            }
            Tok::RBrace => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.into_iter().rev().flatten().next()
}

/// Where the parts that play the `let` named `name` stand: every word of a part line that is the let (or a let made of it, a
/// few levels deep).
fn let_targets(src: &str, toks: &[djlive::lex::Token], name: &str, depth: u32, out: &mut Vec<Target>) {
    use djlive::lex::Tok;
    if depth > 4 {
        return;
    }
    for (i, t) in toks.iter().enumerate() {
        let Tok::Word(w) = &t.tok else { continue };
        if w != name {
            continue;
        }
        // the let's own line (`let name = ...`) is no use of it
        let ls = line_start(src, t.span.start);
        let head = src[ls..t.span.start].trim_start();
        if head == "let" || (head.is_empty() && i > 0 && matches!(&toks[i - 1].tok, Tok::Word(l) if l == "let")) {
            continue;
        }
        // a let of lets: where that one plays
        let mut words = head.split_whitespace();
        if let (Some("let"), Some(other), Some("=")) = (words.next(), words.next(), words.next()) {
            let other = other.to_string();
            let_targets(src, toks, &other, depth + 1, out);
            continue;
        }
        let (deck, role) = deck_and_role_at(src, t.span.start);
        if deck.is_some() {
            let tg = Target { deck, role, section: section_at(toks, t.span.start) };
            if !out.contains(&tg) {
                out.push(tg);
            }
        }
    }
}

/// The parts a reference made, as targets.
fn targets(src: &str, toks: &[djlive::lex::Token], s: &djlive::refs::Site) -> Vec<Target> {
    match s.place {
        // the whole program: every part
        "arrange" => vec![Target { deck: None, role: None, section: None }],
        // a section borrowed whole: every part of it
        "section" => vec![Target { deck: None, role: None, section: s.name.clone() }],
        "part" => {
            let (deck, _) = deck_and_role_at(src, s.replaced.start);
            vec![Target { deck, role: s.name.clone(), section: section_at(toks, s.replaced.start) }]
        }
        _ => {
            let mut out = Vec::new();
            if let Some(n) = &s.name {
                let_targets(src, toks, n, 0, &mut out);
            }
            out
        }
    }
}

/// The answer's parts that came from a reference say so: `from` {call, kind, resolved, place, generator, sections} (and
/// `also_from` when more than one reference made the part), a `gen` call that was fitted to the program's key or tempo says
/// it in `notes` (in check's words), and the answer gains `expanded_cid` and `resolution` (beatscript_api::resolution).
fn referenced(text: &str, cid: &str, v: &mut JsonValue) {
    let r = beatscript_api::resolution::of(text, cid);
    let JsonValue::Object(out) = v else { return };
    out.set("expanded_cid", JsonValue::str(r.expanded_cid.clone()));
    if r.is_empty() {
        return;
    }
    let norm = djlive::lex::normalise_input(text);
    let toks = djlive::lex::lex(&norm).tokens;
    let made: Vec<(Vec<Target>, &djlive::refs::Site)> = r.sites.iter().map(|s| (targets(&norm, &toks, s), s)).collect();
    if let Some(JsonValue::Array(parts)) = out.get_mut("parts") {
        for p in parts.iter_mut() {
            let JsonValue::Object(po) = p else { continue };
            let deck = po.get("deck").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let role = po.get("part").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let mut froms: Vec<JsonValue> = Vec::new();
            for (tgs, s) in &made {
                let hits: Vec<&Target> = tgs.iter().filter(|t| t.deck.as_deref().map_or(true, |d| d == deck) && t.role.as_deref().map_or(true, |x| x == role)).collect();
                if hits.is_empty() {
                    continue;
                }
                let mut o = JsonObject::new();
                o.set("call", JsonValue::str(s.call.clone()));
                o.set("kind", JsonValue::str(s.kind));
                o.set("resolved", s.resolved.clone().map_or(JsonValue::Null, JsonValue::str));
                o.set("place", JsonValue::str(s.place));
                o.set("generator", s.generator.clone().map_or(JsonValue::Null, JsonValue::str));
                // the sections it made the part in; null: wherever the part plays
                let secs: Vec<String> = hits.iter().filter_map(|t| t.section.clone()).collect();
                o.set("sections", if hits.iter().any(|t| t.section.is_none()) { JsonValue::Null } else { JsonValue::Array(secs.into_iter().map(JsonValue::str).collect()) });
                froms.push(JsonValue::Object(o));
            }
            let mut froms = froms.into_iter();
            if let Some(f) = froms.next() {
                po.set("from", f);
                let rest: Vec<JsonValue> = froms.collect();
                if !rest.is_empty() {
                    po.set("also_from", JsonValue::Array(rest));
                }
            }
        }
    }
    // a fitted call, as check notes it: `gen@2 ... -> <cid> (seeded@2): generated in the program's key am and at its tempo 124`
    let host = djlive::gen::host_of(&toks);
    let mut fitted: Vec<String> = Vec::new();
    for s in r.sites.iter().filter(|s| s.kind == "gen" && !s.fitted.is_empty()) {
        if let Ok(res) = djlive::gen::resolve_word(&s.call, &host) {
            let n = res.note();
            if !fitted.contains(&n) {
                fitted.push(n);
            }
        }
    }
    if let Some(JsonValue::Array(notes)) = out.get_mut("notes") {
        notes.extend(fitted.into_iter().map(JsonValue::str));
    }
    out.set("resolution", beatscript_api::resolution::resolution_json(&r, text));
}

pub fn listen_compiled(c: &Compiled, lo: &ListenOptions, t0: Instant) -> Result<JsonValue, RenderError> {
    let check_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let probe = Options { sr: lo.sr, format: Format::F32le, max_bars: u32::MAX, ..Default::default() };
    let full: Plan = render::plan(c, &probe)?;
    let all = stretches(c, full.program_bars);
    // the window
    let (from_bar, mut bars, section) = match &lo.section {
        Some(name) => {
            let Some(i) = all.iter().position(|s| &s.name == name) else {
                let names: BTreeSet<&str> = all.iter().map(|s| s.name.as_str()).filter(|n| !n.is_empty()).collect();
                return Err(RenderError { code: "usage", message: format!("no section is named `{name}` on the timeline{}", if names.is_empty() { String::from(" (the program has no sections)") } else { format!("; it has {}", names.into_iter().collect::<Vec<_>>().join(", ")) }) });
            };
            // its first stretch and those that continue it (an evolve's next generations, a long net's next part)
            let mut n = all[i].bars;
            let mut j = i + 1;
            while j < all.len() && c.segments.get(j).is_some_and(|s| s.chain) {
                n += all[j].bars;
                j += 1;
            }
            (all[i].start, n, Some(name.clone()))
        }
        None => (0, full.program_bars, None),
    };
    let mut notes: Vec<String> = Vec::new();
    if let Some(b) = lo.bars {
        bars = bars.min(b.max(1));
    }
    if from_bar >= lo.max_bars {
        return Err(RenderError { code: "usage", message: format!("the window starts at bar {}: listen renders at most {} bars", from_bar + 1, lo.max_bars) });
    }
    if from_bar + bars > lo.max_bars {
        let cut = lo.max_bars - from_bar;
        notes.push(format!("cut at {cut} of the window's {bars} bars (listen renders at most {} bars from the start)", lo.max_bars));
        bars = cut;
    }
    let sr = f64::from(lo.sr);
    let bar_frames = 4.0 * 60.0 / full.bpm * sr;
    let win = Window { from_bar, bars, bar_frames };
    let opts = Options { sr: lo.sr, format: Format::F32le, max_bars: from_bar + bars, from_bar: 0, chunk_ms: 1000, file: None };

    // the main pass
    let mut tap = ListenTap::new(win, sr, true);
    let mut ev_sink = std::io::sink();
    let mut ev = Events::new(&mut ev_sink);
    let main = render::render_compiled(c, check_ms, &opts, &mut std::io::sink(), &mut ev, t0, &mut tap)?;
    tap.master.finish();
    for m in tap.stems.values_mut() {
        m.finish();
    }
    let main_ms = t0.elapsed().as_secs_f64() * 1000.0;
    notes.extend(tap.notes.iter().cloned());
    let sub_frames = (sr * SUB_SECONDS) as usize;
    let sub_of = |bar: u32| -> usize { ((f64::from(bar.saturating_sub(from_bar)) * bar_frames) / sub_frames as f64).round() as usize };
    let in_win: Vec<&Stretch> = all.iter().filter(|s| s.start < from_bar + bars && s.start + s.bars > from_bar).collect();
    let clip = |s: &Stretch| -> (u32, u32) { (s.start.max(from_bar), (s.start + s.bars).min(from_bar + bars)) };

    // the part passes (a deck the main pass finds silent all through is not taken apart: its parts are silent with it)
    let all_roles = roles(c);
    let deck_heard = |d: &Deck| tap.stems.get(d.name()).is_some_and(|m| !m.stats(0..usize::MAX).silent());
    let roles: BTreeMap<Deck, Vec<String>> = all_roles.iter().map(|(d, r)| (*d, if deck_heard(d) { r.clone() } else { r.iter().take(1).cloned().collect() })).collect();
    let max_roles = roles.values().map(Vec::len).max().unwrap_or(0);
    let mut part_meters: BTreeMap<(Deck, String), (Stats, bool)> = BTreeMap::new();
    let window_seconds = f64::from(from_bar + bars) * bar_frames / sr;
    let mut spent = 0.0;
    let mut skipped_parts = 0;
    if max_roles > 1 {
        for k in 0..max_roles {
            let multi: Vec<(Deck, String)> = roles.iter().filter(|(_, r)| r.len() > 1).filter_map(|(d, r)| r.get(k).map(|x| (*d, x.clone()))).collect();
            if multi.is_empty() {
                continue;
            }
            if spent + window_seconds > lo.budget_seconds {
                skipped_parts += multi.len();
                continue;
            }
            spent += window_seconds;
            let v = solo(c, &roles, k);
            let mut pt = ListenTap::new(win, sr, false);
            let mut ev = Events::new(&mut ev_sink);
            render::render_compiled(&v, check_ms, &opts, &mut std::io::sink(), &mut ev, t0, &mut pt)?;
            for m in pt.stems.values_mut() {
                m.finish();
            }
            for (d, role) in multi {
                // the stretches of the window where this part has notes
                let ranges: Vec<std::ops::Range<usize>> = in_win.iter().filter(|s| s.decks.get(&d).is_some_and(|ir| ir.parts.iter().any(|p| p.role == role && has_notes(p)))).map(|s| { let (a, b) = clip(s); sub_of(a)..sub_of(b) }).collect();
                let st = pt.stems.get(d.name()).map(|m| stats_over(m, &ranges)).unwrap_or_default();
                part_meters.insert((d, role), (st, true));
            }
        }
    }
    if skipped_parts > 0 {
        notes.push(format!("{skipped_parts} part{} not measured on its own: past the budget of {:.0} s of part renders", if skipped_parts == 1 { "" } else { "s" }, lo.budget_seconds));
    }

    // ---- the summary ----
    let mut out = JsonObject::new();
    out.set("version", VERSION.into());
    out.set("cid", JsonValue::str(c.cid.clone()));
    let mut w = JsonObject::new();
    w.set("section", section.map_or(JsonValue::Null, JsonValue::str));
    w.set("from_bar", JsonValue::int(i64::from(from_bar) + 1));
    w.set("bars", JsonValue::int(i64::from(bars)));
    w.set("program_bars", JsonValue::int(i64::from(full.program_bars)));
    w.set("seconds", r3(f64::from(bars) * bar_frames / sr));
    out.set("window", JsonValue::Object(w));

    // master
    let ms = tap.master.stats(0..usize::MAX);
    let mut m = JsonObject::new();
    ms.json(&mut m);
    m.set("clipped_samples", JsonValue::int(tap.clipped as i64));
    m.set("clipped_ms", r3(tap.clipped as f64 / 2.0 / sr * 1000.0));
    out.set("master", JsonValue::Object(m));

    // tempo and key
    let msr = sr / DECIMATE as f64;
    let mut det = tempo_json(&tap.mono, msr, full.bpm);
    if let JsonValue::Object(k) = JsonValue::Object(key_json(&tap.mono, msr, c.key)) {
        for (kk, vv) in k.iter() {
            det.set(kk.to_string(), vv.clone());
        }
    }
    out.set("detected", JsonValue::Object(det));

    // decks and their parts; the silent list
    let mut decks = JsonObject::new();
    let mut parts_out: Vec<JsonValue> = Vec::new();
    let mut silent: Vec<JsonValue> = Vec::new();
    // the reason in force over bars a..b (program bars) for `deck`: the one most bars have
    let reason = |deck: &str, a: u32, b: u32| -> Option<(&'static str, String)> {
        let mut count: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
        for bar in a..b {
            let Some(m) = tap.mix.get((bar - from_bar) as usize).and_then(|x| x.get(deck)) else { continue };
            if let Some((code, words)) = inaudible(deck, m) {
                let e = count.entry(code).or_insert((0, words));
                e.0 += 1;
            }
        }
        let total = b.saturating_sub(a) as usize;
        count.into_iter().max_by_key(|(_, (n, _))| *n).filter(|(_, (n, _))| *n * 2 >= total.max(1)).map(|(c, (_, w))| (c, w))
    };
    let vocoded = |deck: &str| tap.mix.iter().any(|m| m.get(deck).is_some_and(|x| x.vocoder));
    for (d, rs) in &all_roles {
        let name = d.name();
        let with_notes: Vec<&&Stretch> = in_win.iter().filter(|s| s.decks.get(d).is_some_and(|ir| ir.parts.iter().any(has_notes))).collect();
        let ranges: Vec<std::ops::Range<usize>> = with_notes.iter().map(|s| { let (a, b) = clip(s); sub_of(a)..sub_of(b) }).collect();
        let st = tap.stems.get(name).map(|m| stats_over(m, &ranges)).unwrap_or_default();
        let mut o = JsonObject::new();
        st.json(&mut o);
        let plays = !with_notes.is_empty();
        o.set("has_notes", JsonValue::Bool(plays));
        o.set("silent", JsonValue::Bool(plays && st.silent()));
        if vocoded(name) {
            o.set("vocoder", JsonValue::Bool(true));
        }
        o.set("parts", JsonValue::Array(rs.iter().cloned().map(JsonValue::str).collect()));
        decks.set(name.to_string(), JsonValue::Object(o));
        // the bars where the deck has notes and is not heard, in runs of two bars or more (the whole stretch when it is
        // shorter): a deck muted half way through a section shows, not only one that is never heard
        for s in &with_notes {
            let (a, b) = clip(s);
            let ir = s.decks.get(d).expect("with notes");
            let quiet: Vec<u32> = (a..b)
                .filter(|bar| ir.parts.iter().any(|p| has_notes_in_bar(p, bar - s.start)))
                .filter(|bar| tap.stems.get(name).map_or(true, |m| m.stats(sub_of(*bar)..sub_of(bar + 1)).silent()))
                .collect();
            for (ra, rb) in runs(&quiet, 2.min(b - a)) {
                let (code, why) = reason(name, ra, rb).unwrap_or(("no_sound", format!("deck {name} has notes and the mixer lets it through, but it made no sound")));
                silent.push(silent_entry(name, None, &s.name, ra, rb, code, &why));
            }
        }
        for role in rs {
            let has = in_win.iter().any(|s| s.decks.get(d).is_some_and(|ir| ir.parts.iter().any(|p| &p.role == role && has_notes(p))));
            let instrument = all.iter().find_map(|s| s.decks.get(d).and_then(|ir| ir.parts.iter().find(|p| &p.role == role)).and_then(|p| p.instrument.clone().or_else(|| p.kit.clone()).or_else(|| p.sound.clone())));
            let mut p = JsonObject::new();
            p.set("deck", JsonValue::str(name.to_string()));
            p.set("part", JsonValue::str(role.clone()));
            p.set("voice", instrument.clone().map_or(JsonValue::Null, JsonValue::str));
            p.set("has_notes", JsonValue::Bool(has));
            let (pst, measured) = if rs.len() == 1 { (st, true) } else { part_meters.get(&(*d, role.clone())).copied().unwrap_or((Stats::default(), false)) };
            p.set("measured", JsonValue::Bool(measured));
            if measured {
                pst.json(&mut p);
                p.set("silent", JsonValue::Bool(has && pst.silent()));
            } else if plays && st.silent() {
                // the deck is not heard: neither is any part of it (the reason is the deck's, in `silent`)
                p.set("silent", JsonValue::Bool(has));
                p.set("with_deck", JsonValue::Bool(true));
            }
            // a part of a deck that is heard, silent on its own: its voice made nothing (the deck's silence is listed already)
            if measured && has && pst.silent() && rs.len() > 1 && !st.silent() {
                for s in in_win.iter().filter(|s| s.decks.get(d).is_some_and(|ir| ir.parts.iter().any(|p| &p.role == role && has_notes(p)))) {
                    let (a, b) = clip(s);
                    let why = format!("part {role} of deck {name} has notes but its voice{} made no sound", instrument.as_deref().map(|v| format!(" ({v})")).unwrap_or_default());
                    silent.push(silent_entry(name, Some(role), &s.name, a, b, "voice_missing", &why));
                }
            }
            parts_out.push(JsonValue::Object(p));
        }
    }
    out.set("decks", JsonValue::Object(decks));
    out.set("parts", JsonValue::Array(parts_out));
    out.set("silent", JsonValue::Array(silent));

    // sections and the per-bar curve
    let mut secs = Vec::new();
    for s in &in_win {
        let (a, b) = clip(s);
        let st = tap.master.stats(sub_of(a)..sub_of(b));
        let mut o = JsonObject::new();
        o.set("name", if s.name.is_empty() { JsonValue::Null } else { JsonValue::str(s.name.clone()) });
        o.set("from_bar", JsonValue::int(i64::from(a) + 1));
        o.set("bars", JsonValue::int(i64::from(b - a)));
        st.json(&mut o);
        secs.push(JsonValue::Object(o));
    }
    out.set("sections", JsonValue::Array(secs));
    let curve: Vec<JsonValue> = (from_bar..from_bar + bars)
        .map(|bar| {
            let st = tap.master.stats(sub_of(bar)..sub_of(bar + 1));
            let l = (st.loudness > 0.0).then(|| -0.691 + 10.0 * st.loudness.log10()).filter(|l| *l > -70.0);
            r1(l)
        })
        .collect();
    out.set("bar_lufs", JsonValue::Array(curve));

    // what to say first
    let mut warnings: Vec<JsonValue> = Vec::new();
    let mut seen = BTreeSet::new();
    for s in out.get("silent").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        if let Some(w) = s.get("why").and_then(|w| w.as_str()) {
            if seen.insert(w.to_string()) {
                warnings.push(JsonValue::str(w.to_string()));
            }
        }
    }
    if tap.clipped > 0 {
        warnings.push(JsonValue::str(format!("the master clips: {} samples over full scale", tap.clipped)));
    }
    // the macro lines (docs/beatscript/ext-control.md): each call in the window, the bar the engine starts it on, and whether it played
    // (only when the program has one, so every other summary is the one it was)
    let fired: Vec<&render::MacroCall> = main.macros.iter().filter(|m| (from_bar..from_bar + bars).contains(&m.starts_bar())).collect();
    if !fired.is_empty() {
        let mut list = Vec::new();
        for m in fired {
            let mut o = JsonObject::new();
            o.set("line", JsonValue::int(m.line as i64));
            o.set("text", JsonValue::str(m.text.clone()));
            o.set("id", JsonValue::str(m.id.clone()));
            o.set("deck", JsonValue::str(m.target.clone()));
            o.set("bars", m.bars.map_or(JsonValue::Null, |n| JsonValue::int(i64::from(n))));
            if let Some(h) = m.held_bars() {
                o.set("held_bars", r3(h));
            }
            if m.latch {
                o.set("latch", JsonValue::Bool(true));
            }
            // where the engine starts it, read off the deck's own clock (whole when the deck's bars are the program's)
            o.set("at_bar", r3(m.starts_at()));
            o.set("played", JsonValue::Bool(m.refused.is_none()));
            if let Some(why) = &m.refused {
                o.set("refused", JsonValue::str(why.clone()));
                warnings.push(JsonValue::str(format!("line {} `{}`: the engine did not play it at bar {}: {why}", m.line, m.text, r3(m.starts_at()).js_stringified())));
            }
            list.push(JsonValue::Object(o));
        }
        out.set("macros", JsonValue::Array(list));
    }
    out.set("warnings", JsonValue::Array(warnings));
    out.set("notes", JsonValue::Array(notes.into_iter().map(JsonValue::str).collect()));
    let mut t = JsonObject::new();
    t.set("main", r3(main_ms));
    t.set("total", r3(t0.elapsed().as_secs_f64() * 1000.0));
    t.set("part_render_seconds", r3(spent));
    out.set("ms", JsonValue::Object(t));
    Ok(JsonValue::Object(out))
}

/// Stats over several sub-block ranges taken together (a part's stretches).
fn stats_over(m: &Meter, ranges: &[std::ops::Range<usize>]) -> Stats {
    if ranges.is_empty() {
        return Stats::default();
    }
    let blocks: Vec<(f64, f64, f32)> = ranges.iter().flat_map(|r| m.blocks[r.start.min(m.blocks.len())..r.end.min(m.blocks.len())].iter().copied()).collect();
    if blocks.is_empty() {
        return Stats::default();
    }
    let raw = blocks.iter().map(|x| x.1).sum::<f64>() / blocks.len() as f64;
    let peak = blocks.iter().map(|x| x.2).fold(0.0f32, f32::max);
    Stats { lufs: gated(&blocks), rms_db: db_pow(raw), peak_db: db_amp(f64::from(peak)), loudness: blocks.iter().map(|x| x.0).sum::<f64>() / blocks.len() as f64 }
}

fn silent_entry(deck: &str, part: Option<&str>, section: &str, a: u32, b: u32, code: &str, why: &str) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("deck", JsonValue::str(deck.to_string()));
    o.set("part", part.map_or(JsonValue::Null, |p| JsonValue::str(p.to_string())));
    o.set("section", if section.is_empty() { JsonValue::Null } else { JsonValue::str(section.to_string()) });
    o.set("from_bar", JsonValue::int(i64::from(a) + 1));
    o.set("bars", JsonValue::int(i64::from(b - a)));
    o.set("reason", JsonValue::str(code.to_string()));
    o.set("why", JsonValue::str(why.to_string()));
    JsonValue::Object(o)
}

// ---------------------------------------------------------------------------------------------
// the command

/// `beatscript listen [--file F] [--section NAME] [--bars N] [--max-bars N] [--budget S] [--sr HZ]`: the summary on stdout, one
/// line of JSON. Exit 0, or 1 with `{"error":{"code","message"}}` on stdout.
pub fn main(args: Vec<String>) -> i32 {
    let mut lo = ListenOptions::default();
    let mut file: Option<String> = None;
    let mut it = args.iter().skip_while(|a| a.as_str() == "listen").peekable();
    let fail = |code: &str, m: String| -> i32 {
        let mut e = JsonObject::new();
        e.set("code", JsonValue::str(code.to_string()));
        e.set("message", JsonValue::str(m));
        let mut o = JsonObject::new();
        o.set("error", JsonValue::Object(e));
        println!("{}", JsonValue::Object(o).js_stringified());
        1
    };
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next().cloned());
        let num = |v: Option<String>| v.and_then(|v| v.parse::<f64>().ok());
        match flag.as_str() {
            "--file" | "-f" => file = value(),
            "--section" => lo.section = value(),
            "--bars" => lo.bars = num(value()).map(|x| x as u32),
            "--max-bars" => lo.max_bars = num(value()).map_or(lo.max_bars, |x| x as u32),
            "--budget" => lo.budget_seconds = num(value()).unwrap_or(lo.budget_seconds),
            "--sr" => lo.sr = num(value()).map_or(lo.sr, |x| x as u32),
            "-h" | "--help" => {
                println!("beatscript listen [--file F | stdin] [--section NAME] [--bars N] [--max-bars N] [--budget SECONDS] [--sr HZ]");
                return 0;
            }
            f => return fail("usage", format!("unknown flag `{f}`")),
        }
    }
    let mut text = String::new();
    let read = match file.as_deref() {
        Some(p) if p != "-" => std::fs::read_to_string(p).map(|t| text = t),
        _ => std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map(|_| ()),
    };
    if let Err(e) = read {
        return fail("io", e.to_string());
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listen(&text, &lo)));
    match r {
        Ok(Ok(v)) => {
            println!("{}", v.js_stringified());
            0
        }
        Ok(Err(e)) => fail(e.code, e.message),
        Err(_) => fail("internal", "listen panicked".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix() -> DeckMix {
        DeckMix { volume: 0.8, mute: false, crossfader: 0.5, crossfade_gain: 0.707, master: 1.0, kills: vec![], filter: 0.5, insert_filter: None, vocoder: false }
    }

    /// `listen` names each macro line it heard (docs/beatscript/ext-control.md): the bar the engine starts it on, counted as the time words
    /// count (`at bar 2` starts at 2), and whether it played, with the engine's reason when it did not (a song macro on a deck that
    /// plays no net), which is also a warning.
    #[test]
    fn listen_reports_each_macro_line_and_what_the_engine_said() {
        let text = "tempo 120\ndeck A: kick x ~ x ~ | bass c2 ~ c2 ~\nat bar 2: macro filter on A over 2 bars\nat bar 4: macro breakdown on B over 2 bars\nend at bar 6\n";
        let j = listen(text, &ListenOptions { budget_seconds: 0.0, ..Default::default() }).unwrap();
        let m = j.get("macros").and_then(JsonValue::as_array).unwrap_or_else(|| panic!("no macros in {}", j.js_stringified()));
        assert_eq!(m.len(), 2, "{}", j.js_stringified());
        assert_eq!(m[0].canonical_js(), r#"{"at_bar":2,"bars":2,"deck":"A","id":"filter","line":3,"played":true,"text":"at bar 2: macro filter on A over 2 bars"}"#);
        assert_eq!(m[1].get("at_bar").and_then(JsonValue::as_f64), Some(4.0));
        assert_eq!(m[1].get("played").and_then(JsonValue::as_bool), Some(false));
        assert!(m[1].get("refused").and_then(JsonValue::as_str).is_some_and(|w| w.contains("net deck")), "{}", m[1].js_stringified());
        assert!(j.get("warnings").map(|w| w.js_stringified()).is_some_and(|w| w.contains("the engine did not play it at bar 4")), "{}", j.js_stringified());
        // two lines that make the same call are each named; a held one says how long it was held; a deck that comes in later
        // starts it on the bar its line names; `every N bars from 0` starts on 1 (no beat before bar 0), then on 4 and 8
        let text = "tempo 120\nsection intro 4 bars {\n  deck A: kick x ~ x ~\n  at bar 2: macro filter on A over 1 bar\n}\nsection drop 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass c2 ~ c2 ~\n  at bar 2: macro filter on A over 1 bar\n  macro verb_freeze on B over 2 bars\n}\narrange intro drop\n";
        let j = listen(text, &ListenOptions { budget_seconds: 0.0, ..Default::default() }).unwrap();
        let m: Vec<String> = j.get("macros").and_then(JsonValue::as_array).unwrap().iter().map(|m| format!("{} {} {} {}", m.get("line").unwrap().js_stringified(), m.get("id").and_then(JsonValue::as_str).unwrap(), m.get("at_bar").unwrap().js_stringified(), m.get("held_bars").map_or("-".into(), |h| h.js_stringified()))).collect();
        assert_eq!(m, ["4 filter 2 -", "10 verb_freeze 4 2", "9 filter 6 -"], "{}", j.js_stringified());
        let e = listen("tempo 120\ndeck A: kick x ~ x ~\nevery 4 bars from 0: macro filter on A over 1 bar\nend at bar 12\n", &ListenOptions { budget_seconds: 0.0, ..Default::default() }).unwrap();
        let at: Vec<String> = e.get("macros").and_then(JsonValue::as_array).unwrap().iter().map(|m| m.get("at_bar").unwrap().js_stringified()).collect();
        assert_eq!(at, ["1", "4", "8"], "{}", e.js_stringified());
        // a program with none has no `macros` (every other summary is the one it was)
        let plain = listen("tempo 120\ndeck A: kick x ~ x ~\nend at bar 2\n", &ListenOptions { budget_seconds: 0.0, ..Default::default() }).unwrap();
        assert!(plain.get("macros").is_none());
    }

    #[test]
    fn reasons_in_order() {
        assert_eq!(inaudible("B", &mix()), None);
        let m = DeckMix { crossfader: 0.0, crossfade_gain: 0.0, ..mix() };
        let (code, words) = inaudible("B", &m).unwrap();
        assert_eq!(code, "crossfader");
        assert!(words.contains("all the way to A"), "{words}");
        assert_eq!(inaudible("A", &DeckMix { mute: true, ..mix() }).unwrap().0, "mute");
        assert_eq!(inaudible("A", &DeckMix { volume: 0.0, ..mix() }).unwrap().0, "level_0");
        assert_eq!(inaudible("A", &DeckMix { filter: 0.0, ..mix() }).unwrap().0, "filter_closed");
        assert_eq!(inaudible("A", &DeckMix { insert_filter: Some((1.0, 1.0)), ..mix() }).unwrap().0, "filter_closed");
        assert_eq!(inaudible("A", &DeckMix { insert_filter: Some((1.0, 0.2)), ..mix() }), None);
        assert_eq!(inaudible("A", &DeckMix { kills: vec!["low".into(), "mid".into(), "high".into()], ..mix() }).unwrap().0, "eq_killed");
        assert_eq!(inaudible("A", &DeckMix { master: 0.0, mute: true, ..mix() }).unwrap().0, "master_zero");
    }

    #[test]
    fn mix_from_the_reported_json() {
        let v = djengine::json::JsonValue::parse(r#"{"crossfader":0,"master":1,"decks":{"B":{"volume":0.8,"mute":false,"crossfade_gain":0,"kills":[],"filter":0.5,"insert_filter":null,"vocoder":false}}}"#).unwrap();
        let m = deck_mix_from_json(&v, "B").unwrap();
        assert_eq!(inaudible("B", &m).unwrap().0, "crossfader");
        assert!(deck_mix_from_json(&v, "C").is_none());
    }

    #[test]
    fn runs_of_bars() {
        assert_eq!(runs(&[0, 1, 2, 5, 7, 8], 2), vec![(0, 3), (7, 9)]);
        assert_eq!(runs(&[4], 1), vec![(4, 5)]);
        assert!(runs(&[], 2).is_empty());
    }

    /// `listen` counts the master's samples over full scale. A loud mix clips on the engine's master as it is (a full-velocity kick, a
    /// clap, hats and a sub bass at full level); `drive 1` bends the peaks over (the curve never passes about 0.35 a deck), so it
    /// does not add to what `listen` warns about, it takes from it.
    #[test]
    fn drive_on_a_loud_mix_clips_less_than_the_dry_mix_not_more() {
        let song = |lines: &str| {
            format!("tempo 140\nkey am\nsection main 2 bars {{\n  deck A: kick x! ~ ~ ~ x! ~ ~ ~ x! ~ ~ ~ x! ~ ~ ~ vel=127 | clap ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ vel=127 | hihat x*16 vel=127\n  deck B: bass a1 ~ ~ ~ a1 ~ ~ ~ a1 ~ ~ ~ a1 ~ ~ ~ len=3 vel=127 inst=sub-bass\n  deck A: level 1\n  deck B: level 1\n  master: level 1\n{lines}}}\narrange main\n")
        };
        let lo = ListenOptions { bars: Some(2), max_bars: 4, budget_seconds: 0.0, ..ListenOptions::default() };
        let clipped = |text: &str| {
            let r = listen(text, &lo).unwrap_or_else(|e| panic!("{text}: {e:?}"));
            let n = r.get("master").and_then(|m| m.get("clipped_samples")).and_then(|v| v.as_f64()).unwrap_or(-1.0);
            assert!(n >= 0.0, "no master.clipped_samples in {}", r.canonical_js());
            n
        };
        let (dry, driven) = (clipped(&song("")), clipped(&song("  master: drive 1\n  deck A: drive 1\n")));
        assert!(dry > 100.0, "the loud mix is expected to clip as it is: {dry}");
        assert!(driven < dry / 4.0, "drive 1 should take the clipping away (the first hit, before its switch fades in, is clean of it): {dry} -> {driven}");
    }

    #[test]
    fn gating_and_silence() {
        assert_eq!(gated(&[(0.0, 0.0, 0.0); 10]), None);
        // a steady signal: integrated = its block loudness
        let e = 10f64.powf((-20.0 + 0.691) / 10.0);
        let l = gated(&[(e, 0.0, 0.0); 20]).unwrap();
        assert!((l + 20.0).abs() < 1e-9, "{l}");
    }
}
