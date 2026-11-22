//! Port of `ComposeOptions.swift`: the opt-in composition techniques (`compose` overrides
//! beats doesn't have, plus a few numeric overrides of the genre table), as `generate`
//! takes them: validated, normalised, and stored in a `TrackSpec`'s `extra` so a share /
//! session replays them and its CID reflects them. Options that are off are left out, so
//! "all off" is the plain spec (same CID as before). COMPOSE-OPTIONS.md describes each one.

use alloc::format;
use alloc::string::String;

use crate::json::{JsonObject, JsonValue};
use crate::spec::TrackSpec;

/// The keys `generate` accepts.
pub const KEYS: [&str; 13] = [
    "hook",
    "hookBars",
    "chordSnap",
    "chordSnapArp",
    "layerBudget",
    "melodyDensity",
    "bassDensity",
    "ghostNotes",
    "syncopation",
    "humanize",
    "bassFollow",
    "keyLock",
    "vox",
];

/// Where each option lives in `TrackSpec.extra` (beats' own key when it has one).
fn stored_key(k: &str) -> &str {
    if k == "ghostNotes" {
        "ghost-notes"
    } else {
        k
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    pub description: String,
}

impl core::fmt::Display for Invalid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.description)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Invalid {}

/// `spec` with `options` merged into its `extra` (validated; off values removed).
pub fn apply(options: &JsonObject, spec: &TrackSpec) -> Result<TrackSpec, Invalid> {
    let mut spec = spec.clone();
    for (k, v) in options.iter() {
        if !KEYS.contains(&k) {
            return Err(Invalid {
                description: format!("unknown option {k} (one of {})", KEYS.join(", ")),
            });
        }
        let stored = stored_key(k);
        match normalised(k, v)? {
            Some(n) => spec.extra.set(stored, n),
            None => {
                spec.extra.remove(stored);
            }
        }
    }
    if spec.extra.get("hook").is_none() {
        spec.extra.remove("hookBars");
    }
    if spec.extra.get("chordSnap").is_none() {
        spec.extra.remove("chordSnapArp");
    }
    Ok(spec)
}

/// The options a spec carries, under their `generate` names (for deck state).
pub fn describe(spec: &TrackSpec) -> JsonObject {
    let mut o = JsonObject::new();
    for k in KEYS {
        if let Some(v) = spec.extra.get(stored_key(k)) {
            o.set(k, v.clone());
        }
    }
    o
}

/// None = off (not stored).
fn normalised(k: &str, v: &JsonValue) -> Result<Option<JsonValue>, Invalid> {
    let bad = |want: &str| Invalid {
        description: format!("{k}: expected {want}"),
    };
    let unit = || match v.as_f64() {
        Some(d) if (0.0..=1.0).contains(&d) => Ok(Some(JsonValue::Number(d))),
        _ => Err(bad("a number 0…1")),
    };
    match k {
        "hook" | "chordSnapArp" | "bassFollow" | "keyLock" => {
            let b = v.as_bool().ok_or_else(|| bad("true or false"))?;
            Ok(b.then_some(JsonValue::Bool(true)))
        }
        "hookBars" => match v.as_f64() {
            Some(d) if d == 1.0 || d == 2.0 => Ok(Some(JsonValue::int(d as i64))),
            _ => Err(bad("1 or 2")),
        },
        "chordSnap" => {
            if let Some(b) = v.as_bool() {
                return Ok(b.then(|| JsonValue::str("strong")));
            }
            match v.as_str() {
                Some("off") => Ok(None),
                Some(s @ ("strong" | "all")) => Ok(Some(JsonValue::str(s))),
                _ => Err(bad("off | strong | all")),
            }
        }
        "layerBudget" => {
            if v.as_str() == Some("auto") {
                return Ok(Some(JsonValue::str("auto")));
            }
            if v.as_str() == Some("off") || v.as_bool() == Some(false) {
                return Ok(None);
            }
            match v.as_f64() {
                Some(d) if d == crate::json::floor(d) && (0.0..=8.0).contains(&d) => {
                    Ok((d != 0.0).then(|| JsonValue::int(d as i64)))
                }
                _ => Err(bad("\"auto\", \"off\" or an integer 1…8")),
            }
        }
        "melodyDensity" | "bassDensity" => match v.as_f64() {
            Some(d) if d > 0.0 && d <= 1.0 => Ok(Some(JsonValue::Number(d))),
            _ => Err(bad("a number in (0, 1]")),
        },
        "ghostNotes" | "syncopation" => unit(),
        // the formant voice's sung part (djrust gen::vox; docs/VOICE.md)
        "vox" => {
            if v.as_bool() == Some(false) {
                return Ok(None);
            }
            if v.as_bool() == Some(true) {
                return Ok(Some(JsonValue::str("auto")));
            }
            match v.as_str() {
                Some("off") => Ok(None),
                Some(s @ ("choir" | "robot" | "auto")) => Ok(Some(JsonValue::str(s))),
                _ => Err(bad("off | choir | robot | auto (or true / false)")),
            }
        }
        "humanize" => match v.as_f64() {
            Some(d) if (0.0..=100.0).contains(&d) => Ok(Some(JsonValue::Number(d))),
            _ => Err(bad("a number 0…100")),
        },
        _ => Err(Invalid {
            description: format!("unknown option {k}"),
        }),
    }
}
