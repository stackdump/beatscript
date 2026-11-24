# BeatScript player, self-contained

`<beatscript-track>` is a BeatScript program and its player in one element. Everything it needs is in this package:
the engine the editor at https://beatscript.app runs (WebAssembly, inlined as base64), its drum kits, voices and the
phoneme tables `sing` uses, the AudioWorklet and the composer. Nothing is fetched from beatscript.app: the program's
CID and "Open on beatscript.app" are plain links. Sound starts only from a click.

It is made for pages with a strict Content Security Policy, such as a Claude artifact: scripts from the page's own files
or a CDN, fetches only to the page's own origin, no frames.

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.4.0/beatscript-track.js"></script>

<beatscript-track>
  <script type="text/beatscript">
#! beatscript 2
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
| `mode` | `live` (the default): the engine plays as it runs. `render`: render the whole track first, then play it (below) |
| `text` | the program, instead of the child `<script type="text/beatscript">` |
| `cid` | the CID the text must have: a text that is not that program is refused |
| `compact` | a smaller card |
| `theme` | `dark` (the default) or `light` |
| `site` | where the plain links point (default `https://beatscript.app`) |
| `stored` | the site has the program stored: its CID links to `/p/<CID>` |
| `highlight` | show the program under the card, coloured as the editor colours it, with a Copy button |
| `config` | the embed config as JSON (`https://beatscript.app/embed-config.schema.json`, also a child `<script type="application/json" class="beatscript-config">`): one config for the player, `<beatscript-studio>` and the site's `embed` tool. Its theme, compact, site, cid, audio (`live` or `render`), features (`highlight`) and source text become the attributes above, which win over it; what the player cannot do of it (an editor, a station) is in `report.notes` |

The element fires `play` and `stop`, and in render mode `pause` and `rendered`. Stop is silent at once (the engine and
what it had scheduled are let go), and the next Play starts from bar 1.

The CID and "Open on beatscript.app" are both the self-sealing share link (`/#<CID>.<packed>`): the program travels in
the link, so it opens on the site whether or not the site has stored it. With `stored`, the CID links to `/p/<CID>`.

`highlight` colours the comments at once and everything else once the engine's own highlighter has loaded, which
fetches the module the player uses (once, for both). Copy uses the clipboard; where the page may not write it (a frame
without `clipboard-write`), it selects the text for the reader to copy.

### Render, then play

```html
<beatscript-track mode="render">
  <script type="text/beatscript">
#! beatscript 2
tempo 128
section verse 8 bars {
  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
}
section drop 8 bars {
  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
}
arrange { verse drop } loop
  </script>
</beatscript-track>
```

The first Play renders one pass of the arrangement offline, faster than real time, and shows how far it has got (a
percentage and the bars done). The engine is the one live mode plays (the same wasm, `js/engine.js` and its worklet),
driven by a clock of its own instead of an AudioContext: the runtime is serviced every 1024 frames as a live page
services it every 25 ms, the processor renders 128 frames at a time in between, and every message and compile is
waited for before the clock moves on. Nothing depends on the wall clock, so **the same program renders to the same PCM
every time**, on any machine (the tests check a browser's render against Node's, byte for byte).

- It is one pass: an arranged song once (an `end` cuts it), `end at bar N`, else 16 bars, as `/api/render` plans it;
  capped at 256 bars and 600 s. A looping song (`arrange { } loop`) renders one pass and loops it (the loop box).
- It renders in a Worker when the package is on the page's own origin, else (from a CDN, where a page may not start a
  Worker) on the page's thread, in slices that leave the page responsive.
- It plays from a 16-bit, 44.1 kHz WAV in memory through an `<audio>` element on a `blob:` URL, with a seek bar,
  pause and loop. A page whose CSP refuses `blob:` media (a Claude artifact's `default-src 'self'` does) plays the same
  samples through Web Audio instead, with the same controls; the browser reports the refused probe as one
  `securitypolicyviolation`. A later Play plays the same render; `stop()` pauses and goes back to 0.
- `track.wav` is the WAV (a Blob) once it has rendered, and `report.render` says how it went (frames, ms, via,
  playback).

The render is also a module of its own, for a page, a Worker or Node:

```js
import { renderProgram } from '@stackdump/beatscript-player/render';
const module = await WebAssembly.compile(wasmBytes); // wasm/beatscript_bg.wasm (the package has it base64 in wasm/beatscript_bg.wasm.b64.js)
const { left, right, sampleRate, frames, bars } = await renderProgram({ module, text, onProgress: (p) => console.log(p.fraction) });
```

It is the editor's own render, not a copy: beatscript.app's "render" source plays the same files (`js/render.js`, its
Worker, `js/render-client.js` for the WAV and its playback, `js/render-model.js`), which `build.mjs` copies from
`site/web/js` byte for byte; `site/test/render-shared.mjs` checks both render the same PCM.

**The page must allow WebAssembly.** A CSP whose `script-src` lacks `'wasm-unsafe-eval'` (or `'unsafe-eval'`) refuses
to compile any WebAssembly module, and no wasm engine can play under it; the element then says so instead of playing.
`test/csp.html` is a page under an artifact-like policy with `'wasm-unsafe-eval'` added.

Only programs written for this engine play: a program that names sample or loop assets (`uses_assets`) has nothing to
play them from here.

### BeatScript 2

From 0.3.0 the player reads BeatScript 2, the language under its new name: a program whose header says
`#! beatscript 2` (the line the editor's fmt writes first). A program in the old spelling, `#! btw 1` or no version line
at all (every program written before the rename), still plays as it did and keeps its CID; `cid` is checked against the
program in the version it says. The element, its attributes, its events and its types are those of 0.2.0.

## `<beatscript-radio>`: a station

From 0.4.0 the package has the radio too: a BeatScript station (docs/beatscript/radio.md) and its player, on the models of
beatscript.app/radio. A station's tracks are deterministic from its text, so the element computes them in the page with the
engine's own station answer (the wasm's `beatscript_station`, what `POST /api/station` answers): nothing is fetched, and it
stores nothing (no cookies, no localStorage).

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.4.0/beatscript-radio.js"></script>

<beatscript-radio genre="house" locks="tempo key"></beatscript-radio>
```

With no `station` it plays the techno radio. The station can be its text (an attribute, or a child
`<script type="text/beatscript">`), or its CID with the text in the child script (the text must be that station).

| attribute | |
|---|---|
| `station` | the station's program text, or its CID (the text then in a child `<script type="text/beatscript">`) |
| `cid` | the CID the station's text must have: a text that is not that station is refused |
| `genre` `seed` `step` `forms` `tempo` `key` `length` | the station's knobs: each one given retunes the station before it plays (`forms` is a space-separated rotation; `step` is `+N` or `~`) |
| `locks` | knobs held fixed, space or comma separated: "scan" and "more like this" leave them alone |
| `n` | the position to start at (default 0) |
| `segues` | `off`: no segue between tracks (default on, as /radio plays them) |
| `autoplay` | start without a click when the browser lets it (default off: a Claude artifact needs a click, and the element then says so) |
| `compact` | the transport and the title only |
| `theme` | `dark` (the default) or `light` |
| `site` | where the plain links point (default `https://beatscript.app`) |

It shows a transport (previous, play/stop, next), the title, elapsed and total time, the section and bar, the tempo and
key, a level meter, the playlist (what plays, then the station's next tracks and the segue after the one playing, each
with its status: playing, resolving, ready, bridging, failed), "more like this" (one seed along) and "scan" (a notch along
a knob that is not locked), and a peek drawer: the playing program's text, an edit applied on the next bar (a hot
reload), "back to station" and Copy. Changing an attribute retunes it, and what plays moves to the new station's track N.

**No dead air, no gap.** What is next is compiled while the current item plays and primed on an engine of its own (its
nets installed, its decks held). Half a beat before the current item's last bar line, or at once on Next, the held decks
are let go and the outgoing engine fades out under them over 120 ms. When the next item is not ready in time, a small CC0
bridge loop at the outgoing tempo and key is hot-reloaded into what plays (it lands on the next bar line) until it is.
`site/test/player-radio.mjs` holds it to that: across every track change, no silence on the element's output longer than
one 1024-frame buffer. At most three engines run at once (playing, fading, primed).

It fires `play`, `stop`, `trackchange` (detail `{kind, n, cid, title}`) and `bridge`. `radio.output` is the GainNode all
of its sound goes through (after the first Play), `radio.report` what the tests read. Types: `beatscript-radio.d.ts`
(`import '@stackdump/beatscript-player/radio'`). The MCP `artifact` tool's `station` option writes a whole page with it,
pinned to this version with an integrity hash on every module. `test/radio.html` is a page under an artifact-like CSP.

A long track's check and compile (a form's template is written out: a few seconds for a 200-bar fugue) runs on the
page's thread while the current item plays; its sound goes on (the engine is on the audio thread), but the readout
pauses for that long.

## Licence

The code is MIT (`LICENSE`). The programs in this package (`test/csp.html`'s) and the engine's data built into the
wasm (drum kits, voices, the sing phoneme tables) are CC0-1.0 (`LICENSES/CC0-1.0.txt`), as for every BeatScript
program. Source: https://github.com/stackdump/beatscript (`site/player`); `hashes.json` has the sha256 of every file
and the git revision the module was built from.
