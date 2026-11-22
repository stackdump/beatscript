//! The render route's export formats (docs/EXPORT.md): `GET /api/render/<cid>[.<packed>]?format=midi|stems|stems-opus`.
//!
//! - `midi`: the Standard MIDI File, made at once (one render) and kept; `audio/midi`, an attachment.
//! - `stems`, `stems-opus`: the zip of stems (a WAV, or an Ogg Opus, a part and a deck, the master, the .mid and
//!   `manifest.json` with the stored rights). Many renders, so a job: the first request starts it and is answered
//!   `202` with what it is doing (`Retry-After`); asking again is the same until the zip is there, then it is the
//!   zip. One stems job at a time on the server, a few new ones an hour from an address, the last few zips kept.
//!
//! The work is btwd's export (export.rs, linked in by path as render.rs is): this file is the HTTP around it.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::export::{self, ExportOptions, Rights};
use crate::service::{Limiter, RENDER_VERSION};
use crate::{content, go_body, go_json, store, ApiError, AppState};

/// The formats this file serves.
pub const FORMATS: [&str; 3] = ["midi", "stems", "stems-opus"];
/// What an export's files are named by, beside the render version: bumped when the export's bytes change.
const EXPORT_VERSION: &str = "x1";
/// A program longer than this has no stems here (a stem a minute is ~21 MB of float WAV).
const STEMS_MAX_SECONDS: f64 = 600.0;
/// Zips kept on disk (the oldest goes first).
const KEEP_ZIPS: usize = 12;
/// New exports an address may start an hour.
const MIDI_PER_HOUR: usize = 60;
const STEMS_PER_HOUR: usize = 6;
/// Threads a stems job renders on.
const STEMS_THREADS: usize = 2;

#[derive(Clone, Debug)]
enum JobState {
    Running { file: String, done: usize },
    Failed(ApiError),
}

struct Exports {
    jobs: Mutex<HashMap<String, JobState>>,
    limits: Mutex<(Limiter, Limiter)>,
    /// The order zips were made in, for eviction.
    made: Mutex<VecDeque<PathBuf>>,
}

fn exports() -> &'static Exports {
    static E: OnceLock<Exports> = OnceLock::new();
    E.get_or_init(|| Exports {
        jobs: Mutex::new(HashMap::new()),
        limits: Mutex::new((Limiter::new(MIDI_PER_HOUR, Duration::from_secs(3600)), Limiter::new(STEMS_PER_HOUR, Duration::from_secs(3600)))),
        made: Mutex::new(VecDeque::new()),
    })
}

/// The `v=` of an export URL (the render version and the export's own).
pub fn url_version() -> String {
    format!("{RENDER_VERSION}.{EXPORT_VERSION}")
}

fn dir(app: &AppState) -> PathBuf {
    app.cfg.render_dir.join("export")
}

fn file_name(cid: &str, format: &str) -> String {
    let (sr, ext) = match format {
        "midi" => (44_100, "mid"),
        "stems-opus" => (48_000, "stems-opus.zip"),
        _ => (44_100, "stems.zip"),
    };
    format!("{cid}.{sr}.{RENDER_VERSION}.{EXPORT_VERSION}.{ext}")
}

/// The stored rights of `cid` (`api.Meta`), for the manifest and the MIDI copyright line.
fn rights(app: &AppState, cid: &str) -> Rights {
    let m = app.store.meta(cid).unwrap_or(Value::Null);
    let s = |k: &str| m[k].as_str().filter(|v| !v.is_empty()).map(str::to_string);
    Rights {
        license: s("license"),
        name: s("name"),
        author: s("author"),
        forked_from: s("forked_from"),
        credits: m["credits"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).filter(|c| !c.trim().is_empty()).collect()).unwrap_or_default(),
    }
}

/// A download name from the song's name: letters, digits, `-` and `_`.
fn download(name: &str, cid: &str, ext: &str) -> String {
    let n: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let n = n.trim_matches('-');
    if n.is_empty() {
        format!("beatscript-{}.{ext}", &cid[..cid.len().min(12)])
    } else {
        format!("{}.{ext}", &n[..n.len().min(60)])
    }
}

fn json_response(status: u16, v: &Value, retry: Option<u64>) -> Response<Body> {
    let mut b = Response::builder().status(status).header(header::CONTENT_TYPE, "application/json; charset=utf-8").header(header::CACHE_CONTROL, "no-store");
    if let Some(r) = retry {
        b = b.header(header::RETRY_AFTER, r.to_string());
    }
    b.body(go_body(go_json(v) + "\n")).unwrap()
}

fn serve_file(path: &Path, content_type: &'static str, filename: &str, etag: &str, req: &Request) -> Response<Body> {
    let Ok(f) = std::fs::File::open(path) else {
        return ApiError::new(404, "not_found", "that export was just evicted; ask again").response();
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    if let Ok(v) = HeaderValue::from_str(&format!("\"{etag}\"")) {
        h.insert(header::ETAG, v);
    }
    let head = req.method() == axum::http::Method::HEAD;
    let map = if size > 0 && !head { unsafe { memmap2::Mmap::map(&f) }.ok().map(Bytes::from_owner) } else { None };
    content::serve(h, &content::Req { method: req.method(), headers: req.headers() }, size, map)
}

fn render_err(e: crate::render::RenderError) -> ApiError {
    match e.code {
        "restricted" | "uses_assets" => ApiError::new(422, "restricted", e.message),
        "check_failed" => ApiError::new(422, "bad_request", e.message),
        _ => ApiError::new(422, "render_failed", e.message),
    }
}

/// `GET /api/render/<cid>?format=midi|stems|stems-opus`.
pub async fn serve(app: AppState, raw: String, format: String, req: Request) -> Result<Response<Body>, ApiError> {
    let cid = raw.split_once('.').map_or(raw.as_str(), |(c, _)| c).to_string();
    if !store::is_cid(&cid) {
        return Err(ApiError::new(404, "not_found", "no such program"));
    }
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    let ip = crate::client_ip(peer, req.headers());
    if let Some((_, packed)) = raw.split_once('.') {
        let (app2, ip2, cid2, packed) = (app.clone(), ip.clone(), cid.clone(), packed.to_string());
        tokio::task::spawn_blocking(move || app2.from_link(&ip2, &cid2, &packed)).await.map_err(|e| ApiError::internal(e.to_string()))??;
    }
    let (p, rep) = {
        let (app2, cid2) = (app.clone(), cid.clone());
        tokio::task::spawn_blocking(move || app2.renderable(&cid2)).await.map_err(|e| ApiError::internal(e.to_string()))??
    };
    let d = dir(&app);
    std::fs::create_dir_all(&d).map_err(|e| ApiError::internal(e.to_string()))?;
    let name = file_name(&cid, &format);
    let path = d.join(&name);
    let rights = rights(&app, &cid);
    let song = rights.name.clone().unwrap_or_default();
    if format == "midi" {
        if !path.exists() {
            let (ok, retry) = exports().limits.lock().unwrap().0.allow(&ip);
            if !ok {
                return Err(ApiError::new(429, "rate_limited", "too many exports from this address; try again later").retry(retry));
            }
            let opts = ExportOptions { sr: 44_100, max_bars: app.cfg.render_max_bars, rights: rights.clone(), threads: 1 };
            let text = p.text.clone();
            let _slot = app.check_slots.clone().acquire_owned().await.map_err(|e| ApiError::internal(e.to_string()))?;
            let e = tokio::task::spawn_blocking(move || export::midi(&text, &opts)).await.map_err(|e| ApiError::internal(e.to_string()))?.map_err(render_err)?;
            let tmp = d.join(format!(".{name}.{}", std::process::id()));
            std::fs::write(&tmp, &e.midi).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| ApiError::internal(e.to_string()))?;
        }
        return Ok(serve_file(&path, "audio/midi", &download(&song, &cid, "mid"), &name, &req));
    }
    // stems
    if path.exists() {
        return Ok(serve_file(&path, "application/zip", &download(&song, &cid, "zip"), &name, &req));
    }
    if rep["seconds"].as_f64().is_some_and(|s| s > STEMS_MAX_SECONDS) {
        return Err(ApiError::new(422, "too_large", format!("stems are made for programs up to {} minutes; this one is longer (the MIDI export has no limit)", STEMS_MAX_SECONDS / 60.0)));
    }
    let key = name.clone();
    let mut jobs = exports().jobs.lock().unwrap();
    match jobs.get(&key).cloned() {
        Some(JobState::Running { file, done }) => {
            return Ok(json_response(202, &json!({"cid": cid, "format": format, "state": "rendering", "file": file, "done": done}), Some(3)));
        }
        Some(JobState::Failed(e)) => {
            jobs.remove(&key);
            return Err(e);
        }
        None => {}
    }
    if jobs.values().any(|j| matches!(j, JobState::Running { .. })) {
        return Err(ApiError::new(429, "busy", "another program's stems are being made; try again in a minute").retry(30));
    }
    let (ok, retry) = exports().limits.lock().unwrap().1.allow(&ip);
    if !ok {
        return Err(ApiError::new(429, "rate_limited", "too many stems from this address; try again later").retry(retry));
    }
    jobs.insert(key.clone(), JobState::Running { file: "master.wav".into(), done: 0 });
    drop(jobs);
    let opus = format == "stems-opus";
    let opts = ExportOptions { sr: if opus { 48_000 } else { 44_100 }, max_bars: app.cfg.render_max_bars, rights, threads: STEMS_THREADS };
    let (text, cid2, d2, key2) = (p.text.clone(), cid.clone(), d.clone(), key.clone());
    std::thread::Builder::new()
        .name(format!("stems {}", &cid[..cid.len().min(12)]))
        .spawn(move || {
            let work = d2.join(format!(".work-{key2}"));
            let _ = std::fs::remove_dir_all(&work);
            let mut done = 0usize;
            let k3 = key2.clone();
            let mut progress = move |f: &str| {
                if let Some(j) = exports().jobs.lock().unwrap().get_mut(&k3) {
                    *j = JobState::Running { file: f.to_string(), done };
                }
                done += 1;
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), ApiError> {
                let mut e = export::stems(&text, &opts, &work, &mut progress).map_err(render_err)?;
                if opus {
                    to_opus(&mut e, &work, &cid2).map_err(|e| ApiError::internal(e.to_string()))?;
                }
                let tmp = d2.join(format!(".{key2}.{}", std::process::id()));
                let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| ApiError::internal(e.to_string()))?);
                export::zip(&work, &export::bundle_order(&e), &format!("beatscript-{}", &cid2[..cid2.len().min(12)]), &mut f).map_err(|e| ApiError::internal(e.to_string()))?;
                drop(f);
                std::fs::rename(&tmp, d2.join(&key2)).map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(())
            }));
            let _ = std::fs::remove_dir_all(&work);
            let mut jobs = exports().jobs.lock().unwrap();
            match r {
                Ok(Ok(())) => {
                    jobs.remove(&key2);
                    let mut made = exports().made.lock().unwrap();
                    made.push_back(d2.join(&key2));
                    while made.len() > KEEP_ZIPS {
                        if let Some(old) = made.pop_front() {
                            let _ = std::fs::remove_file(old);
                        }
                    }
                }
                Ok(Err(e)) => {
                    jobs.insert(key2, JobState::Failed(e));
                }
                Err(_) => {
                    jobs.insert(key2, JobState::Failed(ApiError::internal("the export panicked".into())));
                }
            }
        })
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(json_response(202, &json!({"cid": cid, "format": format, "state": "rendering", "file": "master.wav", "done": 0}), Some(3)))
}

/// Every WAV of the bundle as Ogg Opus (96 kb/s, the render's own encoder), the manifest written again to say so.
fn to_opus(e: &mut export::Exported, work: &Path, cid: &str) -> std::io::Result<()> {
    let serial0 = cid.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    for (k, (f, _)) in e.files.iter_mut().enumerate() {
        let Some(stem) = f.strip_suffix(".wav") else { continue };
        let src = work.join(&*f);
        let out = format!("{stem}.opus");
        {
            let mut w = export::WavIn::open(&src)?;
            let file = std::io::BufWriter::new(std::fs::File::create(work.join(&out))?);
            let mut enc = crate::opus::OggOpus::new(file, 48_000, 96_000, serial0.wrapping_add(k as u32))?;
            loop {
                let x = w.read(48_000 * 2 / 4)?;
                if x.is_empty() {
                    break;
                }
                enc.push(&x)?;
            }
            enc.finish()?;
        }
        std::fs::remove_file(&src)?;
        *f = out;
    }
    e.codec = "opus";
    std::fs::write(work.join("manifest.json"), export::manifest(e))
}
