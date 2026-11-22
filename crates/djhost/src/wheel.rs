//! The Wheel (djapp's Wheel window, djandroid's Wheel tab): windwheel's linear tonewheel (stackdump/windwheel-bitwrap-io, `windwheel::live`),
//! magnets driven past a patterned coil, played into the mix.
//!
//! The sound goes through an input strip of the engine's own, "Windwheel" (djhost input strips:
//! ring → DC blocker → trim → EQ → fader → master, the cue with PFL), so the wheel plays with
//! the decks, goes out to HEOS with the master, and can be the vocoder's modulator. A feeder
//! thread renders the voice into the strip's ring and keeps it a cushion past the strip's start
//! threshold or the output's block, whichever is bigger (an Android callback can be longer than
//! the threshold: kept at the threshold, the ring ran dry every block and the wheel played
//! gated). The ring's fill is the clock, so the two never drift apart.
//!
//! The strip's knobs stay at a fresh strip's, which keeps the wheel out of saved mixes (a
//! snapshot records only the knobs that differ). The voice's level is its own, and the feeder
//! undoes the fresh strip's trim and fader ([`strip_makeup`]) so the voice's output stage is
//! what the master hears.
//!
//! Threads: the feeder holds the wheel's lock for one chunk at a time; a key or tone change
//! re-winds the instrument (inverse design, a few hundred ms) with no lock held and swaps it in.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::deck::input::InputMirror;
use windwheel::live::{Instrument, Meters, Voice};

pub mod draw;
pub mod memory;

/// The engine's input strip for the wheel.
pub const INPUT_LABEL: &str = crate::insert_view::WHEEL_STRIP;
/// Frames kept in the ring past the strip's start threshold or the output's block (~10 ms at
/// 48 kHz); another this many after each underrun, up to [`MAX_CUSHION`] of them.
const CUSHION: usize = 512;
const MAX_CUSHION: usize = 6;
/// The most the feeder renders per lock.
const CHUNK: usize = 256;

/// One instrument of the library, as the phone lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The motion law: drive, motor, bow, gravity, pendulum, spring, rail.
    pub law: String,
    /// The path: hoop, drum, loop, tube, rack, rail, incline, arc.
    pub path: String,
}

/// One drawable segment of the path, in model metres (y down), with each point's z.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelShape {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub zs: Vec<f64>,
    pub closed: bool,
    pub start_m: f64,
    pub length_m: f64,
    pub first_section: u32,
    pub sections: u32,
}

/// A note wound on the path.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelNote {
    pub segment: u32,
    pub start_section: u32,
    pub count: u32,
    /// "E3", "G#4+12c".
    pub name: String,
    pub freq_hz: f64,
    /// Within this coil's reach at the speed it's played at.
    pub reachable: bool,
    pub reason: Option<String>,
}

/// The instrument loaded: what the tab draws and its controls start from.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelLayout {
    pub id: String,
    pub name: String,
    pub description: String,
    pub law: String,
    pub path: String,
    /// The magnets share one cord (drive, motor, bow); else each moves on its own.
    pub cord: bool,
    pub periodic: bool,
    pub extent_m: f64,
    pub section_length_m: f64,
    pub shapes: Vec<WheelShape>,
    /// The pendulum's pivot, the tube's anchor ([x, y]).
    pub pivot: Option<Vec<f64>>,
    pub anchor: Option<Vec<f64>>,
    /// Signed turns per section.
    pub turns: Vec<i32>,
    pub notes: Vec<WheelNote>,
    pub magnets: u32,
    pub root: String,
    pub transpose: i32,
    /// The note sequence as typed ("" when the notes are the preset's own).
    pub sequence: String,
    pub brightness: f64,
    pub volume_db: f64,
    /// The trigger's pull (0 open … ½ matched … 1 short); None: the fixed load.
    pub trigger: Option<f64>,
    pub load_ohms: f64,
    pub coil_ohms: f64,
    /// The motor runs (drum, bow); the speed it runs at, and the speed notes are wound for.
    pub motor: bool,
    pub speed_mps: f64,
    /// Gravity: a magnet drops by itself every this many seconds (0: by hand only).
    pub interval_s: f64,
    /// The rail's ends.
    pub lo_z: f64,
    pub hi_z: f64,
    pub buildable: bool,
    /// Notes and patterns this coil can't reach in this key.
    pub out_of_reach: u32,
    pub coil_radius_mm: f64,
    pub magnet_mm: f64,
    /// The build knobs (the web toy's): the carrier's own mass (g), the coil's Lenz drag on the
    /// motion, the drop height / pluck size (m; see `release_kind`), the notes' timbre and decay.
    pub carrier_g: f64,
    pub braking: bool,
    pub release_m: f64,
    /// "drop" (tubes, slope: height above the coil), "pluck" (spring: its amplitude), "" (none).
    pub release_kind: String,
    pub timbre: String,
    pub decay_ms: f64,
    /// The timbres to pick from.
    pub timbres: Vec<String>,
    /// Note onsets a lap on a cord round a loop: lcm(notes, magnets). 0 off one.
    pub onsets_per_lap: u32,
}

/// The build knobs, all at once (see [`WheelLayout`]); each applies live.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelKnobs {
    pub magnets: u32,
    pub carrier_g: f64,
    pub braking: bool,
    pub release_m: f64,
    pub load_ohms: f64,
    pub timbre: String,
    pub decay_ms: f64,
}

/// What the knobs set on the voice beyond the patch (the carrier, the drag, the release).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Voicing {
    carrier_g: f64,
    braking: bool,
    release_m: f64,
}

impl Voicing {
    /// The voice's settings for `inst` with these knobs over `p`.
    fn apply(&self, inst: &Instrument, p: &mut windwheel::live::Params) {
        let rt = &inst.resolved.runtime;
        let carrier = self.carrier_g.max(0.0) / 1000.0;
        let magnet = inst.magnet().mass();
        p.mass = (if rt.cord { carrier + p.magnets as f64 * magnet } else { carrier + magnet }).max(1e-4);
        p.braking = self.braking;
        match release_kind(inst) {
            "drop" => p.release = self.release_m.max(0.0),
            "pluck" => p.amplitude = self.release_m.max(0.0),
            _ => {}
        }
    }

    /// The patch's own: the toy's 2 g carrier, drag on, its release.
    fn of(inst: &Instrument) -> Voicing {
        let rt = &inst.resolved.runtime;
        let release_m = match release_kind(inst) {
            "drop" => rt.release_m,
            "pluck" => rt.amplitude_m,
            _ => 0.0,
        };
        Voicing { carrier_g: 2.0, braking: true, release_m }
    }
}

fn release_kind(inst: &Instrument) -> &'static str {
    match inst.resolved.runtime.law {
        "gravity" => "drop",
        "spring" => "pluck",
        _ => "",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WheelMagnet {
    pub z: f64,
    /// Its speed along the path (m/s): the drawing moves it on between polls.
    pub v: f64,
    pub on: bool,
    pub segment: u32,
}

/// Where the magnets are, for the drawing's per-frame poll: only the wheel's own lock, held for
/// a copy (never the engine's), so polling it at the display's rate costs the feeder nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelMotion {
    pub magnets: Vec<WheelMagnet>,
    /// The voice's clock (s): frames it has rendered over the sample rate. The drawing maps it
    /// onto the display's clock, so a poll's timing doesn't jitter the motion.
    pub voice_s: f64,
}

/// The wheel now, for the tab's ~30 Hz poll.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelState {
    pub magnets: Vec<WheelMagnet>,
    /// The cord's speed, or the fastest magnet's (m/s).
    pub speed_mps: f64,
    /// Output peak since the last poll, 0…1.
    pub out_peak: f64,
    /// The coil's drag on the motion (N) and the power it takes out (mW).
    pub drag_n: f64,
    pub power_mw: f64,
    pub trigger: f64,
    /// Dropouts since the player started (they only count up): blocks the strip found its ring
    /// short of and played silence for, and ring overruns it skipped ahead on.
    pub underruns: u32,
    pub skips: u32,
    /// The least the ring held when the feeder looked, since the last poll (0: it ran dry), and
    /// the biggest block the output has taken from the strip at once (its callback), in ms.
    pub ring_low_ms: f64,
    pub block_ms: f64,
    /// What the feeder keeps the ring at, in ms (it grows after an underrun).
    pub ring_target_ms: f64,
    /// The wheel's tempo: note onsets per minute at the cord's actual speed (smoothed over the
    /// last few polls), notes × laps/s × 60. None off a cord (tubes, pendulum, spring, rail).
    pub bpm: Option<f64>,
    /// How that tempo sits against each deck playing (filled by the player).
    pub tempo: Vec<WheelTempo>,
    /// Paused (Play/Pause): everything held where it was, silent, until played again.
    pub paused: bool,
    /// It has been played since the player started (before that: stopped, the button says Play).
    pub started: bool,
    /// What the mix hears of it: the strip's peak after its effects (the Effects page's tricks,
    /// then the Inserts) and its fader, the last block, 0…1. 0 without a strip.
    pub strip_peak: f64,
}

/// The wheel's tempo against one deck's.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelTempo {
    pub deck: String,
    pub deck_bpm: f64,
    /// wheel BPM / deck BPM.
    pub ratio: f64,
    /// The nearest whole-number relation: the wheel at n× the deck (multiple) or 1/n of it.
    pub n: u32,
    pub multiple: bool,
    /// Off that relation by this much (relative; + the wheel is fast).
    pub error: f64,
    /// Within windwheel's tolerance (1%): a whole-number multiple or divisor.
    pub locked: bool,
    /// A rung below / above to step to within the motor's range (the Tempo card's ◀ ▶).
    pub down: bool,
    pub up: bool,
    /// The ratio as a place on the ladder (×1 at 0, ×2 at 1, ½ at −1: windwheel's ladder_place),
    /// and the places the motor reaches (the sync × slider's ends); None: no whole rung in reach.
    pub place: f64,
    pub rung_lo: Option<i32>,
    pub rung_hi: Option<i32>,
}

/// What a tempo sync set.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelSync {
    pub deck: String,
    pub deck_bpm: f64,
    /// The tempo aimed at: n× the deck (`multiple`), or 1/n of it.
    pub target_bpm: f64,
    pub n: u32,
    pub multiple: bool,
    /// The motor's setting for it (the cord settles a little under, against friction and drag).
    pub motor_mps: f64,
    /// The cord speed it settles at.
    pub speed_mps: f64,
}

const EDGE: &str = "the motor's end";

/// Under 1/this of a deck's tempo the wheel counts as still against it.
const STILL_BELOW: f64 = 64.0;

/// The wheel's tempo against `deck` at `deck_bpm`; `range` the tempos the motor reaches (BPM).
pub fn tempo_against(deck: &str, wheel_bpm: f64, deck_bpm: f64, range: Option<(f64, f64)>) -> Option<WheelTempo> {
    // a wheel coasting to rest has a tempo that only tends to 0: below 1/64 of the deck's it is
    // still (no ratio; else 1/ratio overflows the rung count and the card reads ÷4294967295)
    if !(wheel_bpm >= deck_bpm / STILL_BELOW) {
        return None;
    }
    let can = |up: bool| range.is_some_and(|(lo, hi)| windwheel::live::step_rung(wheel_bpm, deck_bpm, up, lo, hi).is_some());
    let span = range.and_then(|(lo, hi)| windwheel::live::rung_span(deck_bpm, lo, hi));
    windwheel::live::relate(wheel_bpm, deck_bpm).map(|r| WheelTempo {
        deck: deck.to_string(),
        deck_bpm,
        ratio: r.ratio,
        n: r.n,
        multiple: r.multiple,
        error: r.error,
        locked: r.locked,
        down: can(false),
        up: can(true),
        place: windwheel::live::ladder_place(r.ratio),
        rung_lo: span.map(|(a, _)| a as i32),
        rung_hi: span.map(|(_, b)| b as i32),
    })
}

/// The notes `text` reads as over `root` ("C3", "E3", …): the sequence field's chips, before it
/// winds (windwheel's own parser), or why it can't be read.
pub fn parse_sequence(text: &str, root: &str) -> Result<Vec<String>, String> {
    windwheel::patch::parse_sequence(text, root).map_err(|e| e.to_string())
}

/// The library, in its order.
pub fn presets() -> Vec<WheelPreset> {
    windwheel::presets::PRESETS
        .iter()
        .filter_map(|(id, json)| {
            let p = windwheel::patch::Patch::from_json(json).ok()?;
            Some(WheelPreset {
                id: id.to_string(),
                name: p.name.clone(),
                description: p.description.clone(),
                law: p.motion.name().to_string(),
                path: path_kind(&p.path).to_string(),
            })
        })
        .collect()
}

fn path_kind(p: &windwheel::patch::PathSpec) -> &'static str {
    use windwheel::patch::PathSpec::*;
    match p {
        Hoop { .. } => "hoop",
        Drum { .. } => "drum",
        Loop { .. } => "loop",
        Tube { .. } => "tube",
        Rack { .. } => "rack",
        Rail { .. } => "rail",
        Incline { .. } => "incline",
        Arc { .. } => "arc",
    }
}

/// What a fresh strip does to its input (trim × fader), undone by the feeder.
fn strip_makeup() -> f32 {
    let fresh = djengine::input::InputSettings::new(INPUT_LABEL);
    let g = djengine::input::gain_of_db(fresh.trim_db) * fresh.volume;
    if g > 0.0 { (1.0 / g) as f32 } else { 1.0 }
}

struct Loaded {
    id: String,
    inst: Instrument,
    volume_db: f64,
    voicing: Voicing,
}

struct Core {
    voice: Voice,
    loaded: Option<Loaded>,
    /// The cord's speed, smoothed over the polls (for the tempo readout).
    smooth_v: f64,
    /// Frames the voice has rendered (its clock, for [`WheelMotion`]).
    frames: u64,
    /// Paused: the feeder stops clocking the voice, so the magnets, their speeds, the motor and
    /// every setting hold exactly where they are until it is let go (the Wheel tab's Play/Pause).
    paused: bool,
    /// The feeder's output gain, eased toward 0 while pausing and back to 1 on play (no click).
    gain: f32,
    /// It has been played: Play pressed, a touch, or seen moving. Before that it is stopped (the
    /// tab's button shows Play); after, the button only pauses and plays.
    started: bool,
}

/// The wheel: its voice, the instrument on it and the feeder.
pub struct Wheel {
    core: Arc<Mutex<Core>>,
    /// The strip's mirror: its ring's fill and its underrun / skip counts.
    mirror: Option<Arc<InputMirror>>,
    feed: Arc<FeedStats>,
    sample_rate: f64,
}

/// What the feeder saw, for [`WheelState`]: the low-water mark since the last poll and the level
/// it fills the ring to (frames).
struct FeedStats {
    low: AtomicU64,
    goal: AtomicU64,
}

impl Wheel {
    /// Start feeding input strip `feed` at `sample_rate`; `start_at` is the strip's start
    /// threshold (frames it waits for before it plays); `mirror` the strip's, for the dropout
    /// counts in [`WheelState`].
    pub fn start(
        sample_rate: f64,
        mut feed: rtrb::Producer<f32>,
        start_at: usize,
        mirror: Option<Arc<InputMirror>>,
        stop: Arc<AtomicBool>,
    ) -> std::io::Result<(Wheel, std::thread::JoinHandle<()>)> {
        let core = Arc::new(Mutex::new(Core { voice: Voice::new(sample_rate), loaded: None, smooth_v: 0.0, frames: 0, paused: true, gain: 0.0, started: false }));
        let shared = core.clone();
        let makeup = strip_makeup();
        let fill_to = start_at + CUSHION;
        let strip = mirror.clone();
        let stats = Arc::new(FeedStats {
            low: AtomicU64::new(u64::MAX),
            goal: AtomicU64::new(fill_to as u64),
        });
        let st = stats.clone();
        // the pause's fade: 10 ms from full to silent (the voice runs on through it)
        let fade_step = (1.0 / (0.010 * sample_rate)) as f32;
        let thread = std::thread::Builder::new().name("djrust-wheel".into()).spawn(move || {
            let mut buf = vec![0f32; CHUNK];
            let cap = feed.buffer().capacity();
            // the ring has been filled once (before that a low ring is the start, not a dropout)
            let mut filled = false;
            let (mut cushions, mut seen) = (1usize, 0u64);
            let mut fill_to = fill_to;
            while !stop.load(Ordering::Relaxed) {
                if let Some(m) = strip.as_deref() {
                    // the strip waits for max(threshold, its biggest block); it skips ahead past
                    // three of those, so the goal stays under two
                    let base = start_at.max(m.block.load(Ordering::Relaxed) as usize);
                    let under = m.underruns.load(Ordering::Relaxed);
                    if under > seen {
                        // the first fill is not an underrun of the feeder's: grow only once playing
                        if filled {
                            cushions = (cushions + 1).min(MAX_CUSHION);
                        }
                        seen = under;
                    }
                    fill_to = (base + cushions * CUSHION).min(2 * base).min(cap - CHUNK);
                    st.goal.store(fill_to as u64, Ordering::Relaxed);
                }
                let level = cap - feed.slots();
                if filled {
                    st.low.fetch_min(level as u64, Ordering::Relaxed);
                }
                if level >= fill_to {
                    filled = true;
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                let n = (fill_to - level).min(CHUNK);
                match shared.lock() {
                    Ok(mut c) => {
                        if c.paused && c.gain <= 0.0 {
                            // held: the voice is not clocked, the strip gets silence
                            buf[..n].fill(0.0);
                        } else {
                            c.voice.render(&mut buf[..n]);
                            c.frames += n as u64;
                            let target = if c.paused { 0.0 } else { 1.0 };
                            if c.gain != target {
                                let mut g = c.gain;
                                for x in &mut buf[..n] {
                                    g = if target > g { (g + fade_step).min(1.0) } else { (g - fade_step).max(0.0) };
                                    *x *= g;
                                }
                                c.gain = g;
                            }
                        }
                    }
                    Err(_) => return,
                }
                let Ok(mut chunk) = feed.write_chunk(n) else { continue };
                let (a, b) = chunk.as_mut_slices();
                for (slot, x) in a.iter_mut().chain(b.iter_mut()).zip(&buf[..n]) {
                    *slot = x * makeup;
                }
                chunk.commit_all();
            }
        })?;
        Ok((Wheel { core, mirror, feed: stats, sample_rate }, thread))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Core) -> T) -> T {
        let mut g = self.core.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    }

    /// Put `inst` on the voice at `volume_db`: the table, the law and the knobs.
    fn install(&self, id: String, inst: Instrument, volume_db: f64, keep_trigger: bool) -> WheelLayout {
        self.install_with(id, inst, volume_db, keep_trigger, None)
    }

    /// Install `inst`; `voicing` (else the loaded one's when keeping, else the patch's own).
    fn install_with(&self, id: String, inst: Instrument, volume_db: f64, keep_trigger: bool, voicing: Option<Voicing>) -> WheelLayout {
        let table = inst.table();
        let mut params = inst.params(volume_db);
        self.with(|c| {
            let voicing = voicing
                .or_else(|| keep_trigger.then(|| c.loaded.as_ref().map(|l| l.voicing)).flatten())
                .unwrap_or_else(|| Voicing::of(&inst));
            if keep_trigger {
                let old = c.voice.params();
                params.trigger_mode = old.trigger_mode;
                params.trigger = old.trigger;
                params.motor = old.motor;
                params.motor_v = old.motor_v;
                params.interval = old.interval;
            }
            voicing.apply(&inst, &mut params);
            c.voice.set_table(table);
            c.voice.set_law(&inst.resolved.runtime);
            c.voice.set_params(&params);
            let layout = layout_of(&id, &inst, volume_db, &params, &voicing);
            c.loaded = Some(Loaded { id, inst, volume_db, voicing });
            layout
        })
    }

    pub fn load(&self, id: &str) -> Result<WheelLayout, String> {
        let json = windwheel::presets::get(id).ok_or_else(|| format!("no instrument {id}"))?;
        let inst = Instrument::from_json(json).map_err(|e| e.to_string())?;
        let db = inst.volume_db(&inst.table());
        Ok(self.install(id.to_string(), inst, db, false))
    }

    pub fn layout(&self) -> Option<WheelLayout> {
        self.with(|c| {
            let l = c.loaded.as_ref()?;
            Some(layout_of(&l.id, &l.inst, l.volume_db, c.voice.params(), &l.voicing))
        })
    }

    /// The build knobs, live: magnets, load and voicing re-wind (a few ms, crossfaded); the
    /// carrier, drag and release go straight to the voice.
    pub fn set_knobs(&self, k: &WheelKnobs) -> Result<WheelLayout, String> {
        let (id, mut inst, db) = self
            .with(|c| c.loaded.as_ref().map(|l| (l.id.clone(), l.inst.clone(), l.volume_db)))
            .ok_or("no instrument loaded")?;
        let e = |e: anyhow::Error| e.to_string();
        if inst.patch.magnets.count != k.magnets.clamp(1, windwheel::live::MAX_MAGNETS as u32) {
            inst.set_magnets(k.magnets).map_err(e)?;
        }
        if (inst.patch.load.ohms - k.load_ohms).abs() > 1e-9 {
            inst.set_load(k.load_ohms).map_err(e)?;
        }
        let changed = inst.voicing().is_some_and(|(t, d)| t != k.timbre || (d * 1000.0 - k.decay_ms).abs() > 1e-6);
        if changed {
            inst.set_voicing(&k.timbre, k.decay_ms / 1000.0).map_err(e)?;
        }
        let voicing = Voicing { carrier_g: k.carrier_g, braking: k.braking, release_m: k.release_m };
        Ok(self.install_with(id, inst, db, true, Some(voicing)))
    }

    fn rewind(&self, f: impl FnOnce(&mut Instrument) -> anyhow::Result<()>) -> Result<WheelLayout, String> {
        let (id, mut inst, db) = self
            .with(|c| c.loaded.as_ref().map(|l| (l.id.clone(), l.inst.clone(), l.volume_db)))
            .ok_or("no instrument loaded")?;
        f(&mut inst).map_err(|e| e.to_string())?;
        Ok(self.install(id, inst, db, true))
    }

    /// Re-wind every note for `root` ("D3") + `transpose` semitones.
    pub fn set_key(&self, root: &str, transpose: i32) -> Result<WheelLayout, String> {
        self.rewind(|i| i.set_key(root, transpose))
    }

    /// Wind a typed note sequence round the path in order ("" removes it).
    pub fn set_sequence(&self, text: &str) -> Result<WheelLayout, String> {
        self.rewind(|i| i.set_sequence(text))
    }

    /// Brightness 0…1: coil radius and magnet size, then re-wind for the new coil.
    pub fn set_brightness(&self, b: f64) -> Result<WheelLayout, String> {
        self.rewind(|i| i.set_brightness(b))
    }

    pub fn set_volume(&self, db: f64) {
        let db = db.clamp(-30.0, 40.0);
        self.with(|c| {
            c.voice.set_gain(10f64.powf(db / 20.0));
            if let Some(l) = c.loaded.as_mut() {
                l.volume_db = db;
            }
        })
    }

    /// The trigger: Some(pull) puts the variable load across the coil, None the patch's load.
    pub fn set_trigger(&self, p: Option<f64>) {
        self.with(|c| {
            let mut params = c.voice.params().clone();
            params.trigger_mode = p.is_some();
            if let Some(p) = p {
                params.trigger = p.clamp(0.0, 1.0);
            }
            c.voice.set_params(&params);
        })
    }

    /// The motor (drum, bow, or a hoop left spinning) on at `speed` m/s, or off.
    pub fn set_motor(&self, on: bool, speed: f64) {
        self.with(|c| {
            let mut params = c.voice.params().clone();
            params.motor = on;
            params.motor_v = speed.clamp(0.0, 25.0);
            c.voice.set_params(&params);
        })
    }

    /// Gravity: drop a magnet every `s` seconds (0: by hand only).
    pub fn set_interval(&self, s: f64) {
        self.with(|c| {
            let mut params = c.voice.params().clone();
            params.interval = s.clamp(0.0, 10.0);
            c.voice.set_params(&params);
        })
    }

    pub fn drive(&self, v: f64) {
        // a hand on it plays it (and lets a pause go)
        self.with(|c| {
            c.paused = false;
            c.started = true;
            c.voice.drive(v)
        })
    }

    pub fn hand(&self, z: f64) {
        // a hand on it plays it (and lets a pause go)
        self.with(|c| {
            c.paused = false;
            c.started = true;
            c.voice.hand(z)
        })
    }

    pub fn release(&self, v: Option<f64>) {
        self.with(|c| c.voice.release(v))
    }

    pub fn pluck(&self, segment: Option<usize>) {
        // a hand on it plays it (and lets a pause go)
        self.with(|c| {
            c.paused = false;
            c.started = true;
            c.voice.pluck(segment)
        })
    }

    /// Pause (hold everything where it is, silent) or play on from exactly there.
    pub fn set_paused(&self, paused: bool) {
        self.with(|c| {
            c.paused = paused;
            c.started |= !paused;
        })
    }

    /// The Play/Pause button: playing (started and not paused) pauses; otherwise plays on from
    /// where it was held, and the very first play kicks it (a cord with its motor off gets the
    /// motor at its speed, anything off a cord is plucked). Returns whether it plays now.
    pub fn play_pause(&self) -> bool {
        self.with(|c| {
            if c.started && !c.paused {
                c.paused = true;
                return false;
            }
            let first = !c.started;
            c.paused = false;
            c.started = true;
            if first {
                if let Some(l) = c.loaded.as_ref() {
                    let cord = l.inst.resolved.runtime.cord;
                    let mut params = c.voice.params().clone();
                    if cord && !params.motor {
                        params.motor = true;
                        c.voice.set_params(&params);
                    } else if !cord {
                        c.voice.pluck(None);
                    }
                }
            }
            true
        })
    }

    /// Played (Play pressed, a touch, or seen moving) and not paused: the button shows Pause.
    pub fn playing(&self) -> bool {
        self.with(|c| c.started && !c.paused)
    }

    pub fn stop(&self) {
        self.with(|c| {
            let mut params = c.voice.params().clone();
            params.motor = false;
            params.interval = 0.0;
            c.voice.set_params(&params);
            c.voice.release(None);
            c.voice.stop();
        })
    }

    /// Set the motor so the wheel's settled tempo lands on the whole-number multiple of
    /// `deck_bpm` nearest its tempo now (at rest: nearest its design tempo).
    pub fn sync(&self, deck: &str, deck_bpm: f64) -> Result<WheelSync, String> {
        self.aim(deck, deck_bpm, |bpm| Ok(windwheel::live::sync_bpm(bpm, deck_bpm)))
    }

    /// Step the wheel one rung of the tempo ladder against `deck` (up: the next whole multiple
    /// of its BPM; down: the one below, and on into ½, ⅓, …), within the motor's range.
    /// None at the end of the motor's range (a press there does nothing).
    pub fn step(&self, deck: &str, deck_bpm: f64, up: bool) -> Result<Option<WheelSync>, String> {
        let range = self.bpm_range().ok_or("only a cord round a loop has a tempo")?;
        match self.aim(deck, deck_bpm, |bpm| {
            windwheel::live::step_rung(bpm, deck_bpm, up, range.0, range.1)
                .map(|r| deck_bpm * r.ratio())
                .ok_or_else(|| EDGE.to_string())
        }) {
            Err(e) if e == EDGE => Ok(None),
            r => r.map(Some),
        }
    }

    /// Set the wheel on rung `index` of `deck`'s ladder (×1 at 0, ×2 at 1, ½ at −1), within the
    /// motor's range (the sync × slider).
    pub fn rung(&self, deck: &str, deck_bpm: f64, index: i64) -> Result<WheelSync, String> {
        let r = windwheel::live::Rung::at(index);
        self.aim(deck, deck_bpm, |_| Ok(deck_bpm * r.ratio()))
    }

    /// The tempos the motor reaches with this instrument (BPM), None off a cord round a loop.
    pub fn bpm_range(&self) -> Option<(f64, f64)> {
        self.with(|c| c.loaded.as_ref().and_then(|l| l.inst.bpm_range()))
    }

    /// Set the motor so the wheel's tempo settles on `pick(its tempo now)`.
    fn aim(&self, deck: &str, deck_bpm: f64, pick: impl FnOnce(f64) -> Result<f64, String>) -> Result<WheelSync, String> {
        if !(deck_bpm > 0.0) {
            return Err(format!("deck {deck} has no tempo"));
        }
        self.with(|c| {
            let l = c.loaded.as_ref().ok_or("no instrument loaded")?;
            let inst = &l.inst;
            // from where the motor is taking it (so ▶ ▶ steps twice while it's still getting
            // there), else from where it is
            let speed = c.voice.speed();
            let p = c.voice.params();
            let now = if p.motor { c.voice.settled_speed(p.motor_v) } else if speed > 0.2 { speed } else { inst.resolved.runtime.speed_mps };
            let bpm = inst.bpm(now).ok_or("only a cord round a loop has a tempo")?;
            let target = pick(bpm)?;
            let v = inst.speed_for_bpm(target).ok_or("only a cord round a loop has a tempo")?;
            if !(windwheel::live::MOTOR_MIN_MPS * (1.0 - 1e-9)..=windwheel::live::MOTOR_MAX_MPS * (1.0 + 1e-9)).contains(&v) {
                return Err(format!(
                    "{target:.0} BPM needs the cord at {v:.1} m/s, outside the motor's {}–{} m/s",
                    windwheel::live::MOTOR_MIN_MPS,
                    windwheel::live::MOTOR_MAX_MPS
                ));
            }
            let motor = c.voice.motor_for_speed(v);
            let mut params = c.voice.params().clone();
            params.motor = true;
            params.motor_v = motor;
            c.voice.set_params(&params);
            let ratio = target / deck_bpm;
            Ok(WheelSync {
                deck: deck.to_string(),
                deck_bpm,
                target_bpm: target,
                n: if ratio >= 1.0 { ratio.round() as u32 } else { (1.0 / ratio).round() as u32 },
                multiple: ratio >= 1.0 - 1e-9,
                motor_mps: motor,
                speed_mps: c.voice.settled_speed(motor),
            })
        })
    }

    pub fn motion(&self) -> WheelMotion {
        let (magnets, frames) = self.with(|c| (c.voice.magnets(), c.frames));
        WheelMotion {
            magnets: magnets
                .into_iter()
                .map(|m| WheelMagnet { z: m.z, v: m.v, on: m.on, segment: m.segment as u32 })
                .collect(),
            voice_s: frames as f64 / self.sample_rate,
        }
    }

    pub fn state(&self) -> WheelState {
        self.with(|c| {
            let speed = c.voice.speed();
            if !c.paused && speed.abs() > 0.05 {
                c.started = true;
            }
            c.smooth_v += (speed - c.smooth_v) * 0.25;
            let bpm = c.loaded.as_ref().and_then(|l| l.inst.bpm(c.smooth_v));
            let m: Meters = c.voice.take_meters();
            WheelState {
                magnets: c
                    .voice
                    .magnets()
                    .into_iter()
                    .map(|m| WheelMagnet { z: m.z, v: m.v, on: m.on, segment: m.segment as u32 })
                    .collect(),
                speed_mps: c.voice.speed(),
                out_peak: m.out_peak,
                drag_n: m.f_drag,
                power_mw: m.p_drag.abs() * 1e3,
                trigger: c.voice.trigger_now(),
                underruns: self.count(|m| m.underruns.load(Ordering::Relaxed)),
                skips: self.count(|m| m.skips.load(Ordering::Relaxed)),
                ring_low_ms: match self.feed.low.swap(u64::MAX, Ordering::Relaxed) {
                    u64::MAX => 0.0,
                    f => self.ms(f),
                },
                block_ms: self.ms(self.count(|m| m.block.load(Ordering::Relaxed) as u64) as u64),
                ring_target_ms: self.ms(self.feed.goal.load(Ordering::Relaxed)),
                bpm,
                tempo: vec![],
                paused: c.paused,
                started: c.started,
                strip_peak: self.mirror.as_ref().map_or(0.0, |m| m.level() as f64),
            }
        })
    }

    fn ms(&self, frames: u64) -> f64 {
        frames as f64 * 1e3 / self.sample_rate
    }

    fn count(&self, f: impl Fn(&InputMirror) -> u64) -> u32 {
        self.mirror.as_deref().map_or(0, |m| f(m).min(u32::MAX as u64) as u32)
    }
}

fn layout_of(id: &str, inst: &Instrument, volume_db: f64, params: &windwheel::live::Params, v: &Voicing) -> WheelLayout {
    let p = &inst.patch;
    let r = &inst.resolved;
    let d = inst.drawing();
    let tuning = p.tuning();
    let out = r.notes.iter().filter(|n| !n.reachable).count() + r.patterns.iter().filter(|f| !f.reachable).count();
    WheelLayout {
        id: id.to_string(),
        name: p.name.clone(),
        description: p.description.clone(),
        law: p.motion.name().to_string(),
        path: path_kind(&p.path).to_string(),
        cord: r.runtime.cord,
        periodic: r.periodic,
        extent_m: r.extent_m,
        section_length_m: r.section_length_m,
        shapes: d
            .shapes
            .iter()
            .zip(&r.segments)
            .map(|(s, seg)| WheelShape {
                xs: s.points.iter().map(|p| p[0]).collect(),
                ys: s.points.iter().map(|p| p[1]).collect(),
                zs: s.z.clone(),
                closed: s.closed,
                start_m: seg.start_m,
                length_m: seg.length_m,
                first_section: seg.first_section as u32,
                sections: seg.sections as u32,
            })
            .collect(),
        pivot: d.pivot.map(|p| p.to_vec()),
        anchor: d.anchor.map(|p| p.to_vec()),
        turns: r.turns.iter().map(|&t| t as i32).collect(),
        notes: r
            .notes
            .iter()
            .map(|n| WheelNote {
                segment: n.segment as u32,
                start_section: n.start_section as u32,
                count: n.count as u32,
                name: n.name.clone(),
                freq_hz: n.freq_hz,
                reachable: n.reachable,
                reason: n.reason.clone(),
            })
            .collect(),
        magnets: params.magnets as u32,
        root: tuning.root.clone(),
        transpose: tuning.transpose,
        sequence: inst.sequence(),
        brightness: inst.brightness(),
        volume_db,
        trigger: params.trigger_mode.then_some(params.trigger),
        load_ohms: p.load.ohms,
        coil_ohms: params.r_coil,
        motor: params.motor,
        speed_mps: params.motor_v,
        interval_s: params.interval,
        lo_z: r.runtime.lo_z,
        hi_z: r.runtime.hi_z,
        buildable: r.buildable,
        out_of_reach: out as u32,
        coil_radius_mm: p.coil.radius_m * 1e3,
        magnet_mm: p.magnets.diameter_m * 1e3,
        carrier_g: v.carrier_g,
        braking: v.braking,
        release_m: v.release_m,
        release_kind: release_kind(inst).to_string(),
        timbre: inst.voicing().map(|(t, _)| t).unwrap_or_default(),
        decay_ms: inst.voicing().map_or(0.0, |(_, d)| d * 1000.0),
        timbres: windwheel::patch::TIMBRES.iter().map(|t| t.to_string()).collect(),
        onsets_per_lap: inst.onsets_per_lap().unwrap_or(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_loads_and_feeds_the_ring() {
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        for p in presets() {
            let l = wheel.load(&p.id).unwrap();
            assert_eq!(l.shapes.len(), l.shapes.iter().filter(|s| s.xs.len() == s.zs.len()).count());
            assert!(l.turns.iter().any(|&t| t != 0), "{} is wound", p.id);
        }
        // the feeder keeps the ring at the threshold plus the cushion, and no further
        std::thread::sleep(Duration::from_millis(100));
        let held = rx.slots();
        assert!((1024..=1024 + CUSHION).contains(&held), "{held} frames held");
        if let Ok(c) = rx.read_chunk(held) {
            c.commit_all();
        }
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn a_flick_sounds_and_the_strip_gain_is_undone() {
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 4096, None, stop.clone()).unwrap();
        wheel.load("hoop-looper").unwrap();
        wheel.set_paused(false);
        wheel.release(Some(4.5));
        // read the ring as the strip does, for 300 ms
        let mut peak = 0f32;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(5));
            let n = rx.slots();
            if let Ok(c) = rx.read_chunk(n) {
                let (a, b) = c.as_slices();
                peak = a.iter().chain(b).fold(peak, |m, x| m.max(x.abs()));
                c.commit_all();
            }
        }
        // the voice peaks near −6 dBFS at the design speed; the feeder pre-gains for the strip
        let expect = 0.5 * strip_makeup();
        assert!(peak > 0.3 * expect && peak < 1.2 * strip_makeup(), "peak {peak} vs ~{expect}");
        let s = wheel.state();
        assert!(s.speed_mps > 0.0 && s.out_peak > 0.0);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// Every instrument after every other: the switch lands (its id, law and drawing), and the
    /// new one sounds once played, whatever the old one was doing (spinning on its motor, paused).
    #[test]
    fn every_instrument_switches_to_every_other() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = Arc::new(AtomicBool::new(true));
        let (w, t) = Wheel::start(48_000.0, tx, 1024, None, stop).unwrap();
        t.join().unwrap();
        let ids: Vec<String> = presets().into_iter().map(|p| p.id).collect();
        let play = |w: &Wheel, l: &WheelLayout| {
            if l.cord {
                w.set_motor(true, l.speed_mps);
            } else if l.law != "rail" {
                w.pluck(None);
            }
        };
        // a rail is played by hand: the sled slid from one end to the other as it renders
        let sounds = |w: &Wheel| {
            let rail = w.layout().filter(|l| l.law == "rail");
            let mut peak = 0f32;
            let mut buf = vec![0f32; 480];
            for i in 0..100 {
                if let Some(l) = &rail {
                    w.hand(l.lo_z + (l.hi_z - l.lo_z) * i as f64 / 99.0);
                }
                w.with(|c| c.voice.render(&mut buf));
                peak = buf.iter().fold(peak, |m, x| m.max(x.abs()));
            }
            if rail.is_some() {
                w.release(None);
            }
            peak
        };
        let mut failed = vec![];
        for a in &ids {
            for b in &ids {
                if a == b {
                    continue;
                }
                for paused in [false, true] {
                    let la = w.load(a).unwrap();
                    play(&w, &la);
                    sounds(&w);
                    w.set_paused(paused);
                    let lb = match w.load(b) {
                        Ok(l) => l,
                        Err(e) => {
                            failed.push(format!("{a} -> {b}: {e}"));
                            continue;
                        }
                    };
                    let fresh = {
                        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(16);
                        let (f, t) = Wheel::start(48_000.0, tx, 8, None, Arc::new(AtomicBool::new(true))).unwrap();
                        t.join().unwrap();
                        f.load(b).unwrap()
                    };
                    if lb.id != *b || lb.law != fresh.law || lb.shapes.len() != fresh.shapes.len() || lb.notes.len() != fresh.notes.len() {
                        failed.push(format!("{a} -> {b}: layout {} {} not {}", lb.id, lb.law, fresh.law));
                    }
                    if lb.motor != fresh.motor || lb.interval_s != fresh.interval_s {
                        failed.push(format!("{a} -> {b}: carried motor {} / interval {} (fresh {} / {})", lb.motor, lb.interval_s, fresh.motor, fresh.interval_s));
                    }
                    w.set_paused(false);
                    play(&w, &lb);
                    let peak = sounds(&w);
                    if peak < 1e-3 {
                        failed.push(format!("{a} -> {b} (paused {paused}): silent after playing it, peak {peak}"));
                    }
                }
            }
        }
        assert!(failed.is_empty(), "{} failures:\n{}", failed.len(), failed.join("\n"));
    }

    #[test]
    fn a_wheel_coasting_to_rest_has_no_ratio() {
        assert!(tempo_against("A", 1e-7, 120.0, Some((60.0, 2000.0))).is_none());
        assert!(tempo_against("A", 0.0, 120.0, None).is_none());
        let t = tempo_against("A", 60.0, 120.0, None).unwrap();
        assert!(!t.multiple && t.n == 2);
    }

    #[test]
    fn pause_holds_the_magnets_and_play_goes_on_from_there() {
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new(8192);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        wheel.load("hoop-looper").unwrap();
        assert!(!wheel.state().started, "stopped until played");
        wheel.set_motor(true, 2.0);
        wheel.set_paused(false);
        let drain = |rx: &mut rtrb::Consumer<f32>, ms: u64| {
            let mut peak = 0f32;
            for _ in 0..ms / 5 {
                std::thread::sleep(Duration::from_millis(5));
                if let Ok(c) = rx.read_chunk(rx.slots()) {
                    let (a, b) = c.as_slices();
                    peak = a.iter().chain(b).fold(peak, |m, x| m.max(x.abs()));
                    c.commit_all();
                }
            }
            peak
        };
        drain(&mut rx, 300);
        wheel.set_paused(true);
        drain(&mut rx, 100); // the fade, and whatever was in the ring
        let held = wheel.motion();
        let v = wheel.state().speed_mps;
        assert!(v > 0.1, "it was moving: {v}");
        let peak = drain(&mut rx, 200);
        let still = wheel.motion();
        assert_eq!(held.voice_s, still.voice_s, "paused: the voice is not clocked");
        assert_eq!(held.magnets, still.magnets, "paused: the magnets hold where they are");
        assert_eq!(peak, 0.0, "paused: silent");
        let s = wheel.state();
        assert!(s.paused && s.speed_mps == v, "the speed is held too");
        assert!(wheel.with(|c| c.voice.params().motor), "the motor setting is kept");
        wheel.drive(1.0); // a hand on it lets the pause go
        assert!(!wheel.state().paused);
        wheel.release(None);
        wheel.set_paused(true);
        wheel.set_paused(false);
        let peak = drain(&mut rx, 200);
        assert!(wheel.motion().voice_s > still.voice_s && peak > 0.0, "play goes on");
        let s = wheel.state();
        assert!(!s.paused && s.started);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn a_new_wheel_is_held_and_silent_until_played() {
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new(8192);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        // an instrument whose motor / gravity would sound by itself once clocked
        wheel.load("hoop-looper").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let mut peak = 0f32;
        if let Ok(c) = rx.read_chunk(rx.slots()) {
            let (a, b) = c.as_slices();
            peak = a.iter().chain(b).fold(0f32, |m, x| m.max(x.abs()));
            c.commit_all();
        }
        assert_eq!(peak, 0.0, "nothing is generated before it is played");
        assert_eq!(wheel.motion().voice_s, 0.0, "the voice is not clocked");
        let s = wheel.state();
        assert!(s.paused && !s.started && !wheel.playing());
        assert!(wheel.play_pause(), "Play starts it");
        assert!(wheel.playing());
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn key_change_rewinds_and_keeps_the_hand_controls() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        let a = wheel.load("music-box").unwrap();
        wheel.set_trigger(Some(0.5));
        let b = wheel.set_key(&a.root, 5).unwrap();
        assert_eq!(b.transpose, 5);
        assert!((b.notes[0].freq_hz / a.notes[0].freq_hz - 2f64.powf(5.0 / 12.0)).abs() < 1e-6);
        assert_eq!(b.trigger, Some(0.5));
        let c = wheel.set_brightness(0.8).unwrap();
        assert!(c.coil_radius_mm < b.coil_radius_mm);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn a_sequence_winds_in_order_and_follows_the_key() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        let a = wheel.load("cmaj7-hoop").unwrap();
        assert_eq!(a.sequence, "C E G B C B G E");
        let names: Vec<_> = a.notes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["C3", "E3", "G3", "B3", "C4", "B3", "G3", "E3"]);
        let b = wheel.set_sequence("D F# A").unwrap();
        assert_eq!(b.sequence, "D F# A");
        assert_eq!(b.notes.iter().map(|n| n.name.clone()).collect::<Vec<_>>(), ["D3", "F#3", "A3"]);
        let c = wheel.set_key("C3", 2).unwrap();
        assert_eq!(c.sequence, "D F# A");
        assert_eq!(c.notes[0].name, "E3");
        assert!(wheel.set_sequence("D Q").is_err());
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// The build knobs apply live and stay through a re-wind; the tempo counts every magnet's
    /// onsets (lcm of notes and magnets), and Sync aims with them.
    #[test]
    fn knobs_apply_and_the_tempo_counts_magnets() {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        let l = wheel.load("cmaj7-hoop").unwrap();
        assert_eq!((l.magnets, l.onsets_per_lap, l.braking), (1, 8, true));
        let k = WheelKnobs {
            magnets: 3,
            carrier_g: 10.0,
            braking: false,
            release_m: 0.0,
            load_ohms: 32.0,
            timbre: "organ".into(),
            decay_ms: 80.0,
        };
        let l = wheel.set_knobs(&k).unwrap();
        assert_eq!((l.magnets, l.onsets_per_lap, l.load_ohms), (3, 24, 32.0));
        assert_eq!((l.timbre.as_str(), l.decay_ms.round(), l.carrier_g, l.braking), ("organ", 80.0, 10.0, false));
        wheel.with(|c| {
            let p = c.voice.params();
            assert_eq!((p.magnets, p.braking, p.r_load), (3, false, 32.0));
            assert!((p.mass - (0.010 + 3.0 * c.loaded.as_ref().unwrap().inst.magnet().mass())).abs() < 1e-12);
        });
        // a key change keeps them
        let l = wheel.rewind(|i| i.set_key("D3", 0)).unwrap();
        assert_eq!((l.magnets, l.onsets_per_lap, l.timbre.as_str(), l.braking), (3, 24, "organ", false));
        // Sync with three magnets: 24 onsets a lap
        let s = wheel.sync("A", 124.0).unwrap();
        let bpm = wheel.with(|c| c.loaded.as_ref().unwrap().inst.bpm(s.speed_mps)).unwrap();
        assert!((bpm - s.target_bpm).abs() < 1e-9 && s.target_bpm % 124.0 == 0.0, "{s:?} at {bpm}");
        // tubes: the drop height
        wheel.load("drop-chimes").unwrap();
        let l = wheel.set_knobs(&WheelKnobs { release_m: 0.3, ..k.clone() }).unwrap();
        assert_eq!((l.release_kind.as_str(), l.release_m, l.onsets_per_lap), ("drop", 0.3, 0));
        assert_eq!(wheel.with(|c| c.voice.params().release), 0.3);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn sync_settles_on_a_multiple_of_the_deck() {
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, tx, 1024, None, stop.clone()).unwrap();
        wheel.load("cmaj7-hoop").unwrap();
        wheel.set_paused(false);
        let s = wheel.sync("A", 124.0).unwrap();
        assert_eq!(s.target_bpm, 124.0 * s.n as f64);
        // let it settle (motor τ 0.8 s), draining the ring as the strip would
        let mut last = None;
        for _ in 0..400 {
            std::thread::sleep(Duration::from_millis(10));
            let n = rx.slots();
            if let Ok(c) = rx.read_chunk(n) {
                c.commit_all();
            }
            last = Some(wheel.state());
        }
        let st = last.unwrap();
        let t = tempo_against("A", st.bpm.unwrap(), 124.0, wheel.bpm_range()).unwrap();
        assert!(t.locked && t.multiple && t.n == s.n, "{st:?} {t:?}");
        // the motor holds its speed: the tempo is the target, not a friction-short of it
        assert!(t.error.abs() < 1e-3, "tempo off by {:+.4}%", t.error * 100.0);
        // ◀ ▶: one rung each way from the rung it's locked on, and the motor's ends refuse
        let up = wheel.step("A", 124.0, true).unwrap().unwrap();
        assert_eq!((up.target_bpm, up.n, up.multiple), (124.0 * (s.n + 1) as f64, s.n + 1, true));
        let (lo, hi) = wheel.bpm_range().unwrap();
        // the slider's rungs: each one lands exactly, and the span is what the motor reaches
        let t = tempo_against("A", up.target_bpm, 124.0, Some((lo, hi))).unwrap();
        let (a, b) = (t.rung_lo.unwrap(), t.rung_hi.unwrap());
        assert!((t.place - s.n as f64).abs() < 1e-9 && a <= t.place as i32 && t.place as i32 <= b);
        for i in [a, (a + b) / 2, b] {
            let r = wheel.rung("A", 124.0, i as i64).unwrap();
            assert!((r.target_bpm / 124.0 - windwheel::live::Rung::at(i as i64).ratio()).abs() < 1e-12);
        }
        assert!(wheel.rung("A", 124.0, b as i64 + 1).is_err(), "past the motor's top");
        wheel.rung("A", 124.0, up.n as i64 - 1).unwrap();
        let mut last = up;
        while let Some(n) = wheel.step("A", 124.0, true).unwrap() {
            assert!(n.target_bpm <= hi + 1e-6 && n.target_bpm > last.target_bpm);
            last = n;
        }
        assert!(last.target_bpm + 124.0 > hi, "stopped short of the top: {last:?} hi {hi}");
        let mut low = last;
        while let Some(n) = wheel.step("A", 124.0, false).unwrap() {
            assert!(n.target_bpm >= lo - 1e-6 && n.target_bpm < low.target_bpm);
            low = n;
        }
        // it stops on the lowest rung in reach: the next one down (×1, then ½, ⅓, … when the
        // hoop is slow enough to reach them) is under the motor's slowest
        let below = windwheel::live::step_rung(low.target_bpm, 124.0, false, 0.0, f64::INFINITY).unwrap();
        assert!(124.0 * below.ratio() < lo, "stopped short of the bottom: {low:?} lo {lo}");
        assert!(wheel.load("drop-chimes").is_ok() && wheel.sync("A", 124.0).is_err(), "tubes have no tempo");
        assert!(wheel.step("A", 124.0, true).is_err());
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// Through the engine's own strip, with an output whose callback is longer than the strip's
    /// start threshold (the Android emulator's is ~45 ms): no dropouts once it is playing, and
    /// the counters say so.
    #[test]
    fn a_long_output_callback_does_not_starve_the_strip() {
        let mut cfg = crate::deck::EngineConfig::new(48_000.0);
        cfg.extended = false;
        cfg.inputs = vec![INPUT_LABEL.to_string()];
        let (mut e, mut r) = crate::deck::engine(cfg);
        let (feed, _) = e.take_input_feed(0).unwrap();
        let mirror = e.inputs[0].mirror.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let (wheel, thread) = Wheel::start(48_000.0, feed, 1024, Some(mirror.clone()), stop.clone()).unwrap();
        wheel.load("hoop-looper").unwrap();
        let l = wheel.layout().unwrap();
        wheel.set_paused(false);
        wheel.set_motor(true, l.speed_mps);
        let n = 2176;
        let (mut a, mut b) = (vec![0f32; n], vec![0f32; n]);
        let mut early = 0;
        for k in 0..40u32 {
            // the callback's pace: a block, then its length (a late sleep only slows it down; a
            // device's callback does not fire twice to catch up)
            std::thread::sleep(Duration::from_secs_f64(n as f64 / 48_000.0));
            r.render_block(&mut a, &mut b);
            if k == 5 {
                early = mirror.underruns.load(Ordering::Relaxed);
            }
        }
        let s = wheel.state();
        assert_eq!(s.underruns as u64, mirror.underruns.load(Ordering::Relaxed));
        assert_eq!(s.underruns as u64, early, "dropouts once playing: {s:?}");
        assert_eq!(s.skips, 0);
        assert!(s.ring_target_ms > n as f64 / 48.0, "kept above a block: {s:?}");
        assert!(a.iter().any(|x| x.abs() > 1e-3), "it sounds");
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }
}

/// Where each Wheel knob's useful values lie, measured: the voice rendered offline while a knob
/// sweeps, its level and onsets printed. Not a check (it asserts nothing); run it by hand when a
/// slider's range or taper is in question:
/// `cargo test --lib wheel::sweep -- --ignored --nocapture`.
#[cfg(test)]
mod sweep {
    use super::*;

    const SR: f64 = 48_000.0;

    /// A wheel with no feeder (its thread stops at once): rendered here, at any speed.
    fn offline(id: &str) -> Wheel {
        let (tx, _rx) = rtrb::RingBuffer::<f32>::new(4096);
        let stop = Arc::new(AtomicBool::new(true));
        let (w, t) = Wheel::start(SR, tx, 1024, None, stop).unwrap();
        t.join().unwrap();
        w.load(id).unwrap();
        w
    }

    /// Render `s` seconds: (peak, rms, onsets: 10 ms blocks rising past a quarter of the peak).
    fn listen(w: &Wheel, s: f64) -> (f32, f32, usize) {
        let n = (s * SR) as usize;
        let mut buf = vec![0f32; n];
        w.with(|c| c.voice.render(&mut buf));
        let peak = buf.iter().fold(0f32, |m, x| m.max(x.abs()));
        let rms = (buf.iter().map(|x| x * x).sum::<f32>() / n as f32).sqrt();
        let blk = (SR * 0.01) as usize;
        let lv: Vec<f32> = buf.chunks(blk).map(|c| c.iter().fold(0f32, |m, x| m.max(x.abs()))).collect();
        let th = peak * 0.25;
        let onsets = lv.windows(2).filter(|p| p[0] < th && p[1] >= th).count();
        (peak, rms, onsets)
    }

    fn db(x: f32) -> String {
        if x <= 1e-9 { "  -inf".into() } else { format!("{:6.1}", 20.0 * x.log10()) }
    }

    fn knobs(w: &Wheel) -> WheelKnobs {
        let l = w.layout().unwrap();
        WheelKnobs { magnets: l.magnets, carrier_g: l.carrier_g, braking: l.braking, release_m: l.release_m, load_ohms: l.load_ohms, timbre: l.timbre, decay_ms: l.decay_ms }
    }

    #[test]
    #[ignore]
    fn sweep() {
        println!("\n== Drop by itself (interval s): 6 s each, onsets and level");
        for id in ["drop-chimes", "marble-run", "spring-bass"] {
            for s in [0.02, 0.05, 0.1, 0.15, 0.2, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0] {
                let w = offline(id);
                w.set_interval(s);
                let (p, r, o) = listen(&w, 6.0);
                println!("{id:12} every {s:5.2} s: peak {} dB rms {} dB onsets {o:3}", db(p), db(r));
            }
        }
        println!("\n== Drop height (release_m): one drop, 2 s");
        for id in ["drop-chimes", "marble-run"] {
            for cm in [0.5, 1.0, 2.0, 3.0, 5.0, 7.5, 10.0, 15.0, 20.0, 30.0, 50.0] {
                let w = offline(id);
                w.set_knobs(&WheelKnobs { release_m: cm / 100.0, ..knobs(&w) }).unwrap();
                w.pluck(Some(0));
                let (p, r, o) = listen(&w, 2.0);
                println!("{id:12} {cm:5.1} cm: peak {} dB rms {} dB onsets {o}", db(p), db(r));
            }
        }
        println!("\n== Pluck (release_m, mm): one pluck, 2 s");
        for id in ["pendulum", "spring-bass"] {
            for mm in [0.5, 1.0, 2.0, 3.0, 5.0, 7.5, 10.0, 15.0, 20.0, 30.0, 50.0] {
                let w = offline(id);
                w.set_interval(0.0);
                w.set_knobs(&WheelKnobs { release_m: mm / 1000.0, ..knobs(&w) }).unwrap();
                w.pluck(None);
                let (p, r, o) = listen(&w, 2.0);
                println!("{id:12} {mm:5.1} mm: peak {} dB rms {} dB onsets {o}", db(p), db(r));
            }
        }
        println!("\n== Decay (ms), hoop at its speed: 2 s, the level (a short decay is quieter, staccato)");
        for id in ["cmaj7-hoop", "music-box"] {
            for ms in [0.0, 5.0, 10.0, 20.0, 40.0, 80.0, 150.0, 300.0, 500.0, 1000.0] {
                let w = offline(id);
                let k = knobs(&w);
                if k.timbre.is_empty() { println!("{id}: no timbre"); break; }
                w.set_knobs(&WheelKnobs { decay_ms: ms, ..k }).unwrap();
                let v = w.layout().unwrap().speed_mps;
                w.set_motor(true, v);
                listen(&w, 1.0);
                let (p, r, _) = listen(&w, 2.0);
                println!("{id:12} decay {ms:6.0} ms: peak {} dB rms {} dB", db(p), db(r));
            }
        }
        println!("\n== Carrier (g), hoop flicked at 4.5 m/s: level over 1 s and the speed after");
        for g in [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0, 70.0, 100.0] {
            let w = offline("hoop-looper");
            w.set_knobs(&WheelKnobs { carrier_g: g, ..knobs(&w) }).unwrap();
            w.release(Some(4.5));
            let (p, r, _) = listen(&w, 1.0);
            println!("carrier {g:5.1} g: peak {} dB rms {} dB, speed after 1 s {:.2} m/s", db(p), db(r), w.with(|c| c.voice.speed()));
        }
        println!("\n== Carrier (g) with the coil's drag on, hoop flicked at 4.5 m/s");
        for g in [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0, 70.0, 100.0] {
            let w = offline("hoop-looper");
            w.set_knobs(&WheelKnobs { carrier_g: g, braking: true, ..knobs(&w) }).unwrap();
            w.release(Some(4.5));
            let (p, r, _) = listen(&w, 1.0);
            println!("carrier {g:5.1} g, drag on: peak {} dB rms {} dB, speed after 1 s {:.2} m/s", db(p), db(r), w.with(|c| c.voice.speed()));
        }
        println!("\n== Level: each preset's own (dB)");
        for p in presets() {
            let w = offline(&p.id);
            println!("{:14} {:+.1} dB", p.id, w.layout().unwrap().volume_db);
        }
        println!("\n== Speed (m/s), hoop-looper motor: level and tempo");
        for v in [0.5, 1.0, 2.0, 3.0, 4.5, 6.0, 8.0, 12.0, 16.0, 20.0] {
            let w = offline("hoop-looper");
            w.set_motor(true, v);
            listen(&w, 1.5);
            let (p, r, _) = listen(&w, 1.0);
            println!("speed {v:5.1}: peak {} dB rms {} dB bpm {:.0}", db(p), db(r), w.state().bpm.unwrap_or(0.0));
        }
        println!("\n== Brightness, hoop at its speed");
        for b in [0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0] {
            let w = offline("cmaj7-hoop");
            w.set_brightness(b).unwrap();
            w.set_motor(true, 4.5);
            listen(&w, 1.0);
            let (p, r, _) = listen(&w, 1.0);
            println!("brightness {b:4.2}: peak {} dB rms {} dB", db(p), db(r));
        }
        println!("\n== Trigger pull, hoop at its speed");
        for t in [0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0] {
            let w = offline("cmaj7-hoop");
            w.set_trigger(Some(t));
            w.set_motor(true, 4.5);
            listen(&w, 1.0);
            let (p, r, _) = listen(&w, 1.0);
            println!("pull {t:4.2}: peak {} dB rms {} dB", db(p), db(r));
        }
    }
}
