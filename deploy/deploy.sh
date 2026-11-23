#!/usr/bin/env bash
# deploy/deploy.sh: build beatscript.app from this repo at ONE commit and deploy it to the serving host, then run
# every live check. It runs on an ops machine with ssh to the serving host; heavy builds run on a build host
# (scripts/build-host.sh: BUILD_HOST, else this machine, else BUILD_HOSTS).
#
#   deploy/deploy.sh all          preflight, build, host, ship, services, restart, check, record (the deploy)
#   deploy/deploy.sh preflight    refuse unless the tree is clean, HEAD is PIN and PIN is on origin/main
#   deploy/deploy.sh build        the wasm (site/web/wasm + gz/br sidecars), its vocals test, beatscript and beatscript-serve for
#                                 x86_64 Linux (dist/), and site/web/docs (BEATSCRIPT.md, arrange.md, cheatsheet.md)
#   deploy/deploy.sh host         APP_DIR on the host: a checkout of this repo, detached at PIN
#   deploy/deploy.sh ship         copy the wasm, the docs and the binary there (site/bin/beatscript-serve-<sha7>)
#   deploy/deploy.sh services     rewrite the btw-rust entry of ~/services (backup first; the old entry is kept in
#                                 STATE_DIR for rollback): dir APP_DIR, CMD "cd APP_DIR && ... -data DATA_DIR ..."
#   deploy/deploy.sh restart      ~/services restart btw-rust; /healthz must report beatscript-serve-<sha7>
#   deploy/deploy.sh check        deploy/live-checks.sh against PUBLIC_URL (every live check, the MCP exposure table,
#                                 verify-beatscript, the live smoke)
#   deploy/deploy.sh record       append "<utc> <PIN> beatscript-serve-<sha7> ok" to DEPLOY_LOG on the host (only `all` does
#                                 this, after every check passed)
#   deploy/deploy.sh rollback     put back the btw-rust entry the last `services` step replaced, restart, /healthz
#   deploy/deploy.sh status       the host checkout, the btw-rust entry, /healthz, the deploy log's tail
#
# `all` rolls back by itself when restart or check fails (NO_ROLLBACK=1 leaves the new version up to look at).
# Every failure names itself on stderr: the step, and the line, command and exit code that stopped it; `all` ends a
# rollback with "deploy: ROLLED BACK ... step <step> failed (exit <code>)" and exits with that code. (The second
# cutover, 2026-11-22, rolled back after a live check exited 28 without a word, so nothing said which step or why.)
# A rollback after the cutover from the old layout restores the old entry exactly (its dir is untouched); after
# later deploys it puts back the previous CMD and its own binary, but the checkout and the wasm stay at the new pin:
# for a full return, deploy the previous pin.
#
# data/ is never copied, moved or read here: beatscript-serve is pointed at DATA_DIR by absolute path. The env file
# (~/.env-services/btw-rust -> btw-bitwrap: the Gemini key, the admin token, the session secret) stays on the host and
# is only ever sourced by ~/services.
#
# Environment (the defaults are the production values):
#   PIN          the commit to deploy (default HEAD; HEAD must equal it)
#   HOST         the serving host over ssh; `local`: this machine (pflow.dev)
#   APP_DIR      the checkout there, relative to its $HOME        (Workspace/beatscript)
#   DATA_DIR     beatscript-serve's data dir there, relative to $HOME     (Workspace/btw-bitwrap-io/data, where it has always been)
#   PUBLIC_URL   the public origin                                (https://beatscript.app)
#   REPO_URL     what the host clones                             (https://github.com/stackdump/beatscript.git)
#   STATE_DIR    on the host: the rollback record                 (.local/state/beatscript)
#   DEPLOY_LOG   on the host: one line per clean deploy           ($STATE_DIR/deploys.log)
#   BUILD_HOST, BUILD_HOSTS, BUILD_HOST_ALIASES, BEATSCRIPT_SMOKE_HOST   scripts/build-host.sh, scripts/build-linux.sh
#   BEATSCRIPT_BUILD_DIR, BEATSCRIPT_WASM_BUILD_DIR  the build host's scratch dirs (beatscript-web-build, beatscript-wasm-build)
#   CHROME       Chrome for the live smoke                        (/usr/bin/google-chrome)
#   NO_ROLLBACK  1: `all` leaves a failed deploy up
set -eEuo pipefail
cd "$(dirname "$0")/.."
root=$(pwd -P)
STEP=start
# -E: the trap runs inside the step functions too. Inside `if restart && check` it does not run (bash suppresses ERR
# there, as it does set -e): `all` reports that pair itself. Only the main shell says it, once per failure.
trap 'rc=$?; [ "$BASHPID" != $$ ] || echo "deploy: FAILED in step $STEP: deploy/deploy.sh line $LINENO (exit $rc): $BASH_COMMAND" >&2' ERR

HOST=${HOST:-pflow.dev}
APP_DIR=${APP_DIR:-Workspace/beatscript}
DATA_DIR=${DATA_DIR:-Workspace/btw-bitwrap-io/data}
PUBLIC_URL=${PUBLIC_URL:-https://beatscript.app}
REPO_URL=${REPO_URL:-https://github.com/stackdump/beatscript.git}
STATE_DIR=${STATE_DIR:-.local/state/beatscript}
DEPLOY_LOG=${DEPLOY_LOG:-$STATE_DIR/deploys.log}
SERVICE=btw-rust
PORT=18099
# the build hosts' scratch dirs, apart from anything else that builds there
export BEATSCRIPT_BUILD_DIR=${BEATSCRIPT_BUILD_DIR:-beatscript-web-build} BEATSCRIPT_WASM_BUILD_DIR=${BEATSCRIPT_WASM_BUILD_DIR:-beatscript-wasm-build}
PIN=${PIN:-$(git rev-parse HEAD)}
PIN=$(git rev-parse "$PIN^{commit}")
SHA7=$(git rev-parse --short=7 "$PIN")
VERSION=beatscript-serve-$SHA7
BIN=site/bin/beatscript-serve-$SHA7

# HOST=local runs the host side here, from $HOME, as ssh would (deploy/test-host.sh drives `host` that way)
r() { if [ "$HOST" = local ]; then (cd "$HOME" && bash -c "$*"); else ssh -o BatchMode=yes "$HOST" "$@"; fi; }
say() { STEP=$1; printf '\n== deploy %s: %s\n' "$VERSION" "$*"; }

preflight() {
  say "preflight"
  [ "$(git rev-parse HEAD)" = "$PIN" ] || { echo "deploy: HEAD is $(git rev-parse --short HEAD), not PIN $SHA7: check out the pin first" >&2; exit 1; }
  git diff --quiet && git diff --cached --quiet || { echo "deploy: the tree has uncommitted changes: deploy a commit, not a working tree" >&2; exit 1; }
  git fetch -q origin main
  git merge-base --is-ancestor "$PIN" origin/main || { echo "deploy: $SHA7 is not on origin/main: push it (or merge it) first" >&2; exit 1; }
  echo "pin $PIN ($VERSION), on origin/main $(git rev-parse --short origin/main)"
}

build() {
  say "build"
  scripts/build-web-wasm.sh site/web/wasm
  node site/test/wasm-vocals.mjs
  # precompressed sidecars (6.75 MB -> ~2 MB): beatscript-serve sends beatscript_bg.wasm.br / .gz to a browser that accepts it and
  # ignores a sidecar older than the wasm. The serving host has no brotli, so they are made here.
  w=site/web/wasm/beatscript_bg.wasm
  gzip -9 -n -k -f "$w"
  if command -v brotli >/dev/null; then brotli -q 11 -k -f "$w"; else rm -f "$w.br"; echo "no brotli here: gzip sidecar only"; fi
  scripts/build-linux.sh                       # beatscript: its `cheatsheet` writes site/web/docs/cheatsheet.md
  BEATSCRIPT_PKG=beatscript-serve scripts/build-linux.sh
  BEATSCRIPT_BIN=$root/dist/beatscript-linux-x86_64 site/scripts/beatscript-gen-docs.sh
  ls -l site/web/wasm site/web/docs dist/beatscript-serve-linux-x86_64
}

host() {
  say host "checkout"
  r "APP=\$HOME/$APP_DIR DATA=\$HOME/$DATA_DIR PIN=$PIN REPO=$REPO_URL bash -s" <<'EOS'
set -euo pipefail
test -d "$DATA" || { echo "deploy: no data dir $DATA on the host" >&2; exit 1; }
if [ ! -d "$APP/.git" ]; then git clone -q --no-checkout "$REPO" "$APP"; fi
cd "$APP"
git fetch -q origin main
git cat-file -e "$PIN^{commit}"
if [ ! -e .git/index ]; then
  # never checked out: the --no-checkout clone just made, or one a failed deploy left behind. It has no index, so
  # `git status` lists every tracked file as a staged deletion: that is not a local change, and checking the pin out
  # is safe, but only into a work tree that holds nothing but .git.
  stray=$(ls -A | grep -vx '\.git' || true)
  [ -z "$stray" ] || { echo "deploy: $APP was never checked out but holds files: refusing" >&2; printf '%s\n' "$stray" | head -20 >&2; exit 1; }
  # (with PIN = HEAD a bare `checkout` moves nothing and leaves the index empty: reset fills index and work tree)
  git checkout -q --detach "$PIN"
  git reset -q --hard
else
  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then echo "deploy: $APP has local changes: refusing" >&2; git status --short | head -40 >&2; exit 1; fi
  git checkout -q --detach "$PIN"
fi
test "$(git rev-parse HEAD)" = "$PIN"
# the checkout is PIN, whole: no tracked file missing or changed
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then echo "deploy: $APP is not a clean checkout of $PIN after checkout" >&2; git status --short | head -40 >&2; exit 1; fi
test -f deploy/deploy.sh || { echo "deploy: $APP has no deploy/deploy.sh at $PIN: the work tree is not populated" >&2; exit 1; }
test ! -e data || { echo "deploy: $APP/data exists; beatscript-serve's data is $DATA (absolute), never inside the checkout" >&2; exit 1; }
mkdir -p site/bin site/web/wasm site/web/docs
echo "host checkout $(git log -1 --format='%h %s')"
EOS
}

ship() {
  say "ship"
  [ -x dist/beatscript-serve-linux-x86_64 ] && [ -f site/web/wasm/beatscript_bg.wasm ] && [ -f site/web/docs/cheatsheet.md ] || { echo "deploy: build first" >&2; exit 1; }
  rsync -az --delete site/web/wasm/ "$HOST:$APP_DIR/site/web/wasm/"
  rsync -az site/web/docs/ "$HOST:$APP_DIR/site/web/docs/"
  rsync -az dist/beatscript-serve-linux-x86_64 "$HOST:$APP_DIR/$BIN"
  want=$(sha256sum dist/beatscript-serve-linux-x86_64 | awk '{print $1}')
  r "cd \$HOME/$APP_DIR && chmod 755 $BIN && chmod -R a+rX site/web/docs site/web/wasm && ./$BIN -h >/dev/null 2>&1; test \"\$(sha256sum $BIN | awk '{print \$1}')\" = $want && echo \"$BIN sha256 $want\""
}

services() {
  say "services"
  r "ln -sfn btw-bitwrap ~/.env-services/btw-rust && ls -l ~/.env-services/btw-rust | awk '{print \$9, \$10, \$11}'"
  r "APP=\$HOME/$APP_DIR DATA=\$HOME/$DATA_DIR STATE=\$HOME/$STATE_DIR BIN=$BIN VERSION=$VERSION PORT=$PORT PUBLIC=$PUBLIC_URL bash -s" <<'EOS'
set -euo pipefail
f=$HOME/services
mkdir -p "$STATE"
b="$f.bak.beatscript-$(date +%Y%m%d%H%M%S)"; cp -p "$f" "$b"; echo "backup: $b"
python3 - "$f" "$STATE/previous-btw-rust.json" <<'PY'
import json, os, re, sys
p, prev_path = sys.argv[1], sys.argv[2]
app, data, binp, version, port, public = (os.environ[k] for k in ("APP", "DATA", "BIN", "VERSION", "PORT", "PUBLIC"))
home = os.environ["HOME"]
s = open(p).read()
lines = s.splitlines(keepends=True)
si = [k for k, l in enumerate(lines) if l.startswith('    ["btw-rust"]="')]
ci = [k for k, l in enumerate(lines) if l.startswith('        btw-rust)')]
assert len(si) == 1 and len(ci) == 1, "one btw-rust SERVICES entry and one CMD line expected"
old_entry = re.search(r'\["btw-rust"\]="([^"]*)"', lines[si[0]]).group(1)
old_cmd = re.search(r'CMD="([^"]*)"', lines[ci[0]]).group(1)
old_port, old_url, old_dir = old_entry.split("|")
cmd = (f"cd {app} && systemd-run --user --scope --quiet --unit=btw-rust-run -p MemoryHigh=1536M -p MemoryMax=2G ./{binp} "
       f"-addr 127.0.0.1:{port} -data {data} -log-dir {data}/log-rs -web site/web -examples site/examples "
       f"-docs site/web/docs -docs-src site/docs-src -public-url {public} -legacy-hosts btw.bitwrap.io "
       f"-render-concurrency 2 -render-max-bars 256 -render-max-seconds 600 -render-cache-mb 512 -version {version}")
# the Android beta sign-up is open only when a flip set both flags; a lone -android-beta-url is the old direct link
url, owners = re.search(r' -android-beta-url (\S+)', old_cmd), re.search(r' -beta-owners (\S+)', old_cmd)
if url and owners:
    cmd = cmd.replace(' -public-url ', f' -android-beta-url {url.group(1)} -beta-owners {owners.group(1)} -public-url ', 1)
# keep the previous entry once per distinct CMD: a rerun of the same deploy must not overwrite it with itself
if old_cmd != cmd:
    json.dump({"entry": old_entry, "cmd": old_cmd}, open(prev_path, "w"), indent=1)
dir_field = app.replace(home, "$HOME", 1) if app.startswith(home + "/") else app
lines[si[0]] = f'    ["btw-rust"]="{old_port}|{old_url}|{dir_field}"\n'
lines[ci[0]] = f'        btw-rust)       CMD="{cmd}" ;;\n'
open(p, "w").write("".join(lines))
print("btw-rust entry was: " + old_entry)
print("btw-rust CMD was:   " + old_cmd)
print("btw-rust entry now: " + f"{old_port}|{old_url}|{dir_field}")
print("btw-rust CMD now:   " + cmd)
PY
bash -n "$f" && echo "services syntax ok"
EOS
}

restart() {
  say "restart"
  # (called inside `if` by `all`: the last line decides)
  r "~/services restart $SERVICE >/dev/null; for i in \$(seq 1 60); do curl -sf -m 2 http://127.0.0.1:$PORT/healthz | grep -q '\"version\":\"$VERSION\"' && break; sleep 1; done; curl -s -m 5 http://127.0.0.1:$PORT/healthz; echo"
  local rc
  r "curl -sf -m 5 http://127.0.0.1:$PORT/healthz | grep -q '\"version\":\"$VERSION\"'" || { rc=$?; echo "deploy: :$PORT/healthz does not report $VERSION (exit $rc)" >&2; return "$rc"; }
}

check() {
  say check "(the live checks)"
  # (called inside `if` by `all`, where set -e is off: every step returns its own failure)
  local rc
  r "cd \$HOME/$APP_DIR && test \"\$(git rev-parse HEAD)\" = $PIN && echo \"host checkout $SHA7\"" || { rc=$?; echo "deploy: the host checkout is not $SHA7 (exit $rc)" >&2; return "$rc"; }
  CHROME=${CHROME:-/usr/bin/google-chrome} deploy/live-checks.sh "$PUBLIC_URL" "$VERSION" || { rc=$?; echo "deploy: deploy/live-checks.sh exited $rc (its LIVE CHECK FAILED line above says which check)" >&2; return "$rc"; }
}

record() {
  r "mkdir -p \$(dirname \$HOME/$DEPLOY_LOG) && echo \"\$(date -u +%Y-%m-%dT%H:%M:%SZ) $PIN $VERSION ok\" >> \$HOME/$DEPLOY_LOG && tail -n 3 \$HOME/$DEPLOY_LOG"
}

rollback() {
  say "rollback"
  r "STATE=\$HOME/$STATE_DIR PORT=$PORT bash -s" <<'EOS'
set -euo pipefail
f=$HOME/services; prev=$STATE/previous-btw-rust.json
[ -f "$prev" ] || { echo "rollback: no $prev: nothing recorded to roll back to" >&2; exit 1; }
cp -p "$f" "$f.bak.beatscript-rollback-$(date +%Y%m%d%H%M%S)"
python3 - "$f" "$prev" <<'PY'
import json, os, re, sys
p, prev = sys.argv[1], json.load(open(sys.argv[2]))
home = os.environ["HOME"]
lines = open(p).read().splitlines(keepends=True)
si = [k for k, l in enumerate(lines) if l.startswith('    ["btw-rust"]="')]
ci = [k for k, l in enumerate(lines) if l.startswith('        btw-rust)')]
assert len(si) == 1 and len(ci) == 1
cmd = prev["cmd"]
d = prev["entry"].split("|")[2].replace("$HOME", home, 1)
# ~/services restart reuses the tmux window and its cwd: a CMD with relative paths must cd first
if not cmd.startswith("cd "):
    cmd = f"cd {d} && {cmd}"
lines[si[0]] = f'    ["btw-rust"]="{prev["entry"]}"\n'
lines[ci[0]] = f'        btw-rust)       CMD="{cmd}" ;;\n'
open(p, "w").write("".join(lines))
print("btw-rust entry back to: " + prev["entry"]); print("btw-rust CMD back to:   " + cmd)
PY
bash -n "$f" && echo "services syntax ok"
~/services restart btw-rust >/dev/null
for i in $(seq 1 60); do curl -sf -m 2 "http://127.0.0.1:$PORT/healthz" >/dev/null && break; sleep 1; done
curl -s -m 5 "http://127.0.0.1:$PORT/healthz"; echo
EOS
}

status() {
  r "cd \$HOME/$APP_DIR 2>/dev/null && git log -1 --format='checkout %h %s' || echo 'no checkout at $APP_DIR'; grep -n 'btw-rust' ~/services | cut -c1-240; curl -s -m 5 http://127.0.0.1:$PORT/healthz; echo; tail -n 5 \$HOME/$DEPLOY_LOG 2>/dev/null || echo 'no deploy log yet'"
}

case "${1:-}" in
preflight|build|host|ship|services|restart|check|record|rollback|status) "$1" ;;
all)
  preflight; build; host; ship; services
  if restart && check; then
    record
    echo; echo "DEPLOYED $VERSION ($PIN) to $PUBLIC_URL from stackdump/beatscript"
  else
    rc=$? failed=$STEP
    echo "deploy: step $failed failed (exit $rc) for $VERSION" >&2
    if [ "${NO_ROLLBACK:-0}" != 1 ]; then
      rollback
      echo "deploy: ROLLED BACK $VERSION: step $failed failed (exit $rc)" >&2
    else
      echo "deploy: NO_ROLLBACK=1: $VERSION left up; step $failed failed (exit $rc)" >&2
    fi
    exit "$rc"
  fi
  ;;
*) sed -n '2,40p' "$0"; exit 2 ;;
esac
