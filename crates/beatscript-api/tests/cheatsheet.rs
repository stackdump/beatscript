//! The cheat sheet is generated from docs/BEATSCRIPT.md: it has the headings and lines the compose prompt needs,
//! leaves out what a prompt does not, is the same every time, and says so when BEATSCRIPT.md moves out from under it.

mod support;

use beatscript_api::cheatsheet::{build, BEATSCRIPT_MD};

#[test]
fn the_plan_finds_everything_it_asks_for_in_btw_md() {
    let (sheet, missing) = build(BEATSCRIPT_MD);
    assert!(missing.is_empty(), "BEATSCRIPT.md no longer has what the cheat sheet takes from it: {missing:?}");
    assert!(!sheet.is_empty());
}

#[test]
fn it_is_deterministic_and_is_what_the_api_serves() {
    let a = beatscript_api::cheatsheet();
    assert_eq!(a, beatscript_api::cheatsheet());
    assert_eq!(a, build(BEATSCRIPT_MD).0);
    assert!(a.ends_with('\n') && !a.contains("\r") && !a.contains("\t"));
}

#[test]
fn it_is_compact() {
    let len = beatscript_api::cheatsheet().len();
    // 7 to 12 KB was the target; every block the brief names (the two EBNF blocks, the tables, the structure and
    // evolve sections) was about 15 KB, and the language extensions (arithmetic, tempo maps, assertions, sets, ...)
    // brought it to about 25 KB. Past 30 KB the compose prompt costs too much: trim BEATSCRIPT.md's sheet plan instead
    assert!((7_000..=30_000).contains(&len), "the cheat sheet is {len} bytes");
}

#[test]
fn it_has_the_headings_of_the_language() {
    let sheet = beatscript_api::cheatsheet();
    for h in [
        "# BeatScript cheat sheet",
        "## The rules in one screen",
        "## Grammar (EBNF)",
        "## Patterns: mini-notation",
        "### Generators",
        "### Parameters",
        "### Settings",
        "## Control",
        "### Time words",
        "## The whole surface: `do`",
        "### The wheel",
        "## Functions and structure",
        "### Ending",
        "### Evolving",
        "### Pattern transforms",
        "### Start from a seed",
        "## Sounds",
        "## On beatscript.app",
    ] {
        assert!(sheet.lines().any(|l| l == h), "no heading `{h}`");
    }
}

/// Lines that are in BEATSCRIPT.md word for word and in the sheet: when BEATSCRIPT.md changes one, this test says which, so
/// the sheet is looked at rather than left to drift.
#[test]
fn its_key_lines_are_btw_mds_own() {
    let sheet = beatscript_api::cheatsheet();
    for line in [
        "- Time is **bars** (four beats), and a beat is a quarter of a bar: `over 2 beats`, `at bar 16 beat 3`. Seconds appear",
        "- Text in, same net out: the same program text always gives the same net CIDs and the same program CID.",
        "| `every 16 bars:` | on bars 16, 32, 48 ... (never on bar 0) |",
        "| `before drop 4 bars:` | once, four bars ahead of the section: the roll that leads into a drop (`before drop 2 beats:` is a half bar ahead) |",
        "| `end after outro` | where the section `outro` ends (the song stops there, later sections never play) |",
        "| `rotate n` | every hit `n` steps (sixteenths) along its bar, wrapping |",
        "| `thin p [seed=s]` | drops the share `p` of the hits, picked by a hash of (seed, generation, bar, step, note): the same seed, the same hits |",
        "| `arp(chord, mode, n)` | the chord's notes one at a time over `n` steps (default 8) in `up`, `down` or `updown`. |",
        "| `vel=90` / `vel=70..100` | 1..127 | velocity. A range gives each hit its own, by a hash of its place: the same text, the same velocities. |",
        "| `degrade(p, a)` / `degrade(p, a, seed)` | drops the share `a` of the hits: seeded, so the same text is the same hits |",
        "- **`section name N bars { ... }`** and **`arrange a b c`**: the song is the sections in that order (a section",
        "  melody: rotate 1 | transpose <0 5 7 0>",
        "    stop when gen == 3",
        "Marks touch their step: `c2!` an accent (velocity +27), `x.` a ghost (40%), `e2^` a slide (held into the next",
        "`tempo 126` (BPM, 40..300), `swing 0.1` (0..1), `key am`.",
        "every 16 bars: freeze phrase bars=2",
        "do <noun> [<verb>] [<positional text>] key=value ... [over N bars [shape]]",
        "| `inst=wobble-lead` | a voice |",
    ] {
        assert!(BEATSCRIPT_MD.contains(line), "docs/BEATSCRIPT.md no longer says: {line}");
        assert!(sheet.contains(line), "the cheat sheet lost: {line}");
    }
}

#[test]
fn the_grammar_is_one_block_with_the_booths_lines_left_out() {
    let sheet = beatscript_api::cheatsheet();
    let ebnf: Vec<&str> = sheet.split("```ebnf\n").skip(1).map(|b| b.split("```").next().unwrap()).collect();
    assert_eq!(ebnf.len(), 1, "the base grammar and its additions are one block");
    let g = ebnf[0];
    for p in [
        // grammar.ebnf, the one grammar (crates/djlive/grammar.ebnf), as the sheet prints it
        "statement = setting | headed | let | def | repeat | section | arrange | arrange_block | start | launch | end | loop",
        "evolve = \"evolve\" \"every\" INT \"bars\" \"{\" { NEWLINE } { rule NEWLINE { NEWLINE } } [ bound NEWLINE { NEWLINE } ] \"}\" ;",
        "when = \"every\" dur [ \"from\" ( arith | pos ) ]",
        "call = NAME \"(\" [ arg { \",\" arg } ] \")\" ;",
        "invoke = NAME \"(\" [ tokens { \",\" tokens } ] \")\" ;",
        "headed = \"deck\" deck \":\" parts [ guard ]",
        "control = action | engine | do | sing ;",
        // the control deck (docs/beatscript/ext-control.md): of its engine calls the web plays a timed tempo, read as the setting's
        "\"control\" \":\" ( engine | do | duck | mix ) [ cguard ]",
        "engine = \"tempo\" tempo",
        "setting = \"tempo\" tempo",
        "action = \"filter\" ( \"lp\" | \"hp\" ) change",
        "\"resonance\" ) change",
        "term = atom [ mark ] [ \"*\" ( INT | \"(\" arith \")\" ) ] { \"?\" [ prob ] } [ \"~\" INT ]",
        "do = ( \"do\" WORD | SUGAR ) [ WORD ]",
        "part = role [ \"/\" INT ] pattern { param }",
        "arrange_block = \"arrange\" \"{\"",
        "mut_target = GLOB | NAME ;",
    ] {
        assert!(g.contains(p), "the grammar lost `{p}`:\n{g}");
    }
    // the later definition replaced the earlier one of its name
    for name in ["statement", "headed", "control", "engine", "when", "value", "term", "arg", "param", "target", "mut_target"] {
        assert_eq!(g.lines().filter(|l| l.starts_with(&format!("{name} = "))).count(), 1, "{name} is defined once");
    }
    // the desktop booth's actions and the snippet import are not for the web
    for gone in ["\"swap\"", "\"crossfader\"", "\"queue\"", "MOVE", "SPEC", "import"] {
        assert!(!g.contains(gone), "`{gone}` is still in the grammar");
    }
    // every production is closed
    for l in g.lines() {
        let code = l.split("(*").next().unwrap().trim_end();
        assert!(!code.is_empty());
    }
}

#[test]
fn what_a_prompt_does_not_need_is_not_there() {
    // (the closing note names the booth's lines on purpose, to tell a model to leave them out)
    let full = beatscript_api::cheatsheet();
    let sheet = full.split("## On beatscript.app").next().unwrap();
    for gone in [
        "Not done",
        "Restricted sounds",
        "peak RSS",
        "render speed",
        "hand-dubstep",
        "Example: a whole track",
        "One song, two texts",
        "## Tests",
        "Seeing it play",
        "Saved programs",
        "Share links",
        "The wheel from code",
        "swap pairs",
        "crossfader",
        "route=td3",
        "djrust::",
        "DeckAction",
    ] {
        assert!(!sheet.contains(gone), "the cheat sheet still has `{gone}`");
    }
}

#[test]
fn a_btw_md_that_moved_is_reported_not_papered_over() {
    let renamed = BEATSCRIPT_MD.replace("## Grammar (EBNF)", "## The grammar");
    let (sheet, missing) = build(&renamed);
    assert!(missing.iter().any(|m| m.contains("## Grammar (EBNF)")), "{missing:?}");
    assert!(!sheet.contains("```ebnf") || missing.len() >= 2, "{missing:?}");
    let reworded = BEATSCRIPT_MD.replace("A **range** `a..b`", "A range `a..b`");
    let (_, missing) = build(&reworded);
    assert!(missing.iter().any(|m| m.contains("A **range**")), "{missing:?}");
    let (_, missing) = build("# nothing here\n");
    assert!(missing.len() >= 10, "{missing:?}");
}

/// The sheet is what the compose prompt teaches: the programs it shows must be ones the checker takes.
#[test]
fn the_patterns_it_teaches_check() {
    let sheet = beatscript_api::cheatsheet();
    let block = sheet.split("## Patterns: mini-notation").nth(1).unwrap().split("```text\n").nth(1).unwrap().split("```").next().unwrap();
    for l in block.lines() {
        // "kick x ~ x ~  4 terms: ...": the program is what comes before the two-space gap
        let prog = l.split("  ").next().unwrap();
        // a computed degree (`$i`) is only meaningful inside a repeat or a def, as the sheet says
        let text = if prog.contains('$') { format!("repeat 1 {{\n  deck A: {prog}\n}}\n") } else { format!("deck A: {prog}\n") };
        let r = support::parse(&beatscript_api::check_json(&text));
        assert!(support::b(&r, "ok") || prog.starts_with("melody") || prog.starts_with("harmony") || prog.starts_with("bass"), "{text}: {}", r.js_stringified());
        assert!(support::b(&r, "ok"), "{text}: {}", r.js_stringified());
    }
}

#[test]
fn it_documents_the_wheel_and_says_the_browser_does_not_play_it() {
    let sheet = beatscript_api::cheatsheet();
    for l in ["wheel load preset=", "wheel key root=", "wheel sequence notes=", "wheel motor on=", "wheel level db=", "wheel play", "wheel pause", "not played here: wheel"] {
        assert!(sheet.contains(l), "the cheat sheet has no `{l}`");
    }
}

#[test]
fn its_closing_note_names_the_site_and_not_the_old_restrictions() {
    let sheet = beatscript_api::cheatsheet();
    let tail = sheet.split("## On beatscript.app").nth(1).expect("the closing note");
    assert!(!sheet.contains("btw.bitwrap.io"));
    // loop and move check on the site (hand-dubstep-140-v4 uses `move loop_roll`): the note must not say otherwise
    assert!(!tail.contains("are not available"));
    // an import (and a borrowed `<cid>`) needs the program: a link carries it, so it checks here (it is no longer "except `import`")
    assert!(tail.contains("`import` and `<cid>` need the program (a link carries it)") && !tail.contains("except `import`"));
}

#[test]
fn the_arrange_section_links_the_eight_line_example() {
    // `arrange-in-eight-lines` is one of the first examples a newcomer sees: the sheet's arrange section and BEATSCRIPT.md's
    // chapter each point to it (and the sheet does not grow for it: the cap test above holds)
    let url = "https://beatscript.app/examples/arrange-in-eight-lines.beatscript";
    let sheet = beatscript_api::cheatsheet();
    let arrange = sheet.split("### Arrange mutations").nth(1).expect("the arrange section").split("\n### ").next().unwrap();
    assert!(arrange.contains(url), "{arrange}");
    let chapter = BEATSCRIPT_MD.split("### Arrange mutations").nth(1).unwrap().split("\n### ").next().unwrap();
    assert!(chapter.contains(url));
}
