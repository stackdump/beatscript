#!/usr/bin/env bash
# deploy/test-services.sh: the ~/services steps of deploy/deploy.sh (detect, services, restart, finalize, rollback,
# record), with HOST=local and a scratch $HOME, against a synthetic ~/services, so it needs no ssh, no tmux, no
# systemd and no network. The case it pins down is the rename of the service btw-rust -> beatscript (2026-11-22):
# one deploy finds btw-rust, adds beatscript beside it, stops the old one, starts the new one and keeps it only when
# every check passes; anything that fails puts btw-rust back and proves it is back by its exact version.
#
# The synthetic host, all under the scratch $HOME:
#   ~/services       the real one's layout (SESSION, SERVICES, SERVICE_ORDER, get_service_cmd arms) and its stop NAME /
#                    start NAME / restart NAME semantics, a pid file standing for each tmux window: stop closes the
#                    window; start refuses an open one; restart on a closed one types its CMD nowhere, as tmux does.
#                    Run bare, with `help`, or an action without a name, it records the call and exits 99.
#   systemd-run      on PATH: refuses a unit whose scope is active, as systemd does, records it, execs the command
#   systemctl        on PATH: --user is-active U.scope (active while that process lives), reset-failed
#   tmux             on PATH: has-session (the session file); anything else is recorded as a stray call
#   site/bin/*       a tiny /healthz server: {"name", "ok", "version"} from its -version, on its -addr. A file
#                    ~/.fake/fail-<version> makes that version die at start (`dead`) or misreport (`wrong-version`:
#                    it answers <version>-not, which a substring match would take for <version>).
# deploy.sh's preflight, build, host and ship are stubbed (they need a build host and a real checkout) and so is
# check (it needs the public site): it fails with the code in ~/.fake/fail-check when that file exists.
#
#   1. the cutover: btw-rust out, beatscript in, the same port|url|dir, the CMD's flags carried over exactly
#   2. the new version dies, or answers another version: rolled back to btw-rust, its exact old version, ~/services
#      byte for byte as it was
#   3. a live check fails: rolled back the same way, exit code passed on
#   4. the deploy after the cutover is the steady state
#   5. the steady state's rollback: back to the exact previous version, and a loud failure when what comes back
#      answers another version
#   6. both names (a half-done rename), neither, a name twice: refused, nothing touched, nothing built
#   7. the cutover's precheck: refused, nothing touched, when btw-rust does not answer as its CMD says or when
#      beatscript-run is already active
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
s=$(mktemp -d)
cleanup() { # every synthetic server this test started
  local p
  for p in "$s"/home/.fake/win/* "$s"/home/.fake/scope/* "$s"/extra.pid; do [ ! -f "$p" ] || kill -9 -- "-$(cat "$p")" "$(cat "$p")" 2>/dev/null || true; done
  rm -rf "$s"
}
trap 'cleanup || true' EXIT
export GIT_CONFIG_GLOBAL=$s/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --global user.name test; git config --global user.email test@example.invalid
git config --global init.defaultBranch main; git config --global commit.gpgsign false; git config --global advice.detachedHead false

# a repo carrying deploy.sh with the build steps stubbed; three pins A, B, C
src=$s/src; mkdir -p "$src/deploy"
cp "$here/deploy.sh" "$src/deploy/deploy.sh"
git -C "$src" init -q; git -C "$src" add -A; git -C "$src" commit -q -m A
A=$(git -C "$src" rev-parse HEAD); git -C "$src" commit -q --allow-empty -m B
B=$(git -C "$src" rev-parse HEAD); git -C "$src" commit -q --allow-empty -m C
C=$(git -C "$src" rev-parse HEAD)
v() { echo "beatscript-serve-$(git -C "$src" rev-parse --short=7 "$1")"; }
VA=$(v "$A") VB=$(v "$B") VC=$(v "$C") OLDV=btwserve-1111111
stub=$src/deploy/deploy-stubbed.sh
awk '/^case "\$\{1:-\}" in$/ {
  print "preflight() { say preflight; }; build() { say build; }; host() { say host; }; ship() { say ship; }"
  print "check() { say check \"(stub)\"; local f=$HOME/.fake/fail-check; [ ! -f \"$f\" ] || { echo \"stub check: failing with $(cat \"$f\")\"; return \"$(cat \"$f\")\"; }; }"
} { print }' "$src/deploy/deploy.sh" > "$stub"; chmod +x "$stub"

# the fakes on PATH
bin=$s/bin; mkdir -p "$bin"
cat > "$bin/systemd-run" <<'SH'
#!/usr/bin/env bash
unit=
while [ $# -gt 0 ]; do case $1 in --user|--scope|--quiet|-q) shift ;; --unit=*) unit=${1#--unit=}; shift ;; -p) shift 2 ;; *) break ;; esac; done
d=$HOME/.fake/scope; mkdir -p "$d"
if [ -f "$d/$unit" ] && "$(dirname "$0")/alive" "$(cat "$d/$unit")"; then echo "Failed to start transient scope unit: Unit $unit.scope was already loaded" >&2; exit 1; fi
echo $$ > "$d/$unit"
exec "$@"
SH
cat > "$bin/alive" <<'SH'
#!/usr/bin/env bash
[ -n "${1:-}" ] && [ -d "/proc/$1" ] && ! grep -q '^State:.*Z' "/proc/$1/status" 2>/dev/null
SH
cat > "$bin/systemctl" <<'SH'
#!/usr/bin/env bash
[ "${1:-}" = --user ] && shift
case ${1:-} in
is-active) p=$HOME/.fake/scope/${2%.scope}; if [ -f "$p" ] && "$(dirname "$0")/alive" "$(cat "$p")"; then echo active; else echo inactive; exit 3; fi ;;
reset-failed) ;;
*) echo "systemctl $*" >> "$HOME/.fake/stray"; exit 1 ;;
esac
SH
cat > "$bin/tmux" <<'SH'
#!/usr/bin/env bash
case ${1:-} in has-session) [ -e "$HOME/.fake/session" ] ;; *) echo "tmux $*" >> "$HOME/.fake/stray"; exit 1 ;; esac
SH
cat > "$s/server.py" <<'PY'
#!/usr/bin/env python3
import http.server, json, os, signal, sys
# C-c in a tmux pane interrupts the server; a background job of a script starts with SIGINT ignored, so undo that
signal.signal(signal.SIGINT, signal.default_int_handler)
a = sys.argv[1:]
addr, version = a[a.index("-addr") + 1], a[a.index("-version") + 1]
fake = os.path.join(os.environ["HOME"], ".fake")
fail = os.path.join(fake, "fail-" + version)
mode = open(fail).read().strip() if os.path.exists(fail) else ""
if mode == "dead":
    sys.exit(1)
open(os.path.join(fake, "env-" + version), "w").write(os.environ.get("FAKE_SECRET", ""))
body = json.dumps({"name": "beatscript" if version.startswith("beatscript-serve-") else "btw", "ok": True,
                   "version": version + ("-not" if mode == "wrong-version" else "")}, separators=(",", ":")).encode()
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *x):
        pass
    def do_GET(self):
        self.send_response(200 if self.path == "/healthz" else 404)
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
host, port = addr.rsplit(":", 1)
http.server.HTTPServer((host, int(port)), H).serve_forever()
PY
chmod +x "$bin"/* "$s/server.py"
export PATH=$bin:$PATH
P=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')

home=$s/home
OLDCMD="cd $home/app && systemd-run --user --scope --quiet --unit=btw-rust-run -p MemoryHigh=1536M -p MemoryMax=2G ./site/bin/$OLDV -addr 127.0.0.1:$P -data $home/data -log-dir $home/data/log-rs -web site/web -examples site/examples -docs site/web/docs -docs-src site/docs-src -android-beta-url https://example.invalid/beta -beta-owners owner@example.invalid -public-url https://beatscript.app -legacy-hosts btw.bitwrap.io -render-concurrency 2 -render-max-bars 256 -render-max-seconds 600 -render-cache-mb 512 -version $OLDV"
fresh() { # a host where btw-rust runs $OLDV, as it did before the rename
  cleanup_home
  mkdir -p "$home/app/site/bin" "$home/data" "$home/.env-services" "$home/.fake/win" "$home/.fake/scope"
  for x in "$OLDV" "$VA" "$VB" "$VC"; do cp "$s/server.py" "$home/app/site/bin/$x"; done
  printf 'FAKE_SECRET=s3cret\n' > "$home/.env-services/btw-bitwrap"; chmod 600 "$home/.env-services/btw-bitwrap"
  ln -s btw-bitwrap "$home/.env-services/btw-rust"
  touch "$home/.fake/session"
  {
    cat <<'SH'
#!/bin/bash
# a synthetic ~/services (deploy/test-services.sh)
SESSION="servers"
ENV_DIR="$HOME/.env-services"
FAKE="$HOME/.fake"

declare -A SERVICES=(
    ["stackdump-com"]="8085|console.stackdump.com|$HOME/Workspace/stackdump-com"
SH
    printf '    ["btw-rust"]="%s|beatscript.app|$HOME/app"\n)\n\n' "$P"
    printf 'SERVICE_ORDER=("stackdump-com" "windwheel" "btw-rust")\n\n'
    printf 'get_service_cmd() {\n    local name="$1"\n    case "$name" in\n'
    printf '        stackdump-com)  CMD="./stackdump-cli serve" ;;\n'
    printf '        btw-rust)       CMD="%s" ;;\n' "$OLDCMD"
    cat <<'SH'
        *)
            echo "Unknown service: $name"
            return 1
            ;;
    esac
}

forbidden() { echo "services $*" >> "$FAKE/forbidden"; exit 99; }
known() { [ -n "${SERVICES[$1]:-}" ] || { echo "Unknown service: $1"; exit 1; }; }
win() { echo "$FAKE/win/$1"; }
launch() { # the window's shell: the env file, then CMD, in the service's dir; the window file holds its process group
    local name=$1 w; w=$(win "$1")
    IFS="|" read -r port url dir <<< "${SERVICES[$name]}"
    get_service_cmd "$name" || return 1
    rm -f "$w.pg"
    ( cd "$dir" && { [ ! -f "$ENV_DIR/$name" ] || { set -a; . "$ENV_DIR/$name"; set +a; }; } &&
      exec setsid bash -c 'echo $$ > "$0"; eval "$1"' "$w.pg" "$CMD" ) </dev/null >>"$FAKE/log-$name" 2>&1 &
    for _ in $(seq 1 50); do [ -s "$w.pg" ] && break; sleep 0.05; done
    mv "$w.pg" "$w"
}
kill_window() { local w; w=$(win "$1"); [ -f "$w" ] && kill -"$2" -- "-$(cat "$w")" 2>/dev/null || true; }
echo "$*" >> "$FAKE/calls"
case "${1:-}" in
    list) for n in "${SERVICE_ORDER[@]}"; do echo "$n"; done ;;
    stop)
        [ -n "${2:-}" ] || forbidden "$@"; known "$2"
        [ -f "$(win "$2")" ] || { echo "$2: no tmux window — already stopped."; exit 0; }
        kill_window "$2" INT; sleep 0.3; kill_window "$2" HUP; rm -f "$(win "$2")"; echo "Stopped $2." ;;
    start)
        [ -n "${2:-}" ] && [ -e "$FAKE/session" ] || forbidden "$@"; known "$2"
        [ ! -f "$(win "$2")" ] || { echo "$2: tmux window already exists — use 'restart $2' to recycle."; exit 1; }
        echo "Starting $2..."; launch "$2" ;;
    restart)
        [ -n "${2:-}" ] && [ -e "$FAKE/session" ] || forbidden "$@"; known "$2"
        echo "Restarting $2..."
        if [ -f "$(win "$2")" ]; then
            IFS="|" read -r port url dir <<< "${SERVICES[$2]}"
            kill_window "$2" INT
            for _ in $(seq 1 16); do curl -s --connect-timeout 1 "http://localhost:$port" >/dev/null 2>&1 || break; sleep 0.5; done
            launch "$2"
        fi
        echo "Restarted $2" ;;
    *) forbidden "$@" ;;
esac
SH
  } > "$home/services"
  chmod 755 "$home/services"
  HOME=$home "$home/services" start btw-rust >/dev/null
  for _ in $(seq 1 50); do [ "$(hzv)" = "$OLDV" ] && break; sleep 0.1; done
  [ "$(hzv)" = "$OLDV" ] || { echo "setup: btw-rust did not come up"; exit 1; }
  cp -p "$home/services" "$s/services.orig"
}
cleanup_home() {
  local p
  for p in "$home"/.fake/win/* "$home"/.fake/scope/*; do [ ! -f "$p" ] || kill -9 -- "-$(cat "$p")" "$(cat "$p")" 2>/dev/null || true; done
  for _ in $(seq 1 50); do curl -s -m 1 -o /dev/null "http://127.0.0.1:$P/" || break; sleep 0.1; done
  rm -rf "$home"
}
hz() { curl -s -m 2 "http://127.0.0.1:$P/healthz" || true; }
hzv() { hz | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])' 2>/dev/null || true; }
hzn() { hz | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])' 2>/dev/null || true; }
scope_alive() { [ -f "$home/.fake/scope/$1" ] && "$bin/alive" "$(cat "$home/.fake/scope/$1")"; }
dp() { # dp PIN: the stubbed deploy.sh all
  rc=0
  HOME=$home HOST=local APP_DIR=app DATA_DIR=data PORT=$P PUBLIC_URL=https://beatscript.app PIN=$1 STOP_WAIT=10 HEALTH_WAIT=4 \
    "$stub" all >"$s/out" 2>&1 || rc=$?
}
cnt() { grep -cE "$1" "$home/services" || true; }   # cnt REGEX: lines of ~/services that match
order() { grep '^SERVICE_ORDER=' "$home/services"; }
arm_cmd() { sed -n "s/^ *$1) *CMD=\"\(.*\)\" ;;\$/\1/p" "$home/services"; }
json() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$home/.local/state/beatscript/$1" "$2" 2>/dev/null || true; }
untouched_calls() { [ ! -s "$home/.fake/calls" ] || [ "$(cat "$home/.fake/calls")" = "start btw-rust" ]; }

fails=0 why=
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; [ -z "$why" ] || echo "     first condition that failed: $why"; sed 's/^/     /' "$s/out"; fails=$((fails + 1)); why=; }
all_of() { local c; for c in "$@"; do eval "$c" || { why=$c; return 1; }; done; }
clean() { [ ! -e "$home/.fake/forbidden" ] && [ ! -e "$home/.fake/stray" ]; }   # never bare, never `help`, never nameless
rolled_back() { # the host exactly as before the cutover, serving $OLDV
  all_of 'cmp -s "$s/services.orig" "$home/services"' '[ "$(hzv)" = "$OLDV" ]' '[ "$(hzn)" = btw ]' \
    'scope_alive btw-rust-run' '! scope_alive beatscript-run' '[ ! -e "$home/.fake/win/beatscript" ]' \
    '[ ! -e "$home/.env-services/beatscript" ]' '[ "$(readlink "$home/.env-services/btw-rust")" = btw-bitwrap ]' \
    '[ ! -e "$home/.local/state/beatscript/deploys.log" ]' clean
}
serving() { # serving VERSION: the host after a clean cutover, beatscript serving VERSION
  V=$1 all_of '[ "$(cnt "^ *\[\"btw-rust\"\]=")" = 0 ]' '[ "$(cnt "^ *btw-rust\)")" = 0 ]' \
    '[ "$(cnt "^ *\[\"beatscript\"\]=")" = 1 ]' '[ "$(cnt "^ *beatscript\)")" = 1 ]' \
    '[ "$(order)" = "SERVICE_ORDER=(\"stackdump-com\" \"windwheel\" \"beatscript\")" ]' \
    'grep -qxF "    [\"beatscript\"]=\"$P|beatscript.app|\$HOME/app\"" "$home/services"' \
    '[ "$(hzv)" = "$V" ]' '[ "$(hzn)" = beatscript ]' 'scope_alive beatscript-run' '! scope_alive btw-rust-run' \
    '[ ! -e "$home/.fake/win/btw-rust" ]' '[ -e "$home/.fake/win/beatscript" ]' \
    '[ "$(readlink "$home/.env-services/beatscript")" = btw-bitwrap ]' '[ ! -e "$home/.env-services/btw-rust" ] && [ ! -L "$home/.env-services/btw-rust" ]' \
    'grep -qx FAKE_SECRET=s3cret "$home/.env-services/btw-bitwrap"' '[ "$(cat "$home/.fake/env-$V")" = s3cret ]' \
    'bash -n "$home/services"' clean
}
last() { tail -n 1 "$s/out"; }
logged() { tail -n 1 "$home/.local/state/beatscript/deploys.log" | cut -d' ' -f2-; }

# 1. the cutover
fresh; dp "$A"
want_cmd=${OLDCMD//--unit=btw-rust-run/--unit=beatscript-run}; want_cmd=${want_cmd//$OLDV/$VA}
bk=$(ls "$home"/services.bak.beatscript-cutover-* 2>/dev/null | head -n 1)
if all_of '[ "$rc" = 0 ]' 'serving "$VA"' '[ "$(arm_cmd beatscript)" = "$want_cmd" ]' \
  '[ -n "$bk" ] && cmp -s "$bk" "$s/services.orig"' \
  '[ "$(json previous-btw-rust.json version)" = "$OLDV" ]' '[ "$(json previous-btw-rust.json cmd)" = "$OLDCMD" ]' \
  '[ "$(json previous-beatscript.json version)" = "$OLDV" ]' '[ "$(json previous-beatscript.json unit)" = beatscript-run ]' \
  '[ "$(json previous-beatscript.json cmd)" = "${OLDCMD//--unit=btw-rust-run/--unit=beatscript-run}" ]' \
  '[ "$(logged)" = "$A $VA ok renamed-from btw-rust" ]' 'grep -q "^DEPLOYED .*(renamed from btw-rust)$" "$s/out"'; then
  ok "cutover: beatscript replaces btw-rust (same port|url|dir, the CMD's flags carried over), records, logs renamed-from"
else bad "cutover (rc $rc)"; fi

# 2. the new version fails its /healthz: dies at start, or answers another version (which only an exact match catches)
for how in dead wrong-version; do
  fresh; echo "$how" > "$home/.fake/fail-$VA"; dp "$A"
  if all_of '[ "$rc" = 1 ]' rolled_back 'grep -q "^deploy: step restart failed (exit 1) for $VA" "$s/out"' \
    '[ "$(last)" = "deploy: ROLLED BACK $OLDV: step restart failed (exit 1)" ]'; then
    ok "cutover, new version $how: rolled back to btw-rust $OLDV exactly, ~/services byte for byte as before"
  else bad "cutover, new version $how (rc $rc)"; fi
done

# 3. a live check fails after the new version came up
fresh; echo 7 > "$home/.fake/fail-check"; dp "$A"
if all_of '[ "$rc" = 7 ]' rolled_back '[ "$(last)" = "deploy: ROLLED BACK $OLDV: step check failed (exit 7)" ]'; then
  ok "cutover, live check fails: beatscript stopped, btw-rust $OLDV back exactly, exit 7 passed on"
else bad "cutover, live check fails (rc $rc)"; fi

# 4. the deploy after the cutover is the steady state
fresh; dp "$A"; [ "$rc" = 0 ] || bad "steady setup: the cutover (rc $rc)"
dp "$B"
if all_of '[ "$rc" = 0 ]' 'serving "$VB"' '[ "$(arm_cmd beatscript)" = "${want_cmd//$VA/$VB}" ]' \
  '[ "$(json previous-beatscript.json version)" = "$VA" ]' '[ "$(logged)" = "$B $VB ok" ]' \
  '! grep -q cutover "$s/out"' 'grep -q "^DEPLOYED $VB " "$s/out"'; then
  ok "after the cutover: a steady deploy (beatscript's CMD rewritten, previous = the cutover's version, no renamed-from)"
else bad "steady deploy after the cutover (rc $rc)"; fi

# 5. the steady state's rollback: exactly the previous version, or a loud failure
echo 5 > "$home/.fake/fail-check"; dp "$C"
if all_of '[ "$rc" = 5 ]' '[ "$(hzv)" = "$VB" ]' '[ "$(arm_cmd beatscript)" = "${want_cmd//$VA/$VB}" ]' \
  'grep -q "^rollback: beatscript serves $VB again (exactly)" "$s/out"' \
  '[ "$(last)" = "deploy: ROLLED BACK $VC: step check failed (exit 5)" ]' clean; then
  ok "steady rollback: back to $VB exactly"
else bad "steady rollback (rc $rc)"; fi
echo wrong-version > "$home/.fake/fail-$VB"; dp "$C"
if all_of '[ "$rc" = 5 ]' '[ "$(hzv)" = "$VB-not" ]' \
  'grep -qF "rollback: :$P/healthz reports '"'"'$VB-not'"'"', not exactly $VB" "$s/out"' \
  'grep -q "^deploy: ROLLBACK FAILED after step check failed (exit 5)" "$s/out"' '! grep -q "ROLLED BACK" "$s/out"'; then
  ok "steady rollback: an answer that only contains the previous version is a failed rollback, said so"
else bad "steady rollback, wrong version back (rc $rc)"; fi
rm -f "$home/.fake/fail-check" "$home/.fake/fail-$VB"

# 6. refused, nothing touched, nothing built: both names, neither, a name twice
refused() { # refused TEXT: ~/services as it was given, no backup, no record, no build, ~/services never run
  T=$1 all_of '[ "$rc" = 1 ]' 'cmp -s "$s/services.given" "$home/services"' '! ls "$home"/services.bak.* >/dev/null 2>&1' \
    '[ ! -e "$home/.local/state/beatscript" ]' '! grep -q ": build$" "$s/out"' untouched_calls clean \
    '[ "$(hzv)" = "$OLDV" ]' 'grep -qF "$T" "$s/out"'
}
# (what an interrupted cutover leaves, step 2 done: beatscript whole beside btw-rust; and a beatscript entry alone)
fresh; sed -i -e "s/^    \[\"btw-rust\"\]=.*/&\n    [\"beatscript\"]=\"$P|beatscript.app|\$HOME\/app\"/" \
  -e 's/^        btw-rust)  .*/&\n        beatscript)     CMD="true" ;;/' -e 's/^SERVICE_ORDER=(.*"btw-rust"/& "beatscript"/' "$home/services"
cp -p "$home/services" "$s/services.given"; dp "$A"
if refused 'REFUSED: ~/services names both btw-rust and beatscript' && refused 'both btw-rust=1/1/1 beatscript=1/1/1'; then
  ok "half-renamed ~/services (both names whole): refused, untouched"; else bad "both names (rc $rc)"; fi
fresh; sed -i "s/^    \[\"btw-rust\"\]=.*/&\n    [\"beatscript\"]=\"$P|beatscript.app|\$HOME\/app\"/" "$home/services"
cp -p "$home/services" "$s/services.given"; dp "$A"
if refused 'REFUSED: ~/services names both btw-rust and beatscript' && refused 'half btw-rust=1/1/1 beatscript=1/0/0'; then
  ok "half-renamed ~/services (a beatscript entry beside btw-rust): refused, untouched"; else bad "both names, partial (rc $rc)"; fi
fresh; sed -i 's/btw-rust/btw-other/g' "$home/services"; cp -p "$home/services" "$s/services.given"; dp "$A"
if refused 'REFUSED: ~/services names neither'; then ok "neither name: refused, untouched"; else bad "neither name (rc $rc)"; fi
fresh; sed -i 's/^    \["btw-rust"\]=.*/&\n&/' "$home/services"; cp -p "$home/services" "$s/services.given"; dp "$A"
if refused 'malformed btw-rust=2/1/1' && refused 'exactly once'; then ok "btw-rust twice in SERVICES: refused, untouched"; else bad "a name twice (rc $rc)"; fi

# 7. the cutover's precheck refuses, nothing touched
fresh; HOME=$home "$home/services" stop btw-rust >/dev/null; : > "$home/.fake/calls"
for _ in $(seq 1 50); do curl -s -m 1 -o /dev/null "http://127.0.0.1:$P/" || break; sleep 0.1; done
cp -p "$home/services" "$s/services.given"; dp "$A"
if all_of '[ "$rc" = 1 ]' 'cmp -s "$s/services.given" "$home/services"' '! ls "$home"/services.bak.* >/dev/null 2>&1' \
  'grep -qF "REFUSED: :$P/healthz reports '"'"'no answer'"'"', not $OLDV" "$s/out"' '! grep -q ": build$" "$s/out"' \
  '[ ! -s "$home/.fake/calls" ]' clean; then
  ok "precheck: btw-rust not answering as its CMD says: refused before the build"
else bad "precheck, no answer (rc $rc)"; fi
fresh; sleep 300 & echo $! > "$s/extra.pid"; cp "$s/extra.pid" "$home/.fake/scope/beatscript-run"
cp -p "$home/services" "$s/services.given"; dp "$A"
if all_of '[ "$rc" = 1 ]' 'cmp -s "$s/services.given" "$home/services"' \
  'grep -qF "REFUSED: beatscript-run.scope is active, not inactive" "$s/out"' untouched_calls '[ "$(hzv)" = "$OLDV" ]' clean; then
  ok "precheck: beatscript-run already active: refused, untouched"
else bad "precheck, scope active (rc $rc)"; fi
kill "$(cat "$s/extra.pid")" 2>/dev/null || true

[ "$fails" = 0 ] && echo "deploy/test-services.sh: all passed" || { echo "deploy/test-services.sh: $fails failed"; exit 1; }
