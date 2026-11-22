//! beatscriptls: the btw language service. Everything an editor asks of a language (diagnostics, quick-fixes, the outline, go to
//! definition, references, rename, completion, hover, signature help, inlay hints, refactors, and a program's references to a
//! generator, a seed, a stored program or a template shown, hidden and ejected: [`refs`]) as pure functions of the text,
//! shaped like LSP so that one [`Server`] answers both `btwd lsp` (stdio, any LSP client: VS Code, IntelliJ via LSP4IJ,
//! Neovim, Helix) and the browser editor (djweb's `BeatscriptLs`, the same JSON through wasm).
//!
//! Pure: no I/O, no clock, no threads. A document is checked against `btwapi::web_env` (the registry and catalog
//! snapshot the site checks against), so the editor, `btwd check` and the page cannot disagree.
//!
//! Positions are LSP positions (`{line, character}`, 0-based); `character` counts UTF-16 units unless the client asked for
//! `utf-8` in `initialize` ([`text::Encoding`]).

pub mod analysis;
pub mod complete;
pub mod symbols;
pub mod diagnostics;
pub mod fix;
pub mod hover;
pub mod inlay;
pub mod json;
pub mod lang;
pub mod nav;
pub mod number;
pub mod refactor;
pub mod refs;
pub mod server;
pub mod text;
pub mod timeline;

pub use analysis::Analysis;
pub use server::Server;
pub use text::{Encoding, LineIndex, Position, Range};

/// The string-in, string-out surface djweb's `BeatscriptLs` wraps (kept here so it is tested natively).
pub mod wasm {
    use djengine::json::JsonValue;

    use crate::json::obj;
    use crate::Server;

    pub fn open(s: &mut Server, uri: &str, text: &str) -> String {
        s.open(uri, text, 0);
        crate::diagnostics::all(&s.docs[uri].analysis, s.encoding).js_stringified()
    }

    /// The check's diagnostics of an open document (`[]` for one that is not open).
    pub fn check(s: &mut Server, uri: &str) -> String {
        match s.docs.get(uri) {
            Some(d) => {
                d.analysis.check();
                crate::diagnostics::all(&d.analysis, s.encoding).js_stringified()
            }
            None => "[]".to_string(),
        }
    }

    pub fn request(s: &mut Server, method: &str, params_json: &str) -> String {
        let params = match JsonValue::parse(params_json) {
            Ok(p) => p,
            Err(e) => return obj([("error", obj([("code", JsonValue::int(-32700)), ("message", JsonValue::str(format!("parse error: {e}")))]))]).js_stringified(),
        };
        match s.request(method, &params) {
            Ok(v) => v.js_stringified(),
            Err((code, message)) => obj([("error", obj([("code", JsonValue::int(code)), ("message", JsonValue::str(message))]))]).js_stringified(),
        }
    }

    pub fn handle(s: &mut Server, message_json: &str) -> String {
        match JsonValue::parse(message_json) {
            Ok(m) => JsonValue::Array(s.handle(&m)).js_stringified(),
            Err(e) => JsonValue::Array(vec![obj([("jsonrpc", JsonValue::str("2.0")), ("id", JsonValue::Null), ("error", obj([("code", JsonValue::int(-32700)), ("message", JsonValue::str(format!("parse error: {e}")))]))])]).js_stringified(),
        }
    }
}
