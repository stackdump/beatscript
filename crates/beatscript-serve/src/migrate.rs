//! `beatscript-serve migrate-rename`: the data migration of the rename (BeatScript 1 -> 2), run by the deploy's own binary.
//!
//! ```text
//! beatscript-serve migrate-rename --data <dir> --backup <file.tar.gz> [--examples <dir>] [--dry-run] [--json]
//! ```
//!
//! Every stored BeatScript 1 program X (text T) gets its BeatScript 2 twin Y = `program_cid(upgrade_canonical(T))`, written
//! as `programs/<Y>.json` through the storage gate (`gate::write_twin`: `gate::admit`, then `gate::write_program`) with X's
//! meta and `forked_from: X`, `derivation: "renamed"`, `source: "renamed"`, `renamed_at`. X's file is never touched (the
//! store is append-only); a text that names other programs is upgraded as text only (old CIDs still resolve). Then:
//!
//! - playlists: an item whose CID has a twin names the twin, with `renamed_from` kept on the item (through the playlists
//!   store's own `update`, which writes each account file atomically);
//! - the render cache: `render/<X>.<sr>.r2.wav` (+ `.meta`) is hard-linked to `render/<Y>...` (a twin plays the same: the
//!   sound is seeded from the legacy CID), or a `.meta` that names X is copied with Y in its place. Opus files are left to
//!   re-render (their Ogg stream serial is derived from the CID, so a link would not be the file the twin renders to), as
//!   are the export cache (MIDI and stems carry the CID and rights) and the share cards (they print the CID).
//!
//! Order, and what makes it safe:
//!
//! 1. Everything is planned read-only first. A run with nothing to do writes nothing at all (not even a backup) and says so:
//!    a second run is a no-op.
//! 2. Without `--dry-run`, `--backup` is required. The backup (`programs/`, `playlists/` and the data directory's small JSON
//!    state files; never `render/`, `cards/` or the logs) is written first, outside the data directory, never over an
//!    existing file, mode 0600; it is read back and verified by content (the sha256 of every file's bytes against the file
//!    on disk, names in byte order as `LC_ALL=C` sorts them) before anything else is written.
//! 3. At the end it checks itself: every BeatScript 1 program has a stored twin whose legacy CID is the original, every
//!    twin verifies, the twin count equals the BeatScript 1 count, and no playlist item still names a program whose twin is
//!    stored. Anything else exits 1 with the reasons.
//!
//! `--dry-run` writes nothing anywhere and prints the plan (`--json`: as one JSON object): programs seen, BeatScript 1,
//! BeatScript 2 already, twins present and to write, what the gate would refuse, unverifiable files, the playlists and items
//! to rewrite, the render files to link, the cards left to regenerate, and the old -> new map. It exits 1 (`"ok": false`)
//! when the real run would fail its self-check (a twin the gate refuses, or a file at a twin's name that is not the twin),
//! so a deploy can run it first as a pre-flight.
//!
//! `--examples` (the server's `-examples`) lets a program that imports an example by its legacy CID check, and maps
//! playlist items that name an example by its legacy CID.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beatscript_serve::gate;
use beatscript_serve::playlists::model::AccountFile;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::Store;

const USAGE: &str = "usage: beatscript-serve migrate-rename --data <dir> --backup <file.tar.gz> [--examples <dir>] [--dry-run] [--json]";

#[derive(Debug, Default)]
struct Args {
    data: PathBuf,
    backup: Option<PathBuf>,
    examples: Option<PathBuf>,
    dry_run: bool,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut data = None;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].trim_start_matches('-');
        if raw == args[i] {
            return Err(format!("unexpected argument {:?}", args[i]));
        }
        let (k, inline) = match raw.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (raw, None),
        };
        let mut value = || -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => {
                    i += 1;
                    args.get(i).cloned().ok_or_else(|| format!("--{k} needs a value"))
                }
            }
        };
        match k {
            "data" => data = Some(PathBuf::from(value()?)),
            "backup" => a.backup = Some(PathBuf::from(value()?)),
            "examples" => a.examples = Some(PathBuf::from(value()?)).filter(|p| !p.as_os_str().is_empty()),
            "dry-run" => a.dry_run = inline.as_deref().is_none_or(|v| matches!(v, "true" | "1" | "yes")),
            "json" => a.json = inline.as_deref().is_none_or(|v| matches!(v, "true" | "1" | "yes")),
            _ => return Err(format!("unknown flag --{k}")),
        }
        i += 1;
    }
    a.data = data.ok_or("--data <dir> is required")?;
    if !a.dry_run && a.backup.is_none() {
        return Err("refusing to migrate without a backup: give --backup <file.tar.gz> (outside the data directory), or --dry-run".into());
    }
    Ok(a)
}

/// The subcommand: its exit code (0 done or nothing to do, 1 failed or the self-check failed, 2 refused before starting).
pub fn main(args: &[String]) -> i32 {
    let a = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("beatscript-serve migrate-rename: {e}\n{USAGE}");
            return 2;
        }
    };
    let (code, report) = match run(&a) {
        Ok(r) => (if r["ok"] == true { 0 } else { 1 }, r),
        Err(Fail { code, message, report }) => {
            eprintln!("beatscript-serve migrate-rename: {message}");
            let mut r = report.unwrap_or_else(|| json!({}));
            r["ok"] = json!(false);
            r["error"] = json!(message);
            (code, r)
        }
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print!("{}", human(&report));
    }
    code
}

struct Fail {
    code: i32,
    message: String,
    report: Option<Value>,
}

fn fail(code: i32, message: impl Into<String>) -> Fail {
    Fail { code, message: message.into(), report: None }
}

// ---------- reading the store ----------

/// A stored file whose text hashes to its CID.
#[derive(Clone, Debug)]
struct Stored {
    cid: String,
    text: String,
    meta: Value,
}

#[derive(Default)]
struct Scan {
    seen: usize,
    v1: Vec<Stored>,
    /// BeatScript 2 programs that verify, by CID.
    v2: BTreeMap<String, Stored>,
    unverifiable: Vec<(String, String)>,
}

/// The `.json` files of the programs directory (not dotfiles: a write in flight), in byte order.
fn json_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") && !n.starts_with('.')).collect();
    names.sort();
    Ok(names)
}

/// A stored file as the store reads one (`Store::get`): the JSON's `cid` is its name and its text hashes to it.
fn read_stored(dir: &Path, name: &str) -> Result<Stored, String> {
    let cid = name.strip_suffix(".json").unwrap_or(name);
    if !crate::store::is_cid(cid) {
        return Err("the file name is not a CID".into());
    }
    let b = std::fs::read(dir.join(name)).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&b).map_err(|_| "not JSON".to_string())?;
    if v["cid"].as_str() != Some(cid) {
        return Err("its cid is not its file name".into());
    }
    let text = v["text"].as_str().filter(|t| !t.is_empty()).ok_or("it has no text")?;
    if djlive::compile::program_cid(text) != cid {
        return Err("its text does not hash to its CID".into());
    }
    Ok(Stored { cid: cid.to_string(), text: text.to_string(), meta: v.get("meta").cloned().unwrap_or(Value::Null) })
}

fn scan(dir: &Path) -> std::io::Result<Scan> {
    let mut s = Scan::default();
    for n in json_names(dir)? {
        s.seen += 1;
        match read_stored(dir, &n) {
            Err(why) => s.unverifiable.push((n, why)),
            Ok(p) => match djlive::version::of_canonical(&p.text) {
                djlive::version::Version::V1 => s.v1.push(p),
                djlive::version::Version::V2 => {
                    s.v2.insert(p.cid.clone(), p);
                }
            },
        }
    }
    Ok(s)
}

/// What a check made here resolves a program's `import` and borrowed parts against: the store, as the server does.
struct Source(Arc<Store>);

impl djlive::env::Snippets for Source {
    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        match self.0.get(query) {
            Some(p) => Ok((p.cid, p.text)),
            None => Err(djlive::env::not_here(query)),
        }
    }
}

// ---------- the plan ----------

enum TwinState {
    Present,
    ToWrite,
    /// The gate would refuse the twin, or its file is there and is not that program.
    Refused(String),
}

struct Plan {
    scan: Scan,
    /// Each BeatScript 1 program, its twin's CID and what is to be done.
    twins: Vec<(Stored, String, TwinState)>,
    /// Old -> new, for every BeatScript 1 program that has (or will have) its twin stored.
    map: BTreeMap<String, String>,
    /// The examples' legacy CIDs -> their CIDs.
    example_map: BTreeMap<String, String>,
    playlists: PlaylistPlan,
    render: RenderPlan,
    cards_left: usize,
}

#[derive(Default)]
struct PlaylistPlan {
    files: usize,
    /// owner uid -> (playlists, items) to rewrite
    accounts: BTreeMap<String, (usize, usize)>,
    unreadable: Vec<String>,
}

#[derive(Default)]
struct RenderPlan {
    files: usize,
    /// (source, target, rewrite: the `.meta` names the old CID)
    jobs: Vec<(String, String, bool)>,
    already: usize,
    /// Files of programs with a twin that are left to be made again (Opus, exports, partial renders).
    left: usize,
}

impl Plan {
    fn twin_of(&self, cid: &str) -> Option<&String> {
        self.map.get(cid).or_else(|| self.example_map.get(cid))
    }
    fn count(&self, f: impl Fn(&TwinState) -> bool) -> usize {
        self.twins.iter().filter(|t| f(&t.2)).count()
    }
    fn nothing_to_do(&self) -> bool {
        self.count(|s| matches!(s, TwinState::ToWrite)) == 0 && self.playlists.accounts.is_empty() && self.render.jobs.is_empty()
    }
}

fn plan(data: &Path, store: &Store) -> Result<Plan, String> {
    let programs = data.join("programs");
    let scan = scan(&programs).map_err(|e| format!("{}: {e}", programs.display()))?;
    let mut twins = Vec::new();
    let mut map = BTreeMap::new();
    for x in &scan.v1 {
        let y = djlive::compile::renamed_cid(&x.text);
        let state = if scan.v2.contains_key(&y) {
            TwinState::Present
        } else if programs.join(format!("{y}.json")).exists() {
            TwinState::Refused(format!("programs/{y}.json is there and is not the twin (it does not verify)"))
        } else {
            match gate::admit_twin(&x.cid, &x.text, &x.meta) {
                Ok(_) => TwinState::ToWrite,
                Err(r) => TwinState::Refused(format!("{}: {}", r.code, r.message)),
            }
        };
        if !matches!(state, TwinState::Refused(_)) {
            map.insert(x.cid.clone(), y.clone());
        }
        twins.push((x.clone(), y, state));
    }
    let example_map: BTreeMap<String, String> = store.example_cids().filter_map(|c| store.renamed_to(&c).map(|y| (c, y))).collect();
    let mut p = Plan { scan, twins, map, example_map, playlists: PlaylistPlan::default(), render: RenderPlan::default(), cards_left: 0 };
    p.playlists = plan_playlists(&data.join("playlists"), &p);
    p.render = plan_render(&data.join("render"), &p.map);
    p.cards_left = std::fs::read_dir(data.join("cards")).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().split('.').next().is_some_and(|c| p.map.contains_key(c))).count()).unwrap_or(0);
    Ok(p)
}

fn account_files(dir: &Path) -> Vec<(String, Result<AccountFile, String>)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") && !n.starts_with('.')).collect();
    names.sort();
    names.into_iter().map(|n| {
        let f = std::fs::read(dir.join(&n)).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<AccountFile>(&b).map_err(|e| e.to_string()));
        (n, f)
    }).collect()
}

fn plan_playlists(dir: &Path, p: &Plan) -> PlaylistPlan {
    let mut out = PlaylistPlan::default();
    for (name, f) in account_files(dir) {
        out.files += 1;
        let f = match f {
            Ok(f) => f,
            Err(e) => {
                out.unreadable.push(format!("{name}: {e}"));
                continue;
            }
        };
        let (mut lists, mut items) = (0, 0);
        for pl in &f.playlists {
            let n = pl.items.iter().filter(|it| p.twin_of(&it.cid).is_some()).count();
            if n > 0 {
                lists += 1;
                items += n;
            }
        }
        if items > 0 {
            out.accounts.insert(f.owner.clone(), (lists, items));
        }
    }
    out
}

/// A WAV render's cache name (`<cid>.<sr>.r2.wav`), or its `.meta`.
fn is_wav_render(name: &str) -> bool {
    let base = name.strip_suffix(".meta").unwrap_or(name);
    base.ends_with(".wav") && crate::service::is_cache_name(base)
}

fn plan_render(dir: &Path, map: &BTreeMap<String, String>) -> RenderPlan {
    let mut out = RenderPlan::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut names: Vec<String> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for n in names {
        out.files += 1;
        let Some((cid, rest)) = n.split_once('.') else { continue };
        let Some(y) = map.get(cid) else { continue };
        if !is_wav_render(&n) {
            out.left += 1;
            continue;
        }
        let target = format!("{y}.{rest}");
        if dir.join(&target).exists() {
            out.already += 1;
            continue;
        }
        let rewrite = n.ends_with(".meta") && std::fs::read(dir.join(&n)).is_ok_and(|b| String::from_utf8_lossy(&b).contains(cid));
        out.jobs.push((n, target, rewrite));
    }
    out
}

// ---------- the backup ----------

/// What goes in the backup, as (name in the archive, path): `programs/*`, `playlists/*` and the data directory's `*.json`.
fn backup_files(data: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for sub in ["programs", "playlists"] {
        let Ok(rd) = std::fs::read_dir(data.join(sub)) else { continue };
        for e in rd.flatten() {
            if e.file_type()?.is_file() {
                let n = e.file_name().to_string_lossy().into_owned();
                out.push((format!("{sub}/{n}"), e.path()));
            }
        }
    }
    for e in std::fs::read_dir(data)?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if e.file_type()?.is_file() && n.ends_with(".json") {
            out.push((n, e.path()));
        }
    }
    // byte order, as LC_ALL=C sorts
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(out)
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// The digest of a set of files: sha256 over `<sha256>  <name>\n` lines in byte order of name (what `sha256sum` prints,
/// sorted under `LC_ALL=C`, then hashed).
fn manifest_digest(files: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    for (n, s) in files {
        h.update(format!("{s}  {n}\n").as_bytes());
    }
    hex::encode(h.finalize())
}

/// The entries of a ustar archive (`opendata::tar`'s shape), by name.
fn untar(raw: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    let mut at = 0;
    while at + 512 <= raw.len() && raw[at] != 0 {
        let h = &raw[at..at + 512];
        let name = String::from_utf8(h[..100].iter().take_while(|c| **c != 0).copied().collect()).map_err(|_| "a name is not UTF-8")?;
        let size = std::str::from_utf8(&h[124..135]).ok().and_then(|s| usize::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).ok()).ok_or("a size does not read")?;
        let body = raw.get(at + 512..at + 512 + size).ok_or("the archive is cut short")?;
        if out.insert(name.clone(), body.to_vec()).is_some() {
            return Err(format!("{name} is in the archive twice"));
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// Write the backup, read it back and verify it by content. Nothing else is written before this returns Ok.
fn backup(data: &Path, out: &Path) -> Result<Value, String> {
    let data_c = data.canonicalize().map_err(|e| format!("{}: {e}", data.display()))?;
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent_c = parent.canonicalize().map_err(|e| format!("the backup's directory {}: {e}", parent.display()))?;
    if parent_c.starts_with(&data_c) {
        return Err(format!("the backup {} is inside the data directory; put it outside", out.display()));
    }
    if out.exists() {
        return Err(format!("{} is there already; not writing over a backup", out.display()));
    }
    let files = backup_files(data).map_err(|e| format!("reading {}: {e}", data.display()))?;
    let mut entries = Vec::with_capacity(files.len());
    let mut want = BTreeMap::new();
    for (name, path) in &files {
        if name.len() > 100 {
            return Err(format!("{name}: the name is too long for the archive"));
        }
        let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        want.insert(name.clone(), sha256_hex(&b));
        entries.push((name.clone(), b));
    }
    let gz = beatscript_serve::opendata::gzip(&beatscript_serve::opendata::tar(&entries));
    drop(entries);
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(out).map_err(|e| format!("{}: {e}", out.display()))?;
        f.write_all(&gz).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    // read back: gunzip (header, CRC, size) and untar
    let back = std::fs::read(out).map_err(|e| format!("reading the backup back: {e}"))?;
    if back.len() < 18 || back[..3] != [0x1f, 0x8b, 8] {
        return Err("the backup read back is not gzip".into());
    }
    let raw = miniz_oxide::inflate::decompress_to_vec(&back[10..back.len() - 8]).map_err(|e| format!("the backup does not inflate: {e:?}"))?;
    let n = back.len();
    if back[n - 8..n - 4] != beatscript_serve::opendata::crc32(&raw).to_le_bytes() || back[n - 4..] != (raw.len() as u32).to_le_bytes() {
        return Err("the backup's CRC or size is wrong".into());
    }
    let got = untar(&raw)?;
    let got: BTreeMap<String, String> = got.iter().map(|(k, v)| (k.clone(), sha256_hex(v))).collect();
    if got.keys().collect::<Vec<_>>() != want.keys().collect::<Vec<_>>() {
        return Err(format!("the backup holds {} files, not the {} it should", got.len(), want.len()));
    }
    // every file's bytes against the file on disk now
    for (name, path) in &files {
        let now = std::fs::read(path).map(|b| sha256_hex(&b)).map_err(|e| format!("{}: {e}", path.display()))?;
        if got.get(name) != Some(&now) {
            return Err(format!("{name}: the backup's copy is not the file on disk (did it change while the backup was made? run again)"));
        }
    }
    Ok(json!({
        "path": out.display().to_string(),
        "files": got.len(),
        "programs": got.keys().filter(|k| k.starts_with("programs/")).count(),
        "playlists": got.keys().filter(|k| k.starts_with("playlists/")).count(),
        "bytes": back.len(),
        "sha256": sha256_hex(&back),
        "manifest_sha256": manifest_digest(&got),
        "verified": true,
    }))
}

// ---------- the writes ----------

fn write_twins(programs: &Path, p: &Plan, at: &str) -> (usize, Vec<String>) {
    let (mut wrote, mut errors) = (0, Vec::new());
    for (x, y, st) in &p.twins {
        if !matches!(st, TwinState::ToWrite) {
            continue;
        }
        match gate::write_twin(programs, &x.cid, &x.text, &x.meta, at, crate::go_json) {
            Ok((cid, Some(_))) if &cid == y => wrote += 1,
            Ok((cid, None)) if &cid == y => {} // stored meanwhile (the server writes twins too)
            Ok((cid, _)) => errors.push(format!("{}: its twin was stored as {cid}, not {y}", x.cid)),
            Err(r) => errors.push(format!("{}: {}", x.cid, r.message)),
        }
    }
    (wrote, errors)
}

fn rewrite_playlists(data: &Path, p: &Plan, at: &str) -> Result<(usize, usize), String> {
    if p.playlists.accounts.is_empty() {
        return Ok((0, 0));
    }
    use beatscript_serve::playlists::model::Limits;
    // the migration rewrites what is there: the per-account caps are for what people add
    let limits = Limits { max_playlists: usize::MAX, max_items: usize::MAX, max_account_bytes: usize::MAX, max_total_bytes: u64::MAX, writes_per_min: usize::MAX, requests_per_min: usize::MAX, resolve_per_min: usize::MAX };
    let (store, _) = beatscript_serve::playlists::store::Store::open(data, limits).map_err(|e| format!("playlists: {e}"))?;
    let (mut lists, mut items) = (0, 0);
    for uid in p.playlists.accounts.keys() {
        let (l, i) = store
            .update(uid, |f| {
                let (mut l, mut i) = (0, 0);
                for pl in &mut f.playlists {
                    let mut changed = 0;
                    for it in &mut pl.items {
                        if let Some(y) = p.twin_of(&it.cid) {
                            it.renamed_from = Some(std::mem::replace(&mut it.cid, y.clone()));
                            changed += 1;
                        }
                    }
                    if changed > 0 {
                        pl.updated = at.to_string();
                        l += 1;
                        i += changed;
                    }
                }
                Ok((l, i))
            })
            .map_err(|e| format!("playlists of one account: {}", e.message))?;
        lists += l;
        items += i;
    }
    Ok((lists, items))
}

fn link_renders(dir: &Path, p: &Plan) -> (usize, usize, Vec<String>) {
    let (mut linked, mut rewritten, mut errors) = (0, 0, Vec::new());
    for (src, dst, rewrite) in &p.render.jobs {
        let (from, to) = (dir.join(src), dir.join(dst));
        let res = if *rewrite {
            // a `.meta` that names the old CID: a copy that names the new one (temp file, then rename)
            let x = src.split('.').next().unwrap_or("");
            let y = dst.split('.').next().unwrap_or("");
            std::fs::read(&from).and_then(|b| {
                let tmp = dir.join(format!(".{dst}.migrate-tmp"));
                std::fs::write(&tmp, String::from_utf8_lossy(&b).replace(x, y))?;
                std::fs::rename(&tmp, &to)
            })
        } else {
            std::fs::hard_link(&from, &to)
        };
        match res {
            Ok(()) if *rewrite => rewritten += 1,
            Ok(()) => linked += 1,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => errors.push(format!("render/{dst}: {e}")),
        }
    }
    (linked, rewritten, errors)
}

// ---------- the self-check ----------

fn self_check(data: &Path, example_map: &BTreeMap<String, String>) -> Value {
    let programs = data.join("programs");
    let s = match scan(&programs) {
        Ok(s) => s,
        Err(e) => return json!({"ok": false, "problems": [format!("{}: {e}", programs.display())]}),
    };
    let mut problems = Vec::new();
    let v1: BTreeSet<&str> = s.v1.iter().map(|p| p.cid.as_str()).collect();
    let mut map = BTreeMap::new();
    for x in &s.v1 {
        let y = djlive::compile::renamed_cid(&x.text);
        match s.v2.get(&y) {
            Some(t) if djlive::compile::renamed_cid(&t.text) == x.cid => {
                map.insert(x.cid.clone(), y);
            }
            Some(_) => problems.push(format!("{}: its twin {y}'s legacy CID is not it", x.cid)),
            None if programs.join(format!("{y}.json")).exists() => problems.push(format!("{}: its twin programs/{y}.json does not verify", x.cid)),
            None => problems.push(format!("{}: no twin stored ({y})", x.cid)),
        }
    }
    // a twin is a BeatScript 2 program whose legacy CID is a stored BeatScript 1 program
    let twins = s.v2.values().filter(|t| v1.contains(djlive::compile::renamed_cid(&t.text).as_str())).count();
    if twins != s.v1.len() {
        problems.push(format!("{twins} twins for {} BeatScript 1 programs", s.v1.len()));
    }
    let mut stale = 0;
    for (name, f) in account_files(&data.join("playlists")) {
        let Ok(f) = f else { continue };
        for pl in &f.playlists {
            for it in &pl.items {
                if map.contains_key(&it.cid) || example_map.contains_key(&it.cid) {
                    stale += 1;
                    if stale <= 20 {
                        problems.push(format!("playlists/{name}: an item still names {}", it.cid));
                    }
                }
            }
        }
    }
    json!({"ok": problems.is_empty(), "v1": s.v1.len(), "twins": twins, "v2": s.v2.len(), "problems": problems})
}

// ---------- the run ----------

fn report_of(a: &Args, p: &Plan) -> Value {
    let s = &p.scan;
    let refused: Vec<Value> = p.twins.iter().filter_map(|(x, y, st)| match st {
        TwinState::Refused(why) => Some(json!({"cid": x.cid, "twin": y, "reason": why})),
        _ => None,
    }).collect();
    json!({
        "dry_run": a.dry_run,
        "data": a.data.display().to_string(),
        "programs_seen": s.seen,
        "v1": s.v1.len(),
        "v2_already": s.v2.len(),
        "twins_present": p.count(|s| matches!(s, TwinState::Present)),
        "twins_to_write": p.count(|s| matches!(s, TwinState::ToWrite)),
        "twins_refused": refused,
        "unverifiable": s.unverifiable.iter().map(|(f, why)| json!({"file": f, "reason": why})).collect::<Vec<_>>(),
        "playlists": {
            "files": p.playlists.files,
            "accounts_to_rewrite": p.playlists.accounts.len(),
            "playlists_to_rewrite": p.playlists.accounts.values().map(|v| v.0).sum::<usize>(),
            "items_to_rewrite": p.playlists.accounts.values().map(|v| v.1).sum::<usize>(),
            "unreadable": p.playlists.unreadable,
        },
        "render": {
            "files": p.render.files,
            "to_link": p.render.jobs.iter().filter(|j| !j.2).count(),
            "to_rewrite_meta": p.render.jobs.iter().filter(|j| j.2).count(),
            "already": p.render.already,
            "left_to_rerender": p.render.left,
        },
        "cards_left": p.cards_left,
        "examples_legacy_cids": p.example_map.len(),
        "map": p.map,
        "example_map": p.example_map,
        "ok": refused.is_empty(),
    })
}

fn run(a: &Args) -> Result<Value, Fail> {
    let programs = a.data.join("programs");
    if !programs.is_dir() {
        return Err(fail(2, format!("{} is not a directory: is --data the server's data directory?", programs.display())));
    }
    // read-only, and every check made here resolves against the store (and the examples), as the server's do
    let (store, warnings) = Store::open(&programs, a.examples.as_deref(), true).map_err(|e| fail(1, e.to_string()))?;
    for w in warnings {
        eprintln!("beatscript-serve migrate-rename: warning: {w}");
    }
    let store = Arc::new(store);
    beatscript_api::known::set_source(Arc::new(Source(store.clone())));
    let p = plan(&a.data, &store).map_err(|e| fail(1, e))?;
    let mut r = report_of(a, &p);
    if a.dry_run {
        return Ok(r);
    }
    if p.nothing_to_do() {
        let check = self_check(&a.data, &p.example_map);
        r["nothing_to_do"] = json!(true);
        r["wrote"] = json!({"twins": 0, "playlists": 0, "items": 0, "render_links": 0, "render_meta_rewrites": 0});
        r["ok"] = json!(check["ok"] == true);
        r["self_check"] = check;
        return Ok(r);
    }
    r["nothing_to_do"] = json!(false);
    let backup_path = a.backup.as_ref().expect("parse_args requires it");
    match backup(&a.data, backup_path) {
        Ok(b) => r["backup"] = b,
        Err(e) => return Err(Fail { code: 1, message: format!("backup: {e}; nothing was migrated"), report: Some(r) }),
    }
    let at = crate::store::rfc3339_now();
    let (twins, mut errors) = write_twins(&programs, &p, &at);
    let (lists, items) = match rewrite_playlists(&a.data, &p, &at) {
        Ok(x) => x,
        Err(e) => {
            errors.push(e);
            (0, 0)
        }
    };
    let (linked, rewritten, render_errors) = link_renders(&a.data.join("render"), &p);
    errors.extend(render_errors);
    r["wrote"] = json!({"twins": twins, "playlists": lists, "items": items, "render_links": linked, "render_meta_rewrites": rewritten});
    let check = self_check(&a.data, &p.example_map);
    r["ok"] = json!(errors.is_empty() && check["ok"] == true);
    r["errors"] = json!(errors);
    r["self_check"] = check;
    Ok(r)
}

/// The report as lines a person reads.
fn human(r: &Value) -> String {
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let len = |v: &Value| v.as_array().map_or(0, Vec::len);
    let mut s = String::new();
    if let Some(e) = r["error"].as_str() {
        s.push_str(&format!("error: {e}\n"));
    }
    if r.get("programs_seen").is_none() {
        return s;
    }
    s.push_str(&format!(
        "{}programs: {} seen, {} BeatScript 1, {} BeatScript 2 already, {} unverifiable\n",
        if r["dry_run"] == true { "dry run (nothing written): " } else { "" },
        n(&r["programs_seen"]),
        n(&r["v1"]),
        n(&r["v2_already"]),
        len(&r["unverifiable"])
    ));
    s.push_str(&format!("twins: {} present, {} to write, {} refused\n", n(&r["twins_present"]), n(&r["twins_to_write"]), len(&r["twins_refused"])));
    for t in r["twins_refused"].as_array().into_iter().flatten() {
        s.push_str(&format!("  refused {}: {}\n", t["cid"].as_str().unwrap_or(""), t["reason"].as_str().unwrap_or("")));
    }
    let pl = &r["playlists"];
    s.push_str(&format!("playlists: {} accounts, {} playlists, {} items to rewrite\n", n(&pl["accounts_to_rewrite"]), n(&pl["playlists_to_rewrite"]), n(&pl["items_to_rewrite"])));
    let rd = &r["render"];
    s.push_str(&format!("render cache: {} to link, {} .meta to rewrite, {} already, {} left to re-render; {} cards left to regenerate\n", n(&rd["to_link"]), n(&rd["to_rewrite_meta"]), n(&rd["already"]), n(&rd["left_to_rerender"]), n(&r["cards_left"])));
    if r["nothing_to_do"] == true {
        s.push_str("nothing to do: every BeatScript 1 program already has its twin; nothing was written\n");
    }
    if let Some(b) = r.get("backup") {
        s.push_str(&format!("backup: {} ({} files, {} bytes, sha256 {}), verified by content\n", b["path"].as_str().unwrap_or(""), n(&b["files"]), n(&b["bytes"]), b["sha256"].as_str().unwrap_or("")));
    }
    if let Some(w) = r.get("wrote") {
        s.push_str(&format!("wrote: {} twins, {} playlists ({} items), {} render links, {} .meta rewrites\n", n(&w["twins"]), n(&w["playlists"]), n(&w["items"]), n(&w["render_links"]), n(&w["render_meta_rewrites"])));
    }
    for e in r["errors"].as_array().into_iter().flatten() {
        s.push_str(&format!("  error: {}\n", e.as_str().unwrap_or("")));
    }
    if let Some(c) = r.get("self_check") {
        s.push_str(&format!("self-check: {} ({} BeatScript 1, {} twins)\n", if c["ok"] == true { "ok" } else { "FAILED" }, n(&c["v1"]), n(&c["twins"])));
        for p in c["problems"].as_array().into_iter().flatten() {
            s.push_str(&format!("  {}\n", p.as_str().unwrap_or("")));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let a = |v: &[&str]| parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(a(&["--data", "d"]).unwrap_err().contains("without a backup"));
        assert!(a(&["--data", "d", "--dry-run"]).unwrap().dry_run);
        let x = a(&["-data=d", "--backup", "/tmp/b.tar.gz", "--json", "--examples", "site/examples"]).unwrap();
        assert_eq!((x.data, x.backup, x.examples, x.json, x.dry_run), (PathBuf::from("d"), Some(PathBuf::from("/tmp/b.tar.gz")), Some(PathBuf::from("site/examples")), true, false));
        assert!(a(&["--dry-run"]).unwrap_err().contains("--data"));
        assert!(a(&["--data", "d", "--dry-run", "--nope"]).unwrap_err().contains("unknown flag"));
        assert!(a(&["--data", "d", "--dry-run", "stray"]).unwrap_err().contains("unexpected"));
    }

    #[test]
    fn untar_reads_what_opendata_writes() {
        let files = vec![("programs/a.json".to_string(), b"{}".to_vec()), ("site.json".to_string(), vec![7u8; 1500])];
        let got = untar(&beatscript_serve::opendata::tar(&files)).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got["site.json"], vec![7u8; 1500]);
        assert_eq!(got["programs/a.json"], b"{}");
    }

    #[test]
    fn wav_renders_only() {
        let c = "z4EBG9izF5J75dpqKKpJ7KeFUeJfSkHoefqEUsSAPWGbcgBdjDx";
        assert!(is_wav_render(&format!("{c}.48000.r2.wav")));
        assert!(is_wav_render(&format!("{c}.48000.r2.wav.meta")));
        assert!(!is_wav_render(&format!("{c}.48000.r2rs.opus")));
        assert!(!is_wav_render(&format!("{c}.48000.r2rs.opus.meta")));
        assert!(!is_wav_render(&format!("{c}.48000.r2.wav.rs.part")));
    }
}
