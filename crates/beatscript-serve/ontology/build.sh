#!/usr/bin/env bash
# build.sh: build the BeatScript ontology release (release/ and release.lock) from its three inputs, then check it.
#
#   upstream/bitwrap-0.2.0/ns.nt   the bitwrap ontology 0.2.0 the fork derives from (vendored by sync.sh upstream)
#   fork.ttl                       the fork's header and the terms it adds by hand
#   generated/beatscript.ttl       regenerated here: beatscript-serve's vocab dump (src/ontology/vocab.rs) through
#                                  tools/ontology/beatscript.py
#
# src/ontology/fork.rs does the rest (re-homing, derivation links, the four serializations, the closure and its
# certificate, the pack, the manifest). Deterministic: the same inputs give the same bytes, and `cargo test` fails when
# the vendored release is not what they build (ontology::tests::fork_integrity) or when the vocabulary and the code
# disagree (ontology::vocab::tests::ontology_matches_beatscript_serve). After a change to the grammar, the instruments, the
# control registry or either MCP server's tools: run this, commit release/, release.lock and generated/ with it.
# A new release is a new version: bump owl:versionInfo / owl:versionIRI / dct:issued in fork.ttl first.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
cd "$repo"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0} CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}

ONTOLOGY_VOCAB_OUT="$tmp/sources.json" cargo test -q -p beatscript-serve --bin beatscript-serve vocab_dump
python3 -I tools/ontology/beatscript.py "$tmp/sources.json" "$here" > "$tmp/beatscript.ttl"
mv "$tmp/beatscript.ttl" "$here/generated/beatscript.ttl"
ONTOLOGY_FORK_OUT="$here/release" cargo test -q -p beatscript-serve --bin beatscript-serve fork_write
"$here/sync.sh" check
