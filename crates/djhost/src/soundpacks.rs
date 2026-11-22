//! The sound packs the app ships: djrust's Toy Box bank (`djengine::sound::toybox`), synthesized
//! from scratch, written into the soundfonts folder at start so the engine's library scan finds it
//! like any `.sf2` there. The file is rewritten when djrust's bank version changes (a sidecar
//! `toybox.version` holds the one written). MuseScore General is fetched by the app on request
//! (`ui/SoundPacks.kt`); nothing that may not be redistributed is ever written here.

use std::path::Path;

use djengine::sound::toybox;

/// The sidecar beside the bank naming the version written.
const VERSION_FILE: &str = "toybox.version";

/// Write `toybox.sf2` into `dir` unless the version there is current. Ok(true): written.
pub fn ensure_toybox(dir: &Path) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?;
    let bank = dir.join(toybox::FILE_NAME);
    let marker = dir.join(VERSION_FILE);
    let want = toybox::VERSION.to_string();
    if bank.is_file() && std::fs::read_to_string(&marker).is_ok_and(|v| v.trim() == want) {
        return Ok(false);
    }
    let tmp = dir.join(format!("{}.part", toybox::FILE_NAME));
    std::fs::write(&tmp, toybox::build())?;
    std::fs::rename(&tmp, &bank)?;
    std::fs::write(&marker, want)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bank_is_written_once_per_version() {
        let dir = std::env::temp_dir().join(format!("djhost-toybox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_toybox(&dir).unwrap());
        assert!(!ensure_toybox(&dir).unwrap());
        assert!(dir.join(toybox::FILE_NAME).metadata().unwrap().len() > 1000);
        let found = crate::sampler::Library::scan(&dir).for_host();
        // a by-name bank (its programs are not General MIDI's): the engine gets a library for it,
        // and the scan counts it beside the SoundFonts (the menu shows the sum)
        assert!(found.library.is_some(), "the engine gets the Toy Box library");
        assert_eq!(found.sound_fonts + found.local_only, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
