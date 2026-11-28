# Pairing a phone or a booth with the BeatScript site (`btw-pair/2`)

The BeatScript Android app (beatscript-app, formerly djandroid; `app.beatscript`) connects to the BeatScript site by scanning a QR
code. Once paired, the site (and an assistant over its MCP) can send BeatScript programs to the phone and
play them there, and the site sees what the phone plays. Server: beatscript-serve `crates/beatscript-serve/src/hub/pair.rs` in this repo (ported from the retired Go `internal/site/server/pair.go`).
Phone: djandroid `rust/src/btwpair.rs` (the protocol core, `network` feature only) and
`app/src/network/java/.../btw/BtwPairingImpl.kt` (the transport). The phone's side is in djandroid
`docs/BTW-PAIR.md`.

## The flow

```
page (signed in)              server                                   phone
POST /api/pair  ───────────▶  offer {pair id, token} (2 min, single use)
◀── QR: https://btw.bitwrap.io/pair#<pair id>.<token>
                                                         scan ◀────────
                              POST /api/pair/claim  ◀──────────────────  {proto, pair_id, token, key, name, ts, sig}
                              verify sig (Ed25519, key), token, expiry
                              device {id, owner = the page's user, key}  ──▶ {device_id, user, address, fingerprint}
GET /api/pair/<id> ── paired, the device's fingerprint (both ends show it)
                              GET /api/device/events (SSE)  ◀───────────  Authorization: BtwDevice <device>.<ts>.<sig>
                              hello, eval, load, stop, get, ping, unpaired ──▶
                              POST /api/device/report (≤ 1/s)  ◀────────  what plays (SessionState)
                              POST /api/device/compose  ◀──────────  {prompt, genre?, key?, length?} (signed)
                              compose as the owner ─────────────────▶  {program, cid, check, share_url, render_url}
POST /api/devices/<id>/push ─▶ eval | load | stop ──────────────────────▶
DELETE /api/devices/<id> ────▶ unpaired ────────────────────────────────▶  (or the phone: DELETE /api/device)
```

## Protocol versions and the site id

`btw-pair/2` (current) signs the site's **id**, not its host: `site: <site id>` where v1 has `host: <host>`. The id is
random, made once (`<data>/site.json`) and travels with the data directory, so the same site served from another
domain keeps every pairing. The phone learns the id from the offer answer (`site_id`), the claim answer, or
`GET <origin>/.well-known/btw-site`. A server verifies a request as v2 first, then as v1 under every host it answers
for (`-alt-origins`), so phones paired before v2 keep working while the old name is an alias. A claim names its
version in `proto`; both are accepted. Pins: `testdata/pair/vectors.json` (v1) and `vectors-v2.json`.

### A moved site

A server started with `-moved-to https://NEW` puts `moved_to` in `/.well-known/btw-site`, in the device hello, and
sends a `moved` event (`{moved_to, site_id, from}`) right after `hello` on every phone stream. The phone must fetch
`NEW/.well-known/btw-site`, check that `site_id` is the one it has stored, and only then switch its origin; it keeps its
device id and key and signs v2 as before, so nothing is re-scanned. A phone that was away finds out the same way when
it next connects (the old name redirects with 308 and keeps answering the well-known document). The runbook is
`site/deploy/MIGRATION.md`.

## What is signed

Both messages are UTF-8 text, signed with the phone's Ed25519 key (64-byte signature, base64url
without padding). Shown as v1; v2 is the same with `btw-pair/2` and `site: <site id>` in place of `host: <host>`
(`host` is the server's public host).

```
btw-pair/1 claim                      btw-pair/1 request
host: <host>                          host: <host>
pair: <pair id>                       device: <device id>
token: <hex sha256(token)>            method: <GET|POST|DELETE>
key: <base64url public key>           path: <URL path, no query>
name: <device name>                   ts: <unix ms>
ts: <unix ms>                         body: <hex sha256(body)>   (of the empty string when there is none)
```

`site/testdata/pair/vectors.json` pins both texts and signatures for a fixed key (seed = 32 bytes
of 0x01). `pair_test.go TestPairVectors` makes it, and djandroid's `btwpair.rs` tests reproduce it
byte for byte, so the two sides cannot drift.

`site/testdata/pair/qr-golden.json` pins one whole pairing QR payload as the site makes it
(`PairLink`, `https://beatscript.app/pair#<22>.<22>`, with `-` and `_` in the token) and the v1 claim a
phone signs for it. `pair_qr_golden_test.go` makes it and checks that a live offer's link is `PairLink`'s;
beatscript-app copies it to `rust/tests/btw_pair_qr_golden.json`, and its `btwpair.rs` test parses the payload
with the app's own parser and reproduces the claim byte for byte. `TestPairQRScanSingleUseExpiry` scans as the
phone does on a beatscript.app site: the claim is accepted once, the same claim again is `410 pair_used`, and a
code past two minutes is `410 pair_expired`, each telling the person to show a new code.

## A pairing code is not an invite (2026-11-19)

The app has two QR codes of its own kinds, and they are different credentials:

- **A pairing code** (this document): `https://<site>/pair#<id>.<token>`, made by the site's mobile panel. It
  binds the phone's device key to the signed-in account. In the app: Menu › beatscript.app › Scan to connect,
  or the phone's camera (the link opens the app).
- **An invite** (`bitwrap-invite.v1`, the app's wallet, Menu › Identity › Redeem an invite): a signed JSON
  credential one member gives another. The site never makes one.

Scanning the site's code from the invite scanner used to answer "That isn't a BeatScript invite code"; that was
the reported bug. The Play app was also built without the `network` feature, so it had no pairing scanner at all
and a camera-opened `/pair` link did nothing. beatscript-app `fix/pairing-qr` sends every scanned code to the
flow it belongs to (one router, whichever scanner read it), says what a code is when it cannot use it, and makes
the Play build a network build. One credential flow stays: pairing is the only way a phone gets a site
credential; an invite never becomes one.

## The host after the move (2026-11-19)

`host` in both signed texts is the server's public host. The public origin moved from btw.bitwrap.io to
**beatscript.app**: a new pairing code is `https://beatscript.app/pair#…` and signs `host: beatscript.app`. A phone
paired before the move keeps its origin (`https://btw.bitwrap.io`) and signs `host: btw.bitwrap.io`; the server
accepts every host in `-legacy-hosts` (btw.bitwrap.io), and nginx keeps `/api/` on btw.bitwrap.io proxied rather
than redirected (the phone does not follow redirects), so nobody re-pairs. djandroid still treats only btw.bitwrap.io
as the official host, so it asks "Connect?" before pairing with a beatscript.app code until it learns the new name.

## Rules

- **Nothing long-lived is in the QR.** The token is 128 random bits and works once, for two minutes.
  Any claim with a wrong token burns the offer. Only its SHA-256 is kept, in memory. It sits in the
  link's fragment, so a camera app that opens the link never sends it to the server (`GET /pair`
  explains that the code is for the BeatScript app's scanner).
- **No secret is stored on the server.** A device record (`<data>/devices.json`, 0600) is its id,
  its owner, its name and its public key. The phone's key never leaves the phone (djandroid keeps the
  seed in `noBackupFilesDir`).
- **Bound to the identity that showed the code.** The owner is the user signed in on the page (an
  eth wallet or a browser Ed25519 key, the site's sign-in). The phone does not need a wallet of its
  own. Both ends show the key's fingerprint (`sha256(key)[:4]` in hex, as `ab12 cd34`).
- **Signed requests.** The `ts` must be within ±2 min of the server's clock, and a signature already
  seen in that window is refused (a replay). A body that differs from the one signed fails.
- **Re-pair and unpair.** One record per key: a phone that pairs again (with the same account or
  another) replaces its old record, and the old stream gets `unpaired`. Unpairing from either side
  deletes the record. A phone whose record is gone gets `410 unpaired` and forgets the pairing.
  At most 8 phones per user (pairing a 9th drops the oldest) and 3 open offers per user (a 4th
  retires the oldest).
- **A phone is a hub session** owned by the user (`SessionInfo.device` names it), so MCP's
  `eval` / `load` / `stop` / `get` with no session reach it as well as the user's open pages.
- **Pairing grants MCP.** The owner's MCP token (the page's "MCP token" button) lists the owner's
  phones with the `devices` tool and drives one alone with `device: <id>` on `eval` / `load` /
  `get` / `stop`. No new credential is made at pairing and nothing about MCP is in the QR: the grant
  is the device record's owner. Another account's token gets `not_found`.
  `/api/devices/<id>/push` runs the same checks as MCP's `eval`: no program with errors, and no
  restricted or asset-using program, is sent. What is sent is the program formatted (`#!` header
  lines and comments kept, the CID the same), as MCP's `eval`, `load` and `push_private` send it.
- Limits: `POST /api/pair` and `POST /api/pair/claim` allow 20 per minute per address. Device
  streams count toward the 40 SSE streams per address, and reports share the hub's token bucket
  (10/s, burst 20).

## Compose from the phone

`POST /api/device/compose` is the web's `POST /api/compose` for a paired phone. It is signed like
every device request (the `btw-pair/1 request` text, so the body's SHA-256 is covered) and runs
compose **as the pairing owner**: the same per-user hourly and daily allowance (shared with the web
and MCP, counted against the owner's account), the per-address day limit, the kill switch and the
daily spend cap. Body `{prompt, genre?, key?, length?, bpm?, base_cid?, current?}` (`current`: the program in the phone's editor, sent with every prompt after the first; vetted like a `base_cid` program and given to the model to modify), unknown fields refused with a 400 that names the field.
The answer is the web route's JSON in one body (the web route does not stream): `program` (canonical
text) and `cid` to play on the phone, plus `check`, `attempts`, `share_url` and `render_url`. The
result is stored publicly (CC0) like any compose. No credential beyond the device key is involved.

### Capabilities (2026-11-20)

The claim answer and `GET /api/device/me` also carry `caps` (e.g. `["compose"]`) and `grant`, a signed
`btw-cap+jws` statement of the **account's** capabilities (its `sub` is the hello's `user`). The phone uses it only
to decide what to show, and refreshes it with `GET /api/auth/grant` (BtwDevice-signed). The server checks the
allowlist on `/api/device/compose` whatever the phone holds: a refusal is `403 capability_required`. Format, keys
and verification: [CAPABILITIES.md](CAPABILITIES.md).

## Share and fetch from the phone (CID-only links)

`PUT /api/shares/<cid>` is the short link's write side (DESIGN.md "Short links"). A paired phone signs it like every device
request and it runs **as the pairing owner**: JSON `{text, license?, forked_from?, credits?, name?}`, public programs only, counted
per account (`-share-hour`, 60) and per address. The answer carries `short_url` (`https://beatscript.app/p/<CID>`)
and `short_social_url`. Fetching by CID needs no signature at all:
`GET /api/programs/<cid>` is public (`{cid, text, meta}` with the rights), and the phone recomputes the CID from the text itself
before it plays (a store that answers another program under a CID is refused). A CID alone does not carry a program: a CID the
site has not stored is `404`; the self-sealing link is what works anywhere.

## The booth: a paired device of kind `booth` (2026-11-27)

The desktop DJ app (djapp, "the booth") pairs with the site exactly as a phone does: the same offer, the same
`btw-pair/2` claim, the same device record, the same signed SSE stream and the same sealed push. There is one
pairing and one push primitive; a local inbox on the booth is only its offline fallback (this replaces the drop
folder, project-datum 6661). What is new is a device **kind**, one event (`set`), and the contract of the read the
booth already may make: fetching a public program by CID (the resolver). Server: `crates/beatscript-serve/src/hub.rs`
and `hub/pair.rs`; booth: djrust-vevr-se djapp (R1b).

### Pairing a booth

A booth has no camera, so it takes the pairing link as text: the panel shows the offer's `url` (the same
`https://beatscript.app/pair#<pair id>.<token>` the QR code carries) under the code with a Copy button, and the person
pastes it into the booth.
The fragment carries the pair id and token as it does for a phone. Everything else about the offer (two minutes,
single use, a wrong token burns it, 3 open offers per user) is unchanged.

The claim names the kind:

```
POST /api/pair/claim   {proto: "btw-pair/2", pair_id, token, key, name, kind: "booth", ts, sig}
```

- `kind` is `"phone"` or `"booth"`. Absent means `"phone"`, so every phone in the field is unchanged. Any other
  value is `400 bad_request` (`kind must be phone or booth`).
- The kind is **signed**. For a booth the claim text gains one line after `name:`; for a phone it has none, so the
  phone's text (and `vectors-v2.json`) is byte for byte what it was:

  ```
  btw-pair/2 claim
  site: <site id>
  pair: <pair id>
  token: <hex sha256(token)>
  key: <base64url public key>
  name: <device name>
  kind: booth
  ts: <unix ms>
  ```

  A `btw-pair/1` claim cannot name a kind: `proto: "btw-pair/1"` with `kind: "booth"` is `400 bad_request`.
  `site/testdata/pair/vectors-v2.json` gains a `claim_booth` case (the same fixed key, seed 32 × 0x01).
- Signed requests (`btw-pair/2 request`) do not change: the kind lives in the device record, not in each request.
- The device record (`<data>/devices.json`) carries `"kind": "booth"` for a booth and no `kind` for a phone (an
  existing file reads back as it was written).
- Booths count toward the 8 devices per user. Re-pairing the same key replaces the record, as for a phone, and may
  change its kind.
- Every view of a device names its kind: the claim answer, `GET /api/pair/<id>` (`device.kind`), `GET /api/devices`,
  MCP `devices`, and the stream's `hello` (`kind`).

### What a booth receives

The booth opens the same stream: `GET /api/device/events` with `Authorization: BtwDevice <device>.<ts>.<sig>`.
It receives the phone's events, read as a booth reads them, plus `set`:

| event | data | the booth |
|---|---|---|
| `hello` | `{device_id, user, address, fingerprint, kind: "booth", events, programs, site_id, caps?, grant?, moved_to?}` | `programs` is the resolver template, `https://beatscript.app/api/programs/{cid}` |
| `eval` | `{type, text, cid, play}` | stores the program in its LiveStore under `cid` and plays it now |
| `load` | `{type, text, cid}` | stores the program in its LiveStore under `cid`; does not play |
| `set` | `{type: "set", text, cid, play, extra: {members: [{n, cid, set}]}}` | stores the set and plays it when `play` is true (see below) |
| `stop` | `{type}` | stops |
| `private` | `{type: "private", extra: <beatscript-seal envelope>}` | opens it with its key; the payload's `kind` is `eval`, `load` or `set`, and it is handled as that event |
| `get`, `ping`, `moved`, `unpaired` | as for a phone | as for a phone |

`text` is always the canonical, formatted program (`#!` header lines and comments kept) and `cid` its CID. The booth
recomputes the CID from `text` and drops an event whose CID does not match.

**A set** is a program: the text with a `set { … }` block, `play <cid>` an item a line (grammar.ebnf `set`), under
its own CID. Call 7 holds: a set is program plus take, never master audio. In the `set` event:

- `cid` is the set's own CID and `text` its text.
- `extra.members` lists the CIDs the set's `play` lines name, in order, each once: `n` is the position of its first
  `play`, `set` is true when that member is itself a set (the booth resolves its members the same way, recursively).
  A `play` of a generator call has no CID and is not a member.
- The server sends a `set` only after the set checks **against this site's store**, so every member of a public set
  push is a program the resolver answers. Nothing else is guaranteed: the booth resolves each member, in this order,
  and fails loudly naming the CID when all three miss:
  1. its own LiveStore;
  2. the paired site's resolver, `GET <origin>/api/programs/<cid>` (public CC programs only, read-only);
  3. the offline inbox (`~/Workspace/booth-inbox/` or wherever R1b puts it).

**Private sets and private members** never travel in the clear and are never stored on the server. They go by
sealed push (`push_private`, the `private` event): each private member first, sealed with payload kind `load` (store,
do not play), then the set, sealed with payload kind `set`. The booth stores every opened `load` before it resolves
the set, so the set's members are local by the time it plays. The server keeps nothing of either (site/PAIRING.md
"private relay", `hub/private.rs`). A sealed `set` payload's `members` are not given by the server (it cannot read
the set); the booth reads them from the set's text.

### Pushing to a booth

```
POST /api/devices/<id>/push   {type: "eval" | "load" | "stop" | "set", text?, play?}
```

- Signed in as the owner (the site's session or the MCP token), as today; another account's device is `404`.
- `type: "set"` is for a booth only: a phone is `422 not_a_booth` (`<name> is a phone; a set goes to a booth`).
- A set must check here: no errors, not `use restricted`, no assets (the same gate as `eval`), and it must be a set
  (`422 not_a_set` when the text has no `set` block). A member this site does not hold fails the check
  (`422 bad_request`, the checker's message names the CID): push it first, or send the set sealed.
- The answer: `{cid, delivered: 1, members: [<cid>, …]}`. A booth that is not connected is `409 no_session`, and
  nothing is queued.
- `eval` and `load` to a booth are as for a phone: the program is checked and formatted, and a restricted or
  asset-using program is refused (send it sealed).
- Limits: the push runs the checker, charged to the caller's address like every check (60 a minute); the stream
  counts toward the 40 SSE streams per address.

MCP: `eval` / `load` with `device: <booth id>` send `eval` / `load`; a set text sent with `eval` or `load` to a booth
goes as a `set` event (to a phone it is refused, as above). `push_private` takes `set: true` to seal a set (payload
kind `set`); `beatscript-seal` accepts `set` as a payload kind.

### The resolver: `GET /api/programs/<cid>`

This is the site's existing public read; the booth uses it as it is, and this is its contract.

- **No auth**, no signature, no cookie. Read-only: the booth never uploads (call 2).
- **200** `application/json`: `{cid, text, meta}`, `meta.license` (`CC0-1.0`, the default, or a `CC-BY…` id, with
  `meta.credits` when given), `meta.visibility: "public"`, and `renamed_to` / `renamed_from` for a BeatScript 1 / 2
  twin. `GET /api/programs/<cid>.beatscript` answers the text alone (`text/beatscript`).
- **Cache**: `Cache-Control: public, max-age=31536000, immutable`, `ETag: "<cid>"`, `304` on `If-None-Match`. A CID
  is its text, so a booth that holds one never asks again.
- **Linked data**: the response's `Link` header names the program's JSON-LD, `</p/<cid>.jsonld>; rel="describedby";
  type="application/ld+json"` (the `Program` document in the BeatScript context, `@id` `https://beatscript.app/p/<cid>`).
  `GET /p/<cid>` with `Accept: application/ld+json` answers the same document.
- **404** `{"error":"not_found"}` for a CID this site does not hold, and for anything that is not a CID. That covers
  every private, restricted, asset-using or non-Creative-Commons program: the storage gate (`gate.rs`) never stores
  one, so there is nothing to refuse with a 403 and the server never says whether such a program exists. A 404 is not
  cached as immutable (a program pushed later answers at once).
- The booth checks `cid` against the text it got (as the phone does) before it plays.
- Discovery: `programs` in the booth's `hello` and in `/.well-known/btw-site` (`<origin>/api/programs/{cid}`).

## Not done (decide)

- The phone does not countersign with its own did:pkh wallet identity. That would need a biometric
  prompt at pairing; the server would accept an optional `wallet` field on the claim.
- An Android App Link for `https://<site>/pair` (opening the app straight from the camera)
  needs `/.well-known/assetlinks.json` with the release key's fingerprint.

## Privacy

A program composed from a paired phone is stored publicly under CC0-1.0 like any other, and the answer says so (`notice`,
`license`). The prompt goes to a third-party model. See `/privacy`.
