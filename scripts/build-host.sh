# build-host.sh: which machine runs a heavy build. Sourced (POSIX sh) by build-linux.sh and
# build-web-wasm.sh; not run on its own.
#
#   BUILD_HOST unset    this machine, in place (`local`), when it can do the build; else each host named in
#                       BUILD_HOSTS (a space-separated list of ssh hosts, tried in order), saying why it passed
#                       over the others. Nothing is named here: the deploy that runs this sets the hosts.
#   BUILD_HOST=<host>   that ssh host only, no fallback: the build fails if it cannot.
#   BUILD_HOST=local    this machine, in place.
#   BUILD_HOST_ALIASES  optional, one machine a line: the names and addresses one machine goes by, so a host that
#                       is this machine is used in place, without ssh to itself.
# A host that is this machine (its short hostname, an alias on its BUILD_HOST_ALIASES line, or one of its addresses)
# is used in place.
#
#   pick_build_host NEED...  NEED: what the build host must have. A command name; `docker` (a daemon this user may
#                            use, not just the CLI); `wasm-bindgen=X.Y.Z` (that exact CLI, on PATH or side by side in
#                            ~/.local/opt/wasm-bindgen-X.Y.Z/bin); `rust-target=T` (rustup's stable toolchain has
#                            target T). `darwin:NEED` and `linux:NEED` apply only on that OS. A host reached over ssh
#                            must also have GNU rsync (below). The probe runs under BH_ENV, since a non-interactive
#                            ssh shell reads neither ~/.profile nor Homebrew's shellenv.
#                            Sets BH (the ssh host, or `local`), BH_NAME (for messages), BH_OS (`uname -s` there) and
#                            BH_RSYNC (the rsync command that reaches it).
#   bh_sh CMD                runs CMD on the build host, from its $HOME (stdin passes through).
#   bh_path PATH             PATH, relative to the build host's $HOME, as rsync and scp want it.
#
# rsync and the Mac: macOS's own /usr/bin/rsync is openrsync, which GNU rsync 3.x cannot talk to (the transfer
# dies with a protocol error), so a Mac build host needs Homebrew's GNU rsync (`brew install rsync`), and BH_RSYNC
# names it with --rsync-path because an ssh session's PATH there does not have /opt/homebrew/bin.

BUILD_HOSTS=${BUILD_HOSTS:-}

# the names one machine goes by, one machine a line (BUILD_HOST_ALIASES; empty: only the hostname and addresses)
BH_ALIASES=${BUILD_HOST_ALIASES:-}

# the shell prelude every build-host command starts with: the rust toolchain, ~/.local/bin and Homebrew on PATH
BH_ENV='. "$HOME/.cargo/env" 2>/dev/null; PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:$PATH"; export PATH;'

bh_names() {
  printf '%s' "$1"
}

bh_is_self() {
  _h=${1#*@}
  [ "$_h" = local ] && return 0
  _me=$(hostname 2>/dev/null | cut -d. -f1)
  _grp=$(printf '%s\n' "$BH_ALIASES" | awk -v h="$_h" '{ for (i = 1; i <= NF; i++) if ($i == h) { print; exit } }')
  [ -n "$_grp" ] || _grp=$_h
  _addrs=
  for _n in $_grp; do
    case $_n in
      *[!0-9.]*) [ "$(printf '%s' "$_n" | cut -d. -f1)" = "$_me" ] && return 0 ;;
      *)
        # an address: is it one of ours?
        if [ -z "$_addrs" ]; then
          _addrs=$( { ip -o addr show 2>/dev/null | awk '{print $4}' | cut -d/ -f1; ifconfig 2>/dev/null | awk '/inet /{print $2}'; } || true)
          [ -n "$_addrs" ] || _addrs=none
        fi
        printf '%s\n' "$_addrs" | grep -qxF "$_n" && return 0 ;;
    esac
  done
  return 1
}

# the check, run on the candidate: prints the first missing thing and exits 3, or prints `ok OS RSYNC`
bh_probe_cmd() {
  _p="$BH_ENV _os=\$(uname -s);"
  for _n in "$@"; do
    _only=
    case $_n in
      darwin:*) _only=Darwin; _n=${_n#darwin:} ;;
      linux:*) _only=Linux; _n=${_n#linux:} ;;
    esac
    case $_n in
      docker) _c="docker info >/dev/null 2>&1 || { echo 'no usable docker (installed, daemon up, user in the docker group?)'; exit 3; }" ;;
      wasm-bindgen=*)
        _v=${_n#wasm-bindgen=}
        _c="{ [ -x \"\$HOME/.local/opt/wasm-bindgen-$_v/bin/wasm-bindgen\" ] || [ \"\$(wasm-bindgen --version 2>/dev/null | awk '{print \$2}')\" = $_v ]; } || { echo 'no wasm-bindgen $_v'; exit 3; }" ;;
      rust-target=*)
        _t=${_n#rust-target=}
        _c="rustup target list --installed --toolchain stable 2>/dev/null | grep -qx '$_t' || { echo 'no rust target $_t (rustup target add $_t)'; exit 3; }" ;;
      gnu-rsync) _c="rsync --version 2>/dev/null | head -n 1 | grep -q '^rsync ' || { echo 'no GNU rsync on PATH (a Mac has openrsync, which GNU rsync cannot talk to: brew install rsync)'; exit 3; }" ;;
      *) _c="command -v $_n >/dev/null 2>&1 || { echo 'no $_n'; exit 3; }" ;;
    esac
    if [ -n "$_only" ]; then _p="$_p if [ \"\$_os\" = $_only ]; then $_c; fi;"; else _p="$_p $_c;"; fi
  done
  printf '%s echo "ok $_os $(command -v rsync)"' "$_p"
}

# bh_try HOST NEED...: 0 and sets BH if HOST can build; else 1 with the reason in BH_WHY and the probe's exit code
# in BH_RC (255: ssh did not reach it)
bh_try() {
  _cand=$1; shift
  if bh_is_self "$_cand"; then
    BH_RC=0
    BH_WHY=$(cd "$HOME" && sh -c "$(bh_probe_cmd "$@")" 2>&1) || BH_RC=$?
    BH_WHY=$(printf '%s\n' "$BH_WHY" | tail -n 1)
    [ "$BH_RC" = 0 ] || return 1
    BH=local
    BH_RSYNC=rsync
  else
    BH_RC=0
    BH_WHY=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$_cand" "$(bh_probe_cmd "$@" gnu-rsync)" 2>&1) || BH_RC=$?
    [ -n "$BH_WHY" ] || BH_WHY="ssh to $_cand failed"
    BH_WHY=$(printf '%s\n' "$BH_WHY" | tail -n 1)
    [ "$BH_RC" = 0 ] || return 1
    BH=$_cand
    BH_RSYNC="rsync --rsync-path=$(printf '%s' "$BH_WHY" | awk '{print $3}')"
  fi
  BH_OS=$(printf '%s' "$BH_WHY" | awk '{print $2}')
  BH_NAME=$_cand
  return 0
}

# bh_try_host LABEL NEED...: each of LABEL's names in turn, until one answers
bh_try_host() {
  _label=$1; shift
  BH_WHY="no host names for $_label"
  for _cand in $(bh_names "$_label"); do
    bh_try "$_cand" "$@" && return 0
    # it answered and cannot build: its other names are the same machine
    [ "$BH_RC" = 255 ] || return 1
  done
  return 1
}

pick_build_host() {
  if [ -n "${BUILD_HOST:-}" ]; then
    bh_try_host "$BUILD_HOST" "$@" || { echo "build host $BUILD_HOST cannot build: $BH_WHY" >&2; return 2; }
    return 0
  fi
  for _label in local $BUILD_HOSTS; do
    bh_try_host "$_label" "$@" && return 0
    echo "build host: $_label cannot build: $BH_WHY" >&2
  done
  echo "build host: none of: local $BUILD_HOSTS can build (BUILD_HOST=<ssh host>, or BUILD_HOSTS=\"a b\")" >&2
  return 2
}

bh_sh() {
  if [ "$BH" = local ]; then (cd "$HOME" && sh -c "$1"); else ssh -o BatchMode=yes "$BH" "$1"; fi
}

bh_path() {
  if [ "$BH" = local ]; then printf '%s/%s' "$HOME" "$1"; else printf '%s:%s' "$BH" "$1"; fi
}
