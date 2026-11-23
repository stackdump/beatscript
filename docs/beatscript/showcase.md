# showcase: lanternfall

*A two minute track in one BeatScript program, written to use the extensions of docs/BEATSCRIPT-EXTENSIONS.md the way a person making music
would, and to find where they do not fit together. Program: `examples/beatscript/showcase.beatscript` (original, CC0-1.0, no host needed).
Render: `~/Workspace/llm-headline-loops/livecode/showcase.wav`. This file is the notes: what is where, what the render plays and
what it does not, the decisions, and what was found.*

```text
$ djctl btw check examples/beatscript/showcase.beatscript
...                                      (26 notes: the ontology's names for the Euclidean rhythms, 14 chance terms, the period, the
                                          performed inputs, the two pending level assertions; no error, no warning about a voice)
mix in intro: A level 0.45, B level 0.3, master level 0.85 -> mixer {...}; reads the mixer first, put back on bar 8
duck B under A.kick 6db attack=5ms release=120ms in groove: trick {"action":"on","amount":0.499,"deck":"B","div":16,"kind":"pump","source":"A"} ...
...
length: 2m04s with the tempo curve, 64 bars
ok  cid z4EBG9j37QBGtpgBbGG2nLFDQ3nvwq1FdePDEjcZDiSLtHVbRVy  228 lines  64 bars
assertions: 4 hold, 2 pending (`djctl btw test` measures them on a render)

$ djctl btw test examples/beatscript/showcase.beatscript
assert ends: the program ends at bar 64: pass
assert length == 2m ± 12s: the program is 2m4.198s (through the tempo curve): pass
assert key == am in intro: the key in intro is am: pass
assert key == bm in hook2_a: the key in hook2_a is bm: pass
assert level A < 2 * B in groove: A 0.141, B 0.094: 0.141 < 0.188: pass
assert level A < B in outro: A 0.022, B 0.037: 0.022 < 0.037: pass
note: this program is performed: levels are measured with every input at 1/2 (the render has no controllers), and only the duck and mix lines are in the render (knob(2), knob(3))
6 assertions pass
```

Program CID `z4EBG9j37QBGtpgBbGG2nLFDQ3nvwq1FdePDEjcZDiSLtHVbRVy`; canonical text 6418 bytes, 228 lines (the source is 253 lines of text
and comments); a fixed point of `btw fmt`; 240 random reformattings (see "How it was checked") keep the CID.

## The form

64 bars, 2m04.2s through the tempo curve. The tempo is written from `let base = 122`. The seconds are the tempo map's
(`btw test` reports them as the level windows). Bars are program bars (the first section is bars 0 to 8).

| section | bars | seconds | tempo (BPM) | key | what plays |
|---|---|---|---|---|---|
| `intro` | 0-8 | 0.0-16.3 | 114 to 122, s-curve | am | pad (voice-led), pluck four notes a bar, the twelve-step hi-hat against the bar, the kick from bar 4 |
| `build` | 8-16 | 16.3-31.7 | 122 to 130, exp | am | kick, eighth hats with a chance, a snare roll in the last two bars, the sub, pad, pluck in eighths, the hook's first phrase as a teaser from bar 4 |
| `drop` = `stack(groove, then(hook_a, hook_b))` | 16-32 | 31.7-61.3 | 130 | am | `groove` (drums, offbeat sub) under `hook_a` (the hook, plucks in eighths) then `hook_b` (the answer, plucks in sixteenths) |
| `breakdown` = `then(hush, lift)` | 32-40 | 61.3-78.8 | 130 to 104 (log, two bars) held, then 104 to 132 (exp) | am | `hush`: soft hats, sub, pad, bells on two notes a bar, a sung line. `lift`: the kick comes back, a snare roll, plucks, a second sung line |
| `drop2` = `stack(groove2, then(hook2_a, hook2_b))` | 40-56 | 78.8-107.9 | 132 | **bm** | the same shapes a tone up, plus a clap, busier hats, a bass with pickups, and the hook sung over the answer |
| `outro` | 56-64 | 107.9-124.2 | 132 to 112, log | am | pad, pluck, bells (the first sung line again), a kick on two beats; the mix steps down |

The arc, as measured on the render the program makes today (`djctl btw render`, 48 kHz, 124.198 s, peak 0.9039 = -0.88 dBFS):

| window | seconds | master RMS | deck A (drums and sub) | deck B (pad, pluck, lead) |
|---|---|---|---|---|
| intro | 0.0-16.3 | 0.0901 | 0.063 | 0.045 |
| build | 16.3-31.7 | 0.1223 | 0.083 | 0.090 |
| drop (`groove` and the hooks) | 31.7-61.3 | 0.1695 | 0.141 | 0.094 |
| hush | 61.3-70.2 | 0.0962 | 0.052 | 0.081 |
| lift | 70.2-78.8 | 0.1453 | 0.097 | 0.108 |
| drop2 | 78.8-107.9 | 0.1661 | 0.132 | 0.102 |
| outro | 107.9-124.2 | 0.0425 | 0.022 | 0.037 |

(Master RMS: the RMS of each window of the WAV, both channels; the deck columns are `btw test`'s stems at 48 kHz, the same render. **Re-measured on the integration branch after the render merge and the second review's fixes**: the master column and the peak of every row, and the deck columns of the drop and the outro, which `btw test` prints (the other deck columns are the first measurement and were not redone). The outro moved most (master 0.0626 to 0.0425, A 0.030 to 0.022, B 0.055 to 0.037, peak 0.8752 to 0.9039) because the render now carries `master: level 0.85 -> 0.3 over 8 bars log` and the other mixer lines, which the first measurement did not play; the small moves elsewhere (0.0001 to 0.0002) are the chance seed no longer hashing the `vel=`/`oct=` of a part (L-05), which draws the ghost hats again.) Intro < build < drop,
hush < drop2, the outro is the quietest; peak is above 0.3 and below 0.95; neither deck is silent where it should play. Over time
(2 s windows) the build climbs in four steps from 0.10 to 0.15, the drop sits at 0.17, and the outro steps down from 0.09 to 0.03.

## Which feature is where

| id | feature | in the program |
|---|---|---|
| F1 | sub-bar timing | `at bar 7 beat 3 for 1 beat: mix { A eq low -24, B eq low -24 }` (the bass leaves for the last beat of the build), `before groove 2 beats:` (a swell), `every 2 beats:` (a half-bar filter pulse in the drop), `at bar 15 beat 3 for 1 beat:` and `at bar 15 beat 2 for 2 beats:` (the bass drops out before the next section), `at bar 3 beat 2 for 2 beats:` in a `repeat 4 every 4 bars` (a filter dip every fourth bar), `over 1 beats`, `over 2 beats` |
| F2 | arithmetic, lets, `when` | `let base = 122`, `tempo base - 8`, `tempo base + 8 -> base - 18 over 2 bars log`; `let half = drop_len / 2` and `section hook_a half bars`; `repeat 8 { at bar $i: master: filter hp 100 + $i * $i * 60 ... reverb 0.15 + 0.4 * $i / $n when $i % 2 == 1 }`; `mix { A level 0.45 + $i * 0.1 }` in a `repeat 4 every 2 bars`; `when $steps == 4` and `when $n >= 3` on part lines of two defs; `3k + knob(3) * 6k` |
| F3 | tempo curves | `tempo base - 8 -> base over 8 bars s-curve`, `base -> base + 8 over 8 bars exp` (the accelerando into the drop), `base + 8 -> base - 18 over 2 bars log` held, `base - 18 -> base + 10 over 4 bars exp`, `base + 10 -> base - 10 over 8 bars log`; a section's own `tempo` over the top-level default; `check` and `assert length` through the curve |
| F4 | shapes, lfo | `deck B: level 0.3 -> 0.55 over 8 bars s-curve` (the pad fades in), `master: level lfo sine 2 bars center=0.85 depth=0.03` in both drops, `master: level 0.85 -> 0.3 over 8 bars log` (the outro), `master: filter lp 500 -> 9k over 8 bars exp`, `lp 18k -> 900 over 2 beats exp` |
| F5 | degrees, numerals | the changes are numerals, `<i VI III VII>`, played by `harmony ... voice-lead` and read by `arp` and `bass`; the hook is `deg(5 ~ 5 3 ~ 5 6 5)` with octave marks (`1'`, `3'`), an octave down (`1,` in the last bell), and a sharp degree (`#4`, the chromatic step into the fifth); **one `key bm` at the head of `groove2` re-keys every degree and numeral of the second drop** (the hooks stack on it, so they play in it): nothing else was written twice |
| F6 | Euclid, polymeter | `kick e(4,16)` (the ontology names it), `snare` on `e(2,16,4)`, `clap e(3,8)`, `hihat maybe(e(5,8), 0.75, 3)`, `kick e(2,16)`; `hihat/12 x ~ x x ~ x` in the intro and the outro: twelve steps against sixteen, `check` says "period: 24 bars" (the note; `assert period == 24` is not in the program: see "After the second review") |
| F7 | chance | `x?0.35` and `x?0.5 x?0.4` ghost hats, `x*8?0.8` in the build, `choose [$in [$in $in]] weights 3 1 seed=5` (a double-time pickup on the bass), `maybe(e(5,8), 0.75, 3)`; every one decided by a hash of (seed, bar, step), so the render is the same each time |
| F8 | mix moves | `duck B under A.kick` (4 dB in the build, 6 dB in each drop), a `mix { }` in every section, and the break's snapshot (`hush`: A 0.3, B 0.55, master 0.85, put back when it ends); `B eq low -6` in the drops, `A eq low -24` for the drop-outs; the crescendo and the fade are stepped mixes (below) |
| F9 | `sing` | four typed lines on the bell or lead melody: `"slow-ly the lan-terns float a-way"` (hush; again over eight bars in the outro), `"we car-ry all the light a-long"` (lift), `"light it up and bring it home, the lan-terns one by one, we are the ones that glow, rise up and meet the sun"` (twenty-five syllables for the twenty-five notes of the answer, over `hook2_b`, labelled `key=bm`). Control lines: **the offline render has no voice, so none of this is in the WAV** |
| F10 | stack, then, aliases | `let drop = stack(groove, then(hook_a, hook_b))`, `let drop2 = stack(groove2, then(hook2_a, hook2_b))`, `let breakdown = then(hush, lift)`, `arrange intro build drop breakdown drop2 outro`; the drums and the bass are one section (16 bars), the two hooks another (8 + 8), and the equal lengths stack |
| F11 | typed ports | `def changes() out: chords`, `def bed(oct) in: chords out: bass` (the pad; it emits the roots), `def sparkle(steps) in: chords`, `def sub_long()`, `sub_pulse()`, `sub_roll()` (`in: bass`); five `wire`s; the progression is written once, in one place, and the pad, the pluck and three different basses all follow it (and the key) |
| F12 | assertions | `ends`, `length == 2m ± 12s`, `key == am in intro`, `key == bm in hook2_a` (proved by `check`); `level A < 2 * B in groove` and `level A < B in outro` (measured: A 0.141 against 0.188, A 0.022 against B 0.037) |
| F13 | performed inputs | `every 2 beats: master: filter lp 14k -> 3k + knob(3) * 6k over 1 beats exp` (the third knob sets how far the pulse closes), `master: delay 0.1 + knob(2) * 0.3`; both are filters or sends, which a render does not play, so `btw test` reading the knobs at 1/2 changes no number |
| F14 | ontology | `#! genre: edm`; `#! tags:` synthwave, synthwave-1 (the progression 1-6-3-7), edm-standard-1 (intro > buildup > drop > breakdown > buildup > drop > outro: the form), four-on-floor, sidechained, warm-pad, sub-bass, pluck, fm-bell, euclidean-rhythm, ghost-note, buildup, drop, breakdown; all are terms of the cached vocabulary (`djctl btw terms`), `djctl btw jsonld` exports them with the Euclidean rhythms linked (`e(3,8)` is the ontology's house snare shape, `e(4,16)` its kick) |

### Not in the program, and why

* **`import`**: needs a saved snippet (the examples harness has no store; `ports-import.beatscript` covers it).
* **`do` calls and moves** (`do vocoder ...`, `move loop_roll`): nothing in this track needs the vocoder or a transition move the sections do not already
  make, and a `do` line is unvalidated in the harness.
* **`pad(n)`, `held(...)`, `fader(...)`**: no musical use here that a render could hear; `knob` is the input that fits a sweep. (`pad(5)` would be a gate:
  `every 2 bars: deck A: low 0 when pad(5)`, as in `perform.beatscript`.)
* **`swing`, `evolve`, `loop`**: the groove is straight and the form is written out; an evolve would hide the arc this file is meant to show.
* **timed `tempo` lines** (`at bar 16: tempo ...`) and **`stack` / `then` over parts** (`deck A: stack(kick ..., snare ...)`): the tempo is per section, the layering is of
  sections; both forms are covered by `tempo.beatscript` and `algebra.beatscript`.
* **`sing` with a neural voice**: the host has none, so the lines name formant voices (`soprano`, `choir-aah`) and `check` is silent about them; `voice=alto` plays the
  formant fallback and warns on every line.
* **An alias in a time word or an assertion** (`before drop 2 beats`, `assert ... in drop`): not accepted (below), so those name a real section (`groove`).

## What the render plays, and what it does not

`djctl btw render` plays the notes, the sections, the tempo curve, the three ducks, the mixes and the stepped fades (1094 moves in all: mostly the tempo staircase). It
does **not** play: the filters and reverbs and the delay (`master: filter ...`, `reverb`, `delay`: the arrangement has no op for an insert), the four `sing`
lines, the performed inputs, and (today) the level ramps and lfos (`deck B: level 0.3 -> 0.55 ...`, `master: level lfo ...`, `master: level 0.85 -> 0.3 ...`).
Every one of these lines is marked `(live)` in the program (the two `repeat`s of filter lines carry the mark on their opening line).

Because of that the **arc of the track is in the mixes**, not in the ramps: each section sets its own `mix { }`, and the gestures a person
would draw as ramps (the build's crescendo, the lift's, the outro's fade) are written as a `repeat` of mixes, which the render plays now (the build's and the outro's are):

```text
repeat 4 every 2 bars {                  # the build: a crescendo in four steps
  at bar 0 for 2 bars: mix { A level 0.45 + $i * 0.1, B level 0.55 + $i * 0.07 }
}
repeat 4 every 2 bars {                  # the outro: the fade, four steps down
  at bar 0 for 2 bars: mix { A level 0.4 - $i * 0.1, B level 0.5 - $i * 0.1 }
}
```

The ramps and lfos that sit beside the mixes are for a render that carries them (the render worker's change) and for live play. They were written so the two
**agree**: a ramp starts at the value its section's mix holds (`mix { B level 0.3 }` and `deck B: level 0.3 -> 0.55`; the master lfo's centre is the mix's 0.85
and a sine starts on its centre), so when both are played the ramp takes over from the mix with no step, and a knob that a stepped mix drives has no ramp on it in the same
section (the build and the outro leave the deck levels to the steps; their master level is a section-wide mix with, in the outro, a ramp that starts where it holds; in the lift deck A is stepped and deck B has a plain mix).
What to look for in a render that carries them: the intro's pad should rise from 0.3 to 0.55 under the 8 bars, the master should breathe by +-0.03 every two bars in
the drops, and the outro should fall to about a third of its start.

## Decisions

* **The mixes carry the arc** (above). The offline engine limits a deck: its kick, pad and bass reach a peak of 0.5 (-6.0 dBFS) at any velocity, so how loud a section is
  comes from which decks play and the strip levels, not from velocities: `vel=` ranges are there for the touch, and a pad at `vel=20` renders within 1 dB of one at `vel=120`
  (RMS 0.131 against 0.142).
* **Master level 0.85 and strips at 0.9 / 0.85 in the drops**: the first full render, at master 0.95, peaked at 1.00 (clipped); at 0.85 it peaks at 0.875.
* **Tempo 114 to 132, 124 seconds.** About two minutes was the brief; at constant 122 the same 64 bars are 125.9 s. The curves move the tempo by 8 to 28 BPM, so they are heard.
* **The key change is one `key bm` line**, in `groove2`, not a transposed copy: the hook is `deg()`, the changes numerals, the bass `$chords`. A stretch of a stack sounds in one key, the first
  keyed section's, and `groove2` is in both stretches of its stack (bars 0 to 8 with `hook2_a`, 8 to 16 with `hook2_b`), so the hooks need no line of their own; `assert key == bm in hook2_a`
  holds and the pitch of a melody-only render confirms the second stretch is in bm too. (Each hook with its own `key bm` also works; the first version of the file had them.)
* **The progression is a def with an output port**, wired to the consumers. `emit $chords` into an `out: bass` is a type error (the value is chords); `emit [$chords]` is a group
  and passes, and it is how the roots of the changes reach a bass part. The bass lines are then rhythms around `$in` in a group (`[~ $in ~ $in ~ $in ~ $in]`): the four copies of the
  alternation inside one bar all read the same chord, so the root follows the bar (checked by tracking the pitch of a bass-only render: A F C G in am, B G D A in bm).
* **Aliases are for `arrange`, `stack` and `then`.** A time word and an assertion name a section, so they say `groove`, not `drop`.
* **The sung lines fit their melodies**: eight syllables for the eight notes of a four-bar bell part (two a bar), twenty-five for the twenty-five notes of `<r1 r2 r3 r4>` (the lyric
  starts again over the second four bars). Hyphens say the syllables the dictionary should not guess. The lines are mine; nothing is quoted.
* **Genre `edm`**, because the form is the ontology's `edm-standard-1`; the progression and the kit are synthwave's, so those are tags.
* **Names**: `bed` and `sparkle`, not `pad`, `drums` or `beat` (reserved), `groove` not `beat`, `hush` and `lift` not `low`.
* **`assert level A < B in outro`** was added to the three the brief named (`period == 24` was too, and was dropped again after the second review, see "After the second review"); it holds with margin on today's render (0.022 against 0.037) and would still hold with
  the ramps (both decks fall together).

## Found while writing it

Nothing here stopped the piece. Each has the exact text and what the tools said; the program works round them.

1. **A `repeat` inside a `def` cannot use its own `$i` or `$n`** (a bug). The def's call fills every `$name` of its body, nested `repeat` included, before the repeat
   has bound its index:

   ```text
   tempo 120
   def swell() {
     repeat 4 {
       at bar $i: master: reverb 0.1 + $i / 10
     }
   }
   section s 4 bars {
     deck A: kick x ~ x ~
     swell()
   }
   arrange s
   ```

   ```text
   error: `$i` is not a parameter of def swell
    --> 4:12
     |
   4 |     at bar $i: master: reverb 0.1 + $i / 10
     |            ^^
     = help: def swell has no parameters
   ```

   With a parameter (`def swell(n) { repeat $n { ... } }`) the help says "did you mean `$n`?", which is the wrong advice for `$i`. The way round is a def with a parameter, called from the `repeat` (`def swell(i) { at bar $i: ... }` and `repeat 4 { swell($i) }`: checked, it works), and a `repeat` in a def that
   does not use `$i` works (the hand-dubstep example). The showcase puts its `repeat`s in the sections.
   The cause is `Expander::fill` (`expand.rs`): it replaces or rejects every `$name` of the def's body at the call. It could leave `$i` and `$n` alone
   when the def has no parameter of that name and the token is inside a `repeat` body.
2. **A `let` that names a composition is not a section for time words and assertions.** `arrange`, `stack` and `then` take it; `before`, `after` and `assert ... in` do not:

   ```text
   tempo 120
   section top 2 bars {
     deck A: kick x ~ x ~
   }
   section wash 2 bars {
     deck B: bass c2 ~ ~ ~
   }
   let both = stack(top, wash)
   arrange both
   assert level A < 2 * B in both
   ```

   ```text
   error: there is no section `both`
     --> 10:27
      |
   10 | assert level A < 2 * B in both
      |                           ^^^^
      = expected: top or wash
      = help: the sections on the timeline are top, wash
   ```

   and `before both 1 bars: master: reverb 0.5` in a section says "there is no section `both` on the timeline". The messages are clear; the alias just cannot be used there. A fix
   could resolve an alias to the sections of its composition (for `assert ... in`, the union of their windows).
3. **`djctl btw nets FILE --sections` prints nothing but an error for a song whose sections' velocity ranges differ**, which is nearly every song. The per-section nets exist
   (`Compiled::net_cids`); the command asks for the longhand first and returns on its error:

   ```text
   tempo 120
   section one 2 bars {
     deck A: hihat x*8 vel=50..60
   }
   section two 2 bars {
     deck A: hihat x*8 vel=60..70
   }
   arrange one two
   ```

   ```text
   $ djctl btw nets nets2.beatscript --sections
   program z4EBG9j4daSGnd9vMsv5MWTqBoh55AVMesiLGgco3BDfzNXeGYw
   djctl: deck A hihat: `vel` differs between sections
   ```

   (`btw_cmd.rs`: `comp.song_net_cids()?` comes before the `--sections` loop; the showcase says the same about its hi-hat.) The error is a documented limit of the longhand form; the listing
   it blocks is not.
4. **Reserved words that are natural names** (an annoyance, the messages are right): `pad` (a def, it is the performed input), `drums` (a def, a port type), `beat` (a section),
   `low` (a section, an EQ band), `a1`-style names (notes). Each error is "`X` is a word of the language" or "reads as a note, chord or key", at the name.
5. **A def body prints its guards unevenly**: `deck A: hihat tick ... when $n < 3` is `when $n <3` in the canonical text, `$n >= 3` is `$n>=3`, `$steps == 4` is `$steps==4`, `$i % 2 == 1`
   is `$i % 2==1`, `$i * $i * 60` is `$i*$i*60`. A def body is printed token by token as written, so this is the existing rule, and the text is a fixed point and reads back; it is just
   not the spacing `Expr::canonical()` gives the same expression outside a def (`3000 + knob(3) * 6000`).
6. **The pump's release is chosen at the program's first-bar tempo for every duck.** `check` says "release 120ms is a 1/16 note at 114 BPM: 132 ms" for the duck in the drop, which runs at 130,
   and for the build's duck, which starts at 122. This is what ext-mix.md says ("the tempo the program starts at"), and the note (1/16) is the same at any of these tempos, so nothing sounds
   different; the report reads oddly in a song with sections at other tempos.
7. **`check` is loud about chance.** Each use of a `let` pattern with a chance in each section says "it differs every bar, unrolled over N bars" again (26 notes for this file, 14 of them
   chance). Harmless; a one-line summary per term would do.

## After the second review

Merging the second review's fixes (fixer b) into the integration branch changed three things in this file's story. None of them is the music.

* **`assert period == 24` is gone from the program.** A part with chance is a hash of the bar and never comes round (L-10), so a program with one has no period to assert: the intro's
  hi-hat has twelve steps, but the bass and the hats of the build have chance, and `assert period == 24` now says "the program has no period: hihat on deck A, bass on deck A have chance,
  which differs every bar (the rest comes round in 24 bars (hihat/12 against the bar))". The assertion was meaningless for this program, so it was removed. The `period:` note of `check` ("24 bars = 384 steps
  (hihat/12 against the bar), in section intro", and 12 bars in the outro) is the polymeter's own and stays. Six assertions pass, not seven.
* **The CID moved, for two reasons that are not the music.** `z4EBG9j8Niv8GBhjXeAK2wtNMG39kqgNCb16sfND1Azj7Trp1qw` became `z4EBG9j37QBGtpgBbGG2nLFDQ3nvwq1FdePDEjcZDiSLtHVbRVy`: the dropped assertion
  is a line of the canonical text, and arithmetic with no input now folds to its number wherever it stands (L-08). Checked by running the program without the dropped line through the build
  before and after the fix: the canonical texts differ in exactly nine lines, each a fold. Eight are tempos: `tempo 122 - 8` is `tempo 114`, `tempo 122 + 8` is `tempo 130`, `tempo 122 + 10` is `tempo 132`, and the five ramps
  (`tempo 122 - 8 -> 122` is `tempo 114 -> 122`, `122 -> 122 + 8` is `122 -> 130`, `122 + 8 -> 122 - 18` is `130 -> 104`, `122 - 18 -> 122 + 10` is `104 -> 132`, `122 + 10 -> 122 - 10` is `132 -> 112`). The ninth is `master: delay 1 / 10 + knob(2) * (3 / 10)`, now `master: delay 0.1 + knob(2) * 0.3`. The numbers are the same, so the render plays
  the same tempos. The `delay` line keeps its `knob(2)`: an expression with an input in it stays an expression.
* **Numbers of the render.** The arc table above was re-measured (see the note under it). The second review's L-05 (a chance part's implicit seed no longer hashes its `vel=`/`oct=`/`inst=`) draws a few ghost
  hats again, so the master RMS of the loud windows moved in the fourth decimal, and the outro now includes the mixer lines the render carries since the render merge.

## How it was checked

* `djctl btw check`, `fmt`, `test`, `render`, `jsonld`, `terms` (the prebuilt binary of the integration branch; no cargo was run in this worktree). `btw fmt` twice gives the same text and
  the same CID (a fixed point).
* **`cargo test -p djlive --test examples_beatscript` was not run** (the brief rules out cargo). Its checks were done by hand: the fixed point as above, and a Python port of the test's
  `reformat` (every token written out again with other whitespace, comments, blank lines, indentation, `|` continuation lines and number spellings, with the glued pairs `x?0.35`
  `hihat/12` `4db` `5ms` `A.kick` `1'` `1,` `==` kept glued) over 240 reformattings of the final file (four seeds; about 300 more of the versions on the way): every one the same CID. The file needs no host: no `import`, no `do`, and `sing` with formant voices.
* The notes were checked by tracking the pitch of single-part renders (naive autocorrelation): the hook in am (`E E C . E F E`), in bm one tone up (`F# F# D ...`), the `#4`
  (D#) in the answer, the bass in am (A1 F2 C2 G2) and in bm (B1 G1 D2 A1), the `choose` pickup (a double hit on the last eighth in four of eight bars).
* The sub-bar mixes land on their beats under the tempo curve: in the WAV the RMS of the last beat of the build (`at bar 7 beat 3`, beat 31 of a ramp from 122 to 130) is 0.057 against
  0.143 the beat before; the last beat of the first drop is 0.092 against 0.167, and the last two beats of the second drop are 0.107 and 0.093 against 0.172 (the bass and the kick's low end
  are out for exactly those beats).
* The duck works in this program: without the three `duck` lines deck B reads 0.104 in the drop, with them 0.094.
* The tags and the genre: a typo (`edmm`, `ghost-notes`) is a warning with a did-you-mean (`djctl btw check` with the header), so the header is read.

The render was made with `djctl btw render examples/beatscript/showcase.beatscript ~/Workspace/llm-headline-loops/livecode/showcase.wav` (48 kHz; about a minute on a debug build). The render worker's extension
(ramps, shapes and lfos on the mixer) lands after this; re-render with it and compare against the table above (it should change the intro, the drops and the outro by the amounts under
"What the render plays").
