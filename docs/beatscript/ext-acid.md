# The acid voice: a 303 made in the engine

`inst=acid` used to be a family word: any name with `acid` in it played the engine's plain saw bass, and `check` said so.
It is now what the name promised, a 303-style voice of its own, and `inst=acid-square` is the same voice on its square
wave. It is all synthesis, in the public engine (stackdump/djrust `src/wave/acid.rs`, MIT, no samples), so the offline
render (`beatscript listen`, `beatscript render`) and the browser (djweb's wasm, the same djrust) play it alike. Example
`examples/beatscript/acid-303.beatscript`. Tests: djrust `tests/wave_acid.rs` and the unit tests in `src/wave/acid.rs`; here
`crates/djlive/tests/expression.rs` (what a slide on the acid voice lowers to).

## Grammar

Nothing new. The voice is a name `inst=` takes, and the marks and params it plays are the ones every tonal part has
("Expression marks" in docs/BEATSCRIPT.md):

```text
deck B: bass a1 a1 a2^ a1 ~ c2! a1 g1^ a1 a1 ~ e2^ d2 a1 c2! a2! decay=180 inst=acid
deck B: melody ~ ~ ~ ~ a3! ~ ~ e3^ g3 ~ ~ ~ c4!^ b3 ~ ~ decay=600 glide=110 inst=acid-square
```

| written | on the acid voice |
|---|---|
| `c2!` | an accent: louder, the filter opened further and its sweep shorter; accents in a row build (the accent sweep) |
| `e2^` | a slide: the voice glides into the next note without striking it again, in 60 ms |
| `glide=110` | the slide's time, ms (1 to 2000), as on every voice |
| `decay=180` | the 303's decay knob: how long the filter's sweep takes to fall, ms (10 to 10000) |
| `pitch-drop=7` | each note starts 7 semitones high and falls onto its pitch in 50 ms, as on every voice |
| `len=` | how long the note's gate is open: the voice is let go 30 ms before the note ends (at most half the note) |
| `vel=` | the level, and the accent: a velocity above 100 is accented in proportion, all of it at 127 |

## What it lowers to

The net is the same as for any tonal part, with one difference: on the acid voice a slide is a 303's. Each note's binding
carries what every tonal binding does (`velocity`, `durationSteps`, and `decay` / `glide` / `pitchDrop` when set), and a
note that a `^` slides into carries `"glide": 60` when the part has no `glide=` (`djlive::compile::SELF_SLIDE_MS`). On
every other voice that note carries no glide, and the `^` holds its note as it always has. Which voices slide by
themselves is the engine's to say: the entries of its instrument table that play its `"acid"` engine
(`djlive::compile::slides_by_itself`), today `acid` and `acid-square`.

The engine plays a net's notes on the acid lane so:

- **An accent is the velocity.** The nets carry an accent as velocity +27, so the voice reads it there: none at 100 and
  below, in proportion above, all of it at 127. A fully accented note is about 5 dB louder than the same note at 100,
  its filter opens about an octave further at the strike, its sweep falls in 90 ms at most (the 303 takes its shortest
  decay on an accent), and a smoothed copy of the accented sweep (one for the lane, as a 303 has one circuit) opens the
  filter up to 1.6 octaves more, so accents in a row climb.
- **A slide is a note's `glide`.** The voice still sounding the note before goes on: its envelopes are not struck again,
  its gate is held to the new note's length, and its pitch glides there as a 303's lag does, fast then settling (95% of
  the interval in the glide's time).
- **One note at a time.** A note that does not slide lets the one before go (8 ms), as a 303 plays one note at a time.
  Notes struck within 25 ms of each other are a chord: each sounds on a voice of its own (a lane has four, a harmony
  part six).
- **The sound**: a saw (or a square), band-limited (PolyBLEP), into a four-pole lowpass (trapezoidal one-pole stages
  with the feedback solved each sample and the stage input saturated), then a 25 Hz coupling highpass and the
  amplifier. The knobs' defaults are a classic squelch: cutoff 300 Hz, resonance 0.72, envelope 2.7 octaves, decay
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
A name that contains `acid` but is not a voice (`inst=acid-bass`) still plays the generic saw bass, and `check` warns that
it does and names `acid` among the voices with a sound of their own.

## Decisions

- **`inst=acid` is the real voice** (Matt's call, given in the item): the name promised a 303. `acid-square` is its
  square switch, a second entry in the instrument table, because a name is how a part picks a sound today.
- **The accent rides in the velocity, not a new key on the binding.** The nets already carry `!` as velocity +27, and the
  acid genre's own lines carry it as +25; a new `accent` key would move the CID of every net with an accented tonal note.
  The cost: a part with a `vel=` above 100 is a little accented without a `!`, as on a MIDI 303.
- **A slide is lowered to a glide on the acid voice, not inferred from held notes.** A held note is the TB-303 MIDI
  convention for a slide, but a tonal part's notes are two steps long unless it says otherwise, so every line of
  sixteenths would slide. The language knows the slides (`^`) and writes them; the engine only obeys `glide`, as it does
  on every voice. No program in the repo has `^` on an acid part without `glide=`, so no pinned CID moves.
- **60 ms** is the default slide: about a 303's. `glide=` changes it on this voice as on every other.
- **`decay=` is the 303's decay knob here, not the note's tail.** On a 303 the decay knob is the filter's; the level is
  gated by the note. On every other voice `decay=` is still the tail.
- **One note at a time, but chords sound.** A 303 is monophonic; a part may still write `[a1,e2]`, and both notes play.
- **The generator's versions keep the voices they had.** `gen@1` and `gen@2` pick a part's voice from the genre's
  instruments that are voices, and `acid` was only a family word when they were pinned, so neither picks it now
  (`beatscript-api::generate::VOICES_AFTER_V2`): every gen call resolves to the program it always did (`tests/gen.rs`). A later
  version may pick it; that is a new version.
- **The knobs are fixed defaults.** The language has no per-part cutoff, resonance or envelope today (Proposals); the
  deck's filter insert (`deck B: filter lp 400 -> 3k over 8 bars`, `resonance`) is the sweep a set already has.

## Not done

- The acid genre's own line (djrust `gen::theme::acid_bass_ring`) holds its slides into the next step and writes no
  `glide`, so a composed acid-house track in the app strikes those notes again instead of sliding. Writing `glide` there
  moves that genre's composed nets (its goldens), so it is left for its own change.
- `sound=synth:acid` (a role's sound option) plays the voice, but a `^` on that part does not slide by itself: the
  language only knows the part's `inst=`.
- A composed track whose bass is `acid` (the default bass of techno, edm, trance, speedcore and acid; an alternative in
  seven more genres) plays the 303 in the app (where no sampled bass is bound), with no slides.

## Proposals

For Matt's yes or no; none is built.

- **`cutoff=`, `res=`, `env=` on a tonal part** (Hz 20 to 12000, 0 to 1, octaves 0 to 6), the acid voice's other knobs,
  carried on each note's binding as `decay=` is (`cutoff`, `resonance`, `envMod`), written only when set so no CID moves,
  and a warning on a voice that has no filter of its own. The engine already reads them from an `"acid"` overlay object
  (`AcidParams::from_json`); the cost is the language's surface (grammar, `check`, beatscriptls, the ontology), not the sound.
- **`accent=` (0 to 1)**, the accent knob, the same way.
- **A cutoff that moves with the song** (`deck B: acid cutoff 300 -> 2k over 8 bars`): the gesture acid is made of,
  which per-part params cannot give. It would be a control noun on the deck's voice, as `resonance` is on its insert.
