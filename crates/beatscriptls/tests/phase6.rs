//! Phase 6: inlay hints.

mod common;

use beatscriptls::inlay::hints;
use beatscriptls::Analysis;
use common::*;

const PROGRAM: &str = include_str!("programs/symbols.btw");

/// (line, label) of every hint.
fn labelled(a: &Analysis) -> Vec<(usize, String)> {
    hints(a).into_iter().map(|h| (a.index.line_of(h.at) + 1, h.label)).collect()
}

#[test]
fn sections_arrange_parts_and_evolves() {
    let a = Analysis::checked(PROGRAM);
    let got = labelled(&a);
    let want: Vec<(usize, String)> = [
        (19, "bar 1 · 0:00"),
        (20, "4 steps"),
        (20, "8 steps"),
        (21, "1 step"),
        (25, "bar 9 · 0:15"),
        (26, "1 step"),
        (26, "4 steps"),
        (27, "1 step"),
        (28, "4 gens"),
        (33, "bar 1 · 0:00"),
        (34, "4 steps"),
        (38, "24 bars · 0:46"),
    ]
    .iter()
    .map(|(l, s)| (*l, s.to_string()))
    .collect();
    assert_eq!(got, want);
    let mut s = server(PROGRAM);
    golden("inlay-hints", &req(&mut s, "textDocument/inlayHint", &format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":17,"character":0}},"end":{{"line":30,"character":0}}}}}}"#)));
}

#[test]
fn an_evolve_that_does_not_fit_its_section_says_so() {
    let text = "tempo 120\nsection a 16 bars {\n  deck A: kick x ~ x ~ | hihat x*8\n  evolve every 4 bars {\n    hihat: rotate 1\n    stop when gen == 8\n  }\n}\narrange a\n";
    let a = Analysis::new(text);
    let warn: Vec<_> = hints(&a).into_iter().filter(|h| h.warn).collect();
    assert_eq!(warn.len(), 1);
    assert_eq!(warn[0].label, "⚠ only 4 of 9 gens fit");
    assert!(warn[0].tooltip.contains("the last 5 never play"), "{}", warn[0].tooltip);
}

#[test]
fn an_hour_long_song_totals_without_the_check() {
    // 1800 bars at 120 BPM: an hour, from the text while the check runs
    let mut t = String::from("tempo 120\n");
    for n in ["a", "b", "c"] {
        t.push_str(&format!("section {n} 100 bars {{\n  deck A: kick x ~ x ~\n}}\n"));
    }
    t.push_str(&format!("arrange {}\n", ["a b c"; 6].join(" ")));
    let a = Analysis::new(t.as_str());
    let total = hints(&a).into_iter().find(|h| h.label.contains("bars ·")).unwrap();
    assert_eq!(total.label, "1800 bars · 1:00:00");
    let a_hint = hints(&a).into_iter().next().unwrap();
    assert_eq!(a_hint.label, "bars 1, 301, 601, 901, 1201, 1501");
}

#[test]
fn steps_count_fast_terms_and_groups() {
    let a = Analysis::new("deck A: hihat x*8 | snare [x x] ~ x ~ | bass/12 c2 ~ e2\n");
    let got: Vec<String> = hints(&a).into_iter().map(|h| h.label).collect();
    assert_eq!(got, ["8 steps", "4 steps", "3 steps of /12"]);
}
