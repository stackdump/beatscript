#!/usr/bin/env bash
# sync.sh: the BeatScript ontology's upstream input and the release's checks.
#
#   sync.sh check           every release file against release.lock and manifest.json, and the upstream input against
#                           upstream.lock (offline; `cargo test` checks the same and more: ontology::tests::fork_integrity)
#   sync.sh upstream REF    vendor the bitwrap release at REF of $ONTOLOGY (default ../ontology-bitwrap-io beside this
#                           checkout) into upstream/bitwrap-<version>/ (its ns.nt and manifest.json) and write
#                           upstream.lock. Only for deliberately merging a later bitwrap release into the fork: the
#                           fork is built from upstream/bitwrap-0.2.0 (src/ontology/fork.rs names it), so a new version
#                           also means changing fork.rs, rebuilding (build.sh) and saying why in the commit.
#
# The release itself (release/, release.lock) is built by build.sh, never copied from bitwrap: the BeatScript ontology
# is a fork of bitwrap 0.2.0 that evolves on its own, and bitwrap is only ever an input, never fetched at serve time.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
ONTOLOGY=${ONTOLOGY:-$(cd "$repo/.." && pwd)/ontology-bitwrap-io}
[ -d "$ONTOLOGY/.git" ] || ONTOLOGY=$HOME/Workspace/ontology-bitwrap-io

sums() { # LOCK DIR: every "<sha256>  <name>" line of LOCK against DIR/<name>
  local bad=0
  while read -r sum name; do
    case "$sum" in ''|\#*|source|inputs) continue ;; esac
    got=$(LC_ALL=C shasum -a 256 "$2/$name" | awk '{print $1}')
    [ "$got" = "$sum" ] || { echo "MISMATCH $2/$name: lock $sum, file $got"; bad=1; }
  done < "$1"
  return $bad
}

check() {
  local bad=0
  sums "$here/release.lock" "$here/release" || bad=1
  sums "$here/upstream.lock" "$here/upstream/bitwrap-0.2.0" || bad=1
  (cd "$here/release" && python3 -I - <<'PY') || bad=1
import hashlib, json, sys
m = json.load(open("manifest.json"))
bad = 0
for name, want in m["files"].items():
    got = hashlib.sha256(open(name, "rb").read()).hexdigest()
    if got != want:
        print(f"MISMATCH {name}: manifest {want}, file {got}"); bad = 1
sys.exit(bad)
PY
  [ $bad = 0 ] && (cd "$here/release" && python3 -I -c 'import json;m=json.load(open("manifest.json"));d=m["derived_from"];print("ok:", m["name"], m["version"], m["pack_content_sha256"][:16], "derived from", d["name"], d["version"])')
  return $bad
}

case "${1:-}" in
check) check ;;
upstream)
  ref=${2:?usage: sync.sh upstream REF}
  sha=$(git -C "$ONTOLOGY" rev-parse --verify "$ref^{commit}")
  ver=$(git -C "$ONTOLOGY" show "$sha:release/manifest.json" | python3 -I -c 'import json,sys;print(json.load(sys.stdin)["version"])')
  dir="$here/upstream/bitwrap-$ver"
  mkdir -p "$dir"
  for f in ns.nt manifest.json; do git -C "$ONTOLOGY" show "$sha:release/$f" > "$dir/$f"; done
  {
    echo "# The bitwrap ontology release the BeatScript ontology derives from, vendored by sync.sh upstream: sha256 per file."
    echo "source ontology-bitwrap-io $sha"
    (cd "$dir" && LC_ALL=C shasum -a 256 ns.nt manifest.json)
  } > "$here/upstream.lock"
  echo "vendored bitwrap $ver ($sha) into ${dir#"$repo/"}; the fork builds from upstream/bitwrap-0.2.0 (src/ontology/fork.rs)"
  ;;
*) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
