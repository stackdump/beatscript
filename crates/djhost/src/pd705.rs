//! PD705 kits through the `pd705` command (pd705-rs) as a separate process (docs/HUB.md, M8).
//!
//! The same route as the TD-3's (`crate::td3`): djrust does not link pd705-rs, which declares
//! MIT in its manifest but ships no LICENSE file (it is Matt's own, but a dependency waits for
//! the file). It runs `pd705 build`, which converts the WAVs and writes `pad-content.hppd`,
//! and `pd705 build --to-pi`, whose script detaches the Pi Zero's USB gadget, loop-mounts
//! `/piusb.bin`, backs up, copies, checks the sha256 and reattaches. Everything here is std
//! only and portable.
//!
//! **Nothing real happens by default.** [`install`] with `really == false` builds the file,
//! asks the installer for its dry-run plan (no connection is opened) and checks that the
//! plan names the sha256 of the very bytes the hub built. A real run does the dry run first,
//! in the same job, then installs, then reads the installer's own `sha256 ok`.
//!
//! The commands go through a [`Runner`] so tests drive the pipeline with a fake and never
//! touch a network.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

use djengine::cid::sha256;
use djengine::gear::pd705_kit::{parse_build_output, BuildReport, KitBook, KitDraft};
use djengine::gear::pd705_load::{install_succeeded, parse_install_output, InstallReport, Record};

use crate::stores::{read_json, support_dir, write_atomic};

/// A built file larger than this has not been tried on the pad (2026-11-04: the largest it
/// imported was the 2.94 MB Acid House kit; pd705-rs docs/FORMAT.md 6.1).
pub const UNTESTED_BYTES: u64 = 3_500_000;

/// The name the pad imports: only this exact file name, in the stick's root.
pub const HPPD_NAME: &str = "pad-content.hppd";

/// Where `pd705` is: `$PD705_BIN`, then `pd705` on `PATH`, then a pd705-rs release build next
/// to this checkout (`~/Workspace/pd705-rs/target/release/pd705`).
pub fn find_pd705() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PD705_BIN").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join("pd705");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let p = home.join("Workspace/pd705-rs/target/release/pd705");
    p.is_file().then_some(p)
}

/// The Pi to install on: `$PD705_PI_HOST`, else the book's.
pub fn pi_host(book: &KitBook) -> String {
    std::env::var("PD705_PI_HOST")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| book.host.clone())
}

/// What a command printed and whether it succeeded.
#[derive(Clone, Debug, PartialEq)]
pub struct RunOut {
    pub ok: bool,
    /// stdout then stderr.
    pub output: String,
}

/// Runs the `pd705` command. The real one is [`Process`]; tests pass their own.
pub trait Runner {
    fn run(&self, bin: &Path, args: &[String]) -> RunOut;
}

/// `std::process::Command`.
pub struct Process;

impl Runner for Process {
    fn run(&self, bin: &Path, args: &[String]) -> RunOut {
        match Command::new(bin).args(args).output() {
            Ok(o) => RunOut {
                ok: o.status.success(),
                output: format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
            },
            Err(e) => RunOut {
                ok: false,
                output: format!("could not run {}: {e}", bin.display()),
            },
        }
    }
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// sha256 of a file, hex.
pub fn file_sha256(path: &Path) -> io::Result<String> {
    Ok(hex(&sha256(&std::fs::read(path)?)))
}

/// A kit name as a folder name.
pub fn safe_name(kit: &str) -> String {
    kit.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A folder only this call uses (process, kit, and a counter), for an install's throwaway
/// build: two jobs, or two tests, never share one.
fn staging_dir(kit: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "djrust-pd705-{}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        safe_name(kit)
    ))
}

/// A folder removed when it goes out of scope.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a build came to.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildResult {
    pub ok: bool,
    pub kit: String,
    /// The built file.
    pub path: Option<PathBuf>,
    /// Its sha256, hex: what the Kits window shows and an install checks.
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
    pub report: BuildReport,
    /// Things worth a look that did not stop the build.
    pub warnings: Vec<String>,
    /// The command's own words, or why it never ran.
    pub output: String,
}

impl BuildResult {
    fn fail(kit: &str, output: impl Into<String>) -> BuildResult {
        BuildResult {
            ok: false,
            kit: kit.to_string(),
            path: None,
            sha256: None,
            bytes: None,
            report: BuildReport::default(),
            warnings: Vec::new(),
            output: output.into(),
        }
    }
}

/// Look at the sample files before running anything: every one must exist.
fn missing_samples(draft: &KitDraft) -> Vec<String> {
    draft
        .pads
        .iter()
        .filter(|p| !Path::new(&p.path).is_file())
        .map(|p| {
            format!(
                "{}: {} is not a file",
                djengine::gear::pd705::input_name(p.input),
                p.path
            )
        })
        .collect()
}

/// Build `draft` into `dir/pad-content.hppd` and hash it.
pub fn build_in(runner: &dyn Runner, bin: &Path, draft: &KitDraft, dir: &Path) -> BuildResult {
    if let Err(e) = draft.validate() {
        return BuildResult::fail(&draft.name, e.join("\n"));
    }
    let missing = missing_samples(draft);
    if !missing.is_empty() {
        return BuildResult::fail(&draft.name, missing.join("\n"));
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        return BuildResult::fail(&draft.name, format!("{}: {e}", dir.display()));
    }
    let out = dir.join(HPPD_NAME);
    let r = runner.run(bin, &draft.build_args(Some(&out.display().to_string())));
    if !r.ok {
        return BuildResult::fail(&draft.name, r.output);
    }
    let bytes = match std::fs::read(&out) {
        Ok(b) => b,
        Err(e) => {
            return BuildResult::fail(
                &draft.name,
                format!("{}\npd705 said it built, but {}: {e}", r.output, out.display()),
            )
        }
    };
    let report = parse_build_output(&r.output);
    let mut warnings = Vec::new();
    if let Some((_, n)) = &report.wrote {
        if *n != bytes.len() as u64 {
            warnings.push(format!(
                "pd705 reported {n} bytes, the file has {}",
                bytes.len()
            ));
        }
    }
    // the builder knows each sound's real length: a pad the hub took for a one-shot that is a
    // loop (or the other way) would be played wrongly
    for p in &draft.pads {
        if let Some((_, secs)) = report.sounds.iter().find(|(n, _)| *n == p.name) {
            let by_length = *secs >= djengine::gear::pd705_kit::LOOP_SECONDS;
            if p.loop_mode.is_none() && by_length != p.is_loop() {
                warnings.push(format!(
                    "{}: {:.1} s on the pad, recorded as {:.1} s: check its loop flag",
                    djengine::gear::pd705::input_name(p.input),
                    secs,
                    p.seconds
                ));
            }
        }
    }
    if bytes.len() as u64 > UNTESTED_BYTES {
        warnings.push(format!(
            "{:.1} MB is bigger than any kit the pad has accepted (the Acid House kit, 2.9 MB): \
             the pad's sample memory is not documented, and an import that does not fit drops the excess",
            bytes.len() as f64 / 1e6
        ));
    }
    BuildResult {
        ok: true,
        kit: draft.name.clone(),
        sha256: Some(hex(&sha256(&bytes))),
        bytes: Some(bytes.len() as u64),
        path: Some(out),
        report,
        warnings,
        output: r.output,
    }
}

/// How an install went.
#[derive(Clone, Debug, PartialEq)]
pub struct InstallResult {
    pub ok: bool,
    /// Whether this was a real install that went through (false: a dry run, or a failure).
    pub installed: bool,
    pub dry_run: bool,
    pub kit: String,
    pub host: String,
    /// The hub's own sha256 of the built file.
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
    pub report: InstallReport,
    /// The kit as the tracker keeps it, set on a real success.
    pub record: Option<Record>,
    pub warnings: Vec<String>,
    /// Step by step, in words.
    pub steps: Vec<String>,
    pub output: String,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Install `draft` on the Pi at `host`. `really == false` is the default everywhere: build,
/// plan, compare, stop. `really == true` goes on to run the installer for real and reads its
/// answer. The caller has already decided the pad is idle (`pd705_load::install_blockers`).
pub fn install(
    runner: &dyn Runner,
    bin: &Path,
    draft: &KitDraft,
    host: &str,
    really: bool,
) -> InstallResult {
    let mut res = InstallResult {
        ok: false,
        installed: false,
        dry_run: !really,
        kit: draft.name.clone(),
        host: host.to_string(),
        sha256: None,
        bytes: None,
        report: InstallReport::default(),
        record: None,
        warnings: Vec::new(),
        steps: Vec::new(),
        output: String::new(),
    };
    // 1. build, and hash what was built
    let scratch = Scratch(staging_dir(&draft.name));
    let dir = scratch.0.clone();
    let b = build_in(runner, bin, draft, &dir);
    res.warnings = b.warnings.clone();
    if !b.ok {
        res.steps.push("build failed".into());
        res.output = b.output;
        return res;
    }
    let sha = b.sha256.clone().expect("a good build has a hash");
    res.steps.push(format!(
        "built {HPPD_NAME}: {} bytes, sha256 {sha}",
        b.bytes.unwrap_or(0)
    ));
    res.sha256 = Some(sha.clone());
    res.bytes = b.bytes;
    let out = dir.join(HPPD_NAME).display().to_string();

    // 2. the installer's dry run: it rebuilds the file itself and prints the script with the
    //    sha256 it will check after the copy. That must be the hash of the bytes shown above.
    let plan = runner.run(bin, &draft.install_args(&out, host, true));
    res.output = plan.output.clone();
    let plan_report = parse_install_output(&plan.output);
    if !plan.ok {
        res.steps.push("the installer's dry run failed".into());
        return res;
    }
    match plan_report.plan_sha256.as_deref() {
        Some(p) if p == sha => res.steps.push(format!(
            "dry run: the installer will check sha256 {p} after the copy on {host}: same bytes"
        )),
        Some(p) => {
            res.steps.push(format!(
                "dry run: the installer would check {p}, but the hub built {sha}"
            ));
            res.warnings
                .push("the build is not byte-stable between two runs: nothing was installed".into());
            return res;
        }
        None => {
            res.steps
                .push("dry run: the plan does not name a sha256 to check".into());
            return res;
        }
    }
    if !really {
        res.ok = true;
        res.steps
            .push("dry run only: nothing was copied and no connection was opened".into());
        res.report = plan_report;
        return res;
    }

    // 3. for real
    res.steps.push(format!("installing on {host} (the pad sees its stick pulled and put back)"));
    let run = runner.run(bin, &draft.install_args(&out, host, false));
    res.output = run.output.clone();
    res.report = parse_install_output(&run.output);
    if install_succeeded(run.ok, &res.report) {
        res.ok = true;
        res.installed = true;
        res.steps.push(format!(
            "copy checked on the stick ({} bytes, sha256 ok), gadget {}",
            res.report.bytes.unwrap_or(0),
            res.report.udc.as_deref().unwrap_or("?")
        ));
        res.steps.push(
            "now on the pad: SETUP+OK > LOAD > Kit > OK > Append > OK, then confirm the kit is loaded"
                .into(),
        );
        res.record = Some(Record {
            kit: draft.name.clone(),
            sha256: sha,
            loop_pads: draft.loop_pads(),
            at: now_secs(),
        });
    } else {
        res.steps.push(
            res.report
                .error
                .clone()
                .unwrap_or_else(|| "the install did not finish with a checked copy".into()),
        );
        if res.report.reattached_after_error {
            res.steps.push("the gadget was reattached after the error".into());
        }
    }
    res
}

/// Run [`build_in`] into `dir` on a worker thread. The built file stays there (the window shows
/// where); the answer arrives on the receiver.
pub fn build_job(bin: PathBuf, draft: KitDraft, dir: PathBuf) -> Receiver<BuildResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(build_in(&Process, &bin, &draft, &dir));
    });
    rx
}

/// Run [`install`] on a worker thread. The answer arrives on the receiver.
pub fn install_job(
    bin: PathBuf,
    draft: KitDraft,
    host: String,
    really: bool,
) -> Receiver<InstallResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(install(&Process, &bin, &draft, &host, really));
    });
    rx
}

// MARK: WAV files

/// A WAV file's length in seconds from its header (PCM or float, any channel count), or None for
/// anything that is not a WAV. Reads the first 64 KiB, enough for the chunks before the data.
pub fn wav_seconds(path: &Path) -> Option<f64> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let total = f.metadata().ok()?.len();
    let mut head = Vec::new();
    f.take(64 * 1024).read_to_end(&mut head).ok()?;
    wav_seconds_of(&head, total)
}

/// [`wav_seconds`] on the first bytes of a file of `total` bytes.
pub fn wav_seconds_of(head: &[u8], total: u64) -> Option<f64> {
    if head.len() < 12 || &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let le32 = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let le16 = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let mut at = 12usize;
    let mut frame_bytes: Option<(u32, u32)> = None; // (sample rate, bytes per frame)
    while at + 8 <= head.len() {
        let id = &head[at..at + 4];
        let size = le32(&head[at + 4..at + 8]) as u64;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= head.len() {
            let channels = le16(&head[body + 2..]) as u32;
            let rate = le32(&head[body + 4..]);
            let bits = le16(&head[body + 14..]) as u32;
            let frame = channels * bits.div_ceil(8);
            if rate == 0 || frame == 0 {
                return None;
            }
            frame_bytes = Some((rate, frame));
        } else if id == b"data" {
            let (rate, frame) = frame_bytes?;
            // a streamed file may say 0 or 0xFFFFFFFF: the rest of the file is the data
            let avail = total.saturating_sub(body as u64);
            let n = if size == 0 || size == 0xFFFF_FFFF || size > avail {
                avail
            } else {
                size
            };
            return Some(n as f64 / frame as f64 / rate as f64);
        }
        at = body.checked_add(size as usize)?.checked_add((size & 1) as usize)?;
    }
    None
}

// MARK: the book on disk

/// The folder the kit library lives in: `$DJ_PD705_DIR`, else `<support>/djrust`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("DJ_PD705_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| support_dir().join("djrust"))
}

/// The kit book's file, `<data dir>/pd705-kits.json`.
pub fn default_book_file() -> PathBuf {
    data_dir().join("pd705-kits.json")
}

/// Where a kit's built `pad-content.hppd` is kept: `<data dir>/pd705-build/<kit>/`.
pub fn default_build_dir(kit: &str) -> PathBuf {
    data_dir().join("pd705-build").join(safe_name(kit))
}

/// Where picked samples are kept (a loop-row clip exported to a WAV, so a kit survives the
/// clip leaving the row): `<data dir>/pd705-samples`.
pub fn default_samples_dir() -> PathBuf {
    data_dir().join("pd705-samples")
}

/// Read the book. A missing file is an empty book; the second value counts kits that did not
/// decode.
pub fn load_book(file: &Path) -> (KitBook, usize) {
    match read_json(file) {
        Some(v) => KitBook::from_json(&v),
        None => (KitBook::default(), 0),
    }
}

/// Write the book (atomically).
pub fn save_book(file: &Path, book: &KitBook) -> io::Result<()> {
    write_atomic(file, (book.to_json().canonical_js() + "\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::gear::pd705_kit::PadSlot;
    use std::cell::RefCell;

    /// A WAV of `frames` mono 16-bit frames at `rate`.
    fn wav_bytes(rate: u32, frames: usize) -> Vec<u8> {
        let data = vec![0u8; frames * 2];
        let mut b = b"RIFF".to_vec();
        b.extend((36 + data.len() as u32).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2).to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("djrust-pd705-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn draft(dir: &Path) -> KitDraft {
        let mut k = KitDraft::new("Test Kit");
        for (pad, name, secs) in [(7u8, "Kick", 0.1), (8, "Snare", 0.1), (1, "Loop", 3.0)] {
            let p = dir.join(format!("{name}.wav"));
            std::fs::write(&p, wav_bytes(48_000, (secs * 48_000.0) as usize)).unwrap();
            k.set_pad(PadSlot::new(pad, &p.display().to_string(), name, secs)).unwrap();
        }
        k
    }

    /// A pd705 that never runs: it writes what a build would, and answers an install the way the
    /// real one does. Records every call.
    struct Fake {
        calls: RefCell<Vec<Vec<String>>>,
        /// What the plan names as the sha256 (None: the hash of what it wrote).
        plan_sha: Option<String>,
        real_output: String,
        real_ok: bool,
        built: RefCell<Vec<u8>>,
        content: Vec<u8>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                calls: RefCell::new(Vec::new()),
                plan_sha: None,
                real_output: "pd705: wrote pad-content.hppd (777 bytes, sha256 ok)\npd705: UDC state: configured\n".into(),
                real_ok: true,
                built: RefCell::new(Vec::new()),
                content: b"fake hppd bytes".to_vec(),
            }
        }
    }

    impl Runner for Fake {
        fn run(&self, _bin: &Path, args: &[String]) -> RunOut {
            self.calls.borrow_mut().push(args.to_vec());
            let content = self.content.clone();
            if args.iter().any(|a| a == "--to-pi") {
                if args.iter().any(|a| a == "--dry-run") {
                    let sha = self.plan_sha.clone().unwrap_or_else(|| hex(&sha256(&content)));
                    return RunOut {
                        ok: true,
                        output: format!("would install x on h\n[ \"$got\" = {sha} ] || {{ exit 4; }}\n"),
                    };
                }
                return RunOut { ok: self.real_ok, output: self.real_output.clone() };
            }
            if let Some(i) = args.iter().position(|a| a == "-o") {
                std::fs::write(&args[i + 1], &content).unwrap();
                *self.built.borrow_mut() = content.clone();
                return RunOut {
                    ok: true,
                    output: format!(
                        "sound Kick  0.100 s  /x/Kick.wav (already 48 kHz/16-bit/mono)\nsound Loop  3.000 s  /x/Loop.wav (already 48 kHz/16-bit/mono)\nwrote {} ({} bytes)\n",
                        args[i + 1],
                        content.len()
                    ),
                };
            }
            RunOut { ok: false, output: "unexpected".into() }
        }
    }

    fn has(calls: &[Vec<String>], flag: &str) -> usize {
        calls.iter().filter(|c| c.iter().any(|a| a == flag)).count()
    }

    #[test]
    fn a_build_hashes_the_file_it_made() {
        let d = tmp("build");
        let f = Fake::new();
        let r = build_in(&f, Path::new("pd705"), &draft(&d), &d.join("out"));
        assert!(r.ok, "{}", r.output);
        let sha = hex(&sha256(b"fake hppd bytes"));
        assert_eq!(r.sha256.as_deref(), Some(sha.as_str()));
        assert_eq!(r.bytes, Some(15));
        assert_eq!(r.path.as_ref().unwrap().file_name().unwrap(), HPPD_NAME);
        assert_eq!(r.report.sounds.len(), 2);
        // the file on disk hashes the same
        assert_eq!(file_sha256(r.path.as_ref().unwrap()).unwrap(), sha);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_kit_bigger_than_any_the_pad_took_gets_a_warning() {
        let d = tmp("big");
        let mut f = Fake::new();
        f.content = vec![7u8; UNTESTED_BYTES as usize + 1];
        let r = build_in(&f, Path::new("pd705"), &draft(&d), &d.join("out"));
        assert!(r.ok);
        assert!(r.warnings.iter().any(|w| w.contains("bigger than any kit the pad has accepted")), "{:?}", r.warnings);
        let ok = build_in(&Fake::new(), Path::new("pd705"), &draft(&d), &d.join("out2"));
        assert!(ok.warnings.is_empty(), "{:?}", ok.warnings);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_bad_draft_or_a_missing_sample_never_reaches_the_command() {
        let d = tmp("refuse");
        let f = Fake::new();
        let mut k = draft(&d);
        k.name = "_bad".into();
        assert!(!build_in(&f, Path::new("pd705"), &k, &d.join("o")).ok);
        let mut k = draft(&d);
        k.pads[0].path = d.join("gone.wav").display().to_string();
        let r = build_in(&f, Path::new("pd705"), &k, &d.join("o"));
        assert!(!r.ok && r.output.contains("is not a file"), "{}", r.output);
        assert!(f.calls.borrow().is_empty(), "the command was never run");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_dry_run_is_the_default_and_opens_no_connection() {
        let d = tmp("dry");
        let f = Fake::new();
        let r = install(&f, Path::new("pd705"), &draft(&d), "me@pi.local", false);
        assert!(r.ok && r.dry_run && !r.installed, "{r:?}");
        assert!(r.record.is_none(), "a dry run records nothing");
        let calls = f.calls.borrow();
        assert_eq!(has(&calls, "--to-pi"), 1);
        assert_eq!(has(&calls, "--dry-run"), 1, "the one install call was the dry run");
        assert!(r.steps.iter().any(|s| s.contains("same bytes")));
        assert!(r.steps.last().unwrap().contains("nothing was copied"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_real_install_does_the_dry_run_first_and_checks_the_copy() {
        let d = tmp("real");
        let f = Fake::new();
        let k = draft(&d);
        let r = install(&f, Path::new("pd705"), &k, "me@pi.local", true);
        assert!(r.ok && r.installed && !r.dry_run, "{r:?}");
        let calls = f.calls.borrow();
        let installs: Vec<&Vec<String>> = calls.iter().filter(|c| c.iter().any(|a| a == "--to-pi")).collect();
        assert_eq!(installs.len(), 2);
        assert!(installs[0].iter().any(|a| a == "--dry-run"), "plan first");
        assert!(!installs[1].iter().any(|a| a == "--dry-run"));
        assert!(installs[1].windows(2).any(|w| w == ["--pi-host", "me@pi.local"]));
        let rec = r.record.unwrap();
        assert_eq!(rec.kit, "Test Kit");
        assert_eq!(rec.loop_pads, [1], "the 3 s sample is a loop pad");
        assert_eq!(rec.sha256, hex(&sha256(b"fake hppd bytes")));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn nothing_is_installed_when_the_plan_names_other_bytes() {
        let d = tmp("mismatch");
        let mut f = Fake::new();
        f.plan_sha = Some("0".repeat(64));
        let r = install(&f, Path::new("pd705"), &draft(&d), "me@pi.local", true);
        assert!(!r.ok && !r.installed);
        assert!(r.warnings.iter().any(|w| w.contains("byte-stable")), "{r:?}");
        assert_eq!(has(&f.calls.borrow(), "--to-pi"), 1, "no second, real call");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_real_run_without_a_checked_copy_is_not_a_success() {
        let d = tmp("nocheck");
        let mut f = Fake::new();
        f.real_output = "pd705: UDC state: configured\n".into();
        let r = install(&f, Path::new("pd705"), &draft(&d), "h", true);
        assert!(!r.ok && !r.installed && r.record.is_none(), "{r:?}");
        let mut f = Fake::new();
        f.real_ok = false;
        f.real_output = "pd705: pad-content.hppd sha256 mismatch after copy\npd705: gadget reattached after an error\n".into();
        let r = install(&f, Path::new("pd705"), &draft(&d), "h", true);
        assert!(!r.ok && r.record.is_none());
        assert!(r.steps.iter().any(|s| s.contains("reattached")), "{:?}", r.steps);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn wav_lengths_come_from_the_header() {
        let b = wav_bytes(48_000, 24_000);
        assert_eq!(wav_seconds_of(&b, b.len() as u64), Some(0.5));
        // a streamed header (size 0xFFFFFFFF) uses the file length
        let mut s = b.clone();
        let at = s.windows(4).position(|w| w == b"data").unwrap() + 4;
        s[at..at + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(wav_seconds_of(&s, s.len() as u64), Some(0.5));
        assert_eq!(wav_seconds_of(b"not a wav at all", 16), None);
        assert_eq!(wav_seconds_of(&b[..30], 30), None, "truncated before data");
        // a LIST chunk before the data is stepped over
        let mut l = b[..36].to_vec();
        l.extend(b"LIST");
        l.extend(5u32.to_le_bytes());
        l.extend([1, 2, 3, 4, 5, 0]); // odd size, padded
        l.extend(&b[36..]);
        assert_eq!(wav_seconds_of(&l, l.len() as u64), Some(0.5));
        let d = tmp("wavfile");
        let p = d.join("x.wav");
        std::fs::write(&p, &b).unwrap();
        assert_eq!(wav_seconds(&p), Some(0.5));
        assert_eq!(wav_seconds(&d.join("none.wav")), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_book_survives_a_round_trip_through_a_file() {
        let d = tmp("book");
        let file = d.join("nested/pd705-kits.json");
        assert_eq!(load_book(&file).0, KitBook::default());
        let mut b = KitBook::default();
        b.add("Acid House").unwrap();
        save_book(&file, &b).unwrap();
        assert_eq!(load_book(&file), (b, 0));
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load_book(&file).0, KitBook::default());
        let _ = std::fs::remove_dir_all(d);
    }

    /// With the real `pd705` installed: the dry-run plan of a real build names the sha256 of
    /// the bytes the hub built, byte for byte, and opens no connection (dry run only).
    #[test]
    fn the_real_pd705_dry_run_names_the_same_bytes() {
        let Some(bin) = find_pd705() else {
            eprintln!("pd705 not found: skipped");
            return;
        };
        let d = tmp("real-pd705");
        let mut k = KitDraft::new("HubTest");
        for (pad, name, frames) in [(7u8, "Kick", 4800usize), (8, "Snare", 9600)] {
            let p = d.join(format!("{name}.wav"));
            std::fs::write(&p, wav_bytes(48_000, frames)).unwrap();
            k.set_pad(PadSlot::new(pad, &p.display().to_string(), name, frames as f64 / 48_000.0))
                .unwrap();
        }
        let r = install(&Process, &bin, &k, "nobody@invalid.example", false);
        assert!(r.ok && r.dry_run && !r.installed, "{r:?}\n{}", r.output);
        assert!(r.steps.iter().any(|s| s.contains("same bytes")), "{:?}", r.steps);
        // byte-stable: the same inputs build the same file twice
        let a = build_in(&Process, &bin, &k, &d.join("a"));
        let b = build_in(&Process, &bin, &k, &d.join("b"));
        assert!(a.ok && b.ok);
        assert_eq!(a.sha256, b.sha256);
        let _ = std::fs::remove_dir_all(d);
    }
}
