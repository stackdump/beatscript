# BeatScript listening tests

`beatscript listen` (crates/beatscript-cli/src/listen.rs) renders a program offline through the site's engine and prints a small JSON
summary: loudness per deck, part and section; every deck or part that has notes and is not heard, with the reason when
the mixer gives one; the tempo and key heard next to the declared ones; clipping. The MCP `listen` tool on
beatscript.app answers the same summary (crates/beatscript-serve/src/mcp/listen.rs).

These tests hold that summary still for every example the site lists and for the fixtures here:

| file | what it guards |
|---|---|
| `fixtures/silent-crossfader.beatscript` | the crossfader all the way to A shuts deck B (BeatScript Android 0.3.40 started its Player there) |
| `fixtures/silent-mute.beatscript` | a deck muted and never put back |
| `fixtures/filter-exit-restores.beatscript` | a `filter_exit` move mutes its deck and the program puts it back at the next phrase line (djapp left it muted) |
| `goldens/<name>.json` | the summary of each program as last accepted (since `listen/2` with the mix: bands, true peak, clashes, targets; docs/beatscript/polish.md) |

A fixture's `# expect:` lines (`silent deck=B reason=crossfader`, `heard deck=B section=later`) are checked on every run,
so regenerating the goldens cannot hide the bug a fixture exists for.

## Running

```bash
# against a beatscript you built (the site examples are site/examples)
python3 tools/listen/listen_ci.py --beatscript target/release/beatscript --examples site/examples

# accept a change (an example was edited, the engine changed on purpose): rewrite the goldens, then commit them
python3 tools/listen/listen_ci.py --beatscript target/release/beatscript --examples site/examples --update

```

A run fails (exit 1) when a deck or part goes silent or comes back, a loudness moves more than 1 LU, the tempo or key
heard changes, or the master starts clipping; it is STALE when an example's CID changed (run `--update`).
`--publish` posts a `ListenRun` event (`_source: beatscript-listen`) on the event bus (LISTEN_BUS_SOCK, a Unix datagram socket) and writes
`LISTEN_LAST` (default `data/listen/last.json`).

The val job is `beatscript-listen` (objective script, nightly at 01:40, `notify: failures`; `val_schedule run
beatscript-listen` for one now). It runs `nightly.sh` as it is on origin/main.
