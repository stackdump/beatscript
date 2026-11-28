# BeatScript

BeatScript is a small, total, deterministic language for writing whole music tracks as text: one line per deck,
16 steps a bar, sections and an arrange. The same canonical text always has the same CID and the same audio.
This repository is everything [beatscript.app](https://beatscript.app) runs: the language, the server, the
browser engine, the web editor, the embed player, the docs, the examples, the lexicon and the ontology, and the
deploy with its live checks.

The engine underneath is [stackdump/djrust](https://github.com/stackdump/djrust) (Petri-net patterns and the
audio host), a git dependency pinned in `Cargo.toml`.

## BeatScript 2

A program starts with its version line, `#! beatscript 2`. BeatScript 2 is the language that was called `btw`,
renamed: every line means what it meant. A text with `#! btw 1`, or with no version line, is BeatScript 1, the old
spelling, and keeps its old CID, so every program, share link and CID made before the rename still opens and plays the
same. The version line is part of a BeatScript 2 program's canonical text, so the same music has a new CID there: `fmt`
upgrades a text to BeatScript 2 (`beatscript fmt --keep-version` does not), and beatscript.app keeps both CIDs, the new
one recorded as based on the old, with `/p/<old-cid>` redirecting to the new. Files are `.beatscript` (`.btw` is still
read), the MIME type is `text/beatscript`. The whole story is "Versions" in [`docs/BEATSCRIPT.md`](docs/BEATSCRIPT.md).

The names: the language and the product are BeatScript; `beatscript` is the name wherever a name is syntax (the CLI,
the crates, the file extension, the editors' language id). The desktop booth in djrust still calls its command noun
`btw` (`djctl btw eval`, `do btw`).

## Layout

| path | what it is |
|---|---|
| `crates/djlive` | the language: parse, compile to djrust nets, mutate, evolve, perform; `grammar.ebnf`, the form templates |
| `crates/djengine` | engine modules on top of djrust: vocals and the lexicon, themes, rights, key, SMF, import (plus booth modules) |
| `crates/beatscript-api` (`beatscript_api`) | check / fmt / open / share / highlight / marks as JSON, pure; shared by the CLI, the server and the wasm |
| `crates/beatscriptls` | the language service (diagnostics, completion, hover, refactors), LSP-shaped and pure |
| `crates/djweb` | the browser module (`beatscript.js`, `beatscript_bg.wasm`): `beatscript-api` + `beatscriptls` + the djrust engine behind wasm-bindgen |
| `crates/beatscript-serve` | the beatscript.app server (binary `beatscript-serve`): site, docs, render (Ogg Opus), check, sign-in, cards, pairing, MCP, ontology |
| `crates/beatscript-seal` | sealed private pushes to a paired phone |
| `crates/beatscript-cli` | the CLI, binary `beatscript`: check, fmt, render, cheatsheet, lsp, export |
| `crates/beatscript-round` | the round trip (binary `beatscript-round`): audio in, a program out that `check same_as` the original |
| `crates/djhost` | host modules (render path, ontology vocabulary, and the booth's devices behind features) |
| `docs/` | the language reference (`BEATSCRIPT.md`), arrange, schema, open data, authorship, `docs/beatscript/` long notes |
| `examples/` | example and reference programs |
| `editors/` | VS Code and IntelliJ support (`beatscript lsp`) |
| `site/` | the web editor, embed and npm player, site examples, API docs source, tests (with its history from beats-bitwrap-io) |
| `deploy/` | `deploy.sh` (build and deploy from one commit), `live-checks.sh` (every live check) and `test-host.sh` (the host checkout step against scratch dirs, in CI) |
| `scripts/` | the wasm and Linux builds, the build-host picker, the public-build guard |
| `tools/` | lexicon, listening tests, MCP catalogue, ontology, open-data dump |

## Build and test

```sh
cargo test --locked -p djlive -p djengine -p beatscript-api -p beatscriptls -p beatscript-seal -p beatscript-serve -p djweb -p beatscript-cli -p beatscript-round
scripts/check-public-build.sh            # what a default build may contain (below)
node --test site/test/unit.mjs           # the page's pure-JS tests
scripts/build-web-wasm.sh site/web/wasm  # the browser module (needs the wasm-bindgen CLI the lockfile pins)
```

Booth-only parts are off by default: djhost's `windwheel` feature (the Wheel, from
[stackdump/windwheel-bitwrap-io](https://github.com/stackdump/windwheel-bitwrap-io)) and its `wallet` feature (signed
set recordings, djrust-wallet). The browser module (djweb) has the Wheel too, the same instrument, so the page plays a
program's `wheel` lines as the booth does (`crates/djweb/src/wheel.rs`). `scripts/check-public-build.sh` fails if a
default build of any other shipped target has windwheel in its tree, if any has the wallet, or the booth's
pronouncing dictionary, which is not in this repository at all.

## Build your own

To put your own interface on BeatScript (a native app, an editor, a site or a tool), see
[`docs/beatscript/sdk.md`](docs/beatscript/sdk.md), served at [beatscript.app/docs/sdk](https://beatscript.app/docs/sdk).
It says which surfaces are supported and how each is versioned: the player and embed, the MCP server, the engine crates
as a git dependency pinned by rev (with the FFI the Android and iPad apps use), and the language service outside the
studio.

## Deploy

`deploy/deploy.sh all` builds the wasm, beatscript-serve and the docs from one commit on `main`, checks that commit out on
the serving host, installs the artifacts, points the service at it, restarts it and runs `deploy/live-checks.sh`. It
rolls back by itself when the restart or a check fails. Build hosts come from the environment
(`scripts/build-host.sh`). The server's data and secrets live on the host only and never in this repository.

## Licence

Code is MIT (`LICENSE`). BeatScript programs and all content are CC0-1.0: every `.beatscript` program, the examples, the
lexicon, phrasebooks and themes, the ontology, the test fixtures, and the docs. `REUSE.toml` says which path is
which; `LICENSES/` holds the texts. The Go fonts under `crates/beatscript-serve/assets/gofont` are BSD-3-Clause, with their
licence beside them. Programs stored on beatscript.app are CC0 by default.
