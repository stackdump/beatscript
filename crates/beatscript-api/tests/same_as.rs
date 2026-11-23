//! `same_as`: two programs are the same song when their nets and song-level settings are, whatever their text.
//! trance-138 against its longhand, a `let` refactor, a renamed section, and a one-note change that names its section.

use djengine::json::JsonValue;

const SONG: &str = include_str!("../../../examples/trance-138.beatscript");
const LONGHAND: &str = include_str!("../../djlive/tests/fixtures/trance-138.live");

fn same(a: &str, b: &str) -> JsonValue {
    JsonValue::parse(&beatscript_api::same_as_json(a, b)).expect("same_as answers JSON")
}

fn b(v: &JsonValue, k: &str) -> bool {
    v.get(k)
        .and_then(JsonValue::as_bool)
        .unwrap_or_else(|| panic!("{k} is not a bool in {}", v.js_stringified()))
}

fn s<'a>(v: &'a JsonValue, k: &str) -> &'a str {
    v.get(k)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{k} is not a string in {}", v.js_stringified()))
}

fn arr<'a>(v: &'a JsonValue, k: &str) -> &'a Vec<JsonValue> {
    v.get(k)
        .and_then(JsonValue::as_array)
        .unwrap_or_else(|| panic!("{k} is not an array"))
}

/// The longhand with the voice the song names (`supersaw` is not an engine voice, see djlive/tests/trance.rs).
fn longhand() -> String {
    LONGHAND.replace("inst=supersaw", "inst=detuned-saw")
}

/// `text` with every use of `name` in another `let` written out in full (a `def`'s arguments are names, so
/// those uses stay), and the `let` itself kept only if something still names it.
fn inline_let(text: &str, name: &str) -> String {
    let prefix = format!("let {name} = ");
    // bracketed: inside `<...>` each term is a bar, and a bar of steps is `[...]` (as the longhand writes it)
    let value = format!(
        "[{}]",
        text.lines()
            .find_map(|l| l.strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("no let {name}"))
    );
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with(&prefix) || !line.starts_with("let ") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let words: Vec<String> = line
            .split(' ')
            .map(|w| {
                if w == name {
                    value.clone()
                } else {
                    replace_ident(w, name, &value)
                }
            })
            .collect();
        out.push_str(&words.join(" "));
        out.push('\n');
    }
    out
}

/// `name` as a whole identifier inside a word such as `deckA(kk,` or `<kk`.
fn replace_ident(w: &str, name: &str, value: &str) -> String {
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = w;
    while let Some(i) = rest.find(name) {
        let before_ok = rest[..i].chars().last().is_none_or(|c| !is_id(c));
        let after_ok = rest[i + name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_id(c));
        out.push_str(&rest[..i]);
        out.push_str(if before_ok && after_ok { value } else { name });
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}

#[test]
fn a_program_is_the_same_song_as_itself() {
    let r = same(SONG, SONG);
    assert!(b(&r, "ok") && b(&r, "same") && b(&r, "text_same"));
    assert_eq!(s(&r, "basis"), "sections");
    assert!(arr(&r, "diffs").is_empty() && arr(&r, "renamed").is_empty());
}

#[test]
fn trance_138_is_its_longhand() {
    // the longhand as written loops forever; the song ends after 64 bars: the nets agree, the song does not
    let raw = same(SONG, &longhand());
    assert!(
        b(&raw, "ok") && !b(&raw, "same"),
        "{}",
        raw.js_stringified()
    );
    assert!(b(&raw, "song_nets_same") && !b(&raw, "sections_compared"));
    let song = raw.get("song").unwrap();
    assert!(
        song.get("ends").is_some() && song.get("song_bars").is_some(),
        "{}",
        song.js_stringified()
    );
    assert!(s(&raw, "why").contains("ends"), "{}", s(&raw, "why"));
    // told to end where the song does, it is the same song, by its longhand nets
    let ended = format!("{}end at bar 64\n", longhand());
    let r = same(SONG, &ended);
    assert!(
        b(&r, "same") && !b(&r, "text_same"),
        "{}",
        r.js_stringified()
    );
    assert_eq!(s(&r, "basis"), "song_nets");
    assert_eq!(
        s(r.get("song_nets").unwrap().get("a").unwrap(), "A"),
        "z4EBG9jG8nT2p449McscV7ZSCfmUJDiFxFU26JpZhJ9hfBFgeLD"
    );
    // and either way round
    assert!(b(&same(&ended, SONG), "same"));
}

#[test]
fn a_let_extraction_is_the_same_song() {
    // write `rst` and `cl` out inside the other lets: different text, different CID, the same nets
    let long = inline_let(&inline_let(SONG, "rst"), "cl");
    assert!(long.len() > SONG.len() + 400, "the refactor did something");
    let r = same(SONG, &long);
    assert!(
        b(&r, "ok") && b(&r, "same") && !b(&r, "text_same"),
        "{}",
        r.js_stringified()
    );
    assert_eq!(s(&r, "basis"), "sections");
    assert!(
        arr(&r, "diffs").is_empty()
            && arr(&r, "only_in_a").is_empty()
            && arr(&r, "only_in_b").is_empty()
    );
}

#[test]
fn a_renamed_section_is_reported_but_is_the_same_song() {
    let renamed = SONG
        .replace("section drop 24 bars", "section peak 24 bars")
        .replace(
            "arrange intro breakdown drop outro",
            "arrange intro breakdown peak outro",
        );
    let r = same(SONG, &renamed);
    assert!(
        b(&r, "same") && !b(&r, "text_same"),
        "{}",
        r.js_stringified()
    );
    let rn = arr(&r, "renamed");
    assert_eq!(rn.len(), 1, "{}", r.js_stringified());
    assert_eq!((s(&rn[0], "a"), s(&rn[0], "b")), ("drop", "peak"));
}

#[test]
fn a_one_note_change_is_a_different_song_and_names_its_section() {
    // `r1` is the last bars of the snare roll, which only the breakdown plays: take one hit out
    let one = SONG.replace(
        "let r1 = x x x x x x x x x x x x x x x x",
        "let r1 = x x x x x x x x x x x x x x x ~",
    );
    assert_ne!(one, SONG);
    let r = same(SONG, &one);
    assert!(b(&r, "ok") && !b(&r, "same"), "{}", r.js_stringified());
    assert_eq!(s(&r, "basis"), "none");
    assert!(b(&r, "song_nets_same") == false);
    let d = arr(&r, "diffs");
    assert_eq!(d.len(), 1, "{}", r.js_stringified());
    assert_eq!((s(&d[0], "section"), s(&d[0], "deck")), ("breakdown", "A"));
    assert_eq!(d[0].get("start_bar").and_then(JsonValue::as_i64), Some(16));
    assert_ne!(s(&d[0], "a_cid"), s(&d[0], "b_cid"));
    assert!(s(&r, "why").contains("breakdown"), "{}", s(&r, "why"));
}

#[test]
fn song_level_settings_count() {
    let faster = SONG.replace("tempo 138", "tempo 140");
    let r = same(SONG, &faster);
    // (the tempo is in the nets too, so they differ as well; the setting is still named)
    assert!(!b(&r, "same"), "{}", r.js_stringified());
    assert!(s(&r, "why").contains("tempo"), "{}", s(&r, "why"));
    let t = r.get("song").unwrap().get("tempo").unwrap();
    assert_eq!(
        (t.get("a").unwrap().as_f64(), t.get("b").unwrap().as_f64()),
        (Some(138.0), Some(140.0))
    );
    let looped = SONG.replace(
        "arrange intro breakdown drop outro",
        "arrange intro breakdown drop outro\nloop",
    );
    let r = same(SONG, &looped);
    assert!(
        !b(&r, "same") && r.get("song").unwrap().get("loops").is_some(),
        "{}",
        r.js_stringified()
    );
}

#[test]
fn a_section_more_or_less_is_only_in_one() {
    let shorter = SONG.replace(
        "arrange intro breakdown drop outro",
        "arrange intro breakdown drop",
    );
    let r = same(SONG, &shorter);
    assert!(!b(&r, "same"));
    let only = arr(&r, "only_in_a");
    assert_eq!(only.len(), 2, "{}", r.js_stringified());
    assert!(only.iter().all(|n| s(n, "section") == "outro"));
}

#[test]
fn a_program_with_errors_is_no_song() {
    let r = same(SONG, "tempo 120\nBAD\n");
    assert!(!b(&r, "ok"));
    assert!(matches!(r.get("same"), Some(JsonValue::Null)));
    assert!(b(r.get("a").unwrap(), "ok") && !b(r.get("b").unwrap(), "ok"));
    assert!(!arr(r.get("b").unwrap(), "diagnostics").is_empty());
}

#[test]
fn the_pair_form_reads_both_from_one_json() {
    let input = JsonValue::Object(djengine::json::JsonObject::from_pairs([
        ("a", JsonValue::str(SONG)),
        ("b", JsonValue::str(SONG)),
    ]))
    .js_stringified();
    let r = JsonValue::parse(&beatscript_api::same_as_json_pair(&input)).unwrap();
    assert!(b(&r, "same"));
    let bad = JsonValue::parse(&beatscript_api::same_as_json_pair("{\"a\": \"tempo 120\"}")).unwrap();
    assert!(!b(&bad, "ok") && s(&bad, "error").contains("\"b\""));
}
