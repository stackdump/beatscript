# beatscriptls: the BeatScript language service

`crates/beatscriptls` is the editor assistant for BeatScript: diagnostics, quick-fixes, the outline, navigation, rename,
completion, hover, signature help, inlay hints and refactors. It is pure (no I/O, no clock), shaped like LSP, and served three
ways:

- `beatscript lsp`: an LSP server on stdio, for VS Code (`editors/vscode`), IntelliJ (LSP4IJ, `editors/intellij`), Neovim, Helix.
- djweb's `BeatscriptLs` (wasm): the same server for the browser editor, positions in UTF-16.
- `beatscriptls::Server` in-process (the golden tests drive it).

## Two tiers

| tier | what | when |
|---|---|---|
| syntax | lex, parse (`parse_with`), the symbol table | every change; everything but the compile's diagnostics needs only this |
| check | `check_with` (expand, compile, evolve orbits) | off the keystroke path: `beatscript lsp` runs it on a worker thread once the text has rested 150 ms; the page calls `BeatscriptLs.check(uri)` |

Until the check of a version is in, the diagnostics shown are the parser's; a check of a version the document has moved past
is dropped.

## Latency (phase 0)

`cargo run --release -p beatscriptls --example bench` on a shared Linux box (4 cores, shared with other builds; medians of 5 to 9 runs):

| program | size | syntax | check |
|---|---|---|---|
| showcase.beatscript | 11.1 KB, 253 lines | 2.4 ms | 9.8 ms |
| deep-house-60.beatscript (60 min, 9 evolves) | 6.5 KB, 192 lines | 1.1 ms | **3.1 to 5.1 s** |
| hand-dubstep-140-v4.beatscript | 4.3 KB | 0.8 ms | 4.8 ms |
| matrix-130.beatscript | 3.6 KB | 0.7 ms | 22.7 ms |
| trance-138.beatscript | 3.2 KB | 0.5 ms | 5.5 ms |
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
| `codeAction` quickfix + `source.fixAll.beatscript` | `fix.rs`: the fixer's repairs (adapter until feat/btw-fixer lands) |
| `codeAction` `refactor.extract` / `refactor.inline` + `codeAction/resolve` | `refactor.rs`: made and proved (same nets) on resolve |
| `documentSymbol`, `foldingRange` | `symbols.rs` (a scan of the text's own tokens) |
| `definition`, `references`, `documentHighlight`, `prepareRename`, `rename` | `nav.rs` |
| `completion` | `complete.rs` + `djlive::partial` (the statement up to the cursor) |
| `hover`, `signatureHelp` | `hover.rs`, `timeline.rs` |
| `inlayHint` | `inlay.rs` |
| `beatscript/diagnostics` (not LSP: the diagnostics as a request) | `diagnostics.rs` |
| `beatscript/numberAt` (not LSP: what an editor's scrubber needs) | `number.rs` |
| `hover` on a reference (`gen@2 ...`, `seed(...)`, `<cid>.drop.bass`, `template(...)`), and on a `# from:` line | `refs.rs` + `djlive::refs` |
| `beatscript/expand` (not LSP: a document's references, their ranges and expansions) | `refs.rs` |
| `codeLens` (`show expansion`, `eject` over each reference), `workspace/executeCommand` | `refs.rs` |
| `codeAction` `refactor.inline` *Eject* (resolved and proved like the refactors) and *Copy the expansion* | `refs.rs`, `refactor.rs` |
| `completion` in a `gen@2` call: genres, names, values (`section=`: the generated program's sections) | `complete.rs` |

The three requests that are not LSP were `btw/diagnostics`, `btw/numberAt` and `btw/expand` before the rename, as the
commands were `btw.expand.toggle`, `btw.expand.copy` and `btw.eject`, the language id `btw` and the grammar's scope
`source.btw`. They are `beatscript/...`, `beatscript.*`, `beatscript` and `source.beatscript` now. The server still answers
the old `btw/...` requests, so an editor built before the rename keeps working.

`beatscript/numberAt` takes `{textDocument, position}` and answers `null` or the literal there: `range` (its span, a `-` sign
included), `text`, `value`, `kind` (`tempo`, `vel`, `oct`, `len`, `glide`, `decay`, `pitch-drop`, `ratchet`, `times`, `chance`, `hits`, `steps`, `rotation`,
`cutoff-lp`, `level`, `mix`, `bars`, `bar`, `beat`, `degree`, `note`, `chord`, `param`, `number`, ...), `label`, `min` and
`max` (`null`: unbounded), `step`, `decimals`, `unit`, `integer`, `scale` (`log` for a cutoff: move it by a ratio),
`exact` (false when the literal is one operand of arithmetic: the limits are of the whole value), `span` for an end of a
range `a..b` (`ordered`: lo <= hi) or a ramp `a -> b` (`op`, `role` start|end, both ends' ranges and values), and for a
note or a Roman numeral `choices` (the notes of the key in force, every semitone with no key; the numerals of the key's
scale) with the `index` of the one written. A `do` param takes its type and range from the registry's `ParamSpec`
(`minimum`/`maximum` of the schema). The servers advertise it as `capabilities.experimental.beatscript.numberAt`. Every limit
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
- **`beatscript/expand`** takes `{textDocument}` (every reference) or `{textDocument, position}` (the one there: whose own text holds
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
  at the reference). Advertised as `capabilities.experimental.beatscript.expand`.
- **Code lenses** (`codeLensProvider`): over each reference `show expansion` (command `beatscript.expand.toggle`) and `eject`
  (`beatscript.eject`), each with one argument `{uri, position, length}` (the reference's start, and the text's length in bytes: an
  eject of a text that has changed since is refused).
- **`workspace/executeCommand`** (`executeCommandProvider.commands`: `beatscript.expand.toggle`, `beatscript.expand.copy`, `beatscript.eject`),
  arguments `[{uri, position, length?}]`:
  - `beatscript.expand.toggle` answers the reference there, as one of `beatscript/expand`'s (`null`: none);
  - `beatscript.expand.copy` answers `{text, call, resolved}`, `text` being the `from` line and the expansion: what eject would write;
  - `beatscript.eject` answers `{label, edit}` (a `WorkspaceEdit`) once the edit is proved, an error with the reason otherwise; when
    the client said `workspace.applyEdit` in `initialize`, the server also sends it the edit as a `workspace/applyEdit`
    request (id `btw/<n>`; its answer is taken quietly).

  **The show/hide state lives in the client.** The server answers what a reference stands for and keeps nothing; the VS Code
  extension and the web page remember which expansions are shown and ask `beatscript/expand` (they never send `beatscript.expand.toggle`
  or `beatscript.expand.copy` to the server). A client that runs those two on the server over stdio (LSP4IJ) gets a
  `window/showMessage` with the call, its CID and the first lines of the expansion as well.
- **Eject** is a `refactor.inline` code action on a reference, `Eject `<call>` into local text`, offered unresolved
  (`data.refactor = "eject"`) and made and proved on `codeAction/resolve` like the other refactors: its edit is the one stretch
  between the text and `djlive::refs::eject_site` of the same reference (exactly what `djlive::refs::eject` writes), and
  `refactor::verify` must find the same nets before it is given. Beside it, `Copy the expansion of `<call>``: no edit, a
  `command` (`beatscript.expand.copy`).
- **Completion** in a `gen@2` call: after its head the installed generator's genres (`djlive::gen::generator().genres(v)`),
  then its names (`djlive::gen::PARAMS`, `seed=` first, each once; on a part line the line's own params after them), then a
  name's values: `length=` the lengths, `key=` the keys, `part=` the roles, `section=` the sections of the program the call
  prints (generated and compiled there, a few ms). `arrange from`, `section NAME from` and `let NAME =` offer `gen@2`.

What each client gets:

| | hover | show / hide | copy | eject |
|---|---|---|---|---|
| VS Code (`editors/vscode`) | yes | lens, or *Show or hide the expansion of the reference here*: a decoration after the reference (its first line, how many more, `[z…cid]`; the whole in its hover); *every reference's*; *open … beside* (a read-only document) | lens-less: the lightbulb or the palette, to the clipboard | lens, lightbulb, palette (`workspace/applyEdit`) |
| IntelliJ (LSP4IJ) | yes (Ctrl+Q) | lens: a notification with the expansion | Alt+Enter: a notification | lens, Alt+Enter |
| the web page (`BeatscriptLs`) | yes | Alt+E, or the `⇒ show expansion` lens at the line's end while the caret is on a reference: a note after the reference (its first line, how many more, `[z…cid]`; hover it for up to 12 lines, click it for the whole); Alt+Shift+E or the Alt+Enter list: every reference's; *Open the whole expansion* (Alt+Enter) | the note's popup, or *Copy the expansion* in Alt+Enter: to the clipboard | Alt+Enter (the refactor, resolved), or the note's popup (`beatscript.eject`, as the lens) |

### The web editor

The page (this repo's `site/`, served at beatscript.app) does what the VS Code extension does, with the same calls:
`site/web/js/assist.js` draws it and `site/web/js/expansion-model.js` is its pure side (what is shown, where a note goes, what
it says, how a command is routed), pinned by `site/test/unit.mjs`; smoke.mjs step 17 drives it in headless Chrome against the
built module. Nothing new was needed of the module: `BeatscriptLs.request` already reaches `beatscript/expand`, `workspace/executeCommand`
and `codeAction/resolve` (`tests/refs.rs` `the_web_page_shows_copies_and_ejects_through_the_string_surface` drives the page's
calls through that string surface, on a server made as `BeatscriptLs::new` makes it).

- **Alt+E** shows or hides the expansion of the reference at the caret (`beatscript/expand` with the caret's position: the reference
  whose own text holds it, else whose statement does). A shown expansion is a read-only note drawn at the end of the line
  the reference ends on, four columns past it (the lightbulb sits in the first ones): `⇒ <its first line>  … N more lines
  [z…cid]`, cut to the room the line leaves (the first line is cut first, then dropped for the line count, then the CID). It
  is drawn over the code, never written into the textarea, so the text, its CID and undo do not move; and never over the
  text: on a row nearly full the gap shrinks to one column, and a line that leaves fewer than three columns gets no note
  (Alt+Enter, *Open* and the popup still reach the expansion). The key is taken only when there is something for it: a
  reference at the caret, or a problem of one there (its reason is said). Off any reference, and in a quoted phrase or a
  comment, Alt+E types as it would (Option+E is é, € or ę on a Mac).
- **The lens**: while the caret is on a reference whose expansion is hidden, `⇒ show expansion` stands where its note would
  (its title says what it does and the keys); a click shows it. This is the visible control for what VS Code's
  `show expansion` lens does.
- **Hover** a note: the call, its CID, kind and half, and the expansion under its `# from:` line, at most 12 lines or 1200
  characters, then how many more. **Click** a note: the whole expansion in the popup under the reference (scrolls), with
  *Hide the expansion*, *Copy the expansion* and *Eject … into local text*; the focus goes to the editor (so the popup's keys
  work) and a screen reader is told what opened, its length and first line, and the keys.
- **Alt+Shift+E**: every reference's expansion shown, or all hidden when any of them is (taken when the program has a
  reference; otherwise it types).
- **Alt+Enter** on a reference lists the service's *Eject … into local text* (a refactor: resolved, made and proved as the
  others are) and *Copy the expansion of …* (a command with no edit), then the page's own *Show/Hide the expansion of …*,
  *Show/Hide every reference's expansion* (when there are several) and *Open the whole expansion of …*.
- **Commands are routed as the extension's middleware routes them** (`expansion-model.route`): `beatscript.expand.toggle` and
  `beatscript.expand.copy` are the page's (it asks `beatscript/expand`; they never go to the server), `beatscript.eject` goes to the server as
  `workspace/executeCommand` with `[{uri, position, length}]`, `length` the UTF-8 byte length of the text the popup was
  opened for (what the server compares: an eject of a text that has changed since is refused, as a lens's is), and its
  `{label, edit}` is applied as one undoable edit; an error (the text changed, it cannot be ejected, no reference there) is a
  toast with the server's reason.
- **Copy** writes the `# from:` line and the expansion (what eject would write, as VS Code copies it). Where the clipboard is
  shut (no permission, an insecure origin) the whole opens under the reference instead, to select and copy by hand.
- **The state** is a set of keys in the page, `kind|call|place|name` (VS Code's), so an edit that moves a reference keeps
  it shown; a program loaded in place of the text (an example, a link, an assistant's `eval`) drops it, as VS Code drops a
  closed document's. An edit hides the notes at once (they would sit on moved text) and they are drawn again for the new
  text when it rests: after the caret's 120 ms and after the page's check; a resize or a font change redraws them where the
  text now wraps. Any edit (a paste with the mouse or a drop too, not only a key) and any load closes the fixes list, the
  outline and an expansion's popup, which were made for the old text.
- **A stored reference typed or pasted** (`<cid>.drop.bass`): the caret's first questions come before the page has fetched
  the program, and `BeatscriptLs` works a text's references out once and keeps them. So whenever the page tells the module a
  program (the Resolver's `know`), the document is opened afresh at the next request, and the caret's reference is asked
  again after the check that follows: the lens, the eject action and Alt+E work as soon as the program is in. A screen
  reader is told what was shown or hidden (an `aria-live` line), and `#ed-help` names the keys.

Decisions:

- **A note at the line's end, not a block under the reference.** The editor is a textarea under a highlighted `<pre>`: a
  line the text does not have cannot be drawn between two that it does without writing it, and a written expansion is
  eject. VS Code draws its decoration the same way (after the line).
- **The whole on a click, a part on hover.** The editor's tip does not take the pointer, so it cannot scroll: it shows 12
  lines (the service's own hover shows 24, under its other facts) and says how many more; the click's popup scrolls.
- **Alt+E toggles; it does not open the popup.** Hiding has to be a key too, and *Open the whole expansion* is in Alt+Enter.
- **Alt+E is taken only where there is a reference.** It is matched by `code` as well as `key`, because Option+E is a dead
  key on a Mac (and € or ę on other layouts), and that is how those characters are typed. So the page takes it only when
  the caret has a reference, or a problem of one, and never in a quoted phrase or a comment; everywhere else it types. A
  chord that never composes (Ctrl+Option+E) would be one more key to learn on one platform; on a reference there is
  nothing to type with Option+E.
- **The lens only at the caret**, not over every reference as VS Code's lenses are: the page has no lens rows, and a note
  on every reference line would crowd the code. Every reference's expansion is one key away.
- **"Every" hides when any is shown**, counting only the program's references now (a key of a reference that is gone does
  not make the next press hide); hiding clears every key.
- **Equal references show together.** Two references with the same kind, call, place and name (the same part call in two
  sections) have one key and one expansion, so one Alt+E shows both, as in VS Code. Telling them apart by their order would
  make an edit that adds an equal one above switch which is shown.

Not done: VS Code's *open … beside* as a second editor (the popup is the page's), and a per-reference lens row.

Positions are UTF-16 throughout, as `BeatscriptLs` counts and as JS strings index.

## Try it

- **VS Code**: `cargo build --release -p beatscript-cli`, then `editors/vscode/README.md` (`npm install`, `code
  --extensionDevelopmentPath=editors/vscode some.beatscript`, set `beatscript.server.path`).
- **IntelliJ**: LSP4IJ pointed at `beatscript lsp`, `editors/intellij/README.md`.
- **Web**: `scripts/build-web-wasm.sh site/web/wasm` (the module has `BeatscriptLs`), then `node site/test/mock-server.mjs` and the
  page it serves (`site/web/js/assist.js`): Ctrl+Space, hover, Alt+Enter, Mod+Shift+O, F12, Alt+E and Alt+Shift+E.
- **Any client**: `beatscript lsp` on stdio.

## Merging with feat/btw-fixer

`fix::fixes_of` is the one function to change: map `d.fixes` (the fixer's `Fix`: span, replacement, confidence, note) to
`QuickFix`, and keep `fix::adapter` for diagnostics with none. If the fixer's `settle` narrows anchored spans only in
`check --fix`, call it (or apply the anchor) before mapping. `djlive::partial` is a new module and touches no line of
`parse.rs`; the fixer edits `parse.rs`, so expect no conflict there. `crates/djlive/src/lib.rs` gains one `pub mod` line
on both branches.
