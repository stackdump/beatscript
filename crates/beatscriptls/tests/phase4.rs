//! Phase 4: completion at the cursor, mid-line included.

mod common;

use beatscriptls::complete::{candidates, EnvNames, Item};
use beatscriptls::Analysis;
use common::*;

const PROGRAM: &str = include_str!("programs/symbols.beatscript");

/// The labels offered with `text` cut... no: with `insert` put at the `nth` `marker` of PROGRAM, the cursor after it.
fn offered_at(text: &str, at: usize) -> Vec<Item> {
    let a = Analysis::new(text);
    candidates(&a, at, &EnvNames::web()).0
}

/// The labels when the line `line` (with `@` as the cursor) is added at the end of PROGRAM (or inside a block: `in_block`
/// puts it before the given marker).
fn labels(line: &str, before: Option<&str>) -> Vec<String> {
    let cursor = line.find('@').unwrap();
    let line = line.replacen('@', "", 1);
    let (text, at) = match before {
        None => (format!("{PROGRAM}{line}\n"), PROGRAM.len() + cursor),
        Some(m) => {
            let k = PROGRAM.find(m).unwrap();
            (format!("{}{line}\n{}", &PROGRAM[..k], &PROGRAM[k..]), k + cursor)
        }
    };
    offered_at(&text, at).into_iter().map(|i| i.label).collect()
}

fn first(v: &[String], n: usize) -> Vec<&str> {
    v.iter().take(n).map(String::as_str).collect()
}

#[test]
fn after_deck_colon_the_roles_come_first() {
    let l = labels("deck A: @", None);
    assert_eq!(first(&l, 8), ["arp", "bass", "clap", "harmony", "hihat", "kick", "melody", "snare"]);
    assert!(l.contains(&"filter".to_string()));
}

#[test]
fn a_role_typed_halfway_is_completed() {
    let l = labels("deck B: bass riff | mel@", None);
    assert_eq!(first(&l, 1), ["melody"]);
}

#[test]
fn after_a_role_the_lets_then_steps_then_generators() {
    let l = labels("deck A: snare @", None);
    assert_eq!(first(&l, 4), ["bars8", "riff", "x", "~"]);
    assert!(l.contains(&"e".to_string()) && l.contains(&"vel=".to_string()));
}

#[test]
fn inst_offers_the_samples_then_the_voices() {
    let l = labels("deck B: bass riff inst=@", None);
    assert_eq!(l[0], "harp");
    assert!(l.contains(&"sub-bass".to_string()) && l.contains(&"warm-pad".to_string()));
    let l = labels("deck B: bass riff inst=sub@", None);
    assert_eq!(l[0], "sub-bass");
}

#[test]
fn arrange_offers_the_sections() {
    let l = labels("arrange intro @", None);
    assert_eq!(first(&l, 4), ["both", "drop", "intro", "outro"]);
}

#[test]
fn after_a_sections_length_bars_is_expected() {
    let l = labels("section x 8 ba@", None);
    assert_eq!(first(&l, 1), ["bars"]);
}

#[test]
fn a_do_line_offers_nouns_then_verbs_and_params() {
    let l = labels("do au@", None);
    assert!(l.contains(&"audio".to_string()), "{l:?}");
    let l = labels("do audio @", None);
    assert_eq!(first(&l, 3), ["devices", "input", "output"]);
    let l = labels("do audio output @", None);
    assert!(l.iter().any(|x| x.ends_with('=')), "{l:?}");
    let l = labels("do audio input channel=@", None);
    assert_eq!(first(&l, 3), ["L", "LR", "R"]);
}

#[test]
fn a_dollar_in_a_def_offers_its_parameters_and_ports() {
    let l = labels("  deck A: bass $@", Some("  deck B: harmony $chords"));
    assert_eq!(l, ["$chords", "$oct"]);
}

#[test]
fn a_wire_offers_the_defs_and_their_ports() {
    let l = labels("wire @", None);
    assert_eq!(l, ["bed", "changes"]);
    let l = labels("wire changes.out -> bed.@", None);
    assert_eq!(l, ["chords"]);
}

#[test]
fn after_a_parts_steps_every_param_the_parser_takes() {
    // held to djlive: a param the language adds (glide=, decay=, pitch-drop=) is offered and has its hover doc
    let l = labels("deck B: bass 1 3 @", None);
    for p in djlive::parse::PARAMS {
        assert!(l.contains(&format!("{p}=")), "`{p}=` is not offered: {l:?}");
        assert!(beatscriptls::lang::PART_PARAMS.iter().any(|(n, _, _)| *n == p), "lang::PART_PARAMS has no `{p}`");
    }
    for (n, _, _) in beatscriptls::lang::PART_PARAMS {
        assert!(djlive::parse::PARAMS.contains(n), "lang::PART_PARAMS offers `{n}`, which djlive does not parse");
    }
}

#[test]
fn inside_an_evolve_the_targets_then_the_operators() {
    let l = labels("    @", Some("    bass: rotate 1"));
    assert!(l[0].ends_with(':'), "{l:?}");
    let l = labels("    melody: @", Some("    bass: rotate 1"));
    assert_eq!(first(&l, 6), ["invert", "octave", "reverse", "rotate", "thin", "transpose"]);
}

#[test]
fn at_the_start_of_a_line_the_statements_and_the_defs() {
    let l = labels("@", None);
    assert_eq!(first(&l, 2), ["deck A:", "deck B:"]);
    for w in ["section", "arrange", "let", "bed", "changes", "evolve", "tempo"] {
        assert!(l.contains(&w.to_string()), "{w}: {l:?}");
    }
    // inside a section: no `section` or `arrange`
    let l = labels("  @", Some("  deck A: kick e(4,16)"));
    assert!(!l.contains(&"section".to_string()) && !l.contains(&"arrange".to_string()) && l.contains(&"repeat".to_string()));
}

#[test]
fn key_offers_keys_and_end_offers_its_forms() {
    let l = labels("key f@", None);
    assert_eq!(first(&l, 4), ["f", "f#", "f#m", "f:lydian"]);
    let l = labels("end @", None);
    assert_eq!(l, ["after", "at bar"]);
    let l = labels("end after @", None);
    assert!(l.contains(&"drop".to_string()));
}

#[test]
fn nothing_in_a_comment() {
    let l = labels("# deck A: @", None);
    assert!(l.is_empty(), "{l:?}");
}

#[test]
fn the_lsp_answer_replaces_the_word() {
    let text = format!("{PROGRAM}deck A: hih\n");
    let mut s = server(&text);
    let (l, c) = find(&text, "deck A: hih", 0, 11);
    let got = req(&mut s, "textDocument/completion", &at(l, c));
    let items = got.get("items").unwrap().as_array().unwrap();
    golden("completion-role", &djengine::json::JsonValue::Array(items.iter().take(2).cloned().collect()));
    assert_eq!(items[0].get("label").and_then(|v| v.as_str()), Some("hihat"));
}

#[test]
fn the_parsers_expected_maps_to_candidates() {
    let a = Analysis::new("x");
    let got: Vec<String> = beatscriptls::complete::expected_items(&a, &["a setting (tempo, key, swing)".into(), "`deck A:`".into(), "a step".into()], 0).into_iter().map(|i| i.label).collect();
    assert_eq!(got, ["tempo", "key", "swing", "deck A:", "x", "~"]);
}

