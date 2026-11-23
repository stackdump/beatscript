//! A recorded set written back out as a program (docs/BEATSCRIPT.md, "Sets"): the tracklist of a
//! `djrust.set-recording/1` (djengine::setrec) as a BeatScript `set { ... }`, in the syntax the
//! language already has, and the take beside it, as a performance is beside its program.
//!
//! - **The program** is `play` lines and `mix` lines. A tracklist line is a program when its CID is
//!   the program CID of a program here (the store, or what the caller hands in): it becomes
//!   `play <cid>`, cut with `until bar N` (in the track's own bars, from the marks' bars) where the
//!   next track cuts it short. The transitions into a program become one `mix N bars with T`: N is
//!   the bars from the incoming track's mark to the last transition into it, T the transition's
//!   name, or the nearest one with a `# performed: <kind>` line above it when the booth's move has
//!   no set spelling (bass_swap, loop_roll, phrase_sync, vocoder_handoff: crossfade), and
//!   `, tempo A -> B` when the master tempo moved over it.
//! - **A generator track** (a `genre:seed` net the arrangement loaded with nothing changed: no
//!   traits, overrides or part seeds) is the one-line program `arrange from seed(genre, n)`, which
//!   plays the generator's notes where it plays them (djlive's seed.rs and its tests): it becomes a
//!   `play` of that program, whose text goes in the take (`tracks`) so `check` can read it.
//! - **What is not a program** is said in a comment, `# not a program: <kind> <label>`: a mashup, a
//!   loop set, raw nets, a net with changes. Its bars are said too: after the first program the
//!   track before plays on through them (its `until` reaches the next program), and before it they
//!   are a `# gap: N bars`, and the set starts later in the recording. A sample deck's content id
//!   (audio from outside, not a `z…` CID) also makes the program `use restricted` and private.
//! - **A live-coded program** reaches the decks as raw nets named `live deck A` / `live deck B`, a
//!   new one at each section: the tracklist has a line for each, and none is the program. They are
//!   the program's own sections once a program is marked before them; with no program marked they
//!   are one gap. A program is in a tracklist only when the booth marks it (`record mark`, kind
//!   track, its program CID).
//! - **The take** is canonical JSON, `{format: 1, type: djrust.set-take, program, recording,
//!   author, tracklist, arrangement}` (and `tracks`, `controls`, `inputs` when there are any), with
//!   a CID made the way a performance's is (djlive::perform::Performance). It names its program by
//!   CID; the program text never holds it.
//!
//! The header lines (`#! name:`, license, visibility, author) are the live store's (live_store.rs),
//! comments to the language, so they are out of the program CID. Pure but for [`cli`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{short_cid, License, Rights, Visibility};
use djengine::session::action::DeckAction;
use djengine::session::arrangement::{bar_beat, ArrangeOp, Arrangement};
use djengine::session::model::DeckSource;
use djengine::setrec::{clock, Entry, EntryKind, RecordedSet};
use djengine::TrackSpec;
use djlive::env::{Env, Snippets};

use crate::live_store::Object;
use crate::stores::{pretty_sorted, read_json, write_atomic};

/// The take object's `type`.
pub const TAKE_TYPE: &str = "djrust.set-take";

/// What [`set_program`] is told besides the recording.
#[derive(Clone, Copy, Default)]
pub struct Options<'a> {
    /// The author the program and the take name ([`recording_author`]).
    pub author: Option<&'a str>,
    /// Where a tracklist CID is looked up to see if it is a program (by its whole program CID).
    pub programs: Option<&'a dyn Snippets>,
}

/// A recorded set as a program, and the take that names it.
#[derive(Clone, Debug, PartialEq)]
pub struct SetProgram {
    /// The `.beatscript` file: the header lines, then the `set`.
    pub text: String,
    /// `djlive::program_cid` of the canonical text.
    pub program_cid: String,
    pub take: JsonValue,
    pub take_cid: String,
    /// The programs it plays that are no saved program (a generator track's `arrange from seed(...)`):
    /// (program CID, canonical text), also in the take as `tracks`.
    pub tracks: Vec<(String, String)>,
}

impl SetProgram {
    /// What `record program` answers.
    pub fn to_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from_pairs([
            ("text", JsonValue::str(self.text.clone())),
            ("program_cid", JsonValue::str(self.program_cid.clone())),
            ("take_cid", JsonValue::str(self.take_cid.clone())),
        ]))
    }
}

/// The author a recording's program names: the recording's own `author`, else `$DJ_AUTHOR`.
pub fn recording_author(set_json: &JsonValue) -> Option<String> {
    set_json
        .get("author")
        .and_then(JsonValue::as_str)
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .or_else(crate::tracks::default_author)
}

/// What a tracklist line is.
enum What {
    /// `seed`: the generator call it is written as, when it is a generator track.
    Program { text: String, seed: Option<String> },
    /// `restricted`: audio from outside (a sample deck's content id).
    NotProgram { restricted: bool },
    /// The raw nets a live-coded program loads (`live deck A`): one of its sections, not the program.
    LiveNets,
    Transition,
}

/// A sample deck's content id (a sha-256 in hex, `seq-…`, `demo-loop`), not a `z…` CID: audio.
fn is_audio_id(cid: &str) -> bool {
    !cid.is_empty() && !cid.starts_with('z')
}

/// The program `cid` names, when it is one here: the lookup has it under that whole CID, and its
/// text is the program the CID names.
fn program_text(programs: Option<&dyn Snippets>, cid: &str) -> Option<String> {
    let (c, text) = programs?.resolve(cid).ok()?;
    (c == cid && djlive::program_cid(&text) == cid).then_some(text)
}

/// The generator net the arrangement loaded under `cid` (what the decks held at the start, or a
/// load or swap).
fn generator_spec(arr: &Arrangement, cid: &str) -> Option<TrackSpec> {
    let moved = arr.events.iter().filter_map(|e| match &e.op {
        ArrangeOp::Deck(DeckAction::Load { source, .. }) | ArrangeOp::Swap { source, .. } => Some(source),
        _ => None,
    });
    arr.decks.values().chain(moved).find_map(|s| match s {
        DeckSource::Net(spec) if djengine::cid::share_cid(&spec.payload()) == cid => Some(spec.clone()),
        _ => None,
    })
}

/// A generator track as the program that plays its notes: (`seed(genre, n[, structure])`, the
/// canonical text of `arrange from` it). None unless the net is the generator's track and nothing
/// else (a seed's genre, a whole seed, a structure, and no traits, overrides or part seeds).
fn seed_program(arr: Option<&Arrangement>, cid: &str) -> Option<(String, String)> {
    let spec = generator_spec(arr?, cid)?;
    let n = spec.seed;
    if n.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&n) || !djlive::seed::GENRES.contains(&spec.genre.as_str()) {
        return None;
    }
    // the generator's "loop" is no structure at all (a net that says "loop" is another track)
    let structure = match spec.structure.as_deref() {
        None => "loop",
        Some(s) if s != "loop" && djlive::seed::STRUCTURES.contains(&s) => s,
        _ => return None,
    };
    if spec != TrackSpec::new(&spec.genre, n, spec.structure.as_deref()) {
        return None;
    }
    let expr = if structure == djlive::seed::STRUCTURES[0] { format!("seed({}, {})", spec.genre, n as u32) } else { format!("seed({}, {}, {structure})", spec.genre, n as u32) };
    let c = djlive::check(&format!("arrange from {expr}\n")).compiled?;
    Some((expr, c.text))
}

/// The master tempo at `beat`: the arrangement's, each move's tempo holding after it.
fn bpm_at(arr: Option<&Arrangement>, fallback: f64, beat: f64) -> f64 {
    let Some(a) = arr else { return fallback };
    let mut bpm = if a.bpm > 0.0 { a.bpm } else { fallback };
    for e in &a.events {
        if e.at > beat + 1e-9 {
            break;
        }
        if e.bpm > 0.0 {
            bpm = e.bpm;
        }
    }
    bpm
}

/// A transition line's move: the first word of its label (`bass_swap B: swapped`, `crossfade A → B`,
/// `auto-dj: …`), as a set would spell it, and whether the set has that spelling.
fn move_of(label: &str) -> (String, bool) {
    let w = label.split_whitespace().next().unwrap_or("").trim_end_matches(':').to_ascii_lowercase();
    if djlive::set::TRANSITIONS.contains(&w.as_str()) {
        return (w, true);
    }
    match crate::perform::MoveKind::parse(&w) {
        Some(k) => (k.name().to_string(), false),
        None if w.is_empty() => ("transition".into(), false),
        None => (w, false),
    }
}

/// A play of the set.
struct Play {
    cid: String,
    label: String,
    /// The generator call it is written as.
    seed: Option<String>,
    /// The bar it starts on (1-based, the recording's).
    bar: i64,
    /// Its natural length in bars (None: it loops, has no end, or does not compile here).
    natural: Option<u32>,
}

/// The mix into a play.
struct Mix {
    transition: String,
    performed: Vec<String>,
    /// The last transition's beat.
    end_beat: f64,
    bars: u32,
    /// The bars the recording overlapped, when the set's had to be fewer.
    clamped_from: Option<u32>,
}

/// "8 bars", "1 bar".
fn bars_word(n: i64) -> String {
    format!("{n} bar{}", if n == 1 { "" } else { "s" })
}

/// The program a recorded set is, and its take. Errors when no tracklist line is a program here.
pub fn set_program(rec: &RecordedSet, opts: &Options) -> Result<SetProgram, String> {
    let mut list: Vec<Entry> = rec.tracklist.clone();
    list.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
    let arr = rec.replay();
    let bar = |e: &Entry| bar_beat(e.beat).0;
    let last_bar = (rec.bars.1 - rec.bars.0 + 1).max(list.iter().map(bar).max().unwrap_or(1));
    let check_env = Env { snippets: opts.programs, ..Env::none() };

    // what each line is
    let whats: Vec<What> = list
        .iter()
        .map(|e| {
            if e.kind == EntryKind::Transition {
                What::Transition
            } else if e.cid.is_empty() {
                What::NotProgram { restricted: false }
            } else if is_audio_id(&e.cid) {
                What::NotProgram { restricted: true }
            } else if let Some(text) = program_text(opts.programs, &e.cid) {
                What::Program { text, seed: None }
            } else if let Some((expr, text)) = (e.kind == EntryKind::Track).then(|| seed_program(arr.as_ref(), &e.cid)).flatten() {
                What::Program { text, seed: Some(expr) }
            } else if e.kind == EntryKind::Track && e.label.starts_with("live deck") {
                What::LiveNets
            } else {
                What::NotProgram { restricted: false }
            }
        })
        .collect();

    // the tracks of the timeline: each line that is one, and the play it is if it is one. A program
    // marked again while it plays (no other track between) goes on; a live-coded program's nets are
    // its own once a program is marked (within a bar)
    let mut plays: Vec<Play> = Vec::new();
    let mut generated: Vec<(String, String)> = Vec::new();
    let mut restricted = false;
    let mut last_track: Option<usize> = None;
    let mut track_play: Vec<Option<Option<usize>>> = vec![None; list.len()];
    for (i, (e, w)) in list.iter().zip(&whats).enumerate() {
        match w {
            What::Program { text, seed } => {
                let cid = djlive::program_cid(text);
                if last_track.and_then(|j| track_play[j].flatten()).is_some_and(|p| plays[p].cid == cid) {
                    continue;
                }
                let natural = djlive::compile::check_with(text, &check_env).compiled.and_then(|c| {
                    restricted |= c.rights.private_required || c.rights.declared_private;
                    match (c.arranged, c.looped) {
                        (true, true) => None,
                        (true, false) => c.end.or_else(|| c.song_bars()),
                        (false, _) => c.end,
                    }
                });
                if seed.is_some() && !generated.iter().any(|(c, _)| *c == cid) {
                    generated.push((cid.clone(), text.clone()));
                }
                let label = if e.label.is_empty() { opts.programs.and_then(|s| s.label(&cid)).unwrap_or_else(|| short_cid(&cid)) } else { e.label.clone() };
                plays.push(Play { cid, label, seed: seed.clone(), bar: bar(e), natural });
                track_play[i] = Some(Some(plays.len() - 1));
                last_track = Some(i);
            }
            What::NotProgram { restricted: r } => {
                restricted |= r;
                track_play[i] = Some(None);
                last_track = Some(i);
            }
            What::LiveNets => {
                let owned = list.iter().zip(&whats).any(|(p, w)| matches!(w, What::Program { .. }) && p.beat <= e.beat + 4.0);
                if !owned {
                    track_play[i] = Some(None);
                    last_track = Some(i);
                }
            }
            What::Transition => {}
        }
    }
    if plays.is_empty() {
        return Err("set-program: no line of the tracklist is a program here, so there is no set to write: a program is in a \
                    tracklist when the booth marks it (record mark, kind track, its program CID) and it is saved here"
            .into());
    }
    let first_beat = list[(0..list.len()).find(|&i| track_play[i].flatten().is_some()).unwrap_or(0)].beat;
    // the bar the next track after line i starts on
    let next_track_bar = |i: usize| (i + 1..list.len()).find(|&j| track_play[j].is_some()).map_or(last_bar + 1, |j| bar(&list[j]));

    // each transition goes to the mix into the track that started last before it; one into a line
    // that is no program (or into the first play) is a comment where it fell
    let mut into: Vec<Vec<usize>> = vec![Vec::new(); plays.len()];
    let mut stray: Vec<bool> = vec![false; list.len()];
    for (i, w) in whats.iter().enumerate() {
        if !matches!(w, What::Transition) {
            continue;
        }
        let incoming = (0..list.len()).rev().find(|&j| j != i && track_play[j].is_some() && list[j].beat <= list[i].beat + 1e-9);
        match incoming.and_then(|j| track_play[j].flatten()) {
            Some(p) if p > 0 => into[p].push(i),
            _ => stray[i] = true,
        }
    }

    // the mixes
    let mut mixes: Vec<Option<Mix>> = Vec::new();
    for (p, ts) in into.iter().enumerate() {
        if p == 0 || ts.is_empty() {
            mixes.push(None);
            continue;
        }
        let moves: Vec<(String, bool)> = ts.iter().map(|&i| move_of(&list[i].label)).collect();
        // a move the set spells (filter_exit, echo_out, cut) is what the mix is; else a crossfade
        let chosen = moves.iter().find(|(m, spelled)| *spelled && m != "crossfade").map_or("crossfade".to_string(), |(m, _)| m.clone());
        let mut performed: Vec<String> = Vec::new();
        for (m, spelled) in &moves {
            let said = *spelled && (m == &chosen || m == "crossfade");
            if !said && !performed.contains(m) {
                performed.push(m.clone());
            }
        }
        let last = *ts.iter().max_by(|a, b| list[**a].beat.partial_cmp(&list[**b].beat).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        let bars = if chosen == "cut" { 0 } else { (bar(&list[last]) - plays[p].bar).max(1) as u32 };
        mixes.push(Some(Mix { transition: chosen, performed, end_beat: list[last].beat, bars, clamped_from: None }));
    }

    // how long each play plays (in its own bars), and what it plays of that
    let lens: Vec<u32> = (0..plays.len())
        .map(|p| {
            let n = match plays.get(p + 1) {
                Some(next) => next.bar - plays[p].bar + mixes[p + 1].as_ref().map_or(0, |m| i64::from(m.bars)),
                None => last_bar - plays[p].bar + 1,
            };
            n.max(1) as u32
        })
        .collect();
    let plays_for: Vec<u32> = plays.iter().zip(&lens).map(|(pl, n)| pl.natural.map_or(*n, |l| l.min(*n))).collect();
    // a mix fits both tracks and leaves room for the mix at the track's other end
    for p in 1..plays.len() {
        let head = mixes[p - 1].as_ref().map_or(0, |m| m.bars);
        if let Some(m) = mixes[p].as_mut() {
            let room = plays_for[p - 1].saturating_sub(head).min(plays_for[p]);
            if m.bars > room {
                m.clamped_from = Some(m.bars);
                m.bars = room;
            }
        }
        if mixes[p].as_ref().is_some_and(|m| m.bars == 0 && m.transition != "cut") {
            // no room for a mix: its moves are said where they fell
            for &i in &into[p] {
                stray[i] = true;
            }
            mixes[p] = None;
        }
    }

    // a run of live nets with no program named: one gap (the run starts at its first line)
    let mut live_runs: Vec<Option<(usize, usize)>> = vec![None; list.len()];
    let mut run: Option<usize> = None;
    for (i, w) in whats.iter().enumerate() {
        match (w, track_play[i]) {
            (What::LiveNets, Some(_)) => {
                let start = *run.get_or_insert(i);
                let (n, _) = live_runs[start].unwrap_or((0, i));
                live_runs[start] = Some((n + 1, i));
            }
            (_, Some(_)) => run = None,
            _ => {}
        }
    }

    // the text, line by line in the order things happened
    let mut body = String::from("set {\n");
    let mut line = |s: &str| {
        body.push_str("  ");
        body.push_str(s);
        body.push('\n');
    };
    let first_bar = bar_beat(first_beat).0;
    for (i, (e, w)) in list.iter().zip(&whats).enumerate() {
        match w {
            What::Program { .. } => {
                let Some(p) = track_play[i].flatten() else { continue };
                if let Some(m) = &mixes[p] {
                    for k in &m.performed {
                        line(&format!("# performed: {k}"));
                    }
                    if let Some(n) = m.clamped_from {
                        line(&format!("# the recording overlapped {} here", bars_word(i64::from(n))));
                    }
                    // (`bars` even for one: the canonical spelling)
                    let mut s = format!("mix {} bars with {}", m.bars, m.transition);
                    let round = |x: f64| (x * 100.0).round() / 100.0;
                    let (from, to) = (round(bpm_at(arr.as_ref(), rec.bpm, e.beat)), round(bpm_at(arr.as_ref(), rec.bpm, m.end_beat)));
                    if m.bars > 0 && from != to {
                        s.push_str(&format!(", tempo {} -> {}", djlive::print::num(from), djlive::print::num(to)));
                    }
                    line(&s);
                }
                let pl = &plays[p];
                match &pl.seed {
                    Some(expr) => line(&format!("# {} {}: the generator's notes, as arrange from {expr}", clock(e.t), pl.label)),
                    None => line(&format!("# {} {}", clock(e.t), pl.label)),
                }
                let until = match pl.natural {
                    Some(l) if l <= lens[p] => String::new(),
                    _ => format!(" until bar {}", lens[p]),
                };
                line(&format!("play {}{until}", pl.cid));
                if let Some(l) = pl.natural.filter(|l| *l < lens[p]) {
                    line(&format!("# it ends at its bar {l}; the recording went on to bar {}", lens[p]));
                }
            }
            What::NotProgram { restricted } => {
                let mut what = e.kind.name().to_string();
                for s in [e.label.clone(), if e.cid.is_empty() { String::new() } else { format!("({})", short_cid(&e.cid)) }] {
                    if !s.is_empty() {
                        what.push(' ');
                        what.push_str(&s);
                    }
                }
                if *restricted {
                    what.push_str(", outside audio");
                }
                let (from, to) = (bar(e), next_track_bar(i) - 1);
                if to < from {
                    line(&format!("# not a program: {what}"));
                } else if e.beat < first_beat {
                    line(&format!("# gap: {} ({what}): the set starts at bar {first_bar} of the recording", bars_word(to - from + 1)));
                } else {
                    line(&format!("# not a program: {what}, bars {from}-{to}: the track before plays on through them"));
                }
            }
            What::LiveNets => {
                if let Some((n, last)) = live_runs[i] {
                    let (from, to) = (bar(e), (next_track_bar(last) - 1).max(bar(e)));
                    line(&format!(
                        "# gap: {} (track live deck, {n} net load{}, a live-coded program the tracklist does not name): the set starts at bar {first_bar} of the recording",
                        bars_word(to - from + 1),
                        if n == 1 { "" } else { "s" }
                    ));
                }
            }
            What::Transition => {
                if stray[i] {
                    line(&format!("# performed: {}", e.label));
                }
            }
        }
    }
    body.push_str("}\n");
    if restricted {
        body.push_str("use restricted\n");
    }

    let rights = Rights {
        license: License::Cc0,
        visibility: if restricted { Visibility::Private } else { Visibility::Public },
        author: opts.author.map(str::to_string),
        forked_from: None,
        credits: Vec::new(),
    };
    let name = if rec.name.trim().is_empty() { "set".to_string() } else { rec.name.clone() };
    let text = Object { name, text: body, rights, links: Default::default() }.to_file();
    let program_cid = program_cid_of(&text)?;

    let mut take = JsonObject::new();
    take.set("format", JsonValue::int(1));
    take.set("type", JsonValue::str(TAKE_TYPE));
    take.set("program", JsonValue::str(program_cid.clone()));
    take.set("recording", JsonValue::str(rec.cid()));
    if let Some(a) = opts.author {
        take.set("author", JsonValue::str(a));
    }
    take.set("tracklist", JsonValue::Array(rec.tracklist.iter().map(Entry::to_json).collect()));
    take.set("arrangement", rec.arrangement.clone());
    // only when there are some, so a take without them is the take it always was
    if !generated.is_empty() {
        take.set("tracks", djlive::set::bundle(&generated));
    }
    // what it was played with, as the recording has it: the controls that made its moves and the
    // input strips (there is no controller map in a recording)
    if let Some(a) = &arr {
        let controls: BTreeSet<&str> = a.events.iter().map(|e| e.via.as_str()).filter(|v| !v.is_empty()).collect();
        if !controls.is_empty() {
            take.set("controls", JsonValue::Array(controls.into_iter().map(JsonValue::str).collect()));
        }
        if !a.start.inputs.is_empty() {
            take.set("inputs", JsonValue::Array(a.start.inputs.iter().map(|s| JsonValue::str(s.label.clone())).collect()));
        }
    }
    let take = JsonValue::Object(take);
    let take_cid = djengine::cid::share_cid(&take);
    Ok(SetProgram { text, program_cid, take, take_cid, tracks: generated })
}

/// Every whole CID a `play` names is taken as it is: the canonical text pins tracks by CID, so it
/// is the same without their texts.
struct AsWritten;

impl Snippets for AsWritten {
    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        Ok((query.to_string(), String::new()))
    }
}

/// The program CID of a set's text, read without its tracks: refused when it does not parse.
fn program_cid_of(text: &str) -> Result<String, String> {
    let parsed = djlive::parse::parse_with(text, &Env { snippets: Some(&AsWritten), ..Env::none() });
    if let Some(d) = parsed.diagnostics.iter().find(|d| d.is_error()) {
        let (line, col, _) = d.position(text);
        return Err(format!("set-program: the program does not read ({line}:{col}: {})", d.message));
    }
    Ok(djlive::program_cid(&djlive::print::canonical(&parsed.program)))
}

/// A file name from a name.
fn file_stem(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '-' }) {
        if c != '-' || !s.ends_with('-') {
            s.push(c);
        }
    }
    let s = s.trim_matches('-');
    if s.is_empty() {
        "set".into()
    } else {
        s.to_string()
    }
}

/// The programs a CLI run looks a CID up in: what the caller hands in, then the saved programs.
struct Here<'a> {
    first: Option<&'a dyn Snippets>,
    store: crate::live_store::LiveStore,
}

impl Snippets for Here<'_> {
    fn label(&self, cid: &str) -> Option<String> {
        self.first.and_then(|s| s.label(cid)).or_else(|| Snippets::label(&self.store, cid))
    }

    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        match self.first.map(|s| s.resolve(query)) {
            Some(Ok(hit)) => Ok(hit),
            _ => Snippets::resolve(&self.store, query),
        }
    }
}

/// `set-program <recording dir | set.json> [--out DIR]`: writes `<name>.beatscript`, `<take cid>.take.json`
/// and, for a generator track it plays, `tracks/<label>.beatscript` (in DIR, default here), and answers the
/// CIDs and the paths as JSON. A CID is looked up in `programs`, then in the saved programs
/// (`$DJ_LIVE_DIR`).
pub fn cli(args: &[String], programs: Option<&dyn Snippets>) -> Result<String, String> {
    cli_with(args, &Here { first: programs, store: crate::live_store::LiveStore::open_default() })
}

/// [`cli`] with the programs given.
pub fn cli_with(args: &[String], programs: &dyn Snippets) -> Result<String, String> {
    const USAGE: &str = "set-program <recording dir | set.json> [--out DIR]";
    let (mut src, mut out): (Option<PathBuf>, PathBuf) = (None, PathBuf::from("."));
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" | "-o" => out = PathBuf::from(it.next().ok_or("--out needs a directory")?),
            a if a.starts_with("--") => return Err(format!("unknown option {a}: {USAGE}")),
            a if src.is_none() => src = Some(PathBuf::from(a)),
            a => return Err(format!("one recording at a time ({a}?): {USAGE}")),
        }
    }
    let src = src.ok_or(USAGE)?;
    let file = if src.is_dir() { src.join("set.json") } else { src.clone() };
    let json = read_json(&file).ok_or_else(|| format!("{}: not a recorded set's set.json", file.display()))?;
    let rec = RecordedSet::from_json(&json)?;
    let author = recording_author(&json);
    let sp = set_program(&rec, &Options { author: author.as_deref(), programs: Some(programs) })?;
    let btw = out.join(format!("{}.beatscript", file_stem(&rec.name)));
    let take = out.join(format!("{}.take.json", sp.take_cid));
    write(&btw, sp.text.as_bytes())?;
    write(&take, pretty_sorted(&sp.take).as_bytes())?;
    let mut tracks = Vec::new();
    for (cid, text) in &sp.tracks {
        // named by the call it is (`seed(techno, 7)`)
        let name = text.trim().strip_prefix("arrange from ").unwrap_or(cid).to_string();
        let path = out.join("tracks").join(format!("{}.beatscript", file_stem(&name)));
        write(&path, Object { name, text: text.clone(), rights: Rights::default(), links: Default::default() }.to_file().as_bytes())?;
        tracks.push(JsonValue::str(path.display().to_string()));
    }
    let mut o = JsonObject::from_pairs([
        ("program_cid", JsonValue::str(sp.program_cid.clone())),
        ("take_cid", JsonValue::str(sp.take_cid.clone())),
        ("program", JsonValue::str(beatscript.display().to_string())),
        ("take", JsonValue::str(take.display().to_string())),
    ]);
    if !tracks.is_empty() {
        o.set("tracks", JsonValue::Array(tracks));
    }
    Ok(JsonValue::Object(o).canonical_js())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
