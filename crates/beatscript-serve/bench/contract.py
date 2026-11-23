#!/usr/bin/env python3
"""Contract test: the same requests to the Go server and to beatscript-serve, answers compared byte for byte.

    contract.py --go 127.0.0.1:28097 --rs 127.0.0.1:28098 --web btwsite/web --examples btwsite/examples \\
                [--groups static,docs,examples,fallback,render] [--out report.json] [-v]

Both servers must run on the same inputs (-web, -examples, -docs, -public-url); beatscript-serve with
`-fallback http://<go>` for the `fallback` group. Each case is sent over a raw socket (parity.py's `http`), so the
status line and the whole header block are compared as written (order, case, values), minus `Date` and
`Connection`, and the bodies by SHA-256. A multipart/byteranges boundary is random on both sides and is replaced by a
placeholder before the comparison.

Groups (phase 1 of docs/RUST-SERVER.md):
  static    every file under -web (GET), plus the page routes (/ /p /p/), directories, HEAD, 405s, dot-segments,
            traversal, Range / multi-range / If-None-Match / If-Range, CORS
  docs      /llms.txt, /llms-full.txt, the raw .md, the HTML pages and their assets, with HEAD, 405, 304, CORS
  examples  /api/examples, every /examples/<name>.beatscript, the misses
  fallback  Go routes asked of beatscript-serve, which passes them to Go: the answer must be Go's own
  p2        /api/info, /.well-known/btw-site, /.well-known/assetlinks.json, /api/instruments, the JSON-LD context
            routes (+ OPTIONS), GET /api/programs/<cid>[.beatscript] (examples, 304, misses), wrong methods, HEAD, CORS
  p3        PUT /api/programs/<cid> (every refusal, first meta wins, the per-address limit), POST /api/check, /api/fmt,
            /api/share, /api/open, /api/qr: answers, bad JSON, too large, wrong methods, the check-ip-min limit
  cards     P5: /c/, /p/, /card/ (pages and JSON-LD byte for byte; PNGs with headers byte for byte except
            Content-Length and the body compared perceptually, PNG_RULE), PUT /api/shares with and without
            credentials (a btw_session cookie signed with pair.sh's session.secret), wrong methods, HEAD
  auth      /api/auth/{challenge,login,logout,me,token} (P4): the sign-in flows with real Ed25519 and EIP-191
            signatures over the challenge each server issued, every refusal, tokens minted, used and revoked
  misc      near misses of moved routes (trailing slashes, unknown /api/ paths): Go's catch-all answer
  render    WAV renders of the examples (streamed, then cached, then a Range): byte-identical audio, same headers.
            Opus is reported (status, header names), not diffed: beatscript-serve's encoder is not Go's (RUST-SERVER.md §4)

P7 (run each on a fresh bench/pair.sh: both are stateful sequences on the five seeded tokens):
  compose   POST /api/compose: the gate (no user, bad token/cookie, BtwDevice), wrong methods, the body rules (type,
            syntax, unknown fields, trailing data, too large), validation, every fake-model scenario (good, repair,
            unrepairable, oversize, max tokens, blocked, model error), base_cid and current, the user-hour and
            address-day windows, the kill switch through POST /api/admin/compose, admin stats/compose (token
            missing, wrong, lower-case scheme; methods; misses; bodies) and the /api/admin redirect
  mcp       /mcp transport (landing, event-stream GET, DELETE, other methods, content types, bad JSON, batches,
            notifications, responses, sampling responses), JSON-RPC (ids, versions, initialize versions, unparsable
            params, unsupported methods), tools/list byte for byte with cursors, and every tool's success and
            error paths, anonymous and with a token; the hub tools against an empty hub

`known` lists the differences the plan accepts, each with its reason; a case that differs and is not on it fails.
Exit status 1 when any case fails.
"""
import argparse, base64, hashlib, hmac, io, json, os, re, sys, time, zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from parity import http  # noqa: E402

IGNORE = {"date", "connection"}
# a stored program's meta.created is the second it was stored: each server stamps its own (time-dependent)
CREATED = re.compile(rb'"created":"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ"')
BOUNDARY = re.compile(rb"[0-9a-f]{60}")

# path -> reason: differences the plan accepts (RUST-SERVER.md "Accepted differences")
KNOWN = {
    "GET /api/playlists/nope": "playlists are beatscript-serve's own routes; Go has none (RUST-SERVER.md §2), so Go's answer is its catch-all 404",
}


def norm(r):
    hs = []
    for k, v in r["headers"]:
        if k.lower() in IGNORE:
            continue
        if k.lower() == "content-type" and "multipart/byteranges" in v:
            v = BOUNDARY.sub(b"B", v.encode()).decode()
        hs.append((k, v))
    body = BOUNDARY.sub(b"B", r["body"]) if b"multipart" in json.dumps(r["headers"]).encode() else r["body"]
    return r["status"], hs, hashlib.sha256(body).hexdigest(), len(body)


def send(addr, method, path, headers=None, body=None):
    h = dict(headers or {})
    if body is not None:
        h["Content-Length"] = str(len(body))
        return http_body(addr, method, path, h, body)
    return http(addr, method, path, h)


def http_body(addr, method, path, headers, body):
    # parity.http has no request body: the same raw exchange, with one
    import socket
    host, port = addr.rsplit(":", 1)
    s = socket.create_connection((host, int(port)), timeout=60)
    req = f"{method} {path} HTTP/1.1\r\nHost: beatscript.app\r\nConnection: close\r\n"
    for k, v in headers.items():
        req += f"{k}: {v}\r\n"
    s.sendall((req + "\r\n").encode() + body)
    buf = b""
    while True:
        try:
            b = s.recv(1 << 20)
        except (ConnectionResetError, socket.timeout):
            break
        if not b:
            break
        buf += b
    s.close()
    head, _, rest = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    hdrs = [tuple(l.split(": ", 1)) for l in lines[1:] if ": " in l]
    if any(k.lower() == "transfer-encoding" for k, _ in hdrs):
        out = b""
        while True:
            line, sep, rest = rest.partition(b"\r\n")
            n = int(line.split(b";")[0] or b"0", 16) if sep else 0
            if n == 0:
                break
            out, rest = out + rest[:n], rest[n + 2:]
        rest = out
    return {"status": lines[0], "headers": hdrs, "body": rest, "broken": False}


# The share cards' PNGs are drawn by another rasterizer (tiny-skia + ab_glyph, not gg + x/image), so they are compared
# perceptually: same size, mean absolute channel difference at most PNG_MEAN (of 255), and at most PNG_BIG of the
# pixels with any channel off by more than 64. Content-Length (and Content-Range) are the only headers allowed to
# differ: the bytes are another encoder's.
# Measured 2026-11-20 on every card of the group (34 PNGs: 8 examples x 2 sizes, links, generic for each reason): mean
# <= 0.053, big <= 0.00001 (about 7 pixels: glyph edges), worst channel 105. For scale: Go's own generic card drawn
# under another site name (btwsite/testdata/cards/samples/default.png: btw.bitwrap.io in the corner, not beatscript.app)
# is mean 0.30, big 0.0023, and another program's card is mean 9.4, big 0.064. The thresholds sit 3x and 50x above what
# is measured and 2x and 4.7x below a changed word of text.
PNG_MEAN = 0.15
PNG_BIG = 0.0005


def png_diff(a, b):
    from PIL import Image, ImageChops
    ia, ib = Image.open(io.BytesIO(a)).convert("RGB"), Image.open(io.BytesIO(b)).convert("RGB")
    if ia.size != ib.size:
        return {"size": [ia.size, ib.size]}
    d = ImageChops.difference(ia, ib)
    h = d.histogram()
    n = ia.size[0] * ia.size[1]
    mean = sum(i * c for i, c in enumerate(h[0:256])) + sum(i * c for i, c in enumerate(h[256:512])) + sum(i * c for i, c in enumerate(h[512:768]))
    r, g, bb = d.split()
    mx = ImageChops.lighter(ImageChops.lighter(r, g), bb).histogram()
    big = sum(mx[65:])
    return {"mean": round(mean / (3 * n), 4), "big": round(big / n, 5), "max": max(i for i, c in enumerate(mx) if c)}


def png_ok(m):
    return "mean" in m and m["mean"] <= PNG_MEAN and m["big"] <= PNG_BIG




class Run:
    def __init__(self, a):
        self.a = a
        self.results = []

    def case(self, group, method, path, headers=None, body=None, go_addr=None, compare_body=True, png=False, share_created=False, fix=None, label=None):
        """fix: (reason, fn(bytes) -> bytes), a named rule for what is random, time-dependent or accepted (§5): a case
        that differs is KNOWN when both bodies are equal once fn is applied to each (Content-Length to match)."""
        g = send(go_addr or self.a.go, method, path, headers, body)
        r = send(self.a.rs, method, path, headers, body)
        created = None
        if (path.startswith("/api/programs/") or share_created) and g["body"] != r["body"]:
            fg, fr = CREATED.sub(b'"created":"T"', g["body"]), CREATED.sub(b'"created":"T"', r["body"])
            if fg == fr and fg != g["body"]:
                created = "created: each server stamps the second it stored the program (exactly that field differs)"
                g, r = dict(g, body=fg), dict(r, body=fr)
        retry = None
        if " 429 " in g["status"] and g["status"] == r["status"] and g["body"] != r["body"]:
            ra = lambda x: next((v for k, v in x["headers"] if k.lower() == "retry-after"), None)
            a, b = ra(g), ra(r)
            if a and b and a.isdigit() and b.isdigit() and abs(int(a) - int(b)) <= 1 and len(a) == len(b):
                # a sliding window's Retry-After is "seconds until the oldest hit leaves it", truncated: the two servers'
                # windows started a few ms apart (the requests alternate), so the count may be one second apart
                retry = "Retry-After: the window opened a few ms apart on each server (within one second, same length)"
                retry_fix = lambda x: dict(x, headers=[(k, a if k.lower() == "retry-after" else v) for k, v in x["headers"]],
                                     body=x["body"].replace(f'"retry_after":{b}'.encode(), f'"retry_after":{a}'.encode()).replace(f"in {b}s".encode(), f"in {a}s".encode()))
                r = retry_fix(r)
        if png:
            return self.png_case(group, method, path, headers, g, r)
        ng, nr = norm(g), norm(r)
        same = ng[0] == nr[0] and ng[1] == nr[1] and (not compare_body or ng[2] == nr[2])
        diff = []
        if not same:
            if ng[0] != nr[0]:
                diff.append(f"status {ng[0]!r} vs {nr[0]!r}")
            gh, rh = ng[1], nr[1]
            if gh != rh:
                diff.append(f"headers go-only {[x for x in gh if x not in rh]} rs-only {[x for x in rh if x not in gh]}" if set(gh) != set(rh) else "header order")
            if compare_body and ng[2] != nr[2]:
                diff.append(f"body {ng[3]} vs {nr[3]} bytes")
        key = f"{method} {path}" + (f" [{label}]" if label else (f" {headers}" if headers else ""))
        known = KNOWN.get(f"{method} {path}")
        if not same and fix and ng[0] == nr[0]:
            reason, fn = fix
            fg, fr = fn(g["body"]), fn(r["body"])

            def relen(hs, n):
                # the rule applies to the header lines too (a Retry-After); Content-Length follows the body
                out = []
                for k, v in hs:
                    if k.lower() == "content-length":
                        out.append((k, str(n)))
                    else:
                        out.append(tuple(fn(f"{k}: {v}".encode()).decode().split(": ", 1)))
                return out
            if fg == fr and relen(ng[1], len(fg)) == relen(nr[1], len(fr)):
                known = reason
        if not same and path.endswith("/status") and ng[0] == nr[0]:
            # the one accepted difference of the status JSON: beatscript-serve's Opus is its own render version (r2rs), so
            # its URL says so. Equal once that is undone, with Content-Length 2 bytes longer, or it is a failure.
            undone = r["body"].replace(b"\\u0026v=r2rs", b"\\u0026v=r2")
            hs = [(k, str(int(v) - (len(r["body"]) - len(undone)))) if k.lower() == "content-length" else (k, v) for k, v in nr[1]]
            if undone == g["body"] and hs == ng[1]:
                known = "status: the Opus URL carries beatscript-serve's Opus render version (v=r2rs)"
        if same and (created or retry):
            same, known = False, created or retry
        ok = same or bool(known)
        self.results.append({"group": group, "case": key, "same": same, "ok": ok, "known": known, "diff": diff, "bytes": ng[3]})
        if self.a.verbose or not ok:
            print(("SAME " if same else ("KNOWN" if known else "DIFF ")) + f" [{group}] {key}" + ("" if same else "  " + "; ".join(diff)))
        return g, r

    def png_case(self, group, method, path, headers, g, r):
        ng, nr = norm(g), norm(r)
        skip = {"content-length", "content-range"}
        gh = [x for x in ng[1] if x[0].lower() not in skip]
        rh = [x for x in nr[1] if x[0].lower() not in skip]
        diff, metric = [], None
        if ng[0] != nr[0]:
            diff.append(f"status {ng[0]!r} vs {nr[0]!r}")
        if gh != rh:
            diff.append(f"headers go-only {[x for x in gh if x not in rh]} rs-only {[x for x in rh if x not in gh]}")
        identical = not diff and ng[2] == nr[2] and ng[1] == nr[1]
        if not diff and method == "GET" and g["body"][:4] == b"\x89PNG":
            metric = png_diff(g["body"], r["body"])
            if not png_ok(metric):
                diff.append(f"png {metric}")
        elif not diff and ng[2] != nr[2] and not (method == "HEAD" or ng[0].split()[1] == "304"):
            diff.append(f"body {ng[3]} vs {nr[3]} bytes")
        ok = not diff
        known = None if identical or not ok else f"PNG: perceptual (mean <= {PNG_MEAN}, big <= {PNG_BIG}); Content-Length is the encoder's"
        key = f"{method} {path}" + (f" {headers}" if headers else "")
        self.results.append({"group": group, "case": key, "same": identical, "ok": ok, "known": known, "diff": diff, "bytes": ng[3], "png": metric})
        if self.a.verbose or not ok:
            print(("SAME " if identical else ("KNOWN" if ok else "DIFF ")) + f" [{group}] {key}" + (f"  {metric}" if metric else "") + ("" if ok else "  " + "; ".join(diff)))
        return g, r

    # ---------- groups ----------

    def static(self):
        web = self.a.web
        files = []
        for root, dirs, fs in os.walk(web):
            dirs[:] = sorted(d for d in dirs if not d.startswith("."))
            for f in sorted(fs):
                if f.startswith("."):
                    continue
                files.append("/" + os.path.relpath(os.path.join(root, f), web).replace(os.sep, "/"))
        for p in files:
            self.case("static", "GET", p)
        for p in ["/", "/p", "/p/", "/index.html", "/privacy", "/privacy/", "/delete-account", "/delete-account/", "/nope", "/nope/",
                  "/.git/config", "/js/.hidden", "/%2e%2e/etc/passwd", "/..%2fetc/passwd", "/js/../index.html", "//index.html",
                  "/index.html?x=1", "/a%20b", "/css/", "/p/a/b"]:
            self.case("static", "GET", p)
        js = next((f for f in files if f.endswith(".js")), "/favicon.svg")
        g, _ = self.case("static", "GET", js)
        etag = next((v for k, v in g["headers"] if k.lower() == "etag"), '"x"')
        self.case("static", "HEAD", "/")
        self.case("static", "HEAD", js)
        for m in ["POST", "PUT", "DELETE", "OPTIONS", "PATCH"]:
            self.case("static", m, "/")
        self.case("static", "POST", js, body=b"{}", headers={"Content-Type": "application/json"})
        for h in [{"Range": "bytes=0-99"}, {"Range": "bytes=-50"}, {"Range": "bytes=10-"}, {"Range": "bytes=0-9,20-29"},
                  {"Range": "bytes=999999999-"}, {"Range": "chars=0-1"}, {"If-None-Match": etag}, {"If-None-Match": "W/" + etag},
                  {"If-None-Match": '"nope"'}, {"If-Match": '"nope"'}, {"Range": "bytes=0-9", "If-Range": etag},
                  {"Range": "bytes=0-9", "If-Range": '"old"'}, {"Origin": "https://evil.example"}]:
            self.case("static", "GET", js, headers=h)
        g, _ = self.case("static", "GET", "/")
        etag = next((v for k, v in g["headers"] if k.lower() == "etag"), '"x"')
        self.case("static", "GET", "/", headers={"If-None-Match": etag})
        self.case("static", "OPTIONS", "/", headers={"Origin": "https://x.example", "Access-Control-Request-Method": "GET"})

    def docs(self):
        paths = ["/llms.txt", "/llms-full.txt", "/docs/BEATSCRIPT.md", "/docs/cheatsheet.md", "/docs/api.md", "/docs/", "/docs/cheatsheet",
                 "/docs/language", "/docs/api", "/docs/assets/docs.css", "/docs/assets/docs.js", "/docs", "/docs/nope", "/docs/assets/",
                 "/docs/BeatScript", "/docs/api/"]
        for p in paths:
            g, _ = self.case("docs", "GET", p)
            etag = next((v for k, v in g["headers"] if k.lower() == "etag"), None)
            if etag:
                self.case("docs", "GET", p, headers={"If-None-Match": etag})
            self.case("docs", "HEAD", p)
        for p in ["/llms.txt", "/docs/", "/docs/language", "/docs/BEATSCRIPT.md"]:
            self.case("docs", "POST", p, body=b"", headers={})
            self.case("docs", "POST", p.rstrip("/") or "/", body=b"", headers={})
            self.case("docs", "GET", p, headers={"Origin": "https://x.example"})
            self.case("docs", "OPTIONS", p, headers={"Origin": "https://x.example"})
            self.case("docs", "GET", p, headers={"Range": "bytes=0-63"})

    def examples(self):
        self.case("examples", "GET", "/api/examples")
        self.case("examples", "HEAD", "/api/examples")
        self.case("examples", "GET", "/api/examples", headers={"Origin": "https://x.example"})
        self.case("examples", "POST", "/api/examples", body=b"")
        idx = json.load(open(os.path.join(self.a.examples, "index.json")))
        for e in idx:
            self.case("examples", "GET", f"/examples/{e['name']}.beatscript")
        for p in ["/examples/nope.beatscript", "/examples/first-beat", "/examples/", "/examples/a/b.beatscript", "/examples/%22q%22.beatscript"]:
            self.case("examples", "GET", p)
        self.case("examples", "DELETE", f"/examples/{idx[0]['name']}.beatscript")

    def fallback(self):
        # asked of beatscript-serve, answered by Go through it: compare with Go asked directly
        for m, p, h, b in [
            ("GET", "/api/info", None, None), ("GET", "/api/instruments", None, None), ("GET", "/.well-known/btw-site", None, None),
            ("GET", "/context/v1.jsonld", None, None), ("GET", "/context.jsonld", None, None), ("GET", "/api/auth/me", None, None),
            ("POST", "/api/check", {"Content-Type": "application/json"}, json.dumps({"text": "tempo 120\ndeck A: kick x ~ x ~\n"}).encode()),
            ("POST", "/api/fmt", {"Content-Type": "application/json"}, json.dumps({"text": "tempo 120\ndeck A: kick x ~ x ~\n"}).encode()),
            ("GET", "/api/programs/zNotAProgram1234567890abc", None, None), ("GET", "/mcp", None, None), ("GET", "/pair", None, None),
            ("GET", "/card/default.png", None, None), ("GET", "/api/nope", None, None), ("DELETE", "/api/render/zabc", None, None),
            ("GET", "/api/session/x/events", None, None), ("OPTIONS", "/api/programs/zabc", {"Origin": "https://x.example"}, None),
        ]:
            # a PNG body is compared perceptually (the cards group's rule: another rasterizer and encoder)
            self.case("fallback", m, p, h, b, png=p.endswith(".png"))
        idx = json.load(open(os.path.join(self.a.examples, "index.json")))
        info = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        if info:
            self.case("fallback", "GET", f"/p/{info[0]['cid']}")
            self.case("fallback", "GET", f"/api/programs/{info[0]['cid']}")
        _ = idx

    # ---------- P2 + P3 (public.rs) ----------

    def p2(self):
        o = {"Origin": "https://x.example"}
        for p in ["/api/info", "/.well-known/btw-site", "/.well-known/assetlinks.json", "/api/instruments"]:
            self.case("p2", "GET", p)
            self.case("p2", "HEAD", p)
            self.case("p2", "GET", p, headers=o)
            self.case("p2", "OPTIONS", p, headers=dict(o, **{"Access-Control-Request-Method": "GET"}))
            self.case("p2", "OPTIONS", p)
            self.case("p2", "POST", p, body=b"{}")
            self.case("p2", "DELETE", p)
        for p in ["/context/v1.jsonld", "/context.jsonld", "/context"]:
            self.case("p2", "GET", p)
            self.case("p2", "HEAD", p)
            self.case("p2", "GET", p, headers={"If-None-Match": '"btw-context-v1"'})
            self.case("p2", "GET", p, headers={"If-None-Match": '"other"'})
            self.case("p2", "GET", p, headers=o)
            self.case("p2", "OPTIONS", p)
            self.case("p2", "OPTIONS", p, headers=dict(o, **{"Access-Control-Request-Method": "GET"}))
            self.case("p2", "POST", p, body=b"")
            self.case("p2", "PUT", p, body=b"x")
        info = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        for e in info:
            self.case("p2", "GET", f"/api/programs/{e['cid']}")
            self.case("p2", "GET", f"/api/programs/{e['cid']}.beatscript")
        cid = info[0]["cid"]
        for h in [None, o, {"If-None-Match": f'"{cid}"'}, {"If-None-Match": '"zzz"'}, {"If-None-Match": f'W/"{cid}"'}]:
            self.case("p2", "GET", f"/api/programs/{cid}", headers=h)
            self.case("p2", "HEAD", f"/api/programs/{cid}", headers=h)
            self.case("p2", "GET", f"/api/programs/{cid}.beatscript", headers=h)
        for p in ["/api/programs/z" + "2" * 40, "/api/programs/z" + "2" * 40 + ".beatscript", "/api/programs/notacid", "/api/programs/z0OIl" + "2" * 20,
                  "/api/programs/" + cid + ".json", "/api/programs/" + cid + ".beatscript.beatscript", "/api/programs/%7a" + cid[1:]]:
            self.case("p2", "GET", p)
        for m in ["POST", "DELETE", "PATCH"]:
            self.case("p2", m, f"/api/programs/{cid}", body=b"x")
        self.case("p2", "OPTIONS", f"/api/programs/{cid}", headers=dict(o, **{"Access-Control-Request-Method": "PUT"}))
        self.case("p2", "OPTIONS", f"/api/programs/{cid}")

    def p3(self):
        J = {"Content-Type": "application/json"}
        T = {"Content-Type": "text/plain"}
        n = [0]

        def ip():  # a fresh address per case keeps the per-minute check budget out of the way (it has its own case)
            n[0] += 1
            return {"X-Real-IP": f"198.51.{n[0] // 250}.{n[0] % 250 + 1}"}

        def post(path, obj=None, raw=None, h=None):
            b = raw if raw is not None else json.dumps(obj).encode()
            return self.case("p3", "POST", path, headers=dict(J, **ip(), **(h or {})), body=b)

        info = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        ex = [open(os.path.join(self.a.examples, e["name"] + ".beatscript")).read() for e in info]
        good, bad = ex[0], "tempo 120\ndeck A: kick x ~ x ~\nthis is not beatscript\n"
        notpub = "#! visibility: private\n" + good
        bad_json = [b"", b"   ", b"{", b"xyz", b"[]", b'"s"', b"5", b"null", b'{"text":5}', b'{"text":{"a":1}}', b'{"text" 1}',
                    b'{"text":"\\q"}', b'{"text":"a\x01"}', b'{"TEXT":"tempo 120"}', b'{"text":"x"} trailing', b'\xa0', b'{"text":"\xe9\xff"}']
        for path in ["/api/check", "/api/fmt", "/api/share", "/api/open"]:
            for t in [good, bad, "", notpub, ex[-1], "tempo 120\n\u2028<&>\x08\n"]:
                post(path, {"text": t})
            for b in bad_json:
                post(path, raw=b)
            post(path, {"text": good, "same_as": ex[1]})
            post(path, raw=json.dumps({"text": "x" * (128 << 10)}).encode())  # over maxCheckBody
            post(path, raw=b'{"text":"' + b"y" * ((128 << 10) - 12) + b'"}')  # at the limit
            self.case("p3", "GET", path, headers=ip())
            self.case("p3", "HEAD", path, headers=ip())
            self.case("p3", "PUT", path, headers=ip(), body=b"{}")
            self.case("p3", "OPTIONS", path, headers={"Origin": "https://x.example"})
        # open: links, fragments, CIDs, the refusals
        sh = json.loads(send(self.a.go, "POST", "/api/share", J, json.dumps({"text": good}).encode())["body"])
        frag = sh["url"].split("#", 1)[1]
        for u in [sh["url"], sh["social_url"], sh["beats_url"], "#" + frag, frag, info[0]["cid"], "  " + info[0]["cid"] + "\n", "-x", "a\x00b",
                  "https://beatscript.app/p/" + info[0]["cid"], frag[:-3] + "AAA", "nonsense", "x" * ((100 << 10) + 1)]:
            post("/api/open", {"url": u})
        post("/api/open", {"url": "", "text": info[0]["cid"]})
        post("/api/open", {"url": "   "})
        post("/api/open", {"text": sh["url"], "url": info[1]["cid"]})
        # qr
        P = "https://beatscript.app"
        for t in [P + "/p#" + frag, P + "/p/" + info[0]["cid"], P + "/#" + frag, P + "/p#" + frag + "~n", "hello", "",
                  P + "/p#" + frag[:20] + "." + "A" * (2000 - len(P + "/p#") - 21), P + "/p#" + frag[:20] + "." + "A" * 2000,
                  P + "/p/" + "1" * 40, P + "/p#" + "0123456789" * 3 + "." + "ABC" * 100]:
            post("/api/qr", {"text": t})
        for b in bad_json + [json.dumps({"text": "x" * 5000}).encode()]:
            post("/api/qr", raw=b)
        self.case("p3", "GET", "/api/qr", headers=ip())
        # the per-address check budget (check-ip-min 120 a minute, shared by check, fmt, share, open, qr, instruments)
        lim = {"X-Real-IP": "203.0.113.50"}
        for i in range(120):
            for a in (self.a.go, self.a.rs):
                send(a, "POST", ["/api/fmt", "/api/qr", "/api/open"][i % 3], dict(J, **lim), b'{"text":"x"}')
        for path in ["/api/check", "/api/fmt", "/api/share", "/api/open", "/api/qr"]:
            self.case("p3", "POST", path, headers=dict(J, **lim), body=b'{"text":"x"}')
        self.case("p3", "GET", "/api/instruments", headers=lim)
        # PUT /api/programs
        def put(cid, text, h=None, raw=None):
            return self.case("p3", "PUT", f"/api/programs/{cid}", headers=dict(T, **ip(), **(h or {})), body=raw if raw is not None else text.encode())
        cids = [json.loads(send(self.a.go, "POST", "/api/fmt", J, json.dumps({"text": t}).encode())["body"])["cid"] for t in ex]
        fresh = "#! name: contract <test> & co\ntempo 121\ndeck A: kick x ~ x ~ x x ~ ~\n"
        fcid = json.loads(send(self.a.go, "POST", "/api/check", J, json.dumps({"text": fresh}).encode())["body"])["cid"]
        put(fcid, fresh)              # 201, meta from the text
        put(fcid, fresh)              # 200, created false: the first meta
        put(fcid, "#! name: other\n" + fresh)  # same program, another name: the first meta wins
        self.case("p3", "GET", f"/api/programs/{fcid}")
        self.case("p3", "GET", f"/api/programs/{fcid}.beatscript")
        self.case("p3", "GET", f"/api/programs/{fcid}", headers={"If-None-Match": f'"{fcid}"'})
        put(cids[0], ex[0])           # an example's text: stored, but GET answers the example's own meta
        self.case("p3", "GET", f"/api/programs/{cids[0]}")
        put(cids[1], ex[0])           # cid_mismatch
        put("notacid", ex[0])
        put(fcid, fresh, h={"Content-Type": "application/json"})
        put(fcid, fresh, h={"Content-Type": "text/x-JSON-ish"})
        put(fcid, "")
        put(fcid, "   \n\t")
        put(fcid, "", raw=b"tempo 120\n\xff\xfe\n")
        put(fcid, notpub)
        put(fcid, bad)
        put(fcid, "x" * 16385)        # over max-program-bytes: refused by the store
        put(fcid, "x" * 16386)        # past max+1: refused while reading
        put(fcid, "x" * 16384)        # at the limit: checked, does not check
        put(fcid, good)               # a good program under another CID
        self.case("p3", "POST", f"/api/programs/{fcid}", headers=ip(), body=b"x")
        self.case("p3", "OPTIONS", f"/api/programs/{fcid}", headers={"Origin": "https://x.example", "Access-Control-Request-Method": "PUT"})
        # put-ip-hour (60): every PUT that names a CID counts, stored or not
        lim = {"X-Real-IP": "203.0.113.60"}
        for i in range(60):
            for a in (self.a.go, self.a.rs):
                send(a, "PUT", f"/api/programs/{fcid}", dict(T, **lim), b"nope")
        self.case("p3", "PUT", f"/api/programs/{fcid}", headers=dict(T, **lim), body=fresh.encode())
        self.case("p3", "PUT", "/api/programs/bad", headers=dict(T, **lim), body=fresh.encode())

    # ---------- auth (P4) ----------

    # the session key pair.sh gives both servers
    SECRET = bytes.fromhex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    NONCE = re.compile(r"^[A-Za-z0-9_-]{32}$")
    TOKEN = re.compile(r"^btw_[A-Za-z0-9_-]{43}$")
    T3339 = "%Y-%m-%dT%H:%M:%SZ"

    def cookie(self, uid, exp):
        """A btw_session value as Go's makeCookieValue makes it (the servers share the key)."""
        p = json.dumps({"u": uid, "e": exp}, separators=(",", ":")).encode()
        sig = hmac.new(self.SECRET, b"btw-session-v1\n" + p, hashlib.sha256).digest()
        b = lambda x: base64.urlsafe_b64encode(x).rstrip(b"=").decode()
        return b(p) + "." + b(sig)

    def norm_auth(self, r):
        """The exact rules for what is random or the clock's in an auth answer; anything else is left to compare.

        challenge: {"expires":E,"message":M,"nonce":N}: N is 32 base64url characters, E is the issue time + 5 min
          (RFC 3339, UTC, seconds), M is exactly "beatscript.app sign-in\nnonce: N\nissued: I\nexpires: E" with I
          within 5 s of now. N, I and E are replaced by fixed values of the same length.
        token: {"max_per_user":5,"token":T}: T is btw_ + 43 base64url characters; replaced.
        Set-Cookie btw_session=<v>: v verifies under the shared key, its payload is {"u":uid,"e":exp} with exp within
          5 s of now + 7 days, and Expires is exp as http.TimeFormat; v and Expires are replaced (the uid stays,
          in the body and as v's payload: "U<uid>").
        A violated rule is returned as a failure, never normalized."""
        body, hdrs, bad = r["body"], [], []
        now = time.time()
        try:
            j = json.loads(body) if body.startswith(b"{") else None
        except ValueError:
            j = None
        if isinstance(j, dict) and set(j) == {"expires", "message", "nonce"}:
            n, m, e = j["nonce"], j["message"], j["expires"]
            lines = m.split("\n")
            ok = bool(self.NONCE.match(n)) and len(lines) == 4 and lines[0] == "beatscript.app sign-in" and lines[1] == "nonce: " + n
            if ok:
                try:
                    import calendar
                    ti = calendar.timegm(time.strptime(lines[2][len("issued: "):], self.T3339))
                    te = calendar.timegm(time.strptime(e, self.T3339))
                    ok = lines[2].startswith("issued: ") and lines[3] == "expires: " + e and te == ti + 300 and abs(ti - now) < 5
                except ValueError:
                    ok = False
            if not ok:
                bad.append(f"challenge rule broken: {j}")
            body = body.replace(n.encode(), b"N" * 32).replace(e.encode(), b"2000-01-01T00:05:00Z").replace(lines[2][8:].encode() if len(lines) > 2 else b"\0", b"2000-01-01T00:00:00Z")
        if isinstance(j, dict) and "token" in j:
            if not self.TOKEN.match(j["token"]):
                bad.append(f"token rule broken: {j['token']!r}")
            body = body.replace(j["token"].encode(), b"btw_" + b"T" * 43)
        for k, v in r["headers"]:
            if k.lower() == "set-cookie" and v.startswith("btw_session=") and not v.startswith("btw_session=;"):
                val, _, rest = v[len("btw_session="):].partition(";")
                try:
                    p64, s64 = val.split(".")
                    pad = lambda x: x + "=" * (-len(x) % 4)
                    payload = base64.urlsafe_b64decode(pad(p64))
                    good = hmac.compare_digest(base64.urlsafe_b64decode(pad(s64)), hmac.new(self.SECRET, b"btw-session-v1\n" + payload, hashlib.sha256).digest())
                    pl = json.loads(payload)
                    exp = pl["e"]
                    want_exp = time.strftime("%a, %d %b %Y %H:%M:%S GMT", time.gmtime(exp))
                    good = good and abs(exp - (now + 7 * 86400)) < 5 and f" Expires={want_exp};" in rest
                    if not good:
                        bad.append(f"cookie rule broken: {v}")
                    v = "btw_session=U" + pl["u"] + ";" + re.sub(r" Expires=[^;]*;", " Expires=E;", rest)
                except Exception as ex:  # noqa: BLE001
                    bad.append(f"cookie unreadable: {v} ({ex})")
            hdrs.append((k, v))
        return dict(r, body=body, headers=hdrs), bad

    def auth_case(self, method, path, go_req, rs_req=None, name=None):
        """One step sent to each server (`rs_req` when the request differs: a signature over that server's own
        challenge, that server's token or cookie); both answers normalized by `norm_auth`, then compared."""
        rs_req = rs_req or go_req
        g = send(self.a.go, method, path, go_req.get("h"), go_req.get("b"))
        r = send(self.a.rs, method, path, rs_req.get("h"), rs_req.get("b"))
        gn, gbad = self.norm_auth(g)
        rn, rbad = self.norm_auth(r)
        ng, nr = norm(gn), norm(rn)
        same_raw = norm(g) == norm(r)
        same = ng[0] == nr[0] and ng[1] == nr[1] and ng[2] == nr[2]
        diff = gbad + rbad
        if not same:
            if ng[0] != nr[0]:
                diff.append(f"status {ng[0]!r} vs {nr[0]!r}")
            if ng[1] != nr[1]:
                diff.append(f"headers go-only {[x for x in ng[1] if x not in nr[1]]} rs-only {[x for x in nr[1] if x not in ng[1]]}" if set(ng[1]) != set(nr[1]) else "header order")
            if ng[2] != nr[2]:
                diff.append(f"body {ng[3]} vs {nr[3]} bytes: go {gn['body'][:300]!r} rs {rn['body'][:300]!r}")
        ok = same and not gbad and not rbad
        known = None if same_raw or not ok else "auth: random nonce/token and the clock, normalized by norm_auth's rules"
        key = f"{method} {path}" + (f" [{name}]" if name else "")
        self.results.append({"group": "auth", "case": key, "same": same_raw, "ok": ok, "known": known, "diff": diff, "bytes": ng[3]})
        if self.a.verbose or not ok:
            print(("SAME " if same_raw else ("KNOWN" if ok else "DIFF ")) + f" [auth] {key}" + ("" if same_raw else "  " + "; ".join(diff)))
        return g, r

    def auth(self):
        from authsig import EdKey, EthKey
        J = {"Content-Type": "application/json"}
        seq = [0]

        def ip():
            # every step its own client address (behind a loopback peer both servers take X-Real-IP), so the
            # per-address limits only bite where a case asks for them
            seq[0] += 1
            return {"X-Real-IP": f"10.42.{seq[0] // 250}.{seq[0] % 250 + 1}"}

        def H(**kw):
            h = ip()
            h.update(kw.pop("extra", {}))
            return h

        def chal():
            h = H()
            g = json.loads(send(self.a.go, "GET", "/api/auth/challenge", h)["body"])
            r = json.loads(send(self.a.rs, "GET", "/api/auth/challenge", h)["body"])
            return g, r

        def login_req(typ, addr, sig, nonce, extra=None):
            return {"h": H(extra=dict(J, **(extra or {}))), "b": json.dumps({"type": typ, "address": addr, "signature": sig, "nonce": nonce}).encode()}

        def cookie_of(resp):
            for k, v in resp["headers"]:
                if k.lower() == "set-cookie" and v.startswith("btw_session="):
                    return v[len("btw_session="):].split(";")[0]
            return None

        # the challenge
        self.auth_case("GET", "/api/auth/challenge", {"h": H()}, name="shape")
        self.auth_case("HEAD", "/api/auth/challenge", {"h": H()}, name="HEAD")
        self.auth_case("GET", "/api/auth/challenge", {"h": H(extra={"Origin": "https://evil.example"})}, name="any origin may ask")
        self.auth_case("GET", "/api/auth/challenge?x=1", {"h": H()}, name="query")

        # sign in with Ed25519, then with an Ethereum wallet, each over the challenge that server issued
        ed = EdKey(b"contract-ed25519")
        g, r = chal()
        gl, rl = self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, ed.sign(g["message"]), g["nonce"]),
                                login_req("ed25519", ed.pub, ed.sign(r["message"]), r["nonce"]), name="ed25519")
        ed_ck = (cookie_of(gl), cookie_of(rl))
        # the same, replayed: the nonce is gone
        self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, ed.sign(g["message"]), g["nonce"]),
                       login_req("ed25519", ed.pub, ed.sign(r["message"]), r["nonce"]), name="ed25519 replayed")
        eth = EthKey(b"contract-eth")
        g, r = chal()
        gl, rl = self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, eth.personal_sign(g["message"]), g["nonce"]),
                                login_req("eth", eth.address, eth.personal_sign(r["message"]), r["nonce"]), name="eth")
        eth_ck = (cookie_of(gl), cookie_of(rl))
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, eth.personal_sign(g["message"]), g["nonce"]),
                       login_req("eth", eth.address, eth.personal_sign(r["message"]), r["nonce"]), name="eth replayed")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address.upper().replace("0X", "0x"), eth.personal_sign(g["message"]), g["nonce"], {"Origin": "https://beatscript.app"}),
                       login_req("eth", eth.address.upper().replace("0X", "0x"), eth.personal_sign(r["message"]), r["nonce"], {"Origin": "https://beatscript.app"}), name="eth upper-case address, own origin")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("ed25519", "0x" + ed.pub.upper(), "0x" + ed.sign(g["message"]), g["nonce"]),
                       login_req("ed25519", "0x" + ed.pub.upper(), "0x" + ed.sign(r["message"]), r["nonce"]), name="ed25519 0x forms")
        # bad signatures (each burns its nonce: tried again, stale)
        other = EthKey(b"contract-other")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, other.personal_sign(g["message"]), g["nonce"]),
                       login_req("eth", eth.address, other.personal_sign(r["message"]), r["nonce"]), name="eth signed by another key")
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, eth.personal_sign(g["message"]), g["nonce"]),
                       login_req("eth", eth.address, eth.personal_sign(r["message"]), r["nonce"]), name="nonce burned by the failure")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, ed.sign(g["message"] + "x"), g["nonce"]),
                       login_req("ed25519", ed.pub, ed.sign(r["message"] + "x"), r["nonce"]), name="ed25519 other message")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, eth.personal_sign(g["message"]), g["nonce"]),
                       login_req("ed25519", ed.pub, eth.personal_sign(r["message"]), r["nonce"]), name="eth signature as ed25519")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, ed.sign(g["message"]), g["nonce"]),
                       login_req("eth", eth.address, ed.sign(r["message"]), r["nonce"]), name="ed25519 signature as eth")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, "0x" + "ab" * 65, g["nonce"]),
                       login_req("eth", eth.address, "0x" + "ab" * 65, r["nonce"]), name="eth garbage signature")
        g, r = chal()
        self.auth_case("POST", "/api/auth/login", login_req("eth", eth.address, "0x" + "ab" * 64 + "a", g["nonce"]),
                       login_req("eth", eth.address, "0x" + "ab" * 64 + "a", r["nonce"]), name="odd-length hex")
        # what is refused before any challenge is looked at: the same request to both
        sig = eth.personal_sign("x")
        for name, b in [
            ("unknown nonce", {"type": "eth", "address": eth.address, "signature": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("unknown type", {"type": "rsa", "address": eth.address, "signature": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("short address", {"type": "eth", "address": "0x1234", "signature": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("ed25519 short key", {"type": "ed25519", "address": "abcd", "signature": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("ed25519 identity point", {"type": "ed25519", "address": "01" + "00" * 31, "signature": "01" + "00" * 63, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("small order key with sign bit", {"type": "ed25519", "address": "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "signature": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("bad nonce", {"type": "eth", "address": eth.address, "signature": sig, "nonce": "short"}),
            ("nonce with a dot", {"type": "eth", "address": eth.address, "signature": sig, "nonce": "AAAAAAAAAAAAAAAA.AAA"}),
            ("non-hex signature", {"type": "eth", "address": eth.address, "signature": "0x" + "zz" * 65, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("short signature", {"type": "eth", "address": eth.address, "signature": "0xabcd", "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("long signature", {"type": "eth", "address": eth.address, "signature": "ab" * 71, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("folded keys", {"TYPE": "eth", "Address": eth.address, "signaturE": sig, "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA"}),
            ("number field", {"type": 1, "address": eth.address}),
            ("object field", {"nonce": {}}),
            ("null fields", {"type": None, "address": None}),
        ]:
            self.auth_case("POST", "/api/auth/login", {"h": H(extra=J), "b": json.dumps(b).encode()}, name=name)
        for name, b in [("empty body", b""), ("form body", b"nonce=1"), ("cut short", b'{"type":"eth"'), ("array", b"[1,2]"), ("string", b'"x"'),
                        ("number", b"12"), ("null", b"null"), ("true", b"true"), ("bad key", b"{x"), ("bad escape", b'{"type":"\\x"}'),
                        ("control char", b'{"type":"a\nb"}'), ("trailing junk", b'{"type":"rsa"} junk'), ("bad literal", b'{"type":nul}'),
                        ("bad number", b'{"type":-}'), ("bad exponent", b'{"type":1e}'), ("missing colon", b'{"type" "x"}'),
                        ("missing comma", b'{"type":"x" "y"}'), ("array junk", b'{"a":[1 2]}'), ("bad \\u", b'{"a":"\\u12x4"}'),
                        ("quote key", b"{'a':1}"), ("binary", b"\x01\xff"), ("utf-8 junk", b'{"type":"\xc3"}'),
                        ("too large", b'{"nonce":"' + b"a" * 10000 + b'"}'), ("large but complete early", b'{"type":"rsa"}' + b" " * 6000),
                        ("large, syntax error early", b"{x" + b" " * 6000)]:
            self.auth_case("POST", "/api/auth/login", {"h": H(extra=J), "b": b}, name=name)

        # cross-site: refused before the nonce is spent
        g, r = chal()
        for name, extra in [("evil origin", {"Origin": "https://evil.example", "Content-Type": "text/plain"}),
                            ("null origin", {"Origin": "null"}), ("legacy host origin", {"Origin": "https://btw.bitwrap.io"}),
                            ("other port", {"Origin": "https://beatscript.app:8443"}), ("junk origin", {"Origin": "::::"}),
                            ("cross-site fetch", {"Sec-Fetch-Site": "cross-site"}), ("same-site fetch", {"Sec-Fetch-Site": "same-site"})]:
            self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, ed.sign(g["message"]), g["nonce"], extra),
                           login_req("ed25519", ed.pub, ed.sign(r["message"]), r["nonce"], extra), name="login " + name)
            for m, p in [("POST", "/api/auth/logout"), ("POST", "/api/auth/token"), ("DELETE", "/api/auth/token")]:
                self.auth_case(m, p, {"h": H(extra=dict(extra, Cookie="btw_session=" + ed_ck[0]))}, {"h": H(extra=dict(extra, Cookie="btw_session=" + ed_ck[1]))}, name=name)
        self.auth_case("POST", "/api/auth/login", login_req("ed25519", ed.pub, ed.sign(g["message"]), g["nonce"], {"Origin": "http://beatscript.app", "Sec-Fetch-Site": "same-origin"}),
                       login_req("ed25519", ed.pub, ed.sign(r["message"]), r["nonce"], {"Origin": "http://beatscript.app", "Sec-Fetch-Site": "same-origin"}), name="after the refusals, the same nonce signs in")

        # me
        ed_uid, eth_uid = "ed25519:" + ed.pub, "eip155:" + eth.address
        now = int(time.time())
        self.auth_case("GET", "/api/auth/me", {"h": H()}, name="anonymous")
        self.auth_case("HEAD", "/api/auth/me", {"h": H()}, name="HEAD anonymous")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Cookie": "btw_session=" + ed_ck[0]})}, {"h": H(extra={"Cookie": "btw_session=" + ed_ck[1]})}, name="ed25519 cookie each server made")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Cookie": "btw_session=" + eth_ck[1]})}, {"h": H(extra={"Cookie": "btw_session=" + eth_ck[0]})}, name="eth cookie the other server made")
        good = self.cookie(ed_uid, now + 3600)
        for name, c in [("cookie", good), ("expired cookie", self.cookie(ed_uid, now - 1)), ("forged cookie", good[:-2] + ("AA" if not good.endswith("AA") else "BB")),
                        ("malformed uid", self.cookie("root", now + 3600)), ("upper-case uid", self.cookie("eip155:0x" + "A" * 40, now + 3600)),
                        ("no dot", "abc"), ("empty cookie", ""), ("two cookies", good + "; btw_session=x.y")]:
            self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Cookie": "a=1; btw_session=" + c})}, name=name)
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Authorization": "Bearer btw_" + "A" * 43})}, name="unknown token")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Authorization": "Bearer some-admin-token"})}, name="foreign bearer")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Authorization": "BtwDevice x.1.y"})}, name="device scheme is not a user here")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Cookie": "btw_session=x.y", "Authorization": "Bearer btw_" + "A" * 43})}, name="bad cookie, bad bearer")

        # tokens: minted with the cookie, used as a bearer, revoked
        ck = self.cookie(eth_uid, now + 3600)
        self.auth_case("POST", "/api/auth/token", {"h": H()}, name="anonymous")
        gt, rt = self.auth_case("POST", "/api/auth/token", {"h": H(extra={"Cookie": "btw_session=" + ck})}, name="mint")
        tok = (json.loads(gt["body"])["token"], json.loads(rt["body"])["token"])
        bearer = lambda t, **x: {"h": H(extra=dict({"Authorization": "Bearer " + t}, **x))}
        self.auth_case("GET", "/api/auth/me", bearer(tok[0]), bearer(tok[1]), name="bearer")
        self.auth_case("GET", "/api/auth/me", bearer(tok[0], Cookie="btw_session=x.y"), bearer(tok[1], Cookie="btw_session=x.y"), name="bearer beats a bad cookie")
        self.auth_case("GET", "/api/auth/me", {"h": H(extra={"Authorization": "bearer " + tok[0]})}, {"h": H(extra={"Authorization": "bearer " + tok[1]})}, name="lower-case scheme")
        self.auth_case("POST", "/api/auth/token", bearer(tok[0]), bearer(tok[1]), name="a token cannot mint a token")
        self.auth_case("GET", "/api/auth/me", bearer(tok[1]), bearer(tok[0]), name="the other server's token")
        for _ in range(5):
            self.auth_case("POST", "/api/auth/token", {"h": H(extra={"Cookie": "btw_session=" + ck})}, name="mint more (5 max, oldest dropped)")
        self.auth_case("GET", "/api/auth/me", bearer(tok[0]), bearer(tok[1]), name="the oldest token was dropped")
        gt, rt = self.auth_case("POST", "/api/auth/token", {"h": H(extra={"Cookie": "btw_session=" + ck})}, name="mint")
        tok = (json.loads(gt["body"])["token"], json.loads(rt["body"])["token"])
        self.auth_case("DELETE", "/api/auth/token", bearer(tok[0]), bearer(tok[1]), name="revoke with the bearer")
        self.auth_case("GET", "/api/auth/me", bearer(tok[0]), bearer(tok[1]), name="revoked at once")
        self.auth_case("DELETE", "/api/auth/token", {"h": H(extra={"Cookie": "btw_session=" + ck})}, name="revoke none left")
        self.auth_case("DELETE", "/api/auth/token", {"h": H()}, name="anonymous")
        self.auth_case("DELETE", "/api/auth/token", {"h": H(extra={"Cookie": "btw_session=" + self.cookie(eth_uid, now - 5)})}, name="expired cookie")

        # logout
        self.auth_case("POST", "/api/auth/logout", {"h": H()}, name="anonymous")
        self.auth_case("POST", "/api/auth/logout", {"h": H(extra={"Cookie": "btw_session=" + ck, "Origin": "https://beatscript.app"})}, name="signed in")

        # methods Go has no pattern for: the static site's answer
        for m, p in [("GET", "/api/auth/login"), ("PUT", "/api/auth/token"), ("GET", "/api/auth/token"), ("POST", "/api/auth/me"),
                     ("DELETE", "/api/auth/challenge"), ("OPTIONS", "/api/auth/login"), ("HEAD", "/api/auth/logout"),
                     ("PATCH", "/api/auth/logout")]:
            # (/api/auth/, /api/auth/nope, /api/auth/me/ match no Go pattern: they are dispatch's, not this group's)
            self.auth_case(m, p, {"h": H()}, name="wrong method or path")
        self.auth_case("OPTIONS", "/api/auth/token", {"h": H(extra={"Origin": "https://x.example", "Access-Control-Request-Method": "POST"})}, name="preflight")

        # the per-address limits: challenge 30/min, then 429 with Retry-After
        h = H()
        for i in range(31):
            self.auth_case("GET", "/api/auth/challenge", {"h": h}, name=f"limit {i + 1}/30")
        h = H()
        for i in range(21):
            self.auth_case("POST", "/api/auth/login", {"h": dict(h, **J), "b": b"{}"}, name=f"login limit {i + 1}/20")
        h = H()
        for i in range(11):
            self.auth_case("POST", "/api/auth/token", {"h": h}, name=f"token limit {i + 1}/10")

    def misc(self):
        # near misses of moved routes: no Go pattern takes them, so Go's catch-all (the static site) answers
        for m, p in [("GET", "/api/info/"), ("GET", "/api/programs/"), ("GET", "/api/auth/"), ("GET", "/api/auth/nope"), ("GET", "/api/auth/me/"),
                     ("GET", "/api/nope"), ("POST", "/api/nope"), ("GET", "/api/check"), ("GET", "/api/shares/"), ("GET", "/api/instruments/x"),
                     ("GET", "/.well-known/"), ("GET", "/.well-known/nope"), ("GET", "/context/"), ("GET", "/context/v2.jsonld"), ("GET", "/api/"),
                     ("HEAD", "/api/nope"), ("DELETE", "/api/programs/zabc"), ("GET", "/api/render/"), ("GET", "/api/render/zabc/x"),
                     ("GET", "/api/playlists/nope"), ("OPTIONS", "/api/nope"),
                     ("GET", "/api/devices/"), ("GET", "/api/device/"), ("GET", "/api/session/"), ("GET", "/api/session/x"), ("GET", "/api/pair/"),
                     ("GET", "/api/pair/a/b"), ("GET", "/api/admin"), ("GET", "/api/compose/"), ("GET", "/pair/"), ("GET", "/mcp/x/y"), ("PUT", "/pair")]:
            self.case("misc", m, p)

    def render(self):
        info = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        for e in info[: self.a.render_limit]:
            cid = e["cid"]
            for q in ["format=wav&v=r2", "format=wav&sr=48000&v=r2"]:
                self.case("render", "GET", f"/api/render/{cid}?{q}")  # cold on both (each its own data dir): streamed
                self.case("render", "GET", f"/api/render/{cid}?{q}")  # cached
                self.case("render", "GET", f"/api/render/{cid}?{q}", headers={"Range": "bytes=44-1043"})
                self.case("render", "HEAD", f"/api/render/{cid}?{q}")
            self.case("render", "GET", f"/api/render/{cid}/status")
            g = send(self.a.go, "GET", f"/api/render/{cid}?format=opus&v=r2")
            r1 = send(self.a.rs, "GET", f"/api/render/{cid}?format=opus&v=r2")
            r2 = send(self.a.rs, "GET", f"/api/render/{cid}?format=opus&v=r2")  # cached: the same bytes
            self.results.append({"group": "render", "case": f"GET /api/render/{cid}?format=opus (report only)", "same": False, "ok": True,
                                 "known": "Opus: another encoder, another render version (r2rs)",
                                 "diff": [f"go {len(g['body'])} B, rs {len(r1['body'])} B, rs streamed == rs cached: {r1['body'] == r2['body']}"],
                                 "bytes": len(g["body"]), "rs_reproducible": r1["body"] == r2["body"]})
            if r1["body"] != r2["body"]:
                self.results[-1]["ok"] = False
                print(f"FAIL [render] opus {cid}: beatscript-serve's streamed and cached Opus differ")

    def hub(self):
        # P6: pairing, devices, sessions, the queue's page side, SSE (bench/contract_hub.py)
        import contract_hub
        contract_hub.run(self)

    # ---------- P7: compose and admin ----------

    def compose(self):
        J = {"Content-Type": "application/json"}

        def tok(n=1):
            return {"Authorization": f"Bearer {TOKEN % n}", "Content-Type": "application/json"}

        def post(label, body, headers=None, raw=None, fix=None, path="/api/compose"):
            b = raw if raw is not None else json.dumps(body).encode()
            return self.case("compose", "POST", path, headers if headers is not None else tok(), b, fix=fix, label=label)

        admin = {"Authorization": "Bearer admin-test-token", "Content-Type": "application/json"}
        # the gate: no user is a 401 before anything else, whatever the request
        post("no user", {"prompt": "x"}, J)
        post("no user, no body", None, {}, raw=b"")
        post("unknown token", {"prompt": "x"}, {"Authorization": "Bearer btw_nope", "Content-Type": "application/json"})
        post("not a beatscript token", {"prompt": "x"}, {"Authorization": "Bearer abc", "Content-Type": "application/json"})
        post("device scheme", {"prompt": "x"}, {"Authorization": "BtwDevice a.1.b", "Content-Type": "application/json"})
        post("bad cookie", {"prompt": "x"}, {"Cookie": "btw_session=abc.def", "Content-Type": "application/json"})
        for m in ["GET", "HEAD", "PUT", "DELETE", "OPTIONS"]:
            self.case("compose", m, "/api/compose", tok())
        # the body
        post("no content type", {"prompt": "x"}, {"Authorization": f"Bearer {TOKEN % 1}"})
        post("text/plain", {"prompt": "x"}, {"Authorization": f"Bearer {TOKEN % 1}", "Content-Type": "text/plain"})
        post("json with charset", {"prompt": ""}, {"Authorization": f"Bearer {TOKEN % 1}", "Content-Type": "application/json; charset=utf-8"})
        for label, raw in [("empty", b""), ("spaces", b"  \n"), ("syntax", b'{"prompt":x}'), ("syntax late", b'{"prompt":"a",, }'), ("cut off", b'{"prompt":"a'),
                           ("number", b"12"), ("array", b"[1]"), ("null", b"null"), ("unknown field", b'{"prompt":"a","nope":1}'),
                           ("wrong type", b'{"prompt":5}'), ("bpm string", b'{"prompt":"a","bpm":"120"}'), ("two values", b'{"prompt":"a"} {}'),
                           ("trailing junk", b'{"prompt":"a"}x'), ("length bool", b'{"prompt":"a","length":true}'), ("case-insensitive", b'{"PROMPT":"  "}'),
                           ("too large", b'{"prompt":"' + b"a" * 120000 + b'"}'), ("control char", b'{"prompt":"a\x01"}')]:
            post(label, None, raw=raw)
        for label, body in [("prompt empty", {"prompt": "   "}), ("prompt too long", {"prompt": "x" * 801}), ("genre", {"prompt": "a", "genre": "a\"b"}),
                            ("bpm low", {"prompt": "a", "bpm": 30}), ("bpm high", {"prompt": "a", "bpm": 301}), ("key", {"prompt": "a", "key": "h#"}),
                            ("length word", {"prompt": "a", "length": "huge"}), ("length 3", {"prompt": "a", "length": 3}), ("length 999", {"prompt": "a", "length": "999"}),
                            ("base_cid bad", {"prompt": "a", "base_cid": "nope"}), ("base_cid unknown", {"prompt": "a", "base_cid": "z" + "1" * 40}),
                            ("current private", {"prompt": "a", "current": "#! visibility: private\ntempo 120\ndeck A: kick x ~\n"}),
                            ("current broken", {"prompt": "a", "current": "tempo 120\ndeck A: kik x ~\n"}),
                            ("current too large", {"prompt": "a", "current": "#" * 16385})]:
            post(label, body)
        ex = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        rv = (RV_REASON, lambda b: b.replace(b"v=r2rs", b"v=r2"))
        # the fake model's scenarios (compose/fixtures), each on both servers in the same order
        post("good", {"prompt": "dark techno, 128", "genre": "techno", "bpm": 128, "key": "Am", "length": "short"}, fix=rv)
        post("good again (stored once)", {"prompt": "dark techno, 128"}, fix=rv)
        post("repair", {"prompt": "fake-repair acid"}, fix=rv)
        post("unrepairable", {"prompt": "fake-unrepairable"})
        post("oversize", {"prompt": "fake-oversize"}, fix=rv)
        post("max tokens", {"prompt": "fake-max-tokens", "length": 64}, fix=rv)
        post("blocked", {"prompt": "fake-blocked"})
        post("model error", {"prompt": "fake-error"})
        if ex:
            post("base_cid example", {"prompt": "make it faster", "base_cid": ex[0]["cid"]}, fix=rv)
            txt = send(self.a.go, "GET", f"/examples/{ex[0]['name']}.beatscript")["body"].decode()
            post("current", {"prompt": "add a snare", "current": txt}, fix=rv)
        # the per-user hour window (10): user 1 has been admitted 10 times above
        post("user hour limit", {"prompt": "one more"}, fix=(RETRY_REASON, retry_norm))
        # the kill switch through the admin endpoint
        self.case("compose", "POST", "/api/admin/compose", admin, b'{"enabled":false}', fix=(DATA_REASON, data_norm), label="switch off")
        post("switched off", {"prompt": "x"}, tok(2))
        self.case("compose", "GET", "/api/admin/stats", admin, fix=(STATS_REASON, stats_norm), label="stats, off")
        self.case("compose", "POST", "/api/admin/compose", admin, b'{"enabled":true}', label="switch on")
        post("switched on again", {"prompt": "fake-blocked"}, tok(2))
        # the per-address day window (40): users 2 to 5 fill it from this address
        n = 0
        for u in [2, 3, 4, 5]:
            for i in range(10):
                g, _ = post(f"fill address window u{u} #{i}", {"prompt": "fake-blocked"}, tok(u), fix=(RETRY_REASON, retry_norm))
                n += 1
        post("address day limit", {"prompt": "fake-blocked"}, tok(5), fix=(RETRY_REASON, retry_norm))
        # admin: token missing, wrong, lower-case scheme; methods; misses; bodies
        for label, h in [("no token", {}), ("wrong token", {"Authorization": "Bearer nope"}), ("lowercase scheme", {"Authorization": "bearer admin-test-token"}),
                         ("empty bearer", {"Authorization": "Bearer "}), ("good", {"Authorization": "Bearer admin-test-token"})]:
            self.case("compose", "GET", "/api/admin/stats", h, fix=(STATS_REASON, stats_norm), label=f"stats {label}")
        self.case("compose", "HEAD", "/api/admin/stats", admin, label="HEAD stats")
        self.case("compose", "GET", "/api/admin/stats/", admin, fix=(STATS_REASON, stats_norm), label="stats slash")
        self.case("compose", "POST", "/api/admin/stats", admin, b"{}")
        self.case("compose", "GET", "/api/admin/compose", admin)
        self.case("compose", "GET", "/api/admin/nope", admin)
        self.case("compose", "GET", "/api/admin/", admin)
        self.case("compose", "GET", "/api/admin/nope")
        for m in ["GET", "HEAD", "POST"]:
            self.case("compose", m, "/api/admin")
        self.case("compose", "GET", "/api/admin?x=1")
        for label, raw, h in [("no type", b'{"enabled":true}', {"Authorization": "Bearer admin-test-token"}), ("empty", b"", admin), ("not bool", b'{"enabled":1}', admin),
                              ("unknown", b'{"enabled":true,"x":1}', admin), ("missing", b"{}", admin), ("two", b'{"enabled":true} {}', admin),
                              ("too large", b'{"enabled":true' + b" " * 2000 + b"}", admin), ("syntax", b"{", admin)]:
            self.case("compose", "POST", "/api/admin/compose", h, raw, label=f"admin body {label}")

    # ---------- P7: MCP ----------

    def rpc(self, label, msg, token=None, headers=None, raw=None, fix=None, path="/mcp"):
        h = {"Content-Type": "application/json"} if headers is None else dict(headers)
        if token:
            h["Authorization"] = f"Bearer {TOKEN % token}"
        b = raw if raw is not None else json.dumps(msg).encode()
        return self.case("mcp", "POST", path, h, b, fix=fix, label=label)

    def call(self, label, name, args=None, token=None, fix=None, rid=1):
        p = {"name": name}
        if args is not None:
            p["arguments"] = args
        return self.rpc(f"tools/call {name}: {label}" + (" (token)" if token else ""), {"jsonrpc": "2.0", "id": rid, "method": "tools/call", "params": p}, token, fix=fix)

    def mcp(self):
        # transport
        for label, h in [("landing", {}), ("landing html", {"Accept": "text/html"}), ("event stream", {"Accept": "text/event-stream"})]:
            self.case("mcp", "GET", "/mcp", h, label=label)
        self.case("mcp", "GET", "/mcp/")
        self.case("mcp", "GET", "/mcp/x")
        for m in ["DELETE", "PUT", "PATCH", "HEAD", "OPTIONS"]:
            self.case("mcp", m, "/mcp")
        self.case("mcp", "OPTIONS", "/mcp", {"Origin": "https://x.example", "Access-Control-Request-Method": "POST"})
        ping = {"jsonrpc": "2.0", "id": 1, "method": "ping"}
        self.rpc("no content type", ping, headers={})
        self.rpc("text/plain", ping, headers={"Content-Type": "text/plain"})
        self.rpc("json; charset", ping, headers={"Content-Type": "application/json; charset=utf-8"})
        self.rpc("ping on /mcp/", ping, path="/mcp/")
        self.rpc("ping on /mcp/x", ping, path="/mcp/x")
        for label, raw in [("empty", b""), ("not json", b"xx"), ("batch", b'[{"jsonrpc":"2.0","id":1,"method":"ping"}]'), ("null", b"null"),
                           ("string", b'"x"'), ("method number", b'{"jsonrpc":"2.0","id":1,"method":5}'), ("jsonrpc number", b'{"jsonrpc":2,"id":1,"method":"ping"}'),
                           ("too large", b'{"jsonrpc":"2.0","id":1,"method":"ping","x":"' + b"a" * 200000 + b'"}'), ("trailing", b'{"jsonrpc":"2.0","id":1,"method":"ping"} x')]:
            self.rpc(label, None, raw=raw)
        for label, msg in [("wrong version", {"jsonrpc": "1.0", "id": 3, "method": "ping"}), ("no version", {"id": 3, "method": "ping"}),
                           ("string id", {"jsonrpc": "2.0", "id": "a-1", "method": "ping"}), ("float id", {"jsonrpc": "2.0", "id": 2.5, "method": "ping"}),
                           ("exp id", {"jsonrpc": "2.0", "id": 1e3, "method": "ping"}), ("bool id", {"jsonrpc": "2.0", "id": True, "method": "ping"}),
                           ("object id", {"jsonrpc": "2.0", "id": {"b": 1, "a": 2}, "method": "ping"}), ("null id", {"jsonrpc": "2.0", "id": None, "method": "ping"}),
                           ("notification", {"jsonrpc": "2.0", "method": "notifications/initialized"}),
                           ("notification, bad params", {"jsonrpc": "2.0", "method": "notifications/initialized", "params": [1]}),
                           ("notification, cancelled", {"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
                           ("ping response", {"jsonrpc": "2.0", "id": 7, "result": {}}), ("a response", {"jsonrpc": "2.0", "id": 7, "result": {"x": 1}}),
                           ("sampling response, no session", {"jsonrpc": "2.0", "id": 7, "result": {"x": 1}, "method": ""}),
                           ("error response", {"jsonrpc": "2.0", "id": 7, "error": {"code": 1, "message": "m"}}),
                           ("unknown method", {"jsonrpc": "2.0", "id": 4, "method": "foo/bar"}), ("no method", {"jsonrpc": "2.0", "id": 4}),
                           ("resources/list", {"jsonrpc": "2.0", "id": 4, "method": "resources/list"}), ("resources/read", {"jsonrpc": "2.0", "id": 4, "method": "resources/read"}),
                           ("prompts/list", {"jsonrpc": "2.0", "id": 4, "method": "prompts/list"}), ("logging/setLevel", {"jsonrpc": "2.0", "id": 4, "method": "logging/setLevel"}),
                           ("tasks/list", {"jsonrpc": "2.0", "id": 4, "method": "tasks/list"}), ("completion/complete", {"jsonrpc": "2.0", "id": 4, "method": "completion/complete"}),
                           ("ping params array", {"jsonrpc": "2.0", "id": 4, "method": "ping", "params": [1]})]:
            self.rpc(label, msg)
        self.rpc("sampling response, session", {"jsonrpc": "2.0", "id": 7, "result": {"x": 1}}, headers={"Content-Type": "application/json", "Mcp-Session-Id": "abc"})
        self.rpc("sampling response, session, string id", {"jsonrpc": "2.0", "id": "x", "result": {"x": 1}}, headers={"Content-Type": "application/json", "Mcp-Session-Id": "abc"})
        # initialize
        for label, params in [("2025-06-18", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
                              ("latest", {"protocolVersion": "2025-11-25"}), ("unknown version", {"protocolVersion": "1999-01-01"}), ("no version", {}),
                              ("params number", 5), ("version number", {"protocolVersion": 5}), ("clientInfo string", {"clientInfo": "x"}),
                              ("clientInfo name number", {"clientInfo": {"name": 1}})]:
            self.rpc(f"initialize {label}", {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params})
        self.rpc("initialize, no params", {"jsonrpc": "2.0", "id": 1, "method": "initialize"})
        self.rpc("initialize (token)", {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}, token=1)
        # tools/list: byte for byte (btwsite/testdata/mcp/tools-list.json pins it in Go)
        self.rpc("tools/list", {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        self.rpc("tools/list (token)", {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, token=2)
        for c in ["Y2hlY2s=", "cmVuZGVy", "enp6", "", "!!", "abc", "YQ"]:
            self.rpc(f"tools/list cursor {c!r}", {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"cursor": c}})
        self.rpc("tools/list cursor number", {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"cursor": 5}})
        # tools/call framing
        self.rpc("call unknown tool", {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "compose", "arguments": {"prompt": "x"}}})
        self.rpc("call name number", {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": 5}})
        self.rpc("call no params", {"jsonrpc": "2.0", "id": 3, "method": "tools/call"})
        self.call("arguments array", "check", [1])
        self.call("arguments string", "check", "x")
        self.call("arguments null", "fmt", None)
        prog = "tempo 120\nkey am\ndeck A: kick x ~ x ~ | hihat ~ x ~ x\n"
        broken = "tempo 120\ndeck A: kik x ~ x ~\n"
        ex = json.loads(send(self.a.go, "GET", "/api/examples")["body"])
        ecid, ename = (ex[0]["cid"], ex[0]["name"]) if ex else ("z" + "1" * 40, "nope")
        unknown = "z" + "2" * 45
        # check
        self.call("ok", "check", {"text": prog})
        self.call("errors", "check", {"text": broken})
        self.call("missing text", "check", {})
        self.call("blank text", "check", {"text": "  \n"})
        self.call("text number", "check", {"text": 5})
        self.call("too large", "check", {"text": "#" * 16385})
        self.call("NUL", "check", {"text": "tempo 1\x00"})
        self.call("unknown arg", "check", {"text": prog, "sesion": "x"})
        self.call("same_as text", "check", {"text": prog, "same_as": "tempo 120\nkey am\ndeck A: kick x ~ x ~ | hihat ~ x ~ ~\n"})
        self.call("same_as itself", "check", {"text": prog, "same_as": prog})
        self.call("same_as example cid", "check", {"text": prog, "same_as": ecid})
        self.call("same_as short link", "check", {"text": prog, "same_as": f"https://beatscript.app/p/{ecid}"})
        self.call("same_as unknown cid", "check", {"text": prog, "same_as": unknown})
        self.call("same_as empty", "check", {"text": prog, "same_as": " "})
        # fmt
        self.call("ok", "fmt", {"text": "tempo   120\n# hi\ndeck A:  kick x ~\n"})
        self.call("errors", "fmt", {"text": broken})
        self.call("extra arg", "fmt", {"text": prog, "x": 1})
        # share (anonymous: the self-sealing links only)
        self.call("anonymous", "share", {"text": prog})
        self.call("errors", "share", {"text": broken})
        self.call("private", "share", {"text": "#! visibility: private\n" + prog})
        self.call("restricted", "share", {"text": "use restricted\n" + prog})
        self.call("bad license", "share", {"text": prog, "license": "not a licence"})
        self.call("bad forked_from", "share", {"text": prog, "forked_from": "x"})
        self.call("credits not a list", "share", {"text": prog, "credits": "me"})
        self.call("credits too many", "share", {"text": prog, "credits": ["a"] * 9})
        self.call("credit two lines", "share", {"text": prog, "credits": ["a\nb"]})
        self.call("name two lines", "share", {"text": prog, "name": "a\nb"})
        cr = (CREATED_REASON, created_norm)
        self.call("stored", "share", {"text": prog, "license": "CC-BY-4.0", "credits": ["me"], "name": "Beat", "forked_from": ecid}, token=1, fix=cr)
        self.call("stored again (first rights stay)", "share", {"text": prog}, token=1, fix=cr)
        g = send(self.a.go, "POST", "/mcp", {"Content-Type": "application/json"}, json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "share", "arguments": {"text": prog}}}).encode())
        out = json.loads(json.loads(g["body"])["result"]["content"][0]["text"])
        pcid, url, social = out.get("cid", ""), out.get("url", ""), out.get("social_url", "")
        # open
        self.call("self-sealing link", "open", {"url": url})
        self.call("social link", "open", {"url": social})
        self.call("fragment", "open", {"url": url[url.find("#"):]})
        self.call("stored cid", "open", {"url": pcid}, fix=cr)
        self.call("short link", "open", {"url": f"https://beatscript.app/p/{pcid}"}, fix=cr)
        self.call("example cid", "open", {"url": ecid})
        self.call("unknown cid", "open", {"url": unknown})
        self.call("tampered", "open", {"url": url[:-6] + ("AAAAAA" if not url.endswith("AAAAAA") else "BBBBBB")})
        self.call("garbage", "open", {"url": "not a link"})
        self.call("empty", "open", {"url": "  "})
        self.call("dash", "open", {"url": "-x"})
        # render, record (the Opus URL carries beatscript-serve's render version, §5.1)
        rv = (RV_REASON, lambda b: b.replace(b"v=r2rs", b"v=r2"))
        self.call("text", "render", {"text": prog}, fix=rv)
        self.call("text wav", "render", {"text": prog, "format": "wav"})
        self.call("cid", "render", {"cid": pcid}, fix=rv)
        self.call("example", "render", {"example": ename}, fix=rv)
        self.call("unknown example", "render", {"example": "nope-nope"})
        self.call("bad example name", "render", {"example": "../x"})
        self.call("unknown cid", "render", {"cid": unknown})
        self.call("bad cid", "render", {"cid": "abc"})
        self.call("two of them", "render", {"text": prog, "cid": pcid})
        self.call("none", "render", {})
        self.call("bad format", "render", {"text": prog, "format": "mp3"})
        self.call("errors", "render", {"text": broken})
        self.call("text", "record", {"text": prog})
        self.call("example", "record", {"example": ename})
        self.call("format not taken", "record", {"text": prog, "format": "wav"})
        # list_examples, instruments
        self.call("all", "list_examples", {})
        self.call("one", "list_examples", {"name": ename})
        self.call("unknown", "list_examples", {"name": "nope-nope"})
        self.call("bad name", "list_examples", {"name": "../x"})
        self.call("ok", "instruments", {})
        self.call("cached", "instruments")
        self.call("extra arg", "instruments", {"x": 1})
        # the tools that need a token: refused without one, then (with one) the argument rules and an empty hub
        for t in ["eval", "load", "get", "stop", "devices", "queue"]:
            self.call("no token", t, {})
            self.rpc(f"tools/call {t}: unknown token", {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": t, "arguments": {}}},
                     headers={"Content-Type": "application/json", "Authorization": "Bearer btw_nope"})
        T = 3
        self.call("no tab", "eval", {"text": prog}, token=T)
        self.call("errors", "eval", {"text": broken}, token=T)
        self.call("restricted", "eval", {"text": "use restricted\n" + prog}, token=T)
        self.call("play not bool", "eval", {"text": prog, "play": "yes"}, token=T)
        self.call("unknown session", "eval", {"text": prog, "session": "abcdef123"}, token=T)
        self.call("bad session", "eval", {"text": prog, "session": "x"}, token=T)
        self.call("session too long", "eval", {"text": prog, "session": "a" * 65}, token=T)
        self.call("unknown device", "eval", {"text": prog, "device": "devabc123"}, token=T)
        self.call("bad device", "eval", {"text": prog, "device": "x!"}, token=T)
        self.call("session and device", "eval", {"text": prog, "session": "abcdef123", "device": "devabc123"}, token=T)
        self.call("missing text", "eval", {"play": True}, token=T)
        self.call("cid, no tab", "load", {"cid": pcid, "play": True}, token=T)
        self.call("example, no tab", "load", {"example": ename}, token=T)
        self.call("url, no tab", "load", {"url": url}, token=T)
        self.call("short url, no tab", "load", {"url": f"https://beatscript.app/p/{pcid}"}, token=T)
        self.call("unknown cid", "load", {"cid": unknown}, token=T)
        self.call("bad cid", "load", {"cid": "abc"}, token=T)
        self.call("unknown example", "load", {"example": "nope-nope"}, token=T)
        self.call("tampered url", "load", {"url": url[:-6] + "AAAAAA"}, token=T)
        self.call("two of them", "load", {"cid": pcid, "example": ename}, token=T)
        self.call("none", "load", {}, token=T)
        self.call("no tab", "get", {}, token=T)
        self.call("unknown session", "get", {"session": "abcdef123"}, token=T)
        self.call("unknown device", "get", {"device": "devabc123"}, token=T)
        self.call("extra arg", "get", {"x": 1}, token=T)
        self.call("no tab", "stop", {}, token=T)
        self.call("unknown session", "stop", {"session": "abcdef123"}, token=T)
        self.call("none paired", "devices", {}, token=T)
        self.call("extra arg", "devices", {"x": 1}, token=T)
        self.call("list, no tab", "queue", {"action": "list"}, token=T)
        self.call("bad action", "queue", {"action": "play"}, token=T)
        self.call("no action", "queue", {}, token=T)
        self.call("items not for list", "queue", {"action": "list", "items": []}, token=T)
        self.call("item not for add", "queue", {"action": "add", "item": "q1"}, token=T)
        self.call("remove without item", "queue", {"action": "remove"}, token=T)
        self.call("add, no items", "queue", {"action": "add", "items": []}, token=T)
        self.call("add, too many", "queue", {"action": "add", "items": [{"cid": pcid}] * 21}, token=T)
        self.call("add, not objects", "queue", {"action": "add", "items": ["x"]}, token=T)
        self.call("add, two of them", "queue", {"action": "add", "items": [{"cid": pcid, "example": ename}]}, token=T)
        self.call("add, bars and minutes", "queue", {"action": "add", "items": [{"cid": pcid, "bars": 8, "minutes": 1}]}, token=T)
        self.call("add, bars fraction", "queue", {"action": "add", "items": [{"cid": pcid, "bars": 1.5}]}, token=T)
        self.call("add, bars range", "queue", {"action": "add", "items": [{"cid": pcid, "bars": 2000}]}, token=T)
        self.call("add, minutes string", "queue", {"action": "add", "items": [{"cid": pcid, "minutes": "1"}]}, token=T)
        self.call("add, title two lines", "queue", {"action": "add", "items": [{"cid": pcid, "title": "a\nb"}]}, token=T)
        self.call("add, unknown cid", "queue", {"action": "add", "items": [{"cid": unknown}]}, token=T)
        self.call("add, unknown example", "queue", {"action": "add", "items": [{"example": "nope-nope"}]}, token=T)
        self.call("add, item extra key", "queue", {"action": "add", "items": [{"cid": pcid, "x": 1}]}, token=T)
        self.call("add, no tab", "queue", {"action": "add", "items": [{"cid": pcid, "bars": 8}, {"example": ename, "minutes": 0.5}, {"url": url, "title": "t"}]}, token=T)
        self.call("skip, no tab", "queue", {"action": "skip"}, token=T)
        self.call("clear, unknown session", "queue", {"action": "clear", "session": "abcdef123"}, token=T)


# ---------- P7: the rules for what may differ, each exact ----------


def cookie(uid, secret_hex="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", exp=4102444800):
    # auth.Service's btw_session value, as `playlists/auth.rs` make_cookie_for_test signs it
    b64 = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
    p = json.dumps({"u": uid, "e": exp}, separators=(",", ":")).encode()
    sig = hmac.new(bytes.fromhex(secret_hex), b"btw-session-v1\n" + p, hashlib.sha256).digest()
    return f"btw_session={b64(p)}.{b64(sig)}"


def raw_pack(text):
    # any deflate is a valid link: Python's own bytes, not Go's (the page must then keep the link's packed text)
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return base64.urlsafe_b64encode(c.compress(text.encode()) + c.flush()).rstrip(b"=").decode()


def go_check(a, text):
    return json.loads(send(a.go, "POST", "/api/check", {"Content-Type": "application/json"}, json.dumps({"text": text}).encode())["body"])


def cards(self):
    a = self.a
    info = json.loads(send(a.go, "GET", "/api/examples")["body"])
    jh = {"Content-Type": "application/json"}
    ck = {"Cookie": cookie("eip155:0x" + "ab" * 20)}
    ckj = dict(jh, **ck)
    # pages, JSON-LD and cards of every example
    for e in info:
        cid = e["cid"]
        for p in [f"/c/{cid}", f"/p/{cid}", f"/c/{cid}.jsonld", f"/p/{cid}.jsonld", f"/c/{cid}/"]:
            self.case("cards", "GET", p)
        self.case("cards", "HEAD", f"/c/{cid}")
        self.case("cards", "HEAD", f"/c/{cid}.jsonld")
        self.case("cards", "GET", f"/c/{cid}.jsonld", headers={"Origin": "https://x.example"})
        for p in [f"/card/{cid}.png", f"/card/{cid}.square.png"]:
            g, _ = self.case("cards", "GET", p, png=True)
            etag = next((v for k, v in g["headers"] if k.lower() == "etag"), '"x"')
            self.case("cards", "GET", p, headers={"If-None-Match": etag}, png=True)
            self.case("cards", "HEAD", p, png=True)
    # a link: the program in the path. The example's canonical text, packed by Go (as its page links it) and by Python
    ex = info[0]
    canon = go_check(a, send(a.go, "GET", f"/examples/{ex['name']}.beatscript")["body"].decode())["canonical"]
    page = send(a.go, "GET", f"/c/{ex['cid']}")["body"].decode()
    gopacked = re.search(r'href="/#z[^."]+\.([^"]+)"', page).group(1)
    for pk in [gopacked, raw_pack(canon), raw_pack(canon + "\n# a comment\n")]:
        for p in [f"/c/{ex['cid']}.{pk}", f"/c/{ex['cid']}.{pk}.jsonld"]:
            self.case("cards", "GET", p)
        self.case("cards", "GET", f"/card/{ex['cid']}.{pk}.png", png=True)
    # a program nobody stored, carried in the link
    text = "#! name: link only\n# a link that is not stored\ntempo 97\nkey dm\nlet k = x ~ ~ ~ x ~ x ~\nsection verse 4 bars {\n  deck A: kick k | bass d1 ~ f1 ~ inst=sub-bass\n}\narrange verse verse\n"
    rep = go_check(a, text)
    cid2, pk2 = rep["cid"], raw_pack(rep["canonical"])
    for p in [f"/c/{cid2}.{pk2}", f"/c/{cid2}.{pk2}.jsonld", f"/c/{cid2}", f"/p/{cid2}", f"/c/{cid2}.jsonld"]:
        self.case("cards", "GET", p)
    for p in [f"/card/{cid2}.{pk2}.png", f"/card/{cid2}.{pk2}.square.png", f"/card/{cid2}.png"]:
        self.case("cards", "GET", p, png=True)
    # a link whose text is another program (mismatch), garbage, a restricted program
    self.case("cards", "GET", f"/c/{ex['cid']}.{pk2}")
    self.case("cards", "GET", f"/card/{ex['cid']}.{pk2}.png", png=True)
    for p in [f"/c/{ex['cid']}.garbage!", f"/c/{ex['cid']}.{pk2}~notes!", f"/card/{ex['cid']}.AAAA.png"]:
        self.case("cards", "GET", p, png=p.endswith(".png"))
    rtext = "use restricted\ntempo 120\ndeck A: kick x ~ x ~\n"
    rr = go_check(a, rtext)
    if rr.get("ok") and rr.get("cid"):
        rp = raw_pack(rr["canonical"])
        self.case("cards", "GET", f"/c/{rr['cid']}.{rp}")
        self.case("cards", "GET", f"/card/{rr['cid']}.{rp}.png", png=True)
    # misses and odd paths
    unknown = "z4EBG9j" + "A" * 44
    for p in [f"/c/{unknown}", f"/p/{unknown}", f"/c/{unknown}.jsonld", "/c/zabc", "/c/", "/c", "/p/zabc", f"/p/{unknown}.x", "/c/zabc.jsonld",
              "/c//x", "/c/%7a" + unknown[1:], "/card", "/card/", "/card/nope", "/card/x.txt", "/card/go.js", "/p/a/b"]:
        self.case("cards", "GET", p)
    for p in [f"/card/{unknown}.png", "/card/zabc.png", "/card/default.png", "/card/default.square.png", "/card/.png", "/card/a/b.png"]:
        self.case("cards", "GET", p, png=True)
    g, _ = self.case("cards", "GET", "/card/default.png", headers={"Origin": "https://x.example"}, png=True)
    etag = next((v for k, v in g["headers"] if k.lower() == "etag"), '"x"')
    self.case("cards", "GET", "/card/default.png", headers={"If-None-Match": etag}, png=True)
    self.case("cards", "HEAD", "/card/default.png", png=True)
    self.case("cards", "HEAD", "/card/go.js")
    self.case("cards", "HEAD", "/c")
    self.case("cards", "OPTIONS", "/card/default.png", headers={"Origin": "https://x.example", "Access-Control-Request-Method": "GET"})
    for m, p in [("POST", f"/c/{ex['cid']}"), ("POST", "/card/default.png"), ("PUT", f"/p/{ex['cid']}"), ("POST", "/c"), ("DELETE", "/card"), ("POST", "/c/")]:
        self.case("cards", m, p, body=b"")
    # shares: refusals before and after the credentials
    sp = f"/api/shares/{cid2}"
    good = json.dumps({"text": text, "license": "CC-BY-4.0", "credits": ["the tests"]}).encode()
    for h in [jh, dict(jh, Cookie="btw_session=bad.sig"), dict(jh, Authorization="Bearer btw_nope"), dict(jh, Authorization="Bearer x"),
              dict(jh, Authorization="BtwDevice a.b.c"), dict(jh, Authorization="BtwDevice dev.1.AAAA")]:
        self.case("cards", "PUT", sp, headers=h, body=good)
    self.case("cards", "PUT", "/api/shares/bad", headers=ckj, body=good)
    self.case("cards", "PUT", "/api/shares/", headers=ckj, body=good)
    self.case("cards", "GET", sp)
    self.case("cards", "POST", sp, headers=ckj, body=good)
    for b in [b"", b"nope", b"[]", b"null", b'{"text": ""}', b'{"text": "x", "extra": 1}', b'{"text": 5}', json.dumps({"text": text, "visibility": "private"}).encode(),
              json.dumps({"text": text, "license": "not a licence"}).encode(), json.dumps({"text": text, "forked_from": "nope"}).encode(),
              json.dumps({"text": text, "credits": ["a"] * 9}).encode(), json.dumps({"text": text, "credits": [""]}).encode(),
              json.dumps({"text": text, "name": "x" * 81}).encode(), json.dumps({"text": "tempo 120\ndeck A: nope(\n"}).encode(),
              json.dumps({"text": "#! visibility: private\n" + text}).encode(), json.dumps({"text": rtext}).encode(),
              json.dumps({"text": "x" * 20000}).encode(), json.dumps({"text": "tempo 120\ndeck A: kick x ~ x ~\n"}).encode(), b"x" * 110000]:
        self.case("cards", "PUT", sp, headers=ckj, body=b)
    # stored, then stored again (first write wins), then its page and card by CID alone
    self.case("cards", "PUT", sp, headers=ckj, body=good, share_created=True)
    self.case("cards", "PUT", sp, headers=ckj, body=json.dumps({"text": rep["canonical"], "TEXT": rep["canonical"]}).encode(), share_created=True)
    for p in [f"/c/{cid2}", f"/p/{cid2}", f"/c/{cid2}.jsonld", f"/c/{cid2}.{pk2}"]:
        self.case("cards", "GET", p)
    for p in [f"/card/{cid2}.png", f"/card/{cid2}.square.png"]:
        self.case("cards", "GET", p, png=True)
    # a second account shares an example's text: the example's own name and licence win on its page
    ck2 = dict(jh, Cookie=cookie("eip155:0x" + "cd" * 20))
    self.case("cards", "PUT", f"/api/shares/{ex['cid']}", headers=ck2, body=json.dumps({"text": canon, "name": "not the example"}).encode(), share_created=True)
    self.case("cards", "GET", f"/c/{ex['cid']}")


Run.cards = cards


TOKEN = "beatscript_contracttesttoken000000000000000%d"  # bench/pair.sh seeds users 1..5
RV_REASON = "render URL: beatscript-serve's Opus is its own render version (v=r2rs, RUST-SERVER.md §5.1); equal once undone"
RETRY_REASON = "rate limit: Retry-After and retry_after are the seconds left in a sliding window, measured at two instants"
DATA_REASON = "kill switch reason: it names each server's own data dir (<data>/compose.off)"
STATS_REASON = "admin stats: reset_in_seconds is the time to UTC midnight, measured at two instants"
CREATED_REASON = "stored meta: created is the second the program was stored on each server"


def retry_norm(b):
    b = re.sub(rb"^Retry-After: \d+$", b"Retry-After: N", b)
    return re.sub(rb'"retry_after":\d+', b'"retry_after":N', re.sub(rb"try again in [^\"]+?\.?\"", b'try again in T"', b))


def data_norm(b):
    return re.sub(rb"the flag file [^ ]+/compose.off", b"the flag file <data>/compose.off", b)


def stats_norm(b):
    return data_norm(re.sub(rb'"reset_in_seconds":\d+', b'"reset_in_seconds":N', b))


def created_norm(b):
    return re.sub(rb'(\\?"created\\?":\\?")[0-9T:Z-]+', rb"\1<t>", b)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--go", required=True)
    ap.add_argument("--rs", required=True)
    ap.add_argument("--web", required=True)
    ap.add_argument("--examples", required=True)
    ap.add_argument("--groups", default="static,docs,examples,fallback,render")
    ap.add_argument("--render-limit", type=int, default=3)
    ap.add_argument("--known", help="JSON file: {\"METHOD path\": reason}")
    ap.add_argument("--out")
    ap.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args()
    if a.known:
        KNOWN.update(json.load(open(a.known)))
    run = Run(a)
    t0 = time.time()
    for g in a.groups.split(","):
        getattr(run, g)()
    by = {}
    for r in run.results:
        s = by.setdefault(r["group"], {"cases": 0, "identical": 0, "known": 0, "failed": 0})
        s["cases"] += 1
        s["identical"] += r["same"]
        s["known"] += (not r["same"]) and r["ok"]
        s["failed"] += not r["ok"]
    print(json.dumps({"groups": by, "seconds": round(time.time() - t0, 1)}, indent=1))
    if a.out:
        json.dump({"groups": by, "results": run.results}, open(a.out, "w"), indent=1)
    sys.exit(1 if any(not r["ok"] for r in run.results) else 0)


if __name__ == "__main__":
    main()
