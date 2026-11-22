//! What `djctl render` is asked for (port of `Musicality/RenderRequest.swift`): the spec
//! (genre, seed, structure, `--cohesion`, `--options` as `generate` takes them) and the stem
//! filter (`--mute` / `--solo`: roles, mixer groups or net ids, [`RoleFilter`]). Parsed here so
//! it can be tested; the CLI only reads the arguments and prints. With no flags the spec is
//! the plain `TrackSpec::new(genre, seed, structure)` and the filter `None`, so the render is
//! the one before.
//!
//! `FileMeasure` (the other half of the PORTING.md row) reads audio files through
//! AVFoundation and is not ported, and there is no `djctl measure` yet: a Rust one would decode
//! with symphonia (`djhost::decode`) and hand the samples to the `musicality` measurements.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compose_options::{self, Invalid};
use crate::json::{JsonObject, JsonValue};
use crate::spec::TrackSpec;
use crate::wave::runner::{RoleFilter, WaveOptions};

/// `RenderRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderRequest {
    pub genre: String,
    pub seed: f64,
    /// None = a loop.
    pub structure: Option<String>,
    pub cohesion: String,
    /// `ComposeOptions`, validated by [`spec`](Self::spec).
    pub options: Option<JsonObject>,
    /// None = everything plays (an empty filter is stored as None).
    pub filter: Option<RoleFilter>,
}

impl RenderRequest {
    /// `RenderRequest.init`. Swift's defaults are `structure: "standard"`, the spec's own
    /// cohesion, no options and no filter; [`RenderRequest::plain`] is that.
    pub fn new(
        genre: &str,
        seed: f64,
        structure: Option<&str>,
        cohesion: Option<&str>,
        options: Option<JsonObject>,
        filter: Option<RoleFilter>,
    ) -> RenderRequest {
        RenderRequest {
            genre: genre.to_string(),
            seed,
            structure: structure.map(String::from),
            cohesion: cohesion
                .map(String::from)
                .unwrap_or_else(|| TrackSpec::new(genre, seed, None).cohesion),
            options,
            filter: filter.filter(|f| !f.is_empty()),
        }
    }

    /// `RenderRequest(genre:seed:)`: a "standard" song, no flags.
    pub fn plain(genre: &str, seed: f64) -> RenderRequest {
        RenderRequest::new(genre, seed, Some("standard"), None, None, None)
    }

    /// `--mute a,b` / `--solo a,b` as a filter; None when neither names anything. A list is
    /// split at commas, trimmed, and its empty entries dropped; a name given twice counts
    /// once (Swift's are sets; these keep the order they were first named in).
    ///
    /// As in Swift, `--solo` given but naming nothing (`--solo ""`) is a filter that keeps
    /// no role: only the absence of the flag leaves `solo` unset.
    pub fn filter(mute: Option<&str>, solo: Option<&str>) -> Option<RoleFilter> {
        fn list(s: Option<&str>) -> Option<Vec<String>> {
            let s = s?;
            let mut out: Vec<String> = Vec::new();
            for name in s.split(',') {
                // `trimmingCharacters(in: .whitespaces)`: spaces and tabs, not line breaks
                let name = name.trim_matches(|c: char| {
                    c == '\t'
                        || (c.is_whitespace()
                            && !matches!(
                                c,
                                '\n' | '\r'
                                    | '\u{b}'
                                    | '\u{c}'
                                    | '\u{85}'
                                    | '\u{2028}'
                                    | '\u{2029}'
                            ))
                });
                if !name.is_empty() && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
            }
            Some(out)
        }
        let f = RoleFilter {
            mute: list(mute).unwrap_or_default(),
            solo: list(solo),
        };
        if f.is_empty() {
            None
        } else {
            Some(f)
        }
    }

    /// `--options JSON`: a JSON object, or None without the flag. Anything else is refused
    /// (Swift throws the parser's own error for malformed text; both are an [`Invalid`] here).
    pub fn options(text: Option<&str>) -> Result<Option<JsonObject>, Invalid> {
        let Some(text) = text else { return Ok(None) };
        let v = JsonValue::parse(text).map_err(|e| Invalid {
            description: format!("--options: {e}"),
        })?;
        match v {
            JsonValue::Object(o) => Ok(Some(o)),
            _ => Err(Invalid {
                description: "--options must be a JSON object".into(),
            }),
        }
    }

    /// The spec to compose (options validated and merged as `generate` does).
    pub fn spec(&self) -> Result<TrackSpec, Invalid> {
        let mut spec = TrackSpec::new(&self.genre, self.seed, self.structure.as_deref());
        spec.cohesion = self.cohesion.clone();
        match &self.options {
            None => Ok(spec),
            Some(o) => compose_options::apply(o, &spec),
        }
    }

    /// `opts` with the stem filter: every render of the request (the main one, `--room`'s dry
    /// one, `--balance`'s solos) goes through this.
    pub fn applied(&self, opts: &WaveOptions) -> WaveOptions {
        let mut o = opts.clone();
        o.role_filter = self.filter.clone();
        o
    }

    /// The report's `filter`, `options` and `cohesion` keys (absent when not asked for, so a
    /// plain render's JSON is unchanged). Swift builds a dictionary, so its key order is
    /// unspecified; these come in that order.
    pub fn report_keys(&self, spec: &TrackSpec) -> JsonObject {
        let mut out = JsonObject::new();
        if let Some(filter) = &self.filter {
            let sorted = |v: &[String]| {
                let mut v = v.to_vec();
                v.sort();
                JsonValue::Array(v.into_iter().map(JsonValue::from).collect())
            };
            let mut f = JsonObject::new();
            f.set("mute", sorted(&filter.mute));
            if let Some(solo) = &filter.solo {
                f.set("solo", sorted(solo));
            }
            out.set("filter", JsonValue::Object(f));
        }
        if self.options.is_some() {
            out.set(
                "options",
                JsonValue::Object(compose_options::describe(spec)),
            );
        }
        if self.cohesion != TrackSpec::new(&self.genre, self.seed, None).cohesion {
            out.set("cohesion", JsonValue::str(self.cohesion.as_str()));
        }
        out
    }
}
