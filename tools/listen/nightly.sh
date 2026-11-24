#!/usr/bin/env bash
# BeatScript listening tests on valoper (val_schedule job `beatscript-listen`, nightly and on demand).
#
# Builds the `beatscript` CLI from stackdump/beatscript at LISTEN_REF (default origin/main) in a worktree of its own
# under $LISTEN_CI_ROOT, takes the examples from that same checkout (site/examples), and runs tools/listen/listen_ci.py
# against the goldens: exit 1 when a deck goes silent or loudness, tempo or key drift past tolerance (the job's notice
# and Datadog check say so), with a ListenRun event on the bus and data/listen/last.json.
#
# Builds run on workbit/gir via ssh; valoper only orchestrates: scripts/build-linux.sh cross-compiles the CLI on the
# build host (scripts/build-host.sh) and copies it back to dist/. The build holds $HEAVY_LOCK (flock) and waits for
# MemAvailable >= $MIN_FREE_GB, which only matters when it falls back to valoper. A commit already built is not
# built again, so an on-demand run after the nightly one only listens. LISTEN_BUILD_DIR (default
# beatscript-listen-build) is the build host's scratch dir, apart from the deploy's.
#
#   nightly.sh [--no-fetch] [listen_ci.py args...]
set -euo pipefail
ROOT=${LISTEN_CI_ROOT:-$HOME/.cache/beatscript-listen}
WS=${WORKSPACE:-$HOME/Workspace}
REPO=${LISTEN_REPO:-beatscript}
HEAVY_LOCK=${HEAVY_LOCK:-/tmp/valoper-heavy-build.lock}
MIN_FREE_GB=${MIN_FREE_GB:-8}
REF=${LISTEN_REF:-origin/main}
FETCH=1
if [ "${1:-}" = "--no-fetch" ]; then FETCH=0; shift; fi
mkdir -p "$ROOT"
log() { echo "[listen $(date +%H:%M:%S)] $*" >&2; }

# a detached worktree of $WS/$REPO at $REF, at $ROOT/beatscript
repo=$WS/$REPO dir=$ROOT/beatscript
[ "$FETCH" = 1 ] && git -C "$repo" fetch -q origin
if [ ! -e "$dir/.git" ]; then
  git -C "$repo" worktree prune
  git -C "$repo" worktree add -q --detach "$dir" "$REF"
else
  git -C "$dir" checkout -q --detach "$REF"
fi
eng=$(git -C "$dir" rev-parse --short HEAD)
ex=$dir/site/examples
log "beatscript $eng: $(ls "$ex"/*.beatscript | wc -l) example programs"

bin=$dir/dist/beatscript-linux-x86_64
stamp=$bin.commit
if [ ! -x "$bin" ] || [ "$(cat "$stamp" 2>/dev/null)" != "$eng" ]; then
  exec 9>"$HEAVY_LOCK"
  log "waiting for the heavy-build lock ($HEAVY_LOCK)"
  flock -w 1200 9 || { log "the heavy-build lock was held for 20 minutes: not building"; exit 1; }
  for _ in $(seq 1 40); do   # 20 minutes at most: a val script job is killed at 60
    free_gb=$(awk '/MemAvailable/ {printf "%d", $2 / 1048576}' /proc/meminfo)
    [ "$free_gb" -ge "$MIN_FREE_GB" ] && break
    log "only ${free_gb} GB free (want $MIN_FREE_GB): waiting"
    sleep 30
  done
  [ "$free_gb" -ge "$MIN_FREE_GB" ] || { log "never had $MIN_FREE_GB GB free: not building"; exit 1; }
  log "building beatscript ($eng) on the build host"
  (cd "$dir" && BUILD_HOST=${BUILD_HOST:-workbit} BEATSCRIPT_BUILD_DIR=${LISTEN_BUILD_DIR:-beatscript-listen-build} scripts/build-linux.sh --no-smoke) >&2
  echo "$eng" > "$stamp"
  flock -u 9
fi

LISTEN_COMMIT="beatscript@$eng" \
  python3 -I "$dir/tools/listen/listen_ci.py" --beatscript "$bin" --examples "$ex" --publish "$@"
