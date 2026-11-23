#!/usr/bin/env bash
# Generate site/web/docs/BEATSCRIPT.md, arrange.md and cheatsheet.md, the copies beatscript.app serves from -docs.
#
# They are made from this repo's docs/BEATSCRIPT.md, docs/ARRANGE.md and `beatscript cheatsheet` at deploy time, so they are in
# .gitignore: deploy/deploy.sh runs this before it copies them to the host. Never `git add -f` its output.
#
#   site/scripts/beatscript-gen-docs.sh                         copy docs/BEATSCRIPT.md; keep the cheatsheet.md that is there
#   BEATSCRIPT_BIN=/path/to/beatscript site/scripts/beatscript-gen-docs.sh  also write cheatsheet.md from `beatscript cheatsheet`
#   BEATSCRIPT_BIN=/path/to/beatscript site/scripts/beatscript-gen-docs.sh --check
#                                                   change nothing; exit 1 if cheatsheet.md or BEATSCRIPT.md is stale
#
# Environment:
#   DJRUST_DIR  the tree with docs/BEATSCRIPT.md                (default: this repo)
#   OUT_DIR     where the files go                      (default site/web/docs)
#   BEATSCRIPT_BIN    the beatscript binary; without it cheatsheet.md is not regenerated. If no cheatsheet.md exists at
#               all the server answers /docs/cheatsheet.md from its own engine instead, so nothing is broken
#               by running without it.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
src=${DJRUST_DIR:-$root}
out=${OUT_DIR:-$root/site/web/docs}
check=0
[ "${1:-}" = "--check" ] && check=1

md="$src/docs/BEATSCRIPT.md"
if [ ! -f "$md" ]; then
  echo "beatscript-gen-docs: $md not found (set DJRUST_DIR to the djrust worktree)" >&2
  exit 1
fi

stale=0
mkdir -p "$out"

# BEATSCRIPT.md: the copy this site serves publicly, with local paths and private repo names removed (redact below)
redact() {
  sed -E -e 's#~/Workspace/[^ `)"]*#<local path>#g' -e 's#/Users/[^ `)"]*#<local path>#g' -e 's#/home/[A-Za-z0-9_]+/[^ `)"]*#<local path>#g' "$1"
}
if [ "$check" = 1 ]; then
  redact "$md" | cmp -s - "$out/BEATSCRIPT.md" || { echo "beatscript-gen-docs: $out/BEATSCRIPT.md differs from $md" >&2; stale=1; }
else
  redact "$md" > "$out/BEATSCRIPT.md.tmp" && mv "$out/BEATSCRIPT.md.tmp" "$out/BEATSCRIPT.md"
  echo "beatscript-gen-docs: BEATSCRIPT.md copied (paths redacted) from $md ($(wc -c < "$out/BEATSCRIPT.md" | tr -d ' ') bytes)"
fi

# arrange.md: the arrange-mutations page (/docs/arrange), redacted as BEATSCRIPT.md is
amd="$src/docs/ARRANGE.md"
if [ -f "$amd" ]; then
  if [ "$check" = 1 ]; then
    redact "$amd" | cmp -s - "$out/arrange.md" || { echo "beatscript-gen-docs: $out/arrange.md differs from $amd" >&2; stale=1; }
  else
    redact "$amd" > "$out/arrange.md.tmp" && mv "$out/arrange.md.tmp" "$out/arrange.md"
    echo "beatscript-gen-docs: arrange.md copied (paths redacted) from $amd"
  fi
fi

# cheatsheet.md: the output of `beatscript cheatsheet`, the same text the compose system prompt uses
if [ -n "${BEATSCRIPT_BIN:-}" ]; then
  if [ ! -x "$BEATSCRIPT_BIN" ]; then
    echo "beatscript-gen-docs: BEATSCRIPT_BIN=$BEATSCRIPT_BIN is not an executable" >&2
    exit 1
  fi
  fresh=$(mktemp "${TMPDIR:-/tmp}/beatscript-cheatsheet.XXXXXX")
  trap 'rm -f "$fresh"' EXIT
  "$BEATSCRIPT_BIN" cheatsheet > "$fresh"
  if [ ! -s "$fresh" ]; then
    echo "beatscript-gen-docs: '$BEATSCRIPT_BIN cheatsheet' printed nothing" >&2
    exit 1
  fi
  if [ "$check" = 1 ]; then
    cmp -s "$fresh" "$out/cheatsheet.md" || { echo "beatscript-gen-docs: $out/cheatsheet.md differs from a fresh '$BEATSCRIPT_BIN cheatsheet'" >&2; stale=1; }
  else
    cp "$fresh" "$out/cheatsheet.md.tmp" && mv "$out/cheatsheet.md.tmp" "$out/cheatsheet.md"
    echo "beatscript-gen-docs: cheatsheet.md written by '$BEATSCRIPT_BIN cheatsheet' ($(wc -c < "$out/cheatsheet.md" | tr -d ' ') bytes)"
  fi
elif [ -f "$out/cheatsheet.md" ]; then
  echo "beatscript-gen-docs: BEATSCRIPT_BIN not set; kept the existing cheatsheet.md (set it to regenerate)"
else
  echo "beatscript-gen-docs: warning: BEATSCRIPT_BIN not set and there is no cheatsheet.md; the server will answer /docs/cheatsheet.md from beatscript itself" >&2
fi

exit "$stale"
