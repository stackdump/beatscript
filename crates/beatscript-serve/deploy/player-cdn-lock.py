#!/usr/bin/env python3
"""Write crates/beatscript-serve/src/mcp/player-cdn.lock.json: the @stackdump/beatscript-player the MCP `artifact` page loads.

    python3 -I crates/beatscript-serve/deploy/player-cdn-lock.py VERSION PACKAGE_DIR [--jsdelivr] [--radio]

--radio writes src/mcp/radio-cdn.lock.json instead, the lock of the artifact tool's station page (its entry
beatscript-radio.js, from 0.4.0): the radio moves to a new version without moving <beatscript-track>'s pages.

PACKAGE_DIR is the published package unpacked (`npm pack @stackdump/beatscript-player@VERSION`, then
`tar xzf stackdump-beatscript-player-VERSION.tgz`: its `package/`), never a local build: the hashes must be of the
bytes jsDelivr serves, which are the registry tarball's. --jsdelivr also fetches every file from
cdn.jsdelivr.net/npm/<package>@VERSION/ and refuses if any differs. The tarball's own integrity is read from
`npm view` when npm is on PATH. A file's integrity is its SRI sha384; sha256 and bytes are there to check by hand
(and against the package's hashes.json). Pinned to an exact version: never a range, a tag or latest.
"""
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys
import urllib.request

PACKAGE = "@stackdump/beatscript-player"
ENTRY = "beatscript-track.js"
RADIO_ENTRY = "beatscript-radio.js"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src", "mcp", "player-cdn.lock.json")
RADIO_OUT = os.path.join(HERE, "..", "src", "mcp", "radio-cdn.lock.json")
COMMENTS = {
    ENTRY: "The MCP artifact page (src/mcp/artifact.rs) loads this package from jsDelivr: the entry as a module script with this integrity and crossorigin=anonymous, and every other module it imports through an import map's integrity, so a changed file anywhere in the graph refuses to run. Pinned to an exact version, never a range, a tag or latest. artifact::tests hold the page to this lock (BEATSCRIPT_PLAYER_DIR=<unpacked package> also checks the bytes). To move it: publish a new version, then run crates/beatscript-serve/deploy/player-cdn-lock.py VERSION <unpacked package> --jsdelivr.",
    RADIO_ENTRY: "The MCP artifact tool's station page (src/mcp/artifact.rs StationPage) loads <beatscript-radio> from this package on jsDelivr: the entry as a module script with this integrity and crossorigin=anonymous, and every other module it imports through an import map's integrity, so a changed file anywhere in the graph refuses to run. Pinned to an exact version, never a range, a tag or latest; a lock of its own (player-cdn.lock.json pins <beatscript-track>'s pages, the VS Code preview's and the Jupyter embed's), so the radio can move to a new version without moving them. artifact::tests hold the page to this lock (BEATSCRIPT_RADIO_PLAYER_DIR=<unpacked package> also checks the bytes). To move it: publish a new version, then run crates/beatscript-serve/deploy/player-cdn-lock.py VERSION <unpacked package> --radio --jsdelivr.",
}


def sri(b):
    return "sha384-" + base64.b64encode(hashlib.sha384(b).digest()).decode()


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    version, pkg = argv[0], argv[1]
    cdn = "--jsdelivr" in argv
    entry, out_path = (RADIO_ENTRY, RADIO_OUT) if "--radio" in argv else (ENTRY, OUT)
    meta = json.load(open(os.path.join(pkg, "package.json")))
    if meta["name"] != PACKAGE or meta["version"] != version:
        print("refused: %s is %s@%s, not %s@%s" % (pkg, meta["name"], meta["version"], PACKAGE, version))
        return 1
    files = {}
    for root, _, names in os.walk(pkg):
        for n in names:
            p = os.path.join(root, n)
            rel = os.path.relpath(p, pkg).replace(os.sep, "/")
            if not rel.endswith(".js"):
                continue
            b = open(p, "rb").read()
            files[rel] = {"integrity": sri(b), "sha256": hashlib.sha256(b).hexdigest(), "bytes": len(b)}
            if cdn:
                url = "https://cdn.jsdelivr.net/npm/%s@%s/%s" % (PACKAGE, version, rel)
                req = urllib.request.Request(url, headers={"User-Agent": "beatscript-player-lock/1"})
                with urllib.request.urlopen(req, timeout=120) as r:
                    got = r.read()
                if got != b:
                    print("refused: %s differs from the tarball's %s" % (url, rel))
                    return 1
    if entry not in files:
        print("refused: the package has no %s" % entry)
        return 1
    tarball = ""
    if shutil.which("npm"):
        out = subprocess.run(["npm", "view", "%s@%s" % (PACKAGE, version), "dist.integrity"], capture_output=True, text=True)
        tarball = out.stdout.strip()
    how = "the registry tarball (npm pack %s@%s)" % (PACKAGE, version) + (", every file also fetched from cdn.jsdelivr.net and byte-identical" if cdn else "")
    lock = {
        "_comment": COMMENTS[entry],
        "package": PACKAGE,
        "version": version,
        "entry": entry,
        "source": how,
        "tarball_integrity": tarball,
        "files": dict(sorted(files.items())),
    }
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(json.dumps(lock, indent=1, ensure_ascii=False) + "\n")
    print("wrote %s: %s@%s, %d files, from %s" % (os.path.relpath(out_path), PACKAGE, version, len(files), how))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
