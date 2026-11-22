//! btwround: audio -> BeatScript program, by analysis by synthesis. See docs/ROUNDTRIP.md.
//!
//! The forward model is `btwd render` (deterministic, frame exact). Recovery proposes events by
//! correlating rendered single-note templates against the residual, then lets an exact re-render
//! accept or reject each proposal. Scoring is L0 (sample identical), L1 (`same_as`), L2 (the diff).

pub mod btwd;
pub mod model;
pub mod recover;
pub mod score;
pub mod spec;
pub mod vel;
pub mod wav;
