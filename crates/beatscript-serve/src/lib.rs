//! beatscript-serve's library half: the parts of the server that are tested from `tests/` (the render route lives in the
//! binary, which includes beatscript's render.rs by path). Today that is the playlists API.

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

/// A setting from the environment by its name after the prefix (`SESSION_SECRET`, `ADMIN_TOKEN`, a flag's `PUBLIC_URL`):
/// `BEATSCRIPT_<name>`, else `BTW_<name>`, its name before the rename, so an env file written then (production's says
/// `BTW_*`) keeps working. Set but empty counts as set, as it did.
pub fn env_var(name: &str) -> Option<String> {
    env_var_in(name, |k| std::env::var(k).ok())
}

/// [`env_var`] over any lookup (a test's map).
pub fn env_var_in(name: &str, get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get(&format!("BEATSCRIPT_{name}")).or_else(|| get(&format!("BTW_{name}")))
}

/// A flag's environment name: `public-url` is `PUBLIC_URL` (`BEATSCRIPT_PUBLIC_URL`, else `BTW_PUBLIC_URL`).
pub fn flag_env(flag: &str) -> Option<String> {
    env_var(&flag.to_uppercase().replace('-', "_"))
}

#[cfg(test)]
mod env_tests {
    use super::env_var_in;
    use std::collections::HashMap;

    #[test]
    fn the_new_name_first_then_the_old_one() {
        let m: HashMap<&str, &str> = [("BEATSCRIPT_A", "new"), ("BTW_A", "old"), ("BTW_B", "old"), ("BEATSCRIPT_C", "")].into();
        let get = |k: &str| m.get(k).map(|v| v.to_string());
        assert_eq!(env_var_in("A", get).as_deref(), Some("new"));
        assert_eq!(env_var_in("B", get).as_deref(), Some("old"), "an env file that still says BTW_* is read");
        assert_eq!(env_var_in("C", get).as_deref(), Some(""), "set but empty is set");
        assert_eq!(env_var_in("D", get), None);
    }

    #[test]
    fn the_process_environment_is_read_the_same_way() {
        // a name no other test reads, so setting it races nothing
        std::env::set_var("BTW_SURFACE_ENV_FALLBACK_PROBE", "from the old env file");
        assert_eq!(super::env_var("SURFACE_ENV_FALLBACK_PROBE").as_deref(), Some("from the old env file"));
        assert_eq!(super::flag_env("surface-env-fallback-probe").as_deref(), Some("from the old env file"));
        std::env::set_var("BEATSCRIPT_SURFACE_ENV_FALLBACK_PROBE", "new");
        assert_eq!(super::env_var("SURFACE_ENV_FALLBACK_PROBE").as_deref(), Some("new"));
    }
}
