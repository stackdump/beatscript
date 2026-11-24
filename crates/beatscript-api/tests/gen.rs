//! `gen@N genre seed=...`: the seeded generator called from BeatScript (djlive::gen, beatscript_api::generate::Seeded,
//! docs/beatscript/ext-ffi.md):
//! - a call is the program `seeded@N` prints, written into the program: the whole of it, a section, a part, or a part's
//!   notes as a let a bar;
//! - it is spelt one way, its version always written; the same text is the same CID, and resolves to the same pinned CID;
//! - every version stays callable (gen@1 is seeded@1, frozen); a sample of calls is pinned;
//! - a call fits the program's key and tempo unless it says its own, and check says so; two seeds in different keys cross;
//! - a referenced part mutates like a local let; eject writes one reference out, plays the same, and is the hand-written
//!   program (one CID).

use std::collections::BTreeSet;

use djlive::compile::Compiled;
use djlive::diag::Severity;
use djlive::env::Env;
use djlive::refs;

fn env() -> Env<'static> {
    beatscript_api::web_env().env()
}

fn check(src: &str) -> djlive::compile::Check {
    djlive::compile::check_with(src, &env())
}

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    let errs: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{:?} `{}` {} {}", d.span, d.span.text(src), d.message, d.help.clone().unwrap_or_default())).collect();
    c.compiled.unwrap_or_else(|| panic!("{src}\n{errs:#?}"))
}

/// Each segment's name, place, length and the nets its decks play.
fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect())).collect()
}

/// (bar from 0, step, role, note) of every hit.
fn played(c: &Compiled) -> BTreeSet<(u32, u8, String, u8)> {
    let mut out = BTreeSet::new();
    for seg in &c.segments {
        for deck in seg.decks.values() {
            for p in &deck.parts {
                for k in 0..seg.bars {
                    for h in &p.cycles[k as usize % p.cycles.len()] {
                        out.insert((seg.start + k, h.step, p.role.clone(), h.note));
                    }
                }
            }
        }
    }
    out
}

fn errors(src: &str) -> Vec<(String, String)> {
    check(src).diagnostics.into_iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
}

fn generated(genre: &str, seed: &str, bpm: Option<f64>, key: &str) -> String {
    beatscript_api::generate::generate(&beatscript_api::GenRequest { genre: genre.into(), seed: seed.into(), bpm, key: key.into(), ..Default::default() }).unwrap().text
}

#[test]
fn a_call_is_the_program_the_generator_prints() {
    beatscript_api::generate::install();
    let whole = compiled("arrange from gen@2 techno seed=42\n");
    let direct = compiled(&generated("techno", "42", None, ""));
    assert_eq!(nets(&whole), nets(&direct));
    assert_eq!((whole.tempo, whole.key, whole.end), (direct.tempo, direct.key, direct.end));
    assert_ne!(whole.cid, direct.cid, "the call's CID is of the call");
    // a section of it is that section's parts
    let src = "tempo 128\nkey c\nsection hit from gen@2 techno seed=42 section=drop\narrange hit\n";
    let sec = compiled(src);
    let drop = direct.segments.iter().find(|s| s.name == "drop").unwrap();
    let mine = &sec.segments[0];
    let roles = |s: &djlive::compile::Segment| s.decks.values().flat_map(|d| d.parts.iter().map(|p| (p.role.clone(), p.cycles.clone()))).collect::<Vec<_>>();
    assert_eq!(roles(mine), roles(drop));
    // a part line plays that part, with its params (the line's own win)
    let part = compiled("tempo 128\nkey c\nsection grv 4 bars {\n  deck B: bass gen@2 techno seed=42 section=drop inst=reese\n}\narrange grv\n");
    let p = &part.segments[0].decks.values().next().unwrap().parts[0];
    assert_eq!(p.instrument.as_deref(), Some("reese"));
    let theirs = drop.decks.values().flat_map(|d| d.parts.iter()).find(|q| q.role == "bass").unwrap();
    assert_eq!(p.cycles, theirs.cycles);
}

#[test]
fn a_call_is_spelt_one_way_with_its_version() {
    beatscript_api::generate::install();
    let a = compiled("let bed = gen techno seed=42 section=drop part=bass\ndeck A: bass bed\n");
    let b = compiled("let   bed = gen@2  techno part=bass  length=standard section=drop seed=42\ndeck A: bass bed\n");
    assert_eq!(a.text, "let bed = gen@2 techno seed=42 section=drop part=bass\ndeck A: bass bed\n");
    assert_eq!((a.cid.clone(), a.text.clone()), (b.cid, b.text));
    let c = compiled("let bed = gen@2 techno part=bass key=Am seed=Sunrise bpm=126.0 length=long section=drop\ndeck A: bass bed\n");
    assert!(c.text.contains("gen@2 techno seed=sunrise bpm=126 key=am length=long section=drop part=bass"), "{}", c.text);
    assert_ne!(a.cid, compiled("let bed = gen@1 techno seed=42 section=drop part=bass\ndeck A: bass bed\n").cid);
    // evolve's `gen` is still evolve's
    let e = compiled("deck A: kick x ~ x ~\nevolve every 4 bars {\n  kick: rotate 1 when gen % 2 == 0\n  stop when gen == 3\n}\n");
    assert!(e.text.contains("when gen % 2 == 0"), "{}", e.text);
}

/// Determinism: the same text twice is the same CID and the same resolution; the pin is the CID of the program the generator
/// prints for the request (what `generate` stores and reports).
#[test]
fn the_same_call_is_the_same_cid_and_resolves_to_the_same_pin() {
    beatscript_api::generate::install();
    let src = "tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: kick x x x x | bass bed\n}\narrange groove\n";
    let (x, y) = (compiled(src), compiled(src));
    assert_eq!(x.cid, y.cid);
    assert_eq!(nets(&x), nets(&y));
    let s1 = refs::sites(src, &env()).unwrap();
    assert_eq!(s1, refs::sites(src, &env()).unwrap());
    assert_eq!(s1.len(), 1);
    let s = &s1[0];
    assert_eq!((s.kind, s.half, s.place, s.name.as_deref(), s.generator.as_deref()), ("gen", refs::ENGINE, "let", Some("bed"), Some("seeded@2")));
    let pin = s.resolved.clone().unwrap();
    // gen@2's program, as seeded@2 prints it (3 prints it in BeatScript 2)
    let req = beatscript_api::GenRequest { genre: "techno".into(), seed: "42".into(), bpm: Some(128.0), key: "c".into(), ..Default::default() };
    let g2 = beatscript_api::generate::generate_version(2, &req).unwrap();
    let report = beatscript_api::check_report(&g2.text);
    assert_eq!(report.get("cid").and_then(djengine::json::JsonValue::as_str), Some(pin.as_str()));
    // and gen@3's is the same program in BeatScript 2: the BeatScript 2 CID of gen@2's
    let g3 = beatscript_api::generate::generate_version(3, &req).unwrap();
    let r3 = beatscript_api::check_report(&g3.text);
    assert_eq!(r3.get("cid").and_then(djengine::json::JsonValue::as_str), report.get("renamed_cid").and_then(djengine::json::JsonValue::as_str));
}

/// Every version stays callable, and a version always resolves as it did: a generator that prints another program for the
/// same inputs is a new version (`BEATSCRIPT_BLESS=1` rewrites the pins only when that is meant).
#[test]
fn every_version_is_callable_and_pinned() {
    beatscript_api::generate::install();
    let calls = ["gen@1 techno seed=42", "gen@2 techno seed=42", "gen@1 dubstep seed=54876", "gen@2 trap seed=54877", "gen@2 house seed=sunrise bpm=124 key=am", "gen@2 dnb seed=7 length=short", "gen@1 lofi seed=3 key=f#m length=long", "gen@3 techno seed=42", "gen@3 house seed=sunrise bpm=124 key=am", "gen@3 acid seed=42", "gen@4 techno seed=42", "gen@4 acid seed=42"];
    let host = djlive::gen::Host::default();
    let got: String = calls.iter().map(|c| format!("{c} -> {}\n", djlive::gen::resolve_word(c, &host).unwrap_or_else(|d| panic!("{c}: {}", d.message)).cid)).collect();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/gen-pins.txt");
    if std::env::var("BEATSCRIPT_BLESS").is_ok() {
        std::fs::write(path, &got).unwrap();
    } else {
        let want = std::fs::read_to_string(path).expect("the pins (BEATSCRIPT_BLESS=1 writes them)");
        assert_eq!(got, want, "a gen call no longer resolves as it did: that is a new version of the generator");
    }
    let v1 = djlive::gen::resolve_word("gen@1 techno seed=42", &host).unwrap();
    assert!(v1.text.contains("#! generator: seeded@1"), "{}", v1.text);
    assert!(djlive::gen::resolve_word("gen@2 techno seed=42", &host).unwrap().text.contains("#! generator: seeded@2"));
    let v3 = djlive::gen::resolve_word("gen@3 techno seed=42", &host).unwrap();
    assert!(v3.text.starts_with("#! beatscript 2\n") && v3.text.contains("#! generator: seeded@3"), "{}", v3.text);
    let e = errors("let bed = gen@5 techno seed=1 section=drop part=bass\n");
    assert!(e.iter().any(|(at, m)| at == "gen@5" && m.contains("gen@1, gen@2, gen@3, gen@4")), "{e:?}");
}

/// `seeded@4` is `seeded@3` with the acid genre's bass on the 303: the acid genre's program differs from 3's only in its
/// generator line and its bass voice (`sub-bass`, 3's fallback, is `acid-bass`), and every other genre only in its generator
/// line. The acid voice's pick draws from the stream as 3's did, so no later choice moves.
#[test]
fn seeded_4_puts_the_acid_genres_bass_on_the_303_and_changes_nothing_else() {
    beatscript_api::generate::install();
    for genre in ["acid", "techno", "house", "dnb"] {
        for seed in ["1", "42", "sunrise"] {
            let req = beatscript_api::GenRequest { genre: genre.into(), seed: seed.into(), ..Default::default() };
            let g3 = beatscript_api::generate::generate_version(3, &req).unwrap().text.replace("seeded@3", "seeded@4");
            let g4 = beatscript_api::generate::generate_version(4, &req).unwrap().text;
            if genre == "acid" {
                assert!(g4.contains("inst=acid-bass") && !g4.contains("inst=sub-bass"), "{genre} {seed}: {g4}");
                assert_eq!(g3.replace("inst=sub-bass", "inst=acid-bass"), g4, "{genre} {seed}");
                // it checks with no word about its voice
                let report = beatscript_api::check_report(&g4);
                let diags = report.get("diagnostics").and_then(djengine::json::JsonValue::as_array).cloned().unwrap_or_default();
                let said = |d: &djengine::json::JsonValue, k: &str| d.get(k).and_then(djengine::json::JsonValue::as_str).unwrap_or("").to_string();
                assert!(!diags.iter().any(|d| said(d, "severity") != "info" && said(d, "message").contains("voice")), "{diags:?}");
            } else {
                assert_eq!(g3, g4, "{genre} {seed}");
            }
        }
    }
}

/// A swung genre's call brings its swing to the whole program (`arrange from`), in every version, unless the program says its
/// own; a part never brings it (it is a setting of the whole program). docs/beatscript/ext-ffi.md, "Swing".
#[test]
fn a_swung_genres_call_brings_its_swing_to_the_whole_program() {
    beatscript_api::generate::install();
    for v in beatscript_api::generate::VERSIONS {
        assert_eq!(compiled(&format!("arrange from gen@{v} jazz seed=1\n")).swing, Some(0.6), "gen@{v} jazz");
        assert_eq!(compiled(&format!("arrange from gen@{v} house seed=1\n")).swing, Some(0.2), "gen@{v} house");
        assert_eq!(compiled(&format!("arrange from gen@{v} techno seed=1\n")).swing, None, "gen@{v} techno is straight");
        assert_eq!(compiled(&format!("swing 0.1\narrange from gen@{v} jazz seed=1\n")).swing, Some(0.1), "the program's own swing wins");
    }
    let jazz = generated("jazz", "1", None, "");
    let bass = beatscript_api::generate::parts_of(&jazz, "gen@2 jazz seed=1", false, Some("bass"), None).unwrap().remove(0);
    let part = compiled(&format!("tempo 110\nkey dm\ndeck A: bass gen@2 jazz seed=1 section={}\n", bass.section));
    assert_eq!(part.swing, None, "a part brings no swing: {}", bass.gen);
    assert!(part.decks.values().any(|d| d.parts.iter().any(|p| p.role == "bass" && p.cycles.iter().any(|c| !c.is_empty()))), "the part plays");
}

/// A call takes the program's key and tempo unless it says its own: two seeds in different keys cross without a clash, and
/// check says what each was fitted to.
#[test]
fn a_call_fits_the_programs_key_and_tempo() {
    beatscript_api::generate::install();
    let src = "tempo 124\nkey am\nlet rust = gen@2 techno seed=42 section=drop part=bass\nlet fog = gen@2 house seed=7 section=drop part=bass\nsection cross 4 bars {\n  deck A: kick x x x x | bass rust\n}\narrange {\n  cross\n  ~ rust_* morph fog_1 0.3\n  cross\n}\n";
    let c = check(src);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics);
    let notes: Vec<String> = c.diagnostics.iter().filter(|d| d.severity == Severity::Info && d.message.contains("generated in")).map(|d| d.message.clone()).collect();
    assert_eq!(notes.len(), 2, "{:?}", c.diagnostics);
    assert!(notes.iter().all(|n| n.contains("the program's key am") && n.contains("its tempo 124")), "{notes:?}");
    assert!(!c.diagnostics.iter().any(|d| d.severity == Severity::Warning), "{:?}", c.diagnostics.iter().filter(|d| d.severity == Severity::Warning).map(|d| &d.message).collect::<Vec<_>>());
    let site = &refs::sites(src, &env()).unwrap()[0];
    assert_eq!(site.fitted, vec!["key".to_string(), "bpm".to_string()]);
    // its own key and tempo are not fitted
    let own = refs::sites("tempo 124\nkey am\nlet rust = gen@2 techno seed=42 bpm=130 key=c section=drop part=bass\n", &env()).unwrap();
    assert!(own[0].fitted.is_empty());
    assert_ne!(own[0].resolved, site.resolved);
}

/// A referenced part is a let a bar: `arrange { }` mutates it as a local let, and check's table has its generations.
#[test]
fn a_referenced_part_mutates_like_a_local_let() {
    beatscript_api::generate::install();
    let lets = "tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: kick x x x x | bass bed inst=reese\n}\n";
    let src = format!("{lets}arrange {{\n  groove\n  ~ bed_* rot 1 | down 1\n  groove\n  @ bed_* oct 1\n  groove\n}}\n");
    let c = compiled(&src);
    let plain = compiled(&format!("{lets}arrange groove groove groove\n"));
    let (n, p) = (nets(&c), nets(&plain));
    assert_eq!(n.len(), 3);
    assert_eq!(n[0].3, p[0].3, "the first play is the part as generated");
    assert_ne!(n[1].3, p[1].3, "`~ bed_* rot 1 | down 1` changed nothing");
    assert_ne!(n[2].3, n[1].3, "`@ bed_* oct 1` changed nothing");
    let g = djlive::mutate::generations(&src, &env()).expect("an arrange table for a program with a reference");
    assert!(g.patterns.keys().any(|k| k.starts_with("bed_")), "{:?}", g.patterns.keys().collect::<Vec<_>>());
    // and it is what the same lets, ejected, play
    let ejected = refs::eject(&src, &env(), src.find("gen@2").unwrap()).unwrap().unwrap();
    assert_eq!(nets(&compiled(&ejected)), n, "{ejected}");
}

/// Eject: one reference written out in place under its provenance line; it plays as the reference did, and without the line
/// it is the program the same notes typed by hand are (one CID).
#[test]
fn eject_plays_the_same_and_is_the_hand_written_program() {
    beatscript_api::generate::install();
    let progs = [
        "tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: kick x x x x | bass bed\n}\narrange groove groove\n",
        "tempo 128\nkey c\nsection groove 4 bars {\n  deck A: kick x x x x\n        | bass gen@2 techno seed=42 section=drop inst=reese\n  deck B: arp seed(trance, 42)[17..20]\n}\narrange groove\n",
        "tempo 126\nkey am\nsection one from gen@2 house seed=42 section=intro\nsection two from gen@2 house seed=42 section=drop\narrange one two\n",
        "arrange from gen@2 techno seed=42 bpm=132\n",
    ];
    for p in progs {
        let c = compiled(p);
        let ss = refs::sites(p, &env()).unwrap();
        assert!(!ss.is_empty(), "{p}");
        for s in &ss {
            let e = refs::eject(p, &env(), s.span.start).unwrap().unwrap();
            let ce = compiled(&e);
            assert_eq!(nets(&ce), nets(&c), "{p}\n--- ejected {}:\n{e}", s.call);
            assert_eq!(played(&ce), played(&c));
            assert_eq!((ce.tempo, ce.key, ce.end, ce.looped), (c.tempo, c.key, c.end, c.looped));
            let line = e.lines().find(|l| l.trim_start().starts_with("# from: ")).expect("a # from: line");
            let (call, cid) = refs::read_from_line(line).unwrap();
            assert_eq!(call, s.call);
            assert_eq!(cid, s.resolved);
            let bare: String = e.lines().filter(|l| !l.trim_start().starts_with('#')).map(|l| format!("{l}\n")).collect();
            assert_eq!(compiled(&bare).cid, ce.cid, "the provenance line is in the CID:\n{e}");
            assert_eq!(refs::sites(&e, &env()).unwrap().len(), ss.len() - 1, "{e}");
        }
    }
}

#[test]
fn check_names_what_is_wrong_at_the_call() {
    beatscript_api::generate::install();
    let cases: [(&str, &str, &str); 9] = [
        ("let b = gen@2 tecno seed=1 section=drop part=bass\n", "tecno", "`techno`"),
        ("let b = gen@2 techno section=drop part=bass\n", "gen@2 techno", "says its seed"),
        ("let b = gen@2 techno seed=1 section=drop part=bas\n", "bas", "`part=bass`"),
        ("let b = gen@2 techno seed=1 section=dorp part=bass\n", "dorp", "`section=drop`"),
        ("let b = gen@2 techno seed=1 part=bass\n", "gen@2 techno seed=1 part=bass", "say which one"),
        ("let b = gen@2 techno seed=1 section=drop\n", "gen@2 techno seed=1 section=drop", "names one part"),
        ("let b = gen@2 techno seed=1 bpm=999 section=drop part=bass\n", "bpm=999", "40 to 300"),
        ("let b = gen@2 techno seed=1 length=huge section=drop part=bass\n", "huge", "short, standard and long"),
        ("let b = gen@2 techno seed=1 key=zz section=drop part=bass\n", "key=zz", "no key"),
    ];
    for (src, at, msg) in cases {
        let e = errors(src);
        assert!(e.iter().any(|(a, m)| a == at && m.contains(msg)), "{src}: {e:?}");
    }
}

/// The example programs that call the generator (examples_beatscript.rs skips them: it checks with no host) check clean here: no
/// warning, the canonical text a fixed point, and each reference ejects to the same nets.
#[test]
fn the_examples_that_call_the_generator_check_clean() {
    beatscript_api::generate::install();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript");
    let mut seen = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "beatscript") {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap();
        if !src.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("gen@")) {
            continue;
        }
        // (one that also borrows from a saved program is checked where that program is given: tests/uses.rs)
        if !djlive::stored::cids_of(&src).is_empty() {
            continue;
        }
        seen += 1;
        let c = check(&src);
        let loud: Vec<String> = c.diagnostics.iter().filter(|d| d.severity != Severity::Info).map(|d| format!("{} `{}`", d.message, d.span.text(&src))).collect();
        assert!(loud.is_empty(), "{}: {loud:#?}", p.display());
        let comp = c.compiled.unwrap();
        assert_eq!(compiled(&comp.text).cid, comp.cid, "{}: the canonical text is no fixed point", p.display());
        for s in refs::sites(&src, &env()).unwrap() {
            let ej = refs::eject(&src, &env(), s.span.start).unwrap().unwrap();
            assert_eq!(nets(&compiled(&ej)), nets(&comp), "{}: eject of {}", p.display(), s.call);
        }
    }
    assert!(seen >= 1, "no example calls the generator");
}

/// Provenance (6670, 6680): a recipe's JSON-LD says what it calls (`isBasedOn`, each with the CID it `resolves` to and the
/// generator version pinned) and the CID it expands to; the written-out program plays the same, and its CID is stable. An
/// ejected program says what it vendored, from its `# from:` line, though its CID is that of the notes alone.
#[test]
fn provenance_is_carried_in_the_json_ld() {
    beatscript_api::generate::install();
    let src = "tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: kick x x x x | bass bed\n}\narrange groove\n";
    let c = compiled(src);
    let doc = djlive::progld::document(&c, &djlive::progld::Facts::default()).js_stringified();
    let site = &refs::sites(src, &env()).unwrap()[0];
    let pin = site.resolved.clone().unwrap();
    assert!(doc.contains(r#""isBasedOn":[{"@type":"CreativeWork","name":"gen@2 techno seed=42 section=drop part=bass","resolves":""#), "{doc}");
    assert!(doc.contains(&format!(r#""resolves":"{pin}","pinnedGenerator":"seeded@2","derivation":"called""#)), "{doc}");
    let (text, x) = refs::expanded(src, &env()).unwrap().unwrap();
    assert_eq!(refs::expanded(src, &env()).unwrap().unwrap().1, x, "the expanded CID is stable");
    assert!(doc.contains(&format!(r#""expandsTo":"{x}""#)), "{doc}");
    let ce = compiled(&text);
    assert_eq!(ce.cid, x);
    assert_eq!(nets(&ce), nets(&c), "the recipe and its written-out program play the same");
    // ejected: the notes, and the call they came from
    let ej = refs::eject(src, &env(), src.find("gen@2").unwrap()).unwrap().unwrap();
    let cj = compiled(&ej);
    let dj = djlive::progld::document(&cj, &djlive::progld::Facts::default()).js_stringified();
    assert!(dj.contains(&format!(r#""name":"gen@2 techno seed=42 section=drop part=bass","resolves":"{pin}","pinnedGenerator":"seeded@2","derivation":"vendored""#)), "{dj}");
    assert!(!dj.contains("expandsTo"), "{dj}");
}

/// The recipe `examples/beatscript/vendored-beds.beatscript` was ejected from: two beds of one generated drop, called by their parameters.
/// BeatScript 1, as the example is; where the examples are BeatScript 2, put `#! beatscript 2` first here and in the example
/// (fmt), and the test says whether the eject's pins and the example still agree.
const VENDORED_RECIPE: &str = "#! beatscript 2\ntempo 124\nkey am\nlet drop_kick = gen@2 techno seed=42 section=drop part=kick\nlet drop_bass = gen@2 techno seed=42 section=drop part=bass\nsection intro 4 bars {\n  deck A: kick drop_kick kit=drums-v8 | hihat ~ x ~ x\n}\nsection drop 8 bars {\n  deck A: kick drop_kick kit=drums-v8 | hihat ~ x ~ x\n  deck B: bass drop_bass inst=drop-bass\n}\narrange intro drop\n";

/// `examples/beatscript/vendored-beds.beatscript` is what eject writes for that recipe, both lets written out: its notes (one CID with the
/// eject's text), and its `# from:` lines the calls and the CIDs they resolve to. It plays as the recipe does, and a save of
/// it keeps those lines as `meta.based_on` (`refs::from_lines`, what the storage gate reads).
#[test]
fn the_vendored_example_is_what_eject_writes() {
    beatscript_api::generate::install();
    let mut t = VENDORED_RECIPE.to_string();
    while let Some(s) = refs::sites(&t, &env()).unwrap().first().cloned() {
        t = refs::eject(&t, &env(), s.span.start).unwrap().unwrap();
    }
    let file = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript/vendored-beds.beatscript")).unwrap_or_default();
    let lines = refs::from_lines(&t);
    assert_eq!(lines.len(), 2, "{t}");
    assert!(lines.iter().all(|l| l.starts_with("gen@2 techno seed=42 section=drop part=")), "{lines:?}");
    assert_eq!(refs::from_lines(&file), lines, "the example's `# from:` lines are not the eject's; it writes:\n{t}");
    let (cf, ct) = (compiled(&file), compiled(&t));
    assert_eq!(cf.cid, ct.cid, "the example is not the eject's notes; it writes:\n{t}");
    assert_eq!(nets(&cf), nets(&compiled(VENDORED_RECIPE)), "the written-out program plays as the recipe");
    assert!(refs::sites(&file, &env()).unwrap().is_empty(), "the example still calls the generator");
}
