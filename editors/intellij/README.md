# BeatScript (btw) in IntelliJ IDEA and the other JetBrains IDEs

The language server is `btwd lsp` (`crates/beatscriptls`); JetBrains IDEs reach it through **LSP4IJ** (Red Hat's free LSP client
plugin, Community and Ultimate editions alike). No plugin of our own is needed.

## Set it up (once)

1. Build the server: `cargo build --release -p btwd` in stackdump/beatscript (gives `target/release/btwd`).
2. Install LSP4IJ: Settings → Plugins → Marketplace → "LSP4IJ".
3. Settings → Languages & Frameworks → Language Servers → `+`:
   - **Server**: name `BeatScript`, command `/path/to/btwd lsp`.
   - **Mappings** → *File name patterns*: `*.btw`, language id `btw`.
   - **Configuration**: nothing needed.
4. Colours: Settings → Editor → TextMate Bundles → `+` → choose `editors/vscode` of this repo (IntelliJ reads the VS Code
   grammar as a TextMate bundle). Without it LSP4IJ still gives everything but the colours.

Open a `.btw` file: diagnostics, quick-fixes (Alt+Enter), the structure view (outline), go to declaration (Ctrl+B), find usages
(Alt+F7), rename (Shift+F6), completion (Ctrl+Space), quick documentation (Ctrl+Q, the hover), parameter info (Ctrl+P),
inlay hints and the refactors (in the Alt+Enter list) come from the server.

## References

A program's references (`gen@2 techno seed=42 section=drop part=bass`, `seed(techno, 42)`, `<cid>.drop.bass`, `template(...)`):

- **Quick documentation** (Ctrl+Q, or the mouse) on one says what it is (an engine call or a structural import), the CID it
  resolves to, the generator and what it was fitted to, and its expansion (cut short for a long one).
- **Alt+Enter** on one offers *Eject … into local text* (made and proved by the server: the program must play the same) and
  *Copy the expansion of …*.
- **Code lenses** over each: `eject` does the same eject (the server sends the edit as `workspace/applyEdit`); `show expansion`
  has no inline view in LSP4IJ, so the server answers it with a notification holding the call, its CID and the first lines
  of the expansion. Copy reaches the server the same way, and is a notification here too: LSP4IJ has no clipboard command of
  a server's. (VS Code and the web page keep the show/hide state themselves.)

## What LSP4IJ needs from the server

Only stdio and the standard requests; beatscriptls answers `initialize` with its capabilities (incremental sync, UTF-16 positions
unless the client offers UTF-8). If something does not show, LSP4IJ's *Language Servers* tool window has a trace of the
traffic (set the server's *Debug → Trace* to *verbose*).
