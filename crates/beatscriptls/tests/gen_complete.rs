//! Completion in a `gen@2` call (djlive::gen): after its head the installed generator's genres, then its names (`seed=` first),
//! then a name's values, `section=` the sections of the program the call prints; a part line's own params after the call; and
//! `gen@2` offered where a call goes (`arrange from`, `section NAME from`, `let NAME =`).

use beatscriptls::complete::{candidates, EnvNames};
use beatscriptls::Analysis;

/// The labels offered with the cursor where `|` is in `text`.
fn labels(text: &str) -> Vec<String> {
    btwapi::generate::install();
    let at = text.find('|').unwrap();
    let text = text.replacen('|', "", 1);
    candidates(&Analysis::new(&text), at, &EnvNames::web()).0.into_iter().map(|i| i.label).collect()
}

#[test]
fn a_gen_call_offers_the_genres_then_its_names_then_their_values() {
    let genres = labels("let bed = gen@2 |\n");
    let want = djlive::gen::generator().unwrap().genres(2);
    assert!(!want.is_empty() && want.iter().all(|g| genres.contains(g)), "{genres:?}");
    assert_eq!(labels("arrange from gen@2 tec|\n")[0], "techno");
    let names = labels("let bed = gen@2 techno |\n");
    assert_eq!(names[0], "seed=", "{names:?}");
    assert!(djlive::gen::PARAMS.iter().all(|p| names.contains(&format!("{p}="))), "{names:?}");
    let later = labels("let bed = gen@2 techno seed=42 |\n");
    assert!(!later.contains(&"seed=".to_string()) && later.contains(&"section=".to_string()) && later.contains(&"part=".to_string()), "{later:?}");
    assert_eq!(labels("let bed = gen@2 techno seed=42 length=|\n"), ["long", "short", "standard"]);
    assert!(labels("let bed = gen@2 techno seed=42 section=drop part=|\n").contains(&"bass".to_string()));
    assert!(labels("let bed = gen@2 techno seed=42 key=|\n").contains(&"am".to_string()));
}

#[test]
fn after_section_the_generated_programs_sections_are_offered() {
    let l = labels("tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=|\n");
    assert!(l.contains(&"drop".to_string()), "{l:?}");
    // they are the sections check takes
    for name in &l {
        let src = format!("tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section={name} part=kick\n");
        let c = djlive::compile::check_with(&src, &btwapi::web_env().env());
        assert!(!c.diagnostics.iter().any(|d| d.is_error() && d.message.contains("no section")), "{name}: {:?}", c.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }
}

#[test]
fn on_a_part_line_the_lines_params_follow_the_call() {
    let l = labels("section s 4 bars {\n  deck A: bass gen@2 techno seed=42 section=drop |\n}\n");
    assert!(l.contains(&"part=".to_string()) && l.contains(&"inst=".to_string()), "{l:?}");
    // a param of the line's ends the call: the line's own completion
    let v = labels("section s 4 bars {\n  deck A: bass gen@2 techno seed=42 section=drop inst=|\n}\n");
    assert!(!v.contains(&"techno".to_string()) && !v.contains(&"long".to_string()), "{v:?}");
}

#[test]
fn evolves_gen_is_not_a_call() {
    let l = labels("deck A: kick x ~ x ~\nevolve every 4 bars {\n  kick: rotate 1 when gen |\n}\n");
    assert!(!l.contains(&"techno".to_string()), "{l:?}");
}

#[test]
fn a_section_from_and_a_let_offer_a_gen_call() {
    assert_eq!(labels("section hook from |\n")[0], "gen@2");
    assert!(labels("let bed = |\n").contains(&"gen@2".to_string()));
}
