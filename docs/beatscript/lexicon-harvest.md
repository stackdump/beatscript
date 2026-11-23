# The lexicon harvest: pronunciations from programs, so the CC0 lexicon grows

*Roadmap 6665 (harvest) as amended by 6666 (an MCP side effect), the first slice of 6659 (the diagnostics log). Store:
`crates/beatscript-serve/src/diagnostics.rs`. Pairs: `djengine::vocal::lexicon::lyric_pairs`.*

A word the BeatScript lexicon (`crates/djengine/data/lexicon.txt`, CC0) does not know is sounded out by rules, and a
writer who hears it wrong fixes it with an escape. Written as an **annotated escape**, `word=/phonemes/`
(`jingle=/j ii ng g uh l/`, [phonemes.md](phonemes.md)), the fix says which word it spells, and beatscript.app collects it:

| pair | source | signal |
|---|---|---|
| `jingle=/j ii ng g uh l/` in a lyric | `annotated` | strong: a person or agent wrote the pronunciation down |
| `zorblax` in a lyric, a word no lexicon knows, with the rules' `/z aw r b l ae k s/` | `rule` | weak: what the rules guessed |

A plain escape (`/j ii ng g uh l/`) has no word, so it gives nothing; a word cut with hyphens (`lan-tern`) is the writer
spelling it already, and gives nothing either; a word the lexicon knows is not a pair.

## What is kept, and what never is

Per word, each pronunciation it was given: the phonemes (as a lexicon line writes them, `j ii ng g uh l`), the source, how
many distinct programs gave it, when it was first and last given (a new program), and up to 5 of those programs' CIDs.

**Never:** the program's text (a stored CC0 program is re-derivable from its CID), anything about the caller (account,
token, address), and nothing at all from `push_private`, a program that uses `sample` or `loop` assets, one that says
`use restricted`, or one whose `#! visibility:` is not public. A program that does not check gives nothing (it has no CID
yet); its pairs are taken once it checks.

The file is append-only JSON lines, `<data>/diagnostics/lexicon-harvest.jsonl`, one record a new (word, phonemes, source,
program) sighting:

```json
{"kind":"lexicon_pair","t":1795372800,"word":"jingle","phones":"j ii ng g uh l","source":"annotated","cid":"z4EBG9j…"}
```

The same program checked again adds nothing, so a count is a count of programs, and the file grows only with new
programs. `kind` leaves room for the rest of 6659's diagnostics (warnings, errors) in the same log. The server rebuilds the
counts from the file when it starts. Bounds: 40 characters a word, 64 pairs a program, 50,000 words.

## As a side effect of the MCP tools

Every BeatScript MCP tool that checks a program records its pairs, with no flag and no extra call: `check`, `eval`,
`load`, `listen`, `render`, `share`, `queue` (each item added), `genre_template` and `generate`. `push_private` records
nothing. Each of those answers carries a short `lexicon` field, `{"harvested": 1, "new": 1}`: the pairs taken from this
program, and how many of them (a word, a pronunciation and a source) no program had given before. The tools' descriptions
end with the same words, from one constant (`mcp::text::LEXICON_DESC`; a test holds this page to it):

> Lexicon harvest, a side effect with no flag: when the program checks and may be kept (no samples or loops, no `use restricted`, public visibility; push_private records nothing), the (word, phonemes) pairs of its lyrics are recorded to grow the CC0 lexicon: each annotated escape word=/phonemes/ (jingle=/j ii ng g uh l/ sings the escape and keeps the word) and each word no lexicon knows, with the rules' pronunciation. Only the word, its phonemes and the program's CID are kept, never the text. The answer's lexicon is {harvested, new}: the pairs taken from this program, and how many no program had given before. When check says a word is not in the lexicon and it sounds wrong, write it as word=/phonemes/.

The lyrics are read from the program's tokens (`djlive::sing::lyrics`: each `sing "..."` and `lyrics="..."`); a
phrasebook line (`sing @id`) is the phrasebook's, already in phonemes, and gives nothing. A tool that loads a stored
program or a link (`load`, `render` by CID, `queue`, `genre_template`, `generate`) checks it again for the gate, unbilled,
and only when it has a pair to record. `record` renders through `render`, so it harvests too, without the field.

## Review: read-only, and the weekly proposal

Nothing the harvest holds changes the lexicon by itself. Two ways to look at it:

- **MCP:** `list_examples` with `{"lexicon": true}` (folded into an existing tool, beside `phrasebook` and `themes`) adds
  `lexicon: {words, total, note}`: the top 50 words, those with an annotated pronunciation first, then by programs. Each is
  `{word, programs, conflict, in_lexicon, variants: [{phones, source, count, first, last, cids}]}`; `conflict` is two
  annotated pronunciations that disagree, and `in_lexicon` is the lexicon's own pronunciation when it already has the word.
- **The weekly proposal:** a val job ([`jobs/lexicon-harvest-weekly.md`](../../jobs/lexicon-harvest-weekly.md), a spec, not
  registered) ranks the harvest, annotated pairs first, writes a proposed diff of `lexicon.txt` with counts and example CIDs
  on a `proposal/lexicon-<date>` branch, and sends conflicts to Matt to decide. Agreement between independent annotations
  raises confidence; Matt approves every line. `tools/lexicon/propose.py` does the same from programs' text, by hand.
