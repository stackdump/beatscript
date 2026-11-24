#!/bin/sh
# install-zig.sh: put zig and cargo-zigbuild on this machine, for scripts/build-linux.sh (no docker, no root, no pip).
#
#   scripts/install-zig.sh                 here
#   ssh <host> sh -s < scripts/install-zig.sh   on a build host
#
# zig is the official x86_64-linux release tarball from ziglang.org, unpacked into ~/.local/zig/zig-$ZIG_VERSION and
# linked as ~/.local/bin/zig. The tarball's sha256 is checked against ziglang.org's download/index.json (fetched over
# https) before anything is unpacked. cargo-zigbuild is `cargo install`ed (~/.cargo/bin). Idempotent. Also rustup's
# x86_64-unknown-linux-gnu target for the stable toolchain, which build-host.sh checks for.
#
# ~/.local/bin is put on PATH by build-host.sh / build-linux.sh themselves (non-interactive ssh skips ~/.bashrc).
set -eu
ZIG_VERSION=${ZIG_VERSION:-0.17.1}
. "$HOME/.cargo/env" 2>/dev/null || true
PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"; export PATH
root=$HOME/.local/zig
dir=$root/zig-x86_64-linux-$ZIG_VERSION
if [ ! -x "$dir/zig" ]; then
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  curl -fsSL https://ziglang.org/download/index.json -o "$tmp/index.json"
  # the index is the authority for the tarball url and its sha256
  set -- $(python3 -I - "$tmp/index.json" "$ZIG_VERSION" <<'PY'
import json, sys
e = json.load(open(sys.argv[1]))[sys.argv[2]]["x86_64-linux"]
print(e["tarball"], e["shasum"])
PY
  )
  url=$1; want=$2
  curl -fsSL "$url" -o "$tmp/zig.tar.xz"
  got=$(sha256sum "$tmp/zig.tar.xz" | awk '{print $1}')
  [ "$got" = "$want" ] || { echo "install-zig: sha256 mismatch for $url: got $got, index.json says $want" >&2; exit 1; }
  mkdir -p "$root"
  tar -xJf "$tmp/zig.tar.xz" -C "$tmp"
  rm -rf "$dir"; mv "$tmp/zig-x86_64-linux-$ZIG_VERSION" "$dir"
fi
mkdir -p "$HOME/.local/bin"
ln -sfn "$dir/zig" "$HOME/.local/bin/zig"
# ~/.cargo/bin is on a non-interactive ssh PATH (rustup sets it up), ~/.local/bin often is not: link zig there too
[ ! -d "$HOME/.cargo/bin" ] || ln -sfn "$dir/zig" "$HOME/.cargo/bin/zig"
echo "zig $(zig version)"
command -v cargo-zigbuild >/dev/null 2>&1 || cargo install cargo-zigbuild --locked
rustup target add --toolchain stable x86_64-unknown-linux-gnu
echo "cargo-zigbuild $(cargo-zigbuild zigbuild --help >/dev/null 2>&1 && echo ok) $(cargo zigbuild --help 2>&1 | head -n 1)"
