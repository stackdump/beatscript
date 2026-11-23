#!/usr/bin/env bash
# build-web-wasm.sh: build the browser module (crates/djweb: beatscript-api + BeatscriptRun + the djrust engine facade)
# into one output directory, ready to be served as site/web/wasm/.
#
#   scripts/build-web-wasm.sh <outdir>
#
# (rename-to-beatscript: keep: btw.js is the legacy shim's name)
# writes  beatscript.js  beatscript_bg.wasm  beatscript.d.ts  beatscript_bg.wasm.d.ts  btw.js  hashes.json
#         (btw.js: site/web/wasm-legacy/btw.js, the module under its names before the rename, for a tab left open across
#         the deploy; one release)
#
# - cargo, release, wasm32-unknown-unknown, into target-web/ (its own target dir: it does not fight a
#   native build for the lock; target-* is git-ignored)
# - wasm-bindgen CLI (--target web, --out-name beatscript). The CLI must be exactly the wasm-bindgen the lockfile
#   pins (the glue and the module must agree to the patch); the script checks it and says so if not.
# - the name and producers sections stripped (BEATSCRIPT_KEEP_NAMES=1 keeps them for a debugger)
# - wasm-opt (binaryen) when installed, else skipped with a note (the .wasm is then unoptimised).
# - build-machine paths are remapped out of panic locations (as djrust's npm/build.sh does), so the same
#   source and toolchain give the same module.
# - hashes.json: sha256 and size of each file, the engine and djweb versions, the git revision it was built from.
#
# - where: scripts/build-host.sh picks the build host. BUILD_HOST unset = this machine when it can, else the
#   hosts in BUILD_HOSTS in order (a host is passed over when ssh does not reach it or it lacks cargo or the pinned
#   wasm-bindgen; a remote one also needs GNU rsync, which on a Mac is Homebrew's: its own openrsync cannot talk to
#   GNU rsync); BUILD_HOST=<ssh host> pins one; BUILD_HOST=local builds here, in place. On another host this tree
#   is rsynced to ~/${BEATSCRIPT_WASM_BUILD_DIR:-beatscript-wasm-build} there, built with BUILD_HOST=local, and the output
#   rsynced back; its cargo target dir stays on that host, so a second build is incremental. The engine,
#   stackdump/djrust, is a git dependency pinned in Cargo.toml, fetched by cargo there. The output is the same
#   either way: paths are remapped and hashes.json carries this tree's git revision. On a Mac, wasm-bindgen is
#   cargo's (~/.cargo/bin) and wasm-opt is Homebrew's binaryen (/opt/homebrew/bin), both on BH_ENV's PATH over ssh.
#
# The output is a build artifact, not source: site/web/wasm/ is git-ignored and deploy/deploy.sh builds and copies
# it.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd -P)
out=${1:?usage: scripts/build-web-wasm.sh <outdir>}
mkdir -p "$out"
out=$(cd "$out" && pwd -P)

want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,"",$0); print; exit}' Cargo.lock)

if [ "${BUILD_HOST:-}" != local ]; then
  . scripts/build-host.sh
  pick_build_host cargo "wasm-bindgen=$want" || exit 2
fi
if [ "${BH:-local}" != local ]; then
  scratch=${BEATSCRIPT_WASM_BUILD_DIR:-beatscript-wasm-build}
  echo "== build-web-wasm on $BH_NAME, scratch ~/$scratch"
  bh_sh "mkdir -p '$scratch' && rm -rf '$scratch/out'"
  RS="$BH_RSYNC -a --delete --delete-excluded --exclude=target/ --exclude=/target-*/ --exclude=.git/ --exclude=/dist/ --exclude=*.wav --exclude=*.sf2"
  $RS "$root/" "$(bh_path "$scratch/beatscript-web/")"
  rev=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
  bh_sh "$BH_ENV cd '$scratch/beatscript-web' && BUILD_HOST=local BEATSCRIPT_GIT_REV='$rev' BEATSCRIPT_WASM_TARGET_DIR=\"\$HOME/$scratch/target-web\" BEATSCRIPT_KEEP_NAMES='${BEATSCRIPT_KEEP_NAMES:-}' scripts/build-web-wasm.sh \"\$HOME/$scratch/out\""
  $BH_RSYNC -a "$(bh_path "$scratch/out/")" "$out/"
  echo "build-web-wasm: built on $BH_NAME, copied to $out"
  ls -l "$out"
  exit 0
fi

# a pinned CLI side by side with another (cargo install wasm-bindgen-cli --version X --root ~/.local/opt/wasm-bindgen-X)
if [ -x "$HOME/.local/opt/wasm-bindgen-$want/bin/wasm-bindgen" ]; then PATH="$HOME/.local/opt/wasm-bindgen-$want/bin:$PATH"; fi
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ -z "$have" ]; then echo "build-web-wasm: wasm-bindgen CLI not found (cargo install wasm-bindgen-cli --version $want)" >&2; exit 2; fi
if [ "$want" != "$have" ]; then
  echo "build-web-wasm: the lockfile pins wasm-bindgen $want but the CLI is $have (cargo install wasm-bindgen-cli --version $want --locked, or cargo update -p wasm-bindgen --precise $have)" >&2
  exit 2
fi

sysroot=$(rustc --print sysroot)
flags="--remap-path-prefix=$root=djweb --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=cargo --remap-path-prefix=$sysroot=rust"
# the engine is a git dependency (Cargo.toml): its checkout under cargo's git dir becomes `djrust`, as it was when it
# sat beside this repo, so panic locations do not carry the checkout's hash (rustc applies the last matching remap)
djrust_dir=$(cargo metadata --locked --format-version 1 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(p["manifest_path"] for p in d["packages"] if p["name"]=="djrust").rsplit("/",1)[0])' || true)
if [ -n "$djrust_dir" ]; then flags="$flags --remap-path-prefix=$djrust_dir=djrust"; fi
export RUSTFLAGS="${RUSTFLAGS:-} $flags"
export CARGO_TARGET_DIR="${BEATSCRIPT_WASM_TARGET_DIR:-$root/target-web}"

# a default build only: no booth dictionary, no windwheel, no wallet (scripts/check-public-build.sh)
scripts/check-public-build.sh --tree
export BEATSCRIPT_PUBLIC_BUILD=1
cargo build --release --locked --target wasm32-unknown-unknown -p djweb
wasm="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/djweb.wasm"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# the name and producers sections are ~1 MB of debugging help the page never reads (BEATSCRIPT_KEEP_NAMES=1 keeps them)
strip_flags="--remove-name-section --remove-producers-section"
[ "${BEATSCRIPT_KEEP_NAMES:-}" = 1 ] && strip_flags=""
wasm-bindgen --target web --out-dir "$tmp" --out-name beatscript $strip_flags "$wasm"

# Homebrew's binaryen when /opt/homebrew/bin is not on PATH (a Mac shell that did not read brew's shellenv)
# (no brace group inside ${...:-$(...)}: macOS /bin/bash 3.2, the bash such a shell finds, cannot parse one)
wasm_opt=${WASM_OPT:-$(command -v wasm-opt || true)}
if [ -z "$wasm_opt" ] && [ -x /opt/homebrew/bin/wasm-opt ]; then wasm_opt=/opt/homebrew/bin/wasm-opt; fi
if [ -n "$wasm_opt" ]; then
  # the module's own features (its target_features section), not --all-features: binaryen 133 with
  # --all-features writes encodings (an import kind 0x7e) that Node 23 and the browsers refuse to compile
  "$wasm_opt" -O3 "$tmp/beatscript_bg.wasm" -o "$tmp/beatscript_bg.wasm"
  opt=yes
else
  echo "build-web-wasm: wasm-opt not found (install binaryen or set WASM_OPT): the .wasm is unoptimised" >&2
  opt=no
fi

for f in beatscript.js beatscript_bg.wasm beatscript.d.ts beatscript_bg.wasm.d.ts; do cp "$tmp/$f" "$out/$f"; done
# the legacy shim, hand-written (not wasm-bindgen output): wasm/btw.js re-exports beatscript.js under the old names
cp site/web/wasm-legacy/btw.js "$out/btw.js"

rev=${BEATSCRIPT_GIT_REV:-$(git rev-parse --short HEAD 2>/dev/null || echo unknown)}
python3 - "$out" "$rev" "$opt" "$have" <<'PY'
import hashlib, json, os, re, sys
out, rev, opt, cli = sys.argv[1:5]
files = {}
for name in ("beatscript.js", "beatscript_bg.wasm", "beatscript.d.ts", "beatscript_bg.wasm.d.ts", "btw.js"):
    data = open(os.path.join(out, name), "rb").read()
    files[name] = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
info = {}
# the versions: djweb's own (the workspace version), and the engine's (djrust, from Cargo.lock)
m = re.search(r'^version = "([^"]+)"', open("Cargo.toml").read(), re.M)
info["djweb"] = m.group(1) if m else None
m = re.search(r'^name = "djrust"\nversion = "([^"]+)"', open("Cargo.lock").read(), re.M)
info["engine"] = m.group(1) if m else None
doc = {"files": files, "versions": info, "git": rev, "wasm_bindgen": cli, "wasm_opt": opt == "yes"}
with open(os.path.join(out, "hashes.json"), "w") as f:
    json.dump(doc, f, indent=2, sort_keys=True)
    f.write("\n")
w = files["beatscript_bg.wasm"]["size"]
print("wrote %s: beatscript_bg.wasm %d bytes (%.2f MB)" % (out, w, w / 1048576))
PY
gz=$(gzip -9 -c "$out/beatscript_bg.wasm" | wc -c | tr -d ' ')
echo "build-web-wasm: beatscript_bg.wasm gzip -9: $gz bytes ($(awk -v n="$gz" 'BEGIN{printf "%.2f", n/1048576}') MB)"
ls -l "$out"
