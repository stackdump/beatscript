# BeatScript (btw) in IntelliJ IDEA and the other JetBrains IDEs

The language server is `btwd lsp` (`crates/btwls`); JetBrains IDEs reach it through **LSP4IJ** (Red Hat's free LSP client
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

## What LSP4IJ needs from the server

Only stdio and the standard requests; btwls answers `initialize` with its capabilities (incremental sync, UTF-16 positions
unless the client offers UTF-8). If something does not show, LSP4IJ's *Language Servers* tool window has a trace of the
traffic (set the server's *Debug → Trace* to *verbose*).
