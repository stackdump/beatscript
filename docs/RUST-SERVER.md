# beatscript.app on Rust: beatscript-serve replaces the Go btwsite server

Goal (Matt, 2026-11-19/20): beatscript.app runs entirely on Rust, for streaming performance. Playlists were
designed to live in beatscript-serve from the start. This document is the plan of record. It takes over from the survey on
beats-bitwrap-io branch `docs/rust-server-playlists` (`docs/RUST-SERVER.md` there), which stays useful for its
benchmarks and for §10 on playlists, and updates the inventory against Go `main` as of 2026-11-20 (f1862c4:
render r2 and `v=`, same-as/diff, parity) plus `feat/docs-html` (the rendered docs, still on its branch).

Status, 2026-11-20 (late): **every route group is ported (P1-P7) on branch `feat/btwserve-full`**: static, docs,
examples, render, info/site/assetlinks/instruments/context, programs, check/fmt/share/open/qr, auth, shares and
cards, pairing/devices/sessions/queue, compose/admin, MCP. `proxy::go_owned` is empty: beatscript-serve answers everything
itself, and `-fallback` is kept only as an escape hatch. The contract suite (`bench/contract-all.sh`) runs every
group against Go with beatscript-serve alone (no fallback). The browser smoke test (`site/test/smoke.mjs --server`) gives
the same result on beatscript-serve as on Go. Nothing is deployed and nginx is unchanged. The cutover kit is
`crates/beatscript-serve/deploy/` (§6).

## 1. Architecture

```
                 nginx (pflow.dev, beatscript.app + btw.bitwrap.io vhosts)
                         │  every location → upstream btw_front
                         ▼
   beatscript-serve :18098 (Rust, axum 0.8 + hyper 1 on tokio, one process)
   ├── owned routes ──► served here: render (engine in-process), static, docs, examples, [later groups]
   └── everything else ─► -fallback http://127.0.0.1:18097 ─► Go btwsite (as it is today)
                         ▲
   shared -data dir: programs/  render/  session.secret  tokens.json  devices.json  site.json  …
```

- **One crate, `crates/beatscript-serve`** in this private repo (it links `beatscript-api`, `djlive`, `djhost`, `djengine`). The
  renderer is the `beatscript` CLI's own `render.rs`, included by path, so the CLI and the server cannot drift. Before the cutover it
  should become `beatscript`'s library target (`[lib]`, `pub mod render`), a one-line change.
- **HTTP**: axum 0.8 + hyper 1 (`http1`, `title_case_headers`) on tokio. Every answer goes through one header
  layer (`site_headers`) that adds Go's security headers, CSP, `publicRead` CORS and license `Link`s, then sorts the
  headers by name as Go writes them. Answers from the Go fallback pass through unchanged.
- **Go's semantics, ported where they show on the wire**: `http.ServeContent` (`content.rs`: `If-Match`,
  `If-None-Match` → 304, `Range`/`If-Range`, multipart ranges, 416, HEAD), ServeMux's redirects (unclean path → 307
  to its clean form, `/docs` → `/docs/`), net/http's chunking rule (a body a handler writes without a length gets
  `Content-Length` up to 2048 bytes and `Transfer-Encoding: chunked` past that: `go_body`), encoding/json's
  `<>&` escaping and float formatting (`go_json`, `go_num`), `http.Error`'s text answers.
- **Render** (from `plan/rust-render-server` + `val/render-cutover-877941d9`): the engine on a pool of
  `-render-workers` threads (earliest deadline first, `sched.rs`), WAV written straight to the cache file and Opus
  encoded in-process (libopus 1.3.1 transpiled, `unsafe-libopus`, pure-Rust Ogg), singleflight on a growing
  `.rs.part` file with followers reading from byte 0, the render stops when its last follower leaves, cache hits
  served from a memory map, LRU by size, redacted access log.
- **The strangler is inside beatscript-serve** (`proxy.rs`), not in nginx. `proxy::go_owned` lists the Go mux's
  patterns that beatscript-serve does not serve yet. Everything under `/api/` that beatscript-serve does not own goes to Go, so a
  route Go adds later is never answered here as a static 404. Moving a route group to Rust is a code change plus a
  contract run, with no nginx edit. Rollback for the whole server is one line in nginx (the upstream).

Why the fallback lives in beatscript-serve rather than in per-route nginx locations: the static site is Go's catch-all
(`/`). Once static moves, which is phase 1, nginx would need an explicit location for every remaining Go route,
and a missed one would be silently answered by Rust's 404. With `-fallback`, nginx keeps one target, and the list
of Go routes lives in code next to the test that checks it. The cost is one loopback hop for unmoved routes
(sub-millisecond; SSE and render streams pass through unbuffered).

## 2. Inventory of the Go server and where each piece stands

Legend: **done** = on `feat/btwserve-phase1`, contract-tested against Go · **branch** = built in Rust on another
branch, not merged · **missing** = still Go's (reaches Go through `-fallback`) · **differs** = on purpose, see §5.

| area | routes / behaviour (Go package) | Rust status |
|---|---|---|
| static site | `/` and `/p` (index.html with the generic card meta), every file under `-web` incl. engine wasm, `/privacy/`, `/delete-account/`, dotted segments 404, content ETag, `no-cache`, Range, 304 (server/handlers.go `handleStatic`, card `InjectGeneric`) | **done**: 86/86 cases identical |
| mux behaviour | unclean path → 307, `/docs` → `/docs/`, 405 JSON with `Allow` from the catch-all | **done** |
| docs (raw) | `/llms.txt`, `/llms-full.txt`, `/docs/BEATSCRIPT.md`, `/docs/cheatsheet.md` (file, else the engine's), `/docs/api.md` (docs/docs.go) | **done**: identical; cheat-sheet fallback is `beatscript_api::cheatsheet()` in-process (Go asks a `btwd` child, the CLI's old name) |
| docs (HTML) | `/docs/`, `/docs/cheatsheet`, `/docs/language`, `/docs/api`, `/docs/assets/docs.{css,js}` (docs/html.go, **feat/docs-html**, goldmark) | **done**: byte-identical on the three shipped documents (pulldown-cmark events written out as goldmark writes them) |
| examples | `GET /api/examples`, `/examples/<name>.beatscript` (index.json order, checked at start) | **done**: 18/18 identical |
| health | `GET /healthz` | **done** (prototype) |
| render | `GET/HEAD /api/render/<cid>[.<packed>]?format=opus\|wav&sr=&v=`, r2 cache names, ETag with version, 429 busy / rate limits, singleflight, 10 min wall, LRU (server/render.go) | **done**: WAV byte-identical (streamed, cached, Range, HEAD) · **differs**: Opus is its own render version `r2rs` |
| render status | `GET /api/render/<cid>/status` (URLs carry `v=`) | **done** · **differs**: the Opus URL says `v=r2rs` |
| render limits | `-render-concurrency`, `-render-ip-hour`, `-render-max-bars/-seconds`, `-render-cache-mb`, `.part` sweep | **done** (render-cutover branch, merged here); plus `-render-workers` EDF pool and `-render-admit-lag-ms` |
| site identity | `/api/info`, `/.well-known/btw-site`, `/.well-known/assetlinks.json`, `site.json` | missing (stateless; P2) |
| instruments | `GET /api/instruments` (check-ip-min limiter) | missing (`beatscript_api::instruments_json` ready; P2) |
| linked data | `/context/v1.jsonld`, `/context.jsonld`, `/context` (+ OPTIONS) (ld/) | missing (static bytes; P2) |
| programs read | `GET /api/programs/<cid>[.beatscript]` (ETag, examples first) | missing (`Store::get` exists; P2) |
| checker | `POST /api/check` incl. **same_as / diff** (`beatscript-api` `same.rs`, `diff.rs`, on main), `/api/fmt`, `/api/share`, `/api/open` (`btwd` children, 10 s, per-address budget) | missing (P3; needs the check budget, SEC-2) |
| QR | `POST /api/qr` (go-qrcode, level L) | missing (P3) |
| programs write | `PUT /api/programs/<cid>` (put-ip-hour, store-max-mb, first meta wins, link(2)) | partial: `put_canonical` for `<cid>.<packed>` renders; route missing (P3) |
| auth | `GET /api/auth/challenge`, `POST login/logout`, `GET me`, `POST/DELETE token`; cookie `btw_session` (HMAC, `session.secret`), `tokens.json` (sha256), EIP-191 + ed25519, nonces in memory (auth/) | missing (P4) · **branch**: cookie, bearer and `BtwDevice` *verification* in `feat/btwserve-playlists` |
| shares | `PUT /api/shares/<cid>` (cookie, bearer or `BtwDevice`), share-hour | **branch** `feat/btwserve-cards` (`cards.rs`): contract group `cards`, identical |
| cards | `/p/<cid>`, `/c/<cid>[.<packed>][.jsonld]`, `/card/<cid>.png`, `.square.png`, `/card/default.png`, `/card/go.js`, PNG LRU 256 MiB (card/, 2 300 lines, gg + Go Mono) | **branch** `feat/btwserve-cards`: pages, JSON-LD, headers byte-identical (the page's packed link is Go's own flate bytes, `cards/goflate.rs`) · **differs**: PNG bytes (tiny-skia + ab_glyph; perceptual, mean diff <= 0.053/255 measured, threshold 0.15) |
| pairing | `GET /pair`, `POST /api/pair`, `GET /api/pair/<id>`, `POST /api/pair/claim` (signed claims `btw-pair/1`/`/2`, single use, ±2 min), `devices.json` (server/pair.go) | **done** (P6, `src/hub.rs`, `hub/pair.rs`): contract group `hub`; the pairing QR PNG byte-identical to go-qrcode + image/png + compress/flate (`hub/qr.rs`, `cards::goflate`) |
| devices | `GET/DELETE /api/devices[/<id>]`, `POST /api/devices/<id>/push`, `GET/DELETE /api/device`, `GET /api/device/events` (SSE), `POST /api/device/report`, `/reply`, `/compose` | **done** (P6); `/api/device/compose` through P7's `compose::get()` |
| sessions | `POST /api/session`, `GET /api/session/<id>/events` (SSE), `report`, `reply`; the hub (128 MiB report budget, 40 streams per address) | **done** (P6, `hub/state.rs`): SSE framing identical; `-sse-ping-ms` (beatscript-serve) sets the 20 s ping |
| compose | `POST /api/compose` (signed in), Gemini, repair rounds, compose-user-hour/day, compose-ip-day, daily spend cap in USD (reserve/settle, `spend/`), kill switch (`compose.off`, `BEATSCRIPT_COMPOSE_ENABLED`), JSONL log, `current`/`base_cid` (compose/, limits/) | missing (P7) |
| admin | `POST /api/admin/compose`, `GET /api/admin/stats` (`BEATSCRIPT_ADMIN_TOKEN`) | missing (P7) |
| MCP | `POST /mcp` (+ landing GET), streamable HTTP, 15 tools: compose, check (same_as), fmt, eval, load, get, share, open, render, record, list_examples, instruments, stop, devices, queue (beats-bitwrap-io **feat/mcp-up-next**, not yet on main); two limiters; `tools/list` pinned by `btwsite/testdata/mcp/tools-list.json` (mcpsrv/) | missing (P7, with the hub) |
| up-next queue | MCP `queue` (list/add/remove/clear/skip) and `get`'s `up_next`; page side `GET /api/session/<id>/queue`, `POST /api/session/<id>/queue/next {reason, presence}`, SSE `event: queue`; `presence` in every report. Per hub session, in memory, gone with the session. Server-decided presence rules: pause at a track boundary on `audio_suspended`, `idle_hidden` (`-queue-idle-minutes` 30), `max_auto_advance` (`-queue-max-auto` 8), `no_listener_report`; sticky until an interaction newer than the pause. Caps `-queue-max-items` 50, `-queue-max-per-add` 20, 256 KiB link text, `-queue-writes-min` 30. Never composes, renders or runs background work (server/queue.go, mcpsrv/queue.go, btwsite/DESIGN.md "Up next") | **done** (P6): `queueState` rules ported with Go's queue_test.go; MCP reaches it through `hub::mcp_api` (`mcp::hubapi::HubApi`) |
| playlists | `/api/playlists*` (new; Go has none) | **branch** `feat/btwserve-playlists` (60fb7fb), on Go's account files, read-only |
| cross-cutting | security headers + CSP, `publicRead` CORS, license headers, client IP (`X-Real-IP` behind loopback, IPv6 /64), recoverer (panic → 500 JSON) | **done**, except the recoverer: a panicking handler drops the connection in Rust (§5) |
| access log | JSONL `access-YYYYMMDD.jsonl`, HMAC'd addresses, 30 days, 100 MiB/day | **done** for Rust-served routes (render-cutover); proxied requests are logged by Go only |
| config / env | every flag also `BEATSCRIPT_<NAME>` (`BTW_<NAME>` before the rename, still read after it); `BEATSCRIPT_SESSION_SECRET`, `BEATSCRIPT_ADMIN_TOKEN`, `GEMINI_API_KEY`, `-public-url`, `-alt-origins`, `-legacy-hosts`, `-moved-to`, `-site-id` | same convention; beatscript-serve reads the shared env file. `-alt-origins`/`-legacy-hosts`/`-moved-to` arrive with P2/P4 |
| data dirs | `programs/`, `render/`, `log/`, `cards/`, `spend/`, `session.secret`, `tokens.json`, `devices.json`, `site.json`, `compose.off` | see §4 |
| webhooks | none: btwsite is deployed by `btwsite/deploy/deploy.sh` from its own checkout (`~/Workspace/btw-bitwrap-io`), not by the main-push webhook | beatscript-serve ships as a built binary as the CLI did (§6) |
| metrics | no metrics endpoint; the access log, `/api/admin/stats` (compose) and the render status route are what there is | unchanged; `/healthz` is the probe |

## 3. Contract tests: the same requests to both servers

`crates/beatscript-serve/bench/contract.py` sends each case to Go and to beatscript-serve over a raw socket and compares the
status line and the whole header block as written (order, case, values; `Date` and `Connection` ignored), plus the
body by SHA-256. A multipart boundary, which is random on both sides, is replaced before the comparison. A
difference fails the run unless it is an accepted one (§5), and an accepted difference is checked exactly. For
example, the status JSON must equal Go's once `v=r2rs` is undone, Content-Length included.

```
contract.py --go 127.0.0.1:28097 --rs 127.0.0.1:28098 --web btwsite/web --examples btwsite/examples \
            [--groups static,docs,examples,fallback,render] [--render-limit 3] [--out report.json]
```

- **static**: every file under `-web` (GET), the page routes, directories, HEAD, all methods on `/`, dot segments,
  traversal, Range / multi-range / If-None-Match / If-Range / If-Match, CORS preflight.
- **docs**: all 11 routes plus misses (`/docs`, `/docs/nope`, `/docs/assets/`), HEAD, POST, 304, CORS, Range.
- **examples**: the list, every example file, the misses and wrong methods.
- **fallback**: Go routes asked of beatscript-serve (info, instruments, check, fmt, programs, MCP, pair, card, context,
  SSE, preflights): the answer must be Go's own.
- **render**: per example, WAV at 44.1 and 48 kHz, streamed cold, then cached, then a Range and a HEAD, all
  byte-identical, plus the status JSON. Opus is reported: Go's size, beatscript-serve's size, and whether beatscript-serve's streamed
  and cached Opus are the same bytes.

`bench/parity.py` (from the render-cutover branch) is the deeper render diff: the whole store plus examples, ten
Range and conditional variants, refusals, limits (429), decoded-PCM SNR for Opus, and a `shadow` mode against the live
Go cache. Each later phase adds its group to `contract.py` and reuses the Go tests' fixtures:
`btwsite/testdata/pair/vectors{,-v2}.json` (P4/P6), `btwsite/testdata/mcp/tools-list.json` (P7, byte for byte),
`btwsite/testdata/compose/*.json` as the fake model (P7), `btwsite/testdata/cards/golden.json` (P5, perceptual).

**Result on the ops host, 2026-11-20** (Go = `origin/main` f1862c4 merged with `feat/docs-html` 9c0daeb, with `btwd` and
ffmpeg 8; beatscript-serve = this branch; same `-web`, `-examples`, `-docs`, separate data dirs):

| group | cases | identical | accepted | failed |
|---|---|---|---|---|
| static | 86 | 86 | 0 | 0 |
| docs | 63 | 63 | 0 | 0 |
| examples | 18 | 18 | 0 | 0 |
| fallback | 18 | 18 | 0 | 0 |
| render (all 8 examples, `--render-limit 8`) | 80 | 64 | 16 | 0 |
| **total** | **265** | **249** | **16** | **0** |

Every WAV case is identical: 8 programs × 2 rates × (streamed cold, cached, Range, HEAD). The 16 accepted render
cases are 8 status answers (the Opus URL says `v=r2rs`, and they are exact otherwise) and 8 Opus reports. In all eight,
beatscript-serve's streamed and cached Opus are the same bytes, and its Opus is about 20% smaller than Go's (for example
1.36 MB against 1.72 MB for the same program). The run takes about ten minutes, nearly all of it rendering.

## 4. Data compatibility: same files, no migration

Both processes run with the same `-data` and the same env file. Nothing is migrated, and rollback never has to
convert anything.

- `programs/<cid>.json`: read by both, in Go's shape. beatscript-serve writes only what Go would for an anonymous PUT (a
  `<cid>.<packed>` render), with link(2) so a race stores once. First meta wins in both.
- `render/`: shared. WAV is `<cid>.<sr>.r2.wav` + `.meta` in both (same bytes, so either serves the other's file).
  beatscript-serve's Opus is `<cid>.<sr>.r2rs.opus`. Go's start-up sweep (`(?:r\d+\.)?`) neither counts nor deletes those, and
  beatscript-serve does not count Go's `r2.opus`. Parts are `*.rs.part` (Rust) and `*.part` (Go). Go deletes every `.part`
  at start, which can fail a Rust render in flight (it is retried). That stops when Go no longer renders. Two LRUs
  over the shared WAV files means the smaller cap wins. Accepted while both render.
- `log/`: **give beatscript-serve its own directory during the split** (`-log-dir data/log-rs`). Both write
  `access-YYYYMMDD.jsonl` with buffered appends, and two processes appending to one file can interleave lines.
  Proxied requests are logged by Go only, so nothing is counted twice.
- `session.secret`, `tokens.json`, `devices.json`, `site.json`: read-only for beatscript-serve until the group that writes them
  moves (P4/P6). They are re-read on size/mtime change (the playlists branch already does this).
- `cards/`, `spend/`, `compose.off`: Go's until P5/P7, then the same files.
- `playlists/`: beatscript-serve's alone (new).

## 5. Accepted differences (each is checked by the contract test, not waived)

1. **Opus bytes.** Go pipes 44.1 kHz PCM through ffmpeg (resample to 48 kHz, system libopus). beatscript-serve renders at
   48 kHz and encodes with libopus 1.3.1 in-process. The two cannot be byte-identical, so beatscript-serve's Opus is its own
   render version `r2rs`: its own file name, its own ETag (`"<cid>-<sr>-r2rs-opus"`), and `v=r2rs` in the status URL.
   No (CID, format, version) ever has two byte streams. beatscript-serve's Opus is deterministic (fixed Ogg serial from the
   CID; streamed == cached == a second render). An `If-Range` carrying a Go Opus ETag fails safe to a full answer.
   At the render cutover, set `RENDER_VERSION` in `site/web/js/api.js` and `api.RenderVersion` to a value that
   distinguishes Opus (or give the page a per-format version), so browser-cached Go Opus at `?v=r2` is not mixed with
   Rust Opus under one URL. If byte-identical Opus with Go were ever required, the only route is ffmpeg as a child
   again, which this plan rejects.
2. **Panics.** Go's recoverer answers 500 JSON. A panic in an axum handler drops the connection. Render jobs already
   catch engine panics (41b7286). Add `tower_http::catch_panic` with Go's body before P3.
3. **Access-log address.** beatscript-serve hashes the client (`X-Real-IP`), where Go hashes the TCP peer (always 127.0.0.1
   behind nginx), and it never logs a packed program.
4. **The HTML docs renderer** is proven byte-identical on the three documents the site serves, not on arbitrary
   markdown. A doc that uses a construct the writer has not met (footnotes, task lists, raw HTML inside a list item)
   may differ, and the docs group of the contract test is the gate for every docs change.

## 6. Cutover and rollback

The kit was `crates/btwserve/deploy/` (since replaced by `deploy/deploy.sh`); every step is one command of `deploy/cutover.sh` run on the ops host (it acts on
pflow.dev over ssh). The first plan used port 18098, but that port is taken on pflow.dev (open-ontologies, behind
ontology.pflow.xyz), so beatscript-serve listens on **127.0.0.1:18099**. That port was checked free on pflow.dev and is
claimed by no console manifest.

1. Build: `BEATSCRIPT_PKG=beatscript-serve scripts/build-linux.sh` (cross-compiled with cargo-zigbuild for glibc 2.39 on the build host, Mac or Linux, no docker) → `dist/beatscript-serve-linux-x86_64`. Run on
   the ops host it builds on the Mac over ssh and copies the binary back: the ops host only orchestrates.
2. `cutover.sh preflight`: read-only checks (port, binary, env file, both /healthz, `nginx -t`).
3. `cutover.sh install`: copies the binary to `btwsite/bin/btwserve`, adds the `btw-rust` entry to `~/services`
   (backup first; the command runs under a `systemd-run --user --scope` with MemoryHigh 1.5G / MemoryMax 2G, using the
   same env file as Go through `~/.env-services/btw-rust` → `btw-bitwrap`), starts it and checks `/healthz` on :18099.
   `deploy/btw-rust.service` is the systemd user unit alternative (use one or the other). `deploy/console-app-btw-rust.yaml`
   is the draft console manifest.
4. `cutover.sh prep-nginx` (one time, no traffic change): writes `/etc/nginx/conf.d/btw-front.conf`
   (`upstream btw_front { server 127.0.0.1:18097; keepalive 16; }`) and makes every `proxy_pass http://127.0.0.1:18097;`
   in `beatscript.app` and `btw.bitwrap.io` into `proxy_pass http://btw_front;`. Each location keeps its own settings.
   It runs `nginx -t` and restores the vhosts if that fails, then reloads and commits in the private sites-available repo.
5. `cutover.sh flip`: the upstream goes to `127.0.0.1:18099`, then `nginx -t` and a reload. It refuses if beatscript-serve is
   not healthy.
6. **Rollback: `cutover.sh rollback`**. The upstream goes back to :18097, then `nginx -t`, a reload, and
   `~/services restart btw-bitwrap`, so Go re-reads `tokens.json` and `devices.json` (it reads them only at start)
   after beatscript-serve wrote them.

Go stays installed and running, idle (no traffic), for rollback. Both servers use the same `data/`, so there is
nothing to migrate either way. beatscript-serve's access log is `data/log-rs`. Watch for a day: `/healthz`, 5xx in
`data/log-rs`, render status, compose spend.

## 7. Next phases (proposed order)

Each phase moves one group into beatscript-serve's owned set (`proxy::go_owned` loses its paths), adds its contract group,
and ships with no nginx change.

1. **P2, read-only public routes**: `/api/info`, `/.well-known/btw-site`, `/.well-known/assetlinks.json`,
   `/api/instruments`, `GET /api/programs/<cid>[.beatscript]`, `/context*`. Stateless, in-process, cheap. They bring
   `site.json` and the `-alt-origins` / `-legacy-hosts` / `-moved-to` flags.
2. **Merge `feat/btwserve-playlists`** here (additive: `/api/playlists*` added to the owned set ahead of the
   `/api/` fallback). It is the first Rust-only route.
3. **P3, the checker and store writes**: `POST /api/check` (with `same_as` / diff), `/api/fmt`, `/api/share`,
   `/api/open`, `/api/qr`, `PUT /api/programs/<cid>`. First the untrusted-check budget (SEC-2: a step/time budget in
   `djlive::compile::check_with`, or a `beatscript` child for link text until then) and the per-address checker budget.
   Removes a fork+exec per request, the biggest latency of these routes today.
4. **P4, auth**: challenge + login together (in-memory nonces), logout, me, tokens. Port the SEC-1 tests first.
   beatscript-serve then owns `session.secret` and `tokens.json` writes.
5. **P5, shares and cards**: `PUT /api/shares`, `/p/<cid>`, `/c/*`, `/card/*` (tiny-skia + ab_glyph + Go Mono,
   perceptual parity against `btwsite/testdata/cards/golden.json`).
6. **P6 + P7 together, the hub**: sessions (with the up-next queue and its presence rules), pairing, devices (SSE), then MCP (`rmcp`, `tools/list` byte for byte)
   and compose + admin (Gemini over reqwest/rustls, limits, spend cap, kill switch). They share the in-memory hub, so
   they move in one release.
7. **P8, retire Go** (in progress 2026-11-22: beats-bitwrap-io branch `retire-go-server` removes the Go server and
   its deploy steps; on pflow.dev `deploy.sh retire-go` stops `btw-bitwrap` and drops its `~/services` entry, and
   `cutover.sh rollback` then refuses): `-fallback` is removed, `btw-bitwrap` stops, and then delete `cmd/btwsite`,
   `internal/btwsite/**` (about 15 500 lines plus tests) and the Go-only testdata harnesses in beats-bitwrap-io. `btwd`
   is no longer deployed (its code lives on in beatscript-serve), and so go `btwsite/deploy/deploy.sh`'s btwd/ffmpeg steps
   and the ffmpeg dependency on pflow.dev for btwsite. `btwsite/web`, the examples and the testdata fixtures stay
   public in beats-bitwrap-io. The console manifest `site/deploy/console-app-btw-bitwrap.yaml` is replaced by a
   `btw-rust` one.

8. **P9, x402 paid compose** (proposed; follows P6 + P7 and the signed capability grant). Anyone can buy access to
   build a track with an HTTP 402 / x402 payment (the public x402 spec) instead of signing in or holding a `compose`
   capability grant. `/api/compose` (and an MCP generate/compose path, if any) answers 402 with x402 payment
   requirements: price, asset, network, `payTo`. The client retries with an `X-PAYMENT` header, and the server
   verifies it (or uses a facilitator) before calling the model. A paid build lets the buyer choose the track's
   license (an SPDX id from an allowed list), stored with the program's rights instead of the CC0-1.0 default.

   Decided: the price is about $0.10 per build, and the build is guaranteed valid. The buyer pays for a program that
   passes `check`. The server retries the model and runs the deterministic fixer until it gets a valid file, and
   settles the payment only after that. If it cannot produce a valid program it does not settle, so the buyer is not
   charged. Failed attempts are still kept and stored, as always, but are not billed.

   Open questions:
   - Whether a non-CC0 paid track is still public-stored or private by default.
   - Which networks and assets are accepted, and whether to use a facilitator or verify directly.
   - Refunds or idempotency on a retried `X-PAYMENT`, and per-payer rate limits.
   - How it interacts with the signed capability grant (`btwserve-oauth` is building it now): a payment can mint a
     short-lived single-build grant.

Efficiency work that is independent of the order (from the earlier survey, §9 there): system libopus behind a
feature (SIMD), one render feeding both formats, pre-render on store, Range on a growing file, engine profiling.

## 8. Related branches (merge order)

- `plan/rust-render-server` (41b7286) ⊂ `val/render-cutover-877941d9` (38a600f, parity.py, EDF pool, zero-copy hits,
  redacted log; its run ended unfinished, and its worktree still holds an uncommitted README edit and, in
  beats-bitwrap-io, uncommitted deploy drafts: `btwsite/deploy/RENDER-CUTOVER.md`, `btw-rust.service`, nginx
  upstream) ⊂ **`feat/btwserve-phase1`** (this work, with `origin/main` merged: render-levels r2, same-as/diff,
  parity).
- `feat/btwserve-playlists` (60fb7fb) branches from 41b7286: merge it into `feat/btwserve-phase1` after this lands
  (expect conflicts only in `main.rs`'s router and config).
- beats-bitwrap-io: `feat/docs-html` is what beatscript-serve's `/docs/*` HTML matches. `feat/btw-pattern-sugar` touches the
  language, not the server: regenerate `BEATSCRIPT.md` (then `BTW.md`) / the cheat-sheet and re-run the docs group when it lands.
