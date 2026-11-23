# F6: Euclidean rhythms and polymeter (`crates/djlive/src/meter.rs`)

Two things in one module: `e(k, n[, rot])` is checked and named, and `role/N` gives a part a cycle of its own.

## Grammar

```ebnf
part        = role [ "/" int ] pattern { param } ;          (* hihat/12 x x ~ x   the cycle is 12 sixteenth steps *)
call        = ( "e" | "euclid" ) "(" int "," int [ "," int ] ")" ;   (* e(k, n, rot): 0 <= k <= n, 1 <= n <= 64, any rot *)
```

* `/` is glued to the role and to the number (`hihat/12`, never `hihat /12` or `hihat/ 12`): the same adjacency rule as
  `x?0.3` and `A.kick`, so a space is an error that says so. `N` is a whole number from 1 to 64.
* `e(k, n, rot)` is not new syntax; what is new is that it is checked, reported and linked to the ontology (below).

## Semantics

**`e(k, n[, rot])`** is Bjorklund's spread of `k` hits over `n` steps (`djrust::gen::euclidean`, the engine's own),
rotated by `rot` (taken modulo `n`, so `e(3,8,10)` is `e(3,8,2)` and `e(3,8,-6)` too), as one term: it fills its slot with
`n` equal steps. `kick e(3,8)` is eight steps over the bar (hits on sixteenths 0, 6, 12: the tresillo), `bass/12 e(5,12)` is twelve
steps over a twelve-step cycle. Errors, each at the call with its line and column and what to write:

| text | says |
|---|---|
| `e(9,8)` | `9 hits do not fit in 8 steps` (help: at most 8; `e(8,8)` is every step) |
| `e(3,0)`, `e(3,65)` | `the step count is a whole number from 1 to 64` |
| `e(-1,8)`, `e(2.5,8)` | `the hit count is a whole number from 0 to 64` |
| `e(3,8,1.5)` | `the rotation is a whole number of steps (negative turns it the other way)` |
| `e(3)` / `e(3,8,1,1)` | `e() takes 2 to 3, and this has N`, with `e(5,8)` as the usage |

**`role/N`**: the part's *cycle* is `N` sixteenth steps instead of 16. Its terms share the cycle equally, exactly as a bar's terms
share the bar: `hihat/12 x*3` is three hits in twelve sixteenths (a hit every fourth step), `hihat/12 x x ~ x` four terms of three steps.
A cycle that is not the bar repeats against the barlines, so the part **drifts**: a polymeter. Its hits are unrolled into the bar cycles of the net
(`PartIr.cycles`) over `lcm(N, 16)` steps, that is `N / gcd(N, 16)` bars:

| `/N` | bars to come round |
|---|---|
| 1, 2, 4, 8, 16 | 1 |
| 32 | 2 |
| 12, 24, 48 | 3 |
| 20, 40 | 5 |
| 7, 14, 28 | 7 |
| 10 | 5 |
| 9, 18, 36 | 9 |
| 63 | 63 |

Alternations inside compose by lcm as before, counted in the part's own cycles: `bass/12 <c2 e2>` is two cycles of 12 = 24 steps, and
with the bar `lcm(24, 16) = 48` steps = 3 bars; `bass/12 <c2 e2 g2>` is 9 bars. A deck's net is the least common multiple of its parts'
lengths ([`meter::part_bars`]). `chord(...)`, `arp(...)`, `markov(...)`, notes and degrees work in a part with a cycle (a generator's
positions are fractions of the cycle). Velocity ranges (`vel=60..90`) are picked by the hash of the net's bar and sixteenth, as for any
part; accents and slides cross the barline with the hit.

A term count that does not divide `N` snaps to the nearest step, with the usual warning (`does not divide its 7-step cycle into whole steps`).

**The period.** The program's period is the least common multiple of all its parts' cycle lengths (sixteenths, then bars): of each deck's
net, and of the decks together. `check` reports it as a note (severity `info`, text begins `period:`), only when some part has a cycle of its
own, once for a flat program and once per section of a song (so every existing program says what it said before):

```text
period: 3 bars = 48 steps (hihat/12 against the bar)
period: 21 bars = 336 steps (hihat/12 and bass/7 against the bar and each other)
period: 1 bar = 16 steps (hihat/8 against the bar), in section drop
```

It is exposed for later workers: `meter::period_of(&decks) -> Period { bars, steps, polymeter }`, `meter::program_period(&Compiled) ->
Vec<(section, Period)>` (one entry for a program with no sections, with the name empty), `meter::part_bars(&Part)`,
`meter::cycle_steps(&Part)`; `PartIr.cycle_steps` carries a part's cycle.

**A net is at most 64 bars** (`MAX_BARS`). A deck whose parts with a cycle of their own take longer than that to come round (with the bar and their
alternations) is an **error** naming the parts and the way out, at the first such part:

```text
error: deck A's parts take 119133 bars to come round: hihat/63 (63 bars), bass/62 (31 bars) and kick/61 (61 bars) against the bar; a net is at most 64
  = help: the period is the least common multiple of the cycles and the 16-step bar: give the parts cycles that share a factor with each other
          and with 16 (/8, /12, /24, /32, /48 are a few bars at most), or put them on two decks
```

Two decks are two nets: `deck A: hihat/9 x` and `deck B: bass/11 c2` are fine (their program period is 99 bars; each net is 9 and 11).
A program with no part of its own cycle is not held to this limit: a long alternation is chained as before.

## The ontology

The Bitwrap Ontology (`https://ontology.bitwrap.io/ns#`) models a Euclidean rhythm as an individual of the class
`ns:EuclideanRhythm` ("K pulses spread as evenly as possible over N steps, then rotated") with the datatype properties `ns:pulses`,
`ns:steps`, `ns:rotation` and `ns:midiNote`, and `ns:forRole` (a `role:kick`, `role:snare` or `role:hihat`). Its individuals are the genres'
drum rhythms, one IRI each under `https://ontology.bitwrap.io/rhythm/` (`rhythm:house-snare` is "house snare: E(3,8) rotation 0").
**The table in `meter.rs` is copied from the published ontology** (the Turtle at `https://ontology.bitwrap.io/ns`, read on 2026-11-17: 57
individuals; the ontology MCP tools were not reachable in the run that wrote it, so it was read with `curl`; nothing in the code or tests
touches the network): `EUCLID_TABLE`, and

```rust
meter::euclid_class_iri()                    // "https://ontology.bitwrap.io/ns#EuclideanRhythm"
meter::EUCLID_PROPERTY_IRIS                  // pulses, steps, rotation, midiNote, forRole
meter::euclid_name(k, n, rot) -> Option<EuclidTerm>          // the first named rhythm with these parameters
meter::euclid_names(k, n, rot) -> Vec<&EuclidTerm>           // all of them, in the ontology's order
meter::euclid_names_for_role(role, k, n, rot)
// EuclidTerm { label: "house snare", iri: ".../rhythm/house-snare", role, pulses, steps, rotation, midi_note }
```

The ontology has **no traditional names** (tresillo, cinquillo, clave ...): searching it for them finds nothing, so none are invented here. A pattern is
named when it is one of the 57. `check` says so for a drum part (`kick`, `snare`, `hihat`) whose own role has the rhythm:

```text
e(3,8): the ontology's house snare rhythm (https://ontology.bitwrap.io/rhythm/house-snare)           (snare e(3,8))
e(4,16): the ontology's kick rhythm of country, edm, house, synthwave, techno and trance (https://.../country-kick and 5 more)
```

`kick e(5,8)` is not named (the ontology's `E(5,8)` rhythms are hihats), a tonal part is not named, and the note appears once per call however many
sections build it. **The notes are said only to a program that is linked to the ontology** (R-20): one with a `#! genre:` or `#! tags:` line in its header block.
They were warnings on every `e(4,16)` of every old program (they are `info` now, but still a line on every program), so a program that
never asked for the ontology is not told it; nothing else changes (the net, the CID and F14's JSON-LD export of the rhythm do not depend on it). F14's JSON-LD export uses the same table: the class and properties above, the individual's IRI when there is one.

## Canonical form

`hihat/12 x x ~ x`: the cycle is printed glued to the role, **`/16` is not printed** (it is the bar, `hihat/16 x` is `hihat x`), `euclid(...)` prints `e(...)`
and a rotation is printed as typed (not reduced modulo `n`: `e(3, 8, 10)` and `e(3, 8, 2)` play the same notes and are two programs, like
`2` and `1 + 1`). `PartIr.text` carries it, so a part's seed (when it has no `seed=`) is a hash of text that includes the cycle.

## Decisions

* **The period error is for parts with a cycle of their own only.** A long alternation (an 80-bar `<...>`) is chained as it always was; adding
  `/N` to a part is what makes a net that cannot be.
* **The error is per deck, the report per program.** A net is a deck's, and it is the net that must fit; the period the note reports is the program's
  (decks together), which is what "when does it all come round" means.
* **Alternations count the part's own cycles** (a `<a b>` in a `/12` part advances every 12 steps, not every bar).
* **The note names a rhythm for its own role.** The ontology's individuals are role-bound; `E(5,8)` is a hihat there. The role-blind lookup
  (`euclid_name`) is for exporters. A traditional name would be an invented IRI, so there are none.
* **`/N` bounds**: 1..=64 steps (a whole number). `e` takes at most 64 steps as before.
* **`e(k,n,rot)`'s rotation** is any whole number (an error for a fraction); the engine's rotation is modulo `n`.
* A hit that rounds past the end of a cycle (`x` at 0.97 of 12 steps) belongs to the next cycle's first step, as a hit past the bar always did.

## Not done

* No triplet / non-sixteenth grid: a term count that does not divide the cycle snaps (with a warning) as it always did.
* A cycle is a whole number of steps; `/12.5` and rational cycles are not read.
* The traditional names of rhythms are not in the ontology; adding them is a vocabulary change there, then a table row here.
* The runtime shows no marker for where the part's own cycle begins (hits light by their tokens, as for any part).
