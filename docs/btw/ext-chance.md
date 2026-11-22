# F7: seeded chance (`crates/djlive/src/chance.rs`)

`x?0.3`, `choose [a b c] weights 3 1 1 seed=4`, `maybe(p, 0.3, 4)`: parts that are not the same every bar, and still **a function of the text
and the bar**: nothing reads a clock or a random source, the same program is the same notes on every machine and every run, and a hot reload of
the same text touches nothing.

## Grammar

```ebnf
term        = atom [ "!" ] [ "^" ] [ "*" int ] { "?" prob } ;       (* x*8?0.7: each of the eight hits is kept with chance 0.7 *)
prob        = number ;                                              (* 0..1 or a percent, at most six decimals: 0.3  30%  0.333333 *)
atom        = ... | "choose" "[" pattern "]" "weights" weight { weight } [ "seed=" int ]   (* exactly one weight an option *)
            | "maybe" "(" pattern "," prob [ "," int ] ")" ;         (* a call, like the other generators and transforms *)
weight      = number ;                                              (* 0 or more, at most six decimals *)
```

* `?` is glued to the term before it and to its number (`x?0.3`, never `x ?0.3` or `x? 0.3`): the same adjacency rule as `hihat/12` and
  `A.kick`. A repeat, accent or slide comes before it (`x*8?0.7`, `c2!?0.5`); after it is an error that says so. `x?0.5?0.5` is a chance of a chance
  (the hit must pass both).
* `choose` takes the terms of its brackets as its options (a comma would stack, so it is an error), then `weights` and **one weight for each option**,
  then optionally the term's own `seed=N`. The weights are read **by count**: `choose [a b] weights 3 1 2` is two weights and then the degree step `2`
  (a warning says so, in case a third weight was meant; a `seed=` written right after the weights ends the choice and the warning).
  Fewer numbers than options is an error naming both counts.
* `maybe(p, prob[, seed])` is the call form for a whole pattern: `p` with `?prob` on each of its hits.

## Semantics

**A chance is decided by an integer hash of `(seed, bar, cycle, step, note)`** (`chance::hash`):

```text
h = 0x24386e5ca1b05d1f
for v in [seed, bar, cycle, step, note]:  h = splitmix64_finalizer(h XOR v as u64)      (* SplitMix64's mixer, wrapping *)
hash = h >> 32                                                                          (* 32 bits *)
```

* **`x?p` keeps a hit when `hash < floor(p * 2^32)`.** `p` is a `Rat` (`0.3` is `3/10`, exactly as typed, at most six decimals; `30%` and
  `33.3333%` are read the same way), the threshold is an integer, the comparison is integer: `p = 0` keeps none, `p = 1` keeps all.
* **`choose` picks the option `i`** for which the hash, as a share of the weights' sum (`floor(hash * total / 2^32)`), first falls under the running
  total of the weights (exact: fractional weights are put over a common denominator first, so `0.25 0.75` is `1 3`). A weight of 0 is never picked.
  The option fills the choice's whole slot, as an `<a b>` alternative does; a `choose` in a pattern is one slot like any term.
* `bar` is the pattern cycle being read from the top (the net's bar for a bar part; the part's own cycle for a `/N` part, F6), `cycle` the cycle the
  term itself is in (it differs from `bar` inside `<a b>`, `fast`, `slow`, `every`), `step` the event's place in the cycle in 960ths taken from the slot
  the term fills (so `x?0.5 x?0.5` is two tosses, not one), `note` the MIDI note of the event (-1 for a drum hit; a chord is tossed note by note;
  a `choose` uses a marker no note has). Position is exact: the slot is tracked as a fraction of the cycle through groups, repeats and terms.
* **The seed** is the term's own (`choose ... seed=4`, `maybe(p, prob, 4)`), else the part's `seed=N`, else a hash (`fnv`) of **the deck, the role and
  the pattern text, and nothing else** (L-05): `deck A:kick x?0.5 x x?0.5 x` is the seed of those words, so nudging a `vel=`, an `oct=` or an `inst=` live
  does not re-roll which hits drop (it used to hash the whole part text, params included: `vel=90` and `vel=91` were two tosses). Another deck, another
  role or another pattern is another toss; `seed=N` keeps one across all of them. A part with **no chance** in it (a `markov` part) keeps the text it always
  hashed, params included, so no old program's notes or CID moves; a part with both uses the new text for both. The term's own seed wins over the part's. **A `seed=` written right after a `choose` is the choice's**; to give the
  part one too, write a second (`choose [a b] weights 1 1 seed=4 seed=9`), or end the pattern with another term.
* **A hit that is not kept does not exist**: it is not in `PartIr.cycles`, so it does not fire a mark. A kept hit lights its token as any hit does (its
  chain is the term it came from, then the `x*8?0.7` that held it, then what held that).
* `maybe(p, 0.3, 4)` and `[x?0.3 ~ x?0.3 ~] seed=4` make the same hits (the test compares them).

**How many bars.** Chance makes bars differ, so a pattern with chance is **unrolled** over a number of bars, and the net is that long (bars repeat after it):

| where | unrolled over |
|---|---|
| a program with no sections | **16 bars** (`chance::DEFAULT_BARS`) |
| a `section` | the section's length, up to **64** (one net, `chance::MAX_UNROLL`); a longer section repeats its 64 bars |
| with an alternation or a cycle of its own | whole copies of what those make, so no copy is cut: `<a b c>` and 16 is 18 bars; 7 bars of `/N` and 16 is 21 |

The structural period (alternations, cycles) is `p`; a chance part makes the net `p * ceil(unroll / p)` bars, or, when that passes 64, the most whole copies of `p`
that fit (`chance::unrolled_period`). A part with no chance in the same deck is simply repeated inside it. A deck whose parts already take longer than a net keeps
its (chained) length. A part with no chance, in a program with none, is exactly what it was: no existing net or CID moves.

**A chance net has no period (L-10, L-21).** The unrolled length is the net's length, a function of where the part is placed (16 bars flat, a section's
bars, 64 at the most), and a chance part never comes round: it is a hash of the bar. So `meter::period_of` counts a deck with chance by what its parts
come round in without the dice (`PartIr::structural_bars`, the alternations and cycles only) and lists the chance parts (`Period::chance`), and
`assert period == N` of a program with any says **"the program has no period: kick on deck A has chance, which differs every bar (the rest comes round in
3 bars (hihat/12 against the bar))"** for every comparison, wherever the part is placed. The `period:` note of `check` (polymeter) never depended on the
placement, and still does not.

`check` reports **every chance term** as a note (severity `info`, once for each length it is unrolled over):

```text
chance: `x*8?0.7` keeps each hit with probability 0.7 (seed 12); it differs every bar, unrolled over 16 bars
chance: `choose [c2 e2 g2] weights 3 1 1 seed=4` picks one of 3 options a bar, weights 3:1:1 (seed 4); it differs every bar, unrolled over 16 bars
chance: `maybe([x ~ x ~], 0.3, 4)` keeps each hit with probability 0.3 (seed 4); it differs every bar, unrolled over 16 bars
```

`bar` is the bar of the net, which in a song is the bar of the section (the same section on the timeline twice plays the same bars twice); in a program with no
sections it is the program's bar modulo the unrolled length.

## Canonical form

`x*8?0.7`, `choose [a b c] weights 3 1 1 seed=4`, `maybe([x ~ x ~], 0.3, 4)`. `?30%` prints `?0.3`; `?0.50` prints `?0.5`; **`?0`, `?0%` fold to a rest and `?1`, `?100%` to the bare term** (`x?0 x` is `~ x`, `x*8?1` is `x*8`: the program
written out, so one CID, and `check` does not say it differs every bar; a chance of a rest is a rest; L-09); weights print as
numbers do (`0.50` is `0.5`); a seed is printed only when the choice has its own. Random reformatting keeps the CID, any edit of a probability, a weight, an option, a seed
or the number of terms changes it (the tests do both).

## Decisions

* **16 bars, not 64, for a program with no sections.** The contract's text says 64; the brief for this run says a bounded default such as 16. Sixteen bars is a phrase and
  a quarter of a net; a section is what says more. The constant is `chance::DEFAULT_BARS`.
* **A chance is per event.** A chord is thinned note by note; to keep or drop a chord whole, put a `choose [chord(am) ~]` or a `maybe` on a one-note pattern.
* **The decision includes the term's cycle and place**, beyond (seed, bar, step, note) of the contract: without the place two `x?0.5` in one part would make one toss; without
  the cycle the two repeats of `fast`, or two alternatives of `<a b>`, would. For a plain pattern `cycle` is `bar` and `step` is the position in the bar.
* **The weights are read by count**, per the brief; too many numbers cannot be told from a degree step that follows, so that case is a warning, not an error.
* **A probability or weight has at most six decimals**, so that it is exactly a `Rat`. An error says so (`x?0.3333333`).
* **`step` is the exact slot fraction rounded to 960ths of the cycle** (60 to a sixteenth), `note` -1 for a drum hit.
* **`maybe` is a transform** (listed in `pattern::TRANSFORMS`, so `reserved` and the did-you-mean see it); `choose` is a keyword atom (both are in `parse::EXT_WORDS` already).
* **A constraint reads every roll.** `assert never ... beside|during ...` (docs/btw/ext-assert.md, "Constraints") is judged on a second grid a part with chance keeps,
  `PartIr::may`, read with `ChanceCtx::all`: every `?` and `maybe` hit kept, every option of a `choose` that a weight can pick played at once. It is a superset of the rolled
  cycles (a test holds it to that through an evolve, a stack and a chained net), so the rule is about every roll and not the one the text rolls. `x?1` and `x?0` are already
  the program written out, so they have no chance to read.
* The hash is SplitMix64's finalizer chained, not `compile::fnv`: FNV over small integers mixes the low bits poorly, and the empirical-rate test (4096 bars) wants a flat 32 bits.
  It is pinned by golden values worked out in Python (`chance::tests`, `ext_chance`).

## Not done

* Chance on a control line (`at bar 4: master: filter ... ?0.5`), on a section, or on a whole `deck` line: only on pattern terms.
* No chance in a `choose` weight (the weights are numbers written out).
* `?` after a rest `~` is allowed, does nothing and is not printed (`~?0.5` is `~`). `maybe(p, 0)` and `maybe(p, 1)` are not folded: they are calls on a pattern.
* A chance takes arithmetic only in parentheses, glued to its `?` (`x?(1 / 4)`, `x?($i / 4)` in a repeat, `x?$p` in a def called with an arithmetic argument, found when F2 met F7);
  a bare `x?1/4` is not read, and a chance takes no performed input (`x?knob(1)` is an error: a net is compiled once).
* The window does not mark a dropped hit (it is not there to mark): showing the tosses that failed is a UI feature.
