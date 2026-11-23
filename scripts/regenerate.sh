#!/usr/bin/env bash
# regenerate.sh: regenerate every generated ontology/release file in one deterministic, idempotent run.
#
#   scripts/regenerate.sh            regenerate in place, then list the generated paths that changed (commit them)
#   scripts/regenerate.sh --check    regenerate, print the generated paths that differ from what is committed and exit 1
#                                    if there are any; the generated paths are put back afterwards (git checkout and
#                                    git clean of those paths only). Refuses to start when they already have changes.
#
# This is what val's merge queue runs after a rebase, so that parallel pull requests stop conflicting on these files:
# a generated file is a function of the tree, so rebuilding it from the rebased tree is the merge.
#
# The generated paths, the only files this script may change (it fails if anything outside them changed because of it):
#   crates/beatscript-serve/src/mcp/testdata/tools-list.json   the MCP tools/list fixture
#   crates/beatscript-serve/ontology/generated/                the vocabulary dumped from the code (beatscript.ttl)
#   crates/beatscript-serve/ontology/release/                  the release: ns.*, pack.json, manifest.json, cert/
#   crates/beatscript-serve/ontology/release.lock              sha256 of every release file
#   crates/beatscript-serve/ontology/fixtures/                 the ontology MCP's recorded answers (mcp-<version>.json)
#   crates/djhost/data/beatscript-ontology.json                the header-term snapshot (keeps the "fetched" date it has)
#
# The steps, in the order they depend on each other (the same as the manual list in ontology/README.md "Changing it"):
#   1. BEATSCRIPT_UPDATE_TOOLS_LIST=1 cargo test ... tools_list_matches_go_fixture     rewrites tools-list.json
#   2. crates/beatscript-serve/ontology/build.sh                                       generated/, release/, release.lock
#   3. crates/beatscript-serve/ontology/fixtures.py against a beatscript-serve built from this tree and run on a free
#      port with a temporary data dir (killed on exit)                                 fixtures/mcp-<version>.json
#   4. tools/ontology/refresh.py --fetched <the date the snapshot already has>        the snapshot (the output must not
#      depend on the day it is run)
#
# Not covered: the goldens that BEATSCRIPT_BLESS=1 rewrites (grammar block, CID pins, api goldens, CONTROL.md): those are
# rewritten on purpose, after reading the diff, not rebuilt from the tree.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
git rev-parse --git-dir >/dev/null 2>&1 || { echo "regenerate: $PWD is not a git checkout" >&2; exit 2; }

GENERATED=(
  crates/beatscript-serve/src/mcp/testdata/tools-list.json
  crates/beatscript-serve/ontology/generated
  crates/beatscript-serve/ontology/release
  crates/beatscript-serve/ontology/release.lock
  crates/beatscript-serve/ontology/fixtures
  crates/djhost/data/beatscript-ontology.json
)
SNAPSHOT=crates/djhost/data/beatscript-ontology.json
RELEASE=crates/beatscript-serve/ontology/release
STEPS=4

mode=normal
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "usage: scripts/regenerate.sh [--check]" >&2; exit 2 ;;
esac
[ $# -le 1 ] || { echo "usage: scripts/regenerate.sh [--check]" >&2; exit 2; }

command -v cargo >/dev/null 2>&1 || PATH=$HOME/.cargo/bin:$PATH
for tool in cargo python3 git; do
  command -v "$tool" >/dev/null 2>&1 || { echo "regenerate: $tool is not on PATH" >&2; exit 2; }
done
# The same debug-info trim build.sh uses, for every cargo step, so the steps share one set of build artifacts.
export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0} CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}

EXCLUDES=()
for p in "${GENERATED[@]}"; do EXCLUDES+=(":(exclude)$p"); done

# What git sees outside the generated paths: the status (text), and a hash of that plus the content of every change, so a
# file that was already modified and then rewritten is noticed too.
outside_status() { git status --porcelain -uall -- . "${EXCLUDES[@]}"; }
outside_hash() { { outside_status; git diff HEAD --binary -- . "${EXCLUDES[@]}"; } | git hash-object --stdin; }
generated_status() { git status --porcelain -uall -- "${GENERATED[@]}"; }

if [ "$mode" = check ] && [ -n "$(generated_status)" ]; then
  echo "regenerate: --check needs the generated paths to match HEAD, but they have changes (commit or git checkout them first):" >&2
  generated_status >&2
  exit 2
fi
before_status=$(outside_status)
before_hash=$(outside_hash)

tmp=$(mktemp -d)
server_pid=
restore_on_exit=0
t_start=$SECONDS

stop_server() {
  [ -n "$server_pid" ] || return 0
  kill -TERM -- "-$server_pid" 2>/dev/null || kill -TERM "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  server_pid=
}
restore_generated() {
  git checkout -q -- "${GENERATED[@]}" || true
  git clean -fdq -- "${GENERATED[@]}" || true
}
cleanup() {
  local rc=$?
  trap - EXIT
  stop_server
  [ "$restore_on_exit" = 1 ] && restore_generated
  rm -rf "$tmp"
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
[ "$mode" = check ] && restore_on_exit=1

step=0
# run LABEL CMD...: one line to say what is happening, the command's output kept for when it fails. A function used as CMD
# runs with errexit off (it is on the left of ||), so each of its commands ends in `|| return 1`.
run() {
  local label=$1 rc=0
  shift
  step=$((step + 1))
  printf 'regenerate: [%d/%d] %s\n' "$step" "$STEPS" "$label"
  "$@" >"$tmp/step.log" 2>&1 || rc=$?
  if [ "$rc" != 0 ]; then
    echo "regenerate: step $step failed (exit $rc): $label" >&2
    tail -n 80 "$tmp/step.log" >&2
    exit 1
  fi
}

tools_list() {
  BEATSCRIPT_UPDATE_TOOLS_LIST=1 cargo test --locked -q -p beatscript-serve --bin beatscript-serve tools_list_matches_go_fixture || return 1
  # a renamed test would match nothing and "pass": the fixture would silently not be rewritten
  grep -q ' 1 passed' "$tmp/step.log" || { echo "tools_list_matches_go_fixture did not run (renamed?)"; return 1; }
}

fixtures() {
  local out bin port=''
  out=$(cargo build --locked -q -p beatscript-serve --bin beatscript-serve --message-format=json) || return 1
  bin=$(printf '%s\n' "$out" | python3 -I -c '
import json, sys
exe = ""
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and "bin" in m["target"]["kind"] and m["target"]["name"] == "beatscript-serve":
        exe = m["executable"]
print(exe)') || return 1
  [ -x "$bin" ] || { echo "no beatscript-serve binary from cargo build"; return 1; }
  mkdir -p "$tmp/data"
  # port 0: the kernel picks a free one and the server says which. Its own session (setsid), so its ontology query workers
  # go with it when it is stopped.
  if command -v setsid >/dev/null 2>&1; then
    setsid "$bin" -addr 127.0.0.1:0 -data "$tmp/data" -log-dir off -web "" -docs "" >"$tmp/serve.log" 2>&1 &
  else
    "$bin" -addr 127.0.0.1:0 -data "$tmp/data" -log-dir off -web "" -docs "" >"$tmp/serve.log" 2>&1 &
  fi
  server_pid=$!
  for _ in $(seq 1 300); do
    port=$(sed -n 's/^beatscript-serve: listening on 127\.0\.0\.1:\([0-9][0-9]*\) .*/\1/p' "$tmp/serve.log" | head -n 1)
    [ -z "$port" ] || break
    kill -0 "$server_pid" 2>/dev/null || { echo "beatscript-serve exited before it listened:"; cat "$tmp/serve.log"; return 1; }
    sleep 0.1
  done
  [ -n "$port" ] || { echo "beatscript-serve did not listen within 30 s:"; cat "$tmp/serve.log"; return 1; }
  python3 -I crates/beatscript-serve/ontology/fixtures.py "http://127.0.0.1:$port/ontology/mcp" || return 1
  stop_server
}

snapshot() {
  # refresh.py stamps today's date unless told one; a date that moves every day makes every run a change. Keep the one the
  # snapshot has (the release's own date when there is none).
  local fetched
  fetched=$(python3 -I -c '
import json, sys
try:
    print(json.load(open(sys.argv[1], encoding="utf-8")).get("fetched", ""))
except (OSError, ValueError):
    print("")' "$SNAPSHOT") || return 1
  [ -n "$fetched" ] || fetched=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["released"])' "$RELEASE/manifest.json") || return 1
  python3 -I tools/ontology/refresh.py --fetched "$fetched" || return 1
}

run "mcp/testdata/tools-list.json (beatscript-serve tools_list_matches_go_fixture)" tools_list
run "ontology generated/, release/, release.lock (ontology/build.sh)" crates/beatscript-serve/ontology/build.sh
run "ontology/fixtures/mcp-<version>.json (ontology/fixtures.py against a local beatscript-serve)" fixtures
run "djhost/data/beatscript-ontology.json (tools/ontology/refresh.py)" snapshot

stop_server

after_hash=$(outside_hash)
if [ "$after_hash" != "$before_hash" ]; then
  echo "regenerate: FAILED: a file outside the generated paths changed (a step touched something it should not have):" >&2
  after_status=$(outside_status)
  if [ "$after_status" != "$before_status" ]; then
    diff <(printf '%s\n' "$before_status") <(printf '%s\n' "$after_status") >&2 || true
  else
    echo "  (same files as before, different content: $(git diff HEAD --name-only -- . "${EXCLUDES[@]}" | tr '\n' ' '))" >&2
  fi
  exit 1
fi

changed=$(generated_status)
elapsed=$((SECONDS - t_start))
if [ "$mode" = check ]; then
  if [ -n "$changed" ]; then
    echo "regenerate: --check: STALE, what the tree generates differs from what is committed (run scripts/regenerate.sh and commit):" >&2
    printf '%s\n' "$changed" >&2
    exit 1
  fi
  echo "regenerate: --check: ok, the committed generated files are what the tree generates (${elapsed}s)"
else
  if [ -n "$changed" ]; then
    echo "regenerate: done in ${elapsed}s, generated paths changed:"
    printf '%s\n' "$changed"
  else
    echo "regenerate: done in ${elapsed}s, generated paths unchanged"
  fi
fi
