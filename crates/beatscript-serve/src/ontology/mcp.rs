//! `/ontology/mcp` (ontology-bitwrap-io `mcp.go`): the ontology's read-only MCP server over Streamable HTTP,
//! stateless (no session, no SSE). Its seven tools, their descriptions and schemas, the caps and every answer's text
//! are the Go server's, so a client sees the same server at either address. This is not beatscript-serve's own `/mcp`
//! (`crate::mcp`, the BeatScript tools): that one is mcp-go's framing, this one is the ontology server's.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::graph::{is_slug, NS, SITE_BASE};
use super::query::{triples_to_nt, Answer, QueryError};
use super::{parse_value, Ontology};
use crate::compose::gojson;

const MCP_MAX_ROWS: usize = 200;

/// The glossary-term namespaces with a page of their own, by URL path (`termBases`).
pub const TERM_BASES: [&str; 11] = ["genre", "family", "scale", "groove", "section", "structure", "instrument", "role", "rhythm", "concept", "program"];

pub const INSTRUCTIONS: &str = "Read-only access to the BeatScript ontology (https://beatscript.app/ontology/ns#), derived from the bitwrap ontology 0.2.0: genres, scales, grooves, song sections, instruments, Euclidean rhythms, Markov melody, the Petri-net engine concepts, and the BeatScript language's own terms (grammar productions, keywords, voices, kits, control nouns and verbs, MCP tools). Queries run over the materialized graph and are capped at 1000 rows and 5 seconds.";

fn str_prop(desc: &str) -> Value {
    json!({"description": desc, "type": "string"})
}

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    let mut schema = Map::new();
    schema.insert("properties".into(), gojson::sorted(props));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("type".into(), json!("object"));
    gojson::obj([("description", json!(desc)), ("inputSchema", Value::Object(schema)), ("name", json!(name))])
}

/// `toolDefs`, in the Go server's order, each a Go map (sorted keys).
pub fn tool_defs() -> Vec<Value> {
    vec![
        tool("sparql_query", "Run a read-only SPARQL query (SELECT, ASK, CONSTRUCT or DESCRIBE) over the BeatScript ontology. Rows are capped and queries time out. Results can contain blank nodes (OWL class expressions); add FILTER(isIRI(?x)) to skip them.", json!({"query": str_prop("SPARQL query text")}), &["query"]),
        tool("describe_term", "Everything the ontology states about one class, property, genre, scale, groove, section, instrument or concept, as N-Triples (the materialized graph). Give a short name such as Genre, bpm, genre:techno or section:drop, or a full IRI in the ontology's namespaces. format \"json\" returns a compact view instead: label, types, definition (its own, else the glossary concept it closeMatches, else its rdfs:comment), direct superclasses and subclasses as asserted, close matches and sources.",
            json!({"term": str_prop("name or IRI"), "format": {"description": "triples (default, N-Triples) or json (compact term view)", "enum": ["triples", "json"], "type": "string"}}), &["term"]),
        tool("search_terms", "Find terms whose label, other label (an alternative or a former name, such as btw for BeatScript) or definition contains the text (case-insensitive). A class with no definition of its own gets the one of the glossary concept it closeMatches, or its rdfs:comment.",
            json!({"text": str_prop("text to look for"), "limit": {"description": "max results, default 25, at most 200", "type": "integer"}}), &["text"]),
        tool("list_classes", "All classes in the ontology namespace with their direct superclasses, as asserted: a superclass another listed one already implies is left out, so LiveNet lists DeadlockFreeNet, not PetriNet too. For the inferred closure, query rdfs:subClassOf with sparql_query.", json!({}), &[]),
        tool("list_genres", "Every genre the engine knows, with its tempo, scale, family and groove. Filter by family (edm, song, jazz, chill) or by a BPM range.",
            json!({"family": str_prop("edm, song, jazz or chill; omit for all"), "min_bpm": {"type": "integer"}, "max_bpm": {"type": "integer"}}), &[]),
        tool("get_genre", "One genre in full: tempo, scale, kick/snare/hihat Euclidean rhythms (pulses, steps, rotation, MIDI note), swing, humanize, densities, instrument per channel, chord progressions, song-structure sections, and links to Wikidata and to pflow ontology terms. Give a slug such as techno or jazz.",
            json!({"genre": str_prop("genre slug, e.g. techno")}), &["genre"]),
        tool("get_release", "Version, pack digest and file hashes of the release being served, and what its certificate covers.", json!({}), &[]),
    ]
}

/// One JSON-RPC answer (`reply`): `json.NewEncoder(w).Encode` of a Go map.
fn reply(id: Option<&Value>, result: Option<Value>, err: Option<(i64, String)>) -> String {
    let mut out = Map::new();
    if let Some((code, msg)) = err {
        out.insert("error".into(), gojson::obj([("code", json!(code)), ("message", json!(msg))]));
    }
    out.insert("id".into(), id.cloned().unwrap_or(Value::Null));
    out.insert("jsonrpc".into(), json!("2.0"));
    if let Some(r) = result {
        out.insert("result".into(), r);
    }
    gojson::line(&Value::Object(out))
}

/// What a POST body answers: None for a notification (202, no body), else the JSON line.
pub async fn handle(o: &Arc<Ontology>, body: &[u8], ip: &str) -> Option<String> {
    let req: Value = match serde_json::from_slice(body) {
        Ok(v @ Value::Object(_)) => v,
        _ => return Some(reply(None, None, Some((-32700, "parse error".into())))),
    };
    if req.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(reply(None, None, Some((-32700, "parse error".into()))));
    }
    let id = req.get("id")?; // a notification: nothing to answer
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    if method == "tools/call" && o.sp.gate(ip).is_err() {
        return Some(reply(Some(id), None, Some((-32000, "rate limit exceeded or client paused after repeated timeouts".into()))));
    }
    Some(match method {
        "initialize" => {
            let pv = params.get("protocolVersion").or_else(|| ci_get(&params, "protocolversion")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("2025-03-26");
            let r = gojson::obj([
                ("capabilities", json!({"tools": {}})),
                ("instructions", json!(INSTRUCTIONS)),
                ("protocolVersion", json!(pv)),
                ("serverInfo", gojson::obj([("name", json!("beatscript-ontology")), ("version", json!(o.version))])),
            ]);
            reply(Some(id), Some(r), None)
        }
        "ping" => reply(Some(id), Some(json!({})), None),
        "tools/list" => reply(Some(id), Some(json!({"tools": tool_defs()})), None),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !params.is_object() || name.is_empty() {
                return Some(reply(Some(id), None, Some((-32602, "invalid params".into()))));
            }
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match call(o, name, &args, ip).await {
                Err(Call::Unknown) => reply(Some(id), None, Some((-32602, format!("unknown tool {name}")))),
                Err(Call::Failed(m)) => reply(Some(id), Some(gojson::obj([("content", json!([{"text": m, "type": "text"}])), ("isError", json!(true))])), None),
                Ok(text) => reply(Some(id), Some(json!({"content": [{"text": text, "type": "text"}]})), None),
            }
        }
        _ => reply(Some(id), None, Some((-32601, "method not found".into()))),
    })
}

pub enum Call {
    Unknown,
    Failed(String),
}

impl From<QueryError> for Call {
    fn from(e: QueryError) -> Call {
        Call::Failed(e.to_string())
    }
}

/// encoding/json's field matching: the exact key, else a case-insensitive one.
fn ci_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let m = v.as_object()?;
    m.get(key).or_else(|| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v))
}

/// A string argument as `json.Unmarshal` into a struct field reads it: Err when the arguments are not an object (or
/// null) or the field holds another type; "" when absent.
fn arg_str(args: &Value, key: &str) -> Result<String, ()> {
    match args {
        Value::Null => Ok(String::new()),
        Value::Object(_) => match ci_get(args, key) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

fn arg_int(args: &Value, key: &str) -> Result<i64, ()> {
    match args {
        Value::Null => Ok(0),
        Value::Object(_) => match ci_get(args, key) {
            None | Some(Value::Null) => Ok(0),
            Some(Value::Number(n)) => n.as_i64().ok_or(()),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

async fn call(o: &Arc<Ontology>, name: &str, args: &Value, ip: &str) -> Result<String, Call> {
    // json.Unmarshal of absent arguments (nil RawMessage) is an error for the tools that take one
    let missing = args.is_null();
    match name {
        "sparql_query" => {
            let q = arg_str(args, "query").ok().filter(|q| !q.is_empty() && !missing).ok_or_else(|| Call::Failed(r#"argument "query" is required, e.g. {"query": "SELECT * WHERE { ?s ?p ?o } LIMIT 5"}"#.into()))?;
            query(o, &q, ip).await
        }
        "describe_term" => {
            let term = arg_str(args, "term").ok().filter(|t| !t.is_empty() && !missing).ok_or_else(|| Call::Failed(r#"argument "term" is required, e.g. {"term": "Place"}"#.into()))?;
            match arg_str(args, "format").unwrap_or_default().as_str() {
                "" | "triples" => describe(o, &term, ip).await,
                "json" => describe_json(o, &term, ip).await,
                _ => Err(Call::Failed(r#"format: "triples" (the default) or "json""#.into())),
            }
        }
        "search_terms" => {
            let text = arg_str(args, "text").ok().filter(|t| !t.trim().is_empty() && !missing);
            let limit = arg_int(args, "limit");
            match (text, limit) {
                (Some(t), Ok(l)) => search(o, &t, l, ip).await,
                _ => Err(Call::Failed(r#"argument "text" is required, e.g. {"text": "inhibitor", "limit": 10}"#.into())),
            }
        }
        "list_classes" => Ok(o.graph.classes_json()),
        "list_genres" => {
            let bad = || Call::Failed("arguments: family (string), min_bpm, max_bpm (integers)".into());
            let fam = arg_str(args, "family").map_err(|_| bad())?;
            let min = arg_int(args, "min_bpm").map_err(|_| bad())?;
            let max = arg_int(args, "max_bpm").map_err(|_| bad())?;
            Ok(o.graph.list_genres(&fam, min, max))
        }
        "get_genre" => {
            let g = arg_str(args, "genre").ok().filter(|g| !g.is_empty() && !missing).ok_or_else(|| Call::Failed(r#"argument "genre" is required, e.g. {"genre": "techno"}"#.into()))?;
            o.graph.get_genre(&g).map_err(Call::Failed)
        }
        "get_release" => Ok(o.release_json()),
        _ => Err(Call::Unknown),
    }
}

/// `query`: a result rendered compactly for a model, capped at 200 rows.
async fn query(o: &Arc<Ontology>, q: &str, ip: &str) -> Result<String, Call> {
    let r = o.sp.run(q, ip).await?;
    let mut truncated = r.truncated;
    let mut out = Map::new();
    match r.answer {
        Answer::Ask(b) => {
            out.insert("boolean".into(), json!(b));
        }
        Answer::Graph(mut ts) => {
            if ts.len() > MCP_MAX_ROWS {
                ts.truncate(MCP_MAX_ROWS);
                truncated = true;
            }
            let nt = triples_to_nt(&ts);
            out.insert("triples".into(), json!(nt.trim().split('\n').collect::<Vec<_>>()));
        }
        Answer::Select { vars, mut rows } => {
            if rows.len() > MCP_MAX_ROWS {
                rows.truncate(MCP_MAX_ROWS);
                truncated = true;
            }
            out.insert("rows".into(), Value::Array(rows.iter().map(|r| plain(r)).collect()));
            out.insert("vars".into(), json!(vars));
        }
    }
    if truncated {
        out.insert("truncated".into(), json!(true));
    }
    Ok(gojson::indent(&gojson::sorted(Value::Object(out)), " "))
}

/// A row as `map[string]string` of plain values (sorted keys).
fn plain(row: &[(String, String)]) -> Value {
    gojson::sorted(Value::Object(row.iter().filter_map(|(k, raw)| parse_value(raw).map(|v| (k.clone(), json!(v)))).collect()))
}

fn sparql_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

pub fn search_query(text: &str, limit: i64) -> String {
    let limit = if limit <= 0 { 25 } else { limit.min(MCP_MAX_ROWS as i64) };
    let t = sparql_string(&text.trim().to_lowercase());
    format!(
        "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\nPREFIX skos: <http://www.w3.org/2004/02/skos/core#>\nPREFIX owl: <http://www.w3.org/2002/07/owl#>\nSELECT DISTINCT ?term ?label ?definition WHERE {{\n  {{ ?term rdfs:label ?label }} UNION {{ ?term skos:prefLabel ?label }}\n  OPTIONAL {{ ?term skos:definition ?definition }}\n  OPTIONAL {{ {{ ?term skos:altLabel ?other }} UNION {{ ?term skos:hiddenLabel ?other }} FILTER(CONTAINS(LCASE(STR(?other)), \"{t}\")) }}\n  FILTER(CONTAINS(LCASE(STR(?label)), \"{t}\") || (BOUND(?definition) && CONTAINS(LCASE(STR(?definition)), \"{t}\")) || BOUND(?other))\n}} ORDER BY ?label LIMIT {limit}"
    )
}

async fn search(o: &Arc<Ontology>, text: &str, limit: i64, ip: &str) -> Result<String, Call> {
    let r = o.sp.run(&search_query(text, limit), ip).await?;
    let Answer::Select { vars, rows } = r.answer else { return Err(Call::Failed("query service unavailable".into())) };
    let mut out_rows = Vec::new();
    for row in &rows {
        let mut p: Map<String, Value> = row.iter().filter_map(|(k, raw)| parse_value(raw).map(|v| (k.clone(), json!(v)))).collect();
        let term = p.get("term").and_then(Value::as_str).unwrap_or("").to_string();
        if p.get("definition").and_then(Value::as_str).unwrap_or("").is_empty() && !term.is_empty() {
            let (d, via) = o.graph.definition(&term);
            if !d.is_empty() {
                p.insert("definition".into(), json!(d));
                if !via.is_empty() {
                    p.insert("definition_from".into(), json!(via));
                }
            }
        }
        out_rows.push(gojson::sorted(Value::Object(p)));
    }
    let mut out = Map::new();
    out.insert("rows".into(), Value::Array(out_rows));
    if r.truncated {
        out.insert("truncated".into(), json!(true));
    }
    out.insert("vars".into(), json!(vars));
    Ok(gojson::indent(&Value::Object(out), " "))
}

fn is_iri_chars(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_./#:~-".contains(&c))
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_ -".contains(&c))
}

/// strings.Title over ASCII: a letter after a separator (anything but a letter, digit or `_`) is upper-cased.
fn go_title(s: &str) -> String {
    let mut prev_sep = true;
    s.chars()
        .map(|c| {
            let out = if prev_sep { c.to_ascii_uppercase() } else { c };
            prev_sep = !(c.is_ascii_alphanumeric() || c == '_');
            out
        })
        .collect()
}

/// `describeIRIs`: a name or IRI as the IRIs worth describing, only ever in the ontology's own namespaces.
pub fn describe_iris(term: &str) -> Result<Vec<String>, String> {
    let term = term.trim();
    if term.contains("://") {
        let ok = term.starts_with(&format!("{SITE_BASE}/")) || term.starts_with(NS);
        if !ok || !is_iri_chars(term) {
            return Err(format!("only IRIs under {SITE_BASE}/ are described"));
        }
        return Ok(vec![term.to_string()]);
    }
    for (prefix, base) in [("genre:", "/genre/"), ("section:", "/section/"), ("instrument:", "/instrument/"), ("scale:", "/scale/"), ("groove:", "/groove/"), ("role:", "/role/"), ("structure:", "/structure/"), ("family:", "/family/"), ("rhythm:", "/rhythm/"), ("concept:", "/concept/"), ("program:", "/program/")] {
        if let Some(slug) = term.strip_prefix(prefix) {
            if !is_slug(slug) {
                return Err("term must be a name such as Genre or genre:techno, or an IRI".into());
            }
            return Ok(vec![format!("{SITE_BASE}{base}{slug}")]);
        }
    }
    let term = term.strip_prefix("ns:").unwrap_or(term);
    if !is_name(term) {
        return Err("term must be a name such as Genre or \"song section\", or an IRI".into());
    }
    let joined = term.replace(' ', "");
    let camel = go_title(&term.to_lowercase()).replace(' ', "");
    let slug = term.to_lowercase().replace([' ', '_'], "-");
    let mut out = vec![format!("{NS}{joined}")];
    for b in TERM_BASES {
        out.push(format!("{SITE_BASE}/{b}/{slug}"));
    }
    if camel != joined {
        out.push(format!("{NS}{camel}"));
    }
    Ok(out)
}

async fn describe(o: &Arc<Ontology>, term: &str, ip: &str) -> Result<String, Call> {
    let iris = describe_iris(term).map_err(Call::Failed)?;
    let mut all = Vec::new();
    for iri in iris {
        if let Answer::Graph(ts) = o.sp.run(&format!("DESCRIBE <{iri}>"), ip).await?.answer {
            all.extend(ts);
        }
    }
    if all.is_empty() {
        return Err(Call::Failed(format!("nothing is stated about {}; try search_terms", go_quote(term))));
    }
    Ok(triples_to_nt(&all))
}

async fn describe_json(o: &Arc<Ontology>, term: &str, ip: &str) -> Result<String, Call> {
    let iris = describe_iris(term).map_err(Call::Failed)?;
    let views: Vec<Value> = iris.iter().filter_map(|i| o.graph.view(i)).collect();
    if views.is_empty() {
        return describe(o, term, ip).await;
    }
    let out = if views.len() == 1 { views.into_iter().next().unwrap() } else { json!({"terms": views}) };
    Ok(gojson::indent(&out, " "))
}

/// `%q` for the plain names describe_term accepts.
fn go_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\x{:02x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iris() {
        assert_eq!(describe_iris("genre:techno").unwrap(), vec!["https://beatscript.app/ontology/genre/techno"]);
        let v = describe_iris("song section").unwrap();
        assert_eq!(v[0], "https://beatscript.app/ontology/ns#songsection");
        assert_eq!(v[5], "https://beatscript.app/ontology/section/song-section");
        assert_eq!(v.last().unwrap(), "https://beatscript.app/ontology/ns#SongSection");
        assert_eq!(describe_iris("Genre").unwrap().len(), 12);
        assert!(describe_iris("https://example.com/x").is_err());
        // bitwrap's own IRIs are upstream: the fork describes its own
        assert!(describe_iris("https://ontology.bitwrap.io/genre/techno").is_err());
        assert!(describe_iris("bad;name").is_err());
    }
}
