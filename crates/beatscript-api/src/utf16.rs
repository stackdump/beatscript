//! UTF-8 byte offsets to UTF-16 code-unit offsets, so JS can slice the string it gave us.

use djengine::json::JsonValue;

/// A map from byte offsets of one text to UTF-16 offsets. An offset inside a character counts as the
/// character's start; one past the end counts as the end.
pub struct Utf16Index {
    /// None for an ASCII text (the offsets are the same).
    map: Option<Vec<u32>>,
    len: usize,
}

impl Utf16Index {
    pub fn new(text: &str) -> Utf16Index {
        if text.is_ascii() {
            return Utf16Index { map: None, len: text.len() };
        }
        let mut map = vec![0u32; text.len() + 1];
        let mut units = 0u32;
        for (b, c) in text.char_indices() {
            for k in 0..c.len_utf8() {
                map[b + k] = units;
            }
            units += c.len_utf16() as u32;
        }
        map[text.len()] = units;
        Utf16Index { map: Some(map), len: text.len() }
    }

    /// The UTF-16 offset of byte offset `at`.
    pub fn at(&self, at: usize) -> usize {
        let at = at.min(self.len);
        match &self.map {
            None => at,
            Some(m) => m[at] as usize,
        }
    }
}

/// Give every object that has a numeric `start` and `end` (byte offsets into `text`) the same span in
/// UTF-16 code units, as `start16` and `end16`. Objects that already have them are left
/// alone. For JSON the runtime builds from spans (marks, in the wasm layer).
pub fn add_utf16_spans(v: &mut JsonValue, text: &str) {
    let ix = Utf16Index::new(text);
    walk(v, &ix);
}

fn walk(v: &mut JsonValue, ix: &Utf16Index) {
    match v {
        JsonValue::Array(a) => a.iter_mut().for_each(|x| walk(x, ix)),
        JsonValue::Object(o) => {
            let keys: Vec<String> = o.keys().map(str::to_string).collect();
            for k in &keys {
                if let Some(c) = o.get_mut(k) {
                    walk(c, ix);
                }
            }
            if !o.contains_key("start16") {
                if let (Some(s), Some(e)) = (o.get("start").and_then(JsonValue::as_f64), o.get("end").and_then(JsonValue::as_f64)) {
                    if s >= 0.0 && e >= s && s.fract() == 0.0 && e.fract() == 0.0 {
                        let (s16, e16) = (ix.at(s as usize), ix.at(e as usize));
                        // appended at the end of the object
                        o.set("start16", JsonValue::int(s16 as i64));
                        o.set("end16", JsonValue::int(e16 as i64));
                    }
                }
            }
        }
        _ => {}
    }
}
