# Arrange mutations

An `arrange { }` block is an arrangement that changes the program as it goes. Between the sections it plays, a
**mutation** rewrites a named pattern, and the change carries into every section after it. It is still BeatScript:
small, total and deterministic. The same text is the same track, so its CID reproduces it and a CC0 program stays CC0.

```text
#! beatscript 2
#! name: arrange-mutations
#! license: CC0-1.0
tempo 132
key am
let acid_a = a1! ~ a1 ~ c2^ ~ a1 e2 ~ a1 ~ ~ g1 ~ a1 ~
let acid_b = a2 ~ ~ c3 ~ ~ e3 ~
let hoov = a3 ~ c4 ~ e4 ~ ~ ~
let hats = ~ x ~ x ~ x ~ x
section intro 4 bars {
  deck A: kick x ~ x ~ | hihat hats
}
section build 4 bars {
  deck A: kick x ~ x ~ | hihat hats
  deck B: bass acid_a inst=fm-bass
}
section drop 8 bars {
  deck A: kick x ~ x ~ | snare ~ x ~ x | hihat hats
  deck B: bass acid_a inst=fm-bass | melody hoov | arp acid_b
}
section outro 4 bars {
  deck A: kick x ~ ~ ~
  deck B: melody hoov
}
arrange {
  intro build drop
  ~ acid_a rot 2 | accent 0.2, hoov up 2
  drop
  ~ acid_* drift 0.15
  @ hoov rev
  drop
  ~ hats thin 0.25 | fill 0.3
} loop 4 then outro end
```

This is the example program `arrange-mutations` on beatscript.app. It plays four passes of
`intro build drop drop drop`, each pass going on from the patterns the last one left, and then the outro.
The smallest one is [arrange-in-eight-lines](https://beatscript.app/examples/arrange-in-eight-lines.beatscript): one section,
played eight times, with a `~` or an `@` between the plays and three comments that say what they do.

## The block

The body is written one item to a line; commas also separate section names.

| item | means |
|---|---|
| `intro build drop` | the sections, played in order |
| `~ acid_a rot 2` | **persistent**: from here on, every section reads `acid_a` rotated |
| `~ acid_a rot 1 \| thin 0.1` | an operator chain, applied left to right |
| `~ acid_a rot 2, hoov up 3` | several targets, each with its own chain |
| `~ acid_* drift 0.15` | a **glob**: every `let` pattern whose name matches (`*` is written into the name) |
| `@ hoov rev` | **one-shot**: on top of the current generation, for the next section only; then it reverts |
| `~3 acid_a rev` | **every 3rd pass**: a count right after the mark; the line fires on passes 3, 6, 9 ... and not on the others |
| `@4 hoov rev` | the one-shot, on every 4th pass only |
| `~ control off` / `~ control on` | **the control deck**: its lines do not fire (or fire again) in the sections from here on |
| `@ control freeze off` | the control deck's `freeze` lines only, in the next section only (with `~`, `@` and counts as above) |

A mutation runs to the end of its line.

After the `}` comes the ending:

| ending | plays |
|---|---|
| nothing, or `end` | the block once |
| `loop` | for ever, each pass a new generation (a render lays out one pass) |
| `loop 4 then end` | four passes, the mutations piling up pass on pass; every pass is rendered |
| `loop 4 then outro end` | four passes, then a coda of the sections named, read with the last generation |

`arrange intro build drop` on one line is unchanged: no mutations, the same canonical text and CID as before.

### Every Nth pass

A count right after the mark, `~3` or `@4` (spaced, `~ 3`, or glued: the same program), makes the line fire only on every
Nth pass of the block. The passes are counted from 1 and the line is reached once a pass, so it fires on passes N, 2N,
3N ...: never on the first (unless N is 1), and `~1`, like no count, is every pass. N is a whole number, 1 to 256, like
`loop N`. It belongs to the whole line: every target and chain on it, and every pattern a glob names, fire on the same
passes.

```text
arrange {
  groove lift
  ~3 riff rev                  # passes 3 and 6: the arp turns round, and turns back
  groove
  ~2 bass_* drift 0.2          # passes 2, 4 and 6
  @4 kik double                # pass 4 only: the next section's kick doubles, then goes back
  lift
} loop 6 then outro end
```

- **Generations count firings**, not passes: this `riff` has generations 0, 1 (pass 3) and 2 (pass 6), so `riff@2` is the
  pass-6 text. `check` says which pass each one is.
- **A one-shot `@4`** is on the next section even across the end of a pass: a `@4` at the end of the block that fires on pass
  4 is read by pass 5's first section (or by the coda after the last pass).
- **The dice** are rolled on the pass, as always, not on the count: a `~2` line's firings on passes 2 and 4 draw differently.
  The count is in the program's text, so it is in the CID.
- **A line that can never fire** is a warning at its count, and the program still plays: `~3` with no `loop`, with `loop 2`,
  with a `loop` that has no count (it is laid out as one pass), or in a named block (its mutations run over one pass).
  `loop 3 then end` is enough for `~3`: it fires once, on the last pass, and the coda reads it.
- **`evolve every N bars`** is another clock: it counts bars inside one section. `~N` counts passes of the block.

### The control deck, per pass

A pattern is rewritten; the control deck (docs/BEATSCRIPT.md "The control deck": the `control:` lines of a section) is switched. A
switch is a line of the block of its own, with the marks and counts of a mutation:

```text
arrange {
  groove
  @3 control freeze off        # pass 3: this lift does not freeze
  lift
  ~2 control off               # passes 2 and 4: the control deck goes quiet from here on ...
  ~2 control duck on           # ... all but the duck, which keeps pumping
  groove lift
  ~ control on                 # and every line is back for the next pass
} loop 4 then outro end
```

- `~ control off` holds back every `control:` line of the sections after it, `~ control on` lets them all fire again;
  `@ control off` holds them back in the next section only.
- `~ control freeze off` switches the lines of one call: the line's first word (`move`, `swap`, `load`, `queue`, `crossfader`,
  `tempo`, `duck`, `mix`) or its registry noun (`freeze`, `vocoder`). There is no switch for one line by itself: two `move` lines
  of one section switch together.
- It reaches a section's own `control:` lines, those in its repeats and those of the defs it calls. A `control:` line outside
  every section plays the whole song, and a line with no head (`move echo_out on B`) is not on the control deck: no switch
  reaches either (a switch that reaches nothing is a warning that says which).
- A section played with lines held back is a copy without them (`drop@2`), as a section read with another generation is. The
  held-back lines are checked all the same, and they fire in the plays that do not hold them back.

`examples/beatscript/control-per-pass.beatscript` is the block above in a whole program. The long note, with the decisions: docs/beatscript/ext-control.md
"Per pass: the switch".

### A named block

Among named arrangements (BEATSCRIPT.md "Several arrangements"), one of them can be a block: `arrange NAME: { ... }`. It ends
the way a named arrangement does: `then NAME`, `loop`, `loop N`, `end`, or nothing. Its mutations run over one pass of
it. A `loop N` in the flow plays that pass N times; it does not pile up N generations (so a counted line, `~3`, never
fires in one: there is one pass to count). The other arrangements play
the sections as written. A section keeps its name and its text, and the block's mutated readings are copies after it
(`drop@2`). An unnamed `arrange { }` cannot sit among named arrangements: give it a name. A program has at most one
block.

```text
arrange open: intro drop then set
arrange set: {
  drop
  ~ acid rot 2
  drop
} loop 2
arrange close: drop outro end
```

`check` reports both: `flow` (the order `open set set close`) and `arrange` (now with `"arrangement": "set"`), its plays
placed at the first time the flow plays them.

## What a mutation changes

A mutation changes the pattern's **text**, its list of tokens, and not just the events it plays. So `check`, Follow
in the editor and the MCP tools can show the generation that is playing as BeatScript: `acid_a@2 = ~ g2 a1 ~ ...`.

- A rebinding is global for that name: every deck and every part that reads `acid_a` reads the new text.
- Generation 0 is the pattern as written. Each `~` that names it makes the next generation, pass after pass.
- A section can name a generation directly: `bass acid_a@0` is always the original, `acid_a@3` the third generation.
  Naming one the block never makes is an error.
- A pattern is changed step by step. Its steps are its top-level terms, so a group `[c2 e2]` or an alternation
  `<a b>` counts as one step. The pitch operators also reach inside groups and alternations.

## The operators

The set is closed: these, and nothing else. `n` is a whole number (signed), `p` is a share from 0 to 1, and `b` is
another `let` pattern. The examples start from `a3 c4 ~ e4 g4 ~ a4 ~` in `key am`.

### Rhythm

| op | does | example |
|---|---|---|
| `rot n` | every step `n` steps later, wrapping (`rot -1` earlier) | `a4 ~ a3 c4 ~ e4 g4 ~` |
| `rev` | the steps backwards | `~ a4 ~ g4 e4 ~ c4 a3` |
| `thin p` | each sounding step becomes a rest with chance `p` | |
| `fill p` | each rest takes the sounding step before it with chance `p` | `fill 1`: `a3 c4 c4 e4 g4 g4 a4 a4` |
| `euclid k` | the sounding steps, in order, re-spread as `k` Euclidean hits over the steps | `euclid 3`: `a3 ~ ~ c4 ~ ~ e4 ~` |
| `swap` | neighbouring steps swapped in pairs | `c4 a3 e4 ~ ~ g4 ~ a4` |
| `mirror` | the first half, then the first half backwards | `a3 c4 ~ e4 e4 ~ c4 a3` |
| `shuffle` | a seeded permutation of the steps | |

### Time

| op | does |
|---|---|
| `half` | half speed: the first half of the steps fill the bar |
| `double` | double speed: the pattern twice in the bar (up to 64 steps) |
| `stutter n` | the first `1/n` of the pattern, `n` times |
| `ratchet p` | each sounding step is played twice in its slot (`*2`) with chance `p` |
| `swing amt` | every second step later by `amt` of a step, on the sixteenth grid. It works only where a step is two sixteenths or more (8 steps or fewer); on a 16-step pattern it does nothing |

### Pitch

These move notes by scale degrees of the program's `key`, so the result stays in the key. A note that is off the
scale snaps down to a scale tone first.

| op | does | example |
|---|---|---|
| `up n` / `down n` | `n` scale degrees up / down | `up 2`: `c4 e4 ~ g4 b4 ~ c5 ~` |
| `oct n` | `n` octaves (`oct -1` down) | `a4 c5 ~ e5 g5 ~ a5 ~` |
| `invert` | turned over about the first note, on the scale | `a3 f3 ~ d3 b2 ~ a2 ~` |
| `drift p` | each note a degree up or down with chance `p` | |

Notes (`c3`), plain degrees (`1 3 5`) and written degrees (`deg(b3 5')`) all move. Chord symbols and Roman
numerals do not.

### Feel

| op | does |
|---|---|
| `vel n` | the text's loudness is its accent mark: `vel 2` accents the two strongest unaccented steps (downbeats first), `vel -2` takes the accent off the last two |
| `accent p` | each sounding step accented (`!`) with chance `p` |

### Cross-pattern

| op | does |
|---|---|
| `morph b p` | each step becomes `b`'s step at the same place with chance `p` |
| `xfer b` | `b`'s rhythm, with this pattern's notes in order |
| `mask b` | a rest wherever `b` has a rest at the same place |

`b` is read at its current generation.

### Generational

| op | does |
|---|---|
| `grow` | one rest, picked by the dice, takes the step before it |
| `decay` | one sounding step, picked by the dice, becomes a rest |

Under `loop`, they accumulate: a `~ hats decay` thins the hats a step a pass.

### Chords

These work on stacked chords, `[a3,c4,e4]`, and on `chord(am7)` steps, alone or in a `<...>` alternation: `~ pads voice 1` on
`let pads = chord(gm7) ~ chord(cm7) ~` is `~ pads voice 1` on `[g3,a#3,d4,f4] ~ [c3,d#3,g3,a#3] ~`. A `chord(...)` is written out as
the notes it plays, in the octave of the part that reads the `let` (`harmony` 3, or its `oct=`), so `up`, `down`, `oct` and
`invert` move it the same way.

| op | does | example on `[a3,c4,e4]` |
|---|---|---|
| `voice n` | inverted `n` times: the lowest note up an octave each time (`voice -1`: the highest down) | `[c4, e4, a4]` |
| `spread` | every second note, low to high, an octave up | `[a3, e4, c5]` |

### Stereo

| op | does | example |
|---|---|---|
| `jux op` | the pattern stays the **left copy**; `op` of it plays as the **right copy**: a second voice, hard right | `jux rev`: left `a3 c4 ~ e4 g4 ~ a4 ~`, right `~ a4 ~ g4 e4 ~ c4 a3` |

`~ lead jux rev`, `~ lead jux rot 3`: Tidal's `jux rev`, on any operator. What comes before `jux` in a chain works on the
pattern (on both copies, when it is already a pair); `jux op` makes the right copy from the left one and does `op` to
it; what comes after works on the right copy only. So `rot 1 | jux rev | up 2` is the left copy `rot 1`, and the right
copy `rot 1`, backwards, then up 2. A later line (`~ lead up 2`) moves both copies; a later line with a `jux` of its own
makes the right copy again from the new left one. A chain has one `jux`, and its operator is not another `jux`.

- **Two parts, one role.** Every part that reads the pattern plays both copies: the part hard left, and a twin of it
  (same role, instrument, `inst=`, `vel=`, `len=`) hard right. Each ear hears a copy at the level the plain part had,
  so `jux id`, two equal copies, sounds as the part did. A copy has its own dice: `jux thin 0.3` thins the right
  copy and leaves the left one as it was.
- **Where it works.** The part lines of a section. A pattern read through a `repeat`, a `def` or a line outside every
  section plays its left copy only (a warning says so). `bass acid_a@2` reads both copies of generation 2.
- **What it costs.** Each twinned part is a second set of voices: a deck of more than four of them is a warning. Sampled
  voices (`kit=`, `sound=cat:...`, a SoundFont) play in the middle, so both copies are heard there: use a `synth:` voice.
  `jux` on a kick or a bass is a warning too: the low end leaves the middle.
- **Elsewhere.** An `evolve` moves both copies. A `sing` line is sung on the left copy's notes. The mixer's crossfader,
  level and filters act on the pair together; the pan macros (`aux macro=hard-left`) win while they last and give the
  copies back their sides. A pitch operator under `jux` on a drum pattern does nothing, and leaves it one copy.

`id` is the identity. It changes nothing, and it is what the fixer writes in place of an operator that is not one.

## Determinism

Every chance an operator takes comes from one hash: (program CID, pass, mutation, target, operator). There is no
wall clock and no global random generator. So:

- the same program text (in any spacing, with any comments, since those are not part of the CID) is the same
  generations, the same nets and the same render;
- an edit anywhere in the program is a new CID, and so new dice: a seeded operator may come out differently.

## What check says

| it finds | it says |
|---|---|
| a target that is not a `let` pattern | an error, with the nearest name |
| an operator that is not one, or with the wrong arguments | an error; the **fixer** replaces it with `id` and keeps what was written in a comment at the end of the line, so nothing is deleted |
| a glob that matches no pattern | a warning: the mutation does nothing |
| a count the block never reaches (`~3` with no `loop`, or `loop 2`) | a warning at the count: the line never fires |
| a count of 0, a fraction, or above 256 | an error at the count |
| a pitch or chord operator on a drum pattern (no pitch in it, or read only by drum parts) | a warning: the operator does nothing there (under `jux`: it gives the pattern no right copy) |
| a `let` of `chord(...)` steps read in two octaves | a warning: its chords are voiced in the first |
| `jux` with no operator, `jux jux`, a second `jux` in a chain | an error; the fixer puts the inner operator (or the second `jux`) out of action as `id` |
| `jux` on a pattern a kick or a bass plays | a warning: the low end leaves the middle |
| `jux` on a pattern read outside a section's lines, by a sampled part, or by more than four parts of a deck | a warning: that reader plays one copy, both copies in the middle, or a lot of voices |
| a section that does not exist | an error, with the nearest name |
| `~ control NAME off` for a call no `control:` line makes | an error, with the nearest call |
| a switch beside a pattern on one line (`~ acid rot 2, control off`), or with no `on` / `off` | an error |
| a switch that reaches no line (no section has a `control:` line, or that call's lines are all outside the sections) | a warning: the switch does nothing |

The fixer's output, for `~ acid_a rot 2 | wobble 3 | thin 0.1`:

```text
  ~ acid_a rot 2 | id | thin 0.1  # fixed: was `wobble 3`
```

`check` (and `eval`, and the MCP `check` tool) report the block as `arrange`:

- `passes`;
- `unbounded`;
- `plays`: every section played, in order, with its `pass`, `coda`, `start_bar` and `bars`. Each play also has
  `reads`: the patterns a mutation touched, each with its `gen`, `once` and `text`; and, when a switch holds lines of its control
  deck back, `control_off`: the calls whose lines do not fire in that play;
- `patterns`: every generation of every mutated pattern, with the `pass` and `mutation` that made it and its `ops`
  (and `every`, when its line has a count);
- `fires`, only when a line has a count: for each counted line (a switch too), its `mutation` (the line's number among the block's
  mutations), `every`, `once`, the `passes` it fired on (from 0), and the `plays` (indices into `plays`) that first read each
  firing.

After a `jux`, a generation and a read also carry `text_right`, the right copy's text (`text` is the left copy's);
a pattern that plays one copy has none.

A report with errors carries `fixed`, the fixer's text, when the fixer has something to fix.

In the editor the header's generation chip shows the play under the playhead, for example
`pass 2/4 · acid_a@3 · hoov@2+once`. Hovering it shows the text of each generation.

## Asserting what a mutation made

A mutation can make a bar you did not mean: `fill` takes a hit into the rests after it, so a backbeat's snare can come out as two
snares in a row. A constraint (docs/beatscript/ext-assert.md) is asked of every bar the song plays, in the generation that plays it, so it
is the guard for a mutation: `assert never A.snare beside A.snare` refuses a `~ backbeat fill 0.19` that rolls a pickup beside
a hit, and the message names the section, the bar and the step (`two snare hits touch on deck A, in lift, bar 4, steps 4 and 5`) and shows the
bar as written. A section a mutation plays in another generation is still its own name for `in` (`assert bass ends on 1 in drop`
covers `drop@2` too). `examples/beatscript/constraints.beatscript` has the three constraints.

## How it runs

The block is lowered before the program is expanded. Each pattern generation that some section reads becomes a
plain `let` (`acid_a@3`). Each section that is played with different generations becomes its own copy (`drop@2`).
A section played with control-deck lines held back by a switch becomes a copy of its own too, without them.
The block itself becomes a plain `arrange intro build drop drop@2 ...`. The engine, the renderer and the phone app
play that, so nothing downstream changes. The canonical text, and so the CID, is the block as written.
