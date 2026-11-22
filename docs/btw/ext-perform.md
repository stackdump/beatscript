# btw extension F13: the performed tier

`knob(3)`, `pad(5)`, `fader(A)`, `held(key)` and `held(c4)` read the controllers **while the program plays**, in a control value or a
control line's `when`. Every value read is logged, by program bar, into a **performance**: an object with its own CID that the same
program replays, call for call. Module `crates/djlive/src/perform.rs` (the inputs, the expression over them, the log, the
performance), small hooks in `parse.rs`, `expr.rs`, `expand.rs`, `compile.rs`, `docall.rs`, `runtime.rs`, `env.rs`; the controllers'
side in `djhost::btw_inputs`, the store in `djhost::live_store`, the verbs in `djhost::control_registry` and `djapp`'s
`control/live.rs`, the offline replay in `djhost::btw_render` and `djctl btw replay`. Example `examples/btw/perform.btw`. Tests:
`crates/djlive/tests/ext_perform.rs` (and the unit tests in `perform.rs`), `djhost` (`btw_inputs`, `live_store`, `btw_render`),
`djctl/tests/btw_replay.rs`, `djapp` (`app_tests.rs`: `performed_inputs_read_what_the_bindings_see`,
`a_performance_is_kept_and_replayed_through_the_btw_verbs`).

## Grammar

```ebnf
input       = "knob" "(" int ")"                     (* the N-th knob of the connected bank, N from 1 *)
            | "pad" "(" int ")"                      (* a pad: 0, or its velocity 0..1 while it is held *)
            | "fader" "(" deck ")"                   (* deck A's or B's channel fader *)
            | "held" "(" "key" ")"                   (* the lowest MIDI key held, over 127; 0 when none *)
            | "held" "(" note ")" ;                  (* 1 while that key is held, else 0: held(c4) *)
value       = arith ;                                (* a control value may name inputs: 200 + knob(3) * 7800 *)
guard       = "when" expr ;                          (* on a control line: when held(key) and knob(2) > 0.5 *)
```

```text
master: filter lp 200 + knob(3) * 7800                       # a cutoff from a knob: 200 Hz down, 8 kHz up
deck B: level 1 - fader(A)                                    # the other deck's fader the other way
master: reverb 0.1 -> 0.2 + knob(2) * 0.6 over 4 bars        # a ramp whose end is a knob, read again at every bar line
do inserts set param=delay.mix target=Master value=knob(2)    # a `do` param
every 1 bars: master: delay 0.5 when held(key)               # fires only when due, and a key is down
every 2 bars: deck A: low 0 when pad(5)
at bar 8: master: reverb 0.9 when held(c4) and knob(1) >= 0.5
```

* **An input is a number in 0..1,** quantised to 1/128 when it is read (`round(v * 128) / 128`), so a value in a log is exact.
  `knob(3)` is the third knob (1 is the first), `pad(5)` a pad, `fader(A)` a deck's channel fader, `held(key)` the lowest MIDI key
  held divided by 127 (0 when none: note 0 cannot be told from none), `held(c4)` 1 or 0 for one key (octave 4 is middle C, 60;
  `held(C4)` and `held(c4)` are one input).
* An input is an expression's leaf: `+ - * / %`, parentheses, `min` / `max`, comparisons, `and` `or` `not` all work over it. A guard
  is true when it is not 0, so `when held(key)` is "a key is down", `when knob(2) > 0.5` a condition, and
  `when held(c4) and fader(B) < 0.1` both. `and` / `or` read both sides, and `min` / `max` every argument, so the inputs a line names
  are the inputs it reads whatever their values. A division by zero reads 0 (a program that plays does not stop).

## Where an input may stand

**Control values and control guards, and nowhere else.** A net is compiled once per section, so an input in a pattern is an error;
the same for every number the program needs while it is read. The error is at the call (line and column) and says what to write
instead:

```text
error: `knob(...)` is a performed input: it is read while the program plays, so it cannot stand here
  --> 1:21
  |  deck A: kick x*(1 + knob(2))
  = help: an input stands in a control value (`master: filter lp 200 + knob(3) * 7800`) or in a control line's `when` (`when held(key)`);
          a pattern is compiled once per section, and a count, a time, a tempo or an `evolve` rule's number is fixed when the program is
          read: drive a control line from the input instead (`deck A: level fader(B)`)
```

* a pattern step, a generator's argument, a part's param (`vel=knob(1)`), a chance (`x?knob(1)`), a count (`x*knob(1)`, `repeat`),
  a time (`at bar knob(1)`, `every knob(1) bars`, `over knob(1) bars`), `tempo` / `swing`: the error above (for a setting it begins
  `expected a tempo in BPM:`);
* a part line's `when` (`deck A: kick x when held(key)`): "an input cannot decide whether a part plays", and the way out;
* a call of a def (`wobble() when held(key)`): "a call of a `def` takes no performed guard": put the `when` on the control lines
  inside the def, where the input is read when each line is due;
* a `let` (`let cut = 200 + knob(3) * 7800`): "a `let` is put in place while the program is read": write the expression where it is
  used, or pass it to a `def` as an argument (`sweep(200 + knob(3) * 7800)`, which keeps its grouping where `$a * 2` stands);
* an `evolve` rule's number: **deferred, and an error** ("cannot stand in an `evolve` rule"): every generation is made from
  generation 0 and the rule alone, and `btw generation` says any generation's CID from the program (a generation a million bars on
  costs the length of the orbit), which a value read from a controller would break. A log could not make that law sound: the CID of
  generation N would depend on a performance, not on the program.

Inputs go through `def`, `repeat`, imports and wired ports like any other token (`tests/ext_perform.rs`): a def argument that has one
keeps its grouping (`(100 + knob(3) * 3900) * 2`), `$i` and `$n` are put in place before the call is read (`knob($i + 1)`).
A wired port carries compile-time values (a `tempo` port is one number: an input there is the error above); a def with ports may
have control lines that name inputs.

Other errors, all with a place: `knob(0)` ("numbered from 1"), `knob(129)` ("a program may use up to 128 knobs": the same on every
machine), `pad(129)`, `knob(1.5)`, `knob()`, `fader(C)` (expected A or B; `fader(a)` says deck names are upper case),
`held(c)` ("a note needs an octave"), `held(x)`. The range is 1..=128 knobs and 1..=128 pads (`InputBank::MAX`: a MIDI channel has 128
controllers and 128 notes), whatever is connected: **whether a program checks never depends on the machine** (`eval`, `check`,
`fmt`, `share`, `open`, `test`, and `LiveStore::check` for `save`, `import` and `jsonld` all agree, R-06). The connected controllers'
own bank (`Env::bank`, `Board::bank()`, shown by `btw perform`) is a run-time matter: a `knob(12)` on a controller with eight knobs
reads 1/2 while the program plays, and `btw get` says "no controller is connected for it: it reads 1/2", as for an input nothing answers.

A value that cannot be in range is an error (`master: level knob(1) + 2`: "always outside the range of `level`: it is 2 to 3 and the
range is 0 to 1"); one that sometimes is, is held to the range when it is read and a note says so (`can be 0 to 2: it is held to 0 to
1`). A cutoff is held to 60 Hz..20 kHz (low-pass) or 20 Hz..8 kHz (high-pass), a level, a send, a delay ... to 0..1. A `do` param has
no range of its own (the registry checks the call); a whole-number param (`bands=2 + knob(1) * 14`) is rounded when it is read; a
param that is not a number cannot take an input.

## Follow: a value line follows its input (R1)

`master: filter lp knob(1)` is the whole of "knob 1 opens the filter". A control line with a **plain value** (not a ramp or an lfo,
which re-read their inputs at every bar line already) that **names an input** and has **no time word** fires on **every bar line from
its first**, not once, and sends a call only when the value it reads has moved (a step of 1/128, after the curve and the rounding:
what the call would carry is compared with what the line last sent). Where it follows:

* at the top of a program: every bar from its first, for as long as the program plays (and again from each lap of a `loop`);
* in a `section`: each bar of it, and it stops with it; what it last sent stays, as a one-shot's does;
* in the body of a `def`, when the call has no time word either (`wob(A)`); a call with one (`at bar 8: wob(A)`) times its lines, which fire
  once, there. The same for a line in a `repeat`: its copies are timed by the repeat.

A line with a time word (`every`, `at`, `on phrase`, `on next phrase` ...), a `for N bars` line, a ramp, an lfo, and a line whose only input is a
`when` (nothing to follow: `master: level 0.5 when held(key)`) **keep the meaning they had**. A line that follows and has a `when` follows
while it holds: when the guard turns false the words say `bar 7: not run, its `when` is false: ...` once, and the line goes on where the
knob is when it holds again. The words say that a line that follows ran once, not at each turn of the knob.

Edits keep the state a line has (its last sent value is kept while its text is as it was), and a lap of a looped song sends it again.

## When it is read (R3)

* **When the line is due.** A performed line is decided when it is due, with the value of each input **where it is due**: a line that
  fires on bar lines reads at the bar line, `every 1/16 bar:` reads sixteen times a bar, `at bar 2 beat 3` reads where the beat is. A read
  is asked of the host the first time at that place (the bar and a 1/64 of it) and kept while the program is there, so a line, a ramp and a
  guard that are due together and name `knob(3)` agree. The rate of a program is the time word it already has: `every 1/16 bar:` is a worst
  lag of 121 ms at 124 BPM, `every 1 beats:` 0.48 s. (Instant, under 30 ms, is a booth binding's job: it acts on the engine, and
  the session recorder journals it.)
* **A ramp that names an input** (`master: level fader(A) -> 0 over 4 bars`) reads it again at every bar line and keeps the value
  inside the bar: within the bar it moves from the bar's start to the bar's end as a ramp does, between the ends it read.
* **A `when` that is false when the line is due** means the line does not fire this time; a one-shot line (`master: level 0.5 when
  held(key)`, due on the first bar line) is not retried when the key comes down later. The idiom for "while a key is down" is a
  repeating line: `every 1 bars: master: level 0.5 when held(key)`, or a value line that follows (above). A `for N bars` line that did not fire has nothing to put back.
  The words of `btw get` say `bar 7: not run, its `when` is false: ...`.
* **No controller, or a knob nothing has moved (R4).** `LiveHost::input` is the host's side (default `None`): an input nothing answers for
  reads its **home**: **1/2 for a knob**, 0 for a pad, a key and a fader that is not there. The words say once `knob(4): no controller is
  connected for it: it reads 1/2` (`btw get` carries it in `warnings`). Nothing is logged for a value that did not come. A knob that is on a
  device but has not moved is at home too (`Board::read`): the middle, the number `check` reads it as, so a performed filter does not start
  at its lowest cutoff, and a program shared to a page (which has no controllers) plays with its knobs in the middle.
* **A cutoff from an input alone (R2).** An input written alone as the value of `filter lp` or `filter hp` (or as the end of its ramp)
  covers the control's range on the filter's own log curve, up being a higher cutoff: low-pass 60 Hz at 0 to 20 kHz at 1, high-pass
  20 Hz to 8 kHz (`compile::sweep_span`). Any arithmetic keeps its unit, hertz, and today's error: "always outside the range", whose help
  adds "or write the input alone to cover the range".
* **check** reports the program's inputs: `performed: knob(3), fader(A), held(key) (read when its line is due)` (a note, severity `info`, on
  the first performed line). `btw get` has `performed`: the inputs, whether it is logging, how many values (and the bars they span). The take's **CID** is the `perform`
verb's (`btw perform`): it hashes the whole take, which can be a quarter of a million readings, and `Live::status` is looked at every
frame by the window, so it does not (R-07; the window's own lines read `staged_cid`, `staged_lands` and `generation`, and the net CIDs
`status` says are worked out once, when the program is evaluated).

## The performance

```json
{"format": 1, "type": "djrust.performance", "program": "z4EBG9j...", "inputs": [{"bar": 1, "input": "knob(3)", "value": 0.5}, {"at": 16, "bar": 2, "input": "knob(1)", "value": 0.25}, ...]}
```

A **performance** is the values the inputs of one program read (at most 262,144 of them: a log that reaches that says so once in
`btw get`'s words, `performed` has `log_full`, and the saved object carries `"truncated": true`, so a cut take is not taken for a whole
one; every other performance's CID is as it was): `program` is the program's CID (of its canonical text), `inputs` the
readings `{bar, input, value}` and, when it was not read on the bar line, `at` (the 1/64 of the bar, 1 to 63: `16` is a quarter of the way in), sorted by bar, then
`at`, then input, one for each bar, place and input. A reading of the bar line has no `at`, so a performance made before there was one is the same object (the same CID) and replays as it
did: a replay looks a reading up by bar and place, and failing that takes the bar line's, so an old take of a program with a line that fires inside the bar answers each firing with the bar's value. The bar is the **clock's bar** (the one `btw get`
counts: 0 is the bar the program was applied in, so the program's own bar 0, the first bar it plays, is bar 1 here; a value past the
program's `end` is refused by `djctl btw replay`, and a bar above 1,048,576 by the reader). `input` is the language's own spelling (`knob(3)`, `fader(A)`, `held(c4)`), `value`
a multiple of 1/128. Its **CID** is the share CID of the canonical JSON (sorted keys), the scheme a program's is, so it is stable
(the order the values were read in is not part of it), and a changed value is another performance (and a different CID). A
performance file that does not read back to its own CID (an edit) is not listed and not replayed; one whose type, input names,
values or bars are not well formed is refused with the reason.

* **A take** is what a running program has logged. Logging is on from the program's first bar; `perform {do: start}` begins a new
  take now (the last one is kept), `{do: stop}` stops logging (the values are still read, and the lines run), `{do: save}` keeps the
  take as `<cid>.perf.json` next to the programs in the store, `{do: list}` lists the saved ones. A take belongs to one program:
  when another program is applied the take ends (kept as the last one, so it can still be saved) and a new one starts. A take that
  begins in the middle of a program (`start` at bar 40) replays from the program's bar 0 and reads the input's home (1/2 for a knob, else 0) where it has no value, with a
  word saying so: a performance is by program bar.
* **Replay.** `Live::replay(performance)` (the verb `btw replay {id}`; `djctl btw replay`) makes the inputs read the log, by bar,
  **instead of the controllers**: the host's `input` is not asked. The same program on the same clock makes the same calls, in the
  same order, with the same numbers (`tests/ext_perform.rs`: a live run with scripted controllers against the replay of its log, on a
  fake clock, equal call for call, over a property loop of programs and scripts; the replay's own log is the same performance, CID
  for CID). A log of **another program is refused** (`refused: this performance is of program z..., and the program that plays is
  z...`), and so is a replay with nothing running. The verb stops what plays, evaluates the performance's program afresh (the log is
  by program bar, and a program counts from its first), and replays; the program is found in the store by the CID the performance
  names (an unsaved program is said: `btw save` it). Evaluating another program ends a replay (`replay ended: another program is
  running`).

## The controllers (`djhost::btw_inputs`)

The values are the ones the bindings see: every message of a plugged-in controller, a control drawn in the Map window or the computer
keyboard takes `hwio::midi_in`, and its decoded value is noted in `App::activity` (what the Map draws its glow and its last values
from). `Board` only **reads** that, so the Aux and APC layouts, the bindings and what a control is bound to are unchanged: a knob
that is bound to the reverb still moves the reverb, and a program that names the same knob reads where it is. Which control a name
means is a rule over the devices attached, in port order:

| input | the control |
|---|---|
| `knob(N)` | the N-th control of the first attached device's `knobs` group (the MPK mini's eight), else of its `faders` group (the APC mini's nine: faders 1-8 and the master) |
| `pad(N)` | the N-th control of the first attached device's note pad group: `pads` (the APC's 64, the PD705's 9) or `padNotes` (the MPK's sixteen, note mode). While it is held its velocity (at least 1/127), else 0 |
| `fader(A)` | deck A's channel fader: the mixer's volume (`mixer deck=A volume=`), whichever control moved it |
| `held(key)` | the lowest key held of the first attached device with a 128-note `keys` group (the MPK), over 127 |
| `held(c4)` | 1 while that note is held on the same keys |

A device that is attached and untouched reads 0 for a pad and a key, and **1/2 for a knob** that never moved (R4: a knob is at home in the
middle); a name no attached device has a control for is `None` (the program reads the input's home, 1/2 for a knob, and says so). `Board::bank()` is how many knobs and pads the attached devices have (a kind none has stays the
default), and `App::btw_bank` hands it to every `check` and `eval` (`Env::bank`) and to `btw perform`; `check` does not consult it
(R-06: it used to, so `knob(9)` was an error with an MPK attached and fine without, and a program saved on one machine could fail to
load on another), so `knob(9)` on an MPK checks and reads 1/2. A device drawn in the Map window and clicked is attached through its software port and answers the same.

## The verbs

| verb | does |
|---|---|
| `btw {action: perform, do?}` | `status` (default): the inputs the program reads, whether it logs, the take's CID and size, the bank; `start`, `stop`, `save` (answers `saved`, the performance's CID, and warns when its program is not saved), `list` |
| `btw {action: replay, id}` | a saved performance (its CID, or a head or tail) played back: see above |
| `djctl btw replay FILE\|CID [OUT.wav] [--program FILE] [--bars N] [--beats N] [--sr N]` | offline: the program played on a fake clock from its first bar with the performance's inputs, no app and no controllers. It prints the calls its control lines make (`bar 2 beat 0.00  mixer {"deck":"A","volume":0.25}`), twice the same; with an output file it renders the song with those calls in it |

**What the offline render honours of the controls:** the arrangement format has an op for a strip's volume, EQ (`low` `mid` `high`)
and mute, the master level and the crossfader (the `mixer` noun's calls), and nothing for an insert, a move or a `do` call. So a
replayed `deck A: level knob(1)` is in the render (as `param` actions at the beat its call was made), and a performed `filter` sweep
is not: `djctl btw replay` says `in the render: N mixer calls; not in it: inserts x8` and prints every call either way. (A render
without a replay carries the program's own mixer lines too, ramps and lfos included, docs/btw/ext-render.md; a replay has the calls
of every such line from its run, so its render carries only the sections, `duck` and `mix` of the program itself and these calls.)
The replay of a program without sections places its song start one bar after its program bar 0, as the live runtime lands a program.

## Launch: a pad brings a scene in

`launch drop on pad(2) next phrase 16` (docs/BTW.md, "Scenes and launch"; grammar `launch`; module `crates/djlive/src/launch.rs`, the runtime in
`runtime.rs`). The pad is `pad(N)` of this page: the first attached device's note pads, 0 or the velocity while held. A launch is a
statement, so it is in the program's text and CID; the hit is not, and is logged in the take.

* **Polled each service pass** (a frame of the booth): `LiveHost::input("pad", "2")` goes from 0 to something: a hit. A pad seen for the first time is
  not a hit; a held pad is one. No new method of `LiveHost`: a host that answers nothing (the page, the phone, a test) never sees a hit.
* **Queue and decision.** A hit queues the scene (`Queued`), the last hit before the decision wins. In the first three quarters of a bar (`APPLY_BY`) the
  runtime asks whether the next bar line is a multiple of the line's bars, counted from the song's first bar (`Running::first`, never moved by a launch);
  if so, the scene's first nets are loaded (`decide_landing`: the scene's program laid over the running one for the call of `sync_nets`, so
  the nets, the sounds and the decks that stop are the loading code's own) and the song plays on to the bar line. `btw get` says `queued`, with
  the bar it lands on, and `scene` once it has.
* **Landing is a lap** (`enter_scene`): at the bar line what the lines hold is put back (`undo_at`, a sung clip), the items are the scene's controls,
  all fresh from that bar, the origin is that bar and the tempo curve is the scene's. `Running::compiled` is then the scene (the same text and CID as
  the program, a timeline of its own), and `Running::base` the program as written: the launches, the edit and the diff are of it, so an edit
  while a scene plays lands on the same scene of the new program, and a scene that is gone sends the song back to its first bar.
* **Each scene is a program** (`Compiled::launches.scenes`): `check` compiles the program once more for each scene a launch names, with the song
  replaced by the scene (`launch::scene_program`: a section is the arrangement `NAME: NAME loop`, an arrangement is `start NAME`; every other
  statement keeps its span). A line that counts from a section the scene has not got (`after verse:`, `before drop 2 beats:`) is left out of the
  scene, with a warning at its name (`drop` is not in the scene `verse`); any other error in a scene is said where it is, with the scene named.
* **After the end** a hit starts its scene at once (`Live::armed`): the song is not stopped for good while a pad could bring it back.
* **The take** has `launches: [{bar, scene}]` (the bar the scene landed on), sorted, said only when there are some. A replay brings each in on its
  bar line, without asking the pads (`PerfState::replay_launch`), and logs the same take. A performance of a program with launches and no input is still
  a performance (`save` keeps it).
* **Not done:** a pad latch (a tap between two frames); a per-device bank; `launch` in a section; a scene that carries a generation over.

## Canonical form

An input prints as written, one spelling: `knob(3)`, `pad(5)`, `fader(A)`, `held(key)`, `held(c4)` (a note is lower case, with sharps:
`held(C#4)` is `held(c#4)`), in the expression's canonical text (`200 + knob(3) * 7800`, one space round a binary operator,
`min(knob(1), 0.5)`). Spacing inside the call (`knob ( 3 )`) is not part of the program; a different input, a different number or
operator is a different program (a different CID). Random reformatting keeps the CID (`tests/ext_perform.rs`: 60 reformattings,
and `examples/btw/perform.btw` through `examples_btw.rs`), and a semantic edit (`knob(3)` to `knob(4)`, `held(key)` to `held(c4)`,
`min` to `max`) changes it. The performance is not in the program's text or its CID: the program is open (it plays differently each
time) and its performances are objects of their own.

## Decisions

* **The placeholder is 1/2.** A control with an input is compiled once as if every input read 1/2: the names, the registry, the
  param types and the range of a literal are checked then, and the line is wrapped in `Effect::Performed` (the compiled effect, the
  guard, a `Patch` per live value: where it goes in the call, its transform (the filter's own knob curve) and its range). What a
  value can be is checked from its **bounds** (interval arithmetic over inputs in 0..1), not from the placeholder, so `level 3 *
  knob(1)` is not refused for being 1.5 at one half.
* **The runtime evaluates the expression itself** (`LiveExpr`, over `f64` with the quantised inputs), not the compile-time rational
  evaluator: that one needs a number it can prove; this one runs while the program plays. What has no input is folded when the line is
  expanded (`$i % 2 == 0` is 1 or 0), so only the live part is evaluated each time. Comparisons are equal within a nano-unit
  (`value >= 0.5` holds for the quantised 0.5 whatever the rounding of the constant).
* **Reads are where a line is due, once for lines due together, and logged on first read.** The log is therefore the values the program
  *used*, one for each bar, place in it and input, and the replay answers exactly those reads. (The alternative, reading every input of
  the program at every bar line, would log values nothing used and read a controller for a line that is not due.) A read the host cannot
  answer is not logged: a replay of it reads the input's home and says so, as the live run did. Faster reads fill a take faster: sixteen
  reads a bar of one knob reach the 262,144 cap in about nine hours, and the take says when it is full.
* **A value line follows (R1), and a knob rests at 1/2 (R4): a change of how today's performed programs play.** An untimed value line used
  to fire once, so `master: filter lp 200 + knob(3) * 7800`, the first example of this page, read the knob once and left the filter where
  the first bar put it. An old take of such a program has one reading, and replays its later bars at the home value with the word the
  replay always had ("the performance has no value for it at bar N"). No CID moves: values never enter the text, and the program
  prints as it did.
* **No `knob 1 -> filter` statement and no binding statement.** The record's line is `master: filter lp knob(1)`: a statement `knob 1 -> master
  filter lp` is no shorter, a third spelling after the booth's bindings and `knob()`. A binding in the text would put a device in the CID and
  break "checks the same on every machine". The mapping from a physical control to a role (`knob(3)`) is a host rule (`djhost::btw_inputs`);
  the mapping from a role to an effect is the program's control line, or a booth binding preset chosen by name.
* **Replay is inside the runtime (`Live::replay`), not a host wrapper:** a host that forgot to answer from the log could not make a
  replay differ, and the checks (the program's CID, the take) are in one place. `LiveHost::input` stays the single host method; the
  brief's "the LiveHost input method answers from the log by bar" is `PerfState::read`'s first branch.
* **The verb is `perform {do: start|stop|save|status|list}` and `replay {id}`**: `action` is the verb of the `btw` noun (eval, check,
  get ...), so the three states are a `do` param, as `moves` has.
* **The knob bank** is the MPK's eight, else the APC's nine faders (the only banks of knobs and faders there are); **the pads** the
  first attached device's note pads; **`fader(A)`** the mixer's volume (the value the bindings produce, whichever control moved it:
  the APC's faders, the Inpulse's, a drawn one, or `deck A: level 0.7` in a program).
* **A one-shot guarded line is due once** (see above). **A def call takes no guard**, and a `let` cannot hold an input: both are
  errors with the way out, rather than a rule that quietly reads the input at expansion.
* **A note is spelled once** (`held(C#4)` prints `held(c#4)`; sharps, lower case): `held(db4)` and `held(c#4)` are one input.
* **`held(key)` is a level like any input:** the key number over 127, quantised to 1/128 (key 60 reads 60/128). A program that wants
  a range of keys compares it (`held(key) > 0.5`) rather than reading the number back; `held(c4)` is the exact test for one key.
* **`do` params with a dot read again.** `param=delay.mix` (the registry's own spelling, in `docs/BTW.md`) had stopped reading since
  the lexer made `a.b` a `Dot` token for F11; `spec_word` joins `word . word` (the sample the contract's `do` examples need).
* **Two lines of `tests/ext_arith.rs` changed** (the only edit of an existing test): a guard `when knob(3) > 0`, which the arithmetic
  worker pinned as "not a function of arithmetic", is a performed input now (the line says `foo(3)` instead), and `tempo knob(3)`
  still says `expected a tempo`.

## Not done

* An input in an `evolve` rule's number (deferred with the reason above), in a `let`, and a guard on a `def` call.
* (`lfo` and the shapes of F4 were not merged when this was written: they are now, see "Meeting the others".)
* A **latched** pad (a tap between two reads is missed: `pad(5)` is the pad's state when its line is due, not "was it hit since"). A `launch` is polled every service pass of the booth, so it only misses a tap shorter than a frame.
* The knob and pad banks of the Inpulse and the TD-3 (they have none by the rule above); a per-device choice of bank (`knob(3)` of the
  second device) is not in the language.
* The offline render has no op for an insert, a move or a `do` call (above, and docs/btw/ext-render.md), so a replayed filter sweep is printed, not rendered.
* A take that begins mid-program replays from bar 0 (above); a performance of a program that was edited while it played is a take of
  the program running at the end of it.
* The djapp window does not show the performance (the Map shows the controls glowing): the verbs and `btw get` do.

## Meeting the others

Found when the performed tier met the other features (`crates/djlive/tests/ext_perform_cross.rs`, `djhost` `btw_test` / `btw_render` /
`live_store`, `djctl/tests/btw_test_cmd.rs`).

* **F4 shapes and lfo.** `master: level 0 -> knob(1) over 4 bars exp` is a performed ramp like the plain one (the shape stays; its
  ends were frozen at the 1/2 the compiler checks with before). An lfo's **`center=` and `depth=`** are control values:
  `master: filter lp lfo sine 2 bars center=2000 + knob(3) * 4000 depth=1000`, and in a `do` line
  `do vocoder set mix=lfo sine 2 bars center=0.5 depth=knob(2) * 0.4`. They are read **once a bar, at the bar line** like a ramp's
  ends (`LfoRun::retune`): the wave goes on in time, and its middle and reach become the new ones, so the value steps at the bar line
  and swings round the new middle from there. The swing `center - depth` .. `center + depth` is held to the range of what it moves
  and turned into the call's own unit (a cutoff's knob) when it is read (`perform::LfoLive`); one that can never be in the range is
  an error, one that sometimes is, a note; a depth below 0 reads 0. **`phase=` is a literal** (`an lfo takes a literal phase`): the
  wave is not re-phased by a knob. `over` is a time and takes no input.
* **F8 mix and duck.** A mix item is one number, set once: **`mix { A level knob(1) }` is an error** (`a mix item sets one number,
  once: it takes no performed input`) that names the control line to write (`deck A: level knob(1)`); an input inside the item's
  arithmetic (`A level 0.5 + knob(1)`) says the same at the call. But a **`when` after a mix, a duck or a sing line** is decided while
  the program plays, as on any control line: it used to be dropped without a word, because these lines are compiled apart from
  `Compiler::control`; they now go through `Compiler::performed`, so `at bar 2: mix { A level 0.2 } when held(key)` fires only if a
  key is down at bar 2. A `for N bars` hold of a mix that did not fire has nothing to put back. `check` lists the guard
  (`only when held(key) is true at the bar it is due`).
* **F11 ports.** A wired value is a number fixed when the program is read: an input in an `emit` for a `tempo` or `bars` port is an
  error (`a tempo is one number, fixed when the program is read: knob(...) is read while the program plays, so a port cannot
  carry it`). A def that has a port may have control lines that name inputs (`master: filter lp $in * 20 + knob(1) * 4000`), and a
  def's arguments, `repeat`, and imports carry inputs as before; `<=` and `>=` in a performed guard stay two glued tokens through
  all of them.
* **F1 sub-bar times.** A control that fires between bar lines (`at bar 2 beat 2:`, `every 2 beats`) reads its input **where it is due**
  (R3: it read the value of the bar it fell in before), and the log holds a reading for the bar, the 1/64 of it and the input. A replay
  answers by bar and place, so a run and its replay are call for call the same (a property loop of scripts).
* **F3 tempo.** `tempo knob(3)`, `at bar 4: tempo 120 + knob(1) * 20`, a tempo curve's end, `swing`, `at bar knob(1)`, `every knob(1)
  bars`, `end at bar knob(1)`: all refused with `expected <what was wanted>: knob(...) is a performed input: it is read while
  the program plays, so it cannot stand here`, at the call. A tempo curve and a performed line play together (the curve goes to the
  decks as before; the replay has the same tempo calls).
* **F10 stacks and sections.** Performed lines in the sections of a `stack`, in sections that set their own key and tempo, and across
  a `loop` fire where their section is and replay equal; reads are by program bar.
* **F12 assertions.** An assertion cannot read an input (`assert level A < knob(1)`: an assertion is judged on an offline render,
  which has no controllers). **`btw test` measures a performed program with every input at 1/2** (the placeholder the compiler checks
  with) and says so: the text has a line `note: this program is performed: levels are measured with every input at 1/2 (the render
  has no controllers), and only the lines a render can carry are in it (mixer levels, EQ, mute, crossfader, play and stop, duck, mix)
  (knob(1), ...)`, and the JSON a `performed` object (`inputs`, `read_as: 0.5`, `note`). The offline render carries the mixer's lines,
  `duck`, `mix` and a tempo curve (docs/btw/ext-render.md), so a performed level, EQ or crossfader (a ramp's end, an lfo's centre too) is in it
  at the value 1/2 gives; a filter or `do` line is not in it whatever the inputs read; any performed line with a `when` is in it exactly
  when the guard is true with every input at 1/2 (`held(key)`, `held(c4)`: yes; `knob(1) < 0.5`: no). `djctl btw render` does the same.
* **F14 links.** A performance names its program by the program's CID, which is of the canonical text: the `#! genre:` and `#! tags:`
  lines are comments to the language, so a program saved with links has the CID it has without them, and changing its tags is a new
  object of the same program. A performance saved before (or after) is still found with its program (`performance_program`), listed
  with the program's name, and replays to the same calls (`live_store` test).

## For later workers

* `perform::{Input, InputBank, LiveExpr, Performed, Patch, Performance, Reading, PerfState}`, `perform::{allowing, with_bank,
  mentions_input, lower_expr, quantise, note_number}`; `Env::bank`; `LiveHost::input`; `Live::{performance, perform_start,
  perform_stop, perform_status, replay, replaying, performed_inputs}`; `Effect::Performed`; `djhost::btw_inputs::Board`;
  `LiveStore::{save_performance, load_performance, list_performances, performance_program}`;
  `btw_render::{replay_controls, replay_bars, arrangement_with_replay}`.
* A control value that is read by a parser function other than `Parser::change` / `do_number` needs
  `perform::allowing(|| self.num(...))` round the read and a patch built where its effect is compiled (`Compiler::performed` knows
  `Action::Change` and `do` params).
* `Control.effect` can be `Effect::Performed(p)`: `p.base` is the effect as compiled (read it for anything that does not play, like
  `docall::derive_undo` does), `p.inputs` the inputs it reads.
