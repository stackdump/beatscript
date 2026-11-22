//! Mixxx controller mappings (`*.midi.xml`) as djrust devices (docs/MIXXX-MAPPINGS.md).
//!
//! - [`preset`]: the XML, as data
//! - [`vocab`]: Mixxx's `[Group] key` control names onto the registry's targets
//! - [`profile`]: a mapping as a [`SurfaceProfile`](djrust::control::surface::SurfaceProfile)
//!   the router attaches, with the report of what mapped and what did not
//! - [`discover`]: which mapping suits which MIDI port, by name
//!
//! Pure and `no_std`: the files are read by `djhost::mixxx`, from the user's own Mixxx install or
//! a folder they chose. No mapping is in this repo.

pub mod discover;
pub mod preset;
pub mod profile;
pub mod vocab;

pub use preset::{parse, Preset};
pub use profile::{build, device_name, MixxxProfile, Prefilter, Report, Unmapped, Why, DEVICE_PREFIX};
