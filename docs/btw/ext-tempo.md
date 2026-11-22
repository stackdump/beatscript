# btw extension F3: tempo curves

A tempo that moves: `tempo 100 -> 130 over 8 bars`, a `tempo` inside a section, a timed line `at bar 16: tempo 128 -> 140 over 4 bars`.
The tempo is linear in **beats** (shaped by F4's shapes), so the bar clock never moves: a bar is four beats at any tempo, every
timing word of the language (`at bar 16 beat 3`, `every 2 beats`, `over 1/2 bar`, F1) is in bars and beats and means what it meant,
and what a curve changes is the **seconds**, which are an integral. Module `crates/djlive/src/tempo.rs` (the types, the parser's side,
the map and its integrals, the compiler's side), hooks in `parse.rs`, `ast.rs`, `print.rs`, `expand.rs`, `compile.rs`, `runtime.rs`;
`djhost::btw_render` follows the map; `djctl btw check` and the eval answer report the length. Example `examples/btw/tempo.btw`.
Tests: `crates/djlive/tests/ext_tempo.rs` (and the unit tests in `tempo.rs`), `crates/djhost` (`btw_render`). It builds on F1 (durations
and positions) and F4 (the shapes).

## Grammar

```ebnf
setting     = "tempo" value [ "->" value "over" dur [ shape ] ] ;     (* the program's first bar *)
section-set = "tempo" value [ "->" value "over" dur [ shape ] ] ;     (* a statement of a section, a def or a repeat: from where that body starts *)
timed       = when ":" "tempo" value [ "->" value "over" dur [ shape ] ] ;   (* at bar 16: tempo 128 -> 140 over 4 bars *)
when        = "at" pos | "after" NAME | "before" NAME dur | (nothing) ;      (* one place; not every / on phrase / on next phrase *)
shape       = "linear" | "exp" | "log" | "s-curve" | "step" int ;            (* docs/btw/ext-shape.md *)
```

```text
tempo 100 -> 130 over 8 bars exp
at bar 16: tempo 128 -> 140 over 4 bars
at bar 16 beat 2: tempo 140
section chorus 8 bars {
  tempo 140 -> 150 over 2 bars s-curve
  deck A: kick x x
}
before drop 2 bars: tempo 100 -> 140 over 2 bars
```

The API other extensions use: `djlive::tempo::TempoMap`, from `Compiled::tempo_map()`:

| call | answers |
|---|---|
| `seconds_at(bar)` | seconds from the first bar to `bar` (a `Rat`, a `u32`, a `Pos` or a `Dur`: anything that is `IntoBars`), exact bars in, `f64` seconds out |
| `bar_at(seconds)` (also `bars_at`) | the bar, a fraction of one (`f64`), that falls that many seconds in: the inverse |
| `total_seconds(bars)` | the same as `seconds_at`: the length of a song of that many bars |
| `seconds_between(a, b)` | `seconds_at(b) - seconds_at(a)` |
| `bpm_at(bar)` / `declared_at(bar)` | the tempo at `bar`; `declared_at` is `None` before the program's first tempo, `bpm_at` counts 120 there |
| `drives()` / `is_constant()` / `constant_bpm()` | whether the tempo changes in the course of the program (so the runtime sets it), or is one number |
| `staircase(beats, step)` | the tempo as constant steps whose seconds add up exactly (the offline render's) |
| `events()` / `range()` | the tempos the program says / the lowest and highest |

`Compiled::length_bars()`, `length_seconds()` and `length_report()` (`length: 3m42s with the tempo curve, 96 bars`) are what `check` says;
`Eval.length` carries it in the eval answer (`length`), and `djctl btw check` prints it.

## Semantics

* **A ramp, then a hold.** `tempo 100 -> 130 over 8 bars` ramps the tempo from the program's first bar for 8 bars (linear in beats unless
  it has a shape) and holds 130 until another tempo says. A plain `tempo N` in a section sets the tempo at that section's first bar and
  holds it until another `tempo`; a section with none keeps the tempo before it. A `tempo` in a `def` or a `repeat` is from where that
  body starts (`at bar 8: build()`, or the shifted base of `repeat 3 every 16 bars { tempo 100 + $i * 10 }`).
* **One place.** A tempo happens once, at a place. `at`, `after NAME`, `before NAME dur` and no time word are places; `every` and `on
  phrase` and `on next phrase` repeat and are an error that says so and what to write (`repeat 4 every 8 bars { tempo ... }` is the
  way to say it more than once). A tempo has no target (`deck A: tempo 120` is an error) and nothing to put back (`for 4 bars` is
  an error: end it with a later tempo).
* **The map.** The tempos of the program (the top-level settings, then every tempo line, sections included) are the events of a
  [`TempoMap`], built once when the program is compiled. An event jumps to its `from` at its bar, ramps to `to` along its shape over
  its `over`, and holds `to`. Two events at one place: the later one in the text wins. A ramp that the next event cuts short stops
  there (a warning says so: "this tempo ramp is cut off at bar 4 by the next tempo"). Before its first tempo a program says nothing:
  the decks keep theirs, and a length counts 120 (`DEFAULT_BPM`) there and says so.
* **Exact integration.** Seconds are the integral of `60 / bpm` over the beats, per piece, in closed form: a constant `a` over `L` beats
  is `60 L / a`; a linear ramp `a -> b` over `L` beats is `60 L ln(b / a) / (b - a)` (with a series where `b - a` is a hair);
  `exp` and `log` (`bpm = p + q e^(k ln2 u)` in the beat fraction `u`) integrate in closed form too, `(t - ln(bpm(t) / a) / (k ln 2)) / p` with
  `p = a - (b - a) / (2^k - 1)` for `exp` and the mirror for `log`; `step N` is a sum of `N` constants. Only `s-curve` has no closed form
  (a cubic under a reciprocal): it is 16 panels of 8-point Gauss-Legendre, exact to the last digit of a double. All of them are
  checked against a brute-force integral in the tests. `bar_at` inverts `seconds_at` by bisection per piece (to 1e-15 of a bar).
* **The law.** `bar_at(seconds_at(b)) == b` to 1e-9 for any bar, and the seconds of the bars add up to the total
  (`tests/ext_tempo.rs`, over random curves of random shapes). A tempo curve never moves a bar: every time word of the program
  compiles to the same `Timing` with or without it, and fires on the same beats of the fake clock.
* **Range.** A tempo is 40 to 300 BPM, in every place (a ramp's ends, a timed line, a section's); outside it is an error that names the
  range ("a tempo is 40 to 300 BPM") at the number. Shapes keep a ramp between its ends, so the range holds all along it.
* **The nets.** A deck's net says its tempo in its envelope (`tempo`), and so in its CID. A program whose tempo is **one number** (every
  program written before F3, and any with only a constant `tempo`) keeps exactly its nets and CIDs. A program with a **curve** makes
  each section's nets (each stretch of a net that is chained, F6) at the tempo the curve has where it begins, so a net that lands
  on its bar line is already at the tempo the curve has there, and the nets of a program that never changes its mind are not
  touched. (`Compiled.tempo` is the tempo at bar 0: what the first nets are made at.)
* **The runtime.** Each service pass the runtime sets the tempo the map has at the program's position (`LiveHost::set_tempo(deck, bpm)`,
  by default the registry's `deck tempo`: `deck {action: tempo, deck: A, bpm: 117.4}`) on every deck the program has a net on: the program's
  tempo is the master, the decks follow it. It sends only what moved (0.01 BPM) and once a bar again (a net that landed on the bar line
  starts at its own tempo, then the curve's goes on it). A program whose tempo is one number is **not driven** (no call at all, as
  before). A song that loops follows one lap's map again (the tempo comes back to the first section's); `end` stops it. `get` says the tempo
  now (`tempo`, `tempo_curve`). A host that cannot set a tempo (the call fails, or the deck answers `applied: ignored`) is told
  once in `get`'s words ("tempo curve: deck A: ...; this host cannot set the tempo, so the deck stays at its own (the bars and every
  timing word are still exact)") and is not asked again for that program; the program goes on.
* **The offline render** (`djctl btw render`, `djhost::btw_render`) follows the map: see "The offline render" below.
* **Editing a running program.** A tempo line is not a control, so the diff that says what an edit will do (`Plan`) says it in its own
  words: `tempo_changed` / the line "tempo curve changed" (and the plan is not empty) when the program's map is another one (and one of the two is not
  one number); the map of the running program is replaced as the edit lands, and the runtime goes on from the program's position.
* **What `check` says.** `length: 3m42s with the tempo curve, 96 bars` (a song, `end at bar N`, `end after NAME`; a program that plays on
  by itself has no length), `length: 1m49s at 140, 64 bars` for a constant tempo, with "(the program says no tempo)" or "(counting 120
  before the first tempo)" where the program does not say. The evolve report's seconds use the map from where the evolving section
  starts ("... ≈ 1m02s with the tempo curve" in place of "... at 124").

### The offline render

`djhost::btw_render::arrangement` (with `djhost::btw_tempo`) writes a program whose tempo moves as a **staircase** of constant tempos
(`TempoMap::staircase`): a constant stretch is one step, a ramp is cut into steps of a quarter of a beat (a sixteenth), and the tempo of each step is
the constant one that takes **exactly** the seconds the curve takes over it (`60 * beats / seconds`), so the render is as long as
`total_seconds` says, to the frame, and every event (a section's net, a deck's start) falls on its exact second. Each step is a
tempo-fader move of every deck that plays (`pitch`), with the fader's travel set (`pitch_range`) to the smallest of the engine's ±8 %, ±16 %, ±50 % that reaches
the curve, relative to the net's own tempo; and every event of the arrangement carries the tempo it falls in (`bpm`), which is what the
renderer's own clock (`Arrangement::seconds_at`) reads to put a move on its second. Two facts of the engine shape it (both found by
rendering a kick on every beat and listening for where it falls: `crates/djhost/tests/btw_tempo.rs`):

* the engine takes a net's own tempo as the deck's base **the moment the net is asked for** (two beats before its bar line), so the fader's
  steps for a section start there;
* a net **lands** on the tick before its bar line and the engine sets the deck to the new net's own tempo as it does, so the render makes each
  section's net (the first's apart) at the tempo the curve has just before its bar line: it lands exactly on the bar line, and the
  fader moves it to where the curve is from there (a jump of tempo at a section line is exact too).

The fader reaches ±50 % of a net's tempo, no more (the engine's own limit): a curve in one net that goes further (`tempo 80 -> 200`
with no sections) is held at the edge, and `djhost::btw_tempo::notes` (printed as `note:` lines by `djctl btw render`) says so and says the way out: sections,
each net made at the tempo it begins at. A program whose tempo is one number is rendered exactly as it was (no fader moves, the same
events). The test renders a kick on every beat at 8 kHz and demands every kick within 12 ms of the second the map gives its beat (it
lands within about 2), and a render that is as long as the integral, longer or shorter than the constant tempo it starts at by exactly the integral.

## Canonical form

* `tempo 100 -> 130 over 8 bars`: a plain `tempo N` prints exactly as it did; a ramp prints its two ends as numbers (arithmetic
  prints as its expression, F2) and its length as a duration (F1: `8 bars`, `2 beats`, `1/3 bars`) and a shape (`exp`, `log`,
  `s-curve`, `step 4`; `linear` is the default and is not printed).
* A timed line prints as one: `at bar 16: tempo 128 -> 140 over 4 bars`, `at bar 16 beat 2: tempo 140`. A section's `tempo` is a
  statement of the section and prints as one in it, in the place it was written.
* Random reformatting keeps the CID, a semantic edit (the shape, an end, the length, the bar) changes it; two spellings of one time are one
  program (`over 1/2 bar` and `over 2 beats`; `at bar 8 beat 0` and `at bar 8`).

## Decisions

* **Where `tempo` may stand.** The brief allowed it in a section; the reading that is uniform is "from where its body starts": a
  section, a `def` (from the call's bar) and a `repeat` (from the copy's shifted bar). At the top of the program it is the program's first bar. It
  is a statement of the section (not only its head, as `key` is): `section` bodies are lines in order and a tempo has no order to keep.
  `swing` is still refused in a block.
* **A tempo is an event, not a control.** A timed tempo line is compiled into the tempo map and is **not** a control line: it has no
  registry call and no `Control` (so no key, no hold, no undo), and `check`'s control list does not show it. The runtime's one job is
  the map. The line is still a line of the program for the editor: it is in `active_lines`, and its text is lit with the lines of its
  section.
* **A stack (F10).** A section stacked with others (`stack(groove, wash)`) says its tempo for the stretch it plays in: the stretches it is cut into
  (`groove+wash`, then `groove`) do not say it again (the carried copy of a line is not a second tempo), and each stretch's nets are made at the curve's
  tempo where it begins. Two sections that play together and set **different** tempos on one bar are an error naming both sections and the line of the first
  ("sections groove and wash play at the same time and both set the tempo on bar 0, to different things"), as two different keys are; the same tempo
  twice is one tempo. Sections one after the other never clash (a section's tempo is its own from its first bar; the program's top-level one is the default
  it overrides). The length is the longer section's.
* **A repeating time word is an error.** A tempo map is a function of the bar, built at compile time: `every 8 bars: tempo ...` would
  need the program's length to unroll (a program with no end has none). `repeat N every M bars { ... }` is total and says the same.
* **Before the first tempo.** A program that says no tempo at its first bar (`at bar 8: tempo 140` alone) leaves the decks at their
  own until it does; the map counts 120 there (the number `evolve`'s report always used) so that a length can be said, and the length
  says so.
* **The tempo of a net is the curve's where the net begins**, only for a program whose tempo is not one number (so nothing that was
  written before moves). The generations of an `evolve` carry their section's tempo (the generation's CID is of its content), not the
  tempo at their own first bar: the runtime sets the tempo over it each pass anyway.
* **A jump of tempo at a section line lands a net a hair off the second** in the live app: the engine installs a net on the tick before its bar line
  at the net's own tempo, so the one tick is as long as the new tempo says. The program's clock is the engine's beat, so nothing drifts
  and every timing word is where it was; only the second a net lands at moves (by less than a tick). The offline render makes the nets of
  a section at the tempo before its line so that there is no such shift in a render (see above).
* **`Compiled.tempo`** is the tempo at the program's first bar (None where the program says none there): the top-level `tempo`, or a
  section's that begins on the first bar. It is what the render and the nets start from.
* **Two top-level tempos** are a warning as before ("tempo is set twice; the last one wins"); two events at one place from lines
  are not (a section that sets the tempo on its first bar overrides the top-level one, which is the point of the top-level one being a default).
* **`LiveHost::set_tempo` has a default** (the registry's `deck tempo`), so the app's host and every host that has `call` follow
  the curve with no change; a test host overrides it to record the tempos.
* **Once a bar again.** A net lands on a bar line with its own tempo (its envelope's, which is the curve's there, so a hair off at
  most); the once-a-bar clear of what was sent makes the next pass put the curve's value on it, whatever the engine did.
* **A song that loops** has the map of one lap: the program position the runtime reads is the lap's (`line + frac - origin`).
* **The curve starts on the first bar you hear, for a program with no sections too** (R-02): the runtime reads the tempo map's position
  from the bar the music lands on (`Running::tempo0`, the bar line after the evaluation when a deck was playing), so `tempo 100 -> 140 over
  4 bars` sends 100 on the bar the net lands on and 140 four bars later, as the render's staircase has it, and a `tempo` line's `at bar` is
  on that count too. The program's other timed lines count from the bar it was evaluated in (`clock0`), as every program has always run
  live (docs/BTW.md "Time words"). It used to read the map from the evaluation bar as well, so a quarter of the ramp (10 BPM here) had gone
  by when the music began. Before the net lands, the first tempo is sent (the old net's deck is brought to it).
* **A ramp is not a control, so it has no progress bar** in the editor: only control lines have one. (The tempo shows in `get`.)
* **A set has a tempo map** (set.rs, docs/BTW.md "Sets and the extensions"): each track's own map where it plays alone, shifted to the
  track's place, and each mix's ramp (linear, from the outgoing track's tempo at the bar the mix starts to the incoming one's where it
  ends, or the `tempo A -> B` the mix says) over the mix's bars. It is the same `TempoMap`, so the runtime follows it with
  `follow_tempo` (and puts the decks' tempo back when the set stops), `length` and the plan read it, and a set at one tempo is a
  constant map that is not driven. A track's `until 20m` is its seconds through its own map (`bar_at`). A track's curve that is under way
  where a mix begins is cut there (the mix governs); a looping track's curve is not repeated.
* **No tempo lfo** (`tempo lfo ...`): a tempo that wobbles has no closed-form integral in the contract's sense and no use the
  curves do not cover; the grammar of F4's lfo is for control values.

## Not done

* **A finished ramp's tempo is sent again every bar** (the once-a-bar clear of what was sent) until the program stops, so a manual
  nudge of a deck's tempo is overridden within a bar. That is the program being the tempo master: to move it by hand, stop the
  program, or write a plain `tempo N` (not driven: it is in the nets). R-22; documented, not changed.

* The editor does not draw a progress bar on a tempo line, and `get` does not list the map's pieces (only the tempo now).
* The offline render holds a ramp in a single net at ±50 % of the net's own tempo (the engine's fader range) and says so; a curve
  that needs more is made of sections (each section's net is made at the tempo it begins at) or accepts the limit.
* `tempo` in an `evolve` rule, and a tempo that follows a deck (a tap, a sync): the program's tempo is the master, not a follower.
* (Done since: `assert length` (F12) reads `Compiled::length_seconds`, the tempo map's integral, docs/btw/ext-assert.md.)
