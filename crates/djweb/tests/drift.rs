//! Grammar parity between the server and the page (project-datum 6675): the seeded generator's output, for every genre over
//! many seeds, checked by the server (`beatscript_api::check_json`, what `/api/check`, the MCP `check` and beatscript answer) and by the
//! entry points the page's wasm module exports, compiled natively: `beatscript_check` (the page's facade, wasm.js `check`) and
//! `BeatscriptLs` (the editor's own diagnostics, `open` on every keystroke and `check` once the text rests). They must say the same
//! thing, error for error, and the generator must only print what every one of them accepts.
//!
//! What this cannot see is a stale deploy: the module the page serves is built by scripts/build-web-wasm.sh and copied to the
//! site by hand, not by the server's push-to-deploy, so the page can run an older grammar than the server it talks to. The
//! live check for that is the module's own `hashes.json` (its `git` is the revision it was built from):
//!
//! ```text
//! curl -s https://beatscript.app/wasm/hashes.json | jq -r .git     # must be the revision beatscript-serve was built from
//! ```

use djrust::json::JsonValue;

/// Seeds per genre of the current generator (`BEATSCRIPT_DRIFT_SEEDS=N` for another count), and of the frozen seeded@1.
const SEEDS: u32 = 50;
const SEEDS_V1: u32 = 8;

fn seeds() -> u32 {
    std::env::var("BEATSCRIPT_DRIFT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(SEEDS)
}

/// The check as a list of (severity, start, end, first line of the message): the server's JSON (`check_json`'s answer).
fn server(raw: &str) -> (bool, Vec<(String, i64, i64, String)>) {
    let v = JsonValue::parse(raw).expect("check_json is JSON");
    let ok = v.get("ok").and_then(JsonValue::as_bool).unwrap_or(false);
    let ds = v
        .get("diagnostics")
        .and_then(JsonValue::as_array)
        .map(|a| {
            a.iter()
                .map(|d| {
                    let sev = d.get("severity").and_then(JsonValue::as_str).unwrap_or("").to_string();
                    let n = |k: &str| d.get(k).and_then(JsonValue::as_i64).unwrap_or(-1);
                    let msg = d.get("message").and_then(JsonValue::as_str).unwrap_or("").lines().next().unwrap_or("").to_string();
                    (sev, n("start"), n("end"), msg)
                })
                .collect()
        })
        .unwrap_or_default();
    (ok, ds)
}

/// The editor's diagnostics (LSP-shaped: severity 1 error, 2 warning, 3 information; `data.start`/`data.end` are byte offsets;
/// the message has the expected and help lines under it) in the server's terms.
fn editor(json: &str) -> Vec<(String, i64, i64, String)> {
    let v = JsonValue::parse(json).expect("BeatscriptLs answers JSON");
    v.as_array()
        .expect("an array of diagnostics")
        .iter()
        .map(|d| {
            let sev = match d.get("severity").and_then(JsonValue::as_i64) {
                Some(1) => "error",
                Some(2) => "warning",
                Some(3) => "info",
                _ => "?",
            };
            let n = |k: &str| d.get("data").and_then(|x| x.get(k)).and_then(JsonValue::as_i64).unwrap_or(-1);
            let msg = d.get("message").and_then(JsonValue::as_str).unwrap_or("").lines().next().unwrap_or("").to_string();
            (sev.to_string(), n("start"), n("end"), msg)
        })
        .collect()
}

/// djweb's exports set a panic hook that reports to the browser console, a wasm import that cannot run natively: put the
/// default hook back, so a failing assertion here says what failed.
fn native_hook() {
    let _ = djweb::beatscript_check("");
    let _ = djweb::BeatscriptLs::new();
    let _ = std::panic::take_hook();
}

/// `facade`: also hold the page's `beatscript_check` to the server's answer byte for byte (one function: a sample of the programs is
/// enough to keep the export wired to it, and each program costs a compile less without it).
fn same_everywhere(what: &str, text: &str, ls: &mut djweb::BeatscriptLs, facade: bool) {
    let raw = beatscript_api::check_json(text);
    if facade {
        assert_eq!(djweb::beatscript_check(text), raw, "{what}: beatscript_check (the page) and check_json (the server) differ");
    }
    let (ok, srv) = server(&raw);
    assert!(ok, "{what}: the generator printed a program the server's check refuses: {srv:?}\n{text}");
    // the editor: the parse tier on every keystroke finds no error in a program that checks, and its check is the server's
    let typed = editor(&ls.open("main", text));
    assert!(typed.iter().all(|d| d.0 != "error"), "{what}: the editor's parse tier has an error the server's check does not: {typed:?}");
    let rested = editor(&ls.check("main"));
    assert_eq!(rested, srv, "{what}: the editor's check and the server's check differ");
}

#[test]
fn generator_output_checks_the_same_on_the_server_and_in_the_page() {
    native_hook();
    let mut ls = djweb::BeatscriptLs::new();
    let mut n = 0;
    for genre in beatscript_api::generate::genres() {
        for seed in 1..=seeds() {
            let req = beatscript_api::GenRequest { genre: genre.clone(), seed: seed.to_string(), ..Default::default() };
            let g = beatscript_api::generate(&req).unwrap_or_else(|e| panic!("seeded@2 {genre} {seed}: {e}"));
            same_everywhere(&format!("seeded@2 {genre} seed={seed}"), &g.text, &mut ls, seed == 1);
            n += 1;
        }
    }
    for genre in beatscript_api::generate_v1::genres() {
        for seed in 1..=SEEDS_V1 {
            let req = beatscript_api::generate_v1::GenRequest { genre: genre.clone(), seed: seed.to_string(), ..Default::default() };
            let g = beatscript_api::generate_v1::generate(&req).unwrap_or_else(|e| panic!("seeded@1 {genre} {seed}: {e}"));
            same_everywhere(&format!("seeded@1 {genre} seed={seed}"), &g.text, &mut ls, seed == 1);
            n += 1;
        }
    }
    assert!(n >= beatscript_api::generate::genres().len(), "too few programs: {n}");
}

/// Radio (docs/beatscript/radio.md): the page's `beatscript_station` is the server's answer byte for byte (the MCP `station` tool and
/// `POST /api/station` answer `station_json`), retuned or not, and a station program checks the same in the page and the
/// editor as on the server.
#[test]
fn the_page_tunes_a_station_as_the_server_does() {
    native_hook();
    let station = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia\n}\n";
    let quote = |s: &str| djrust::json::JsonValue::str(s.to_string()).js_stringified();
    for req in [
        format!("{{\"station\": {}, \"n\": 3}}", quote(station)),
        format!("{{\"station\": {}, \"n\": 3, \"count\": 2, \"retune\": {{\"tempo\": \"+1\"}}, \"locks\": [\"key\"], \"neighbours\": true}}", quote(station)),
        "{\"station\": \"tempo 120\\n\"}".to_string(),
        "not json".to_string(),
    ] {
        let page = djweb::beatscript_station(&req);
        assert_eq!(page, beatscript_api::station_json(&req), "beatscript_station (the page) and station_json (the server) differ for {req}");
        assert!(JsonValue::parse(&page).is_ok(), "{page}");
    }
    let mut ls = djweb::BeatscriptLs::new();
    same_everywhere("a station", station, &mut ls, true);
}

/// The program of the report: MCP generate (trap, seed 54877, dm, 140) prints its arps as Roman numerals, `arp(i, up, 16)`,
/// which the editor of a page built before 425458c (6596, numerals in chord() and arp()) refused as "`i` is not a chord".
#[test]
fn the_reported_trap_program_and_its_numeral_arps_check_everywhere() {
    native_hook();
    let req = beatscript_api::GenRequest { genre: "trap".into(), seed: "54877".into(), key: "dm".into(), bpm: Some(140.0), ..Default::default() };
    let g = beatscript_api::generate(&req).unwrap();
    let arps = g.text.lines().find(|l| l.starts_with("let arps = ")).expect("a let of arps");
    assert!(arps.contains("arp(i, ") && arps.contains("arp(VII, "), "{arps}");
    let mut ls = djweb::BeatscriptLs::new();
    same_everywhere("trap seed=54877 key=dm bpm=140", &g.text, &mut ls, true);
    // and the numerals one by one, in a program of their own, as the editor's check reads them
    for numeral in ["i", "VII", "VI", "iv", "bVII", "V7"] {
        let text = format!("tempo 140\nkey dm\ndeck B: arp arp({numeral}, up, 16) inst=bright-pluck\n");
        same_everywhere(&format!("arp({numeral}, up, 16)"), &text, &mut ls, true);
    }
}
