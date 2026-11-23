# F14: ontology links (`#! genre:`, `#! tags:`, JSON-LD)

*Extension F14 of `docs/BEATSCRIPT-EXTENSIONS.md`. Code: `djlive::linked` (pure: the header model, the checks, the JSON-LD
builder), `djhost::ontology` (the cached vocabulary), `djhost::live_store` (the header lines in saved `.beatscript` objects, the
`<object cid>.jsonld` beside each), the `btw` verbs `jsonld` and `terms` (registry in `djhost::control_registry`, handler in
`djapp/src/control/live.rs`), `djctl btw jsonld | terms`. Tests: `crates/djlive/tests/ext_linked.rs`,
`crates/djhost/tests/ext_linked.rs`, `crates/djctl/tests/ext_linked.rs`. Example: `examples/beatscript/linked.beatscript`.*

A program can say what it is. Two more `#!` header lines, read with the rights lines (`#! license:` ...) of a `.beatscript` file:

```text
#! genre: https://beatscript.app/ontology/genre/techno          (or: techno, genre:techno, bw:Techno)
#! tags: acid, supersaw, four on floor, drop                 (terms of the ontology)
```

They link the program to the **BeatScript ontology** (https://beatscript.app/ontology/ns#, a CC0 fork of the bitwrap ontology), a public (CC0) vocabulary of the genres the
engine generates, and of the scales, grooves, instruments, song sections, structures, Euclidean rhythms and chord progressions
around them. A host that has the vocabulary checks them and exports the program as linked data (JSON-LD).

The language never reads the lines. They are comments: **not in the program's CID** (as `#! license:` is not), so a program
with and without them is the same program (`tests/ext_linked.rs`: `header_lines_are_comments_not_in_the_cid...`). A change of
genre or tags is **a new object of the same program**, as a new licence is: the program CID is what links and `forkedFrom`
name, the object CID names the file.

## Grammar

```ebnf
header      = { header-line } ;                                (* the leading run of lines that start with `#!`, as live_store reads them *)
header-line = "#!" key ":" value NEWLINE ;
genre-line  = "#!" "genre" ":" term ;                          (* one genre: a term of kind genre *)
tags-line   = "#!" "tags"  ":" term { "," term } ;             (* any terms; several lines are one set *)
term        = iri | curie | name ;
iri         = "https://beatscript.app/ontology/" kind "/" local ;  (* a published term; also the namespace spelling bw-ns "#" Name;
                                                                  the same path under https://ontology.bitwrap.io/, what programs wrote
                                                                  before the BeatScript ontology, is read as this IRI *)
curie       = "bw" ":" name                                    (* the vocabulary's namespace: any kind *)
            | kind ":" name ;                                  (* genre:techno  instrument:acid  groove:four-on-floor *)
name        = local-name | label ;                             (* techno   four on floor   Four-On-Floor   four_on_floor *)
kind        = "genre" | "instrument" | "scale" | "groove" | "section" | "family" | "role"
            | "rhythm" | "progression" | "structure" | "concept" ;
```

Example (`examples/beatscript/linked.beatscript`): `#! tags: acid, supersaw, four on floor, drop`.

A header line is read only in the **header block**: the first lines of the text, as long as each starts with `#!` in column 1.
A `#! genre:` or `#! tags:` line after the program has begun is a plain comment, and `check` says so (a warning, "this `#! genre:`
line is not read").

**Spellings.** A name is compared in lower case with spaces, underscores and hyphens as one hyphen: `Four On Floor`, `four_on_floor`
and `four-on-floor` are one name. It matches a term's local name (the last segment of its IRI) or its `rdfs:label`. Several
terms of different kinds may have one name (`bossa` is a genre and a groove, `bass` an instrument and a role, `edm` a genre and a
family): a bare name goes to the **first kind in the order** genre, instrument, scale, groove, section, family, role, rhythm,
progression, structure, concept, and `groove:bossa` says the other. On the genre line only genres are looked at, so `bossa` there is
the genre whatever else it is. Every spelling is **stored as the full IRI** of the term it names.

## What is checked, and where

The vocabulary is a cached snapshot (`crates/djhost/data/beatscript-ontology.json`, below). `djlive` is handed it through the
`djlive::linked::Ontology` trait in a new optional field `Env::ontology`, so the language crate stays pure and a test can use a fake.

| where | an unknown, misspelt or wrong-kind term is | with |
|---|---|---|
| `btw check` / `eval` / `djctl btw check` (`check_with`, when `Env::ontology` is set) | a **warning**: the program still compiles and plays, its CID is the same | line and column of the term, a did-you-mean |
| `btw save` (the text's header lines, or the `genre` / `tags` params), `btw import`, `djctl btw save`, `btw jsonld {text}` | an **error**: nothing is kept | the message, line and column, the did-you-mean |

```text
warning: unknown genre `tehcno`
 --> 1:11
  |
1 | #! genre: tehcno
  |           ^^^^^^
  = help: did you mean `techno`?
```

A warning, not an error, in `check`: a comment must never stop a live program. The messages: `unknown genre `X`` / `unknown tag `X``
(help: `did you mean `Y`?`, or several; with no near name a genre line lists the genres in `expected`, a tag points at
`djctl btw terms`), `` `groove:bossa` is a groove, not a genre ``, `a program has one genre` (a second `genre:` line, or commas in
it), ``a genre line names no genre``. An IRI under `https://beatscript.app/ontology/` (or the old `https://ontology.bitwrap.io/`) that is not a term suggests the IRIs it may have
meant (`.../genre/tehcno` gives `.../genre/techno`; `.../section/acid` gives `.../instrument/acid`); an IRI of someone else's is
unknown.

## The vocabulary: a cached snapshot

`crates/djhost/data/beatscript-ontology.json` is the source of truth for tests and for a machine that is offline; it is compiled in
(`include_str!`, `djhost::ontology::vocabulary()`), so **nothing at build time, at run time or in a test fetches anything**. It is the
music terms of the BeatScript ontology release in this repository (`tools/ontology/refresh.py` reads
`crates/beatscript-serve/ontology/release/ns.nt`, checked against its manifest and lock). It holds the namespace, the version and
release date, the day it was taken, where it is served (`https://beatscript.app/ontology/ns.nt`) and that file's SHA-256, the
ontology's pack digest, `verified`, how to refresh it, and 265 terms sorted by IRI, one a line: `iri`, `kind`, `local`,
`label`, and for a Euclidean rhythm its `pulses`, `steps` and `rotation`. It is 41 KB.

The terms by kind: genre 19, genre family 4, scale 9, groove 7, instrument 30, instrument role 8, song section 10, song structure 71,
Euclidean rhythm 57, chord progression 32, engine concept 16. (Not kept: the structures' numbered slots, the drum variants and the
section profiles, which are scaffolding, not names an author writes; and the ontology's classes and properties, which are not tags.)

**Refresh:** `python3 tools/ontology/refresh.py` (needs the network; Python standard library only). It fetches `ns.nt`, checks it against the
release's `SHA256SUMS` (a mismatch writes nothing), and rewrites the snapshot. `--from FILE --fetched DATE` rebuilds it from a saved copy,
offline (then `verified` is `false`). Then `cargo test -p djhost` (the tests pin the counts and that the file is sorted). The snapshot that is
committed was fetched on 2026-11-18 from the published namespace (version 0.1.0, released 2026-11-11), and **was verified**
(`verified: true`): its SHA-256 `35e60199...` is the one in the release's `SHA256SUMS`. (The Bitwrap Ontology MCP tools were not available in the
run that made it; the published files were fetched over HTTPS instead.)

`djctl btw terms [KIND]` and the verb `btw terms {kind}` list it. `djhost::ontology::describe()` says where the copy is from.

## Saved programs

`djhost::live_store`. A `.beatscript` file's header gets the two lines after the rights lines, as full IRIs, tags sorted, each once (a set):

```text
#! beatscript 2
#! name: night bus
#! license: CC0-1.0
#! visibility: public
#! author: did:key:z6Mk...
#! genre: https://beatscript.app/ontology/genre/techno
#! tags: https://beatscript.app/ontology/groove/four-on-floor, https://beatscript.app/ontology/instrument/acid
tempo 128
...
```

* **The object** (what the object CID is of) gets `genre` and `tags` keys **only when there is something to say**: an object that links to
  nothing is, byte for byte, the object it was before this feature, so no saved file moves. The same set in any spelling or order is the
  same object.
* **Save.** `save_with(name, text, rights)`: the text's own header lines are the links (resolved, an unknown term is an error with line and
  column); a text with none keeps the links of the original already saved (as it keeps its rights). `btw save {genre, tags}` sets them
  (a blank genre or an empty list clears), and is refused **before** anything is kept when a term is unknown. `LiveStore::set_links(id, LinkEdit)` is
  the same for a saved program: a new object of the same program, the old file (and its JSON-LD) goes; only the half that is named is looked up.
* **Export / import.** `export` writes the header lines; `import` (a path or text) reads them in any spelling and keeps them; an unknown term is an
  error ("not imported, unknown tag ..."). A file with no such lines has no links. An export then import is the same object.
* **Fork** carries the links on, with the rights. `set_rights` (license, visibility) keeps them. The list rows (`btw list`, a save's answer) have
  `genre` (an IRI or null) and `tags` (a list).
* **A term the vocabulary has since lost** (a refresh dropped it) does not lose a program: the store reads what it kept, and only a fresh save or
  edit says the term is unknown.
* **The JSON-LD beside each object.** Every write of an object also writes `<object cid>.jsonld` (the document below, `djlive::linked::pretty`
  of it, a newline at the end) next to `<object cid>.beatscript`; when an object is replaced or deleted its pair goes. It is derived: a failure to write it
  does not fail a save, and `btw jsonld {id}` makes the same text again. A program that no longer compiles (an import is gone) has none.

## The JSON-LD document

`btw jsonld {id | text}` (a registry verb, `Read`), `LiveStore::jsonld` / `jsonld_text`, `djctl btw jsonld FILE`. `id`: a saved program (CID, part
of one, name), whose document is the one beside its object. `text`: a program or a `.beatscript` file's text; its genre and tags are its header lines'
(an unknown term is an error), and it **claims rights only when its header has rights lines** (`license`, `visibility`, `author`, `forked-from`,
`credit`): a program that was never saved says nothing about its licence. With neither, the window's text.

Keys of every object are sorted (UTF-16 order, as `canonical_js`) and printed with a two-space indent, so the same facts are the same bytes: there is
no timestamp, no random number and no path of this machine in it.

| key | is | from |
|---|---|---|
| `@context` | the URL `https://beatscript.app/context/beatscript-v1.jsonld` (`linked::DOCUMENT_CONTEXT_URL`), the BeatScript context: `bs:` the program's terms (`https://beatscript.app/schema#`), `bw` the BeatScript ontology's namespace (`https://beatscript.app/ontology/ns#`), and the terms below; `jsonld FILE --inline` spells them out. Published (CORS, `application/ld+json`, immutable). The first context, `https://beatscript.app/context/v1.jsonld` (`docs/btw-context-v1.jsonld`, `linked::context()`), stays served for the documents made before 2026-11-23; no document names it now | fixed |
| `@id` | `https://beatscript.app/p/<program CID>`: the program's page, as the program block names it (`progld::IRI_PREFIX`) | the CID |
| `@type` | `Program` (`bs:Program`) | |
| `cid` | the program CID, as a plain string | |
| `name` | `dct:title` | the saved name |
| `genre` | IRI | `#! genre:` |
| `tag` | IRIs, sorted | `#! tags:` |
| `key`, `tempo` | `am`, `128` (the canonical spelling; absent when the program sets none) | `key`, `tempo` |
| `sections` | an ordered list of `{@type Section, name, start, bars, key?}`: **the song's sections, one for each time the arrangement places one**, by start bar (not the compiled stretches: `stack(groove, wash)` lists `groove` and `wash`, each from bar 0 with its own length, never a `groove+wash`). `start` is the 0-based bar; `key` is the one the section sets itself (its first `key` statement), absent when it sets none. A section's chained stretches and an `evolve`'s generations are one section with its bars as written. Absent for a program with no sections | `arrange` |
| `licenseId`, `license`, `visibility`, `author` | `CC-BY-4.0`, `https://creativecommons.org/licenses/by/4.0/` (`dct:license`; none for all-rights-reserved), `public`, `dct:creator` | the rights |
| `forkedFrom`, `isBasedOn` | the parent's CID, and its IRI `https://beatscript.app/p/<CID>` | `forked-from` |
| `credit` | the credited programs: `{@id, @type Program, cid, name, author, licenseId, license}` | `credit` |
| `euclid` | one node per distinct Euclidean shape of the program: `@type bw:EuclideanRhythm`, `bw:pulses`, `bw:steps`, `bw:rotation`, `uses` (how many times the expanded song says it), `name` when the ontology has exactly one rhythm of that shape, and `sameShapeAs`: the IRIs of the ontology's named rhythms of exactly that shape | the program's `e(k,n[,rot])` |

The Euclidean terms are the `e(k, n[, rot])` (and `euclid(...)`) calls **of the expanded song** (`Compiled::linked`, read from the lowered program by
`linked::structure`): a def's body once it is called (`snare e($k, 8)` called as `pat(3)` is E(3,8)), arithmetic worked out (`e(2 + 2, 16)`), a
`repeat`'s copies, a part with a cycle of its own (`bass/12 e(5,12)`), a term under chance (`e(5,12)?0.5`), a choice or an alternation. `uses` counts every
line of the expanded song that says the shape (a def called twice, a `repeat 4` and a section placed twice count their copies; a part carried into the
later stretch of a stack is the same part, not another; a def nobody calls plays nothing, so it counts nothing). `0 <= k <= n`, `n >= 1`, and a rotation
is taken modulo `n` (`e(3,8,-6)` is `e(3,8,2)`), as the ontology's are. `@type` is the vocabulary's own Euclidean class (`Ontology::euclid_class`, the
published `ns#EuclideanRhythm`, written `bw:EuclideanRhythm`).

**Names and `sameShapeAs` are F6's table** (`meter::EUCLID_TABLE`, the 57 published `ns:EuclideanRhythm` individuals; `meter::euclid_names(k, n, rot)`), not a
second copy: `sameShapeAs` lists every individual of the shape, **whatever role plays it** (the table is role-blind here: `kick e(3,8)` is E(3,8), the
ontology's "house snare"), plus any rhythm of the host's vocabulary with that shape that the table lacks. `name` is the individual's label (`house snare`) only
when the shape is **one** individual of the ontology; six genres share E(4,16) (`country-kick` ... `trance-kick`), so a name picked among them would be arbitrary and
the node has `sameShapeAs` only. The ontology names its rhythms by genre and role, not by folk name: there is no "tresillo" in it, so none is invented.
The document does not depend on which vocabulary the host hands in, except for the class IRI and rhythms the table lacks.

**A part that every section shadows is not counted** (L-18). A section's own part of a role replaces the song's part of that role in that section (as the
compiler and `scope_parts` do), so `deck A: kick e(3,8)` outside every section, with a `deck A: kick ...` in every section, plays nowhere: its `e(3,8)` is
not in `euclid`. A song's part is counted once when at least one section does not shadow it (and always in a program with no sections); a section's own part
is counted for each time the section is placed.

**At most 64 tags** (R-24; `linked::MAX_TAGS`). Every tag is looked up in the vocabulary (about a millisecond, with a did-you-mean), so a header of 20,000
tags kept `check` busy for 15 s and `jsonld` for 43 s. The 65th item is an error ("a program has at most 64 tags: this is the 65th", at that tag; several
`#! tags:` lines are one set; a warning from `check`, an error from save and import like any header problem) and the rest of the line is not looked up;
`canonical_links` refuses a list over 64 the same way.

`examples/beatscript/linked.beatscript` as a document (against the cached vocabulary):

The document below names its `@context` by URL, the default; `jsonld --inline` spells the BeatScript context's terms out instead.

```json
{
  "@context": "https://beatscript.app/context/beatscript-v1.jsonld",
  "@id": "https://beatscript.app/p/z4EBG9j4u6fpnAHQStTXMchz4DxYovWLf9RAfPH2f157ESLSwxg",
  "@type": "Program",
  "cid": "z4EBG9j4u6fpnAHQStTXMchz4DxYovWLf9RAfPH2f157ESLSwxg",
  "euclid": [
    {
      "@type": "bw:EuclideanRhythm",
      "bw:pulses": 5,
      "bw:rotation": 0,
      "bw:steps": 8,
      "sameShapeAs": [
        "https://beatscript.app/ontology/rhythm/lofi-hihat",
        "https://beatscript.app/ontology/rhythm/techno-hihat"
      ],
      "uses": 2
    },
    {
      "@type": "bw:EuclideanRhythm",
      "bw:pulses": 2,
      "bw:rotation": 4,
      "bw:steps": 16,
      "sameShapeAs": [
        "https://beatscript.app/ontology/rhythm/blues-snare",
        "https://beatscript.app/ontology/rhythm/dubstep-snare",
        "https://beatscript.app/ontology/rhythm/edm-snare",
        "https://beatscript.app/ontology/rhythm/lofi-snare",
        "https://beatscript.app/ontology/rhythm/synthwave-snare",
        "https://beatscript.app/ontology/rhythm/techno-snare",
        "https://beatscript.app/ontology/rhythm/trance-snare",
        "https://beatscript.app/ontology/rhythm/trap-snare"
      ],
      "uses": 1
    },
    {
      "@type": "bw:EuclideanRhythm",
      "bw:pulses": 4,
      "bw:rotation": 0,
      "bw:steps": 16,
      "sameShapeAs": [
        "https://beatscript.app/ontology/rhythm/country-kick",
        "https://beatscript.app/ontology/rhythm/edm-kick",
        "https://beatscript.app/ontology/rhythm/house-kick",
        "https://beatscript.app/ontology/rhythm/synthwave-kick",
        "https://beatscript.app/ontology/rhythm/techno-kick",
        "https://beatscript.app/ontology/rhythm/trance-kick"
      ],
      "uses": 2
    }
  ],
  "genre": "https://beatscript.app/ontology/genre/techno",
  "key": "am",
  "sections": [
    {
      "@type": "Section",
      "bars": 8,
      "name": "intro",
      "start": 0
    },
    {
      "@type": "Section",
      "bars": 16,
      "name": "drop",
      "start": 8
    }
  ],
  "tag": [
    "https://beatscript.app/ontology/groove/four-on-floor",
    "https://beatscript.app/ontology/instrument/acid",
    "https://beatscript.app/ontology/instrument/supersaw",
    "https://beatscript.app/ontology/section/drop"
  ],
  "tempo": 128
}
```

## Verbs

| | |
|---|---|
| `btw jsonld {id? , text?}` | the document above |
| `btw terms {kind?}` | the vocabulary: `kind`, `local`, `iri`, `label` of each term, and `version`, `fetched`, `verified` |
| `btw save {..., genre?, tags?}` | `genre`: a term (`""` clears); `tags`: `a, b, c` or a list (`[]` clears); a new object of the same program |
| `djctl btw jsonld FILE` | prints the document; the file's header lines give its links and rights |
| `djctl btw terms [KIND]` | prints `kind  local  label` a line, and where the copy is from on stderr |
| `djctl btw check FILE`, `save FILE` | check says it as warnings; save as errors |

## Canonical form

Nothing in the program's canonical text changes: the header lines are comments and are dropped from it (`btw fmt` returns them in `notes`, as it does
the rights lines), so **no CID moves** and the fixtures that pin existing programs are untouched. What is canonical is the *stored* form of the
links: full IRIs, tags sorted and unique, genre first; the same links in any spelling or order are the same object. The JSON-LD is canonical by
construction (sorted keys, no volatile data); `tests/ext_linked.rs::a_semantic_edit_changes_the_id_and_formatting_does_not` reformats a program
at random and demands the same document, and an edit a different `@id`.

## Decisions

* **The IRIs.** The contract writes `https://beatscript.app/ontology/ns#Techno` and `bw:Techno`. The published ontology has its **classes and properties** in
  `ns#` (`bw:Genre`, `bw:pulses`) and its **terms** at `https://beatscript.app/ontology/<kind>/<slug>` (`.../genre/techno`, `.../instrument/acid`), where
  each is a page (Turtle, N-Triples, HTML). I store and export the published term IRIs; `bw:Techno`, `bw:techno` and `ns#Techno` are accepted spellings of
  the genre `techno` (matched by local name, any kind), so what the contract wrote works.
* **Warning in check, error in save and import.** A header line is a comment, so a typo must not stop a live program; keeping a typo in a saved object
  would be a lie that persists. (`linked::check_headers` takes the severity.)
* **The header block is the leading run of `#!` lines**, because that is what `live_store` already reads, and a line elsewhere is not read (and is said so).
* **Tags are a set, stored sorted as IRIs.** The same set is then the same object. A tag may be any term kind; a bare name two kinds share goes to the first
  kind in `KIND_ORDER`. A tag the vocabulary lacks (`driving`, `303`: the contract's examples) is an error with a did-you-mean: the vocabulary is closed.
* **Commas split tags**, so a term whose label has a comma in it (the Euclidean rhythms' labels, `techno kick: E(4,16) rotation 0`) is written by its
  local name (`techno-kick`), `rhythm:techno-kick` or its IRI.
* **IRIs in the file, short names allowed in.** An exported file always has full IRIs (unambiguous, readable by any tool, no vocabulary needed to read it);
  a human may write `techno`. Reading is lenient, writing is canonical.
* **`@id` is the program's page** `https://beatscript.app/p/<CID>`, rather than an unregistered `urn:cid:`; the terms are the BeatScript context's `bs:` ones.
  Until 2026-11-23 the `@id` was `https://beats.bitwrap.io/p#<CID>` and the terms `djrust:` (`https://beats.bitwrap.io/ns/djrust#`, never published):
  BeatScript data lives on beatscript.app only.
* **`btw jsonld {text}` claims rights only if the text's header has them**, so a bare program does not come out CC0 by default.
* **`sections` are the song's sections**, one entry each time the arrangement places one (a section arranged twice is listed twice), in the order of their start
  bars (two stacked ones start on one bar, in the order of the normal form), with the section's own bars and `start`. They are read from F10's placements
  (`Lowered::placements`, kept as `Compiled::linked.placements`), not from the compiled segments, which are the *stretches* a stack cuts the song into (`groove+wash`,
  then `groove`). A program with no `section` has no `sections` key (its length is not a section); one the placements do not cover (a bare top-level `evolve`, a
  flat program chained from nets of 64 bars) lists its segments with a chain as one.
* **A section's `key` is the one it sets itself**, not the key it happens to play in: a section that sets none inherits the program's, and in a stack takes the
  key of the stretch it is in (`Compiled::section_key`, F5/F10), which depends on what it is stacked with and can change within it. The program's `key` and
  `tempo` are the ones it says (`tempo 100 + 20` is 120: the evaluated number, not the text).
* **`name` only for a shape the ontology names once**; `sameShapeAs` for all of them.
* **Tags and genre are not in the program text, not in the share link.** A link seals the canonical text; links to the ontology are the author's statement
  about it, kept with the saved object (and in the document), as the licence is.
* **The `.jsonld` is named by the object CID**, pairing with `<object cid>.beatscript`; a new object (a licence, a tag) is a new pair.
* **`terms` is a verb** (read only) so an agent can find what to write, besides `djctl btw terms`.

## Not done

* **No refresh from inside the app.** The snapshot is refreshed by the script and committed; there is no in-app "update the ontology" (it would make the
  run time depend on the network). A refresh that drops a term leaves old programs readable but their next save asks for a new term.
* **Folk names of rhythms** ("tresillo", "cinquillo") are not in the ontology, so the document has none (F6 decided the same).
* **A tempo that moves** (F3 curves) is not in the document: `tempo` is the program's own setting.
* **No SHACL / OWL validation** of the document.
* **Genre-aware checks** (tempo against the genre's `bw:bpm`, key against its scale) are not done; the vocabulary carries them, the snapshot does not.
* **The window** (`djapp/src/live_panel.rs`) does not show the links; they are in `btw list` / `btw get` rows and the files.
* The **Bitwrap Ontology MCP tools** were denied in the unattended run that built this; the published files were fetched with `curl` (and checked against the
  release's checksums) instead.

## For later workers

* `djlive::linked`: `Ontology`, `StaticOntology`, `Links`, `find`, `canonical_links`, `check_headers`, `header_lines` / `header_end`, `Structure` / `Placed`,
  `sections`, `jsonld`, `pretty`, `Facts`. `Env::ontology`, `Lowered::placements` and `Compiled::linked` are the shared-file fields. `compile::check_with` calls `check_headers` and `linked::structure` once.
* **F6:** the document's names and `sameShapeAs` come from `meter::euclid_names`; a vocabulary change there (a new individual) shows in the document and in `check`'s notes together.
* **`#! skip-test:`** (the examples harness) is another header line: `linked::header_lines` lists it with the rest and nothing here reads it; `examples_beatscript.rs` finds it anywhere in the header block.
* A `btw` verb's registry entry, its handler arm in `djapp/src/control/live.rs` and its `ACTIONS_MORE` entry go together: `jsonld`, `terms` are examples.
