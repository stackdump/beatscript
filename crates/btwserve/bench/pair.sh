#!/bin/bash
# Start the Go btwsite server and btwserve side by side on the same inputs, for contract.py.
#
#   bench/pair.sh start <base-port> [extra btwserve flags...]   # Go on <base>, btwserve on <base>+1
#   bench/pair.sh stop  <base-port>
#
# Environment:
#   GO_BIN     the Go server binary        (default $HOME/.cache/btwref/btwsite)
#   BTWD       btwd for the Go server       (default $HOME/.cache/btwref/btwd)
#   RS_BIN     btwserve                     (default <repo>/target/release/btwserve)
#   REF        a beats-bitwrap-io checkout  (default ~/Workspace/.val-worktrees/beats-bitwrap-io/btwserve-ref)
#   WORK       where the data dirs go       (default ~/.cache/btwpair/<base>)
#   FALLBACK   set to 1 to give btwserve -fallback to the Go server (default: none, btwserve alone)
#
# Both servers get their own data dir, seeded with the same identity files (session.secret, site.json, a tokens.json
# with five MCP tokens for contract.py's compose and mcp groups), so a cookie or token minted by one verifies on the
# other and the site id is the same, and the same -assetlinks-file (BTW_ASSETLINKS_FILE; set it empty for none). Go
# reads tokens.json only at start, hence the seed. Compose uses Go's recorded fixture model on both (-model fake);
# BTW_ADMIN_TOKEN is "admin-test-token".
set -euo pipefail
cmd=${1:?start|stop}; base=${2:?base port}; shift 2 || true
here=$(cd "$(dirname "$0")" && pwd); repo=$(cd "$here/../../.." && pwd)
GO_BIN=${GO_BIN:-$HOME/.cache/btwref/btwsite}
BTWD=${BTWD:-$HOME/.cache/btwref/btwd}
RS_BIN=${RS_BIN:-${CARGO_TARGET_DIR:-$repo/target}/release/btwserve}
REF=${REF:-$HOME/Workspace/.val-worktrees/beats-bitwrap-io/btwserve-ref}
WORK=${WORK:-$HOME/.cache/btwpair/$base}
go_port=$base; rs_port=$((base + 1))

stop() {
  for f in "$WORK"/go.pid "$WORK"/rs.pid; do
    [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true
    rm -f "$f"
  done
}

if [ "$cmd" = stop ]; then stop; exit 0; fi
stop
rm -rf "$WORK"; mkdir -p "$WORK/go" "$WORK/rs"
secret=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
for d in go rs; do
  printf '%s\n' "$secret" > "$WORK/$d/session.secret"; chmod 600 "$WORK/$d/session.secret"
  printf '{"site_id":"site-contract-test-0001","created":"2026-11-20T00:00:00Z"}\n' > "$WORK/$d/site.json"
  # contract.py's MCP/compose tokens: btw_contracttesttoken000000000000000<N> is user ed25519:<N x 64>, N = 1..5
  printf '%s\n' '[{"hash":"478173122f4061671b92177d3f4d8b5a4fb4f9222085563b050b243000d50cb3","user":"ed25519:1111111111111111111111111111111111111111111111111111111111111111","created":"2026-11-20T00:00:00Z"},{"hash":"1a5c010f6ed6531efe8559215163edf1f6d67c7472878f7fac37fbd6632a2b2e","user":"ed25519:2222222222222222222222222222222222222222222222222222222222222222","created":"2026-11-20T00:00:00Z"},{"hash":"6b760dea679bbed0a9c0d873cd501e389854ccb7cd6f1c3ca03be4158d548d1f","user":"ed25519:3333333333333333333333333333333333333333333333333333333333333333","created":"2026-11-20T00:00:00Z"},{"hash":"4e3314ef19aac6ec2f9c2dd79dc863155169b200236db8fa459894e57566f5a2","user":"ed25519:4444444444444444444444444444444444444444444444444444444444444444","created":"2026-11-20T00:00:00Z"},{"hash":"1cef896b76060a9d6a710836281856d1edd6ffeddf89b855dc8ba55093a6789d","user":"ed25519:5555555555555555555555555555555555555555555555555555555555555555","created":"2026-11-20T00:00:00Z"}]' > "$WORK/$d/tokens.json"; chmod 600 "$WORK/$d/tokens.json"
done
common=(-public-url https://beatscript.app -web "$REF/btwsite/web" -examples "$REF/btwsite/examples" -docs "$REF/btwsite/web/docs"
        -render-max-bars 2400 -render-max-seconds 3600 -render-cache-mb 2048 -legacy-hosts btw.bitwrap.io -model fake)
printf '%s\n' '[{"relation":["delegate_permission/common.handle_all_urls"],"target":{"namespace":"android_app","package_name":"app.beatscript","sha256_cert_fingerprints":["AA:BB"]}}]' > "$WORK/assetlinks.json"
export BTW_ADMIN_TOKEN=admin-test-token GEMINI_API_KEY= BTW_SESSION_SECRET= BTW_ASSETLINKS_FILE=${BTW_ASSETLINKS_FILE-$WORK/assetlinks.json}
(cd "$REF" && exec "$GO_BIN" -addr "127.0.0.1:$go_port" -data "$WORK/go" -btwd "$BTWD" "${common[@]}" > "$WORK/go.log" 2>&1) &
echo $! > "$WORK/go.pid"
fb=(); [ "${FALLBACK:-}" = 1 ] && fb=(-fallback "http://127.0.0.1:$go_port")
(exec "$RS_BIN" -addr "127.0.0.1:$rs_port" -data "$WORK/rs" -log-dir "$WORK/rs/log" -docs-src "$REF/internal/btwsite/docs" \
      "${common[@]}" "${fb[@]}" "$@" > "$WORK/rs.log" 2>&1) &
echo $! > "$WORK/rs.pid"
for p in $go_port $rs_port; do
  for _ in $(seq 1 100); do curl -sf "http://127.0.0.1:$p/healthz" >/dev/null && break; sleep 0.2; done
  curl -sf "http://127.0.0.1:$p/healthz" >/dev/null || { echo "server on :$p did not start; see $WORK/*.log" >&2; tail -n 5 "$WORK"/*.log >&2; stop; exit 1; }
done
echo "go 127.0.0.1:$go_port  rs 127.0.0.1:$rs_port  work $WORK"
