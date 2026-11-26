# Changelog

## Unreleased

### Android App Links: the server embeds the statement, all four Play fingerprints, on every host (2026-11-26)

- `/.well-known/assetlinks.json` is served from the binary (`crates/beatscript-serve/src/public/assetlinks.json`, the one
  list of fingerprints in the repo): 200, `application/json`, no redirect, whatever Host. It was a host file with one
  fingerprint, and Play Console reported "Domain ownership not verified" on all three domains. `-assetlinks-file` stays
  as an explicit override; production leaves it unset.
- nginx: `btw.bitwrap.io` no longer 301s `/.well-known/assetlinks.json` and `beats.bitwrap.io` no longer 404s it
  (`site/deploy/nginx/assetlinks.snippet.conf`; `site/deploy/deploy.sh nginx-assetlinks`, `verify-assetlinks`).

### `listen` hears the mix: polish it by ear, with `compare` (2026-11-24)

- `listen` (the MCP tool and `beatscript listen`) measures the mix: each measured part and the master have `bands` (dB in
  sub, low, low_mid, mid, high_mid, high), `centroid_hz`, `width`, `correlation` and `crest_db`; the master also
  `true_peak_db` (4x oversampled), `plr_db`, `short_term_max_lufs` and `lra` (EBU Tech 3342); each section its `bands` and
  `true_peak_db`. The summary's version is `listen/2`.
- `clashes`: parts that carry the same band in the same bars at levels that mask each other, bar by bar (a duck that takes
  a bass out of the kick's way clears one even at the same levels). Two drums sharing a band are a kit, not a clash.
- `targets` (default streaming: -14 LUFS, -1 dBTP; also apple, broadcast, club, or `LUFS/dBTP`): the master's deltas and
  `met`.
- `warnings` come in the order to fix them (not heard, clipping, clashes, balance, loudness, width) and each names the
  BeatScript line that fixes it (`master: level 0.84`, `duck B under A.kick 6db`, `mix { B eq low -6 }`, `vel=`); each has a
  stable id in `warning_ids`.
- `compare` (the previous version, by CID or text; `--compare FILE` on the command line): deltas of the master, each part and
  each section, the targets before and after, and the warnings fixed, introduced and kept, with a one-line summary.
- A new page, [Polish a mix](https://beatscript.app/docs/polish): the loop, the targets, the order of fixes and the lines that
  make them, and what the language cannot do yet (no pan or width on a strip, no limiter, EQ per deck only), which is in
  docs/ROADMAP.md. llms.txt, the MCP instructions and the listen tool's description link it.

### The radio in any page: `<beatscript-radio>`, and the artifact tool's `station` (2026-11-23)

- `@stackdump/beatscript-player` 0.4.0 has `<beatscript-radio>` beside `<beatscript-track>`: a station and its player, its
  tracks and segues computed in the page by the engine's station answer (no server, nothing fetched, nothing stored). It
  plays on the `/radio` page's models (the play order, now `station-model.js` `playOrder`, the handoff and its CC0 bridges,
  the playlist's formatting, the presence pause) with a transport, now playing, section and bar, a level meter, the playlist
  of the station's next tracks and a peek drawer. Attributes: `station` (text or CID), `cid`, the seven knobs, `locks`, `n`,
  `segues`, `autoplay` (off by default), `compact`, `theme`, `site`.
- No gap at a track change: the next item is primed on an engine of its own (nets installed, decks held) and let go half a
  beat before the outgoing item's last bar line, or at once on Next, the outgoing engine fading out under it. The bridge
  covers a next item that is not ready. `site/test/player-radio.mjs` measures the element's output in headless Chrome.
- The MCP `artifact` tool takes `station` (a station's text, a stored station's CID or the name of an example that is one) and `n`,
  and answers a radio page: `<beatscript-radio>` from jsDelivr, pinned to 0.4.0 with an integrity hash on every module.
- The player instantiates the engine asynchronously: the wasm is over 8 MB, and Chrome refuses a synchronous
  instantiation of a module that size on the page's thread (`<beatscript-track>` built from today's engine did not load).
- `handoff-model.js` lets go of a next item that was prepared and then passed over (the order changed), instead of keeping
  its compiled run.
### `up` and `down` keep the key on markov and arp lines (2026-11-24)

- In an `arrange { }`, `up n` / `down n` on a let of `markov(...)` or `arp(...)` now move the notes it picks by `n` degrees
  of the key's scale, written on the call as `shift=n` (`markov(0.3, 1303)` up 2 is `markov(0.3, 1303, shift=2)`; in
  A minor `arp(am, up, 8)` up 1 plays b d f). Before, they did nothing and `check` warned. A `shift=` that comes back to 0
  is taken off, and the operators reach a generator `oct` already wrapped in `octave(...)`. Other lines move as before.
- `markov` and `arp` take `shift=n` (-21 to 21) written by hand too. A note off the scale (an arp chord's raised seventh)
  snaps down to a scale tone first, as the arrange pitch operators do with notes. Taste call: the alternative was a
  semitone transpose that keeps the key's name; scale degrees keep every note in the key instead.
- The implicit seed of a part with no chance leaves `shift=` out, so an unseeded `markov(...)` plays the same tune, moved.
- `oct` on a generator already played some octaves on adds to them (`octave(markov(...), 2)`) instead of doing nothing.

### Arrange mutations and evolve work where they are written (2026-11-23)

- A mutation that takes a pattern down to one step (`decay` or `thin` of a one-`[...]` let, `half` of `x ~`) no longer
  breaks the section that reads it: its generation is written `[~]`, a pattern, not a one-value let (which is put in place
  of its name before the lowering's lets exist, so the name stayed a word: "`acid@1` is not a step"). This was the error
  behind lets that read other lets, sections with a `sing` line, and `half` / `decay`.
- A let written as one `[...]` is its steps: every operator works inside the brackets. A `<...>` cycle of bars is worked a
  bar at a time (`rot 3`, `accent 0.3`, `ratchet` reach the bars instead of doing nothing or doubling the whole cycle).
- `ratchet` on a sixteenth strikes inside the step (`x:2`) instead of `x*2`, which snapped and lost hits; a coarser step
  still doubles on the grid (`x*2`).
- Roman numerals move by scale degrees (`<i i iv v>` up 2 is `<III III VI VII>` in A minor), and a let of `markov(...)` /
  `arp(...)` is pitched (`oct n` plays it `n` octaves on; the other pitch operators say they leave it): neither is called a
  drum pattern any more.
- `evolve` takes `fill p seed=s`, `slow n` and `fast n`; a mutation operator it does not take is an error that says it is an
  `arrange { }` operator.

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

### The acid voice by its own name, and its knobs

- `inst=acid-bass` is the 303-style acid voice (`acid` is the same voice and stays; `acid-square` its square). `instruments`
  lists it, and `check` says nothing about it.
- Its knobs are tonal part params: `cutoff=` (Hz, 20 to 12000), `res=`, `env=`, `accent=` and `drive=` (0 to 100 each),
  carried on each note's binding only when set, so no existing net or CID moves. `drive=` is new to the voice (the
  filter's saturation; 0, the default, is the voice as it was). They are not reserved words. On a drum part they are
  errors; on another voice, a warning. docs/beatscript/ext-acid.md.
- The seeded generator's `gen@4` (now the default) is `gen@3` with the acid genre's bass on `inst=acid-bass`; `gen@1` to
  `gen@3` print what they always printed.

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
