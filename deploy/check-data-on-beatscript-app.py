#!/usr/bin/env python3
"""BeatScript data lives on beatscript.app only: fail when a served page, a program's linked data or an MCP answer says
"bitwrap.io", except the exact texts deploy/bitwrap-allowlist.tsv lists (history and attribution, never a link).

    python3 -I deploy/check-data-on-beatscript-app.py [ORIGIN]     default https://beatscript.app

Fetches the pages a reader or an agent follows (the home page, llms.txt, the docs, the schema and the @context, the
open-data feeds, the ontology page, a stored program's /p, /c and .jsonld forms, with their response headers) and calls
the MCP tools whose answers carry links or IRIs (tools/list, share, open, check with a genre, genre_template,
list_examples, instruments). Prints one line per finding with the text around it; exit 1 if there is any, 0 if none.
Python standard library only. Run by deploy/live-checks.sh after every deploy.
"""

import json
import os
import sys
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ORIGIN = (sys.argv[1] if len(sys.argv) > 1 else "https://beatscript.app").rstrip("/")
NEEDLE = "bitwrap.io"
UA = {"User-Agent": "beatscript-live-check"}

PAGES = [
    "/", "/llms.txt", "/docs/", "/docs/cheatsheet.md", "/docs/BEATSCRIPT.md", "/docs/api.md", "/docs/forms.md",
    "/docs/found-phrases.md", "/schema", "/context/v1.jsonld", "/context/beatscript-v1.jsonld", "/feed.json", "/feed.xml",
    "/atom.xml", "/feed.atom", "/sitemap.xml", "/radio", "/embed.js", "/authorship", "/privacy/", "/ontology/", "/api/info",
    "/llms-full.txt", "/schema.md", "/schema.jsonld", "/grammar", "/themes", "/themes.jsonld", "/phrasebook", "/phrasebook.jsonld",
    "/docs/embed.md", "/docs/radio.md", "/docs/open-data.md", "/.well-known/beatscript-site", "/beatscript-capabilities.json",
]


def allowlist():
    out = []
    for line in open(os.path.join(HERE, "bitwrap-allowlist.tsv"), encoding="utf-8"):
        if not line.strip() or line.startswith("#"):
            continue
        f = line.rstrip("\n").split("\t")
        if len(f) >= 2:
            out.append((f[0], f[1]))
    return out


ALLOW = allowlist()


def fetch(path, data=None, headers=None):
    req = urllib.request.Request(ORIGIN + path, data=data, headers={**UA, **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, "".join(f"{k}: {v}\n" for k, v in r.getheaders()), r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, "".join(f"{k}: {v}\n" for k, v in e.headers.items()), e.read().decode("utf-8", "replace")


def findings(where, text):
    for prefix, allowed in ALLOW:
        if where == prefix or where.startswith(prefix):
            text = text.replace(allowed, "")
    out, i = [], text.find(NEEDLE)
    while i >= 0:
        out.append(text[max(0, i - 70): i + 40].replace("\n", " "))
        i = text.find(NEEDLE, i + 1)
    return out


def mcp(method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}).encode()
    st, hdr, b = fetch("/mcp", body, {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    return st, hdr, b


def tool_text(b):
    try:
        return json.loads(json.loads(b)["result"]["content"][0]["text"])
    except Exception:
        return None


def main():
    bad, checked = [], 0

    def check(where, *texts):
        nonlocal checked
        checked += 1
        for t in texts:
            for f in findings(where, t):
                bad.append(f"{where}: ...{f}...")

    for p in PAGES:
        st, hdr, body = fetch(p)
        if st == 404:
            continue
        check(p, hdr, body)

    # a stored program, from the open-data feed (else an example's: a fresh server has stored none)
    st, _, feed = fetch("/feed.json")
    cid = None
    try:
        cid = json.loads(feed)["items"][0]["id"].rstrip("/").rsplit("/", 1)[-1]
    except Exception:
        pass
    if not cid:
        try:
            ex = tool_text(mcp("tools/call", {"name": "list_examples", "arguments": {}})[2])
            ex = ex.get("examples", ex) if isinstance(ex, dict) else ex
            cid = next(e["cid"] for e in ex if isinstance(e, dict) and e.get("cid"))
        except Exception:
            pass
    if cid:
        for p in (f"/p/{cid}", f"/c/{cid}", f"/p/{cid}.jsonld", f"/c/{cid}.jsonld", f"/api/programs/{cid}"):
            st, hdr, body = fetch(p)
            check(p, hdr, body)
    else:
        bad.append("feed.json and list_examples: no program to check the /p, /c and .jsonld forms with")

    # MCP: the tool list (descriptions) and the answers that carry links or IRIs
    st, hdr, b = mcp("tools/list")
    check("mcp:tools/list", hdr, b)
    st, _, b = mcp("tools/call", {"name": "list_examples", "arguments": {}})
    check("mcp:list_examples", b)
    ex = tool_text(b)
    name = None
    if isinstance(ex, dict):
        items = ex.get("examples") or ex.get("items") or []
        name = items[0].get("name") if items and isinstance(items[0], dict) else None
    text = "#! genre: techno\ntempo 128\ndeck A: kick x ~ x ~\n"
    if name:
        _, _, b = mcp("tools/call", {"name": "list_examples", "arguments": {"name": name}})
        one = tool_text(b)
        if isinstance(one, dict) and isinstance(one.get("text"), str):
            text = one["text"]
    for tool, args in (
        ("share", {"text": text}),
        ("check", {"text": "#! genre: techno\ntempo 128\ndeck A: kick x ~ x ~\n"}),
        ("check", {"text": text, "genre": "trap"}),
        ("genre_template", {"genre": "techno"}),
        ("instruments", {}),
    ):
        _, _, b = mcp("tools/call", {"name": tool, "arguments": args})
        check(f"mcp:{tool}", b)
        if tool == "share":
            sh = tool_text(b) or {}
            url = sh.get("url") if isinstance(sh, dict) else None
            # the program as linked data, from the link alone (a social /c/<CID>.<packed>.jsonld), and the context it names
            for k in ("jsonld_url", "context"):
                u = sh.get(k) if isinstance(sh, dict) else None
                if isinstance(u, str) and "/" in u.split("://", 1)[-1]:
                    path = "/" + u.split("://", 1)[-1].split("/", 1)[1]
                    st2, hdr2, body2 = fetch(path)
                    check(path if k == "context" else "/c/<link>.jsonld", hdr2, body2)
            if url:
                _, _, b2 = mcp("tools/call", {"name": "open", "arguments": {"url": url}})
                check("mcp:open", b2)
            else:
                bad.append(f"mcp:share: no url in the answer: {b[:200]}")

    for line in bad:
        print("bitwrap.io:", line)
    print(f"{'FAIL' if bad else 'ok'}: {checked} answers from {ORIGIN} checked, {len(bad)} with bitwrap.io outside deploy/bitwrap-allowlist.tsv")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
