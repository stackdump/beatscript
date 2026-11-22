# Call and response (long note)

The short section is "Call and response" in docs/BTW.md; the code is `crates/djlive/src/answer.rs`, the tests
`crates/djlive/tests/answer.rs` (the hits and what `check` says) and `crates/btwapi/tests/answer.rs` (the nets, proved against
the longhand with `same-as`), the example `examples/btw/answer.btw`.

## What it is for

Write the call once; another deck plays in the gaps it leaves, and follows when the call is edited. One pattern call does it:

```text
rests(p)        a hit on every step where p rests
rests(p, q)     q, kept only on the steps where p rests
```

There is no new statement, no keyword and no grammar change: `rests(p)` and `rests(p, q)` are the `call` production
(`NAME "(" [ arg { "," arg } ] ")"`) with a `let` name, a `[ pattern ]` or another call as each argument. As with every call, an
alternation given as an argument is written in brackets: `rests([<call ~>], reply)`.

## What was already there, and what was not

- **A shared `let`** on two decks, and `<call ~>` on one deck against `<~ call>` on the other, already make a call and a response a
  whole bar at a time. What was missing is the complement of a rhythm inside the bar.
- **`mask b`** (an arrange mutation) is the other way round: it keeps the steps where `b` rests *of a pattern*, rewriting the let.
  It does not make a second deck's part, and it does not move when the call is edited somewhere else.
- **`duck B under A.kick`** is a level move on a low part (and is an error under `snare`: the engine's sidechain listens for a
  deck's kick). It is not note by note.
- **Typed ports** (`def call() out: rhythm`, `wire call.out -> answer.in`) already carry one rhythm to two decks, and `rests` is
  among the calls a `rhythm` may hold, so the call can be wired. A port cannot compute the opposite of a rhythm: that is the gap.
- **Passing a token between decks while they play** is refused by the language's own rule, not by taste: a net is compiled once,
  so a pattern cannot read an input that arrives while it plays, and an input cannot decide whether a part plays (see
  "Performed inputs"). A token from one deck deciding another's part is the same thing: the nets would no longer be independent
  (each deck's net is keyed by its own text, `check` keys them per deck and section), a net's CID would depend on another deck's,
  and the offline render and `same-as` would need run-time state.

Not built, on purpose: a `deck B answers A.snare` statement or an `A.snare` pattern expression (a reference from one part to
another needs an order and a loop check, and is a third way to say it); a `link` or `respond` statement (nothing to hang it on);
a new arrange mutation (a fourth vocabulary for transforms); a complement symbol (`!p`: the language prefers a named call); a
complement on the sixteenth grid (it would answer `x ~ x ~` with fourteen hits).

## The rule

**Steps.** `steps(p, bar)` is how many equal steps `p` has in the bar:

| the pattern is | its steps |
|---|---|
| several terms | the number of terms |
| one term `[..]` | the steps of what is inside (`[a b, c d e]`: the grid the layers share, three against two is six) |
| one term `<a b>` | the steps of the term this bar plays |
| one term `t*k` | `k` times the steps of `t` (read at its first repeat) |
| one term `e(k, n)` or `euclid` | `n` |
| one term `arp(c, m, n)` | `n` (8 when left out) |
| one term `fast(p, k)` | `k` times the steps of `p` |
| one term `slow(p, k)` | the steps of `p` over `k`: an error when they do not divide |
| one term `every(n, p, q)` | the steps of the one this bar plays (a rest when `q` is left out and it is not the `n`th bar) |
| one term, another transform or `rests` | the steps of its first pattern |
| anything else (a note, a rest, a chord, a numeral) | one |

**Sounding.** A step sounds when `p` has an event whose position is in `[i/n, (i+1)/n)`, with the 1e-9 slack the other transforms
use. Only onsets count: a ghost, a flam, a ratchet or a chord is one event; a note held by `len=`, a slide or `c2~3` does not hold
the steps after it.

**The answer.** `rests(p)`: for each bar, one plain `x` hit (no velocity, no marks) at `i/n` for every step `i` that does not
sound. `rests(p, q)`: the events of `q` for that bar, read as in this part and otherwise untouched, whose position falls in a step
that does not sound. These are the events of a hand-written complement, so the nets and CIDs are the longhand's.

**Read as this part.** `p` is read with this part's key, octave and bar, for onsets only: in a drum part a numeral or a chord is
read as a chord's onset (it never errors there), and the part's voice leading is left where it was, so `q`'s chords are led from the
chord before the call. A part with `/N` reads `p` in its own cycle: give both decks the same cycle.

**Period and totality.** The period is the lcm of the call's and the reply's; at most `steps(p)` events a bar, inside the existing
event and read budgets (`MAX_EVENTS`, `MAX_READS`). The same text and the same bars are the same CID. There is no clock and no state.

## What `check` says

All at the argument, with line and column, in the language's terms:

```text
error: rests() takes 1 to 2, and this has 0
  = help: rests(call) or rests(call, reply): the call, and the pattern kept where it rests

error: `rests(call)` answers a fixed rhythm, and `call` has chance in it (`x*8?0.5`)
  = help: each part rolls its own dice (its seed is of its deck, its role and its text), so the call and its answer would
    disagree: take the chance out of the call and put it on the part that plays it (`snare maybe(call, 0.8)`)

warning: `rests(call)` plays nothing: `call` sounds on every step of its bar
  = help: leave a rest in the call, or answer a different pattern
```

An unknown name keeps the message it has (`` `hist` is not a step, a note or a scale degree ``), a near miss of the call's name
is told (`rest(` is `rests`), a `name=value` is an error (`rests() takes patterns, not `name=value``), and `slow` over bars that
do not divide the steps says so. "Chance" is `?`, `choose`, `maybe`, `degrade` or `markov` anywhere in `p` (`humanize` moves
velocities, not hits). Why it is refused: with `let h = x*8?0.5` on two decks the checker reports a different seed for each
(the implicit seed is of the deck, the role and the text, and the note enters the hash), so the dice would differ even for one
shared `let`. Chance in `q` is the reply's own and is allowed. An arrange mutation's dice (`thin`) rewrite the let's text before
the call is read, so they are fine.

## What the nets can say

Nothing new at run time, which is the point. Each part's net stays an independent ring with its one token; the two decks share no
place and no transition. "The answer's hits and the call's are disjoint and together cover the call's steps" is a property of the
bindings, true by construction here, and it is what `crates/djlive/tests/answer.rs` checks over random calls with fixed seeds.
It is not provable by reachability: there is no arc to reach through.

## Decisions

- **Both forms** (`rests(p)` and `rests(p, q)`). Without the second a bass cannot answer with notes (it gets root hits).
- **No chance in the call** (an error). Forbidding is the safe default and can be relaxed later; the reverse cannot.
- **The name is `rests`**, over `gaps` and `answer`: it says what the call is a hit on. It is not in `pattern::TRANSFORMS`
  (`parse::reserved` reads that list), so `let rests = ...` is still a valid program with the CID it had (pinned in the tests); the
  word is the call only before a `(`, as `knob(` and `pad(` are (`parse::is_syntax_here`): a `let` of one value named `rests` is put
  in place of the word wherever it stands except there.
- **A lone term is looked through, and a repeat multiplies.** `[x ~ x ~]*2` is eight steps, not one step (which would make the
  answer to a doubled call nothing). When the copies of a repeat differ (an alternation under a repeat) the first copy gives the
  count, because no grid of equal steps fits them all.
- **A silent call is one rest**, so one hit on the downbeat: the second form answers a whole bar.
- **The answer is a function of the call's text.** Muting or emptying the call's deck leaves the gaps where they are.

## Known limits

- A held note does not hold its steps (a sung vowel rings up to half a bar into the gaps).
- `slots()` (the step count `rotate` and `reverse` use) counts a lone `e(3,8)` as one step, so `rotate(e(3,8), 1)` is a silent
  no-op, and so is `rotate(rests(e(3,8)), 1)`. `rests` does not use `slots()`; changing it would move the CIDs of programs that
  use `rotate` that way, so it is a decision of its own.
- An alternation under a `*k` whose copies have different steps is counted at the first copy.

## Tests

`crates/djlive/tests/answer.rs`: the hits for eight, four and sixteen steps, groups, `e(k,n)`, `<..>`, transforms, repeats and
layers; the two-argument form; a call that comes round over bars; the disjoint-and-covering property over random calls on 2, 4,
8 and 16 steps; a numeral in a drum part; the voice leading left alone; the three messages with line and column; the `let rests`
program and its CID; the canonical text as typed; a rhythm port; an arrange mutation; reformatting. `crates/btwapi/tests/answer.rs`:
`same-as` between every form, the three examples, and their longhand.
