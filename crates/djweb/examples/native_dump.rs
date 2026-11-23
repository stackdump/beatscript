//! The native side of scripts/test-web-wasm.mjs: for each program file given, print as one JSON document what the
//! browser module must answer byte for byte (check, fmt, share, open), what the runtime does on a fake clock
//! (a hash of every service pass and of the marks at every bar), and a hash of the engine's first seconds of
//! audio for the program's first deck-A net (`render_offline`, the engine's own native render).
//!
//!   cargo run -q -p djweb --example native_dump -- [--seconds 3] [--passes 640] FILE.beatscript ...
//!
//! The fake clock is the one `tests/run.rs` of beatscript-api uses and the Node test repeats exactly: the transport starts
//! at beat 0 when the first deck plays, then advances a quarter beat a pass.

use std::collections::BTreeMap;

use djrust::json::{JsonObject, JsonValue};
use sha2::{Digest, Sha256};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

fn j(s: impl Into<String>) -> JsonValue {
    JsonValue::str(s)
}

/// One program on the fake clock: (the eval answer, the hash of each pass's commands, the hash of the marks every 16 passes,
/// the status at the end, the first load_net envelope of deck A as JSON text).
fn simulate(text: &str, passes: usize) -> (String, Vec<String>, Vec<String>, String, Option<String>) {
    let mut run = beatscript_api::run::Run::new();
    let eval = run.eval_str(text);
    let mut beat = 0.0f64;
    let mut playing: BTreeMap<&str, bool> = BTreeMap::from([("A", false), ("B", false)]);
    let (mut cmd_hashes, mut mark_hashes) = (Vec::new(), Vec::new());
    let mut first_a: Option<String> = None;
    for pass in 0..passes {
        let any = playing.values().any(|p| *p);
        if any {
            beat += 0.25;
        }
        let grid = match ["A", "B"].into_iter().find(|d| playing[d]) {
            None => "null".to_string(),
            Some(d) => format!("{{\"deck\":\"{d}\",\"beat\":{beat},\"bpm\":140,\"playing\":{{\"A\":{},\"B\":{}}}}}", playing["A"], playing["B"]),
        };
        let out = run.service_str(&grid).expect("grid json");
        let cmds = JsonValue::parse(&out).expect("commands are json");
        for c in cmds.as_array().unwrap() {
            match c.get("op").and_then(JsonValue::as_str) {
                Some("play") => {
                    let on = c.get("on").and_then(JsonValue::as_bool).unwrap();
                    if on && !playing.values().any(|p| *p) {
                        beat = 0.0;
                    }
                    let d = if c.get("deck").and_then(JsonValue::as_str) == Some("B") { "B" } else { "A" };
                    playing.insert(d, on);
                }
                Some("load_net") if first_a.is_none() && c.get("deck").and_then(JsonValue::as_str) == Some("A") => {
                    first_a = Some(c.get("envelope").unwrap().js_stringified());
                }
                _ => {}
            }
        }
        cmd_hashes.push(sha(&out));
        if pass % 16 == 15 {
            mark_hashes.push(sha(&run.marks_str()));
        }
    }
    (eval, cmd_hashes, mark_hashes, run.status_str(), first_a)
}

fn main() {
    let mut seconds = 3.0f64;
    let mut passes = 640usize;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()).expect("--seconds N"),
            "--passes" => passes = args.next().and_then(|v| v.parse().ok()).expect("--passes N"),
            _ => files.push(a),
        }
    }
    let mut all = JsonObject::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        // keyed by the path as given: the same file name is in more than one directory
        let name = f.clone();
        let mut o = JsonObject::new();
        o.set("check", j(beatscript_api::check_json(&text)));
        o.set("fmt", j(beatscript_api::fmt_json(&text)));
        let share = beatscript_api::share_json(&text, None, false, beatscript_api::DEFAULT_ORIGIN);
        let url = JsonValue::parse(&share).ok().and_then(|v| v.get("url").and_then(JsonValue::as_str).map(str::to_string));
        o.set("open", url.as_deref().map_or(JsonValue::Null, |u| j(beatscript_api::open_json(u))));
        o.set("share", j(share));
        o.set("highlight", j(beatscript_api::highlight_json_utf16(&text)));
        let ok = JsonValue::parse(&beatscript_api::check_json(&text)).ok().and_then(|v| v.get("ok").and_then(JsonValue::as_bool)) == Some(true);
        if ok {
            let (eval, cmds, marks, status, first_a) = simulate(&text, passes);
            let strs = |v: Vec<String>| JsonValue::Array(v.into_iter().map(JsonValue::str).collect());
            let mut r = JsonObject::new();
            r.set("eval", j(eval));
            r.set("passes", JsonValue::int(passes as i64));
            r.set("commands", strs(cmds));
            r.set("marks", strs(marks));
            r.set("status", j(status));
            o.set("run", JsonValue::Object(r));
            if let Some(env) = first_a {
                let frames = (seconds * 48000.0) as usize;
                let mut r = JsonObject::new();
                r.set("envelope_sha256", j(sha(&env)));
                match djrust::wasm::render_offline(&env, seconds, 48000.0) {
                    Ok(samples) => {
                        let mut h = Sha256::new();
                        for s in &samples {
                            h.update(s.to_le_bytes());
                        }
                        r.set("frames", JsonValue::int((samples.len() / 2) as i64));
                        r.set("sha256", j(hex(&h.finalize())));
                        r.set("peak", JsonValue::Number(samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) as f64));
                    }
                    Err(_) => {
                        r.set("frames", JsonValue::int(frames as i64));
                        r.set("error", j("render_offline failed"));
                    }
                }
                o.set("render", JsonValue::Object(r));
            }
        }
        all.set(name, JsonValue::Object(o));
    }
    println!("{}", JsonValue::Object(all).js_stringified());
}
