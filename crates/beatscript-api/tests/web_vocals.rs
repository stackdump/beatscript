//! Vocals in the browser (djlive sing.rs, "In the nets"): on the web host a `sing` line is a part of its deck's net, a
//! formant voice with a syllable on each note, so the browser engine sings it through the deck's strip, level and
//! crossfader, and the line stops when its net does. These tests drive the same `Run` the page's wasm drives, and render
//! the nets it loads with the engine the page runs.

use beatscript_api::run::Run;
use djengine::json::JsonValue;

const SING: &str = "tempo 120\nkey am\ndeck A: kick x ~ x ~ | melody <[a3 c4 e4 c4] [a3 c4 e4 g4]>\ndeck A: sing \"we are the rave to-night\" on melody voice=robot-choir\n";

/// The envelopes `Run` asks to load, by deck, on a first pass with no clock (the decks start at once).
fn loads(text: &str) -> Vec<(String, JsonValue)> {
    let mut run = Run::new();
    let ev = run.eval(text);
    assert_eq!(ev.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", ev.js_stringified());
    let out = run.service(None);
    out.as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("op").and_then(JsonValue::as_str) == Some("load_net"))
        .map(|c| (c.get("deck").and_then(JsonValue::as_str).unwrap().to_string(), c.get("envelope").unwrap().clone()))
        .collect()
}

fn calls(text: &str) -> Vec<String> {
    let mut run = Run::new();
    run.eval(text);
    let mut nouns = Vec::new();
    for _ in 0..3 {
        for c in run.service(None).as_array().unwrap() {
            if c.get("op").and_then(JsonValue::as_str) == Some("call") {
                nouns.push(c.get("noun").and_then(JsonValue::as_str).unwrap().to_string());
            }
        }
    }
    nouns
}

fn net_instruments(env: &JsonValue) -> Vec<(String, String)> {
    let nets = env.get("nets").and_then(JsonValue::as_object).expect("an envelope has nets");
    nets.iter()
        .map(|(id, n)| {
            let inst = n.get("track").and_then(|t| t.get("instrument")).and_then(JsonValue::as_str).unwrap_or("").to_string();
            (id.to_string(), inst)
        })
        .collect()
}

fn render(env: &JsonValue, seconds: f64) -> Vec<f32> {
    use djengine::host::deck::engine::EngineConfig;
    use djengine::host::deck::host::host_engine;
    djengine::host::beats::render_offline(
        || {
            let mut cfg = EngineConfig::new(48000.0);
            cfg.history_beats = None;
            host_engine(cfg, false, true)
        },
        &env.js_stringified(),
        seconds,
    )
    .unwrap()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn a_sing_line_is_a_part_of_its_decks_net_with_a_syllable_on_each_note() {
    let l = loads(SING);
    let (deck, env) = l.iter().find(|(d, _)| d == "A").expect("deck A loads");
    assert_eq!(deck, "A");
    let inst = net_instruments(env);
    assert!(inst.iter().any(|(id, i)| id == "vox" && i == "voice-robot-choir"), "{inst:?}");
    let text = env.js_stringified();
    assert!(text.contains("\"vowel\":\"we\"") || text.contains("\"vowel\":\"wi\""), "the first note sings `we`: {}", &text[..text.len().min(400)]);
    assert!(text.contains("+ih.v\""), "`rave` glides and closes on its v");
    // and it is not a call the browser would ignore
    assert!(!calls(SING).iter().any(|n| n == "vocal"), "{:?}", calls(SING));
}

#[test]
fn the_sung_part_sounds_on_its_deck() {
    let with = loads(SING);
    let without = loads(&SING.replace("deck A: sing \"we are the rave to-night\" on melody voice=robot-choir\n", ""));
    let a = render(&with[0].1, 4.0);
    let b = render(&without[0].1, 4.0);
    assert!(rms(&a) > 1e-3, "deck A is silent");
    let diff: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    assert!(rms(&diff) > 0.2 * rms(&b), "the voice adds to deck A: diff rms {} against {}", rms(&diff), rms(&b));
    // the sung part on its own (the deck's other parts muted out of the envelope): not silent
    let mut solo = with[0].1.clone();
    if let JsonValue::Object(o) = &mut solo {
        if let Some(JsonValue::Object(nets)) = o.get("nets").cloned().as_ref() {
            let mut keep = djengine::json::JsonObject::new();
            for (id, n) in nets.iter() {
                if id.starts_with("vox") {
                    keep.set(id, n.clone());
                }
            }
            o.set("nets", JsonValue::Object(keep));
        }
    }
    let v = render(&solo, 4.0);
    assert!(rms(&v) > 1e-3, "the sung part alone is silent: rms {}", rms(&v));
}

#[test]
fn an_escaped_word_sings_the_same_whatever_lexicon_there_is_and_the_same_text_the_same_net() {
    // the escape is sung as written: the nets of a program are the same however often it is compiled (its CID is its text)
    let t = "tempo 120\ndeck A: melody [a3 c4 e4 g4]\ndeck A: sing \"/w ee - aa: r - dh uh - r ay! v/\" on melody\n";
    let a = loads(t);
    let b = loads(t);
    assert_eq!(a, b);
    let mut run = Run::new();
    let cid1 = run.eval(t).get("cid").cloned();
    let cid2 = Run::new().eval(t).get("cid").cloned();
    assert_eq!(cid1, cid2);
    // `aa:` holds over two notes: four syllables on four notes, `are` taking two
    let text = a[0].1.js_stringified();
    assert_eq!(text.matches("\"vowel\":").count(), 3, "{text}");
}

#[test]
fn a_phrase_from_the_phrasebook_sings_and_prints_as_its_id() {
    let t = "tempo 124\ndeck A: melody [a3 c4 e4 g4 a4 g4 e4 c4]\ndeck A: sing @hands_up on melody\n";
    let mut run = Run::new();
    let ev = run.eval(t);
    assert_eq!(ev.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", ev.js_stringified());
    assert!(ev.get("canonical").and_then(JsonValue::as_str).unwrap().contains("sing @hands_up on melody"));
    let l = loads(t);
    assert!(net_instruments(&l[0].1).iter().any(|(id, _)| id == "vox"));
    // an unknown phrase is an error with what it meant
    let bad = Run::new().eval("deck A: melody [a3 c4]\ndeck A: sing @hands_upp on melody\n");
    assert!(bad.js_stringified().contains("did you mean `@hands_up`"), "{}", bad.js_stringified());
}

#[test]
fn a_bad_phoneme_is_an_error_at_its_place_with_a_did_you_mean() {
    let t = "deck A: melody [a3 c4]\ndeck A: sing \"we /dh AH0/ rave\" on melody\n";
    let ev = Run::new().check(t);
    let d = &ev.get("diagnostics").unwrap().as_array().unwrap()[0];
    let msg = d.js_stringified();
    assert!(msg.contains("did you mean `uh`"), "{msg}");
    // it points at `AH0`: line 2, the column of the symbol
    let start = d.get("start").and_then(JsonValue::as_i64).unwrap() as usize;
    assert_eq!(&t[start..start + 3], "AH0");
}

/// check is capability-aware: where the browser engine does not play a line, it warns at that line instead of passing
/// silently; what it plays (the vocoder, a section-start sing line) says nothing.
#[test]
fn check_warns_where_the_browser_does_not_play_a_line() {
    let warn = |t: &str| -> Vec<String> {
        let ev = Run::new().check(t);
        ev.get("diagnostics").unwrap().as_array().unwrap().iter().filter(|d| d.get("severity").and_then(JsonValue::as_str) == Some("warning")).map(|d| d.get("message").and_then(JsonValue::as_str).unwrap().to_string()).collect()
    };
    let plays = "deck A: melody [a3 c4 e4 g4]\ndeck B: bass a2 ~ e2 ~\ndo vocoder set carrier=B modulator=A mix=1 insert=true on=true\n";
    assert!(warn(plays).iter().all(|w| !w.starts_with("the browser")), "{:?}", warn(plays));
    let timed = "deck A: melody [a3 c4 e4 g4]\nat bar 4: deck A: sing \"/h ay/\" on melody\n";
    assert!(warn(timed).iter().any(|w| w.starts_with("the browser does not play this") && w.contains("sing")), "{:?}", warn(timed));
    // every move but a phrase sync that steps a seed plays here (engine-calls.js MoveScheduler: tests/moves_web.rs)
    let echo = "deck A: melody [a3 c4 e4 g4]\ndeck B: bass a2 ~ e2 ~\nat bar 2: move echo_out on B over 4 bars\n";
    assert!(warn(echo).iter().all(|w| !w.starts_with("the browser")), "{:?}", warn(echo));
    let seed = "deck A: melody [a3 c4 e4 g4]\ndeck B: bass a2 ~ e2 ~\nat bar 2: move phrase_sync on B\n";
    let w = warn(seed);
    assert!(w.iter().any(|w| w.starts_with("the browser does not play this") && w.contains("phrase_sync")), "{w:?}");
}
