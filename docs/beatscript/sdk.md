# Build your own

BeatScript is built so that you can put your own interface on it: a native app, an editor, a site or a tool. Everything
beatscript.app runs is in this repository, and the same engine runs the Android app and the iPad app. This page says
which parts you can build on, how each one is versioned, and where each one already lives. It is a map. The details stay
in the pages it links to, so nothing here is a second copy.

There are four ways in:

- **The player and the embed**, to play a program on a page or in an app (`<beatscript-track>`, `<beatscript-studio>`,
  the iframe, oEmbed, the card and the audio). See [Embed a program](/docs/embed).
- **The MCP server**, so that an assistant can write, check and play programs: `https://beatscript.app/mcp`, with the
  tools listed at [/mcp](/mcp).
- **The engine crates**, to build a native app in Rust, or in Kotlin or Swift through FFI. See below.
- **The language service**, to check, format, expand and colour a program outside the studio. See below.

## What is supported, and how it is versioned

A surface on this list changes only in the way its row says. Every change to it is in `CHANGELOG.md`. A change that
would break a caller comes as a new version, and nothing is changed in place.

| surface | what you build on | how it is versioned | how you pin it | what holds it |
|---|---|---|---|---|
| the language | programs, their canonical text and their CIDs | the version line, `#! beatscript 2`, in the text. A BeatScript 1 text keeps its meaning and its CID. Seeds and templates carry their own versions (`seed(...)` v1, `#! templates: v1`). The one grammar is [grammar.ebnf](/grammar.ebnf) | the CID: the same canonical text always has the same CID, on every device | the CID pins (`crates/beatscript-api/tests/cid-pins.json`), held by the Android and iPad goldens too |
| the JSON reports | check, fmt, expand, highlight, share and open as JSON (`site/DESIGN.md`, "JSON shapes") | fields are added, and a field is never renamed or given a new meaning in place. A report carries the language `version` of the program it reports on | the engine's git revision | the api goldens (`crates/beatscript-api/tests/golden/`); `crates/djweb/tests/drift.rs` holds the server, the wasm facade and the language service to one answer |
| the embed config | `https://beatscript.app/embed-config.schema.json` | its own `"v": 1` | the `v` field | `site/test/embed-config.test.mjs` and the `embed` tool's tests |
| the npm packages | `@stackdump/beatscript-player` (`<beatscript-track>`, `<beatscript-radio>`) and `@stackdump/beatscript-studio` | semver, published to npm and served by jsDelivr | an exact version and its SRI hash, never a range or `latest`. The site's own pins are `crates/beatscript-serve/src/mcp/*-cdn.lock.json` | `site/test/player-csp.mjs`, `site/test/studio-csp.mjs` |
| the MCP server | the tools at `https://beatscript.app/mcp`: their names, arguments and answers | a tool is added, and its arguments are added to. A tool is not removed or renamed in place | nothing: it is a service | the recorded `tools/list` (`crates/beatscript-serve/src/mcp/testdata/tools-list.json`) and the ontology, which names every tool |
| the HTTP API | the routes in [the API reference](/docs/api) (`POST /api/check`, `/api/fmt`, `/api/render/<CID>` and the rest) | as the MCP server | nothing: it is a service | the server's route tests and `deploy/live-checks.sh` |
| the ontology | `https://beatscript.app/ontology/`: the vocabulary, its SPARQL endpoint and its MCP server | a release, `owl:versionIRI https://beatscript.app/ontology/<version>`, whose files are locked by sha256 | the version IRI | `crates/beatscript-serve/ontology/release.lock` |
| the engine crates | `djlive`, `beatscript-api`, `beatscriptls`, `djengine`, `djhost`, `beatscript-seal`, `djweb` | not on crates.io. The Rust API has no semver promise yet: it is whatever the revision you pin has | a git dependency pinned by `rev` (below) | each crate's tests at that revision |

These are **not** supported surfaces, and they change without notice: the site's own scripts (`site/web/js/`), the
module the site serves at `/wasm/beatscript.js` (it changes with every deploy, so build your own from a pinned revision
instead, as described below), the server's internals, and the booth-only features (`windwheel`, `wallet`).

### Licences

The code is MIT. Everything else in this repository is CC0-1.0: the programs and examples, the lexicon, the phrasebook
and themes, the ontology and the docs. `REUSE.toml` says which path is which. Programs stored on beatscript.app are CC0
by default, and so is the audio a CC0 program renders to. An author may choose a CC BY licence instead, and a program
keeps the licence its author gives it ([Authorship and rights](/authorship)). What you build on these is yours, and your
own code and programs need not use these licences. `docs/LICENSING.md` explains how rights travel with a program into its
CID and how a fork credits what it came from.

One rule holds on beatscript.app: content that is not Creative Commons never reaches
the site. A program that uses samples (`decode`, below) or says `use restricted` plays on your own device, but it is not
stored, shared or embedded. A default build of every shipped target leaves the booth-only parts out, and
`scripts/check-public-build.sh` checks that.

## The engine crates, for native builders

The engine is a set of Rust crates in this repository on top of [stackdump/djrust](https://github.com/stackdump/djrust)
(the Petri-net patterns and the audio host). Use them as git dependencies, pinned by commit:

```toml
[dependencies]
# every stackdump/beatscript rev must be the same commit
djlive = { git = "https://github.com/stackdump/beatscript", rev = "<commit>" }
beatscript-api = { git = "https://github.com/stackdump/beatscript", rev = "<commit>" }
djhost = { git = "https://github.com/stackdump/beatscript", rev = "<commit>", features = ["audio", "sf2"] }
```

There are two rules. Every `stackdump/beatscript` revision must be the same commit. And `djrust`, `djrust-wallet` and
`windwheel`, if you name them yourself, must be at the revisions this repository's root `Cargo.toml` pins, or cargo builds
two copies of each and their types do not unify. To move the pin, change every rev and run
`cargo update -p djengine -p djhost -p djlive -p beatscript-api -p beatscript-seal`.

| crate | what it is | I/O |
|---|---|---|
| `djlive` | the language: parse, check, compile to nets, CIDs, share links, evolve, perform | none: the same text is the same net |
| `beatscript-api` | the language as JSON (check, fmt, expand, highlight, share, open), the seeded generator, stations | none |
| `beatscriptls` | the language service, shaped like LSP | none |
| `djengine` | engine modules on top of djrust: vocals and the lexicon, themes, rights, key, SMF, import | none (`std` on by default; `transfer` for a prepared track as bytes) |
| `djhost` | the host: the render path, the decks, and every device behind a feature | only what its features turn on |
| `beatscript-seal` | sealed private pushes from beatscript.app to a paired device | none |
| `djweb` | the browser module: all of the above behind wasm-bindgen | none |

`djhost`'s features, all off by default:

| feature | what it turns on | notes |
|---|---|---|
| `audio` | sound out through cpal: CoreAudio on macOS and iOS, ALSA on Linux, AAudio on Android | Linux needs `libasound2-dev` to build |
| `sf2` | SoundFont 2 palettes through rustysynth, pure Rust | |
| `midi` | controllers through midir: CoreMIDI on macOS, the ALSA sequencer on Linux | midir has no Android backend, so the Android app leaves it out |
| `decode` | audio files onto a sample deck through symphonia (WAV, AIFF, FLAC, MP3, AAC, ALAC, Ogg Vorbis) | your own files, so they stay on your device (see Licences) |
| `transfer` | a prepared track as bytes | |
| `network` | djnet, the LAN protocol between devices (`docs/NET-PROTOCOL.md`) | off: no listener, no discovery, no client. The default build is the no-network build |
| `neural` | beat detection for outside audio (Beat This!) | fetches its model files on first use; off, nothing touches the network |
| `windwheel`, `wallet` | booth-only parts | not part of the supported surface |

With no features, `djhost` still renders. `beatscript render` (the CLI) and the server's render are built that way, and
they build on a bare Linux box.

### The FFI, as the Android and iPad apps use it

Both apps reach the engine through one Rust crate, `djmobile`, which puts [UniFFI](https://mozilla.github.io/uniffi-rs/)
over the engine crates. It lives in `stackdump/beatscript-app` (`rust/`). That repository is private for now, so take its
shape as the pattern for your own crate:

- A `cdylib` and `lib` crate that depends on `djlive`, `djengine`, `beatscript-api` and `djhost` by one `rev`, with
  UniFFI, plus a `uniffi-bindgen` binary that reads the library's metadata.
- `djhost` with `audio` and `sf2`, and without `midi`. Network code (djnet, pairing, sealed pushes through
  `beatscript-seal`) sits behind the crate's own `network` feature, which is off by default.
- **Android** (`app.beatscript`) builds it for aarch64, x86_64 and armv7 and loads the `.so` with Kotlin bindings.
  Gradle passes the feature (`-Pdjandroid.network=true`).
- **iPad** ([stackdump/beatscript-ios](https://github.com/stackdump/beatscript-ios), public) builds the same crate as
  an xcframework with Swift bindings (`scripts/build-xcframework.sh`). `rust/PINS` holds two revisions: the crate's and
  this repository's. It builds with `network`, for pairing.
- Both hold the engine to the same CIDs: the iPad's XCTests check against the Android app's goldens and against what
  beatscript.app serves.

### The browser module

`crates/djweb` is the same engine for a page. `scripts/build-web-wasm.sh <outdir>` builds it from your checkout into
`beatscript.js`, `beatscript_bg.wasm`, the TypeScript declarations and `hashes.json` (the sha256 of each file and the git
revision it was built from). It needs the wasm-bindgen CLI that `Cargo.lock` pins. To play a program on a page without
building anything, use the npm player, which carries the module inside it.

## The language service, outside the studio

Check, format, expand and highlight already run in seven places, and every one of them gives the same answer. Pick the one
that fits what you are building:

| from | check | fmt | expand | highlight | more | where it is |
|---|---|---|---|---|---|---|
| a shell | `beatscript check` | `beatscript fmt` | `beatscript expand` | `beatscript highlight` | `open`, `share`, `diff`, `same-as`, `render`, `cheatsheet`; JSON on stdout, `--file F` or `-` | `crates/beatscript-cli` (binary `beatscript`): `cargo install --git https://github.com/stackdump/beatscript --rev <commit> --locked beatscript-cli` |
| an editor | diagnostics | `textDocument/formatting` | `beatscript/expand`, and the eject refactor | the TextMate grammar in `editors/vscode` (the server has no semantic tokens) | quick-fixes, outline, navigation, completion, hover, inlay hints | `beatscript lsp` (LSP over stdio): [docs/beatscriptls.md](https://github.com/stackdump/beatscript/blob/main/docs/beatscriptls.md), `editors/vscode`, `editors/intellij` |
| an assistant | `check` | `fmt` | `expand` | | `listen`, `generate`, `share`, `embed`, the rest | the MCP server, `https://beatscript.app/mcp` |
| a page, offline | `check` | `fmt` | `expand` | `highlight` | `generate`, `share`, `open`, `station` | `<beatscript-studio>`'s `element.tools`, or its postMessage bridge: [Embed a program](/docs/embed), "The studio" |
| JavaScript | `beatscript_check(text)` | `beatscript_fmt(text)` | `beatscript_expand(text, lineage)` | `beatscript_highlight(text)` | `BeatscriptLs` (LSP messages as JSON: `open`, `change`, `check`, `request`, `handle`) | the browser module, above |
| Rust | `beatscript_api::check_json` | `fmt_json` | `expand_json` | `highlight_json` | the `beatscriptls` crate | the engine crates, above |
| HTTP | `POST /api/check` | `POST /api/fmt` | | | | [the API reference](/docs/api) |

They agree because they are one implementation. `beatscript-api` and `beatscriptls` are pure Rust, and the CLI, the
server, the MCP, the wasm module and the studio all call them. `crates/djweb/tests/drift.rs` checks that the server,
the page's facade and the language service give the same diagnostics for the same text.

## Not yet

- The engine crates on crates.io, with a semver promise for their Rust API.
- `djmobile`, the apps' FFI crate, as a public crate.
- The browser module (`djweb`) on npm by itself, without the player around it.
- A `highlight` tool on the site's MCP server. The studio has one, and the CLI and the wasm module have it too.
