//! "Say it out loud": trance-138, 64 bars, written two ways. `fixtures/trance-138.live` is the longhand
//! (trance-138.py makes it): every part one 64-term `<...>` alternation, a rest bar wherever the part
//! is out. `examples/trance-138.btw` says the same song with lets, two defs, four sections and an
//! `arrange`. Two very different texts, one process: the song written out longhand
//! (`Compiled::song_nets`) has the same net CIDs as the longhand program's own nets.

use djlive::ast::Deck;
use djlive::compile::{check, Compiled};

const LONGHAND: &str = include_str!("fixtures/trance-138.live");
const SONG: &str = include_str!("../../../examples/trance-138.btw");

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

/// The longhand with the one instrument the engine does not have (`supersaw`, which the app played as
/// its default lead) named as the song names it: `detuned-saw`, a voice the engine does have.
fn longhand_with_a_real_voice() -> String {
    LONGHAND.replace("inst=supersaw", "inst=detuned-saw")
}

#[test]
fn the_song_written_out_longhand_has_the_longhands_net_cids() {
    let long = compiled(&longhand_with_a_real_voice());
    let song = compiled(SONG);
    assert!(song.arranged && song.song_bars() == Some(64));
    let want = long.net_cids();
    let got = song.song_net_cids().expect("the song has a longhand");
    assert_eq!(want.keys().collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(got, want, "sections and arrange expand to the longhand's nets");
    // and not by accident: the nets are 64 bars of music
    for d in [Deck::A, Deck::B] {
        let ir = &song.song_nets().unwrap()[&d];
        assert_eq!(ir.period, 64);
        assert!(ir.parts.iter().all(|p| p.cycles.len() == 64));
    }
}

#[test]
fn the_notes_agree_part_by_part_not_just_in_the_hash() {
    let long = compiled(&longhand_with_a_real_voice());
    let song = compiled(SONG).song_nets().unwrap();
    for (d, ir) in &long.decks {
        let theirs = &song[d];
        let mut roles: Vec<&str> = ir.parts.iter().map(|p| p.role.as_str()).collect();
        let mut theirs_roles: Vec<&str> = theirs.parts.iter().map(|p| p.role.as_str()).collect();
        roles.sort();
        theirs_roles.sort();
        assert_eq!(roles, theirs_roles, "deck {}", d.name());
        for p in &ir.parts {
            let q = theirs.parts.iter().find(|q| q.role == p.role).unwrap();
            assert_eq!(p.instrument, q.instrument, "{} {}", d.name(), p.role);
            assert_eq!(p.cycles, q.cycles, "deck {} {}: its 64 bars", d.name(), p.role);
        }
    }
}

#[test]
fn the_one_difference_in_the_longhand_as_it_was_written_is_a_voice_the_engine_lacks() {
    // as Matt's longhand stands, the melody asks for `supersaw`: not a voice (it played the default
    // lead). That one instrument name is the only thing between the two sets of CIDs.
    let long = compiled(LONGHAND);
    let song = compiled(SONG).song_net_cids().unwrap();
    assert_eq!(long.net_cids()["A"], song["A"], "deck A is the same net");
    assert_ne!(long.net_cids()["B"], song["B"], "deck B names supersaw where the song names detuned-saw");
}

#[test]
fn the_timeline_lines_land_on_the_same_bars_as_the_longhands_at_bar_lines() {
    use djlive::compile::Timing;
    let song = compiled(SONG);
    // (the longhand's `at bar N` counts the first music bar as bar 1; a song's own bars count from 0)
    let at = |key: &str| song.controls.iter().find(|c| c.key.contains(key)).map(|c| c.timing);
    assert_eq!(at("filter lp 400 -> 16000"), Some(Timing::At(0)));
    assert_eq!(at("filter lp 16000 -> 1500"), Some(Timing::At(24)), "longhand: at bar 25");
    assert_eq!(at("filter lp 1500 -> 18000"), Some(Timing::At(25)), "longhand: at bar 26");
    assert_eq!(at("filter lp 18000 -> 800"), Some(Timing::At(56)), "longhand: at bar 57");
    let echoes: Vec<Timing> = song.controls.iter().filter(|c| c.key.contains("move echo_out")).map(|c| c.timing).collect();
    assert_eq!(echoes, vec![Timing::At(63), Timing::At(63)], "longhand: at bar 64, A and B");
    // and the song ends after the outro where the longhand would have gone round again
    assert_eq!((song.end, song.looped), (Some(64), false));
    assert_eq!(compiled(&longhand_with_a_real_voice()).end, None);
}
