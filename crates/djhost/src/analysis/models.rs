//! The Beat This! model files: where they live, what they must hash to, and fetching them.
//!
//! Two ONNX files, about 11 MB together, from the `beat-this-rs` repository (MIT) at a pinned
//! commit: the log-mel front end and the small beat model (the converted `small1` checkpoint of
//! Beat This!, MIT, JKU Linz). They are **never committed here**: [`ensure`] downloads whatever
//! is missing on first use, refuses a file whose length or SHA-256 is not the pinned one, and
//! writes it atomically. docs/BEAT-DETECTION.md has the licences.

use std::path::{Path, PathBuf};

/// One model file.
#[derive(Clone, Copy, Debug)]
pub struct ModelFile {
    /// The file name in the models folder.
    pub name: &'static str,
    pub url: &'static str,
    /// Lower-case hex SHA-256 of the file.
    pub sha256: &'static str,
    pub bytes: u64,
}

/// The `beat-this-rs` commit the files are pinned to (the v1.1.0 tag).
pub const PINNED_COMMIT: &str = "1ae768e78f1ad83b0ed3886241dc29ffde853c40";

pub const MEL: ModelFile = ModelFile {
    name: "beat-this-mel.onnx",
    url: "https://raw.githubusercontent.com/danigb/beat-this-rs/1ae768e78f1ad83b0ed3886241dc29ffde853c40/models/mel_spectrogram.onnx",
    sha256: "fdd59e65c515331308e4c8841edf99972deca646bdf6197744c2a5b7755e3de9",
    bytes: 270_742,
};

pub const SMALL: ModelFile = ModelFile {
    name: "beat-this-small.onnx",
    url: "https://raw.githubusercontent.com/danigb/beat-this-rs/1ae768e78f1ad83b0ed3886241dc29ffde853c40/models/beat_this_small.onnx",
    sha256: "a5f8d39d989f31859454ba27afe61c5317ca95e4d9373e6853e5361b8937172f",
    bytes: 10_555_592,
};

pub const FILES: [ModelFile; 2] = [MEL, SMALL];

/// `$DJRUST_MODELS`, else `<support>/models/beat-this`.
pub fn default_dir() -> PathBuf {
    crate::stores::env_or_support("DJRUST_MODELS", "models/beat-this")
}

/// What is in a models folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub dir: PathBuf,
    /// Every file is there at its pinned length (the hash is checked at download and at load).
    pub ready: bool,
    pub missing: Vec<&'static str>,
    /// Bytes still to download.
    pub to_fetch: u64,
}

pub fn status(dir: &Path) -> Status {
    let mut missing = Vec::new();
    let mut to_fetch = 0;
    for f in FILES {
        let ok = std::fs::metadata(dir.join(f.name)).map(|m| m.len() == f.bytes).unwrap_or(false);
        if !ok {
            missing.push(f.name);
            to_fetch += f.bytes;
        }
    }
    Status { dir: dir.to_path_buf(), ready: missing.is_empty(), missing, to_fetch }
}

/// Lower-case hex SHA-256 of `bytes` (the `neural` feature has the hash; without it nothing is
/// downloaded, so nothing needs one).
#[cfg(feature = "neural")]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Check `bytes` is the file `f` should be.
#[cfg(feature = "neural")]
pub fn verify(f: &ModelFile, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != f.bytes {
        return Err(format!("{}: {} bytes, expected {}", f.name, bytes.len(), f.bytes));
    }
    let got = sha256_hex(bytes);
    if got != f.sha256 {
        return Err(format!("{}: SHA-256 {got} is not the pinned {}", f.name, f.sha256));
    }
    Ok(())
}

/// Fetch every missing file with `fetch` (the file's bytes, from wherever), check it against the
/// pin and write it into `dir`. Stops at the first failure and leaves nothing half-written.
#[cfg(feature = "neural")]
pub fn ensure(
    dir: &Path,
    fetch: &mut dyn FnMut(&ModelFile) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for f in FILES {
        let path = dir.join(f.name);
        if std::fs::metadata(&path).map(|m| m.len() == f.bytes).unwrap_or(false) {
            continue;
        }
        let bytes = fetch(&f)?;
        verify(&f, &bytes)?;
        let tmp = dir.join(format!("{}.part", f.name));
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// The network fetch: GET `f.url` (https, redirects followed, 60 s) up to a little over the
/// pinned length.
#[cfg(feature = "neural")]
pub fn http_fetch(f: &ModelFile) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .build()
        .into();
    let mut resp = agent.get(f.url).call().map_err(|e| format!("{}: {e}", f.url))?;
    let mut out = Vec::with_capacity(f.bytes as usize);
    resp.body_mut()
        .as_reader()
        .take(f.bytes + 1024)
        .read_to_end(&mut out)
        .map_err(|e| format!("{}: {e}", f.url))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names_what_is_missing() {
        let dir = std::env::temp_dir().join(format!("djmodels-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = status(&dir);
        assert!(!s.ready);
        assert_eq!(s.missing, vec![MEL.name, SMALL.name]);
        assert_eq!(s.to_fetch, MEL.bytes + SMALL.bytes);
        // about 11 MB, as the brief says
        assert!(s.to_fetch > 10_000_000 && s.to_fetch < 12_000_000);
        // a file of the wrong length is not there
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MEL.name), b"short").unwrap();
        assert_eq!(status(&dir).missing.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pins_are_well_formed() {
        for f in FILES {
            assert_eq!(f.sha256.len(), 64);
            assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(f.url.starts_with("https://"));
            assert!(f.url.contains(PINNED_COMMIT), "a model URL must be pinned to a commit, not a branch");
        }
    }

    #[cfg(feature = "neural")]
    #[test]
    fn a_bad_checksum_is_refused_and_nothing_is_written() {
        let dir = std::env::temp_dir().join(format!("djmodels-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // right length, wrong bytes
        let mut fetch = |f: &ModelFile| Ok(vec![7u8; f.bytes as usize]);
        let e = ensure(&dir, &mut fetch).unwrap_err();
        assert!(e.contains("SHA-256"), "{e}");
        assert!(!status(&dir).ready);
        assert!(!dir.join(MEL.name).exists());
        // wrong length
        let mut fetch = |_: &ModelFile| Ok(vec![0u8; 10]);
        assert!(ensure(&dir, &mut fetch).unwrap_err().contains("bytes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "neural")]
    #[test]
    fn sha256_is_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
