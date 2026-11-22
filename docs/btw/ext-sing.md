# F9 `sing`: vocals in code

*Part of the btw extensions (docs/BTW-EXTENSIONS.md, F9). Module `crates/djlive/src/sing.rs`; the host's voices in
`djhost::vocals` and `djhost::btw_env`. Example: `examples/btw/sing.btw`. Tests: `crates/djlive/tests/ext_sing.rs`, the unit
tests in `sing.rs`, and `djhost` (`btw_env`, `vocals`).*

```text
deck B: melody <[a4 c5 e5 c5] [g4 b4 d5 b4]>
deck B: sing "lamps in the harbor, glowing slow" on melody voice=alto
```

Typed lyrics, sung on the notes of one of the program's own parts. Each syllable lands on a note, in order, timed to the part
(the line has no rhythm of its own); the lyric wraps while there are notes left and stops when they run out. It is a control line
that makes a `vocal` line (docs/VOICE.md) and puts it on the loop row.

## Grammar

```ebnf
sing    = "sing" STRING "on" role { param } ;                 (* as the body of a line: deck B: sing "words" on melody *)
role    = "bass" | "melody" | "harmony" | "arp" ;              (* a tonal part: the drums have no pitch (an error says so) *)
param   = "bars=" int                                         (* 1 to 32: how many bars of the part the line covers *)
        | "key=" key                                          (* the key the vocal is labelled with *)
        | "voice=" NAME ;                                     (* a formant preset or a neural voice *)
line    = [ when ":" ] [ target ":" ] sing ;                  (* `target` is the deck; with none, the only deck that has the role *)
```

Parameters come in any order, each once, and print sorted by name: `sing "robots on the road" on melody bars=2 key=am voice=alto`.
`sing` and `voice` are reserved words. The lyric is a quoted string with the escapes of every string in btw (`\"`, `\\`, `\n`).

## What it does

**1. The part.** The deck is the one the line is under (`deck B: sing ...`); a line with no deck is sung on the only deck that has
the role (two decks with it: an error that says to name one; the `master` has no parts). The notes are the part's **onsets** over
the bars the line covers, its cycles going round (an alternation `<a b>` is as long as it is): one note an onset, the **highest** when
several sound together (a chord sings its top note). A note lasts until the next onset less a sixteenth of air (more than a sixteenth
away; at most half a bar), the last until the end of the line (at most three beats): the rule of the voice's own fit
(`djengine::vocal::fit`), so a `sing` line and a `vocal create` line phrase alike. A rest in the part is a rest.

**2. The bars it covers.** `bars=N` says; else, in a **section**, the section's length; else the part's own cycle (the bars its net
takes to come round, with the deck's other alternations). At most 32 (the vocal noun's own limit): a longer cover is cut, with a
warning. A line outside every section of an arranged song is sung on the first section that has its part. A section that
plays in several stretches (F10: `stack(verse, extra)` with sections of other lengths) has its `sing` line said once, in the
first stretch, and sings over the whole section: the notes are taken from each stretch in turn, as that stretch plays them
(a chance or an alternation goes on from where the section was, as in the net), and `bars=` takes the first N bars of them.

**3. The syllables.** The lyric is read as the voice reads text (`djengine::vocal::lexicon::words`): letters, digits and apostrophes
make a word; a space, comma, full stop or line break ends one; a **hyphen** cuts a word into stretches (`ro-bo-tic`, a syllable
boundary you say yourself); `_` is a **rest**: a syllable that takes a note and sounds nothing. Text between slashes is
**phonemes** (`/h ii/`, docs/btw/phonemes.md), and a `/` with a space after it (or at the end of the lyric) is a **phrase break**:
`"/h ii/ ! / /h ii/ !"` is an escape, a break and an escape, and `"hold me / close"` a break between plain words. A stretch is counted in **vowel
groups**: a run of `a e i o u` is one syllable (`road` 1, `robots` 2, `melody` 3), `y` is a vowel unless it begins the word or sits
between two vowels, a silent final `e` does not count (`make`, `fire`), nor an `-ed` or `-es` that is not a syllable of its own
(`walked`, `makes`; `wanted` and `boxes` are two), a consonant and `le` is one (`table`); a stretch with no vowel (`hmm`) and a number
are one syllable. This is rough on purpose and written down; where the voice's own dictionary knows better the host says so (below)
and **its count wins**, so `fire` (two syllables to the voice) gets two notes and nothing drifts.

**4. The assignment.** Syllable *i* of the lyric is sung on note *i*; when the notes outnumber the syllables the lyric starts again
(`notes ≥ syllables`: `ceil(notes / syllables)` passes, the last cut at the word that reaches the last note); when the syllables
outnumber the notes the lyric stops (the last ones are not sung, and a warning says how many and that `bars=` covers more).

**5. The calls.** Two, in order, on the `vocal` noun:

```text
vocal create lyrics="robots on the road robots on" notes=[{start,dur,midi} ...] bars=2 name="robots on the road"
             preset=robot key="A minor" bpm=120
vocal to_row play=true
```

`notes` is the vocal noun's own parameter (a list of `{start, dur, midi}` in beats from the start of the line, one for each syllable
in order): **no extension of the noun was needed.** `preset` is the voice, `key` is `key=` or the program's `key` (a mode becomes its
relative major or minor), `bpm` is the program's `tempo` (none: the master's, as for every vocal), `name` is the first words of the
lyric. `to_row play=true` is what `vocal hook` does: the line is rendered to exactly its bars and **loops on the loop row**, on the
loop channel's boundary. The two calls are one control: they fire together, once, at the line's bar.

**6. When it fires and what the editor shows.** A control line like any other: it fires at its time word (no word: the first bar
line after it is added, or its section's first bar), carries its `span` and `via` through `let`, `def`, `repeat` and `section`
expansion, lights as it fires (`Marks`) and shows in `Compiled.controls`. Its **identity** is the line and what it is sung on
(`<line> [notes 1a2b3c4d]`, the hash of the first call): the same text over an edited melody is another control, so a hot reload sings
it again over the new notes, and an edit of nothing does not. `for N bars` is an error: a sung line loops on the loop row and has no
off of its own (see Not done).

## The voice

`voice=NAME` is a name from the host's voices: the formant presets (`robot`, `robot-choir`, `android`, `choir-aah`, `choir-ooh`,
`soprano`, `bass-choir`, `talkbox`; `voice-robot` works too, as it does for `vocal`) and the **neural voices** (`alto`, `tenor`,
`baritone`, `bass`: `djhost::vocals::NEURAL_VOICES`). A name that is neither is an error at the name with the nearest names
(`` `altoo` is not a voice ``, `did you mean `alto`?`). A line that names none sings in the host's **default**: the neural voice when
it is installed (`alto`), else the formant default (`robot`).

**The neural voice and its fallback.** A line that asks for a neural voice (by name, or by being the default) on a host where it is
not installed is still valid:

* `check` warns, at the voice: `the neural voice is not installed: falling back to the formant voice` (`djlive::sing::FALLBACK`),
  with `it plays on `robot`; name a formant voice to say so plainly: voice=robot`;
* the line plays on the formant fallback (`robot`): the compiled call carries `preset=robot`;
* when the line fires, the runtime's log says the same (`Control.notice`, a word in `Live::words`: "bar 3: the neural voice is not
  installed: falling back to the formant voice");
* the canonical text, and so the CID, keep what was written (`voice=alto`): the program is the same on every host.

Which host has the neural voice is told by `env::Voices` (`neural_installed`); with **no voices to ask** (`Env::none`, a checker
with no host) the line compiles, the voice is passed on as written, and a warning says `cannot check the voice: nothing here knows
the voices, or whether the neural voice is installed`.

The `vocal` sugar says the same. `vocal create "words" preset=alto` (and `hook`) is checked against the same voices: an unknown
`preset` is an error with the nearest names, a neural one that is not installed warns with the same wording at the `preset` value and plays
on `robot`, and on a host whose default is the neural voice and has it, a line that names no `preset` sings in it. A host with no
`voices` in its `Env` leaves the sugar exactly as it was, and so does a host whose default voice is a formant one (the case here).

## The host's side (`djhost`)

* `djhost::vocals`: `NEURAL_VOICES`, `FALLBACK_VOICE` (`robot`), `voice_names()`, `default_voice(installed)`,
  `neural_models_dir()` (`$DJRUST_VOICE_MODELS`, else `<support>/models/voice`), `neural_installed()` and the pure
  `neural_installed_in(engine_built, dir)`.
* `djhost::btw_env::VoicesView` implements `djlive::env::Voices` (its `neural` field is `vocals::neural_installed()` and a test sets it),
  and `BtwEnv::env()` hands it to the checker (`Env.voices`). `sung_syllables` is the dictionary's own count
  (`djengine::vocal::fit::syllables`), so the checker agrees with what the voice will sing.

## Canonical form

`sing "text" on ROLE` then the parameters sorted by name, each `name=value`: `sing "robots on the road" on melody bars=2 key=am voice=alto`.
The key is spelled as everywhere (`Am` → `am`, `Bbm` → `a#m`). The string keeps what was typed (it is the program), spacing round the
tokens and comments are free, so random reformatting keeps the CID (tested 150 times) and every semantic edit (a word, the role, the
voice, `bars=`, the time word, a note of the melody) changes it. The example is a fixed point
(`tests/examples_btw.rs`).

## Decisions

* **There is no neural voice in this repo.** The brief took `feat/vocal-neural` for a neural singing voice; that branch's "neural" is the
  *beat detector* (Beat This!, `djhost/neural`, docs/BEAT-DETECTION.md) turned on by default, and its voice is the formant one
  (docs/VOICE.md lists Kokoro as "not built"). So the mechanism is built and tested against fake environments, and the real host
  says honestly that the neural voice is not installed (`vocals::NEURAL_ENGINE_BUILT` is `false`, the one switch for a future engine,
  with the models' folder `neural_models_dir()`). The neural **names** (`alto`, `tenor`, `baritone`, `bass`) are reserved by the host
  so `voice=alto` (the contract's own example) is a voice, today a fallback. None of them shadows a formant preset.
* **The vowel-group syllabifier is the language's; the dictionary's count wins when the host knows it** (`Voices::sung_syllables`). The
  rule is simple and written down; the exception keeps a note and a syllable together where the two would disagree (`fire`, `being`,
  `real`). Without a host the rule alone is used and is deterministic.
* **Whole words go to the voice, not our pieces.** The lyric is repeated and cut at a word, and the voice drops the surplus syllables
  of the last word (it already says "the last N are not sung" for notes it has no room for). Splitting words into our own pieces
  would have lost the dictionary's pronunciation of the whole word.
* **The cover is the section, else the part's own cycle**, not the lyric's length: a melody of one bar sung over an eight-bar
  section is eight bars of notes (the lyric wraps), and over no section is its one bar (`bars=` says more). A warning says when a lyric
  is longer than the notes it has.
* **Legato.** A note lasts to the next with a sixteenth of air, as `vocal`'s own fit does, not as long as the part's `len=`: a sung
  vowel needs the time, and a rest after a note holds it (up to half a bar).
* **The highest note of a chord**, the top line, is sung.
* **The control's key carries a hash of the notes** so an edited melody sings again on a hot reload (see 6).
* **No voice named, no warning on a host that has no neural voice at all** (the default is then a formant voice). The warning is for
  a line that asks for a neural voice, by name or by being the host's default.
* **A global line in a song** is sung on the first section that has the part, at its own time word.
* **A drum has no pitch**: `sing ... on kick` is an error naming the tonal roles.
* **`for N bars` is refused** (no off: the clip id the loop row gives is only known when the line fires).
* **Control order.** `sing` controls come after the program's other control lines in `Compiled.controls` (they need the decks' nets
  built, every stretch's of them): lines that fire on the same bar run in that order.
* **A constraint reads the line's windows** (`assert never B during sing`, docs/btw/ext-assert.md, "Constraints"): `Compiled::sing` has, for each line, the bar it starts on, the
  bar its section ends on, the bars of one pass and the notes with their lengths (`SingLine`), recorded whatever the host (so the verdict is the same with the browser's sung part in the
  nets, with the desktop's loop row and with no host). A line timed `every N bars`, `on next phrase` or inside a bar, or guarded by a `when` that reads a key, has no start: a constraint
  about the vocal says so and does not guess.
* **A carried copy does not sing again.** The copy of a line that F10 carries into a later stretch of a stacked section is not a
  second control (as for a mix or a duck): the line is in the stretch where its section begins and covers the section.

## The sung voice's own sound (6627)

`sing` lines were hard to understand, and nobody can listen in CI, so `crates/btwapi/tests/vocal_distinct.rs` measures the voice headless
through the engine the page runs (the F1 and F2 of a vowel held on one pitch and their distance in Bark, A-weighted level per word,
how far a note rings after it ends, the level of each preset) and pins what it found. The engine is the public `djrust` crate; its
changes are sound only, no program's text, net or CID moves:

* **A syllable is held and let go.** A net's note on a voice used a plucked string's envelope: a decay 1.1 times as long as the note, only 1
  to 7 dB down 0.1 to 0.3 s after it ended, so every syllable smeared into the next. The vowel now keeps its level for the note's length (about
  2 dB of droop), never under a 90 ms nucleus, the syllable **closes on its last consonant** (the lane never began it: the `t` of "street" and the
  `v` of "rave" were silent in the browser), and the note releases in about 30 ms.
* **`decay=` on a voice part is that release, never the vowel.** `deck A: melody ... inst=voice-robot decay=160` used to put 160 ms on the vowel's
  own decay (27 dB down by the middle of a one-second note: mostly consonants left). It now sets how long the tail rings after the note; the
  held vowel is the same with or without it. A sung part (`sing ... on melody`) has no `decay=` of its own and ignores the melody's.
* **The back vowels are apart.** A held `/oh/` was exactly a held `/uu/` (0.00 Bark): the lips now round on its way to `oo`, so it settles on
  F1 440, F2 830 Hz; `/aa/` and `/uh/` (the schwa of "the" and "a") are 2.0 Bark apart, were 1.0. (The choir presets' "aah" is a little more
  central.) "the" is the loudest word of `we see the street`; `we` and `street` are 5 and 6 dB under it, by their vowel.
* **The voices are as loud as each other.** The track's loudness gain is set by a probe that plays at 8 kHz, where a voice's upper formants do not
  exist: `voice-soprano` came out 19 dB under `voice-robot`, and one `/ee/` in a line took 13 dB off every note of it. A track with a voice is probed
  at 24 kHz, and `voice-android` and `voice-talkbox` (parallel formants, 13 and 18 dB under the rest) are made up: every preset is within 4 dB.
  There is no reverb in the voice presets (reverb is a `reverb` control line), so there was no preset reverb to take out; what made
  `voice-soprano` ring was its envelope and its level.

## Not done

* **A real neural voice.** See Decisions: the engine, its models and their pinned download do not exist yet; the fallback is what
  runs.
* **A sung line is not stopped when its section ends.** It loops on the loop row until stopped (`loops stop_all`, or the row's own
  controls); the clip's id is only known to the host when it fires, so `for N bars` and section ends cannot put it back by a static
  call. A later worker could have `vocal to_row` answer an id the runtime remembers for an undo.
* **The line starts on the loop row's own boundary**, not tied to the net's phase: a line fired at a section's first bar starts with it
  (the loop channel's quantum), one fired mid-loop is where the quantum puts it.
* **An `evolve` is not followed.** The notes are the part as written (generation 0): a melody that rotates each generation is sung
  as it began.
* **No melisma** (one syllable over several notes) and **no word split over a rest**: a note is a syllable (`vocal`'s own limit); a `_`
  is how to leave a note out.
* **English, and the dictionary's own spelling-to-sound rules for words it does not know** (`vocal`'s limits). Numbers count as one
  syllable without a voice host to spell them.
* **The offline render** (`btw render`) carries the mixer's control lines but has no op for a vocal call (docs/btw/ext-render.md), so a sung line is not in it (and is counted under `not in this render`, kind `vocal`).
* **A consonant the line drops (found while measuring 6627, not changed).** When a syllable has two consonants before its note (the
  `s` and `t` of "street") or two after its coda (the `d` and `z` of "hands"), the cluster part (`voxc`) puts them on the same step
  and note, and `sung_part` keeps only the first (`dedup_by_key((step, note))`): "street" is sung `s r ee t` and `@hands_up` loses its
  `z`. Giving each its own step changes the net of any program with such a syllable, and `examples/btw/vocals.btw` (`@hands_up`) is
  pinned (`cid-pins.json`), so it is left for a decision about the pins.
* The vocal noun's handler does not know the neural names: when an engine lands, `djapp/src/control/vocal.rs` takes `preset=alto`.
