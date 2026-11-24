#!/usr/bin/env bash
# render-parity.sh: does the server's render (`beatscript render`: the stream, `listen`, the stems) play what the page plays?
#
#   site/test/parity/render-parity.sh [--beatscript PATH] [run.mjs flags...]
#   PARITY_WASM_DIR=<a built module> site/test/parity/render-parity.sh ...     (the module to hold it to; default site/web/wasm)
#
# Renders the site examples named below both ways (the stream: `beatscript render`; the page: js/engine.js + the djweb wasm in
# headless Chrome) and holds them to the level gate (run.mjs --level-gate): every bar within LUFS_TOL LU and MEL_TOL dB of
# log-mel spectrum, the whole song within INT_TOL LU. A program with a known gap (render-ceilings.json: why, and its own bounds)
# is held to where it is, so it cannot get worse unseen. Needs what run.mjs needs (site/web/wasm/ or PARITY_WASM_DIR, Chrome,
# node, a beatscript binary) and SKIPs (exit 0) without it. Runs on a build host (gir, workbit), never on valoper.
# The examples are the ones with the control lines the page plays: master and deck filters and their ramps, resonance,
# reverb, levels, the Wheel (hand-dubstep-140-v4, matrix-130), kit= (matrix-130), sections and evolve.
set -euo pipefail
cd "$(dirname "$0")/../../.."
EXAMPLES=${RENDER_PARITY_EXAMPLES:-'^(hand-dubstep-140-v4|acid-line|markers-demo|evolve-demo|matrix-130)$'}
exec node site/test/parity/run.mjs --corpus examples --only "$EXAMPLES" --gate '^$' --level-gate '.' \
  --lufs-tol "${LUFS_TOL:-1}" --mel-tol "${MEL_TOL:-3}" --int-tol "${INT_TOL:-0.5}" --level-ceilings site/test/parity/render-ceilings.json \
  --out "${OUT:-site/test/parity/out-render}" "$@"
