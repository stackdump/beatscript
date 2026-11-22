//! The Claude agent the app spawns: how to start `claude`, how to read its stream-json output,
//! how to send it a message. Pure, so the whole protocol is tested against recorded lines and a
//! fake `claude` without spending a cent. Port of `Control/AgentStream.swift`.
//!
//! ```text
//! claude -p --input-format stream-json --output-format stream-json --verbose --agent dj
//!        --permission-mode dontAsk --strict-mcp-config --mcp-config <only the app's server>
//!        --allowedTools mcp__dj-vevr-se,Skill,Read,Glob,Grep --max-budget-usd N [--model M]
//!        [--resume <session>]
//! ```
//!
//! One long-lived process: each user line is a turn; the agent drives the app through the same
//! MCP tools anyone else does, so everything it does is recorded.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};
use crate::mcp::SERVER_NAME;

/// `AgentEvent`.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    Ready {
        session: String,
        model: Option<String>,
    },
    Text(String),
    ToolUse {
        name: String,
        summary: String,
    },
    ToolResult {
        text: String,
        is_error: bool,
    },
    /// A turn ended: `ok` false for errors (budget, max turns); cost is the session's so far.
    TurnDone {
        ok: bool,
        cost: Option<f64>,
        turns: Option<i64>,
        message: Option<String>,
    },
}

/// `AgentStream.mcpServerName`.
pub const MCP_SERVER_NAME: &str = SERVER_NAME;
const TOOL_PREFIX: &str = "mcp__dj-vevr-se__";

fn prefix(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn count(s: &str) -> usize {
    s.chars().count()
}

/// The events one output line carries (none for lines the console doesn't show).
pub fn parse(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = JsonValue::parse(line) else {
        return Vec::new();
    };
    let Some(ty) = v.get("type").and_then(JsonValue::as_str) else {
        return Vec::new();
    };
    let str_of = |x: Option<&JsonValue>, k: &str| -> Option<String> {
        x.and_then(|x| x.get(k))
            .and_then(JsonValue::as_str)
            .map(ToString::to_string)
    };
    match ty {
        "system" => {
            if v.get("subtype").and_then(JsonValue::as_str) != Some("init") {
                return Vec::new();
            }
            let Some(sid) = str_of(Some(&v), "session_id") else {
                return Vec::new();
            };
            alloc::vec![AgentEvent::Ready {
                session: sid,
                model: str_of(Some(&v), "model"),
            }]
        }
        "assistant" => {
            let mut out = Vec::new();
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(JsonValue::as_array);
            for b in blocks.into_iter().flatten() {
                match b.get("type").and_then(JsonValue::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(JsonValue::as_str) {
                            if !t.trim().is_empty() {
                                out.push(AgentEvent::Text(t.to_string()));
                            }
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(JsonValue::as_str).unwrap_or("?");
                        out.push(AgentEvent::ToolUse {
                            name: short_tool(name).to_string(),
                            summary: summarize(b.get("input")),
                        });
                    }
                    _ => {}
                }
            }
            out
        }
        "user" => {
            let Some(content) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(JsonValue::as_array)
            else {
                return Vec::new();
            };
            content
                .iter()
                .filter(|b| b.get("type").and_then(JsonValue::as_str) == Some("tool_result"))
                .map(|b| AgentEvent::ToolResult {
                    text: one_line(&result_text(b.get("content"))),
                    is_error: b
                        .get("is_error")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        }
        "result" => {
            let ok = v.get("subtype").and_then(JsonValue::as_str) == Some("success")
                && v.get("is_error").and_then(JsonValue::as_bool) != Some(true);
            alloc::vec![AgentEvent::TurnDone {
                ok,
                cost: v.get("total_cost_usd").and_then(JsonValue::as_f64),
                turns: v.get("num_turns").and_then(JsonValue::as_i64),
                message: if ok {
                    None
                } else {
                    str_of(Some(&v), "result").or_else(|| str_of(Some(&v), "subtype"))
                },
            }]
        }
        _ => Vec::new(),
    }
}

fn short_tool(name: &str) -> &str {
    name.strip_prefix(TOOL_PREFIX).unwrap_or(name)
}

/// `{"action":"show","set":"x"}` as `action=show set=x`: compact, for a console line.
pub fn summarize(input: Option<&JsonValue>) -> String {
    let Some(o) = input.and_then(JsonValue::as_object) else {
        return String::new();
    };
    if o.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = o
        .iter()
        .map(|(k, v)| match v {
            JsonValue::String(s) => {
                if count(s) > 40 {
                    format!("{k}={}…", prefix(s, 37))
                } else {
                    format!("{k}={s}")
                }
            }
            JsonValue::Object(_) | JsonValue::Array(_) => {
                let t = v.js_stringified();
                if count(&t) > 48 {
                    format!("{k}={}…", prefix(&t, 45))
                } else {
                    format!("{k}={t}")
                }
            }
            _ => format!("{k}={}", v.js_stringified()),
        })
        .collect();
    let s = parts.join(" ");
    if count(&s) > 200 {
        format!("{}…", prefix(&s, 197))
    } else {
        s
    }
}

fn result_text(c: Option<&JsonValue>) -> String {
    if let Some(s) = c.and_then(JsonValue::as_str) {
        return s.to_string();
    }
    c.and_then(JsonValue::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("text").and_then(JsonValue::as_str))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn one_line(s: &str) -> String {
    let flat = s.replace('\n', " ");
    if count(&flat) > 220 {
        format!("{}…", prefix(&flat, 217))
    } else {
        flat
    }
}

/// One user turn, as a stream-json input line (no trailing newline).
pub fn user_message(text: &str) -> String {
    JsonValue::Object(JsonObject::from_pairs([
        ("type", JsonValue::from("user")),
        (
            "message",
            JsonValue::Object(JsonObject::from_pairs([
                ("role", JsonValue::from("user")),
                ("content", JsonValue::from(text)),
            ])),
        ),
    ]))
    .js_stringified()
}

/// `AgentStream.Launch`.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub agent: String,
    pub model: Option<String>,
    pub budget_usd: Option<f64>,
    /// Resume this session (the conversation continues), else a fresh one.
    pub resume: Option<String>,
    /// A JSON file naming only the app's MCP server; `None` = whatever the user's config has.
    pub mcp_config_path: Option<String>,
    pub allowed_tools: String,
}

impl Default for Launch {
    fn default() -> Self {
        Launch {
            agent: "dj".into(),
            model: None,
            budget_usd: Some(5.0),
            resume: None,
            mcp_config_path: None,
            allowed_tools: "mcp__dj-vevr-se,Skill,Read,Glob,Grep".into(),
        }
    }
}

/// C's `%g` (six significant digits, trailing zeros trimmed), for `--max-budget-usd`.
pub fn format_g(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { "0".into() } else { x.to_string() };
    }
    let e = libm::floor(libm::log10(libm::fabs(x))) as i32;
    if !(-4..6).contains(&e) {
        // exponent form, e.g. 1.5e+07
        let m = x / libm::pow(10.0, e as f64);
        let mut s = format!("{m:.5}");
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        return format!("{s}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = (5 - e).max(0) as usize;
    let mut s = format!("{x:.decimals$}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    s
}

/// `AgentStream.arguments`.
pub fn arguments(l: &Launch) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--agent",
        &l.agent,
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        &l.allowed_tools,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(p) = &l.mcp_config_path {
        a.extend([
            "--strict-mcp-config".into(),
            "--mcp-config".into(),
            p.clone(),
        ]);
    }
    if let Some(m) = l.model.as_deref().filter(|m| !m.is_empty()) {
        a.extend(["--model".into(), m.to_string()]);
    }
    if let Some(b) = l.budget_usd.filter(|b| *b > 0.0) {
        a.extend(["--max-budget-usd".into(), format_g(b)]);
    }
    if let Some(r) = l.resume.as_deref().filter(|r| !r.is_empty()) {
        a.extend(["--resume".into(), r.to_string()]);
    }
    a
}

/// The MCP config naming only the app's own server (`djctl mcp` over the control socket).
pub fn mcp_config(djctl: &str) -> String {
    let server = JsonValue::Object(JsonObject::from_pairs([
        ("command", JsonValue::from(djctl)),
        ("args", JsonValue::Array(alloc::vec!["mcp".into()])),
    ]));
    JsonValue::Object(JsonObject::from_pairs([(
        "mcpServers",
        JsonValue::Object(JsonObject::from_pairs([(MCP_SERVER_NAME, server)])),
    )]))
    .js_stringified()
}

/// Where `claude` is: `$DJ_CLAUDE`, then PATH, then the usual install places.
/// `env` looks a variable up; `exists` says whether a path is an executable.
pub fn find_claude(
    env: &dyn Fn(&str) -> Option<String>,
    home: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if let Some(p) = env("DJ_CLAUDE") {
        if exists(&p) {
            return Some(p);
        }
    }
    let mut dirs: Vec<String> = env("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();
    dirs.extend([
        format!("{home}/.local/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        format!("{home}/.claude/local"),
        format!("{home}/.npm-global/bin"),
    ]);
    dirs.into_iter()
        .map(|d| format!("{d}/claude"))
        .find(|p| exists(p))
}

/// `djctl` for the MCP config: `$DJ_DJCTL`, the project's `build/mcp/djctl` (`make mcp-install`).
pub fn find_djctl(
    env: &dyn Fn(&str) -> Option<String>,
    cwd: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if let Some(p) = env("DJ_DJCTL") {
        if exists(&p) {
            return Some(p);
        }
    }
    let p = format!("{cwd}/build/mcp/djctl");
    exists(&p).then_some(p)
}
