//! The forward model: `beatscript render` as a subprocess (it is a bin crate, and this crate must not
//! change existing crates), and `beatscript-api` in process for check / nets / same_as.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use djengine::json::JsonValue;

use crate::wav::{Pcm, SAMPLE_RATE};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct BeatscriptCli {
    bin: PathBuf,
    dir: PathBuf,
}

impl BeatscriptCli {
    /// `bin`: the beatscript binary. `dir`: scratch for program files (created if missing).
    pub fn new(bin: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Result<BeatscriptCli, String> {
        let (bin, dir) = (bin.into(), dir.into());
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(BeatscriptCli { bin, dir })
    }

    /// Where a `beatscript` is usually found: `$BEATSCRIPT_CLI`, else `target/release/beatscript` up from here, else PATH.
    pub fn find_bin() -> PathBuf {
        if let Ok(p) = std::env::var("BEATSCRIPT_CLI") {
            return p.into();
        }
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        for profile in ["release", "debug"] {
            let p = here.join("../../target").join(profile).join("beatscript");
            if p.exists() {
                return p;
            }
        }
        "beatscript".into()
    }

    /// Render a program to s16 stereo, exactly what `beatscript render` streams.
    pub fn render(&self, text: &str) -> Result<Pcm, String> {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let file = self.dir.join(format!("p{}-{id}.beatscript", std::process::id()));
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        let out = Command::new("nice")
            .args(["-n", "10"])
            .arg(&self.bin)
            .args(["render", "--format", "s16le", "--file"])
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", self.bin.display()));
        let _ = std::fs::remove_file(&file);
        let out = out?;
        if !out.status.success() {
            return Err(format!("beatscript render failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
        }
        let samples = out.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        Ok(Pcm { sr: SAMPLE_RATE, samples })
    }
}

pub fn parse_json(s: &str) -> Result<JsonValue, String> {
    JsonValue::parse(s).map_err(|e| format!("json: {e:?}"))
}

/// `check`: ok flag and the program CID.
pub fn check(text: &str) -> Result<(bool, String), String> {
    let v = parse_json(&beatscript_api::check_json(text))?;
    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
    let cid = v.get("cid").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok((ok, cid))
}

/// The velocities of the hits of one part's net, in transition order (t0, t1, ...), per deck.
pub fn part_velocities(text: &str, deck: &str, part: &str) -> Result<Vec<u8>, String> {
    let v = parse_json(&beatscript_api::nets_json(text, true))?;
    let nets = v
        .get("decks")
        .and_then(|d| d.get(deck))
        .and_then(|d| d.get("envelope"))
        .and_then(|e| e.get("nets"))
        .and_then(|n| n.get(part))
        .ok_or_else(|| format!("no net {deck}/{part}"))?;
    let tr = nets.get("transitions").and_then(|t| t.as_object()).ok_or("no transitions")?;
    let mut hits: Vec<(u32, u8)> = Vec::new();
    for (k, t) in tr.iter() {
        if let Some(m) = t.get("midi") {
            let n: u32 = k.trim_start_matches('t').parse().unwrap_or(0);
            let vel = m.get("velocity").and_then(|x| x.as_f64()).unwrap_or(0.0) as u8;
            hits.push((n, vel));
        }
    }
    hits.sort();
    Ok(hits.into_iter().map(|h| h.1).collect())
}

/// The `same_as` answer for `recovered` against `original`: (same, diff summary lines).
pub fn same_as(recovered: &str, original: &str) -> Result<(bool, Vec<String>), String> {
    let v = parse_json(&beatscript_api::same_as_json(recovered, original))?;
    let same = v.get("same").and_then(|x| x.as_bool()).unwrap_or(false);
    let mut lines = Vec::new();
    if let Some(s) = v.get("diff").and_then(|d| d.get("summary")).and_then(|s| s.as_array()) {
        for l in s {
            if let Some(t) = l.as_str() {
                lines.push(t.to_string());
            }
        }
    }
    Ok((same, lines))
}
