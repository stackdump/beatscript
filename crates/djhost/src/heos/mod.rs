//! Denon HEOS zones on the LAN (docs/HEOS.md): find the players, read and set each zone's volume,
//! mute and transport, see what it plays, group and ungroup them, and have them play djapp's
//! master mix, served here as an HTTP stream.
//!
//! - [`proto`]: the HEOS CLI protocol (Denon's public spec; TCP 1255, a command line out, a JSON
//!   line back), pure.
//! - [`discover`]: SSDP and mDNS, so no address is written down.
//! - [`conn`]: one CLI connection.
//! - [`service`]: the worker thread and the [`Snapshot`] the app reads without waiting.
//! - [`encode`] / [`stream`]: the master as an endless FLAC or WAV over HTTP.
//!
//! std only: no new dependencies, and nothing here touches the audio callback.

pub mod conn;
pub mod discover;
pub mod encode;
pub mod proto;
pub mod service;
pub mod stream;

pub use encode::Format;
pub use proto::{Group, NowPlaying, PlayState, Player};
pub use service::{Cmd, Heos, Snapshot, Target};
pub use stream::{StreamServer, StreamStats};
