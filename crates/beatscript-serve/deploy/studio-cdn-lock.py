#!/usr/bin/env python3
"""Write crates/beatscript-serve/src/mcp/studio-cdn.lock.json: the @stackdump/beatscript-studio the MCP `embed` studio page loads.

    python3 -I crates/beatscript-serve/deploy/studio-cdn-lock.py VERSION PACKAGE_DIR [--jsdelivr]

PACKAGE_DIR is the published package unpacked (`npm pack @stackdump/beatscript-studio@VERSION`, then
`tar xzf stackdump-beatscript-studio-VERSION.tgz`: its `package/`), never a local build: the hashes must be of the
bytes jsDelivr serves, which are the registry tarball's. --jsdelivr also fetches every file from
cdn.jsdelivr.net/npm/<package>@VERSION/ and refuses if any differs. The tarball's own integrity is read from
`npm view` when npm is on PATH. A file's integrity is its SRI sha384; sha256 and bytes are there to check by hand.
Pinned to an exact version: never a range, a tag or latest. The lock it writes has no `placeholder` key, so the
embed tool stops saying the studio page cannot run yet.
"""
import base64
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.request

PACKAGE = "@stackdump/beatscript-studio"
ENTRY = "beatscript-studio.js"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src", "mcp", "studio-cdn.lock.json")


def sri(b):
    return "sha384-" + base64.b64encode(hashlib.sha384(b).digest()).decode()


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    version, pkg = argv[0], argv[1]
    cdn = "--jsdelivr" in argv
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?", version):
        print("refused: %s is not an exact version (never a range, a tag or latest)" % version)
        return 1
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
                req = urllib.request.Request(url, headers={"User-Agent": "beatscript-studio-lock/1"})
                with urllib.request.urlopen(req, timeout=120) as r:
                    got = r.read()
                if got != b:
                    print("refused: %s differs from the tarball's %s" % (url, rel))
                    return 1
    if ENTRY not in files:
        print("refused: the package has no %s" % ENTRY)
        return 1
    tarball = ""
    if shutil.which("npm"):
        out = subprocess.run(["npm", "view", "%s@%s" % (PACKAGE, version), "dist.integrity"], capture_output=True, text=True)
        tarball = out.stdout.strip()
    how = "the registry tarball (npm pack %s@%s)" % (PACKAGE, version) + (", every file also fetched from cdn.jsdelivr.net and byte-identical" if cdn else "")
    lock = {
        "_comment": "The MCP embed tool's studio page (src/mcp/embed.rs) loads this package from jsDelivr: the entry as a module script with this integrity and crossorigin=anonymous, and every other module it imports through an import map's integrity, so a changed file anywhere in the module graph refuses to run. Pinned to an exact version, never a range, a tag or latest. embed::tests hold the page to this lock. To move it: publish a new version, then run crates/beatscript-serve/deploy/studio-cdn-lock.py VERSION <unpacked package> --jsdelivr.",
        "package": PACKAGE,
        "version": version,
        "entry": ENTRY,
        "source": how,
        "tarball_integrity": tarball,
        "files": dict(sorted(files.items())),
    }
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(json.dumps(lock, indent=1, ensure_ascii=False) + "\n")
    print("wrote %s: %s@%s, %d files, from %s" % (os.path.relpath(OUT), PACKAGE, version, len(files), how))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
