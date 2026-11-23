# BeatScript extension F2: arithmetic

Every place the language takes a number takes arithmetic. It is worked out **once, when the program is read**, over exact
rationals, so a program stays total and deterministic: there is no loop, no recursion, no clock and no state in it.
Module `crates/djlive/src/expr.rs` (the reader and evaluator: `parse_arith`, `parse_expr`, `eval`; the parser's side:
`impl Parser { starts_arith, arith_num, paren_int, guard }`), hooks in `parse.rs`, `expand.rs`, `print.rs`.
Example: `examples/beatscript/arith.beatscript`. Tests: `crates/djlive/tests/ext_arith.rs` (and the unit tests in `expr.rs`).

## Grammar

```ebnf
arith       = sum ;                                   (* + - * / %, parentheses, unary -, min(...) max(...), numbers *)
sum         = term { ( "+" | "-" ) term } ;
term        = unary { ( "*" | "/" | "%" ) unary } ;   (* `%` is written with a space: `$i % 4` (`4%` is 4 percent) *)
unary       = [ "-" ] primary ;
primary     = number | "$" NAME | "(" arith ")" | ( "min" | "max" ) "(" arith { "," arith } ")" ;
expr        = or-and-not-comparison over arith and words (docs/BEATSCRIPT-EXTENSIONS.md, section 2) ;

let         = "let" NAME "=" ( arith | token | pattern ) ;       (* arithmetic wins when every token is numeric, there is at
                                                                    least one operator, parenthesis or min/max call, and it
                                                                    is the whole value: `let n = 2 * 4` is the number 8 *)
value       = arith ;                                            (* tempo, swing, a param's value, a control value, a `do`
                                                                    value, a ramp's two ends, a generator's argument,
                                                                    `at bar N`, `every N bars [from N]`, `for N bars`,
                                                                    `before NAME N bars`, `over N bars`, `repeat N [every N bars]`,
                                                                    `section NAME N bars`, `end at bar N`, `queue jump N`,
                                                                    `load pair X group N`, `evolve every N bars`, `stop when gen == N` *)
param       = NAME "=" ( arith [ ".." arith ] | word ) ;          (* vel=(60 + $i * 4)..100   oct=min(3, 4) *)
atom        = ... | "(" arith ")" ;                               (* a degree computed: bass 1 ($i + 1) 5 *)
term        = atom [ "!" ] [ "^" ] [ "*" ( int | "(" arith ")" ) ] ;   (* x*4   x*($i % 3 + 1) *)
arg         = ... | arith ;                                       (* e($i % 4 + 1, 8)   markov(am, 1 / 2, 7) *)
guard       = "when" expr ;                                       (* after a part line, a control line, a `do` line or a def call *)
line        = [ when ":" ] [ target ":" ] body [ guard ] ;
```

```text
tempo 60 + 6 * 2
let total = 4 * 4
deck A: kick e($i % 4 + 1, 8) | bass 1 ($i + 1) 5 vel=(60 + $i * 4)..100 oct=3 - 1
repeat 8 { at bar $i * 2 + 1: master: filter lp 300 * ($i + 1) when $i % 4 == 3 }
```

## Semantics

* **Evaluated once, at compile time.** `$i` and `$n` (a `repeat`'s index and count) and a `def`'s `$params` are put in place as
  tokens before a line is read (as they always were), so `$i * 2 + 1` is `3 * 2 + 1`. A `$name` that is still there when an
  expression is evaluated is an error at its place: "`$i` has no value here" with the help that says where `$i` and `$n` exist.
* **Exact.** `/` is exact (`1 / 4` is a quarter, `7 / 2` is not 3); `%` is Euclidean (`-1 % 4` is 3); an overflow or a division
  by zero is an error with the line and column of the operand, never a wrap. A number that must be a whole number (a bar, a
  count, a degree, a repeat count, `int` params) says so when the result is not (`repeat 3 / 2`: "how many times is a whole number"; a
  position, by contrast, may be a rational once F1 is in: `at bar 3 / 2` is `at bar 1 beat 2`, docs/beatscript/ext-timing.md).
  Decimals are read as the rational they were typed as (`0.3` is 3/10), with at most six places.
* **A lone literal is not arithmetic.** A number with no operator after it is read exactly as before: its spelling (`2k` is
  2000, `50%` is 0.5) and its exact `f64` value (`0.123456789` is not squeezed through a rational). The parser reads arithmetic
  only when the number is followed by `+ - * / %`, or starts with `(`, `$name`, `min(`, `max(`. Where numbers stand side by
  side the literal reader is kept: an evolve's `transpose <0 -5 7>` and `rotate 1`, an `x*4` repeat count, a `gen % 4 == 3`.
* **`let` folds.** `let n = 2 * 4` is the number 8, a scalar put in place of every later `n` (also inside defs). A result that
  is not whole becomes a decimal token that reads back exactly (`let q = 1 / 4` is `0.25`; `1 / 3` is `0.3333333333333333`,
  which reads back as 1/3; the canonical text of the `let` says it exactly, `let third = 1 / 3`, see "Canonical form"). A result that no decimal can carry (`1 / 7000000`) is an error. An earlier scalar is a number in
  the expression (`let m = n + 1`). If the expression is not the whole value (`let r = 3*2 4`), or has a word in it (`c2 + 1`),
  or has no operator, parenthesis or call (`let r = 3 3`), it is not arithmetic and the old reading stands (a token, a pattern).
  **`let r = 3*2` is the number 6 now** (it was the degree 3 played twice): write `let r = [3 3]` or `let r = 3 3` for that.
* **`when` guards.** `deck A: kick x ~ x ~ when $i % 4 == 3`, `at bar $i: master: filter lp 300 when $i % 2 == 0`,
  `drop(A, 8) when $i != 0`, `do vocoder set mix=1 when $deck == A`. A guard is evaluated **where the line is expanded**, first thing:
  a false guard removes the line before its time word is turned into a bar, before the def it calls is read, before the part
  is gathered and compiled, so a guarded-off line may name things that do not exist in this repetition (a section, a second
  `kick`, an `at bar 0`). Syntax errors in the line are still reported (the line has been read). Inside a `repeat` or a `def`
  the guard sees `$i`, `$n`, `$params`; at the top level it is a constant (`when 1 == 1`), allowed and kept in the canonical
  text. A guard that is not a condition (`when 1`), or has an unbound `$name`, or divides by zero, is an error at its place.
  Words compare with `==` and `!=` only (`when $deck == A`). A kept line is the line without its guard: the guard is not part
  of the key that identifies a control across evaluations (editing a guard that still holds keeps the control's state).
  `when` cannot start a pattern step (`kick x ~ when ...` ends the pattern there). An evolve's own `when gen % 2 == 1` is
  unchanged.
* **A def's arithmetic argument keeps its grouping.** `cuts(1 + 2)` with `at bar $a * 2` is `(1 + 2) * 2`: an argument that is
  all numbers with an operator outside any parenthesis, or a `min(...)` / `max(...)` call, is wrapped in parentheses where it is
  put in place (not a lone negative, not the plain ratio `1/4`, which a `do` value reads as a word, except in a glued slot).
  Where it lands decides: in a **glued** slot (`x*$k`, `x?$p`, `hihat/$n`, `weights $w 1 1`) every such argument is wrapped,
  a ratio too (`gt(A, 8 / 2)`); beside an operator it is wrapped as above; in a **step of a pattern** (`bass $p ~ 5`) it is
  wrapped unless it is only numbers and `*` at the top (`1*4`, `1*2 3`, `(1 + 2)*3`), which keep the meaning they had before
  arithmetic existed (the degree 1 four times; two terms), so no old program moves.
* **`$i` and `$n` inside a def.** A `repeat` in a def's body has its own `$i` and `$n`: the def leaves them for it. (A
  parameter of the def named `i` or `n` wins, as it did.) Outside a repeat they say where they exist, and are never
  suggested for a misspelt parameter.
* **A computed degree.** `(expr)` is a pattern step: the scale degree it comes to (1 or more). A step word before a `(`
  (`x`, a note such as `c2`) is a step, and the `(` opens the next one: `bass c2 ($i + 1)`. `name(` is a call only for a name
  that is not a step word.

## Canonical form

* A single literal keeps today's spelling (`2k` is `2000`, `50%` is `0.5`).
* Any other **number** (tempo, swing, a param's value, a control value, a `do` value, a ramp end, a generator argument) **is the
  number it comes to** when nothing in it is read while the program plays: `tempo 60 + 6` is `tempo 66`, `level 0.25 + 0.25` is
  `level 0.5`, `e(1 + 2, 4 * 2)` is `e(3, 8)`, `vel=(60 + 12)..100` is `vel=72..100`, one program each (L-08; they used to stay
  expressions, so equal values were two CIDs). A rational prints as its decimal when it has one of at most six places (`0.7`,
  `0.375`) and otherwise as the exact division, `1 / 3`, never a rounded decimal: `level 1 / 3` is `level 1 / 3`.
* A number or a guard with an **input** in it (`knob(3)`, `held(key)`, `fader(A)`) cannot be folded: it prints as its expression,
  `Expr::canonical()`: one space round a binary operator, parentheses only where precedence needs them, words and calls as
  written, and every constant piece of it folded as above: `level 200 + knob(3) * 7800`, `* 0.7` (not `* (7 / 10)`),
  `when knob(1) > 0.5` (not `> 1 / 2`), `knob(1) * (0.5 + 0.2)` is `knob(1) * 0.7`. A comparison is not folded (a constant guard
  such as `when 1 == 1` is kept, as written); only the arithmetic inside it is.
* A **count, a bar, a length, a degree** (`at bar 2 + 2`, `repeat 2 + 1`, `section drop 4 * 4 bars`, `x*(1 + 1)`, `(1 + 2)`)
  is a whole number of its own place and folds to it: `at bar 4`, `repeat 3`, `section drop 16 bars`, `x*2`, `3`. A duration or a
  position has the canonical form of docs/beatscript/ext-timing.md (`over 2 beats`, `at bar 16 beat 3`).
* A `let` of arithmetic prints as its value (`let n = 8`, `let half = 0.5`), a ratio that is no short decimal as the fraction it is
  (`let third = 1 / 3`: it folds back to the same number, so the text is a fixed point, and it is not the rounded
  `0.3333333333333333`, which a denominator past a million would have turned into another number), and its uses as the value. A
  lone use of such a name is said as the fraction too (`level third` is `level 1 / 3`, the text `level 1/3` has, one CID: the reader
  notes which names are a ratio that came of arithmetic, `LetDef.ratio` and `Parser::ratios`); a `let` of a *literal* is never turned
  into a fraction, whatever digits it has (an old program's `let old = 0.3333333333333333` and its uses keep their spelling and CID).
  The kept-as-tokens bodies of a `def` and a `repeat` print a use as the number the reader put in place, as before.
* A range end that is arithmetic with an input is parenthesised so that the text reads back as it was; one that folded to a
  plain number is that number: `vel=72..100`.
* A `do` value that folds to a ratio that is no short decimal keeps its parentheses (`mix=(1 / 3)`): `1/3` alone is the
  registry's ratio word. A short decimal is a number (`mix=(1/4)` is `mix=0.25`).
* A **mix** item is one number set once: it is the value it comes to, and it is exactly a decimal of at most six places or an
  error ("a mix value has at most six decimals"), as a chance and a weight are: `A level 1 / 3` is not `0.333333` (it was rounded
  silently, so those two were one program).
* A guard prints after the line: `... when $i % 4 == 3` (`Expr::canonical()`).
* The bodies of a `def`, a `repeat` (and an `evolve` in one) print as the tokens they were written in, one statement a line,
  with the printer's spacing rules, as always: `at bar $i + 1: master: level 0.5 when $i % 4==3`. That is not pretty, it is a
  fixed point and it is the same whatever the spacing was; the existing canonical text of bodies must not move.
* Random reformatting keeps the CID, a semantic edit changes it, the canonical text is a fixed point (tests).

## Decisions

* **Where the expression is kept.** An AST `Num` is `Copy` and used everywhere, so it carries its canonical expression text as an
  interned `&'static str` (`Num.canon`; `expr::intern`: a process-wide table, equal texts stored once, leaked for good: an
  editing session adds a few thousand short strings at most). The alternative, a non-`Copy` `Num` or a table threaded through
  the printer, would have touched every consumer. `Num::lit(value, int, span)` makes a literal.
* **What prints as an expression and what folds.** Contract: "a single literal keeps today's spelling, any other expression
  prints with `Expr::canonical()`". Durations and positions have a canonical form of their own (F1: `N bars`, `bar W beat B`)
  and counts, bars and degrees are whole numbers, so those fold. **Revised (L-08, second review): the free numbers fold too**, to
  the number they come to, wherever nothing in them is read while the program plays; only an expression with an input in it
  stays an expression (constant pieces folded). Why: `master: level 0.25 + 0.25` and `level 0.5` were two CIDs, the contract
  folded some places and not others, and the printed text of a ratio was `(7 / 10)`. No program written before the extensions
  has arithmetic, so no pinned CID moved; the canonical texts the extension tests had pinned were re-pinned. `Num.canon`
  now holds the folded text (`0.5`, `1 / 3`) or the expression with an input; `Rat::decimal6` is the one printer of a number.
* **`Rat::from_f64` reads a decimal of at most six places exactly** (`0.999999` is 999999/1000000: the continued fraction stopped
  at the shorter 999998/999999, within 1e-12, which the canonical text then printed as a fraction).
* **`let` arithmetic precedes the token and pattern readings, but only for the whole value.** A leftover (`3*2 4`) is a pattern
  as before. An error inside something that looks like arithmetic (a division by zero, an unclosed parenthesis) is reported
  once, at its operand, and the name stands for 0 so that nothing else is said of it.
* **The `do` ratio.** `ratio=1/4` (number, `/`, number, nothing arithmetic after it) is the registry's word `1/4` as it was;
  `ratio=1/4 * 2` and `ratio=(1/4)` are arithmetic.
* **Guards on settings and blocks.** `when` belongs to a line (a part line, a control line, a `do`, a def call). Not after
  `tempo`, `key`, `let`, `def`, `section`, `arrange`, `end` or inside an evolve (an evolve's rule has its own `when gen`).
* **Arithmetic reads decimals as rationals with at most six places;** a number with more is an error when it is inside an
  expression ("too large or too precise for exact arithmetic"), and kept as typed when it stands alone.
* **A guard is evaluated with no hooks at expansion:** a name that is not `min` / `max` is an error in a compile-time expression. (The performed tier,
  F13, has since given a control line's `when` its inputs: `when knob(3) > 0.5` on a control line is decided while the program plays, and a part
  line's guard, a def call's and a `let`'s refuse an input; see docs/beatscript/ext-perform.md.)
* **Def arguments are wrapped, not evaluated.** A def is a macro; the wrapping only protects the grouping of an argument.
  An argument `4/2` that is a plain ratio of two numbers is not wrapped (a `do` ratio is a word), so `1 / $x` with `x = 4/2`
  is `1 / 4/2`.
* **Highlighting:** `when`, `and`, `or`, `not` are keywords; `min(` and `max(` are drawn as generators (a name before `(`).
* **The slots that are written glued to what comes before them take parenthesised arithmetic** (found when F2 met F6 and F7): a
  part's cycle `hihat/(4 + 8)`, a chance `x?(1 / 4)`, a choice's weight `weights (1 + 1) 1 1`. Without it a def's arithmetic argument
  could not go in place of `$n` in `hihat/$n`, `x?$p`, `weights $w 1 1` (a def puts such an argument in place in parentheses,
  a ratio too: `x?$p` with `1 / 4` is `x?(1 / 4)`).
  They fold to the number they are (`hihat/12`, `x?0.25`, `weights 2 1 1`): a cycle is steps, a chance and a weight are one
  number each. A bare `x?1/4` is still not read (a `1/4` argument is not wrapped, because a `do` value reads it as a word): write
  `x?(1/4)` or give the def `0.25`.
* **A comparison in a call:** a call's arguments are cut at the commas outside any bracket, and `<` `>` count as the brackets of
  `<a b>`; `<=` and `>=` (`=` glued after them) are comparisons and do not. A lone `<` or `>` in an argument is still a bracket:
  `gate(2 < 3)` says the call is never closed and that a comparison belongs in the def (`when $x < 2`).
* **An `emit` takes no guard** (`emit x ~ x when $i == 0` is an error that says a pattern term was expected): an emit is the value of
  a port, and a value that is sometimes not there has no type. Put the guard on the line that uses the value.

## Not done

* Arithmetic in an evolve's own arguments (`transpose (1 + 1)`, `rotate 1 + 1`) and in its `when gen` guard: they stay
  literals, because numbers stand side by side there.
* Arithmetic on pitches and keys (`c2 + 5`, `am + 2`) and on patterns: numbers only.
* A guard on a `let`, `def`, `section`, `tempo`/`key`/`swing`, or a section's header; guards on the lines of an `evolve` rule.
* Strings and string building; no functions beyond `min` and `max` (`floor`, `abs`, `round` would be small and are left out until a
  program needs one).
* (Done since by F13: `expr::Hooks` gives a control value and a control line's `when` their inputs, docs/beatscript/ext-perform.md.)
