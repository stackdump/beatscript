#!/usr/bin/env python3
"""End-to-end check of the MCP OAuth 2.1 flow and the capability grant, over HTTP, against a local beatscript-serve.

    python3 bench/oauth_e2e.py [--bin target/debug/beatscript-serve] [--web <btwsite/web>] [--base http://127.0.0.1:PORT]

Without --base it starts its own beatscript-serve (-public-url http://127.0.0.1:PORT, a throwaway -data dir, BEATSCRIPT_ADMIN_TOKEN
set) and stops it at the end. With --base it drives a server you started (set ADMIN_TOKEN in the environment for the
caps-admin steps). Needs Python `cryptography` (Ed25519). Exit status 0 only if every check passed.

What it covers: discovery (RFC 9728, RFC 8414), dynamic client registration (RFC 7591) and its refusals, the consent
page, sign-in over /api/auth/challenge + /login with an Ed25519 key, the consent decision (a bearer token cannot
approve), the code exchange (a wrong PKCE verifier and a wrong redirect are refused, a code works once), the bto_ token
at /mcp (works for an auth tool, 401 with WWW-Authenticate without or with a bad token, refused at /api/compose and
/api/auth/grant), refresh rotation and reuse detection (reuse revokes the grant), revocation, a legacy btw_ token
still working, and the btw-cap+jws grant verifying against /.well-known/btw-capabilities.json.
"""
import argparse, base64, hashlib, json, os, secrets, shutil, socket, subprocess, sys, tempfile, time
import urllib.error, urllib.parse, urllib.request

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives import serialization

FAILS = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else f"   [{detail}]"))
    if not ok:
        FAILS.append(name)
    return ok


def b64u(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def unb64u(s):
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


class Client:
    def __init__(self, base):
        self.base = base.rstrip("/")

    def req(self, method, path, body=None, headers=None, form=None, raw=None):
        h = {"Origin": self.base}
        h.update(headers or {})
        data = None
        if form is not None:
            data = urllib.parse.urlencode(form).encode()
            h["Content-Type"] = "application/x-www-form-urlencoded"
        elif body is not None:
            data = json.dumps(body).encode()
            h["Content-Type"] = "application/json"
        elif raw is not None:
            data = raw
        r = urllib.request.Request(self.base + path, data=data, method=method, headers=h)
        try:
            resp = OPENER.open(r, timeout=20)
        except urllib.error.HTTPError as e:
            resp = e
        b = resp.read()
        try:
            j = json.loads(b)
        except Exception:
            j = None
        return resp.status, dict((k.lower(), v) for k, v in resp.headers.items()), b, j


def pkce():
    v = b64u(secrets.token_bytes(32))
    return v, b64u(hashlib.sha256(v.encode()).digest())


def mcp_call(c, token, name, args=None, rid=1):
    h = {"Accept": "application/json, text/event-stream"}
    if token:
        h["Authorization"] = "Bearer " + token
    return c.req("POST", "/mcp", {"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                                  "params": {"name": name, "arguments": args or {}}}, h)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start_server(binp, web):
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    data = tempfile.mkdtemp(prefix="btw-oauth-e2e-")
    env = dict(os.environ, BEATSCRIPT_ADMIN_TOKEN="admin-e2e-token")
    args = [binp, "-addr", f"127.0.0.1:{port}", "-public-url", base, "-data", data, "-model", "fake"]
    if web:
        args += ["-web", web, "-docs", os.path.join(web, "docs")]
    log = open(os.path.join(data, "server.log"), "w")
    p = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(100):
        try:
            socket.create_connection(("127.0.0.1", port), 0.2).close()
            return p, base, data
        except OSError:
            if p.poll() is not None:
                break
            time.sleep(0.1)
    p.kill()
    print(open(os.path.join(data, "server.log")).read())
    sys.exit("server did not start")


class Wallet:
    """An Ed25519 identity, as the browser keeps one: signs the sign-in challenge."""

    def __init__(self):
        self.sk = Ed25519PrivateKey.generate()
        self.pub = self.sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
        self.uid = "ed25519:" + self.pub

    def login(self, c):
        _, _, _, ch = c.req("GET", "/api/auth/challenge")
        sig = self.sk.sign(ch["message"].encode()).hex()
        st, h, _, j = c.req("POST", "/api/auth/login", {"type": "ed25519", "address": self.pub, "signature": sig, "nonce": ch["nonce"]})
        ck = h.get("set-cookie", "").split(";")[0]
        return st, ck, j


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(os.path.dirname(__file__), "../../../target/debug/beatscript-serve"))
    ap.add_argument("--web", default=os.environ.get("BEATSCRIPT_WEB", ""))
    ap.add_argument("--base", default="")
    a = ap.parse_args()
    proc = data = None
    if a.base:
        base = a.base
        admin = os.environ.get("ADMIN_TOKEN", "")
    else:
        proc, base, data = start_server(os.path.abspath(a.bin), a.web)
        admin = "admin-e2e-token"
    try:
        run(base, admin)
    finally:
        if proc:
            proc.terminate()
            proc.wait(5)
            if FAILS:
                print("server log:", os.path.join(data, "server.log"))
            else:
                shutil.rmtree(data, ignore_errors=True)
    print(f"\n{len(FAILS)} failed" if FAILS else "\nall checks passed")
    sys.exit(1 if FAILS else 0)


def run(base, admin):
    c = Client(base)
    redirect = "http://127.0.0.1:53682/callback"

    # ---- discovery
    st, h, _, prm = c.req("GET", "/.well-known/oauth-protected-resource")
    check("protected-resource metadata", st == 200 and prm["resource"] == base + "/mcp" and base in prm["authorization_servers"][0], prm)
    st, _, _, prm2 = c.req("GET", "/.well-known/oauth-protected-resource/mcp")
    check("protected-resource metadata (path-suffixed)", st == 200 and prm2 == prm)
    st, _, _, asm = c.req("GET", "/.well-known/oauth-authorization-server")
    ok = st == 200 and asm.get("code_challenge_methods_supported") == ["S256"] and asm["token_endpoint"] == base + "/oauth/token" \
        and asm["registration_endpoint"] == base + "/oauth/register" and "none" in asm["token_endpoint_auth_methods_supported"]
    check("authorization-server metadata", ok, asm)
    st, h, _, _ = c.req("OPTIONS", "/oauth/token", headers={"Access-Control-Request-Method": "POST", "Origin": "https://claude.ai"})
    check("CORS preflight on the token endpoint", st in (200, 204) and "access-control-allow-origin" in h, (st, h))

    # ---- registration (RFC 7591) and its refusals
    st, _, _, reg = c.req("POST", "/oauth/register", {"client_name": "e2e client", "redirect_uris": [redirect]})
    check("register", st in (200, 201) and reg["client_id"].startswith("btwc_") and reg["token_endpoint_auth_method"] == "none", reg)
    cid = reg["client_id"]
    st, _, _, j = c.req("POST", "/oauth/register", {"client_name": "x", "redirect_uris": ["http://evil.example/cb"]})
    check("register refuses plain http to a non-loopback host", st == 400 and j["error"] == "invalid_redirect_uri", (st, j))
    st, _, _, j = c.req("POST", "/oauth/register", {"client_name": "x", "redirect_uris": [redirect], "grant_types": ["implicit"]})
    check("register refuses an unknown grant type", st == 400, (st, j))
    st, _, _, j = c.req("POST", "/oauth/register", {"client_name": "x", "redirect_uris": ["https://x.example/cb#frag"]})
    check("register refuses a fragment", st == 400, (st, j))

    # ---- authorize: GET (anonymous), then sign in, then GET again and decide
    ver, chal = pkce()
    q = {"response_type": "code", "client_id": cid, "redirect_uri": redirect, "code_challenge": chal, "code_challenge_method": "S256",
         "state": "st-123", "resource": base + "/mcp", "scope": "mcp"}
    qs = urllib.parse.urlencode(q)
    st, h, body, _ = c.req("GET", "/oauth/authorize?" + qs)
    page = body.decode()
    check("consent page (anonymous): asks to sign in", st == 200 and "text/html" in h["content-type"] and "signin-key" in page and "e2e client" in page, (st, page[:200]))
    st, h, body, _ = c.req("GET", "/oauth/authorize?" + urllib.parse.urlencode(dict(q, redirect_uri="http://127.0.0.1:1/other")))
    check("authorize: an unregistered redirect_uri gets an error page, not a redirect", st == 200 and "location" not in h and "Cannot connect" in body.decode(), (st, h))
    st, h, _, _ = c.req("GET", "/oauth/authorize?" + urllib.parse.urlencode(dict(q, code_challenge_method="plain")))
    check("authorize: PKCE plain is refused (redirected with an error)", st == 302 and "error=" in h["location"], (st, h))
    st, h, _, _ = c.req("GET", "/oauth/authorize?" + urllib.parse.urlencode({k: v for k, v in q.items() if k != "code_challenge"}))
    check("authorize: a missing code_challenge is refused", st == 302 and "error=" in h["location"], (st, h))
    st, h, body, _ = c.req("GET", "/oauth/authorize?" + urllib.parse.urlencode(dict(q, resource="https://other.example/mcp")))
    check("authorize: a resource other than this server's /mcp is refused", (st == 302 and "error=" in h.get("location", "")) or "Cannot connect" in body.decode(), (st, h))

    w = Wallet()
    st, cookie, j = w.login(c)
    check("sign in with an ed25519 key", st == 200 and cookie.startswith("btw_session=") and j["user"]["id"] == w.uid, (st, j))
    ck = {"Cookie": cookie}
    st, _, body, _ = c.req("GET", "/oauth/authorize?" + qs, headers=ck)
    page = body.decode()
    import re
    m = re.search(r'id="approve" class="primary" data-req="([^"]+)" data-consent="([^"]+)"', page)
    check("consent page (signed in): Allow button with a sealed request", st == 200 and bool(m) and w.pub[:6] in page, page[:300])
    sealed, consent = m.group(1), m.group(2)

    # decision refusals
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": consent, "approve": True})
    check("decision without the cookie is refused", st == 401, (st, j))
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": consent, "approve": True}, headers=dict(ck, Origin="https://evil.example"))
    check("decision from another site is refused", st == 403, (st, j))
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": "0" * len(consent), "approve": True}, headers=ck)
    check("decision with a wrong consent MAC is refused", st >= 400, (st, j))
    other = Wallet()
    _, ock, _ = other.login(c)
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": consent, "approve": True}, headers={"Cookie": ock})
    check("decision by a different account than the one the page was shown to is refused", st >= 400, (st, j))

    # deny path
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": consent, "approve": False}, headers=ck)
    loc = (j or {}).get("redirect", "")
    check("deny redirects back with error=access_denied and the state", st == 200 and "error=access_denied" in loc and "state=st-123" in loc, (st, j))

    # approve
    st, _, _, j = c.req("POST", "/oauth/authorize", {"req": sealed, "consent": consent, "approve": True}, headers=ck)
    loc = (j or {}).get("redirect", "")
    pr = urllib.parse.urlparse(loc)
    qd = urllib.parse.parse_qs(pr.query)
    check("approve redirects to the registered URI with code and state", st == 200 and loc.startswith(redirect) and qd.get("state") == ["st-123"] and "code" in qd, (st, j))
    code = qd["code"][0]
    iss_ok = qd.get("iss", [base])[0] == base
    check("redirect carries iss = this server (RFC 9207) when present", iss_ok, qd)

    # ---- token exchange
    def tok(**f):
        return c.req("POST", "/oauth/token", form=f)
    base_form = {"grant_type": "authorization_code", "client_id": cid, "redirect_uri": redirect, "resource": base + "/mcp"}
    st, _, _, j = tok(**base_form, code=code, code_verifier=b64u(secrets.token_bytes(32)))
    check("token: a wrong PKCE verifier is refused (invalid_grant)", st == 400 and j["error"] == "invalid_grant", (st, j))
    # the code may be burned by the failed attempt; take a fresh one for the rest
    def fresh_code(verifier_challenge, wallet_cookie=ck, resource=base + "/mcp"):
        qq = dict(q, code_challenge=verifier_challenge, resource=resource)
        _, _, body, _ = c.req("GET", "/oauth/authorize?" + urllib.parse.urlencode(qq), headers=wallet_cookie)
        mm = re.search(r'id="approve" class="primary" data-req="([^"]+)" data-consent="([^"]+)"', body.decode())
        _, _, _, jj = c.req("POST", "/oauth/authorize", {"req": mm.group(1), "consent": mm.group(2), "approve": True}, headers=wallet_cookie)
        return urllib.parse.parse_qs(urllib.parse.urlparse(jj["redirect"]).query)["code"][0]
    v2, c2 = pkce()
    code2 = fresh_code(c2)
    st, _, _, j = tok(**dict(base_form, redirect_uri="http://127.0.0.1:1/other"), code=code2, code_verifier=v2)
    check("token: a different redirect_uri is refused", st == 400 and j["error"] == "invalid_grant", (st, j))
    code2 = fresh_code(c2)
    st, h, _, tk = tok(**base_form, code=code2, code_verifier=v2)
    check("token: the right verifier works", st == 200 and tk["access_token"].startswith("bto_") and tk["refresh_token"].startswith("btr_") and tk["token_type"].lower() == "bearer" and 0 < tk["expires_in"] <= 3600, (st, tk))
    check("token: responses are not cacheable", "no-store" in h.get("cache-control", ""), h)
    access, refresh = tk["access_token"], tk["refresh_token"]
    st, _, _, j = tok(**base_form, code=code2, code_verifier=v2)
    check("token: an authorization code works once", st == 400 and j["error"] == "invalid_grant", (st, j))

    # ---- the bto_ token
    st, h, _, j = mcp_call(c, access, "devices")
    check("bto_ token: /mcp tools/call (devices, an auth tool) is accepted", st == 200 and "error" not in (j or {}) or (st == 200 and j.get("result") is not None), (st, j))
    st, h, _, j = mcp_call(c, None, "devices")
    check("no token: an auth tool at /mcp is 401 with WWW-Authenticate resource_metadata", st == 401 and "resource_metadata=" in h.get("www-authenticate", ""), (st, h))
    st, h, _, j = mcp_call(c, "bto_garbage", "devices")
    check("bad bto_ token: /mcp is 401 with WWW-Authenticate error=invalid_token", st == 401 and "invalid_token" in h.get("www-authenticate", ""), (st, h))
    st, h, _, j = c.req("POST", "/mcp", {"jsonrpc": "2.0", "id": 7, "method": "tools/list"}, {"Accept": "application/json, text/event-stream"})
    check("anonymous tools/list still works (discovery)", st == 200 and "tools" in j.get("result", {}), (st, j))
    st, _, _, j = c.req("POST", "/api/compose", {"prompt": "a calm techno loop"}, {"Authorization": "Bearer " + access})
    check("bto_ token is refused at /api/compose", st in (401, 403), (st, j))
    st, _, _, j = c.req("GET", "/api/auth/grant", headers={"Authorization": "Bearer " + access})
    check("bto_ token is refused at /api/auth/grant", st in (401, 403), (st, j))
    st, _, _, j = c.req("GET", "/api/auth/me", headers={"Authorization": "Bearer " + access})
    check("bto_ token does not identify at /api/auth/me", st == 200 and (j or {}).get("user") is None or st in (401, 403), (st, j))

    # the panel's list of connections
    st, _, _, j = c.req("GET", "/api/auth/clients", headers=ck)
    lst = j.get("clients", j) if isinstance(j, dict) else j
    check("GET /api/auth/clients lists the connection", st == 200 and any("e2e client" in json.dumps(x) for x in lst), (st, j))
    st, _, _, j = c.req("GET", "/api/auth/clients")
    check("GET /api/auth/clients needs a session", st in (401, 403), (st, j))

    # ---- refresh rotation
    rf = {"grant_type": "refresh_token", "client_id": cid, "refresh_token": refresh}
    st, _, _, t2 = tok(**rf)
    check("refresh: a new access and refresh token", st == 200 and t2["access_token"] != access and t2["refresh_token"] != refresh, (st, t2))
    st, _, _, j = mcp_call(c, t2["access_token"], "devices")
    check("refreshed access token works", st == 200, (st, j))
    st, _, _, j = tok(**rf)
    check("refresh: the old refresh token is refused when reused", st == 400 and j["error"] == "invalid_grant", (st, j))
    st, h, _, j = mcp_call(c, t2["access_token"], "devices")
    check("refresh reuse revokes the whole grant (the new access token stops working)", st == 401, (st, j))
    st, _, _, j = tok(**dict(rf, refresh_token=t2["refresh_token"]))
    check("refresh reuse revokes the new refresh token too", st == 400, (st, j))

    # ---- revoke (RFC 7009) and DELETE /api/auth/clients/<id>
    v3, c3 = pkce()
    code3 = fresh_code(c3)
    st, _, _, t3 = tok(**base_form, code=code3, code_verifier=v3)
    st, _, _, _ = mcp_call(c, t3["access_token"], "devices")
    ok1 = st == 200
    st, _, _, _ = c.req("POST", "/oauth/revoke", form={"token": t3["refresh_token"], "client_id": cid})
    check("revoke answers 200", st == 200, st)
    st, _, _, _ = mcp_call(c, t3["access_token"], "devices")
    check("revoked: the access token stops working", ok1 and st == 401, st)
    st, _, _, j = tok(**dict(rf, refresh_token=t3["refresh_token"]))
    check("revoked: the refresh token stops working", st == 400, (st, j))

    v4, c4 = pkce()
    st, _, _, t4 = tok(**base_form, code=fresh_code(c4), code_verifier=v4)
    st, _, _, lst = c.req("GET", "/api/auth/clients", headers=ck)
    items = lst.get("clients", lst) if isinstance(lst, dict) else lst
    gid = next((x.get("id") for x in items if x.get("id")), None)
    st, _, _, j = c.req("DELETE", f"/api/auth/clients/{gid}", headers=ck)
    check("DELETE /api/auth/clients/<id> disconnects", st == 200 and gid is not None, (st, j, items))
    st, _, _, _ = mcp_call(c, t4["access_token"], "devices")
    check("disconnected: the access token stops working", st == 401, st)

    # ---- legacy btw_ token still works at /mcp, and /api/auth/token revokes OAuth grants too
    st, _, _, j = c.req("POST", "/api/auth/token", headers=ck)
    legacy = (j or {}).get("token", "")
    check("create a legacy btw_ token", st in (200, 201) and legacy.startswith("btw_"), (st, j))
    st, _, _, j = mcp_call(c, legacy, "devices")
    check("legacy btw_ token works at /mcp", st == 200, (st, j))
    st, _, _, j = c.req("GET", "/api/auth/grant", headers={"Authorization": "Bearer " + legacy})
    check("a btw_ token may fetch the grant", st == 200 and "grant" in j, (st, j))
    v5, c5 = pkce()
    st, _, _, t5 = tok(**base_form, code=fresh_code(c5), code_verifier=v5)
    st, _, _, j = c.req("DELETE", "/api/auth/token", headers=ck)
    check("DELETE /api/auth/token reports the OAuth grants it revoked", st == 200 and j.get("oauth_revoked", 0) >= 1, (st, j))
    st, _, _, _ = mcp_call(c, t5["access_token"], "devices")
    check("after 'revoke all', the OAuth access token is dead", st == 401, st)
    st, _, _, _ = mcp_call(c, legacy, "devices")
    check("after 'revoke all', the legacy token is dead too", st == 401, st)

    # ---- the capability grant
    st, _, _, j = c.req("GET", "/api/auth/grant", headers=ck)
    check("GET /api/auth/grant (cookie)", st == 200 and set(j) >= {"caps", "exp", "grant"}, (st, j))
    st, _, _, me = c.req("GET", "/api/auth/me", headers=ck)
    check("/api/auth/me carries caps", st == 200 and "caps" in me, me)
    st, _, _, jw = c.req("GET", "/.well-known/btw-capabilities.json")
    check("capabilities key set is published", st == 200 and jw["keys"] and jw["keys"][0]["alg"] == "Ed25519", (st, jw))
    hdr, pay, sig = j["grant"].split(".")
    header, payload = json.loads(unb64u(hdr)), json.loads(unb64u(pay))
    key = next((k for k in jw["keys"] if k["kid"] == header.get("kid")), None)
    verified = False
    if key:
        try:
            Ed25519PublicKey.from_public_bytes(unb64u(key["pub"])).verify(unb64u(sig), (hdr + "." + pay).encode())
            verified = True
        except Exception as e:
            print("   verify error:", e)
    check("grant header is {alg:EdDSA, typ:btw-cap+jws} with a published kid", header.get("alg") == "EdDSA" and header.get("typ") == "btw-cap+jws" and key is not None, header)
    check("grant signature verifies against the published key", verified)
    now = time.time()
    check("grant payload: iss = public URL, sub = the account id, exp ~7 days", payload["iss"] == base and payload["sub"] == w.uid and 6 * 86400 < payload["exp"] - now <= 7 * 86400 + 60 and payload["exp"] == j["exp"], payload)
    tampered = pay[:-2] + ("AA" if pay[-2:] != "AA" else "BB")
    bad = False
    try:
        Ed25519PublicKey.from_public_bytes(unb64u(key["pub"])).verify(unb64u(sig), (hdr + "." + tampered).encode())
    except Exception:
        bad = True
    check("a tampered payload does not verify", bad)

    # ---- the allowlist: enforce on, an identity without compose cannot compose; with it, it can
    if admin:
        ah = {"Authorization": "Bearer " + admin}
        st, _, _, j = c.req("GET", "/api/admin/caps", headers=ah)
        check("admin: GET caps (default enforce false)", st == 200 and j.get("enforce") is False, (st, j))
        st, _, _, j = c.req("POST", "/api/admin/caps", {"identity": w.uid, "caps": ["compose"]}, ah)
        check("admin: grant compose to the account", st == 200 and j["identities"].get(w.uid) == ["compose"], (st, j))
        st, _, _, g1 = c.req("GET", "/api/auth/grant", headers=ck)
        check("the grant now carries compose", g1.get("caps") == ["compose"], g1)
        st, _, _, j = c.req("POST", "/api/admin/caps", {"enforce": True}, ah)
        check("admin: enforce on", st == 200 and j["enforce"] is True, (st, j))
        st, _, _, j = c.req("POST", "/api/compose", {"prompt": "a calm techno loop"}, ck)
        check("enforce on, account allowed: compose is not refused for capability", not (st == 403 and "capab" in json.dumps(j).lower()), (st, j))
        st, _, _, j = c.req("POST", "/api/admin/caps", {"identity": w.uid, "caps": []}, ah)
        st, _, _, j = c.req("POST", "/api/compose", {"prompt": "a calm techno loop"}, ck)
        check("enforce on, account removed: compose is refused (403)", st == 403, (st, j))
        c.req("POST", "/api/admin/caps", {"enforce": False}, ah)
    else:
        print("skip caps-admin steps (no ADMIN_TOKEN)")


if __name__ == "__main__":
    main()
