//! `same_as(a, b)`: are two programs the same song? Answered from what they compile to, not from their text:
//! refactoring with `let`, `def` or `repeat` changes the program's CID (of its canonical text) and leaves its
//! nets alone, so equal net CIDs plus equal song-level settings are a proof, not a heuristic.
//!
//! Two proofs are tried, and `basis` says which one held:
//! - `sections`: both programs are arranged (or both are not) and every net matches, section by section
//!   (`diffs`, `only_in_a`, `only_in_b` list the ones that do not; with one arranged and one not,
//!   `sections_compared` is false and the lists are empty).
//!   Sections are matched by where they start and their deck; a section that is only renamed is reported in
//!   `renamed`, and is not a difference by itself.
//! - `song_nets`: the longhand, one net per deck for the whole song (`Compiled::song_nets`). This is how an
//!   arranged song and a section-less program that writes it out bar by bar are the same song, and how two
//!   songs that cut the same bars into different sections are.
//!
//! The song-level settings always count: `tempo`, `key`, `swing`, `song_bars`, `ends`, `end_bar`, `loops`.
//! Control lines (filters, moves, `at bar` calls) are not nets and are not compared.
//!
//! When they are not the same song, `diff` says what changed in music: the song's settings, the arrangement
//! (sections inserted, removed, moved, repeated, renamed) and, part by part, a transform (transposed, rotated,
//! reversed, ...) or the notes that changed (diff.rs). It is null when they are the same.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};
use djlive::compile::{check_with, Compiled, MAX_BARS};

use crate::report::{diagnostics_json, int, obj, opt_num, opt_str};
use crate::snapshot::web_env;
use crate::utf16::Utf16Index;

/// One net of a program: where it plays and its CID.
#[derive(Clone)]
struct Net {
    section: Option<String>,
    cid: String,
}

/// The nets by (start bar, deck): a song's sections, or a program's own decks at bar 0.
fn nets_of(c: &Compiled) -> BTreeMap<(u32, String), Net> {
    let mut m = BTreeMap::new();
    if c.arranged {
        for seg in &c.segments {
            for (d, ir) in &seg.decks {
                m.insert(
                    (seg.start, d.name().to_string()),
                    Net {
                        section: Some(seg.name.clone()),
                        cid: ir.net_cid(),
                    },
                );
            }
        }
    } else {
        for (d, ir) in &c.decks {
            m.insert(
                (0, d.name().to_string()),
                Net {
                    section: None,
                    cid: ir.net_cid(),
                },
            );
        }
    }
    m
}

/// The longhand by deck; None where there is none (a part that changes instrument between sections) or it is
/// chained nets with no one CID.
fn longhand(c: &Compiled) -> Option<BTreeMap<String, String>> {
    let nets = c.song_nets().ok()?;
    if nets.values().any(|ir| ir.period as u64 > MAX_BARS) {
        return None;
    }
    Some(
        nets.into_iter()
            .map(|(d, ir)| (d.name().to_string(), ir.net_cid()))
            .collect(),
    )
}

fn length_bars(c: &Compiled) -> Option<u32> {
    if c.arranged && !c.flat_clock {
        c.song_bars()
    } else {
        c.end
    }
}

/// The song-level settings, as JSON values, in a fixed order.
fn song_fields(c: &Compiled) -> Vec<(&'static str, JsonValue)> {
    let bars = length_bars(c);
    vec![
        ("tempo", opt_num(c.tempo)),
        ("key", opt_str(c.key.map(|k| k.text()))),
        ("swing", opt_num(c.swing)),
        (
            "song_bars",
            bars.map_or(JsonValue::Null, |b| JsonValue::int(i64::from(b))),
        ),
        ("ends", JsonValue::Bool(c.end.is_some())),
        (
            "end_bar",
            c.end
                .map_or(JsonValue::Null, |e| JsonValue::int(i64::from(e))),
        ),
        ("loops", JsonValue::Bool(c.looped)),
    ]
}

fn net_json(start: u32, deck: &str, n: &Net) -> JsonValue {
    obj([
        ("section", opt_str(n.section.clone())),
        ("deck", JsonValue::str(deck)),
        ("start_bar", int(start as usize)),
        ("cid", JsonValue::str(n.cid.clone())),
    ])
}

fn deck_map_json(m: &Option<BTreeMap<String, String>>) -> JsonValue {
    match m {
        Some(m) => JsonValue::Object(JsonObject::from_pairs(
            m.iter()
                .map(|(k, v)| (k.as_str(), JsonValue::str(v.clone())))
                .collect::<Vec<_>>(),
        )),
        None => JsonValue::Null,
    }
}

/// The comparison as a value. `a` and `b` are program texts. When either has errors the answer is `ok: false`
/// with each side's diagnostics, and `same` is null: a program that does not compile is no song.
pub fn same_as_report(a: &str, b: &str) -> JsonValue {
    let env = web_env().env();
    let (ca, cb) = (check_with(a, &env), check_with(b, &env));
    let (Some(x), Some(y)) = (ca.compiled.as_ref(), cb.compiled.as_ref()) else {
        let side = |chk: &djlive::compile::Check, text: &str| {
            let ix = Utf16Index::new(text);
            obj([
                ("ok", JsonValue::Bool(chk.compiled.is_some())),
                ("diagnostics", diagnostics_json(&chk.diagnostics, text, &ix)),
            ])
        };
        return obj([
            ("ok", JsonValue::Bool(false)),
            ("same", JsonValue::Null),
            ("text_same", JsonValue::Null),
            (
                "error",
                JsonValue::str(
                    "a program has errors, so there is no song to compare: `check` says where",
                ),
            ),
            ("a", side(&ca, a)),
            ("b", side(&cb, b)),
        ]);
    };

    // section by section, matched on (start bar, deck): only when both are songs with sections, or neither is
    // (an arranged song against its longhand has no sections to match; the longhand proof covers it)
    let sections_compared = x.arranged == y.arranged;
    let (na, nb) = if sections_compared {
        (nets_of(x), nets_of(y))
    } else {
        (BTreeMap::new(), BTreeMap::new())
    };
    let mut diffs = Vec::new();
    let mut renamed: Vec<JsonValue> = Vec::new();
    let mut seen_rename = std::collections::BTreeSet::new();
    let (mut only_a, mut only_b) = (Vec::new(), Vec::new());
    for ((start, deck), n) in &na {
        match nb.get(&(*start, deck.clone())) {
            None => only_a.push(net_json(*start, deck, n)),
            Some(m) => {
                if n.section != m.section
                    && seen_rename.insert((*start, n.section.clone(), m.section.clone()))
                {
                    renamed.push(obj([
                        ("start_bar", int(*start as usize)),
                        ("a", opt_str(n.section.clone())),
                        ("b", opt_str(m.section.clone())),
                    ]));
                }
                if n.cid != m.cid {
                    diffs.push(obj([
                        (
                            "section",
                            opt_str(n.section.clone().or_else(|| m.section.clone())),
                        ),
                        ("b_section", opt_str(m.section.clone())),
                        ("deck", JsonValue::str(deck.clone())),
                        ("start_bar", int(*start as usize)),
                        ("a_cid", JsonValue::str(n.cid.clone())),
                        ("b_cid", JsonValue::str(m.cid.clone())),
                    ]));
                }
            }
        }
    }
    for ((start, deck), m) in &nb {
        if !na.contains_key(&(*start, deck.clone())) {
            only_b.push(net_json(*start, deck, m));
        }
    }
    let sections_same =
        sections_compared && diffs.is_empty() && only_a.is_empty() && only_b.is_empty();

    // the longhand, deck by deck
    let (la, lb) = (longhand(x), longhand(y));
    let song_nets_same = match (&la, &lb) {
        (Some(p), Some(q)) => Some(p == q),
        _ => None,
    };

    // the song-level settings
    let (fa, fb) = (song_fields(x), song_fields(y));
    let mut song = JsonObject::new();
    for ((k, va), (_, vb)) in fa.iter().zip(fb.iter()) {
        if va.js_stringified() != vb.js_stringified() {
            song.set(*k, obj([("a", va.clone()), ("b", vb.clone())]));
        }
    }
    let settings_same = song.is_empty();

    let basis = if sections_same {
        "sections"
    } else if song_nets_same == Some(true) {
        "song_nets"
    } else {
        "none"
    };
    let same = settings_same && basis != "none";
    let why = if same {
        if basis == "sections" {
            "the same song: every net and every song-level setting is the same".to_string()
        } else {
            "the same song: the sections are cut differently, but the whole song written out longhand has the same nets, and every song-level setting is the same".to_string()
        }
    } else {
        let mut parts = Vec::new();
        if basis == "none" {
            let names: std::collections::BTreeSet<String> = diffs
                .iter()
                .chain(only_a.iter())
                .chain(only_b.iter())
                .filter_map(|d| {
                    d.get("section")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                })
                .collect();
            parts.push(if !sections_compared {
                "one program has sections and the other does not, and the whole song written out longhand has different nets".to_string()
            } else if names.is_empty() { "the nets differ".to_string() } else { format!("the nets differ in {}", names.into_iter().collect::<Vec<_>>().join(", ")) });
        }
        if !settings_same {
            parts.push(format!(
                "the song-level settings differ ({})",
                song.keys().collect::<Vec<_>>().join(", ")
            ));
        }
        format!("not the same song: {}", parts.join("; "))
    };

    // what changed, in music (diff.rs): only when they are not the same song
    let diff = if same {
        JsonValue::Null
    } else {
        crate::diff::diff(x, y, &song)
    };
    obj([
        ("ok", JsonValue::Bool(true)),
        ("same", JsonValue::Bool(same)),
        ("text_same", JsonValue::Bool(x.cid == y.cid)),
        ("basis", JsonValue::str(basis)),
        ("why", JsonValue::str(why)),
        ("a_cid", JsonValue::str(x.cid.clone())),
        ("b_cid", JsonValue::str(y.cid.clone())),
        ("sections_compared", JsonValue::Bool(sections_compared)),
        ("sections_same", JsonValue::Bool(sections_same)),
        (
            "song_nets_same",
            song_nets_same.map_or(JsonValue::Null, JsonValue::Bool),
        ),
        ("diffs", JsonValue::Array(diffs)),
        ("only_in_a", JsonValue::Array(only_a)),
        ("only_in_b", JsonValue::Array(only_b)),
        ("renamed", JsonValue::Array(renamed)),
        ("song", JsonValue::Object(song)),
        (
            "song_nets",
            obj([("a", deck_map_json(&la)), ("b", deck_map_json(&lb))]),
        ),
        ("diff", diff),
    ])
}

/// `same_as(a, b)` as the JSON string `beatscript same-as` prints.
pub fn same_as_json(a: &str, b: &str) -> String {
    same_as_report(a, b).js_stringified()
}

/// `beatscript same-as` reads both programs as one JSON object on stdin, `{"a": text, "b": text}` (the server's
/// subprocess protocol has one stdin). Bad input is `ok: false` with `error`.
pub fn same_as_json_pair(input: &str) -> String {
    let bad = |e: &str| {
        obj([
            ("ok", JsonValue::Bool(false)),
            ("same", JsonValue::Null),
            ("text_same", JsonValue::Null),
            ("error", JsonValue::str(e)),
        ])
        .js_stringified()
    };
    let v = match JsonValue::parse(input) {
        Ok(v) => v,
        Err(_) => return bad("same-as reads {\"a\": text, \"b\": text} as JSON on stdin"),
    };
    match (
        v.get("a").and_then(|s| s.as_str()),
        v.get("b").and_then(|s| s.as_str()),
    ) {
        (Some(a), Some(b)) => same_as_json(a, b),
        _ => bad("same-as needs both \"a\" and \"b\", each a program's text"),
    }
}
