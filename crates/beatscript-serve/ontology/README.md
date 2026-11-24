# The BeatScript ontology

What `/ontology/` on beatscript.app serves (`src/ontology.rs`): the vocabulary behind BeatScript as Turtle,
JSON-LD, N-Triples and RDF/XML, a read-only SPARQL endpoint and an MCP server, CC0.

It is the **BeatScript ontology**, version 0.4.0 (`owl:versionIRI https://beatscript.app/ontology/0.4.0`). 0.2.0 was
the release of BeatScript 2, the language's second version, with the name BeatScript where bitwrap's wording said btw;
0.3.0 adds bitwrap's genre conventions module, re-homed, so that everything BeatScript links to is here; 0.4.0 has every
genre the seeded generator knows: the ones beats' catalogue has come from bitwrap, and djrust's own (acid, deephouse,
electro, italo) are generated from the generator's tables, so `#! genre: acid` and check's style advice know them. Its
namespace is `https://beatscript.app/ontology/ns#`. It was **derived from the bitwrap ontology 0.2.0**
(ontology.bitwrap.io, CC0) and **evolves on its own** from here. ontology.bitwrap.io and its repository are not
touched by anything in this directory, and nothing here is fetched from them, at build or at serve time.

## What is in a release

A release is built from four inputs:

| Input | What |
|---|---|
| `upstream/bitwrap-0.2.0/ns.nt` (+ `manifest.json`, pinned by `upstream.lock`) | the bitwrap 0.2.0 release, vendored by `sync.sh upstream` |
| `upstream/bitwrap-conventions/conventions.ttl` (pinned by `conventions.lock`) | bitwrap's genre conventions module (tempo ranges, usual kits and voices, section lengths), CC0; re-homed and linked back like the 0.2.0 terms. `beatscript_serve::genres` (check's style, `genre_template`) reads it from the release |
| `fork.ttl` | the fork's own module: its `owl:Ontology` header (title, version, version IRI, issued, creator, licence, `prov:wasDerivedFrom` the bitwrap release) and the few terms it adds by hand (the products its MCP tools belong to) |
| `generated/beatscript.ttl` | the BeatScript vocabulary, generated from beatscript-serve's own code: every production and keyword of `crates/djlive/grammar.ebnf`, the voices, kits, families and palettes (`beatscript_api::instruments_json()`) and each family word's generic sound (djrust `wave::synth::tonal_voice`), the control registry's nouns and verbs, djlive's generators, transforms and evolve and mutation operators, both MCP servers' tools, and the seeded generator's genres that bitwrap 0.2.0 does not have (`vocab::generator_genres`, from djrust's genre tables: tempo, scale, root, groove, family, drum rhythms and variants, chord progressions, the voices and kits it picks per role) |

`src/ontology/fork.rs` assembles them:

- **Re-homing.** Every bitwrap IRI under `https://ontology.bitwrap.io/` moves under `https://beatscript.app/ontology/`:
  `…/ns#Genre` becomes `https://beatscript.app/ontology/ns#Genre`, `…/genre/techno` becomes
  `https://beatscript.app/ontology/genre/techno`. bitwrap's own header is dropped; the fork has its own.
- **Restated wording.** Where `fork.ttl` gives a bitwrap term an `rdfs:label`, `rdfs:comment`, `skos:prefLabel`,
  `skos:definition` or `skos:scopeNote` in the language of bitwrap's, the fork's replaces bitwrap's (`fork::RESTATABLE`,
  `fork::restated`); nothing else of bitwrap's is replaced, only added to. A replaced label must be kept as the term's
  `skos:hiddenLabel` (or `skos:altLabel`), or the build refuses, so a search for the old name still finds the term. This
  is how `ns:Program` became "BeatScript program" (hidden label "btw program") and the wording that said btw says
  BeatScript; the old wording stays at the bitwrap IRI the term's `prov:wasDerivedFrom` names.
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

- A change to the grammar, the instruments, the control registry, the language's versions (`djlive::version`, listed in
  `vocab::VERSIONS`), either MCP server's tools or the generator's genre tables (a djrust bump that adds a genre or moves
  a genre's tempo, scale, rhythm or progressions) changes the vocabulary: run `build.sh`, then `tools/ontology/refresh.py`
  (the header snapshot `#! genre:` reads), and commit `generated/`, `release/`, `release.lock` and the snapshot with the
  change. `ontology::vocab::tests::ontology_matches_beatscript_serve` fails until you do. A genre bitwrap 0.2.0 already
  has is not regenerated: if the generator moves one of its values, the test names it, and the fix is a decision (see the
  fork's open questions), not a rebuild.
- A change to the ontology's own terms goes in `fork.ttl` (never in `upstream/`), then `build.sh`. A release that
  changes what the ontology says is a new version: bump `owl:versionInfo`, `owl:versionIRI` (and `owl:priorVersion`)
  and `dct:issued` first, and the version `ontology::tests` expects. Only the current release is served: the version
  IRI of an earlier one names it, and its bytes are in git.
- A program's upgrade to a later language version is a second program, never a second CID of the first:
  `ns:programCid` is functional, so the upgrade is its own individual with its own `ns:programCid` and
  `ns:inLanguageVersion`, `prov:wasRevisionOf` the original (`ontology::tests::functional_properties_have_one_value`
  refuses two values of a functional property).
- Merging a later bitwrap release in is deliberate: `sync.sh upstream REF` vendors it beside 0.2.0, then
  `src/ontology/fork.rs` is pointed at it and the commit says why.
- `fixtures.py` records the ontology MCP's answers from a running beatscript-serve into `fixtures/mcp-<version>.json`
  (`ontology::tests::mcp_tools_match_fixtures` replays them), so a changed answer is a change made on purpose.

`cargo test -p beatscript-serve --bin beatscript-serve ontology` checks the rest (`ontology::tests::fork_integrity`): the served files
are the vendored ones, the vendored ones are what the inputs build, the lock and manifest hashes match, the version is
0.2.0 and derived from bitwrap 0.2.0, every IRI is in the fork's namespace except the declared derivation links,
every bitwrap 0.2.0 term and triple has its counterpart (or is wording `fork.ttl` restates, and the restatement is in),
the serializations are one graph, the certificate replays to the pack's graph, and the vocabulary has no drift.

## Nothing reads bitwrap

BeatScript data lives on beatscript.app only (2026-11-22). Programs link to this ontology: a `#! genre:` / `#! tags:` line
resolves against `crates/djhost/data/beatscript-ontology.json` (the music terms of this release, refreshed offline by
`tools/ontology/refresh.py`), and is written with this ontology's IRIs; an IRI under `https://ontology.bitwrap.io/`, which
programs wrote before, is read as the same path here (`djlive::linked::rehome`). The program JSON-LD's `bw:` prefix is
`https://beatscript.app/ontology/ns#` (the BeatScript context), `beatscript_serve::genres` reads this release (bitwrap's
conventions module is one of its inputs since 0.3.0), and `ontology::tests::genres_read_the_fork_as_bitwrap` shows they
answer exactly what they answered from bitwrap's files, in this ontology's IRIs. Header lines are not in a program's CID,
so no CID moved. The only bitwrap IRIs left are the `prov:wasDerivedFrom` links in this ontology's own files.
