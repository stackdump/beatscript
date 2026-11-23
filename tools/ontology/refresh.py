#!/usr/bin/env python3
"""Refresh crates/djhost/data/bitwrap-ontology.json from the published bitwrap ontology.

The snapshot is of the bitwrap ontology itself, not of the BeatScript ontology served at beatscript.app/ontology/ (a
fork of bitwrap 0.2.0, crates/beatscript-serve/ontology/README.md): programs link to bitwrap's IRIs, which are upstream and stay
valid. Offline, the bitwrap release the fork was derived from is vendored at
crates/beatscript-serve/ontology/upstream/bitwrap-0.2.0/ns.nt (pass it with --from).

    python3 tools/ontology/refresh.py                       # fetch https://ontology.bitwrap.io/ns.nt (needs network)
    python3 tools/ontology/refresh.py --from ns.nt --fetched 2026-11-18   # offline, from a saved copy of ns.nt

The snapshot is what `btw`'s `#! genre:` / `#! tags:` header lines are validated against (docs/beatscript/ext-linked.md).
It is small and deterministic: the terms an author may name (genres, genre families, scales, grooves, instruments,
instrument roles, song sections, song structures, Euclidean rhythms, chord progressions, engine concepts), sorted by
IRI, one per line, with their local name, label and kind; a Euclidean rhythm also has its pulses, steps and rotation
(so a program's e(k,n,rot) can be linked to the rhythms that have the same shape). Nothing else of the ontology is kept.

The fetched file is checked against the release's SHA256SUMS before it is believed. Python standard library only.
The published data is untrusted input: only IRIs under https://ontology.bitwrap.io/ are read, and only as strings.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import sys
import urllib.request

BASE = "https://ontology.bitwrap.io/"
NS = BASE + "ns#"
RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
RDFS_LABEL = "http://www.w3.org/2000/01/rdf-schema#label"
SKOS_PREF = "http://www.w3.org/2004/02/skos/core#prefLabel"

# the class of an individual -> the kind a header names it by (also its path segment under BASE)
KINDS = {
    NS + "Genre": "genre",
    NS + "GenreFamily": "family",
    NS + "Scale": "scale",
    NS + "Groove": "groove",
    NS + "Instrument": "instrument",
    NS + "InstrumentRole": "role",
    NS + "SongSection": "section",
    NS + "SongStructure": "structure",
    NS + "EuclideanRhythm": "rhythm",
    NS + "ChordProgression": "progression",
    NS + "EngineConcept": "concept",
}

TRIPLE = re.compile(r'^(<[^>]*>|_:\S+)\s+<([^>]*)>\s+(.*?)\s*\.$')
LITERAL = re.compile(r'^"((?:[^"\\]|\\.)*)"(?:@[A-Za-z-]+|\^\^<[^>]*>)?$')


def unescape(s):
    return re.sub(r'\\(["\\nrt]|u[0-9a-fA-F]{4})', lambda m: {'"': '"', "\\": "\\", "n": "\n", "r": "\r", "t": "\t"}.get(m.group(1)) or chr(int(m.group(1)[1:], 16)), s)


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "djrust-ontology-refresh"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read(), r.headers.get("Date")


def parse(nt_text):
    types, labels, ints = {}, {}, {}
    for line in nt_text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = TRIPLE.match(line)
        if not m:
            continue
        s, p, o = m.groups()
        if not s.startswith("<" + BASE) or s.startswith("<" + NS):
            continue
        s = s[1:-1]
        if p == RDF_TYPE and o.startswith("<"):
            types.setdefault(s, set()).add(o[1:-1])
        elif p in (RDFS_LABEL, SKOS_PREF):
            lm = LITERAL.match(o)
            if lm and (p == RDFS_LABEL or s not in labels):
                labels[s] = unescape(lm.group(1))
        elif p in (NS + "pulses", NS + "steps", NS + "rotation"):
            lm = LITERAL.match(o)
            if lm and lm.group(1).lstrip("-").isdigit():
                ints.setdefault(s, {})[p[len(NS):]] = int(lm.group(1))
    return types, labels, ints


def terms(types, labels, ints):
    out = []
    for iri in sorted(types):
        kinds = sorted({KINDS[t] for t in types[iri] if t in KINDS})
        if not kinds:
            continue
        kind = kinds[0]
        rest = iri[len(BASE):]
        seg, _, local = rest.partition("/")
        if seg != kind or not local or "/" in local or "#" in local:
            continue
        term = {"iri": iri, "kind": kind, "local": local, "label": labels.get(iri, local)}
        if kind == "rhythm":
            r = ints.get(iri, {})
            if all(k in r for k in ("pulses", "steps", "rotation")):
                term.update(pulses=r["pulses"], steps=r["steps"], rotation=r["rotation"])
        out.append(term)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--from", dest="src", help="read this saved ns.nt instead of fetching it")
    ap.add_argument("--fetched", help="the date to record (YYYY-MM-DD); default: the server's Date, or today")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates", "djhost", "data", "bitwrap-ontology.json"))
    a = ap.parse_args()

    version, released, pack_digest, fetched = "unknown", "unknown", "unknown", a.fetched
    if a.src:
        raw = open(a.src, "rb").read()
        verified = False
    else:
        raw, date = fetch(BASE + "ns.nt")
        sums, _ = fetch(BASE + "release/SHA256SUMS")
        want = {l.split()[1]: l.split()[0] for l in sums.decode().splitlines() if len(l.split()) == 2}.get("ns.nt")
        got = hashlib.sha256(raw).hexdigest()
        if want != got:
            sys.exit(f"ns.nt does not match the release's SHA256SUMS ({got} against {want}): not written")
        verified = True
        void, _ = fetch(BASE + ".well-known/void")
        m = re.search(r'owl:versionInfo "([^"]+)"', void.decode())
        version = m.group(1) if m else version
        m = re.search(r'dcterms:modified "([^"]+)"', void.decode())
        released = m.group(1) if m else released
        llms, _ = fetch(BASE + "llms.txt")
        m = re.search(r"Pack content digest: ([0-9a-f]{64})", llms.decode())
        pack_digest = m.group(1) if m else pack_digest
        if not fetched:
            fetched = datetime.datetime.strptime(date, "%a, %d %b %Y %H:%M:%S %Z").strftime("%Y-%m-%d") if date else datetime.date.today().isoformat()
    if not fetched:
        fetched = datetime.date.today().isoformat()

    types, labels, ints = parse(raw.decode("utf-8"))
    ts = terms(types, labels, ints)
    if not any(t["kind"] == "genre" for t in ts):
        sys.exit("no genres in that file: not written")

    head = {
        "format": 1,
        "type": "djrust.ontology-snapshot",
        "namespace": NS,
        "base": BASE,
        "version": version,
        "released": released,
        "fetched": fetched,
        "source": BASE + "ns.nt",
        "source_sha256": hashlib.sha256(raw).hexdigest(),
        "pack_digest": pack_digest,
        "verified": verified,
        "classes": {"euclidean_rhythm": NS + "EuclideanRhythm", "genre": NS + "Genre"},
        "refresh": "python3 tools/ontology/refresh.py   (needs network; rewrites this file; then cargo test -p djhost)",
    }
    lines = ["{"]
    for k, v in head.items():
        lines.append(f"  {json.dumps(k)}: {json.dumps(v, ensure_ascii=False, sort_keys=True)},")
    lines.append('  "terms": [')
    for i, t in enumerate(ts):
        lines.append("    " + json.dumps(t, ensure_ascii=False, sort_keys=True) + ("," if i + 1 < len(ts) else ""))
    lines.append("  ]")
    lines.append("}")
    with open(a.out, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    kinds = {}
    for t in ts:
        kinds[t["kind"]] = kinds.get(t["kind"], 0) + 1
    print(f"wrote {os.path.normpath(a.out)}: {len(ts)} terms {dict(sorted(kinds.items()))}, verified={verified}")


if __name__ == "__main__":
    main()
