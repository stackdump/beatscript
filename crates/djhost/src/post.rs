//! The post-processing chain on files (docs/POST.md): read a render, run a
//! [`PostChain`](djengine::fx::post::PostChain) over it ([`djengine::fx::post::process`]), write
//! WAV, or MP3 when an encoder (`lame`, else `ffmpeg`) is on the PATH. The DSP is the engine's,
//! in Rust; the encoder is the only outside program, and only for `.mp3`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use djengine::fx::post::{process, PostChain, PostReport};
use djengine::json::{JsonObject, JsonValue};
use djengine::session::wav::RenderedAudio;

/// A file's audio at its own rate, stereo (a mono file on both channels).
pub fn read_audio(path: &Path) -> Result<(Vec<f32>, Vec<f32>, f64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some((b, rate)) = crate::decode::read_wav(&bytes) {
        let r = b.right.unwrap_or_else(|| b.left.clone());
        return Ok((b.left, r, rate));
    }
    #[cfg(feature = "decode")]
    {
        let n = crate::decode::decode_native(path, true).map_err(|e| e.to_string())?;
        let rate = n
            .rate
            .ok_or_else(|| format!("{}: no sample rate", path.display()))?;
        let r = n.right.unwrap_or_else(|| n.left.clone());
        Ok((n.left, r, rate))
    }
    #[cfg(not(feature = "decode"))]
    Err(format!(
        "{}: not a WAV (this build reads WAV only; rebuild djctl with --features decode for MP3, FLAC, AAC…)",
        path.display()
    ))
}

/// An MP3 encoder on the PATH: `lame`, else `ffmpeg`.
pub fn mp3_encoder() -> Option<(&'static str, PathBuf)> {
    let path = std::env::var_os("PATH")?;
    for name in ["lame", "ffmpeg"] {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Some((name, p));
            }
        }
    }
    None
}

fn is_mp3(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
}

/// What [`write_audio`] wrote.
pub struct Written {
    pub path: PathBuf,
    /// `wav`, `wav-float32` or `mp3`.
    pub format: &'static str,
    /// The encoder that made an MP3, or why there is a WAV instead.
    pub note: Option<String>,
}

impl Written {
    pub fn json(&self) -> JsonValue {
        let mut o = JsonObject::from_pairs([
            ("out", JsonValue::str(self.path.display().to_string())),
            ("format", JsonValue::str(self.format)),
        ]);
        if let Some(n) = &self.note {
            o.set("note", JsonValue::str(n.clone()));
        }
        JsonValue::Object(o)
    }
}

/// Write stereo audio: a `.mp3` path is encoded (192 kbps CBR, by `lame` or `ffmpeg`) when one
/// is found, else the WAV is written beside it (`x.mp3` → `x.wav`) and `note` says so; any
/// other path is a WAV (16-bit, or 32-bit float).
pub fn write_audio(path: &Path, audio: &RenderedAudio, float32: bool) -> Result<Written, String> {
    let wav = djengine::session::wav::encode_wav(audio, float32);
    let fmt = if float32 { "wav-float32" } else { "wav" };
    if !is_mp3(path) {
        crate::stores::write_atomic(path, &wav).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(Written {
            path: path.to_path_buf(),
            format: fmt,
            note: None,
        });
    }
    let Some((name, exe)) = mp3_encoder() else {
        let p = path.with_extension("wav");
        crate::stores::write_atomic(&p, &wav).map_err(|e| format!("{}: {e}", p.display()))?;
        return Ok(Written {
            path: p,
            format: fmt,
            note: Some("no MP3 encoder (lame or ffmpeg) on the PATH: wrote WAV".into()),
        });
    };
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let tmp = std::env::temp_dir().join(format!(
        "djpost-{}-{}.wav",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    // 16-bit for the encoder: what it would make of float anyway
    std::fs::write(&tmp, djengine::session::wav::encode_wav(audio, false))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut cmd = std::process::Command::new(&exe);
    if name == "lame" {
        cmd.args(["--quiet", "-b", "192", "--cbr"]).arg(&tmp).arg(path);
    } else {
        cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&tmp)
            .args(["-codec:a", "libmp3lame", "-b:a", "192k"])
            .arg(path);
    }
    let out = cmd.output().map_err(|e| format!("{name}: {e}"));
    let _ = std::fs::remove_file(&tmp);
    let out = out?;
    if !out.status.success() {
        return Err(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(Written {
        path: path.to_path_buf(),
        format: "mp3",
        note: Some(format!("encoded by {name}, 192 kbps")),
    })
}

/// Run `chain` over `audio` in place; the report, with how long it took.
pub fn post_audio(audio: &mut RenderedAudio, chain: &PostChain) -> (PostReport, f64) {
    let t0 = Instant::now();
    let rep = process(&mut audio.left, &mut audio.right, audio.sample_rate, chain);
    (rep, t0.elapsed().as_secs_f64())
}

/// The report as JSON, with the time it took and what was written.
pub fn report_json(rep: &PostReport, took: f64, written: Option<&Written>) -> JsonValue {
    let mut o = match rep.json() {
        JsonValue::Object(o) => o,
        _ => JsonObject::new(),
    };
    o.set(
        "post_s",
        JsonValue::Number((took * 1000.0).round() / 1000.0),
    );
    if let Some(w) = written {
        if let JsonValue::Object(x) = w.json() {
            for (k, v) in x.iter() {
                o.set(k, v.clone());
            }
        }
    }
    JsonValue::Object(o)
}

/// `post <in> <out>`: read, run the chain, write (WAV or MP3), report before / after.
pub fn process_file(
    input: &Path,
    output: &Path,
    chain: &PostChain,
    float32: bool,
) -> Result<JsonValue, String> {
    let (left, right, sample_rate) = read_audio(input)?;
    let mut audio = RenderedAudio {
        sample_rate,
        left,
        right,
    };
    let (rep, took) = post_audio(&mut audio, chain);
    let w = write_audio(output, &audio, float32)?;
    let mut j = report_json(&rep, took, Some(&w));
    if let JsonValue::Object(o) = &mut j {
        o.set("in", JsonValue::str(input.display().to_string()));
    }
    Ok(j)
}

/// The measurements alone (no processing), as `post` reports them.
pub fn measure_file(input: &Path) -> Result<JsonValue, String> {
    let (l, r, sr) = read_audio(input)?;
    let mut o = match djengine::fx::post::measure(&l, &r, sr).json() {
        JsonValue::Object(o) => o,
        _ => JsonObject::new(),
    };
    o.set("in", JsonValue::str(input.display().to_string()));
    o.set("sample_rate", JsonValue::Number(sr));
    Ok(JsonValue::Object(o))
}

/// The socket / MCP `post` method on files, as `djctl serve`, djapp and both MCP servers take
/// it: `action` presets (the list), measure (`in`), or process (default: `in`, `out`, `preset`
/// or `post` / `post_chain`, `float32`). `out` defaults to `<in>-<preset>.wav` beside the input.
pub fn file_method(p: &JsonValue) -> Result<JsonValue, String> {
    let action = p.get("action").and_then(JsonValue::as_str).unwrap_or("process");
    match action {
        "presets" | "list" => Ok(JsonValue::Object(JsonObject::from_pairs([
            ("presets", PostChain::presets_json()),
            (
                "default",
                JsonValue::str(djengine::fx::post::DEFAULT_PRESET),
            ),
            (
                "mp3_encoder",
                mp3_encoder().map_or(JsonValue::Null, |(n, _)| JsonValue::str(n)),
            ),
        ]))),
        "measure" => {
            let input = p
                .get("in")
                .or_else(|| p.get("path"))
                .and_then(JsonValue::as_str)
                .ok_or("measure: `in` (a WAV, or with decode MP3 / FLAC / …)")?;
            measure_file(Path::new(input))
        }
        "process" | "run" => {
            let input = p
                .get("in")
                .or_else(|| p.get("path"))
                .and_then(JsonValue::as_str)
                .ok_or("post: `in` (the file to process)")?;
            let chain = chain_param(p)?
                .or_else(|| PostChain::preset(djengine::fx::post::DEFAULT_PRESET))
                .expect("the default preset exists");
            let out = match p.get("out").and_then(JsonValue::as_str) {
                Some(o) => PathBuf::from(o),
                None => {
                    let i = Path::new(input);
                    let stem = i.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                    i.with_file_name(format!("{stem}-{}.wav", chain.label()))
                }
            };
            let float32 = p.get("float32").and_then(JsonValue::as_bool).unwrap_or(false);
            process_file(Path::new(input), &out, &chain, float32)
        }
        other => Err(format!("post: no action {other:?} (presets, measure, process)")),
    }
}

/// A request's chain: `post_chain` (JSON, or a JSON text) over `post` / `preset` (a preset
/// name, true for the default, or a chain). None when neither is given.
pub fn chain_param(p: &JsonValue) -> Result<Option<PostChain>, String> {
    let post = p.get("post").or_else(|| p.get("preset"));
    PostChain::from_params(post, p.get("post_chain"))
}
