#!/usr/bin/env bash
# dump.sh: make the open-data archive of every public CC0 BeatScript program (docs/OPEN-DATA.md).
#
#   tools/open-data/dump.sh            on the host that serves beatscript.app, over ssh
#   tools/open-data/dump.sh --local    here, against a local programs store (BTW_DATA)
#
# btw-dump reads the programs store, never writes it, and writes <data>/dumps/beatscript-cc0-<hash>.tar.gz once
# plus latest.json; with nothing new it changes nothing. The server serves <data>/dumps at /dumps/.
#
# Environment:
#   BTW_HOST     the serving host                       (default pflow.dev)
#   BTW_DIR      the site's checkout there              (default ~/Workspace/btw-bitwrap-io)
#   BTW_DATA     its data dir                           (default $BTW_DIR/data)
#   BTW_DUMP     the btw-dump binary there              (default $BTW_DIR/btwsite/bin/btw-dump; deploy ships it)
#   BTW_PUBLIC   the public origin                      (default https://beatscript.app)
set -euo pipefail
host=${BTW_HOST:-pflow.dev}
dir=${BTW_DIR:-'~/Workspace/btw-bitwrap-io'}
data=${BTW_DATA:-"$dir/data"}
bin=${BTW_DUMP:-"$dir/btwsite/bin/btw-dump"}
public=${BTW_PUBLIC:-https://beatscript.app}
cmd="$bin -programs $data/programs -out $data/dumps -public-url $public"
if [ "${1:-}" = "--local" ]; then
  eval "$cmd"
else
  # shellcheck disable=SC2029 # the paths are expanded on the host, as meant
  ssh -o BatchMode=yes "$host" "$cmd"
fi
