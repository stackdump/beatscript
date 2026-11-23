# Found phrases: borrowing text for its shape

A found phrase is a line of public-domain text borrowed for its **shape**, not its meaning: its meter, its stresses, its
syllables, its phonemes. A trochee lines up with a four-on-the-floor kick. A line of pentameter becomes a buried texture
that now and then comes forward. The words may leak through, and are not the point. It is the lineage of sampling, cut-ups
and readymades, with one difference: what is borrowed is structure, not a recording, and only from sources that are clean
by construction (first published before 1929, curated, each with its source and year on the record).

The technique in one line: **pick a line for its meter, match the meter to the groove, write the vocal so the stresses land
where the meter says, keep it low, and let it cycle between buried and spotlit.**

Everything here is one source with many views: the corpus is the [phrasebook](/phrasebook) (a CC0 file and its linked data,
by id and by CID); `sing @id on melody` sings a line from it; `arrange from template(deep-house, @id)` compiles one to a whole
track (`docs/beatscript/ext-template.md`); the MCP `list_examples` tool lists the phrasebook (by meter or syllables) and
`genre_template` takes a `phrase`. This page is the technique they all share. Its melodic twin is the [theme library](/themes), and the [forms](/docs/forms) (quodlibet, passacaglia,
canon, rondo, fugue, theme and variations) are whole shapes built from both.

## 1. Pick a line for its meter, then match the meter to the groove

Every phrasebook line carries its **meter**, a **stress pattern** (one step a syllable, `x` stressed and `~` not, read the same
way as `deck A: hihat ~ x ~ x`), its **syllable count**, and its **feel** (`straight`, the sixteenth grid, or `triplet`,
groups of three). The meter says which groove it sits on:

| the line's meter | its stress | the groove | a line from the book |
|---|---|---|---|
| **trochees** (`trochaic`, `x ~ x ~`) | a stress on every other syllable, starting on the beat | chant on the kick: techno, EDM, four on the floor | Blake's "Tyger! Tyger! burning bright" (`tyger_burning_bright`, 1794) |
| **iambs** (`iambic`, `~ x ~ x`) | the stress is the second syllable, so the line lilts on the offbeat | house, deep house, anything with an offbeat hat | Prospero's "we are such stuff as dreams are made on" (`tempest_such_stuff`, 1623) |
| **anapests** (`anapestic`, `~ ~ x`) and **dactyls** (`dactylic`, `x ~ ~`) | a stress every third syllable | a triplet feel: drum and bass, breakbeat, shuffles | Byron's "The Assyrian came down like the wolf on the fold" (`byron_assyrian_wolf`, 1815); Longfellow's Evangeline (`evangeline_forest_primeval`, 1847) |
| **common meter**, 8-6-8-6 (`common-meter`) | alternating four and three stresses | fills a 16-bar section: a line a four-bar phrase | Dickinson, "Because I could not stop for Death" (`dickinson_chariot_death`); Newton, "Amazing Grace" (`newton_amazing_grace`, 1779) |
| **shanties** (`call-and-response`) | the book's `call` says how many of the first syllables are the leader's | call and response: the leader, then the crew | `shanty_drunken_sailor`, `shanty_blow_the_man_down` |
| **psalm parallelism** (`parallelism`) | free; the second half says the first again, changed | repeat plus variation: the same figure twice, the second answered | KJV Psalm 42:7, "Deep calleth unto deep" (`psalm42_deep_calleth`, 1611) |

The other families are `sprung` (Hopkins: stresses, not syllables, are counted), `nursery` (a clapping or lullaby dimeter) and
`oratory` (the Gettysburg Address: speech as rhythm). The table is a rule of thumb: a stress pattern is a **step pattern**, so
the quickest test of a match is to read it beside the drums.

```text
tyger_burning_bright   x ~ x ~ x ~ x x ~ x ~ x ~ x      the stresses fall on the kick's steps 0, 4, 8, 12 ...
deck A: kick           x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
```

## 2. Write the vocal line so the stresses land where the meter says

A line's stresses go on beats (steps divisible by 4 in the straight feel, by 6 in the triplet feel, where a bar is twelve
sixteenths, `melody/12`), and its weak syllables fill the eighths between. A line that starts weak (an iamb) takes a **pickup**:
it starts a step or two late so that its first stress is on a beat. Give a stressed syllable a higher accent (`!`) so that a
quiet voice still shows the meter.

You do not have to do this by hand. `sing @tempest_such_stuff on melody` sings the book's line on a part's own notes, and
`arrange from template(deep-house, @tempest_such_stuff)` writes the whole vocal line out, pickup and accents included, as plain
BeatScript you can read and edit (`beatscript expand`, or the MCP `expand` tool). Reading what it wrote is the quickest way to learn
what the rules are.

## 3. Keep the vocal low

Keep a sung line **low, about A3 to F4**. Higher up, the formant voice smears: the consonants that carry the meter are lost and
the line stops being a rhythm. Octave up only for a moment (`@ vox_* oct 1` for one section), never as the part's home.

## 4. Cycle buried and spotlit

The line should not be heard the whole time. Cycle two kinds of section:

- **night**: a full mix in which the vocal is *texture*: the voice buried (`mix { B level 0.45, B eq high -9 }`, put back where
  the section ends), ducked under the kick (`duck B under A.kick 3db`), the pad and the kalimba above it.
- **spot**: a sparse section in which the vocal *comes forward*: the voice up (`B level 0.95`), the pad and the instruments
  dropped under it.

Between them, change one thing at a time with a one-shot `@`, which applies to the next section only: `@ vox oct 1` brings the
voice up an octave for one `spot`, then it goes home.

```text
arrange {
  night
  @ vox oct 1
  spot
  ~ drm_* fill 0.15
  night
  @ vox xfer drm_shk
  spot
  ~ drm_* grow
} loop 4 then end
```

`beatscript listen` says it as numbers: in the deep-house template the sung part is 5 dB under the pad in `night` and 3 dB above it
in `spot`.

## 5. Cross the vocal onto the drum rhythms

`@ vox xfer <drum>` plays the vocal's notes on another part's rhythm: the voice **chants on the kicks** (`@ vox xfer drm_kick`)
or **skips on the hats** (`@ vox xfer drm_hat`, `drm_shk`), for that section only, and the stresses move with them.

Keep the **kick outside the `drm_*` glob**. The arrangement mutates `drm_*` (`~ drm_* grow`, `~ drm_* fill 0.15`) and leaves
anything else alone, so name the steady four-on-the-floor `kick4` and the backbeat `clap24`: they stay put while the rest of the
drums grow, and the voice has a floor to stand on. The mutations that only add hits never leave the shaker with fewer notes
than the vocal has syllables, so no syllable is left unsung (a line longer than a shaker's hits in a section is the `sing`
warning "the last N are not sung").

## 6. Split a two-line quote across sections

A quotation of two lines is two sections' worth: put the first line in one section and the second in the next, so that **the
second answers the first**. In a phrasebook line the slash is the break (`we are such stuff as dreams are made on / and our
little life is rounded with a sleep`). Sing the first half in `night` and the second in `spot`, or the question on the kick and
the answer on the hats (`@ vox xfer`). A shanty's `call` column is this made explicit: the first `call` syllables are the
leader's, the rest is the crew's answer.

In a lyric string a lone `/` with whitespace on both sides is a phrase break, so `sing "deep er in to night / hold me close /
we move slow" on melody` reads three phrases and not an escape.

## 7. Fix the pronunciation with phoneme escapes

A word the lexicon does not know is sounded out by rules, and `check` warns and says how to respell it. Write the sounds you
want between slashes: `/dh uh/` for *the*, `/t ie g er/` for *Tyger*, `/b r ie t/` for *bright*, and `-` joins the syllables
of one word. Write the word before the escape with `=` (`tyger=/t ie g er/`) and it is kept: the escape is sung, the program still
says which word it spells, and the tools collect the pair for the lexicon (`phonemes.md`, `lexicon-harvest.md`). The phonemes are `docs/beatscript/phonemes.md`'s. Every phrasebook line already comes with its escapes (`lyric`), so
that `sing @id on melody` checks with no unknown-word warnings; do the same for your own words and run `check` until it says
nothing about them.

```text
deck B: sing "/t ie g er/, /t ie g er/, /b er n ii ng - b r ie t/, in /dh uh/ /f aw r ii s t s/ of /dh uh/ night" on melody voice=robot-choir
```

## Reference programs

Two whole tracks use the technique. Both are CC0-1.0 and carry their source in a comment and in their JSON-LD (`isBasedOn`).

| program | the line | meter, groove | CID |
|---|---|---|---|
| `such-stuff-as-dreams-122` | Prospero, *The Tempest* 4.1 (1623): "we are such stuff as dreams are made on / and our little life is rounded with a sleep" | iambic pentameter; deep house at 122, the lilt on the offbeat | `z4EBG9jAv9C8XQ55DMd9iGXybFcMbX93mrXjwrNLVLuKUe5p1QK` |
| `tyger-tyger-128` | Blake, *The Tyger* (1794): "Tyger! Tyger! burning bright / In the forests of the night" | catalectic trochaic tetrameter; four on the floor at 128, the chant on the kick | `z4EBG9jB6T7bKvVFQ6ZW6UPPCYh2RQbAHx86rZ8EKczfdupgqnn` |

The programs are `examples/beatscript/such-stuff-as-dreams-122-reference.beatscript` and `examples/beatscript/tyger-tyger-128.beatscript`, each with its
source in a comment and in a `.jsonld` beside it. `check` warns of a few words the lexicon lacks (`such`, `stuff`, `dreams`,
`little` and `rounded` in the first; `burning` in the second); both are pinned as written, because an escape would change the
code and so the CID. The first has a template twin: `arrange from template(deep-house, @tempest_such_stuff)` writes the shape
of `examples/beatscript/such-stuff-as-dreams-122.beatscript`.

## Rules of the found

- **Pre-1929 public domain only**, each line from a pre-1929 edition it names (no modern translation, edition or transcription),
  and each with its work, author, year and edition on the record (`/phrasebook`, and in its JSON-LD).
- **Structure, not content**: the meter, stress, syllable count, phonemes, call and response and (later) rhyme and line length
  are what is borrowed. Each must map plainly to a musical structure.
- **Provenance travels**: a program that sings a found line says where it came from in a comment, and its JSON-LD keeps it.
- **The corpus is data**: new lines and new meter families are a change to `crates/djengine/data/phrasebook-pd.txt`
  (a new corpus version, a new CID), not to the language.

## For an agent

1. `list_examples` with `phrasebook` true lists the phrasebook's lines, with `meter` (a family, or a word such as
   `pentameter`) and `syllables` (`8` or `8-12`) to narrow them: each line comes with its text, source, year, stress pattern,
   syllables and phoneme escapes.
2. Pick a line for the groove you are writing (the table above).
3. Write `sing @id on melody`, or start from `genre_template` with `phrase` set to the id (or to any words): the line is sung
   on each section's melody part, and the answer says how well its meter fits the genre.
4. `check` until the lyric has no unknown-word warning; `listen` to hear that the vocal is where you meant it, buried in
   `night` and forward in `spot`.
