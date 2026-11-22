"""The `hub` group of contract.py (P6): pairing, devices, the device and session SSE streams, sessions, the queue's
page routes, auth, wrong methods and the cheap limits, driven as identical sequences on both servers.

Both servers run from bench/pair.sh: the same session.secret (so one cookie signs in on both), the same site.json
(site id `site-contract-test-0001`), empty devices. A phone is an Ed25519 key (Python `cryptography`); it signs
claims and requests exactly as djandroid does (btwsite/PAIRING.md), once per server (a signature is single use).

Normalization: only what is random or read off a clock, each by a named rule, everything else byte for byte.
  ids       a value the server made at random (session id, pair id, QR token, device id, get request id) is
            replaced by a label naming which one it is (`<session:1>`), on each side from its own answers. Both
            are 22 base64url characters, so no length changes.
  time      an RFC 3339 time (`2026-11-20T12:00:00Z`, with a fraction for a report's `ts`) becomes `<time>`.
            A report's ts has a variable-length fraction, so after this rule Content-Length is checked against
            each side's own body and then compared as the normalized length.
  qr        the `qr` data URL of POST /api/pair draws a random link, so it cannot be compared across servers. Each
            side's PNG must decode (CRCs checked) to a 320x320 two-colour image of the same QR version (module
            count, from the finder pattern); the field is then `<qr …>`, and as the PNG's length follows the link's
            content, the framing headers are each checked against their own body. Byte parity of the PNG itself
            with Go (go-qrcode + image/png + compress/flate) is pinned by the unit tests in src/hub/qr.rs: 542
            module-matrix goldens and four pairing links' PNG bytes printed by Go.
  retry     a rate limit's seconds (`Retry-After`, `retry_after`, "try again in Ns") may differ by one when the
            two answers straddle a second; equal within one, then normalized.
Accepted differences are recorded in KNOWN_HUB with their reason.
"""
import base64, hashlib, hmac, json, re, socket, struct, threading, time, zlib

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

SECRET = bytes.fromhex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
SITE = "site-contract-test-0001"
ORIGIN = "https://beatscript.app"
TIME = re.compile(rb"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z")
PROG = "tempo 120\ndeck A: kick x ~ x ~\n"

# case -> reason
KNOWN_HUB = {}


def b64u(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def cookie(uid, exp=2_000_000_000):
    p = json.dumps({"u": uid, "e": exp}, separators=(",", ":")).encode()
    sig = hmac.new(SECRET, b"btw-session-v1\n" + p, hashlib.sha256).digest()
    return f"btw_session={b64u(p)}.{b64u(sig)}"


ALICE = "ed25519:" + "a1" * 32
BOB = "ed25519:" + "b2" * 32
CA, CB = cookie(ALICE), cookie(BOB)


def raw(addr, method, path, headers=None, body=None, timeout=30):
    host, port = addr.rsplit(":", 1)
    s = socket.create_connection((host, int(port)), timeout=timeout)
    req = f"{method} {path} HTTP/1.1\r\nHost: beatscript.app\r\nConnection: close\r\n"
    h = dict(headers or {})
    if body is not None:
        h["Content-Length"] = str(len(body))
    for k, v in h.items():
        req += f"{k}: {v}\r\n"
    s.sendall((req + "\r\n").encode() + (body or b""))
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
    return parse(buf, method)


def dechunk(rest):
    out = b""
    while True:
        line, sep, rest = rest.partition(b"\r\n")
        if not sep:
            return out, False
        n = int(line.split(b";")[0] or b"0", 16)
        if n == 0:
            return out, True
        if len(rest) < n + 2:
            return out + rest[:n], False
        out, rest = out + rest[:n], rest[n + 2:]


def parse(buf, method="GET"):
    head, _, rest = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    hdrs = [tuple(l.split(": ", 1)) for l in lines[1:] if ": " in l]
    if any(k.lower() == "transfer-encoding" for k, _ in hdrs) and method != "HEAD":
        rest, _ = dechunk(rest)
    return {"status": lines[0], "headers": hdrs, "body": rest}


class SSE:
    """An open event stream, read in the background."""

    def __init__(self, addr, path, headers=None):
        host, port = addr.rsplit(":", 1)
        self.s = socket.create_connection((host, int(port)), timeout=60)
        req = f"GET {path} HTTP/1.1\r\nHost: beatscript.app\r\nConnection: close\r\n"
        for k, v in (headers or {}).items():
            req += f"{k}: {v}\r\n"
        self.s.sendall((req + "\r\n").encode())
        self.buf, self.closed = b"", False
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            try:
                b = self.s.recv(65536)
            except OSError:
                b = b""
            with self.lock:
                if not b:
                    self.closed = True
                    return
                self.buf += b

    def snapshot(self):
        with self.lock:
            head, sep, rest = self.buf.partition(b"\r\n\r\n")
            if not sep:
                return None, b"", self.closed
            r = parse(head + b"\r\n\r\n", "HEAD")
            chunked = any(k.lower() == "transfer-encoding" for k, _ in r["headers"])
            body, done = dechunk(rest) if chunked else (rest, self.closed)
            return r, body, self.closed or done

    def frames(self, n, timeout=4.0):
        """The head, the body up to n frames (blank-line separated blocks), and whether the stream ended."""
        t = time.time() + timeout
        while True:
            r, body, ended = self.snapshot()
            if r is not None and (ended or body.count(b"\n\n") >= n or not r["status"].endswith("200 OK")) or time.time() > t:
                return r, body, ended
            time.sleep(0.02)

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


class Phone:
    def __init__(self, seed):
        self.k = Ed25519PrivateKey.from_private_bytes(bytes([seed]) * 32)
        self.key = b64u(self.k.public_key().public_bytes_raw())
        self.dev = {}  # side -> device id

    def sign(self, msg):
        return b64u(self.k.sign(msg.encode()))

    def claim(self, pair_id, token, name="Pixel 9", ts=None, proto="btw-pair/2", host="beatscript.app", site=SITE, key=None):
        ts = int(time.time() * 1000) if ts is None else ts
        key = key or self.key
        th = hashlib.sha256(token.encode()).hexdigest()
        if proto == "btw-pair/2":
            msg = f"btw-pair/2 claim\nsite: {site}\npair: {pair_id}\ntoken: {th}\nkey: {key}\nname: {name}\nts: {ts}"
        else:
            msg = f"btw-pair/1 claim\nhost: {host}\npair: {pair_id}\ntoken: {th}\nkey: {key}\nname: {name}\nts: {ts}"
        return json.dumps({"proto": proto, "pair_id": pair_id, "token": token, "key": key, "name": name, "ts": ts, "sig": self.sign(msg)}).encode()

    def auth(self, side, method, path, body=b"", ts=None, v=2, dev=None):
        ts = int(time.time() * 1000) if ts is None else ts
        dev = dev or self.dev[side]
        bh = hashlib.sha256(body).hexdigest()
        if v == 2:
            msg = f"btw-pair/2 request\nsite: {SITE}\ndevice: {dev}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {bh}"
        else:
            msg = f"btw-pair/1 request\nhost: btw.bitwrap.io\ndevice: {dev}\nmethod: {method}\npath: {path}\nts: {ts}\nbody: {bh}"
        return f"BtwDevice {dev}.{ts}.{self.sign(msg)}"


def png_info(b):
    """(width, height, palette size, modules across) of a go-qrcode PNG: bilevel, quiet zone of 4 modules."""
    assert b[:8] == b"\x89PNG\r\n\x1a\n"
    i, idat, ihdr, plte = 8, b"", None, None
    while i < len(b):
        n = struct.unpack(">I", b[i:i + 4])[0]
        t, d = b[i + 4:i + 8], b[i + 8:i + 8 + n]
        if zlib.crc32(t + d) != struct.unpack(">I", b[i + 8 + n:i + 12 + n])[0]:
            raise ValueError("bad crc")
        if t == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", d)
        elif t == b"PLTE":
            plte = d
        elif t == b"IDAT":
            idat += d
        i += 12 + n
    w, h, depth, ctype = ihdr[0], ihdr[1], ihdr[2], ihdr[3]
    data = zlib.decompress(idat)
    bpp = max(1, depth // 8)
    stride = (w * depth + 7) // 8
    rows, prev, j = [], bytearray(stride), 0
    for _ in range(h):
        f, line = data[j], bytearray(data[j + 1:j + 1 + stride])
        j += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            up = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + up) & 255
            elif f == 3:
                line[x] = (line[x] + (a + up) // 2) & 255
            elif f == 4:
                p = a + up - c
                pa, pb, pc = abs(p - a), abs(p - up), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else up if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    def px(x, y):
        if depth == 8:
            return rows[y][x]
        return (rows[y][x * depth // 8] >> (8 - depth - (x * depth % 8))) & ((1 << depth) - 1)
    # the finder pattern's top-left corner: the first dark pixel on the diagonal; its width is 7 modules
    dark = plte[3:6] == b"\x00\x00\x00"  # index 1 black
    isdark = (lambda v: v == 1) if dark else (lambda v: v == 0)
    k = next(t for t in range(w) if isdark(px(t, t)))
    run = next(t for t in range(k, w) if not isdark(px(t, k))) - k
    return w, h, len(plte) // 3, round(w * 7 / run)


class Hub:
    def __init__(self, run):
        self.r = run
        self.a = run.a
        self.maps = {"go": {}, "rs": {}}
        self.counts = {}
        self.ipn = 0

    def ip(self):
        self.ipn += 1
        return f"198.51.100.{self.ipn}"

    def reg(self, label, g, r):
        """A value each server made at random, by what it is."""
        if not g or not r:
            return
        n = self.counts.get(label, 0) + 1
        self.counts[label] = n
        self.maps["go"][g] = f"<{label}:{n}>"
        self.maps["rs"][r] = f"<{label}:{n}>"

    def norm(self, side, resp, rules):
        body = resp["body"]
        applied = []
        if "qr" in rules:
            m = re.search(rb'"qr":"data:image/png;base64,([^"]+)"', body)
            if m:
                info = png_info(base64.b64decode(m.group(1)))
                body = body.replace(m.group(1), f"<qr {info}>".encode())
                applied.append("qr")
        for v, lab in sorted(self.maps[side].items(), key=lambda x: -len(x[0])):
            body = body.replace(v.encode(), lab.encode())
        hs = []
        if "time" in rules and TIME.search(body):
            body = TIME.sub(b"<time>", body)
            applied.append("time")
        if "retry" in rules:
            body = re.sub(rb'"retry_after":\d+', b'"retry_after":N', body)
            body = re.sub(rb"try again in \d+s", b"try again in Ns", body)
            applied.append("retry")
        framing = "qr" in applied or "time" in applied
        for k, v in resp["headers"]:
            kl = k.lower()
            if kl in ("date", "connection"):
                continue
            for val, lab in self.maps[side].items():
                v = v.replace(val, lab)
            if kl == "retry-after" and "retry" in rules:
                v = "N"
            if framing and kl == "content-length":
                if int(v) != len(resp["body"]):
                    v = f"WRONG {v}"
                else:
                    v = "<len>"
            if framing and kl == "transfer-encoding" and "qr" in applied:
                v, k = "<len>", "Content-Length"
            hs.append((k, v))
        return resp["status"], hs, body, applied

    def compare(self, name, g, r, rules=(), known=None):
        ng, nr = self.norm("go", g, rules), self.norm("rs", r, rules)
        same = ng[:3] == nr[:3]
        diff = []
        if not same:
            if ng[0] != nr[0]:
                diff.append(f"status {ng[0]!r} vs {nr[0]!r}")
            if ng[1] != nr[1]:
                diff.append(f"headers go-only {[x for x in ng[1] if x not in nr[1]]} rs-only {[x for x in nr[1] if x not in ng[1]]}")
            if ng[2] != nr[2]:
                i = next((k for k in range(min(len(ng[2]), len(nr[2]))) if ng[2][k] != nr[2][k]), min(len(ng[2]), len(nr[2])))
                diff.append(f"body ({len(ng[2])} vs {len(nr[2])} B, first difference at {i}) go …{ng[2][max(0, i - 60):i + 120]!r} rs …{nr[2][max(0, i - 60):i + 120]!r}")
        known = known or KNOWN_HUB.get(name)
        if not same and b"v=r2rs" in r["body"]:
            # the one accepted difference of a render URL (RUST-SERVER.md §5.1): btwserve's Opus is its own render
            # version, r2rs. Exact once that is undone, Content-Length two bytes per URL shorter.
            fix = r["body"].replace(b"v=r2rs", b"v=r2")
            r2 = dict(r, body=fix, headers=[(k, str(int(v) - (len(r["body"]) - len(fix)))) if k.lower() == "content-length" else (k, v) for k, v in r["headers"]])
            if self.norm("go", g, rules)[:3] == self.norm("rs", r2, rules)[:3]:
                known = "render URL: btwserve's Opus is its own render version (v=r2rs, RUST-SERVER.md §5.1); equal once undone"
        ok = same or bool(known)
        rules_used = sorted(set(ng[3]) | set(nr[3]))
        self.r.results.append({"group": "hub", "case": name, "same": same, "ok": ok, "known": None if same else known, "diff": diff,
                               "bytes": len(g["body"]), "rules": rules_used})
        if self.a.verbose or not ok:
            print(("SAME " if same else ("KNOWN" if known else "DIFF ")) + f" [hub] {name}" + (f" (rules: {','.join(rules_used)})" if rules_used else "")
                  + ("" if same else "  " + "; ".join(diff)))
        return g, r

    def both(self, name, method, path, headers=None, body=None, rules=(), hdr_fn=None, known=None):
        """One request to each server. hdr_fn(side) gives per-side headers (a phone's signature)."""
        hg = dict(headers or {}, **(hdr_fn("go") if hdr_fn else {}))
        hr = dict(headers or {}, **(hdr_fn("rs") if hdr_fn else {}))
        g = raw(self.a.go, method, path, hg, body)
        r = raw(self.a.rs, method, path, hr, body)
        return self.compare(name or f"{method} {path}", g, r, rules, known)

    def path2(self, tmpl, label_vals):
        """A path with each side's own random value in it: {'go': path, 'rs': path}."""
        return {s: tmpl.format(*[v[s] for v in label_vals]) for s in ("go", "rs")}

    def split(self, name, method, paths, headers=None, bodies=None, rules=(), hdr_fn=None, known=None):
        """Like both, but the path (and body) differ per side (they carry that side's random ids)."""
        hg = dict(headers or {}, **(hdr_fn("go", paths["go"]) if hdr_fn else {}))
        hr = dict(headers or {}, **(hdr_fn("rs", paths["rs"]) if hdr_fn else {}))
        bg = bodies["go"] if bodies else None
        br = bodies["rs"] if bodies else None
        g = raw(self.a.go, method, paths["go"], hg, bg)
        r = raw(self.a.rs, method, paths["rs"], hr, br)
        return self.compare(name, g, r, rules, known)

    def sse_pair(self, name, paths, headers_fn, n):
        sg = SSE(self.a.go, paths["go"], headers_fn("go", paths["go"]))
        sr = SSE(self.a.rs, paths["rs"], headers_fn("rs", paths["rs"]))
        return sg, sr

    def sse_compare(self, name, sg, sr, n, rules=("time",)):
        rg, bg, eg = sg.frames(n)
        rr, br, er = sr.frames(n)
        g = {"status": rg["status"] if rg else "(none)", "headers": rg["headers"] if rg else [], "body": bg + (b"<end>" if eg else b"")}
        r = {"status": rr["status"] if rr else "(none)", "headers": rr["headers"] if rr else [], "body": br + (b"<end>" if er else b"")}
        return self.compare(name, g, r, rules)

    # ---------- the sequences ----------

    def run(self):
        self.methods_and_auth()
        self.pairing()
        self.sessions()
        self.mcp_drive()
        self.limits()

    def methods_and_auth(self):
        b = self.both
        b(None, "GET", "/pair")
        b(None, "HEAD", "/pair")
        b(None, "POST", "/pair", body=b"")
        for m, p in [("GET", "/api/pair"), ("PUT", "/api/pair"), ("POST", "/api/pair/xyz"), ("POST", "/api/devices"), ("GET", "/api/devices/x"),
                     ("GET", "/api/devices/x/push"), ("PUT", "/api/device"), ("POST", "/api/device/events"), ("GET", "/api/device/report"),
                     ("GET", "/api/device/compose"), ("GET", "/api/session"), ("DELETE", "/api/session/x/events"), ("GET", "/api/session/x/report"),
                     ("GET", "/api/session/x/reply"), ("POST", "/api/session/x/queue"), ("GET", "/api/session/x/queue/next"),
                     ("OPTIONS", "/api/session"), ("HEAD", "/api/session")]:
            b(f"{m} {p} (wrong method)", m, p, body=b"" if m not in ("GET", "HEAD", "OPTIONS") else None)
        # auth missing / bad
        for name, h in [("none", {}), ("bad cookie", {"Cookie": "btw_session=junk.junk"}), ("expired cookie", {"Cookie": cookie(ALICE, 1_000_000_000)}),
                        ("unknown token", {"Authorization": "Bearer btw_nope0000000000000000000000000000"}),
                        ("a phone's credential", {"Authorization": "BtwDevice x.1.y"}), ("not ours", {"Authorization": "Bearer abc"})]:
            b(f"POST /api/pair (auth {name})", "POST", "/api/pair", h, b"")
            b(f"GET /api/devices (auth {name})", "GET", "/api/devices", h)
            b(f"GET /api/pair/x (auth {name})", "GET", "/api/pair/x", h)
            b(f"DELETE /api/devices/x (auth {name})", "DELETE", "/api/devices/x", h)
            b(f"POST /api/devices/x/push (auth {name})", "POST", "/api/devices/x/push", h, b"{}")
        b("HEAD /api/devices (no auth)", "HEAD", "/api/devices")
        b("GET /api/pair/claim (signed in)", "GET", "/api/pair/claim", {"Cookie": CA})
        b("GET /api/pair/nope (signed in)", "GET", "/api/pair/nope", {"Cookie": CA})
        b("GET /api/devices (signed in, none)", "GET", "/api/devices", {"Cookie": CA})
        b("DELETE /api/devices/nope (signed in)", "DELETE", "/api/devices/nope", {"Cookie": CA})
        b("POST /api/devices/nope/push", "POST", "/api/devices/nope/push", {"Cookie": CA}, b'{"type":"stop"}')
        for o in ["https://evil.example", "null", "http://beatscript.app.evil"]:
            b(f"POST /api/pair (Origin {o})", "POST", "/api/pair", {"Cookie": CA, "Origin": o}, b"")
        b("POST /api/pair (Sec-Fetch-Site cross-site)", "POST", "/api/pair", {"Cookie": CA, "Sec-Fetch-Site": "cross-site"}, b"")
        b("DELETE /api/devices/x (cross-site)", "DELETE", "/api/devices/x", {"Cookie": CA, "Origin": "https://evil.example"})
        b("POST /api/devices/x/push (cross-site)", "POST", "/api/devices/x/push", {"Cookie": CA, "Origin": "https://evil.example"}, b"{}")
        # the phone routes without a phone
        for m, p in [("GET", "/api/device"), ("DELETE", "/api/device"), ("GET", "/api/device/events"), ("POST", "/api/device/report"),
                     ("POST", "/api/device/reply"), ("POST", "/api/device/compose")]:
            for name, h in [("none", {}), ("cookie", {"Cookie": CA}), ("malformed", {"Authorization": "BtwDevice a.b"}),
                            ("bad ts", {"Authorization": "BtwDevice a.x1.AAAA"}), ("short sig", {"Authorization": "BtwDevice a.1.AAAA"}),
                            ("stale", {"Authorization": f"BtwDevice dev.1000.{b64u(bytes(64))}"}),
                            ("unpaired", {"Authorization": f"BtwDevice nodevice.{int(time.time() * 1000)}.{b64u(bytes(64))}"})]:
                b(f"{m} {p} (phone auth {name})", m, p, h, b"{}" if m == "POST" else None)

    def offer(self, who=CA, name="POST /api/pair"):
        hg = {"Cookie": who, "Origin": ORIGIN}
        g = raw(self.a.go, "POST", "/api/pair", hg, b"")
        r = raw(self.a.rs, "POST", "/api/pair", hg, b"")
        jg, jr = json.loads(g["body"] or b"{}"), json.loads(r["body"] or b"{}")
        self.reg("pair", jg.get("pair_id"), jr.get("pair_id"))
        tg = jg.get("url", "#.").split("#")[1].split(".")[1] if "url" in jg else None
        tr = jr.get("url", "#.").split("#")[1].split(".")[1] if "url" in jr else None
        self.reg("token", tg, tr)
        self.compare(name, g, r, ("time", "qr"))
        for side, j in (("go", jg), ("rs", jr)):
            if "url" in j and not re.match(r"^https://beatscript\.app/pair#[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{22}$", j["url"]):
                print(f"FAIL [hub] {side}: the QR link is not the phone's form: {j['url']}")
        return {"go": jg.get("pair_id"), "rs": jr.get("pair_id")}, {"go": tg, "rs": tr}

    def claim_both(self, name, mk, rules=("time",)):
        """mk(side) -> body bytes."""
        hg = {"Content-Type": "application/json", "X-Real-IP": self.claim_ip}
        g = raw(self.a.go, "POST", "/api/pair/claim", hg, mk("go"))
        r = raw(self.a.rs, "POST", "/api/pair/claim", hg, mk("rs"))
        return self.compare(name, g, r, rules)

    def pairing(self):
        self.claim_ip = self.ip()
        alice = Phone(1)
        pid, tok = self.offer()
        st = lambda name, extra=None: self.split(name, "GET", self.path2("/api/pair/{}", [pid]), {"Cookie": CA}, rules=("time",))
        st("GET /api/pair/{id} (waiting)")
        self.split("GET /api/pair/{id} (another user)", "GET", self.path2("/api/pair/{}", [pid]), {"Cookie": CB})
        cl = self.claim_both
        cl("claim: not JSON", lambda s: b"x")
        cl("claim: empty", lambda s: b"")
        cl("claim: cut short", lambda s: b'{"proto":')
        cl("claim: trailing garbage is ignored, then proto", lambda s: b'{"proto":"x"} trailing')
        cl("claim: ts is a string", lambda s: b'{"ts":"1"}')
        cl("claim: ts is a float", lambda s: b'{"ts":1.5}')
        cl("claim: ts overflows", lambda s: b'{"ts":99999999999999999999}')
        cl("claim: an array", lambda s: b"[]")
        cl("claim: over 4 KiB", lambda s: json.dumps({"name": "x" * 5000}).encode())
        cl("claim: bad proto", lambda s: alice.claim(pid[s], tok[s], proto="btw-pair/3"))
        cl("claim: bad key", lambda s: alice.claim(pid[s], tok[s], key="AAAA"))
        cl("claim: empty name", lambda s: alice.claim(pid[s], tok[s], name=""))
        cl("claim: name with a newline", lambda s: alice.claim(pid[s], tok[s], name="a\nb"))
        cl("claim: name too long", lambda s: alice.claim(pid[s], tok[s], name="x" * 65))
        cl("claim: clock skew", lambda s: alice.claim(pid[s], tok[s], ts=int(time.time() * 1000) - 180_000))
        cl("claim: bad signature", lambda s: alice.claim(pid[s], tok[s]).replace(b'"sig": "', b'"sig": "AA'))
        cl("claim: v2 signed for another site", lambda s: alice.claim(pid[s], tok[s], site="another-site-id-0000"))
        cl("claim: v1 for a host this site does not answer for", lambda s: alice.claim(pid[s], tok[s], proto="btw-pair/1", host="evil.example"))
        cl("claim: unknown code", lambda s: alice.claim("unknownPAIRid0000000000", tok[s]))
        # a good v2 claim
        self.claim_ip = self.ip()  # 20 claims a minute per address: the rest of the sequence on another one
        bg, br = alice.claim(pid["go"], tok["go"]), alice.claim(pid["rs"], tok["rs"])
        hg = {"Content-Type": "application/json", "X-Real-IP": self.claim_ip}
        g = raw(self.a.go, "POST", "/api/pair/claim", hg, bg)
        r = raw(self.a.rs, "POST", "/api/pair/claim", hg, br)
        dg, dr = json.loads(g["body"]).get("device_id"), json.loads(r["body"]).get("device_id")
        self.reg("device", dg, dr)
        alice.dev = {"go": dg, "rs": dr}
        self.compare("claim: v2 (pairs)", g, r, ("time",))
        g = raw(self.a.go, "POST", "/api/pair/claim", hg, bg)
        r = raw(self.a.rs, "POST", "/api/pair/claim", hg, br)
        self.compare("claim: the same claim again (single use)", g, r)
        st("GET /api/pair/{id} (paired)")
        self.both("GET /api/devices (one, not connected)", "GET", "/api/devices", {"Cookie": CA}, rules=("time",))
        self.both("GET /api/devices (bob sees none)", "GET", "/api/devices", {"Cookie": CB})
        # a wrong token burns the code
        pid2, tok2 = self.offer(name="POST /api/pair (second)")
        cl("claim: wrong token", lambda s: Phone(2).claim(pid2[s], "wrongTOKENwrongTOKEN00"))
        self.split("GET /api/pair/{id} (used)", "GET", self.path2("/api/pair/{}", [pid2]), {"Cookie": CA}, rules=("time",))
        cl("claim: after the wrong token", lambda s: Phone(2).claim(pid2[s], tok2[s]))
        # four offers: the oldest of three open ones stops working
        offers = [self.offer(name=f"POST /api/pair (open {i})") for i in range(4)]
        self.split("GET /api/pair/{id} (oldest of four: used)", "GET", self.path2("/api/pair/{}", [offers[0][0]]), {"Cookie": CA}, rules=("time",))
        # a v1 claim under the legacy host (a second phone), a second later: GET /api/devices sorts by the paired
        # time in seconds, and Go's sort of a tie is not stable (map order), so the two must not share a second
        time.sleep(1.1)
        phone1 = Phone(3)
        p3, t3 = offers[3]
        bg, br = phone1.claim(p3["go"], t3["go"], proto="btw-pair/1", host="btw.bitwrap.io"), phone1.claim(p3["rs"], t3["rs"], proto="btw-pair/1", host="btw.bitwrap.io")
        g = raw(self.a.go, "POST", "/api/pair/claim", hg, bg)
        r = raw(self.a.rs, "POST", "/api/pair/claim", hg, br)
        self.reg("device", json.loads(g["body"]).get("device_id"), json.loads(r["body"]).get("device_id"))
        phone1.dev = {"go": json.loads(g["body"]).get("device_id"), "rs": json.loads(r["body"]).get("device_id")}
        self.compare("claim: v1 under the legacy host (pairs)", g, r, ("time",))
        self.devices(alice, phone1)

    def signed(self, name, phone, method, path, body=b"", v=2, rules=("time",), ts=None):
        bodies = None if body is None else {"go": body, "rs": body}
        return self.split(name, method, {"go": path, "rs": path}, bodies=bodies, rules=rules,
                          hdr_fn=lambda s, p: {"Authorization": phone.auth(s, method, p, body or b"", v=v, ts=ts)})

    def devices(self, alice, phone1):
        sg = self.signed
        sg("GET /api/device (v2)", alice, "GET", "/api/device", None)
        sg("GET /api/device (v1, legacy host)", phone1, "GET", "/api/device", None, v=1)
        self.split("GET /api/device (signature over another path)", "GET", {"go": "/api/device", "rs": "/api/device"},
                   hdr_fn=lambda s, p: {"Authorization": alice.auth(s, "GET", "/api/devices", b"")})
        self.split("GET /api/device (clock skew)", "GET", {"go": "/api/device", "rs": "/api/device"},
                   hdr_fn=lambda s, p: {"Authorization": alice.auth(s, "GET", p, b"", ts=int(time.time() * 1000) + 200_000)})
        # replay: the same credential twice
        cred = {s: alice.auth(s, "GET", "/api/device", b"") for s in ("go", "rs")}
        self.split("GET /api/device (first use)", "GET", {"go": "/api/device", "rs": "/api/device"}, hdr_fn=lambda s, p: {"Authorization": cred[s]}, rules=("time",))
        self.split("GET /api/device (replayed)", "GET", {"go": "/api/device", "rs": "/api/device"}, hdr_fn=lambda s, p: {"Authorization": cred[s]})
        sg("POST /api/device/report (before the stream)", alice, "POST", "/api/device/report", b'{"playing":true}')
        sg("POST /api/device/reply (before the stream)", alice, "POST", "/api/device/reply", b'{"request_id":"x"}')
        sg("POST /api/device/reply (bad body)", alice, "POST", "/api/device/reply", b'{"request_id":""}')
        sg("POST /api/device/reply (not JSON)", alice, "POST", "/api/device/reply", b"{x")
        sg("POST /api/device/report (not JSON)", alice, "POST", "/api/device/report", b"{x")
        sg("POST /api/device/report (empty)", alice, "POST", "/api/device/report", b"")
        sg("POST /api/device/report (trailing data)", alice, "POST", "/api/device/report", b"{} {}")
        sg("POST /api/device/report (type error)", alice, "POST", "/api/device/report", b'{"bar":"x"}')
        sg("POST /api/device/report (presence type error)", alice, "POST", "/api/device/report", b'{"presence":{"idle_ms":1.5}}')
        sg("POST /api/device/report (over 256 KiB)", alice, "POST", "/api/device/report", b'{"text":"' + b"x" * (256 << 10) + b'"}')
        # device compose: everything before compose itself
        for nm, body in [("not JSON", b"x"), ("empty", b""), ("unknown field", b'{"prompt":"x","colour":"red"}'), ("type error", b'{"prompt":"x","bpm":"fast"}'),
                         ("no prompt", b'{"genre":"techno"}'), ("blank prompt", b'{"prompt":"  "}'), ("bad length", b'{"prompt":"x","length":true}'),
                         ("nested type error", b'{"prompt":1}')]:
            sg(f"POST /api/device/compose ({nm})", alice, "POST", "/api/device/compose", body)
        sg("POST /api/device/compose (valid prompt)", alice, "POST", "/api/device/compose", b'{"prompt":"dark techno 128","length":"short"}', rules=("time",))
        # the phone's event stream
        st = self.sse_pair("dev", {"go": "/api/device/events", "rs": "/api/device/events"}, lambda s, p: {"Authorization": alice.auth(s, "GET", p, b"")}, 1)
        sgo, srs = st
        self.sse_compare("GET /api/device/events (hello)", sgo, srs, 1)
        sg("POST /api/device/report (streaming)", alice, "POST", "/api/device/report", json.dumps({"playing": True, "cid": "zSomething", "section": "intro", "bar": 3, "marks": {"a": [1, 2]}, "text": "<b>&"}).encode())
        self.both("GET /api/devices (connected, with state)", "GET", "/api/devices", {"Cookie": CA}, rules=("time",))
        sg("POST /api/device/reply (unknown request id)", alice, "POST", "/api/device/reply", b'{"request_id":"nope","state":{}}')
        # the page drives the phone
        dev = alice.dev
        push = lambda name, body, rules=(): self.split(name, "POST", self.path2("/api/devices/{}/push", [dev]), {"Cookie": CA}, {"go": body, "rs": body}, rules=rules)
        push("POST /api/devices/{id}/push (stop)", b'{"type":"stop"}')
        push("POST /api/devices/{id}/push (eval)", json.dumps({"type": "eval", "text": PROG, "play": True}).encode())
        push("POST /api/devices/{id}/push (load)", json.dumps({"type": "load", "text": PROG}).encode())
        push("POST /api/devices/{id}/push (errors)", json.dumps({"type": "eval", "text": "deck A: nonsense ((("}).encode())
        push("POST /api/devices/{id}/push (no text)", b'{"type":"eval","text":"  "}')
        push("POST /api/devices/{id}/push (bad type)", b'{"type":"dance"}')
        push("POST /api/devices/{id}/push (not JSON)", b"nope")
        push("POST /api/devices/{id}/push (type error)", b'{"play":"yes"}')
        self.split("POST /api/devices/{id}/push (bob)", "POST", self.path2("/api/devices/{}/push", [dev]), {"Cookie": CB}, {"go": b'{"type":"stop"}', "rs": b'{"type":"stop"}'})
        self.sse_compare("GET /api/device/events (after pushes)", sgo, srs, 5)
        # phone1 unpaired by the page: its stream would say so; then alice's phone forgets itself
        self.split("DELETE /api/devices/{id} (bob)", "DELETE", self.path2("/api/devices/{}", [phone1.dev]), {"Cookie": CB})
        self.split("DELETE /api/devices/{id}", "DELETE", self.path2("/api/devices/{}", [phone1.dev]), {"Cookie": CA})
        self.split("DELETE /api/devices/{id} (again)", "DELETE", self.path2("/api/devices/{}", [phone1.dev]), {"Cookie": CA})
        sg("GET /api/device (unpaired by the page)", phone1, "GET", "/api/device", None)
        sg("DELETE /api/device (the phone forgets)", alice, "DELETE", "/api/device", None)
        self.sse_compare("GET /api/device/events (unpaired, ended)", sgo, srs, 6)
        sgo.close(), srs.close()
        sg("GET /api/device (after forgetting)", alice, "GET", "/api/device", None)
        self.both("GET /api/devices (none left)", "GET", "/api/devices", {"Cookie": CA})

    def sessions(self):
        ip = self.ip()
        H = {"X-Real-IP": ip}
        g = raw(self.a.go, "POST", "/api/session", H, b"")
        r = raw(self.a.rs, "POST", "/api/session", H, b"")
        anon = {"go": json.loads(g["body"])["id"], "rs": json.loads(r["body"])["id"]}
        self.reg("session", anon["go"], anon["rs"])
        self.compare("POST /api/session (anonymous)", g, r)
        hc = dict(H, Cookie=CA)
        g = raw(self.a.go, "POST", "/api/session", hc, b"")
        r = raw(self.a.rs, "POST", "/api/session", hc, b"")
        mine = {"go": json.loads(g["body"])["id"], "rs": json.loads(r["body"])["id"]}
        self.reg("session", mine["go"], mine["rs"])
        self.compare("POST /api/session (signed in)", g, r)
        g = raw(self.a.go, "POST", "/api/session", dict(H, Authorization="Bearer btw_unknown000000000000000000000"), b"")
        r = raw(self.a.rs, "POST", "/api/session", dict(H, Authorization="Bearer btw_unknown000000000000000000000"), b"")
        self.reg("session", json.loads(g["body"])["id"], json.loads(r["body"])["id"])
        self.compare("POST /api/session (unknown token: anonymous)", g, r)
        P = lambda t: self.path2(t, [anon])
        M = lambda t: self.path2(t, [mine])
        sp = lambda name, paths, body, h=None, rules=(): self.split(name, "POST", paths, h or {}, {"go": body, "rs": body}, rules=rules)
        # events
        self.both("GET /api/session/{unknown}/events", "GET", "/api/session/nosuchsession0000000000/events")
        s_anon = self.sse_pair("a", P("/api/session/{}/events"), lambda s, p: {}, 0)
        self.sse_compare("GET /api/session/{id}/events (opens)", *s_anon, 1)
        s_mine = self.sse_pair("m", M("/api/session/{}/events"), lambda s, p: {"Cookie": CA}, 0)
        self.split("GET /api/session/{id}/events (bob, on alice's)", "GET", M("/api/session/{}/events"), {"Cookie": CB})
        # reports
        for nm, body in [("not JSON", b"x"), ("empty", b""), ("cut short", b'{"text":'), ("type error", b'{"playing":"yes"}'), ("nested type error", b'{"presence":{"visible":1}}'),
                         ("presence not an object", b'{"presence":5}'), ("an array", b"[1]"), ("bad escape", b'{"text":"\\x"}'), ("over 256 KiB", b'{"text":"' + b"x" * (256 << 10) + b'"}'),
                         ("case-insensitive keys", b'{"CID":"zA","Playing":true}'), ("unknown keys", b'{"zzz":1}')]:
            sp(f"POST /api/session/{{id}}/report ({nm})", P("/api/session/{}/report"), body)
        sp("POST /api/session/{id}/report (presence)", P("/api/session/{}/report"), json.dumps({"playing": True, "presence": {"visible": True, "focused": True, "audio": "running", "idle_ms": 500}}).encode())
        self.both("POST /api/session/{unknown}/report", "POST", "/api/session/nosuch/report", None, b"{}")
        sp("POST /api/session/{id}/report (bob on alice's)", M("/api/session/{}/report"), b"{}", {"Cookie": CB})
        sp("POST /api/session/{id}/report (alice)", M("/api/session/{}/report"), b'{"text":"t"}', {"Cookie": CA})
        # replies
        for nm, body in [("no request id", b'{"state":{}}'), ("not JSON", b"{"), ("type error", b'{"request_id":5}'), ("state type error", b'{"request_id":"r","state":{"bar":true}}'),
                         ("unknown request", b'{"request_id":"nope","state":{"cid":"z"}}')]:
            sp(f"POST /api/session/{{id}}/reply ({nm})", P("/api/session/{}/reply"), body)
        self.both("POST /api/session/{unknown}/reply", "POST", "/api/session/nosuch/reply", None, b'{"request_id":"r"}')
        # the queue's page side
        self.split("GET /api/session/{id}/queue (none yet)", "GET", M("/api/session/{}/queue"), {"Cookie": CA})
        self.split("GET /api/session/{id}/queue (anonymous, reported presence)", "GET", P("/api/session/{}/queue"))
        self.both("GET /api/session/{unknown}/queue", "GET", "/api/session/nosuch/queue")
        self.split("GET /api/session/{id}/queue (bob)", "GET", M("/api/session/{}/queue"), {"Cookie": CB})
        for nm, body in [("bad reason", b'{"reason":"because"}'), ("no body", b""), ("not JSON", b"x"), ("presence type error", b'{"reason":"ended","presence":{"audio":1}}'),
                         ("over 4 KiB", json.dumps({"reason": "ended", "x": "y" * 5000}).encode()),
                         ("ended, empty queue", json.dumps({"reason": "ended", "presence": {"visible": True, "audio": "running", "idle_ms": 0}}).encode()),
                         ("skip, suspended", json.dumps({"reason": "skip", "presence": {"visible": False, "audio": "suspended"}}).encode()),
                         ("resume, no presence", b'{"reason":"resume"}'), ("length, null presence", b'{"reason":"length","presence":null}')]:
            sp(f"POST /api/session/{{id}}/queue/next ({nm})", M("/api/session/{}/queue/next"), body, {"Cookie": CA})
        self.both("POST /api/session/{unknown}/queue/next", "POST", "/api/session/nosuch/queue/next", None, b'{"reason":"ended"}')
        sp("POST /api/session/{id}/queue/next (bob)", M("/api/session/{}/queue/next"), b'{"reason":"ended"}', {"Cookie": CB})
        self.split("GET /api/session/{id}/queue (after next)", "GET", M("/api/session/{}/queue"), {"Cookie": CA})
        self.sse_compare("GET /api/session/{id}/events (signed in, after the queue calls)", *s_mine, 1)
        for s in s_anon + s_mine:
            s.close()
        # five streams on one session: the oldest is closed by the hub
        streams = [self.sse_pair(f"x{i}", P("/api/session/{}/events"), lambda s, p: {}, 0) for i in range(5)]
        time.sleep(0.3)
        self.sse_compare("GET /api/session/{id}/events (oldest of five: closed)", *streams[0], 1)
        for p in streams:
            for s in p:
                s.close()

    def mcp(self, name, tool, args, rid=1, rules=("time",)):
        """An MCP tool call with user 4's bearer token (bench/pair.sh seeds it on both servers)."""
        b = json.dumps({"jsonrpc": "2.0", "id": rid, "method": "tools/call", "params": {"name": tool, "arguments": args}}).encode()
        h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream", "Authorization": "Bearer btw_contracttesttoken0000000000000004",
             "X-Real-IP": self.mcp_ip}
        return self.both(f"MCP {tool}: {name}", "POST", "/mcp", h, b, rules=rules)

    def mcp_drive(self):
        """MCP drives a signed-in tab through the hub (the Hub API P7's tools call): push, get, the queue."""
        self.mcp_ip = self.ip()
        U4 = cookie("ed25519:" + "4" * 64)
        H = {"Cookie": U4, "X-Real-IP": self.ip()}
        self.mcp("no tab", "queue", {"action": "list"})
        self.mcp("no tab", "get", {})
        self.mcp("no tab", "stop", {})
        g = raw(self.a.go, "POST", "/api/session", H, b"")
        r = raw(self.a.rs, "POST", "/api/session", H, b"")
        sid = {"go": json.loads(g["body"])["id"], "rs": json.loads(r["body"])["id"]}
        self.reg("session", sid["go"], sid["rs"])
        self.compare("POST /api/session (user 4)", g, r)
        tab = self.sse_pair("t", self.path2("/api/session/{}/events", [sid]), lambda s, p: {"Cookie": U4}, 0)
        self.sse_compare("GET /api/session/{id}/events (user 4)", *tab, 1)
        rep = json.dumps({"text": PROG, "cid": "", "playing": False, "presence": {"visible": True, "focused": True, "audio": "running", "idle_ms": 100}}).encode()
        self.split("POST /api/session/{id}/report (user 4, listening)", "POST", self.path2("/api/session/{}/report", [sid]), {"Cookie": U4}, {"go": rep, "rs": rep})
        self.mcp("a tab, nothing queued", "queue", {"action": "list"})
        self.mcp("eval to all my tabs", "eval", {"text": PROG})
        self.mcp("stop", "stop", {})
        ex = json.loads(raw(self.a.go, "GET", "/api/examples")["body"])
        self.mcp("add two", "queue", {"action": "add", "items": [{"example": ex[0]["name"]}, {"example": ex[1]["name"], "bars": 8}]})
        self.mcp("list", "queue", {"action": "list"})
        self.mcp("skip (a listener)", "queue", {"action": "skip"})
        self.mcp("remove q1", "queue", {"action": "remove", "item": "q1"})
        self.mcp("remove nothing", "queue", {"action": "remove", "item": "q9"})
        self.mcp("another account's session", "queue", {"action": "list", "session": "nosuchsession000000000"})
        self.split("GET /api/session/{id}/queue (after MCP adds)", "GET", self.path2("/api/session/{}/queue", [sid]), {"Cookie": U4})
        nb = json.dumps({"reason": "ended", "presence": {"visible": True, "focused": True, "audio": "running", "idle_ms": 50}}).encode()
        self.split("POST /api/session/{id}/queue/next (plays q2)", "POST", self.path2("/api/session/{}/queue/next", [sid]), {"Cookie": U4}, {"go": nb, "rs": nb})
        self.split("POST /api/session/{id}/queue/next (empty now)", "POST", self.path2("/api/session/{}/queue/next", [sid]), {"Cookie": U4}, {"go": nb, "rs": nb})
        self.mcp("get (a fresh report)", "get", {})
        self.sse_compare("GET /api/session/{id}/events (user 4: eval, stop, queue events)", *tab, 8)
        for s_ in tab:
            s_.close()

    def limits(self):
        # 120 sessions an hour per address: the 121st is refused
        ip = self.ip()
        last = None
        for i in range(121):
            g = raw(self.a.go, "POST", "/api/session", {"X-Real-IP": ip}, b"")
            r = raw(self.a.rs, "POST", "/api/session", {"X-Real-IP": ip}, b"")
            if i < 120:
                if json.loads(g["body"]).get("id") is None or json.loads(r["body"]).get("id") is None:
                    print(f"FAIL [hub] session {i} refused early")
            last = (g, r)
        self.compare("POST /api/session (the 121st in an hour)", *last, rules=("retry",))
        # 20 claims a minute per address
        ip = self.ip()
        for i in range(21):
            g = raw(self.a.go, "POST", "/api/pair/claim", {"X-Real-IP": ip}, b"{}")
            r = raw(self.a.rs, "POST", "/api/pair/claim", {"X-Real-IP": ip}, b"{}")
        self.compare("POST /api/pair/claim (the 21st in a minute)", g, r, rules=("retry",))
        # 20 offers a minute per address
        ip = self.ip()
        for i in range(21):
            g = raw(self.a.go, "POST", "/api/pair", {"X-Real-IP": ip, "Cookie": CB}, b"")
            r = raw(self.a.rs, "POST", "/api/pair", {"X-Real-IP": ip, "Cookie": CB}, b"")
        self.compare("POST /api/pair (the 21st in a minute)", g, r, rules=("retry",))


def run(contract_run):
    Hub(contract_run).run()
