//! `beatscript render`: a BeatScript program as raw interleaved stereo PCM on stdout, **streamed as it renders**.
//!
//! The same offline engine path as `djctl btw render` (`djhost::deck::offline`: the engine djapp plays
//! on, deterministic, each move on the exact frame of its beat, blocks of 128 frames), but the master is
//! not collected into a file: every block is converted and appended to a chunk, and each full chunk is
//! written and flushed at once, so the first bytes reach the reader after the engine has started and
//! `--chunk-ms` of audio (a smaller first chunk) has been rendered, never after the whole song.
//!
//! Start-up is kept small on purpose: the program is checked, the arrangement is made **lazily** (the
//! first section's decks are loaded before the first chunk; each later section's nets are built when the
//! render reaches the bar before it), and nothing is read or written but stdin / `--file`, stdout and
//! stderr. The engine is stepped from frame 0 whatever `--from-bar` says (a seek renders the first bars
//! and drops them), so the output of a seek is exactly the tail of the full render, and the chunk size
//! cannot change a byte: chunks are cut from the converted stream, never from the engine's stepping.
//!
//! stderr carries one JSON object a line (DESIGN.md "Render"): `start`, `progress` (at most 4 a
//! second), `end`, or `error` (then exit 1). See crates/beatscript-cli/RENDER.md.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use djengine::json::{JsonObject, JsonValue};
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{ArrangeEvent, ArrangeOp, Arrangement};
use djengine::session::DeckSource;
use djhost::beatscript_env::BeatscriptEnv;
use djhost::deck::offline::{arrangement_conductor, stand_on_start, ArrangeRenderOptions, OfflineConductor};
use djlive::ast::Deck;
use djlive::compile::{Compiled, DeckIr};

/// The browser engine's reading of the control calls (site/web/js/engine-calls.js `mapCall`), on this render's engine.
#[path = "browser.rs"]
pub mod browser;

/// Frames the engine renders in one step (`OfflineConductor::BLOCK_FRAMES`).
const BLOCK: usize = djhost::deck::offline::OfflineConductor::BLOCK_FRAMES;
/// Samples (both channels) of one block.
pub const BLOCK_SAMPLES: usize = 2 * BLOCK;
/// A program text larger than this is refused (the site stores 16 KiB; this is only a guard).
const MAX_TEXT_BYTES: usize = 1 << 20;
/// Progress lines are at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// The first chunk is at most this much audio, so the reader has bytes soon even when `--chunk-ms` is large.
const FIRST_CHUNK_MS: u32 = 60;

// ---------------------------------------------------------------------------------------------
// options

/// The sample format on stdout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    S16le,
    F32le,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::S16le => "s16le",
            Format::F32le => "f32le",
        }
    }
    /// Bytes of one frame (stereo).
    pub fn frame_bytes(self) -> usize {
        match self {
            Format::S16le => 4,
            Format::F32le => 8,
        }
    }
}

/// What `beatscript render` was asked for.
#[derive(Clone, Debug)]
pub struct Options {
    /// The program text's file (None: stdin; `-` also means stdin).
    pub file: Option<String>,
    pub sr: u32,
    pub format: Format,
    pub chunk_ms: u32,
    pub max_bars: u32,
    pub from_bar: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options { file: None, sr: 44_100, format: Format::S16le, chunk_ms: 250, max_bars: 256, from_bar: 0 }
    }
}

/// Why a render did not happen (or stopped): becomes the `error` event.
#[derive(Debug)]
pub struct RenderError {
    /// A short machine word: `usage`, `io`, `closed` (stdout's reader left), `check_failed`, `uses_assets`, `restricted`, `internal`.
    pub code: &'static str,
    pub message: String,
}

impl RenderError {
    fn new(code: &'static str, message: impl Into<String>) -> RenderError {
        RenderError { code, message: message.into() }
    }
}

impl From<std::io::Error> for RenderError {
    fn from(e: std::io::Error) -> RenderError {
        // the reader closed stdout: it asked for this, and the CLI ends quietly
        let code = if e.kind() == std::io::ErrorKind::BrokenPipe { "closed" } else { "io" };
        RenderError::new(code, e.to_string())
    }
}

/// `beatscript render` flags (the words after `render`) to [`Options`].
pub fn parse_args(args: &[String]) -> Result<Options, RenderError> {
    let mut o = Options::default();
    let usage = |m: String| RenderError::new("usage", m);
    let mut it = args.iter().peekable();
    // `beatscript render ...` may hand us its own name first
    if it.peek().map(|a| a.as_str()) == Some("render") {
        it.next();
    }
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, RenderError> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => it.next().cloned().ok_or_else(|| usage(format!("{name} needs a value"))),
            }
        };
        let num = |name: &str, v: String| -> Result<u32, RenderError> { v.parse::<u32>().map_err(|_| usage(format!("{name}: a whole number, not `{v}`"))) };
        match flag.as_str() {
            "--file" | "-f" => o.file = Some(value("--file")?),
            "-" => o.file = None,
            "--sr" => {
                let v = num("--sr", value("--sr")?)?;
                if !(8_000..=192_000).contains(&v) {
                    return Err(usage(format!("--sr {v}: between 8000 and 192000")));
                }
                o.sr = v;
            }
            "--format" => {
                o.format = match value("--format")?.as_str() {
                    "s16le" => Format::S16le,
                    "f32le" => Format::F32le,
                    other => return Err(usage(format!("--format {other}: s16le or f32le"))),
                }
            }
            "--chunk-ms" => {
                let v = num("--chunk-ms", value("--chunk-ms")?)?;
                if !(1..=60_000).contains(&v) {
                    return Err(usage(format!("--chunk-ms {v}: between 1 and 60000")));
                }
                o.chunk_ms = v;
            }
            "--max-bars" => {
                let v = num("--max-bars", value("--max-bars")?)?;
                if v == 0 {
                    return Err(usage("--max-bars 0: at least 1".into()));
                }
                o.max_bars = v;
            }
            "--from-bar" => o.from_bar = num("--from-bar", value("--from-bar")?)?,
            other => return Err(usage(format!("unknown argument `{other}` (beatscript render [--file F] [--sr N] [--format s16le|f32le] [--chunk-ms N] [--max-bars N] [--from-bar N])"))),
        }
    }
    Ok(o)
}

// ---------------------------------------------------------------------------------------------
// events

/// The JSON-line sink (stderr in the CLI).
pub struct Events<'a> {
    out: &'a mut dyn Write,
}

impl<'a> Events<'a> {
    pub fn new(out: &'a mut dyn Write) -> Events<'a> {
        Events { out }
    }

    fn emit(&mut self, pairs: Vec<(&str, JsonValue)>) {
        let line = JsonValue::Object(JsonObject::from_pairs(pairs)).js_stringified();
        // one write, so two lines never interleave; a closed stderr is not worth failing a render for
        let _ = self.out.write_all(format!("{line}\n").as_bytes());
        let _ = self.out.flush();
    }

    pub fn error(&mut self, e: &RenderError) {
        self.emit(vec![("event", "error".into()), ("code", e.code.into()), ("message", JsonValue::str(e.message.clone()))]);
    }
}

fn num(x: f64) -> JsonValue {
    JsonValue::Number((x * 1000.0).round() / 1000.0)
}

// ---------------------------------------------------------------------------------------------
// the arrangement, lazily

/// What a section sets going, in the order `djhost::beatscript_render::arrangement` makes its events: the first
/// section's decks are the arrangement's start; each later one loads its nets a bar ahead (so they land on
/// its bar line), starts a deck that was not playing and stops one it does not use. The events come out in
/// beat order already (a section's are at `4·start − 2`, `− 1`, `0`; the next section starts at least one
/// bar on), and a test compares the whole run with `beatscript_render::arrangement`'s.
struct Sections<'a> {
    sets: Vec<(u32, &'a BTreeMap<Deck, DeckIr>)>,
    bpm: f64,
    /// The next section to emit events for (>= 1).
    k: usize,
    pending: std::collections::VecDeque<ArrangeEvent>,
    /// The parts' `kit=` / `sound=` (deck, role, option) of each `Load` in `pending`, in the same order: the page sets them once the
    /// net has landed (engine.js `sound` ops), the render right after the load.
    sounds: std::collections::VecDeque<Vec<(String, String, String)>>,
    /// Beat of the last move (the song's end).
    beats: f64,
    done: bool,
}

impl<'a> Sections<'a> {
    fn new(c: &'a Compiled, bars: u32) -> Sections<'a> {
        let sets: Vec<(u32, &BTreeMap<Deck, DeckIr>)> = if c.arranged && !c.segments.is_empty() { c.segments.iter().map(|s| (s.start, &s.decks)).collect() } else { vec![(0, &c.decks)] };
        Sections { sets, bpm: c.tempo.unwrap_or(120.0), k: 1, pending: Default::default(), sounds: Default::default(), beats: f64::from(bars) * 4.0, done: false }
    }

    /// The decks of the first section: the arrangement's start.
    fn start(&self) -> (BTreeMap<String, DeckSource>, Vec<String>) {
        let mut decks = BTreeMap::new();
        let mut playing = Vec::new();
        for (d, ir) in self.sets[0].1.iter() {
            decks.insert(d.name().to_string(), DeckSource::RawNets(ir.envelope(0)));
            playing.push(d.name().to_string());
        }
        playing.sort();
        (decks, playing)
    }

    fn fill(&mut self) {
        if self.k < self.sets.len() {
            let k = self.k;
            self.k += 1;
            let (start, nets) = (self.sets[k].0, self.sets[k].1);
            let prev = self.sets[k - 1].1;
            let bpm = self.bpm;
            for (d, ir) in nets.iter() {
                let src = DeckSource::RawNets(ir.envelope(0));
                self.pending.push_back(ArrangeEvent { at: f64::from(start) * 4.0 - 2.0, bpm, op: ArrangeOp::Deck(DeckAction::Load { deck: d.name().to_string(), source: src }), via: "btw".into(), text: format!("section at bar {start}: deck {}", d.name()) });
                self.sounds.push_back(sounds_of(d, ir));
                // (on the section's bar line, as djhost::beatscript_render's: a deck that is not playing starts its net when it plays)
                if !prev.contains_key(d) {
                    self.pending.push_back(ArrangeEvent { at: f64::from(start) * 4.0, bpm, op: ArrangeOp::Deck(DeckAction::Play { deck: d.name().to_string(), on: true }), via: "btw".into(), text: format!("deck {} in", d.name()) });
                }
            }
            for d in prev.keys() {
                if !nets.contains_key(d) {
                    self.pending.push_back(ArrangeEvent { at: f64::from(start) * 4.0, bpm, op: ArrangeOp::Deck(DeckAction::Play { deck: d.name().to_string(), on: false }), via: "btw".into(), text: format!("deck {} out", d.name()) });
                }
            }
        } else if !self.done {
            self.done = true;
            // the render runs to the last move's bar: a last event at the song's end keeps it as long as the song
            self.pending.push_back(ArrangeEvent { at: self.beats - 0.01, bpm: self.bpm, op: ArrangeOp::Live { method: "cue".into(), params: JsonValue::Object(Default::default()) }, via: "btw".into(), text: "the end".into() });
        }
    }

    fn next(&mut self) -> Option<ArrangeEvent> {
        while self.pending.is_empty() && !self.done {
            self.fill();
        }
        self.pending.pop_front()
    }

    /// The first section's `kit=` / `sound=`, deck by deck.
    fn start_sounds(&self) -> Vec<(String, String, String)> {
        self.sets[0].1.iter().flat_map(|(d, ir)| sounds_of(d, ir)).collect()
    }
}

/// A deck's parts' `kit=` / `sound=` as the runtime hands them to the page (`{op:'sound', deck, role, option}`).
fn sounds_of(d: &Deck, ir: &DeckIr) -> Vec<(String, String, String)> {
    ir.sounds().into_iter().map(|(role, opt)| (d.name().to_string(), role, opt)).collect()
}

/// The whole arrangement `Sections` makes, as `djhost::beatscript_render::arrangement` would (for the test that
/// pins the two together).
#[doc(hidden)]
#[allow(dead_code)] // the tests
pub fn full_arrangement(c: &Compiled) -> (Arrangement, f64) {
    let bars = if c.arranged { c.song_bars().unwrap_or(16) } else { 16 };
    let beats = f64::from(bars) * 4.0;
    let mut s = Sections::new(c, bars);
    let (decks, playing) = s.start();
    let mut events = Vec::new();
    while let Some(e) = s.next() {
        events.push(e);
    }
    let mut a = Arrangement::new(s.bpm, decks, events);
    a.start.playing = Some(playing);
    (a, beats)
}

// ---------------------------------------------------------------------------------------------
// the control lines

/// One control event on the render's timeline.
#[derive(Clone)]
enum Move {
    /// A mixer, transport, duck or mix move, as `djhost::beatscript_render::control_events` makes it for the desktop's render: a
    /// strip's level, EQ and mute, the master level, the crossfader, ramps and lfos sampled the way the runtime sends them.
    Arrange(ArrangeEvent),
    /// A registry call the arrangement has no op for (an insert, the FX unit's drive, the vocoder, the Wheel), as the runtime makes
    /// it (`djhost::beatscript_render::control_calls`), played as the browser's engine plays it ([`browser::Browser`]); what the
    /// page has no op for, the booth's way.
    Call(djhost::beatscript_render::TimedCall),
    /// What the program's control net fires (`djlive::nets`): a `macro` line's `effects fire` / `release` (the net's other events
    /// are the calls' and the moves' here). With the beat it is on and the line (its control's key) it is of.
    Net(djlive::nets::Event, f64, Option<String>),
}

/// A `macro` line's call as the render made it (docs/beatscript/ext-control.md): `listen` reports each one, with what the engine said.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroCall {
    /// The beat the call was made on (a beat ahead of the bar line its line names: the engine starts it on the next one).
    pub beat: f64,
    /// The beat the engine starts it on: the target deck's next bar line after the call, read off the deck's own clock when the call
    /// is made (what `engage` does), so a deck whose bars are not the program's would show here.
    pub start_beat: f64,
    pub target: String,
    pub id: String,
    pub bars: Option<u32>,
    pub latch: bool,
    /// A held one-shot (a tape stop): the beat it was let go on.
    pub released_beat: Option<f64>,
    /// The line of the program text (1-based) and its text, and its control's key.
    pub line: usize,
    pub text: String,
    pub key: String,
    /// Why the engine refused it, when it did (a net macro on a deck that plays no net).
    pub refused: Option<String>,
}

impl MacroCall {
    /// The program bar the engine starts it on, in bars (whole when the deck's bars are the program's). A call the engine refused
    /// starts nowhere: this is the bar it asked for, the program's next bar line after the call.
    pub fn starts_at(&self) -> f64 {
        if self.refused.is_some() {
            let b = self.beat / 4.0;
            return if (b - b.round()).abs() < 1e-9 { b.round() + 1.0 } else { b.ceil() };
        }
        self.start_beat / 4.0
    }

    /// The program bar (0-based) it starts in.
    pub fn starts_bar(&self) -> u32 {
        (self.starts_at() + 1e-6).floor().max(0.0) as u32
    }

    /// How many bars a held one-shot was held (from its start to its release).
    pub fn held_bars(&self) -> Option<f64> {
        self.released_beat.map(|r| (r - self.start_beat) / 4.0)
    }
}

/// The beat (on the render's own clock, `bpm`) the engine starts an engagement asked for at `frame` on `deck`: its next bar line at
/// least the engine's lead ahead (`deck_grid_frame`'s rule), or at once when it is not playing.
fn engagement_start(engine: &djengine::host::deck::engine::Engine, deck: &str, frame: i64, bpm: f64) -> f64 {
    const LEAD_FRAMES: f64 = 384.0;
    let sr = engine.sample_rate;
    let at = |f: f64| f / (60.0 / bpm * sr);
    let Some(d) = engine.deck(deck) else { return at(frame as f64) };
    let Some(s) = d.beat_state(sr).filter(|s| s.bpm > 0.0 && d.playing) else { return at(frame as f64) };
    let fpb = 60.0 * sr / s.bpm;
    let mut ahead = ((s.beat / 4.0).ceil() * 4.0 - s.beat) * fpb;
    if ahead < LEAD_FRAMES {
        ahead += 4.0 * fpb;
    }
    at(frame as f64 + ahead.round())
}

/// The program's control lines as one timeline, in frames: the arrangement's control moves first on a tie, then the net's,
/// then the DJ moves the net leaves beside it (`djhost::offline_moves`: the booth's scheduler, serviced on every block).
/// Built once, before the first chunk, for the whole program (not `--max-bars` or `--from-bar`), so a cut or a seek hears
/// exactly the moves the full render does up to that point.
pub struct Controls {
    moves: Vec<(i64, u8, usize, Move)>,
    next: usize,
    /// Control lines the render does not hear (neither the arrangement nor the net carries them): for the start event's notes.
    unheard: Vec<String>,
    /// A macro line's place for its call: its control's key to (line, text).
    macro_lines: BTreeMap<String, (usize, String)>,
    /// The tempo the render's clock runs at (a frame is a beat at it).
    bpm: f64,
    /// The render's sample rate (the moves' scheduler reads beats off frames at it).
    sr: f64,
    /// The macro calls made so far, with what the engine said.
    pub macros: Vec<MacroCall>,
    /// Macro calls on a deck that did not play yet when they came due (a deck that comes in on the bar the macro starts on): the
    /// frame of that bar, and the move. Made just before the deck starts (the engine starts an engagement on a deck that does not
    /// play at once, so it starts with the deck), or at that bar if the deck never comes.
    deferred: Vec<(i64, usize)>,
    /// The program's `move` lines, armed on the render's engine with the booth's scheduler (None: the program has none).
    perform: Option<djhost::offline_moves::ProgramMoves>,
    /// The page's engine around this one: the calls' ops, and the Wheel beside the decks.
    pub browser: browser::Browser,
    /// Whether the Wheel is added to the master (the stems' passes leave it out of a deck's or a part's stem).
    pub wheel: bool,
}

/// The calls a render plays as calls ([`Move::Call`]): the nouns the arrangement format has no op for, which the browser's
/// engine (and, for what it does not have, the booth's registry) plays; and a `mixer` call with a field the arrangement does not
/// carry (a strip's `gain` or `filter` knob).
fn taken_as_call(x: &djlive::compile::Call) -> bool {
    match x.noun.as_str() {
        "inserts" | "fx" | "vocoder" | "wheel" => true,
        "mixer" => x.params.get("gain").is_some() || x.params.get("filter").is_some(),
        _ => false,
    }
}

/// Whether a render plays every call of a control line: the arrangement's (the mixer, a deck's play and stop, `duck`, `mix`), a
/// call the browser's engine plays or the booth's chain takes (an insert), the vocoder and the Wheel, a move the booth's scheduler
/// plays, or a `macro` line (the net). A line with a call none of them has is named in the start event's notes.
fn heard(ctl: &djlive::compile::Control, moves: &djhost::offline_moves::ProgramMoves) -> bool {
    use djlive::compile::Effect;
    let e = match &ctl.effect {
        Effect::Performed(p) => match p.at_half() {
            Some(e) => e,
            None => return true,
        },
        e => e.clone(),
    };
    let calls: Vec<djlive::compile::Call> = match &e {
        Effect::Calls(cs) => cs.clone(),
        Effect::Ramp(r) => r.prelude.iter().cloned().chain([r.at(0.0)]).collect(),
        Effect::Lfo(l) => l.prelude.iter().cloned().chain([l.call_at(0.0)]).collect(),
        Effect::Duck(_) | Effect::Mix(_) | Effect::Performed(_) => return true,
    };
    calls.iter().all(|x| {
        let p = &x.params;
        let action = p.get("action").and_then(JsonValue::as_str);
        match x.noun.as_str() {
            "mixer" => ["volume", "low", "mid", "high", "mute", "master", "crossfader", "gain", "filter"].iter().any(|k| p.get(k).is_some()),
            "deck" => matches!(action, Some("play" | "stop" | "pause")),
            "inserts" | "wheel" => true,
            "fx" => browser::map_call("fx", p).ignored.is_none(),
            "vocoder" => browser::map_call("vocoder", p).ignored.is_none(),
            "moves" => moves.plays(&ctl.key),
            "effects" => matches!(action, Some("fire" | "release")),
            _ => false,
        }
    })
}

/// The program's next bar line after `beat`, in beats (a bar line itself: the next one).
fn next_bar_beat(beat: f64) -> f64 {
    let b = beat / 4.0;
    4.0 * if (b - b.round()).abs() < 1e-9 { b.round() + 1.0 } else { b.ceil() }
}

impl Controls {
    /// The control lines of `c` in a render of `program_bars` bars at `sr`, on the program's tempo.
    pub fn new(c: &Compiled, program_bars: u32, sr: f64) -> Controls {
        let beats = f64::from(program_bars) * 4.0;
        let bpm = c.tempo.unwrap_or(120.0);
        let frame_of = |beat: f64| -> i64 { (beat * 60.0 / bpm * sr).round() as i64 };
        let mut moves: Vec<(i64, u8, usize, Move)> = Vec::new();
        let browser = browser::Browser::new(sr);
        if c.controls.is_empty() {
            return Controls { moves, next: 0, unheard: Vec::new(), macro_lines: BTreeMap::new(), bpm, sr, macros: Vec::new(), deferred: Vec::new(), perform: None, browser, wheel: true };
        }
        // the mixer and the transport, `duck` and `mix`: the arrangement's events, as the desktop's render makes them
        let (arr, left_out) = djhost::beatscript_render::control_events(c, bpm, beats, true);
        for (i, e) in arr.into_iter().enumerate() {
            moves.push((frame_of(e.at), 0, i, Move::Arrange(e)));
        }
        // the calls the arrangement has no op for, as the runtime makes them, for the browser's engine to play
        for (i, call) in djhost::beatscript_render::control_calls(c, beats, &taken_as_call).into_iter().enumerate() {
            moves.push((frame_of(call.beat), 1, i, Move::Call(call)));
        }
        // the control net's macro lines (`effects fire` / `release`): the booth's DJ catalogue (the page has none yet)
        let net = djlive::nets::control_net(c, program_bars);
        for (i, (tick, key, e)) in net.events_keyed(None).into_iter().enumerate() {
            if !matches!(e, djlive::nets::Event::Fire { .. } | djlive::nets::Event::Release { .. }) {
                continue;
            }
            let beat = tick as f64 / 4.0;
            if beat >= beats {
                continue;
            }
            moves.push((frame_of(beat), 2, i, Move::Net(e, beat, key)));
        }
        moves.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
        // every move line on the booth's scheduler (the page's MoveScheduler is its port): a move starts on the line after the one
        // that armed it, and a vocoder handoff swaps the vocoder this render plays
        let perform = djhost::offline_moves::ProgramMoves::every(c, beats, djhost::offline_moves::Clock::steady(bpm, sr), true);
        // unheard: a line with a call none of these plays (a vocal, a `do` to another noun, a move that cannot act offline)
        let unplayed: BTreeSet<usize> = c
            .controls
            .iter()
            .filter(|ctl| !heard(ctl, &perform))
            .map(|ctl| djlive::diag::line_col(&c.source, djhost::beatscript_render::place_of(c, ctl).0).0)
            .collect();
        let unheard = left_out
            .lines
            .iter()
            .filter(|l| unplayed.contains(&l.line))
            .map(|l| {
                let why = c.controls.iter().find(|ctl| djlive::diag::line_col(&c.source, djhost::beatscript_render::place_of(c, ctl).0).0 == l.line).and_then(|ctl| perform.why_unplayed(&ctl.key));
                match why {
                    Some(w) => format!("line {} `{}` ({}: {w})", l.line, l.text, l.kind),
                    None => format!("line {} `{}` ({})", l.line, l.text, l.kind),
                }
            })
            .collect();
        let perform = (!perform.is_empty()).then_some(perform);
        let mut macro_lines = BTreeMap::new();
        for ctl in c.controls.iter().filter(|ctl| matches!(ctl.effect.base(), djlive::compile::Effect::Calls(cs) if cs.iter().any(|x| x.noun == "effects"))) {
            let line = djlive::diag::line_col(&c.source, ctl.span.start).0;
            let text = ctl.span.text(&c.source).lines().next().unwrap_or("").trim().to_string();
            macro_lines.insert(ctl.key.clone(), (line, text));
        }
        Controls { moves, next: 0, unheard, macro_lines, bpm, sr, macros: Vec::new(), deferred: Vec::new(), perform, browser, wheel: true }
    }

    fn frame_of(&self, beat: f64) -> i64 {
        (beat * 60.0 / self.bpm * self.sr).round() as i64
    }

    /// The next frame after `frame` a control line is due on: the timeline's next move, a deferred macro's bar, or what the DJ
    /// moves do next.
    fn peek(&self, frame: i64, engine: &djengine::host::deck::engine::Engine) -> Option<i64> {
        let line = self.moves.get(self.next).map(|m| m.0);
        let deferred = self.deferred.iter().map(|(f, _)| *f).min();
        let moves = self.perform.as_ref().and_then(|p| p.next_frame(engine, frame, &|b| self.frame_of(b)));
        [line, deferred, moves].into_iter().flatten().min()
    }

    /// Apply every move due at or before `frame` (a macro on a deck not playing yet waits for it), then the DJ moves' pass on the
    /// render's clock.
    fn apply_due(&mut self, frame: i64, engine: &mut djengine::host::deck::engine::Engine) {
        while let Some((f, ..)) = self.moves.get(self.next) {
            if *f > frame {
                break;
            }
            let i = self.next;
            self.next += 1;
            // a macro on a deck that does not play yet waits for it (`deferred`)
            if let Move::Net(djlive::nets::Event::Fire { target, .. }, beat, _) = &self.moves[i].3 {
                if engine.deck(target).is_some_and(|d| !d.playing) {
                    let deadline = (next_bar_beat(*beat) * 60.0 / self.bpm * engine.sample_rate).round() as i64;
                    self.deferred.push((deadline, i));
                    continue;
                }
            }
            self.apply_move(i, frame, engine);
        }
        self.apply_deferred(None, frame, engine);
        // the moves: the scheduler's pass on the render's clock, every block (after the net's events, as beatscript_render's)
        let (bpm, sr) = (self.bpm, self.sr);
        if let Some(p) = self.perform.as_mut() {
            p.run(engine, frame, &|b| (b * 60.0 / bpm * sr).round() as i64);
        }
    }

    /// The deferred macro calls on `deck`, just before it starts to play; with None, those whose bar has come (the deck never did).
    pub fn apply_deferred(&mut self, deck: Option<&str>, frame: i64, engine: &mut djengine::host::deck::engine::Engine) {
        let mut k = 0;
        while k < self.deferred.len() {
            let (deadline, i) = self.deferred[k];
            let hit = match deck {
                Some(d) => matches!(&self.moves[i].3, Move::Net(djlive::nets::Event::Fire { target, .. }, ..) if target == d),
                None => deadline <= frame,
            };
            if hit {
                self.deferred.remove(k);
                self.apply_move(i, frame, engine);
            } else {
                k += 1;
            }
        }
    }

    fn apply_move(&mut self, i: usize, frame: i64, engine: &mut djengine::host::deck::engine::Engine) {
        let m = self.moves[i].3.clone();
        {
            let f = &frame;
            match &m {
                // refusals are not the listener's business (the desktop's render counts them; a stream has no report)
                Move::Arrange(e) => match &e.op {
                    ArrangeOp::Live { method, params } => {
                        let _ = engine.apply_live_op(method, params);
                    }
                    op => {
                        // the page gives the program's master level to the Wheel too
                        if let ArrangeOp::Deck(DeckAction::Master(v)) = op {
                            self.browser.master_level(*v);
                        }
                        let _ = engine.apply_arrange_op(op);
                    }
                },
                Move::Call(t) => {
                    let m = self.browser.call(engine, &t.call.noun, &t.call.params);
                    if m.ignored.is_some() {
                        self.browser.booth(engine, &t.call.noun, &t.call.params);
                    }
                }
                Move::Net(e, beat, key) => {
                    // a macro's call is kept, with where the engine starts it and what it said, for `listen`; a held one's release too
                    let start = match e {
                        djlive::nets::Event::Fire { target, .. } => Some(engagement_start(engine, target, *f, self.bpm)),
                        _ => None,
                    };
                    let r = djhost::beatscript_render::apply_event(engine, e);
                    let key = key.clone().unwrap_or_default();
                    match e {
                        djlive::nets::Event::Fire { target, id, bars, latch } => {
                            let (line, text) = self.macro_lines.get(&key).cloned().unwrap_or_default();
                            let start_beat = start.unwrap_or(*beat);
                            self.macros.push(MacroCall { beat: *beat, start_beat, target: target.clone(), id: id.clone(), bars: *bars, latch: *latch, released_beat: None, line, text, key, refused: r.err() });
                        }
                        djlive::nets::Event::Release { target, id } => {
                            if let Some(m) = self.macros.iter_mut().rev().find(|m| m.key == key && &m.target == target && &m.id == id && m.released_beat.is_none()) {
                                m.released_beat = Some(*beat);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the plan: what to render, from the check

/// What a program is to render as, from its check.
pub struct Plan {
    pub bpm: f64,
    /// Bars the program is (an arranged song: its timeline once; `end at bar N`; else 16).
    pub program_bars: u32,
    /// Bars the render covers: `program_bars` capped at `--max-bars`.
    pub bars: u32,
    #[allow(dead_code)]
    pub arranged: bool,
    pub notes: Vec<String>,
}

/// The rights and the shape of `c`, or why it is not rendered.
pub fn plan(c: &Compiled, o: &Options) -> Result<Plan, RenderError> {
    if c.rights.restricted() {
        return Err(RenderError::new("restricted", "this program is restricted (`use restricted`, `#! visibility: private` or a restricted asset): it is private and beatscript does not render it"));
    }
    if c.rights.uses_assets() {
        return Err(RenderError::new("uses_assets", "this program uses samples or loops (`sample` / `loop \"...\" as`): beatscript has no files and does not render them"));
    }
    let bpm = c.tempo.unwrap_or(120.0);
    let program_bars = djhost::beatscript_render::program_bars(c);
    let bars = program_bars.min(o.max_bars);
    let mut notes = Vec::new();
    if c.looped && c.arranged {
        notes.push("the song loops: the render is one pass through its sections".to_string());
    }
    if bars < program_bars {
        notes.push(format!("cut at --max-bars: {bars} of the program's {program_bars} bars"));
    }
    if !c.arranged && c.end.is_none() {
        notes.push("the program has no end: 16 bars".to_string());
    }
    Ok(Plan { bpm, program_bars, bars, arranged: c.arranged, notes })
}

// ---------------------------------------------------------------------------------------------
// the sink: samples to bytes, bytes to chunks

struct Sink<'a> {
    out: &'a mut dyn Write,
    format: Format,
    buf: Vec<u8>,
    /// Frames the current chunk may hold before it is written.
    limit: usize,
    steady: usize,
    frames_out: u64,
    first_byte: Option<Duration>,
    t0: Instant,
    wrote_any: bool,
}

impl<'a> Sink<'a> {
    fn push(&mut self, l: &[f32], r: &[f32]) -> std::io::Result<()> {
        match self.format {
            Format::S16le => {
                for (a, b) in l.iter().zip(r) {
                    self.buf.extend_from_slice(&s16(*a).to_le_bytes());
                    self.buf.extend_from_slice(&s16(*b).to_le_bytes());
                }
            }
            Format::F32le => {
                for (a, b) in l.iter().zip(r) {
                    self.buf.extend_from_slice(&a.to_bits().to_le_bytes());
                    self.buf.extend_from_slice(&b.to_bits().to_le_bytes());
                }
            }
        }
        self.frames_out += l.len() as u64;
        if self.buf.len() >= self.limit * self.format.frame_bytes() {
            self.flush_chunk()?;
        }
        Ok(())
    }

    fn flush_chunk(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.out.write_all(&self.buf)?;
        self.out.flush()?;
        if self.first_byte.is_none() {
            self.first_byte = Some(self.t0.elapsed());
        }
        self.wrote_any = true;
        self.buf.clear();
        self.limit = self.steady;
        Ok(())
    }
}

/// A float sample as the WAV writer makes it (`djrust::wav::wav_bytes`): clamp to -1..1, negative
/// scaled by 32768 and the rest by 32767, rounding half away from zero.
fn s16(x: f32) -> i16 {
    let c = x.clamp(-1.0, 1.0);
    let v = if c < 0.0 { (c * 32768.0).round() } else { (c * 32767.0).round() };
    v as i16
}

// ---------------------------------------------------------------------------------------------
// the render

/// What a finished render made.
#[derive(Debug, Clone)]
#[allow(dead_code)] // read by the tests
pub struct Summary {
    pub frames: u64,
    pub seconds: f64,
    pub ttfb_ms: f64,
    pub render_ms: f64,
    pub bars: u32,
    /// The macro lines' calls the render made (`listen` reports them).
    pub macros: Vec<MacroCall>,
}

/// Check `text` and stream its render: PCM to `pcm`, events to `ev`. `t0` is when the process (or the
/// call) began, for the time to the first byte.
pub fn render_to(text: &str, o: &Options, pcm: &mut dyn Write, ev: &mut Events, t0: Instant) -> Result<Summary, RenderError> {
    let c = check_text(text)?;
    let check_ms = t0.elapsed().as_secs_f64() * 1000.0;
    render_compiled(&c, check_ms, o, pcm, ev, t0, &mut NoTap)
}

/// Check `text` as the render does: the programs it was told (`--with`; on the server, the store's) and no names (beatscript reads
/// nothing from disk), the real registry and instrument catalogue.
pub fn check_text(text: &str) -> Result<Compiled, RenderError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(RenderError::new("usage", format!("the program is {} bytes: at most {MAX_TEXT_BYTES}", text.len())));
    }
    // a `gen@2 ...` call resolves against the seeded generator, as the check does (beatscript-api installs it with its web env)
    beatscript_api::generate::install();
    let mut env = BeatscriptEnv::new(Some(&beatscript_api::known::Known));
    // a render sings a `sing` line as the browser does, in its deck's net (it has no vocal noun to call)
    env.voices.in_nets = true;
    let checked = djlive::compile::check_with(text, &env.env());
    let Some(c) = checked.compiled else {
        let errs: Vec<_> = checked.diagnostics.iter().filter(|d| d.is_error()).cloned().collect();
        let shown = djlive::diag::render_all(&errs, text);
        return Err(RenderError::new("check_failed", format!("the program has {} error{}: {}", errs.len(), if errs.len() == 1 { "" } else { "s" }, shown.trim())));
    };
    Ok(c)
}

/// What watches a render from the inside (`listen`): it sees the conductor before the first block and after each block,
/// with that block's master. The render's own output does not change with one.
pub trait Tap {
    /// Once, the engine on the first section, before the first block. `notes` are the render's notes (what it leaves out).
    fn start(&mut self, _cond: &mut OfflineConductor, _plan: &Plan, _notes: &[String]) {}
    /// After each step: the master of the `l.len()` frames that began at engine frame `at`.
    fn block(&mut self, _cond: &mut OfflineConductor, _at: i64, _l: &[f32], _r: &[f32]) {}
}

/// No tap: the plain render.
pub struct NoTap;
impl Tap for NoTap {}

/// [`render_to`] of a program already checked (`c` may be one `listen` made from a checked one), watched by `tap`.
pub fn render_compiled(c: &Compiled, check_ms: f64, o: &Options, pcm: &mut dyn Write, ev: &mut Events, t0: Instant, tap: &mut dyn Tap) -> Result<Summary, RenderError> {
    let plan = plan(c, o)?;
    let sr = f64::from(o.sr);
    let bpm = plan.bpm;
    let frame_of = |beat: f64| -> i64 { (beat * 60.0 / bpm * sr).round() as i64 };
    let want = frame_of(f64::from(plan.bars) * 4.0).max(1);
    if o.from_bar >= plan.bars {
        return Err(RenderError::new("usage", format!("--from-bar {}: the render has {} bar{}", o.from_bar, plan.bars, if plan.bars == 1 { "" } else { "s" })));
    }
    let skip = frame_of(f64::from(o.from_bar) * 4.0).min(want);
    // the control lines: levels, mixer moves and ramps (the desktop's arrangement events), inserts and their ramps (the net)
    let mut ctls = Controls::new(c, plan.program_bars, sr);
    let mut notes = plan.notes.clone();
    if !ctls.unheard.is_empty() {
        let n = ctls.unheard.len();
        let mut shown: Vec<String> = ctls.unheard.iter().take(3).cloned().collect();
        if n > 3 {
            shown.push(format!("{} more", n - 3));
        }
        notes.insert(0, format!("not in this render: {n} control line{} (no offline op for it yet): {}", if n == 1 { "" } else { "s" }, shown.join(", ")));
    }
    let out_frames = (want - skip) as u64;
    let seconds = out_frames as f64 / sr;

    let steady = ((u64::from(o.chunk_ms) * u64::from(o.sr)) / 1000).max(1) as usize;
    let first = (((u64::from(o.chunk_ms.min(FIRST_CHUNK_MS))) * u64::from(o.sr)) / 1000).max(1) as usize;
    let sink = Sink { out: pcm, format: o.format, buf: Vec::with_capacity(steady * o.format.frame_bytes()), limit: first, steady, frames_out: 0, first_byte: None, t0, wrote_any: false };
    let start = vec![
        ("event", "start".into()),
        ("sample_rate", JsonValue::int(i64::from(o.sr))),
        ("channels", JsonValue::int(2)),
        ("format", o.format.name().into()),
        ("seconds", num(seconds)),
        ("bars", JsonValue::int(i64::from(plan.bars - o.from_bar))),
        ("bpm", num(bpm)),
        ("from_bar", JsonValue::int(i64::from(o.from_bar))),
        ("total_bars", JsonValue::int(i64::from(plan.program_bars))),
        ("cid", JsonValue::str(c.cid.clone())),
        ("notes", JsonValue::Array(notes.iter().cloned().map(JsonValue::str).collect())),
    ];
    let mut d = Stream { sink, ev, start: Some(start), t0, skip, want, sr, engine_ms: 0.0, ready_ms: 0.0, last_progress: None, tap, plan: &plan, notes, tap_l: Vec::with_capacity(BLOCK), tap_r: Vec::with_capacity(BLOCK) };
    drive(&c, &plan, sr, want, &mut ctls, &mut d)?;
    let Stream { mut sink, ev, engine_ms, ready_ms, .. } = d;
    sink.flush_chunk()?;
    let render_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let ttfb_ms = sink.first_byte.map_or(render_ms, |d| d.as_secs_f64() * 1000.0);
    let frames = sink.frames_out;
    ev.emit(vec![
        ("event", "end".into()),
        ("seconds", num(frames as f64 / sr)),
        ("frames", JsonValue::int(frames as i64)),
        ("check_ms", num(check_ms)),
        ("engine_ms", num(engine_ms)),
        ("ready_ms", num(ready_ms)),
        ("ttfb_ms", num(ttfb_ms)),
        ("render_ms", num(render_ms)),
    ]);
    Ok(Summary { frames, seconds: frames as f64 / sr, ttfb_ms, render_ms, bars: plan.bars, macros: std::mem::take(&mut ctls.macros) })
}

/// What [`drive`] hands the one who runs it: the engine once it stands on the first section, every block it
/// renders, and the engine again between blocks. The render (`render_to`) streams the blocks; the export (export.rs)
/// reads the notes the engine plays and mutes lanes for a stem.
pub trait Driver {
    /// The conductor is made (the decks not loaded yet).
    fn built(&mut self) {}
    /// The arrangement's start stands: the first section's decks are loaded, nothing is rendered.
    fn ready(&mut self, _cond: &mut OfflineConductor) -> Result<(), RenderError> {
        Ok(())
    }
    /// Frames `at .. at + l.len()` of the render.
    fn block(&mut self, at: i64, l: &[f32], r: &[f32]) -> Result<(), RenderError>;
    /// After every step: the engine between two blocks.
    fn after(&mut self, _cond: &mut OfflineConductor, _produced: i64) {}
}

/// The render's loop, once for every caller: `want` frames of `c` (`plan`) at `sr`, every section's moves and every
/// control line on its exact frame, in blocks of at most [`BLOCK`].
pub fn drive(c: &Compiled, plan: &Plan, sr: f64, want: i64, ctls: &mut Controls, d: &mut dyn Driver) -> Result<(), RenderError> {
    let bpm = plan.bpm;
    let frame_of = |beat: f64| -> i64 { (beat * 60.0 / bpm * sr).round() as i64 };
    // the engine on the first section
    let mut secs = Sections::new(c, plan.program_bars);
    let (decks, playing) = secs.start();
    let mut arr = Arrangement::new(bpm, decks, Vec::new());
    arr.start.playing = Some(playing);
    let opts = ArrangeRenderOptions { sample_rate: sr, ..Default::default() };
    let mut cond = arrangement_conductor(&arr, &opts);
    d.built();
    let _refused = stand_on_start(&mut cond, &arr, None, None);
    // the first section's sounds, once its nets are in (the page's `sound` ops come after the loads they follow)
    for (deck, role, opt) in secs.start_sounds() {
        ctls.browser.sound(cond.engine_mut(), &deck, &role, &opt);
    }
    d.ready(&mut cond)?;

    cond.engine_mut().trace_via = "playback".into();
    let mut next_ev: Option<(i64, ArrangeEvent)> = secs.next().map(|e| (frame_of(e.at), e));
    let mut produced: i64 = 0;
    let (mut wl, mut wr) = (Vec::with_capacity(BLOCK), Vec::with_capacity(BLOCK));
    while produced < want {
        let frame = cond.engine().frame();
        while let Some((f, _)) = &next_ev {
            if *f > frame {
                break;
            }
            let (_, e) = next_ev.take().expect("just matched");
            // a deck that comes in: the macros waiting for it first (on a deck that does not play yet they start at once)
            if let ArrangeOp::Deck(DeckAction::Play { deck, on: true }) = &e.op {
                ctls.apply_deferred(Some(deck), frame, cond.engine_mut());
            }
            match &e.op {
                ArrangeOp::Live { method, params } => {
                    // refusals (the closing `cue`) are not the listener's business
                    let _ = cond.engine_mut().apply_live_op(method, params);
                }
                op => {
                    let _ = cond.engine_mut().apply_arrange_op(op);
                }
            }
            // a section's net loaded: its parts' sounds, as the page sets them once the net is in
            if let ArrangeOp::Deck(DeckAction::Load { .. }) = &e.op {
                for (deck, role, opt) in secs.sounds.pop_front().unwrap_or_default() {
                    ctls.browser.sound(cond.engine_mut(), &deck, &role, &opt);
                }
            }
            next_ev = secs.next().map(|e| (frame_of(e.at), e));
        }
        // the control lines due on this frame, after the sections' moves (the desktop's order on a tie)
        ctls.apply_due(frame, cond.engine_mut());
        let mut n = ((want - produced) as usize).min(BLOCK);
        if let Some((f, _)) = &next_ev {
            n = n.min((f - frame).max(1) as usize);
        }
        if let Some(f) = ctls.peek(frame, cond.engine()) {
            n = n.min((f - frame).max(1) as usize);
        }
        let (l, r) = cond.step_frames(n);
        if ctls.wheel && ctls.browser.wheel_used() {
            // the Wheel, after the decks, into the master (the worklet's `render`, then the voice's `render_add`)
            wl.clear();
            wl.extend_from_slice(l);
            wr.clear();
            wr.extend_from_slice(r);
            ctls.browser.render_add(&mut wl, &mut wr);
            d.block(produced, &wl, &wr)?;
        } else {
            ctls.browser.skip(n);
            d.block(produced, l, r)?;
        }
        produced += n as i64;
        d.after(&mut cond, produced);
    }
    Ok(())
}

/// `render_to`'s driver: the frames past `--from-bar` into the chunked sink, the start and progress events, and
/// the [`Tap`] (`listen`) shown the conductor before the first block and after each block, with that block's master.
struct Stream<'s, 'a, 'e> {
    sink: Sink<'a>,
    ev: &'s mut Events<'e>,
    start: Option<Vec<(&'static str, JsonValue)>>,
    t0: Instant,
    skip: i64,
    want: i64,
    sr: f64,
    engine_ms: f64,
    ready_ms: f64,
    last_progress: Option<Instant>,
    tap: &'s mut dyn Tap,
    plan: &'s Plan,
    notes: Vec<String>,
    // the block, copied out for the tap: it needs the conductor too, which `after` has
    tap_l: Vec<f32>,
    tap_r: Vec<f32>,
}

impl Driver for Stream<'_, '_, '_> {
    fn built(&mut self) {
        self.engine_ms = self.t0.elapsed().as_secs_f64() * 1000.0;
    }
    fn ready(&mut self, cond: &mut OfflineConductor) -> Result<(), RenderError> {
        self.ready_ms = self.t0.elapsed().as_secs_f64() * 1000.0;
        self.tap.start(cond, self.plan, &self.notes);
        if let Some(start) = self.start.take() {
            self.ev.emit(start);
        }
        Ok(())
    }
    fn block(&mut self, at: i64, l: &[f32], r: &[f32]) -> Result<(), RenderError> {
        let n = l.len();
        // the frames of this step that are on the far side of `--from-bar`
        let before = (self.skip - at).clamp(0, n as i64) as usize;
        if before < n {
            self.sink.push(&l[before..], &r[before..])?;
        }
        let produced = at + n as i64;
        if self.sink.wrote_any && self.last_progress.map_or(true, |t| t.elapsed() >= PROGRESS_EVERY) && produced < self.want {
            self.last_progress = Some(Instant::now());
            self.ev.emit(vec![("event", "progress".into()), ("seconds_rendered", num(self.sink.frames_out as f64 / self.sr))]);
        }
        self.tap_l.clear();
        self.tap_l.extend_from_slice(l);
        self.tap_r.clear();
        self.tap_r.extend_from_slice(r);
        Ok(())
    }
    fn after(&mut self, cond: &mut OfflineConductor, _produced: i64) {
        // the engine frame the block began at (the step moved it on by the block's length)
        let at = cond.engine().frame() - self.tap_l.len() as i64;
        self.tap.block(cond, at, &self.tap_l, &self.tap_r);
    }
}

// ---------------------------------------------------------------------------------------------
// the command

/// `beatscript render`: `args` are the flags (a leading `render` is ignored). The program text is on stdin, or
/// `--file`. Returns the exit code: 0 rendered, 1 an `error` event was printed.
pub fn main(args: Vec<String>) -> i32 {
    let t0 = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), RenderError> {
        let o = parse_args(&args)?;
        let text = read_text(&o)?;
        let mut err = std::io::stderr();
        let mut ev = Events::new(&mut err);
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        render_to(&text, &o, &mut lock, &mut ev, t0).map(|_| ())
    }));
    let mut err = std::io::stderr();
    let mut ev = Events::new(&mut err);
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            // a reader that went away is not an error worth a line: it asked for this
            if e.code == "closed" {
                return 0;
            }
            ev.error(&e);
            1
        }
        Err(p) => {
            let m = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
            ev.error(&RenderError::new("internal", format!("internal error: {m}")));
            1
        }
    }
}

fn read_text(o: &Options) -> Result<String, RenderError> {
    let mut bytes = Vec::new();
    match o.file.as_deref() {
        Some(p) if p != "-" => {
            let f = std::fs::File::open(p).map_err(|e| RenderError::new("io", format!("{p}: {e}")))?;
            f.take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| RenderError::new("io", format!("{p}: {e}")))?;
        }
        _ => {
            std::io::stdin().lock().take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        }
    }
    String::from_utf8(bytes).map_err(|_| RenderError::new("usage", "the program is not UTF-8 text"))
}
