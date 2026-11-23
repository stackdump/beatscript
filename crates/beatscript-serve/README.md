# beatscript-serve: beatscript.app in Rust (phase 1: render, static site, docs, examples, Go fallback)

The plan of record is `docs/RUST-SERVER.md` in this repo. beatscript-serve replaces the Go btwsite server
(`stackdump/beats-bitwrap-io`, `cmd/btwsite` + `internal/btwsite`) route group by route group. Every route it does not
own yet goes to the Go server through `-fallback` (the strangler), so nginx has one target.

The render route runs beatscript's own `render_to` (included by path) on a thread pool, and its PCM goes straight into the WAV
file or into an in-process Ogg Opus encoder. There is no `beatscript` child and no `ffmpeg` child.

```
cargo build --release -p beatscript-serve
target/release/beatscript-serve -addr 127.0.0.1:18098 -data <the Go server's -data> -log-dir <data>/log-rs \
    -web btwsite/web -examples btwsite/examples -docs site/web/docs -docs-src internal/btwsite/docs \
    -public-url https://beatscript.app -fallback http://127.0.0.1:18097
```

Routes served here: `GET|HEAD /api/render/<cid>[.<packed>]?format=opus|wav&sr=44100|48000[&v=]`,
`/api/render/<cid>/status`, `/healthz`, `/api/examples`, `/examples/<name>.beatscript`, `/llms.txt`, `/llms-full.txt`,
`/docs/{BeatScript,cheatsheet,api}.md`, `/docs/` + `/docs/{cheatsheet,language,api}` (HTML) + `/docs/assets/*`, and the static
site for every path no Go route takes. The rest goes to `-fallback`, or is a JSON 404 without it.

`bench/contract.py` sends the same requests to both servers and compares status, headers and bodies (results in
`docs/RUST-SERVER.md` §3). The sections below are the render route's, from the prototype.

## What is the same as Go

- **Data on disk**: `<data>/programs/<cid>.json` (read and, for `<cid>.<packed>`, written in Go's shape), and
  `<data>/render/<cid>.<sr>.r2.wav` + `.meta`, Go's names, so either server serves the other's WAV. Opus is
  `<cid>.<sr>.r2rs.opus`: another encoder, so another render version (RUST-SERVER.md §5). Parts are `*.rs.part`.
- **CID check**: the store re-hashes the text (`djlive::compile::program_cid`, what Go's `btw.ProgramCID` mirrors);
  `<cid>.<packed>` is opened with `beatscript_api::open_json` (what `beatscript open` prints), refused on a mismatch (422
  `cid_mismatch`), and stored like an anonymous `PUT /api/programs/<cid>`, with the per-address limit.
- **Responses**: status codes, error JSON (`{"error":{...}}` + newline, Go's `<>&` escaping), and every header
  (security, CSP, license `Link`s, CORS for `Origin`, `X-Btw-*`, `ETag`, `Range`/`If-None-Match`, 416 as Go 1.23+
  sends it), sorted and title-cased as Go writes them. `bench/compare.sh` diffs both servers: **23 of 24 cases
  identical** in status, headers and body; the 24th is the Opus audio itself.
- **WAV bytes**: identical to Go's, streamed and cached (same engine, same PCM), checked locally and on pflow.dev.
- **Jobs**: singleflight (`X-Btw-Cache: joined`, joiners read from byte 0), the render stops when its last follower
  leaves, `-render-concurrency` slots (429 `busy` with Go's `Retry-After`), `-render-ip-hour`, LRU at
  `-render-cache-mb`, 10-minute wall limit, a failed render aborts the connection. The threads are reniced to 10.

## What differs, on purpose

- **Opus is encoded in-process**: libopus 1.3.1 transpiled to Rust (`unsafe-libopus`, BSD-3-Clause, no C toolchain)
  in a pure-Rust Ogg muxer (`ogg`, BSD-3-Clause). 96 kb/s VBR, music, 20 ms frames, a page per 200 ms like ffmpeg's
  `-page_duration 200000`, but the two header pages leave before any audio and the first audio page after 60 ms.
- **The engine renders Opus at 48 kHz** (Opus has no 44.1 kHz mode; ffmpeg resampled). `sr` keeps naming the cache
  file and is written into the OpusHead `input_sample_rate`, as ffmpeg did. Decoded, it lines up with a 48 kHz WAV
  render at lag 0 with 18.4 dB waveform SNR (ffmpeg's: 15.4 dB, partly the resample).
- **The Opus bytes are reproducible** (fixed stream serial from the CID; libopus is deterministic): serial and
  pipelined encoders gave the same file.

## Measured (cold renders, one request at a time; `bench/bench.py`: TTFB, total, CPU incl. reaped children, peak RSS of the process tree)

A Linux box (i7-4790, 8 threads), median of 3, Go = btwsite + this tree's beatscript + ffmpeg 8.0:

| program (audio) | format | server | TTFB | total | CPU | peak RSS |
|---|---|---|---|---|---|---|
| hand-dubstep-140-v4 (116.6 s) | opus | Go | 486 ms | 8.20 s | 9.9 s | 168 MB |
| | opus | Rust | **219 ms** | 8.96 s | 10.6 s | **111 MB** |
| | wav | Go | 209 ms | 8.24 s | 8.3 s | 119 MB |
| | wav | Rust | 225 ms | 8.04 s | 8.1 s | 110 MB |
| trance-138 (111.3 s) | opus | Go | 696 ms | 12.04 s | 13.1 s | 196 MB |
| | opus | Rust | **457 ms** | 11.58 s | 13.2 s | **148 MB** |
| | wav | Go | 395 ms | 10.28 s | 10.4 s | 146 MB |
| | wav | Rust | 441 ms | 10.54 s | 10.6 s | 147 MB |

pflow.dev (Xeon 8259CL, 2 vCPU), against the **live** Go process on 127.0.0.1:18097 (beatscript 0.1.0, ffmpeg 6.1.1),
beatscript-serve run once from /tmp on its own data dir; Opus, a fresh CID per program (`<cid>.<packed>`), n = 1 each:

| program | server | TTFB | total | CPU | peak RSS |
|---|---|---|---|---|---|
| trance @137 | Go live | 841 ms | 17.5 s | 20.6 s | 191 MB |
| | Rust | 781 ms | 18.3 s | 21.4 s | 137 MB |
| trance @136 | Go live | 850 ms | 16.5 s | 19.3 s | 192 MB |
| | Rust | 782 ms | 19.4 s | 22.8 s | 137 MB |
| dubstep @141 | Go live | 507 ms | 12.6 s | 15.5 s | 165 MB |
| | Rust | 351 ms | 14.6 s | 17.5 s | 100 MB |
| dubstep @139 | Go live | 482 ms | 12.6 s | 15.9 s | 165 MB |
| | Rust | 412 ms | 16.8 s | 18.5 s | 100 MB |

On that box the same `beatscript render` of one program measured 11.1 to 14.4 s of CPU across three runs, so these single
samples are good to about ±20%; the trend (TTFB and memory better, CPU a little worse) is what holds. Where the CPU
goes: the engine is about 85% of it, and the 48 kHz Opus render is about 9% more engine work than 44.1 kHz. The
transpiled libopus has no SIMD (ffmpeg's system libopus has SSE/AVX). The obvious next steps are in the plan.

## Licensing

This crate is MIT, like the rest of stackdump/beatscript (LICENSE, REUSE.toml). It links djhost for its ontology
vocabulary only; djhost's experimental `windwheel` and booth `wallet` features are off by default, so neither is in
its tree (scripts/check-public-build.sh). The engine is stackdump/djrust, a git dependency pinned in Cargo.toml. The
new third-party crates are MIT / Apache-2.0 / BSD-3-Clause (unsafe-libopus carries libopus's BSD notice: keep it in any
binary distribution's notices).
