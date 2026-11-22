//! Call and response between decks (docs/BTW.md "Call and response", djlive/src/answer.rs): `rests(p)` is a hit on every step
//! where `p` rests, `rests(p, q)` is `q` kept only on those steps. Every test here fails on a build without `rests`
//! ("`rests` is not a generator"). That the nets are the longhand's is proved with `same_as` in btwapi/tests/answer.rs; here
//! the hits themselves are looked at, and what `check` says.

mod common;

use common::{reformat, Rng};
use djlive::ast::Deck;
use djlive::compile::{check, Compiled};
use djlive::diag::Diagnostic;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    c.compiled.unwrap_or_else(|| panic!("does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check(src);
    assert!(c.compiled.is_none(), "should not compile: {src}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn warnings(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics.into_iter().filter(|d| d.severity == djlive::Severity::Warning).collect()
}

/// The sixteenth steps that have a hit in a part (a chord is one), bar by bar of its net.
fn steps(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<u8>> {
    let part = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role}"));
    part.cycles
        .iter()
        .map(|b| {
            let mut v: Vec<u8> = b.iter().map(|h| h.step).collect();
            v.dedup();
            v
        })
        .collect()
}

/// (step, note) of every hit of a part, bar by bar.
fn notes(c: &Compiled, deck: Deck, role: &str) -> Vec<Vec<(u8, u8)>> {
    let part = c.decks[&deck].parts.iter().find(|p| p.role == role).unwrap_or_else(|| panic!("no {role}"));
    part.cycles.iter().map(|b| b.iter().map(|h| (h.step, h.note)).collect()).collect()
}

/// What `deck B: clap <pattern>` plays, bar by bar.
fn clap(pattern: &str) -> Vec<Vec<u8>> {
    steps(&ok(&format!("deck B: clap {pattern}\n")), Deck::B, "clap")
}

fn one_bar(v: &[u8]) -> Vec<Vec<u8>> {
    vec![v.to_vec()]
}

// ---------------------------------------------------------------------------------------------------------------
// what it plays

#[test]
fn the_answer_is_a_hit_on_every_step_the_call_rests() {
    // eight steps: the tresillo kick, 1 4 7; the answer is 2 3 5 6 8, on the sixteenths 2 4 8 10 14
    assert_eq!(clap("rests([x ~ ~ x ~ ~ x ~])"), one_bar(&[2, 4, 8, 10, 14]));
    // the call is a `let`, as it is meant to be written, and plays on its own deck
    let c = ok("let call = x ~ ~ x ~ ~ x ~\ndeck A: kick call\ndeck B: clap rests(call)\n");
    assert_eq!(steps(&c, Deck::A, "kick"), one_bar(&[0, 6, 12]));
    assert_eq!(steps(&c, Deck::B, "clap"), one_bar(&[2, 4, 8, 10, 14]));
}

#[test]
fn the_steps_are_the_calls_own_not_the_sixteenths() {
    // four steps (x ~ x ~) answer with two hits, not fourteen: the gaps of a call are its own
    assert_eq!(clap("rests([x ~ x ~])"), one_bar(&[4, 12]));
    // sixteen steps and two steps
    assert_eq!(clap("rests([x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~])"), one_bar(&[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15]));
    assert_eq!(clap("rests([x ~])"), one_bar(&[8]));
    assert_eq!(clap("rests([~ x])"), one_bar(&[0]));
}

#[test]
fn a_step_sounds_when_something_starts_in_it() {
    // a group, a repeat and a group with a rest are all onsets in their step
    assert_eq!(clap("rests([[x x] ~ [~ x] x*2 ~ [x ~] [~ ~] ~])"), one_bar(&[2, 8, 12, 14]));
    // a ghost, a flam, a ratchet and an accent are one onset each, in their step
    assert_eq!(clap("rests([x. x& x:3 x! ~ ~ ~ ~])"), one_bar(&[8, 10, 12, 14]));
    // a chord is one onset
    assert_eq!(clap("rests([chord(am) ~ chord(c) ~])"), one_bar(&[4, 12]));
}

#[test]
fn a_call_that_is_one_term_is_looked_through() {
    // a lone group is its inside
    assert_eq!(clap("rests([[x ~ x ~]])"), one_bar(&[4, 12]));
    assert_eq!(clap("rests([x ~ x ~, ~ x ~ x])"), one_bar(&[]));
    // layers on a shared grid: three against two is six steps; the onsets are 0 1/3 2/3 and 0 1/2, so steps 2 and 6 of six
    // are the rests, and they snap to the sixteenths they are nearest (a warning says so)
    assert_eq!(clap("rests([x x x, x x])"), one_bar(&[3, 13]));
    // a Euclidean rhythm is its steps: e(3,8) hits 1 4 7, the answer is the other five of eight
    assert_eq!(clap("rests(e(3,8))"), one_bar(&[2, 4, 8, 10, 14]));
    // a transform keeps the steps of its pattern: reverse(x ~ ~ x ~ ~ x ~) = ~ x ~ ~ x ~ ~ x
    assert_eq!(clap("rests(reverse([x ~ ~ x ~ ~ x ~]))"), one_bar(&[0, 4, 6, 10, 12]));
    assert_eq!(clap("rests(rotate([x ~ ~ ~], 1))"), one_bar(&[0, 8, 12]));
    // a repeat is that many times the steps: [x ~ x ~]*2 is eight steps
    assert_eq!(clap("rests([x ~ x ~]*2)"), one_bar(&[2, 6, 10, 14]));
    // fast is the same
    assert_eq!(clap("rests(fast([x ~], 2))"), one_bar(&[4, 12]));
    // arp is its n, e(k, n) is its n
    assert_eq!(clap("rests(arp(am, up, 4))"), one_bar(&[]));
    // slow, over bars that divide its steps: the first half of eight steps stretched is four steps a bar
    assert_eq!(clap("rests(slow([x ~ x ~ x ~ ~ ~], 2))"), vec![vec![4, 12], vec![4, 8, 12]]);
    // an every picks the bar's own pattern
    assert_eq!(clap("rests(every(2, [x x x x], [x ~ ~ ~]))"), vec![vec![4, 8, 12], vec![]]);
}

#[test]
fn a_call_that_comes_round_over_bars_is_answered_bar_by_bar() {
    // the call is silent every second bar: that bar is one step, and it is answered whole
    assert_eq!(clap("rests([<[x ~ x ~] ~>])"), vec![vec![4, 12], vec![0]]);
    let c = ok("let call = <[x ~ ~ x] ~>\ndeck A: kick call\ndeck B: clap rests(call)\n");
    assert_eq!(steps(&c, Deck::B, "clap"), vec![vec![4, 8], vec![0]]);
    // an alternation of different grids: four steps one bar, two the next
    assert_eq!(clap("rests([<[x ~ x ~] [x ~]>])"), vec![vec![4, 12], vec![8]]);
    // the net is as long as the longer of the call and the reply
    assert_eq!(clap("rests([x ~ x ~], [<[x x x x] [x x x x] [~ ~ ~ ~]>])"), vec![vec![4, 12], vec![4, 12], vec![]]);
}

#[test]
fn a_silent_call_is_one_step_and_the_answer_is_one_hit() {
    // a lone rest is one step: the downbeat
    assert_eq!(clap("rests([~])"), one_bar(&[0]));
    // and the reply is whole
    assert_eq!(clap("rests([~], [x ~ x x])"), one_bar(&[0, 8, 12]));
}

// ---------------------------------------------------------------------------------------------------------------
// the two-argument form: the reply, kept where the call rests

#[test]
fn a_reply_is_kept_only_where_the_call_rests() {
    let c = ok("let call = x ~ ~ x ~ ~ x ~\nlet reply = ~ x ~ x ~ x ~ x\ndeck A: kick call\ndeck B: clap rests(call, reply)\n");
    // the reply plays 2 4 6 8 of eight; the call holds 1 4 7; step 4's reply is lost to the call
    assert_eq!(steps(&c, Deck::B, "clap"), one_bar(&[2, 10, 14]));
    // a reply on its own, finer grid: sixteen steps against a call of four
    assert_eq!(clap("rests([x ~ x ~], [x*16])"), one_bar(&[4, 5, 6, 7, 12, 13, 14, 15]));
    // notes, in a tonal part, as they are written
    let c = ok("deck B: bass rests([x ~ x ~], [c2 d2 e2 f2])\n");
    assert_eq!(notes(&c, Deck::B, "bass"), vec![vec![(4, 38), (12, 41)]]);
    // the reply is read as this part reads it: its key and octave
    let c = ok("key dm\ndeck B: harmony rests([x ~ x ~], [1 2 3 4]) oct=3\n");
    assert_eq!(notes(&c, Deck::B, "harmony").iter().map(Vec::len).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn chance_in_the_reply_is_the_replys_own() {
    // only the call has to be fixed: a reply may roll dice, as any part may
    let c = ok("deck B: clap rests([x ~ x ~], [x*4?0.5])\n");
    assert!(c.decks[&Deck::B].parts[0].chance);
    // and it differs from bar to bar as a chance part does, within the call's gaps
    for bar in steps(&c, Deck::B, "clap") {
        assert!(bar.iter().all(|s| *s == 4 || *s == 12), "{bar:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// the property: the call and its answer are disjoint, and together they are the call's grid

#[test]
fn the_call_and_its_answer_are_disjoint_and_cover_the_calls_steps() {
    // (a step of the call, whether it sounds)
    let terms: [(&str, bool); 8] = [("x", true), ("~", false), ("[x x]", true), ("[~ x]", true), ("[x ~]", true), ("x*2", true), ("[~ ~]", false), ("c4", true)];
    let mut rng = Rng(0x5eed_6599_0000_0001);
    for n in [2usize, 4, 8, 16] {
        for round in 0..60 {
            // (on sixteen steps a step has no room inside it: a group or a repeat would land on the next one)
            let room = if n == 16 { &[terms[0], terms[1], terms[7]][..] } else { &terms[..] };
            let picked: Vec<(&str, bool)> = (0..n).map(|_| room[rng.below(room.len() as u64) as usize]).collect();
            let call = picked.iter().map(|t| t.0).collect::<Vec<_>>().join(" ");
            let src = format!("deck A: melody [{call}]\ndeck B: clap rests([{call}])\n");
            let c = ok(&src);
            let (a, b) = (&steps(&c, Deck::A, "melody")[0], &steps(&c, Deck::B, "clap")[0]);
            let width = (16 / n) as u8;
            for (i, (_, sounds)) in picked.iter().enumerate() {
                let (lo, hi) = (i as u8 * width, (i as u8 + 1) * width);
                let in_a = a.iter().any(|s| (lo..hi).contains(s));
                let in_b = b.iter().any(|s| (lo..hi).contains(s));
                assert_eq!(in_a, *sounds, "n={n} round={round} step {i} of [{call}]: the call plays {a:?}");
                assert_ne!(in_a, in_b, "n={n} round={round} step {i} of [{call}]: the call plays {a:?}, the answer {b:?}");
            }
            // a hit of the answer is on the grid of the call, and only one in a step
            assert!(b.iter().all(|s| s % width == 0), "n={n} [{call}]: {b:?}");
            assert_eq!(b.len(), picked.iter().filter(|t| !t.1).count(), "[{call}]");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// read as this part

#[test]
fn a_numeral_in_a_drum_call_is_read_for_its_onset() {
    // the call is read for onsets only: a chord's numeral in a drum part is an onset, not an error
    let c = ok("key am\ndeck A: snare rests([I ~ V ~])\n");
    assert_eq!(steps(&c, Deck::A, "snare"), one_bar(&[4, 12]));
    // and a tonal part's own numerals do what they do there
    let c = ok("key am\ndeck A: harmony rests([x ~ x ~], [I V vi IV])\n");
    assert_eq!(steps(&c, Deck::A, "harmony"), one_bar(&[4, 12]));
}

#[test]
fn a_call_does_not_move_the_voice_leading_of_its_part() {
    // `rests(p, q)` reads p's chords for their onsets only: the chords of q are led from where the part was, as if p were not there
    // (the chords of the call, I and ii, would have moved where iii is voiced from: this pair told the two apart when they were read in place)
    let with = ok("key am\ndeck A: harmony [I ~ rests([~ I ~ ii], [iii ~ I ~]) ~] voice-lead\n");
    let without = ok("key am\ndeck A: harmony [I ~ [iii ~ I ~] ~] voice-lead\n");
    assert_eq!(notes(&with, Deck::A, "harmony"), notes(&without, Deck::A, "harmony"));
}

#[test]
fn a_hit_in_a_tonal_part_is_its_root() {
    let c = ok("key dm\ndeck B: bass rests([x ~ x ~])\n");
    assert_eq!(notes(&c, Deck::B, "bass"), vec![vec![(4, 38), (12, 38)]]);
}

// ---------------------------------------------------------------------------------------------------------------
// what check says

#[test]
fn the_wrong_number_of_arguments_is_said_at_the_call() {
    let src = "deck B: clap rests()\n";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].message, "rests() takes 1 to 2, and this has 0");
    assert_eq!(e[0].help.as_deref(), Some("rests(call) or rests(call, reply): the call, and the pattern kept where it rests"));
    assert_eq!(e[0].position(src), (1, 14, 21));
    assert_eq!(errors("deck B: clap rests([x], [x], [x])\n")[0].message, "rests() takes 1 to 2, and this has 3");
    // a name=value is not a pattern
    let e = errors("deck B: clap rests([x ~ x ~], seed=3)\n");
    assert_eq!(e[0].message, "rests() takes patterns, not `name=value`");
}

#[test]
fn chance_in_the_call_is_an_error_at_the_call_with_the_way_out() {
    let src = "let call = x*8?0.5\ndeck A: snare call\ndeck B: clap rests(call)\n";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].message, "`rests(call)` answers a fixed rhythm, and `call` has chance in it (`x*8?0.5`)");
    assert_eq!(e[0].position(src), (3, 20, 24), "at the argument, where it was typed");
    let help = e[0].help.as_deref().unwrap();
    assert!(help.contains("take the chance out of the call") && help.contains("`snare maybe(call, 0.8)`"), "{help}");
    // every way of rolling dice, written in the call or reached through a let
    for dice in ["[x ~ x x?0.5]", "[x ~ choose [x x] weights 1 1 ~]", "maybe([x x ~ x], 0.5)", "degrade([x x x x], 0.3)", "markov(0.5)", "[x ~ <x x?0.9> ~]", "reverse(maybe([x x], 0.5))", "[x ~ [x x?]]", "every(2, [x?0.3 x])"] {
        let e = errors(&format!("deck B: clap rests({dice})\n"));
        assert!(e.iter().any(|d| d.message.contains("has chance in it")), "{dice}: {e:?}");
    }
    // a call that only moves velocities is fixed
    ok("deck B: clap rests(humanize([x ~ x ~], 7))\n");
    // and a chance in an arrange mutation's let is rewritten text before it is read, so it rolls before the call is read
    ok("let call = x ~ x ~\nsection verse 4 bars {\n  deck A: kick call\n  deck B: clap rests(call)\n}\narrange {\n  verse\n  ~ call thin 0.4\n  verse\n}\n");
}

#[test]
fn an_unknown_name_keeps_the_message_it_has() {
    let e = errors("deck B: clap rests(hist)\n");
    assert!(e[0].message.contains("`hist` is not a step, a note or a scale degree"), "{e:?}");
    // a near miss of the call's own name is told
    let e = errors("deck B: clap rest([x ~ x ~])\n");
    assert_eq!(e[0].help.as_deref(), Some("did you mean `rests`?"), "{e:?}");
}

#[test]
fn a_call_that_sounds_on_every_step_answers_nothing_and_says_so() {
    let src = "let call = x x x x\ndeck B: clap rests(call)\n";
    let w = warnings(src);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].message, "`rests(call)` plays nothing: `call` sounds on every step of its bar");
    assert_eq!(w[0].position(src), (2, 20, 24));
    assert_eq!(w[0].help.as_deref(), Some("leave a rest in the call, or answer a different pattern"));
    // with a reply, as well
    assert_eq!(warnings("deck B: clap rests([x x x x], [x ~ x ~])\n").len(), 1);
    // a call that rests in any bar of its round is no warning: that bar answers
    assert!(warnings("deck B: clap rests([<[x x x x] ~>])\n").is_empty());
    assert!(warnings("deck B: clap rests([x ~ x ~])\n").is_empty());
}

#[test]
fn steps_that_do_not_divide_are_said() {
    // three steps stretched over two bars are not whole steps in a bar
    let src = "deck B: clap rests(slow([x ~ x], 2))\n";
    let e = errors(src);
    assert!(e[0].message.contains("cannot tell the steps of the call") && e[0].message.contains("3 steps over 2 bars"), "{e:?}");
    assert_eq!(e[0].position(src).0, 1);
}

// ---------------------------------------------------------------------------------------------------------------
// it is a call like the others: the let, the sugar, the canonical text

#[test]
fn rests_is_still_a_name_a_let_may_take() {
    // `rests` is not a generator or a transform, so the word is free; the program is the one it was (the CID is pinned)
    let src = "let rests = ~ x ~ x\ndeck A: clap rests\n";
    let c = ok(src);
    assert_eq!(c.cid, "z4EBG9j8U5HLfeTE9MRbEno1B6u6tFnp5gMwwGHPHXtLKANhkgu");
    assert_eq!(steps(&c, Deck::A, "clap"), one_bar(&[4, 12]));
    assert_eq!(djlive::parse::reserved("rests"), None);
    // the call and the name side by side: before a `(` it is the call
    let c = ok("let rests = ~ x ~ x\ndeck A: clap rests\ndeck B: hihat rests(rests)\n");
    assert_eq!(steps(&c, Deck::A, "clap"), one_bar(&[4, 12]));
    assert_eq!(steps(&c, Deck::B, "hihat"), one_bar(&[0, 8]));
    // a `let` of one value is put in place of its name wherever the name stands, and the call before a `(` is left alone
    let c = ok("let rests = 4\ndeck A: kick x*rests | clap rests([x ~ x ~])\n");
    assert_eq!(steps(&c, Deck::A, "kick"), one_bar(&[0, 4, 8, 12]));
    assert_eq!(steps(&c, Deck::A, "clap"), one_bar(&[4, 12]));
}

#[test]
fn the_canonical_text_keeps_the_call_as_typed() {
    let c = ok("let call = x ~ ~ x\ndeck A: kick call\ndeck B: clap rests( call ,[~ x ~ x]) | hihat rests(call)\n");
    assert!(c.text.contains("clap rests(call, [~ x ~ x]) | hihat rests(call)"), "{}", c.text);
    // the answer of an answer, a transform of it, and the sugar inside it
    let c = ok("deck B: clap rotate(rests(rests([x ~ x ~], [x*4])), 1)\n");
    assert!(c.text.contains("rotate(rests(rests([x ~ x ~], [x*4])), 1)"), "{}", c.text);
    assert_eq!(clap("rests(ply([x ~ x ~], 2))"), clap("rests([x*2 ~*2 x*2 ~*2])"));
    assert_eq!(clap("rests(palindrome([x ~ ~ x]))"), clap("rests([<[x ~ ~ x] reverse([x ~ ~ x])>])"));
}

#[test]
fn a_transform_of_an_answer_moves_its_steps() {
    // rotate counts the steps of the answer's call, as it does for the other transforms: 1 4 7 of eight, answered, then rotated by one
    assert_eq!(clap("rotate(rests([x ~ ~ x ~ ~ x ~]), 1)"), one_bar(&[0, 4, 6, 10, 12]));
}

#[test]
fn a_part_with_a_cycle_of_its_own_answers_in_that_cycle() {
    // the call is read in the part's own cycle: twelve sixteenths, three steps of four
    let c = ok("deck B: clap/12 rests([x ~ x])\n");
    assert_eq!(steps(&c, Deck::B, "clap")[0], vec![4]);
}

#[test]
fn a_rhythm_port_carries_a_call_into_an_answer() {
    // the call goes over a wire as a rhythm, and `rests` is among the calls that keep a pattern a rhythm
    let src = "tempo 122\nkey am\n\ndef call() out: rhythm {\n  emit x ~ ~ x ~ ~ x ~\n}\ndef answer() in: rhythm {\n  deck B: clap rests($in, [~ x ~ x ~ x ~ x])\n}\nwire call.out -> answer.in\n";
    let c = check(src);
    assert!(c.compiled.is_some(), "{}", djlive::diag::render_all(&c.diagnostics, src));
}

#[test]
fn an_arrange_mutation_of_the_call_answers_again() {
    // the let is rewritten between sections, and the answer is read from the new text
    let src = "let lead = d5 ~ f5 ~ a5 ~ ~ f5\nlet reply = ~ d2 ~ d2 ~ a1 c2 ~\nsection call 4 bars {\n  deck B: melody lead | bass rests(lead, reply)\n}\narrange {\n  call\n  ~ lead rot 2\n  call\n}\n";
    let c = ok(src);
    let bass = |i: usize| c.segments[i].decks[&Deck::B].parts.iter().find(|p| p.role == "bass").unwrap().cycles[0].iter().map(|h| h.step).collect::<Vec<_>>();
    assert_eq!(c.segments.len(), 2);
    // the lead has hits on 1 3 5 8 of eight and the bass keeps all four of its notes; rotated two steps the lead holds 2 3 5 7, so the bass keeps two
    assert_eq!(bass(0), vec![2, 6, 10, 12]);
    assert_eq!(bass(1), vec![6, 10]);
}

#[test]
fn an_answer_is_stable_under_reformatting() {
    let src = "tempo 122\nkey am\n\nlet call = x ~ ~ x ~ ~ x ~\ndeck A: kick call\ndeck B: clap rests(call, [~ x ~ x ~ x ~ x]) | hihat rests(call) vel=50..75\n";
    let c = ok(src);
    let again = ok(&c.text);
    assert_eq!(again.text, c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(0x5eed_6599_0000_0002);
    for round in 0..40 {
        let text = reformat(src, &mut rng);
        assert_eq!(ok(&text).cid, c.cid, "round {round}:\n{text}");
    }
}

#[test]
fn the_answers_of_two_parts_of_one_deck_each_read_their_own_part() {
    // the same call, answered by a drum and by a tonal part of one deck
    let c = ok("let call = x ~ x ~\ndeck B: clap rests(call) | bass rests(call, [c2 c2 c2 c2])\n");
    assert_eq!(steps(&c, Deck::B, "clap"), one_bar(&[4, 12]));
    assert_eq!(steps(&c, Deck::B, "bass"), one_bar(&[4, 12]));
}

#[test]
fn no_argument_makes_rests_panic() {
    // numbers past anything a count could be, empty and nested arguments, a call inside a call, and the other transforms' own counts
    let frags = [
        "", "x", "~", "[x ~ x ~]", "[<x ~> [x x]]", "[x*64]", "e(3,8)", "e(0,1)", "e(9,8)", "e(3,99999999999)", "arp(am, up, 99999999999)", "9223372036854775808", "99999999999999999999", "0", "-1", "0.5",
        "every(9223372036854775808, [x x], [x])", "every(0, [x])", "slow([x ~ x], 9223372036854775808)", "slow([x ~ x], 0)", "fast([x x], 99999999999)", "fast([x x], 0)", "rests([x], [x])", "rests()", "rests(rests(rests([x ~])))", "reverse([x ~ x])",
        "degrade([x x], 0.5)", "[x?0.5 ~]", "[x*64]*64", "[[x*64]*64]*64", "deg(1 3 5)", "[I V]", "chord(am)", "hist", "seed=3", "[~ ~ ~]",
    ];
    let mut rng = Rng(0x5eed_6599_0000_0003);
    for round in 0..400 {
        let (a, b, c) = (frags[rng.below(frags.len() as u64) as usize], frags[rng.below(frags.len() as u64) as usize], frags[rng.below(frags.len() as u64) as usize]);
        let call = match rng.below(3) {
            0 => format!("rests({a})"),
            1 => format!("rests({a}, {b})"),
            _ => format!("rests({a}, {b}, {c})"),
        };
        let part = ["clap", "bass", "harmony"][rng.below(3) as usize];
        let src = format!("key am\ndeck B: {part} {call}\n");
        let result = std::panic::catch_unwind(|| check(&src));
        assert!(result.is_ok(), "round {round}: check panicked on {src:?}");
    }
}
