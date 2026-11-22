//! Stations as JSON (docs/btw/radio.md): the one answer the MCP `station` tool, `POST /api/station` and the page wasm's
//! `btw_station` give, so they cannot disagree. A station's walk, tracks and segues are djlive's (`djlive::station`); this
//! adds the request (retune, locks, neighbours), the checks with the seeded generator installed, and each program's
//! `expanded_cid` and resolution.
//!
//! Request: `{"station": "<program>", "n": 0, "count": 1, "segue": true, "check": false, "retune": {"tempo": "+1"},
//! "locks": ["key"], "neighbours": false}`. Answer: `{"ok", "version", "station": {...}, "n", "tracks": [...],
//! "segues": [...], "neighbours": [...], "retuned", "locks", "diagnostics"}`, or `{"ok": false, "error", "diagnostics"}`.

use djengine::json::{JsonObject, JsonValue};
use djlive::station::{Station, KNOBS};

/// At most this many tracks an answer (each is checked when asked).
pub const MAX_COUNT: u64 = 16;

/// What a request asks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StationAsk {
    /// The station program's text.
    pub station: String,
    pub n: u64,
    pub count: u64,
    pub segue: bool,
    pub check: bool,
    /// Knobs set before anything else, in order: (knob, value or `+1`/`-1`).
    pub retune: Vec<(String, String)>,
    pub locks: Vec<String>,
    pub neighbours: bool,
}

impl StationAsk {
    /// The request's JSON read; unknown fields are refused, as the MCP tools refuse them.
    pub fn from_json(v: &JsonValue) -> Result<StationAsk, String> {
        let JsonValue::Object(o) = v else { return Err("a station request is a JSON object".into()) };
        for (k, _) in o.iter() {
            if !["station", "n", "count", "segue", "check", "retune", "locks", "neighbours"].contains(&k) {
                return Err(format!("`{k}` is no field of a station request: station, n, count, segue, check, retune, locks, neighbours"));
            }
        }
        let station = match o.get("station") {
            Some(JsonValue::String(s)) if !s.trim().is_empty() => s.clone(),
            _ => return Err("`station` is the station program's text".into()),
        };
        let whole = |k: &str, d: u64| -> Result<u64, String> {
            match o.get(k) {
                None | Some(JsonValue::Null) => Ok(d),
                Some(JsonValue::Number(x)) if *x >= 0.0 && x.fract() == 0.0 && *x <= 9.0e15 => Ok(*x as u64),
                Some(JsonValue::String(s)) => s.trim().parse::<u64>().map_err(|_| format!("`{k}` is a whole number of 0 or more")),
                _ => Err(format!("`{k}` is a whole number of 0 or more")),
            }
        };
        let flag = |k: &str, d: bool| -> Result<bool, String> {
            match o.get(k) {
                None | Some(JsonValue::Null) => Ok(d),
                Some(JsonValue::Bool(b)) => Ok(*b),
                _ => Err(format!("`{k}` is true or false")),
            }
        };
        let count = whole("count", 1)?;
        if count == 0 || count > MAX_COUNT {
            return Err(format!("`count` is 1 to {MAX_COUNT}"));
        }
        let mut retune = Vec::new();
        match o.get("retune") {
            None | Some(JsonValue::Null) => {}
            Some(JsonValue::Object(r)) => {
                for (k, v) in r.iter() {
                    let v = match v {
                        JsonValue::String(s) => s.clone(),
                        JsonValue::Number(x) => {
                            if x.fract() == 0.0 {
                                format!("{}", *x as i64)
                            } else {
                                format!("{x}")
                            }
                        }
                        JsonValue::Array(a) => a.iter().filter_map(JsonValue::as_str).collect::<Vec<_>>().join(" "),
                        _ => return Err(format!("`retune.{k}` is a value, or \"+1\" / \"-1\" for a notch")),
                    };
                    retune.push((k.to_string(), v));
                }
            }
            _ => return Err("`retune` is an object of knobs: {\"tempo\": \"+1\"}".into()),
        }
        let mut locks = Vec::new();
        match o.get("locks") {
            None | Some(JsonValue::Null) => {}
            Some(JsonValue::Array(a)) => {
                for l in a {
                    let Some(l) = l.as_str() else { return Err("`locks` is a list of knob names".into()) };
                    if !KNOBS.contains(&l) {
                        return Err(format!("`{l}` is no knob: {}", KNOBS.join(" ")));
                    }
                    locks.push(l.to_string());
                }
            }
            _ => return Err("`locks` is a list of knob names".into()),
        }
        Ok(StationAsk { station, n: whole("n", 0)?, count, segue: flag("segue", true)?, check: flag("check", false)?, retune, locks, neighbours: flag("neighbours", false)? })
    }
}

fn err(e: impl Into<String>, diagnostics: Vec<JsonValue>) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs([("ok", JsonValue::Bool(false)), ("error", JsonValue::str(e.into())), ("diagnostics", JsonValue::Array(diagnostics))]))
}

/// A program checked: `{ok, expanded_cid, bars, seconds, errors}` (and `refs`, `resolution`).
pub fn checked(text: &str) -> JsonValue {
    let rep = crate::report::check_report(text);
    let mut o = JsonObject::new();
    o.set("ok", rep.get("ok").cloned().unwrap_or(JsonValue::Bool(false)));
    o.set("expanded_cid", rep.get("expanded_cid").cloned().unwrap_or(JsonValue::Null));
    o.set("bars", rep.get("song_bars").cloned().unwrap_or(JsonValue::Null));
    o.set("seconds", rep.get("seconds").cloned().unwrap_or(JsonValue::Null));
    let errors: Vec<JsonValue> = rep
        .get("diagnostics")
        .and_then(JsonValue::as_array)
        .map(|ds| ds.iter().filter(|d| d.get("severity").and_then(JsonValue::as_str) == Some("error")).filter_map(|d| d.get("message").cloned()).collect())
        .unwrap_or_default();
    o.set("errors", JsonValue::Array(errors));
    JsonValue::Object(o)
}

/// The station a program is, with its tempo and key filled in from the generator when it does not say them.
pub fn station_of(text: &str) -> Result<(Station, Station), (String, Vec<JsonValue>)> {
    crate::generate::install();
    let s = djlive::station::read(text).map_err(|ds| {
        let first = ds.iter().find(|d| d.is_error()).map_or_else(|| "the station does not read".to_string(), |d| d.message.clone());
        let ix = crate::utf16::Utf16Index::new(text);
        (first, ds.iter().map(|d| crate::report::diagnostic_json(d, text, &ix)).collect())
    })?;
    let filled = s.with_defaults().map_err(|d| (d.message.clone(), vec![crate::report::diagnostic_json(&d, text, &crate::utf16::Utf16Index::new(text))]))?;
    Ok((s, filled))
}

/// The answer to a request.
pub fn station_answer(ask: &StationAsk) -> JsonValue {
    let (mut station, _) = match station_of(&ask.station) {
        Ok(s) => s,
        Err((e, ds)) => return err(e, ds),
    };
    let mut retuned = Vec::new();
    for (k, v) in &ask.retune {
        if ask.locks.iter().any(|l| l == k) {
            return err(format!("`{k}` is locked: unlock it to turn it"), Vec::new());
        }
        // (a notch of a tempo or key the station does not say turns the from call's own)
        let base = if matches!(k.as_str(), "tempo" | "key") && v.trim_start_matches(['+', '-']).chars().all(|c| c.is_ascii_digit()) && v.starts_with(['+', '-']) {
            match station.with_defaults() {
                Ok(f) => {
                    let mut b = station.clone();
                    if k == "tempo" {
                        b.tempo = f.tempo;
                    } else {
                        b.key = f.key;
                    }
                    b
                }
                Err(d) => return err(d.message, Vec::new()),
            }
        } else {
            station.clone()
        };
        match base.retune(k, v) {
            Ok(s) => station = s,
            Err(e) => return err(e, Vec::new()),
        }
        retuned.push(JsonValue::str(k.clone()));
    }
    let filled = match station.with_defaults() {
        Ok(f) => f,
        Err(d) => return err(d.message, Vec::new()),
    };
    let tracks = match filled.tracks(ask.n, ask.count) {
        Ok(t) => t,
        Err(e) => return err(e, Vec::new()),
    };
    // the station as the listener tuned it: its own text, the defaults it does not say left unsaid
    let mut st = station.to_json();
    if let (JsonValue::Object(o), JsonValue::Object(f)) = (&mut st, filled.to_json()) {
        o.set("tempo", f.get("tempo").cloned().unwrap_or(JsonValue::Null));
        o.set("key", f.get("key").cloned().unwrap_or(JsonValue::Null));
    }
    let mut ts = Vec::new();
    for t in &tracks {
        let mut j = t.to_json();
        if let JsonValue::Object(o) = &mut j {
            // the track says which station it is of: the tuned one
            o.set("station", JsonValue::str(station.cid()));
            if ask.check {
                o.set("check", checked(&t.text));
            }
        }
        ts.push(j);
    }
    let mut segues = Vec::new();
    if ask.segue {
        for t in &tracks {
            match filled.segue(t.n) {
                Ok(sg) => {
                    let mut j = sg.to_json();
                    if ask.check {
                        if let JsonValue::Object(o) = &mut j {
                            o.set("check", checked(&sg.text));
                        }
                    }
                    segues.push(j);
                }
                Err(e) => return err(e, Vec::new()),
            }
        }
    }
    let mut ns = Vec::new();
    if ask.neighbours {
        for (knob, notch, s) in filled.neighbours(&ask.locks) {
            let mut o = JsonObject::new();
            o.set("knob", JsonValue::str(knob.clone()));
            o.set("notch", JsonValue::str(notch));
            let j = s.to_json();
            o.set("value", j.get(&knob).cloned().unwrap_or(JsonValue::Null));
            o.set("text", JsonValue::str(s.text()));
            o.set("cid", JsonValue::str(s.cid()));
            if let Ok(t) = s.track(ask.n) {
                o.set("track", t.to_json());
            }
            ns.push(JsonValue::Object(o));
        }
    }
    let mut o = JsonObject::new();
    o.set("ok", JsonValue::Bool(true));
    o.set("version", JsonValue::str(format!("stations/{}", djlive::station::VERSION)));
    o.set("station", st);
    o.set("n", JsonValue::int(ask.n as i64));
    o.set("tracks", JsonValue::Array(ts));
    o.set("segues", JsonValue::Array(segues));
    if ask.neighbours {
        o.set("neighbours", JsonValue::Array(ns));
    }
    o.set("retuned", JsonValue::Array(retuned));
    o.set("locks", JsonValue::Array(ask.locks.iter().map(|l| JsonValue::str(l.clone())).collect()));
    o.set("diagnostics", JsonValue::Array(Vec::new()));
    JsonValue::Object(o)
}

/// `station(json)` -> the answer as a JSON string (the wasm's `btw_station`, `btwd station`).
pub fn station_json(input: &str) -> String {
    let v = match JsonValue::parse(if input.trim().is_empty() { "{}" } else { input }) {
        Ok(v) => v,
        Err(_) => return err("the request is not JSON", Vec::new()).js_stringified(),
    };
    match StationAsk::from_json(&v) {
        Ok(ask) => station_answer(&ask).js_stringified(),
        Err(e) => err(e, Vec::new()).js_stringified(),
    }
}
