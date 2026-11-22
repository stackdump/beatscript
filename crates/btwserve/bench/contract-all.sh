#!/bin/bash
# Every contract group, each on a fresh Go + btwserve pair (pair.sh: btwserve alone, no -fallback), and one summary.
#
#   bench/contract-all.sh [base-port] [groups]     # default 28100, every group; RENDER_LIMIT=8 (all examples)
#
# Reports land in $OUT (default ~/.cache/btwpair/report-<base>/<group>.json) and the summary on stdout.
# Exit 1 when any case of any group failed.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
base=${1:-28100}
groups=${2:-static,docs,examples,misc,fallback,p2,p3,cards,auth,compose,mcp,hub,render}
REF=${REF:-$HOME/Workspace/.val-worktrees/beats-bitwrap-io/btwserve-ref}
OUT=${OUT:-$HOME/.cache/btwpair/report-$base}
mkdir -p "$OUT"
rc=0
for g in ${groups//,/ }; do
  "$here/pair.sh" start "$base" >/dev/null || { echo "$g: the pair did not start" >&2; rc=1; continue; }
  python3 "$here/contract.py" --go "127.0.0.1:$base" --rs "127.0.0.1:$((base + 1))" --web "$REF/btwsite/web" --examples "$REF/btwsite/examples" \
    --groups "$g" --render-limit "${RENDER_LIMIT:-8}" --out "$OUT/$g.json" > "$OUT/$g.log" 2>&1 || rc=1
  grep -E '^(DIFF|FAIL)' "$OUT/$g.log" | cut -c1-240 | head -5
done
"$here/pair.sh" stop "$base"
python3 - "$OUT" ${groups//,/ } <<'EOF'
import json, os, sys
out, groups = sys.argv[1], sys.argv[2:]
tot = {"cases": 0, "identical": 0, "known": 0, "failed": 0}
print(f"{'group':10} {'cases':>6} {'identical':>9} {'accepted':>8} {'failed':>6}")
for g in groups:
    try:
        s = json.load(open(os.path.join(out, g + ".json")))["groups"][g]
    except Exception as e:
        print(f"{g:10} no report ({e})"); continue
    print(f"{g:10} {s['cases']:6} {s['identical']:9} {s['known']:8} {s['failed']:6}")
    for k in tot: tot[k] += s[k]
print(f"{'total':10} {tot['cases']:6} {tot['identical']:9} {tot['known']:8} {tot['failed']:6}")
EOF
exit $rc
