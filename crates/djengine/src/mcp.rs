//! The MCP face of the control socket: stdio JSON-RPC in, one control request out per tool
//! call. Port of `Control/MCPBridge.swift`. It knows the tool list and how to phrase errors; the
//! host decides what each request does. Free of I/O so it can be tested line by line.
//!
//! The tool text (descriptions, schemas, the server instructions, the tool → method map) is
//! `data/mcp-tools.json`, extracted from the Swift source by `tools/mcp-catalog/extract.py` so it
//! stays byte-identical instead of being retyped.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use once_cell::race::OnceBox;

use crate::console::{Property, ToolInfo};
use crate::json::{JsonObject, JsonValue};

/// The embedded catalog file.
pub const CATALOG_JSON: &str = include_str!("../data/mcp-tools.json");

/// `serverInfo.name` and the key of the app's server in an MCP config. beats-btw-dj's Swift
/// still says `dj-vevr-se` here (the app was renamed on 2026-10-22, this string was not), and
/// clients key their tool names on it (`mcp__dj-vevr-se__dj_set`), so it is kept.
pub const SERVER_NAME: &str = "dj-vevr-se";

/// `BuildInfo.dev`'s id: what an unbundled build reports as its version.
pub const DEV_VERSION: &str = "dev";

/// The parsed catalog file.
pub struct Catalog {
    pub source: String,
    pub commit: String,
    pub instructions: String,
    /// Tool name → control method, in the Swift dictionary's declaration order.
    pub methods: Vec<(String, String)>,
    /// `MCPBridge.tools`: the MCP `tools/list` entries.
    pub tools: Vec<JsonValue>,
}

static SHARED: OnceBox<Catalog> = OnceBox::new();

impl Catalog {
    /// The catalog bundled with the crate, parsed once.
    pub fn shared() -> &'static Catalog {
        SHARED.get_or_init(|| {
            let v = JsonValue::parse(CATALOG_JSON).expect("data/mcp-tools.json parses");
            let text = |k: &str| {
                v.get(k)
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let methods = v
                .get("methods")
                .and_then(JsonValue::as_object)
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, m)| m.as_str().map(|m| (k.to_string(), m.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            let tools = v
                .get("tools")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            Box::new(Catalog {
                source: text("source"),
                commit: text("commit"),
                instructions: text("instructions"),
                methods,
                tools,
            })
        })
    }

    /// Tool name → control method (`MCPBridge.methods[name]`).
    pub fn method(&self, tool: &str) -> Option<&str> {
        self.methods
            .iter()
            .find(|(n, _)| n == tool)
            .map(|(_, m)| m.as_str())
    }

    /// Every tool the MCP lists, with its control method, in the MCP's order
    /// (`MCPBridge.catalog`).
    pub fn catalog(&self) -> Vec<ToolInfo> {
        self.tools
            .iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?;
                let method = self.method(name)?;
                let schema = t.get("inputSchema");
                let properties = schema
                    .and_then(|s| s.get("properties"))
                    .and_then(JsonValue::as_object)
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| Property {
                                name: k.to_string(),
                                type_: v
                                    .get("type")
                                    .and_then(JsonValue::as_str)
                                    .unwrap_or("any")
                                    .to_string(),
                                description: v
                                    .get("description")
                                    .and_then(JsonValue::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let required = schema
                    .and_then(|s| s.get("required"))
                    .and_then(JsonValue::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(ToolInfo {
                    name: name.to_string(),
                    method: method.to_string(),
                    description: t
                        .get("description")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    properties,
                    required,
                })
            })
            .collect()
    }
}

/// `MCPBridge.catalog`, for the console parser.
pub fn catalog() -> Vec<ToolInfo> {
    Catalog::shared().catalog()
}

/// What a control request does: `(method, params)` → the result, or the error text (a remote
/// error, "DJApp is not running …", …) that becomes an `isError` tool result.
pub type Call<'a> = dyn FnMut(&str, &JsonValue) -> Result<JsonValue, String> + 'a;

/// `MCPBridge`: stdio JSON-RPC in (`djctl mcp`), one control request out per tool call.
pub struct McpBridge<'a> {
    pub version: String,
    /// Reads the PNG a `dj_screenshot` call saved, so it comes back as the image itself (the
    /// app wrote it on this machine). `None` = the text part only.
    pub read_file: Option<&'a dyn Fn(&str) -> Option<Vec<u8>>>,
    /// Tools beyond the Swift catalog (beyond Swift: `dj_post`, docs/POST.md), listed after the
    /// catalog's: (tool `tools/list` entry, control method). The host's `call` answers their
    /// methods; the catalog itself stays byte-identical to Swift's.
    pub extra: Vec<(JsonValue, String)>,
}

/// `dj_post` (docs/POST.md): the post chain on a file, answered by the host itself (`post`).
pub fn post_tool() -> (JsonValue, String) {
    let schema = JsonValue::parse(POST_SCHEMA).expect("the post schema parses");
    (
        obj(alloc::vec![
            ("name", "dj_post".into()),
            ("description", POST_DESCRIPTION.into()),
            ("inputSchema", schema),
        ]),
        "post".to_string(),
    )
}

/// What `dj_post` and `djapp_post` say about the file actions.
pub const POST_DESCRIPTION: &str = "Post-process (master) an audio file offline, natively in Rust (no ffmpeg): an ordered chain of stages (highpass, shelves / EQ, spectral FFT denoise with noise tracking, downward expander, loudness to a target LUFS with a true-peak ceiling in two passes like ffmpeg loudnorm, limiter). action: presets (the list: name, stages, the ffmpeg graph each stands for; the default is clean-medium = highpass 25 Hz, medium denoise for line-input hiss, -14 LUFS / -1 dBTP), measure (in: LUFS, LRA, true peak, noise floor), process (default: in, out (.wav, or .mp3 when lame/ffmpeg is installed), preset or post_chain). Returns the before / after LUFS, LRA, true peak and noise-floor estimate. Never touches what plays.";

const POST_SCHEMA: &str = r#"{"type":"object","properties":{"action":{"type":"string","enum":["process","measure","presets"]},"in":{"type":"string","description":"the audio file to read (WAV; MP3/FLAC/AAC with djctl's decode feature)"},"out":{"type":"string","description":"the file to write: .wav, or .mp3 (encoded by lame or ffmpeg if on the PATH, else a .wav beside it); default <in>-<preset>.wav"},"preset":{"type":"string","enum":["none","clean-medium","clean-light","clean-strong","expander","treble-shelf","master"]},"post_chain":{"description":"a custom chain: an array of stages {stage: highpass|lowpass|highshelf|lowshelf|peak|denoise|expander|gain|loudness|limiter, ...params} and preset names, overriding preset"},"float32":{"type":"boolean"}}}"#;

fn obj(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

fn text_content(text: String, is_error: bool) -> JsonValue {
    obj(alloc::vec![
        (
            "content",
            JsonValue::Array(alloc::vec![obj(alloc::vec![
                ("type", "text".into()),
                ("text", text.into())
            ])])
        ),
        ("isError", is_error.into()),
    ])
}

fn result(id: &JsonValue, r: JsonValue) -> String {
    obj(alloc::vec![
        ("jsonrpc", "2.0".into()),
        ("id", id.clone()),
        ("result", r)
    ])
    .js_stringified()
}

fn error(id: &JsonValue, code: i64, message: &str) -> String {
    obj(alloc::vec![
        ("jsonrpc", "2.0".into()),
        ("id", id.clone()),
        (
            "error",
            obj(alloc::vec![
                ("code", JsonValue::int(code)),
                ("message", message.into())
            ])
        ),
    ])
    .js_stringified()
}

/// Standard base64 with padding (`Data.base64EncodedString()`).
pub fn base64_std(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

impl<'a> McpBridge<'a> {
    pub fn new(version: impl Into<String>) -> Self {
        McpBridge {
            version: version.into(),
            read_file: None,
            extra: Vec::new(),
        }
    }

    /// One JSON-RPC line in → the reply line, or `None` for a notification.
    pub fn handle(&self, line: &str, call: &mut Call<'_>) -> Option<String> {
        let Ok(req) = JsonValue::parse(line) else {
            return Some(error(&JsonValue::Null, -32700, "parse error"));
        };
        let id = match req.get("id") {
            Some(JsonValue::Null) | None => return None, // notifications
            Some(id) => id.clone(),
        };
        let cat = Catalog::shared();
        let method = req.get("method").and_then(JsonValue::as_str).unwrap_or("");
        match method {
            "initialize" => {
                let asked = req
                    .get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("2025-06-18");
                Some(result(
                    &id,
                    obj(alloc::vec![
                        ("protocolVersion", asked.into()),
                        (
                            "capabilities",
                            obj(alloc::vec![("tools", obj(alloc::vec![]))])
                        ),
                        (
                            "serverInfo",
                            obj(alloc::vec![
                                ("name", SERVER_NAME.into()),
                                ("version", self.version.as_str().into())
                            ])
                        ),
                        ("instructions", cat.instructions.as_str().into()),
                    ]),
                ))
            }
            "ping" => Some(result(&id, obj(alloc::vec![]))),
            "tools/list" => {
                let mut tools = cat.tools.clone();
                tools.extend(self.extra.iter().map(|(t, _)| t.clone()));
                Some(result(&id, obj(alloc::vec![("tools", JsonValue::Array(tools))])))
            }
            "tools/call" => {
                let params = req.get("params");
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let args = params
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| obj(alloc::vec![]));
                let extra = self
                    .extra
                    .iter()
                    .find(|(t, _)| t.get("name").and_then(JsonValue::as_str) == Some(name))
                    .map(|(_, m)| m.as_str());
                let Some(target) = cat.method(name).or(extra) else {
                    return Some(error(&id, -32602, &alloc::format!("unknown tool {name}")));
                };
                match call(target, &args) {
                    Ok(out) => {
                        // A screenshot comes back as the image itself.
                        if name == "dj_screenshot" {
                            let png = out
                                .get("path")
                                .and_then(JsonValue::as_str)
                                .and_then(|p| self.read_file.and_then(|f| f(p)));
                            if let Some(png) = png {
                                return Some(result(
                                    &id,
                                    obj(alloc::vec![
                                        (
                                            "content",
                                            JsonValue::Array(alloc::vec![
                                                obj(alloc::vec![
                                                    ("type", "image".into()),
                                                    ("data", base64_std(&png).into()),
                                                    ("mimeType", "image/png".into()),
                                                ]),
                                                obj(alloc::vec![
                                                    ("type", "text".into()),
                                                    ("text", out.js_stringified().into()),
                                                ]),
                                            ])
                                        ),
                                        ("isError", false.into()),
                                    ]),
                                ));
                            }
                        }
                        Some(result(&id, text_content(out.js_stringified(), false)))
                    }
                    Err(e) => Some(result(&id, text_content(e, true))),
                }
            }
            _ => Some(error(
                &id,
                -32601,
                &alloc::format!("method not found: {method}"),
            )),
        }
    }
}
