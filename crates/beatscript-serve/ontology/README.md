# The BeatScript ontology

What `/ontology/` on beatscript.app serves (`src/ontology.rs`): the vocabulary behind BeatScript as Turtle,
JSON-LD, N-Triples and RDF/XML, a read-only SPARQL endpoint and an MCP server, CC0.

It is the **BeatScript ontology**, version 0.1.0 (`owl:versionIRI https://beatscript.app/ontology/0.1.0`),
namespace `https://beatscript.app/ontology/ns#`. It was **derived from the bitwrap ontology 0.2.0**
(ontology.bitwrap.io, CC0) and **evolves on its own** from here. ontology.bitwrap.io and its repository are not
touched by anything in this directory, and nothing here is fetched from them, at build or at serve time.

## What is in a release

A release is built from three inputs:

| Input | What |
|---|---|
| `upstream/bitwrap-0.2.0/ns.nt` (+ `manifest.json`, pinned by `upstream.lock`) | the bitwrap 0.2.0 release, vendored by `sync.sh upstream` |
| `fork.ttl` | the fork's own module: its `owl:Ontology` header (title, version, version IRI, issued, creator, licence, `prov:wasDerivedFrom` the bitwrap release) and the few terms it adds by hand (the products its MCP tools belong to) |
| `generated/beatscript.ttl` | the BeatScript vocabulary, generated from beatscript-serve's own code: every production and keyword of `crates/djlive/grammar.ebnf`, the voices, kits, families and palettes (`beatscript-api::instruments_json()`), the control registry's nouns and verbs, djlive's generators, transforms and evolve and mutation operators, and both MCP servers' tools |

`src/ontology/fork.rs` assembles them:

- **Re-homing.** Every bitwrap IRI under `https://ontology.bitwrap.io/` moves under `https://beatscript.app/ontology/`:
  `…/ns#Genre` becomes `https://beatscript.app/ontology/ns#Genre`, `…/genre/techno` becomes
  `https://beatscript.app/ontology/genre/techno`. bitwrap's own header is dropped; the fork has its own.
- **Derivation links.** Each term that came from bitwrap gets exactly one `prov:wasDerivedFrom` to the bitwrap IRI
  it came from, and the ontology itself is `prov:wasDerivedFrom <https://ontology.bitwrap.io/release/bitwrap-ontology-0.2.0.tar>`
  (bitwrap 0.2.0 states no `owl:versionIRI`, and its header's `owl:versionInfo` still reads 0.1.0, so its versioned
  release file, as its VoID names it, is the IRI that names 0.2.0). `prov:wasDerivedFrom` was chosen over
  `owl:equivalentClass` / `owl:equivalentProperty` / `owl:sameAs` because it says where a term came from without
  saying the two must stay the same: reasoning over the fork never pulls bitwrap's IRIs in, and the fork can change a
  term without contradicting its upstream. These links are the only bitwrap IRIs in the release.
- **Serializations.** `ns.nt` (sorted), `ns.ttl`, `ns.jsonld`, `ns.rdf`: one graph, the asserted ontology.
- **The certificate and the pack.** The closure under the RDFS and OWL 2 RL schema rules (the `owl-rl` profile of
  open-ontologies, the engine bitwrap releases with) with one derivation step per inferred triple, in
  open-ontologies' `oo-cert/1` format (`cert/asserted.tsv`, `asserted.sha256`, `derivations.tsv`, `scope.tsv`), and
  `pack.json`: the materialized graph and the certificate, bound by open-ontologies' `oo-pack-content/1` digest. Run
  on bitwrap 0.2.0 itself, the same closure gives exactly that release's pack graph, its certificate replays here,
  and the digest is its `content_sha256` (`fork::tests::upstream_pack_reproduced`).
- **`manifest.json`.** Name, version, version IRI, what it was derived from (with bitwrap's pack digest and the
  sha256 of the `ns.nt` it was built from), the pack's digest and size, the licence, and the sha256 of every file.

The build is deterministic: the same inputs give the same bytes.

## Changing it

```
crates/beatscript-serve/ontology/build.sh      # dump beatscript-serve's vocabulary, regenerate generated/, build release/, lock, check
crates/beatscript-serve/ontology/sync.sh check # every file against release.lock, manifest.json and upstream.lock (offline)
```

- A change to the grammar, the instruments, the control registry or either MCP server's tools changes the
  vocabulary: run `build.sh` and commit `generated/`, `release/` and `release.lock` with the change.
  `ontology::vocab::tests::ontology_matches_beatscript_serve` fails until you do.
- A change to the ontology's own terms goes in `fork.ttl` (never in `upstream/`), then `build.sh`. A release that
  changes what the ontology says is a new version: bump `owl:versionInfo`, `owl:versionIRI` and `dct:issued` first.
- Merging a later bitwrap release in is deliberate: `sync.sh upstream REF` vendors it beside 0.2.0, then
  `src/ontology/fork.rs` is pointed at it and the commit says why.
- `fixtures.py` records the ontology MCP's answers from a running beatscript-serve into `fixtures/mcp-<version>.json`
  (`ontology::tests::mcp_tools_match_fixtures` replays them), so a changed answer is a change made on purpose.

`cargo test -p beatscript-serve --bin beatscript-serve ontology` checks the rest (`ontology::tests::fork_integrity`): the served files
are the vendored ones, the vendored ones are what the inputs build, the lock and manifest hashes match, the version is
0.1.0 and derived from bitwrap 0.2.0, every IRI is in the fork's namespace except the declared derivation links,
every bitwrap 0.2.0 term and triple has its counterpart, the serializations are one graph, the certificate replays to
the pack's graph, and the vocabulary has no drift.

## What still reads bitwrap

BeatScript programs link to the bitwrap ontology, not to this fork: a `#! genre:` / `#! tags:` line resolves against
`crates/djhost/data/bitwrap-ontology.json` (a snapshot of bitwrap's terms, refreshed by `tools/ontology/refresh.py`),
the program JSON-LD's `bw:` prefix is `https://ontology.bitwrap.io/ns#`, and `beatscript-serve::genres` reads bitwrap's IRIs.
Those IRIs are upstream and stay valid, and nothing a program's CID covers changes.
`ontology::tests::genres_read_the_fork_as_bitwrap` shows the genre conventions and `check`'s style warnings read the
same from the fork, so they can move to it later without a change in what they say.
