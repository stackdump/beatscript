//! djengine: the private engine modules on top of the public `djrust` crate (the bitwrap audio
//! engine, ../djrust). Everything djrust has is re-exported here, so `djengine::session`,
//! `djengine::wave` and the rest still resolve; the modules below are the ones that stay
//! private:
//!
//! - [`mcp`], [`console`], [`agent`]: the MCP face of the control socket, the console's command
//!   language and the Claude agent protocol
//! - [`import`], [`compose_options`], [`render_request`]: score import, the opt-in composition
//!   techniques and what `djctl render` is asked for
//! - [`smf`]: Standard MIDI Files out (a program's notes for a DAW)
//! - [`rights`]: the license, visibility, author and lineage a user-made track or set carries
//!   in its canonical JSON (docs/LICENSING.md)

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub use djrust::*;

pub mod agent;
pub mod beatgrid;
pub mod compose_options;
pub mod console;
pub mod genre_templates;
pub mod import;
pub mod key;
pub mod keylock;
pub mod mashup;
pub mod mcp;
pub mod mixxx;
pub mod render_request;
pub mod rights;
pub mod setrec;
pub mod smf;
pub mod stems;
pub mod themes;
pub mod vocal;
