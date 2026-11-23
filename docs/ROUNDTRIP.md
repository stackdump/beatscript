# ROUNDTRIP: audio -> BeatScript program

Goal: take a rendered BeatScript track (audio) and recover a program for which
`check same_as <original>` is true, that is, a program that compiles to the same nets as the
original. If the round trip closes on programs we wrote ourselves, we can trust the same machinery
on audio we did not write.

Status: plan, plus rung 1 **closed**: `crates/beatscript-round` (branch `feat/btw-roundtrip`) recovers first-beat from
its audio alone, sample identical, `same_as` true, and with the original's own program CID (section 7).
Written 2026-11-21. Everything here is CC0-compatible: public or permissive sources only, no
Demucs weights bundled, no GPL code (openDJ is GPL-2.0: ideas only).

## 1. The idea: analysis by synthesis

Most "audio to score" work has to guess at an unknown instrument and an unknown renderer. We have
neither problem.

- The **renderer is known and deterministic**: `beatscript render` (`djhost::deck::offline`, blocks of 128
  frames, no threads, no randomness; chance in the language is a hash of text and bar).
- The **instrument set is known and finite**: `beatscript instruments`.
- The **language is total and small**: one line per deck, 16 steps a bar, `let` / `def` / `repeat`,
  sections and an arrange, no recursion.
- The **forward model is free to call**, locally, as many times as we like. The public `render` MCP
  tool is rate limited to about 30 an hour, so the local `beatscript` is the search engine and the public
  tool is only the cross-check. Measured 2026-11-21: `beatscript render` is **bit-identical** to the server's
  r2 render, on first-beat (682,839 frames, 0 samples differ) and on acid-line (1,282,909 frames, 0
  differ, control lines included). L0 against a downloaded server render is real.

So inversion is a search over programs scored by re-rendering, and transcription is only a way to
propose good candidates. A transcriber that is wrong 5% of the time is fine, because the render
and the comparison catch it. This is the opposite of the usual order, and it is why we build the
scoring loop first and the fancy transcription last.

```
audio --> grid (bpm, bar 1) --> per-slot decomposition against rendered templates
      --> program lines --> pattern induction (lets, e(k,n,rot), repeat, sections, arrange)
      --> candidate program --> beatscript render --> compare with the audio --> residual
      --> (residual says where to look next) ... --> check same_as original
```

### The score, three levels, strictest first

| level | test | meaning |
|---|---|---|
| L0 sample | recovered render == input PCM, bit for bit (or within 1 LSB of s16 rounding) | exact; only possible when the input is a `beatscript` render |
| L1 net | `beatscript check --same-as original` -> `same: true` | the same song by compiled nets; text, lets, defs, section names may differ |
| L2 music | `same_as.diff.summary` | when L1 is false, what differs in music: "drop bass: 1 note changed (1:7 c3 -> d3)". This is the progress meter and the failure report. |

For audio that is not a `beatscript` render (a browser capture, an Opus transcode, a stem), L0 is
unavailable and the score is a perceptual distance (multi-resolution log-STFT, onset F-measure,
chroma distance) between input and re-render, plus L1/L2 when the ground-truth program is known.
Ground truth is always the program text, because we wrote the fixtures.

## 2. What exists and what is missing (survey 2026-11-21)

Everything in the maintainer's workspace; BeatScript work is spread over worktrees
because `main` of the engine repo is behind its feature branches.

### Exists

| piece | where | notes |
|---|---|---|
| BeatScript engine (parser, compiler, nets) | the BeatScript repo (private then, stackdump/beatscript now) `crates/beatscript-api`, `crates/beatscript-cli`, `djlive` (`ast`, `compile`); branch `feat/btw-same-as` and its merge worktrees | `beatscript check [--same-as]`, `diff`, `nets`, `fmt`, `instruments`, `render`. Not on the main checkout's `main` yet. |
| Deterministic offline renderer | `beatscript render` (`crates/beatscript-cli/src/render.rs`), `djhost::deck::offline` | raw PCM on stdout, events on stderr. Renders control lines since 2026-11-19 (see section 6). |
| Server (beatscript-serve), MCP `check`/`render`/`list_examples` | Rust: `crates/beatscript-serve`; Go front: `beats-bitwrap-io` `internal/btwsite` (worktree `btwserve-ref`) | the public path; rate limited |
| MIDI and stem export | `origin/feat/midi-stems`: `beatscript export` (cc1fabc), `beatscript-serve` `render?format=midi|stems|stems-opus` (cbe7ac8); the beatscript.app Export button is `beats-bitwrap-io` 088e8ed | **ground truth note events**: use it to score a transcriber's notes directly, never as an input to recovery |
| listen / state report | `feat/listen-state`, sibling run `bs-listen-ci` | the state of a browser tab, not audio analysis; not needed for this work |
| Beat, downbeat, tempo | public `djrust` `src/musicality/beat_tracker.rs` (onset flux, autocorrelation tempo, Ellis DP beats, downbeat by low-band phase); `docs/BEAT-DETECTION.md` Beat This! (`beat-this` crate 1.1.0, MIT, model weights MIT, ~11 MB fetched on first use), DSP fallback | licences checked 2026-11-17 in that doc |
| Key detection | `djrust` `src/musicality/loop_key.rs` (Krumhansl-Kessler chroma correlation, confidence); `djengine::key` (Camelot) | so key detection is **not** missing, it is a 12-bin guess: fine for a hint, not for transcription |
| Pitch of a held note | `djrust` `src/musicality/pitch_measure.rs` (YIN, 30 Hz to 2 kHz), `voices::pitch` | monophonic only |
| Spectrum, FFT, loudness, energy curve, section segmenter | `djrust` `src/musicality/{fft,spectrum,energy_curve,section_segmenter,loudness}.rs` | reusable as is |
| Euclidean and Markov generators | `djrust` `src/gen` | the same Bjorklund the language uses: the inverse fit (steps -> `e(k,n,rot)`) must agree with it |
| Stem separation | `demucs-rs` (stackdump/demucs-rs; branch `feat/htdemucs-ort` then): `demucs-rs separate/batch`, htdemucs via `ort`, 4 stems, CPU, model downloaded and sha256-pinned to `~/.cache`, never committed | only needed for audio that is **not** a clean render; for our own renders the decks are separable by synthesis (section 3) |

### Missing (after rung 1)

1. Anything beyond one repeating bar: sections, `let`/`def`/`repeat`, arranges, `evolve`, generators and chance (rungs 3 and 4).
2. More instrument identification than a short list of candidate voices (rung 1 tries 3 bass voices and 3 pads; the template bank is cheap to widen, the cost is renders).
3. Tonal parts that are not one note a step (`melody`, `arp`, chords other than a triad on the pad; polyphony).
4. Accents (`!`) and slides (`^`) as per-note flags, `len=`, `swing`, `key`, tempo curves (rung 2).
5. Control-line inference from audio (the controls *are* in the render, section 6, so this is analysis, not a ceiling).
6. Audio that is not a `beatscript` render: resampling, transcodes, browser captures (rung 6): L0 is lost; the beat tracker and stems (`demucs-rs`) come in here.
7. Deck assignment is unobservable from the sum of the decks (they add exactly, section 3); rung 1 uses the convention drums on A, tonal on B. Wrong guesses show up only through interactions inside a deck.

## 3. Why synthesis beats transcription here (and what rung 1 taught)

| subproblem | the usual approach | what we do instead |
|---|---|---|
| tempo and bar 1 | beat tracker, then hope | a `beatscript` render starts at frame 0 and is exactly `bars * 240 / bpm` seconds, to the frame. That fixes `bars / bpm = 1/15.5` for first-beat, but not which: 62 x 4, 93 x 6, 124 x 8 all fit. The onsets pick: the right 16th lattice catches onset mass that the same lattice shifted half a step does not; ties go to the coarsest lattice. |
| drum hit classification | trained classifier | each kit piece is rendered once into a template and matched in a band-power feature space (below) |
| pitch and polyphony | pitch tracker, chord recogniser | render every candidate pitch of every candidate voice; the pursuit picks. Chords are a small dictionary (24 triads), not open polyphony |
| instrument identity | embedding classifier | try each voice in a short list; keep the one whose templates explain the most |
| velocity | amplitude heuristics | a range `vel=lo..hi` gives each hit a velocity from a hash (`fnv("{deck}:{role}:{bar}:{step}:{note}") % (hi-lo+1)`); fitted gains give the *shape*, every (lo, hi) is ranked by that shape for free (the hash is replicated in `vel.rs`, pinned to the compiler by a test), and a real render picks the winner. It found `vel=60..90` exactly. |
| pattern structure | none, usually | the language **is** the structure: equal bars become one `repeat` or one `let`; a hit row that is Bjorklund(k, n) rotated becomes `e(k,n,rot)` (not needed yet) |

What the first rung measured, each of which changed the design:

- **Decks add exactly** (render of A+B differs from the full mix by at most 1 LSB), so the mix splits by deck for free. Parts *within* a deck are **not** additive (a deck-level bus: fitted part gains were 0.86 / 0.32 / 0.75 and left 20% residual). A template sum is a proposal, never a verdict.
- **Noise voices draw different noise for every hit** (hihat, snare): a hit at another step is a different waveform, so waveform correlation cannot propose hats. Band powers can.
- **Kick and bass interfere by phase** and share the low end; judging one hit at a time against a residual that still holds the other is confounded (a correct kick *raised* the error). Fix: orthogonal matching pursuit, all gains refitted together, in a phase-blind feature (band powers of a 4096-point spectrum, one band per bin from 32 Hz to 400 Hz so a bass note is resolved to a semitone, log bands above, every band weighted to count alike).
- **Retriggering** (a kick's long tail is cut by the next kick) makes a lone added event look wrong until its neighbours exist. Hence the climb scores whole programs by a real render, and proposes drum events also as sets.
- **An event lands on the next whole frame**: `ceil(step * bar_frames / 16)` (5334.68 -> 5335, 16004.03 -> 16005).
- **Velocity is a sample question only after the events are right.** Reading velocities per hit window while other parts still had the wrong velocity let the parts compensate (a kick read as 127). The shape-from-gains plus whole-program render fixes that.
- **Cost**: 187 single-event renders for the template bank (4 drums, 3 bass voices x 37 pitches, 3 pads x 24 triads), a few hundred probe renders for the climb and velocity stages; 3.5 minutes on a loaded box (load 30), 3 render processes. Every probe is 2 bars; only the end is a full render.

Because `same_as` compares **nets**, not text, the recovered program does not need the original's
spelling. A `let` refactor, a renamed section, `x*8` against eight `x`'s are all the same song.
The induction step only needs to find *a* program with those nets, so the cheapest one (everything
written longhand, one section) is a valid answer for L1; compression (lets, repeats, euclid) is a
separate, later goal scored by program length.

## 4. The ladder

Each rung adds one thing to prove. A rung is done when its fixtures reach L1 `same: true` and, for
`beatscript` renders, L0.

| rung | fixture | what it adds | expected hard part |
|---|---|---|---|
| 1 | `first-beat` (CID `z4EBG9jFzzkVrdxGXZhj7EUYYSYzLhhZJ4rGkQXpuGm7KeDA7Ya`): 8 bars, tempo 124, 3 drum voices on deck A, bass and a pad chord on deck B | grid, template bank, drums by NNLS, bass pitch, one chord, `vel=60..90` (velocity from a hash of place) | the hihat velocity range: the net records a range and a hash, not 8 numbers; and the pad's tail overlapping the next bar |
| 2 | `acid-line` (`z4EBG9j3EdRodQCNYE84djfsVgKs4XeSvKQDyXhLBzRuLJuXmtq`): FM bass with accent `!` and slide `^`, kick, hats, filter cutoff and resonance ramps over 16 bars | accents and slides as per-note flags, monophonic legato, **first control lines** | ramps are control nets: needs the fidelity policy of section 6 |
| 3 | `evolve-demo`, `markers-demo`, `trance-138` | sections, `let`/`def`, `repeat`, `evolve` (a program that changes itself: induction must find the generation rule or settle for longhand) | recognising equal bars and the arrange |
| 4 | Deep Hours `z4EBG9j7sAyRKuuVhha6DACwUEvCeuLZPRC1fhj9kU73EwNFzTW` (the track that ends) and its loop fork `z4EBG9j288TZehcWFMcZrtyFnKE1TPuRxizk5D2Rfg9TS9vYo2x` | a real multi-section track; the fork is a free **diff test**: the recovered programs of the two must differ exactly where the programs do | scale (bars, decks), markov and seeded chance (`maybe`, `?0.45`) which must be re-derived from the text hash, not fitted |
| 5 | `matrix-130`, `hand-dubstep-140-v2/v4` | wobble basses, risers, 9 sections | control-heavy |
| 6 | browser capture and Opus transcodes of the above | the first audio that is not a `beatscript` render: L0 is lost, stems may help | separation, perceptual score |

Chance and generators (`markov`, `maybe`, `degrade`, `?p`) are deterministic functions of the part's
text and the bar, so a recovered program that *contains* `markov(am, 0.7)` reproduces the notes by
construction, but fitting the notes back to a `markov(...)` call is a hash inversion. Policy: write
generated parts longhand as notes (valid for L1 only if the original's nets are note-level, which
they are: a generator compiles to its notes), and treat finding the generator as compression.

## 5. Components (Rust, `crates/beatscript-round`, a new crate: no existing crate changed)

`beatscript-api` is used in process (`check`, `nets`, `same_as`); `beatscript render` is a subprocess, because `beatscript` is a
bin crate and rung 1 must not touch it. (Exposing its render as a library function would remove the process
spawn per probe; that is a change to an existing crate, so it waits.)

| module | job |
|---|---|
| `wav` | read WAV (s16 or f32; a streaming header with a bogus data length is fine), write WAV |
| `beatscript` | `render` (subprocess), `check`, `part_velocities`, `same_as` (in process) |
| `model` | the slice of the language a recovery is written in (drums on A; a bass and a triad pad on B; one bar, repeated): `State` -> program text |
| `spec` | the phase-blind feature: band powers of a Hann-windowed 4096-point FFT, hop 512 |
| `vel` | the `vel=lo..hi` hash, pinned to the compiler's nets by a test |
| `recover` | grid; template bank; orthogonal matching pursuit; render-scored hill climb; velocity fit |
| `score` | L0 (sample diff) |
| `bin/beatscript-round` | `beatscript-round recover in.wav [--ref program.beatscript] [--out out.beatscript] [--beatscript PATH] [--jobs N] [-v]`; `beatscript-round render prog.beatscript out.wav` |

Tests: unit tests for every pure piece (`cargo test -p beatscript-round`); the acceptance test
`tests/first_beat.rs` (`#[ignore]`, needs a built `beatscript`, minutes) renders first-beat, recovers, and
asserts L0 identical, `same_as` true and an equal program CID. `fixtures/` holds the two CC0 example programs.

## 6. The fidelity ceiling: control lines (measured, and not where the task statement put it)

The task statement said the offline render plays notes and sounds but not control lines. Measured on
2026-11-21 with acid-line (`master: filter lp 300 -> 3200 over 16 bars`, `master: resonance 0.2 -> 0.7`):

| comparison | result |
|---|---|
| server `render` (r2) vs local `beatscript render` of the program | **bit-identical**, 1,282,909 frames, 0 samples differ |
| server render vs local render with the two `master:` lines deleted | 31,477 max difference; 0.02% of samples equal |

So the public render **does** play at least master filter and resonance ramps (the `render` tool's note
saying otherwise is out of date for r2). The `beatscript render` documentation (`crates/beatscript-cli/RENDER.md`) lists what
is rendered (deck and master level, EQ, mute, crossfader, ducking, ramps and lfos, filter/reverb/delay/crush
inserts, timed lines) and what is not: a `do` call to another noun, vocals, most moves, tempo curves. The
start event's `notes` array names what a given program leaves out (empty for both fixtures).

Consequences:

1. Controls are **in the audio**, so recovering them is analysis, not a ceiling. Rung 2 does it by
   synthesis: recover the notes, render them without the control lines, divide the input spectrum by that
   frame by frame, and the ratio *is* the filter/reverb trajectory; fit it to the language's vocabulary (a ramp
   `filter lp A -> B over N bars`, an lfo, a step) with a small parametric search, then verify by re-render.
   Start with one control (the master low-pass cutoff of acid-line).
2. The real ceiling is the list `beatscript` itself skips (`do` calls to other nouns, vocals, samples and
   `restricted` programs, which the renderer refuses, tempo curves). Audio made with those is unrecoverable
   *by this method*, and the report must name which part of the program it could not hear.
3. Audio from the **browser engine** (a capture) can contain more than `beatscript` renders; for that the
   claim drops to a perceptual score (rung 6).
4. **Never invent.** If a control cannot be fitted below a residual threshold, the recovered program omits
   it and the report lists the residual's band and bars.
5. Score notes and controls separately (`same_as` already reports per-deck `diffs`), so a control-bearing
   fixture can read "notes: same; controls: not yet attempted", which is true and not a failure.

## 7. Rung 1: first-beat, closed

Input: first-beat's audio, downloaded from the public server (`render?format=wav`, r2, CID
`z4EBG9jFzzkVrdxGXZhj7EUYYSYzLhhZJ4rGkQXpuGm7KeDA7Ya`). The recovery sees only the samples. The original
program is passed to `--ref` for scoring and is never read by the recovery.

```
beatscript-round recover first-beat.wav --ref first-beat.beatscript --beatscript target/release/beatscript --jobs 3 -v
```

Recovered (3 min 38 s on a shared Linux box at load 30, 3 render processes):

```
#! btw 1
tempo 124
deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | snare ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ vel=60..90
deck B: bass a2 ~ ~ a2 ~ ~ a2 ~ ~ ~ c3 ~ e2 ~ ~ ~ inst=sub-bass | harmony chord(am) inst=warm-pad
end at bar 8
```

| level | result |
|---|---|
| L0 | **sample identical** to the server render: 682,839 frames, max difference 0, 0 samples differ |
| L1, local `beatscript check --same-as` | `same: true` |
| L1, public BeatScript MCP `check` with `same_as` = first-beat's CID | `same: true`, `text_same: true`, `sections_same`, `song_nets_same`, no diffs |
| program CID | the recovered text has the original's own CID, `z4EBG9jFzzkVrdxGXZhj7EUYYSYzLhhZJ4rGkQXpuGm7KeDA7Ya` |

What the pipeline did, in order (from the `-v` log): the length fixes bars/bpm, the onsets pick 124 x 8
over 62 x 4, 93 x 6 and the rest of the family; 187 templates (4 drums, 3 bass voices x 37 pitches, 3 pad voices
x 24 triads) were rendered; the pursuit found, in one pass of 15 columns, all four kicks, both
snares, all four hats and all five bass notes with the right pitches and the right voice (`sub-bass`), leaving
the pad; the climb added `chord(am)` on `warm-pad` once the loud parts were right (spectral error 3.7 -> 0); the
velocity stage turned the hat's fitted gains [0.445, 0.509, 0.558, 0.610] into `vel=60..90` (sample error
4.8e11 -> 3.6e9 with the pad missing, -> 0 with it) and left kick, snare and bass at the default.

Honest limits of this result, so it is not read as more than it is:

- The instrument search was over a **short list** (3 bass voices, 3 pad voices), the default drum kit, a
  triad pad, bass notes in MIDI 28..64. The truth was inside that space. The method scales with the size of
  the bank, which costs renders; it does not yet find a voice outside the list.
- The pattern model is **one bar repeated** (`end at bar N`), the shape of first-beat. Nothing here yet
  recovers sections, repeats, lets, evolve or per-bar variation.
- Deck assignment is a convention (drums A, tonal B); it is not observable from audio (section 3).
- It is one fixture. The claim is "rung 1 closes", not "round trips work".
- First-beat has no control lines; section 6 covers what is known about those.

Reproduce: build `beatscript` and `beatscript-round` (`cargo build --release -p beatscript-cli -p beatscript-round` in this repo
on `feat/btw-roundtrip`), then `cargo test --release -p beatscript-round -- --ignored --nocapture`.

## 8. Later rung: provenance, "was this track generated by our stack?"

Plan only, not built. It reuses the recovery machinery; it adds a decision and a fast path.

### Tiers

| tier | test | verdict |
|---|---|---|
| 1 exact | the recovered program re-renders sample-identical (L0) **and** its CID is in the store | proof of reproducibility, plus the stored record's lineage and signatures |
| 2 near | transcoded, trimmed, gained, re-encoded or remixed audio: score by re-render similarity (multi-resolution spectral distance, onset F, chroma) with a calibrated confidence threshold | likely derived from program P at similarity s, with the explained fraction reported |
| 3 negative | nothing in the language reproduces it above the null threshold | not reproducible by our stack: say so, with the best partial |

### Cheap fast path: fingerprint index

Index an audio fingerprint of **every stored render** (landmark or chroma-hash style, built from
`beatscript` renders at store time, content-addressed by program CID). A query first looks up the
fingerprint; a hit on a long enough aligned run names the candidate CID, which is then verified by
re-render (tiers 1/2), so a known track never needs the round trip. The round trip is for audio
that never hit the server: local and private programs, and post-export edits. The index is derived
data (rebuildable from the store), so it lives beside the store, not in it.

### Confidence scales with specificity

A four-on-the-floor kick matches millions of programs. The confidence reported must grow with how
much of the track the program explains **and how unlikely that explanation is**:

- *explained fraction*: the share of spectral energy (and onsets) the recovered program accounts
  for;
- *specificity*: description length of the program under the language's own prior (bits to specify
  its notes, voices, sections), so a one-line kick pattern is a few bits and a 9-section track is
  thousands;
- *null calibration*: the similarity a random or foreign track reaches against the same candidate
  set, measured, not assumed; the threshold is a quantile of that null at a stated false-positive
  rate;
- a floor on specificity below which the answer is "consistent with, not evidence of".

### What it does and does not prove

It proves **reproducibility**: this audio can be produced by this program on this stack. It does not
prove **authorship**. Authorship stays with signatures (the program's signature and the account that
stored it). A trivial pattern can be reproduced by anyone; the tier and the specificity say how much
that means.

### Order of work

First-beat rung first (this document's section 7), then the ladder in section 4. Provenance follows
rung 2 at the earliest: tier 1 needs only L0 plus a store lookup, so it is the first provenance
milestone and costs almost nothing once L0 works; the fingerprint index and the null calibration
come after.

## 9. Rules and risks

- **Licences.** Public or permissive sources only. Beat This! and its weights: MIT. rten, rubato:
  MIT/Apache. Demucs: the htdemucs weights are **never** bundled or committed (`demucs-rs` fetches
  and sha256-pins into a user cache); recovery of our own renders does not need them. No GPL code
  (openDJ ideas only). Fixtures are CC0 programs and their CC0 renders. Outputs are CC0-compatible.
- **Memory.** The ops host ran out of memory today; build and search jobs run niced, `-j` limited, and
  the render is in process (no 30-process fan-out). Bank building is cached by CID.
- **Rate limits.** The public `render` is about 30 an hour: used only to fetch fixtures and one
  parity check, never in the search loop.
- **Divergence.** Local `beatscript` and the server's r2 render were bit-identical on both fixtures tried
  (first-beat, acid-line). If a future server version differs, L0 against a downloaded server render is
  impossible and the claim drops to L1 plus a spectral bound; the report must say which renderer produced
  each side, and the render version (`v=r2`) belongs in the record.
- **Branch hygiene.** All work on `feat/btw-roundtrip` (off `feat/btw-same-as`); nothing pushed to
  `main`. The change is self-contained: a new crate (`crates/beatscript-round`), one line in the workspace
  `members`, its `Cargo.lock` entry, and this document. No existing crate's behaviour changes.
