//! beatscript-api: BeatScript as JSON, pure (no I/O, no clock), shared by the `beatscript` CLI and the `djweb` wasm module so they
//! cannot disagree. See docs/BEATSCRIPT.md and site/DESIGN.md (beats-bitwrap-io, feat/btw-site).
//!
//! Every function takes the program text (or a link) and answers a JSON string:
//! [`check_json`], [`fmt_json`], [`open_json`], [`share_json`], [`generation_json`], [`nets_json`],
//! [`same_as_json`], [`instruments_json`], [`highlight_json`] / [`highlight_json_utf16`], [`station_json`] (radio), and [`cheatsheet`] (markdown).
//! Programs are checked against [`web_env`]: the control registry and the engine's instrument names as a
//! static snapshot of djhost's, with no file lookups and no snippet store of names: an `import` is resolved from the programs
//! the host knows by CID ([`known`]).

pub mod cheatsheet;
pub mod diff;
pub mod generate;
pub mod generate_v1;
pub mod highlight;
pub mod known;
pub mod report;
pub mod resolution;
pub mod run;
pub mod same;
pub mod share;
pub mod snapshot;
pub mod station;
pub mod utf16;
/// The Wheel as the page plays it (the `wheel` feature: djweb and `beatscript render`).
#[cfg(feature = "wheel")]
pub mod wheel;

pub use cheatsheet::cheatsheet;
pub use generate::{generate, generate_json, GenRequest, Generated};
pub use highlight::{highlight_json, highlight_json_utf16};
pub use report::{check_json, check_json_with, check_report, check_report_with, diagnostic_json, docs_json, expand_json, fmt_json, format_text, generation_json, instruments_json, nets_json, upgrade_text};
pub use same::{same_as_json, same_as_json_pair, same_as_report};
pub use share::{open_json, share_json, DEFAULT_ORIGIN};
pub use snapshot::{web_env, WebEnv};
pub use station::{station_answer, station_json, StationAsk};
pub use utf16::{add_utf16_spans, Utf16Index};
