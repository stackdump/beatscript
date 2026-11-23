//! Generates `data/snapshot.json` from djhost's control-language registry and the engine's catalog:
//! the static data beatscript-api checks programs against, so the pure crate need not depend on djhost.
//! Shared by `examples/gen_snapshot.rs` (writes the file) and `tests/snapshot.rs` (fails on drift).
#![allow(dead_code)]

use djengine::json::{JsonObject, JsonValue};
use djhost::beatscript_env::{CatalogView, RegistryView};
use djhost::control_registry;
use djlive::env::{Catalog, ParamSpec, Registry};

fn strs(v: &[String]) -> JsonValue {
    JsonValue::Array(v.iter().map(|s| JsonValue::str(s.clone())).collect())
}

fn param_json(p: &ParamSpec) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("name", JsonValue::str(p.name.clone()));
    o.set("types", strs(&p.types));
    if !p.options.is_empty() {
        o.set("options", strs(&p.options));
    }
    if p.required {
        o.set("required", JsonValue::Bool(true));
    }
    if let Some(m) = p.minimum {
        o.set("minimum", JsonValue::Number(m));
    }
    if let Some(m) = p.maximum {
        o.set("maximum", JsonValue::Number(m));
    }
    JsonValue::Object(o)
}

/// One noun, as one line of the snapshot.
fn noun_json(n: &control_registry::Noun) -> JsonValue {
    let spec = RegistryView.noun(n.name).expect("every noun of the table resolves");
    let mut o = JsonObject::new();
    o.set("name", JsonValue::str(spec.name.clone()));
    o.set("default_action", JsonValue::str(spec.default_action.clone()));
    if n.local {
        o.set("local", JsonValue::Bool(true));
    }
    if !n.aliases.is_empty() {
        o.set("aliases", JsonValue::Array(n.aliases.iter().map(|a| JsonValue::str(*a)).collect()));
    }
    let actions: Vec<JsonValue> = spec
        .actions
        .iter()
        .map(|a| {
            let mut ao = JsonObject::new();
            ao.set("name", JsonValue::str(a.name.clone()));
            ao.set("effect", JsonValue::str(a.effect.clone()));
            if !a.params.is_empty() {
                ao.set("params", JsonValue::Array(a.params.iter().map(param_json).collect()));
            }
            JsonValue::Object(ao)
        })
        .collect();
    o.set("actions", JsonValue::Array(actions));
    o.set("params", JsonValue::Array(spec.params.iter().map(param_json).collect()));
    JsonValue::Object(o)
}

/// The snapshot file's text: one noun a line, so a change reads as a diff.
pub fn generate() -> String {
    let c = CatalogView;
    let line = |k: &str, v: JsonValue| format!("  {}: {}", JsonValue::str(k).js_stringified(), v.js_stringified());
    let mut out = String::from("{\n");
    out.push_str(&line("version", JsonValue::int(1)));
    out.push_str(",\n");
    out.push_str(&line("note", JsonValue::str("Generated from djhost's control registry and the engine's catalog by crates/beatscript-api/examples/gen_snapshot.rs (BEATSCRIPT_BLESS=1 cargo test -p beatscript-api --test snapshot). Do not edit.")));
    out.push_str(",\n");
    out.push_str(&line("voices", strs(&c.voices())));
    out.push_str(",\n");
    out.push_str(&line("families", strs(&c.families())));
    out.push_str(",\n");
    out.push_str(&line("kits", strs(&c.kits())));
    out.push_str(",\n");
    out.push_str(&line("palettes", strs(&c.palettes())));
    out.push_str(",\n  \"nouns\": [\n");
    let nouns = control_registry::nouns();
    for (i, n) in nouns.iter().enumerate() {
        out.push_str("    ");
        out.push_str(&noun_json(n).js_stringified());
        out.push_str(if i + 1 < nouns.len() { ",\n" } else { "\n" });
    }
    out.push_str("  ]\n}\n");
    out
}
