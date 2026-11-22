//! `template(name, phrase)` (template.rs, docs/BTW.md "Phrase templates"):
//! - the phrase's stress, syllables and meter make the vocal line: accents, length, subdivision (hand-made profiles);
//! - a template is deterministic: the same phrase is the same text and CID, another phrase is another;
//! - the reference plays as the BeatScript it is written out to, and that text is pinned (templates v1);
//! - `check`'s errors and notes, and `template` as a plain name.

mod common;

use common::{reformat, Rng};
use djlive::compile::{check_with, Check, Compiled};
use djlive::env::{Env, StaticVoices};
use djlive::template::{self, layout, Cell, Feel, Profile};

/// `src` checked as a host with the formant voices checks it.
fn check(src: &str) -> Check {
    let v = StaticVoices::default();
    check_with(src, &Env { voices: Some(&v), ..Env::default() })
}

fn compiled(src: &str) -> Compiled {
    let c = check(src);
    let errs: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| format!("{:?} {}", d.span, d.message)).collect();
    c.compiled.unwrap_or_else(|| panic!("{src}\n{errs:#?}"))
}

fn errors(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

fn warnings(src: &str) -> Vec<String> {
    check(src).diagnostics.iter().filter(|d| d.severity == djlive::Severity::Warning).map(|d| d.message.clone()).collect()
}

fn expanded(src: &str) -> String {
    let v = StaticVoices::default();
    match template::expand(src, &Env { voices: Some(&v), ..Env::default() }) {
        Some(Ok(e)) => e.text,
        other => panic!("{src}: {other:?}"),
    }
}

fn nets(c: &Compiled) -> Vec<(String, u32, u32, Vec<String>)> {
    c.segments.iter().map(|s| (s.name.clone(), s.start, s.bars, s.decks.values().map(|d| d.envelope(0).js_stringified()).collect::<Vec<_>>())).collect()
}

fn on(n: usize, stress: &[usize], feel: Feel) -> Profile {
    let mut s = vec![false; n];
    for i in stress {
        s[*i] = true;
    }
    Profile::new("la la la", s, feel)
}

fn text_of(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(|c| match c {
            Cell::Rest => "~".to_string(),
            Cell::Note(d, true) => format!("{d}!"),
            Cell::Note(d, false) => d.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------- the phrase makes the vocal line

/// The stress sets the accents and the beats: a stressed syllable is accented and sits on a beat; the unstressed ones
/// follow it on the eighths (iambic: with a pickup, so that the first stress is on a beat).
#[test]
fn stress_sets_the_accents_and_the_beats() {
    // trochaic: STRESS weak STRESS weak: the stresses on the beats 0, 4, 8, 12
    let l = layout(&on(4, &[0, 2], Feel::Straight)).unwrap();
    assert_eq!(l.steps, vec![0, 2, 4, 6]);
    let accents: Vec<bool> = l.cells[0].iter().filter_map(|c| if let Cell::Note(_, a) = c { Some(*a) } else { None }).collect();
    assert_eq!(accents, vec![true, false, true, false]);
    // iambic: a pickup, then the stress lands on the beat (step 4), not on the first note
    let l = layout(&on(4, &[1, 3], Feel::Straight)).unwrap();
    assert_eq!(l.steps, vec![2, 4, 6, 8]);
    assert!(l.steps.iter().enumerate().all(|(i, s)| (i % 2 == 1) == (s % 4 == 0)), "{:?}", l.steps);
    // two stresses in a row each get a beat: a rest is left between them
    let l = layout(&on(3, &[0, 1, 2], Feel::Straight)).unwrap();
    assert_eq!(l.steps, vec![0, 4, 8]);
    // an odd run of unstressed syllables leaves an eighth of rest before the next stress
    let l = layout(&on(5, &[0, 4], Feel::Straight)).unwrap();
    assert_eq!(l.steps, vec![0, 2, 4, 6, 8]);
    let l = layout(&on(4, &[0, 3], Feel::Straight)).unwrap();
    assert_eq!(l.steps, vec![0, 2, 4, 8]);
    // every accent is on a stressed syllable and nowhere else
    assert_eq!(l.cells[0].iter().filter(|c| matches!(c, Cell::Note(_, true))).count(), 2);
}

/// The syllables set the vocal line's length: one to an eighth note, the line ending a beat before its cycle.
#[test]
fn syllables_set_the_length() {
    let bars = |n: usize| layout(&on(n, &[0], Feel::Straight)).map(|l| l.cycles);
    assert_eq!(bars(1), Ok(1));
    assert_eq!(bars(6), Ok(1));
    assert_eq!(bars(7), Ok(2));
    assert_eq!(bars(14), Ok(2));
    assert_eq!(bars(15), Ok(4));
    assert_eq!(bars(20), Ok(4));
    assert_eq!(bars(29), Ok(4));
    let long = bars(40).unwrap_err();
    assert!(long.contains("at most"), "{long}");
    // the cycles with a note in them are the line's lets; the breath at its end is a rest in the part
    let l = layout(&on(20, &[0, 4, 8, 12, 16], Feel::Straight)).unwrap();
    assert_eq!((l.cycles, l.used), (4, 3));
    assert_eq!(l.lets().lines().count(), 3);
    assert_eq!(l.part(), "<vox_a vox_b vox_c ~>");
    assert_eq!(l.length(), "4 bars");
    let l = layout(&on(5, &[0], Feel::Straight)).unwrap();
    assert_eq!(l.part(), "vox_a");
    // in the triplet feel a cycle is twelve steps, so the same syllables take more of them
    let t = |n: usize| layout(&on(n, &[0], Feel::Triplet)).map(|l| l.cycles);
    assert_eq!(t(3), Ok(1));
    assert_eq!(t(4), Ok(2));
    assert_eq!(t(9), Ok(2));
    assert_eq!(t(10), Ok(4));
    assert_eq!(t(21), Ok(4));
    assert_eq!(t(22), Ok(8));
    assert_eq!(t(45), Ok(8));
    assert!(t(46).is_err());
}

/// The meter sets the subdivision: straight is the sixteenth grid (a bar of eight eighths); the triplet feel is a cycle of
/// twelve sixteenths, six eighths, three syllables to the beat.
#[test]
fn meter_sets_the_subdivision() {
    let s = layout(&on(6, &[0, 3], Feel::Straight)).unwrap();
    assert_eq!(s.cells[0].len(), 8);
    assert_eq!(s.cycle_mark(), "");
    // a dactyl: S w w S w w: the stresses on the beats of six steps, the weak syllables on the eighths between
    let t = layout(&on(6, &[0, 3], Feel::Triplet)).unwrap();
    assert_eq!(t.cells[0].len(), 6);
    assert_eq!(t.cycle_mark(), "/12");
    assert_eq!(t.steps, vec![0, 2, 4, 6, 8, 10]);
    assert_eq!(text_of(&t.cells[0]).split(' ').map(|w| w.ends_with('!')).collect::<Vec<_>>(), vec![true, false, false, true, false, false]);
    // an anapest: w w S: a pickup of two, the first stress on the beat at six
    let a = layout(&on(6, &[2, 5], Feel::Triplet)).unwrap();
    assert_eq!(a.steps, vec![2, 4, 6, 8, 10, 12]);
    assert_eq!(a.length(), "1 1/2 bars");
    // a line sits in its cycles: the 12-step ones are three bars to come round at most
    assert_eq!(layout(&on(10, &[0], Feel::Triplet)).unwrap().length(), "3 bars");
}

/// The notes: stressed syllables on chord tones, unstressed on passing degrees, the line ending on the root, the fifth or the
/// octave; and the contour is a hash of the words, so different words are different lines.
#[test]
fn notes_are_chord_tones_on_the_stress_and_passing_between() {
    let p = Profile::new("we are such stuff as dreams are made on and our little life", vec![false, true, false, true, false, true, false, true, false, false, false, true, false, true], Feel::Straight);
    let l = layout(&p).unwrap();
    let notes: Vec<(u8, bool)> = l.cells.iter().flatten().filter_map(|c| if let Cell::Note(d, a) = c { Some((*d, *a)) } else { None }).collect();
    assert_eq!(notes.len(), 14);
    let n = notes.len();
    for (i, (d, accent)) in notes.iter().enumerate() {
        assert!((1..=8).contains(d), "{d}");
        if i == n - 1 {
            assert!([1, 5, 8].contains(d), "the last syllable rests on the chord: {d}");
        } else if *accent {
            assert!([1, 3, 5, 8].contains(d), "stressed: {d}");
        } else {
            assert!([2, 4, 6, 7].contains(d), "unstressed: {d}");
        }
    }
    let lines: std::collections::BTreeSet<String> = ["robots on the road", "a different set of words", "feel the bass", "one more round", "the night is young", "hands up", "turn it up", "lights down"]
        .iter()
        .map(|w| layout(&Profile::new(w, p.stress.clone(), Feel::Straight)).unwrap().lets())
        .collect();
    assert!(lines.len() >= 3, "the words pick the contour: {lines:?}");
    // the same profile is always the same line
    assert_eq!(layout(&p).unwrap(), l);
}

/// A phrase with no stress at all still has a first beat.
#[test]
fn a_phrase_always_has_a_stress() {
    let p = Profile::new("x", vec![false, false, false], Feel::Straight);
    assert_eq!(p.stress, vec![true, false, false]);
}

// ---------------------------------------------------------------- the phrase's profile

/// Any text: function words are unstressed, the first syllable of any other word is stressed, and the feel is straight
/// unless the stresses fall three syllables apart.
#[test]
fn a_text_has_a_worked_out_profile() {
    let p = Profile::from_text("we can dance in the light of the night", None).unwrap();
    assert_eq!(p.stress_text(), "~~x~~x~~x");
    assert_eq!((p.meter, p.feel), ("anapestic", Feel::Triplet));
    assert!(!p.curated);
    let p = Profile::from_text("make the drop deeper and move the kick", None).unwrap();
    assert_eq!(p.feel, Feel::Straight);
    assert_eq!(p.stress_text(), "x~xx~~x~x");
    let d = Profile::from_text("everyone beautiful energy forever", None).unwrap();
    assert_eq!((d.meter, d.feel), ("dactylic", Feel::Triplet));
    // the lyric is the words, and whitespace is one space
    assert_eq!(Profile::from_text("  robots\n on   the road ", None).unwrap().lyric, "robots on the road");
    assert!(Profile::from_text("", None).is_err());
    assert!(Profile::from_text(" ... ", None).is_err());
}

/// A curated profile (the phrasebook's): its stress and feel as given, padded or cut to the syllables the voice sings.
#[test]
fn a_curated_profile_is_taken_as_given() {
    let p = Profile::curated("we are such stuff", "~x~x", Feel::Straight, None).unwrap();
    assert_eq!(p.stress_text(), "~x~x");
    assert_eq!(p.meter, "iambic");
    assert!(p.curated);
    // short: padded with unstressed; long: cut
    assert_eq!(Profile::curated("we are such stuff", "~x", Feel::Straight, None).unwrap().stress_text(), "~x~~");
    assert_eq!(Profile::curated("we are", "~x~x~x", Feel::Straight, None).unwrap().stress_text(), "~x");
}

// ---------------------------------------------------------------- the track

const STRAIGHT: &str = "arrange from template(deep-house, \"make the drop deeper and move the kick\")\n";
const TRIPLET: &str = "arrange from template(deep-house, \"we can dance in the light of the night\")\n";

/// The same template and phrase are the same text and CID, every time; another phrase is another.
#[test]
fn a_template_is_deterministic() {
    let a = compiled(STRAIGHT);
    let b = compiled(STRAIGHT);
    assert_eq!((a.cid.clone(), a.text.clone()), (b.cid.clone(), b.text));
    assert_eq!(expanded(STRAIGHT), expanded(STRAIGHT));
    assert_ne!(a.cid, compiled(TRIPLET).cid);
    assert_ne!(expanded(STRAIGHT), expanded(TRIPLET));
    // the CID is of the reference, not of what it stands for
    let e = compiled(&expanded(STRAIGHT));
    assert_ne!(a.cid, e.cid);
    assert!(a.text.contains("template(deep-house, "), "{}", a.text);
    // and the written-out text has no reference left
    assert!(template::expand(&expanded(STRAIGHT), &Env::none()).is_none());
}

/// The reference is spelt one way: spacing and a phrasebook id or its words are the program's text, and so its CID.
#[test]
fn a_reference_is_spelt_one_way() {
    let a = compiled("arrange from template(deep-house, \"make the drop deeper\")\n");
    let b = compiled("arrange   from template( deep-house ,\"make the drop deeper\" )\n");
    assert_eq!(a.text, "arrange from template(deep-house, \"make the drop deeper\")\n");
    assert_eq!(a.cid, b.cid);
    assert_ne!(a.cid, compiled("arrange from template(deep-house, \"make the drop deeper!\")\n").cid);
    let c = compiled("arrange from template(deep-house, @hands_up)\n");
    assert_eq!(c.text, "arrange from template(deep-house, @hands_up)\n");
    // a quote in the words is escaped, and the text is the same program again
    let q = compiled("arrange from template(deep-house, \"say \\\"hey\\\" to the beat\")\n");
    assert_eq!(compiled(&q.text).cid, q.cid);
}

/// The reference plays as the BeatScript it is written out to: the same nets, tempo, key and ending.
#[test]
fn a_template_plays_as_its_expansion() {
    for src in [STRAIGHT, TRIPLET, "arrange from template(deep-house, @hands_up)\n", "tempo 118\nkey dm\narrange from template(deep-house, \"turn it up\")\n"] {
        let c = compiled(src);
        let e = compiled(&expanded(src));
        assert_eq!(nets(&c), nets(&e), "{src}: the nets differ");
        assert_eq!((c.tempo, c.key, c.end, c.looped), (e.tempo, e.key, e.end, e.looped));
        let again = compiled(&c.text);
        assert_eq!(again.cid, c.cid);
        assert_eq!(nets(&again), nets(&c));
    }
}

/// The program's own `tempo`, `key` and `swing` win over the template's.
#[test]
fn the_programs_own_settings_win() {
    let own = compiled("tempo 118\nkey dm\nswing 0.3\narrange from template(deep-house, \"turn it up\")\n");
    assert_eq!(own.tempo, Some(118.0));
    assert_eq!(own.swing, Some(0.3));
    assert_eq!(own.key.map(|k| k.tonic), Some(2));
    let text = expanded("tempo 118\nkey dm\nswing 0.3\narrange from template(deep-house, \"turn it up\")\n");
    // (each setting once: the program's own, not the template's)
    let settings = text.lines().filter(|l| ["tempo ", "key ", "swing "].iter().any(|k| l.starts_with(k))).collect::<Vec<_>>();
    assert_eq!(settings, vec!["tempo 118", "key dm", "swing 0.3"], "{text}");
    let plain = compiled("arrange from template(deep-house, \"turn it up\")\n");
    assert_eq!((plain.tempo, plain.swing), (Some(122.0), Some(0.12)));
}

/// The arrangement is the template's own, whatever the phrase: night and spot, four passes, the voice an octave up and
/// crossed onto the shaker, and `kick4` outside the `drm_*` glob.
#[test]
fn the_arrangement_is_the_templates_own() {
    for src in [STRAIGHT, TRIPLET] {
        let text = expanded(src);
        let arrange = text.split("arrange {").nth(1).expect("an arrange block");
        assert!(arrange.contains("@ vox_* oct 1") && arrange.contains("@ vox_* xfer drm_shk") && arrange.contains("~ drm_* "), "{arrange}");
        assert!(!arrange.contains("kick4"), "{arrange}");
        assert!(arrange.trim_end().ends_with("} loop 4 then end"), "{arrange}");
        let c = compiled(src);
        // four passes of night, spot, night, spot
        assert_eq!(c.segments.len(), 16, "{src}");
    }
    // the sections are as long as the vocal cycles make them come round: 8 bars straight, 12 for the 12-step cycle
    assert_eq!(compiled(STRAIGHT).segments[0].bars, 8);
    assert_eq!(compiled(TRIPLET).segments[0].bars, 12);
}

/// The vocal line is as long as the phrase says, and every syllable has a note: no phrase leaves its words unsung.
#[test]
fn every_syllable_has_a_note() {
    for text in [
        "turn it up",
        "make the drop deeper and move the kick",
        "we can dance in the light of the night",
        "everyone beautiful energy forever",
        "hey",
        "put your hands up everybody move to the music now and feel the bass drop",
    ] {
        let src = format!("arrange from template(deep-house, \"{text}\")\n");
        let w = warnings(&src);
        assert!(w.is_empty(), "{text}: {w:?}");
    }
}

/// Every line of the public-domain phrasebook is a phrase a template takes: it lays out, its notes are as many as its
/// syllables, and its feel is the book's own (the anapests and dactyls are the 12-step cycle, the iambs and trochees the bar).
#[test]
fn every_public_domain_line_is_a_phrase() {
    let lines = djengine::vocal::phoneme::pd_phrases();
    assert!(lines.len() >= 40);
    for p in lines {
        let src = format!("arrange from template(deep-house, @{})\n", p.id);
        let w = warnings(&src);
        assert!(w.is_empty(), "{}: {w:?}", p.id);
        let text = expanded(&src);
        let triplet = p.feel == "triplet";
        assert_eq!(text.contains("melody/12"), triplet, "{}: {}", p.id, p.feel);
        assert_eq!(text.contains("hihat/12"), triplet, "{}: {}", p.id, p.feel);
        assert!(text.contains(&format!("section night {} bars", if triplet { 12 } else { 8 })), "{}", p.id);
        // one note for each syllable, one accent for each stressed one, whatever the book writes
        let notes = text.lines().filter(|l| l.starts_with("let vox_")).flat_map(|l| l.split(" = ").nth(1).unwrap_or("").split(' ').map(str::to_string).collect::<Vec<_>>()).filter(|t| t != "~").count();
        assert_eq!(notes, p.syllables, "{}: {text}", p.id);
        let accents = text.lines().filter(|l| l.starts_with("let vox_")).map(|l| l.matches('!').count()).sum::<usize>();
        assert_eq!(accents, p.accents().iter().filter(|a| **a).count(), "{}", p.id);
        let note = check(&src).diagnostics.iter().find(|d| d.message.starts_with("template deep-house:")).map(|d| d.message.clone()).unwrap_or_default();
        assert!(note.contains(&format!("{} syllables", p.syllables)) && note.contains(&format!("{} feel", p.feel)) && !note.contains("worked out"), "{}: {note}", p.id);
    }
}

/// An anapestic line is the 12-cycle feel and an iambic one the bar: Byron's Assyrian and Shakespeare's such stuff.
#[test]
fn an_anapest_is_triplet_and_an_iamb_is_straight() {
    let wolf = expanded("arrange from template(deep-house, @byron_assyrian_wolf)\n");
    assert!(wolf.contains("melody/12 <vox_a vox_b vox_c ~>") && wolf.contains("hihat/12 drm_shk") && wolf.contains("let drm_shk = x ~ x x ~ x\n"), "{wolf}");
    assert!(wolf.contains("# phrase: @byron_assyrian_wolf, \"The Assyrian came down like the wolf on the fold,\" (The Destruction of Semnacherib"), "{wolf}");
    let stuff = expanded("arrange from template(deep-house, @tempest_such_stuff)\n");
    assert!(!stuff.contains("/12") && stuff.contains("melody <vox_a vox_b vox_c ~>"), "{stuff}");
    assert!(stuff.contains("(The Tempest, 4.1.156-158, William Shakespeare, 1623)"), "{stuff}");
    // the stress is the book's: the iambs have a pickup, so the line starts an eighth in, and every stress is on a beat
    let lets = stuff.lines().filter(|l| l.starts_with("let vox_")).collect::<Vec<_>>();
    assert!(lets[0].starts_with("let vox_a = ~ "), "{lets:?}");
    // the phrase is its words: another id is another vocal line
    assert_ne!(stuff, expanded("arrange from template(deep-house, @sonnet18_summers_day)\n"));
}

/// Any phrase that fits a line checks with no warning: no syllable is left unsung, in any generation of the arrangement (the
/// one-shot that crosses the voice onto the shaker included). Eighty phrases of lexicon words, of 1 to 28 words each.
#[test]
fn any_phrase_checks_clean() {
    let words: Vec<String> = djengine::vocal::phoneme::lexicon_entries().into_iter().map(|e| e.0).filter(|w| w.chars().all(|c| c.is_ascii_lowercase())).collect();
    let mut rng = Rng(0x0dd5_eed5_0000_0001);
    let (mut fitted, mut too_long) = (0, 0);
    for _ in 0..80 {
        let n = 1 + rng.below(28) as usize;
        let phrase: Vec<&str> = (0..n).map(|_| words[rng.below(words.len() as u64) as usize].as_str()).collect();
        let src = format!("arrange from template(deep-house, \"{}\")\n", phrase.join(" "));
        let c = check(&src);
        if let Some(d) = c.diagnostics.iter().find(|d| d.is_error()) {
            assert!(d.message.contains("a vocal line is at most"), "{src}: {}", d.message);
            too_long += 1;
            continue;
        }
        fitted += 1;
        let w: Vec<String> = c.diagnostics.iter().filter(|d| d.severity == djlive::Severity::Warning).map(|d| d.message.clone()).collect();
        assert!(w.is_empty(), "{src}: {w:?}");
    }
    assert!(fitted >= 30 && too_long >= 3, "{fitted} fitted, {too_long} too long");
}

/// A phrase from the phrasebook is a phrase like any other (its words and its stress).
#[test]
fn a_phrasebook_id_is_a_phrase() {
    let src = "arrange from template(deep-house, @hands_up)\n";
    assert!(warnings(src).is_empty(), "{:?}", warnings(src));
    let text = expanded(src);
    assert!(text.contains("# phrase: @hands_up, \"put your hands up\""), "{text}");
    assert_ne!(compiled(src).cid, compiled("arrange from template(deep-house, @feel_the_bass)\n").cid);
}

/// The templates v1 are frozen: the written-out program of a template and a phrase is pinned (tests/fixtures/template/).
/// A change to a template that changed any track is templates v2.
#[test]
fn templates_v1_are_pinned() {
    for (name, src) in [
        ("deep-house-straight", STRAIGHT),
        ("deep-house-triplet", TRIPLET),
        ("deep-house-tempest", "arrange from template(deep-house, @tempest_such_stuff)\n"),
        ("deep-house-assyrian", "arrange from template(deep-house, @byron_assyrian_wolf)\n"),
        ("form-quodlibet", "arrange from template(quodlibet, @psalm42_deep_calleth)\n"),
        ("form-passacaglia", "arrange from template(passacaglia, @eccl3_season)\n"),
        ("form-canon", "arrange from template(canon, @mgm_pease_porridge_hot)\n"),
        ("form-rondo", "arrange from template(rondo, @shanty_drunken_sailor)\n"),
        ("form-fugue", "arrange from template(fugue, @tyger_burning_bright)\n"),
        ("form-theme-and-variations", "arrange from template(theme-and-variations, @dickinson_chariot_death)\n"),
        ("form-rondo-triplet", "arrange from template(rondo, @byron_assyrian_wolf)\n"),
    ] {
        let path = format!("{}/tests/fixtures/template/{name}.btw", env!("CARGO_MANIFEST_DIR"));
        let got = expanded(src);
        if std::env::var("BTW_BLESS").is_ok() {
            std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no golden {path} (BTW_BLESS=1 writes it)"));
        assert_eq!(got, want, "templates {} changed {name}: that is templates v2 (BTW_BLESS=1 rewrites the golden)", template::VERSION);
    }
}

/// The written-out program checked on its own is the same song as a hand-written longhand: the lets and sections typed out.
#[test]
fn the_expansion_is_the_longhand() {
    let src = "arrange from template(deep-house, \"turn it up\")\n";
    let long = "\
tempo 122
key am
swing 0.12
let kick4 = x x x x
let clap24 = ~ x ~ x
let drm_hat = ~ x ~ x ~ x ~ x
let drm_shk = x ~ x x ~ x x ~
let drm_rim = ~ x ~ ~ ~ x ~ ~
let bass_a = 1 ~ ~ 1 ~ ~ 8 ~ 1 ~ ~ 5 ~ 1 ~ ~
let bass_f = 6 ~ ~ 6 ~ ~ 13 ~ 6 ~ ~ 10 ~ 6 ~ ~
let bass_d = 4 ~ ~ 4 ~ ~ 11 ~ 4 ~ ~ 8 ~ 4 ~ ~
let bass_g = 7 ~ ~ 7 ~ ~ 14 ~ 7 ~ ~ 11 ~ 7 ~ ~
let vox_a = 8! 4 1 ~ ~ ~ ~ ~
section night 8 bars {
  deck A: kick kick4 | clap clap24 | hihat drm_hat | bass <bass_a bass_f bass_d bass_g> inst=sub-bass
  deck B: harmony <i VI iv VII> inst=warm-pad len=16 vel=28..42 | melody vox_a inst=kalimba oct=3 vel=24..38
  deck B: sing \"turn it up\" on melody voice=robot-choir
  duck B under A.kick 3db attack=5ms release=120ms
  mix { B level 0.45, B eq high -9 }
}
section spot 8 bars {
  deck A: kick kick4 | clap clap24 | hihat drm_shk | snare drm_rim | bass <bass_a bass_f bass_d bass_g> inst=sub-bass
  deck B: harmony <i VI iv VII> inst=warm-pad len=16 vel=4..10 | melody vox_a inst=kalimba oct=3 vel=10..18
  deck B: sing \"turn it up\" on melody voice=robot-choir
  duck B under A.kick 3db attack=5ms release=120ms
  mix { B level 0.95 }
}
arrange {
  night
  @ vox_* oct 1
  spot
  ~ drm_* fill 0.15
  night
  @ vox_* xfer drm_shk
  spot
  ~ drm_* grow
} loop 4 then end
";
    let a = compiled(src);
    let b = compiled(long);
    assert_eq!(nets(&a), nets(&b), "the template's nets are not the longhand's");
    assert_eq!(compiled(&expanded(src)).cid, b.cid);
}

// ---------------------------------------------------------------- check says

/// check says what the phrase gave, in a note.
#[test]
fn check_says_what_the_phrase_gave() {
    let c = check(TRIPLET);
    let note = c.diagnostics.iter().find(|d| d.message.starts_with("template deep-house:")).expect("a note");
    assert_eq!(note.severity, djlive::Severity::Info);
    assert!(note.message.contains("9 syllables") && note.message.contains("anapestic") && note.message.contains("triplet feel") && note.message.contains("12 bars"), "{}", note.message);
    assert!(note.message.contains("worked out from the words"), "{}", note.message);
    // a phrasebook phrase's stress is the book's own
    let c = check("arrange from template(deep-house, @hands_up)\n");
    let note = c.diagnostics.iter().find(|d| d.message.starts_with("template deep-house:")).expect("a note");
    assert!(!note.message.contains("worked out"), "{}", note.message);
    // the words are the program's own (the check of a template points at the reference, not into its text)
    assert!(c.diagnostics.iter().all(|d| d.span.end <= "arrange from template(deep-house, @hands_up)".len()));
}

/// A word the lexicon does not know is the usual lexicon warning, at the phrase.
#[test]
fn an_unknown_word_is_the_lexicon_warning() {
    let w = warnings("arrange from template(deep-house, \"quixotic zeppelin\")\n");
    assert!(w.iter().any(|m| m.contains("not in the BeatScript lexicon")), "{w:?}");
}

#[test]
fn an_unknown_template_lists_the_known_ones() {
    let src = "arrange from template(deep-hose, \"turn it up\")\n";
    let c = check(src);
    let d = c.diagnostics.iter().find(|d| d.is_error()).expect("an error");
    assert!(d.message.contains("`deep-hose` is not a template"), "{}", d.message);
    let help = d.help.clone().unwrap_or_default();
    assert!(help.contains("did you mean `deep-house`") && help.contains("The templates: deep-house"), "{help}");
    // at the name
    assert_eq!(d.span.text(src), "deep-hose");
}

#[test]
fn an_empty_phrase_says_what_to_write() {
    for p in ["\"\"", "\"  \"", "\"...\""] {
        let src = format!("arrange from template(deep-house, {p})\n");
        let c = check(&src);
        let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("{p}: an error"));
        assert!(d.message.contains("nothing to sing"), "{}", d.message);
        assert!(d.help.clone().unwrap_or_default().contains("template(deep-house, \""), "{:?}", d.help);
    }
}

#[test]
fn an_unknown_phrase_id_and_a_long_phrase_are_errors() {
    let e = errors("arrange from template(deep-house, @hands_upp)\n");
    assert!(e.iter().any(|m| m.contains("`@hands_upp` is not a phrase of the phrasebook")), "{e:?}");
    let long = "turn it up ".repeat(20);
    let e = errors(&format!("arrange from template(deep-house, \"{long}\")\n"));
    assert!(e.iter().any(|m| m.contains("a vocal line is at most")), "{e:?}");
}

#[test]
fn a_template_goes_after_arrange_from_and_once() {
    let e = errors("section a 2 bars {\n  deck A: kick x\n}\nsection verse from template(deep-house, \"turn it up\")\narrange verse\n");
    assert!(e.iter().any(|m| m.contains("goes after `arrange from`")), "{e:?}");
    let e = errors("let riff = template(deep-house, \"turn it up\")\narrange from template(deep-house, \"turn it up\")\n");
    assert!(!e.is_empty());
    let e = errors("arrange from template(deep-house, \"turn it up\")\narrange from template(deep-house, \"feel the bass\")\n");
    assert!(e.iter().any(|m| m.contains("one `arrange from`")), "{e:?}");
    let e = errors("arrange from template(deep-house, \"turn it up\")\narrange from seed(techno, 42)\n");
    assert!(e.iter().any(|m| m.contains("one `arrange from`")), "{e:?}");
    // a malformed reference is one error, at the reference
    for bad in ["arrange from template(deep-house)\n", "arrange from template(, \"x\")\n", "arrange from template(deep-house, 42)\n", "arrange from template(deep-house, \"x\"\n"] {
        let e = errors(bad);
        assert_eq!(e.len(), 1, "{bad}: {e:?}");
    }
}

#[test]
fn the_version_marker_is_checked() {
    assert!(errors("#! templates: v1\narrange from template(deep-house, \"turn it up\")\n").is_empty());
    let e = errors("#! templates: v2\narrange from template(deep-house, \"turn it up\")\n");
    assert!(e.iter().any(|m| m.contains("templates v1, not `v2`")), "{e:?}");
    // the marker is a comment: the same CID with or without it
    assert_eq!(compiled("#! templates: v1\narrange from template(deep-house, \"turn it up\")\n").cid, compiled("arrange from template(deep-house, \"turn it up\")\n").cid);
}

// ---------------------------------------------------------------- the word `template`

/// `template` is not a reserved word: a let, a def and its call and a section may be named so, and they check as they did
/// (the CIDs are those of the build before templates).
#[test]
fn template_is_still_a_name() {
    let let_ = "tempo 120\nlet template = x ~ x ~\nsection verse 2 bars {\n  deck A: kick template\n}\narrange verse\n";
    let def = "tempo 120\ndef template(r) {\n  deck A: kick $r\n}\nsection verse 2 bars {\n  template(x x x x)\n  deck B: melody c4 e4\n}\narrange verse\n";
    let section = "tempo 120\nsection template 2 bars {\n  deck A: kick x x x x\n}\nsection outro 2 bars {\n  deck A: kick x ~ x ~\n}\narrange template outro\n";
    assert_eq!(compiled(let_).cid, "z4EBG9jFfCT7URMYJqG89bTrR3Jb9hA9AvCymTz7Hz3rqw1iDfG");
    assert_eq!(compiled(def).cid, "z4EBG9jFtRLMoGAJWmXyJncuXHi5dJBVfmZnbT7Tto1mzabpKpz");
    assert_eq!(compiled(section).cid, "z4EBG9j54JxxjoXYySXLYYriDMbxCZP2rErhfhzHd8TdUqv4Zca");
}

/// A template program is a fixed point and keeps its CID through 40 reformattings.
#[test]
fn a_template_program_is_canonical_and_reformat_stable() {
    for src in [STRAIGHT, TRIPLET, "#! templates: v1\ntempo 118\narrange from template(deep-house, @hands_up)\n"] {
        let c = compiled(src);
        let again = compiled(&c.text);
        assert_eq!((again.text.clone(), again.cid.clone()), (c.text.clone(), c.cid.clone()));
        let mut rng = Rng(0x7e47_1a7e_0000_0000);
        for round in 0..40 {
            let text = reformat(src, &mut rng);
            let r = compiled(&text);
            assert_eq!(r.cid, c.cid, "round {round}:\n{text}");
        }
    }
}

/// The registry: the names and each template's example is a program that checks.
#[test]
fn every_template_has_a_checking_example() {
    assert_eq!(template::names(), ["deep-house", "quodlibet", "passacaglia", "canon", "rondo", "fugue", "theme-and-variations"].map(String::from).to_vec());
    for t in &template::TEMPLATES {
        assert!(!t.summary.is_empty());
        let w = warnings(&format!("{}\n", t.example));
        assert!(w.is_empty(), "{}: {w:?}", t.name);
        compiled(&format!("{}\n", t.example));
    }
    assert!(template::find("deep-house").is_some() && template::find("nope").is_none());
}

/// Each template's example programs are in `examples/btw/`, use that template, and say what you hear (a comment line).
#[test]
fn a_templates_examples_are_programs_that_use_it() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    for t in template::TEMPLATES.iter().filter(|t| t.form.is_none()) {
        assert!(t.examples.len() >= 3, "{}", t.name);
        for name in t.examples {
            let src = std::fs::read_to_string(dir.join(format!("{name}.btw"))).unwrap_or_else(|_| panic!("examples/btw/{name}.btw"));
            assert!(src.contains(&format!("arrange from template({}, ", t.name)), "{name}");
            assert!(src.lines().filter(|l| l.starts_with("# ")).count() >= 3, "{name} says what it is and what you hear");
            let w = warnings(&src);
            assert!(w.is_empty(), "{name}: {w:?}");
        }
    }
    // the three of deep-house: an iambic line, an anapestic one (the triplet feel), and any words
    let feel = |name: &str| expanded(&std::fs::read_to_string(dir.join(format!("{name}.btw"))).unwrap()).contains("melody/12");
    assert_eq!((feel("such-stuff-as-dreams-122"), feel("assyrian-122"), feel("commit-message-122")), (false, true, false));
}

// ---------------------------------------------------------------- the forms (docs/btw/forms.md)

/// The arrange block of a written-out program: the lines between `arrange {` and its `}`.
fn arrange_lines(text: &str) -> Vec<String> {
    text.lines().skip_while(|l| !l.starts_with("arrange {")).skip(1).take_while(|l| !l.starts_with('}')).map(|l| l.trim().to_string()).collect()
}

/// Every form takes every line of the phrasebook: it checks with no warning (no syllable unsung in any generation, the
/// one-shot that crosses the voice onto the shaker included), the line is sung in the form's own sung section, and the
/// sections are the vocal's (8 bars, 12 in the triplet feel).
#[test]
fn every_form_takes_every_public_domain_line() {
    for (t, _) in template::forms() {
        for p in djengine::vocal::phoneme::pd_phrases() {
            let src = format!("arrange from template({}, @{})\n", t.name, p.id);
            let c = check(&src);
            let bad: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error() || d.severity == djlive::Severity::Warning).map(|d| d.message.clone()).collect();
            assert!(bad.is_empty() && c.compiled.is_some(), "{} @{}: {bad:?}", t.name, p.id);
            let text = expanded(&src);
            assert_eq!(text.matches("sing \"").count(), 1, "{} @{}: one sung section", t.name, p.id);
            let bars = if p.feel == "triplet" { 12 } else { 8 };
            assert!(text.lines().any(|l| l.starts_with("section ") && l.ends_with(&format!(" {bars} bars {{")) && (l.contains(" sung ") || l.contains(" voices "))), "{} @{}", t.name, p.id);
        }
    }
}

/// Any words a form takes check clean too (the forms' twin of `any_phrase_checks_clean`).
#[test]
fn any_words_in_a_form_check_clean() {
    let words: Vec<String> = djengine::vocal::phoneme::lexicon_entries().into_iter().map(|e| e.0).filter(|w| w.chars().all(|c| c.is_ascii_lowercase())).collect();
    let mut rng = Rng(0xf0f0_5eed_0000_0002);
    let mut fitted = 0;
    for (t, _) in template::forms() {
        for _ in 0..8 {
            let n = 1 + rng.below(20) as usize;
            let phrase: Vec<&str> = (0..n).map(|_| words[rng.below(words.len() as u64) as usize].as_str()).collect();
            let src = format!("arrange from template({}, \"{}\")\n", t.name, phrase.join(" "));
            let c = check(&src);
            if let Some(d) = c.diagnostics.iter().find(|d| d.is_error()) {
                assert!(d.message.contains("a vocal line is at most"), "{src}: {}", d.message);
                continue;
            }
            fitted += 1;
            let w: Vec<String> = c.diagnostics.iter().filter(|d| d.severity == djlive::Severity::Warning).map(|d| d.message.clone()).collect();
            assert!(w.is_empty(), "{src}: {w:?}");
        }
    }
    assert!(fitted >= 30, "{fitted}");
}

/// A form says what it is, and its template is that: every mutation operator its plan names is used by its arrangement and
/// every one its arrangement uses is named; the themes it names are the theme slots it fills, each a theme of the library;
/// the vocal is never thinned, decayed or morphed (no syllable is lost), and the themes are its ground.
#[test]
fn a_form_is_what_its_table_says() {
    let ops = djlive::mutate::operator_names();
    for (t, f) in template::forms() {
        assert!(!f.label.is_empty() && !f.shape.is_empty() && !f.plan.is_empty(), "{}", t.name);
        let text = expanded(&format!("{}\n", t.example));
        let lines = arrange_lines(&text);
        assert!(!lines.is_empty(), "{}", t.name);
        let mut used = std::collections::BTreeSet::new();
        for l in lines.iter().filter(|l| l.starts_with('~') || l.starts_with('@')) {
            for w in l.split(|c: char| c.is_whitespace() || c == '|') {
                if ops.iter().any(|o| *o == w) {
                    used.insert(w.to_string());
                }
            }
            if l.contains("vox_") {
                assert!(!["thin", "decay", "morph", "mask", "euclid"].iter().any(|o| l.contains(o)), "{}: the vocal loses notes: {l}", t.name);
            }
        }
        let named: std::collections::BTreeSet<String> = f.mutations.iter().map(|m| m.to_string()).collect();
        assert_eq!(used, named, "{}: the operators its arrange uses and the ones its table names", t.name);
        let slots: std::collections::BTreeSet<String> = t.text().split('$').skip(1).filter_map(|s| s.strip_prefix("THEME_")).map(|s| s.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect::<String>().to_ascii_lowercase()).collect();
        let themes: std::collections::BTreeSet<String> = f.themes.iter().map(|m| m.to_string()).collect();
        assert_eq!(slots, themes, "{}: the theme slots and the themes its table names", t.name);
        for th in f.themes {
            let th = djengine::themes::theme(th).unwrap_or_else(|| panic!("{}: {th} is no theme", t.name));
            assert!(text.contains(&th.lets()), "{}: {} is written out whole", t.name, th.id);
        }
    }
}

/// A form's reference program is a whole hand-written track in `examples/btw/` whose text is the CID the table names.
#[test]
fn a_forms_reference_is_the_program_it_names() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    let mut n = 0;
    for (t, f) in template::forms() {
        let Some((name, cid)) = f.reference else { continue };
        let src = std::fs::read_to_string(dir.join(format!("{name}.btw"))).unwrap_or_else(|_| panic!("{}: examples/btw/{name}.btw", t.name));
        assert_eq!(compiled(&src).cid, cid, "{}: {name}", t.name);
        // a check-clean holdout: no warning, but for the one the program was written with (its code is never edited, so an
        // escape for the words cannot be added; the day the lexicon has them, this says so)
        let w = warnings(&src);
        let known: &[&str] = if name == "full-fathom-five" { &["not in the BeatScript lexicon, sounded out by the rules: `full` /f uh l/, `five` /f ie v/"] } else { &[] };
        assert_eq!(w, known, "{name}: its warnings moved (update the holdout and the comment in the file)");
        // and it carries its source: in its comments, and as linked data beside it (isBasedOn the public-domain works)
        assert!(src.lines().filter(|l| l.starts_with("# ")).any(|l| l.contains("public domain") || l.contains("public-domain")), "{name} says where its themes are from");
        let ld = djengine::json::JsonValue::parse(&std::fs::read_to_string(dir.join(format!("{name}.jsonld"))).unwrap_or_else(|_| panic!("examples/btw/{name}.jsonld"))).unwrap();
        assert_eq!(ld.get("identifier").and_then(|v| v.as_str()), Some(cid), "{name}.jsonld");
        assert_eq!(ld.get("genre").and_then(|v| v.as_str()), Some(format!("https://beatscript.app/ontology/concept/form-{}", t.name).as_str()));
        let based = ld.get("isBasedOn").and_then(|v| v.as_array()).unwrap_or_else(|| panic!("{name}.jsonld has no isBasedOn"));
        assert!(!based.is_empty());
        for w in based {
            let year: u32 = w.get("dateCreated").and_then(|v| v.as_str()).and_then(|y| y.parse().ok()).unwrap_or(9999);
            assert!(year < 1929, "{name}: a source of {year}");
        }
        n += 1;
    }
    assert_eq!(n, 3);
}

/// The two reference programs of docs/btw/found-phrases.md are whole hand-written tracks in `examples/btw/`: each is the text
/// whose CID the page names, with its source in a comment and beside it as linked data. Their lexicon warnings are pinned (the
/// programs are as written, so an escape cannot be added; the day the lexicon has the words, this says so).
#[test]
fn the_found_phrases_reference_programs_are_the_programs_the_page_names() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    let found: &[(&str, &str, &[&str])] = &[
        ("such-stuff-as-dreams-122-reference", "z4EBG9jAv9C8XQ55DMd9iGXybFcMbX93mrXjwrNLVLuKUe5p1QK", &[
            "not in the BeatScript lexicon, sounded out by the rules: `such` /s uh ch/, `stuff` /s t uh f/, `dreams` /d r ee m s/",
            "not in the BeatScript lexicon, sounded out by the rules: `little` /l ii t l/, `rounded` /r ow n d ii d/",
        ]),
        ("tyger-tyger-128", "z4EBG9jB6T7bKvVFQ6ZW6UPPCYh2RQbAHx86rZ8EKczfdupgqnn", &[
            "not in the BeatScript lexicon, sounded out by the rules: `burning` /b er n ii ng/",
            "not in the BeatScript lexicon, sounded out by the rules: `burning` /b er n ii ng/",
        ]),
    ];
    for (name, cid, known) in found {
        let src = std::fs::read_to_string(dir.join(format!("{name}.btw"))).unwrap_or_else(|_| panic!("examples/btw/{name}.btw"));
        assert_eq!(compiled(&src).cid, *cid, "{name}");
        assert_eq!(warnings(&src), *known, "{name}: its warnings moved (update the holdout and the comment in the file)");
        assert!(src.lines().filter(|l| l.starts_with("# ")).any(|l| l.to_ascii_lowercase().contains("public domain")), "{name} says where its words are from");
        let ld = djengine::json::JsonValue::parse(&std::fs::read_to_string(dir.join(format!("{name}.jsonld"))).unwrap_or_else(|_| panic!("examples/btw/{name}.jsonld"))).unwrap();
        assert_eq!(ld.get("identifier").and_then(|v| v.as_str()), Some(*cid), "{name}.jsonld");
        let based = ld.get("isBasedOn").and_then(|v| v.as_array()).unwrap_or_else(|| panic!("{name}.jsonld has no isBasedOn"));
        assert!(!based.is_empty());
        for w in based {
            let year: u32 = w.get("dateCreated").and_then(|v| v.as_str()).and_then(|y| y.parse().ok()).unwrap_or(9999);
            assert!(year < 1929, "{name}: a source of {year}");
        }
    }
}
