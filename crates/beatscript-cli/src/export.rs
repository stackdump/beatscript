//! `beatscript export`: a BeatScript program for a DAW. A Standard MIDI File of what the engine plays, and stems: one WAV
//! per part and per deck that add back up to the master render. docs/EXPORT.md says what is in the files and why.
//!
//! Everything comes out of the render itself ([`crate::render::drive`], the same loop the streaming render runs):
//!
//! - **MIDI.** The engine's own record of every note it strikes (`FireEvent`: frame, note, velocity as bound, the
//!   seeded `vel=` picks included, channel, length), mapped to its part by the net that fired it, put on its
//!   sixteenth step. So the file is the render's notes, not a second reading of the program.
//! - **Stems.** One more render per part, and per deck, with every other part's lanes at level 0 (`Lane::set_base_level`):
//!   the muted lanes still run, their kicks still duck the bass, their envelopes still drive what they modulate, so the
//!   part plays exactly as it does in the mix. The deck's output stage, its strip, and the master's own effects (a
//!   `master: filter ...`) are applied to each stem as they are to the mix. They are linear except the deck's soft
//!   clip, so the stems add up to the master to within what that clip takes off a loud mix: the manifest's
//!   `sum_check` measures it on the files themselves.
//!
//! Deterministic: the same canonical text gives the same bytes (WAV, MIDI, manifest and zip have no clock in them).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use djengine::json::{JsonObject, JsonValue};
use djengine::musicality::note_log::group_name;
use djengine::smf;
use djengine::fx::GainTap;
use djengine::wave::FireFlags;
use djhost::beatscript_env::BeatscriptEnv;
use djhost::deck::offline::OfflineConductor;
use djlive::compile::Compiled;

use crate::render::{drive, plan, Controls, Driver, Options, RenderError};

/// Bits a sample in the stems' WAVs: 32-bit float, so nothing is clipped (a mix that peaks over full scale stays
/// whole, and the stems still add up to it).
const WAV_BITS: u16 = 32;
/// Bytes of [`wav_header`].
const WAV_HEADER: u64 = 58;

/// What an export is asked for.
#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub sr: u32,
    pub max_bars: u32,
    /// The rights the program is stored with, for the manifest and the MIDI copyright line (None: the program's own
    /// `#! license:`, else CC0-1.0).
    pub rights: Rights,
    /// Stems renders run side by side on this many threads (each its own engine); 1: one after the other.
    pub threads: usize,
    /// Dry stems ([`stems`]): each part's voices before the deck and the master, the moves taken out written to the manifest.
    pub dry: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { sr: 44_100, max_bars: 256, rights: Rights::default(), threads: 1, dry: false }
    }
}

/// The stored rights (`api.Meta`): what the manifest credits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rights {
    pub license: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub forked_from: Option<String>,
    pub credits: Vec<String>,
}

/// One note the engine played.
#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub deck: String,
    pub part: String,
    /// The song's sixteenth step it sounded on (0: the first).
    pub step: i64,
    pub note: u8,
    pub velocity: u8,
    /// The engine's channel, 1…16 (10…15 are its drum lanes).
    pub channel: u8,
    pub len_steps: f64,
    pub drum: bool,
}

/// A part of the song: a deck and a role.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub deck: String,
    pub role: String,
    pub instrument: Option<String>,
    pub drum: bool,
    /// The MIDI channel it is written on (0-based).
    pub channel: u8,
    pub program: Option<u8>,
    pub notes: usize,
}

impl Part {
    /// `A kick`: the MIDI track's name.
    pub fn title(&self) -> String {
        format!("{} {}", self.deck, self.role)
    }
    /// `A-kick.wav`.
    pub fn file(&self) -> String {
        format!("{}-{}.wav", self.deck, self.role)
    }
}

/// How far the stems are from the master, on the files.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SumCheck {
    /// The master's largest sample.
    pub peak: f64,
    /// The largest |sum of stems − master| over every sample.
    pub max_abs: f64,
    pub rms: f64,
}

/// What an export made.
#[derive(Clone, Debug)]
#[allow(dead_code)] // `played` and the rest are read by the tests and beatscript-serve
pub struct Exported {
    pub cid: String,
    pub name: String,
    pub bpm: f64,
    pub key: Option<String>,
    pub bars: u32,
    pub frames: u64,
    pub sr: u32,
    pub parts: Vec<Part>,
    pub decks: Vec<String>,
    pub played: Vec<Played>,
    pub midi: Vec<u8>,
    /// `(path in the bundle, kind)`, in order.
    pub files: Vec<(String, &'static str)>,
    /// Sum of the part stems, and of the deck stems, against the master (stems only).
    pub parts_sum: Option<SumCheck>,
    pub decks_sum: Option<SumCheck>,
    pub master_fx: Vec<String>,
    pub notes: Vec<String>,
    pub rights: Rights,
    pub tempo_curve: bool,
    /// The stems' codec: `wav` (32-bit float), or `opus` once a server has encoded them.
    pub codec: &'static str,
    /// Dry stems: the control lines whose moves the parts were rendered without (None: the stems are wet, as in the mix).
    pub dry: Option<Vec<Automation>>,
}

/// A control line the dry stems were rendered without, and what it did in the mix ([`crate::render::Controls::keep_transport`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Automation {
    /// 1-based, in the program text, and the line as written.
    pub line: usize,
    pub text: String,
    /// `master`, `deck` (then `deck` names it) or `other` (the crossfader, a line on no one deck).
    pub scope: &'static str,
    pub deck: Option<String>,
    /// Where the line fires, in beats from the start; `hold`: put back after this many bars.
    pub fires: Vec<f64>,
    pub hold_bars: Option<f64>,
    /// The moves it made in the mix, in time order.
    pub moves: Vec<crate::render::RemovedMove>,
}

// ---------------------------------------------------------------------------------------------
// the passes

/// Which lanes a pass hears: all (the master), one part, or one deck.
#[derive(Clone, Debug, PartialEq)]
enum Solo {
    All,
    Part(String, String),
    Deck(String),
    /// The Wheel alone (`wheel.wav`): every deck's lanes at level 0, the Wheel added as the page adds it to the master. Every
    /// other stem leaves it out, so the stems still add up to the master.
    Wheel,
}

/// The deck's peak limiter in a pass (`djengine::fx::GainTap`): the master pass records the gain it gave the mix,
/// a part's pass replays it, so the part comes out as it sat in the mix (see the module's words).
enum Curve {
    Off,
    Record(BTreeMap<String, BufWriter<File>>),
    Replay(BTreeMap<String, BufReader<File>>),
}

/// The gains a replaying runner is kept ahead by (samples).
const CURVE_AHEAD: usize = 4 * crate::render::BLOCK_SAMPLES;


struct Pass<'w> {
    decks: Vec<String>,
    solo: Solo,
    curve: Curve,
    io_err: Option<std::io::Error>,
    collect: bool,
    spt: f64,
    bpm: f64,
    /// (deck, generation) -> the part of each of its nets, in graph order.
    gens: BTreeMap<(String, i32), Vec<String>>,
    played: Vec<Played>,
    out: Option<&'w mut WavOut>,
    /// A dry pass: the soloed deck's strip at unity, its limiter out and its room and echo sends at 0.
    dry: bool,
}

impl Pass<'_> {
    fn touch(&mut self, cond: &mut OfflineConductor) {
        if self.dry {
            self.dry_strip(cond);
        }
        for d in &self.decks {
            let Some(core) = cond.renderer.deck_mut(d) else { continue };
            core.runner.record_events = self.collect;
            match (&mut self.curve, core.limiter.as_mut()) {
                (Curve::Record(files), Some(lim)) => {
                    lim.tap = GainTap::Record;
                    let w = files.get_mut(d).expect("a file per deck");
                    let mut b = Vec::with_capacity(lim.tap_gains.len() * 4);
                    for g in lim.tap_gains.drain(..) {
                        b.extend_from_slice(&g.to_le_bytes());
                    }
                    if let Err(e) = w.write_all(&b) {
                        self.io_err.get_or_insert(e);
                    }
                }
                (Curve::Replay(files), Some(lim)) => {
                    lim.tap = GainTap::Replay;
                    let r = files.get_mut(d).expect("a file per deck");
                    let q = &mut lim.tap_gains;
                    if q.len() < CURVE_AHEAD {
                        let mut b = vec![0u8; CURVE_AHEAD * 4 * 2];
                        let mut got = 0;
                        while got < b.len() {
                            match r.read(&mut b[got..]) {
                                Ok(0) => break,
                                Ok(k) => got += k,
                                Err(e) => {
                                    self.io_err.get_or_insert(e);
                                    break;
                                }
                            }
                        }
                        q.extend(b[..got - got % 4].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])));
                    }
                }
                _ => {}
            }
            if let Some(p) = core.runner.project_mut() {
                let key = (d.clone(), p.generation);
                if !self.gens.contains_key(&key) {
                    let names: Vec<String> = p.graph.nets.iter().map(|n| group_name(&n.bundle, &n.id)).collect();
                    // the right copy of a `jux`'d part (its twin net, `melody_r`) is in its part: one MIDI track and one stem
                    let twins: Vec<String> = names.iter().map(|n| n.strip_suffix(djlive::compile::TWIN_SUFFIX).filter(|base| names.iter().any(|m| m == base)).unwrap_or(n).to_string()).collect();
                    self.gens.insert(key.clone(), twins);
                }
                if self.solo != Solo::All {
                    let names = &self.gens[&key];
                    let ids: BTreeMap<&str, &str> = p.graph.nets.iter().zip(names).map(|(n, g)| (n.id.as_str(), g.as_str())).collect();
                    let keep = |net: &str| match &self.solo {
                        Solo::All => true,
                        Solo::Deck(sd) => sd == d && net != "<live>",
                        Solo::Part(sd, role) => sd == d && ids.get(net).is_some_and(|g| g == role),
                        Solo::Wheel => false,
                    };
                    let kept: Vec<bool> = p.lanes().iter().map(|l| keep(&l.net_id)).collect();
                    for (lane, k) in p.lanes_mut().iter_mut().zip(kept) {
                        if !k && lane.level() != 0.0 {
                            lane.set_base_level(0.0);
                        }
                    }
                    // the riser (`.riser`) belongs to no part: it is in the master only
                    p.riser = None;
                }
            }
            if self.collect {
                core.runner.anchors.clear();
                for f in core.runner.fires.drain(..) {
                    if !(0..=127).contains(&f.note) {
                        continue;
                    }
                    let Some(part) = self.gens.get(&(d.clone(), f.generation)).and_then(|v| v.get(f.net.max(0) as usize)) else { continue };
                    let len_steps = if f.dur_steps > 0.0 { f64::from(f.dur_steps) } else { f64::from(f.dur_ms) / 1000.0 * self.bpm * 4.0 / 60.0 };
                    self.played.push(Played {
                        deck: d.clone(),
                        part: part.clone(),
                        step: (f.frame as f64 / self.spt).round() as i64,
                        note: f.note as u8,
                        velocity: f.velocity.min(127),
                        channel: f.channel.clamp(1, 16),
                        len_steps,
                        drum: f.flags.contains(FireFlags::DRUM),
                    });
                }
            }
        }
    }
}

impl Pass<'_> {
    /// The dry pass's deck as the voices make it (export.rs's module words): its strip at unity (volume 1, trim 1, the
    /// crossfader full on its side, the master level 1), its limiter out, its room and echo sends at 0. Set before every
    /// block, so a section's new project is dry from its first; the strip's knobs are sent only when they moved.
    fn dry_strip(&mut self, cond: &mut OfflineConductor) {
        let deck = match &self.solo {
            Solo::Part(d, _) | Solo::Deck(d) => d.clone(),
            Solo::All => return,
            Solo::Wheel => {
                // the Wheel joins the master after the decks: only the master level is on its way
                let e = cond.engine_mut();
                if e.master != 1.0 {
                    e.set_master(1.0);
                }
                return;
            }
        };
        let e = cond.engine_mut();
        if e.deck(&deck).is_some_and(|m| m.volume != 1.0) {
            e.set_volume(&deck, 1.0);
        }
        if e.deck(&deck).is_some_and(|m| m.gain != 0.5) {
            e.set_gain(&deck, 0.5); // trim_for_gain(0.5) = 1
        }
        let xf = match deck.as_str() {
            "A" => Some(0.0),
            "B" => Some(1.0),
            _ => None,
        };
        if let Some(x) = xf.filter(|x| e.crossfader != *x) {
            e.set_crossfader(x);
        }
        if e.master != 1.0 {
            e.set_master(1.0);
        }
        for d in &self.decks {
            if let Some(core) = cond.renderer.deck_mut(d) {
                core.limiter = None;
                core.runner.set_unit_sends(0.0, 0.0, 0.0, 1);
            }
        }
    }
}

impl Driver for Pass<'_> {
    fn ready(&mut self, cond: &mut OfflineConductor) -> Result<(), RenderError> {
        if self.solo != Solo::All {
            // the first section's projects installed now, so their lanes are muted before the first note
            cond.pump();
        }
        self.touch(cond);
        Ok(())
    }
    fn block(&mut self, _at: i64, l: &[f32], r: &[f32]) -> Result<(), RenderError> {
        if let Some(w) = self.out.as_deref_mut() {
            w.push(l, r)?;
        }
        Ok(())
    }
    fn after(&mut self, cond: &mut OfflineConductor, _produced: i64) {
        self.touch(cond);
    }
}

/// A program checked and planned for export.
struct Job {
    c: Compiled,
    plan: crate::render::Plan,
    sr: f64,
    want: i64,
    decks: Vec<String>,
}

fn job(text: &str, o: &ExportOptions) -> Result<Job, RenderError> {
    let env = BeatscriptEnv::new(Some(&beatscript_api::known::Known));
    let checked = djlive::compile::check_with(text, &env.env());
    let Some(c) = checked.compiled else {
        let errs: Vec<_> = checked.diagnostics.iter().filter(|d| d.is_error()).cloned().collect();
        let shown = djlive::diag::render_all(&errs, text);
        return Err(RenderError { code: "check_failed", message: format!("the program has {} error{}: {}", errs.len(), if errs.len() == 1 { "" } else { "s" }, shown.trim()) });
    };
    let ro = Options { sr: o.sr, max_bars: o.max_bars, ..Options::default() };
    let plan = plan(&c, &ro)?;
    let sr = f64::from(o.sr);
    let want = ((f64::from(plan.bars) * 4.0 * 60.0 / plan.bpm * sr).round() as i64).max(1);
    let mut decks: Vec<String> = Vec::new();
    let mut add = |d: &str| {
        if !decks.iter().any(|x| x == d) {
            decks.push(d.to_string());
        }
    };
    if c.arranged && !c.segments.is_empty() {
        for s in &c.segments {
            for d in s.decks.keys() {
                add(d.name());
            }
        }
    } else {
        for d in c.decks.keys() {
            add(d.name());
        }
    }
    decks.sort();
    Ok(Job { c, plan, sr, want, decks })
}

impl Job {
    fn pass(&self, solo: Solo, collect: bool, curve: Curve, out: Option<&mut WavOut>) -> Result<Vec<Played>, RenderError> {
        self.pass_with(solo, collect, curve, out, false)
    }

    /// [`pass`](Self::pass), dry or not: a dry pass hears only the moves that start and stop a deck ([`Controls::keep_transport`]).
    fn pass_with(&self, solo: Solo, collect: bool, curve: Curve, out: Option<&mut WavOut>, dry: bool) -> Result<Vec<Played>, RenderError> {
        let bpm = self.plan.bpm;
        let sr = self.sr;
        let mut ctls = Controls::new(&self.c, self.plan.program_bars, sr);
        if dry {
            ctls.keep_transport();
        }
        // the Wheel is in the master and in its own stem, and in no deck's or part's
        ctls.wheel = matches!(solo, Solo::All | Solo::Wheel);
        let mut p = Pass { decks: self.decks.clone(), solo, curve, io_err: None, collect, spt: sr * 60.0 / (bpm * 4.0), bpm, gens: BTreeMap::new(), played: Vec::new(), out, dry };
        drive(&self.c, &self.plan, sr, self.want, &mut ctls, &mut p)?;
        if let Curve::Record(files) = &mut p.curve {
            for w in files.values_mut() {
                w.flush()?;
            }
        }
        if let Some(e) = p.io_err {
            return Err(e.into());
        }
        Ok(p.played)
    }

    /// The parts, in the order the program says them (deck, then the deck's line), with what they played.
    fn parts(&self, played: &[Played]) -> Vec<Part> {
        let mut parts: Vec<Part> = Vec::new();
        let sets: Vec<&BTreeMap<djlive::ast::Deck, djlive::compile::DeckIr>> = if self.c.arranged && !self.c.segments.is_empty() { self.c.segments.iter().map(|s| &s.decks).collect() } else { vec![&self.c.decks] };
        for set in sets {
            for (d, ir) in set {
                for p in &ir.parts {
                    if !parts.iter().any(|x| x.deck == d.name() && x.role == p.role) {
                        parts.push(Part { deck: d.name().to_string(), role: p.role.clone(), instrument: p.instrument.clone(), drum: false, channel: 0, program: None, notes: 0 });
                    }
                }
            }
        }
        // a net the engine played that no line names (none known): a part of its own, at the end
        for n in played {
            if !parts.iter().any(|x| x.deck == n.deck && x.role == n.part) {
                parts.push(Part { deck: n.deck.clone(), role: n.part.clone(), instrument: None, drum: false, channel: 0, program: None, notes: 0 });
            }
        }
        for p in parts.iter_mut() {
            let mine = played.iter().filter(|n| n.deck == p.deck && n.part == p.role);
            p.notes = mine.clone().count();
            p.drum = mine.clone().any(|n| n.drum);
        }
        parts.sort_by_key(|p| p.deck.clone()); // stable: each deck's in its line's order
        parts.retain(|p| p.notes > 0);
        let mut next = 0u8;
        for p in parts.iter_mut() {
            if p.drum {
                p.channel = smf::DRUM_CHANNEL;
            } else {
                if next == smf::DRUM_CHANNEL {
                    next += 1;
                }
                p.channel = next % 16;
                next = (next + 1) % 16;
                p.program = p.instrument.as_deref().and_then(smf::gm_program);
            }
        }
        parts
    }

    fn midi(&self, parts: &[Part], played: &[Played], rights: &Rights) -> (Vec<u8>, bool) {
        let c = &self.c;
        let steps = i64::from(self.plan.bars) * 16;
        let tick = |step: f64| (step * f64::from(smf::STEP_TICKS)).round().max(0.0) as u32;
        let map = c.tempo_map();
        let curve = map.drives();
        let tempos: Vec<(u32, f64)> = if curve { map.staircase(steps as f64 / 4.0, 0.25).into_iter().map(|(beat, bpm)| (tick(beat * 4.0), bpm)).collect() } else { vec![(0, self.plan.bpm)] };
        let key = c.key.map(|k| {
            let iv = k.intervals();
            smf::key_sig(k.tonic, iv.contains(&3) && !iv.contains(&4))
        });
        let markers: Vec<(u32, String)> = if c.arranged { c.segments.iter().filter(|s| !s.chain && i64::from(s.start) * 16 < steps).map(|s| (tick(f64::from(s.start) * 16.0), s.name.clone())).collect() } else { Vec::new() };
        let tracks = parts
            .iter()
            .map(|p| smf::Track {
                name: p.title(),
                channel: p.channel,
                program: p.program,
                notes: played
                    .iter()
                    .filter(|n| n.deck == p.deck && n.part == p.role && n.step < steps)
                    .map(|n| smf::Note { tick: tick(n.step as f64), len: tick(n.len_steps).max(1), key: if p.drum { smf::gm_drum_note(&p.role, n.note) } else { n.note }, velocity: n.velocity })
                    .collect(),
            })
            .collect();
        let song = smf::Song { name: song_name(c, rights), copyright: Some(copyright(c, rights)), tempos, key, markers, tracks, end: tick(steps as f64) };
        (smf::write(&song), curve)
    }

    fn exported(&self, parts: Vec<Part>, played: Vec<Played>, midi: Vec<u8>, curve: bool, rights: &Rights) -> Exported {
        let c = &self.c;
        let mut notes = self.plan.notes.clone();
        if curve {
            notes.push("the tempo moves: the MIDI file carries its curve, the audio is rendered at the opening tempo".into());
        }
        // the master's own lines (`master: filter ...`, timed or not), as the program says them
        let master_fx: Vec<String> = c.controls.iter().filter_map(|ctl| c.source.get(ctl.span.start..ctl.span.end)).map(|t| t.trim().to_string()).filter(|t| t.starts_with("master") || t.contains(": master")).collect();
        Exported {
            cid: c.cid.clone(),
            name: song_name(c, rights),
            bpm: self.plan.bpm,
            key: c.key.map(|k| k.text()),
            bars: self.plan.bars,
            frames: self.want as u64,
            sr: self.sr as u32,
            parts,
            decks: self.decks.clone(),
            played,
            midi,
            files: Vec::new(),
            parts_sum: None,
            decks_sum: None,
            master_fx,
            notes,
            rights: resolved_rights(c, rights),
            tempo_curve: curve,
            codec: "wav",
            dry: None,
        }
    }

    /// The control lines a dry stem is rendered without: each line that made a move the dry pass takes out (its moves, where
    /// they fire), and each line that is not a move on the timeline at all (a DJ move, a line the render plays another way).
    /// A line that only starts or stops a deck, or plays the Wheel, is not one: the dry stems keep it.
    fn automation(&self) -> Vec<Automation> {
        let c = &self.c;
        let removed = Controls::new(c, self.plan.program_bars, self.sr).keep_transport();
        let mut out: Vec<Automation> = Vec::new();
        let mut seen: BTreeMap<usize, usize> = BTreeMap::new();
        for ctl in &c.controls {
            let base = ctl.effect.base();
            if let djlive::compile::Effect::Calls(cs) = base {
                // a deck's start and stop, and the Wheel (an instrument, with its own stem): the dry stems keep them
                if !cs.is_empty() && cs.iter().all(|x| x.noun == "deck" || x.noun == "wheel") {
                    continue;
                }
            }
            let own = ctl.span.text(&c.source).lines().next().unwrap_or("").trim().to_string();
            let line = djlive::diag::line_col(&c.source, ctl.span.start).0;
            let text = if own.is_empty() || own.starts_with("import") { ctl.key.clone() } else { own };
            let moves: Vec<crate::render::RemovedMove> = removed.iter().filter(|m| m.key == ctl.key).cloned().collect();
            let fires: Vec<f64> = djhost::beatscript_render::fire_pos(ctl.timing, self.plan.program_bars).into_iter().map(|r| r.to_f64() * 4.0).filter(|b| *b < f64::from(self.plan.bars) * 4.0).collect();
            let (scope, deck) = scope_of(&text, &moves);
            let a = Automation { line, text, scope, deck, fires, hold_bars: ctl.hold.map(|h| h.as_bars().to_f64()), moves };
            // a line a `repeat` or a `def` copies: one entry, its fires and moves together
            match seen.get(&a.line).copied().filter(|&i| out[i].text == a.text) {
                Some(i) => {
                    out[i].fires.extend(a.fires);
                    out[i].moves.extend(a.moves);
                }
                None => {
                    seen.insert(a.line, out.len());
                    out.push(a);
                }
            }
        }
        for a in out.iter_mut() {
            a.fires.sort_by(f64::total_cmp);
            a.fires.dedup();
            a.moves.sort_by(|x, y| x.beat.total_cmp(&y.beat));
        }
        out
    }
}

/// Whom a control line acts on: the targets of its moves (a deck, the master), else its text (`master: ...`, `deck A: ...`,
/// `... on A`).
fn scope_of(text: &str, moves: &[crate::render::RemovedMove]) -> (&'static str, Option<String>) {
    let mut decks: Vec<String> = Vec::new();
    let mut master = false;
    for m in moves {
        let t = m.target.as_str();
        if t.eq_ignore_ascii_case("master") || (t.is_empty() && m.param != "crossfader") {
            master = true;
        } else if !t.is_empty() && !decks.iter().any(|d| d == t) {
            decks.push(t.to_string());
        }
    }
    if decks.len() == 1 && !master {
        return ("deck", decks.pop());
    }
    if master && decks.is_empty() {
        return ("master", None);
    }
    if !moves.is_empty() {
        return ("other", None);
    }
    // no move to read it from: the text
    let words: Vec<&str> = text.split(|ch: char| ch.is_whitespace() || ch == ':').filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| *w == "master") {
        return ("master", None);
    }
    for (i, w) in words.iter().enumerate() {
        if (*w == "deck" || *w == "on") && i + 1 < words.len() {
            let d = words[i + 1];
            if d.len() <= 2 && d.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()) && d.chars().next().is_some_and(|ch| ch.is_ascii_uppercase()) {
                return ("deck", Some(d.to_string()));
            }
        }
    }
    ("other", None)
}

fn header(c: &Compiled, key: &str) -> Option<String> {
    c.source.lines().take_while(|l| l.starts_with("#!")).find_map(|l| {
        let (k, v) = l.trim_start_matches("#!").split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

fn song_name(c: &Compiled, r: &Rights) -> String {
    r.name.clone().filter(|n| !n.is_empty()).or_else(|| header(c, "name")).unwrap_or_else(|| format!("BeatScript {}", &c.cid[..c.cid.len().min(12)]))
}

fn resolved_rights(c: &Compiled, r: &Rights) -> Rights {
    let mut out = r.clone();
    if out.license.as_deref().map_or(true, str::is_empty) {
        out.license = Some(header(c, "license").unwrap_or_else(|| "CC0-1.0".into()));
    }
    out.name = Some(song_name(c, r));
    out
}

fn copyright(c: &Compiled, r: &Rights) -> String {
    let r = resolved_rights(c, r);
    let mut s = format!("License: {}", r.license.unwrap_or_default());
    if let Some(a) = r.author.filter(|a| !a.is_empty()) {
        s = format!("{a}. {s}");
    }
    s
}

// ---------------------------------------------------------------------------------------------
// the entry points

/// The Standard MIDI File of `text`: one render, its notes.
pub fn midi(text: &str, o: &ExportOptions) -> Result<Exported, RenderError> {
    let j = job(text, o)?;
    let played = j.pass(Solo::All, true, Curve::Off, None)?;
    let parts = j.parts(&played);
    let (midi, curve) = j.midi(&parts, &played, &o.rights);
    let mut e = j.exported(parts, played, midi, curve, &o.rights);
    e.files.push(("song.mid".into(), "midi"));
    Ok(e)
}

/// The stems of `text` into `dir`: `master.wav`, a WAV per deck (`deck-A.wav`) and per part (`A-kick.wav`),
/// `song.mid` and `manifest.json`. `progress` hears each file as it starts (for a status line).
pub fn stems(text: &str, o: &ExportOptions, dir: &Path, progress: &mut (dyn FnMut(&str) + Send)) -> Result<Exported, RenderError> {
    let j = job(text, o)?;
    std::fs::create_dir_all(dir)?;
    progress("master.wav");
    let mut master = WavOut::create(&dir.join("master.wav"), o.sr)?;
    let curve_file = |d: &str| dir.join(format!(".curve-{d}.f32"));
    // the master is the mix either way; only the wet parts replay its limiter
    let master_curve = if o.dry {
        Curve::Off
    } else {
        let mut rec = BTreeMap::new();
        for d in &j.decks {
            rec.insert(d.clone(), BufWriter::new(File::create(curve_file(d))?));
        }
        Curve::Record(rec)
    };
    let played = j.pass(Solo::All, true, master_curve, Some(&mut master))?;
    master.finish(o.sr)?;
    let parts = j.parts(&played);
    let (midi, curve) = j.midi(&parts, &played, &o.rights);
    let mut e = j.exported(parts, played, midi, curve, &o.rights);
    e.files.push(("master.wav".into(), "master"));
    // the decks' and the parts' renders: independent of each other, so side by side on `threads` (dry: the parts only; a dry
    // deck would be the sum of its dry parts)
    let mut jobs: Vec<(String, &'static str, Solo)> = if o.dry { Vec::new() } else { j.decks.iter().map(|d| (format!("deck-{d}.wav"), "deck", Solo::Deck(d.clone()))).collect() };
    jobs.extend(e.parts.iter().map(|p| (p.file(), "part", Solo::Part(p.deck.clone(), p.role.clone()))));
    // a program that plays the Wheel: its own stem (it is no deck's: the page adds it to the master after the decks)
    if plays_wheel(&j.c) {
        jobs.push(("wheel.wav".into(), "wheel", Solo::Wheel));
    }
    let run = |(f, _, solo): &(String, &'static str, Solo)| -> Result<(), RenderError> {
        let mut w = WavOut::create(&dir.join(f), o.sr)?;
        if o.dry {
            j.pass_with(solo.clone(), false, Curve::Off, Some(&mut w), true)?;
            w.finish(o.sr)?;
            return Ok(());
        }
        let curve = match solo {
            // the deck's own runner plays its whole mix: its limiter is the master's as it is
            Solo::Deck(_) | Solo::All | Solo::Wheel => Curve::Off,
            Solo::Part(..) => {
                let mut rep = BTreeMap::new();
                for d in &j.decks {
                    rep.insert(d.clone(), BufReader::new(File::open(curve_file(d))?));
                }
                Curve::Replay(rep)
            }
        };
        j.pass(solo.clone(), false, curve, Some(&mut w))?;
        w.finish(o.sr)?;
        Ok(())
    };
    let threads = o.threads.clamp(1, 16).min(jobs.len().max(1));
    if threads == 1 {
        for jb in &jobs {
            progress(&jb.0);
            run(jb)?;
        }
    } else {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let said = std::sync::Mutex::new(&mut *progress);
        let results: Vec<Result<(), RenderError>> = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    sc.spawn(|| -> Result<(), RenderError> {
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(jb) = jobs.get(k) else { return Ok(()) };
                            (said.lock().unwrap())(&jb.0);
                            run(jb)?;
                        }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(RenderError { code: "internal", message: "a stem's render panicked".into() }))).collect()
        });
        for r in results {
            r?;
        }
    }
    let mut deck_files = Vec::new();
    let mut part_files = Vec::new();
    for (f, kind, _) in &jobs {
        e.files.push((f.clone(), kind));
        match *kind {
            "deck" => deck_files.push(dir.join(f)),
            // the Wheel is in both sums: the decks and the parts add up to the master with it
            "wheel" => {
                deck_files.push(dir.join(f));
                part_files.push(dir.join(f));
            }
            _ => part_files.push(dir.join(f)),
        }
    }
    for d in &j.decks {
        let _ = std::fs::remove_file(curve_file(d));
    }
    std::fs::write(dir.join("song.mid"), &e.midi)?;
    e.files.push(("song.mid".into(), "midi"));
    if o.dry {
        // dry parts do not add up to the master (that is the point): no sum check; what they leave out is in the manifest
        e.dry = Some(j.automation());
        e.notes.push("dry stems: each part before its deck and the master (no level, crossfader, EQ, filter, insert, send, trick, move or master effect, the deck's limiter off); master.wav is the mix as it plays, and `automation` in manifest.json is every control move the parts were rendered without".into());
        std::fs::write(dir.join("manifest.json"), manifest(&e))?;
        e.files.push(("manifest.json".into(), "manifest"));
        return Ok(e);
    }
    e.parts_sum = Some(sum_check(&dir.join("master.wav"), &part_files)?);
    e.decks_sum = Some(sum_check(&dir.join("master.wav"), &deck_files)?);
    if let Some(s) = e.parts_sum.filter(|s| s.peak > 1.0) {
        e.notes.push(format!("the master peaks at {:.1} dBFS, over full scale: the float WAVs keep every sample whole; turn the master down in the DAW rather than clip it", 20.0 * s.peak.log10()));
    }
    std::fs::write(dir.join("manifest.json"), manifest(&e))?;
    e.files.push(("manifest.json".into(), "manifest"));
    Ok(e)
}

// ---------------------------------------------------------------------------------------------
// the manifest

fn n3(x: f64) -> JsonValue {
    JsonValue::Number((x * 1000.0).round() / 1000.0)
}

fn opt_str(v: &Option<String>) -> JsonValue {
    v.as_ref().map_or(JsonValue::Null, |s| JsonValue::str(s.clone()))
}

/// `manifest.json`: what the bundle is, for a person and a DAW script. Deterministic (no clock, fixed key order).
pub fn manifest(e: &Exported) -> String {
    let sum = |s: &Option<SumCheck>| {
        s.map_or(JsonValue::Null, |s| {
            let db = |x: f64| if x > 0.0 && s.peak > 0.0 { n3(20.0 * (x / s.peak).log10()) } else { JsonValue::Null };
            JsonValue::Object(JsonObject::from_pairs(vec![("peak", n3(s.peak)), ("max_abs", JsonValue::Number(s.max_abs)), ("rms", JsonValue::Number(s.rms)), ("max_abs_db", db(s.max_abs)), ("rms_db", db(s.rms))]))
        })
    };
    let parts = e
        .parts
        .iter()
        .map(|p| {
            let mut pairs = vec![
                ("deck", JsonValue::str(p.deck.clone())),
                ("role", JsonValue::str(p.role.clone())),
                ("file", JsonValue::str(if e.codec == "wav" { p.file() } else { p.file().replace(".wav", &format!(".{}", e.codec)) })),
                ("midi_track", JsonValue::str(p.title())),
                ("midi_channel", JsonValue::int(i64::from(p.channel) + 1)),
                ("drums", JsonValue::Bool(p.drum)),
                ("instrument", opt_str(&p.instrument)),
                ("gm_program", p.program.map_or(JsonValue::Null, |g| JsonValue::int(i64::from(g) + 1))),
                ("notes", JsonValue::int(p.notes as i64)),
            ];
            if let Some(auto) = &e.dry {
                // the deck's lines: what its strip, inserts and moves did to this part in the mix
                pairs.push(("automation", JsonValue::Array(auto.iter().filter(|a| a.scope == "deck" && a.deck.as_deref() == Some(p.deck.as_str())).map(automation_json).collect())));
            }
            JsonValue::Object(JsonObject::from_pairs(pairs))
        })
        .collect();
    let files = e.files.iter().map(|(f, k)| JsonValue::Object(JsonObject::from_pairs(vec![("path", JsonValue::str(f.clone())), ("kind", JsonValue::str(*k))]))).collect();
    let r = &e.rights;
    let stems = match &e.dry {
        None => JsonValue::Object(JsonObject::from_pairs(vec![
            ("method", JsonValue::str("each stem is the render with every other part's lanes at level 0: ducking, modulation and the deck's own effects still run; a program's Wheel is in the master and in wheel.wav only")),
            ("master_fx", JsonValue::str("applied per stem: the master chain (and each deck's output stage) is applied to every stem as to the mix")),
            ("master_fx_lines", JsonValue::Array(e.master_fx.iter().cloned().map(JsonValue::str).collect())),
            ("sum_check_on", JsonValue::str("the 32-bit float WAVs, before any encoding")),
            ("parts_sum_check", sum(&e.parts_sum)),
            ("decks_sum_check", sum(&e.decks_sum)),
        ])),
        Some(_) => JsonValue::Object(JsonObject::from_pairs(vec![
            ("dry", JsonValue::Bool(true)),
            ("method", JsonValue::str("dry: each stem is its part's voices alone, before the deck and the master: the deck's strip at unity (volume, trim, crossfader, EQ and filter untouched by any line), its limiter off, its room and echo sends at 0, and no control move but the ones that start and stop a deck. The sections still load and start the decks and every other part's lanes still run at level 0, so each part plays every note it plays in the mix, and a kick still ducks the bass under a duck depth the program gives its voices")),
            ("master_fx", JsonValue::str("not applied: the master's lines are master_fx_lines, their moves master_automation")),
            ("master_fx_lines", JsonValue::Array({
                // each line once (a `repeat` or a `def` makes many of one), in the program's order
                let mut seen = std::collections::BTreeSet::new();
                e.master_fx.iter().filter(|l| seen.insert(l.as_str())).cloned().map(JsonValue::str).collect()
            })),
            ("automation", JsonValue::str("each part's `automation` is its deck's lines (levels, EQ, filter, resonance, inserts, sends, tricks, moves) with the moves they made in the mix; master_automation is the master's, other_automation the rest (the crossfader, a line on no one deck); the Wheel is an instrument, in wheel.wav, and its lines are not here. `at` and `fires` are beats from the start, `bar` and `beat` 1-based; `points` are [at, value] pairs of one knob")),
            ("parts_sum_check", JsonValue::Null),
        ])),
    };
    let lines = |scope: &str| JsonValue::Array(e.dry.iter().flatten().filter(|a| a.scope == scope).map(automation_json).collect());
    let mut o = JsonObject::from_pairs(vec![
        ("generator", JsonValue::str("BeatScript")),
        ("cid", JsonValue::str(e.cid.clone())),
        ("name", JsonValue::str(e.name.clone())),
        ("tempo", n3(e.bpm)),
        ("tempo_curve", JsonValue::Bool(e.tempo_curve)),
        ("key", opt_str(&e.key)),
        ("time_signature", JsonValue::str("4/4")),
        ("bars", JsonValue::int(i64::from(e.bars))),
        ("seconds", n3(e.frames as f64 / f64::from(e.sr))),
        ("sample_rate", JsonValue::int(i64::from(e.sr))),
        ("codec", JsonValue::str(e.codec)),
        ("bits", if e.codec == "wav" { JsonValue::int(i64::from(WAV_BITS)) } else { JsonValue::Null }),
        ("license", opt_str(&r.license)),
        ("author", opt_str(&r.author)),
        ("forked_from", opt_str(&r.forked_from)),
        ("credits", JsonValue::Array(r.credits.iter().cloned().map(JsonValue::str).collect())),
        ("decks", JsonValue::Array(e.decks.iter().cloned().map(JsonValue::str).collect())),
        ("parts", JsonValue::Array(parts)),
        ("files", JsonValue::Array(files)),
        ("stems", stems),
        ("notes", JsonValue::Array(e.notes.iter().cloned().map(JsonValue::str).collect())),
    ]);
    if e.dry.is_some() {
        o.set("master_automation", lines("master"));
        o.set("other_automation", lines("other"));
    }
    JsonValue::Object(o).js_stringified() + "\n"
}

/// `(bar, beat)`, both 1-based, of `at` beats from the start.
fn bar_beat(at: f64) -> (i64, f64) {
    let bar = (at / 4.0 + 1e-9).floor();
    (bar as i64 + 1, ((at - bar * 4.0 + 1.0) * 1000.0).round() / 1000.0)
}

/// One line of a dry export's automation: the line, whom it acts on, where it fires, and its moves: a knob's values as `points`
/// (one series a knob, in time order), anything else (an insert or a trick switched, a macro) as `events`.
fn automation_json(a: &Automation) -> JsonValue {
    let mut series: Vec<((String, String), Vec<JsonValue>)> = Vec::new();
    let mut events = Vec::new();
    for m in &a.moves {
        let switch = m.param.ends_with(" on") || m.param == "mute" || m.param.starts_with("trick ") || m.value.is_none();
        if switch {
            let (bar, beat) = bar_beat(m.beat);
            let mut ev = vec![("at", n3(m.beat)), ("bar", JsonValue::int(bar)), ("beat", JsonValue::Number(beat)), ("target", JsonValue::str(m.target.clone())), ("param", JsonValue::str(m.param.clone()))];
            if let Some(v) = m.value {
                ev.push(("value", n3(v)));
            }
            ev.push(("move", JsonValue::str(m.text.clone())));
            events.push(JsonValue::Object(JsonObject::from_pairs(ev)));
            continue;
        }
        let k = (m.target.clone(), m.param.clone());
        let pt = JsonValue::Array(vec![n3(m.beat), JsonValue::Number(m.value.map_or(0.0, |v| (v * 1e6).round() / 1e6))]);
        match series.iter_mut().find(|(key, _)| *key == k) {
            Some((_, v)) => v.push(pt),
            None => series.push((k, vec![pt])),
        }
    }
    let series: Vec<JsonValue> = series.into_iter().map(|((t, p), pts)| JsonValue::Object(JsonObject::from_pairs(vec![("target", JsonValue::str(t)), ("param", JsonValue::str(p)), ("points", JsonValue::Array(pts))]))).collect();
    JsonValue::Object(JsonObject::from_pairs(vec![
        ("line", JsonValue::int(a.line as i64)),
        ("text", JsonValue::str(a.text.clone())),
        ("scope", JsonValue::str(a.scope)),
        ("deck", opt_str(&a.deck)),
        ("fires", JsonValue::Array(a.fires.iter().map(|b| n3(*b)).collect())),
        ("hold_bars", a.hold_bars.map_or(JsonValue::Null, n3)),
        ("series", JsonValue::Array(series)),
        ("events", JsonValue::Array(events)),
        (
            "note",
            if a.moves.is_empty() { JsonValue::str("no move on the render's timeline the dry pass takes out (a DJ move): left out of the dry stems") } else { JsonValue::Null },
        ),
    ]))
}

// ---------------------------------------------------------------------------------------------
// WAV

/// A 32-bit float stereo WAV (`WAVE_FORMAT_IEEE_FLOAT`, with its `fact` chunk) written as it renders; the sizes
/// are patched in at the end.
pub struct WavOut {
    f: BufWriter<File>,
    frames: u64,
}

fn wav_header(sr: u32, frames: u32) -> Vec<u8> {
    let ch = 2u16;
    let block = ch * WAV_BITS / 8;
    let data = frames.saturating_mul(u32::from(block));
    let mut h = Vec::with_capacity(WAV_HEADER as usize);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(WAV_HEADER as u32 - 8).saturating_add(data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&18u32.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    h.extend_from_slice(&ch.to_le_bytes());
    h.extend_from_slice(&sr.to_le_bytes());
    h.extend_from_slice(&(sr * u32::from(block)).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&WAV_BITS.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    h.extend_from_slice(b"fact");
    h.extend_from_slice(&4u32.to_le_bytes());
    h.extend_from_slice(&frames.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

impl WavOut {
    pub fn create(path: &Path, sr: u32) -> std::io::Result<WavOut> {
        let mut f = BufWriter::new(File::create(path)?);
        f.write_all(&wav_header(sr, 0))?;
        Ok(WavOut { f, frames: 0 })
    }
    pub fn push(&mut self, l: &[f32], r: &[f32]) -> std::io::Result<()> {
        let mut b = Vec::with_capacity(l.len() * 8);
        for (a, c) in l.iter().zip(r) {
            b.extend_from_slice(&a.to_le_bytes());
            b.extend_from_slice(&c.to_le_bytes());
        }
        self.frames += l.len() as u64;
        self.f.write_all(&b)
    }
    pub fn finish(mut self, sr: u32) -> std::io::Result<()> {
        let frames = self.frames.min(u64::from(u32::MAX / 8 - 64)) as u32;
        self.f.seek(SeekFrom::Start(0))?;
        self.f.write_all(&wav_header(sr, frames))?;
        self.f.flush()
    }
}

/// Reads the samples of a WAV [`WavOut`] wrote, interleaved.
pub struct WavIn {
    r: BufReader<File>,
    left: u64,
}

impl WavIn {
    pub fn open(path: &Path) -> std::io::Result<WavIn> {
        let mut r = BufReader::new(File::open(path)?);
        let mut h = [0u8; WAV_HEADER as usize];
        r.read_exact(&mut h)?;
        let n = WAV_HEADER as usize;
        let data = u32::from_le_bytes([h[n - 4], h[n - 3], h[n - 2], h[n - 1]]);
        Ok(WavIn { r, left: u64::from(data) / 4 })
    }
    #[allow(dead_code)] // beatscript-serve
    pub fn sample_rate(path: &Path) -> std::io::Result<u32> {
        let mut h = [0u8; 28];
        File::open(path)?.read_exact(&mut h)?;
        Ok(u32::from_le_bytes([h[24], h[25], h[26], h[27]]))
    }
    /// Up to `n` samples (fewer at the end; empty when done).
    pub fn read(&mut self, n: usize) -> std::io::Result<Vec<f32>> {
        let k = (n as u64).min(self.left) as usize;
        let mut b = vec![0u8; k * 4];
        self.r.read_exact(&mut b)?;
        self.left -= k as u64;
        Ok(b.chunks_exact(4).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]])).collect())
    }
}

/// The sum of `stems` against `master`, sample by sample, on the files.
pub fn sum_check(master: &Path, stems: &[PathBuf]) -> std::io::Result<SumCheck> {
    let mut m = WavIn::open(master)?;
    let mut ins: Vec<WavIn> = stems.iter().map(|p| WavIn::open(p)).collect::<Result<_, _>>()?;
    let (mut peak, mut max_abs, mut sq, mut n) = (0f64, 0f64, 0f64, 0u64);
    loop {
        let a = m.read(1 << 16)?;
        if a.is_empty() {
            break;
        }
        let mut s = vec![0f64; a.len()];
        for w in ins.iter_mut() {
            for (i, x) in w.read(a.len())?.into_iter().enumerate() {
                s[i] += f64::from(x);
            }
        }
        for (x, y) in a.iter().zip(&s) {
            let x = f64::from(*x);
            peak = peak.max(x.abs());
            let d = (y - x).abs();
            max_abs = max_abs.max(d);
            sq += d * d;
        }
        n += a.len() as u64;
    }
    Ok(SumCheck { peak, max_abs, rms: if n > 0 { (sq / n as f64).sqrt() } else { 0.0 } })
}

// ---------------------------------------------------------------------------------------------
// zip

fn crc32(data: &[u8], mut c: u32) -> u32 {
    c = !c;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// A zip of `files` (relative to `dir`, in this order), stored (WAV and MIDI hardly compress), every entry dated
/// 1980-01-01 so the bytes have no clock in them. `wrap` names a folder the entries go in ("" for none).
pub fn zip(dir: &Path, files: &[String], wrap: &str, out: &mut dyn Write) -> std::io::Result<u64> {
    const DATE: u16 = 0x0021; // 1980-01-01
    let mut central = Vec::new();
    let mut off: u64 = 0;
    for f in files {
        let name = if wrap.is_empty() { f.clone() } else { format!("{wrap}/{f}") };
        let path = dir.join(f);
        // the CRC first (a second read: entries can be large, and nothing is held in memory)
        let mut crc = 0u32;
        let mut size = 0u64;
        {
            let mut r = BufReader::new(File::open(&path)?);
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let k = r.read(&mut buf)?;
                if k == 0 {
                    break;
                }
                crc = crc32(&buf[..k], crc);
                size += k as u64;
            }
        }
        if size > u64::from(u32::MAX) || off > u64::from(u32::MAX) {
            return Err(std::io::Error::other("a stem is over 4 GiB: too long a program for a zip"));
        }
        let mut h = Vec::new();
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes()); // version needed
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        h.extend_from_slice(&0u16.to_le_bytes()); // stored
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&DATE.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(size as u32).to_le_bytes());
        h.extend_from_slice(&(size as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        out.write_all(&h)?;
        std::io::copy(&mut BufReader::new(File::open(&path)?), out)?;
        let mut c = Vec::new();
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // made by
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&DATE.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&(size as u32).to_le_bytes());
        c.extend_from_slice(&(size as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        c.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        c.extend_from_slice(&(off as u32).to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&c);
        off += h.len() as u64 + size;
    }
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0; 4]);
    end.extend_from_slice(&(files.len() as u16).to_le_bytes());
    end.extend_from_slice(&(files.len() as u16).to_le_bytes());
    end.extend_from_slice(&(central.len() as u32).to_le_bytes());
    end.extend_from_slice(&(off as u32).to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&central)?;
    out.write_all(&end)?;
    Ok(off + central.len() as u64 + end.len() as u64)
}

/// Whether `c` has a `wheel` line: its stems then have the Wheel's own (`wheel.wav`).
fn plays_wheel(c: &Compiled) -> bool {
    c.controls.iter().any(|ctl| match ctl.effect.base() {
        djlive::compile::Effect::Calls(cs) => cs.iter().any(|x| x.noun == "wheel"),
        _ => false,
    })
}

/// The folder a bundle's zip puts its files in: `beatscript-<CID12>`, `beatscript-<CID12>-dry` for dry stems (both can sit side by
/// side once unzipped).
pub fn bundle_folder(e: &Exported) -> String {
    let base = format!("beatscript-{}", &e.cid[..e.cid.len().min(12)]);
    if e.dry.is_some() {
        base + "-dry"
    } else {
        base
    }
}

/// The bundle's files in zip order: the manifest and the MIDI first (small: a reader sees what it is at once), then
/// the master, the decks, the parts.
pub fn bundle_order(e: &Exported) -> Vec<String> {
    let rank = |k: &str| match k {
        "manifest" => 0,
        "midi" => 1,
        "master" => 2,
        "deck" => 3,
        "wheel" => 4,
        _ => 5,
    };
    let mut v: Vec<(usize, usize, String)> = e.files.iter().enumerate().map(|(i, (f, k))| (rank(k), i, f.clone())).collect();
    v.sort();
    v.into_iter().map(|(_, _, f)| f).collect()
}

// ---------------------------------------------------------------------------------------------
// the command

/// `beatscript export --format midi|stems [--dry] [--sr N] [--max-bars N] [--file F | -]`: the .mid, or the zip of stems (`--dry`:
/// dry stems), on stdout.
pub fn main(args: Vec<String>) -> i32 {
    let run = || -> Result<(), RenderError> {
        let usage = |m: String| RenderError { code: "usage", message: m };
        let mut o = ExportOptions { threads: std::thread::available_parallelism().map_or(1, |n| n.get()).min(8), ..ExportOptions::default() };
        let (mut format, mut file) = (String::from("midi"), None::<String>);
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = |n: &str| it.next().cloned().ok_or_else(|| usage(format!("{n} needs a value")));
            match a.as_str() {
                "--format" => format = val("--format")?,
                "--file" | "-f" => file = Some(val("--file")?),
                "--sr" => o.sr = val("--sr")?.parse().map_err(|_| usage("--sr: a whole number".into()))?,
                "--threads" => o.threads = val("--threads")?.parse().map_err(|_| usage("--threads: a whole number".into()))?,
                "--max-bars" => o.max_bars = val("--max-bars")?.parse().map_err(|_| usage("--max-bars: a whole number".into()))?,
                "--license" => o.rights.license = Some(val("--license")?),
                "--credit" => o.rights.credits.push(val("--credit")?),
                "--dry" => o.dry = true,
                "-" => file = None,
                other => return Err(usage(format!("unknown argument `{other}` (beatscript export --format midi|stems [--dry] [--sr N] [--max-bars N] [--license L] [--credit C]... [--file F])"))),
            }
        }
        let mut text = String::new();
        match file.as_deref() {
            Some(p) if p != "-" => text = std::fs::read_to_string(p).map_err(|e| RenderError { code: "io", message: format!("{p}: {e}") })?,
            _ => {
                std::io::stdin().read_to_string(&mut text)?;
            }
        }
        let mut out = std::io::stdout().lock();
        match format.as_str() {
            "midi" => {
                let e = midi(&text, &o)?;
                out.write_all(&e.midi)?;
            }
            "stems" => {
                let dir = std::env::temp_dir().join(format!("beatscript-export-{}", std::process::id()));
                let r = stems(&text, &o, &dir, &mut |f| eprintln!("{{\"event\":\"file\",\"name\":\"{f}\"}}"));
                let z = r.and_then(|e| zip(&dir, &bundle_order(&e), &bundle_folder(&e), &mut out).map_err(RenderError::from));
                let _ = std::fs::remove_dir_all(&dir);
                z?;
            }
            f => return Err(usage(format!("--format {f}: midi or stems"))),
        }
        out.flush()?;
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) if e.code == "closed" => 0,
        Err(e) => {
            eprintln!("{{\"event\":\"error\",\"code\":\"{}\",\"message\":{:?}}}", e.code, e.message);
            1
        }
    }
}
