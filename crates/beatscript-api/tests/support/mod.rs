//! Helpers shared by the beatscript-api test crates: the corpus, the extra programs, JSON reading and printing, a
//! tiny deterministic random source, and the invariants every diagnostic must satisfy.
#![allow(dead_code)]

pub mod snapshot_gen;

use std::path::{Path, PathBuf};

use djengine::json::JsonValue;

pub fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(sub)
}

fn read_all(sub: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(dir(sub))
        .unwrap_or_else(|e| panic!("tests/{sub}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "beatscript"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    v.sort();
    v
}

/// The CC0 programs of tests/corpus (the same ones the site ships as examples).
pub fn corpus() -> Vec<(String, String)> {
    read_all("corpus")
}

/// Programs written for these tests: do-lines, ramps, transforms, evolve, sections, assets, settings.
pub fn extras() -> Vec<(String, String)> {
    read_all("extra")
}

pub fn all_programs() -> Vec<(String, String)> {
    let mut v = corpus();
    v.extend(extras());
    v
}

pub fn parse(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap_or_else(|e| panic!("not JSON ({e}): {}", &s[..s.len().min(300)]))
}

pub fn get<'a>(v: &'a JsonValue, path: &str) -> &'a JsonValue {
    let mut cur = v;
    for k in path.split('.') {
        cur = match k.parse::<usize>() {
            Ok(i) => cur.at(i),
            Err(_) => cur.get(k),
        }
        .unwrap_or_else(|| panic!("no `{k}` in {path} of {}", cur.js_stringified()));
    }
    cur
}

pub fn s<'a>(v: &'a JsonValue, path: &str) -> &'a str {
    get(v, path).as_str().unwrap_or_else(|| panic!("{path} is not a string: {}", get(v, path).js_stringified()))
}

pub fn n(v: &JsonValue, path: &str) -> i64 {
    get(v, path).as_i64().unwrap_or_else(|| panic!("{path} is not a number"))
}

pub fn b(v: &JsonValue, path: &str) -> bool {
    get(v, path).as_bool().unwrap_or_else(|| panic!("{path} is not a bool"))
}

/// JSON printed one key a line, scalars and short arrays inline: what a golden file holds.
pub fn pretty(v: &JsonValue) -> String {
    let mut out = String::new();
    write_pretty(v, 0, &mut out);
    out.push('\n');
    out
}

fn write_pretty(v: &JsonValue, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth + 1);
    match v {
        JsonValue::Array(a) if !a.is_empty() && a.iter().all(|x| !matches!(x, JsonValue::Array(_) | JsonValue::Object(_))) && v.js_stringified().len() < 100 => out.push_str(&v.js_stringified()),
        JsonValue::Array(a) if !a.is_empty() => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                out.push_str(&pad);
                write_pretty(x, depth + 1, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            out.push_str(&"  ".repeat(depth));
            out.push(']');
        }
        JsonValue::Object(o) if !o.is_empty() => {
            out.push_str("{\n");
            let n = o.len();
            for (i, (k, x)) in o.iter().enumerate() {
                out.push_str(&pad);
                out.push_str(&JsonValue::str(k).js_stringified());
                out.push_str(": ");
                write_pretty(x, depth + 1, out);
                out.push_str(if i + 1 < n { ",\n" } else { "\n" });
            }
            out.push_str(&"  ".repeat(depth));
            out.push('}');
        }
        _ => out.push_str(&v.js_stringified()),
    }
}

/// xorshift: a repeatable stream of numbers for the property tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

/// A diagnostic as `check` gives it must agree with the text it is about: byte offsets on character
/// boundaries, UTF-16 offsets that slice the same characters out of a JS string, line and column (1-based,
/// in characters) of the start.
pub fn assert_diagnostic_invariants(text: &str, d: &JsonValue) {
    let (start, end) = (n(d, "start") as usize, n(d, "end") as usize);
    assert!(start <= end && end <= text.len(), "span {start}..{end} of {}: {}", text.len(), d.js_stringified());
    assert!(text.is_char_boundary(start) && text.is_char_boundary(end), "span {start}..{end} is inside a character: {}", d.js_stringified());
    let u16s: Vec<u16> = text.encode_utf16().collect();
    let (s16, e16) = (n(d, "start16") as usize, n(d, "end16") as usize);
    assert!(s16 <= e16 && e16 <= u16s.len(), "utf16 span {s16}..{e16} of {}", u16s.len());
    assert_eq!(String::from_utf16(&u16s[s16..e16]).expect("utf16 slice is whole characters"), &text[start..end], "utf16 and utf8 spans cut different text");
    // the desktop's rule (`djlive::diag::line_col`): a line ends at `\n` or at a `\r` that no `\n` follows, and a BOM at the very
    // start is not a column
    let (line, col) = djlive::diag::line_col(text, start);
    assert_eq!((n(d, "line") as usize, n(d, "col") as usize), (line, col), "line:col of {}", d.js_stringified());
    assert!(n(d, "end_col") as usize > col - 1, "end_col");
    assert!(matches!(s(d, "severity"), "error" | "warning"));
    assert!(!s(d, "message").is_empty());
    assert!(!s(d, "render").is_empty());
}

pub fn assert_report_invariants(text: &str, report: &JsonValue) {
    let ds = get(report, "diagnostics").as_array().expect("diagnostics");
    for d in ds {
        assert_diagnostic_invariants(text, d);
    }
    let errors = ds.iter().filter(|d| s(d, "severity") == "error").count() as i64;
    assert_eq!(n(report, "errors"), errors);
    assert_eq!(n(report, "warnings"), ds.len() as i64 - errors);
    assert_eq!(b(report, "ok"), errors == 0, "ok iff no errors");
    if b(report, "ok") {
        assert!(get(report, "cid").as_str().is_some() && get(report, "canonical").as_str().is_some());
    } else {
        assert!(matches!(get(report, "cid"), JsonValue::Null) && matches!(get(report, "canonical"), JsonValue::Null));
    }
}
