# BeatScript extension: the offline render carries the mixer's control lines

`djctl btw render` (and `btw test`'s level assertions, and a replay's render) plays a BeatScript program on the engine offline. It used to
play the program's sections, nets and sounds, and `duck` and `mix`, and **dropped every other control line**: a fade
(`deck A: level 0 -> 1 over 8 bars exp`), a tremolo (`master: level lfo sine 1 bar center=0.8 depth=0.1`) or a crossfader move was
silent in it, so a showcase render, a level assertion over a faded-in deck and a replay did not hear them. Now every control line
whose effect the arrangement format (`djrust.arrangement/1`) has an op for is in the render, at the beat the program says and with the
value the live runtime sends at that beat; and the lines it still has no op for are counted and named, never dropped quietly.

Code: `crates/djhost/src/beatscript_render.rs` (the render), `beatscript_test.rs` (the report), `crates/djctl/src/btw_cmd.rs`; small additions to
`crates/djlive/src/shape.rs` (a sampler for an lfo and the places a value leaps) and `runtime.rs` (`RAMP_EPS` public). Example
`examples/beatscript/render-controls.beatscript`. Tests: `crates/djhost/tests/beatscript_render_controls.rs` (35), `beatscript_render` and `beatscript_test` unit tests,
`crates/djctl/tests/btw_render_cmd.rs`. The engine's arrangement (`../djrust`) is not touched: nothing here needs a new op.

## What changes: no grammar, no canonical form

No syntax is added and no program text, canonical form or CID moves: this is the render of a program that already means something. The
grammar of the lines the render now carries is the language's own (docs/BEATSCRIPT.md, ext-shape.md, ext-timing.md):

```ebnf
carried     = [ when ] [ "for" dur ] ":" ( target-line | duck | mix ) ;
target-line = deck "level" change                       (* mixer {deck, volume} *)
            | deck ( "low" | "mid" | "high" ) change    (* mixer {deck, low | mid | high}: the strip's EQ *)
            | "master" "level" change                   (* mixer {master} *)
            | "crossfader" change                       (* mixer {crossfader} *)
            | deck ( "mute" | "unmute" | "play" | "stop" )   (* mixer {deck, mute}, deck {action: play | stop} *)
            | "do" "mixer" params | "do" "deck" ( "play" | "stop" | "pause" ) params ;
change      = value | value "->" value "over" dur [ shape ] | lfo ;     (* ext-shape.md *)
when        = "at" pos | "every" dur [ "from" pos ] | "after" NAME | "before" NAME dur | ... ;   (* any: ext-timing.md *)
```

```text
deck A: level 0 -> 1 over 8 bars exp                  # a fade: a sample every sixty-fourth of a bar, the exp curve to the last digit
master: level lfo sine 1 bar center=0.8 depth=0.1     # a tremolo to the end of its section
at bar 3: crossfader 0.5 -> 0.7 over 2 bars s-curve
at bar 6 for 2 beats: deck A: mute                    # muted at bar 6, unmuted two beats later
```

| line | in the render as |
|---|---|
| `deck A: level v` | `param {deck: A, name: volume, value: v}` |
| `deck A: low / mid / high v` | `param {..., name: eqLow / eqMid / eqHigh}` (the mixer's own 0..1 knob, as `mix` writes it) |
| `master: level v` | `master v` |
| `crossfader v` | `crossfader v` |
| `deck A: mute` / `unmute` | `param {..., name: mute, value: 1 / 0}` |
| `deck A: play` / `stop` (`do deck pause`) | `play {deck: A, on: true / false}` |
| a ramp, a shaped ramp, an lfo on any of the above | a run of those, below |
| `duck`, `mix` | `trick` live ops, `param` / `master` actions, as they were (ext-mix.md) |
| `for N bars` / `for 2 beats` on a mute or a play | the put-back (`unmute`, `stop`) where the hold ends |
| a performed value or `when` (F13) | read with every input at 1/2, as everywhere in a render (ext-perform.md) |

## Semantics

* **Placement.** Every event is placed in **beats** from the render's first beat (program bar 0), exactly where the program's timing
  puts it: `at bar 2 beat 2` is beat 10, `every 2 beats from bar 1` fires on every second beat from beat 4, a section's lines count from the
  section's bar. Under a **tempo curve** (F3) every event carries the tempo of the step of the staircase it falls in (`beatscript_tempo::Curve::tag`),
  so it lands on the second `Compiled::tempo_map()` gives its beat (a test compares `Arrangement::seconds_at` with the map's seconds at every
  event of a ramp and an lfo under a curve).
* **A moving value is sampled** (`SAMPLE_BEATS`, 1/16 beat, a sixty-fourth of a bar, 31 ms at 120 BPM): on the grid of absolute multiples
  of it (so the samples fall on the tempo staircase's own steps), plus the run's start and its end, whatever beat they are. The value is the
  runtime's own: `Ramp::at(t)` (so `Shape::apply`, whichever shape, and the six-decimal rounding are the runtime's), `Lfo::value_at` through
  `Lfo::sample`, which drives the runtime's own `LfoRun`. A sample is **sent only when the value moved** by what the runtime moves a knob for
  (`runtime::RAMP_EPS`, 0.002, for a ramp; an lfo's own fraction of its depth, at most 0.002), the last value of a ramp always, and **never
  when the mixer already holds it** (equal consecutive values are not repeated). So a fade of 8 bars is a few hundred events at most however fine the
  grid is, a held value is one event, and a `step 4` ramp is its four.
* **Leaps are on their beat.** The grid is not where a `step N` ramp leaps (at `k / N` of its length) or a square lfo changes sign (every
  half period, moved by its `phase`): those beats are sampled as well (`Shape::breaks`, `Lfo::leaps`), so an edge is exact, not up to a grid step late.
* **How long a run goes.** A ramp runs to its `over` and sends its last value (it goes on past its section, as the runtime's does). An lfo runs to
  the earliest of its `over`, the end of its section and the end of its hold (`Lfo::stops_at`, the runtime's own rule) and sends the value there once, unless
  that is also the instant its line fires again; with none of them to the render's end. A line that fires again (`every 1 bars: ...`) **starts a new run at
  once**: the old run is cut there, as the runtime's item has one run at a time, and sends nothing at its cut.
* **Holds and put-backs.** A hold ends where `at + for` says (a half bar, `2 beats`, a fraction) and the calls the compiler derived to put the
  line back (`Control::undo`) are the events there: `unmute`, `stop`. (A level, an EQ and a crossfader have no off, so the language refuses `for` on them,
  "a level or an EQ has no off", and never reaches the render.) At one beat, **put-backs come first, then the lines that fire, then the samples of
  moving values**: the runtime's order (a bar line's put-backs, then its lines, then the pass that moves the ramps).
* **Bars are counted from the first beat.** A program with no sections renders as if its decks started at once (the case `btw` has when nothing plays), so
  `at bar 2` is beat 8, as `duck` and `mix` have always been placed. In a flat program evaluated while a deck is playing, bar 0 is the bar the
  evaluation fell in and the music lands on bar 1; the offline render and `btw test` start the music at bar 0 (nothing was playing): a timed line of a
  flat program therefore lands one bar later, counted from the first bar you hear, in the render than it did live (`at bar 2` is the second bar you hear
  live and the third in the render). Arranged songs are not affected: both count from the first section, so the runtime comparison tests use sections.
  This is the old live count, kept on purpose (docs/BEATSCRIPT.md "Time words" and "Decisions"). The tempo is counted from the first bar you hear in both: a
  tempo curve starts on beat 0 here and on the bar the music lands on live (`djhost/tests/fix_c.rs` runs the same flat program on the runtime's fake
  clock and through the render).
* **A snapshot reads what the render has set.** A section's `mix` is put back to what the mixer held when it fired. The render keeps a record of what
  it has set (a plain value, a sample, an earlier mix), so the put-back is to that, and to the engine's starting positions (fader 0.8, EQ 0.5, master 1,
  crossfader 0.5) only for a knob nothing set. (It used to go back to the starting positions whatever the program's own lines did.)
* **Performed values** (F13) are read **1/2**: a performed line is carried as the line it is with every input 1/2 (`Performed::at_half`), so `deck A: level knob(1)`
  is level 0.5, `deck A: level 0 -> knob(1) over 4 bars` a ramp to 0.5, an lfo with `center=knob(2)` is centred on 0.5; a line whose `when` is false at
  1/2 is not in the render and is **not** listed as left out (it is not run at 1/2; it is not a limitation). Unchanged rule, wider reach.
* **A replay** (`beatscript_render::arrangement_with_replay`, `djctl btw replay OUT.wav`) has the calls of every mixer line of the program from its own run on the fake
  clock (ramps and lfos too, at the runtime's passes), so its render carries the program's sections, `duck` and `mix`, and those calls (and a deck's play and
  stop) and **not** the program's own mixer lines a second time.
* **Bounded and deterministic.** The same text is the same events, in the same order (a test renders twice and reformats the text). At most one sample per grid
  point per run, a run no longer than the render, a line no more than 4096 times (`MAX_FIRES`); nothing reads a clock or a random source. Events at or after the
  render's last beat are left out, as for `mix`.

## What is not in the render (and how it says so)

A `sing` line that starts with its section (or the program) **is** in the render: it is sung as a part of its deck's net, as the
browser sings it (docs/beatscript/phonemes.md, "In the browser"). The arrangement format has no op for an **insert** (`filter`, `delay`,
`reverb`, `crush`, `resonance`), a **move** (`move bass_swap on B`), a timed or performed `sing` line or a `do vocal ...` line,
`deck A: sync`, or a `do` call to anything but the mixer and a deck's play and stop (`do vocoder ...`, `do loops ...`). They are
silent in a render, and **counted**:

```text
note: not in this render: 4 control lines (filter x1, move x1, reverb x1, vocoder x1)
  line 7 `master: filter lp 300 -> 2k over 4 bars`
  line 8 `deck A: reverb 0.4`
  line 9 `at bar 2: move bass_swap on B`
  1 more
```

* `djctl btw render FILE OUT.wav` prints that on stderr (before the render).
* `btw test` (the verb, `djctl btw test`): the report has `note: not in this render: 2 control lines (filter x1, reverb x1): line 10 `master: filter lp 300 -> 2k over
  4 bars`, line 11 `deck B: reverb 0.4`` before its summary line, whenever a render was made and something was left out; the JSON has `not_in_render`
  (`count`, `kinds`, `lines: [{line, col, kind, text}]`, `note`), `null` when nothing was left out or no render was needed. A level assertion that the missing lines
  would change is judged on the render without them, and the note says that.
* The API: `beatscript_render::arrangement_report(&Compiled) -> (JsonValue, beats, NotInRender)` (`arrangement` is the first two); `NotInRender { lines }` with `count()`,
  `kinds()`, `headline()`, `first(n)`, `note()`, `to_json()`. One entry a line of the text (a line a `def` or a `repeat` of the same text copies is one: the def's own line; a line that came from an
  **import** has no text of its own in this program, so each call that brought it is an entry, named `at bar 5: sweep(): master: filter lp 300`), in the order of the text; its
  `kind` is the effect for an insert (`filter`, `delay`, `reverb`, `crush`), the FX unit's knob (`drive`), `move`, or the registry noun (`vocal`, `vocoder`, `deck sync`).
* A line is "left out" when any of its calls has no op, even if another call of it has one.

## btw test

`btw test` measures on `beatscript_render::arrangement`, so its level windows follow the render: **a deck that fades in over a section is measured fading in**. A test
(`beatscript_test::tests::a_level_assertion_over_a_faded_in_deck_measures_the_fade`): the same section with `deck A: level 0 -> 1 over 4 bars` and with `deck A: level 1`,
deck B at 0.7. The fade measures 1/sqrt(3) (0.577) of the constant level (the root of the mean square of a straight fade), `assert level A < B in fade` passes
with the fade and fails with the constant level. The performed note now says "only the lines a render can carry are in it (mixer levels, EQ, mute, crossfader, play and
stop, duck, mix)".

## Tests

`crates/djhost/tests/beatscript_render_controls.rs`, 35 tests: events for **each control kind at known beats** (deck level, the three EQ bands, master, crossfader, mute and
unmute, play and stop, `do mixer` and `do deck`); **a linear ramp's** 129 samples, first and last exact; **every shape** against the hand formula at every grid beat
(held value within the runtime's threshold, exact ends) and `step N` leaps on exact beats (also off the grid: `step 7` over 3 bars); **an lfo** of each wave against its
formula, with a phase, with `over` (last value once), to its section's end, a square's edges on their beats (also off the grid, `phase=0.1`), the fastest lfo the
language allows; **holds** (`for 2 beats` and `for 1 bars` on a mute, with `every`, on a play) put back exactly where they end; **sub-bar** starts and `every 2 beats`; a
ramp **cut** by its line firing again; a ramp running past its section; **a tempo curve plus a ramp, an lfo and a hold** with every event's `Arrangement::seconds_at` equal
to `tempo_map().seconds_at` of its beat, and the render as long as the integral; **the runtime's own values**: the program run on a fake-clock runtime
(`beatscript_render::runtime_controls`, passes 1/16 beat apart) and the render agree, at every pass, on what each knob holds (every shape, every wave, a mute with a hold, a
restarting ramp, two lines on one knob, **stacked and sequenced sections** (F10)), to the runtime's threshold; **what it sounds like**: a faded deck's stem against a constant one's has the ratio of the line
(1/8, 1/2, 7/8 of the way, and 15 to 1 between the first and last second), an lfo tremolo is as deep as it says, a crossfader move takes deck A out of the mix;
**determinism**; the **report** of the lines with no op (counts, kinds, lines, the note, the JSON; a program the render carries whole says nothing; a line a def copies is one;
a performed line at 1/2; a false guard is not "left out"); a ramp, a mute and a filter from an **imported** def; the example; and a bound: a 64-bar song with four lfos and two ramps is 17 thousand events and renders in about a second and a half. `crates/djctl/tests/btw_render_cmd.rs` (3): the stderr note and the example. `shape.rs` unit test for
`breaks`, `leaps`, `stops_at` and `sample`. Existing `beatscript_render`, `beatscript_test`, `beatscript_tempo`, `beatscript_test_tempo` and `beatscript_replay` tests stay green.

## Decisions

* **The resolution is 1/16 beat, not a quarter beat.** The brief suggested the tempo staircase's quarter beat ("pick the coarsest that is inaudible for levels"). At a
  quarter beat a one-bar fade is 16 steps of 6 % and a tremolo of a bar 16 steps a period, which can be heard as a stair (the engine glides a strip's gain only within one
  block, a few milliseconds, to what it was last set to). A sixty-fourth of a bar costs nothing, because a sample is only sent when it moved by the runtime's own threshold: the cost of a ramp is bounded by
  `|to - from| / 0.002` events, not by the grid. The grid is on absolute multiples of it so its beats fall on the staircase's steps.
* **Thinned by the runtime's own rule, not only de-duplicated.** "Dedupe equal consecutive values" is implied; the runtime also does not send a ramp's value until it
  has moved 0.002 (an lfo's `depth * 0.002`), and the render does the same, so the render holds what the runtime holds to that threshold at every beat: that is the law
  the tests check against a fake-clock runtime. (`RAMP_EPS` is public for it; the lfo's rule is the runtime's `LfoRun`, driven through `Lfo::sample`.)
* **The leaps of `step N` and of a square wave are sampled on their exact beat**, which the runtime itself does not (it sees them at its next pass, up to 16 ms later).
  This is a refinement of the runtime, not a divergence: at every pass the two hold the same value.
* **A knob the mixer already holds is not set again** by a moving line, but a plain line always sets (a line is a statement; a `mix` and a `deck A: level 0.5` still
  produce their events, as the existing tests count them). A ramp whose last value equals its last sample (a `step` shape: the value was reached a leap before the end)
  has no event at its end; its last event is the leap that reached the value.
* **The ramp's run is cut where its line fires again, with no end value**, and an lfo whose natural end is the very instant its line fires again sends nothing at
  the end: both because the runtime replaces the run before the pass that would have sent it.
* **A replay does not play the program's own mixer lines**: it has their calls (the run is the program). `build(c, own_lines: false)`; `mix_events` is what it carries
  of the program itself.
* **`not in this render` counts lines of the text**, not copies: a def of the same text called three times is one line (its own). A def that came by `import` has no
  line in this text, so each call is one (the import's `span` is the `import` statement; the call site is the only place of it the program has).
* **Not an error, not hidden.** A program that uses a filter still renders; the list is a note, in the places a person looks (`djctl btw render`'s stderr, `btw test`'s report),
  not a diagnostic of `check` (the live app plays the filter; nothing is wrong with the program).
* **`deck A: sync`** has no op carried: `deck {action: sync}` is "sync to the other deck", which on the render's own clock (the tempo staircase moves the fader) would fight it.
* **`play` / `stop` events** use the deck's `play` op (`on: true / false`) as the section changes already do.

## Not done

* An insert's sweep (a filter, delay, reverb or crush ramp, an lfo on one), `resonance`, a move, a vocal line, a `do` call to a noun other than the mixer and a deck's
  play and stop: the arrangement format has no op for the registry's `inserts` calls (the engine's arrangement is `../djrust`'s, and is not changed here). The render says so
  (`not in this render`). A strip's own `filter` knob (`DeckAction::Param { name: "filter" }`) is a different filter (a strip EQ's, not the `filter.sweep` insert) and
  is not used as a stand-in.
* A mixer read at a bar line (`mixer {}`) is not available offline, so a snapshot's put-back is the render's record of what it set, not the engine's value (they agree: the render
  is the only writer).
* The deck's `sync`, `load`, queue and `autodj` calls.
* An lfo faster than a sixteenth of a bar is not allowed by the language; one at the shortest period is sampled four times a period (a rough wave: the grid is 1/16 beat).
