//! Clips from a render: a public CC0 BeatScript program, named by its program CID, rendered offline for a sample deck,
//! whole or a slice of its bars, as the mix or as one of its four DJ stems. It is the public half of the DJ language's
//! "sample from render" (its syntax is not written yet): a DJ plays the render of any public CC0 program on a sample deck,
//! and the beat grid is exact, because the program says its tempo. Nothing is detected.
//!
//! - **What a clip is.** [`render_clip`] gives stereo PCM from the offline renderer ([`crate::beatscript_render`], the engine the
//!   booth plays on), the program's tempo and bars, and a grid with bar 1 on frame 0, four beats a bar and the program's
//!   tempo map (under a tempo curve, each beat where the renderer played it). Its words are `render:<cid>`, or
//!   `render:<cid>[a..b]` for bars `a` to `b`, counted from 1 and both included, as `seed(techno, 42).bass[17..24]`
//!   counts them ([`ClipSpec`]). A slice is checked against the program's length ([`crate::beatscript_render::program_bars`]).
//! - **Only a CC0 public program.** A program kept with another licence, kept private, or carrying credits (CC BY work it
//!   draws on) is refused. So is a program that is restricted or names a sample or a loop: the same rule the site's render
//!   refuses by (`RightsReport::restricted`, `RightsReport::uses_assets`). So is a program that draws on others whose
//!   rights are not known to be CC0 and public. Each refusal is a [`ClipError`] that says why.
//! - **The same every time.** The same CID, slice, stem and sample rate give the same samples. [`RenderClip::sha256`] is
//!   the content id the engine names the audio by (`SampleBuffer::content_id`: SHA-256 over the rate and both channels'
//!   float bits), and [`RENDERER`] names what made it.
//! - **Exact stems.** [`render_clip_stem`] renders one of the four DJ stems: drums, bass, vocals, other. Every part of a
//!   program is its own net, so the split is exact, not a separation. [`NetStems`] sorts the nets: drums by channel 10-15
//!   or a drum name, bass and vocals by net id, and harmony, melody and anything else are `other`. The other stems' parts
//!   still play, at level 0, as the export makes its stems (docs/EXPORT.md): a kick that is not heard still pumps the bass,
//!   and a deck's loudness is still measured on its whole net. A deck's peak limiter is the one stage that is not linear,
//!   so a stem is two renders: the mix, each limiter keeping the gain it gave each frame, then the stem, each limiter
//!   giving those gains again. So a stem is what its parts add to the mix, and the four add up to the mix (but for a riser
//!   into a drop, a stinger and a control net, which are in no stem). A render's stems are the program's own parts: they
//!   are exact, and they are CC0 when the program is.
//!
//! [`to_prepared_sample`] and [`deck_source`] put a clip on a sample deck with the program's grid, so no beat detection runs.
//! The booth's `deck load spec=render:...` is a thin call into this module.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use djengine::beatgrid::{BeatGrid, GridSource};
use djengine::fx::GainTap;
use djengine::rights::{short_cid, License, Rights, Visibility};
use djengine::session::DeckSource;
use djengine::stems::{NetStems, StemClass, StemSource};
use djlive::compile::Compiled;
use djlive::env::Snippets;

use crate::beatscript_env::BeatscriptEnv;
use crate::beatscript_render;
use crate::deck::offline::OfflineConductor;
use crate::deck::sample::PreparedSample;
use crate::live_store::{LiveStore, Object};

/// The renderer a clip is made by, and its version: what [`RenderClip::renderer`] says. A change that moves a sample of a
/// render is a new version.
pub const RENDERER: &str = "render@1";

/// What a clip's words start with: `render:<cid>`.
pub const SPEC_PREFIX: &str = "render:";

// ---- the words

/// A clip as words: `render:<cid>`, or `render:<cid>[a..b]` for bars `a` to `b` (from 1, both included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSpec {
    /// The program's CID.
    pub cid: String,
    /// The bars, when it is a slice.
    pub slice: Option<(u32, u32)>,
}

impl ClipSpec {
    /// Read `render:<cid>` or `render:<cid>[a..b]`.
    pub fn parse(spec: &str) -> Result<ClipSpec, ClipError> {
        let s = spec.trim();
        let bad = |why: &str| ClipError::Spec(format!("{s:?} {why}: write render:<cid> or render:<cid>[a..b]"));
        let rest = s.strip_prefix(SPEC_PREFIX).ok_or_else(|| bad("is not a render"))?;
        let (cid, slice) = match rest.strip_suffix(']') {
            Some(head) => {
                let (cid, inner) = head.split_once('[').ok_or_else(|| bad("has a `]` with no `[`"))?;
                let (a, b) = inner.split_once("..").ok_or_else(|| bad("has no `..` between its bars"))?;
                let bar = |t: &str| t.trim().parse::<u32>().map_err(|_| bad("has bars that are not whole numbers"));
                (cid, Some((bar(a)?, bar(b)?)))
            }
            None => (rest, None),
        };
        if cid.is_empty() || !cid.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(bad("does not name a program by its CID"));
        }
        if let Some((a, b)) = slice {
            check_order(a, b)?;
        }
        Ok(ClipSpec { cid: cid.to_string(), slice })
    }

    /// The words again: `render:<cid>` or `render:<cid>[a..b]`.
    pub fn text(&self) -> String {
        match self.slice {
            Some((a, b)) => format!("{SPEC_PREFIX}{}[{a}..{b}]", self.cid),
            None => format!("{SPEC_PREFIX}{}", self.cid),
        }
    }
}

fn check_order(a: u32, b: u32) -> Result<(), ClipError> {
    if a == 0 {
        return Err(ClipError::Slice(format!("[{a}..{b}]: bars count from 1")));
    }
    if b < a {
        return Err(ClipError::Slice(format!("[{a}..{b}] runs backwards: the first bar comes first")));
    }
    Ok(())
}

// ---- the four DJ stems

/// The four stems a DJ mixes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DjStem {
    Drums,
    Bass,
    Vocals,
    Other,
}

impl DjStem {
    pub const ALL: [DjStem; 4] = [DjStem::Drums, DjStem::Bass, DjStem::Vocals, DjStem::Other];

    pub fn name(self) -> &'static str {
        match self {
            DjStem::Drums => "drums",
            DjStem::Bass => "bass",
            DjStem::Vocals => "vocals",
            DjStem::Other => "other",
        }
    }

    /// `drums`, `bass`, `vocals` or `other` (and the spellings [`StemClass::parse`] takes for those four).
    pub fn parse(s: &str) -> Option<DjStem> {
        match StemClass::parse(s)? {
            c @ (StemClass::Drums | StemClass::Bass | StemClass::Vocals | StemClass::Other) => Some(DjStem::of(c)),
            _ => None,
        }
    }

    /// The DJ stem a net of this class is in: harmony and melody are `other`.
    pub fn of(class: StemClass) -> DjStem {
        match class {
            StemClass::Drums => DjStem::Drums,
            StemClass::Bass => DjStem::Bass,
            StemClass::Vocals => DjStem::Vocals,
            StemClass::Harmony | StemClass::Melody | StemClass::Other => DjStem::Other,
        }
    }
}

/// The nets of `project` in `stem`, by id ([`NetStems`]; a control net or a stinger is in none).
pub fn stem_nets(project: &djengine::pflow::Project, stem: DjStem) -> BTreeSet<String> {
    NetStems.stems(Some(project)).unwrap_or_default().into_iter().filter(|s| DjStem::of(s.class) == stem).flat_map(|s| s.nets).collect()
}

// ---- why not

/// Why a clip was not made.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipError {
    /// The words are not `render:<cid>` or `render:<cid>[a..b]`.
    Spec(String),
    /// The sample rate is not one a render takes.
    SampleRate(String),
    /// No program has that CID here, or the text given is another program.
    NotFound(String),
    /// The program does not check (or its `#!` header lines do not read).
    Check(String),
    /// It is kept with this licence, not CC0-1.0.
    NotCc0(License),
    /// It is kept private.
    Private,
    /// It carries credits: it has CC BY work in it. The credit lines.
    Credits(Vec<String>),
    /// It is restricted: `use restricted`, `#! visibility: private` or a restricted asset.
    Restricted,
    /// It names samples or loops: their labels.
    UsesAssets(Vec<String>),
    /// It draws on a program whose rights are not CC0 and public, or not known here.
    DrawsOn(String),
    /// The bars are not bars of the program.
    Slice(String),
    /// The renderer failed.
    Render(String),
}

impl ClipError {
    /// A short machine word.
    pub fn code(&self) -> &'static str {
        match self {
            ClipError::Spec(_) => "spec",
            ClipError::SampleRate(_) => "sample_rate",
            ClipError::NotFound(_) => "not_found",
            ClipError::Check(_) => "check_failed",
            ClipError::NotCc0(_) => "not_cc0",
            ClipError::Private => "private",
            ClipError::Credits(_) => "credits",
            ClipError::Restricted => "restricted",
            ClipError::UsesAssets(_) => "uses_assets",
            ClipError::DrawsOn(_) => "draws_on",
            ClipError::Slice(_) => "slice",
            ClipError::Render(_) => "render",
        }
    }
}

impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const ONLY: &str = "a clip is made only from a public CC0 program, so that its audio and its stems are CC0 too";
        match self {
            ClipError::Spec(s) | ClipError::SampleRate(s) | ClipError::NotFound(s) | ClipError::Check(s) | ClipError::Slice(s) => f.write_str(s),
            ClipError::NotCc0(l) => write!(f, "the program is {}, not CC0 1.0: {ONLY}", l.label()),
            ClipError::Private => write!(f, "the program is private: {ONLY}"),
            ClipError::Credits(c) => write!(f, "the program has CC BY work in it ({}): {ONLY}", c.join("; ")),
            ClipError::Restricted => write!(f, "the program is restricted (`use restricted`, `#! visibility: private` or a restricted asset): it is private, and {ONLY}"),
            ClipError::UsesAssets(a) => write!(f, "the program uses samples or loops ({}): a render has no files, and their sounds are not CC0", a.join(", ")),
            ClipError::DrawsOn(s) => write!(f, "{s}: {ONLY}"),
            ClipError::Render(s) => write!(f, "the render failed: {s}"),
        }
    }
}

impl std::error::Error for ClipError {}

// ---- where the program comes from

/// A program and the rights it is kept with.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub text: String,
    pub rights: Rights,
}

/// Finds a program by its CID, with its rights: the booth's saved programs ([`LiveStore`]), or any store that keeps rights.
pub trait ProgramLookup {
    /// The program a whole program CID names, or why not.
    fn find(&self, cid: &str) -> Result<Stored, String>;
    /// Where the program's `import`s are read from (None: nowhere, so a program that imports does not check).
    fn snippets(&self) -> Option<&dyn Snippets> {
        None
    }
}

impl ProgramLookup for LiveStore {
    fn find(&self, cid: &str) -> Result<Stored, String> {
        let s = self.load(cid)?;
        // `load` takes a name or a part of a CID too: a clip names the whole CID
        if s.cid != cid {
            return Err(format!("no saved program has the CID {cid}"));
        }
        Ok(Stored { text: s.text, rights: s.rights })
    }

    fn snippets(&self) -> Option<&dyn Snippets> {
        Some(self)
    }
}

/// Where a clip's program comes from.
#[derive(Clone, Copy)]
pub enum Program<'a> {
    /// Its text, `#!` header lines and all, as a `.beatscript` file has it: the rights are the header's (`#! license:`,
    /// `#! visibility:`), CC0 and public where it says none. It may not import: nothing here says what an import is kept as.
    Text(&'a str),
    /// Looked up by its CID, with the rights it is kept with.
    Lookup(&'a dyn ProgramLookup),
}

/// A refusal unless the rights are CC0-1.0, public and credit nobody.
fn gate(r: &Rights) -> Result<(), ClipError> {
    if r.visibility != Visibility::Public {
        return Err(ClipError::Private);
    }
    if r.license != License::Cc0 {
        return Err(ClipError::NotCc0(r.license));
    }
    if !r.credits.is_empty() {
        return Err(ClipError::Credits(r.credits.iter().map(|c| c.text()).collect()));
    }
    Ok(())
}

/// The program `cid` names, checked as a render checks it, or why it is not rendered.
fn prepare(program: Program<'_>, cid: &str) -> Result<Compiled, ClipError> {
    let (stored, lookup) = match program {
        Program::Text(t) => {
            let (o, _) = Object::from_file(t).map_err(ClipError::Check)?;
            (Stored { text: t.to_string(), rights: o.rights }, None)
        }
        Program::Lookup(l) => (l.find(cid).map_err(ClipError::NotFound)?, Some(l)),
    };
    gate(&stored.rights)?;
    let mut env = BeatscriptEnv::new(lookup.and_then(|l| l.snippets()));
    // a render sings a `sing` line in its deck's net, as the page does (it has no vocal noun to call)
    env.voices.in_nets = true;
    let checked = djlive::compile::check_with(&stored.text, &env.env());
    let Some(c) = checked.compiled else {
        let errs: Vec<_> = checked.diagnostics.iter().filter(|d| d.is_error()).cloned().collect();
        let shown = djlive::diag::render_all(&errs, &stored.text);
        return Err(ClipError::Check(format!("the program has {} error{}: {}", errs.len(), if errs.len() == 1 { "" } else { "s" }, shown.trim())));
    };
    if c.cid != cid {
        return Err(ClipError::NotFound(format!("the program given is {}, not {}", short_cid(&c.cid), short_cid(cid))));
    }
    if c.rights.restricted() {
        return Err(ClipError::Restricted);
    }
    if c.rights.uses_assets() {
        return Err(ClipError::UsesAssets(c.rights.assets.iter().map(|a| a.label()).collect()));
    }
    // what it draws on (its imports, and theirs) is in the render too
    for (dep, _) in &c.plays {
        let short = short_cid(dep);
        let Some(l) = lookup else {
            return Err(ClipError::DrawsOn(format!("it draws on {short}, and a program given as text says nothing of that one's rights")));
        };
        let s = l.find(dep).map_err(|e| ClipError::DrawsOn(format!("it draws on {short}, which is not kept here ({e})")))?;
        gate(&s.rights).map_err(|e| ClipError::DrawsOn(format!("it draws on {short}, and {e}")))?;
    }
    Ok(c)
}

// ---- the clip

/// A render of a program, or of a slice of its bars, ready for a sample deck.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderClip {
    /// The program's CID.
    pub cid: String,
    /// `render:<cid>` or `render:<cid>[a..b]`: the clip's words ([`ClipSpec::text`]).
    pub source: String,
    /// The program's bars it covers, from 1, both included (the whole program: 1 to its length).
    pub from_bar: u32,
    pub to_bar: u32,
    pub bars: u32,
    /// One stem of it ([`render_clip_stem`]), or the mix.
    pub stem: Option<DjStem>,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: f64,
    /// The program's tempo at the clip's first beat (under a tempo curve, the tempo that beat is played at).
    pub bpm: f64,
    /// Every beat of the clip in seconds, bar 1 at 0, four beats a bar: steady at the program's tempo, or each beat where
    /// the tempo curve put it. Its source is `known`: nothing was detected.
    pub grid: BeatGrid,
    /// The content id the engine names the audio by (`SampleBuffer::content_id`), lower-case hex.
    pub sha256: String,
    /// [`RENDERER`].
    pub renderer: &'static str,
}

impl RenderClip {
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate
    }

    /// What a deck shows: the words, and the stem when it is one (`render:z…[17..24] bass`).
    pub fn title(&self) -> String {
        match self.stem {
            Some(s) => format!("{} {}", self.source, s.name()),
            None => self.source.clone(),
        }
    }
}

/// The render of the program `cid` names, or of its bars `slice` (from 1, both included), at `sample_rate`, or why not.
pub fn render_clip(program: Program<'_>, cid: &str, slice: Option<(u32, u32)>, sample_rate: f64) -> Result<RenderClip, ClipError> {
    render(program, cid, slice, sample_rate, None)
}

/// One DJ stem of [`render_clip`]: only the parts of `stem` are heard (see the module's words).
pub fn render_clip_stem(program: Program<'_>, cid: &str, slice: Option<(u32, u32)>, sample_rate: f64, stem: DjStem) -> Result<RenderClip, ClipError> {
    render(program, cid, slice, sample_rate, Some(stem))
}

/// The gains one deck's peak limiter gave the mix, one a frame ([`GainTap`]), and how many a replay has given. A loud mix
/// is limited on most of its frames (the test's: 96 in 100), so they are kept as they are: four bytes a frame a deck, half what
/// the clip's own audio takes.
#[derive(Default)]
struct Gains {
    kept: Vec<f32>,
    given: usize,
}

impl Gains {
    /// The next `n` gains (fewer at the end) onto the limiter's queue.
    fn feed(&mut self, q: &mut std::collections::VecDeque<f32>, n: usize) {
        let k = n.min(self.kept.len() - self.given);
        q.extend(&self.kept[self.given..self.given + k]);
        self.given += k;
    }
}

/// How far ahead of the engine a replay keeps a limiter's queue (frames): more than a block, which is the most the engine
/// takes between two looks.
const GAINS_AHEAD: usize = 4 * OfflineConductor::BLOCK_FRAMES;

/// What a stem's render is shown after every block (as the export makes its stems, docs/EXPORT.md). Its first pass is the
/// mix, every deck's limiter recording the gain it gave each frame. Its second is the stem: every lane of a deck's net that
/// is not in the stem plays at level 0, as each net is installed, and each deck's limiter applies the mix's gains instead
/// of its own. So a stem is its parts as they sat in the limited mix, and the four add up to the mix.
struct StemPass {
    /// None: the mix, recording.
    stem: Option<DjStem>,
    gains: BTreeMap<String, Gains>,
    /// (deck, the net's generation) to the ids of its nets that are in the stem.
    kept: BTreeMap<(String, i32), BTreeSet<String>>,
}

impl StemPass {
    fn apply(&mut self, cond: &mut OfflineConductor) {
        let decks: Vec<String> = cond.engine().decks.iter().map(|d| d.name.clone()).collect();
        for d in decks {
            let Some(core) = cond.renderer.deck_mut(&d) else { continue };
            if let Some(lim) = core.limiter.as_mut() {
                let g = self.gains.entry(d.clone()).or_default();
                match self.stem {
                    None => {
                        lim.tap = GainTap::Record;
                        g.kept.extend(lim.tap_gains.drain(..));
                    }
                    Some(_) => {
                        lim.tap = GainTap::Replay;
                        let short = GAINS_AHEAD.saturating_sub(lim.tap_gains.len());
                        g.feed(&mut lim.tap_gains, short);
                    }
                }
            }
            let Some(stem) = self.stem else { continue };
            let Some(p) = core.runner.project_mut() else { continue };
            let keep = self.kept.entry((d, p.generation)).or_insert_with(|| stem_nets(&p.graph.project, stem));
            for lane in p.lanes_mut() {
                if !keep.contains(&lane.net_id) && lane.level() != 0.0 {
                    lane.set_base_level(0.0);
                }
            }
            // the riser into a drop belongs to no part: it is in the mix only
            p.riser = None;
        }
    }
}

fn render(program: Program<'_>, cid: &str, slice: Option<(u32, u32)>, sample_rate: f64, stem: Option<DjStem>) -> Result<RenderClip, ClipError> {
    let sr = crate::beatscript_test::sample_rate(sample_rate).map_err(ClipError::SampleRate)?;
    if let Some((a, b)) = slice {
        check_order(a, b)?;
    }
    let c = prepare(program, cid)?;
    let total = beatscript_render::program_bars(&c);
    let (from, to) = slice.unwrap_or((1, total));
    if to > total {
        return Err(ClipError::Slice(format!("[{from}..{to}] is past the program's end: it is {total} bar{} long", if total == 1 { "" } else { "s" })));
    }
    let (json, beats) = beatscript_render::arrangement_bars(&c, total);
    let arr = beatscript_render::parsed(&json).map_err(ClipError::Render)?;
    let (b0, b1) = (f64::from(from - 1) * 4.0, f64::from(to) * 4.0);
    let r = match stem {
        Some(s) => {
            // the mix first, for the gains its limiters gave it, then the stem with those gains (`StemPass`)
            let mut mix = StemPass { stem: None, gains: BTreeMap::new(), kept: BTreeMap::new() };
            beatscript_render::render_with(&c, &arr, beats, b1, sr, true, Some(&mut |cond: &mut OfflineConductor| mix.apply(cond))).map_err(ClipError::Render)?;
            let mut one = StemPass { stem: Some(s), gains: mix.gains, kept: BTreeMap::new() };
            beatscript_render::render_with(&c, &arr, beats, b1, sr, true, Some(&mut |cond: &mut OfflineConductor| one.apply(cond)))
        }
        None => beatscript_render::render_with(&c, &arr, beats, b1, sr, true, None),
    }
    .map_err(ClipError::Render)?;
    // the frames of the bars, on the frames the renderer put their beats on
    let frame_of = |beat: f64| (arr.seconds_at(beat) * sr).round().max(0.0) as usize;
    let (start, end) = (frame_of(b0), frame_of(b1));
    if r.audio.left.len() < end || r.audio.right.len() < end || start >= end {
        return Err(ClipError::Render(format!("frames {start}..{end} were asked for and {} were rendered", r.audio.left.len())));
    }
    let (left, right) = (r.audio.left[start..end].to_vec(), r.audio.right[start..end].to_vec());
    // the grid: the tempo the program says (a curve: each beat where the renderer's clock put it)
    let n = (to - from + 1) as usize * 4;
    let (bpm, period, steady, beat_times) = if c.tempo_map().drives() {
        let t0 = arr.seconds_at(b0);
        let times: Vec<f64> = (0..n).map(|k| arr.seconds_at(b0 + k as f64) - t0).collect();
        (60.0 / (arr.seconds_at(b0 + 1.0) - t0), (arr.seconds_at(b1) - t0) / n as f64, false, times)
    } else {
        let period = 60.0 / arr.bpm;
        (arr.bpm, period, true, (0..n).map(|k| k as f64 * period).collect())
    };
    let grid = BeatGrid { source: GridSource::Known, period, beats_per_bar: 4, steady, confidence: 1.0, residual_ms: 0.0, beats: beat_times, first_downbeat: 0 };
    let sha256 = djengine::session::model::sample_content_id(sr, &left, Some(&right));
    let source = ClipSpec { cid: c.cid.clone(), slice }.text();
    Ok(RenderClip { cid: c.cid, source, from_bar: from, to_bar: to, bars: to - from + 1, stem, left, right, sample_rate: sr, bpm, grid, sha256, renderer: RENDERER })
}

// ---- onto a sample deck

/// The engine's grid for a clip: its tempo, beat 1 on its first sample. The engine's sample grid is one tempo, so a clip
/// under a tempo curve has its first beat's tempo there (the clip's own [`RenderClip::grid`] has every beat).
pub fn engine_grid(clip: &RenderClip) -> djengine::deck::beat_grid::BeatGrid {
    djengine::deck::beat_grid::BeatGrid::new(clip.bpm, 0.0, 1.0)
}

/// The clip ready for `Engine::load_sample`: its audio, its title, and the program's grid, so no beat detection runs.
pub fn to_prepared_sample(clip: &RenderClip) -> PreparedSample {
    let buffer = djengine::deck::turntable::SampleBuffer::new(clip.left.clone(), Some(clip.right.clone()), clip.sample_rate);
    PreparedSample::with_grid(Arc::new(buffer), &clip.title(), Some(engine_grid(clip)))
}

/// What a session records the deck as holding: the sample by its content id (the clip's `sha256`), its title and grid.
pub fn deck_source(clip: &RenderClip) -> DeckSource {
    DeckSource::Sample { id: clip.sha256.clone(), title: clip.title(), grid: Some(engine_grid(clip)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 22_050.0;
    /// Four bars at a tempo whose beat is not a whole number of frames.
    const TINY: &str = "tempo 127\ndeck A: kick x ~ x ~ | bass ~ c2 ~ c2\nend at bar 4\n";

    fn cid_of(text: &str) -> String {
        djlive::compile::check_with(text, &BeatscriptEnv::new(None).env()).compiled.unwrap_or_else(|| panic!("{text}")).cid
    }

    fn rms(a: &[f32], sr: f64, from: f64, to: f64) -> f64 {
        let (i, j) = ((from * sr) as usize, ((to * sr) as usize).min(a.len()));
        (a[i..j].iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / (j - i).max(1) as f64).sqrt()
    }

    fn store(tag: &str) -> (LiveStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("djhost-clips-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        (LiveStore::new(&dir), dir)
    }

    #[test]
    fn the_words_read_and_print() {
        let s = ClipSpec::parse("render:zAbc123").unwrap();
        assert_eq!((s.cid.as_str(), s.slice), ("zAbc123", None));
        let s = ClipSpec::parse(" render:zAbc123[17..24] ").unwrap();
        assert_eq!(s.slice, Some((17, 24)));
        assert_eq!(s.text(), "render:zAbc123[17..24]");
        assert_eq!(ClipSpec::parse("render:zAbc123[3..3]").unwrap().slice, Some((3, 3)));
        for bad in ["render:", "sample:zAbc", "render:z/x", "render:zAbc[1..x]", "render:zAbc[1-4]", "render:zAbc]"] {
            assert_eq!(ClipSpec::parse(bad).unwrap_err().code(), "spec", "{bad}");
        }
        assert_eq!(ClipSpec::parse("render:zAbc[0..4]").unwrap_err().code(), "slice");
        assert_eq!(ClipSpec::parse("render:zAbc[5..4]").unwrap_err().code(), "slice");
        assert_eq!(DjStem::parse("vox"), Some(DjStem::Vocals));
        assert_eq!(DjStem::parse("melody"), None, "a melody is in `other`, not a stem of its own");
        assert_eq!(DjStem::of(StemClass::Harmony), DjStem::Other);
    }

    #[test]
    fn a_tiny_cc0_program_renders_with_an_exact_grid() {
        let cid = cid_of(TINY);
        let clip = render_clip(Program::Text(TINY), &cid, None, SR).unwrap();
        assert_eq!((clip.from_bar, clip.to_bar, clip.bars), (1, 4, 4));
        assert_eq!(clip.source, format!("render:{cid}"));
        assert_eq!(clip.renderer, "render@1");
        assert_eq!(clip.bpm, 127.0);
        let want = 4.0 * 4.0 * 60.0 / 127.0 * SR;
        assert!((clip.frames() as f64 - want).abs() <= 1.0, "{} frames, {want} wanted", clip.frames());
        assert_eq!(clip.right.len(), clip.frames());
        // bar 1 on frame 0, four beats a bar, every beat at the program's tempo; nothing detected
        let g = &clip.grid;
        assert_eq!((g.source, g.beats_per_bar, g.steady, g.first_downbeat, g.beats.len()), (GridSource::Known, 4, true, 0, 16));
        assert_eq!(g.beats[0], 0.0);
        assert!((g.bpm() - 127.0).abs() < 1e-9);
        assert!((g.beats[15] - 15.0 * 60.0 / 127.0).abs() < 1e-12);
        let peak = clip.left.iter().chain(&clip.right).fold(0f32, |p, x| p.max(x.abs()));
        assert!(peak > 0.01, "it is heard: {peak}");
        assert_eq!(clip.sha256.len(), 64);
        assert!(clip.sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_slice_is_its_bars_of_the_whole_render_to_the_frame() {
        let cid = cid_of(TINY);
        let whole = render_clip(Program::Text(TINY), &cid, None, SR).unwrap();
        let slice = render_clip(Program::Text(TINY), &cid, Some((2, 3)), SR).unwrap();
        assert_eq!(slice.source, format!("render:{cid}[2..3]"));
        assert_eq!(slice.bars, 2);
        let want = 2.0 * 4.0 * 60.0 / 127.0 * SR;
        assert!((slice.frames() as f64 - want).abs() <= 1.0, "{} frames, {want} wanted", slice.frames());
        // the same samples as the whole render's bars 2 and 3 (bar 1 is 4 beats long)
        let start = (4.0 * 60.0 / 127.0 * SR).round() as usize;
        assert_eq!(slice.left[..], whole.left[start..start + slice.frames()]);
        assert_eq!(slice.right[..], whole.right[start..start + slice.frames()]);
        assert_eq!(slice.grid.beats.len(), 8);
        assert_eq!(slice.grid.beats[0], 0.0);
        // one bar, the last
        assert_eq!(render_clip(Program::Text(TINY), &cid, Some((4, 4)), SR).unwrap().bars, 1);
        // past the end, from bar 0, backwards
        for (s, code) in [((3, 5), "slice"), ((0, 2), "slice"), ((3, 2), "slice")] {
            assert_eq!(render_clip(Program::Text(TINY), &cid, Some(s), SR).unwrap_err().code(), code, "{s:?}");
        }
        assert_eq!(render_clip(Program::Text(TINY), &cid, None, 0.0).unwrap_err().code(), "sample_rate");
    }

    #[test]
    fn the_same_cid_and_slice_give_the_same_sha256() {
        let cid = cid_of(TINY);
        let a = render_clip(Program::Text(TINY), &cid, Some((1, 2)), SR).unwrap();
        let b = render_clip(Program::Text(TINY), &cid, Some((1, 2)), SR).unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert_eq!(a.left, b.left);
        let other = render_clip(Program::Text(TINY), &cid, Some((2, 3)), SR).unwrap();
        assert_ne!(a.sha256, other.sha256);
        // from the store, the same program is the same clip
        let (s, dir) = store("same");
        s.save("tiny", TINY).unwrap();
        let c = render_clip(Program::Lookup(&s), &cid, Some((1, 2)), SR).unwrap();
        assert_eq!(c.sha256, a.sha256);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_program_that_is_not_public_cc0_or_uses_assets_is_refused() {
        // the header's rights
        let by = format!("#! license: CC-BY-4.0\n{TINY}");
        assert_eq!(render_clip(Program::Text(&by), &cid_of(&by), None, SR).unwrap_err(), ClipError::NotCc0(License::CcBy4));
        let arr = format!("#! license: all-rights-reserved\n{TINY}");
        assert_eq!(render_clip(Program::Text(&arr), &cid_of(&arr), None, SR).unwrap_err(), ClipError::NotCc0(License::AllRightsReserved));
        let private = format!("#! visibility: private\n{TINY}");
        assert_eq!(render_clip(Program::Text(&private), &cid_of(&private), None, SR).unwrap_err(), ClipError::Private);
        // the text's own: restricted, a sample (even a CC0 one: a render has no files)
        let restricted = format!("use restricted\n{TINY}");
        assert_eq!(render_clip(Program::Text(&restricted), &cid_of(&restricted), None, SR).unwrap_err(), ClipError::Restricted);
        let assets = "sample harp from \"/no/such/file.wav\" license cc0 else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~\nend at bar 2\n";
        let e = render_clip(Program::Text(assets), &cid_of(assets), None, SR).unwrap_err();
        assert_eq!(e.code(), "uses_assets", "{e}");
        assert!(e.to_string().contains("harp"), "it names the sample: {e}");
        // the store's rights
        let (s, dir) = store("rights");
        let cid = cid_of(TINY);
        s.save_with("tiny", TINY, Some(Rights { license: License::CcBy4, ..Rights::default() })).unwrap();
        let e = render_clip(Program::Lookup(&s), &cid, None, SR).unwrap_err();
        assert_eq!(e, ClipError::NotCc0(License::CcBy4));
        assert!(e.to_string().contains("CC BY 4.0"), "{e}");
        s.set_rights(&cid, Some(License::AllRightsReserved), None).unwrap();
        assert_eq!(render_clip(Program::Lookup(&s), &cid, None, SR).unwrap_err(), ClipError::NotCc0(License::AllRightsReserved));
        s.set_rights(&cid, Some(License::Cc0), Some(Visibility::Private)).unwrap();
        assert_eq!(render_clip(Program::Lookup(&s), &cid, None, SR).unwrap_err(), ClipError::Private);
        s.set_rights(&cid, None, Some(Visibility::Public)).unwrap();
        assert!(render_clip(Program::Lookup(&s), &cid, Some((1, 1)), SR).is_ok(), "CC0 and public again");
        // a CID the store does not have, and a text that is another program
        assert_eq!(render_clip(Program::Lookup(&s), "zNoSuchProgram", None, SR).unwrap_err().code(), "not_found");
        assert_eq!(render_clip(Program::Text(TINY), &cid_of(assets), None, SR).unwrap_err().code(), "not_found");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_drums_stem_has_the_kick_and_not_the_bass() {
        // a kick on beat 1, the bass on beat 3 (a beat is half a second)
        let text = "tempo 120\ndeck A: kick x ~ ~ ~ | bass ~ ~ c2 ~\nend at bar 2\n";
        let cid = cid_of(text);
        let drums = render_clip_stem(Program::Text(text), &cid, None, SR, DjStem::Drums).unwrap();
        let bass = render_clip_stem(Program::Text(text), &cid, None, SR, DjStem::Bass).unwrap();
        assert_eq!((drums.stem, drums.title()), (Some(DjStem::Drums), format!("render:{cid} drums")));
        let (kick, low) = ((0.0, 0.12), (1.0, 1.12));
        let d = (rms(&drums.left, SR, kick.0, kick.1), rms(&drums.left, SR, low.0, low.1));
        let b = (rms(&bass.left, SR, kick.0, kick.1), rms(&bass.left, SR, low.0, low.1));
        assert!(d.0 > 0.01, "the kick is in the drums: {d:?}");
        assert!(d.1 < d.0 / 10.0, "the bass is not: {d:?}");
        assert!(b.1 > 0.01 && b.0 < b.1 / 10.0, "and the bass stem is the other way round: {b:?}");
        assert_ne!(drums.sha256, bass.sha256);
        assert_eq!(drums.frames(), bass.frames());
    }

    #[test]
    fn the_four_stems_add_up_to_the_mix() {
        let text = "tempo 124\nkey am\ndeck A: kick x ~ x ~ | hihat ~ x ~ x | bass a1 ~ a1 c2\ndeck B: melody <[a3 c4 e4 c4] [a3 c4 e4 g4]> | harmony chord(am) inst=warm-pad\ndeck B: sing \"we are the rave to-night\" on melody voice=robot-choir\nend at bar 4\n";
        let cid = cid_of(text);
        let peak = |a: &[f32]| a.iter().fold(0f32, |p, x| p.max(x.abs()));
        // the limiter moves in this mix, so the stems add up only because each replays the mix's gains: before they did
        // (each stem limited on its own), the sum was 9.0 dB under the peak at worst and 29.2 dB in rms
        let c = prepare(Program::Text(text), &cid).unwrap();
        let (json, beats) = beatscript_render::arrangement_bars(&c, beatscript_render::program_bars(&c));
        let arr = beatscript_render::parsed(&json).unwrap();
        let mut rec = StemPass { stem: None, gains: BTreeMap::new(), kept: BTreeMap::new() };
        let r = beatscript_render::render_with(&c, &arr, beats, beats, SR, true, Some(&mut |cond: &mut OfflineConductor| rec.apply(cond))).unwrap();
        for (d, g) in &rec.gains {
            assert_eq!(g.kept.len(), r.audio.left.len(), "deck {d}'s limiter kept a gain a frame");
        }
        let limited = rec.gains.values().flat_map(|g| &g.kept).filter(|g| **g < 1.0).count();
        let deck_frames: usize = rec.gains.values().map(|g| g.kept.len()).sum();
        assert!(limited > 0, "a limiter moved in this mix");
        for slice in [None, Some((2, 3))] {
            let mix = render_clip(Program::Text(text), &cid, slice, SR).unwrap();
            let stems: Vec<RenderClip> = DjStem::ALL.iter().map(|s| render_clip_stem(Program::Text(text), &cid, slice, SR, *s).unwrap()).collect();
            for s in &stems {
                assert_eq!(s.frames(), mix.frames());
                assert!(peak(&s.left) > 0.001, "the {} stem is heard: {}", s.stem.unwrap().name(), peak(&s.left));
            }
            let (mut max_abs, mut sq) = (0f64, 0f64);
            for i in 0..mix.frames() {
                for (ch, m) in [(0, mix.left[i]), (1, mix.right[i])] {
                    let sum: f64 = stems.iter().map(|s| f64::from(if ch == 0 { s.left[i] } else { s.right[i] })).sum();
                    let d = (sum - f64::from(m)).abs();
                    max_abs = max_abs.max(d);
                    sq += d * d;
                }
            }
            let p = f64::from(peak(&mix.left).max(peak(&mix.right)));
            let rms_err = (sq / (2 * mix.frames()) as f64).sqrt();
            let db = |x: f64| 20.0 * (x.max(1e-12) / p).log10();
            eprintln!("stems sum check {slice:?}: peak {p:.4}, max |sum - mix| {max_abs:.3e} ({:.1} dB under the peak), rms {rms_err:.3e} ({:.1} dB); {limited} of {deck_frames} deck frames limited", db(max_abs), db(rms_err));
            // measured on gir (22.05 kHz, the whole program): the worst sample 136.1 dB under the peak (1.4e-7), the rms
            // 156.5 dB under it (1.4e-8), float rounding; the bounds leave 16 dB
            assert!(db(max_abs) < -120.0, "the stems add up to the mix: the worst sample is {:.1} dB under the peak", db(max_abs));
            assert!(db(rms_err) < -140.0, "the stems add up to the mix: the rms error is {:.1} dB under the peak", db(rms_err));
        }
    }

    #[test]
    fn a_tempo_curve_is_in_the_grid() {
        let text = "tempo 100 -> 130 over 4 bars\ndeck A: kick x ~ x ~\nend at bar 4\n";
        let cid = cid_of(text);
        let clip = render_clip(Program::Text(text), &cid, None, SR).unwrap();
        let g = &clip.grid;
        assert!(!g.steady);
        assert_eq!((g.beats.len(), g.beats[0]), (16, 0.0));
        let first = g.beats[1] - g.beats[0];
        let last = g.beats[15] - g.beats[14];
        assert!(first > last, "the beats close up as the tempo rises: {first} then {last}");
        assert!((clip.bpm - 100.0).abs() < 2.0, "{}", clip.bpm);
        // as long as the tempo map says, to the frame
        let c = djlive::compile::check_with(text, &BeatscriptEnv::new(None).env()).compiled.unwrap();
        let secs = c.tempo_map().total_seconds(4u32);
        assert!((clip.frames() as f64 - secs * SR).abs() <= 1.0, "{} frames, {} wanted", clip.frames(), secs * SR);
    }

    #[test]
    fn a_clip_goes_on_a_sample_deck_with_its_grid() {
        let cid = cid_of(TINY);
        let clip = render_clip(Program::Text(TINY), &cid, Some((1, 2)), SR).unwrap();
        let p = to_prepared_sample(&clip);
        assert_eq!(p.buffer.content_id(), clip.sha256, "the deck names the audio by the clip's sha256");
        let g = p.grid.expect("the program's grid, not a detected one");
        assert_eq!((g.bpm, g.offset, g.confidence), (127.0, 0.0, 1.0));
        assert_eq!(p.title, clip.source);
        let (mut e, _r) = crate::deck::engine(crate::deck::EngineConfig::new(SR));
        assert!(e.load_sample("A", p));
        assert!(e.is_sample_deck("A"));
        assert_eq!(e.base_bpm("A"), Some(127.0));
        match deck_source(&clip) {
            DeckSource::Sample { id, title, grid } => {
                assert_eq!((id, title), (clip.sha256.clone(), clip.source.clone()));
                assert_eq!(grid.map(|g| g.bpm), Some(127.0));
            }
            other => panic!("{other:?}"),
        }
    }
}
