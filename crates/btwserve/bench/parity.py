#!/usr/bin/env python3
"""Parity diff of the render route: the Go server (btwsite + btwd + ffmpeg) against btwserve, request by request.

    parity.py prepare <programs dir>          add the refusal fixtures (too long, restricted, failing) to a store copy
    parity.py run --go 127.0.0.1:18201 --rs 127.0.0.1:18202 --btwd <btwd> --store <programs dir> \\
                  [--examples btwsite/examples] [--limit N] [--out report.json]
    parity.py limits --go ... --rs ...        the 429 cases (start both servers with -render-ip-hour 1, -render-concurrency 1)
    parity.py shadow --go 127.0.0.1:18097 --rs 127.0.0.1:18098 --render-dir <Go's data/render> [--limit N] [--sleep S]
        live verification before the flip: every render already in the Go server's cache is asked of btwserve (it
        renders it, at its own cost), the WAV bytes are checked against Go's file, and the cached answers of both
        (headers, Range, status) are diffed. The Go server only serves cache hits: no new render work for it.

Every case is sent to both servers over a raw socket, so the header block is compared as written (order, case,
values; `Date` ignored). Bodies are compared by SHA-256. The corpus is every program in the store plus the site's
examples, and for each: the streamed render (WAV, Opus), the cached file, HEAD, ten Range / conditional variants and
the status JSON; plus the route's errors, CORS and share links.

WAV must be byte-identical. Opus cannot be (Go encodes with ffmpeg + the system libopus after resampling 44.1 to
48 kHz; btwserve renders at 48 kHz and encodes with libopus 1.3.1 in-process), so for Opus the diff reports: headers
(minus Content-Length), that btwserve's own bytes are reproducible (streamed == cached == a second render), and the
decoded PCM of each against the 48 kHz WAV render of the same program (SNR in dB, after aligning by cross-correlation
over the first second). Needs ffmpeg for the decode.
"""
import argparse, array, hashlib, json, math, os, re, socket, subprocess, sys, tempfile, threading, time
from concurrent.futures import ThreadPoolExecutor

# Date changes every second; Connection is hop-by-hop (nginx never forwards it, and Go writes its `close` after
# Content-Length where hyper writes it before)
IGNORE = {"date", "connection"}


def http(addr, method, path, headers=None, timeout=900):
    host, port = addr.rsplit(":", 1)
    s = socket.create_connection((host, int(port)), timeout=timeout)
    req = f"{method} {path} HTTP/1.1\r\nHost: beatscript.app\r\nConnection: close\r\n"
    for k, v in (headers or {}).items():
        req += f"{k}: {v}\r\n"
    s.sendall((req + "\r\n").encode())
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
    head, _, body = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    status = lines[0]
    hdrs = [tuple(l.split(": ", 1)) for l in lines[1:] if ": " in l]
    te = [v for k, v in hdrs if k.lower() == "transfer-encoding"]
    broken = False
    if te and "chunked" in te[0] and method != "HEAD":
        out, rest = b"", body
        while True:
            line, sep, rest = rest.partition(b"\r\n")
            if not sep:
                broken = True
                break
            try:
                n = int(line.split(b";")[0], 16)
            except ValueError:
                broken = True
                break
            if n == 0:
                break
            if len(rest) < n:
                out += rest
                broken = True
                break
            out, rest = out + rest[:n], rest[n + 2:]
        body = out
    return {"status": status, "headers": hdrs, "body": body, "broken": broken}


def norm(r, drop=()):
    hs = [(k, v) for k, v in r["headers"] if k.lower() not in IGNORE and k.lower() not in drop]
    return hs


def boundary_free(r):
    """Multipart bodies and their Content-Type differ only in the random boundary."""
    m = [v for k, v in r["headers"] if k.lower() == "content-type" and v.startswith("multipart/byteranges; boundary=")]
    if not m:
        return r
    b = m[0].split("boundary=", 1)[1]
    r = dict(r)
    r["headers"] = [(k, v.replace(b, "BOUNDARY")) for k, v in r["headers"]]
    r["body"] = r["body"].replace(b.encode(), b"BOUNDARY")
    return r


def status_free(r):
    """A status answer with the Opus file size left out (Opus bytes differ by design; the rest must not)."""
    if not r["body"].startswith(b'{"bars"'):
        return r
    try:
        j = json.loads(r["body"])
    except ValueError:
        return r
    o = j.get("formats", {}).get("opus", {})
    if "bytes" not in o or o.get("state") == "rendering":
        return r
    o.pop("bytes")
    r = dict(r)
    r["body"] = json.dumps(j).encode()
    r["opus_bytes_dropped"] = True
    return r


def sha(b):
    return hashlib.sha256(b).hexdigest()[:16]


class Diff:
    def __init__(self):
        self.cases = []
        self.lock = threading.Lock()

    def compare(self, name, a, b, drop=(), body=True):
        a, b = boundary_free(a), boundary_free(b)
        a, b, opus_bytes = status_free(a), status_free(b), None
        if a.get("opus_bytes_dropped") or b.get("opus_bytes_dropped"):
            drop = tuple(drop) + ("content-length",)
        diffs = []
        if a["status"] != b["status"]:
            diffs.append(f"status {a['status']!r} vs {b['status']!r}")
        ha, hb = norm(a, drop), norm(b, drop)
        if ha != hb:
            sa, sb = set(ha), set(hb)
            diffs.append(f"headers: go-only {sorted(sa - sb)} rs-only {sorted(sb - sa)}" + ("" if sa != sb else " (order)"))
        if body and sha(a["body"]) != sha(b["body"]):
            diffs.append(f"body {len(a['body'])}B {sha(a['body'])} vs {len(b['body'])}B {sha(b['body'])}")
        if a["broken"] != b["broken"]:
            diffs.append(f"broken stream {a['broken']} vs {b['broken']}")
        with self.lock:
            self.cases.append({"case": name, "same": not diffs, "diffs": diffs, "status": a["status"].split(" ", 1)[1] if " " in a["status"] else a["status"]})
            print(("SAME " if not diffs else "DIFF ") + name + ("" if not diffs else "\n     " + "\n     ".join(diffs)), flush=True)
        return not diffs


def hdr(r, name):
    for k, v in r["headers"]:
        if k.lower() == name:
            return v
    return None


def decode_f32(data, sr=48000):
    with tempfile.NamedTemporaryFile(suffix=".opus") as f:
        f.write(data)
        f.flush()
        out = subprocess.run(["ffmpeg", "-v", "error", "-i", f.name, "-f", "f32le", "-ac", "2", "-ar", str(sr), "-"], capture_output=True).stdout
    a = array.array("f")
    a.frombytes(out[: len(out) // 4 * 4])
    return a


def wav_f32(data):
    a = array.array("h")
    a.frombytes(data[44: 44 + (len(data) - 44) // 2 * 2])
    return array.array("f", (x / 32768.0 for x in a))


def snr(ref, x):
    """Best lag over +-1000 samples (stereo frames) on the first second, then SNR over the overlap."""
    n = min(len(ref), len(x))
    win = min(96000, n)
    best, lag = -1, 0
    for L in range(-1000, 1001, 2):
        s = 0.0
        for i in range(2000, win - 2002, 7):
            j = i + L
            s += ref[i] * x[j]
        if s > best:
            best, lag = s, L
    sig = err = 0.0
    lo, hi = max(0, -lag), min(len(ref), len(x) - lag)
    for i in range(lo, hi, 3):
        r = ref[i]
        d = r - x[i + lag]
        sig += r * r
        err += d * d
    return (10 * math.log10(sig / err) if err > 0 else 99.0), lag // 2


def corpus(store, examples, btwd):
    cids = []
    for f in sorted(os.listdir(store)):
        if f.endswith(".json"):
            cids.append(("store", f[:-5]))
    if examples:
        for f in sorted(os.listdir(examples)):
            if f.endswith(".btw"):
                rep = json.loads(subprocess.run([btwd, "check", "--file", os.path.join(examples, f)], capture_output=True, text=True).stdout)
                cids.append(("example:" + f[:-4], rep["cid"]))
    return cids


def prepare(store, btwd):
    fixtures = {
        "too-long": "tempo 120\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nend at bar 3000\n",
        "restricted": "use restricted\ntempo 120\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nend at bar 2\n",
        "fails": "tempo 120\ndeck A: kick x ~ ~ ~ nosuchword ~ ~ ~\nend at bar 2\n",
    }
    for name, text in fixtures.items():
        rep = json.loads(subprocess.run([btwd, "check"], input=text, capture_output=True, text=True).stdout)
        cid = rep.get("cid")
        canon = rep.get("canonical") or text
        if not cid:
            print(f"{name}: no CID from btwd check, skipped")
            continue
        with open(os.path.join(store, cid + ".json"), "w") as f:
            json.dump({"cid": cid, "text": canon, "meta": {"license": "CC0-1.0", "visibility": "public", "name": "parity-" + name, "source": "put"}}, f)
        print(name, cid, "ok" if rep.get("ok") else "does not check")


def run(a):
    d = Diff()
    G, R = a.go, a.rs
    pool = ThreadPoolExecutor(8)

    def both(m, p, h=None):
        # the same request to both servers at once (a render takes as long on either)
        fg, fr = pool.submit(http, G, m, p, h), pool.submit(http, R, m, p, h)
        return fg.result(), fr.result()

    items = corpus(a.store, a.examples, a.btwd)[: a.limit or None]
    opus = []

    def one(src, cid):
        base = f"/api/render/{cid}"
        st = both("GET", base + "/status")
        if not st[0]["status"].endswith("200 OK"):
            # a refused program: the render answers must match too
            d.compare(f"{src} {cid[:12]} status (refused)", *st)
            d.compare(f"{src} {cid[:12]} GET wav (refused)", *both("GET", base + "?format=wav"))
            return
        js = json.loads(st[0]["body"])
        if js.get("restricted") or js.get("bars", 0) > 2400:
            d.compare(f"{src} {cid[:12]} status", *st)
            d.compare(f"{src} {cid[:12]} GET wav (refused)", *both("GET", base + "?format=wav"))
            d.compare(f"{src} {cid[:12]} HEAD wav (refused)", *both("HEAD", base + "?format=wav"))
            return
        d.compare(f"{src} {cid[:12]} status (none)", *st)
        d.compare(f"{src} {cid[:12]} HEAD wav (miss)", *both("HEAD", base + "?format=wav"))
        g, r = both("GET", base + "?format=wav")
        d.compare(f"{src} {cid[:12]} GET wav (streamed)", g, r)
        if not g["status"].endswith("200 OK"):
            return
        for _ in range(100):  # the job table empties a moment after the last byte
            if json.loads(http(G, "GET", base + "/status")["body"])["formats"]["wav"]["state"] == "ready" and json.loads(http(R, "GET", base + "/status")["body"])["formats"]["wav"]["state"] == "ready":
                break
            time.sleep(0.1)
        hit = both("GET", base + "?format=wav")
        d.compare(f"{src} {cid[:12]} GET wav (cached)", *hit)
        etag = hdr(hit[0], "etag")
        d.compare(f"{src} {cid[:12]} HEAD wav (cached)", *both("HEAD", base + "?format=wav"))
        for name, h in [
            ("range 0-99", {"Range": "bytes=0-99"}),
            ("range 100-", {"Range": "bytes=100-"}),
            ("range -500", {"Range": "bytes=-500"}),
            ("range 0-0", {"Range": "bytes=0-0"}),
            ("range -0", {"Range": "bytes=-0"}),
            ("range past end", {"Range": "bytes=999999999-"}),
            ("range garbage", {"Range": "bytes=abc"}),
            ("range not bytes", {"Range": "items=0-1"}),
            ("range multi", {"Range": "bytes=0-10,20-30,-5"}),
            ("range multi oversize", {"Range": "bytes=0-,0-"}),
            ("if-none-match", {"If-None-Match": etag}),
            ("if-none-match weak list", {"If-None-Match": f'"x", W/{etag}'}),
            ("if-none-match other", {"If-None-Match": '"nope"'}),
            ("if-match other", {"If-Match": '"nope"'}),
            ("if-match *", {"If-Match": "*"}),
            ("if-range same", {"Range": "bytes=0-99", "If-Range": etag}),
            ("if-range other", {"Range": "bytes=0-99", "If-Range": '"nope"'}),
            ("if-range date", {"Range": "bytes=0-99", "If-Range": "Wed, 21 Oct 2015 07:28:00 GMT"}),
        ]:
            d.compare(f"{src} {cid[:12]} wav {name}", *both("GET", base + "?format=wav", h))
        d.compare(f"{src} {cid[:12]} status (wav ready)", *both("GET", base + "/status"))
        # opus
        g, r = both("GET", base)
        d.compare(f"{src} {cid[:12]} GET opus (streamed) headers", g, r, drop=("content-length",), body=False)
        time.sleep(0.5)
        gh, rh = both("GET", base + "?format=opus")
        d.compare(f"{src} {cid[:12]} GET opus (cached) headers", gh, rh, drop=("content-length",), body=False)
        d.compare(f"{src} {cid[:12]} opus range 0-99 headers", *both("GET", base + "?format=opus", {"Range": "bytes=0-99"}), drop=("content-length", "content-range"), body=False)
        stg, str_ = both("GET", base + "/status")
        for x in (stg, str_):
            j = json.loads(x["body"])
            j["formats"]["opus"].pop("bytes", None)
            x["body"] = json.dumps(j).encode()
        d.compare(f"{src} {cid[:12]} status (both ready; opus bytes left out)", stg, str_, drop=("content-length",))
        with d.lock:
            opus.append({"src": src, "cid": cid, "go": g["body"], "rs": r["body"], "rs_cached": rh["body"], "go_cached": gh["body"]})
    # two programs at a time: both servers then render two at once (their -render-concurrency 2)
    with ThreadPoolExecutor(a.jobs) as ex:
        list(ex.map(lambda it: one(*it), items))
    # ---- the route's own errors, CORS, share links
    ex = items[-1][1] if items else "z4EBG9jGG9mZezk4KPriWXnEa8ec879Q2HDteNAxo9VR723R2Vz"
    unknown = "z4EBG9jGG9mZezk4KPriWXnEa8ec879Q2HDteNAxo9VR723R2Vy"
    for name, m, p, h in [
        ("bad cid", "GET", "/api/render/abc", None),
        ("unknown cid", "GET", f"/api/render/{unknown}", None),
        ("unknown cid status", "GET", f"/api/render/{unknown}/status", None),
        ("bad cid status", "GET", "/api/render/abc/status", None),
        ("format mp3", "GET", f"/api/render/{ex}?format=mp3", None),
        ("sr 22050", "GET", f"/api/render/{ex}?sr=22050", None),
        ("sr abc", "GET", f"/api/render/{ex}?sr=abc", None),
        ("status sr 48000", "GET", f"/api/render/{ex}/status?sr=48000", None),
        ("status sr junk", "GET", f"/api/render/{ex}/status?sr=7", None),
        ("CORS preflight", "OPTIONS", f"/api/render/{ex}", {"Origin": "https://example.org", "Access-Control-Request-Method": "GET"}),
        ("CORS GET cached", "GET", f"/api/render/{ex}?format=wav", {"Origin": "https://example.org"}),
        ("CORS HEAD", "HEAD", f"/api/render/{ex}?format=wav", {"Origin": "https://example.org"}),
        ("healthz", "GET", "/healthz", None),
        ("link garbage", "GET", f"/api/render/{unknown}.@@@@", None),
        ("link empty", "GET", f"/api/render/{unknown}.", None),
    ]:
        d.compare("route: " + name, *both(m, p, h))
    # share links: a program in no store, carried in the URL
    for name, text in [("link new", "tempo 121\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nend at bar 2\n"), ("link restricted", "use restricted\ntempo 121\ndeck A: kick x ~ x ~\nend at bar 2\n")]:
        sh = json.loads(subprocess.run([a.btwd, "share"], input=text, capture_output=True, text=True).stdout)
        if not sh.get("url"):
            continue
        frag = sh["url"].split("#", 1)[1]
        cid, packed = frag.split(".", 1)
        d.compare(f"route: {name} wav", *both("GET", f"/api/render/{cid}.{packed}?format=wav"))
        d.compare(f"route: {name} again (stored now)", *both("GET", f"/api/render/{cid}.{packed}?format=wav"))
        other = items[0][1] if items else unknown
        d.compare(f"route: {name} under another CID", *both("GET", f"/api/render/{other[:-1] + ('x' if other[-1] != 'x' else 'y')}.{packed}?format=wav"))
    # ---- opus: reproducible here, and close to the WAV there
    orep = []
    for o in opus:
        row = {"src": o["src"], "cid": o["cid"], "go_bytes": len(o["go"]), "rs_bytes": len(o["rs"]),
               "rs_streamed_eq_cached": sha(o["rs"]) == sha(o["rs_cached"]), "go_streamed_eq_cached": sha(o["go"]) == sha(o["go_cached"])}
        if a.snr and len(orep) < a.snr:
            w48 = http(R, "GET", f"/api/render/{o['cid']}?format=wav&sr=48000")["body"]
            ref = wav_f32(w48)
            row["snr_go_db"], row["lag_go"] = snr(ref, decode_f32(o["go"]))
            row["snr_rs_db"], row["lag_rs"] = snr(ref, decode_f32(o["rs"]))
            row["snr_go_vs_rs_db"], _ = snr(decode_f32(o["go"]), decode_f32(o["rs"]))
        orep.append(row)
        print("OPUS", json.dumps({k: (round(v, 1) if isinstance(v, float) else v) for k, v in row.items()}), flush=True)
    same = sum(c["same"] for c in d.cases)
    print(f"\n{same}/{len(d.cases)} cases identical")
    if a.out:
        json.dump({"cases": d.cases, "opus": orep, "same": same, "total": len(d.cases)}, open(a.out, "w"), indent=1)
    return 0 if same == len(d.cases) else 1


def limits(a):
    """Both servers started with -render-ip-hour 1 -render-concurrency 1 and fresh caches."""
    d = Diff()
    G, R = a.go, a.rs
    texts = [f"tempo {t}\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nend at bar 120\n" for t in (90, 91, 92)]
    links = []
    for t in texts:
        sh = json.loads(subprocess.run([a.btwd, "share"], input=t, capture_output=True, text=True).stdout)
        links.append(sh["url"].split("#", 1)[1])
    # one long render running on each, from 10.0.0.1
    import threading
    hold = {}
    def first(addr):
        hold[addr] = http(addr, "GET", f"/api/render/{links[0]}?format=wav", {"X-Real-IP": "10.0.0.1"})
    ts = [threading.Thread(target=first, args=(x,)) for x in (G, R)]
    [t.start() for t in ts]
    time.sleep(2.0)
    both = lambda m, p, h=None: (http(G, m, p, h), http(R, m, p, h))
    # same address, a second new render: rate_limited (checked before the slots)
    d.compare("limit: 2nd new render, same address (rate_limited)", *both("GET", f"/api/render/{links[1]}?format=wav", {"X-Real-IP": "10.0.0.1"}), drop=("content-length",))
    # another address while the one slot is taken: busy (Retry-After depends on progress: compared without it)
    g, r = both("GET", f"/api/render/{links[2]}?format=wav", {"X-Real-IP": "10.0.0.2"})
    for x in (g, r):
        x["body"] = re.sub(rb'"retry_after":\d+', b'"retry_after":N', x["body"])
    d.compare("limit: new render, other address, slot busy (busy)", g, r, drop=("retry-after", "content-length"))
    print("busy Retry-After go", hdr(g, "retry-after"), "rs", hdr(r, "retry-after"))
    # joining the running one is not a new render
    g, r = both("HEAD", f"/api/render/{links[0].split('.')[0]}?format=wav", {"X-Real-IP": "10.0.0.3"})
    d.compare("limit: HEAD of the running render (joined)", g, r)
    [t.join() for t in ts]
    d.compare("limit: the long render itself", hold[G], hold[R])
    same = sum(c["same"] for c in d.cases)
    print(f"\n{same}/{len(d.cases)} cases identical")
    if a.out:
        json.dump({"cases": d.cases, "same": same, "total": len(d.cases)}, open(a.out, "w"), indent=1)
    return 0 if same == len(d.cases) else 1


def shadow(a):
    d = Diff()
    G, R = a.go, a.rs
    # Go's cache names since r2: <cid>.<sr>.r2.<fmt> (render version in the name)
    names = sorted(n for n in os.listdir(a.render_dir) if re.match(r"^z[1-9A-HJ-NP-Za-km-z]{20,80}\.\d+\.r2\.(wav|opus)$", n))
    if a.limit:
        names = names[: a.limit]
    rows = []
    for n in names:
        cid, sr, _ver, fmt = n.split(".")
        path = f"/api/render/{cid}?format={fmt}&sr={sr}&v=r2"
        # btwserve's Opus is its own render version (r2rs): its length and ETag are its own
        drop = ("content-length", "etag") if fmt == "opus" else ()
        t = time.time()
        first = http(R, "GET", path)  # renders on btwserve (or joins / hits)
        took = time.time() - t
        row = {"file": n, "rs_first": first["status"], "rs_first_s": round(took, 2), "rs_cache": hdr(first, "x-btw-cache")}
        if fmt == "wav":
            with open(os.path.join(a.render_dir, n), "rb") as f:
                row["wav_identical"] = sha(f.read()) == sha(first["body"])
            print(("SAME " if row["wav_identical"] else "DIFF ") + f"{n} WAV bytes vs the Go server's cached file", flush=True)
        for _ in range(50):
            if json.loads(http(R, "GET", f"/api/render/{cid}/status?sr={sr}")["body"]).get("formats", {}).get(fmt, {}).get("state") == "ready":
                break
            time.sleep(0.2)
        d.compare(f"{n} GET (cached on both)", http(G, "GET", path), http(R, "GET", path), drop=drop, body=fmt == "wav")
        d.compare(f"{n} HEAD", http(G, "HEAD", path), http(R, "HEAD", path), drop=drop)
        d.compare(f"{n} Range 0-1023", http(G, "GET", path, {"Range": "bytes=0-1023"}), http(R, "GET", path, {"Range": "bytes=0-1023"}), drop=drop + ("content-range",), body=fmt == "wav")
        rows.append(row)
        time.sleep(a.sleep)
    same = sum(c["same"] for c in d.cases)
    wav = [r for r in rows if "wav_identical" in r]
    print(f"\n{same}/{len(d.cases)} cached answers identical; WAV bytes identical {sum(r['wav_identical'] for r in wav)}/{len(wav)}")
    if a.out:
        json.dump({"cases": d.cases, "rows": rows}, open(a.out, "w"), indent=1)
    return 0 if same == len(d.cases) and all(r["wav_identical"] for r in wav) else 1


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("cmd", choices=["prepare", "run", "limits", "shadow"])
    p.add_argument("--render-dir")
    p.add_argument("--sleep", type=float, default=2.0)
    p.add_argument("dir", nargs="?")
    p.add_argument("--go", default="127.0.0.1:18201")
    p.add_argument("--rs", default="127.0.0.1:18202")
    p.add_argument("--btwd", default="btwd")
    p.add_argument("--store")
    p.add_argument("--examples")
    p.add_argument("--limit", type=int, default=0)
    p.add_argument("--jobs", type=int, default=2, help="programs in flight at once (keep <= -render-concurrency)")
    p.add_argument("--snr", type=int, default=6, help="programs whose Opus is decoded and compared to the WAV")
    p.add_argument("--out")
    a = p.parse_args()
    if a.cmd == "prepare":
        prepare(a.dir, a.btwd)
    elif a.cmd == "run":
        sys.exit(run(a))
    elif a.cmd == "shadow":
        sys.exit(shadow(a))
    else:
        sys.exit(limits(a))
