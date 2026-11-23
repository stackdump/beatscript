# Open data

Every public CC0 BeatScript program is open data. You can fetch it in every format, find it without searching, and
download all of it at once. Anyone may mirror, index, study or reuse it: CC0 puts it in the public domain.

Every route here is a plain `GET` (or `HEAD`). They need no sign-in, send no cookies, and answer cross-origin requests
with `Access-Control-Allow-Origin: *`.

## What is published, and what never is

A program is published when all of these hold:

- it was shared publicly;
- its licence is **CC0-1.0** (the default);
- its text hashes to its CID.

These never appear in a feed, the sitemap or a dump:

- a private program;
- a program whose `#! visibility:` line says it is not public;
- a program under **CC BY** or **all rights reserved**;
- a program that uses samples, or `use restricted`. These are never stored at all.

A public program under another licence still has its page and its formats (below), carrying its licence. It is just
left out of the bulk channels.

## One program, every format

A program's name is its CID: the hash of its canonical text.

| URL | what |
|---|---|
| `/p/<CID>` | its page: a link preview, and the program as a `<beatscript-view>` block |
| `/p/<CID>.beatscript` | its **canonical text**, `text/plain`. This is exactly what the CID hashes |
| `/p/<CID>.jsonld` | its linked data (`application/ld+json`) |
| `/api/render/<CID>?format=opus` | its audio, the offline render (`format=wav` for WAV) |

`/p/<CID>` also answers by `Accept`:

| `Accept` | answer |
|---|---|
| `text/plain` or `text/x-beatscript` | the `.beatscript` |
| `application/ld+json` or `application/json` | the `.jsonld` |
| `audio/*` | `303 See Other` to the render |
| anything else (a browser) | the page |

The page names its other formats in `<link rel="alternate">` tags in its head, and its licence in `<link
rel="license">`. The `Link` header says the same. All of a page's existing preview tags (`og:*`, `twitter:*`, title,
description, canonical) are unchanged.

### Licences are URIs

In the linked data, `license` is a URI and `licenseId` is the SPDX id beside it:

| `licenseId` | `license` |
|---|---|
| `CC0-1.0` | `https://creativecommons.org/publicdomain/zero/1.0/` |
| `CC-BY-4.0` | `https://creativecommons.org/licenses/by/4.0/` |
| `all-rights-reserved` | `http://rightsstatements.org/vocab/InC/1.0/` (In Copyright) |

### Links to the bitwrap ontology

Where the [bitwrap ontology](https://ontology.bitwrap.io/ns#) has a term, a program's linked data links to it:

- `scale`: the key's mode, e.g. `key am` gives `https://ontology.bitwrap.io/scale/minor`;
- `instrument`: the voices its parts name that are ontology instruments, e.g. `inst=acid`;
- `genre`: from `#! genre:`;
- `tag`: from `#! tags:`.

A value with no term stays a plain value. The terms are defined at [/schema](/schema), whose `@vocab` is
`https://beatscript.app/schema#`.

## Finding programs

| URL | what |
|---|---|
| `/feed.json` | [JSON Feed 1.1](https://jsonfeed.org/version/1.1): the 50 newest public CC0 programs |
| `/feed.atom` | the same as Atom 1.0 |
| `/sitemap.xml` | every public CC0 program's page, with the date it was shared |
| `/robots.txt` | allows everything, and names the sitemap |
| `/llms.txt` | the site for a language model, linking all of this |

Each feed item is a program:
- **title**: its name, or `BeatScript` and the end of its CID;
- **link**: its page;
- **id / guid**: its page URL;
- **date**: when it was shared;
- **text**: its canonical text;
- **enclosures / attachments**: its `.beatscript`, `.jsonld` and render.

The JSON Feed's `_beatscript` member carries the CID and licence of each item, and at the top it links the dump, the
sitemap, the Atom feed and the schema. The item shape follows beats.bitwrap.io's `/feed.rss`.

## All of it at once: the dumps

`/dumps/latest.json` points to the newest archive of every public CC0 program. `/dumps/` lists every archive.

An archive is a `.tar.gz` named by its own hash: `beatscript-cc0-<first 16 hex of its sha256>.tar.gz`. It holds:

- `manifest.jsonld`: `@type` `["ProgramDump", "Dataset"]`, the `count`, and the sorted list of `cids`;
- `programs/<CID>.beatscript`: each program's canonical text;
- `programs/<CID>.jsonld`: each program's JSON-LD (the `<beatscript-view>` block's document).

The bytes are deterministic. Files are sorted by CID, every tar header is the same except name and size (mtime 0,
uid 0), the gzip header has no name or time, and the manifest says nothing about when it was made. So the same
programs give the same archive and the same name, and a dump with nothing new changes nothing. The pointer's
`createdAt` says when an archive was first made.

To check an archive:
1. its sha256 must match its name;
2. each `programs/<CID>.beatscript` must hash to its CID (`{"format":1,"text":<the file>,"type":"djrust.live"}`, sha2-256,
   CIDv1 dag-json, base58btc);
3. each `.jsonld`'s `@id` must be that CID.

The manifest follows beats.bitwrap.io's snapshot manifest (`version`, `count`, `cids`, `docs`).

### Making a dump

`beatscript-dump`, a binary of the beatscript-serve crate, reads the programs store (it never writes there) and writes the archive and
`latest.json`:

```text
beatscript-dump -programs <data>/programs -out <data>/dumps -public-url https://beatscript.app
```

The server serves what it finds in `-dump-dir` (default `<data>/dumps`). `tools/open-data/` in the engine repo has the
job definition, which nothing schedules yet.

## The public-domain phrasebook

Besides programs, the site publishes one curated corpus: lines of verse, scripture, speech and work song, each first published before
1929, to sing by id (`sing @tempest_such_stuff on melody`). The file is CC0, and every text in it is in the public domain. Each line
has its source, its meter as a step pattern and the lyric BeatScript sings it as; `docs/beatscript/phonemes.md` ("The public-domain
phrasebook") says what every field is.

```text
https://beatscript.app/phrasebook                  the page: every line with its year, meter, stress and a sing example
https://beatscript.app/phrasebook.txt              the corpus, exactly as shipped (text/plain)
https://beatscript.app/phrasebook.jsonld           the same as linked data (application/ld+json), one node a line
https://beatscript.app/phrasebook/<CID>.txt        the file, by CID
https://beatscript.app/phrasebook/<CID>.jsonld     the linked data, by CID
```

The corpus's CID is the CID of the file's text, by the function a program's CID is (`{"format":1,"text":<the file>,"type":"djrust.live"}`,
sha2-256, CIDv1 dag-json, base58btc), so a copy can be checked the way a program is. The two routes with a CID in them answer only
for the CID the server ships and are `404` for any other: a new version of the corpus is a new CID, never a change under an old one.
