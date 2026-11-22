//! Small helpers over djengine's JSON (the crate the rest of the language speaks; no serde here, so the wasm module stays
//! one JSON implementation).

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::Span;

use crate::text::{Encoding, LineIndex, Position, Range};

pub fn obj<const N: usize>(pairs: [(&str, JsonValue); N]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

pub fn arr(v: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(v)
}

pub fn s(v: impl Into<String>) -> JsonValue {
    JsonValue::str(v.into())
}

pub fn int(n: usize) -> JsonValue {
    JsonValue::int(n as i64)
}

pub fn pos_json(p: Position) -> JsonValue {
    obj([("line", JsonValue::int(i64::from(p.line))), ("character", JsonValue::int(i64::from(p.character)))])
}

pub fn range_json(r: Range) -> JsonValue {
    obj([("start", pos_json(r.start)), ("end", pos_json(r.end))])
}

/// A span of `text` as an LSP range.
pub fn span_range(ix: &LineIndex, text: &str, sp: Span, enc: Encoding) -> JsonValue {
    range_json(ix.range(text, sp, enc))
}

pub fn get_pos(v: &JsonValue) -> Option<Position> {
    Some(Position { line: v.get("line")?.as_f64()? as u32, character: v.get("character")?.as_f64()? as u32 })
}

pub fn get_range(v: &JsonValue) -> Option<Range> {
    Some(Range { start: get_pos(v.get("start")?)?, end: get_pos(v.get("end")?)? })
}

pub fn get_str<'a>(v: &'a JsonValue, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str()
}

/// A `TextEdit`.
pub fn text_edit(ix: &LineIndex, text: &str, sp: Span, new_text: &str, enc: Encoding) -> JsonValue {
    obj([("range", span_range(ix, text, sp, enc)), ("newText", s(new_text))])
}
