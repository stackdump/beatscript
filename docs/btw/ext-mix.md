# btw extension F8: mix moves (`duck`, `mix { }`)

*Part of docs/BTW-EXTENSIONS.md. Code: `crates/djlive/src/mixmoves.rs` (the language), hooks in `runtime.rs`, `compile.rs`,
`parse.rs`, `print.rs`, `highlight.rs`, `lex.rs`; `crates/djhost/src/btw_render.rs` (the offline render); tests
`crates/djlive/tests/ext_mix.rs`; example `examples/btw/mix.btw`.*

Two lines for the mix, and nothing new in the engine:

```text
duck B under A.kick 6db attack=5ms release=120ms     # B goes down 6 dB on every kick of A
mix {                                                # in a section: applied on its first bar, put back when it ends
  A level 0.7
  B level 0.9
  A eq low -3
  master level 0.9
}
```

## Grammar

```ebnf
duck        = "duck" deck "under" deck "." role db { "attack=" time | "release=" time } ;
mix         = "mix" "{" mixitem { ( "," | NEWLINE ) mixitem } "}" ;
mixitem     = deck "level" arith
            | deck "eq" band arith [ "db" ]
            | "master" "level" arith
            | deck "pan" arith ;                 (* read, and refused: the mixer has no pan *)
band        = "low" | "mid" | "high" ;
db          = number "db" ;                      (* glued: 6db; no sign: it is how far the deck goes down *)
time        = number ( "ms" | "s" ) ;            (* glued: 5ms, 0.12s *)
```

`duck` and `mix` are lines like any other, so they take a time word and `for N bars` before them (`at bar 8: mix { ... }`,
`for 4 bars: mix { ... }`, `every 16 bars: mix { ... }`), they can be inside a `def` or a `repeat` (`duck $b under $a.kick
$db`: a `.` may follow a `$name` now), and neither takes a `deck X:` or `master:` before it (an error that says so).
`under` is a reserved word. `mix` (before `{`), `duck` (before a deck), `eq` / `pan` (after a deck in a mix), `attack` and `release` (before `=`) are read as syntax there and name a `let`, a `def` or a section elsewhere; `mix(` and `duck(` call a def of that name (BTW.md "Decisions").
`arith` is arithmetic over numbers (`0.5 + 0.2`, `(1 - 0.1) * 0.5`): evaluated once, when the program is compiled.

## What they come to

Everything maps onto verbs of the control-language registry that exist already; `check` prints the calls
(`Compiled::mix_report`, `djctl btw check`, the `mix` field of the `check` / `eval` answers).

| line | registry calls |
|---|---|
| `duck B under A.kick 6db attack=5ms release=120ms` | `trick {action: on, deck: B, kind: pump, source: A, amount: 0.499, div: 16}`; put back with `trick {action: off, deck: B, kind: pump}` |
| `A level 0.7` | `mixer {deck: A, volume: 0.7}` |
| `A eq low -3` | `mixer {deck: A, low: 0.4375}` (also `mid`, `high`) |
| `master level 0.9` | `mixer {master: 0.9}` |
| (the read, for a snapshot) | `mixer {}`: answers `{set: [], mixer: {strips: [{deck, volume, eq: {low, mid, high}}], master}}` |

### The duck is the engine's pump

The engine has a sidechain already: `trick` kind `pump` (docs/TRICKS.md): a kick detector (two 120 Hz low-passes, a fast
envelope against a slow one) on the `source` deck's pre-fader signal ducks the deck by `amount` in 2 ms and lets it back over a
1/`div` note. So a duck adds nothing to the engine, and it is **keyed from the audio of the deck, not from the hit times the
compiler knows**: the same sound is made live and in the offline render.

* **Depth.** `6db` is the gain taken off at the bottom of the duck: `amount = 1 - 10^(-dB/20)` (6 dB is 0.499, 12 dB 0.749),
  to three places. 0 < dB <= 60.
* **Release.** The pump releases over a note: `div` is the one of 4, 6, 8, 12, 16, 24, 32 whose note at the tempo is nearest
  `release` (by ratio). At 124 BPM 120 ms is a 1/16 (121 ms). The tempo is the program's `tempo`, else the grid deck's when the
  line fires. `check` says what it came to, and **warns** when the nearest note is more than 1.4 times off (a 2 s release is a
  quarter note: the longest the pump offers) and the program's tempo is known.
* **Attack.** The pump's is 2 ms and is not a parameter of it. `attack=` is recorded (it is part of the program, the default is
  5 ms), and **warns** only when it is slower than 10 ms: the pump cannot be slower than it is.
* **The part.** The detector hears the low end of the source deck, so `A.kick` keys it, and `A.bass` too with a warning (a bass
  note ducks on its own onset); a snare, a hat or a melody cannot, which is an **error** with the way out (`duck B under
  A.kick 6db`), not a duck that never happens. The part has to be in the deck in that section (or in the song's own lines): an
  error names the parts the deck does have. A duck of a deck under itself is an error. A duck outside every section in a song
  with sections warns for each section where the part is missing (it idles there).
* **How long.** In a section, to the section's end (`trick off` on the bar line the section ends on). Outside a section it is
  held until the program ends or `btw stop` (the deck panel's "held" mark says so). `for N bars:` puts it back after N bars.

### The mix is the mixer

* **Units.** `level` is the fader's 0..1 (an error outside it; `50%` is 0.5). `eq` is a gain in dB, -24 to +6, as the mixer's EQ
  knob is (`Deck.eqDecibels`: knob 0 is -24 dB, 0.5 is flat, 1 is +6 dB; the band is bypassed within 0.01 of flat); the knob is
  `0.5 + dB/48` below flat and `0.5 + dB/12` above, so -3 dB is 0.4375. `master level` is `mixer {master}`.
  `A eq low -3db` is accepted and prints as `A eq low -3`.
* **In a section** the mix is applied on the section's **first bar** and **put back on the bar line the section ends on** (the
  next section's first bar: the same bar `after NAME:` means). The runtime reads the mixer first (`mixer {}`) and puts each knob
  back to what it held, last set first. A snapshot taken on a bar line comes after the section before it was put back (the
  sections' lines are in song order). A mix with `at bar N:` in a section fires then and is put back at the section's end; a time
  word that repeats (`every`, `on phrase`, `after`/`before` outside the section) is an error there ("a section's mix fires
  once"). `at bar N` past the section's end is an error.
* **With `for N bars:`** the same, N bars on, anywhere.
* **Outside a section, with no `for`,** a mix just sets: it reads nothing and puts nothing back (`every 8 bars: mix { ... }` is
  a line that sets again every eight bars).
* **No read, no problem.** If the host cannot answer `mixer {}` (or leaves a knob out), a knob goes back to **the program's own
  last line before it** (a `deck A: level 0.6` or an earlier plain `mix`, by the bar it fires on), else to the engine's starting
  position (strip fader 0.8, EQ flat 0.5, master 1.0). `btw get` says so in its words: `mix: the mixer cannot be read here, so it
  goes back A level to 0.6 (the program's own last line), master level to 1 (neutral)`.
* **Whatever ends the section puts it back:** the next section, `end`, the song's end, a loop's lap, `btw stop`, an edit that
  drops the line (the old line is put back; the edited line waits for its section to come round, as every `at bar` line does).
* `A pan -1` is read and **refused**: the mixer has no pan on a strip. The error names what exists: a net deck's parts can be
  panned for some bars with `do aux macro=hard-left deck=A bars=4` (also `hard-right`, `mono`, `ping-pong`, `auto-pan`).

## Canonical form

```text
duck B under A.kick 6db attack=5ms release=120ms
at bar 8: mix {
  A level 0.7
  B level 0.9
  A eq low -3
}
```

* A duck prints **both** times, defaults filled in, always in ms (`release=0.12s` and no `release=` print as `release=120ms`), so
  one duck has one spelling and one CID. Params in any order are sorted: `attack`, `release`. `6.0db` is `6db`.
* A `mix` prints one item a line, indented a level in, `}` on a line of its own, after its time word. Commas and line breaks
  between items are the same program. Each number is the value its arithmetic **comes to** (six decimals at most), so `A level 0.5
  + 0.2` and `A level 0.7` are one program with one CID, and the text is a fixed point (a rational printed as `1/2` would read back
  as a division). The order of the items is meaning (it is the order they are sent in) and is kept.
* In a `def` or `repeat` body the tokens are printed one statement a line as every body is (not normalised: a body is meaning).
* Tests: `ext_mix.rs` reformats five programs 120 times each (spacing, tabs, comments, blank lines, number spelling): same CID; one
  semantic edit (a depth, a time, a band, a number, a deck, the order of two items) changes it.

## Errors

Every error has a line and column, what was expected and a help line. Among them:

| written | says |
|---|---|
| `duck B under A.kick 6 db` | a unit is glued to its number: `6db`, not `6 db` (help: write `6db`) |
| `duck B under A.kick 6` | a duck's depth needs its unit (help: write `6db`) |
| `duck B under A.kick -6db` | a duck takes no sign: it is how far the deck goes down |
| `6dB`, `120MS` | (the lexer) units are lower case and glued: `` `6db` `` |
| `duck B under A` / `under A kick` | `under` takes a deck's part, like `A.kick` (help: write `A.kick`, with no space) |
| `A.kik` | `kik` is not a part (help: did you mean `kick`?) |
| `duck b under ...` | decks are upper case: `B` |
| `atack=5ms` / `attack=5` / `attack 5ms` | did you mean `attack`? / needs its unit / is written `attack=5ms` |
| `duck B under A.snare 6db` | `snare` cannot key a duck: the engine's sidechain listens for a deck's kick |
| `duck B under A.kick 6db` where A has no kick here | deck A has no `kick` part in section `drop` (help: deck A plays: snare) |
| `duck B under B.kick 6db` | deck B cannot duck under itself |
| `mix { A leval 0.7 }`, `A eq lo -3`, `A low -3` | did you mean `level`? / `low`? / an EQ band is `A eq low -3` |
| `mix { A level 0.7 B level 0.9 }` | expected `,` or the end of the line |
| `mix { A level 1.5 }`, `A eq low -30` | a level is 0 to 1 / an EQ band is -24 to +6 dB |
| `mix { A level 0.7, A level 0.8 }` | `A level` is set twice in this mix |
| an error in one item | one error, not one for each line left in the block (the parser skips to the closing `}`) |

## Offline render (`djctl btw render`, `djhost::btw_render`)

The render honours both: `duck` is a `trick` live op at the bar line (the offline
engine runs the pump as the app does, `source: A` hears deck A's kick), `mix` is `param` (`volume`, `eqLow`, `eqMid`, `eqHigh`)
and `master` actions. There is no mixer to read offline, so a snapshot goes back to what the **render's own lines** left: the
engine's starting positions at first, and the value of the program's own `deck A: level 0.6` or ramp once the render plays those
too (docs/btw/ext-render.md: it carries the mixer's control lines). Events at or
after the render's last bar are left out. `every` timings are expanded over the render's length.

## Tests

* `crates/djlive/src/mixmoves.rs`: parse and canonical form, the pump's depth and note, the EQ scale (and the engine's own
  `eq_decibels` agreeing), reading the engine's own `MixerState` answer, every error above, scope and hold, the fallback table,
  a `def` that takes the decks and the depth.
* `crates/djlive/tests/ext_mix.rs` (27 tests): one spelling and a changed CID for every semantic edit; 120 random
  reformattings of five programs keep the CID; the check's report; warnings; line and column; on a fake host with a mixer you
  can read and a fake clock: a section's snapshot is applied on its first bar, read first, put back on the bar the section ends on
  (and the next section reads what was put back), a mixer that cannot be read goes back to the program's own line and says so,
  `stop`, the song's end, a loop's lap, an edit that drops the line, `for N bars`, a plain mix, the duck on and off with the
  release note from the program's tempo or the deck's, the marks (`held`, the span of the whole block, `via` of a def call); the
  **law** that a snapshot is put back whatever the song (80 random songs, with a host that reads and one that cannot); the
  example; a registry that lacks what the lines are made of.
* `crates/djhost`: `btw_render` (the events of the render; **the pump heard ducking B under A's kick**: B's stem is at 0.4 of
  itself just after a kick and at 1.0 before the next, and the engine refuses nothing) and `btw_env` (the example checks against
  the real registry, every param of the calls is the registry's own).
* `crates/djapp` `app_tests`: on the real engine a duck turns the pump on for B, a `for` mix reads the real mixer and `btw stop`
  puts the knobs and the pump back.

## Joins with the other extensions (integration, `tests/ext_cross5.rs`)

* **Arithmetic (F2).** A mix item is arithmetic evaluated when the program is compiled, and the canonical text writes the number it
  comes to (`A level (1 + 1) / 4` is `A level 0.5`, `master level 1 / 3` is `master level 0.333333`): `1/2` printed as the
  rational would read back as a division, and the folded number is a fixed point. A def's or repeat's `$name` is put in place before
  the expression is read, so `mix { $under level 0.5 + $k / 10 }` called with `$i` is `0.5`, `0.6`, `0.7`; the def body itself is
  printed as written. A duck's depth and times are literals with a unit glued (`6db`, `5ms`): a def's arithmetic argument is put
  in place in parentheses and `(3 + 3)db` is not a number with a unit, so the error says arithmetic is for a mix's numbers. A
  `when` guard after a mix goes after its closing brace (`} when $i % 2 == 0`), in a def body too.
* **Sub-bar timing (F1).** `at bar 4 beat 2: mix { ... }`, `at bar 2 beat 1 for 2 beats: ...` and `every 2 beats: ...` work
  wherever the line is: a mix in a section is put back at the end of the section whatever beat it fired on, a `for 2 beats` is put back
  exactly two beats later (the runtime's sub-bar pass puts back from the snapshot too), and the offline render places the events on
  those beats. The check says `put back on bar 4`, `bar 2 beat 3`.
* **Stack (F10).** A section stacked with a longer or a shorter one is cut into stretches and its lines are in the first of them (a
  copy goes on into the rest, marked `phase`, which is not a second duck or mix): a mix or a duck is held to the bar where **its own
  section** ends (found by following the copies of the line), not to the end of the stretch it is in. Two mixes that hold one knob
  at the same time, in sections that play together or with `for` that overlap, are an error naming both lines: the one that ends
  first would put back the other's value and the one that ends last would put back that one, leaving the mixer changed after
  the song. Different knobs of one deck, and the same knob one after the other, are fine. A section placed twice in a row
  (`then(one, one)`) puts back each time.
* **Ramps and lfos (F4).** A mix item is **one number set once**: `A level 0 -> 1 over 2 bars exp` and `A level lfo sine 2 bars` inside a `mix { }` are an
  error at the arrow or the `lfo` that says so and says what to write (`deck A: level 0 -> 1 over 4 bars exp`, `deck A: level lfo sine 2 bars center=0.7
  depth=0.2`, control lines F4 reads); the two kinds of line sit side by side in a section. A shaped ramp on a mixer knob counts at its end value (`to`) in
  what a mix's report says is put back; an lfo has no settled value and is not counted.
* **Tempo curves (F3).** The pump is chosen at the tempo the program starts at (the top-level `tempo`, a section's `tempo` on the first bar, or a curve's first
  value): the tempo map is settled before the mix lines are read. The offline render places a mix or a duck on its beat and tags it with the tempo of the step
  it falls in, so under a curve it lands on the second the curve gives that beat (`crates/djhost/tests/btw_tempo.rs`).
* **The bar line.** At a bar line every put back runs before any line fires, so a line written at the bar a section's mix ends on
  (`at bar 4: mix { A level 0.9 }`) has the last word and is not undone the moment it was said.

## Decisions

* **The sidechain is the existing pump, not a new gain stage** (the brief: add an engine sidechain only if none exists). It is
  sample-accurate and independent of the fader, which a gain envelope driven from the service pass (60 Hz, a 5 ms attack in
  16 ms steps) could not be, and a duck would fight a `level` line for the same fader. The cost is what the pump is: keyed from
  the audio's low end (so only `kick` and `bass` can key it), a 2 ms attack, a release that is a note.
* A duck prints both times (defaults filled in) and the value an arithmetic comes to: one spelling for one meaning.
* "Restored on the section's last bar line" is read as the bar line that **ends** the section (the next one's first bar), so the
  section's last bar is mixed as the rest of it is.
* A mix in a section fires once (`at bar N` allowed); a repeating time word is an error. Outside a section, no `for`, a mix just
  sets (as the contract says), so it also reads nothing.
* The read is `mixer {}` through `LiveHost::call`, so `LiveHost` has no new method and a host with no answer is the "no read" case.
* A duck outside a section is held until the program stops (an item's `undo_at` of "never", so `btw stop` and the end put it back):
  leaving the pump on after `btw stop` would duck a deck under a program that is gone.
* `A pan` is parsed (the contract's grammar has it) and refused, because the mixer has none.
* `attack=` is kept in the program and only warned about, as the pump's is fixed; the offline render and the live app agree
  because both run the same pump.
* The EQ's dB scale is the mixer's own (-24 .. +6), taken from `djrust::fx::strip::eq_decibels`, and not a symmetric -12..+12.
* `.` may follow a `$param` (`$a.kick`) in the lexer: a def cannot be written without it.

## Not done

* A duck keyed by a role that is not low (a snare, a hat, a melody): the engine's sidechain cannot hear it. It needs a detector
  or a hit-keyed gain stage in the engine (the compile knows the hit times: `Compiled` has every part's cycles).
* An attack that is a parameter of the pump (the pump's is a constant 2 ms in `djrust::fx::tricks::Pump`), and a release in
  milliseconds rather than a note (the pump's `div` is snapped to seven divisions).
* A strip pan on the mixer.
* (The offline render used to ignore the program's other mixer lines, so a snapshot went back to the engine's starting position; it plays
  them now, docs/btw/ext-render.md, and a snapshot goes back to what they left at that beat.)
* `mix` inside `evolve` rules, and `mix` items that ramp (`A level 0.5 -> 0.9 over 4 bars`): a ramp is a `level` line already.
