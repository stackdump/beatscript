# `beatscript render`: the streaming renderer

A BeatScript program in, raw interleaved stereo PCM out, **as it renders**. It is the offline engine of
`djctl btw render` (`docs/HEADLESS.md`) with the file taken out: each 128-frame block of the master is
converted and appended to a chunk, and a full chunk is written to stdout and flushed at once. The first bytes
leave after the engine has started and about 60 ms of audio is done, not after the song.

```
beatscript render [--file F | -] [--sr 44100] [--format s16le|f32le] [--chunk-ms 250]
            [--max-bars 256] [--from-bar 0]
```

The program text is on stdin (or `--file F`). PCM goes to stdout, one JSON object a line to stderr. Exit 0:
rendered (or the reader closed stdout: it asked for that, no line). Exit 1: one `{"event":"error",...}` line,
nothing on stdout. No disk, no network.

## What is rendered

- **Checked first**, with the real control registry and instrument catalogue and no snippet store (`import`
  of a saved snippet does not resolve: the server hands over the full text). A program with errors, one that
  `uses_assets` (`sample`, `loop "..." as`) or is `restricted` (`use restricted`, `#! visibility: private`,
  a restricted asset) is **refused**: `error` with `code` `check_failed` / `uses_assets` / `restricted`.
- **Length**: an arranged song is its timeline once (a `loop`ing song is one pass: the start event's `notes`
  say so); a program with `end at bar N` is N bars; a program with no end is **16 bars**. All capped at
  `--max-bars` (default 256; the cap is on the *end bar*, so `--from-bar` does not extend it: a seek into a long
  program costs the render of the bars before it). The song ends where its last bar ends: **no tail** is added
  (the last bar's sound is cut at the bar line, exactly as `djctl btw render` cuts it).
- **Control lines are rendered as the browser plays them** (since 2026-11-24, render version `r3`; `src/browser.rs`). The page
  (beatscript.app) runs the live runtime and turns each registry call into ops on djrust's wasm facade (`Dj`), which is a thin
  wrapper over the very `Engine` this render steps; `browser.rs` is that mapping (site/web/js/engine-calls.js `mapCall`) in
  Rust, held to the JS op for op by `tests/browser_calls.rs`, so a line the page plays is played here by the same engine call:
  - the mixer and the transport (deck and master `level`, EQ, mute, crossfader, `play` / `stop`, `duck`, `mix`, plain values,
    ramps of every shape and lfos, sampled and thinned the way the runtime sends them: `beatscript_render::control_events`);
  - every other call on the same timeline (`beatscript_render::control_calls`): the insert filter and its resonance (the
    `inserts` live op: the booth's filter), reverb as the page has it (each deck strip's reverb wet; the master's is both
    decks'), the master crush as the page has it (the decks' master FX crush bits), the FX unit's drive, the vocoder (`vocoder`
    live op) and **the Wheel** (beatscript-api `wheel`, the very voice the page plays: wound and played on the same calls, a
    128-frame quantum at a time, added to the master after the decks with the page's master reverb, crush and level);
  - `kit=` / `sound=` (`setKit` / `setRoleVoice`, right after each section's nets are loaded);
  - every `move` line on the booth's scheduler (`djhost::offline_moves`, which the page's `MoveScheduler` ports), serviced
    every block on the render's clock, so a move starts on the line after the bar line that armed it, as on the page, and a
    `vocoder_handoff` swaps the vocoder;
  - and, where the page has no op yet, the booth's way: a delay, a crush on a deck or the filter's mix on the strip's insert
    chain, a `macro` line (the control net's `effects fire` / `release`) and a `duck` (the `trick` pump).

  Timing is honoured: a section's lines fire on its bar line, `at` / `every` / `for N bars` on theirs (the page acts within
  a pass of its 25 ms loop; the render on the exact frame). A line none of these plays (a `do` call to another noun, a
  timed or performed vocal, a phrase sync's seed step) is named in the start event's `notes` (`not in this render: ...`, a
  move with why). The whole program's control timeline is built before the first chunk, so `--from-bar` and `--max-bars`
  hear exactly what the full render does up to there. Not followed: a tempo curve (the render runs at the program's first
  tempo). How close the render is to the page, program by program: site/PARITY.md.
- **Sound**: the engine djapp plays on (`EngineConfig::extended`), deterministic (composes inline, no
  background threads), at `--sr`.

## stderr: JSON lines

```
{"event":"start","sample_rate":44100,"channels":2,"format":"s16le","seconds":109.714,"bars":64,"bpm":140,
 "from_bar":0,"total_bars":64,"cid":"z4EBG9j...","notes":["control lines (...) are not rendered ..."]}
{"event":"progress","seconds_rendered":3.199}                 // at most 4 a second
{"event":"end","seconds":109.714,"frames":4838400,"check_ms":7.4,"ready_ms":52.1,"ttfb_ms":57.0,"render_ms":2104.3}
{"event":"error","code":"check_failed","message":"the program has 1 error: error: ... --> 3:1 ..."}
```

`seconds` / `bars` in `start` are what will be **streamed** (after `--from-bar`); `total_bars` is the
program's; `cid` the program CID. In `end`: `frames` streamed, and where the time went in wall milliseconds
since the process began its work: `check_ms` (parse and compile), `ready_ms` (engine built and the first
section's decks loaded), `ttfb_ms` (first byte handed to stdout), `render_ms` (all of it). Progress lines start
after the first chunk and never come faster than 250 ms apart.

Error `code`s: `usage` (a bad flag, `--from-bar` past the end, text over 1 MiB), `io`, `check_failed`,
`uses_assets`, `restricted`, `internal` (a panic: the message says where). A reader that closes stdout ends
the render with exit 0 and no line (`closed`).

## Latency, determinism, equality

**First byte.** Start-up is kept small on purpose. The program is compiled once; the arrangement is built
**lazily** (`Sections`): only the first section's decks are made before the first chunk, and each later
section's nets are built when the render reaches the bar before it (the same events, in the same order, as
`djhost::beatscript_render::arrangement`: a test compares the two whole). The first chunk is at most 60 ms of audio
(`--chunk-ms` governs the rest), so the reader has bytes as soon as the engine is up. Numbers: below.

**Determinism.** The same text and `--sr` give byte-identical PCM. The engine is always stepped from frame 0 in
its own blocks; **chunks are cut from the converted stream**, so `--chunk-ms` cannot change a byte (tested at
1, 64, 250, 1000 and 60000 ms). `--from-bar N` renders the first N bars and drops them: its output is exactly
the tail of the full render (tested), at the price of rendering those bars (about 30 to 50 times faster than
real time).

**Against `djctl btw render`.** For a program's notes and sections (no control lines), the stream is the same samples as the WAV `djctl btw render FILE OUT.wav
--sr 44100` writes (with control lines they differ by design: this render plays them as the page does, djctl as the booth does): it is the same `render_arrangement` loop (same engine config, same block cuts at each
move, same closing event), so the s16 PCM equals the WAV's data chunk **byte for byte** (a test renders the
official arrangement through `render_arrangement` and compares; `f32le` is the same signal before
quantising, and the s16 conversion is `djrust::wav::wav_bytes`'s). The differences are only of defaults and
framing: djctl's default rate is 48 000 Hz (`beatscript`'s 44 100); djctl writes a WAV header and a program with no
end is 16 bars in both; **`beatscript` honours `end at bar N` and `--max-bars`, djctl ignores them** (it renders 16
bars or `--beats`).

## Numbers

`scripts/bench-render.sh -n 5`: a fresh process each run, `ttfb` from spawn to the first byte on stdout,
min / median / max of 5; `x_rt` is audio seconds over wall seconds for the whole song; `rss` the peak resident
set. Release build, 2026-11-18. The contract is **first byte under 1 s, cold**: met for every program on both
machines.

**A Linux box** (Ubuntu, 8 cores, load about 2 to 4; the glibc-2.39 binary `dist/beatscript-linux-x86_64`):

| program | audio s | ttfb ms (min / med / max) | x_rt (med) | rss MB |
|---|---|---|---|---|
| first-beat | 15.5 | 139 / 141 / 150 | 18.8 | 57 |
| acid-line | 29.1 | 123 / 126 / 133 | 22.0 | 74 |
| markers-demo | 46.5 | 109 / 115 / 130 | 20.6 | 78 |
| evolve-demo | 65.1 | 161 / 175 / 253 | 16.2 | 79 |
| hand-dubstep-140-v2 | 109.7 | 155 / 157 / 161 | 18.1 | 93 |
| hand-dubstep-140-v3 | 116.6 | 154 / 161 / 224 | 17.4 | 94 |
| hand-dubstep-140-v4 | 116.6 | 157 / 164 / 258 | 16.9 | 95 |
| trance-138 | 111.3 | 270 / 274 / 341 | 12.1 | 122 |
| matrix-130 | 147.7 | 97 / 103 / 132 | 12.1 | 95 |

The same binary in a bare `ubuntu:24.04` container on that box (nothing installed; `ldd` shows only libc, libm
and libgcc_s; the newest glibc symbol it names is 2.34): markers-demo, first byte 112 ms, 46.5 s of audio in
2.2 s wall.

**This Mac** (M4 Pro, 14 cores, **under a load average of 20 to 30** from the other overnight builds, so the
maxima are noise from contention, not the renderer):

| program | audio s | ttfb ms (min / med / max) | x_rt (med) | rss MB |
|---|---|---|---|---|
| first-beat | 15.5 | 87 / 98 / 108 | 33 | 55 |
| acid-line | 29.1 | 75 / 90 / 117 | 35 | 71 |
| markers-demo | 46.5 | 95 / 107 / 116 | 16 | 73 |
| evolve-demo | 65.1 | 103 / 112 / 114 | 33 | 74 |
| hand-dubstep-140-v2 | 109.7 | 173 / 201 / 331 | 16 | 88 |
| hand-dubstep-140-v3 | 116.6 | 139 / 258 / 668 | 12 | 89 |
| hand-dubstep-140-v4 | 116.6 | 76 / 81 / 145 | 46 | 90 |
| trance-138 | 111.3 | 196 / 226 / 407 | 24 | 124 |
| matrix-130 | 147.7 | 59 / 62 / 86 | 31 | 90 |

**Where the first-byte time goes** (the `end` event's `check_ms`, `engine_ms`, `ready_ms`, `ttfb_ms`): the
check is 5 to 20 ms; building the engine 35 to 65 ms; loading the first section's nets onto the decks 20 to
200 ms (the part that grows with the program: trance-138's first section is the heaviest); the first 60 ms of
audio 2 to 5 ms. Only the first section's nets are built before the first chunk (the rest are built a bar
before they are needed), which is what the lazy `Sections` is for; nothing else in start-up is avoidable
without changing the engine. The rest of the song renders 12 to 45 times faster than real time, so a listener
who starts the stream is ahead of the clock after the first second.

## Tests

```
cargo test -p beatscript-cli                       # render_inproc (12) + render_cli (6) (+ R1's cli tests), about 15 s
scripts/bench-render.sh [-n RUNS] [-b BEATSCRIPT_CLI] [FILES]    # ttfb / total / x real time / peak RSS per program
```

`render_inproc.rs`: determinism, chunk invariance (and the sizes of the writes), the small first chunk, the
event lines, refusals (error, `sample`, `use restricted`, `#! visibility: private`), no-end = 16 bars,
`--max-bars`, `--from-bar` = tail, `f32le` = `s16le`, the lazy arrangement = `beatscript_render::arrangement` for the
corpus, and the stream = `render_arrangement`'s WAV samples. `render_cli.rs`: stdin / `--file`, exit codes
and usage errors, restricted / asset refusals on stderr, the first byte long before the end, a reader that
leaves.

## Linux

`scripts/build-linux.sh` builds `dist/beatscript-linux-x86_64` (+ `.sha256`) for Ubuntu 24.04 (glibc 2.39) on a build
host (`scripts/build-host.sh`): cross-compiled with cargo-zigbuild (Mac or Linux host, no docker) for
`x86_64-unknown-linux-gnu.2.39` and smoked on an x86_64 Linux box. It
refuses a binary that needs a GLIBC symbol version above 2.39; see the script's header.
