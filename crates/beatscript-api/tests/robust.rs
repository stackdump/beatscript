//! No text makes beatscript-api panic (a panic in wasm is an abort of the page's module), and what it says about any
//! text is consistent: diagnostics point inside the text, UTF-16 offsets slice the same characters, a good
//! program's canonical text is a fixed point, a good public program's link opens to it.

mod support;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use djengine::json::JsonValue;
use support::*;

const VOCAB: &[&str] = &[
    "deck", "A", "B", "C", "master", "kick", "snare", "hihat", "clap", "bass", "melody", "harmony", "arp", "x", "~", "e", "(", ")", "[", "]", "<", ">", ",", "|", "*", "!", "^", "=", "vel", "oct", "len", "inst", "seed", "kit", "sound", "route", "tempo", "swing",
    "key", "let", "def", "repeat", "section", "arrange", "import", "end", "loop", "evolve", "every", "bars", "bar", "phrase", "on", "at", "after", "before", "for", "over", "->", "..", ":", "{", "}", "$a", "$i", "0", "1", "3", "8", "16", "64", "0.5", "2k", "50%",
    "999999999", "-1", "c2", "f#3", "bb4", "am7", "am", "d:dorian", "\"x\"", "\"héllo 🎵\"", "✓", "é", "🎵", "\n", "\n", "\n", "\r\n", "\t", " ", " ", " ", "#", "# note", "sample", "use", "restricted", "from", "license", "else", "do", "vocal", "create", "filter", "lp", "reverb",
    "level", "stop", "when", "gen", "gens", "%", "==", "rotate", "transpose", "thin", "wobble-lead", "drums-808", "section a 4 bars {", "}", "arrange", "chord(am)", "markov(am, 0.7)", "e(5,8)", "x*8", "[x x]", "<a b>", "\0", "\u{7f}", "\u{2028}", "/", "\\",
];

/// Everything beatscript-api does with a text, and the invariants that must hold.
fn exercise(text: &str) {
    let r = parse(&beatscript_api::check_json(text));
    assert_report_invariants(text, &r);
    let f = parse(&beatscript_api::fmt_json(text));
    assert_eq!(b(&f, "ok"), b(&r, "ok"));
    assert_eq!(f.get("diagnostics").unwrap().as_array().unwrap().len(), r.get("diagnostics").unwrap().as_array().unwrap().len());
    for d in f.get("diagnostics").unwrap().as_array().unwrap() {
        assert_diagnostic_invariants(text, d);
    }
    if b(&r, "ok") {
        // the canonical text is a fixed point with the same CID and the same nets
        let c = s(&r, "canonical");
        let again = parse(&beatscript_api::check_json(c));
        assert!(b(&again, "ok"), "the canonical text does not check:\n{c}");
        assert_eq!(s(&again, "cid"), s(&r, "cid"), "the canonical text has another CID:\n{text}\n--\n{c}");
        assert_eq!(s(&again, "canonical"), c);
        assert_eq!(get(&again, "nets").js_stringified(), get(&r, "nets").js_stringified());
        assert_eq!(s(&f, "canonical"), c);
        // a public program's link opens to it
        let sh = parse(&beatscript_api::share_json(text, None, true, ""));
        if b(&r, "restricted") {
            assert!(!b(&sh, "ok"));
        } else {
            assert!(b(&sh, "ok"), "{}", sh.js_stringified());
            let o = parse(&beatscript_api::open_json(s(&sh, "url")));
            assert!(b(&o, "ok") && b(&o, "verified"), "{}", o.js_stringified());
            assert_eq!(s(&o, "cid"), s(&r, "cid"));
            assert_eq!(s(&o, "canonical"), c);
        }
    } else {
        assert!(!b(&parse(&beatscript_api::share_json(text, None, false, "")), "ok"));
    }
    // highlighting: in order, inside the text, whole characters, and the UTF-16 form slices the same tokens
    let spans = parse(&beatscript_api::highlight_json(text));
    let spans16 = parse(&beatscript_api::highlight_json_utf16(text));
    let u16s: Vec<u16> = text.encode_utf16().collect();
    let (a, a16) = (spans.as_array().unwrap(), spans16.as_array().unwrap());
    assert_eq!(a.len(), a16.len());
    let mut at = 0;
    for (x, y) in a.iter().zip(a16) {
        let (s0, e0) = (n(x, "0") as usize, n(x, "1") as usize);
        assert!(s0 >= at && s0 < e0 && e0 <= text.len() && text.is_char_boundary(s0) && text.is_char_boundary(e0), "span {s0}..{e0} (the one before ended at {at}, len {}) in {text:?}", text.len());
        at = e0;
        assert_eq!(String::from_utf16(&u16s[n(y, "0") as usize..n(y, "1") as usize]).unwrap(), &text[s0..e0]);
    }
    let _ = beatscript_api::nets_json(text, false);
    let _ = beatscript_api::generation_json(text, None, 5);
    let _ = beatscript_api::open_json(text);
}

fn run_all(inputs: Vec<String>, what: &str) {
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut slowest = (0.0f64, String::new());
    for text in inputs {
        let t0 = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| exercise(&text)));
        let took = t0.elapsed().as_secs_f64();
        if took > slowest.0 {
            slowest = (took, text.chars().take(120).collect());
        }
        if let Err(e) = r {
            let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            failures.push((text, msg));
        }
    }
    eprintln!("{what}: slowest input {:.3}s: {:?}", slowest.0, slowest.1);
    assert!(slowest.0 < 5.0, "{what}: one text took {:.1}s: {:?}", slowest.0, slowest.1);
    if let Some((text, msg)) = failures.first() {
        panic!("{what}: {} of the inputs failed; the first:\n{msg}\nin {text:?}", failures.len());
    }
}

#[test]
fn token_soup_never_panics_and_is_always_consistent() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let inputs: Vec<String> = (0..3000)
        .map(|_| {
            let n = 1 + rng.below(60);
            let mut t = String::new();
            for i in 0..n {
                t.push_str(rng.pick(VOCAB));
                if rng.below(4) != 0 && i + 1 < n {
                    t.push(' ');
                }
            }
            t.push('\n');
            t
        })
        .collect();
    run_all(inputs, "token soup");
}

#[test]
fn mutated_programs_never_panic_and_good_ones_are_fixed_points() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let base = all_programs();
    let mut inputs = Vec::new();
    for _ in 0..1500 {
        let mut t: Vec<char> = rng.pick(&base).1.chars().collect();
        for _ in 0..1 + rng.below(4) {
            if t.is_empty() {
                break;
            }
            let at = rng.below(t.len());
            match rng.below(6) {
                0 => {
                    t.remove(at);
                }
                1 => {
                    let n = (1 + rng.below(20)).min(t.len() - at);
                    t.drain(at..at + n);
                }
                2 => {
                    let ins: Vec<char> = rng.pick(VOCAB).chars().collect();
                    for (k, c) in ins.into_iter().enumerate() {
                        t.insert(at + k, c);
                    }
                }
                3 => {
                    let n = (1 + rng.below(30)).min(t.len() - at);
                    let dup: Vec<char> = t[at..at + n].to_vec();
                    for (k, c) in dup.into_iter().enumerate() {
                        t.insert(at + k, c);
                    }
                }
                4 => t.truncate(at),
                _ => {
                    let c = *rng.pick(&['é', '🎵', '✓', '{', '}', '"', '#', '|', '\r', '\u{0}']);
                    t[at] = c;
                }
            }
        }
        inputs.push(t.into_iter().collect());
    }
    run_all(inputs, "mutations");
}

#[test]
fn a_big_program_is_checked_in_reasonable_time() {
    // the site caps a program at 16 KB; a text four times that, and a deeply nested one, still answer
    let mut text = String::from("tempo 128\nkey am\n");
    for i in 0..900 {
        text.push_str(&format!("section s{i} 4 bars {{\n  deck A: kick x ~ x ~ | hihat x*8\n}}\n"));
    }
    text.push_str(&format!("arrange {}\n", (0..900).map(|i| format!("s{i}")).collect::<Vec<_>>().join(" ")));
    assert!(text.len() > 40_000);
    let t0 = Instant::now();
    let r = parse(&beatscript_api::check_json(&text));
    assert!(t0.elapsed().as_secs_f64() < 20.0, "{:?}", t0.elapsed());
    assert_eq!(b(&r, "ok"), n(&r, "errors") == 0);
    let deep = format!("{}x{}", "[".repeat(400), "]".repeat(400));
    let r = parse(&beatscript_api::check_json(&format!("deck A: kick {deep}\n")));
    assert_eq!(b(&r, "ok"), n(&r, "errors") == 0);
    let r = parse(&beatscript_api::check_json(&format!("{}\n", "(".repeat(5000))));
    assert!(!b(&r, "ok"));
    let _ = JsonValue::Null;
}
