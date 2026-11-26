# Android App Links for beatscript.app

The Share panel's QR holds `https://beatscript.app/p#<CID>.<packed>`. The BeatScript app (package `app.beatscript`)
claims `https://beatscript.app/p` and opens that form with no network, verifying the CID. On Android 12+ a link opens
in the app only if the host proves the app is its own, by serving `/.well-known/assetlinks.json`; without it the
phone's camera opens the browser (the page at `/p#...` still works there).

## Serving it

`beatscript-serve` serves the statement itself, from the binary: `crates/beatscript-serve/src/public/assetlinks.json`
is the one list of fingerprints in the repo (embedded with `include_bytes!`). Change the list there, with the test
`the_embedded_assetlinks_names_all_four_play_fingerprints`, and deploy.

- `GET /.well-known/assetlinks.json`: 200, `application/json`, never a redirect (the verifier does not follow one), on
  **every** Host the server is asked for. It does not look at the host, so any vhost nginx proxies here gets it.
- `-assetlinks-file` / `BEATSCRIPT_ASSETLINKS_FILE` is an explicit override (read on each request, refused at start if it
  is not JSON). **Leave it unset in production**: a file on the host would shadow the repo's list, which is how Play came
  to report "Domain ownership not verified" (the host file had one fingerprint).

## Expected content

`sha256_cert_fingerprints` lists every key an install can be signed with, as Play Console generated it for
`app.beatscript` (handle_all_urls):

| # | Fingerprint (SHA-256) | Which key |
|---|---|---|
| 1 | `B6:64:E3:34:FB:56:4F:12:C0:0B:74:38:94:1E:99:1A:64:5A:DF:47:26:AD:BB:18:CF:01:35:2E:55:8A:8D:47` | Direct download (CDN) build, release key, CN=djandroid |
| 2 | `36:62:1E:BF:4F:FE:6A:1B:1B:AB:FC:97:7A:12:7A:52:1E:C0:E3:A7:63:0F:B4:83:C6:AF:9B:5C:BD:A4:BF:C9` | Play Console's list |
| 3 | `4F:89:F0:5F:0C:50:33:EE:8B:F6:D7:65:84:79:46:1A:AA:9F:D1:5D:A9:97:CA:57:14:14:53:FC:67:99:E1:B9` | Play Console's list |
| 4 | `9E:15:70:3B:51:6C:03:2D:3F:EC:66:40:CA:B0:4E:4D:52:DA:2B:89:9E:D2:1B:C0:6A:07:EB:B7:54:33:AE:6B` | Play Console's list |

## Hosts

The app claims three hosts and each must serve the file itself with a 200 (Play verifies each domain separately):

| Host | Served by | How |
|---|---|---|
| `beatscript.app` | nginx -> `btw_front` (beatscript-serve) | the static-file regex location proxies `.json` |
| `btw.bitwrap.io` | nginx -> `btw_front` | `include snippets/beatscript-assetlinks.conf`: an exact-match location above the `location /` that 301s every page |
| `beats.bitwrap.io` | nginx (the beats-bitwrap-io app on :8089 has no such route) | the same include: an exact-match location proxying to `btw_front` |

`nginx/assetlinks.snippet.conf` is the snippet. `site/deploy/deploy.sh nginx-assetlinks` installs it, reinstalls the
btw.bitwrap.io vhost (`nginx/btw.bitwrap.io.redirect.conf` includes it) and adds the one include line to
beats.bitwrap.io's vhost (certbot owns that file, so it is edited in place, once; backups `*.bak.assetlinks-<stamp>`),
`nginx -t`, reload, and restores both files if `nginx -t` fails.

## Check

```
site/deploy/deploy.sh verify-assetlinks                          # the three hosts: 200, application/json, 4 fingerprints
curl -sI https://beatscript.app/.well-known/assetlinks.json     # 200, content-type: application/json
curl -s "https://digitalassetlinks.googleapis.com/v1/statements:list?source.web.site=https://beatscript.app&relation=delegate_permission/common.handle_all_urls"
adb shell pm get-app-links app.beatscript                        # beatscript.app: verified
```
