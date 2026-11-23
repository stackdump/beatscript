# Phrase templates (`crates/djlive/src/template.rs`)

`template(name, phrase)` compiles a phrase to a whole track. The phrase is a phrasebook id (`@tempest_such_stuff`) or any
words in quotes; the template is a BeatScript text with slots. Applying one writes plain BeatScript out before the program
is lowered, exactly as `seed(...)` does (`seed.rs`, "Start from a seed"), so the compiler, the browser and the phone never
see a template.

## Grammar

```ebnf
arrange  = ... | "arrange" "from" ( seed | template ) ;
template = "template" "(" NAME "," ( PHRASE | STRING ) ")" ;   (* a template's name and a phrase: a phrasebook id or words in quotes *)
```

The reference is read only where it stands after `from` (`arrange from template(...)`), so `template` is **not a reserved
word**: a `let`, a `def` and its call, or a `section` may be named `template` and check as they did, with the CIDs they had
(`tests/template.rs`, `template_is_still_a_name`, pins three of them against the build before templates). `section NAME from
template(...)` is an error that says a template makes the whole track. One `arrange from` a program: a template or a seed.

The canonical text keeps the reference as written, so the CID is of the reference: `template(deep-house, @tempest_such_stuff)`
or `template(deep-house, "robots on the road")` (spacing is not part of it; a quote in the words is escaped). It is the same
CID wherever the reference is checked, because the written-out text is never part of it.

## Semantics

**The phrase drives the vocal line and nothing else.** The sections, the drums, the bass, the pad and the arrangement are
the template's own. The program's own `tempo`, `key` and `swing` lines win over the template's (as with a seed); the vocal's
notes are degrees of the key, so a program that says `key dm` gets a track in D minor with the same shape.

### The phrase's profile

| | a phrasebook id | any words |
|---|---|---|
| the lyric | the book's `lyric`: words of the lexicon, phoneme escapes for the rest | the words, whitespace made one space |
| the syllables | the notes `sing` counts in the lyric (the book's `syllables`) | the same count |
| the stress | the book's `stress`, `x` and `~` a syllable | worked out (below) |
| the feel | the book's `feel` | straight, or triplet when the stresses fall three syllables apart |

A stock phrase (`@hands_up`) has no curated stress: it is worked out from the words it says when those are as many
syllables as its lyric (else a plain trochee).

**The syllable count is `sing`'s.** `template::expand` takes the check's `Env`, and the host's voices count the words as the
`sing` line it writes will count them (`Voices::sung_syllables`, else vowel groups), so every syllable has a note whatever
the host. The CID is of the reference, so a host that counts a word differently changes the written-out text, never the
program's CID.

**The stress of any words**: a function word is unstressed (`a an and are as at be but by can for from have he her him his I
if in is it its me my no not of on or our she so than that the their them then there they this to up us was we were what
when where which who will with would you your` and a few contractions); the first syllable of any other word is stressed,
one-syllable ones included; a word written in phonemes is stressed where it has a `!`, else on its first syllable; a rest
`_` is unstressed. With nothing stressed the first syllable is.

**The meter**, named from the stresses when they show one (`classify`): `iambic` and `trochaic` (a stress on every second
syllable), `anapestic`, `dactylic` and `amphibrachic` (on every third), else `free`. The feel of any words is triplet for a
meter of three, straight otherwise.

### The vocal line

Each syllable is an eighth note (two steps). The line starts at step 0, or at step 2 or 4 when it has a **pickup** (leading
unstressed syllables), so that the first stressed syllable is on a beat. Then, syllable by syllable:

1. a stressed syllable sits on a **beat**: a step divisible by 4 in the straight feel, by 6 in the triplet feel. If the next
   eighth is not on a beat, a rest (one eighth, or two in the triplet feel) is left first;
2. an unstressed syllable sits on the next eighth.

So `~ x ~ x` (two iambs) is a pickup and then a stress on each beat; `x ~ ~ x ~ ~` (two dactyls) in the triplet feel puts
the stresses on steps 0 and 6 with the weak syllables on the eighths between; `x x x` is three quarter notes.

The line takes **1, 2 or 4 cycles** (a triplet line also 8), the fewest that hold it with at least a beat to breathe. A
cycle is a bar (16 steps), or twelve steps in the triplet feel (`melody/12`, six eighths: three syllables to the beat, a
6/8 bar against the 4/4 drums, and the shaker in the same cycle, `hihat/12`). A cycle with notes is a `let`
(`vox_a`, `vox_b` ...), eight terms for a bar and six for twelve steps; the breath at the end is a `~` in the part's
alternation, not a `let` (a bar of rests is a drum pattern to the arrangement's pitch operators, and nothing for them to
move). **A section is 8 bars (12 in the triplet feel)**: a whole number of the vocal cycles and of the four-bar harmony, so
the line is sung again and again in it.

| | straight | triplet |
|---|---|---|
| a cycle | 16 steps (a bar) | 12 steps (`/12`) |
| a beat | 4 steps | 6 steps |
| the line | 1, 2 or 4 cycles: up to 6, 14 or about 30 syllables | 1, 2, 4 or 8 cycles: up to 3, 9, 21 or about 45 |
| a section | 8 bars | 12 bars |

A longer phrase is an error at the phrase: the line takes N steps and is at most M.

**The notes** are degrees of the key. Along a contour (an arch, a fall, a rise or a wave, picked by a hash of the words,
so two phrases differ and one phrase is always the same) a stressed syllable takes the nearest chord tone (1 3 5 8), an
unstressed one the nearest passing degree (2 4 6 7, and not the one just sung), and the last syllable rests on 1, 5 or 8.
Stressed syllables are accented (`!`): the kalimba on the vocal line plays them louder.

### The form templates

Six templates are classical forms (`docs/beatscript/forms.md`): each [`Form`] in `TEMPLATES` says its section shape, its mutation
plan, the mutation operators the plan uses and the themes it is built from, and a test holds the template's text to it
(`a_form_is_what_its_table_says`). Their material comes from the public-domain theme library (`djengine::themes`, the CC0
`data/themes-pd.txt`): every theme is a slot, `$THEME_<ID>` (`$THEME_ODE_TO_JOY`), filled with its `let` lines (a let a bar,
then their alternation), so a form names its themes and the library is their one source. The phrase is sung in the form's
`sung` (or `voices`) section, `$BARS` bars long, with `$VOX` on `melody$CYCLE` and the `sing` line as in deep-house; the
shaker is `shk`, outside the `drm_*` glob, so the arrangement's `grow` and `decay` never take a hit the crossed voice needs.
Each form takes every phrasebook line with no warning (`every_form_takes_every_public_domain_line`). A form keeps its own
key: its themes are written in it, so a program's `key` moves the vocal line and not the themes.

### The deep-house template

Deep house at 122 in A minor (`swing 0.12`). The drums and the bass are on deck A, the pad, a kalimba on the vocal line
and the `sing` line (robot-choir) on deck B.

- `kick4` (four on the floor) and `clap24` are outside the `drm_*` glob, so the arrangement leaves them alone; `drm_hat`
  (the offbeat hat), `drm_shk` (the shaker) and `drm_rim` are what it changes.
- The bass plays the roots of Am F Dm G (`bass_a bass_f bass_d bass_g`, a bar each) under a pad on the same chords.
- **`night`**: the voice is buried: `mix { B level 0.45, B eq high -9 }` (put back where the section ends), a duck of
  deck B under the kick, and the pad and the kalimba above the voice. **`spot`**: the voice is up front (`B level 0.95`), the
  pad and the kalimba drop under it, and the shaker and the rim come in. (Measured with `beatscript listen` on the Tempest line:
  the sung part is 5.5 dB below the pad in `night` and 1.6 dB above it, 0.7 above the kalimba, in `spot`.)
- The arrangement, four passes (`loop 4 then end`): `night`, `@ vox_* oct 1` and `spot` (the voice an octave up for that
  section only), `~ drm_* fill 0.15`, `night`, `@ vox_* xfer drm_shk` and `spot` (the voice's notes on the shaker's rhythm,
  for that section only), `~ drm_* grow`. The `~` mutations only add hits, so the shaker never has fewer notes than the
  vocal has syllables, and no generation leaves a syllable unsung (`any_phrase_checks_clean` holds it).

`check` reports the generations of the arrangement as for any `arrange { }`.

## Nested parts: each role a CID of its own

`template::expand` writes a reference out as one text, but inside it every role the reference binds (docs/beatscript/ext-roles.md) was
written in by a binding: a gen call's part, a stored program's sections, a theme, notes, or another form whose own roles are
bindings in turn. Each of those is a **part** of the expansion (`crates/djlive/src/nested.rs`, roadmap 6680 item 2), and each
part has a CID of its own:

```text
arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876))

path              kind      call                                                     cid         resolves    generator
episodes          template  template(passacaglia, ground=gen@1 dubstep seed=54876)  z4EBG9...   -           -
episodes/ground   gen       gen@1 dubstep seed=54876                                 z4EBG9...   z4EBG9...   seeded@1
```

- **path** is where it stands: the role, after the roles of the forms it is inside (`episodes/ground` is the `ground` of the form
  bound to `episodes`). A program has one template reference, so a path names one part.
- **text** (with check's `expand`) is a program of its own: the `tempo` and `key` at the top of the program it was written into,
  then the lines its binding wrote, every reference in them written out (a gen call fitted and written out as its lets, a stored
  program's or a seed's sections written in), printed canonical. **cid** is that text's CID, so it names exactly what plays there.
  The lines are the expansion's own: what a part's text says after its key and tempo, the whole expansion says too.
- **resolves** is the pin of its source: the program a gen call prints (fitted to the key it plays in), a stored program's CID.
  A theme, notes and a form have none (a form's sources are its own parts).
- The parts are in the order the form names its roles, each with its own (`parts`), at any depth. A role bound to a stored program
  (`episodes=<cid>.ground`) is one part, of kind `stored`, whatever that program is: a stored template recipe's own roles are its
  own, not parts of the program that borrows from it.

Where they are: `check`'s `resolution` (and `listen`'s, `expand`'s and `genre_template`'s) gives the template's entry `parts`, as
`{path, role, kind, call, cid, resolves, generator, parts}`, with `text` when `expand` is asked; the gen call a role wrote is listed
`nested` with its own pin, resolved in the key and tempo of the written-out program. The program's JSON-LD block (docs/SCHEMA.md)
has them as `hasPart`: each a CreativeWork whose `@id` is the part's CID, named by its path, `isBasedOn` its source (`resolves`,
`pinnedGenerator`, `derivation` `called` or `borrowed`), with its own `hasPart`.

## The stored expansion

A program with references (a template, a gen call, a seed, a borrowed section) stored on beatscript.app is stored with its
**expansion** beside it (`crates/beatscript-serve/src/store.rs`, `put_expansion`, roadmap 6680 item 3): the program with every reference
written out as plain BeatScript, canonical, as a program of its own under its CID (check's `expanded_cid`), through the storage
gate like any other. `/p/<expanded_cid>` loads it, and it plays as the recipe does. Every write path does it, through one function
(`compose::expansion_beside`): the MCP `share` (signed in; also `render`, `artifact` with `store` and compose, through
`compose::put`), `PUT /api/programs/<cid>` and `PUT /api/shares/<cid>`.

- **It is written out first, under the checker's rules.** The expansion is written out, checked by the gate and its copies found
  (`store::plan_expansion`) under the checker's per-address budget, slots and timeout, before the recipe is written; the recipe's
  meta names it (`expands_to`) only once it is stored. One already stored is answered at once, with no work.
- **First write wins**, as for any program: an expansion someone stored first (the same text shared on its own) is the one, with
  the licence and credits they gave it, and the answer says that licence (`expansion_license`).
- **It is stored only when its rights, its size and the store's room allow.** Every program it copies (the stored programs its
  references borrow from, and what those borrow in turn: the recipe's `uses`) must be CC0, or the recipe author's own under the
  recipe's licence; it must be at most a program's size (`-max-program-bytes`) and fit the store's cap. Else nothing is written,
  and the answer's `expansion_note` says why ("it copies z4EBG9…xy (CC-BY-NC-ND-4.0), and an expansion is stored only when ...").
- The answers: `expanded_cid` (a program with references), with `expanded_url` (the share routes) and `expansion_license` when it
  is stored, `expansion_note` when it is not.

| | the recipe's meta | the expansion's meta |
|---|---|---|
| names the other | `expands_to: <expanded_cid>`, once the expansion is stored | `expanded_from: <recipe CID>` |
| licence, credits, name, author | its own | the recipe's |
| `source` | as stored (`put`, `share`, ...) | `expansion` |
| what it is based on | its text (its calls and borrows) | `based_on`: each reference the recipe wrote out into it, `"<call> -> <cid>"`, once (a theme, notes: the call alone; a template whose written-out program is the expansion itself: the call alone) |

Lineage (`/p/<cid>/tree`, the MCP `lineage` tool) reads both: the recipe's node `expandsTo` its expansion (the CID `check` gives, so
a recipe stored before this has one too), the expansion's is `expandedFrom` each recipe that writes out to it (at most 50, then
`expandedFromTruncated` and the tree's `truncated`) and `isBasedOn` each source (`derivation: expanded`, with `resolves`, never
itself), and a tree shows the other among its programs. The two program pages' blocks say
the same (`expandsTo`; `expandedFrom` and `isBasedOn`).

## What check says

| it finds | it says |
|---|---|
| the phrase | a note: `template deep-house: 9 syllables, anapestic, triplet feel, a vocal line of 3 bars in sections of 12 bars`, and `(the stress is worked out from the words)` for any words |
| an unknown template | an error at the name: `` `deep-hose` is not a template ``, `did you mean deep-house? The templates: deep-house, quodlibet, ...` |
| an empty phrase (`""`, `"..."`) | an error: there is nothing to sing, with a phrase to write |
| an unknown `@id` | an error with the nearest ids, or to give the words in quotes |
| a phrase too long for a line | an error: the line takes N steps and is at most M |
| a word the lexicon does not know | the usual warning: sounded out by the rules, and how to respell it |
| `template(...)` anywhere but after `arrange from` | an error: it makes the whole track |
| two `arrange from` | an error: a template or a seed makes the whole arrangement, not both |
| `#! templates: v2` | an error: this BeatScript has templates v1 |

A malformed reference is one error at the reference, with the shape to write.

## Determinism and versions

Same text, same CID: the stress, the layout and the notes are integer arithmetic over the phrase (the contour's hash is the
language's own `fnv`), and the arrangement's dice are the arrange mutations' own (a hash of the program's CID). No clock, no
global random generator. The written-out text of a template and a phrase is pinned in `tests/fixtures/template/` and a
change to it fails `templates_v1_are_pinned`; a change to a template that changed any track would be templates v2.

## Decisions

- **The slot syntax is `template(name, phrase)` after `arrange from`**, by analogy with `seed(...)`: music from elsewhere,
  written out as plain BeatScript before lowering, so `beatscript expand` and the MCP `expand` tool already show it and nothing
  downstream learns a new construct. It is one place in the grammar (one production and an alternative in `arrange`), and
  one place in the code (`template.rs`: `runs`, `read_ref`).
- **The slots in the template are `$NAME`**, the language's parameter token class, upper case; they are filled by plain text
  substitution (`fill`), so the template text is the template's whole meaning.
- **Triplet is a cycle of twelve sixteenths.** The language has no non-sixteenth grid (a term that does not divide the
  cycle snaps), so three syllables to the beat is a 6/8 bar (`/12`: six eighths) against the 4/4 drums, which comes round
  every three bars. The shaker takes the cycle too, so that the voice can be crossed onto it (`xfer` takes the other pattern's
  terms, and eight of them would not divide twelve).
- **The breath is a `~` in the part, not a `let`.** An all-rest `let` is a drum pattern to the mutation operators: `oct` on it
  warns, and `xfer` onto it plays the shaker's hits as the key's root.
- **The voice is `robot-choir`**: the formant voices differ by 25 dB in level (`choir-aah` is 8 dB under it, `soprano` 26), and a sung
  line is quiet next to a synth pad, so the loudest of the mid-range voices keeps the words from being texture. One word
  in the template to change.
- **A straight line is at most four bars**, because the voice is crossed onto the shaker, which has five hits a bar: a longer
  line would have syllables the crossed section has no note for (the `sing` warning "the last N are not sung").
- **A part is a binding, and its CID is of what it plays.** The tree is the roles a reference binds (a form in a role has its
  own), because those are the places another program can be put; a part's text is its binding's lines written out all the way,
  so its CID is the same notes wherever they come from, as the expanded CID is for a whole program. The pin of its source is
  beside it (`resolves`), as `resolved` is beside a reference. (The other way, a part's text with its gen calls left in, would
  be a second recipe CID per part: one CID a part keeps the answer small.)
- **A part's text opens with the key and tempo it was written into.** A gen call is fitted to them and a theme is moved into
  them, so the same binding in a program at another tempo or key is other notes, and its CID says so; a part's text is then a
  program that plays as that part does. The written-out template's own CID is the template reference's `resolved`, as before.
- **The parts come from the binding itself, not from reading the text back.** roles.rs records what each binding wrote (a
  thread-local recorder, as gen.rs keeps the programs a check generated), and nested.rs writes each one out; the written-out
  program is the same text as before, and no part is found by parsing comments.
- **`hasPart` on the recipe's block, by bare CID**, as the block's own `@id` is: the parts are what the recipe makes, so they are
  said where the recipe is read; schema.org's `hasPart` is the term (a set), and each part `isBasedOn` its source with the
  provenance terms the block already has. A new term in the vocabulary, not a new version of the block's shape.
- **The expansion is stored with the recipe's licence**: CC0 when the recipe is (the default, and the roadmap's case), CC BY with
  the same credits when it is CC BY, because the expansion holds the recipe's own text. It is the same track, so it keeps the
  recipe's name; `source: expansion` and `expanded_from` say what it is.
- **An expansion that copies another's work under other rights is not stored.** A recipe only names what it borrows; its
  expansion is a copy, written by the site. So it is stored only when every program it copies is CC0 or the recipe author's own
  under the recipe's licence, and otherwise the answer says why and the recipe stays a recipe. (Storing it under the most
  restrictive licence with the union of credits was the other way; refusing is the one that never publishes a licence someone
  did not choose.) The store keeps no author for a shared program today, so in practice an expansion copies CC0 programs only.
- **A program is never based on itself.** A template with nothing to write out after it has a written-out program that is the
  expansion itself; its `based_on` line says the call alone, and the block and lineage drop any source that resolves to the
  document's own CID.
- **The expansion is the costly half, so it runs as a check does**: under the checker's budget and timeout, charged to the
  caller's address, at most a program's size and within the store's cap; a panic in it leaves the recipe stored and says the
  expansion is not; one already stored costs nothing.
- **Each side names the other in its meta, and lineage also computes it.** The recipe's `expands_to` (written only once the
  expansion is stored, which is why the expansion is stored first) and the expansion's `expanded_from` let the index know the pair
  without compiling; first write wins, so a recipe stored before this has no `expands_to`, and its `expandsTo` comes from its
  check (the expanded CID is of its text). Several recipes can write out to one program (`expandedFrom` is a list, cut at 50 as
  siblings are).
- **The recorder pauses inside a binding's own compiles.** A stored source, a generated program and a gen call's probe are
  compiled whole inside a binding; the recorder is paused there, so their bindings are never parts of this one, and it is put
  back by a drop guard, so a panic leaves no thread recording.
- **An expansion's sources are not counted twice.** Its `based_on` lines are its recipe's references, which the recipe already
  counts for curation (roadmap 6670): an expansion stored by the site is not a second program that chose to vendor them.
- **Each document is written out in the host's environment.** The block's `expandsTo` and `hasPart` are computed with the
  checker's environment (its voices count a template's syllables as `check` does), so they are the CIDs `check` gives and the
  store keeps; with no environment (a plain host) only what resolves without one is said.

## Not done

- Seven templates: `deep-house` and the six forms (quodlibet, passacaglia, canon, rondo, fugue, theme-and-variations,
  `docs/beatscript/forms.md`). Another is a text in `templates/` and a line in `TEMPLATES` (with its `Form`, for a form).
- The phrase controls the vocal only: not the key, the tempo or the drums. (The program's own `tempo`, `key` and `swing` do.)
- No `call` and `response` from a phrasebook line's `call` column: a line is sung straight through.
- A phrase longer than one line (about 30 syllables) is an error; there is no multi-line verse.
- The stress of any words is a plain rule, not a dictionary's: a word of the lexicon is not stressed on its dictionary
  syllable. A phrasebook line has its stress curated.
- A part is not stored on its own: a role can borrow a section of the stored expansion (`<expanded_cid>.<section>`, which holds
  every part's sections), but not a part by its CID until parts are stored too. A borrowed stored program's own references are
  not parts of the borrower (its sections arrive compiled).
- The expansion is stored when a program is stored; programs stored before this get theirs when they are stored again (a
  share of the same text), not by a sweep of the store.
- An expansion that copies a CC BY program (even the sharer's own) is not stored, because the store keeps no author to match;
  the answer says so. A share that records its author would let the recipe author's own work through.
- The `djrust:` document (`/p/<cid>.jsonld`, context v1, immutable) does not say `expandsTo` or `hasPart`; the block does.
