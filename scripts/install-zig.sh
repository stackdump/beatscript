#!/bin/sh
# install-zig.sh: put zig and cargo-zigbuild on this machine, for scripts/build-linux.sh (no docker, no root, no pip).
#
#   sh scripts/install-zig.sh                 here
#   ssh <host> sh -s < scripts/install-zig.sh   on a build host
#
# zig is the official tarball from ziglang.org, unpacked under ~/.local/zig and linked from ~/.local/bin/zig. Its sha256
# is read from ziglang.org's own download index and checked before unpacking. cargo-zigbuild is `cargo install`ed
# (~/.cargo/bin) and told where zig is through CARGO_ZIGBUILD_ZIG_PATH (printed at the end; zig on PATH also works).
# Also adds rustup's x86_64-unknown-linux-gnu target, which build-host.sh checks for. Safe to rerun.
set -eu
ZIG_VERSION=${ZIG_VERSION:-0.17.1}
. "$HOME/.cargo/env" 2>/dev/null || true
PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"; export PATH

arch=$(uname -m)
name=zig-$arch-linux-$ZIG_VERSION
dir=$HOME/.local/zig/$name
if [ ! -x "$dir/zig" ]; then
  for t in curl python3 tar xz sha256sum; do
    command -v "$t" >/dev/null 2>&1 || { echo "install-zig: need $t" >&2; exit 1; }
  done
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  curl -fsSL https://ziglang.org/download/index.json -o "$tmp/index.json"
  # prints "<tarball url> <sha256>" for this version and arch
  set -- $(python3 -c 'import json,sys
e=json.load(open(sys.argv[1]))[sys.argv[2]][sys.argv[3]+"-linux"]
print(e["tarball"],e["shasum"])' "$tmp/index.json" "$ZIG_VERSION" "$arch")
  url=$1; want=$2
  curl -fsSL "$url" -o "$tmp/zig.tar.xz"
  got=$(sha256sum "$tmp/zig.tar.xz" | cut -d' ' -f1)
  [ "$got" = "$want" ] || { echo "install-zig: sha256 mismatch for $url: got $got want $want" >&2; exit 1; }
  mkdir -p "$HOME/.local/zig"
  tar -xJf "$tmp/zig.tar.xz" -C "$tmp"
  rm -rf "$dir.part"; mv "$tmp/$name" "$dir.part"; rm -rf "$dir"; mv "$dir.part" "$dir"
fi
mkdir -p "$HOME/.local/bin"
ln -sf "$dir/zig" "$HOME/.local/bin/zig"
CARGO_ZIGBUILD_ZIG_PATH=$dir/zig; export CARGO_ZIGBUILD_ZIG_PATH
echo "zig $("$CARGO_ZIGBUILD_ZIG_PATH" version)"
command -v cargo-zigbuild >/dev/null 2>&1 || cargo install cargo-zigbuild --locked
rustup target add --toolchain stable x86_64-unknown-linux-gnu
# cargo-zigbuild has no --version: check the binary exists and the subcommand answers --help
command -v cargo-zigbuild >/dev/null 2>&1 && cargo zigbuild --help >/dev/null
echo "cargo-zigbuild ok ($(command -v cargo-zigbuild)); export CARGO_ZIGBUILD_ZIG_PATH=$CARGO_ZIGBUILD_ZIG_PATH"
