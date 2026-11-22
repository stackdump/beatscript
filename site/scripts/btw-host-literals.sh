#!/usr/bin/env bash
# Fails when the site's web page names a production host outside a comment or the mock, or when the API reference
# names the legacy host. The page's host belongs in btwserve's -public-url, not in code: see site/deploy/MIGRATION.md.
# The API reference (site/docs-src/api.md, served at /docs/api.md) writes https://beatscript.app, which btwserve
# replaces with its -public-url as it serves the page; btw.bitwrap.io is only a legacy name for the same server.
set -euo pipefail
cd "$(dirname "$0")/../.."
bad=$(git grep -nE 'btw\.bitwrap\.io|beatscript\.app' -- 'site/web/js/*.js' 'site/web/index.html' \
  ':!*_test.go' | grep -vE '^[^:]+:[0-9]+:\s*(//|\*|/\*)' | grep -vE 'site/web/js/wasm-mock.js' |
  grep -vF 'beatscript.app › ' || true)   # the Android app's menu label ("beatscript.app › Scan to connect"), not a host
if [ -n "$bad" ]; then echo "host literal in code (use PublicURL):"; echo "$bad"; exit 1; fi
legacy=$(git grep -nE 'btw\.bitwrap\.io' -- 'site/docs-src/*.md' || true)
if [ -n "$legacy" ]; then echo "the legacy host in the API reference (write https://beatscript.app):"; echo "$legacy"; exit 1; fi
echo "no host literals"
