# BeatScript studio, self-contained

`<beatscript-studio>` is the BeatScript editor, its checker and the player in one element: the program is highlighted
and checked as it is typed, the problems are listed (a click jumps to one), and Play runs it on the engine the editor at
https://beatscript.app runs (WebAssembly, inlined as base64). Nothing is fetched from beatscript.app. Sound starts
only from a click.

The package carries the whole player, `@stackdump/beatscript-player` (`<beatscript-track>`, `./player`), so a page with
both elements loads one engine. It depends on nothing.

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-studio@0.1.0/beatscript-studio.js"></script>

<beatscript-studio>
  <script type="text/beatscript">
tempo 120
deck A: kick x ~ x ~ | snare ~ x ~ x
  </script>
</beatscript-studio>
```

With no program it starts from a short one.

## The config

The studio reads the one embed config every BeatScript surface reads (`./config`, `embed-config.js`; the schema is
https://beatscript.app/embed-config.schema.json): JSON in the `config` attribute or a child
`<script type="application/json" class="beatscript-config">`, with single attributes over it: `audio` (`live`, the
default, or `render`), `features`, `capabilities`, `theme` (`dark`, `light`, `auto`), `compact`, `title`, `site`,
`width`, `height` and `bridge`.

The studio's defaults are the features `highlight editor share download compose` and the capabilities
`sample mcp db downloads`. What the host cannot do is dropped, and the line under the editor says so in a sentence.

A `set` source (`[{text, title}, ...]`) is a list in the toolbar, each item by its title, else its `#! name:`; picking
one puts it in the editor, and when a program plays to its end the next item plays. An item with only a `cid` says it
needs its text. A `station` source needs a module with the station tool; without one the line under the editor says so.

## The toolbar

| button | |
|---|---|
| Play / Stop | play the program live; Ctrl+Enter (Cmd+Enter) plays, and while playing puts the edit into the running program |
| Render | with `audio: render`: render one whole pass, then play the WAV |
| fmt | format the program, as one undo step |
| share | copy the self-sealing link (the program is in the link); where the clipboard is refused, the link is selected |
| open | open a share link (its text is checked against its CID) |
| generate | the seeded generator: a genre and a seed |
| .beatscript, WAV | export the program or a render, through the host's `downloads` capability or a download link |
| compose | with the `sample` capability: ask Claude for a program or a change |
| store | with the `mcp` capability: store the program through the viewer's BeatScript connector |
| library | with the `db` capability: your saved programs |
| jam | with the `room` capability: send the program to everyone with the page open |
| dictate | with the `dictation` capability: beside the compose prompt, type what you say into it |
| read aloud | with the `tts` capability: on the status line, read the problems aloud (the list stays on screen) |
| ? | the cheatsheet |

The capability buttons appear only when the host resolved the capability (a Claude artifact's `claude.use()`).

## Tools

`element.tools` is the studio's tools as an MCP-shaped surface (`./tools`, `tool-surface.js`): `tools.list()` and
`tools.call(name, args)` (an MCP CallToolResult; it never rejects). The deterministic tools (check, fmt, expand,
generate, highlight, share, open, instruments, cheatsheet, station) run in the page; the others need their capability.
With `bridge`, a parent frame calls them over postMessage as JSON-RPC (`tools/list`, `tools/call`).

The element fires `change`, `check`, `play` and `stop`, and `ready` once the engine and the host have answered.

## A strict Content Security Policy

It is made for pages such as a Claude artifact: scripts from the page's own files or a CDN, fetches only to the page's
own origin. The styles are a constructed stylesheet in the shadow root, so `style-src 'self'` shows them. The page's
CSP must allow WebAssembly (`'wasm-unsafe-eval'` in `script-src`); without it the editor still edits and says why it
cannot check or play. `test/csp.html` is a page under such a CSP.

The code is MIT; the programs and the engine's data (kits, voices, the sing tables) are CC0-1.0.
