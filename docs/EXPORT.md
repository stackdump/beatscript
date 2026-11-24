# Export: a BeatScript program for a DAW

A program leaves BeatScript as a **Standard MIDI File** (its notes), as **stems** (its audio, one file a part and a
deck, that add back up to the master) or as **dry stems** (each part before its deck and the master, with the moves it
leaves out written down). All come out of the render itself, so the same canonical text (the same CID) gives the same
bytes.

| Where | How |
|---|---|
| shell | `beatscript export --format midi\|stems [--dry] [--sr N] [--max-bars N] [--license L] [--credit C]... [--file F]` (bytes on stdout) |
| site | `GET /api/render/<CID>?format=midi\|stems\|stems-opus\|stems-dry\|stems-dry-opus` (the MCP `render` tool's `format`) |
| web editor | the Export for a DAW buttons: MIDI, Stems (WAV / Opus), Dry stems (WAV / Opus) |
| code | `crates/beatscript-cli/src/export.rs` (the passes, the WAVs, the manifest, the zip), `djengine::smf` (the MIDI file, General MIDI) |

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
beatscript-<CID12>/wheel.wav      (a program with `wheel` lines)
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

**The Wheel** (a program's `wheel` lines) is no deck's: the render adds it to the master after the decks, as the page
does. It is in `master.wav` and in a stem of its own, `wheel.wav` (every deck's lanes at level 0, the Wheel through the
master's reverb, crush and level as in the mix), and in no deck's or part's stem; both sum checks include it.

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

## Dry stems

Stems as they sit in the mix are what you want to rebuild the mix; for finishing in a DAW you want them **dry**: each
part as its voices make it, and the moves that shaped it written down so you can redo them with your own tools.

`beatscript export --format stems --dry`, or `format=stems-dry` / `stems-dry-opus` on the site (the same job as
`stems`: a `202` with `{state, file, done}` and `Retry-After` until the zip is made, then the zip). The zip's folder is
`beatscript-<CID12>-dry/`, so it unzips beside the wet one:

```
beatscript-<CID12>-dry/manifest.json
beatscript-<CID12>-dry/song.mid
beatscript-<CID12>-dry/master.wav          the mix as it plays (the same samples as the wet bundle's master)
beatscript-<CID12>-dry/A-kick.wav  A-bass.wav  B-melody.wav ...
beatscript-<CID12>-dry/wheel.wav         a program that plays the Wheel: the Wheel alone, before the master
```

No deck stems: a dry deck would be the sum of its dry parts. The Wheel is an instrument: its own lines (`wheel play`,
`wheel sequence ...`) are kept in its dry stem, and only the master's reach it in the mix.

**How a dry stem is made.** One render a part, every other part's lanes at level 0 as for a wet stem, but:

- **no control move but a deck's start and stop** (`Controls::keep_transport`): no level, EQ, crossfader or master
  level, no insert (`filter`, `resonance`, `reverb`, `delay`, `crush`) or its ramp, no trick, no macro, no `duck`
  line, no DJ `move`. The sections still load and start the decks, so the part plays every note it plays in the mix
  (`played` is the wet export's, a test pins it);
- **the deck's strip at unity**: volume 1, trim 1, the crossfader full on the part's side, the master level 1;
- **the deck's limiter off** and **its room and echo sends at 0** (`set_unit_sends`).

What stays is the voice: the instrument, its envelope and its own filter, the notes' velocities, and a duck depth the
program gives its voices (a kick dipping the bass inside the deck's engine). The test exports the dubstep example
twice, the second time with its master and deck lines changed (lowpass made highpass, other levels, more reverb): the
dry stems are the same samples, the wet ones are not.

**The manifest** says `"stems":{"dry":true, ...}` (no sum check: dry parts do not add up to the master, by design) and
carries what was left out:

- `stems.master_fx_lines`: the master's lines, each once;
- each part's `automation`: its deck's lines;
- `master_automation`: the master's lines; `other_automation`: the rest (the crossfader, a line on no one deck).

One entry a line of the program (a line a `repeat` or a `def` copies is one entry, all its fires together):

```json
{"line":88,"text":"deck A: resonance 0.2 -> 0.4 over 8 bars","scope":"deck","deck":"A",
 "fires":[176],"hold_bars":null,
 "series":[{"target":"A","param":"filter.res","points":[[176,0.2],[176.25,0.2025], … ,[208,0.4]]}],
 "events":[{"at":176,"bar":45,"beat":1,"target":"A","param":"filter on","value":1,"move":"insert A filter on"}],
 "note":null}
```

`fires` and `at` are beats from the start; `bar` and `beat` are 1-based. `series` is one knob's values over time, the
values the render sent the engine (a ramp sampled as the runtime samples it), as `[at, value]` pairs: a level is
`volume` 0…1, the master's `level`, an insert knob its own (`filter.sweep`, `filter.res`, `reverb.mix`). `events` are
the switches: an insert on or off, a mute, a trick, a macro fired or let go. A line with no move on the render's
timeline the dry pass takes out (a DJ `move`) has empty `series` and `events` and says so in `note`.

## Determinism

Nothing in the files has a clock: the zip's entries are dated 1980-01-01, stored in a fixed order (manifest, MIDI,
master, decks, parts), the manifest's keys in a fixed order, the MIDI's events sorted by tick with note-offs first.
`crates/beatscript-cli/tests/export.rs` exports twice and compares the bytes.

## Not in it

- **A tempo curve in the audio.** The site's render plays a program at its opening tempo; the MIDI file has the
  curve (the manifest's `tempo_curve` and its notes say so).
- **Control lines the render does not hear** (see the render's own notes): not in the stems either. The render plays
  a program's control lines as the browser's engine plays them (crates/beatscript-cli/RENDER.md), so the stems are what
  the page plays (in the dry manifest a line it does not hear is listed with the rest, with a `note`).
- **Samples and loops** (`sample` / `loop "..." as`) and restricted programs are not rendered, so not exported.
