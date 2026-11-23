# Changelog

## Unreleased

### The radio page is a player (2026-11-23)

- `/radio` plays as a player: a transport (previous, play/pause, next, stop), an LED readout (title, elapsed/total, section
  and bar, tempo and key), a level meter and the decks' roles lit as they fire, in a beveled dark chrome. The tuner and the
  station's program fold away under it.
- A playlist: the tab's "up next" queue, then the station's upcoming tracks, numbered, with title, genre, BPM, duration and
  each row's render status (none, queued, rendering, ready with the Opus link, failed) and where it plays from (render or
  live). Rows are added by CID, share link or example name, removed, reordered (drag, the arrow keys with Alt, or the
  buttons) and played by double-click or Enter. The header names the station's family and has more like this, scan and locks.
- "Play from": auto (the default: a row's render when it is ready, else live), render, live. Render statuses are asked for
  sparingly; a render is started only for the next row and only while someone is listening.
- Never dead air: the next item is fetched and compiled while one plays; when it is late or fails a bridge loop (CC0, the
  outgoing tempo and key) is hot-reloaded on the bar grid until it is ready. The order is documented in
  docs/beatscript/radio.md.
- Peek: the playing program in a drawer; an edit applies on the next bar, "back to station" puts it back.
- The queue: `POST /api/session/<id>/queue` (the page's add, remove, move, clear), `queue/next` takes `item` (and the reason
  `pick`), and the MCP `queue` tool has `move` (`item`, `to`) and `skip` to an `item`. The editor's "Up next" strip links to
  the player.

### The ontology knows every genre the generator knows (ontology 0.4.0, 2026-11-23)

- The BeatScript ontology has the generator's own genres, acid, deephouse, electro and italo, generated from djrust's genre
  tables (tempo, scale, root, groove, family, drum rhythms and variants, chord progressions, the voices and kits the generator
  picks per role), each `ns:followsRulesOf` the beats genre whose musicality rules it follows, with a usual tempo range.
  `#! genre: acid` resolves, and `check` gives acid real style advice instead of "the BeatScript ontology has no genre".
- The genres are structural: `ontology/build.sh` reads the generator's genre table, and the drift test
  (`ontology_matches_beatscript_serve`) fails when the generator and the ontology disagree on the set of genres or a genre's
  tempo, root, scale, family, groove, drum rhythms or progressions. It also holds the instrument roles to `djlive::parse::ROLES`
  and the part parameters (new `ns:PartParameter` terms) to `djlive::parse::PARAMS`.
- Each voice family says its generic sound (`ns:familyWave`: soft, sine, saw, bell or square), from the engine's tonal voice.
- The MCP fixtures are re-recorded as `mcp-0.4.0.json` (with `get_genre acid`); the header snapshot has 277 terms.

## v0.1.0 (2026-11-22): the first public release, and BeatScript 2

The first release of stackdump/beatscript as the single upstream of BeatScript and beatscript.app.

### The language is BeatScript, and BeatScript 2 is a new language version

The language was called btw. It is now BeatScript everywhere: the crates and binaries (`beatscript-api`,
`beatscript-cli` with the `beatscript` binary, `beatscriptls`, `beatscript-round`, `beatscript-seal`,
`beatscript-serve`), the docs (`docs/BEATSCRIPT.md`), the file extension (`.beatscript`; `.btw` is still read),
the MIME type (`text/beatscript`), the editors (VS Code language id `beatscript`, grammar `source.beatscript`), the
language server's methods (`beatscript/expand`, ...; the old `btw/*` methods still answer) and the beatscript.app
service.

- A program says its version on its first line: `#! beatscript 2`. Its canonical text keeps that line, and its CID is
  of `{"format":2,"text":<canonical text>,"type":"beatscript"}`, so the same music has a new CID in BeatScript 2.
- A text with `#! btw 1`, or with no version line, is BeatScript 1 (the old spelling). It is read, canonicalized and
  CID'd exactly as before: every old program, share link and CID keeps working.
- `fmt` (the editor's, the MCP tool's, `beatscript fmt`) upgrades a text to BeatScript 2. Nothing upgrades a program
  silently: the server's autoformat keeps the version it is given.
- `check` reports a program's `version` and `renamed_cid` (its CID in the other version). A program and its upgrade
  play exactly the same: the same nets, the same mutation dice, the same control net.
- The store keeps both CIDs: the BeatScript 2 twin of a stored program says it `isBasedOn` the old one with derivation
  `renamed`, and `/p/<old-cid>` redirects to `/p/<new-cid>`.
- The JSON-LD context is `beatscript-context-v1.jsonld`; the old context stays served, unchanged, at
  `/context/v1.jsonld`.
- The seeded generator's `gen@3` writes BeatScript 2; `gen@1` and `gen@2` print what they always printed.

### What else this release holds (2026-11-22)

- **Independence.** beatscript.app builds and deploys from this repo alone (`deploy/deploy.sh`, live checks, CI).
  Code is MIT; BeatScript programs and all content (lexicon, phrasebooks, themes, ontology, fixtures, docs) are
  CC0-1.0.
- **gen@N references.** A program can name a part of a seeded generator's program (`gen@3 techno seed=42
  section=drop part=bass`) and the reference resolves the same way forever; `check` reports a program's references.
- **The control deck.** `control:` holds the engine calls that take no target.
- **Forms, roles and cross.** Classical forms with roles, and `cross`, which composes a transition between two
  programs as a program of its own.
- **Radio, phase 1.** Endless, deterministic stations: position N of a station is always the same track, retuning
  keeps the position, and each segue is a `cross` with its own CID. `/radio`, the MCP `station` tool and
  `POST /api/station`.
- **The player and the artifact.** `@stackdump/beatscript-player` 0.1.0 and 0.2.0 (render-then-play mode), and the
  MCP `artifact` tool, which makes a program a page to publish with the pinned player or inline audio.
- **Embeds.** One configurable embed system: `<beatscript-player>` and `<beatscript-track>`, the embed config
  (`embed-config.schema.json`), `@stackdump/beatscript-studio`, README cards (`/card/<CID>.svg`), and clients for
  Jupyter and VS Code.
- **The seeded generator** on its own toolbar button in the editor, with no model and no sign-in.
- **Share links keep comments.** A link packs the formatted program (header, comments and blank lines), and its CID
  is still the canonical text's; links are made on beatscript.app.
- **OAuth.** MCP clients connect to beatscript.app with OAuth and capability grants.
