//! Features that meet, fourth file: composition (F10, `stack` / `then` / `silence` over sections and parts) against a key per
//! section (F5), chance and polymeter (F7, F6), arithmetic and guards (F2), ports (F11), defs, repeats and imports. The tests
//! the integrator writes when two branches that were built apart land on one. (`ext_cross.rs` to `ext_cross3.rs` are the
//! first three files; each feature's own tests are in `ext_<feature>.rs`.)

mod common;

use common::{reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check_with, Compiled, Timing};
use djlive::env::{Env, StaticSnippets};
use djlive::{check, diag};

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// (line, column, message, help) of every error.
fn errs(src: &str) -> Vec<(usize, usize, String, String)> {
    let c = check_with(src, &Env::none());
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone().unwrap_or_default())).collect()
}

fn said(e: &[(usize, usize, String, String)], what: &str) -> bool {
    e.iter().any(|(_, _, m, h)| m.contains(what) || h.contains(what))
}

fn warnings(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect()
}

/// (step, note) of cycle `n` of the part `role` of deck `deck` in segment `seg`.
fn played(c: &Compiled, seg: usize, deck: Deck, role: &str, n: usize) -> Vec<(u8, u8)> {
    let p = c.segments[seg].decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part in segment {seg}"));
    p.cycles[n % p.cycles.len()].iter().map(|h| (h.step, h.note)).collect()
}

/// (step, note) of cycle `n` of the part `role` of deck `deck` of a program with no timeline.
fn flat(c: &Compiled, deck: Deck, role: &str, n: usize) -> Vec<(u8, u8)> {
    let p = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role} part"));
    p.cycles[n % p.cycles.len()].iter().map(|h| (h.step, h.note)).collect()
}

/// (name, start, bars, key) of every segment.
fn layout(c: &Compiled) -> Vec<(String, u32, u32, String)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, c.key_in(s).text())).collect()
}

fn stable(src: &str, seed: u64) {
    let c = ok(src);
    let again = ok(&c.text);
    assert_eq!(again.text, c.text, "the canonical text is not a fixed point");
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(seed);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        let r = check(&text);
        let got = r.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", diag::render_all(&r.diagnostics, &text))).cid;
        assert_eq!(got, c.cid, "round {round} forked:\n{text}");
    }
}

// ---- F10 x F5: a key per stretch

const HEAD: &str = "tempo 120\nkey dm\n";

/// Sections that can be stacked in pairs without clashing on a deck and role: what each plays is in the key it is in.
const SECS: &str = "\
section intro 4 bars {
  deck A: kick x ~ x ~
  deck B: melody deg(1 3 5 3)
}
section groove 8 bars {
  deck A: hihat x*8
  deck B: bass deg(1 5 1 5)
}
section verse 8 bars {
  key am
  deck A: snare ~ x ~ x
  deck B: harmony i iv
}
section bridge 4 bars {
  key fm
  deck A: clap ~ x ~ ~
}
section coda 4 bars {
  key a:minor
  deck A: kick x ~ ~ ~
}
";

fn song(arrange: &str) -> String {
    format!("{HEAD}{SECS}{arrange}\n")
}

/// The first cycle of the melody (or bass) that `deg(1 3 5 3)` makes in `key`, as a one-section program writes it.
fn deg_in(key: &str, role: &str, pattern: &str) -> Vec<(u8, u8)> {
    let c = ok(&format!("tempo 120\nkey {key}\ndeck B: {role} deg({pattern})\n"));
    let p = c.decks[&Deck::B].parts.iter().find(|p| p.role == role).unwrap();
    p.cycles[0].iter().map(|h| (h.step, h.note)).collect()
}

#[test]
fn a_stack_sounds_in_the_key_of_the_first_section_that_sets_one() {
    // intro sets no key and plays stacked with verse (am), then alone (the program's dm)
    let c = ok(&song("arrange stack(intro, verse) intro"));
    assert_eq!(layout(&c), vec![("intro+verse".into(), 0, 4, "am".into()), ("verse".into(), 4, 4, "am".into()), ("intro".into(), 8, 4, "dm".into())]);
    // the unkeyed section's degrees are in the key of the stretch it is in
    let stacked = played(&c, 0, Deck::B, "melody", 0);
    assert_eq!(stacked, deg_in("am", "melody", "1 3 5 3"));
    let alone = played(&c, 2, Deck::B, "melody", 0);
    assert_eq!(alone, deg_in("dm", "melody", "1 3 5 3"));
    assert_ne!(stacked, alone, "the key moved the notes");
    // the net of a stretch says the key it is in, where the deck plays pitches
    assert_eq!(c.segments[0].decks[&Deck::B].key.map(|k| k.text()), Some("am".to_string()));
    // and the keyed section's own numerals did not move
    let verse_alone = ok(&song("arrange verse"));
    assert_eq!(played(&c, 0, Deck::B, "harmony", 0), played(&verse_alone, 0, Deck::B, "harmony", 0));
}

#[test]
fn two_stacked_sections_in_different_keys_are_an_error_that_names_both() {
    // verse is in am and bridge in fm: two keys cannot sound at once. The error is at the second `key`, names both sections
    // and both keys (and the line of the first), and is the same whichever way the stack is written
    let a = errs(&song("arrange stack(verse, bridge)"));
    let b = errs(&song("arrange stack(bridge, verse)"));
    assert_eq!(a, b, "stack is symmetric: so is its error");
    let key_errors: Vec<_> = a.iter().filter(|e| e.2.contains("different keys")).collect();
    assert_eq!(key_errors.len(), 1, "{a:#?}");
    let (line, _, msg, help) = key_errors[0];
    assert!(msg.contains("`bridge`") && msg.contains("`verse`") && msg.contains("key fm") && msg.contains("key am"), "{msg}");
    assert!(msg.contains("line"), "it says where the first key is: {msg}");
    assert!(help.contains("then("), "{help}");
    // it is at a `key` of the section text, not at the arrange
    let src = song("arrange stack(verse, bridge)");
    assert!(src.lines().nth(line - 1).unwrap().trim_start().starts_with("key "), "line {line}: {}", src.lines().nth(line - 1).unwrap());
    // once however often the two meet
    let twice = errs(&song("arrange stack(verse, bridge) stack(bridge, verse) stack(verse, bridge)"));
    assert_eq!(twice.iter().filter(|e| e.2.contains("different keys")).count(), 1, "{twice:#?}");
    // one after the other they are fine, and each is in its own key
    let c = ok(&song("arrange then(verse, bridge)"));
    assert_eq!(layout(&c).iter().map(|l| l.3.as_str()).collect::<Vec<_>>(), ["am", "fm"]);
}

#[test]
fn stacked_sections_that_set_the_same_key_agree_however_it_is_spelled() {
    // verse says `key am`, coda says `key a:minor`: the same key
    let c = ok(&song("arrange stack(verse, coda)"));
    assert_eq!(layout(&c)[0].3, "am");
    assert_eq!(layout(&c)[1], ("verse".into(), 4, 4, "am".into()));
}

#[test]
fn a_then_changes_the_key_between_sections_and_what_is_stacked_over_it_follows() {
    // groove sets no key: under verse it is in am, under bridge in fm (the stretch's key), and intro (nothing either) after it
    let c = ok(&song("arrange stack(then(verse, bridge), then(groove, intro))"));
    assert_eq!(layout(&c), vec![("groove+verse".into(), 0, 8, "am".into()), ("intro+bridge".into(), 8, 4, "fm".into())]);
    assert_eq!(played(&c, 0, Deck::B, "bass", 0), deg_in("am", "bass", "1 5 1 5"));
    assert_eq!(played(&c, 1, Deck::B, "melody", 0), deg_in("fm", "melody", "1 3 5 3"));
}

#[test]
fn a_longer_section_with_no_key_follows_the_stretch_so_the_key_ends_when_the_keyed_one_does() {
    // the decision, written down: a stretch has one key, the keyed sections' in it, else the program's; groove (8 bars,
    // no key) under coda (4 bars, am) is in am for four bars and in dm for the four after
    let c = ok(&song("arrange stack(groove, coda)"));
    assert_eq!(layout(&c), vec![("coda+groove".into(), 0, 4, "am".into()), ("groove".into(), 4, 4, "dm".into())]);
    assert_eq!(played(&c, 0, Deck::B, "bass", 0), deg_in("am", "bass", "1 5 1 5"));
    assert_eq!(played(&c, 1, Deck::B, "bass", 0), deg_in("dm", "bass", "1 5 1 5"));
}

#[test]
fn the_key_of_a_section_and_the_sections_at_a_bar_are_asked_in_one_vocabulary() {
    // `sections_at(bar)` names sections, `section_key(name)` answers for a section, a segment (`a+b`) and a stack's
    // member that never plays alone: every name one gives, the other can use
    let c = ok(&song("arrange stack(intro, verse) then(groove, stack(bridge, groove))"));
    let bars = c.song_bars().unwrap();
    assert_eq!(bars, 24, "intro+verse 8 bars, then groove 8, then bridge with groove 8");
    for bar in 0..bars as i64 {
        let names = c.sections_at(bar);
        assert!(!names.is_empty(), "bar {bar}");
        let seg = c.segments.iter().rev().find(|s| i64::from(s.start) <= bar).unwrap();
        assert_eq!(names, seg.name.split('+').collect::<Vec<_>>());
        for n in names {
            assert!(c.section_key(n).is_some(), "bar {bar}: no key for `{n}`");
        }
        assert!(c.section_key(&seg.name).is_some());
    }
    // a keyed section is in its own key wherever it plays; one with none has the key of the first stretch it plays in
    assert_eq!(c.section_key("verse").unwrap().text(), "am");
    assert_eq!(c.section_key("bridge").unwrap().text(), "fm");
    assert_eq!(c.section_key("intro").unwrap().text(), "am", "intro plays first with verse");
    assert_eq!(c.section_key("groove").unwrap().text(), "dm", "groove first plays alone");
    assert_eq!(c.section_key("intro+verse").unwrap().text(), "am");
    assert_eq!(c.section_key("nowhere"), None);
    // a segment is in its own key: the second stretch of the chain is verse alone, in am
    assert_eq!(c.key_in(&c.segments[1]).text(), "am");
    assert_eq!(c.key_in(c.segments.last().unwrap()).text(), "dm");
}

#[test]
fn a_key_said_twice_warns_once_and_a_bad_key_is_one_error_however_the_section_is_stacked() {
    let twice = "section intro 4 bars {\n  key am\n  key fm\n  deck A: kick x\n}\nsection verse 4 bars {\n  deck B: bass c2\n}\n";
    let src = format!("{HEAD}{twice}arrange stack(intro, verse) stack(intro, verse)\n");
    let w: Vec<_> = warnings(&src).into_iter().filter(|m| m.contains("set twice")).collect();
    assert_eq!(w.len(), 1, "{w:?}");
    let c = ok(&src);
    assert!(layout(&c).iter().all(|l| l.3 == "fm"), "{:?}", layout(&c));
    let bad = "section intro 4 bars {\n  key zzz\n  deck A: kick x\n}\nsection verse 4 bars {\n  deck B: bass c2\n}\n";
    let e = errs(&format!("{HEAD}{bad}arrange stack(intro, verse) stack(intro, verse)\n"));
    assert_eq!(e.iter().filter(|e| e.2.contains("is not a key")).count(), 1, "{e:#?}");
    assert!(!e.iter().any(|e| e.2.contains("different keys")), "a key that is not a key does not clash: {e:#?}");
}

// ---- F10 x F7 / F6: a later stretch goes on from where the section was

/// The hits of the part `role` of deck A, bar by bar, in segment `seg`.
fn bars(c: &Compiled, seg: usize, role: &str) -> Vec<Vec<(u8, u8)>> {
    let p = c.segments[seg].decks[&Deck::A].parts.iter().find(|p| p.role == role).unwrap();
    p.cycles.iter().map(|b| b.iter().map(|h| (h.step, h.note)).collect()).collect()
}

#[test]
fn a_later_stretch_of_a_stack_goes_on_from_where_chance_alternation_polymeter_and_choice_were() {
    // a 12-bar section stacked with two 4-bar ones one after the other: three stretches, the second and third begin on bar 4
    // and bar 8 of it. What each stretch plays on its bar j is what the section played alone on bar (phase + j)
    for line in [
        "deck A: hihat x*8",
        "deck A: hihat <x*4 x*8 x ~ x>",
        "deck A: hihat x*8?0.5",
        "deck A: hihat/12 x ~ x ~ x ~",
        "deck A: hihat/12 x*3?0.5 x ~ x?0.5",
        "deck A: hihat choose [x*8 x*4 [x ~]] weights 3 1 1 seed=3",
        "deck A: hihat maybe([x ~ x ~], 0.5, 4)",
        "deck A: hihat <x?0.5 x*2?0.5 x>",
    ] {
        let alone = ok(&format!("tempo 120\nsection long 12 bars {{\n  {line}\n}}\narrange long\n"));
        let stacked = ok(&format!("tempo 120\nsection long 12 bars {{\n  {line}\n}}\nsection one 4 bars {{\n  deck B: bass c2 ~ e2 ~\n}}\nsection two 4 bars {{\n  deck B: melody c4 ~ e4 ~\n}}\narrange stack(long, then(one, two))\n"));
        assert_eq!(stacked.segments.len(), 3, "{line}");
        let a = bars(&alone, 0, "hihat");
        for (k, phase) in [(0usize, 0usize), (1, 4), (2, 8)] {
            let s = bars(&stacked, k, "hihat");
            for j in 0..4 {
                assert_eq!(s[j % s.len()], a[(phase + j) % a.len()], "{line}: stretch {k} bar {j} is not bar {} of the section alone", phase + j);
            }
        }
    }
}

#[test]
fn a_chance_part_stacked_with_a_shorter_section_still_differs_every_bar_of_the_longer() {
    // (what the fix is for: the later stretch's chance used to be unrolled over its own four bars, so it repeated the first four)
    let c = ok("tempo 120\nsection long 8 bars {\n  deck A: hihat x*8?0.5\n}\nsection short 4 bars {\n  deck B: bass c2 ~ e2 ~\n}\narrange stack(long, short)\n");
    let first = bars(&c, 0, "hihat");
    let second = bars(&c, 1, "hihat");
    let (a, b): (Vec<_>, Vec<_>) = (first.iter().take(4).collect(), second.iter().take(4).collect());
    assert_ne!(a, b, "bars 4 to 7 are not bars 0 to 3 again");
}

#[test]
fn then_over_parts_shares_one_cycle_as_it_shares_its_parameters() {
    let c = ok(&format!("{HEAD}deck A: then(hihat/12 x ~ x, hihat/12 x x x)\n"));
    assert_eq!(c.text, format!("{HEAD}deck A: hihat/12 <[x ~ x] [x x x]>\n"));
    let e = errs(&format!("{HEAD}deck A: then(hihat/12 x ~ x, hihat x x x)\n"));
    assert!(said(&e, "share their cycle"), "{e:#?}");
    assert!(said(&e, "hihat/12"), "{e:#?}");
    // and a stack of parts keeps each part's own cycle
    let s = ok(&format!("{HEAD}deck A: stack(kick x ~ x ~, hihat/12 x ~ x)\n"));
    assert_eq!(s.text, format!("{HEAD}deck A: hihat/12 x ~ x | kick x ~ x ~\n"));
}

// ---- F10 x F2 / F11, defs, repeats, imports

#[test]
fn stack_and_then_over_parts_in_a_def_called_from_a_repeat_take_arithmetic_and_guards() {
    let src = "\
tempo 120
key am
def lay(n) {
  deck A: stack(kick x*$n, then(snare ~ x ~ x vel=(60 + $n * 2)..100, snare x ~ x ~ vel=(60 + $n * 2)..100))
}
repeat 3 { lay($i + 1) when $i == 1 }
";
    let c = ok(src);
    let by_hand = ok("tempo 120\nkey am\ndeck A: kick x*2 | snare <[~ x ~ x] [x ~ x ~]> vel=(60 + 2 * 2)..100\n");
    for role in ["kick", "snare"] {
        assert_eq!(flat(&c, Deck::A, role, 0), flat(&by_hand, Deck::A, role, 0), "{role}");
    }
    let snare = |c: &Compiled| c.decks[&Deck::A].parts.iter().find(|p| p.role == "snare").unwrap().clone();
    assert_eq!(snare(&c).cycles.len(), 2);
    assert_eq!(snare(&c).cycles, snare(&by_hand).cycles);
    assert_eq!(snare(&c).vel_range, Some((64, 100)));
    stable(src, 0xf10_01);
}

#[test]
fn a_wired_port_and_arithmetic_go_into_the_operands_of_a_stack_over_parts() {
    let src = "\
tempo 120
def pulse() out: rhythm {
  emit e(3, 8)
}
def lay(i) in: rhythm {
  deck A: stack(kick $rhythm vel=(60 + $i * 4)..100, snare ~ x ~ x)
}
wire pulse.out -> lay.rhythm
section intro 4 bars {
  lay(2)
}
section verse 8 bars {
  deck B: bass c2 ~ e2 ~
}
arrange stack(intro, verse)
";
    let c = ok(src);
    let by_hand = ok("tempo 120\ndeck A: kick e(3, 8) vel=(60 + 2 * 4)..100 | snare ~ x ~ x\n");
    for role in ["kick", "snare"] {
        assert_eq!(played(&c, 0, Deck::A, role, 0), flat(&by_hand, Deck::A, role, 0), "{role}");
    }
    // the bass of the longer section goes on over the four bars the shorter does not fill, and deck A stops
    assert_eq!(c.segments.iter().map(|s| (s.name.as_str(), s.decks.contains_key(&Deck::A))).collect::<Vec<_>>(), vec![("intro+verse", true), ("verse", false)]);
    stable(src, 0xf10_02);
}

#[test]
fn guards_arithmetic_and_controls_in_stacked_sections_fire_once_and_count_from_their_own_bar() {
    let src = "\
tempo 120
key dm
def swell(i) {
  at bar $i + 1: master: filter lp 400 + $i * 100 when $i % 2 == 1
}
section intro 4 bars {
  key am
  repeat 4 { swell($i) }
  deck A: kick x ~ x ~
}
section groove 8 bars {
  at bar 6: master: reverb 1 / 4 + 1 / 8
  repeat 2 { deck B: bass deg(1 5 1 5) when $i == 1 }
}
arrange stack(intro, groove) groove
";
    let c = ok(src);
    // the two filters of intro (bars 2 and 4 of it, `$i` of 1 and 3) and groove's reverb, each once, groove's twice over
    // (it plays in two items) at its own bar 6 of each playing
    let at: Vec<(String, Timing)> = c.controls.iter().map(|k| (k.key.clone(), k.timing.clone())).collect();
    assert_eq!(at.iter().filter(|(k, _)| k.contains("filter")).map(|(_, t)| t.clone()).collect::<Vec<_>>(), vec![Timing::At(2), Timing::At(4)], "{at:?}");
    assert_eq!(at.iter().filter(|(k, _)| k.contains("reverb")).map(|(_, t)| t.clone()).collect::<Vec<_>>(), vec![Timing::At(6), Timing::At(14)], "{at:?}");
    // the bass (a guard on `$i`) is there in all three stretches, in the key of the stretch it is in
    // (groove sets no key: it is in intro's am under it, and in the program's dm once intro has ended)
    assert_eq!(layout(&c).iter().map(|l| (l.0.as_str(), l.3.as_str())).collect::<Vec<_>>(), [("groove+intro", "am"), ("groove", "dm"), ("groove", "dm")]);
    assert_eq!(played(&c, 0, Deck::B, "bass", 0), deg_in("am", "bass", "1 5 1 5"));
    assert_eq!(played(&c, 2, Deck::B, "bass", 0), deg_in("dm", "bass", "1 5 1 5"));
    stable(src, 0xf10_03);
}

#[test]
fn a_section_name_is_text_so_a_macros_name_in_its_place_says_why_it_cannot_be() {
    let secs = "tempo 120\nsection intro 4 bars {\n  deck A: kick x\n}\nsection verse 4 bars {\n  deck B: bass c2\n}\n";
    for tail in ["arrange stack(intro, $i)\n", "arrange $i intro\n", "let both = stack(intro, $n)\narrange both\n", "arrange then(intro, stack($n))\n"] {
        let e = errs(&format!("{secs}{tail}"));
        assert!(said(&e, "expected a section") || said(&e, "needs the sections"), "{tail}: {e:#?}");
        assert!(said(&e, "no `$name`"), "{tail}: {e:#?}");
    }
    // and inside a def or a repeat the statements are refused where they stand
    let e = errs(&format!("{secs}def song(n) {{\n  arrange stack(intro, verse)\n}}\nsong(1)\n"));
    assert!(said(&e, "belongs at the top of the program"), "{e:#?}");
    let e = errs(&format!("{secs}repeat 2 {{\n  let both = stack(intro, verse)\n}}\n"));
    assert!(said(&e, "belongs at the top of the program"), "{e:#?}");
}

/// A store of saved snippets (each may import the ones before it).
fn snippets(items: &[(&str, &str)]) -> StaticSnippets {
    let mut snips = StaticSnippets::default();
    for (name, text) in items {
        let env = Env { snippets: Some(&snips), ..Env::default() };
        let c = check_with(text, &env);
        let canon = c.compiled.unwrap_or_else(|| panic!("snippet {name} does not compile:\n{text}\n{}", diag::render_all(&c.diagnostics, text)));
        snips.items.push((name.to_string(), canon.cid.clone(), canon.text.clone()));
    }
    snips
}

#[test]
fn a_def_that_stacks_parts_over_lines_comes_through_an_import_and_its_marks_stay_in_the_text() {
    let snips = snippets(&[("beds", "def drumbed(n) {\n  deck A: stack(kick x*$n,\n    then(hihat x*8,\n         hihat x*4))\n}\n")]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"beds\"\ntempo 120\nkey am\nsection intro 4 bars {\n  drumbed(4)\n}\nsection verse 8 bars {\n  key am\n  deck B: bass deg(1 5 1 5)\n}\narrange stack(intro, verse)\n";
    let c = check_with(src, &env);
    assert!(c.ok(), "{}", diag::render_all(&c.diagnostics, src));
    let imported = c.compiled.unwrap();
    let by_hand = ok("tempo 120\nkey am\nsection intro 4 bars {\n  deck A: stack(kick x*4, then(hihat x*8, hihat x*4))\n}\nsection verse 8 bars {\n  key am\n  deck B: bass deg(1 5 1 5)\n}\narrange stack(intro, verse)\n");
    for role in ["kick", "hihat"] {
        assert_eq!(played(&imported, 0, Deck::A, role, 0), played(&by_hand, 0, Deck::A, role, 0), "{role}");
        assert_eq!(played(&imported, 0, Deck::A, role, 1), played(&by_hand, 0, Deck::A, role, 1), "{role}");
    }
    assert!(!played(&imported, 0, Deck::A, "hihat", 1).is_empty());
    // the marks of an imported hit point at text that is there
    for seg in &imported.segments {
        for ir in seg.decks.values() {
            for p in &ir.parts {
                for chain in &p.spans {
                    assert!(chain.iter().all(|s| s.end <= src.len() && s.start <= s.end), "{chain:?}");
                }
            }
        }
    }
}

#[test]
fn an_import_brings_no_composition_and_the_name_it_would_have_given_is_not_a_section() {
    // a snippet that has sections and a `let both = stack(...)` of them is a program of its own; importing it brings its
    // defs and lets, not its sections or a composition of them
    let snips = snippets(&[("whole", "def drumbed(n) {\n  deck A: kick x*$n\n}\nsection intro 4 bars {\n  deck A: kick x\n}\nsection verse 4 bars {\n  deck B: bass c2\n}\nlet both = stack(intro, verse)\narrange both\n")]);
    let env = Env { snippets: Some(&snips), ..Env::default() };
    let src = "import \"whole\"\ntempo 120\nsection chorus 4 bars {\n  drumbed(4)\n}\narrange both\n";
    let c = check_with(src, &env);
    assert!(!c.ok());
    let e: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(e.iter().any(|m| m.contains("there is no section `both`")), "{e:?}");
}

// ---- the laws, with everything in the sections

/// Sections with everything merged so far in their bodies: guards and arithmetic, a def called from a repeat, a wired port,
/// a cycle of its own, chance, degrees, a key, a control. Every (deck, role) is its own, so any two can be stacked, except
/// where the two keys differ.
type Pool = Vec<(&'static str, u32, Option<&'static str>)>;

fn rich() -> (String, Pool) {
    let text = "\
tempo 120
key dm
def pulse() out: rhythm {
  emit e(3, 8)
}
def lay(i) in: rhythm {
  deck A: kick $rhythm vel=(60 + $i * 4)..100
}
def swell(i) {
  at bar $i + 1: master: filter lp 400 + $i * 100 when $i % 2 == 1
}
wire pulse.out -> lay.rhythm
section intro 4 bars {
  repeat 2 { lay($i + 1) when $i == 1 }
  repeat 4 { swell($i) }
}
section groove 8 bars {
  key am
  deck A: hihat/12 x*3?0.5 x ~ x?0.5
  deck B: bass deg(1 5 (1 + 1) 5)
}
section verse 8 bars {
  key am
  deck A: then(snare ~ x ~ x, snare x ~ x ~)
  deck B: harmony i iv V voice-lead
}
section bridge 4 bars {
  key fm
  deck B: melody deg(1 3 5 3)
  at bar 2: master: reverb 1 / 4
}
section lift 5 bars {
  deck A: clap choose [x ~ [x x]] weights 2 1 1 seed=2
  repeat 3 { deck B: arp arp(am7, up, 8) when $i == 2 }
}
section coda 2 bars {
  deck B: kick x ~ ~ ~
}
";
    (text.to_string(), vec![("intro", 4, None), ("groove", 8, Some("am")), ("verse", 8, Some("am")), ("bridge", 4, Some("fm")), ("lift", 5, None), ("coda", 2, None)])
}

#[derive(Clone, Debug)]
enum T {
    Leaf(usize),
    Silence,
    Stack(Vec<T>),
    Then(Vec<T>),
}

fn gen(rng: &mut Rng, pool: &mut Vec<usize>, n: usize) -> T {
    let mut t = if n == 1 {
        T::Leaf(pool.pop().expect("a leaf left"))
    } else {
        let k = 2 + rng.below((n.min(4) - 1) as u64) as usize;
        let mut sizes = vec![1usize; k];
        for _ in 0..(n - k) {
            let j = rng.below(k as u64) as usize;
            sizes[j] += 1;
        }
        let kids: Vec<T> = sizes.iter().map(|&s| gen(rng, pool, s)).collect();
        if rng.below(2) == 0 {
            T::Stack(kids)
        } else {
            T::Then(kids)
        }
    };
    if rng.below(6) == 0 {
        t = if rng.below(2) == 0 { T::Stack(vec![t]) } else { T::Then(vec![t]) };
    }
    t
}

fn render(t: &T, names: &[&str], rng: &mut Rng, top: bool) -> String {
    let seps = [", ", ",", " , ", ",\n    "];
    let kids = |xs: &[T], rng: &mut Rng| xs.iter().map(|x| render(x, names, rng, false)).collect::<Vec<_>>();
    match t {
        T::Leaf(i) => names[*i].to_string(),
        T::Silence => "silence".into(),
        T::Stack(xs) => format!("stack({})", kids(xs, rng).join(seps[rng.below(4) as usize])),
        T::Then(xs) => {
            if top && rng.below(2) == 0 {
                kids(xs, rng).join(" ")
            } else {
                format!("then({})", kids(xs, rng).join(seps[rng.below(4) as usize]))
            }
        }
    }
}

fn reassociate(t: &T, rng: &mut Rng) -> T {
    fn regroup(xs: &[T], rng: &mut Rng, make: fn(Vec<T>) -> T) -> Vec<T> {
        let mut ys: Vec<T> = xs.iter().map(|x| reassociate(x, rng)).collect();
        if ys.len() >= 3 && rng.below(3) != 0 {
            let len = 2 + rng.below((ys.len() - 2) as u64) as usize;
            let start = rng.below((ys.len() - len + 1) as u64) as usize;
            let group: Vec<T> = ys.drain(start..start + len).collect();
            ys.insert(start, make(group));
        }
        ys
    }
    match t {
        T::Then(xs) => T::Then(regroup(xs, rng, T::Then)),
        T::Stack(xs) => T::Stack(regroup(xs, rng, T::Stack)),
        other => other.clone(),
    }
}

fn units(t: &T, rng: &mut Rng) -> T {
    let mut t2 = match t {
        T::Stack(xs) => T::Stack(xs.iter().map(|x| units(x, rng)).collect()),
        T::Then(xs) => T::Then(xs.iter().map(|x| units(x, rng)).collect()),
        other => other.clone(),
    };
    if let T::Stack(xs) | T::Then(xs) = &mut t2 {
        for _ in 0..rng.below(3) {
            let at = rng.below(xs.len() as u64 + 1) as usize;
            xs.insert(at, T::Silence);
        }
    }
    match rng.below(7) {
        0 => T::Then(vec![t2, T::Silence]),
        1 => T::Then(vec![T::Silence, t2]),
        2 => T::Stack(vec![T::Silence, t2]),
        3 => T::Stack(vec![t2, T::Silence]),
        4 => T::Then(vec![t2]),
        5 => T::Stack(vec![t2]),
        _ => t2,
    }
}

fn permute(t: &T, rng: &mut Rng) -> T {
    match t {
        T::Stack(xs) => {
            let mut ys: Vec<T> = xs.iter().map(|x| permute(x, rng)).collect();
            for i in (1..ys.len()).rev() {
                ys.swap(i, rng.below(i as u64 + 1) as usize);
            }
            T::Stack(ys)
        }
        T::Then(xs) => T::Then(xs.iter().map(|x| permute(x, rng)).collect()),
        other => other.clone(),
    }
}

/// Where each leaf plays and how long the tree is.
fn place(t: &T, pool: &[(&'static str, u32, Option<&'static str>)], at: u32, out: &mut Vec<(usize, u32)>) -> u32 {
    match t {
        T::Leaf(i) => {
            out.push((*i, at));
            pool[*i].1
        }
        T::Silence => 0,
        T::Then(xs) => {
            let mut acc = 0;
            for x in xs {
                acc += place(x, pool, at + acc, out);
            }
            acc
        }
        T::Stack(xs) => xs.iter().map(|x| place(x, pool, at, out)).max().unwrap_or(0),
    }
}

/// The key every stretch should be in, by the rule written in the docs: the keyed sections playing in it agree (that key), or two
/// of them differ (None: an error), and with none, the program's `dm`. (bar of the stretch, the sections playing, the key)
fn expected_keys(t: &T, pool: &[(&'static str, u32, Option<&'static str>)]) -> Option<Vec<(u32, Vec<&'static str>, &'static str)>> {
    let mut placed = Vec::new();
    place(t, pool, 0, &mut placed);
    let mut cuts: Vec<u32> = placed.iter().flat_map(|&(i, at)| [at, at + pool[i].1]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let mut playing: Vec<&'static str> = Vec::new();
        let mut key: Option<&'static str> = None;
        for &(i, at) in &placed {
            if at <= w[0] && w[0] < at + pool[i].1 {
                playing.push(pool[i].0);
                if let Some(k) = pool[i].2 {
                    match key {
                        Some(other) if other != k => return None,
                        _ => key = Some(k),
                    }
                }
            }
        }
        playing.sort_unstable();
        out.push((w[0], playing, key.unwrap_or("dm")));
    }
    Some(out)
}

/// A message without the number of the line it points at (reformatting moves lines).
fn no_line(m: &str) -> String {
    match m.find("(line ") {
        Some(i) => format!("{}(line N){}", &m[..i], &m[m[i..].find(')').map_or(m.len(), |j| i + j + 1)..]),
        None => m.to_string(),
    }
}

#[test]
fn property_the_laws_hold_with_guards_arithmetic_ports_chance_cycles_and_keys_in_the_sections() {
    let (defs, pool) = rich();
    let names: Vec<&str> = pool.iter().map(|p| p.0).collect();
    let mut rng = Rng(0xf10_5eed_u64);
    let (mut oks, mut clashes) = (0, 0);
    for round in 0..120 {
        let mut idx: Vec<usize> = (0..pool.len()).collect();
        for i in (1..idx.len()).rev() {
            idx.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let n = 1 + rng.below(5) as usize;
        let t = gen(&mut rng, &mut idx, n);
        let program = |t: &T, rng: &mut Rng| format!("{defs}arrange {}\n", render(t, &names, rng, true));
        let base = program(&t, &mut rng);
        let base_result = check(&base);
        let want = expected_keys(&t, &pool);
        let variants = [
            ("reassociated", reassociate(&reassociate(&t, &mut rng), &mut rng)),
            ("with units", units(&t, &mut rng)),
            ("permuted", permute(&t, &mut rng)),
            ("all three", permute(&units(&reassociate(&t, &mut rng), &mut rng), &mut rng)),
        ];
        match want {
            None => {
                // two keys at once: an error that names both, the same error for every spelling
                clashes += 1;
                assert!(!base_result.ok(), "round {round}: two keys at once compiled:\n{base}");
                let msgs: Vec<String> = base_result.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
                assert!(msgs.iter().any(|m| m.contains("different keys")), "round {round}: {msgs:?}\n{base}");
                for (what, v) in variants {
                    let text = reformat(&program(&v, &mut rng), &mut rng);
                    let r = check(&text);
                    let got: Vec<String> = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| no_line(&d.message)).collect();
                    let want: Vec<String> = msgs.iter().map(|m| no_line(m)).collect();
                    assert_eq!(got, want, "round {round}, {what}:\n{base}\n--- became\n{text}");
                }
            }
            Some(stretches) => {
                oks += 1;
                assert!(base_result.ok(), "round {round}:\n{base}\n{}", diag::render_all(&base_result.diagnostics, &base));
                let c = base_result.compiled.unwrap();
                // the key of every stretch is the rule's, and every name `sections_at` gives has a key
                if !stretches.is_empty() {
                    for (at, playing, key) in &stretches {
                        let mut got: Vec<&str> = c.sections_at(i64::from(*at));
                        got.sort_unstable();
                        assert_eq!(&got, playing, "round {round} bar {at}:\n{base}");
                        let seg = c.segments.iter().rev().find(|s| s.start <= *at).unwrap();
                        assert_eq!(c.key_in(seg).text(), *key, "round {round} bar {at}:\n{base}");
                        for n in got {
                            assert!(c.section_key(n).is_some(), "round {round}: no key for `{n}`");
                        }
                    }
                }
                for (what, v) in variants {
                    let text = reformat(&program(&v, &mut rng), &mut rng);
                    let r = check(&text);
                    assert!(r.ok(), "round {round}, {what}:\n{text}\n{}", diag::render_all(&r.diagnostics, &text));
                    let got = r.compiled.unwrap();
                    assert_eq!(got.text, c.text, "round {round}, {what}:\n{base}\n--- became\n{text}");
                    assert_eq!(got.cid, c.cid, "round {round}, {what}");
                    assert_eq!(layout(&got), layout(&c), "round {round}, {what}");
                }
            }
        }
    }
    assert!(oks > 40 && clashes > 5, "the pool should exercise both: {oks} compiled, {clashes} clashing");
}

#[test]
fn a_def_body_is_kept_as_written_so_a_stack_inside_one_is_not_reordered_but_means_the_same() {
    // the laws normalise what is read at the top level; a def's body is a macro and prints as written (like every def body), so
    // two orders of a stack inside a def are two programs with two CIDs that play the same notes
    let a = ok("tempo 120\ndef lay() {\n  deck A: stack(kick x ~ x ~, snare ~ x ~ x)\n}\nlay()\n");
    let b = ok("tempo 120\ndef lay() {\n  deck A: stack(snare ~ x ~ x, kick x ~ x ~)\n}\nlay()\n");
    assert_ne!(a.cid, b.cid);
    for role in ["kick", "snare"] {
        assert_eq!(flat(&a, Deck::A, role, 0), flat(&b, Deck::A, role, 0));
    }
    // but a stack of parts written where a line is (not in a def) is
    let c = ok("tempo 120\ndeck A: stack(kick x ~ x ~, snare ~ x ~ x)\n");
    let d = ok("tempo 120\ndeck A: stack(snare ~ x ~ x, kick x ~ x ~)\n");
    assert_eq!(c.cid, d.cid);
}
