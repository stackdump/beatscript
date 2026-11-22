#!/usr/bin/env bash
# deploy/test-live-checks.sh: how deploy/live-checks.sh and deploy/deploy.sh fail, against a fake origin on 127.0.0.1,
# with HOST=local and scratch directories, so it needs no ssh and no network. The case it pins down (the second
# cutover, 2026-11-22): a 25 s network gap on the ops machine's side made one curl time out (exit 28) inside
# `EH=$(curl ...)`; set -e exited there without a word, deploy.sh said only "restart or a live check failed" and
# rolled back, and nothing said which check, which step or which exit code.
#
#   1. a check that fails names itself: LIVE CHECK FAILED, its line, its exit code, its command
#   2. a curl with no answer inside $(...) (the cutover's case) names itself too, after its transport retries
#   3. a short gap with no answer is retried, and the answer that follows is judged as before
#   4. deploy.sh check names the step and passes live-checks' exit code on
#   5. deploy.sh all, when check fails: the failing step and its exit code, the rollback, exit with that code
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
s=$(mktemp -d)
srv=
cleanup() { [ -z "$srv" ] || kill "$srv" 2>/dev/null || true; rm -rf "$s"; }
trap cleanup EXIT
export GIT_CONFIG_GLOBAL=$s/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --global user.name test; git config --global user.email test@example.invalid
git config --global init.defaultBranch main; git config --global commit.gpgsign false; git config --global advice.detachedHead false

# a repo carrying this deploy.sh and live-checks.sh, PIN its only commit, checked out as the host's APP_DIR too
src=$s/src; mkdir -p "$src/deploy" "$src/site/test/shots"
cp "$here/deploy.sh" "$here/live-checks.sh" "$src/deploy/"
printf 'x\n' > "$src/site/test/shots/keep.png"
git -C "$src" init -q; git -C "$src" add -A; git -C "$src" commit -q -m pin
PIN=$(git -C "$src" rev-parse HEAD); V=btwserve-$(git -C "$src" rev-parse --short=7 HEAD)
home=$s/home; mkdir -p "$home"; git clone -q "$src" "$home/app"; git -C "$home/app" checkout -q --detach "$PIN"

# the fake origin: /healthz answers V; every other path is 404, except that MODE makes it close the connection with no
# answer at all (curl exit 52): `drop N` for the first N other requests, `drop-grammar` for /grammar always
cat > "$s/origin.py" <<'PY'
import http.server, os, sys
mode, version = sys.argv[1], sys.argv[2]
seen = {"n": 0}
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def answer(self):
        if self.path == "/healthz":
            body = ('{"name":"btw","ok":true,"version":"%s"}' % version).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body); return
        seen["n"] += 1
        if (mode.startswith("drop ") and seen["n"] <= int(mode.split()[1])) or (mode == "drop-grammar" and self.path == "/grammar"):
            self.close_connection = True; return
        self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers()
    do_GET = do_HEAD = do_POST = answer
s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
open(sys.argv[3], "w").write(str(s.server_address[1]))
s.serve_forever()
PY
origin() { # origin <mode>: (re)start the fake origin, set U
  [ -z "$srv" ] || { kill "$srv" 2>/dev/null || true; wait "$srv" 2>/dev/null || true; }
  rm -f "$s/port"; python3 -I "$s/origin.py" "$1" "$V" "$s/port" & srv=$!
  for _ in $(seq 1 50); do [ -s "$s/port" ] && break; sleep 0.1; done
  U=http://127.0.0.1:$(cat "$s/port")
}

fails=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; sed 's/^/     /' "$s/out"; fails=$((fails + 1)); }
export LIVE_CHECK_RETRY_S=0
lc() { rc=0; (cd "$src" && deploy/live-checks.sh "$U" "$V") >"$s/out" 2>&1 || rc=$?; }

# 1. the first page check gets a 404: the script stops there, and says so
origin "404"; lc
if [ "$rc" = 1 ] && grep -q '^/grammar 404$' "$s/out" &&
  grep -qE '^LIVE CHECK FAILED: deploy/live-checks.sh line [0-9]+ \(exit 1\): test "\$c" = 200$' "$s/out" &&
  ! grep -q 'retry' "$s/out"; then ok "a failing check names its line, exit code and command (no retry on an answer)"; else bad "failing check (rc $rc)"; fi

# 2. no answer at all, every time, inside c=$(curl ...): retried, then named with curl's exit code
origin "drop-grammar"; lc
if [ "$rc" = 52 ] && [ "$(grep -c 'curl exit 52, no answer (transport): retry' "$s/out")" = 3 ] &&
  grep -qE '^LIVE CHECK FAILED: deploy/live-checks.sh line [0-9]+ \(exit 52\): c=\$\(curl ' "$s/out"; then
  ok "no answer inside \$(curl ...): three retries, then LIVE CHECK FAILED (exit 52)"; else bad "no answer inside \$(...) (rc $rc)"; fi

# 3. a gap of two unanswered requests, then an answer: retried, and the answer (404) is judged as before
origin "drop 2"; lc
if [ "$rc" = 1 ] && [ "$(grep -c 'retry' "$s/out")" = 2 ] && grep -q '^/grammar 404$' "$s/out" &&
  grep -q 'LIVE CHECK FAILED: .*(exit 1): test "$c" = 200' "$s/out"; then
  ok "a short gap is retried; the answer after it is judged as before"; else bad "short gap (rc $rc)"; fi

# the checkout is left clean (the EXIT trap restores the screenshots) and the retry temp files are gone
if [ -z "$(git -C "$src" status --porcelain)" ]; then ok "live-checks leaves the checkout clean"; else git -C "$src" status --short > "$s/out"; bad "dirty checkout"; fi

dp() { rc=0; HOME=$home HOST=local APP_DIR=app PUBLIC_URL=$U PIN=$PIN CHROME=/bin/false "$@" >"$s/out" 2>&1 || rc=$?; }

# 4. deploy.sh check: names the step, passes the code on
origin "drop-grammar"; dp "$src/deploy/deploy.sh" check
if [ "$rc" = 52 ] && grep -q '^host checkout ' "$s/out" && grep -q 'LIVE CHECK FAILED: .*(exit 52)' "$s/out" &&
  grep -q '^deploy: deploy/live-checks.sh exited 52' "$s/out" && grep -q '^deploy: FAILED in step check: ' "$s/out"; then
  ok "deploy.sh check: names the step and exits with live-checks' code"; else bad "deploy.sh check (rc $rc)"; fi

# 5. deploy.sh all with the steps before restart, restart and rollback stubbed (they need a build and ~/services):
# the real `all` branch and the real check step
stub=$src/deploy/deploy-stubbed.sh
awk '/^case "\$\{1:-\}" in$/ { print "preflight() { say preflight; }; build() { say build; }; host() { say host; }; ship() { say ship; }; services() { say services; }"; print "restart() { say restart; echo restarted; }; rollback() { say rollback; echo rolled-back-stub; }" } { print }' \
  "$src/deploy/deploy.sh" > "$stub"; chmod +x "$stub"
origin "404"; dp "$stub" all
if [ "$rc" = 1 ] && grep -q '^deploy: step check failed (exit 1) for ' "$s/out" && grep -q '^rolled-back-stub$' "$s/out" &&
  [ "$(tail -n 1 "$s/out")" = "deploy: ROLLED BACK $V: step check failed (exit 1)" ] && ! grep -q '^DEPLOYED' "$s/out"; then
  ok "deploy.sh all: the failing step and its exit code, the rollback, then exit with that code"; else bad "deploy.sh all (rc $rc)"; fi
NO_ROLLBACK=1 dp "$stub" all
if [ "$rc" = 1 ] && ! grep -q 'rolled-back-stub' "$s/out" && grep -q 'NO_ROLLBACK=1: .* step check failed (exit 1)' "$s/out"; then
  ok "deploy.sh all, NO_ROLLBACK=1: says so, no rollback"; else bad "deploy.sh all NO_ROLLBACK (rc $rc)"; fi
rm -f "$stub"

[ "$fails" = 0 ] && echo "deploy/test-live-checks.sh: all passed" || { echo "deploy/test-live-checks.sh: $fails failed"; exit 1; }
