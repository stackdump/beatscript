# Changelog

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
- **Share links keep comments.** A link packs the formatted program (header, comments and blank lines), and its CID
  is still the canonical text's; links are made on beatscript.app.
- **OAuth.** MCP clients connect to beatscript.app with OAuth and capability grants.
