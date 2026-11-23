//! Phase 5: hover and signature help.

mod common;

use common::*;
use djengine::json::JsonValue;

const PROGRAM: &str = include_str!("programs/symbols.beatscript");

fn hover_text(s: &mut beatscriptls::Server, text: &str, needle: &str, nth: usize, skip: u32) -> String {
    let (l, c) = find(text, needle, nth, skip);
    let h = req(s, "textDocument/hover", &at(l, c));
    h.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or("").to_string()
}

#[test]
fn a_section_says_its_bars_and_seconds_at_the_tempo() {
    let mut s = server(PROGRAM);
    // intro is `bars8` = 8 bars at 124 BPM, placed first (in `both`, with outro)
    let h = hover_text(&mut s, PROGRAM, "section intro", 0, 9);
    assert!(h.starts_with("```beatscript\nsection intro bars8 bars\n```"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "section drop", 0, 9);
    // drop: 16 bars after both (8 bars): bars 9-24, 0:31 long from 0:15 at 124 BPM
    assert!(h.contains("16 bars, bars 9–24 · 0:31 long, from 0:15 to 0:46 at 124 BPM"), "{h}");
    golden("hover-section", &req(&mut s, "textDocument/hover", &{
        let (l, c) = find(PROGRAM, "section drop", 0, 9);
        at(l, c)
    }));
}

#[test]
fn before_the_check_the_text_says_where_a_plain_arrange_puts_a_section() {
    let text = "tempo 120\nlet n = 4 * 2\nsection a n bars {\n  deck A: kick x\n}\nsection b 16 bars {\n  deck A: kick x x\n}\narrange a b a\n";
    let a = beatscriptls::Analysis::new(text);
    assert!(a.check_ready().is_none());
    let md = beatscriptls::hover::symbol_markdown(&a, a.symbols.named("b").next().unwrap().0);
    assert!(md.contains("16 bars, bars 9–24 · 0:32 long, from 0:16 to 0:48 at 120 BPM") && md.contains("from the text"), "{md}");
    let md = beatscriptls::hover::symbol_markdown(&a, a.symbols.named("a").find(|(_, s)| s.kind == beatscriptls::symbols::Kind::Section).unwrap().0);
    assert!(md.contains("placed 2 times"), "{md}");
    // a let of arithmetic says what it came to
    let md = beatscriptls::hover::symbol_markdown(&a, a.symbols.named("n").next().unwrap().0);
    assert!(md.contains("let n = 4 * 2") && md.contains("= 8"), "{md}");
}

#[test]
fn generators_params_roles_and_voices() {
    let mut s = server(PROGRAM);
    let h = hover_text(&mut s, PROGRAM, "e(4,16)", 0, 0);
    assert!(h.starts_with("```beatscript\ne(k, n, [rot])\n```") && h.contains("Euclidean"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "oct=$oct", 0, 1);
    assert!(h.contains("octave"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "inst=sub-bass", 0, 6);
    assert_eq!(h, "`sub-bass`: a voice with a sound of its own");
    let h = hover_text(&mut s, PROGRAM, "hihat x*8", 0, 1);
    assert_eq!(h, "`hihat`: the hi-hat part");
    let h = hover_text(&mut s, PROGRAM, "evolve every", 0, 1);
    assert!(h.starts_with("`evolve`:"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "rotate 1", 0, 1);
    assert!(h.starts_with("`rotate` (evolve)"), "{h}");
}

#[test]
fn a_def_a_parameter_and_a_let() {
    let mut s = server(PROGRAM);
    let h = hover_text(&mut s, PROGRAM, "bed(3)", 0, 1);
    // (the call and the wire)
    assert!(h.contains("def bed(oct) in: chords") && h.contains("used 2 times"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "$oct", 0, 2);
    assert!(h.contains("parameter of `def bed`, used once"), "{h}");
    let h = hover_text(&mut s, PROGRAM, "bass riff", 0, 6);
    assert!(h.contains("let riff = c2 ~ [e2 g2] ~") && h.contains("used 2 times"), "{h}");
}

#[test]
fn a_do_line_says_the_noun_verb_and_param() {
    let text = "do audio input channel=L\n";
    let mut s = server(text);
    let h = hover_text(&mut s, text, "audio", 0, 1);
    assert!(h.contains("do audio") && h.contains("`devices` (read)"), "{h}");
    let h = hover_text(&mut s, text, "input", 0, 1);
    assert!(h.contains("do audio input"), "{h}");
    let h = hover_text(&mut s, text, "channel", 0, 1);
    assert!(h.contains("one of `L`, `R`, `LR`"), "{h}");
}

#[test]
fn signature_help_in_a_generator_a_def_and_a_do_line() {
    let text = format!("{PROGRAM}deck A: kick e(5, \n");
    let mut s = server(&text);
    let (l, c) = find(&text, "e(5, ", 0, 5);
    let got = req(&mut s, "textDocument/signatureHelp", &at(l, c));
    golden("signature-euclid", &got);
    assert_eq!(got.get("activeParameter").and_then(JsonValue::as_i64), Some(1));
    // a def's parameters
    let text = format!("{PROGRAM}section x 4 bars {{\n  bed(\n}}\n");
    let mut s = server(&text);
    let (l, c) = find(&text, "  bed(", 0, 6);
    let got = req(&mut s, "textDocument/signatureHelp", &at(l, c));
    assert_eq!(got.get("signatures").and_then(|v| v.at(0)).and_then(|v| v.get("label")).and_then(JsonValue::as_str), Some("bed(oct)"));
    // nested: inside the inner call
    let text = "deck A: kick maybe([x ~], 0.5, rev\n";
    let mut s = server(text);
    let got = req(&mut s, "textDocument/signatureHelp", &at(0, text.len() as u32 - 1));
    assert_eq!(got.get("activeParameter").and_then(JsonValue::as_i64), Some(2));
    // a do line: the param being typed
    let text = "do audio input channel=\n";
    let mut s = server(text);
    let got = req(&mut s, "textDocument/signatureHelp", &at(0, 23));
    let label = got.get("signatures").and_then(|v| v.at(0)).and_then(|v| v.get("label")).and_then(JsonValue::as_str).unwrap().to_string();
    assert!(label.starts_with("do audio input "), "{label}");
    let active = got.get("activeParameter").and_then(JsonValue::as_i64).unwrap() as usize;
    let p = got.get("signatures").unwrap().at(0).unwrap().get("parameters").unwrap().at(active).unwrap().get("label").unwrap().ints().unwrap();
    assert!(label[p[0] as usize..p[1] as usize].starts_with("channel="), "{label} {p:?}");
}

#[test]
fn nothing_on_a_comment_or_blank() {
    let mut s = server(PROGRAM);
    assert_eq!(req(&mut s, "textDocument/hover", &at(0, 3)), JsonValue::Null);
    assert_eq!(req(&mut s, "textDocument/hover", &at(7, 0)), JsonValue::Null);
}
