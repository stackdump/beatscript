//! project-datum 6679: every shipped feature is reachable and described through MCP.
//!
//! The table is `testdata/exposure.json`, one row per feature: `say` (text its tool's description or input schema must
//! contain, as tools/list shows it) and `calls` (real calls through [`Tools::dispatch`] and what each answer must carry).
//! crates/beatscript-serve/deploy/mcp-exposure.py runs the same table live over HTTPS in every beatscript.app deploy script.
//! Every page in docs/beatscript/*.md is some row's `docs` or a `not_features` entry with its reason, so a feature documented
//! there without a row fails here.

use super::tests::TakesAll;
use super::*;
use std::collections::HashMap;

const TABLE: &str = include_str!("../testdata/exposure.json");
const DOCS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/beatscript");

/// `${name}` in a string replaced by what an earlier call saved, in every string of `v`.
fn fill(v: &Value, saved: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => Value::String(saved.iter().fold(s.clone(), |s, (k, x)| s.replace(&format!("${{{k}}}"), x))),
        Value::Array(a) => Value::Array(a.iter().map(|x| fill(x, saved)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), fill(x, saved))).collect()),
        x => x.clone(),
    }
}

/// One `[pointer, op, value]` against an answer: Err says what was wrong.
fn expect(ans: &Value, e: &Value, saved: &HashMap<String, String>) -> Result<(), String> {
    let e = fill(e, saved);
    let ptr = e[0].as_str().ok_or("expect: a pointer")?;
    let op = e[1].as_str().ok_or("expect: an op")?;
    let got = ans.pointer(ptr);
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let ok = match op {
        "=" => got == Some(&e[2]),
        "has" => got.is_some_and(|v| !v.is_null()),
        "~" => got.is_some_and(|v| text(v).contains(&text(&e[2]))),
        "^" => got.is_some_and(|v| text(v).starts_with(&text(&e[2]))),
        "!~" => !got.is_some_and(|v| text(v).contains(&text(&e[2]))),
        _ => return Err(format!("expect: unknown op {op}")),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{ptr} {op} {}: got {}", e.get(2).map(text).unwrap_or_default(), got.map(text).unwrap_or_else(|| "nothing".into())))
    }
}

/// One call of a row's, as one signed-in caller. The checker allows a caller 60 calls a minute, and the table makes more than that
/// (68 with nested's and acid's rows): each row counts against a budget of its own (`row`, the limiter's key; the same account
/// and address), so a row is never refused for the rows before it.
fn call(t: &Tools, row: &str, name: &str, args: Value) -> Result<Value, String> {
    let c = Caller { uid: Some("u-exposure".into()), ip: "127.0.0.1".into(), key: format!("u:u-exposure:{row}") };
    let Value::Object(m) = args else { return Err("args: an object".into()) };
    match t.dispatch(name, &c, Args(m)) {
        Ok(Text::Json(v)) => Ok(v),
        Ok(Text::Raw(s)) => serde_json::from_str(&s).map_err(|e| format!("{name}: not JSON: {e}")),
        // a page (artifact): its summary, with the page as `html` (deploy/mcp-exposure.py reads the two blocks so too)
        Ok(Text::Page(mut v, html)) => {
            v["html"] = json!(html);
            Ok(v)
        }
        Err(e) => Err(format!("{name}: {}", e.err.message)),
    }
}

#[test]
fn every_shipped_feature_is_described_and_reachable_through_mcp() {
    let table: Value = serde_json::from_str(TABLE).expect("exposure.json");
    let features = table["features"].as_array().expect("features");
    let tools: HashMap<String, String> = tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30))
        .into_iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), format!("{}\n{}", t["description"].as_str().unwrap(), t["inputSchema"])))
        .collect();

    // a documented feature has a row: every docs/beatscript page is claimed by one, or said not to be an MCP feature
    let mut claimed: Vec<String> = features.iter().flat_map(|f| f["docs"].as_array().unwrap().iter().map(|d| d.as_str().unwrap().to_string())).collect();
    claimed.extend(table["not_features"].as_object().unwrap().keys().cloned());
    let mut pages: Vec<String> = std::fs::read_dir(DOCS_DIR).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".md")).collect();
    pages.sort();
    let missing: Vec<&String> = pages.iter().filter(|p| !claimed.contains(p)).collect();
    assert!(missing.is_empty(), "docs/beatscript pages with no row in mcp/testdata/exposure.json (add a feature row, or a not_features entry saying why it is not one): {missing:?}");
    let stale: Vec<&String> = claimed.iter().filter(|c| !pages.contains(c)).collect();
    assert!(stale.is_empty(), "exposure.json names docs/beatscript pages that do not exist: {stale:?}");

    let dir = std::env::temp_dir().join(format!("btw-mcp-exposure-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let app = crate::test_app(&dir);
    crate::ontology::install_genres();
    let mut t = Tools::new(&app);
    t.set_hub(Arc::new(TakesAll));

    let mut fails = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in features {
        let id = f["id"].as_str().unwrap();
        assert!(seen.insert(id.to_string()), "two rows are {id}");
        let say = f["say"].as_array().unwrap();
        let calls = f["calls"].as_array().unwrap();
        assert!(!say.is_empty() && !calls.is_empty(), "{id}: a row needs what the tools say and a call that carries it");
        for s in say {
            let (tool, text) = (s[0].as_str().unwrap(), s[1].as_str().unwrap());
            match tools.get(tool) {
                Some(d) if d.contains(text) => {}
                Some(_) => fails.push(format!("{id}: {tool}'s description and schema do not say {text:?}")),
                None => fails.push(format!("{id}: no tool {tool}")),
            }
        }
        let mut saved = HashMap::new();
        for c in calls {
            let tool = c["tool"].as_str().unwrap();
            let ans = match call(&t, id, tool, fill(&c["args"], &saved)) {
                Ok(a) => a,
                Err(e) => {
                    fails.push(format!("{id}: {e}"));
                    continue;
                }
            };
            for (k, p) in c.get("save").and_then(Value::as_object).into_iter().flatten() {
                saved.insert(k.clone(), ans.pointer(p.as_str().unwrap()).and_then(Value::as_str).unwrap_or("").to_string());
            }
            for e in c["expect"].as_array().unwrap() {
                if let Err(e) = expect(&ans, e, &saved) {
                    fails.push(format!("{id}: {tool}: {e}"));
                }
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(fails.is_empty(), "features not exposed through MCP as exposure.json says:\n  {}", fails.join("\n  "));
}
