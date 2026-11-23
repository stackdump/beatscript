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
//! takes one, so a write path that skips the gate does not compile. It also carries what the text as sent says it
//! vendored (its `# from: <call> -> <cid>` lines, [`Admitted::based_on`]), which `write_program` keeps in `meta.based_on`:
//! the stored text is canonical and has no comments, so every write path keeps the provenance the same way. `tests/storage_gate.rs` also reads the source
//! tree and fails if anything else writes into the programs directory.
//!
//! Write paths (each calls [`admit`]; `tests/storage_gate.rs` enumerates them):
//! `PUT /api/programs/<cid>`, `PUT /api/shares/<cid>` (signed-in share, web and phone), the render route's
//! `<cid>.<packed>` store, MCP `share` (signed in), MCP `render` / `record` with `text`, playlists (an item must name
//! an admitted stored program or an example: [`admit_stored`]) and MCP `queue` items (held in memory, gated the same
//! way). Beta sign-ups are not programs and are not gated.
//!
//! The rename (BeatScript 1 -> 2): a BeatScript 1 program is stored with its BeatScript 2 twin, the same program with the
//! version line first ([`write_twin`]), so every old CID has its new one in the store. The twin goes through [`admit`] and
//! [`write_program`] like any program; the old file is never rewritten.

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
    based_on: Vec<String>,
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
    /// The provenance lines of the text as sent (`djlive::refs::from_lines`: each `# from: <call> -> <cid>` comment as
    /// `<call> -> <cid>`, in order, each once), what [`write_program`] keeps as `meta.based_on`. Empty for a canonical text.
    pub fn based_on(&self) -> &[String] {
        &self.based_on
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
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap_or(Value::Null);
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
    Ok(Admitted { cid: cid.to_string(), canonical: canonical.to_string(), license: lic, based_on: djlive::refs::from_lines(text) })
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
/// into `meta`, and `meta.based_on` the admitted text's provenance lines (none: no key; a caller's own is replaced, so
/// the text as sent is its only source), through a temp file and link(2) (first write wins; a race with another server
/// stores once). Ok(Some(bytes)) when this call stored it, Ok(None) when the file was already there.
///
/// A BeatScript 1 program (old clients still send the old spelling) is stored with its BeatScript 2 twin, whether this call
/// stored it or it was already there and has no twin yet ([`write_twin`]): the map from old CIDs to new stays complete. A twin
/// that cannot be stored is said on stderr and does not fail the program's own write.
pub fn write_program(dir: &Path, a: &Admitted, meta: Value, encode: impl Fn(&Value) -> String) -> std::io::Result<Option<usize>> {
    let out = write_file(dir, a, meta, &encode)?;
    if djlive::version::of_canonical(&a.canonical) == djlive::version::Version::V1 {
        let x_meta = stored_meta(dir, &a.cid).unwrap_or(Value::Null);
        if let Err(r) = write_twin(dir, &a.cid, &a.canonical, &x_meta, &now_rfc3339(), &encode) {
            eprintln!("beatscript-serve: the BeatScript 2 twin of {} is not stored: {}", a.cid, r.message);
        }
    }
    Ok(out)
}

/// What a twin's derivation is called, in its `meta.derivation` and in the lineage.
pub const RENAMED: &str = "renamed";

/// The meta of the twin of program `x_cid` whose stored meta is `x_meta`: everything `x_meta` says (its name, credits,
/// author, licence, `created`...), with `forked_from` the old CID, `derivation` [`RENAMED`], `source` `renamed` and
/// `renamed_at` the time the twin is made.
pub fn twin_meta(x_meta: &Value, x_cid: &str, renamed_at: &str) -> Value {
    let mut m = match x_meta {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    m.insert("forked_from".into(), json!(x_cid));
    m.insert("derivation".into(), json!(RENAMED));
    m.insert("source".into(), json!("renamed"));
    m.insert("renamed_at".into(), json!(renamed_at));
    Value::Object(m)
}

/// Store the BeatScript 2 twin of the BeatScript 1 program `x_cid` (canonical text `x_canonical`, stored meta `x_meta`):
/// `djlive::version::upgrade_canonical` of its text, run through [`admit`] with its licence and written by [`write_program`]
/// with [`twin_meta`]. References to other programs in the text are left as they are (old CIDs still resolve). Ok((the
/// twin's CID, Some(bytes) when this call stored it, None when it was already there)); Err when `x_canonical` is not a
/// BeatScript 1 program with that CID, or the gate refuses the twin.
pub fn write_twin(dir: &Path, x_cid: &str, x_canonical: &str, x_meta: &Value, renamed_at: &str, encode: impl Fn(&Value) -> String) -> Result<(String, Option<usize>), Refusal> {
    let mut y = admit_twin(x_cid, x_canonical, x_meta)?;
    if dir.join(format!("{}.json", y.cid)).exists() {
        return Ok((y.cid, None));
    }
    // the twin is the same program: what the old one vendored (its stored `based_on`, which the gate wrote from the text
    // as sent) is the twin's too, as the rest of its meta is (the canonical text it is admitted from has no `# from:` lines)
    y.based_on = x_meta["based_on"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let n = write_file(dir, &y, twin_meta(x_meta, x_cid, renamed_at), &encode).map_err(|e| Refusal::new(500, "internal", e.to_string()))?;
    Ok((y.cid, n))
}

/// The gate's verdict on the twin of a BeatScript 1 program (what [`write_twin`] stores, and what a dry run asks).
pub fn admit_twin(x_cid: &str, x_canonical: &str, x_meta: &Value) -> Result<Admitted, Refusal> {
    use djlive::version::{of_canonical, upgrade_canonical, Version};
    if of_canonical(x_canonical) != Version::V1 || djlive::compile::program_cid(x_canonical) != x_cid {
        return Err(Refusal::new(422, "not_beatscript_1", format!("{x_cid} is not a BeatScript 1 program with that text")));
    }
    let want = djlive::compile::renamed_cid(x_canonical);
    let y = admit(&upgrade_canonical(x_canonical), x_meta["license"].as_str().unwrap_or(""), "")?;
    if y.cid != want {
        return Err(Refusal::new(502, "checker_error", format!("the BeatScript 2 text of {x_cid} checks as {}, not its twin {want}", y.cid)));
    }
    Ok(y)
}

/// The `meta` of a stored file, when it reads.
fn stored_meta(dir: &Path, cid: &str) -> Option<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{cid}.json"))).ok()?).ok()?;
    (v["cid"].as_str() == Some(cid)).then(|| v["meta"].clone())
}

/// Now, as RFC 3339 UTC with seconds.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    crate::playlists::model::rfc3339(secs)
}

/// The write itself (see [`write_program`]).
fn write_file(dir: &Path, a: &Admitted, meta: Value, encode: &impl Fn(&Value) -> String) -> std::io::Result<Option<usize>> {
    let mut meta = match meta {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    meta.insert("license".into(), json!(a.license));
    meta.insert("visibility".into(), json!("public"));
    if a.based_on.is_empty() {
        if meta.contains_key("based_on") {
            meta = meta.into_iter().filter(|(k, _)| k != "based_on").collect();
        }
    } else {
        meta.insert("based_on".into(), json!(a.based_on));
    }
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

    fn read(dir: &Path, cid: &str) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(format!("{cid}.json"))).unwrap()).unwrap()
    }

    #[test]
    fn a_beatscript_1_program_is_stored_with_its_twin() {
        let d = tempfile::tempdir().unwrap();
        let x = admit(OK, "cc-by-4.0", "").unwrap();
        assert_eq!(djlive::version::of_canonical(x.canonical()), djlive::version::Version::V1);
        let meta = json!({"name": "four", "author": "ed25519:aa", "credits": ["someone"], "created": "2026-11-01T00:00:00Z", "source": "share"});
        assert!(write_program(d.path(), &x, meta, |v| v.to_string()).unwrap().is_some());
        let y = djlive::compile::renamed_cid(x.canonical());
        let f = read(d.path(), &y);
        assert_eq!(f["cid"], y);
        assert_eq!(f["text"].as_str().unwrap(), djlive::version::upgrade_canonical(x.canonical()));
        assert_eq!(djlive::compile::program_cid(f["text"].as_str().unwrap()), y);
        let m = &f["meta"];
        assert_eq!((m["forked_from"].as_str(), m["derivation"].as_str(), m["source"].as_str()), (Some(x.cid()), Some(RENAMED), Some("renamed")));
        // the rest of the old program's meta, its created kept
        assert_eq!((m["name"].as_str(), m["author"].as_str(), m["license"].as_str(), m["visibility"].as_str()), (Some("four"), Some("ed25519:aa"), Some("CC-BY-4.0"), Some("public")));
        assert_eq!(m["credits"], json!(["someone"]));
        assert_eq!(m["created"], "2026-11-01T00:00:00Z");
        assert!(m["renamed_at"].as_str().is_some_and(|t| t.ends_with('Z')));
        // the old file is the old program, untouched; the twin's legacy CID is the original
        assert_eq!(read(d.path(), x.cid())["text"].as_str(), Some(x.canonical()));
        assert_eq!(djlive::compile::renamed_cid(f["text"].as_str().unwrap()), x.cid());
        assert_eq!(audit_dir(d.path()), Vec::<(String, String)>::new());
        // a BeatScript 2 program has no twin
        let v2 = admit(&format!("#! beatscript 2\n{OK}").replace("x ~ x ~", "x x x x"), "", "").unwrap();
        assert_eq!(djlive::version::of_canonical(v2.canonical()), djlive::version::Version::V2);
        write_program(d.path(), &v2, json!({}), |v| v.to_string()).unwrap();
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 3);
    }

    #[test]
    fn an_old_program_stored_before_the_rename_gets_its_twin_when_it_is_sent_again() {
        let d = tempfile::tempdir().unwrap();
        let x = admit(OK, "", "").unwrap();
        // as the store was: the old file alone
        std::fs::write(d.path().join(format!("{}.json", x.cid())), json!({"cid": x.cid(), "text": x.canonical(), "meta": {"license": "CC0-1.0", "visibility": "public", "name": "old", "created": "2026-01-01T00:00:00Z"}}).to_string()).unwrap();
        assert_eq!(write_program(d.path(), &x, json!({"name": "new"}), |v| v.to_string()).unwrap(), None, "first write wins");
        let f = read(d.path(), &djlive::compile::renamed_cid(x.canonical()));
        assert_eq!((f["meta"]["name"].as_str(), f["meta"]["created"].as_str()), (Some("old"), Some("2026-01-01T00:00:00Z")), "the twin's meta is the stored program's");
        // and again: nothing more
        assert_eq!(write_twin(d.path(), x.cid(), x.canonical(), &json!({}), "t", |v| v.to_string()).unwrap().1, None);
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_twin_is_refused_for_what_is_not_a_beatscript_1_program() {
        let x = admit(OK, "", "").unwrap();
        let other = admit("tempo 121\ndeck A: kick x ~ x ~\n", "", "").unwrap();
        assert_eq!(admit_twin(other.cid(), x.canonical(), &json!({})).unwrap_err().code, "not_beatscript_1");
        let v2 = djlive::version::upgrade_canonical(x.canonical());
        assert_eq!(admit_twin(&djlive::compile::program_cid(&v2), &v2, &json!({})).unwrap_err().code, "not_beatscript_1");
        // the gate still decides: a licence it does not take is refused for the twin too
        assert_eq!(admit_twin(x.cid(), x.canonical(), &json!({"license": "all-rights-reserved"})).unwrap_err().code, "not_creative_commons");
    }

    /// The text as sent says what it vendored (its `# from: <call> -> <cid>` lines): the write keeps them in `meta.based_on`
    /// beside the canonical text, which has none. The text is their only source: a caller's own `based_on` is replaced.
    #[test]
    fn the_write_keeps_the_from_lines_of_the_text_as_sent() {
        let pin = "z4EBG9jBATRr9XWGL7beKXF7dm5Je1rPzaSymt6pvg12kd1nqHf";
        let line = format!("gen@2 techno seed=42 section=drop part=bass -> {pin}");
        let sent = format!("# from: {line}\n# from: no pin\n{OK}# from: {line}\n");
        let a = admit(&sent, "", "").unwrap();
        assert_eq!(a.based_on(), [line.clone()]);
        assert_eq!(a.cid(), admit(OK, "", "").unwrap().cid(), "the comments are not in the CID");
        assert!(admit(a.canonical(), "", "").unwrap().based_on().is_empty(), "the canonical text has no comments");
        let d = tempfile::tempdir().unwrap();
        let read = |cid: &str| -> Value { serde_json::from_slice(&std::fs::read(d.path().join(format!("{cid}.json"))).unwrap()).unwrap() };
        assert!(write_program(d.path(), &a, json!({"license": "", "visibility": "", "name": "kept", "based_on": ["forged -> zforged"]}), |v| v.to_string()).unwrap().is_some());
        let v = read(a.cid());
        assert_eq!(v["text"], json!(OK));
        assert_eq!(v["meta"]["based_on"], json!([line]));
        assert_eq!(v["meta"]["name"], "kept");
        // a text with no lines: no key, and a caller's is dropped
        let plain = admit("tempo 121\ndeck A: kick x ~ x ~\n", "", "").unwrap();
        write_program(d.path(), &plain, json!({"based_on": ["forged -> zforged"], "name": "plain"}), |v| v.to_string()).unwrap();
        let v = read(plain.cid());
        assert!(v["meta"].get("based_on").is_none(), "{v}");
        assert_eq!(v["meta"]["name"], "plain");
        // a BeatScript 1 text is stored with its twin, and the twin vendored what it did
        assert_eq!(djlive::version::of_canonical(a.canonical()), djlive::version::Version::V1);
        let twin = read(&djlive::compile::renamed_cid(a.canonical()));
        assert_eq!(twin["meta"]["based_on"], json!([line]), "{twin}");
        assert!(read(&djlive::compile::renamed_cid(plain.canonical()))["meta"].get("based_on").is_none());
    }
}
