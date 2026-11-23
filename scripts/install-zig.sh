#!/bin/sh
# install-zig.sh: put zig and cargo-zigbuild on this machine, for scripts/build-linux.sh (no docker, no root).
#
#   scripts/install-zig.sh                 here
#   ssh <host> sh -s < scripts/install-zig.sh   on a build host
#
# zig comes from the `ziglang` wheel (the official release tarball repackaged), into ~/.local/opt/zig-$ZIG_VERSION, with
# ~/.local/bin/zig linked to it; cargo-zigbuild is `cargo install`ed (~/.cargo/bin). Idempotent. Also rustup's
# x86_64-unknown-linux-gnu target for the stable toolchain, which build-host.sh checks for.
set -eu
ZIG_VERSION=${ZIG_VERSION:-0.17.1}
. "$HOME/.cargo/env" 2>/dev/null || true
PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"; export PATH
dir=$HOME/.local/opt/zig-$ZIG_VERSION
if [ ! -x "$dir/ziglang/zig" ]; then
  pip3 install --quiet --no-deps --target "$dir" "ziglang==$ZIG_VERSION"
fi
mkdir -p "$HOME/.local/bin"
ln -sf "$dir/ziglang/zig" "$HOME/.local/bin/zig"
echo "zig $(zig version)"
command -v cargo-zigbuild >/dev/null 2>&1 || cargo install cargo-zigbuild --locked
rustup target add --toolchain stable x86_64-unknown-linux-gnu
echo "cargo-zigbuild $(cargo zigbuild --version 2>&1 | head -n 1)"
