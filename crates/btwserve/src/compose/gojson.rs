//! encoding/json as the Go server writes it, for the P7 routes (compose, admin, MCP): a writer over serde_json
//! values that formats floats as Go's `float64` encoder does (`120`, `0.008265`, `1e-7`, `1e+21`), escapes `<`, `>`,
//! `&`, U+2028 and U+2029 (Go's HTML-safe default), and can indent as `json.MarshalIndent(v, "", " ")`. Object keys
//! come out in the order the value holds them (serde_json's `preserve_order`): struct order is insertion order, and a
//! Go map is built sorted with `sorted`.
//!
//! Also the clock: RFC 3339 and RFC 3339 with nanoseconds (Go's `time.RFC3339Nano`, trailing zeros trimmed), the UTC
//! day as `2006-01-02` and `20060102`.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// `json.Marshal(v)`.
pub fn to_string(v: &Value) -> String {
    let mut s = String::new();
    write(&mut s, v, None, 0);
    s
}

/// `json.NewEncoder(w).Encode(v)`: Marshal plus a newline.
pub fn line(v: &Value) -> String {
    let mut s = to_string(v);
    s.push('\n');
    s
}

/// `json.MarshalIndent(v, "", indent)`.
pub fn indent(v: &Value, ind: &str) -> String {
    let mut s = String::new();
    write(&mut s, v, Some(ind), 0);
    s
}

fn newline(out: &mut String, ind: Option<&str>, depth: usize) {
    if let Some(i) = ind {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(i);
        }
    }
}

fn write(out: &mut String, v: &Value, ind: Option<&str>, depth: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let _ = write!(out, "{i}");
            } else if let Some(u) = n.as_u64() {
                let _ = write!(out, "{u}");
            } else {
                out.push_str(&float(n.as_f64().unwrap_or(0.0)));
            }
        }
        Value::String(s) => string(out, s),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, ind, depth + 1);
                write(out, x, ind, depth + 1);
            }
            newline(out, ind, depth);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, ind, depth + 1);
                string(out, k);
                out.push(':');
                if ind.is_some() {
                    out.push(' ');
                }
                write(out, x, ind, depth + 1);
            }
            newline(out, ind, depth);
            out.push('}');
        }
    }
}

/// Go's float64 encoding: shortest repr, `f` format, `e` below 1e-6 and from 1e21 (`1e-7`, `1e+21`).
pub fn float(f: f64) -> String {
    if !f.is_finite() {
        return "null".into();
    }
    let a = f.abs();
    if a != 0.0 && !(1e-6..1e21).contains(&a) {
        let s = format!("{f:e}");
        // Rust writes 1e21 / 1e-7; Go writes 1e+21 / 1e-7
        match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        }
    } else {
        let s = format!("{f}");
        if s == "-0" {
            "-0".into()
        } else {
            s
        }
    }
}

fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A JSON float the writer formats as Go does (`json!(x)` of an f64; `0.0` becomes `0`).
pub fn num(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// An object with its keys sorted (a Go `map[string]…`), shallow.
pub fn sorted(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut e: Vec<(String, Value)> = m.into_iter().collect();
            e.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(e.into_iter().collect())
        }
        v => v,
    }
}

/// `sorted`, at every depth.
pub fn sorted_deep(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut e: Vec<(String, Value)> = m.into_iter().map(|(k, v)| (k, sorted_deep(v))).collect();
            e.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(e.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted_deep).collect()),
        v => v,
    }
}

/// An ordered object from (key, value) pairs (a Go struct).
pub fn obj<const N: usize>(fields: [(&str, Value); N]) -> Value {
    let mut m = Map::new();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

// ---------- time ----------

/// Unix seconds and nanoseconds now.
pub fn now() -> (i64, u32) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_nanos())
}

/// (year, month, day, hour, minute, second) of unix seconds, UTC.
pub fn civil(s: i64) -> (i64, u32, u32, u32, u32, u32) {
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

/// `t.UTC().Format(time.RFC3339)`.
pub fn rfc3339(s: i64) -> String {
    let (y, mo, d, h, mi, se) = civil(s);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z")
}

/// `t.UTC().Format(time.RFC3339Nano)`: the fraction without trailing zeros, none at all for a whole second.
pub fn rfc3339_nano(s: i64, ns: u32) -> String {
    let base = rfc3339(s);
    if ns == 0 {
        return base;
    }
    let frac = format!("{ns:09}");
    format!("{}.{}Z", &base[..base.len() - 1], frac.trim_end_matches('0'))
}

/// `2006-01-02`, UTC.
pub fn day(s: i64) -> String {
    let (y, m, d, ..) = civil(s);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `20060102`, UTC.
pub fn day_compact(s: i64) -> String {
    let (y, m, d, ..) = civil(s);
    format!("{y:04}{m:02}{d:02}")
}

/// Unix seconds of the next UTC midnight.
pub fn next_midnight(s: i64) -> i64 {
    (s.div_euclid(86_400) + 1) * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn floats_as_go() {
        assert_eq!(float(120.0), "120");
        assert_eq!(float(0.008265), "0.008265");
        assert_eq!(float(1e-7), "1e-7");
        assert_eq!(float(1.5e-7), "1.5e-7");
        assert_eq!(float(1e21), "1e+21");
        assert_eq!(float(1e20), "100000000000000000000");
        assert_eq!(float(0.000001), "0.000001");
        assert_eq!(float(2.0), "2");
        assert_eq!(to_string(&json!({"a": 0.0, "b": [1, 2.5], "c": "<&>\u{2028}"})), "{\"a\":0,\"b\":[1,2.5],\"c\":\"\\u003c\\u0026\\u003e\\u2028\"}");
    }

    #[test]
    fn indent_as_go() {
        let v = obj([("date", json!("2026-11-20")), ("usd", num(0.008265)), ("outcomes", json!({"ok": 1}))]);
        assert_eq!(indent(&v, " "), "{\n \"date\": \"2026-11-20\",\n \"usd\": 0.008265,\n \"outcomes\": {\n  \"ok\": 1\n }\n}");
    }

    #[test]
    fn times() {
        assert_eq!(rfc3339(1_795_214_474), "2026-11-20T22:41:14Z");
        assert_eq!(rfc3339_nano(1_795_214_474, 196_868_844), "2026-11-20T22:41:14.196868844Z");
        assert_eq!(rfc3339_nano(1_795_214_474, 100_000_000), "2026-11-20T22:41:14.1Z");
        assert_eq!(day(1_795_214_474), "2026-11-20");
        assert_eq!(day_compact(1_795_214_474), "20261120");
    }
}
