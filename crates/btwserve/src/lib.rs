//! btwserve's library half: the parts of the server that are tested from `tests/` (the render route lives in the
//! binary, which includes btwd's render.rs by path). Today that is the playlists API.

pub mod autoformat;
pub mod diagnostics;
pub mod gate;
pub mod genres;
pub mod lineage;
pub mod playlists;
pub mod opendata;
pub mod forms;
pub mod found;
pub mod phrasebook;
pub mod rights;
pub mod themes;
