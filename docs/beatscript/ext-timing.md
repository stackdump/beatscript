# BeatScript extension F1: sub-bar timing

Time words finer than a bar: `over 1/2 bar`, `over 2 beats`, `for 2 beats`, `every 2 beats`, `every 1/2 bar from 3`,
`at bar 16 beat 3`, `before drop 2 beats`. A beat is a quarter of a bar. Module `crates/djlive/src/dur.rs` (the types and the
parser's helpers), hooks in `parse.rs`, `expand.rs`, `compile.rs`, `runtime.rs`, `print.rs`. Example `examples/beatscript/timing.beatscript`.
Tests: `crates/djlive/tests/ext_timing.rs` (and the unit tests in `dur.rs`). It builds on F2: a length or a position is arithmetic
(`over (1 + 1) beats`, `at bar $i beat 1 + 1`).

## Grammar

```ebnf
dur         = arith ( "bar" | "bars" | "beat" | "beats" ) ;     (* over 1/2 bar   over 2 beats   every 3/4 bar   the unit's number is not checked: `1 bars` is fine *)
pos         = "bar" arith [ "beat" arith ] ;                    (* at bar 16   at bar 16 beat 3   at bar 16 beat 5/2   at bar 3/2 *)
when        = "every" dur [ "from" ( arith | pos ) ]
            | "at" pos
            | "before" NAME dur
            | "after" NAME | "on" "phrase" | ... as before ;
span        = "over" dur ;                                      (* a change, a ramp, a `do` value that moves, a move (whole bars only) *)
hold        = "for" dur ;                                       (* at bar 4 for 2 beats: master: filter lp 800 *)
```

```text
at bar 2 beat 3: master: delay 0.3
every 2 beats: master: reverb 0.1
every 1/2 bar from 3: deck B: level 0.8
master: filter lp 300 -> 2k over 1/2 bar
at bar 4 for 2 beats: master: filter lp 800
before drop 2 beats: do vocoder set mix=0 -> 1 over 3 beats
```

The API other extensions use (docs/BEATSCRIPT-EXTENSIONS.md: tempo curves, shapes, lfos): `djlive::dur::{Dur, Pos}` and, inside
`impl Parser`, `self.dur(what) -> Result<(Dur, Span), Diagnostic>`, `self.pos() -> Result<(Pos, Span), Diagnostic>`,
`self.over() -> Result<Option<(Dur, Span)>, Diagnostic>`. `Dur` is a [`Rat`] of bars (never negative): `Dur::bars(n)`,
`Dur::from_beats(r)`, `as_bars()`, `as_beats()`, `to_f64()`, `whole_bars()`, `is_zero()`, `canonical()`. `Pos` is a `Rat` of bars
from the program's first bar line: `bar_of()`, `beat_of()`, `whole_bar()`, `canonical()`. `Ramp.bars` is a `Dur` and
`Control.hold` an `Option<Dur>`.

## Semantics

* **A beat is a quarter of a bar. Beat offsets are 0-based like bars:** `at bar 16 beat 3` is the last quarter of the bar whose line
  is bar 16; `beat 0` is the line itself (`at bar 16 beat 0` is `at bar 16`); a beat is `0 <= b < 4` and `beat 4` is an error that
  says so and names `at bar 17`. A position is one exact number of bars, so `at bar 3/2` is `at bar 1 beat 2`, and the bar
  and the beat are arithmetic (`at bar $i beat $i % 4`).
* **`every dur`** fires every `dur` from its first multiple, never at 0, as `every N bars` never does (`every 2 beats` fires at
  1/2, 1, 3/2 ... of a bar, counted from the program's first bar line, or from a section's first bar line inside a section).
  `every dur from X` starts at `X`: a bar number (`from 3`, a rational is fine) or a position (`from bar 3 beat 2`).
  `every 2 bars from bar 1 beat 2` is every two bars, starting a half bar late. A program counts from the bar its first
  evaluation landed on, as `at bar` always did.
* **`over dur`** is a ramp that long (`over 2 beats` is half a bar); **`for dur`** puts the effect back that long after it fired
  (`at bar 4 for 2 beats` is put back at 4 and a half); **`before NAME dur`** is `dur` before the section's line
  (`before drop 2 beats` is a half bar before it). Sections stay a whole number of bars. A `move` takes whole bars (the
  registry's `moves arm` takes bars): `move bass_swap on B over 2 beats` is an error that says so. `after NAME` and `on phrase`,
  `on next bar`, `on next phrase` are as before.
* **The clock is the deck's beat position, not seconds.** The runtime's program position (the bar it counts and how far into
  the bar the grid deck is: the same number the editor marks run on) is checked each service pass: a line with a time between bar
  lines fires when the position **crosses** it, once, whatever the pass rate (the exact time it was due is where its ramp and its
  hold start from, not the pass that saw it). A time equal to a bar line fires on the pass that crosses the line, after the lines
  that are on the bar line. A time that is before the bar the program landed on does not fire (as a whole-bar line does not).
  A clock that jumps (a stall, a seek) fires what is due in between, in order of time, at most 32 times for one repeating line.
  `every 2 beats` after the program has ended does not fire; an `at` after `end at bar N` does not either.
* **A ramp over a fraction of a bar ends exactly:** the pass that reaches `start + length` sends the end value (once) and the ramp
  is over. A hold ends on the first pass at or after its time (`for 2 beats` is exactly two beats on a pass that lands there).
  A hold of whole bars is put back on the bar line, as before.
* **Positions inside a section** count from the section's first bar line (`at bar 1 beat 2` in a section that starts on song
  bar 8 is 9 1/2); `before` counts back from the named section's start. Lines in a `def` or `repeat`: the times are read after
  their `$` names are put in place, and a `repeat ... every N bars` still shifts whole bars. A **call of a def** takes a whole
  bar (`at bar 2: groove(A)`); a position with a beat on a call is an error that says so.
* **The whole bar 0 of a program has nothing at it, a time inside it does** (L-07). `at bar 0` and `at bar 0 beat 0` in a program
  with no sections are errors ("bars are counted from 1": the program lands on bar 1, and `at bar 1` is the first bar after it
  starts); `at bar 1/2`, `at bar 0 beat 2` and `at bar 1/4` are legal, since beats count from 0 like bars: they fire once, when the
  position crosses them, between the program's start and bar 1 (a program evaluated after that time in its first bar does not fire
  them: it was too late). In a section or a song, bar 0 is its first bar and a position inside it is fine. (The first review had
  refused every time before bar 1 at the top while `every 1/2 bar from 1/4` worked there and `at bar 0 beat 2` worked in a section.)
* **A time beyond its section's end is a warning** (L-15): `at bar 9`, `at bar 4 beat 2`, `every 8 bars` and `every 2 bars from 6`
  in a 4-bar section never fire in it (a section counts its own bars from 0, so its last is bar 3). The warning is on the time word
  and names the bar and the section's length ("this line is due at bar 9 of section intro, which is only 4 bars long (bars 0 to
  3): it does not fire in the section"); it is due at that bar of the song, after the section, and fires there if the song goes
  on. A ramp that starts inside and takes longer than the section is said too ("it is still moving when the section ends, and
  goes on after it": a ramp is not cut at the end of its section). Words that name a section or a phrase (`after drop`, `on phrase`)
  are not numbers of bars and are left alone; in a stack the section's own length counts.

## Canonical form

* A duration prints `N bars` when `N` is whole (as today, `1 bars`), else `N beats` (`1 beat`) when the beats are whole, else
  `P/Q bars` in lowest terms: `over 1/2 bar`, `over 2 beats`, `over 4/8 bars` and `over 0.5 bars` are all `over 2 beats`.
* A position prints `bar W` when whole, else `bar W beat B` with `B` a rational in lowest terms (`at bar 16 beat 5/2`).
  After `from`, a whole bar stays a bare number (`every 4 bars from 8`, as today) and anything else is spelled `bar W beat B`
  (`every 2 beats from bar 3 beat 2`).
* Every whole-bar spelling prints exactly as before, so no existing CID moves (the fixtures pin them; `ext_timing.rs` checks
  each whole-bar word).
* Random reformatting keeps the CID; two spellings of one time are one program; another time is another program.

## Decisions

* **Whole-bar forms keep their old AST.** `When::Every(n)`, `EveryFrom(n, f)`, `At(n)`, `Before(name, n)` and `Timing::Every`,
  `EveryFrom`, `At` are what a whole-bar word still makes; the new variants `When::EveryPos(Dur, Option<Pos>)`,
  `AtPos(Pos)`, `BeforeDur(Word, Dur)` and `Timing::AtPos(Rat)`, `EveryPos { step, from }` (at the end of their enums) are made
  only when a duration or a position is not whole bars. So a whole-bar program is the tree it was, and a later worker's `match`
  on `When` / `Timing` has the old arms untouched.
* **`Line.hold`, `Action::Move.over`, `Action::Change.over`, `DoCall.over`, `FlatLine.hold`, `Control.hold` and `Ramp.bars` carry
  a `Dur`** (the brief: the runtime needs the exact length). Three assertions of the existing tests that compared them with a
  whole number now say `Dur::bars(n)` (`beatscript.rs` twice, `language.rs` once); everything else of the existing suites is untouched
  and green.
* **`from` takes a bare bar number or a position.** `every 1/2 bar from 3` (the contract's example) and `every 4 bars from 8` (the
  old form) are the same grammar; `from bar 3 beat 2` also reads.
* **A time between bar lines counts in bars (exact) from the program's origin;** the runtime compares it with the program's
  position as a float with a nanobar of slack (`1e-9`, the same as `lines_crossed`), so a beat a hair short of a bar line is the bar
  line. The fraction of a bar the runtime uses (`frac`) was `fract(beat / 4)` and is now `fract(beat / 4 + 1e-9)`, so that the bar
  count and the fraction agree at a line; nothing else reads it to that precision.
* **A repeating time is not capped for its whole run, only per pass:** at most 32 firings of one line in one pass. An `every`
  finer than 1/64 of a bar (a sixteenth of a beat: `dur::min_every`) is an error ("faster than the clock"): the runtime
  looks at the position about sixty times a second, and a line that repeats faster would only flood the host.
* **A def call takes a whole bar.** The base of a call's lines is a bar number (`Scope.base`); a finer base is a larger change
  (every line in the body would carry a fractional base) and no program needs it yet: the error names what to write instead.
* **Zero lengths:** `every 0 beats` and `for 0 beats` are the same errors as the bar forms (the messages say the unit it was written in: R-16);
  `over 0 beats` is "a ramp takes some time". `before drop 0 bars` is the section's line, as it was.
* **`1 bars` / `2 bar`:** the unit's number is not checked, as it never was.
* **A program with no sections counts its bars from the bar it was evaluated in** (R-02, kept; docs/BEATSCRIPT.md "Time words"): with a deck playing, its
  music lands on bar 1 and `at bar 1` is the first bar you hear; the offline render starts the music on its bar 0, so the same program has its timed
  lines one bar later there. A song, and the tempo curve, count from the first bar you hear in both.

## Not done

* A section, an evolve generation and a `repeat ... every` are whole bars: no `section drop 1/2 bar`, no `every 2 beats` shift of a
  repeat (the contract says sections stay whole).
* A call of a `def` with a time between bar lines (see above).
* A fractional `move ... over` (the registry's `moves arm` takes whole bars).
* The offline render (`beatscript_render`) runs no control line but `duck` and `mix` (and a tempo curve), so a time between bar lines is in it only for those
  two, placed on its beat (docs/beatscript/ext-mix.md); every other line with a time is not in a render, as for any control line.
