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
//! - **sections**: each section's loudness and a per-bar loudness curve (and `control_off` on a section an `arrange { }` switch
//!   plays with lines of its control deck held back); **clipping** on the master.
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

use crate::mix::{self, MixStats, Spectrum, Targets, TruePeak};
use crate::render::{self, Events, Format, Options, Plan, RenderError, Tap};

/// A deck or part with notes that measures under this (integrated, or no block above the absolute gate) is silent.
pub const SILENT_LUFS: f64 = -60.0;
/// The summary's own version: a golden made by another version is not compared number for number.
pub const VERSION: &str = "listen/2";
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
    /// The delivery target the master is measured against (`targets` in the answer, and the loudness warnings).
    pub targets: Targets,
}

impl Default for ListenOptions {
    fn default() -> Self {
        ListenOptions { section: None, bars: None, max_bars: 128, budget_seconds: 900.0, sr: SR, targets: Targets::default() }
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
    acc_st: (f64, f64, f64),
    /// Per sub-block: the sums of l², r² and l·r (the stereo image).
    stereo: Vec<(f64, f64, f64)>,
    /// The bands, frame by frame (a part's and the master's: mix.rs).
    spec: Option<Spectrum>,
    /// The true peak (the master's).
    tp: Option<TruePeak>,
}

impl Meter {
    fn new(sr: f64) -> Meter {
        Meter::with(sr, false, false)
    }

    /// A meter that also takes the spectrum (`spec`) and the true peak (`tp`).
    fn with(sr: f64, spec: bool, tp: bool) -> Meter {
        let sub = ((sr * SUB_SECONDS) as usize).max(1);
        Meter {
            kl: KWeighting::new(sr),
            kr: KWeighting::new(sr),
            sub,
            acc_k: 0.0,
            acc_raw: 0.0,
            acc_peak: 0.0,
            acc_n: 0,
            blocks: Vec::new(),
            acc_st: (0.0, 0.0, 0.0),
            stereo: Vec::new(),
            spec: spec.then(|| Spectrum::new(sr, sub)),
            tp: tp.then(|| TruePeak::new(sub)),
        }
    }

    fn push(&mut self, l: &[f32], r: &[f32]) {
        if let Some(s) = &mut self.spec {
            s.push(l, r);
        }
        if let Some(t) = &mut self.tp {
            t.push(l, r);
        }
        for (a, b) in l.iter().zip(r) {
            let (ka, kb) = (self.kl.process(f64::from(*a)), self.kr.process(f64::from(*b)));
            self.acc_k += ka * ka + kb * kb;
            let (x, y) = (f64::from(*a), f64::from(*b));
            self.acc_raw += (x * x + y * y) * 0.5;
            self.acc_st.0 += x * x;
            self.acc_st.1 += y * y;
            self.acc_st.2 += x * y;
            self.acc_peak = self.acc_peak.max(a.abs()).max(b.abs());
            self.acc_n += 1;
            if self.acc_n == self.sub {
                self.close();
            }
        }
    }

    /// Feed silence (a stem that did not sound in this block) without running the filters for nothing.
    fn push_silence(&mut self, n: usize) {
        if let Some(s) = &mut self.spec {
            s.push_zeros(n);
        }
        if let Some(t) = &mut self.tp {
            t.push_zeros(n);
        }
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
        self.stereo.push(self.acc_st);
        self.acc_st = (0.0, 0.0, 0.0);
        self.acc_k = 0.0;
        self.acc_raw = 0.0;
        self.acc_peak = 0.0;
        self.acc_n = 0;
    }

    fn finish(&mut self) {
        if self.acc_n > 0 {
            self.close();
        }
        if let Some(s) = &mut self.spec {
            s.finish();
        }
        if let Some(t) = &mut self.tp {
            t.finish();
        }
    }

    /// The bands, centroid and stereo image over sub-block `ranges` taken together.
    fn mix_over(&self, ranges: &[std::ops::Range<usize>]) -> MixStats {
        let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
        for r in ranges {
            let end = r.end.min(self.stereo.len());
            for s in &self.stereo[r.start.min(end)..end] {
                ll += s.0;
                rr += s.1;
                lr += s.2;
            }
        }
        let spec = self.spec.as_ref().and_then(|sp| mix::band_means(ranges.iter().flat_map(|r| sp.frames_in(r.clone()).iter())));
        MixStats::of(spec, ll, rr, lr)
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
    /// The stems whose spectrum is taken (None: every one).
    spec_for: Option<BTreeSet<String>>,
}

impl ListenTap {
    fn new(win: Window, sr: f64, main: bool) -> ListenTap {
        let master = if main { Meter::with(sr, true, true) } else { Meter::new(sr) };
        ListenTap { win, sr, master, stems: BTreeMap::new(), clipped: 0, mono: Vec::new(), mono_acc: (0.0, 0), mix: Vec::new(), decks: Vec::new(), main, notes: Vec::new(), spec_for: None }
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
                let spec = self.spec_for.as_ref().map_or(true, |s| s.contains(name));
                let m = self.stems.entry(name.clone()).or_insert_with(|| {
                    let mut m = Meter::with(self.sr, spec, false);
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
    switched(text, &mut v);
    referenced(text, &c.cid, &mut v);
    Ok(v)
}

/// The sections an `arrange { }` switch plays with lines of their control deck held back (`~ control off`, `@ control freeze
/// off`: docs/beatscript/ext-control.md) say which, as `control_off`: the calls (`move`, `freeze` ...) whose lines do not fire there, as
/// check's `arrange.plays[].control_off` says it. A section whose control deck fires whole has none, so the answer for every
/// other program is as it was.
fn switched(text: &str, v: &mut JsonValue) {
    let env = djhost::beatscript_env::BeatscriptEnv::new(Some(&beatscript_api::known::Known));
    let Some(t) = djlive::mutate::generations(text, &env.env()) else { return };
    let off: BTreeMap<&str, &Vec<String>> = t.plays.iter().filter(|p| !p.control_off.is_empty()).map(|p| (p.name.as_str(), &p.control_off)).collect();
    if off.is_empty() {
        return;
    }
    let JsonValue::Object(out) = v else { return };
    let Some(JsonValue::Array(secs)) = out.get_mut("sections") else { return };
    for s in secs.iter_mut() {
        let JsonValue::Object(o) = s else { continue };
        let Some(calls) = o.get("name").and_then(|n| n.as_str()).and_then(|n| off.get(n)).copied() else { continue };
        o.set("control_off", JsonValue::Array(calls.iter().cloned().map(JsonValue::str).collect()));
    }
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
        // a `use` line's name for a phrase (or a role's source): the parts its `sing NAME on ROLE` lines sing on, and no other
        "use" => {
            let mut out = Vec::new();
            let Some(n) = &s.name else { return out };
            for (i, t) in toks.iter().enumerate() {
                let sung = matches!(&t.tok, djlive::lex::Tok::Word(w) if w == n) && matches!(toks.get(i + 1).map(|x| &x.tok), Some(djlive::lex::Tok::Word(o)) if o == "on");
                if let (true, Some(djlive::lex::Tok::Word(role))) = (sung, toks.get(i + 2).map(|x| &x.tok)) {
                    let (deck, _) = deck_and_role_at(src, t.span.start);
                    let tg = Target { deck, role: Some(role.clone()), section: section_at(toks, t.span.start) };
                    if !out.contains(&tg) {
                        out.push(tg);
                    }
                }
            }
            out
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
                // a `use` line's: the name it gave its source
                if s.kind == "use" {
                    o.set("name", s.name.clone().map_or(JsonValue::Null, JsonValue::str));
                }
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
    if !r.imports.is_empty() {
        out.set("imports", beatscript_api::resolution::imports_json(&r.imports, text));
    }
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

    // the main pass (a deck of one part is that part: its spectrum is taken here; a deck of several, in the part passes)
    let all_roles = roles(c);
    let mut tap = ListenTap::new(win, sr, true);
    tap.spec_for = Some(all_roles.iter().filter(|(_, r)| r.len() == 1).map(|(d, _)| d.name().to_string()).collect());
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
    let deck_heard = |d: &Deck| tap.stems.get(d.name()).is_some_and(|m| !m.stats(0..usize::MAX).silent());
    let roles: BTreeMap<Deck, Vec<String>> = all_roles.iter().map(|(d, r)| (*d, if deck_heard(d) { r.clone() } else { r.iter().take(1).cloned().collect() })).collect();
    let max_roles = roles.values().map(Vec::len).max().unwrap_or(0);
    let mut part_meters: BTreeMap<(Deck, String), (Stats, bool)> = BTreeMap::new();
    // a part of a deck of several, as its own pass heard it (its bands)
    let mut part_specs: BTreeMap<(Deck, String), Meter> = BTreeMap::new();
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
                part_meters.insert((d, role.clone()), (st, true));
                if let Some(m) = pt.stems.remove(d.name()) {
                    part_specs.insert((d, role), m);
                }
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
    // the mix: the true peak, the dynamics, the loudness over time, the bands and the stereo image
    let true_peak = tap.master.tp.as_ref().and_then(|t| t.over(0..usize::MAX));
    let master_mix = tap.master.mix_over(&[0..usize::MAX]);
    let kblocks: Vec<f64> = tap.master.blocks.iter().map(|b| b.0).collect();
    let st = mix::short_term(&kblocks, (3.0 / SUB_SECONDS).round() as usize);
    m.set("true_peak_db", r1(true_peak));
    m.set("crest_db", r1(crest(&ms)));
    m.set("plr_db", r1(true_peak.zip(ms.lufs).map(|(p, l)| p - l)));
    m.set("short_term_max_lufs", r1(st.iter().copied().reduce(f64::max)));
    m.set("lra", r1(mix::loudness_range(&st)));
    master_mix.json(&mut m);
    out.set("master", JsonValue::Object(m));
    out.set("targets", lo.targets.json(ms.lufs, true_peak));

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
    // the parts heard, for the mix's warnings and clashes
    let mut heard: Vec<Heard> = Vec::new();
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
                // the part's mix, over the bars it has notes in
                let pranges: Vec<std::ops::Range<usize>> = in_win
                    .iter()
                    .filter(|s| s.decks.get(d).is_some_and(|ir| ir.parts.iter().any(|p| &p.role == role && has_notes(p))))
                    .map(|s| {
                        let (a, b) = clip(s);
                        sub_of(a)..sub_of(b)
                    })
                    .collect();
                let meter = if rs.len() == 1 { tap.stems.get(name) } else { part_specs.get(&(*d, role.clone())) };
                if let Some(mt) = meter.filter(|m| m.spec.is_some()) {
                    let mm = mt.mix_over(&pranges);
                    p.set("crest_db", r1(crest(&pst)));
                    mm.json(&mut p);
                    if has && !pst.silent() {
                        heard.push(Heard { deck: *d, role: role.clone(), roles: rs.len(), lufs: pst.lufs, mix: mm });
                    }
                }
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
        o.set("true_peak_db", r1(tap.master.tp.as_ref().and_then(|t| t.over(sub_of(a)..sub_of(b)))));
        let mm = tap.master.mix_over(&[sub_of(a)..sub_of(b)]);
        if mm.bands.iter().any(Option::is_some) {
            o.set("bands", mix::bands_json(&mm.bands));
        }
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

    // what to say first, in the order to fix it (docs/beatscript/polish.md): what is not heard, clipping, clashes, balance,
    // loudness, width. Each warning has a stable id (`warning_ids`, side by side), so a compare can say which went.
    let mut warnings: Vec<mix::Warning> = Vec::new();
    let mut seen = BTreeSet::new();
    for s in out.get("silent").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        if let Some(w) = s.get("why").and_then(|w| w.as_str()) {
            if seen.insert(w.to_string()) {
                let part = s.get("part").and_then(|p| p.as_str()).map(|p| format!(".{p}")).unwrap_or_default();
                let id = format!("silent:{}{part}:{}", s.get("deck").and_then(|d| d.as_str()).unwrap_or(""), s.get("reason").and_then(|r| r.as_str()).unwrap_or(""));
                warnings.push((id, w.to_string()));
            }
        }
    }
    if tap.clipped > 0 {
        warnings.push(("clip".into(), format!("the master clips: {} samples over full scale", tap.clipped)));
    }
    // the mixer the window had: the master fader and each deck's (their highest)
    let master_level = tap.mix.iter().filter_map(|m| m.values().next().map(|x| x.master)).fold(f64::NAN, f64::max);
    let master_level = if master_level.is_nan() { 1.0 } else { master_level };
    let deck_level = |deck: &str| tap.mix.iter().filter_map(|m| m.get(deck).map(|x| x.volume)).fold(f64::NAN, f64::max);
    let target_w = mix::target_warnings(&lo.targets, ms.lufs, true_peak, master_level);
    // a master that clips: its true peak is fixed now; one only over the target's ceiling is delivery, with the loudness
    let clipping = tap.clipped > 0;
    if clipping {
        warnings.extend(target_w.iter().filter(|(id, _)| id == "true_peak").cloned());
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
                warnings.push((format!("macro:{}", m.line), format!("line {} `{}`: the engine did not play it at bar {}: {why}", m.line, m.text, r3(m.starts_at()).js_stringified())));
            }
            list.push(JsonValue::Object(o));
        }
        out.set("macros", JsonValue::Array(list));
    }
    // clashes: two parts heard in the same band in the same bars
    let specs: Vec<mix::PartSpectrum> = heard
        .iter()
        .filter_map(|h| {
            let m = if h.roles == 1 { tap.stems.get(h.deck.name()) } else { part_specs.get(&(h.deck, h.role.clone())) };
            m.and_then(|m| m.spec.as_ref()).map(|spec| mix::PartSpectrum { deck: h.deck.name().to_string(), role: h.role.clone(), spec })
        })
        .collect();
    let bar_subs: Vec<(u32, std::ops::Range<usize>)> = (from_bar..from_bar + bars).map(|b| (b, sub_of(b)..sub_of(b + 1))).collect();
    let section_of = |bar: u32| all.iter().find(|s| s.start <= bar && bar < s.start + s.bars).map(|s| s.name.clone()).filter(|n| !n.is_empty());
    let clashes = match tap.master.spec.as_ref() {
        Some(master) if specs.len() > 1 => mix::clashes(&specs, master, &bar_subs, &section_of, if bars < 4 { 1 } else { 2 }),
        _ => Vec::new(),
    };
    out.set("clashes", JsonValue::Array(clashes.iter().map(mix::Clash::json).collect()));
    let roles_of = |deck: &str| all_roles.iter().find(|(d, _)| d.name() == deck).map_or(1, |(_, r)| r.len());
    // one warning a pair of parts (its worst band), the four worst pairs
    let mut pairs = BTreeSet::new();
    for c in &clashes {
        if pairs.len() >= 4 {
            break;
        }
        let pair = if c.a <= c.b { (c.a.clone(), c.b.clone()) } else { (c.b.clone(), c.a.clone()) };
        if pairs.insert(pair) {
            warnings.push(mix::clash_warning(c, &roles_of));
        }
    }
    // balance: a part far under the mix
    if let Some(ml) = ms.lufs {
        for h in &heard {
            if let Some(l) = h.lufs.filter(|l| *l < ml - BURIED_LU) {
                warnings.push(mix::buried_warning(h.deck.name(), &h.role, ml - l, h.roles, deck_level(h.deck.name())));
            }
        }
    }
    // loudness and the ceiling against the target, and the dynamics
    warnings.extend(target_w.into_iter().filter(|(id, _)| !(clipping && id == "true_peak")));
    // width: low end that is wide, a master that cancels in mono
    for h in &heard {
        if let Some(w) = h.mix.width.filter(|w| *w > WIDE_LOW && h.mix.low_share() >= 0.6) {
            warnings.push(mix::wide_low_warning(h.deck.name(), &h.role, w));
        }
    }
    if let Some(corr) = master_mix.correlation.filter(|c| *c < 0.0) {
        let widest = heard.iter().filter_map(|h| h.mix.width.map(|w| (h, w))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(h, w)| (h.deck.name(), h.role.as_str(), w));
        warnings.push(mix::phase_warning(corr, widest));
    }
    out.set("warning_ids", JsonValue::Array(warnings.iter().map(|(id, _)| JsonValue::str(id.clone())).collect()));
    out.set("warnings", JsonValue::Array(warnings.into_iter().map(|(_, w)| JsonValue::str(w)).collect()));
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

/// A part far under the master's loudness: a warning (LU).
const BURIED_LU: f64 = 22.0;
/// A part whose power is mostly under 250 Hz and is wider than this: a warning.
const WIDE_LOW: f64 = 0.3;

/// A part heard, for the mix's warnings.
struct Heard {
    deck: Deck,
    role: String,
    /// The parts its deck has.
    roles: usize,
    lufs: Option<f64>,
    mix: MixStats,
}

/// Peak over RMS (dB).
fn crest(s: &Stats) -> Option<f64> {
    s.peak_db.zip(s.rms_db).map(|(p, r)| p - r)
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

/// `beatscript listen [--file F] [--section NAME] [--bars N] [--max-bars N] [--budget S] [--sr HZ] [--targets T] [--compare F]`:
/// the summary on stdout, one line of JSON (with `compare`: the summary gains `compare`, what changed from that program,
/// [`mix::compare`]). Exit 0, or 1 with `{"error":{"code","message"}}` on stdout.
pub fn main(args: Vec<String>) -> i32 {
    let mut lo = ListenOptions::default();
    let mut file: Option<String> = None;
    let mut compare: Option<String> = None;
    let mut targets: Option<String> = None;
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
            "--targets" => targets = value(),
            "--compare" => compare = value(),
            "-h" | "--help" => {
                println!("beatscript listen [--file F | stdin] [--section NAME] [--bars N] [--max-bars N] [--budget SECONDS] [--sr HZ] [--targets streaming|apple|broadcast|club|LUFS/dBTP] [--compare F]");
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
    if let Some(t) = targets {
        match Targets::parse(&t) {
            Ok(t) => lo.targets = t,
            Err(e) => return fail("usage", e),
        }
    }
    let before = match compare.as_deref().map(std::fs::read_to_string) {
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return fail("io", e.to_string()),
        None => None,
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut v = listen(&text, &lo)?;
        if let Some(b) = &before {
            let prev = listen(b, &lo)?;
            if let JsonValue::Object(o) = &mut v {
                let cmp = mix::compare(&prev, &v_clone_without_compare(o));
                o.set("compare", cmp);
            }
        }
        Ok::<JsonValue, RenderError>(v)
    }));
    match r {
        Ok(Ok(v)) => {
            println!("{}", v.js_stringified());
            0
        }
        Ok(Err(e)) => fail(e.code, e.message),
        Err(_) => fail("internal", "listen panicked".into()),
    }
}

/// An answer as an object value (for [`mix::compare`], before `compare` is set on it).
fn v_clone_without_compare(o: &JsonObject) -> JsonValue {
    let mut c = o.clone();
    c.remove("compare");
    JsonValue::Object(c)
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

    /// The value at `path` (`master/bands/sub`, `parts/0/lufs`) of an answer.
    fn at<'a>(v: &'a JsonValue, path: &str) -> Option<&'a JsonValue> {
        path.split('/').try_fold(v, |x, k| match k.parse::<usize>() {
            Ok(i) => x.as_array().and_then(|a| a.get(i)),
            Err(_) => x.get(k),
        })
    }

    fn near(v: &JsonValue, path: &str, want: f64, tol: f64) {
        let got = at(v, path).and_then(JsonValue::as_f64).unwrap_or_else(|| panic!("no number at {path} in {}", v.canonical_js()));
        assert!((got - want).abs() <= tol, "{path}: {got}, the golden is {want} (within {tol})");
    }

    fn part<'a>(v: &'a JsonValue, deck: &str, role: &str) -> &'a JsonValue {
        let parts = v.get("parts").and_then(JsonValue::as_array).expect("parts");
        parts.iter().find(|p| p.get("deck").and_then(JsonValue::as_str) == Some(deck) && p.get("part").and_then(JsonValue::as_str) == Some(role)).unwrap_or_else(|| panic!("no part {deck}.{role}"))
    }

    fn ids(v: &JsonValue) -> Vec<String> {
        v.get("warning_ids").and_then(JsonValue::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }

    fn without_ms(v: &JsonValue) -> String {
        let JsonValue::Object(o) = v else { return v.canonical_js() };
        let mut o = o.clone();
        o.remove("ms");
        JsonValue::Object(o).canonical_js()
    }

    const FIRST_BEAT: &str = include_str!("../../../site/examples/first-beat.beatscript");
    const DUBSTEP_V4: &str = include_str!("../../../site/examples/hand-dubstep-140-v4.beatscript");

    /// The mix (mix.rs) of first-beat, as numbers taken 2026-11-24 (it has no control lines, so how the render plays those
    /// does not move it): the master's loudness, true peak, dynamics, bands and stereo; the parts' bands and centroids; the
    /// kick and the sub bass sharing 60-250 Hz all through; the pad 26 LU under the mix; the targets; and the warnings, in
    /// the order to fix them. The same text gives the same answer.
    #[test]
    fn mix_golden_numbers_of_first_beat() {
        let lo = ListenOptions::default();
        let v = listen(FIRST_BEAT, &lo).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(v.get("version").and_then(JsonValue::as_str), Some("listen/2"));
        near(&v, "master/lufs", -12.7, 0.3);
        near(&v, "master/true_peak_db", -0.2, 0.3);
        near(&v, "master/crest_db", 13.8, 0.5);
        near(&v, "master/plr_db", 12.5, 0.5);
        near(&v, "master/short_term_max_lufs", -12.2, 0.3);
        near(&v, "master/lra", 0.6, 0.5);
        for (band, want) in [("sub", -24.1), ("low", -14.6), ("low_mid", -32.7), ("mid", -43.0), ("high_mid", -35.0), ("high", -33.9)] {
            near(&v, &format!("master/bands/{band}"), want, 1.0);
        }
        near(&v, "master/centroid_hz", 231.0, 15.0);
        near(&v, "master/width", 0.0, 0.02);
        near(&v, "master/correlation", 1.0, 0.02);
        let kick = part(&v, "A", "kick");
        near(kick, "centroid_hz", 117.0, 10.0);
        near(kick, "bands/low", -18.2, 1.0);
        near(kick, "crest_db", 10.8, 0.5);
        let hat = part(&v, "A", "hihat");
        near(hat, "centroid_hz", 8743.0, 100.0);
        near(hat, "bands/high", -29.8, 1.0);
        near(part(&v, "A", "snare"), "centroid_hz", 1701.0, 60.0);
        near(part(&v, "B", "bass"), "bands/low", -16.3, 1.0);
        near(part(&v, "B", "harmony"), "lufs", -38.7, 1.0);
        near(part(&v, "B", "harmony"), "width", 0.14, 0.05);
        let clashes = v.get("clashes").and_then(JsonValue::as_array).expect("clashes");
        assert_eq!(clashes.len(), 1, "{}", v.canonical_js());
        assert_eq!(clashes[0].get("parts").map(JsonValue::canonical_js).as_deref(), Some(r#"["B.bass","A.kick"]"#));
        assert_eq!(clashes[0].get("band").and_then(JsonValue::as_str), Some("low"));
        assert_eq!(clashes[0].get("bars").map(JsonValue::canonical_js).as_deref(), Some("[[1,8]]"));
        near(&v, "targets/lufs_delta", 1.3, 0.3);
        assert_eq!(at(&v, "targets/met").and_then(JsonValue::as_bool), Some(false));
        assert_eq!(ids(&v), ["clash:A.kick/B.bass:low", "buried:B.harmony", "true_peak", "loudness:high"], "{}", v.canonical_js());
        let w = v.get("warnings").map(JsonValue::canonical_js).unwrap_or_default();
        assert!(w.contains("duck B under A.kick 6db") && w.contains("`master: level 0.86` fixes both"), "{w}");
        assert_eq!(without_ms(&listen(FIRST_BEAT, &lo).unwrap()), without_ms(&v), "the same text, the same answer");
    }

    /// The loop, end to end: the duck first-beat's clash warning names, written in, takes the clash away, and a compare with the
    /// version before says the warning was fixed and the bass went down in the low band.
    #[test]
    fn the_duck_the_warning_names_fixes_the_clash_and_compare_says_so() {
        let lo = ListenOptions::default();
        let before = listen(FIRST_BEAT, &lo).unwrap();
        let ducked = FIRST_BEAT.replace("end at bar 8", "duck B under A.kick 6db\nend at bar 8");
        let after = listen(&ducked, &lo).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!ids(&after).iter().any(|i| i.starts_with("clash:")), "{}", after.canonical_js());
        let c = mix::compare(&before, &after);
        let fixed = c.get("fixed").map(JsonValue::canonical_js).unwrap_or_default();
        assert!(fixed.contains("deck B bass and deck A kick clash"), "{}", c.canonical_js());
        assert_eq!(c.get("introduced").map(JsonValue::canonical_js).as_deref(), Some("[]"), "{}", c.canonical_js());
        let bass = c.get("parts").and_then(JsonValue::as_array).and_then(|a| a.iter().find(|p| p.get("part").and_then(JsonValue::as_str) == Some("bass"))).cloned().expect("the bass moved");
        assert!(at(&bass, "bands/low").and_then(JsonValue::as_f64).is_some_and(|d| d < -0.5), "{}", c.canonical_js());
        // the bass a little lower brings the master within 1 LU of the target too: that warning goes as well
        assert!(c.get("summary").and_then(JsonValue::as_str).is_some_and(|s| s.ends_with("2 warnings fixed, 0 introduced, 2 kept")), "{}", c.canonical_js());
        assert!(fixed.contains("LU over the streaming target"), "{}", c.canonical_js());
    }

    /// The mix of hand-dubstep-140-v4 (the example the polish page works through), as numbers taken 2026-11-24: sections, two
    /// decks of several parts (each part measured on its own pass), the kick and the wobble bass sharing deck A and the sub in
    /// the drops, a true peak at the ceiling with the loudness under the target. Its control lines are played by the render
    /// as it was then: a change to how the render plays them moves these numbers, and is a reason to re-take them. Re-taken
    /// at render r3 (its control lines as the page plays them: the Wheel, the reverb as each strip's wet): the drop's true peak
    /// -1.8 -> -0.8 dB, A.bass's sub band -19.7 -> -21.1 dB, the rest within a few tenths.
    #[test]
    fn mix_golden_numbers_of_hand_dubstep_v4() {
        let v = listen(DUBSTEP_V4, &ListenOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
        near(&v, "master/lufs", -15.8, 0.5);
        near(&v, "master/true_peak_db", -0.8, 0.5);
        near(&v, "master/lra", 10.9, 1.5);
        near(&v, "master/short_term_max_lufs", -12.2, 0.5);
        for (band, want) in [("sub", -27.8), ("low", -22.0), ("low_mid", -24.2), ("mid", -25.8), ("high_mid", -35.5), ("high", -44.3)] {
            near(&v, &format!("master/bands/{band}"), want, 1.5);
        }
        near(&v, "master/centroid_hz", 377.0, 40.0);
        let bass = part(&v, "A", "bass");
        near(bass, "bands/sub", -21.1, 1.5);
        near(bass, "centroid_hz", 76.0, 10.0);
        near(part(&v, "A", "hihat"), "centroid_hz", 8729.0, 150.0);
        near(part(&v, "B", "melody"), "centroid_hz", 541.0, 50.0);
        let drop = v.get("sections").and_then(JsonValue::as_array).and_then(|a| a.iter().find(|s| s.get("name").and_then(JsonValue::as_str) == Some("drop"))).cloned().expect("drop");
        near(&drop, "true_peak_db", -0.8, 0.5);
        near(&drop, "bands/sub", -23.5, 1.5);
        let first = at(&v, "clashes/0").expect("a clash");
        assert_eq!(first.get("parts").map(JsonValue::canonical_js).as_deref(), Some(r#"["A.bass","A.kick"]"#), "{}", v.canonical_js());
        assert_eq!(first.get("band").and_then(JsonValue::as_str), Some("sub"));
        let ids = ids(&v);
        // it does not clip, so its true peak (at the ceiling) is said with the loudness, last
        assert_eq!(ids.first().map(String::as_str), Some("clash:A.bass/A.kick:sub"), "{ids:?}");
        assert_eq!(ids[ids.len().saturating_sub(2)..], ["true_peak", "loudness:low"], "{ids:?}");
        let w = v.get("warnings").map(JsonValue::canonical_js).unwrap_or_default();
        assert!(w.contains("put the bass on a deck of its own and `duck` it under A.kick"), "{w}");
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

    /// A switch of the control deck (`~ control off`, docs/beatscript/ext-control.md) changes the render: the section's `mix` that takes
    /// deck B down fires in the first play and is held back in the second, so B is silent in one and heard in the other, and
    /// listen names the held-back call on the section that plays without it.
    #[test]
    fn a_switch_of_the_control_deck_is_heard_and_said() {
        let text = "tempo 120\nkey am\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n  deck B: bass a1 ~ a1 ~ inst=sub-bass\n  control: mix { B level 0 }\n}\narrange {\n  drop\n  ~ control off\n  drop\n}\n";
        let lo = ListenOptions { max_bars: 8, budget_seconds: 0.0, ..ListenOptions::default() };
        let r = listen(text, &lo).unwrap_or_else(|e| panic!("{e:?}"));
        let secs = r.get("sections").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let name = |v: &JsonValue, k: &str| v.get(k).and_then(|n| n.as_str()).unwrap_or("").to_string();
        assert_eq!(secs.iter().map(|s| name(s, "name")).collect::<Vec<_>>(), ["drop", "drop@2"], "{}", r.canonical_js());
        assert!(secs[0].get("control_off").is_none(), "{}", r.canonical_js());
        assert_eq!(secs[1].get("control_off").map(|v| v.canonical_js()).as_deref(), Some(r#"["mix"]"#));
        let silent = r.get("silent").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let b_in = |sec: &str| silent.iter().any(|s| name(s, "deck") == "B" && name(s, "section") == sec);
        assert!(b_in("drop") && !b_in("drop@2"), "deck B is silent under the mix and heard without it: {}", r.canonical_js());
        // the same text, the same answer
        let again = listen(text, &lo).unwrap();
        assert_eq!(again.get("sections").map(|v| v.canonical_js()), r.get("sections").map(|v| v.canonical_js()));
    }

    /// The acid voice's classic line (docs/beatscript/ext-acid.md), as the item that asked for `acid-bass` wrote it, but for two
    /// words that do not check: a section cannot be named `a` (it reads as a note), and a plain `arrange` takes no `end` (a song
    /// ends after its last section by default), so the section is `main` and the arrangement `arrange main`.
    const ACID_LINE: &str = "#! beatscript 2\ntempo 124\nkey am\nlet acid = [1 1 ~ 1 3 ~ 1 5 ~ 1 4 ~ 1 1 7 1]\nsection main 8 bars {\n  deck A: kick e(4, 16) kit=drums-808\n  deck B: bass acid inst=acid-bass\n}\narrange main\n";

    /// The bass alone, mono, at 44.1 kHz: `extra` is added to its part line.
    fn acid_bass_alone(extra: &str) -> Vec<f64> {
        let text = ACID_LINE.replace("  deck A: kick e(4, 16) kit=drums-808\n", "").replace("inst=acid-bass", &format!("inst=acid-bass{extra}"));
        let o = Options { format: Format::F32le, max_bars: 2, ..Options::default() };
        let (mut pcm, mut err) = (Vec::new(), Vec::new());
        let mut ev = Events::new(&mut err);
        render::render_to(&text, &o, &mut pcm, &mut ev, Instant::now()).unwrap_or_else(|e| panic!("{e:?}"));
        pcm.chunks_exact(8).map(|f| (f64::from(f32::from_le_bytes([f[0], f[1], f[2], f[3]])) + f64::from(f32::from_le_bytes([f[4], f[5], f[6], f[7]]))) * 0.5).collect()
    }

    /// How much brighter each long note's start (5 to 45 ms in) is than its end (170 to 215 ms in), over two bars: the notes on
    /// steps 1, 4, 7 and 10, which have a rest after them, so each sounds its two steps (242 ms at 124) without the next note
    /// cutting in. Windows of 40 ms hold two periods of the lowest note (A1, 55 Hz), so where in its wave a window falls
    /// hardly matters. Over 1 when the filter's envelope sweeps down on each note (about 1.25 at the voice's defaults),
    /// about 1 when it does not (`env=0`: 0.97).
    fn strike_sweep(x: &[f64]) -> f64 {
        let bright = |w: &[f64]| {
            let e: f64 = w.iter().map(|v| v * v).sum();
            let d: f64 = w.windows(2).map(|p| (p[1] - p[0]) * (p[1] - p[0])).sum();
            (d / e.max(1e-30)).sqrt()
        };
        let step = 44_100.0 * 60.0 / 124.0 / 4.0;
        let ms = |m: f64| (m * 44.1) as usize;
        let (mut head, mut tail) = (0.0, 0.0);
        for bar in 0..2 {
            for h in [1, 4, 7, 10] {
                let at = ((bar * 16 + h) as f64 * step) as usize;
                assert!(at + ms(215.0) <= x.len(), "two bars rendered: {}", x.len());
                head += bright(&x[at + ms(5.0)..at + ms(45.0)]);
                tail += bright(&x[at + ms(170.0)..at + ms(215.0)]);
            }
        }
        head / tail
    }

    /// `inst=acid-bass` (the acid voice by its own name): the classic acid line checks with no word about its voice, `listen`
    /// hears the bass, and on each strike its filter moves (the envelope sweeps the cutoff down), which turning `env=0` takes away.
    #[test]
    fn the_acid_bass_line_checks_clean_is_heard_and_its_filter_moves() {
        let env = djhost::beatscript_env::BeatscriptEnv::new(Some(&beatscript_api::known::Known));
        let checked = djlive::compile::check_with(ACID_LINE, &env.env());
        assert!(checked.compiled.is_some(), "{:?}", checked.diagnostics);
        assert!(checked.diagnostics.is_empty(), "no warning, about its voice or anything: {:?}", checked.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
        let r = listen(ACID_LINE, &ListenOptions { max_bars: 8, budget_seconds: 0.0, ..ListenOptions::default() }).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.get("silent").and_then(JsonValue::as_array).is_some_and(|s| s.is_empty()), "{}", r.canonical_js());
        let parts = r.get("parts").and_then(JsonValue::as_array).cloned().unwrap_or_default();
        let bass = parts.iter().find(|p| p.get("part").and_then(JsonValue::as_str) == Some("bass")).unwrap_or_else(|| panic!("no bass part: {}", r.canonical_js()));
        assert_eq!(bass.get("voice").and_then(JsonValue::as_str), Some("acid-bass"));
        let lufs = bass.get("lufs").and_then(JsonValue::as_f64).unwrap_or(-99.0);
        assert!(lufs > -30.0, "the bass is heard: {}", bass.canonical_js());
        let (moving, flat) = (strike_sweep(&acid_bass_alone("")), strike_sweep(&acid_bass_alone(" env=0")));
        assert!(moving > 1.15, "each strike opens the filter and it falls: {moving}");
        assert!(moving > flat + 0.15, "env=0 takes the sweep away: {moving} vs {flat}");
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
