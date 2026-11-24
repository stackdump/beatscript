# The acid voice: a 303 made in the engine

`inst=acid` used to be a family word: any name with `acid` in it played the engine's plain saw bass, and `check` said so.
It is now what the name promised, a 303-style voice of its own. Its own name is `inst=acid-bass` (`acid` is the same
voice: one instrument table entry each, alike), and `inst=acid-square` is the same voice on its square wave. Its knobs
are part params: `cutoff=`, `res=`, `env=`, `accent=`, `drive=` ("Its knobs", below). It is all synthesis, in the public engine (stackdump/djrust `src/wave/acid.rs`, MIT, no samples), so the offline
render (`beatscript listen`, `beatscript render`) and the browser (djweb's wasm, the same djrust) play it alike. Example
`examples/beatscript/acid-303.beatscript`. Tests: djrust `tests/wave_acid.rs` and the unit tests in `src/wave/acid.rs`; here
`crates/djlive/tests/expression.rs` (what a slide on the acid voice lowers to).

## Grammar

Nothing new. The voice is a name `inst=` takes, and the marks and params it plays are the ones every tonal part has
("Expression marks" in docs/BEATSCRIPT.md):

```text
deck B: bass a1 a1 a2^ a1 ~ c2! a1 g1^ a1 a1 ~ e2^ d2 a1 c2! a2! decay=180 inst=acid
deck B: melody ~ ~ ~ ~ a3! ~ ~ e3^ g3 ~ ~ ~ c4!^ b3 ~ ~ decay=600 glide=110 inst=acid-square
deck B: bass acid inst=acid-bass cutoff=220 res=90 env=70 accent=100 drive=25
```

| written | on the acid voice |
|---|---|
| `c2!` | an accent: louder, the filter opened further and its sweep shorter; accents in a row build (the accent sweep) |
| `e2^` | a slide: the voice glides into the next note without striking it again, in 60 ms |
| `glide=110` | the slide's time, ms (1 to 2000), as on every voice |
| `decay=180` | the 303's decay knob: how long the filter's sweep takes to fall, ms (10 to 10000) |
| `pitch-drop=7` | each note starts 7 semitones high and falls onto its pitch in 50 ms, as on every voice |
| `len=` | how long the note's gate is open: the voice is let go 30 ms before the note ends (at most half the note) |
| `vel=` | the level, and the accent: a velocity above 110 is accented in proportion, all of it at 127 (a humanized 100 is not) |
| `cutoff=220` | the cutoff knob, Hz (20 to 12000, default 300): the filter's floor, before a strike's envelope opens it |
| `res=90` | the resonance knob, 0 to 100 (default 72): how hard the filter rings at its cutoff (100 rings hardest, and stays bounded) |
| `env=70` | the env mod knob, 0 to 100 (default 45): how far each strike opens the filter, 100 is 6 octaves (45 is 2.7) |
| `accent=100` | the accent knob, 0 to 100 (default 80): how much an accented note is louder, opens further and sweeps shorter |
| `drive=25` | the drive knob, 0 to 100 (default 0): how hard the oscillator drives the filter's saturation; the level is given back after it, so it is grit, not loudness |

## Its knobs

A 303 has six knobs; five are part params here and the sixth, decay, is `decay=`. Each is one whole number for the part
(not a range), carried on every note's binding under its own key (`cutoff` Hz; `res`, `env`, `accent`, `drive` 0 to 100),
written only when set, so a part without them compiles to the net it always did and no pinned CID moves. The engine
reads them per note (djrust `pflow::AcidKnobs`, applied in `wave::acid` when a note is struck or slid into), over the
voice's own settings (`AcidParams`, the instrument table's): a knob the note does not set is the voice's, computed
exactly as before, so a note without knobs sounds as it always did, to the sample. The knobs are not reserved words
(`compile::ACID_PARAMS`, beside `parse::PARAMS`): `section drive` and `let accent = ...` are still names. On a drum part
they are errors; on a voice that is not the acid voice they are a warning, and the part plays as it would without them.

## What it lowers to

The net is the same as for any tonal part, with one difference: on the acid voice a slide is a 303's. Each note's binding
carries what every tonal binding does (`velocity`, `durationSteps`, and `decay` / `glide` / `pitchDrop` when set), and a
note that a `^` slides into carries `"glide": 60` when the part has no `glide=` (`djlive::compile::SELF_SLIDE_MS`), and
the note the `^` is on carries `"slide": true`, so the voice holds its gate until the glide comes, as a 303 keeps its gate
high on a slide step. (The ring cuts a note's length at its end, so a `^` on the last step would otherwise let go 30 ms
before the bar line and swell back into the downbeat.) On every other voice neither key is written, and the `^` holds
its note as it always has. Which voices slide by
themselves is the engine's to say: the entries of its instrument table that play its `"acid"` engine
(`djlive::compile::slides_by_itself`), today `acid-bass`, `acid` and `acid-square`.

The engine plays a net's notes on the acid lane so:

- **An accent is the velocity.** The nets carry an accent as velocity +27, so the voice reads it there: none at 110 and
  below (over the ten a humanize moves a 100), in proportion above, all of it at 127. At the default accent knob (0.8) a
  fully accented note is about 4 dB louder than the same note at 100, its filter opens about an octave further at the
  strike, its sweep is shorter (132 ms at the default decay of 300, 108 ms at `decay=180`: the 303 takes its shortest
  decay on an accent), and a smoothed copy of the accented sweep (one for the lane, as a 303 has one circuit) opens the
  filter up to 1.3 octaves more, so accents in a row climb.
- **A slide is a note's `glide`.** The voice still sounding the note before goes on: its envelopes are not struck again,
  its gate is held to the new note's length, and its pitch glides there as a 303's lag does, fast then settling (95% of
  the interval in the glide's time). A note that says `slide` holds its gate for half its length again, past any swing of
  the note the glide comes on.
- **One note at a time.** A note that does not slide lets the one before go (8 ms), as a 303 plays one note at a time.
  Notes struck within 12 ms of each other are a chord: each sounds on a voice of its own (a lane has four, a harmony
  part six). A flam's grace, 22 ms ahead of its note, is let go when the note comes.
- **The sound**: a saw (or a square), band-limited (PolyBLEP), into a four-pole lowpass (trapezoidal one-pole stages
  with the feedback solved each sample and the stage input saturated), a symmetric soft clip (a 303's output stage
  clips softly), then a 25 Hz coupling highpass and the amplifier. The saw's reset rings a resonant ladder into
  one-sided spikes; clipped there, an acid line is no harder on a deck's safety limiter than the wavetable bass on the
  same line (a test: plain sixteenths and a busy accented line through `PeakLimiter` under `WaveOptions::extended`,
  limited on no more samples, no deeper, and with under 1% DC). The knobs' defaults are a classic squelch: cutoff 300 Hz, resonance 0.72, envelope 2.7 octaves, decay
  300 ms, accent 0.8. No key tracking: a 303 has none. Its level sits within 3 dB of the lead voice (djrust's
  instrument test), as every voice of the table does. The engine's own shaping of a synth voice (`.subtractive`'s
  filter, `.unison`'s detuned pair) leaves it alone; a genre that puts a sine an octave under its bass (dnb, dubstep,
  trap) puts one under this voice too.
- **Deterministic, rate-independent, bounded.** Arithmetic and `libm` only; time constants in seconds and the filter
  prewarped, so 24, 44.1, 48 and 96 kHz sound alike (within 1 dB, a test); every state flushed to zero under 1e-18; the
  saturation keeps the ladder bounded at full resonance, and a busy line of accents and slides at the voice's own level
  peaks well under full scale (a test).

beats' own engine (beats.bitwrap.io's `wave-engine/synth.js`, the parity oracle djrust's `tonal_voice` copies "as the JS
regexes do") keeps `acid` its plain saw bass: the 303 is in djrust's instrument table, which beats' parity path does not
read. beatscript.app plays only djrust (the engine in the page and the render behind the stream), so no JavaScript copy
of the voice is needed or kept.

## What it refuses

Nothing new. `glide=`, `decay=` and `pitch-drop=` on a drum part are errors, as before (the help names the drum marks).
A name that contains `acid` but is not a voice (`inst=acid-lead`) still plays the generic saw bass, and `check` warns that
it does and names `acid-bass` among the voices with a sound of their own. `cutoff=` and the other knobs on a drum part
are errors; out of range, or a range, are errors; on a voice that is not the acid voice, a warning.

## Decisions

- **`inst=acid` is the real voice** (Matt's call, given in the item): the name promised a 303. `acid-square` is its
  square switch, a second entry in the instrument table, because a name is how a part picks a sound today.
- **The accent rides in the velocity, not a new key on the binding.** The nets already carry `!` as velocity +27, and the
  acid genre's own lines carry it as +25; a new `accent` key would move the CID of every net with an accented tonal note.
  The accent starts above 110, so a humanized 100 (`humanize( )`, the runner's groove) stays unaccented; the cost is
  that a `vel=` above 110 is a little accented without a `!`, as on a MIDI 303, and an accent on a part at `vel=80`
  (107) is only louder.
- **A slide is lowered to a glide on the acid voice, not inferred from held notes.** A held note is the TB-303 MIDI
  convention for a slide, but a tonal part's notes are two steps long unless it says otherwise, so every line of
  sixteenths would slide. The language knows the slides (`^`) and writes them; the engine only obeys `glide`, as it does
  on every voice. No program in the repo has `^` on an acid part without `glide=`, so no pinned CID moves.
- **60 ms** is the default slide: about a 303's. `glide=` changes it on this voice as on every other.
- **`decay=` is the 303's decay knob here, not the note's tail.** On a 303 the decay knob is the filter's; the level is
  gated by the note. On every other voice `decay=` is still the tail.
- **One note at a time, but chords sound.** A 303 is monophonic; a part may still write `[a1,e2]`, and both notes play.
- **The note a slide leaves says `slide`.** The ring cuts a note's length at its end, so the engine cannot tell from the
  last step's length that a glide follows; the flag says it, is written only on the acid voice's parts (no pinned net
  moves: the only program with it is `examples/beatscript/acid-303.beatscript`, pinned with it), and every other voice ignores it.
- **The loudness probe stays at 8 kHz for the acid voice.** There the filter is held under the 4 kHz Nyquist and the
  voice measures about 1 dB quiet, so a track with it comes out a little loud. A sung voice is probed at 24 kHz (it
  measured 13 to 19 dB wrong at 8 kHz), but the probe's target is calibrated at 8 kHz: probed at 24 kHz, whole edm
  and trance tracks landed 0.7 dB over it, and the genres' spread went past its 2 LU (djhost's deck_acceptance). The
  1 dB is the smaller error.
- **A soft output stage, not a lower level.** Trimming the level would not have helped: the loudness gain brings the
  line back up, and the spikes with it. Clipping them at a symmetric knee before the highpass takes the limiter off the
  line and adds the grit a driven 303 has.
- **The generator's versions keep the voices they had.** `gen@1` and `gen@2` pick a part's voice from the genre's
  instruments that are voices, and `acid` was only a family word when they were pinned, so neither picks it now
  (`beatscript-api::generate::VOICES_AFTER_V2`): every gen call resolves to the program it always did (`tests/gen.rs`). A later
  version may pick it; that is a new version.
- **The genre templates keep the voices they had, too.** The bitwrap ontology names `acid` as the bass of techno, edm and
  speedcore. `genre_template` writes a genre's voice only when it is a voice, so it now would write `inst=acid` there,
  and `genre@1` (djengine `data/genre-templates.txt`, what `use genre@1 techno.drop as drop` borrows) froze those bass
  lines without it. `genre_template` leaves the same two voices out, so it still writes what `genre@1` froze
  (beatscript-serve's `genre_templates_v1_are_frozen`). Naming the 303 there is a `genre@2` (Proposals).
- **`acid-bass` is the voice's name, and `acid` stays.** The item that asked for the voice named it `acid-bass` (a name
  that starts with a letter: `303` cannot be written). `acid` is beats' word for the genre's bass and what programs already
  say, so both are entries of the instrument table, alike: `inst=acid` plays the same 303 with no warning, and no program
  that says it moves.
- **The knobs are part params, 0 to 100.** A 303's knobs are dials; a whole-number turn of 0 to 100 needs no decimals in
  the text or floats on the binding (the binding's numbers stay whole, as every other key's are), and the cutoff, the one
  knob with a unit, is in Hz. They are per part, carried per note as `decay=` is, so a slide into a note takes the new
  note's knobs. They are not reserved words: `drive` names sections (the pinned `matrix-130` has `section drive`).
- **Drive is new to the voice, and off by default.** The oscillator's level into the ladder's saturation was fixed; the
  knob raises it by up to 4x and divides the level back out after the ladder, so the grit changes and the loudness does
  not (much). At 0 the arithmetic is what it was, so `acid` and `acid-bass` sound exactly as `acid` did.
- **The oscillator is chosen by name.** `acid-bass` is the saw and `acid-square` the square, as before: a part picks a
  sound by its `inst=`, and the square's level is calibrated apart from the saw's. A `wave=` param would be a word-valued
  knob the binding does not carry today.
- **`gen@4` puts the acid genre's bass on the 303.** `seeded@3` could not pick the acid voice (it was not a voice when
  `gen@1` and `gen@2` were pinned, `VOICES_AFTER_V2`) and wrote `sub-bass` for the acid genre's bass, so a track in that genre
  had to name a stand-in by hand. `seeded@4` writes `inst=acid-bass` there and nothing else differs: the pick draws from
  the seed's stream as 3's did, so every later choice is 3's, and every other genre prints 3's program but its
  `#! generator:` line (`tests/gen.rs`). The bitwrap ontology's other genres whose bass is acid (techno, edm, speedcore)
  keep theirs: that is the `genre@2` question below.

## Not done

- The acid genre's own line (djrust `gen::theme::acid_bass_ring`) holds its slides into the next step and writes no
  `glide`, so a composed acid-house track in the app strikes those notes again instead of sliding. Writing `glide` there
  moves that genre's composed nets (its goldens), so it is left for its own change.
- `sound=synth:acid` (a role's sound option) plays the voice, but a `^` on that part does not slide by itself: the
  language only knows the part's `inst=`.
- A composed track whose bass is `acid` (the default bass of techno, edm, trance, speedcore and acid; an alternative in
  seven more genres) plays the 303 in the app (where no sampled bass is bound), with no slides. The composer's genre
  tables are beats' (djrust `data/genres-native.json`) and still name it `acid`: the same voice as `acid-bass`, so the
  composed acid genre already plays it; renaming it there would move that genre's composed nets for no change in sound.
- The knobs on `sound=synth:acid` (a role's sound option) are not read: the language only knows a part's `inst=`.

## Proposals

For Matt's yes or no; none is built. (`cutoff=`, `res=`, `env=` and `accent=`, once proposals here, are built: "Its knobs".)

- **The ontology's part params**: the ontology run reads `parse::PARAMS` for its part-parameter terms; it should chain
  `compile::ACID_PARAMS` so the five knobs are terms too.
- **`genre@2`, the genre templates with the 303**: techno, edm and speedcore written with `inst=acid` on their bass, as
  the ontology names it, in a second file beside `genre@1` (djengine `genre_templates::VERSIONS`). It is the same
  question as the composed acid basses under Not done: whether a genre whose bass is acid plays the 303 now.
- **A cutoff that moves with the song** (`deck B: acid cutoff 300 -> 2k over 8 bars`): the gesture acid is made of,
  which per-part params cannot give. It would be a control noun on the deck's voice, as `resonance` is on its insert.
