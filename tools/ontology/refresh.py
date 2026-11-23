#!/usr/bin/env python3
"""Refresh crates/djhost/data/beatscript-ontology.json from the BeatScript ontology release in this repository.

The snapshot is of the BeatScript ontology (https://beatscript.app/ontology/, a CC0 fork of the bitwrap ontology 0.2.0,
crates/beatscript-serve/ontology/README.md): it is read from the release files in git
(crates/beatscript-serve/ontology/release/ns.nt, checked against release.lock and manifest.json). Nothing is fetched.

    python3 tools/ontology/refresh.py            # after crates/beatscript-serve/ontology/build.sh
    python3 tools/ontology/refresh.py --fetched 2026-11-22

The snapshot is what a BeatScript program's `#! genre:` / `#! tags:` header lines are validated against (docs/beatscript/ext-linked.md).
It is small and deterministic: the terms an author may name (genres, genre families, scales, grooves, instruments,
instrument roles, song sections, song structures, Euclidean rhythms, chord progressions, engine concepts), sorted by
IRI, one per line, with their local name, label and kind; a Euclidean rhythm also has its pulses, steps and rotation
(so a program's e(k,n,rot) can be linked to the rhythms that have the same shape). Only the music terms the release
carries from its upstream (those with a prov:wasDerivedFrom) are kept: the BeatScript vocabulary's own individuals (the
grammar, voices, tools) are not header terms. Nothing else of the ontology is kept. Python standard library only.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import sys

BASE = "https://beatscript.app/ontology/"
PROV_DERIVED = "http://www.w3.org/ns/prov#wasDerivedFrom"
HERE = os.path.dirname(os.path.abspath(__file__))
ONTOLOGY = os.path.join(HERE, "..", "..", "crates", "beatscript-serve", "ontology")
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


def parse(nt_text):
    types, labels, ints, derived = {}, {}, {}, set()
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
        if p == PROV_DERIVED:
            derived.add(s)
        elif p == RDF_TYPE and o.startswith("<"):
            types.setdefault(s, set()).add(o[1:-1])
        elif p in (RDFS_LABEL, SKOS_PREF):
            lm = LITERAL.match(o)
            if lm and (p == RDFS_LABEL or s not in labels):
                labels[s] = unescape(lm.group(1))
        elif p in (NS + "pulses", NS + "steps", NS + "rotation"):
            lm = LITERAL.match(o)
            if lm and lm.group(1).lstrip("-").isdigit():
                ints.setdefault(s, {})[p[len(NS):]] = int(lm.group(1))
    return {k: v for k, v in types.items() if k in derived}, labels, ints


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
    ap.add_argument("--release", default=os.path.join(ONTOLOGY, "release"), help="the release directory (default: the repository's)")
    ap.add_argument("--fetched", help="the date to record (YYYY-MM-DD); default: today")
    ap.add_argument("--out", default=os.path.join(HERE, "..", "..", "crates", "djhost", "data", "beatscript-ontology.json"))
    a = ap.parse_args()

    raw = open(os.path.join(a.release, "ns.nt"), "rb").read()
    manifest = json.load(open(os.path.join(a.release, "manifest.json"), encoding="utf-8"))
    got = hashlib.sha256(raw).hexdigest()
    want = manifest.get("files", {}).get("ns.nt")
    lock = os.path.join(a.release, "..", "release.lock")
    locked = None
    if os.path.exists(lock):
        for l in open(lock, encoding="utf-8"):
            f = l.split()
            if len(f) == 2 and f[1] == "ns.nt":
                locked = f[0]
    if got != want or (locked is not None and got != locked):
        sys.exit(f"ns.nt ({got}) does not match the release's manifest ({want}) / release.lock ({locked}): not written")
    verified = True
    version, released, pack_digest = manifest.get("version", "unknown"), manifest.get("released", "unknown"), manifest.get("pack_content_sha256", "unknown")
    fetched = a.fetched or datetime.date.today().isoformat()

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
        "refresh": "python3 tools/ontology/refresh.py   (offline, from crates/beatscript-serve/ontology/release; then cargo test -p djhost)",
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
