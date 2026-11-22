//! The debug console's command language: a CLI over the same tools the MCP exposes — one
//! parser, built from the MCP's own tool list, so a tool added to the MCP is in the console
//! the same moment, with its help and completion. Pure: a line in, a command out. Port of
//! `Control/ConsoleCommand.swift`.
//!
//! ```text
//! state                                  a tool, by its short name (dj_ is optional)
//! set show first-night                   bare words fill the tool's usual first arguments
//! set action=plan set=fri count=6 shape=arc bpm=[124,150]    key=value, values read as JSON
//! perform op=crossfader value=0          dj_perform: key=value pairs are the action's fields
//! compose {"deck":"A","parts":{…}}        a JSON object is merged into the arguments
//! raw loadShare {"deck":"A","link":"…"}   a control-socket method, directly
//! help [tool] · tools [filter] · clear · history · expand · agent <start|stop|say …>
//! ```
//!
//! `MCPBridge.catalog` (the tool list) is the MCP bridge's to build; the parser takes it as a
//! parameter ([`ToolInfo`]).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};

/// `MCPToolInfo.Property`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub type_: String,
    pub description: String,
}

/// `MCPToolInfo`: one MCP tool as the console sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInfo {
    /// dj_set
    pub name: String,
    /// set (the control-socket method)
    pub method: String,
    pub description: String,
    pub properties: Vec<Property>,
    pub required: Vec<String>,
}

fn prefix_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl ToolInfo {
    /// `set` for `dj_set`.
    pub fn short_name(&self) -> &str {
        self.name.strip_prefix("dj_").unwrap_or(&self.name)
    }

    /// The first sentence, for the tool list.
    pub fn summary(&self) -> String {
        let flat = self.description.replace('\n', " ");
        let dot = flat.find(". ").or_else(|| flat.find(": "));
        if let Some(d) = dot {
            let s = &flat[..d];
            if s.chars().count() >= 16 {
                return if s.chars().count() > 110 {
                    format!("{}…", prefix_chars(s, 107))
                } else {
                    s.to_string()
                };
            }
        }
        if flat.chars().count() > 110 {
            format!("{}…", prefix_chars(&flat, 107))
        } else {
            flat
        }
    }
}

/// `ConsoleMessage`: a console message that is an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleMessage(pub String);

/// `ConsoleCommand`.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleCommand {
    Blank,
    Help(Option<String>),
    Tools(Option<String>),
    Clear,
    History,
    Expand,
    Tool {
        name: String,
        method: String,
        params: JsonValue,
    },
    Raw {
        method: String,
        params: JsonValue,
    },
    /// `agent <sub> [text]`: the Claude process.
    Agent(String, String),
    Error(String),
}

/// Bare words fill these arguments in order, for the tools whose first arguments are usually
/// obvious. A word past the list is an error.
fn positional(tool: &str) -> &'static [&'static str] {
    match tool {
        "dj_set" => &["action", "set"],
        "dj_compositions" => &["action", "id"],
        "dj_favorites" | "dj_journal" | "dj_loops" | "dj_notes" | "dj_view" | "dj_settings"
        | "dj_variations" => &["action"],
        "dj_record" => &["on"],
        "dj_palettes" => &["deck"],
        "dj_generate" => &["deck", "genre", "seed"],
        "dj_load_share" => &["deck", "link"],
        "dj_load_file" => &["deck", "path"],
        "dj_import" => &["path", "deck"],
        "dj_perform" => &["op", "deck"],
        "dj_press_pad" => &["group", "index"],
        "dj_copy_deck" => &["from", "to"],
        _ => &[],
    }
}

pub const BUILTINS: [&str; 7] = [
    "help", "tools", "clear", "history", "expand", "raw", "agent",
];

/// A value as JSON when it reads as JSON (numbers, true / false / null, arrays, objects),
/// else text.
pub fn value(s: &str) -> JsonValue {
    match s {
        "true" => return JsonValue::Bool(true),
        "false" => return JsonValue::Bool(false),
        "null" => return JsonValue::Null,
        _ => {}
    }
    if let Some(first) = s.chars().next() {
        if "{[-0123456789".contains(first) {
            if let Ok(v) = JsonValue::parse(s) {
                return v;
            }
        }
    }
    JsonValue::str(s)
}

/// `tokens`: split a line into words: quotes group (and drop their quotes), a `{…}` / `[…]`
/// JSON value is one word (balanced, quotes inside respected), also after `key=`.
pub fn tokens(line: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut i = 0;
    let flush = |cur: &mut String, started: &mut bool, out: &mut Vec<String>| {
        if *started {
            out.push(core::mem::take(cur));
        }
        cur.clear();
        *started = false;
    };
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' {
            flush(&mut cur, &mut started, &mut out);
            i += 1;
        } else if c == '"' || c == '\'' {
            let q = c;
            started = true;
            i += 1;
            while i < chars.len() && chars[i] != q {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                cur.push(chars[i]);
                i += 1;
            }
            if i >= chars.len() {
                return Err("unterminated quote".to_string());
            }
            i += 1;
        } else if c == '{' || c == '[' {
            started = true;
            let mut depth = 0i32;
            let mut in_string = false;
            let mut esc = false;
            while i < chars.len() {
                let d = chars[i];
                cur.push(d);
                if in_string {
                    if esc {
                        esc = false;
                    } else if d == '\\' {
                        esc = true;
                    } else if d == '"' {
                        in_string = false;
                    }
                } else if d == '"' {
                    in_string = true;
                } else if d == '{' || d == '[' {
                    depth += 1;
                } else if d == '}' || d == ']' {
                    depth -= 1;
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            if depth != 0 {
                return Err(format!("unbalanced {c} in JSON"));
            }
        } else {
            started = true;
            cur.push(c);
            i += 1;
        }
    }
    flush(&mut cur, &mut started, &mut out);
    Ok(out)
}

/// `ConsoleParser`, over a tool catalog.
pub struct ConsoleParser<'a> {
    pub catalog: &'a [ToolInfo],
}

impl<'a> ConsoleParser<'a> {
    pub fn new(catalog: &'a [ToolInfo]) -> Self {
        ConsoleParser { catalog }
    }

    /// Resolve `word` to a tool: its full name, its short name, or an unambiguous prefix
    /// (3+ letters).
    pub fn resolve(&self, word: &str) -> Result<&'a ToolInfo, ConsoleMessage> {
        let w = word.to_lowercase();
        if let Some(t) = self
            .catalog
            .iter()
            .find(|t| t.name == w || t.short_name() == w)
        {
            return Ok(t);
        }
        let near: Vec<&ToolInfo> = self
            .catalog
            .iter()
            .filter(|t| t.short_name().starts_with(&w) || t.name.starts_with(&w))
            .collect();
        if near.len() == 1 && w.chars().count() >= 3 {
            return Ok(near[0]);
        }
        if near.len() > 1 {
            let mut names: Vec<&str> = near.iter().map(|t| t.short_name()).collect();
            names.sort();
            return Err(ConsoleMessage(format!(
                "`{word}` is ambiguous: {}",
                names.join(", ")
            )));
        }
        let mut guess: Vec<&str> = self
            .catalog
            .iter()
            .map(ToolInfo::short_name)
            .filter(|s| s.contains(&w))
            .collect();
        guess.sort();
        Err(ConsoleMessage(format!(
            "unknown command `{word}`{}",
            if guess.is_empty() {
                " (help lists the tools)".to_string()
            } else {
                format!(": did you mean {}?", guess.join(", "))
            }
        )))
    }

    pub fn parse(&self, line: &str) -> ConsoleCommand {
        let trimmed = line.trim_matches(|c: char| c.is_whitespace());
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return ConsoleCommand::Blank;
        }
        let words = match tokens(trimmed) {
            Ok(w) => w,
            Err(m) => return ConsoleCommand::Error(m),
        };
        let Some(head) = words.first().map(|w| w.to_lowercase()) else {
            return ConsoleCommand::Blank;
        };
        let rest = &words[1..];
        match head.as_str() {
            "help" | "?" => return ConsoleCommand::Help(rest.first().cloned()),
            "tools" | "ls" => return ConsoleCommand::Tools(rest.first().cloned()),
            "clear" | "cls" => return ConsoleCommand::Clear,
            "history" => return ConsoleCommand::History,
            "expand" => return ConsoleCommand::Expand,
            "agent" => {
                let sub = rest
                    .first()
                    .map(|s| s.to_lowercase())
                    .unwrap_or_else(|| "status".into());
                // the text after the subcommand, as typed
                let needle = rest.first().map(String::as_str).unwrap_or("\u{0}");
                let text = match trimmed.find(needle) {
                    Some(p) => trimmed[p + needle.len()..]
                        .trim_matches(|c| c == ' ' || c == '\t')
                        .to_string(),
                    None => String::new(),
                };
                return ConsoleCommand::Agent(sub, text);
            }
            "raw" => {
                let Some(method) = rest.first() else {
                    return ConsoleCommand::Error(
                        "raw <method> [json]: a control-socket method".into(),
                    );
                };
                let mut params = JsonObject::new();
                for w in &rest[1..] {
                    if w.starts_with('{') {
                        let Some(o) = JsonValue::parse(w)
                            .ok()
                            .and_then(|v| v.as_object().cloned())
                        else {
                            return ConsoleCommand::Error(format!("not a JSON object: {w}"));
                        };
                        for (k, x) in o.iter() {
                            params.set(k, x.clone());
                        }
                    } else if let Some(eq) = w.find('=') {
                        params.set(&w[..eq], value(&w[eq + 1..]));
                    } else {
                        return ConsoleCommand::Error(format!(
                            "raw takes key=value or a JSON object, not `{w}`"
                        ));
                    }
                }
                return ConsoleCommand::Raw {
                    method: method.clone(),
                    params: JsonValue::Object(params),
                };
            }
            _ => {}
        }

        let tool = match self.resolve(&head) {
            Ok(t) => t,
            Err(m) => return ConsoleCommand::Error(m.0),
        };
        let mut params = JsonObject::new();
        // dj_perform: loose key=value pairs are the action's fields
        let mut extras = JsonObject::new();
        let mut order: Vec<&str> = positional(&tool.name).to_vec();
        for w in rest {
            if w.starts_with('{') {
                let Some(o) = JsonValue::parse(w)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                else {
                    return ConsoleCommand::Error(format!("not a JSON object: {w}"));
                };
                for (k, x) in o.iter() {
                    params.set(k, x.clone());
                }
                continue;
            }
            let eq = w.find('=').filter(|&e| {
                e != 0
                    && w[..e]
                        .chars()
                        .all(|c| c.is_alphabetic() || c.is_numeric() || c == '_' || c == '.')
            });
            if let Some(eq) = eq {
                let key = &w[..eq];
                let val = value(&w[eq + 1..]);
                if tool.name == "dj_perform" && key != "action" {
                    extras.set(key, val);
                } else {
                    params.set(key, val);
                    order.retain(|o| *o != key);
                }
            } else {
                if order.is_empty() {
                    return ConsoleCommand::Error(format!(
                        "`{w}`: no argument to put it in. Use key=value ({})",
                        tool.properties
                            .iter()
                            .take(6)
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let key = order.remove(0);
                if tool.name == "dj_perform" {
                    extras.set(key, value(w));
                } else {
                    params.set(key, value(w));
                }
            }
        }
        if tool.name == "dj_perform" && !extras.is_empty() {
            let mut action = params
                .get("action")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default();
            for (k, x) in extras.iter() {
                action.set(k, x.clone());
            }
            params.set("action", JsonValue::Object(action));
        }
        ConsoleCommand::Tool {
            name: tool.name.clone(),
            method: tool.method.clone(),
            params: JsonValue::Object(params),
        }
    }

    /// Candidates for the word being typed at the end of `line` (a trailing space starts a
    /// new word).
    pub fn complete(&self, line: &str) -> Vec<String> {
        let ends_with_space = line.ends_with(' ');
        let words: Vec<String> = tokens(line).unwrap_or_else(|_| {
            line.split(' ')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        });
        if words.is_empty() || (words.len() == 1 && !ends_with_space) {
            let prefix = words.first().map(|w| w.to_lowercase()).unwrap_or_default();
            let mut all: BTreeSet<String> = BUILTINS.iter().map(|s| s.to_string()).collect();
            all.extend(self.catalog.iter().map(|t| t.short_name().to_string()));
            return all.into_iter().filter(|s| s.starts_with(&prefix)).collect();
        }
        let Ok(tool) = self.resolve(&words[0]) else {
            return Vec::new();
        };
        let current: &str = if ends_with_space {
            ""
        } else {
            words.last().unwrap()
        };
        if current.contains('=') {
            return Vec::new();
        }
        let used: BTreeSet<&str> = words[1..]
            .iter()
            .filter_map(|w| w.split('=').next())
            .collect();
        let mut out: Vec<String> = tool
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| n.starts_with(current) && !used.contains(n))
            .map(|n| format!("{n}="))
            .collect();
        out.sort();
        out
    }

    /// `ConsoleHelp.overview`.
    pub fn overview(&self, filter: Option<&str>) -> String {
        let f = filter.map(|f| f.to_lowercase());
        let all: Vec<&ToolInfo> = self
            .catalog
            .iter()
            .filter(|t| match &f {
                None => true,
                Some(f) => {
                    t.name.contains(f.as_str()) || t.description.to_lowercase().contains(f.as_str())
                }
            })
            .collect();
        let width = all
            .iter()
            .map(|t| t.short_name().chars().count())
            .max()
            .unwrap_or(8)
            + 2;
        all.iter()
            .map(|t| format!("{:<width$}{}", t.short_name(), t.summary()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `ConsoleHelp.detail`.
    pub fn detail(&self, word: &str) -> String {
        match self.resolve(word) {
            Err(m) => m.0,
            Ok(t) => {
                let mut s = format!(
                    "{}  (control method `{}`)\n\n{}\n",
                    t.name, t.method, t.description
                );
                if !t.properties.is_empty() {
                    s.push_str("\narguments:\n");
                    let w = t
                        .properties
                        .iter()
                        .map(|p| p.name.chars().count())
                        .max()
                        .unwrap_or(4)
                        + 1;
                    for p in &t.properties {
                        let req = if t.required.contains(&p.name) {
                            " (required)"
                        } else {
                            ""
                        };
                        s.push_str(&format!(
                            "  {:<w$}{}{req}  {}\n",
                            p.name, p.type_, p.description
                        ));
                    }
                }
                let pos = positional(&t.name);
                if !pos.is_empty() {
                    s.push_str(&format!("\nbare words fill: {}", pos.join(", ")));
                }
                s
            }
        }
    }
}

/// `ConsoleHelp.usage`.
pub const USAGE: &str = "A CLI over the same tools the MCP exposes (every `dj_*` tool; the dj_ is optional).

  <tool> [word …] [key=value …] [{json}]     e.g.  state · set show first-night · perform play A on=true
  raw <method> [key=value …] [{json}]        a control-socket method directly
  tools [filter]    list the tools        help <tool>    one tool's arguments
  history           your commands         clear · expand (the last long output in full)
  agent <start|stop|restart|status|say <text>|wake on|off|cwd <dir>|model <m>|budget <usd>>
                    the Claude agent: start it, talk to it, see what it does

Bare words fill a tool's usual first arguments (set show NAME → action=show set=NAME).
Values read as JSON (6, true, [124,150]) else text. `perform` takes its fields loose:
perform op=crossfader value=0. ↑/↓ history · ⇥ complete · ⌘K clear.";
