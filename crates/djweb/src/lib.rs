//! djweb: the one `.wasm` the page loads (btwsite/DESIGN.md, "crates/djweb").
//!
//! It is two things in one module, so the language and the sound can never be from different builds:
//!
//! - **the language**: thin `wasm-bindgen` wrappers over `btwapi` (`btw_check`, `btw_fmt`, `btw_open`, ... and the
//!   [`BtwRun`] class, the Live runtime behind a command queue). Strings in, JSON strings out. All the logic is
//!   in `btwapi`, which the `btwd` binary shares, so the browser and the server answer the same bytes.
//! - **the engine**: djrust's public browser facade, re-exported (`Dj`, `DjComposer`, `render_offline`, ...), which
//!   runs in an AudioWorklet (`Dj`) and a Worker (`DjComposer`).
//!
//! Native builds compile this crate too (the workspace tests do), but only the wasm32 target can *call* the
//! exports: `cargo test -p djweb` checks what does not need a JS host.

use wasm_bindgen::prelude::*;

// The engine: Dj, DjComposer, engine_version, render_offline, compose_project, share_cid, ...
pub use djrust::wasm::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(s: &str);
}

/// Panics become a console error naming the place (the engine's own `Dj::new` silences the hook in the
/// worklet's instance; this is the page's instance, where a message helps).
fn init() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| console_error(&format!("djweb panic: {info}"))));
    });
}

// ---- the language ----

/// `check(text)` -> CheckReport JSON (docs: btwsite/DESIGN.md "JSON shapes").
#[wasm_bindgen]
pub fn btw_check(text: &str) -> String {
    init();
    btwapi::check_json(text)
}

/// `fmt(text)` -> `{"ok","canonical","cid","comments","diagnostics"}`.
#[wasm_bindgen]
pub fn btw_fmt(text: &str) -> String {
    init();
    btwapi::fmt_json(text)
}

/// `expand(text, lineage)`: the program with its `seed(...)` references written out as plain BeatScript (btwapi).
#[wasm_bindgen]
pub fn btw_expand(text: &str, lineage: bool) -> String {
    init();
    btwapi::expand_json(text, lineage)
}

/// `generate(json)`: the seeded generator, no model. `{"genre","bpm","key","length","seed"}` (all optional) ->
/// `{"ok","text","cid","name","generator","seed",...,"check"}` or `{"ok":false,"error"}`. Same inputs, same text and CID.
#[wasm_bindgen]
pub fn btw_generate(request_json: &str) -> String {
    init();
    btwapi::generate_json(request_json)
}

/// Tell the module a program by CID and canonical text (what `/api/programs/<cid>` answers, or a link carried): an `import`
/// of that CID then checks, plays and shares as it does where the program is saved. The text is re-hashed; one that is not
/// the program its CID names is refused (`{"ok":false,"error"}`).
#[wasm_bindgen]
pub fn btw_know(cid: &str, text: &str) -> String {
    init();
    btwapi::known::know_json(cid, text)
}

/// The voices, families, kits and palettes a part may name.
#[wasm_bindgen]
pub fn btw_instruments() -> String {
    init();
    btwapi::instruments_json()
}

/// A link, fragment (`#<CID>.<packed>`) or bare CID decoded and verified -> OpenReport JSON.
#[wasm_bindgen]
pub fn btw_open(url_or_cid_or_fragment: &str) -> String {
    init();
    btwapi::open_json(url_or_cid_or_fragment)
}

/// The self-sealing link for a program. `sig`: the author's signature over the CID, carried as given.
/// `notes`: carry the comments (unsealed, `~notes`). `origin`: the site's origin (default `https://btw.bitwrap.io`).
#[wasm_bindgen]
pub fn btw_share(text: &str, sig: Option<String>, notes: bool, origin: Option<String>) -> String {
    init();
    btwapi::share_json(text, sig.as_deref().filter(|s| !s.is_empty()), notes, origin.as_deref().filter(|s| !s.is_empty()).unwrap_or(btwapi::DEFAULT_ORIGIN))
}

/// The CID of generation `n` of an `evolve` (`scope` empty: the first one's) -> JSON.
#[wasm_bindgen]
pub fn btw_generation(text: &str, scope: Option<String>, n: u32) -> String {
    init();
    btwapi::generation_json(text, scope.as_deref().filter(|s| !s.is_empty()), n)
}

/// Highlight spans as JSON `[[start16,end16,"class"],...]`, UTF-16 offsets so `text.slice(a, b)` works.
#[wasm_bindgen]
pub fn btw_highlight(text: &str) -> String {
    init();
    btwapi::highlight_json_utf16(text)
}

/// The compact grammar, generated from docs/BTW.md (markdown).
#[wasm_bindgen]
pub fn btw_cheatsheet() -> String {
    init();
    btwapi::cheatsheet()
}

/// Versions of what is in this module: `{"djweb","engine"}`.
#[wasm_bindgen]
pub fn btw_build_info() -> String {
    format!("{{\"djweb\":\"{}\",\"engine\":\"{}\"}}", env!("CARGO_PKG_VERSION"), djrust::wasm::engine_version())
}

// ---- the runtime ----

/// A program running on a clock the page keeps: the Live runtime with a host that is a command queue.
///
/// The page calls `eval` when the person evaluates, then `service(grid)` about every 25 ms with where the
/// sound's clock is (`engine.grid()`), applies the commands it returns (`engine.apply`), and reads `marks()`
/// to light the text. `stop()` returns the commands that put back what lines held. See
/// `btwapi::run` for the shapes.
#[wasm_bindgen]
pub struct BtwRun {
    inner: btwapi::run::Run,
}

impl Default for BtwRun {
    fn default() -> Self {
        BtwRun::new()
    }
}

#[wasm_bindgen]
impl BtwRun {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BtwRun {
        init();
        BtwRun { inner: btwapi::run::Run::new() }
    }

    /// Evaluate: the app's eval answer as JSON (`ok`, `diagnostics`, `cid`, `nets`, `changes`, `lands`, `lineage`, `kept`...).
    /// A program with errors changes nothing.
    pub fn eval(&mut self, text: &str) -> String {
        self.inner.eval_str(text)
    }

    /// `eval`'s answer without staging the program.
    pub fn check(&self, text: &str) -> String {
        self.inner.check(text).js_stringified()
    }

    /// End the program: the commands (JSON array) that put back what its lines held. The decks play on.
    pub fn stop(&mut self) -> String {
        self.inner.stop_str()
    }

    /// One pass. `grid_json` is `{"deck":"A","beat":12.5,"bpm":140,"playing":{"A":true,"B":false}}`
    /// or `null` (no clock). Returns the commands as a JSON array.
    pub fn service(&mut self, grid_json: &str) -> Result<String, JsError> {
        self.inner.service_str(grid_json).map_err(|e| JsError::new(&e))
    }

    /// What to light: active lines, header playhead, fires, controls, generation (`Live::marks().to_json()`),
    /// every span with `start`/`end` (UTF-8) and `start16`/`end16` (UTF-16, into the evaluated text).
    pub fn marks(&self) -> String {
        self.inner.marks_str()
    }

    /// `Live::status()`: section, bar, ended, staged, the last words.
    pub fn status(&self) -> String {
        self.inner.status_str()
    }

    /// The CID of the program that is running, if one is.
    pub fn running_cid(&self) -> Option<String> {
        self.inner.running_cid()
    }
}


// ---- the language service ----

/// The language service for the editor (crates/btwls): the same server `btwd lsp` runs, with positions in UTF-16 (JS string
/// offsets) and no `initialize` needed. Documents are named by any string (`"main"`).
///
/// - `open(uri, text)` / `change(uri, text)` -> the parser's diagnostics (JSON array), at once: every keystroke.
/// - `check(uri)` -> the check's diagnostics (the compile: a few ms, seconds on a long evolving song), for the page to call
///   once the text has rested (and from a Worker for long programs).
/// - `request(method, params)` -> the LSP result as JSON (`textDocument/completion`, `textDocument/hover`, ...), or
///   `{"error":{code,message}}`.
/// - `handle(message)` -> every message to send back (JSON array): a whole JSON-RPC message in, as over stdio.
#[wasm_bindgen]
pub struct BtwLs {
    inner: btwls::Server,
}

impl Default for BtwLs {
    fn default() -> Self {
        BtwLs::new()
    }
}

#[wasm_bindgen]
impl BtwLs {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BtwLs {
        init();
        let mut inner = btwls::Server::with_encoding(btwls::Encoding::Utf16);
        inner.deferred = true;
        BtwLs { inner }
    }

    /// Open or replace a document: its diagnostics as JSON.
    pub fn open(&mut self, uri: &str, text: &str) -> String {
        btwls::wasm::open(&mut self.inner, uri, text)
    }

    /// The document's whole new text: its diagnostics as JSON.
    pub fn change(&mut self, uri: &str, text: &str) -> String {
        btwls::wasm::open(&mut self.inner, uri, text)
    }

    /// The check of an open document (computed now if it is not done): its diagnostics as JSON.
    pub fn check(&mut self, uri: &str) -> String {
        btwls::wasm::check(&mut self.inner, uri)
    }

    pub fn close(&mut self, uri: &str) {
        self.inner.docs.remove(uri);
    }

    /// One LSP request; `params_json` is its params (with `textDocument.uri` naming an open document).
    pub fn request(&mut self, method: &str, params_json: &str) -> String {
        btwls::wasm::request(&mut self.inner, method, params_json)
    }

    /// One JSON-RPC message in; every message out, as a JSON array.
    pub fn handle(&mut self, message_json: &str) -> String {
        btwls::wasm::handle(&mut self.inner, message_json)
    }
}

#[cfg(test)]
mod tests {
    // The wasm-bindgen exports cannot be called off wasm32 (JsError, console); what they wrap is tested in
    // btwapi, and the built module by scripts/test-web-wasm.mjs. This only keeps the pieces that need no JS host honest.
    #[test]
    fn the_runtime_wrapper_is_the_btwapi_one() {
        let mut r = btwapi::run::Run::new();
        let ev = r.eval_str("deck A: kick x ~ x ~\n");
        assert!(ev.contains("\"ok\":true"), "{ev}");
        // nothing plays, so the program starts at once: its nets, then the deck
        let first = r.service_str("null").unwrap();
        assert!(first.starts_with("[{\"op\":\"load_net\",\"deck\":\"A\"") && first.contains("{\"op\":\"play\",\"deck\":\"A\",\"on\":true}"), "{first}");
        assert_eq!(r.service_str("null").unwrap(), "[]");
    }

    #[test]
    fn the_language_service_wrapper_answers_json() {
        let mut ls = btwls::Server::with_encoding(btwls::Encoding::Utf16);
        ls.deferred = true;
        let d = btwls::wasm::open(&mut ls, "main", "tempo 12x\n");
        assert!(d.starts_with('[') && d.contains("\"severity\":1"), "{d}");
        // an unknown voice: only the check says so
        assert_eq!(btwls::wasm::open(&mut ls, "main", "deck A: bass a1 inst=nopenope\n"), "[]");
        assert!(btwls::wasm::check(&mut ls, "main").contains("nopenope"));
        let bad = btwls::wasm::request(&mut ls, "nope", "{}");
        assert!(bad.contains("\"error\""), "{bad}");
        let out = btwls::wasm::handle(&mut ls, r#"{"jsonrpc":"2.0","id":7,"method":"shutdown"}"#);
        assert_eq!(out, r#"[{"jsonrpc":"2.0","id":7,"result":null}]"#);
    }

    /// What the page asks of a reference (docs/btwls.md "The web editor"): `btw/expand` and the copy command, in UTF-16.
    #[test]
    fn the_language_service_answers_a_references_expansion() {
        let mut ls = btwls::Server::with_encoding(btwls::Encoding::Utf16);
        ls.deferred = true;
        let text = "# é\ntempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: bass bed\n}\narrange groove\n";
        btwls::wasm::open(&mut ls, "main", text);
        let all = btwls::wasm::request(&mut ls, "btw/expand", r#"{"textDocument":{"uri":"main"}}"#);
        assert!(all.contains(r#""kind":"gen""#) && all.contains(r#""range":{"start":{"line":3,"character":10}"#) && all.contains("let bed_1 = "), "{all}");
        let copy = btwls::wasm::request(&mut ls, "workspace/executeCommand", r#"{"command":"btw.expand.copy","arguments":[{"uri":"main","position":{"line":3,"character":12}}]}"#);
        assert!(copy.contains(r##""text":"# from: gen@2 techno seed=42 section=drop part=bass -> z"##), "{copy}");
    }

    #[test]
    fn share_default_origin_is_the_site() {
        assert_eq!(btwapi::DEFAULT_ORIGIN, "https://btw.bitwrap.io");
        let j = btwapi::share_json("deck A: kick x ~ x ~\n", None, false, btwapi::DEFAULT_ORIGIN);
        assert!(j.contains("https://btw.bitwrap.io/#"), "{j}");
    }
}
