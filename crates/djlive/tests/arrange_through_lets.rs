//! An `arrange { }` mutation of a let's bars reaches the part that plays the let of them (mutate.rs, docs/beatscript/forms.md: "a part
//! plays the alternation; a mutation changes the bars, with a glob"). Before, a section read only the names written in its own
//! lines, so `~ ode_to_joy_* morph ...` changed nothing a part played when the part played `ode_to_joy`.

fn nets(src: &str) -> Vec<String> {
    let c = djlive::check(src).compiled.unwrap_or_else(|| panic!("{src}: {:?}", djlive::check(src).diagnostics));
    c.segments.iter().map(|s| s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>().join("|")).collect()
}

const LETS: &str = "key c\nlet m_1 = c4 d4 e4 f4\nlet m_2 = g4 a4 b4 c5\nlet m = <m_1 m_2>\n";

#[test]
fn a_mutation_of_the_bars_reaches_the_alternation() {
    let via = nets(&format!("{LETS}section s 2 bars {{\n  deck B: melody m\n}}\narrange {{\n  s\n  ~ m_* up 2\n  s\n  @ m_2 rev\n  s\n  s\n}}\n"));
    let direct = nets(&format!("{LETS}section s 2 bars {{\n  deck B: melody <m_1 m_2>\n}}\narrange {{\n  s\n  ~ m_* up 2\n  s\n  @ m_2 rev\n  s\n  s\n}}\n"));
    assert_ne!(via[0], via[1], "`~ m_* up 2` did not reach `melody m`");
    assert_ne!(via[1], via[2], "`@ m_2 rev` did not reach `melody m`");
    assert_eq!(via[1], via[3], "a one-shot lasts one section");
    // through the let or written in the section, the same music
    assert_eq!(via, direct);
}

#[test]
fn it_reads_through_lets_of_lets_and_the_table_says_so() {
    let src = format!("{LETS}let both = [m, m]\nsection s 2 bars {{\n  deck B: melody both\n}}\narrange {{\n  s\n  ~ m_1 down 1\n  s\n}}\n");
    let n = nets(&src);
    assert_ne!(n[0], n[1], "two lets deep, the mutation did not reach the part");
    let t = djlive::mutate::generations(&src, &djlive::env::Env::none()).unwrap();
    let reads: Vec<(String, u32)> = t.plays[1].reads.iter().map(|r| (r.pattern.clone(), r.generation)).collect();
    assert!(reads.contains(&("m_1".to_string(), 1)), "{reads:?}");
}

#[test]
fn an_unmutated_let_of_lets_is_the_program_it_was() {
    // no mutation reaches inside: the same sections and lets as before (no derived let), the same nets as a plain arrange
    let with = nets(&format!("{LETS}let k = x x x x\nsection s 2 bars {{\n  deck A: kick k\n  deck B: melody m\n}}\narrange {{\n  s\n  ~ k rot 1\n  s\n}}\n"));
    let plain = nets(&format!("{LETS}let k = x x x x\nsection s 2 bars {{\n  deck A: kick k\n  deck B: melody m\n}}\narrange s s\n"));
    assert_eq!(with[0], plain[0]);
}

#[test]
fn the_rondos_refrain_now_changes_pass_on_pass() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/template/form-rondo.beatscript")).unwrap();
    let c = djlive::check(&src).compiled.expect("rondo");
    let refrains: std::collections::BTreeSet<String> = c.segments.iter().filter(|s| s.name.starts_with("refrain")).map(|s| s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>().join("|")).collect();
    assert!(refrains.len() > 1, "every refrain plays the same: its plan's mutations of the_entertainer_* reach nothing");
}
