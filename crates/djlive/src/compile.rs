//! From the syntax tree to what plays: pattern lines become one raw-nets envelope per deck (the
//! piano-roll form, `djengine::roll`, so they take the engine's own path onto a deck and get a
//! share CID), control lines become calls on the control-language registry. Pure and
//! deterministic: the same text compiles to the same envelopes, the same CIDs.

use std::collections::BTreeMap;

use djengine::cid;
use djengine::json::{JsonObject, JsonValue};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::docall;
use crate::dur::Dur;
use crate::env::Env;
use crate::expand::{self, FlatLine};
use crate::music::{self, Key};
use crate::parse::{self, PARAMS, ROLES};
use crate::pattern::{self, Ctx, Sound};
use crate::print;
use crate::rat::Rat;

pub const MOVES: [&str; 6] = ["filter_exit", "echo_out", "loop_roll", "bass_swap", "phrase_sync", "vocoder_handoff"];
/// Bars in a phrase.
pub const PHRASE_BARS: u32 = 8;
/// The longest one net may be, in bars: the roll's 1024 sixteenth steps (`djrust::roll::MAX_STEPS`, from the
/// Swift piano roll). A deck whose parts take longer to come round is chained from nets of this length.
pub const MAX_BARS: u64 = 64;
/// The longest a program may be, in bars: the most any alternation may take to come round, the most a
/// timeline may add up to.
pub const MAX_SONG_BARS: u64 = 16384;

/// One note of a part in one cycle.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub step: u8,
    pub note: u8,
    pub dur: u8,
    pub vel: u8,
    /// `e2!`: an accent (a 303's, and a hard hit anywhere).
    pub accent: bool,
    /// `e3^`: a slide into the next step.
    pub slide: bool,
    /// `x.` `x&` `x:3`: a ghost (already in `vel`), a flam, a ratchet.
    pub marks: Marks,
    /// Where in the text it came from: an index into its part's `spans` (u32::MAX: unknown). Not part of
    /// what a hit is: two hits are equal whatever text wrote them.
    pub sid: u32,
    /// The syllable a formant voice sings on it (`djrust::fx::formant::Syllable::code`; 0: none): a sung
    /// line's part (sing.rs, "In the nets").
    pub syl: u32,
}

impl PartialEq for Hit {
    fn eq(&self, o: &Hit) -> bool {
        (self.step, self.note, self.dur, self.vel, self.accent, self.slide, self.marks, self.syl) == (o.step, o.note, o.dur, o.vel, o.accent, o.slide, o.marks, o.syl)
    }
}

/// What a part with chance could play: the part's cycles read with every `?` hit kept and every option of a `choose` stacked
/// (`ChanceCtx::all`), a bound the constraints (`assert never ...`, constraint.rs) are judged on. Empty for a part with no
/// chance. It is not part of what a part is: two parts are equal whatever it says, as two hits are whatever text wrote them.
#[derive(Clone, Debug, Default)]
pub struct MayGrid(pub Vec<Vec<Hit>>);

impl PartialEq for MayGrid {
    fn eq(&self, _: &MayGrid) -> bool {
        true
    }
}

/// A tonal part's envelope beyond its notes: `glide=` (a slide `^` bends into the next note over that many ms),
/// `decay=` (each note's tail, ms) and `pitch-drop=` (each note starts that many semitones high and falls onto
/// its pitch), and the acid voice's knobs (`cutoff=` `res=` `env=` `accent=` `drive=`, [`ACID_PARAMS`]). None of
/// them set is a part as it always was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartExpr {
    pub glide_ms: Option<u16>,
    pub decay_ms: Option<u16>,
    pub pitch_drop: Option<u8>,
    pub acid: djengine::pflow::AcidKnobs,
}

/// The acid voice's knobs a tonal part may turn (`deck B: bass ... inst=acid-bass cutoff=400 res=80`): `cutoff=` the
/// filter's floor in Hz ([`ACID_CUTOFF`]), and `res=` (resonance), `env=` (the envelope's depth), `accent=` (how much
/// an accent does) and `drive=` (the filter's saturation), each 0 to 100, a knob's turn. Each is carried on every note's
/// binding (`cutoff`, `res`, `env`, `accent`, `drive`), only when set, so no part without them moves. They are not
/// reserved words (`section drive` is still a section), and a voice that is not the acid voice ignores them (`check` says so).
pub const ACID_PARAMS: [&str; 5] = ["cutoff", "res", "env", "accent", "drive"];
/// The range of `cutoff=`, Hz; the knobs' range, a turn of 0 to 100.
pub const ACID_CUTOFF: (f64, f64) = (20.0, 12000.0);
pub const ACID_KNOB: (f64, f64) = (0.0, 100.0);

/// What a slide `^` glides in on a voice that slides by itself when its part gives no `glide=`, ms: a 303's slide.
pub const SELF_SLIDE_MS: u16 = 60;

/// Does the voice `inst` slide by itself on a `^`, as a 303 does? The acid voices (`acid`, `acid-square`): the entries of
/// the engine's instrument table that play its `"acid"` engine. Every other voice's `^` holds its note, and bends only with
/// `glide=`. On such a voice the note a `^` is on also says `"slide": true`, so it holds its gate until the glide comes.
pub fn slides_by_itself(inst: &str) -> bool {
    djengine::wave::instruments::InstrumentTable::shared().entries.get(inst).and_then(|e| e.get("engine")).and_then(JsonValue::as_str) == Some("acid")
}

/// The range of each: `glide=` and `decay=` in milliseconds, `pitch-drop=` in semitones.
pub const GLIDE_MS: (f64, f64) = (1.0, 2000.0);
pub const DECAY_MS: (f64, f64) = (10.0, 10000.0);
pub const PITCH_DROP: (f64, f64) = (1.0, 24.0);

/// A part: a role's notes, cycle by cycle.
#[derive(Clone, Debug, PartialEq)]
pub struct PartIr {
    pub role: String,
    pub instrument: Option<String>,
    /// `sound=synth:reese`: the role's sound option, set on the deck once the net lands.
    pub sound: Option<String>,
    /// `kit=drums-808`: a drum part's kit.
    pub kit: Option<String>,
    /// `route=td3`: the part is also sent to the TD-3 (accents and slides riding along).
    pub route: Option<String>,
    /// `vel=70..100`: each hit's velocity is picked by a hash of its bar, step and note.
    pub vel_range: Option<(i32, i32)>,
    /// The places in the text each hit came from (its `sid` is an index here): the token, the terms round it,
    /// the part, the line, and the calls that expanded it. The tokens and their `let` and `def` sources are all
    /// there, so a hit lights both the call site and the def body.
    pub spans: Vec<Vec<Span>>,
    /// The calls and repeats that expanded the part's line (innermost first): the call sites to light with it.
    pub via: Vec<Span>,
    /// The part as the canonical text says it.
    pub text: String,
    /// `hihat/12`: the part's own cycle in sixteenth steps (16 for a part with none): F6, meter.rs.
    pub cycle_steps: u32,
    /// `cycles[c]`: the hits of pattern cycle `c` (the part's own alternations make them differ).
    pub cycles: Vec<Vec<Hit>>,
    /// The part has chance in it (`?`, `choose`, `maybe`): it differs every bar, so it has no period (L-10).
    pub chance: bool,
    /// The bars the part's alternations and its own cycle take to come round, before any chance is unrolled (what its
    /// period is without the dice): the same wherever the part is placed (L-10, L-21).
    pub structural_bars: u64,
    /// `glide=` `decay=` `pitch-drop=`.
    pub expr: PartExpr,
    /// What the part could play, a bar at a time as `cycles` is, when it has chance (empty otherwise): see [`MayGrid`].
    pub may: MayGrid,
    /// After a `jux`: the part's right copy, a part of the same role that plays on the right (the part itself on the left).
    /// It rides inside its owner, so everything that keys a deck's parts by role sees one part; the envelope writes it as
    /// a net of its own ([`DeckIr::envelope`]).
    pub twin: Option<Box<PartIr>>,
    /// The seed a part with chance draws its dice from: its own `seed=`, else the one its deck and its text make (L-05). None for a
    /// part with no chance. A part written with it (`seed=N`) on another deck plays the same hits (stored.rs).
    pub seed: Option<u32>,
}

/// How many parts of a deck may have a twin before `check` says each one plays a second set of voices.
pub const MAX_TWINNED: usize = 4;

/// Where a `jux`'d part and its twin sit: the part hard left, the twin hard right (the engine's `track.pan`, -1 to 1).
pub const JUX_PAN: f64 = 1.0;

/// The net id of a part's twin: the role and this (a name the share format takes, and no other part has).
pub const TWIN_SUFFIX: &str = "_r";

impl PartIr {
    /// The grids of hits a change to the part's notes must reach alike: `cycles`, and the chance grid `may` where it has one
    /// (a rotation, a transposition or a thinning of the part is one of the notes it may play too).
    pub fn grids_mut(&mut self) -> Vec<&mut Vec<Vec<Hit>>> {
        let mut grids = vec![&mut self.cycles];
        if !self.may.0.is_empty() {
            grids.push(&mut self.may.0);
        }
        grids
    }

    /// A fingerprint of what the part plays (not how it was written).
    pub fn fingerprint(&self) -> String {
        let mut j = self.json(0, self.cycles.len().max(1));
        if let JsonValue::Object(o) = &mut j {
            for (k, v) in [("sound", &self.sound), ("kit", &self.kit), ("route", &self.route)] {
                if let Some(v) = v {
                    o.set(k, JsonValue::str(v.clone()));
                }
            }
            // the twin is part of what the part plays: a hot reload that only moves it must reload
            if let Some(t) = &self.twin {
                o.set("twin", t.json(0, t.cycles.len().max(1)));
            }
        }
        cid::share_cid(&j)
    }

    fn json(&self, phase: usize, period: usize) -> JsonValue {
        let n = self.cycles.len().max(1);
        let mut notes = Vec::new();
        let bars: Vec<&[Hit]> = (0..period).map(|bar| self.cycles.get((bar + phase) % n).map_or(&[][..], |c| c.as_slice())).collect();
        // `glide=`: a note glides when the step before it (the net comes round, so the last step is before the
        // first) has a slide on it. On a voice that slides by itself (the acid voice) a slide glides without it, in a 303's time.
        let self_sliding = self.instrument.as_deref().is_some_and(slides_by_itself);
        let glide = self.expr.glide_ms.or(self_sliding.then_some(SELF_SLIDE_MS)).filter(|_| bars.iter().any(|b| b.iter().any(|h| h.slide)));
        let mut slid = glide.is_some() && bars.iter().rev().find(|b| !b.is_empty()).is_some_and(|b| {
            let last = b.iter().map(|h| h.step).max().unwrap_or(0);
            b.iter().any(|h| h.step == last && h.slide)
        });
        let (mut at, mut sliding) = (usize::MAX, false);
        for (bar, hits) in bars.iter().enumerate() {
            for h in hits.iter() {
                let step = bar * 16 + usize::from(h.step);
                if step != at {
                    if at != usize::MAX {
                        slid = sliding;
                    }
                    (at, sliding) = (step, false);
                }
                sliding |= h.slide;
                let mut note = vec![JsonValue::int(step as i64), JsonValue::int(i64::from(h.note)), JsonValue::int(i64::from(h.dur)), JsonValue::int(i64::from(h.vel))];
                // how the note is played, only what is set: a part with none of it keeps its envelope and its CID
                let mut x = JsonObject::new();
                if let (Some(ms), true) = (glide, slid) {
                    x.set("glide", JsonValue::int(i64::from(ms)));
                }
                if let Some(ms) = self.expr.decay_ms {
                    x.set("decay", JsonValue::int(i64::from(ms)));
                }
                if let Some(st) = self.expr.pitch_drop {
                    x.set("pitchDrop", JsonValue::int(i64::from(st)));
                }
                if self.expr.acid.cutoff_hz > 0 {
                    x.set("cutoff", JsonValue::int(i64::from(self.expr.acid.cutoff_hz)));
                }
                for (k, v) in [("res", self.expr.acid.res), ("env", self.expr.acid.env), ("accent", self.expr.acid.accent), ("drive", self.expr.acid.drive)] {
                    if let Some(v) = v {
                        x.set(k, JsonValue::int(i64::from(v)));
                    }
                }
                if let Some(r) = h.marks.ratchet {
                    x.set("ratchet", JsonValue::int(i64::from(r)));
                }
                // on a voice that slides by itself the note a slide leaves says so: its gate stays open for the glide,
                // as a 303's does on a slide step (the last step's length is cut at the ring's end, the gate must not be)
                if self_sliding && glide.is_some() && h.slide {
                    x.set("slide", JsonValue::Bool(true));
                }
                if h.marks.flam {
                    x.set("flam", JsonValue::Bool(true));
                }
                // a sung note says its syllable (only then: a part without one keeps its envelope): the plain
                // string when that is all, inside the object when the note is also played some way
                let syl = djengine::fx::formant::Syllable::from_code(h.syl);
                if x.is_empty() {
                    if let Some(syl) = syl {
                        note.push(JsonValue::str(syl.text()));
                    }
                } else {
                    if let Some(syl) = syl {
                        x.set("syl", JsonValue::str(syl.text()));
                    }
                    note.push(JsonValue::Object(x));
                }
                notes.push(JsonValue::Array(note));
            }
        }
        let mut o = JsonObject::new();
        if let Some(i) = &self.instrument {
            o.set("instrument", JsonValue::str(i.clone()));
        }
        o.set("notes", JsonValue::Array(notes));
        JsonValue::Object(o)
    }
}

/// A deck's net: its parts, and how many bars it takes to come round.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckIr {
    pub deck: Deck,
    pub period: usize,
    pub parts: Vec<PartIr>,
    pub tempo: Option<f64>,
    pub swing: Option<f64>,
    pub key: Option<Key>,
}

impl DeckIr {
    /// The raw-nets envelope (`djengine::roll::envelope`) of the net starting `phase` bars into
    /// its pattern: bar 0 of the net plays pattern cycle `phase`, so a net swapped in on a bar
    /// line goes on from where the old one was.
    pub fn envelope(&self, phase: usize) -> JsonValue {
        let mut parts = JsonObject::new();
        for p in &self.parts {
            let mut part = p.json(phase, self.period);
            // a part with a twin (`jux`) plays hard left, and its twin is a net of the same role hard right
            if let Some(t) = &p.twin {
                if let JsonValue::Object(o) = &mut part {
                    o.set("pan", JsonValue::Number(-JUX_PAN));
                }
                let mut twin = t.json(phase, self.period);
                if let JsonValue::Object(o) = &mut twin {
                    o.set("role", JsonValue::str(p.role.clone()));
                    o.set("pan", JsonValue::Number(JUX_PAN));
                }
                parts.set(p.role.as_str(), part);
                parts.set(format!("{}{TWIN_SUFFIX}", p.role), twin);
            } else {
                parts.set(p.role.as_str(), part);
            }
        }
        let mut o = JsonObject::new();
        o.set("name", JsonValue::str(format!("live deck {}", self.deck.name())));
        o.set("parts", JsonValue::Object(parts));
        o.set("steps", JsonValue::int(self.period as i64 * 16));
        if let Some(t) = self.tempo {
            o.set("tempo", JsonValue::Number(t));
        }
        if let Some(s) = self.swing {
            // the setting is 0..1 and djrust's project swing 0..100, the share of half a sixteenth each off-step is late
            // (wave/runner.rs: swing / 100 * sixteenth * 0.5): `swing 0.6` is beats' 60, 0.3 of a sixteenth (BEATSCRIPT.md, Settings)
            // (rounded to 1e-9 so that 0.15 is 15, not 15.000000000000002, in the envelope and its CID)
            o.set("swing", JsonValue::Number((s * 100.0 * 1e9).round() / 1e9));
        }
        if let Some(k) = self.key {
            o.set("key", JsonValue::str(k.text()));
        }
        match djengine::roll::envelope(&JsonValue::Object(o)) {
            Ok(e) => e,
            Err(e) => JsonValue::str(format!("roll: {e}")),
        }
    }

    /// The share CID of the net at phase 0: the same text gives the same one.
    pub fn net_cid(&self) -> String {
        cid::share_cid(&self.envelope(0))
    }

    /// What was loaded: the net, and the sounds set on it after it lands.
    pub fn load_key(&self) -> String {
        let mut k = self.net_cid();
        for (role, opt) in self.sounds() {
            k.push_str(&format!("|{role}={opt}"));
        }
        k
    }

    /// (role, option) for every part with its own `sound=` or `kit=`.
    pub fn sounds(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        for p in &self.parts {
            if let Some(s) = &p.sound {
                v.push((p.role.clone(), s.clone()));
            }
            if let Some(k) = &p.kit {
                v.push((p.role.clone(), format!("kit:{k}")));
            }
        }
        v
    }

    /// The part to send to the TD-3 (`route=td3`), if any.
    pub fn td3_part(&self) -> Option<&PartIr> {
        self.parts.iter().find(|p| p.route.as_deref() == Some("td3"))
    }

    /// Roles and their fingerprints, in role order.
    pub fn fingerprints(&self) -> BTreeMap<String, String> {
        self.parts.iter().map(|p| (p.role.clone(), p.fingerprint())).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    /// Once, on the first bar line after the statement was added.
    Now,
    /// On bars n, 2n, 3n ...
    Every(u32),
    /// `every n bars from f`: on bars f, f+n, f+2n ...
    EveryFrom(u32, u32),
    At(u32),
    /// `on next phrase 16`: once, on the first bar that is a multiple of n.
    NextMult(u32),
    /// `at bar 2 beat 3`, `before drop 2 beats`: once, when the program's position (in bars, exact) reaches this
    /// and it is not on a bar line (an exact bar is [`Timing::At`]).
    AtPos(Rat),
    /// `every 2 beats`, `every 1/2 bar from 3`: at `from`, `from + step`, `from + 2 step` ... (in bars, exact).
    EveryPos { step: Rat, from: Rat },
}

/// One call on the control-language registry: a noun and its params (the `action` among them).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub noun: String,
    pub params: JsonValue,
}

impl Call {
    fn new<const N: usize>(noun: &str, pairs: [(&str, JsonValue); N]) -> Call {
        Call { noun: noun.to_string(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }
    }

    pub fn text(&self) -> String {
        format!("{} {}", self.noun, self.params.canonical_js())
    }
}

/// A value that moves: `from` to `to` over `bars`, sent as `field` of `base` (a registry call).
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub prelude: Vec<Call>,
    pub noun: String,
    pub base: JsonValue,
    pub field: String,
    pub from: f64,
    pub to: f64,
    /// How long it takes, exactly (`over 8 bars`, `over 2 beats`).
    pub bars: Dur,
    /// How its time becomes its progress (F4, shape.rs): linear unless the line says.
    pub shape: crate::shape::Shape,
}

impl Ramp {
    /// The value a fraction `t` (0..1) of the way through: `from + (to - from) * shape(t)`.
    pub fn value(&self, t: f64) -> f64 {
        self.from + (self.to - self.from) * self.shape.apply(t)
    }

    pub fn at(&self, t: f64) -> Call {
        let v = self.value(t);
        let mut o = self.base.as_object().cloned().unwrap_or_default();
        o.set(self.field.as_str(), JsonValue::Number((v * 1e6).round() / 1e6));
        Call { noun: self.noun.clone(), params: JsonValue::Object(o) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Calls(Vec<Call>),
    Ramp(Ramp),
    /// `duck B under A.kick 6db`: the engine's sidechain pump on a deck (mixmoves.rs).
    Duck(crate::mixmoves::DuckPlan),
    /// `mix { A level 0.7 }`: the mixer set, read first and put back when it restores (mixmoves.rs).
    Mix(crate::mixmoves::MixPlan),
    /// A value that keeps moving: `lfo sine 2 bars ...` (F4, shape.rs).
    Lfo(crate::shape::Lfo),
    /// A line that names an input (`master: filter lp 200 + knob(3) * 7800`, `... when held(key)`): decided and filled in with
    /// the values read when it is due (F13, perform.rs).
    Performed(Box<crate::perform::Performed>),
}

impl Effect {
    /// The effect as compiled: a performed line's own (read with every input at 1/2), anything else itself.
    pub fn base(&self) -> &Effect {
        match self {
            Effect::Performed(p) => p.base.base(),
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// The canonical text of the line (with `#2` for a second identical one): what identifies it
    /// across evaluations.
    pub key: String,
    pub timing: Timing,
    pub effect: Effect,
    pub span: Span,
    /// `for N bars`, `for 2 beats`: put back after that long.
    pub hold: Option<Dur>,
    /// The calls that put it back.
    pub undo: Vec<Call>,
    /// A `move` ends by muting its deck's channel: after it (this many bars), at the next phrase line,
    /// the strip is put back whatever else happens. None for everything that is not a move.
    pub restore: Option<u32>,
    /// The calls and repeats that expanded the line (innermost first).
    pub via: Vec<Span>,
    /// Said in the runtime's log when the line fires: what the check found that is still true then (a neural voice that is
    /// not installed, and the formant voice the line plays on instead).
    pub notice: Option<String>,
    /// A track's line in a set runs only on these program bars (from, to: to is the first bar it does not).
    pub window: Option<(u32, u32)>,
    /// How far ahead of its line's time the call is made, in bars (a `macro`'s beat: macros.rs `lead`); None for a call made
    /// on its line's time. What is judged against the line's place (a section's end, a set track's window) is `timing` + this.
    pub lead: Option<Rat>,
}

/// One stretch of an arranged song: the decks' nets for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub name: String,
    pub start: u32,
    pub bars: u32,
    pub decks: BTreeMap<Deck, DeckIr>,
    /// An `evolve`'s generation (0 is the section as written).
    pub gen: Option<u32>,
    /// The `section` statement's span (its first line is the header the playhead sits on).
    pub header: Option<Span>,
    /// The statements in this stretch of the song, and the calls that expanded them: the lines that are
    /// playing while it does.
    pub lines: Vec<Span>,
    /// A continuation of the segment before it (the same section: its next generation, or the next
    /// stretch of a net too long for one), not a section of its own.
    pub chain: bool,
}

/// One generation of an `evolve`: its CID (of its nets), the CID of the generation it came from, and where
/// on the timeline it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Generation {
    pub scope: String,
    pub gen: u32,
    pub cid: String,
    pub forked_from: Option<String>,
    pub start: u32,
    pub bars: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Compiled {
    /// The program's rights from the assets it names (assets.rs): its tier, why, and what it must credit.
    pub rights: crate::assets::RightsReport,
    /// The canonical text.
    pub text: String,
    /// The program's CID: of the canonical text.
    pub cid: String,
    pub tempo: Option<f64>,
    pub swing: Option<f64>,
    pub key: Option<Key>,
    /// The nets (an arranged song: those of its first section).
    pub decks: BTreeMap<Deck, DeckIr>,
    pub controls: Vec<Control>,
    pub warnings: Vec<Diagnostic>,
    /// There is an `arrange`: `segments` is the timeline.
    pub arranged: bool,
    pub segments: Vec<Segment>,
    /// The comments, in order, `#` off: not part of the program's CID (a link carries them unsealed).
    pub notes: Vec<String>,
    /// The bar the program ends on: the song's (an arranged song ends after its last section unless it
    /// is told to `loop`), or `end at bar N` of the program's own count. None: it plays on.
    pub end: Option<u32>,
    /// `loop`: a song starts again from its first section when it gets to its end.
    pub looped: bool,
    /// The bar a looping song goes back to at its end: 0, its first, unless named arrangements loop on a later one.
    pub loop_from: u32,
    /// The flow of the named arrangements, if there are any (flow.rs).
    pub flow: Option<crate::expand::FlowReport>,
    /// A program too long for one net (no sections: its parts take more than 64 bars to come round) is
    /// chained from nets of 64 bars and loops; its `at bar` and `every` lines still count program bars from
    /// the bar it was evaluated on, as a flat program's do.
    pub flat_clock: bool,
    /// The text as it was evaluated: every span in the program (a hit's, a line's) is an offset into it.
    /// Not part of the canonical form or any CID.
    pub source: String,
    /// Every statement of a program with no timeline (what is playing, when nothing is a section).
    pub flat_lines: Vec<Span>,
    /// The generations of every `evolve`, in order, each with the one it came from.
    pub lineage: Vec<Generation>,
    /// Every `evolve` as a rule over generation 0, with what it comes to: its tail and period. Any
    /// generation's CID is [`Compiled::generation_cid`] of these.
    pub evolves: Vec<EvolveRun>,
    /// The key in force in each section of the timeline: its own `key` (the head of its body), else the
    /// program's. F5, scale.rs; read through [`Compiled::section_key`].
    pub section_keys: BTreeMap<String, Key>,
    /// What the program says of its own structure for the JSON-LD export (F14, linked.rs): the sections the song places
    /// and the Euclidean rhythms its expanded lines use.
    pub linked: crate::linked::Structure,
    /// The program's tempo through its bars, exactly (F3, tempo.rs): through [`Compiled::tempo_map`].
    pub tempo_map: crate::tempo::TempoMap,
    /// The program's `assert` statements, in order, each with what `check` found out about it (F12, assertion.rs): a static
    /// one passed (a false one refuses the program, except in [`check_report`]), a `level` one is pending until `btw test`.
    pub assertions: Vec<crate::assertion::Checked>,
    /// A set (set.rs): its tracks, mixes and length.
    pub set: Option<crate::set::SetReport>,
    /// A station (station.rs): its knobs, the walk and the first tracks. A station compiles to its track 0.
    pub station: Option<crate::station::StationReport>,
    /// The programs this one draws on, for a link to carry: the tracks a set plays and the snippets an `import` brings, each with
    /// what it draws on in turn, the ones a program needs before the one that needs them: (CID, canonical text), each once.
    pub plays: Vec<(String, String)>,
    /// Every `sing` line: when and where it sings, host or no host, for `assert never X during sing` (constraint.rs).
    pub sing: Vec<crate::sing::SingLine>,
    /// The `launch` lines (a pad brings a scene in) and each scene as a program of its own (launch.rs). Empty for a program with none.
    pub launches: crate::launch::Launches,
}

/// One `evolve`: the section (or program) it varies as written, the rule, and where its orbit comes round.
#[derive(Clone, Debug, PartialEq)]
pub struct EvolveRun {
    pub scope: String,
    pub block: EvolveBlock,
    pub gen0: BTreeMap<Deck, DeckIr>,
    pub analysis: crate::evolve::Analysis,
    /// The last generation the timeline plays.
    pub last: u32,
    /// The bars the generations play in: the section's (a program's own evolve: its timeline's).
    pub bars: u32,
}

impl EvolveRun {
    /// The last generation that plays: `last`, or the last to start before the section is over.
    pub fn played_last(&self) -> u32 {
        let fit = self.bars.saturating_sub(1) / self.block.every.max(1);
        self.last.min(fit)
    }

    /// Whether the generations that play come back to one they have had: the orbit's tail and one period are played.
    pub fn cycles(&self) -> bool {
        self.analysis.period.is_some_and(|p| u64::from(self.played_last()) >= u64::from(self.analysis.tail) + p)
    }
}

impl Compiled {
    /// The key in force in the section `name` (the program's key where the section sets none; C major where the program
    /// has none). None: no such section on the timeline. `name` is a section's or a segment's (`intro+verse`, F10): a section
    /// has the key of the first stretch it plays in (its own where it sets one, so everywhere; a section that sets none
    /// follows the stretch, see `algebra::Lower::stretch_keys`). The names [`Compiled::sections_at`] answers all have a key.
    pub fn section_key(&self, name: &str) -> Option<Key> {
        if !name.contains('+') {
            if let Some(k) = self.segments.iter().find(|s| s.name.split('+').any(|n| n == name)).and_then(|s| self.section_keys.get(&s.name)) {
                return Some(*k);
            }
        }
        self.section_keys.get(name).copied()
    }

    /// The key in force in a segment of the timeline (a segment's name says its layers, and the layers say its key).
    pub fn key_in(&self, seg: &Segment) -> Key {
        self.section_keys.get(&seg.name).copied().or(self.key).unwrap_or(Key::C_MAJOR)
    }

    /// The CID of generation `n` of the evolve in `scope` (a section's name; "evolve" for a program's own),
    /// made from generation 0 and the rule alone: nothing stored, and a generation far past the end costs the
    /// length of the orbit, not `n`.
    pub fn generation_cid(&self, scope: &str, n: u32) -> Option<String> {
        let run = self.evolves.iter().find(|r| r.scope == scope)?;
        let decks = if n == 0 { run.gen0.clone() } else { crate::evolve::nth(&run.block, &run.gen0, &run.analysis, n)? };
        Some(crate::evolve::generation_cid(&decks))
    }

    /// What each `evolve` comes to, one line each: its tail and period, in bars and in time at the tempo.
    pub fn evolve_report(&self) -> Vec<String> {
        let bpm = self.tempo.unwrap_or(120.0);
        self.evolves
            .iter()
            .map(|r| {
                let scope = if r.scope == "evolve" { "" } else { &r.scope };
                // the orbit is the rule's; what plays is generations 0 to `last` in the section's bars. Its cycle is said only
                // when the timeline plays a generation the orbit comes back to (6685: a `stop when gen == 5` in 24 bars
                // was reported as a 26-generation tail and a cycle it never reaches)
                if !r.cycles() {
                    return r.analysis.text_played(scope, r.played_last(), r.bars, r.block.last.map(|(n, _)| n));
                }
                // with a tempo curve the seconds are the integral from where the section starts (F3)
                match (self.tempo_map.drives(), r.analysis.period_bars()) {
                    (true, Some(pb)) => {
                        let start = self.segments.iter().find(|s| s.name == r.scope && s.gen.is_none_or(|g| g == 0)).map_or(0, |s| s.start);
                        let end = Rat::int((u64::from(start) + pb).min(i64::MAX as u64) as i64);
                        let secs = self.tempo_map.seconds_between(start, end);
                        r.analysis.text_secs(scope, secs, "with the tempo curve")
                    }
                    _ => r.analysis.text(scope, bpm),
                }
            })
            .collect()
    }

    /// Deck → net CID (an arranged song: `section:deck`).
    pub fn net_cids(&self) -> BTreeMap<String, String> {
        if self.arranged {
            let mut m = BTreeMap::new();
            for seg in &self.segments {
                for (d, ir) in &seg.decks {
                    m.insert(format!("{}@{}:{}", seg.name, seg.start, d.name()), ir.net_cid());
                }
            }
            return m;
        }
        self.decks.iter().map(|(d, ir)| (d.name().to_string(), ir.net_cid())).collect()
    }

    /// The nets that play on bar `rel` of the song (a song with no timeline: its nets); the song's
    /// last section goes on past its end. None before the song.
    pub fn nets_at(&self, rel: i64) -> Option<(Option<&Segment>, &BTreeMap<Deck, DeckIr>)> {
        if !self.arranged {
            return Some((None, &self.decks));
        }
        if rel < 0 {
            return None;
        }
        let seg = self.segments.iter().rev().find(|s| i64::from(s.start) <= rel).or_else(|| self.segments.first())?;
        Some((Some(seg), &seg.decks))
    }

    /// The section a bar is in, how far into it, and how long it is (an evolving section's generations
    /// are one section).
    pub fn section_at(&self, rel: i64) -> Option<(&str, i64, u32)> {
        if !self.arranged || rel < 0 {
            return None;
        }
        let i = self.segments.iter().rposition(|s| i64::from(s.start) <= rel)?;
        let name = self.segments[i].name.as_str();
        let (mut lo, mut hi) = (i, i);
        while lo > 0 && self.segments[lo].chain {
            lo -= 1;
        }
        while hi + 1 < self.segments.len() && self.segments[hi + 1].chain {
            hi += 1;
        }
        let start = self.segments[lo].start;
        let len = self.segments[hi].start + self.segments[hi].bars - start;
        Some((name, rel - i64::from(start), len))
    }

    /// The evolve generation playing on a bar, if the section evolves.
    pub fn generation_at(&self, rel: i64) -> Option<u32> {
        if !self.arranged || rel < 0 {
            return None;
        }
        self.segments.iter().rev().find(|s| i64::from(s.start) <= rel).and_then(|s| s.gen)
    }

    /// The song written out longhand: one net per deck for the whole timeline, every part one long
    /// bar-by-bar alternation with a rest bar wherever its section does not have it. This is the
    /// normal form a section-less program says the same song in, so a song and its longhand have the
    /// same net CIDs. (What plays live is the sections' own nets, swapped on the bar lines; the notes
    /// are the same, the CIDs of the nets are not, because a net is a whole loop.) A part whose
    /// instrument, kit or sound differs between sections has no single longhand and is an error.
    pub fn song_nets(&self) -> Result<BTreeMap<Deck, DeckIr>, String> {
        if !self.arranged {
            return Ok(self.decks.clone());
        }
        let total = self.song_bars().unwrap_or(0) as usize;
        let mut decks: Vec<Deck> = self.segments.iter().flat_map(|s| s.decks.keys().copied()).collect();
        decks.sort();
        decks.dedup();
        let mut out = BTreeMap::new();
        for d in decks {
            let mut roles: Vec<&str> = Vec::new();
            for seg in &self.segments {
                if let Some(ir) = seg.decks.get(&d) {
                    for p in &ir.parts {
                        if !roles.contains(&p.role.as_str()) {
                            roles.push(&p.role);
                        }
                    }
                }
            }
            let mut parts = Vec::new();
            for role in roles {
                let mut first: Option<&PartIr> = None;
                let mut cycles: Vec<Vec<Hit>> = vec![Vec::new(); total];
                // the twin of a `jux`'d part, bar by bar: silent in the sections where the part has none
                let mut twin_first: Option<&PartIr> = None;
                let mut twin_cycles: Vec<Vec<Hit>> = vec![Vec::new(); total];
                for seg in &self.segments {
                    let Some(p) = seg.decks.get(&d).and_then(|ir| ir.parts.iter().find(|p| p.role == role)) else { continue };
                    match first {
                        None => first = Some(p),
                        Some(f) => {
                            let differs = |what: &str, a: &Option<String>, b: &Option<String>| (a != b).then(|| format!("deck {} {role}: `{what}` is {} in one section and {} in another: a part has one {what} in the longhand", d.name(), a.as_deref().unwrap_or("not set"), b.as_deref().unwrap_or("not set")));
                            if let Some(e) = differs("inst", &f.instrument, &p.instrument).or_else(|| differs("kit", &f.kit, &p.kit)).or_else(|| differs("sound", &f.sound, &p.sound)).or_else(|| differs("route", &f.route, &p.route)) {
                                return Err(e);
                            }
                            if f.vel_range != p.vel_range {
                                return Err(format!("deck {} {role}: `vel` differs between sections", d.name()));
                            }
                        }
                    }
                    for (copy, into, is_twin) in [(Some(p), &mut cycles, false), (p.twin.as_deref(), &mut twin_cycles, true)] {
                        let Some(p) = copy else { continue };
                        if is_twin {
                            twin_first.get_or_insert(p);
                        }
                        let n = p.cycles.len().max(1);
                        for local in 0..seg.bars as usize {
                            let bar = seg.start as usize + local;
                            if bar >= total {
                                break;
                            }
                            let mut hits = p.cycles.get(local % n).cloned().unwrap_or_default();
                            if let Some((lo, hi)) = p.vel_range {
                                // the velocity is picked by the bar of the whole net: here, the song's
                                for h in &mut hits {
                                    let v = lo + (fnv(&format!("{}:{}:{}:{}:{}", d.name(), role, bar, h.step, h.note)) % (hi - lo + 1) as u32) as i32;
                                    h.vel = (if h.accent { v + 27 } else { v }).clamp(1, 127) as u8;
                                }
                            }
                            into[bar] = hits;
                        }
                    }
                }
                let f = first.expect("a role comes from a part");
                let twin = twin_first.map(|t| Box::new(PartIr { cycles: twin_cycles, may: MayGrid::default(), ..t.clone() }));
                parts.push(PartIr { cycles, may: MayGrid::default(), twin, ..f.clone() });
            }
            let any = self.segments.iter().find_map(|s| s.decks.get(&d)).expect("a deck comes from a section");
            out.insert(d, DeckIr { deck: d, period: total, parts, tempo: any.tempo, swing: any.swing, key: self.key });
        }
        Ok(out)
    }

    /// Deck to net CID of [`song_nets`](Self::song_nets).
    pub fn song_net_cids(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.song_nets()?.into_iter().map(|(d, ir)| (d.name().to_string(), ir.net_cid())).collect())
    }

    /// F10: the sections playing on bar `rel` of the song, by name (a stack plays several: its segment is named `a+b`,
    /// and a section's name has no `+`). Empty when there is no timeline or the bar is outside it.
    pub fn sections_at(&self, rel: i64) -> Vec<&str> {
        if !self.arranged || rel < 0 {
            return Vec::new();
        }
        self.segments.iter().rev().find(|s| i64::from(s.start) <= rel).map_or(Vec::new(), |s| s.name.split('+').collect())
    }

    /// The whole song's length in bars (None: no timeline).
    pub fn song_bars(&self) -> Option<u32> {
        self.arranged.then(|| self.segments.last().map_or(0, |s| s.start + s.bars))
    }
}

/// Everything a check finds.
pub struct Check {
    pub compiled: Option<Compiled>,
    /// Errors and warnings, by place.
    pub diagnostics: Vec<Diagnostic>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.compiled.is_some()
    }
}

/// Each diagnostic once, and a diagnostic found in a line that a call or a repeat expanded says which call: every call of a
/// def and every copy of a repeat shares the body's text, so the same thing is found at the same place as many times as the
/// body is run (SYN-24), and without the call that made the line the place alone does not say where the fault is.
fn settle(diags: &mut Vec<Diagnostic>, lowered: &expand::Lowered, src: &str) {
    let mut seen: std::collections::BTreeSet<(usize, usize, String, u8)> = std::collections::BTreeSet::new();
    diags.retain(|d| seen.insert((d.span.start, d.span.end, d.message.clone(), d.severity as u8)));
    let lines: Vec<&expand::FlatLine> = lowered.globals.iter().chain(lowered.segments.iter().flat_map(|s| s.lines.iter())).filter(|f| !f.via.is_empty()).collect();
    if lines.is_empty() {
        return;
    }
    for d in diags.iter_mut() {
        let said = d.help.as_deref().is_some_and(|h| h.contains("(in ") || h.starts_with("in "));
        if said || d.span.is_empty() {
            continue;
        }
        let mut calls: Vec<Span> = Vec::new();
        for f in lines.iter().filter(|f| f.span.start <= d.span.start && d.span.end <= f.span.end) {
            if !calls.contains(&f.via[0]) {
                calls.push(f.via[0]);
            }
        }
        let Some(first) = calls.first() else { continue };
        let text = first.text(src).lines().next().unwrap_or("").trim().trim_end_matches('{').trim();
        let at = crate::diag::line_col(src, first.start).0;
        let more = calls.len() - 1;
        let place = if text.starts_with("repeat") { format!("in the repeat on line {at}") } else { format!("in `{text}`, line {at}") };
        let place = if more > 0 { format!("{place}, and {more} other call{}", if more == 1 { "" } else { "s" }) } else { place };
        d.help = Some(match d.help.take() {
            Some(h) => format!("{h} ({place})"),
            None => place,
        });
    }
}

/// A program's notes: its comments, `#` off. BeatScript 2's version line is not one (it is part of the program: version.rs);
/// BeatScript 1's `#! btw 1`, a comment then as now, still is, so nothing a BeatScript 1 program says moves.
fn notes_of(comments: &[Span], src: &str) -> Vec<String> {
    comments
        .iter()
        .map(|sp| sp.text(src))
        .filter(|t| t.trim() != crate::version::LINE)
        .map(|t| t.trim_start_matches('#').trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The CID of a program's canonical text. The text says its version (version.rs): a BeatScript 2 canonical text (it starts
/// `#! beatscript 2`) is hashed as `{"format":2,"text":...,"type":"beatscript"}`, and any other as BeatScript 1 always was,
/// `{"format":1,"text":...,"type":"djrust.live"}`, so every CID made before the rename is the same CID.
pub fn program_cid(canonical: &str) -> String {
    cid::share_cid(&program_doc(canonical))
}

/// The document a program's CID is of (see [`program_cid`]).
pub fn program_doc(canonical: &str) -> JsonValue {
    let v = crate::version::of_canonical(canonical);
    JsonValue::Object(JsonObject::from_pairs([
        ("format", JsonValue::int(v.format())),
        ("text", JsonValue::str(canonical)),
        ("type", JsonValue::str(v.doc_type())),
    ]))
}

/// The CID a program's sound is seeded from where a CID seeds it (the dice of `arrange { }`'s mutations, the control net's
/// name): its BeatScript 1 CID, of its canonical text without the version line. For a BeatScript 1 program that is its CID;
/// a program and its upgrade play the same.
pub fn sound_cid(canonical: &str) -> String {
    program_cid(crate::version::legacy_canonical(canonical))
}

/// The CID a program has in the other version: for a BeatScript 2 canonical text its legacy CID (BeatScript 1, the old
/// spelling), and for a BeatScript 1 canonical text the CID it is renamed to. The store's `renamed` lineage is this pair.
pub fn renamed_cid(canonical: &str) -> String {
    match crate::version::of_canonical(canonical) {
        crate::version::Version::V2 => program_cid(crate::version::legacy_canonical(canonical)),
        crate::version::Version::V1 => program_cid(&crate::version::upgrade_canonical(canonical)),
    }
}

/// Parse, lower and compile `src` with nothing to check names against (no registry, no instrument
/// catalog, no snippets): see [`check_with`].
pub fn check(src: &str) -> Check {
    check_with(src, &Env::none())
}

/// Parse, lower and compile `src`. With any error nothing compiles: the diagnostics say why. A false static `assert` is an
/// error (the program is refused).
pub fn check_with(src: &str, env: &Env) -> Check {
    check_mode(src, env, true)
}

/// [`check_with`], except that a *false* static `assert` does not refuse the program: it is a `Fail` in
/// [`Compiled::assertions`], so `btw test` can report it together with the render-based ones. (An assertion that names
/// nothing, `in dorp`, is still an error.) Not for `eval`.
pub fn check_report(src: &str, env: &Env) -> Check {
    check_mode(src, env, false)
}

/// Where the host's engine cannot play a line's calls (Registry::plays), a warning at the line: capability-aware, so a
/// program that checks is not silently different where it plays.
fn capability_warnings(c: &Compiler, diags: &mut Vec<Diagnostic>) {
    let Some(reg) = c.env.registry else { return };
    let mut seen: std::collections::BTreeSet<(usize, String)> = std::collections::BTreeSet::new();
    for ctl in &c.controls {
        let calls: Vec<(&str, &JsonValue)> = match ctl.effect.base() {
            Effect::Calls(cs) => cs.iter().map(|x| (x.noun.as_str(), &x.params)).collect(),
            Effect::Ramp(r) => r.prelude.iter().map(|x| (x.noun.as_str(), &x.params)).chain(std::iter::once((r.noun.as_str(), &r.base))).collect(),
            _ => Vec::new(),
        };
        for (noun, params) in calls {
            if let Err(why) = reg.plays(noun, params) {
                if seen.insert((ctl.span.start, why.clone())) {
                    // (a macro line plays in the offline render and listen: the booth has not got `effects fire` yet)
                    let help = if noun == "effects" && matches!(params.get("action").and_then(JsonValue::as_str), Some("fire" | "release")) {
                        "it checks, and `render` and `listen` play it; here the line does nothing"
                    } else {
                        "it checks, and plays in the BeatScript app (the booth); here the line does nothing"
                    };
                    diags.push(Diagnostic::warning(ctl.span, format!("{} does not play this: {why}", reg.engine_name())).help(help));
                }
            }
        }
    }
}

fn check_mode(src: &str, env: &Env, enforce: bool) -> Check {
    check_mode_in(src, env, enforce, None)
}

/// [`check_mode`], or a scene of its program: the same program with every `launch` left out and the song replaced by the scene
/// `launch` names (launch.rs), compiled to be the timeline the runtime swaps in when a pad brings the scene.
fn check_mode_in(src: &str, env: &Env, enforce: bool, scene: Option<&crate::launch::Launch>) -> Check {
    let mut parsed = parse::parse_with(src, env);
    let mut diags = parsed.diagnostics.clone();
    if diags.iter().any(Diagnostic::is_error) {
        return Check { compiled: None, diagnostics: diags };
    }
    // `seed(...)`, `template(...)` and `<cid>.drop`: written out as the BeatScript they stand for, and that is what is checked; the
    // canonical text (and so the CID) is still the program as written, with its references (seed.rs, template.rs, stored.rs; a
    // template's expansion may hold seeds and a seed's a stored reference, which the check of it writes out in turn). What a
    // stored program's text is comes from the host's snippets; the programs it drew on are carried by the link (`plays`).
    // a `gen` call (gen.rs) is resolved to the program the generator makes for it; its part lines and sections are then that
    // program's, borrowed as a stored program's are, with it known for as long as this check runs (`with_generated`)
    let mut generated: Vec<(String, String)> = Vec::new();
    // `use SOURCE as NAME` lines first (uses.rs): written out as the definitions their sources make, the programs they borrow
    // from (a canned section's, a genre template's) known to the stages after them as a gen call's program is
    let used: Option<Result<(crate::seed::Expansion, Vec<(String, String)>), Vec<Diagnostic>>> = match crate::uses::expand(src, env) {
        Some(Ok((e, progs))) => {
            generated = progs;
            Some(Ok((e, Vec::new())))
        }
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };
    let expanded = if used.is_some() { used } else { crate::template::expand_drawing(src, env)
        .or_else(|| crate::seed::expand(src).map(|r| r.map(|e| (e, Vec::new()))))
        .or_else(|| {
            crate::gen::expand(src).map(|r| {
                r.map(|(e, progs)| {
                    generated = progs;
                    (e, Vec::new())
                })
            })
        })
        .or_else(|| crate::stored::expand(src, env)) };
    if let Some(exp) = expanded {
        let (exp, uses) = match exp {
            Ok(e) => e,
            Err(errs) => {
                diags.extend(errs);
                diags.sort_by_key(|d| d.span.start);
                return Check { compiled: None, diagnostics: diags };
            }
        };
        // (a scene of a program that holds references is the scene of what they stand for)
        let mut chk = crate::gen::with_generated(&generated, || check_mode_in(&exp.text, env, enforce, scene));
        for d in &mut chk.diagnostics {
            d.span = exp.back(d.span);
        }
        chk.diagnostics.extend(exp.warnings.iter().cloned());
        chk.diagnostics.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
        if let Some(c) = chk.compiled.as_mut() {
            c.text = print::canonical(&parsed.program);
            c.cid = program_cid(&c.text);
            c.warnings = chk.diagnostics.clone();
            // the comments are the program's own, not the `# from seed(...)` lines of its expansion
            c.notes = notes_of(&parsed.comments, src);
            // the programs a reference drew on go with the program
            for (cid, text) in uses {
                // (a generated program is no program a link need carry: the call makes it again)
                if crate::gen::generated(&cid).is_some() {
                    continue;
                }
                if !c.plays.iter().any(|(x, _)| *x == cid) {
                    c.plays.push((cid, text));
                }
            }
        }
        return chk;
    }
    // the files the program names are found (or missing) before anything is built from it
    let rights = crate::assets::resolve(&mut parsed, src, env, &mut diags);
    if diags.iter().any(Diagnostic::is_error) {
        return Check { compiled: None, diagnostics: diags };
    }
    // `#! genre:` and `#! tags:` header lines, when the host has the ontology: warnings, a comment never stops a program
    if let Some(o) = env.ontology {
        diags.extend(crate::linked::check_headers(src, o, crate::diag::Severity::Warning).1);
    }
    // a station is radio: it compiles to its track 0, with the station's report (station.rs)
    if parsed.program.stmts.iter().any(|s| matches!(s.kind, StmtKind::Station(_))) {
        let compiled = crate::station::check(&parsed, src, env, &mut diags);
        diags.sort_by_key(|d| (d.span.start, d.severity == crate::diag::Severity::Warning));
        return match compiled {
            Some(c) if !diags.iter().any(Diagnostic::is_error) => Check { compiled: Some(c), diagnostics: diags },
            _ => Check { compiled: None, diagnostics: diags },
        };
    }
    // a set is a program of programs: composed from its tracks, not lowered from sections
    if parsed.program.stmts.iter().any(|s| matches!(s.kind, StmtKind::Set(_))) {
        let compiled = crate::set::check(&parsed, src, env, rights, &mut diags);
        diags.sort_by_key(|d| (d.span.start, d.severity == crate::diag::Severity::Warning));
        return match compiled {
            Some(c) if !diags.iter().any(Diagnostic::is_error) => Check { compiled: Some(c), diagnostics: diags },
            _ => Check { compiled: None, diagnostics: diags },
        };
    }
    // `arrange { }` with mutations: lowered to plain sections and lets (mutate.rs), the canonical text kept as written
    let as_written = parsed.program.clone();
    if as_written.stmts.iter().any(|s| matches!(s.kind, StmtKind::ArrangeBlock(_))) {
        // the dice are seeded from the program's sound, which the version line is not: its BeatScript 1 CID (version.rs), the
        // CID it always had, so an upgraded program plays as it did
        let cid = sound_cid(&print::canonical(&as_written));
        crate::mutate::apply(&mut parsed, src, &cid, &mut diags);
        if diags.iter().any(Diagnostic::is_error) {
            return Check { compiled: None, diagnostics: diags };
        }
    }
    let _scene = scene.map(|l| {
        crate::launch::scene_program(&mut parsed.program, l);
        crate::launch::enter_scene(&l.scene)
    });
    let lowered = expand::lower(&parsed, src, &mut diags);
    let mut c = Compiler { src, env: *env, diags: Vec::new(), tempo: None, swing: None, key: None, decks: BTreeMap::new(), controls: Vec::new(), samples: crate::assets::voices(&rights), chance_bars: crate::chance::DEFAULT_BARS, tempos: Vec::new(), tempo_map: Default::default(), seg_end: None, sing_into: None, sung: Vec::new(), sing_lines: Vec::new() };
    c.settings(&parsed.program);
    let key = c.key.unwrap_or(Key::C_MAJOR);
    let section_keys = crate::scale::section_keys(&lowered, key, &mut diags);
    let mut segments = c.build(&lowered, key);
    // the control-deck lines an `arrange { }` switch holds back in a play are checked as the lines they are, and fire nowhere
    c.check_quiet(&lowered);
    crate::meter::report_period(&mut c, &lowered);
    crate::perform::report(&mut c);
    let (lineage, evolves, top_total) = c.evolve(&lowered, &mut segments);
    let flat_chain = c.chain(&lowered, &mut segments, key);
    let tempo_map = std::mem::take(&mut c.tempo_map);
    capability_warnings(&c, &mut diags);
    diags.extend(std::mem::take(&mut c.diags));
    settle(&mut diags, &lowered, src);
    if let Some(first) = lowered.launches.first().filter(|_| !diags.iter().any(Diagnostic::is_error)) {
        let table = crate::launch::Launches { table: lowered.launches.clone(), scenes: Default::default() };
        diags.push(Diagnostic::info(first.span, table.info()));
    }
    diags.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
    parse::name_lets(&mut diags, &parsed.scalar_lets, src);
    if diags.iter().any(Diagnostic::is_error) {
        return Check { compiled: None, diagnostics: diags };
    }
    let text = print::canonical(&as_written);
    // (a section played in several generations is lowered once a generation: its warnings are said once)
    let mut seen: Vec<(usize, usize, String)> = Vec::new();
    diags.retain(|d| {
        let k = (d.span.start, d.span.end, d.message.clone());
        if seen.contains(&k) {
            false
        } else {
            seen.push(k);
            true
        }
    });
    let warnings: Vec<Diagnostic> = diags.clone();
    let arranged = lowered.arranged || flat_chain;
    let decks = if arranged { segments.first().map(|s| s.decks.clone()).unwrap_or_default() } else { c.decks };
    let mut compiled = Compiled {
        cid: program_cid(&text),
        text,
        tempo: c.tempo,
        swing: c.swing,
        key: c.key,
        decks,
        controls: number_duplicates(c.controls),
        warnings,
        arranged,
        segments,
        end: if lowered.end_default && top_total.is_some() { top_total } else { lowered.end },
        looped: lowered.looped || flat_chain,
        loop_from: if flat_chain { 0 } else { lowered.loop_from },
        flow: lowered.flow.clone(),
        flat_clock: flat_chain,
        lineage,
        evolves,
        section_keys,
        linked: crate::linked::structure(&lowered),
        tempo_map,
        assertions: Vec::new(),
        source: src.to_string(),
        rights,
        set: None,
        station: None,
        plays: parse::closure_of(&parsed.imports),
        sing: c.sing_lines,
        launches: crate::launch::Launches { table: lowered.launches.clone(), scenes: Default::default() },
        flat_lines: lowered.globals.iter().flat_map(|f| std::iter::once(f.span).chain(f.via.iter().copied())).collect(),
        notes: notes_of(&parsed.comments, src),
    };
    // F12: the assertions. A name that is nowhere is an error; a false static one is one too when enforcing; a level one is a note
    // (a scene is not judged: its assertions are the program's, about the song as written)
    let (checked, mut adiags) = if scene.is_some() { (Vec::new(), Vec::new()) } else { crate::assertion::check(&parsed.program, &compiled) };
    if enforce {
        for c in checked.iter().filter(|c| c.status == crate::assertion::Status::Fail) {
            let d = Diagnostic::error(c.span, c.line());
            adiags.push(match &c.help {
                Some(h) => d.help(h.clone()),
                None => d,
            });
        }
    }
    if !adiags.is_empty() {
        diags.extend(adiags.iter().cloned());
        diags.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
        if adiags.iter().any(Diagnostic::is_error) {
            return Check { compiled: None, diagnostics: diags };
        }
        compiled.warnings.extend(adiags);
        compiled.warnings.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
    }
    compiled.assertions = checked;
    // each scene a `launch` names, compiled as a program of its own: the song replaced by it (launch.rs). A line that counts from a section
    // the scene has not got does not fire there, and is said where it is; an error in the scene is said where it is, and says which scene.
    if scene.is_none() && !compiled.launches.is_empty() {
        let mut failed = false;
        for l in lowered.launches.iter() {
            if compiled.launches.scenes.contains_key(&l.scene) {
                continue;
            }
            let sub = check_mode_in(src, env, false, Some(l));
            let line = crate::diag::line_col(src, l.span.start).0;
            match sub.compiled {
                Some(v) => {
                    compiled.launches.scenes.insert(l.scene.clone(), Box::new(v));
                    // what the scene leaves out: a line that counts from a section it has not got is said where it is
                    for d in sub.diagnostics.iter().filter(|d| d.message.contains("is not in the scene")) {
                        let said = Diagnostic { help: Some(format!("the `launch` on line {line} plays the scene on its own, so what it counts from must be in it")), ..d.clone() };
                        if !diags.contains(&said) {
                            diags.push(said.clone());
                            compiled.warnings.push(said);
                        }
                    }
                }
                None => {
                    failed = true;
                    for d in sub.diagnostics.iter().filter(|d| d.is_error()).take(3) {
                        let said = format!("in the scene `{}` (the `launch` on line {line}), which plays on its own", l.scene);
                        diags.push(Diagnostic { help: Some(d.help.as_ref().map_or(said.clone(), |h| format!("{h} ({said})"))), ..d.clone() });
                    }
                }
            }
        }
        diags.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
        compiled.warnings.sort_by_key(|d| (d.span.start, d.severity != crate::diag::Severity::Error));
        if failed {
            return Check { compiled: None, diagnostics: diags };
        }
    }
    Check { compiled: Some(compiled), diagnostics: diags }
}

fn number_duplicates(mut cs: Vec<Control>) -> Vec<Control> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for c in &mut cs {
        let n = seen.entry(c.key.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            c.key = format!("{}#{}", c.key, n);
        }
    }
    cs
}

pub(crate) struct Compiler<'a> {
    pub(crate) src: &'a str,
    pub(crate) env: Env<'a>,
    pub(crate) diags: Vec<Diagnostic>,
    pub(crate) tempo: Option<f64>,
    pub(crate) swing: Option<f64>,
    pub(crate) key: Option<Key>,
    pub(crate) decks: BTreeMap<Deck, DeckIr>,
    pub(crate) controls: Vec<Control>,
    /// The program's samples and loops, `name -> else voice`: `inst=NAME` is one of these.
    pub(crate) samples: BTreeMap<String, Option<String>>,
    /// How many bars a pattern with chance is unrolled over in the scope being built: a section's length (up to a net),
    /// [`crate::chance::DEFAULT_BARS`] with no sections (F7).
    pub(crate) chance_bars: u32,
    /// Every tempo the program says, in the order it said them: the top-level settings, then the lines (F3, tempo.rs).
    pub(crate) tempos: Vec<crate::tempo::TempoEvent>,
    /// The map of `tempos`, once every line is read ([`Compiler::settle_tempo`]).
    pub(crate) tempo_map: crate::tempo::TempoMap,
    /// The bar (of the song) the section whose control lines are being compiled ends on: an lfo with no `over` stops
    /// there (F4, shape.rs). None outside a section.
    pub(crate) seg_end: Option<u32>,
    /// The section a `sing` line being compiled belongs to (its index among the segments, and its first bar), for a sung
    /// part that goes into that section's nets (sing.rs, "In the nets").
    pub(crate) sing_into: Option<(usize, u32)>,
    /// The sung parts to add to the decks' nets once every line is compiled.
    pub(crate) sung: Vec<crate::sing::SungPart>,
    /// Every `sing` line's windows, for the constraints (constraint.rs): the same whatever the host.
    pub(crate) sing_lines: Vec<crate::sing::SingLine>,
}

/// A part gathered from the program, before its notes are read.
#[derive(Clone, Copy)]
pub(crate) struct RawPart<'a> {
    pub(crate) part: &'a Part,
    /// The line it is on, and the calls and repeats that expanded it (innermost first).
    pub(crate) line: Span,
    pub(crate) via: &'a [Span],
    /// F10: the bar of its section this part's stretch begins at (a stacked section's later stretches): the
    /// part's cycles are rotated to start there. 0 for everything else.
    pub(crate) phase: u32,
}

/// The MIDI note an event of a part plays: its own, or (a hit) its drum's, or the key's tonic in the part's octave.
fn hit_note(role: &str, key: Key, octave: i32, sound: Sound) -> i32 {
    match sound {
        Sound::Note(n) => n,
        Sound::Hit => match role {
            "kick" => 36,
            "snare" => 38,
            "hihat" => 42,
            "clap" => 39,
            _ => music::midi(key.tonic, octave),
        },
    }
}

pub(crate) fn fnv(s: &str) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    ((h >> 32) ^ h) as u32
}

impl<'a> Compiler<'a> {
    pub(crate) fn err(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    pub(crate) fn settings(&mut self, p: &Program) {
        for s in &p.stmts {
            let StmtKind::Setting(set) = &s.kind else { continue };
            match set {
                Setting::Tempo(n) => {
                    if !(40.0..=300.0).contains(&n.value) {
                        self.err(Diagnostic::error(n.span, "a tempo is 40 to 300 BPM").found(n.span.text(self.src).to_string()));
                    } else {
                        self.tempos.push(crate::tempo::TempoEvent::set(crate::rat::Rat::ZERO, n.value, s.span));
                    }
                    if self.tempo.is_some() {
                        self.diags.push(Diagnostic::warning(s.span, "tempo is set twice; the last one wins"));
                    }
                    self.tempo = Some(n.value);
                }
                Setting::TempoRamp(r) => {
                    if self.tempo.is_some() {
                        self.diags.push(Diagnostic::warning(s.span, "tempo is set twice; the last one wins"));
                    }
                    if let Some(first) = self.tempo_setting(s.span, &crate::tempo::TempoSpec::Ramp(r.clone())) {
                        self.tempo = Some(first);
                    }
                }
                Setting::Swing(n) => {
                    if !(0.0..=1.0).contains(&n.value) {
                        self.err(Diagnostic::error(n.span, "swing is 0 to 1").found(n.span.text(self.src).to_string()));
                    }
                    if self.swing.is_some() {
                        self.diags.push(Diagnostic::warning(s.span, "swing is set twice; the last one wins"));
                    }
                    self.swing = Some(n.value);
                }
                Setting::Key(k) => match Key::parse(&k.text) {
                    Some(key) => {
                        if self.key.is_some() {
                            self.diags.push(Diagnostic::warning(s.span, "key is set twice; the last one wins"));
                        }
                        self.key = Some(key);
                    }
                    None => self.err(
                        Diagnostic::error(k.span, format!("`{}` is not a key", k.text))
                            .expected(&["a key like am, c, f#m or d:dorian"])
                            .found(format!("`{}`", k.text))
                            .help(format!("modes: {}", music::MODES.join(", "))),
                    ),
                },
            }
        }
    }

    /// Gather a scope's part lines by deck (a role twice on one deck is an error).
    pub(crate) fn gather<'l>(&mut self, lines: &'l [FlatLine]) -> BTreeMap<Deck, Vec<RawPart<'l>>> {
        let mut raw: BTreeMap<Deck, Vec<RawPart>> = BTreeMap::new();
        for fl in lines {
            let Body::Parts(ps) = &fl.line.body else { continue };
            let Some((Target::Deck(d), _)) = fl.line.target else { continue };
            let slot = raw.entry(d).or_default();
            for part in ps {
                // (a right copy follows the part it copies: a part of the same role, on purpose)
                if let Some(first) = slot.iter().find(|r| r.part.role.text == part.role.text).filter(|_| !part.right) {
                    let at = crate::diag::line_col(self.src, first.part.role.span.start);
                    self.diags.push(
                        Diagnostic::error(part.role.span, format!("deck {} already has a `{}` part (line {})", d.name(), part.role.text, at.0))
                            .help("one part per role: put two sounds in one pattern with [a, b], or use another role"),
                    );
                    continue;
                }
                slot.push(RawPart { part, line: fl.span, via: &fl.via, phase: fl.phase.unwrap_or(0) });
            }
        }
        raw
    }

    /// The control-deck lines a switch of an `arrange { }` holds back ([`expand::Lowered::quiet`], docs/beatscript/ext-control.md): each
    /// is compiled as a control line is (a `duck` or a `mix` by the mix pass, in the section it was held back in), for what the
    /// compiler says of it, and kept nowhere (no control, no tempo event), so a line that no play fires is checked all the same.
    /// What a line said where another play fires it is not said twice.
    pub(crate) fn check_quiet(&mut self, lw: &expand::Lowered) {
        if lw.quiet.is_empty() {
            return;
        }
        let (controls, tempos, said) = (self.controls.len(), self.tempos.len(), self.diags.len());
        let seg_end = self.seg_end.take();
        for (seg, fl) in &lw.quiet {
            // an lfo with no `over` runs to the end of the section it was held back in, as where it plays (F4)
            self.seg_end = seg.and_then(|i| lw.segments.get(i)).map(|s| crate::mixmoves::section_span(fl, s, lw).1);
            self.controls_of(std::slice::from_ref(fl));
        }
        self.seg_end = None;
        // a `duck` and a `mix` are the mix pass's (mixmoves.rs), read in the section they were held back in
        self.check_quiet_mixes(lw);
        self.controls.truncate(controls);
        self.tempos.truncate(tempos);
        self.seg_end = seg_end;
        for d in self.diags.split_off(said) {
            if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                self.diags.push(d);
            }
        }
    }

    pub(crate) fn controls_of(&mut self, lines: &[FlatLine]) {
        for fl in lines {
            // (F10: a line carried into a later stretch of a stacked section already fired in the first)
            if fl.phase.is_some() {
                continue;
            }
            match &fl.line.body {
                Body::Action(_) | Body::Do(_) => self.control(fl),
                _ => {}
            }
        }
    }

    /// Every `evolve`: each section (or the whole program) it is in becomes one segment per generation,
    /// the parts of generation n+1 made from those of generation n by its rules. The generations and where
    /// they came from are the lineage.
    pub(crate) fn evolve(&mut self, lw: &expand::Lowered, segments: &mut Vec<Segment>) -> (Vec<Generation>, Vec<EvolveRun>, Option<u32>) {
        let mut lineage = Vec::new();
        let mut runs: Vec<EvolveRun> = Vec::new();
        let mut top_total = None;
        let mut order: Vec<&(usize, EvolveBlock)> = lw.evolves.iter().collect();
        order.sort_by_key(|(k, _)| std::cmp::Reverse(*k));
        let mut made: Vec<(usize, Vec<Generation>)> = Vec::new();
        for (k, block) in order {
            let Some(mut seg) = segments.get(*k).cloned() else { continue };
            let before = self.diags.len();
            crate::evolve::check(block, &seg.decks, &mut self.diags);
            if self.diags[before..].iter().any(Diagnostic::is_error) {
                continue;
            }
            let analysis = crate::evolve::analyse(block, &seg.decks);
            // the last generation: said, or (a section) as many as fit, or (a program of its own) its orbit
            let last = match block.last {
                Some((n, _)) => n,
                None if lw.top_unbounded && *k == 0 => match analysis.period {
                    Some(p) if u64::from(analysis.tail) + p <= u64::from(crate::evolve::MAX_GENS) => {
                        let g = analysis.tail + p as u32;
                        seg.bars = (g + 1) * block.every;
                        segments[*k].bars = seg.bars;
                        top_total = Some(seg.bars);
                        g
                    }
                    _ => {
                        self.err(Diagnostic::error(block.span, format!("this evolve has no `stop`, and its orbit is {} (tail {}): too long to play through", analysis.period.map_or("not found".to_string(), |p| format!("{} generations", crate::evolve::group(p))), analysis.tail)).help(format!("say where it stops: `stop when gen == N` (up to {})", crate::evolve::MAX_GENS)));
                        continue;
                    }
                },
                None => {
                    let fit = seg.bars.div_ceil(block.every).saturating_sub(1);
                    if fit > crate::evolve::MAX_GENS {
                        self.diags.push(Diagnostic::warning(block.span, format!("{} generations fit this section: it evolves {} times and plays the last generation to its end", fit + 1, crate::evolve::MAX_GENS)).help("a longer `every`, or `stop when gen == N`"));
                    }
                    fit.min(crate::evolve::MAX_GENS)
                }
            };
            runs.push(EvolveRun { scope: seg.name.clone(), block: block.clone(), gen0: seg.decks.clone(), analysis, last, bars: seg.bars });
            let mut gens: Vec<Segment> = Vec::new();
            let mut cur = seg.decks.clone();
            let mut forked: Option<String> = None;
            let mut g = 0u32;
            let mut gl = Vec::new();
            let mut net_cids = crate::evolve::NetCids::default();
            loop {
                let start = g * block.every;
                if start >= seg.bars {
                    self.diags.push(Diagnostic::warning(block.span, format!("generations {g} to {last} never play: the section is {} bars and a generation is {}", seg.bars, block.every)));
                    break;
                }
                // the last generation plays on to the end of the section
                let bars = if g >= last { seg.bars - start } else { block.every.min(seg.bars - start) };
                let cid = crate::evolve::generation_cid_with(&cur, &mut net_cids);
                gl.push(Generation { scope: seg.name.clone(), gen: g, cid: cid.clone(), forked_from: forked.clone(), start: seg.start + start, bars });
                forked = Some(cid);
                gens.push(Segment { name: seg.name.clone(), start: seg.start + start, bars, decks: cur.clone(), gen: Some(g), header: seg.header, lines: seg.lines.clone(), chain: seg.chain || g > 0 });
                if g >= last {
                    break;
                }
                g += 1;
                cur = crate::evolve::step(block, &cur, g);
            }
            segments.splice(*k..=*k, gens);
            made.push((*k, gl));
        }
        made.sort_by_key(|(k, _)| *k);
        for (_, gl) in made {
            lineage.extend(gl);
        }
        runs.reverse();
        (lineage, runs, top_total)
    }

    /// Nets are at most [`MAX_BARS`] long. A deck whose parts take longer to come round is cut into stretches
    /// of that many bars, each its own net, loaded on the bar line the one before ends on: the same notes, so the
    /// song can be as long as its timeline. Decks that fit are left as they are (the same net across the
    /// stretches, so it is not reloaded). A program with no sections is given a timeline of the bars its
    /// decks take to come round together, and loops. Answers whether it made one.
    pub(crate) fn chain(&mut self, lw: &expand::Lowered, segments: &mut Vec<Segment>, key: Key) -> bool {
        let long = |ir: &DeckIr| ir.period as u64 > MAX_BARS;
        let _ = key;
        if !lw.arranged {
            if !self.decks.values().any(long) {
                return false;
            }
            let mut total = 1u64;
            for ir in self.decks.values() {
                total = lcm(total, ir.period as u64);
            }
            if total > MAX_SONG_BARS {
                self.err(Diagnostic::error(Span::default(), format!("the decks take {total} bars to come round together; the most is {MAX_SONG_BARS}")).help("give the parts alternations whose lengths share factors"));
                return false;
            }
            let decks = std::mem::take(&mut self.decks);
            segments.clear();
            segments.push(Segment { name: "main".into(), start: 0, bars: total as u32, decks: decks.clone(), gen: None, header: None, lines: lw.globals.iter().flat_map(|f| std::iter::once(f.span).chain(f.via.iter().copied())).collect(), chain: false });
            self.decks = decks;
            self.diags.push(Diagnostic::warning(Span::default(), format!("this program takes {total} bars to come round: it plays as nets of {MAX_BARS} bars chained, and loops")).help("`end at bar N` stops it; `section` and `arrange` say its structure"));
        }
        let mut out: Vec<Segment> = Vec::new();
        for seg in segments.drain(..) {
            if !seg.decks.values().any(long) {
                out.push(seg);
                continue;
            }
            let mut k = 0u32;
            while k * MAX_BARS as u32 <= seg.bars.saturating_sub(1) {
                let from = k * MAX_BARS as u32;
                let len = (seg.bars - from).min(MAX_BARS as u32);
                let mut decks = BTreeMap::new();
                for (d, ir) in &seg.decks {
                    if long(ir) {
                        let stretch = |grid: &[Vec<Hit>]| -> Vec<Vec<Hit>> { if grid.is_empty() { Vec::new() } else { (0..len as usize).map(|i| grid[(from as usize + i) % grid.len()].clone()).collect() } };
                        let long_part = |p: &PartIr| PartIr { cycles: stretch(&p.cycles), may: MayGrid(stretch(&p.may.0)), ..p.clone() };
                        // (a `jux`'d part's twin is stretched with it)
                        let parts = ir.parts.iter().map(|p| PartIr { twin: p.twin.as_ref().map(|t| Box::new(long_part(t))), ..long_part(p) }).collect();
                        let tempo = self.tempo_for_net(seg.start + from).or(ir.tempo);
                        decks.insert(*d, DeckIr { period: len as usize, parts, tempo, ..ir.clone() });
                    } else {
                        decks.insert(*d, ir.clone());
                    }
                }
                out.push(Segment { name: seg.name.clone(), start: seg.start + from, bars: len, decks, gen: seg.gen, header: seg.header, lines: seg.lines.clone(), chain: seg.chain || k > 0 });
                k += 1;
            }
        }
        *segments = out;
        !lw.arranged
    }

    /// `route=td3`: the part's first bar as the TD-3 editor's sixteen steps (note, octave shift,
    /// accent, slide, rest), one `td3 step` call each, once when its net lands. Sending it to the
    /// hardware is `do td3 write confirm=true`: a dry run unless asked.
    pub(crate) fn td3_control(&mut self, deck: Deck, section: Option<&str>, ir: &DeckIr, timing: Timing) {
        let Some(part) = ir.td3_part() else { return };
        const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let hits = part.cycles.first().cloned().unwrap_or_default();
        let mut calls = Vec::new();
        for step in 0..16u8 {
            let h = hits.iter().find(|h| h.step == step);
            let mut o = vec![("action", JsonValue::str("step")), ("step", JsonValue::int(i64::from(step)))];
            match h {
                Some(h) => {
                    let n = i32::from(h.note);
                    o.push(("note", JsonValue::str(NAMES[(n.rem_euclid(12)) as usize])));
                    o.push(("transpose", JsonValue::str(if n < 36 { "down" } else if n >= 48 { "up" } else { "none" })));
                    o.push(("accent", h.accent.into()));
                    o.push(("slide", h.slide.into()));
                    o.push(("rest", false.into()));
                }
                None => {
                    o.push(("rest", true.into()));
                    o.push(("accent", false.into()));
                    o.push(("slide", false.into()));
                }
            }
            calls.push(Call { noun: "td3".into(), params: JsonValue::Object(JsonObject::from_pairs(o)) });
        }
        calls.push(Call::new("td3", [("action", "step".into()), ("active", JsonValue::int(16))]));
        let label = section.map_or(String::new(), |s| format!("[{s}] "));
        self.controls.push(Control { key: format!("{label}route td3 deck {} {}", deck.name(), part.role), timing, effect: Effect::Calls(calls), span: Span::default(), hold: None, undo: Vec::new(), restore: None, via: Vec::new(), notice: None, window: None, lead: None });
    }

    /// L-15: a line in a section whose time comes after the section ends (`at bar 9` in a 4-bar section, an `every 8 bars` that
    /// first fires on bar 8) does not fire in it: a section counts its own bars from 0. Said once for the line, with the bar and
    /// the section's length. A ramp that starts inside and takes longer than the section is said too (it goes on after it).
    /// `controls[from..]` are the controls the line made. Words that name a section (`after drop`) or a phrase are not numbers
    /// of bars and are left alone.
    fn beyond_its_section(&mut self, fl: &FlatLine, seg: &expand::SegmentLines, lw: &expand::Lowered, from: usize) {
        use crate::dur::Pos;
        if fl.phase.is_some() {
            return;
        }
        let counted = matches!(&fl.line.when, Some((When::At(_) | When::AtPos(_) | When::Every(_) | When::EveryFrom(..) | When::EveryPos(..), _)));
        let (start, end) = crate::mixmoves::section_span(fl, seg, lw);
        let name = crate::mixmoves::section_of_path(&fl.path).map_or_else(|| seg.name.clone(), |(n, _)| n.to_string());
        let bars = end.saturating_sub(start);
        let long = format!("{bars} bar{}", if bars == 1 { "" } else { "s" });
        let (s, e) = (Rat::int(i64::from(start)), Rat::int(i64::from(end)));
        let at = fl.line.when.as_ref().map_or(fl.span, |(_, sp)| *sp);
        let rel = |r: Rat| Pos::from_bars(r.sub(s).unwrap_or(r)).map_or_else(|| format!("bar {}", r.sub(s).unwrap_or(r)), Pos::canonical);
        // (once for a word in the text: the copies of a `repeat` and a section placed twice share it)
        let said = format!("of section {name}, which is only");
        if self.diags.iter().any(|d| d.span == at && d.message.contains(&said)) {
            return;
        }
        for c in &self.controls[from..] {
            let (first, repeats) = match c.timing {
                Timing::At(b) => (Rat::int(i64::from(b)), false),
                Timing::AtPos(p) => (p, false),
                Timing::EveryFrom(_, f) => (Rat::int(i64::from(f)), true),
                Timing::EveryPos { from, .. } => (from, true),
                _ => continue,
            };
            // (a macro's call goes a beat ahead: its line's own time is what the section has or has not)
            let first = c.lead.and_then(|l| first.add(l).ok()).unwrap_or(first);
            if first >= e {
                if !counted {
                    continue;
                }
                let what = if repeats { "first fires" } else { "is due" };
                let msg = format!("this line {what} at {} of section {name}, which is only {long} long (bars 0 to {}): it does not fire in the section", rel(first), bars.saturating_sub(1));
                let help = format!("a section counts its own bars from 0, so `at bar 3` is the last bar of a 4-bar section; this is due at bar {} of the song, after the section ends", first.floor());
                self.diags.push(Diagnostic::warning(at, msg).help(help));
                return;
            }
            if !repeats {
                if let Effect::Ramp(r) = c.effect.base() {
                    if let Ok(done) = first.add(r.bars.as_bars()) {
                        if done > e {
                            let msg = format!("this ramp takes {} and starts at {} of section {name}, which is only {long} long: it is still moving when the section ends, and goes on after it", r.bars.canonical(), rel(first));
                            self.diags.push(Diagnostic::warning(at, msg).help("a ramp is not cut at the end of its section; `over` the bars that are left, or give the section more bars"));
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Everything: the controls of every scope, and the nets (for a song with a timeline, one set
    /// per section: the song's own part lines under each section's).
    pub(crate) fn build(&mut self, lw: &expand::Lowered, key: Key) -> Vec<Segment> {
        self.controls_of(&lw.globals);
        for seg in &lw.segments {
            for fl in &seg.lines {
                // an lfo with no `over` runs to the end of its section (F4): in a stack of unequal lengths that is not the end of
                // the stretch the line is in, but the bar where the section itself ends (F10, as a mix is held)
                self.seg_end = Some(crate::mixmoves::section_span(fl, seg, lw).1);
                let before = self.controls.len();
                self.controls_of(std::slice::from_ref(fl));
                self.beyond_its_section(fl, seg, lw, before);
            }
        }
        self.seg_end = None;
        self.tempo_clashes(lw);
        // every tempo is said: the map, and the tempo the program starts at (F3); a duck's pump is chosen at that tempo (F8), so
        // this comes before the mix lines are read
        self.settle_tempo();
        self.mix_controls(lw);
        let global = self.gather(&lw.globals);
        if !lw.arranged {
            for (deck, parts) in &global {
                if let Some(ir) = self.deck(*deck, parts, key) {
                    self.td3_control(*deck, None, &ir, Timing::Now);
                    self.decks.insert(*deck, ir);
                }
            }
            let decks = std::mem::take(&mut self.decks);
            self.sing_controls(&lw.globals, &decks);
            self.decks = decks;
            for sp in std::mem::take(&mut self.sung) {
                crate::sing::add_sung_part(&mut self.decks, sp);
            }
            return Vec::new();
        }
        let mut out = Vec::new();
        let program_key = key;
        for seg in &lw.segments {
            // (F10: a later stretch of a stack begins `phase` bars into its section, so its chance is unrolled over those bars
            // and the stretch's own: the net the part is rotated from says every bar the section has said by then)
            self.chance_bars = seg.phase.saturating_add(seg.bars).clamp(1, crate::chance::MAX_UNROLL);
            // a section's own `key` re-keys its degrees, numerals and markov default (F5)
            let key = crate::scale::key_of(seg, program_key);
            // the nets of a section are made at the tempo a curve has where it begins (F3)
            let program_tempo = self.tempo;
            if let Some(t) = self.tempo_for_net(seg.start) {
                self.tempo = Some(t);
            }
            let own = self.gather(&seg.lines);
            let mut decks: BTreeMap<Deck, DeckIr> = BTreeMap::new();
            let mut all: Vec<Deck> = global.keys().chain(own.keys()).copied().collect();
            all.sort();
            all.dedup();
            for d in all {
                let mut parts: Vec<RawPart> = own.get(&d).cloned().unwrap_or_default();
                for g in global.get(&d).into_iter().flatten() {
                    if !parts.iter().any(|p| p.part.role.text == g.part.role.text) {
                        // (F10: in a later stretch of a stack the song's own parts go on from where they were)
                        parts.push(RawPart { phase: seg.phase, ..*g });
                    }
                }
                // the song's parts first, as written: stable whatever the section adds
                parts.sort_by_key(|p| p.part.span.start);
                if let Some(mut ir) = self.deck(d, &parts, key) {
                    // the net says the key it is in where the section re-keyed it and the deck plays pitches (a drum
                    // deck's net does not change with the key)
                    if seg.key.is_some() && ir.parts.iter().any(|p| !matches!(p.role.as_str(), "kick" | "snare" | "hihat" | "clap")) {
                        ir.key = Some(key);
                    }
                    self.td3_control(d, Some(&format!("{}@{}", seg.name, seg.start)), &ir, Timing::At(seg.start));
                    decks.insert(d, ir);
                }
            }
            // the lines that play in it: the song's own, and the section's, with the calls that made them
            let lines: Vec<Span> = lw.globals.iter().chain(seg.lines.iter()).flat_map(|f| std::iter::once(f.span).chain(f.via.iter().copied())).collect();
            let mut lines = lines;
            lines.extend(seg.header_more.iter().copied());
            self.tempo = program_tempo;
            out.push(Segment { name: seg.name.clone(), start: seg.start, bars: seg.bars, decks, gen: None, header: seg.header, lines, chain: seg.chain });
        }
        self.sing_sections(&lw.segments, &out);
        self.sing_globals(&lw.globals, &out);
        for sp in std::mem::take(&mut self.sung) {
            if let Some(seg) = sp.segment.and_then(|i| out.get_mut(i)) {
                crate::sing::add_sung_part(&mut seg.decks, sp);
            }
        }
        out
    }

    // ---- patterns

    pub(crate) fn deck(&mut self, deck: Deck, written: &[RawPart], key: Key) -> Option<DeckIr> {
        // pattern sugar (`c2~3`, `x?`, `ply(..)`, `poly(..)` ...) is its longhand from here on (sugar.rs): the parts
        // are evaluated written out, so sugar and longhand are the same nets by construction
        let mut longs = Vec::with_capacity(written.len());
        let mut sugar_failed = false;
        for r in written {
            let (long, diags) = crate::sugar::part(r.part);
            for d in diags {
                sugar_failed |= d.is_error();
                if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                    self.err(d);
                }
            }
            longs.push(long);
        }
        if sugar_failed {
            return None;
        }
        let parts: Vec<RawPart> = written.iter().zip(&longs).map(|(r, part)| RawPart { part, ..*r }).collect();
        let parts = &parts[..];
        let mut period = 1u64;
        for r in parts {
            match crate::meter::part_bars(r.part) {
                Some(p) => period = lcm(period, p),
                None => {
                    self.err(Diagnostic::error(r.part.pattern.span, "this pattern takes too many bars to come round").help(format!("the most is {MAX_SONG_BARS} bars")));
                    return None;
                }
            }
        }
        // parts with a cycle of their own (`hihat/12`) must come round inside one net (F6)
        let all: Vec<&Part> = parts.iter().map(|r| r.part).collect();
        if !self.polymeter_fits(deck, &all, period) {
            return None;
        }
        // a pattern with chance differs every bar: its net is as long as the section (F7)
        if parts.iter().any(|r| crate::chance::has_chance(&r.part.pattern)) {
            period = crate::chance::unrolled_period(period, self.chance_bars);
        }
        if period > MAX_SONG_BARS {
            let at = parts.first().map_or(Span::default(), |r| r.part.span);
            self.err(Diagnostic::error(at, format!("deck {}'s parts take {period} bars to come round together; the most is {MAX_SONG_BARS}", deck.name())).help("shorten an alternation <...> or give two parts alternations of the same length"));
            return None;
        }
        let mut out: Vec<PartIr> = Vec::new();
        let mut ok = true;
        for (i, r) in parts.iter().enumerate() {
            match self.part(deck, r.part, written[i].part, r.line, r.via, key, period as usize) {
                Some(mut p) => {
                    // F10: a stacked section's later stretch begins `phase` bars into it
                    if r.phase > 0 && !p.cycles.is_empty() {
                        for grid in p.grids_mut() {
                            let n = grid.len();
                            grid.rotate_left(r.phase as usize % n);
                        }
                    }
                    // a right copy (`jux`) is its part's twin, not a part of its own
                    match out.last_mut().filter(|_| r.part.right) {
                        Some(owner) => {
                            // a sampled voice plays on the sampler, which sits in the middle: both copies are heard there
                            let sampled = owner.kit.as_deref().is_some_and(|k| !k.starts_with("synth:")) || owner.sound.as_deref().is_some_and(|s| s.starts_with("inst:") || s.starts_with("cat:") || s.starts_with("sf:") || s.starts_with("kit:"));
                            if sampled {
                                let what = owner.kit.as_deref().map(|k| format!("kit={k}")).or_else(|| owner.sound.as_deref().map(|s| format!("sound={s}"))).unwrap_or_default();
                                self.diags.push(
                                    Diagnostic::warning(r.part.role.span, format!("`jux` on `{}` with `{what}`: a sampled voice plays in the middle, so both copies are heard there", owner.role))
                                        .help("a `synth:` voice (`sound=synth:NAME`, `kit=synth:NAME`) can be placed to one side"),
                                );
                            }
                            owner.twin = Some(Box::new(p))
                        }
                        None => out.push(p),
                    }
                }
                None => ok = false,
            }
        }
        // every part with a twin plays a second set of voices
        let twinned: Vec<&PartIr> = out.iter().filter(|p| p.twin.is_some()).collect();
        if twinned.len() > MAX_TWINNED {
            if let Some(r) = parts.iter().filter(|r| r.part.right).nth(MAX_TWINNED) {
                self.diags.push(
                    Diagnostic::warning(r.part.role.span, format!("deck {} has {} parts with a right copy (`jux`): each plays a second set of voices, and a phone may drop notes", deck.name(), twinned.len()))
                        .help(format!("{MAX_TWINNED} is plenty: jux the parts that matter")),
                );
            }
        }
        ok.then_some(DeckIr { deck, period: period as usize, parts: out, tempo: self.tempo, swing: self.swing, key: self.key })
    }

    /// `inst=name` against the engine's voices: an exact one, a family word (it plays the family's
    /// generic voice: said), or an error with the nearest.
    pub(crate) fn check_voice(&mut self, w: &Word, drum: bool) -> bool {
        let name = w.text.as_str();
        if let Some(stand_in) = self.samples.get(name).cloned() {
            // the engine has no sampler for a user's file on a net yet: the part plays the stand-in
            let plays = stand_in.map_or_else(|| "the role's default voice".to_string(), |v| format!("`{v}`"));
            self.diags.push(Diagnostic::warning(w.span, format!("`{name}` is a sample the engine cannot play yet: this part plays {plays}")).help("`else VOICE` on the sample line names the stand-in"));
            return true;
        }
        let Some(cat) = self.env.catalog else { return true };
        let voices = cat.voices();
        if voices.iter().any(|v| v == name) {
            return true;
        }
        let fam = cat.families();
        if let Some(f) = fam.iter().find(|f| name.contains(f.as_str())) {
            self.diags.push(
                Diagnostic::warning(w.span, format!("`{name}` has no voice of its own: it plays the generic `{f}` family voice (a plain one)"))
                    .help(format!("a voice with a sound of its own: {} (`btw instruments` lists them all)", pick(name, &voices))),
            );
            return true;
        }
        if drum && cat.kits().iter().any(|k| k == name) {
            return true;
        }
        let mut all = voices.clone();
        all.extend(fam);
        let mut d = Diagnostic::error(w.span, format!("`{name}` is not an instrument the engine has")).found(format!("`{name}`"));
        let near = crate::env::suggest(name, &all, 3);
        d = d.help(if near.is_empty() { "`btw instruments` lists every voice, palette and kit".to_string() } else { format!("did you mean {}? (`btw instruments` lists them all)", near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")) });
        self.err(d);
        false
    }

    pub(crate) fn check_kit(&mut self, w: &Word) -> bool {
        let Some(cat) = self.env.catalog else { return true };
        let kits = cat.kits();
        let name = w.text.strip_prefix("synth:").unwrap_or(&w.text);
        if kits.iter().any(|k| k == name) {
            return true;
        }
        let mut d = Diagnostic::error(w.span, format!("`{}` is not a drum kit", w.text)).expected(&kits.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{}`", w.text));
        if let Some(n) = crate::env::suggest(name, &kits, 1).first() {
            d = d.help(format!("did you mean `{n}`?"));
        }
        self.err(d);
        false
    }

    /// `sound=own|synth|synth:NAME|inst:NAME|cat:...`: a role's sound option.
    pub(crate) fn check_sound(&mut self, w: &Word) -> bool {
        if self.env.catalog.is_none() {
            return true;
        }
        let t = w.text.as_str();
        if matches!(t, "own" | "synth") {
            return true;
        }
        if let Some(name) = t.strip_prefix("synth:").or_else(|| t.strip_prefix("inst:")) {
            return self.check_voice(&Word { text: name.to_string(), span: w.span }, false);
        }
        if t.starts_with("cat:") || t.starts_with("sf:") || t.starts_with("kit:") {
            return true;
        }
        self.err(Diagnostic::error(w.span, format!("`{t}` is not a sound option")).expected(&["own", "synth", "synth:NAME", "inst:NAME", "cat:CATEGORY"]).found(format!("`{t}`")));
        false
    }

    /// `part` is the part written out (sugar.rs), `written` as the text has it (its canonical text, its notes).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn part(&mut self, deck: Deck, part: &Part, written: &Part, line: Span, via: &[Span], key: Key, period: usize) -> Option<PartIr> {
        let role = part.role.text.as_str();
        let drum = matches!(role, "kick" | "snare" | "hihat" | "clap");
        let mut octave = music::role_octave(role);
        let mut len = if drum { 1 } else { 2 };
        let mut vel: Option<(i32, i32)> = None;
        let mut inst = None;
        let mut sound = None;
        let mut kit = None;
        let mut route = None;
        let mut seed: Option<u32> = None;
        let mut lead = false;
        let mut expr = PartExpr::default();
        let mut seen: Vec<&str> = Vec::new();
        let mut bad = false;
        for pr in &part.params {
            let name = pr.name.text.as_str();
            if seen.contains(&name) {
                self.err(Diagnostic::error(pr.name.span, format!("`{name}` is given twice")));
                bad = true;
                continue;
            }
            seen.push(name);
            let ranged = |lo: f64, hi: f64, c: &mut Self| -> Option<(i32, i32)> {
                let (a, b) = match &pr.value {
                    ParamValue::Num(n) => (n, n),
                    ParamValue::Range(a, b) => (a, b),
                    ParamValue::Word(w) => {
                        c.err(Diagnostic::error(w.span, format!("`{name}` is a number")).found(format!("`{}`", w.text)));
                        return None;
                    }
                };
                for n in [a, b] {
                    if n.value.fract() != 0.0 || n.value < lo || n.value > hi {
                        c.err(Diagnostic::error(n.span, format!("`{name}` is a whole number {lo} to {hi}")).found(n.span.text(c.src).to_string()));
                        return None;
                    }
                }
                if a.value > b.value {
                    c.err(Diagnostic::error(a.span.to(b.span), "a range goes up: low..high"));
                    return None;
                }
                Some((a.value as i32, b.value as i32))
            };
            match name {
                "vel" => match ranged(1.0, 127.0, self) {
                    Some(r) => vel = Some(r),
                    None => bad = true,
                },
                "oct" => match ranged(-1.0, 8.0, self) {
                    Some((a, b)) if a == b => octave = a,
                    Some(_) => {
                        self.err(Diagnostic::error(pr.span, "`oct` is one octave, not a range"));
                        bad = true;
                    }
                    None => bad = true,
                },
                "len" => match ranged(1.0, 16.0, self) {
                    Some((a, b)) if a == b => len = a,
                    Some(_) => {
                        self.err(Diagnostic::error(pr.span, "`len` is one length, not a range"));
                        bad = true;
                    }
                    None => bad = true,
                },
                "seed" => match ranged(0.0, 4_294_967_295.0, self) {
                    Some((a, b)) if a == b => seed = Some(a as u32),
                    Some(_) => {
                        self.err(Diagnostic::error(pr.span, "`seed` is one number, not a range"));
                        bad = true;
                    }
                    None => bad = true,
                },
                "inst" => match &pr.value {
                    ParamValue::Word(w) => {
                        if self.check_voice(w, drum) {
                            // a sample plays its stand-in voice (or the default)
                            inst = match self.samples.get(w.text.as_str()) {
                                Some(stand_in) => stand_in.clone(),
                                None => Some(w.text.clone()),
                            };
                        } else {
                            bad = true;
                        }
                    }
                    other => {
                        let sp = match other {
                            ParamValue::Num(n) => n.span,
                            ParamValue::Range(a, b) => a.span.to(b.span),
                            ParamValue::Word(w) => w.span,
                        };
                        self.err(Diagnostic::error(sp, "`inst` is an instrument's name").help("inst=sub-bass"));
                        bad = true;
                    }
                },
                "kit" | "sound" | "route" => match &pr.value {
                    ParamValue::Word(w) => {
                        let ok = match name {
                            "kit" if !drum => {
                                self.err(Diagnostic::error(pr.name.span, "`kit` is for a drum part (kick, snare, hihat, clap)").help("a tonal part takes `inst=` or `sound=`"));
                                false
                            }
                            "kit" => self.check_kit(w),
                            "sound" => self.check_sound(w),
                            _ => {
                                if w.text != "td3" {
                                    self.err(Diagnostic::error(w.span, format!("`{}` is not somewhere a part can be routed", w.text)).expected(&["td3"]).found(format!("`{}`", w.text)));
                                    false
                                } else if drum {
                                    self.err(Diagnostic::error(w.span, "the TD-3 plays notes: route a bass, melody, harmony or arp part"));
                                    false
                                } else {
                                    true
                                }
                            }
                        };
                        if ok {
                            match name {
                                "kit" => kit = Some(w.text.clone()),
                                "sound" => sound = Some(w.text.clone()),
                                _ => route = Some(w.text.clone()),
                            }
                        } else {
                            bad = true;
                        }
                    }
                    _ => {
                        self.err(Diagnostic::error(pr.span, format!("`{name}` is a name, not a number")));
                        bad = true;
                    }
                },
                "glide" | "decay" | "pitch-drop" if drum => {
                    self.err(Diagnostic::error(pr.name.span, format!("`{name}` is for a tonal part (bass, melody, harmony, arp)")).help("a drum part takes accents `x!`, ghosts `x.`, flams `x&` and ratchets `x:3`"));
                    bad = true;
                }
                "glide" | "decay" | "pitch-drop" => {
                    let (lo, hi) = match name {
                        "glide" => GLIDE_MS,
                        "decay" => DECAY_MS,
                        _ => PITCH_DROP,
                    };
                    match ranged(lo, hi, self) {
                        Some((a, b)) if a == b => match name {
                            "glide" => expr.glide_ms = Some(a as u16),
                            "decay" => expr.decay_ms = Some(a as u16),
                            _ => expr.pitch_drop = Some(a as u8),
                        },
                        Some(_) => {
                            self.err(Diagnostic::error(pr.span, format!("`{name}` is one number, not a range")));
                            bad = true;
                        }
                        None => bad = true,
                    }
                }
                "cutoff" | "res" | "env" | "accent" | "drive" if drum => {
                    self.err(Diagnostic::error(pr.name.span, format!("`{name}` is a knob of the acid voice, for a tonal part (bass, melody, harmony, arp)")).help("a drum part takes accents `x!`, ghosts `x.`, flams `x&` and ratchets `x:3`"));
                    bad = true;
                }
                "cutoff" | "res" | "env" | "accent" | "drive" => {
                    let (lo, hi) = if name == "cutoff" { ACID_CUTOFF } else { ACID_KNOB };
                    match ranged(lo, hi, self) {
                        Some((a, b)) if a == b => match name {
                            "cutoff" => expr.acid.cutoff_hz = a as u16,
                            "res" => expr.acid.res = Some(a as u8),
                            "env" => expr.acid.env = Some(a as u8),
                            "accent" => expr.acid.accent = Some(a as u8),
                            _ => expr.acid.drive = Some(a as u8),
                        },
                        Some(_) => {
                            self.err(Diagnostic::error(pr.span, format!("`{name}` is one number, not a range")));
                            bad = true;
                        }
                        None => bad = true,
                    }
                }
                "voice-lead" => match crate::scale::voice_lead_param(self, pr, role) {
                    Some(on) => lead = on,
                    None => bad = true,
                },
                _ => {
                    let mut d = Diagnostic::error(pr.name.span, format!("`{name}` is not a parameter of a part")).expected(&PARAMS.map(|p| p)).found(format!("`{name}`"));
                    if let Some(n) = parse::nearest(name, PARAMS.iter().copied().chain(ACID_PARAMS).chain(["voice-lead"])) {
                        d = d.help(format!("did you mean `{n}`?"));
                    }
                    self.err(d);
                    bad = true;
                }
            }
        }
        // the acid voice's knobs on a voice that is not the acid voice: it has no such filter, and plays as it would without
        if !expr.acid.is_plain() && !inst.as_deref().is_some_and(slides_by_itself) {
            if let Some(pr) = part.params.iter().find(|p| ACID_PARAMS.contains(&p.name.text.as_str())) {
                let plays = inst.as_deref().map_or_else(|| "the part's default voice".to_string(), |v| format!("`{v}`"));
                self.diags.push(
                    Diagnostic::warning(pr.name.span, format!("`{}=` is a knob of the acid voice: {plays} has none, and plays as it would without it", pr.name.text))
                        .help("`inst=acid-bass` (or `acid-square`) is the voice these knobs turn"),
                );
            }
        }
        let canonical = print::part(written);
        // The implicit seed of a chance part is of the deck, the role and the pattern (L-05): nudging a `vel=` or swapping
        // an `inst=` does not re-roll which hits drop. A part with no chance keeps the text it always hashed (the params
        // too), so a `markov` part's notes, and the CIDs of every old program, do not move.
        // (of the part written out: sugar is the notes of its longhand, seeds and all; with no sugar it is the canonical text)
        let seed_text = if crate::chance::has_chance(&part.pattern) { print::part_pattern(part) } else { print::part(part) };
        let seed = seed.unwrap_or_else(|| fnv(&format!("{}:{}", deck.name(), seed_text)));
        let base_vel = vel.map_or(100, |(a, b)| (a + b) / 2);
        let before = self.diags.len();
        let cycle_steps = crate::meter::cycle_steps(part);
        let mut table = crate::pattern::SpanTable::default();
        let mut diags = Vec::new();
        // the part's cycles, read once as it plays (a chance as the text and the bar roll it) and, for a part with chance, once
        // more with every chance kept (`all`: what it may play, constraint.rs): same reading, so the second says nothing new
        let read = |all: bool, table: &mut crate::pattern::SpanTable, diags: &mut Vec<Diagnostic>| -> (Vec<Vec<Hit>>, bool) {
            let mut cycles: Vec<Vec<Hit>> = vec![Vec::new(); period];
            let mut snapped = false;
            let hit_root = (!drum).then(|| music::midi(key.tonic, octave));
            let mut ctx = Ctx { key, octave, vel: base_vel, seed, hit_root, src: self.src, diags, spans: table, chance: crate::chance::ChanceCtx { all, ..Default::default() }, scale: crate::scale::ScaleCtx::new(role, lead), reads: 0, over: false };
            // a part with a cycle of its own (`hihat/12`) says `period * 16 / N` cycles in the net, each `N` steps long
            for c in 0..crate::meter::cycles_in_net(period, cycle_steps) {
                ctx.chance.bar = c as i64;
                ctx.chance.slot = Some((crate::rat::Rat::ZERO, crate::rat::Rat::ONE));
                let evs = pattern::eval_part(&part.pattern, c as i64, &mut ctx);
                for e in evs {
                    let pos = e.at * f64::from(cycle_steps);
                    let rounded = (pos + 0.5 + 1e-9).floor() as i64;
                    if (pos - rounded as f64).abs() > 1e-6 {
                        snapped = true;
                    }
                    let (cyc, step) = crate::meter::place(c, rounded, cycle_steps, period);
                    let step = i64::from(step);
                    let note = hit_note(role, key, octave, e.sound);
                    if !(0..=127).contains(&note) {
                        ctx.diags.push(Diagnostic::error(part.pattern.span, format!("a note is out of the MIDI range (it came to {note})")).help("check `oct=`"));
                        continue;
                    }
                    let v = match vel {
                        Some((lo, hi)) if lo != hi => {
                            let h = fnv(&format!("{}:{}:{}:{}:{}", deck.name(), role, cyc, step, note));
                            lo + (h % (hi - lo + 1) as u32) as i32
                        }
                        Some((lo, _)) => lo,
                        None => e.vel.unwrap_or(100),
                    };
                    let v = if e.accent { v + 27 } else { v };
                    // a ghost: a fraction of the velocity it would have had
                    let v = if e.marks.ghost { (v * GHOST_PERCENT + 50) / 100 } else { v };
                    let hit = Hit { step: step as u8, note: note as u8, dur: len as u8, vel: v.clamp(1, 127) as u8, accent: e.accent, slide: e.slide, marks: e.marks, sid: e.sid, syl: 0 };
                    if !cycles[cyc].iter().any(|h| h.step == hit.step && h.note == hit.note) {
                        cycles[cyc].push(hit);
                    }
                }
            }
            (cycles, snapped)
        };
        let (mut cycles, snapped) = read(false, &mut table, &mut diags);
        let mut may = MayGrid(if crate::chance::has_chance(&part.pattern) { read(true, &mut table, &mut Vec::new()).0 } else { Vec::new() });
        self.diags.extend(diags);
        let new_errors = self.diags[before..].iter().any(Diagnostic::is_error);
        if bad || new_errors {
            return None;
        }
        if snapped && cycle_steps != crate::meter::BAR_STEPS {
            self.diags.push(
                Diagnostic::warning(part.pattern.span, format!("`{}` does not divide its {cycle_steps}-step cycle into whole steps; its hits snap to the nearest one", print::seq(&written.pattern)))
                    .help(format!("a cycle of {cycle_steps} steps is cut evenly by a term count that divides {cycle_steps}")),
            );
        } else if snapped {
            self.diags.push(
                Diagnostic::warning(part.pattern.span, format!("`{}` does not divide the bar into sixteenths; its hits snap to the nearest one", print::seq(&written.pattern)))
                    .help("steps count 4, 8, 16 fit exactly; 3 or 5 or 6 per bar snap"),
            );
        }
        // what `check` says of the Euclidean terms the ontology names (F6) and of every chance term (F7)
        self.euclid_notes(written);
        if crate::chance::has_chance(&part.pattern) {
            self.chance_notes(written, seed, period);
        }
        for c in cycles.iter_mut().chain(may.0.iter_mut()) {
            c.sort_by_key(|h| (h.step, h.note));
        }
        // `glide=` bends a slide into the next note: with no slide in the part it does nothing, which is worth a word
        if expr.glide_ms.is_some() && !cycles.iter().flatten().any(|h| h.slide) {
            if let Some(pr) = part.params.iter().find(|p| p.name.text == "glide") {
                self.diags.push(Diagnostic::warning(pr.span, "`glide=` bends a slide into the next note, and this part has no slide").help("mark the note to glide from with `^`: f1^ f2"));
            }
        }
        // a slide runs on into the next step: the note is held through it
        for c in cycles.iter_mut().chain(may.0.iter_mut()) {
            let steps: Vec<u8> = c.iter().map(|h| h.step).collect();
            for h in c.iter_mut().filter(|h| h.slide) {
                let next = steps.iter().copied().filter(|s| *s > h.step).min().unwrap_or(16);
                h.dur = (next - h.step + 1).clamp(1, 16);
            }
        }
        // every place a hit is in the text: its token and the terms round it, then the part, the line, the calls
        let spans: Vec<Vec<Span>> = table
            .chains
            .into_iter()
            .map(|mut ch| {
                ch.push(part.span);
                ch.push(line);
                ch.extend(via.iter().copied());
                ch
            })
            .collect();
        Some(PartIr { role: role.to_string(), instrument: inst, sound, kit, route, vel_range: vel.filter(|(a, b)| a != b), spans, via: via.to_vec(), text: canonical, cycle_steps, cycles, chance: crate::chance::has_chance(&part.pattern), structural_bars: crate::meter::part_bars(part).unwrap_or(1), expr, may, twin: None, seed: crate::chance::has_chance(&part.pattern).then_some(seed) })
    }

    // ---- control

    pub(crate) fn control(&mut self, fl: &FlatLine) {
        let l = &fl.line;
        // a tempo is an event of the tempo map, not a call on the registry (F3, tempo.rs)
        if let Body::Action(Action::Tempo(spec)) = &l.body {
            self.tempo_line(fl, spec);
            return;
        }
        let target = l.target.map(|(t, _)| t);
        // a macro's length is its `over` (macros.rs): `for N bars` has nothing more to say
        if let (Body::Action(Action::Macro { head, .. }), Some((_, hsp))) = (&l.body, l.hold) {
            self.err(Diagnostic::error(hsp, "a macro's length is its `over`, so it takes no `for`").help(format!("{} ID on A over 4 bars", head.text(self.src))));
            return;
        }
        let effect = match &l.body {
            Body::Action(a) => self.action(fl.span, target, l.target.map(|(_, sp)| sp), a),
            Body::Do(d) => docall::compile_do(d, &self.env, self.src, &mut self.diags, self.seg_end),
            _ => return,
        };
        let Some(mut effect) = effect else { return };
        // `vocal create ... preset=alto` asks for a voice: a neural one that is not installed falls back (sing.rs)
        let notice = match &l.body {
            Body::Do(d) => self.vocal_voice_hook(d, &mut effect),
            _ => None,
        };
        // a value or a guard that names an input is read while the program plays (perform.rs)
        let Some(effect) = self.performed(fl, effect) else { return };
        let mut undo = Vec::new();
        let mut restore = None;
        // a move (filter_exit, echo_out ...) mutes its deck at the end: the program puts the strip back
        let moved = match &l.body {
            Body::Action(Action::Move { deck: Some(d), over, .. }) => Some((d.name(), over.and_then(|(o, _)| o.whole_bars()).unwrap_or(8))),
            Body::Do(d) if d.noun.text == "moves" && d.verb.as_ref().is_some_and(|v| v.text == "arm") => {
                let deck = d.params.iter().find(|p| p.name.text == "deck").and_then(|p| if let DoValue::Word(w) = &p.value { Some(w.text.clone()) } else { None });
                let bars = d.params.iter().find(|p| p.name.text == "bars").and_then(|p| if let DoValue::Num(n) = &p.value { Some(n.value as u32) } else { None });
                deck.map(|dk| (if dk.eq_ignore_ascii_case("b") { "B" } else { "A" }, bars.unwrap_or(8)))
            }
            _ => None,
        };
        if let Some((deck, len)) = moved {
            restore = Some(len);
            undo.push(Call::new("mixer", [("deck", deck.into()), ("mute", false.into())]));
        }
        // a macro (macros.rs): its call a beat ahead of a bar-line time (the engine starts it on the next bar line), a held one-shot
        // let go after its bars, and `every N bars from 0` two controls (its first fire has no beat before it; the rest are led)
        if let Body::Action(Action::Macro { version, id, over, .. }) = &l.body {
            let key = format!("{}{}", fl.path, print::line(l));
            let held = crate::macros::find(*version, &id.text).ok().and_then(|e| match e.length {
                crate::macros::Length::Held(n) => Some(over.and_then(|(o, _)| o.whole_bars()).unwrap_or(n)),
                _ => None,
            });
            // (the calls under a performed guard too: the release is the fire's, whatever decided it)
            let release: Vec<Call> = match (held, effect.base()) {
                (Some(_), Effect::Calls(cs)) => cs
                    .iter()
                    .map(|c| {
                        let mut p = c.params.clone();
                        if let JsonValue::Object(o) = &mut p {
                            o.set("action", JsonValue::str("release"));
                        }
                        Call { noun: c.noun.clone(), params: p }
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for (k, (timing, lead)) in crate::macros::calls(fl.timing).into_iter().enumerate() {
                let hold = held.and_then(|n| Rat::int(i64::from(n)).add(crate::macros::wait_to_bar(timing, lead)).ok()).and_then(Dur::from_bars);
                let key = if k == 0 { key.clone() } else { format!("{key} (from bar {})", crate::macros::later_from(fl.timing)) };
                self.controls.push(Control { key, timing, effect: effect.clone(), span: fl.span, hold, undo: release.clone(), restore: None, via: fl.via.clone(), notice: notice.clone(), window: None, lead });
            }
            return;
        }
        if let Some(n) = fl.hold {
            undo.extend(docall::derive_undo(&effect));
            if undo.is_empty() {
                let at = l.hold.map_or(fl.span, |(_, s)| s);
                self.err(
                    Diagnostic::error(at, format!("`for {n}` needs something that can be put back, and this has no off"))
                        .help("filter, delay, reverb, crush, mute, freeze on, trick on, move: yes. A level or an EQ has no off: ramp it back with a second line"),
                );
                return;
            }
        }
        self.controls.push(Control { key: format!("{}{}", fl.path, print::line(l)), timing: fl.timing, effect, span: fl.span, hold: fl.hold, undo, restore, via: fl.via.clone(), notice, window: None, lead: None });
    }

    pub(crate) fn need_target(&mut self, what: &str, target: Option<Target>, span: Span, want: &str) -> bool {
        let _ = target;
        self.err(Diagnostic::error(span, format!("{what} {want}")));
        false
    }

    pub(crate) fn action(&mut self, span: Span, target: Option<Target>, tspan: Option<Span>, a: &Action) -> Option<Effect> {
        let on_target = tspan.unwrap_or(span);
        match a {
            Action::SwapPairs { with } => {
                if target.is_some() {
                    self.need_target("`swap pairs`", target, on_target, "takes no target: write it with no `deck` or `master` before it");
                    return None;
                }
                let style = match with.as_ref().map(|w| w.text.as_str()) {
                    None => "auto",
                    Some("filter_exit") => "filter",
                    Some("echo_out") => "echo",
                    Some("loop_roll") => "roll",
                    Some(_) => {
                        let w = with.as_ref().unwrap();
                        self.err(
                            Diagnostic::error(w.span, format!("`{}` is not a move a pair swap can use", w.text))
                                .expected(&["filter_exit", "echo_out", "loop_roll"])
                                .found(format!("`{}`", w.text))
                                .help("for the other moves: `move bass_swap on B`"),
                        );
                        return None;
                    }
                };
                Some(Effect::Calls(vec![
                    Call::new("autodj", [("action", "set".into()), ("style", style.into())]),
                    Call::new("autodj", [("action", "next".into())]),
                ]))
            }
            Action::Move { kind, deck, over } => {
                if target.is_some() {
                    self.need_target("`move`", target, on_target, "takes no target: `move bass_swap on B`");
                    return None;
                }
                if !MOVES.contains(&kind.text.as_str()) {
                    let mut d = Diagnostic::error(kind.span, format!("`{}` is not a move", kind.text)).expected(&MOVES.map(|m| m)).found(format!("`{}`", kind.text));
                    if let Some(n) = parse::nearest(&kind.text, MOVES) {
                        d = d.help(format!("did you mean `{n}`?"));
                    }
                    self.err(d);
                    return None;
                }
                let Some(deck) = deck else {
                    self.err(Diagnostic::error(kind.span, format!("`move {}` needs the deck it acts on", kind.text)).help(format!("move {} on B", kind.text)));
                    return None;
                };
                let mut pairs = vec![("action", JsonValue::str("arm")), ("kind", JsonValue::str(kind.text.clone())), ("deck", JsonValue::str(deck.name()))];
                if let Some((o, osp)) = over {
                    match o.whole_bars() {
                        Some(n) => pairs.push(("bars", JsonValue::int(i64::from(n)))),
                        None => {
                            self.err(Diagnostic::error(*osp, "a move takes whole bars").found(o.canonical()).help("move bass_swap on B over 4 bars"));
                            return None;
                        }
                    }
                }
                Some(Effect::Calls(vec![Call { noun: "moves".into(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }]))
            }
            Action::LoadPair { spec, group } => {
                if target.is_some() {
                    self.need_target("`load pair`", target, on_target, "takes no target (it loads both decks)");
                    return None;
                }
                let mut pairs = vec![("action", JsonValue::str("load_pair")), ("spec", JsonValue::str(spec.text.clone()))];
                if let Some(g) = group {
                    pairs.push(("group", JsonValue::int(g.value as i64)));
                }
                Some(Effect::Calls(vec![Call { noun: "loopsets".into(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }]))
            }
            Action::Load { spec } => {
                let Some(Target::Deck(d)) = target else {
                    self.need_target("`load`", target, span, "needs a deck: `deck B: load techno:42`");
                    return None;
                };
                Some(Effect::Calls(vec![Call::new("deck", [("action", "load".into()), ("deck", d.name().into()), ("spec", spec.text.clone().into())])]))
            }
            Action::Queue { verb, arg, n } => {
                if target.is_some() {
                    self.need_target("`queue`", target, on_target, "takes no target");
                    return None;
                }
                const VERBS: [&str; 7] = ["play", "pause", "stop", "next", "loop", "jump", "add"];
                if !VERBS.contains(&verb.text.as_str()) {
                    self.err(Diagnostic::error(verb.span, format!("`{}` is not a queue verb", verb.text)).expected(&VERBS.map(|v| v)).found(format!("`{}`", verb.text)));
                    return None;
                }
                let mut pairs = vec![("action", JsonValue::str(verb.text.clone()))];
                if let Some(n) = n {
                    pairs.push(("n", JsonValue::int(n.value as i64)));
                }
                if let Some(a) = arg {
                    if verb.text == "loop" {
                        match a.text.as_str() {
                            "on" => pairs.push(("on", true.into())),
                            "off" => pairs.push(("on", false.into())),
                            _ => {
                                self.err(Diagnostic::error(a.span, "loop takes `on` or `off`").expected(&["on", "off"]).found(format!("`{}`", a.text)));
                                return None;
                            }
                        }
                    } else {
                        pairs.push(("id", JsonValue::str(a.text.clone())));
                    }
                }
                Some(Effect::Calls(vec![Call { noun: "queue".into(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }]))
            }
            Action::Transport(w) => {
                let Some(Target::Deck(d)) = target else {
                    self.need_target(&format!("`{}`", w.text), target, w.span, &format!("needs a deck: `deck A: {}`", w.text));
                    return None;
                };
                let call = match w.text.as_str() {
                    "mute" | "unmute" => Call::new("mixer", [("deck", d.name().into()), ("mute", (w.text == "mute").into())]),
                    other => Call::new("deck", [("action", other.into()), ("deck", d.name().into())]),
                };
                Some(Effect::Calls(vec![call]))
            }
            Action::Change { name, mode, from, to, over } => self.change(span, target, tspan, name, mode.as_ref(), from.as_ref(), to.as_ref(), over.as_ref()),
            Action::Shaped { name, mode, motion } => self.shaped(span, target, tspan, name, mode.as_ref(), motion),
            // (a tempo line is read in `control`, before it gets here)
            Action::Tempo(_) => None,
            Action::Macro { version, head, id, deck, over } => {
                if target.is_some() {
                    self.need_target("`macro`", target, on_target, &format!("takes no target: it names its deck itself, `{} {} on A`", head.text(self.src), id.text));
                    return None;
                }
                self.macro_call(*version, *head, id, *deck, over.as_ref())
            }
        }
    }

    /// `macro ID on DECK over N bars` (macros.rs): checked against its version's table, lowered to the registry's `effects fire`.
    fn macro_call(&mut self, version: u32, head: Span, id: &Word, deck: Option<Deck>, over: Option<&(Dur, Span)>) -> Option<Effect> {
        use crate::macros::{self, Length, Miss};
        let written = head.text(self.src).to_string();
        let e = match macros::find(version, &id.text) {
            Ok(e) => e,
            Err(Miss::NoVersion) => {
                let have = macros::version_names();
                self.err(
                    Diagnostic::error(head, format!("this BeatScript has {}, not `{written}`", have.join(", ")))
                        .help(format!("macro {} ...   (a bare `macro` is macro@1, and a version always means the same catalogue)", id.text)),
                );
                return None;
            }
            Err(Miss::NotIn { near, later }) => {
                let mut d = Diagnostic::error(id.span, format!("`{}` is not in the macro catalogue of {}", id.text, if version == macros::BARE { "macro@1 (a bare `macro`)".to_string() } else { format!("macro@{version}") }))
                    .found(format!("`{}`", id.text));
                d = match (later, near) {
                    (Some(v), _) => d.help(format!("`{}` is in macro@{v}: `{} {} ...`", id.text, macros::head_text(v), id.text)),
                    (None, Some(n)) => d.help(format!("did you mean `{n}`?")),
                    (None, None) => d.help("the `macros` list has every id: riser, tape_stop, breakdown, cathedral, ping-pong, feel-build ..."),
                };
                self.err(d);
                return None;
            }
        };
        let Some(deck) = deck else {
            self.err(Diagnostic::error(id.span, format!("`{written} {}` needs the deck it fires on", id.text)).help(format!("{written} {} on A", id.text)));
            return None;
        };
        let bars = match over {
            Some((_, osp)) if e.length == Length::Own => {
                self.err(Diagnostic::error(*osp, format!("`{}` is a one-shot: it runs its own length, so it takes no `over`", e.id)).help(format!("{written} {} on {}", e.id, deck.name())));
                return None;
            }
            Some((o, osp)) => {
                let n = o.whole_bars();
                match n {
                    Some(n) if (1..=macros::MAX_BARS).contains(&n) => Some(n),
                    // (a whole number of bars, out of range: none, or more than a count holds, is not a fraction)
                    _ if n.is_some() || o.as_bars().is_int() => {
                        let near = if n == Some(0) { "1 bar".to_string() } else { format!("{} bars", macros::MAX_BARS) };
                        self.err(
                            Diagnostic::error(*osp, format!("a macro runs 1 to {} bars, not {}", macros::MAX_BARS, o.canonical()))
                                .found(o.canonical())
                                .help(format!("{written} {} on {} over {near}", e.id, deck.name())),
                        );
                        return None;
                    }
                    _ => {
                        self.err(Diagnostic::error(*osp, "a macro takes whole bars").found(o.canonical()).help(format!("{written} {} on {} over 4 bars", e.id, deck.name())));
                        return None;
                    }
                }
            }
            None => e.length.default_bars(),
        };
        let mut pairs = vec![("action", JsonValue::str("fire")), ("target", JsonValue::str(deck.name())), ("id", JsonValue::str(e.id))];
        // a held one-shot is fired with no length and let go after it (`control`); an own-length one has none
        if let (Length::Bars(_) | Length::Sweep(_), Some(n)) = (e.length, bars) {
            pairs.push(("bars", JsonValue::int(i64::from(n))));
        }
        Some(Effect::Calls(vec![Call { noun: "effects".into(), params: JsonValue::Object(JsonObject::from_pairs(pairs)) }]))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn change(&mut self, span: Span, target: Option<Target>, tspan: Option<Span>, name: &Word, mode: Option<&Word>, from: Option<&Num>, to: Option<&Num>, over: Option<&(Dur, Span)>) -> Option<Effect> {
        let n = name.text.as_str();
        let on_target = tspan.unwrap_or(name.span);
        let ins_target = |t: Target| match t {
            Target::Deck(d) => d.name().to_string(),
            Target::Master => "Master".to_string(),
        };
        // ramps: `a -> b over N bars`
        match (to, over) {
            (Some(t), None) => {
                self.err(Diagnostic::error(t.span, "a ramp needs a length: `... -> b over 8 bars`"));
                return None;
            }
            (None, Some((_, osp))) => {
                self.err(Diagnostic::error(*osp, "`over` has nothing to move: write `a -> b over N bars`"));
                return None;
            }
            _ => {}
        }
        let unit = |c: &mut Self, v: &Num, what: &str| -> Option<f64> {
            // (a performed value is held to its range when it is read: perform.rs checks what it can be)
            if (0.0..=1.0).contains(&v.value) || crate::perform::is_live(v) {
                Some(v.value)
            } else {
                c.err(Diagnostic::error(v.span, format!("{what} is 0 to 1 (or 0% to 100%)")).found(v.span.text(c.src).to_string()));
                None
            }
        };
        let bars = over.map(|(d, _)| *d);
        // (call params base, value field, the values in the call's own unit, calls before)
        let plan: Option<(String, JsonValue, &str, f64, Option<f64>, Vec<Call>)> = match n {
            "filter" => {
                let Some(t) = target else {
                    self.err(Diagnostic::error(name.span, "`filter` needs a target").help("master: filter lp 2k   or   deck A: filter hp 400"));
                    return None;
                };
                let tg = ins_target(t);
                let on = |on: bool| Call::new("inserts", [("action", "on".into()), ("target", tg.clone().into()), ("effect", "filter".into()), ("on", on.into())]);
                let m = mode?;
                match m.text.as_str() {
                    "off" => return Some(Effect::Calls(vec![on(false)])),
                    "open" => {
                        return Some(Effect::Calls(vec![
                            on(true),
                            Call::new("inserts", [("action", "set".into()), ("target", tg.clone().into()), ("param", "filter.sweep".into()), ("value", JsonValue::Number(0.5))]),
                        ]))
                    }
                    "lp" | "hp" => {
                        let hp = m.text == "hp";
                        let hz = |c: &mut Self, v: &Num| -> Option<f64> {
                            let (lo, hi) = if hp { (20.0, 8000.0) } else { (60.0, 20000.0) };
                            if (v.value < lo || v.value > hi) && !crate::perform::is_live(v) {
                                c.err(
                                    Diagnostic::error(v.span, format!("a {} cutoff is {lo} Hz to {hi} Hz", if hp { "high-pass" } else { "low-pass" }))
                                        .found(v.span.text(c.src).to_string())
                                        .help("hertz: 400, 2k, 12k"),
                                );
                                return None;
                            }
                            Some(sweep_knob(v.value, hp))
                        };
                        let a = hz(self, from?)?;
                        let b = match to {
                            Some(t) => Some(hz(self, t)?),
                            None => None,
                        };
                        let base = JsonValue::Object(JsonObject::from_pairs([("action", "set".into()), ("target", tg.clone().into()), ("param", "filter.sweep".into())]));
                        Some(("inserts".to_string(), base, "value", a, b, vec![on(true)]))
                    }
                    other => {
                        self.err(Diagnostic::error(m.span, format!("`filter {other}`: not a filter mode")).expected(&["lp", "hp", "off", "open"]).found(format!("`{other}`")));
                        return None;
                    }
                }
            }
            "delay" | "reverb" | "crush" => {
                let Some(t) = target else {
                    self.err(Diagnostic::error(name.span, format!("`{n}` needs a target")).help(format!("master: {n} 0.3")));
                    return None;
                };
                let tg = ins_target(t);
                let a = unit(self, from?, &format!("{n}"))?;
                let b = match to {
                    Some(t) => Some(unit(self, t, n)?),
                    None => None,
                };
                let on = Call::new("inserts", [("action", "on".into()), ("target", tg.clone().into()), ("effect", n.into()), ("on", true.into())]);
                let base = JsonValue::Object(JsonObject::from_pairs([("action", "set".into()), ("target", tg.into()), ("param", format!("{n}.mix").into())]));
                Some(("inserts".to_string(), base, "value", a, b, vec![on]))
            }
            "resonance" => {
                // the filter's resonance: it needs the filter on, and sits at the knob's own 0..1
                let Some(t) = target else {
                    self.err(Diagnostic::error(name.span, "`resonance` needs a target").help("master: resonance 0.7   or   deck A: resonance 0.4"));
                    return None;
                };
                let tg = ins_target(t);
                let a = unit(self, from?, "resonance")?;
                let b = match to {
                    Some(t) => Some(unit(self, t, "resonance")?),
                    None => None,
                };
                let on = Call::new("inserts", [("action", "on".into()), ("target", tg.clone().into()), ("effect", "filter".into()), ("on", true.into())]);
                let base = JsonValue::Object(JsonObject::from_pairs([("action", "set".into()), ("target", tg.into()), ("param", "filter.res".into())]));
                Some(("inserts".to_string(), base, "value", a, b, vec![on]))
            }
            "drive" => {
                // the FX unit's drive (djrust `fx::unit`, the master chain's distortion) on a deck's chain; `master:` is both decks',
                // as the `fx` noun says when it is given no deck. 0 is off: the unit at neutral changes no sample.
                let Some(t) = target else {
                    self.err(Diagnostic::error(name.span, "`drive` needs a target").help("master: drive 0.6   or   deck A: drive 0.4"));
                    return None;
                };
                let held = |c: &mut Self, v: &Num| -> Option<f64> {
                    if (0.0..=1.0).contains(&v.value) || crate::perform::is_live(v) {
                        return Some(v.value);
                    }
                    let near = if v.value < 0.0 { "0, which is off" } else { "1, the most" };
                    c.err(Diagnostic::error(v.span, "drive is 0 to 1 (or 0% to 100%)").found(v.span.text(c.src).to_string()).help(format!("the nearest is {near}")));
                    None
                };
                let a = held(self, from?)?;
                let b = match to {
                    Some(t) => Some(held(self, t)?),
                    None => None,
                };
                let mut base = JsonObject::from_pairs([("action", "set".into())]);
                if let Target::Deck(d) = t {
                    base.set("deck", d.name().into());
                }
                base.set("param", "drive".into());
                Some(("fx".to_string(), JsonValue::Object(base), "value", a, b, vec![]))
            }
            "level" => {
                let a = unit(self, from?, "level")?;
                let b = match to {
                    Some(t) => Some(unit(self, t, "level")?),
                    None => None,
                };
                match target {
                    Some(Target::Master) => Some(("mixer".to_string(), JsonValue::Object(JsonObject::new()), "master", a, b, vec![])),
                    Some(Target::Deck(d)) => Some(("mixer".to_string(), JsonValue::Object(JsonObject::from_pairs([("deck", d.name().into())])), "volume", a, b, vec![])),
                    None => {
                        self.err(Diagnostic::error(name.span, "`level` needs a target").help("master: level 0.8   or   deck A: level 0.7"));
                        return None;
                    }
                }
            }
            "low" | "mid" | "high" => {
                let Some(Target::Deck(d)) = target else {
                    self.err(Diagnostic::error(if tspan.is_some() { on_target } else { name.span }, format!("`{n}` is a deck strip's EQ: `deck A: {n} 0.5`")));
                    return None;
                };
                let a = unit(self, from?, n)?;
                let b = match to {
                    Some(t) => Some(unit(self, t, n)?),
                    None => None,
                };
                Some(("mixer".to_string(), JsonValue::Object(JsonObject::from_pairs([("deck", d.name().into())])), n, a, b, vec![]))
            }
            "crossfader" => {
                if target.is_some() {
                    self.err(Diagnostic::error(on_target, "`crossfader` takes no target"));
                    return None;
                }
                let a = unit(self, from?, "crossfader")?;
                let b = match to {
                    Some(t) => Some(unit(self, t, "crossfader")?),
                    None => None,
                };
                Some(("mixer".to_string(), JsonValue::Object(JsonObject::new()), "crossfader", a, b, vec![]))
            }
            _ => {
                self.err(Diagnostic::error(span, format!("`{n}` is not a parameter")));
                return None;
            }
        };
        let (noun, base, field, a, b, prelude) = plan?;
        match (b, bars) {
            (Some(b), Some(bars)) if !bars.is_zero() => Some(Effect::Ramp(Ramp { prelude, noun, base, field: field.to_string(), from: a, to: b, bars, shape: crate::shape::Shape::Linear })),
            (Some(_), Some(_)) => {
                self.err(Diagnostic::error(over.map_or(span, |o| o.1), "a ramp takes some time").help("over 1 bars, over 2 beats"));
                None
            }
            _ => {
                let mut calls = prelude;
                let mut o = base.as_object().cloned().unwrap_or_default();
                o.set(field, JsonValue::Number(a));
                calls.push(Call { noun, params: JsonValue::Object(o) });
                Some(Effect::Calls(calls))
            }
        }
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
    a / gcd(a, b) * b
}

/// The Master / deck filter's knob (0..1) for a cutoff: the inverse of `fx::insert::sweep_corners`
/// (below 0.5 low-pass 20 kHz .. 60 Hz, above 0.5 high-pass 20 Hz .. 8 kHz, a dead zone at 0.5).
pub fn sweep_knob(hz: f64, hp: bool) -> f64 {
    const DEAD: f64 = 0.02;
    let v = if hp {
        let t = (hz / 20.0).ln() / (8000.0_f64 / 20.0).ln();
        0.5 + DEAD + t.clamp(0.0, 1.0) * (0.5 - DEAD)
    } else {
        let t = (hz / 20_000.0).ln() / (60.0_f64 / 20_000.0).ln();
        0.5 - DEAD - t.clamp(0.0, 1.0) * (0.5 - DEAD)
    };
    (v * 1e6).round() / 1e6
}

/// The filter's knob (0..1) for a place `v` (0..1) along the cutoff's own range, on its log curve: 0 is the lowest cutoff the line may
/// say (60 Hz low-pass, 20 Hz high-pass), 1 the highest (20 kHz, 8 kHz), so up is always a higher cutoff. What `filter lp knob(1)` is:
/// an input written alone covers the range.
pub fn sweep_span(v: f64, hp: bool) -> f64 {
    let (lo, hi): (f64, f64) = if hp { (20.0, 8000.0) } else { (60.0, 20_000.0) };
    sweep_knob(lo * (hi / lo).powf(v.clamp(0.0, 1.0)), hp)
}

#[allow(dead_code)]
const _ROLES: [&str; 8] = ROLES;

/// Voices worth trying in place of a family word's generic voice, by what the name is after.
const FAMILY_HINTS: [(&str, &[&str]); 8] = [
    ("reese", &["wobble-lead", "detuned-saw", "big-saw", "hoover"]),
    ("acid", &["acid-bass", "acid-square", "pwm-lead", "sync-lead"]),
    ("bass", &["bass", "808-bass", "wobble-lead", "detuned-saw"]),
    ("sub", &["808-bass", "bass", "wobble-lead"]),
    ("pad", &["warm-pad", "dark-pad", "am-pad", "glass-pad"]),
    ("pluck", &["pluck", "bright-pluck", "edm-pluck", "muted-pluck"]),
    ("piano", &["piano", "electric-piano", "clavinet"]),
    ("organ", &["organ", "rave-organ"]),
];

fn pick(name: &str, voices: &[String]) -> String {
    let mut list = crate::env::suggest(name, voices, 3);
    if list.is_empty() {
        for (fam, hints) in FAMILY_HINTS {
            if name.contains(fam) {
                list = hints.iter().filter(|h| voices.iter().any(|v| v == *h)).map(|h| h.to_string()).collect();
                break;
            }
        }
    }
    if list.is_empty() {
        list = voices.iter().take(4).cloned().collect();
    }
    list.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}
