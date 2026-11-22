//! The LSP server as a pure state machine: a JSON-RPC message in, the messages to send back out (responses and
//! notifications such as `textDocument/publishDiagnostics`). `btwd lsp` adds the stdio framing; djweb's `BtwLs` calls
//! [`Server::request`] and [`Server::open`] directly.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};

use crate::analysis::Analysis;
use crate::json::{arr, get_range, get_str, obj, s};
use crate::text::Encoding;

/// An open document: its version and its analysis.
pub struct Document {
    pub version: i64,
    pub analysis: Analysis,
}

/// JSON-RPC error codes.
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const REQUEST_FAILED: i64 = -32803;
pub const SERVER_NOT_INITIALIZED: i64 = -32002;

pub type Reply = Result<JsonValue, (i64, String)>;

#[derive(Default)]
pub struct Server {
    pub docs: BTreeMap<String, Document>,
    pub encoding: Encoding,
    initialized: bool,
    shutdown: bool,
    /// The client sent `exit`: the host stops reading.
    pub exited: bool,
    /// The check (the compile) of a changed document is left for the host to run off the request path ([`Server::stale`],
    /// [`Server::install`]); the diagnostics published at once are the parser's. Off: every change is checked before it is
    /// answered (tests, and hosts with no worker).
    pub deferred: bool,
    /// The registry completion and hover read the `do` nouns, verbs and params from (None: the site's snapshot, which has
    /// no param docs; `btwd lsp` hands in the host's own, which has them). Checks always use the site's environment.
    pub registry: Option<&'static dyn djlive::env::Registry>,
}

pub fn invalid(what: &str) -> (i64, String) {
    (INVALID_PARAMS, format!("invalid params: {what}"))
}

impl Server {
    pub fn new() -> Server {
        Server::default()
    }

    /// A server that needs no `initialize` (the browser editor), counting positions in `enc`.
    pub fn with_encoding(enc: Encoding) -> Server {
        Server { encoding: enc, initialized: true, ..Server::default() }
    }

    /// Handle one message; answer with what to send.
    pub fn handle(&mut self, msg: &JsonValue) -> Vec<JsonValue> {
        let method = msg.get("method").and_then(JsonValue::as_str).unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(JsonValue::Null);
        let mut out = Vec::new();
        match id {
            // a response from the client to a request of ours (we send none): nothing to do
            Some(_) if method.is_empty() => {}
            Some(id) => {
                let reply = if !self.initialized && method != "initialize" {
                    Err((SERVER_NOT_INITIALIZED, "the server is not initialized".to_string()))
                } else {
                    self.request(&method, &params)
                };
                out.push(match reply {
                    Ok(result) => obj([("jsonrpc", s("2.0")), ("id", id), ("result", result)]),
                    Err((code, message)) => obj([("jsonrpc", s("2.0")), ("id", id), ("error", obj([("code", JsonValue::int(code)), ("message", s(message))]))]),
                });
            }
            None => out.extend(self.notify(&method, &params)),
        }
        out
    }

    fn notify(&mut self, method: &str, params: &JsonValue) -> Vec<JsonValue> {
        match method {
            "initialized" => Vec::new(),
            "exit" => {
                self.exited = true;
                Vec::new()
            }
            "textDocument/didOpen" => {
                let (Some(uri), Some(text)) = (get_str(params, &["textDocument", "uri"]), get_str(params, &["textDocument", "text"])) else { return Vec::new() };
                let version = params.get("textDocument").and_then(|t| t.get("version")).and_then(JsonValue::as_f64).unwrap_or(0.0) as i64;
                let uri = uri.to_string();
                self.open(&uri, text, version);
                vec![self.publish(&uri)]
            }
            "textDocument/didChange" => {
                let Some(uri) = get_str(params, &["textDocument", "uri"]).map(str::to_string) else { return Vec::new() };
                let version = params.get("textDocument").and_then(|t| t.get("version")).and_then(JsonValue::as_f64).unwrap_or(0.0) as i64;
                let Some(changes) = params.get("contentChanges").and_then(JsonValue::as_array) else { return Vec::new() };
                let mut text = match self.docs.get(&uri) {
                    Some(d) => d.analysis.text.clone(),
                    None => String::new(),
                };
                for c in changes {
                    let Some(new) = c.get("text").and_then(JsonValue::as_str) else { continue };
                    match c.get("range").and_then(get_range) {
                        // an incremental change: positions are into the text as the earlier changes left it
                        Some(r) => {
                            let ix = crate::text::LineIndex::new(&text);
                            let sp = ix.span(&text, r, self.encoding);
                            text.replace_range(sp.start..sp.end, new);
                        }
                        None => text = new.to_string(),
                    }
                }
                self.open(&uri, &text, version);
                vec![self.publish(&uri)]
            }
            "textDocument/didClose" => {
                let Some(uri) = get_str(params, &["textDocument", "uri"]).map(str::to_string) else { return Vec::new() };
                self.docs.remove(&uri);
                // the editor forgets the document's diagnostics
                vec![obj([("jsonrpc", s("2.0")), ("method", s("textDocument/publishDiagnostics")), ("params", obj([("uri", s(uri)), ("diagnostics", arr(Vec::new()))]))])]
            }
            _ => Vec::new(),
        }
    }

    /// Open (or replace) a document and analyse it (and check it, unless checks are [`Server::deferred`]).
    pub fn open(&mut self, uri: &str, text: &str, version: i64) {
        let analysis = if self.deferred { Analysis::new(text) } else { Analysis::checked(text) };
        self.docs.insert(uri.to_string(), Document { version, analysis });
    }

    /// The documents whose check is not done: (uri, version, text), for a worker to check with [`crate::analysis::full_check`].
    pub fn stale(&self) -> Vec<(String, i64, String)> {
        self.docs.iter().filter(|(_, d)| d.analysis.check_ready().is_none()).map(|(u, d)| (u.clone(), d.version, d.analysis.text.clone())).collect()
    }

    /// A worker's check of `uri` at `version`: kept if the document is still at that version, and its diagnostics published.
    pub fn install(&mut self, uri: &str, version: i64, text: &str, check: djlive::compile::Check) -> Option<JsonValue> {
        let d = self.docs.get(uri)?;
        if d.version != version || d.analysis.text != text || d.analysis.check_ready().is_some() {
            return None;
        }
        d.analysis.set_check(check);
        Some(self.publish(uri))
    }

    pub fn doc(&self, uri: &str) -> Option<&Document> {
        self.docs.get(uri)
    }

    /// The `publishDiagnostics` notification for a document.
    pub fn publish(&self, uri: &str) -> JsonValue {
        let (diags, version) = match self.docs.get(uri) {
            Some(d) => (crate::diagnostics::all(&d.analysis, self.encoding), JsonValue::int(d.version)),
            None => (arr(Vec::new()), JsonValue::Null),
        };
        obj([("jsonrpc", s("2.0")), ("method", s("textDocument/publishDiagnostics")), ("params", obj([("uri", s(uri)), ("version", version), ("diagnostics", diags)]))])
    }

    /// The document a request's `textDocument.uri` names.
    pub fn doc_of(&self, params: &JsonValue) -> Result<&Analysis, (i64, String)> {
        let uri = get_str(params, &["textDocument", "uri"]).ok_or_else(|| invalid("textDocument.uri"))?;
        self.docs.get(uri).map(|d| &d.analysis).ok_or_else(|| (REQUEST_FAILED, format!("{uri} is not open")))
    }

    /// The byte offset a request's `position` names in its document.
    pub fn offset_of(&self, a: &Analysis, params: &JsonValue) -> Result<usize, (i64, String)> {
        let p = params.get("position").and_then(crate::json::get_pos).ok_or_else(|| invalid("position"))?;
        Ok(a.index.offset(&a.text, p, self.encoding))
    }

    /// Answer one request.
    pub fn request(&mut self, method: &str, params: &JsonValue) -> Reply {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "shutdown" => {
                self.shutdown = true;
                Ok(JsonValue::Null)
            }
            // not LSP: the diagnostics of an open document as a request, for a host that does not take notifications
            "btw/diagnostics" => {
                let a = self.doc_of(params)?;
                Ok(crate::diagnostics::all(a, self.encoding))
            }
            // not LSP: the number, note or numeral at a position, its range, step and unit, for an editor's scrubber
            // (number.rs). `null` where there is none.
            "btw/numberAt" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                let names = self.env_names();
                Ok(match crate::number::number_at(a, at, names.registry) {
                    Some(n) => crate::number::to_json(a, &n, self.encoding),
                    None => JsonValue::Null,
                })
            }
            "textDocument/documentSymbol" => {
                let a = self.doc_of(params)?;
                Ok(crate::nav::document_symbols(a, self.encoding))
            }
            "textDocument/foldingRange" => {
                let a = self.doc_of(params)?;
                Ok(crate::nav::folding_ranges(a))
            }
            "textDocument/definition" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                let uri = get_str(params, &["textDocument", "uri"]).unwrap_or("");
                Ok(crate::nav::definition(a, uri, at, self.encoding))
            }
            "textDocument/references" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                let uri = get_str(params, &["textDocument", "uri"]).unwrap_or("");
                let decl = params.get("context").and_then(|c| c.get("includeDeclaration")).and_then(JsonValue::as_bool).unwrap_or(true);
                Ok(crate::nav::references(a, uri, at, decl, self.encoding))
            }
            "textDocument/documentHighlight" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                Ok(crate::nav::highlight(a, at, self.encoding))
            }
            "textDocument/prepareRename" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                crate::nav::prepare_rename(a, at, self.encoding).map_err(|m| (REQUEST_FAILED, m))
            }
            "textDocument/rename" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                let uri = get_str(params, &["textDocument", "uri"]).unwrap_or("");
                let new = params.get("newName").and_then(JsonValue::as_str).ok_or_else(|| invalid("newName"))?;
                crate::nav::rename(a, uri, at, new, self.encoding).map_err(|m| (REQUEST_FAILED, m))
            }
            "textDocument/completion" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                Ok(crate::complete::completion(a, at, &self.env_names(), self.encoding))
            }
            "textDocument/hover" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                Ok(crate::hover::hover(a, at, &self.env_names(), self.encoding))
            }
            "textDocument/signatureHelp" => {
                let a = self.doc_of(params)?;
                let at = self.offset_of(a, params)?;
                Ok(crate::hover::signature_help(a, at, &self.env_names()))
            }
            "textDocument/inlayHint" => {
                let a = self.doc_of(params)?;
                let range = match params.get("range").and_then(get_range) {
                    Some(r) => a.index.span(&a.text, r, self.encoding),
                    None => djlive::Span::new(0, a.text.len()),
                };
                Ok(crate::inlay::inlay_hints(a, range, self.encoding))
            }
            "codeAction/resolve" => {
                // a quick-fix comes with its edit; a refactor is made and proved here
                if params.get("edit").is_some() || params.get("data").and_then(|d| d.get("refactor")).is_none() {
                    return Ok(params.clone());
                }
                let uri = params.get("data").and_then(|d| d.get("uri")).and_then(JsonValue::as_str).unwrap_or("");
                let a = self.docs.get(uri).map(|d| &d.analysis).ok_or_else(|| (REQUEST_FAILED, format!("{uri} is not open")))?;
                crate::refactor::resolve(a, params, self.encoding).map_err(|m| (REQUEST_FAILED, m))
            }
            "textDocument/codeAction" => {
                let a = self.doc_of(params)?;
                let uri = get_str(params, &["textDocument", "uri"]).unwrap_or("");
                let r = params.get("range").and_then(get_range).ok_or_else(|| invalid("range"))?;
                let range = a.index.span(&a.text, r, self.encoding);
                let only: Vec<String> = params.get("context").and_then(|c| c.get("only")).and_then(JsonValue::strings).unwrap_or_default();
                let mut actions = crate::fix::code_actions(a, uri, range, &only, self.encoding);
                actions.extend(self.refactors(a, uri, range, &only));
                Ok(arr(actions))
            }
            _ => Err((METHOD_NOT_FOUND, format!("unhandled method {method}"))),
        }
    }

    fn initialize(&mut self, params: &JsonValue) -> JsonValue {
        // utf-8 when the client offers it (djlive's own offsets), else the LSP default
        let offered: Vec<String> = params.get("capabilities").and_then(|c| c.get("general")).and_then(|g| g.get("positionEncodings")).and_then(JsonValue::strings).unwrap_or_default();
        self.encoding = if offered.iter().any(|e| e == "utf-8") { Encoding::Utf8 } else { Encoding::Utf16 };
        self.initialized = true;
        obj([("capabilities", self.capabilities()), ("serverInfo", obj([("name", s("btwls")), ("version", s(env!("CARGO_PKG_VERSION")))]))])
    }

    /// What this server can do.
    pub fn capabilities(&self) -> JsonValue {
        let mut c = JsonObject::new();
        c.set("positionEncoding", s(self.encoding.name()));
        // incremental: the client sends ranges
        c.set("textDocumentSync", obj([("openClose", JsonValue::Bool(true)), ("change", JsonValue::int(2))]));
        c.set("completionProvider", obj([("triggerCharacters", arr(["=", ":", "$", ".", "|", "("].iter().map(|t| s(*t)).collect())), ("resolveProvider", JsonValue::Bool(false))]));
        c.set("hoverProvider", JsonValue::Bool(true));
        c.set("signatureHelpProvider", obj([("triggerCharacters", arr(["(", ",", " "].iter().map(|t| s(*t)).collect()))]));
        c.set("inlayHintProvider", JsonValue::Bool(true));
        c.set("documentSymbolProvider", JsonValue::Bool(true));
        c.set("foldingRangeProvider", JsonValue::Bool(true));
        c.set("definitionProvider", JsonValue::Bool(true));
        c.set("referencesProvider", JsonValue::Bool(true));
        c.set("documentHighlightProvider", JsonValue::Bool(true));
        c.set("renameProvider", obj([("prepareProvider", JsonValue::Bool(true))]));
        // the requests beyond LSP this server answers
        c.set("experimental", obj([("btw", obj([("numberAt", JsonValue::Bool(true)), ("diagnostics", JsonValue::Bool(true))]))]));
        c.set("codeActionProvider", obj([("codeActionKinds", arr(Self::ACTION_KINDS.iter().map(|k| s(*k)).collect())), ("resolveProvider", JsonValue::Bool(true))]));
        JsonValue::Object(c)
    }

    /// The environment's names for completion and hover.
    pub fn env_names(&self) -> crate::complete::EnvNames<'static> {
        match self.registry {
            Some(r) => crate::complete::EnvNames::with_registry(r),
            None => crate::complete::EnvNames::web(),
        }
    }

    /// The kinds of code action this server offers.
    pub const ACTION_KINDS: &'static [&'static str] = &["quickfix", "source.fixAll.btw", "refactor.extract", "refactor.inline", "refactor.rewrite"];

    /// The refactors offered for a range: unresolved; `codeAction/resolve` makes and proves them.
    fn refactors(&self, a: &Analysis, uri: &str, range: djlive::Span, only: &[String]) -> Vec<JsonValue> {
        crate::refactor::actions(a, uri, range, only)
    }

    pub fn is_shut_down(&self) -> bool {
        self.shutdown
    }
}
