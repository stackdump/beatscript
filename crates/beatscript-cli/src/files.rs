//! Which files are BeatScript programs: `.beatscript`, and `.btw`, the extension before the rename, read as a legacy alias
//! (a program saved as `.btw` opens and imports as it always did; nothing new is written under that name).
//! (rename-to-beatscript: keep. This file names the old extension on purpose.)

/// The extensions of a program file, the current one first.
pub const EXTENSIONS: &[&str] = &["beatscript", "btw"];

/// Is `path` a program file, by its extension?
pub fn is_program(path: &std::path::Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| EXTENSIONS.contains(&x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn both_extensions_are_programs_and_nothing_else_is() {
        assert!(is_program(Path::new("a/song.beatscript")));
        assert!(is_program(Path::new("song.btw")));
        for p in ["song.txt", "song", "beatscript", "song.beatscript.bak", "btw"] {
            assert!(!is_program(Path::new(p)), "{p}");
        }
    }
}
