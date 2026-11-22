//! Score import: a Standard MIDI File or MusicXML → a piano roll → a raw-`nets` share
//! envelope (`dj_import`). Ports `Session/MidiImport.swift` (+ `ScoreImport`) and
//! `Session/MusicXMLImport.swift`.
//!
//! Pure: bytes in, a [`midi::Converted`] out. The app reads the file and loads the
//! envelope; it never publishes (a raw-nets share embeds the notes: mind the file's licence).

pub mod midi;
pub mod musicxml;
pub(crate) mod xml;

pub use midi::{
    convert, convert_bytes, parse, Converted, MidiImportError, Options, Parsed, RawNote,
    ScoreImport, Source,
};
