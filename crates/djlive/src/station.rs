//! Radio: stations (docs/btw/radio.md). A station is a program that names a run of tracks with no end: its first track's
//! generator call, a step from one track to the next and the forms the tracks take in turn. It is DETERMINISTIC: position N
//! of a station is always the same track (the same text, so the same CID), worked out from the station's text alone, with
//! no clock and no store. A listener does not steer a station; they turn its knobs (genre, seed, step, forms, tempo, key,
//! length), and turning one moves them to a neighbouring station that already exists, at the same position N.
//!
//! ```text
//! tempo 128
//! key am
//! station {
//!   from gen@2 techno seed=42
//!   next seed=~
//!   forms seeded passacaglia rondo fugue
//! }
//! ```
//!
//! - The walk: s_0 is the from call's seed. `next seed=+n` counts on (s_N = s_0 + nN); `next seed=~` draws s_N+1 from a hash
//!   of the CID of the program `arrange from gen@V GENRE seed=s_N`: the walk's track is the generator call, not its tempo,
//!   key or form, so turning those knobs keeps the walk.
//! - The track at N plays form `forms[N mod R]` (R forms). A pass of the forms is a chapter, and the seed of its first track
//!   is the chapter's theme: a form's first role is bound to it (it recurs through the chapter), its second role to s_N.
//! - The segue from N to N+1 is the `cross` form of their two seeds, a program with its own CID.
//!
//! Pure: the walk, the tracks and the segues are text and CIDs, made with no generator. [`check`] (a station program's
//! check) compiles the tracks of the first chapter and the first segue with the installed generator, and a station
//! compiles to its track 0.

use djengine::json::{JsonObject, JsonValue};

use crate::ast::*;
use crate::compile::{program_cid, Compiled};
use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::gen::{GenCall, Host};
use crate::lex::lex;
use crate::parse::Parsed;

/// The rules of stations: `#! stations: v1` (also what a program with no such line means). A change that moved any track
/// of any station would be stations v2.
pub const VERSION: u32 = 1;
/// What `forms` takes: `seeded` (the seeded generator's own arrangement) and the forms whose roles a seed fills.
pub const FORMS: [&str; 7] = ["seeded", "passacaglia", "canon", "rondo", "fugue", "theme-and-variations", "cross"];
/// The genres whose seeds play no harmony and no arp on the bar's grid: a cross binds their melodies, a rondo's episodes
/// are their melody.
pub const SPARSE: [&str; 4] = ["ambient", "bossa", "jazz", "lofi"];
/// The knobs of a station, in the order the answer gives them.
pub const KNOBS: [&str; 7] = ["genre", "seed", "step", "forms", "tempo", "key", "length"];
/// How far a `seed=~` walk is worked out (each step is a CID).
pub const MAX_WALK: u64 = 100_000;
/// At most this many forms in a rotation.
pub const MAX_FORMS: usize = 16;
/// The public-domain phrasebook's lines a form sings, pinned for stations v1 (a new line in the book moves no track).
pub const PHRASES: [&str; 46] = [
    "tempest_such_stuff", "sonnet18_summers_day", "macbeth_tomorrow", "paradise_lost_better_to_reign", "paradise_lost_justify_the_ways", "tyger_burning_bright", "tyger_anvil_grasp", "hiawatha_should_you_ask",
    "hiawatha_gitche_gumee", "hiawatha_dark_behind", "byron_assyrian_wolf", "byron_sheen_of_their_spears", "byron_leaves_of_the_forest", "lear_derry_down_derry", "evangeline_forest_primeval", "evangeline_ye_who_believe",
    "evangeline_silently_one_by_one", "evangeline_talk_not_of_wasted", "dickinson_chariot_death", "dickinson_frigate_book", "newton_amazing_grace", "watts_god_our_help", "percy_patrick_spens_king", "hopkins_pied_beauty_glory",
    "hopkins_pied_beauty_swift_slow", "hopkins_pied_beauty_landscape", "hopkins_windhover_brute_beauty", "hopkins_spring_and_fall_margaret", "mgm_high_diddle_diddle", "mgm_hush_a_by_baby", "mgm_little_jack_horner", "mgm_pease_porridge_hot",
    "psalm42_deep_calleth", "psalm42_waves_billows", "eccl3_season", "psalm19_day_unto_day", "psalm121_lift_up_mine_eyes", "gettysburg_four_score", "gettysburg_highly_resolve", "gettysburg_new_birth",
    "gettysburg_of_the_people", "shanty_blow_the_man_down", "shanty_drunken_sailor", "shanty_leave_her_johnny", "shanty_sally_brown", "shanty_haul_away_joe",
];
/// The circle of fifths, as Camelot numbers count it: a key one notch round is a fifth up or down in its own mode.
const FIFTHS: [&str; 12] = ["c", "g", "d", "a", "e", "b", "f#", "c#", "g#", "d#", "a#", "f"];

/// A station, read from its program.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    /// The generator's version: `gen@2`.
    pub version: u32,
    pub genre: String,
    /// The first track's seed as written (a whole number, or a word).
    pub seed: String,
    /// `length=` of the calls (None is standard).
    pub length: Option<String>,
    pub step: SeedStep,
    pub forms: Vec<String>,
    /// The program's `tempo` and `key`: every track and segue is generated at them. None: the from call's own.
    pub tempo: Option<f64>,
    pub key: Option<String>,
}

/// The track at a position of a station.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub n: u64,
    /// The walk's seed at n (the fresh role's), and the chapter's theme seed (the recurring role's).
    pub seed: u32,
    pub theme: u32,
    pub form: String,
    /// The phrasebook line the form sings (`@id`); none for `seeded`.
    pub phrase: Option<String>,
    /// The program: its provenance header lines, its tempo and key, its `arrange from` line.
    pub text: String,
    /// The CID of its canonical text (references and all, never the expansion).
    pub cid: String,
    /// Track n-1's CID (none for track 0): what it was composed after (`#! from: next(<cid>)`).
    pub parent: Option<String>,
}

/// The segue from track n to n+1: the `cross` form of their two seeds.
#[derive(Clone, Debug, PartialEq)]
pub struct Segue {
    pub from: u64,
    pub to: u64,
    pub phrase: String,
    pub text: String,
    pub cid: String,
}

/// What a station program's check reports.
#[derive(Clone, Debug, PartialEq)]
pub struct StationReport {
    pub station: Station,
    /// The CID of `arrange from <the from call>`: where the walk starts.
    pub start_cid: String,
    /// The tracks of the first chapter (one of each form), checked.
    pub tracks: Vec<Track>,
    /// The segue from track 0 to track 1, checked.
    pub segue: Option<Segue>,
}

impl StationReport {
    pub fn to_json(&self) -> JsonValue {
        let mut o = self.station.to_json();
        if let JsonValue::Object(m) = &mut o {
            m.set("start_cid", JsonValue::str(self.start_cid.clone()));
            m.set("tracks", JsonValue::Array(self.tracks.iter().map(Track::to_json).collect()));
            m.set("segue", self.segue.as_ref().map_or(JsonValue::Null, Segue::to_json));
        }
        o
    }
}

// ---------------------------------------------------------------- hashing

/// FNV-1a 32 over the bytes: a CID to a seed (the case of base58 kept).
pub fn fold32(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// A seed as the generator reads it: a whole number is itself (to 32 bits), a word the FNV-1a 32 of its lowercased text.
pub fn seed_value(seed: &str) -> u32 {
    let s = seed.trim();
    if let Ok(n) = s.parse::<u64>() {
        return (n & 0xffff_ffff) as u32;
    }
    fold32(&s.to_lowercase())
}

/// The CID of the program `arrange from CALL`: a walk's track, what `seed=~` draws the next seed from.
pub fn call_cid(call: &str) -> String {
    program_cid(&crate::print::canonical(&crate::parse::parse(&format!("arrange from {call}\n")).program))
}

/// The call `call` (a generator call word) with its seed stepped: `+n` counts on, `~` draws from the CID of
/// `arrange from CALL`. A set's `next` and a station's walk step the same way.
pub fn stepped_call(call: &str, step: SeedStep) -> Result<String, String> {
    let toks = lex(call).tokens;
    let Some(run) = crate::gen::runs(&toks).into_iter().next() else { return Err(format!("`{call}` is no generator call")) };
    let mut c = run.r.map_err(|d| d.message)?;
    let next = match step {
        SeedStep::By(n) => seed_value(&c.seed).wrapping_add(n),
        SeedStep::Hash => fold32(&call_cid(&c.text())),
    };
    c.seed = next.to_string();
    Ok(c.text())
}

// ---------------------------------------------------------------- the station

impl Station {
    /// The from call's spelling with `seed`: `gen@2 techno seed=42 length=long`.
    pub fn call(&self, seed: &str) -> String {
        let mut s = format!("gen@{} {} seed={seed}", self.version, self.genre);
        if let Some(l) = self.length.as_deref().filter(|l| *l != "standard") {
            s.push_str(&format!(" length={l}"));
        }
        s
    }

    /// The from call as written.
    pub fn from_call(&self) -> String {
        self.call(&self.seed)
    }

    /// The program's text, canonical: `tempo`, `key`, the block with the lines that are not the defaults.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if let Some(t) = self.tempo {
            s.push_str(&format!("tempo {}\n", num(t)));
        }
        if let Some(k) = &self.key {
            s.push_str(&format!("key {k}\n"));
        }
        s.push_str(&format!("station {{\n  from {}\n", self.from_call()));
        if self.step != SeedStep::By(1) {
            s.push_str(&format!("  next {}\n", crate::print::step_word(self.step)));
        }
        if self.forms != ["seeded"] {
            s.push_str(&format!("  forms {}\n", self.forms.join(" ")));
        }
        s.push_str("}\n");
        crate::print::canonical(&crate::parse::parse(&s).program)
    }

    /// The station's CID: of its canonical text.
    pub fn cid(&self) -> String {
        program_cid(&self.text())
    }

    /// The CID the walk starts from: of `arrange from <the from call>`.
    pub fn start_cid(&self) -> String {
        call_cid(&self.from_call())
    }

    /// The walk's seeds s_0 ..= s_upto.
    pub fn seeds(&self, upto: u64) -> Result<Vec<u32>, String> {
        let s0 = seed_value(&self.seed);
        match self.step {
            SeedStep::By(n) => Ok((0..=upto).map(|i| s0.wrapping_add(((u64::from(n) * i) & 0xffff_ffff) as u32)).collect()),
            SeedStep::Hash => {
                if upto > MAX_WALK {
                    return Err(format!("a `seed=~` station is worked out to position {MAX_WALK}: {upto} is past it"));
                }
                let mut out = Vec::with_capacity(upto as usize + 1);
                out.push(s0);
                let mut call = self.from_call();
                for _ in 0..upto {
                    let next = fold32(&call_cid(&call));
                    out.push(next);
                    call = self.call(&next.to_string());
                }
                Ok(out)
            }
        }
    }

    /// The seed written at walk position i: track 0 keeps the from call's own spelling (a word stays a word).
    fn seed_word(&self, i: u64, s: u32) -> String {
        if i == 0 {
            self.seed.clone()
        } else {
            s.to_string()
        }
    }

    fn sparse(&self) -> bool {
        SPARSE.contains(&self.genre.as_str())
    }

    /// The `arrange from` reference of the track at n, given the walk (s_0 ..= s_n at least).
    fn reference(&self, n: u64, seeds: &[u32]) -> (String, String, Option<String>, u32, u32) {
        let r = self.forms.len().max(1) as u64;
        let form = self.forms[(n % r) as usize].clone();
        let t_at = n - n % r;
        let (s, t) = (seeds[n as usize], seeds[t_at as usize]);
        let fresh = self.call(&self.seed_word(n, s));
        let theme = self.call(&self.seed_word(t_at, t));
        let phrase = PHRASES[(s % PHRASES.len() as u32) as usize];
        let melody = if self.sparse() { " part=melody" } else { "" };
        let roles = match form.as_str() {
            "seeded" => return (fresh, form, None, s, t),
            "passacaglia" => format!("ground={theme}, upper={fresh}"),
            "canon" => format!("subject={fresh}, ground={theme}"),
            "rondo" => format!("refrain={theme}, episodes={fresh} part={}", if self.sparse() { "melody" } else { "arp" }),
            "fugue" => format!("subject={theme}, counter={fresh}"),
            "theme-and-variations" => format!("theme={theme}"),
            _ => format!("a={theme}{melody}, b={fresh}{melody}"),
        };
        (format!("template({form}, @{phrase}, {roles})"), form, Some(format!("@{phrase}")), s, t)
    }

    /// The program of a reference: the station's tempo and key, then `arrange from` it; canonical.
    fn body(&self, reference: &str) -> String {
        let mut s = String::new();
        if let Some(t) = self.tempo {
            s.push_str(&format!("tempo {}\n", num(t)));
        }
        if let Some(k) = &self.key {
            s.push_str(&format!("key {k}\n"));
        }
        s.push_str(&format!("arrange from {reference}\n"));
        crate::print::canonical(&crate::parse::parse(&s).program)
    }

    /// The tracks at n .. n+count.
    pub fn tracks(&self, n: u64, count: u64) -> Result<Vec<Track>, String> {
        let last = n.checked_add(count.saturating_sub(1)).ok_or("past the end of the walk")?;
        let seeds = self.seeds(last)?;
        let station = self.cid();
        let mut parent = (n > 0).then(|| {
            let (rf, ..) = self.reference(n - 1, &seeds);
            program_cid(&self.body(&rf))
        });
        let mut out = Vec::new();
        for i in n..=last {
            let (rf, form, phrase, seed, theme) = self.reference(i, &seeds);
            let body = self.body(&rf);
            let cid = program_cid(&body);
            let mut text = format!("#! station: {station} track {i}\n");
            if let Some(p) = &parent {
                text.push_str(&format!("#! from: next({p}) {}\n", crate::print::step_word(self.step)));
            }
            text.push_str(&body);
            out.push(Track { n: i, seed, theme, form, phrase, text, cid: cid.clone(), parent: parent.clone() });
            parent = Some(cid);
        }
        Ok(out)
    }

    /// The track at n.
    pub fn track(&self, n: u64) -> Result<Track, String> {
        self.tracks(n, 1).map(|mut v| v.remove(0))
    }

    /// The segue from track n to n+1: the cross of their seeds.
    pub fn segue(&self, n: u64) -> Result<Segue, String> {
        let seeds = self.seeds(n + 1)?;
        let (a, b) = (seeds[n as usize], seeds[n as usize + 1]);
        let melody = if self.sparse() { " part=melody" } else { "" };
        let phrase = PHRASES[((u64::from(a) + u64::from(b)) % PHRASES.len() as u64) as usize];
        let rf = format!("template(cross, @{phrase}, a={}{melody}, b={}{melody})", self.call(&self.seed_word(n, a)), self.call(&b.to_string()));
        let body = self.body(&rf);
        let cid = program_cid(&body);
        let text = format!("#! station: {} segue {n} to {}\n{body}", self.cid(), n + 1);
        Ok(Segue { from: n, to: n + 1, phrase: format!("@{phrase}"), text, cid })
    }

    /// The station with one knob set: `value` is the knob's new value, or `+1` / `-1` for one notch up or down.
    pub fn retune(&self, knob: &str, value: &str) -> Result<Station, String> {
        let mut s = self.clone();
        let notch = match value.trim() {
            "+1" | "+" | "up" => Some(1i64),
            "-1" | "-" | "down" => Some(-1i64),
            _ => None,
        };
        match knob {
            "genre" => {
                if notch.is_some() {
                    return Err("a genre has no notch yet (its neighbourhoods are phase 2): name the genre".into());
                }
                s.genre = value.trim().to_lowercase();
            }
            "seed" => match notch {
                Some(d) => s.seed = (i64::from(seed_value(&self.seed)) + d).rem_euclid(1 << 32).to_string(),
                None => {
                    let v = value.trim();
                    if v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                        return Err(format!("`{v}` is no seed: a whole number or a word"));
                    }
                    s.seed = if let Ok(n) = v.parse::<u64>() { (n & 0xffff_ffff).to_string() } else { v.to_lowercase() };
                }
            },
            "step" => {
                s.step = match (notch, value.trim()) {
                    (Some(_), _) => match self.step {
                        SeedStep::Hash => SeedStep::By(1),
                        SeedStep::By(_) => SeedStep::Hash,
                    },
                    (None, "~" | "seed=~") => SeedStep::Hash,
                    (None, v) => {
                        let n = v.trim_start_matches("seed=").trim_start_matches('+');
                        match n.parse::<u32>() {
                            Ok(n) if n >= 1 => SeedStep::By(n),
                            _ => return Err(format!("`{v}` is no step: +1, +2 ... or ~")),
                        }
                    }
                }
            }
            "forms" => match notch {
                Some(d) => {
                    let len = s.forms.len();
                    if len > 1 {
                        s.forms.rotate_left(if d > 0 { 1 } else { len - 1 });
                    } else {
                        let at = FORMS.iter().position(|f| *f == s.forms[0]).unwrap_or(0) as i64;
                        s.forms = vec![FORMS[(at + d).rem_euclid(FORMS.len() as i64) as usize].to_string()];
                    }
                }
                None => {
                    let names: Vec<String> = value.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_string).collect();
                    if let Some(bad) = names.iter().find(|n| !FORMS.contains(&n.as_str())) {
                        return Err(format!("`{bad}` is no form a station takes: {}", FORMS.join(" ")));
                    }
                    if names.is_empty() || names.len() > MAX_FORMS {
                        return Err(format!("a station takes 1 to {MAX_FORMS} forms"));
                    }
                    s.forms = names;
                }
            },
            "tempo" => {
                let t = match notch {
                    Some(d) => self.tempo.ok_or("the station's tempo is its from call's own: say one, then turn it")? + 2.0 * d as f64,
                    None => value.trim().parse::<f64>().map_err(|_| format!("`{value}` is no tempo"))?,
                };
                if !(40.0..=300.0).contains(&t) {
                    return Err(format!("a tempo is 40 to 300, not {}", num(t)));
                }
                s.tempo = Some(t);
            }
            "key" => {
                s.key = Some(match notch {
                    Some(d) => fifth(self.key.as_deref().ok_or("the station's key is its from call's own: say one, then turn it")?, d)?,
                    None => {
                        let k = value.trim().to_lowercase();
                        if crate::music::Key::parse(&k).is_none() {
                            return Err(format!("`{value}` is no key"));
                        }
                        crate::music::canonical_pitch_word(&k)
                    }
                });
            }
            "length" => {
                s.length = match notch {
                    Some(d) => {
                        let at = crate::gen::LENGTHS.iter().position(|l| Some(*l) == self.length.as_deref()).unwrap_or(1) as i64;
                        Some(crate::gen::LENGTHS[(at + d).clamp(0, 2) as usize].to_string())
                    }
                    None => {
                        let l = value.trim();
                        if !crate::gen::LENGTHS.contains(&l) {
                            return Err(format!("`{l}` is no length: short, standard or long"));
                        }
                        Some(l.to_string())
                    }
                };
                if s.length.as_deref() == Some("standard") {
                    s.length = None;
                }
            }
            _ => return Err(format!("`{knob}` is no knob of a station: {}", KNOBS.join(" "))),
        }
        Ok(s)
    }

    /// The stations one notch away along each knob not in `locks`, (knob, the notch, the station): a lock is a knob held
    /// fixed. A genre has no notch yet. A knob at its end (a tempo at 300) gives one neighbour.
    pub fn neighbours(&self, locks: &[String]) -> Vec<(String, String, Station)> {
        let mut out = Vec::new();
        for knob in KNOBS {
            if knob == "genre" || locks.iter().any(|l| l == knob) {
                continue;
            }
            let notches: &[&str] = if knob == "step" { &["+1"] } else { &["-1", "+1"] };
            for v in notches {
                if let Ok(s) = self.retune(knob, v) {
                    if s != *self && !out.iter().any(|(_, _, x)| *x == s) {
                        out.push((knob.to_string(), v.to_string(), s));
                    }
                }
            }
        }
        out
    }

    /// The station with its tempo and key filled in from its from call's own when it does not say them (the program
    /// `gen@V GENRE seed=N` prints, with the installed generator).
    pub fn with_defaults(&self) -> Result<Station, Diagnostic> {
        if self.tempo.is_some() && self.key.is_some() {
            return Ok(self.clone());
        }
        let r = crate::gen::resolve_word(&self.from_call(), &Host::default())?;
        let h = crate::gen::host_of(&lex(&r.text).tokens);
        let mut s = self.clone();
        s.tempo = s.tempo.or(h.tempo);
        s.key = s.key.or(h.key);
        Ok(s)
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("text", JsonValue::str(self.text()));
        o.set("cid", JsonValue::str(self.cid()));
        o.set("genre", JsonValue::str(self.genre.clone()));
        o.set("generator", JsonValue::str(format!("gen@{}", self.version)));
        o.set("seed", self.seed.parse::<u64>().map_or_else(|_| JsonValue::str(self.seed.clone()), |n| JsonValue::int(n as i64)));
        o.set("step", JsonValue::str(match self.step {
            SeedStep::By(n) => format!("+{n}"),
            SeedStep::Hash => "~".to_string(),
        }));
        o.set("forms", JsonValue::Array(self.forms.iter().map(|f| JsonValue::str(f.clone())).collect()));
        o.set("tempo", self.tempo.map_or(JsonValue::Null, JsonValue::Number));
        o.set("key", self.key.clone().map_or(JsonValue::Null, JsonValue::str));
        o.set("length", JsonValue::str(self.length.clone().unwrap_or_else(|| "standard".into())));
        o.set("start_cid", JsonValue::str(self.start_cid()));
        o.set("version", JsonValue::str(format!("stations/{VERSION}")));
        JsonValue::Object(o)
    }
}

impl Track {
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("n", JsonValue::int(self.n as i64));
        o.set("seed", JsonValue::int(i64::from(self.seed)));
        o.set("theme", JsonValue::int(i64::from(self.theme)));
        o.set("form", JsonValue::str(self.form.clone()));
        o.set("phrase", self.phrase.clone().map_or(JsonValue::Null, JsonValue::str));
        o.set("text", JsonValue::str(self.text.clone()));
        o.set("cid", JsonValue::str(self.cid.clone()));
        o.set("parent", self.parent.clone().map_or(JsonValue::Null, JsonValue::str));
        JsonValue::Object(o)
    }
}

impl Segue {
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("from", JsonValue::int(self.from as i64));
        o.set("to", JsonValue::int(self.to as i64));
        o.set("form", JsonValue::str("cross"));
        o.set("phrase", JsonValue::str(self.phrase.clone()));
        o.set("text", JsonValue::str(self.text.clone()));
        o.set("cid", JsonValue::str(self.cid.clone()));
        JsonValue::Object(o)
    }
}

/// A key one fifth round the circle (`+1` up, `-1` down), in its own mode: am -> em, c -> g.
fn fifth(key: &str, d: i64) -> Result<String, String> {
    let k = key.trim().to_lowercase();
    let (root, mode) = match k.split_once(':') {
        Some((r, m)) => (r.to_string(), format!(":{m}")),
        None if k.ends_with('m') && k.len() > 1 => (k[..k.len() - 1].to_string(), "m".to_string()),
        None => (k.clone(), String::new()),
    };
    let flats = [("db", "c#"), ("eb", "d#"), ("gb", "f#"), ("ab", "g#"), ("bb", "a#")];
    let sharp = flats.iter().find(|(f, _)| *f == root).map_or(root.as_str(), |(_, s)| *s);
    let at = FIFTHS.iter().position(|r| *r == sharp).ok_or_else(|| format!("`{key}` is no key a notch turns"))? as i64;
    let next = format!("{}{mode}", FIFTHS[(at + d).rem_euclid(12) as usize]);
    if crate::music::Key::parse(&next).is_none() {
        return Err(format!("`{key}` is no key a notch turns"));
    }
    Ok(crate::music::canonical_pitch_word(&next))
}

fn num(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

// ---------------------------------------------------------------- reading a station program

/// The station a program is, or what is wrong with it (with no generator: the call is read, not resolved).
pub fn read(src: &str) -> Result<Station, Vec<Diagnostic>> {
    let parsed = crate::parse::parse(src);
    if parsed.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(parsed.diagnostics);
    }
    let mut diags = Vec::new();
    match from_parsed(&parsed, src, &mut diags) {
        Some((s, _)) if !diags.iter().any(Diagnostic::is_error) => Ok(s),
        _ => Err(diags),
    }
}

/// The station of a parsed program, and the spans of its block and its lines (from, forms).
fn from_parsed(parsed: &Parsed, src: &str, diags: &mut Vec<Diagnostic>) -> Option<(Station, [Span; 3])> {
    let mut block: Option<(&StationBlock, Span)> = None;
    for st in &parsed.program.stmts {
        match &st.kind {
            StmtKind::Station(b) => {
                if block.is_some() {
                    diags.push(Diagnostic::error(st.span, "a program has one `station`"));
                }
                block = Some((b, st.span));
            }
            StmtKind::Setting(Setting::Tempo(_)) | StmtKind::Setting(Setting::Key(_)) => {}
            _ => diags.push(Diagnostic::error(st.span, "a program with a `station` holds the station, its `tempo` and its `key`, nothing else").help("the tracks are the station's: its forms and its seeds make them")),
        }
    }
    // `#! stations: v1`
    for sp in &parsed.comments {
        let line = sp.text(src);
        if let Some(v) = line.trim_start_matches('#').trim_start_matches('!').trim().strip_prefix("stations:") {
            if v.trim() != "v1" {
                diags.push(Diagnostic::error(*sp, format!("stations {} is not here: this host has stations v1", v.trim())).help("#! stations: v1"));
            }
        }
    }
    let (b, span) = block?;
    let (mut from, mut step, mut forms) = (None, None, None);
    for it in &b.items {
        match it {
            StationItem::From(w, sp) => {
                if from.is_some() {
                    diags.push(Diagnostic::error(*sp, "a station has one `from`: its first track"));
                }
                from = Some((w, *sp));
            }
            StationItem::Next(s, sp) => {
                if step.is_some() {
                    diags.push(Diagnostic::error(*sp, "a station has one `next`: its step"));
                }
                step = Some(*s);
            }
            StationItem::Forms(names, sp) => {
                if forms.is_some() {
                    diags.push(Diagnostic::error(*sp, "a station has one `forms` line: the forms in turn"));
                }
                for w in names {
                    if !FORMS.contains(&w.text.as_str()) {
                        let all: Vec<String> = FORMS.iter().map(|f| f.to_string()).collect();
                        let mut d = Diagnostic::error(w.span, format!("`{}` is no form a station takes", w.text)).expected(&FORMS).found(format!("`{}`", w.text));
                        if let Some(n) = crate::env::suggest(&w.text, &all, 1).first() {
                            d = d.help(format!("did you mean `{n}`?"));
                        }
                        diags.push(d);
                    }
                }
                if names.len() > MAX_FORMS {
                    diags.push(Diagnostic::error(*sp, format!("a station takes at most {MAX_FORMS} forms")));
                }
                forms = Some((names.iter().map(|w| w.text.clone()).collect::<Vec<_>>(), *sp));
            }
        }
    }
    let Some((w, from_span)) = from else {
        diags.push(Diagnostic::error(span, "a station says where it starts: `from gen@2 techno seed=42`").help("station {\n  from gen@2 techno seed=42\n}"));
        return None;
    };
    let call: GenCall = match crate::gen::runs(&lex(&w.text).tokens).into_iter().next().map(|r| r.r) {
        Some(Ok(c)) => c,
        Some(Err(mut d)) => {
            d.span = w.span;
            diags.push(d);
            return None;
        }
        None => {
            diags.push(Diagnostic::error(w.span, format!("a station starts from a generator call, not `{}`", w.text)).help("from gen@2 techno seed=42"));
            return None;
        }
    };
    for (said, what) in [(call.section.is_some(), "section="), (call.part.is_some(), "part=")] {
        if said {
            diags.push(Diagnostic::error(w.span, format!("a station starts from a whole track: `{what}` picks a piece of one")).help(format!("from {}", call.program_text())));
        }
    }
    for (said, what) in [(call.bpm.is_some(), "bpm="), (call.key.is_some(), "key=")] {
        if said {
            diags.push(Diagnostic::error(w.span, format!("a station's tempo and key are its `tempo` and `key` lines, not the call's `{what}`")).help("tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n}"));
        }
    }
    let host = crate::gen::host_of(&lex(src).tokens);
    let (forms, forms_span) = forms.unwrap_or((vec!["seeded".to_string()], from_span));
    let s = Station { version: call.version, genre: call.genre.clone(), seed: call.seed.clone(), length: call.length.clone().filter(|l| l != "standard"), step: step.unwrap_or(SeedStep::By(1)), forms, tempo: host.tempo, key: host.key };
    Some((s, [span, from_span, forms_span]))
}

/// Check a program that holds a `station`: read it, fill in its tempo and key, and compile the tracks of its first chapter
/// (one of each form) and the segue from track 0 to 1 with the host's generator. A station compiles to its track 0.
pub fn check(parsed: &Parsed, src: &str, env: &Env, diags: &mut Vec<Diagnostic>) -> Option<Compiled> {
    let (station, [span, from_span, forms_span]) = from_parsed(parsed, src, diags)?;
    if diags.iter().any(Diagnostic::is_error) {
        return None;
    }
    if let Some(g) = crate::gen::generator() {
        let genres = g.genres(station.version);
        if !genres.is_empty() && !genres.contains(&station.genre) {
            let mut d = Diagnostic::error(from_span, format!("`{}` is no genre of gen@{}", station.genre, station.version));
            if let Some(n) = crate::env::suggest(&station.genre, &genres, 3).first() {
                d = d.help(format!("did you mean `{n}`? ({})", genres.join(" ")));
            }
            diags.push(d);
            return None;
        }
    }
    let filled = match station.with_defaults() {
        Ok(s) => s,
        Err(mut d) => {
            d.span = from_span;
            diags.push(d);
            return None;
        }
    };
    let r = filled.forms.len() as u64;
    let tracks = match filled.tracks(0, r) {
        Ok(t) => t,
        Err(e) => {
            diags.push(Diagnostic::error(span, e));
            return None;
        }
    };
    let mut first: Option<Compiled> = None;
    let mut checked = Vec::new();
    for t in tracks.iter().take(r as usize) {
        let c = crate::compile::check_with(&t.text, env);
        match c.compiled {
            Some(comp) => {
                if first.is_none() {
                    first = Some(comp);
                }
                checked.push(t.clone());
            }
            None => {
                let msg = c.diagnostics.iter().find(|d| d.is_error()).map_or_else(String::new, |d| d.message.clone());
                let at = if r == 1 { from_span } else { forms_span };
                diags.push(Diagnostic::error(at, format!("track {} ({} of {} seed={}) does not check: {msg}", t.n, t.form, filled.genre, t.seed)).help(format!("forms {}", FORMS.iter().filter(|f| **f != t.form).copied().collect::<Vec<_>>().join(" "))));
            }
        }
    }
    let segue = match filled.segue(0) {
        Ok(sg) => {
            let c = crate::compile::check_with(&sg.text, env);
            if c.compiled.is_none() {
                let msg = c.diagnostics.iter().find(|d| d.is_error()).map_or_else(String::new, |d| d.message.clone());
                diags.push(Diagnostic::error(from_span, format!("the segue from track 0 to 1 does not check: {msg}")));
            }
            Some(sg)
        }
        Err(e) => {
            diags.push(Diagnostic::error(span, e));
            None
        }
    };
    let mut comp = first?;
    if diags.iter().any(Diagnostic::is_error) {
        return None;
    }
    let report = StationReport { start_cid: filled.start_cid(), station: filled.clone(), tracks: checked, segue };
    let list: Vec<String> = report.tracks.iter().map(|t| format!("{} {} {}", t.n, t.form, short(&t.cid))).collect();
    diags.push(Diagnostic::info(span, format!(
        "station {} (gen@{}): from seed={} {}, forms {}, at tempo {} in key {}; tracks {}; segue 0 to 1 {}",
        filled.genre,
        filled.version,
        filled.seed,
        crate::print::step_word(filled.step).replace("seed=", "next seed="),
        filled.forms.join(" "),
        filled.tempo.map_or("?".into(), num),
        filled.key.clone().unwrap_or_else(|| "?".into()),
        list.join(", "),
        report.segue.as_ref().map_or("-".into(), |s| short(&s.cid)),
    )));
    comp.text = crate::print::canonical(&parsed.program);
    comp.cid = program_cid(&comp.text);
    comp.notes = parsed.comments.iter().map(|sp| sp.text(src).trim_start_matches('#').trim().to_string()).filter(|n| !n.is_empty()).collect();
    comp.station = Some(report);
    Some(comp)
}

fn short(cid: &str) -> String {
    if cid.len() > 12 {
        format!("z…{}", &cid[cid.len() - 8..])
    } else {
        cid.to_string()
    }
}
