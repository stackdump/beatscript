//! `btwd selftest`: the CC0 corpus through the library in-process. Prints `ok` when every program checks,
//! formats to a fixed point, round-trips through a link and the cheat sheet and instruments are there.

use djengine::json::JsonValue;

const CORPUS: [(&str, &str); 6] = [
    ("hand-dubstep-140-v2", include_str!("../../btwapi/tests/corpus/hand-dubstep-140-v2.btw")),
    ("hand-dubstep-140-v3", include_str!("../../btwapi/tests/corpus/hand-dubstep-140-v3.btw")),
    ("hand-dubstep-140-v4", include_str!("../../btwapi/tests/corpus/hand-dubstep-140-v4.btw")),
    ("markers-demo", include_str!("../../btwapi/tests/corpus/markers-demo.btw")),
    ("matrix-130", include_str!("../../btwapi/tests/corpus/matrix-130.btw")),
    ("trance-138", include_str!("../../btwapi/tests/corpus/trance-138.btw")),
];

fn parse(s: &str) -> Result<JsonValue, String> {
    JsonValue::parse(s).map_err(|e| e.to_string())
}

pub fn run() -> Result<(), String> {
    for (name, src) in CORPUS {
        let c = parse(&btwapi::check_json(src))?;
        if c.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(format!("{name}: does not check: {}", c.get("diagnostics").map(JsonValue::js_stringified).unwrap_or_default()));
        }
        let cid = c.get("cid").and_then(JsonValue::as_str).ok_or("no cid")?.to_string();
        let canonical = c.get("canonical").and_then(JsonValue::as_str).ok_or("no canonical text")?.to_string();
        // the canonical text is a fixed point with the same CID
        let again = parse(&btwapi::check_json(&canonical))?;
        if again.get("cid").and_then(JsonValue::as_str) != Some(cid.as_str()) {
            return Err(format!("{name}: the canonical text has another CID"));
        }
        // a link round trip
        let share = parse(&btwapi::share_json(src, None, false, ""))?;
        let url = share.get("url").and_then(JsonValue::as_str).ok_or_else(|| format!("{name}: no link"))?.to_string();
        let open = parse(&btwapi::open_json(&url))?;
        if open.get("verified").and_then(JsonValue::as_bool) != Some(true) || open.get("cid").and_then(JsonValue::as_str) != Some(cid.as_str()) {
            return Err(format!("{name}: the link does not open to the same program"));
        }
    }
    let sheet = btwapi::cheatsheet();
    if sheet.len() < 4000 {
        return Err(format!("the cheat sheet is only {} bytes", sheet.len()));
    }
    let ins = parse(&btwapi::instruments_json())?;
    if ins.get("voices").and_then(JsonValue::as_array).map_or(true, |v| v.len() < 10) {
        return Err("no instruments".into());
    }
    println!("ok ({} programs)", CORPUS.len());
    Ok(())
}
