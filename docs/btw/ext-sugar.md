# Pattern sugar (long note)

The short section is "Pattern sugar" in docs/BTW.md; the code is `crates/djlive/src/sugar.rs`, the proofs
`crates/btwapi/tests/sugar.rs`, the example `examples/btw/sugar.btw` (proved against its longhand,
`crates/btwapi/tests/sugar_fixtures/longhand.btw`).

## What "sugar" means here

A sugar is a spelling with a **longhand that already compiles**. The compiler writes every sugar out to that longhand
before a part is evaluated (`Compiler::deck` calls `sugar::part` on each part), so the sugar's nets *are* the longhand's
nets: there is no second evaluator to drift. `btw same-as` checks it from the outside, by net CIDs, for every row of the
table in BTW.md and for the whole example.

Three things follow from "written out first":

- **Determinism is the longhand's.** Nothing in sugar.rs draws a random number; a rewrite is a function of the text. A
  part's implicit seed (for chance and `markov`) is hashed from the part *written out*, so `ply([~ x?0.5 ~ x], 2)` drops
  exactly the hits that `[~*2 x*2?0.5 ~*2 x*2]` drops. For a part with no sugar the written-out part is the part, so no old
  program's seed or CID moves.
- **The canonical text keeps the sugar.** `fmt` prints `c2~3`, not `c2 ~ ~`: the CID of the text changes, the nets do not
  (the same relation a `let` refactor has to its inlined text). The bare `x?` is the one exception and prints `x?0.5`,
  because the longhand is shorter to read than to explain.
- **Errors are said where the sugar is**, with a help line that shows the way out (tests: `check_says_what_is_wrong_...`).

## The survey (ranked)

Concepts read from the TidalCycles and Strudel documentation, in our own words; no code, tests or doc text were taken
(Strudel is AGPL-3.0). "Sugar" means it has a longhand in today's btw; "new" means it needs something the evaluator or
the nets do not have.

| rank | idea | what it does | btw | kind | determinism | value for whole tracks | source page |
|---|---|---|---|---|---|---|---|
| 1 | replicate `!n` | a step repeated as separate steps | not yet: `x:3` is the ratchet (see Decisions) | sugar | none needed | high: drum lines and held bass are mostly repeats | Strudel "Mini-notation" (strudel.cc/learn/mini-notation); Tidal "Mini notation" (tidalcycles.org/docs/reference/mini_notation) |
| 2 | elongate `@n` | a step given more of the bar | `c2~3` (rests after it) | sugar | none needed | high: bass and chord rhythms with long notes | same pages |
| 3 | polymeter `{a b, c d e}`, `{..}%n` | patterns of different lengths stepped at one rate | `poly(..)`, `poly(n, ..)` | sugar (alternation of the bars it takes to come round) | none needed | high: the cheapest way to make 8 bars that do not repeat | same pages |
| 4 | `off` / `superimpose` | a moved (and changed) copy stacked on the original | `off(p, n[, k])` | sugar (stack + `rotate` + `transpose`) | none needed | high: echoes, canons, octave doubles | Tidal "Time" functions (tidalcycles.org/docs/reference/time); Strudel "Pattern structure"/"Accumulation" (strudel.cc/learn/accumulation) |
| 5 | `ply` | each event repeated inside its own span | `ply(p, n)` | sugar (`*n` on every step) | none needed | medium-high: hat rolls, stutters | Tidal "Repetition" (tidalcycles.org/docs/reference/repetitions); Strudel "Time modifiers" (strudel.cc/learn/time-modifiers) |
| 6 | `iter` | the pattern starting a part later each cycle | `iter(p, n)` | sugar (alternation of `rotate`) | none needed | medium: a riff that turns over a phrase | Tidal "Time"; Strudel "Time modifiers" |
| 7 | `palindrome` | forwards, then backwards | `palindrome(p)` | sugar (alternation of `reverse`) | none needed | medium | Tidal "Time"; Strudel "Time modifiers" |
| 8 | `?` with no number | drop half, at random | `x?` = `x?0.5` | sugar (already seeded) | btw's chance hash | low-medium | Strudel "Mini-notation" |
| 9 | `struct` / `mask` | one pattern's rhythm on another's values; gating | design note | new: needs step extents | would use no randomness | medium-high | Strudel "Pattern structure"/"Conditional modifiers"; Tidal "Conditions" (tidalcycles.org/docs/reference/conditions) |
| 10 | Euclid on a value `c2(3,8)` | a note on a Euclidean rhythm | design note (falls out of `struct`) | new: as `struct` | none | medium | Strudel "Mini-notation" |
| 11 | `sometimes` / `often` / `rarely` | apply a transform to some events | design note (`either(p, q, prob, seed)`) | new: per-event choice between two patterns | btw's chance hash, by step and note | medium | Tidal "Randomness" (tidalcycles.org/docs/reference/randomness); Strudel "Random modifiers" (strudel.cc/learn/random-modifiers) |
| 12 | `chunk` | a transform on a different part each cycle | design note | new: a time window term + function arguments | none | medium-low | Tidal "Time"; Strudel "Time modifiers" |
| 13 | `jux` | a transform in one ear | design note | new: pan per event (a net field) | none | medium-low (two parts and a mix pan do it) | Tidal "Accumulation"/"Panning"; Strudel "Accumulation" |
| 14 | `range` / `segment` | continuous signals sampled to steps | design note | new term kind; `lfo` covers controls | seeded if random | low for notes, covered for controls | Strudel "Signals" (strudel.cc/learn/signals); Tidal "Oscillators" |
| 15 | `swing` / `nudge` | per-pattern shuffle and micro-timing | design note | new: timing within a part's net | none | low (song `swing` exists) | Tidal "Time"; Strudel "Time modifiers" |
| 16 | top-level `,` stacking | stack without brackets | not adopted | — | — | low; reads badly next to `|` | Strudel "Mini-notation" |

Ranked by value for writing whole tracks (how often the longhand gets typed in real programs, and how much shorter the
sugar is) over cost. Ranks 2–8 are implemented; rank 1 waits on its spelling.

## Decisions

- **No replicate mark, for now.** The first draft spelt it `x:3` (`!n`, the mini-notation's, is out: `!` is btw's accent,
  and `x!3` already reads as an accented `x` and then the degree `3`). But `x:N` is now the ratchet, an expression mark
  (N strikes inside one sixteenth step, docs/BTW.md "Expression marks"), with pinned CIDs and programs that use it, so
  `:` is taken. A replicate needs another spelling (`x;3` is one that is free in patterns); until one is chosen, three
  slots of `x` are written out.
- **`~n`, not `@n`, for elongate.** `@` already means two things in btw: a one-shot mutation line inside
  `arrange { }`, and, glued to a name, a generation of a mutated pattern (`acid_a@2`). The lexer reads `c2@3` as one
  word, "generation 3 of a pattern named `c2`", and `riff@2` could be either. `~` is the rest, and `c2~3` is literally the
  longhand compressed: `c2`, then rests, three slots in all. It counts only glued on both sides (`c2~3`); `x ~ 3`, `x~ 3`
  and `x ~3` are three steps, as before. No program in the repo or the contract corpus wrote a step, a `~` and a number
  glued together, and `fmt` always printed them apart, so no canonical text changes meaning. Tidal's other spelling,
  a lone `_` after the step, was out too: `_` is a letter of btw's names (`acid_a`), and `c2 _ _` is no shorter than
  `c2 ~ ~` (Tidal mini-notation table, tidalcycles.org/docs/reference/mini_notation).
- **`~n` pads with rests.** In btw an event is an onset and a note's length is the part's `len=`; "the step takes three slots"
  is `c2 ~ ~`. A sustaining `~n` would be a per-step length, which is not sugar (no longhand), so it is a note, not a quiet
  change of meaning. `reverse([a~3 b])` is `reverse([a ~ ~ b])`, which is `b ~ ~ a`: the rests are reversed too.
- **`~n` inside `< >` is an error**, because a term of an alternation is a bar and "more of the bar" means nothing there;
  the help says to write the term that many times (three bars of it).
- **`poly(..)`, not braces.** `{ }` are btw's blocks (`section`, `repeat`, `set`), and several passes count them; a pattern
  that opens one would be a hazard for every one of them and a puzzle to read. A call also gives the step count a place
  (`poly(8, ..)`) without a new `%n` mark.
- **`poly` and alternations inside it.** The longhand is an alternation per pattern, so an alternation *inside* one of the
  patterns turns once per round of it. This is the longhand's meaning, and is documented rather than special-cased.
- **`off` rotates.** The copy moves with `rotate`, so what goes past the bar line comes back at its start in the same bar
  (exactly right for a one-bar riff; a multi-bar riff's copy is per bar).
- **`ply` refuses calls.** A generator's events have no written steps to repeat; the error shows `transpose(ply(riff, 2), 5)`
  and writing the steps out.
- **Bounds.** Counts are 1 to 64; `poly` takes at most 64 bars per pattern; the written-out part is at most 16,384 terms
  (`sugar::MAX_TERMS`), said once at the sugar. A call that copies its pattern (`poly`, `iter`, `off`, `palindrome`) is
  charged for each copy at what it holds, so nesting multiplies the count as it multiplies the longhand:
  `poly(64, [poly(64, [x x x x x x x])])` is an error and not a wait.
- **A `let`, a `def` and a `repeat` print their sugar as typed.** Their bodies are printed from their tokens, and a
  token printer that spaced `c2~3` would turn one long note into three steps (`c2 ~ 3`): a `~` (or a ratchet's `:`) that
  touched its step and its count in the text (`$n` may be either) is printed touching, and one that did not is printed
  apart.
- **Arrange mutations work on the longhand.** A mutation rewrites the steps of a `let` pattern (`~ acid rot 1`), so the
  pattern is written out before the operators run: `c2~3 e2` is four steps there as it is when it plays, and the song is
  its longhand's under every generation. A part with a cycle of its own counts its bars written out too (`meter`).
- **The names are not reserved.** A program with a `let iter = ...` keeps working; `iter(` in a pattern is the sugar.

## Design notes for the rest

See the "Not sugar yet" list in BTW.md. The one change that unlocks the most is a **slot extent on each event** (where the
step that made it ends): `struct`, `mask`, Euclid on a value and a sustaining `~n` all need it, and none of them needs a new
kind of net, only the evaluator to carry it. The per-event choice `either(p, q, prob, seed)` would then give
`sometimes`/`often`/`rarely` without function arguments. `jux` and per-part timing are the ones that need the nets
themselves to change.
