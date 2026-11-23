//! djweb: the one `.wasm` the page loads (site/DESIGN.md, "crates/djweb").
//!
//! It is two things in one module, so the language and the sound can never be from different builds:
//!
//! - **the language**: thin `wasm-bindgen` wrappers over `beatscript-api` (`beatscript_check`, `beatscript_fmt`, `beatscript_open`, ... and the
//!   [`BeatscriptRun`] class, the Live runtime behind a command queue). Strings in, JSON strings out. All the logic is
//!   in `beatscript-api`, which the `beatscript` binary shares, so the browser and the server answer the same bytes.
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

/// `check(text)` -> CheckReport JSON (docs: site/DESIGN.md "JSON shapes"), with the language `version` and `renamed_cid`.
#[wasm_bindgen]
pub fn beatscript_check(text: &str) -> String {
    init();
    beatscript_api::check_json(text)
}

/// `fmt(text)` -> `{"ok","canonical","cid","version","comments","diagnostics","text"}` (and `from_cid`): the program formatted
/// and in BeatScript 2, a BeatScript 1 text upgraded (the editor's fmt button).
#[wasm_bindgen]
pub fn beatscript_fmt(text: &str) -> String {
    init();
    beatscript_api::fmt_json(text)
}

/// `expand(text, lineage)`: the program with its `seed(...)` references written out as plain BeatScript (beatscript-api).
#[wasm_bindgen]
pub fn beatscript_expand(text: &str, lineage: bool) -> String {
    init();
    beatscript_api::expand_json(text, lineage)
}

/// `generate(json)`: the seeded generator, no model. `{"genre","bpm","key","length","seed"}` (all optional) ->
/// `{"ok","text","cid","name","generator","seed",...,"check"}` or `{"ok":false,"error"}`. Same inputs, same text and CID.
#[wasm_bindgen]
pub fn beatscript_generate(request_json: &str) -> String {
    init();
    beatscript_api::generate_json(request_json)
}

/// `station(json)`: radio (docs/beatscript/radio.md). A station request, `{"station": "<program>", "n", "count", "segue", "check",
/// "retune", "locks", "neighbours"}` -> `{"ok","version","station","n","tracks","segues",...}` or `{"ok":false,"error",
/// "diagnostics"}`: the answer the MCP `station` tool and `POST /api/station` give (beatscript_api::station_json).
#[wasm_bindgen]
pub fn beatscript_station(request_json: &str) -> String {
    init();
    beatscript_api::station_json(request_json)
}

/// Tell the module a program by CID and canonical text (what `/api/programs/<cid>` answers, or a link carried): an `import`
/// of that CID then checks, plays and shares as it does where the program is saved. The text is re-hashed; one that is not
/// the program its CID names is refused (`{"ok":false,"error"}`).
#[wasm_bindgen]
pub fn beatscript_know(cid: &str, text: &str) -> String {
    init();
    beatscript_api::known::know_json(cid, text)
}

/// The voices, families, kits and palettes a part may name.
#[wasm_bindgen]
pub fn beatscript_instruments() -> String {
    init();
    beatscript_api::instruments_json()
}

/// A link, fragment (`#<CID>.<packed>`) or bare CID decoded and verified -> OpenReport JSON.
#[wasm_bindgen]
pub fn beatscript_open(url_or_cid_or_fragment: &str) -> String {
    init();
    beatscript_api::open_json(url_or_cid_or_fragment)
}

/// The self-sealing link for a program. `sig`: the author's signature over the CID, carried as given.
/// The link packs the text formatted (`#!` headers and comments kept,
/// unsealed: the CID is the canonical text's). `notes`: carry the comments in beats' link too (`~notes`). `origin`: the site's
/// origin (default `https://beatscript.app`).
#[wasm_bindgen]
pub fn beatscript_share(text: &str, sig: Option<String>, notes: bool, origin: Option<String>) -> String {
    init();
    beatscript_api::share_json(text, sig.as_deref().filter(|s| !s.is_empty()), notes, origin.as_deref().filter(|s| !s.is_empty()).unwrap_or(beatscript_api::DEFAULT_ORIGIN))
}

/// The CID of generation `n` of an `evolve` (`scope` empty: the first one's) -> JSON.
#[wasm_bindgen]
pub fn beatscript_generation(text: &str, scope: Option<String>, n: u32) -> String {
    init();
    beatscript_api::generation_json(text, scope.as_deref().filter(|s| !s.is_empty()), n)
}

/// Highlight spans as JSON `[[start16,end16,"class"],...]`, UTF-16 offsets so `text.slice(a, b)` works.
#[wasm_bindgen]
pub fn beatscript_highlight(text: &str) -> String {
    init();
    beatscript_api::highlight_json_utf16(text)
}

/// The compact grammar, generated from docs/BEATSCRIPT.md (markdown).
#[wasm_bindgen]
pub fn beatscript_cheatsheet() -> String {
    init();
    beatscript_api::cheatsheet()
}

/// Versions of what is in this module: `{"djweb","engine"}`.
#[wasm_bindgen]
pub fn beatscript_build_info() -> String {
    format!("{{\"djweb\":\"{}\",\"engine\":\"{}\"}}", env!("CARGO_PKG_VERSION"), djrust::wasm::engine_version())
}

// ---- the runtime ----

/// A program running on a clock the page keeps: the Live runtime with a host that is a command queue.
///
/// The page calls `eval` when the person evaluates, then `service(grid)` about every 25 ms with where the
/// sound's clock is (`engine.grid()`), applies the commands it returns (`engine.apply`), and reads `marks()`
/// to light the text. `stop()` returns the commands that put back what lines held. See
/// `beatscript_api::run` for the shapes.
#[wasm_bindgen]
pub struct BeatscriptRun {
    inner: beatscript_api::run::Run,
}

impl Default for BeatscriptRun {
    fn default() -> Self {
        BeatscriptRun::new()
    }
}

#[wasm_bindgen]
impl BeatscriptRun {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BeatscriptRun {
        init();
        BeatscriptRun { inner: beatscript_api::run::Run::new() }
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

/// The language service for the editor (crates/beatscriptls): the same server `beatscript lsp` runs, with positions in UTF-16 (JS string
/// offsets) and no `initialize` needed. Documents are named by any string (`"main"`).
///
/// - `open(uri, text)` / `change(uri, text)` -> the parser's diagnostics (JSON array), at once: every keystroke.
/// - `check(uri)` -> the check's diagnostics (the compile: a few ms, seconds on a long evolving song), for the page to call
///   once the text has rested (and from a Worker for long programs).
/// - `request(method, params)` -> the LSP result as JSON (`textDocument/completion`, `textDocument/hover`, ...), or
///   `{"error":{code,message}}`.
/// - `handle(message)` -> every message to send back (JSON array): a whole JSON-RPC message in, as over stdio.
#[wasm_bindgen]
pub struct BeatscriptLs {
    inner: beatscriptls::Server,
}

impl Default for BeatscriptLs {
    fn default() -> Self {
        BeatscriptLs::new()
    }
}

#[wasm_bindgen]
impl BeatscriptLs {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BeatscriptLs {
        init();
        let mut inner = beatscriptls::Server::with_encoding(beatscriptls::Encoding::Utf16);
        inner.deferred = true;
        BeatscriptLs { inner }
    }

    /// Open or replace a document: its diagnostics as JSON.
    pub fn open(&mut self, uri: &str, text: &str) -> String {
        beatscriptls::wasm::open(&mut self.inner, uri, text)
    }

    /// The document's whole new text: its diagnostics as JSON.
    pub fn change(&mut self, uri: &str, text: &str) -> String {
        beatscriptls::wasm::open(&mut self.inner, uri, text)
    }

    /// The check of an open document (computed now if it is not done): its diagnostics as JSON.
    pub fn check(&mut self, uri: &str) -> String {
        beatscriptls::wasm::check(&mut self.inner, uri)
    }

    pub fn close(&mut self, uri: &str) {
        self.inner.docs.remove(uri);
    }

    /// One LSP request; `params_json` is its params (with `textDocument.uri` naming an open document).
    pub fn request(&mut self, method: &str, params_json: &str) -> String {
        beatscriptls::wasm::request(&mut self.inner, method, params_json)
    }

    /// One JSON-RPC message in; every message out, as a JSON array.
    pub fn handle(&mut self, message_json: &str) -> String {
        beatscriptls::wasm::handle(&mut self.inner, message_json)
    }
}

#[cfg(test)]
mod tests {
    // The wasm-bindgen exports cannot be called off wasm32 (JsError, console); what they wrap is tested in
    // beatscript-api, and the built module by scripts/test-web-wasm.mjs. This only keeps the pieces that need no JS host honest.
    #[test]
    fn the_runtime_wrapper_is_the_beatscript_api_one() {
        let mut r = beatscript_api::run::Run::new();
        let ev = r.eval_str("deck A: kick x ~ x ~\n");
        assert!(ev.contains("\"ok\":true"), "{ev}");
        // nothing plays, so the program starts at once: its nets, then the deck
        let first = r.service_str("null").unwrap();
        assert!(first.starts_with("[{\"op\":\"load_net\",\"deck\":\"A\"") && first.contains("{\"op\":\"play\",\"deck\":\"A\",\"on\":true}"), "{first}");
        assert_eq!(r.service_str("null").unwrap(), "[]");
    }

    #[test]
    fn the_language_service_wrapper_answers_json() {
        let mut ls = beatscriptls::Server::with_encoding(beatscriptls::Encoding::Utf16);
        ls.deferred = true;
        let d = beatscriptls::wasm::open(&mut ls, "main", "tempo 12x\n");
        assert!(d.starts_with('[') && d.contains("\"severity\":1"), "{d}");
        // an unknown voice: only the check says so
        assert_eq!(beatscriptls::wasm::open(&mut ls, "main", "deck A: bass a1 inst=nopenope\n"), "[]");
        assert!(beatscriptls::wasm::check(&mut ls, "main").contains("nopenope"));
        let bad = beatscriptls::wasm::request(&mut ls, "nope", "{}");
        assert!(bad.contains("\"error\""), "{bad}");
        let out = beatscriptls::wasm::handle(&mut ls, r#"{"jsonrpc":"2.0","id":7,"method":"shutdown"}"#);
        assert_eq!(out, r#"[{"jsonrpc":"2.0","id":7,"result":null}]"#);
    }

    /// What the page asks of a reference (docs/beatscriptls.md "The web editor"): `beatscript/expand` and the copy command, in UTF-16.
    #[test]
    fn the_language_service_answers_a_references_expansion() {
        let mut ls = beatscriptls::Server::with_encoding(beatscriptls::Encoding::Utf16);
        ls.deferred = true;
        let text = "# é\ntempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: bass bed\n}\narrange groove\n";
        beatscriptls::wasm::open(&mut ls, "main", text);
        let all = beatscriptls::wasm::request(&mut ls, "beatscript/expand", r#"{"textDocument":{"uri":"main"}}"#);
        assert!(all.contains(r#""kind":"gen""#) && all.contains(r#""range":{"start":{"line":3,"character":10}"#) && all.contains("let bed_1 = "), "{all}");
        let copy = beatscriptls::wasm::request(&mut ls, "workspace/executeCommand", r#"{"command":"beatscript.expand.copy","arguments":[{"uri":"main","position":{"line":3,"character":12}}]}"#);
        assert!(copy.contains(r##""text":"# from: gen@2 techno seed=42 section=drop part=bass -> z"##), "{copy}");
    }

    #[test]
    fn share_default_origin_is_the_site() {
        assert_eq!(beatscript_api::DEFAULT_ORIGIN, "https://beatscript.app");
        // what beatscript_share does with no origin given; the link opens to the text as written, comment and all
        let j = beatscript_api::share_json("# a comment\ndeck A: kick x ~ x ~\n", None, false, beatscript_api::DEFAULT_ORIGIN);
        assert!(j.contains("\"url\":\"https://beatscript.app/#z") && !j.contains("btw.bitwrap.io"), "{j}");
        let url = j.split("\"url\":\"").nth(1).and_then(|r| r.split('"').next()).unwrap();
        let o = beatscript_api::open_json(url);
        assert!(o.contains("\"text\":\"# a comment\\ndeck A: kick x ~ x ~\\n\""), "{o}");
    }
}
