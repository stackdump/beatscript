//! Call and response (djlive/src/answer.rs, docs/BTW.md "Call and response") is proved by `same_as`: `rests(p)` and
//! `rests(p, q)` compile to exactly the nets of the longhand a person would write for the same answer, so the answer is
//! the call's gaps, no more and no less. The hits themselves are looked at in djlive/tests/answer.rs.

use djengine::json::JsonValue;

fn program(parts: &str) -> String {
    format!("tempo 124\nkey am\n{parts}\n")
}

fn same(a: &str, b: &str) -> JsonValue {
    JsonValue::parse(&btwapi::same_as_json(a, b)).expect("same_as answers JSON")
}

/// Two whole programs are the same song by their nets, and are not the same text.
fn proves_text(a: &str, b: &str) {
    let r = same(a, b);
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "both compile:\n{a}\n{b}\n{}", r.js_stringified());
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(true), "same nets:\n{a}\n{b}\n{}", r.js_stringified());
    assert_eq!(r.get("text_same").and_then(JsonValue::as_bool), Some(false), "two texts:\n{a}\n{b}");
}

/// Two lines (parts of a program) are the same song by their nets.
fn proves(a: &str, b: &str) {
    proves_text(&program(a), &program(b));
}

fn differs(a: &str, b: &str) {
    let r = same(&program(a), &program(b));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(false), "{a} / {b}: {}", r.js_stringified());
}

#[test]
fn the_answer_is_the_hand_written_complement() {
    // eight steps: x ~ ~ x ~ ~ x ~ rests on 2 3 5 6 8
    proves("deck B: clap rests([x ~ ~ x ~ ~ x ~])", "deck B: clap ~ x x ~ x x ~ x");
    // four steps and sixteen steps are their own, not the sixteenths: x ~ x ~ answers with two hits
    proves("deck B: clap rests([x ~ x ~])", "deck B: clap ~ x ~ x");
    proves("deck B: clap rests([x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~])", "deck B: clap ~ x x x ~ x x x ~ x x x ~ x x x");
    // a step with something starting in it sounds, wherever in the step
    proves("deck B: clap rests([[x x] ~ [~ x] x*2 ~ [x ~] [~ ~] ~])", "deck B: clap ~ x ~ ~ x ~ x x");
    // a Euclidean rhythm is its steps; a transform of a call is read for its own steps
    proves("deck B: clap rests(e(3,8))", "deck B: clap ~ x x ~ x x ~ x");
    proves("deck B: clap rests(reverse([x ~ ~ x ~ ~ x ~]))", "deck B: clap x ~ x x ~ x x ~");
    proves("deck B: clap rests([x ~ ~ ~]*2)", "deck B: clap ~ x x x ~ x x x");
    // a call that is silent a bar in two: that bar is answered whole, on one step
    proves("deck B: clap rests([<[x ~ ~ x] ~>])", "deck B: clap <[~ x x ~] x>");
    // a lone group and the same call written out
    proves("deck B: clap rests([[x ~ x ~]])", "deck B: clap rests([x ~ x ~])");
    // the answer of the answer is the call's own steps again
    proves("deck B: clap rests(rests([x ~ ~ x ~ ~ x ~]))", "deck B: clap x ~ ~ x ~ ~ x ~");
}

#[test]
fn a_reply_is_the_longhand_kept_where_the_call_rests() {
    proves("deck B: clap rests([x ~ ~ x ~ ~ x ~], [~ x ~ x ~ x ~ x])", "deck B: clap ~ x ~ ~ ~ x ~ x");
    // notes keep their pitch and their step
    proves("deck B: bass rests([x ~ x ~], [c2 d2 e2 f2])", "deck B: bass ~ d2 ~ f2");
    // a reply on a finer grid is cut at the call's steps
    proves("deck B: clap rests([x ~ x ~], [x*16])", "deck B: clap ~ ~ ~ ~ x x x x ~ ~ ~ ~ x x x x");
    // and a reply of its own period meets the call's
    proves("deck B: clap rests([x ~ x ~], [<[x x x x] ~>])", "deck B: clap <[~ x ~ x] [~ ~ ~ ~]>");
    // a transform of an answer, and an answer given to a part that plays several things
    proves("deck B: clap rotate(rests([x ~ ~ x ~ ~ x ~]), 1)", "deck B: clap rotate([~ x x ~ x x ~ x], 1)");
    proves("deck B: clap rests([x ~ x ~]) | hihat rests([x ~ x ~], [x*8])", "deck B: clap ~ x ~ x | hihat ~ ~ x x ~ ~ x x");
}

#[test]
fn what_is_answered_is_the_text_of_the_call() {
    differs("deck B: clap rests([x ~ x ~])", "deck B: clap rests([x ~ ~ x])");
    // (the answer is not the call turned round, though it is on a call that is its own mirror: x ~ x ~ answers ~ x ~ x)
    differs("deck B: clap rests([x ~ ~ x])", "deck B: clap reverse([x ~ ~ x])");
    // the call a let names, given to both decks: the answer follows the let
    let a = "let call = x ~ ~ x ~ ~ x ~\ndeck A: kick call\ndeck B: clap rests(call)\n";
    let b = "let call = x ~ x ~ ~ x ~ ~\ndeck A: kick call\ndeck B: clap rests(call)\n";
    let r = same(&program(a), &program(b));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(false), "{}", r.js_stringified());
}

#[test]
fn a_kick_calls_and_a_clap_and_a_hat_answer() {
    let call = "tempo 122\nkey am\n\nlet call = x ~ ~ x ~ ~ x ~\ndeck A: kick call\ndeck B: clap rests(call, [~ x ~ x ~ x ~ x]) | hihat rests(call) vel=50..75\n";
    let long = "tempo 122\nkey am\n\nlet call = x ~ ~ x ~ ~ x ~\ndeck A: kick call\ndeck B: clap ~ x ~ ~ ~ x ~ x | hihat ~ x x ~ x x ~ x vel=50..75\n";
    proves_text(call, long);
    // and it is the program `examples/btw/answer.btw` carries
    let file = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw/answer.btw")).expect("examples/btw/answer.btw");
    let r = same(&file, &file.replace("clap rests(call, [~ x ~ x ~ x ~ x])", "clap ~ x ~ ~ ~ x ~ x").replace("hihat rests(call)", "hihat ~ x x ~ x x ~ x"));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(true), "{}", r.js_stringified());
}

#[test]
fn a_sung_line_is_answered_by_a_deck() {
    let call = "tempo 88\nkey am\n\nlet line = <[a4 ~ c5 e5 ~ ~ d5 ~] ~>\ndeck A: harmony rests(line, [chord(am) ~ chord(f) ~ chord(c) ~ chord(g) ~])\ndeck B: melody line\nsection verse 8 bars {\n  deck B: sing \"hush now lan-tern\" on melody voice=robot-choir\n}\narrange verse\n";
    let long = "tempo 88\nkey am\n\nlet line = <[a4 ~ c5 e5 ~ ~ d5 ~] ~>\ndeck A: harmony <[~ ~ ~ ~ chord(c) ~ ~ ~] [chord(am) ~ chord(f) ~ chord(c) ~ chord(g) ~]>\ndeck B: melody line\nsection verse 8 bars {\n  deck B: sing \"hush now lan-tern\" on melody voice=robot-choir\n}\narrange verse\n";
    proves_text(call, long);
}

#[test]
fn a_bass_answers_a_lead_across_sections() {
    let head = "tempo 126\nkey dm\n\nlet lead  = d5 ~ f5 ~ a5 ~ ~ f5\nlet reply = ~ d2 ~ d2 ~ a1 c2 ~\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n\n";
    let call = format!("{head}section call 4 bars {{\n  deck B: melody lead inst=pwm-lead | bass rests(lead, reply)\n}}\narrange {{\n  call\n  ~ lead rot 2\n  call\n  ~ lead thin 0.4\n  call\n}}\n");
    // each pass written out: the lead as the mutations leave it, the bass as the lead's gaps leave it
    let long = "tempo 126\nkey dm\n\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n\nsection call 4 bars {\n  deck B: melody d5 ~ f5 ~ a5 ~ ~ f5 inst=pwm-lead | bass ~ d2 ~ d2 ~ a1 c2 ~\n}\nsection call2 4 bars {\n  deck B: melody ~ f5 d5 ~ f5 ~ a5 ~ inst=pwm-lead | bass ~ ~ ~ d2 ~ a1 ~ ~\n}\nsection call3 4 bars {\n  deck B: melody ~ f5 d5 ~ ~ ~ ~ ~ inst=pwm-lead | bass ~ ~ ~ d2 ~ a1 c2 ~\n}\narrange call call2 call3\n";
    proves_text(&call, long);
}
