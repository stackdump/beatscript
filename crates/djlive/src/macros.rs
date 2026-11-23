//! `macro ID on DECK over N bars`: one entry of the DJ macro catalogue fired by name, for a length, on a deck (docs/beatscript/ext-control.md,
//! "macro"). An engine call, so a line of the control deck, and (as `move` is) a line with no head:
//!
//! ```text
//! control: macro riser on A over 8 bars
//! at bar 24: control: macro cathedral on B over 2 bars
//! macro@1 breakdown on A over 4 bars
//! ```
//!
//! **Versioned.** The catalogue is the engine's (`djengine::fx::catalogue::ENTRIES`, the 39 pads of the apps' Effects window), and
//! it moves; a program must not. `macro@1` is [`V1`], a copy of the catalogue's ids as of the engine this repo pins (each with its
//! category, its family, whether it is a transition, whether it needs a net deck, and its length), compiled in here and never
//! edited: a stored program's `macro riser` means what it meant when it was stored. A later catalogue is `macro@2`, a table of its
//! own beside the first ([`VERSIONS`]); a bare `macro` is `macro@1` for good, and the canonical text writes the version only when it
//! is not 1. `tests/macros.rs` pins the table's CID and holds it to the engine's catalogue.
//!
//! **Lowering.** `macro breakdown on A over 4 bars` is `effects {action: fire, target: A, id: breakdown, bars: 4}`: the registry's
//! `effects fire`, which is `Engine::engage(deck, id, Engagement::Bars(n))` (`djhost::fx_view::fire`), so the booth, the Effects
//! window and the program fire the same thing. How long each entry runs is its [`Length`].

use djengine::json::{JsonObject, JsonValue};

use crate::compile::Timing;
use crate::rat::Rat;

/// How long an entry runs, and what `over N bars` does to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    /// Whole bars: `over N bars` says how many (1 to [`MAX_BARS`]); with none, this many (the Effects window's own length:
    /// 1 bar, 2 for a sweep).
    Bars(u32),
    /// A one-shot whose own length is a sweep of whole bars (the riser): `over N bars` is how many bars it rises over; with none,
    /// its own (4).
    Sweep(u32),
    /// A one-shot that runs while it is held (tape stop, spin back, reverb freeze): held `over N bars` (with none, this many), then
    /// let go (`effects release`).
    Held(u32),
    /// A one-shot that runs its own length (cut, double drop): it takes no `over`.
    Own,
}

impl Length {
    /// The bars it runs (or is held, or sweeps) when the line says no `over`; None for [`Length::Own`].
    pub fn default_bars(self) -> Option<u32> {
        match self {
            Length::Bars(n) | Length::Sweep(n) | Length::Held(n) => Some(n),
            Length::Own => None,
        }
    }

    /// The word the table says it with: `bars`, `sweep`, `held`, `own`.
    pub fn kind(self) -> &'static str {
        match self {
            Length::Bars(_) => "bars",
            Length::Sweep(_) => "sweep",
            Length::Held(_) => "held",
            Length::Own => "own",
        }
    }
}

/// One entry of a version's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroEntry {
    /// What `Engine::engage` takes: the trick's name or the macro's id.
    pub id: &'static str,
    /// The pad's label in the Effects window.
    pub label: &'static str,
    /// The panel category (`djengine::fx::catalogue::Category::id`).
    pub category: &'static str,
    /// Which engine family fires it: `trick`, `net`, `master` or `aux`.
    pub family: &'static str,
    /// A one-shot of the Transition category: no latch, and a length only when it has one ([`Length`]).
    pub transition: bool,
    /// Works a net deck's parts (a song, pan or shape macro): refused by the engine on a deck that plays no net.
    pub needs_net: bool,
    pub length: Length,
}

/// The most whole bars a macro runs: the engine's bound on `Engagement::Bars` (the Effects window offers 1 to 4).
pub const MAX_BARS: u32 = 64;

const fn m(id: &'static str, label: &'static str, category: &'static str, family: &'static str, needs_net: bool, length: Length) -> MacroEntry {
    MacroEntry { id, label, category, family, transition: matches!(category.as_bytes(), b"transition"), needs_net, length }
}

use Length::{Bars, Held, Own, Sweep};

/// `macro@1`: the catalogue as of djrust 7420204 (the workspace pin), in panel order. Never edited: a change is `macro@2`.
pub const V1: [MacroEntry; 39] = [
    // transitions: one-shots
    m("riser", "Riser", "transition", "trick", false, Sweep(4)),
    m("tape_stop", "Tape Stop", "transition", "trick", false, Held(1)),
    m("spin_back", "Spin Back", "transition", "trick", false, Held(1)),
    m("verb_freeze", "Reverb Freeze", "transition", "trick", false, Held(1)),
    m("cut", "Cut", "transition", "net", true, Own),
    m("double-drop", "Double Drop", "transition", "net", true, Own),
    // filter and tone
    m("filter", "Filter", "filter-tone", "trick", false, Bars(1)),
    m("filter-res", "Res Ping", "filter-tone", "master", false, Bars(1)),
    m("build-crush", "Bit Crush", "filter-tone", "master", false, Bars(1)),
    m("phaser-drone", "Phaser Drone", "filter-tone", "master", false, Bars(1)),
    // space
    m("delay", "Delay", "space", "trick", false, Bars(1)),
    m("wash", "Wash", "space", "trick", false, Bars(1)),
    m("cathedral", "Cathedral", "space", "aux", false, Bars(1)),
    // rhythm
    m("stutter", "Stutter", "rhythm", "trick", false, Bars(1)),
    m("pump", "Pump", "rhythm", "trick", false, Bars(1)),
    m("beat-repeat", "Gate Chop", "rhythm", "net", true, Bars(1)),
    m("half_time", "Half Time", "rhythm", "trick", false, Bars(1)),
    m("double_time", "Double Time", "rhythm", "trick", false, Bars(1)),
    m("tempo-anchor", "Anchor", "rhythm", "net", true, Bars(1)),
    // arrange
    m("drop", "Drums Only", "arrange", "net", true, Bars(1)),
    m("breakdown", "Breakdown", "arrange", "net", true, Bars(1)),
    // pitch
    m("octave-up", "Octave Up", "pitch", "master", false, Bars(1)),
    m("octave-down", "Octave Down", "pitch", "master", false, Bars(1)),
    m("pitch-bend", "Pitch Bend", "pitch", "master", false, Bars(1)),
    // stereo and shape
    m("ping-pong", "Ping-Pong", "stereo-shape", "aux", true, Bars(1)),
    m("auto-pan", "Auto-Pan", "stereo-shape", "aux", true, Bars(1)),
    m("mono", "Mono", "stereo-shape", "aux", true, Bars(1)),
    m("hard-left", "Hard Left", "stereo-shape", "aux", true, Bars(1)),
    m("hard-right", "Hard Right", "stereo-shape", "aux", true, Bars(1)),
    m("tighten", "Tighten", "stereo-shape", "aux", true, Bars(1)),
    m("loosen", "Loosen", "stereo-shape", "aux", true, Bars(1)),
    m("pulse", "Pulse", "stereo-shape", "aux", true, Bars(1)),
    // feel
    m("feel-chill", "Chill", "feel", "master", false, Bars(1)),
    m("feel-drive", "Drive", "feel", "master", false, Bars(1)),
    m("feel-ambient", "Ambient", "feel", "master", false, Bars(1)),
    m("feel-euphoric", "Euphoric", "feel", "master", false, Bars(1)),
    // sweeps: bars, no latch (the window's sweep length is 2)
    m("feel-build", "Build Up", "sweeps", "master", false, Bars(2)),
    m("feel-wind-down", "Wind Down", "sweeps", "master", false, Bars(2)),
    m("feel-reset", "Reset", "sweeps", "master", false, Bars(2)),
];

/// Every version's table: `VERSIONS[0]` is `macro@1`. A new catalogue is a table appended here, never an edit of one.
pub const VERSIONS: &[&[MacroEntry]] = &[&V1];

/// The version a bare `macro` is: 1, for good (the canonical text leaves it out).
pub const BARE: u32 = 1;

/// The head of a macro line: `macro` (version 1) or `macro@N`. None for any other word (and for `macro@` with no number).
pub fn head_version(word: &str) -> Option<u32> {
    if word == "macro" {
        return Some(BARE);
    }
    let n = word.strip_prefix("macro@")?;
    if n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(n.parse::<u32>().unwrap_or(u32::MAX))
}

/// Is `word` the head of a macro line?
pub fn is_head(word: &str) -> bool {
    head_version(word).is_some()
}

/// The head as the canonical text writes it: `macro` for version 1, `macro@N` for another.
pub fn head_text(version: u32) -> String {
    if version == BARE {
        "macro".into()
    } else {
        format!("macro@{version}")
    }
}

/// Version `v`'s table, if this BeatScript has it.
pub fn table(v: u32) -> Option<&'static [MacroEntry]> {
    VERSIONS.get(usize::try_from(v).ok()?.checked_sub(1)?).copied()
}

/// Why an id was not found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss {
    /// No table of that version.
    NoVersion,
    /// Not in that version's table: the nearest id of it (edit distance 2 at most), and the first later version that has it.
    NotIn { near: Option<&'static str>, later: Option<u32> },
}

/// `id` in version `v` of `tables` (`tables[0]` is version 1): the entry, or why not. [`find`] is this over [`VERSIONS`]; the
/// tables are a parameter so a test can give a second version that does not exist yet.
pub fn find_in(tables: &[&'static [MacroEntry]], v: u32, id: &str) -> Result<&'static MacroEntry, Miss> {
    let idx = usize::try_from(v).ok().and_then(|v| v.checked_sub(1)).filter(|i| *i < tables.len()).ok_or(Miss::NoVersion)?;
    let t = tables[idx];
    if let Some(e) = t.iter().find(|e| e.id == id) {
        return Ok(e);
    }
    let near = crate::parse::nearest(id, t.iter().map(|e| e.id));
    let later = tables.iter().enumerate().skip(idx + 1).find(|(_, t)| t.iter().any(|e| e.id == id)).map(|(i, _)| i as u32 + 1);
    Err(Miss::NotIn { near, later })
}

/// `id` in `macro@v`.
pub fn find(v: u32, id: &str) -> Result<&'static MacroEntry, Miss> {
    find_in(VERSIONS, v, id)
}

/// The versions this BeatScript has, as the text writes them: `macro@1`, ...
pub fn version_names() -> Vec<String> {
    (1..=VERSIONS.len() as u32).map(|v| format!("macro@{v}")).collect()
}

/// How far ahead of its time a macro's call goes: a beat. The engine starts every engagement on the deck's **next** bar line (at
/// least a few milliseconds ahead), so a call made on bar 24's line would land on bar 25; made a beat before it, it lands on 24.
pub fn lead_bars() -> Rat {
    Rat::new(1, 4).unwrap_or(Rat::ZERO)
}

/// The time a macro line's call is made, for a line whose time is `t`: a bar-line time (`at bar 24`, `every 16 bars`, `every 4
/// bars from 8`, a section's first bar) a beat earlier, so the macro starts on the bar the line names. A time between bar lines is
/// kept (the macro starts on the next bar line, as a pad does), and so is a time with no beat before it: the song's first bar, a
/// line with no time word outside a section (it fires when the program is applied, in the bar before the one it lands on), and
/// `on next phrase`, which waits for its bar.
pub fn lead(t: Timing) -> Timing {
    let early = |b: u32| Rat::int(i64::from(b)).sub(lead_bars()).ok().filter(|r| !r.is_neg());
    match t {
        Timing::At(b) if b >= 1 => early(b).map_or(t, Timing::AtPos),
        Timing::Every(n) if n >= 1 => early(n).map_or(t, |from| Timing::EveryPos { step: Rat::int(i64::from(n)), from }),
        Timing::EveryFrom(n, f) if n >= 1 && f >= 1 => early(f).map_or(t, |from| Timing::EveryPos { step: Rat::int(i64::from(n)), from }),
        other => other,
    }
}

/// How long after its call (made at `called`, a line written at `written`) the engine starts the macro: a beat when the call was
/// led ([`lead`]); to the next bar line from a time between two; a whole bar from a bar line (the engine's next bar).
pub fn wait_to_bar(written: Timing, called: Timing) -> Rat {
    if written != called {
        return lead_bars();
    }
    let to_line = |p: Rat| Rat::int(p.ceil()).sub(p).ok().filter(|w| !w.is_zero()).unwrap_or(Rat::ONE);
    match called {
        Timing::AtPos(p) => to_line(p),
        Timing::EveryPos { step, from } if step.is_int() => to_line(from),
        _ => Rat::ONE,
    }
}

/// One entry as the `macros` MCP tool and the docs list it.
pub fn entry_json(e: &MacroEntry) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("id", JsonValue::str(e.id));
    o.set("label", JsonValue::str(e.label));
    o.set("category", JsonValue::str(e.category));
    o.set("family", JsonValue::str(e.family));
    o.set("transition", JsonValue::Bool(e.transition));
    o.set("needs_net", JsonValue::Bool(e.needs_net));
    o.set("length", JsonValue::str(e.length.kind()));
    o.set("default_bars", e.length.default_bars().map_or(JsonValue::Null, |n| JsonValue::int(i64::from(n))));
    o.set("takes_over", JsonValue::Bool(e.length != Length::Own));
    JsonValue::Object(o)
}

/// Version `v`'s table as JSON (`{version, head, max_bars, entries: [...]}`), or None when there is no such version.
pub fn table_json(v: u32) -> Option<JsonValue> {
    let t = table(v)?;
    let mut o = JsonObject::new();
    o.set("version", JsonValue::int(i64::from(v)));
    o.set("head", JsonValue::str(head_text(v)));
    o.set("max_bars", JsonValue::int(i64::from(MAX_BARS)));
    o.set("entries", JsonValue::Array(t.iter().map(entry_json).collect()));
    Some(JsonValue::Object(o))
}

/// The CID of version `v`'s table (its JSON, canonical): what `tests/macros.rs` pins, so an edit of a frozen table fails.
pub fn table_cid(v: u32) -> Option<String> {
    table_json(v).map(|j| djengine::cid::share_cid(&j))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_head_is_macro_or_macro_at_a_number() {
        assert_eq!(head_version("macro"), Some(1));
        assert_eq!(head_version("macro@2"), Some(2));
        assert_eq!(head_version("macro@"), None);
        assert_eq!(head_version("macros"), None);
        assert_eq!(head_version("macro@x"), None);
        assert_eq!(head_text(1), "macro");
        assert_eq!(head_text(2), "macro@2");
    }

    #[test]
    fn an_id_is_found_in_its_version_or_said_why_not() {
        assert_eq!(find(1, "riser").unwrap().length, Length::Sweep(4));
        assert_eq!(find(1, "rizer"), Err(Miss::NotIn { near: Some("riser"), later: None }));
        assert_eq!(find(7, "riser"), Err(Miss::NoVersion));
        assert_eq!(find(0, "riser"), Err(Miss::NoVersion));
        // a second catalogue (there is none yet): an id only it has, written against @1, says which version has it
        static V2: [MacroEntry; 1] = [m("vinyl-brake", "Vinyl Brake", "transition", "master", false, Own)];
        let tables: &[&'static [MacroEntry]] = &[&V1, &V2];
        assert_eq!(find_in(tables, 1, "vinyl-brake"), Err(Miss::NotIn { near: None, later: Some(2) }));
        assert_eq!(find_in(tables, 2, "vinyl-brake").unwrap().id, "vinyl-brake");
        assert!(find_in(tables, 2, "riser").is_err());
    }

    #[test]
    fn the_table_is_the_catalogue_s_shape() {
        assert_eq!(V1.iter().filter(|e| e.transition).count(), 6);
        assert_eq!(V1.iter().filter(|e| e.family == "trick").count(), 11);
        assert_eq!(V1.iter().filter(|e| e.family == "net").count(), 6);
        assert_eq!(V1.iter().filter(|e| e.family == "master").count(), 13);
        assert_eq!(V1.iter().filter(|e| e.family == "aux").count(), 9);
        for e in V1.iter() {
            assert_eq!(e.transition, matches!(e.length, Length::Sweep(_) | Length::Held(_) | Length::Own), "{}", e.id);
        }
        assert!(table_json(1).is_some() && table_json(2).is_none());
    }
}
