#!/usr/bin/env python3
"""Propose BeatScript lexicon entries from programs' lyrics, for a person to review. It never edits the lexicon.

    tools/lexicon/propose.py [PATH ...]           .btw files or directories of them (default: examples/btw)
    tools/lexicon/propose.py --url URL ...        stored public programs too (a JSON list of {text} or of strings, or a
                                                  .btw / text body), e.g. a dump of beatscript.app's public programs

Prints, most used first, the lyric words the lexicon (crates/djengine/data/lexicon.txt) lacks, one line each:

    harbour<TAB>3<TAB>/h aa r b ow r/   (rules)

the third column the letter-to-sound rules' spelling when BTW_RULES (a btwd binary: `btwd vocal words`) is there, else
blank. A reviewer corrects the spelling, then adds the line to lexicon.txt by hand (sorted, one entry a word) and runs
`cargo test -p djengine phoneme` (every entry must be in the phoneme set). Escaped text (/…/) and @phrases are skipped:
they are phonemes already. Only public, CC0 or otherwise reusable programs should be read.
"""
import json, os, re, subprocess, sys, urllib.request
from collections import Counter

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
LEX = os.path.join(ROOT, "crates", "djengine", "data", "lexicon.txt")

def lexicon():
    words = set()
    for line in open(LEX, encoding="utf-8"):
        line = line.strip()
        if line and not line.startswith("#") and not line.startswith("version "):
            words.add(line.split()[0])
    return words

SING = re.compile(r'\bsing\s+"((?:[^"\\]|\\.)*)"')
VOCAL = re.compile(r'\blyrics\s*=\s*"((?:[^"\\]|\\.)*)"')

def lyrics(text):
    for m in SING.finditer(text):
        yield m.group(1)
    for m in VOCAL.finditer(text):
        yield m.group(1)

def words(lyric):
    lyric = re.sub(r"/[^/]*/", " ", lyric)          # escapes are phonemes already
    for w in re.split(r"[^A-Za-z'’-]+", lyric):
        for piece in w.split("-"):                  # a hyphen cut is the author's spelling: its pieces are not words
            if "-" in w:
                continue
            piece = piece.strip("'’").lower()
            if piece and piece.isascii():
                yield piece

def texts(args):
    paths, urls, it = [], [], iter(args)
    for a in it:
        if a == "--url":
            urls.append(next(it))
        else:
            paths.append(a)
    if not paths and not urls:
        paths = [os.path.join(ROOT, "examples", "btw")]
    for p in paths:
        if os.path.isdir(p):
            for dp, _, fs in os.walk(p):
                for f in sorted(fs):
                    if f.endswith(".btw"):
                        yield open(os.path.join(dp, f), encoding="utf-8").read()
        else:
            yield open(p, encoding="utf-8").read()
    for u in urls:
        body = urllib.request.urlopen(u, timeout=30).read().decode("utf-8", "replace")
        try:
            data = json.loads(body)
            for item in data if isinstance(data, list) else data.get("programs", []):
                yield item if isinstance(item, str) else item.get("text", "")
        except ValueError:
            yield body

def rules_spelling(word):
    exe = os.environ.get("BTW_RULES")
    if not exe:
        return ""
    try:
        out = subprocess.run([exe, "vocal", "words", word], capture_output=True, text=True, timeout=10).stdout.strip()
        return out.splitlines()[0] if out else ""
    except (OSError, subprocess.SubprocessError):
        return ""

def main():
    known = lexicon()
    seen = Counter()
    for t in texts(sys.argv[1:]):
        for l in lyrics(t):
            for w in words(l):
                if w not in known:
                    seen[w] += 1
    if not seen:
        print("# every lyric word is in the lexicon", file=sys.stderr)
    for w, n in sorted(seen.items(), key=lambda x: (-x[1], x[0])):
        print(f"{w}\t{n}\t{rules_spelling(w)}")

if __name__ == "__main__":
    main()
