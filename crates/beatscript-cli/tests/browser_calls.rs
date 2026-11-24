//! The render's reading of a control call is the page's: `src/browser.rs` `map_call` / `map_sound` against
//! site/web/js/engine-calls.js `mapCall` / `mapSound`, run under node on the same calls (every branch of `mapCall`, and every
//! call the site's examples make), op for op. Needs `node` (NODE=, or on PATH); without it the comparison says SKIP and passes.

#[path = "../src/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{map_call, map_sound, Op};
use djengine::json::{JsonObject, JsonValue};

fn v(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
}

/// An op in the shape engine-calls.js gives it, its `json` / `params` strings parsed (so key order does not count).
fn shape(op: &Op) -> JsonValue {
    let o = |pairs: Vec<(&str, JsonValue)>| JsonValue::Object(JsonObject::from_pairs(pairs));
    match op {
        Op::Param { deck, name, value } => o(vec![("type", "param".into()), ("deck", deck.as_str().into()), ("name", name.as_str().into()), ("value", JsonValue::Number(*value))]),
        Op::Crossfader(x) => o(vec![("type", "crossfader".into()), ("value", JsonValue::Number(*x))]),
        Op::Perform { deck, json } => o(vec![("type", "perform".into()), ("deck", deck.as_str().into()), ("json", json.clone())]),
        Op::Play { deck, on } => o(vec![("type", "play".into()), ("deck", deck.as_str().into()), ("on", JsonValue::Bool(*on))]),
        Op::Live { method, params, phrase } => {
            let mut p = vec![("type", "live".into()), ("method", method.as_str().into()), ("params", params.clone())];
            if let Some((d, b)) = phrase {
                p.push(("phrase", o(vec![("deck", d.as_str().into()), ("bars", JsonValue::int(i64::from(*b)))])));
            }
            o(p)
        }
        Op::Move { action, kind, deck, bars, cut } => {
            let mut p = vec![("type", "move".into()), ("action", action.as_str().into())];
            if let Some(k) = kind {
                p.push(("kind", k.as_str().into()));
            }
            if let Some(d) = deck {
                p.push(("deck", d.as_str().into()));
            }
            if let Some(b) = bars {
                p.push(("bars", JsonValue::int(i64::from(*b))));
            }
            if *cut {
                p.push(("do", "cut".into()));
            }
            o(p)
        }
        Op::Wheel { action, params } => o(vec![("type", "wheel".into()), ("action", action.as_str().into()), ("params", params.clone())]),
    }
}

/// Every branch of `mapCall`, and the odd inputs it reads (strings for numbers, a missing deck, an unknown noun).
fn corpus() -> Vec<(String, JsonValue)> {
    let calls: &[(&str, &str)] = &[
        ("mixer", r#"{"deck":"A","volume":0.4}"#),
        ("mixer", r#"{"deck":"B","low":0,"mid":"0.3","high":1.4}"#),
        ("mixer", r#"{"deck":"A","gain":0.7,"filter":0.2}"#),
        ("mixer", r#"{"deck":"A","mute":true}"#),
        ("mixer", r#"{"deck":"B","mute":"false"}"#),
        ("mixer", r#"{"deck":"B","mute":1}"#),
        ("mixer", r#"{"master":0.8}"#),
        ("mixer", r#"{"crossfader":0.25}"#),
        ("mixer", r#"{"crossfader":"x"}"#),
        ("mixer", r#"{"deck":"C","volume":0.4}"#),
        ("mixer", r#"{"deck":"A","pfl":true,"headphone_mix":0.5}"#),
        ("deck", r#"{"action":"play","deck":"A"}"#),
        ("deck", r#"{"action":"stop","deck":"B"}"#),
        ("deck", r#"{"action":"pause","deck":"B"}"#),
        ("deck", r#"{"action":"load","deck":"A","source":"techno:42"}"#),
        ("deck", r#"{"action":"play"}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"filter","on":true}"#),
        ("inserts", r#"{"action":"on","target":"A","effect":"filter","on":false}"#),
        ("inserts", r#"{"action":"set","target":"Master","param":"filter.sweep","value":0.31}"#),
        ("inserts", r#"{"action":"set","target":"B","param":"filter.res","value":1.7}"#),
        ("inserts", r#"{"action":"set","target":"A","param":"filter.mix","value":0.5}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"reverb","on":true}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"reverb","on":false}"#),
        ("inserts", r#"{"action":"set","target":"Master","param":"reverb.mix","value":0.45}"#),
        ("inserts", r#"{"action":"set","target":"B","param":"reverb.mix","value":0.6}"#),
        ("inserts", r#"{"action":"set","target":"B","param":"reverb.size","value":0.6}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"crush","on":true}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"crush","on":false}"#),
        ("inserts", r#"{"action":"set","target":"Master","param":"crush.mix","value":0.333}"#),
        ("inserts", r#"{"action":"set","target":"Master","param":"crush.bits","value":0.2}"#),
        ("inserts", r#"{"action":"on","target":"A","effect":"crush","on":true}"#),
        ("inserts", r#"{"action":"set","target":"A","param":"crush.mix","value":0.6}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"delay","on":true}"#),
        ("inserts", r#"{"action":"on","target":"Master","effect":"delay","on":false}"#),
        ("inserts", r#"{"action":"set","target":"A","param":"delay.mix","value":0.3}"#),
        ("inserts", r#"{"action":"set","target":"Wheel","param":"filter.sweep","value":0.3}"#),
        ("fx", r#"{"action":"set","param":"drive","value":0.6}"#),
        ("fx", r#"{"action":"set","deck":"B","param":"drive","value":"0.4"}"#),
        ("fx", r#"{"action":"set","deck":"C","param":"drive","value":0.4}"#),
        ("fx", r#"{"action":"set","deck":"A","param":"echo","value":0.4}"#),
        ("fx", r#"{"action":"reset"}"#),
        ("vocoder", r#"{"carrier":"A","modulator":"B","mix":0.5}"#),
        ("vocoder", r#"{"action":"on"}"#),
        ("vocoder", r#"{"action":"on","on":false,"instance":2}"#),
        ("vocoder", r#"{"action":"swap"}"#),
        ("vocoder", r#"{"action":"add","instance":3}"#),
        ("vocoder", r#"{"action":"remove"}"#),
        ("vocoder", r#"{"action":"status"}"#),
        ("vocoder", r#"{"action":"set"}"#),
        ("effects", r#"{"action":"fire","target":"A","id":"riser","bars":4}"#),
        ("effects", r#"{"action":"release","target":"A","id":"tape-stop"}"#),
        ("effects", r#"{"action":"list"}"#),
        ("moves", r#"{"action":"arm","kind":"vocoder_handoff","deck":"b"}"#),
        ("moves", r#"{"action":"arm","kind":"filter_exit","deck":"A","bars":2}"#),
        ("moves", r#"{"action":"toggle","kind":"loop_roll","deck":" b "}"#),
        ("moves", r#"{"action":"arm","kind":"bass_swap","deck":"B","bars":0}"#),
        ("moves", r#"{"action":"arm","kind":"echo_out","deck":"A","bars":200}"#),
        ("moves", r#"{"action":"arm","kind":"phrase_sync","deck":"B"}"#),
        ("moves", r#"{"action":"arm","kind":"phrase_sync","deck":"B","do":"cut"}"#),
        ("moves", r#"{"action":"arm","kind":"loop_roll","deck":"C"}"#),
        ("moves", r#"{"action":"cancel","kind":"loop_roll","deck":"A"}"#),
        ("moves", r#"{"action":"cancel"}"#),
        ("moves", r#"{"action":"cancel_all"}"#),
        ("moves", r#"{"action":"status"}"#),
        ("wheel", r#"{"action":"load","preset":"cmaj7-hoop"}"#),
        ("wheel", r#"{"action":"key","root":"F"}"#),
        ("wheel", r#"{"action":"motor","on":true,"speed":1.05}"#),
        ("wheel", r#"{"action":"sync","deck":"A","ratio":2}"#),
        ("wheel", r#"{"action":"status"}"#),
        ("wheel", r#"{"action":"presets"}"#),
        ("wheel", r#"{"action":"spin"}"#),
        ("wheel", r#"{}"#),
        ("vocal", r#"{"action":"sing","text":"words"}"#),
        ("td3", r#"{"action":"play"}"#),
    ];
    let mut out: Vec<(String, JsonValue)> = calls.iter().map(|(n, p)| (n.to_string(), v(p))).collect();
    // every call the site's examples make, as the render's timeline makes them
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../site/examples");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "beatscript")).collect();
    files.sort();
    let mut env = djhost::beatscript_env::BeatscriptEnv::new(Some(&beatscript_api::known::Known));
    env.voices.in_nets = true;
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let Some(c) = djlive::compile::check_with(&text, &env.env()).compiled else { continue };
        let beats = f64::from(djhost::beatscript_render::program_bars(&c)) * 4.0;
        for t in djhost::beatscript_render::control_calls(&c, beats, &|_| true) {
            if !out.iter().any(|(n, p)| *n == t.call.noun && p.canonical_js() == t.call.params.canonical_js()) {
                out.push((t.call.noun.clone(), t.call.params.clone()));
            }
        }
    }
    out
}

/// What node makes of the same calls with the page's own module.
fn node_map(calls: &[(String, JsonValue)]) -> Option<JsonValue> {
    let node = std::env::var("NODE").unwrap_or_else(|_| "node".into());
    let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/web/js/engine-calls.js").canonicalize().unwrap();
    let script = format!(
        r#"import {{ mapCall, mapSound }} from {:?};
const parse = (o) => {{ const r = {{ ...o }}; if (typeof r.json === 'string') r.json = JSON.parse(r.json); if (typeof r.params === 'string') r.params = JSON.parse(r.params); return r; }};
let input = ''; process.stdin.on('data', (d) => (input += d)); process.stdin.on('end', () => {{
  const q = JSON.parse(input);
  const calls = q.calls.map(([noun, params]) => {{ const m = mapCall(noun, params); return {{ ops: m.ops.map(parse), ignored: m.ignored ?? null }}; }});
  const sounds = q.sounds.map(([d, r, o]) => parse(mapSound(d, r, o)));
  process.stdout.write(JSON.stringify({{ calls, sounds }}));
}});"#,
        format!("file://{}", module.display())
    );
    let dir = std::env::temp_dir().join(format!("browser-calls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("map.mjs");
    std::fs::write(&path, script).ok()?;
    let input = JsonValue::Object(JsonObject::from_pairs([
        ("calls", JsonValue::Array(calls.iter().map(|(n, p)| JsonValue::Array(vec![n.as_str().into(), p.clone()])).collect())),
        ("sounds", JsonValue::Array(sounds().iter().map(|(d, r, o)| JsonValue::Array(vec![(*d).into(), (*r).into(), (*o).into()])).collect())),
    ]));
    let mut child = std::process::Command::new(&node).arg(&path).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().ok()?;
    use std::io::Write;
    child.stdin.take()?.write_all(input.canonical_js().as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "node: {}", String::from_utf8_lossy(&out.stdout));
    Some(v(&String::from_utf8(out.stdout).unwrap()))
}

fn sounds() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![("A", "kick", "kit:drums-808"), ("B", "bass", "synth:wobble-lead"), ("A", "melody", "piano")]
}

#[test]
fn the_render_reads_every_call_as_the_page_does() {
    let calls = corpus();
    assert!(calls.len() > 80, "the corpus: {}", calls.len());
    let Some(js) = node_map(&calls) else {
        eprintln!("SKIP: no node (NODE=, or on PATH): engine-calls.js not compared");
        return;
    };
    let js_calls = js.get("calls").and_then(JsonValue::as_array).unwrap();
    assert_eq!(js_calls.len(), calls.len());
    let mut bad = Vec::new();
    for ((noun, params), j) in calls.iter().zip(js_calls) {
        let m = map_call(noun, params);
        let ours = JsonValue::Array(m.ops.iter().map(shape).collect()).canonical_js();
        let theirs = j.get("ops").unwrap().canonical_js();
        let ig = j.get("ignored").and_then(JsonValue::as_str).map(str::to_string);
        if ours != theirs || m.ignored != ig {
            bad.push(format!("{noun} {}\n  rust: {ours} ignored {:?}\n  js:   {theirs} ignored {ig:?}", params.canonical_js(), m.ignored));
        }
    }
    let js_sounds = js.get("sounds").and_then(JsonValue::as_array).unwrap();
    for ((d, r, o), j) in sounds().into_iter().zip(js_sounds) {
        let ours = shape(&map_sound(d, r, o)).canonical_js();
        if ours != j.canonical_js() {
            bad.push(format!("sound {d} {r} {o}\n  rust: {ours}\n  js:   {}", j.canonical_js()));
        }
    }
    assert!(bad.is_empty(), "{} of {} calls read otherwise than the page reads them:\n{}", bad.len(), calls.len(), bad.join("\n"));
}
