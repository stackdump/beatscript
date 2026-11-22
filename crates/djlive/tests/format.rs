//! `fmt` keeps what the canonical form leaves out (djlive::format): the `#!` header lines, every comment and the blank lines
//! between blocks. It makes the same program (the CID of the formatted text is the program's), and a formatted program formats
//! to itself. The corpus is every `.btw` of the repo, and each of them again written out with other whitespace, comments, blank
//! lines and continuation lines (`common::reformat`), so a comment lands in every place a person might have put one.

mod common;

use std::path::{Path, PathBuf};

use common::{reformat, Rng};
use djlive::compile::check;
use djlive::env::Env;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "btw") {
            out.push(p);
        }
    }
}

/// Every program of the repo's examples, docs and test fixtures.
fn corpus() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    for dir in ["examples", "docs", "crates/btwapi/tests", "crates/djlive/tests", "crates/btwls/tests", "crates/btwround/fixtures", "tools/listen/fixtures"] {
        walk(&root().join(dir), &mut paths);
    }
    paths.sort();
    paths.into_iter().map(|p| (p.strip_prefix(root()).unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap())).collect()
}

fn fmt(src: &str) -> Result<String, String> {
    djlive::format::format(src, &Env::none()).map_err(|d| djlive::diag::render_all(&d, src))
}

/// What the comments of a text say, sorted: each comment's text between its `#`s (two comments after one line become one, `#`
/// inside it, and a comment inside a statement is put above it, so neither the count nor the order is kept, only the words).
fn comments(src: &str) -> Vec<String> {
    let mut all: Vec<String> = djlive::parse::parse(src).comments.iter().flat_map(|c| c.text(src).split('#').map(|w| w.trim().to_string()).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect();
    all.sort();
    all
}

const TRACK: &str = "\
#! btw 1
#! name:   Test Track
#! license: CC0-1.0

# A leading comment block
# second line


tempo   120   # trailing on tempo
key am
let riff = c2 ~ [e2 g2] ~   # a riff

# groove def
def groove(deck, bars) {   # doc of def
  # inside the def
  deck $deck: kick x ~ ~ ~ x ~ ~ ~   # kick line

  repeat $bars every 2 bars {
    master: filter lp 300 -> 2800 over 1 bars  # sweep
  }
  # last in def
}

section intro 8 bars {   # the intro
  # drums first
  deck A: kick   e(4,16) |   snare ~ x ~ x   # drums
  deck B: bass <a1 ~ c2 ~>  # bass

  deck B: harmony chord(am)
  # dangling in section
}

# between
section drop 16 bars { deck A: kick e(4,16) }   # one-liner

section outro 16 bars {
  deck A: kick e(4,16) | melody a3 ~ c4 ~
  evolve every 8 bars {
    # why rotate
    melody: rotate 1   # rot
    stop when gen == 4  # stop
  }
}

arrange intro drop outro   # the order
# the end
";

const TRACK_FORMATTED: &str = "\
#! btw 1
#! name:   Test Track
#! license: CC0-1.0

# A leading comment block
# second line

tempo 120  # trailing on tempo
key am
let riff = c2 ~ [e2 g2] ~  # a riff

# groove def
def groove(deck, bars) {  # doc of def
  # inside the def
  deck $deck: kick x ~ ~ ~ x ~ ~ ~  # kick line

  repeat $bars every 2 bars {
    master: filter lp 300 -> 2800 over 1 bars  # sweep
  }
  # last in def
}

section intro 8 bars {  # the intro
  # drums first
  deck A: kick e(4, 16) | snare ~ x ~ x  # drums
  deck B: bass <a1 ~ c2 ~>  # bass

  deck B: harmony chord(am)
  # dangling in section
}

# between
section drop 16 bars {
  deck A: kick e(4, 16)
}  # one-liner

section outro 16 bars {
  deck A: kick e(4, 16) | melody a3 ~ c4 ~
  evolve every 8 bars {
    # why rotate
    melody: rotate 1  # rot
    stop when gen == 4  # stop
  }
}

arrange intro drop outro  # the order
# the end
";

#[test]
fn fmt_keeps_the_header_the_comments_and_the_blank_lines() {
    assert_eq!(fmt(TRACK).unwrap(), TRACK_FORMATTED);
}

#[test]
fn what_fmt_made_is_the_program_it_was_given_and_formats_to_itself() {
    let formatted = fmt(TRACK).unwrap();
    let (a, b) = (check(TRACK), check(&formatted));
    let (a, b) = (a.compiled.expect("the fixture checks"), b.compiled.expect("the formatted fixture checks"));
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.text, b.text);
    assert_eq!(fmt(&formatted).unwrap(), formatted);
    // the header and the comments are not in the canonical text, and are all in the formatted one
    assert!(!a.text.contains('#'));
    assert_eq!(comments(&formatted), comments(TRACK));
}

#[test]
fn a_program_that_does_not_read_is_not_formatted() {
    let e = fmt("#! btw 1\ntempo 120 # fine\nkey\n").unwrap_err();
    assert!(e.contains("key"), "{e}");
    assert!(fmt("deck A: kick x |\n").is_err());
}

#[test]
fn a_comment_inside_a_statement_printed_whole_goes_above_it_in_order() {
    // a deck line written over three lines, with a comment on each: the line is one line, the comments are above it
    let src = "tempo 120\ndeck A: kick x ~ x ~ # kick\n  # between\n  | snare ~ x ~ x # snare\n  | hihat x*8 # hats\nkey am\n";
    let f = fmt(src).unwrap();
    assert_eq!(f, "tempo 120\n# between\ndeck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8  # kick # snare # hats\nkey am\n");
    assert_eq!(fmt(&f).unwrap(), f);
    // a mix block: the comments inside go above it, one after its first line stays there
    let src = "tempo 120\nsection verse 4 bars {\n  deck A: kick x\n  mix {  # the mix\n    # level first\n    A level 0.5\n    B level 1   # b\n  }  # done\n}\narrange verse\n";
    let f = fmt(src).unwrap();
    assert_eq!(fmt(&f).unwrap(), f, "{f}");
    assert_eq!(comments(&f), ["b", "done", "level first", "the mix"], "{f}");
    assert_eq!(check(src).compiled.unwrap().cid, check(&f).compiled.unwrap().cid);
}

#[test]
fn comments_only_and_empty_texts_are_kept() {
    assert_eq!(fmt("# nothing here\n\n\n# yet\n").unwrap(), "# nothing here\n\n# yet\n");
    assert_eq!(fmt("").unwrap(), "");
    assert_eq!(fmt("  \n\n").unwrap(), "");
    // blank lines are never added at the start, after a `{` or before a `}`, and a run of them is one
    assert_eq!(fmt("\n\n# a\n\n\n\ntempo 100\nsection a 1 bars {\n\n  deck A: kick x\n\n}\n").unwrap(), "# a\n\ntempo 100\nsection a 1 bars {\n  deck A: kick x\n}\n");
}

#[test]
fn windows_line_endings_come_out_as_lines() {
    let f = fmt("#! btw 1\r\n# note\r\ntempo   120   # a\r\n\r\n\r\nkey am\r\n").unwrap();
    assert_eq!(f, "#! btw 1\n# note\ntempo 120  # a\n\nkey am\n");
}

#[test]
fn a_sample_line_is_kept_as_it_was_written() {
    let src = "#! visibility: private\nuse restricted\nsample harp from \"harp.wav\" license restricted-private   # mine\ntempo 100\n";
    let f = fmt(src).unwrap();
    assert!(f.contains("sample harp from \"harp.wav\" license restricted-private  # mine"), "{f}");
    assert_eq!(fmt(&f).unwrap(), f);
}

#[test]
fn a_comment_that_looks_like_a_sharp_numeral_stays_a_comment() {
    // `#v` after a pitched part is a numeral, not a comment; a comment is `# ` or at the head of a line
    let src = "tempo 120\nkey cm\ndeck A: harmony I IV # I then IV\n#v the five\ndeck B: kick x   #4 the hits\n";
    let f = fmt(src).unwrap();
    assert_eq!(fmt(&f).unwrap(), f);
    assert_eq!(check(src).compiled.unwrap().cid, check(&f).compiled.unwrap().cid);
    assert_eq!(comments(&f), comments(src));
}

#[test]
fn every_program_of_the_repo_formats_to_itself_with_its_cid_and_every_comment() {
    let mut formatted = 0;
    let mut with_comments = 0;
    for (name, src) in corpus() {
        // (a program that needs a host, or that does not read, is not the formatter's: it has its own tests)
        let Ok(f) = fmt(&src) else { continue };
        formatted += 1;
        assert_eq!(fmt(&f).unwrap(), f, "{name}: formatting twice is not formatting once");
        assert_eq!(comments(&f), comments(&src), "{name}: a comment was lost, moved to another line or changed");
        if !comments(&src).is_empty() {
            with_comments += 1;
        }
        if let (Some(a), Some(b)) = (check(&src).compiled, check(&f).compiled) {
            assert_eq!(a.cid, b.cid, "{name}: the CID moved");
            assert_eq!(a.text, b.text, "{name}");
        }
    }
    assert!(formatted >= 40, "only {formatted} programs formatted");
    assert!(with_comments >= 20, "only {with_comments} of them have comments");
}

#[test]
fn reformatted_programs_keep_their_comments_and_cid_through_fmt() {
    // (BTW_FMT_ROUNDS=300 for a longer hunt)
    let rounds: u64 = std::env::var("BTW_FMT_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(12);
    for (name, src) in corpus() {
        if check(&src).compiled.is_none() {
            continue;
        }
        // (the deep-house hour takes seconds to check: its reformattings are not the point of this test)
        if src.len() > 4000 {
            continue;
        }
        let mut rng = Rng(0xf0a7_0000_0000_0000 ^ name.len() as u64);
        for round in 0..rounds {
            let text = reformat(&src, &mut rng);
            let f = fmt(&text).unwrap_or_else(|e| panic!("{name} round {round} does not format: {e}\n{text}"));
            assert_eq!(fmt(&f).unwrap_or_else(|e| panic!("{name} round {round}: the formatted text does not format: {e}\n{f}")), f, "{name} round {round}: not idempotent\n--- from\n{text}\n--- to\n{f}");
            assert_eq!(comments(&f), comments(&text), "{name} round {round}: comments\n--- from\n{text}\n--- to\n{f}");
            // (the CID of the text as it was reformatted: this is about fmt, not about the reformatting)
            let was = check(&text).compiled.map(|c| c.cid);
            assert_eq!(check(&f).compiled.map(|c| c.cid), was, "{name} round {round}: the CID moved\n--- from\n{text}\n--- to\n{f}");
        }
    }
}
