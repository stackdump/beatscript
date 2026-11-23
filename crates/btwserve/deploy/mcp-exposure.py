#!/usr/bin/env python3
"""The MCP exposure check (project-datum 6679), live: the table btwserve's test runs, over MCP on HTTPS.

    python3 -I crates/btwserve/deploy/mcp-exposure.py [URL] [--token TOKEN]     default https://beatscript.app

Reads crates/btwserve/src/mcp/testdata/exposure.json (the same rows mcp::tools::exposure checks against the real
Tools in `cargo test`): each feature's tools/list text must say what the row says, and each call's answer must carry
what it expects. Calls marked auth (a signed-in caller; eval, load and queue also need a tab) or stores (they store a
program on the site) run only with --token (or BTW_TOKEN); without one they are listed as skipped, as is a call whose
${name} an earlier call did not save. Exits 1 on any failure, printing each one.
"""
import json
import os
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
TABLE = os.path.join(HERE, "..", "src", "mcp", "testdata", "exposure.json")


def rpc(url, token, method, params, n=[0]):
    n[0] += 1
    body = json.dumps({"jsonrpc": "2.0", "id": n[0], "method": method, "params": params}).encode()
    h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream", "User-Agent": "beatscript-deploy-check/1"}
    if token:
        h["Authorization"] = "Bearer " + token
    with urllib.request.urlopen(urllib.request.Request(url, body, h), timeout=60) as r:
        d = json.loads(r.read())
    if "error" in d:
        raise RuntimeError(d["error"].get("message", d["error"]))
    return d["result"]


def pointer(v, p):
    """RFC 6901, as serde_json's Value::pointer: (found, value)."""
    if p == "":
        return True, v
    for tok in p.split("/")[1:]:
        tok = tok.replace("~1", "/").replace("~0", "~")
        if isinstance(v, dict) and tok in v:
            v = v[tok]
        elif isinstance(v, list) and tok.isdigit() and int(tok) < len(v):
            v = v[int(tok)]
        else:
            return False, None
    return True, v


def as_text(v):
    # as the Rust runner: a string is itself, anything else its JSON (serde_json's compact form)
    return v if isinstance(v, str) else json.dumps(v, separators=(",", ":"), ensure_ascii=False)


def fill(v, saved):
    if isinstance(v, str):
        for k, x in saved.items():
            v = v.replace("${%s}" % k, x)
        return v
    if isinstance(v, list):
        return [fill(x, saved) for x in v]
    if isinstance(v, dict):
        return {k: fill(x, saved) for k, x in v.items()}
    return v


def expect(ans, e, saved):
    e = fill(e, saved)
    ptr, op = e[0], e[1]
    found, got = pointer(ans, ptr)
    want = e[2] if len(e) > 2 else None
    if op == "=":
        ok = found and got == want and type(got) is type(want)
    elif op == "has":
        ok = found and got is not None
    elif op == "~":
        ok = found and as_text(want) in as_text(got)
    elif op == "!~":
        ok = not (found and as_text(want) in as_text(got))
    else:
        return "unknown op %s" % op
    if ok:
        return None
    return "%s %s %s: got %s" % (ptr, op, "" if want is None else as_text(want), as_text(got) if found else "nothing")


def main(argv):
    args = [a for a in argv if not a.startswith("--token")]
    token = os.environ.get("BTW_TOKEN", "")
    for i, a in enumerate(argv):
        if a == "--token" and i + 1 < len(argv):
            token = argv[i + 1]
            args.remove(argv[i + 1])
        elif a.startswith("--token="):
            token = a.split("=", 1)[1]
    base = (args[0] if args else "https://beatscript.app").rstrip("/")
    url = base + "/mcp"
    with open(TABLE, encoding="utf-8") as f:
        table = json.load(f)
    tools = {t["name"]: t["description"] + "\n" + json.dumps(t["inputSchema"], separators=(",", ":"), ensure_ascii=False) for t in rpc(url, token, "tools/list", {})["tools"]}
    fails, skipped, ran = [], [], 0
    for feat in table["features"]:
        fid = feat["id"]
        for tool, text in feat["say"]:
            if tool not in tools:
                fails.append("%s: no tool %s" % (fid, tool))
            elif text not in tools[tool]:
                fails.append("%s: %s's description and schema do not say %r" % (fid, tool, text))
        saved = {}
        for c in feat["calls"]:
            tool = c["tool"]
            a = fill(c["args"], saved)
            if (c.get("auth") or c.get("stores")) and not token:
                skipped.append("%s: %s (needs a token)" % (fid, tool))
                continue
            if "${" in json.dumps(a):
                skipped.append("%s: %s (an earlier call it needs was skipped)" % (fid, tool))
                continue
            try:
                r = rpc(url, token, "tools/call", {"name": tool, "arguments": a})
            except Exception as e:  # noqa: BLE001 (any failure is the check's answer)
                fails.append("%s: %s: %s" % (fid, tool, e))
                continue
            blocks = [x.get("text", "") for x in r.get("content", []) if x.get("type") == "text"]
            text = "".join(blocks)
            if r.get("isError"):
                fails.append("%s: %s: tool error: %s" % (fid, tool, text[:300]))
                continue
            try:
                # a page (artifact) is two blocks, its JSON summary and the page: read as the summary with the page as
                # `html`, as the Rust runner does
                ans = json.loads(blocks[0] if len(blocks) > 1 else text)
                if len(blocks) > 1 and isinstance(ans, dict):
                    ans["html"] = blocks[1]
            except ValueError:
                fails.append("%s: %s: the answer is not JSON: %s" % (fid, tool, text[:200]))
                continue
            ran += 1
            for k, p in (c.get("save") or {}).items():
                found, v = pointer(ans, p)
                if found and isinstance(v, str):
                    saved[k] = v
            for e in c["expect"]:
                m = expect(ans, e, saved)
                if m:
                    fails.append("%s: %s: %s" % (fid, tool, m))
    for s in skipped:
        print("skipped: " + s)
    if fails:
        print("MCP exposure (6679) FAILED at %s:\n  %s" % (base, "\n  ".join(fails)))
        return 1
    print("MCP exposure (6679) ok at %s: %d features described, %d calls carry their fields, %d skipped" % (base, len(table["features"]), ran, len(skipped)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
