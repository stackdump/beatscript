//! The seeded generator: determinism, the program checks, and its body is canonical.

use beatscript_api::generate::{genres, generate, GenRequest};
use djengine::json::JsonValue;

fn req(genre: &str, seed: &str) -> GenRequest {
    GenRequest { genre: genre.into(), seed: seed.into(), ..Default::default() }
}

fn check(text: &str) -> JsonValue {
    beatscript_api::check_report(text)
}

fn cid(text: &str) -> String {
    check(text).get("cid").and_then(JsonValue::as_str).unwrap_or("").to_string()
}

#[test]
fn same_seed_same_text_and_cid() {
    for seed in ["1234", "sunrise"] {
        let a = generate(&req("techno", seed)).unwrap();
        let b = generate(&req("techno", seed)).unwrap();
        assert_eq!(a.text, b.text);
        assert!(!cid(&a.text).is_empty());
        assert_eq!(cid(&a.text), cid(&b.text));
        assert!(a.text.contains(&format!("#! seed: {seed}\n")));
        assert!(a.text.contains(&format!("#! generator: seeded@{}\n", beatscript_api::generate::GENERATOR_VERSION)));
        assert!(a.text.starts_with("#! beatscript 2\n"), "the current generator writes BeatScript 2");
    }
}

#[test]
fn every_genre_length_and_many_seeds_check_and_are_canonical() {
    for g in genres() {
        for len in ["short", "standard", "long"] {
            for seed in ["1", "2", "77", "1234", "dusk", "4294967295"] {
                let r = GenRequest { genre: g.clone(), length: len.into(), seed: seed.into(), ..Default::default() };
                let out = generate(&r).unwrap();
                let rep = check(&out.text);
                assert_eq!(rep.get("ok").and_then(JsonValue::as_bool), Some(true), "{g} {len} {seed}:\n{}\n{}", out.text, rep.js_stringified());
                assert_eq!(rep.get("errors").and_then(JsonValue::as_i64), Some(0));
                // fmt does not change the body: the generator prints canonical BeatScript (2: the version line first)
                let f = JsonValue::parse(&beatscript_api::fmt_json(&out.text)).unwrap();
                let canon = f.get("canonical").and_then(JsonValue::as_str).unwrap().to_string();
                assert_eq!(canon, format!("{}{}", djlive::version::CANONICAL_HEAD, out.body), "{g} {len} {seed}: not canonical");
                assert_eq!(f.get("cid").and_then(JsonValue::as_str).unwrap(), cid(&out.text));
            }
        }
    }
}

#[test]
fn different_seeds_different_programs() {
    let mut cids = std::collections::BTreeSet::new();
    for seed in 1..=12 {
        let out = generate(&req("house", &seed.to_string())).unwrap();
        cids.insert(cid(&out.text));
    }
    assert!(cids.len() >= 11, "only {} distinct programs from 12 seeds", cids.len());
    assert_ne!(cid(&generate(&req("techno", "a")).unwrap().text), cid(&generate(&req("techno", "b")).unwrap().text));
}

#[test]
fn inputs_are_honoured_and_checked() {
    let r = GenRequest { genre: "trance".into(), bpm: Some(150.0), key: "f#m".into(), length: "short".into(), seed: "9".into() };
    let out = generate(&r).unwrap();
    assert!(out.body.starts_with("tempo 150\nkey f#m\n"), "{}", out.body);
    assert!(generate(&GenRequest { genre: "polka".into(), ..Default::default() }).is_err());
    assert!(generate(&GenRequest { key: "h".into(), ..Default::default() }).is_err());
    let j = JsonValue::parse(&beatscript_api::generate_json(r#"{"genre":"dnb","seed":42}"#)).unwrap();
    assert_eq!(j.get("ok").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(j.get("seed").and_then(JsonValue::as_str), Some("42"));
}

/// Swing: a genre that swings in the beats tables (`swing`, 0..100: jazz 60, blues 50, house 20 ...) prints the program-wide
/// `swing` setting on BeatScript's 0..1 scale (the table's value / 100, at most 0.6), and a straight genre prints none. Every
/// version has done so since seeded@1 (docs/beatscript/ext-ffi.md, "Swing"); `seed(...)`, another generator, does not.
#[test]
fn a_swung_genre_prints_its_swing_in_every_version() {
    let tables = djengine::gen::tables::BeatsTables::shared();
    let mut swung = Vec::new();
    for g in genres() {
        let table = tables.genres.get(&g).map_or(0.0, |d| d.swing);
        let want = (table / 100.0).clamp(0.0, 0.6);
        for v in beatscript_api::generate::VERSIONS {
            let Ok(out) = beatscript_api::generate::generate_version(v, &req(&g, "42")) else { continue };
            let lines: Vec<&str> = out.body.lines().filter(|l| l.starts_with("swing ")).collect();
            let heard = check(&out.text).get("swing").and_then(JsonValue::as_f64);
            if want > 0.0 {
                assert_eq!(lines.len(), 1, "seeded@{v} {g} (table swing {table}):\n{}", out.body);
                assert_eq!(heard, Some(want), "seeded@{v} {g}: {lines:?}");
                assert!(out.body.starts_with(&format!("tempo {}\nkey {}\nswing ", djlive::print::num(out.bpm), out.key)), "seeded@{v} {g}:\n{}", out.body);
            } else {
                assert!(lines.is_empty() && heard.is_none(), "seeded@{v} {g} is straight (table swing {table}) and says {lines:?}");
            }
        }
        if want > 0.0 {
            swung.push(g);
        }
    }
    // (the walk met the swung genres it is about)
    for g in ["jazz", "blues", "bossa", "lofi", "funk", "house"] {
        assert!(swung.iter().any(|s| s == g), "{g} does not swing: {swung:?}");
    }
}

#[test]
fn reads_like_beatscript() {
    let out = generate(&req("techno", "1234")).unwrap();
    for needle in ["let pulse = e(", "arp(", "voice-lead", "markov(", "def groove(", "section ", "arrange ", " -> ", "every(4, [fill]"] {
        assert!(out.text.contains(needle), "missing {needle:?} in\n{}", out.text);
    }
}

// ---- the key: one line moves the whole song

use djlive::ast::Deck;
use djlive::compile::Compiled;

/// Every pitched part of every segment of the program, as (segment, deck, role, bar, step) -> the pitch classes
/// sounding there, so a program and its transposition can be laid side by side. Drums have no pitch to compare.
fn pitch_classes(text: &str) -> std::collections::BTreeMap<String, Vec<u8>> {
    let c: Compiled = djlive::compile::check(text).compiled.unwrap_or_else(|| panic!("does not compile:\n{text}"));
    let mut out = std::collections::BTreeMap::new();
    for (i, seg) in c.segments.iter().enumerate() {
        for (deck, ir) in &seg.decks {
            for p in ir.parts.iter().filter(|p| matches!(p.role.as_str(), "bass" | "melody" | "harmony" | "arp")) {
                for (bar, hits) in p.cycles.iter().enumerate() {
                    let mut by_step: std::collections::BTreeMap<u8, Vec<u8>> = std::collections::BTreeMap::new();
                    for h in hits {
                        by_step.entry(h.step).or_default().push(h.note % 12);
                    }
                    for (step, mut pcs) in by_step {
                        pcs.sort_unstable();
                        pcs.dedup();
                        out.insert(format!("{i}:{} {} {} bar {bar} step {step}", seg.name, if *deck == Deck::A { "A" } else { "B" }, p.role), pcs);
                    }
                }
            }
        }
    }
    out
}

fn up(pcs: &[u8], k: u8) -> Vec<u8> {
    let mut v: Vec<u8> = pcs.iter().map(|p| (p + k) % 12).collect();
    v.sort_unstable();
    v
}

/// `am` seven semitones up is `em`: the tonic's name moved, the mode word kept.
fn key_up(key: &str, k: usize) -> String {
    const NAMES: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    let split = if key.len() > 1 && matches!(key.as_bytes()[1], b'#' | b'b') { 2 } else { 1 };
    let (tonic, mode) = key.split_at(split);
    let pc = match tonic {
        "db" => 1,
        "eb" => 3,
        "gb" => 6,
        "ab" => 8,
        "bb" => 10,
        t => NAMES.iter().position(|n| *n == t).unwrap(),
    };
    format!("{}{mode}", NAMES[(pc + k) % 12])
}

#[test]
fn a_key_change_moves_every_pitched_note_of_every_genre() {
    // The old generator wrote the chords and the arps as absolute chord symbols: a `key` line moved the bass and the
    // melody and left the pad and the arp where they were (half the song).
    let (mut pads, mut arps) = (0, 0);
    for g in genres() {
        for seed in ["1", "77"] {
            let r = GenRequest { genre: g.clone(), seed: seed.into(), ..Default::default() };
            let out = generate(&r).unwrap();
            let before = pitch_classes(&out.text);
            pads += before.keys().filter(|k| k.contains(" harmony ")).count();
            arps += before.keys().filter(|k| k.contains(" arp ")).count();
            for k in [1usize, 5, 7] {
                let moved = out.text.replacen(&format!("\nkey {}\n", out.key), &format!("\nkey {}\n", key_up(&out.key, k)), 1);
                assert_ne!(moved, out.text);
                let after = pitch_classes(&moved);
                assert_eq!(before.len(), after.len(), "{g} {seed} +{k}");
                for (place, pcs) in &before {
                    assert_eq!(up(pcs, k as u8), after[place], "{g} {seed} +{k}: {place} did not move with the key\n{}", out.text);
                }
            }
        }
    }
    // (so the walk did meet the pads and the arps it is about)
    assert!(pads > 100 && arps > 100, "{pads} pad steps, {arps} arp steps");
}

#[test]
fn a_section_that_says_its_own_key_moves_as_a_whole() {
    let out = generate(&req("trance", "9")).unwrap();
    let before = pitch_classes(&out.text);
    // the first section that has a pad
    let name = out.text.lines().filter_map(|l| l.strip_prefix("section ")).map(|l| l.split(' ').next().unwrap()).find(|n| before.keys().any(|k| k.contains(&format!(":{n} B harmony")))).expect("a section with a pad").to_string();
    let header = out.text.lines().find(|l| l.starts_with(&format!("section {name} "))).unwrap().to_string();
    let lift = key_up(&out.key, 3);
    let text = out.text.replacen(&format!("{header}\n"), &format!("{header}\n  key {lift}\n"), 1);
    let after = pitch_classes(&text);
    for (place, pcs) in &before {
        let moves = place.split(' ').next().unwrap().ends_with(&format!(":{name}"));
        let want = if moves { up(pcs, 3) } else { pcs.clone() };
        assert_eq!(after[place], want, "{place} (section {name} says `key {lift}`)");
    }
}

#[test]
fn the_chords_and_the_arps_say_no_pitch() {
    for g in genres() {
        let out = generate(&req(&g, "5")).unwrap();
        for l in out.body.lines().filter(|l| l.starts_with("let chords") || l.starts_with("let arps") || l.starts_with("let lead")) {
            assert!(!l.contains("chord(") && !l.contains("markov(a") && !l.contains("markov(c") && !l.contains("markov(d") && !l.contains("markov(e") && !l.contains("markov(f") && !l.contains("markov(g") && !l.contains("markov(b"), "{g}: {l}");
        }
        assert!(out.body.contains("harmony chords") && out.body.contains("voice-lead=on"), "{g}");
    }
}
