# BeatScript for VS Code

Highlighting from a TextMate grammar; everything else from `beatscript lsp`, the BeatScript language server (`crates/beatscriptls` in
stackdump/beatscript). The browser editor at beatscript.app runs the same server as wasm, so the two say the same things.

What the server gives: diagnostics (the parser's at once, the compile's after the text has rested 150 ms), quick-fixes and fix
all, the outline, folding, go to definition, find references, highlight, rename (a section renamed in `arrange` too; `a`,
`kick`, `stack` refused as names), completion (mid-line, from the registry, the sounds and the program's own names), hover
(a section's bars and seconds at the tempo, a let's value, generator signatures, param docs), signature help, inlay hints
(start bars, song length, evolve generations, steps), and extract / inline let (proved by the nets they compile to).

References (`gen@2 techno seed=42 section=drop part=bass`, `seed(techno, 42)`, `<cid>.drop.bass`, `template(...)`): hover one
for what it is, the CID it resolves to, the generator and what it was fitted to, and its expansion. Two code lenses sit over
each: **show expansion** (again: **hide expansion**) puts its first line after it, the whole of it in that decoration's hover,
and **eject** writes it out in place under a `# from:` line once the server has proved the program plays the same. The same
eject, and *Copy the expansion*, are in the lightbulb (Ctrl+.). The palette has *Show or hide every reference's expansion* and
*Open the expansion of the reference here beside* (a read-only BeatScript document). Which expansions are shown is this
extension's own state; the server only answers `beatscript/expand`.

## Try it

```bash
# 1. the server
cd beatscript && cargo build --release -p beatscript-cli     # target/release/beatscript
# 2. the extension, from this directory
npm install
code --extensionDevelopmentPath="$PWD" ../../examples/beatscript/showcase.beatscript
```

Set `beatscript.server.path` to the `beatscript` binary if it is not on `PATH` (Settings → BeatScript). `BeatScript: Restart the language
server` picks up a rebuilt `beatscript`. `beatscript.trace.server: verbose` logs the traffic.

`.beatscript` files and `.btw` files (the extension before the rename) are both BeatScript. The settings were called
`btw.server.path`, `btw.server.args` and `btw.trace.server` before the rename: they are still read when the `beatscript.`
setting is not set, and VS Code marks them deprecated. *Format Document* formats and upgrades a BeatScript 1 text (the old
spelling: `#! btw 1`, or no version line) to BeatScript 2 (`#! beatscript 2` first: the same program under a new CID);
the lightbulb on the first line of such a text offers *Upgrade to BeatScript 2*, which writes the version line alone.

To install it for good, package it: `npx @vscode/vsce package --allow-missing-repository --skip-license` makes
`beatscript-0.1.0.vsix`; then `code --install-extension beatscript-0.1.0.vsix`.

`npm test` checks the grammar's patterns, and the manifest and the settings (the old names read as a fallback).
