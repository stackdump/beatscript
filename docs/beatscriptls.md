# beatscriptls: the btw language service

`crates/beatscriptls` is the editor assistant for BeatScript (btw): diagnostics, quick-fixes, the outline, navigation, rename,
completion, hover, signature help, inlay hints and refactors. It is pure (no I/O, no clock), shaped like LSP, and served three
ways:

- `btwd lsp`: an LSP server on stdio, for VS Code (`editors/vscode`), IntelliJ (LSP4IJ, `editors/intellij`), Neovim, Helix.
- djweb's `BeatscriptLs` (wasm): the same server for the browser editor, positions in UTF-16.
- `beatscriptls::Server` in-process (the golden tests drive it).

## Two tiers

| tier | what | when |
|---|---|---|
| syntax | lex, parse (`parse_with`), the symbol table | every change; everything but the compile's diagnostics needs only this |
| check | `check_with` (expand, compile, evolve orbits) | off the keystroke path: `btwd lsp` runs it on a worker thread once the text has rested 150 ms; the page calls `BeatscriptLs.check(uri)` |

Until the check of a version is in, the diagnostics shown are the parser's; a check of a version the document has moved past
is dropped.

## Latency (phase 0)

`cargo run --release -p beatscriptls --example bench` on a shared Linux box (4 cores, shared with other builds; medians of 5 to 9 runs):

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
| `hover` on a reference (`gen@2 ...`, `seed(...)`, `<cid>.drop.bass`, `template(...)`), and on a `# from:` line | `refs.rs` + `djlive::refs` |
| `btw/expand` (not LSP: a document's references, their ranges and expansions) | `refs.rs` |
| `codeLens` (`show expansion`, `eject` over each reference), `workspace/executeCommand` | `refs.rs` |
| `codeAction` `refactor.inline` *Eject* (resolved and proved like the refactors) and *Copy the expansion* | `refs.rs`, `refactor.rs` |
| `completion` in a `gen@2` call: genres, names, values (`section=`: the generated program's sections) | `complete.rs` |

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

## References: show, hide, copy, eject

A program's references are what `djlive::refs::sites` lists: a call of the seeded generator (`gen@2 techno seed=42
section=drop part=bass`, an engine call), a seed (`seed(techno, 42).bass[1..8]`, an engine call), a stored program's section
or part (`<cid>.drop.bass`, a structural import) and a template (`arrange from template(...)`, a structural import). They are
worked out once per text (`Analysis::refs`: a generator call is generated and its program compiled, a few ms), and only for a
text with something a reference starts with in its tokens, so a program with none never pays.

- **Hover** on a reference: its canonical call, which half it is, the CID it resolves to, the generator and version, the
  track, what it was fitted to (`fitted to the program's key `am` and its tempo 124`), where it stands, and its expansion in a
  `btw` fence, at most 24 lines or 2400 characters, then `… N more lines not shown (M in all)`. On a `# from: <call> ->
  <cid>` line that an eject wrote, where the text came from. Nothing new for a client to do: it is `textDocument/hover`.
- **`btw/expand`** takes `{textDocument}` (every reference) or `{textDocument, position}` (the one there: whose own text holds
  the position, else whose statement does) and answers

  ```jsonc
  {"references": [{
    "range": {...},          // the reference itself, in the negotiated encoding (UTF-16 unless both said utf-8)
    "replaced": {...},       // what eject replaces: the reference, or the whole let, section or arrange line it stands in
    "kind": "gen",           // gen | seed | stored | template
    "half": "engine",        // engine | structure
    "call": "gen@2 techno seed=42 section=drop part=bass",
    "resolved": "z4EB…",     // the pinned CID (a stored reference: the CID it names)
    "generator": "seeded@2", // null but for gen
    "fitted": ["key", "bpm"],// what a gen call took from the program
    "place": "let",          // part | let | section | arrange
    "name": "bed",           // the let's or section's name, the part's role
    "text": "let bed_1 = ...",// what eject writes in place of `replaced`
    "from": "# from: gen@2 techno seed=42 section=drop part=bass -> z4EB…",
    "track": "...", "model": null, "lines": 5
  }]}
  ```

  and `{"references": [], "errors": [<LSP diagnostics>]}` when a reference cannot be worked out (the check says the same
  at the reference). Advertised as `capabilities.experimental.btw.expand`.
- **Code lenses** (`codeLensProvider`): over each reference `show expansion` (command `btw.expand.toggle`) and `eject`
  (`btw.eject`), each with one argument `{uri, position, length}` (the reference's start, and the text's length in bytes: an
  eject of a text that has changed since is refused).
- **`workspace/executeCommand`** (`executeCommandProvider.commands`: `btw.expand.toggle`, `btw.expand.copy`, `btw.eject`),
  arguments `[{uri, position, length?}]`:
  - `btw.expand.toggle` answers the reference there, as one of `btw/expand`'s (`null`: none);
  - `btw.expand.copy` answers `{text, call, resolved}`, `text` being the `from` line and the expansion: what eject would write;
  - `btw.eject` answers `{label, edit}` (a `WorkspaceEdit`) once the edit is proved, an error with the reason otherwise; when
    the client said `workspace.applyEdit` in `initialize`, the server also sends it the edit as a `workspace/applyEdit`
    request (id `btw/<n>`; its answer is taken quietly).

  **The show/hide state lives in the client.** The server answers what a reference stands for and keeps nothing; the VS Code
  extension and the web page remember which expansions are shown and ask `btw/expand` (they never send `btw.expand.toggle`
  or `btw.expand.copy` to the server). A client that runs those two on the server over stdio (LSP4IJ) gets a
  `window/showMessage` with the call, its CID and the first lines of the expansion as well.
- **Eject** is a `refactor.inline` code action on a reference, `Eject `<call>` into local text`, offered unresolved
  (`data.refactor = "eject"`) and made and proved on `codeAction/resolve` like the other refactors: its edit is the one stretch
  between the text and `djlive::refs::eject_site` of the same reference (exactly what `djlive::refs::eject` writes), and
  `refactor::verify` must find the same nets before it is given. Beside it, `Copy the expansion of `<call>``: no edit, a
  `command` (`btw.expand.copy`).
- **Completion** in a `gen@2` call: after its head the installed generator's genres (`djlive::gen::generator().genres(v)`),
  then its names (`djlive::gen::PARAMS`, `seed=` first, each once; on a part line the line's own params after them), then a
  name's values: `length=` the lengths, `key=` the keys, `part=` the roles, `section=` the sections of the program the call
  prints (generated and compiled there, a few ms). `arrange from`, `section NAME from` and `let NAME =` offer `gen@2`.

What each client gets:

| | hover | show / hide | copy | eject |
|---|---|---|---|---|
| VS Code (`editors/vscode`) | yes | lens, or *Show or hide the expansion of the reference here*: a decoration after the reference (its first line, how many more, `[z…cid]`; the whole in its hover); *every reference's*; *open … beside* (a read-only document) | lens-less: the lightbulb or the palette, to the clipboard | lens, lightbulb, palette (`workspace/applyEdit`) |
| IntelliJ (LSP4IJ) | yes (Ctrl+Q) | lens: a notification with the expansion | Alt+Enter: a notification | lens, Alt+Enter |
| the web page (`BeatscriptLs`) | yes | the JS below | the JS below | Alt+Enter as it is (the existing `run` resolves it) |

### The web editor

beats-bitwrap-io `btwsite/web/js/assist.js` needs no change for the hover or for eject (it is a refactor in the Alt+Enter
list, resolved by `run` as the others are). Showing an expansion and copying one are three small additions:

```js
// onKey, beside Alt+Enter: Alt+E shows or hides the expansion of the reference at the caret
if (e.altKey && (e.key === 'e' || e.key === 'E')) { e.preventDefault(); this.toggleExpansion(); return true; }

// run(action), first: an action with a command and no edit (Copy the expansion) is the page's to carry out
if (a.command && !a.edit && !(a.data && a.data.refactor)) {
  const r = this.req('workspace/executeCommand', a.command);
  if (!r || r.error) { this.toast(r && r.error ? r.error : 'Nothing to copy here.', 'warn'); return; }
  navigator.clipboard.writeText(`${r.text}\n`).then(() => this.toast('Copied the expansion.', 'ok'));
  return;
}

// a method: the expansion in the popup, under the reference (which one is shown is the page's state: the list)
toggleExpansion() {
  if (this.list && this.list.kind === 'expansion') { this.close(); return; }
  const res = this.req('btw/expand', { position: this.caretPos() });
  const ref = res && res.references && res.references[0];
  if (!ref) { this.toast(res && res.errors && res.errors.length ? res.errors[0].message : 'No reference here.', 'ok'); return; }
  const doc = `\`${ref.call}\` → \`${ref.resolved || ''}\`\n\n\`\`\`btw\n${ref.from}\n${ref.text}\n\`\`\``;
  this.list = { kind: 'expansion', rows: [{ label: ref.kind, detail: ref.half, doc }], sel: 0, anchor: offsetOf(this.ed.text, ref.range.start) };
  this.paint();
}
```

(`.as-doc` wants a `max-height` and `overflow: auto` for a long expansion.) Positions are UTF-16 there, as `BeatscriptLs` counts.

## Try it

- **VS Code**: `cargo build --release -p btwd`, then `editors/vscode/README.md` (`npm install`, `code
  --extensionDevelopmentPath=editors/vscode some.btw`, set `btw.server.path`).
- **IntelliJ**: LSP4IJ pointed at `btwd lsp`, `editors/intellij/README.md`.
- **Web**: `scripts/build-web-wasm.sh <beats>/btwsite/web/wasm` (the module now has `BeatscriptLs`), then the page's
  `btwsite/web/js/assist.js`: Ctrl+Space, hover, Alt+Enter, Mod+Shift+O, F12.
- **Any client**: `btwd lsp` on stdio.

## Merging with feat/btw-fixer

`fix::fixes_of` is the one function to change: map `d.fixes` (the fixer's `Fix`: span, replacement, confidence, note) to
`QuickFix`, and keep `fix::adapter` for diagnostics with none. If the fixer's `settle` narrows anchored spans only in
`check --fix`, call it (or apply the anchor) before mapping. `djlive::partial` is a new module and touches no line of
`parse.rs`; the fixer edits `parse.rs`, so expect no conflict there. `crates/djlive/src/lib.rs` gains one `pub mod` line
on both branches.
