# Keys, Camelot and key lock

Each deck's key, written on the Camelot wheel, with how the two decks mix and the transpose that
would make them, plus key lock (a tempo change that keeps the pitch). The code is in
`djengine::key` and `djengine::keylock` (pure, `no_std`), the app side in
`crates/djapp/src/keymash.rs`, the socket's `key` noun, and `djctl key`.

## Where a deck's key comes from

| source | when | how |
|---|---|---|
| `track` | a generated (net) deck | the key it was **composed** in: the project's `rootNote` and `scaleName`, the same thing the loop sets write as "A minor". Exact, and nothing is analysed. |
| `tag` | someone set it (`key set`) | a hand tag on what the deck holds; it wins until the deck loads something else |
| `audio` | a sample deck | **detected** from the audio on a worker thread as soon as it loads, cached by content id |
| `detecting` / `atonal` / `none` | | still running / the audio has no pitch to speak of (drums, noise) / nothing loaded |

Modes fold onto the major or minor their third gives, with the same tonic: dorian, phrygian,
aeolian, harmonic minor, minor pentatonic and blues read as minor, and mixolydian, lydian and
major pentatonic as major. That is what the wheel can say, and it is what a DJ hears as the key
centre.

## Camelot

The wheel DJs write keys on: 1A–12A minor, 1B–12B major. One step round the wheel is a fifth, and
A ↔ B at one number is the relative major / minor. 8B is C major, 8A is A minor.

`compat(a, b)` grades a pair by that geometry:

| compat | example | mixes? |
|---|---|---|
| `same` | 8A ↔ 8A | yes |
| `relative` | 8A ↔ 8B | yes (the same notes) |
| `adjacent` | 8A ↔ 7A / 9A | yes (one note differs) |
| `diagonal` | 8A ↔ 9B | for a phrase |
| `boost` | 8A ↔ 10A | for a short blend (the "energy boost") |
| `parallel` | A minor ↔ A major | a deliberate colour change |
| `clash` | anything else | transpose, or cut |

`suggest_transpose(a, b)` finds the shift of `b` within ±6 semitones that mixes with `a`. Each
semitone moved costs against the compatibility score, because a stretched part sounds less like
itself the further it moves. So a 1-semitone move onto the relative key beats a 5-semitone move
onto the same key. The status bar shows `A 8A · B 3B  clash (B -1 → 8B)`, coloured green when the
decks mix, amber when they blend, red when they clash.

## Detection (external audio)

1. **Chroma** (`key::chroma_with`): 8192-point Hann frames with a 4096 hop over at most 60 s
   (longer audio is read with a larger hop, not truncated). FFT magnitudes are fourth-root
   compressed so one loud partial cannot own a frame, and summed per **semitone** between 50 Hz
   and 4 kHz.
2. **Harmonic subtraction** ("Overtones", below): lowest note first, each semitone's 2nd–6th
   harmonics lose `0.8 / h` of its energy. Then everything folds onto 12 pitch classes.
3. **Key profiles**: Pearson correlation with the 24 rotations of three published major / minor
   profiles:
   - Krumhansl & Kessler 1982, probe-tone ratings (*Psychological Review* 89, table 1). djrust's
     loop-root estimate already uses this one.
   - Temperley 2007, the Kostka–Payne corpus profile (*Music and Probability*, MIT Press; the
     numbers as music21 publishes them).
   - Albrecht & Shanahan 2013 (*Music Perception* 31(1)).

   `ensemble` (the default) averages the three correlations per key. They err in different
   directions (KK leans to the relative key, Temperley to the dominant), and `votes` shows each
   one's own answer so a caller can see when they disagree.
4. `confidence` (0–1) is how clearly the winner beat the next key. `tonal` is false when the
   chroma is flat (drums, noise), and the deck then shows no key rather than a guess.

### Overtones

A synth note at pitch class *p* also lights up *p + 7* through its 3rd and 6th harmonics, and
*p + 4* through its 5th. Plain chroma therefore votes for the key a fifth above: on rendered
generated tracks it answered one Camelot step up about a quarter of the time. Subtracting the
harmonics in the semitone domain before folding halves that.

### How well it works

Measured on 44 renders of generated tracks, whose keys are known (22 genres × seeds 1–2, 30 s
each; `crates/djengine/tests/key_corpus.rs`, opt-in):

| | exact key | mixes with the true key |
|---|---|---|
| plain chroma (djrust's), ensemble | 10 / 44 | 24 / 44 |
| **default** (harmonic subtraction, 50 Hz–4 kHz), ensemble | **14 / 44** | **33 / 44** |
| ceiling: the profiles on the notes the nets actually play | 25 / 44 | 36 / 44 |

So even with the exact notes, profile matching agrees with the generator's own label on only 25
of these modal, pentatonic, riff-based tracks. A dorian loop *is* ambiguous between its tonic
minor and the major a tone below, and pentatonic material between three keys. The audio estimate
gets within reach of that ceiling on what a DJ needs (a key that mixes: 33 vs 36). On tonal
material it is right every time: synthetic I–IV–V–I / i–iv–V–i cadences in all 24 keys,
`tests/key.rs`.

That is why a generated deck never uses detection: its composed key is exact.

## Key lock

A plain tempo change on a sample deck is varispeed: 5 % faster is 5 % sharp (+0.84 semitones).
With key lock on (`key lock deck=A on=true`), `key tempo deck=A bpm=126` does this:

1. A worker thread renders a copy of the deck's **original** audio, time-stretched to the new
   tempo with the pitch kept. This is djrust's WSOLA, `stretch::time_fit`: Hann frames laid at a
   fixed hop, each read near where the tempo says and nudged to the offset whose waveform best
   continues the previous frame. It always stretches from the original, never a stretch of a
   stretch.
2. The copy is written to `<support>/keylock/<content-id>.wav` and named in the action log's
   sample map, so a replay or an offline render finds the audio the deck played.
3. On the window's next pass the copy goes onto the deck at the same place in the music (the
   position scales by the rate), with a beat grid at the new tempo, and playing resumes if it was
   playing. The engine then sees a track that *is* at 126 BPM, so sync, cues and the pitch fader
   act around that.

Below 0.1 semitones of drift (about 0.6 % of tempo) nothing is rendered, and the widest stretch
is 0.5×–2×. **Net decks are always locked**: their notes are synthesised at the tempo asked, so
their pitch never moves (`key_lock_inherent: true`).

Not done: key lock that follows the pitch fader *continuously*. The swap is a reload, so it fits
a deliberate tempo change (`key tempo`, a mashup's tempo), not riding the fader. Continuous lock
needs a stretching voice inside djrust's render path (engine work in `stackdump/djrust`). The
loop row's clips already have their own key lock (`clips key_lock`), which re-renders off-thread
the same way.

## Control (`key` noun, MCP `djapp_key`)

- `status` (default): each deck's `name`, `short`, `camelot`, `source`, `confidence`,
  `key_lock`, `key_lock_inherent`, `bpm`, and `pair` (`compat`, `score`, `harmonic`, `hint`,
  plus `suggest` when they clash).
- `detect {deck?, profile?}`: the analysis now, with `votes` and the `chroma`.
- `set {deck, key|null}`, `lock {deck, on?}`, `tempo {deck, bpm}`, `compat {a, b}`.

Offline:

```
djctl key detect song.wav [--profile temperley]     # any WAV; more formats with --features decode
djctl key camelot "F#m"                            # 11A
djctl key compat 8A 3B                             # clash; suggest -1 → 8B (relative)
djctl key track dnb 2602                           # G minor, 6A, 174 BPM, its CID
```

## Tests

- `crates/djengine/tests/key.rs`: cadences in all 24 keys (ensemble 24/24, each profile ≥ 22),
  noise is not tonal, Camelot both ways, parsing, generated tracks' keys, compatibility,
  transposes.
- `crates/djengine/tests/keylock.rs`: a stretched sine keeps 440 Hz at the new length; varispeed
  arithmetic; transposing.
- `crates/djengine/tests/key_corpus.rs` (opt-in, `--ignored`): the table above.
- `crates/djapp/src/keymash_tests.rs`: two sample decks' keys on the socket and in the readout,
  tags, profiles, and key lock end to end (the copy lands shorter and reads as the same key).
