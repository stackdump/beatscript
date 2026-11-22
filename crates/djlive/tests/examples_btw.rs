//! Every program in `examples/btw/` (one per extension feature, plus the showcase; docs/BTW-EXTENSIONS.md):
//! it compiles, its canonical text is a fixed point with the same CID, and 40 random reformattings
//! (`common::reformat`) keep the CID. A file with a `#! skip-test: <why>` line in its header block (the leading
//! `#!` lines, as `djlive::linked::header_lines` reads them, so it may come before or after `#! genre:` and the
//! rights lines) is left out (it needs a host's registry, say); its feature has its own tests.

mod common;

use common::{reformat, Rng};
use djlive::compile::check;

fn examples() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "btw") {
            out.push((p.file_name().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    out.sort();
    out
}

/// The reason a file asks to be left out: its `#! skip-test:` line, when the header block has one. Only the header
/// block counts (a `#! skip-test` after the program has begun is a comment, as every `#!` line there is).
fn skip_reason(src: &str) -> Option<String> {
    djlive::linked::header_lines(src).into_iter().find(|h| h.key == "skip-test").map(|h| h.value)
}

#[test]
fn a_skip_line_is_found_among_the_other_header_lines_and_only_there() {
    // the first line, as the examples have always put it
    assert_eq!(skip_reason("#! skip-test: needs a registry\ntempo 1\n").as_deref(), Some("needs a registry"));
    // after, before and between the F14 and rights lines (a reason may itself say `genre:` or a comma)
    assert_eq!(skip_reason("#! genre: techno\n#! skip-test: needs a snippet, genre: x\n#! tags: acid\ntempo 1\n").as_deref(), Some("needs a snippet, genre: x"));
    assert_eq!(skip_reason("#! license: CC0-1.0\n#! tags: acid, drop\n#! skip-test: why\n").as_deref(), Some("why"));
    // not a header: the program has begun, or a plain comment came first
    assert_eq!(skip_reason("tempo 1\n#! skip-test: no\n"), None);
    assert_eq!(skip_reason("# a comment\n#! skip-test: no\n"), None);
    assert_eq!(skip_reason("#! genre: techno\ntempo 1\n"), None);
}

#[test]
fn every_example_compiles_and_is_canonical() {
    let all = examples();
    for (name, src) in &all {
        if let Some(why) = skip_reason(src) {
            eprintln!("skipped {name}: {why}");
            continue;
        }
        let c = check(src);
        let compiled = c.compiled.unwrap_or_else(|| panic!("{name} does not compile:\n{}", djlive::diag::render_all(&c.diagnostics, src)));
        let again = check(&compiled.text).compiled.unwrap_or_else(|| panic!("{name}: the canonical text does not compile:\n{}", compiled.text));
        assert_eq!(again.text, compiled.text, "{name}: the canonical text is not a fixed point");
        assert_eq!(again.cid, compiled.cid, "{name}");
        let mut rng = Rng(0x5eed_0000_0000_0000 ^ name.len() as u64);
        for round in 0..40 {
            let text = reformat(src, &mut rng);
            let c = check(&text);
            let got = c.compiled.unwrap_or_else(|| panic!("{name} round {round} no longer compiles:\n{text}\n{}", djlive::diag::render_all(&c.diagnostics, &text))).cid;
            assert_eq!(got, compiled.cid, "{name} round {round} forked:\n{text}");
        }
    }
}
