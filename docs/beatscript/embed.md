# Embed a program

A BeatScript program can play on another page: a Claude artifact, your blog, a forum post, a notebook, an editor's
preview, a README, a feed, the Android app. Every one of these surfaces reads the same small JSON object, the **embed
config**: what plays, how it sounds, what it shows and which of the host's capabilities it may use. Give the same config
to each surface and each does what it can with it. Where a surface cannot do something the config asks for, it does
the next best thing and says so in one plain sentence. The schema is `https://beatscript.app/embed-config.schema.json`.

Two rules hold everywhere. Nothing starts on its own: browsers allow sound only after a click, and every player waits
for one. There is no autoplay attribute. And the player, the iframe, the card and the audio are read-only: they never
store, write or sign anything. The studio writes only through what the host grants it (the viewer's BeatScript
connector, the artifact's own storage, a download), and only when one of its tools is called (below).

What the site serves (the iframe, oEmbed, link previews, the card and the rendered audio) is what the site stores:
public, Creative Commons programs (CC0, the default, or a CC BY licence) that use no samples. A private program, one
with all rights reserved, or a link to a program the site does not keep is not embeddable: its embed URL is a 404.
Share a program while signed in to store it. Through MCP, the `share` tool called signed in answers the player's URL as
`embed_url`. The elements need no stored program: they carry the text and play it with the engine in the page.

## The config

```json
{
  "kind": "player",
  "source": { "cid": "<CID>" },
  "theme": "light",
  "compact": true
}
```

| field | values | default | what it does |
|---|---|---|---|
| `v` | `1` | `1` | the config's own version |
| `kind` | `player`, `studio`, `card`, `audio` | `player` | which surface (below) |
| `source.text` | the program text, at most 64 KiB | | what plays |
| `source.cid` | a CID | | alone: a program stored on the site, for the site's own forms and the `embed` tool (the elements fetch nothing, so they need the text). With `text`: the CID the text must have (any other text is refused) |
| `source.set` | 1 to 64 items, each `{text}` or `{cid}`, with an optional `title` | | a set or playlist, played in order |
| `source.station` | a radio station's name | | position `n` always gives the same track (`/radio`) |
| `source.n` | 0 or more | `0` | with `station`: the position to start at |
| `source.count` | 1 to 64 | `8` | with `station`: how many tracks to queue |
| `audio` | `live`, `render`, `inline`, `url` | by kind | how it sounds |
| `features` | `highlight`, `editor`, `mixer`, `knobs`, `download`, `share`, `compose` | by kind | what the surface shows |
| `capabilities` | `sample`, `mcp`, `db`, `downloads`, `room`, `dictation`, `tts` | by kind | the host capabilities it may use, where the host allows them |
| `theme` | `dark`, `light`, `auto` | `dark` | `auto` follows the reader's colour scheme |
| `compact` | `true`, `false` | `false` | the small form: an 80-pixel player, a studio without its side panel |
| `width`, `height` | CSS pixels: width 80 to 4096, height 40 to 4096 | the surface's own | where the host needs a size: an iframe, oEmbed |
| `title` | at most 80 characters | the program's `#! name:` | the name shown and the page's title |
| `site` | an `https://` URL | `https://beatscript.app` | where plain links point, and where the iframe, card and audio come from |
| `bridge` | `true`, `false` | `false` | studio only: answer MCP-shaped JSON-RPC from a parent frame (below) |
| `pin.version` | an exact version, such as `0.2.0` | the generator's own | for generators that write script tags: the package version to load |
| `pin.sri` | `true`, `false` | `true` | whether each script tag gets its Subresource Integrity hash |
| `$schema` | the schema's URL | | for an editor that checks the config |

`source` takes exactly one of `text`, `cid`, `set` or `station`; `cid` may go with `text`.

The three fields whose default depends on the kind:

| kind | `audio` | `features` | `capabilities` |
|---|---|---|---|
| `player` | `live` | `share` | `downloads` |
| `studio` | `live` | `highlight`, `editor`, `download`, `share`, `compose` | `sample`, `mcp`, `db`, `downloads` |
| `card` | `url` | `share` | none |
| `audio` | `inline` | `download` | none |

What the values mean:

- `audio`. `live`: the wasm engine plays as it goes. `render`: the engine renders one whole pass in the page first,
  then plays it with a seek bar. `inline`: the site's Ogg Opus render carried in the page as a `data:` URI (no script,
  no engine; a stored program only). `url`: the site's render by URL (an `<audio src>`, a feed's enclosure).
- `features`. `highlight`: the program, coloured as the editor colours it, with Copy. `editor`: the text is editable
  and re-checked as it is typed. `mixer`: deck levels and mutes. `knobs`: the program's control-deck knobs as sliders.
  `download`: save the `.beatscript` text and a WAV. `share`: the self-sealing link and the CID. `compose`: ask Claude for a
  program or a change (it needs `sample`).
- `capabilities`. `sample`: compose and repair ask Claude. `mcp`: store, render and queue through the viewer's
  BeatScript connector. `db`: a private library of programs. `downloads`: save a WAV and the `.beatscript`. `room`: a live jam
  with everyone who has the page open. `dictation`: speak a compose prompt. `tts`: read the problems aloud.

### Three ways to give it

As JSON in the `config` attribute:

```html
<beatscript-track config='{"source":{"text":"tempo 120\ndeck A: kick x ~ x ~\n"},"theme":"light","compact":true}'></beatscript-track>
```

As JSON in a child script, which needs no quoting:

```html
<beatscript-studio>
  <script type="application/json" class="beatscript-config">
    { "capabilities": ["sample", "mcp", "downloads"], "bridge": true }
  </script>
  <script type="text/beatscript">
tempo 120
deck A: kick x ~ x ~ | snare ~ x ~ x
  </script>
</beatscript-studio>
```

As single attributes, which win over the JSON:

```html
<beatscript-track mode="render" highlight theme="light" text="tempo 120&#10;deck A: kick x ~ x ~&#10;"></beatscript-track>
```

| attribute | sets |
|---|---|
| `text`, `cid`, `station`, `n`, `count` | `source` |
| `audio`, or `mode` | `audio` |
| `features` | `features`, a list split on spaces or commas; a bare `highlight` attribute adds `highlight` |
| `capabilities`, or `caps` | `capabilities`, the same kind of list |
| `theme`, `title`, `site`, `width`, `height` | the field of that name |
| `compact`, `bridge` | `true` when present, `false` for the value `false` |

The program text can also come from a child `<script type="text/beatscript">` (or `text/x-beatscript`), as above; its
common indent is taken off. The tag gives the kind: a `<beatscript-track>` is a player and a `<beatscript-studio>` a
studio. The player reads `config` from version 0.3.0. Version 0.2.0 reads only the single attributes it had (`text`,
`cid`, `mode`, `highlight`, `theme`, `compact`, `site`, `stored`), so generated markup writes those as attributes and
puts only the rest in `config`.

### How a surface degrades

A surface first checks the config against the schema and fills in the defaults. An error names the field; a setting
that does nothing there is a warning ("n and count apply only to a station"). The player then notes what it cannot do
of the config, such as an editor.

The studio goes further. It asks the host what it can do: whether this is a Claude artifact, whether WebAssembly
compiles under the page's Content Security Policy, whether there is an AudioWorklet, an OfflineAudioContext and a
clipboard, and which capabilities `claude.use()` grants (a capability with no answer in 10 seconds is absent). Then it
drops whatever the host cannot do and keeps a note for each change: what changed, whether it still `works`, is
`degraded` or is `unsupported`, and a sentence that says so:

- "compose needs Claude: the sample capability in a Claude artifact"
- "the mcp capability is not available here"
- "download falls back to a plain browser download"
- "there are no knobs yet"
- "this page does not allow WebAssembly ('wasm-unsafe-eval'), so it cannot play here"

The studio shows its notes under the editor. Both elements give them in `element.report.notes`, and the `embed` tool
gives them beside its snippet.

## The kinds

- **`player`**: a card with a Play button, the track's name, the section and bar playing, and its rights (licence,
  author and credits as stored, and a link to [authorship and rights](/authorship)). It plays in the reader's browser
  with the same engine the editor uses. If a stored section has a doc comment (a `#` line above it), the site's player
  shows it while that section plays. In a page it is `<beatscript-track>`; on the site it is `/embed/<CID>`.
- **`studio`**: the editor, its checker and the player in one element, `<beatscript-studio>`. The deterministic tools
  run in the page, offline, on the wasm engine; the rest use the host's capabilities when it has them. In a Claude
  artifact it is a BeatScript MCP server inside the page (below).
- **`card`**: a static card: the name, the tempo and key, the sections as bars, the licence and the CID, with no script
  and no sound. The site draws it as an image, `/card/<CID>.svg` or `/card/<CID>.png`, and a program's social page,
  `/c/<CID>`, carries it. A CID that is not embeddable gets the generic card.
- **`audio`**: a plain `<audio controls>` of the site's render, `/api/render/<CID>?format=opus`. No script and no
  engine.

## The studio: an MCP server inside the page

`<beatscript-studio>` offers its tools the way an MCP server does: each has a name, a title, a description, an input
schema and annotations, and a call answers an MCP `CallToolResult` (`content`, `structuredContent`, `isError`). The
page, the model through `sample`, or a parent frame can call them.

The local tools run on the wasm engine in the page. They need no network, no sign-in and no capability, so they work
offline. Their names and arguments are those of the site's MCP tools:

| tool | what it does |
|---|---|
| `check` | the errors and warnings at line:col, the CID of the canonical text, tempo, key, sections and seconds |
| `fmt` | the program as the site stores it, with its `#!` lines and comments kept |
| `expand` | the program with its `seed(...)`, `template(...)` and `gen@N` references written out |
| `generate` | the seeded generator: a whole program for a genre and a seed, with no model |
| `highlight` | the highlighter's spans, `[start, end, class]` |
| `share` | the self-sealing links and the CID (nothing is stored) |
| `open` | open a self-sealing link and check its text against its CID |
| `instruments` | the voices, kits and palettes a program may name |
| `cheatsheet` | the compact grammar, as Markdown |
| `station` | a radio station's tracks from position `n` |

The capability tools use what the host grants. `tools.list()` lists every tool, each with `available` and, when it is
not, `why`:

| tool | needs | what it does |
|---|---|---|
| `compose` | `sample` | ask Claude for a program or a change; the answer is checked here and its problems sent back at most twice |
| `repair` | `sample` | send a program and its problems to Claude until it checks, at most twice |
| `store` | `mcp` | store a program that checks, publicly, through the viewer's connector (its `share`) |
| `render` | `mcp` | the site's render of a text or a CID (its `render`); a text is stored publicly to render it |
| `queue` | `mcp` | the "up next" list on one of the viewer's BeatScript tabs (its `queue`) |
| `library_list`, `library_save`, `library_get`, `library_delete` | `db` and `user` | the viewer's private library in the artifact's storage |
| `export` | `downloads` | save the text (`name.beatscript.txt`) or a WAV rendered in the page (`name.wav.zip`); the viewer confirms |
| `jam_send` | `room` | send a program that checks, at most 4 KiB, to everyone with the page open; their studios check it again |

Nothing that writes runs on its own: each tool acts only when it is called, by one of the studio's buttons or by a
caller.

From the page, `element.tools` is the surface once the element has fired `ready`:

```js
const studio = document.querySelector('beatscript-studio');
studio.addEventListener('ready', async () => {
  for (const t of studio.tools.list()) console.log(t.name, t.via, t.available ? 'yes' : t.why);
  const r = await studio.tools.call('check', { text: 'tempo 120\ndeck A: kick x ~ x ~\n' });
  console.log(r.structuredContent.ok, r.structuredContent.cid);
});
```

`call` never rejects: a failure is a result with `isError: true` and a sentence that says why. Where the engine cannot
load (a page that refuses WebAssembly), `element.tools` stays `null`.

### The postMessage bridge

With `bridge` on, the studio answers JSON-RPC 2.0 messages posted to its window: `initialize`, `tools/list`,
`tools/call` and `ping`. `tools/list` lists only the tools that are available. A parent page that frames a page with the
studio calls it like this:

```js
const frame = document.querySelector('iframe');
addEventListener('message', (e) => {
  if (e.source === frame.contentWindow && e.data && e.data.jsonrpc === '2.0') console.log(e.data.id, e.data.result);
});
frame.contentWindow.postMessage({ jsonrpc: '2.0', id: 1, method: 'tools/list' }, '*');
frame.contentWindow.postMessage({ jsonrpc: '2.0', id: 2, method: 'tools/call',
  params: { name: 'fmt', arguments: { text: 'tempo 120\ndeck A:   kick x ~ x ~\n' } } }, '*');
```

It answers any window that can post to it. The capability tools still act only as far as the viewer allowed them.

## Where it works

| target | kinds | engine runs? | what you get | status | how it is tested |
|---|---|---|---|---|---|
| Claude artifact | player, studio | yes: artifacts allow `'wasm-unsafe-eval'` | a page with the player, or the whole studio with the viewer's capabilities | works | `site/test/player-csp.mjs` and `site/test/studio-csp.mjs`: under an artifact's CSP in headless Chrome; `site/test/studio-tools.test.mjs`: the tools against fakes of the capabilities |
| any web page (the element) | player, studio | yes | `<beatscript-track>` or `<beatscript-studio>`; `<beatscript-player>` from `embed.js` for a stored program | works | `player-csp.mjs` (beside the page and from a second origin, as from a CDN), `studio-csp.mjs`; `site/test/unit.mjs` runs `embed.js` |
| iframe | player | yes, in the frame | the site's player, `/embed/<CID>` | works | `crates/beatscript-serve/tests/embed_routes.rs`, `deploy/live-checks.sh` |
| oEmbed (WordPress, Discourse, Notion, Ghost) | player, card | yes, in the frame | a pasted link becomes the iframe | works | `embed_routes.rs`, `deploy/live-checks.sh` |
| link previews (X, Discord, Slack, iMessage, Mastodon) | card, player | X: yes, in its frame. The rest: no | X plays the player in the post; Discord, Slack and iMessage play `og:audio`; Mastodon shows the card and links the audio | works on X; degraded elsewhere | `embed_routes.rs`, `deploy/live-checks.sh` |
| Markdown, GitHub README | card, player, audio | no: scripts and frames are stripped | an SVG card at `/card/<CID>.svg`, linked to the program, and a link to its audio | degraded | `cards/svg.rs` unit and route tests; the snippet in `mcp/embed.rs`'s tests |
| Jupyter | player, studio | yes, in the cell's output | the element, from `beatscript_embed.py` | works in a trusted notebook (scripts in an untrusted one stay off: the cell shows a link to the program) | `site/embed/jupyter/test_beatscript_embed.py`, without a notebook |
| VS Code | player | yes, in a webview | "BeatScript: Preview with the player", beside the editor | works | `editors/vscode/test/preview.test.js`, without VS Code |
| Obsidian and other Markdown apps | player, card | yes, in the frame | the iframe only: scripts are stripped | degraded | the iframe's tests and the snippet in `mcp/embed.rs`'s; not tested in the apps |
| plain `<audio>` | audio | no | the site's render, Ogg Opus | works, but not in older Safari | `embed_routes.rs` |
| RSS enclosure | audio | no | the site's render as the item's `audio/ogg` enclosure | works | `opendata.rs` unit tests (the site's own feeds); the snippet in `mcp/embed.rs`'s tests |
| the Android app | player, studio, card | yes, the app's own | the self-sealing link opens the program in the app | the app claims `/p` on beatscript.app (and, for links made before the move, on the site's older hosts) | `site/test/unit.mjs` (the link the QR code holds); not the app |

Where the engine cannot run, the target gets the server's work instead:

- A README gets an SVG card at `/card/<CID>.svg` and a link to the audio: GitHub strips scripts and frames.
- RSS and `<audio>` get the server's render. The same program always gives the same bytes.
- Link previews other than X get `og:audio`, the same render.
- Obsidian strips scripts, so only the iframe plays there.

## Recipes

### A Claude artifact: the studio

A page with `<beatscript-studio>` declares the capabilities it uses when it is published as an artifact. For the
studio's defaults and a jam room, the Artifact tool's `capabilities` are:

```json
{
  "sample": {},
  "mcp": { "servers": [{ "server": "Beatscript", "tools": ["share", "render", "queue"] }] },
  "db": {},
  "user": {},
  "downloads": true,
  "room": { "topics": { "program": "interact" } }
}
```

`sample` lets compose and repair ask Claude. `mcp` names the viewer's BeatScript connector, "Beatscript", and the three
of its tools the studio calls. `db` keeps the library, and `user` says whose library it is. `downloads` saves files.
`room` is the live jam: a jam is sent on the `program` topic, which this opens to the page's contributors (a topic that
is not declared is for its editors only; viewers receive but do not send). Leave out what you do not want: the tools that need it then say they are not available, and the
local tools still work. The `embed` tool with target `artifact` and kind `studio` answers the page and this object.
The page loads `@stackdump/beatscript-studio` from jsDelivr, pinned by version and SRI (`crates/beatscript-serve/src/mcp/studio-cdn.lock.json`).

### Any page: jsDelivr with SRI

```html
<script type="importmap">{"integrity": {"https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.2.0/js/engine.js": "sha384-..."}}</script>
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.2.0/beatscript-track.js"
        integrity="sha384-..." crossorigin="anonymous"></script>

<beatscript-track>
  <script type="text/beatscript">
tempo 120
deck A: kick x ~ x ~ | snare ~ x ~ x
  </script>
</beatscript-track>
```

Pin an exact version, never a range or `latest`. The module script's `integrity` covers the entry; the import map's
`integrity` covers every module the entry imports, so a changed file anywhere refuses to run. The `artifact` and `embed`
tools write these tags with the real hashes, which are in `crates/beatscript-serve/src/mcp/player-cdn.lock.json`. The page must
allow scripts from `cdn.jsdelivr.net` and WebAssembly (`'wasm-unsafe-eval'`).

### As an iframe

The player is a page, `https://beatscript.app/embed/<CID>`, that any site may frame:

```html
<iframe src="https://beatscript.app/embed/<CID>" width="480" height="152"
        title="BeatScript player" loading="lazy" frameborder="0"
        sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"></iframe>
```

- `?compact=1`: a one-line player, 80 pixels high (`height="80"`).
- `?theme=light`: the light theme (dark is the default).
- `/embed/<CID>.<packed>`, the form of a self-sealing share link, works too, for a stored program.

The sandbox above is what the player needs: to run, to load the engine from its own site, and to open its links ("Open
in BeatScript", the program's page) in a new tab. The frame can do nothing else to your page, and your page's CSS and
scripts cannot reach into it.

### The element for a stored program

One script, then one element per program:

```html
<script src="https://beatscript.app/embed.js" defer></script>

<beatscript-player cid="<CID>"></beatscript-player>
<beatscript-player link="https://beatscript.app/p/<CID>" compact theme="light"></beatscript-player>
```

| attribute | |
|---|---|
| `cid` | the program's CID |
| `link` | instead of `cid`: a `/p/<CID>` or `/c/<CID>` link, an editor link (`/#<CID>.…` or `/?cid=<CID>`) |
| `compact` | the one-line player |
| `theme` | `dark` (the default) or `light` |

The element draws the sandboxed iframe above inside its own shadow root, so it needs nothing else from your page. It is
a plain script with no inline styles, so a page with a strict Content Security Policy needs only to allow
`https://beatscript.app` in `script-src` and `frame-src`. There is no autoplay attribute.

### oEmbed

Paste a program's link, `https://beatscript.app/p/<CID>` or `https://beatscript.app/c/<CID>`, into a site that reads
oEmbed (WordPress, Discourse, Notion, Ghost and others) and it becomes the player. The program's page says where its
oEmbed is:

```html
<link rel="alternate" type="application/json+oembed"
      href="https://beatscript.app/oembed?url=https%3A%2F%2Fbeatscript.app%2Fp%2F<CID>&amp;format=json">
```

and `GET /oembed?url=<link>&format=json` answers `type: "rich"` with the iframe as `html`, its `width` and `height`
(480 by 152; within `maxwidth`, and the compact 80-pixel player when `maxheight` is under 152), the `title`,
`author_name` (the program's author, else its credits), the provider, the program's card as `thumbnail_url`, and its
`license` and `license_url`. A link to anything but an embeddable program's page is a 404; `format=xml` is not served
(501).

### Link previews that play

A program's social page, `https://beatscript.app/c/<CID>`, carries a player card: X shows the player in the post
(`twitter:card` `player`, the embed as `twitter:player`, 480 by 152), and Discord, Slack and iMessage can play its audio
(`og:audio`). Mastodon shows the card and links the audio. The short page, `/p/<CID>`, keeps the large image card.

### Jupyter

Put `site/embed/jupyter/beatscript_embed.py` beside the notebook. It is one file and needs only the standard library.

```python
from beatscript_embed import Track, show

text = open('song.beatscript').read()
Track(text=text, theme='light')     # the cell's last value shows the player
show(text)                          # display it from anywhere in a cell
```

`Track` and `show` take the config's keys as arguments (`theme`, `compact`, `audio`, `features`, `title`, `site`,
`pin` and the rest). `Studio(...)`, or `show(text, kind='studio')`, is the studio.

The notebook must be trusted (one you ran yourself) to run the output's scripts, and it must reach `cdn.jsdelivr.net`.
Where the output's scripts do not run (an untrusted notebook, a static viewer such as GitHub's), the output is a plain
link to the program instead.

### VS Code

Open a `.beatscript` file and run **BeatScript: Preview with the player** from the Command Palette, or press its button in the
editor's title bar. The preview opens beside the editor and plays the file with the published player. It follows the
file once the typing rests and when it is saved, and it follows the colour theme. Its page allows scripts from
`cdn.jsdelivr.net` only, WebAssembly, and no connections at all.

### Obsidian and other Markdown apps

They strip scripts, so paste the iframe. Obsidian shows it in reading view and live preview:

```html
<iframe src="https://beatscript.app/embed/<CID>?compact=1" width="480" height="80"
        title="BeatScript player" loading="lazy" frameborder="0"
        sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"></iframe>
```

### A README

```markdown
[![My track](https://beatscript.app/card/<CID>.svg)](https://beatscript.app/p/<CID>)

[Listen](https://beatscript.app/api/render/<CID>?format=opus)
```

The SVG has no script and loads nothing; `/card/<CID>.png` is the same card as a PNG.

### Plain audio

Where a site strips frames and scripts, use the rendered file:

```html
<audio controls preload="none" src="https://beatscript.app/api/render/<CID>?format=opus"></audio>
```

It is Ogg Opus, rendered once per CID by the same engine offline, cached, and served with `Cache-Control: immutable`:
the same program always gives the same bytes. `/p/<CID>` with `Accept: audio/*` redirects to it. Only stored programs
are rendered (anything else is a 404), and a first request waits for the render while it streams. Older Safari does not
play Ogg Opus, and there is no AAC render.

### An RSS enclosure

```xml
<enclosure url="https://beatscript.app/api/render/<CID>?format=opus" length="0" type="audio/ogg"/>
```

The site's own feeds, `/feed.atom` and `/feed.json`, give each new public program this render the same way.

### The Android app

The self-sealing link, `https://beatscript.app/p#<CID>.<packed>`, is the one the Share panel's QR code holds. The
program travels in the link, so the BeatScript Android app opens it offline and checks the CID itself. Android offers
the app for the link; without the app, the link opens the program in the editor on the site. The app claims `/p`
on beatscript.app, and on the site's older hosts for links made before the move.

## The `embed` MCP tool

The site's MCP server has an `embed` tool: give it a target and a config, and it answers the snippet or the whole page
for that target. The targets are `artifact`, `web`, `iframe`, `oembed`, `preview`, `markdown`, `jupyter`, `vscode`,
`obsidian`, `audio`, `rss`, `android` and `json` (the normalized config). The answer says whether the config `works`
there, is `degraded` or is `unsupported`, whether the engine runs there, and the notes on what the target cannot do.
With target `artifact` it gives the page and the `capabilities` to publish it with.

For one program as one page to publish, the `artifact` tool is the short form. It takes `text`, `cid` or `example`, and
answers a page that plays with the player from jsDelivr, pinned by version and SRI, or with the site's Opus render
carried inline.

## Not yet

- An iframe of the studio: the site frames only `/embed/`.
- Knobs and the mixer.
- A station in the studio: this build's engine has no station answer (`beatscript_station`), so the studio says so and points at /radio. A CID alone in the studio or the player: neither fetches anything, so give the text too.
- A set or playlist as one embed on the site (`/embed/set/<id>`). The player plays a set's first item.
- The `embed` tool's `audio` target with the render inline; it links it by URL. The `artifact` tool carries it inline.
- The artifact player page with `audio` `render`, `highlight`, `theme` or `compact`.
- The Android app opening beatscript.app links.
- The section's doc comment under its name.
