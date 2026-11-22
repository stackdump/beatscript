# btwls: the btw language service

`crates/btwls` is the editor assistant for BeatScript (btw): diagnostics, quick-fixes, the outline, navigation, rename,
completion, hover, signature help, inlay hints and refactors. It is pure (no I/O, no clock), shaped like LSP, and served three
ways:

- `btwd lsp`: an LSP server on stdio, for VS Code (`editors/vscode`), IntelliJ (LSP4IJ, `editors/intellij`), Neovim, Helix.
- djweb's `BtwLs` (wasm): the same server for the browser editor, positions in UTF-16.
- `btwls::Server` in-process (the golden tests drive it).

## Two tiers

| tier | what | when |
|---|---|---|
| syntax | lex, parse (`parse_with`), the symbol table | every change; everything but the compile's diagnostics needs only this |
| check | `check_with` (expand, compile, evolve orbits) | off the keystroke path: `btwd lsp` runs it on a worker thread once the text has rested 150 ms; the page calls `BtwLs.check(uri)` |

Until the check of a version is in, the diagnostics shown are the parser's; a check of a version the document has moved past
is dropped.

## Latency (phase 0)

`cargo run --release -p btwls --example bench` on a shared Linux box (4 cores, shared with other builds; medians of 5 to 9 runs):

| program | size | syntax | check |
|---|---|---|---|
| showcase.btw | 11.1 KB, 253 lines | 2.4 ms | 9.8 ms |
| deep-house-60.btw (60 min, 9 evolves) | 6.5 KB, 192 lines | 1.1 ms | **3.1 to 5.1 s** |
| hand-dubstep-140-v4.btw | 4.3 KB | 0.8 ms | 4.8 ms |
| matrix-130.btw | 3.6 KB | 0.7 ms | 22.7 ms |
| trance-138.btw | 3.2 KB | 0.5 ms | 5.5 ms |
| synthetic: 60 min at 120 BPM, 8 sections arranged (1800 bars) | 2.4 KB | 0.5 ms | 31.7 ms |
| synthetic: 60 min, 113 distinct 16-bar sections | 25 KB, 569 lines | 4.7 ms | 33.7 ms |

An hour of sections is cheap to check (~30 ms). What is slow is `evolve`: with deep-house-60's evolves removed its check is
33 ms. Timed by stage, the orbit analysis (`evolve::analyse`) is 2 to 6 ms a block; nearly all of the rest went to the
generations' CIDs (`evolve::generation_cid`: each of the ~105 generations builds and hashes every deck's raw-net envelope,
a section long). `evolve::NetCids` now reuses the net CID of a deck no rule changed since the last generation (the drum
deck, whose only rule is `thin ... when gen % 7 == k`), which halves it: 6.5 s to 2.9 s on a loaded box, the same
105 lineage CIDs. What is left is the melodic deck's envelope, rebuilt every generation; a cheaper net CID (hashing the
parts without building the envelope) would change every generation CID, so it is a decision, not a quick win. The check
stays off the keystroke path.

## What it answers

| request | from |
|---|---|
| `publishDiagnostics` (parser at once, check after a rest) | djlive |
| `codeAction` quickfix + `source.fixAll.btw` | `fix.rs`: the fixer's repairs (adapter until feat/btw-fixer lands) |
| `codeAction` `refactor.extract` / `refactor.inline` + `codeAction/resolve` | `refactor.rs`: made and proved (same nets) on resolve |
| `documentSymbol`, `foldingRange` | `symbols.rs` (a scan of the text's own tokens) |
| `definition`, `references`, `documentHighlight`, `prepareRename`, `rename` | `nav.rs` |
| `completion` | `complete.rs` + `djlive::partial` (the statement up to the cursor) |
| `hover`, `signatureHelp` | `hover.rs`, `timeline.rs` |
| `inlayHint` | `inlay.rs` |
| `btw/diagnostics` (not LSP: the diagnostics as a request) | `diagnostics.rs` |
| `btw/numberAt` (not LSP: what an editor's scrubber needs) | `number.rs` |

`btw/numberAt` takes `{textDocument, position}` and answers `null` or the literal there: `range` (its span, a `-` sign
included), `text`, `value`, `kind` (`tempo`, `vel`, `oct`, `len`, `glide`, `decay`, `pitch-drop`, `ratchet`, `times`, `chance`, `hits`, `steps`, `rotation`,
`cutoff-lp`, `level`, `mix`, `bars`, `bar`, `beat`, `degree`, `note`, `chord`, `param`, `number`, ...), `label`, `min` and
`max` (`null`: unbounded), `step`, `decimals`, `unit`, `integer`, `scale` (`log` for a cutoff: move it by a ratio),
`exact` (false when the literal is one operand of arithmetic: the limits are of the whole value), `span` for an end of a
range `a..b` (`ordered`: lo <= hi) or a ramp `a -> b` (`op`, `role` start|end, both ends' ranges and values), and for a
note or a Roman numeral `choices` (the notes of the key in force, every semitone with no key; the numerals of the key's
scale) with the `index` of the one written. A `do` param takes its type and range from the registry's `ParamSpec`
(`minimum`/`maximum` of the schema). The servers advertise it as `capabilities.experimental.btw.numberAt`. Every limit
row is proved against the checker (`tests/number.rs`: both ends check, one step past each is refused), and a scrub of
every number of a program one step either way is shown to keep it checkable and canonical.

The arrangement syntax is covered: an `arrange { }` block (its sections and `~` / `@` mutations: one outline entry, the
sections and the mutations' let patterns are references, completion offers sections and the two marks at a line's start,
the let patterns after a mark or a `,`, the operators after a target or a `|`, and `end` / `loop` / `then` after the
block; hover says what an operator and a mark do; numberAt knows an operator's argument, -64 to 64 or 0 to 1, and
`loop N`, 1 to 256), and named arrangements (`arrange set: ...` is a name; `then set` and `start set` are references,
go to definition and rename work, completion offers the arrangements after `then` and `start`). The words are not a second
copy: the statement keywords and the endings are held to `crates/djlive/grammar.ebnf` (`lang::first_words`,
`lang::words_of`; `tests/arrange.rs` fails when the grammar starts a statement with a word `lang::STATEMENTS` has no doc
for, or the other way round), the operators are `djlive::mutate::operator_names` with their argument kinds from
`mutate::operator_args`, and their signatures and docs are read from docs/ARRANGE.md's tables (a test fails for an
operator with no row, or a row whose argument count is not djlive's). djlive's highlighter colours a mutation line's mark
and operators, and so does the VS Code grammar (`tests/editors.rs` holds its operator list to djlive's).

`refactor.rewrite` collapses a repeated arrangement, `arrange a b a b a b outro` to `arrange { a b } loop 3 then outro end`,
made and proved on resolve like the others (the same nets).

Not done: semantic tokens (the editors colour from the TextMate grammar, `editors/vscode`, checked against djlive's word lists by
`tests/editors.rs`).

## Try it

- **VS Code**: `cargo build --release -p btwd`, then `editors/vscode/README.md` (`npm install`, `code
  --extensionDevelopmentPath=editors/vscode some.btw`, set `btw.server.path`).
- **IntelliJ**: LSP4IJ pointed at `btwd lsp`, `editors/intellij/README.md`.
- **Web**: `scripts/build-web-wasm.sh <beats>/btwsite/web/wasm` (the module now has `BtwLs`), then the beats branch
  `feat/btwls-web` (`btwsite/web/js/assist.js`): Ctrl+Space, hover, Alt+Enter, Mod+Shift+O, F12.
- **Any client**: `btwd lsp` on stdio.

## Merging with feat/btw-fixer

`fix::fixes_of` is the one function to change: map `d.fixes` (the fixer's `Fix`: span, replacement, confidence, note) to
`QuickFix`, and keep `fix::adapter` for diagnostics with none. If the fixer's `settle` narrows anchored spans only in
`check --fix`, call it (or apply the anchor) before mapping. `djlive::partial` is a new module and touches no line of
`parse.rs`; the fixer edits `parse.rs`, so expect no conflict there. `crates/djlive/src/lib.rs` gains one `pub mod` line
on both branches.
