//! beatscript-api gives the diagnostics and CIDs the desktop does: the same programs through the desktop's environment
//! (`djhost::beatscript_env::BeatscriptEnv`, the live registry and catalog), through `djlive` directly, and through the real
//! `djctl btw ...` binary, agree on CIDs, nets, canonical text and every diagnostic.

mod support;

use std::process::Command;

use djengine::json::JsonValue;
use djhost::beatscript_env::{BeatscriptEnv, RegistryView};
use djhost::control_registry;
use djlive::compile::check_with;
use djlive::env::Registry;
use support::*;

/// Sources that are wrong in the ways a registry, a catalog or an import can say.
const BAD: &[&str] = &[
    "do vocoer on\n",
    "do vocal create bars=2\n",
    "do mixer decks=A\n",
    "do btw eval text=\"deck A: kick x\" nme=\"x\"\n",
    "do loopsets load_pair group=\"two\"\n",
    "do vocal creat lyrics=\"x\"\n",
    "wheel sinc deck=A\n",
    "wheel sync dek=A\n",
    "deck A: bass c2 ~ inst=wobble-lad\n",
    "deck A: bass c2 ~ inst=reese\n",
    "deck A: kick x ~ kit=drums-909\n",
    "deck A: kick x ~ kit=drums-808\n",
    "deck A: kick |\n",
    "tempo 999\n",
    "key h\n",
    "deck C: kick x\n",
    "section x 4 bars {\n",
    "import \"nothing\"\ndeck A: kick x\n",
    "sample s from \"~/a.wav\"\ndeck A: bass c2 inst=s\n",
    "use restricted\nsample s from \"~/a.wav\" else bright-pluck\ndeck A: bass c2 inst=s\n",
    "sample s from \"~/a.wav\" license cc0 else nonesuch\ndeck A: bass c2 inst=s\n",
    "deck A: kick x*3 | hihat x x x\n",
    "evolve every 4 bars {\n  melody: rotate 1\n}\n",
    "let kick = x\n",
    "def a() { b() }\ndef b() { a() }\na()\n",
];

fn desktop(text: &str) -> djlive::Check {
    let be = BeatscriptEnv::new(None);
    check_with(text, &be.env())
}

/// What the web says that the desktop does not, on purpose: where the browser's engine does not play a line (the
/// desktop plays it). Set aside before the two are compared.
const BROWSER_ONLY: &str = "the browser does not play this";

fn web(text: &str) -> djlive::Check {
    let mut c = check_with(text, &beatscript_api::web_env().env());
    c.diagnostics.retain(|d| !d.message.starts_with(BROWSER_ONLY));
    if let Some(comp) = c.compiled.as_mut() {
        comp.warnings.retain(|d| !d.message.starts_with(BROWSER_ONLY));
    }
    c
}

/// A program the browser sings in its nets (a `sing` line): its nets differ from the desktop's, which sings it with calls.
fn sings(text: &str) -> bool {
    text.lines().any(|l| l.split('#').next().unwrap_or("").contains("sing "))
}

/// The import line is the one thing the web says in its own words: the same error at the same place.
fn comparable(text: &str, ds: &[djlive::Diagnostic]) -> Vec<(bool, usize, usize, String, Vec<String>, String)> {
    ds.iter().map(|d| (d.is_error(), d.span.start, d.span.end, d.message.clone(), d.expected.clone(), d.found.clone())).filter(|_| !text.contains("import")).collect()
}

#[test]
fn the_web_env_checks_every_program_as_the_desktop_env_does() {
    let mut programs: Vec<(String, String)> = all_programs();
    programs.extend(BAD.iter().enumerate().map(|(i, t)| (format!("bad{i}"), t.to_string())));
    for (name, text) in &programs {
        let (w, d) = (web(text), desktop(text));
        if text.contains("import") {
            // desktop: no store at all; web: a store that says why. Both refuse, at the same place.
            assert_eq!(w.ok(), d.ok(), "{name}");
            assert_eq!(w.diagnostics.iter().map(|x| x.span).collect::<Vec<_>>(), d.diagnostics.iter().map(|x| x.span).collect::<Vec<_>>(), "{name}");
            continue;
        }
        assert_eq!(w.diagnostics, d.diagnostics, "{name}: diagnostics differ");
        assert_eq!(w.ok(), d.ok(), "{name}");
        if let (Some(wc), Some(dc)) = (&w.compiled, &d.compiled) {
            assert_eq!(wc.cid, dc.cid, "{name}");
            assert_eq!(wc.text, dc.text, "{name}");
            if !sings(text) {
                assert_eq!(wc.net_cids(), dc.net_cids(), "{name}");
                assert_eq!(wc.song_net_cids(), dc.song_net_cids(), "{name}");
            }
            assert_eq!(wc.evolve_report(), dc.evolve_report(), "{name}");
            assert_eq!(wc.rights, dc.rights, "{name}");
        }
        let _ = comparable(text, &w.diagnostics);
    }
}

#[test]
fn the_json_report_is_the_desktop_compilation_in_json() {
    for (name, text) in all_programs() {
        let mut r = parse(&beatscript_api::check_json(&text));
        if let JsonValue::Object(o) = &mut r {
            if let Some(JsonValue::Array(ds)) = o.get("diagnostics").cloned() {
                let keep: Vec<JsonValue> = ds.into_iter().filter(|d| !d.get("message").and_then(JsonValue::as_str).unwrap_or("").starts_with(BROWSER_ONLY)).collect();
                o.set("diagnostics", JsonValue::Array(keep));
            }
        }
        let d = desktop(&text);
        assert_eq!(b(&r, "ok"), d.ok(), "{name}");
        assert_eq!(get(&r, "diagnostics").as_array().unwrap().len(), d.diagnostics.len(), "{name}");
        for (j, dd) in get(&r, "diagnostics").as_array().unwrap().iter().zip(&d.diagnostics) {
            assert_eq!(s(j, "message"), dd.message, "{name}");
            assert_eq!((n(j, "start") as usize, n(j, "end") as usize), (dd.span.start, dd.span.end), "{name}");
            assert_eq!(s(j, "render"), dd.render(&text), "{name}: the caret form is the desktop's own");
            assert_eq!(get(j, "help").as_str(), dd.help.as_deref(), "{name}");
        }
        let Some(c) = d.compiled else { continue };
        assert_eq!(s(&r, "cid"), c.cid, "{name}");
        assert_eq!(s(&r, "canonical"), c.text, "{name}");
        let nets = get(&r, "nets").as_object().unwrap();
        assert_eq!(nets.iter().map(|(k, v)| (k.to_string(), v.as_str().unwrap().to_string())).collect::<std::collections::BTreeMap<_, _>>(), c.net_cids(), "{name}");
        // a deck whose song is longer than one net is chained nets with no one CID: djctl prints none, and so does the report
        match c.song_nets().map(|n| n.into_iter().filter(|(_, ir)| ir.period as u64 <= djlive::compile::MAX_BARS).map(|(d, ir)| (d.name().to_string(), ir.net_cid())).collect::<std::collections::BTreeMap<_, _>>()) {
            Ok(m) => assert_eq!(get(&r, "song_nets").as_object().unwrap().iter().map(|(k, v)| (k.to_string(), v.as_str().unwrap().to_string())).collect::<std::collections::BTreeMap<_, _>>(), m, "{name}"),
            Err(_) => assert!(matches!(get(&r, "song_nets"), JsonValue::Null), "{name}"),
        }
        assert_eq!(get(&r, "tempo").as_f64(), c.tempo, "{name}");
        assert_eq!(get(&r, "key").as_str().map(str::to_string), c.key.map(|k| k.text()), "{name}");
        assert_eq!(get(&r, "swing").as_f64(), c.swing, "{name}");
        assert_eq!(b(&r, "arranged"), c.arranged, "{name}");
        assert_eq!(b(&r, "loops"), c.looped, "{name}");
        assert_eq!(get(&r, "end_bar").as_i64(), c.end.map(i64::from), "{name}");
        assert_eq!(get(&r, "evolve").strings().unwrap(), c.evolve_report(), "{name}");
        assert_eq!(get(&r, "lineage").as_array().unwrap().len(), c.lineage.len(), "{name}");
        for (j, g) in get(&r, "lineage").as_array().unwrap().iter().zip(&c.lineage) {
            assert_eq!((s(j, "scope"), n(j, "gen"), s(j, "cid")), (g.scope.as_str(), i64::from(g.gen), g.cid.as_str()), "{name}");
        }
    }
}

/// Every verb of every noun of the control language, written as a `do` line with the params it requires:
/// both environments accept it and make the same call; and with the params left off, say the same thing.
#[test]
fn every_registry_verb_checks_the_same_in_both_environments() {
    let mut tried = 0;
    for n in control_registry::nouns().iter().filter(|n| !n.local) {
        let spec = RegistryView.noun(n.name).unwrap();
        let verbs: Vec<Option<&str>> = if n.actions.is_empty() { vec![None] } else { n.actions.iter().map(|a| Some(a.name)).collect() };
        for v in verbs {
            let mut bare = format!("do {}", n.name);
            if let Some(v) = v {
                bare.push_str(&format!(" {v}"));
            }
            let mut full = bare.clone();
            if let Some(a) = v.and_then(|v| spec.action(v)) {
                for p in a.params.iter().filter(|p| p.required) {
                    let f = spec.param(&p.name).unwrap_or(p);
                    let value = if let Some(o) = f.options.first() {
                        o.clone()
                    } else if f.types.iter().any(|t| t == "string") {
                        "\"x\"".into()
                    } else if f.types.iter().any(|t| t == "integer") {
                        "1".into()
                    } else if f.types.iter().any(|t| t == "number") {
                        "0.5".into()
                    } else if f.types.iter().any(|t| t == "boolean") {
                        "on".into()
                    } else if f.types.iter().any(|t| t == "array") {
                        "[1]".into()
                    } else {
                        "1".into()
                    };
                    full.push_str(&format!(" {}={value}", p.name));
                }
            }
            for line in [bare, full] {
                let text = format!("{line}\n");
                let (w, d) = (web(&text), desktop(&text));
                assert_eq!(w.diagnostics, d.diagnostics, "{text}");
                assert_eq!(w.compiled.map(|c| c.cid), d.compiled.map(|c| c.cid), "{text}");
                tried += 1;
            }
        }
    }
    assert!(tried > 500, "only {tried} lines tried");
}

// ---------- the real djctl ----------

/// target/debug/djctl, built if it is not there. None when it cannot be built (a note says so and the test passes).
fn djctl() -> Option<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::var("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("target"));
    let bin = target.join("debug/djctl");
    let built = Command::new(option_env!("CARGO").unwrap_or("cargo")).args(["build", "-q", "-p", "djctl"]).current_dir(&root).status().map(|s| s.success()).unwrap_or(false);
    if built && bin.exists() {
        Some(bin)
    } else {
        eprintln!("note: djctl could not be built here: the djctl parity test is skipped");
        None
    }
}

#[test]
fn the_same_cids_as_djctl_btw_on_every_program() {
    let Some(bin) = djctl() else { return };
    let store = std::env::temp_dir().join(format!("beatscript-api-djctl-store-{}", std::process::id()));
    std::fs::create_dir_all(&store).unwrap();
    let run = |args: &[&str], file: &std::path::Path| {
        let out = Command::new(&bin).arg("btw").args(args).arg(file).env("DJ_LIVE_DIR", &store).output().expect("run djctl");
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let tmp = std::env::temp_dir().join(format!("beatscript-api-djctl-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut compared = 0;
    for (name, text) in all_programs() {
        let file = tmp.join(format!("{name}.beatscript"));
        std::fs::write(&file, &text).unwrap();
        let r = parse(&beatscript_api::check_json(&text));
        let (ok, out, err) = run(&["check"], &file);
        assert_eq!(ok, b(&r, "ok"), "{name}: djctl says {err}");
        if !ok {
            // the same errors, in the same caret form
            for d in get(&r, "diagnostics").as_array().unwrap() {
                assert!(err.contains(s(d, "render")), "{name}: djctl's stderr has no\n{}\nin\n{err}", s(d, "render"));
            }
            continue;
        }
        let cid = out.lines().find_map(|l| l.strip_prefix("ok  cid ")).and_then(|l| l.split_whitespace().next()).unwrap_or_else(|| panic!("{name}: no cid in {out}"));
        assert_eq!(cid, s(&r, "cid"), "{name}");
        // fmt: the canonical text, then the CID on stderr
        let (_, fmt_out, fmt_err) = run(&["fmt"], &file);
        assert_eq!(fmt_out, s(&r, "canonical"), "{name}");
        assert!(fmt_err.contains(&format!("cid {}", s(&r, "cid"))), "{name}");
        // nets: program CID and the longhand nets
        let (nets_ok, nets_out, _) = run(&["nets"], &file);
        if nets_ok {
            assert!(nets_out.lines().next().unwrap().ends_with(s(&r, "cid")), "{name}");
            for l in nets_out.lines().filter_map(|l| l.strip_prefix("deck ")) {
                let mut w = l.split_whitespace();
                let (deck, ncid) = (w.next().unwrap(), w.next().unwrap());
                if ncid == "longer" {
                    // chained nets: no single CID, none in the report either
                    assert!(get(&r, "song_nets").get(deck).is_none(), "{name}");
                    continue;
                }
                assert_eq!(s(&r, &format!("song_nets.{deck}")), ncid, "{name}");
            }
        }
        // share: the site's link (the formatted text, on beatscript.app), and with --beats the same link as beats'
        let (_, link, _) = run(&["share"], &file);
        let beats = Command::new(&bin).args(["btw", "share"]).arg(&file).arg("--beats").env("DJ_LIVE_DIR", &store).output().expect("run djctl");
        let beats = String::from_utf8_lossy(&beats.stdout).to_string();
        if !b(&r, "restricted") {
            let sh = parse(&beatscript_api::share_json(&text, None, false, ""));
            assert_eq!(link.trim(), s(&sh, "url"), "{name}");
            assert_eq!(beats.trim(), s(&sh, "beats_url"), "{name}");
        }
        compared += 1;
    }
    assert!(compared >= 12, "{compared}");
    let out = Command::new(&bin).args(["btw", "instruments"]).env("DJ_LIVE_DIR", &store).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), beatscript_api::instruments_json());
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&store);
}
