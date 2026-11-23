#!/bin/sh
# build-btwd-linux.sh: a release `btwd` for x86_64 Ubuntu 24.04 (glibc 2.39: the deploy host pflow.dev), built on
# a build host (scripts/build-host.sh: BUILD_HOST, else this machine, else BUILD_HOSTS), with no system libraries
# beyond libc.
#
#   scripts/build-btwd-linux.sh [--clean] [--no-smoke]
#
# Writes dist/btwd-linux-x86_64 and dist/btwd-linux-x86_64.sha256 (check with `cd dist && shasum -a 256 -c
# btwd-linux-x86_64.sha256`). dist/ is gitignored: the binary is never committed; deploy/deploy.sh copies it to the
# serving host.
#
# Two ways to build, by the build host's OS:
#
# A Mac: cross-compiled with cargo-zigbuild for --target x86_64-unknown-linux-gnu.2.39 (zig links against glibc
#   2.39's symbol versions, so the binary needs nothing newer than the deploy host has). No docker.
#   - On the Mac itself: in place, into target-linux/ (BTWD_TARGET_DIR overrides; target-* is git-ignored), so the
#     next build is incremental.
#   - From another machine (the deploy orchestrator): rsync this tree into ~/btw-web-build/btw-web on the Mac
#     (never target/, .git/ or dist/), run this script there with BUILD_HOST=local, and copy the binary back. The
#     engine (stackdump/djrust) is a git dependency pinned in Cargo.toml: cargo fetches it there over HTTPS.
#   Needs zig (brew install zig), cargo-zigbuild (cargo install cargo-zigbuild), rustup's x86_64-unknown-linux-gnu
#   target for the stable toolchain (rust-toolchain.toml's), and for the ssh route Homebrew's GNU rsync.
#   The Mac cannot run the binary: the smoke runs on BTWD_SMOKE_HOST (see below).
#   A crate whose build script will not cross-compile (a C dependency: ring, openssl-sys, an audio -sys crate) is
#   named in the error; build that one on a Linux host (BUILD_HOST=<linux host>).
#
# A Linux host: in an ubuntu:24.04 container, as before.
#   1. rsync this tree into ~/btw-web-build/btw-web on the build host (as above).
#   2. builds the image btwd-build:ubuntu24.04 once (ubuntu:24.04 + gcc + rustup stable): the toolchain is
#      in the image, the crate registry and target/ are kept under the scratch dir, so a second build is
#      incremental.
#   3. `cargo build --release -p btwd --locked` in that container, as the host user (no root-owned files),
#      symbols stripped. The container's glibc is the deploy host's (2.39), so the binary needs nothing newer.
#   4. copies the binary back, and (unless --no-smoke) runs it in a fresh ubuntu:24.04 container with
#      nothing installed: `ldd`, then a render of a corpus program, timed.
#
# Either way the script checks the highest GLIBC symbol version the binary asks for (objdump -T: binutils on
# Linux, LLVM's /usr/bin/objdump on a Mac) and refuses one above 2.39.
#   --clean removes the scratch dir (or, in place on the Mac, target-linux/) and the image's build cache from
#   the build host afterwards (for a one-off on a host short of disk).
#
# The smoke of a cross-compiled binary (unless --no-smoke): scp it and a corpus program to BTWD_SMOKE_HOST (an ssh
# host that is x86_64 Linux; `local` = this machine, which must be x86_64 Linux; unset: this machine when it is
# x86_64 Linux), then `ldd` and the libc there, and for btwd `btwd --version` and `btwd render < program` (exit 0
# and a nonzero byte count, timed); for btwserve a start with `-version smoke`, GET /healthz (it must report that
# version), and a stop. Nothing is left on the smoke host.
#
# Why glibc and not musl: it is exactly the libc of the host it runs on, the toolchain is the stock one, and the
# render is a tight float loop with allocations that musl's allocator would only slow down. A musl build
# (`--target x86_64-unknown-linux-musl`) is a one-line change if a host ever has an older glibc.
#
# Environment: BUILD_HOST, BUILD_HOSTS (scripts/build-host.sh; BTWD_BUILD_HOST is the older name of BUILD_HOST),
# BTWD_BUILD_DIR (default btw-web-build, relative to the build host's $HOME), BTWD_TARGET_DIR (the Mac's in-place
# target dir, default <repo>/target-linux), BTWD_SMOKE_HOST (above), BTWD_UNLOCKED=1 (drop --locked: for a
# Cargo.lock that is mid-edit), BTWD_PKG=btwserve (build the server instead: dist/btwserve-linux-x86_64; its smoke
# is a server start and stop, in the bare container on a Linux build host, with /healthz on the smoke host for a
# cross-compiled one).
set -eu

PKG=${BTWD_PKG:-btwd}
case $PKG in btwd|btwserve) ;; *) echo "build-btwd-linux: BTWD_PKG is btwd or btwserve" >&2; exit 2 ;; esac
SCRATCH=${BTWD_BUILD_DIR:-btw-web-build}
IMAGE=btwd-build:ubuntu24.04
# the deploy host's glibc, and the zig target that links against exactly it
GLIBC_MAX=2.39
ZIG_TARGET=x86_64-unknown-linux-gnu.$GLIBC_MAX
CLEAN=0
SMOKE=1
for a in "$@"; do
  case $a in
    --clean) CLEAN=1 ;;
    --no-smoke) SMOKE=0 ;;
    -h|--help) awk 'NR > 1 && /^set -eu$/ { exit } NR > 1' "$0"; exit 0 ;;
    *) echo "build-btwd-linux: unknown argument $a" >&2; exit 2 ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd -P)
dist="$root/dist"
mkdir -p "$dist"
smoke=$(ls "$root"/crates/btwapi/tests/corpus/markers-demo.btw 2>/dev/null || ls "$root"/examples/*.btw | head -n 1)

[ -n "${BUILD_HOST:-}" ] || BUILD_HOST=${BTWD_BUILD_HOST:-}
. "$root/scripts/build-host.sh"
pick_build_host darwin:cargo darwin:zig darwin:cargo-zigbuild "darwin:rust-target=x86_64-unknown-linux-gnu" linux:docker || exit 2
HOST=$BH_NAME
case $BH_OS in
  Darwin) METHOD=zig ;;
  Linux) METHOD=docker ;;
  *) echo "build-btwd-linux: no way to build on $HOST ($BH_OS)" >&2; exit 2 ;;
esac
RS="$BH_RSYNC -a --delete --delete-excluded --exclude=target/ --exclude=/target-*/ --exclude=.git/ --exclude=/dist/ --exclude=*.wav --exclude=*.sf2"
LOCKED=--locked
[ "${BTWD_UNLOCKED:-0}" = 1 ] && LOCKED=

# the newest GLIBC_x.y the binary asks for, and a refusal when that is newer than the deploy host's
glibc_check() {
  need=$(objdump -T "$1" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -u |
    awk -F. '{ v = $1 * 1000000 + $2 * 1000 + $3; if (v > m) { m = v; s = $0 } } END { print s }')
  [ -n "$need" ] || { echo "build-btwd-linux: objdump -T shows no GLIBC symbol version in $1" >&2; return 1; }
  echo "-- newest glibc symbol version: GLIBC_$need (the deploy host has $GLIBC_MAX)"
  if ! awk -v a="$need" -v b="$GLIBC_MAX" 'BEGIN { split(a, x, "."); split(b, y, ".");
      exit !(x[1] * 1000000 + x[2] * 1000 + x[3] <= y[1] * 1000000 + y[2] * 1000 + y[3]) }'; then
    echo "build-btwd-linux: the binary needs GLIBC_$need, newer than the deploy host's glibc $GLIBC_MAX" >&2
    return 1
  fi
}

fetched() {
  chmod 755 "$dist/$PKG-linux-x86_64"
  ( cd "$dist" && shasum -a 256 "$PKG-linux-x86_64" > "$PKG-linux-x86_64.sha256" )
  cat "$dist/$PKG-linux-x86_64.sha256"
  ls -l "$dist/$PKG-linux-x86_64" | awk '{printf "size %.1f MB\n", $5/1048576}'
}

# the smoke of a cross-compiled binary, on an x86_64 Linux host that is not the Mac
zig_smoke() {
  SH=
  for n in $(bh_names "${BTWD_SMOKE_HOST:-local}"); do
    if [ "$n" = local ] || bh_is_self "$n"; then SH=local; break; fi
    if ssh -o BatchMode=yes -o ConnectTimeout=5 "$n" true 2>/dev/null; then SH=$n; break; fi
  done
  if [ -z "$SH" ] && [ "$(uname -s)/$(uname -m)" = Linux/x86_64 ]; then
    echo "-- smoke host ${BTWD_SMOKE_HOST:-local} did not answer: smoke here"
    SH=local
  fi
  [ -n "$SH" ] || { echo "build-btwd-linux: no smoke host (${BTWD_SMOKE_HOST:-local} did not answer, and this is not x86_64 Linux): BTWD_SMOKE_HOST=<host>, or --no-smoke" >&2; return 1; }
  echo "== smoke on $SH"
  if [ "$SH" = local ]; then
    sdir=$(mktemp -d)
    cp "$dist/$PKG-linux-x86_64" "$sdir/$PKG"; cp "$smoke" "$sdir/smoke.btw"
    run_smoke() { (cd "$sdir" && PKG=$PKG sh -s); }
  else
    sdir=$(ssh -o BatchMode=yes "$SH" 'mktemp -d')
    scp -q "$dist/$PKG-linux-x86_64" "$SH:$sdir/$PKG"; scp -q "$smoke" "$SH:$sdir/smoke.btw"
    run_smoke() { ssh -o BatchMode=yes "$SH" "cd '$sdir' && PKG=$PKG sh -s"; }
  fi
  rc=0
  run_smoke <<'SMOKE' || rc=$?
set -eu
echo "-- $(uname -n), $(ldd --version 2>&1 | head -n 1)"
echo "-- ldd"; ldd "./$PKG"
if [ "$PKG" = btwserve ]; then
  port=$(( 20000 + $$ % 10000 ))
  mkdir -p data
  ./btwserve -addr "127.0.0.1:$port" -data ./data -log-dir off -version smoke >log 2>&1 &
  pid=$!
  i=0
  until curl -fsS "http://127.0.0.1:$port/healthz" >healthz 2>/dev/null; do
    i=$((i + 1))
    if [ $i -ge 100 ] || ! kill -0 $pid 2>/dev/null; then cat log; kill $pid 2>/dev/null || true; echo "btwserve did not answer /healthz" >&2; exit 1; fi
    sleep 0.1
  done
  head -n 5 log
  echo "-- GET /healthz: $(cat healthz)"
  kill $pid; wait $pid 2>/dev/null || true
  grep -q '"version":"smoke"' healthz || { echo "btwserve: /healthz does not report -version smoke" >&2; exit 1; }
else
  echo "-- $(./btwd --version)"
  start=$(date +%s%N)
  ./btwd render <smoke.btw >out.raw 2>err || { cat err >&2; echo "btwd render failed" >&2; exit 1; }
  end=$(date +%s%N)
  tail -n 1 err
  bytes=$(wc -c <out.raw | tr -d ' ')
  echo "-- btwd render < smoke.btw: exit 0, $bytes bytes, wall $(( (end - start) / 1000000 )) ms"
  [ "$bytes" -gt 0 ] || { echo "btwd render wrote nothing" >&2; exit 1; }
fi
SMOKE
  if [ "$SH" = local ]; then rm -rf "$sdir"; else ssh -o BatchMode=yes "$SH" "rm -rf '$sdir'"; fi
  [ "$rc" = 0 ] || { echo "build-btwd-linux: the smoke failed on $SH" >&2; return 1; }
}

if [ "$METHOD" = zig ] && [ "$BH" = local ]; then
  # this Mac, in place
  tdir=${BTWD_TARGET_DIR:-$root/target-linux}
  mkdir -p "$tdir"
  log="$tdir/build-$PKG.log"
  echo "== build host $HOST (this machine): cargo zigbuild --release -p $PKG --target $ZIG_TARGET, target dir $tdir"
  t0=$(date +%s)
  rm -f "$tdir/x86_64-unknown-linux-gnu/release/$PKG"
  if ! ( cd "$root" && CARGO_TARGET_DIR="$tdir" CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_INCREMENTAL=0 CARGO_TERM_COLOR=never \
      cargo zigbuild --release -p "$PKG" $LOCKED --target "$ZIG_TARGET" ) >"$log" 2>&1; then
    tail -n 40 "$log" >&2
    crate=$(grep -o 'failed to run custom build command for `[^`]*`' "$log" | head -n 1 | sed 's/^.*for `//; s/`$//')
    [ -z "$crate" ] || echo "build-btwd-linux: $crate does not cross-compile with zig: build $PKG on a Linux host (BUILD_HOST=<linux host>)" >&2
    echo "build-btwd-linux: cargo zigbuild failed (log: $log)" >&2
    exit 1
  fi
  tail -n 1 "$log"
  echo "-- built in $(( $(date +%s) - t0 )) s"
  bin="$tdir/x86_64-unknown-linux-gnu/release/$PKG"
  test -x "$bin"
  glibc_check "$bin"
  cp "$bin" "$dist/$PKG-linux-x86_64"
  fetched
  [ "$SMOKE" = 0 ] || zig_smoke
  if [ "$CLEAN" = 1 ]; then echo "== clean $tdir"; rm -rf "$tdir"; fi
  echo "== done: $dist/$PKG-linux-x86_64"
  exit 0
fi

echo "== build host $HOST$( [ "$BH" = local ] && echo ' (this machine)') ($METHOD), scratch ~/$SCRATCH"
bh_sh "mkdir -p '$SCRATCH'"

echo "== rsync sources"
$RS "$root/" "$(bh_path "$SCRATCH/btw-web/")"

if [ "$METHOD" = zig ]; then
  # the Mac over ssh: this script, there, in place in the scratch copy
  echo "== build on $HOST (cargo zigbuild, BUILD_HOST=local there)"
  bh_sh "$BH_ENV cd '$SCRATCH/btw-web' && BUILD_HOST=local BTWD_PKG='$PKG' BTWD_UNLOCKED='${BTWD_UNLOCKED:-0}' BTWD_TARGET_DIR=\"\$HOME/$SCRATCH/target-linux\" sh scripts/build-btwd-linux.sh --no-smoke"
  echo "== fetch"
  scp -q "$(bh_path "$SCRATCH/btw-web/dist/$PKG-linux-x86_64")" "$dist/$PKG-linux-x86_64"
  glibc_check "$dist/$PKG-linux-x86_64"
  fetched
  [ "$SMOKE" = 0 ] || zig_smoke
  if [ "$CLEAN" = 1 ]; then echo "== clean the build host"; bh_sh "rm -rf '$SCRATCH' && echo cleaned"; fi
  echo "== done: $dist/$PKG-linux-x86_64"
  exit 0
fi

echo "== build (image if needed, then cargo build --release -p $PKG)"
bh_sh "HOST='$HOST' PKG='$PKG' SCRATCH='$SCRATCH' IMAGE='$IMAGE' UNLOCKED='${BTWD_UNLOCKED:-0}' sh -s" <<'REMOTE'
set -eu
cd "$HOME/$SCRATCH"
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "-- building $IMAGE (once)"
  docker build -t "$IMAGE" - <<'DOCKERFILE'
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl gcc libc6-dev \
 && rm -rf /var/lib/apt/lists/* \
 && curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
 && chmod -R a+rX /opt/rustup /opt/cargo
DOCKERFILE
fi
mkdir -p cargo-home target
LOCKED=--locked
[ "$UNLOCKED" = 1 ] && LOCKED=
# RUSTUP_TOOLCHAIN overrides the repo's rust-toolchain.toml (which asks for the wasm target too)
rm -f "target/release/$PKG"
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/cargo-home -e CARGO_TARGET_DIR=/work/target -e HOME=/work/cargo-home \
  -e RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu \
  -e CARGO_PROFILE_RELEASE_STRIP=symbols -e CARGO_INCREMENTAL=0 -e CARGO_TERM_COLOR=never \
  -v "$PWD:/work" -w /work/btw-web "$IMAGE" \
  cargo build --release -p "$PKG" $LOCKED >build.log 2>&1 || { tail -n 40 build.log >&2; echo "build-btwd-linux: cargo failed (log: $HOST:~/$SCRATCH/build.log)" >&2; exit 1; }
tail -n 3 build.log
test -x "target/release/$PKG"
REMOTE

echo "== fetch"
scp -q "$(bh_path "$SCRATCH/target/release/$PKG")" "$dist/$PKG-linux-x86_64"
fetched

echo "== the binary in a fresh ubuntu:24.04 (nothing installed)"
scp -q "$smoke" "$(bh_path "$SCRATCH/smoke.btw")"
bh_sh "PKG='$PKG' SCRATCH='$SCRATCH' IMAGE='$IMAGE' SMOKE='$SMOKE' sh -s" <<'REMOTE'
set -eu
cd "$HOME/$SCRATCH"
# the newest glibc symbol the binary names, and every shared library it wants
docker run --rm -v "$PWD/target/release:/b:ro" ubuntu:24.04 sh -c "echo '-- ldd'; ldd /b/$PKG"
need=$(docker run --rm -v "$PWD/target/release:/b:ro" "$IMAGE" sh -c "objdump -T /b/$PKG | grep -o 'GLIBC_[0-9.]*' | sort -u -V | tail -n 1")
echo "-- newest glibc symbol version: $need (the deploy host has 2.39)"
newest=$(printf '%s\nGLIBC_2.39\n' "$need" | sort -V | tail -n 1)
if [ "$newest" != GLIBC_2.39 ]; then echo "build-btwd-linux: the binary needs $need, newer than the deploy host's glibc 2.39" >&2; exit 1; fi
if [ "$SMOKE" = 1 ] && [ "$PKG" = btwserve ]; then
  echo "-- btwserve in ubuntu:24.04: start, render smoke.btw over HTTP (WAV and Opus), stop"
  docker run --rm -i -v "$PWD/target/release:/b:ro" -v "$PWD/btw-web/crates/btwserve/bench:/bench:ro" ubuntu:24.04 sh -c '
    mkdir -p /d/programs && cat > /tmp/p.btw
    /b/btwserve -addr 127.0.0.1:18098 -data /d -log-dir off >/tmp/log 2>&1 &
    sleep 1; cat /tmp/log
    echo "(no curl in a bare image: the HTTP smoke is the listen line above; renders are checked by parity.py)"
    kill $!' < smoke.btw
elif [ "$SMOKE" = 1 ]; then
  echo "-- render smoke.btw in ubuntu:24.04"
  docker run --rm -i -v "$PWD/target/release:/b:ro" ubuntu:24.04 sh -c '
    start=$(date +%s%N)
    /b/btwd render 2>/tmp/err | wc -c
    end=$(date +%s%N)
    tail -n 1 /tmp/err
    echo "wall $(( (end - start) / 1000000 )) ms"' < smoke.btw
fi
REMOTE

if [ "$CLEAN" = 1 ]; then
  echo "== clean the build host"
  bh_sh "rm -rf '$SCRATCH' && docker rmi '$IMAGE' >/dev/null 2>&1; docker builder prune -f >/dev/null 2>&1; echo cleaned"
fi
echo "== done: $dist/$PKG-linux-x86_64"
