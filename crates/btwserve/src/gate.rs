//! The storage gate: beatscript.app only ever stores Creative Commons programs. Users do whatever they like on
//! their own devices; the server keeps nothing else.
//!
//! [`admit`] is the one function that decides. It re-checks the text itself (it does not trust a caller's report) and
//! admits a program only when
//!
//! - it checks, and its canonical text hashes to its CID;
//! - its licence is a Creative Commons id ([`cc_license`]: CC0-1.0, the default, or CC-BY…-x.y);
//! - its visibility is public, and its text does not say otherwise (`#! visibility:`);
//! - it names no sample or loop asset and does not `use restricted`.
//!
//! An [`Admitted`] can only be made here, and [`write_program`] (the only code that writes `programs/<cid>.json`)
//! takes one, so a write path that skips the gate does not compile. `tests/storage_gate.rs` also reads the source
//! tree and fails if anything else writes into the programs directory.
//!
//! Write paths (each calls [`admit`]; `tests/storage_gate.rs` enumerates them):
//! `PUT /api/programs/<cid>`, `PUT /api/shares/<cid>` (signed-in share, web and phone), the render route's
//! `<cid>.<packed>` store, MCP `share` (signed in), MCP `render` / `record` with `text`, playlists (an item must name
//! an admitted stored program or an example: [`admit_stored`]) and MCP `queue` items (held in memory, gated the same
//! way). Beta sign-ups are not programs and are not gated.

use std::path::Path;

use serde_json::{json, Value};

/// What a refused program is told.
pub const CC_ONLY: &str = "beatscript.app only stores Creative Commons programs; keep this one local";

/// The licence a program gets when none is given.
pub const DEFAULT_LICENSE: &str = "CC0-1.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal { status, code, message: message.into() }
    }
    fn cc(why: &str) -> Refusal {
        Refusal::new(422, "not_creative_commons", format!("{CC_ONLY} ({why})"))
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A program the gate let through. Its fields are private: only [`admit`] makes one.
#[derive(Clone, Debug)]
pub struct Admitted {
    cid: String,
    canonical: String,
    license: String,
}

impl Admitted {
    pub fn cid(&self) -> &str {
        &self.cid
    }
    pub fn canonical(&self) -> &str {
        &self.canonical
    }
    /// The licence as stored: the SPDX spelling of the CC id.
    pub fn license(&self) -> &str {
        &self.license
    }
}

/// The SPDX id of a Creative Commons licence, in its canonical spelling, or None. CC0-1.0 and the six CC BY
/// families at versions 1.0, 2.0, 2.5, 3.0 and 4.0 (the ports, `-IGO` and the like, are not accepted: say CC BY 4.0).
pub fn cc_license(id: &str) -> Option<String> {
    let u = id.trim().to_ascii_uppercase();
    if u == "CC0-1.0" || u == "CC0" {
        return Some("CC0-1.0".into());
    }
    let rest = u.strip_prefix("CC-BY")?;
    let (family, version) = rest.rsplit_once('-')?;
    if !matches!(family, "" | "-SA" | "-NC" | "-ND" | "-NC-SA" | "-NC-ND") {
        return None;
    }
    if !matches!(version, "1.0" | "2.0" | "2.5" | "3.0" | "4.0") {
        return None;
    }
    Some(format!("CC-BY{family}-{version}"))
}

/// `#! visibility:` with any value but `public`, in any spelling (the store's `declares_not_public`).
pub fn declares_not_public(text: &str) -> bool {
    text.split('\n').any(|l| {
        let Some(rest) = l.trim().strip_prefix("#!") else { return false };
        let squeezed: String = rest.split_whitespace().collect::<String>().to_lowercase();
        matches!(squeezed.strip_prefix("visibility:"), Some(v) if v != "public")
    })
}

/// The decision. `text` is the program as sent (it is checked here, again); `license` and `visibility` are the
/// rights asked for ("" = the defaults, CC0-1.0 and public).
pub fn admit(text: &str, license: &str, visibility: &str) -> Result<Admitted, Refusal> {
    let lic = if license.trim().is_empty() { DEFAULT_LICENSE } else { license.trim() };
    let Some(lic) = cc_license(lic) else {
        return Err(Refusal::cc(&format!("its licence is {}, not a Creative Commons licence such as CC0-1.0 or CC-BY-4.0", lic)));
    };
    let vis = visibility.trim();
    if !vis.is_empty() && !vis.eq_ignore_ascii_case("public") {
        return Err(Refusal::cc(&format!("it is {}, and only public programs are stored", vis.to_lowercase())));
    }
    if text.trim().is_empty() {
        return Err(Refusal::new(400, "bad_request", "program text is empty"));
    }
    if declares_not_public(text) {
        return Err(Refusal::new(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site"));
    }
    let rep: Value = serde_json::from_str(&btwapi::check_json(text)).unwrap_or(Value::Null);
    if rep["ok"] != true {
        // a program that draws on one this site does not keep says which: store that one first (what a program needs comes before it)
        let first = rep["diagnostics"].as_array().and_then(|d| d.iter().find(|d| d["severity"] == "error")).and_then(|d| d["message"].as_str()).filter(|m| djlive::env::is_not_here(m));
        let why = first.map(|m| format!(": {m}; PUT the programs it draws on first")).unwrap_or_default();
        return Err(Refusal::new(422, "bad_request", format!("the program has {} error(s){why}", rep["errors"].as_i64().unwrap_or(0))));
    }
    if rep["restricted"] == true || rep["uses_assets"] == true {
        return Err(Refusal::new(422, "restricted", "programs that use restricted sounds or sample/loop assets are not stored on this site"));
    }
    let (Some(cid), Some(canonical)) = (rep["cid"].as_str(), rep["canonical"].as_str()) else {
        return Err(Refusal::new(502, "checker_error", "the checker failed; try again"));
    };
    if canonical.is_empty() || djlive::compile::program_cid(canonical) != cid {
        return Err(Refusal::new(502, "checker_error", "the checker failed; try again"));
    }
    // the canonical text is what is stored: it must say the same (a canonical form never adds a header, but the
    // gate does not rely on that)
    if declares_not_public(canonical) {
        return Err(Refusal::new(422, "restricted", "a program that says it is not public (`#! visibility:`) is not stored on this site"));
    }
    Ok(Admitted { cid: cid.to_string(), canonical: canonical.to_string(), license: lic })
}

/// A stored file's verdict: its text and meta run through [`admit`] again (what `tests/storage_gate.rs` and
/// `examples/audit_store.rs` use to list what is already stored and would not be admitted today).
pub fn audit_file(body: &[u8]) -> Result<Admitted, Refusal> {
    let v: Value = serde_json::from_slice(body).map_err(|_| Refusal::new(500, "unreadable", "the file is not JSON"))?;
    let text = v["text"].as_str().unwrap_or("");
    let a = admit(text, v["meta"]["license"].as_str().unwrap_or(""), v["meta"]["visibility"].as_str().unwrap_or(""))?;
    if v["cid"].as_str() != Some(a.cid()) {
        return Err(Refusal::new(500, "cid_mismatch", "the file's cid is not its text's"));
    }
    Ok(a)
}

/// For a playlist item or a queue entry, which only points at a program: the program `cid` names must be an example
/// or a stored file the gate admits today. `read` gives a stored file's bytes.
pub fn admit_stored(cid: &str, is_example: bool, read: impl FnOnce() -> Option<Vec<u8>>) -> Result<(), Refusal> {
    if is_example {
        return Ok(());
    }
    let Some(b) = read() else {
        return Err(Refusal::new(422, "not_stored", format!("no program {cid} is stored here: share it first (signed in); {CC_ONLY}")));
    };
    audit_file(&b).map(|_| ())
}

/// Every stored file the gate would refuse today: (file name, why). Nothing is changed or deleted.
pub fn audit_dir(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut names: Vec<_> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") && !n.starts_with('.')).collect();
    names.sort();
    for n in names {
        match std::fs::read(dir.join(&n)) {
            Err(e) => out.push((n, e.to_string())),
            Ok(b) => {
                if let Err(r) = audit_file(&b) {
                    out.push((n, format!("{}: {}", r.code, r.message)));
                }
            }
        }
    }
    out
}

/// The only write of `programs/<cid>.json`: `{"cid","text","meta"}` with the admitted licence and `public` forced
/// into `meta`, through a temp file and link(2) (first write wins; a race with another server stores once).
/// Ok(Some(bytes)) when this call stored it, Ok(None) when the file was already there.
pub fn write_program(dir: &Path, a: &Admitted, meta: Value, encode: impl Fn(&Value) -> String) -> std::io::Result<Option<usize>> {
    let mut meta = match meta {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    meta.insert("license".into(), json!(a.license));
    meta.insert("visibility".into(), json!("public"));
    let mut m = serde_json::Map::new();
    m.insert("cid".into(), json!(a.cid));
    m.insert("text".into(), json!(a.canonical));
    m.insert("meta".into(), Value::Object(meta));
    let body = encode(&Value::Object(m));
    let path = dir.join(format!("{}.json", a.cid));
    if path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let mut raw = [0u8; 8];
    let _ = getrandom::fill(&mut raw);
    let tmp = dir.join(format!(".tmp-rs-{}-{}-{}", std::process::id(), hex::encode(raw), a.cid));
    let res = (|| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&tmp, &body)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        std::fs::hard_link(&tmp, &path)
    })();
    let _ = std::fs::remove_file(&tmp);
    match res {
        Ok(()) => Ok(Some(body.len())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "tempo 120\ndeck A: kick x ~ x ~\n";
    pub const PRIVATE_SAMPLE: &str = "use restricted\nsample harp from \"sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\" license restricted-private else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~ | bass c2 ~ c2 ~ inst=harp\n";

    #[test]
    fn cc_ids() {
        for (i, o) in [("CC0-1.0", "CC0-1.0"), ("cc0", "CC0-1.0"), ("cc-by-4.0", "CC-BY-4.0"), ("CC-BY-SA-4.0", "CC-BY-SA-4.0"), ("CC-BY-NC-ND-3.0", "CC-BY-NC-ND-3.0"), ("CC-BY-2.5", "CC-BY-2.5")] {
            assert_eq!(cc_license(i).as_deref(), Some(o), "{i}");
        }
        for bad in ["all-rights-reserved", "MIT", "", "CC-BY", "CC-BY-5.0", "CC-BY-XX-4.0", "LicenseRef-x", "CC-BY-4.0-IGO", "Proprietary"] {
            assert_eq!(cc_license(bad), None, "{bad}");
        }
    }

    #[test]
    fn admits_a_plain_public_program() {
        let a = admit(OK, "", "").expect("admitted");
        assert_eq!(a.license(), "CC0-1.0");
        assert!(a.cid().starts_with('z'));
        assert_eq!(admit(OK, "cc-by-4.0", "public").unwrap().license(), "CC-BY-4.0");
    }

    #[test]
    fn refuses_what_is_not_cc() {
        let r = admit(OK, "all-rights-reserved", "").unwrap_err();
        assert_eq!((r.status, r.code), (422, "not_creative_commons"));
        assert!(r.message.starts_with(CC_ONLY), "{}", r.message);
        assert_eq!(admit(OK, "", "private").unwrap_err().code, "not_creative_commons");
        assert_eq!(admit(OK, "", "unlisted").unwrap_err().code, "not_creative_commons");
        assert_eq!(admit(&format!("#! visibility: private\n{OK}"), "", "").unwrap_err().code, "restricted");
        assert_eq!(admit(&format!("use restricted\n{OK}"), "", "").unwrap_err().code, "restricted");
        assert_eq!(admit(PRIVATE_SAMPLE, "", "").unwrap_err().code, "restricted");
        // a sample is refused even when its own tag is open: the server stores no program that names a file
        assert_eq!(admit(&PRIVATE_SAMPLE.replacen("use restricted\n", "", 1).replace("restricted-private", "cc0"), "", "").unwrap_err().code, "restricted");
        assert_eq!(admit("deck A: kick x (\n", "", "").unwrap_err().code, "bad_request");
    }
}
