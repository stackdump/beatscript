# BeatScript extensions: the contract for the 14 features

> **Status: the contract of the overnight run; superseded by [BEATSCRIPT.md](BEATSCRIPT.md).** The grammar, the semantics, the canonical forms and the decisions
> below were folded into `docs/BEATSCRIPT.md` (its EBNF, one section for each feature under "The language extensions", "Categorical notes", "Decisions" and
> "Not done"), and each feature has a long note, `docs/beatscript/ext-<feature>.md`. Where this file and `BEATSCRIPT.md` or a long note disagree, the later text and
> the parser are right. This file is kept as history: its section 5 is the log of how the features met when they were merged.

*Working document of the overnight run (branch `feat/btw-extensions`, from `feat/btw-language`). It fixes ONE grammar for
all fourteen extensions so that features built in parallel fit together. When the features land, the grammar and the
semantics here are folded into `docs/BEATSCRIPT.md`, and the decisions into its "Not done" and "Decisions" parts.*

Read `docs/BEATSCRIPT.md` first (grammar, canonical form, CIDs, evolve, sections, the marks for the editor).

## 0. Rules for every worker

*(The overnight run's working rules for its workers, about worktrees, the build machines and the booth, are left
out of the public copy. Sections 1 to 5 are the contract as it was.)*

## 1. Modules and who owns what

| feature | module(s) in `crates/djlive/src` | other crates |
|---|---|---|
| shared (done) | `lex.rs` (tokens), `rat.rs` (exact rationals), `expr.rs` (expressions) | |
| F1 sub-bar timing | `dur.rs` | |
| F2 arithmetic | `expr.rs` (reader hooks in `parse.rs`), `dur.rs` | |
| F3 tempo curves | `tempo.rs` | `djhost::beatscript_render`, host tempo |
| F4 shapes, lfo | `shape.rs` | |
| F5 degrees, Roman numerals | `scale.rs` | |
| F6 euclid, polymeter | `meter.rs` | |
| F7 chance | `chance.rs` | |
| F8 mix moves | `mixmoves.rs` | `djengine`/`djhost` (a sidechain if none) |
| F9 sing | `sing.rs` | `djhost::beatscript_env` (vocal) |
| F10 stack / then | `algebra.rs` | |
| F11 ports | `ports.rs` | |
| F12 assertions | `assertion.rs` | `djhost::beatscript_render`, `djctl` (`btw test`) |
| F13 performed tier | `perform.rs` | `djhost`, `djapp` (controller bindings), `live_store` |
| F14 ontology links | `linked.rs` | `djhost::live_store`, `djctl` |

## 2. What is shared (already in the base commit)

**Tokens** (`lex.rs`): `+` `Plus`, `?` `Question`, `'` `Apostrophe`, `.` between two words `Dot` (`A.kick`, `a.out`), `±`
`PlusMinus`, and **units**: a number with a unit glued to it is `Num` then `Unit` (`6db`, `120ms`, `2s`, `440hz`, `3m30s` is
`3` `m` `30` `s`; the units are `db ms hz s m`, `lex::UNITS`). `°` ends a word (`ii°`). `==`, `!=`, `<=`, `>=` are two
glued tokens (`Eq Eq`, `Bang Eq`, `Lt Eq`, `Gt Eq`; `expr.rs` reads them). `%` after a digit with no space is the percent
mark (`50%`), so **write modulo with a space: `$i % 4`**.

**Rat** (`rat.rs`): exact rationals, checked (an overflow is an `Err`, never a wrap), canonical `3`, `1/2`, `-5/4`;
`Rat::from_f64` recovers the decimal a person typed (`0.3` is `3/10`). Numbers reach the parser as `f64`; convert with
`Rat::from_f64`.

**Expr** (`expr.rs`): `+ - * / %` over rationals (`/` exact, `%` Euclidean), `min(...)` `max(...)`, parentheses, unary
minus, comparisons `== != < <= > >=`, `and` `or` `not`, words as operands for `==` / `!=`. `parse_arith` (no comparison) and
`parse_expr` read from a `Cursor` over the parser's tokens; `eval` / `eval_num` / `eval_bool` evaluate with a `Hooks`
(extension calls such as `knob(3)`). `Expr::canonical()` is the canonical text of an expression. A `$name` that is still
there is `Expr::Unbound` and evaluates to an error that says where `$i` and `$n` exist.

**Test helpers**: `tests/common/mod.rs` (`Rng`, `reformat`, `glued_pair`), `tests/examples_beatscript.rs`.

## 3. The grammar of the fourteen features

Notation as in `docs/BEATSCRIPT.md`. `expr` / `arith` are `expr.rs`'s. `dur` is a duration (F1). Every feature below says what its
canonical form prints. New keywords (reserved already): `stack then silence wire assert sing duck under mix choose maybe
weights deg lfo sine tri square saw exp log step beat beats in out ends period length knob pad fader held min max and or
not voice-lead voice genre tags replay test eq attack release center depth phase s-curve linear`.

### F2 Arithmetic (module `expr`, hooks in `parse`, `expand`)

```ebnf
let         = "let" NAME "=" ( arith | token | pattern ) ;     (* arithmetic wins when every token is numeric (numbers,
                                                                  operators, parentheses, min/max calls, earlier scalar lets)
                                                                  and there is at least one operator, parenthesis or call *)
value       = arith ;                                          (* anywhere the language takes a number: tempo, swing, params,
                                                                  `at bar N`, `every N bars`, `over N bars`, `repeat N`, control values *)
param       = NAME "=" ( arith [ ".." arith ] | word ) ;        (* vel=(60 + $i * 4)..100 *)
guard       = "when" expr ;                                    (* after a part line or a control line: kept when true *)
term        = atom [ "!" ] [ "^" ] [ "*" ( int | "(" arith ")" ) ] ;
atom        = ... | "(" arith ")" ;                             (* a degree computed: ($i + 1) *)
arg         = ... | arith ;                                     (* generator arguments: e($i % 4 + 1, 8) *)
```

* **Evaluated once, at compile time.** `$i` and `$n` (a `repeat`'s index and count) and a `def`'s `$params` are tokens put in
  place before an expression is read, so `$i * 2 + 1` is `3 * 2 + 1`.
* `let n = 2 * 4` is the number 8 (a scalar: put in place of every later `n`). `let r = 3*2` is arithmetic now (6; it was the
  degree `3` played twice): a program that means the degree writes `let r = [3 3]` or `let r = 3 3`. This is the one deliberate
  change of old meaning (no fixture uses it).
* `when` guards: `deck A: kick x ~ x ~ when $i % 4 == 3`, `at bar $i: master: filter lp 300 when $i % 2 == 0`. A guard that
  is false removes the line **before** it is checked or compiled (so a guarded-off line may name things that do not exist
  in this repetition). A guard is evaluated where the line is expanded: inside a `repeat` or a `def` it sees `$i`, `$n`,
  `$params`; at the top level it is a constant (`when 1 == 1`), allowed and kept in the canonical text as written. A guard
  that is not a condition, or has an unbound `$name`, is an error at its span.
* Words in expressions compare with `==` / `!=` only: `when $deck == A`.
* Canonical form: an arithmetic value that is a single literal keeps today's spelling (`2k` is `2000`); any other
  expression prints with `Expr::canonical()` (one space round a binary operator, rationals in lowest terms). A scalar `let`
  of an expression is folded: the value is put where used, as a literal.

### F1 Sub-bar timing (module `dur`)

```ebnf
dur         = arith ( "bar" | "bars" | "beat" | "beats" ) ;      (* over 1/2 bar   over 2 beats   every 3/4 bar *)
when        = "every" dur [ "from" pos ]
            | "at" pos
            | "before" NAME dur  | "after" NAME  | ... as before ;
pos         = "bar" arith [ "beat" arith ] ;                     (* at bar 16 beat 3   at bar 16 beat 5/2 *)
span        = "over" dur ;
hold        = "for" dur ;                                        (* for 2 beats *)
```

* A beat is a quarter of a bar (4 beats a bar, as everywhere). **Beat offsets are 0-based like bars**: `at bar 16 beat 3`
  is the last quarter of the bar whose line is bar 16; `beat 0` is the line itself; a beat is `0 <= b < 4` (an error says so).
* `every 2 beats` fires every half bar from the program's start (never at 0, as `every N bars` never does); `every 1/2 bar
  from 3` fires at 3, 3 1/2, 4 ... (`from` is a position in bars, rational allowed).
* `over 2 beats` is a ramp a half bar long; `before drop 2 beats` is a half bar before the section's line.
* The clock is the deck's beat position (the runtime's `pos`, exact in beats), not seconds: sub-bar words fire when the
  position **crosses** their time, once, deterministically; a test on the fake clock steps through them.
* **Canonical form**: a duration is `N bars` when `N` is whole (as today, `1 bars`), else `N beats` / `1 beat` when the
  beats are whole (`over 1/2 bar` and `over 2 beats` are the same program: `over 2 beats`), else `P/Q bars` in lowest terms
  (`1/3 bars`). A position is `bar W` when whole, else `bar W beat B` with `B` a rational in lowest terms (`at bar 16 beat
  5/2`). Whole-bar words print exactly as before, so no existing CID moves.
* Sections stay a whole number of bars.

### F3 Tempo curves (module `tempo`)

```ebnf
setting     = "tempo" value [ "->" value "over" dur [ shape ] ] ;     (* top level: from the program's first bar *)
section-set = "tempo" value [ "->" value "over" dur [ shape ] ] ;     (* inside a section: from its first bar *)
timed       = when ":" "tempo" value [ "->" value "over" dur [ shape ] ] ;   (* at bar 16: tempo 128 -> 140 over 4 bars *)
```

* `tempo 100 -> 130 over 8 bars` ramps the tempo (BPM linear **in beats**, so the bar clock never moves) and then holds
  130. A plain `tempo N` in a section sets it at that section's first bar and holds it until another `tempo`.
* **Exact integration.** Seconds are the integral of `60 / bpm` over beats, in closed form per segment (`60 L ln(b/a) / (b - a)`
  beats `L` from `a` to `b`, `60 L / a` when constant). `TempoMap::seconds_at(bar: Rat)` and `bars_at(seconds)` are what
  `check` (length), `assert length`, the offline render and the runtime use. Bars and every timing word are in bars/beats and
  stay exact whatever the tempo does.
* The runtime sets the tempo each service pass from the position (decks follow: the program's tempo is the sync master).
  The offline render honours the map. A host with no tempo control gets a clear word in `btw get`.
* Canonical form: `tempo 100 -> 130 over 8 bars` (a plain `tempo N` prints as it does today).

### F4 Shapes and lfo (module `shape`)

```ebnf
shape       = "exp" | "log" | "s-curve" | "step" int | "linear" ;
change      = value                                              (* a plain value *)
            | value "->" value "over" dur [ shape ]              (* a ramp, linear unless it says *)
            | lfo ;
lfo         = "lfo" wave rate { "depth=" value | "center=" value | "phase=" value } [ "over" dur ] ;
wave        = "sine" | "tri" | "square" | "saw" ;
rate        = dur ;                                              (* the period: 2 bars, 1/2 bar, 4 beats *)
```

* Shapes map the ramp's time `t` in 0..1 to a progress: `linear` = t; `exp` = `(2^(k t) - 1)/(2^k - 1)` with k = 4 (slow start);
  `log` = its mirror (fast start); `s-curve` = smoothstep `3t^2 - 2t^3`; `step n` = `floor(t n) / (n - 1)` clamped (n >= 2 jumps;
  n steps). `linear` is the default and is not printed.
* `lfo` is a value anywhere a change takes one: `master: filter lp lfo sine 2 bars depth=1k center=3k over 16 bars`,
  `do inserts set target=Master param=delay.mix value=lfo tri 1 bar depth=0.2 center=0.3`. The value is
  `center + depth * wave(2π (t / period + phase))`, `wave` in -1..1 (`tri`, `square`, `saw` defined the usual way; `saw` rises).
  `depth` default 1 (in the param's own unit), `center` default 0, `phase` 0..1 default 0. It runs for `over dur`, or **until the
  section ends** (a program with no sections: until it is stopped or ends). Sent on the service pass like a ramp.
* A numeric param ranges as before (clamped by the registry/knob). Everything a ramp can drive, an lfo can.
* Canonical: `lfo sine 2 bars center=3000 depth=1000 over 16 bars` (params sorted by name, as everywhere).

### F5 Scale degrees and Roman numerals (module `scale`)

```ebnf
degrees     = "deg" "(" deg { deg } ")" ;                        (* a group term: deg(1 b3 5 b7), one slot, squeezed as [..] is *)
deg         = [ "b" | "#" ] int { "'" } { "," }                  (* 5' an octave up, 1, an octave down, b3 flat third, #4 sharp *)
            | "[" deg { deg } "]" | "<" deg { deg } ">" | "~" | "x" | "(" deg ")" ;
roman       = [ "b" | "#" ] numeral [ quality ] [ "/" ( "3" | "5" | "7" ) ] ;       (* as a pattern step *)
numeral     = "I" | "II" | ... | "VII" | "i" | ... | "vii" ;     (* upper case: major, lower: minor *)
quality     = "7" | "maj7" | "°" | "dim" | "ø" | "aug" | "sus2" | "sus4" | "6" | "9" ;
set         = "key" key                                          (* now also inside a section, first statements *)
voice-lead  = "voice-lead" [ "=" ( "on" | "off" ) ] ;            (* a harmony part's param; bare means on *)
```

* **A bare `b3` stays the note B in octave 3** (it always was). A flat or sharp degree is written inside `deg( )` only; there
  `b3` / `#4` are degrees (the lexer reads `#` + digit as a sharp degree **inside parentheses**; elsewhere `#` still starts a
  comment). A plain integer step (`bass 1 ~ 3 5`) is a degree as before; `deg( )` adds accidentals, octave marks and a way to
  write them as one group. `5'` is an octave up, `5''` two; `1,` an octave down (a comma **glued to the degree and followed by
  space, `)` or `]`**; a comma followed directly by another token (`[1,3,5]`) is the stack comma, as in any group).
* Degrees are relative to the program's `key` (its mode: `am` is the natural minor scale, `d:dorian` dorian, ...): `1` the tonic,
  `b3` a flat third *of that scale's degree 3*, `8` the tonic an octave up. They resolve at compile time to the part's octave.
* **Roman numerals** are steps of any tonal part: a chord in `harmony`, its root in `bass` / `melody` / `arp` (a chord in a
  single-note role plays the root; `arp` arpeggiates). Case sets the quality (`i` minor, `I` major); `°` / `dim` diminished, `ø`
  half-diminished, `7` a dominant or minor seventh by case, `maj7`, `aug`, `sus2` / `sus4`, `6`, `9`. `bVII` and `#iv` raise or
  lower the root a semitone. **`I/3` is the chord with its third in the bass (first inversion), `I/5` second, `V7/7` third.**
  They are read where a chord is read today (`harmony`, `chord(...)`'s place); the numerals `I..VII` in either case are
  reserved words (no `let I`).
* **A key change per section re-keys them:** `section drop 16 bars { key fm ... }`: a `key` statement as the first statement(s) of a
  section applies to that section's degrees, numerals and `markov` default. The program's `key` is the default and still
  tags the share. `assert key == dm in climax` (F12) reads this.
* **`voice-lead`** on a harmony part: each chord after the first is voiced (octave and inversion) for minimal total semitone
  motion from the chord before; the first chord is the root-position voicing in the part's octave. Deterministic: ties go to
  the lower inversion, then the lower octave. Without it chords are voiced as today.
* Canonical: numerals print as written but with the canonical case of their quality letters (`ii°`, `V7`, `bVII`, `I/3`); `deg(1 b3 5' 1,)` prints
  its degrees one space apart, accidentals and marks glued. `voice-lead` prints `voice-lead=on`.

### F6 Euclidean rhythms and polymeter (module `meter`)

```ebnf
part        = role [ "/" int ] pattern { param } ;               (* hihat/12 x x ~ x ... : a cycle of 12 sixteenth steps *)
call        = "e" "(" int "," int [ "," int ] ")" ;              (* as before: also `euclid`; checked: 0 <= k <= n, n >= 1 *)
```

* `e(k, n[, rot])` already exists as a term; here it is verified to be a pattern in its own right (`kick e(3,8)` is eight steps over
  the bar: tresillo), reported by `check` with its **ontology name when the ontology names it** ("e(3,8): tresillo") and
  exported in F14's JSON-LD as the ontology's Euclidean rhythm (class and pulses/steps/rotation) where the vocabulary has it.
* **`role/N`**: the part's *cycle* is `N` sixteenth steps instead of 16 (a bar). Its terms share the cycle equally, exactly as a
  bar's terms share the bar (so `hihat/12 x*3` is three hits in twelve sixteenths, `bass/12 e(5,12)` twelve steps). `N` is 1..=64.
  A part with `/N` repeats every `N` steps against the bar, so it drifts across barlines: a **polymeter**. Its hits are unrolled
  into the bar cycles of the net over the least common multiple of `N` and 16 steps (`lcm(N,16)/16` bars).
* **The program's period** is the least common multiple of all its parts' cycle lengths (in sixteenths, then in bars). `check`
  reports it ("period: 3 bars (hihat/12 against the bar)") in `warnings`-style notes and in `btw get`; it is a bound a
  net must fit under (64 bars, `MAX_BARS`): a program whose period exceeds it is an error that says which parts and the way out.
* Canonical: `hihat/12 x x ~ x`; `/16` is the default and is not printed.

### F7 Seeded chance (module `chance`)

```ebnf
term        = atom [ "!" ] [ "^" ] [ "*" int ] [ "?" prob ] ;    (* x*8?0.7: each of the eight hits is kept with chance 0.7 *)
prob        = number ;                                           (* 0..1, or 70% *)
atom        = ... | "choose" "[" pattern "]" "weights" number { number } [ "seed=" int ]
            | "maybe" "(" pattern "," prob [ "," int ] ")" ;      (* maybe(p, 0.3, 4) *)
```

* **A chance term is decided by a hash of (seed, bar, step, note)**, so it is a function of the text and the bar: the same
  program is the same notes every time, and every bar can differ. `seed` is the term's own, else the part's `seed=`, else a hash of
  the part's text (as `markov` does).
* `x?0.3`: the hit sounds in 30% of bars. The probability is stored exactly as typed (a `Rat`), the comparison is integer:
  `hash32 < p * 2^32` rounded down.
* `choose [a b c] weights 3 1 1 seed=4`: one of its options is played per slot per bar, chosen by the same hash walked over the
  cumulative weights; there are exactly as many weights as options (an error names both counts). A `choose` in a pattern is one
  slot like any term, and its seed is the term's own `seed=` or the part's.
* `maybe(p, prob, seed)` is the call form for a whole pattern (each hit of `p` kept with `prob`); equal to `p` with `?prob` on each
  hit.
* `check` reports every chance in the program (term, probability, seed) and the **period over which the pattern repeats**: the
  hash is of the bar, so a chance pattern cycles over `lcm` of its alternations only if it has none; the report says "differs every
  bar" and the net chains (64 bars unrolled) — chance in a net costs its bars: a program with chance and no sections unrolls 64
  bars (documented), a section unrolls its length.
* Spans: a hit that is not kept does not fire, so its marker does not light; a kept hit lights its token as usual.
* Canonical: `x*8?0.7`, `choose [a b c] weights 3 1 1 seed=4`, `maybe([x ~ x ~], 0.3, 4)`; `?30%` prints `?0.3`.

### F8 Mix moves (module `mixmoves`)

```ebnf
duck        = "duck" deck "under" deck "." role db { "attack=" time | "release=" time } ;   (* duck B under A.kick 6db attack=5ms release=120ms *)
mix         = "mix" "{" mixitem { ( "," | NEWLINE ) mixitem } "}" ;  (* in a section: a snapshot, restored at its end *)
mixitem     = deck "level" arith | deck "eq" ( "low" | "mid" | "high" ) arith [ "db" ] | "master" "level" arith | deck "pan" arith ;
db          = number "db" ;
time        = number ( "ms" | "s" ) ;
```

* `duck B under A.kick 6db`: while deck A's `kick` plays, deck B is attenuated by 6 dB with the given attack and release. A
  sidechain duck is **keyed from the part's own hits** (the compile knows when A.kick fires), so it needs no audio-rate detector:
  it is a gain envelope on B's strip driven from the hit times, with `attack` / `release` (defaults 5 ms / 120 ms). Map it onto
  an existing duck/sidechain/ducking verb or insert if the engine has one; **add an engine sidechain only if none exists**, and
  then as a small gain stage with a preallocated envelope state (no allocation on the audio thread).
* `mix { A level 0.7, B level 0.9, A eq low -3 }`: in a `section`, the values are applied on the section's first bar and **put
  back when the section ends** (the runtime reads each value before it changes it, through the host, and restores it on the
  section's last bar line; with no read verb, it restores the program's own prior mix line or the neutral value, and says so).
  Outside a section a `mix` block just sets. `eq low -3` is a gain in dB (the mixer's own scale is what the host maps to).
* Everything maps onto existing registry verbs (`mixer`, `inserts`); the check reports the calls (`describe`-style).
* Canonical: `duck B under A.kick 6db attack=5ms release=120ms` (params sorted); a `mix` block prints one item a line:
  `mix {` newline `  A level 0.7` ... `}`.

### F9 Vocals in code (module `sing`)

```ebnf
sing        = "sing" STRING "on" role [ "voice=" word ] { "bars=" int | "key=" key } ;   (* deck B: sing "words" on melody voice=alto *)
```

* `sing "lyrics" on melody voice=<name>` sets typed lyrics to the notes of the named part of the deck it is under (`deck B:`
  target, or the only deck that has that role): each syllable lands on a note, in order, wrapping if the lyric is shorter than the
  notes (it is **timed to the part's notes**, not to a new rhythm). It compiles to `vocal` calls (the neural vocal noun of
  feat/vocal-neural; the existing `vocal create "text" key=.. bars=..` sugar stays) carrying the part's notes.
* If the neural models are not present (the host reports it), `check` warns "the neural voice is not installed: falling back to the
  formant voice" and the line plays on the formant voice; the program is still valid. The environment says which; with no
  environment the line compiles and the diagnostic says nothing can be checked.
* Typed lyrics only: tests and examples use original lines of your own, never someone else's lyrics.
* Canonical: `sing "robots on the road" on melody voice=alto`.

### F10 Composition made literal (module `algebra`)

```ebnf
sexpr       = NAME                                               (* a section *)
            | "silence"                                          (* the empty section: the unit of both operations *)
            | "stack" "(" sexpr { "," sexpr } ")"                (* ⊗ parallel: at the same time *)
            | "then" "(" sexpr { "," sexpr } ")" ;               (* ∘ sequential: one after another *)
arrange     = "arrange" sexpr { sexpr } ;                        (* a bare list is `then` of them *)
letsection  = "let" NAME "=" sexpr ;                             (* a name for a composition *)
pexpr       = part | "stack" "(" pexpr { "," pexpr } ")" | "then" "(" pexpr { "," pexpr } ")" ;   (* over parts, after `deck X:` *)
```

* `stack(a, b)` plays both sections at once (a parallel ⊗): its length is the longer; the shorter is padded with silence (its
  lines end, its decks stop); the sections' parts of one deck and role may not both exist (an error names the deck and role). `then(a, b)` is
  one after the other (∘). `silence` is the zero-bar section: the unit of both. `arrange a b c` is `then(a, b, c)`.
  Section bodies desugar to `stack` of their lines (internally; not printed).
* Over **parts** (`deck A: stack(kick x ~ x ~, snare ~ x ~ x)`): `stack` is `|`; `then(p, q)` of two parts of the same role is the
  per-bar sequence of their patterns (`kick <p q>`), an error if the roles differ.
* **The laws, normalised in the canonical form** (so equal-by-the-laws programs have equal CIDs): `then` and `stack` are associative
  (nested ones flatten: `then(a, then(b, c))` = `then(a, b, c)`); `silence` is dropped (`then(a, silence)` = `a`); `stack` is
  symmetric (operands sorted by canonical text); a one-operand `stack` / `then` is its operand; `then(silence)` is `silence`.
  `then` is NOT commutative and is not reordered. Property tests: random re-association, unit insertion and operand permutation of
  `stack` keep the CID; swapping two `then` operands (when different) changes it; `arrange a b c` equals `arrange then(a,b,c)`.
* Update the categorical notes in `docs/BEATSCRIPT.md` (the monoid of evolve's endomorphisms, the symmetric monoidal structure of
  sections: objects are bar counts, `then` composes, `stack` tensors, `silence` is the unit, the braiding is the sort).

### F11 Typed ports on imports (module `ports`)

```ebnf
def         = "def" NAME "(" [ params ] ")" [ "in" ":" port { "," port } ] [ "out" ":" port { "," port } ] "{" ... "}" ;
port        = type ;                                             (* a port's name is its type's, or `in` / `out` when it is the only one *)
type        = "chords" | "key" | "bass" | "melody" | "drums" | "rhythm" | "bars" | "tempo" | "pattern" ;
emit        = "emit" [ NAME ] value-or-pattern ;                 (* in a def with out ports: what the port carries *)
wire        = "wire" NAME "." portname "->" NAME "." portname ;  (* wire changes.out -> walker.chords *)
```

* A `def` may declare a **boundary**: `in:` ports it needs and `out:` ports it makes, each a *type* from the list above. In the body,
  an `in` port's value is `$name` (its type's name: `$chords`, `$key`, or `$in` when it is the only one); an `out` port is made with
  `emit` (a pattern, or for `key` / `tempo` / `bars` a value). A def with no ports is the macro it always was.
* `wire a.out -> b.chords` glues `a`'s output to `b`'s input (the two defs, and imported defs too: `import "harmony"` brings its defs with
  their ports). A def called with its inputs wired expands with the emitted tokens in place of `$chords`; the wiring is a DAG
  (a cycle is an error naming the loop), each port wired once (an unwired `in` port is an error naming it), and
  **types must match**: `wire drums.out -> walker.chords` is "type error: `drums.out` is a drums pattern, `walker.chords` wants chords".
  `bass`, `melody` and `drums` are all `pattern`s (a `pattern` input takes any of them); `chords` takes chord / Roman-numeral terms only.
* **The cospan reading** (the notes in docs): a def with ports is an open system with an interface of inputs and outputs; `wire` composes
  two along a shared interface; imports are pinned by CID, so the composition is of exact modules, and the canonical text holds the CID.
* Canonical: `def walker(oct) in: chords, key out: bass {`; `wire changes.out -> walker.chords`.

### F12 Assertions (module `assertion`)

```ebnf
assert      = "assert" ( "ends"
                       | "period" "(" NAME ")" cmp int                    (* period(drop) == 32: an evolve's period, in bars *)
                       | "length" cmp clock [ "±" clock ]                 (* length == 3m30s ± 5s *)
                       | "level" levelexpr cmp levelexpr [ "in" NAME ]    (* level A < 2 * B in drop *)
                       | "key" "==" key [ "in" NAME ] ) ;                  (* key == dm in climax *)
clock       = number "m" [ number "s" ] | number "s" ;
levelexpr   = arith over deck names (A, B, master) read as their levels ;
```

* **Static** (proved by `check`, an error with the actual value when false): `ends`, `period(...)` (the evolve analysis),
  `length` (the program's bars through the tempo map: exact, F3), `key == k in section` (the section's key, F5). `assert ends` holds
  when the program ends (an `end`, or a timeline with no `loop`, or a bounded evolve).
* **Render-based**: `level A < 2 * B in drop` is estimated from an **offline render** of the program (`btw render`): the RMS of each
  deck's output within the section (or the whole program without `in`). `check` records it as *pending*; **`btw test`** (verb and
  `djctl btw test FILE`) renders and reports each assertion pass/fail with the measured values. A program whose static
  assertions fail is refused by `eval`; a pending one does not block it.
* Canonical: statements print in order, `assert length == 3m30s ± 5s`, `assert level A < 2 * B in drop`.

### F13 Performed tier (module `perform`)

```ebnf
input       = "knob" "(" int ")" | "pad" "(" int ")" | "fader" "(" deck ")" | "held" "(" ( "key" | note ) ")" ;
value       = ... | arith-with-inputs ;                          (* a control value may contain inputs: 200 + knob(3) * 7800 *)
guard       = "when" expr-with-inputs ;                          (* on a control line: when held(key) *)
```

* An input is a number in 0..1 read from the **controller bindings** (APC mini, PD705, MPK; the Aux/APC layout mapping stays as it is):
  `knob(3)` the third knob of the connected bank, `pad(5)` a pad (0 or 1, or its velocity 0..1), `fader(A)` deck A's fader, `held(key)`
  the lowest MIDI key held (note number / 127, 0 when none) and `held(c4)` 1 or 0 for that key.
* **Where they may stand: control values and control guards** (a filter cutoff, a level, a `do` param, a ramp's end, a `when`), not
  in patterns (a net is compiled once per section; an input is not known then). They are read **at bar boundaries** (each service
  pass for a ramp keeps the value of the bar it started) and, for an `evolve` rule's number, at **generation** boundaries (if the
  evolve's CID-from-the-rule law makes that unsound the feature says so and defers it).
* **Every value read is logged** (bar, input, value as read, quantised to 1/128) into a **performance**: an object
  `{type: djrust.performance, program: <program CID>, inputs: [...]}` with its **own CID**. `btw replay <perf CID>` feeds the log to the
  same program in place of the controllers and reproduces the controls exactly (a test on the fake clock demands identical calls).
  `live_store` saves performances; `djctl btw replay` replays offline into the render.
* Canonical: inputs print as written, `knob(3)`, `fader(A)`.

### F14 Ontology links (module `linked`)

```text
#! genre: https://ontology.bitwrap.io/ns#Techno       (or a CURIE: bw:Techno; stored as the full IRI)
#! tags: acid, driving, 303                            (terms or IRIs of the ontology)
```

* Two more `#!` header lines (comments to the language, read by `live_store` with the rights lines). `genre` names a genre of the bitwrap
  ontology (https://ontology.bitwrap.io/ns#); `tags` are terms of it (genres, instruments, scales, grooves, sections ...), by local
  name, label or IRI. **Validated against a cached copy of the vocabulary** (a file under `crates/djhost/data/`, refreshed from the
  published namespace when online; offline with the cache is fine): an unknown IRI or term is an error with a did-you-mean from the
  cache. The cache is the source of truth for tests; nothing in the test suite touches the network.
* **Exported as JSON-LD** alongside the program object: `btw jsonld {id | text}` and `djctl btw jsonld FILE` give a document typed
  `djrust:Program` with `@id` the program CID, the genre, tags, key, tempo, sections (name, bars), the licence, `forkedFrom` and
  the Euclidean terms linked to the ontology's Euclidean rhythm class (F6), so a saved program is linked data (`<cid>.jsonld` next to
  `<cid>.beatscript` in the store).
* The language never reads the header lines; they are not in the CID (comments are not), exactly as `#! license:` is not. (A tag
  change is a new object of the same program, as a new licence is.)

## 4. Integration (the orchestrator's, not the workers')

Workers commit on their own branches. The orchestrator merges them one by one into `feat/btw-extensions` in this order:
F2+F1, F5, F6+F7, F10, F11, F8, F9, F14, then F3+F4, F12, F13, resolving conflicts by keeping both sides' work and re-running the
full tests after each. After all land: the showcase, a fresh-eyes grammar review for ambiguities and conflicts, then the docs are
folded into `docs/BEATSCRIPT.md`, the branch is fetched/rebased onto the newest `origin/feat/btw-language`, pushed, and the release is
built (not installed).

## 5. Decisions log (the orchestrator's; workers' are in their own docs)

* Arithmetic is evaluated on rationals, `/` exact; `let r = 3*2` is now 6, not the degree 3 twice (the only deliberate change of
  old meaning).
* `b3` outside `deg( )` remains the note B3; flat and sharp degrees live in `deg( )`.
* Beat offsets are 0-based like bars.
* BPM ramps are linear in beats (the bar clock is exact; seconds are an integral).
* `stack` of sections of unequal length pads the shorter with silence.
* Roman numerals use case for quality; `I/3` is first inversion.
* Chance is a hash of (seed, bar, step, note): a function of the program text and the bar, never a runtime random.
* Tokens that must touch (`x?0.3`, `hihat/12`, `e2!`, `A.kick`, `6db`, `==`) keep touching when a macro puts a value in place of a
  `$name` and when an import re-spans a def or a let: `lex::put_at` gives such a token the empty span at the end of the place it
  stands in (found by joining ports with chance and polymeter). A construct whose tokens must touch goes in `lex::touches` AND in
  `tests/common::glued_pair`; the tests of `tests/ext_cross.rs` join features that were built apart.
* A slot written glued to what comes before it (`hihat/12`, `x?0.3`) and a choice's weights also take arithmetic in parentheses
  (`hihat/(4 + 8)`, `x?(1 / 4)`, `weights (1 + 1) 1 1`), so that a def's arithmetic argument, which is put in place wrapped in
  parentheses, fits where `$n` stands; they fold to the number. A call's arguments do not count `<=` / `>=` as the brackets of
  `<a b>`. (Found by joining F2 with F6, F7 and F11 in `tests/ext_cross2.rs`.)
* F5 meets the rest (`tests/ext_cross3.rs`): the octave marks `5'` and `1,` (token `OctDown`) are in `lex::touches` and
  `tests/common::glued_pair` (a test keeps the two lists in step); `1,` is a mark only directly inside `deg(`; a `$name` before a comma
  in `deg( )` counts as a degree; `(arith)` in `deg( )` is a computed degree; a `chords` port and a `harmony` part read numerals alike;
  a trailing comment written `#I like it` is a sharp numeral, so the error that follows says a comment starts with `# `.
* F10 meets F5, F6 and F7 (`tests/ext_cross4.rs`): a stretch of a stack sounds in one key, the first keyed section's in the order of the normal
  form, else the program's; two sections that play together in different keys are an error naming both; a section with no key follows the
  stretch it is in; `Compiled::section_key(name)` takes a section's or a segment's name and answers every name `sections_at` gives. A later
  stretch of a stack unrolls its chance over `phase + bars`, so it goes on from where the section was (it used to repeat the first bars).
  `then` of parts needs one cycle (`hihat/12`). A def's body is a macro and prints as written: `stack` is sorted only where it is read at
  the top level. A `$name` where a section is expected says so (`arrange` and `let` are not in defs or repeats).
* F8 meets F2, F1 and F10 (`tests/ext_cross5.rs`): a mix item's arithmetic is folded to the number it comes to in the canonical text
  (`1/2` would read back as a division); a duck's depth and times are literals with their unit (`(3 + 3)db` is no number with a
  unit: arithmetic is for a mix); a mix or a duck takes `at bar 4 beat 2` and `for 2 beats` (the render places them on the beat);
  in a stack a mix is held to the bar where its own section ends (not the stretch's), a copy carried into a later stretch is not a
  second control, and two mixes holding one knob at once are an error; at a bar line every put back runs before any line fires; a
  `when` guard after a mix goes after its brace.
* F9 meets F2, F10, F5, F6, F7, F11 and F1 (`tests/ext_cross6.rs`): a `sing` line reads the part as compiled, so a degree or numeral
  part, a chance, a polymeter and a wired value are sung as they play; a def's string, `voice=$v` and `bars=$n` (arithmetic too:
  `bars=1 + $i`, folded to the whole number) come through a def, a repeat and an import; a `when` guard after the line picks the
  repeat's copies that sing. In a stack of sections of unequal lengths the line fires once, in the stretch where its section begins,
  and sings over the whole section with the notes each stretch plays (it used to fire again in every stretch, over the first one's
  bars only); the controls of the lines come after the stretches' nets are built. `Control.notice` is said in the log where the line
  fires, with the bar it was due at (`bar 2.750:` between bar lines). A `repeat` now says an error only a later copy comes to
  (`bars=$i / 2 + 1`: copy 2 is 3/2); it used to drop the copy's line without a word.
* F14 meets F6, F10, F5, F2 and the examples harness (`tests/ext_linked.rs`, `djhost/tests/ext_linked.rs`): the JSON-LD names and links Euclidean
  rhythms from F6's table (`meter::euclid_names`, role-blind: `kick e(3,8)` is the ontology's "house snare" shape; `name` only when the ontology has one
  individual of the shape, `sameShapeAs` every one), and counts the `e()` calls of the **expanded song** (a def's `$k`, arithmetic, `/N` parts, chance
  terms; `Compiled::linked`), not the text. Its `sections` are the song's sections as F10 places them (`Lowered::placements`: name, `start`, bars, the
  `key` the section sets), never the stretches a stack cuts the song into (`groove+wash`). F6's `check` notes ("the ontology's house snare rhythm")
  are always there, so a test of the header lines looks at the diagnostics about the header only. A `#! skip-test:` line may be anywhere in the header
  block, among `#! genre:`, `#! tags:` and the rights lines: `examples_beatscript` reads it from the block, and `live_store` ignores it (it is not kept).
* F3 and F4 meet F10, F8, F1, F2, F11, F5 and F6 (`tests/ext_cross7.rs`, `djhost/tests/beatscript_tempo.rs`): the tempo of a stretch of a stack is the one its
  sections agree on, or the one of the only section that sets any; two sections that play at the same time and set different tempos on one bar are an
  error naming both (like two keys: `tempo_clashes`; sections that follow each other never clash, and a section's tempo overrides the program's own
  default). An lfo with no `over` ends where **its own section** ends, which in a stack of unequal lengths is not the end of the stretch its line sits in
  (the same `mixmoves::section_span` that holds a mix). A mix item is one number set once: a ramp (`A level 0 -> 1 over 2 bars`) or an lfo in it is an
  error that says to use a control line (`deck A: level 0 -> 1 over 4 bars exp`). The tempo map is settled **before** the mix lines are read, so a duck's
  pump is chosen at the tempo a section sets, not only at the top-level one. A song with a tempo curve and a mix or a duck renders offline: the moves are
  placed on the beats the program says, and each carries the tempo of the step it falls in, so the renderer's clock puts it on the second the curve gives that
  beat (a `djhost` test hears the pump dip B after the chorus's kicks, at the seconds the map gives their beats). The nets of a section are made at the tempo
  the curve has where it begins and at its key, so a program with both keeps one net per section, while a constant tempo keeps the nets (and CIDs) it had.
  `length_report` takes the song's length from `song_bars`, so a stack is as long as the longer section; `meter::program_period` does not move with a curve.
  Edge left as it is: a flat program too long for one net (a chain of 64-bar nets, `flat_clock`) follows its tempo map once from where it was evaluated, so
  after the chain starts again the held tempo of the curve's end is what the runtime sends (the nets themselves restart at the tempo they were made at).
* F12 meets F3, F10, F5, F6 and F2 (`tests/ext_assert_cross.rs`, `djhost/tests/beatscript_test_tempo.rs`): `assert length` is `Compiled::length_seconds` (the tempo
  map's closed-form integral, so exact under a curve, a section's tempo and a stack; a loop has no length and says its lap through the same map; the message says
  `(through the tempo curve)` when the tempo moves). `in NAME` names a **section**, also one inside a stack (the segment `long+short` is the stretch, `in short` and
  `in long` are what a person wrote): the windows of a level assertion are every stretch the section is in, and a wrong name lists the sections, not the stretches. A
  section with no `key` of its own that is stacked under one that has one answers the key of the first stretch it plays in (`section_key`); two stacked sections in
  different keys are F10's error and no assertion is judged. The period is `meter::program_period` (a chance part unrolls over its net's own period). A `let`'s value, an
  imported one too, folds into an assertion (`assert period == n * 2 - 3`); `bars` and the other language words are no names for one. `btw test` reports the seconds each
  level was measured over (`windows`): a section's window is `tempo_map().seconds_at(start)..seconds_at(end)` because the render of a curve carries each step's tempo on
  every event, and the measure is the same as a reading of the stems over those seconds. `btw`'s verbs are now 20 (`generation instruments jsonld terms test` after the
  fifteen): the live action list and the registry agree.
* F13 meets F4, F8, F9, F11, F2, F1, F3, F10, F12 and F14 (`tests/ext_perform_cross.rs`, `djhost` `beatscript_test` / `beatscript_render` / `live_store`,
  `djctl/tests/beatscript_test_cmd.rs`; docs/beatscript/ext-perform.md "Meeting the others"): a shaped ramp's ends and an lfo's `center=` / `depth=` are control values
  (an lfo re-reads them at each bar line and goes on from the new middle; `phase=` stays literal); a `when` after a mix, a duck or a sing line is
  decided while the program plays (those lines are compiled apart from `Compiler::control`, so the guard used to be dropped silently), while a mix
  item takes no input and the error names the control line to write; a port carries a number fixed when the program is read, an assertion cannot
  read an input, and `btw test` / the offline render read every input as 1/2 and say so (a duck or mix with a performed `when` is in the render
  when its guard is true at 1/2; level, filter and `do` lines are not in a render at all); a sub-bar control reads the value of its bar; the tempo
  words and every time word refuse an input at the call. A performance's program CID is of the canonical text, so links and their changes do not
  detach it. `btw`'s verbs are now 22 (`perform replay` after the twenty).
* Sets meet the extensions (the merge of `origin/feat/btw-language` into this branch, `tests/ext_cross8.rs`; docs/BEATSCRIPT.md "Sets and the extensions"):
  a time is a number and its unit glued to it everywhere (`3m30s` in an `assert length` and in a set's `until 20m`, `1h30m`: `h` joins the units
  and the set's own `Secs` token is gone, so no pinned CID of `tests/sets.rs` or of an old program moved); `set`, `until` and a set's `mix N bars
  with` are words read by position and reserve nothing (`let set = 2`, `section mix` and a `mix { }` block all keep reading), `set` is in
  `STATEMENT_WORDS`, and the highlight knows them; a set has a tempo map (each track's, curve and all, where it plays alone, and each mix's ramp),
  so the runtime follows it like any program's curve and `length` and the plan read it; a track's lfo, duck and sub-bar lines (`every 2 beats`)
  stop with their track on the set's clock. Not done: a sung loop of a track stops with the set, not with its track; a mix ramp is linear.
