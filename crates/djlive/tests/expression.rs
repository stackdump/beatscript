//! The expression marks (docs/BTW.md, "Expression marks"): a ghost `x.`, a flam `x&`, a ratchet `x:N`, and the tonal
//! params `glide=`, `decay=`, `pitch-drop=`. How they read and print, what they do to the hits and the net's bindings,
//! the errors and the warning, and that a program without them compiles to exactly what it did.

use djengine::json::JsonValue;
use djlive::ast::Deck;
use djlive::compile::check;
use djlive::print::canonical;

fn compiled(src: &str) -> djlive::compile::Compiled {
    let c = check(src);
    assert!(c.ok(), "{src:?}: {:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    c.compiled.unwrap()
}

fn canon(src: &str) -> String {
    compiled(src).text
}

fn one_error(src: &str) -> String {
    let c = check(src);
    let e: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert_eq!(e.len(), 1, "{src:?}: {e:?}");
    e[0].clone()
}

/// The MIDI bindings of a deck's net at phase 0, by role then step: (step, binding).
fn bindings(src: &str, deck: Deck, role: &str) -> Vec<(usize, JsonValue)> {
    let c = compiled(src);
    let env = c.decks[&deck].envelope(0);
    let net = env.get("nets").and_then(|n| n.get(role)).unwrap_or_else(|| panic!("no net {role} in {}", env.js_stringified()));
    let ts = net.get("transitions").and_then(JsonValue::as_object).unwrap();
    let mut out: Vec<(usize, JsonValue)> = ts.iter().filter_map(|(id, t)| t.get("midi").map(|m| (id[1..].parse().unwrap(), m.clone()))).collect();
    out.sort_by_key(|(s, _)| *s);
    out
}

fn int(m: &JsonValue, k: &str) -> Option<i64> {
    m.get(k).and_then(JsonValue::as_i64)
}

#[test]
fn the_marks_read_in_any_order_and_print_in_one() {
    assert_eq!(canon("deck A: hihat x x. x& x:3 x.&:4 c2^\n"), "deck A: hihat x x. x& x:3 x.&:4 c2^\n");
    assert_eq!(canon("deck A: hihat x:3&. x^! [x x]&\n"), "deck A: hihat x.&:3 x!^ [x x]&\n");
    assert_eq!(canon("deck A: hihat x:3*2 x.?0.5 ~ x\n"), "deck A: hihat x:3*2 x.?0.5 ~ x\n");
    assert_eq!(canon("key am\ndeck A: melody deg(1. 3& 5:2 1')\n"), "key am\ndeck A: melody deg(1. 3& 5:2 1')\n");
    // (a part's params print in name order)
    let t = "deck B: bass f1 ~ f1^ f2 decay=900 glide=70 pitch-drop=7\n";
    assert_eq!(canon(t), t);
    assert_eq!(canon("deck B: bass f1 ~ f1^ f2 pitch-drop=7 glide=70 decay=900\n"), t);
    // the canonical text is a fixed point
    for src in ["deck A: hihat x.&:4 x!^:2\n", t] {
        let once = canon(src);
        assert_eq!(canon(&once), once);
    }
    // a `.` that is not a ghost is what it was: a deck's part, a range
    assert!(check("deck A: kick x x | hihat x*8 vel=60..90\n").ok());
}

#[test]
fn a_ghost_is_forty_percent_of_its_velocity_and_a_flam_and_a_ratchet_ride_on_the_binding() {
    let b = bindings("deck A: hihat x x. x& x:3 vel=100\n", Deck::A, "hihat");
    assert_eq!(b.len(), 4);
    assert_eq!(b.iter().map(|(s, m)| (*s, int(m, "velocity").unwrap())).collect::<Vec<_>>(), [(0, 100), (4, 40), (8, 100), (12, 100)]);
    assert!(b[0].1.get("flam").is_none() && b[0].1.get("ratchet").is_none());
    assert_eq!(b[2].1.get("flam").and_then(JsonValue::as_bool), Some(true));
    assert_eq!(int(&b[3].1, "ratchet"), Some(3));
    // a ghost after an accent on its group, and on a range's velocity
    let b = bindings("deck A: snare x. ~ ~ ~ vel=80\n", Deck::A, "snare");
    assert_eq!(int(&b[0].1, "velocity"), Some(32));
    // a mark on a group marks every hit in it
    let b = bindings("deck A: hihat [x x]& ~\n", Deck::A, "hihat");
    assert!(b.iter().all(|(_, m)| m.get("flam").is_some()), "{b:?}");
}

#[test]
fn glide_bends_a_slide_into_the_next_note_and_without_it_a_slide_only_holds() {
    let src = "deck B: bass f1 ~ ~ ~ f1^ ~ ~ ~ f2 ~ ~ ~ ~ ~ ~ ~ glide=70\n";
    let b = bindings(src, Deck::B, "bass");
    assert_eq!(b.iter().map(|(s, m)| (*s, int(m, "glide"))).collect::<Vec<_>>(), [(0, None), (4, None), (8, Some(70))]);
    // the slide still holds its note to the next one
    assert_eq!(int(&b[1].1, "durationSteps"), Some(5));
    let plain = bindings("deck B: bass f1 ~ ~ ~ f1^ ~ ~ ~ f2 ~ ~ ~ ~ ~ ~ ~\n", Deck::B, "bass");
    assert!(plain.iter().all(|(_, m)| m.get("glide").is_none()));
    assert_eq!(int(&plain[1].1, "durationSteps"), Some(5));
    // the last step's slide glides into the first, round the loop
    let b = bindings("deck B: bass f1 ~ f2 eb2^ glide=40\n", Deck::B, "bass");
    assert_eq!(b.iter().map(|(_, m)| int(m, "glide")).collect::<Vec<_>>(), [Some(40), None, None]);
}

#[test]
fn decay_and_pitch_drop_are_on_every_note_of_the_part() {
    let b = bindings("deck B: bass f1 ~ ab1 ~ decay=900 pitch-drop=7\n", Deck::B, "bass");
    assert_eq!(b.len(), 2);
    for (_, m) in &b {
        assert_eq!((int(m, "decay"), int(m, "pitchDrop")), (Some(900), Some(7)));
    }
}

#[test]
fn a_program_without_the_marks_has_the_bindings_it_always_had() {
    for (src, role, deck) in [("deck A: hihat x x! x^ x vel=90\n", "hihat", Deck::A), ("deck B: bass f1 ~ f1^ f2 len=3\n", "bass", Deck::B)] {
        for (_, m) in bindings(src, deck, role) {
            let keys: Vec<&str> = m.as_object().unwrap().keys().collect();
            assert_eq!(keys, ["note", "channel", "velocity", "durationSteps"], "{src:?}");
        }
    }
}

#[test]
fn the_errors_say_where_and_why() {
    assert!(one_error("deck A: hihat x:9\n").contains("2 to 8"));
    assert!(one_error("deck A: hihat x:1\n").contains("2 to 8"));
    assert!(one_error("deck A: hihat x!.\n").contains("not both"));
    assert!(one_error("deck A: hihat x&&\n").contains("already on this step"));
    assert!(one_error("deck A: kick x x glide=50\n").contains("tonal part"));
    assert!(one_error("deck A: snare x x decay=200\n").contains("tonal part"));
    assert!(one_error("deck B: bass f1 f2 glide=5000\n").contains("1 to 2000"));
    assert!(one_error("deck B: bass f1 f2 pitch-drop=30\n").contains("1 to 24"));
    assert!(one_error("deck B: bass f1 f2 decay=100..200\n").contains("one number"));
}

#[test]
fn glide_with_no_slide_is_worth_a_warning() {
    let c = check("deck B: bass f1 ~ f2 ~ glide=60\n");
    assert!(c.ok());
    assert!(c.diagnostics.iter().any(|d| !d.is_error() && d.message.contains("no slide")), "{:?}", c.diagnostics);
    let c = check("deck B: bass f1^ ~ f2 ~ glide=60\n");
    assert!(!c.diagnostics.iter().any(|d| d.message.contains("no slide")));
}

#[test]
fn the_parser_reads_the_canonical_text_back() {
    // a def body is printed from its tokens: a ratchet stays glued
    let src = "def roll(n) {\n  deck A: hihat x x:3 x. x& vel=$n\n}\nroll(90)\n";
    let c = compiled(src);
    assert_eq!(canonical(&djlive::parse::parse(&c.text).program), c.text);
    assert!(c.text.contains("x:3 x. x&"), "{}", c.text);
}
