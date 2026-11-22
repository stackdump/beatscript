//! Golden LSP tests: a session of requests against [`btwls::Server`], each response compared with a file under
//! `tests/golden/`. `BTWLS_BLESS=1 cargo test -p btwls` writes the files from what the server answers now (then read the diff).

#![allow(dead_code)]

use btwls::Server;
use djengine::json::JsonValue;

pub const URI: &str = "file:///test.btw";

pub fn server(text: &str) -> Server {
    let mut s = Server::new();
    let init = JsonValue::parse(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-16"]}}}}"#).unwrap();
    s.handle(&init);
    s.open(URI, text, 1);
    s
}

pub fn req(s: &mut Server, method: &str, params: &str) -> JsonValue {
    let params = JsonValue::parse(params).unwrap_or_else(|e| panic!("{params}: {e}"));
    match s.request(method, &params) {
        Ok(v) => v,
        Err((code, m)) => panic!("{method}: error {code}: {m}"),
    }
}

/// Params naming the test document and a position.
pub fn at(line: u32, character: u32) -> String {
    format!(r#"{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":{line},"character":{character}}}}}"#)
}

pub fn doc() -> String {
    format!(r#"{{"textDocument":{{"uri":"{URI}"}}}}"#)
}

/// The (line, character) of the `nth` occurrence of `needle` in `text`, plus `skip` characters (ASCII texts).
pub fn find(text: &str, needle: &str, nth: usize, skip: u32) -> (u32, u32) {
    let at = text.match_indices(needle).nth(nth).unwrap_or_else(|| panic!("no {nth}th {needle:?}")).0;
    let line = text[..at].matches('\n').count() as u32;
    let col = (at - text[..at].rfind('\n').map_or(0, |n| n + 1)) as u32;
    (line, col + skip)
}

/// Pretty JSON, keys in the order the server wrote them: one value per line for arrays of objects, so diffs read.
pub fn pretty(v: &JsonValue) -> String {
    fn go(v: &JsonValue, ind: usize, out: &mut String) {
        let pad = "  ".repeat(ind);
        // a short value on one line (a position, a range, a small edit)
        let compact = v.js_stringified();
        if compact.len() <= 96 {
            out.push_str(&compact);
            return;
        }
        match v {
            JsonValue::Array(a) if a.is_empty() => out.push_str("[]"),
            JsonValue::Array(a) => {
                out.push_str("[\n");
                for (i, x) in a.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str("  ");
                    go(x, ind + 1, out);
                    if i + 1 < a.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push(']');
            }
            JsonValue::Object(o) if o.is_empty() => out.push_str("{}"),
            JsonValue::Object(o) => {
                out.push_str("{\n");
                let n = o.len();
                for (i, (k, x)) in o.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str("  ");
                    out.push_str(&JsonValue::str(k).js_stringified());
                    out.push_str(": ");
                    go(x, ind + 1, out);
                    if i + 1 < n {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push('}');
            }
            other => out.push_str(&other.js_stringified()),
        }
    }
    let mut out = String::new();
    go(v, 0, &mut out);
    out.push('\n');
    out
}

/// Compare `v` with `tests/golden/<name>.json` (write it under `BTWLS_BLESS=1`).
pub fn golden(name: &str, v: &JsonValue) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
    let got = pretty(v);
    if std::env::var_os("BTWLS_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no golden file {}: run with BTWLS_BLESS=1", path.display()));
    if got != want {
        panic!("{name}: the response is not the golden one ({}).\n--- got\n{got}\n--- want\n{want}", path.display());
    }
}
