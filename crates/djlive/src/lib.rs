//! The live-coding language (docs/BTW.md): a small text language of our own. Pattern lines
//! compile to djrust Petri nets (raw-nets envelopes, `djengine::roll`) and control lines to calls
//! on the control-language registry; [`runtime::Live`] applies a program on the next bar and keeps
//! the last good one when an evaluation has errors.
//!
//! Pure: no I/O, no clock, no threads. The same text is the same net: [`compile::check`] gives the
//! same envelopes and CIDs every time, and the runtime runs on a [`runtime::LiveHost`] the app (or a
//! test's fake clock) provides.

pub mod algebra;
pub mod answer;
pub mod assertion;
pub mod assets;
pub mod ast;
pub mod chance;
pub mod compile;
pub mod constraint;
pub mod diag;
pub mod doc;
pub mod docall;
pub mod dur;
pub mod env;
pub mod evolve;
pub mod expand;
pub mod expr;
pub mod flow;
pub mod format;
pub mod gen;
pub mod grammar;
pub mod highlight;
pub mod launch;
pub mod lex;
pub mod linked;
pub mod meter;
pub mod mixmoves;
pub mod music;
pub mod mutate;
pub mod nets;
pub mod parse;
pub mod partial;
pub mod pattern;
pub mod perform;
pub mod ports;
pub mod print;
pub mod progld;
pub mod rat;
pub mod refs;
pub mod roles;
pub mod runtime;
pub mod scale;
pub mod seed;
pub mod set;
pub mod shape;
pub mod share;
pub mod sing;
pub mod station;
pub mod stored;
pub mod sugar;
pub mod tempo;
pub mod template;

pub use compile::{check, program_cid, Check, Compiled};
pub use diag::{Diagnostic, Severity, Span};
pub use runtime::{Live, LiveHost};
