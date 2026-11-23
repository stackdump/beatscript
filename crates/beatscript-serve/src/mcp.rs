//! P7: `/mcp` and `/mcp/` (Go `internal/btwsite/mcpsrv` on mark3labs/mcp-go v0.45.0): the MCP server, stateless
//! streamable HTTP. Every request stands alone (no session id), answers are one JSON body (never SSE: a stateless
//! server has nothing to send unprompted), and the framing, status codes and JSON-RPC errors are mcp-go's:
//!
//! - GET: the landing page, or 405 for a GET that asks for an event stream; DELETE: 200 empty; anything else but
//!   POST: `404 page not found`.
//! - POST: `Content-Type` must be `application/json` (400 text otherwise); a body that is not a JSON object is
//!   `-32700 request body is not valid json` with HTTP 400; a notification or a response is 202 with no body; a
//!   request is 200 with `{"jsonrpc","id","result"|"error"}` and a newline.
//! - Methods: `initialize`, `ping`, `tools/list` (cursor pagination as mcp-go's), `tools/call`; the capabilities the
//!   server does not have answer `-32601 <x> not supported`, anything else `-32601 Method <m> not found`.
//!
//! The tools are in `mcp/tools.rs`; the hub-backed ones (eval, load, get, stop, devices, queue) go through the
//! `hubapi::HubApi` trait, which P6's hub implements.

pub mod hubapi;
pub mod limiter;
pub mod text;
pub mod tools;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use axum::routing::any;
use axum::Router;
use serde_json::{json, Map, Value};

use crate::compose::gojson;
use crate::AppState;

pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const VALID_PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// `http.Error`: text/plain, nosniff, the message and a newline.
fn http_error(status: u16, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(format!("{msg}\n")))
        .unwrap()
}

/// A JSON-RPC id as mcp-go echoes it: `json.Marshal` of what `any` decoded (numbers are float64, maps sorted).
fn go_id(v: &Value) -> Value {
    match v {
        Value::Number(n) => gojson::num(n.as_f64().unwrap_or(0.0)),
        Value::Array(a) => Value::Array(a.iter().map(go_id).collect()),
        Value::Object(m) => {
            let mut e: Vec<(String, Value)> = m.iter().map(|(k, x)| (k.clone(), go_id(x))).collect();
            e.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(e.into_iter().collect())
        }
        v => v.clone(),
    }
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    gojson::obj([("jsonrpc", json!("2.0")), ("id", go_id(id)), ("error", gojson::obj([("code", json!(code)), ("message", json!(message))]))])
}

fn result_response(id: &Value, result: Value) -> Value {
    gojson::obj([("jsonrpc", json!("2.0")), ("id", go_id(id)), ("result", result)])
}

fn json_body(status: u16, v: &Value) -> Response<Body> {
    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json").body(crate::go_body(gojson::line(v))).unwrap()
}

/// The JSON type word encoding/json's errors use.
fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A field of a JSON object looked up as encoding/json does (exact name first, then case-insensitively).
fn field<'a>(m: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    m.get(name).or_else(|| m.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
}

/// The unmarshal checks of mcp-go's typed requests, as Go's error text: `json: cannot unmarshal <kind> into Go struct
/// field <path> of type <type>`. Covers params and the fields the server reads; deeper ones are taken as they come.
fn unparsable(method: &str, msg: &Map<String, Value>) -> Option<String> {
    let te = |kind: &str, path: &str, ty: &str| Some(format!("unparsable {method} request: json: cannot unmarshal {kind} into Go struct field {path} of type {ty}"));
    // (the struct that holds params, as the error names it; params' own type)
    let (req, params_ty) = match method {
        "initialize" => ("InitializeRequest", "mcp.InitializeParams"),
        "ping" => ("PingRequest.Request", "mcp.RequestParams"),
        "tools/list" => ("ListToolsRequest.PaginatedRequest", "mcp.PaginatedParams"),
        "tools/call" => ("CallToolRequest", "mcp.CallToolParams"),
        _ => return None,
    };
    if let Some(m) = field(msg, "method") {
        if !m.is_string() && !m.is_null() {
            return te(json_kind(m), &format!("{req}.method"), "mcp.MCPMethod");
        }
    }
    let p = match field(msg, "params") {
        None | Some(Value::Null) => return None,
        Some(Value::Object(p)) => p,
        Some(v) => return te(json_kind(v), &format!("{req}.params"), params_ty),
    };
    let s = |k: &str, owner: &str, ty: &str| -> Option<String> {
        match field(p, k) {
            Some(v) if !v.is_string() && !v.is_null() => te(json_kind(v), &format!("{owner}.params.{k}"), ty),
            _ => None,
        }
    };
    match method {
        "initialize" => {
            if let Some(e) = s("protocolVersion", "InitializeParams", "string") {
                return Some(e);
            }
            for (k, ty) in [("capabilities", "mcp.ClientCapabilities"), ("clientInfo", "mcp.Implementation")] {
                if let Some(v) = field(p, k).filter(|v| !v.is_object() && !v.is_null()) {
                    return te(json_kind(v), &format!("InitializeParams.params.{k}"), ty);
                }
            }
            if let Some(Value::Object(ci)) = field(p, "clientInfo") {
                for k in ["name", "version", "title", "description", "websiteUrl"] {
                    if let Some(v) = field(ci, k).filter(|v| !v.is_string() && !v.is_null()) {
                        return te(json_kind(v), &format!("Implementation.params.clientInfo.{k}"), "string");
                    }
                }
            }
            None
        }
        "tools/list" => s("cursor", "PaginatedParams.PaginatedRequest", "mcp.Cursor"),
        "tools/call" => {
            if let Some(e) = s("name", "CallToolParams", "string") {
                return Some(e);
            }
            match field(p, "_meta") {
                Some(v) if !v.is_object() && !v.is_null() => te(json_kind(v), "CallToolParams.params._meta", "map[string]interface {}"),
                _ => None,
            }
        }
        "ping" => match field(p, "_meta") {
            Some(v) if !v.is_object() && !v.is_null() => te(json_kind(v), "RequestParams.Request.params._meta", "map[string]interface {}"),
            _ => None,
        },
        _ => None,
    }
}

pub struct Mcp {
    pub app: AppState,
    pub tools: Arc<tools::Tools>,
}

impl Mcp {
    /// `HandleMessage`: one JSON-RPC message, the answer (None: a notification or a response, 202).
    pub async fn handle(&self, raw: &[u8], ctx: &tools::CallInfo) -> Option<Value> {
        let msg = match serde_json::from_slice::<Value>(raw) {
            Ok(Value::Object(m)) => m,
            Ok(Value::Null) => Map::new(),
            _ => return Some(error_response(&Value::Null, PARSE_ERROR, "Failed to parse message")),
        };
        let jsonrpc = field(&msg, "jsonrpc").cloned().unwrap_or(Value::Null);
        let method = field(&msg, "method").cloned().unwrap_or(Value::Null);
        let id = field(&msg, "id").cloned().unwrap_or(Value::Null);
        if !matches!(jsonrpc, Value::String(_) | Value::Null) || !matches!(method, Value::String(_) | Value::Null) {
            return Some(error_response(&Value::Null, PARSE_ERROR, "Failed to parse message"));
        }
        if jsonrpc.as_str() != Some("2.0") {
            return Some(error_response(&id, INVALID_REQUEST, "Invalid JSON-RPC version"));
        }
        if id.is_null() {
            // a notification: params must be an object (or null); no handlers are registered
            return match field(&msg, "params") {
                None | Some(Value::Null) | Some(Value::Object(_)) => None,
                Some(_) => Some(error_response(&Value::Null, PARSE_ERROR, "Failed to parse notification")),
            };
        }
        if field(&msg, "result").is_some_and(|r| !r.is_null()) {
            return None;
        }
        let method = method.as_str().unwrap_or("");
        let unsupported = |what: &str| Some(error_response(&id, METHOD_NOT_FOUND, &format!("{what} not supported")));
        match method {
            "initialize" | "ping" | "tools/list" | "tools/call" => {
                if let Some(e) = unparsable(method, &msg) {
                    return Some(error_response(&id, INVALID_REQUEST, &e));
                }
            }
            _ => {}
        }
        let params = field(&msg, "params").and_then(|p| p.as_object()).cloned().unwrap_or_default();
        match method {
            "initialize" => {
                let asked = field(&params, "protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
                let asked = if asked.is_empty() { "2025-03-26" } else { asked };
                let pv = if VALID_PROTOCOL_VERSIONS.contains(&asked) { asked } else { LATEST_PROTOCOL_VERSION };
                let r = gojson::obj([
                    ("protocolVersion", json!(pv)),
                    ("capabilities", json!({"tools": {}})),
                    ("serverInfo", json!({"name": "beatscript", "version": "0.1.0"})),
                    ("instructions", json!(self.tools.instructions())),
                ]);
                Some(result_response(&id, r))
            }
            "ping" => Some(result_response(&id, json!({}))),
            "logging/setLevel" => unsupported("logging"),
            "resources/list" | "resources/templates/list" | "resources/read" => unsupported("resources"),
            "prompts/list" | "prompts/get" => unsupported("prompts"),
            "tasks/get" | "tasks/list" | "tasks/result" | "tasks/cancel" => unsupported("tasks"),
            "completion/complete" => unsupported("completions"),
            "tools/list" => {
                let cursor = field(&params, "cursor").and_then(|v| v.as_str()).unwrap_or("");
                match self.tools.list(cursor) {
                    Ok(t) => Some(result_response(&id, json!({"tools": t}))),
                    Err(e) => Some(error_response(&id, INVALID_PARAMS, &e)),
                }
            }
            "tools/call" => {
                let name = field(&params, "name").and_then(|v| v.as_str()).unwrap_or("");
                if !self.tools.has(name) {
                    return Some(error_response(&id, INVALID_PARAMS, &format!("tool '{name}' not found: tool not found")));
                }
                let args = field(&params, "arguments").cloned().unwrap_or(Value::Null);
                let r = self.tools.call(name, args, ctx).await;
                Some(result_response(&id, r))
            }
            m => Some(error_response(&id, METHOD_NOT_FOUND, &format!("Method {m} not found"))),
        }
    }
}

/// `isJSONEmpty`.
fn json_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Object(m)) => m.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        _ => false,
    }
}

fn bearer(h: &HeaderMap) -> String {
    let a = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("").trim();
    if a.len() > 7 && a.is_char_boundary(7) && a[..7].eq_ignore_ascii_case("bearer ") {
        let t = a[7..].trim();
        if t.len() <= 256 {
            return t.to_string();
        }
    }
    String::new()
}

async fn handle(State(m): State<Arc<Mcp>>, req: Request) -> Response<Body> {
    let method = req.method().clone();
    if method == Method::GET {
        if req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("").contains("text/event-stream") {
            let mut r = http_error(405, "this server sends no unprompted events: POST JSON-RPC to /mcp");
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
            return r;
        }
        return m.tools.landing();
    }
    if method == Method::DELETE {
        return Response::builder().status(200).body(Body::empty()).unwrap();
    }
    if method != Method::POST {
        return http_error(404, "404 page not found");
    }
    if !crate::compose::is_json(req.headers()) {
        return http_error(400, "Invalid content type: must be 'application/json'");
    }
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    let ctx = tools::CallInfo { token: bearer(req.headers()), ip: crate::client_ip(peer, req.headers()) };
    let has_session = req.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).filter(|s| !s.is_empty()).is_some();
    let limit = 8 * m.app.cfg.max_program_bytes + (64 << 10);
    let raw = read_all(req.into_body(), limit).await;
    let Some(raw) = raw else {
        return json_body(400, &error_response(&Value::Null, PARSE_ERROR, "read request body error: http: request body too large"));
    };
    // the transport's first look: an object with id, result, error, method
    let probe: Option<Map<String, Value>> = match serde_json::from_slice::<Value>(&raw) {
        Ok(Value::Object(o)) => Some(o),
        Ok(Value::Null) => Some(Map::new()),
        _ => None,
    };
    let Some(probe) = probe.filter(|o| field(o, "method").is_none_or(|v| v.is_string() || v.is_null())) else {
        return json_body(400, &error_response(&Value::Null, PARSE_ERROR, "request body is not valid json"));
    };
    let method_s = field(&probe, "method").and_then(|v| v.as_str()).unwrap_or("");
    // authorization (oauth.rs): a token that is not good is a 401 that points at the resource metadata, so a client
    // can sign in by OAuth; so is an anonymous call of a tool that needs an account. Anonymous reads stay open.
    if !ctx.token.is_empty() {
        if tools::token_user(&m.app, &ctx.token).is_none() {
            return unauthorized(Some("invalid_token"), "the token is unknown, expired or revoked: connect again (OAuth) or make a new one in the MCP panel");
        }
    } else if method_s == "tools/call" && field(&probe, "params").and_then(|p| p.get("name")).and_then(|n| n.as_str()).is_some_and(tools::auth_tool) {
        return unauthorized(None, "this tool needs a signed-in account: connect with OAuth, or send Authorization: Bearer btw_… from the MCP panel");
    }
    let has_id = field(&probe, "id").is_some();
    if method_s.is_empty() && has_id {
        let (res, err) = (field(&probe, "result"), field(&probe, "error"));
        if json_empty(res) && json_empty(err) {
            // a ping response
            return Response::builder().status(200).body(Body::empty()).unwrap();
        }
        // a sampling response: there are no sessions to deliver it to
        let body = if !has_session {
            "Missing session ID for sampling response\nFailed to handle sampling response\n"
        } else if field(&probe, "id").and_then(|v| v.as_f64()).is_none_or(|f| f != f.trunc()) {
            "Invalid request ID in sampling response\nFailed to handle sampling response\n"
        } else {
            "Failed to deliver response\nFailed to handle sampling response\n"
        };
        let status = if !has_session || body.starts_with("Invalid") { 400 } else { 500 };
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header("x-content-type-options", "nosniff")
            .body(Body::from(body))
            .unwrap();
    }
    match m.handle(&raw, &ctx).await {
        None => Response::builder().status(StatusCode::ACCEPTED).body(Body::empty()).unwrap(),
        Some(v) => json_body(200, &v),
    }
}

/// A 401 from `/mcp`, with `WWW-Authenticate: Bearer resource_metadata=…` (RFC 9728 §5.1, the MCP spec).
fn unauthorized(error: Option<&str>, desc: &str) -> Response<Body> {
    let v = json!({"error": error.unwrap_or("unauthorized"), "error_description": desc});
    Response::builder()
        .status(401)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::WWW_AUTHENTICATE, HeaderValue::from_str(&crate::oauth::www_authenticate(error)).unwrap())
        .body(Body::from(v.to_string() + "\n"))
        .unwrap()
}

/// The whole body, or None past `limit` (a `MaxBytesReader`).
async fn read_all(body: Body, limit: usize) -> Option<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    let mut s = body.into_data_stream();
    while let Some(c) = s.next().await {
        let c = c.ok()?;
        out.extend_from_slice(&c);
        if out.len() > limit {
            return None;
        }
    }
    Some(out)
}

pub fn routes(app: &AppState) -> Router {
    let m = Arc::new(Mcp { app: app.clone(), tools: Arc::new(tools::Tools::new(app)) });
    Router::new().route("/mcp", any(handle)).route("/mcp/", any(handle)).route("/mcp/{*rest}", any(handle)).with_state(m)
}
