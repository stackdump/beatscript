# BeatScript extension F4: shapes and lfo

A ramp can go another way than straight, and a value can keep moving: `master: filter lp 300 -> 2k over 8 bars exp`, `master:
filter lp lfo sine 2 bars center=3k depth=1k over 16 bars`. A **shape** maps a ramp's time `t` (0..1) to its progress; an **lfo** is
`center + depth * wave(t / period + phase)`. Both are values of every place a change takes one: a control line
(`filter`, `delay`, `reverb`, `crush`, `drive`, `resonance`, `level`, `low`, `mid`, `high`, `crossfader`), a `do` value, and (shapes only)
the tempo (docs/beatscript/ext-tempo.md). Module `crates/djlive/src/shape.rs` (the shapes, the waves, the parser's side, the compiled
lfo and the runtime's running one), hooks in `parse.rs`, `ast.rs`, `print.rs`, `compile.rs`, `docall.rs`, `runtime.rs`,
`highlight.rs`. Example `examples/beatscript/shapes.beatscript`. Tests: `crates/djlive/tests/ext_shape.rs` (and the unit tests in `shape.rs`),
`crates/djhost/tests/beatscript_tempo.rs` (the real registry). It builds on F1 (durations: a period is `2 bars`, `4 beats`) and F2
(every number may be arithmetic).

## Grammar

```ebnf
shape       = "linear" | "exp" | "log" | "s-curve" | "step" int ;
change      = value                                              (* a plain value *)
            | value "->" value "over" dur [ shape ]              (* a ramp, linear unless it says *)
            | lfo ;
lfo         = "lfo" wave rate { "depth=" value | "center=" value | "phase=" value } [ "over" dur ] ;
wave        = "sine" | "tri" | "square" | "saw" ;
rate        = dur ;                                              (* the period: 2 bars, 1/2 bar, 4 beats *)
do-value    = ... | lfo ;                                        (* value=lfo tri 1 bar depth=0.2 center=0.3 *)
do-line     = "do" NOUN [ VERB ] { param } [ "over" dur [ shape ] ] ;   (* a ramp's shape comes after its `over` *)
```

```text
master: filter lp 300 -> 2k over 8 bars exp
master: reverb 0.1 -> 0.6 over 8 bars s-curve
deck B: level 0 -> 1 over 4 bars step 4
master: filter lp lfo sine 2 bars center=3k depth=1k over 16 bars
deck A: level lfo saw 4 beats center=0.8 depth=0.1
do vocoder set mix=0 -> 1 over 8 bars log
do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3 over 8 bars
```

## Semantics

* **Shapes.** With `t` the fraction of the ramp's length gone (0..1, clamped), the ramp's value is `from + (to - from) * shape(t)`:

  | shape | `shape(t)` | feel |
  |---|---|---|
  | `linear` | `t` | the default, never printed |
  | `exp` | `(2^(4t) - 1) / 15` | slow start, fast end: a riser |
  | `log` | `1 - exp(1 - t)` | fast start, slow end: its mirror |
  | `s-curve` | `3 t^2 - 2 t^3` | gentle at both ends |
  | `step N` | `floor(t N) / (N - 1)`, at most 1 | `N` values, held, `N - 1` leaps; `2 <= N <= 1024` |

  Every shape is 0 at `t = 0` and exactly 1 at `t = 1`. By hand, `exp` is 1/15, 1/5, 7/15 of the way at a quarter, a half and three
  quarters; `log` 8/15, 4/5, 14/15; the `s-curve` 5/32, 1/2, 27/32; `step 4` is 0, 1/3, 2/3 and 1 held a quarter each (the tests
  check each against a hand formula written out in the test). A ramp sends only what moved (`RAMP_EPS`, 0.002), so a `step N`
  ramp sends `N` values (and the end value again when it ends), and an `exp` ramp sends fewer at its start. The shape of a
  filter's ramp is of the knob, as its linearity always was.
* **Lfos.** The value is `center + depth * wave(2 pi (t / period + phase))` in the field's own unit (a filter's knob; see below), `t`
  the time since it started, in bars. Defaults: `depth` 1, `center` 0, `phase` 0 (a share of one period, 0..1). The four waves are the oscillators':
  all rise from the midline at phase 0 (a square starts at the top), so `phase=` means the same for each.

  | wave | `wave(p)` for `p` periods in | at p = 0, 1/4, 1/2, 3/4 |
  |---|---|---|
  | `sine` | `sin(2 pi p)` | 0, 1, 0, -1 |
  | `tri` | `(2 / pi) asin(sin(2 pi p))` | 0, 1, 0, -1 (straight lines between) |
  | `square` | 1 for the first half, -1 for the second | 1, 1, -1, -1 |
  | `saw` | `2 (p - round(p))`: rises to 1 at the half, falls to -1, rises to 0 | 0, 1/2, -1, -1/2 |

* **How long.** An lfo runs `over dur` when it says; else **until its section ends** (a line in a section stops where the section does:
  the runtime knows the section's end); a line outside every section, or in a program with no sections, until the program stops or
  ends. It stops with the line's hold too (`at bar 4 for 2 bars: master: filter lp lfo ...` stops after two bars, when the filter is put
  back). A span's last pass sends the value at the end of it, once (two whole periods in, a sine is back at its centre); a program
  that ends takes its lfos with it on the pass before.
* **Sent like a ramp**, each service pass, only when the value moved (`0.002`, or a fraction of the depth where that is smaller, so a
  depth of 0.01 is still drawn): the same `inserts` / `mixer` / `do` call a ramp would make, with the value in the field.
* **A filter's lfo swings between two knobs.** `filter lp lfo sine 2 bars center=3k depth=1k` goes between 2 kHz and 4 kHz as a
  ramp between them goes: linear in the filter's knob, so a quarter period in it is exactly at 4 kHz and three quarters at 2 kHz.
  A `filter hp` is the same in its own knob.
* **Out of range.** `center - depth` and `center + depth` must be in the param's range (a low-pass cutoff 60 to 20000 Hz, a high-pass 20 to 8000,
  everything else 0 to 1): an lfo that goes outside it is **held at the edge** (the contract says clamped) and a warning says so ("this lfo
  swings from -1 to 1, outside the range of `delay` (0 to 1): it is held at the edge"). The default depth is 1, so say `center=` and `depth=`. **A filter's lfo with no `depth=` is a warning** (L-17: `filter lp lfo sine 1 bar center=3k`
  swings 1 Hz either side of 3000, which is no movement): "this lfo has no `depth=`, so it swings 1 either side of its centre: next to a low-pass
  cutoff in hertz that is no movement at all", with `depth=1k` as what to write. The warning is for the targets whose range is wider than 1 (the cutoffs);
  a 0..1 knob with the default depth is already said when its swing leaves the range. Decision: warn, not a default by target (`depth=` half the
  range), because the contract's default is 1 and a program that left it out would change its sound.
* **`do` lines.** `value=lfo tri 1 bar depth=0.2 center=0.3` is the one moving param of the line (a line can ramp or have an lfo, one of
  them: "one param a line can move"); the lfo's own `over` ends it, else the line's, else its section's end. A word after the lfo's
  params that is `NAME=` and not one of the three belongs to the line (`mix=lfo sine 1 bar depth=0.5 level=0.7`). The value must be a number param.
  A shape after a `do` line's `over` shapes its ramp: `mix=0 -> 1 over 8 bars exp`.
* **The editor.** `Marks` controls carry `progress`: a shaped ramp's is its time (0..1, as a linear ramp's); an lfo's is how far through its
  span it is (`over`, its section's end), or through its period when it has no end. `get` calls a running lfo `ramping`. `lfo`, the waves
  and the shapes are keywords in `highlight.rs` where they are read as syntax, and `depth` `center` `phase` before an `=` are params. `lfo` and `s-curve` are
  reserved words (no `let lfo`); the waves, `exp`, `log`, `step`, `linear`, `depth`, `center` and `phase` are read as syntax after `lfo`, after a duration
  and before `=`, and name a `let`, a `def` or a section elsewhere (BEATSCRIPT.md "Decisions").
* **Errors** carry a line and column and a did-you-mean: `lfo sin` says `did you mean sine?`, `triangle` says `tri`, `expo` and `scurve` say `exp` and
  `s-curve`, `deph=` says `depth`. A shape on something that does not ramp: "`exp` needs a ramp: `0.4 -> 1 over 4 bars exp`"; a ramp with a shape and
  no length: "`exp` needs the ramp's length: `over` is missing". A depth below 0, a phase outside 0..1, a period of 0 or under a quarter of a beat ("faster than the
  clock"), a repeated param, `step 1`, an lfo on a param that is not a number all say what is wrong and what would do.

## Canonical form

* A ramp: `master: filter lp 300 -> 2000 over 8 bars exp` (the shape after the length; `linear` is the default, so `over 8 bars linear` and `over 8 bars` are
  the same program and print without it). A plain value and a linear ramp keep their old tree (`Action::Change`), so nothing written before F4 moves.
* An lfo: `lfo sine 2 bars center=3000 depth=1000 over 16 bars`: the wave, the period as a duration (F1: `2 bars`, `2 beats`, `1/3 bars`), the params **sorted by name**
  with a param that is its default (`depth=1`, `center=0`, `phase=0`) **left out**, then `over`. Two spellings of one lfo are one program; any other
  value is another.
* A `do` line: `do vocoder set mix=0 -> 1 over 8 bars exp`, `value=lfo tri 1 bars center=0.3 depth=0.2` (params of the call sorted by name as ever).

## Decisions

* **The tree.** A ramp with a shape or an lfo is a new variant (`Action::Shaped { name, mode, motion: Motion }`, `Motion::Ramp | Motion::Lfo`; `DoValue::Lfo`;
  `DoCall.shape`), at the end of its enum; `Ramp` gets a `shape` field (`Shape::Linear` for everything that was) and `Effect::Lfo(Lfo)` is a new
  effect. The runtime keeps an lfo in `Item.lfo` (a `LfoRun`), beside `Item.ramp`.
* **An lfo is compiled through a ramp.** `Compiler::shaped` makes the call a ramp from `center - depth` to `center + depth` would make (so the range checks, the
  unit of every param and a filter's prelude are the ramp's, once) and takes its two ends in the field's own unit: `center = (a + b) / 2`, `depth = (b - a) / 2`. A low-pass's
  knob falls as its cutoff rises, so `depth` is negative there and the wave still goes up with the cutoff.
* **Waves start on the midline** (a family with the oscillators' phase) rather than `saw` from the bottom: a filter that starts at its centre does not jump when the lfo begins.
* **Defaults are left out of the canonical text** (`depth=1`, `center=0`, `phase=0`): the contract only says params are sorted; two spellings of one
  meaning being one program is the contract's own rule for everything else.
* **Out of range is a warning and a clamp, not an error**: the contract says "clamped by the registry/knob". A `do` lfo is not checked (the registry clamps it).
* **The lfo's section end is the compiler's:** `Compiler.seg_end` is the end of the section whose lines are being compiled (the song's bar, from its first bar), and
  becomes `Lfo.until`; the runtime adds the bar the song landed on. A line outside every section has no end.
* **In a stack (F10)** an lfo with no `over` ends where **its own section** ends: in `stack(groove, wash)` with `groove` 8 bars and `wash` 4, an lfo of
  `groove` (said in the first stretch, bars 0 to 4) goes on to bar 8; a section's own lines are found through the copies the stretches carry
  (`mixmoves::section_span`), the way a mix is held. A `do` line is the same.
* **A mix item takes no shape and no lfo** (`mix { A level 0 -> 1 over 2 bars exp }` is an error that points at the control line to write): a mix is one
  number set once (docs/beatscript/ext-mix.md).
* **The shortest period is a quarter of a beat** (1/16 bar): the runtime looks at the position about sixty times a second.
* **`step N` is bounded at 1024** (a ramp sends one value a step) and is at least 2 (one step has nothing to step to).
* **`do` words with a dot** (`param=delay.mix`, the contract's own example) were not read at all (the lexer cuts at the dot); `spec_word` joins `word . word` now, so they are one word again.
* **No shape on an lfo, no lfo on a tempo.** A shape is for a ramp; an lfo has a wave. (`tempo` takes a ramp and its shape: F3.)
* **An edit that moves a section's length** under an lfo that is running (the line's text has not changed, so it keeps its state) does not move where
  that lfo stops until the line fires again: it ends where it was told when it started.
* **Final value.** An lfo with an end sends its last value once; a ramp of the last section does not get that either (the program ends first): same for both.

## Not done

* A shape on a `move ... over` (its length is the registry's bars) and on `swap pairs`.
* An lfo of a tempo, a pattern or a part param (`vel=lfo ...`): control values only.
* A random wave (`noise`, sample-and-hold) would need a seed in the text; the four waves are deterministic by their formulas.
* (The offline render carries a ramp, a shape and an lfo on a mixer knob, sampled with this module's own maths: docs/beatscript/ext-render.md. A filter, delay, reverb or crush sweep is an insert, which the arrangement has no op for, and is not in it.)
