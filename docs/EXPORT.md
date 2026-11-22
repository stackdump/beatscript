# Export: a BeatScript program for a DAW

A program leaves BeatScript as a **Standard MIDI File** (its notes) or as **stems** (its audio, one file a part and a
deck, that add back up to the master). Both come out of the render itself, so the same canonical text (the same
CID) gives the same bytes.

| Where | How |
|---|---|
| shell | `btwd export --format midi\|stems [--sr N] [--max-bars N] [--license L] [--credit C]... [--file F]` (bytes on stdout) |
| site | `GET /api/render/<CID>?format=midi\|stems\|stems-opus` (the MCP `render` tool's `format`) |
| web editor | the Export buttons: MIDI, Stems |
| code | `crates/btwd/src/export.rs` (the passes, the WAVs, the manifest, the zip), `djengine::smf` (the MIDI file, General MIDI) |

## The MIDI file

Type 1, 480 ticks a quarter (a sixteenth step is 120 ticks).

- **Track 0, the conductor:** the song's name, a copyright line from the stored rights (`License: CC0-1.0`, the
  author first when there is one), 4/4, the key signature, the tempo map, and a marker at the first bar of every
  section the arrangement plays (`intro`, `drop`, …).
- **The tempo map:** one tempo for a program with one; a tempo that moves (`tempo 120 -> 140 over 8 bars`, a
  section's own `tempo`, `at bar 16: tempo ...`) is the staircase the desktop render plays (`TempoMap::staircase`,
  a step every sixteenth of a bar, each the constant tempo that takes the curve's own seconds).
- **One track per part** (deck x role, named `A kick`, `B arp`) that plays a note, in the program's order. The notes
  are the engine's: every hit the render strikes (`FireEvent`), on its sixteenth step, with the velocity it played
  (a `vel=70..95` part's seeded picks, the same values every time), and its length. A part's lines in every section
  are one track; a note struck again on its own key cuts the one before.
- **Key signature:** the key's, by the circle of fifths; a mode with a flat third (minor, dorian, phrygian, …) is
  written minor with its relative major's signature, the others major.

### Drums: channel 10, General MIDI

Every drum part (a part whose hits the engine plays on a drum lane) is on channel 10. The engine already plays GM
percussion notes, and they are kept as they are:

| Role | GM note |
|---|---|
| kick | 36 Bass Drum 1 |
| snare | 38 Acoustic Snare |
| clap | 39 Hand Clap |
| hihat | 42 Closed Hi-Hat (46 Open Hi-Hat where the part opens it) |
| toms, cymbals | their GM notes (35…81) as the engine plays them |

A drum hit outside 35…81 (a drum part written with a pitch) is written as its role's note above (another role: 38).

### Program changes

The other parts get channels 1…16 (10 skipped) in order, and a program change at bar 1 where the part's `inst=` has
a General MIDI match (`djengine::smf::gm_program`): `piano` 1, `electric-piano` 5, `organ` 17, `bass` 34,
`sub-bass` / `808-bass` / `fm-bass` 39 (Synth Bass 1), `strings` 49, `choir` 53, `pluck`s 46, `trumpet` 57,
`brass` and the stabs 62, `sax` 66, `flute` 74, `square-lead` / `chiptune` 81, the saw leads 82, the distorted leads
85, `warm-pad` / `pad` 90, `glass-pad` 93, `dark-pad` 95, `sitar` 105, `kalimba` 109, `steel-drum` 115 (numbers are
1-based, as a DAW shows them). A voice named with a family word (`reese-bass`, `warm-strings`) takes the family's.
No match (a `laser`, a `noise-hit`, a talkbox) and no `inst=`: no program change, the DAW's default sound.

## Stems

A zip:

```
beatscript-<CID12>/manifest.json
beatscript-<CID12>/song.mid
beatscript-<CID12>/master.wav
beatscript-<CID12>/deck-A.wav  deck-B.wav
beatscript-<CID12>/A-kick.wav  A-hihat.wav  A-bass.wav  B-arp.wav ...
```

WAV is 32-bit float stereo at the render's rate (44.1 kHz; `stems-opus` is the same bundle with every stem as Ogg Opus
at 48 kHz, as the site's Opus render). Float, so a master that peaks over full scale is kept whole rather than clipped,
and the stems still add up to it.

**How a stem is made.** One render a part (and one a deck), with every other part's lanes at level 0
(`Lane::set_base_level`): the muted lanes still run, so a kick that is not heard still ducks the bass, an envelope
still drives what it modulates, the room and the echo still get what the part sends them. Each deck ends in a
true-peak limiter, the one nonlinear stage of the path: the master pass records the gain it gave every sample
(`djrust::fx::GainTap::Record`) and every part's pass replays that gain (`Replay`). So a stem is its part **as it sat
in the mix**: the limiter's movement, the deck's strip, and **the master's effects (a `master: filter ...`) are
applied per stem**, as they are to the mix. The manifest lists the program's master lines.

**The sum check.** After the passes the exporter adds the stems up, sample by sample, from the files, against the
master, and writes the result into the manifest (`parts_sum_check`, `decks_sum_check`: peak, largest difference,
RMS, both in dB under the peak). The parts add up to the master to float rounding (about -140 dB); a sample the
mix's limiter had to clamp at its ceiling is the one place they can differ.

**The master** in the bundle is the site's render of the program, sample for sample (a test pins it).

### manifest.json

```json
{"generator":"BeatScript","cid":"z…","name":"trance-138","tempo":138,"tempo_curve":false,"key":"am",
 "time_signature":"4/4","bars":64,"seconds":111.304,"sample_rate":44100,"bits":32,
 "license":"CC0-1.0","author":null,"forked_from":null,"credits":[],"decks":["A","B"],
 "parts":[{"deck":"A","role":"kick","file":"A-kick.wav","midi_track":"A kick","midi_channel":10,
           "drums":true,"instrument":null,"gm_program":null,"notes":192}, …],
 "files":[{"path":"master.wav","kind":"master"}, …],
 "stems":{"method":"…","master_fx":"applied per stem: …","master_fx_lines":["master: filter lp 400 -> 16k over 16 bars", …],
          "parts_sum_check":{"peak":1.956,"max_abs":1.2e-6,"rms":7.8e-8,"max_abs_db":-124.2,"rms_db":-148.0},
          "decks_sum_check":{…}},
 "notes":[…]}
```

The license, author, credits and forked_from are the stored rights (`share` stores them; CC0-1.0 by default).

## Determinism

Nothing in the files has a clock: the zip's entries are dated 1980-01-01, stored in a fixed order (manifest, MIDI,
master, decks, parts), the manifest's keys in a fixed order, the MIDI's events sorted by tick with note-offs first.
`crates/btwd/tests/export.rs` exports twice and compares the bytes.

## Not in it

- **A tempo curve in the audio.** The site's render plays a program at its opening tempo; the MIDI file has the
  curve (the manifest's `tempo_curve` and its notes say so).
- **Control lines the render does not hear** (see the render's own notes): not in the stems either.
- **Samples and loops** (`sample` / `loop "..." as`) and restricted programs are not rendered, so not exported.
