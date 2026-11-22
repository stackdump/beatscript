#!/usr/bin/env python3
# Cross-server check: an account session the GO server issues is the one btwserve trusts.
#
#   btwsite  -addr 127.0.0.1:18297 -data D -public-url http://127.0.0.1:18297 -btwd <btwd> &
#   btwserve -addr 127.0.0.1:18298 -data D -public-url http://127.0.0.1:18297 &
#   python3 -I crates/btwserve/bench/playlists_go_session.py
#
# Signs in to Go with a fresh Ed25519 key (the browser-key sign-in), uses Go's btw_session cookie on btwserve,
# mints a bearer token on Go and uses it on btwserve, revokes it on Go and expects 401 from btwserve.
# Needs python3 with `cryptography`. Run on 2026-11-19: every line passed.
import json, urllib.request, http.cookiejar
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
GO, RS = "http://127.0.0.1:18297", "http://127.0.0.1:18298"
k = Ed25519PrivateKey.generate()
pub = k.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
cj = http.cookiejar.CookieJar()
op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))
def call(method, url, body=None, headers={}):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method, headers={**({"Content-Type": "application/json"} if data else {}), **headers})
    try:
        with op.open(r) as resp: return resp.status, json.loads(resp.read() or b"null")
    except urllib.error.HTTPError as e: return e.code, json.loads(e.read() or b"null")
ch = call("GET", GO + "/api/auth/challenge")[1]
sig = k.sign(ch["message"].encode()).hex()
st, me = call("POST", GO + "/api/auth/login", {"type": "ed25519", "address": pub, "signature": sig, "nonce": ch["nonce"]})
print("go login", st, me["user"]["id"])
cookie = [c for c in cj if c.name == "btw_session"][0].value
# the cookie the Go server issued, sent to the Rust server (both on 127.0.0.1: the jar sends it to both ports)
st, p = call("POST", RS + "/api/playlists", {"name": "e2e"})
print("rust create with Go's cookie", st, p.get("owner") == me["user"]["id"])
st, tok = call("POST", GO + "/api/auth/token", {})
print("go mints token", st)
cj.clear()
st, lst = call("GET", RS + "/api/playlists", headers={"Authorization": "Bearer " + tok["token"]})
print("rust list with Go's bearer token", st, lst["account"] == me["user"]["id"], len(lst["playlists"]))
st, _ = call("DELETE", GO + "/api/auth/token", headers={"Authorization": "Bearer " + tok["token"]})
st, lst = call("GET", RS + "/api/playlists", headers={"Authorization": "Bearer " + tok["token"]})
print("after Go revokes the token, rust says", st)
assert st == 401, st
print("ok")
