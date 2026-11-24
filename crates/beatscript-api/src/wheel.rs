//! The Wheel as the browser plays it: windwheel's linear tonewheel (public stackdump/windwheel-bitwrap-io, `windwheel::live`),
//! the same instrument the booth plays (crates/djhost/src/wheel.rs), so a `wheel` line sounds here as it does there.
//!
//! The booth has two halves and so does this:
//!
//! - **winding** ([`Winder`]): loading a preset, a key, a note sequence or a brightness re-winds the instrument (the
//!   live inverse design, a few ms to a few tens of ms). The booth does that off the audio lock; here it is done off the
//!   audio thread, in the composer Worker (engine-composer.js), and the result is handed to the voice as bytes
//!   ([`Wound`]): the table, the motion law and the knobs, exactly what `djhost::wheel::Wheel::install_with` puts on its
//!   voice.
//! - **the voice** ([`WheelCore`]): `windwheel::live::Voice` and the booth's player state around it (paused, the
//!   10 ms pause fade, the first play's kick, the motor, the level, the tempo sync), in the AudioWorklet. Its output goes
//!   through what the booth's Windwheel input strip does to it at a fresh strip's knobs (djrust `host::deck::input`):
//!   the feeder's make-up gain, the strip's 20 Hz DC blocker, its trim and its fader (which the make-up undoes), mono on
//!   both sides, added to the master after the decks.
//!
//! Every method of `djhost::wheel::Wheel` a program line reaches has its mirror here, in the same words, so the two can
//! be read side by side; djweb's `tests/wheel_parity.rs` renders both from the same lines and compares the samples.
//!
//! **One Wheel for the page and the server.** This module is behind beatscript-api's `wheel` feature: djweb turns it on and
//! wraps it for the page (`WheelWinder` in the Worker, `WheelVoice` in the worklet), and `beatscript render` turns it on
//! and drives the same two halves on its offline engine (beatscript-cli `browser.rs`), so a render's Wheel is the page's,
//! sample for sample.

use windwheel::live::{Instrument, Params, Table, Voice, MAX_MAGNETS, MOTOR_MAX_MPS, MOTOR_MIN_MPS};
use windwheel::patch::Runtime;

/// The motion laws a runtime names (`windwheel::live::Law::of`): what [`Wound`] encodes as an index.
const LAWS: [&str; 7] = ["drive", "motor", "bow", "gravity", "pendulum", "spring", "rail"];

/// The pause's fade, full to silent (djhost::wheel: `fade_step`).
const FADE_S: f64 = 0.010;
/// The most the booth's feeder renders at once (djhost::wheel `CHUNK`): the voice's block here.
pub const BLOCK: usize = 256;

// ---- the voicing (djhost::wheel::Voicing, the same numbers) ----

#[derive(Debug, Clone, Copy, PartialEq)]
struct Voicing {
    carrier_g: f64,
    braking: bool,
    release_m: f64,
}

impl Voicing {
    fn apply(&self, inst: &Instrument, p: &mut Params) {
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

// ---- what a wind hands the voice ----

/// What the instrument's tempo is computed from (`Instrument::bpm`, `speed_for_bpm`, `bpm_range`): note onsets round a
/// lap and the lap, on a cord round a loop (`onsets` None off one), and the speed the notes are wound for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    pub onsets: Option<u32>,
    pub extent_m: f64,
    pub design_mps: f64,
}

impl Tempo {
    fn of(inst: &Instrument) -> Tempo {
        Tempo { onsets: inst.onsets_per_lap().map(|n| n as u32), extent_m: inst.resolved.extent_m, design_mps: inst.resolved.runtime.speed_mps }
    }
    /// `Instrument::bpm`.
    pub fn bpm(&self, speed_mps: f64) -> Option<f64> {
        Some(windwheel::live::wheel_bpm(self.onsets? as usize, self.extent_m, speed_mps))
    }
    /// `Instrument::speed_for_bpm`.
    pub fn speed_for_bpm(&self, bpm: f64) -> Option<f64> {
        let n = self.onsets?;
        Some(bpm / 60.0 * self.extent_m / n as f64)
    }
    /// `Instrument::bpm_range`.
    pub fn bpm_range(&self) -> Option<(f64, f64)> {
        Some((self.bpm(MOTOR_MIN_MPS)?, self.bpm(MOTOR_MAX_MPS)?))
    }
}

/// An instrument wound and ready for the voice: `djhost::wheel::Wheel::install_with`'s table, law and knobs (the
/// knobs with the voicing applied), and whether the voice keeps its own trigger, motor and interval (a re-wind of the
/// loaded instrument) or takes the patch's (a load).
#[derive(Debug, Clone)]
pub struct Wound {
    pub id: String,
    pub keep: bool,
    pub cord: bool,
    pub tempo: Tempo,
    pub params: Params,
    pub runtime: Runtime,
    pub table: Table,
}

// ---- the winding half ----

struct Loaded {
    id: String,
    inst: Instrument,
    volume_db: f64,
    voicing: Voicing,
}

/// The instrument and what a re-wind needs (djhost::wheel's `Loaded`): lives where winding may take its time.
#[derive(Default)]
pub struct Winder {
    loaded: Option<Loaded>,
}

impl Winder {
    pub fn new() -> Winder {
        Winder::default()
    }

    /// The loaded instrument's id.
    pub fn id(&self) -> Option<&str> {
        self.loaded.as_ref().map(|l| l.id.as_str())
    }

    /// The loaded instrument's key: (root, transpose).
    pub fn key(&self) -> Option<(String, i32)> {
        self.loaded.as_ref().map(|l| {
            let t = l.inst.patch.tuning();
            (t.root, t.transpose)
        })
    }

    /// The loaded instrument's level (dB).
    pub fn volume_db(&self) -> Option<f64> {
        self.loaded.as_ref().map(|l| l.volume_db)
    }

    /// `djhost::wheel::Wheel::install_with` up to the voice: the knobs for `inst` at `volume_db` (the trigger, motor
    /// and interval are copied from the voice's own by [`WheelCore::install`] when `keep`).
    fn install(&mut self, id: String, inst: Instrument, volume_db: f64, keep: bool) -> Wound {
        let voicing = keep.then(|| self.loaded.as_ref().map(|l| l.voicing)).flatten().unwrap_or_else(|| Voicing::of(&inst));
        let mut params = inst.params(volume_db);
        voicing.apply(&inst, &mut params);
        let w = Wound {
            id: id.clone(),
            keep,
            cord: inst.resolved.runtime.cord,
            tempo: Tempo::of(&inst),
            params,
            runtime: inst.resolved.runtime.clone(),
            table: inst.table(),
        };
        self.loaded = Some(Loaded { id, inst, volume_db, voicing });
        w
    }

    /// `Wheel::load`.
    pub fn load(&mut self, id: &str) -> Result<Wound, String> {
        let json = windwheel::presets::get(id).ok_or_else(|| format!("no instrument {id}"))?;
        let inst = Instrument::from_json(json).map_err(|e| e.to_string())?;
        let db = inst.volume_db(&inst.table());
        Ok(self.install(id.to_string(), inst, db, false))
    }

    /// `Wheel::rewind`.
    fn rewind(&mut self, f: impl FnOnce(&mut Instrument) -> anyhow::Result<()>) -> Result<Wound, String> {
        let (id, mut inst, db) = self.loaded.as_ref().map(|l| (l.id.clone(), l.inst.clone(), l.volume_db)).ok_or("no instrument loaded")?;
        f(&mut inst).map_err(|e| e.to_string())?;
        Ok(self.install(id, inst, db, true))
    }

    /// `Wheel::set_key`.
    pub fn set_key(&mut self, root: &str, transpose: i32) -> Result<Wound, String> {
        self.rewind(|i| i.set_key(root, transpose))
    }

    /// `Wheel::set_sequence`.
    pub fn set_sequence(&mut self, text: &str) -> Result<Wound, String> {
        self.rewind(|i| i.set_sequence(text))
    }

    /// `Wheel::set_brightness`.
    pub fn set_brightness(&mut self, b: f64) -> Result<Wound, String> {
        self.rewind(|i| i.set_brightness(b))
    }

    /// `Wheel::set_volume`'s half here: the level a later re-wind keeps.
    pub fn set_volume(&mut self, db: f64) {
        if let Some(l) = self.loaded.as_mut() {
            l.volume_db = db.clamp(-30.0, 40.0);
        }
    }

    /// A `wheel` line's call (the registry's `wheel` noun: `{action, ...}`), as this half takes it: Some(wound) for a
    /// load or a re-wind, None for a call that is the voice's alone (`level` is both: it is kept here too).
    pub fn call(&mut self, p: &Args) -> Result<Option<Wound>, String> {
        match p.action() {
            "load" => self.load(p.str("preset").ok_or("load needs preset=")?).map(Some),
            "key" => {
                let (root_now, transpose_now) = self.key().ok_or("no instrument loaded")?;
                let root = match p.str("root") {
                    Some(r) => with_octave(r, &root_now),
                    None => root_now,
                };
                let transpose = p.num("transpose").map(|t| t.round() as i32).unwrap_or(transpose_now);
                self.set_key(&root, transpose.clamp(-24, 24)).map(Some)
            }
            "sequence" => self.set_sequence(p.str("notes").unwrap_or("")).map(Some),
            "brightness" => self.set_brightness(p.num("value").ok_or("brightness needs value=")?).map(Some),
            "level" => {
                if let Some(db) = p.num("db") {
                    self.set_volume(db);
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

/// Whether a call re-winds (the winding half's) or is the voice's alone.
pub fn winds(action: &str) -> bool {
    matches!(action, "load" | "key" | "sequence" | "brightness")
}

/// A root without an octave keeps the current one ("F" over "C3" is "F3"; the registry: "without an octave it keeps
/// the current one").
fn with_octave(root: &str, now: &str) -> String {
    let root = root.trim();
    if root.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        return root.to_string();
    }
    let octave: String = now.chars().skip_while(|c| !c.is_ascii_digit() && *c != '-').collect();
    format!("{root}{octave}")
}

// ---- the voice half ----

/// What the voice keeps of the loaded instrument.
#[derive(Debug, Clone)]
struct Info {
    cord: bool,
    tempo: Tempo,
}

/// The one-pole DC blocker of the booth's input strip (djrust `input::DcBlock`, 20 Hz), the same arithmetic, with its
/// state readable so a silent wheel costs nothing.
#[derive(Debug, Clone)]
struct Dc {
    r: f32,
    x1: f32,
    y1: f32,
}

impl Dc {
    fn new(sample_rate: f64) -> Dc {
        Dc { r: (-2.0 * std::f64::consts::PI * 20.0 / sample_rate).exp() as f32, x1: 0.0, y1: 0.0 }
    }
    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let y = *x - self.x1 + self.r * self.y1;
            self.x1 = *x;
            self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
            *x = self.y1;
        }
    }
    fn idle(&self) -> bool {
        self.x1 == 0.0 && self.y1 == 0.0
    }
}

/// The voice and the booth's player state around it (djhost::wheel's `Core`), and the strip it plays through.
pub struct WheelCore {
    voice: Voice,
    info: Option<Info>,
    paused: bool,
    gain: f32,
    started: bool,
    fade_step: f32,
    /// The deck whose tempo `sync`, `step` and `interval beats` read when the line names none.
    follow: String,
    // the strip: the feeder's make-up, the DC blocker, the trim and the fader
    makeup: f32,
    dc: Dc,
    trim: f32,
    fader: f32,
    /// The feeder's last block and how much of it has been played out.
    block: Box<[f32; BLOCK]>,
    at: usize,
    frames: u64,
    /// The master as the browser plays it (see [`WheelCore::master`]): the decks' own FX unit chain and strip reverb, set
    /// as a program's `master:` lines set both decks', so the wheel is filtered and reverbed with the mix.
    master: djengine::fx::master_chain::MasterChain,
    reverb: djengine::fx::strip_reverb::StripReverb,
    /// The master level (`mixer master=`, djrust `Engine::set_master`: a gain after the master's inserts, 1 at the start).
    level: f32,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl WheelCore {
    pub fn new(sample_rate: f64) -> WheelCore {
        // a fresh strip's trim and fader (djrust input::InputSettings::new), undone by the feeder (djhost strip_makeup)
        let fresh = djengine::input::InputSettings::new("Windwheel");
        let g = djengine::input::gain_of_db(fresh.trim_db) * fresh.volume;
        WheelCore {
            voice: Voice::new(sample_rate),
            info: None,
            paused: true,
            gain: 0.0,
            started: false,
            fade_step: (1.0 / (FADE_S * sample_rate)) as f32,
            follow: "A".into(),
            makeup: if g > 0.0 { (1.0 / g) as f32 } else { 1.0 },
            dc: Dc::new(sample_rate),
            trim: djengine::input::gain_of_db(fresh.trim_db) as f32,
            fader: fresh.volume as f32,
            block: Box::new([0.0; BLOCK]),
            at: BLOCK,
            frames: 0,
            master: {
                let mut c = djengine::fx::master_chain::MasterChain::new(sample_rate);
                c.set_rest_bypass(true);
                c
            },
            reverb: djengine::fx::strip_reverb::StripReverb::new(sample_rate),
            level: 1.0,
            l: vec![0.0; 128],
            r: vec![0.0; 128],
        }
    }

    /// `Wheel::install_with`, the voice's half: a load takes the wound knobs as they are; a re-wind keeps the voice's
    /// trigger, motor and interval.
    pub fn install(&mut self, w: &Wound) {
        let mut params = w.params.clone();
        if w.keep {
            let old = self.voice.params();
            params.trigger_mode = old.trigger_mode;
            params.trigger = old.trigger;
            params.motor = old.motor;
            params.motor_v = old.motor_v;
            params.interval = old.interval;
        }
        self.voice.set_table(w.table.clone());
        self.voice.set_law(&w.runtime);
        self.voice.set_params(&params);
        self.info = Some(Info { cord: w.cord, tempo: w.tempo });
    }

    /// `Wheel::set_volume`.
    pub fn set_volume(&mut self, db: f64) {
        let db = db.clamp(-30.0, 40.0);
        self.voice.set_gain(10f64.powf(db / 20.0));
    }

    /// `Wheel::set_trigger`.
    pub fn set_trigger(&mut self, p: Option<f64>) {
        let mut params = self.voice.params().clone();
        params.trigger_mode = p.is_some();
        if let Some(p) = p {
            params.trigger = p.clamp(0.0, 1.0);
        }
        self.voice.set_params(&params);
    }

    /// `Wheel::set_motor`.
    pub fn set_motor(&mut self, on: bool, speed: f64) {
        let mut params = self.voice.params().clone();
        params.motor = on;
        params.motor_v = speed.clamp(0.0, 25.0);
        self.voice.set_params(&params);
    }

    /// `Wheel::set_interval`.
    pub fn set_interval(&mut self, s: f64) {
        let mut params = self.voice.params().clone();
        params.interval = s.clamp(0.0, 10.0);
        self.voice.set_params(&params);
    }

    /// `Wheel::pluck`.
    pub fn pluck(&mut self, segment: Option<usize>) {
        self.paused = false;
        self.started = true;
        self.voice.pluck(segment)
    }

    /// `Wheel::set_paused`.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.started |= !paused;
    }

    /// `Wheel::play_pause`.
    pub fn play_pause(&mut self) -> bool {
        if self.started && !self.paused {
            self.paused = true;
            return false;
        }
        let first = !self.started;
        self.paused = false;
        self.started = true;
        if first {
            if let Some(cord) = self.info.as_ref().map(|i| i.cord) {
                let mut params = self.voice.params().clone();
                if cord && !params.motor {
                    params.motor = true;
                    self.voice.set_params(&params);
                } else if !cord {
                    self.voice.pluck(None);
                }
            }
        }
        true
    }

    /// `Wheel::playing`.
    pub fn playing(&self) -> bool {
        self.started && !self.paused
    }

    /// `Wheel::stop`.
    pub fn stop(&mut self) {
        let mut params = self.voice.params().clone();
        params.motor = false;
        params.interval = 0.0;
        self.voice.set_params(&params);
        self.voice.release(None);
        self.voice.stop();
    }

    /// `Wheel::aim`: the motor so the tempo settles on `pick(its tempo now)`.
    fn aim(&mut self, deck: &str, deck_bpm: f64, pick: impl FnOnce(f64) -> Result<f64, String>) -> Result<String, String> {
        if !(deck_bpm > 0.0) {
            return Err(format!("deck {deck} has no tempo"));
        }
        let t = self.info.as_ref().ok_or("no instrument loaded")?.tempo;
        let speed = self.voice.speed();
        let p = self.voice.params();
        let now = if p.motor { self.voice.settled_speed(p.motor_v) } else if speed > 0.2 { speed } else { t.design_mps };
        let bpm = t.bpm(now).ok_or("only a cord round a loop has a tempo")?;
        let target = pick(bpm)?;
        let v = t.speed_for_bpm(target).ok_or("only a cord round a loop has a tempo")?;
        if !(MOTOR_MIN_MPS * (1.0 - 1e-9)..=MOTOR_MAX_MPS * (1.0 + 1e-9)).contains(&v) {
            return Err(format!("{target:.0} BPM needs the cord at {v:.1} m/s, outside the motor's {MOTOR_MIN_MPS}–{MOTOR_MAX_MPS} m/s"));
        }
        let motor = self.voice.motor_for_speed(v);
        let mut params = self.voice.params().clone();
        params.motor = true;
        params.motor_v = motor;
        self.voice.set_params(&params);
        Ok(format!("{{\"deck\":\"{deck}\",\"deck_bpm\":{deck_bpm},\"target_bpm\":{target},\"motor_mps\":{motor}}}"))
    }

    /// `Wheel::sync`: the nearest whole multiple of the deck's tempo.
    pub fn sync(&mut self, deck: &str, deck_bpm: f64) -> Result<String, String> {
        self.aim(deck, deck_bpm, |bpm| Ok(windwheel::live::sync_bpm(bpm, deck_bpm)))
    }

    /// `Wheel::rung`: rung `index` of the deck's ladder (×1 at 0, ×2 at 1, ½ at −1).
    pub fn rung(&mut self, deck: &str, deck_bpm: f64, index: i64) -> Result<String, String> {
        let r = windwheel::live::Rung::at(index);
        self.aim(deck, deck_bpm, |_| Ok(deck_bpm * r.ratio()))
    }

    /// `Wheel::step`: one rung up or down, within the motor's range (Ok("null") at its end, as the booth's None).
    pub fn step(&mut self, deck: &str, deck_bpm: f64, up: bool) -> Result<String, String> {
        let range = self.info.as_ref().and_then(|i| i.tempo.bpm_range()).ok_or("only a cord round a loop has a tempo")?;
        const EDGE: &str = "the motor's end";
        match self.aim(deck, deck_bpm, |bpm| windwheel::live::step_rung(bpm, deck_bpm, up, range.0, range.1).map(|r| deck_bpm * r.ratio()).ok_or_else(|| EDGE.to_string())) {
            Err(e) if e == EDGE => Ok("null".into()),
            r => r,
        }
    }

    /// A `wheel` line's call, the voice's half. `bpm(deck)` is a deck's tempo now (the engine's), for `sync`, `step` and
    /// `interval beats`. Ok: what it did (JSON); Err: why not, as the booth would say it.
    pub fn call(&mut self, p: &Args, bpm: impl Fn(&str) -> f64) -> Result<String, String> {
        let deck = |s: &Self| p.str("deck").map(str::to_string).unwrap_or_else(|| s.follow.clone());
        match p.action() {
            "play" => {
                if !self.playing() {
                    self.play_pause();
                }
                Ok("{\"playing\":true}".into())
            }
            "pause" => {
                self.set_paused(true);
                Ok("{\"playing\":false}".into())
            }
            "toggle" => Ok(format!("{{\"playing\":{}}}", self.play_pause())),
            "stop" => {
                self.stop();
                Ok("{}".into())
            }
            "motor" => {
                let now = self.voice.params();
                let on = p.bool("on").unwrap_or(now.motor);
                let speed = p.num("speed").unwrap_or(now.motor_v);
                self.set_motor(on, speed);
                Ok("{}".into())
            }
            "level" => {
                self.set_volume(p.num("db").ok_or("level needs db=")?);
                Ok("{}".into())
            }
            "trigger" => {
                let on = p.bool("on").unwrap_or(true);
                self.set_trigger(on.then(|| p.num("pull").unwrap_or(0.5)));
                Ok("{}".into())
            }
            "pluck" => {
                self.pluck(p.num("tube").map(|t| t.max(0.0) as usize));
                Ok("{}".into())
            }
            "follow" => {
                let d = p.str("deck").ok_or("follow needs deck=")?;
                if !matches!(d, "A" | "B") {
                    return Err(format!("no deck {d}: the wheel follows A or B"));
                }
                self.follow = d.into();
                Ok("{}".into())
            }
            "interval" => {
                let s = match (p.num("seconds"), p.ratio("beats")) {
                    (Some(s), _) => s,
                    (None, Some(beats)) => {
                        let d = deck(self);
                        let b = bpm(&d);
                        if !(b > 0.0) {
                            return Err(format!("deck {d} has no tempo"));
                        }
                        // in hundredths, as the motor's knob takes it
                        (beats * 60.0 / b * 100.0).round() / 100.0
                    }
                    (None, None) => return Err("interval needs seconds= or beats=".into()),
                };
                self.set_interval(s);
                Ok(format!("{{\"seconds\":{}}}", self.voice.params().interval))
            }
            "sync" => {
                let d = deck(self);
                let b = bpm(&d);
                match p.ratio("ratio") {
                    None => self.sync(&d, b),
                    Some(r) if r >= 1.0 && (r - r.round()).abs() < 1e-9 => self.rung(&d, b, r.round() as i64 - 1),
                    Some(r) if r > 0.0 && r < 1.0 && ((1.0 / r) - (1.0 / r).round()).abs() < 1e-6 => self.rung(&d, b, 1 - (1.0 / r).round() as i64),
                    Some(_) => Err(format!("ratio {}: a whole multiple (2, 3) or a divisor (1/2, 1/4)", p.text("ratio"))),
                }
            }
            "step" => {
                let d = deck(self);
                let b = bpm(&d);
                self.step(&d, b, p.bool("up").unwrap_or(true))
            }
            "status" | "presets" => Ok("{}".into()),
            // the winding half's calls reach the voice as a wound instrument, not here
            a if winds(a) => Ok("{}".into()),
            a => Err(format!("wheel {a}")),
        }
    }

    /// A program's master, as the browser plays it on the decks (engine-calls.js: `master: filter` is both decks' FX
    /// unit `lp-freq` / `hp-freq`, `master: crush` its `crush-bits`, `master: reverb` both strips' `reverbWet`): the
    /// same on the wheel's own chain and reverb. In the booth the wheel's strip goes into the master bus, and the
    /// master's inserts are on it; here the master is the decks' own units, so the wheel gets units of its own, set
    /// alike. `name`: a master FX key and `value` its slider position 0..1, `reverbWet` and the wet 0..1, or `level`
    /// and the master level 0..1 (`mixer master=`).
    pub fn master(&mut self, name: &str, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("master {name}: not a number"));
        }
        if name == "level" {
            self.level = value.clamp(0.0, 1.0) as f32;
            return Ok(());
        }
        if name == "reverbWet" {
            self.reverb.set(djengine::fx::strip_reverb::ReverbKnob::Wet, value.clamp(0.0, 1.0));
            return Ok(());
        }
        let k = djengine::fx::master_macro::MasterFxKey::from_id(name).ok_or_else(|| format!("no master FX {name}"))?;
        let now = self.master.time_at(0);
        self.master.apply(k, k.slider_value(value) as i32, now);
        Ok(())
    }

    /// The voice's next `n` frames through the strip and the master, added to `left` and `right`.
    pub fn render_add(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        if !self.started && !self.sounding() {
            // never played: nothing in it, nothing in the master's tails
            return;
        }
        if self.l.len() < n {
            self.l.resize(n, 0.0);
            self.r.resize(n, 0.0);
        }
        for i in 0..n {
            if self.at == self.block.len() {
                self.next_block();
            }
            let mut x = [self.block[self.at]];
            self.at += 1;
            self.dc.process(&mut x);
            let a = self.fader * (x[0] * self.trim);
            self.l[i] = a;
            self.r[i] = a;
        }
        let (l, r) = (&mut self.l[..n], &mut self.r[..n]);
        self.master.process(l, Some(&mut *r), 0, n);
        self.master.finish(n);
        self.reverb.process(l, r);
        let m = self.level;
        for i in 0..n {
            left[i] += l[i] * m;
            right[i] += r[i] * m;
        }
    }

    /// The feeder's output, frame by frame (before the strip): `out.len()` frames, for the parity test against the
    /// booth's feeder.
    pub fn feed(&mut self, out: &mut [f32]) {
        for o in out.iter_mut() {
            if self.at == self.block.len() {
                self.next_block();
            }
            *o = self.block[self.at];
            self.at += 1;
        }
    }

    /// The feeder's body (djhost::wheel `Wheel::start`), one [`BLOCK`] at a time: held while paused (silence, the
    /// voice not clocked), else rendered, faded and made up for the strip. The voice is rendered in the booth's own
    /// block size because its crossfades between windings are kept block by block: rendered in other sizes, a re-wind's
    /// fade lands a few samples apart.
    fn next_block(&mut self) {
        self.at = 0;
        let out = &mut self.block[..];
        if self.paused && self.gain <= 0.0 {
            out.fill(0.0);
            return;
        }
        self.voice.render(out);
        self.frames += out.len() as u64;
        let target = if self.paused { 0.0 } else { 1.0 };
        if self.gain != target {
            let mut g = self.gain;
            for x in out.iter_mut() {
                g = if target > g { (g + self.fade_step).min(1.0) } else { (g - self.fade_step).max(0.0) };
                *x *= g;
            }
            self.gain = g;
        }
        let m = self.makeup;
        for x in out.iter_mut() {
            *x *= m;
        }
    }

    /// Whether a render would add anything: played or fading, a block not yet played out, or the DC blocker still
    /// settling.
    pub fn sounding(&self) -> bool {
        !(self.paused && self.gain <= 0.0 && self.dc.idle() && self.block[self.at..].iter().all(|x| *x == 0.0))
    }

    /// `{loaded, playing, started, paused, speed_mps, motor, motor_mps, frames}`.
    pub fn status(&self) -> String {
        let p = self.voice.params();
        format!(
            "{{\"loaded\":{},\"playing\":{},\"started\":{},\"paused\":{},\"speed_mps\":{},\"motor\":{},\"motor_mps\":{},\"frames\":{}}}",
            self.info.is_some(),
            self.playing(),
            self.started,
            self.paused,
            fin(self.voice.speed()),
            p.motor,
            fin(p.motor_v),
            self.frames
        )
    }
}

fn fin(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

// ---- a call's arguments ----

/// A `wheel` call's params (`{"action":"motor","on":true,"speed":1.05}`), read as the registry types them: numbers may
/// come as numbers or numeric text, booleans as `true`/`false` or `on`/`off`, a ratio as `1/4` or `0.25`.
pub struct Args(djengine::json::JsonValue);

impl Args {
    pub fn parse(json: &str) -> Result<Args, String> {
        let v = djengine::json::JsonValue::parse(json).map_err(|e| format!("wheel: {e}"))?;
        if v.as_object().is_none() {
            return Err("wheel: the call's params are not an object".into());
        }
        Ok(Args(v))
    }
    pub fn action(&self) -> &str {
        self.str("action").unwrap_or("status")
    }
    pub fn str(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(djengine::json::JsonValue::as_str)
    }
    fn text(&self, k: &str) -> String {
        self.0.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.canonical_js())).unwrap_or_default()
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        let v = self.0.get(k)?;
        v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|x: &f64| x.is_finite())
    }
    pub fn bool(&self, k: &str) -> Option<bool> {
        let v = self.0.get(k)?;
        v.as_bool().or_else(|| match v.as_str()? {
            "true" | "on" | "yes" | "1" => Some(true),
            "false" | "off" | "no" | "0" => Some(false),
            _ => None,
        }).or_else(|| v.as_f64().map(|x| x != 0.0))
    }
    /// `1/4`, `2`, `0.5`.
    pub fn ratio(&self, k: &str) -> Option<f64> {
        if let Some(x) = self.num(k) {
            return Some(x);
        }
        let s = self.str(k)?.trim();
        let (a, b) = s.split_once('/')?;
        let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        (b != 0.0).then(|| a / b).filter(|x| x.is_finite())
    }
}

// ---- the bytes between the two halves ----

struct Out(Vec<u8>);
impl Out {
    fn u8(&mut self, x: u8) {
        self.0.push(x);
    }
    fn u32(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn f64(&mut self, x: f64) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn b(&mut self, x: bool) {
        self.u8(x as u8);
    }
    fn s(&mut self, x: &str) {
        self.u32(x.len() as u32);
        self.0.extend_from_slice(x.as_bytes());
    }
    fn v(&mut self, x: &[f64]) {
        self.u32(x.len() as u32);
        for &y in x {
            self.f64(y);
        }
    }
}

struct In<'a>(&'a [u8]);
impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("wheel: a wound instrument cut short".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn b(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }
    fn s(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
    }
    fn v(&mut self) -> Result<Vec<f64>, String> {
        let n = self.u32()? as usize;
        (0..n).map(|_| self.f64()).collect()
    }
}

const MAGIC: &[u8; 4] = b"WHL1";

impl Wound {
    /// Bytes for the worklet (every number exactly: f64 and f32 as their bits).
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out(Vec::with_capacity(64 + self.table.data.len() * 4));
        o.0.extend_from_slice(MAGIC);
        o.s(&self.id);
        o.b(self.keep);
        o.b(self.cord);
        o.u32(self.tempo.onsets.map_or(0, |n| n + 1));
        o.f64(self.tempo.extent_m);
        o.f64(self.tempo.design_mps);
        let p = &self.params;
        o.u32(p.magnets as u32);
        for x in [p.r_coil, p.r_load, p.mass, p.friction, p.gain, p.trigger, p.motor_v, p.release, p.amplitude, p.interval] {
            o.f64(x);
        }
        for x in [p.braking, p.trigger_mode, p.motor] {
            o.b(x);
        }
        let r = &self.runtime;
        o.u8(LAWS.iter().position(|l| *l == r.law).map_or(255, |i| i as u8));
        o.b(r.cord);
        o.b(r.periodic);
        for x in [
            r.speed_mps, r.jitter, r.accel_mps2, r.release_m, r.interval_s, r.length_m, r.g_mps2, r.center_z, r.rest_z, r.frequency_hz, r.amplitude_m, r.damping_per_s, r.lo_z,
            r.hi_z, r.friction_per_s, r.follow_hz, r.max_speed_mps,
        ] {
            o.f64(x);
        }
        o.v(&r.start_z);
        o.v(&r.release_z);
        o.v(&r.end_z);
        let t = &self.table;
        o.f64(t.dz);
        o.f64(t.z0);
        o.b(t.periodic);
        o.f64(t.extent);
        o.u32(t.data.len() as u32);
        for &x in &t.data {
            o.0.extend_from_slice(&x.to_le_bytes());
        }
        o.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Wound, String> {
        let mut i = In(bytes);
        if i.take(4)? != MAGIC {
            return Err("wheel: not a wound instrument".into());
        }
        let id = i.s()?;
        let keep = i.b()?;
        let cord = i.b()?;
        let onsets = match i.u32()? {
            0 => None,
            n => Some(n - 1),
        };
        let tempo = Tempo { onsets, extent_m: i.f64()?, design_mps: i.f64()? };
        let magnets = i.u32()? as usize;
        let mut f = [0.0; 10];
        for x in f.iter_mut() {
            *x = i.f64()?;
        }
        let (braking, trigger_mode, motor) = (i.b()?, i.b()?, i.b()?);
        let params = Params {
            magnets: magnets.clamp(1, MAX_MAGNETS),
            r_coil: f[0],
            r_load: f[1],
            mass: f[2],
            braking,
            friction: f[3],
            gain: f[4],
            trigger_mode,
            trigger: f[5],
            motor,
            motor_v: f[6],
            release: f[7],
            amplitude: f[8],
            interval: f[9],
        };
        let law = *LAWS.get(i.u8()? as usize).ok_or("wheel: a motion law this player does not know")?;
        let (rcord, periodic) = (i.b()?, i.b()?);
        let mut g = [0.0; 17];
        for x in g.iter_mut() {
            *x = i.f64()?;
        }
        let runtime = Runtime {
            law,
            cord: rcord,
            periodic,
            speed_mps: g[0],
            jitter: g[1],
            accel_mps2: g[2],
            release_m: g[3],
            interval_s: g[4],
            length_m: g[5],
            g_mps2: g[6],
            center_z: g[7],
            rest_z: g[8],
            frequency_hz: g[9],
            amplitude_m: g[10],
            damping_per_s: g[11],
            lo_z: g[12],
            hi_z: g[13],
            friction_per_s: g[14],
            follow_hz: g[15],
            max_speed_mps: g[16],
            start_z: i.v()?,
            release_z: i.v()?,
            end_z: i.v()?,
        };
        let (dz, z0, tperiodic, extent) = (i.f64()?, i.f64()?, i.b()?, i.f64()?);
        let n = i.u32()? as usize;
        let raw = i.take(n * 4)?;
        let data = raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        if !i.0.is_empty() {
            return Err("wheel: trailing bytes after a wound instrument".into());
        }
        Ok(Wound { id, keep, cord, tempo, params, runtime, table: Table { data, dz, z0, periodic: tperiodic, extent } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wound_instrument_crosses_as_bytes_exactly() {
        for (id, _) in windwheel::presets::PRESETS {
            let mut w = Winder::new();
            let a = w.load(id).unwrap();
            let b = Wound::decode(&a.encode()).unwrap();
            assert_eq!(a.params, b.params, "{id}");
            assert_eq!(a.runtime, b.runtime, "{id}");
            assert_eq!(a.table.data, b.table.data, "{id}");
            assert_eq!((a.table.dz, a.table.z0, a.table.periodic, a.table.extent), (b.table.dz, b.table.z0, b.table.periodic, b.table.extent));
            assert_eq!((a.id, a.keep, a.cord, a.tempo), (b.id, b.keep, b.cord, b.tempo));
        }
        assert!(Wound::decode(b"WHL1").is_err());
        assert!(Wound::decode(b"nope").is_err());
    }

    #[test]
    fn a_root_without_an_octave_keeps_the_current_one() {
        assert_eq!(with_octave("F", "C3"), "F3");
        assert_eq!(with_octave("F#", "Bb2"), "F#2");
        assert_eq!(with_octave("Eb4", "C3"), "Eb4");
    }

    #[test]
    fn args_read_as_the_registry_types_them() {
        let a = Args::parse(r#"{"action":"sync","ratio":"1/4","on":"off","speed":"1.5","up":false}"#).unwrap();
        assert_eq!(a.ratio("ratio"), Some(0.25));
        assert_eq!(a.bool("on"), Some(false));
        assert_eq!(a.bool("up"), Some(false));
        assert_eq!(a.num("speed"), Some(1.5));
        assert_eq!(Args::parse("{}").unwrap().action(), "status");
    }
}
