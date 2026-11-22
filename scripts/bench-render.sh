#!/bin/sh
# bench-render.sh: the streaming renderer's headline numbers.
#
#   scripts/bench-render.sh [-n RUNS] [-b BTWD] [PROGRAM.btw ...]
#
# For each program (default: crates/btwapi/tests/corpus/*.btw and examples/*.btw) it starts a fresh
# `btwd render` process RUNS times (default 5) and reports
#   ttfb     milliseconds from the moment the process is spawned to its first byte on stdout
#   total    milliseconds to the last byte
#   x_rt     real-time factor over the whole song (audio seconds / render seconds)
#   rss_mb   peak resident set of the process (what the OS says, children of this script)
# as min / median / max over the runs, one line a program, and a last line with the worst ttfb.
# Needs python3 (stdlib only). BTWD defaults to target/release/btwd next to this script's repo.
# Exit 1 if any program's worst ttfb is 1000 ms or more (the contract: under 1 s, cold).
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
runs=5
bin=${BTWD:-"$here/target/release/btwd"}
while [ $# -gt 0 ]; do
  case $1 in
    -n) runs=$2; shift 2 ;;
    -b) bin=$2; shift 2 ;;
    *) break ;;
  esac
done
if [ $# -eq 0 ]; then
  set -- "$here"/crates/btwapi/tests/corpus/*.btw "$here"/examples/*.btw
fi
[ -x "$bin" ] || { echo "bench-render: $bin is not built (cargo build --release -p btwd)" >&2; exit 2; }
exec python3 - "$bin" "$runs" "$@" <<'PY'
import json, os, resource, statistics, subprocess, sys, time

bin_, runs, files = sys.argv[1], int(sys.argv[2]), sys.argv[3:]

def one(path):
    t0 = time.perf_counter()
    p = subprocess.Popen([bin_, "render", "--file", path], stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
    first = p.stdout.read(1)
    ttfb = (time.perf_counter() - t0) * 1000
    n = len(first)
    while True:
        b = p.stdout.read(1 << 16)
        if not b:
            break
        n += len(b)
    total = (time.perf_counter() - t0) * 1000
    err = p.stderr.read().decode()
    p.wait()
    start = end = None
    for line in err.splitlines():
        try:
            j = json.loads(line)
        except ValueError:
            continue
        if j.get("event") == "start": start = j
        if j.get("event") == "end": end = j
        if j.get("event") == "error": raise SystemExit("%s: %s" % (path, line))
    secs = start["seconds"]
    return ttfb, total, secs / (total / 1000), n, secs

def rss_mb(path):
    # peak RSS of one run, measured on its own so the figure is not a max over earlier runs
    code = "import resource,subprocess,sys;subprocess.run(sys.argv[1:],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,stdin=subprocess.DEVNULL);r=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss;print(r)"
    out = subprocess.run([sys.executable, "-c", code, bin_, "render", "--file", path], capture_output=True, text=True).stdout.strip()
    r = int(out)
    return r / (1024 * 1024) if sys.platform == "darwin" else r / 1024

def fmt(xs, d=0):
    return "%.*f/%.*f/%.*f" % (d, min(xs), d, statistics.median(xs), d, max(xs))

print("%-28s %5s %6s  %-18s %-18s %-14s %s" % ("program", "secs", "runs", "ttfb ms min/med/max", "total ms", "x_realtime", "rss_mb"))
worst = 0.0
for f in files:
    rs = [one(f) for _ in range(runs)]
    worst = max(worst, max(r[0] for r in rs))
    print("%-28s %5.1f %6d  %-18s %-18s %-14s %.0f" % (os.path.basename(f)[:28], rs[0][4], runs, fmt([r[0] for r in rs]), fmt([r[1] for r in rs]), fmt([r[2] for r in rs], 1), rss_mb(f)))
print("worst ttfb %.0f ms over %d programs x %d runs (%s)" % (worst, len(files), runs, "ok: under 1000 ms" if worst < 1000 else "TOO SLOW"))
sys.exit(0 if worst < 1000 else 1)
PY
