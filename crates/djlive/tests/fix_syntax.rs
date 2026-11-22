//! The syntax reviewer's findings (SYN-01 ... SYN-26), each with the program that showed it. SYN-01 first: the programs of the
//! people who wrote before the extensions use names that the extensions made words; a word of the extensions is reserved only
//! where it starts a construct, and everywhere else it is read as syntax by its position.

mod common;

use common::{reformat, Rng};
use djlive::compile::{check, check_with, Compiled};
use djlive::diag::{render_all, Diagnostic};
use djlive::env::Env;

fn ok(src: &str) -> Compiled {
    let c = check(src);
    assert!(c.ok(), "{src}\n{}", render_all(&c.diagnostics, src));
    c.compiled.unwrap()
}

fn errors(src: &str) -> Vec<Diagnostic> {
    let c = check_with(src, &Env::none());
    assert!(!c.ok(), "expected an error in {src:?}");
    c.diagnostics.into_iter().filter(Diagnostic::is_error).collect()
}

fn warnings(src: &str) -> Vec<Diagnostic> {
    check(src).diagnostics.into_iter().filter(|d| !d.is_error()).collect()
}

// ---- SYN-01: the names of old programs

/// The names an old program may well have used, which the extensions made words.
const FREED: [&str; 40] = [
    "pad", "drums", "depth", "test", "genre", "tags", "replay", "length", "period", "step", "in", "out", "chords", "pattern", "rhythm", "center", "phase", "release", "attack", "eq", "exp", "log", "beat", "beats", "knob", "fader", "held", "min", "max",
    "sine", "tri", "square", "saw", "linear", "weights", "ends", "mix", "duck", "i", "v",
];

#[test]
fn the_names_of_old_programs_name_a_let_a_def_and_a_section_again() {
    for name in FREED {
        // a pattern let
        let src = format!("let {name} = c3 e3 g3\ndeck B: harmony {name} inst=warm-pad\n");
        let c = ok(&src);
        assert_eq!(c.text, src, "{name}");
        // a scalar let
        let src = format!("let {name} = 3\ndeck B: kick x*{name}\n");
        let c = ok(&src);
        assert!(c.text.contains("kick x*3"), "{name}: {}", c.text);
        // a def, called
        let src = format!("def {name}(deck) {{\n  deck $deck: kick x ~ x ~\n}}\n{name}(A)\n");
        let c = ok(&src);
        assert_eq!(c.text, src, "{name}");
        // a section, arranged
        let src = format!("section {name} 8 bars {{\n  deck A: kick x ~ x ~\n}}\narrange {name}\n");
        let c = ok(&src);
        assert_eq!(c.text, src, "{name}");
    }
}

#[test]
fn the_old_programs_words_name_what_they_named_and_keep_their_text() {
    // what yellow-rose and the saved snippets of this repo's author had: `pad`, `drums`, `depth`
    let src = "tempo 124\nkey am\nlet pad = <c3 c3 f3 c3>\ndef drums(k, h) {\n  deck A: kick $k | hihat $h\n}\nsection depth 8 bars {\n  deck A: harmony pad inst=warm-pad len=16 vel=70\n  drums(x ~ x ~, x*4)\n}\nsection test 4 bars {\n  deck A: kick x\n}\narrange depth test\n";
    let c = ok(src);
    assert_eq!(c.text, src);
    // the names a person gives a riff: a numeral, a letter, a word of the shapes
    let src = "let i = 3\nlet v = c2 e2\nlet step = 2\ndeck A: bass v | kick x*i\nat bar step: master: level 0.5\n";
    let c = ok(src);
    assert!(c.text.contains("at bar 2: master: level 0.5"), "{}", c.text);
    assert!(c.text.contains("kick x*3"), "{}", c.text);
    assert!(c.text.contains("bass v"), "{}", c.text);
}

#[test]
fn a_scalar_let_named_like_a_word_of_the_new_syntax_does_not_break_that_syntax() {
    // each program has the let and then the construct that reads the word as syntax
    let cases: [(&str, &str); 22] = [
        ("step", "tempo 100 -> 130 over 4 bars step 4\ndeck A: kick x\n"),
        ("exp", "deck A: kick x\nat bar 2: master: filter lp 300 -> 2000 over 4 bars exp\n"),
        ("log", "deck A: kick x\nat bar 2: master: filter lp 300 -> 2000 over 4 bars log\n"),
        ("linear", "deck A: kick x\nat bar 2: master: filter lp 300 -> 2000 over 4 bars linear\n"),
        ("sine", "deck A: kick x\nmaster: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars\n"),
        ("tri", "deck A: kick x\nmaster: filter lp lfo tri 2 bars center=3k depth=1k over 16 bars\n"),
        ("depth", "deck A: kick x\nmaster: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars\n"),
        ("center", "deck A: kick x\nmaster: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars\n"),
        ("beats", "deck A: kick x\nat bar 2: master: filter lp 300 -> 2000 over 2 beats\n"),
        ("beat", "deck A: kick x\nat bar 2 beat 2: master: level 0.5\n"),
        ("length", "tempo 120\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\nassert length == 8s\n"),
        ("ends", "section verse 4 bars {\n  deck A: kick x\n}\narrange verse\nassert ends\n"),
        ("in", "tempo 120\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\nassert level A < 2 * B in verse\n"),
        ("min", "deck A: kick x\nmaster: level min(1, 2) / 4\n"),
        ("pad", "deck A: kick x\nmaster: filter lp 200 + pad(3) * 7800\n"),
        ("knob", "deck A: kick x\nmaster: filter lp 200 + knob(3) * 7800\n"),
        ("weights", "deck A: bass choose [c2 e2] weights 3 1 seed=4\n"),
        ("eq", "section verse 4 bars {\n  deck A: kick x\n  mix {\n    A eq low -3\n  }\n}\narrange verse\n"),
        ("drums", "def walker(oct) in: drums out: bass {\n  emit bass c2\n}\n"),
        ("chords", "def changes() out: chords, key {\n  emit chords chord(am7)\n  emit key am\n}\ndef walker() in: chords {\n  deck B: harmony $chords\n}\nwire changes.chords -> walker.chords\nwalker()\n"),
        ("mix", "section verse 4 bars {\n  deck A: kick x\n  mix {\n    A level 0.5\n  }\n}\narrange verse\n"),
        ("duck", "deck A: kick x\nduck B under A.kick 6db\n"),
    ];
    for (name, program) in cases {
        let with_let = format!("let {name} = 2\n{program}");
        let c = check(&with_let);
        assert!(c.ok(), "let {name}:\n{with_let}\n{}", render_all(&c.diagnostics, &with_let));
        // the let changed nothing in the program that does not use the name: the same text as without it
        let without = ok(program);
        let with = c.compiled.unwrap();
        assert_eq!(with.text.strip_prefix(&format!("let {name} = 2\n")).unwrap_or(&with.text), without.text, "let {name}");
    }
}

#[test]
fn a_def_named_like_a_word_of_the_new_syntax_is_called_as_a_def() {
    for name in ["mix", "duck", "pad", "min", "knob", "held"] {
        let src = format!("def {name}(deck) {{\n  deck $deck: kick x\n}}\n{name}(A)\n");
        let c = ok(&src);
        assert_eq!(c.text, src, "{name}");
    }
    // and the words still start their statements
    ok("section verse 4 bars {\n  deck A: kick x\n  mix {\n    A level 0.5\n  }\n}\narrange verse\n");
    ok("deck A: kick x\nduck B under A.kick 6db\n");
}

#[test]
fn a_def_may_be_called_stack_or_then_and_is_called_with_no_deck_before_it() {
    // the reviewer's own probe: valid before the extensions
    for (name, parts) in [("stack", "kick x ~ x ~, snare ~ x ~ x"), ("then", "kick x ~ x ~, kick ~ x ~ x")] {
        let src = format!("def {name}(deck) {{\n  deck $deck: kick x ~ x ~\n}}\n{name}(A)\n");
        let c = ok(&src);
        assert_eq!(c.text, src);
        // and the construct is still the construct after a deck
        ok(&format!("def {name}(deck) {{\n  deck $deck: hihat x\n}}\n{name}(B)\ndeck A: {name}({parts})\n"));
    }
    // without a def of that name, `stack(` at the start of a line says where it goes
    let e = errors("stack(kick x ~ x ~, snare ~ x ~ x)\n");
    assert!(e[0].message.contains("needs a deck"), "{:?}", e[0]);
    let e = errors("stack(A)\n");
    assert!(e[0].message.contains("`stack` is not a def") && e[0].help.as_deref().unwrap_or("").contains("deck A: stack("), "{:?}", e[0]);
    // a let or a section of that name is still refused
    let e = errors("let stack = 3\ndeck A: kick x\n");
    assert!(e[0].message.contains("cannot be a name"));
}

#[test]
fn a_word_of_the_language_that_cannot_name_a_thing_says_what_to_write_instead() {
    for name in ["silence", "wire", "assert", "sing", "under", "choose", "maybe", "deg", "lfo", "and", "or", "not", "emit", "s-curve", "kick", "tempo", "a", "x", "c3", "idim"] {
        let e = errors(&format!("let {name} = 3\ndeck A: kick x\n"));
        let d = &e[0];
        assert!(d.message.contains("cannot be a name"), "{name}: {}", d.message);
        let help = d.help.clone().unwrap_or_default();
        assert!(help.contains("rename it, for example `"), "{name}: {help}");
        // the name offered is free
        let offered = help.split('`').nth(1).unwrap();
        ok(&format!("let {offered} = 3\ndeck A: kick x\n"));
    }
    let e = errors("def silence(deck) {\n  deck $deck: kick x\n}\n");
    assert!(e[0].help.as_deref().unwrap_or("").contains("rename it"), "{:?}", e[0]);
    let e = errors("section then 4 bars {\n  deck A: kick x\n}\n");
    assert!(e[0].help.as_deref().unwrap_or("").contains("rename it"), "{:?}", e[0]);
}

#[test]
fn a_def_param_and_a_def_header_are_not_put_in_place_of_a_scalar_let() {
    // SYN-06: a let named like a def's parameter used to be read as a number in the header
    let src = "let n = 4\ndef grv(deck, n) {\n  deck $deck: kick x*$n\n}\ngrv(A, 2)\n";
    let c = ok(src);
    assert_eq!(c.text, src);
    // a section, a def named like a let: said, with the let named
    let e = errors("let drop = 8\nsection drop 8 bars {\n  deck A: kick x\n}\narrange drop\n");
    let m = &e[0].message;
    assert!(m.contains("`drop` is the let on line 1 (= 8)") && m.contains("a section needs a name of its own"), "{m}");
    assert!(e[0].help.as_deref().unwrap_or("").contains("rename it"));
    let e = errors("let grv = 8\ndef grv(deck) {\n  deck $deck: kick x\n}\n");
    assert!(e[0].message.contains("`grv` is the let on line 1 (= 8)") && e[0].message.contains("a def needs a name of its own"), "{}", e[0].message);
}

#[test]
fn a_name_that_is_a_let_where_a_name_was_meant_says_which_let() {
    // `n` is put in place as 4 wherever it stands: a sample named n, for one
    let e = errors("let n = 4\nsample n from \"x.wav\"\n");
    let h = e[0].help.clone().unwrap_or_default();
    assert!(h.contains("`n` is the let on line 1 (= 4)"), "{:?}", e[0]);
}

#[test]
fn old_programs_text_is_stable_under_reformatting_with_the_freed_names() {
    let src = "let pad = c3 e3 g3\ndef drums(k) {\n  deck A: kick $k\n}\nsection depth 8 bars {\n  deck A: harmony pad\n  drums(x ~ x ~)\n}\narrange depth\n";
    let cid = ok(src).cid;
    let mut rng = Rng(7);
    for _ in 0..30 {
        let r = reformat(src, &mut rng);
        assert_eq!(ok(&r).cid, cid, "{r}");
    }
}

#[test]
fn the_freed_words_are_still_words_where_the_new_syntax_reads_them() {
    // `step` after a duration is the shape, `eq` the mix item, `chords` the port type: with lets of their own names around
    let src = "let step = 3\nlet exp = 2\nlet eq = 1\nlet chords = 4\ntempo 100 -> 130 over 4 bars step 4\ndef walk() in: chords {\n  deck A: harmony $in\n}\n";
    let c = ok(src);
    assert!(c.text.contains("over 4 bars step 4"), "{}", c.text);
    let _ = warnings(src);
}

// ---- SYN-12: a comment that begins with a numeral's letters

#[test]
fn a_trailing_comment_glued_to_its_hash_is_a_comment_unless_it_is_a_numeral_in_a_pitched_part() {
    let canon = |src: &str| ok(src).text;
    // after a drum part, on a control line, at the head of a block: always a comment (as before the numerals)
    assert_eq!(canon("deck A: kick x ~ x ~ #v\n"), "deck A: kick x ~ x ~\n");
    assert_eq!(canon("deck A: kick x ~ x ~ #vi is the sixth\n"), "deck A: kick x ~ x ~\n");
    assert_eq!(canon("deck A: kick x | snare ~ x #I like it\n"), "deck A: kick x | snare ~ x\n");
    assert_eq!(canon("deck A: kick x\nat bar 2: master: filter lp 300 #v\n"), "deck A: kick x\nat bar 2: master: filter lp 300\n");
    assert_eq!(canon("section verse 4 bars { #I\n  deck A: kick x\n}\narrange verse\n"), "section verse 4 bars {\n  deck A: kick x\n}\narrange verse\n");
    assert_eq!(canon("let n = 4 #v\ndeck A: kick x*n\n"), "let n = 4\ndeck A: kick x*4\n");
    // the last part of a line decides: a drum part after a pitched one
    assert_eq!(canon("deck A: harmony I | kick x #v\n"), "deck A: harmony I | kick x\n");
    // a pitched part followed by words that are no steps: a comment
    assert_eq!(canon("deck B: melody c3 e3 #vi is the sixth\n"), "deck B: melody c3 e3\n");
    assert_eq!(canon("deck B: harmony I V #I like it\n"), "deck B: harmony I V\n");
    // and a sharp numeral is still one in a pitched part, a let and an emit
    assert_eq!(canon("key c\ndeck B: harmony I #iv V\n"), "key c\ndeck B: harmony I #iv V\n");
    assert_eq!(canon("key c\ndeck B: harmony I #iv | kick x\n"), "key c\ndeck B: harmony I #iv | kick x\n");
    assert_eq!(canon("key c\nlet prog = I #iv V\ndeck B: harmony prog\n"), "key c\nlet prog = I #iv V\ndeck B: harmony prog\n");
    assert_eq!(canon("def ch() out: chords {\n  emit I #iv V\n}\n"), "def ch() out: chords {\n  emit I #iv V\n}\n");
    // the same cid with the comment and without
    assert_eq!(ok("deck A: kick x ~ x ~ #v\n").cid, ok("deck A: kick x ~ x ~\n").cid);
}

// ---- SYN-04: the comma in a `deg( )` group

#[test]
fn a_comma_inside_a_bracketed_group_of_deg_is_always_the_stack_comma() {
    let cid = |src: &str| ok(&format!("key c\ndeck B: harmony {src}\n")).cid;
    // one spacing or another, a chord
    let chord = cid("deg([1,3,5])");
    for spelling in ["deg([1, 3, 5])", "deg([1 ,3 ,5])", "deg([1 , 3 , 5])", "deg( [ 1,3,5 ] )"] {
        assert_eq!(cid(spelling), chord, "{spelling}");
    }
    // a rest is no different from a degree
    assert_eq!(cid("deg([1, ~])"), cid("deg([1,~])"));
    assert_eq!(cid("deg([~, 1])"), cid("deg([~,1])"));
    // an octave down inside the group is the comma before its end, and a stack after an octave mark has a space before
    // its comma (the canonical text)
    let down = ok("key c\ndeck B: harmony deg([3 1,])\n");
    assert!(down.text.contains("deg([3 1,])"), "{}", down.text);
    let both = ok("key c\ndeck B: harmony deg([1, ,3])\n");
    assert!(both.text.contains("deg([1, ,3])"), "{}", both.text);
    // at the top of the deg( ) the comma with a space after it is the octave mark, as documented: three degrees an octave down,
    // and a warning says it is not the chord
    let w = warnings("key c\ndeck B: harmony deg(1, 3, 5)\n");
    assert!(w.iter().any(|d| d.message.contains("octave-down mark") && d.help.as_deref().unwrap_or("").contains("`deg(1,3,5)`")), "{w:?}");
    for quiet in ["key c\ndeck B: harmony deg(1,3,5)\n", "key c\ndeck B: harmony deg(1 3 5)\n", "key c\ndeck B: harmony deg(1, 5' [1,5])\n", "key c\ndeck B: harmony deg([3 1,])\n", "key c\ndeck B: melody deg(1,)\n"] {
        assert!(!warnings(quiet).iter().any(|d| d.message.contains("octave-down mark")), "{quiet}");
    }
    let low = ok("key c\ndeck B: harmony deg(1, 3, 5)\n");
    assert_ne!(low.cid, chord);
    assert!(low.text.contains("deg(1, 3, 5)"), "{}", low.text);
}

// ---- SYN-05: an error at the end of the text is on the line that stopped short

#[test]
fn an_error_at_the_end_of_the_text_is_on_the_last_line_with_code() {
    let at = |src: &str| {
        let e = errors(src);
        let d = e.iter().find(|d| d.found == "the end of the text").unwrap_or_else(|| panic!("{e:?}"));
        d.position(src)
    };
    // the same line and column with a trailing newline, blank lines and a comment after it, or without
    let base = "deck A: kick x\nat bar 2: master: filter lp 300 -> 200 over 4";
    let (line, col, _) = at(base);
    assert_eq!((line, col), (2, 46));
    for tail in ["\n", "\n\n\n", "\n# end\n", "\n\n   \n# one\n# two\n"] {
        assert_eq!(at(&format!("{base}{tail}")).0, 2, "{tail:?}");
        assert_eq!(at(&format!("{base}{tail}")).1, col, "{tail:?}");
    }
    // each of the statements the finding named
    for src in ["tempo 100 ->\n\n# end\n", "at bar\n\n", "assert level A > 0.1 in\n\n# c\n", "deck B: sing\n\n", "duck B under\n\n", "section a 4\n\n"] {
        let e = errors(src);
        assert!(e.iter().all(|d| d.position(src).0 == 1), "{src:?}: {:?}", e.iter().map(|d| d.position(src)).collect::<Vec<_>>());
    }
}

// ---- SYN-09: a `when` guard after a line of several parts

#[test]
fn a_when_guard_after_a_line_of_several_parts_says_it_guards_them_all() {
    for src in [
        "deck A: kick x ~ x ~ | snare ~ x ~ x when 1 == 2\n",
        "repeat 2 {\n  deck A: kick x ~ x ~ | snare ~ x ~ x when $i == 1\n}\n",
        "deck A: stack(kick x ~ x ~, snare ~ x ~ x) when 1 == 1\n",
        "deck A: kick x | snare x | hihat x when 1 == 1\n",
    ] {
        let c = check(src);
        assert!(c.ok(), "{src}");
        let w: Vec<_> = c.diagnostics.iter().filter(|d| d.message.contains("guards all")).collect();
        assert!(!w.is_empty() && w.iter().all(|d| !d.is_error() && d.help.as_deref().unwrap_or("").contains("its own line")), "{src}\n{:?}", c.diagnostics);
    }
    // the whole line goes when the guard is false, as the warning says (and that is how it was read before)
    let c = ok("deck A: kick x ~ x ~ | snare ~ x ~ x when 1 == 2\ndeck B: bass c2\n");
    assert!(!c.decks.contains_key(&djlive::ast::Deck::A));
    // one part, a control line and a guarded line each: no word
    for src in ["deck A: kick x ~ x ~ when 1 == 1\ndeck A: snare ~ x ~ x when 1 == 2\nat bar 2: master: level 0.5 when 1 == 1\n", "deck A: stack(kick x ~ x ~) when 1 == 1\n"] {
        let c = check(src);
        assert!(c.ok() && c.diagnostics.is_empty(), "{src}: {:?}", c.diagnostics);
    }
}

// ---- SYN-08: a typo of a word of the language says which word

/// The first error (message, help) of a program that does not read.
fn first(src: &str) -> (String, String) {
    let e = errors(src);
    (e[0].message.clone(), e[0].help.clone().unwrap_or_default())
}

fn helps(src: &str) -> Vec<String> {
    errors(src).into_iter().filter_map(|d| d.help).collect()
}

#[test]
fn a_typo_at_the_start_of_a_statement_names_the_statement() {
    for (typo, word, src) in [
        ("asert", "assert", "asert ends\n"),
        ("tmpo", "tempo", "tmpo 120\n"),
        ("wre", "wire", "wre a.out -> b.in\n"),
        ("wir", "wire", "wir a.out -> b.in\n"),
        ("sng", "sing", "sng \"la\" on melody\n"),
        ("mx", "mix", "mx { A level 0.5 }\n"),
        ("mixx", "mix", "mixx { A level 0.5 }\n"),
        ("dck", "duck", "dck B under A.kick 6db\n"),
        ("dck", "deck", "dck A: kick x\n"),
        ("sectoin", "section", "sectoin a 4 bars {\n  deck A: kick x\n}\n"),
    ] {
        let h = helps(src);
        assert!(h.iter().any(|h| h.contains(&format!("did you mean `{word}`?"))), "{typo}: {h:?}");
    }
    // and the list of what a line may start with names the new statements
    let e = errors("asert ends\n");
    assert!(e[0].expected.iter().any(|w| w == "assert") && e[0].expected.iter().any(|w| w == "wire") && e[0].expected.iter().any(|w| w == "duck"), "{:?}", e[0].expected);
    // `emit` is a statement of a def's body: outside one it says so
    assert!(first("emit x\n").1.contains("a def that has `out:` ports"));
}

#[test]
fn a_typo_inside_a_statement_names_the_word() {
    let base = "section sa 4 bars {\n  deck A: kick x ~ x ~\n}\n";
    for (word, src) in [
        ("choose", "deck A: bass choos [c2 e2] weights 1 1\n"),
        ("deg", "key am\ndeck A: bass dg(1 3 5)\n"),
        ("lfo", "master: filter lp lf sine 2 bars\n"),
        ("under", "deck A: kick x\nduck B undr A.kick 6db\n"),
        ("on", "deck B: melody a4\ndeck B: sing \"la\" onn melody\n"),
        ("bar", "at bat 2: master: level 0.5\n"),
        ("bars", "at bar 2: master: filter lp 300 -> 200 over 2 bats\n"),
        ("over", "at bar 2: master: filter lp 300 -> 200 oer 4 bars\n"),
        ("knob", "every 1 bars: master: filter lp 200 + knb(3) * 100\n"),
        ("voice-lead", "key am\ndeck B: harmony I V voice-led\n"),
        ("when", "deck A: kick x whn 1 == 1\n"),
        ("weights", "deck A: bass choose [c2 e2] wights 1 1\n"),
    ] {
        let h = helps(src);
        assert!(h.iter().any(|h| h.contains(&format!("`{word}`"))), "{word}: {h:?}");
    }
    let h = helps(&format!("{base}arrange sa silenc\n"));
    assert!(h.iter().any(|h| h.contains("did you mean `silence`?")), "{h:?}");
    let h = helps(&format!("{base}assert level A > 0.1 inn sa\n"));
    assert!(h.iter().any(|h| h.contains("did you mean `in`?")), "{h:?}");
    // a known generator that is wrong inside says nothing of its name
    let h = helps("deck A: kick e(5,)\n");
    assert!(!h.iter().any(|h| h.contains("is no generator")), "{h:?}");
}

#[test]
fn a_block_that_did_not_read_is_skipped_whole_and_a_one_line_block_reads() {
    // SYN-17: after a header that does not read, its body is not read as statements of the top level
    for (src, only) in [
        ("def (d) {\n  deck $d: kick x\n  emit x\n}\ndeck A: kick x\n", 1),
        ("deck A: kick x\nevolve every 8 bars {\n  kick: rotate knob(3)\n  kick: thin\n  emit x\n}\ndeck B: kick x\n", 1),
        ("def grv(deck, ) {\n  deck $deck: kick x\n  repeat 2 {\n    emit x\n  }\n}\n", 1),
        ("section verse {\n  deck A: kick x\n}\ndeck A: kick x\n", 1),
        ("section verse 4 bars\n{\n  deck A: kick x\n}\ndeck A: kick x\n", 1),
    ] {
        let e = errors(src);
        assert_eq!(e.len(), only, "{src}\n{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
    }
    // a section on one line closes on that line
    let c = ok("section verse 4 bars { deck A: kick x ~ x ~ }\narrange verse\n");
    assert!(c.text.contains("section verse 4 bars {\n  deck A: kick x ~ x ~\n}"), "{}", c.text);
    // an error in a section's body leaves the section's own brace
    let e = errors("section verse 4 bars {\n  deck A: kick ??? x\n  deck B: bass c2\n}\narrange verse\n");
    assert!(e.iter().all(|d| !d.message.contains("never closed") && !d.message.contains("with no `{`")), "{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
}

#[test]
fn hz_after_a_filters_cutoff_is_the_number_and_a_unit_anywhere_else_says_where_units_go() {
    // SYN-13
    let a = ok("master: filter lp 300hz\n");
    let b = ok("master: filter lp 300\n");
    assert_eq!(a.cid, b.cid);
    assert_eq!(ok("at bar 2: master: filter hp 200hz -> 2000hz over 4 bars\n").cid, ok("at bar 2: master: filter hp 200 -> 2000 over 4 bars\n").cid);
    let (_, h) = first("do vocoder set mix=6db\n");
    assert!(h.contains("the unit `db` is read where the language takes one"), "{h}");
    let (_, h) = first("master: level 0.5hz\n");
    assert!(h.contains("`300hz`"), "{h}");
    // khz is the k mark
    let e = errors("master: filter lp 2khz\n");
    assert!(e.iter().any(|d| d.help.as_deref().unwrap_or("").contains("2k")), "{e:?}");
}

#[test]
fn the_ascii_spelling_of_the_tolerance_and_other_marks() {
    // SYN-15: `+-` where a tolerance stands
    let src = |tol: &str| format!("tempo 120\nsection verse 4 bars {{\n  deck A: kick x\n}}\narrange verse\nassert length == 8s {tol} 1s\n");
    assert_eq!(ok(&src("+-")).cid, ok(&src("\u{b1}")).cid);
    assert!(ok(&src("+-")).text.contains("8s \u{b1} 1s"));
    // a `+` and a minus elsewhere stay arithmetic
    ok("tempo 100 +-5\ndeck A: kick x\n");
    let (m, h) = first(&src("+"));
    assert!(m.contains("expected the end of the line") && h.contains("`+-`"), "{m} {h}");
    let (_, h) = first("tempo 120\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\nassert length == 3m30\n");
    assert!(h.contains("`3m30s`"), "{h}");
}

#[test]
fn a_gt_in_a_calls_arguments_gets_the_help_a_lt_does() {
    // SYN-19
    let src = "def gate(d, c) {\n  deck $d: kick x when $c\n}\ngate(A, 3 > 2)\n";
    let (m, h) = first(src);
    assert!(m.contains("never closed") && h.contains("write a comparison in the def"), "{m} {h}");
}

#[test]
fn marks_that_belong_to_deg_say_so_outside_it() {
    // SYN-21
    for src in ["deck A: bass 5'\n", "deck A: bass c3'\n"] {
        assert!(first(src).1.contains("inside `deg( )`"), "{src}");
    }
    assert!(first("deck A: bass 1, 3\n").1.contains("inside `deg( )`"));
    // a `#` that is no sharp inside deg( ) is an error of its own, not a comment that eats the line
    let e = errors("key am\ndeck A: bass deg(1 #k 5)\n");
    assert!(e.iter().any(|d| d.message.contains("a `#` inside `deg( )` is a sharp degree")), "{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
    let e = errors("key am\ndeck A: bass deg(1 # 4)\n");
    assert!(e.iter().any(|d| d.message.contains("a comment cannot start inside the parentheses")));
    // a negative degree says how to go below the tonic
    let (m, h) = first("key am\ndeck A: bass deg(-1 3)\n");
    assert!(m.contains("cannot be negative") && h.contains("`1,`"), "{m} {h}");
}

#[test]
fn a_percent_glued_to_a_number_warns_and_odd_characters_say_what_they_are() {
    // SYN-22 (an error, not a warning: L-11 made it one first, and in every old build a number glued to a `%`'s number was an error)
    let w = errors("let n = 9%4\ndeck A: kick x\n");
    assert!(w.iter().any(|d| d.message.contains("`9%` is the percent mark") && d.help.as_deref().unwrap_or("").contains("`9 % 4`")), "{w:?}");
    assert!(!warnings("deck A: kick x?50%\n").iter().any(|d| d.message.contains("percent mark")));
    // SYN-23
    for (src, want) in [
        ("deck B: melody a4\ndeck B: sing \u{201c}la la\u{201d} on melody\n", "straight quotes"),
        ("tempo 100 \u{2192} 130 over 4 bars\n", "`->`"),
        ("tempo 100 \u{2013} 1\n", "plain `-`"),
        ("deck A: kick\u{a0}x\n", "ordinary space"),
        ("tempo 120 \u{ab}x\u{bb}\n", "typographic quote"),
        ("deck A: kick x \u{d7} 2\n", "write `*`"),
        ("deck A: kick x \u{b0}\n", "only at the end of a Roman numeral"),
    ] {
        let e = errors(src);
        assert!(e.iter().any(|d| d.help.as_deref().unwrap_or("").contains(want)), "{src:?}: {:?}", e.iter().map(|d| (&d.message, &d.help)).collect::<Vec<_>>());
    }
}

// ---- SYN-14: a let that holds a unit value

#[test]
fn a_let_holds_a_number_with_its_unit() {
    let src = "let atk = 5ms\nlet depth_db = 6db\ndeck A: kick x\nduck B under A.kick depth_db attack=atk release=120ms\n";
    let c = ok(src);
    assert!(c.text.contains("let atk = 5ms") && c.text.contains("let depth_db = 6db"), "{}", c.text);
    // the same program as with the values written where they are used
    let direct = ok("deck A: kick x\nduck B under A.kick 6db attack=5ms release=120ms\n");
    let body = |t: &str| t.lines().filter(|l| l.starts_with("duck") || l.starts_with("deck")).collect::<Vec<_>>().join("\n");
    assert_eq!(body(&c.text), body(&direct.text));
    // the canonical text is a fixed point and survives reformatting
    let again = ok(&c.text);
    assert_eq!(again.cid, c.cid);
    let mut rng = Rng(11);
    for _ in 0..20 {
        assert_eq!(ok(&reformat(src, &mut rng)).cid, c.cid);
    }
    // through a def and a repeat
    let src = "let atk = 5ms\ndef pump(d) {\n  duck $d under A.kick 6db attack=atk\n}\ndeck A: kick x\npump(B)\n";
    ok(src);
}

// ---- SYN-18: the editor's classes

#[test]
fn the_editor_colours_the_new_words_by_what_they_are_where_they_stand() {
    use djlive::highlight::{highlight, Class};
    let classes = |src: &str| -> Vec<(String, Class)> { highlight(src).into_iter().map(|(s, c)| (s.text(src).to_string(), c)).collect() };
    let class_of = |cs: &[(String, Class)], w: &str| cs.iter().find(|(t, _)| t == w).map(|(_, c)| *c);
    // a `sing` line's params are params, like `voice=`
    let cs = classes("deck B: sing \"la la\" on melody voice=alto bars=2 key=am\n");
    for w in ["voice", "bars", "key"] {
        assert_eq!(class_of(&cs, w), Some(Class::Param), "{w}");
    }
    // `pan` in a mix item is a keyword, as `eq` is
    let cs = classes("mix {\n  A pan -0.5\n  B eq low -3\n}\n");
    assert_eq!(class_of(&cs, "pan"), Some(Class::Keyword));
    assert_eq!(class_of(&cs, "eq"), Some(Class::Keyword));
    // `period` after `assert` is the assertion's word
    assert_eq!(class_of(&classes("assert period(drop) == 4\n"), "period"), Some(Class::Keyword));
    // a word of the extensions that names a let, a def or a section is a plain word there and a keyword in its syntax
    let cs = classes("let pad = c3 e3\nsection depth 8 bars {\n  deck A: harmony pad\n}\ndef drums(k) in: chords {\n  deck A: kick $k\n}\ndrums(x)\n");
    let all = |w: &str| cs.iter().filter(|(t, _)| t == w).map(|(_, c)| *c).collect::<Vec<_>>();
    assert_eq!(all("pad"), vec![Class::Word, Class::Word], "{cs:?}");
    assert_eq!(all("depth"), vec![Class::Word]);
    // (a def's name before its parenthesis is coloured as a call, as every def is)
    assert_eq!(all("drums"), vec![Class::Generator, Class::Generator]);
    assert_eq!(all("in"), vec![Class::Keyword]);
    assert_eq!(all("chords"), vec![Class::Keyword]);
    let cs = classes("master: filter lp lfo sine 2 bars center=3k depth=1k over 4 bars exp\nsection exp 4 bars {\n  deck A: kick x\n}\n");
    assert_eq!(cs.iter().filter(|(t, _)| t == "exp").map(|(_, c)| *c).collect::<Vec<_>>(), vec![Class::Keyword, Class::Word]);
    assert_eq!(class_of(&cs, "sine"), Some(Class::Keyword));
    assert_eq!(class_of(&cs, "depth"), Some(Class::Param));
}

// ---- SYN-20: a warning is for what may be wrong; what the check found out is a note

#[test]
fn a_chance_a_period_an_ontology_name_and_a_pending_assertion_are_notes_not_warnings() {
    use djlive::diag::Severity;
    let all = |src: &str| check(src).diagnostics;
    // the notes: info, whatever the chance
    for src in ["deck A: kick x*8?0.7\n", "deck A: hihat/12 x x ~ x\n", "tempo 120\nsection verse 4 bars {\n  deck A: kick x\n}\narrange verse\nassert level A < B in verse\n"] {
        let d = all(src);
        assert!(!d.is_empty() && d.iter().all(|d| d.severity == Severity::Info), "{src}: {d:?}");
        assert_eq!(Severity::Info.name(), "info");
    }
    // (a chance of 0 or 1 is the program written out, L-09: nothing to say of it)
    assert!(all("deck A: kick x?1\n").is_empty() && all("deck A: kick x?0\n").is_empty());
    // a program that reads and says nothing wrong has no warning, and `render_all` puts the errors first
    let src = "deck A: kick x*8?0.7\ndeck B: bass ???\n";
    let rendered = render_all(&check(src).diagnostics, src);
    assert!(rendered.starts_with("error:"), "{rendered}");
    // a value held to a range, and a guard that can divide by zero, are still warnings
    let d = all("master: filter lp knob(3) * 100000\n");
    assert!(d.iter().any(|d| d.severity == Severity::Warning && d.message.contains("is held to")), "{d:?}");
}

// ---- SYN-24: one error for the body of a def however many calls run it, and which call

#[test]
fn an_error_found_in_an_expanded_line_is_said_once_and_names_the_call() {
    let src = "def gt(d, k) {\n  deck $d: kick e($k, 8)\n}\ngt(A, 99)\ngt(B, 98)\n";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{:?}", e.iter().map(|d| (&d.message, &d.help)).collect::<Vec<_>>());
    let h = e[0].help.clone().unwrap_or_default();
    assert!(h.contains("in `gt(A, 99)`, line 4") && h.contains("and 1 other call"), "{h}");
    // one call: no `other`
    let e = errors("def gt(d, k) {\n  deck $d: kick e($k, 8)\n}\ngt(A, 99)\n");
    assert_eq!(e[0].help.as_deref(), Some("in `gt(A, 99)`, line 4"), "{:?}", e[0]);
    // a repeat of a line with an error: the repeat is named
    let e: Vec<_> = errors("repeat 3 {\n  deck A: kick e(99, 8)\n}\n").into_iter().filter(|d| d.message.contains("hit count")).collect();
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].help.as_deref().unwrap_or("").contains("the repeat on line 1"), "{:?}", e[0]);
    // an error in a line outside any call says nothing of one
    let e = errors("deck A: kick e(99, 8)\n");
    assert!(e[0].help.is_none(), "{:?}", e[0]);
}

// ---- the merge of the two recoveries (parser safety and syntax): a bad block head skips the whole block, once

#[test]
fn a_block_whose_head_did_not_read_is_skipped_whole_and_said_once() {
    let src = "section 8 bars {\n  deck A: kick x ~ x ~\n  repeat 2 {\n    deck B: bass c2\n  }\n}\ndeck A: kick x\nsection ok 4 bars\n{\n  deck A: kick x\n}\ndef (a) {\n  deck $a: kick x\n}\nrepeat x {\n  deck A: kick x\n}\nevolve {\n  deck A: kick x\n}\ndeck B: hihat x\n";
    let e = errors(src);
    let said: Vec<&str> = e.iter().map(|d| d.message.as_str()).collect();
    // one error for each of the five heads, and none for a `}` with no `{`, or a line of a body read as a statement of its own
    assert_eq!(e.len(), 5, "{said:?}");
    assert!(!said.iter().any(|m| m.contains("with no `{`") || m.contains("cannot start")), "{said:?}");
    // and the statements after them were read: a mistake there is found as well
    let e = errors(&format!("{src}deck A: kik x\n"));
    assert_eq!(e.len(), 6, "{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
}

#[test]
fn an_error_at_the_name_of_a_let_says_which_let_once() {
    // the def and the section that take a let's name say it in the message; the compile's own note of it is not said again
    for src in ["let n = 4\ndef n(a) {\n  deck $a: kick x\n}\n", "let n = 3\nsection n 4 bars {\n  deck A: kick x\n}\n"] {
        let e = errors(src);
        assert_eq!(e.len(), 1, "{src}: {:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>());
        let text = format!("{} {}", e[0].message, e[0].help.clone().unwrap_or_default());
        assert_eq!(text.matches("on line 1").count(), 1, "{text}");
    }
    // a let that came to a number where a pattern was meant: named at the use, in the help
    let e = errors("let pat = 1 -1\ndeck B: bass pat\n");
    assert!(e[0].help.as_deref().unwrap_or("").contains("`pat` is the `let` on line 1"), "{:?}", e[0]);
}

// ---- a failed section does not take the `arrange` with it

/// `section a` fails because `a` reads as a note. That is the program's one mistake: the `arrange` and the `loop` are
/// written correctly, so the checker must not add "a program with no `arrange` plays on by itself" about them.
#[test]
fn a_section_that_fails_to_parse_does_not_make_loop_warn_that_there_is_no_arrange() {
    let src = "tempo 120\nsection a 4 bars { deck A: kick x ~ x ~ }\narrange a\nloop\n";
    let c = check(src);
    assert!(c.diagnostics.iter().any(|d| d.is_error() && d.message.contains("cannot name a section")), "{}", render_all(&c.diagnostics, src));
    assert!(!c.diagnostics.iter().any(|d| d.message.contains("plays on by itself")), "a cascade warning: {}", render_all(&c.diagnostics, src));
    // the warning is still right where it was: a program that really has no `arrange` and no sections
    let w = warnings("tempo 120\ndeck A: kick x ~ x ~\nloop\n");
    assert!(w.iter().any(|d| d.message.contains("plays on by itself")), "{w:?}");
}
