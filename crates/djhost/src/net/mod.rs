//! djnet: djrust devices working together on the LAN (docs/NET-PROTOCOL.md). Behind the
//! `network` feature: without it nothing here is compiled.
//!
//! - [`proto`]: versions, error codes, the pairing proof, the remote allowlist.
//! - [`clock`]: monotonic time, the NTP-style offset, beat points.
//! - [`store`]: this side's id and the tokens of the sides it paired with.
//! - [`ssdp`]: the host's responder and the peer's search.
//! - [`server`]: the host (the control socket's handler, on TCP).
//! - [`client`]: the peer's connection and its managed link.
//! - [`djl`] / [`receiver`]: the master with its clock as `/master.djl`, and the receiver that
//!   plays one into an input strip.

pub mod client;
pub mod clock;
pub mod djl;
pub mod proto;
pub mod receiver;
pub mod server;
pub mod ssdp;
pub mod store;

pub use client::{Conn, Hello, Link, LinkStatus, Me};
pub use clock::{mono_ns, BeatPoint};
pub use proto::NetError;
pub use server::{Host, HostConfig, PeerLive, Trust};
pub use store::Store;
