# BeatScript player, self-contained

`<beatscript-track>` is a BeatScript program and its player in one element. Everything it needs is in this package:
the engine the editor at https://beatscript.app runs (WebAssembly, inlined as base64), its drum kits, voices and the
phoneme tables `sing` uses, the AudioWorklet and the composer. Nothing is fetched from beatscript.app: the program's
CID and "Open on beatscript.app" are plain links. Sound starts only from a click.

It is made for pages with a strict Content Security Policy, such as a Claude artifact: scripts from the page's own files
or a CDN, fetches only to the page's own origin, no frames.

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.1.0/beatscript-track.js"></script>

<beatscript-track>
  <script type="text/beatscript">
tempo 120
deck A: kick x ~ x ~ | snare ~ x ~ x
  </script>
</beatscript-track>
```

Or from npm, `npm install @stackdump/beatscript-player`, then `import '@stackdump/beatscript-player'` in a bundled
page (types for the element are in `beatscript-track.d.ts`), or the same files beside the page:
`<script type="module" src="./beatscript-track.js">`. A page that loads classic
scripts only can use `beatscript-track.umd.js` (it imports the module from beside itself).

| attribute | |
|---|---|
| `text` | the program, instead of the child `<script type="text/beatscript">` |
| `cid` | the CID the text must have: a text that is not that program is refused |
| `compact` | a smaller card |
| `theme` | `dark` (the default) or `light` |
| `site` | where the plain links point (default `https://beatscript.app`) |

The element fires `play` and `stop` events.

**The page must allow WebAssembly.** A CSP whose `script-src` lacks `'wasm-unsafe-eval'` (or `'unsafe-eval'`) refuses
to compile any WebAssembly module, and no wasm engine can play under it; the element then says so instead of playing.
`test/csp.html` is a page under an artifact-like policy with `'wasm-unsafe-eval'` added.

Only programs written for this engine play: a program that names sample or loop assets (`uses_assets`) has nothing to
play them from here.

## Licence

The code is MIT (`LICENSE`). The programs in this package (`test/csp.html`'s) and the engine's data built into the
wasm (drum kits, voices, the sing phoneme tables) are CC0-1.0 (`LICENSES/CC0-1.0.txt`), as for every BeatScript
program. Source: https://github.com/stackdump/beatscript (`site/player`); `hashes.json` has the sha256 of every file
and the git revision the module was built from.
