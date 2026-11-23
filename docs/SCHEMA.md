# BeatScript schema

A BeatScript program travels as JSON-LD. A page that shows a program embeds it as a `<beatscript-view>` element whose
child `<script type="application/ld+json">` is the program's source, the way pflow.xyz embeds a net in `<petri-view>`.
The editor loads the program from that script, and so can anything else that reads the page: the Android app, the MCP
`get` tool, a crawler, a script.

```html
<beatscript-view data-editor data-ui="bar"><script type="application/ld+json">
{"@context":["https://beatscript.app/schema","https://schema.org"],
 "@type":["Program","MusicComposition"],
 "@version":"1",
 "@id":"z4EBG9jGgmxN7Nk9BuV8pKFtH2ivh2HFMC4rw1MwxjkD9YmkJFa",
 "name":"arrange-mutations",
 "license":"https://creativecommons.org/publicdomain/zero/1.0/",
 "licenseId":"CC0-1.0",
 "tempo":132,
 "key":"am",
 "scale":"https://beatscript.app/ontology/scale/minor",
 "text":"#! beatscript 2\ntempo 132\nkey am\nlet acid_a = ...\n"}
</script></beatscript-view>
```

This page is `https://beatscript.app/schema`. A JSON-LD processor that asks for it with `Accept: application/ld+json`
gets the context below. A browser gets this page. `https://beatscript.app/schema.jsonld` is always the context, and
`https://beatscript.app/schema.md` is this page's markdown. All three are served with `Access-Control-Allow-Origin: *`.

## The block

| key | value |
|---|---|
| `@context` | `["https://beatscript.app/schema", "https://schema.org"]`: this vocabulary first, then schema.org |
| `@type` | `["Program", "MusicComposition"]`: a BeatScript program, and to schema.org a musical composition |
| `@version` | `"1"`: the version of this block's format, as pflow.xyz versions its documents. A change to the shape is a new version. JSON-LD processors do not read it |
| `@id` | the program's **CID**: the hash of its canonical text (below) |
| `name` | the program's name (`#! name:`), when it has one |
| `license` | its licence as a URI: `https://creativecommons.org/publicdomain/zero/1.0/` (CC0, the default), `https://creativecommons.org/licenses/by/4.0/` (CC BY), `http://rightsstatements.org/vocab/InC/1.0/` (all rights reserved) |
| `licenseId` | the same licence's id: `CC0-1.0`, `CC-BY-4.0` or `all-rights-reserved` |
| `tempo` | beats per minute, when the program sets one |
| `key` | the program's key (`am`, `c`, `d:dorian`), when it sets one |
| `scale` | the key's mode as a [BeatScript ontology](https://beatscript.app/ontology/ns#) term (`https://beatscript.app/ontology/scale/minor`), when the ontology has it |
| `genre` | the `#! genre:` line as an ontology term, when it names one |
| `tag` | the `#! tags:` that are ontology terms |
| `instrument` | the voices its parts name (`inst=acid`) that are ontology instruments |
| `section` | the sections with a doc comment, in order: each `{"name", "description"}` (schema.org's), the description being the `#` comment lines written directly above its `section` line; only when some section has one |
| `text` | the **canonical program text** |
| `author` | the author's address or DID, when the program is signed in to an account |
| `forkedFrom` | the CID of the program this one was forked from, when it was |
| `isBasedOn` | what it is based on: each generator call, borrow and vendored part, with the CID it `resolves` to; on a stored expansion, each reference its recipe wrote out into it |
| `hasPart` | for a program with a template whose roles are bound, the nested parts of its expansion: each a CreativeWork whose `@id` is the CID of what it is written out as, named by its path (`episodes/ground`), based on its source, with its own `hasPart` |
| `expandsTo` | for a program with references, the CID of the program they write out to |
| `expandedFrom` | on a stored expansion, the CID of the recipe it was written out from |
| `signature` | the author's signature over the CID, when the share is signed |

### The CID is `@id`, and `text` is canonical

`@id` is the program's CID: CIDv1, dag-json multicodec, sha2-256, base58btc (`z…`), the same CID a share link and
`check` give. It is the hash of a document that names the language version
([Versions](/docs/language#versions)):

- a BeatScript 2 program (one that starts `#! beatscript 2`): `{"format":2,"text":<canonical text>,"type":"beatscript"}`;
- a BeatScript 1 program (the old spelling: `#! btw 1`, or no version line): `{"format":1,"text":<canonical text>,"type":"djrust.live"}`,
  as every program before the rename.

The text in the block is the canonical text: the program parsed and printed again, with no comments and no `#!`
lines but the version line, which a BeatScript 2 text keeps first (`#! beatscript 2\n` and then the statements; a
BeatScript 1 text has no `#!` line at all). A reader tells the two apart by that first line alone. So `text` hashes to
`@id`. A reader must recompute it, and must refuse a block whose text does not hash to its
`@id`, rather than play it. The editor, the Android app and the server all do. A reformatted or edited text is a
different CID, which is what makes a program's name its content.

`#!` header lines (name, licence, author, genre, tags) are not part of the CID. In the block they are the fields
beside `text`. The version line is the exception: it is part of the program, so the same music written in BeatScript 2
has another CID than in BeatScript 1. beatscript.app keeps both, and gives the BeatScript 2 program an `isBasedOn` of the
BeatScript 1 one with the derivation `renamed`.

### Attributes of `<beatscript-view>`

| attribute | means |
|---|---|
| `data-editor` | this is the program the page's editor opens. A page has at most one |
| `data-ui="bar"` | how much of the player to show: `bar`, the transport bar |

## The context

The context is served at `https://beatscript.app/schema.jsonld`, and at `/schema` to a processor that asks for
`application/ld+json`. It is built by the same code that builds the block (`djlive::progld::schema_context`), so it
cannot drift from it. In outline:
- `@vocab` and the prefix `bs:` are `https://beatscript.app/schema#`;
- `bw:` is the [BeatScript ontology](https://beatscript.app/ontology/ns#);
- every term below maps to `bs:<term>`;
- `license`, `genre`, `scale`, `tag` and `instrument` are IRIs (`@type: @id`), and `tag` and `instrument` are sets.

A test keeps every term the context defines listed in [Terms](#terms).

The block lists schema.org second, so where both contexts define a term, schema.org's definition wins: `name`, `text`,
`license` and `author` read as `schema:name`, `schema:text`, `schema:license` and `schema:author`. `tempo`, `key`,
`forkedFrom` and `signature` are not schema.org terms, so they keep this vocabulary's (`bs:`) meaning.

## Terms

Each term of the vocabulary (`@vocab` `https://beatscript.app/schema#`) is defined here. Its IRI is this page with
the term as the fragment.

<a id="Program"></a>**Program**: a BeatScript program. Its `@id` is its CID, and its `text` is the canonical text that
hashes to it.

<a id="text"></a>**text**: the canonical program text.

<a id="tempo"></a>**tempo**: beats per minute, a number.

<a id="key"></a>**key**: the program's key as BeatScript writes it (`am`, `c`, `d:dorian`).

<a id="scale"></a>**scale**: the key's mode, as a BeatScript ontology scale.

<a id="genre"></a>**genre**: a BeatScript ontology genre.

<a id="tag"></a>**tag**: a BeatScript ontology term the program is tagged with.

<a id="instrument"></a>**instrument**: a BeatScript ontology instrument one of the program's parts plays.

<a id="section"></a>**section**: the program's documented sections, in order, each a `name` and a `description` (schema.org's
terms). A section's doc comment is the run of `#` lines directly above its `section` line, with no blank line between
(a `#!` header line is never one), joined into one paragraph, and a leading `<name>:` that repeats the section's name
dropped. Comments are not in the canonical text, so a doc comment is carried beside the program and never changes its
CID.

<a id="license"></a>**license**: the licence, as a URI.

<a id="licenseId"></a>**licenseId**: the licence's id (SPDX for the CC licences).

<a id="author"></a>**author**: who made it: an address or a DID.

<a id="forkedFrom"></a>**forkedFrom**: the CID of the program this one was forked from.

<a id="isBasedOn"></a>**isBasedOn**: what the program is based on, schema.org's term: one CreativeWork for each engine call and
borrow its text makes (`gen@2 techno seed=42 section=drop part=bass`, a seed, a stored program's section by CID), for the BeatScript 1
program a BeatScript 2 one was renamed from, and for each
part an eject wrote out into it (its `# from: <call> -> <cid>` line; for a stored program, whose text is canonical, the lines its
save kept as `meta.based_on`), each named by its call (`name`), with `resolves`, `pinnedGenerator` and `derivation`. Like the doc
comments, it is outside the CID. A program is never based on itself: a source
that resolves to the block's own `@id` is named by its call alone.

<a id="resolves"></a>**resolves**: the CID a reference resolves to: for a generator call, the CID of the program the generator
prints for it (its pin); for a borrow, the stored program's CID.

<a id="pinnedGenerator"></a>**pinnedGenerator**: the generator and version a call names (`seeded@2`): a version always prints the
same program for the same inputs.

<a id="derivation"></a>**derivation**: how a source came in: `called` (an engine call the text makes), `borrowed` (a structural
reference the text makes), `vendored` (written out by an eject; the text has the notes, and the `# from:` line the call),
`renamed` (the same program in BeatScript 1, the old spelling: the same music, under its old CID) or
`expanded` (on a stored expansion: written out with the rest of its recipe, which the store says in the expansion's
`based_on`).

<a id="expandsTo"></a>**expandsTo**: a recipe's written-out program (every reference written out as BeatScript): its CID. The
same recipe always expands to the same program, and the two play the same.

<a id="expandedFrom"></a>**expandedFrom**: on a written-out program, the CID of the recipe it was written out from. A program with
references stored on beatscript.app is stored with its expansion beside it, so `/p/<expandsTo>` loads it; in the lineage
document `expandedFrom` is a list (several recipes can write out to one program).

<a id="hasPart"></a>**hasPart**: the nested parts of a recipe's expansion, schema.org's term (a set): each role its template
reference binds, at any depth, as a CreativeWork whose `@id` is the CID of what it is written out as (a program of its own: the
key and tempo it plays in and the lines its binding wrote, every reference in them written out, canonical), whose `name` is its
path (`episodes/ground`: the `ground` of the form bound to `episodes`), which `isBasedOn` its source (`resolves`,
`pinnedGenerator`, `derivation` `called` for a gen call and `borrowed` for structure), and which has its own `hasPart`. The same
recipe always has the same parts; like `isBasedOn`, they are outside the CID.

<a id="signature"></a>**signature**: the author's signature over the CID, opaque to everything but the wallet that
checks it.

<a id="Section"></a>**Section**: a section of a program's song, as its program document lists it: its `name`, its `start`
and its `bars`, and its `key` when the section sets one itself.

<a id="sections"></a>**sections**: the song's sections in order, a list of `Section`s, one for each time the arrangement places
one, by start bar (a section placed twice is listed twice). It is the same property as `section` (`bs:section`), under the
name the program documents give it.

<a id="start"></a>**start**: the bar a section starts on, counted from 0.

<a id="bars"></a>**bars**: a length in bars: how long a section is.

<a id="cid"></a>**cid**: a program's CID as a value, beside an `@id` (a credited program carries its own).

<a id="credit"></a>**credit**: the programs this one credits, a set: a CC BY parent it was forked from, a program it imports.
Each is named by its `@id` and `cid`, with its `name`, `author`, `licenseId` and `license`.

<a id="visibility"></a>**visibility**: `public` or `private`. A private program is never stored or shared by the site.

<a id="euclid"></a>**euclid**: the Euclidean rhythms the program's song plays (its `e(k, n[, rot])` calls, written out), one node
for each distinct shape: a BeatScript ontology `EuclideanRhythm` with its pulses, steps and rotation, its `uses`, and the named
rhythms of the same shape (`sameShapeAs`).

<a id="uses"></a>**uses**: how many times the written-out song plays a Euclidean rhythm.

<a id="sameShapeAs"></a>**sameShapeAs**: the BeatScript ontology's named rhythms of exactly this shape, as IRIs (the ontology
names its rhythms by genre and role, so a shape may have several, or none).

<a id="ProgramDump"></a>**ProgramDump**: an archive of programs: `count`, the sorted `cids`, and the files (see
[Open data](/docs/open-data)).

## The other documents

The block is the program with its source. A program also has a fuller linked-data document with the same CID:

- `https://beatscript.app/p/<CID>.jsonld` (or `/c/<CID>.jsonld`): the program's structure. It gives the name, key, tempo,
  sections, author, and licence (`licenseId`, and `license` as a URI). Its rights statement adds only what the site
  stores: the credits (`credit`) and the program it was forked from (`forkedFrom`, and schema.org's `isBasedOn`), and
  `usageInfo`, `https://beatscript.app/authorship`: who authored a program and what rights it has, in plain language.
  A section the shared text documented (a `#` comment line directly above its `section` line) has that doc comment as
  its `description`. The context it names is its program's version's ([the two contexts](#the-two-contexts), below).
- The engine's own JSON-LD export (on the command line, `djctl btw jsonld FILE`) says more in the same vocabulary: Euclidean rhythms linked to the BeatScript
  ontology (`https://beatscript.app/ontology/ns#`), the genre and tags as ontology IRIs, credits and `forkedFrom`.
- `https://beatscript.app/phrasebook.jsonld` (and `/phrasebook/<CID>.jsonld`): the public-domain phrasebook as linked data, a
  `Dataset` whose `@id` is the CID of the corpus's text and whose `hasPart` is one `Quotation` node a line. Its `@context` is
  `[this vocabulary, schema.org, {a local context}]`: the local one maps each key a node writes (`stress`, `family`, `feel`,
  `syllables`, `firstPublished`, `edition`, `sourceWork`, `sourceAuthor`, `lyric` and the rest) to a term of
  [the BeatScript ontology](/ontology/), so every concept in it resolves there. Its `@type`s `Phrasebook` and `PhrasebookEntry`
  are the ontology's.
- The program pages `/p/<CID>` and `/c/<CID>` carry that document in their head, as before, and the block in their
  body. Their `og:*` and `twitter:*` tags, title, description and canonical link are unchanged.
- The editor at `/?cid=<CID>` carries the block for a stored public program.

### The two contexts

A program document names one of two contexts, by the version its program is written in
([Versions](/docs/language#versions)). Both stay served, and neither is ever edited: a change is a new context.

- **`https://beatscript.app/context/beatscript-v1.jsonld`** (`docs/beatscript-context-v1.jsonld`): BeatScript 2 programs.
  The document's `@id` is `https://beatscript.app/p/<CID>`. Its vocabulary is this page's, `https://beatscript.app/schema#`
  (prefix `bs:`), beside `bw:` (the BeatScript ontology, beatscript.app/ontology/ns#) and `schema:` (schema.org): each `bs:` term it maps is defined under
  [Terms](#terms). Three terms moved from the old context: `author` is `bs:author` (it was `dct:creator`), `license` is
  `bs:license` (it was `dct:license`) and `name` is `schema:name` (it was `dct:title`). `description`, `isBasedOn` and
  `usageInfo` are schema.org's, in the context itself.
- **`https://beatscript.app/context/v1.jsonld`** (`docs/btw-context-v1.jsonld`): BeatScript 1 programs, the old spelling,
  unchanged. Their documents keep `@type` `djrust:Program` and the `djrust:` and `dct:` terms, and the schema.org terms
  come in a local context beside it (`@context` is `[v1, {...}]`).

## What changed from the beats conventions, and why

This format reuses the conventions of the beats sequencer and the first (`djrust:`) context where it can:

- the term names `name`, `tempo`, `key`, `license`, `author`, `forkedFrom` and `signature`;
- the CID scheme;
- the `/p/<CID>` URL shape;
- the escaping of JSON inside a `<script>` tag;
- a custom element that reads its source from a child script, as `<petri-note>` does.

Where it differs:

| beats / the first `djrust:` context | this block | why |
|---|---|---|
| `@id` was a share IRI on the beats host (before 2026-11-23) | `@id` is the bare CID | Programs moved to beatscript.app and the CID is the name: it is the same wherever the block is embedded, and a reader checks it against `text`. As an IRI it resolves relative to the page's URL; the canonical page is `https://beatscript.app/p/<CID>` |
| `@context` is one URL, `…/context/v1.jsonld` | `["https://beatscript.app/schema", "https://schema.org"]` | The block is also meant to be read as schema.org (`MusicComposition`, `name`, `license`, `text`) by tools that know nothing of BeatScript. Since 2026-11-23 every program document names the BeatScript context (`context/beatscript-v1.jsonld`); `context/v1` stays served, unchanged, for the documents made before |
| `@type` is `djrust:Program` (or `BeatsShare` for a beats track) | `["Program", "MusicComposition"]` | The same class in this vocabulary, plus its schema.org reading |
| The source is not in the document: a beats envelope is its own payload, and the `djrust:` document points to the program by its share IRI | `text` holds the canonical program | The block is the program's source: the editor loads it from the page without another request, and the phone and MCP read the same bytes |
| `license` is a URI and `licenseId` the SPDX id (`djrust:`), the URI only for CC0; beats tracks are always CC BY 4.0 | `license` a URI and `licenseId` the id, for every licence | The same two fields. Every licence now has a URI, rightsstatements.org's "In Copyright" for all rights reserved (no CC licence says that), and the `djrust:` document gives the same URIs |
| `genre` and `tag` are ontology IRIs (`djrust:`); beats tracks' genre is a string | `genre`, `tag`, plus `scale` and `instrument`, as ontology IRIs where a term exists | The same links, and the key and voices too, since the ontology has scales and instruments. A value with no term is left out rather than invented |
| `signer` {type, address} and `signature` over the envelope's CID (BeatsShare) | `signature` over the program's CID, and `author` | A signed share link carries the author's signature over the CID (`<CID>.<packed>.<sig>`). The block passes it on, opaque, as share links do |
| `@version` 1.1 inside the context (a JSON-LD keyword) | `"@version": "1"` on the block itself | This follows pflow.xyz's embedded documents: it is the block's format version, not the JSON-LD version |

## For implementers

- The builder and the reader are one Rust module, `djlive::progld` in the engine:
  - `document` builds the block's JSON;
  - `view_html` builds the element;
  - `read_view` and `read_document` read a block back and check it;
  - `schema_context` returns the context.
  The server, the MCP and the Android app all use it, so the bytes are the same everywhere.
- Inside the script, `<`, `>`, `&`, U+2028 and U+2029 are written as `\u` escapes. The result is still the same JSON,
  and no program text can close the script.
