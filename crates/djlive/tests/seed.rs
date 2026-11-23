//! `seed(genre, n[, structure])` (seed.rs, docs/BEATSCRIPT.md "Start from a seed"):
//! - the track a seed names is beats.bitwrap.io's: the generator's model CID is the one beats' own JavaScript wrote;
//! - its BeatScript plays the notes the generator plays: every onset and pitch, bar by bar, for a sample of genres,
//!   seeds and structures (velocity and length are a part's, so those are near, not exact; measured below);
//! - expand(p) plays as p does, and the CID of p (unexpanded) is stable;
//! - check's errors and warnings.

use std::collections::BTreeSet;

use djengine::json::JsonValue;
use djlive::compile::Compiled;
use djlive::diag::Severity;
use djlive::seed;

fn compiled(src: &str) -> Compiled {
    let c = djlive::check(src);
    let errs: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{:?} {}", d.span, d.message)).collect();
    c.compiled.unwrap_or_else(|| panic!("{src}\n{errs:#?}"))
}

/// (bar from 0, step, role, note) of every hit a compiled song plays, from its timeline.
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

fn generated(t: &seed::Track) -> BTreeSet<(u32, u8, String, u8)> {
    let mut out = BTreeSet::new();
    for p in &t.parts {
        for (b, bar) in p.bars.iter().enumerate() {
            for (s, slot) in bar.iter().enumerate() {
                for n in slot {
                    out.insert((b as u32, s as u8, p.role.to_string(), n.note));
                }
            }
        }
    }
    out
}

fn fixture() -> Vec<JsonValue> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seed/beats-projects.json")).unwrap();
    JsonValue::parse(&text).unwrap().get("rows").and_then(JsonValue::as_array).unwrap().clone()
}

fn row(r: &JsonValue) -> (String, u32, String, String) {
    let s = |k: &str| r.get(k).and_then(JsonValue::as_str).unwrap().to_string();
    (s("genre"), r.get("seed").and_then(JsonValue::as_f64).unwrap() as u32, s("structure"), s("modelCID"))
}

/// The track of a seed is the project beats' own JavaScript generates: the same model CID, for every one of the
/// fixture's genres at seed 42 and a few structures at seed 1.
#[test]
fn a_seed_is_the_track_beats_generates() {
    let rows = fixture();
    assert!(rows.len() >= 19);
    for r in &rows {
        let (g, n, st, cid) = row(r);
        let t = seed::track(&g, n, &st);
        assert_eq!(t.model_sha, cid, "seed({g}, {n}, {st}): not beats' project");
        assert_eq!(t.tempo, r.get("tempo").and_then(JsonValue::as_f64).unwrap(), "seed({g}, {n}, {st}) tempo");
    }
}

/// `arrange from seed(...)` plays every note the generator plays, where it plays it, and nothing else.
#[test]
fn its_beatscript_plays_the_generators_notes() {
    let mut vel_err = (0u64, 0u64);
    for r in fixture() {
        let (g, n, st, _) = row(&r);
        let src = format!("arrange from seed({g}, {n}, {st})\n");
        let c = compiled(&src);
        let t = seed::track(&g, n, &st);
        let want = generated(&t);
        let got = played(&c);
        let missing: Vec<_> = want.difference(&got).take(5).collect();
        let extra: Vec<_> = got.difference(&want).take(5).collect();
        assert!(missing.is_empty() && extra.is_empty(), "seed({g}, {n}, {st}): {} notes, missing {missing:?}, extra {extra:?}", want.len());
        if t.looped {
            assert!(c.looped && c.end.is_none(), "seed({g}, {n}, loop) goes round");
        } else {
            assert_eq!(c.end, Some(t.bars), "seed({g}, {n}, {st}) length");
        }
        assert_eq!(c.tempo, Some(t.tempo));
        // velocities: a part's range and its accents, so near (reported, and held to a loose bound)
        for seg in &c.segments {
            for deck in seg.decks.values() {
                for p in &deck.parts {
                    let pt = t.part(&p.role).unwrap();
                    for k in 0..seg.bars {
                        for h in &p.cycles[k as usize % p.cycles.len()] {
                            let src_note = pt.bars[(seg.start + k) as usize][h.step as usize].iter().find(|x| x.note == h.note).unwrap();
                            vel_err.0 += u64::from(src_note.vel.abs_diff(h.vel));
                            vel_err.1 += 1;
                        }
                    }
                }
            }
        }
    }
    let mean = vel_err.0 as f64 / vel_err.1 as f64;
    eprintln!("velocity: mean |error| {mean:.1} over {} notes", vel_err.1);
    assert!(mean < 12.0, "velocities drifted: mean |error| {mean:.1}");
}

/// expand(p) is plain BeatScript that plays as p plays; p's own CID is of its text with the references in it.
#[test]
fn expand_plays_the_same_and_the_reference_keeps_its_cid() {
    let progs = [
        "arrange from seed(techno, 42)\n",
        "tempo 126\nkey am\nsection intro from seed(house, 42)[1..8]\nsection drop from seed(house, 42)[17..28]\narrange intro drop\n",
        "tempo 140\ndeck A: kick x x x x | bass seed(trance, 42).bass[17..24] inst=fm-bass\ndeck B: arp seed(trance, 42)[17..20]\n",
        "let riff = seed(dnb, 1, loop).bass[1..4]\nsection groove 8 bars {\n  deck A: bass riff\n}\narrange groove groove\n",
    ];
    for p in progs {
        let c = compiled(p);
        let e = match seed::expand(p) {
            Some(Ok(e)) => e,
            other => panic!("{p}: {other:?}"),
        };
        // (a `# from seed(...)` comment says where it came from; no reference is left)
        assert!(seed::expand(&e.text).is_none(), "{}", e.text);
        let ce = compiled(&e.text);
        assert_eq!(played(&c), played(&ce), "{p}");
        let env = |c: &Compiled| c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>())).collect::<Vec<_>>();
        assert_eq!(env(&c), env(&ce), "{p}: the nets differ");
        assert_eq!((c.tempo, c.key, c.end, c.looped), (ce.tempo, ce.key, ce.end, ce.looped));
        // the reference's CID is of the reference, not of what it stands for
        assert_ne!(c.cid, ce.cid);
        assert!(c.text.contains("seed("), "{}", c.text);
        // and the canonical text is a program that is the same program
        let again = compiled(&c.text);
        assert_eq!(again.cid, c.cid);
        assert_eq!(played(&again), played(&c));
    }
}

/// The same reference, however it is spelt, is the same canonical text and CID; a different one is not.
#[test]
fn a_reference_is_spelt_one_way() {
    let a = compiled("arrange from seed(techno, 42)\n");
    let b = compiled("arrange   from seed( techno ,42 , standard )\n");
    assert_eq!(a.text, "arrange from seed(techno, 42)\n");
    assert_eq!(a.cid, b.cid);
    assert_ne!(a.cid, compiled("arrange from seed(techno, 43)\n").cid);
    assert_ne!(a.cid, compiled("arrange from seed(techno, 42, loop)\n").cid);
    let c = compiled("deck A: bass seed(techno, 42)[5..5]\n");
    assert!(c.text.contains("seed(techno, 42)[5]"), "{}", c.text);
}

/// Seeds v1 are frozen: the same reference is the same music in every build. The model CID, the expansion and the program
/// CID of `arrange from seed(techno, 42)` are pinned; a change to the generator that moves them is seeds v2.
#[test]
fn seeds_v1_are_pinned() {
    let t = seed::track("techno", 42, "standard");
    assert_eq!(t.name, "techno · Rusted Spine");
    let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seed/techno-42.beatscript")).unwrap();
    let e = match seed::expand("arrange from seed(techno, 42)\n") {
        Some(Ok(e)) => e,
        other => panic!("{other:?}"),
    };
    if std::env::var("BEATSCRIPT_BLESS").is_ok() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seed/techno-42.beatscript"), &e.text).unwrap();
    } else {
        assert_eq!(e.text, golden, "seed(techno, 42) no longer writes out as it did: that is seeds v2 (BEATSCRIPT_BLESS=1 only if it is meant)");
    }
    // the program CID of the one-line reference
    assert_eq!(compiled("arrange from seed(techno, 42)\n").cid, "z4EBG9jEPpNLAqXAf6Rz4KkpfPnEWFKNhK2vxXhx2Hqzcx3CcoE");
    // and the written-out program is a program of its own, which plays the same
    assert_eq!(played(&compiled(&golden)), played(&compiled("arrange from seed(techno, 42)\n")));
}

fn diags(src: &str) -> Vec<(Severity, String, String)> {
    djlive::check(src).diagnostics.into_iter().map(|d| (d.severity, d.span.text(src).to_string(), format!("{} {}", d.message, d.help.unwrap_or_default()))).collect()
}

#[test]
fn check_names_what_is_wrong() {
    // an unknown genre: an error with the nearest names
    let d = diags("arrange from seed(tecno, 42)\n");
    assert!(d.iter().any(|(s, at, m)| *s == Severity::Error && at == "tecno" && m.contains("`techno`")), "{d:?}");
    // bars outside the track
    let d = diags("deck A: bass seed(techno, 42)[50..70]\n");
    assert!(d.iter().any(|(s, at, m)| *s == Severity::Error && at == "[50..70]" && m.contains("56 bars")), "{d:?}");
    // a part the genre does not have: a warning, and the part is silent
    let c = djlive::check("deck B: arp seed(techno, 42)[1..4]\n");
    assert!(c.ok());
    assert!(c.diagnostics.iter().any(|d| d.severity == Severity::Warning && d.message.contains("has no arp")), "{:?}", c.diagnostics);
    // a part that is no role
    let d = diags("deck A: bass seed(techno, 42).bas[1..4]\n");
    assert!(d.iter().any(|(s, at, m)| *s == Severity::Error && at == "bas" && m.contains("`.bass`")), "{d:?}");
    // a let says which part
    let d = diags("let riff = seed(techno, 42)[1..4]\n");
    assert!(d.iter().any(|(s, _, m)| *s == Severity::Error && m.contains("say which")), "{d:?}");
    // a structure the generator does not have
    let d = diags("arrange from seed(techno, 42, huge)\n");
    assert!(d.iter().any(|(s, at, _)| *s == Severity::Error && at == "huge"), "{d:?}");
    // bars that run backwards, a seed that is no whole number
    assert!(diags("deck A: bass seed(techno, 42)[8..1]\n").iter().any(|(s, _, m)| *s == Severity::Error && m.contains("backwards")));
    assert!(diags("deck A: bass seed(techno, 4.5)\n").iter().any(|(s, _, m)| *s == Severity::Error && m.contains("whole number")));
    // somewhere a pattern does not go
    assert!(diags("master: filter lp seed(techno, 42)\n").iter().any(|(s, _, _)| *s == Severity::Error));
    // a version this BeatScript does not have
    let d = diags("#! seeds: v2\narrange from seed(techno, 42)\n");
    assert!(d.iter().any(|(s, _, m)| *s == Severity::Error && m.contains("seeds v1")), "{d:?}");
    assert!(djlive::check("#! seeds: v1\narrange from seed(techno, 42)\n").ok());
    // an error inside what a seed wrote out is at the reference (a part said twice on one deck)
    let src = "deck A: bass seed(techno, 42)[17..20] | bass x\n";
    let c = djlive::check(src);
    for d in c.diagnostics.iter().filter(|d| d.is_error()) {
        assert!(d.span.end <= src.len(), "{d:?}");
    }
}

/// A part's own params win over the ones the seed would write; the rest are written.
#[test]
fn a_part_keeps_its_own_params() {
    let e = match seed::expand("deck A: bass seed(techno, 42)[17..20] inst=fm-bass\n") {
        Some(Ok(e)) => e.text,
        other => panic!("{other:?}"),
    };
    assert!(e.contains("inst=fm-bass") && !e.contains("inst=reese"), "{e}");
    assert!(e.contains("vel="), "{e}");
    // a voice whose name starts with a digit cannot be written: its letter-named kin stands in
    let e = match seed::expand("deck A: bass seed(dubstep, 42)[17..20]\n") {
        Some(Ok(e)) => e.text,
        other => panic!("{other:?}"),
    };
    assert!(e.contains("inst=sub-bass"), "{e}");
    assert!(djlive::check(&e).ok());
}

/// Every genre's whole track, at a seed the fixture has not got, checks and is within the language's bounds.
#[test]
fn every_genre_checks() {
    for g in seed::GENRES {
        let c = compiled(&format!("arrange from seed({g}, 7)\n"));
        let e = match seed::expand(&format!("arrange from seed({g}, 7)\n")) {
            Some(Ok(e)) => e.text,
            other => panic!("{other:?}"),
        };
        assert!(e.len() < 16 * 1024, "seed({g}, 7) writes out {} bytes", e.len());
        assert!(c.end.is_some());
    }
}
