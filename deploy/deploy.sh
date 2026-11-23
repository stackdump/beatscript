#!/usr/bin/env bash
# deploy/deploy.sh: build beatscript.app from this repo at ONE commit and deploy it to the serving host, then run
# every live check. It runs on an ops machine with ssh to the serving host; heavy builds run on a build host
# (scripts/build-host.sh: BUILD_HOST, else this machine, else BUILD_HOSTS).
#
#   deploy/deploy.sh all          preflight, detect, build, host, ship, services, restart, check, record (the deploy)
#   deploy/deploy.sh preflight    refuse unless the tree is clean, HEAD is PIN and PIN is on origin/main
#   deploy/deploy.sh build        the wasm (site/web/wasm + gz/br sidecars), its vocals test, beatscript and beatscript-serve for
#                                 x86_64 Linux (dist/), and site/web/docs (BEATSCRIPT.md, arrange.md, cheatsheet.md)
#   deploy/deploy.sh host         APP_DIR on the host: a checkout of this repo, detached at PIN
#   deploy/deploy.sh ship         copy the wasm, the docs and the binary there (site/bin/beatscript-serve-<sha7>)
#   deploy/deploy.sh services     write this version's entry in ~/services (backup first; the entry it replaces is kept
#                                 in STATE_DIR for rollback): dir APP_DIR, CMD "cd APP_DIR && ... -data DATA_DIR ..."
#   deploy/deploy.sh restart      bring this version up; /healthz must report beatscript-serve-<sha7>
#   deploy/deploy.sh check        deploy/live-checks.sh against PUBLIC_URL (every live check, the MCP exposure table,
#                                 verify-beatscript, the live smoke)
#   deploy/deploy.sh finalize     the cutover only: take btw-rust out of ~/services once beatscript passed every check
#   deploy/deploy.sh record       append "<utc> <PIN> beatscript-serve-<sha7> ok" to DEPLOY_LOG on the host (only `all` does
#                                 this, after every check passed; the cutover's line ends "ok renamed-from btw-rust")
#   deploy/deploy.sh rollback     put back what the last `services` step replaced, bring it up, and require /healthz to
#                                 report exactly the version that was there before
#   deploy/deploy.sh status       the host checkout, the service's entry, the scopes, /healthz, the deploy log's tail
#
# The service is named beatscript (systemd scope beatscript-run). It was btw-rust (scope btw-rust-run) until the
# rename, and the deploy finds out which one it is talking to, never assumes it: ~/services must name exactly one
# of them, once in SERVICES, once as a get_service_cmd arm and once in SERVICE_ORDER.
#   btw-rust alone     this deploy is THE CUTOVER (below)
#   beatscript alone   the steady state: rewrite beatscript's CMD, ~/services restart beatscript, check
#   anything else      both names (a half-done rename), neither, or a name twice: refused before anything is
#                      touched, with the counts. A half-done cutover is finished with `finalize` or undone with
#                      `rollback`, by hand.
# Both rewrites assert that the other name is absent. ~/services is never run bare or with `help` (the bare form
# starts every service, killing the tmux session first), and never with stop/start/restart and no name: every call
# names its service. An edit of ~/services is made on a copy, checked with bash -n and moved over the file.
#
# The cutover (btw-rust -> beatscript, one deploy, every step checked; port 18099, the data dir, the env file
# ~/.env-services/btw-bitwrap and nginx's btw_front upstream stay as they are):
#   1. refuse unless ~/services names btw-rust exactly once in each place and beatscript nowhere, btw-rust's entry
#      is on PORT, :PORT/healthz answers with exactly the -version btw-rust's CMD runs, btw-rust-run.scope is
#      active and beatscript-run.scope inactive, ~/.env-services/btw-rust is the link -> btw-bitwrap, and the tmux
#      session is up (without it, `~/services start NAME` starts every service). Then back up ~/services
#      (timestamped) and write $STATE_DIR/previous-btw-rust.json (entry, CMD, version), unconditionally.
#   2. add beatscript beside btw-rust: the same port|url|dir in SERVICES, a CMD built as every deploy builds it
#      (--unit=beatscript-run, -version beatscript-serve-<sha7>, the beta flags carried over), right after btw-rust
#      in SERVICE_ORDER; ~/.env-services/beatscript -> btw-bitwrap; bash -n.
#   3. ~/services stop btw-rust, then wait until :PORT refuses connections and btw-rust-run.scope is inactive: an
#      old process still on the shared port would answer the next check.
#   4. ~/services start beatscript (start, not restart: a stopped service has no tmux window, and restart types its
#      CMD into a window that is not there), and require /healthz to report beatscript-serve-<sha7>, which the old
#      process cannot say.
#   5. deploy/live-checks.sh against PUBLIC_URL.
#   6. if 3, 4 or 5 fails: stop beatscript, take its three entries and its env link out again, start btw-rust, and
#      require /healthz to report EXACTLY the old version (from previous-btw-rust.json); "deploy: ROLLED BACK <old
#      version>: step <step> failed (exit <code>)", and exit with that code.
#   7. only after 5 passed: take btw-rust's three entries and ~/.env-services/btw-rust out, write
#      previous-beatscript.json (the old binary under the new name: what `rollback` goes back to), and log the
#      deploy with "renamed-from btw-rust".
#
# `all` rolls back by itself when a step after `services` fails (NO_ROLLBACK=1 leaves the new version up to look
# at). Every failure names itself on stderr: the step, and the line, command and exit code that stopped it; `all`
# ends a rollback with "deploy: ROLLED BACK ... step <step> failed (exit <code>)" (or "deploy: ROLLBACK FAILED ...")
# and exits with that code. (The second cutover, 2026-11-22, rolled back after a live check exited 28 without a
# word, so nothing said which step or why.) A rollback puts back the previous CMD and its own binary, but the
# checkout and the wasm stay at the new pin: for a full return, deploy the previous pin.
#
# data/ is never copied, moved or read here: beatscript-serve is pointed at DATA_DIR by absolute path. The env file
# (~/.env-services/beatscript -> btw-bitwrap: the Gemini key, the admin token, the session secret) stays on the host
# and is only ever sourced by ~/services.
#
# Environment (the defaults are the production values):
#   PIN          the commit to deploy (default HEAD; HEAD must equal it)
#   HOST         the serving host over ssh; `local`: this machine, from $HOME (the deploy tests drive it that way)
#   APP_DIR      the checkout there, relative to its $HOME        (Workspace/beatscript)
#   DATA_DIR     beatscript-serve's data dir there, relative to $HOME     (Workspace/btw-bitwrap-io/data, where it has always been)
#   PUBLIC_URL   the public origin                                (https://beatscript.app)
#   REPO_URL     what the host clones                             (https://github.com/stackdump/beatscript.git)
#   STATE_DIR    on the host: the rollback records                (.local/state/beatscript)
#   DEPLOY_LOG   on the host: one line per clean deploy           ($STATE_DIR/deploys.log)
#   PORT         the service's port, behind nginx's btw_front     (18099)
#   STOP_WAIT    seconds a stopped service has to free PORT and its scope   (30)
#   HEALTH_WAIT  seconds a started service has to report its version        (60)
#   BUILD_HOST, BUILD_HOSTS, BUILD_HOST_ALIASES, BEATSCRIPT_SMOKE_HOST   scripts/build-host.sh, scripts/build-linux.sh
#   BEATSCRIPT_BUILD_DIR, BEATSCRIPT_WASM_BUILD_DIR  the build host's scratch dirs (beatscript-web-build, beatscript-wasm-build;
#                the older BTWD_BUILD_DIR and BTW_WASM_BUILD_DIR are read when the new ones are unset)
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
PORT=${PORT:-18099}
STOP_WAIT=${STOP_WAIT:-30}
HEALTH_WAIT=${HEALTH_WAIT:-60}
# the build hosts' scratch dirs, apart from anything else that builds there
export BEATSCRIPT_BUILD_DIR=${BEATSCRIPT_BUILD_DIR:-${BTWD_BUILD_DIR:-beatscript-web-build}}
export BEATSCRIPT_WASM_BUILD_DIR=${BEATSCRIPT_WASM_BUILD_DIR:-${BTW_WASM_BUILD_DIR:-beatscript-wasm-build}}
PIN=${PIN:-$(git rev-parse HEAD)}
PIN=$(git rev-parse "$PIN^{commit}")
SHA7=$(git rev-parse --short=7 "$PIN")
VERSION=beatscript-serve-$SHA7
BIN=site/bin/beatscript-serve-$SHA7
CUTOVER=   # 1: this run is the btw-rust -> beatscript cutover (set by `all` from detect, or by a step from ~/services)

# HOST=local runs the host side here, from $HOME, as ssh would (deploy/test-host.sh and deploy/test-services.sh)
r() { if [ "$HOST" = local ]; then (cd "$HOME" && bash -c "$*"); else ssh -o BatchMode=yes "$HOST" "$@"; fi; }
say() { STEP=$1; printf '\n== deploy %s: %s\n' "$VERSION" "$*"; }

# The host side of the ~/services steps: rs <<'EOS' ... EOS runs a script on the host with HOST_LIB in front of it
# and this deploy's values in its environment.
read -r -d '' HOST_LIB <<'LIB' || true
set -euo pipefail
f=$HOME/services
OLD=btw-rust OLD_UNIT=btw-rust-run NEW=beatscript NEW_UNIT=beatscript-run
refuse() { echo "deploy: REFUSED: $*" >&2; exit 1; }
fail() { echo "deploy: $*" >&2; exit 1; }
# svc: every reading and edit of ~/services. An edit writes OUT (a copy), never ~/services itself.
svc() { python3 - "$@" <<'PY'
import json, os, re, shlex, sys
OLD, NEW = "btw-rust", "beatscript"
UNIT = {OLD: "btw-rust-run", NEW: "beatscript-run"}
ORDER_RE = re.compile(r'^SERVICE_ORDER=\((.*)\)(\s*)$')

def die(msg):
    sys.exit("deploy: services: " + msg)

def read(p):
    return open(p).read().splitlines(keepends=True)

def write(p, ls):
    open(p, "w").write("".join(ls))

def svc_ix(ls, n):
    return [k for k, l in enumerate(ls) if re.match(r'\s*\["%s"\]="' % re.escape(n), l)]

def arm_ix(ls, n):
    return [k for k, l in enumerate(ls) if re.match(r'\s*%s\)' % re.escape(n), l)]

def order_ix(ls):
    ks = [k for k, l in enumerate(ls) if l.startswith("SERVICE_ORDER=")]
    if len(ks) != 1 or not ORDER_RE.match(ls[ks[0]]):
        die(f"one SERVICE_ORDER=(...) line expected, found {len(ks)} SERVICE_ORDER lines")
    return ks[0]

def order(ls):
    m = ORDER_RE.match(ls[order_ix(ls)])
    names = re.findall(r'"([^"]*)"', m.group(1))
    if " ".join(f'"{x}"' for x in names) != m.group(1):
        die("SERVICE_ORDER is not a plain list of quoted names")
    return names

def set_order(ls, names):
    k = order_ix(ls)
    ls[k] = "SERVICE_ORDER=(" + " ".join(f'"{x}"' for x in names) + ")" + ORDER_RE.match(ls[k]).group(2)

def counts(ls, n):
    return (len(svc_ix(ls, n)), len(arm_ix(ls, n)), order(ls).count(n))

def mode(ls):
    o, n = counts(ls, OLD), counts(ls, NEW)
    if o == (1, 1, 1) and n == (0, 0, 0):
        m = "cutover"
    elif n == (1, 1, 1) and o == (0, 0, 0):
        m = "steady"
    elif o == (1, 1, 1) and n == (1, 1, 1):
        m = "both"                  # mid-cutover: steps 2-6 (finalize or rollback acts on it)
    elif any(o) and any(n):
        m = "half"                  # both names, but not each once in every place
    elif not any(o) and not any(n):
        m = "neither"
    else:
        m = "malformed"
    return m, f"{m} {OLD}={'/'.join(map(str, o))} {NEW}={'/'.join(map(str, n))} (SERVICES/CMD/SERVICE_ORDER)"

def entry(ls, n):
    s, c = svc_ix(ls, n), arm_ix(ls, n)
    if len(s) != 1 or len(c) != 1:
        die(f"one {n} SERVICES entry and one {n} CMD arm expected, found {len(s)} and {len(c)}")
    e, cmd = re.search(r'\["%s"\]="([^"]*)"' % re.escape(n), ls[s[0]]), re.search(r'CMD="([^"]*)"', ls[c[0]])
    if not e or not cmd:
        die(f"cannot read {n}'s entry or CMD")
    return s[0], c[0], e.group(1), cmd.group(1)

def flag(cmd, rx):
    m = re.search(rx, cmd)
    return m.group(1) if m else ""

def version_of(cmd):
    return flag(cmd, r' -version (\S+)')

def unit_of(cmd):
    return flag(cmd, r'--unit=(\S+)')

def svc_line(n, e):
    return f'    ["{n}"]="{e}"\n'

def arm_line(n, cmd):
    return f'        {n + ")":<16}CMD="{cmd}" ;;\n'

def build_cmd(old_cmd):
    # the one CMD every deploy writes; the Android beta sign-up is open only when a flip set both flags (a lone
    # -android-beta-url is the old direct link), so those two are carried over from the CMD it replaces
    app, data, binp, version, port, public = (os.environ[k] for k in ("APP", "DATA", "BIN", "VERSION", "PORT", "PUBLIC"))
    cmd = (f"cd {app} && systemd-run --user --scope --quiet --unit={UNIT[NEW]} -p MemoryHigh=1536M -p MemoryMax=2G ./{binp} "
           f"-addr 127.0.0.1:{port} -data {data} -log-dir {data}/log-rs -web site/web -examples site/examples "
           f"-docs site/web/docs -docs-src site/docs-src -public-url {public} -legacy-hosts btw.bitwrap.io "
           f"-render-concurrency 2 -render-max-bars 256 -render-max-seconds 600 -render-cache-mb 512 -version {version}")
    url, owners = re.search(r' -android-beta-url (\S+)', old_cmd), re.search(r' -beta-owners (\S+)', old_cmd)
    if url and owners:
        cmd = cmd.replace(' -public-url ', f' -android-beta-url {url.group(1)} -beta-owners {owners.group(1)} -public-url ', 1)
    return cmd

def words(cmd):
    # a CMD's words, less the ones that name the build (unit, binary, version)
    drop = {f"--unit={unit_of(cmd)}", version_of(cmd), flag(cmd, r' (\./\S+) -addr ')}
    return [w for w in cmd.split() if w not in drop]

def rec(path, n, e, cmd, **more):
    json.dump(dict({"service": n, "entry": e, "cmd": cmd, "version": version_of(cmd), "unit": unit_of(cmd)}, **more),
              open(path, "w"), indent=1)

def want(ls, m):
    got = mode(ls)
    if got[0] != m:
        die(f"~/services is not what this step needs ({m}): {got[1]}")

op, a = sys.argv[1], sys.argv[2:]
if op == "mode":                                  # mode FILE
    print(mode(read(a[0]))[1])
elif op == "vars":                                # vars FILE NAME PREFIX: shell assignments
    _, _, e, cmd = entry(read(a[0]), a[1])
    for k, v in (("entry", e), ("cmd", cmd), ("version", version_of(cmd)), ("unit", unit_of(cmd)), ("port", e.split("|")[0])):
        print(f"{a[2]}_{k}={shlex.quote(v)}")
elif op == "record":                              # record FILE NAME JSON: NAME's rollback record, unconditionally
    _, _, e, cmd = entry(read(a[0]), a[1])
    rec(a[2], a[1], e, cmd)
    print(f"recorded {a[1]} ({version_of(cmd)}): {a[2]}")
elif op == "cutover-add":                         # cutover-add FILE OUT: beatscript beside btw-rust (step 2)
    ls = read(a[0]); want(ls, "cutover")
    si, ci, e, old = entry(ls, OLD)
    if e.split("|")[0] != os.environ["PORT"]:
        die(f"{OLD}'s entry is on port {e.split('|')[0]}, not {os.environ['PORT']}")
    new = build_cmd(old)
    names = order(ls); names.insert(names.index(OLD) + 1, NEW); set_order(ls, names)
    ls.insert(ci + 1, arm_line(NEW, new))
    ls.insert(si + 1, svc_line(NEW, e))
    want(ls, "both")
    write(a[1], ls)
    print(f"{NEW} entry: {e}\n{NEW} CMD:   {new}\nSERVICE_ORDER: ... {OLD} {NEW} ...")
    lost, gained = [w for w in words(old) if w not in words(new)], [w for w in words(new) if w not in words(old)]
    if lost or gained:
        print(f"note: words of {OLD}'s CMD the new one drops: {' '.join(lost) or '-'}; adds: {' '.join(gained) or '-'}")
elif op == "remove":                              # remove FILE OUT NAME: NAME's three entries out, the other kept
    ls = read(a[0]); n = a[2]; other = OLD if n == NEW else NEW
    want(ls, "both")
    si, ci, e, cmd = entry(ls, n)
    set_order(ls, [x for x in order(ls) if x != n])
    for k in sorted((si, ci), reverse=True):
        del ls[k]
    if counts(ls, n) != (0, 0, 0) or counts(ls, other) != (1, 1, 1):
        die(f"removing {n} did not leave {other} alone: {mode(ls)[1]}")
    write(a[1], ls)
    print(f"removed {n} (SERVICES, CMD, SERVICE_ORDER): it ran {version_of(cmd)}")
elif op == "steady-set":                          # steady-set FILE OUT JSON: this version's beatscript CMD
    ls = read(a[0]); want(ls, "steady")
    si, ci, e, old = entry(ls, NEW)
    port, url, _ = e.split("|")
    if port != os.environ["PORT"]:
        die(f"{NEW}'s entry is on port {port}, not {os.environ['PORT']}")
    cmd = build_cmd(old)
    # keep the previous entry once per distinct CMD: a rerun of the same deploy must not overwrite it with itself
    if old != cmd:
        rec(a[2], NEW, e, old)
    app, home = os.environ["APP"], os.environ["HOME"]
    d = app.replace(home, "$HOME", 1) if app.startswith(home + "/") else app
    ls[si], ls[ci] = svc_line(NEW, f"{port}|{url}|{d}"), arm_line(NEW, cmd)
    write(a[1], ls)
    print(f"{NEW} entry was: {e}\n{NEW} CMD was:   {old}\n{NEW} entry now: {port}|{url}|{d}\n{NEW} CMD now:   {cmd}")
elif op == "steady-restore":                      # steady-restore FILE OUT JSON: the recorded CMD back
    ls = read(a[0]); want(ls, "steady")
    prev = json.load(open(a[2]))
    if prev.get("service") != NEW:
        die(f"{a[2]} is a record of {prev.get('service')!r}, not {NEW}")
    si, ci, _, _ = entry(ls, NEW)
    cmd, d = prev["cmd"], prev["entry"].split("|")[2].replace("$HOME", os.environ["HOME"], 1)
    # ~/services restart reuses the tmux window and its cwd: a CMD with relative paths must cd first
    if not cmd.startswith("cd "):
        cmd = f"cd {d} && {cmd}"
    if not version_of(cmd):
        die(f"the recorded CMD names no -version: a rollback to it could not be proven")
    ls[si], ls[ci] = svc_line(NEW, prev["entry"]), arm_line(NEW, cmd)
    write(a[1], ls)
    print(f"{NEW} entry back to: {prev['entry']}\n{NEW} CMD back to:   {cmd}")
elif op == "renamed-record":                      # renamed-record OLDJSON NEWJSON: the old binary under the new name
    o = json.load(open(a[0]))
    cmd = o["cmd"].replace(f"--unit={o.get('unit') or UNIT[OLD]}", f"--unit={UNIT[NEW]}", 1)
    rec(a[1], NEW, o["entry"], cmd, renamed_from=OLD)
    print(f"recorded {NEW}'s previous version: {version_of(cmd)} (the {OLD} binary, as {UNIT[NEW]}): {a[1]}")
else:
    die(f"unknown op {op}")
PY
}
# svcctl ACTION NAME: ~/services with a service name, always (bare, `help`, or an action without a name acts on
# every service)
svcctl() {
  case "${1:-}" in stop|start|restart) ;; *) fail "svcctl: refusing ~/services ${1:-<bare>}" ;; esac
  [ -n "${2:-}" ] || fail "svcctl: refusing ~/services $1 without a service name"
  "$f" "$1" "$2" </dev/null
}
# install_services COPY: bash -n the edited copy, then move it over ~/services (a rename: a ~/services that is running
# now keeps reading the file it started with)
install_services() {
  bash -n "$1" || { rm -f "$1"; fail "the edited ~/services does not pass bash -n: ~/services left as it was"; }
  [ ! -L "$f" ] || { rm -f "$1"; fail "~/services is a symlink: not replacing it"; }
  mv -f "$1" "$f"
}
edit() { # edit OP ARGS...: svc OP on a copy of ~/services, then install it
  local new=$f.beatscript-new op=$1; shift
  cp -p "$f" "$new"
  svc "$op" "$f" "$new" "$@" || { rm -f "$new"; return 1; }
  install_services "$new"
}
backup() { local b; b="$f.bak.beatscript-$1-$(date +%Y%m%d%H%M%S)"; cp -p "$f" "$b"; echo "backup: $b"; }
hz() { curl -s -m 3 "http://127.0.0.1:$PORT/healthz" 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin).get("version",""))' 2>/dev/null || true; }
hz_show() { curl -s -m 5 "http://127.0.0.1:$PORT/healthz" || true; echo; }
scope() { local s; s=$(systemctl --user is-active "$1.scope" </dev/null 2>/dev/null) || true; echo "${s:-unknown}"; }
scope_down() { case $(scope "$1") in inactive|failed) return 0 ;; *) return 1 ;; esac; }
port_refused() { local rc=0; curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/healthz" || rc=$?; [ "$rc" = 7 ]; }
down() { port_refused && scope_down "$1"; }
wait_down() { local i; for i in $(seq 1 "$STOP_WAIT"); do down "$1" && return 0; sleep 1; done; down "$1"; }
wait_version() { local i; for i in $(seq 1 "$HEALTH_WAIT"); do [ "$(hz)" = "$1" ] && return 0; sleep 1; done; [ "$(hz)" = "$1" ]; }
session_up() {
  local s; s=$(sed -n 's/^SESSION="\([^"]*\)".*/\1/p' "$f" | head -n 1)
  [ -n "$s" ] && tmux has-session -t "$s" </dev/null 2>/dev/null
}
# start_service NAME UNIT: ~/services stop NAME (nothing to do when it has no window), wait until PORT and UNIT are
# free, then ~/services start NAME
start_service() {
  svcctl stop "$1"
  wait_down "$2" || fail ":$PORT still answers or $2.scope is still $(scope "$2") after $STOP_WAIT s: $1 cannot start"
  [ "$(scope "$2")" != failed ] || systemctl --user reset-failed "$2.scope" </dev/null
  svcctl start "$1"
}
mode_word() { local m; m=$(svc mode "$f"); echo "${m%% *}"; }
cutover_check() { # step 1's refusals; reads only
  local m v st vars e=$HOME/.env-services
  m=$(svc mode "$f"); [ "${m%% *}" = cutover ] || refuse "~/services is not btw-rust alone: $m"
  vars=$(svc vars "$f" "$OLD" old); eval "$vars"
  [ "$old_port" = "$PORT" ] || refuse "btw-rust's SERVICES entry is on port $old_port, not $PORT"
  [ -n "$old_version" ] || refuse "btw-rust's CMD has no -version: a rollback could not prove the old version is back"
  [ -n "$old_unit" ] && [ "$old_unit" != "$NEW_UNIT" ] || refuse "btw-rust's CMD runs as systemd unit '$old_unit'"
  session_up || refuse "the tmux session ~/services runs in is down (~/services start NAME would start every service)"
  v=$(hz); [ "$v" = "$old_version" ] || refuse ":$PORT/healthz reports '${v:-no answer}', not $old_version (btw-rust's CMD): a rollback could not prove the old version is back"
  st=$(scope "$old_unit"); [ "$st" = active ] || refuse "$old_unit.scope is $st, not active"
  st=$(scope "$NEW_UNIT"); [ "$st" = inactive ] || refuse "$NEW_UNIT.scope is $st, not inactive (systemd-run --unit=$NEW_UNIT would fail; a failed one: systemctl --user reset-failed $NEW_UNIT.scope)"
  [ -f "$e/btw-bitwrap" ] && [ ! -L "$e/btw-bitwrap" ] || refuse "no env file $e/btw-bitwrap"
  [ -L "$e/$OLD" ] && [ "$(readlink "$e/$OLD")" = btw-bitwrap ] || refuse "$e/$OLD is not the link -> btw-bitwrap (step 7 deletes it, so it must be a link, never the env file itself)"
  if [ -e "$e/$NEW" ] || [ -L "$e/$NEW" ]; then
    [ -L "$e/$NEW" ] && [ "$(readlink "$e/$NEW")" = btw-bitwrap ] || refuse "$e/$NEW exists and is not the link -> btw-bitwrap"
  fi
  echo "precheck: btw-rust serves $old_version on :$PORT from $old_unit.scope; $NEW_UNIT.scope inactive; $e/$OLD -> btw-bitwrap; tmux session up"
}
LIB
rs() { { printf '%s\n' "$HOST_LIB"; cat; } | r "APP=\$HOME/$APP_DIR DATA=\$HOME/$DATA_DIR STATE=\$HOME/$STATE_DIR LOG=\$HOME/$DEPLOY_LOG BIN=$BIN VERSION=$VERSION PORT=$PORT PUBLIC=$PUBLIC_URL STOP_WAIT=$STOP_WAIT HEALTH_WAIT=$HEALTH_WAIT bash -s"; }
# the first word of `svc mode` on the host now: cutover, steady, both, half, neither or malformed
mode_now() { rs <<<'mode_word'; }

preflight() {
  say "preflight"
  [ "$(git rev-parse HEAD)" = "$PIN" ] || { echo "deploy: HEAD is $(git rev-parse --short HEAD), not PIN $SHA7: check out the pin first" >&2; exit 1; }
  git diff --quiet && git diff --cached --quiet || { echo "deploy: the tree has uncommitted changes: deploy a commit, not a working tree" >&2; exit 1; }
  git fetch -q origin main
  git merge-base --is-ancestor "$PIN" origin/main || { echo "deploy: $SHA7 is not on origin/main: push it (or merge it) first" >&2; exit 1; }
  echo "pin $PIN ($VERSION), on origin/main $(git rev-parse --short origin/main)"
}

# detect: which service ~/services runs. Prints cutover or steady on stdout; anything else is refused here, before
# the build and before anything on the host is touched.
detect() {
  local out
  out=$(rs <<<'svc mode "$f"') || return
  printf '%s\n' "$out" >&2
  case ${out%% *} in
  cutover) echo "deploy: ~/services runs btw-rust: this deploy is the cutover to beatscript" >&2; echo cutover ;;
  steady) echo steady ;;
  both) echo "deploy: REFUSED: ~/services names both btw-rust and beatscript (a half-done rename): finish it with deploy/deploy.sh finalize, or undo it with deploy/deploy.sh rollback, first" >&2; return 1 ;;
  half) echo "deploy: REFUSED: ~/services names both btw-rust and beatscript, and not each once in SERVICES, CMD and SERVICE_ORDER: put it right by hand (backups: ~/services.bak.beatscript-*)" >&2; return 1 ;;
  neither) echo "deploy: REFUSED: ~/services names neither btw-rust nor beatscript" >&2; return 1 ;;
  *) echo "deploy: REFUSED: ~/services must name btw-rust or beatscript exactly once in SERVICES, once as a CMD arm and once in SERVICE_ORDER" >&2; return 1 ;;
  esac
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

# --- the cutover: btw-rust -> beatscript (steps 1-7 in the header) -------------------------------------------------

precheck() {
  say precheck "(the cutover, step 1: refuse unless btw-rust is whole and beatscript absent)"
  rs <<<'cutover_check'
}

services_cutover() {
  # (precheck, step 1's refusals, runs just before this; the rollback record is written first, so that a failure
  # anywhere after it rolls back to what this run found)
  say services "(the cutover, steps 1-2: back up, record btw-rust, add beatscript beside it)"
  rs <<'EOS'
mkdir -p "$STATE"
backup cutover
svc record "$f" "$OLD" "$STATE/previous-btw-rust.json"
test -x "$APP/$BIN" || fail "no executable $APP/$BIN: ship first"
edit cutover-add
ln -sfn btw-bitwrap "$HOME/.env-services/$NEW"
echo "env: $NEW -> $(readlink "$HOME/.env-services/$NEW")"
bash -n "$f" && echo "services syntax ok: $(svc mode "$f")"
EOS
}

restart_cutover() {
  say restart "(the cutover, steps 3-4: stop btw-rust, start beatscript)"
  rs <<'EOS'
[ "$(mode_word)" = both ] || refuse "~/services is not mid-cutover: $(svc mode "$f")"
vars=$(svc vars "$f" "$OLD" old); eval "$vars"
vars=$(svc vars "$f" "$NEW" new); eval "$vars"
[ "$new_version" = "$VERSION" ] || refuse "beatscript's CMD runs $new_version, not $VERSION"
session_up || refuse "the tmux session ~/services runs in is down (~/services start NAME would start every service)"
svcctl stop "$OLD"
wait_down "$old_unit" || fail "btw-rust did not stop within $STOP_WAIT s: :$PORT $(port_refused && echo refuses || echo 'still answers'), $old_unit.scope $(scope "$old_unit")"
echo "btw-rust stopped: :$PORT refuses connections, $old_unit.scope $(scope "$old_unit")"
start_service "$NEW" "$NEW_UNIT"
wait_version "$VERSION" || true
hz_show
v=$(hz); [ "$v" = "$VERSION" ] || fail ":$PORT/healthz reports '${v:-no answer}', not $VERSION"
[ "$(scope "$NEW_UNIT")" = active ] || fail "$NEW_UNIT.scope is $(scope "$NEW_UNIT"), not active"
scope_down "$old_unit" || fail "$old_unit.scope is $(scope "$old_unit") again"
echo "beatscript serves $VERSION from $NEW_UNIT.scope (btw-rust was stopped first: this answer is the new process)"
EOS
}

finalize() {
  [ "$CUTOVER" = 1 ] || { echo "deploy: finalize: ~/services is not mid-cutover (it does not name both btw-rust and beatscript): nothing to finalize" >&2; return 1; }
  say finalize "(the cutover, step 7: take btw-rust out)"
  rs <<'EOS'
[ "$(mode_word)" = both ] || refuse "~/services is not mid-cutover: $(svc mode "$f")"
vars=$(svc vars "$f" "$OLD" old); eval "$vars"
vars=$(svc vars "$f" "$NEW" new); eval "$vars"
[ "$new_version" = "$VERSION" ] || refuse "beatscript's CMD runs $new_version, not $VERSION"
v=$(hz); [ "$v" = "$VERSION" ] || refuse ":$PORT/healthz reports '${v:-no answer}', not $VERSION"
[ "$(scope "$NEW_UNIT")" = active ] || refuse "$NEW_UNIT.scope is $(scope "$NEW_UNIT"), not active"
scope_down "$old_unit" || refuse "$old_unit.scope is $(scope "$old_unit"), not inactive"
[ -f "$STATE/previous-btw-rust.json" ] || refuse "no $STATE/previous-btw-rust.json"
# (its window went in step 3; once its entry is gone ~/services can no longer stop it, so make sure now)
svcctl stop "$OLD"
backup finalize
edit remove "$OLD"
e=$HOME/.env-services/$OLD
if [ -L "$e" ]; then rm -f "$e"; echo "removed the env link $e"; elif [ -e "$e" ]; then echo "deploy: $e is not a link: left as it is" >&2; fi
svc renamed-record "$STATE/previous-btw-rust.json" "$STATE/previous-beatscript.json"
bash -n "$f" && echo "services syntax ok: $(svc mode "$f")"
EOS
}

rollback_cutover() {
  say rollback "(the cutover, step 6: back to btw-rust)"
  rs <<'EOS'
prev=$STATE/previous-btw-rust.json
[ -f "$prev" ] || refuse "no $prev: nothing recorded to roll back to"
want=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$prev")
[ -n "$want" ] || refuse "$prev names no version"
m=$(mode_word)
case $m in both|cutover) ;; *) refuse "~/services is $(svc mode "$f"): not a cutover to roll back" ;; esac
vars=$(svc vars "$f" "$OLD" old); eval "$vars"
# the cutover never edits btw-rust's lines: a record of another version is stale, and acting on it could never be
# proven, so nothing is touched
[ "$old_version" = "$want" ] || refuse "btw-rust's CMD runs $old_version but $prev says $want: not rolling back on a stale record"
session_up || refuse "the tmux session ~/services runs in is down (~/services start NAME would start every service)"
if [ "$m" = both ]; then
  svcctl stop "$NEW"
  i=0; until scope_down "$NEW_UNIT"; do i=$((i + 1)); [ "$i" -le "$STOP_WAIT" ] || break; sleep 1; done
  backup rollback
  edit remove "$NEW"
fi
e=$HOME/.env-services/$NEW
if [ -L "$e" ]; then rm -f "$e"; echo "removed the env link $e"; fi
bash -n "$f" && echo "services syntax ok: $(svc mode "$f")"
if [ "$(hz)" = "$want" ] && [ "$(scope "$old_unit")" = active ]; then
  echo "btw-rust still serves $want: left running"
else
  start_service "$OLD" "$old_unit"
  wait_version "$want" || true
fi
hz_show
v=$(hz); [ "$v" = "$want" ] || fail "rollback: :$PORT/healthz reports '${v:-no answer}', not exactly $want"
echo "rollback: btw-rust serves $want again (exactly); ~/services: $(svc mode "$f")"
EOS
}

old_version() { rs <<<'python3 -c "import json,sys; print(json.load(open(sys.argv[1]))[\"version\"])" "$STATE/previous-btw-rust.json"'; }

# --- the steady state: beatscript alone ---------------------------------------------------------------------------

services_steady() {
  say services
  rs <<'EOS'
[ "$(mode_word)" = steady ] || refuse "~/services is not beatscript alone: $(svc mode "$f")"
e=$HOME/.env-services/$NEW
[ ! -e "$e" ] || [ -L "$e" ] || refuse "$e is a file, not the link -> btw-bitwrap"
ln -sfn btw-bitwrap "$e" && echo "env: $NEW -> $(readlink "$e")"
mkdir -p "$STATE"
backup deploy
edit steady-set "$STATE/previous-beatscript.json"
echo "services syntax ok"
EOS
}

restart_steady() {
  say restart
  rs <<'EOS'
[ "$(mode_word)" = steady ] || refuse "~/services is not beatscript alone: $(svc mode "$f")"
session_up || refuse "the tmux session ~/services runs in is down (~/services restart NAME would start every service)"
svcctl restart "$NEW" >/dev/null
wait_version "$VERSION" || true
hz_show
v=$(hz); [ "$v" = "$VERSION" ] || fail ":$PORT/healthz reports '${v:-no answer}', not $VERSION"
EOS
}

rollback_steady() {
  say rollback
  rs <<'EOS'
prev=$STATE/previous-beatscript.json
[ -f "$prev" ] || refuse "no $prev: nothing recorded to roll back to"
[ "$(mode_word)" = steady ] || refuse "~/services is not beatscript alone: $(svc mode "$f")"
session_up || refuse "the tmux session ~/services runs in is down (~/services restart NAME would start every service)"
backup rollback
edit steady-restore "$prev"
vars=$(svc vars "$f" "$NEW" cur); eval "$vars"
svcctl restart "$NEW" >/dev/null
wait_version "$cur_version" || true
hz_show
v=$(hz); [ "$v" = "$cur_version" ] || fail "rollback: :$PORT/healthz reports '${v:-no answer}', not exactly $cur_version (the version before)"
echo "rollback: beatscript serves $cur_version again (exactly)"
EOS
}

# --- the steps by name: the cutover's or the steady state's -------------------------------------------------------

services() { if [ "$CUTOVER" = 1 ]; then services_cutover; else services_steady; fi; }
restart() { if [ "$CUTOVER" = 1 ]; then restart_cutover; else restart_steady; fi; }
rollback() { if [ "$CUTOVER" = 1 ]; then rollback_cutover; else rollback_steady; fi; }

check() {
  say check "(the live checks)"
  # (called inside `if` by `all`, where set -e is off: every step returns its own failure)
  local rc
  r "cd \$HOME/$APP_DIR && test \"\$(git rev-parse HEAD)\" = $PIN && echo \"host checkout $SHA7\"" || { rc=$?; echo "deploy: the host checkout is not $SHA7 (exit $rc)" >&2; return "$rc"; }
  CHROME=${CHROME:-/usr/bin/google-chrome} deploy/live-checks.sh "$PUBLIC_URL" "$VERSION" || { rc=$?; echo "deploy: deploy/live-checks.sh exited $rc (its LIVE CHECK FAILED line above says which check)" >&2; return "$rc"; }
}

record() { # record [note]: the clean deploy's line, "<utc> <PIN> <version> ok[ note]"
  say record
  r "mkdir -p \$(dirname \$HOME/$DEPLOY_LOG) && echo \"\$(date -u +%Y-%m-%dT%H:%M:%SZ) $PIN $VERSION ok${1:+ $1}\" >> \$HOME/$DEPLOY_LOG && tail -n 3 \$HOME/$DEPLOY_LOG"
}

status() {
  rs <<'EOS'
(cd "$APP" 2>/dev/null && git log -1 --format='checkout %h %s') || echo "no checkout at $APP"
svc mode "$f"
grep -nE '^ *(\["(btw-rust|beatscript)"\]=|(btw-rust|beatscript)\))' "$f" | cut -c1-240
for u in "$OLD_UNIT" "$NEW_UNIT"; do echo "$u.scope $(scope "$u")"; done
hz_show
tail -n 5 "$LOG" 2>/dev/null || echo 'no deploy log yet'
EOS
}

case "${1:-}" in
preflight|build|host|ship|check|record|status) "$1" ;;
detect) detect >/dev/null ;;
services)
  m=$(detect); [ "$m" != cutover ] || CUTOVER=1
  [ "$CUTOVER" != 1 ] || precheck
  services ;;
restart|rollback|finalize)
  # mid-cutover (both names) is the cutover's step; beatscript alone is the steady state's
  m=$(mode_now)
  case $m in
  both) CUTOVER=1 ;;
  steady) ;;
  cutover) [ "$1" = rollback ] && CUTOVER=1 || { echo "deploy: $1: ~/services runs btw-rust alone: no cutover in flight (deploy/deploy.sh all is the cutover)" >&2; exit 1; } ;;
  *) echo "deploy: $1: ~/services is $m: refusing" >&2; exit 1 ;;
  esac
  "$1" ;;
all)
  preflight
  STEP=detect; m=$(detect); [ "$m" != cutover ] || CUTOVER=1
  # the cutover's refusals before the build (fast), and again just before the first write (the host may have moved)
  [ "$CUTOVER" != 1 ] || precheck
  build; host; ship
  if [ "$CUTOVER" = 1 ]; then
    precheck
    if services && restart && check; then
      finalize || { rc=$?; echo "deploy: $VERSION serves and passed every check, but step finalize failed (exit $rc): ~/services still names btw-rust beside beatscript; deploy/deploy.sh finalize finishes it (no rollback: the new version is good)" >&2; exit "$rc"; }
      record "renamed-from btw-rust"
      echo; echo "DEPLOYED $VERSION ($PIN) to $PUBLIC_URL from stackdump/beatscript; the service is beatscript now (renamed from btw-rust)"
      exit 0
    else
      rc=$? failed=$STEP
    fi
  else
    services
    if restart && check; then
      record
      echo; echo "DEPLOYED $VERSION ($PIN) to $PUBLIC_URL from stackdump/beatscript"
      exit 0
    else
      rc=$? failed=$STEP
    fi
  fi
  echo "deploy: step $failed failed (exit $rc) for $VERSION${CUTOVER:+ (the cutover from btw-rust)}" >&2
  if [ "${NO_ROLLBACK:-0}" = 1 ]; then
    echo "deploy: NO_ROLLBACK=1: $VERSION left up; step $failed failed (exit $rc)${CUTOVER:+; ~/services names btw-rust and beatscript until deploy/deploy.sh rollback or finalize}" >&2
  elif ! rollback; then
    echo "deploy: ROLLBACK FAILED after step $failed failed (exit $rc): look at ~/services and :$PORT/healthz on $HOST now" >&2
  elif [ "$CUTOVER" = 1 ]; then
    echo "deploy: ROLLED BACK $(old_version): step $failed failed (exit $rc)" >&2
  else
    echo "deploy: ROLLED BACK $VERSION: step $failed failed (exit $rc)" >&2
  fi
  exit "$rc"
  ;;
*) awk 'NR > 1 && /^set -eEuo/ { exit } NR > 1' "$0"; exit 2 ;;
esac
