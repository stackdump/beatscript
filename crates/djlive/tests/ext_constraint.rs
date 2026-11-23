//! F12 constraints: `assert never X beside Y`, `assert never X during Y`, `assert X ends on D` (docs/beatscript/ext-assert.md,
//! "Constraints"). They are decided by `check` over the whole finite timeline by exact enumeration: here each form passing and
//! failing with its message (line, column, bar and step), the seams (a bar line, a section, a `loop`), the vocal's windows,
//! chance (judged on what a part may play), `ends on` in each key, every misuse in the language's terms, the words that stay
//! names, and the laws (canonical fixed point, reformatting keeps the CID, the same verdict on every host).

mod common;

use common::{reformat, Rng};
use djlive::assertion::Status;
use djlive::compile::{check, check_report, check_with, Compiled};
use djlive::diag::{line_col, render_all, Diagnostic, Severity};
use djlive::env::{Env, StaticVoices, Voices};

// ---- helpers

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", render_all(&c.diagnostics, src)))
}

fn report(src: &str) -> Compiled {
    let c = check_report(src, &Env::none());
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{src}\n{}", render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "expected an error in {src:?}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn one_error(src: &str) -> Diagnostic {
    let mut e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    e.remove(0)
}

fn warnings(src: &str) -> Vec<String> {
    ok(src).warnings.iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message.clone()).collect()
}

/// (line, column) of an error.
fn at(src: &str, d: &Diagnostic) -> (usize, usize) {
    line_col(src, d.span.start)
}

/// The canonical text of one `assert` statement (read, not checked: what it names need not be there).
fn canon(assert: &str) -> String {
    let p = djlive::parse::parse(&format!("{assert}\n"));
    assert!(p.ok(), "`{assert}` does not read:\n{}", render_all(&p.diagnostics, assert));
    djlive::print::canonical(&p.program).trim_end().to_string()
}

/// One bar of sixteen steps with a hit on each of `steps`: `bar(&[4, 12])` is `~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~`.
fn bar(steps: &[usize]) -> String {
    (0..16).map(|s| if steps.contains(&s) { "x" } else { "~" }).collect::<Vec<_>>().join(" ")
}

/// What the program's assertions came to: status and the statement with its message.
fn verdicts(src: &str) -> Vec<(Status, String)> {
    report(src).assertions.iter().map(|a| (a.status, a.line())).collect()
}

/// The message of the one failing assertion, and its help.
fn failure(src: &str) -> (String, String) {
    let r = report(src);
    let f: Vec<_> = r.assertions.iter().filter(|a| a.status == Status::Fail).collect();
    assert_eq!(f.len(), 1, "{:?}", r.assertions.iter().map(|a| a.line()).collect::<Vec<_>>());
    (f[0].line(), f[0].help.clone().unwrap_or_default())
}

fn holds(src: &str) {
    let r = report(src);
    assert!(r.assertions.iter().all(|a| a.status == Status::Pass), "{:?}", r.assertions.iter().map(|a| a.line()).collect::<Vec<_>>());
    assert!(!r.assertions.is_empty());
    ok(src);
}

fn broken(src: &str) -> Diagnostic {
    let e = one_error(src);
    assert!(e.message.starts_with("assert "), "{}", e.message);
    e
}

/// A deck-A drum program of one section of `bars` bars, with the statements after it.
fn drums(bars: u32, lines: &str, asserts: &str) -> String {
    format!("section verse {bars} bars {{\n{lines}\n}}\narrange verse\n{asserts}\n")
}

// ---- the canonical form

#[test]
fn every_constraint_prints_in_its_canonical_form() {
    for (written, canonical) in [
        ("assert never A.snare beside A.snare", "assert never A.snare beside A.snare"),
        ("assert   never   A.snare   beside   B.kick", "assert never A.snare beside B.kick"),
        ("assert never snare beside kick", "assert never snare beside kick"),
        ("assert never B during sing", "assert never B during sing"),
        ("assert never B during sing in verse", "assert never B during sing in verse"),
        ("assert never A during B", "assert never A during B"),
        ("assert never bass during A.kick in drop", "assert never bass during A.kick in drop"),
        ("assert bass ends on 1", "assert bass ends on 1"),
        ("assert B.melody ends on 5 in lift", "assert B.melody ends on 5 in lift"),
        ("assert  harmony  ends  on  3", "assert harmony ends on 3"),
        ("assert bass ends on 1 + 1", "assert bass ends on 2"),
    ] {
        assert_eq!(canon(written), canonical, "{written}");
        // the canonical text is a fixed point
        assert_eq!(canon(canonical), canonical);
    }
}

#[test]
fn a_constraint_is_part_of_the_program_so_an_edit_is_another_cid() {
    let base = drums(2, "  deck A: snare x ~ ~ ~", "");
    let one = ok(&format!("{base}assert never A.snare beside A.snare\n")).cid;
    let two = ok(&format!("{base}assert never A.snare during A.snare\n")).cid;
    let none = ok(&base).cid;
    assert!(one != two && one != none && two != none);
    // spacing and comments are not the program
    let again = ok(&format!("{base}# nothing touches\nassert   never A.snare   beside A.snare   # the rule\n")).cid;
    assert_eq!(again, one);
}

#[test]
fn reformatting_keeps_the_cid_of_a_program_with_constraints() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/constraints.beatscript")).unwrap();
    let cid = ok(&src).cid;
    let mut rng = Rng(0xc0_57a1_0000_0001);
    for round in 0..40 {
        let text = reformat(&src, &mut rng);
        let c = check(&text);
        assert_eq!(c.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{text}\n{}", render_all(&c.diagnostics, &text))).cid, cid, "round {round}:\n{text}");
    }
}

// ---- the three programs of the proposal (the regression: the old code stops at the first word, `never` or `bass`)

const BACKBEAT: &str = "# backbeat: the snare sits on 2 and 4; the lift adds a pickup
tempo 126
key am

let kicks = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
let backbeat = ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~
let hats = x*8

section groove 4 bars {
  deck A: kick kicks | snare backbeat | hihat hats vel=50..80
  deck B: bass a1 ~ ~ a1 ~ ~ e1 ~ oct=1
}

section lift 4 bars {
  deck A: kick kicks | snare backbeat | hihat hats vel=60..90
  deck B: bass a1 ~ ~ a1 ~ ~ e1 ~ oct=1
}

arrange {
  groove
  ~ backbeat fill FILL
  lift
}

assert never A.snare beside A.snare
";

#[test]
fn no_two_snares_touch_after_an_arrange_mutation() {
    // (a mutation is chance rolled by the text, so the pattern the lift reads moves with the program's CID: these two numbers
    // are the ones that roll one pickup and then a pair, in this program)
    let good = BACKBEAT.replace("FILL", "0.17");
    holds(&good);
    let bad = BACKBEAT.replace("FILL", "0.19");
    let e = broken(&bad);
    assert_eq!(at(&bad, &e), (25, 1));
    assert_eq!(e.message, "assert never A.snare beside A.snare: two `snare` hits touch on deck A, in `lift`, bar 4, steps 4 and 5");
    let help = e.help.unwrap();
    assert!(help.starts_with("Bar 4 of deck A's `snare` in `lift` is `~ ~ ~ ~ x x ~ ~ ~ ~ ~ ~ x ~ ~ ~` (line 15)."), "{help}");
    assert!(help.contains("It comes up again in bars 5, 6, 7."), "{help}");
    assert!(help.ends_with("To fix it: move or drop one of the two hits."), "{help}");
}

const ANSWER: &str = "# answer: the singer owns the first bar and a half; deck B answers in the gap
tempo 96
key am

deck A: kick x ~ x ~ | hihat x*8 vel=50..80 | melody <[a4 c5 e5 c5] ~> vel=80
deck B: bass <~ [BASS]> inst=sub-bass

section verse 8 bars {
  deck A: sing \"hold the lan-tern\" on melody bars=2 voice=robot
}

arrange verse

assert never B during sing
";

#[test]
fn deck_b_stays_out_while_the_vocal_sings() {
    let good = ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~");
    holds(&good);
    let bad = ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ e2 ~ ~ ~ ~ ~ a1 ~ ~ ~");
    let e = broken(&bad);
    assert_eq!(at(&bad, &e), (14, 1));
    assert_eq!(e.message, "assert never B during sing: deck B plays `bass` (e2) at bar 1, step 6, while the vocal sings in `verse`");
    let help = e.help.unwrap();
    assert!(help.contains("The `sing` on line 9 holds a note from bar 0, step 12 to bar 1, step 7; deck B is clear of the vocal from bar 1, step 8."), "{help}");
    assert!(help.contains("It comes up again in bars 3, 5, 7."), "{help}");
}

const HOME: &str = "# home: every section comes back to its own tonic, even after the key moves
tempo 100
key am

deck A: kick x ~ ~ ~ x ~ ~ ~ | hihat x*8 vel=50..80

section verse 8 bars {
  deck B: harmony <i VI III VII i VI v i> voice-lead | bass <i VI III VII i VI v i> oct=2
}

section lift 8 bars {
  key cm
  deck B: harmony <i bII V7 i i bII V7 i> voice-lead | bass <i bII V7 i i bII V7 i> oct=2
}

arrange verse lift verse

assert bass ends on 1
";

#[test]
fn every_section_comes_back_to_its_own_tonic() {
    holds(HOME);
    let v = verdicts(HOME);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].1, "assert bass ends on 1: the bass ends on degree 1 in 3 places: `verse` (bar 7), `lift` (bar 15), `verse` (bar 23)");
    // end `lift` on a half close
    let half = HOME.replace("<i bII V7 i i bII V7 i>", "<i bII V7 i i bII i V7>");
    let e = broken(&half);
    assert_eq!(at(&half, &e), (18, 1));
    assert_eq!(e.message, "assert bass ends on 1: in `lift` (key cm) the bass ends on g, degree 5, not c, at bar 15, step 0");
    assert_eq!(e.help.unwrap(), "The last note comes from `V7` on line 13. End on c, or write `assert bass ends on 5 in lift` if that ending is meant.");
    // ... and says so when it is meant
    let said = half.replace("assert bass ends on 1\n", "assert bass ends on 1 in verse\nassert bass ends on 5 in lift\n");
    holds(&said);
}

// ---- beside

#[test]
fn neighbouring_steps_touch_across_a_bar_line() {
    // a hit on the last step of bar 0 and one on the first of bar 1 (a one-bar cycle would repeat its own: two bars alternate)
    let src = format!("deck A: snare <[{}] [{}]>\nassert never A.snare beside A.snare\n", bar(&[15]), bar(&[0]));
    let (m, help) = failure(&src);
    assert_eq!(m, "assert never A.snare beside A.snare: two `snare` hits touch on deck A, bar 0, step 15 and bar 1, step 0");
    assert!(help.starts_with("Bar 0 of deck A's `snare` is `~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ x` (line 1)."), "{help}");
    // a step apart is not beside, and the program holds
    let apart = format!("deck A: snare <[{}] [{}]>\nassert never A.snare beside A.snare\n", bar(&[14]), bar(&[0]));
    holds(&apart);
}

#[test]
fn two_hits_on_one_step_are_together_not_beside() {
    let src = drums(2, &format!("  deck A: snare {} | clap {}", bar(&[4]), bar(&[4])), "assert never A.snare beside A.clap\n");
    holds(&src);
    let next = drums(2, &format!("  deck A: snare {} | clap {}", bar(&[4]), bar(&[5])), "assert never A.snare beside A.clap\n");
    let (m, _) = failure(&next);
    assert_eq!(m, "assert never A.snare beside A.clap: `snare` on deck A and `clap` on deck A touch, in `verse`, bar 0, steps 4 and 5");
    // beside is symmetric: the clap first is the same rule
    let before = drums(2, &format!("  deck A: snare {} | clap {}", bar(&[5]), bar(&[4])), "assert never A.snare beside A.clap\n");
    assert_eq!(failure(&before).0, "assert never A.snare beside A.clap: `clap` on deck A and `snare` on deck A touch, in `verse`, bar 0, steps 4 and 5");
}

#[test]
fn a_seam_between_sections_is_judged() {
    let src = format!("section verse 2 bars {{\n  deck A: snare {}\n}}\nsection chorus 2 bars {{\n  deck A: snare {}\n}}\narrange verse chorus\nassert never A.snare beside A.snare\n", bar(&[15]), bar(&[0, 8]));
    let (m, _) = failure(&src);
    assert_eq!(m, "assert never A.snare beside A.snare: two `snare` hits touch on deck A, from `verse` into `chorus`, bar 1, step 15 and bar 2, step 0");
    // `in chorus` is about what plays in the chorus: the pair is across its edge
    holds(&format!("{src}assert never A.snare beside A.snare in chorus\n").replace(&format!("{src}"), &src.replace("assert never A.snare beside A.snare\n", "")));
}

#[test]
fn a_loop_turns_from_its_last_step_to_its_first() {
    let body = format!("section verse 2 bars {{\n  deck A: snare <[{}] [{}]>\n}}\narrange verse\n", bar(&[0]), bar(&[15]));
    // without the loop the song ends: the steps are two bars apart
    holds(&format!("{body}assert never A.snare beside A.snare\n"));
    // with it the last step is beside the first
    let (m, _) = failure(&format!("{body}loop\nassert never A.snare beside A.snare\n"));
    assert_eq!(m, "assert never A.snare beside A.snare: two `snare` hits touch on deck A, in `verse`, bar 1, step 15 and bar 0, step 0, where the song starts over");
    // a program with no sections and no end is round and round its own period
    let flat = format!("deck A: snare <[{}] [{}]>\nassert never A.snare beside A.snare\n", bar(&[0]), bar(&[15]));
    assert!(failure(&flat).0.ends_with("where the song starts over"));
    // with an end there is no turn: two bars, and the pair is a bar apart (a longer end would play the cycle again)
    holds(&format!("{flat}end at bar 2\n"));
    assert!(failure(&format!("{flat}end at bar 4\n")).0.ends_with("bar 1, step 15 and bar 2, step 0"));
}

#[test]
fn subjects_name_a_deck_a_part_or_a_role_on_every_deck() {
    let two = |a: &str, b: &str, rule: &str| format!("deck A: kick {} | snare {}\ndeck B: kick {}\n{}\nend at bar 2\n", bar(&[0]), bar(a_steps(a)), bar(a_steps(b)), rule);
    fn a_steps(s: &str) -> &'static [usize] {
        match s {
            "1" => &[1],
            "2" => &[2],
            _ => &[],
        }
    }
    // A.kick on 0, A.snare on 1, B.kick on 2
    let src = two("1", "2", "assert never A.kick beside A.snare");
    assert!(failure(&src).0.contains("touch"), "kick 0 beside snare 1");
    // the role on both decks: A.kick 0 and B.kick 2 are two steps apart
    holds(&two("1", "2", "assert never kick beside kick"));
    // a deck as a whole: A's snare on 1 touches B's kick on 2
    let src = two("1", "2", "assert never A beside B");
    assert!(failure(&src).0.contains("`snare` on deck A and `kick` on deck B touch"), "{}", failure(&src).0);
}

#[test]
fn in_names_the_section_the_rule_is_about() {
    let src = |scope: &str| {
        format!(
            "section verse 2 bars {{\n  deck A: snare x ~ ~ ~ | clap {}\n}}\nsection chorus 2 bars {{\n  deck A: snare x ~ ~ ~ | clap {}\n}}\narrange verse chorus\nassert never A.snare beside A.clap{scope}\n",
            bar(&[4, 12]),
            bar(&[1, 12])
        )
    };
    // the clap touches the snare only in the chorus
    assert!(failure(&src("")).0.contains("in `chorus`"));
    holds(&src(" in verse"));
    assert!(failure(&src(" in chorus")).0.contains("in `chorus`"));
    // a section that is nowhere
    let e = one_error(&src(" in chorous"));
    assert_eq!(e.message, "there is no section `chorous`");
    assert!(e.help.as_deref().unwrap().contains("did you mean `chorus`?"), "{:?}", e.help);
}

#[test]
fn a_section_an_arrange_mutation_plays_again_is_still_that_section() {
    // `~ tom mirror` makes the hit on the last step of the first half a pair (7 and 8); the section is played in that generation
    // as a copy of its own (`groove@2`), and the rule is about every place the person's `groove` plays
    let src = |scope: &str| {
        format!("let tom = ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ ~\nsection groove 2 bars {{\n  deck A: snare tom\n}}\narrange {{\n  groove\n  ~ tom mirror\n  groove\n}}\nassert never A.snare beside A.snare{scope}\n")
    };
    let (m, _) = failure(&src(""));
    assert_eq!(m, "assert never A.snare beside A.snare: two `snare` hits touch on deck A, in `groove`, bar 2, steps 7 and 8");
    assert_eq!(failure(&src(" in groove")).0, "assert never A.snare beside A.snare in groove: two `snare` hits touch on deck A, in `groove`, bar 2, steps 7 and 8");
    // and `ends on` judges each of the places, the copy too
    let ends = "let tom = ~ ~ ~ ~ ~ ~ ~ c2 ~ ~ ~ ~ ~ ~ ~ ~\nsection groove 2 bars {\n  deck B: bass tom\n}\narrange {\n  groove\n  ~ tom up 1\n  groove\n}\nassert bass ends on 1 in groove\n";
    let (m, _) = failure(ends);
    assert!(m.starts_with("assert bass ends on 1 in groove: in `groove` (key c) the bass ends on d, degree 2, not c, at bar 3, step 7"), "{m}");
}

#[test]
fn in_needs_a_section_and_a_program_with_none_says_so() {
    let e = one_error("deck A: snare x ~ ~ ~\nassert never A.snare beside A.snare in verse\n");
    assert_eq!(e.message, "there is no section `verse`: this program has no sections");
}

// ---- during

#[test]
fn a_note_sounds_for_its_length_and_is_cut_where_its_section_ends() {
    // the bass sounds for 4 steps from step 14 of the last bar of the verse: steps 14, 15 and (not the chorus's) nothing more
    let src = |kick: usize| {
        format!(
            "section verse 1 bars {{\n  deck B: bass {} len=4\n}}\nsection chorus 1 bars {{\n  deck A: kick {}\n}}\narrange verse chorus\nassert never B during A\n",
            bar(&[14]).replace('x', "c2"),
            bar(&[kick])
        )
    };
    // a kick on the first step of the chorus: the bass is cut at the section's end, so they do not meet
    holds(&src(0));
    // the same note, with the kick under its tail inside the verse
    let inside = "section verse 1 bars {\n  deck B: bass ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ c2 ~ len=4\n  deck A: kick ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ x\n}\narrange verse\nassert never B during A\n";
    let (m, _) = failure(inside);
    assert_eq!(m, "assert never B during A: deck B plays `bass` (c2) while deck A plays `kick`, at bar 0, step 15 in `verse`");
}

#[test]
fn a_part_never_sounds_during_itself_with_another_of_its_notes() {
    // a bass of two notes that overlap is not one voice: the rule is that no two of its notes sound at once
    // eight terms a bar, two steps each: c2 on step 0 and e2 on step 4
    let mono = |len: u32| format!("deck B: bass c2 ~ e2 ~ ~ ~ ~ ~ len={len}\nend at bar 2\nassert never bass during bass\n");
    holds(&mono(4));
    let (m, _) = failure(&mono(5));
    assert!(m.starts_with("assert never bass during bass: deck B plays `bass` (c2) while deck B plays `bass` (e2), at bar 0, step 4"), "{m}");
    // a note is not during itself, whatever it holds
    holds("deck B: bass c2 len=16\nend at bar 2\nassert never bass during bass\n");
    // a chord is several notes at once, which is what a chord is
    assert!(failure("deck B: harmony [c3,e3,g3]\nend at bar 2\nassert never harmony during harmony\n").0.contains("while deck B plays `harmony`"));
}

#[test]
fn what_sounds_together_is_judged_on_every_step_of_the_song() {
    // kick on A at step 8, a pad on B holding steps 4 to 11 of the second bar only
    let src = "section verse 2 bars {\n  deck A: kick ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~\n  deck B: harmony <~ [~ ~ ~ ~ c3 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]> len=8\n}\narrange verse\nassert never B during A.kick\n";
    let (m, _) = failure(src);
    assert_eq!(m, "assert never B during A.kick: deck B plays `harmony` (c3) while deck A plays `kick`, at bar 1, step 8 in `verse`");
    // in the first bar the pad rests: the rule is about the section as a whole, so `in` a section that is not it holds
    let two = "section verse 1 bars {\n  deck A: kick x ~ ~ ~\n}\nsection chorus 1 bars {\n  deck B: harmony c3 ~ ~ ~\n}\narrange verse chorus\nassert never B during A\n";
    holds(two);
}

// ---- the vocal

fn vocal(extra: &str, rule: &str) -> String {
    format!(
        "tempo 96\nkey am\ndeck A: melody <[a4 c5 e5 c5] ~>\ndeck B: bass {}\nsection verse 4 bars {{\n  deck A: sing \"hold the lan-tern\" on melody bars=2{extra}\n}}\narrange verse\n{rule}\n",
        "<~ [~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~]>"
    )
}

#[test]
fn the_vocal_sings_from_each_onset_until_the_voice_lets_go() {
    // the notes are at steps 0, 4, 8 and 12 of the bar; a note lasts to the next less a sixteenth (3 steps) and the last to the
    // end of the line's two bars (12 steps at most): steps 0-2, 4-6, 8-10, then 12 of bar 0 to step 7 of bar 1. A kick on every
    // other bar, on one step of it, is under the vocal or in the clear
    let src = |step: usize| vocal("", "assert never kick during sing").replace("deck B: bass", &format!("deck B: kick <[{}] ~> | bass", bar(&[step])));
    for step in [3, 7] {
        holds(&src(step));
    }
    for step in [0, 2, 4, 6, 8, 10, 12, 15] {
        let m = failure(&src(step)).0;
        assert!(m.contains(&format!("at bar 0, step {step}, while the vocal sings in `verse`")), "{step}: {m}");
    }
    // the same kick, a bar on: the held note is still sounding at steps 0 to 7 of bar 1 and clear from 8
    let late = |step: usize| vocal("", "assert never kick during sing").replace("deck B: bass", &format!("deck B: kick <~ [{}]> | bass", bar(&[step])));
    holds(&late(8));
    assert!(failure(&late(7)).0.contains("at bar 1, step 7, while the vocal sings"));
    assert!(failure(&late(0)).0.contains("at bar 1, step 0, while the vocal sings"));
}

#[test]
fn the_line_goes_round_its_bars_through_its_section_and_stops_with_it() {
    // `bars=2` in a section of 4: the second pass starts on bar 2, so a bass note on bar 2 step 6 is under it (as bar 0's step 6 is)
    let src = |bass: &str| vocal("", "assert never B during sing").replace("<~ [~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~]>", bass);
    holds(&src("<~ [~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~]>"));
    let (m, _) = failure(&src("<~ ~ [~ ~ ~ ~ ~ ~ e2 ~ ~ ~ ~ ~ ~ ~ ~ ~] ~>"));
    assert!(m.contains("at bar 2, step 6, while the vocal sings in `verse`"), "{m}");
    // the section is 4 bars: nothing is sung in a later one
    let later = "tempo 96\nkey am\ndeck A: melody <[a4 c5 e5 c5] ~>\nsection verse 2 bars {\n  deck A: sing \"hold the lan-tern\" on melody bars=2\n}\nsection chorus 2 bars {\n  deck B: bass c2 ~ ~ ~\n}\narrange verse chorus\nassert never B during sing\n";
    holds(later);
}

#[test]
fn a_rest_in_the_lyric_still_counts_as_singing() {
    // `_` is a syllable with no sound: the part's note is still where the voice is held (the safe side)
    let src = "tempo 96\nkey am\ndeck A: melody [a4 c5 e5 c5]\ndeck B: bass ~ ~ x ~\nsection verse 2 bars {\n  deck A: sing \"hold _ the lantern\" on melody\n}\narrange verse\nassert never B during sing\n";
    // (bass step 8 is the third onset, sung by `the`; a lyric of four syllables with a rest still holds that note)
    let _ = report(src);
    let e = report(src).assertions.iter().map(|a| a.status).collect::<Vec<_>>();
    assert_eq!(e, vec![Status::Fail]);
}

#[test]
fn a_line_timed_at_a_bar_sings_from_that_bar() {
    let src = |at: &str| {
        format!("tempo 96\nkey am\ndeck A: melody [a4 c5 e5 c5]\ndeck B: kick x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~\nsection verse 4 bars {{\n  {at}deck A: sing \"hold the lan-tern\" on melody bars=1\n}}\narrange verse\nassert never B during sing\n")
    };
    // from the start the vocal sings on bar 0 step 0, where the kick is; from bar 1, the kick on bar 0 is free (and bar 1's is not)
    assert!(failure(&src("")).0.contains("at bar 0, step 0"));
    assert!(failure(&src("at bar 1: ")).0.contains("at bar 1, step 0"));
    // a time inside the section that is after its end sings nothing
    holds(&src("at bar 9: "));
}

#[test]
fn when_the_vocal_cannot_be_told_the_constraint_says_so() {
    let line = |timing: &str| format!("tempo 96\nkey am\ndeck A: melody [a4 c5 e5 c5]\ndeck B: kick x ~ ~ ~\n{timing}deck A: sing \"hold the lan-tern\" on melody\nassert never B during sing\n");
    for (timing, why) in [("every 4 bars: ", "it is timed `every 4 bars`"), ("on next phrase 16: ", "it is timed `on next phrase 16`"), ("at bar 1 beat 2: ", "it is timed inside a bar")] {
        let src = line(timing);
        let e = one_error(&src);
        assert_eq!(e.message, format!("cannot tell when the vocal sings: the `sing` on line 5 is not at a known time ({why})"), "{timing}");
        assert_eq!(at(&src, &e), (6, 23), "{timing}");
        assert!(e.help.as_deref().unwrap().contains("give that line no time word, or `at bar N`"));
    }
    // a line that waits for a key is not at a known time either
    let e = one_error(&line("").replace("on melody\n", "on melody when held(key)\n"));
    assert!(e.message.contains("its `when` reads a key or a pad"), "{}", e.message);
}

#[test]
fn every_misuse_of_sing_is_said_in_the_languages_terms() {
    let lines = "tempo 96\nkey am\ndeck A: melody [a4 c5 e5 c5]\ndeck B: kick x ~ ~ ~\nsection verse 2 bars {\n  deck A: sing \"hold the lan-tern\" on melody\n}\narrange verse\n";
    let e = one_error(&format!("{lines}assert never sing beside sing\n"));
    assert_eq!(e.message, "`sing` is a time, not a part: it goes after `during`");
    assert_eq!(at(&format!("{lines}assert never sing beside sing\n"), &e), (9, 14));
    let e = one_error(&format!("{lines}assert never B beside sing\n"));
    assert_eq!(e.message, "`sing` is a time, not a part: `beside` needs a part, and `during` is the word for the vocal");
    // no sing line at all
    let e = one_error("deck B: kick x ~ ~ ~\nassert never B during sing\n");
    assert_eq!(e.message, "there is no `sing` line in this program, so nothing sings");
    assert!(e.help.as_deref().unwrap().contains("write one first"), "{:?}", e.help);
    // sung on the very part the rule is about
    let src = format!("{lines}assert never A during sing\n");
    let e = one_error(&src);
    assert_eq!(e.message, "the vocal is sung on deck A's own `melody` (the `sing` on line 6), so deck A sounds whenever it sings");
    assert_eq!(at(&src, &e), (9, 14));
    assert!(e.help.as_deref().unwrap().contains("assert never B during sing"), "{:?}", e.help);
    let e = one_error(&format!("{lines}assert never melody during sing\n"));
    assert!(e.message.starts_with("the vocal is sung on deck A's own `melody`"), "{}", e.message);
    // another part of that deck is fine to name
    // (the notes are on steps 0, 4, 8 and 12 and the voice lets go a step before the next: step 3 is clear of it)
    holds(&format!("{lines}assert never A.kick during sing\n").replace("deck B: kick x ~ ~ ~", &format!("deck A: kick {}", bar(&[3]))));
}

/// The same program in a host that sings in the nets (the browser): the voice's own part is in the deck's net, and the verdict
/// is the one the language gives with no host at all.
struct InNets(StaticVoices);

impl Voices for InNets {
    fn names(&self) -> Vec<String> {
        self.0.names()
    }
    fn is_neural(&self, name: &str) -> bool {
        self.0.is_neural(name)
    }
    fn neural_installed(&self) -> bool {
        self.0.neural_installed()
    }
    fn default_voice(&self) -> String {
        self.0.default_voice()
    }
    fn sings_in_nets(&self) -> bool {
        true
    }
}

#[test]
fn the_verdict_does_not_depend_on_the_host() {
    let voices = StaticVoices::default();
    let nets = InNets(StaticVoices::default());
    for src in [ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~"), ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ e2 ~ ~ ~ ~ ~ a1 ~ ~ ~")] {
        let judge = |env: &Env| {
            let c = djlive::compile::check_report(&src, env).compiled.expect("compiles");
            c.assertions.iter().map(|a| a.line()).collect::<Vec<_>>()
        };
        let plain = judge(&Env::none());
        assert_eq!(plain, judge(&Env { voices: Some(&voices), ..Env::default() }));
        assert_eq!(plain, judge(&Env { voices: Some(&nets), ..Env::default() }));
    }
    // the browser's sung part is in the nets, and the constraint does not read it (a part no program wrote)
    let c = check_with(&ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~"), &Env { voices: Some(&nets), ..Env::default() }).compiled.unwrap();
    assert!(c.segments.iter().flat_map(|s| s.decks.values()).flat_map(|d| d.parts.iter()).any(|p| p.role.starts_with("vox")), "the nets have the voice's part");
    assert_eq!(c.assertions.len(), 1);
    // a rule about deck A's own parts does not see the voice's part as one of them
    let c = check_with(&ANSWER.replace("BASS", "~ ~ ~ ~ ~ ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~").replace("assert never B during sing", "assert never A.hihat beside A.hihat"), &Env { voices: Some(&nets), ..Env::default() }).compiled.unwrap();
    assert_eq!(c.assertions[0].status, Status::Pass);
}

// ---- chance: judged on what a part may play

#[test]
fn a_part_with_chance_is_judged_on_everything_it_may_play() {
    // two chance hits on neighbouring steps: some roll keeps both
    let src = format!("deck A: snare ~ ~ ~ ~ x?0.5 x?0.5 {}\nend at bar 4\nassert never A.snare beside A.snare\n", "~ ~ ~ ~ ~ ~ ~ ~ ~ ~");
    let e = broken(&src);
    assert!(e.message.starts_with("assert never A.snare beside A.snare: two `snare` hits can touch on deck A, bar 0, steps 4 and 5"), "{}", e.message);
    let help = e.help.unwrap();
    assert!(help.contains("is kept only by chance: `x?0.5` at 1:"), "{help}");
    // the same hits with no chance on one of them: the other can never touch it
    let sure = src.replace("x?0.5 x?0.5", "x ~?0.5");
    holds(&sure.replace("assert never A.snare beside A.snare", "assert never A.snare beside A.snare"));
}

#[test]
fn the_verdict_for_chance_is_the_same_whatever_the_text_rolls() {
    // the rolled grid of `x?0.5 x?0.5` depends on the text (its seed is a hash of it): nudging the velocity does not re-roll,
    // but a second part or another seed does. The verdict is about every roll, so none of them changes it.
    let base = |tail: &str| format!("deck A: snare ~ ~ ~ ~ x?0.5 x?0.5 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~{tail}\nend at bar 6\nassert never A.snare beside A.snare\n");
    for tail in ["", " seed=1", " seed=2", " seed=77", " vel=90", " vel=10..127"] {
        let src = base(tail);
        let c = report(&src);
        assert_eq!(c.assertions[0].status, Status::Fail, "{tail}");
    }
    // a roll that happens to play only one of the pair does not hide the other
    let rolled: Vec<usize> = (1..=40)
        .filter(|seed| {
            let c = report(&format!("deck A: snare ~ ~ ~ ~ x?0.5 x?0.5 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ seed={seed}\nend at bar 1\nassert never A.snare beside A.snare\n"));
            let hits = &c.decks[&djlive::ast::Deck::A].parts[0].cycles[0];
            hits.len() < 2
        })
        .collect();
    assert!(!rolled.is_empty(), "some seed rolls fewer than both hits");
    for seed in rolled {
        let src = format!("deck A: snare ~ ~ ~ ~ x?0.5 x?0.5 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ seed={seed}\nend at bar 1\nassert never A.snare beside A.snare\n");
        assert_eq!(report(&src).assertions[0].status, Status::Fail, "seed {seed}: the roll played fewer, the rule is about every roll");
    }
}

#[test]
fn every_option_of_a_choose_may_play() {
    // `choose [a b]` plays one of them a bar; the rule holds for both at once (an option is something that could play)
    let src = |weights: &str| format!("deck A: snare choose [{} {}] weights {weights}\nend at bar 4\nassert never A.snare beside A.snare\n", "x*0", "x");
    let _ = src;
    let two = "deck A: snare choose [[x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~] [~ x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]] weights 1 1\nend at bar 4\nassert never A.snare beside A.snare\n";
    assert!(failure(two).0.contains("can touch"), "{}", failure(two).0);
    // an option that no weight can pick is not one
    let zero = two.replace("weights 1 1", "weights 1 0");
    holds(&zero);
}

#[test]
fn maybe_and_a_chance_on_a_group_count_too() {
    let src = "deck A: snare maybe([~ ~ ~ ~ x x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~], 0.1)\nend at bar 2\nassert never A.snare beside A.snare\n";
    assert!(failure(src).0.contains("can touch"));
    let grp = "deck A: snare [~ ~ ~ ~ x x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]?0.3\nend at bar 2\nassert never A.snare beside A.snare\n";
    assert!(failure(grp).0.contains("can touch"));
}

#[test]
fn a_part_with_no_chance_has_no_chance_grid() {
    let c = ok(&drums(2, &format!("  deck A: snare {}", bar(&[4])), ""));
    for seg in &c.segments {
        for ir in seg.decks.values() {
            for p in &ir.parts {
                assert!(p.may.0.is_empty());
            }
        }
    }
    // and a part with chance has one, a superset of what it plays
    let c = ok(&drums(4, "  deck A: snare x?0.5 x?0.5 x?0.5 x?0.5", ""));
    let p = &c.segments[0].decks[&djlive::ast::Deck::A].parts[0];
    assert_eq!(p.may.0.len(), p.cycles.len());
    for (played, may) in p.cycles.iter().zip(&p.may.0) {
        assert_eq!(may.len(), 4);
        for h in played {
            assert!(may.iter().any(|m| m.step == h.step && m.note == h.note));
        }
    }
}

/// Every hit a part plays is one it may play: through an evolve's generations, a stack's later stretches and a long net's chain.
fn played_is_may(c: &Compiled) {
    let all: Vec<&djlive::compile::PartIr> = if c.arranged { c.segments.iter().flat_map(|s| s.decks.values()).flat_map(|d| d.parts.iter()).collect() } else { c.decks.values().flat_map(|d| d.parts.iter()).collect() };
    let mut seen = 0;
    for p in all.into_iter().filter(|p| p.chance) {
        assert!(!p.may.0.is_empty(), "a part with chance has its grid");
        assert_eq!(p.may.0.len(), p.cycles.len(), "the grids are the same length");
        for (played, may) in p.cycles.iter().zip(&p.may.0) {
            for h in played {
                assert!(may.iter().any(|m| m.step == h.step && m.note == h.note), "{} plays {h:?}, which it may not", p.text);
            }
        }
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn the_chance_grid_follows_every_change_made_to_the_notes() {
    // an evolve's rotate, reverse, transpose, invert and thin
    let evolve = "section drop 16 bars {\n  deck A: snare x?0.6 ~ x?0.6 ~ x?0.4 ~ ~ ~ | bass c2?0.7 e2 ~ g2?0.5 ~ ~ ~ ~\n  evolve every 2 bars {\n    snare: rotate 1 | reverse | thin 0.2\n    bass: transpose 2 | invert | thin 0.1 seed=3\n    stop when gen == 6\n  }\n}\narrange drop\n";
    played_is_may(&ok(evolve));
    // a stack of sections of unequal lengths: the later stretch of the longer one is rotated
    let stack = "section short 4 bars {\n  deck A: kick x?0.5 ~ x ~\n}\nsection long 8 bars {\n  deck A: hihat <x?0.5 x> x?0.5 x ~ x?0.5\n}\narrange stack(short, long)\n";
    played_is_may(&ok(stack));
    // a net too long for one (more than 64 bars): chained from nets of 64
    let long = format!("deck A: snare <{}>?0.5 x ~ ~ x?0.3\ndeck B: bass <{}>\n", "x ~ x ~ ".repeat(1).trim_end(), (0..67).map(|i| format!("c{}", 2 + i % 3)).collect::<Vec<_>>().join(" "));
    let c = ok(&long);
    assert!(c.arranged, "chained");
    played_is_may(&c);
}

#[test]
fn a_constraint_sees_the_generation_that_is_playing() {
    // the rotate walks the pair together: it is a pair only on the last step of the bar (14 and 15 after one rotation of the
    // generation, bar 2's)
    let src = "section drop 6 bars {\n  deck A: snare x ~ x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~\n  evolve every 2 bars {\n    snare: rotate 13\n    stop when gen == 2\n  }\n}\narrange drop\nassert never A.snare beside A.snare\n";
    // generation 0 has hits on 0 and 2 (every bar), generation 1 on 13 and 15, generation 2 on 10 and 12: a step apart always
    holds(src);
    let touch = src.replace("snare x ~ x ~", "snare x x ~ ~");
    let (m, _) = failure(&touch);
    assert!(m.contains("bar 0, steps 0 and 1"), "{m}");
    // only from generation 1: the pair is split by 13 steps in generation 0 and comes together when the rotate wraps
    let late = "section drop 6 bars {\n  deck A: snare x ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ x\n  evolve every 2 bars {\n    snare: rotate 1\n    stop when gen == 2\n  }\n}\narrange drop\nassert never A.snare beside A.snare\n";
    // generation 0: hits on 0 and 15 (a bar apart? no: step 15 and step 0 of the next bar are neighbours)
    let (m, _) = failure(late);
    assert!(m.contains("bar 0, step 15 and bar 1, step 0"), "{m}");
}

// ---- ends on

#[test]
fn the_degree_is_the_sections_own_key() {
    let src = |degree: u32, key: &str, bass: &str| format!("key {key}\ndeck B: bass {bass}\nend at bar 2\nassert bass ends on {degree}\n");
    holds(&src(1, "am", "a2 ~ ~ ~"));
    holds(&src(5, "am", "a2 ~ e2 ~"));
    holds(&src(3, "am", "a2 ~ c3 ~"));
    // the same note is another degree in another key
    holds(&src(6, "cm", "c2 ~ ~ g#2"));
    assert!(failure(&src(1, "cm", "c2 ~ ~ g#2")).0.contains("ends on g#, degree 6, not c"));
    // a mode: the third of d dorian is f; the seventh is c
    holds(&src(3, "d:dorian", "d2 ~ ~ f2"));
    holds(&src(7, "d:dorian", "d2 ~ ~ c3"));
    // any octave
    holds(&src(1, "am", "a1 ~ ~ ~"));
    holds(&src(1, "am", "a4 ~ ~ ~"));
    // the message when it is not a note of the key at all
    let (m, _) = failure(&src(1, "am", "a2 ~ ~ d#3"));
    assert!(m.contains("ends on d#, which is no note of the key, not a"), "{m}");
}

#[test]
fn a_scale_of_five_notes_goes_round_into_the_next_octave() {
    // degree 6 and 7 of a pentatonic key are the tonic and the second an octave up, as a degree in a pattern is
    holds("key a:pent\ndeck B: bass a2 ~ ~ ~\nend at bar 1\nassert bass ends on 1\nassert bass ends on 6\n");
    holds("key a:pent\ndeck B: bass b2 ~ ~ ~\nend at bar 1\nassert bass ends on 7\n");
}

#[test]
fn the_last_onset_is_what_counts_and_its_lowest_note() {
    // a chord on the last onset: its lowest note
    let src = |chord: &str| format!("key am\ndeck B: harmony a3 ~ ~ ~ {chord}\nend at bar 1\nassert harmony ends on 1\n");
    holds(&src("[a3,c4,e4]"));
    // an inversion: the lowest note is the third
    let (m, _) = failure(&src("[c4,e4,a4]"));
    assert!(m.contains("ends on c, degree 3, not a"), "{m}");
    // later notes of other parts do not matter: it is about the part named
    holds("key am\ndeck B: harmony a3 ~ ~ ~ | bass c2 ~ ~ ~\nend at bar 1\nassert harmony ends on 1\n");
    // a role names it on every deck: the last onset is the latest of the two, whichever deck it is on
    holds(&format!("key am\ndeck A: bass a2 ~ e2 ~\ndeck B: bass {}\nend at bar 1\nassert bass ends on 1\n", bar(&[14]).replace('x', "a1")));
    assert!(failure("key am\ndeck A: bass a2 ~ ~ ~\ndeck B: bass ~ ~ e2 ~\nend at bar 1\nassert bass ends on 1\n").0.contains("ends on e, degree 5"));
    // ... and one deck's part by its name: A's bass ends on its tonic whatever B plays after it
    holds("key am\ndeck A: bass a2 ~ ~ ~\ndeck B: bass ~ ~ e2 ~\nend at bar 1\nassert A.bass ends on 1\n");
}

#[test]
fn in_names_the_section_and_the_others_are_not_judged() {
    let src = "key am\nsection verse 2 bars {\n  deck B: bass a2 ~ ~ ~\n}\nsection lift 2 bars {\n  deck B: bass a2 ~ e2 ~\n}\nsection coda 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange verse lift coda\n";
    // the coda plays no bass: skipped, and named
    let (m, _) = failure(&format!("{src}assert bass ends on 1\n"));
    assert_eq!(m, "assert bass ends on 1: in `lift` (key am) the bass ends on e, degree 5, not a, at bar 3, step 8 (not judged where the bass plays no note: `coda`)");
    holds(&format!("{src}assert bass ends on 1 in verse\nassert bass ends on 5 in lift\n"));
    // a part with no note in the section named
    let e = one_error(&format!("{src}assert bass ends on 1 in coda\n"));
    assert_eq!(e.message, "the bass plays no note in `coda`, so there is nothing for it to end on");
    // a section that is not there
    assert_eq!(one_error(&format!("{src}assert bass ends on 1 in lyft\n")).message, "there is no section `lyft`");
}

#[test]
fn a_section_that_plays_twice_is_judged_each_time_and_an_evolve_is_one_section() {
    let src = "key am\nsection verse 4 bars {\n  deck B: bass <a2 a2 a2 e2>\n}\narrange verse verse\nassert bass ends on 1\n";
    // the last bar of the verse is the fourth: e, both times
    let (m, help) = failure(src);
    assert!(m.starts_with("assert bass ends on 1: in `verse` (key am) the bass ends on e, degree 5, not a, at bar 3, step 0"), "{m}");
    assert!(help.contains("It ends on another degree in `verse` too."), "{help}");
    // an evolve's generations are one place: the section ends on its last generation's last note
    let evolving = "key am\nsection verse 8 bars {\n  deck B: bass a2 ~ e2 ~\n  evolve every 2 bars {\n    bass: transpose 7\n    stop when gen == 3\n  }\n}\narrange verse\nassert bass ends on 1\n";
    let (m, _) = failure(evolving);
    assert!(m.contains("at bar 7, step 8"), "the last note of the last generation: {m}");
}

#[test]
fn a_stack_is_one_stretch_and_in_names_either_of_its_sections() {
    // `short` (2 bars) stacked with `long` (4): the first two bars are the stretch `long+short`, the next two `long` alone
    let src = "key am\nsection short 2 bars {\n  deck B: bass <a2 e2>\n}\nsection long 4 bars {\n  deck A: kick ~ ~ x ~\n}\narrange stack(short, long)\n";
    // in `short` the last bass note is bar 1's e (the stretch is judged with everything that plays in it)
    let (m, _) = failure(&format!("{src}assert bass ends on 1 in short\n"));
    assert!(m.starts_with("assert bass ends on 1 in short: in `short` (key am) the bass ends on e, degree 5, not a, at bar 1, step 0"), "{m}");
    // in `long` the stretches that go on are one place: it ends at bar 4, and the bass last played in bar 1
    let (m, _) = failure(&format!("{src}assert bass ends on 1 in long\n"));
    assert!(m.contains("in `long` (key am) the bass ends on e, degree 5, not a, at bar 1, step 0"), "{m}");
    // and a rule about notes is judged over the stack as one stretch
    holds(&format!("{src}assert never A.kick beside B.bass\nassert never A.kick during B.bass in short\n"));
}

#[test]
fn a_program_with_no_sections_is_one_section() {
    holds("key am\ndeck B: bass a2 ~ ~ ~ | harmony a3 ~ ~ ~\nassert bass ends on 1\n");
    let (m, _) = failure("key am\ndeck B: bass a2 ~ e2 ~\nassert bass ends on 1\n");
    assert!(m.starts_with("assert bass ends on 1: in the program (key am) the bass ends on e, degree 5, not a, at bar 0, step 8"), "{m}");
    // there is no section to say `in` of in what would fix it
    assert!(failure("key am\ndeck B: bass a2 ~ e2 ~\nassert bass ends on 1\n").1.contains("or write `assert bass ends on 5` if that ending is meant"));
}

#[test]
fn which_note_is_last_must_not_depend_on_the_dice() {
    let src = "key am\ndeck B: bass a2 ~ e2?0.5 ~\nassert bass ends on 1\n";
    let e = one_error(src);
    assert_eq!(e.message, "which note the bass of deck B ends on depends on the dice: it has chance in it (line 2)");
    assert_eq!(at(src, &e), (3, 8));
    assert_eq!(e.help.as_deref(), Some("put the closing note outside the chance, or ask about a part that has none"));
}

#[test]
fn ends_on_is_about_one_pitched_part_and_a_degree() {
    let base = "key am\ndeck A: kick x ~ ~ ~ | bass a2 ~ ~ ~\n";
    let e = one_error(&format!("{base}assert A.kick ends on 1\n"));
    assert_eq!(e.message, "the kick has no pitch: `ends on` is for bass, melody, harmony and arp");
    let e = one_error(&format!("{base}assert A ends on 1\n"));
    assert_eq!(e.message, "deck A has several parts: `ends on` is about one of them");
    assert_eq!(e.help.as_deref(), Some("say which: `assert A.bass ends on 1`"));
    let e = one_error(&format!("{base}assert master ends on 1\n"));
    assert_eq!(e.message, "`master` has no parts: it is the mix of both decks");
    let e = one_error(&format!("{base}assert sing ends on 1\n"));
    assert_eq!(e.message, "the vocal has no notes of its own to end on: it sings the notes of a part");
    for d in ["0", "8", "9", "12"] {
        let src = format!("{base}assert bass ends on {d}\n");
        let e = one_error(&src);
        assert_eq!(e.message, "a degree is 1 to 7", "{d}");
        assert_eq!(at(&src, &e), (3, 21));
    }
    // a bass that is nowhere
    let e = one_error("key am\ndeck A: kick x ~ ~ ~\nassert bass ends on 1\n");
    assert_eq!(e.message, "the bass plays no note in this program, so there is nothing for it to end on");
}

// ---- reading

#[test]
fn what_is_wrong_is_said_where_it_is_with_a_did_you_mean() {
    struct Case {
        src: &'static str,
        at: (usize, usize),
        message: &'static str,
        help: &'static str,
    }
    let cases = [
        Case { src: "deck A: kick x\nassert never A.snar beside A.snare\n", at: (2, 16), message: "`snar` is not a part", help: "did you mean `snare`?" },
        Case { src: "deck A: kick x\nassert never A.snare\n", at: (2, 21), message: "`never A.snare` is followed by `beside` or `during`, found the end of the line", help: "assert never A.snare beside A.snare" },
        Case { src: "deck A: kick x\nassert never A.snare besid A.snare\n", at: (2, 22), message: "`never A.snare` is followed by `beside` or `during`, found `besid`", help: "did you mean `beside`?" },
        Case { src: "deck A: kick x\nassert never\n", at: (2, 13), message: "expected what must never happen: a part like `A.snare` or `bass`, or a deck, found the end of the line", help: "assert never B during sing" },
        Case { src: "deck A: kick x\nassert never a.snare beside A.snare\n", at: (2, 14), message: "expected what must never happen: a part like `A.snare` or `bass`, or a deck, found `a`", help: "decks are upper case: `A`" },
        Case { src: "deck A: kick x\nassert never kik beside kick\n", at: (2, 14), message: "expected what must never happen: a part like `A.snare` or `bass`, or a deck, found `kik`", help: "did you mean `kick`?" },
        Case { src: "deck A: kick x\nassert never kick beside\n", at: (2, 25), message: "expected the part it must not touch, found the end of the line", help: "assert never B during sing" },
        Case { src: "deck A: kick x\nassert never kick during\n", at: (2, 25), message: "expected what it must not sound with: a part, a deck or `sing`, found the end of the line", help: "assert never B during sing" },
        Case { src: "deck A: kick x\nassert never master.kick beside kick\n", at: (2, 14), message: "`master` has no parts: it is the mix of both decks", help: "a part belongs to a deck" },
        Case { src: "deck A: kick x\nassert bass\n", at: (2, 12), message: "`assert bass` is followed by `ends on`, found the end of the line", help: "assert bass ends on 1" },
        Case { src: "deck A: kick x\nassert bass end on 1\n", at: (2, 13), message: "`assert bass` is followed by `ends on`, found `end`", help: "did you mean `ends`?" },
        Case { src: "deck A: kick x\nassert bass ends 1\n", at: (2, 18), message: "`ends` is followed by `on` and a degree, found a number", help: "assert bass ends on 1" },
        Case { src: "deck A: kick x\nassert bass ends on\n", at: (2, 20), message: "expected a degree of the key, 1 to 7, found the end of the line", help: "the tonic is 1" },
        Case { src: "deck A: kick x\nassert bass ends on i\n", at: (2, 21), message: "expected a degree of the key, 1 to 7, found `i`", help: "a chord numeral like `i` is not one" },
        Case { src: "deck A: kick x\nassert kik ends on 1\n", at: (2, 8), message: "`assert` is followed by what to assert, found `kik`", help: "did you mean `kick`?" },
        Case { src: "deck A: kick x\nassert nevr A.kick beside A.kick\n", at: (2, 8), message: "`assert` is followed by what to assert, found `nevr`", help: "did you mean `never`?" },
        Case { src: "deck A: kick x\nassert never A.kick beside A.kick in\n", at: (2, 37), message: "expected the name of a section, found the end of the line", help: "" },
    ];
    let mut wrong = Vec::new();
    for c in cases {
        // (not the last line of the text, so that "the end of the line" is what is found there)
        let src = format!("{}deck B: kick x\n", c.src);
        let Some(d) = errors(&src).into_iter().next() else {
            wrong.push(format!("{:?}: no error", c.src));
            continue;
        };
        let all = format!("{} {}", d.help.clone().unwrap_or_default(), d.expected.join(" "));
        if (at(&src, &d), d.message.as_str()) != (c.at, c.message) || !all.contains(c.help) {
            wrong.push(format!("{:?}: {:?} {:?} help {:?} expected {:?}", c.src, at(&src, &d), d.message, d.help, d.expected));
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[test]
fn a_constraint_belongs_at_the_top_of_the_program() {
    let e = one_error("section one 4 bars {\n  deck A: kick x\n  assert never A.kick beside A.kick\n}\narrange one\n");
    assert_eq!(e.message, "an `assert` belongs at the top of the program, outside blocks");
}

#[test]
fn the_words_of_a_constraint_still_name_a_let_a_def_or_a_section() {
    // `never`, `beside` and `during` mean something only inside an `assert`, as `ends` and `period` do: they name lets here,
    // are put in place where they are names, and are the words where they are syntax
    let src = "let never = 3\nlet beside = 2\nlet during = 4\nlet ends = 1\ndeck A: kick x ~ ~ ~ | hihat x*during\ndeck B: bass ~ c2 ~ ~\nend at bar 2\nassert never A.kick beside A.hihat\nassert never B during A.kick\nassert bass ends on ends\n";
    let c = ok(src);
    assert_eq!(c.assertions.len(), 3);
    assert!(c.text.contains("assert never A.kick beside A.hihat"), "{}", c.text);
    assert!(c.text.contains("assert never B during A.kick"), "{}", c.text);
    // the `ends` after `on` is a degree, and the let's number is put in its place
    assert!(c.text.contains("assert bass ends on 1\n"), "{}", c.text);
    // names of a section, a def, a pattern let
    ok("section never 2 bars {\n  deck A: kick x ~ ~ ~\n}\nsection during 2 bars {\n  deck A: kick x ~ ~ ~\n}\nsection beside 2 bars {\n  deck B: bass c2 ~ ~ ~\n}\narrange never during beside\nassert never A.kick beside A.kick in never\n");
    // the editor colours the words by what they are where they stand
    let text = "assert never A.snare beside A.snare\nlet never = 3\n";
    let hl = djlive::highlight::highlight(text);
    let class = |word: &str, nth: usize| {
        let spans: Vec<_> = hl.iter().filter(|(s, _)| s.text(text) == word).collect();
        format!("{:?}", spans[nth].1)
    };
    assert_eq!(class("never", 0), "Keyword");
    assert_eq!(class("beside", 0), "Keyword");
    assert_ne!(class("never", 1), "Keyword", "`let never` names a let");
}

// ---- warnings, edges, the timeline

#[test]
fn a_rule_that_holds_for_nothing_says_so() {
    let src = drums(2, &format!("  deck A: snare {}", bar(&[4])), "assert never A.clap beside A.snare\n");
    let w = warnings(&src);
    assert_eq!(w, vec!["assert never A.clap beside A.snare: `A.clap` plays no note in this program, so there is nothing for the rule to hold against".to_string()]);
    // it still holds, and says what it checked
    assert_eq!(verdicts(&src)[0].0, Status::Pass);
    // in a section where the subject is silent
    let two = "section verse 2 bars {\n  deck A: snare x ~ ~ ~\n}\nsection coda 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange verse coda\nassert never A.snare beside A.snare in coda\n";
    assert_eq!(warnings(two), vec!["assert never A.snare beside A.snare in coda: `A.snare` plays no note in `coda`, so there is nothing for the rule to hold against".to_string()]);
    // a rule about the vocal where the vocal says nothing in that section
    let none = "tempo 96\nkey am\ndeck A: melody [a4 c5 e5 c5]\ndeck B: kick x ~ ~ ~\nsection verse 2 bars {\n  deck A: sing \"hold the lan-tern\" on melody\n}\nsection coda 2 bars {\n  deck B: kick x ~ ~ ~\n}\narrange verse coda\nassert never B during sing in coda\n";
    assert!(warnings(none).iter().any(|w| w.contains("the vocal plays no note in `coda`")), "{:?}", warnings(none));
}

#[test]
fn a_program_that_ends_is_judged_to_its_end_and_a_last_section_goes_on_past_its_own() {
    // a four-bar song whose pair of neighbouring hits is in bar 2: `end at bar 2` stops before it plays
    let src = |end: &str| format!("section verse 4 bars {{\n  deck A: snare <[{}] [{}] [{}] [{}]>\n}}\narrange verse\n{end}assert never A.snare beside A.snare\n", bar(&[0]), bar(&[0]), bar(&[0, 1]), bar(&[0]));
    holds(&src("end at bar 2\n"));
    let (m, _) = failure(&src("end at bar 3\n"));
    assert!(m.ends_with("in `verse`, bar 2, steps 0 and 1"), "{m}");
    assert!(failure(&src("")).0.ends_with("in `verse`, bar 2, steps 0 and 1"));
    // `end at bar 6` past a two-bar song: the last section plays on, so its cycle comes round again and bar 3 is judged
    let long = |end: u32| format!("section verse 2 bars {{\n  deck A: snare <[{}] [{}]>\n}}\narrange verse\nend at bar {end}\nassert never A.snare beside A.snare\n", bar(&[0]), bar(&[15]));
    holds(&long(2));
    assert!(failure(&long(6)).0.ends_with("in `verse`, bar 1, step 15 and bar 2, step 0"), "{}", failure(&long(6)).0);
}

#[test]
fn a_program_too_long_to_come_round_cannot_be_decided_and_is_not_passed() {
    // two decks that take 64 and 63 bars to come round: 4032 bars; a vocal of 31 bars on a flat program makes it 124992
    let alt = |n: usize, note: &str| format!("<{}>", (0..n).map(|i| if i == 0 { note.to_string() } else { "~".to_string() }).collect::<Vec<_>>().join(" "));
    let src = format!("deck A: melody {}\ndeck B: bass {}\nsing \"hold the lan-tern\" on melody bars=31\nassert never B during sing\n", alt(64, "c4"), alt(63, "c2"));
    let e = one_error(&src);
    assert!(e.message.starts_with("cannot tell whether this holds: this program takes 124992 bars to come round, and an assertion walks at most 4096"), "{}", e.message);
    assert_eq!(e.help.as_deref(), Some("give the program sections and an `arrange`, or `end at bar N`"));
    // the same without the vocal is 4032 bars: decided
    holds(&format!("deck A: melody {}\ndeck B: bass {}\nassert never A.melody beside B.bass\n", alt(64, "c4"), alt(63, "c2")));
}

#[test]
fn a_judged_program_is_refused_by_check_and_reported_by_check_report() {
    let src = drums(2, &format!("  deck A: snare {}", bar(&[4, 5])), "assert never A.snare beside A.snare\n");
    assert!(check(&src).compiled.is_none());
    let r = report(&src);
    assert_eq!(r.assertions.len(), 1);
    assert_eq!(r.assertions[0].status, Status::Fail);
    assert_eq!(r.assertions[0].text, "assert never A.snare beside A.snare");
    assert!(r.assertions[0].level.is_none(), "a constraint is decided here, not measured");
    // a true one is silent in `check`: no diagnostic for it
    let fine = drums(2, &format!("  deck A: snare {}", bar(&[4, 12])), "assert never A.snare beside A.snare\n");
    let c = check(&fine);
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(verdicts(&fine), vec![(Status::Pass, "assert never A.snare beside A.snare: held over 2 bars".to_string())]);
}

#[test]
fn a_held_constraint_says_what_it_was_held_over() {
    let src = |extra: &str| format!("section verse 2 bars {{\n  deck A: snare {}{extra}\n}}\nsection chorus 6 bars {{\n  deck A: snare {}\n}}\narrange verse chorus\n", bar(&[4, 12]), bar(&[4, 12]));
    assert_eq!(verdicts(&format!("{}assert never A.snare beside A.snare\n", src(""))), vec![(Status::Pass, "assert never A.snare beside A.snare: held over 8 bars".to_string())]);
    assert_eq!(verdicts(&format!("{}assert never A.snare beside A.snare in chorus\n", src(""))), vec![(Status::Pass, "assert never A.snare beside A.snare in chorus: held over the 6 bars of `chorus`".to_string())]);
    // chance counted as played
    let dice = "deck A: snare x?0.5 ~ ~ ~\nend at bar 3\nassert never A.snare beside A.snare\n";
    assert_eq!(verdicts(dice), vec![(Status::Pass, "assert never A.snare beside A.snare: held over 3 bars, counting every chance hit as played".to_string())]);
}

#[test]
fn a_constraint_does_not_change_what_plays() {
    // the compiled nets (their CIDs) of a program are those of the program without the statement: `assert` says nothing to the engine
    let plain = drums(2, &format!("  deck A: snare {} | bass c2?0.5 e2 ~ ~", bar(&[4, 12])), "");
    let with = format!("{plain}assert never A.snare beside A.snare\nassert never A.snare beside A.bass\n");
    assert_eq!(ok(&plain).net_cids(), ok(&with).net_cids());
}
