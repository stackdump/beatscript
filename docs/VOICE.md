# The formant voice

*Beyond Swift.* Robotic, stylised vocals, a Daft-Punk-ish robot choir rather than realistic
singing, synthesised from nothing: no recorded singing, no samples, no voice model. It is pure
Rust in the public engine (`djrust::fx::formant`, MIT), `no_std`, and allocation-free on the
render thread. The nets sequence it the way they sequence notes.

| Where | What |
|---|---|
| `djrust/src/fx/formant.rs` | The DSP: sources, resonators, vowels, syllables, presets, `FormantVoice` (one note) and `FormantSynth` (polyphonic, for the vocoder and offline renders) |
| `djrust/src/wave/runner.rs` | The wave engine's `"formant"` engine: a tonal lane whose pooled voices sing |
| `djrust/src/wave/instruments.rs` | `voice-<preset>` instruments become formant voices |
| `djrust/src/pflow.rs` | A binding's `"vowel"` (`MidiBinding::syllable`) |
| `djrust/src/gen/vox.rs` | The `vox` compose option: a choir or robot part |
| `djrust/src/host/deck/vocoder.rs` | The vocoder's synth can sing (`voice`) |
| `djapp` `voice` noun | Socket, `djapp_voice` MCP tool, `djctl app voice …` |
| `djctl voice` | Offline: `presets`, `sing` |
| `djengine::vocal` | Typed lyrics: the dictionary, the letter-to-sound rules, syllables, the fit to a key, the `VocalLine` object and its render (see "Typed lyrics" below) |
| `djapp` `vocal` noun, `djapp_vocal`, the Vocal window, `djctl vocal` | Lyrics sung on pitch: create, save, license, fork, send to the loop row, a deck or the vocoder |

## The sound

A **source** runs through five **formant resonators**:

- **Sources.** `saw` (PolyBLEP: buzzy, the classic robot), `pulse` (PolyBLEP with a duty
  cycle: hollow, nasal), and `glottal`, the derivative of Rosenberg's (1971) trigonometric
  glottal pulse, band-limited at the closure. `glottal` is the most voice-like.
- **The tract.** Five Klatt (1980) digital resonators (`y = A x + B y1 + C y2`):
  - **Cascade** (default): in series, so the formants' relative levels come out of the filter
    by themselves. The result is smooth and natural. The output is levelled by F1's Q, so every
    vowel sits at about the same loudness.
  - **Parallel**: each formant is its own band-pass, normalised to its peak with its own level,
    and the signs alternate as in Klatt's parallel branch. Brighter, more like a talk box.
- **Vowels.** `a e i o u`, plus the choir's `ah` (as in "but") and `oo` (as in "book"), in
  knob order 0–6. F1–F3 are Peterson & Barney's (1952) adult-male averages, except that `a` (770/1040 Hz) and
  `ah` (610/1260) sit a little further apart than speech has them (730/1090 and 640/1190: 1.0 Bark, now 2.0), because a
  held note has no context to tell the schwa from `a`. F4 and F5 and the
  bandwidths are Klatt's (1980) defaults. A fractional position morphs between neighbours
  (log frequency). A diphthong that glides to `oo` (`oh`, `ow`) ends with the lips rounded (F1 x 0.95,
  F2 x 0.78), so a held `oh` is not a held `oo` (docs/btw/ext-sing.md, "The sung voice's own sound").
- **The note.** In a net a note is held for its length (about 2 dB of droop, never under 90 ms), closes on its
  syllable's last consonant, and releases in about 30 ms; the part's `decay=` is the length of that release.
  A track with a voice in it is leveled by a loudness probe at 24 kHz (8 kHz for everything else), and the presets
  are within 4 dB of each other.
- **Morphing.** A note glides its formants from wherever they were to its vowel (a one-pole
  glide on log frequency, `morph_ms`). Consecutive notes therefore morph `ah → oo` rather
  than jump. A voice that was silent snaps to its vowel.
- **Formant shift.** `formant_shift` in semitones moves every formant and its bandwidth:
  + is a smaller, brighter tract (soprano, child), - is a bigger one (bass).
- **Vibrato.** A rate (Hz) and a depth (cents), plus a delay over which it fades in, as a
  singer's does.
- **Choir.** `unison` copies (up to 8) of each note:
  - Detuned evenly across ±`spread` cents.
  - Each copy has its own vibrato phase and rate and a slow random `drift`.
  - The copies alternate between two resonator banks `formant_spread` (±3 %) apart, so the
    section sounds like two vocal tracts, not one.
  - `breath` adds aspiration noise through the formants.

### Syllables

A note can sing a **syllable**: an optional consonant, a vowel, an optional glide to a second
vowel (a diphthong) and an optional closing consonant (the coda). The consonants are textbook
phonetics, not taken from any synthesiser's code:

| onset | how it sounds |
|---|---|
| `b d g` | A short closure, a noise burst, then the formants glide out of the place's locus (F2 800 / 1700 / 2000 Hz) |
| `p t k` | The burst, then aspiration through the formants until voicing starts (~50 ms) |
| `m n ng` | A quiet, dark murmur (60-70 ms), then the release |
| `l r w y` | Voiced all through, with a slow transition from the consonant's formants (r's low F3, w's low F2, y's high F2) |
| `s sh` | High-passed noise (5 kHz, 2.6 kHz), then the voice |
| `f th` | The same, weaker and broader (6.5 kHz, 7 kHz) |
| `z zh v dh` | The fricative with the voice's buzz under it (a dark, quiet voice bar), then the voice |
| `ch j` | An affricate: a burst first, then the noise (`j` is the "j" of "jazz"; voiced) |
| `h` | Aspiration, then the voice |

As a **coda** the same consonants close the note when it is let go: the formants glide to the
consonant's locus, a voiceless one cuts the voice and hisses or bursts, a voiced one keeps a
quiet buzz, a nasal or liquid keeps the voice and darkens it. The synth holds the gate open for the
consonant (50-95 ms), then releases. A consonant can also stand alone (`.t`): noise and a hum,
no vowel: the `s` and `t` of "street" are such cluster events (see "Typed lyrics").

The vowels are `a e i o u ah oo` (knob positions 0-6, the live vowel knob's range) and three more
that only a syllable names: `ae` ("cat"), `ih` ("bit"), `er` ("bird"), positions 7-9. A diphthong
glides from its first vowel to its second after the note has settled (110 ms, then a 75 ms time
constant): `ay` is `a+ih`, `ow` `o+oo`, `oy` `o+ih`, `ey` `e+ih`.

The text is `[onset]vowel[+glide][.coda]`: `da`, `sha`, `ah+ih` (the "i" of "ride"), `ra+ih.t`
("right"), `.t` (a consonant on its own). In a document a syllable is that text (`"da"`). On a
binding it is packed into a `u16` (`Syllable::code`: `onset << 8 | vowel + 1`, 0 = none); the packed
form carries the onset and the vowel only, and every code from before the new consonants means what
it did.

### Presets

| preset | instrument | source | voices | character |
|---|---|---|---|---|
| `robot` | `voice-robot` | saw | 1 | hard vowel changes, no vibrato: the vocoder robot |
| `robot-choir` | `voice-robot-choir` | saw | 4 | a few cents apart, quick morphs: the machine choir |
| `android` | `voice-android` | pulse, parallel | 2 | hollow, fast shallow vibrato |
| `choir-aah` | `voice-choir-aah` | glottal | 6 | aah, late slow vibrato, drift, a little breath |
| `choir-ooh` | `voice-choir-ooh` | glottal | 6 | ooh, darker |
| `soprano` | `voice-soprano` | glottal | 4 | formants +4 st |
| `bass-choir` | `voice-bass-choir` | glottal | 5 | formants -3 st |
| `talkbox` | `voice-talkbox` | saw, parallel | 1 | a talk box on a lead |

`djctl voice presets` lists them.

## Sequenced by the nets

- **Per note.** A transition's MIDI binding may carry `"vowel"`:

  ```json
  "t3": {"midi": {"note": 64, "velocity": 84, "duration": 300, "vowel": "da"}}
  ```

  A formant lane sings that syllable when the transition fires. A note without one sings the
  lane's vowel: the preset's own, or the one set live (below).
- **Documents.** `"vowel"` is written only when a note has one, so every existing document
  keeps its canonical text and CID (tests/voice.rs holds this). A value that does not parse is
  no vowel.
- **The instrument.** A track whose instrument is `voice-<preset>` plays through the formant
  engine on the app's path (`WaveOptions::extended`). On beats' parity path
  (`WaveOptions::default`) the instrument table is not consulted, so a `voice-*` track plays
  its name's family voice, as beats itself would.
- **Live vowel.** `WaveRunner::set_vowel(net?, v)` and the deck's `Engine::set_vowel(deck, net?,
  v)` move the plain notes' vowel. The sounding notes glide there at the preset's morph time.
  Notes with their own syllable keep it.

### The generator: `vox`

`vox` is an opt-in compose option, like `hook` or `chordSnap`. It is stored in the spec's
`extra`, so a share replays it and its CID reflects it:

| value | part |
|---|---|
| `choir` | The chord plan as a held, stacked choir (three chord tones per chord), one vowel per chord from a seeded cycle (`ah oo ah o`, …), on `voice-choir-aah` or `voice-choir-ooh` |
| `robot` | A chant: a seeded four-syllable word (`ro bo da ti`, …) repeated on the chord tones in a seeded rhythm (quarters, off-beats, a three-step figure), on `voice-robot-choir` or `voice-robot` |
| `auto` (or `true`) | What the genre suits: trance, synthwave, ambient (and italo) get a choir; house, techno, edm, garage (electro, deephouse, acid) a robot; the rest none |
| `off` / `false` | Nothing (the plain spec) |

- **The net.** The part is the net `vox` on MIDI channel 8 (free in every genre), in the
  octave above the root. It is in group `harmony` (choir) or `melody` (robot).
- **Instruments.** `instruments: {"vox": "voice-talkbox"}` picks the voice.
- **Structure.** With a structure, it sings in the sections where the melody, harmony or arp
  plays, except the intro and outro.
- **Nothing else moves.** Like the other techniques, it draws only from its own Mulberry32 (the
  seed with `vox/<genre>`) and is added after every other net is composed, so switching it on
  changes nothing but `vox` and `struct-vox`. Every net of `vox: off` is byte-identical with
  `vox` on (tests/voice.rs).

```sh
djctl render trance:3 out.wav --extended --options '{"vox":"choir"}'
djctl render house:7 vox.wav --extended --options '{"vox":"robot"}' --solo vox
```

## Into the vocoder

The vocoder's internal synth can **sing**. With `voice` set to a preset, the synth side renders
`FormantSynth` instead of the oscillators:

- Its notes are the synth's (`notes`, `note`, the keys).
- Its level is `synth_level`.
- Its vowel is the vocoder's `vowel` knob (a e i o u).
- As the **carrier**, a mic talking through it is a robot choir that stays on the notes.
- As the **modulator**, it shapes another source.

| how | |
|---|---|
| socket / MCP | `vocoder voice=robot-choir`, `voice=off`; a source spelled `voice` or `voice:NAME` is that synth (`carrier=voice:choir-aah`) |
| live op / arrangement | `vocoder {"voice": "choir-aah"}`, `{"carrier": "voice:robot"}`; the setup ops replay it |
| `voice` noun | `voice vocoder preset=choir-aah carrier=true` |
| state | `vocoder.settings.voice` (null: the oscillator synth) |

Switching costs nothing on the render thread (tests/vocoder.rs counts allocations), and the
action log says `vocoder voice choir-aah`.

## Control: the `voice` noun

djapp's socket method `voice` (alias `formant`) is the MCP tool `djapp_voice`, and on the
command line `djctl app voice <verb> k=v…`. It is not `vox`: that noun is the vocal sample
pads.

| verb | |
|---|---|
| `status` (default) | Presets, vowels, onsets, each deck's sung nets (net, instrument), the vocoder's voice |
| `presets` | The presets (name, summary, instrument, source, unison) |
| `vowel` | `deck`?, `vowel` (a e i o u ah oo, or 0–6), `net`?: the vowel a deck's voices sing, live. Refused on a deck with no voice |
| `load` | `deck`?, `genre` (trance), `seed`, `style` (choir · robot · auto; default the preset's kind), `preset`, `structure`: compose a track with a sung part onto the deck |
| `vocoder` | `preset` (or `off`), `carrier`?: the vocoder's synth sings |

```sh
djctl app voice load deck=A genre=synthwave seed=12 preset=robot-choir
djctl app voice vowel deck=A vowel=oo
djctl app voice vocoder preset=choir-aah carrier=true
djctl voice sing robot.wav --preset robot-choir --melody "A3:ro C4:bo E4:da G4:ti A4:oo" --bpm 100
djctl voice sing pad.wav --preset choir-ooh --notes A2,E3,A3,C4 --seconds 8
```

## Tests

- `djrust/src/fx/formant.rs`:
  - Each vowel's cascade response peaks within 6 % of its F1, F2 and F3.
  - A rendered 60 Hz glottal note has spectral peaks at each vowel's F1–F3 (within 8 % + one
    harmonic).
  - The formant shift moves the peaks.
  - Morphing glides rather than jumps.
  - Onsets start from their locus.
  - Every source and topology is bounded.
  - Syllables round-trip.
  - Renders are bit-identical.
- `djrust/tests/voice.rs`:
  - `vox` adds only its part.
  - `auto` by genre.
  - `"vowel"` round-trips and leaves plain documents' CIDs alone.
  - Every `voice-*` instrument renders through formant lanes, bit-identically.
  - The live vowel moves.
  - The parity path ignores the voice table.
  - The instruments override applies.
- `djrust/tests/vocoder.rs`: the voice carrier sounds, differs from the synth, follows the
  vowel knob, talks through the mic, does not allocate, logs, and replays through the setup
  ops and the live op.
- djapp (`app_tests.rs`): `voice` load / status / vowel / vocoder and the refusals. The
  registry round trip checks every verb.
- djengine (`tests/techniques.rs`): `vox` is a compose option.
- djctl (`voice_cmd.rs`): `sing` is deterministic and refuses bad input.
- `djrust/src/fx/formant.rs`, the lyrics' additions: the extended syllable text round-trips; `s` is brighter than
  `sh`, which is brighter than `f`/`th`; `z v zh` buzz and `s f sh` do not; an affricate bursts first; a coda keeps the
  note sounding for its consonant (a hiss for `s`, a hum for `n`) and ends by itself; a diphthong glides; every
  onset and coda is bounded and deterministic.
- `djengine::vocal`: the dictionary loads (124 925 words) and knows the hooks' words; the rules; syllabification;
  the fit (in the key, deterministic, on the grid, refuses too many syllables); the object round-trips, its license
  is in its CID, a fork credits; a render is exactly the loop long, audible in every bar, under -0.9 dBFS peak.
- `djhost::vocals`: the store. djapp (`app_tests.rs`): `vocal` create, send, save, relicense, fork, share and the refusals.
  djctl (`vocal_cmd.rs`): create, render, and the same audio from lyrics or from the saved JSON.

## Typed lyrics: the vocal line

*Built* (the CC0 path of the notes below). Type words, hear them sung on pitch by the robot
voice. No model, no recording, no download.

```
lyrics ──► words ──► phones ───────► syllables ──► notes ─────────► the formant voice
          (digits    lexicon, else   maximal onset  fitted to a key  onset, vowel (+glide),
           spelled)  letter-to-sound  rule          and a bar grid   coda, cluster events
```

**Phonemes.** `djengine::vocal::lexicon` and `djengine::vocal::phoneme` (docs/btw/phonemes.md, the headline):

- **The escape.** Text between slashes is BeatScript phonemes (`/w ee - aa r/`), set `beatscript-v1`, CC0, our own: sung as
  written, no dictionary asked, so it sounds the same on every engine and build.
- **The BeatScript lexicon** (`crates/djengine/data/lexicon.txt`, versioned, CC0, written by hand here): common English words that
  spelling rules get wrong, and the genre's words. Looked up first.
- **The rules** are for everything else, written here from English spelling conventions: digraphs (`sh ch th ph wh ck ng qu`),
  soft `c` and `g`, a silent final `e` that lengthens the vowel before it, the common vowel pairs, `-ed` and `-es`, doubled
  letters, a lone letter read as its name. Nothing comes from espeak-ng or any GPL code. They are rough, and the words they sound
  out are reported (`check` warns with the rules' spelling; `vocal create` answers `warnings`; the Vocal window shows them in
  amber). Respell (`ro-bo-tic`), escape, or propose the word for the lexicon (`tools/lexicon/propose.py`).
- **The booth only:** the desktop app links `crates/djlexicon-booth`, the CMU Pronouncing Dictionary (BSD-2-Clause, its notice
  in that crate), consulted after the BeatScript lexicon and before the rules. It is not in the public repo (code MIT, programs and
  content CC0): no public build
  (the browser wasm, beatscript.app's server, the public crate) can link it (its `build.rs` and `scripts/check-public-build.sh`).
- **Numbers** are spelled (`140` is "one hundred forty"); a comma, full stop or line break ends a
  phrase; `_` is a rest.
- Other licences considered: misaki (Apache-2.0, but its fallback is espeak-ng) and voice-g2p
  (MIT, dictionary provenance unchecked) need no use now; both stay options for more languages.

**Syllables to notes.** A syllable becomes one sung note (`djengine::vocal::phone`). The consonant
next to the vowel is its onset, the first after it its coda; the other consonants of a cluster
(the `s` and `t` of "street", the `s` of "tracks") are short noise or burst events just before
or just after the note. A voiced consonant that cannot be heard on its own (`l`, `r`, `h`) is dropped
from a cluster. The vowels map to the formant table (`AA` a, `AE` ae, `AH` ah, `AO` o, `EH` e, `ER` er,
`IH` ih, `IY` i, `UH` oo, `UW` u) and the diphthongs `AW AY EY OW OY` glide.

**The fit** (`djengine::vocal::fit`): the syllables are spread over the bars (at most sixteen a
bar, as sixteenths); in a bar they take the strongest steps first (the beat, the half bar, the
backbeats, the off-beats: 4 syllables sit on the quarters, 8 on the eighths); a note lasts until
the next with a sixteenth of air; a phrase's last note rings on. The tune is a walk in the key: a
stressed syllable lands on a chord tone near where it is, the others step, the first note starts at
the root or the third and the last comes home to the root or the fifth; the register is A3 to
G4 for the root. It is deterministic (a seed from the text, key and the `seed` parameter; the
rhythm does not depend on it), and the fitted notes are written into the object, so a later change
to the fit cannot move a saved line. `notes=` gives your own pitches and rhythm instead.

**The object** (`djengine::vocal::VocalLine`): canonical JSON with a content ID, as a track is:

```json
{"format":1,"type":"djrust.vocal","name":"I P O, Claude is","lyrics":"I P O, Claude is on the road",
 "key":"Bb minor","bpm":124,"bars":2,"preset":"robot","seed":0,
 "notes":[{"start":0,"dur":0.75,"midi":58,"syl":"a+ih","word":"i"}, …,
          {"start":7,"dur":1,"midi":65,"syl":"ro+oo.d","word":"road"}],
 "license":"CC0-1.0","visibility":"public","author":"did:key:z…"}
```

A note may carry `pre` / `post` (cluster consonants). Rights are inside the CID (docs/LICENSING.md):
**CC0 and public by default**, the license picker (CC0 1.0, CC BY 4.0, all rights reserved) and the
visibility change it (a new CID), `fork` credits the parent (a CC BY parent's author must be
credited) and keeps the lineage, a private line is never shared. Saved as `<cid>.json` in
`<support>/vocals` (`$DJ_VOCALS_DIR`).

**Sound.** `VocalLine::render(sr)`: the voice preset (default `robot`: one saw voice, the most
intelligible; `robot-choir`, `android`, `talkbox`, the choirs, `soprano`, `bass-choir`) sings the notes through
`FormantSynth`, with the cluster events around them. One pass is **exactly `bars` bars at `bpm`
long**, so it loops on the bar: what spills over the end (a held note's release, a cluster) wraps to the start. It
peaks at about -1.4 dBFS and is deterministic. Intelligibility is robot-grade: the vowels and the
loud consonants (`s sh t k ch`) carry; `f th v` are faint.

**Where it goes.** A line is sent as a loop, kept in the capture library as a *made* loop
(`kind: "made"`, source `vocal`; the app looks for no beat in it, it names its tempo), and put on the
loop row, a deck, or into the vocoder as its modulator (the words shape a carrier: the classic
talking synth) or its carrier. `play` is the headphone cue once.

```sh
# offline
djctl vocal words "street tracks, zorbl"
djctl vocal create --lyrics "I P O, Claude is on the road" --key "Bb minor" --bpm 124 --bars 2 --out ipo.json
djctl vocal render ipo.json ipo.wav --seconds 20
djctl vocal hook house:12:extended --lyrics "I P O, I P O, Claude is on the road" --wav hook.wav --seconds 20
djctl vocal check preview.wav --bpm 124          # level, clipping, silence, the tempo heard
# in the app: djctl app vocal …  (or the MCP tool djapp_vocal)
djctl app vocal create lyrics="I P O, Claude is on the road" key="Bb minor" bpm=124 bars=2
djctl app vocal to_row play=true                 # on the loop row, looping on its boundary
djctl app vocal to_vocoder role=modulator        # or carrier
djctl app vocal hook lyrics="Six million lines of proof" deck=A   # in deck A's track key, the master tempo
djctl app vocal save name="IPO hook" license=CC0-1.0
djctl app vocal render path=/tmp/ipo.wav seconds=20
```

The Vocal window (top bar **Vocal**): type the words, pick key, voice, tempo (the master's by
default) and bars, **♪ Sing it** (fit and play on the loop row), Make, Hear on cue, the notes, Send
(loop row, deck A or B, vocoder modulator or carrier), the license and visibility pickers, Save, Fork and
the saved lines. Its content is `App::vocal_ui`, which owns no window chrome, so it can become a
centre tab (UX-REVIEW.md, option 2) as the Live window can.

**Hooks for a track.** `vocal hook` (and `djctl vocal hook SPEC`) takes the key of a generated
track and the tempo (the master's, or the one given), fits the lyrics, and puts the vocal on a line of its
own: the loop row, looping beside the decks. It is a separate line, not a net of the deck's track: the
generator's `vox` option writes its own sung part into the track.

**Limits.** English only (the lexicon and the rules; any language's sounds can be escaped in the set). One syllable a note, so a word cannot stretch over several notes
yet (a rest `_` or a longer note is the workaround). A cluster's voiced liquids are dropped. The
unknown-word rules are rough. A vocal-only clip carries no beats for the neural detector: see
docs/BEAT-DETECTION.md ("clips that know their tempo").

### Why this path (the analysis behind it)

**The constraint (from Matt, 2026-11-17).** The output must be releasable as **CC0**: hosted,
cleanly licensed, electronic or pop with robotic vocals, not realistic singing. Every component
in the lyrics pipeline must allow that, with no voice cloning and no likeness rights:

- the TTS code;
- the weights;
- the phonemizer / G2P and its dictionary;
- any voice data or speaker embedding.

### The candidates against the CC0 constraint

| component | licence | meets CC0 output? |
|---|---|---|
| **Kokoro-82M weights** (hexgrad) | Apache-2.0 | **Not cleanly.** The licence itself allows it. But the model card says the training set includes synthetic audio "from closed TTS models from large providers" (whose terms may restrict what their output trains or makes), and CC BY datasets (Koniwa `tnc`, CC BY 3.0; SIWIS, CC BY 4.0). Its 54 voicepacks are named speaker styles whose sources are not documented. That is a likeness question we cannot answer. Treat it as **conditional**: usable for drafts, not for a CC0 release until the provenance is known. |
| **kokoro** inference code (hexgrad) | Apache-2.0 (per the repo) | **Yes**, if ported: we would write our own Burn port anyway |
| **misaki** G2P (hexgrad) | Apache-2.0 (reported; confirm the LICENSE file) | **Yes**, without its fallback. Its out-of-vocabulary fallback is espeak-ng, so it must run dictionary-only, or with our own fallback rules |
| **espeak-ng** | GPL-3.0 | **No.** Not vendored, not linked, not called |
| **voice-g2p** (Rust port of misaki's English pipeline) | MIT | **Yes**, dictionary path. Its espeak-ng fallback is optional and must stay off. Check the provenance of its dictionaries |
| **CMU Pronouncing Dictionary** | BSD-2-Clause (the CMU notice) | **Not in any public build** (2026-11-21; the public repo is MIT code and CC0 content). Kept for the desktop booth only, in the private crate `djlexicon-booth`; a permissive data licence, and a phoneme sequence carries no voice. The notice is kept with the data; the audio is unaffected |
| **Our formant voice** (this doc) | MIT, our code, no data | **Yes**: no recordings, no speakers, no likeness. Its output can be CC0 |
| Piper and its voices | Not verified here. The newer piper1 is GPL-3.0, and each voice's dataset has its own licence | **Assume no** until checked voice by voice |

### Not built: Kokoro as an optional extra

The vocoder (formant voice as carrier, a rendered "speech" line as modulator) could sharpen the
consonants further. **Kokoro** stays an optional extra behind a feature, with downloaded weights and
a per-render licence flag: a render that used it would be marked "not CC0" in its provenance until
its training data and voicepacks are cleared. Open questions that remain:

1. **Kokoro's provenance.** Which closed-provider synthetic audio is in the training set, and
   under what terms? Where do the voicepacks come from?
2. **misaki's licence file** and **voice-g2p's dictionary sources** (neither may pull in espeak-ng data).
3. **A word over several notes** (melisma) and a "rap" mode where the words' own rhythm drives the part.
4. **Other languages** need their own permissive dictionaries, or are out of scope.

## Licences and sources of this work

The voice was written from published science, not code:

- Peterson & Barney (1952), JASA 24(2).
- Klatt (1980), JASA 67(3).
- Rosenberg (1971), JASA 49(2B).

The consonants, codas and diphthongs are the same textbook phonetics (Klatt 1980 for the fricatives' noise
and the voice bar). Phonemes come from the BeatScript phoneme set, lexicon and phrasebook (CC0, written here; the public-domain phrasebook quotes only texts first published before 1929) and rules written here; the desktop
booth alone adds the CMU Pronouncing Dictionary (BSD-2-Clause, notice in `crates/djlexicon-booth/data/CMUDICT-LICENSE`). Nothing is derived from espeak-ng or any other GPL code, and no audio or speaker data is used.
The engine part is in the public `djrust` crate under its MIT licence.
