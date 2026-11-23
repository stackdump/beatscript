//! Cross-feature checks of the extensions against sets (programs of programs, `set { play X  mix N bars with T  play Y
//! until 20m }`, merged from origin/feat/btw-language after the extensions): the lexer's units (`3m30s` is `3` `m` `30`
//! `s`, and so is a set's `until 20m`), the set words against the extension words and the highlight, the canonical forms,
//! a set of tracks that use the extensions (arithmetic, a tempo curve, chance, `stack`/`then`), the set's tempo through the
//! same tempo map every program has, and a share link of a set whose tracks carry `#! genre:` and `#! tags:` lines.

use djengine::json::JsonValue;
use djlive::env::{Env, StaticSnippets};
use djlive::highlight::{highlight, Class};
use djlive::lex::{lex, Tok};
use djlive::runtime::{Grid, Live, LiveHost};

// ---- helpers (the ones tests/sets.rs has)

fn store(progs: &[(&str, &str)]) -> StaticSnippets {
    let mut s = StaticSnippets::default();
    for (name, text) in progs {
        let c = djlive::check(text).compiled.unwrap_or_else(|| panic!("{name} does not compile: {:?}", djlive::check(text).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        s.items.push((name.to_string(), c.cid, c.text));
    }
    s
}

fn compile(text: &str, snips: &StaticSnippets) -> djlive::Check {
    djlive::compile::check_with(text, &Env { snippets: Some(snips), ..Env::default() })
}

fn ok(text: &str, snips: &StaticSnippets) -> djlive::Compiled {
    let c = compile(text, snips);
    assert!(c.compiled.is_some(), "{text}\n{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    c.compiled.unwrap()
}

fn errors(text: &str, snips: &StaticSnippets) -> Vec<String> {
    compile(text, snips).diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{} {}", d.message, d.help.clone().unwrap_or_default())).collect()
}

fn words(src: &str) -> Vec<Tok> {
    lex(src).tokens.into_iter().map(|t| t.tok).filter(|t| !matches!(t, Tok::Newline | Tok::Eof)).collect()
}

fn num(v: f64) -> Tok {
    Tok::Num { value: v, int: v.fract() == 0.0 }
}

fn unit(u: &str) -> Tok {
    Tok::Unit(u.to_string())
}

const LONG: &str = "# long\ntempo 120\ndeck A: kick x ~ ~ ~\nloop\n";

// ---- (1) the lexer: one reading of a time, for a set and for everything else

#[test]
fn a_time_is_a_number_and_its_unit_whether_it_is_a_sets_until_or_an_assertions_length() {
    // the extensions' tokens, unchanged
    assert_eq!(words("3m30s"), vec![num(3.0), unit("m"), num(30.0), unit("s")]);
    assert_eq!(words("20m"), vec![num(20.0), unit("m")]);
    assert_eq!(words("90s"), vec![num(90.0), unit("s")]);
    assert_eq!(words("1.5s"), vec![Tok::Num { value: 1.5, int: false }, unit("s")]);
    // hours join them: `1h`, and `1h30m` the way `3m30s` is
    assert_eq!(words("1h"), vec![num(1.0), unit("h")]);
    assert_eq!(words("1h30m"), vec![num(1.0), unit("h"), num(30.0), unit("m")]);
    // `hz` is still `hz`, and a unit that runs into letters is no unit
    assert_eq!(words("300hz"), vec![num(300.0), unit("hz")]);
    assert!(!lex("5hr").errors.is_empty());

    // every spelling of a time is one time in a set: one CID
    let s = store(&[("long", LONG)]);
    let cid = |t: &str| ok(&format!("set {{ play long until {t} }}"), &s).cid;
    assert_eq!(cid("20m"), cid("1200s"));
    assert_eq!(cid("20m"), cid("0.3333333333333333h"));
    assert_eq!(cid("1h"), cid("3600s"));
    assert_eq!(cid("1h"), cid("60m"));
    assert_eq!(cid("3m30s"), cid("210s"));
    assert_eq!(cid("1h30m"), cid("5400s"));
    assert_eq!(cid("1h30m"), cid("90m"));
    assert_eq!(cid("1h30m15s"), cid("5415s"));
    assert_ne!(cid("3m30s"), cid("3m31s"));
    // and the canonical text says it the shortest exact way
    assert!(ok("set { play long until 3600s }", &s).text.ends_with("until 1h\n}\n"));
    assert!(ok("set { play long until 1h30m }", &s).text.ends_with("until 90m\n}\n"));
    assert!(ok("set { play long until 3m30s }", &s).text.ends_with("until 210s\n}\n"));

    // a time that is not one is said, with the way out
    for bad in ["20", "20ms", "20db", "hz", "1h 30m x"] {
        let e = errors(&format!("set {{ play long until {bad} }}"), &s);
        assert!(!e.is_empty(), "{bad}");
    }
    let e = errors("set { play long until 20 }", &s);
    assert!(e[0].contains("`until` takes a time (20m)"), "{e:?}");
    // outside a set an hour is a unit the language does not read there: the error names it and where units go
    let c = djlive::check("tempo 1h\ndeck A: kick x\n");
    assert!(c.compiled.is_none());
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(d.message.contains("the unit `h`") && d.help.clone().unwrap().contains("after a set's `until`"), "{d:?}");

    // `assert length` reads its clock the same way, in a program and in a track of a set
    let song = "tempo 128\nsection intro 8 bars {\n  deck A: kick x ~ ~ ~\n}\narrange intro\n";
    let with = |a: &str| format!("{song}assert length {a}\n");
    assert!(djlive::check(&with("< 3m30s")).compiled.is_some());
    assert!(djlive::check(&with("< 90s")).compiled.is_some());
    assert!(djlive::check(&with("> 3m30s")).compiled.is_none(), "an eight-bar song is not three and a half minutes long");
    let s2 = store(&[("song", &with("< 3m30s"))]);
    let c = ok("set { play song\n mix 4 bars with crossfade\n play song }", &s2);
    assert_eq!(c.set.as_ref().unwrap().total_bars, 12);
}

// ---- (2) the words

#[test]
fn the_set_words_are_names_everywhere_but_where_the_set_reads_them() {
    use djlive::parse::{EXT_RESERVED, EXT_WORDS, STATEMENT_WORDS};
    // `set` starts a statement (a typo of it is hinted); it and `until` are words of the extensions, and neither is reserved:
    // a name keeps working, as `mix` does
    assert!(STATEMENT_WORDS.contains(&"set"));
    assert!(EXT_WORDS.contains(&"set") && EXT_WORDS.contains(&"until") && EXT_WORDS.contains(&"mix"));
    assert!(!EXT_RESERVED.contains(&"set") && !EXT_RESERVED.contains(&"until") && !EXT_RESERVED.contains(&"mix"));
    let plain = "let set = 2\nlet until = 3\ndef mix(n) {\n  at bar $n: master: reverb 0.2\n}\nsection play2 4 bars {\n  deck A: kick x*set\n}\narrange play2\nmix(until)\n";
    let c = djlive::check(plain);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    // a section may be named `mix` and `set` (not while a `let` of that name stands in its place)
    let named = "section mix 4 bars {\n  deck A: kick x ~ ~ ~\n}\nsection set 4 bars {\n  deck B: bass a2 ~ ~ ~\n}\narrange mix set\n";
    assert!(djlive::check(named).compiled.is_some(), "{:?}", djlive::check(named).diagnostics);
    // a plain program's `mix { }` block is the mix block it was
    let block = "tempo 124\nsection drop 8 bars {\n  deck A: kick x ~ ~ ~\n  mix {\n    A level 0.85\n    B eq low -6\n  }\n}\narrange drop\n";
    let c = djlive::check(block);
    assert!(c.compiled.is_some(), "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    assert!(c.compiled.unwrap().text.contains("mix {"));
    // and the whole mix example of the extensions still compiles with a set's word `mix` in the same file's neighbours
    let mix_example = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/mix.beatscript")).unwrap();
    assert!(djlive::check(&mix_example).compiled.is_some());

    // a set parses as it did, `play` is the action word it is elsewhere, and a deck line with `play` is untouched
    let s = store(&[("alpha", "# a\ntempo 130\ndeck A: kick x ~ ~ ~\nend at bar 8\n"), ("beta", "# b\ntempo 122\ndeck B: kick x ~ x ~\nend at bar 8\n")]);
    let set = "set {\n  play alpha\n  mix 4 bars with crossfade\n  play beta until bar 6\n}\n";
    ok(set, &s);
    let action = "tempo 128\ndeck A: kick x ~ ~ ~\nat bar 2: deck B: play\nat bar 6: deck A: stop\nevery 4 bars: deck A: sync\n";
    assert!(djlive::check(action).compiled.is_some());

    // a `let` next to a set is the program's mistake and says so (a set holds the set, `end`, `loop` and `use restricted`)
    let e = errors(&format!("let mix = 4\n{set}"), &s);
    assert!(e.iter().any(|m| m.contains("holds the set")), "{e:?}");
    // a set may not be inside a section or after other statements
    let e = errors("section intro 4 bars {\n  set {\n    play alpha\n  }\n}\narrange intro\n", &s);
    assert!(e.iter().any(|m| m.contains("a `set` is a whole program")), "{e:?}");
    // a brace on the next line is no set: the hint says where it goes (and does not say `did you mean set`)
    let e = errors("set\n{\n  play alpha\n}\n", &s);
    assert!(e.iter().any(|m| m.contains("brace on the same line")), "{e:?}");
    // a typo of `set` is hinted
    let e = errors("sett {\n  play alpha\n}\n", &s);
    assert!(e.iter().any(|m| m.contains("did you mean `set`?")), "{e:?}");

    // the highlight: the set's words are keywords where the set reads them and words where they are names
    let class = |src: &str, w: &str| highlight(src).into_iter().find(|(sp, _)| sp.text(src) == w).map(|(_, c)| c);
    assert_eq!((class(set, "set"), class(set, "mix"), class(set, "until"), class(set, "play"), class(set, "with"), class(set, "bars")), (Some(Class::Keyword), Some(Class::Keyword), Some(Class::Keyword), Some(Class::Action), Some(Class::Keyword), Some(Class::Keyword)));
    assert_eq!(class(plain, "set"), Some(Class::Word));
    assert_eq!(class(named, "mix"), Some(Class::Word));
    assert_eq!(class(block, "mix"), Some(Class::Keyword), "a mix block's `mix` is still a keyword");
}

// ---- (3) canonical forms and CIDs

#[test]
fn the_extensions_programs_keep_their_canonical_text_and_cid_through_the_merge() {
    // the showcase is every extension at once: its CID is the one the extensions branch had
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/beatscript/showcase.beatscript")).unwrap();
    let c = djlive::check(&src).compiled.expect("the showcase compiles");
    assert_eq!(c.cid, "z4EBG9j37QBGtpgBbGG2nLFDQ3nvwq1FdePDEjcZDiSLtHVbRVy");
    // and a program with no `set` in it is not touched by the set's path (one compile, same canonical text twice)
    assert_eq!(djlive::check(&c.text).compiled.unwrap().text, c.text);
    // the set's canonical text is a fixed point too, tracks pinned by CID
    let s = store(&[("long", LONG)]);
    let set = ok("set { play long until 1h30m }", &s);
    let again = compile(&set.text, &s).compiled.unwrap();
    assert_eq!((again.text.clone(), again.cid.clone()), (set.text.clone(), set.cid.clone()));
}

// ---- (5) a set of tracks that use the extensions

/// Arithmetic (`let`, `2 * 4`, `2 + 2`), a tempo curve, chance (`?0.45`, `choose`), and a section that is a `then` of two
/// sections; on both decks.
const TRACK_X: &str = "# x: arithmetic, a tempo curve and chance\ntempo 100 -> 128 over 4 bars\nkey am\nlet total = 2 * 4\nsection groove total bars {\n  deck A: kick e(2 + 2, 16) | hihat x*16?0.45 vel=(40 + 2 * 4)..85\n  deck B: bass choose [a1 c2 e2] weights 4 2 1 seed=4 ~ a1 ~ oct=1\n}\narrange groove\n";
/// `stack` and `then`, arithmetic in a pattern, chance, a length assertion read in minutes and seconds.
const TRACK_Y: &str = "# y: stack and then\ntempo 120\nkey em\nsection lift 4 bars {\n  deck A: kick x ~ x ~ | hihat x*(2 + 2)\n}\nsection wash 4 bars {\n  deck B: melody <e4 g4 b4 g4> | bass [e2 ~ e2 ~]?0.7\n}\narrange then(lift, stack(lift, wash))\nassert length < 3m30s\n";
const TWO: &str = "set {\n  play x\n  mix 4 bars with crossfade\n  play y\n}\n";

#[test]
fn a_set_of_tracks_with_arithmetic_a_tempo_curve_chance_and_stack_then_checks_and_has_stable_distinct_cids() {
    let s = store(&[("x", TRACK_X), ("y", TRACK_Y)]);
    let (cx, cy) = (s.items[0].1.clone(), s.items[1].1.clone());
    assert_ne!(cx, cy);
    let c = ok(TWO, &s);
    let r = c.set.as_ref().unwrap();
    // 8 + 8 - 4
    assert_eq!(r.total_bars, 12);
    assert_eq!(r.tracks.iter().map(|t| t.cid.clone()).collect::<Vec<_>>(), vec![cx.clone(), cy.clone()]);
    assert_eq!(c.text, format!("set {{\n  play {cx}\n  mix 4 bars with crossfade\n  play {cy}\n}}\n"));
    assert!(c.warnings.iter().all(|w| w.is_error() == false));
    // the tracks keep their keys (the set reports them, Camelot too), and the key of each stretch is its track's
    assert_eq!((r.tracks[0].camelot.as_deref(), r.tracks[1].camelot.as_deref()), (Some("8A"), Some("9A")));
    assert_eq!(c.section_key("1. groove").unwrap().text(), "am");
    assert_eq!(c.section_key("2. lift+wash").map(|k| k.text()).or_else(|| c.segments.iter().find(|s| s.name.starts_with("2.")).and_then(|s| c.section_keys.get(&s.name)).map(|k| k.text())), Some("em".to_string()));
    // stable: the same text, the same CID, however it is spelled; and a name, the end of a CID or a quote is the same track
    let tail = &cy[cy.len() - 12..];
    let spelled = format!("set  {{\n\n  play \"x\"   # first\n  mix  4 bars   with crossfade\n  play {}\n}}\n", if tail.starts_with(|c: char| c.is_ascii_digit()) { format!("\"{tail}\"") } else { tail.to_string() });
    assert_eq!(ok(&spelled, &s).cid, c.cid);
    assert_eq!(ok(TWO, &s).cid, c.cid);
    // distinct: another mix, another track, another transition, another time is another set
    for other in [TWO.replace("4 bars", "2 bars"), TWO.replace("crossfade", "echo_out"), TWO.replace("play y", "play x"), TWO.replace("play x", "play y")] {
        assert_ne!(ok(&other, &s).cid, c.cid, "{other}");
    }
    // a track that changes an extension's number is another track, so another set; a comment is not a change
    let x2 = TRACK_X.replace("2 * 4", "2 * 5");
    let s2 = store(&[("x", &x2), ("y", TRACK_Y)]);
    assert_ne!(s2.items[0].1, cx);
    assert_ne!(ok(TWO, &s2).cid, c.cid);
    let s3 = store(&[("x", &format!("{TRACK_X}# one more note to self\n")), ("y", TRACK_Y)]);
    assert_eq!(s3.items[0].1, cx);
    assert_eq!(ok(TWO, &s3).cid, c.cid);

    // the tracks play on the decks they say: x on A and B, y on A and B, and a stretch of the mix has both tracks' parts
    let segs: Vec<(String, u32, u32)> = c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars)).collect();
    assert_eq!(segs.first().map(|s| (s.1, s.2)), Some((0, 4)));
    assert!(segs.iter().filter(|s| s.0.starts_with("mix")).count() == 4, "{segs:?}");
    let mix_bar = c.segments.iter().find(|s| s.name.starts_with("mix")).unwrap();
    let roles: Vec<&str> = mix_bar.decks.values().flat_map(|d| d.parts.iter().map(|p| p.role.as_str())).collect();
    assert!(roles.contains(&"kick") && roles.contains(&"hihat") && roles.contains(&"bass"), "{roles:?}");

    // `until` reads a track's seconds through its tempo curve: 100 -> 128 over four bars, then 128: 1200 s is not 1200 * 100 / 240 bars
    let sl = store(&[("curve", "# curve\ntempo 100 -> 128 over 4 bars\ndeck A: kick x ~ ~ ~\nloop\n"), ("flat", "# flat\ntempo 100\ndeck A: kick x ~ ~ ~\nloop\n")]);
    let bars = |t: &str, until: &str| ok(&format!("set {{ play {t} until {until} }}"), &sl).set.unwrap().total_bars;
    assert_eq!(bars("flat", "20m"), 500);
    let through = bars("curve", "20m");
    assert!(through > 500 && through < 640, "{through}");
    assert_eq!(bars("curve", "90s"), (djlive::check("tempo 100 -> 128 over 4 bars\ndeck A: kick x ~ ~ ~\nloop\n").compiled.unwrap().tempo_map().bar_at(90.0)).round() as u32);
}

// ---- the set's tempo is a tempo map like every program's

#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    tempos: Vec<(f64, f64)>,
    loads: usize,
    calls: Vec<(f64, String)>,
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        Some(Grid { deck: "A".into(), beat: self.beat, bpm: 120.0 })
    }
    fn playing(&self, deck: &str) -> bool {
        self.playing[usize::from(deck == "B")]
    }
    fn load_net(&mut self, _deck: &str, _env: &JsonValue) -> Result<(), String> {
        self.loads += 1;
        Ok(())
    }
    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[usize::from(deck == "B")] = on;
    }
    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, format!("{noun} {}", params.canonical_js())));
        Ok(JsonValue::Null)
    }
    fn set_tempo(&mut self, _deck: &str, bpm: f64) -> Result<(), String> {
        self.tempos.push((self.beat, bpm));
        Ok(())
    }
}

#[test]
fn a_sets_tempo_is_one_tempo_map_the_tracks_curves_and_the_mixes_ramps_in_it() {
    let s = store(&[("x", TRACK_X), ("y", TRACK_Y)]);
    let c = ok(TWO, &s);
    let m = c.tempo_map();
    assert!(m.drives(), "the set's tempo moves: the runtime follows it like any program's curve");
    // x's own curve while it plays alone (set bars 0 to 3): 100 -> 128 over its first four bars
    assert!((m.bpm_at(0u32) - 100.0).abs() < 1e-9);
    assert!(m.bpm_at(2u32) > 100.0 && m.bpm_at(2u32) < 128.0);
    // the mix (set bars 4 to 7) ramps from where x is to where y is: 128 -> 120
    assert!((m.bpm_at(4u32) - 128.0).abs() < 1e-9);
    assert!((m.bpm_at(6u32) - 124.0).abs() < 1e-6, "{}", m.bpm_at(6u32));
    // y alone (set bars 8 to 11) at its own
    assert!((m.bpm_at(8u32) - 120.0).abs() < 1e-9 && (m.bpm_at(11u32) - 120.0).abs() < 1e-9);
    // the report, the length and the plan agree with it: seconds are the map's integral, not bars at one tempo
    let r = c.set.as_ref().unwrap();
    assert!((r.seconds - m.total_seconds(12u32)).abs() < 1e-9);
    assert!(r.seconds > 12.0 * 240.0 / 128.0 && r.seconds < 12.0 * 240.0 / 100.0, "{}", r.seconds);
    let len = c.length_report().unwrap();
    assert!(len.contains("with the tempo curve") && len.contains("12 bars"), "{len}");
    assert_eq!(r.mixes[0].plan, "mix 1→2: 128→120 over 4 bars (crossfade)");
    // a plan that only moves a mix's ramp is one change, not "no changes"
    let s_alt = ok(&TWO.replace("crossfade", "crossfade, tempo 128 -> 118"), &s);
    let plan = djlive::runtime::Plan { tempo_changed: !c.tempo_map().same_tempos(s_alt.tempo_map()), ..Default::default() };
    assert!(plan.tempo_changed);
    assert_eq!(plan.summary(), "1 changed");

    // on a fake clock the host is told the curve: from x's 100, up to 128, down along the mix to y's 120, and no further
    let mut h = Fake { beat: 8.0, playing: [true, false], ..Fake::default() };
    let mut live = Live::new();
    let ev = live.eval_with(TWO, &Env { snippets: Some(&s), ..Env::default() });
    assert!(ev.ok);
    assert!(ev.length.as_deref().is_some_and(|l| l.contains("with the tempo curve")), "{:?}", ev.length);
    assert!(ev.set.is_some());
    for _ in 0..16 * 16 {
        h.beat += 0.25;
        live.service(&mut h);
    }
    assert!(h.loads > 0);
    let t: Vec<f64> = h.tempos.iter().map(|(_, b)| *b).collect();
    assert!(!t.is_empty());
    assert!(t.iter().all(|b| (99.99..=128.01).contains(b)), "{t:?}");
    assert!(t.iter().cloned().fold(f64::MIN, f64::max) > 127.0 && t.iter().any(|b| (b - 124.0).abs() < 1.0), "the curve reaches 128 and the mix passes 124: {t:?}");
    assert!((t.last().unwrap() - 120.0).abs() < 0.5, "{t:?}");
    // nothing is driven for a set at one tempo: the nets carry it, as before
    let steady = store(&[("p", "# p\ntempo 124\ndeck A: kick x ~ ~ ~\nend at bar 8\n"), ("q", "# q\ntempo 124\ndeck B: kick x ~ x ~\nend at bar 8\n")]);
    let c = ok("set { play p\n mix 4 bars with crossfade\n play q }", &steady);
    assert!(!c.tempo_map().drives() && c.tempo_map().constant_bpm() == Some(124.0));
    assert_eq!(c.length_report().unwrap(), "length: 25.8s at 124, 12 bars".replace("25.8s", &djlive::evolve::human(12.0 * 240.0 / 124.0)));
}

// ---- (6) share links of a set whose tracks carry the link features

#[test]
fn a_share_link_of_a_set_carries_tracks_whose_genre_and_tags_lines_are_comments_not_part_of_any_cid() {
    let plain_x = TRACK_X.to_string();
    let tagged_x = format!("#! genre: house\n#! tags: euclid, chance\n{TRACK_X}");
    let tagged_y = format!("#! genre: trance\n#! tags: stack\n{TRACK_Y}");
    let s_plain = store(&[("x", &plain_x), ("y", TRACK_Y)]);
    let s_tagged = store(&[("x", &tagged_x), ("y", &tagged_y)]);
    // the headers are comments: the tracks' CIDs, and so the set's, are the same
    assert_eq!(s_plain.items[0].1, s_tagged.items[0].1);
    assert_eq!(s_plain.items[1].1, s_tagged.items[1].1);
    let c = ok(TWO, &s_tagged);
    assert_eq!(c.cid, ok(TWO, &s_plain).cid);
    assert!(!c.text.contains("genre"), "the canonical text of a set names no genre");
    // the link inlines the tracks (their canonical texts, headers gone) and opens where nothing is saved
    let (url, inlined) = djlive::share::encode_with_tracks(&c, None, false);
    assert!(inlined, "two tracks of this size travel inline");
    let link = djlive::share::parse_link(&url).unwrap();
    assert_eq!(link.tracks.len(), 2);
    assert!(link.tracks.values().all(|t| !t.contains("genre")), "{:?}", link.tracks);
    let nothing = StaticSnippets::default();
    let opened = djlive::share::open(&url, &Env { snippets: Some(&nothing), ..Env::default() }).unwrap();
    let o = opened.check.compiled.expect("a link of a set opens without its tracks saved");
    assert_eq!(o.cid, c.cid);
    assert_eq!(o.set.unwrap().total_bars, 12);
    // a link by CID alone (a track left out) is refused by name when the receiver has not got the track
    let (url2, inlined2) = djlive::share::encode_with_tracks(&c, Some("matt"), true);
    assert_eq!(djlive::share::parse_link(&url2).unwrap().tracks.len() == 2, inlined2);
    // the set's own genre header is a comment of the set: not in its CID either
    let with_header = format!("#! genre: dj-set\n{TWO}");
    let hdr = ok(&with_header, &s_plain);
    assert_eq!(hdr.cid, ok(TWO, &s_plain).cid);
    // ... but it is a note of the set, and a link may carry it (unsealed, and the opener is told)
    let (url3, _) = djlive::share::encode_with_tracks(&hdr, None, true);
    let link3 = djlive::share::parse_link(&url3).unwrap();
    assert!(link3.notes.iter().any(|n| n.contains("genre: dj-set")), "{:?}", link3.notes);
    let opened3 = djlive::share::open(&url3, &Env { snippets: Some(&nothing), ..Env::default() }).unwrap();
    assert_eq!(opened3.check.compiled.unwrap().cid, hdr.cid);
}

// ---- a track's lines that run on (an lfo, a duck) stop with the track, on the set's clock

#[test]
fn a_tracks_lfo_and_duck_stop_where_the_track_does_in_a_set() {
    // track p has no section: its lfo would run until the program stops; track q's lfo stops at the end of its section, which
    // is a bar of q's own song, not of the set
    let p = "tempo 120\ndeck A: kick x ~ ~ ~\nmaster: filter lp lfo sine 2 bars center=3k depth=1k\nduck B under A.kick 6db\nend at bar 4\n";
    let q = "tempo 120\nsection drop 8 bars {\n  deck B: kick x ~ ~ ~\n  master: filter lp lfo sine 2 bars center=3k depth=1k\n}\narrange drop\n";
    let s = store(&[("p", p), ("q", q)]);
    let text = "set { play p\n play q }";
    let c = ok(text, &s);
    // the lfos' stops are set bars: p's at the end of p (bar 4), q's at the end of q (bar 12)
    let stops: Vec<Option<u32>> = c.controls.iter().filter_map(|k| if let djlive::compile::Effect::Lfo(l) = k.effect.base() { Some(l.until) } else { None }).collect();
    assert_eq!(stops, vec![Some(4), Some(12)]);
    // the duck that held until the program stopped is put back when p ends
    let duck = c.controls.iter().find(|k| matches!(k.effect.base(), djlive::compile::Effect::Duck(_))).unwrap();
    assert_eq!(duck.hold.map(|d| d.to_f64()), Some(4.0));

    let mut h = Fake { beat: 8.0, playing: [true, false], ..Fake::default() };
    let mut live = Live::new();
    assert!(live.eval_with(text, &Env { snippets: Some(&s), ..Env::default() }).ok);
    let mut origin: Option<f64> = None;
    for _ in 0..16 * 14 {
        h.beat += 0.25;
        live.service(&mut h);
        if live.is_running() && origin.is_none() {
            origin = Some(h.beat);
        }
    }
    let o = origin.unwrap();
    // filter calls a bar, counted from the bar the set began on: one lfo at a time (about 16 a bar), never two at once, none
    // after the set ends (12 bars)
    let mut per_bar = [0usize; 16];
    for (b, c) in h.calls.iter().filter(|(_, c)| c.contains("filter")) {
        per_bar[(((b - o) / 4.0).floor() as usize).min(15)] += 1;
    }
    assert!(per_bar[..12].iter().all(|n| *n > 0), "{per_bar:?}");
    assert!(per_bar[1..12].iter().all(|n| *n <= 20), "two lfos at once: {per_bar:?}");
    assert!(per_bar[12..].iter().all(|n| *n <= 16), "{per_bar:?}");
    assert_eq!(per_bar[13..].iter().sum::<usize>(), 0, "nothing after the set: {per_bar:?}");
    // q's lfo runs to the end of q (it was cut at set bar 8, the bar q's section ends on in q's own count, before)
    assert!(per_bar[9] >= 12 && per_bar[10] >= 12 && per_bar[11] >= 12, "{per_bar:?}");
}

#[test]
fn a_tracks_sub_bar_lines_run_only_while_the_track_does_in_a_set() {
    // `every 2 beats` and `at bar 1 beat 3` are not bar lines: the runtime fires them between bars, and in a set only while their
    // track plays (the first track is four bars long, the second eight)
    let p = "tempo 120\ndeck A: kick x ~ ~ ~\nevery 2 beats: master: reverb 0.3\nat bar 1 beat 3: master: delay 0.4\nend at bar 4\n";
    let q = "tempo 120\ndeck B: kick x ~ ~ ~\nat bar 2 beat 1: master: crush 0.2\nend at bar 8\n";
    let s = store(&[("p", p), ("q", q)]);
    let text = "set { play p\n play q }";
    let c = ok(text, &s);
    let windows: Vec<_> = c.controls.iter().map(|k| (k.timing, k.window)).collect();
    assert!(windows.iter().all(|w| w.1.is_some()), "{windows:?}");
    let mut h = Fake { beat: 8.0, playing: [true, false], ..Fake::default() };
    let mut live = Live::new();
    assert!(live.eval_with(text, &Env { snippets: Some(&s), ..Env::default() }).ok);
    // the set's own clock: the runtime's bar position at each beat, from the bar the set began on
    let (mut origin, mut offset): (Option<i64>, Option<f64>) = (None, None);
    for _ in 0..16 * 16 {
        h.beat += 0.25;
        live.service(&mut h);
        if live.is_running() && origin.is_none() {
            origin = Some(live.origin());
            offset = Some(live.marks().pos - h.beat / 4.0);
        }
    }
    let (origin, offset) = (origin.unwrap() as f64, offset.unwrap());
    let at = |what: &str| -> Vec<f64> { h.calls.iter().filter(|(_, c)| c.contains(what)).map(|(b, _)| ((b / 4.0 + offset - origin) * 100.0).round() / 100.0).collect() };
    let reverb = at("reverb.mix");
    // every two beats from the first two beats over four bars: seven times (the eighth is bar 4: the next track's), none after
    assert_eq!(reverb.len(), 7, "{reverb:?}");
    assert!(reverb.iter().all(|b| *b >= 0.0 && *b < 4.0), "{reverb:?}");
    // once each, in their own track's bars: bar 1 beat 3 of p is 1.75 (a beat is 0 to 3); bar 2 beat 1 of q is set bar 6.25
    assert_eq!(at("delay.mix").len(), 1);
    assert!((at("delay.mix")[0] - 1.75).abs() < 0.1, "{:?}", at("delay.mix"));
    assert_eq!(at("crush.mix").len(), 1);
    assert!((at("crush.mix")[0] - 6.25).abs() < 0.1, "{:?}", at("crush.mix"));
}

#[test]
fn a_set_is_linked_data_too_its_stretches_are_its_sections_and_it_has_no_rhythm_of_its_own() {
    let s = store(&[("x", TRACK_X), ("y", TRACK_Y)]);
    let c = ok(TWO, &s);
    let secs = djlive::linked::sections(&c);
    assert_eq!(secs.iter().map(|f| (f.name.as_str(), f.start, f.bars)).collect::<Vec<_>>().first().copied(), Some(("1. groove", 0, 4)));
    assert_eq!(secs.iter().map(|f| f.bars).sum::<u32>(), 12);
    let doc = djlive::linked::pretty(&djlive::linked::jsonld(&c, &djlive::linked::Facts::default(), None));
    assert!(doc.contains(&c.cid), "the document names the set's own CID");
    assert_eq!(c.linked.euclids.len(), 0, "the euclid in a track is the track's: a set lists none");
}
