//! Genre conventions, read from the bitwrap ontology (https://ontology.bitwrap.io/ns#, CC0): what a genre's tempo,
//! kit, voices, chord progressions and song sections usually are, as the ontology's own data says.
//!
//! - [`Genre::load`] / [`find`]: one genre and what the ontology says of it, through a [`Graph`] (the server's
//!   loaded ontology; tests give a small one). No table here names a genre, a tempo or an instrument: every number
//!   and name comes from the graph, so a new ontology release changes them without a change here.
//! - [`style`]: the style warnings of a compiled program against a genre: a tempo outside the genre's range, a kit or
//!   voice that neither the genre nor its family uses, chords far from every progression the genre has. They are
//!   advice, never errors: `check` lists them under `style` and stays `ok`.
//! - [`template`]: a starting program for a genre: its tempo, key, kit and voices, its family's standard song
//!   structure with each section's active roles and length from the family's section profiles, its drum rhythms and
//!   its first chord progression. It passes `check`.
//!
//! What the ontology does not say is left out, not guessed: no range, no tempo warning; no progression, no chord
//! warning.

use std::collections::{BTreeMap, BTreeSet};

use djlive::compile::Compiled;
use djlive::music::{Key, MODES};
use djlive::scale::Roman;
use serde_json::{json, Map, Value};

/// The ontology's namespace and the base of its individuals.
pub const NS: &str = "https://ontology.bitwrap.io/ns#";
pub const BASE: &str = "https://ontology.bitwrap.io/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// What this module needs of a loaded ontology: the objects of a subject and property (IRIs as IRIs, literals as
/// their plain lexical value, sorted), and the individuals of a class.
pub trait Graph {
    fn all(&self, s: &str, p: &str) -> Vec<String>;
    fn of_type(&self, class: &str) -> Vec<String>;
    fn first(&self, s: &str, p: &str) -> Option<String> {
        self.all(s, p).into_iter().next()
    }
    fn num(&self, s: &str, p: &str) -> Option<f64> {
        self.first(s, p).and_then(|v| v.trim().parse().ok())
    }
}

fn ns(local: &str) -> String {
    format!("{NS}{local}")
}

/// The last segment of an IRI (`.../instrument/sub-bass` is `sub-bass`).
pub fn local(iri: &str) -> &str {
    iri.rsplit(['/', '#']).next().unwrap_or(iri)
}

/// An instrument the ontology names, and the BeatScript voice (or kit) that plays it: its `ns:beatscriptVoice`, else
/// its own name.
#[derive(Clone, Debug, PartialEq)]
pub struct Inst {
    pub iri: String,
    pub name: String,
    pub voice: String,
}

/// One Euclidean rhythm: e(pulses, steps, rotation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rhythm {
    pub pulses: u32,
    pub steps: u32,
    pub rotation: u32,
}

/// A section of a family's song: its name, the roles that play in it and how long it usually is.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    pub roles: Vec<String>,
    pub bars: Option<u32>,
    pub energy: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Genre {
    pub iri: String,
    pub slug: String,
    pub label: String,
    pub family: Option<String>,
    pub bpm: Option<f64>,
    pub min_bpm: Option<f64>,
    pub max_bpm: Option<f64>,
    /// The default scale's local name (`minor`, `dorian`, `minor-pentatonic`).
    pub scale: Option<String>,
    pub root_note: Option<u32>,
    /// 0 (straight) to about 60.
    pub swing: Option<f64>,
    pub kit: Option<Inst>,
    /// role -> the genre's instrument for it (melody, arp, bass).
    pub voices: BTreeMap<String, Inst>,
    /// Instruments the genre usually has (`ns:typicalInstrument`, `ns:typicalMelody`, `ns:typicalArp`, `ns:typicalBass`,
    /// `ns:typicalKit`).
    pub typical: Vec<Inst>,
    /// Each progression's scale degrees (1-based).
    pub progressions: Vec<Vec<u32>>,
    pub steps_per_chord: Option<u32>,
    /// role -> rhythm (kick, snare, hihat).
    pub rhythms: BTreeMap<String, Rhythm>,
}

fn inst(g: &dyn Graph, iri: String) -> Inst {
    let name = local(&iri).to_string();
    let voice = g.first(&iri, &ns("beatscriptVoice")).unwrap_or_else(|| name.clone());
    Inst { iri, name, voice }
}

impl Genre {
    /// The genre with this IRI, None when the graph has no such genre.
    pub fn load(g: &dyn Graph, iri: &str) -> Option<Genre> {
        if !g.all(iri, RDF_TYPE).iter().any(|t| *t == ns("Genre")) {
            return None;
        }
        let first = |p: &str| g.first(iri, &ns(p));
        let num = |p: &str| g.num(iri, &ns(p));
        let slug = first("genreId").unwrap_or_else(|| local(iri).to_string());
        let mut voices = BTreeMap::new();
        // the generator's instrument for the role, else the convention's (`ns:typicalMelody` ...): the graph is the
        // asserted triples, so each property is read by name
        let cap = |r: &str| format!("{}{}", r[..1].to_uppercase(), &r[1..]);
        for role in ["melody", "arp", "bass"] {
            if let Some(i) = first(&format!("{role}Instrument")).or_else(|| first(&format!("typical{}", cap(role)))) {
                voices.insert(role.to_string(), inst(g, i));
            }
        }
        let mut typical: Vec<Inst> = Vec::new();
        for p in ["typicalInstrument", "typicalMelody", "typicalArp", "typicalBass", "typicalKit"] {
            for i in g.all(iri, &ns(p)) {
                if !typical.iter().any(|t| t.iri == i) {
                    typical.push(inst(g, i));
                }
            }
        }
        let mut rhythms = BTreeMap::new();
        for role in ["kick", "snare", "hihat"] {
            if let Some(r) = first(&format!("{role}Rhythm")) {
                if let (Some(p), Some(s)) = (g.num(&r, &ns("pulses")), g.num(&r, &ns("steps"))) {
                    rhythms.insert(role.to_string(), Rhythm { pulses: p as u32, steps: s as u32, rotation: g.num(&r, &ns("rotation")).unwrap_or(0.0) as u32 });
                }
            }
        }
        let progs = g.all(iri, &ns("hasProgression"));
        let progressions = progs.iter().filter_map(|p| g.first(p, &ns("degrees"))).map(|d| d.split_whitespace().filter_map(|x| x.parse().ok()).collect::<Vec<u32>>()).filter(|v| !v.is_empty()).collect();
        let steps_per_chord = progs.first().and_then(|p| g.num(p, &ns("stepsPerChord"))).map(|x| x as u32);
        Some(Genre {
            iri: iri.to_string(),
            label: g.first(iri, RDFS_LABEL).unwrap_or_else(|| slug.clone()),
            slug,
            family: first("inFamily"),
            bpm: num("bpm"),
            min_bpm: num("minBpm"),
            max_bpm: num("maxBpm"),
            scale: first("defaultScale").map(|s| local(&s).to_string()),
            root_note: num("rootNote").map(|x| x as u32),
            swing: num("swing"),
            kit: first("drumKit").or_else(|| first("typicalKit")).map(|i| inst(g, i)),
            voices,
            typical,
            progressions,
            steps_per_chord,
            rhythms,
        })
    }

    /// Every BeatScript voice and kit name the genre itself uses.
    fn own_names(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        for i in self.kit.iter().chain(self.voices.values()).chain(self.typical.iter()) {
            s.insert(i.voice.clone());
            s.insert(i.name.clone());
        }
        s
    }
}

/// Every genre in the graph, by slug.
pub fn genres(g: &dyn Graph) -> Vec<Genre> {
    let mut v: Vec<Genre> = g.of_type(&ns("Genre")).iter().filter_map(|i| Genre::load(g, i)).collect();
    v.sort_by(|a, b| a.slug.cmp(&b.slug));
    v
}

/// The genre `text` names: a slug (`techno`), a label, `genre:techno`, `bw:Techno` or its IRI. Case and spacing do
/// not matter.
pub fn find(g: &dyn Graph, text: &str) -> Option<Genre> {
    let t = text.trim();
    let t = t.strip_prefix("genre:").or_else(|| t.strip_prefix("bw:")).unwrap_or(t);
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let want = norm(local(t));
    genres(g).into_iter().find(|x| x.iri == t || norm(&x.slug) == want || norm(&x.label) == want)
}

/// The other genres of `genre`'s family.
pub fn family_of(g: &dyn Graph, genre: &Genre) -> Vec<Genre> {
    match &genre.family {
        None => Vec::new(),
        Some(f) => genres(g).into_iter().filter(|x| x.family.as_deref() == Some(f) && x.iri != genre.iri).collect(),
    }
}

/// The family's standard song: its first structure of kind `standard` (by IRI), each slot's section with the family's
/// section profile for it. Empty when the ontology has none.
pub fn structure(g: &dyn Graph, genre: &Genre) -> Vec<Section> {
    let Some(fam) = &genre.family else { return Vec::new() };
    let mut structs: Vec<String> = g
        .of_type(&ns("SongStructure"))
        .into_iter()
        .filter(|s| g.first(s, &ns("forFamily")).as_deref() == Some(fam.as_str()) && g.first(s, &ns("templateKind")).as_deref() == Some("standard"))
        .collect();
    structs.sort();
    let Some(st) = structs.first() else { return Vec::new() };
    let mut slots: Vec<(u32, String)> = g.all(st, &ns("hasSlot")).iter().filter_map(|x| Some((g.num(x, &ns("position"))? as u32, g.first(x, &ns("ofSection"))?))).collect();
    slots.sort();
    let profiles: Vec<String> = g.of_type(&ns("SectionProfile")).into_iter().filter(|p| g.first(p, &ns("forFamily")).as_deref() == Some(fam.as_str())).collect();
    slots
        .into_iter()
        .map(|(_, sec)| {
            let prof = profiles.iter().find(|p| g.first(p, &ns("ofSection")).as_deref() == Some(sec.as_str()));
            let name = g.first(&sec, RDFS_LABEL).unwrap_or_else(|| local(&sec).to_string());
            Section {
                name,
                roles: prof.map(|p| g.all(p, &ns("activeRole")).iter().map(|r| local(r).to_string()).collect()).unwrap_or_default(),
                bars: prof.and_then(|p| g.num(p, &ns("bars"))).or_else(|| g.num(&sec, &ns("bars"))).map(|b| b as u32),
                energy: prof.and_then(|p| g.num(p, &ns("energy"))),
            }
        })
        .collect()
}

// ---------- style warnings ----------

/// One style warning: advice, never an error.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// `tempo`, `kit`, `voice` or `progression`.
    pub code: &'static str,
    pub message: String,
    /// 1-based, when the warning is about one place in the text.
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl Warning {
    pub fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("severity".into(), json!("style"));
        m.insert("code".into(), json!(self.code));
        m.insert("message".into(), json!(self.message));
        if let (Some(l), Some(c)) = (self.line, self.col) {
            m.insert("line".into(), json!(l));
            m.insert("col".into(), json!(c));
        }
        Value::Object(m)
    }
}

/// The 1-based line and column of the first match of `needle` in `text` that starts a word.
fn place(text: &str, needle: &str) -> (Option<usize>, Option<usize>) {
    for (i, l) in text.lines().enumerate() {
        let mut from = 0;
        while let Some(at) = l[from..].find(needle) {
            let a = from + at;
            let before = l[..a].chars().next_back();
            let after = l[a + needle.len()..].chars().next();
            if before.is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_')) && after.is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_')) {
                return (Some(i + 1), Some(l[..a].chars().count() + 1));
            }
            from = a + needle.len();
        }
    }
    (None, None)
}

fn fmt_bpm(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// Every part of the program: its role, voice and kit, and the hits of its cycles.
fn parts(c: &Compiled) -> impl Iterator<Item = &djlive::compile::PartIr> {
    c.decks.values().chain(c.segments.iter().flat_map(|s| s.decks.values())).flat_map(|d| d.parts.iter())
}

const DRUM_ROLES: &[&str] = &["kick", "snare", "hihat", "clap", "drums", "perc", "tom", "ride", "crash", "cymbal", "open-hat", "rim"];

/// The scale degrees (1-based) the program's harmony moves through, chord by chord: each chord a `harmony` part strikes
/// (its notes at one step), in the order of the bar, as the degree whose triad the notes are (in any voicing:
/// `voice-lead` turns chords over, and a chord's lowest note is then its third or its fifth), else the degree of the
/// lowest note. A bar's first hit counts even when it is one note; a lone note later in the bar is a line moving over
/// the chord, not a new chord. The same chord struck again in a bar is not counted twice in a row. Without a harmony
/// part, a `bass` part's first hit of each bar. None without a key or without such a part.
pub fn degrees(c: &Compiled) -> Option<Vec<u32>> {
    let key = c.key?;
    let iv = key.intervals();
    let n = iv.len();
    let pc = |note: u8| (i64::from(note) - i64::from(key.tonic)).rem_euclid(12);
    let degree = |at: &[u8]| -> Option<u32> {
        let tones: BTreeSet<i64> = at.iter().map(|x| pc(*x)).collect();
        let triad = (0..n).find(|d| tones == [0, 2, 4].iter().map(|k| iv[(d + k) % n].rem_euclid(12)).collect::<BTreeSet<i64>>());
        let low = at.iter().min().map(|x| pc(*x));
        triad.or_else(|| low.and_then(|l| iv.iter().position(|x| x.rem_euclid(12) == l))).map(|d| d as u32 + 1)
    };
    let pick = |role: &str, every_chord: bool| -> Vec<u32> {
        let mut out = Vec::new();
        for p in parts(c).filter(|p| p.role == role) {
            for hits in &p.cycles {
                let steps: BTreeSet<u8> = hits.iter().map(|h| h.step).collect();
                let mut last = None;
                for (i, s) in steps.iter().enumerate() {
                    let at: Vec<u8> = hits.iter().filter(|h| h.step == *s).map(|h| h.note).collect();
                    if i > 0 && (!every_chord || at.len() < 2) {
                        continue;
                    }
                    if let Some(d) = degree(&at) {
                        if last != Some(d) {
                            out.push(d);
                        }
                        last = Some(d);
                    }
                }
            }
        }
        out
    };
    let h = pick("harmony", true);
    let d = if h.is_empty() { pick("bass", false) } else { h };
    (!d.is_empty()).then_some(d)
}

/// How close a program's degrees are to a progression: the share of degrees the two have in common (Jaccard).
fn closeness(a: &[u32], b: &[u32]) -> f64 {
    let (x, y): (BTreeSet<u32>, BTreeSet<u32>) = (a.iter().copied().collect(), b.iter().copied().collect());
    let u = x.union(&y).count();
    if u == 0 {
        return 1.0;
    }
    x.intersection(&y).count() as f64 / u as f64
}

/// Chords further than this from every one of the genre's progressions are "far" from them.
pub const FAR: f64 = 0.34;

/// The style warnings of `c` (whose source is `text`) against `genre`. `family` is the genre's family (from
/// [`family_of`]): a kit or voice one of them uses is not unusual.
pub fn style(c: &Compiled, text: &str, genre: &Genre, family: &[Genre]) -> Vec<Warning> {
    let mut out = Vec::new();
    let label = &genre.label;
    // tempo: the genre's range, when the ontology gives one
    if let (Some(t), Some(lo), Some(hi)) = (c.tempo, genre.min_bpm, genre.max_bpm) {
        if t < lo || t > hi {
            let (line, col) = place(text, "tempo");
            let how = if t < lo { "slow" } else { "fast" };
            out.push(Warning { code: "tempo", message: format!("tempo {} is {how} for {label}: it usually runs {}-{} BPM", fmt_bpm(t), fmt_bpm(lo), fmt_bpm(hi)), line, col });
        }
    }
    let mut usual = genre.own_names();
    for f in family {
        usual.extend(f.own_names());
    }
    let mut seen = BTreeSet::new();
    // kits
    for k in parts(c).filter_map(|p| p.kit.clone()) {
        if usual.contains(&k) || !seen.insert(format!("kit={k}")) {
            continue;
        }
        let (line, col) = place(text, &format!("kit={k}"));
        let theirs = genre.kit.as_ref().map(|i| format!(" (its kit is {})", i.voice)).unwrap_or_default();
        out.push(Warning { code: "kit", message: format!("kit {k} is unusual for {label}{theirs}"), line, col });
    }
    // voices: a tonal part's instrument that neither the genre nor its family uses (only when the genre names any)
    if !genre.voices.is_empty() || !genre.typical.is_empty() {
        for p in parts(c).filter(|p| !DRUM_ROLES.contains(&p.role.as_str())) {
            let Some(v) = &p.instrument else { continue };
            if usual.contains(v) || !seen.insert(format!("inst={v}")) {
                continue;
            }
            let (line, col) = place(text, &format!("inst={v}"));
            let theirs: Vec<String> = genre.voices.values().chain(genre.typical.iter()).map(|i| i.voice.clone()).collect::<BTreeSet<_>>().into_iter().collect();
            out.push(Warning { code: "voice", message: format!("voice {v} is unusual for {label} (its voices: {})", theirs.join(", ")), line, col });
        }
    }
    // chords: far from every progression the genre has
    if !genre.progressions.is_empty() {
        if let Some(d) = degrees(c) {
            let best = genre.progressions.iter().map(|p| closeness(&d, p)).fold(0.0, f64::max);
            if best < FAR {
                let mut uniq: Vec<u32> = Vec::new();
                for x in &d {
                    if uniq.last() != Some(x) {
                        uniq.push(*x);
                    }
                }
                let show = |v: &[u32]| v.iter().take(8).map(u32::to_string).collect::<Vec<_>>().join(" ");
                let theirs: Vec<String> = genre.progressions.iter().map(|p| show(p)).collect();
                out.push(Warning { code: "progression", message: format!("the chords move through degrees {} of the key, far from {label}'s progressions ({})", show(&uniq), theirs.join("; ")), line: None, col: None });
            }
        }
    }
    out
}

// ---------- the template ----------

/// The mode word a key takes for an ontology scale (`minor` -> `m`, `dorian` -> `:dorian`); None for one
/// BeatScript keys do not have.
fn mode_of(scale: &str) -> Option<usize> {
    let m = match scale {
        "harmonic-minor" => "harmonic",
        "pentatonic" => "pent",
        "minor-pentatonic" => "minpent",
        s => s,
    };
    MODES.iter().position(|x| *x == m)
}

/// The hits of e(pulses, steps, rotation): Bjorklund's spread, as the language's `e(...)`.
fn euclid(r: Rhythm) -> Vec<bool> {
    let (p, n) = (r.pulses.min(r.steps), r.steps.max(1));
    let mut v: Vec<bool> = (0..n).map(|i| (i * p) % n < p).collect();
    // start on a hit, as e(p, n) does
    v.rotate_left((r.rotation % n) as usize);
    v
}

/// The template's text for `genre` (`voices` and `kits` are the names BeatScript has; a genre's instrument that is
/// neither is left to the role's own voice).
pub fn template(g: &dyn Graph, genre: &Genre, voices: &BTreeSet<String>, kits: &BTreeSet<String>) -> String {
    let mut s = String::new();
    s.push_str(&format!("#! name: {} starter\n#! genre: {}\n#! license: CC0-1.0\n", genre.label, genre.slug));
    s.push_str(&format!("# A starting point for {}, from the bitwrap ontology: its tempo, key, kit and voices, its family's\n# standard song structure and its first chord progression. Change anything.\n", genre.label));
    if let Some(b) = genre.bpm {
        s.push_str(&format!("tempo {}\n", fmt_bpm(b)));
    }
    if let Some(sw) = genre.swing.filter(|x| *x > 0.0) {
        s.push_str(&format!("swing {}\n", fmt_bpm((sw / 100.0 * 100.0).round() / 100.0)));
    }
    let key = Key { tonic: (genre.root_note.unwrap_or(0) % 12) as u8, mode: genre.scale.as_deref().and_then(mode_of).unwrap_or(1) };
    s.push_str(&format!("key {}\n\n", key.text()));
    let voice = |role: &str| genre.voices.get(role).map(|i| i.voice.clone()).filter(|v| voices.contains(v));
    let kit = genre.kit.as_ref().map(|i| i.voice.clone()).filter(|k| kits.contains(k));
    // the chords, one a bar: the first progression (else the tonic), as Roman numerals. A numeral says the degree and the
    // colour, never the pitch: a `key` line (the program's, or a section's) moves the chords, the bass, the arp and the
    // tune together
    let prog: Vec<u32> = genre.progressions.first().cloned().unwrap_or_else(|| vec![1]);
    let chords: Vec<String> = prog.iter().map(|d| Roman::triad(key, *d).text()).collect();
    s.push_str(&format!("let chords = <{}>\n", chords.join(" ")));
    // the bass: the chord's root where the kick plays (a numeral in a bass part is its bass tone), a bar per chord
    let kick = genre.rhythms.get("kick").copied().unwrap_or(Rhythm { pulses: 4, steps: 16, rotation: 0 });
    let ek = euclid(kick);
    let n = ek.len();
    let kick16: Vec<bool> = (0..16).map(|i| (i * n) % 16 == 0 && ek[i * n / 16]).collect();
    let bar = |f: &dyn Fn(usize) -> Option<String>| (0..16).map(|i| f(i).unwrap_or_else(|| "~".into())).collect::<Vec<_>>().join(" ");
    let bass_bars: Vec<String> = chords.iter().map(|c| bar(&|i| kick16[i].then(|| c.clone()))).collect();
    // the arp: the chord's root, third and fifth in sixteenths, four to a beat (`i*4`: an arp plays a numeral in four steps);
    // the melody: three chord tones a bar, the fifth, the third and the root (`i/5`, `i/3`, `i`: the tone a numeral puts in the bass)
    let arp_bars: Vec<String> = chords.iter().map(|c| format!("{c}*4")).collect();
    let mel_bars: Vec<String> = chords
        .iter()
        .map(|c| {
            bar(&|i| match i {
                0 => Some(format!("{c}/5")),
                6 => Some(format!("{c}/3")),
                8 => Some(c.clone()),
                _ => None,
            })
        })
        .collect();
    // one `let` a bar, then each part an alternation of them (a chord a bar)
    for (name, bars) in [("bass", &bass_bars), ("arp", &arp_bars), ("tune", &mel_bars)] {
        for (i, b) in bars.iter().enumerate() {
            s.push_str(&format!("let {name}{} = {b}\n", i + 1));
        }
        s.push_str(&format!("let {name}line = <{}>\n", (1..=bars.len()).map(|i| format!("{name}{i}")).collect::<Vec<_>>().join(" ")));
    }
    s.push('\n');
    // the sections: the family's standard song, each with its profile's roles and length
    let mut secs = structure(g, genre);
    if secs.is_empty() {
        secs.push(Section { name: "loop".into(), roles: vec!["kick".into(), "snare".into(), "hihat".into(), "bass".into(), "harmony".into(), "melody".into()], bars: None, energy: None });
    }
    let e = |role: &str| genre.rhythms.get(role).map(|r| if r.rotation > 0 { format!("e({},{},{})", r.pulses, r.steps, r.rotation) } else { format!("e({},{})", r.pulses, r.steps) });
    let mut defined = BTreeSet::new();
    let mut order = Vec::new();
    for sec in &secs {
        let name: String = sec.name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
        order.push(name.clone());
        if !defined.insert(name.clone()) {
            continue;
        }
        let has = |r: &str| sec.roles.iter().any(|x| x == r);
        let mut a: Vec<String> = Vec::new();
        for (i, role) in ["kick", "snare", "hihat"].iter().enumerate() {
            if has(role) {
                if let Some(p) = e(role) {
                    let k = if i == 0 || a.is_empty() { kit.as_ref().map(|k| format!(" kit={k}")).unwrap_or_default() } else { String::new() };
                    a.push(format!("{role} {p}{k}"));
                }
            }
        }
        if has("clap") {
            if let Some(p) = e("snare") {
                a.push(format!("clap {p}"));
            }
        }
        if has("bass") {
            a.push(format!("bass bassline{}", voice("bass").map(|v| format!(" inst={v}")).unwrap_or_default()));
        }
        let mut b: Vec<String> = Vec::new();
        if has("harmony") {
            b.push("harmony chords".into());
        }
        if has("arp") {
            b.push(format!("arp arpline{}", voice("arp").map(|v| format!(" inst={v}")).unwrap_or_default()));
        }
        if has("melody") {
            b.push(format!("melody tuneline{}", voice("melody").map(|v| format!(" inst={v}")).unwrap_or_default()));
        }
        let bars = sec.bars.unwrap_or(8);
        s.push_str(&format!("section {name} {bars} bars {{\n"));
        if !a.is_empty() {
            s.push_str(&format!("  deck A: {}\n", a.join(" | ")));
        }
        if !b.is_empty() {
            s.push_str(&format!("  deck B: {}\n", b.join(" | ")));
        }
        if a.is_empty() && b.is_empty() {
            s.push_str("  deck A: kick ~\n");
        }
        s.push_str("}\n\n");
    }
    s.push_str(&format!("arrange {}\n", order.join(" ")));
    s
}

/// The BeatScript voices and kits (`/api/instruments`), as two sets of names.
pub fn instrument_names() -> (BTreeSet<String>, BTreeSet<String>) {
    let v: Value = serde_json::from_str(&btwapi::instruments_json()).unwrap_or(Value::Null);
    let names = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string).or_else(|| x["name"].as_str().map(str::to_string))).collect()).unwrap_or_default();
    (names("voices"), names("kits"))
}

/// What a genre is, for the `genre_template` answer.
pub fn summary(g: &dyn Graph, genre: &Genre) -> Value {
    let i = |x: &Inst| json!({"instrument": x.iri, "voice": x.voice});
    json!({
        "genre": genre.iri,
        "label": genre.label,
        "family": genre.family,
        "tempo": genre.bpm,
        "tempo_range": match (genre.min_bpm, genre.max_bpm) { (Some(a), Some(b)) => json!([a, b]), _ => Value::Null },
        "scale": genre.scale,
        "kit": genre.kit.as_ref().map(i),
        "voices": genre.voices.iter().map(|(r, x)| (r.clone(), i(x))).collect::<Map<String, Value>>(),
        "progressions": genre.progressions,
        "sections": structure(g, genre).iter().map(|s| json!({"name": s.name, "bars": s.bars, "roles": s.roles, "energy": s.energy})).collect::<Vec<_>>(),
    })
}

// ---------- the server's ontology, and what check and the MCP tool answer ----------

/// A loaded ontology the process shares.
pub type Shared = std::sync::Arc<dyn Graph + Send + Sync>;

static GRAPH: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();

/// Give the process its ontology (the server's loaded one): once; a later call is ignored.
pub fn install(g: Shared) {
    let _ = GRAPH.set(g);
}

/// The installed ontology, None until [`install`].
pub fn installed() -> Option<Shared> {
    GRAPH.get().cloned()
}

/// [`check_style`] with the installed ontology.
pub fn installed_style(text: &str, genre: &str) -> Value {
    let g = installed();
    check_style(g.as_ref().map(|a| a.as_ref() as &dyn Graph), text, genre)
}

/// [`template_answer_full`] with the installed ontology.
pub fn installed_template(genre: &str, phrase: Option<&str>, form: Option<&str>, theme: Option<&str>, origin: &str) -> Result<Value, String> {
    let g = installed();
    template_answer_full(g.as_ref().map(|a| a.as_ref() as &dyn Graph), genre, phrase, form, theme, origin)
}

/// The genre a check is about: the `genre` argument, else the text's `#! genre:` line. None: neither names one.
pub fn wanted(text: &str, arg: Option<&str>) -> Option<String> {
    arg.map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).or_else(|| djlive::linked::read_links(text).genre)
}

/// What `check` adds as `style` for a genre (`genre` from [`wanted`]): the genre's IRI and label and the style
/// warnings, or, when the ontology is not loaded or has no such genre, an empty list and a `note` saying so. Never an
/// error: a program that checks stays ok. A program that does not compile has no style warnings (its errors say why).
pub fn check_style(g: Option<&dyn Graph>, text: &str, genre: &str) -> Value {
    let Some(g) = g else {
        return json!({"genre": genre, "warnings": [], "note": "genre conventions are not available on this server right now"});
    };
    let Some(gn) = find(g, genre) else {
        return json!({"genre": genre, "warnings": [], "note": format!("the bitwrap ontology has no genre {genre:?}: see list_genres at https://ontology.bitwrap.io")});
    };
    let env = btwapi::web_env().env();
    let warnings: Vec<Value> = match djlive::compile::check_with(text, &env).compiled {
        Some(c) => style(&c, text, &gn, &family_of(g, &gn)).iter().map(Warning::json).collect(),
        None => Vec::new(),
    };
    json!({"genre": gn.iri, "label": gn.label, "warnings": warnings})
}

/// The `genre_template` answer: the starting program, its CID and check (it is checked here, and must pass), and
/// what the ontology says of the genre. Err: the ontology has no such genre (or is not loaded).
pub fn template_answer(g: Option<&dyn Graph>, genre: &str) -> Result<Value, String> {
    template_answer_with(g, genre, None)
}

/// Where a phrase is sung in a template: after each `melody tuneline` part's deck line, a `sing` line over that part's notes.
/// Returns the text and how many sections got the line.
pub fn with_vocal(text: &str, sing: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + 64);
    let mut n = 0;
    for l in text.lines() {
        out.push_str(l);
        out.push('\n');
        let t = l.trim_start();
        if t.starts_with("deck B:") && t.contains("melody tuneline") {
            out.push_str(&format!("{}deck B: sing {sing} on melody\n", &l[..l.len() - t.len()]));
            n += 1;
        }
    }
    (out, n)
}

/// [`template_answer`] with a phrase to sing (found-phrases, docs/btw/found-phrases.md): a phrasebook id or any words. The
/// phrase is sung on the melody part of each section that has one, and the answer gains `phrase`: the line's source, meter,
/// stress, syllables and escapes, and how well its meter fits the genre. A genre whose structure has no melody part says so
/// in `phrase.note` and the program is the genre's own, unsung.
pub fn template_answer_with(g: Option<&dyn Graph>, genre: &str, phrase: Option<&str>) -> Result<Value, String> {
    let g = g.ok_or_else(|| "genre conventions are not available on this server right now".to_string())?;
    let gn = find(g, genre).ok_or_else(|| format!("the bitwrap ontology has no genre {genre:?}; known: {}", genres(g).iter().map(|x| x.slug.clone()).collect::<Vec<_>>().join(", ")))?;
    let (voices, kits) = instrument_names();
    let mut text = template(g, &gn, &voices, &kits);
    let mut sung = None;
    if let Some(p) = phrase.map(str::trim).filter(|p| !p.is_empty()) {
        let mut chosen = crate::found::choose(p, Some(&gn.slug))?;
        let (t, n) = with_vocal(&text, &chosen.sing);
        if n == 0 {
            chosen.json["note"] = json!(format!("{} has no melody part in its song structure, so there is nothing to sing on: add `deck B: sing {} on melody` after a melody part", gn.label, chosen.sing));
        } else {
            text = t;
            chosen.json["sung_in_sections"] = json!(n);
        }
        sung = Some(chosen.json);
    }
    let rep: Value = serde_json::from_str(&btwapi::check_json(&text)).map_err(|e| e.to_string())?;
    if rep["ok"] != true {
        return Err(format!("the template for {} did not pass check: {}", gn.label, rep["diagnostics"]));
    }
    let mut out = json!({"text": text, "cid": rep["cid"], "canonical": rep["canonical"], "seconds": rep["seconds"], "genre": summary(g, &gn)});
    if let Some(s) = sung {
        out["phrase"] = s;
        out["warnings"] = rep["warnings"].clone();
    }
    Ok(out)
}

/// [`template_answer_with`], and two more ways in (docs/btw/forms.md):
///
/// - `form` (quodlibet, passacaglia, canon, rondo, fugue, theme-and-variations): the program is the form's template at the
///   genre's tempo and swing, `arrange from template(<form>, <phrase>)` (the phrase given, else the form's own example's),
///   and the answer gains `form` (its shape, plan, mutations, themes and reference track) and `expanded`, the program
///   written out as plain BeatScript to edit. The form keeps its own key: its themes are written in it.
/// - `theme` (an id of the public-domain theme library): the genre's own program with the theme's lets in it, moved to the
///   genre's key, played by the part of its role in every section that has one; the answer gains `theme`.
///
/// A form brings its own themes, so `form` and `theme` are one or the other.
pub fn template_answer_full(g: Option<&dyn Graph>, genre: &str, phrase: Option<&str>, form: Option<&str>, theme: Option<&str>, origin: &str) -> Result<Value, String> {
    let form = form.map(str::trim).filter(|f| !f.is_empty());
    let theme = theme.map(str::trim).filter(|f| !f.is_empty());
    if form.is_some() && theme.is_some() {
        return Err("give form or theme, not both: a form brings its own themes (see its `themes`)".into());
    }
    let Some(form) = form else {
        let mut out = template_answer_with(g, genre, phrase)?;
        if let Some(id) = theme {
            let gr = g.ok_or_else(|| "genre conventions are not available on this server right now".to_string())?;
            let gn = find(gr, genre).ok_or_else(|| format!("the bitwrap ontology has no genre {genre:?}"))?;
            let (text, j) = crate::themes::splice(out["text"].as_str().unwrap_or(""), id, i32::from((gn.root_note.unwrap_or(0) % 12) as u8))?;
            let rep: Value = serde_json::from_str(&btwapi::check_json(&text)).map_err(|e| e.to_string())?;
            if rep["ok"] != true {
                return Err(format!("the template for {} with the theme {id} did not pass check: {}", gn.label, rep["diagnostics"]));
            }
            for k in ["cid", "canonical", "seconds"] {
                out[k] = rep[k].clone();
            }
            out["text"] = json!(text);
            out["theme"] = j;
            out["warnings"] = rep["warnings"].clone();
        }
        return Ok(out);
    };
    let g = g.ok_or_else(|| "genre conventions are not available on this server right now".to_string())?;
    let gn = find(g, genre).ok_or_else(|| format!("the bitwrap ontology has no genre {genre:?}; known: {}", genres(g).iter().map(|x| x.slug.clone()).collect::<Vec<_>>().join(", ")))?;
    let (t, f) = crate::forms::find(form)?;
    let (sing, sung) = match phrase.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            let c = crate::found::choose(p, Some(&gn.slug))?;
            (c.sing, Some(c.json))
        }
        None => (crate::forms::default_phrase(t).to_string(), None),
    };
    let mut text = format!("#! name: {} {}\n#! genre: {}\n#! license: CC0-1.0\n", gn.label, f.label, gn.slug);
    text.push_str(&format!("# A {} (docs/forms.md) at the tempo of {}: {}. Its themes and its key are the form's; `expand` writes it out to edit.\n", f.label, gn.label, f.shape));
    if let Some(b) = gn.bpm {
        text.push_str(&format!("tempo {}\n", fmt_bpm(b)));
    }
    if let Some(sw) = gn.swing.filter(|x| *x > 0.0) {
        text.push_str(&format!("swing {}\n", fmt_bpm((sw / 100.0 * 100.0).round() / 100.0)));
    }
    text.push_str(&format!("arrange from template({}, {sing})\n", t.name));
    let rep: Value = serde_json::from_str(&btwapi::check_json(&text)).map_err(|e| e.to_string())?;
    if rep["ok"] != true {
        return Err(format!("the {} for {} did not pass check: {}", f.label, gn.label, rep["diagnostics"]));
    }
    let expanded: Value = serde_json::from_str(&btwapi::expand_json(&text, false)).map_err(|e| e.to_string())?;
    let mut out = json!({"text": text, "cid": rep["cid"], "canonical": rep["canonical"], "seconds": rep["seconds"], "genre": summary(g, &gn), "form": crate::forms::form_json(origin, t, f), "expanded": expanded["text"], "warnings": rep["warnings"]});
    if let Some(s) = sung {
        out["phrase"] = s;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
