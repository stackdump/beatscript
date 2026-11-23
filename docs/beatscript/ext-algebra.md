# F10: composition made literal (`stack`, `then`, `silence`)

*Extension of BeatScript (docs/BEATSCRIPT-EXTENSIONS.md, F10). Module `crates/djlive/src/algebra.rs`; tests
`crates/djlive/tests/ext_algebra.rs`; example `examples/beatscript/algebra.beatscript`. When the extensions are folded into
docs/BEATSCRIPT.md, the grammar goes into its EBNF, the semantics into "Functions and structure", the canonical rules into
"Canonical form", and the block under "What the laws promise" replaces the categorical remarks (it is ready to paste).
Done: docs/BEATSCRIPT.md has all of it ("Composition: stack and then", "Categorical notes", the canonical-form rows).*

A song is a composition of sections. Until now the only composition was the list in `arrange`. This extension makes
both ways of putting sections together literal and gives them a normal form:

```text
section groove 8 bars {
  deck A: kick x ~ x ~ | snare ~ x ~ x
}
section wash 8 bars {
  deck B: harmony <chord(am) chord(f)>
}
section riff 4 bars {
  deck B: melody <a4 c5 e5 g5>
}

let verse = stack(groove, wash)                    # a name for two sections played together
arrange intro stack(then(riff, riff), verse) coda  # intro, then the verse with the riff twice over it, then the coda
```

* `stack(a, b)` plays `a` and `b` **at the same time** (the tensor). It is as long as the longer; the shorter is
  padded with silence: its decks stop where it ends.
* `then(a, b)` plays `a` and then `b` (composition). `arrange a b c` is `then(a, b, c)`.
* `silence` is the empty section, zero bars, the unit of both.
* The same two words work over **parts** after `deck X:`: `stack` is `|`, and `then` of parts of one role is the
  per-bar alternation of their patterns.

## Grammar

```ebnf
sexpr       = NAME                                               (* a section, or a let that names a composition *)
            | "silence"                                          (* the empty section: the unit of both operations *)
            | "stack" "(" sexpr { "," sexpr } ")"                (* parallel: at the same time *)
            | "then" "(" sexpr { "," sexpr } ")" ;               (* sequential: one after another *)
arrange     = "arrange" sexpr { sexpr } ;                        (* a bare list is `then` of them *)
letsection  = "let" NAME "=" sexpr ;                             (* a name for a composition; a lone name or `silence` is the scalar let *)
pexpr       = part | "silence"
            | "stack" "(" pexpr { "," pexpr } ")"
            | "then" "(" pexpr { "," pexpr } ")" ;               (* over parts, after `deck X:` *)
parts       = pexpr { "|" pexpr } ;                              (* the old `part { "|" part }` is the case of plain parts *)
```

A newline is allowed after `(` and `,` and before `)` of a section or part expression, so a long composition can run
over lines (the canonical form puts it on one). One example line per addition:

| addition | example |
|---|---|
| `stack` over sections | `arrange intro stack(groove, riff) coda` |
| `then` over sections | `arrange stack(then(riff, riff), verse)` |
| `silence` | `arrange intro silence verse` (is `arrange intro verse`) |
| a named composition | `let verse = stack(groove, wash)` |
| `stack` over parts | `deck A: stack(kick x ~ x ~, snare ~ x ~ x)` |
| `then` over parts | `deck A: then(kick x ~ x ~, kick x x x x)` (is `kick <[x ~ x ~] [x x x x]>`) |

`stack`, `then` and `silence` are reserved words (`parse::EXT_RESERVED`; the other words of the extensions are reserved by position, BEATSCRIPT.md "Decisions"): none names a section, a
let or a def.

## The laws, and the canonical form

An expression is **normalised as it is read**, by the axioms of a symmetric monoidal structure, and the canonical
text is the normal form. So two programs equal by the laws have the same text and the same CID.

| law | rule | example (all one program) |
|---|---|---|
| associativity | a `then` in a `then` and a `stack` in a `stack` flatten | `then(a, then(b, c))` = `then(then(a, b), c)` = `then(a, b, c)` = `arrange a b c` |
| unit | `silence` is dropped | `then(a, silence)` = `stack(silence, a)` = `a` |
| one operand | a one-operand `then` / `stack` is its operand | `then(a)` = `stack(then(a))` = `a` |
| empty | with nothing left, the result is `silence` | `then(silence)` = `stack(silence, silence)` = `silence` |
| symmetry | the operands of a `stack` are sorted by their canonical text (the braiding is the sort) | `stack(b, a)` = `stack(a, b)` |
| order | `then` is **not** commutative and never reordered | `then(a, b)` is not `then(b, a)` |

Printing:

* A plain `arrange a b c` prints as `arrange a b c`, exactly as before (every existing program keeps its canonical text
  and CID). `arrange then(a, then(b, c))` prints the same. A top-level `then` is printed as the list; a `stack` (or a
  `then` inside a `stack`) is printed with its call: `arrange intro stack(drums, then(riff, riff)) coda`.
* Operands are separated by `, `; `stack(` and `then(` are written glued to their paren.
* `let both = stack(verse, intro)` prints `let both = stack(intro, verse)`. A let whose composition is a lone name
  (`let first = then(intro, silence)`) *is* the scalar `let first = intro` by the laws, and is read and printed as it:
  the value is put where the name is used (a rewrite before the scalar lets are read, `algebra::collapse_lets`).
* Over parts: `stack(...)` flattens, drops `silence` and **sorts its parts by their canonical text**, then prints as
  the `|` list (`deck A: kick x ~ x ~ | snare ~ x ~ x`). A `|` list as written is not reordered (it is text that
  exists, and the order of parts in a line has always been in the CID). `then(...)` of parts is desugared to the
  alternation and printed as it: `kick <[x ~ x ~] [x x x x]>`; a one-term operand goes into the alternation as itself
  (`then(kick x, kick ~)` is `kick <x ~>`).
* `stack` is **not idempotent**: `stack(a, a)` keeps both copies (they are two sections, and they will usually clash,
  below).

What the laws do not do: **names are opaque**. `let both = stack(a, b)` then `arrange both` is not the text
`arrange stack(a, b)`: the let is a statement of the program, so the two are different programs that play the same
song. The interchange law and the equalities of music are not normalised (see "What the laws promise").

## Meaning: the timeline

A **placement** is a section at a start bar. The expression is evaluated to placements: a name is a placement of that
section (a let is replaced by its expression, each time it is used), `then` puts its operands one after another
(each starts where the one before ends), `stack` starts all its operands at the same bar and is as long as the longest,
`silence` is zero bars. The song is the sum of the items of the `arrange`, and a plain list is one placement a
section, as it has always been (`places`, for `after NAME`, `before NAME N bars` and `end after NAME`, gets every
placement; a section named twice plays twice, and `after NAME` of one that plays more than once says so, as before).

The song is then **cut into stretches** at every bar where a placement starts or ends. A stretch is the part of the
song over which the same placements play, and it is one segment of the compiled song: its nets are made from the
parts of every placement that covers it. Nothing after `expand` knows about stacking: the compiler, the runtime
(`nets_at`, `section_at`, the diff), the offline render (`beatscript_render`) and `song_nets` read segments, as they always
did.

```text
arrange stack(groove, riff) coda        groove 8 bars, riff 4, coda 4

bar        0         4         8        12
groove     |---------|---------|
riff       |---------|
coda                           |--------|
segments   groove+riff | groove | coda
           (chain: false) (chain: true) (chain: false)
```

* **The shorter pads with silence.** In the stretch after `riff` has ended, `riff`'s parts are not there: a deck only
  `riff` played stops on that bar line (`Segment.decks` has no entry, the runtime pauses it). A deck `groove` also
  plays goes on: if its net is unchanged across the boundary it is not reloaded.
* **The later stretches go on from where the section was.** A section's parts alternate per bar (`<a b c>`); a stretch
  that begins on bar 4 of an 8-bar section has the section's cycles rotated so that its bar 0 plays the section's bar 4
  (`FlatLine.phase`, applied in `Compiler::deck` as a rotation of `PartIr.cycles`). The notes (and the hash velocities,
  which are of the net's bar) are those of a net that played the section whole.
* **Controls fire once.** A section's control lines (`at bar N:`, ramps, `every`, `do ...`) are added by the first
  stretch of the section and carried (as marks only) into the later ones: they are not fired again, and they count
  from the section's own first bar, wherever it starts. An `every N bars` line in a section behaves exactly as it does in
  a section of a plain `arrange` (it is not cut at the section's end).
* **Names and the runtime's section.** A stretch is named by its layers: `groove`, `groove+riff`. The stretches of one
  item of the `arrange` after the first are marked `chain` (as the stretches of a net cut in 64-bar pieces are), so
  `section_at` and `btw get` read them as the one section the item is: `groove` bar 6 of 8.
* **Marks.** Every line of every placement that plays in a stretch is in the segment's `lines` (the gutter lights all of
  them); the playhead (`Marks.header`) sits on the header of the section that ends last in the stretch (the first of
  them on a tie), and the headers of the others are lit as lines. A hit's spans are those of the token in the section's
  text, in every stretch it is carried into. `via`, `let` and `def` expansion are as in a plain section.
* **Clashes.** Two placements that play at the same time may not both define the same deck's role: an error at the second
  definition, naming both sections and the line of the first (`sections `a` and `b` play at the same time, and both define
  deck A's `kick` (the other is on line 2)`, or `section `a` is stacked with itself, and it defines ...`). The song's own
  lines (outside every section) stay under every stacked section and a section's own part of a role wins, as in a plain
  section; their alternations go on across the stretches of one item of the `arrange` (and start again at the next
  item, as they always did at the next section).
* **Evolve plays alone.** An `evolve` rewrites its section's nets generation by generation; a section with an `evolve`
  may be in a `then` (or on its own) but not in a `stack` with another section: an error at the `evolve` says so.
  (Evolve semantics are untouched.)
* **Limits.** `stack` and `then` nest at most 32 deep when written. A chain of lets is resolved at most 256 levels deep,
  which is **128 lets** when each is a `then` or `stack` of the one before (the hop through a let and through the
  composition it names costs two levels; a let that is only another let's name costs one). A program places sections at
  most 16,384 times (MAX_SONG_BARS), on top of the song's own 16,384-bar cap and the 4,096-line expansion cap: the
  expansion is total. The names in an expression are checked before anything is placed.

### Names and lets

A `let NAME = stack(...)` / `then(...)` may use sections and the lets **above** it; using one below (or itself) is an
error that says so. Using an unknown name is `there is no section `vers`` at the name's own line and column, with
`did you mean `verse`?` from the sections and the lets known so far. A let that does not read is said once, not again
at every use. A composition's let is a statement at the top of the program (not in a `def` or `repeat`), and it is not
brought in by `import` (an import brings the lets and defs of a snippet, and a snippet's sections are not imported).

### Over parts

```text
deck A: stack(kick x ~ x ~, snare ~ x ~ x)         ->  deck A: kick x ~ x ~ | snare ~ x ~ x
deck B: then(bass c2 e2, bass g2 ~)                 ->  deck B: bass <[c2 e2] [g2 ~]>
deck B: then(bass <c2 e2>, bass g2)                 ->  deck B: bass <<c2 e2> g2>      (c2 g2 e2 g2)
```

* `then(p, q)` is one part whose pattern is the alternation `<p q>`: bar 0 plays `p`, bar 1 plays `q`, and round. An
  operand with an alternation of its own (`<c2 e2>`) advances once each time its turn comes round (the alternation of
  an alternation, as mini-notation already has it). It is a bar of each exactly when the operands are one bar.
* **`then` is a sequence, so a nested `then` flattens and the music is the flat one's.** The associativity law is stated on the
  text (nested `then`s flatten in the canonical form: one text, one CID) and the music of a program is the music of its
  canonical text, so the law holds for what is played too: `then(then(a, b), c)`, `then(a, then(b, c))` and `then(a, b, c)` are
  one program, `kick <a b c>`, whose bar 0 plays `a`, bar 1 `b`, bar 2 `c`, and bar 3 plays `a` again at *its own second
  cycle*. When an operand alternates (`a` is `<x ~>`), that is `x x ~`, `~ x ~`, round: each operand advances once each time its
  turn comes. What `then` never means is the alternation of an alternation that the nesting would suggest (`<<a b> c>`: `a c b c`,
  because the inner one advances once per visit of the outer): that is a different program and is written as the alternation it
  is, `kick <<<x ~> x> ~>`, which has its own text and CID. (Found by the laws reviewer, L-24: `then(then(kick <x ~>, kick x),
  kick ~)` is `kick <<x ~> x ~>` and plays `x x ~ ~ x ~`, not what the nested alternation `<<<x ~> x> ~>` plays.)
* The operands must be parts of **one role** and carry **the same parameters** (`vel=`, `oct=`, `inst=` ...) and **the same cycle**
  (`hihat/12`, F6: the alternation advances once per cycle of the part): the result is one part. An error names the second operand: ``then` follows a part with another part of its role: `snare` is
  not `kick``, and ``then` makes one `bass` part, so its operands share their parameters: this one has none, the first
  `vel=90``. An operand that is a stack of several parts is an error (stack the roles outside:
  `stack(then(kick .., kick ..), then(snare .., snare ..))`).
* `silence` is the unit of both over parts too; a line that comes to no part (`deck A: silence`) is an error.
* `stack(...)` may stand anywhere a part does, between `|`s; it needs a deck and takes no time word, like any part.

### Errors

Every error has a line and column. A call that is not `stack` / `then` (`stak(a, b)`) says `did you mean `stack`?`;
`stack` without its parenthesis, a missing `)`, an empty `stack()`, a trailing comma, an `arrange` of nothing but
`silence`, nesting past 32 and a `let` below its use each have a message of their own (the tests list them).

## For code that reads the compiled song

* A segment of a stack is named by its layers, `a+b` (a section's name has no `+`); `Compiled::sections_at(bar)` gives
  the names playing on a bar, `section_at(bar)` the name of the segment, the bar in the item and the item's length (the
  segments of one item of the `arrange` after the first are `chain`).
* `expand::Lowered.segments` are stretches; `SegmentLines` has `chain`, `header_more` (the other sections' headers) and
  `phase` (how far into its item the stretch begins: the song's own parts are phased by it); `FlatLine.phase` is
  `Some(k)` on a copy carried into a later stretch of a stacked section (its parts' cycles are rotated by `k` in
  `Compiler::deck`, its controls are not added again).
* `algebra::SExpr` (the normal form), `arrange_items`, `arrange_text` and `Lower::plan_arrangement` (placements, stretches,
  `places`) are the pieces a later feature (an `assert ... in NAME`, a per-section key) can build on; `places` has every
  placement of a section.

### Joins with the other features

Found by merging this branch into one that already had F1, F2, F5, F6, F7 and F11 (`crates/djlive/tests/ext_cross4.rs`).

* **A key per stretch (F5).** A section's own `key` (the head of its body) is read once per placement. A stretch has ONE key: the key of the
  first section playing in it that sets one (in the order of the normal form, so `stack(a, b)` and `stack(b, a)` agree), else the program's
  (C major where it has none). Two sections that play at the same time and set different keys are an error naming both (at the second `key`, with
  the line of the first, once however often the two meet); two that set the same key, however spelled (`am`, `a:minor`), agree. A section with no
  key follows the stretch it is in: a longer unkeyed section under a shorter keyed one is in the keyed one's key for its bars and in the
  program's after (`stack(groove, coda)` with `coda` in `am` is `am` for four bars and `dm` for the next four; write a `key` in `groove` to keep it).
  `then` is not a clash: each of `then(verse, bridge)` is in its own. `Compiled::section_key(name)` answers for a section (the first stretch it plays
  in: its own key where it sets one) and for a segment's name (`intro+verse`), so every name `sections_at(bar)` gives has a key;
  `key_in(&Segment)` is the segment's own.
* **Chance goes on across stretches (F7).** A later stretch begins `phase` bars into its section, so its chance is unrolled over `phase +
  bars` bars (`Compiler::chance_bars`) before the cycles are rotated: bar j of the stretch plays what bar `phase + j` of the section alone
  plays, for chance, `choose`, `maybe`, alternations and parts with a cycle of their own (`hihat/12`; the net's bars are what is rotated).
* **Cycles (F6).** `then` of parts needs one cycle (`hihat/12` and `hihat/12`); `stack` keeps each part's own.
* **Defs, repeats, wires, imports (F2, F11).** A part expression is read where a part is: in a def body (kept as written, so a `stack` inside
  a def is not reordered: two orders are two CIDs that play the same notes), with `$i` and arithmetic in its operands (`stack(kick x*$n, ...)`),
  with a wired port as an operand's pattern (`kick $rhythm`), across lines inside the parentheses, and through an import (a def that has one is
  imported as any def). A section name is text: `arrange` and `let NAME = sexpr` are statements of the top of the program, so a `$name` in
  their place is an error that says there is no `$name` there (`arrange stack(intro, $i)`).

## Decisions

1. **Normalise when reading.** The parser applies the laws as it builds an expression, so the AST, the canonical text,
   the placements and every later stage see the normal form only. (The alternative, normalising when printing, would
   leave two ASTs for one program.)
2. **A plain list stays `Arrange`.** `StmtKind::Arrange(Vec<Word>)` is still what `arrange a b c` is, and any
   expression whose normal form is a list of names is the same (`arrange then(a, then(b, c))`); only a normal form
   with a `stack`, a `then` inside one, or `silence` is the new `StmtKind::ArrangeExpr`. Existing programs go through
   unchanged code paths in the parser and printer.
3. **Stretches, not overlapping segments.** Stacked sections are laid out as consecutive segments of the compiled song, cut
   where a placement starts or ends, rather than adding overlapping segments to the runtime. The cost is that a stack of
   unequal sections is several segments (names `a+b`, `chain`), and a deck whose net is the same across a boundary is
   not reloaded; a net whose alternation is out of step is reloaded on the bar line (a swap, as any section change is).
   The gain is that the runtime, the diff, the render, `song_nets`, the marks and `btw get` needed no change.
4. **Stack of parts sorts only what is spelled `stack(...)`.** A `|` list is existing text whose order is part of its
   CID; sorting it would move CIDs. `stack(...)` is new, so it gets the symmetric form. The two meet when a `stack`
   is written between `|`s (it is sorted in its place).
5. **Names are opaque to the laws** (above).
6. **`then` over parts is the alternation** and needs one role and one parameter set; an operand with one term is put
   in the alternation as itself, one with several as a group `[...]`. **A nested `then` is part of the sequence, not an
   inner alternation** (the law is on the text, and the music is the music of the flattened text; see "Over parts"): to
   nest alternations, write them (`<<a b> c>`).
7. **A let of a lone name is the scalar let** (above), so the laws hold for lets too and the canonical form stays a
   fixed point.
8. **Controls are not clipped at the end of a padded section.** The brief says the shorter's "lines end": its *parts* do
   (its decks stop); its control lines keep the timing they have in any section (an `every` line continues, a ramp runs
   its `over N bars`). Making `every` stop at a section's end would change plain sections.
9. **The header on the playhead is the one that ends last** (ties: the first in placement order); the other headers are
   lit as lines. `Marks.header` is a single span, and it is not worth a second field in the window for this.
10. **Unequal stacks and `evolve`** are an error, not a silent change of what an evolve means.
11. **Section names are checked in lowering, not when the expression is read**, because the parser does not know the
    sections yet (they may be written below the `arrange`); one pass reports every unknown name, each with the sections
    that exist. The shape (`stack(`, `then(`, the parentheses) is checked when it is read.
12. **The timeline is one sweep.** The song is cut at the placements' edges and swept left to right with the placements
    playing (a plain list has one at a time), so an `arrange` of thousands of names costs what it did.

## What the laws promise (and what they do not)

*The categorical remarks, ready to paste.* These replace the one sentence about iteration in a monoid in the
"Evolving" section of docs/BEATSCRIPT.md ("transforms are endomorphisms ..., and iterating them is iteration in their monoid"),
and go in a new subsection "Composition: stack and then" after the `section` and `arrange` bullets of "Functions and
structure", with a row in the canonical-form table.

---

````markdown
**Evolve's transforms are a monoid.** A transform is an endomorphism of the realised hits of a part (a part to a part
of the same shape). Writing them in a rule, `rotate 1 | transpose 5`, is composition in order, which is associative
and has an identity (`transpose 0`, `rotate 0`, `octave 0`): the transforms of one part form a monoid, and a rule
whose transforms do not mention `gen` makes generation n the n-fold composite `f^n`, iteration in that monoid. It is not
commutative (`rotate 1 | reverse` is not `reverse | rotate 1`). A rule that does mention `gen`, or `thin`, which hashes
the generation, is a *sequence* of endomorphisms (`f_1, f_2, ...`) rather than one repeated, and generation n is
`f_n(...f_1(gen 0))`: still a composite, no longer a power. Nothing about evolve changed with `stack` and `then`.

### Composition: stack and then

A song is a composition of sections, and the two ways of composing them are in the language:

    arrange intro stack(then(riff, riff), verse) coda

`then(a, b)` plays `a` and then `b`; `stack(a, b)` plays them at the same time, as long as the longer (the shorter is
padded with silence: its decks stop); `silence` is the empty section; `arrange a b c` is `then(a, b, c)`; a `let`
names a composition. Over parts after `deck X:`, `stack` is `|` and `then` is the per-bar alternation `<p q>` of
parts of one role.

**The structure.** Give every section expression a length in bars: a section's own, `then(a, b)` is `|a| + |b|`,
`stack(a, b)` is `max(|a|, |b|)`, `silence` is 0. Sections form a symmetric monoidal structure in the sense of
Hudak's music algebra (`:+:` and `:=:` of Haskore / Euterpea): a section of n bars is an arrow from bar line k to bar
line k + n, `then` is composition of arrows (end to start) with the 0-bar `silence` as identity, and `stack` is the
tensor of two arrows from the same line, padded to end together, with `silence` as its unit and the swap of its
operands as the braiding. Objects are the bar counts (the lines of the song), `then` composes, `stack` tensors,
`silence` is the unit, the braiding is the sort in the canonical form.

**What the laws promise.** The canonical form normalises exactly the axioms of that structure: associativity of
`then` and of `stack` (nested ones flatten), the unit (`silence` is dropped; a one-operand `stack` or `then` is its
operand; `then(silence)` is `silence`), and the symmetry of `stack` (its operands are sorted by canonical text, which
is the braiding made canonical). Programs equal by those laws have the same canonical text, so the same CID and the
same compiled song, and `btw fmt` shows the normal form. The length is a homomorphism: to `(N, +, 0)` for `then`
and to `(N, max, 0)` for `stack`. `then` is not commutative and is never reordered, and `stack` is not idempotent
(`stack(a, a)` plays `a` twice: two sections, which clash if they define one deck's role).

**What they do not.** Equal *music* is not equal *text*. The interchange law, `stack(then(a, b), then(c, d))` equal to
`then(stack(a, c), stack(b, d))` when `|a| = |c|`, holds for what is played (the same notes in the same bars, so the
same nets of the song written out longhand) but it is not normalised: the two are different programs with different
CIDs. A section cut in two halves is not the section, because every section has its own clock (`at bar N`, `every`
and its alternations count from its first bar). A `let` names a composition and a name is opaque to the laws:
`let both = stack(a, b)` and `stack(a, b)` are not the same text. And the laws are about the shape of the
composition, not about the sounds: they say nothing about whether two sections sound alike.
````

---

## Not done

* `stack` and `then` are over sections and parts only. There are no compositions of control lines, and `stack` does not
  take parameters (`stack(a, b) oct=2` is an error: the parameters are the parts').
* The interchange law and the equalities of music (above) are not normalised.
* `import` does not bring a `let NAME = stack(...)` (a snippet's sections are not imported either).
* A section with an `evolve` cannot be stacked with another.
* One header on the playhead (the one that ends last); the others are lit as lines.
* A stack of unequal sections reloads a deck's net at the boundary when the alternation's phase moves (identical nets
  are not reloaded); the swap lands on the bar line like any other.
* `after NAME` of a section that plays more than once is still an error (it says which count; `at bar N` to pick one is
  the message's promise and is not built, as before).
* Control lines of a padded section are not cut at its end (Decision 8).
* Two keys at once are refused, not resolved: there is no per-section key inside one stretch (a net has one key).

## Tests

`crates/djlive/tests/ext_cross4.rs` (18, the joins above and the laws again with guards, arithmetic, a wired port, chance, a cycle, degrees and keys
in the sections). `crates/djlive/tests/ext_algebra.rs` (50): the canonical form by the laws; unequal stacks, `chain`, phases, `places`;
errors with positions and suggestions; parts; the highlight words; a fake host and clock (the shorter's deck stops,
nothing else reloads, controls fire once, the marks follow the stack); and the property tests (160 random compositions
of 1 to 7 of eight sections each): re-association, unit insertion and operand permutation keep the CID (and survive
`reformat`), a swap of two `then` operands changes it, `stack(a, b)` and `stack(b, a)` compile to one timeline,
`arrange a b c` is `arrange then(a, b, c)` for any list, and the compiled song of a random composition has the same net
CIDs as the longhand written by an independent oracle. Unit tests in `algebra.rs` (6): the constructors' laws.
