# Embed a program

Any program stored on beatscript.app can play on another page: your blog, a forum post, a course, a
README's site. There is one player and several ways in. The player is a small card with a play button,
the track's name, the section and bar playing, and its rights (licence, author and credits as stored,
and a link to [authorship and rights](/authorship)). It plays in the listener's browser with the same
engine the editor uses: nothing is downloaded but the engine, and only after a click.

What can be embedded is what the site stores: public, Creative Commons programs (CC0, the default, or a
CC BY licence) that use no samples. A private program, one with all rights reserved, or a link to a
program the site does not keep is not embeddable: its embed URL is a 404. Share a program while signed
in to store it. Through MCP, the `share` tool called signed in answers the player's URL as `embed_url`.
If a stored section has a doc comment (a `#` line above it), the player shows it while that section plays.

Every embed is read-only. It never stores, writes or signs anything, and it never starts on its own:
browsers allow sound only after a click, and the player waits for one.

## As an iframe

The player is a page, `https://beatscript.app/embed/<CID>`, that any site may frame:

```html
<iframe src="https://beatscript.app/embed/<CID>" width="480" height="152"
        title="BeatScript player" loading="lazy" frameborder="0"
        sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"></iframe>
```

- `?compact=1`: a one-line player, 80 pixels high (`height="80"`).
- `?theme=light`: the light theme (dark is the default).
- `/embed/<CID>.<packed>`, the form of a self-sealing share link, works too, for a stored program.

The sandbox above is what the player needs: to run, to load the engine from its own site, and to open
its links ("Open in BeatScript", the program's page) in a new tab. The frame can do nothing else to
your page, and your page's CSS and scripts cannot reach into it.

## As an element

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

The element draws the sandboxed iframe above inside its own shadow root, so it needs nothing else from
your page. It is a plain script with no inline styles, so a page with a strict Content Security Policy
needs only to allow `https://beatscript.app` in `script-src` and `frame-src`. There is no autoplay
attribute.

## oEmbed

Paste a program's link, `https://beatscript.app/p/<CID>` or `https://beatscript.app/c/<CID>`, into a
site that reads oEmbed (WordPress, Discourse, Notion, Ghost and others) and it becomes the player. The
program's page says where its oEmbed is:

```html
<link rel="alternate" type="application/json+oembed"
      href="https://beatscript.app/oembed?url=https%3A%2F%2Fbeatscript.app%2Fp%2F<CID>&amp;format=json">
```

and `GET /oembed?url=<link>&format=json` answers `type: "rich"` with the iframe as `html`, its `width`
and `height` (480 by 152; within `maxwidth`, and the compact 80-pixel player when `maxheight` is under
152), the `title`, `author_name` (the program's author, else its credits), the provider, the program's
card as `thumbnail_url`, and its `license` and `license_url`. A link to anything but an embeddable
program's page is a 404; `format=xml` is not served (501).

## Link previews that play

A program's social page, `https://beatscript.app/c/<CID>`, carries a player card: X shows the player in
the post (`twitter:card` `player`, the embed as `twitter:player`, 480 by 152), and Discord, Slack and
iMessage can play its audio (`og:audio`). Mastodon shows the card and links the audio. The short page,
`/p/<CID>`, keeps the large image card.

## Plain audio

Where a site strips iframes and scripts, or for a feed's enclosure, use the rendered file:

```html
<audio controls preload="none" src="https://beatscript.app/api/render/<CID>?format=opus"></audio>
```

It is Ogg Opus, rendered once per CID by the same engine offline, cached, and served with
`Cache-Control: immutable`: the same program always gives the same bytes. `/p/<CID>` with
`Accept: audio/*` redirects to it. Only stored programs are rendered (anything else is a 404), and a
first request waits for the render while it streams.

## Not yet

- A set or playlist as one embed (`/embed/set/<id>`).
- The section's doc comment under its name.
