#!/usr/bin/env python3
"""Listening tests for BeatScript: `beatscript listen` summaries of every example and fixture, held to goldens.

    listen_ci.py [--beatscript PATH] [--examples DIR] [--only NAME]... [--update] [--json] [--publish]

For each program (the site's examples, from `--examples`, plus tools/listen/fixtures/*.beatscript) it runs
`beatscript listen --file <program>` and compares the summary with tools/listen/goldens/<name>.json:

- a deck or part that was heard and is now silent, or the other way round, or a changed `silent` entry
  (deck, part, section, reason): FAIL
- loudness (LUFS) of the master, a deck, a part or a section moved more than LUFS_TOL, or went to or from null: FAIL
- the tempo heard (bpm_match, or the bpm by more than BPM_TOL) or the key heard (key, key_match) changed: FAIL
- the master clips now and did not: FAIL
- the program's CID or the summary's version is not the golden's: STALE (a changed example; run --update)

A fixture's `# expect: silent deck=B reason=crossfader` and `# expect: heard deck=A` lines are checked on every run,
golden or not, so a regenerated golden cannot hide the regression it exists for.

--update writes the goldens from this run (after the expect lines pass). --publish posts one ListenRun event on the
event bus and writes data/listen/last.json. Exit 0 when every program passes, 1 on any FAIL or STALE, 2 on usage or
a beatscript that would not run. Stdlib only.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import socket
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDENS = os.path.join(HERE, "goldens")
FIXTURES = os.path.join(HERE, "fixtures")
LUFS_TOL = 1.0
BPM_TOL = 0.01
TIMEOUT_S = 300
BUS_SOCK = os.environ.get("LISTEN_BUS_SOCK", "data/metrics/publish.sock")
LAST = os.environ.get("LISTEN_LAST", "data/listen/last.json")
# what a golden keeps (the rest is timing or curve detail that is not compared). Since listen/2 it also records the mix
# (clashes, targets, warning_ids, and the bands and true peak inside master, parts and sections): kept so a change shows in
# the golden's diff, and not a reason to fail (the gate below is unchanged).
KEEP = ("version", "cid", "window", "master", "detected", "decks", "parts", "silent", "sections", "clashes", "targets", "warnings", "warning_ids")


def listen(beatscript: str, path: str) -> dict:
    p = subprocess.run([beatscript, "listen", "--file", path], capture_output=True, text=True, timeout=TIMEOUT_S)
    try:
        v = json.loads(p.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        raise RuntimeError(f"beatscript listen {os.path.basename(path)}: no JSON (exit {p.returncode}): {p.stderr.strip()[-400:]}")
    if "error" in v:
        raise RuntimeError(f"beatscript listen {os.path.basename(path)}: {v['error'].get('code')}: {v['error'].get('message')}")
    return v


def golden_of(v: dict) -> dict:
    return {k: v[k] for k in KEEP if k in v}


def expects(path: str) -> list[tuple[str, dict]]:
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*expect:\s*(silent|heard)\s+(.*)$", line.strip())
            if m:
                out.append((m.group(1), dict(kv.split("=", 1) for kv in m.group(2).split() if "=" in kv)))
    return out


def check_expects(v: dict, exp: list[tuple[str, dict]]) -> list[str]:
    errs = []
    for kind, want in exp:
        deck = want.get("deck")
        part = want.get("part")
        sec = want.get("section")
        hits = [s for s in v.get("silent", []) if s.get("deck") == deck and (part is None or s.get("part") == part) and (sec is None or s.get("section") == sec)]
        if kind == "silent":
            if not hits:
                errs.append(f"expected deck {deck}{' part ' + part if part else ''} silent, and listen hears it")
            elif "reason" in want and not any(s.get("reason") == want["reason"] for s in hits):
                errs.append(f"expected deck {deck} silent for {want['reason']}, listen says {sorted({s.get('reason') for s in hits})}")
        else:
            d = v.get("decks", {}).get(deck)
            if d is None:
                errs.append(f"expected deck {deck} heard: listen has no deck {deck}")
            elif (d.get("silent") and sec is None) or hits:
                errs.append(f"expected deck {deck} heard, listen says silent: {[s.get('why') for s in hits]}")
    return errs


def _lufs(a, b, where: str, out: list[str]):
    if (a is None) != (b is None):
        out.append(f"{where}: loudness {a} -> {b}")
    elif a is not None and abs(a - b) > LUFS_TOL:
        out.append(f"{where}: loudness {a} -> {b} LUFS (tolerance {LUFS_TOL})")


def compare(g: dict, v: dict) -> tuple[str, list[str]]:
    """('pass' | 'fail' | 'stale', reasons)."""
    if g.get("version") != v.get("version"):
        return "stale", [f"summary version {g.get('version')} -> {v.get('version')}: run --update"]
    if g.get("cid") != v.get("cid"):
        return "stale", [f"the program changed (cid {g.get('cid')} -> {v.get('cid')}): run --update"]
    out: list[str] = []
    gs = {(s.get("deck"), s.get("part"), s.get("section"), s.get("reason")) for s in g.get("silent", [])}
    vs = {(s.get("deck"), s.get("part"), s.get("section"), s.get("reason")) for s in v.get("silent", [])}
    for s in sorted(vs - gs, key=str):
        out.append(f"SILENT now: deck {s[0]}{' part ' + s[1] if s[1] else ''}{' in ' + s[2] if s[2] else ''} ({s[3]})")
    for s in sorted(gs - vs, key=str):
        out.append(f"heard now (golden: silent): deck {s[0]}{' part ' + s[1] if s[1] else ''}{' in ' + s[2] if s[2] else ''} ({s[3]})")
    _lufs(g.get("master", {}).get("lufs"), v.get("master", {}).get("lufs"), "master", out)
    for name, gd in g.get("decks", {}).items():
        vd = v.get("decks", {}).get(name)
        if vd is None:
            out.append(f"deck {name} is gone")
            continue
        if bool(gd.get("silent")) != bool(vd.get("silent")):
            out.append(f"deck {name}: silent {gd.get('silent')} -> {vd.get('silent')}")
        _lufs(gd.get("lufs"), vd.get("lufs"), f"deck {name}", out)
    vparts = {(p.get("deck"), p.get("part")): p for p in v.get("parts", [])}
    for gp in g.get("parts", []):
        vp = vparts.get((gp.get("deck"), gp.get("part")))
        where = f"deck {gp.get('deck')} part {gp.get('part')}"
        if vp is None:
            out.append(f"{where} is gone")
            continue
        if gp.get("measured") and vp.get("measured"):
            if bool(gp.get("silent")) != bool(vp.get("silent")):
                out.append(f"{where}: silent {gp.get('silent')} -> {vp.get('silent')}")
            _lufs(gp.get("lufs"), vp.get("lufs"), where, out)
    for gsec, vsec in zip(g.get("sections", []), v.get("sections", [])):
        _lufs(gsec.get("lufs"), vsec.get("lufs"), f"section {gsec.get('name') or ''} at bar {gsec.get('from_bar')}".replace("  ", " "), out)
    if len(g.get("sections", [])) != len(v.get("sections", [])):
        out.append(f"sections {len(g.get('sections', []))} -> {len(v.get('sections', []))}")
    gt, vt = g.get("detected", {}), v.get("detected", {})
    if gt.get("bpm_match") != vt.get("bpm_match"):
        out.append(f"tempo heard: {gt.get('bpm_match')} ({gt.get('bpm')}) -> {vt.get('bpm_match')} ({vt.get('bpm')}), declared {vt.get('declared_bpm')}")
    elif gt.get("bpm") and vt.get("bpm") and abs(gt["bpm"] - vt["bpm"]) > BPM_TOL * gt["bpm"]:
        out.append(f"tempo heard: {gt['bpm']} -> {vt['bpm']} bpm")
    if gt.get("key") != vt.get("key") or gt.get("key_match") != vt.get("key_match"):
        out.append(f"key heard: {gt.get('key')} ({gt.get('key_match')}) -> {vt.get('key')} ({vt.get('key_match')}), declared {vt.get('declared_key')}")
    gc, vc = g.get("master", {}).get("clipped_samples", 0), v.get("master", {}).get("clipped_samples", 0)
    if not gc and vc:
        out.append(f"the master clips now: {vc} samples over full scale")
    return ("fail" if out else "pass"), out


def programs(examples: str | None, only: list[str]) -> list[tuple[str, str, str]]:
    """(name, path, kind)."""
    out = []
    if examples:
        for p in sorted(glob.glob(os.path.join(examples, "*.beatscript"))):
            out.append((os.path.splitext(os.path.basename(p))[0], p, "example"))
    for p in sorted(glob.glob(os.path.join(FIXTURES, "*.beatscript"))):
        out.append(("fixture-" + os.path.splitext(os.path.basename(p))[0], p, "fixture"))
    if only:
        out = [x for x in out if x[0] in only]
    return out


def publish(report: dict) -> None:
    ev = {"_source": "beatscript-listen", "_event": "ListenRun", "ok": report["ok"], "counts": report["counts"],
          "failed": [{"name": r["name"], "status": r["status"], "reasons": r["reasons"][:4]} for r in report["results"] if r["status"] != "pass"][:20],
          "beatscript": report.get("beatscript"), "commit": report.get("commit"), "ts": report["ts"]}
    data = json.dumps(ev, separators=(",", ":")).encode()[:8192]
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        s.settimeout(0.5)
        s.sendto(data, BUS_SOCK)
        s.close()
    except OSError as e:
        print(f"listen_ci: bus: {e}", file=sys.stderr)
    try:
        os.makedirs(os.path.dirname(LAST), exist_ok=True)
        tmp = LAST + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(report, f, indent=1)
        os.replace(tmp, LAST)
    except OSError as e:
        print(f"listen_ci: {LAST}: {e}", file=sys.stderr)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--beatscript", default=os.environ.get("BEATSCRIPT_CLI", "beatscript"))
    ap.add_argument("--examples", default=os.environ.get("BEATSCRIPT_EXAMPLES"))
    ap.add_argument("--only", action="append", default=[])
    ap.add_argument("--update", action="store_true")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--publish", action="store_true")
    ap.add_argument("--commit", default=os.environ.get("LISTEN_COMMIT"))
    a = ap.parse_args(argv)
    progs = programs(a.examples, a.only)
    if not progs:
        print("listen_ci: no programs (give --examples DIR)", file=sys.stderr)
        return 2
    results = []
    t0 = time.time()
    for name, path, kind in progs:
        r = {"name": name, "kind": kind, "status": "pass", "reasons": []}
        try:
            v = listen(a.beatscript, path)
        except FileNotFoundError:
            print(f"listen_ci: no beatscript at {a.beatscript}", file=sys.stderr)
            return 2
        except (RuntimeError, subprocess.TimeoutExpired) as e:
            r.update(status="fail", reasons=[str(e)])
            results.append(r)
            continue
        r["ms"] = v.get("ms", {}).get("total")
        errs = check_expects(v, expects(path))
        gpath = os.path.join(GOLDENS, name + ".json")
        if a.update:
            if errs:
                r.update(status="fail", reasons=errs + ["golden not written"])
            else:
                os.makedirs(GOLDENS, exist_ok=True)
                with open(gpath, "w", encoding="utf-8") as f:
                    json.dump(golden_of(v), f, indent=1, sort_keys=True)
                    f.write("\n")
                r["status"] = "updated"
        elif not os.path.exists(gpath):
            r.update(status="stale", reasons=errs + [f"no golden {os.path.relpath(gpath, HERE)}: run --update"])
        else:
            with open(gpath, encoding="utf-8") as f:
                g = json.load(f)
            st, why = compare(g, golden_of(v))
            r.update(status="fail" if errs else st, reasons=errs + why)
        results.append(r)
    counts: dict[str, int] = {}
    for r in results:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    ok = all(r["status"] in ("pass", "updated") for r in results)
    report = {"ok": ok, "counts": counts, "results": results, "seconds": round(time.time() - t0, 1), "beatscript": a.beatscript,
              "commit": a.commit, "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    if a.publish:
        publish(report)
    if a.json:
        print(json.dumps(report, indent=1))
    else:
        for r in results:
            mark = {"pass": "ok  ", "updated": "upd ", "stale": "STALE", "fail": "FAIL"}[r["status"]]
            print(f"{mark} {r['name']}" + (f"  ({r.get('ms', 0) / 1000:.1f}s)" if r.get("ms") else ""))
            for why in r["reasons"]:
                print(f"       {why}")
        print(f"listen: {'ok' if ok else 'FAILED'} {counts} in {report['seconds']}s")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
