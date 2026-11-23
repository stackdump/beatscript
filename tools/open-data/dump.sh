#!/usr/bin/env bash
# dump.sh: make the open-data archive of every public CC0 BeatScript program (docs/OPEN-DATA.md).
#
#   tools/open-data/dump.sh            on the host that serves beatscript.app, over ssh
#   tools/open-data/dump.sh --local    here, against a local programs store (BEATSCRIPT_DATA)
#
# beatscript-dump reads the programs store, never writes it, and writes <data>/dumps/beatscript-cc0-<hash>.tar.gz once
# plus latest.json; with nothing new it changes nothing. The server serves <data>/dumps at /dumps/.
#
# Environment:
#   BEATSCRIPT_HOST     the serving host                       (default pflow.dev)
#   BEATSCRIPT_DIR      the site's checkout there              (default ~/Workspace/btw-bitwrap-io)
#   BEATSCRIPT_DATA     its data dir                           (default $BEATSCRIPT_DIR/data)
#   BEATSCRIPT_DUMP     the beatscript-dump binary there              (default $BEATSCRIPT_DIR/btwsite/bin/beatscript-dump; deploy ships it)
#   BEATSCRIPT_PUBLIC   the public origin                      (default https://beatscript.app)
set -euo pipefail
host=${BEATSCRIPT_HOST:-pflow.dev}
dir=${BEATSCRIPT_DIR:-'~/Workspace/btw-bitwrap-io'}
data=${BEATSCRIPT_DATA:-"$dir/data"}
bin=${BEATSCRIPT_DUMP:-"$dir/btwsite/bin/beatscript-dump"}
public=${BEATSCRIPT_PUBLIC:-https://beatscript.app}
cmd="$bin -programs $data/programs -out $data/dumps -public-url $public"
if [ "${1:-}" = "--local" ]; then
  eval "$cmd"
else
  # shellcheck disable=SC2029 # the paths are expanded on the host, as meant
  ssh -o BatchMode=yes "$host" "$cmd"
fi
