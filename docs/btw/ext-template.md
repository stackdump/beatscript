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

Six templates are classical forms (`docs/btw/forms.md`): each [`Form`] in `TEMPLATES` says its section shape, its mutation
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
  pad and the kalimba drop under it, and the shaker and the rim come in. (Measured with `btwd listen` on the Tempest line:
  the sung part is 5.5 dB below the pad in `night` and 1.6 dB above it, 0.7 above the kalimba, in `spot`.)
- The arrangement, four passes (`loop 4 then end`): `night`, `@ vox_* oct 1` and `spot` (the voice an octave up for that
  section only), `~ drm_* fill 0.15`, `night`, `@ vox_* xfer drm_shk` and `spot` (the voice's notes on the shaker's rhythm,
  for that section only), `~ drm_* grow`. The `~` mutations only add hits, so the shaker never has fewer notes than the
  vocal has syllables, and no generation leaves a syllable unsung (`any_phrase_checks_clean` holds it).

`check` reports the generations of the arrangement as for any `arrange { }`.

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
  written out as plain BeatScript before lowering, so `btwd expand` and the MCP `expand` tool already show it and nothing
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

## Not done

- Seven templates: `deep-house` and the six forms (quodlibet, passacaglia, canon, rondo, fugue, theme-and-variations,
  `docs/btw/forms.md`). Another is a text in `templates/` and a line in `TEMPLATES` (with its `Form`, for a form).
- The phrase controls the vocal only: not the key, the tempo or the drums. (The program's own `tempo`, `key` and `swing` do.)
- No `call` and `response` from a phrasebook line's `call` column: a line is sung straight through.
- A phrase longer than one line (about 30 syllables) is an error; there is no multi-line verse.
- The stress of any words is a plain rule, not a dictionary's: a word of the lexicon is not stressed on its dictionary
  syllable. A phrasebook line has its stress curated.
