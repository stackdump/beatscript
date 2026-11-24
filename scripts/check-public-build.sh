#!/usr/bin/env bash
# check-public-build.sh: what may be in a default build of this repo. Code is MIT, BeatScript programs and content
# are CC0 (LICENSE, REUSE.toml); nothing private, nothing under a licence that bars redistribution.
#
#   scripts/check-public-build.sh            all checks
#   scripts/check-public-build.sh --tree     only the dependency check (fast)
#
# 1. The dependency check: a default build of every shipped target (the wasm, the server, the CLIs, the language
#    crates) has none of
#      - djlexicon-booth   the booth's private pronouncing dictionary (CMUdict); it is not in this repo at all
#      - windwheel         the Wheel (public, MIT): behind djhost's off-by-default `windwheel` feature, and in the browser
#                          module only (djweb src/wheel.rs: the page plays a program's `wheel` lines); no other target
#      - djrust-wallet     the booth identity, behind djhost's off-by-default `wallet` feature
#    in its tree. djhost itself may be there (beatscript-serve reads its ontology vocabulary, beatscript its render path).
# 2. The workspace has exactly the allowlisted members: a new crate is a decision, not an accident.
# 3. No pronouncing-dictionary data: no file named like CMUdict, and no file of dictionary-shaped ARPAbet lines.
#    The lexicon (crates/djengine/data/lexicon.txt) is written by hand and CC0.
# 4. No Apple loop audio (.caf/.aif/.aiff): programs reference loops by hash only.
set -euo pipefail
cd "$(dirname "$0")/.."
targets=(djweb beatscript-serve beatscript-api beatscriptls djlive djengine beatscript-seal beatscript beatscript-round)
members=(beatscript-api beatscript-cli beatscriptls beatscript-round beatscript-seal beatscript-serve djengine djhost djlive djweb)
banned='^(djlexicon-booth|windwheel|djrust-wallet) '
fail=0

# the browser module plays the Wheel (djweb src/wheel.rs): windwheel is allowed there and nowhere else
banned_in() { if [ "$1" = djweb ]; then printf '%s' '^(djlexicon-booth|djrust-wallet) '; else printf '%s' "$banned"; fi; }

for p in "${targets[@]}"; do
  if hits=$(cargo tree -q -p "$p" -e normal,build --prefix none 2>/dev/null | grep -E "$(banned_in "$p")" | sort -u) && [ -n "$hits" ]; then
    echo "check-public-build: a default build of $p has $(printf '%s' "$hits" | awk '{print $1}' | tr '\n' ' ')in its tree" >&2
    for b in $(printf '%s\n' "$hits" | awk '{print $1}'); do cargo tree -q -p "$p" -e normal,build -i "$b" >&2 || true; done
    fail=1
  fi
done
if [ "${1:-}" = "--tree" ]; then
  [ $fail = 0 ] && echo "check-public-build: ok (no default build has djlexicon-booth, windwheel (but djweb's) or djrust-wallet)"
  exit $fail
fi

have=$(cargo metadata -q --no-deps --format-version 1 | python3 -c 'import json,sys; print(" ".join(sorted(p["name"] for p in json.load(sys.stdin)["packages"])))')
want=$(printf '%s\n' "${members[@]}" | sort | tr '\n' ' ' | sed 's/ $//')
if [ "$have" != "$want" ]; then
  echo "check-public-build: workspace members are [$have], the allowlist is [$want] (scripts/check-public-build.sh)" >&2
  fail=1
fi

list() { if git rev-parse --git-dir >/dev/null 2>&1; then git ls-files -co --exclude-standard; else find . -type f -not -path './target*' | sed 's|^\./||'; fi; }
if n=$(list | grep -iE 'cmudict|cmu[-_]?dict|\.dict(\.z)?$') && [ -n "$n" ]; then
  echo "check-public-build: pronouncing-dictionary files: $n" >&2; fail=1
fi
# ARPAbet with stress digits, `WORD  W ER1 D`: a dictionary has thousands of such lines; allow a handful (docs)
arpa="^[A-Z][A-Z'.-]*(\([0-9]\))?  ?([A-Z]{1,2}[012]? )+[A-Z]{1,2}[012]?$"
if d=$(list | grep -vE '\.(png|ttf|wasm|gz|br)$' | xargs -d '\n' grep -cIE "$arpa" 2>/dev/null | awk -F: '$2 > 50') && [ -n "$d" ]; then
  echo "check-public-build: dictionary-shaped ARPAbet lines in: $d" >&2; fail=1
fi
if a=$(list | grep -iE '\.(caf|aif|aiff)$') && [ -n "$a" ]; then
  echo "check-public-build: Apple loop audio in the tree: $a" >&2; fail=1
fi

[ $fail = 0 ] && echo "check-public-build: ok (no default build has djlexicon-booth, windwheel (but djweb's) or djrust-wallet; members as allowlisted; no dictionary data; no loop audio)"
exit $fail
