//! `same_as`'s `diff`: what changed between two programs, in music. Each test edits a small song with one of the
//! language's own transforms (`transpose(p, n)`, `rotate(p, n)`, ...) or one structural change, and checks that the
//! diff names it: the transform and its amount, the section, and a one-line summary.

use djengine::json::JsonValue;

const SONG: &str = "tempo 124
key am
let bl = a2 ~ ~ a2 ~ ~ c3 ~ e2 ~ ~ e2 ~ ~ g2 ~
let hh = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
let kk = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
section intro 4 bars {
  deck A: kick kk | hihat hh
}
section drop 8 bars {
  deck A: kick kk | hihat hh | bass bl inst=sub-bass
}
section outro 4 bars {
  deck A: kick kk
}
arrange intro drop outro
";

const DROP: &str = "  deck A: kick kk | hihat hh | bass bl inst=sub-bass";

fn report(a: &str, b: &str) -> JsonValue {
    let r = JsonValue::parse(&beatscript_api::same_as_json(a, b)).expect("JSON");
    assert_eq!(
        r.get("ok").and_then(JsonValue::as_bool),
        Some(true),
        "{}",
        r.js_stringified()
    );
    r
}

/// The diff of SONG against `b` (it must not be the same song).
fn diff(b: &str) -> JsonValue {
    let r = report(SONG, b);
    assert_eq!(
        r.get("same").and_then(JsonValue::as_bool),
        Some(false),
        "{}",
        r.js_stringified()
    );
    r.get("diff").expect("a diff").clone()
}

fn arr<'a>(v: &'a JsonValue, k: &str) -> &'a Vec<JsonValue> {
    v.get(k)
        .and_then(JsonValue::as_array)
        .unwrap_or_else(|| panic!("{k} in {}", v.js_stringified()))
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{k} in {}", v.js_stringified()))
}

fn summary(d: &JsonValue) -> Vec<String> {
    arr(d, "summary")
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

/// The one part change, with its transform's kind and amount.
fn only_part(d: &JsonValue) -> &JsonValue {
    let p = arr(d, "parts");
    assert_eq!(p.len(), 1, "{}", d.js_stringified());
    &p[0]
}

fn transform_of(p: &JsonValue) -> (&str, i64) {
    let t = p
        .get("transform")
        .unwrap_or_else(|| panic!("no transform in {}", p.js_stringified()));
    (
        s(t, "kind"),
        t.get("by").and_then(JsonValue::as_i64).unwrap(),
    )
}

/// SONG with the drop's deck A line replaced.
fn drop_as(line: &str) -> String {
    let b = SONG.replace(DROP, line);
    assert_ne!(b, SONG);
    b
}

#[test]
fn the_same_song_has_no_diff() {
    let r = report(SONG, &SONG.replace("let kk", "# the kick\nlet kk"));
    assert!(
        matches!(r.get("diff"), Some(JsonValue::Null)),
        "{}",
        r.js_stringified()
    );
}

#[test]
fn a_transposed_section() {
    let d = diff(&drop_as(
        "  deck A: kick kk | hihat hh | bass transpose(bl, 2) inst=sub-bass",
    ));
    let p = only_part(&d);
    assert_eq!(
        (s(p, "b_section"), s(p, "deck"), s(p, "role")),
        ("drop", "A", "bass")
    );
    assert_eq!(transform_of(p), ("transpose", 2));
    assert_eq!(s(p, "summary"), "drop bass: transposed +2");
    assert!(
        summary(&d).contains(&"drop changed (1 part)".to_string()),
        "{:?}",
        summary(&d)
    );
    // an octave is its own word, and down is negative
    let p2 = diff(&drop_as(
        "  deck A: kick kk | hihat hh | bass octave(bl, -1) inst=sub-bass",
    ));
    assert_eq!(transform_of(only_part(&p2)), ("octave", -1));
}

#[test]
fn an_inserted_section_that_is_a_transposed_copy() {
    // drop2: the drop again, its bass a semitone up: inserted, and described against the drop
    let b = SONG.replace(
        "arrange intro drop outro",
        "section drop2 8 bars {\n  deck A: kick kk | hihat hh | bass transpose(bl, 1) inst=sub-bass\n}\narrange intro drop drop2 outro",
    );
    let d = diff(&b);
    let a = arr(&d, "arrangement");
    assert_eq!(a.len(), 1, "{}", d.js_stringified());
    assert_eq!(s(&a[0], "change"), "inserted");
    assert_eq!(s(a[0].get("b").unwrap(), "name"), "drop2");
    assert_eq!(s(a[0].get("like").unwrap(), "name"), "drop");
    let p = only_part(&d);
    assert_eq!(transform_of(p), ("transpose", 1));
    assert_eq!(s(p, "summary"), "drop2 bass = drop bass transposed +1");
    let sum = summary(&d);
    assert!(sum.contains(&"song_bars 16 -> 24".to_string()), "{sum:?}");
    assert!(
        sum.contains(&"inserted drop2 (8 bars) at bar 13, a variant of drop".to_string()),
        "{sum:?}"
    );
    assert!(
        !sum.iter().any(|l| l.starts_with("end_bar")),
        "end_bar repeats song_bars: {sum:?}"
    );
}

#[test]
fn an_inserted_section_with_nothing_in_common() {
    let b = SONG.replace("arrange intro drop outro", "section pad 2 bars {\n  deck B: melody c4 ~ ~ ~ e4 ~ ~ ~ g4 ~ ~ ~ ~ ~ ~ ~ inst=warm-pad\n}\narrange intro drop pad outro");
    let d = diff(&b);
    let a = arr(&d, "arrangement");
    assert_eq!((a.len(), s(&a[0], "change")), (1, "inserted"));
    assert!(a[0].get("like").is_none());
    assert!(arr(&d, "parts").is_empty());
    assert!(
        summary(&d).contains(&"inserted pad (2 bars) at bar 13".to_string()),
        "{:?}",
        summary(&d)
    );
}

#[test]
fn rotated_hats() {
    let d = diff(&drop_as(
        "  deck A: kick kk | hihat rotate(hh, 1) | bass bl inst=sub-bass",
    ));
    let p = only_part(&d);
    assert_eq!((s(p, "role"), transform_of(p)), ("hihat", ("rotate", 1)));
    assert_eq!(s(p, "summary"), "drop hihat: rotated +1 step");
    let back = diff(&drop_as(
        "  deck A: kick kk | hihat rotate(hh, -3) | bass bl inst=sub-bass",
    ));
    // the hats are a four-step pulse, so -3 is +1
    assert_eq!(transform_of(only_part(&back)), ("rotate", 1));
}

#[test]
fn reversed_and_slowed_and_thinned() {
    assert_eq!(
        transform_of(only_part(&diff(&drop_as(
            "  deck A: kick kk | hihat hh | bass reverse(bl) inst=sub-bass"
        )))),
        ("reverse", 0)
    );
    assert_eq!(
        transform_of(only_part(&diff(&drop_as(
            "  deck A: kick kk | hihat hh | bass slow(bl, 2) inst=sub-bass"
        )))),
        ("slow", 2)
    );
    assert_eq!(
        transform_of(only_part(&diff(&drop_as(
            "  deck A: kick kk | hihat hh | bass fast(bl, 2) inst=sub-bass"
        )))),
        ("fast", 2)
    );
    // half the kicks: thin, which for a pulse is also slow 2 (in `also`)
    let p = diff(&drop_as(
        "  deck A: kick x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ | hihat hh | bass bl inst=sub-bass",
    ));
    let p = only_part(&p);
    assert_eq!(transform_of(p), ("thin", 2));
    assert!(
        p.get("also").unwrap().js_stringified().contains("slow"),
        "{}",
        p.js_stringified()
    );
}

#[test]
fn a_moved_section() {
    // the outro played before the drop: the short section moved, the long ones stayed
    let d = diff(&SONG.replace("arrange intro drop outro", "arrange intro outro drop"));
    let a = arr(&d, "arrangement");
    assert_eq!(a.len(), 1, "{}", d.js_stringified());
    assert_eq!(s(&a[0], "change"), "moved");
    assert_eq!(s(a[0].get("b").unwrap(), "name"), "outro");
    assert_eq!(summary(&d), ["moved outro: bar 13 -> bar 5"]);
}

#[test]
fn a_repeated_and_a_removed_and_a_renamed_section() {
    let d = diff(&SONG.replace("arrange intro drop outro", "arrange intro drop drop outro"));
    let a = arr(&d, "arrangement");
    assert_eq!(
        (a.len(), s(&a[0], "change")),
        (1, "repeated"),
        "{}",
        d.js_stringified()
    );
    assert!(
        summary(&d).contains(&"repeated drop at bar 13".to_string()),
        "{:?}",
        summary(&d)
    );
    let d = diff(&SONG.replace("arrange intro drop outro", "arrange intro drop"));
    assert!(
        summary(&d).contains(&"removed outro (4 bars, was at bar 13)".to_string()),
        "{:?}",
        summary(&d)
    );
    // a renamed section alone is the same song (same_as says renamed); renamed and changed is a diff saying both
    let b = SONG
        .replace("section drop 8 bars", "section peak 8 bars")
        .replace("arrange intro drop outro", "arrange intro peak outro")
        .replace(
            DROP,
            "  deck A: kick kk | hihat hh | bass transpose(bl, 5) inst=sub-bass",
        );
    let d = diff(&b);
    let sum = summary(&d);
    assert!(
        sum.iter()
            .any(|l| l == "peak bass = drop bass transposed +5"),
        "{sum:?}"
    );
}

#[test]
fn a_tempo_change_is_a_song_change_only() {
    // the tempo is in every net too; the diff says it once, at the song level, and no section changed
    let d = diff(&SONG.replace("tempo 124", "tempo 126"));
    assert_eq!(summary(&d), ["tempo 124 -> 126"]);
    let song = arr(&d, "song");
    assert_eq!(
        (
            s(&song[0], "field"),
            song[0].get("old").unwrap().as_f64(),
            song[0].get("new").unwrap().as_f64()
        ),
        ("tempo", Some(124.0), Some(126.0))
    );
    assert!(arr(&d, "arrangement").is_empty() && arr(&d, "parts").is_empty());
    assert_eq!(
        d.get("unchanged_sections").and_then(JsonValue::as_i64),
        Some(3)
    );
}

#[test]
fn a_plain_note_edit() {
    // one note of the bass line, c3 -> d3 on step 7: no transform explains it, so a note diff at bar:step
    let d = diff(&SONG.replace("let bl = a2 ~ ~ a2 ~ ~ c3", "let bl = a2 ~ ~ a2 ~ ~ d3"));
    let p = only_part(&d);
    assert_eq!(s(p, "change"), "notes");
    let n = p.get("notes").unwrap();
    let ch = arr(n, "changed");
    assert_eq!(ch.len(), 1);
    assert_eq!(
        (s(&ch[0], "at"), s(&ch[0], "old"), s(&ch[0], "new")),
        ("1:7", "c3", "d3")
    );
    assert!(arr(n, "added").is_empty() && arr(n, "removed").is_empty());
    assert_eq!(s(p, "summary"), "drop bass: 1 note changed (1:7 c3 -> d3)");
    // a note taken out of a pitched line, and one added
    let d = diff(&SONG.replace(
        "let bl = a2 ~ ~ a2 ~ ~ c3 ~ e2 ~ ~ e2 ~ ~ g2 ~",
        "let bl = a2 ~ ~ ~ ~ ~ c3 ~ e2 ~ ~ e2 ~ b2 g2 ~",
    ));
    let n = only_part(&d).get("notes").unwrap();
    assert_eq!(
        (
            n.get("added_count").and_then(JsonValue::as_i64),
            n.get("removed_count").and_then(JsonValue::as_i64)
        ),
        (Some(1), Some(1))
    );
    assert_eq!(s(&arr(n, "removed")[0], "at"), "1:4");
    assert_eq!(s(&arr(n, "added")[0], "note"), "b2");
}

#[test]
fn trance_138_edits_name_their_sections() {
    let song = include_str!("../../../examples/trance-138.beatscript");
    // a melody note in the anthem: the breakdown and the drop both play it
    let b = song.replace(
        "let m3 = g5 ~ ~ ~ ~ ~ e5 ~ c5",
        "let m3 = g5 ~ ~ ~ ~ ~ f5 ~ c5",
    );
    let r = report(song, &b);
    let d = r.get("diff").unwrap();
    let parts = arr(d, "parts");
    let secs: Vec<&str> = parts.iter().map(|p| s(p, "b_section")).collect();
    assert_eq!(secs, ["breakdown", "drop"], "{}", d.js_stringified());
    assert!(parts
        .iter()
        .all(|p| s(p, "role") == "melody" && s(p, "change") == "notes"));
    // and against its longhand (no sections) the diff is of the whole song, deck by deck
    let long = include_str!("../../djlive/tests/fixtures/trance-138.live")
        .replace("inst=supersaw", "inst=detuned-saw");
    let long = format!(
        "{}end at bar 64\n",
        long.replace(
            "[e5 ~ ~ ~ ~ ~ d5 ~ c5 ~ ~ ~ ~ ~ ~ ~] [~ ~ ~ ~ ~ ~ ~ ~ a4",
            "[e5 ~ ~ ~ ~ ~ d5 ~ c5 ~ ~ ~ ~ ~ ~ ~] [~ ~ ~ ~ ~ ~ ~ ~ b4"
        )
    );
    let r = report(song, &long);
    let d = r.get("diff").unwrap();
    assert_eq!(s(d, "basis"), "song_nets");
    let p = only_part(d);
    assert_eq!(
        (s(p, "deck"), s(p, "role"), s(p, "change")),
        ("B", "melody", "notes")
    );
    assert!(matches!(p.get("b_section"), Some(JsonValue::Null)));
}

#[test]
fn a_jux_is_a_right_copy_added_and_a_changed_one_is_said_as_a_part_is() {
    // the drop's bass gets a right copy, a reversed one: the part plays what it played, and has a twin
    let jux = |op: &str| SONG.replace("arrange intro drop outro\n", &format!("arrange {{\n  intro\n  ~ bl jux {op}\n  drop\n  outro\n}}\n"));
    let d = diff(&jux("rev"));
    let p = only_part(&d);
    assert_eq!((s(p, "role"), s(p, "copy"), s(p, "change")), ("bass", "right", "added"));
    assert_eq!(s(p, "summary"), "drop bass: right copy added");
    // the same two programs with another right copy: the twin moved (rotated), the part did not
    let r = report(&jux("rev"), &jux("rot 2"));
    assert_eq!(r.get("same").and_then(JsonValue::as_bool), Some(false));
    let p = only_part(r.get("diff").unwrap());
    assert_eq!((s(p, "role"), s(p, "copy")), ("bass", "right"));
    assert!(s(p, "summary").starts_with("drop bass: right copy "), "{}", s(p, "summary"));
    // and a program is the same song as itself
    assert_eq!(report(&jux("rev"), &jux("rev")).get("same").and_then(JsonValue::as_bool), Some(true));
}
