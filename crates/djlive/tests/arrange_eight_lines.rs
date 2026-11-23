//! `arrange-in-eight-lines`, one of the first programs a newcomer meets (examples/beatscript/arrange-in-eight-lines.beatscript, beatscript.app's
//! example of the same name): one section played eight times, and an `arrange { }` that rewrites the hats and the arp between the
//! plays. Pinned here: its CID, that it checks clean, every play's patterns (the generation table `check` reports, which is a
//! function of the text), and that the formatter the server runs on arrival leaves it exactly as it is, comments and all.

use djlive::compile::check;
use djlive::mutate::generations;

/// The program as the example file has it, comments included (they are the teaching).
const PROGRAM: &str = r#"#! beatscript 2
#! name: arrange-in-eight-lines
#! license: CC0-1.0
# One section, played eight times. The arrange does the rest.
# ~ changes a part from here on (changes stack, pass after pass).
# @ changes it for the next section only, then it snaps back.
tempo 124
key am

let drm_kick = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
let drm_hat = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
let syn_arp = a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4 c4 a3 c4

section groove 4 bars {
  deck A: kick drm_kick kit=drums-v8 | hihat drm_hat kit=drums-v8 vel=70..105
  deck B: arp syn_arp inst=bright-pluck vel=70..100
}

arrange {
  groove
  ~ drm_hat euclid 7
  groove
  ~ syn_arp rot 3
  groove
  @ drm_kick double
  groove
  ~ syn_* invert
  groove
  ~ drm_* grow
  groove
  @ syn_arp half
  groove
  ~ drm_* decay
  groove
}
"#;

/// Its CID, which the comments and the header lines do not move.
const CID: &str = "z4EBG9jHEKgonTqe3ucsjCSx1dMpeceWXr68HEfZrw2n6Yd62sj";

/// What each of the eight plays reads: (play, [(pattern, generation, one-shot, text)]), the let patterns in name order.
#[rustfmt::skip]
const PLAYS: [(&str, [(&str, u32, bool, &str); 3]); 8] = [
    ("groove", [("drm_hat", 0, false, "~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~"), ("drm_kick", 0, false, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 0, false, "a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4 c4 a3 c4")]),
    ("groove@2", [("drm_hat", 1, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x ~"), ("drm_kick", 0, false, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 0, false, "a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4 c4 a3 c4")]),
    ("groove@3", [("drm_hat", 1, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x ~"), ("drm_kick", 0, false, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 1, false, "c4 a3 c4 a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4")]),
    ("groove@4", [("drm_hat", 1, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x ~"), ("drm_kick", 0, true, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 1, false, "c4 a3 c4 a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4")]),
    ("groove@5", [("drm_hat", 1, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x ~"), ("drm_kick", 0, false, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 2, false, "c4 e4 c4 e4 c4 a3 e3 f3 a3 c4 a3 g4 e4 c4 g3 a3")]),
    ("groove@6", [("drm_hat", 2, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x x"), ("drm_kick", 1, false, "x ~ ~ x x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 2, false, "c4 e4 c4 e4 c4 a3 e3 f3 a3 c4 a3 g4 e4 c4 g3 a3")]),
    ("groove@7", [("drm_hat", 2, false, "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x x"), ("drm_kick", 1, false, "x ~ ~ x x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 2, true, "c4 e4 c4 e4 c4 a3 e3 f3")]),
    ("groove@8", [("drm_hat", 3, false, "x ~ ~ x ~ ~ ~ x ~ ~ x ~ x ~ x x"), ("drm_kick", 2, false, "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), ("syn_arp", 2, false, "c4 e4 c4 e4 c4 a3 e3 f3 a3 c4 a3 g4 e4 c4 g3 a3")]),
];

/// Each pattern the arrange rewrites: (pattern, [(generation, operators, text)]), generation 0 as written.
#[rustfmt::skip]
const GENERATIONS: [(&str, &[(u32, &str, &str)]); 3] = [
    ("drm_hat", &[(0, "", "~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~"), (1, "euclid 7", "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x ~"), (2, "grow", "x ~ ~ x ~ x ~ x ~ ~ x ~ x ~ x x"), (3, "decay", "x ~ ~ x ~ ~ ~ x ~ ~ x ~ x ~ x x")]),
    ("drm_kick", &[(0, "", "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), (1, "grow", "x ~ ~ x x ~ ~ ~ x ~ ~ ~ x ~ ~ ~"), (2, "decay", "x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~")]),
    ("syn_arp", &[(0, "", "a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4 c4 a3 c4"), (1, "rot 3", "c4 a3 c4 a3 c4 e4 a4 g4 e4 c4 e4 f3 a3 c4 f4 e4"), (2, "invert", "c4 e4 c4 e4 c4 a3 e3 f3 a3 c4 a3 g4 e4 c4 g3 a3")]),
];
fn table() -> djlive::mutate::GenTable {
    generations(PROGRAM, &djlive::env::Env::none()).expect("a generation table")
}

#[test]
fn it_is_the_example_file_and_checks_clean() {
    assert_eq!(PROGRAM, include_str!("../../../examples/beatscript/arrange-in-eight-lines.beatscript"), "the const is the example file, byte for byte");
    let c = check(PROGRAM);
    assert!(c.diagnostics.is_empty(), "{}", djlive::diag::render_all(&c.diagnostics, PROGRAM));
    let c = c.compiled.expect("it compiles");
    assert_eq!(c.cid, CID);
    // one section of four bars, played eight times: thirty-two bars, in eight segments
    assert_eq!(c.song_bars(), Some(32));
    let names: Vec<&str> = c.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["groove", "groove@2", "groove@3", "groove@4", "groove@5", "groove@6", "groove@7", "groove@8"]);
    // the comments and header lines are not in the canonical text, and the canonical text is the same program
    assert!(!c.text.contains('#'), "{}", c.text);
    assert_eq!(check(&c.text).compiled.unwrap().cid, CID);
}

#[test]
fn its_eight_plays_read_exactly_these_patterns() {
    let t = table();
    assert_eq!((t.passes, t.unbounded, t.arrangement.as_deref()), (1, false, None));
    assert_eq!(t.plays.len(), 8);
    for (play, (name, reads)) in t.plays.iter().zip(PLAYS) {
        assert_eq!((play.section.as_str(), play.name.as_str(), play.pass, play.coda), ("groove", name, 0, false));
        let got: Vec<(&str, u32, bool, &str)> = play.reads.iter().map(|r| (r.pattern.as_str(), r.generation, r.once, r.text.as_str())).collect();
        assert_eq!(got, reads, "{name}");
    }
}

#[test]
fn its_patterns_change_in_the_order_the_comments_say() {
    let t = table();
    assert_eq!(t.patterns.len(), 3);
    for (name, want) in GENERATIONS {
        let got: Vec<(u32, &str, &str)> = t.patterns[name].iter().map(|g| (g.generation, g.ops.as_str(), g.text.as_str())).collect();
        assert_eq!(got, want, "{name}");
    }
    // the hat: euclid 7, then grow, then decay; the arp: rot 3, then invert; `drm_*` and `syn_*` take the kick with the hat
    // (grow, decay), and `syn_*` the arp, in the same plays as the lines above them say
    let ops = |p: &str| t.patterns[p].iter().skip(1).map(|g| g.ops.as_str()).collect::<Vec<_>>();
    assert_eq!((ops("drm_hat"), ops("syn_arp"), ops("drm_kick")), (vec!["euclid 7", "grow", "decay"], vec!["rot 3", "invert"], vec!["grow", "decay"]));
    // `~` is for good: every play after a change reads that generation or a later one, never an earlier
    for p in ["drm_hat", "drm_kick", "syn_arp"] {
        let gens: Vec<u32> = t.plays.iter().filter_map(|pl| pl.reads.iter().find(|r| r.pattern == p && !r.once).map(|r| r.generation)).collect();
        assert!(gens.windows(2).all(|w| w[0] <= w[1]), "{p}: {gens:?}");
    }
}

#[test]
fn the_at_marks_last_one_play_and_then_the_pattern_is_back() {
    let t = table();
    let read = |play: usize, pat: &str| t.plays[play].reads.iter().find(|r| r.pattern == pat).unwrap().clone();
    // `@ drm_kick double` before the fourth play: the kick twice in the bar, once, then the generation it was
    let (before, during, after) = (read(2, "drm_kick"), read(3, "drm_kick"), read(4, "drm_kick"));
    assert!(!before.once && during.once && !after.once);
    assert_eq!(during.text, format!("{} {}", before.text, before.text));
    assert_eq!((after.generation, &after.text), (before.generation, &before.text));
    // `@ syn_arp half` before the seventh: the first half of the arp, once, then the generation it was
    let (before, during, after) = (read(5, "syn_arp"), read(6, "syn_arp"), read(7, "syn_arp"));
    assert!(!before.once && during.once && !after.once);
    assert_eq!(during.text, before.text.split(' ').take(8).collect::<Vec<_>>().join(" "));
    assert_eq!((after.generation, &after.text), (before.generation, &before.text));
    // and no other play has a one-shot
    let once: Vec<usize> = (0..8).filter(|&i| t.plays[i].reads.iter().any(|r| r.once)).collect();
    assert_eq!(once, [3, 6]);
}

#[test]
fn the_formatter_the_server_runs_on_arrival_leaves_it_as_it_is() {
    // `djlive::format` is what fmt, eval, load, share, queue and push_private run: this text is already in its form, so it
    // comes back byte for byte, with the three header lines, the three teaching comments and the same CID
    let f = djlive::format::format(PROGRAM, &djlive::env::Env::none()).expect("it reads");
    assert_eq!(f, PROGRAM);
    for line in [
        "#! beatscript 2",
        "#! name: arrange-in-eight-lines",
        "#! license: CC0-1.0",
        "# One section, played eight times. The arrange does the rest.",
        "# ~ changes a part from here on (changes stack, pass after pass).",
        "# @ changes it for the next section only, then it snaps back.",
    ] {
        assert!(f.lines().any(|l| l == line), "the formatted text lost {line:?}:\n{f}");
    }
    assert_eq!(check(&f).compiled.unwrap().cid, CID);
    // a loosely written copy (spaces, extra blank lines, a trailing comment) keeps every comment and header line, the CID, and
    // formats to a fixed point
    let loose = PROGRAM.replace("tempo 124\n", "tempo   124   # a tempo\n\n\n").replace("  ~ drm_hat euclid 7", "  ~   drm_hat   euclid 7");
    let f = djlive::format::format(&loose, &djlive::env::Env::none()).expect("it reads");
    for line in ["#! beatscript 2", "#! name: arrange-in-eight-lines", "#! license: CC0-1.0", "# @ changes it for the next section only, then it snaps back."] {
        assert!(f.lines().any(|l| l == line), "{line:?} in {f}");
    }
    assert!(f.contains("tempo 124  # a tempo\n") && f.contains("  ~ drm_hat euclid 7\n"), "{f}");
    assert_eq!(check(&f).compiled.unwrap().cid, CID);
    assert_eq!(djlive::format::format(&f, &djlive::env::Env::none()).unwrap(), f);
}
