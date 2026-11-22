//! The control language's registry: one table of every noun djapp's control socket answers, its
//! verbs and their params. Everything that lists the surface is generated from it, so nothing
//! can drift from it:
//!
//! - `describe` (the socket method, and the MCP tool `djapp_describe`) answers from it;
//! - `djctl app-mcp`'s `tools/list` is [`tools`]: one tool per noun, named `djapp_<noun>`;
//! - `docs/CONTROL.md`'s "Methods at a glance" is [`glance_markdown`] (a test fails when the
//!   file is stale; `DJ_REGEN_DOCS=1 cargo test -p djhost control_registry` rewrites it);
//! - djapp's tests send every `(noun, action)` it lists to a real socket and check the app
//!   answers it, and that the app's own "unknown action" choices are exactly this list.
//!
//! # The shape
//!
//! ```text
//! request   {"id": 1, "method": <noun>, "params": {"action": <verb>, ...params}}
//!           {"id": 1, "method": <noun>, "action": <verb>, ...params}      (params inline)
//!           {"id": 1, "method": "<noun>.<verb>", "params": {...}}          (dotted)
//! reply     {"id": 1, "result": ...}   or   {"id": 1, "error": "<noun>: <what went wrong>"}
//! reads     state {section: <noun>}  ·  <noun> {action: status | list | get}  ·  describe
//! ```
//!
//! A noun is what a panel or a piece of gear is (`deck`, `mixer`, `loopsets`, `vocoder`); a verb
//! is `action`. Nouns that are a bag of knobs with no verbs (`mixer`, `input`, `aux`) take their
//! knobs as params and answer the state after. Every write answers the noun's state after it, so
//! a caller reads what its call did. `describe` lists every noun, verb and param.

use djengine::json::{JsonObject, JsonValue};

mod nouns;

/// What an action does to the booth, for a caller deciding whether to ask first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Reads, changes nothing.
    Read,
    /// Changes the app's state (what the window shows, what plays).
    Write,
    /// Reaches outside the process or the booth's ears: a device, the network, gear, a file, the
    /// speakers. Ask first unless the person asked for it.
    Live,
}

impl Effect {
    pub fn name(self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::Live => "live",
        }
    }
}

/// One parameter of an action (or of a noun with no verbs).
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    /// A JSON-schema type, or several joined by `|` (`string|null`).
    pub ty: &'static str,
    pub doc: &'static str,
    pub required: bool,
}

impl Param {
    pub const fn new(name: &'static str, ty: &'static str, doc: &'static str) -> Param {
        Param { name, ty, doc, required: false }
    }

    pub const fn req(mut self) -> Param {
        self.required = true;
        self
    }
}

/// One verb of a noun.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub name: &'static str,
    pub doc: &'static str,
    pub effect: Effect,
    pub params: &'static [Param],
    /// The params the round-trip test sends (JSON object text; `action` is added). Empty: `{}`.
    /// An action that must not run for real in a test (it writes a file, touches gear) names
    /// params the app refuses before it acts; `"!"`: one that has none (it reaches the network
    /// with no params) is never sent, and only its name is checked.
    pub probe: &'static str,
}

impl Action {
    pub const fn new(name: &'static str, doc: &'static str, effect: Effect) -> Action {
        Action { name, doc, effect, params: &[], probe: "" }
    }

    pub const fn with(mut self, params: &'static [Param]) -> Action {
        self.params = params;
        self
    }

    pub const fn probe(mut self, probe: &'static str) -> Action {
        self.probe = probe;
        self
    }
}

/// One noun: a socket method, and the MCP tool that fronts it.
#[derive(Clone, Copy, Debug)]
pub struct Noun {
    /// The socket method.
    pub name: &'static str,
    /// The MCP tool (`djapp_<name>`), or None for a method that is not one.
    pub tool: Option<&'static str>,
    /// One line, for the glance table and `describe`'s list.
    pub summary: &'static str,
    /// The action taken when a call names none ("" when the noun has no verbs).
    pub default_action: &'static str,
    pub actions: &'static [Action],
    /// The long description (the MCP tool's).
    pub doc: &'static str,
    /// The MCP input schema as JSON text; "" generates one from `actions` and `params`.
    pub schema: &'static str,
    /// Params of a noun with no verbs (or shared by all its verbs).
    pub params: &'static [Param],
    /// Other names the socket accepts for this noun.
    pub aliases: &'static [&'static str],
    /// Served by the MCP server itself, not the socket (`render`).
    pub local: bool,
}

/// Every noun, in the order the docs list them.
pub fn nouns() -> &'static [Noun] {
    nouns::NOUNS
}

/// The noun a socket method or an alias names.
pub fn noun(name: &str) -> Option<&'static Noun> {
    nouns().iter().find(|n| n.name == name || n.aliases.contains(&name))
}

/// The noun an MCP tool fronts.
pub fn tool_noun(tool: &str) -> Option<&'static Noun> {
    nouns().iter().find(|n| n.tool == Some(tool))
}

/// The canonical socket method for `method` (an alias resolves), as the dispatcher spells it.
pub fn canonical(method: &str) -> Option<&'static str> {
    noun(method).map(|n| n.name)
}

/// The socket's methods, for `methods` and the unknown-method error (no local ones).
pub fn methods() -> Vec<&'static str> {
    nouns().iter().filter(|n| !n.local).map(|n| n.name).collect()
}

impl Noun {
    pub fn action(&self, name: &str) -> Option<&'static Action> {
        self.actions.iter().find(|a| a.name == name)
    }

    /// The input schema: the hand-written one, or one generated from the verbs and params.
    pub fn input_schema(&self) -> JsonValue {
        if !self.schema.is_empty() {
            return JsonValue::parse(self.schema).expect("a registry schema parses");
        }
        generated_schema(self)
    }

    /// The params a call to this noun may carry, as (name, type, doc): the schema's properties.
    pub fn param_list(&self) -> Vec<(String, String, String)> {
        let schema = self.input_schema();
        let Some(props) = schema.get("properties").and_then(JsonValue::as_object) else {
            return Vec::new();
        };
        props
            .iter()
            .map(|(k, v)| {
                let ty = match v.get("type") {
                    Some(JsonValue::String(t)) => t.clone(),
                    Some(JsonValue::Array(ts)) => {
                        ts.iter().filter_map(JsonValue::as_str).collect::<Vec<_>>().join("|")
                    }
                    _ => "any".into(),
                };
                let doc = v.get("description").and_then(JsonValue::as_str).unwrap_or("");
                (k.to_string(), ty, doc.to_string())
            })
            .collect()
    }
}

fn obj<const N: usize>(pairs: [(&str, JsonValue); N]) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

fn strs<'a>(it: impl IntoIterator<Item = &'a str>) -> JsonValue {
    JsonValue::Array(it.into_iter().map(JsonValue::str).collect())
}

fn type_json(ty: &str) -> JsonValue {
    if ty.contains('|') {
        strs(ty.split('|'))
    } else {
        JsonValue::str(ty)
    }
}

fn param_json(p: &Param) -> JsonValue {
    let mut o = JsonObject::new();
    o.set("type", type_json(p.ty));
    if !p.doc.is_empty() {
        o.set("description", JsonValue::str(p.doc));
    }
    JsonValue::Object(o)
}

/// A schema from `actions` and `params`: `action` is the enum of the verbs, every other
/// property the union of the verbs' params (the first doc wins).
fn generated_schema(n: &Noun) -> JsonValue {
    let mut props = JsonObject::new();
    if !n.actions.is_empty() {
        let mut a = JsonObject::new();
        a.set("type", "string".into());
        a.set("enum", strs(n.actions.iter().map(|a| a.name)));
        if !n.default_action.is_empty() {
            a.set("description", JsonValue::str(format!("default {}", n.default_action)));
        }
        props.set("action", JsonValue::Object(a));
    }
    for p in n.params.iter().chain(n.actions.iter().flat_map(|a| a.params.iter())) {
        if !props.contains_key(p.name) {
            props.set(p.name, param_json(p));
        }
    }
    let mut o = JsonObject::new();
    o.set("type", "object".into());
    o.set("properties", JsonValue::Object(props));
    JsonValue::Object(o)
}

/// The MCP `tools/list` entries: one per noun that has a tool.
pub fn tools() -> Vec<JsonValue> {
    nouns()
        .iter()
        .filter_map(|n| {
            let tool = n.tool?;
            let doc: String = n.doc.split_whitespace().collect::<Vec<_>>().join(" ");
            Some(obj([
                ("name", tool.into()),
                ("description", doc.into()),
                ("inputSchema", n.input_schema()),
            ]))
        })
        .collect()
}

fn action_json(a: &Action) -> JsonValue {
    obj([
        ("action", a.name.into()),
        ("doc", a.doc.into()),
        ("effect", a.effect.name().into()),
        (
            "params",
            JsonValue::Array(
                a.params
                    .iter()
                    .map(|p| {
                        obj([
                            ("name", p.name.into()),
                            ("type", p.ty.into()),
                            ("required", p.required.into()),
                            ("doc", p.doc.into()),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// What a call looks like, said once, for `describe` with no arguments.
const LANGUAGE: &str = "One request is {\"method\": <noun>, \"params\": {\"action\": <verb>, ...params}}; \
params may also sit beside method ({\"method\":\"loopsets\",\"action\":\"load_pair\",\"spec\":\"dnb:2602\",\"group\":2}), \
and \"<noun>.<verb>\" names both in the method. Reads: state {section: <noun>}, or a noun's own \
status / list / get. A write answers the noun's state after it. Errors read \"<noun>: <what>\" and \
name the choices. describe {noun} lists a noun's verbs and params, describe {noun, action} one verb. \
Effects: read changes nothing; write changes the app; live reaches the booth's speakers, gear, \
network or disk, so ask first unless asked. batch {commands: [...]} runs several commands as one \
request: all or nothing (atomic), and landing together on the bar grid (at: next_bar).";

/// `describe`: no params → the language and every noun; `noun` → its verbs and params;
/// `noun` + `action` → one verb; `search` → nouns and verbs whose name or doc has the word.
pub fn describe(p: &JsonValue) -> Result<JsonValue, String> {
    let want = p.get("noun").or_else(|| p.get("method")).and_then(JsonValue::as_str);
    let verb = p.get("action").and_then(JsonValue::as_str);
    if let Some(q) = p.get("search").and_then(JsonValue::as_str) {
        let q = q.to_lowercase();
        let mut hits = Vec::new();
        for n in nouns() {
            if n.name.contains(&q) || n.summary.to_lowercase().contains(&q) {
                hits.push(obj([("noun", n.name.into()), ("doc", n.summary.into())]));
            }
            for a in n.actions {
                if a.name.contains(&q) || a.doc.to_lowercase().contains(&q) {
                    hits.push(obj([
                        ("noun", n.name.into()),
                        ("action", a.name.into()),
                        ("doc", a.doc.into()),
                    ]));
                }
            }
        }
        return Ok(obj([("search", q.into()), ("hits", JsonValue::Array(hits))]));
    }
    let Some(want) = want else {
        return Ok(obj([
            ("language", LANGUAGE.into()),
            (
                "nouns",
                JsonValue::Array(
                    nouns()
                        .iter()
                        .map(|n| {
                            obj([
                                ("noun", n.name.into()),
                                ("tool", n.tool.map_or(JsonValue::Null, Into::into)),
                                ("summary", n.summary.into()),
                                ("default_action", n.default_action.into()),
                                ("actions", strs(n.actions.iter().map(|a| a.name))),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]));
    };
    let want = want.split('.').next().unwrap_or(want);
    let n = noun(want).ok_or_else(|| {
        format!("describe: no noun {want:?}; one of {}", nouns().iter().map(|n| n.name).collect::<Vec<_>>().join(", "))
    })?;
    if let Some(v) = verb {
        let a = n.action(v).ok_or_else(|| {
            format!(
                "describe: {} has no action {v:?}; one of {}",
                n.name,
                n.actions.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
            )
        })?;
        let mut o = match action_json(a) {
            JsonValue::Object(o) => o,
            _ => unreachable!(),
        };
        o.set("noun", n.name.into());
        if a.params.is_empty() {
            o.set(
                "note",
                "this verb's params are among the noun's: see describe {noun}".into(),
            );
        }
        return Ok(JsonValue::Object(o));
    }
    Ok(obj([
        ("noun", n.name.into()),
        ("tool", n.tool.map_or(JsonValue::Null, Into::into)),
        ("aliases", strs(n.aliases.iter().copied())),
        ("summary", n.summary.into()),
        ("doc", n.doc.split_whitespace().collect::<Vec<_>>().join(" ").into()),
        ("default_action", n.default_action.into()),
        ("actions", JsonValue::Array(n.actions.iter().map(action_json).collect())),
        (
            "params",
            JsonValue::Array(
                n.param_list()
                    .into_iter()
                    .map(|(k, t, d)| obj([("name", k.into()), ("type", t.into()), ("doc", d.into())]))
                    .collect(),
            ),
        ),
        ("schema", n.input_schema()),
    ]))
}

/// The "Methods at a glance" table of docs/CONTROL.md, from the registry.
pub fn glance_markdown() -> String {
    let mut s = String::from("| method | MCP tool | what | actions |\n|---|---|---|---|\n");
    for n in nouns() {
        let acts = if n.actions.is_empty() {
            "(params only)".to_string()
        } else {
            n.actions
                .iter()
                .map(|a| {
                    if a.name == n.default_action {
                        format!("**{}**", a.name)
                    } else {
                        a.name.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let tool = n.tool.map_or("—".to_string(), |t| format!("`{t}`"));
        s.push_str(&format!("| `{}` | {} | {} | {} |\n", n.name, tool, n.summary, acts));
    }
    s
}

/// The markers around the generated table in docs/CONTROL.md.
pub const GLANCE_BEGIN: &str = "<!-- BEGIN GENERATED: methods at a glance (control_registry) -->";
pub const GLANCE_END: &str = "<!-- END GENERATED -->";

/// `docs` with the glance block regenerated; None when the markers are missing.
pub fn regenerate_glance(docs: &str) -> Option<String> {
    let a = docs.find(GLANCE_BEGIN)? + GLANCE_BEGIN.len();
    let b = a + docs[a..].find(GLANCE_END)?;
    Some(format!("{}\n{}{}", &docs[..a], glance_markdown(), &docs[b..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_every_tool_is_djapp_noun() {
        let mut seen = std::collections::HashSet::new();
        for n in nouns() {
            assert!(seen.insert(n.name), "duplicate noun {}", n.name);
            for al in n.aliases {
                assert!(seen.insert(al), "alias {al} collides");
            }
            if let Some(t) = n.tool {
                assert_eq!(t, format!("djapp_{}", n.name), "{}", n.name);
            }
            let mut acts = std::collections::HashSet::new();
            for a in n.actions {
                assert!(acts.insert(a.name), "{}: duplicate action {}", n.name, a.name);
                if !a.probe.is_empty() && a.probe != "!" {
                    JsonValue::parse(a.probe).unwrap_or_else(|_| panic!("{} {}: probe is JSON", n.name, a.name));
                }
            }
            for a in n.actions.iter().filter(|a| a.effect == Effect::Live) {
                assert!(!a.probe.is_empty(), "{} {}: a live action names a probe the tests may send", n.name, a.name);
            }
            assert!(
                n.default_action.is_empty() || n.action(n.default_action).is_some(),
                "{}: default action {:?} is not one of its actions",
                n.name,
                n.default_action
            );
            assert!(!n.summary.is_empty() && !n.doc.is_empty(), "{}", n.name);
        }
    }

    /// The hand-written schemas can't name verbs the registry lacks, nor leave out ones it has.
    #[test]
    fn a_schema_action_enum_is_the_registry_s_actions() {
        for n in nouns() {
            let schema = n.input_schema();
            assert_eq!(schema.get("type").and_then(JsonValue::as_str), Some("object"), "{}", n.name);
            let Some(en) = schema.get("properties").and_then(|p| p.get("action")).and_then(|a| a.get("enum")) else {
                continue;
            };
            let mut have: Vec<String> = en.strings().unwrap_or_default();
            let mut want: Vec<String> = n.actions.iter().map(|a| a.name.to_string()).collect();
            have.sort();
            want.sort();
            assert_eq!(have, want, "{}: the schema's action enum and the registry's verbs differ", n.name);
        }
    }

    #[test]
    fn describe_lists_every_noun_then_one_noun_then_one_verb() {
        let all = describe(&JsonValue::Object(JsonObject::new())).unwrap();
        let listed = all.get("nouns").and_then(JsonValue::as_array).unwrap();
        assert_eq!(listed.len(), nouns().len());
        let p = JsonValue::parse(r#"{"noun":"loopsets"}"#).unwrap();
        let n = describe(&p).unwrap();
        assert_eq!(n.get("noun").and_then(JsonValue::as_str), Some("loopsets"));
        let p = JsonValue::parse(r#"{"noun":"loopsets","action":"load_pair"}"#).unwrap();
        let a = describe(&p).unwrap();
        let names: Vec<_> = a.get("params").and_then(JsonValue::as_array).unwrap().iter().filter_map(|p| p.get("name")?.as_str()).collect();
        assert!(names.contains(&"group") && names.contains(&"spec"), "{names:?}");
        let e = describe(&JsonValue::parse(r#"{"noun":"loopsets","action":"nope"}"#).unwrap()).unwrap_err();
        assert!(e.contains("one of") && e.contains("load_pair"), "{e}");
        assert!(describe(&JsonValue::parse(r#"{"noun":"zzz"}"#).unwrap()).is_err());
        let hits = describe(&JsonValue::parse(r#"{"search":"pair"}"#).unwrap()).unwrap();
        assert!(hits.get("hits").and_then(JsonValue::as_array).is_some_and(|h| !h.is_empty()));
    }

    #[test]
    fn generated_schemas_list_the_verbs_and_the_params() {
        let n = noun("loopsets").unwrap();
        let s = n.input_schema();
        let en = s.get("properties").and_then(|p| p.get("action")).and_then(|a| a.get("enum")).unwrap();
        assert!(en.strings().unwrap().contains(&"load_pair".to_string()));
        assert!(s.get("properties").and_then(|p| p.get("group")).is_some());
    }

    /// docs/CONTROL.md carries the glance table; this fails when it is stale.
    #[test]
    fn control_md_has_the_current_glance_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONTROL.md");
        let docs = std::fs::read_to_string(path).expect("docs/CONTROL.md");
        let fresh = regenerate_glance(&docs).expect("CONTROL.md has the glance markers");
        if std::env::var_os("DJ_REGEN_DOCS").is_some() {
            std::fs::write(path, &fresh).unwrap();
            return;
        }
        assert!(
            docs == fresh,
            "docs/CONTROL.md's \"Methods at a glance\" is stale: DJ_REGEN_DOCS=1 cargo test -p djhost control_registry"
        );
    }

    /// Every noun has a row in CONTROL.md's prose table too (the long notes stay by hand).
    #[test]
    fn control_md_has_prose_for_every_noun() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONTROL.md");
        let docs = std::fs::read_to_string(path).expect("docs/CONTROL.md");
        let docs = docs.split(GLANCE_END).nth(1).expect("the glance block ends");
        for n in nouns() {
            assert!(
                docs.contains(&format!("\n| `{}` | ", n.name)),
                "docs/CONTROL.md has no \"| `{}` |\" row",
                n.name
            );
        }
    }
}
