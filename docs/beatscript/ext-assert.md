# F12: assertions, `btw test`

*Extension F12 of docs/BEATSCRIPT-EXTENSIONS.md. Code: `crates/djlive/src/assertion.rs` with small hooks in `ast.rs`, `parse.rs`,
`print.rs`, `expand.rs`, `compile.rs` (`check_report`, `Compiled::assertions`) and `highlight.rs`; the constraints (the notes) are
`crates/djlive/src/constraint.rs`, with the vocal's windows recorded in `sing.rs` (`Compiled::sing`) and a part's chance grid in
`compile.rs` (`PartIr::may`) and `chance.rs` (`ChanceCtx::all`); the measuring is
`crates/djhost/src/beatscript_test.rs`; the verb is `btw test` (registry: `djhost/src/control_registry/nouns.rs`, handler:
`djapp/src/control/live.rs`) and `djctl btw test FILE` (`djctl/src/btw_cmd.rs`). Tests: `crates/djlive/tests/ext_assert.rs`, the
module tests of `beatscript_test.rs`, `crates/djctl/tests/btw_test_cmd.rs` and `the_btw_test_verb_judges_assertions_and_measures_levels` in
djapp's `app_tests.rs`; the constraints': `crates/djlive/tests/ext_constraint.rs`. Examples: `examples/beatscript/assert.beatscript`,
`examples/beatscript/constraints.beatscript`.*

A program can say what must be true of it, and the language checks it.

```text
tempo 120
key am
section intro 8 bars { ... }
section drop 16 bars { key dm ... }
section outro 8 bars { ... }
arrange intro drop outro

assert ends                              # proved by `check`
assert length == 1m4s ± 2s               # proved by `check`: 32 bars at 120 BPM
assert key == dm in drop                 # proved by `check`
assert level A > 0.2 * B in drop         # measured: a note at `check`, judged by `btw test`
```

## Grammar (EBNF)

```ebnf
statement   = ... | assert ;                                         (* at the top of the program, never inside a block *)
assert      = "assert" ( "ends"
                       | "period" [ "(" NAME ")" ] cmp int           (* assert period == 3    assert period(drop) == 32 *)
                       | "length" cmp clock [ "±" clock ]            (* assert length == 3m30s ± 5s    assert length < 4m *)
                       | "level" levelexpr cmp levelexpr [ "in" NAME ]    (* assert level A < 2 * B in drop *)
                       | "key" ( "==" | "!=" ) key [ "in" NAME ]   (* assert key == dm in climax *)
                       | "never" subject ( "beside" | "during" ) subject [ "in" NAME ]   (* assert never A.snare beside A.snare *)
                       | subject "ends" "on" int [ "in" NAME ] ) ;                        (* assert bass ends on 1 *)
subject     = "sing" | ldeck | [ ldeck "." ] role ;           (* the vocal; a deck; one deck's part; a part on every deck *)
cmp         = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
clock       = number "m" [ number "s" ] | number "s" | number "ms" ; (* 3m30s   90s   4m   500ms: the unit touches its number *)
levelexpr   = arith over the words A, B, master and numbers ;     (* expr.rs's arithmetic: + - * / %, ( ), min(..) max(..) *)
```

`assert` is a reserved word; `ends`, `period`, `length` and `never` (right after `assert`), `ends` before `on`, `beside` and `during` (in an
`assert never`) and `in` (in an `assert`) are read as syntax there and name a `let`, a `def` or a section elsewhere (BEATSCRIPT.md "Decisions"); `level` and `key` were words before; `A`, `B` and `master` are
words of a level expression only (and `A`, `B` and `master` begin a subject of a constraint). `NAME` after `in` is a section on the timeline; after `period(` it is a section with an `evolve`, or
`evolve` for the program's own. A clock is a number with its unit glued to it; `3m30s` is the tokens `3` `m` `30` `s` (the lexer's
units), `3m 30s` reads the same, `3 m` is an error. A `±` may be glued or spaced. Seconds may have decimals (`3m10.476s`) and
`ms` is allowed alone (`± 500ms`).

## The static assertions: proved by `check`

A false one is an **error at the statement** that says the actual value, so `eval` refuses the program (the last good one plays on) and
`btw check` / `btw fmt` / `btw save` say where. A true one says nothing.

| statement | holds when | the message when false |
|---|---|---|
| `assert ends` | the program ends: an `end`, a timeline (sections and an `arrange`) with no `loop`, or a top-level `evolve` (its own timeline ends, bounded or not) | `the program does not end: nothing stops it` / `the program loops: `loop` starts the song again at its end` |
| `assert period == N` | the program's period in bars, as `meter` reports it: the least common multiple of the periods of its nets (of each section, for a song) is `N` | `the program's period is 3 bars (hihat/12 against the bar)` |
| `assert period(drop) == N` | the evolve in section `drop` has period `N` bars, from the evolve analysis (`period 4 generations = 32 bars`); `period(evolve)` is a program's own evolve | `the period of the evolve in drop is 32 bars (4 generations of 8 bars)`; an orbit that was not found says so |
| `assert length cmp clock [± clock]` | the program's length (its end bar through the tempo) compares to the clock; `==` and `!=` take a tolerance (0 when none): `|actual - clock| <= tolerance` | `the program is 2m58s` (to the millisecond: `3m10.476s`); a program that plays on or loops says `the program plays on, so it has no length` and, for a loop, the lap |
| `assert key == k [in S]` | the key in force in section `S` (its own `key`, else the program's: `Compiled::section_key`) is `k`; without `in`, the program's key (C major when none is set, and the message says so) | `the key in climax is dm` |

Comparisons: `==` `!=` `<` `<=` `>` `>=` everywhere except a key (`==` and `!=`); `length` compares with a margin of a nanosecond for
float noise, strict otherwise (`length < 4m` is false at exactly 4m). A tolerance with `<` `<=` `>` `>=` is an error (write the bound you mean).

The length is the bars the program ends on (`end at bar N`, `end after NAME`, or a song's last bar) **through F3's tempo map**
(`Compiled::length_seconds`, the same number `check`'s `length:` line says): the integral of `60 / bpm` over the beats, in closed form, so
`assert length == 3m30s ± 5s` is exact under `tempo 100 -> 140 over 6 bars`, a section's own `tempo` (and `tempo A -> B over N bars` inside
one), a timed `at bar 16: tempo ...`, and before the first tempo (120). A stack of sections is as long as the longer (F10). It is made by ONE
function, `assertion::program_seconds`, and the line says `(through the tempo curve)` when the tempo moves. A loop has no length, and says
the lap (`one lap is 15.542s`), measured through the same map.

## The level assertions: measured on a render

`assert level A < 2 * B in drop` compares what deck A and deck B are *at*, in section `drop` (the whole program without `in`). `check`
cannot know, so it records the statement as **pending**: a note (severity `info`: not an error, and not a warning, which is for what may be wrong) that says where it will be measured, and
the program is not refused. **`btw test`** measures it:

* **The verb**: `btw test {text | id, sample_rate?}` (registry verb on the `btw` noun; no `text` or `id`: the running program, else the window's)
  answers `{ok, cid, assertions, passed, failed, rendered: {bars, seconds, sample_rate, refused}, summary, results: [{line, col,
  assertion, kind: static | level, status, pass, detail, text, measured: {A, B, master}, help}]}`; a program with an error answers `{ok: false,
  diagnostics}`. **It is refused while a program is running or staged** (R-10): the render takes seconds on the thread that runs the window and the
  bar clock (7.6 s for 16 bars in a debug build), and a program that plays would miss its bar lines and fire them in a burst; `btw stop` first, or run
  `djctl btw test FILE` (a process of its own). `sample_rate` is validated as the CLI's `--sr` is (`djhost::beatscript_test::sample_rate`).
* **The shell**: `djctl btw test FILE [--sr N] [--json]` prints one line an assertion and a summary, and **exits 1 when an assertion fails**
  (or the program has an error). The lines are the contract's:

  ```text
  assert ends: the program ends at bar 32: pass
  assert length == 1m4s ± 2s: the program is 1m4s: pass
  assert level A > 0.2 * B in drop: A 0.110, B 0.129: 0.110 > 0.026: pass
  assert level A < 0.01 in outro: A 4.1e-4: 4.1e-4 < 0.010: pass
  10 assertions pass
  ```

  A failure reads the same with `: fail` (and its `help`, for a static). `btw test` re-checks the statics too (`compile::check_report`: a
  false one is a failed line here instead of a refusal), so one run says everything.
* **How a deck's level is measured** (`djhost::beatscript_test`): ONE offline render of the program (`beatscript_render::arrangement` played by the same
  `render_arrangement` as `btw render`, the engine djapp plays on) with the engine's **stems** on: the renderer already keeps each mixer
  strip's share of the master block by block (`A`, `B`, ... after the fader, the crossfader and the master level; they add up to the master
  mix). Deck `A`'s level is the **RMS of the `A` stem over the bars of the section**, `master` the RMS of the master mix. The RMS is linear
  over both channels, `sqrt(sum(l^2 + r^2) / 2n)`: about 0.7 at full scale, exactly 0 for silence. A section that is on the timeline more than once
  (`arrange verse drop verse`) is measured over all its places together. The windows are exact bar lines: the frame of bar `b` is
  `Arrangement::seconds_at(4 b) x sample_rate`, which follows the tempo the arrangement carries. Only what is needed is rendered: from bar 0 to
  the end of the last window asked for (a program with no end and no sections is 16 bars, as `btw render`'s; a looping song is one lap). Nothing reads a clock or a
  random source, so the same program gives the same numbers every time.
* **Judging** is the language's (`assertion::LevelCheck::judge`): the two sides are evaluated in floats over the measured leaves, `==` is exact
  (useful against `0`, silence, or `A < 0.01`), and the line lists the leaves read, in the order met, then the two sides.

## Constraints: the notes, decided by `check`

```text
assert never A.snare beside A.snare      # no two snares on neighbouring steps
assert never B during sing in verse      # deck B rests while the vocal is sung
assert bass ends on 1                    # every section lands on its tonic
```

The other assertions read structure (`ends`, `period`, `key`), time (`length`) and loudness (`level`); a constraint reads **notes**:
where each part's hits fall, bar by bar, in the song as it plays. It is decided by `check` like the static ones, and a false one is an
error at the statement that says the bar and the step (`assert never A.snare beside A.snare: two `snare` hits touch on deck A, in
`lift`, bar 4, steps 4 and 5`), so `eval` refuses the program and the last good one plays on; `check_report` (and so `btw test`)
keeps it as a `Fail`.

**What "decided" means here.** A program is a finite timeline of bars, and each part's hits in each bar are in the compiled program
(`PartIr::cycles`, with the arrange mutations, `repeat`, `def`, `evolve` generations and key changes already applied). `check` asks a
constraint of every step of it, by **exact enumeration**: nothing is sampled, nothing is solved. It is not a pflow proof, and the
message does not say it is: a compiled part is a one-token ring (places `p0..p31`, one transition for each hit), whose P- and
T-invariants hold for every program and say nothing about which transitions carry a snare; and the reachable markings of a ring are
its 16n positions in order, so walking them is the enumeration. The constraints depend on nothing but the text: the same verdict on
every host, with or without the voices.

### Subjects

| written | is | in `beside` / `during` | in `ends on` |
|---|---|---|---|
| `A.snare` | deck A's snare part | its hits | that part |
| `snare` | the snare on every deck | its hits | the part on every deck: the last note of any of them |
| `A` | every part of deck A | all of them | an error: say which part (`A.bass`) |
| `sing` | the vocal: every note of every `sing` line | after `during` only | an error: the vocal sings a part's notes |
| `master` | the mix | an error: it has no parts | an error |

Only parts the program wrote count: a host that sings in the nets adds a part of its own for a `sing` line (`vox`), and no
constraint reads it, so a verdict cannot depend on the machine.

### `never X beside Y`

No step where X has a hit and Y has one on the **next or the previous step**, across bar lines, across section seams and across the
turn of a `loop` (the last step of the last bar is beside the first step of the lap; a program with no sections and no `end` turns
the same way at the end of its period). Two hits on the *same* step are together, not beside: `during` is the word for those.
`never A.snare beside A.snare` is "no two snares in a row"; `beside` is symmetric, so there is one definition. A part is not beside
itself: a pair is two different hits.

### `never X during Y`

No step where a note of X sounds while a note of Y does. A note sounds from its step for its `len=` (a sixteenth for a drum, two for
a pitched part, as the nets say), and is **cut where its section ends** (the next section's nets replace it). Two different notes
of the same subject count, so `assert never bass during bass` is "the bass is one voice"; a chord is several notes at once, so it is
during itself. `during` with a part is simultaneity; for the notes of two parts on neighbouring steps use `beside`.

`sing` is the vocal's own windows, the same whatever the host: for each `sing` line, every note of the part it is sung on, from its
onset until the voice lets go of it (to the next onset less a sixteenth, at most half a bar; the last note to the end of the line's
bars, at most three beats), round and round the line's bars from where the line starts through its section. A `_` rest in the lyric
is still a held note (the safe side). A line with no time word starts with its section; `at bar N` starts it there; one timed
`every N bars`, `on next phrase`, or inside a bar, or guarded by a `when` that reads a key, cannot be told, and the constraint is an
error that names the line (`cannot tell when the vocal sings`) rather than a guess. A vocal sung on the very part the rule is about is
an error too: that part sounds whenever it sings (`the vocal is sung on deck B's own `melody` (the `sing` on line 9), so deck B
sounds whenever it sings`), name the other deck. The windows are the vocal's notes at the part's notes: a sung line follows the part
it is sung on, so a part with chance gives the notes it rolled.

### `X ends on D`

In each **place a section plays** (every time it is on the timeline; or only the ones `in NAME`), the lowest note of the part's last
onset is degree `D`, 1 to 7, of that section's own key (`Compiled::section_key`: a section with a `key` of its own is judged in it,
so `i` after a key change is the new tonic; the degrees of a mode count in that mode, and on a scale of fewer than seven notes `6`
and `7` go round into the next octave, as a degree in a pattern does). The lowest note is the root of a chord in root position and
the third of one in first inversion, as `voice-lead` voiced it: "judged as voiced" is the honest answer. A section where the part
plays no note is **skipped and named** in the message; if none is left the constraint is an error. A degree is a number, not a
numeral (`ends on i`): the case of a numeral is the quality of a chord, so `i` in a major key would mean a minor chord, and degree 1
is the tonic in every mode. A stack of sections is one stretch; an evolve's generations and the nets of a long section are one place,
ending on the last note of the last of them.

### Chance

A part with chance (`x?0.5`, `choose`, `maybe`) differs every bar, and which hits it keeps is a hash of the text: a rule judged on
that roll would be true of one roll and flip when the text changes. So a constraint is judged on what the part **may** play: a second
grid is read with every `?` and `maybe` hit kept and every option of a `choose` at once (`PartIr::may`, `ChanceCtx::all`; an option no
weight can pick is not one), and the rule must hold for it. The message says `can touch` where the pair needs a hit that only a
chance could play, and names the term (`` `x?0.5` at 4:12 ``). The options of a `choose` are one bar's alternatives, so two hits from two
options are flagged although no roll plays both: the strict side, said. `degrade` and the arrange mutations are not chance (they are
text, rolled once, and the constraint reads the text as written). `ends on` is refused for a part with chance: "which note the bass of
deck B ends on depends on the dice: put the closing note outside the chance".

### The timeline

* A program with sections and an `arrange`: the stretches of `Compiled::segments`, in order, to the bar it ends on (`end at bar N` stops
  the walk there; a later `end` lets the last section go on, as it plays). A `loop` turns at `loop_from` (the bar a looping song goes
  back to).
* A program with no sections: the decks' nets, each round and round, so one lap of them all together (the least common multiple of
  their periods and of the bars of a `sing` line), with the seam; with an `end`, bars up to it. More than 4096 bars is **cannot tell
  whether this holds**: an error, never a pass.
* Tempo and swing change seconds, not steps; mute, level, crossfader and filter change what is heard, not what is written (a muted
  deck still "plays": `assert level B < 0.01 in verse` is the rule for that).
* A rule about a subject that plays nowhere (a part the program never writes, or no note in the section named) is a **warning** on the
  statement, not a silent pass: `` `A.clap` plays no note in this program, so there is nothing for the rule to hold against ``.

### Messages

A violation names the bar and the step (counted from 0, as `at bar N` is), the section, what plays there and what would fix it, and
points at the line the notes came from:

```text
error: assert never A.snare beside A.snare: two `snare` hits touch on deck A, in `lift`, bar 4, steps 4 and 5
  = help: Bar 4 of deck A's `snare` in `lift` is `~ ~ ~ ~ x x ~ ~ ~ ~ ~ ~ x ~ ~ ~` (line 15). It comes up again in bars 5, 6, 7. To fix it: move or drop one of the two hits.
error: assert never B during sing: deck B plays `bass` (e2) at bar 1, step 6, while the vocal sings in `verse`
  = help: Bar 1 of deck B's `bass` in `verse` is `~ ~ ~ ~ ~ ~ e2 ~ ~ ~ ~ ~ a1 ~ ~ ~` (line 6). The `sing` on line 9 holds a note from bar 0, step 12 to bar 1, step 7; deck B is clear of the vocal from bar 1, step 8. ...
error: assert bass ends on 1: in `lift` (key cm) the bass ends on g, degree 5, not c, at bar 15, step 0
  = help: The last note comes from `V7` on line 13. End on c, or write `assert bass ends on 5 in lift` if that ending is meant.
```

## Canonical form

Statements print in order, one a line: `assert ends`, `assert period == 3`, `assert period(drop) == 32`, `assert length == 3m30s ± 5s`,
`assert level A < 2 * B in drop`, `assert key == dm in climax`, `assert never A.snare beside A.snare`, `assert never B during sing in verse`,
`assert bass ends on 1`.

* A subject prints as written (`A.snare`, `snare`, `A`, `sing`), a degree as a number (`ends on 1 + 1` is `ends on 2`).
* A clock prints as minutes and seconds: `90s` is `1m30s`, `180s` is `3m`, `3600s` is `60m`, `500ms` is `0.5s`; spaces between `3m` and `30s` and around `±` do not matter.
* A level expression prints with one space round a binary operator and parentheses only where precedence needs them; **constants are folded**
  (`(1 + 1) * A` is `2 * A`) and a number that is a terminating decimal prints as one (`5%` and `1 / 20` are `0.05`; `1 / 3` stays `1 / 3`).
* A key prints in the canonical pitch spelling (`Bbm` is `a#m`, `D:Dorian` is `d:dorian`).
* A statement is part of the program: its CID is of the canonical text, so an edit to an assertion is another program, as is a new order. Comments
  are not. A program with no `assert` prints, and has the CID, it always had (the fixtures pin them).

## Errors

All have a line, a column and, where a name was wrong, a did-you-mean:

* `assert lenght == 3m`: "`assert` is followed by what to assert, found `lenght`", help "did you mean `length`?" (the five words are the expected list);
* `in climx`: "there is no section `climx`", the sections on the timeline listed, "did you mean `climax`?"; in a program with no sections, "this program has no sections"; a
  section declared but left out of the `arrange` is not on the timeline;
* `period(tail)` with no evolve: "section `tail` has no `evolve`", and where the evolves are; `period(dorp)`: did you mean `drop`;
* `level C < B`, `level a < B`: "`C` is not something with a level", help "did you mean `A`?" (`mastr`: `master`); `level 1 < 2`: "reads no level"; `A / 0`: division by zero;
  `sqrt(A)`: not a function of levels; a comparison inside a side: "a comparison goes between two levels";
* `length == 3`: "a length needs a unit glued to its number"; `6db`: "not a unit of time"; `-3m`: "not negative"; `length < 3m ± 5s`: a tolerance goes with `==` or `!=`;
* `key == zz`: "is not a key" with the modes; `key < c`: keys are compared with `==` and `!=`;
* an `assert` in a `section`, `def` or `repeat`: "belongs at the top of the program" (said once however many copies a `repeat` makes);
* a false static: `assert length == 3m30s ± 5s: the program is 2m58s`, at the whole statement;
* a constraint: `A.snar` is "`snar` is not a part", did you mean `snare`; `never A.snare` alone is "followed by `beside` or `during`", `besid` did you
  mean `beside`; `a.snare` "decks are upper case"; `assert bass end on 1` did you mean `ends`; `ends on i` "a degree is a number, 1 to 7"; `ends on 9` "a degree is
  1 to 7"; `assert never sing beside sing` "`sing` is a time, not a part: it goes after `during`"; `assert never B during sing` with no `sing` line "there is no
  `sing` line in this program"; `master` "has no parts"; `assert A.kick ends on 1` "the kick has no pitch"; `assert A ends on 1` "deck A has several parts".

## Decisions

1. **An assertion is a top-level statement.** `in SECTION` says where it is about; inside a block is an error (a section's own body would make `in` redundant and the
   canonical form two ways to say it).
2. **Statics are errors, not warnings, when false** (the contract), so `eval` / `check` refuse them; **`check_report`** is `check` with a false static kept as
   `Fail` in `Compiled::assertions`, for `btw test`. A reference that names nothing (`in dorp`) is an error in both.
3. **A pending assertion is a note** (severity `info`, SYN-20): one per level assertion, in `check`'s diagnostics and the program's `warnings`, never an error.
4. **`period` with no name is the least common multiple over the nets `meter::program_period` lists** (each section's, for a song), the contract's "as meter.rs reports it".
   `period(NAME)` is **only an evolve's** (the contract), so a section without an `evolve` is an error that lists where the evolves are, not a fallback to the section's net period.
5. **A program with no end has no length.** `assert length ...` on a program that plays on or loops is false and says why (and a loop's lap); `length` is not guessed from one lap.
6. **A tolerance only with `==` and `!=`**: it is the width of "equal"; on `<` it would be ambiguous (lenient or strict), so it is an error that says how to write the bound.
7. **`ms` is a clock unit** (`± 500ms`), an addition to the contract's `m` and `s`; `±` and the units are the lexer's.
8. **Keys take `!=` as well as `==`** (`assert key != dm in outro`).
9. **Level `==` and `!=` are exact** on the measured floats. `==` against `0` is silence, which a stopped deck reads as exactly (a deck's release tail reads as a small number: use `< 0.01`).
10. **Levels are linear RMS from the render's stems** (see above), not dBFS: the contract's example (`A 0.083, B 0.051: 0.083 < 0.102`) is linear. Numbers print with three decimals,
    `4.1e-4` below a thousandth.
11. **A level assertion must read a deck** (`level 1 < 2` is an error: it would be a tautology).
12. **Assertions are not imported** (an `import` brings lets, defs and wires), and not borrowed (`<cid>.drop` brings parts, not control lines); they are part of the program they are written in.
13. **A render covers up to the end of the last window**, and `end`/`end after` bound it: a section that never plays reads 0 and the line says "nothing of it plays: the program ends at bar N".
14. **The default render rate is 44.1 kHz** (`ArrangeRenderOptions`'s), `--sr` / `sample_rate` (8000..192000 in the verb) change it; the ratios the language asks about hardly move.
15. **`in NAME` takes a section of a stack** (F10): `arrange stack(short, long)` plays `short` and `long` together in a stretch the song calls `long+short`, but `in short`
    and `in long` are the sections, so the windows of a level assertion are every stretch the section is in (`short`: bars 0 to 4; `long`: 0 to 4 and 4 to 8), and the
    sections a wrong name lists are `long, short`, not the stretches. The key of a section that sets none and is stacked with one that does is the key of the first stretch
    it plays in (`Compiled::section_key`: it follows the stretch, F10), so `assert key == dm in wash` holds when `wash` is stacked under a `key dm` section
    for its first bars and alone in `am` for the rest. Two stacked sections in different keys are F10's error, said before any assertion is judged.
16. **The level windows are the tempo map's seconds.** A section's window in the render is `Arrangement::seconds_at(4 * bar)` x the sample rate: the render of a tempo
    curve tags every event with the step it falls in, so the clock is the curve's integral at each step's start, and `btw test` reports the window it measured
    (`windows`, in seconds, in the JSON and the text) so that it can be compared with `Compiled::tempo_map().seconds_at(start)..seconds_at(end)` (`djhost/tests/beatscript_test_tempo.rs`).

17. **Constraints fold into `assert`, with no statement of their own.** `never`, `beside` and `during` are words of the extensions that mean something only in an `assert` (read by
    position, like `ends`), so a `let never = 3` still reads and no CID moves; `after` was not used (it is reserved, and in English it reads "at any distance"). No `constraint`
    statement (a second verb for the same job), no `twice in a row` (fits one rule; `beside` is symmetric), no `B rests during sing` (a second spelling of `never B during sing`),
    no numeral for the tonic (the case of a numeral is a chord's quality), no `within N steps` (a knob).
18. **Decided by enumeration, not by pflow.** `djlive` has no pflow dependency and a program is a finite schedule: enumerating its hits is exact, where an invariant of the rings says
    nothing about which transition carries a snare. The docs and messages say "decided by `check`", never "proved by pflow".
19. **Chance is judged on every roll** (the strict reading, Matt's): the part's second grid keeps every chance hit. The cost is said: the options of one `choose` can be flagged together.
20. **A false constraint refuses the program**, like every static `assert`; `check_report` keeps it as a `Fail` for `btw test`.
21. **`ends on` takes a degree, 1 to 7**, not a chord: degree 1 is the tonic in every mode, and a numeral's case is its quality.
22. **The lowest note of the last onset**, in the section's own key; a part with chance is refused; a section where the part is silent is skipped and named.
23. **A vocal that cannot be timed is an error**, not a guess; the windows are the line's meaning (through its section), the same on every host.
24. **A note is cut at the end of its section, and two hits on one step are together, not beside.** Two different hits only: a note is never during itself.

## Not done

* The window does not draw a panel of assertion states and `btw check`'s answer carries the pending notes as `diagnostics`, not an `assertions` list; `btw test` is the report.
* A deck that **comes in** at a section is started a beat before its bar line by `beatscript_render::arrangement` (it loads two beats ahead and plays one beat ahead), so the window of the
  section before it hears up to a beat of it. The measurement says what the render plays; the arrangement's timing is the render's (shared with `btw render`), not changed here.
* Control lines are in the offline render as far as `beatscript_render` puts them (docs/beatscript/ext-render.md): the mixer's (a fade, a tremolo, a mute, the crossfader), `duck` and `mix` are, so a level
  assertion measures a deck fading in; a filter, delay, reverb, crush, move or `do` call to anything but the mixer is not, and a level assertion about one measures the render without it. The report
  says how many such lines there are (`note: not in this render: 2 control lines (filter x1, reverb x1): line 10 ...`, the JSON's `not_in_render`).
* A level is measured per deck strip and the master, not per part or per instrument (`A.kick`); `A` is the deck after its fader.
* No assertion on `tempo` or on the number of parts; no `assert` inside a section for that section's own bars (use `in`). Notes: only the three constraints above (no `within N steps`, no
  `A.kick ends on i`, no chord shapes, no velocity); a constraint reads the notes as written, not the sound (a muted deck still plays).
* `degrade` is read as written, not as chance (the language does not count it as chance: it has a period); the options of one `choose` are flagged together although one plays a bar.
* A vocal that rolls on past its section on the desktop's loop row is judged only through its section (where the nets stop it), and a `sing` timed `every N bars` cannot be judged.
