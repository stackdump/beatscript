# Polish a mix

`listen` hears a program as numbers: it renders the program offline through the same engine as `render` and measures
the result. This page is the loop for polishing a mix with it, by ear but without audio: listen, read the warnings, change
one line, and listen again with `compare` to see what the change did. It is written for a model working over MCP, and it
works the same from the command line (`beatscript listen --file new.beatscript --compare old.beatscript`).

## The loop

1. `check` the program until `ok` is true.
2. `listen` to it. Read `warnings` from the top: they come in the order to fix them (below: not heard, clipping, clashes,
   balance, loudness and the ceiling, width), and each one names the BeatScript line that fixes it. `warning_ids` gives
   each warning a stable id, side by side.
3. Change **one** thing: the line the first warning names. One change at a time keeps the cause of every delta clear.
4. `check`, then `listen` again with `compare` set to the previous version (its CID, or its text). The answer gains
   `compare`: a one-line `summary`, the deltas (now minus before) of the master, each part and each section, the
   `targets` before and after, and the warnings `fixed`, `introduced` and `kept`.
5. Keep a change that fixed its warning and introduced nothing worse; undo one that did not. Repeat until `warnings` is
   empty (or holds only what you mean to keep) and `targets.met` is true.
6. Then play it (`eval`) and listen with ears. The numbers find problems; they do not judge taste.

Both listens of a compare are cached by CID, section, bars and targets, so going back to a version you have heard is free.
Use `section` and `bars` to work on one stretch: a compare uses the same window for both versions. On a long program, work
a section at a time: a deck of several parts is measured part by part on renders of its own, within a budget, and `notes`
says when some parts were past it (they have no bands, and take no part in `clashes`). A section of a few bars fits them all.

```json
{"tool": "listen", "arguments": {"cid": "z...new", "compare": "z...old", "targets": "streaming"}}
```

### A worked example

The `first-beat` example (eight bars: a kick, snare and hats on deck A, a sub bass and a pad on deck B). Its listen,
trimmed:

```json
{"master": {"lufs": -12.7, "true_peak_db": -0.2, "crest_db": 13.8, "plr_db": 12.5, "lra": 0.6,
            "bands": {"sub": -24.1, "low": -14.6, "low_mid": -32.7, "mid": -43, "high_mid": -35, "high": -33.9},
            "centroid_hz": 231, "width": 0, "correlation": 1},
 "clashes": [{"parts": ["B.bass", "A.kick"], "band": "low", "hz": "60-250 Hz", "bars": [[1, 8]], "overlap": 0.54, "gap_db": 1.9}],
 "targets": {"name": "streaming", "lufs": -14, "true_peak_db": -1, "lufs_delta": 1.3, "true_peak_delta": 0.8, "met": false},
 "warnings": [
   "deck B bass and deck A kick clash at 60-250 Hz (low) in bars 1-8: `duck B under A.kick 6db`, or `mix { B eq low -6 }` (on all of deck B)",
   "deck B harmony is 26 LU under the mix and barely heard: raise its `vel=`",
   "the master's true peak is -0.2 dBTP, over the streaming ceiling of -1 dBTP: take 1.3 dB off (`master: level 0.86`), which also brings it to the target",
   "the master is -12.7 LUFS, 1.3 LU over the streaming target of -14 LUFS, which turns it down by as much: the true peak's `master: level 0.86` fixes both"],
 "warning_ids": ["clash:A.kick/B.bass:low", "buried:B.harmony", "true_peak", "loudness:high"]}
```

Nothing is silent and nothing clips, so the clash is first. Add `duck B under A.kick 6db` and listen with `compare` set to
the first version:

```json
{"compare": {"summary": "master -12.7 -> -13.1 LUFS (-0.4); true peak -0.2 -> -0.3 dBTP; 2 warnings fixed, 0 introduced, 2 kept",
             "parts": [{"deck": "B", "part": "bass", "lufs": -0.6, "bands": {"sub": -2.7, "low": -0.6}}],
             "fixed": ["deck B bass and deck A kick clash at 60-250 Hz (low) in bars 1-8: ...",
                       "the master is -12.7 LUFS, 1.3 LU over the streaming target of -14 LUFS, ..."],
             "introduced": []}}
```

The duck cleared the clash, and taking the bass down on every kick brought the master within a LU of the target too.
Next come the pad (raise its `vel=`), then the true peak (`master: level`, last, because every change before it moves
the peaks), each with a listen and a compare.

## What it measures

Every measured part (a role of a deck's line: `kick`, `bass`, `melody` ...) and the master have:

| field | what it is |
|---|---|
| `lufs`, `rms_db`, `peak_db` | integrated loudness (BS.1770), RMS and sample peak (dBFS), over the bars the part has notes in |
| `bands` | power in six bands, dB on `rms_db`'s scale (a full-scale sine is -3): `sub` 20-60 Hz, `low` 60-250, `low_mid` 250-500, `mid` 500-2k, `high_mid` 2k-6k, `high` 6k and up |
| `centroid_hz` | the spectral centroid: where the part's energy sits, on average. Low is dark, high is bright |
| `width` | the side's share of the energy: 0 mono, about 0.5 two unrelated channels, over 0.5 left and right working against each other |
| `correlation` | left against right, -1 to 1: 1 mono, 0 unrelated, under 0 out of phase (it thins out on a mono speaker) |
| `crest_db` | peak over RMS: high is punchy and spiky, low is dense or squashed |

The master also has:

| field | what it is |
|---|---|
| `true_peak_db` | the peak between samples (4x oversampled, BS.1770-4 annex 2), dBTP. What a converter or a lossy encoder will reach |
| `plr_db` | true peak over integrated loudness: under about 6 dB the master is squashed |
| `short_term_max_lufs` | the loudest 3 seconds |
| `lra` | loudness range (EBU Tech 3342), LU: how much the loudness moves through the track. Dance music is often 3 to 8; breakdowns and drops push it up |
| `clipped_samples` | samples over full scale |

Each section has its `lufs`, `rms_db`, `peak_db`, `true_peak_db` and `bands`. `bar_lufs` is a loudness a bar.

`clashes` lists two parts that carry the same band in the same bars at levels close enough to mask each other:
`{parts, band, hz, bars, overlap, gap_db, sections}`, the louder part first. `bars` are runs of bars (1-based, inclusive).
`overlap` (0 to 1) is how much of the quieter part's energy in the band meets as much of the other's, frame by frame: a duck
that takes a bass out of the kick's way brings it down even when the two are as loud as before. Two drums (kick, snare,
hihat, clap) sharing a band are a kit, not a clash, and are not listed.

`targets` is the master against a delivery target: `{name, lufs, true_peak_db, lufs_delta, true_peak_delta, met}`. A delta
is the master's value minus the target's; `met` is a loudness within 1 LU of the target and a true peak at or under the
ceiling.

## Targets

| `targets` | loudness | ceiling | what it is |
|---|---|---|---|
| `streaming` (default) | -14 LUFS | -1 dBTP | Spotify, YouTube, Tidal and most streaming services turn a louder master down to about -14 |
| `apple` | -16 LUFS | -1 dBTP | Apple Music's Sound Check |
| `broadcast` | -23 LUFS | -1 dBTP | EBU R 128 |
| `club` | -8 LUFS | -0.3 dBTP | a loud dance master for DJ play: common practice, not a standard |
| `-12/-1` | any | any | your own: LUFS and dBTP |

A master louder than its target is not better: the service turns it down by the difference and the peaks it gave up are
gone. A master quieter than its target plays quieter than the tracks around it.

## The order of fixes

Fix in this order. Each step changes what the next one measures, and a later fix can undo an earlier one if it is done
first (raising the loudness before the clashes are fixed raises the clashes too).

### 1. What is not heard

`silent` lists every deck or part that has notes and is not heard, with the reason; the warning says it in words.

| reason | the fix |
|---|---|
| `mute` | `deck A: unmute` |
| `level_0` | `deck A: level 0.8` |
| `crossfader` | `crossfader 0.5` (a two-deck program with the crossfader at one side shuts the other) |
| `filter_closed` | `deck A: filter off`, or a ramp that ends open |
| `eq_killed` | `deck A: low 0.5` / `mid` / `high` (0.5 is flat) |
| `master_zero` | `master: level 1` |
| `voice_missing`, `no_sound` | the voice made nothing: check its name with `instruments`, or its octave (`oct=`) and notes |

### 2. Clipping

`clipped_samples` over 0: the master goes over full scale and distorts, and every number after it is of the distortion.
The `true_peak` warning comes right after the clipping one and names the level that ends it. Take level off where it
comes from:

- `master: level 0.8` (the master fader, 0 to 1; its law is linear, so 0.8 is -1.9 dB). The warning names the level that
  meets the ceiling.
- `deck A: level 0.7`, or `vel=` on the loudest part (a hard-hit kick at `vel=127` is often the peak).
- `master: drive 0.3` (or `deck A: drive`) saturates: it rounds the peaks off and colours the sound. BeatScript has **no
  limiter** or compressor; drive is the only tool that changes a peak without changing the level.

### 3. Clashes

Two parts in one band at once mask each other: the bass loses its notes under the kick, a pad blurs a lead. The warning
says which and where, and picks the part to move (under 250 Hz the kick and the bass own the band, so the other one moves;
above it, the quieter one, unless it is a drum).

- **The kick and the bass**: `duck B under A.kick 6db`: deck B goes down 6 dB on every kick of deck A (the engine's
  sidechain pump; docs/beatscript/ext-mix.md). It keys on the kick's low end, so only a `kick` (or, with a warning, a
  `bass`) can key it, and it ducks a whole deck.
- **Cut the band on one deck**: `mix { B eq low -6 }` (in a section: applied on its first bar, put back when it ends), or
  `deck B: low 0.375`. The deck EQ is three bands: a low shelf at 250 Hz, a mid peak at 1 kHz (1.5 octaves wide) and a high
  shelf at 4 kHz, -24 to +6 dB. `low_mid` and `mid` clashes take `eq mid`; `high_mid` and `high` take `eq high`.
- **Take the low end off a part that does not need it**: `deck B: filter hp 150`.
- **Two parts on one deck**: the EQ, the filter and a duck move the whole deck, so: lower one's `vel=`, move one an octave
  (`oct=`), give one fewer notes, or put one on a deck of its own and fix it there.

### 4. Balance

A part `buried` (over 22 LU under the mix) is barely heard; a part much louder than the rest buries them.

- `deck B: level 0.9` for a deck of one part; `vel=` for one part of a deck of several.
- `mix { A level 0.7, B level 0.9 }` in a section to change the balance for that section only.
- Compare the parts' `lufs` and the sections' `lufs`: a drop a few LU louder than the build is the point; a breakdown that
  is louder than the drop is usually not.

### 5. Loudness and the ceiling

With the master clean and balanced, move it to the target. A true peak over the target's ceiling (and no clipping) is
said here, with the loudness: both are the master's level, and one warning's level agrees with the other's (one level
that meets both, or "round the peaks first" when the peaks stand so far over the loudness that the level the ceiling needs
would leave the master well under the target). The warning names the `master: level` that gets there, or says why a level
alone cannot:

- under the target with true-peak room: `master: level` up (to 1 at most), then the decks' `level`.
- under the target with no room (the true peak is already at the ceiling): the peaks are in the way. `master: drive`
  first, then the level. Without a limiter, a dense mix with a low crest factor gets louder more easily than a spiky one.
- over the target: `master: level` down. It costs nothing: the service would have turned it down anyway.
- `plr_db` under 6: the master is squashed; ease `drive`.

### 6. Width

- `correlation` under 0 on the master: left and right work against each other and the mix thins out in mono. The warning
  names the widest part.
- A part that lives under 250 Hz and is wide (`width` over 0.3): low end is best kept mono, on a club system above all.

BeatScript has **no pan or width control on a strip** (`A pan` in a `mix` is read and refused). What there is: the voice
(`inst=`: some voices are wider than others), and on a net deck the aux macros for some bars (`do aux macro=mono deck=A
bars=4`, also `hard-left`, `hard-right`, `ping-pong`, `auto-pan`).

## What BeatScript cannot do yet

These would make a mix easier to polish; they are on the roadmap (docs/ROADMAP.md), not in the language:

- a pan and a width control on a strip;
- a limiter and a compressor on the master and on a deck (drive saturates; it is not a limiter);
- EQ on a part rather than a deck, and more than three bands;
- a duck keyed by a part that is not low (a snare, a hat, a vocal), and one that ducks a part rather than a deck.

## How it measures

The render runs at 22.05 kHz (half the site's rate, for speed). Bands and the centroid come from a short-time Fourier
transform of each part's own render (Hann window of 2048 points, a frame every 512 samples: 23 ms), left and right taken
in one complex transform; a band more than 90 dB down is `null` (nothing there). A part of a deck of several is rendered
on its own (the part passes, within a budget: the MCP tool renders at most 96 bars and 240 seconds of part renders, and a
part past the budget is not `measured` and has no bands). Stereo is from the sums of l², r² and l·r. The true peak is 4x oversampled with a windowed
sinc (12 taps a phase). The loudness range is EBU Tech 3342 over 3 s windows every 100 ms. A clash is found bar by bar: a
band that holds at least a quarter of each part's energy in the bar, both within 10 dB of the master's band, and an
overlap of at least 0.5; bars in runs of two or more. Everything is deterministic: the same program gives the same numbers.

Code: `crates/beatscript-cli/src/mix.rs` (the measures, the warnings, the compare) and `listen.rs`;
`crates/beatscript-serve/src/mcp/listen.rs` (the MCP tool). Tests: `mix.rs` (synthetic signals: a sine's band and centroid,
width and correlation, a true peak between samples, a loudness range, a clash and the duck that clears it) and `listen.rs`
(golden numbers on the examples). The nightly listening tests (`tools/listen`) hold every example's summary still.
