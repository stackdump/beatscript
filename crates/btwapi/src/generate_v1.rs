//! `seeded@1`, frozen: the seeded generator as it printed until 2026-11-22, when `seeded@2` wrote its chords as Roman numerals
//! (425458c). A `gen@1` call (djlive::gen) is this printer, kept so that a call that named it keeps the music it named: a version
//! is never reprinted. Copied from `generate.rs` as it was at 425458c^, with its JSON wrappers left out (generate.rs has them);
//! do not edit it: a change here is a change to every `gen@1` program (tests/gen.rs pins a sample). One thing it cannot freeze:
//! the instruments it picks are filtered by the web catalogue (`snapshot.json`), as they always were.
//!
//! `generate`: the seeded, non-LLM composer. Genre, BPM, key, length and a seed in; a whole BeatScript program out,
//! as source a person reads and edits: named `let` patterns, a `def` for the groove the sections share, sections and
//! an `arrange`, filter ramps, and generator calls (`e()`, `chord()`, `arp()`, `markov()`, `every`/`rotate`/`degrade`)
//! with their seeds written out.
//!
//! The *choices* come from the beats generator's own tables and its mulberry32 stream (djrust `gen`): the genre's
//! euclid parameters, drum-style variants, chord progressions, instrument sets, song blueprints and per-section energy.
//! This module only prints them as BeatScript. Pure: no clock, no I/O, so the same inputs are the same text, and the
//! same text is the same CID. The body (everything after the `#!` header) is already canonical: `fmt` does not change it.

use std::fmt::Write as _;

use djengine::gen::composer::generate_track_name;
use djengine::gen::structure::generate_structure;
use djengine::gen::tables::{BeatsTables, ChordProg, GenreDef};
use djengine::json::JsonObject;
use djengine::rng::Mulberry32;

use crate::snapshot::web_env;

/// `#! generator:` names this; bump the version whenever the same inputs would print a different program.
pub const GENERATOR: &str = "seeded";
pub const GENERATOR_VERSION: u32 = 1;

/// What a caller may set. Empty / `None` fields take the genre's own.
#[derive(Clone, Debug, Default)]
pub struct GenRequest {
    pub genre: String,
    pub bpm: Option<f64>,
    /// `am`, `c`, `f#m`, `d:dorian`
    pub key: String,
    /// `short` | `standard` | `long`
    pub length: String,
    /// A number or a word.
    pub seed: String,
}

#[derive(Clone, Debug)]
pub struct Generated {
    /// The whole program: the `#!` header then the canonical body.
    pub text: String,
    /// The body alone (what `fmt` prints back unchanged).
    pub body: String,
    pub name: String,
    pub genre: String,
    pub seed: String,
    pub seed_value: u32,
    pub bpm: f64,
    pub key: String,
    pub length: String,
}

/// The genres `generate` knows (beats' catalogue).
pub fn genres() -> Vec<String> {
    let t = BeatsTables::shared();
    t.genre_ids.iter().filter(|g| t.genres.get(*g).is_some_and(|d| !d.kick.is_empty())).cloned().collect()
}

/// A seed as given -> the generator's 32-bit seed. A whole number in range is itself; a word is its FNV-1a hash
/// (lower-cased, trimmed), so `"sunrise"` is the same track everywhere.
pub fn seed_value(seed: &str) -> u32 {
    let s = seed.trim();
    if let Ok(n) = s.parse::<u64>() {
        return (n & 0xffff_ffff) as u32;
    }
    let mut h: u32 = 0x811c_9dc5;
    for b in s.to_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ---------- music helpers ----------

const NOTE_NAMES: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
const FLAT_NAMES: [&str; 12] = ["c", "db", "d", "eb", "e", "f", "gb", "g", "ab", "a", "bb", "b"];

/// The key's tonic (pitch class), BTW mode word ("" major, "m" minor, or a `:mode`) and the heptatonic parent the
/// chords are spelled against.
#[derive(Clone, Debug)]
struct Key {
    pc: i64,
    /// `c`, `am`, `d:dorian`: how the program writes it
    text: String,
    parent: &'static [i64],
    flats: bool,
}

const MAJOR: [i64; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i64; 7] = [0, 2, 3, 5, 7, 8, 10];
const DORIAN: [i64; 7] = [0, 2, 3, 5, 7, 9, 10];
const MIXO: [i64; 7] = [0, 2, 4, 5, 7, 9, 10];
const PHRYG: [i64; 7] = [0, 1, 3, 5, 7, 8, 10];
const HARM: [i64; 7] = [0, 2, 3, 5, 7, 8, 11];

fn mode_of(word: &str) -> Option<(&'static str, &'static [i64])> {
    Some(match word {
        "" | "major" => ("", &MAJOR),
        "m" | "minor" => ("m", &MINOR),
        "dorian" => (":dorian", &DORIAN),
        "mixolydian" => (":mixolydian", &MIXO),
        "phrygian" => (":phrygian", &PHRYG),
        "harmonic" => (":harmonic", &HARM),
        "pent" => (":pent", &MAJOR),
        "minpent" => (":minpent", &MINOR),
        "blues" => (":blues", &MINOR),
        _ => return None,
    })
}

fn genre_mode(scale: &str) -> &'static str {
    match scale {
        "MinorScale" => "m",
        "DorianScale" => "dorian",
        "MixolydianScale" => "mixolydian",
        "PhrygianScale" => "phrygian",
        "HarmonicMinScale" => "harmonic",
        "PentatonicScale" => "pent",
        "MinPentatonicScale" => "minpent",
        "BluesScale" => "blues",
        _ => "",
    }
}

fn tonic_pc(t: &str) -> Option<(i64, bool)> {
    let b = t.as_bytes();
    let base = match b.first()? {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    match &t[1..] {
        "" => Some((base, false)),
        "#" => Some(((base + 1) % 12, false)),
        "b" => Some(((base + 11) % 12, true)),
        _ => None,
    }
}

fn make_key(pc: i64, mode: &str, flats: bool) -> Key {
    let (suffix, parent) = mode_of(mode).unwrap_or(("", &MAJOR));
    let names = if flats { &FLAT_NAMES } else { &NOTE_NAMES };
    Key { pc, text: format!("{}{suffix}", names[pc as usize]), parent, flats }
}

/// `am`, `c`, `f#m`, `bb:dorian` -> a Key; None when it is not one.
fn parse_key(s: &str) -> Option<Key> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some((t, m)) = s.split_once(':') {
        let (pc, flats) = tonic_pc(t)?;
        mode_of(m)?;
        return Some(make_key(pc, m, flats));
    }
    if let Some((pc, flats)) = tonic_pc(&s) {
        return Some(make_key(pc, "", flats));
    }
    let t = s.strip_suffix('m')?;
    let (pc, flats) = tonic_pc(t)?;
    Some(make_key(pc, "m", flats))
}

impl Key {
    fn name(&self, pc: i64) -> &'static str {
        let names = if self.flats { &FLAT_NAMES } else { &NOTE_NAMES };
        names[pc.rem_euclid(12) as usize]
    }
    /// The diatonic triad on scale degree `d` (0-based) as a BTW chord word: `am`, `f`, `bdim`.
    fn chord(&self, d: i64) -> String {
        let p = self.parent;
        let at = |i: i64| p[i.rem_euclid(7) as usize] + 12 * i.div_euclid(7);
        let root = at(d);
        let third = at(d + 2) - root;
        let fifth = at(d + 4) - root;
        let q = match (third, fifth) {
            (4, 7) => "",
            (3, 7) => "m",
            (3, 6) => "dim",
            (4, 8) => "aug",
            _ => "",
        };
        format!("{}{q}", self.name(self.pc + root))
    }
}

// ---------- the printer ----------

fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.strip_prefix("0.").map_or(s.to_string(), |f| format!("0.{f}"))
    }
}

fn euclid(k: i64, n: i64, rot: i64) -> String {
    let n = n.clamp(1, 64);
    let k = k.clamp(0, n);
    let rot = rot.rem_euclid(n);
    if rot == 0 {
        format!("e({k}, {n})")
    } else {
        format!("e({k}, {n}, {rot})")
    }
}

struct Pick<'a> {
    rng: &'a mut Mulberry32,
}

impl Pick<'_> {
    fn int(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            self.rng.next_int(n)
        }
    }
    fn seed(&mut self) -> i64 {
        1 + self.int(9998) as i64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.rng.next() < p
    }
    fn of<'b>(&mut self, v: &'b [String]) -> Option<&'b String> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.int(v.len())])
        }
    }
}

/// A voice the program may name: a real voice (not just a family word) that starts with a letter.
fn voice_ok(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase()) && web_env().catalog.voices.iter().any(|v| v == name)
}

fn kit_ok(name: &str) -> bool {
    web_env().catalog.kits.iter().any(|v| v == name)
}

fn instrument(t: &BeatsTables, genre: &str, role: &str, p: &mut Pick) -> Option<String> {
    let mut cands: Vec<String> = t
        .instrument_set(genre)
        .and_then(|m| m.get(role).cloned())
        .or_else(|| t.fallback_set(role))
        .unwrap_or_default()
        .into_iter()
        .filter(|n| voice_ok(n))
        .collect();
    if cands.is_empty() {
        cands = match role {
            "bass" => vec!["sub-bass".into()],
            "harmony" => vec!["dark-pad".into()],
            "arp" => vec!["bright-pluck".into()],
            _ => vec!["lead".into()],
        };
        cands.retain(|n| voice_ok(n));
    }
    p.of(&cands).cloned()
}

fn kit(t: &BeatsTables, genre: &str, p: &mut Pick) -> Option<String> {
    let cands: Vec<String> = t
        .instrument_set(genre)
        .and_then(|m| m.get("kick").cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| kit_ok(n))
        .collect();
    p.of(&cands).cloned()
}

/// The genre's euclid triple for a drum, with a drum-style variant applied.
fn drum(def: &GenreDef, role: &str, variant: &str) -> (i64, i64, i64) {
    let base = match role {
        "kick" => &def.kick,
        "snare" => &def.snare,
        _ => &def.hihat,
    };
    let (mut k, n, mut r) = (base.first().copied().unwrap_or(4), base.get(1).copied().unwrap_or(16), base.get(2).copied().unwrap_or(0));
    if let Some(v) = def.theory.as_ref().and_then(|t| t.drum_styles.get(role)).and_then(|s| s.get(variant)) {
        k = ((k as f64) * v.hits_mul.unwrap_or(1.0)).round() as i64 + v.hits_add;
        r += v.rotation_add;
    }
    (k.clamp(1, n), n, r)
}

/// A bass figure for one chord: eight eighths, hits from a euclid of the genre's density, the chord root (as a
/// degree of the key) with an octave jump or a fifth now and then.
fn bass_bar(root_deg: i64, density: f64, p: &mut Pick, walking: bool) -> String {
    let k = ((density * 8.0).round() as i64).clamp(2, 7);
    let hits = djengine::gen::euclidean::bjorklund(k, 8);
    let d = root_deg + 1;
    let fifth = (root_deg + 4) % 7 + 1;
    let mut terms = Vec::new();
    for (i, h) in hits.iter().enumerate() {
        if *h == 0 {
            terms.push("~".to_string());
            continue;
        }
        let t = if i > 0 && walking && p.chance(0.35) {
            ((root_deg + 1 + p.int(4) as i64) % 7 + 1).to_string()
        } else if i > 0 && p.chance(0.2) {
            fifth.to_string()
        } else if i > 0 && p.chance(0.15) {
            (d + 7).to_string()
        } else {
            d.to_string()
        };
        terms.push(t);
    }
    format!("[{}]", terms.join(" "))
}

fn section_bars(steps: i64) -> i64 {
    // the blueprint's sixteenth counts, read at eighths: 64 -> 8 bars, 192 -> 24
    ((steps / 8).max(4) / 4) * 4
}

/// Cutoff (Hz) for a section's energy.
fn cutoff(open: f64) -> i64 {
    let hz = 300.0 + open.clamp(0.0, 1.0).powi(2) * 15_700.0;
    ((hz / 100.0).round() * 100.0) as i64
}

/// Build the program.
pub fn generate(req: &GenRequest) -> Result<Generated, String> {
    let t = BeatsTables::shared();
    let genre = if req.genre.trim().is_empty() { "techno".to_string() } else { req.genre.trim().to_lowercase() };
    let def = t.genres.get(&genre).filter(|d| !d.kick.is_empty()).ok_or_else(|| format!("unknown genre {genre:?}; one of: {}", genres().join(", ")))?;
    let seed_text = if req.seed.trim().is_empty() { "1".to_string() } else { req.seed.trim().to_string() };
    let sv = seed_value(&seed_text);
    let length = match req.length.trim() {
        "" | "standard" | "medium" => "standard",
        "short" | "minimal" => "short",
        "long" | "extended" => "long",
        other => return Err(format!("unknown length {other:?}; one of: short, standard, long")),
    };
    let size = match length {
        "short" => "minimal",
        "long" => "extended",
        _ => "standard",
    };
    let bpm = req.bpm.unwrap_or(def.bpm);
    if !(40.0..=300.0).contains(&bpm) {
        return Err(format!("bpm {bpm} is out of range (40..300)"));
    }
    let key = if req.key.trim().is_empty() {
        let pc = def.root_note.rem_euclid(12);
        make_key(pc, genre_mode(&def.scale_name), matches!(pc, 1 | 3 | 5 | 8 | 10) && genre_mode(&def.scale_name) == "")
    } else {
        parse_key(&req.key).ok_or_else(|| format!("{:?} is not a key (write am, c, f#m, d:dorian)", req.key))?
    };

    let mut rng = Mulberry32::new(sv as f64);
    let tmpl = generate_structure(&genre, size, &mut rng);
    let mut p = Pick { rng: &mut rng };

    // ---- material ----
    let progs: Vec<ChordProg> = def.theory.as_ref().map(|th| th.chord_progs.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| {
        vec![if key.parent[2] == 3 { t.minor_chord_prog.clone() } else { t.major_chord_prog.clone() }]
    });
    let prog = progs[p.int(progs.len())].clone();
    let mut roots: Vec<i64> = prog.chords.iter().map(|c| c.root.rem_euclid(7)).collect();
    if roots.is_empty() {
        roots = vec![0, 5, 2, 4];
    }
    // the seed turns the progression: the same chords, a different first one
    let turn = p.int(roots.len());
    roots.rotate_left(turn);
    let chords: Vec<String> = roots.iter().map(|d| key.chord(*d)).collect();

    let kick = drum(def, "kick", "A");
    let snare = drum(def, "snare", "A");
    let hat = drum(def, "hihat", "A");
    let hat_b = drum(def, "hihat", "B");
    let kick_c = drum(def, "kick", "C");
    let swing = (def.swing / 100.0).clamp(0.0, 0.6);

    let inst_bass = instrument(t, &genre, "bass", &mut p);
    let inst_lead = instrument(t, &genre, "melody", &mut p);
    let inst_pad = instrument(t, &genre, "harmony", &mut p);
    let inst_arp = instrument(t, &genre, "arp", &mut p);
    let drum_kit = kit(t, &genre, &mut p);

    let bassline: Vec<String> = roots.iter().map(|d| bass_bar(*d, def.bass_density, &mut p, def.walking_bass)).collect();
    let density = num(def.melody_density.clamp(0.1, 0.9));
    let lead_seed = p.seed();
    let answer_seed = p.seed();
    let hat_degrade_seed = p.seed();
    let ghost_seed = p.seed();
    let thin_seed = p.seed();
    let arp_mode = ["up", "down", "updown"][p.int(3)];
    let arp_steps = [8, 16][p.int(2)];
    let fill_rot = p.int(4) as i64;
    let lead_shift = p.int(2) as i64 * 2 + 1;

    // ---- print ----
    let mut b = String::new();
    let ln = |b: &mut String, s: &str| {
        b.push_str(s);
        b.push('\n');
    };
    ln(&mut b, &format!("tempo {}", num(bpm)));
    ln(&mut b, &format!("key {}", key.text));
    if swing > 0.0 {
        ln(&mut b, &format!("swing {}", num(swing)));
    }
    let kick_let = euclid(kick.0, kick.1, kick.2);
    ln(&mut b, &format!("let pulse = {kick_let}"));
    ln(&mut b, &format!("let pulse_b = {}", euclid(kick_c.0, kick_c.1, kick_c.2)));
    ln(&mut b, &format!("let backbeat = {}", euclid(snare.0, snare.1, snare.2)));
    ln(&mut b, &format!("let hats = {}", euclid(hat.0, hat.1, hat.2)));
    ln(&mut b, &format!("let hats_b = {}", euclid(hat_b.0, hat_b.1, hat_b.2)));
    let mut fill = vec!["~"; 16];
    let fills: [&[usize]; 4] = [&[10, 12, 13, 14, 15], &[12, 13, 14, 15], &[8, 11, 14, 15], &[10, 11, 13, 14, 15]];
    for i in fills[fill_rot as usize] {
        fill[*i] = "x";
    }
    ln(&mut b, &format!("let fill = {}", fill.join(" ")));
    ln(&mut b, &format!("let chords = <{}>", chords.iter().map(|c| format!("chord({c}, 3)")).collect::<Vec<_>>().join(" ")));
    ln(&mut b, &format!("let arps = <{}>", chords.iter().map(|c| format!("arp({c}, {arp_mode}, {arp_steps})")).collect::<Vec<_>>().join(" ")));
    ln(&mut b, &format!("let bassline = <{}>", bassline.join(" ")));
    // fmt spells a `:mode` key inside a call with a space after the colon (`markov(g: dorian, ...)`)
    let k = key.text.replace(':', ": ");
    ln(&mut b, &format!("let lead = <markov({k}, {density}, {lead_seed}) markov({k}, {density}, {answer_seed})>"));
    let inst = |name: &Option<String>| name.as_ref().map(|n| format!(" inst={n}")).unwrap_or_default();
    let kit_param = drum_kit.as_ref().map(|n| format!(" kit={n}")).unwrap_or_default();
    // the groove every full section shares
    ln(&mut b, "def groove(k, h) {");
    ln(&mut b, &format!("  deck A: kick $k{kit_param} | snare every(4, [fill], backbeat) | hihat $h vel=70..100"));
    ln(&mut b, "}");
    ln(&mut b, "def band(m) {");
    ln(
        &mut b,
        &format!("  deck B: bass bassline{} | harmony chords{} | melody $m{}", inst(&inst_bass), inst(&inst_pad), inst(&inst_lead)),
    );
    ln(&mut b, "}");

    let theme = t.theme_table(&genre);
    let mut seen: Vec<String> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for s in &tmpl.sections {
        order.push(s.name.clone());
        if seen.contains(&s.name) {
            continue;
        }
        seen.push(s.name.clone());
        let bars = section_bars(s.steps);
        let has = |r: &str| s.active.contains(r);
        let open = theme.and_then(|th| th.energy.get(&s.name)).map(|e| e.filter_open).unwrap_or(match s.name.as_str() {
            "intro" | "outro" | "breakdown" => 0.4,
            "buildup" | "verse" | "pre-chorus" => 0.6,
            _ => 1.0,
        });
        ln(&mut b, &format!("section {} {bars} bars {{", s.name));
        match s.name.as_str() {
            "intro" | "buildup" | "pre-chorus" => {
                let from = cutoff(open * 0.5);
                let to = cutoff((open + 0.4).min(1.0));
                ln(&mut b, &format!("  master: filter lp {from} -> {to} over {bars} bars exp"));
            }
            "outro" => {
                ln(&mut b, &format!("  master: filter lp {} -> {} over {bars} bars", cutoff(open), cutoff(0.1)));
            }
            "breakdown" | "bridge" => {
                ln(&mut b, &format!("  master: filter lp {}", cutoff(open)));
                ln(&mut b, "  master: reverb 0.35");
            }
            _ => {
                ln(&mut b, "  master: filter off");
                ln(&mut b, "  master: reverb 0.15");
            }
        }
        let full = has("kick") && has("snare") && has("hihat");
        let drop_like = matches!(s.name.as_str(), "drop" | "chorus");
        if full {
            let (kp, hp) = if drop_like {
                ("pulse".to_string(), "hats".to_string())
            } else if s.name == "verse" {
                ("pulse".to_string(), format!("degrade(hats, 0.2, {hat_degrade_seed})"))
            } else {
                ("pulse_b".to_string(), "rotate(hats_b, 2)".to_string())
            };
            ln(&mut b, &format!("  groove({kp}, {hp})"));
        } else if has("kick") || has("hihat") || has("snare") {
            let mut parts = Vec::new();
            if has("kick") {
                parts.push(format!("kick pulse{kit_param}"));
            }
            if has("snare") {
                parts.push("snare backbeat".to_string());
            }
            if has("hihat") {
                parts.push(format!("hihat degrade(hats, 0.4, {ghost_seed}) vel=60..90"));
            }
            ln(&mut b, &format!("  deck A: {}", parts.join(" | ")));
        }
        let tonal = has("bass") && has("harmony") && has("melody");
        if tonal && !has("arp") {
            if drop_like {
                ln(&mut b, "  band(lead)");
            } else {
                ln(&mut b, &format!("  band(transpose(lead, {}))", if lead_shift == 1 { 12 } else { 0 }));
            }
        } else {
            let mut parts = Vec::new();
            if has("bass") {
                parts.push(format!("bass bassline{}", inst(&inst_bass)));
            }
            if has("harmony") {
                parts.push(format!("harmony chords{}", inst(&inst_pad)));
            }
            if has("melody") {
                parts.push(format!("melody lead{}", inst(&inst_lead)));
            }
            if has("arp") {
                parts.push(format!("arp arps{}", inst(&inst_arp)));
            }
            if !parts.is_empty() {
                ln(&mut b, &format!("  deck B: {}", parts.join(" | ")));
            }
        }
        if drop_like && full && bars >= 8 {
            ln(&mut b, "  evolve every 4 bars {");
            ln(&mut b, &format!("    hihat: thin 0.15 seed={thin_seed}"));
            if has("melody") {
                ln(&mut b, "    melody: transpose <0 5 7 0>");
            }
            ln(&mut b, &format!("    stop when gen == {}", (bars / 4 - 1).max(1)));
            ln(&mut b, "  }");
        }
        ln(&mut b, "}");
    }
    ln(&mut b, &format!("arrange {}", order.join(" ")));

    let name = generate_track_name(&genre, sv as f64);
    let mut text = String::new();
    let _ = writeln!(text, "#! name: {name}");
    let _ = writeln!(text, "#! generator: {GENERATOR}@{GENERATOR_VERSION}");
    let _ = writeln!(text, "#! seed: {seed_text}");
    let _ = writeln!(text, "#! genre: {genre}");
    let _ = writeln!(text, "#! length: {length}");
    text.push_str(&b);
    Ok(Generated { text, body: b, name, genre, seed: seed_text, seed_value: sv, bpm, key: key.text, length: length.to_string() })
}
