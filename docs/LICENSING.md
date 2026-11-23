# Licensing, visibility and lineage of tracks and sets

*Beyond Swift.* djrust is heading towards "GitHub for music": people find tracks (through MCP
among other ways), fork them and remix them. For that to work, every **user-made** object carries
its own terms. The license is **inside** the object, so it is part of the object's content ID
(CID) and is carried by every fork.

Code: `djengine::rights` (the block: pure, `no_std`), `djhost::tracks` (the track object and its
store), `djhost::setlist` (sets carry the same block), `djapp/src/tracks_panel.rs` (the Tracks
window and the shared pickers), `djapp/src/control/tracks.rs` (the `track` verbs),
`djctl/src/track_cmd.rs` (`djctl track`).

## The rights block

Every user-made track and set has these fields at the top level of its canonical JSON:

| field | values | default | meaning |
|---|---|---|---|
| `license` | `CC0-1.0` · `CC-BY-4.0` · `all-rights-reserved` | `CC0-1.0` | what others may do with it |
| `visibility` | `public` · `private` | `public` | whether it may leave this machine |
| `author` | a signer identity (`did:key:z…`, `0x…`) | none (`$DJ_AUTHOR` if set) | who made it |
| `forkedFrom` | the parent's CID | none | set on a fork / remix |
| `credits` | `[{cid, author, license, name}]` | none | the lineage: every parent it was forked from, and every CC BY work it uses |

The two fields are independent: a private CC0 track is fine, and so is a public track with all
rights reserved.

| license | id | reuse | fork |
|---|---|---|---|
| CC0 1.0 (the default) | `CC0-1.0` | anyone, no credit needed (public domain dedication) | fine |
| CC BY 4.0 | `CC-BY-4.0` | anyone, crediting the author | fine; the fork **must** credit the author, and does so automatically |
| All rights reserved | `all-rights-reserved` | none without permission | allowed (it may be your own), but **warns** |

**Never "unlicensed".** The word appears nowhere in the code, the UI or the JSON: it reads like the
*Unlicense*, which is a public-domain dedication, which is the opposite of what it would mean here.
`all-rights-reserved` says what it means. An object whose `license` is not one of the three ids
exactly (not `cc0`, not `unlicensed`) is refused when it is read. The verbs and the CLI accept a
loose spelling (`cc by`, `arr`) and write the id.

**Visibility.** A `private` object is never shared by CID and never uploaded. The one gate is
`Rights::check_shareable`: `track share`, `setlist share` and `djctl track share` all go through
it and answer *"it is private: private tracks and sets are never shared or uploaded"*. Any later
upload path (publishing a track or set to the CDN, a djnet send of a saved track) must call it too.

## Into the CID

The block is written into the canonical JSON (sorted keys, JS numbers) that the `z…` CID hashes.
So:

- **changing the license or visibility makes a new object with a new CID.** The stores replace
  the old file with the new one (`TrackStore::set_rights`, a set's Save);
- a fork copies the block forward, so **the license survives the fork**;
- the music itself does not move. A track's `spec` is the deck's share payload, and its
  `music_cid` (the payload's own share CID, the one a share link names) is the same whatever the
  rights are. Swift parity of share CIDs is untouched.

Sets saved before sets had rights have no block. They read as CC0 / public and **keep the CID they
were saved under**: `SetList::rights` is `None` until a license or visibility is set, and a `None`
writes nothing. Every new set (`SetList::new`, the generator) carries the block explicitly.

## Tracks: `djrust.track`

```json
{"format":1,"type":"djrust.track","name":"Night Bus (remix)",
 "spec":{"cohesion":"v2","genre":"dnb","seed":82819},
 "license":"CC-BY-4.0","visibility":"public","author":"did:key:zBob",
 "forkedFrom":"z4EBG9jFxJJ…dW4cXT",
 "credits":[{"author":"did:key:zAlice","cid":"z4EBG9jFxJJ…dW4cXT","license":"CC-BY-4.0","name":"forked from: Night Bus"}]}
```

Kept as `<cid>.json` in `<support>/tracks` (`$DJ_TRACKS_DIR`; on macOS
`~/Library/Application Support/djrust/tracks`). A file whose name is not its content's CID is not
listed, as with the set lists.

## Vocal lines: `djrust.vocal`

Typed lyrics sung by the formant voice (docs/VOICE.md, "Typed lyrics") are the same kind of object: the
rights block rides inside the canonical JSON, **CC0 and public by default**, changing the license is a new
CID, a fork credits its parent, a private line is never shared. Kept as `<cid>.json` in
`<support>/vocals` (`$DJ_VOCALS_DIR`). The sung audio can be CC0 because nothing in it is recorded or
cloned: the phonemes come from the BeatScript phoneme set, lexicon and phrasebook (CC0, written for this project,
docs/beatscript/phonemes.md) and rules written here, and the voice is synthesised. (The desktop booth alone also consults the
CMU Pronouncing Dictionary, BSD-2-Clause, in the private crate `djlexicon-booth`; no public build links it.)

The one data file whose words are not written for this project is the public-domain phrasebook (`crates/djengine/data/phrasebook-pd.txt`,
docs/beatscript/phonemes.md, "The public-domain phrasebook"): each line is a text first published before 1929 and quoted from the pre-1929
edition the line names, so it is in the public domain; the file around the texts (the lyrics' phoneme spellings, the meters, the notes) is
written here, and the file is CC0. No modern edition, translation or transcription is quoted.

## Lineage, forks and attribution

`Rights::fork(parent_cid, parent_name, author)` gives the child:

- the parent's `license` and `visibility` (the person may change either afterwards);
- `forkedFrom` = the parent's CID;
- `author` = whoever forked it (`author` param, else `$DJ_AUTHOR`);
- `credits` = the parent's credits, **plus the parent itself, whatever its license** (its CID, its
  author, its license, and its name after the note `forked from`). So the original author stays
  in the lineage while `author` names the forker. For a CC BY 4.0 parent that one credit is also
  the attribution the license asks for. A parent that names no author adds no credit, unless it is
  CC BY: then it is credited by its CID alone. Credits are inherited by every later fork and no
  verb removes them. A chain Alice → Bob → Carol credits Alice and Bob, whether they chose CC0 or
  CC BY. The same work by the same author is never credited twice. (A program's fork names its
  parent by the program's CID, which an unedited fork shares with its own parent, so the author is
  what tells those two steps apart.)

A credit for a fork parent is marked in its `name`: `forked from`, or `forked from: Night Bus` when
the parent had a name. A saved program writes it as a header line:

```
#! credit: z4EBG9... | did:key:zAlice | CC0-1.0 | forked from: night bus
```

An object that uses another without forking it (a program that imports a snippet or borrows a
section) adds that object's credits, and the object itself only when it is CC BY.

Warnings (returned by the verb, printed by the CLI, shown in the window):

- the parent is **all rights reserved**: "its author has not granted reuse, so this fork may not
  be shared or published without their permission";
- the parent is CC BY but names **no author**: it is credited by its CID alone.

**Signer identity.** `author` is the identity that signs the object: the wallet's `did:key:z…`
(djrust-wallet, in the public djrust repo), or an `0x…` address. Nothing in this repo signs yet;
when an object is signed, a verifier must check that `author` is the signature's issuer, and a
credit names the parent's `author` as its signer signed it. Until then `author` is a claim, set
from the `author` param or `$DJ_AUTHOR`.

**What is not covered.** Generated deterministic snapshots (genre + seed + structure: the
wallet's `beats-snapshot.v1`) stay CC0 by definition, as decided in the 2026-11-10 schema. They are
nobody's to license. Discovery and chart claims stay attribution-only. Saving a deck as a *track*
is what makes a user-made object with a choice of license.

## Where it shows

- **The Tracks window** (top bar, `windows open window=tracks`): a save row with a name, a license
  picker and a visibility picker (CC0 1.0 + public until changed), **Save A** / **Save B**; each
  saved track with its license badge (coloured, the license's meaning and URL on hover), its
  `forkedFrom`, its credit line ("Contains work by did:key:zAlice · forked from “Night Bus”
  (z…dW4cXT), CC BY 4.0"), pickers to change the license and visibility, →A / →B, Fork, Share
  (copies the CID; refused when private) and ✕.
- **The deck header**: when a deck plays a saved track (its share CID is a saved track's
  `music_cid`), the track's name, its license badge and its credit line under the title.
- **The Set window**: each saved set's badge, a Fork button, and license / visibility pickers and
  the credit line in the editor.

## The control language

`track` (MCP tool `djapp_track`; docs/CONTROL.md):

| verb | params | does |
|---|---|---|
| `list` | | the saved tracks with their rights, which one each deck plays |
| `get` | `id` or `deck` | one track: object, CIDs, rights, credit line |
| `save` | `deck`, `name`, `license`, `visibility`, `author` | save what a deck plays (CC0 / public by default) |
| `license` | `id`, `value` | read it; with `value`, set it (a new CID) |
| `visibility` | `id`, `value` | read it; with `value`, set it (a new CID) |
| `fork` | `id`, `name`, `author` | save a remix: lineage, credits, warnings |
| `share` | `id` | its CID, music CID and object; refused when private |
| `load` | `id`, `deck` | play it on a deck |
| `delete` | `id` | |

`setlist` gains `license {value, author, id}`, `visibility {value, id}` (on the working set;
`id` reads a saved set's), `fork {id, name, author}` and `share {id}`.

`djctl app track license id=Night value=CC-BY-4.0` drives the running app. `djctl track …` works
on the store directly with no app running:

```
djctl track new dnb:82819 --name "Night Bus" --license CC-BY-4.0 --author did:key:zAlice
djctl track fork "Night Bus" --author did:key:zBob
djctl track list
djctl track visibility "Night Bus" private
djctl track share "Night Bus"          # djctl: track z…: it is private: … never shared or uploaded
djctl track check some-track.json      # reads a track object, prints its CID and rights
```

## Tests

- `djengine/tests/rights.rs`: the default is CC0 / public; the ids round-trip; "unlicensed" is
  never accepted; private refuses; the license survives a fork; all-rights-reserved warns; every
  fork credits its parent's author (CC0 too), marked `forked from`, and a CC BY parent once; a fork
  of a fork credits both; a parent with no author adds none; credits are inherited down a chain
  and not duplicated; an import credits only what its license asks.
- `djhost/src/live_store.rs`: a program's fork keeps the original author in its lineage, its
  program CID does not move, and the `#! credit:` header lines round-trip through export, load
  and import.
- `djhost/src/tracks/tests.rs`: a new track is CC0 / public in its JSON; the CID changes with the
  license and with the visibility and the music CID does not; private refuses sharing; the
  license survives a fork; the CC BY credit is in the forked object and in its share; the store
  saves, relicenses (replacing the file), forks and deletes.
- `djhost/src/setlist/tests.rs`: the pinned set CID now includes the block; a set saved before
  rights keeps its CID and reads as CC0 / public; a set's CID changes with its rights and a fork
  keeps them.
- `djapp/src/app_tests.rs`: the `track` verbs end to end through the control path, and the
  window's buttons, badges and credit line; the `setlist` license / visibility / fork / share
  verbs.
