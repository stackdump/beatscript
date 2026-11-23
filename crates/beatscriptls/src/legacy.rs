//! The names this server answered to before the rename (btw -> beatscript): its custom methods and its command ids. They
//! are still answered, as the new ones are, so an editor set up before the rename (a client that sends `btw/expand`, a
//! keybinding to `btw.eject`) keeps working for a release; only the new names are advertised (`initialize`).
//! (rename-to-beatscript: keep. This file spells the old names on purpose.)

/// Old custom method -> its name now.
pub const METHODS: &[(&str, &str)] = &[("btw/expand", "beatscript/expand"), ("btw/numberAt", "beatscript/numberAt"), ("btw/diagnostics", "beatscript/diagnostics")];

/// Old command id -> its id now (crate::refs::COMMANDS).
pub const COMMANDS: &[(&str, &str)] = &[("btw.expand.toggle", crate::refs::TOGGLE), ("btw.expand.copy", crate::refs::COPY), ("btw.eject", crate::refs::EJECT)];

/// The fix-all kind an editor set up before the rename asks for (`editor.codeActionsOnSave`).
pub const FIX_ALL: &str = "source.fixAll.btw";

/// The method's name now: an old one mapped, any other as it is.
pub fn method(m: &str) -> &str {
    METHODS.iter().find(|(old, _)| *old == m).map_or(m, |(_, new)| new)
}

/// The command's id now: an old one mapped, any other as it is.
pub fn command(c: &str) -> &str {
    COMMANDS.iter().find(|(old, _)| *old == c).map_or(c, |(_, new)| new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_old_name_maps_to_one_the_server_has_now() {
        for (old, new) in METHODS {
            assert_eq!(method(old), *new);
            assert!(new.starts_with("beatscript/") && old.starts_with("btw/") && old[4..] == new[11..], "{old} {new}");
        }
        for (old, new) in COMMANDS {
            assert_eq!(command(old), *new);
            assert!(crate::refs::COMMANDS.contains(new), "{new}");
            assert_eq!(&old[4..], &new[11..]);
        }
        assert_eq!(method("textDocument/hover"), "textDocument/hover");
        assert_eq!(command("beatscript.eject"), "beatscript.eject");
    }
}
