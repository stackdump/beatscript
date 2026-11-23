"""One cold render through a server: TTFB, total, bytes, CPU (server + reaped children), peak RSS (server + descendants).
usage: bench.py <url> <server-pid>   -> one JSON line"""
import json, os, sys, threading, time, http.client, urllib.parse
TICK = os.sysconf('SC_CLK_TCK'); PAGE = os.sysconf('SC_PAGE_SIZE')
def stat(pid):
    with open(f'/proc/{pid}/stat') as f: s = f.read()
    rest = s[s.rindex(')') + 2:].split()
    return {'ppid': int(rest[1]), 'cpu': sum(int(x) for x in rest[11:15]) / TICK, 'rss': int(rest[21]) * PAGE}
def tree(root):
    kids = {}
    for d in os.listdir('/proc'):
        if d.isdigit():
            try: kids.setdefault(stat(int(d))['ppid'], []).append(int(d))
            except Exception: pass
    out, todo = [], [root]
    while todo:
        p = todo.pop(); out.append(p); todo += kids.get(p, [])
    return out
url, pid = sys.argv[1], int(sys.argv[2])
base_rss = stat(pid)['rss']; cpu0 = stat(pid)['cpu']
peak = [0, 0]; done = threading.Event()
def sample():
    while not done.is_set():
        tot = 0; n = 0
        for p in tree(pid):
            try: tot += stat(p)['rss']; n += 1
            except Exception: pass
        if tot > peak[0]: peak[0] = tot
        peak[1] = max(peak[1], n - 1)
        time.sleep(0.01)
th = threading.Thread(target=sample); th.start()
u = urllib.parse.urlsplit(url)
t0 = time.perf_counter()
c = http.client.HTTPConnection(u.hostname, u.port, timeout=600)
c.request('GET', u.path + ('?' + u.query if u.query else ''))
r = c.getresponse()
first = r.read(1); ttfb = time.perf_counter() - t0
n = len(first)
while True:
    b = r.read(65536)
    if not b: break
    n += len(b)
total = time.perf_counter() - t0
time.sleep(0.5)  # let the server reap its children
done.set(); th.join()
cpu = stat(pid)['cpu'] - cpu0
print(json.dumps({'status': r.status, 'cache': r.getheader('X-Btw-Cache'), 'seconds': float(r.getheader('X-Btw-Seconds') or 0), 'ttfb_ms': round(ttfb * 1000, 1), 'total_s': round(total, 3),
                  'bytes': n, 'cpu_s': round(cpu, 2), 'peak_rss_mb': round(peak[0] / 2**20, 1), 'rss_delta_mb': round((peak[0] - base_rss) / 2**20, 1), 'children': peak[1]}))
